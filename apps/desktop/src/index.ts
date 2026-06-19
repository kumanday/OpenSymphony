import {
  HttpGatewayTransport,
  type ActionCapableTransport,
  type ActionDispatch,
  type ActionReceipt,
  type GatewayTransport,
} from "@opensymphony/api-client";
import type { ConnectionProfile } from "@opensymphony/gateway-schema";
import {
  renderOpenSymphonyApp,
  type EditableProfileInput,
  type ProfileController,
} from "@opensymphony/ui-core";

const DEFAULT_GATEWAY_URL = "http://127.0.0.1:2468";

type TauriInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

interface TauriGlobal {
  invoke?: TauriInvoke;
  core?: {
    invoke?: TauriInvoke;
  };
}

interface NativeProfileResponse {
  id: string;
  label: string;
  kind: string;
  gateway_url?: string;
  gatewayUrl?: string;
  managed?: boolean;
  active?: boolean;
  daemon_path?: string | null;
  daemonPath?: string | null;
  transport?: ConnectionProfile["transport"];
}

export interface TauriTransportAdapter extends ActionCapableTransport {
  attach(): Promise<void>;
}

class DesktopTransportAdapter implements TauriTransportAdapter {
  private readonly actionInner: ActionCapableTransport;

  constructor(
    private readonly inner: GatewayTransport,
    private readonly baseUrl: string,
  ) {
    this.actionInner = asActionCapableTransport(inner, baseUrl);
  }

  get baseUri(): string {
    return this.inner.baseUri;
  }

  health(): ReturnType<GatewayTransport["health"]> {
    return this.invokeOrHttp("gateway_capabilities", {}, () => this.inner.health());
  }

  snapshot(): ReturnType<GatewayTransport["snapshot"]> {
    return this.invokeOrHttp("dashboard_snapshot", {}, () => this.inner.snapshot());
  }

  taskGraph(projectId: string): ReturnType<GatewayTransport["taskGraph"]> {
    return this.invokeOrHttp("task_graph", { projectId }, () => this.inner.taskGraph(projectId));
  }

  runDetail(runId: string): ReturnType<GatewayTransport["runDetail"]> {
    return this.invokeOrHttp("run_detail", { runId }, () => this.inner.runDetail(runId));
  }

  runEvents(
    runId: string,
    cursor?: Parameters<GatewayTransport["runEvents"]>[1],
  ): ReturnType<GatewayTransport["runEvents"]> {
    return this.invokeOrHttp(
      "run_events",
      {
        runId,
        pageToken: cursor?.page_token ?? null,
        pageSize: cursor?.page_size ?? null,
      },
      () => this.inner.runEvents(runId, cursor),
    );
  }

  runTimeline(runId: string): ReturnType<GatewayTransport["runTimeline"]> {
    return this.inner.runTimeline(runId);
  }

  runLogs(
    runId: string,
    cursor?: Parameters<GatewayTransport["runLogs"]>[1],
    limit?: Parameters<GatewayTransport["runLogs"]>[2],
  ): ReturnType<GatewayTransport["runLogs"]> {
    return this.inner.runLogs(runId, cursor, limit);
  }

  terminalSnapshot(
    runId: string,
    terminalId: string,
    cursor?: Parameters<GatewayTransport["terminalSnapshot"]>[2],
  ): ReturnType<GatewayTransport["terminalSnapshot"]> {
    return this.inner.terminalSnapshot(runId, terminalId, cursor);
  }

  terminalSearch(
    runId: string,
    terminalId: string,
    query: string,
  ): ReturnType<GatewayTransport["terminalSearch"]> {
    return this.inner.terminalSearch(runId, terminalId, query);
  }

  terminalJumpToEvent(
    runId: string,
    terminalId: string,
    eventId: string,
  ): ReturnType<GatewayTransport["terminalJumpToEvent"]> {
    return this.inner.terminalJumpToEvent(runId, terminalId, eventId);
  }

