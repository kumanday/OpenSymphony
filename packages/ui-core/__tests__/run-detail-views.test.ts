/**
 * @jest-environment jsdom
 *
 * Fixture tests for diff, validation, approval, and run action views across
 * active, quiet, degraded, stalled, detached, cancel-failed, and terminal
 * run states.
 */

import { renderOpenSymphonyApp } from "../src/app-shell.js";
import { MockGatewayTransport } from "@opensymphony/api-client";
import { schemaVersionV1 } from "@opensymphony/gateway-schema";
import type {
  ApprovalRequest,
  ChangedFileEntry,
  DashboardSnapshot,
  FileDiffPage,
  GatewayCapabilities,
  RunDetail,
  RunValidationSummary,
  TaskGraphSnapshot,
} from "@opensymphony/gateway-schema";

const capabilities: GatewayCapabilities = {
  schema_version: schemaVersionV1(),
  gateway_version: "test-gw",
  supported_api_versions: ["1.0.0"],
  transports: [
    {
      transport: "loopback_http",
      modes: ["json"],
      supported_encodings: ["utf-8"],
      bidirectional: false,
    },
  ],
  features: [
    { feature: "task_graph", available: true, requires_auth: false },
    { feature: "terminal_stream", available: false, requires_auth: false },
  ],
  auth_modes: ["none"],
  max_event_page_size: 1000,
  max_terminal_frame_batch: 500,
};

const dashboard: DashboardSnapshot = {
  schema_version: schemaVersionV1(),
  generated_at: "2025-09-01T00:00:00Z",
  sequence: 1,
  health: "healthy",
  metrics: {
    running_issue_count: 1,
    retry_queue_depth: 0,
    total_input_tokens: 100,
    total_output_tokens: 50,
    total_cache_read_tokens: 0,
    total_cost_micros: 0,
  },
  projects: [
    {
      project_id: "proj-1",
      name: "Test Project",
      milestone_count: 1,
      issue_count: 1,
      running_count: 1,
      completed_count: 0,
      failed_count: 0,
    },
  ],
  recent_events: [],
};

const taskGraph: TaskGraphSnapshot = {
  schema_version: schemaVersionV1(),
  project_id: "proj-1",
  generated_at: "2025-09-01T00:00:00Z",
  root_ids: ["issue-1"],
  nodes: [
    {
      schema_version: schemaVersionV1(),
      node_id: "issue-1",
      kind: "issue",
      identifier: "run-414",
      title: "Run detail views",
      state: "In Progress",
      state_category: "in_progress",
      children: [],
      blocked_by: [],
      labels: [],
    },
  ],
};

const runDetail: RunDetail = {
  schema_version: schemaVersionV1(),
  run_id: "run-414",
  issue_id: "issue-414",
  issue_identifier: "COE-414",
  worker_id: "worker-1",
  status: "running",
  claimed_at: "2025-09-01T00:00:00Z",
  started_at: "2025-09-01T00:00:00Z",
  turn_count: 1,
  max_turns: 8,
  input_tokens: 100,
  output_tokens: 50,
  cache_read_tokens: 0,
  runtime_seconds: 30,
  allowed_actions: ["retry", "cancel", "rehydrate", "detach", "comment", "create_followup", "open_workspace", "debug"],
  safe_actions: {
    retry: true,
    cancel: true,
    rehydrate: true,
    detach: true,
  },
};

const files: ChangedFileEntry[] = [
  {
    path: "src/config.ts",
    change_kind: "modified",
    lines_added: 4,
    lines_removed: 1,
    size_bytes: 256,
  },
  {
    path: "src/new.ts",
    change_kind: "created",
    lines_added: 42,
    lines_removed: 0,
    size_bytes: 1024,
  },
];

const diff: FileDiffPage = {
  schema_version: schemaVersionV1(),
  run_id: "run-414",
  file_path: "src/config.ts",
  hunks: [
    {
      file_path: "src/config.ts",
      header: "@@ -1,3 +1,4 @@",
      start_line: 1,
      old_line_count: 3,
      new_line_count: 4,
      lines: [
        { type: "context", line: "const API_URL = 'https://example.com';" },
        { type: "deletion", line: "export const timeout = 5000;" },
        { type: "addition", line: "export const timeout = 10000;" },
        { type: "addition", line: "export const retries = 3;" },
      ],
    },
  ],
  total_lines_added: 2,
  total_lines_removed: 1,
};

const validation: RunValidationSummary = {
  schema_version: schemaVersionV1(),
  run_id: "run-414",
  generated_at: "2025-09-01T00:00:00Z",
  overall_status: "passed",
  commands: [
    {
      command_id: "cmd-1",
      command: "npm test",
      status: "passed",
      exit_code: 0,
      stdout_summary: "all tests passed",
    },
  ],
  evidence: [
    {
      evidence_id: "ev-1",
      label: "type coverage",
      status: "passed",
      summary: "Coverage is 92%",
      file_path: "src/config.ts",
      line_number: 7,
    },
  ],
};

