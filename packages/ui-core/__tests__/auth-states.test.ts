/**
 * @jest-environment jsdom
 *
 * Auth-aware shell placeholder states for COE-419.
 *
 * Verifies the shared app shell renders distinct unauthenticated,
 * unauthorized, and forbidden states for hosted gateways, while local
 * unauthenticated (`auth_modes:["none"]`) gateways render the dashboard
 * normally with no login gate.
 */

import { renderOpenSymphonyApp } from "../src/app-shell.js";
import { MockGatewayTransport } from "@opensymphony/api-client";
import { schemaVersionV1 } from "@opensymphony/gateway-schema";
import type {
  DashboardSnapshot,
  GatewayCapabilities,
  RunDetail,
  TaskGraphSnapshot,
} from "@opensymphony/gateway-schema";

const localCapabilities: GatewayCapabilities = {
  schema_version: schemaVersionV1(),
  gateway_version: "local-test",
  supported_api_versions: ["1.0.0"],
  transports: [{ transport: "loopback_http", modes: ["json"], supported_encodings: ["utf-8"], bidirectional: false }],
  features: [
    { feature: "task_graph", available: true, requires_auth: false },
    { feature: "terminal_stream", available: false, requires_auth: false },
  ],
  auth_modes: ["none"],
  max_event_page_size: 1000,
  max_terminal_frame_batch: 500,
};

const hostedCapabilities: GatewayCapabilities = {
  ...localCapabilities,
  gateway_version: "hosted-test",
  auth_modes: ["bearer_token"],
  features: [
    { feature: "task_graph", available: true, requires_auth: true },
    { feature: "terminal_stream", available: true, requires_auth: true },
  ],
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
    total_output_tokens: 20,
    total_cache_read_tokens: 0,
    total_cost_micros: 0,
  },
  projects: [
    { project_id: "proj-hosted", name: "Hosted Project", milestone_count: 1, issue_count: 2, running_count: 1, completed_count: 1, failed_count: 0 },
  ],
  recent_events: [],
};

const taskGraph: TaskGraphSnapshot = {
  schema_version: schemaVersionV1(),
  project_id: "proj-hosted",
  generated_at: "2025-09-01T00:00:00Z",
  root_ids: ["m-hosted"],
  nodes: [
    {
      schema_version: schemaVersionV1(),
      node_id: "m-hosted",
      kind: "milestone",
      identifier: "M-Hosted",
      title: "Hosted milestone",
      state: "In Progress",
      state_category: "in_progress",
      children: [],
      blocked_by: [],
      labels: ["hosted"],
    },
  ],
};

const runDetail: RunDetail = {
  schema_version: schemaVersionV1(),
  run_id: "run-hosted",
  issue_id: "issue-hosted",
  issue_identifier: "COE-600",
  worker_id: "worker-hosted",
  status: "running",
  claimed_at: "2025-09-01T00:00:00Z",
  started_at: "2025-09-01T00:00:30Z",
  turn_count: 1,
  max_turns: 8,
  input_tokens: 100,
  output_tokens: 20,
  cache_read_tokens: 0,
  runtime_seconds: 10,
  workspace_path: "/tmp/opensymphony/projects/COE-600",
  safe_actions: { retry: false, cancel: true, rehydrate: false, detach: false },
};

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

function mount(authFailure: {
  code: "unauthenticated" | "unauthorized" | "forbidden";
  methods?: Array<"health" | "snapshot">;
}) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const transport = new MockGatewayTransport({
    baseUri: "https://hosted.opensymphony.example",
    health: hostedCapabilities,
    snapshot: dashboard,
    taskGraph,
    runDetails: [runDetail],
    authFailure: { code: authFailure.code, methods: authFailure.methods ?? ["snapshot"] },
  });
  const handle = renderOpenSymphonyApp({
    root,
    mode: "web",
    transport,
  });
  return { root, handle, transport };
}

