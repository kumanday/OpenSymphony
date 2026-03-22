use std::{
    cmp::{max, min},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use ftui::{
    core::geometry::Rect,
    prelude::{App, Cmd, Event, Frame, KeyCode, Model, ScreenMode},
    runtime::{Every, Subscription},
    widgets::{Widget, paragraph::Paragraph},
};
use opensymphony_control::{
    ControlPlaneClient, ControlPlaneClientError, ControlPlaneStreamUpdate, IssueSnapshot,
    MetricsSnapshot, RecentEvent, SnapshotEnvelope, log_stream_error,
};
use thiserror::Error;
use url::Url;

const CONTROL_PLANE_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);
const CONTROL_PLANE_STREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CONTROL_PLANE_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(45);
const CONTROL_PLANE_RETRY_DELAY: Duration = Duration::from_millis(750);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiState {
    pub focus: FocusPane,
    pub timeline_mode: TimelineMode,
    pub connection: ConnectionState,
    pub selected_issue: usize,
    pub latest_snapshot: Option<SnapshotEnvelope>,
    pub status_line: String,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            focus: FocusPane::Issues,
            timeline_mode: TimelineMode::Events,
            connection: ConnectionState::Connecting,
            selected_issue: 0,
            latest_snapshot: None,
            status_line: "connecting to control plane".to_owned(),
        }
    }
}

impl TuiState {
    pub fn reduce(&mut self, action: TuiAction) {
        match action {
            TuiAction::BootstrapSnapshotReceived(envelope) => {
                self.apply_snapshot(*envelope);
            }
            TuiAction::SnapshotReceived(envelope) => {
                if !self.apply_snapshot(*envelope) {
                    return;
                }
                self.connection = ConnectionState::Live;
                self.status_line = "live control-plane stream".to_owned();
            }
            TuiAction::ConnectionLost(reason) => {
                self.connection = ConnectionState::Reconnecting(reason.clone());
                self.status_line = format!("reconnecting after: {reason}");
            }
            TuiAction::MoveSelectionUp => {
                self.selected_issue = self.selected_issue.saturating_sub(1);
            }
            TuiAction::MoveSelectionDown => {
                let count = self.issue_count();
                if count > 0 {
                    self.selected_issue = min(self.selected_issue + 1, count - 1);
                }
            }
            TuiAction::FocusNext => {
                self.focus = match self.focus {
                    FocusPane::Issues => FocusPane::Detail,
                    FocusPane::Detail => FocusPane::Timeline,
                    FocusPane::Timeline => FocusPane::Issues,
                };
            }
            TuiAction::ToggleTimelineMode => {
                self.timeline_mode = match self.timeline_mode {
                    TimelineMode::Events => TimelineMode::Metrics,
                    TimelineMode::Metrics => TimelineMode::Events,
                };
            }
        }
    }

    pub fn render_text(&self, width: usize, height: usize) -> String {
        if width == 0 || height == 0 {
            return String::new();
        }

        let (body_rows, timeline_rows) = section_layout(height);
        let mut lines = Vec::new();
        let snapshot = self.latest_snapshot.as_ref();
        let issue_count = snapshot
            .map(|value| value.snapshot.issues.len())
            .unwrap_or_default();
        let sequence = snapshot.map(|value| value.sequence).unwrap_or_default();
        let generated = snapshot
            .map(|value| format_timestamp(value.snapshot.generated_at))
            .unwrap_or_else(|| "--:--:--".to_owned());
        let status = format!(
            "OpenSymphony | focus={} | bottom={} | conn={} | status={} | seq={} | issues={} | updated={} | q quit  tab focus  e toggle",
            self.focus.label(),
            self.timeline_mode.label(),
            self.connection.label(),
            self.status_line,
            sequence,
            issue_count,
            generated
        );
        lines.push(fit(&status, width));
        lines.push("=".repeat(width));

        if width >= 80 {
            let left_width = max(26, width * 2 / 5);
            let right_width = width.saturating_sub(left_width + 3);
            let left = self.issue_lines(left_width, body_rows);
            let right = self.detail_lines(right_width);
            lines.extend(fit_section(
                two_column_block(&left, &right, left_width, right_width),
                body_rows,
                width,
            ));
        } else {
            let (issue_rows, detail_rows) = stacked_section_layout(body_rows);
            lines.extend(fit_section(
                self.issue_lines(width, issue_rows),
                issue_rows,
                width,
            ));
            if detail_rows > 0 {
                lines.push("-".repeat(width));
                lines.extend(fit_section(self.detail_lines(width), detail_rows, width));
            }
        }

        if timeline_rows > 0 {
            lines.push("=".repeat(width));
            lines.extend(fit_section(
                self.timeline_lines(width),
                timeline_rows,
                width,
            ));
        }

        if lines.len() > height {
            lines.truncate(height);
        }
        while lines.len() < height {
            lines.push(" ".repeat(width));
        }
        lines.join("\n")
    }

