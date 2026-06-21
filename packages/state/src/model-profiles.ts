/** Reducer and storage helpers for model configuration profiles. */

import {
  createModelProfile,
  defaultModelProfiles,
  validateModelProfileCredentials,
  type CredentialStorage,
  type ModelConfigurationProfile,
  type ModelCredentialMode,
  type ModelHarnessKind,
  type ModelProfileOwner,
  type SubscriptionCredentialAuthMethod,
  type SubscriptionCredentialBootstrap,
} from "@opensymphony/gateway-schema";

/** Small storage surface shared by browser and desktop webviews. */
export type ModelProfileStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;

/** Model configuration state slice. */
export interface ModelProfileState {
  profiles: ModelConfigurationProfile[];
  activeProfileId: string | null;
}

/** Initial model profile state. */
export const initialModelProfileState: ModelProfileState = {
  profiles: [],
  activeProfileId: null,
};

/** Model profile CRUD actions. */
export type ModelProfileAction =
  | { type: "MODEL_PROFILES_LOAD"; profiles: ModelConfigurationProfile[] }
  | { type: "MODEL_PROFILE_ADD"; profile: ModelConfigurationProfile }
  | { type: "MODEL_PROFILE_UPDATE"; profile: ModelConfigurationProfile }
  | { type: "MODEL_PROFILE_REMOVE"; profileId: string }
  | { type: "MODEL_PROFILE_SET_ACTIVE"; profileId: string }
  | { type: "MODEL_PROFILES_RESET" };

/** Apply model profile CRUD actions. */
export function modelProfileReducer(
  state: ModelProfileState,
  action: ModelProfileAction,
): ModelProfileState {
  switch (action.type) {
    case "MODEL_PROFILES_LOAD":
      return normalizeModelProfileState({
        profiles: action.profiles,
        activeProfileId:
          action.profiles.find((profile) => profile.active)?.id
          ?? state.activeProfileId,
      });
    case "MODEL_PROFILE_ADD":
      return normalizeModelProfileState({
        ...state,
        profiles: [...state.profiles, action.profile],
      });
    case "MODEL_PROFILE_UPDATE":
      return normalizeModelProfileState({
        ...state,
        profiles: state.profiles.map((profile) =>
          profile.id === action.profile.id ? action.profile : profile,
        ),
      });
    case "MODEL_PROFILE_REMOVE": {
      const remainingProfiles = state.profiles.filter((profile) => profile.id !== action.profileId);
      return normalizeModelProfileState({
        profiles: remainingProfiles,
        activeProfileId:
          state.activeProfileId === action.profileId
            ? remainingProfiles.find((profile) => profile.active)?.id
              ?? remainingProfiles[0]?.id
              ?? null
            : state.activeProfileId,
      });
    }
    case "MODEL_PROFILE_SET_ACTIVE":
      if (!state.profiles.some((profile) => profile.id === action.profileId)) {
        return state;
      }
      return normalizeModelProfileState({
        profiles: state.profiles.map((profile) => ({
          ...profile,
          active: profile.id === action.profileId,
        })),
        activeProfileId: action.profileId,
      });
    case "MODEL_PROFILES_RESET":
      return initialModelProfileState;
    default:
      return state;
  }
}

/** Return the active model profile. */
export function getActiveModelProfile(
  state: ModelProfileState,
): ModelConfigurationProfile | null {
  return state.profiles.find((profile) => profile.id === state.activeProfileId)
    ?? state.profiles.find((profile) => profile.active)
    ?? null;
}

/** Normalize active flags and fill empty profile lists with defaults. */
export function normalizeModelProfileState(
  state: ModelProfileState,
): ModelProfileState {
  const profiles = state.profiles.length > 0
    ? state.profiles
    : defaultModelProfiles();
  const activeProfileId = state.activeProfileId
    && profiles.some((profile) => profile.id === state.activeProfileId)
    ? state.activeProfileId
    : profiles.find((profile) => profile.active)?.id ?? null;
  return {
    profiles: profiles.map((profile) => ({
      ...profile,
      active: profile.id === activeProfileId,
      harnesses: [...profile.harnesses],
    })),
    activeProfileId,
  };
}

/** CRUD service that persists only when a storage backend is supplied. */
export interface ModelProfileStore {
  listProfiles(): Promise<ModelConfigurationProfile[]>;
  storeProfile(profile: ModelConfigurationProfile): Promise<ModelConfigurationProfile>;
  setActiveProfile(profileId: string): Promise<ModelConfigurationProfile>;
  removeProfile(profileId: string): Promise<ModelConfigurationProfile[]>;
}

