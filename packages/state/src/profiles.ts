/**
 * Connection profile state management.
 *
 * Manages profile selection, storage, and gateway URL override.
 */

import type { ConnectionProfile, ConnectionProfileKind } from "@opensymphony/gateway-schema";

/** Profile state slice. */
export interface ProfileState {
  /** Available connection profiles. */
  profiles: ConnectionProfile[];
  /** ID of the currently active profile. */
  activeProfileId: string | null;
  /** Manual gateway URL override. */
  gatewayUrlOverride: string | null;
  /** Whether discovery is in progress. */
  discovering: boolean;
  /** Discovery error message. */
  discoveryError: string | null;
  /** Last successful discovery timestamp. */
  lastDiscoveryAt: number | null;
}

/** Initial profile state with default profiles. */
export const initialProfileState: ProfileState = {
  profiles: [],
  activeProfileId: null,
  gatewayUrlOverride: null,
  discovering: false,
  discoveryError: null,
  lastDiscoveryAt: null,
};

/** Profile-related action types. */
export type ProfileAction =
  | { type: "PROFILE_ADD"; profile: ConnectionProfile }
  | { type: "PROFILE_UPDATE"; profile: ConnectionProfile }
  | { type: "PROFILE_REMOVE"; profileId: string }
  | { type: "PROFILE_SET_ACTIVE"; profileId: string }
  | { type: "PROFILES_LOAD"; profiles: ConnectionProfile[] }
  | { type: "GATEWAY_URL_OVERRIDE"; url: string | null }
  | { type: "DISCOVERY_START" }
  | { type: "DISCOVERY_SUCCESS"; timestamp: number }
  | { type: "DISCOVERY_FAILURE"; error: string }
  | { type: "PROFILE_RESET" };

/** Profile reducer. */
export function profileReducer(
  state: ProfileState,
  action: ProfileAction,
): ProfileState {
  switch (action.type) {
    case "PROFILE_ADD":
      return {
        ...state,
        profiles: [...state.profiles, action.profile],
      };

    case "PROFILE_UPDATE":
      return {
        ...state,
        profiles: state.profiles.map((p) =>
          p.id === action.profile.id ? action.profile : p,
        ),
      };

    case "PROFILE_REMOVE":
      return {
        ...state,
        profiles: state.profiles.filter((p) => p.id !== action.profileId),
        activeProfileId:
          state.activeProfileId === action.profileId
            ? null
            : state.activeProfileId,
      };

    case "PROFILE_SET_ACTIVE": {
      const profile = state.profiles.find((p) => p.id === action.profileId);
      if (!profile) {
        return state;
      }
      return {
        ...state,
        activeProfileId: action.profileId,
        profiles: state.profiles.map((p) => ({
          ...p,
          active: p.id === action.profileId,
        })),
      };
    }

    case "PROFILES_LOAD":
      return {
        ...state,
        profiles: action.profiles,
        activeProfileId:
          action.profiles.find((p) => p.active)?.id ?? state.activeProfileId,
      };

    case "GATEWAY_URL_OVERRIDE":
      return {
        ...state,
        gatewayUrlOverride: action.url,
      };

    case "DISCOVERY_START":
      return {
        ...state,
        discovering: true,
        discoveryError: null,
      };

    case "DISCOVERY_SUCCESS":
      return {
        ...state,
        discovering: false,
        discoveryError: null,
        lastDiscoveryAt: action.timestamp,
      };

    case "DISCOVERY_FAILURE":
      return {
        ...state,
        discovering: false,
        discoveryError: action.error,
      };

    case "PROFILE_RESET":
      return initialProfileState;

    default:
      return state;
  }
}

/**
 * Get the effective gateway URL for the active profile.
 *
 * Returns the override URL if set, otherwise the active profile's URL.
 */
export function getEffectiveGatewayUrl(state: ProfileState): string | null {
  if (state.gatewayUrlOverride) {
    return state.gatewayUrlOverride;
  }
  const activeProfile = state.profiles.find(
    (p) => p.id === state.activeProfileId,
  );
  return activeProfile?.gatewayUrl ?? null;
}

/**
 * Get the active profile.
 */
export function getActiveProfile(
  state: ProfileState,
): ConnectionProfile | null {
  return (
    state.profiles.find((p) => p.id === state.activeProfileId) ?? null
  );
}

/**
 * Check if the active profile is managed (daemon supervision enabled).
 */
export function isManagedProfile(state: ProfileState): boolean {
  const activeProfile = getActiveProfile(state);
  return activeProfile?.managed ?? false;
}

/**
 * Check if the active profile uses local transport (daemon or embedded).
 *
 * Heuristic: local profiles include daemon-managed modes and local gateways.
 * - local_daemon: unmanaged local daemon (user started externally)
 * - supervised_local_daemon: app-managed daemon lifecycle
 * - embedded_host: daemon runs in-process with the desktop app
 * - external_gateway: local gateway on loopback or LAN (not hosted cloud)
 *
 * hosted_gateway is excluded because it uses a remote cloud endpoint that
 * requires authentication and different connection handling.
 */
export function isLocalProfile(state: ProfileState): boolean {
  const activeProfile = getActiveProfile(state);
  if (!activeProfile) {
    return false;
  }
  const localKinds: ConnectionProfileKind[] = [
    "local_daemon",
    "supervised_local_daemon",
    "embedded_host",
    "external_gateway",
  ];
  return localKinds.includes(activeProfile.kind);
}
