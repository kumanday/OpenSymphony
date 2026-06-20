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
      metadata: {
        contextWindowTokens: 200000,
        reasoningEffort: "provider-ultra",
        costProfile: "premium",
        recommendedFor: ["implementation", "validation"],
      },
    };

    await store.storeProfile(profile);
    await store.setActiveProfile(profile.id);

    const reloaded = createModelProfileStore({ storage });
    const profiles = await reloaded.listProfiles();
    const saved = profiles.find((candidate) => candidate.id === profile.id);

    expect(saved?.model).toBe("provider/freeform-model");
    expect(saved?.harnesses).toContain("custom_harness");
    expect(saved?.metadata.reasoningEffort).toBe("provider-ultra");
    expect(profiles.find((candidate) => candidate.id === profile.id)?.active).toBe(true);
  });
});
