/**
 * @jest-environment node
 */

import { createWebProfileController } from "../src/profile-controller";

class MemoryStorage {
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

describe("web profile controller", () => {
  it("returns an active local gateway profile with the configured default URL", async () => {
    const controller = createWebProfileController({
      defaultGatewayUrl: "http://127.0.0.1:5173",
      storage: new MemoryStorage(),
    });

    const profiles = await controller.listProfiles();

    expect(profiles).toHaveLength(1);
    expect(profiles[0]).toMatchObject({
      id: "web-local-daemon",
      label: "Local Gateway",
      kind: "local_daemon",
      active: true,
      gatewayUrl: "http://127.0.0.1:5173",
    });
  });

  it("persists profile edits for the next controller instance", async () => {
    const storage = new MemoryStorage();
    const controller = createWebProfileController({
      defaultGatewayUrl: "http://127.0.0.1:5173",
      storage,
    });

    await controller.storeProfile({
      id: "web-local-daemon",
      label: "Loopback Gateway",
      kind: "local_daemon",
      gatewayUrl: "http://localhost:2468",
    });

    const restored = createWebProfileController({
      defaultGatewayUrl: "http://127.0.0.1:5173",
      storage,
    });
    const profiles = await restored.listProfiles();

    expect(profiles).toHaveLength(1);
    expect(profiles[0]).toMatchObject({
      id: "web-local-daemon",
      label: "Loopback Gateway",
      active: true,
      gatewayUrl: "http://localhost:2468",
    });
  });
});