    fn issue_lines(&self, width: usize, max_rows: usize) -> Vec<String> {
        if max_rows == 0 {
            return Vec::new();
        }

        let mut lines = vec![fit(
            &pane_title("ISSUES", self.focus == FocusPane::Issues),
            width,
        )];
        match &self.latest_snapshot {
            Some(snapshot) if snapshot.snapshot.issues.is_empty() => {
                lines.push(fit("no issues in snapshot", width));
            }
            Some(snapshot) => {
                let visible_issues = max_rows.saturating_sub(1) / 2;
                if visible_issues == 0 {
                    return lines;
                }

                let (start, end) = visible_issue_window(
                    snapshot.snapshot.issues.len(),
                    visible_issues,
                    self.selected_issue,
                );
                for (index, issue) in snapshot.snapshot.issues[start..end].iter().enumerate() {
                    let absolute_index = start + index;
                    let marker = if absolute_index == self.selected_issue {
                        ">"
                    } else {
                        " "
                    };
                    let line = format!(
                        "{marker} {} [{} / {}]",
                        issue.identifier,
                        issue.runtime_state.as_str(),
                        issue.tracker_state
                    );
                    lines.push(fit(&line, width));
                    lines.push(fit(&format!("  {}", issue.title), width));
                }
            }
            None => {
                lines.push(fit("awaiting first snapshot", width));
            }
        }
        lines
    }

    fn detail_lines(&self, width: usize) -> Vec<String> {
        let mut lines = vec![fit(
            &pane_title("ISSUE + WORKSPACE DETAIL", self.focus == FocusPane::Detail),
            width,
        )];
        match self.selected_issue() {
            Some(issue) => {
                lines.push(fit(&format!("{} {}", issue.identifier, issue.title), width));
                lines.push(fit(
                    &format!("workspace path: {}", issue.workspace_path_suffix),
                    width,
                ));
                lines.push(fit(
                    &format!("conversation id: {}", issue.conversation_id_suffix),
                    width,
                ));
                lines.push(fit(
                    &format!(
                        "tracker: {} | runtime: {}",
                        issue.tracker_state,
                        issue.runtime_state.as_str()
                    ),
                    width,
                ));
                lines.push(fit(
                    &format!(
                        "outcome: {} | retries: {} | blocked: {}",
                        issue.last_outcome.as_str(),
                        issue.retry_count,
                        issue.blocked
                    ),
                    width,
                ));
                lines.push(fit(
                    &format!("last event: {}", format_timestamp(issue.last_event_at)),
                    width,
                ));
            }
            None => {
                lines.push(fit("no selected issue", width));
            }
        }
        lines
    }

    fn timeline_lines(&self, width: usize) -> Vec<String> {
        let title = match self.timeline_mode {
            TimelineMode::Events => "RECENT EVENTS",
            TimelineMode::Metrics => "METRICS",
        };
        let mut lines = vec![fit(
            &pane_title(title, self.focus == FocusPane::Timeline),
            width,
        )];
        match (&self.timeline_mode, &self.latest_snapshot) {
            (_, None) => lines.push(fit("waiting for stream data", width)),
            (TimelineMode::Events, Some(snapshot)) => {
                lines.extend(event_lines(&snapshot.snapshot.recent_events, width));
            }
            (TimelineMode::Metrics, Some(snapshot)) => {
                lines.extend(metric_lines(&snapshot.snapshot.metrics, width));
            }
        }
        lines
    }

    fn selected_issue(&self) -> Option<&IssueSnapshot> {
        self.latest_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.snapshot.issues.get(self.selected_issue))
    }

    fn issue_count(&self) -> usize {
        self.latest_snapshot
            .as_ref()
            .map(|snapshot| snapshot.snapshot.issues.len())
            .unwrap_or_default()
    }

    fn clamp_selection(&mut self) {
        let count = self.issue_count();
        if count == 0 {
            self.selected_issue = 0;
        } else {
            self.selected_issue = min(self.selected_issue, count - 1);
        }
    }

    fn restore_selection(&mut self, selected_identifier: Option<&str>) {
        if let Some(index) = selected_identifier.and_then(|identifier| {
            self.latest_snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .snapshot
                    .issues
                    .iter()
                    .position(|issue| issue.identifier == identifier)
            })
        }) {
            self.selected_issue = index;
        } else {
            self.clamp_selection();
        }
    }

    fn apply_snapshot(&mut self, envelope: SnapshotEnvelope) -> bool {
        if !self.should_accept_snapshot(&envelope) {
            return false;
        }

        let selected_identifier = self.selected_issue().map(|issue| issue.identifier.clone());
        self.latest_snapshot = Some(envelope);
        self.restore_selection(selected_identifier.as_deref());
        true
    }

    fn should_accept_snapshot(&self, incoming: &SnapshotEnvelope) -> bool {
        let Some(current) = self.latest_snapshot.as_ref() else {
            return true;
        };

        if incoming.sequence >= current.sequence {
            return true;
        }

        incoming.published_at > current.published_at
            && incoming.snapshot.generated_at > current.snapshot.generated_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    Issues,
    Detail,
    Timeline,
}

