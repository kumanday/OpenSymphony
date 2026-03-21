//! WebSocket-first attachment, event reconciliation, and state mirroring.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, FixedOffset};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tracing::{debug, warn};

use crate::cache::EventCache;
use crate::client::OpenHandsClient;
use crate::config::WebSocketConfig;
use crate::error::{OpenHandsError, Result};
use crate::wire::{
    ConversationInfo, RemoteExecutionStatus, RuntimeEventEnvelope, RuntimeEventPayload,
};

type EventSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamControl {
    Reconnect,
    Stop,
}

enum ConnectControl {
    Connected(EventSocket),
    Stop,
}

enum ReadyControl {
    Ready,
    Stop,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FieldTimestamp {
    raw: String,
    parsed: Option<DateTime<FixedOffset>>,
}

impl FieldTimestamp {
    fn parse(timestamp: &str) -> Self {
        Self {
            raw: timestamp.to_string(),
            parsed: DateTime::parse_from_rfc3339(timestamp).ok(),
        }
    }

    fn is_newer_or_equal_than(&self, current: &Self) -> bool {
        match (&current.parsed, &self.parsed) {
            (Some(current), Some(incoming)) => current <= incoming,
            (Some(_), None) => false,
            (None, Some(_)) => true,
            (None, None) => current.raw <= self.raw,
        }
    }
}

/// Cached conversation state derived from WebSocket events plus REST refreshes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConversationStateMirror {
    /// Latest execution status observed by the adapter.
    pub execution_status: Option<RemoteExecutionStatus>,
    /// Forward-compatible field map derived from full snapshots and keyed updates.
    pub fields: Map<String, Value>,
    #[serde(skip, default)]
    field_timestamps: HashMap<String, FieldTimestamp>,
}

impl ConversationStateMirror {
    /// Replaces the mirror from an authoritative REST response.
    pub fn refresh_from_conversation(&mut self, info: &ConversationInfo) {
        let snapshot_timestamp = info
            .updated_at
            .as_deref()
            .or(info.created_at.as_deref())
            .map(ToOwned::to_owned);
        if let Some(status) = info.execution_status {
            self.execution_status = Some(status);
            self.fields.insert(
                "execution_status".to_string(),
                serde_json::to_value(status).unwrap_or(Value::Null),
            );
            self.record_timestamp("execution_status", snapshot_timestamp.as_deref());
        }
        if let Some(title) = &info.title {
            self.fields
                .insert("title".to_string(), Value::String(title.clone()));
            self.record_timestamp("title", snapshot_timestamp.as_deref());
        }
        self.fields.insert(
            "workspace".to_string(),
            serde_json::to_value(&info.workspace).unwrap_or(Value::Null),
        );
        self.record_timestamp("workspace", snapshot_timestamp.as_deref());
        if let Some(agent) = &info.agent {
            self.fields.insert("agent".to_string(), agent.clone());
            self.record_timestamp("agent", snapshot_timestamp.as_deref());
        }
        if let Some(persistence_dir) = &info.persistence_dir {
            self.fields.insert(
                "persistence_dir".to_string(),
                Value::String(persistence_dir.clone()),
            );
            self.record_timestamp("persistence_dir", snapshot_timestamp.as_deref());
        }
    }

    /// Applies a streamed state-update event to the mirror.
    pub fn apply_event(&mut self, event: &RuntimeEventEnvelope) {
        let RuntimeEventPayload::ConversationStateUpdate(update) = &event.payload else {
            return;
        };
        if update.key == "full_state" {
            if let Some(map) = update.value.as_object() {
                for (key, value) in map {
                    self.apply_field_update(key, value.clone(), &event.timestamp);
                }
            }
            return;
        }
        self.apply_field_update(&update.key, update.value.clone(), &event.timestamp);
    }

