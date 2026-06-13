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

describe("desktop app shell render", () => {
  it("mounts the shared OpenSymphony app shell with the expected viewport markup", async () => {
    document.body.innerHTML = `<div id="root"></div>`;
    const root = document.getElementById("root") as HTMLElement;

    // Fake reader that always rejects so the app shell falls back to its
    // built-in alpha-fixture flow. We are intentionally exercising the
    // fallback render path because that's what users will see when the
    // gateway daemon is offline during cold launch.
    const reader: GatewayReader = {
      baseUri: "http://127.0.0.1:8000",
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
});