impl FocusPane {
    fn label(&self) -> &'static str {
        match self {
            FocusPane::Issues => "issues",
            FocusPane::Detail => "detail",
            FocusPane::Timeline => "timeline",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineMode {
    Events,
    Metrics,
}

impl TimelineMode {
    fn label(&self) -> &'static str {
        match self {
            TimelineMode::Events => "events",
            TimelineMode::Metrics => "metrics",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Live,
    Reconnecting(String),
}

impl ConnectionState {
    fn label(&self) -> &str {
        match self {
            ConnectionState::Connecting => "connecting",
            ConnectionState::Live => "live",
            ConnectionState::Reconnecting(_) => "reconnecting",
        }
    }
}

#[derive(Debug, Clone)]
pub enum TuiAction {
    BootstrapSnapshotReceived(Box<SnapshotEnvelope>),
    SnapshotReceived(Box<SnapshotEnvelope>),
    ConnectionLost(String),
    MoveSelectionUp,
    MoveSelectionDown,
    FocusNext,
    ToggleTimelineMode,
}

#[derive(Debug, Default)]
struct BridgeMailbox {
    latest_bootstrap_snapshot: Option<Box<SnapshotEnvelope>>,
    latest_snapshot: Option<Box<SnapshotEnvelope>>,
    latest_connection_loss: Option<String>,
}

impl BridgeMailbox {
    fn push_bootstrap_snapshot(&mut self, snapshot: SnapshotEnvelope) {
        self.latest_bootstrap_snapshot = Some(Box::new(snapshot));
    }

    fn push_snapshot(&mut self, snapshot: SnapshotEnvelope) {
        self.latest_bootstrap_snapshot = None;
        self.latest_snapshot = Some(Box::new(snapshot));
    }

    fn push_connection_loss(&mut self, reason: String) {
        // Any queued streamed snapshot was observed before the disconnect and would
        // incorrectly flip the reducer back to `live` on the next UI tick.
        self.latest_snapshot = None;
        self.latest_connection_loss = Some(reason);
    }

    fn take_action(&mut self) -> Option<TuiAction> {
        if let Some(reason) = self.latest_connection_loss.take() {
            return Some(TuiAction::ConnectionLost(reason));
        }

        if let Some(snapshot) = self.latest_snapshot.take() {
            return Some(TuiAction::SnapshotReceived(snapshot));
        }

        if let Some(snapshot) = self.latest_bootstrap_snapshot.take() {
            return Some(TuiAction::BootstrapSnapshotReceived(snapshot));
        }

        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum ScriptedExitOutcome {
    #[default]
    Pending,
    Succeeded,
    Failed(String),
}

#[derive(Debug, Clone, Copy)]
enum AppMessage {
    Tick,
    MoveSelectionUp,
    MoveSelectionDown,
    FocusNext,
    ToggleTimelineMode,
    Quit,
}

impl From<Event> for AppMessage {
    fn from(event: Event) -> Self {
        match event {
            Event::Key(key) => match key.code {
                KeyCode::Char('q') => AppMessage::Quit,
                KeyCode::Char('k') | KeyCode::Up => AppMessage::MoveSelectionUp,
                KeyCode::Char('j') | KeyCode::Down => AppMessage::MoveSelectionDown,
                KeyCode::Tab => AppMessage::FocusNext,
                KeyCode::Char('e') => AppMessage::ToggleTimelineMode,
                _ => AppMessage::Tick,
            },
            _ => AppMessage::Tick,
        }
    }
}

pub fn run_operator(base_url: Url, exit_after: Option<Duration>) -> Result<(), TuiError> {
    let bridge = Arc::new(Mutex::new(BridgeMailbox::default()));
    let scripted_exit = Arc::new(Mutex::new(ScriptedExitOutcome::Pending));
    spawn_bridge(base_url, Arc::clone(&bridge));
    let app = OperatorApp::new(bridge, exit_after, Arc::clone(&scripted_exit));
    App::new(app)
        .screen_mode(ScreenMode::Inline { ui_height: 22 })
        .run()
        .map_err(TuiError::Runtime)?;

    let outcome = scripted_exit
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    match outcome {
        ScriptedExitOutcome::Pending | ScriptedExitOutcome::Succeeded => Ok(()),
        ScriptedExitOutcome::Failed(last_status) => {
            Err(TuiError::ControlPlaneUnavailable(last_status))
        }
    }
}

#[derive(Debug)]
struct OperatorApp {
    state: TuiState,
    bridge: Arc<Mutex<BridgeMailbox>>,
    exit_after: Option<Duration>,
    started_at: Instant,
    scripted_exit: Arc<Mutex<ScriptedExitOutcome>>,
}

impl OperatorApp {
    fn new(
        bridge: Arc<Mutex<BridgeMailbox>>,
        exit_after: Option<Duration>,
        scripted_exit: Arc<Mutex<ScriptedExitOutcome>>,
    ) -> Self {
        Self {
            state: TuiState::default(),
            bridge,
            exit_after,
            started_at: Instant::now(),
            scripted_exit,
        }
    }

    fn drain_bridge(&mut self) {
        let mut bridge = self
            .bridge
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while let Some(action) = bridge.take_action() {
            let stop_after_render = matches!(action, TuiAction::ConnectionLost(_));
            self.state.reduce(action);
            if stop_after_render {
                break;
            }
        }
    }

    fn record_scripted_exit(&self) {
        let outcome = if self.state.connection == ConnectionState::Live {
            ScriptedExitOutcome::Succeeded
        } else {
            ScriptedExitOutcome::Failed(format!("last status: {}", self.state.status_line))
        };
        let mut scripted_exit = self
            .scripted_exit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *scripted_exit = outcome;
    }
}

fn spawn_bridge(base_url: Url, bridge: Arc<Mutex<BridgeMailbox>>) {
    thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                push_connection_loss(&bridge, error.to_string());
                return;
            }
        };

        runtime.block_on(async move {
            let retry_delay = CONTROL_PLANE_RETRY_DELAY;
            let client = ControlPlaneClient::with_timeouts(
                base_url,
                CONTROL_PLANE_SNAPSHOT_TIMEOUT,
                CONTROL_PLANE_STREAM_CONNECT_TIMEOUT,
                CONTROL_PLANE_STREAM_IDLE_TIMEOUT,
            );
            loop {
                match client.fetch_snapshot().await {
                    Ok(snapshot) => {
                        push_bootstrap_snapshot(&bridge, snapshot);
                    }
                    Err(error) => {
                        push_connection_loss(&bridge, error.to_string());
                        tokio::time::sleep(retry_delay).await;
                        continue;
                    }
                }

                let mut stream = match client.stream_updates() {
                    Ok(stream) => stream,
                    Err(error) => {
                        push_connection_loss(&bridge, error.to_string());
                        tokio::time::sleep(retry_delay).await;
                        continue;
                    }
                };

                let mut should_retry = false;
                while let Some(update) = stream.next().await {
                    match update {
                        Ok(stream_update) => {
                            handle_stream_update(&bridge, stream_update);
                        }
                        Err(error) => {
                            handle_bridge_error(&bridge, &error);
                            should_retry = true;
                            break;
                        }
                    }
                }

                stream.close();
                if !should_retry {
                    push_connection_loss(&bridge, "control-plane stream closed".to_owned());
                }
                tokio::time::sleep(retry_delay).await;
            }
        });
    });
}

impl Model for OperatorApp {
    type Message = AppMessage;

