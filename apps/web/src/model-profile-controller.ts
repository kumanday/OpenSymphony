import {
  createModelProfileStore,
  type ModelProfileStorage,
} from "@opensymphony/state";
import type { ModelProfileController } from "@opensymphony/ui-core";

const STORAGE_KEY = "opensymphony.web.modelProfiles.v1";

interface HostGlobal {
  modelProfileStorage?: ModelProfileStorage | null;
}

export interface WebModelProfileControllerOptions {
  storage?: ModelProfileStorage | null;
}

export function createWebModelProfileController(
  options: WebModelProfileControllerOptions = {},
): ModelProfileController {
  const storage = options.storage === undefined
    ? hostModelProfileStorage() ?? browserStorage()
    : options.storage;
  const durable = storage !== null;
  const quarantineMessages: string[] = [];
  return {
    ...createModelProfileStore({
      storage,
      storageKey: STORAGE_KEY,
      onQuarantine: (reason) => {
        quarantineMessages.push(reason);
      },
    }),
    quarantineMessages,
    takeQuarantineMessages() {
      return quarantineMessages.splice(0);
    },
    persistence: durable
      ? {
          kind: "durable",
          label: "Model profiles persist in host storage.",
        }
      : {
          kind: "session",
          label: "Model profiles are session-only because host storage is unavailable.",
        },
  };
}

function hostModelProfileStorage(): ModelProfileStorage | null {
  const host = (globalThis as Record<string, unknown>).__OPENSYMPHONY_HOST__ as
    | HostGlobal
    | undefined;
  return host?.modelProfileStorage ?? null;
}

function browserStorage(): ModelProfileStorage | null {
  try {
    return typeof window !== "undefined" ? window.localStorage : null;
  } catch {
    return null;
  }
}
