import { describe, expect, it } from "@jest/globals";
import {
  createModelProfile,
  defaultModelProfiles,
  redactCredentialRef,
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
    expect(profiles[0].apiKeyRef).toBe("local_keychain:openai-api-key");
    expect(profiles[1].subscriptionCredentialRef).toBe("openhands_auth:openai");
    expect(profiles[1].harnesses).toEqual([
      "openhands_agent_server",
      "codex_app_server",
    ]);
  });

  it("preserves arbitrary provider model and routing metadata strings", () => {
    const profile: ModelConfigurationProfile = {
      ...createModelProfile("api_key"),
      model: "vendor/custom-model-2026-06-20",
      harnesses: ["openhands_agent_server", "custom_harness"],
      metadata: {
        contextWindowTokens: 123456,
        reasoningEffort: "provider-specific-ultra",
        costProfile: "enterprise-tier",
        recommendedFor: ["implementation", "specialized-audit"],
      },
    };

    expect(profile.model).toBe("vendor/custom-model-2026-06-20");
    expect(profile.harnesses).toContain("custom_harness");
    expect(profile.metadata.reasoningEffort).toBe("provider-specific-ultra");
    expect(profile.metadata.recommendedFor).toContain("specialized-audit");
  });

  it("redacts credential references for display", () => {
    expect(redactCredentialRef("local_keychain:openai-api-key")).toBe("loca...-key");
    expect(redactCredentialRef("short")).toBe("****");
    expect(redactCredentialRef(null)).toBe("Not configured");
  });
});