    fn update(&mut self, message: Self::Message) -> Cmd<Self::Message> {
        self.drain_bridge();
        match message {
            AppMessage::Tick => {}
            AppMessage::MoveSelectionUp => self.state.reduce(TuiAction::MoveSelectionUp),
            AppMessage::MoveSelectionDown => self.state.reduce(TuiAction::MoveSelectionDown),
            AppMessage::FocusNext => self.state.reduce(TuiAction::FocusNext),
            AppMessage::ToggleTimelineMode => self.state.reduce(TuiAction::ToggleTimelineMode),
            AppMessage::Quit => return Cmd::quit(),
        }

        if self
            .exit_after
            .is_some_and(|limit| self.started_at.elapsed() >= limit)
        {
            self.record_scripted_exit();
            return Cmd::quit();
        }

        Cmd::none()
    }

    fn view(&self, frame: &mut Frame<'_>) {
        let content = self
            .state
            .render_text(frame.width() as usize, frame.height() as usize);
        Paragraph::new(content).render(Rect::new(0, 0, frame.width(), frame.height()), frame);
    }

    fn subscriptions(&self) -> Vec<Box<dyn Subscription<Self::Message>>> {
        vec![Box::new(Every::new(Duration::from_millis(250), || {
            AppMessage::Tick
        }))]
    }
}

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("failed to render FrankenTUI runtime: {0}")]
    Runtime(std::io::Error),
    #[error("control plane never reached a live stream before --exit-after-ms elapsed ({0})")]
    ControlPlaneUnavailable(String),
}

