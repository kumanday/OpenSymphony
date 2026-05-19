use std::{
    cmp::{max, min},
    collections::{HashMap, HashSet, VecDeque},
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use crate::opensymphony_control::{ControlPlaneClient, ControlPlaneClientError};
use crate::opensymphony_domain::{
    ControlPlaneIssueRuntimeState, ControlPlaneIssueSnapshot as IssueSnapshot,
    ControlPlaneMetricsSnapshot as MetricsSnapshot, ControlPlaneRecentEvent as RecentEvent,
    SnapshotEnvelope,
};
use chrono::{DateTime, Utc};
use crossterm::terminal;
use ftui::{
    ProgramConfig, ResizeBehavior, RuntimeDiffConfig, Style,
    core::geometry::Rect,
    prelude::{Cmd, Event, Frame, KeyCode, Model},
    render::budget::{FrameBudgetConfig, PhaseBudgets},
    render::cell::PackedRgba,
    runtime::{Every, Subscription},
    text::text::{Line, Span, Text},
    widgets::{Widget, paragraph::Paragraph},
};
use thiserror::Error;
use tokio::sync::watch;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use url::Url;

const MIN_TIMELINE_LINES: usize = 4;
const MAX_TIMELINE_LINES: usize = 6;
const MIN_TUI_WIDTH: u16 = 20;
const MIN_TUI_HEIGHT: u16 = 8;
const FALLBACK_TUI_WIDTH: u16 = 120;
const FALLBACK_TUI_HEIGHT: u16 = 40;
const TUI_SIZE_WAIT: Duration = Duration::from_millis(1_500);
const TUI_CONVERSATION_TEXT_LIMIT: usize = 260;

const RED: PackedRgba = PackedRgba::rgb(205, 0, 0);
const GREEN: PackedRgba = PackedRgba::rgb(0, 205, 0);
const YELLOW: PackedRgba = PackedRgba::rgb(205, 205, 0);
const BLUE: PackedRgba = PackedRgba::rgb(0, 255, 255); // Bright cyan instead of dark blue
const MAGENTA: PackedRgba = PackedRgba::rgb(205, 0, 205);
const CYAN: PackedRgba = PackedRgba::rgb(0, 205, 205);
const BRIGHT_GREEN: PackedRgba = PackedRgba::rgb(0, 255, 0);
const BRIGHT_YELLOW: PackedRgba = PackedRgba::rgb(255, 255, 0);
const BRIGHT_BLACK: PackedRgba = PackedRgba::rgb(127, 127, 127);

/// Format a number with k/M/B/T suffix for thousands/millions/billions/trillions.
/// Uses 2 decimal places for values >= 100, 1 decimal place for values >= 10, none for smaller.
fn format_metric(num: u64) -> String {
    if num >= 1_000_000_000_000 {
        format!("{:.2}T", num as f64 / 1_000_000_000_000.0)
    } else if num >= 1_000_000_000 {
        format!("{:.2}B", num as f64 / 1_000_000_000.0)
    } else if num >= 1_000_000 {
        format!("{:.2}M", num as f64 / 1_000_000.0)
    } else if num >= 1_000 {
        format!("{:.1}k", num as f64 / 1_000.0)
    } else {
        format!("{}", num)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiState {
    pub focus: FocusPane,
    pub timeline_mode: TimelineMode,
    pub connection: ConnectionState,
    pub selected_issue: usize,
    pub latest_snapshot: Option<SnapshotEnvelope>,
    pub status_line: String,
    workspace_status: HashMap<String, WorkspaceStatusEntry>,
    selected_changed_file: usize,
    detail_diff_open: bool,
    detail_issue_identifier: Option<String>,
    conversation_scroll_offset: usize,
    diff_scroll_offset: usize,
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
            workspace_status: HashMap::new(),
            selected_changed_file: 0,
            detail_diff_open: false,
            detail_issue_identifier: None,
            conversation_scroll_offset: 0,
            diff_scroll_offset: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceStatusData {
    branch: String,
    pr_url: Option<String>,
    changes: WorkspaceChangeState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkspaceStatusEntry {
    Loading,
    Loaded(WorkspaceStatusData),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkspacePrDisplay {
    Loading,
    Available(String),
    None,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceChangeState {
    Available(WorkspaceChangeSummary),
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceChangeSummary {
    pub files_changed: usize,
    pub additions: u64,
    pub deletions: u64,
    pub files: Vec<WorkspaceFileChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFileChange {
    pub display_path: String,
    pub query_path: String,
    pub previous_path: Option<String>,
    pub status_code: String,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub diff: WorkspaceFileDiffState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceFileDiffState {
    Unloaded,
    Loading,
    Loaded(Vec<WorkspaceDiffLine>),
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDiffLine {
    pub kind: WorkspaceDiffLineKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceDiffLineKind {
    Header,
    Hunk,
    Addition,
    Deletion,
    Context,
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SelectedDiffDisplay {
    Closed,
    Loading(String),
    Available {
        title: String,
        additions: Option<u64>,
        deletions: Option<u64>,
        lines: Vec<WorkspaceDiffLine>,
    },
    Unavailable(String),
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
                self.retain_workspace_status_for_visible_issues();
                self.sync_detail_state();
            }
            TuiAction::StreamAttached => {
                self.connection = ConnectionState::Live;
                self.status_line = "live control-plane stream".to_owned();
            }
            TuiAction::ConnectionLost(reason) => {
                self.connection = ConnectionState::Reconnecting(reason.clone());
                self.status_line = format!("reconnecting after: {reason}");
            }
            TuiAction::MoveSelectionUp => match self.focus {
                FocusPane::Issues => self.move_issue_selection_up(),
                FocusPane::Detail => self.move_changed_file_selection_up(),
                FocusPane::Activity => {
                    if self.detail_diff_open {
                        self.move_diff_scroll_up();
                    } else {
                        self.move_conversation_scroll_up();
                    }
                }
            },
            TuiAction::MoveSelectionDown => match self.focus {
                FocusPane::Issues => self.move_issue_selection_down(),
                FocusPane::Detail => self.move_changed_file_selection_down(),
                FocusPane::Activity => {
                    if self.detail_diff_open {
                        self.move_diff_scroll_down();
                    } else {
                        self.move_conversation_scroll_down();
                    }
                }
            },
            TuiAction::FocusNext => {
                self.focus = match self.focus {
                    FocusPane::Issues => FocusPane::Detail,
                    FocusPane::Detail => FocusPane::Activity,
                    FocusPane::Activity => FocusPane::Issues,
                };
            }
            TuiAction::FocusPrevious => {
                self.focus = match self.focus {
                    FocusPane::Issues => FocusPane::Activity,
                    FocusPane::Detail => FocusPane::Issues,
                    FocusPane::Activity => FocusPane::Detail,
                };
            }
            TuiAction::ToggleDetailDiff => {
                if matches!(self.focus, FocusPane::Detail | FocusPane::Activity)
                    && self.selected_file_change().is_some()
                {
                    self.detail_diff_open = !self.detail_diff_open;
                    if self.detail_diff_open {
                        self.focus = FocusPane::Activity;
                        self.diff_scroll_offset = 0;
                    } else {
                        self.diff_scroll_offset = 0;
                    }
                }
                self.sync_detail_state();
            }
            TuiAction::ToggleTimelineMode => {
                self.timeline_mode = match self.timeline_mode {
                    TimelineMode::Events => TimelineMode::Metrics,
                    TimelineMode::Metrics => TimelineMode::Events,
                };
            }
            TuiAction::WorkspaceStatusRequested(issue_identifier) => {
                self.workspace_status
                    .entry(issue_identifier)
                    .or_insert(WorkspaceStatusEntry::Loading);
            }
            TuiAction::WorkspaceStatusLoaded {
                issue_identifier,
                branch,
                pr_url,
                changes,
            } => {
                let changes = self.merge_workspace_changes(&issue_identifier, changes);
                self.workspace_status.insert(
                    issue_identifier,
                    WorkspaceStatusEntry::Loaded(WorkspaceStatusData {
                        branch,
                        pr_url,
                        changes,
                    }),
                );
                self.sync_detail_state();
            }
            TuiAction::WorkspaceDiffRequested {
                issue_identifier,
                query_path,
            } => {
                self.set_file_diff_state(
                    &issue_identifier,
                    &query_path,
                    WorkspaceFileDiffState::Loading,
                );
            }
            TuiAction::WorkspaceDiffLoaded {
                issue_identifier,
                query_path,
                diff,
            } => {
                let diff_state = match diff {
                    Ok(lines) => WorkspaceFileDiffState::Loaded(lines),
                    Err(message) => WorkspaceFileDiffState::Unavailable(message),
                };
                self.set_file_diff_state(&issue_identifier, &query_path, diff_state);
                self.sync_detail_state();
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
        header.push("q quit  tab focus  shift-tab back  enter diff  e toggle".to_owned());
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

        let mut lines = Vec::new();
        let snapshot = self.latest_snapshot.as_ref();
        let issue_count = snapshot
            .map(|value| value.snapshot.issues.len())
            .unwrap_or_default();
        let sequence = snapshot.map(|value| value.sequence).unwrap_or_default();
        let generated = snapshot
            .map(|value| format_timestamp(value.snapshot.generated_at))
            .unwrap_or_else(|| "--:--:--".to_owned());

        // Header (2 rows)
        lines.push(self.header_line_styled(width, snapshot, sequence, &generated, issue_count));
        lines.push(Line::from(Span::styled(
            "=".repeat(width),
            Style::new().dim(),
        )));

        // Calculate section heights
        let available_rows = height.saturating_sub(2); // After header
        let split_rows = available_rows.saturating_sub(1); // separator between upper and lower panes
        let upper_section_rows = split_rows / 2;
        let bottom_section_rows = split_rows.saturating_sub(upper_section_rows);

        // Width configuration for recent events panel
        // Must accommodate: "HH:MM:SS snapshot_published polled tracker; running=N, retry_queue=N"
        const RECENT_EVENTS_MIN_WIDTH: usize = 70; // 68 chars + padding
        const RECENT_EVENTS_MAX_WIDTH: usize = 75; // Cap at reasonable max
        let upper_right_width = (width / 5).clamp(RECENT_EVENTS_MIN_WIDTH, RECENT_EVENTS_MAX_WIDTH);
        let upper_left_width = width.saturating_sub(upper_right_width + 3);

        if width >= 100 {
            // Three-section layout:
            // Upper: Left (issues remaining) | Right (recent events fixed ~70 chars)
            // Bottom: Left (metadata 30%) | Right (conversation 70%)

            let bottom_left_width = width * 3 / 10;
            let bottom_right_width = width.saturating_sub(bottom_left_width + 3);

            // Upper section: Issues | Recent Events
            let issues_lines = self.issue_lines_styled(upper_left_width, upper_section_rows);
            let events_lines = self.timeline_lines_styled(upper_right_width, upper_section_rows);
            lines.extend(fit_section_styled(
                two_column_block_styled(
                    &issues_lines,
                    &events_lines,
                    upper_left_width,
                    upper_right_width,
                ),
                upper_section_rows,
                width,
            ));

            // Separator
            lines.push(Line::from(Span::styled(
                "-".repeat(width),
                Style::new().dim(),
            )));

            // Bottom section: Metadata + Modified Files | Conversation Activity
            let meta_lines = self.metadata_and_files_lines(bottom_left_width, bottom_section_rows);
            let activity_lines =
                self.bottom_right_lines_styled(bottom_right_width, bottom_section_rows);
            lines.extend(fit_section_styled(
                two_column_block_styled(
                    &meta_lines,
                    &activity_lines,
                    bottom_left_width,
                    bottom_right_width,
                ),
                bottom_section_rows,
                width,
            ));
        } else if width >= 80 {
            // Two-section layout for medium width
            // Use same fixed-width for recent events

            // Upper: Issues | Recent Events (fixed width)
            let issues_lines = self.issue_lines_styled(upper_left_width, upper_section_rows);
            let events_lines = self.timeline_lines_styled(upper_right_width, upper_section_rows);
            lines.extend(fit_section_styled(
                two_column_block_styled(
                    &issues_lines,
                    &events_lines,
                    upper_left_width,
                    upper_right_width,
                ),
                upper_section_rows,
                width,
            ));

            // Separator
            lines.push(Line::from(Span::styled(
                "-".repeat(width),
                Style::new().dim(),
            )));

            // Bottom: Selected issue detail (full width)
            let detail_lines = self.detail_lines_styled(width, bottom_section_rows);
            lines.extend(fit_section_styled(detail_lines, bottom_section_rows, width));
        } else {
            // Narrow layout: stacked vertically
            let (upper_rows, bottom_rows) = (upper_section_rows, bottom_section_rows);

            lines.extend(fit_section_styled(
                self.issue_lines_styled(width, upper_rows),
                upper_rows,
                width,
            ));

            if bottom_rows > 0 {
                lines.push(Line::from(Span::styled(
                    "-".repeat(width),
                    Style::new().dim(),
                )));
                lines.extend(fit_section_styled(
                    self.detail_lines_styled(width, bottom_rows),
                    bottom_rows,
                    width,
                ));
            }
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
                crate::opensymphony_domain::ControlPlaneDaemonState::Ready => {
                    Style::new().fg(GREEN)
                }
                crate::opensymphony_domain::ControlPlaneDaemonState::Starting => {
                    Style::new().fg(YELLOW)
                }
                crate::opensymphony_domain::ControlPlaneDaemonState::Degraded => {
                    Style::new().fg(RED)
                }
                crate::opensymphony_domain::ControlPlaneDaemonState::Stopped => {
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

        // Add token information (always show, even when 0)
        if let Some(snap) = snapshot {
            let metrics = &snap.snapshot.metrics;
            let cache_suffix = if metrics.cache_read_tokens > 0 {
                format!(" ({} cache)", format_metric(metrics.cache_read_tokens))
            } else {
                String::new()
            };
            spans.push(Span::raw(" | "));
            spans.push(Span::styled(
                format!(
                    "{} input{}",
                    format_metric(metrics.input_tokens),
                    cache_suffix
                ),
                Style::new().fg(GREEN),
            ));
            spans.push(Span::raw(", "));
            spans.push(Span::styled(
                format!("{} output", format_metric(metrics.output_tokens)),
                Style::new().fg(CYAN),
            ));
            spans.push(Span::raw(", "));
            spans.push(Span::styled(
                format!("{} total", format_metric(metrics.total_tokens)),
                Style::new().fg(CYAN),
            ));
        }

        spans.push(Span::raw(" | "));
        spans.push(Span::styled(
            format!("updated={generated}"),
            Style::new().dim(),
        ));
        spans.push(Span::raw(" | "));
        spans.push(Span::styled(
            "q quit  tab focus  shift-tab back  enter diff  e toggle",
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

        Line::from_spans(vec![
            Span::styled(marker, marker_style),
            Span::styled(" ", base_style),
            Span::styled(&issue.identifier, id_style),
            Span::styled(" [", base_style),
            Span::styled(issue.runtime_state.as_str(), state_style),
            Span::styled(" / ", base_style),
            Span::styled(&issue.tracker_state, tracker_style),
            Span::styled("] ", base_style),
            Span::styled(&issue.title, title_style),
        ])
    }

    #[allow(dead_code)]
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

                let branch = self.branch_text(issue);
                lines.push(Line::from_spans(vec![
                    Span::styled("branch: ", Style::new().dim()),
                    Span::styled(branch, Style::new().fg(CYAN)),
                ]));

                lines.push(self.pr_line_styled(issue));

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

                if lines.len() < max_rows {
                    lines.push(Line::from(Span::styled(
                        "-".repeat(width.min(40)),
                        Style::new().dim(),
                    )));
                    let remaining_rows = max_rows.saturating_sub(lines.len());
                    if matches!(self.selected_diff_display(), SelectedDiffDisplay::Closed) {
                        let file_rows = remaining_rows.min(max(4, remaining_rows / 2));
                        lines.extend(self.modified_files_lines_styled(width, issue, file_rows));

                        let conversation_rows = max_rows.saturating_sub(lines.len() + 1);
                        if conversation_rows > 0 {
                            lines.push(Line::from(Span::styled(
                                "-".repeat(width.min(40)),
                                Style::new().dim(),
                            )));
                            lines
                                .extend(self.conversation_activity_lines(width, conversation_rows));
                        }
                    } else {
                        let file_rows = remaining_rows.min(max(4, remaining_rows / 3));
                        lines.extend(self.modified_files_lines_styled(width, issue, file_rows));

                        let diff_rows = max_rows.saturating_sub(lines.len() + 1);
                        if diff_rows > 0 {
                            lines.push(Line::from(Span::styled(
                                "-".repeat(width.min(40)),
                                Style::new().dim(),
                            )));
                            lines.extend(self.selected_diff_lines_styled(width, diff_rows));
                        }
                    }
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

    fn modified_files_lines_styled(
        &self,
        width: usize,
        issue: &IssueSnapshot,
        max_rows: usize,
    ) -> Vec<Line> {
        let mut lines = vec![Line::from(Span::styled(
            "MODIFIED FILES",
            Style::new().bold().dim(),
        ))];

        if max_rows <= 1 {
            return lines;
        }

        if issue.workspace_path_suffix == "-" {
            lines.push(Line::from(Span::styled(
                "workspace unavailable",
                Style::new().dim(),
            )));
            return lines;
        }

        match self.workspace_status_entry(issue) {
            Some(WorkspaceStatusEntry::Loaded(status)) => match &status.changes {
                WorkspaceChangeState::Unavailable(message) => {
                    lines.push(Line::from(Span::styled(
                        fit(message, width),
                        Style::new().dim(),
                    )));
                }
                WorkspaceChangeState::Available(summary) => {
                    lines.push(change_summary_line_styled(summary));
                    if summary.files.is_empty() {
                        lines.push(Line::from(Span::styled(
                            "no modified files",
                            Style::new().dim(),
                        )));
                        return lines;
                    }

                    let visible_rows = max_rows.saturating_sub(lines.len());
                    if visible_rows == 0 {
                        return lines;
                    }

                    let (start, end) = issue_window(
                        summary.files.len(),
                        self.selected_changed_file,
                        visible_rows,
                    );
                    for (index, file) in summary.files[start..end].iter().enumerate() {
                        let global_index = start + index;
                        lines.push(self.changed_file_line_styled(
                            file,
                            global_index == self.selected_changed_file,
                            width,
                        ));
                    }
                }
            },
            _ => {
                lines.push(Line::from(Span::styled(
                    "loading git changes...",
                    Style::new().dim(),
                )));
            }
        }

        lines
    }

    fn changed_file_line_styled(
        &self,
        file: &WorkspaceFileChange,
        is_selected: bool,
        width: usize,
    ) -> Line {
        let marker = if is_selected {
            if self.detail_diff_open { "v" } else { ">" }
        } else {
            " "
        };
        let additions_text = change_count_text('+', file.additions);
        let deletions_text = change_count_text('-', file.deletions);
        let reserved_width = 4 + display_width(&additions_text) + display_width(&deletions_text);
        let path_width = max(1, width.saturating_sub(reserved_width));
        let path_text = fit(&file.display_path, path_width);
        let path_style = if is_selected {
            Style::new().bold()
        } else {
            Style::new()
        };

        Line::from_spans(vec![
            Span::styled(marker, Style::new().fg(BRIGHT_GREEN).bold()),
            Span::raw(" "),
            Span::styled(path_text, path_style),
            Span::raw(" "),
            Span::styled(additions_text, Style::new().fg(BRIGHT_GREEN).bold()),
            Span::raw(" "),
            Span::styled(deletions_text, Style::new().fg(RED).bold()),
        ])
    }

    fn selected_diff_lines_styled(&self, width: usize, max_rows: usize) -> Vec<Line> {
        let diff_focused = self.focus == FocusPane::Activity;
        let title_style = if diff_focused {
            Style::new().bold()
        } else {
            Style::new().dim()
        };
        let mut lines = vec![Line::from(Span::styled(
            pane_title("FILE DIFF", diff_focused),
            title_style,
        ))];

        match self.selected_diff_display() {
            SelectedDiffDisplay::Closed => {
                lines.push(Line::from(Span::styled(
                    "press enter on a changed file to show its diff",
                    Style::new().dim(),
                )));
            }
            SelectedDiffDisplay::Loading(title) => {
                lines.push(change_target_line_styled(&title, None, None, width));
                lines.push(Line::from(Span::styled(
                    "loading diff...",
                    Style::new().dim(),
                )));
            }
            SelectedDiffDisplay::Available {
                title,
                additions,
                deletions,
                lines: diff_lines,
            } => {
                lines.push(change_target_line_styled(
                    &title, additions, deletions, width,
                ));
                if diff_lines.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "no diff output",
                        Style::new().dim(),
                    )));
                } else {
                    let visible_rows = max_rows.saturating_sub(lines.len());
                    let start = min(
                        self.diff_scroll_offset,
                        diff_lines.len().saturating_sub(visible_rows),
                    );
                    for diff_line in diff_lines.into_iter().skip(start).take(visible_rows) {
                        lines.push(render_workspace_diff_line_styled(&diff_line, width));
                    }
                }
            }
            SelectedDiffDisplay::Unavailable(message) => {
                lines.push(Line::from(Span::styled(
                    fit(&message, width),
                    Style::new().dim(),
                )));
            }
        }

        lines
    }

    fn bottom_right_lines_styled(&self, width: usize, max_rows: usize) -> Vec<Line> {
        match self.selected_diff_display() {
            SelectedDiffDisplay::Closed => self.conversation_activity_lines(width, max_rows),
            _ => self.selected_diff_lines_styled(width, max_rows),
        }
    }

    fn timeline_lines_styled(&self, _width: usize, max_rows: usize) -> Vec<Line> {
        let title = match self.timeline_mode {
            TimelineMode::Events => "RECENT EVENTS",
            TimelineMode::Metrics => "METRICS",
        };
        let mut lines = vec![Line::from(Span::styled(title, Style::new().bold().dim()))];

        match &self.latest_snapshot {
            Some(snapshot) => match self.timeline_mode {
                TimelineMode::Events => {
                    let show_count = max_rows
                        .saturating_sub(1)
                        .min(snapshot.snapshot.recent_events.len());
                    for event in snapshot.snapshot.recent_events.iter().take(show_count) {
                        let kind_style = match event.kind {
                            crate::opensymphony_domain::ControlPlaneRecentEventKind::WorkerStarted => {
                                Style::new().fg(GREEN)
                            }
                            crate::opensymphony_domain::ControlPlaneRecentEventKind::WorkerCompleted => {
                                Style::new().fg(CYAN)
                            }
                            crate::opensymphony_domain::ControlPlaneRecentEventKind::Warning => {
                                Style::new().fg(RED)
                            }
                            crate::opensymphony_domain::ControlPlaneRecentEventKind::SnapshotPublished => {
                                Style::new().dim()
                            }
                            _ => Style::new().dim(),
                        };

                        // Parse and colorize the summary: colorize running=# (green) and retry_queue=# (orange/yellow)
                        let summary_spans = parse_summary_with_colors(&event.summary);
                        let mut line_spans = vec![
                            Span::styled(
                                format!("{} ", format_timestamp(event.happened_at)),
                                Style::new().dim(),
                            ),
                            Span::styled(event.kind.as_str(), kind_style),
                            Span::raw(" "),
                        ];
                        line_spans.extend(summary_spans);
                        lines.push(Line::from_spans(line_spans));
                    }
                }
                TimelineMode::Metrics => {
                    let m = &snapshot.snapshot.metrics;
                    lines.push(Line::from_spans(vec![
                        Span::styled("running: ", Style::new().dim()),
                        Span::styled(format!("{}", m.running_issues), Style::new().fg(GREEN)),
                    ]));
                    if lines.len() < max_rows {
                        lines.push(Line::from_spans(vec![
                            Span::styled("retry queue: ", Style::new().dim()),
                            Span::raw(format!("{}", m.retry_queue_depth)),
                        ]));
                    }
                    if lines.len() < max_rows {
                        lines.push(Line::from_spans(vec![
                            Span::styled("tokens: ", Style::new().dim()),
                            Span::raw(format!("{}", m.total_tokens)),
                        ]));
                    }
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

    fn metadata_and_files_lines(&self, width: usize, max_rows: usize) -> Vec<Line> {
        let detail_files_focused = self.focus == FocusPane::Detail;
        let title_style = if detail_files_focused {
            Style::new().bold()
        } else {
            Style::new().dim()
        };
        let mut lines = vec![Line::from(Span::styled(
            pane_title("ISSUE + WORKSPACE DETAIL", detail_files_focused),
            title_style,
        ))];

        match self.selected_issue() {
            Some(issue) => {
                // Issue identifier and title
                let id_style = Style::new().fg(CYAN).bold();
                lines.push(Line::from_spans(vec![
                    Span::styled(&issue.identifier, id_style),
                    Span::raw(" "),
                    Span::styled(&issue.title, Style::new().bold()),
                ]));

                // Tracker / Runtime / Outcome
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

                let branch = self.branch_text(issue);
                lines.push(Line::from_spans(vec![
                    Span::styled("branch: ", Style::new().dim()),
                    Span::styled(branch, Style::new().fg(CYAN)),
                ]));

                lines.push(self.pr_line_styled(issue));

                // Last event, retries, blocked
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

                // Token usage for this issue (always show, even when 0)
                let cache_suffix = if issue.cache_read_tokens > 0 {
                    format!(" ({} cache)", format_metric(issue.cache_read_tokens))
                } else {
                    String::new()
                };
                lines.push(Line::from_spans(vec![
                    Span::styled("tokens: ", Style::new().dim()),
                    Span::styled(
                        format!(
                            "{} input{}",
                            format_metric(issue.input_tokens),
                            cache_suffix
                        ),
                        Style::new().fg(GREEN),
                    ),
                    Span::raw(", "),
                    Span::styled(
                        format!("{} output", format_metric(issue.output_tokens)),
                        Style::new().fg(CYAN),
                    ),
                    Span::raw(", "),
                    Span::styled(
                        format!(
                            "{} total",
                            format_metric(issue.input_tokens + issue.output_tokens)
                        ),
                        Style::new().dim(),
                    ),
                ]));

                // Separator
                if lines.len() < max_rows {
                    lines.push(Line::from(Span::styled(
                        "-".repeat(min(width, 40)),
                        Style::new().dim(),
                    )));
                }

                if lines.len() < max_rows {
                    let remaining_file_rows = max_rows.saturating_sub(lines.len());
                    lines.extend(self.modified_files_lines_styled(
                        width,
                        issue,
                        remaining_file_rows,
                    ));
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

    fn conversation_activity_lines(&self, width: usize, max_rows: usize) -> Vec<Line> {
        let activity_focused = self.focus == FocusPane::Activity && !self.detail_diff_open;
        let title_style = if activity_focused {
            Style::new().bold()
        } else {
            Style::new().dim()
        };
        let mut lines = vec![Line::from(Span::styled(
            pane_title("CONVERSATION ACTIVITY", activity_focused),
            title_style,
        ))];

        match self.selected_issue() {
            Some(issue) => {
                let body_lines = self.conversation_activity_body_lines_styled(issue, width);
                if body_lines.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "no recent activity",
                        Style::new().dim(),
                    )));
                } else {
                    let visible_rows = max_rows.saturating_sub(1);
                    let start = conversation_scroll_start(
                        body_lines.len(),
                        visible_rows,
                        self.conversation_scroll_offset,
                    );
                    for line in body_lines.into_iter().skip(start).take(visible_rows) {
                        lines.push(line);
                    }
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
                lines.push(fit(&format!("branch: {}", self.branch_text(issue)), width));
                lines.push(fit(&format!("pr: {}", self.pr_text(issue)), width));
                lines.push(fit(
                    &format!(
                        "last event: {} | retries: {} | blocked: {}",
                        format_timestamp(issue.last_event_at),
                        issue.retry_count,
                        issue.blocked
                    ),
                    width,
                ));

                if lines.len() < max_rows {
                    lines.push("-".repeat(width.min(40)));
                    let remaining_rows = max_rows.saturating_sub(lines.len());
                    if matches!(self.selected_diff_display(), SelectedDiffDisplay::Closed) {
                        let file_rows = remaining_rows.min(max(4, remaining_rows / 2));
                        lines.extend(self.modified_files_lines(width, issue, file_rows));

                        let conversation_rows = max_rows.saturating_sub(lines.len() + 1);
                        if conversation_rows > 0 {
                            lines.push("-".repeat(width.min(40)));
                            lines.extend(self.conversation_events_lines(
                                width,
                                issue,
                                conversation_rows,
                            ));
                        }
                    } else {
                        let file_rows = remaining_rows.min(max(4, remaining_rows / 3));
                        lines.extend(self.modified_files_lines(width, issue, file_rows));

                        let diff_rows = max_rows.saturating_sub(lines.len() + 1);
                        if diff_rows > 0 {
                            lines.push("-".repeat(width.min(40)));
                            lines.extend(self.selected_diff_lines(width, diff_rows));
                        }
                    }
                }
            }
            None => {
                lines.push(fit("no selected issue", width));
            }
        }
        lines
    }

    fn modified_files_lines(
        &self,
        width: usize,
        issue: &IssueSnapshot,
        max_rows: usize,
    ) -> Vec<String> {
        let mut lines = vec![fit("MODIFIED FILES", width)];

        if max_rows <= 1 {
            return lines;
        }

        if issue.workspace_path_suffix == "-" {
            lines.push(fit("workspace unavailable", width));
            return lines;
        }

        match self.workspace_status_entry(issue) {
            Some(WorkspaceStatusEntry::Loaded(status)) => match &status.changes {
                WorkspaceChangeState::Unavailable(message) => {
                    lines.push(fit(message, width));
                }
                WorkspaceChangeState::Available(summary) => {
                    lines.push(fit(&change_summary_line_text(summary), width));
                    if summary.files.is_empty() {
                        lines.push(fit("no modified files", width));
                        return lines;
                    }

                    let visible_rows = max_rows.saturating_sub(lines.len());
                    if visible_rows == 0 {
                        return lines;
                    }

                    let (start, end) = issue_window(
                        summary.files.len(),
                        self.selected_changed_file,
                        visible_rows,
                    );
                    for (index, file) in summary.files[start..end].iter().enumerate() {
                        let global_index = start + index;
                        lines.push(change_target_line_text(
                            &file.display_path,
                            file.additions,
                            file.deletions,
                            width,
                            global_index == self.selected_changed_file,
                            self.detail_diff_open,
                        ));
                    }
                }
            },
            _ => lines.push(fit("loading git changes...", width)),
        }

        lines
    }

    fn selected_diff_lines(&self, width: usize, max_rows: usize) -> Vec<String> {
        let diff_focused = self.focus == FocusPane::Activity;
        let mut lines = vec![fit(&pane_title("FILE DIFF", diff_focused), width)];

        match self.selected_diff_display() {
            SelectedDiffDisplay::Closed => {
                lines.push(fit("press enter on a changed file to show its diff", width));
            }
            SelectedDiffDisplay::Loading(title) => {
                lines.push(change_target_line_text(
                    &title, None, None, width, false, false,
                ));
                lines.push(fit("loading diff...", width));
            }
            SelectedDiffDisplay::Available {
                title,
                additions,
                deletions,
                lines: diff_lines,
            } => {
                lines.push(change_target_line_text(
                    &title, additions, deletions, width, false, false,
                ));
                if diff_lines.is_empty() {
                    lines.push(fit("no diff output", width));
                } else {
                    let visible_rows = max_rows.saturating_sub(lines.len());
                    let start = min(
                        self.diff_scroll_offset,
                        diff_lines.len().saturating_sub(visible_rows),
                    );
                    for diff_line in diff_lines.into_iter().skip(start).take(visible_rows) {
                        lines.push(fit(&diff_line.text, width));
                    }
                }
            }
            SelectedDiffDisplay::Unavailable(message) => {
                lines.push(fit(&message, width));
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
        let activity_focused = self.focus == FocusPane::Activity && !self.detail_diff_open;
        let mut lines = vec![fit(
            &pane_title("CONVERSATION ACTIVITY", activity_focused),
            width,
        )];
        let body_lines = self.conversation_activity_body_lines(issue, width);
        if body_lines.is_empty() {
            lines.push(fit("no recent activity", width));
        } else {
            let visible_rows = max_rows.saturating_sub(1);
            let start = conversation_scroll_start(
                body_lines.len(),
                visible_rows,
                self.conversation_scroll_offset,
            );
            for line in body_lines.into_iter().skip(start).take(visible_rows) {
                lines.push(fit(&line, width));
            }
        }
        lines
    }

    fn conversation_activity_body_lines_styled(
        &self,
        issue: &IssueSnapshot,
        width: usize,
    ) -> Vec<Line> {
        issue
            .recent_events
            .iter()
            .rev()
            .flat_map(|event| wrap_conversation_event_styled(event, width))
            .collect()
    }

    fn conversation_activity_body_lines(&self, issue: &IssueSnapshot, width: usize) -> Vec<String> {
        issue
            .recent_events
            .iter()
            .rev()
            .flat_map(|event| wrap_conversation_event_text(event, width))
            .collect()
    }

    fn timeline_lines(&self, width: usize) -> Vec<String> {
        let title = match self.timeline_mode {
            TimelineMode::Events => "RECENT EVENTS",
            TimelineMode::Metrics => "METRICS",
        };
        let mut lines = vec![fit(title, width)];
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

    fn workspace_status_entry(&self, issue: &IssueSnapshot) -> Option<&WorkspaceStatusEntry> {
        self.workspace_status.get(&issue.identifier)
    }

    fn branch_text(&self, issue: &IssueSnapshot) -> String {
        if issue.workspace_path_suffix == "-" {
            return "unavailable".to_owned();
        }

        match self.workspace_status_entry(issue) {
            Some(WorkspaceStatusEntry::Loaded(status)) => status.branch.clone(),
            _ => "loading...".to_owned(),
        }
    }

    fn pr_display(&self, issue: &IssueSnapshot) -> WorkspacePrDisplay {
        if issue.workspace_path_suffix == "-" {
            return WorkspacePrDisplay::Unavailable;
        }

        match self.workspace_status_entry(issue) {
            Some(WorkspaceStatusEntry::Loaded(status)) => match status.pr_url.clone() {
                Some(pr_url) => WorkspacePrDisplay::Available(pr_url),
                None => WorkspacePrDisplay::None,
            },
            _ => WorkspacePrDisplay::Loading,
        }
    }

    fn pr_text(&self, issue: &IssueSnapshot) -> String {
        match self.pr_display(issue) {
            WorkspacePrDisplay::Loading => "loading...".to_owned(),
            WorkspacePrDisplay::Available(pr_url) => pr_url,
            WorkspacePrDisplay::None => "none".to_owned(),
            WorkspacePrDisplay::Unavailable => "unavailable".to_owned(),
        }
    }

    fn pr_line_styled(&self, issue: &IssueSnapshot) -> Line {
        match self.pr_display(issue) {
            WorkspacePrDisplay::Loading => Line::from_spans(vec![
                Span::styled("pr: ", Style::new().dim()),
                Span::styled("loading...", Style::new().dim()),
            ]),
            WorkspacePrDisplay::Available(pr_url) => Line::from_spans(vec![
                Span::styled("pr: ", Style::new().dim()),
                Span::styled(pr_url.clone(), Style::new().fg(BLUE).underline()).link(pr_url),
            ]),
            WorkspacePrDisplay::None => Line::from_spans(vec![
                Span::styled("pr: ", Style::new().dim()),
                Span::styled("none", Style::new().dim()),
            ]),
            WorkspacePrDisplay::Unavailable => Line::from_spans(vec![
                Span::styled("pr: ", Style::new().dim()),
                Span::styled("unavailable", Style::new().dim()),
            ]),
        }
    }

    fn issue_count(&self) -> usize {
        self.latest_snapshot
            .as_ref()
            .map(|snapshot| snapshot.snapshot.issues.len())
            .unwrap_or_default()
    }

    fn move_issue_selection_up(&mut self) {
        self.selected_issue = self.selected_issue.saturating_sub(1);
        self.sync_detail_state();
    }

    fn move_issue_selection_down(&mut self) {
        let count = self.issue_count();
        if count > 0 {
            self.selected_issue = min(self.selected_issue + 1, count - 1);
        }
        self.sync_detail_state();
    }

    fn move_changed_file_selection_up(&mut self) {
        self.selected_changed_file = self.selected_changed_file.saturating_sub(1);
        self.sync_detail_state();
    }

    fn move_changed_file_selection_down(&mut self) {
        let file_count = self.selected_changed_file_count();
        if file_count > 0 {
            self.selected_changed_file = min(self.selected_changed_file + 1, file_count - 1);
        }
        self.sync_detail_state();
    }

    fn move_conversation_scroll_up(&mut self) {
        self.conversation_scroll_offset = self.conversation_scroll_offset.saturating_add(1);
    }

    fn move_conversation_scroll_down(&mut self) {
        self.conversation_scroll_offset = self.conversation_scroll_offset.saturating_sub(1);
    }

    fn move_diff_scroll_up(&mut self) {
        self.diff_scroll_offset = self.diff_scroll_offset.saturating_sub(1);
    }

    fn move_diff_scroll_down(&mut self) {
        let diff_line_count = self.selected_diff_line_count();
        if diff_line_count > 0 {
            self.diff_scroll_offset = min(self.diff_scroll_offset + 1, diff_line_count - 1);
        }
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

    fn retain_workspace_status_for_visible_issues(&mut self) {
        let visible_issues = self
            .latest_snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .snapshot
                    .issues
                    .iter()
                    .map(|issue| issue.identifier.clone())
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();

        self.workspace_status
            .retain(|issue_identifier, _| visible_issues.contains(issue_identifier));
    }

    fn workspace_change_summary(&self, issue: &IssueSnapshot) -> Option<&WorkspaceChangeSummary> {
        match self.workspace_status_entry(issue) {
            Some(WorkspaceStatusEntry::Loaded(status)) => match &status.changes {
                WorkspaceChangeState::Available(summary) => Some(summary),
                WorkspaceChangeState::Unavailable(_) => None,
            },
            _ => None,
        }
    }

    fn selected_changed_file_count(&self) -> usize {
        self.selected_issue()
            .and_then(|issue| self.workspace_change_summary(issue))
            .map(|summary| summary.files.len())
            .unwrap_or_default()
    }

    fn selected_file_change(&self) -> Option<&WorkspaceFileChange> {
        let issue = self.selected_issue()?;
        self.workspace_change_summary(issue)?
            .files
            .get(self.selected_changed_file)
    }

    fn selected_diff_display(&self) -> SelectedDiffDisplay {
        if !self.detail_diff_open {
            return SelectedDiffDisplay::Closed;
        }

        let Some(change) = self.selected_file_change() else {
            return SelectedDiffDisplay::Unavailable("no changed file selected".to_owned());
        };

        match &change.diff {
            WorkspaceFileDiffState::Unloaded | WorkspaceFileDiffState::Loading => {
                SelectedDiffDisplay::Loading(change.display_path.clone())
            }
            WorkspaceFileDiffState::Loaded(lines) => SelectedDiffDisplay::Available {
                title: change.display_path.clone(),
                additions: change.additions,
                deletions: change.deletions,
                lines: lines.clone(),
            },
            WorkspaceFileDiffState::Unavailable(message) => {
                SelectedDiffDisplay::Unavailable(message.clone())
            }
        }
    }

    fn selected_diff_line_count(&self) -> usize {
        match self.selected_file_change().map(|change| &change.diff) {
            Some(WorkspaceFileDiffState::Loaded(lines)) => lines.len(),
            _ => 0,
        }
    }

    fn sync_detail_state(&mut self) {
        let selected_issue_identifier = self.selected_issue().map(|issue| issue.identifier.clone());
        if self.detail_issue_identifier != selected_issue_identifier {
            self.selected_changed_file = 0;
            self.detail_diff_open = false;
            self.conversation_scroll_offset = 0;
            self.diff_scroll_offset = 0;
        }
        self.detail_issue_identifier = selected_issue_identifier;

        let file_count = self.selected_changed_file_count();
        if file_count == 0 {
            self.selected_changed_file = 0;
            self.detail_diff_open = false;
            self.diff_scroll_offset = 0;
        } else {
            self.selected_changed_file = min(self.selected_changed_file, file_count - 1);
        }

        if self
            .selected_issue()
            .is_none_or(|issue| issue.recent_events.is_empty())
        {
            self.conversation_scroll_offset = 0;
        }

        if !self.detail_diff_open {
            self.diff_scroll_offset = 0;
        } else {
            self.diff_scroll_offset = min(
                self.diff_scroll_offset,
                self.selected_diff_line_count().saturating_sub(1),
            );
        }
    }

    fn merge_workspace_changes(
        &self,
        issue_identifier: &str,
        changes: WorkspaceChangeState,
    ) -> WorkspaceChangeState {
        let WorkspaceChangeState::Available(mut summary) = changes else {
            return changes;
        };

        if let Some(WorkspaceStatusEntry::Loaded(existing_status)) =
            self.workspace_status.get(issue_identifier)
            && let WorkspaceChangeState::Available(existing_summary) = &existing_status.changes
        {
            for file in &mut summary.files {
                if let Some(existing_file) = existing_summary.files.iter().find(|existing_file| {
                    existing_file.query_path == file.query_path
                        && existing_file.previous_path == file.previous_path
                        && existing_file.status_code == file.status_code
                        && existing_file.additions == file.additions
                        && existing_file.deletions == file.deletions
                }) {
                    file.diff = existing_file.diff.clone();
                }
            }
        }

        WorkspaceChangeState::Available(summary)
    }

    fn set_file_diff_state(
        &mut self,
        issue_identifier: &str,
        query_path: &str,
        diff: WorkspaceFileDiffState,
    ) {
        let Some(WorkspaceStatusEntry::Loaded(status)) =
            self.workspace_status.get_mut(issue_identifier)
        else {
            return;
        };

        let WorkspaceChangeState::Available(summary) = &mut status.changes else {
            return;
        };

        if let Some(file) = summary
            .files
            .iter_mut()
            .find(|file| file.query_path == query_path)
        {
            file.diff = diff;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    Issues,
    Detail,
    Activity,
}

impl FocusPane {
    fn label(&self) -> &'static str {
        match self {
            FocusPane::Issues => "issues",
            FocusPane::Detail => "detail",
            FocusPane::Activity => "activity",
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
    FocusPrevious,
    ToggleDetailDiff,
    ToggleTimelineMode,
    WorkspaceStatusRequested(String),
    WorkspaceStatusLoaded {
        issue_identifier: String,
        branch: String,
        pr_url: Option<String>,
        changes: WorkspaceChangeState,
    },
    WorkspaceDiffRequested {
        issue_identifier: String,
        query_path: String,
    },
    WorkspaceDiffLoaded {
        issue_identifier: String,
        query_path: String,
        diff: Result<Vec<WorkspaceDiffLine>, String>,
    },
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

#[derive(Debug)]
enum WorkspaceStatusRequest {
    Refresh {
        issue_identifier: String,
        workspace_path: PathBuf,
    },
    LoadDiff {
        issue_identifier: String,
        workspace_path: PathBuf,
        query_path: String,
        previous_path: Option<String>,
        status_code: String,
    },
    Shutdown,
}

#[derive(Debug, Default)]
struct WorkspaceStatusMailbox {
    pending_actions: VecDeque<TuiAction>,
}

impl WorkspaceStatusMailbox {
    fn push_loaded(
        &mut self,
        issue_identifier: String,
        branch: String,
        pr_url: Option<String>,
        changes: WorkspaceChangeState,
    ) {
        self.pending_actions
            .push_back(TuiAction::WorkspaceStatusLoaded {
                issue_identifier,
                branch,
                pr_url,
                changes,
            });
    }

    fn push_diff_loaded(
        &mut self,
        issue_identifier: String,
        query_path: String,
        diff: Result<Vec<WorkspaceDiffLine>, String>,
    ) {
        self.pending_actions
            .push_back(TuiAction::WorkspaceDiffLoaded {
                issue_identifier,
                query_path,
                diff,
            });
    }

    fn take_action(&mut self) -> Option<TuiAction> {
        self.pending_actions.pop_front()
    }
}

#[derive(Debug)]
struct WorkspaceStatusHandle {
    mailbox: Arc<Mutex<WorkspaceStatusMailbox>>,
    request_tx: mpsc::Sender<WorkspaceStatusRequest>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl WorkspaceStatusHandle {
    fn spawn() -> Self {
        let mailbox = Arc::new(Mutex::new(WorkspaceStatusMailbox::default()));
        let (request_tx, request_rx) = mpsc::channel();
        let join_handle = thread::spawn({
            let mailbox = Arc::clone(&mailbox);
            move || run_workspace_status_thread(mailbox, request_rx)
        });
        Self {
            mailbox,
            request_tx,
            join_handle: Some(join_handle),
        }
    }

    fn mailbox(&self) -> Arc<Mutex<WorkspaceStatusMailbox>> {
        Arc::clone(&self.mailbox)
    }

    fn request_sender(&self) -> mpsc::Sender<WorkspaceStatusRequest> {
        self.request_tx.clone()
    }

    fn shutdown(mut self) -> Result<(), TuiError> {
        let _ = self.request_tx.send(WorkspaceStatusRequest::Shutdown);
        if let Some(join_handle) = self.join_handle.take() {
            join_handle
                .join()
                .map_err(|_| TuiError::WorkspaceStatusThreadPanicked)?;
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
    FocusPrevious,
    ToggleDetailDiff,
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
                KeyCode::BackTab => AppMessage::FocusPrevious,
                KeyCode::Enter => AppMessage::ToggleDetailDiff,
                KeyCode::Char('e') => AppMessage::ToggleTimelineMode,
                _ => AppMessage::Tick,
            },
            _ => AppMessage::Tick,
        }
    }
}

pub fn run_operator(base_url: Url, exit_after: Option<Duration>) -> Result<(), TuiError> {
    let bridge = BridgeHandle::spawn(base_url);
    let workspace_status = WorkspaceStatusHandle::spawn();
    let outcome = Arc::new(Mutex::new(RunOutcome::default()));
    let app = OperatorApp::new(
        bridge.mailbox(),
        workspace_status.mailbox(),
        workspace_status.request_sender(),
        exit_after,
        Arc::clone(&outcome),
    );
    let config = tui_program_config();
    let run_result = ftui::Program::with_config(app, config)
        .and_then(|mut program| program.run())
        .map_err(TuiError::Runtime);
    let workspace_shutdown_result = workspace_status.shutdown();
    let shutdown_result = bridge.shutdown();
    let timeout_before_live = outcome
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .timeout_before_live
        .clone();

    match (run_result, workspace_shutdown_result, shutdown_result) {
        (Err(error), _, _) => Err(error),
        (Ok(()), Err(error), _) => Err(error),
        (Ok(()), Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(()), Ok(())) => match timeout_before_live {
            Some(status_line) => Err(TuiError::AttachTimeout(status_line)),
            None => Ok(()),
        },
    }
}

#[derive(Debug, Default)]
struct RunOutcome {
    timeout_before_live: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceStatusRequestKey {
    issue_identifier: String,
    sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceDiffRequestKey {
    issue_identifier: String,
    sequence: u64,
    query_path: String,
}

#[derive(Debug)]
struct OperatorApp {
    state: TuiState,
    bridge: Arc<Mutex<BridgeMailbox>>,
    workspace_status: Arc<Mutex<WorkspaceStatusMailbox>>,
    workspace_status_requests: mpsc::Sender<WorkspaceStatusRequest>,
    last_workspace_status_request: Option<WorkspaceStatusRequestKey>,
    last_workspace_diff_request: Option<WorkspaceDiffRequestKey>,
    exit_after: Option<Duration>,
    started_at: Instant,
    saw_live_stream: bool,
    outcome: Arc<Mutex<RunOutcome>>,
}

impl OperatorApp {
    fn new(
        bridge: Arc<Mutex<BridgeMailbox>>,
        workspace_status: Arc<Mutex<WorkspaceStatusMailbox>>,
        workspace_status_requests: mpsc::Sender<WorkspaceStatusRequest>,
        exit_after: Option<Duration>,
        outcome: Arc<Mutex<RunOutcome>>,
    ) -> Self {
        Self {
            state: TuiState::default(),
            bridge,
            workspace_status,
            workspace_status_requests,
            last_workspace_status_request: None,
            last_workspace_diff_request: None,
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

    fn drain_workspace_status(&mut self) {
        let mut workspace_status = self
            .workspace_status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while let Some(action) = workspace_status.take_action() {
            self.state.reduce(action);
        }
    }

    fn request_selected_workspace_status(&mut self) {
        let Some(snapshot) = self.state.latest_snapshot.as_ref() else {
            self.last_workspace_status_request = None;
            return;
        };
        let Some(issue) = self.state.selected_issue() else {
            self.last_workspace_status_request = None;
            return;
        };
        let issue_identifier = issue.identifier.clone();
        let workspace_suffix = issue.workspace_path_suffix.clone();
        let snapshot_sequence = snapshot.sequence;
        if workspace_suffix == "-" {
            self.last_workspace_status_request = None;
            return;
        }

        let Some(workspace_path) = workspace_path_for_issue(snapshot, workspace_suffix.as_str())
        else {
            self.last_workspace_status_request = None;
            return;
        };

        let request_key = WorkspaceStatusRequestKey {
            issue_identifier: issue_identifier.clone(),
            sequence: snapshot_sequence,
        };
        if self.last_workspace_status_request.as_ref() == Some(&request_key) {
            return;
        }

        self.last_workspace_status_request = Some(request_key);
        self.state.reduce(TuiAction::WorkspaceStatusRequested(
            issue_identifier.clone(),
        ));
        let _ = self
            .workspace_status_requests
            .send(WorkspaceStatusRequest::Refresh {
                issue_identifier,
                workspace_path,
            });
    }

    fn request_selected_workspace_diff(&mut self) {
        let Some(snapshot) = self.state.latest_snapshot.as_ref() else {
            self.last_workspace_diff_request = None;
            return;
        };
        let Some(issue) = self.state.selected_issue() else {
            self.last_workspace_diff_request = None;
            return;
        };
        if !self.state.detail_diff_open {
            self.last_workspace_diff_request = None;
            return;
        }

        let Some(file_change) = self.state.selected_file_change() else {
            self.last_workspace_diff_request = None;
            return;
        };

        if !matches!(file_change.diff, WorkspaceFileDiffState::Unloaded) {
            self.last_workspace_diff_request = None;
            return;
        }
        let query_path = file_change.query_path.clone();
        let previous_path = file_change.previous_path.clone();
        let status_code = file_change.status_code.clone();

        let issue_identifier = issue.identifier.clone();
        let workspace_suffix = issue.workspace_path_suffix.clone();
        if workspace_suffix == "-" {
            self.last_workspace_diff_request = None;
            return;
        }

        let Some(workspace_path) = workspace_path_for_issue(snapshot, workspace_suffix.as_str())
        else {
            self.last_workspace_diff_request = None;
            return;
        };

        let request_key = WorkspaceDiffRequestKey {
            issue_identifier: issue_identifier.clone(),
            sequence: snapshot.sequence,
            query_path: query_path.clone(),
        };
        if self.last_workspace_diff_request.as_ref() == Some(&request_key) {
            return;
        }

        self.last_workspace_diff_request = Some(request_key);
        self.state.reduce(TuiAction::WorkspaceDiffRequested {
            issue_identifier: issue_identifier.clone(),
            query_path: query_path.clone(),
        });
        let _ = self
            .workspace_status_requests
            .send(WorkspaceStatusRequest::LoadDiff {
                issue_identifier,
                workspace_path,
                query_path,
                previous_path,
                status_code,
            });
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
    stream: &mut crate::opensymphony_control::ControlPlaneEventStream,
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

fn run_workspace_status_thread(
    mailbox: Arc<Mutex<WorkspaceStatusMailbox>>,
    request_rx: mpsc::Receiver<WorkspaceStatusRequest>,
) {
    while let Ok(request) = request_rx.recv() {
        match request {
            WorkspaceStatusRequest::Refresh {
                issue_identifier,
                workspace_path,
            } => {
                let status = inspect_workspace(&workspace_path);
                let mut mailbox = mailbox
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                mailbox.push_loaded(
                    issue_identifier,
                    status.branch,
                    status.pr_url,
                    status.changes,
                );
            }
            WorkspaceStatusRequest::LoadDiff {
                issue_identifier,
                workspace_path,
                query_path,
                previous_path,
                status_code,
            } => {
                let diff = load_workspace_diff(
                    &workspace_path,
                    &query_path,
                    previous_path.as_deref(),
                    &status_code,
                );
                let mut mailbox = mailbox
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                mailbox.push_diff_loaded(issue_identifier, query_path, diff);
            }
            WorkspaceStatusRequest::Shutdown => return,
        }
    }
}

fn inspect_workspace(workspace_path: &Path) -> WorkspaceStatusData {
    WorkspaceStatusData {
        branch: git_branch(workspace_path),
        pr_url: gh_pr_url(workspace_path),
        changes: inspect_workspace_changes(workspace_path),
    }
}

fn inspect_workspace_changes(workspace_path: &Path) -> WorkspaceChangeState {
    match build_workspace_change_summary(workspace_path) {
        Ok(summary) => WorkspaceChangeState::Available(summary),
        Err(error) => WorkspaceChangeState::Unavailable(format!(
            "git comparison unavailable: {}",
            single_line(&error)
        )),
    }
}

fn build_workspace_change_summary(workspace_path: &Path) -> Result<WorkspaceChangeSummary, String> {
    let comparison_base = workspace_comparison_base(workspace_path)?;
    let mut files = tracked_workspace_file_changes(workspace_path, &comparison_base)?;
    files.extend(untracked_workspace_file_changes(workspace_path)?);
    files.sort_by(|left, right| left.display_path.cmp(&right.display_path));

    let additions = files.iter().filter_map(|file| file.additions).sum();
    let deletions = files.iter().filter_map(|file| file.deletions).sum();

    Ok(WorkspaceChangeSummary {
        files_changed: files.len(),
        additions,
        deletions,
        files,
    })
}

fn tracked_workspace_file_changes(
    workspace_path: &Path,
    comparison_base: &WorkspaceComparisonBase,
) -> Result<Vec<WorkspaceFileChange>, String> {
    let output = command_output_args(
        workspace_path,
        "git",
        [
            "diff".to_owned(),
            "--name-status".to_owned(),
            "-z".to_owned(),
            "--find-renames".to_owned(),
            comparison_base.merge_base.clone(),
            "--".to_owned(),
        ],
    )?;
    let mut fields = output
        .split('\0')
        .filter(|field| !field.is_empty())
        .peekable();
    let mut files = Vec::new();

    while let Some(status_code) = fields.next() {
        if status_code.starts_with('R') || status_code.starts_with('C') {
            let previous_path = fields
                .next()
                .ok_or_else(|| "missing previous path for rename entry".to_owned())?;
            let query_path = fields
                .next()
                .ok_or_else(|| "missing current path for rename entry".to_owned())?;
            let (additions, deletions) = git_numstat_for_change(
                workspace_path,
                comparison_base,
                query_path,
                Some(previous_path),
            )?;
            files.push(WorkspaceFileChange {
                display_path: format!("{previous_path} -> {query_path}"),
                query_path: query_path.to_owned(),
                previous_path: Some(previous_path.to_owned()),
                status_code: status_code.to_owned(),
                additions,
                deletions,
                diff: WorkspaceFileDiffState::Unloaded,
            });
        } else {
            let query_path = fields
                .next()
                .ok_or_else(|| "missing path for git diff entry".to_owned())?;
            let (additions, deletions) =
                git_numstat_for_change(workspace_path, comparison_base, query_path, None)?;
            files.push(WorkspaceFileChange {
                display_path: query_path.to_owned(),
                query_path: query_path.to_owned(),
                previous_path: None,
                status_code: status_code.to_owned(),
                additions,
                deletions,
                diff: WorkspaceFileDiffState::Unloaded,
            });
        }
    }

    Ok(files)
}

fn untracked_workspace_file_changes(
    workspace_path: &Path,
) -> Result<Vec<WorkspaceFileChange>, String> {
    let output = command_output(
        workspace_path,
        "git",
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?;

    let mut files = Vec::new();
    for query_path in output.split('\0').filter(|field| !field.is_empty()) {
        files.push(WorkspaceFileChange {
            display_path: query_path.to_owned(),
            query_path: query_path.to_owned(),
            previous_path: None,
            status_code: "??".to_owned(),
            additions: count_untracked_lines(workspace_path, query_path),
            deletions: Some(0),
            diff: WorkspaceFileDiffState::Unloaded,
        });
    }

    Ok(files)
}

fn git_numstat_for_change(
    workspace_path: &Path,
    comparison_base: &WorkspaceComparisonBase,
    query_path: &str,
    previous_path: Option<&str>,
) -> Result<(Option<u64>, Option<u64>), String> {
    let mut args = vec![
        "diff".to_owned(),
        "--numstat".to_owned(),
        "--find-renames".to_owned(),
        comparison_base.merge_base.clone(),
        "--".to_owned(),
    ];
    if let Some(previous_path) = previous_path {
        args.push(previous_path.to_owned());
    }
    args.push(query_path.to_owned());

    let output = command_output_args(workspace_path, "git", args)?;
    let Some(line) = output.lines().find(|line| !line.trim().is_empty()) else {
        return Ok((Some(0), Some(0)));
    };
    let mut fields = line.split('\t');
    Ok((
        parse_numstat_count(fields.next()),
        parse_numstat_count(fields.next()),
    ))
}

fn parse_numstat_count(field: Option<&str>) -> Option<u64> {
    match field.map(str::trim) {
        Some("-") | None => None,
        Some(value) => value.parse().ok(),
    }
}

fn count_untracked_lines(workspace_path: &Path, query_path: &str) -> Option<u64> {
    let bytes = fs::read(workspace_path.join(query_path)).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    let text = String::from_utf8_lossy(&bytes);
    Some(text.lines().count() as u64)
}

fn load_workspace_diff(
    workspace_path: &Path,
    query_path: &str,
    previous_path: Option<&str>,
    status_code: &str,
) -> Result<Vec<WorkspaceDiffLine>, String> {
    let comparison_base = workspace_comparison_base(workspace_path)?;
    let output = if status_code.starts_with("??") {
        command_output_args_allow_status(
            workspace_path,
            "git",
            [
                "diff".to_owned(),
                "--no-index".to_owned(),
                "--".to_owned(),
                "/dev/null".to_owned(),
                query_path.to_owned(),
            ],
            &[1],
        )?
    } else {
        let mut args = vec![
            "diff".to_owned(),
            "--find-renames".to_owned(),
            comparison_base.merge_base,
            "--".to_owned(),
        ];
        if let Some(previous_path) = previous_path {
            args.push(previous_path.to_owned());
        }
        args.push(query_path.to_owned());
        command_output_args(workspace_path, "git", args)?
    };

    Ok(parse_workspace_diff(&output))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceComparisonBase {
    merge_base: String,
}

fn workspace_comparison_base(workspace_path: &Path) -> Result<WorkspaceComparisonBase, String> {
    for reference in ["main", "origin/main"] {
        if git_ref_exists(workspace_path, reference)? {
            return Ok(WorkspaceComparisonBase {
                merge_base: command_single_line(
                    workspace_path,
                    "git",
                    &["merge-base", "HEAD", reference],
                )?,
            });
        }
    }

    Err("main branch unavailable".to_owned())
}

fn git_ref_exists(workspace_path: &Path, reference: &str) -> Result<bool, String> {
    let output = command_output_args_allow_status(
        workspace_path,
        "git",
        ["rev-parse", "--verify", "--quiet", reference],
        &[1],
    )?;
    Ok(!output.trim().is_empty())
}

fn parse_workspace_diff(output: &str) -> Vec<WorkspaceDiffLine> {
    output
        .lines()
        .map(|line| WorkspaceDiffLine {
            kind: if line.starts_with("diff --git")
                || line.starts_with("index ")
                || line.starts_with("--- ")
                || line.starts_with("+++ ")
            {
                WorkspaceDiffLineKind::Header
            } else if line.starts_with("@@") {
                WorkspaceDiffLineKind::Hunk
            } else if line.starts_with('+') && !line.starts_with("+++") {
                WorkspaceDiffLineKind::Addition
            } else if line.starts_with('-') && !line.starts_with("---") {
                WorkspaceDiffLineKind::Deletion
            } else if line.starts_with("Binary files ") || line.starts_with("Only in ") {
                WorkspaceDiffLineKind::Note
            } else {
                WorkspaceDiffLineKind::Context
            },
            text: single_line(line),
        })
        .collect()
}

fn git_branch(workspace_path: &Path) -> String {
    match command_single_line(workspace_path, "git", &["branch", "--show-current"]) {
        Ok(branch) if !branch.is_empty() => branch,
        _ => match command_single_line(workspace_path, "git", &["rev-parse", "--short", "HEAD"]) {
            Ok(commit) if !commit.is_empty() => format!("detached @ {commit}"),
            _ => "unavailable".to_owned(),
        },
    }
}

fn gh_pr_url(workspace_path: &Path) -> Option<String> {
    command_single_line(
        workspace_path,
        "gh",
        &["pr", "view", "--json", "url", "--jq", ".url"],
    )
    .ok()
    .filter(|url| !url.is_empty())
}

fn command_single_line(
    workspace_path: &Path,
    program: &str,
    args: &[&str],
) -> Result<String, String> {
    command_output_args(workspace_path, program, args.iter().copied())
        .map(|output| single_line(output.trim()))
}

fn command_output(workspace_path: &Path, program: &str, args: &[&str]) -> Result<String, String> {
    command_output_args(workspace_path, program, args.iter().copied())
}

fn command_output_args<I, S>(
    workspace_path: &Path,
    program: &str,
    args: I,
) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    command_output_args_allow_status(workspace_path, program, args, &[])
}

fn command_output_args_allow_status<I, S>(
    workspace_path: &Path,
    program: &str,
    args: I,
    allowed_status_codes: &[i32],
) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(program)
        .args(args)
        .current_dir(workspace_path)
        .output()
        .map_err(|error| error.to_string())?;

    if output.status.success()
        || output
            .status
            .code()
            .is_some_and(|code| allowed_status_codes.contains(&code))
    {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = single_line(stderr.trim());
        if stderr.is_empty() {
            Err(format!("{program} exited with {}", output.status))
        } else {
            Err(stderr)
        }
    }
}

fn workspace_path_for_issue(
    snapshot: &SnapshotEnvelope,
    workspace_suffix: &str,
) -> Option<PathBuf> {
    let candidate = PathBuf::from(workspace_suffix);
    if candidate.components().count() != 1 {
        return None;
    }

    Some(PathBuf::from(&snapshot.snapshot.daemon.workspace_root).join(workspace_suffix))
}

impl Model for OperatorApp {
    type Message = AppMessage;

    fn update(&mut self, message: Self::Message) -> Cmd<Self::Message> {
        self.drain_bridge();
        self.drain_workspace_status();
        match message {
            AppMessage::Tick => {}
            AppMessage::MoveSelectionUp => self.state.reduce(TuiAction::MoveSelectionUp),
            AppMessage::MoveSelectionDown => self.state.reduce(TuiAction::MoveSelectionDown),
            AppMessage::FocusNext => self.state.reduce(TuiAction::FocusNext),
            AppMessage::FocusPrevious => self.state.reduce(TuiAction::FocusPrevious),
            AppMessage::ToggleDetailDiff => self.state.reduce(TuiAction::ToggleDetailDiff),
            AppMessage::ToggleTimelineMode => self.state.reduce(TuiAction::ToggleTimelineMode),
            AppMessage::Quit => return Cmd::quit(),
        }
        self.request_selected_workspace_status();
        self.request_selected_workspace_diff();
        self.drain_workspace_status();
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

fn tui_program_config() -> ProgramConfig {
    let mut config = ProgramConfig::fullscreen();
    if cfg!(debug_assertions) {
        config = config
            .with_budget(debug_tui_frame_budget())
            .with_diff_config(RuntimeDiffConfig::default().with_bayesian_enabled(false))
            .with_resize_behavior(ResizeBehavior::Immediate);
        if let Some((width, height)) = debug_initial_tui_size_override() {
            config = config.with_forced_size(width, height);
        }
    }
    config
}

fn debug_initial_tui_size_override() -> Option<(u16, u16)> {
    let deadline = Instant::now() + TUI_SIZE_WAIT;
    loop {
        if current_terminal_size().is_some() {
            return None;
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    env_terminal_size().or(Some((FALLBACK_TUI_WIDTH, FALLBACK_TUI_HEIGHT)))
}

fn current_terminal_size() -> Option<(u16, u16)> {
    terminal::size()
        .ok()
        .filter(|(width, height)| *width >= MIN_TUI_WIDTH && *height >= MIN_TUI_HEIGHT)
}

fn env_terminal_size() -> Option<(u16, u16)> {
    let width = env::var("COLUMNS").ok()?.parse::<u16>().ok()?;
    let height = env::var("LINES").ok()?.parse::<u16>().ok()?;
    (width >= MIN_TUI_WIDTH && height >= MIN_TUI_HEIGHT).then_some((width, height))
}

fn debug_tui_frame_budget() -> FrameBudgetConfig {
    FrameBudgetConfig {
        total: Duration::from_millis(250),
        phase_budgets: PhaseBudgets {
            diff: Duration::from_millis(50),
            present: Duration::from_millis(75),
            render: Duration::from_millis(125),
        },
        allow_frame_skip: false,
        degradation_cooldown: 10,
        upgrade_threshold: 0.5,
    }
}

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("failed to render FrankenTUI runtime: {0}")]
    Runtime(std::io::Error),
    #[error("background control-plane bridge thread panicked during shutdown")]
    BridgeThreadPanicked,
    #[error("background workspace status thread panicked during shutdown")]
    WorkspaceStatusThreadPanicked,
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

/// Parse summary text and colorize metrics: running=# (green), retry_queue=# (yellow)
fn parse_summary_with_colors(summary: &str) -> Vec<Span<'_>> {
    let mut spans = Vec::new();
    let mut remaining = summary;

    // Patterns to match: "running=N" and "retry_queue=N"
    while let Some(pos) = remaining.find("running=") {
        // Add text before the match
        if pos > 0 {
            spans.push(Span::raw(&remaining[..pos]));
        }

        // Find the number after "running="
        let after_marker = &remaining[pos + 8..]; // skip "running="
        let num_end = after_marker
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after_marker.len());
        let num = &after_marker[..num_end];

        // Add "running=" + number in green
        spans.push(Span::styled(
            format!("running={}", num),
            Style::new().fg(GREEN),
        ));

        remaining = &after_marker[num_end..];
    }

    // Add any remaining text after last "running=" match
    if !remaining.is_empty() {
        // Now look for retry_queue in the remaining text
        let mut retry_remaining = remaining;
        while let Some(pos) = retry_remaining.find("retry_queue=") {
            // Add text before the match
            if pos > 0 {
                spans.push(Span::raw(&retry_remaining[..pos]));
            }

            // Find the number after "retry_queue="
            let after_marker = &retry_remaining[pos + 12..]; // skip "retry_queue="
            let num_end = after_marker
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(after_marker.len());
            let num = &after_marker[..num_end];

            // Add "retry_queue=" + number in yellow
            spans.push(Span::styled(
                format!("retry_queue={}", num),
                Style::new().fg(YELLOW),
            ));

            retry_remaining = &after_marker[num_end..];
        }

        if !retry_remaining.is_empty() {
            spans.push(Span::raw(retry_remaining));
        }
    } else if spans.is_empty() {
        // No patterns found, return the whole summary as-is
        spans.push(Span::raw(summary));
    }

    spans
}

#[derive(Clone, Copy)]
enum ConversationDisplayRole {
    User,
    Assistant,
    Action,
    Observation,
    State,
    Error,
    Event,
}

impl ConversationDisplayRole {
    fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Action => "action",
            Self::Observation => "observation",
            Self::State => "state",
            Self::Error => "error",
            Self::Event => "event",
        }
    }

    fn style(self) -> Style {
        match self {
            Self::User => Style::new().fg(BRIGHT_GREEN).bold(),
            Self::Assistant => Style::new().fg(MAGENTA).bold(),
            Self::Action => Style::new().fg(BLUE).bold(),
            Self::Observation => Style::new().fg(BRIGHT_BLACK),
            Self::State => Style::new().fg(BRIGHT_BLACK),
            Self::Error => Style::new().fg(RED).bold(),
            Self::Event => Style::new().fg(CYAN),
        }
    }
}

struct ConversationEventDisplay {
    role: ConversationDisplayRole,
    text: String,
}

fn conversation_event_display(
    event: &crate::opensymphony_domain::ControlPlaneConversationEvent,
) -> ConversationEventDisplay {
    let (role, text) = classify_conversation_event(event);
    ConversationEventDisplay {
        role,
        text: summarize_conversation_text(&normalize_conversation_text(text)),
    }
}

fn classify_conversation_event(
    event: &crate::opensymphony_domain::ControlPlaneConversationEvent,
) -> (ConversationDisplayRole, &str) {
    if let Some((role, text)) = strip_message_role(&event.summary) {
        return (role, text);
    }

    match event.kind.as_str() {
        "ActionEvent" | "tool" | "tool_call" | "tool_use" => {
            (ConversationDisplayRole::Action, event.summary.as_str())
        }
        "ObservationEvent" => (ConversationDisplayRole::Observation, event.summary.as_str()),
        "ConversationErrorEvent" | "error" => {
            (ConversationDisplayRole::Error, event.summary.as_str())
        }
        "ConversationStateUpdateEvent" | "state" => {
            (ConversationDisplayRole::State, event.summary.as_str())
        }
        "MessageEvent" | "message" => (ConversationDisplayRole::Assistant, event.summary.as_str()),
        "assistant" => (ConversationDisplayRole::Assistant, event.summary.as_str()),
        "user" => (ConversationDisplayRole::User, event.summary.as_str()),
        _ => (ConversationDisplayRole::Event, event.summary.as_str()),
    }
}

fn strip_message_role(summary: &str) -> Option<(ConversationDisplayRole, &str)> {
    let trimmed = summary.trim_start();
    let (role, prefix) = if trimmed.starts_with("assistant:") {
        (ConversationDisplayRole::Assistant, "assistant:")
    } else if trimmed.starts_with("user:") {
        (ConversationDisplayRole::User, "user:")
    } else {
        return None;
    };
    Some((role, trimmed[prefix.len()..].trim_start()))
}

fn wrap_conversation_event_styled(
    event: &crate::opensymphony_domain::ControlPlaneConversationEvent,
    width: usize,
) -> Vec<Line> {
    let display = conversation_event_display(event);
    let timestamp_text = format!("{} ", format_timestamp(event.happened_at));
    let continuation_prefix = "  ";
    let label_text = format!("{}>", display.role.label());
    let prefix_width = display_width(&timestamp_text) + display_width(&label_text) + 1;
    let chunks = wrap_text_by_widths(
        &display.text,
        width.saturating_sub(prefix_width),
        width.saturating_sub(display_width(continuation_prefix)),
    );
    let mut lines = Vec::new();

    if let Some(first_chunk) = chunks.first() {
        lines.push(Line::from_spans(vec![
            Span::styled(timestamp_text, Style::new().dim()),
            Span::styled(label_text, display.role.style()),
            Span::raw(" "),
            Span::raw(first_chunk.clone()),
        ]));
    }

    for chunk in chunks.into_iter().skip(1) {
        lines.push(Line::from_spans(vec![
            Span::styled(continuation_prefix, Style::new().dim()),
            Span::raw(chunk),
        ]));
    }

    lines
}

fn wrap_conversation_event_text(
    event: &crate::opensymphony_domain::ControlPlaneConversationEvent,
    width: usize,
) -> Vec<String> {
    let display = conversation_event_display(event);
    let timestamp_text = format!("{} ", format_timestamp(event.happened_at));
    let continuation_prefix = "  ";
    let label_text = format!("{}>", display.role.label());
    let prefix_width = display_width(&timestamp_text) + display_width(&label_text) + 1;
    let chunks = wrap_text_by_widths(
        &display.text,
        width.saturating_sub(prefix_width),
        width.saturating_sub(display_width(continuation_prefix)),
    );
    let mut lines = Vec::new();

    if let Some(first_chunk) = chunks.first() {
        lines.push(format!("{timestamp_text}{label_text} {first_chunk}"));
    }

    for chunk in chunks.into_iter().skip(1) {
        lines.push(format!("{continuation_prefix}{chunk}"));
    }

    lines
}

fn normalize_conversation_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

fn summarize_conversation_text(text: &str) -> String {
    if text.chars().count() <= TUI_CONVERSATION_TEXT_LIMIT {
        return text.to_owned();
    }

    let shortened = text
        .chars()
        .take(TUI_CONVERSATION_TEXT_LIMIT.saturating_sub(3))
        .collect::<String>();
    format!("{shortened}...")
}

fn wrap_text_by_widths(value: &str, first_width: usize, continuation_width: usize) -> Vec<String> {
    let value = single_line(value);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    let mut target_width = max(1, first_width);

    for ch in value.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width + ch_width > target_width && !current.is_empty() {
            chunks.push(current.trim_end().to_owned());
            current.clear();
            current_width = 0;
            target_width = max(1, continuation_width);
        }

        if current.is_empty() && ch.is_whitespace() {
            continue;
        }

        current.push(ch);
        current_width += ch_width;
    }

    if !current.is_empty() {
        chunks.push(current.trim_end().to_owned());
    }

    if chunks.is_empty() {
        chunks.push(String::new());
    }

    chunks
}

fn conversation_scroll_start(
    total_lines: usize,
    visible_rows: usize,
    scroll_offset_from_bottom: usize,
) -> usize {
    total_lines.saturating_sub(visible_rows.saturating_add(scroll_offset_from_bottom))
}

fn change_count_text(prefix: char, count: Option<u64>) -> String {
    match count {
        Some(count) => format!("{prefix}{count}"),
        None => format!("{prefix}?"),
    }
}

fn change_summary_line_text(summary: &WorkspaceChangeSummary) -> String {
    let noun = if summary.files_changed == 1 {
        "file"
    } else {
        "files"
    };
    format!(
        "{} {} changed {} {}",
        summary.files_changed,
        noun,
        change_count_text('+', Some(summary.additions)),
        change_count_text('-', Some(summary.deletions))
    )
}

fn change_summary_line_styled(summary: &WorkspaceChangeSummary) -> Line {
    Line::from_spans(vec![
        Span::raw(format!(
            "{} {} changed ",
            summary.files_changed,
            if summary.files_changed == 1 {
                "file"
            } else {
                "files"
            }
        )),
        Span::styled(
            change_count_text('+', Some(summary.additions)),
            Style::new().fg(BRIGHT_GREEN).bold(),
        ),
        Span::raw(" "),
        Span::styled(
            change_count_text('-', Some(summary.deletions)),
            Style::new().fg(RED).bold(),
        ),
    ])
}

fn change_target_line_text(
    title: &str,
    additions: Option<u64>,
    deletions: Option<u64>,
    width: usize,
    is_selected: bool,
    is_open: bool,
) -> String {
    let marker = if is_selected {
        if is_open { "v" } else { ">" }
    } else {
        " "
    };
    let additions_text = change_count_text('+', additions);
    let deletions_text = change_count_text('-', deletions);
    let reserved_width = 4 + display_width(&additions_text) + display_width(&deletions_text);
    let path_width = max(1, width.saturating_sub(reserved_width));
    format!(
        "{marker} {} {} {}",
        fit(title, path_width),
        additions_text,
        deletions_text
    )
}

fn change_target_line_styled(
    title: &str,
    additions: Option<u64>,
    deletions: Option<u64>,
    width: usize,
) -> Line {
    let additions_text = change_count_text('+', additions);
    let deletions_text = change_count_text('-', deletions);
    let reserved_width = 4 + display_width(&additions_text) + display_width(&deletions_text);
    let path_width = max(1, width.saturating_sub(reserved_width));

    Line::from_spans(vec![
        Span::raw("  "),
        Span::styled(fit(title, path_width), Style::new().bold()),
        Span::raw(" "),
        Span::styled(additions_text, Style::new().fg(BRIGHT_GREEN).bold()),
        Span::raw(" "),
        Span::styled(deletions_text, Style::new().fg(RED).bold()),
    ])
}

fn render_workspace_diff_line_styled(diff_line: &WorkspaceDiffLine, width: usize) -> Line {
    let style = match diff_line.kind {
        WorkspaceDiffLineKind::Header => Style::new().dim(),
        WorkspaceDiffLineKind::Hunk => Style::new().fg(CYAN),
        WorkspaceDiffLineKind::Addition => Style::new().fg(BRIGHT_GREEN),
        WorkspaceDiffLineKind::Deletion => Style::new().fg(RED),
        WorkspaceDiffLineKind::Context => Style::new(),
        WorkspaceDiffLineKind::Note => Style::new().dim(),
    };
    Line::from(Span::styled(fit(&diff_line.text, width), style))
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
                    let mut truncated_span =
                        Span::styled(truncated.to_string(), span.style.unwrap_or_default());
                    if let Some(link) = &span.link {
                        truncated_span = truncated_span.link(link.clone().into_owned());
                    }
                    spans.push(truncated_span);
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
                    let mut truncated_span =
                        Span::styled(truncated.to_string(), span.style.unwrap_or_default());
                    if let Some(link) = &span.link {
                        truncated_span = truncated_span.link(link.clone().into_owned());
                    }
                    spans.push(truncated_span);
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

impl WorkerOutcomeLabel for crate::opensymphony_domain::ControlPlaneWorkerOutcome {
    fn as_str(&self) -> &'static str {
        match self {
            crate::opensymphony_domain::ControlPlaneWorkerOutcome::Unknown => "unknown",
            crate::opensymphony_domain::ControlPlaneWorkerOutcome::Running => "running",
            crate::opensymphony_domain::ControlPlaneWorkerOutcome::Continued => "continued",
            crate::opensymphony_domain::ControlPlaneWorkerOutcome::Completed => "completed",
            crate::opensymphony_domain::ControlPlaneWorkerOutcome::Failed => "failed",
            crate::opensymphony_domain::ControlPlaneWorkerOutcome::Canceled => "canceled",
        }
    }
}

trait DaemonStateLabel {
    fn as_str(&self) -> &'static str;
}

impl DaemonStateLabel for crate::opensymphony_domain::ControlPlaneDaemonState {
    fn as_str(&self) -> &'static str {
        match self {
            crate::opensymphony_domain::ControlPlaneDaemonState::Starting => "starting",
            crate::opensymphony_domain::ControlPlaneDaemonState::Ready => "ready",
            crate::opensymphony_domain::ControlPlaneDaemonState::Degraded => "degraded",
            crate::opensymphony_domain::ControlPlaneDaemonState::Stopped => "stopped",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AppMessage, BLUE, BridgeHandle, BridgeMailbox, ConnectionState, ControlPlaneClientError,
        FocusPane, OperatorApp, RunOutcome, TuiAction, TuiState, WorkspaceChangeState,
        WorkspaceChangeSummary, WorkspaceDiffLine, WorkspaceDiffLineKind, WorkspaceFileChange,
        WorkspaceFileDiffState, build_workspace_change_summary, display_width, fit,
        handle_bridge_error, issue_window, load_workspace_diff, section_layout,
        stacked_body_layout, two_column_block_styled, visible_issue_count,
    };
    use crate::opensymphony_domain::{
        ControlPlaneAgentServerStatus as AgentServerStatus, ControlPlaneConversationEvent,
        ControlPlaneDaemonSnapshot as DaemonSnapshot, ControlPlaneDaemonState as DaemonState,
        ControlPlaneDaemonStatus as DaemonStatus, ControlPlaneFileChange,
        ControlPlaneFileChangeKind, ControlPlaneIssueRuntimeState as IssueRuntimeState,
        ControlPlaneIssueSnapshot as IssueSnapshot, ControlPlaneMetricsSnapshot as MetricsSnapshot,
        ControlPlaneRecentEvent as RecentEvent, ControlPlaneRecentEventKind as RecentEventKind,
        ControlPlaneWorkerOutcome as WorkerOutcome, SnapshotEnvelope,
    };
    use chrono::{TimeZone, Utc};
    use ftui::{
        Style,
        prelude::Model,
        text::text::{Line, Span},
    };
    use std::{
        fs,
        path::Path,
        process::Command,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };
    use tempfile::TempDir;
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
                    input_tokens: 512,
                    output_tokens: 512,
                    cache_read_tokens: 256,
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
                        input_tokens: 1024 + (index as u64 * 100),
                        output_tokens: 512 + (index as u64 * 50),
                        cache_read_tokens: 256 + (index as u64 * 25),
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

    fn run_git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("git command should spawn");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
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
        let workspace_status = Arc::new(Mutex::new(super::WorkspaceStatusMailbox::default()));
        let (workspace_status_tx, workspace_status_rx) = mpsc::channel();
        drop(workspace_status_rx);
        let outcome = Arc::new(Mutex::new(RunOutcome::default()));
        let mut app = OperatorApp::new(
            Arc::clone(&bridge),
            workspace_status,
            workspace_status_tx,
            None,
            outcome,
        );
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
        let workspace_status = Arc::new(Mutex::new(super::WorkspaceStatusMailbox::default()));
        let (workspace_status_tx, workspace_status_rx) = mpsc::channel();
        drop(workspace_status_rx);
        let outcome = Arc::new(Mutex::new(RunOutcome::default()));
        let mut app = OperatorApp::new(
            Arc::clone(&bridge),
            workspace_status,
            workspace_status_tx,
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
        assert!(rendered.contains("branch: loading..."));
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
    fn two_column_layout_preserves_hyperlinks() {
        let left = vec![Line::from_spans(vec![
            Span::styled(
                "https://github.com/kumanday/OpenSymphony/pull/42",
                Style::new().fg(BLUE).underline(),
            )
            .link("https://github.com/kumanday/OpenSymphony/pull/42"),
        ])];
        let right = vec![Line::from(Span::raw("activity"))];

        let rendered = two_column_block_styled(&left, &right, 20, 10);
        let first_span = rendered[0]
            .spans()
            .first()
            .expect("left span should still exist");

        assert_eq!(
            first_span.link.as_deref(),
            Some("https://github.com/kumanday/OpenSymphony/pull/42")
        );
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

    #[test]
    fn wide_detail_metadata_marks_focus_when_detail_is_active() {
        let mut state = TuiState::default();
        state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(8, 1))));
        state.focus = FocusPane::Detail;

        let title = state.metadata_and_files_lines(48, 12)[0].to_plain_text();

        assert_eq!(title.trim_end(), "[x] ISSUE + WORKSPACE DETAIL");
    }

    #[test]
    fn diff_pane_claims_focus_when_opened_from_detail_navigation() {
        let mut state = TuiState::default();
        state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(8, 1))));
        state.reduce(TuiAction::WorkspaceStatusLoaded {
            issue_identifier: "COE-255".to_owned(),
            branch: "codex/tui-workspace-git-status".to_owned(),
            pr_url: None,
            changes: WorkspaceChangeState::Available(WorkspaceChangeSummary {
                files_changed: 1,
                additions: 3,
                deletions: 1,
                files: vec![WorkspaceFileChange {
                    display_path: "src/lib.rs".to_owned(),
                    query_path: "src/lib.rs".to_owned(),
                    previous_path: None,
                    status_code: "M".to_owned(),
                    additions: Some(3),
                    deletions: Some(1),
                    diff: WorkspaceFileDiffState::Loaded(vec![WorkspaceDiffLine {
                        kind: WorkspaceDiffLineKind::Addition,
                        text: "+assert!(true);".to_owned(),
                    }]),
                }],
            }),
        });
        state.focus = FocusPane::Detail;
        state.reduce(TuiAction::ToggleDetailDiff);

        let detail_title = state.metadata_and_files_lines(48, 12)[0].to_plain_text();
        let diff_title = state.selected_diff_lines_styled(48, 12)[0].to_plain_text();

        assert_eq!(detail_title.trim_end(), "[ ] ISSUE + WORKSPACE DETAIL");
        assert_eq!(diff_title.trim_end(), "[x] FILE DIFF");
        assert_eq!(state.focus, FocusPane::Activity);
    }

    #[test]
    fn activity_focus_scrolls_loaded_diff_instead_of_changing_selected_file() {
        let mut state = TuiState::default();
        state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(8, 1))));
        state.reduce(TuiAction::WorkspaceStatusLoaded {
            issue_identifier: "COE-255".to_owned(),
            branch: "codex/tui-workspace-git-status".to_owned(),
            pr_url: None,
            changes: WorkspaceChangeState::Available(WorkspaceChangeSummary {
                files_changed: 2,
                additions: 5,
                deletions: 1,
                files: vec![
                    WorkspaceFileChange {
                        display_path: "src/lib.rs".to_owned(),
                        query_path: "src/lib.rs".to_owned(),
                        previous_path: None,
                        status_code: "M".to_owned(),
                        additions: Some(4),
                        deletions: Some(1),
                        diff: WorkspaceFileDiffState::Loaded(vec![
                            WorkspaceDiffLine {
                                kind: WorkspaceDiffLineKind::Context,
                                text: " line 1".to_owned(),
                            },
                            WorkspaceDiffLine {
                                kind: WorkspaceDiffLineKind::Addition,
                                text: "+line 2".to_owned(),
                            },
                            WorkspaceDiffLine {
                                kind: WorkspaceDiffLineKind::Addition,
                                text: "+line 3".to_owned(),
                            },
                        ]),
                    },
                    WorkspaceFileChange {
                        display_path: "src/main.rs".to_owned(),
                        query_path: "src/main.rs".to_owned(),
                        previous_path: None,
                        status_code: "M".to_owned(),
                        additions: Some(1),
                        deletions: Some(0),
                        diff: WorkspaceFileDiffState::Unloaded,
                    },
                ],
            }),
        });
        state.focus = FocusPane::Detail;
        state.reduce(TuiAction::ToggleDetailDiff);
        state.reduce(TuiAction::MoveSelectionDown);

        assert_eq!(state.selected_changed_file, 0);
        assert_eq!(state.focus, FocusPane::Activity);
        assert_eq!(state.diff_scroll_offset, 1);
    }

    #[test]
    fn conversation_activity_lines_use_available_width_before_truncating() {
        let mut state = TuiState::default();
        let mut snapshot = fixture(8, 1);
        let summary =
            "this summary is intentionally longer than forty characters for the detail pane";
        snapshot.snapshot.issues[0].recent_events = vec![ControlPlaneConversationEvent {
            event_id: "evt-long".to_owned(),
            happened_at: snapshot.snapshot.generated_at,
            kind: "message".to_owned(),
            summary: summary.to_owned(),
        }];
        state.reduce(TuiAction::SnapshotReceived(Box::new(snapshot)));

        let lines = state.conversation_activity_lines(120, 4);
        let rendered = lines[1].to_plain_text();

        assert!(rendered.contains(summary));
    }

    #[test]
    fn conversation_activity_wraps_long_summaries_across_multiple_rows() {
        let mut state = TuiState::default();
        let mut snapshot = fixture(8, 1);
        snapshot.snapshot.issues[0].recent_events = vec![ControlPlaneConversationEvent {
            event_id: "evt-wrap".to_owned(),
            happened_at: snapshot.snapshot.generated_at,
            kind: "ObservationEvent".to_owned(),
            summary:
                "this summary is long enough that it should wrap into the next visual row cleanly"
                    .to_owned(),
        }];
        state.reduce(TuiAction::SnapshotReceived(Box::new(snapshot)));

        let lines = state.conversation_activity_lines(48, 6);

        assert!(lines.len() >= 3);
        assert!(lines[1].to_plain_text().contains("observation>"));
        assert!(!lines[2].to_plain_text().trim().is_empty());
    }

    #[test]
    fn activity_focus_scrolls_conversation_history() {
        let mut state = TuiState::default();
        let mut snapshot = fixture(8, 1);
        snapshot.snapshot.issues[0].recent_events = vec![
            ControlPlaneConversationEvent {
                event_id: "evt-4".to_owned(),
                happened_at: snapshot.snapshot.generated_at,
                kind: "message".to_owned(),
                summary: "newest event".to_owned(),
            },
            ControlPlaneConversationEvent {
                event_id: "evt-3".to_owned(),
                happened_at: snapshot.snapshot.generated_at,
                kind: "message".to_owned(),
                summary: "newer event".to_owned(),
            },
            ControlPlaneConversationEvent {
                event_id: "evt-2".to_owned(),
                happened_at: snapshot.snapshot.generated_at,
                kind: "message".to_owned(),
                summary: "middle event".to_owned(),
            },
            ControlPlaneConversationEvent {
                event_id: "evt-1".to_owned(),
                happened_at: snapshot.snapshot.generated_at,
                kind: "message".to_owned(),
                summary: "oldest event".to_owned(),
            },
        ];
        state.reduce(TuiAction::SnapshotReceived(Box::new(snapshot)));
        state.reduce(TuiAction::FocusNext);
        state.reduce(TuiAction::FocusNext);
        state.reduce(TuiAction::MoveSelectionUp);

        let lines = state.conversation_activity_lines(80, 4);
        let rendered = lines
            .iter()
            .map(Line::to_plain_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(state.focus, FocusPane::Activity);
        assert!(rendered.contains("[x] CONVERSATION ACTIVITY"));
        assert!(!rendered.contains("newest event"));
        assert!(rendered.contains("middle event"));
        assert!(rendered.contains("oldest event"));
    }

    #[test]
    fn conversation_activity_defaults_to_latest_output_and_tracks_new_snapshots() {
        let mut state = TuiState::default();
        let mut snapshot = fixture(8, 1);
        snapshot.snapshot.issues[0].recent_events = vec![
            ControlPlaneConversationEvent {
                event_id: "evt-4".to_owned(),
                happened_at: snapshot.snapshot.generated_at,
                kind: "message".to_owned(),
                summary: "fourth event".to_owned(),
            },
            ControlPlaneConversationEvent {
                event_id: "evt-3".to_owned(),
                happened_at: snapshot.snapshot.generated_at,
                kind: "message".to_owned(),
                summary: "third event".to_owned(),
            },
            ControlPlaneConversationEvent {
                event_id: "evt-2".to_owned(),
                happened_at: snapshot.snapshot.generated_at,
                kind: "message".to_owned(),
                summary: "second event".to_owned(),
            },
            ControlPlaneConversationEvent {
                event_id: "evt-1".to_owned(),
                happened_at: snapshot.snapshot.generated_at,
                kind: "message".to_owned(),
                summary: "first event".to_owned(),
            },
        ];
        state.reduce(TuiAction::SnapshotReceived(Box::new(snapshot.clone())));
        state.reduce(TuiAction::FocusNext);
        state.reduce(TuiAction::FocusNext);

        let initial = state
            .conversation_activity_lines(80, 3)
            .iter()
            .map(Line::to_plain_text)
            .collect::<Vec<_>>()
            .join("\n");

        snapshot.snapshot.issues[0].recent_events.insert(
            0,
            ControlPlaneConversationEvent {
                event_id: "evt-5".to_owned(),
                happened_at: snapshot.snapshot.generated_at,
                kind: "message".to_owned(),
                summary: "fifth event".to_owned(),
            },
        );
        state.reduce(TuiAction::SnapshotReceived(Box::new(snapshot)));

        let updated = state
            .conversation_activity_lines(80, 3)
            .iter()
            .map(Line::to_plain_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(initial.contains("third event"));
        assert!(initial.contains("fourth event"));
        assert!(!initial.contains("second event"));
        assert!(!initial.contains("first event"));
        assert!(updated.contains("fourth event"));
        assert!(updated.contains("fifth event"));
        assert!(!updated.contains("third event"));
    }

    #[test]
    fn workspace_change_summary_includes_branch_commits_and_uncommitted_edits_vs_main() {
        let repo = TempDir::new().expect("temp repo should be created");
        run_git(repo.path(), &["init", "-b", "main"]);
        run_git(repo.path(), &["config", "user.name", "Test User"]);
        run_git(repo.path(), &["config", "user.email", "test@example.com"]);

        fs::create_dir_all(repo.path().join("src")).expect("src dir should exist");
        fs::write(repo.path().join("src/lib.rs"), "alpha\nbeta\n")
            .expect("base file should be written");
        run_git(repo.path(), &["add", "src/lib.rs"]);
        run_git(repo.path(), &["commit", "-m", "base"]);

        run_git(repo.path(), &["switch", "-c", "feature"]);
        fs::write(repo.path().join("src/lib.rs"), "alpha\nbeta\ngamma\n")
            .expect("branch file should be written");
        run_git(repo.path(), &["add", "src/lib.rs"]);
        run_git(repo.path(), &["commit", "-m", "branch change"]);

        fs::write(
            repo.path().join("src/lib.rs"),
            "alpha\nbeta\ngamma\ndelta\n",
        )
        .expect("working tree edit should be written");
        fs::write(repo.path().join("notes.txt"), "note one\nnote two\n")
            .expect("untracked file should be written");

        let summary =
            build_workspace_change_summary(repo.path()).expect("workspace summary should load");
        let lib_change = summary
            .files
            .iter()
            .find(|file| file.query_path == "src/lib.rs")
            .expect("lib.rs change should be present");
        let notes_change = summary
            .files
            .iter()
            .find(|file| file.query_path == "notes.txt")
            .expect("notes.txt change should be present");

        assert_eq!(summary.files_changed, 2);
        assert_eq!(summary.additions, 4);
        assert_eq!(summary.deletions, 0);
        assert_eq!(lib_change.additions, Some(2));
        assert_eq!(lib_change.deletions, Some(0));
        assert_eq!(notes_change.additions, Some(2));
        assert_eq!(notes_change.deletions, Some(0));

        let diff = load_workspace_diff(repo.path(), "src/lib.rs", None, "M")
            .expect("workspace diff should load");
        let diff_text = diff
            .into_iter()
            .map(|line| line.text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(diff_text.contains("+gamma"));
        assert!(diff_text.contains("+delta"));
    }
}