  runFiles(runId: string): ReturnType<GatewayTransport["runFiles"]> {
    return this.invokeOrHttp<{ files?: Awaited<ReturnType<GatewayTransport["runFiles"]>> }>(
      "run_files",
      { runId },
      async () => ({ files: await this.inner.runFiles(runId) }),
    ).then((response) => response.files ?? []);
  }

  runDiffs(runId: string, filePath?: string): ReturnType<GatewayTransport["runDiffs"]> {
    return this.invokeOrHttp("run_diffs", { runId, filePath: filePath ?? null }, () =>
      this.inner.runDiffs(runId, filePath),
    );
  }

  runApprovals(runId: string): ReturnType<GatewayTransport["runApprovals"]> {
    return this.invokeOrHttp<{ approvals?: Awaited<ReturnType<GatewayTransport["runApprovals"]>> }>(
      "run_approvals",
      { runId },
      async () => ({ approvals: await this.inner.runApprovals(runId) }),
    ).then((response) => response.approvals ?? []);
  }

  runValidation(runId: string): ReturnType<GatewayTransport["runValidation"]> {
    return this.invokeOrHttp("run_validation", { runId }, () => this.inner.runValidation(runId));
  }

  events(
    fromCursor?: Parameters<GatewayTransport["events"]>[0],
  ): ReturnType<GatewayTransport["events"]> {
    return this.inner.events(fromCursor);
  }

  terminalFrames(
    runId: string,
  ): ReturnType<GatewayTransport["terminalFrames"]> {
    return this.inner.terminalFrames(runId);
  }

  close(): ReturnType<GatewayTransport["close"]> {
    return this.inner.close();
  }

  dispatchAction(action: ActionDispatch): Promise<ActionReceipt> {
    return this.actionInner.dispatchAction(action);
  }

  cancelRun(runId: string): Promise<ActionReceipt> {
    return this.actionInner.cancelRun(runId);
  }

  retryRun(runId: string): Promise<ActionReceipt> {
    return this.actionInner.retryRun(runId);
  }

  resumeRun(runId: string): Promise<ActionReceipt> {
    return this.actionInner.resumeRun(runId);
  }

  rehydrateRun(runId: string): Promise<ActionReceipt> {
    return this.actionInner.rehydrateRun(runId);
  }

  commentRun(runId: string, text: string): Promise<ActionReceipt> {
    return this.actionInner.commentRun(runId, text);
  }

  createFollowup(runId: string, payload: unknown): Promise<ActionReceipt> {
    return this.actionInner.createFollowup(runId, payload);
  }

  approvalDecision(
    approvalId: string,
    decision: "approved" | "rejected",
    explanation?: string,
  ): Promise<ActionReceipt> {
    return this.actionInner.approvalDecision(approvalId, decision, explanation);
  }

  openWorkspace(runId: string): Promise<ActionReceipt> {
    return this.actionInner.openWorkspace(runId);
  }

  debugRun(runId: string): Promise<ActionReceipt> {
    return this.actionInner.debugRun(runId);
  }

  async attach(): Promise<void> {
    const invoke = getTauriInvoke();
    if (!invoke) {
      return;
    }
    await invoke("attach_gateway", {
      req: {
        base_url: this.baseUrl,
        auth_token: null,
      },
    }).catch(() => undefined);
  }

  private async invokeOrHttp<T>(
    command: string,
    args: Record<string, unknown>,
    fallback: () => Promise<T>,
  ): Promise<T> {
    const invoke = getTauriInvoke();
    if (!invoke) {
      return fallback();
    }
    return invoke<T>(command, args);
  }
}

export function createDesktopTransport(
  baseUri = DEFAULT_GATEWAY_URL,
): TauriTransportAdapter {
  return new DesktopTransportAdapter(new HttpGatewayTransport({
    baseUri,
    transport: "loopback_http",
  }), baseUri);
}

