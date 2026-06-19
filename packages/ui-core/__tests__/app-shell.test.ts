/**
 * @jest-environment jsdom
 *
 * App-shell mount smoke tests for COE-449 desktop alpha recovery.
 */

import { renderOpenSymphonyApp } from "../src/app-shell.js";
import { MockGatewayTransport } from "@opensymphony/api-client";
import { schemaVersionV1 } from "@opensymphony/gateway-schema";
import type {
  EditableProfileInput,
  ProfileController,
} from "../src/app-shell.js";
import type {
  ConnectionProfile,
  ChangedFileEntry,
  DashboardSnapshot,
  FileDiffPage,
  GatewayCapabilities,
  RunDetail,
  TaskGraphSnapshot,
} from "@opensymphony/gateway-schema";

const capabilities: GatewayCapabilities = {
  schema_version: schemaVersionV1(),
  gateway_version: "alpha-test",
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
  sequence: 7,
  health: "healthy",
  metrics: {
    running_issue_count: 2,
    retry_queue_depth: 1,
    total_input_tokens: 4500,
    total_output_tokens: 1100,
    total_cache_read_tokens: 600,
    total_cost_micros: 250,
  },
  projects: [
    {
      project_id: "proj-alpha",
      name: "Alpha Recovery",
      milestone_count: 2,
      issue_count: 4,
      running_count: 1,
      completed_count: 2,
      failed_count: 1,
    },
  ],
  recent_events: [
    {
      happened_at: "2025-09-01T00:00:00Z",
      kind: "client_attached",
      issue_identifier: "COE-449",
      summary: "App shell mounted under test",
    },
    {
      happened_at: "2025-09-01T00:00:01Z",
      kind: "snapshot_published",
      issue_identifier: "COE-450",
      summary: "published dependency-aware snapshot",
    },
    {
      happened_at: "2025-09-01T00:00:02Z",
      kind: "run_event",
      issue_identifier: "COE-451",
      summary: "captured runtime event",
    },
    {
      happened_at: "2025-09-01T00:00:03Z",
      kind: "hidden_event",
      issue_identifier: "COE-452",
      summary: "should not render in compact status",
    },
  ],
};

