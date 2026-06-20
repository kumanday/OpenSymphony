/** Model configuration profiles for provider-aware execution choices. */

/** Supported credential mode for a model profile. */
export type ModelCredentialMode = "api_key" | "subscription";

/** Scope that owns a model profile or credential reference. */
export type ModelProfileOwner = "user" | "organization" | "project";

/** Storage location for the credential material referenced by a profile. */
export type CredentialStorage =
  | "local_keychain"
  | "openhands_auth_directory"
  | "hosted_secret_store";

/** Public harness kind strings that can consume a model profile. */
export type ModelHarnessKind =
  | "openhands_agent_server"
  | "codex_app_server"
  | "rust_native"
  | (string & {});

/** Optional task-shape recommendations for routing policy inputs. */
export type ModelTaskRecommendation =
  | "planning"
  | "implementation"
  | "refactor"
  | "debugging"
  | "testing"
  | "validation"
  | "documentation"
  | "browser_verification"
  | "code_review"
  | (string & {});

/** Optional reasoning effort hint. Provider-specific values remain allowed. */
export type ModelReasoningEffort =
  | "provider_default"
  | "none"
  | "low"
  | "medium"
  | "high"
  | (string & {});

/** Operator-supplied metadata used as future routing inputs. */
export interface ModelRoutingMetadata {
  /** Optional context window size in tokens. */
  contextWindowTokens?: number | null;
  /** Optional reasoning effort hint, not constrained to one provider enum. */
  reasoningEffort?: ModelReasoningEffort | null;
  /** Optional human-readable cost profile such as "low", "premium", or "$/1M". */
  costProfile?: string | null;
  /** Optional task types where this profile is a good default. */
  recommendedFor: ModelTaskRecommendation[];
}

/** Model settings and routing metadata saved by operators. */
export interface ModelConfigurationProfile {
  id: string;
  label: string;
  active: boolean;
  mode: ModelCredentialMode;
  owner: ModelProfileOwner;
  /** API-compatible base URL. Subscription profiles may inherit provider defaults. */
  baseUrl: string;
  /** Provider model string; intentionally arbitrary for API-compatible endpoints. */
  model: string;
  /** Reference to a stored API key, never the raw key. */
  apiKeyRef?: string | null;
  /** Reference to a stored subscription credential, never raw OAuth material. */
  subscriptionCredentialRef?: string | null;
  /** Provider for subscription profiles, for example "openai". */
  subscriptionProvider?: string | null;
  credentialStorage: CredentialStorage;
  harnesses: ModelHarnessKind[];
  metadata: ModelRoutingMetadata;
}

let _modelProfileIdCounter = 0;

/** Default model profiles that show both supported credential modes. */
export function defaultModelProfiles(): ModelConfigurationProfile[] {
  return [
    {
      id: "openai-api-compatible",
      label: "OpenAI API-compatible",
      active: true,
      mode: "api_key",
      owner: "user",
      baseUrl: "https://api.openai.com/v1",
      model: "gpt-4.1",
      apiKeyRef: null,
      subscriptionCredentialRef: null,
      subscriptionProvider: null,
      credentialStorage: "local_keychain",
      harnesses: ["openhands_agent_server"],
      metadata: {
        contextWindowTokens: null,
        reasoningEffort: "provider_default",
        costProfile: null,
        recommendedFor: ["implementation", "debugging", "testing"],
      },
    },
    {
      id: "openai-subscription",
      label: "OpenAI subscription",
      active: false,
      mode: "subscription",
      owner: "user",
      baseUrl: "",
      model: "codex",
      apiKeyRef: null,
      subscriptionCredentialRef: null,
      subscriptionProvider: "openai",
      credentialStorage: "openhands_auth_directory",
      harnesses: ["openhands_agent_server", "codex_app_server"],
      metadata: {
        contextWindowTokens: null,
        reasoningEffort: "provider_default",
        costProfile: null,
        recommendedFor: ["implementation", "code_review"],
      },
    },
  ];
}

/** Create a profile with stable defaults while preserving arbitrary strings. */
export function createModelProfile(
  mode: ModelCredentialMode = "api_key",
): ModelConfigurationProfile {
  _modelProfileIdCounter++;
  const template = defaultModelProfiles().find((profile) => profile.mode === mode)
    ?? defaultModelProfiles()[0];
  return {
    ...template,
    id: `${mode}-model-${Date.now()}-${_modelProfileIdCounter}`,
    label: mode === "subscription" ? "Subscription model" : "API-compatible model",
    active: false,
    metadata: {
      ...template.metadata,
      recommendedFor: [...template.metadata.recommendedFor],
    },
    harnesses: [...template.harnesses],
  };
}

/** Return a display-safe credential reference. */
export function redactCredentialRef(value: string | null | undefined): string {
  const trimmed = value?.trim();
  if (!trimmed) {
    return "Not configured";
  }
  return "Configured";
}
