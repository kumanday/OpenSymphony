/** Reducer and storage helpers for model configuration profiles. */

import {
  defaultModelProfiles,
  validateModelProfileCredentials,
  type ModelConfigurationProfile,
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
}

const DEFAULT_STORAGE_KEY = "opensymphony.modelProfiles.v1";

export function createModelProfileStore(
  options: ModelProfileStoreOptions = {},
): ModelProfileStore {
  const storage = options.storage ?? null;
  const storageKey = options.storageKey ?? DEFAULT_STORAGE_KEY;
  let fallback = normalizeModelProfileState({
    profiles: options.defaults ?? defaultModelProfiles(),
    activeProfileId: null,
  });
  let writeQueue: Promise<unknown> = Promise.resolve();

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
      return normalizeModelProfileState({
        profiles: Array.isArray(parsed.profiles) ? parsed.profiles : fallback.profiles,
        activeProfileId: parsed.activeProfileId ?? null,
      });
    } catch {
      return fallback;
    }
  }

  function write(state: ModelProfileState): ModelProfileState {
    const normalized = normalizeModelProfileState(state);
    fallback = normalized;
    if (storage) {
      storage.setItem(storageKey, JSON.stringify(normalized));
    }
    return normalized;
  }

  function serialize<T>(operation: () => T): Promise<T> {
    const next = writeQueue.then(operation, operation);
    writeQueue = next.catch(() => undefined);
    return next;
  }

  return {
    async listProfiles() {
      return read().profiles;
    },
    async storeProfile(profile) {
      return serialize(() => {
        const validationError = validateModelProfileCredentials(profile);
        if (validationError) {
          throw new Error(validationError);
        }
        const current = read();
        const index = current.profiles.findIndex((candidate) => candidate.id === profile.id);
        const profiles = [...current.profiles];
        if (index >= 0) {
          profiles[index] = profile;
        } else {
          profiles.push(profile);
        }
        const next = write({
          profiles,
          activeProfileId: profile.active
            ? profile.id
            : current.activeProfileId === profile.id ? null : current.activeProfileId,
        });
        return next.profiles.find((candidate) => candidate.id === profile.id)!;
      });
    },
    async setActiveProfile(profileId) {
      return serialize(() => {
        const current = read();
        if (!current.profiles.some((profile) => profile.id === profileId)) {
          throw new Error(`Unknown model profile: ${profileId}`);
        }
        const next = write({
          profiles: current.profiles,
          activeProfileId: profileId,
        });
        return next.profiles.find((profile) => profile.id === profileId)!;
      });
    },
    async removeProfile(profileId) {
      return serialize(() => {
        const current = read();
        if (!current.profiles.some((profile) => profile.id === profileId)) {
          throw new Error(`Unknown model profile: ${profileId}`);
        }
        if (current.profiles.length <= 1) {
          throw new Error("Cannot remove the last model profile");
        }
        const profiles = current.profiles.filter((profile) => profile.id !== profileId);
        const next = write({
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
