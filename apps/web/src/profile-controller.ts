import type { ConnectionProfile } from "@opensymphony/gateway-schema";
import type { EditableProfileInput, ProfileController } from "@opensymphony/ui-core";

const STORAGE_KEY = "opensymphony.web.connectionProfiles.v1";
const DEFAULT_PROFILE_ID = "web-local-daemon";

type ProfileStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;

interface StoredProfiles {
  profiles: ConnectionProfile[];
  activeProfileId: string | null;
}

export interface WebProfileControllerOptions {
  defaultGatewayUrl: string;
  storage?: ProfileStorage | null;
}

let profileCounter = 0;

export function createWebProfileController(
  options: WebProfileControllerOptions,
): ProfileController {
  const storage = options.storage ?? browserStorage();
  let fallback = normalizeStored(null, options.defaultGatewayUrl);

  function read(): StoredProfiles {
    if (!storage) {
      return fallback;
    }
    try {
      const raw = storage.getItem(STORAGE_KEY);
      return normalizeStored(raw ? JSON.parse(raw) : null, options.defaultGatewayUrl);
    } catch {
      return normalizeStored(null, options.defaultGatewayUrl);
    }
  }

  function write(stored: StoredProfiles): void {
    const normalized = normalizeStored(stored, options.defaultGatewayUrl);
    fallback = normalized;
    if (!storage) {
      return;
    }
    try {
      storage.setItem(STORAGE_KEY, JSON.stringify(normalized));
    } catch {
      // Keep the in-memory fallback usable when storage is unavailable or full.
    }
  }

  return {
    async listProfiles() {
      return read().profiles;
    },

    async storeProfile(input: EditableProfileInput) {
      const stored = read();
      const id = input.id ?? nextProfileId();
      const saved = profileFromInput(input, id);
      const profiles = stored.profiles.filter((profile) => profile.id !== id);
      const next = normalizeStored(
        {
          profiles: [...profiles, saved],
          activeProfileId: saved.id,
        },
        options.defaultGatewayUrl,
      );
      write(next);
      return next.profiles.find((profile) => profile.id === saved.id)!;
    },

    async setActiveProfile(profileId: string) {
      const stored = read();
      const active = stored.profiles.find((profile) => profile.id === profileId);
      if (!active) {
        throw new Error(`Unknown profile: ${profileId}`);
      }
      const next = normalizeStored(
        {
          profiles: stored.profiles.map((profile) => ({
            ...profile,
            active: profile.id === profileId,
          })),
          activeProfileId: profileId,
        },
        options.defaultGatewayUrl,
      );
      write(next);
      return next.profiles.find((profile) => profile.id === profileId)!;
    },

    async removeProfile(profileId: string) {
      const stored = read();
      if (!stored.profiles.some((profile) => profile.id === profileId)) {
        throw new Error(`Unknown profile: ${profileId}`);
      }
      if (stored.profiles.length <= 1) {
        throw new Error("Cannot remove the last profile");
      }
      const profiles = stored.profiles.filter((profile) => profile.id !== profileId);
      const next = normalizeStored(
        {
          profiles,
          activeProfileId:
            stored.activeProfileId === profileId
              ? profiles[0]?.id ?? null
              : stored.activeProfileId,
        },
        options.defaultGatewayUrl,
      );
      write(next);
      return next.profiles;
    },
  };
}

export function defaultWebGatewayUrl(): string {
  if (typeof window === "undefined") {
    return "";
  }
  return window.location.origin;
}

function browserStorage(): ProfileStorage | null {
  try {
    return typeof window !== "undefined" ? window.localStorage : null;
  } catch {
    return null;
  }
}

function normalizeStored(
  value: Partial<StoredProfiles> | null,
  defaultGatewayUrl: string,
): StoredProfiles {
  const profiles = Array.isArray(value?.profiles) && value.profiles.length > 0
    ? value.profiles
    : [defaultProfile(defaultGatewayUrl)];
  const activeProfileId = value?.activeProfileId
    ?? profiles.find((profile) => profile.active)?.id
    ?? profiles[0]?.id
    ?? null;
  return {
    profiles: profiles.map((profile) => ({
      ...profile,
      active: profile.id === activeProfileId,
      gatewayUrl: profile.gatewayUrl || defaultGatewayUrl,
    })) as ConnectionProfile[],
    activeProfileId,
  };
}

function defaultProfile(defaultGatewayUrl: string): ConnectionProfile {
  return {
    id: DEFAULT_PROFILE_ID,
    label: "Local Gateway",
    kind: "local_daemon",
    active: true,
    gatewayUrl: defaultGatewayUrl,
    transport: "loopback_http",
    managed: false,
  };
}

function profileFromInput(input: EditableProfileInput, id: string): ConnectionProfile {
  const base = {
    id,
    label: input.label,
    kind: input.kind,
    active: true,
  };
  switch (input.kind) {
    case "supervised_local_daemon":
      return {
        ...base,
        kind: "supervised_local_daemon",
        gatewayUrl: input.gatewayUrl,
        transport: "loopback_http",
        managed: true,
        daemonPath: "",
        daemonArgs: [],
        daemonEnv: {},
        startupTimeoutSecs: 30,
        autoRestart: true,
      };
    case "embedded_host":
      return {
        ...base,
        kind: "embedded_host",
        gatewayUrl: input.gatewayUrl,
        transport: "in_process_channel",
        managed: true,
      };
    case "external_gateway":
      return {
        ...base,
        kind: "external_gateway",
        gatewayUrl: input.gatewayUrl,
        transport: "loopback_http",
        managed: false,
        probeOnConnect: true,
      };
    case "hosted_gateway":
      return {
        ...base,
        kind: "hosted_gateway",
        gatewayUrl: input.gatewayUrl,
        transport: "websocket",
        managed: false,
        probeOnConnect: true,
      };
    case "local_daemon":
    default:
      return {
        ...base,
        kind: "local_daemon",
        gatewayUrl: input.gatewayUrl,
        transport: "loopback_http",
        managed: false,
      };
  }
}

function nextProfileId(): string {
  profileCounter += 1;
  return `web-profile-${Date.now()}-${profileCounter}`;
}
