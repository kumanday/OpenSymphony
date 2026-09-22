//! Private host-session observation seam for a separate debug process.
//! Prompt authority and scheduler holds are deliberately owned by the later control-handoff layer.
use crate::opensymphony_acp::{
    ControlResult, HostError, SessionControl, SessionEvent, SessionHost,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use serde::Deserialize;
use std::{convert::Infallible, sync::Arc, time::Duration};

#[derive(Clone)]
struct HostState {
    host: SessionHost,
    bearer: Arc<str>,
}
#[derive(Deserialize)]
struct Binding {
    generation: u64,
}
#[derive(Deserialize)]
struct Command {
    generation: u64,
    #[serde(flatten)]
    action: SessionControl,
}

pub(super) fn router(host: SessionHost, bearer: String) -> Result<Router, &'static str> {
    if bearer.len() < 32 || bearer.chars().any(char::is_whitespace) {
        return Err("ACP control bearer must contain at least 32 non-whitespace characters");
    }
    Ok(Router::new()
        .route("/api/v1/acp/{owner_id}", post(command))
        .route("/api/v1/acp/{owner_id}/events", get(events))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(HostState {
            host,
            bearer: bearer.into(),
        }))
}
fn authorize(headers: &HeaderMap, state: &HostState) -> Result<(), StatusCode> {
    let provided = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    // Check every byte when lengths agree. Tokens never appear in URLs or errors.
    if provided.len() != state.bearer.len()
        || provided
            .bytes()
            .zip(state.bearer.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            != 0
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}
fn status(error: HostError) -> StatusCode {
    match error {
        HostError::IdentityMismatch => StatusCode::CONFLICT,
        HostError::Busy => StatusCode::CONFLICT,
        HostError::ResourceLimit => StatusCode::TOO_MANY_REQUESTS,
        HostError::Unavailable => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
async fn command(
    State(state): State<HostState>,
    Path(owner_id): Path<String>,
    headers: HeaderMap,
    Json(command): Json<Command>,
) -> Result<Json<ControlResult>, StatusCode> {
    authorize(&headers, &state)?;
    // Retirement is a host/scheduler lifecycle command; observation cannot delete context.
    if matches!(command.action, SessionControl::Retire) {
        return Err(StatusCode::FORBIDDEN);
    }
    let handle = state
        .host
        .lookup(owner_id, command.generation)
        .await
        .map_err(status)?;
    handle
        .control(command.action)
        .await
        .map(Json)
        .map_err(status)
}
async fn events(
    State(state): State<HostState>,
    Path(owner_id): Path<String>,
    Query(binding): Query<Binding>,
    headers: HeaderMap,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    authorize(&headers, &state)?;
    let handle = state
        .host
        .lookup(owner_id, binding.generation)
        .await
        .map_err(status)?;
    let mut events = handle.subscribe();
    let initial = handle.inspect().await.map_err(status)?;
    let history = handle.source_history();
    let last_sequence = history
        .events
        .iter()
        .filter_map(|event| match event {
            SessionEvent::Source { frame, .. } => Some(frame.sequence),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let stream = async_stream::stream! {
        yield Ok(Event::default().event("state").json_data(SessionEvent::State { snapshot: Box::new(initial) }).expect("serializable snapshot"));
        if history.truncated { yield Ok(Event::default().event("gap").data("retained source history is incomplete")); }
        for event in history.events { yield Ok(Event::default().event("session").json_data(event).expect("serializable source event")); }
        loop {
            match events.recv().await {
                Ok(event) => {
                    if matches!(&event, SessionEvent::Source { frame, .. } if frame.sequence <= last_sequence) { continue; }
                    let ended = matches!(event, SessionEvent::Ended { .. });
                    yield Ok(Event::default().event("session").json_data(event).expect("serializable session event"));
                    if ended { break; }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Source events have no agent cursor; do not silently pretend loss is replayable.
                    yield Ok(Event::default().event("gap").data("subscriber lagged; reattach and inspect session state"));
                    break;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}
