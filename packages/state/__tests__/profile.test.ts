/**
 * Unit tests for connection profile state management.
 *
 * Tests profile reducer actions, state transitions, and selector functions.
 */

import { describe, it, expect, beforeEach } from "@jest/globals";
import {
  profileReducer,
  initialProfileState,
  getEffectiveGatewayUrl,
  getActiveProfile,
  isManagedProfile,
  isLocalProfile,
  type ProfileState,
  type ProfileAction,
} from "../src/profiles";
import {
  createProfile,
  type ConnectionProfile,
  type LocalDaemonProfile,
  type SupervisedLocalDaemonProfile,
  type ExternalGatewayProfile,
} from "@opensymphony/gateway-schema";

function testProfile(overrides?: Partial<LocalDaemonProfile>): LocalDaemonProfile {
  const base = createProfile("local_daemon") as LocalDaemonProfile;
  return {
    ...base,
    ...overrides,
  };
}

describe("profileReducer", () => {
  let state: ProfileState;

  beforeEach(() => {
    state = { ...initialProfileState };
  });

  it("returns initial state for unknown action", () => {
    const newState = profileReducer(state, { type: "UNKNOWN" } as any);
    expect(newState).toEqual(state);
  });

  describe("PROFILE_ADD", () => {
    it("adds a new profile to the list", () => {
      const profile = testProfile({ id: "test-1", label: "Test Profile" });
      const newState = profileReducer(state, {
        type: "PROFILE_ADD",
        profile,
      });
      expect(newState.profiles).toHaveLength(1);
      expect(newState.profiles[0].id).toBe("test-1");
    });

    it("appends profiles without replacing existing ones", () => {
      const profile1 = testProfile({ id: "p1" });
      const profile2 = testProfile({ id: "p2" });
      let newState = profileReducer(state, {
        type: "PROFILE_ADD",
        profile: profile1,
      });
      newState = profileReducer(newState, {
        type: "PROFILE_ADD",
        profile: profile2,
      });
      expect(newState.profiles).toHaveLength(2);
      expect(newState.profiles.map((p) => p.id)).toEqual(["p1", "p2"]);
    });
  });

  describe("PROFILE_UPDATE", () => {
    it("updates an existing profile", () => {
      const profile = testProfile({ id: "test-1", label: "Old Label" });
      let newState = profileReducer(state, {
        type: "PROFILE_ADD",
        profile,
      });
      const updatedProfile = { ...profile, label: "New Label" };
      newState = profileReducer(newState, {
        type: "PROFILE_UPDATE",
        profile: updatedProfile,
      });
      expect(newState.profiles[0].label).toBe("New Label");
    });

    it("leaves other profiles unchanged", () => {
      const p1 = testProfile({ id: "p1", label: "P1" });
      const p2 = testProfile({ id: "p2", label: "P2" });
      let newState = profileReducer(state, {
        type: "PROFILE_ADD",
        profile: p1,
      });
      newState = profileReducer(newState, {
        type: "PROFILE_ADD",
        profile: p2,
      });
      const updatedP1 = { ...p1, label: "Updated P1" };
      newState = profileReducer(newState, {
        type: "PROFILE_UPDATE",
        profile: updatedP1,
      });
      expect(newState.profiles.find((p) => p.id === "p1")?.label).toBe(
        "Updated P1",
      );
      expect(newState.profiles.find((p) => p.id === "p2")?.label).toBe("P2");
    });
  });

  describe("PROFILE_REMOVE", () => {
    it("removes a profile by ID", () => {
      const profile = testProfile({ id: "to-remove" });
      let newState = profileReducer(state, {
        type: "PROFILE_ADD",
        profile,
      });
      newState = profileReducer(newState, {
        type: "PROFILE_REMOVE",
        profileId: "to-remove",
      });
      expect(newState.profiles).toHaveLength(0);
    });

    it("clears activeProfileId if the removed profile was active", () => {
      const profile = testProfile({ id: "active-profile", active: true });
      let newState = profileReducer(state, {
        type: "PROFILE_ADD",
        profile,
      });
      newState = profileReducer(newState, {
        type: "PROFILE_SET_ACTIVE",
        profileId: "active-profile",
      });
      expect(newState.activeProfileId).toBe("active-profile");
      newState = profileReducer(newState, {
        type: "PROFILE_REMOVE",
        profileId: "active-profile",
      });
      expect(newState.activeProfileId).toBeNull();
    });
  });

  describe("PROFILE_SET_ACTIVE", () => {
    it("sets the active profile ID", () => {
      const p1 = testProfile({ id: "p1" });
      const p2 = testProfile({ id: "p2" });
      let newState = profileReducer(state, {
        type: "PROFILE_ADD",
        profile: p1,
      });
      newState = profileReducer(newState, {
        type: "PROFILE_ADD",
        profile: p2,
      });
      newState = profileReducer(newState, {
        type: "PROFILE_SET_ACTIVE",
        profileId: "p2",
      });
      expect(newState.activeProfileId).toBe("p2");
    });

    it("updates the active flag on profiles", () => {
      const p1 = testProfile({ id: "p1" });
      const p2 = testProfile({ id: "p2" });
      let newState = profileReducer(state, {
        type: "PROFILE_ADD",
        profile: p1,
      });
      newState = profileReducer(newState, {
        type: "PROFILE_ADD",
        profile: p2,
      });
      newState = profileReducer(newState, {
        type: "PROFILE_SET_ACTIVE",
        profileId: "p2",
      });
      expect(newState.profiles.find((p) => p.id === "p1")?.active).toBe(false);
      expect(newState.profiles.find((p) => p.id === "p2")?.active).toBe(true);
    });

    it("does nothing for non-existent profile ID", () => {
      const newState = profileReducer(state, {
        type: "PROFILE_SET_ACTIVE",
        profileId: "nonexistent",
      });
      expect(newState).toEqual(state);
    });
  });

  describe("PROFILES_LOAD", () => {
    it("replaces all profiles", () => {
      const profiles: ConnectionProfile[] = [
        testProfile({ id: "p1" }),
        testProfile({ id: "p2" }),
      ];
      const newState = profileReducer(state, {
        type: "PROFILES_LOAD",
        profiles,
      });
      expect(newState.profiles).toHaveLength(2);
    });

    it("sets activeProfileId if a loaded profile is active", () => {
      const profiles: ConnectionProfile[] = [
        testProfile({ id: "p1" }),
        testProfile({ id: "p2", active: true }),
      ];
      const newState = profileReducer(state, {
        type: "PROFILES_LOAD",
        profiles,
      });
      expect(newState.activeProfileId).toBe("p2");
    });
  });

  describe("GATEWAY_URL_OVERRIDE", () => {
    it("sets the override URL", () => {
      const newState = profileReducer(state, {
        type: "GATEWAY_URL_OVERRIDE",
        url: "http://custom-server:9999",
      });
      expect(newState.gatewayUrlOverride).toBe("http://custom-server:9999");
    });

    it("clears the override URL when null", () => {
      let newState = profileReducer(state, {
        type: "GATEWAY_URL_OVERRIDE",
        url: "http://custom-server:9999",
      });
      newState = profileReducer(newState, {
        type: "GATEWAY_URL_OVERRIDE",
        url: null,
      });
      expect(newState.gatewayUrlOverride).toBeNull();
    });
  });

  describe("DISCOVERY_START/SUCCESS/FAILURE", () => {
    it("marks discovery as in progress", () => {
      const newState = profileReducer(state, {
        type: "DISCOVERY_START",
      });
      expect(newState.discovering).toBe(true);
      expect(newState.discoveryError).toBeNull();
    });

    it("marks discovery as successful with timestamp", () => {
      let newState = profileReducer(state, {
        type: "DISCOVERY_START",
      });
      const timestamp = Date.now();
      newState = profileReducer(newState, {
        type: "DISCOVERY_SUCCESS",
        timestamp,
      });
      expect(newState.discovering).toBe(false);
      expect(newState.discoveryError).toBeNull();
      expect(newState.lastDiscoveryAt).toBe(timestamp);
    });

    it("marks discovery as failed with error", () => {
      let newState = profileReducer(state, {
        type: "DISCOVERY_START",
      });
      newState = profileReducer(newState, {
        type: "DISCOVERY_FAILURE",
        error: "Connection refused",
      });
      expect(newState.discovering).toBe(false);
      expect(newState.discoveryError).toBe("Connection refused");
    });
  });

  describe("PROFILE_RESET", () => {
    it("returns to initial state", () => {
      const profile = testProfile({ id: "p1" });
      let newState = profileReducer(state, {
        type: "PROFILE_ADD",
        profile,
      });
      newState = profileReducer(newState, {
        type: "PROFILE_SET_ACTIVE",
        profileId: "p1",
      });
      newState = profileReducer(newState, {
        type: "PROFILE_RESET",
      });
      expect(newState).toEqual(initialProfileState);
    });
  });
});

