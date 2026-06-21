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
  ModelProfileController,
  ProfileController,
} from "../src/app-shell.js";
import type {
  ConnectionProfile,
  ChangedFileEntry,
  DashboardSnapshot,
  FileDiffPage,
  GatewayCapabilities,
  ModelConfigurationProfile,
  RunDetail,
  RunEventPage,
  TaskGraphSnapshot,
} from "@opensymphony/gateway-schema";
import { defaultModelProfiles } from "@opensymphony/gateway-schema";

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
      children: ["app-shell", "desktop-alpha", "hosted-auth", "follow-up", "skip-target"],
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
      node_id: "skip-target",
      kind: "issue",
      identifier: "COE-453",
      title: "Nested hosted follow-up",
      state: "Todo",
      state_category: "todo",
      parent_id: "m7-milestone",
      children: [],
      blocked_by: ["COE-449", "COE-450"],
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

const runEvents: RunEventPage = {
  schema_version: schemaVersionV1(),
  run_id: "COE-449",
  events: [
    {
      sequence: 1,
      event_id: "evt-action",
      happened_at: "2025-09-01T00:00:05Z",
      kind: "ActionEvent",
      summary: "action",
      payload: { tool_name: "terminal", command: "npm test -- apps/desktop" },
    },
    {
      sequence: 2,
      event_id: "evt-observation",
      happened_at: "2025-09-01T00:00:06Z",
      kind: "ObservationEvent",
      summary: "A long observation detail should receive the full activity card width.\nSecond line stays hidden until expanded.",
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
    runEvents: [
      runEvents,
      { ...runEvents, run_id: "desktop-alpha" },
    ],
  });
}

