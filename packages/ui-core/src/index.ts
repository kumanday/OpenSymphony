/**
 * Shared UI core module.
 *
 * Provides terminal rendering utilities, scrollback buffer management,
 * and benchmark harness for high-throughput terminal/log output.
 */

import type {
  DashboardSnapshot,
  TaskGraphNode,
  RunDetail,
  TerminalFrame,
  RunTimeline,
  TimelineEntry,
  TimelineEntryKind,
  TerminalLogAssociation,
} from "@opensymphony/gateway-schema";

// Terminal renderer module
export * from "./terminal-renderer/index.js";

// Timeline renderer module
export {
  renderTimeline,
  filterTimelineEntries,
  findTimelineEntryByEventId,
  findTimelineEntryByEntityId,
} from "./timeline.js";

export {
  renderOpenSymphonyApp,
} from "./app-shell.js";
export type {
  EditableProfileInput,
  GatewayReader,
  OpenSymphonyAppHandle,
  OpenSymphonyAppOptions,
  ProfileController,
} from "./app-shell.js";

export interface UiTheme {
  mode: "light" | "dark";
  accent?: string;
}

export interface TerminalRenderConfig {
  fontFamily: string;
  fontSize: number;
  lineHeight: number;
  wrapLines: boolean;
  maxVisibleFrames: number;
}

export type TerminalFrameWithMeta = TerminalFrame & {
  renderedAt: string;
};

export type DashboardData = DashboardSnapshot;
export type TaskGraphData = TaskGraphNode[];
export type RunData = RunDetail;
export type RunTimelineData = RunTimeline;
export type TimelineEntryData = TimelineEntry;
export type TimelineEntryKindData = TimelineEntryKind;
export type TerminalLogAssociationData = TerminalLogAssociation;