export function createDesktopProfileController(): ProfileController | undefined {
  const invoke = getTauriInvoke();
  if (!invoke) {
    return undefined;
  }

  return {
    async listProfiles() {
      const profiles = await invoke<NativeProfileResponse[]>("list_profiles", {});
      return profiles.map(toConnectionProfile);
    },

    async storeProfile(profile: EditableProfileInput) {
      const stored = await invoke<NativeProfileResponse>("store_profile", {
        req: {
          id: profile.id ?? null,
          label: profile.label,
          kind: profile.kind,
          gateway_url: profile.gatewayUrl,
          daemon_path: null,
          daemon_args: [],
          auto_restart: false,
          startup_timeout_secs: 30,
        },
      });
      return toConnectionProfile(stored);
    },

    async setActiveProfile(profileId: string) {
      const active = await invoke<NativeProfileResponse>("set_active_profile", {
        profileId,
      });
      return toConnectionProfile(active);
    },
  };
}

function asActionCapableTransport(
  transport: GatewayTransport,
  baseUrl: string,
): ActionCapableTransport {
  if ("dispatchAction" in transport) {
    return transport as ActionCapableTransport;
  }
  // Fallback: when the inner transport is not action-capable (e.g. a plain
  // read-only channel), open a separate loopback HTTP connection to the
  // gateway for action dispatch. This is intentional for desktop because the
  // Tauri channel implementation is action-capable; the HTTP fallback is the
  // documented baseline and preserves the same auth/CORS contract as the
  // desktop app's own loopback server.
  return new HttpGatewayTransport({
    baseUri: baseUrl || DEFAULT_GATEWAY_URL,
    transport: "loopback_http",
  });
}

function getTauriInvoke(): TauriInvoke | undefined {
  const tauri = (globalThis as Record<string, unknown>).__TAURI__ as
    | TauriGlobal
    | undefined;
  return tauri?.invoke ?? tauri?.core?.invoke;
}

function toConnectionProfile(profile: NativeProfileResponse): ConnectionProfile {
  const gatewayUrl = profile.gatewayUrl ?? profile.gateway_url ?? DEFAULT_GATEWAY_URL;
  const base = {
    id: profile.id,
    label: profile.label,
    active: profile.active ?? false,
    gatewayUrl,
    transport: profile.transport ?? "loopback_http",
    managed: profile.managed ?? isManagedKind(profile.kind),
  };

  switch (profile.kind) {
    case "supervised_local_daemon":
      return {
        ...base,
        kind: "supervised_local_daemon",
        managed: true,
        daemonPath: profile.daemonPath ?? profile.daemon_path ?? "",
        daemonArgs: [],
        daemonEnv: {},
        startupTimeoutSecs: 30,
        autoRestart: false,
      };
    case "embedded_host":
      return {
        ...base,
        kind: "embedded_host",
        managed: true,
      };
    case "hosted_gateway":
      return {
        ...base,
        kind: "hosted_gateway",
        managed: false,
        probeOnConnect: true,
        transport: "websocket",
      };
    case "external_gateway":
      return {
        ...base,
        kind: "external_gateway",
        managed: false,
        probeOnConnect: true,
      };
    case "local_daemon":
      return {
        ...base,
        kind: "local_daemon",
        managed: false,
      };
    default:
      return {
        ...base,
        kind: profile.kind as ConnectionProfile["kind"],
        managed: profile.managed ?? false,
      } as ConnectionProfile;
  }
}

function isManagedKind(kind: string): boolean {
  return kind === "embedded_host" || kind === "supervised_local_daemon";
}

async function createTransportForGateway(gatewayUrl: string): Promise<TauriTransportAdapter> {
  const base = gatewayUrl || DEFAULT_GATEWAY_URL;
  const transport = createDesktopTransport(base);
  await transport.attach();
  return transport;
}

const root = document.getElementById("root");
if (root) {
  const transport = createDesktopTransport();
  void transport.attach();
  renderOpenSymphonyApp({
    root,
    mode: "desktop",
    title: "OpenSymphony Desktop",
    transport,
    profileController: createDesktopProfileController(),
    initialProfiles: [
      {
        id: "local-daemon",
        label: "Local Daemon",
        kind: "local_daemon",
        active: true,
        gatewayUrl: DEFAULT_GATEWAY_URL,
        transport: "loopback_http",
        managed: false,
      },
    ],
    onGatewayUrlChanged: createTransportForGateway,
  });
}