export interface ModelProfileStoreOptions {
  storage?: ModelProfileStorage | null;
  storageKey?: string;
  defaults?: ModelConfigurationProfile[];
  onQuarantine?: (reason: string) => void;
}

export interface AsyncModelProfileStoreOptions {
  load(): Promise<string | null>;
  save(value: string): Promise<void>;
  defaults?: ModelConfigurationProfile[];
  onQuarantine?: (reason: string) => void;
}

const DEFAULT_STORAGE_KEY = "opensymphony.modelProfiles.v1";

const credentialStorages = new Set<CredentialStorage>([
  "local_keychain",
  "openhands_auth_directory",
  "hosted_secret_store",
]);

const credentialModes = new Set<ModelCredentialMode>(["api_key", "subscription"]);
const owners = new Set<ModelProfileOwner>(["user", "organization", "project"]);
const authMethods = new Set<SubscriptionCredentialAuthMethod>([
  "browser",
  "device_code",
  "cached",
]);
const harnessKinds = new Set<ModelHarnessKind>([
  "openhands_agent_server",
  "codex_app_server",
  "rust_native",
]);

const modelProfileKnownFields = new Set([
  "id",
  "label",
  "active",
  "mode",
  "owner",
  "baseUrl",
  "model",
  "apiKeyRef",
  "subscriptionCredential",
  "credentialStorage",
  "harnesses",
]);

export function createModelProfileStore(
  options: ModelProfileStoreOptions = {},
): ModelProfileStore {
  const storage = options.storage ?? null;
  const storageKey = options.storageKey ?? DEFAULT_STORAGE_KEY;
  const defaults = sanitizeModelProfiles(
    options.defaults ?? defaultModelProfiles(),
    options.onQuarantine,
  );
  let fallback = normalizeModelProfileState({
    profiles: defaults,
    activeProfileId: null,
  });

  function read(): ModelProfileState {
    if (!storage) {
      return fallback;
    }
    try {
      const raw = storage.getItem(storageKey);
      if (!raw) {
        return fallback;
      }
      const parsed = JSON.parse(raw) as Partial<ModelProfileState>;
      const profiles = sanitizeModelProfiles(parsed.profiles, options.onQuarantine);
      const state = normalizeModelProfileState({
        profiles: profiles.length > 0 ? profiles : fallback.profiles,
        activeProfileId: parsed.activeProfileId ?? null,
      });
      fallback = state;
      return state;
    } catch (error) {
      options.onQuarantine?.(`Failed to read stored model profiles: ${errorMessage(error)}`);
      return fallback;
    }
  }

  function write(state: ModelProfileState): ModelProfileState {
    const normalized = normalizeModelProfileState(state);
    if (storage) {
      storage.setItem(storageKey, JSON.stringify(normalized));
    }
    fallback = normalized;
    return normalized;
  }

  return createModelProfileStoreFromStateAccess({
    read,
    write,
  });
}

export function createAsyncModelProfileStore(
  options: AsyncModelProfileStoreOptions,
): ModelProfileStore {
  const defaults = sanitizeModelProfiles(
    options.defaults ?? defaultModelProfiles(),
    options.onQuarantine,
  );
  let fallback = normalizeModelProfileState({
    profiles: defaults,
    activeProfileId: null,
  });

  async function read(): Promise<ModelProfileState> {
    try {
      const raw = await options.load();
      if (!raw) {
        return fallback;
      }
      const parsed = JSON.parse(raw) as Partial<ModelProfileState>;
      const profiles = sanitizeModelProfiles(parsed.profiles, options.onQuarantine);
      const state = normalizeModelProfileState({
        profiles: profiles.length > 0 ? profiles : fallback.profiles,
        activeProfileId: parsed.activeProfileId ?? null,
      });
      fallback = state;
      return state;
    } catch (error) {
      options.onQuarantine?.(`Failed to read stored model profiles: ${errorMessage(error)}`);
      return fallback;
    }
  }

  async function write(state: ModelProfileState): Promise<ModelProfileState> {
    const normalized = normalizeModelProfileState(state);
    await options.save(JSON.stringify(normalized));
    fallback = normalized;
    return normalized;
  }

  return createModelProfileStoreFromStateAccess({
    read,
    write,
  });
}