describe("Auth-aware shell placeholder states (COE-419)", () => {
  it("renders an unauthenticated sign-in placeholder when the hosted gateway rejects the snapshot with 401", async () => {
    const { root, handle } = mount({ code: "unauthenticated" });

    await flushUntil(() => root.querySelector("[data-testid='auth-placeholder']") !== null);

    const shell = root.querySelector("[data-opensymphony-app-shell='mounted']");
    expect(shell?.getAttribute("data-auth-state")).toBe("unauthenticated");

    const placeholder = root.querySelector("[data-testid='auth-placeholder']");
    expect(placeholder?.getAttribute("data-auth-state")).toBe("unauthenticated");
    expect(root.querySelector("[data-testid='auth-sign-in']")).not.toBeNull();
    expect(root.textContent).toContain("Sign in");
    expect(root.textContent).toContain("Sign in required");

    // The core dashboard, task graph, and run panels are not rendered behind
    // the auth gate.
    expect(root.querySelector(".os-task-graph-panel")).toBeNull();
    expect(root.querySelector(".os-run-detail-panel")).toBeNull();

    await handle.destroy();
    root.remove();
  });

  it("renders an unauthenticated placeholder when the gateway rejects capabilities (health) with 401", async () => {
    // Exercises the separate `loadGatewayState` code path where `health()`
    // itself rejects with an auth error (capabilities never resolve).
    const { root, handle } = mount({ code: "unauthenticated", methods: ["health"] });

    await flushUntil(() => root.querySelector("[data-testid='auth-placeholder']") !== null);

    const shell = root.querySelector("[data-opensymphony-app-shell='mounted']");
    expect(shell?.getAttribute("data-auth-state")).toBe("unauthenticated");
    const placeholder = root.querySelector("[data-testid='auth-placeholder']");
    expect(placeholder?.getAttribute("data-auth-state")).toBe("unauthenticated");
    expect(root.querySelector("[data-testid='auth-sign-in']")).not.toBeNull();
    expect(root.textContent).toContain("Sign in required");
    // Core panels are not rendered behind the auth gate.
    expect(root.querySelector(".os-task-graph-panel")).toBeNull();

    await handle.destroy();
    root.remove();
  });

  it("renders a forbidden placeholder when the hosted gateway denies the snapshot with 403 forbidden", async () => {
    const { root, handle } = mount({ code: "forbidden" });

    await flushUntil(() => root.querySelector("[data-testid='auth-placeholder']") !== null);

    expect(root.querySelector("[data-opensymphony-app-shell='mounted']")?.getAttribute("data-auth-state")).toBe("forbidden");
    const placeholder = root.querySelector("[data-testid='auth-placeholder']");
    expect(placeholder?.getAttribute("data-auth-state")).toBe("forbidden");
    expect(root.textContent).toContain("Access forbidden");
    // Forbidden has no sign-in CTA (caller is presumed authenticated).
    expect(root.querySelector("[data-testid='auth-sign-in']")).toBeNull();
    expect(root.querySelector("[data-testid='auth-refresh']")).not.toBeNull();
    // A hard 403 deny of the workspace does not offer org/project selectors.
    expect(root.querySelector("[data-testid='auth-scope']")).toBeNull();

    await handle.destroy();
    root.remove();
  });

  it("renders an unauthorized placeholder when the gateway returns an explicit unauthorized code", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    // The mock's authFailure throws a GatewayRequestError with the explicit
    // "unauthorized" code (a permission denial), which the UI maps to the
    // access-denied placeholder. This mirrors a real 403 whose body carries
    // an `error_code: "unauthorized"` permission signal.
    const transport = new MockGatewayTransport({
      baseUri: "https://hosted.opensymphony.example",
      health: hostedCapabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
      authFailure: { code: "unauthorized", methods: ["snapshot"] },
    });
    const handle = renderOpenSymphonyApp({ root, mode: "web", transport });

    await flushUntil(() => root.querySelector("[data-testid='auth-placeholder']") !== null);

    expect(root.querySelector("[data-opensymphony-app-shell='mounted']")?.getAttribute("data-auth-state")).toBe("unauthorized");
    expect(root.querySelector("[data-testid='auth-placeholder']")?.getAttribute("data-auth-state")).toBe("unauthorized");
    expect(root.textContent).toContain("Access denied");
    expect(root.textContent).toContain("do not have permission");
    // Unauthorized (permission denial) still offers org/project switching.
    expect(root.querySelector("[data-testid='auth-scope']")).not.toBeNull();

    await handle.destroy();
    root.remove();
  });

  it("renders the org/project selection placeholder surface for hosted auth states", async () => {
    const { root, handle } = mount({ code: "unauthenticated" });

    await flushUntil(() => root.querySelector("[data-testid='auth-scope']") !== null);

    expect(root.querySelector("[data-testid='auth-scope']")).not.toBeNull();
    expect(root.querySelector("[data-testid='auth-org']")).not.toBeNull();
    expect(root.querySelector("[data-testid='auth-project']")).not.toBeNull();

    await handle.destroy();
    root.remove();
  });

  it("keeps local unauthenticated (auth_modes:none) gateway straightforward with no login gate", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const transport = new MockGatewayTransport({
      baseUri: "http://127.0.0.1:2468",
      health: localCapabilities,
      snapshot: dashboard,
      taskGraph,
      runDetails: [runDetail],
    });
    const handle = renderOpenSymphonyApp({ root, mode: "web", transport });

    await flushUntil(() => root.querySelector("[data-opensymphony-app-shell='mounted']") !== null);
    await flushUntil(() => root.querySelector(".os-task-graph-panel") !== null);

    expect(root.querySelector("[data-opensymphony-app-shell='mounted']")?.getAttribute("data-auth-state")).toBe("open");
    expect(root.querySelector("[data-testid='auth-placeholder']")).toBeNull();
    expect(root.querySelector(".os-task-graph-panel")).not.toBeNull();
    expect(root.textContent).not.toContain("Sign in required");

    await handle.destroy();
    root.remove();
  });

  it("recovers from the unauthenticated placeholder when the gateway later permits the snapshot", async () => {
    const { root, handle, transport } = mount({ code: "unauthenticated" });

    await flushUntil(() => root.querySelector("[data-testid='auth-placeholder']") !== null);
    expect(root.querySelector("[data-opensymphony-app-shell='mounted']")?.getAttribute("data-auth-state")).toBe("unauthenticated");

    transport.clearAuthFailure();
    (root.querySelector("[data-testid='auth-refresh']") as HTMLButtonElement).click();

    await flushUntil(() => root.querySelector(".os-task-graph-panel") !== null);
    expect(root.querySelector("[data-testid='auth-placeholder']")).toBeNull();
    expect(root.querySelector("[data-opensymphony-app-shell='mounted']")?.getAttribute("data-auth-state")).toBe("open");

    await handle.destroy();
    root.remove();
  });
});