describe("selectors", () => {
  let state: ProfileState;

  beforeEach(() => {
    state = { ...initialProfileState };
  });

  describe("getEffectiveGatewayUrl", () => {
    it("returns override URL when set", () => {
      state = profileReducer(state, {
        type: "GATEWAY_URL_OVERRIDE",
        url: "http://override:8888",
      });
      expect(getEffectiveGatewayUrl(state)).toBe("http://override:8888");
    });

    it("returns active profile URL when no override", () => {
      const profile = testProfile({
        id: "active",
        gatewayUrl: "http://active:7777",
      });
      state = profileReducer(state, {
        type: "PROFILE_ADD",
        profile,
      });
      state = profileReducer(state, {
        type: "PROFILE_SET_ACTIVE",
        profileId: "active",
      });
      expect(getEffectiveGatewayUrl(state)).toBe("http://active:7777");
    });

    it("returns null when no active profile and no override", () => {
      expect(getEffectiveGatewayUrl(state)).toBeNull();
    });
  });

  describe("getActiveProfile", () => {
    it("returns the active profile", () => {
      const profile = testProfile({ id: "active" });
      state = profileReducer(state, {
        type: "PROFILE_ADD",
        profile,
      });
      state = profileReducer(state, {
        type: "PROFILE_SET_ACTIVE",
        profileId: "active",
      });
      const active = getActiveProfile(state);
      expect(active?.id).toBe("active");
    });

    it("returns null when no profile is active", () => {
      expect(getActiveProfile(state)).toBeNull();
    });
  });

  describe("isManagedProfile", () => {
    it("returns true for supervised_local_daemon", () => {
      const profile = createProfile("supervised_local_daemon") as SupervisedLocalDaemonProfile;
      profile.id = "supervised";
      state = profileReducer(state, {
        type: "PROFILE_ADD",
        profile,
      });
      state = profileReducer(state, {
        type: "PROFILE_SET_ACTIVE",
        profileId: "supervised",
      });
      expect(isManagedProfile(state)).toBe(true);
    });

    it("returns false for local_daemon", () => {
      const profile = createProfile("local_daemon") as LocalDaemonProfile;
      profile.id = "local";
      state = profileReducer(state, {
        type: "PROFILE_ADD",
        profile,
      });
      state = profileReducer(state, {
        type: "PROFILE_SET_ACTIVE",
        profileId: "local",
      });
      expect(isManagedProfile(state)).toBe(false);
    });

    it("returns false when no profile is active", () => {
      expect(isManagedProfile(state)).toBe(false);
    });
  });

  describe("isLocalProfile", () => {
    it("returns true for local_daemon", () => {
      const profile = createProfile("local_daemon");
      profile.id = "local";
      state = profileReducer(state, {
        type: "PROFILE_ADD",
        profile,
      });
      state = profileReducer(state, {
        type: "PROFILE_SET_ACTIVE",
        profileId: "local",
      });
      expect(isLocalProfile(state)).toBe(true);
    });

    it("returns true for external_gateway", () => {
      const profile = createProfile("external_gateway") as ExternalGatewayProfile;
      profile.id = "external";
      state = profileReducer(state, {
        type: "PROFILE_ADD",
        profile,
      });
      state = profileReducer(state, {
        type: "PROFILE_SET_ACTIVE",
        profileId: "external",
      });
      expect(isLocalProfile(state)).toBe(true);
    });

    it("returns false for hosted_gateway", () => {
      const profile = createProfile("hosted_gateway");
      profile.id = "hosted";
      state = profileReducer(state, {
        type: "PROFILE_ADD",
        profile,
      });
      state = profileReducer(state, {
        type: "PROFILE_SET_ACTIVE",
        profileId: "hosted",
      });
      expect(isLocalProfile(state)).toBe(false);
    });

    it("returns false when no profile is active", () => {
      expect(isLocalProfile(state)).toBe(false);
    });
  });
});
