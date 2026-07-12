import {
  HttpGatewayTransport,
  createGraphVizDemoTransport,
  type ActionCapableTransport,
  type ActionDispatch,
  type ActionReceipt,
  type GatewayTransport,
} from "@opensymphony/api-client";
import {
  createFixtureGraphAdapter,
  createFixtureCodeGraphAdapter,
  createGatewayCodeGraphAdapter,
  createGatewayGraphAdapter,
  createTauriNativeCodeGraphAdapter,
  createTauriNativeGraphAdapter,
  graphVizFixtureBundleList,
  graphVizFixtureCommunityList,
  graphVizFixtureCompletedTasks,
  graphVizFixtureConceptDetail,
  graphVizFixtureSnapshot,
  memoryDeepLinkPrefix,
  codeDeepLinkFromLocationSearch,
  type CodeFileOutline,
  type CodeGraphSnapshot,
  type CodeRepoList,
  type CodeSymbolDetail,
  type CodeDiffOverlay,
  type CodeGraphRequestOptions,
  type CodeGraphDiffOptions,
  type NativeCodeGraphApi,
  type MemoryBundleList,
  type MemoryCommunityList,
  type MemoryCompletedTaskPage,
  type MemoryConceptDetail,
  type MemoryGraphSnapshot,
  type MemorySearchResponse,
  type NativeGraphApi,
} from "@opensymphony/graph";
import type { ConnectionProfile } from "@opensymphony/gateway-schema";
import type { RunDetail } from "@opensymphony/gateway-schema";
import { defaultModelProfiles } from "@opensymphony/gateway-schema";
import {
  createAsyncModelProfileStore,
  createModelProfileStore,
} from "@opensymphony/state";
import {
  renderOpenSymphonyApp,
  type EditableProfileInput,
  type ModelProfileController,
  type OpenSymphonyAppHandle,
  type ProfileController,
} from "@opensymphony/ui-core";

const DEFAULT_GATEWAY_URL = "http://127.0.0.1:2468";

type TauriInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export function createDesktopGraphAdapter(gatewayUrl = DEFAULT_GATEWAY_URL) {
  const invoke = getTauriInvoke();
  if (invoke) {
    return createDesktopNativeGraphAdapter(createDesktopNativeGraphApi(invoke));
  }
  return createGatewayGraphAdapter(gatewayUrl);
}

export function createDesktopCodeGraphAdapter(gatewayUrl = DEFAULT_GATEWAY_URL) {
  const invoke = getTauriInvoke();
  if (invoke) return createDesktopNativeCodeGraphAdapter(createDesktopNativeCodeGraphApi(invoke));
  return createGatewayCodeGraphAdapter(gatewayUrl);
}

export function createDesktopNativeGraphAdapter(api: NativeGraphApi) {
  return createTauriNativeGraphAdapter(api);
}

export function createDesktopNativeCodeGraphAdapter(api: NativeCodeGraphApi) {
  return createTauriNativeCodeGraphAdapter(api);
}

interface TauriGlobal {
  invoke?: TauriInvoke;
  core?: {
    invoke?: TauriInvoke;
  };
}

interface NativeProfileResponse {
  id: string;
  label: string;
  kind: string;
  gateway_url?: string;
  gatewayUrl?: string;
  managed?: boolean;
  active?: boolean;
  daemon_path?: string | null;
  daemonPath?: string | null;
  transport?: ConnectionProfile["transport"];
}

interface NativeSettingResponse {
  value?: NativeSettingValue | null;
}

interface NativeCopyResponse {
  copied: boolean;
}

interface NativeOpenDeeplinkResponse {
  opened: boolean;
}

type NativeSettingValue =
  | { type: "Text"; value: string }
  | { type: "Flag"; value: boolean }
  | { type: "Number"; value: number };

