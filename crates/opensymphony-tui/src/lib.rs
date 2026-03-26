use std::{
    cmp::{max, min},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use ftui::{
    Style,
    core::geometry::Rect,
    prelude::{App, Cmd, Event, Frame, KeyCode, Model, ScreenMode},
    render::cell::PackedRgba,
    runtime::{Every, Subscription},
    text::text::{Line, Span, Text},
    widgets::{Widget, paragraph::Paragraph},
};
use opensymphony_control::{ControlPlaneClient, ControlPlaneClientError};
use opensymphony_domain::{
    ControlPlaneIssueRuntimeState, ControlPlaneIssueSnapshot as IssueSnapshot,
    ControlPlaneMetricsSnapshot as MetricsSnapshot, ControlPlaneRecentEvent as RecentEvent,
    SnapshotEnvelope,
};
use thiserror::Error;
use tokio::sync::watch;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use url::Url;

const INLINE_UI_HEIGHT: u16 = 42;
const MIN_TIMELINE_LINES: usize = 4;
const MAX_TIMELINE_LINES: usize = 6;

const RED: PackedRgba = PackedRgba::rgb(205, 0, 0);
const GREEN: PackedRgba = PackedRgba::rgb(0, 205, 0);
const YELLOW: PackedRgba = PackedRgba::rgb(205, 205, 0);
const BLUE: PackedRgba = PackedRgba::rgb(0, 255, 255); // Bright cyan instead of dark blue
const MAGENTA: PackedRgba = PackedRgba::rgb(205, 0, 205);
const CYAN: PackedRgba = PackedRgba::rgb(0, 205, 205);
const BRIGHT_GREEN: PackedRgba = PackedRgba::rgb(0, 255, 0);
const BRIGHT_YELLOW: PackedRgba = PackedRgba::rgb(255, 255, 0);
const BRIGHT_BLACK: PackedRgba = PackedRgba::rgb(127, 127, 127);

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
                let selected_issue_identifier =
                    self.selected_issue().map(|issue| issue.identifier.clone());
                self.latest_snapshot = Some(*envelope);
                if !matches!(self.connection, ConnectionState::Live) {
                    self.status_line = match self.connection {
                        ConnectionState::Connecting => {
                            "bootstrap snapshot loaded; waiting for live stream".to_owned()
                        }
                        ConnectionState::Reconnecting(_) => {
                            "snapshot refreshed; waiting for live stream".to_owned()
                        }
                        ConnectionState::Live => "live control-plane stream".to_owned(),
                    };
                }
                self.restore_selection(selected_issue_identifier.as_deref());
            }
            TuiAction::StreamAttached => {
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
        let daemon = snapshot
            .map(daemon_status_summary)
            .unwrap_or_else(|| "daemon=--".to_owned());
        let agent = snapshot
            .map(agent_server_status_summary)
            .unwrap_or_else(|| "agent=--".to_owned());
        let mut header = vec!["OpenSymphony".to_owned(), daemon, agent];
        header.push(connection_status_summary(self));
        header.push(format!("seq={sequence}"));
        header.push(format!("focus={}", self.focus.label()));
        header.push(format!("bottom={}", self.timeline_mode.label()));
        header.push(format!("issues={issue_count}"));
        header.push(format!("updated={generated}"));
        header.push("q quit  tab focus  e toggle".to_owned());
        lines.push(fit(&header.join(" | "), width));
        lines.push("=".repeat(width));

        if width >= 80 {
            let left_width = max(50, width * 3 / 5);
            let right_width = width.saturating_sub(left_width + 3);
            let left = self.issue_lines(left_width, body_rows);
            let right = self.detail_lines(right_width, body_rows);
            lines.extend(fit_section(
                two_column_block(&left, &right, left_width, right_width),
                body_rows,
                width,
            ));
        } else {
            let (issue_rows, detail_rows) = stacked_body_layout(body_rows);
            lines.extend(fit_section(
                self.issue_lines(width, issue_rows),
                issue_rows,
                width,
            ));
            if detail_rows > 0 {
                lines.push("-".repeat(width));
                lines.extend(fit_section(
                    self.detail_lines(width, detail_rows),
                    detail_rows,
                    width,
                ));
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

    pub fn render_text_styled(&self, width: usize, height: usize) -> Text {
        if width == 0 || height == 0 {
            return Text::raw("");
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

        lines.push(self.header_line_styled(width, snapshot, sequence, &generated, issue_count));
        lines.push(Line::from(Span::styled(
            "=".repeat(width),
            Style::new().dim(),
        )));

        if width >= 80 {
            let left_width = max(50, width * 3 / 5);
            let right_width = width.saturating_sub(left_width + 3);
            let left = self.issue_lines_styled(left_width, body_rows);
            let right = self.detail_lines_styled(right_width, body_rows);
            lines.extend(fit_section_styled(
                two_column_block_styled(&left, &right, left_width, right_width),
                body_rows,
                width,
            ));
        } else {
            let (issue_rows, detail_rows) = stacked_body_layout(body_rows);
            lines.extend(fit_section_styled(
                self.issue_lines_styled(width, issue_rows),
                issue_rows,
                width,
            ));
            if detail_rows > 0 {
                lines.push(Line::from(Span::styled(
                    "-".repeat(width),
                    Style::new().dim(),
                )));
                lines.extend(fit_section_styled(
                    self.detail_lines_styled(width, detail_rows),
                    detail_rows,
                    width,
                ));
            }
        }

        if timeline_rows > 0 {
            lines.push(Line::from(Span::styled(
                "=".repeat(width),
                Style::new().dim(),
            )));
            lines.extend(fit_section_styled(
                self.timeline_lines_styled(width),
                timeline_rows,
                width,
            ));
        }

        if lines.len() > height {
            lines.truncate(height);
        }
        while lines.len() < height {
            lines.push(Line::from(Span::raw(" ".repeat(width))));
        }
        Text::from_lines(lines)
    }

    fn header_line_styled(
        &self,
        width: usize,
        snapshot: Option<&SnapshotEnvelope>,
        sequence: u64,
        generated: &str,
        issue_count: usize,
    ) -> Line {
        let mut spans = vec![
            Span::styled("OpenSymphony", Style::new().bold()),
            Span::raw(" | "),
        ];

        if let Some(snap) = snapshot {
            let daemon = &snap.snapshot.daemon;
            let daemon_style = match daemon.state {
                opensymphony_domain::ControlPlaneDaemonState::Ready => Style::new().fg(GREEN),
                opensymphony_domain::ControlPlaneDaemonState::Starting => Style::new().fg(YELLOW),
                opensymphony_domain::ControlPlaneDaemonState::Degraded => Style::new().fg(RED),
                opensymphony_domain::ControlPlaneDaemonState::Stopped => {
                    Style::new().fg(BRIGHT_BLACK)
                }
            };
            spans.push(Span::styled(
                format!("daemon={}", daemon.state.as_str()),
                daemon_style,
            ));
            spans.push(Span::raw(" | "));

            let agent = &snap.snapshot.agent_server;
            let agent_style = if agent.reachable {
                Style::new().fg(GREEN)
            } else {
                Style::new().fg(RED)
            };
            spans.push(Span::styled(
                format!("agent={}", if agent.reachable { "up" } else { "down" }),
                agent_style,
            ));
            spans.push(Span::raw(" | "));
        } else {
            spans.push(Span::styled("daemon=--", Style::new().dim()));
            spans.push(Span::raw(" | "));
            spans.push(Span::styled("agent=--", Style::new().dim()));
            spans.push(Span::raw(" | "));
        }

        let conn_style = match &self.connection {
            ConnectionState::Live => Style::new().fg(GREEN).bold(),
            ConnectionState::Connecting => Style::new().fg(YELLOW),
            ConnectionState::Reconnecting(_) => Style::new().fg(RED),
        };
        spans.push(Span::styled(
            format!("conn={}", self.connection.label()),
            conn_style,
        ));
        spans.push(Span::raw(" | "));
        spans.push(Span::styled(format!("seq={sequence}"), Style::new().dim()));
        spans.push(Span::raw(" | "));
        spans.push(Span::styled(
            format!("issues={issue_count}"),
            Style::new().dim(),
        ));
        spans.push(Span::raw(" | "));
        spans.push(Span::styled(
            format!("updated={generated}"),
            Style::new().dim(),
        ));
        spans.push(Span::raw(" | "));
        spans.push(Span::styled(
            "q quit  tab focus  e toggle",
            Style::new().fg(BRIGHT_BLACK),
        ));

        let line = Line::from_spans(spans);
        Line::from_spans(vec![Span::raw(fit(&line.to_plain_text(), width))])
    }

    fn issue_lines_styled(&self, width: usize, max_rows: usize) -> Vec<Line> {
        let title_style = if self.focus == FocusPane::Issues {
            Style::new().bold()
        } else {
            Style::new().dim()
        };
        let mut lines = vec![Line::from(Span::styled(
            pane_title("ISSUES", self.focus == FocusPane::Issues),
            title_style,
        ))];

        match &self.latest_snapshot {
            Some(snapshot) if snapshot.snapshot.issues.is_empty() => {
                lines.push(Line::from(Span::styled(
                    "no issues in snapshot",
                    Style::new().dim(),
                )));
            }
            Some(snapshot) => {
                let (start, end) = issue_window(
                    snapshot.snapshot.issues.len(),
                    self.selected_issue,
                    visible_issue_count(max_rows),
                );
                for (index, issue) in snapshot.snapshot.issues[start..end].iter().enumerate() {
                    let global_index = start + index;
                    let is_selected = global_index == self.selected_issue;
                    lines.push(self.issue_line_styled(issue, is_selected, width));
                }
            }
            None => {
                lines.push(Line::from(Span::styled(
                    "awaiting first snapshot",
                    Style::new().dim(),
                )));
            }
        }
        lines
    }

    fn issue_line_styled(&self, issue: &IssueSnapshot, is_selected: bool, _width: usize) -> Line {
        // Use reverse video (swap foreground/background) for selected items
        // This works on all terminals regardless of color support
        let base_style = if is_selected {
            Style::new().reverse().bold()
        } else {
            Style::new()
        };

        let marker = if is_selected { ">" } else { " " };
        let marker_style = if is_selected {
            Style::new().reverse().bold()
        } else {
            Style::new().fg(BRIGHT_GREEN).bold()
        };

        let id_style = base_style.merge(&Style::new().fg(CYAN).bold());
        let state_style =
            base_style.merge(&Style::new().fg(runtime_state_color(&issue.runtime_state)));
        let tracker_style = base_style.merge(&Style::new().dim());
        let title_style = base_style;

        let line = Line::from_spans(vec![
            Span::styled(marker, marker_style),
            Span::styled(" ", base_style),
            Span::styled(&issue.identifier, id_style),
            Span::styled(" [", base_style),
            Span::styled(issue.runtime_state.as_str(), state_style),
            Span::styled(" / ", base_style),
            Span::styled(&issue.tracker_state, tracker_style),
            Span::styled("] ", base_style),
            Span::styled(&issue.title, title_style),
        ]);
        line
    }

    fn detail_lines_styled(&self, width: usize, max_rows: usize) -> Vec<Line> {
        let title_style = if self.focus == FocusPane::Detail {
            Style::new().bold()
        } else {
            Style::new().dim()
        };
        let mut lines = vec![Line::from(Span::styled(
            pane_title("ISSUE + WORKSPACE DETAIL", self.focus == FocusPane::Detail),
            title_style,
        ))];

        match self.selected_issue() {
            Some(issue) => {
                let id_style = Style::new().fg(CYAN).bold();
                lines.push(Line::from_spans(vec![
                    Span::styled(&issue.identifier, id_style),
                    Span::raw(" "),
                    Span::raw(&issue.title),
                ]));

                let runtime_style = Style::new().fg(runtime_state_color(&issue.runtime_state));
                lines.push(Line::from_spans(vec![
                    Span::styled("tracker: ", Style::new().dim()),
                    Span::raw(&issue.tracker_state),
                    Span::raw(" | "),
                    Span::styled("runtime: ", Style::new().dim()),
                    Span::styled(issue.runtime_state.as_str(), runtime_style),
                    Span::raw(" | "),
                    Span::styled("outcome: ", Style::new().dim()),
                    Span::raw(issue.last_outcome.as_str()),
                ]));

                lines.push(Line::from_spans(vec![
                    Span::styled("workspace: ", Style::new().dim()),
                    Span::raw(&issue.workspace_path_suffix),
                    Span::raw(" | "),
                    Span::styled("conv: ", Style::new().dim()),
                    Span::raw(&issue.conversation_id_suffix),
                ]));

                let blocked_style = if issue.blocked {
                    Style::new().fg(YELLOW)
                } else {
                    Style::new().fg(GREEN)
                };
                lines.push(Line::from_spans(vec![
                    Span::styled("last event: ", Style::new().dim()),
                    Span::raw(format_timestamp(issue.last_event_at)),
                    Span::raw(" | "),
                    Span::styled("retries: ", Style::new().dim()),
                    Span::raw(format!("{}", issue.retry_count)),
                    Span::raw(" | "),
                    Span::styled("blocked: ", Style::new().dim()),
                    Span::styled(format!("{}", issue.blocked), blocked_style),
                ]));

                let detail_header_rows = 5;
                let remaining_rows = max_rows.saturating_sub(detail_header_rows);

                if remaining_rows >= 4 {
                    lines.push(Line::from(Span::styled(
                        "-".repeat(width.min(40)),
                        Style::new().dim(),
                    )));
                    lines.extend(self.conversation_events_lines_styled(
                        width,
                        issue,
                        remaining_rows / 2,
                    ));
                }

                if remaining_rows >= 6 {
                    lines.push(Line::from(Span::styled(
                        "-".repeat(width.min(40)),
                        Style::new().dim(),
                    )));
                    lines.extend(self.modified_files_lines_styled(width, issue));
                }
            }
            None => {
                lines.push(Line::from(Span::styled(
                    "no selected issue",
                    Style::new().dim(),
                )));
            }
        }
        lines
    }

    fn conversation_events_lines_styled(
        &self,
        _width: usize,
        issue: &IssueSnapshot,
        max_rows: usize,
    ) -> Vec<Line> {
        let mut lines = vec![Line::from(Span::styled(
            "[ ] CONVERSATION ACTIVITY",
            Style::new().bold(),
        ))];

        if issue.recent_events.is_empty() {
            lines.push(Line::from(Span::styled(
                "no recent activity",
                Style::new().dim(),
            )));
        } else {
            let show_count = max_rows.saturating_sub(1).min(issue.recent_events.len());
            for event in issue.recent_events.iter().rev().take(show_count) {
                let kind_style = event_kind_style(&event.kind);
                let summary = if event.summary.len() > 40 {
                    format!("{}...", &event.summary[..37])
                } else {
                    event.summary.clone()
                };
                lines.push(Line::from_spans(vec![
                    Span::styled(
                        format!("{} ", format_timestamp(event.happened_at)),
                        Style::new().dim(),
                    ),
                    Span::styled(&event.kind, kind_style),
                    Span::raw(" "),
                    Span::raw(summary),
                ]));
            }
        }
        lines
    }

    fn modified_files_lines_styled(&self, width: usize, issue: &IssueSnapshot) -> Vec<Line> {
        let mut lines = vec![Line::from(Span::styled(
            "[ ] MODIFIED FILES",
            Style::new().bold(),
        ))];

        if issue.modified_files.is_empty() {
            lines.push(Line::from(Span::styled(
                "no modified files",
                Style::new().dim(),
            )));
        } else {
            for file in &issue.modified_files {
                let (change_symbol, change_style) = match file.change_kind {
                    opensymphony_domain::ControlPlaneFileChangeKind::Created => {
                        ("+", Style::new().fg(GREEN))
                    }
                    opensymphony_domain::ControlPlaneFileChangeKind::Modified => {
                        ("~", Style::new().fg(YELLOW))
                    }
                    opensymphony_domain::ControlPlaneFileChangeKind::Removed => {
                        ("-", Style::new().fg(RED))
                    }
                };
                let path = if file.path.len() > width.saturating_sub(12) {
                    let truncated_len = width.saturating_sub(15);
                    format!(
                        "...{}",
                        &file.path[file.path.len().saturating_sub(truncated_len)..]
                    )
                } else {
                    file.path.clone()
                };
                lines.push(Line::from_spans(vec![
                    Span::styled(change_symbol, change_style.bold()),
                    Span::raw(" "),
                    Span::raw(path),
                    Span::raw(" "),
                    Span::styled(
                        format!("(+{}/-{})", file.lines_added, file.lines_removed),
                        Style::new().dim(),
                    ),
                ]));
            }
        }
        lines
    }

    fn timeline_lines_styled(&self, _width: usize) -> Vec<Line> {
        let title = match self.timeline_mode {
            TimelineMode::Events => "RECENT EVENTS",
            TimelineMode::Metrics => "METRICS",
        };
        let title_style = if self.focus == FocusPane::Timeline {
            Style::new().bold()
        } else {
            Style::new().dim()
        };
        let mut lines = vec![Line::from(Span::styled(title, title_style))];

        match &self.latest_snapshot {
            Some(snapshot) => match self.timeline_mode {
                TimelineMode::Events => {
                    for event in &snapshot.snapshot.recent_events {
                        let kind_style = match event.kind {
                            opensymphony_domain::ControlPlaneRecentEventKind::WorkerStarted => {
                                Style::new().fg(GREEN)
                            }
                            opensymphony_domain::ControlPlaneRecentEventKind::WorkerCompleted => {
                                Style::new().fg(CYAN)
                            }
                            opensymphony_domain::ControlPlaneRecentEventKind::Warning => {
                                Style::new().fg(RED)
                            }
                            opensymphony_domain::ControlPlaneRecentEventKind::SnapshotPublished => {
                                Style::new().dim()
                            }
                            _ => Style::new().dim(),
                        };
                        lines.push(Line::from_spans(vec![
                            Span::styled(
                                format!("{} ", format_timestamp(event.happened_at)),
                                Style::new().dim(),
                            ),
                            Span::styled(event.kind.as_str(), kind_style),
                            Span::raw(" "),
                            Span::raw(&event.summary),
                        ]));
                    }
                }
                TimelineMode::Metrics => {
                    let m = &snapshot.snapshot.metrics;
                    lines.push(Line::from_spans(vec![
                        Span::styled("running: ", Style::new().dim()),
                        Span::styled(format!("{}", m.running_issues), Style::new().fg(GREEN)),
                    ]));
                    lines.push(Line::from_spans(vec![
                        Span::styled("retry queue: ", Style::new().dim()),
                        Span::raw(format!("{}", m.retry_queue_depth)),
                    ]));
                    lines.push(Line::from_spans(vec![
                        Span::styled("tokens: ", Style::new().dim()),
                        Span::raw(format!("{}", m.total_tokens)),
                    ]));
                }
            },
            None => {
                lines.push(Line::from(Span::styled(
                    "awaiting first snapshot",
                    Style::new().dim(),
                )));
            }
        }
        lines
    }

    fn issue_lines(&self, width: usize, max_rows: usize) -> Vec<String> {
        let mut lines = vec![fit(
            &pane_title("ISSUES", self.focus == FocusPane::Issues),
            width,
        )];
        match &self.latest_snapshot {
            Some(snapshot) if snapshot.snapshot.issues.is_empty() => {
                lines.push(fit("no issues in snapshot", width));
            }
            Some(snapshot) => {
                let (start, end) = issue_window(
                    snapshot.snapshot.issues.len(),
                    self.selected_issue,
                    visible_issue_count(max_rows),
                );
                for (index, issue) in snapshot.snapshot.issues[start..end].iter().enumerate() {
                    let global_index = start + index;
                    let marker = if global_index == self.selected_issue {
                        ">"
                    } else {
                        " "
                    };
                    let line = format!(
                        "{marker} {} [{} / {}] {}",
                        issue.identifier,
                        issue.runtime_state.as_str(),
                        issue.tracker_state,
                        issue.title
                    );
                    lines.push(fit(&line, width));
                }
            }
            None => {
                lines.push(fit("awaiting first snapshot", width));
            }
        }
        lines
    }

    fn detail_lines(&self, width: usize, max_rows: usize) -> Vec<String> {
        let mut lines = vec![fit(
            &pane_title("ISSUE + WORKSPACE DETAIL", self.focus == FocusPane::Detail),
            width,
        )];
        match self.selected_issue() {
            Some(issue) => {
                lines.push(fit(&format!("{} {}", issue.identifier, issue.title), width));
                lines.push(fit(
                    &format!(
                        "tracker: {} | runtime: {} | outcome: {}",
                        issue.tracker_state,
                        issue.runtime_state.as_str(),
                        issue.last_outcome.as_str()
                    ),
                    width,
                ));
                lines.push(fit(
                    &format!(
                        "workspace: {} | conv: {}",
                        issue.workspace_path_suffix, issue.conversation_id_suffix
                    ),
                    width,
                ));
                lines.push(fit(
                    &format!(
                        "last event: {} | retries: {} | blocked: {}",
                        format_timestamp(issue.last_event_at),
                        issue.retry_count,
                        issue.blocked
                    ),
                    width,
                ));

                let detail_header_rows = 5;
                let remaining_rows = max_rows.saturating_sub(detail_header_rows);

                if remaining_rows >= 4 {
                    lines.push("-".repeat(width.min(40)));
                    lines.extend(self.conversation_events_lines(width, issue, remaining_rows / 2));
                }

                if remaining_rows >= 6 {
                    lines.push("-".repeat(width.min(40)));
                    lines.extend(self.modified_files_lines(width, issue));
                }
            }
            None => {
                lines.push(fit("no selected issue", width));
            }
        }
        lines
    }

    fn conversation_events_lines(
        &self,
        width: usize,
        issue: &IssueSnapshot,
        max_rows: usize,
    ) -> Vec<String> {
        let mut lines = vec![fit("[ ] CONVERSATION ACTIVITY", width)];
        if issue.recent_events.is_empty() {
            lines.push(fit("no recent activity", width));
        } else {
            let show_count = max_rows.saturating_sub(1).min(issue.recent_events.len());
            for event in issue.recent_events.iter().rev().take(show_count) {
                let summary = if event.summary.len() > 40 {
                    format!("{}...", &event.summary[..37])
                } else {
                    event.summary.clone()
                };
                lines.push(fit(
                    &format!(
                        "{} {} {}",
                        format_timestamp(event.happened_at),
                        event.kind,
                        summary
                    ),
                    width,
                ));
            }
        }
        lines
    }

    fn modified_files_lines(&self, width: usize, issue: &IssueSnapshot) -> Vec<String> {
        let mut lines = vec![fit("[ ] MODIFIED FILES", width)];
        if issue.modified_files.is_empty() {
            lines.push(fit("no modified files", width));
        } else {
            for file in &issue.modified_files {
                let change_symbol = match file.change_kind {
                    opensymphony_domain::ControlPlaneFileChangeKind::Created => "+",
                    opensymphony_domain::ControlPlaneFileChangeKind::Modified => "~",
                    opensymphony_domain::ControlPlaneFileChangeKind::Removed => "-",
                };
                let path = if file.path.len() > width.saturating_sub(12) {
                    let truncated_len = width.saturating_sub(15);
                    format!(
                        "...{}",
                        &file.path[file.path.len().saturating_sub(truncated_len)..]
                    )
                } else {
                    file.path.clone()
                };
                lines.push(fit(
                    &format!(
                        "{} {} (+{}/-{})",
                        change_symbol, path, file.lines_added, file.lines_removed
                    ),
                    width,
                ));
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

    fn restore_selection(&mut self, selected_issue_identifier: Option<&str>) {
        let count = self.issue_count();
        if count == 0 {
            self.selected_issue = 0;
            return;
        }

        if let Some(identifier) = selected_issue_identifier
            && let Some(selected_issue) = self.latest_snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .snapshot
                    .issues
                    .iter()
                    .position(|issue| issue.identifier == identifier)
            })
        {
            self.selected_issue = selected_issue;
            return;
        }

        self.selected_issue = min(self.selected_issue, count - 1);
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
    StreamAttached,
    ConnectionLost(String),
    MoveSelectionUp,
    MoveSelectionDown,
    FocusNext,
    ToggleTimelineMode,
}

#[derive(Debug, Default)]
struct BridgeMailbox {
    latest_snapshot: Option<Box<SnapshotEnvelope>>,
    stream_attached: bool,
    latest_connection_loss: Option<String>,
}

impl BridgeMailbox {
    fn push_snapshot(&mut self, snapshot: SnapshotEnvelope) {
        self.latest_snapshot = Some(Box::new(snapshot));
    }

    fn push_attached_snapshot(&mut self, snapshot: SnapshotEnvelope) {
        self.latest_connection_loss = None;
        self.latest_snapshot = Some(Box::new(snapshot));
        self.stream_attached = true;
    }

    fn push_connection_loss(&mut self, reason: String) {
        self.stream_attached = false;
        self.latest_connection_loss = Some(reason);
    }

    fn take_action(&mut self) -> Option<TuiAction> {
        if let Some(snapshot) = self.latest_snapshot.take() {
            return Some(TuiAction::SnapshotReceived(snapshot));
        }

        if let Some(reason) = self.latest_connection_loss.take() {
            return Some(TuiAction::ConnectionLost(reason));
        }

        self.stream_attached.then(|| {
            self.stream_attached = false;
            TuiAction::StreamAttached
        })
    }
}

#[derive(Debug)]
struct BridgeHandle {
    mailbox: Arc<Mutex<BridgeMailbox>>,
    shutdown: watch::Sender<bool>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl BridgeHandle {
    fn spawn(base_url: Url) -> Self {
        let mailbox = Arc::new(Mutex::new(BridgeMailbox::default()));
        let (shutdown, shutdown_rx) = watch::channel(false);
        let join_handle = thread::spawn({
            let mailbox = Arc::clone(&mailbox);
            move || run_bridge_thread(base_url, mailbox, shutdown_rx)
        });
        Self {
            mailbox,
            shutdown,
            join_handle: Some(join_handle),
        }
    }

    fn mailbox(&self) -> Arc<Mutex<BridgeMailbox>> {
        Arc::clone(&self.mailbox)
    }

    fn shutdown(mut self) -> Result<(), TuiError> {
        let _ = self.shutdown.send(true);
        if let Some(join_handle) = self.join_handle.take() {
            join_handle
                .join()
                .map_err(|_| TuiError::BridgeThreadPanicked)?;
        }
        Ok(())
    }
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
    let bridge = BridgeHandle::spawn(base_url);
    let outcome = Arc::new(Mutex::new(RunOutcome::default()));
    let app = OperatorApp::new(bridge.mailbox(), exit_after, Arc::clone(&outcome));
    let run_result = App::new(app)
        .screen_mode(ScreenMode::Inline {
            ui_height: INLINE_UI_HEIGHT,
        })
        .run()
        .map_err(TuiError::Runtime);
    let shutdown_result = bridge.shutdown();
    let timeout_before_live = outcome
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .timeout_before_live
        .clone();

    match (run_result, shutdown_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => match timeout_before_live {
            Some(status_line) => Err(TuiError::AttachTimeout(status_line)),
            None => Ok(()),
        },
    }
}

#[derive(Debug, Default)]
struct RunOutcome {
    timeout_before_live: Option<String>,
}

#[derive(Debug)]
struct OperatorApp {
    state: TuiState,
    bridge: Arc<Mutex<BridgeMailbox>>,
    exit_after: Option<Duration>,
    started_at: Instant,
    saw_live_stream: bool,
    outcome: Arc<Mutex<RunOutcome>>,
}

impl OperatorApp {
    fn new(
        bridge: Arc<Mutex<BridgeMailbox>>,
        exit_after: Option<Duration>,
        outcome: Arc<Mutex<RunOutcome>>,
    ) -> Self {
        Self {
            state: TuiState::default(),
            bridge,
            exit_after,
            started_at: Instant::now(),
            saw_live_stream: false,
            outcome,
        }
    }

    fn drain_bridge(&mut self) {
        let mut bridge = self
            .bridge
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while let Some(action) = bridge.take_action() {
            self.state.reduce(action);
        }
        self.saw_live_stream |= matches!(self.state.connection, ConnectionState::Live);
    }
}

fn run_bridge_thread(
    base_url: Url,
    bridge: Arc<Mutex<BridgeMailbox>>,
    shutdown: watch::Receiver<bool>,
) {
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

    runtime.block_on(run_bridge_loop(base_url, bridge, shutdown));
}

async fn run_bridge_loop(
    base_url: Url,
    bridge: Arc<Mutex<BridgeMailbox>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let retry_delay = Duration::from_millis(750);
    let client = ControlPlaneClient::new(base_url);

    loop {
        let snapshot_result = match fetch_snapshot_or_shutdown(&client, &mut shutdown).await {
            Some(result) => result,
            None => return,
        };
        match snapshot_result {
            Ok(snapshot) => push_snapshot(&bridge, snapshot),
            Err(error) => {
                push_connection_loss(&bridge, error.to_string());
                if !sleep_or_shutdown(&mut shutdown, retry_delay).await {
                    return;
                }
                continue;
            }
        }

        let mut stream = match client.stream_updates() {
            Ok(stream) => stream,
            Err(error) => {
                push_connection_loss(&bridge, error.to_string());
                if !sleep_or_shutdown(&mut shutdown, retry_delay).await {
                    return;
                }
                continue;
            }
        };

        let mut should_retry = false;
        let mut stream_attached = false;
        loop {
            let update = match next_update_or_shutdown(&mut stream, &mut shutdown).await {
                Some(update) => update,
                None => {
                    stream.close();
                    return;
                }
            };

            match update {
                Some(Ok(snapshot)) => {
                    if !stream_attached {
                        push_attached_snapshot(&bridge, snapshot);
                        stream_attached = true;
                    } else {
                        push_snapshot(&bridge, snapshot);
                    }
                }
                Some(Err(error)) => {
                    handle_bridge_error(&bridge, &error);
                    should_retry = true;
                    break;
                }
                None => break,
            }
        }

        stream.close();
        if !should_retry {
            push_connection_loss(&bridge, "control-plane stream closed".to_owned());
        }
        if !sleep_or_shutdown(&mut shutdown, retry_delay).await {
            return;
        }
    }
}

async fn fetch_snapshot_or_shutdown(
    client: &ControlPlaneClient,
    shutdown: &mut watch::Receiver<bool>,
) -> Option<Result<SnapshotEnvelope, ControlPlaneClientError>> {
    if shutdown_requested(shutdown) {
        return None;
    }

    tokio::select! {
        _ = shutdown.changed() => None,
        result = client.fetch_snapshot() => Some(result),
    }
}

async fn next_update_or_shutdown(
    stream: &mut opensymphony_control::ControlPlaneEventStream,
    shutdown: &mut watch::Receiver<bool>,
) -> Option<Option<Result<SnapshotEnvelope, ControlPlaneClientError>>> {
    if shutdown_requested(shutdown) {
        return None;
    }

    tokio::select! {
        _ = shutdown.changed() => None,
        update = stream.next() => Some(update),
    }
}

async fn sleep_or_shutdown(shutdown: &mut watch::Receiver<bool>, delay: Duration) -> bool {
    if shutdown_requested(shutdown) {
        return false;
    }

    tokio::select! {
        _ = shutdown.changed() => false,
        _ = tokio::time::sleep(delay) => true,
    }
}

fn shutdown_requested(shutdown: &watch::Receiver<bool>) -> bool {
    *shutdown.borrow()
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
        self.saw_live_stream |= matches!(self.state.connection, ConnectionState::Live);

        if self
            .exit_after
            .is_some_and(|limit| self.started_at.elapsed() >= limit)
        {
            if !self.saw_live_stream {
                let mut outcome = self
                    .outcome
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                outcome.timeout_before_live = Some(self.state.status_line.clone());
            }
            return Cmd::quit();
        }

        Cmd::none()
    }

    fn view(&self, frame: &mut Frame<'_>) {
        let content = self
            .state
            .render_text_styled(frame.width() as usize, frame.height() as usize);
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
    #[error("background control-plane bridge thread panicked during shutdown")]
    BridgeThreadPanicked,
    #[error("control-plane stream did not become live before exit: {0}")]
    AttachTimeout(String),
}

fn handle_bridge_error(bridge: &Arc<Mutex<BridgeMailbox>>, error: &ControlPlaneClientError) {
    push_connection_loss(bridge, error.to_string());
}

fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.format("%H:%M:%S").to_string()
}

fn connection_status_summary(state: &TuiState) -> String {
    let detail = match &state.connection {
        ConnectionState::Connecting => {
            if state.latest_snapshot.is_none() {
                None
            } else if state
                .status_line
                .eq_ignore_ascii_case("bootstrap snapshot loaded; waiting for live stream")
            {
                Some("stream pending")
            } else {
                informative_status(&state.status_line, &["connecting to control plane"])
            }
        }
        ConnectionState::Live => {
            informative_status(&state.status_line, &["live control-plane stream"])
        }
        ConnectionState::Reconnecting(reason) => {
            let reconnect_status_line = format!("reconnecting after: {reason}");
            if state
                .status_line
                .eq_ignore_ascii_case("snapshot refreshed; waiting for live stream")
            {
                Some("refreshed; stream pending")
            } else {
                informative_status(
                    &state.status_line,
                    &["reconnecting", reconnect_status_line.as_str()],
                )
                .or_else(|| informative_status(reason, &[]))
            }
        }
    };
    status_segment(format!("conn={}", state.connection.label()), detail)
}

fn daemon_status_summary(snapshot: &SnapshotEnvelope) -> String {
    let daemon = &snapshot.snapshot.daemon;
    status_segment(
        format!("daemon={}", daemon.state.as_str()),
        informative_status(
            &daemon.status_line,
            &[
                daemon.state.as_str(),
                "ready",
                "healthy",
                "scheduler heartbeat healthy",
            ],
        ),
    )
}

fn agent_server_status_summary(snapshot: &SnapshotEnvelope) -> String {
    let agent_server = &snapshot.snapshot.agent_server;
    let base = if agent_server.reachable {
        format!("agent=up/{}", agent_server.conversation_count)
    } else {
        "agent=down".to_owned()
    };
    status_segment(
        base,
        informative_status(
            &agent_server.status_line,
            &["healthy", "local agent-server healthy", "down"],
        ),
    )
}

fn status_segment(base: String, detail: Option<&str>) -> String {
    match detail {
        Some(detail) => format!("{base} ({detail})"),
        None => base,
    }
}

fn informative_status<'a>(status_line: &'a str, ignored: &[&str]) -> Option<&'a str> {
    let status_line = status_line.trim();
    if status_line.is_empty() {
        return None;
    }
    if ignored
        .iter()
        .any(|ignored| status_line.eq_ignore_ascii_case(ignored))
    {
        return None;
    }
    Some(status_line)
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

    if height <= HEADER_ROWS {
        return (0, 0);
    }

    let available = height.saturating_sub(HEADER_ROWS);
    if available <= TIMELINE_SEPARATOR_ROWS {
        return (available, 0);
    }

    let max_timeline_rows = available.saturating_sub(TIMELINE_SEPARATOR_ROWS + 1);
    let timeline_rows = min(
        min(MAX_TIMELINE_LINES, max_timeline_rows),
        max(MIN_TIMELINE_LINES, available / 3),
    );
    let body_rows = available.saturating_sub(TIMELINE_SEPARATOR_ROWS + timeline_rows);
    (body_rows, timeline_rows)
}

fn stacked_body_layout(body_rows: usize) -> (usize, usize) {
    const DETAIL_SEPARATOR_ROWS: usize = 1;
    const MIN_ISSUE_ROWS: usize = 4;
    const MIN_DETAIL_ROWS: usize = 8;

    if body_rows <= DETAIL_SEPARATOR_ROWS {
        return (body_rows, 0);
    }

    let available = body_rows.saturating_sub(DETAIL_SEPARATOR_ROWS);
    if available < MIN_ISSUE_ROWS + 2 {
        return (available, 0);
    }

    let detail_rows = min(
        max(MIN_DETAIL_ROWS, available / 2),
        available.saturating_sub(MIN_ISSUE_ROWS),
    );
    let issue_rows = available.saturating_sub(detail_rows);
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

fn visible_issue_count(max_rows: usize) -> usize {
    max(1, max_rows.saturating_sub(1))
}

fn issue_window(
    issue_count: usize,
    selected_issue: usize,
    visible_issue_count: usize,
) -> (usize, usize) {
    if issue_count == 0 {
        return (0, 0);
    }

    let visible_issue_count = min(max(1, visible_issue_count), issue_count);
    let last_start = issue_count.saturating_sub(visible_issue_count);
    let start = min(
        selected_issue.saturating_sub(visible_issue_count / 2),
        last_start,
    );
    let end = min(start + visible_issue_count, issue_count);
    (start, end)
}

fn push_snapshot(bridge: &Arc<Mutex<BridgeMailbox>>, snapshot: SnapshotEnvelope) {
    let mut bridge = bridge
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    bridge.push_snapshot(snapshot);
}

fn push_attached_snapshot(bridge: &Arc<Mutex<BridgeMailbox>>, snapshot: SnapshotEnvelope) {
    let mut bridge = bridge
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    bridge.push_attached_snapshot(snapshot);
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

    let value = single_line(value);
    let value_width = display_width(&value);
    if value_width == width {
        return value;
    }
    if value_width < width {
        return pad_to_width(value, width);
    }

    if width == 1 {
        return "~".to_owned();
    }

    let mut shortened = String::new();
    let max_width = width - 1;
    let mut shortened_width = 0;
    for ch in value.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if shortened_width + ch_width > max_width {
            break;
        }
        shortened.push(ch);
        shortened_width += ch_width;
    }
    shortened.push('~');
    pad_to_width(shortened, width)
}

fn single_line(value: &str) -> String {
    value
        .lines()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

fn pad_to_width(mut value: String, width: usize) -> String {
    let value_width = display_width(&value);
    if value_width < width {
        value.push_str(&" ".repeat(width - value_width));
    }
    value
}

fn runtime_state_color(state: &ControlPlaneIssueRuntimeState) -> PackedRgba {
    match state {
        ControlPlaneIssueRuntimeState::Running => GREEN,
        ControlPlaneIssueRuntimeState::Failed => RED,
        ControlPlaneIssueRuntimeState::Idle => YELLOW,
        ControlPlaneIssueRuntimeState::Completed => CYAN,
        ControlPlaneIssueRuntimeState::RetryQueued => BRIGHT_YELLOW,
        ControlPlaneIssueRuntimeState::Releasing => MAGENTA,
    }
}

fn event_kind_style(kind: &str) -> Style {
    match kind {
        "tool" | "tool_call" | "tool_use" | "ActionEvent" => Style::new().fg(BLUE),
        "message" | "MessageEvent" => Style::new().fg(CYAN),
        "error" | "ConversationErrorEvent" => Style::new().fg(RED),
        "assistant" => Style::new().fg(GREEN),
        "user" => Style::new().fg(YELLOW),
        "state" | "ConversationStateUpdateEvent" => Style::new().dim(),
        _ => Style::new(),
    }
}

fn two_column_block_styled(
    left: &[Line],
    right: &[Line],
    left_width: usize,
    right_width: usize,
) -> Vec<Line> {
    let row_count = max(left.len(), right.len());
    (0..row_count)
        .map(|index| {
            let left_line = left.get(index);
            let right_line = right.get(index);

            // Build combined spans preserving original styling
            let mut spans: Vec<Span<'_>> = Vec::new();

            // Add left line spans (or empty if none)
            if let Some(line) = left_line {
                // Get spans from the line, truncated to width
                let mut line_width = 0;
                for span in line.spans() {
                    let span_text = span.as_str();
                    let remaining = left_width.saturating_sub(line_width);
                    if remaining == 0 {
                        break;
                    }
                    let truncated = if span_text.width() > remaining {
                        &span_text[..span_text
                            .char_indices()
                            .take_while(|(i, _)| *i <= remaining)
                            .last()
                            .map(|(i, _)| i)
                            .unwrap_or(0)]
                    } else {
                        span_text
                    };
                    spans.push(Span::styled(
                        truncated.to_string(),
                        span.style.unwrap_or_default(),
                    ));
                    line_width += truncated.width();
                }
                // Pad if needed
                if line_width < left_width {
                    spans.push(Span::raw(" ".repeat(left_width - line_width)));
                }
            } else {
                spans.push(Span::raw(" ".repeat(left_width)));
            }

            // Add separator
            spans.push(Span::raw(" | "));

            // Add right line spans (or empty if none)
            if let Some(line) = right_line {
                let mut line_width = 0;
                for span in line.spans() {
                    let span_text = span.as_str();
                    let remaining = right_width.saturating_sub(line_width);
                    if remaining == 0 {
                        break;
                    }
                    let truncated = if span_text.width() > remaining {
                        &span_text[..span_text
                            .char_indices()
                            .take_while(|(i, _)| *i <= remaining)
                            .last()
                            .map(|(i, _)| i)
                            .unwrap_or(0)]
                    } else {
                        span_text
                    };
                    spans.push(Span::styled(
                        truncated.to_string(),
                        span.style.unwrap_or_default(),
                    ));
                    line_width += truncated.width();
                }
                // Pad if needed
                if line_width < right_width {
                    spans.push(Span::raw(" ".repeat(right_width - line_width)));
                }
            } else {
                spans.push(Span::raw(" ".repeat(right_width)));
            }

            Line::from_spans(spans)
        })
        .collect()
}

fn fit_section_styled(mut lines: Vec<Line>, max_rows: usize, width: usize) -> Vec<Line> {
    if max_rows == 0 {
        return Vec::new();
    }

    if lines.len() > max_rows {
        lines.truncate(max_rows);
        if let Some(last) = lines.last_mut() {
            *last = Line::from(Span::styled("...", Style::new().dim()));
        }
    }

    while lines.len() < max_rows {
        lines.push(Line::from(Span::raw(" ".repeat(width))));
    }

    lines
}

trait RuntimeStateLabel {
    fn as_str(&self) -> &'static str;
}

impl RuntimeStateLabel for ControlPlaneIssueRuntimeState {
    fn as_str(&self) -> &'static str {
        match self {
            ControlPlaneIssueRuntimeState::Idle => "idle",
            ControlPlaneIssueRuntimeState::Running => "running",
            ControlPlaneIssueRuntimeState::RetryQueued => "retry_queued",
            ControlPlaneIssueRuntimeState::Releasing => "releasing",
            ControlPlaneIssueRuntimeState::Completed => "completed",
            ControlPlaneIssueRuntimeState::Failed => "failed",
        }
    }
}

trait WorkerOutcomeLabel {
    fn as_str(&self) -> &'static str;
}

impl WorkerOutcomeLabel for opensymphony_domain::ControlPlaneWorkerOutcome {
    fn as_str(&self) -> &'static str {
        match self {
            opensymphony_domain::ControlPlaneWorkerOutcome::Unknown => "unknown",
            opensymphony_domain::ControlPlaneWorkerOutcome::Running => "running",
            opensymphony_domain::ControlPlaneWorkerOutcome::Continued => "continued",
            opensymphony_domain::ControlPlaneWorkerOutcome::Completed => "completed",
            opensymphony_domain::ControlPlaneWorkerOutcome::Failed => "failed",
            opensymphony_domain::ControlPlaneWorkerOutcome::Canceled => "canceled",
        }
    }
}

trait DaemonStateLabel {
    fn as_str(&self) -> &'static str;
}

impl DaemonStateLabel for opensymphony_domain::ControlPlaneDaemonState {
    fn as_str(&self) -> &'static str {
        match self {
            opensymphony_domain::ControlPlaneDaemonState::Starting => "starting",
            opensymphony_domain::ControlPlaneDaemonState::Ready => "ready",
            opensymphony_domain::ControlPlaneDaemonState::Degraded => "degraded",
            opensymphony_domain::ControlPlaneDaemonState::Stopped => "stopped",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AppMessage, BridgeHandle, BridgeMailbox, ConnectionState, ControlPlaneClientError,
        OperatorApp, RunOutcome, TuiAction, TuiState, display_width, fit, handle_bridge_error,
        issue_window, section_layout, stacked_body_layout, visible_issue_count,
    };
    use chrono::{TimeZone, Utc};
    use ftui::prelude::Model;
    use opensymphony_domain::{
        ControlPlaneAgentServerStatus as AgentServerStatus, ControlPlaneConversationEvent,
        ControlPlaneDaemonSnapshot as DaemonSnapshot, ControlPlaneDaemonState as DaemonState,
        ControlPlaneDaemonStatus as DaemonStatus, ControlPlaneFileChange,
        ControlPlaneFileChangeKind, ControlPlaneIssueRuntimeState as IssueRuntimeState,
        ControlPlaneIssueSnapshot as IssueSnapshot, ControlPlaneMetricsSnapshot as MetricsSnapshot,
        ControlPlaneRecentEvent as RecentEvent, ControlPlaneRecentEventKind as RecentEventKind,
        ControlPlaneWorkerOutcome as WorkerOutcome, SnapshotEnvelope,
    };
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };
    use tracing::{
        Event, Id, Metadata, Subscriber,
        span::{Attributes, Record},
    };
    use url::Url;

    struct EventCounter {
        events: Arc<AtomicUsize>,
    }

    impl Subscriber for EventCounter {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, _event: &Event<'_>) {
            self.events.fetch_add(1, Ordering::SeqCst);
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    fn fixture(sequence: u64, issue_count: usize) -> SnapshotEnvelope {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 21, 20, 0, 0)
            .single()
            .expect("valid fixed test timestamp")
            + chrono::Duration::seconds(sequence as i64);
        SnapshotEnvelope {
            sequence,
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
                        server_base_url: Some("http://127.0.0.1:3000".to_owned()),
                        transport_target: Some("loopback".to_owned()),
                        http_auth_mode: Some("none".to_owned()),
                        websocket_auth_mode: Some("none".to_owned()),
                        websocket_query_param_name: None,
                        recent_events: vec![
                            ControlPlaneConversationEvent {
                                event_id: format!("evt-{}-1", index),
                                happened_at: now,
                                kind: "tool_call".to_owned(),
                                summary: "editing src/main.rs".to_owned(),
                            },
                            ControlPlaneConversationEvent {
                                event_id: format!("evt-{}-2", index),
                                happened_at: now,
                                kind: "message".to_owned(),
                                summary: "implementing feature".to_owned(),
                            },
                        ],
                        modified_files: vec![
                            ControlPlaneFileChange {
                                path: format!("workspace-{}/src/main.rs", index),
                                change_kind: ControlPlaneFileChangeKind::Modified,
                                lines_added: 42,
                                lines_removed: 10,
                            },
                            ControlPlaneFileChange {
                                path: format!("workspace-{}/src/lib.rs", index),
                                change_kind: ControlPlaneFileChangeKind::Created,
                                lines_added: 100,
                                lines_removed: 0,
                            },
                        ],
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
        state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(8, 12))));

        let rendered = state.render_text(100, 22);
        assert!(rendered.contains("RECENT EVENTS"));
        assert!(rendered.contains("snapshot updated"));
        assert_eq!(rendered.lines().count(), 22);
    }

    #[test]
    fn coalesces_bridge_snapshots_to_latest_value() {
        let mut mailbox = BridgeMailbox::default();
        let first = fixture(1, 1);
        let second = fixture(3, 1);

        mailbox.push_snapshot(first);
        mailbox.push_snapshot(second.clone());

        match mailbox.take_action() {
            Some(TuiAction::SnapshotReceived(snapshot)) => {
                assert_eq!(*snapshot, second);
            }
            other => panic!("expected latest snapshot, got {other:?}"),
        }
        assert!(mailbox.take_action().is_none());
    }

    #[test]
    fn keeps_latest_snapshot_when_connection_drops() {
        let mut mailbox = BridgeMailbox::default();
        let snapshot = fixture(5, 1);

        mailbox.push_snapshot(snapshot.clone());
        mailbox.push_connection_loss("stream closed".to_owned());

        match mailbox.take_action() {
            Some(TuiAction::SnapshotReceived(received)) => {
                assert_eq!(*received, snapshot);
            }
            other => panic!("expected latest snapshot, got {other:?}"),
        }

        match mailbox.take_action() {
            Some(TuiAction::ConnectionLost(reason)) => assert_eq!(reason, "stream closed"),
            other => panic!("expected reconnecting state, got {other:?}"),
        }
    }

    #[test]
    fn delivers_attached_snapshot_before_the_live_transition() {
        let mut mailbox = BridgeMailbox::default();
        let snapshot = fixture(5, 1);

        mailbox.push_attached_snapshot(snapshot.clone());

        match mailbox.take_action() {
            Some(TuiAction::SnapshotReceived(received)) => {
                assert_eq!(*received, snapshot);
            }
            other => panic!("expected latest snapshot, got {other:?}"),
        }

        match mailbox.take_action() {
            Some(TuiAction::StreamAttached) => {}
            other => panic!("expected stream attachment, got {other:?}"),
        }
    }

    #[test]
    fn connection_loss_clears_pending_stream_attachment() {
        let mut mailbox = BridgeMailbox {
            stream_attached: true,
            ..BridgeMailbox::default()
        };
        mailbox.push_connection_loss("stream closed".to_owned());

        match mailbox.take_action() {
            Some(TuiAction::ConnectionLost(reason)) => assert_eq!(reason, "stream closed"),
            other => panic!("expected reconnecting state, got {other:?}"),
        }
        assert!(mailbox.take_action().is_none());
    }

    #[test]
    fn draining_an_attached_snapshot_never_marks_live_on_the_old_snapshot() {
        let bridge = Arc::new(Mutex::new(BridgeMailbox::default()));
        let outcome = Arc::new(Mutex::new(RunOutcome::default()));
        let mut app = OperatorApp::new(Arc::clone(&bridge), None, outcome);
        app.state
            .reduce(TuiAction::SnapshotReceived(Box::new(fixture(4, 1))));

        {
            let mut mailbox = bridge
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            mailbox.push_attached_snapshot(fixture(5, 2));
        }

        app.drain_bridge();

        assert_eq!(app.state.connection, ConnectionState::Live);
        assert_eq!(
            app.state
                .latest_snapshot
                .as_ref()
                .expect("latest snapshot after the live transition")
                .sequence,
            5
        );
        assert_eq!(app.state.status_line, "live control-plane stream");
    }

    #[test]
    fn timed_exit_records_a_failure_until_a_live_stream_is_seen() {
        let bridge = Arc::new(Mutex::new(BridgeMailbox::default()));
        let outcome = Arc::new(Mutex::new(RunOutcome::default()));
        let mut app = OperatorApp::new(
            Arc::clone(&bridge),
            Some(Duration::ZERO),
            Arc::clone(&outcome),
        );

        let _ = app.update(AppMessage::Tick);

        let outcome = outcome.lock().expect("run outcome should stay unlocked");
        assert_eq!(
            outcome.timeout_before_live.as_deref(),
            Some("connecting to control plane")
        );
    }

    #[test]
    fn keeps_detail_visible_in_narrow_layouts() {
        let mut state = TuiState::default();
        state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(8, 12))));

        let rendered = state.render_text(72, 22);
        assert!(rendered.contains("[ ] ISSUE + WORKSPACE DETAIL"));
        assert!(rendered.contains("workspace: workspace-0"));
        assert!(rendered.contains("RECENT EVENTS"));
    }

    #[test]
    fn visible_issue_count_reserves_header_row() {
        assert_eq!(visible_issue_count(0), 1);
        assert_eq!(visible_issue_count(4), 3);
        assert_eq!(visible_issue_count(13), 12);
    }

    #[test]
    fn issue_window_keeps_selected_issue_inside_the_visible_slice() {
        assert_eq!(issue_window(12, 0, 6), (0, 6));
        assert_eq!(issue_window(12, 7, 6), (4, 10));
        assert_eq!(issue_window(12, 11, 6), (6, 12));
    }

    #[test]
    fn fit_collapses_embedded_newlines_before_padding() {
        assert_eq!(fit("a\nb", 4), "a b ");
        assert_eq!(fit("a\r\nb", 4), "a b ");
    }

    #[test]
    fn multiline_event_text_does_not_expand_the_frame_row_budget() {
        let mut state = TuiState::default();
        let mut snapshot = fixture(8, 12);
        snapshot.snapshot.recent_events[0].summary = "first line\nsecond line".to_owned();
        state.reduce(TuiAction::SnapshotReceived(Box::new(snapshot)));

        let rendered = state.render_text(100, 22);
        assert_eq!(rendered.lines().count(), 22);
        assert!(rendered.contains("first line second line"));
        assert!(!rendered.contains("first line\nsecond line"));
    }

    #[test]
    fn compact_issue_rows_show_more_issues_in_default_inline_layout() {
        let mut state = TuiState::default();
        state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(8, 12))));

        let lines = state.issue_lines(100, 13);

        assert_eq!(lines.len(), 13);
        assert!(lines[1].contains("COE-255"));
        assert!(lines[1].contains("Issue 0"));
        assert!(lines[12].contains("COE-266"));
        assert!(lines[12].contains("Issue 11"));
    }

    #[test]
    fn header_surfaces_daemon_and_agent_health() {
        let mut state = TuiState::default();
        let mut snapshot = fixture(8, 3);
        snapshot.snapshot.daemon.state = DaemonState::Degraded;
        snapshot.snapshot.agent_server.reachable = false;
        state.reduce(TuiAction::SnapshotReceived(Box::new(snapshot)));

        let rendered = state.render_text(140, 22);
        let header = rendered.lines().next().expect("header row");
        assert!(header.contains("daemon=degraded"));
        assert!(header.contains("agent=down"));
    }

    #[test]
    fn header_renders_connection_and_backend_status_text() {
        let mut state = TuiState::default();
        let mut snapshot = fixture(8, 3);
        snapshot.snapshot.daemon.state = DaemonState::Degraded;
        snapshot.snapshot.daemon.status_line = "scheduler poll overdue".to_owned();
        snapshot.snapshot.agent_server.reachable = false;
        snapshot.snapshot.agent_server.status_line = "agent-server refused connection".to_owned();

        state.reduce(TuiAction::SnapshotReceived(Box::new(snapshot)));
        state.reduce(TuiAction::ConnectionLost("sse stalled".to_owned()));

        let rendered = state.render_text(200, 22);
        let header = rendered.lines().next().expect("header row");
        assert!(header.contains("conn=reconnecting (sse stalled)"));
        assert!(header.contains("daemon=degraded (scheduler poll overdue)"));
        assert!(header.contains("agent=down (agent-server refused connection)"));
    }

    #[test]
    fn reconnecting_header_prefers_refreshed_snapshot_status_text() {
        let mut state = TuiState::default();
        state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(8, 3))));
        state.reduce(TuiAction::StreamAttached);
        state.reduce(TuiAction::ConnectionLost("sse stalled".to_owned()));
        state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(9, 3))));

        let rendered = state.render_text(200, 22);
        let header = rendered.lines().next().expect("header row");
        assert!(header.contains("conn=reconnecting (refreshed; stream pending)"));
        assert!(!header.contains("conn=reconnecting (sse stalled)"));
    }

    #[test]
    fn fit_uses_terminal_cell_width_for_padding_and_truncation() {
        assert_eq!(fit("界", 4), "界  ");
        assert_eq!(fit("界abc", 4), "界a~");
    }

    #[test]
    fn fit_replaces_control_characters_before_padding() {
        assert_eq!(fit("a\tb", 4), "a b ");
        assert_eq!(fit("a\u{0007}b", 4), "a b ");
    }

    #[test]
    fn wide_glyphs_stay_within_the_frame_width_budget() {
        let mut state = TuiState::default();
        let mut snapshot = fixture(8, 3);
        snapshot.snapshot.issues[0].title = "界面 dashboard".to_owned();
        snapshot.snapshot.recent_events[0].summary = "多字节 health event".to_owned();
        state.reduce(TuiAction::SnapshotReceived(Box::new(snapshot)));

        let rendered = state.render_text(40, 22);
        assert!(rendered.lines().all(|line| display_width(line) <= 40));
        assert!(rendered.contains("界面"));
        assert!(rendered.contains("多字节"));
    }

    #[test]
    fn control_characters_do_not_escape_the_frame_width_budget() {
        let mut state = TuiState::default();
        let mut snapshot = fixture(8, 3);
        snapshot.snapshot.issues[0].title = "tab\tseparated".to_owned();
        snapshot.snapshot.recent_events[0].summary = "bell\u{0007}event".to_owned();
        state.reduce(TuiAction::SnapshotReceived(Box::new(snapshot)));

        let rendered = state.render_text(40, 22);
        assert!(rendered.lines().all(|line| display_width(line) <= 40));
        assert!(!rendered.contains('\t'));
        assert!(!rendered.contains('\u{0007}'));
        assert!(rendered.contains("tab separated"));
        assert!(rendered.contains("bell event"));
    }

    #[test]
    fn handle_bridge_error_only_queues_connection_loss_without_tracing_output() {
        let bridge = Arc::new(Mutex::new(BridgeMailbox::default()));
        let event_count = Arc::new(AtomicUsize::new(0));
        let subscriber = EventCounter {
            events: Arc::clone(&event_count),
        };
        let error = ControlPlaneClientError::InvalidBaseUrl {
            base_url: "http://127.0.0.1:4010".to_owned(),
            path: "api/v1/events",
            source: url::ParseError::RelativeUrlWithoutBase,
        };

        tracing::subscriber::with_default(subscriber, || handle_bridge_error(&bridge, &error));

        assert_eq!(event_count.load(Ordering::SeqCst), 0);

        let mut bridge = bridge
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match bridge.take_action() {
            Some(TuiAction::ConnectionLost(reason)) => assert_eq!(reason, error.to_string()),
            other => panic!("expected reconnecting state, got {other:?}"),
        }
    }

    #[test]
    fn shutdown_joins_the_background_bridge_thread() {
        let bridge = BridgeHandle::spawn(
            Url::parse("http://127.0.0.1:9/").expect("valid test control-plane base url"),
        );
        let (done_tx, done_rx) = mpsc::channel();

        thread::spawn(move || {
            let _ = done_tx.send(bridge.shutdown());
        });

        match done_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => panic!("expected clean bridge shutdown, got {error}"),
            Err(_) => panic!("bridge shutdown did not complete promptly"),
        }
    }

    #[test]
    fn reserves_rows_for_the_timeline_section() {
        let (body_rows, timeline_rows) = section_layout(22);
        assert_eq!((body_rows, timeline_rows), (13, 6));
    }

    #[test]
    fn reserves_rows_for_detail_in_narrow_layout() {
        let (issue_rows, detail_rows) = stacked_body_layout(13);
        assert_eq!((issue_rows, detail_rows), (4, 8));
    }
}
