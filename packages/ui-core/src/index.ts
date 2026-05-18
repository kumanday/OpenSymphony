/**
 * Shared UI core module.
 *
 * This package is a placeholder for shared UI utilities and type
 * definitions that both desktop and web shells will consume. The
 * actual component framework (React, Svelte, etc.) will be added
 * in a future ticket.
 */

import type {
  DashboardSnapshot,
  TaskGraphNode,
  RunDetail,
  TerminalFrame,
} from "@opensymphony/gateway-schema";

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