function createDesktopNativeGraphApi(invoke: TauriInvoke): NativeGraphApi {
  return {
    listBundles: () => invoke<MemoryBundleList>("memory_bundles", { visibility: null }),
    getGraphSnapshot: (bundleId, options) =>
      invoke<MemoryGraphSnapshot>("memory_graph", {
        bundleId,
        visibility: options?.visibility ?? null,
      }),
    getConceptDetail: (bundleId, conceptId, options) =>
      invoke<MemoryConceptDetail>("memory_concept_detail", {
        bundleId,
        conceptId,
        visibility: options?.visibility ?? null,
      }),
    getCommunities: (bundleId, options) =>
      invoke<MemoryCommunityList>("memory_communities", {
        bundleId,
        visibility: options?.visibility ?? null,
      }),
    search: (query, options) =>
      invoke<MemorySearchResponse>("memory_search", {
        query,
        limit: options?.limit ?? null,
        bundleId: options?.bundleId ?? null,
        visibility: options?.visibility ?? null,
      }),
    getCompletedTasks: (options) =>
      invoke<MemoryCompletedTaskPage>("memory_completed_tasks", {
        query: options?.query ?? null,
        sort: options?.sort ?? null,
        limit: options?.limit ?? null,
        offset: options?.offset ?? null,
        visibility: options?.visibility ?? null,
      }),
  };
}

function createDesktopNativeCodeGraphApi(invoke: TauriInvoke): NativeCodeGraphApi {
  return {
    listRepos: (options) => invoke<CodeRepoList>("code_repos", { includeStale: options?.includeStale ?? null }),
    getGraphSnapshot: (repoId, options?: CodeGraphRequestOptions) =>
      invoke<CodeGraphSnapshot>("code_graph", {
        repoId,
        mode: options?.mode ?? null,
        path: options?.path ?? null,
        symbolKey: options?.symbolKey ?? null,
        depth: options?.depth ?? null,
        aggregate: options?.aggregate ?? null,
        includeStale: options?.includeStale ?? null,
      }),
    getSymbolDetail: (repoId, symbolKey, options) =>
      invoke<CodeSymbolDetail>("code_symbol_detail", {
        repoId,
        symbolKey,
        includeStale: options?.includeStale ?? null,
        visibility: options?.visibility ?? null,
      }),
    getFileOutline: (runId, filePath, repoId) =>
      invoke<CodeFileOutline>("run_code_outline", { runId, filePath, repoId: repoId ?? null, limit: null }),
    getDiffOverlay: (repoId, baseRevision, headRevision, options?: CodeGraphDiffOptions) =>
      invoke<CodeDiffOverlay>("code_diff_overlay", {
        repoId,
        baseRevision,
        headRevision,
        limit: options?.limit ?? null,
      }),
    getRunDiffOverlay: (runId, repoId, options?: CodeGraphDiffOptions) =>
      invoke<CodeDiffOverlay>("run_code_diff_overlay", {
        runId,
        repoId: repoId ?? null,
        limit: options?.limit ?? null,
      }),
  };
}

export interface TauriTransportAdapter extends ActionCapableTransport {
  attach(): Promise<void>;
}

class DesktopTransportAdapter implements TauriTransportAdapter {
  private readonly actionInner: ActionCapableTransport;

  constructor(
    private readonly inner: GatewayTransport,
    private readonly baseUrl: string,
  ) {
    this.actionInner = asActionCapableTransport(inner, baseUrl);
  }

  get baseUri(): string {
    return this.inner.baseUri;
  }

  health(): ReturnType<GatewayTransport["health"]> {
    return this.invokeOrHttp("gateway_capabilities", {}, () => this.inner.health());
  }

  snapshot(): ReturnType<GatewayTransport["snapshot"]> {
    return this.invokeOrHttp("dashboard_snapshot", {}, () => this.inner.snapshot());
  }

  taskGraph(projectId: string): ReturnType<GatewayTransport["taskGraph"]> {
    return this.invokeOrHttp("task_graph", { projectId }, () => this.inner.taskGraph(projectId));
  }

  runDetail(runId: string): ReturnType<GatewayTransport["runDetail"]> {
    return this.invokeOrHttp("run_detail", { runId }, () => this.inner.runDetail(runId));
  }

  runEvents(
    runId: string,
    cursor?: Parameters<GatewayTransport["runEvents"]>[1],
  ): ReturnType<GatewayTransport["runEvents"]> {
    return this.invokeOrHttp(
      "run_events",
      {
        runId,
        pageToken: cursor?.page_token ?? null,
        pageSize: cursor?.page_size ?? null,
      },
      () => this.inner.runEvents(runId, cursor),
    );
  }

  runTimeline(runId: string): ReturnType<GatewayTransport["runTimeline"]> {
    return this.inner.runTimeline(runId);
  }