function createModelProfileStoreFromStateAccess(access: {
  read(): ModelProfileState | Promise<ModelProfileState>;
  write(state: ModelProfileState): ModelProfileState | Promise<ModelProfileState>;
}): ModelProfileStore {
  let writeQueue: Promise<unknown> = Promise.resolve();

  function serialize<T>(operation: () => T): Promise<T> {
    const next = writeQueue.then(operation, operation);
    writeQueue = next.catch(() => undefined);
    return next;
  }

  return {
    async listProfiles() {
      return (await access.read()).profiles;
    },
    async storeProfile(profile) {
      return serialize(async () => {
        const validationError = validateModelProfileCredentials(profile);
        if (validationError) {
          throw new Error(validationError);
        }
        const current = await access.read();
        const index = current.profiles.findIndex((candidate) => candidate.id === profile.id);
        const profiles = [...current.profiles];
        if (index >= 0) {
          profiles[index] = profile;
        } else {
          profiles.push(profile);
        }
        const next = await access.write({
          profiles,
          activeProfileId: profile.active
            ? profile.id
            : current.activeProfileId === profile.id ? null : current.activeProfileId,
        });
        return next.profiles.find((candidate) => candidate.id === profile.id)!;
      });
    },
    async setActiveProfile(profileId) {
      return serialize(async () => {
        const current = await access.read();
        if (!current.profiles.some((profile) => profile.id === profileId)) {
          throw new Error(`Unknown model profile: ${profileId}`);
        }
        const next = await access.write({
          profiles: current.profiles,
          activeProfileId: profileId,
        });
        return next.profiles.find((profile) => profile.id === profileId)!;
      });
    },
    async removeProfile(profileId) {
      return serialize(async () => {
        const current = await access.read();
        if (!current.profiles.some((profile) => profile.id === profileId)) {
          throw new Error(`Unknown model profile: ${profileId}`);
        }
        if (current.profiles.length <= 1) {
          throw new Error("Cannot remove the last model profile");
        }
        const profiles = current.profiles.filter((profile) => profile.id !== profileId);
        const next = await access.write({
          profiles,
          activeProfileId:
            current.activeProfileId === profileId
              ? profiles.find((profile) => profile.active)?.id ?? profiles[0]?.id ?? null
              : current.activeProfileId,
        });
        return next.profiles;
      });
    },
  };
}

export function sanitizeModelProfiles(
  value: unknown,
  onQuarantine?: (reason: string) => void,
): ModelConfigurationProfile[] {
  if (!Array.isArray(value)) {
    if (value !== undefined) {
      onQuarantine?.("Dropped malformed model profile list from durable storage");
    }
    return [];
  }
  return value.flatMap((profile) => {
    const sanitized = sanitizeModelProfile(profile, onQuarantine);
    if (!sanitized) {
      onQuarantine?.("Dropped malformed model profile from durable storage");
      return [];
    }
    const validationError = validateModelProfileCredentials(sanitized);
    if (validationError) {
      onQuarantine?.(`Dropped invalid model profile ${sanitized.id}: ${validationError}`);
      return [];
    }
    return [sanitized];
  });
}

function sanitizeModelProfile(
  value: unknown,
  onQuarantine?: (reason: string) => void,
): ModelConfigurationProfile | null {
  const record = objectRecord(value);
  if (!record) {
    return null;
  }
  const mode = stringUnion(record.mode, credentialModes);
  if (!mode) {
    onQuarantine?.(`Dropped model profile with invalid mode: ${String(record.mode)}`);
    return null;
  }
  const template = createModelProfile(mode);
  const id = stringField(record, "id");
  if (!id) {
    onQuarantine?.("Dropped model profile with missing id");
    return null;
  }
  const credentialStorage =
    record.credentialStorage === undefined
      ? template.credentialStorage
      : stringUnion(record.credentialStorage, credentialStorages);
  if (!credentialStorage) {
    onQuarantine?.(`Dropped invalid model profile ${id}: invalid credential storage ${String(record.credentialStorage)}`);
    return null;
  }
  const subscriptionCredential = mode === "subscription"
    ? sanitizeSubscriptionCredential(record.subscriptionCredential)
      ?? template.subscriptionCredential
      ?? null
    : null;
  if (
    mode === "api_key"
    && record.subscriptionCredential !== undefined
    && record.subscriptionCredential !== null
  ) {
    onQuarantine?.(`Dropped invalid model profile ${id}: API key profiles must not store subscription credential metadata`);
    return null;
  }
  const apiKeyRef = mode === "api_key"
    ? nullableString(record.apiKeyRef)
    : null;
  if (mode === "subscription" && record.apiKeyRef !== undefined && record.apiKeyRef !== null) {
    onQuarantine?.(`Dropped invalid model profile ${id}: Subscription profiles must not store API key references`);
    return null;
  }
  const extraMetadata = safeExtraMetadata(record, id, onQuarantine);
  const harnesses = sanitizeHarnesses(record.harnesses, template.harnesses, id, onQuarantine);
  if (!harnesses) {
    return null;
  }

  return {
    ...extraMetadata,
    id,
    label: stringField(record, "label") ?? template.label,
    active: typeof record.active === "boolean" ? record.active : false,
    mode,
    owner: stringUnion(record.owner, owners) ?? template.owner,
    baseUrl: stringValue(record.baseUrl) ?? template.baseUrl,
    model: stringValue(record.model) ?? template.model,
    apiKeyRef,
    subscriptionCredential,
    credentialStorage,
    harnesses,
  } as ModelConfigurationProfile;
}

