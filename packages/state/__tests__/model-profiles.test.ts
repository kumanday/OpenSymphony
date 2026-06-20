import { describe, expect, it } from "@jest/globals";
import {
  createModelProfile,
  defaultModelProfiles,
} from "@opensymphony/gateway-schema";
import {
  createModelProfileStore,
  getActiveModelProfile,
  initialModelProfileState,
  modelProfileReducer,
} from "../src/model-profiles";

class MemoryStorage implements Pick<Storage, "getItem" | "setItem" | "removeItem"> {
  private readonly values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }
}

describe("modelProfileReducer", () => {
  it("loads profiles and selects the active profile", () => {
    const profiles = defaultModelProfiles();
    const state = modelProfileReducer(initialModelProfileState, {
      type: "MODEL_PROFILES_LOAD",
      profiles,
    });

    expect(state.profiles).toHaveLength(2);
    expect(getActiveModelProfile(state)?.id).toBe("openai-api-compatible");
  });

  it("adds, updates, activates, and removes model profiles", () => {
    const apiProfile = createModelProfile("api_key");
    const subscriptionProfile = createModelProfile("subscription");
    let state = modelProfileReducer(initialModelProfileState, {
      type: "MODEL_PROFILE_ADD",
      profile: apiProfile,
    });
    state = modelProfileReducer(state, {
      type: "MODEL_PROFILE_ADD",
      profile: subscriptionProfile,
    });
    state = modelProfileReducer(state, {
      type: "MODEL_PROFILE_UPDATE",
      profile: { ...apiProfile, model: "vendor/custom" },
    });
    state = modelProfileReducer(state, {
      type: "MODEL_PROFILE_SET_ACTIVE",
      profileId: subscriptionProfile.id,
    });

    expect(state.profiles.find((profile) => profile.id === apiProfile.id)?.model).toBe("vendor/custom");
    expect(getActiveModelProfile(state)?.id).toBe(subscriptionProfile.id);

    state = modelProfileReducer(state, {
      type: "MODEL_PROFILE_REMOVE",
      profileId: subscriptionProfile.id,
    });
    expect(state.profiles.some((profile) => profile.id === subscriptionProfile.id)).toBe(false);
  });

  it("falls back to the first remaining profile after removing the active profile", () => {
    const first = { ...createModelProfile("api_key"), id: "first", active: false };
    const second = { ...createModelProfile("subscription"), id: "second", active: true };
    const state = modelProfileReducer(
      { profiles: [first, second], activeProfileId: "second" },
      { type: "MODEL_PROFILE_REMOVE", profileId: "second" },
    );

    expect(getActiveModelProfile(state)?.id).toBe("first");
  });
});

describe("createModelProfileStore", () => {
  it("persists model profile CRUD through storage", async () => {
    const storage = new MemoryStorage();
    const store = createModelProfileStore({ storage });
    const profile = {
      ...createModelProfile("api_key"),
      id: "custom-api",
      active: true,
      model: "provider/freeform-model",
      apiKeyRef: "local_keychain:custom-secret",
      harnesses: ["openhands_agent_server", "custom_harness"],
    };

    await store.storeProfile(profile);
    await store.setActiveProfile(profile.id);

    const reloaded = createModelProfileStore({ storage });
    const profiles = await reloaded.listProfiles();
    const saved = profiles.find((candidate) => candidate.id === profile.id);

    expect(saved?.model).toBe("provider/freeform-model");
    expect(saved?.harnesses).toContain("custom_harness");
    expect(profiles.find((candidate) => candidate.id === profile.id)?.active).toBe(true);

    await reloaded.removeProfile(profile.id);
    const afterRemove = await reloaded.listProfiles();
    expect(afterRemove.some((candidate) => candidate.id === profile.id)).toBe(false);
    expect(afterRemove.length).toBeGreaterThan(0);
  });

  it("serializes concurrent profile writes", async () => {
    const storage = new MemoryStorage();
    const store = createModelProfileStore({ storage });
    const first = { ...createModelProfile("api_key"), id: "first-concurrent" };
    const second = { ...createModelProfile("subscription"), id: "second-concurrent" };

    await Promise.all([
      store.storeProfile(first),
      store.storeProfile(second),
    ]);

    const profiles = await store.listProfiles();
    expect(profiles.some((profile) => profile.id === first.id)).toBe(true);
    expect(profiles.some((profile) => profile.id === second.id)).toBe(true);
  });

  it("preserves profile order when updating an existing profile", async () => {
    const store = createModelProfileStore({ storage: new MemoryStorage() });
    const before = (await store.listProfiles()).map((profile) => profile.id);
    const [first] = await store.listProfiles();

    await store.storeProfile({ ...first, model: "provider/order-preserved" });

    const profiles = await store.listProfiles();
    expect(profiles.map((profile) => profile.id)).toEqual(before);
    expect(profiles[0].model).toBe("provider/order-preserved");
  });

  it("rejects invalid credential references for all store callers", async () => {
    const store = createModelProfileStore({ storage: new MemoryStorage() });
    const profile = {
      ...createModelProfile("api_key"),
      apiKeyRef: "sk-secret-value-123456789",
    };

    await expect(store.storeProfile(profile)).rejects.toThrow("API key secret");
  });

  it("allows the active model profile to be deactivated", async () => {
    const store = createModelProfileStore({ storage: new MemoryStorage() });
    const [active] = await store.listProfiles();

    await store.storeProfile({ ...active, active: false });

    const state = {
      profiles: await store.listProfiles(),
      activeProfileId: null,
    };
    expect(state.profiles.find((profile) => profile.id === active.id)?.active).toBe(false);
    expect(getActiveModelProfile(state)).toBeNull();
  });

  it("rejects removing an unknown or final model profile", async () => {
    const profile = { ...createModelProfile("api_key"), id: "only-profile" };
    const store = createModelProfileStore({ defaults: [profile] });

    await expect(store.removeProfile("missing-profile")).rejects.toThrow("Unknown model profile");
    await expect(store.removeProfile(profile.id)).rejects.toThrow("Cannot remove the last model profile");
  });
});