  runLogs(
    runId: string,
    cursor?: Parameters<GatewayTransport["runLogs"]>[1],
    limit?: Parameters<GatewayTransport["runLogs"]>[2],
  ): ReturnType<GatewayTransport["runLogs"]> {
    return this.inner.runLogs(runId, cursor, limit);
  }

  terminalSnapshot(
    runId: string,
    terminalId: string,
    cursor?: Parameters<GatewayTransport["terminalSnapshot"]>[2],
  ): ReturnType<GatewayTransport["terminalSnapshot"]> {
    return this.inner.terminalSnapshot(runId, terminalId, cursor);
  }

  terminalSearch(
    runId: string,
    terminalId: string,
    query: string,
  ): ReturnType<GatewayTransport["terminalSearch"]> {
    return this.inner.terminalSearch(runId, terminalId, query);
  }

  terminalJumpToEvent(
    runId: string,
    terminalId: string,
    eventId: string,
  ): ReturnType<GatewayTransport["terminalJumpToEvent"]> {
    return this.inner.terminalJumpToEvent(runId, terminalId, eventId);
  }

  runFiles(runId: string): ReturnType<GatewayTransport["runFiles"]> {
    return this.invokeOrHttp<{ files?: Awaited<ReturnType<GatewayTransport["runFiles"]>> }>(
      "run_files",
      { runId },
      async () => ({ files: await this.inner.runFiles(runId) }),
    ).then((response) => response.files ?? []);
  }

  runDiffs(runId: string, filePath?: string): ReturnType<GatewayTransport["runDiffs"]> {
    return this.invokeOrHttp("run_diffs", { runId, filePath: filePath ?? null }, () =>
      this.inner.runDiffs(runId, filePath),
    );
  }

  runApprovals(runId: string): ReturnType<GatewayTransport["runApprovals"]> {
    return this.invokeOrHttp<{ approvals?: Awaited<ReturnType<GatewayTransport["runApprovals"]>> }>(
      "run_approvals",
      { runId },
      async () => ({ approvals: await this.inner.runApprovals(runId) }),
    ).then((response) => response.approvals ?? []);
  }

  runValidation(runId: string): ReturnType<GatewayTransport["runValidation"]> {
    return this.invokeOrHttp("run_validation", { runId }, () => this.inner.runValidation(runId));
  }

  events(
    fromCursor?: Parameters<GatewayTransport["events"]>[0],
  ): ReturnType<GatewayTransport["events"]> {
    return this.inner.events(fromCursor);
  }

  terminalFrames(
    runId: string,
  ): ReturnType<GatewayTransport["terminalFrames"]> {
    return this.inner.terminalFrames(runId);
  }

  close(): ReturnType<GatewayTransport["close"]> {
    return this.inner.close();
  }

  dispatchAction(action: ActionDispatch): Promise<ActionReceipt> {
    return this.actionInner.dispatchAction(action);
  }

  cancelRun(runId: string): Promise<ActionReceipt> {
    return this.actionInner.cancelRun(runId);
  }

  retryRun(runId: string): Promise<ActionReceipt> {
    return this.actionInner.retryRun(runId);
  }

  resumeRun(runId: string): Promise<ActionReceipt> {
    return this.actionInner.resumeRun(runId);
  }

  rehydrateRun(runId: string): Promise<ActionReceipt> {
    return this.actionInner.rehydrateRun(runId);
  }

  commentRun(runId: string, text: string): Promise<ActionReceipt> {
    return this.actionInner.commentRun(runId, text);
  }

  createFollowup(runId: string, payload: unknown): Promise<ActionReceipt> {
    return this.actionInner.createFollowup(runId, payload);
  }

  approvalDecision(
    approvalId: string,
    decision: "approved" | "rejected",
    explanation?: string,
  ): Promise<ActionReceipt> {
    return this.actionInner.approvalDecision(approvalId, decision, explanation);
  }

  openWorkspace(runId: string): Promise<ActionReceipt> {
    return this.runDetail(runId).then((run) => this.copyWorkspacePath(run));
  }

  debugRun(runId: string): Promise<ActionReceipt> {
    return this.runDetail(runId).then((run) => this.openOrCopyDebugTarget(run));
  }

  async attach(): Promise<void> {
    const invoke = getTauriInvoke();
    if (!invoke) {
      return;
    }
    await invoke("attach_gateway", {
      req: {
        base_url: this.baseUrl,
        auth_token: null,
      },
    }).catch(() => undefined);
  }

