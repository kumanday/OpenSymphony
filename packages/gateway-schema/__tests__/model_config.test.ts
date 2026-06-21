import { describe, expect, it } from "@jest/globals";
import {
  createModelProfile,
  defaultModelProfiles,
  redactCredentialRef,
  validateModelProfileCredentials,
  validateStoredCredentialRef,
  type ModelConfigurationProfile,
} from "@opensymphony/gateway-schema";

describe("model configuration profiles", () => {
  it("ships API-compatible and subscription-backed defaults", () => {
    const profiles = defaultModelProfiles();

    expect(profiles.map((profile) => profile.mode)).toEqual([
      "api_key",
      "subscription",
    ]);
    expect(profiles[0].baseUrl).toBe("https://api.openai.com/v1");
    expect(profiles[0].apiKeyRef).toBeNull();
    expect(profiles[1].subscriptionCredential?.provider).toBe("openai");
    expect(profiles[1].subscriptionCredential?.authDirectoryEnv).toBe("OPENHANDS_AUTH_DIR");
    expect(profiles[1].harnesses).toEqual([
      "openhands_agent_server",
      "codex_app_server",
    ]);
  });

  it("preserves arbitrary provider model strings and harness compatibility", () => {
    const profile: ModelConfigurationProfile = {
      ...createModelProfile("api_key"),
      model: "vendor/custom-model-2026-06-20",
      harnesses: ["openhands_agent_server", "custom_harness"],
    };

    expect(profile.model).toBe("vendor/custom-model-2026-06-20");
    expect(profile.harnesses).toContain("custom_harness");
  });

  it("redacts credential references for display", () => {
    expect(redactCredentialRef("local_keychain:openai-api-key")).toBe("Configured");
    expect(redactCredentialRef("short")).toBe("Configured");
    expect(redactCredentialRef(null)).toBe("Not configured");
  });

  it("validates stored credential references with strict backend prefixes", () => {
    expect(validateStoredCredentialRef("local_keychain:openai-api-key", "local_keychain")).toBeNull();
    expect(validateStoredCredentialRef("sk-secret-value-123456789", "local_keychain")).toContain("local_keychain:");
    expect(validateStoredCredentialRef("openhands_auth:openai", "local_keychain")).toContain("local_keychain:");
  });

  it("validates credential-bearing profile fields for all callers", () => {
    const apiProfile = {
      ...createModelProfile("api_key"),
      apiKeyRef: "local_keychain:custom-secret",
    };
    const subscriptionProfile = {
      ...createModelProfile("subscription"),
      subscriptionCredential: {
        provider: "openai",
        authDirectoryEnv: "OPENHANDS_AUTH_DIR",
        authMethod: "device_code" as const,
        openBrowser: false,
        forceLogin: false,
      },
    };

    expect(validateModelProfileCredentials(apiProfile)).toBeNull();
    expect(validateModelProfileCredentials(subscriptionProfile)).toBeNull();
    expect(validateModelProfileCredentials({
      ...subscriptionProfile,
      credentialStorage: "local_keychain",
    })).toContain("openhands_auth_directory");
    expect(validateModelProfileCredentials({
      ...apiProfile,
      subscriptionCredential: subscriptionProfile.subscriptionCredential,
    })).toContain("must not store subscription credential");
    expect(validateModelProfileCredentials({
      ...subscriptionProfile,
      subscriptionCredential: {
        ...subscriptionProfile.subscriptionCredential,
        authDirectoryEnv: "not-valid",
      },
    })).toContain("environment variable");
  });
});
