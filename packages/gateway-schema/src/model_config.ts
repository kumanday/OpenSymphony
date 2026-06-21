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

/** Supported subscription credential bootstrap methods. */
export type SubscriptionCredentialAuthMethod =
  | "browser"
  | "device_code"
  | "cached";

/** Bootstrap metadata for subscription-backed model access. */
export interface SubscriptionCredentialBootstrap {
  provider: string;
  /** Environment variable that points at an auth directory, never a raw token. */
  authDirectoryEnv: string | null;
  authMethod: SubscriptionCredentialAuthMethod;
  openBrowser: boolean;
  forceLogin: boolean;
  /** Optional account identity header name, not the resolved account value. */
  accountIdentityHeader?: string | null;
}

/** Model settings saved by operators. */
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
  /** Structured subscription bootstrap metadata, never raw OAuth material. */
  subscriptionCredential?: SubscriptionCredentialBootstrap | null;
  credentialStorage: CredentialStorage;
  harnesses: ModelHarnessKind[];
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
      model: "gpt-5.5",
      apiKeyRef: null,
      subscriptionCredential: null,
      credentialStorage: "local_keychain",
      harnesses: ["openhands_agent_server"],
    },
    {
      id: "openai-subscription",
      label: "OpenAI subscription",
      active: false,
      mode: "subscription",
      owner: "user",
      baseUrl: "",
      model: "gpt-5.5",
      apiKeyRef: null,
      subscriptionCredential: {
        provider: "openai",
        authDirectoryEnv: "OPENHANDS_AUTH_DIR",
        authMethod: "device_code",
        openBrowser: false,
        forceLogin: false,
        accountIdentityHeader: null,
      },
      credentialStorage: "openhands_auth_directory",
      harnesses: ["openhands_agent_server", "codex_app_server"],
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

/** Return the required stored-reference prefix for a credential backend. */
export function credentialReferencePrefix(storage: CredentialStorage): string {
  switch (storage) {
    case "openhands_auth_directory":
      return "openhands_auth:";
    case "hosted_secret_store":
      return "hosted_secret:";
    case "local_keychain":
    default:
      return "local_keychain:";
  }
}

/** Validate an API-key credential reference without accepting raw secrets. */
export function validateStoredCredentialRef(
  value: string | null | undefined,
  storage: CredentialStorage,
): string | null {
  const trimmed = value?.trim();
  if (!trimmed) {
    return null;
  }
  const prefix = credentialReferencePrefix(storage);
  const escapedPrefix = prefix.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const referencePattern = new RegExp(`^${escapedPrefix}[A-Za-z0-9._/@-]{1,128}$`);
  if (!referencePattern.test(trimmed)) {
    return `API key secret must use ${prefix}<name>`;
  }
  return null;
}

/** Validate subscription bootstrap metadata. */
export function validateSubscriptionCredential(
  value: SubscriptionCredentialBootstrap | null | undefined,
): string | null {
  if (!value) {
    return null;
  }
  if (!value.provider.trim()) {
    return "Subscription provider is required";
  }
  if (
    value.authDirectoryEnv
    && !/^[A-Z_][A-Z0-9_]{0,127}$/.test(value.authDirectoryEnv)
  ) {
    return "Subscription auth directory env must be an environment variable name";
  }
  return null;
}

/** Validate credential-bearing portions of a model profile for all callers. */
export function validateModelProfileCredentials(
  profile: ModelConfigurationProfile,
): string | null {
  if (profile.mode === "api_key") {
    if (profile.subscriptionCredential) {
      return "API key profiles must not store subscription credential metadata";
    }
    return validateStoredCredentialRef(profile.apiKeyRef, profile.credentialStorage);
  }
  if (profile.credentialStorage !== "openhands_auth_directory") {
    return "Subscription profiles must use openhands_auth_directory credential storage";
  }
  if (profile.apiKeyRef) {
    return "Subscription profiles must not store API key references";
  }
  return validateSubscriptionCredential(profile.subscriptionCredential);
}