    fn apply_field_update(&mut self, key: &str, value: Value, timestamp: &str) {
        let timestamp = FieldTimestamp::parse(timestamp);
        if !self.should_apply(key, &timestamp) {
            return;
        }
        self.fields.insert(key.to_string(), value.clone());
        self.field_timestamps.insert(key.to_string(), timestamp);
        if key == "execution_status" {
            self.execution_status = serde_json::from_value(value).ok();
        }
    }

    fn should_apply(&self, key: &str, timestamp: &FieldTimestamp) -> bool {
        self.field_timestamps
            .get(key)
            .is_none_or(|current| timestamp.is_newer_or_equal_than(current))
    }

    fn record_timestamp(&mut self, key: &str, timestamp: Option<&str>) {
        if let Some(timestamp) = timestamp {
            self.field_timestamps
                .insert(key.to_string(), FieldTimestamp::parse(timestamp));
        }
    }
}

/// Attached runtime stream that owns the event cache, state mirror, and reconnect loop.
#[derive(Debug)]
pub struct AttachedConversation {
    conversation_id: String,
    client: OpenHandsClient,
    cache: Arc<Mutex<EventCache>>,
    state: Arc<Mutex<ConversationStateMirror>>,
    status_rx: watch::Receiver<Option<RemoteExecutionStatus>>,
    stop_tx: Option<watch::Sender<bool>>,
    task: Option<JoinHandle<()>>,
    websocket: WebSocketConfig,
}

impl AttachedConversation {
    /// Attaches to a conversation using initial sync, readiness barrier, and post-ready reconcile.
    pub async fn attach(
        client: OpenHandsClient,
        conversation_id: impl Into<String>,
        websocket: WebSocketConfig,
    ) -> Result<Self> {
        let conversation_id = conversation_id.into();
        let conversation = client
            .get_conversation(&conversation_id)
            .await?
            .ok_or_else(|| OpenHandsError::ConversationNotFound {
                conversation_id: conversation_id.clone(),
            })?;

        let cache = Arc::new(Mutex::new(EventCache::default()));
        let state = Arc::new(Mutex::new(ConversationStateMirror::default()));
        {
            let mut mirror = state.lock().await;
            mirror.refresh_from_conversation(&conversation);
        }

        let initial_events = client.search_events_all(&conversation_id).await?;
        merge_events(&cache, &state, initial_events).await;

        let (socket_url, request) = client.websocket_request(&conversation_id)?;
        let mut stream = connect_socket(request, &socket_url, websocket.ready_timeout()).await?;
        wait_for_ready(
            &mut stream,
            &socket_url,
            websocket.ready_timeout(),
            &cache,
            &state,
        )
        .await?;

        reconcile_and_refresh(&client, &conversation_id, &cache, &state).await?;

        let initial_status = state.lock().await.execution_status;
        let (status_tx, status_rx) = watch::channel(initial_status);
        let (stop_tx, stop_rx) = watch::channel(false);

        let task = tokio::spawn(
            StreamRuntime {
                client: client.clone(),
                conversation_id: conversation_id.clone(),
                websocket: websocket.clone(),
                cache: cache.clone(),
                state: state.clone(),
                status_tx,
                stop_rx,
                socket_url,
                stream,
            }
            .run(),
        );

        Ok(Self {
            conversation_id,
            client,
            cache,
            state,
            status_rx,
            stop_tx: Some(stop_tx),
            task: Some(task),
            websocket,
        })
    }

    /// Returns the conversation identifier associated with this stream.
    #[must_use]
    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    /// Returns the cached event count.
    pub async fn event_count(&self) -> usize {
        self.cache.lock().await.len()
    }

    /// Returns a cloned ordered snapshot of the current cache.
    pub async fn cached_events(&self) -> Vec<RuntimeEventEnvelope> {
        self.cache.lock().await.events().to_vec()
    }

    /// Returns a cloned copy of the current state mirror.
    pub async fn state(&self) -> ConversationStateMirror {
        self.state.lock().await.clone()
    }

