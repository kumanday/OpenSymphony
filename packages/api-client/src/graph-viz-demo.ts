import type {
  ChangedFileEntry,
  DashboardSnapshot,
  FileDiffPage,
  GatewayCapabilities,
  RunDetail,
  RunEventPage,
  TaskGraphNode,
  TaskGraphSnapshot,
} from "@opensymphony/gateway-schema";
import { MockGatewayTransport } from "./mock.js";

/**
 * Deterministic task-graph demo fixture for graph-visualization work.
 *
 * The graph deliberately contains the dependency shapes that stress the
 * task-graph arrow routing: several skip-level dependencies fanning out from
 * one blocker, skip-level dependencies from *different* blockers spanning the
 * same rows (the case that conflated the old L-shaped routes), plus ordinary
 * next-row edges. Paired with the knowledge-graph fixture in
 * `@opensymphony/graph` it powers the desktop `?fixtures` workbench; see
 * docs/graph-view.md ("Graph visualization workbench").
 */

const schema_version = { major: 1, minor: 0, patch: 0 };
const generatedAt = "2026-07-04T00:00:00Z";
const projectId = "viz-workbench";

interface DemoIssue {
  id: string;
  title: string;
  state: "Todo" | "In Progress" | "Human Review" | "Done";
  blockedBy?: string[];
  running?: boolean;
}

// Row order matters: the renderer lays nodes out top-to-bottom in array
// order, so the blockedBy spans below encode the arrow shapes under test.
const demoIssues: DemoIssue[] = [
  { id: "VIZ-101", title: "Scene model and shared projector", state: "Done" },
  { id: "VIZ-102", title: "Area hull computation", state: "In Progress", blockedBy: ["VIZ-101"], running: true },
  { id: "VIZ-103", title: "Hover highlight and tooltip", state: "In Progress", blockedBy: ["VIZ-101"], running: true },
  { id: "VIZ-104", title: "Node dragging interactions", state: "Todo", blockedBy: ["VIZ-103"] },
  { id: "VIZ-105", title: "Zoom-dependent label fades", state: "Todo", blockedBy: ["VIZ-101", "VIZ-102"] },
  { id: "VIZ-106", title: "Camera framing animations", state: "Todo", blockedBy: ["VIZ-102"] },
  { id: "VIZ-107", title: "Task graph lane routing", state: "Human Review", blockedBy: ["VIZ-101"] },
  { id: "VIZ-108", title: "Arrow palette and markers", state: "Todo", blockedBy: ["VIZ-107"] },
  { id: "VIZ-109", title: "Dependency hover emphasis", state: "Todo", blockedBy: ["VIZ-103", "VIZ-107"] },
  { id: "VIZ-110", title: "Fixture workbench docs", state: "Todo", blockedBy: ["VIZ-102"] },
  { id: "VIZ-111", title: "Playwright visual sweep", state: "Todo", blockedBy: ["VIZ-105", "VIZ-108"] },
  { id: "VIZ-112", title: "Reduced motion audit", state: "Todo", blockedBy: ["VIZ-103"] },
  { id: "VIZ-113", title: "Release notes and demo capture", state: "Todo", blockedBy: ["VIZ-111", "VIZ-112", "VIZ-101"] },
];

function demoNode(issue: DemoIssue): TaskGraphNode {
  const stateCategory = issue.state === "Done"
    ? "done"
    : issue.state === "Todo"
      ? "todo"
      : "in_progress";
  return {
    schema_version,
    node_id: issue.id.toLowerCase(),
    kind: "issue",
    identifier: issue.id,
    title: issue.title,
    state: issue.state,
    state_category: stateCategory,
    parent_id: "viz-milestone",
    children: [],
    blocked_by: issue.blockedBy ?? [],
    labels: ["graph-viz"],
    run_id: issue.running ? issue.id : undefined,
  };
}

export const graphVizDemoTaskGraph: TaskGraphSnapshot = {
  schema_version,
  project_id: projectId,
  generated_at: generatedAt,
  root_ids: ["viz-milestone"],
  nodes: [
    {
      schema_version,
      node_id: "viz-milestone",
      kind: "milestone",
      identifier: "M13",
      title: "Graph Visualization Command Center",
      state: "In Progress",
      state_category: "in_progress",
      children: demoIssues.map((issue) => issue.id.toLowerCase()),
      blocked_by: [],
      labels: ["graph-viz"],
    },
    ...demoIssues.map(demoNode),
  ],
};