fn handle_bridge_error(bridge: &Arc<Mutex<BridgeMailbox>>, error: &ControlPlaneClientError) {
    log_stream_error(error);
    push_connection_loss(bridge, error.to_string());
}

fn handle_stream_update(bridge: &Arc<Mutex<BridgeMailbox>>, update: ControlPlaneStreamUpdate) {
    match update {
        ControlPlaneStreamUpdate::Snapshot(snapshot) => push_snapshot(bridge, snapshot),
        ControlPlaneStreamUpdate::Reconnecting(error) => handle_bridge_error(bridge, &error),
    }
}

fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.format("%H:%M:%S").to_string()
}

fn pane_title(title: &str, focused: bool) -> String {
    let marker = if focused { "[x]" } else { "[ ]" };
    format!("{marker} {title}")
}

fn event_lines(events: &[RecentEvent], width: usize) -> Vec<String> {
    if events.is_empty() {
        return vec![fit("no recent events", width)];
    }

    events
        .iter()
        .map(|event| {
            let scope = event.issue_identifier.as_deref().unwrap_or("daemon");
            fit(
                &format!(
                    "{} {} {}",
                    format_timestamp(event.happened_at),
                    scope,
                    event.summary
                ),
                width,
            )
        })
        .collect()
}

fn metric_lines(metrics: &MetricsSnapshot, width: usize) -> Vec<String> {
    vec![
        fit(
            &format!("running issues: {}", metrics.running_issues),
            width,
        ),
        fit(
            &format!("retry queue depth: {}", metrics.retry_queue_depth),
            width,
        ),
        fit(&format!("total tokens: {}", metrics.total_tokens), width),
        fit(
            &format!(
                "total cost: ${:.4}",
                metrics.total_cost_micros as f64 / 1_000_000.0
            ),
            width,
        ),
    ]
}

fn two_column_block(
    left: &[String],
    right: &[String],
    left_width: usize,
    right_width: usize,
) -> Vec<String> {
    let row_count = max(left.len(), right.len());
    (0..row_count)
        .map(|index| {
            format!(
                "{} | {}",
                fit(
                    left.get(index).map(String::as_str).unwrap_or(""),
                    left_width
                ),
                fit(
                    right.get(index).map(String::as_str).unwrap_or(""),
                    right_width
                ),
            )
        })
        .collect()
}

fn section_layout(height: usize) -> (usize, usize) {
    const HEADER_ROWS: usize = 2;
    const TIMELINE_SEPARATOR_ROWS: usize = 1;
    const MIN_TIMELINE_ROWS: usize = 4;

    if height <= HEADER_ROWS {
        return (0, 0);
    }

    let available = height.saturating_sub(HEADER_ROWS);
    if available <= TIMELINE_SEPARATOR_ROWS {
        return (available, 0);
    }

    let max_timeline_rows = available.saturating_sub(TIMELINE_SEPARATOR_ROWS + 1);
    let timeline_rows = min(max(MIN_TIMELINE_ROWS, available / 3), max_timeline_rows);
    let body_rows = available.saturating_sub(TIMELINE_SEPARATOR_ROWS + timeline_rows);
    (body_rows, timeline_rows)
}

fn stacked_section_layout(body_rows: usize) -> (usize, usize) {
    const DETAIL_SEPARATOR_ROWS: usize = 1;
    const MIN_DETAIL_ROWS: usize = 6;

    if body_rows <= DETAIL_SEPARATOR_ROWS {
        return (body_rows, 0);
    }

    let max_detail_rows = body_rows.saturating_sub(DETAIL_SEPARATOR_ROWS + 1);
    if max_detail_rows == 0 {
        return (body_rows, 0);
    }

    let detail_rows = min(max(MIN_DETAIL_ROWS, body_rows / 2), max_detail_rows);
    let issue_rows = body_rows.saturating_sub(DETAIL_SEPARATOR_ROWS + detail_rows);
    (issue_rows, detail_rows)
}

fn fit_section(mut lines: Vec<String>, max_rows: usize, width: usize) -> Vec<String> {
    if max_rows == 0 {
        return Vec::new();
    }

    if lines.len() > max_rows {
        lines.truncate(max_rows);
        if let Some(last) = lines.last_mut() {
            *last = fit("...", width);
        }
    }

    while lines.len() < max_rows {
        lines.push(" ".repeat(width));
    }

    lines
}

fn push_snapshot(bridge: &Arc<Mutex<BridgeMailbox>>, snapshot: SnapshotEnvelope) {
    let mut bridge = bridge
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    bridge.push_snapshot(snapshot);
}

fn push_bootstrap_snapshot(bridge: &Arc<Mutex<BridgeMailbox>>, snapshot: SnapshotEnvelope) {
    let mut bridge = bridge
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    bridge.push_bootstrap_snapshot(snapshot);
}