    /// Forces an immediate reconcile pass and returns how many new events were added.
    pub async fn reconcile_now(&self) -> Result<usize> {
        let events = self.client.search_events_all(&self.conversation_id).await?;
        Ok(merge_events(&self.cache, &self.state, events).await)
    }

    /// Waits for a terminal execution status using WebSocket updates plus REST fallback.
    pub async fn wait_for_terminal(&mut self, timeout: Duration) -> Result<ConversationInfo> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let current_status = {
                let status = self.status_rx.borrow();
                *status
            };
            if let Some(status) = current_status {
                if status.is_terminal() {
                    let info =
                        refresh_and_snapshot(&self.client, &self.conversation_id, &self.state)
                            .await?;
                    self.best_effort_reconcile_after_terminal().await;
                    return Ok(info);
                }
            }

            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Err(OpenHandsError::Timeout {
                    operation: "terminal execution status",
                    timeout,
                });
            }
            let sleep_for = std::cmp::min(self.websocket.poll_interval(), deadline - now);

            tokio::select! {
                changed = self.status_rx.changed() => {
                    if changed.is_err() {
                        debug!(conversation_id = %self.conversation_id, "status channel closed; relying on REST fallback");
                    }
                }
                _ = tokio::time::sleep(sleep_for) => {
                    let info = refresh_and_snapshot(&self.client, &self.conversation_id, &self.state).await?;
                    if info.execution_status.map(RemoteExecutionStatus::is_terminal).unwrap_or(false) {
                        self.best_effort_reconcile_after_terminal().await;
                        return Ok(info);
                    }
                }
            }
        }
    }

    /// Stops the background reconnect loop and waits for it to exit.
    pub async fn close(mut self) -> Result<()> {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(true);
        }
        if let Some(task) = self.task.take() {
            task.await.map_err(|error| OpenHandsError::Join {
                message: error.to_string(),
            })?;
        }
        Ok(())
    }

    async fn best_effort_reconcile_after_terminal(&self) {
        if let Err(error) = self.reconcile_now().await {
            warn!(
                conversation_id = %self.conversation_id,
                error = %error,
                "terminal reconcile failed after authoritative terminal refresh",
            );
        }
    }
}