const demoCapabilities: GatewayCapabilities = {
  schema_version,
  gateway_version: "graph-viz-demo",
  supported_api_versions: ["1.0.0"],
  transports: [
    { transport: "loopback_http", modes: ["json"], supported_encodings: ["utf-8"], bidirectional: false },
  ],
  features: [{ feature: "task_graph", available: true, requires_auth: false }],
  auth_modes: ["none"],
  max_event_page_size: 1000,
  max_terminal_frame_batch: 500,
};

const demoDashboard: DashboardSnapshot = {
  schema_version,
  generated_at: generatedAt,
  sequence: 1,
  health: "healthy",
  metrics: {
    running_issue_count: demoIssues.filter((issue) => issue.running).length,
    retry_queue_depth: 0,
    total_input_tokens: 128_000,
    total_output_tokens: 24_000,
    total_cache_read_tokens: 64_000,
    total_cost_micros: 4_200,
  },
  projects: [
    {
      project_id: projectId,
      name: "Graph Viz Workbench",
      milestone_count: 1,
      issue_count: demoIssues.length,
      running_count: demoIssues.filter((issue) => issue.running).length,
      completed_count: demoIssues.filter((issue) => issue.state === "Done").length,
      failed_count: 0,
    },
  ],
  recent_events: [
    {
      happened_at: generatedAt,
      kind: "snapshot_published",
      issue_identifier: "VIZ-102",
      summary: "graph viz fixture snapshot published",
    },
  ],
};

function demoRunDetail(runId: string): RunDetail {
  return {
    schema_version,
    run_id: runId,
    issue_id: `issue-${runId}`,
    issue_identifier: runId,
    worker_id: "viz-worker",
    status: "running",
    claimed_at: generatedAt,
    started_at: generatedAt,
    turn_count: 2,
    max_turns: 8,
    input_tokens: 12_000,
    output_tokens: 2_400,
    cache_read_tokens: 6_000,
    runtime_seconds: 210,
    workspace_path: `/tmp/opensymphony/projects/${runId}`,
    branch_name: `opensymphony/${runId.toLowerCase()}-graph-viz`,
    safe_actions: { retry: false, cancel: true, rehydrate: true, detach: false },
  };
}

const demoChangedFiles: ChangedFileEntry[] = [
  { path: "packages/ui-core/src/knowledge-graph-renderer.ts", change_kind: "modified", lines_added: 320, lines_removed: 180 },
  { path: "packages/ui-core/src/knowledge-graph-scene.ts", change_kind: "created", lines_added: 400, lines_removed: 0 },
  { path: "packages/graph/src/viz-fixture.ts", change_kind: "created", lines_added: 240, lines_removed: 0 },
];

function demoDiff(runId: string, filePath: string): FileDiffPage {
  return {
    schema_version,
    run_id: runId,
    file_path: filePath,
    hunks: [
      {
        file_path: filePath,
        header: "@@ -1,2 +1,4 @@",
        start_line: 1,
        old_line_count: 2,
        new_line_count: 4,
        lines: [
          { type: "context", line: "// graph viz workbench" },
          { type: "addition", line: "export const hulls = buildAreaHulls(layout);" },
          { type: "addition", line: "export const tooltip = buildTooltipModel(hover);" },
          { type: "context", line: "" },
        ],
      },
    ],
    total_lines_added: 2,
    total_lines_removed: 0,
  };
}

function demoEvents(runId: string): RunEventPage {
  return {
    schema_version,
    run_id: runId,
    events: [
      {
        sequence: 1,
        event_id: `${runId}-evt-1`,
        happened_at: generatedAt,
        kind: "ActionEvent",
        summary: `Working ${runId}: iterating on graph visuals`,
      },
    ],
  };
}

/**
 * Full mock gateway wired with the graph-viz demo data. The desktop app
 * mounts on this transport when started with `?fixtures` (see
 * apps/desktop/src/index.ts) so graph work can iterate against dense,
 * stable data without a running daemon.
 */
export function createGraphVizDemoTransport(): MockGatewayTransport {
  const runningIds = demoIssues.filter((issue) => issue.running).map((issue) => issue.id);
  return new MockGatewayTransport({
    baseUri: "http://graph-viz.fixtures.local",
    health: demoCapabilities,
    snapshot: demoDashboard,
    taskGraph: graphVizDemoTaskGraph,
    runDetails: runningIds.map(demoRunDetail),
    runFiles: runningIds.map((runId) => ({ runId, files: demoChangedFiles })),
    runDiffs: runningIds.flatMap((runId) =>
      demoChangedFiles.map((file) => ({ runId, filePath: file.path, diff: demoDiff(runId, file.path) })),
    ),
    runEvents: runningIds.map(demoEvents),
  });
}
