import {
  HttpGatewayTransport,
  TransportFactory,
  type GatewayTransport,
} from "@opensymphony/api-client";
import type { ConnectionProfile } from "@opensymphony/gateway-schema";
import {
  renderOpenSymphonyApp,
  type EditableProfileInput,
  type ProfileController,
} from "@opensymphony/ui-core";

const DEFAULT_GATEWAY_URL = "http://127.0.0.1:8000";

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

export interface TauriTransportAdapter extends GatewayTransport {
  attach(): Promise<void>;
}

class DesktopTransportAdapter implements TauriTransportAdapter {
  constructor(
    private readonly inner: GatewayTransport,
    private readonly baseUrl: string,
  ) {}

  get baseUri(): string {
    return this.inner.baseUri;
  }

  health(): ReturnType<GatewayTransport["health"]> {
    return this.inner.health();
  }

  snapshot(): ReturnType<GatewayTransport["snapshot"]> {
    return this.inner.snapshot();
  }

  taskGraph(projectId: string): ReturnType<GatewayTransport["taskGraph"]> {
    return this.inner.taskGraph(projectId);
  }

  runDetail(runId: string): ReturnType<GatewayTransport["runDetail"]> {
    return this.inner.runDetail(runId);
  }

  runEvents(
    runId: string,
    cursor?: Parameters<GatewayTransport["runEvents"]>[1],
  ): ReturnType<GatewayTransport["runEvents"]> {
    return this.inner.runEvents(runId, cursor);
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
        profile_id: profileId,
      });
      return toConnectionProfile(active);
    },
  };
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
  const fallback = () => createDesktopTransport(base);
  const capabilities = await new HttpGatewayTransport({
    baseUri: base,
    transport: "loopback_http",
  }).health().catch(() => undefined);
  if (!capabilities) {
    return fallback();
  }
  const transport = await TransportFactory.create(
    { baseUri: base, transport: "loopback_http" },
    capabilities,
  ).catch(() => undefined);
  if (!transport) {
    return fallback();
  }
  return new DesktopTransportAdapter(transport, base);
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