impl Drop for AttachedConversation {
    fn drop(&mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(true);
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

struct StreamRuntime {
    client: OpenHandsClient,
    conversation_id: String,
    websocket: WebSocketConfig,
    cache: Arc<Mutex<EventCache>>,
    state: Arc<Mutex<ConversationStateMirror>>,
    status_tx: watch::Sender<Option<RemoteExecutionStatus>>,
    stop_rx: watch::Receiver<bool>,
    socket_url: url::Url,
    stream: EventSocket,
}

impl StreamRuntime {
    async fn run(mut self) {
        let mut delay = self.websocket.reconnect_initial();
        loop {
            match read_until_disconnect(
                &mut self.stream,
                &self.socket_url,
                &self.cache,
                &self.state,
                &self.status_tx,
                &mut self.stop_rx,
            )
            .await
            {
                StreamControl::Reconnect => {}
                StreamControl::Stop => return,
            }

            loop {
                tokio::select! {
                    changed = self.stop_rx.changed() => {
                        if changed.is_err() || *self.stop_rx.borrow() {
                            return;
                        }
                    }
                    _ = tokio::time::sleep(delay) => {}
                }

                let (next_url, request) = match self.client.websocket_request(&self.conversation_id)
                {
                    Ok(result) => result,
                    Err(error) => {
                        warn!(conversation_id = %self.conversation_id, error = %error, "failed to build websocket reconnect request");
                        delay =
                            std::cmp::min(delay.saturating_mul(2), self.websocket.reconnect_max());
                        continue;
                    }
                };
                self.socket_url = next_url;

                match connect_socket_until_stopped(
                    request,
                    &self.socket_url,
                    self.websocket.ready_timeout(),
                    &mut self.stop_rx,
                )
                .await
                {
                    Ok(ConnectControl::Connected(mut next_stream)) => {
                        match wait_for_ready_until_stopped(
                            &mut next_stream,
                            &self.socket_url,
                            self.websocket.ready_timeout(),
                            &self.cache,
                            &self.state,
                            &mut self.stop_rx,
                        )
                        .await
                        {
                            Ok(ReadyControl::Ready) => {
                                match reconcile_and_refresh(
                                    &self.client,
                                    &self.conversation_id,
                                    &self.cache,
                                    &self.state,
                                )
                                .await
                                {
                                    Ok(()) => {
                                        let _ = self
                                            .status_tx
                                            .send(self.state.lock().await.execution_status);
                                        self.stream = next_stream;
                                        delay = self.websocket.reconnect_initial();
                                        break;
                                    }
                                    Err(error) => {
                                        warn!(conversation_id = %self.conversation_id, error = %error, "reconcile-after-reconnect failed");
                                    }
                                }
                            }
                            Ok(ReadyControl::Stop) => return,
                            Err(error) => {
                                warn!(conversation_id = %self.conversation_id, error = %error, "websocket reconnect readiness failed");
                            }
                        }
                    }
                    Ok(ConnectControl::Stop) => return,
                    Err(error) => {
                        warn!(
                            conversation_id = %self.conversation_id,
                            error = %error,
                            "websocket reconnect attempt failed",
                        );
                    }
                }

                delay = std::cmp::min(delay.saturating_mul(2), self.websocket.reconnect_max());
            }
        }
    }
}

async fn connect_socket(
    request: tokio_tungstenite::tungstenite::http::Request<()>,
    socket_url: &url::Url,
    timeout: Duration,
) -> Result<EventSocket> {
    match tokio::time::timeout(timeout, connect_async(request)).await {
        Ok(Ok((stream, _))) => Ok(stream),
        Ok(Err(source)) => Err(OpenHandsError::WebSocket {
            url: socket_url.to_string(),
            source,
        }),
        Err(_) => Err(OpenHandsError::Timeout {
            operation: "websocket handshake",
            timeout,
        }),
    }
}

async fn connect_socket_until_stopped(
    request: tokio_tungstenite::tungstenite::http::Request<()>,
    socket_url: &url::Url,
    timeout: Duration,
    stop_rx: &mut watch::Receiver<bool>,
) -> Result<ConnectControl> {
    let reconnect_request = request.clone();
    tokio::select! {
        changed = stop_rx.changed() => {
            if changed.is_err() || *stop_rx.borrow() {
                Ok(ConnectControl::Stop)
            } else {
                connect_socket(request, socket_url, timeout)
                    .await
                    .map(ConnectControl::Connected)
            }
        }
        result = connect_socket(reconnect_request, socket_url, timeout) => {
            result.map(ConnectControl::Connected)
        }
    }
}

async fn wait_for_ready(
    stream: &mut EventSocket,
    socket_url: &url::Url,
    timeout: Duration,
    cache: &Arc<Mutex<EventCache>>,
    state: &Arc<Mutex<ConversationStateMirror>>,
) -> Result<()> {
    let ready = tokio::time::timeout(timeout, async {
        loop {
            match stream.next().await {
                Some(Ok(message)) => {
                    if let Some(event) = decode_message(message, socket_url).await? {
                        let is_ready_event = matches!(
                            event.payload,
                            RuntimeEventPayload::ConversationStateUpdate(_)
                        );
                        apply_event(cache, state, &event).await;
                        if is_ready_event {
                            return Ok(());
                        }
                    }
                }
                Some(Err(source)) => {
                    return Err(OpenHandsError::WebSocket {
                        url: socket_url.to_string(),
                        source,
                    });
                }
                None => {
                    return Err(OpenHandsError::Protocol {
                        message: "websocket closed before readiness event".to_string(),
                    });
                }
            }
        }
    })
    .await;

    match ready {
        Ok(result) => result,
        Err(_) => Err(OpenHandsError::Timeout {
            operation: "websocket readiness barrier",
            timeout,
        }),
    }
}

async fn wait_for_ready_until_stopped(
    stream: &mut EventSocket,
    socket_url: &url::Url,
    timeout: Duration,
    cache: &Arc<Mutex<EventCache>>,
    state: &Arc<Mutex<ConversationStateMirror>>,
    stop_rx: &mut watch::Receiver<bool>,
) -> Result<ReadyControl> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(OpenHandsError::Timeout {
                operation: "websocket readiness barrier",
                timeout,
            });
        }
        let remaining = deadline - now;

        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() {
                    return Ok(ReadyControl::Stop);
                }
            }
            next = stream.next() => {
                match next {
                    Some(Ok(message)) => {
                        if let Some(event) = decode_message(message, socket_url).await? {
                            let is_ready_event = matches!(
                                event.payload,
                                RuntimeEventPayload::ConversationStateUpdate(_)
                            );
                            apply_event(cache, state, &event).await;
                            if is_ready_event {
                                return Ok(ReadyControl::Ready);
                            }
                        }
                    }
                    Some(Err(source)) => {
                        return Err(OpenHandsError::WebSocket {
                            url: socket_url.to_string(),
                            source,
                        });
                    }
                    None => {
                        return Err(OpenHandsError::Protocol {
                            message: "websocket closed before readiness event".to_string(),
                        });
                    }
                }
            }
            _ = tokio::time::sleep(remaining) => {
                return Err(OpenHandsError::Timeout {
                    operation: "websocket readiness barrier",
                    timeout,
                });
            }
        }
    }
}

