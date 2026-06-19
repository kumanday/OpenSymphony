/**
 * @jest-environment jsdom
 *
 * Hosted sign-in form flow (COE-420).
 *
 * Verifies the web client shell, when wired with an `AppAuthIntegration`,
 * renders an email/password sign-in form on the unauthenticated placeholder,
 * authenticates through the integration on submit, rebuilds the transport with
 * the resolved session token, and renders the authenticated dashboard. A
 * failed sign-in surfaces an inline error and keeps the placeholder. A signed-
 * in shell offers a sign-out control that clears the session.
 */

import { renderOpenSymphonyApp } from "../src/app-shell.js";
import type {
  AppAuthIntegration,
  AppLoginCredentials,
  GatewayReader,
} from "../src/app-shell.js";
import { MockGatewayTransport } from "@opensymphony/api-client";
import { schemaVersionV1 } from "@opensymphony/gateway-schema";
import type {
  DashboardSnapshot,
  GatewayCapabilities,
  RunDetail,
  TaskGraphSnapshot,
} from "@opensymphony/gateway-schema";

const hostedCapabilities: GatewayCapabilities = {
  schema_version: schemaVersionV1(),
  gateway_version: "hosted-test",
  supported_api_versions: ["1.0.0"],
  transports: [{ transport: "loopback_http", modes: ["json"], supported_encodings: ["utf-8"], bidirectional: false }],
  features: [
    { feature: "task_graph", available: true, requires_auth: true },
    { feature: "terminal_stream", available: true, requires_auth: true },
  ],
  auth_modes: ["bearer_token"],
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

async function flushUntil(predicate: () => boolean, maxIterations = 60): Promise<void> {
  for (let i = 0; i < maxIterations; i++) {
    if (predicate()) return;
    await flushAsync();
  }
  throw new Error(`flushUntil timed out after ${maxIterations} iterations`);
}

/**
 * A test `AppAuthIntegration` that records calls and can be programmed to
 * succeed/fail. Mirrors how `apps/web/src/main.ts` wires `HostedAuthClient`.
 */
class FakeAuthIntegration implements AppAuthIntegration {
  loginCalls: AppLoginCredentials[] = [];
  logoutCalls = 0;
  applySessionCalls: (string | undefined)[] = [];
  failWith: Error | null = null;

  async login(credentials: AppLoginCredentials): Promise<string> {
    this.loginCalls.push(credentials);
    if (this.failWith) throw this.failWith;
    return "sess-xyz";
  }
  async logout(): Promise<void> {
    this.logoutCalls += 1;
  }
  async applySession(token: string | undefined): Promise<GatewayReader> {
    this.applySessionCalls.push(token);
    // Return a fresh authenticated transport so the snapshot now succeeds.
    this.authenticatedTransport.clearAuthFailure();
    return this.authenticatedTransport as unknown as GatewayReader;
  }
  authenticatedTransport: MockGatewayTransport;

  constructor(authenticatedTransport: MockGatewayTransport) {
    this.authenticatedTransport = authenticatedTransport;
  }
}

function mountWithAuth(integration: FakeAuthIntegration, unauthenticatedTransport: MockGatewayTransport) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const handle = renderOpenSymphonyApp({
    root,
    mode: "web",
    transport: unauthenticatedTransport,
    authIntegration: integration,
  });
  return { root, handle };
}

function makeUnauthenticatedTransport(): MockGatewayTransport {
  return new MockGatewayTransport({
    baseUri: "https://hosted.opensymphony.example",
    health: hostedCapabilities,
    snapshot: dashboard,
    taskGraph,
    runDetails: [runDetail],
    authFailure: { code: "unauthenticated", methods: ["snapshot"] },
  });
}

function makeAuthenticatedTransport(): MockGatewayTransport {
  return new MockGatewayTransport({
    baseUri: "https://hosted.opensymphony.example",
    health: hostedCapabilities,
    snapshot: dashboard,
    taskGraph,
    runDetails: [runDetail],
  });
}

