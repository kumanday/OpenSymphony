/**
 * @jest-environment node
 */

import { createModelProfile } from "@opensymphony/gateway-schema";
import { createWebModelProfileController } from "../src/model-profile-controller";

class MemoryStorage implements Pick<Storage, "getItem" | "setItem" | "removeItem"> {
  private values = new Map<string, string>();

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

describe("web model profile controller", () => {
  it("persists model profiles when host storage is supplied", async () => {
    const storage = new MemoryStorage();
    const controller = createWebModelProfileController({ storage });
    const profile = {
      ...createModelProfile("api_key"),
      id: "web-api",
      active: true,
      model: "provider/web-api",
      baseUrl: "https://models.example/v1",
      apiKeyRef: "local_keychain:web-api-key",
      harnesses: ["openhands_agent_server", "codex_app_server"],
    };

    await controller.storeProfile(profile);
    await controller.setActiveProfile(profile.id);

    const restored = createWebModelProfileController({ storage });
    const profiles = await restored.listProfiles();
    const saved = profiles.find((candidate) => candidate.id === profile.id);

    expect(restored.persistence?.kind).toBe("durable");
    expect(saved).toMatchObject({
      model: "provider/web-api",
      baseUrl: "https://models.example/v1",
      apiKeyRef: "local_keychain:web-api-key",
      active: true,
    });
    expect(saved?.harnesses).toEqual(["openhands_agent_server", "codex_app_server"]);
  });

  it("reports session-only behavior when host storage is unavailable", async () => {
    const first = createWebModelProfileController({ storage: null });
    const profile = {
      ...createModelProfile("api_key"),
      id: "session-api",
      model: "provider/session-only",
    };

    await first.storeProfile(profile);
    const second = createWebModelProfileController({ storage: null });
    const profiles = await second.listProfiles();

    expect(first.persistence?.kind).toBe("session");
    expect(first.persistence?.label).toContain("session-only");
    expect(profiles.some((candidate) => candidate.id === "session-api")).toBe(false);
  });

  it("records quarantine warnings for malformed durable profiles", async () => {
    const storage = new MemoryStorage();
    storage.setItem("opensymphony.web.modelProfiles.v1", JSON.stringify({
      profiles: [
        {
          ...createModelProfile("api_key"),
          id: "raw-secret",
          apiKeyRef: "sk-secret-value-123456789",
        },
      ],
      activeProfileId: "raw-secret",
    }));
    const controller = createWebModelProfileController({ storage });

    await controller.listProfiles();

    expect(controller.quarantineMessages).toEqual([
      expect.stringContaining("raw-secret"),
    ]);
  });
});