async fn read_until_disconnect(
    stream: &mut EventSocket,
    socket_url: &url::Url,
    cache: &Arc<Mutex<EventCache>>,
    state: &Arc<Mutex<ConversationStateMirror>>,
    status_tx: &watch::Sender<Option<RemoteExecutionStatus>>,
    stop_rx: &mut watch::Receiver<bool>,
) -> StreamControl {
    loop {
        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() {
                    return StreamControl::Stop;
                }
            }
            next = stream.next() => {
                match next {
                    Some(Ok(message)) => {
                        match decode_message(message, socket_url).await {
                            Ok(Some(event)) => {
                                apply_event(cache, state, &event).await;
                                let _ = status_tx.send(state.lock().await.execution_status);
                            }
                            Ok(None) => {}
                            Err(error) => {
                                warn!(error = %error, "dropping malformed websocket event");
                            }
                        }
                    }
                    Some(Err(source)) => {
                        warn!(
                            error = %OpenHandsError::WebSocket { url: socket_url.to_string(), source },
                            "websocket stream disconnected",
                        );
                        return StreamControl::Reconnect;
                    }
                    None => return StreamControl::Reconnect,
                }
            }
        }
    }
}

async fn decode_message(
    message: tokio_tungstenite::tungstenite::Message,
    socket_url: &url::Url,
) -> Result<Option<RuntimeEventEnvelope>> {
    match message {
        tokio_tungstenite::tungstenite::Message::Text(text) => {
            let raw_json: Value = serde_json::from_str(&text)?;
            Ok(Some(RuntimeEventEnvelope::from_json(raw_json)?))
        }
        tokio_tungstenite::tungstenite::Message::Binary(binary) => {
            let raw_json: Value = serde_json::from_slice(&binary)?;
            Ok(Some(RuntimeEventEnvelope::from_json(raw_json)?))
        }
        tokio_tungstenite::tungstenite::Message::Close(_) => Err(OpenHandsError::Protocol {
            message: format!("websocket {socket_url} closed"),
        }),
        tokio_tungstenite::tungstenite::Message::Ping(_)
        | tokio_tungstenite::tungstenite::Message::Pong(_)
        | tokio_tungstenite::tungstenite::Message::Frame(_) => Ok(None),
    }
}