  private async invokeOrHttp<T>(
    command: string,
    args: Record<string, unknown>,
    fallback: () => Promise<T>,
  ): Promise<T> {
    const invoke = getTauriInvoke();
    if (!invoke) {
      return fallback();
    }
    return invoke<T>(command, args);
  }

  private async copyWorkspacePath(run: RunDetail): Promise<ActionReceipt> {
    if (!run.workspace_path) {
      return desktopActionReceipt("open_workspace", run.run_id, "rejected", "workspace path unavailable");
    }
    await copyText(run.workspace_path);
    return desktopActionReceipt("open_workspace", run.run_id, "accepted", "workspace path copied");
  }

  private async openOrCopyDebugTarget(run: RunDetail): Promise<ActionReceipt> {
    if (run.harness_type === "codex_app_server") {
      if (!run.codex_thread_id) {
        return desktopActionReceipt("debug", run.run_id, "rejected", "Codex thread id unavailable");
      }
      const url = `codex://threads/${run.codex_thread_id}`;
      try {
        await openDeeplink(url);
        return desktopActionReceipt("debug", run.run_id, "accepted", "Codex thread opened");
      } catch (error) {
        await copyText(url);
        return desktopActionReceipt("debug", run.run_id, "accepted", `Codex deeplink copied: ${stringifyError(error)}`);
      }
    }

    if (!run.workspace_path) {
      return desktopActionReceipt("debug", run.run_id, "rejected", "workspace path unavailable");
    }
    const command = `cd ${shellQuote(run.workspace_path)} && opensymphony debug ${shellQuote(run.issue_identifier)}`;
    await copyText(command);
    return desktopActionReceipt("debug", run.run_id, "accepted", "debug command copied");
  }
}

async function copyText(text: string): Promise<void> {
  const invoke = getTauriInvoke();
  try {
    if (!invoke) {
      await navigator.clipboard.writeText(text);
      return;
    }
    await invoke<NativeCopyResponse>("copy_to_clipboard", { req: { text } });
  } catch (error) {
    throw new Error(`clipboard copy failed: ${stringifyError(error)}`);
  }
}

async function openDeeplink(url: string): Promise<void> {
  const invoke = getTauriInvoke();
  if (!invoke) {
    throw new Error("desktop opener unavailable");
  }
  await invoke<NativeOpenDeeplinkResponse>("open_deeplink", { req: { url } });
}

function desktopActionReceipt(
  action: ActionDispatch["action_kind"],
  runId: string,
  status: ActionReceipt["status"],
  reason: string,
): ActionReceipt {
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    action_id: `${action}-${runId}`,
    correlation_id: `${action}-${runId}-${crypto.randomUUID()}`,
    status,
    reason,
    expected_followup: [],
    issued_at: new Date().toISOString(),
  };
}

function shellQuote(value: string): string {
  // POSIX shell string for the copied debug command. Keep this as data for the
  // operator to paste; do not reuse it as a run-in-place command executor.
  return `'${value.replaceAll("'", "'\\''")}'`;
}

function stringifyError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function createDesktopTransport(
  baseUri = DEFAULT_GATEWAY_URL,
): TauriTransportAdapter {
  return new DesktopTransportAdapter(new HttpGatewayTransport({
    baseUri,
    transport: "loopback_http",
  }), baseUri);
}

export function createDesktopProfileController(): ProfileController | undefined {
  const invoke = getTauriInvoke();
  if (!invoke) {
    return undefined;
  }

  return {
    async listProfiles() {
      const profiles = await invoke<NativeProfileResponse[]>("list_profiles", {});
      return profiles.map(toConnectionProfile);
    },

    async storeProfile(profile: EditableProfileInput) {
      const stored = await invoke<NativeProfileResponse>("store_profile", {
        req: {
          id: profile.id ?? null,
          label: profile.label,
          kind: profile.kind,
          gateway_url: profile.gatewayUrl,
          daemon_path: null,
          daemon_args: [],
          auto_restart: false,
          startup_timeout_secs: 30,
        },
      });
      return toConnectionProfile(stored);
    },

    async setActiveProfile(profileId: string) {
      const active = await invoke<NativeProfileResponse>("set_active_profile", {
        profileId,
      });
      return toConnectionProfile(active);
    },

    async removeProfile(profileId: string) {
      const profiles = await invoke<NativeProfileResponse[]>("remove_profile", {
        profileId,
      });
      return profiles.map(toConnectionProfile);
    },
  };
}