describe("Hosted sign-in form flow (COE-420)", () => {
  it("renders an email/password sign-in form when an authIntegration is wired", async () => {
    const authenticated = makeAuthenticatedTransport();
    const integration = new FakeAuthIntegration(authenticated);
    const { root, handle } = mountWithAuth(integration, makeUnauthenticatedTransport());

    await flushUntil(() => root.querySelector("[data-testid='auth-form']") !== null);

    expect(root.querySelector("[data-testid='auth-form']")).not.toBeNull();
    expect(root.querySelector("[data-testid='auth-email']")).not.toBeNull();
    expect(root.querySelector("[data-testid='auth-password']")).not.toBeNull();
    expect(root.querySelector("[data-testid='auth-org-slug']")).not.toBeNull();
    expect(root.querySelector("[data-testid='auth-sign-in']")?.textContent).toContain("Sign in");

    await handle.destroy();
    root.remove();
  });

  it("authenticates on submit, applies the session token, and renders the dashboard", async () => {
    const authenticated = makeAuthenticatedTransport();
    const integration = new FakeAuthIntegration(authenticated);
    const { root, handle } = mountWithAuth(integration, makeUnauthenticatedTransport());

    await flushUntil(() => root.querySelector("[data-testid='auth-form']") !== null);
    expect(root.querySelector(".os-task-graph-panel")).toBeNull();

    // Fill and submit the sign-in form.
    (root.querySelector("[data-testid='auth-email']") as HTMLInputElement).value = "admin@example.com";
    (root.querySelector("[data-testid='auth-password']") as HTMLInputElement).value = "pw-admin";
    (root.querySelector("[data-testid='auth-org-slug']") as HTMLInputElement).value = "acme";
    (root.querySelector("[data-testid='auth-form']") as HTMLFormElement).dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );

    // The integration was called with the submitted credentials.
    await flushUntil(() => integration.loginCalls.length === 1);
    expect(integration.loginCalls[0]).toEqual({
      email: "admin@example.com",
      password: "pw-admin",
      organizationSlug: "acme",
    });
    // The session token was applied to rebuild the transport.
    expect(integration.applySessionCalls).toEqual(["sess-xyz"]);

    // The authenticated dashboard now renders and the placeholder is gone.
    await flushUntil(() => root.querySelector(".os-task-graph-panel") !== null);
    expect(root.querySelector("[data-testid='auth-form']")).toBeNull();
    expect(root.querySelector("[data-testid='auth-sign-out']")).not.toBeNull();

    await handle.destroy();
    root.remove();
  });

  it("surfaces an inline error and stays on the placeholder when login fails", async () => {
    const authenticated = makeAuthenticatedTransport();
    const integration = new FakeAuthIntegration(authenticated);
    // A classified unauthenticated failure from the auth client.
    const err = new Error("HTTP 401: bad credentials") as Error & { code: string };
    err.code = "unauthenticated";
    integration.failWith = err;

    const { root, handle } = mountWithAuth(integration, makeUnauthenticatedTransport());

    await flushUntil(() => root.querySelector("[data-testid='auth-form']") !== null);
    (root.querySelector("[data-testid='auth-email']") as HTMLInputElement).value = "admin@example.com";
    (root.querySelector("[data-testid='auth-password']") as HTMLInputElement).value = "wrong";
    (root.querySelector("[data-testid='auth-form']") as HTMLFormElement).dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );

    await flushUntil(() => integration.loginCalls.length === 1);
    await flushAsync();

    // The session was never applied and the dashboard did not render.
    expect(integration.applySessionCalls).toEqual([]);
    expect(root.querySelector(".os-task-graph-panel")).toBeNull();
    // An inline error is shown and the form remains available.
    expect(root.querySelector("[data-testid='auth-error']")).not.toBeNull();
    expect(root.querySelector("[data-testid='auth-form']")).not.toBeNull();
    // The button is re-enabled (no longer "Signing in...").
    expect(root.querySelector("[data-testid='auth-sign-in']")?.textContent).toContain("Sign in");

    await handle.destroy();
    root.remove();
  });

  it("signs out via the topbar control, clearing the session and returning to the placeholder", async () => {
    const authenticated = makeAuthenticatedTransport();
    const integration = new FakeAuthIntegration(authenticated);
    const { root, handle } = mountWithAuth(integration, makeUnauthenticatedTransport());

    await flushUntil(() => root.querySelector("[data-testid='auth-form']") !== null);
    (root.querySelector("[data-testid='auth-email']") as HTMLInputElement).value = "admin@example.com";
    (root.querySelector("[data-testid='auth-password']") as HTMLInputElement).value = "pw-admin";
    (root.querySelector("[data-testid='auth-form']") as HTMLFormElement).dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );

    await flushUntil(() => root.querySelector(".os-task-graph-panel") !== null);
    expect(root.querySelector("[data-testid='auth-sign-out']")).not.toBeNull();

    (root.querySelector("[data-testid='auth-sign-out']") as HTMLButtonElement).click();

    await flushUntil(() => integration.logoutCalls === 1);
    // applySession was called with undefined to clear the token.
    expect(integration.applySessionCalls).toContain(undefined);

    await handle.destroy();
    root.remove();
  });
});