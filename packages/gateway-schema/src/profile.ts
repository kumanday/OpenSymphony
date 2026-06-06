/**
 * Connection profiles define how the desktop client connects to an OpenSymphony gateway.
 *
 * Profiles determine the transport, discovery behavior, and daemon supervision settings.
 */

import type { TransportProfile } from "./transport.js";

/** Profile type discriminator. */
export type ConnectionProfileKind =
  | "local_daemon"
  | "supervised_local_daemon"
  | "embedded_host"
  | "external_gateway"
  | "hosted_gateway";

/** Base connection profile fields shared by all profile kinds. */
export interface ConnectionProfileBase {
  /** Unique identifier for this profile instance. */
  id: string;
  /** Human-readable label shown in UI. */
  label: string;
  /** Discriminates the profile kind. */
  kind: ConnectionProfileKind;
  /** Whether this profile is currently active. */
  active: boolean;
}

/** Local daemon: connects to a separately-started daemon on loopback. */
export interface LocalDaemonProfile extends ConnectionProfileBase {
  kind: "local_daemon";
  /** Gateway base URL, defaults to http://127.0.0.1:8080. */
  gatewayUrl: string;
  /** Preferred transport for local communication. */
  transport: TransportProfile;
  /** Do not start/stop the daemon; connect to an externally-managed process. */
  managed: false;
}

/** Supervised local daemon: desktop app owns the daemon lifecycle. */
export interface SupervisedLocalDaemonProfile extends ConnectionProfileBase {
  kind: "supervised_local_daemon";
  /** Gateway base URL, defaults to http://127.0.0.1:8080. */
  gatewayUrl: string;
  /** Preferred transport for local communication. */
  transport: TransportProfile;
  /** Desktop app supervises the daemon process. */
  managed: true;
  /** Path to the daemon executable. */
  daemonPath: string;
  /** Optional arguments passed to the daemon. */
  daemonArgs: string[];
  /** Environment overrides for the daemon process. */
  daemonEnv: Record<string, string>;
  /** Maximum seconds to wait for the daemon to become healthy. */
  startupTimeoutSecs: number;
  /** Whether to restart the daemon if it exits unexpectedly. */
  autoRestart: boolean;
}

/** Embedded/direct host: OpenSymphony runs in-process with the desktop shell. */
export interface EmbeddedHostProfile extends ConnectionProfileBase {
  kind: "embedded_host";
  /** Gateway base URL (loopback for embedded HTTP fallback). */
  gatewayUrl: string;
  /** Preferred in-process transport. */
  transport: TransportProfile;
  /** Desktop app owns the host lifecycle. */
  managed: true;
}

/** External gateway: connects to a local server on loopback or trusted network. */
export interface ExternalGatewayProfile extends ConnectionProfileBase {
  kind: "external_gateway";
  /** Gateway base URL, user-configurable. */
  gatewayUrl: string;
  /** Loopback HTTP/WebSocket transport. */
  transport: TransportProfile;
  /** No daemon management. */
  managed: false;
  /** Whether to probe /healthz on startup. */
  probeOnConnect: boolean;
}

/** Hosted gateway: connects to a remote hosted OpenSymphony server. */
export interface HostedGatewayProfile extends ConnectionProfileBase {
  kind: "hosted_gateway";
  /** Gateway base URL for the hosted server. */
  gatewayUrl: string;
  /** Authenticated HTTPS/WSS transport. */
  transport: TransportProfile;
  /** No daemon management. */
  managed: false;
  /** Whether to probe /healthz on startup. */
  probeOnConnect: boolean;
}

/** Union of all connection profile variants. */
export type ConnectionProfile =
  | LocalDaemonProfile
  | SupervisedLocalDaemonProfile
  | EmbeddedHostProfile
  | ExternalGatewayProfile
  | HostedGatewayProfile;

/** Default connection profiles shipped with the desktop app. */
export function defaultProfiles(): ConnectionProfile[] {
  return [
    {
      id: "local-daemon",
      label: "Local Daemon",
      kind: "local_daemon",
      active: false,
      gatewayUrl: "http://127.0.0.1:8080",
      transport: "loopback_http",
      managed: false,
    },
    {
      id: "supervised-local-daemon",
      label: "Supervised Local Daemon",
      kind: "supervised_local_daemon",
      active: false,
      gatewayUrl: "http://127.0.0.1:8080",
      transport: "loopback_http",
      managed: true,
      daemonPath: "",
      daemonArgs: [],
      daemonEnv: {},
      startupTimeoutSecs: 30,
      autoRestart: true,
    },
    {
      id: "embedded-host",
      label: "Embedded Host",
      kind: "embedded_host",
      active: false,
      gatewayUrl: "http://127.0.0.1:8080",
      transport: "in_process_channel",
      managed: true,
    },
    {
      id: "external-gateway",
      label: "External Gateway",
      kind: "external_gateway",
      active: false,
      gatewayUrl: "http://127.0.0.1:8080",
      transport: "loopback_http",
      managed: false,
      probeOnConnect: true,
    },
    {
      id: "hosted-gateway",
      label: "Hosted Gateway",
      kind: "hosted_gateway",
      active: false,
      gatewayUrl: "",
      transport: "websocket",
      managed: false,
      probeOnConnect: true,
    },
  ];
}

/** Monotonic counter to ensure unique IDs even within the same millisecond. */
let _profileIdCounter = 0;

/** Create a blank profile of the given kind with sensible defaults. */
export function createProfile(kind: ConnectionProfileKind): ConnectionProfile {
  const defaults = defaultProfiles().find((p) => p.kind === kind);
  if (!defaults) {
    throw new Error(`Unknown connection profile kind: ${kind}`);
  }
  _profileIdCounter++;
  return {
    ...defaults,
    id: `${kind}-${Date.now()}-${_profileIdCounter}`,
    active: false,
  };
}
