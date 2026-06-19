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
import { createDesktopProfileController, createDesktopTransport } from "../src/index";

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

    // Fake reader that always rejects so the app shell renders the real
    // offline/error state users see when the gateway daemon is unavailable
    // during cold launch.
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

  it("selects active profiles with Tauri's snake_case command argument", async () => {
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
        args: { profile_id: "local-daemon" },
      },
    ]);
    expect(profile.gatewayUrl).toBe("http://127.0.0.1:2468");
  });

  it("uses native Tauri commands for desktop gateway reads", async () => {
    const calls: TauriInvokeCall[] = [];
    (globalThis as unknown as { __TAURI__: unknown }).__TAURI__ = {
      core: {
        async invoke(command: string, args?: Record<string, unknown>) {
          calls.push({ command, args });
          switch (command) {
            case "gateway_capabilities":
              return {
                schema_version: { major: 1, minor: 0, patch: 0 },
                gateway_version: "test",
                supported_api_versions: ["1.0.0"],
                transports: [],
                features: [],
                auth_modes: [],
              };
            case "dashboard_snapshot":
              return { projects: [] };
            case "task_graph":
              return { project_id: args?.project_id, nodes: [], edges: [], root_ids: [] };
            case "run_detail":
              return { run_id: args?.run_id, status: "running" };
            case "run_events":
              return { run_id: args?.run_id, events: [] };
            case "run_files":
              return { files: [{ path: "src/config.ts", status: "modified" }] };
            case "run_diffs":
              return { file_path: args?.file_path, hunks: [] };
            case "run_validation":
              return { status: "passed", checks: [] };
            case "run_approvals":
              return { approvals: [] };
            default:
              throw new Error(`unexpected command ${command}`);
          }
        },
      },
    };

    const transport = createDesktopTransport("http://127.0.0.1:2468");

    await transport.health();
    await transport.snapshot();
    await transport.taskGraph("opensymphony");
    await transport.runDetail("run-1");
    await transport.runEvents("run-1", { page_token: "opaque-token", page_size: 25 });
    await transport.runFiles("run-1");
    await transport.runDiffs("run-1", "src/config.ts");
    await transport.runValidation("run-1");
    await transport.runApprovals("run-1");

    expect(calls).toEqual([
      { command: "gateway_capabilities", args: {} },
      { command: "dashboard_snapshot", args: {} },
      { command: "task_graph", args: { project_id: "opensymphony" } },
      { command: "run_detail", args: { run_id: "run-1" } },
      { command: "run_events", args: { run_id: "run-1", page_token: "opaque-token", page_size: 25 } },
      { command: "run_files", args: { run_id: "run-1" } },
      { command: "run_diffs", args: { run_id: "run-1", file_path: "src/config.ts" } },
      { command: "run_validation", args: { run_id: "run-1" } },
      { command: "run_approvals", args: { run_id: "run-1" } },
    ]);
  });

  it("falls back to loopback HTTP when a native desktop command fails", async () => {
    const calls: TauriInvokeCall[] = [];
    const fetchCalls: string[] = [];
    const originalFetch = globalThis.fetch;
    (globalThis as unknown as { __TAURI__: unknown }).__TAURI__ = {
      core: {
        async invoke(command: string, args?: Record<string, unknown>) {
          calls.push({ command, args });
          throw new Error("native command unavailable");
        },
      },
    };
    globalThis.fetch = jest.fn(async (input: RequestInfo | URL) => {
      fetchCalls.push(String(input));
      return {
        ok: true,
        async json() {
          return {
            schema_version: { major: 1, minor: 0, patch: 0 },
            gateway_version: "test",
            supported_api_versions: ["1.0.0"],
            transports: [],
            features: [],
            auth_modes: [],
          };
        },
      } as Response;
    });

    try {
      const transport = createDesktopTransport("http://127.0.0.1:2468");
      await transport.health();

      expect(calls).toEqual([{ command: "gateway_capabilities", args: {} }]);
      expect(fetchCalls).toEqual(["http://127.0.0.1:2468/api/v1/capabilities"]);
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
});