fn push_connection_loss(bridge: &Arc<Mutex<BridgeMailbox>>, reason: String) {
    let mut bridge = bridge
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    bridge.push_connection_loss(reason);
}

fn fit(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let value_length = value.chars().count();
    if value_length == width {
        return value.to_owned();
    }
    if value_length < width {
        return format!("{value:<width$}");
    }

    if width == 1 {
        return "~".to_owned();
    }

    let mut shortened = value.chars().take(width - 1).collect::<String>();
    shortened.push('~');
    shortened
}

fn visible_issue_window(
    total_issues: usize,
    visible_issues: usize,
    selected_issue: usize,
) -> (usize, usize) {
    if total_issues <= visible_issues {
        return (0, total_issues);
    }

    let max_start = total_issues.saturating_sub(visible_issues);
    let start = min(selected_issue.saturating_sub(visible_issues / 2), max_start);
    let end = min(start + visible_issues, total_issues);
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::{
        BridgeMailbox, ConnectionState, OperatorApp, ScriptedExitOutcome, TuiState, section_layout,
        stacked_section_layout,
    };
    use chrono::{TimeZone, Utc};
    use opensymphony_control::{
        AgentServerStatus, ControlPlaneClientError, ControlPlaneStreamUpdate, DaemonSnapshot,
        DaemonState, DaemonStatus, IssueRuntimeState, IssueSnapshot, MetricsSnapshot, RecentEvent,
        RecentEventKind, SnapshotEnvelope, WorkerOutcome,
    };
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn fixture(issue_count: usize) -> SnapshotEnvelope {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 21, 20, 0, 0)
            .single()
            .expect("fixture timestamp should be valid");
        SnapshotEnvelope {
            sequence: 8,
            published_at: now,
            snapshot: DaemonSnapshot {
                generated_at: now,
                daemon: DaemonStatus {
                    state: DaemonState::Ready,
                    last_poll_at: now,
                    workspace_root: "/tmp/opensymphony".to_owned(),
                    status_line: "ready".to_owned(),
                },
                agent_server: AgentServerStatus {
                    reachable: true,
                    base_url: "http://127.0.0.1:3000".to_owned(),
                    conversation_count: issue_count as u32,
                    status_line: "healthy".to_owned(),
                },
                metrics: MetricsSnapshot {
                    running_issues: 1,
                    retry_queue_depth: 0,
                    total_tokens: 1024,
                    total_cost_micros: 50_000,
                },
                issues: (0..issue_count)
                    .map(|index| IssueSnapshot {
                        identifier: format!("COE-{}", 255 + index),
                        title: format!("Issue {index}"),
                        tracker_state: "In Progress".to_owned(),
                        runtime_state: IssueRuntimeState::Running,
                        last_outcome: WorkerOutcome::Running,
                        last_event_at: now,
                        conversation_id_suffix: format!("conv-{index}"),
                        workspace_path_suffix: format!("workspace-{index}"),
                        retry_count: index as u32,
                        blocked: false,
                    })
                    .collect(),
                recent_events: vec![RecentEvent {
                    happened_at: now,
                    issue_identifier: Some("COE-255".to_owned()),
                    kind: RecentEventKind::SnapshotPublished,
                    summary: "snapshot updated".to_owned(),
                }],
            },
        }
    }

    #[test]
    fn reserves_bottom_pane_space_for_timeline() {
        let mut state = TuiState::default();
        state.reduce(super::TuiAction::SnapshotReceived(Box::new(fixture(12))));

        let rendered = state.render_text(100, 22);
        assert!(rendered.contains("RECENT EVENTS"));
        assert!(rendered.contains("snapshot updated"));
        assert_eq!(rendered.lines().count(), 22);
    }

    #[test]
    fn renders_reducer_status_line_in_header() {
        let mut state = TuiState::default();
        state.reduce(super::TuiAction::ConnectionLost("stream closed".to_owned()));

        let rendered = state.render_text(120, 22);

        assert!(rendered.contains("status=reconnecting after: stream closed"));
    }

    #[test]
    fn coalesces_bridge_snapshots_to_latest_value() {
        let mut mailbox = BridgeMailbox::default();
        let first = fixture(1);
        let second = fixture(3);

        mailbox.push_snapshot(first);
        mailbox.push_snapshot(second.clone());

        match mailbox.take_action() {
            Some(super::TuiAction::SnapshotReceived(snapshot)) => {
                assert_eq!(*snapshot, second);
            }
            other => panic!("expected latest snapshot, got {other:?}"),
        }
        assert!(mailbox.take_action().is_none());
    }

    #[test]
    fn live_snapshot_supersedes_pending_bootstrap_snapshot() {
        let mut mailbox = BridgeMailbox::default();
        let bootstrap = fixture(2);
        let live = fixture(3);

        mailbox.push_bootstrap_snapshot(bootstrap);
        mailbox.push_snapshot(live.clone());

        match mailbox.take_action() {
            Some(super::TuiAction::SnapshotReceived(snapshot)) => {
                assert_eq!(*snapshot, live);
            }
            other => panic!("expected live snapshot, got {other:?}"),
        }
        assert!(mailbox.take_action().is_none());
    }

    #[test]
    fn keeps_latest_snapshot_visible_across_disconnects() {
        let mut mailbox = BridgeMailbox::default();
        let latest = fixture(5);
        let mut state = TuiState::default();
        state.reduce(super::TuiAction::SnapshotReceived(Box::new(latest.clone())));
        mailbox.push_connection_loss("stream closed".to_owned());
        while let Some(action) = mailbox.take_action() {
            state.reduce(action);
        }

        assert_eq!(state.latest_snapshot, Some(latest));
        assert_eq!(
            state.connection,
            ConnectionState::Reconnecting("stream closed".to_owned())
        );
    }

    #[test]
    fn queued_connection_loss_is_rendered_before_recovery_snapshot() {
        let bridge = Arc::new(Mutex::new(BridgeMailbox::default()));
        let scripted_exit = Arc::new(Mutex::new(ScriptedExitOutcome::Pending));
        let mut app = OperatorApp::new(
            Arc::clone(&bridge),
            Some(Duration::from_millis(250)),
            Arc::clone(&scripted_exit),
        );

        {
            let mut mailbox = bridge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            mailbox.push_snapshot(fixture(1));
        }
        app.drain_bridge();
        assert_eq!(app.state.connection, ConnectionState::Live);
        assert_eq!(
            app.state
                .latest_snapshot
                .as_ref()
                .map(|snapshot| snapshot.snapshot.issues.len()),
            Some(1)
        );

        {
            let mut mailbox = bridge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            mailbox.push_connection_loss("stream closed".to_owned());
            mailbox.push_snapshot(fixture(2));
        }
        app.drain_bridge();
        assert_eq!(
            app.state.connection,
            ConnectionState::Reconnecting("stream closed".to_owned())
        );
        assert_eq!(
            app.state
                .latest_snapshot
                .as_ref()
                .map(|snapshot| snapshot.snapshot.issues.len()),
            Some(1)
        );

        app.drain_bridge();
        assert_eq!(app.state.connection, ConnectionState::Live);
        assert_eq!(
            app.state
                .latest_snapshot
                .as_ref()
                .map(|snapshot| snapshot.snapshot.issues.len()),
            Some(2)
        );
    }

    #[test]
    fn queued_connection_loss_discards_pre_disconnect_snapshot_until_recovery_arrives() {
        let bridge = Arc::new(Mutex::new(BridgeMailbox::default()));
        let scripted_exit = Arc::new(Mutex::new(ScriptedExitOutcome::Pending));
        let mut app = OperatorApp::new(
            Arc::clone(&bridge),
            Some(Duration::from_millis(250)),
            Arc::clone(&scripted_exit),
        );

        {
            let mut mailbox = bridge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            mailbox.push_snapshot(fixture(1));
        }
        app.drain_bridge();
        assert_eq!(app.state.connection, ConnectionState::Live);
        assert_eq!(
            app.state
                .latest_snapshot
                .as_ref()
                .map(|snapshot| snapshot.snapshot.issues.len()),
            Some(1)
        );

        {
            let mut mailbox = bridge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            mailbox.push_snapshot(fixture(2));
            mailbox.push_connection_loss("stream closed".to_owned());
        }
        app.drain_bridge();
        assert_eq!(
            app.state.connection,
            ConnectionState::Reconnecting("stream closed".to_owned())
        );
        assert_eq!(
            app.state
                .latest_snapshot
                .as_ref()
                .map(|snapshot| snapshot.snapshot.issues.len()),
            Some(1)
        );

        app.drain_bridge();
        assert_eq!(
            app.state.connection,
            ConnectionState::Reconnecting("stream closed".to_owned())
        );
        assert_eq!(
            app.state
                .latest_snapshot
                .as_ref()
                .map(|snapshot| snapshot.snapshot.issues.len()),
            Some(1)
        );

        {
            let mut mailbox = bridge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            mailbox.push_snapshot(fixture(3));
        }
        app.drain_bridge();
        assert_eq!(app.state.connection, ConnectionState::Live);
        assert_eq!(
            app.state
                .latest_snapshot
                .as_ref()
                .map(|snapshot| snapshot.snapshot.issues.len()),
            Some(3)
        );
    }

    #[test]
    fn scripted_exit_fails_when_live_stream_never_arrives() {
        let bridge = Arc::new(Mutex::new(BridgeMailbox::default()));
        {
            let mut mailbox = bridge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            mailbox.push_connection_loss("stream closed".to_owned());
        }
        let scripted_exit = Arc::new(Mutex::new(ScriptedExitOutcome::Pending));
        let mut app = OperatorApp::new(
            Arc::clone(&bridge),
            Some(Duration::from_millis(250)),
            Arc::clone(&scripted_exit),
        );

        app.drain_bridge();
        app.record_scripted_exit();

        assert_eq!(
            *scripted_exit
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            ScriptedExitOutcome::Failed(
                "last status: reconnecting after: stream closed".to_owned()
            )
        );
    }

    #[test]
    fn scripted_exit_succeeds_after_streamed_snapshot_becomes_live() {
        let bridge = Arc::new(Mutex::new(BridgeMailbox::default()));
        {
            let mut mailbox = bridge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            mailbox.push_snapshot(fixture(2));
        }
        let scripted_exit = Arc::new(Mutex::new(ScriptedExitOutcome::Pending));
        let mut app = OperatorApp::new(
            Arc::clone(&bridge),
            Some(Duration::from_millis(250)),
            Arc::clone(&scripted_exit),
        );

        app.drain_bridge();
        app.record_scripted_exit();

        assert_eq!(
            *scripted_exit
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            ScriptedExitOutcome::Succeeded
        );
    }

    #[test]
    fn scripted_exit_fails_after_live_stream_falls_back_to_reconnecting() {
        let bridge = Arc::new(Mutex::new(BridgeMailbox::default()));
        let scripted_exit = Arc::new(Mutex::new(ScriptedExitOutcome::Pending));
        let mut app = OperatorApp::new(
            Arc::clone(&bridge),
            Some(Duration::from_millis(250)),
            Arc::clone(&scripted_exit),
        );

        {
            let mut mailbox = bridge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            mailbox.push_snapshot(fixture(1));
        }
        app.drain_bridge();
        assert_eq!(app.state.connection, ConnectionState::Live);

        {
            let mut mailbox = bridge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            mailbox.push_connection_loss("stream closed".to_owned());
        }
        app.drain_bridge();
        app.record_scripted_exit();

        assert_eq!(
            *scripted_exit
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            ScriptedExitOutcome::Failed(
                "last status: reconnecting after: stream closed".to_owned()
            )
        );
    }

    #[test]
    fn retrying_stream_update_marks_state_reconnecting_until_next_snapshot() {
        let bridge = Arc::new(Mutex::new(BridgeMailbox::default()));
        let mut state = TuiState::default();

        super::handle_stream_update(&bridge, ControlPlaneStreamUpdate::Snapshot(fixture(1)));
        {
            let mut mailbox = bridge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            while let Some(action) = mailbox.take_action() {
                state.reduce(action);
            }
        }
        assert_eq!(state.connection, ConnectionState::Live);

        super::handle_stream_update(
            &bridge,
            ControlPlaneStreamUpdate::Reconnecting(ControlPlaneClientError::StreamConnectTimeout(
                Duration::from_millis(50),
            )),
        );
        {
            let mut mailbox = bridge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            while let Some(action) = mailbox.take_action() {
                state.reduce(action);
            }
        }
        assert!(matches!(state.connection, ConnectionState::Reconnecting(_)));
        assert!(state.status_line.contains("reconnecting after:"));

        super::handle_stream_update(&bridge, ControlPlaneStreamUpdate::Snapshot(fixture(2)));
        {
            let mut mailbox = bridge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            while let Some(action) = mailbox.take_action() {
                state.reduce(action);
            }
        }
        assert_eq!(state.connection, ConnectionState::Live);
        assert_eq!(state.status_line, "live control-plane stream");
        assert_eq!(
            state
                .latest_snapshot
                .as_ref()
                .map(|snapshot| snapshot.snapshot.issues.len()),
            Some(2)
        );
    }

    #[test]
    fn reserves_rows_for_the_timeline_section() {
        let (body_rows, timeline_rows) = section_layout(22);
        assert_eq!((body_rows, timeline_rows), (13, 6));
    }

    #[test]
    fn reserves_detail_rows_in_narrow_layout() {
        let (issue_rows, detail_rows) = stacked_section_layout(13);
        assert_eq!((issue_rows, detail_rows), (6, 6));

        let mut state = TuiState::default();
        state.reduce(super::TuiAction::SnapshotReceived(Box::new(fixture(6))));

        let rendered = state.render_text(60, 22);
        assert!(rendered.contains("ISSUE + WORKSPACE DETAIL"));
        assert!(rendered.contains("workspace path: workspace-0"));
        assert_eq!(rendered.lines().count(), 22);
    }
}