async fn merge_events(
    cache: &Arc<Mutex<EventCache>>,
    state: &Arc<Mutex<ConversationStateMirror>>,
    events: Vec<RuntimeEventEnvelope>,
) -> usize {
    let mut added = 0;
    for event in events {
        if apply_event(cache, state, &event).await {
            added += 1;
        }
    }
    added
}

async fn apply_event(
    cache: &Arc<Mutex<EventCache>>,
    state: &Arc<Mutex<ConversationStateMirror>>,
    event: &RuntimeEventEnvelope,
) -> bool {
    let inserted = {
        let mut cache = cache.lock().await;
        cache.insert(event.clone())
    };
    if inserted {
        let mut mirror = state.lock().await;
        mirror.apply_event(event);
    }
    inserted
}

async fn reconcile_and_refresh(
    client: &OpenHandsClient,
    conversation_id: &str,
    cache: &Arc<Mutex<EventCache>>,
    state: &Arc<Mutex<ConversationStateMirror>>,
) -> Result<()> {
    let events = client.search_events_all(conversation_id).await?;
    let _ = merge_events(cache, state, events).await;
    let _ = refresh_and_snapshot(client, conversation_id, state).await?;
    Ok(())
}

async fn refresh_and_snapshot(
    client: &OpenHandsClient,
    conversation_id: &str,
    state: &Arc<Mutex<ConversationStateMirror>>,
) -> Result<ConversationInfo> {
    let info = client
        .get_conversation(conversation_id)
        .await?
        .ok_or_else(|| OpenHandsError::ConversationNotFound {
            conversation_id: conversation_id.to_string(),
        })?;
    {
        let mut mirror = state.lock().await;
        mirror.refresh_from_conversation(&info);
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state_update_event(
        id: &str,
        timestamp: &str,
        key: &str,
        value: Value,
    ) -> RuntimeEventEnvelope {
        RuntimeEventEnvelope::from_json(json!({
            "id": id,
            "timestamp": timestamp,
            "source": "environment",
            "kind": "ConversationStateUpdateEvent",
            "key": key,
            "value": value,
        }))
        .expect("state update event should decode")
    }

    #[tokio::test]
    async fn out_of_order_execution_status_does_not_rewind_the_mirror() {
        let cache = Arc::new(Mutex::new(EventCache::default()));
        let state = Arc::new(Mutex::new(ConversationStateMirror::default()));

        let finished = state_update_event(
            "finished",
            "2026-03-21T15:00:02Z",
            "execution_status",
            json!(RemoteExecutionStatus::Finished),
        );
        let running = state_update_event(
            "running",
            "2026-03-21T15:00:01Z",
            "execution_status",
            json!(RemoteExecutionStatus::Running),
        );

        assert!(apply_event(&cache, &state, &finished).await);
        assert!(apply_event(&cache, &state, &running).await);

        let mirror = state.lock().await.clone();
        assert_eq!(
            mirror.execution_status,
            Some(RemoteExecutionStatus::Finished)
        );
    }

    #[tokio::test]
    async fn offset_timestamps_do_not_rewind_the_mirror() {
        let cache = Arc::new(Mutex::new(EventCache::default()));
        let state = Arc::new(Mutex::new(ConversationStateMirror::default()));

        let finished = state_update_event(
            "finished",
            "2026-03-21T15:00:02Z",
            "execution_status",
            json!(RemoteExecutionStatus::Finished),
        );
        let running = state_update_event(
            "running",
            "2026-03-21T16:00:01+01:00",
            "execution_status",
            json!(RemoteExecutionStatus::Running),
        );

        assert!(apply_event(&cache, &state, &finished).await);
        assert!(apply_event(&cache, &state, &running).await);

        let mirror = state.lock().await.clone();
        assert_eq!(
            mirror.execution_status,
            Some(RemoteExecutionStatus::Finished)
        );
    }
}
