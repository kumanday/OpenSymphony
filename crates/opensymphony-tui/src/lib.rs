use std::{
    cmp::{max, min},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use ftui::{
    core::geometry::Rect,
    prelude::{App, Cmd, Event, Frame, KeyCode, Model, ScreenMode},
    runtime::{Every, Subscription},
    widgets::{paragraph::Paragraph, Widget},
};
use opensymphony_control::{log_stream_error, ControlPlaneClient, ControlPlaneClientError};
use opensymphony_domain::{IssueSnapshot, MetricsSnapshot, RecentEvent, SnapshotEnvelope};
use thiserror::Error;
use url::Url;

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
            TuiAction::SnapshotReceived(envelope) => {
                self.latest_snapshot = Some(*envelope);
                self.connection = ConnectionState::Live;
                self.status_line = "live control-plane stream".to_owned();
                self.clamp_selection();
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
            "OpenSymphony | conn={} | focus={} | bottom={} | seq={} | issues={} | updated={} | q quit  tab focus  e toggle",
            self.connection.label(),
            self.focus.label(),
            self.timeline_mode.label(),
            sequence,
            issue_count,
            generated
        );
        lines.push(fit(&status, width));
        lines.push("=".repeat(width));

        if width >= 80 {
            let left_width = max(26, width * 2 / 5);
            let right_width = width.saturating_sub(left_width + 3);
            let left = self.issue_lines(left_width);
            let right = self.detail_lines(right_width);
            lines.extend(two_column_block(&left, &right, left_width, right_width));
        } else {
            lines.extend(self.issue_lines(width));
            lines.push("-".repeat(width));
            lines.extend(self.detail_lines(width));
        }

        lines.push("=".repeat(width));
        lines.extend(self.timeline_lines(width));

        if lines.len() > height {
            lines.truncate(height);
        }
        while lines.len() < height {
            lines.push(" ".repeat(width));
        }
        lines.join("\n")
    }

    fn issue_lines(&self, width: usize) -> Vec<String> {
        let mut lines = vec![fit(
            &pane_title("ISSUES", self.focus == FocusPane::Issues),
            width,
        )];
        match &self.latest_snapshot {
            Some(snapshot) if snapshot.snapshot.issues.is_empty() => {
                lines.push(fit("no issues in snapshot", width));
            }
            Some(snapshot) => {
                for (index, issue) in snapshot.snapshot.issues.iter().enumerate() {
                    let marker = if index == self.selected_issue {
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
                lines.push(fit(&format!("tracker: {}", issue.tracker_state), width));
                lines.push(fit(
                    &format!("runtime: {}", issue.runtime_state.as_str()),
                    width,
                ));
                lines.push(fit(
                    &format!("last outcome: {}", issue.last_outcome.as_str()),
                    width,
                ));
                lines.push(fit(
                    &format!("last event: {}", format_timestamp(issue.last_event_at)),
                    width,
                ));
                lines.push(fit(
                    &format!("workspace path: {}", issue.workspace_path_suffix),
                    width,
                ));
                lines.push(fit(
                    &format!("conversation id: {}", issue.conversation_id_suffix),
                    width,
                ));
                lines.push(fit(&format!("retry count: {}", issue.retry_count), width));
                lines.push(fit(&format!("blocked: {}", issue.blocked), width));
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
    SnapshotReceived(Box<SnapshotEnvelope>),
    ConnectionLost(String),
    MoveSelectionUp,
    MoveSelectionDown,
    FocusNext,
    ToggleTimelineMode,
}

#[derive(Debug)]
enum BridgeMessage {
    Snapshot(Box<SnapshotEnvelope>),
    ConnectionLost(String),
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
    let (sender, receiver) = mpsc::channel();
    spawn_bridge(base_url, sender);
    let app = OperatorApp::new(receiver, exit_after);
    App::new(app)
        .screen_mode(ScreenMode::Inline { ui_height: 22 })
        .run()
        .map_err(TuiError::Runtime)
}

#[derive(Debug)]
struct OperatorApp {
    state: TuiState,
    receiver: Receiver<BridgeMessage>,
    exit_after: Option<Duration>,
    started_at: Instant,
}

impl OperatorApp {
    fn new(receiver: Receiver<BridgeMessage>, exit_after: Option<Duration>) -> Self {
        Self {
            state: TuiState::default(),
            receiver,
            exit_after,
            started_at: Instant::now(),
        }
    }

    fn drain_bridge(&mut self) {
        loop {
            match self.receiver.try_recv() {
                Ok(BridgeMessage::Snapshot(snapshot)) => {
                    self.state.reduce(TuiAction::SnapshotReceived(snapshot));
                }
                Ok(BridgeMessage::ConnectionLost(reason)) => {
                    self.state.reduce(TuiAction::ConnectionLost(reason));
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
    }
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
            return Cmd::quit();
        }

        Cmd::none()
    }

    fn view(&self, frame: &mut Frame) {
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
}

fn spawn_bridge(base_url: Url, sender: Sender<BridgeMessage>) {
    thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = sender.send(BridgeMessage::ConnectionLost(error.to_string()));
                return;
            }
        };

        runtime.block_on(async move {
            let retry_delay = Duration::from_millis(750);
            let client = ControlPlaneClient::new(base_url);
            loop {
                match client.fetch_snapshot().await {
                    Ok(snapshot) => {
                        let _ = sender.send(BridgeMessage::Snapshot(Box::new(snapshot)));
                    }
                    Err(error) => {
                        let _ = sender.send(BridgeMessage::ConnectionLost(error.to_string()));
                        tokio::time::sleep(retry_delay).await;
                        continue;
                    }
                }

                let mut stream = match client.stream_updates() {
                    Ok(stream) => stream,
                    Err(error) => {
                        let _ = sender.send(BridgeMessage::ConnectionLost(error.to_string()));
                        tokio::time::sleep(retry_delay).await;
                        continue;
                    }
                };

                let mut should_retry = false;
                while let Some(update) = stream.next().await {
                    match update {
                        Ok(snapshot) => {
                            let _ = sender.send(BridgeMessage::Snapshot(Box::new(snapshot)));
                        }
                        Err(error) => {
                            handle_bridge_error(&sender, &error);
                            should_retry = true;
                            break;
                        }
                    }
                }

                stream.close();
                if !should_retry {
                    let _ = sender.send(BridgeMessage::ConnectionLost(
                        "control-plane stream closed".to_owned(),
                    ));
                }
                tokio::time::sleep(retry_delay).await;
            }
        });
    });
}

fn handle_bridge_error(sender: &Sender<BridgeMessage>, error: &ControlPlaneClientError) {
    log_stream_error(error);
    let _ = sender.send(BridgeMessage::ConnectionLost(error.to_string()));
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

trait RuntimeStateLabel {
    fn as_str(&self) -> &'static str;
}

impl RuntimeStateLabel for opensymphony_domain::IssueRuntimeState {
    fn as_str(&self) -> &'static str {
        match self {
            opensymphony_domain::IssueRuntimeState::Idle => "idle",
            opensymphony_domain::IssueRuntimeState::Running => "running",
            opensymphony_domain::IssueRuntimeState::RetryQueued => "retry_queued",
            opensymphony_domain::IssueRuntimeState::Releasing => "releasing",
            opensymphony_domain::IssueRuntimeState::Completed => "completed",
            opensymphony_domain::IssueRuntimeState::Failed => "failed",
        }
    }
}

trait WorkerOutcomeLabel {
    fn as_str(&self) -> &'static str;
}

impl WorkerOutcomeLabel for opensymphony_domain::WorkerOutcome {
    fn as_str(&self) -> &'static str {
        match self {
            opensymphony_domain::WorkerOutcome::Unknown => "unknown",
            opensymphony_domain::WorkerOutcome::Running => "running",
            opensymphony_domain::WorkerOutcome::Continued => "continued",
            opensymphony_domain::WorkerOutcome::Completed => "completed",
            opensymphony_domain::WorkerOutcome::Failed => "failed",
            opensymphony_domain::WorkerOutcome::Canceled => "canceled",
        }
    }
}
