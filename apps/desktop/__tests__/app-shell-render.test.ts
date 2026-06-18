/**
 * App-shell render smoke test for the desktop alpha.
 *
 * The Tauri desktop entrypoint should mount the real shared OpenSymphony
 * app shell -- not a generated stub page. This test renders the shared
 * `renderOpenSymphonyApp()` entry into a JSDOM mount and asserts the
 * expected top-level viewport is produced, against a fake gateway reader
 * that always rejects.
 *
 * The contract is intentionally different from the dist/bundle smoke
 * test in `build-smoke.test.ts`: the bundle test catches "I shipped the
 * stub instead of the app shell" at build time; this render test
 * exercises the published mount-point code path so a regression that
 * drops the actual `os-app` markup fails here even if the artifact
 * bundle still emits the data attribute by accident.
 *
 * @jest-environment jsdom
 */

import { renderOpenSymphonyApp } from "@opensymphony/ui-core";
import type {
  GatewayReader,
  OpenSymphonyAppHandle,
} from "@opensymphony/ui-core";
import { createDesktopProfileController } from "../src/index";

interface TauriInvokeCall {
  command: string;
  args?: Record<string, unknown>;
}

describe("desktop app shell render", () => {
  afterEach(() => {
    delete (globalThis as unknown as { __TAURI__?: unknown }).__TAURI__;
    document.body.innerHTML = "";
  });

  it("mounts the shared OpenSymphony app shell with the expected viewport markup", async () => {
    document.body.innerHTML = `<div id="root"></div>`;
    const root = document.getElementById("root") as HTMLElement;

    // Fake reader that always rejects so the app shell falls back to its
    // built-in alpha-fixture flow. We are intentionally exercising the
    // fallback render path because that's what users will see when the
    // gateway daemon is offline during cold launch.
    const reader: GatewayReader = {
      baseUri: "http://127.0.0.1:2468",
      async health() {
        throw new Error("gateway unreachable for smoke test");
      },
      async snapshot() {
        throw new Error("gateway unreachable for smoke test");
      },
      async taskGraph() {
        throw new Error("gateway unreachable for smoke test");
      },
      async runDetail() {
        throw new Error("gateway unreachable for smoke test");
      },
      async close() {
        return undefined;
      },
    };

    const handle: OpenSymphonyAppHandle = renderOpenSymphonyApp({
      root,
      mode: "desktop",
      title: "OpenSymphony Desktop",
      transport: reader,
      initialProfiles: [],
    });

    // The initial render is scheduled synchronously via refresh(); allow
    // its inner await chain to finish before asserting on innerHTML.
    await handle.refresh();

    const shell = root.querySelector('[data-opensymphony-app-shell="mounted"]');
    expect(shell).not.toBeNull();
    expect(shell?.getAttribute("data-mode")).toBe("desktop");
    expect(root.querySelector(".os-topbar")).not.toBeNull();
    expect(root.querySelector(".os-grid")).not.toBeNull();
    expect(root.querySelector(".os-profile-panel")).not.toBeNull();
    expect(root.querySelector("[data-profile-select]")).not.toBeNull();
    expect(root.querySelector("[data-profile-gateway]")).not.toBeNull();
    expect(root.querySelector("[data-save-profile]")).not.toBeNull();

    await handle.destroy();
  });

  it("selects active profiles with Tauri's camelCase command argument", async () => {
    const calls: TauriInvokeCall[] = [];
    (globalThis as unknown as { __TAURI__: unknown }).__TAURI__ = {
      core: {
        async invoke(command: string, args?: Record<string, unknown>) {
          calls.push({ command, args });
          return {
            id: "local-daemon",
            label: "Local Gateway",
            kind: "local_daemon",
            gateway_url: "http://127.0.0.1:2468",
            transport: "loopback_http",
            managed: false,
            active: true,
          };
        },
      },
    };

    const controller = createDesktopProfileController();

    expect(controller).toBeDefined();
    const profile = await controller!.setActiveProfile("local-daemon");

    expect(calls).toEqual([
      {
        command: "set_active_profile",
        args: { profileId: "local-daemon" },
      },
    ]);
    expect(profile.gatewayUrl).toBe("http://127.0.0.1:2468");
  });
});