const approvals: ApprovalRequest[] = [
  {
    schema_version: schemaVersionV1(),
    approval_id: "approval-1",
    run_id: "run-414",
    issue_id: "issue-414",
    kind: "file_write",
    title: "Allow config update",
    description: "Agent wants to update local config file.",
    actor: { actor_id: "agent-1", actor_kind: "agent", display_name: "OpenHands Agent" },
    target_context: {
      file_path: "src/config.ts",
      command: "write_file",
      issue_id: "issue-414",
      issue_identifier: "COE-414",
      run_id: "run-414",
    },
    risk_summary: { level: "medium", reasons: ["modifies tracked config"] },
    requested_at: "2025-09-01T00:00:00Z",
    status: "pending",
    correlation_id: "corr-approval-1",
  },
];

function buildTransport(run: RunDetail): MockGatewayTransport {
  return new MockGatewayTransport({
    baseUri: "http://127.0.0.1:2468",
    health: capabilities,
    snapshot: dashboard,
    taskGraph,
    runDetails: [run],
    runFiles: [{ runId: run.run_id, files }],
    runDiffs: [{ runId: run.run_id, filePath: "src/config.ts", diff }],
    runApprovals: [{ runId: run.run_id, approvals }],
    runValidation: [{ runId: run.run_id, summary: validation }],
  });
}

function flushAsync(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

async function flushUntil(predicate: () => boolean, maxIterations = 40): Promise<void> {
  for (let i = 0; i < maxIterations; i++) {
    if (predicate()) return;
    await flushAsync();
  }
  throw new Error(`flushUntil timed out after ${maxIterations} iterations`);
}

async function openRun(root: HTMLElement): Promise<void> {
  await flushUntil(() => root.querySelector("[data-node-id='issue-1']") !== null);
  const node = root.querySelector("[data-node-id='issue-1']") as HTMLElement;
  node.click();
  await flushAsync();
  const openRun = root.querySelector("[data-open-run='issue-1']") as HTMLElement;
  openRun.click();
  await flushUntil(() => root.querySelector("[data-testid='run-action-bar']") !== null && root.querySelector(".os-run-head strong") !== null);
}

describe("Run detail views", () => {
  it("renders changed files, diff, validation, and approvals", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(runDetail),
    });

    await openRun(root);

    expect(root.querySelector("[data-testid='changed-file-list']")).not.toBeNull();
    const fileItems = root.querySelectorAll("[data-testid='changed-file-item']");
    expect(fileItems.length).toBe(files.length);
    expect(root.querySelector("[data-testid='file-diff']")).not.toBeNull();
    expect(root.querySelector("[data-testid='validation-summary']")).not.toBeNull();
    expect(root.querySelector("[data-testid='approval-list']")).not.toBeNull();
    expect(root.querySelector("[data-testid='run-action-bar']")).not.toBeNull();

    await handle.destroy();
  });

  it("selecting a changed file updates the diff panel", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(runDetail),
    });

    await openRun(root);
    const fileItem = root.querySelector("[data-path='src/new.ts']") as HTMLElement;
    expect(fileItem).not.toBeNull();
    fileItem.click();
    await flushAsync();

    // The selected file gets the os-selected class after the DOM re-renders.
    const selectedItem = root.querySelector("[data-path='src/new.ts']") as HTMLElement;
    expect(selectedItem.classList.contains("os-selected")).toBe(true);

    await handle.destroy();
  });

  it("approval card shows actor, target context, and risk summary", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(runDetail),
    });

    await openRun(root);
    const approval = root.querySelector("[data-testid='approval-item']") as HTMLElement;
    expect(approval).not.toBeNull();
    expect(approval.textContent).toContain("OpenHands Agent");
    expect(approval.textContent).toContain("src/config.ts");
    expect(approval.textContent).toContain("COE-414");
    expect(approval.textContent).toContain("medium");
    expect(root.querySelector("[data-testid='approve-button']")).not.toBeNull();
    expect(root.querySelector("[data-testid='deny-button']")).not.toBeNull();

    await handle.destroy();
  });

  it("clicking a run action dispatches and renders the receipt and audit trail", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(runDetail),
    });

    await openRun(root);
    const cancelButton = root.querySelector("[data-action='cancel']") as HTMLButtonElement;
    expect(cancelButton).not.toBeNull();
    expect(cancelButton.disabled).toBe(false);
    cancelButton.click();
    await flushAsync();

    expect(root.querySelector("[data-testid='action-receipt']")).not.toBeNull();
    expect(root.querySelector("[data-testid='audit-trail']")).not.toBeNull();
    const audit = root.querySelector("[data-testid='audit-trail-entry']") as HTMLElement;
    expect(audit).not.toBeNull();
    expect(audit.textContent).toContain("cancel");
    expect(audit.textContent).toContain("accepted");

    await handle.destroy();
  });

  it("submitting an approval decision updates the audit trail", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(runDetail),
    });

    await openRun(root);
    const explanation = root.querySelector("[data-testid='approval-explanation']") as HTMLInputElement;
    if (explanation) explanation.value = "approved in test";
    const approveButton = root.querySelector("[data-testid='approve-button']") as HTMLButtonElement;
    approveButton.click();
    await flushAsync();

    const audit = root.querySelector("[data-testid='audit-trail-entry']") as HTMLElement;
    expect(audit).not.toBeNull();
    expect(audit.textContent).toContain("approval_approved");
    expect(audit.textContent).toContain("approved in test");

    await handle.destroy();
  });
});

