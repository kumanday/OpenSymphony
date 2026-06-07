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
  kind: ConnectionProfile["kind"];
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

export function createDesktopTransport(
  baseUri = DEFAULT_GATEWAY_URL,
): TauriTransportAdapter {
  const transport = new HttpGatewayTransport({
    baseUri,
    transport: "loopback_http",
  }) as HttpGatewayTransport & { attach(): Promise<void> };

  transport.attach = async () => {
    const invoke = getTauriInvoke();
    if (!invoke) {
      return;
    }
    await invoke("attach_gateway", {
      req: {
        base_url: baseUri,
        auth_token: null,
      },
    }).catch(() => undefined);
  };

  return transport;
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
    kind: profile.kind,
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
    default:
      return {
        ...base,
        kind: "local_daemon",
        managed: false,
      };
  }
}

function isManagedKind(kind: ConnectionProfile["kind"]): boolean {
  return kind === "embedded_host" || kind === "supervised_local_daemon";
}

async function createTransportForGateway(gatewayUrl: string): Promise<TauriTransportAdapter> {
  const base = gatewayUrl || DEFAULT_GATEWAY_URL;
  const capabilities = await new HttpGatewayTransport({
    baseUri: base,
    transport: "loopback_http",
  }).health().catch(() => undefined);
  const transport = await TransportFactory.create(
    { baseUri: base, transport: "loopback_http" },
    capabilities,
  );
  const withAttach = transport as GatewayTransport & { attach(): Promise<void> };
  withAttach.attach = async () => {
    const invoke = getTauriInvoke();
    if (!invoke) {
      return;
    }
    await invoke("attach_gateway", {
      req: {
        base_url: base,
        auth_token: null,
      },
    }).catch(() => undefined);
  };
  return withAttach;
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