const MODEL_PROFILE_SETTINGS_KEY = "opensymphony.desktop.modelProfiles.v1";

export function createDesktopModelProfileController(): ModelProfileController {
  const invoke = getTauriInvoke();
  if (!invoke) {
    const quarantineMessages: string[] = [];
    const store = createModelProfileStore({
      defaults: defaultModelProfiles(),
      onQuarantine: (reason) => {
        quarantineMessages.push(reason);
      },
    });
    return {
      ...store,
      quarantineMessages,
      takeQuarantineMessages() {
        return quarantineMessages.splice(0);
      },
      persistence: {
        kind: "session",
        label: "Model profiles are session-only because desktop settings are unavailable.",
      },
    };
  }
  const tauriInvoke = invoke;

  const quarantineMessages: string[] = [];
  const recordQuarantine = (reason: string) => {
    quarantineMessages.push(reason);
  };
  const store = createAsyncModelProfileStore({
    defaults: defaultModelProfiles(),
    onQuarantine: recordQuarantine,
    async load() {
      const response = await tauriInvoke<NativeSettingResponse>("get_setting", {
        req: { key: MODEL_PROFILE_SETTINGS_KEY },
      });
      const value = response.value;
      if (!value) {
        return null;
      }
      if (value.type !== "Text") {
        recordQuarantine("Dropped malformed desktop model profile setting: expected Text value");
        return null;
      }
      return value.value;
    },
    async save(value) {
      await tauriInvoke("set_setting", {
        req: {
          key: MODEL_PROFILE_SETTINGS_KEY,
          value: {
            type: "Text",
            value,
          },
        },
      });
    },
  });

  return {
    ...store,
    quarantineMessages,
    takeQuarantineMessages() {
      return quarantineMessages.splice(0);
    },
    persistence: {
      kind: "durable",
      label: "Model profiles persist in desktop settings.",
    },
  };
}

function asActionCapableTransport(
  transport: GatewayTransport,
  baseUrl: string,
): ActionCapableTransport {
  if ("dispatchAction" in transport) {
    return transport as ActionCapableTransport;
  }
  // Fallback: when the inner transport is not action-capable (e.g. a plain
  // read-only channel), open a separate loopback HTTP connection to the
  // gateway for action dispatch. This is intentional for desktop because the
  // Tauri channel implementation is action-capable; the HTTP fallback is the
  // documented baseline and preserves the same auth/CORS contract as the
  // desktop app's own loopback server.
  return new HttpGatewayTransport({
    baseUri: baseUrl || DEFAULT_GATEWAY_URL,
    transport: "loopback_http",
  });
}

function getTauriInvoke(): TauriInvoke | undefined {
  const tauri = (globalThis as Record<string, unknown>).__TAURI__ as
    | TauriGlobal
    | undefined;
  return tauri?.invoke ?? tauri?.core?.invoke;
}

function toConnectionProfile(profile: NativeProfileResponse): ConnectionProfile {
  const gatewayUrl = profile.gatewayUrl ?? profile.gateway_url ?? DEFAULT_GATEWAY_URL;
  const base = {
    id: profile.id,
    label: profile.label,
    active: profile.active ?? false,
    gatewayUrl,
    transport: profile.transport ?? "loopback_http",
    managed: profile.managed ?? isManagedKind(profile.kind),
  };

  switch (profile.kind) {
    case "supervised_local_daemon":
      return {
        ...base,
        kind: "supervised_local_daemon",
        managed: true,
        daemonPath: profile.daemonPath ?? profile.daemon_path ?? "",
        daemonArgs: [],
        daemonEnv: {},
        startupTimeoutSecs: 30,
        autoRestart: false,
      };
    case "embedded_host":
      return {
        ...base,
        kind: "embedded_host",
        managed: true,
      };
    case "hosted_gateway":
      return {
        ...base,
        kind: "hosted_gateway",
        managed: false,
        probeOnConnect: true,
        transport: "websocket",
      };
    case "external_gateway":
      return {
        ...base,
        kind: "external_gateway",
        managed: false,
        probeOnConnect: true,
      };
    case "local_daemon":
      return {
        ...base,
        kind: "local_daemon",
        managed: false,
      };
    default:
      return {
        ...base,
        kind: profile.kind as ConnectionProfile["kind"],
        managed: profile.managed ?? false,
      } as ConnectionProfile;
  }
}

