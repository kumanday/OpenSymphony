import { describe, expect, it } from "@jest/globals";
import {
  createModelProfile,
  defaultModelProfiles,
  type ModelConfigurationProfile,
} from "@opensymphony/gateway-schema";
import {
  createAsyncModelProfileStore,
  createModelProfileStore,
  getActiveModelProfile,
  initialModelProfileState,
  modelProfileReducer,
} from "../src/model-profiles";

class MemoryStorage implements Pick<Storage, "getItem" | "setItem" | "removeItem"> {
  private readonly values = new Map<string, string>();
  failWrites = false;

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    if (this.failWrites) {
      throw new Error("storage write failed");
    }
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
      harnesses: ["openhands_agent_server", "codex_app_server"],
    };

    await store.storeProfile(profile);
    await store.setActiveProfile(profile.id);

    const reloaded = createModelProfileStore({ storage });
    const profiles = await reloaded.listProfiles();
    const saved = profiles.find((candidate) => candidate.id === profile.id);

    expect(saved?.model).toBe("provider/freeform-model");
    expect(saved?.harnesses).toContain("codex_app_server");
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

  it("rejects mismatched subscription storage for all store callers", async () => {
    const store = createModelProfileStore({ storage: new MemoryStorage() });
    const profile = {
      ...createModelProfile("subscription"),
      credentialStorage: "local_keychain" as const,
    };

    await expect(store.storeProfile(profile)).rejects.toThrow("openhands_auth_directory");
  });

  it("quarantines malformed stored profiles before returning UI state", async () => {
    const storage = new MemoryStorage();
    const goodProfile = {
      ...createModelProfile("api_key"),
      id: "safe-api",
      model: "provider/safe",
      apiKeyRef: "local_keychain:safe-api-key",
      active: true,
      routingPolicy: { tier: "fast" },
      providerConfig: {
        apiKey: "sk-secret-nested",
        region: "us-east-1",
        tokenEndpoint: "https://auth.example/token",
        credentialId: "public-credential-id",
        oauthProvider: "openai",
        tokens: ["scope:read"],
      },
      nestedList: [{ oauth_token: "oauth-secret", label: "kept" }],
      token: "should-not-survive",
    };
    storage.setItem("opensymphony.modelProfiles.v1", JSON.stringify({
      profiles: [
        goodProfile,
        {
          ...createModelProfile("api_key"),
          id: "raw-secret",
          apiKeyRef: "sk-secret-value-123456789",
          active: true,
        },
        {
          ...createModelProfile("subscription"),
          id: "wrong-storage",
          credentialStorage: "local_keychain",
        },
        { id: "missing-mode" },
      ],
      activeProfileId: "raw-secret",
    }));
    const quarantineReasons: string[] = [];
    const store = createModelProfileStore({
      storage,
      onQuarantine: (reason) => quarantineReasons.push(reason),
    });

    const profiles = await store.listProfiles();

    expect(profiles.map((profile) => profile.id)).toEqual(["safe-api"]);
    expect(profiles[0].active).toBe(true);
    expect((profiles[0] as ModelConfigurationProfile & { routingPolicy?: unknown }).routingPolicy).toEqual({
      tier: "fast",
    });
    expect((profiles[0] as ModelConfigurationProfile & { providerConfig?: unknown }).providerConfig).toEqual({
      region: "us-east-1",
      tokenEndpoint: "https://auth.example/token",
      credentialId: "public-credential-id",
      oauthProvider: "openai",
      tokens: ["scope:read"],
    });
    expect((profiles[0] as ModelConfigurationProfile & { nestedList?: unknown }).nestedList).toEqual([
      { label: "kept" },
    ]);
    expect((profiles[0] as ModelConfigurationProfile & { token?: unknown }).token).toBeUndefined();
    expect(quarantineReasons).toEqual(expect.arrayContaining([
      expect.stringContaining("raw-secret"),
      expect.stringContaining("wrong-storage"),
      expect.stringContaining("safe-api: providerConfig.apiKey"),
      expect.stringContaining("safe-api: nestedList.0.oauth_token"),
      expect.stringContaining("safe-api: token"),
      "Dropped malformed model profile from durable storage",
    ]));
  });

  it("warns when the durable profile list is not an array", async () => {
    const quarantineReasons: string[] = [];
    const store = createAsyncModelProfileStore({
      async load() {
        return JSON.stringify({ profiles: { bad: true }, activeProfileId: null });
      },
      async save() {
        return undefined;
      },
      onQuarantine: (reason) => quarantineReasons.push(reason),
    });

    const profiles = await store.listProfiles();

    expect(profiles.length).toBeGreaterThan(0);
    expect(quarantineReasons).toContain("Dropped malformed model profile list from durable storage");
  });

  it("keeps the last valid sync state after a transient storage read failure", async () => {
    const storage = new MemoryStorage();
    const validProfile = {
      ...createModelProfile("api_key"),
      id: "valid-before-error",
      model: "provider/valid-before-error",
    };
    storage.setItem("opensymphony.modelProfiles.v1", JSON.stringify({
      profiles: [validProfile],
      activeProfileId: validProfile.id,
    }));
    const store = createModelProfileStore({ storage });

    await expect(store.listProfiles()).resolves.toEqual([
      expect.objectContaining({ id: validProfile.id }),
    ]);
    storage.setItem("opensymphony.modelProfiles.v1", "{not-json");

    await expect(store.listProfiles()).resolves.toEqual([
      expect.objectContaining({ id: validProfile.id }),
    ]);
  });

  it("does not update sync fallback when durable writes fail", async () => {
    const storage = new MemoryStorage();
    const originalProfile = {
      ...createModelProfile("api_key"),
      id: "saved-before-write-failure",
      model: "provider/original",
    };
    storage.setItem("opensymphony.modelProfiles.v1", JSON.stringify({
      profiles: [originalProfile],
      activeProfileId: originalProfile.id,
    }));
    const store = createModelProfileStore({ storage });

    await expect(store.listProfiles()).resolves.toEqual([
      expect.objectContaining({ id: originalProfile.id }),
    ]);
    storage.failWrites = true;
    await expect(store.storeProfile({
      ...originalProfile,
      model: "provider/not-saved",
    })).rejects.toThrow("storage write failed");
    storage.failWrites = false;

    await expect(store.listProfiles()).resolves.toEqual([
      expect.objectContaining({ id: originalProfile.id, model: "provider/original" }),
    ]);
  });

  it("quarantines profiles with conflicting mode-specific credential fields", async () => {
    const storage = new MemoryStorage();
    storage.setItem("opensymphony.modelProfiles.v1", JSON.stringify({
      profiles: [
        {
          ...createModelProfile("api_key"),
          id: "api-with-subscription",
          subscriptionCredential: {
            provider: "openai",
          },
        },
        {
          ...createModelProfile("subscription"),
          id: "subscription-with-api-key",
          apiKeyRef: "local_keychain:legacy",
        },
      ],
    }));
    const quarantineReasons: string[] = [];
    const store = createModelProfileStore({
      storage,
      onQuarantine: (reason) => quarantineReasons.push(reason),
    });

    const profiles = await store.listProfiles();

    expect(profiles.map((profile) => profile.id)).toEqual(
      defaultModelProfiles().map((profile) => profile.id),
    );
    expect(quarantineReasons).toEqual(expect.arrayContaining([
      expect.stringContaining("api-with-subscription"),
      expect.stringContaining("subscription-with-api-key"),
    ]));
  });

  it("reports specific quarantine reasons for malformed core profile fields", async () => {
    const storage = new MemoryStorage();
    storage.setItem("opensymphony.modelProfiles.v1", JSON.stringify({
      profiles: [
        { id: "invalid-mode", mode: "api_key_typo" },
        { mode: "api_key" },
        {
          ...createModelProfile("api_key"),
          id: "invalid-storage",
          credentialStorage: "plain_text",
        },
      ],
    }));
    const quarantineReasons: string[] = [];
    const store = createModelProfileStore({
      storage,
      onQuarantine: (reason) => quarantineReasons.push(reason),
    });

    await store.listProfiles();

    expect(quarantineReasons).toEqual(expect.arrayContaining([
      "Dropped model profile with invalid mode: api_key_typo",
      "Dropped model profile with missing id",
      "Dropped invalid model profile invalid-storage: invalid credential storage plain_text",
    ]));
  });

  it("drops profiles when persisted harnesses contain no known harness kinds", async () => {
    const storage = new MemoryStorage();
    storage.setItem("opensymphony.modelProfiles.v1", JSON.stringify({
      profiles: [{
        ...createModelProfile("api_key"),
        id: "invalid-harnesses",
        harnesses: ["openhands_agent_server_typo"],
      }],
    }));
    const quarantineReasons: string[] = [];
    const store = createModelProfileStore({
      storage,
      onQuarantine: (reason) => quarantineReasons.push(reason),
    });

    const profiles = await store.listProfiles();

    expect(profiles.map((profile) => profile.id)).toEqual(
      defaultModelProfiles().map((profile) => profile.id),
    );
    expect(quarantineReasons).toEqual(expect.arrayContaining([
      expect.stringContaining("invalid-harnesses"),
      "Dropped malformed model profile from durable storage",
    ]));
  });

  it("warns and uses defaults when persisted harnesses are not an array", async () => {
    const storage = new MemoryStorage();
    storage.setItem("opensymphony.modelProfiles.v1", JSON.stringify({
      profiles: [{
        ...createModelProfile("api_key"),
        id: "non-array-harnesses",
        harnesses: "openhands_agent_server",
      }],
    }));
    const quarantineReasons: string[] = [];
    const store = createModelProfileStore({
      storage,
      onQuarantine: (reason) => quarantineReasons.push(reason),
    });

    const profiles = await store.listProfiles();
    const saved = profiles.find((profile) => profile.id === "non-array-harnesses");

    expect(saved?.harnesses).toEqual(["openhands_agent_server"]);
    expect(quarantineReasons).toContain(
      "Dropped malformed model profile harnesses for non-array-harnesses: expected array",
    );
  });

  it("uses the same CRUD path for async durable storage", async () => {
    let stored: string | null = null;
    const quarantineReasons: string[] = [];
    const store = createAsyncModelProfileStore({
      async load() {
        return stored;
      },
      async save(value) {
        stored = value;
      },
      onQuarantine: (reason) => quarantineReasons.push(reason),
    });
    const profile = {
      ...createModelProfile("subscription"),
      id: "async-subscription",
      active: true,
      model: "codex-async",
      baseUrl: "https://subscription.example/v1",
      harnesses: ["openhands_agent_server", "codex_app_server"],
    };

    await store.storeProfile(profile);
    await store.setActiveProfile(profile.id);

    const restored = createAsyncModelProfileStore({
      async load() {
        return stored;
      },
      async save(value) {
        stored = value;
      },
      onQuarantine: (reason) => quarantineReasons.push(reason),
    });
    const profiles = await restored.listProfiles();
    const saved = profiles.find((candidate) => candidate.id === profile.id);

    expect(saved).toMatchObject({
      active: true,
      model: "codex-async",
      baseUrl: "https://subscription.example/v1",
    });
    expect(saved?.harnesses).toEqual(["openhands_agent_server", "codex_app_server"]);
    expect(quarantineReasons).toEqual([]);
  });

  it("does not update async fallback when durable writes fail", async () => {
    let stored: string | null = null;
    let failWrites = false;
    const originalProfile = {
      ...createModelProfile("subscription"),
      id: "async-before-write-failure",
      model: "codex-original",
    };
    stored = JSON.stringify({
      profiles: [originalProfile],
      activeProfileId: originalProfile.id,
    });
    const store = createAsyncModelProfileStore({
      async load() {
        return stored;
      },
      async save(value) {
        if (failWrites) {
          throw new Error("async write failed");
        }
        stored = value;
      },
    });

    await expect(store.listProfiles()).resolves.toEqual([
      expect.objectContaining({ id: originalProfile.id }),
    ]);
    failWrites = true;
    await expect(store.storeProfile({
      ...originalProfile,
      model: "codex-not-saved",
    })).rejects.toThrow("async write failed");
    failWrites = false;

    await expect(store.listProfiles()).resolves.toEqual([
      expect.objectContaining({ id: originalProfile.id, model: "codex-original" }),
    ]);
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