function sanitizeSubscriptionCredential(
  value: unknown,
): SubscriptionCredentialBootstrap | null {
  const record = objectRecord(value);
  if (!record) {
    return null;
  }
  const provider = stringField(record, "provider");
  if (!provider) {
    return null;
  }
  return {
    provider,
    authDirectoryEnv: nullableString(record.authDirectoryEnv),
    authMethod:
      stringUnion(record.authMethod, authMethods) ?? "device_code",
    openBrowser: typeof record.openBrowser === "boolean" ? record.openBrowser : false,
    forceLogin: typeof record.forceLogin === "boolean" ? record.forceLogin : false,
    accountIdentityHeader: nullableString(record.accountIdentityHeader),
  };
}

function objectRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function stringValue(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function stringField(record: Record<string, unknown>, field: string): string | null {
  const value = stringValue(record[field])?.trim();
  return value ? value : null;
}

function nullableString(value: unknown): string | null {
  if (value === null || value === undefined) {
    return null;
  }
  return typeof value === "string" ? value.trim() || null : null;
}

function stringUnion<T extends string>(
  value: unknown,
  allowed: Set<T>,
): T | null {
  return typeof value === "string" && allowed.has(value as T)
    ? value as T
    : null;
}

function sanitizeHarnesses(
  value: unknown,
  defaults: ModelHarnessKind[],
  profileId: string,
  onQuarantine?: (reason: string) => void,
): ModelHarnessKind[] | null {
  if (!Array.isArray(value)) {
    if (value !== undefined) {
      onQuarantine?.(`Dropped malformed model profile harnesses for ${profileId}: expected array`);
    }
    return [...defaults];
  }
  const invalidItems: string[] = [];
  const items = value.flatMap((item) => {
    if (typeof item !== "string") {
      invalidItems.push(String(item));
      return [];
    }
    const trimmed = item.trim();
    if (!trimmed || !harnessKinds.has(trimmed as ModelHarnessKind)) {
      invalidItems.push(trimmed || "<empty>");
      return [];
    }
    return [trimmed as ModelHarnessKind];
  });
  if (invalidItems.length > 0) {
    onQuarantine?.(`Dropped invalid model profile harnesses for ${profileId}: ${invalidItems.join(", ")}`);
  }
  if (items.length === 0) {
    return null;
  }
  return Array.from(new Set(items));
}

function safeExtraMetadata(
  record: Record<string, unknown>,
  profileId: string,
  onQuarantine?: (reason: string) => void,
): Record<string, unknown> {
  const extra: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(record)) {
    if (modelProfileKnownFields.has(key)) {
      continue;
    }
    if (looksSecretBearingKey(key)) {
      recordDroppedMetadata(profileId, key, onQuarantine);
      continue;
    }
    extra[key] = stripSecretMetadata(value, profileId, key, onQuarantine);
  }
  return extra;
}

function stripSecretMetadata(
  value: unknown,
  profileId: string,
  path: string,
  onQuarantine?: (reason: string) => void,
): unknown {
  if (Array.isArray(value)) {
    return value.map((item, index) =>
      stripSecretMetadata(item, profileId, `${path}.${index}`, onQuarantine)
    );
  }
  const record = objectRecord(value);
  if (!record) {
    return value;
  }
  const stripped: Record<string, unknown> = {};
  for (const [key, child] of Object.entries(record)) {
    const childPath = `${path}.${key}`;
    if (looksSecretBearingKey(key)) {
      recordDroppedMetadata(profileId, childPath, onQuarantine);
      continue;
    }
    stripped[key] = stripSecretMetadata(child, profileId, childPath, onQuarantine);
  }
  return stripped;
}

function looksSecretBearingKey(key: string): boolean {
  return /^(?:api[_-]?key|apiKey|secret|token|oauth|oauth[_-]?token|oauthToken|password|credential)$/i.test(key);
}

function recordDroppedMetadata(
  profileId: string,
  path: string,
  onQuarantine?: (reason: string) => void,
): void {
  onQuarantine?.(`Dropped secret-bearing model profile metadata for ${profileId}: ${path}`);
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