function isManagedKind(kind: string): boolean {
  return kind === "embedded_host" || kind === "supervised_local_daemon";
}

async function createTransportForGateway(gatewayUrl: string): Promise<TauriTransportAdapter> {
  const base = gatewayUrl || DEFAULT_GATEWAY_URL;
  const transport = createDesktopTransport(base);
  await transport.attach();
  return transport;
}

/**
 * Fixture workbench mode: `?fixtures` mounts the app on deterministic demo
 * data (dense knowledge graph + dependency-heavy task graph) instead of the
 * local gateway. Used for graph-visualization iteration and screenshots via
 * the vite dev server; a packaged Tauri build never carries a query string,
 * so production behavior is unchanged. See docs/graph-view.md.
 */
function fixtureWorkbenchRequested(): boolean {
  try {
    return new URLSearchParams(globalThis.location?.search ?? "").has("fixtures");
  } catch {
    return false;
  }
}

/**
 * `?memory=opensymphony://memory/...` opens the app on a memory deep link —
 * the same entry point task-graph artifacts will use once they carry capsule
 * links. Handy for testing links end to end from the fixture workbench.
 */
function openMemoryDeepLinkFromLocation(app: OpenSymphonyAppHandle): void {
  try {
    // URLSearchParams.get() percent-decodes the value, which would collapse
    // load-bearing %2F/%3A escapes inside a raw pasted link's bundle or
    // community ids. Read the raw parameter instead; a link that was itself
    // encodeURIComponent-ed as a whole (so it doesn't start with the plain
    // scheme) is restored with one explicit decode.
    const raw = /[?&]memory=([^&]*)/.exec(globalThis.location?.search ?? "")?.[1];
    if (!raw) return;
    const link = raw.startsWith(memoryDeepLinkPrefix) ? raw : decodeURIComponent(raw);
    void app.openMemoryDeepLink(link);
  } catch {
    // No usable location (tests, packaged builds without query strings).
  }
}

function openCodeDeepLinkFromLocation(app: OpenSymphonyAppHandle): void {
  try {
    const link = codeDeepLinkFromLocationSearch(globalThis.location?.search ?? "");
    if (!link) return;
    void app.ready()
      .then(() => app.openCodeDeepLink(link))
      .catch(() => undefined);
  } catch {
    // No usable location (tests, packaged builds without query strings).
  }
}

const root = document.getElementById("root");
if (root && fixtureWorkbenchRequested()) {
  const app = renderOpenSymphonyApp({
    root,
    mode: "desktop",
    title: "OpenSymphony Desktop (fixtures)",
    transport: createGraphVizDemoTransport(),
    graphAdapter: createFixtureGraphAdapter({
      bundles: graphVizFixtureBundleList,
      snapshot: graphVizFixtureSnapshot,
      communities: graphVizFixtureCommunityList,
      conceptDetail: (_bundleId, conceptId) => graphVizFixtureConceptDetail(conceptId),
      completedTasks: graphVizFixtureCompletedTasks,
    }),
    codeGraphAdapter: createFixtureCodeGraphAdapter(),
    modelProfileController: createDesktopModelProfileController(),
  });
  openMemoryDeepLinkFromLocation(app);
  openCodeDeepLinkFromLocation(app);
} else if (root) {
  const transport = createDesktopTransport();
  void transport.attach();
  const app = renderOpenSymphonyApp({
    root,
    mode: "desktop",
    title: "OpenSymphony Desktop",
    transport,
    graphAdapter: createDesktopGraphAdapter(DEFAULT_GATEWAY_URL),
    codeGraphAdapter: createDesktopCodeGraphAdapter(DEFAULT_GATEWAY_URL),
    profileController: createDesktopProfileController(),
    initialProfiles: [
      {
        id: "local-daemon",
        label: "Local Daemon",
        kind: "local_daemon",
        active: true,
        gatewayUrl: DEFAULT_GATEWAY_URL,
        transport: "loopback_http",
        managed: false,
      },
    ],
    modelProfileController: createDesktopModelProfileController(),
    onGatewayUrlChanged: createTransportForGateway,
    onGraphGatewayUrlChanged: createDesktopGraphAdapter,
    onCodeGraphGatewayUrlChanged: createDesktopCodeGraphAdapter,
  });
  openMemoryDeepLinkFromLocation(app);
  openCodeDeepLinkFromLocation(app);
}