describe("Run action availability across states", () => {
  const cases: Array<{
    name: string;
    run: RunDetail;
    expectedDisabled: RunDetail["allowed_actions"];
    expectedWarnings: string[];
  }> = [
    {
      name: "active",
      run: { ...runDetail, liveness: { phase: "active", stream: "healthy" }, safe_actions: { retry: false, cancel: true, rehydrate: true, detach: false } },
      expectedDisabled: ["retry"],
      expectedWarnings: ["retry"],
    },
    {
      name: "quiet",
      run: { ...runDetail, liveness: { phase: "quiet", stream: "healthy" }, safe_actions: { retry: true, cancel: true, rehydrate: true, detach: false } },
      expectedDisabled: [],
      expectedWarnings: [],
    },
    {
      name: "degraded",
      run: { ...runDetail, liveness: { phase: "degraded", stream: "stale" }, safe_actions: { retry: false, cancel: true, rehydrate: true, detach: true } },
      expectedDisabled: ["retry"],
      expectedWarnings: ["retry"],
    },
    {
      name: "stalled",
      run: { ...runDetail, liveness: { phase: "stalled", stream: "dead" }, safe_actions: { retry: false, cancel: false, rehydrate: true, detach: true } },
      expectedDisabled: ["retry", "cancel"],
      expectedWarnings: ["retry", "cancel"],
    },
    {
      name: "detached",
      run: { ...runDetail, status: "released", release_reason: "cancelled", detached: true, liveness: { phase: "detached", stream: "dead" }, safe_actions: { retry: true, cancel: false, rehydrate: true, detach: false } },
      expectedDisabled: ["cancel"],
      expectedWarnings: ["cancel"],
    },
    {
      name: "cancel-failed",
      run: { ...runDetail, status: "released", release_reason: "cancel_failed", cancel_failed: true, liveness: { phase: "cancelled", stream: "dead" }, safe_actions: { retry: true, cancel: false, rehydrate: false, detach: true } },
      expectedDisabled: ["cancel", "rehydrate"],
      expectedWarnings: ["cancel", "rehydrate"],
    },
    {
      name: "terminal",
      run: { ...runDetail, status: "released", release_reason: "completed", liveness: { phase: "completed", stream: "healthy" }, safe_actions: { retry: false, cancel: false, rehydrate: true, detach: false } },
      expectedDisabled: ["retry", "cancel"],
      expectedWarnings: ["retry", "cancel"],
    },
  ];

  for (const { name, run, expectedDisabled, expectedWarnings } of cases) {
    it(`${name}: renders the expected action bar state`, async () => {
      const root = document.createElement("div");
      document.body.appendChild(root);
      const handle = renderOpenSymphonyApp({
        root,
        mode: "web",
        transport: buildTransport(run),
      });

      await openRun(root);
      const phase = root.querySelector(".os-run-grid strong")?.textContent;
      expect(phase).toBe(run.liveness?.phase);

      for (const action of expectedDisabled) {
        const button = root.querySelector(`[data-action='${action}']`) as HTMLButtonElement | null;
        expect(button?.disabled).toBe(true);
      }

      for (const action of expectedWarnings) {
        const warning = root.querySelector(`[data-action='${action}'] + .os-action-warning, [data-action='${action}'] ~ .os-action-warning`);
        expect(warning).not.toBeNull();
      }

      await handle.destroy();
    });
  }
});

describe("Detached and cancel-failed visibility", () => {
  it("shows detached and cancel-failed pills and diagnostics", async () => {
    const run: RunDetail = {
      ...runDetail,
      status: "released",
      release_reason: "cancel_failed",
      detached: true,
      cancel_failed: true,
      diagnostics: {
        cancel_failed: true,
        cancel_acknowledged: false,
      },
      liveness: { phase: "detached", stream: "dead" },
    };
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(run),
    });

    await openRun(root);
    expect(root.querySelector("[data-testid='run-pill-detached']")).not.toBeNull();
    expect(root.querySelector("[data-testid='run-pill-cancel-state']")).not.toBeNull();
    expect(root.querySelector("[data-testid='cancel-failed']")).not.toBeNull();

    await handle.destroy();
  });
});
