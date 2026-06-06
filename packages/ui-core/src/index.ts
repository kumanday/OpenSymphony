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
} from "@opensymphony/gateway-schema";

// Terminal renderer module
export * from "./terminal-renderer/index.js";

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