function buildModelProfileController(
  initial = defaultModelProfiles(),
): ModelProfileController & { saved: ModelConfigurationProfile[] } {
  const saved = initial.map((profile) => ({
    ...profile,
    harnesses: [...profile.harnesses],
  }));
  return {
    saved,
    async listProfiles() {
      return saved;
    },
    async storeProfile(profile) {
      const index = saved.findIndex((candidate) => candidate.id === profile.id);
      if (index >= 0) {
        saved[index] = profile;
      } else {
        saved.push(profile);
      }
      return profile;
    },
    async setActiveProfile(profileId) {
      const active = saved.find((profile) => profile.id === profileId);
      if (!active) {
        throw new Error(`Unknown model profile: ${profileId}`);
      }
      saved.forEach((profile) => {
        profile.active = profile.id === profileId;
      });
      return active;
    },
    async removeProfile(profileId) {
      const index = saved.findIndex((profile) => profile.id === profileId);
      if (index < 0) {
        throw new Error(`Unknown model profile: ${profileId}`);
      }
      if (saved.length <= 1) {
        throw new Error("Cannot remove the last model profile");
      }
      saved.splice(index, 1);
      if (!saved.some((profile) => profile.active) && saved[0]) {
        saved[0].active = true;
      }
      return saved;
    },
  };
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

async function expandSettingsPanel(
  root: HTMLElement,
  panel: "connection" | "model",
  readySelector: string,
): Promise<void> {
  const toggle = root.querySelector(
    `[data-toggle-settings='${panel}']`,
  ) as HTMLButtonElement;
  expect(toggle).not.toBeNull();
  toggle.click();
  await flushUntil(() => root.querySelector(readySelector) !== null);
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
    expect(root.querySelector(".os-task-graph-link-skip")).not.toBeNull();
    expect(root.querySelector(".os-task-graph-link-skip")?.getAttribute("d")).toMatch(/ H \d+ V \d+ H /);
    expect((root.querySelector("[data-node-id='app-shell']") as HTMLElement).style.getPropertyValue("--os-lane")).toBe("1");
    expect(root.querySelector("[data-node-id='desktop-alpha'] [data-testid='dependency-suffix']")?.textContent).toContain("blocks COE-450, COE-452");
    expect(root.querySelector("[data-node-id='app-shell'] [data-testid='dependency-suffix']")?.textContent).toContain("blocked by COE-449");
    expect(root.querySelector("[data-node-id='app-shell'] .os-badge-blocker")).toBeNull();
    expect(root.querySelector("[data-node-id='desktop-alpha'] .os-badge-blocker")).not.toBeNull();
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
    const activityEntries = Array.from(root.querySelectorAll("[data-testid='run-activity-entry']"));
    expect(activityEntries.map((entry) => entry.getAttribute("data-event-id"))).toEqual([
      "evt-observation",
      "evt-action",
    ]);
    expect(root.querySelector(".os-activity-entry-action-event .os-activity-preview")?.textContent).toBe("terminal: npm test -- apps/desktop");
    expect(root.querySelector(".os-activity-entry-action-event .os-activity-detail")).toBeNull();
    expect(root.querySelector(".os-activity-entry-observation-event .os-activity-meta strong")?.textContent).toBe("ObservationEvent");
    expect(root.querySelector(".os-activity-entry-observation-event .os-activity-preview")?.textContent).toContain("Second line stays hidden");
    expect(root.querySelector(".os-activity-entry-observation-event .os-activity-detail")).toBeNull();

    (root.querySelector(".os-activity-entry-observation-event [data-activity-toggle]") as HTMLButtonElement).click();
    await flushUntil(
      () => root.querySelector(".os-activity-entry-observation-event .os-activity-detail") !== null,
    );
    expect(root.querySelector(".os-activity-entry-observation-event [data-activity-toggle]")?.getAttribute("aria-expanded")).toBe("true");
    expect(root.querySelector(".os-activity-entry-observation-event .os-activity-detail")?.textContent).toContain("Second line stays hidden until expanded.");

    (root.querySelector("[data-testid='changed-file-item']") as HTMLButtonElement).click();
    await flushUntil(() => root.querySelector(".os-run-evidence-panel [data-testid='file-diff']") !== null);
    expect(root.querySelector("[data-evidence-view='diff']")?.classList.contains("is-selected")).toBe(true);

    await handle.destroy();
  });

  it("edits an API-compatible model profile and shows a redacted credential reference", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const profiles = defaultModelProfiles();
    const modelProfileController = buildModelProfileController(profiles);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
      initialModelProfiles: profiles,
    });

    await flushUntil(() => root.querySelector("[data-testid='model-profile-panel']") !== null);

    expect(root.querySelector(".os-model-panel h2")?.textContent).toBe("Model Configuration");
    const collapsedToggle = root.querySelector("[data-toggle-settings='model']") as HTMLButtonElement;
    expect(collapsedToggle).not.toBeNull();
    expect(collapsedToggle.classList.contains("os-activity-toggle")).toBe(true);
    expect(collapsedToggle.textContent?.trim()).toBe(">");
    expect(collapsedToggle.getAttribute("aria-expanded")).toBe("false");
    expect(collapsedToggle.textContent).not.toContain("Collapse");
    expect(collapsedToggle.textContent).not.toContain("Edit");
    expect(root.querySelector("[data-testid='model-redacted-credential']")?.textContent).toContain("Not configured");
    expect(root.querySelector("[data-model-credential-ref]")).toBeNull();
    await expandSettingsPanel(root, "model", "[data-model-credential-ref]");
    const expandedToggle = root.querySelector("[data-toggle-settings='model']") as HTMLButtonElement;
    expect(expandedToggle.textContent?.trim()).toBe("v");
    expect(expandedToggle.getAttribute("aria-expanded")).toBe("true");
    expect(expandedToggle.textContent).not.toContain("Collapse");
    expect(expandedToggle.textContent).not.toContain("Edit");
    expect((root.querySelector("[data-model-credential-ref]") as HTMLInputElement).type).toBe("password");
    expect(root.textContent).not.toContain("Cost Profile");
    expect(root.textContent).not.toContain("Context Window");
    expect(root.textContent).not.toContain("Recommended For");
    expect(root.textContent).not.toContain("Reasoning");
    expect(root.textContent).not.toContain("Subscription Provider");
    expect(root.textContent).not.toContain("Credential Storage");

    (root.querySelector("[data-model-name]") as HTMLInputElement).value = "provider/custom-model-name";
    (root.querySelector("[data-model-base-url]") as HTMLInputElement).value = "https://models.example.test/v1";
    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "local_keychain:custom-api-key";
    (root.querySelector("[data-model-harnesses]") as HTMLInputElement).value = "openhands_agent_server, custom_harness";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      modelProfileController.saved.some((profile) => profile.model === "provider/custom-model-name"),
    );

    const saved = modelProfileController.saved.find((profile) => profile.model === "provider/custom-model-name");
    expect(saved?.mode).toBe("api_key");
    expect(saved?.baseUrl).toBe("https://models.example.test/v1");
    expect(saved?.apiKeyRef).toBe("local_keychain:custom-api-key");
    expect(saved?.harnesses).toContain("custom_harness");
    await flushUntil(() =>
      root.querySelector("[data-testid='model-redacted-credential']")?.textContent?.includes("Configured") ?? false,
    );

    await handle.destroy();
  });

  it("rejects raw secrets and mismatched credential reference prefixes", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
    });

    await expandSettingsPanel(root, "model", "[data-model-credential-ref]");

    (root.querySelector("[data-model-name]") as HTMLInputElement).value = "provider/custom-model-name";
    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "sk-secret-value-123456789";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();
    await flushUntil(() => root.textContent?.includes("API key secret must use local_keychain:") ?? false);

    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "openhands_auth:openai";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();
    await flushUntil(() => root.textContent?.includes("API key secret must use local_keychain:") ?? false);

    expect(modelProfileController.saved.some((profile) => profile.model === "provider/custom-model-name")).toBe(false);

    await handle.destroy();
  });

  it("edits a subscription-backed model profile", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const profiles = defaultModelProfiles();
    profiles[1] = {
      ...profiles[1],
      subscriptionCredential: {
        ...profiles[1].subscriptionCredential!,
        authMethod: "browser",
        openBrowser: true,
        forceLogin: true,
        accountIdentityHeader: "X-OpenSymphony-Account",
      },
    };
    const modelProfileController = buildModelProfileController(profiles);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(),
      modelProfileController,
      initialModelProfiles: profiles,
    });

    await expandSettingsPanel(root, "model", "[data-model-profile-select]");

    (root.querySelector("[data-model-profile-select]") as HTMLSelectElement).value = "openai-subscription";
    (root.querySelector("[data-model-profile-select]") as HTMLSelectElement).dispatchEvent(
      new Event("change", { bubbles: true }),
    );
    await flushUntil(() => (root.querySelector("[data-model-mode]") as HTMLSelectElement).value === "subscription");
    expect((root.querySelector("[data-model-credential-ref]") as HTMLInputElement).type).toBe("text");

    (root.querySelector("[data-model-name]") as HTMLInputElement).value = "codex-subscription-preview";
    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "OPENHANDS_AUTH_DIR";
    (root.querySelector("[data-model-harnesses]") as HTMLInputElement).value = "openhands_agent_server, codex_app_server";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      modelProfileController.saved.some((profile) => profile.model === "codex-subscription-preview"),
    );

    const saved = modelProfileController.saved.find((profile) => profile.model === "codex-subscription-preview");
    expect(saved?.mode).toBe("subscription");
    expect(saved?.apiKeyRef).toBeNull();
    expect(saved?.subscriptionCredential?.authDirectoryEnv).toBe("OPENHANDS_AUTH_DIR");
    expect(saved?.subscriptionCredential?.provider).toBe("openai");
    expect(saved?.subscriptionCredential?.authMethod).toBe("browser");
    expect(saved?.subscriptionCredential?.openBrowser).toBe(true);
    expect(saved?.subscriptionCredential?.forceLogin).toBe(true);
    expect(saved?.subscriptionCredential?.accountIdentityHeader).toBe("X-OpenSymphony-Account");
    expect(saved?.credentialStorage).toBe("openhands_auth_directory");
    expect(saved?.harnesses).toEqual(["openhands_agent_server", "codex_app_server"]);

    await handle.destroy();
  });

  it("preserves API-key credential storage when editing a profile", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const profiles = defaultModelProfiles();
    profiles[0] = {
      ...profiles[0],
      credentialStorage: "openhands_auth_directory",
      apiKeyRef: "openhands_auth:openai-api-key",
    };
    const modelProfileController = buildModelProfileController(profiles);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
      initialModelProfiles: profiles,
    });

    await expandSettingsPanel(root, "model", "[data-model-credential-ref]");
    (root.querySelector("[data-model-name]") as HTMLInputElement).value = "provider/custom-model-name";
    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "openhands_auth:edited-api-key";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      modelProfileController.saved.some((profile) => profile.model === "provider/custom-model-name"),
    );

    const saved = modelProfileController.saved.find((profile) => profile.model === "provider/custom-model-name");
    expect(saved?.mode).toBe("api_key");
    expect(saved?.credentialStorage).toBe("openhands_auth_directory");
    expect(saved?.apiKeyRef).toBe("openhands_auth:edited-api-key");

    await handle.destroy();
  });

  it("normalizes an empty API-key secret reference to null", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const profiles = defaultModelProfiles();
    profiles[0] = {
      ...profiles[0],
      apiKeyRef: "local_keychain:openai-api-key",
    };
    const modelProfileController = buildModelProfileController(profiles);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
      initialModelProfiles: profiles,
    });

    await expandSettingsPanel(root, "model", "[data-model-credential-ref]");
    const profileId = (root.querySelector("[data-model-profile-select]") as HTMLSelectElement).value;
    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      modelProfileController.saved.some((profile) => profile.id === profileId && profile.apiKeyRef === null),
    );
    const saved = modelProfileController.saved.find((profile) => profile.id === profileId);
    expect(saved?.apiKeyRef).toBeNull();

    await handle.destroy();
  });

  it("preserves model profile order when editing", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
    });

    await expandSettingsPanel(root, "model", "[data-model-profile-select]");
    const before = Array.from(root.querySelectorAll<HTMLOptionElement>("[data-model-profile-select] option"))
      .map((option) => option.value);
    (root.querySelector("[data-model-name]") as HTMLInputElement).value = "provider/order-preserved";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      modelProfileController.saved.some((profile) => profile.model === "provider/order-preserved"),
    );
    const after = Array.from(root.querySelectorAll<HTMLOptionElement>("[data-model-profile-select] option"))
      .map((option) => option.value);
    expect(after).toEqual(before);

    await handle.destroy();
  });

  it("rerenders model credential fields when mode changes", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
    });

    await expandSettingsPanel(root, "model", "[data-model-mode]");
    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "local_keychain:openai-api-key";
    const modeSelect = root.querySelector("[data-model-mode]") as HTMLSelectElement;
    modeSelect.value = "subscription";
    modeSelect.dispatchEvent(new Event("change", { bubbles: true }));

    await flushUntil(() => (root.querySelector("[data-model-mode]") as HTMLSelectElement).value === "subscription");
    expect(root.textContent).toContain("OpenHands Auth Directory Env");
    const credentialInput = root.querySelector("[data-model-credential-ref]") as HTMLInputElement;
    expect(credentialInput.type).toBe("text");
    expect(credentialInput.value).toBe("");

    await handle.destroy();
  });

  it("creates and removes model profiles from the panel", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const confirmSpy = jest.spyOn(window, "confirm")
      .mockReturnValueOnce(false)
      .mockReturnValueOnce(true);
    const modelProfileController = buildModelProfileController();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
    });

    await expandSettingsPanel(root, "model", "[data-new-model-profile]");
    const startingCount = root.querySelectorAll("[data-model-profile-select] option").length;
    (root.querySelector("[data-new-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      root.querySelectorAll("[data-model-profile-select] option").length === startingCount + 1,
    );
    const createdId = (root.querySelector("[data-model-profile-select]") as HTMLSelectElement).value;
    expect(modelProfileController.saved.some((profile) => profile.id === createdId)).toBe(true);

    (root.querySelector("[data-remove-model-profile]") as HTMLButtonElement).click();
    await flushAsync();
    expect(modelProfileController.saved.some((profile) => profile.id === createdId)).toBe(true);

    (root.querySelector("[data-remove-model-profile]") as HTMLButtonElement).click();
    await flushUntil(() =>
      !modelProfileController.saved.some((profile) => profile.id === createdId),
    );
    expect(root.querySelectorAll("[data-model-profile-select] option")).toHaveLength(startingCount);
    expect(confirmSpy).toHaveBeenCalledTimes(2);

    confirmSpy.mockRestore();
    await handle.destroy();
  });

  it("deactivates the active model profile with the explicit Active checkbox", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
    });

    await expandSettingsPanel(root, "model", "[data-model-active]");
    expect((root.querySelector("[data-model-active]") as HTMLInputElement).checked).toBe(true);

    (root.querySelector("[data-model-active]") as HTMLInputElement).checked = false;
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      modelProfileController.saved.find((profile) => profile.id === "openai-api-compatible")?.active === false,
    );
    expect((root.querySelector("[data-model-active]") as HTMLInputElement).checked).toBe(false);

    await handle.destroy();
  });

  it("keeps model profile save failures separate from gateway connection health", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController: {
        ...modelProfileController,
        async storeProfile() {
          throw new Error("secure settings unavailable");
        },
      },
    });

    await expandSettingsPanel(root, "model", "[data-model-credential-ref]");
    await flushUntil(() => root.querySelector(".os-status-connected") !== null);

    (root.querySelector("[data-model-name]") as HTMLInputElement).value = "provider/custom-model-name";
    (root.querySelector("[data-model-credential-ref]") as HTMLInputElement).value = "local_keychain:custom-api-key";
    (root.querySelector("[data-save-model-profile]") as HTMLButtonElement).click();

    await flushUntil(() =>
      root.querySelector("[data-testid='model-profile-error']")?.textContent?.includes("Model profile save failed: secure settings unavailable") ?? false,
    );
    expect(root.querySelector(".os-topbar p")?.textContent).not.toContain("Model profile save failed");
    expect(root.querySelector(".os-status-connected")).not.toBeNull();
    expect(root.querySelector(".os-status-failed")).toBeNull();

    await handle.destroy();
  });

  it("keeps model profile load failures separate from gateway connection health", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController: {
        ...modelProfileController,
        async listProfiles() {
          throw new Error("settings store unavailable");
        },
      },
    });

    await flushUntil(() =>
      root.querySelector("[data-testid='model-profile-error']")?.textContent?.includes("Model profiles unavailable: settings store unavailable") ?? false,
    );
    await flushUntil(() => root.querySelector(".os-status-connected") !== null);
    expect(root.querySelector(".os-topbar p")?.textContent).not.toContain("Model profiles unavailable");
    expect(root.querySelector(".os-status-failed")).toBeNull();

    await handle.destroy();
  });

  it("reports session-only model profile persistence in the panel", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    modelProfileController.persistence = {
      kind: "session",
      label: "Model profiles are session-only because host storage is unavailable.",
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "web",
      transport: buildTransport(),
      modelProfileController,
    });

    await flushUntil(() => root.querySelector("[data-testid='model-persistence-status']") !== null);

    expect(root.querySelector("[data-testid='model-persistence-status']")?.textContent).toContain("session-only");
    expect(root.querySelector(".os-model-persistence-session")).not.toBeNull();

    await handle.destroy();
  });

  it("surfaces model profile quarantine warnings in the panel", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    modelProfileController.quarantineMessages = [
      "Dropped invalid model profile raw-secret: API key secret must use local_keychain:<name>",
    ];
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
    });

    await flushUntil(() =>
      root.querySelector("[data-testid='model-profile-error']")?.textContent?.includes("Dropped invalid model profile raw-secret") ?? false,
    );

    expect(root.querySelector("[data-testid='model-profile-error']")?.textContent).toContain("Model profile storage warning");

    await handle.destroy();
  });

  it("uses the model profile controller warning drain when available", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const modelProfileController = buildModelProfileController();
    modelProfileController.quarantineMessages = ["stale warning"];
    modelProfileController.takeQuarantineMessages = jest.fn(() => [
      "Dropped model profile with missing id",
    ]);
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      modelProfileController,
    });

    await flushUntil(() =>
      root.querySelector("[data-testid='model-profile-error']")?.textContent?.includes("missing id") ?? false,
    );

    expect(modelProfileController.takeQuarantineMessages).toHaveBeenCalled();
    expect(modelProfileController.quarantineMessages).toEqual(["stale warning"]);

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

    await expandSettingsPanel(root, "connection", "[data-save-profile]");
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
      async removeProfile(): Promise<ConnectionProfile[]> {
        return [];
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

    await expandSettingsPanel(root, "connection", "[data-save-profile]");

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

  it("creates and deletes connection profiles from the panel", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const confirmSpy = jest.spyOn(window, "confirm")
      .mockReturnValueOnce(false)
      .mockReturnValueOnce(true);

    const profiles: ConnectionProfile[] = [
      {
        id: "local",
        label: "Local",
        kind: "local_daemon",
        active: true,
        gatewayUrl: "http://127.0.0.1:2468",
        transport: "loopback_http",
        managed: false,
      },
    ];
    const controller: ProfileController = {
      async listProfiles() {
        return profiles;
      },
      async storeProfile(profile) {
        const saved: ConnectionProfile = {
          id: profile.id ?? "created",
          label: profile.label,
          kind: profile.kind,
          active: true,
          gatewayUrl: profile.gatewayUrl,
          transport: "loopback_http",
          managed: false,
        };
        const index = profiles.findIndex((candidate) => candidate.id === saved.id);
        if (index >= 0) {
          profiles[index] = saved;
        } else {
          profiles.push(saved);
        }
        return saved;
      },
      async setActiveProfile(profileId) {
        const active = profiles.find((profile) => profile.id === profileId);
        if (!active) {
          throw new Error(`Unknown profile: ${profileId}`);
        }
        profiles.forEach((profile) => {
          profile.active = profile.id === profileId;
        });
        return active;
      },
      async removeProfile(profileId) {
        const index = profiles.findIndex((profile) => profile.id === profileId);
        if (index < 0) {
          throw new Error(`Unknown profile: ${profileId}`);
        }
        profiles.splice(index, 1);
        if (profiles[0]) {
          profiles[0].active = true;
        }
        return profiles;
      },
    };
    const handle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      transport: buildTransport(),
      profileController: controller,
    });

    await expandSettingsPanel(root, "connection", "[data-new-profile]");
    (root.querySelector("[data-new-profile]") as HTMLButtonElement).click();
    await flushUntil(() =>
      Array.from(root.querySelectorAll("[data-profile-select] option")).some((option) =>
        option.getAttribute("value") === "created"
      ),
    );
    expect(profiles.some((profile) => profile.id === "created")).toBe(true);

    (root.querySelector("[data-remove-profile]") as HTMLButtonElement).click();
    await flushAsync();
    expect(profiles.some((profile) => profile.id === "created")).toBe(true);

    (root.querySelector("[data-remove-profile]") as HTMLButtonElement).click();
    await flushUntil(() => !profiles.some((profile) => profile.id === "created"));
    expect(root.querySelectorAll("[data-profile-select] option")).toHaveLength(1);
    expect(confirmSpy).toHaveBeenCalledTimes(2);

    confirmSpy.mockRestore();
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
    expect(root.querySelector("[data-profile-select]")).toBeNull();
    const collapsedToggle = root.querySelector("[data-toggle-settings='connection']") as HTMLButtonElement;
    expect(collapsedToggle).not.toBeNull();
    expect(collapsedToggle.classList.contains("os-activity-toggle")).toBe(true);
    expect(collapsedToggle.textContent?.trim()).toBe(">");
    expect(collapsedToggle.getAttribute("aria-expanded")).toBe("false");
    expect(collapsedToggle.textContent).not.toContain("Collapse");
    expect(collapsedToggle.textContent).not.toContain("Edit");
    await expandSettingsPanel(root, "connection", "[data-profile-select]");
    const expandedToggle = root.querySelector("[data-toggle-settings='connection']") as HTMLButtonElement;
    expect(expandedToggle.textContent?.trim()).toBe("v");
    expect(expandedToggle.getAttribute("aria-expanded")).toBe("true");
    expect(expandedToggle.textContent).not.toContain("Collapse");
    expect(expandedToggle.textContent).not.toContain("Edit");
    const select = root.querySelector(
      "[data-profile-select]",
    ) as HTMLSelectElement;
    expect(select).not.toBeNull();
    // Without a profile controller the shell uses the default UI profile.
    expect(select.options.length).toBeGreaterThan(0);
    await handle.destroy();
  });
});