const taskGraph: TaskGraphSnapshot = {
  schema_version: schemaVersionV1(),
  project_id: "proj-alpha",
  generated_at: "2025-09-01T00:00:00Z",
  root_ids: ["m7-milestone"],
  nodes: [
    {
      schema_version: schemaVersionV1(),
      node_id: "m7-milestone",
      kind: "milestone",
      identifier: "M7",
      title: "Shared Client and Desktop Alpha",
      state: "In Progress",
      state_category: "in_progress",
      children: ["app-shell", "desktop-alpha", "hosted-auth", "follow-up"],
      blocked_by: [],
      labels: ["desktop"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "app-shell",
      kind: "issue",
      identifier: "COE-450",
      title: "Desktop follow-on review",
      state: "Todo",
      state_category: "todo",
      parent_id: "m7-milestone",
      children: [],
      blocked_by: ["COE-449"],
      labels: ["desktop", "recovery"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "desktop-alpha",
      kind: "issue",
      identifier: "COE-449",
      title: "Replace stubs with functional app",
      state: "In Progress",
      state_category: "in_progress",
      parent_id: "m7-milestone",
      children: [],
      blocked_by: [],
      labels: ["transport"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "hosted-auth",
      kind: "issue",
      identifier: "COE-452",
      title: "Hosted auth placeholders",
      state: "Todo",
      state_category: "todo",
      parent_id: "m7-milestone",
      children: [],
      blocked_by: ["COE-449"],
      labels: ["hosted"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "follow-up",
      kind: "issue",
      identifier: "COE-451",
      title: "Released prerequisite detail",
      state: "Todo",
      state_category: "todo",
      parent_id: "m7-milestone",
      children: [],
      blocked_by: ["completed-prereq"],
      labels: ["transport"],
    },
    {
      schema_version: schemaVersionV1(),
      node_id: "completed-prereq",
      kind: "issue",
      identifier: "COE-448",
      title: "Completed prerequisite",
      state: "Done",
      state_category: "done",
      parent_id: "m7-milestone",
      children: [],
      blocked_by: [],
      labels: ["transport"],
    },
  ],
};

const runDetail: RunDetail = {
  schema_version: schemaVersionV1(),
  run_id: "COE-449",
  issue_id: "issue-coe-449",
  issue_identifier: "COE-449",
  worker_id: "worker-alpha",
  status: "running",
  claimed_at: "2025-09-01T00:00:00Z",
  started_at: "2025-09-01T00:00:30Z",
  turn_count: 3,
  max_turns: 8,
  input_tokens: 4500,
  output_tokens: 1100,
  cache_read_tokens: 600,
  runtime_seconds: 90,
  workspace_path: "/tmp/opensymphony/projects/COE-449",
  safe_actions: {
    retry: false,
    cancel: true,
    rehydrate: true,
    detach: false,
  },
};

const changedFiles: ChangedFileEntry[] = [
  {
    path: "src/config.ts",
    change_kind: "modified",
    lines_added: 12,
    lines_removed: 3,
  },
];

const fileDiff: FileDiffPage = {
  schema_version: schemaVersionV1(),
  run_id: "COE-449",
  file_path: "src/config.ts",
  hunks: [
    {
      file_path: "src/config.ts",
      header: "@@ -1 +1 @@",
      start_line: 1,
      old_line_count: 1,
      new_line_count: 1,
      lines: [{ type: "addition", line: "export const gateway = true;" }],
    },
  ],
  total_lines_added: 12,
  total_lines_removed: 3,
};

function buildTransport(opts?: { failHealth?: boolean; failTaskGraphStructured?: boolean }): MockGatewayTransport {
  if (opts?.failHealth) {
    class AlwaysFailHealthTransport extends MockGatewayTransport {
      override async health(): Promise<never> {
        throw new Error("simulated health probe failure");
      }
    }
    return new AlwaysFailHealthTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
    });
  }
  if (opts?.failTaskGraphStructured) {
    class StructuredTaskGraphFailureTransport extends MockGatewayTransport {
      override async taskGraph(): Promise<never> {
        throw { type: "Gateway", message: "simulated structured task graph failure" };
      }
    }
    return new StructuredTaskGraphFailureTransport({
      baseUri: "http://127.0.0.1:2468",
      health: capabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
    });
  }
  return new MockGatewayTransport({
    baseUri: "http://127.0.0.1:2468",
    health: capabilities,
    snapshot: dashboard,
    taskGraph,
    // Map the desktop-alpha task graph node to the COE-449 run detail so
    // the actual mock gateway response drives the run detail panel.
    runDetails: [
      runDetail,
      { ...runDetail, run_id: "desktop-alpha", issue_id: "desktop-alpha" },
    ],
    runFiles: [
      { runId: "COE-449", files: changedFiles },
      { runId: "desktop-alpha", files: changedFiles },
    ],
    runDiffs: [
      { runId: "COE-449", filePath: "src/config.ts", diff: fileDiff },
      { runId: "desktop-alpha", filePath: "src/config.ts", diff: { ...fileDiff, run_id: "desktop-alpha" } },
    ],
  });
}

function flushAsync(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

async function flushUntil(
  predicate: () => boolean,
  maxIterations = 40,
): Promise<void> {
  for (let i = 0; i < maxIterations; i++) {
    if (predicate()) return;
    await flushAsync();
  }
  throw new Error(
    `flushUntil timed out after ${maxIterations} iterations waiting for predicate`,
  );
}

describe("OpenSymphonyApp mount", () => {
  it("flushUntil rejects with a clear timeout message instead of returning silently", async () => {
    // Regression coverage for the reviewer finding that exhausted
    // flushUntil iterations used to resolve silently, which masked the
    // real failure inside a later null assertion.
    await expect(flushUntil(() => false, 4)).rejects.toThrow(
      /timed out after 4 iterations/,
    );
  });

  it("mounts the shared app shell with the marker attribute", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      title: "OpenSymphony Desktop",
      transport: buildTransport(),
    });
    await flushUntil(
      () =>
        root.querySelector(".os-app[data-opensymphony-app-shell='mounted']") !==
        null,
    );

    expect(
      root.querySelector(".os-app[data-opensymphony-app-shell='mounted']"),
    ).not.toBeNull();
    expect(root.querySelector(".os-app[data-mode='desktop']")).not.toBeNull();
    expect(root.textContent).toContain("OpenSymphony Desktop");

    await handle.destroy();
    expect(root.children.length).toBe(0);
  });

  it("keeps dark-mode tabs and changed-file rows readable", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      title: "OpenSymphony Desktop",
      transport: buildTransport(),
    });

    await flushUntil(
      () =>
        root.querySelector(".os-app[data-opensymphony-app-shell='mounted']") !==
        null,
    );

    const styleText = root.querySelector("style")?.textContent ?? "";
    expect(styleText).toContain("@media (prefers-color-scheme: dark)");
    expect(styleText).toContain(
      ".os-view-tab, .os-plan-tab, .os-changed-file",
    );
    expect(styleText).toContain(".os-changed-file .os-file-path");
    expect(styleText).toContain(".os-changed-file .os-file-stats");

    await handle.destroy();
  });

  it("lays out status, task graph, run detail, and activity panels", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await flushUntil(
      () => root.querySelector("[data-node-id='desktop-alpha']") !== null,
    );

    expect(root.querySelector(".os-status-panel h2")?.textContent).toBe("Status");
    expect(root.querySelector(".os-profile-panel h2")?.textContent).toBe("Connection");
    expect(root.querySelector(".os-task-graph-panel h2")?.textContent).toBe("Task Graph");
    expect(root.querySelector(".os-run-detail-panel h2")?.textContent).toBe("Run Detail");
    expect(root.querySelector(".os-run-evidence-panel h2")?.textContent).toBe("Inspector");
    expect(root.querySelector("[data-profile-label]")).toBeNull();
    expect(root.querySelector(".os-metrics")).not.toBeNull();
    expect(root.querySelector("[data-project-id='proj-alpha']")).toBeNull();
    expect(root.querySelectorAll(".os-events li")).toHaveLength(3);
    expect(root.querySelector(".os-event-time")).not.toBeNull();
    expect(root.textContent).not.toContain("should not render in compact status");
    expect(root.textContent).not.toContain("1 running, 2 done, 1 failed");
    expect(root.querySelector("[data-tg-create='milestone']")).toBeNull();
    expect(root.querySelector("[data-tg-create='issue']")).toBeNull();
    expect(root.querySelector("[data-tg-edit]")).toBeNull();
    expect(root.querySelector("[data-tg-deps]")).toBeNull();
    expect(root.querySelector("[data-tg-comment]")).toBeNull();
    expect(root.querySelector("[data-tg-create-child]")).toBeNull();
    expect(root.querySelector("[data-testid='task-graph-visualization']")).not.toBeNull();
    expect(root.querySelector("[data-testid='task-graph-link']")).not.toBeNull();
    expect((root.querySelector("[data-node-id='app-shell']") as HTMLElement).style.getPropertyValue("--os-lane")).toBe("1");
    expect(root.querySelector("[data-node-id='desktop-alpha'] [data-testid='dependency-suffix']")?.textContent).toContain("blocks COE-450, COE-452");
    expect(root.querySelector("[data-node-id='app-shell'] [data-testid='dependency-suffix']")?.textContent).toContain("blocked by COE-449");
    expect(root.textContent).not.toContain("blocked by COE-448");
    await flushUntil(() => root.querySelector(".os-run-head strong")?.textContent === "COE-449");

    taskGraph.root_ids.forEach((rootId) => {
      expect(root.querySelector(`[data-node-id='${rootId}']`)).not.toBeNull();
    });

    await flushUntil(() => root.querySelector(".os-run-grid") !== null);
    await flushUntil(() => root.querySelector("[data-testid='changed-file-item']") !== null);

    const runSection = root.querySelector(".os-run-grid");
    expect(runSection).not.toBeNull();
    // The issue identifier is rendered in the .os-run-head strip, not
    // inside the .os-run-grid metrics block. Verify the run detail
    // panel reflects the navigation event with the mock gateway response.
    expect(root.querySelector(".os-run-head strong")?.textContent).toBe("COE-449");
    expect(root.querySelector(".os-pill")?.textContent).toBe("running");
    expect(root.querySelector("[data-testid='dependency-detail']")?.textContent).toContain("blocks COE-450, COE-452");
    expect(root.querySelector(".os-run-detail-panel [data-testid='changed-file-list']")).not.toBeNull();
    expect(root.querySelector(".os-run-evidence-panel [data-testid='evidence-toggle']")).not.toBeNull();
    expect(root.querySelector(".os-run-evidence-panel [data-testid='file-diff']")).not.toBeNull();

    (root.querySelector("[data-evidence-view='activity']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector(".os-run-evidence-panel [data-testid='run-activity']") !== null);
    expect(root.querySelector(".os-run-evidence-panel [data-testid='run-activity']")).not.toBeNull();

    (root.querySelector("[data-testid='changed-file-item']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector(".os-run-evidence-panel [data-testid='file-diff']") !== null);
    expect(root.querySelector("[data-evidence-view='diff']")?.classList.contains("is-selected")).toBe(true);

    await handle.destroy();
  });

  it("reports a failed connection instead of falling back to fixture data", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({ failHealth: true }),
    });

    await flushUntil(
      () =>
        root.querySelector("[data-opensymphony-app-shell='mounted']") !== null,
    );

    await flushUntil(() => root.querySelector(".os-status-failed") !== null);

    expect(root.querySelector(".os-status-failed")).not.toBeNull();
    expect(root.textContent).toContain("Failed");
    expect(root.textContent).toContain("Gateway unavailable");
    expect(root.textContent).not.toContain("desktop-alpha");

    await handle.destroy();
  });

  it("renders structured native errors with their message", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport({ failTaskGraphStructured: true }),
    });

    await flushUntil(() => root.textContent?.includes("simulated structured task graph failure") ?? false);

    expect(root.textContent).toContain("Task graph unavailable: simulated structured task graph failure");
    expect(root.textContent).not.toContain("[object Object]");

    await handle.destroy();
  });

  it("disables profile save when no profile controller is provided", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector("[data-save-profile]") !== null);
    const save = root.querySelector("[data-save-profile]") as HTMLButtonElement;
    expect(save).not.toBeNull();
    expect(save.disabled).toBe(true);

    await handle.destroy();
  });

  it("routes a saved profile through ProfileController and refreshes the active gateway URL", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);

    const newUrl = "http://127.0.0.1:9001";
    let lastConnect: string | null = null;
    const controller: ProfileController = {
      async listProfiles(): Promise<ConnectionProfile[]> {
        return [];
      },
      async storeProfile(profile: EditableProfileInput): Promise<ConnectionProfile> {
        return {
          id: "profile-saved",
          label: profile.label,
          kind: profile.kind,
          active: true,
          gatewayUrl: profile.gatewayUrl,
          transport: "loopback_http",
          managed: false,
        };
      },
      async setActiveProfile(profileId: string): Promise<ConnectionProfile> {
        return {
          id: profileId,
          label: "Saved profile",
          kind: "external_gateway",
          active: true,
          gatewayUrl: newUrl,
          transport: "loopback_http",
          managed: false,
        };
      },
    };

    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      profileController: controller,
      onGatewayUrlChanged: async (url) => {
        lastConnect = url;
        return buildTransport();
      },
    });

    await flushUntil(() => root.querySelector("[data-save-profile]") !== null);

    const gatewayInput = root.querySelector(
      "[data-profile-gateway]",
    ) as HTMLInputElement;
    const save = root.querySelector("[data-save-profile]") as HTMLButtonElement;

    gatewayInput.value = newUrl;
    save.click();

    await flushUntil(() => lastConnect === newUrl);
    expect(lastConnect).toBe(newUrl);
    expect(save.disabled).toBe(false);

    await handle.destroy();
  });

  it("renders the profile panel and provided initial profile when no controller is set", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
    });

    await flushUntil(() => root.querySelector(".os-profile-panel") !== null);
    const select = root.querySelector(
      "[data-profile-select]",
    ) as HTMLSelectElement;
    expect(select).not.toBeNull();
    // Without a profile controller the shell uses the default UI profile.
    expect(select.options.length).toBeGreaterThan(0);
    await handle.destroy();
  });
});
