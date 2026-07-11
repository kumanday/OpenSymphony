import type {
  ActionDispatch,
  ActionReceipt,
  ChangedFileEntry,
  ConnectionProfile,
  DashboardSnapshot,
  FileDiffPage,
  GatewayEnvelope,
  GatewayCapabilities,
  ApprovalRequest,
  RunAction,
  RunDetail,
  RunEvent,
  RunEventPage,
  RunPhase,
  RunStreamLiveness,
  RunValidationSummary,
  TaskGraphNode,
  TaskGraphNodeKind,
  TaskGraphSnapshot,
  AuthState,
  ModelConfigurationProfile,
  ModelCredentialMode,
  MemoryGraphUpdatedEvent,
  CodeGraphUpdatedEvent,
  CodeGraphNode,
  CodeGraphSnapshot,
  CodeSymbolDetail,
} from "@opensymphony/gateway-schema";
import {
  applyCodeGraphFilters,
  cachedConceptDetail,
  codeEdgeVisualStyle,
  codeGraphLayoutKindForMode,
  codeGraphReducer,
  codeGraphSnapshotForRendering,
  codeNodeVisualStyle,
  isConceptDetailStale,
  createGraphLayoutAdapter,
  createInitialCodeGraphState,
  createInitialGraphState,
  currentCodeGraphSnapshot,
  formatMemoryDeepLink,
  graphLayoutKindForMode,
  graphReducer,
  currentGraphSnapshot,
  sameGraphTopology,
  parseMemoryDeepLink,
  parseCodeDeepLink,
  codeDeepLinkToGraphState,
  resolveMemoryDeepLinkNode,
  visibleGraphSnapshot,
  type GraphDataAdapter,
  type CodeGraphAdapter,
  type CodeGraphFilters,
  type CodeGraphState,
  type CodeGraphMode,
  type GraphLayoutAdapter,
  type GraphLayoutResult,
  type GraphState,
  type MemoryCompletedTask,
  type MemoryCompletedTaskPage,
  type MemoryConceptDetail,
  type MemoryGraphNode,
  type MemoryTaskPullRequest,
} from "@opensymphony/graph";
import {
  authStateFromError,
  createModelProfile,
  defaultModelProfiles,
  redactCredentialRef,
  validateStoredCredentialRef,
  validateSubscriptionCredential,
} from "@opensymphony/gateway-schema";
import { renderChangedFileList, renderFileDiff } from "./diff.js";
import { renderValidationSummary } from "./validation.js";
import { renderApprovalList, type ApprovalDecision } from "./approval.js";
import {
  buildActionBarItems,
  renderActionBar,
  renderActionReceipt,
  renderAuditTrailEntry,
} from "./run-actions.js";
import {
  buildRuntimeOverlay,
  defaultTaskGraphFilter,
  filterTaskGraphNodes,
  renderBadge,
  renderTaskGraphFilters,
  type TaskGraphFilter,
} from "./task-graph-editor.js";
import {
  emptyCommentEdit,
  emptyDependencyEdit,
  emptyEditorDialog,
  emptyInlineEdit,
  renderCommentEditor,
  renderCreateDialog,
  renderDependencyEditor,
  renderTaskGraphNode,
  renderTaskGraphToolbar,
  type CommentEditState,
  type DependencyEditState,
  type EditorDialogState,
  type InlineEditState,
} from "./task-graph-editor-ui.js";
import {
  applyNodeUpdate,
  buildCreatedNode,
  dispatchTaskGraphComment,
  dispatchTaskGraphCreate,
  dispatchTaskGraphDependencies,
  dispatchTaskGraphUpdate,
  isActionCapable,
} from "./task-graph-editor-actions.js";
import { generateId } from "./id.js";
import {
  addCriterion,
  addMessage,
  addPlanningNode,
  addVerification,
  buildFixturePlanningWorkspaceState,
  emptyPlanningWorkspaceState,
  removePlanningNode,
  removeCriterion,
  removeVerification,
  selectArtifact,
  selectRevision,
  toggleCriterion,
  toggleNodeExpanded,
  toggleVerification,
  updateArtifactContent,
  updateCriterion,
  updateNodeDependencies,
  updatePlanningNode,
  updateVerification,
  type PlanningWorkspaceState,
} from "./planning-workspace.js";
import {
  emptyPlanningEditState,
  renderPlanningWorkspace,
  type PlanningEditState,
} from "./planning-workspace-ui.js";
import {
  bindKnowledgeGraphListNavigation,
  mountCodeGraphRenderer,
  renderCodeGraphInspector,
  renderCodeGraphNodeList,
  renderCodeGraphSurface,
  disposeCodeGraphRenderer,
  createKnowledgeGraphViewState,
  disposeKnowledgeGraphRenderer,
  type KnowledgeGraphViewState,
  mountKnowledgeGraphRenderer,
  renderKnowledgeGraphInspector,
  renderKnowledgeGraphNodeList,
  renderKnowledgeGraphSurface,
} from "./knowledge-graph-renderer.js";
import { morphChildren } from "./dom-morph.js";

export interface GatewayReader {
  readonly baseUri: string;
  health(): Promise<GatewayCapabilities>;
  snapshot(): Promise<DashboardSnapshot>;
  taskGraph(projectId: string): Promise<TaskGraphSnapshot>;
  runDetail(runId: string): Promise<RunDetail>;
  runEvents?(runId: string): Promise<RunEventPage>;
  runFiles?(runId: string): Promise<ChangedFileEntry[]>;
  runDiffs?(runId: string, filePath?: string): Promise<FileDiffPage>;
  runValidation?(runId: string): Promise<RunValidationSummary>;
  runApprovals?(runId: string): Promise<ApprovalRequest[]>;
  events?(fromCursor?: { sequence: number; partition: string }): AsyncIterable<GatewayEnvelope>;
  /** Optional action dispatch for gateway-mediated mutations. */
  dispatchAction?(action: ActionDispatch): Promise<ActionReceipt>;
  close(): Promise<void>;
}

export interface ProfileController {
  listProfiles(): Promise<ConnectionProfile[]>;
  storeProfile(profile: EditableProfileInput): Promise<ConnectionProfile>;
  setActiveProfile(profileId: string): Promise<ConnectionProfile>;
  removeProfile(profileId: string): Promise<ConnectionProfile[]>;
}

export interface ModelProfileController {
  persistence?: ModelProfilePersistenceInfo;
  quarantineMessages?: string[];
  takeQuarantineMessages?(): string[];
  listProfiles(): Promise<ModelConfigurationProfile[]>;
  storeProfile(profile: ModelConfigurationProfile): Promise<ModelConfigurationProfile>;
  setActiveProfile(profileId: string): Promise<ModelConfigurationProfile>;
  removeProfile(profileId: string): Promise<ModelConfigurationProfile[]>;
}

export interface ModelProfilePersistenceInfo {
  kind: "durable" | "session";
  label: string;
}

export interface EditableProfileInput {
  id?: string;
  label: string;
  kind: ConnectionProfile["kind"];
  gatewayUrl: string;
}

export interface OpenSymphonyAppOptions {
  root: HTMLElement;
  mode: "desktop" | "web";
  transport: GatewayReader;
  title?: string;
  profileController?: ProfileController;
  modelProfileController?: ModelProfileController;
  initialProfiles?: ConnectionProfile[];
  initialModelProfiles?: ModelConfigurationProfile[];
  onGatewayUrlChanged?: (gatewayUrl: string) => Promise<GatewayReader>;
  graphAdapter?: GraphDataAdapter;
  onGraphGatewayUrlChanged?: (gatewayUrl: string) => GraphDataAdapter;
  codeGraphAdapter?: CodeGraphAdapter;
  onCodeGraphGatewayUrlChanged?: (gatewayUrl: string) => CodeGraphAdapter;
}

export interface OpenSymphonyAppHandle {
  refresh(): Promise<void>;
  destroy(): Promise<void>;
  /**
   * Navigate to a memory deep link (opensymphony://memory/...): switches to
   * the Knowledge Graph pane, loads the linked bundle, drills into the
   * community, and selects the linked concept capsule. Resolves false when
   * the link does not parse or its target is not present in the graph.
   * External surfaces (task graph artifacts, notifications) call this.
   */
  openMemoryDeepLink(url: string): Promise<boolean>;
  openCodeDeepLink(url: string): Promise<boolean>;
}

type ConnectionMode = "connecting" | "connected" | "failed";

interface AppState {
  connectionMode: ConnectionMode;
  connectionMessage: string;
  authState: AuthState;
  capabilities: GatewayCapabilities | null;
  snapshot: DashboardSnapshot | null;
  taskGraph: TaskGraphSnapshot | null;
  selectedProjectId: string | null;
  selectedNodeId: string | null;
  graphPaneView: GraphPaneView;
  knowledgeGraph: GraphState;
  codeGraph: CodeGraphState;
  knowledgeGraphLayout: GraphLayoutResult | null;
  runDetail: RunDetail | null;
  runFiles: ChangedFileEntry[] | null;
  selectedDiffPath: string | null;
  runDiff: FileDiffPage | null;
  evidenceView: EvidenceView;
  runEvents: RunEvent[] | null;
  expandedActivityEvents: Set<string>;
  collapsedActivityEvents: Set<string>;
  workspacePaneSizes: WorkspacePaneSizesBySurface;
  // Widths (px) of the collapsible task-graph side panes; the Current pane
  // between them flexes to absorb the remainder.
  taskPaneSizes: { done: number; backlog: number };
  // Height (px) of the lower Run Detail / Inspector row; drag the divider
  // between the graph pane and this row to resize it vertically.
  lowerRowHeight: number;
  eventLogModalOpen: boolean;
  runValidation: RunValidationSummary | null;
  runApprovals: ApprovalRequest[] | null;
  lastActionReceipt: ActionReceipt | null;
  auditTrail: AuditTrailEntry[];
  profiles: ConnectionProfile[];
  activeProfileId: string | null;
  gatewayDraft: string;
  profilePanelExpanded: boolean;
  modelProfiles: ModelConfigurationProfile[];
  activeModelProfileId: string | null;
  modelProfileError: string | null;
  modelPanelExpanded: boolean;
  loading: boolean;
  activeView: "dashboard" | "planning";
  // Task graph editor state
  taskGraphFilter: TaskGraphFilter;
  // Three-pane task graph (desktop): Completed | Current | Backlog
  taskPaneCollapsed: { done: boolean; backlog: boolean };
  completedTasks: MemoryCompletedTaskPage | null;
  completedTasksError: string | null;
  completedTasksParams: { query: string; sort: string; page: number };
  collapsedProjectGroups: Set<string>;
  inlineEdit: InlineEditState;
  createDialog: EditorDialogState;
  dependencyEdit: DependencyEditState;
  commentEdit: CommentEditState;
  runOverlays: Map<string, RunDetail>;
  pendingMutations: Set<string>;
  pendingCreates: Map<string, string>;
  pendingSnapshots: Map<string, TaskGraphNode | null>;
  // Planning workspace state
  planningWorkspace: PlanningWorkspaceState;
  planningEdit: PlanningEditState;
}

interface AuditTrailEntry {
  timestamp: string;
  actor: string;
  action: string;
  target: string;
  status: string;
  details?: string;
}

/** Evidence for one run, fetched concurrently and applied atomically. */
interface RunDetailBundle {
  runFiles: ChangedFileEntry[];
  selectedDiffPath: string | null;
  runDiff: FileDiffPage | null;
  runEvents: RunEvent[];
  runValidation: RunValidationSummary | null;
  runApprovals: ApprovalRequest[];
  warnings: string[];
}

type EvidenceView = "diff" | "activity";
type GraphPaneView = "task" | "knowledge" | "code";
type WorkspacePaneResizeHandle = "lower-columns";

interface WorkspacePaneSizes {
  left: number;
  right: number;
}

type WorkspacePaneSizesBySurface = Record<GraphPaneView, WorkspacePaneSizes>;

const schemaVersion = { major: 1, minor: 0, patch: 0 };
// Two consecutive failures avoid noisy transient warnings while still surfacing stale live data.
const liveRefreshFailureThreshold = 2;
const liveRefreshPollIntervalMs = 5_000;
const defaultWorkspacePaneSizes: WorkspacePaneSizes = { left: 50, right: 50 };
const minWorkspacePaneSizes: WorkspacePaneSizes = { left: 30, right: 30 };
type TaskSidePane = "done" | "backlog";
const defaultTaskPaneSizes: { done: number; backlog: number } = { done: 360, backlog: 340 };
// Side panes stay usable without starving the flexible Current pane.
const taskPaneSizeBounds: Record<TaskSidePane, { min: number; max: number }> = {
  done: { min: 260, max: 640 },
  backlog: { min: 240, max: 620 },
};
const taskPaneResizeStep = 24;
// Lower Run Detail / Inspector row: taller by default than the old fixed
// clamp, and drag-resizable vertically between these bounds.
const defaultLowerRowHeight = 520;
const lowerRowHeightBounds = { min: 240, max: 1000 };
const lowerRowResizeStep = 24;
const completedTasksPageSize = 25;
const defaultCompletedTasksParams = { query: "", sort: "completed_desc", page: 1 } as const;

export function renderOpenSymphonyApp(
  options: OpenSymphonyAppOptions,
): OpenSymphonyAppHandle {
  const app = new OpenSymphonyApp(options);
  void app.refresh();
  return app;
}

class OpenSymphonyApp implements OpenSymphonyAppHandle {
  private options: OpenSymphonyAppOptions;
  private transport: GatewayReader;
  private graphAdapter: GraphDataAdapter | null;
  private codeGraphAdapter: CodeGraphAdapter | null;
  private state: AppState;
  private destroyed = false;
  private eventSubscription: {
    active: boolean;
    transport: GatewayReader;
    iterator?: AsyncIterator<GatewayEnvelope>;
  } | null = null;
  private latestGatewayEventCursor: { sequence: number; partition: string } | null = null;
  private liveRefreshInFlight = false;
  private liveRefreshQueued = false;
  private liveRefreshFailureCount = 0;
  private liveRefreshTimer: ReturnType<typeof setInterval> | null = null;
  private knowledgeGraphLoadInFlight: Promise<void> | null = null;
  private knowledgeGraphLoadQueuedBundleId: string | null | undefined = undefined;
  private graphLayoutAdapter: GraphLayoutAdapter = createGraphLayoutAdapter(() => null);
  private pendingGraphLayoutAdapter: GraphLayoutAdapter | null = null;
  private graphLayoutRun = 0;
  private knowledgeGraphView: KnowledgeGraphViewState = createKnowledgeGraphViewState();
  private knowledgeGraphLayoutSize: { width: number; height: number } | null = null;
  private codeGraphLayout: GraphLayoutResult | null = null;
  private codeGraphLoadInFlight: Promise<void> | null = null;
  private codeGraphLoadQueued = false;
  private codeGraphLayoutRun = 0;
  private codeGraphNavigationVersion = 0;
  private codeGraphView: KnowledgeGraphViewState = createKnowledgeGraphViewState();
  private codeGraphSymbolRequest: string | null = null;
  private codeGraphRawRecord = false;
  /** Concept-detail request currently in flight, keyed `${bundleId}:${conceptId}`. */
  private knowledgeCapsuleRequest: string | null = null;
  /** Guards in-flight completed-tasks pages (superseded by newer queries). */
  private completedTasksSeq = 0;
  /** Debounce for the Completed pane's search input. */
  private completedTasksSearchTimer: ReturnType<typeof setTimeout> | null = null;
  /** Cross-pane edge repositioning is rAF-coalesced per burst of scroll/resize. */
  private crossLinksFrame: number | null = null;
  /** Last failed concept-detail request; keyed so a new selection is unaffected. */
  private knowledgeCapsuleError: { key: string; message: string } | null = null;
  /**
   * Monotonic counter bumped by every user-initiated navigation (task click,
   * project switch, diff-file click, full refresh). Background work snapshots
   * the epoch when it starts and discards its results if the user navigated
   * while it was in flight, so a slow poll can never revert a selection.
   */
  private interactionEpoch = 0;
  /**
   * Guards in-flight openRun loads. Deliberately separate from
   * interactionEpoch: selecting a diff file bumps the epoch (to invalidate
   * background refreshes) but must not cancel a task open that is still
   * loading — only a newer open, project switch, or full refresh does.
   */
  private runOpenSeq = 0;
  /** Guards in-flight selectDiffFile loads (superseded by newer diff clicks or opens). */
  private diffSelectSeq = 0;
  /** Tracks bindEvents sites already attached per element (see listen()). */
  private boundListeners = new WeakMap<Element, Set<string>>();

  constructor(options: OpenSymphonyAppOptions) {
    this.options = options;
    this.transport = options.transport;
    this.graphAdapter = options.graphAdapter ?? null;
    this.codeGraphAdapter = options.codeGraphAdapter ?? null;
    this.installBrowserGraphLayoutAdapter();
    const profiles = options.initialProfiles ?? [];
    const activeProfile = profiles.find((profile) => profile.active) ?? profiles[0] ?? null;
    const modelProfiles = options.initialModelProfiles ?? defaultModelProfiles();
    const activeModelProfile = modelProfiles.find((profile) => profile.active) ?? null;
    this.state = {
      connectionMode: "connecting",
      connectionMessage: "Connecting",
      authState: "open",
      capabilities: null,
      snapshot: null,
      taskGraph: null,
      selectedProjectId: null,
      selectedNodeId: null,
      graphPaneView: "task",
      knowledgeGraph: createInitialGraphState(),
      codeGraph: createInitialCodeGraphState(),
      knowledgeGraphLayout: null,
      runDetail: null,
      runFiles: null,
      selectedDiffPath: null,
      runDiff: null,
      evidenceView: "diff",
      runEvents: null,
      expandedActivityEvents: new Set(),
      collapsedActivityEvents: new Set(),
      workspacePaneSizes: createDefaultWorkspacePaneSizes(),
      taskPaneSizes: { ...defaultTaskPaneSizes },
      lowerRowHeight: defaultLowerRowHeight,
      eventLogModalOpen: false,
      runValidation: null,
      runApprovals: null,
      lastActionReceipt: null,
      auditTrail: [],
      profiles,
      activeProfileId: activeProfile?.id ?? null,
      gatewayDraft: activeProfile?.gatewayUrl ?? this.transport.baseUri,
      profilePanelExpanded: false,
      modelProfiles,
      activeModelProfileId: activeModelProfile?.id ?? null,
      modelProfileError: null,
      modelPanelExpanded: false,
      loading: true,
      activeView: "dashboard",
      taskGraphFilter: { ...defaultTaskGraphFilter },
      taskPaneCollapsed: { done: false, backlog: false },
      completedTasks: null,
      completedTasksError: null,
      completedTasksParams: { ...defaultCompletedTasksParams },
      collapsedProjectGroups: new Set(),
      inlineEdit: { ...emptyInlineEdit },
      createDialog: { ...emptyEditorDialog },
      dependencyEdit: { ...emptyDependencyEdit },
      commentEdit: { ...emptyCommentEdit },
      runOverlays: new Map(),
      pendingMutations: new Set(),
      pendingCreates: new Map(),
      pendingSnapshots: new Map(),
      planningWorkspace: emptyPlanningWorkspaceState(),
      planningEdit: { ...emptyPlanningEditState },
    };
    this.loadPlanningWorkspace("opensymphony-local");
    // Document-level so Escape works even when focus sits on the body (the
    // graph canvas itself is not focusable). Removed in destroy().
    this.options.root.ownerDocument.addEventListener("keydown", this.onDocumentKeydown);
    // Cross-pane task edges are measured from live card positions, so any
    // viewport resize must recompute them. Removed in destroy().
    this.options.root.ownerDocument.defaultView?.addEventListener("resize", this.onWindowResize);
  }

  private onWindowResize = (): void => {
    this.scheduleCrossLinksReposition();
  };

  private onDocumentKeydown = (event: KeyboardEvent): void => {
    if (this.destroyed || event.key !== "Escape") {
      return;
    }
    if (this.state.graphPaneView === "code") {
      if (this.state.codeGraph.breadcrumbs.length > 0 || this.state.codeGraph.mode !== "atlas") {
        this.stepBackCodeGraphView();
      }
      return;
    }
    if (this.state.graphPaneView !== "knowledge") return;
    const graph = this.state.knowledgeGraph;
    if (
      graph.mode !== "atlas"
      || graph.focusedNodeId
      || graph.selectedNodeIds.length > 0
      || graph.filters.communities.length > 0
    ) {
      // Escape pops one drill level (concept → area → atlas) instead of
      // jumping straight home; repeated presses still land on the atlas.
      this.stepBackKnowledgeGraphView();
    }
  };

  /**
   * Fetch all evidence for a run concurrently, without touching state. The
   * caller applies the returned bundle atomically (applyRunDetailBundle), so
   * the UI never renders a half-cleared panel while requests are in flight.
   */
  private async fetchRunDetailBundle(
    runId: string,
    preferredDiffPath: string | null,
  ): Promise<RunDetailBundle> {
    const warnings: string[] = [];
    const filesAndDiff = (async () => {
      let runFiles: ChangedFileEntry[] = [];
      try {
        runFiles = typeof this.transport.runFiles === "function"
          ? await this.transport.runFiles(runId)
          : [];
      } catch (error) {
        warnings.push(`Changed files unavailable: ${errorMessage(error)}`);
      }
      const selectedDiffPath = preferredDiffPath && runFiles.some((file) => file.path === preferredDiffPath)
        ? preferredDiffPath
        : runFiles[0]?.path ?? null;
      let runDiff: FileDiffPage | null = null;
      if (selectedDiffPath && typeof this.transport.runDiffs === "function") {
        try {
          runDiff = await this.transport.runDiffs(runId, selectedDiffPath);
        } catch (error) {
          warnings.push(`Diff unavailable: ${errorMessage(error)}`);
        }
      }
      return { runFiles, selectedDiffPath, runDiff };
    })();
    const events = (async () => {
      try {
        return typeof this.transport.runEvents === "function"
          ? (await this.transport.runEvents(runId)).events
          : [];
      } catch (error) {
        warnings.push(`Conversation activity unavailable: ${errorMessage(error)}`);
        return [];
      }
    })();
    const validation = (async () => {
      try {
        return typeof this.transport.runValidation === "function"
          ? await this.transport.runValidation(runId)
          : null;
      } catch (error) {
        warnings.push(`Validation summary unavailable: ${errorMessage(error)}`);
        return null;
      }
    })();
    const approvals = (async () => {
      try {
        return typeof this.transport.runApprovals === "function"
          ? await this.transport.runApprovals(runId)
          : [];
      } catch (error) {
        warnings.push(`Approvals unavailable: ${errorMessage(error)}`);
        return [];
      }
    })();

    const [
      { runFiles, selectedDiffPath, runDiff },
      runEvents,
      runValidation,
      runApprovals,
    ] = await Promise.all([filesAndDiff, events, validation, approvals]);
    return { runFiles, selectedDiffPath, runDiff, runEvents, runValidation, runApprovals, warnings };
  }

  private applyRunDetailBundle(bundle: RunDetailBundle): void {
    this.state.runFiles = bundle.runFiles;
    this.state.selectedDiffPath = bundle.selectedDiffPath;
    this.state.runDiff = bundle.runDiff;
    this.state.runEvents = bundle.runEvents;
    this.state.runValidation = bundle.runValidation;
    this.state.runApprovals = bundle.runApprovals;
    if (bundle.warnings.length > 0) {
      this.state.connectionMessage = bundle.warnings[bundle.warnings.length - 1];
    }
  }

  async refresh(): Promise<void> {
    if (this.destroyed) {
      return;
    }
    this.interactionEpoch += 1;
    this.runOpenSeq += 1;
    this.diffSelectSeq += 1;
    this.state.loading = true;
    this.render();

    await this.loadProfiles();
    await this.loadModelProfiles();
    await this.loadGatewayState();
    this.state.loading = false;
    this.render();
  }

  async destroy(): Promise<void> {
    this.destroyed = true;
    this.options.root.ownerDocument.removeEventListener("keydown", this.onDocumentKeydown);
    this.options.root.ownerDocument.defaultView?.removeEventListener("resize", this.onWindowResize);
    if (this.completedTasksSearchTimer !== null) {
      clearTimeout(this.completedTasksSearchTimer);
      this.completedTasksSearchTimer = null;
    }
    this.stopLiveRefreshTimer();
    this.stopEventSubscription();
    this.graphLayoutAdapter.dispose();
    this.pendingGraphLayoutAdapter?.dispose();
    this.pendingGraphLayoutAdapter = null;
    disposeKnowledgeGraphRenderer(this.options.root);
    disposeCodeGraphRenderer(this.options.root);
    await this.transport.close().catch(() => undefined);
    this.options.root.replaceChildren();
  }

  private async loadProfiles(): Promise<void> {
    if (!this.options.profileController) {
      return;
    }
    try {
      const profiles = await this.options.profileController.listProfiles();
      this.state.profiles = profiles;
      const active = profiles.find((profile) => profile.active) ?? profiles[0] ?? null;
      this.state.activeProfileId = active?.id ?? null;
      this.state.gatewayDraft = active?.gatewayUrl ?? this.state.gatewayDraft;
      if (
        active
        && this.options.onGatewayUrlChanged
        && active.gatewayUrl !== this.transport.baseUri
      ) {
        this.stopEventSubscription();
        await this.transport.close().catch(() => undefined);
        this.transport = await this.options.onGatewayUrlChanged(active.gatewayUrl);
        this.graphAdapter = this.options.onGraphGatewayUrlChanged?.(active.gatewayUrl) ?? this.graphAdapter;
        this.codeGraphAdapter = this.options.onCodeGraphGatewayUrlChanged?.(active.gatewayUrl) ?? this.codeGraphAdapter;
        this.resetKnowledgeGraph();
        this.resetCodeGraph();
      }
    } catch (error) {
      this.state.connectionMessage = `Profiles unavailable: ${errorMessage(error)}`;
    }
  }

  private async loadModelProfiles(): Promise<void> {
    if (!this.options.modelProfileController) {
      return;
    }
    try {
      const profiles = await this.options.modelProfileController.listProfiles();
      this.state.modelProfiles = profiles.length > 0 ? profiles : defaultModelProfiles();
      const active = this.state.modelProfiles.find((profile) => profile.active) ?? null;
      this.state.activeModelProfileId = active?.id ?? null;
      const warnings = this.options.modelProfileController.takeQuarantineMessages?.()
        ?? this.options.modelProfileController.quarantineMessages?.slice()
        ?? [];
      this.state.modelProfileError = warnings.length > 0
        ? `Model profile storage warning: ${warnings.join("; ")}`
        : null;
    } catch (error) {
      this.state.modelProfileError = `Model profiles unavailable: ${errorMessage(error)}`;
    }
  }

  private async loadGatewayState(): Promise<void> {
    // Fetch capabilities first so an auth failure on the dashboard snapshot
    // still tells us whether the gateway advertises auth (hosted mode). A
    // gateway that requires auth typically succeeds on /capabilities but
    // rejects the snapshot with 401/403 until the client authenticates.
    let capabilities: GatewayCapabilities | null = null;
    try {
      capabilities = await this.transport.health();
    } catch (error) {
      this.state.capabilities = null;
      this.state.authState = this.resolveAuthState(error);
      if (this.state.authState !== "open") {
        this.state.connectionMode = "connected";
        this.state.connectionMessage = this.authMessage(this.state.authState);
        this.clearGatewayData();
        return;
      }
      this.state.connectionMode = "failed";
      this.state.connectionMessage = `Gateway unavailable: ${errorMessage(error)}`;
      this.clearGatewayData();
      return;
    }

    try {
      const snapshot = await this.transport.snapshot();
      this.state.capabilities = capabilities;
      this.state.snapshot = snapshot;
      this.state.connectionMode = "connected";
      this.state.authState = this.resolveAuthState(null);
      this.state.connectionMessage = `Connected to ${this.transport.baseUri || "same-origin gateway"}`;
      this.liveRefreshFailureCount = 0;
      const previousProjectId = this.state.selectedProjectId;
      this.state.selectedProjectId = snapshot.projects.some((project) => project.project_id === previousProjectId)
        ? previousProjectId
        : snapshot.projects[0]?.project_id ?? "default";
      const selectedProjectId = this.state.selectedProjectId ?? "default";
      await this.loadTaskGraph(selectedProjectId, true);
      this.startEventSubscription();
      this.startLiveRefreshTimer();
      this.loadPlanningWorkspace(selectedProjectId);
      this.state.planningWorkspace = {
        ...this.state.planningWorkspace,
        project_id: selectedProjectId,
      };
    } catch (error) {
      this.state.capabilities = capabilities;
      this.state.authState = this.resolveAuthState(error);
      if (this.state.authState !== "open") {
        // Capabilities resolved, but the protected resource rejected us.
        // Treat the connection as established so the auth placeholder renders
        // instead of a generic offline banner.
        this.state.connectionMode = "connected";
        this.state.connectionMessage = this.authMessage(this.state.authState);
      } else {
        this.state.connectionMode = "failed";
        this.state.connectionMessage = `Gateway unavailable: ${errorMessage(error)}`;
      }
      this.clearGatewayData();
    }
  }

  private clearGatewayData(): void {
    this.stopLiveRefreshTimer();
    this.resetCompletedTasks();
    this.state.snapshot = null;
    this.state.taskGraph = null;
    this.state.selectedProjectId = null;
    this.state.selectedNodeId = null;
    this.state.runDetail = null;
    this.state.runFiles = null;
    this.state.runDiff = null;
    this.state.evidenceView = "diff";
    this.state.runEvents = null;
    this.state.expandedActivityEvents = new Set();
    this.state.collapsedActivityEvents = new Set();
    this.state.runValidation = null;
    this.state.runApprovals = null;
  }

  /**
   * Drop the current gateway's Completed-pane rows. Called on any context
   * change (gateway switch, disconnect) so another gateway's completed
   * tasks — titles, PR URLs — never linger beside a new task graph.
   */
  private resetCompletedTasks(): void {
    // Bump the sequence so any in-flight completed-tasks request is
    // abandoned rather than repopulating the cleared page after a context
    // change (its seq check will now fail).
    this.completedTasksSeq += 1;
    this.state.completedTasks = null;
    this.state.completedTasksError = null;
    this.state.completedTasksParams = { ...defaultCompletedTasksParams };
  }

  private resetKnowledgeGraph(): void {
    this.resetCompletedTasks();
    this.state.knowledgeGraph = createInitialGraphState();
    this.state.knowledgeGraphLayout = null;
    this.knowledgeGraphLayoutSize = null;
    this.knowledgeGraphLoadInFlight = null;
    this.knowledgeGraphLoadQueuedBundleId = undefined;
    this.knowledgeCapsuleRequest = null;
    this.knowledgeCapsuleError = null;
    // A different bundle/gateway is different content: drop the camera and
    // any node drag overrides along with the graph state.
    this.knowledgeGraphView = createKnowledgeGraphViewState();
  }

  private resetCodeGraph(): void {
    const shouldReload = this.state.graphPaneView === "code" && !this.destroyed;
    this.state.codeGraph = createInitialCodeGraphState();
    this.codeGraphLayout = null;
    this.codeGraphLoadInFlight = null;
    this.codeGraphLoadQueued = false;
    this.codeGraphLayoutRun += 1;
    this.codeGraphNavigationVersion += 1;
    this.codeGraphSymbolRequest = null;
    this.codeGraphRawRecord = false;
    this.codeGraphView = createKnowledgeGraphViewState();
    if (shouldReload) void this.loadCodeGraph();
  }

  /**
   * Resolve the auth-facing state from a thrown error.
   *
   * The shell only gates on auth when a protected read fails: a classified
   * auth error (`unauthenticated`/`unauthorized`/`forbidden`) wins, and any
   * successful load (authenticated caller or local no-auth gateway) is
   * `open`. Advertised `auth_modes` are not consulted here; capabilities are
   * fetched separately so a snapshot that 401s still reports the gateway's
   * auth modes, but they do not change the gate decision.
   */
  private resolveAuthState(error: unknown): AuthState {
    return authStateFromError(error);
  }

  private authMessage(state: AuthState): string {
    switch (state) {
      case "unauthenticated":
        return "Sign in required";
      case "unauthorized":
        return "Access denied: insufficient permission";
      case "forbidden":
        return "Access forbidden";
      case "open":
      default:
        return "";
    }
  }

  private loadPlanningWorkspace(projectId: string | null): void {
    // The fixture planning session is loaded once so the UI renders immediately.
    // Subsequent gateway/project changes only update the project_id; the workspace
    // session (messages, edits, criteria) is intentionally kept across project switches.
    if (this.state.planningWorkspace && this.state.planningWorkspace.session_id) {
      return;
    }
    this.state.planningWorkspace = buildFixturePlanningWorkspaceState(projectId ?? "opensymphony-local");
  }

  private async loadTaskGraph(projectId: string | null, preserveSelection = false): Promise<void> {
    if (!projectId) {
      this.state.taskGraph = null;
      this.state.selectedNodeId = null;
      return;
    }
    const previousNodeId = preserveSelection ? this.state.selectedNodeId : null;
    const previousRunId = preserveSelection ? this.state.runDetail?.run_id ?? null : null;
    let taskGraph: TaskGraphSnapshot;
    try {
      taskGraph = await this.transport.taskGraph(projectId);
    } catch (error) {
      this.state.taskGraph = null;
      this.state.selectedNodeId = null;
      this.state.runDetail = null;
      this.state.runFiles = null;
      this.state.runDiff = null;
      this.state.evidenceView = "diff";
      this.state.runEvents = null;
      this.state.expandedActivityEvents = new Set();
      this.state.collapsedActivityEvents = new Set();
      this.state.runValidation = null;
      this.state.runApprovals = null;
      this.state.connectionMessage = `Task graph unavailable: ${errorMessage(error)}`;
      return;
    }

    this.state.taskGraph = taskGraph;
    const initialNode = taskGraph.nodes.find((node) => node.node_id === previousNodeId)
      ?? taskGraph.nodes.find((node) => node.run_id === previousRunId)
      ?? initialSelectedTaskNode(taskGraph.nodes, taskGraph.root_ids);
    this.state.selectedNodeId = initialNode?.node_id ?? null;
    this.state.runDetail = null;
    this.state.runFiles = null;
    this.state.runDiff = null;
    this.state.evidenceView = "diff";
    this.state.runEvents = null;
    this.state.expandedActivityEvents = new Set();
    this.state.collapsedActivityEvents = new Set();
    this.state.runValidation = null;
    this.state.runApprovals = null;
    this.state.selectedDiffPath = null;
    if (this.options.mode === "desktop") {
      // Completed pane data loads independently: a memory-server hiccup must
      // not delay the Current/Backlog graph. A project switch
      // (!preserveSelection) is a context change, so drop the prior page up
      // front rather than showing it beside the new project's graph.
      void this.loadCompletedTasks(!preserveSelection);
    }
    await Promise.all([
      this.fetchRunOverlays(taskGraph).then((overlays) => {
        // A concurrently opened run may already have stored a fresher detail.
        for (const [runId, run] of this.state.runOverlays) {
          if (!overlays.has(runId)) {
            overlays.set(runId, run);
          }
        }
        this.state.runOverlays = overlays;
      }),
      // A preserved backlog selection stays selected but has no run to
      // open — probing it would only produce "Run unavailable" noise.
      initialNode && initialNode.state_category !== "backlog"
        ? this.openRun(initialNode)
        : Promise.resolve(),
    ]);
  }

  /**
   * Fetch the Completed pane's current page from the memory-backed
   * completed-tasks endpoint. Newer requests supersede in-flight ones.
   *
   * `contextChanged` (gateway/project switch) drops the previous context's
   * rows up front and resets paging/search, so the new Current/Backlog
   * graph never renders beside another gateway's completed tasks (titles,
   * PR URLs) while the fetch is in flight or if it fails. In-context
   * reloads keep the last good page on failure and surface the error
   * inline.
   */
  private async loadCompletedTasks(contextChanged = false): Promise<void> {
    const adapter = this.graphAdapter;
    if (contextChanged) {
      // resetCompletedTasks bumps completedTasksSeq, so an in-flight request
      // from the previous context can no longer repopulate the page.
      this.resetCompletedTasks();
    }
    if (!adapter?.getCompletedTasks) {
      // A gateway without a memory endpoint has no completed tasks: never
      // leave the prior context's rows on screen, and invalidate any
      // still-in-flight request so it cannot land after this return.
      if (contextChanged || this.state.completedTasks || this.state.completedTasksError) {
        this.resetCompletedTasks();
      }
      return;
    }
    const seq = ++this.completedTasksSeq;
    const { query, sort, page } = this.state.completedTasksParams;
    try {
      const result = await adapter.getCompletedTasks({
        query: query || undefined,
        sort,
        limit: completedTasksPageSize,
        offset: (page - 1) * completedTasksPageSize,
      });
      if (this.destroyed || seq !== this.completedTasksSeq) {
        return;
      }
      this.state.completedTasks = result;
      this.state.completedTasksError = null;
    } catch (error) {
      if (this.destroyed || seq !== this.completedTasksSeq) {
        return;
      }
      this.state.completedTasksError = errorMessage(error);
    }
    this.render();
  }

  private async loadKnowledgeGraph(bundleId?: string): Promise<void> {
    if (this.knowledgeGraphLoadInFlight) {
      if (bundleId !== undefined) {
        this.knowledgeGraphLoadQueuedBundleId = bundleId ?? null;
      } else if (this.state.knowledgeGraph.freshnessStatus === "stale" && this.knowledgeGraphLoadQueuedBundleId === undefined) {
        this.knowledgeGraphLoadQueuedBundleId = null;
      }
      return this.knowledgeGraphLoadInFlight;
    }
    const load = this.loadKnowledgeGraphOnce(bundleId).finally(async () => {
      this.knowledgeGraphLoadInFlight = null;
      const queuedBundleId = this.knowledgeGraphLoadQueuedBundleId;
      this.knowledgeGraphLoadQueuedBundleId = undefined;
      if (queuedBundleId !== undefined && !this.destroyed) {
        await this.loadKnowledgeGraph(queuedBundleId ?? undefined);
      }
    });
    this.knowledgeGraphLoadInFlight = load;
    return load;
  }

  private async loadKnowledgeGraphOnce(bundleId?: string): Promise<void> {
    if (!this.graphAdapter) {
      this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, {
        type: "LAYOUT_STATUS_SET",
        status: "failed",
        error: "Knowledge Graph unavailable for the active transport",
      });
      this.render();
      return;
    }
    // Only surface the loading state while the very first snapshot is being
    // fetched. Once a snapshot exists the status belongs to the layout
    // pipeline: marking background refreshes as "loading" left the status
    // dangling whenever the poll redelivered an identical snapshot (nothing
    // resets it), pinning the pill on "Stabilizing" and blocking both resize
    // relayouts and retries of failed layouts.
    if (!currentGraphSnapshot(this.state.knowledgeGraph)) {
      this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "LAYOUT_STATUS_SET", status: "loading" });
      this.render();
    }
    try {
      if (!this.state.knowledgeGraph.bundles) {
        const bundles = await this.graphAdapter.listBundles();
        this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "BUNDLES_LOADED", bundles });
      }
      const selectedBundleId = bundleId ?? this.state.knowledgeGraph.selectedBundleId ?? this.state.knowledgeGraph.bundles?.bundles[0]?.id ?? null;
      if (!selectedBundleId) {
        this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "LAYOUT_STATUS_SET", status: "ready" });
        this.render();
        return;
      }
      if (this.state.knowledgeGraph.selectedBundleId !== selectedBundleId) {
        this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "BUNDLE_SELECTED", bundleId: selectedBundleId });
      }
      await this.refreshKnowledgeGraphSnapshot(selectedBundleId);
    } catch (error) {
      this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "LAYOUT_STATUS_SET", status: "failed", error: errorMessage(error) });
      this.render();
    }
  }

  private async refreshKnowledgeGraphSnapshot(bundleId: string): Promise<void> {
    if (!this.graphAdapter) return;
    const snapshot = await this.graphAdapter.getGraphSnapshot(bundleId);
    const previousGraph = this.state.knowledgeGraph;
    const previousSnapshot = currentGraphSnapshot(previousGraph);
    this.state.knowledgeGraph = graphReducer(previousGraph, { type: "SNAPSHOT_LOADED", snapshot });
    const acceptedSnapshot = this.state.knowledgeGraph !== previousGraph;
    // Only recompute the layout when the graph's topology (its node/edge set)
    // actually changed — not merely because the reducer accepted a newer
    // snapshot. The live memory snapshot's cursor advances on every poll even
    // when the content is identical; relaying out on each such tick recomputed
    // node positions and, because the renderer reframes on a new layout,
    // yanked the operator's zoom back out to the area overview every five
    // seconds. Capsule staleness still keys on acceptance (see SNAPSHOT_LOADED),
    // so an edited capsule still refetches without a needless relayout.
    const topologyChanged = acceptedSnapshot && !sameGraphTopology(previousSnapshot, snapshot);
    if (topologyChanged) {
      this.state.knowledgeGraphLayout = null;
      this.knowledgeGraphLayoutSize = null;
      this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "LAYOUT_STATUS_SET", status: "idle" });
    } else if (!this.state.knowledgeGraphLayout && this.state.knowledgeGraph.layoutStatus === "failed") {
      // Identical poll while nothing is on screen after a failed layout:
      // return to idle so the next bind pass schedules a retry instead of
      // staying failed until a newer snapshot happens to arrive.
      this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "LAYOUT_STATUS_SET", status: "idle" });
    }
    this.render();
  }

  private async loadCodeGraph(): Promise<void> {
    if (this.codeGraphLoadInFlight) {
      this.codeGraphLoadQueued = true;
      return this.codeGraphLoadInFlight;
    }
    const load = this.loadCodeGraphOnce();
    const completion = load.finally(async () => {
      this.codeGraphLoadInFlight = null;
      if (this.codeGraphLoadQueued && !this.destroyed) {
        this.codeGraphLoadQueued = false;
        await this.loadCodeGraph();
      }
    });
    this.codeGraphLoadInFlight = completion;
    return completion;
  }

  private async loadCodeGraphOnce(): Promise<void> {
    const navigationVersion = this.codeGraphNavigationVersion;
    if (!this.codeGraphAdapter) {
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, {
        type: "LAYOUT_STATUS_SET",
        status: "failed",
        error: "Code Graph unavailable for the active transport",
      });
      this.render();
      return;
    }
    try {
      if (!this.state.codeGraph.repos) {
        const repos = await this.codeGraphAdapter.listRepos();
        if (this.destroyed || navigationVersion !== this.codeGraphNavigationVersion) return;
        this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "REPOS_LOADED", repos });
      }
      const repoId = this.state.codeGraph.repoId ?? this.state.codeGraph.repos?.repos[0]?.repo_id ?? null;
      if (!repoId) {
        this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "LAYOUT_STATUS_SET", status: "ready" });
        this.render();
        return;
      }
      if (this.state.codeGraph.repoId !== repoId) {
        this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "REPO_SELECTED", repoId });
      }
      const requestKey = this.codeGraphRequestKey();
      await this.refreshCodeGraphSnapshot(repoId, requestKey, navigationVersion);
      if (this.destroyed || navigationVersion !== this.codeGraphNavigationVersion || requestKey !== this.codeGraphRequestKey()) return;
      if (this.state.codeGraph.mode === "diff" && this.state.codeGraph.baseRevision && this.state.codeGraph.headRevision) {
        await this.loadCodeDiffOverlay(repoId, this.state.codeGraph.baseRevision, this.state.codeGraph.headRevision, requestKey, navigationVersion);
      }
    } catch (error) {
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, {
        type: "LAYOUT_STATUS_SET",
        status: "failed",
        error: errorMessage(error),
      });
      this.render();
    }
  }

  private async refreshCodeGraphSnapshot(repoId: string, requestKey: string, navigationVersion: number): Promise<void> {
    if (!this.codeGraphAdapter) return;
    const code = this.state.codeGraph;
    const previousSnapshot = currentCodeGraphSnapshot(code);
    const mode = code.mode === "diff"
      ? code.path ? "file" : code.symbolKey ? "neighborhood" : "atlas"
      : code.mode;
    const snapshot = await this.codeGraphAdapter.getGraphSnapshot(repoId, {
      mode,
      path: code.path ?? undefined,
      symbolKey: code.symbolKey ?? undefined,
      depth: code.depth,
      aggregate: mode === "atlas" ? "directory" : undefined,
      includeStale: code.filters.freshness.includes("stale"),
    });
    if (this.destroyed || navigationVersion !== this.codeGraphNavigationVersion || requestKey !== this.codeGraphRequestKey()) return;
    this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "SNAPSHOT_LOADED", snapshot });
    if (this.state.codeGraph.snapshot !== previousSnapshot && (!previousSnapshot || !sameCodeGraphTopology(previousSnapshot, snapshot))) {
      this.invalidateCodeGraphLayout();
    } else {
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "LAYOUT_STATUS_SET", status: "idle" });
    }
    this.render();
  }

  private async loadCodeDiffOverlay(
    repoId: string,
    baseRevision: string,
    headRevision: string,
    requestKey: string,
    navigationVersion: number,
  ): Promise<void> {
    if (!this.codeGraphAdapter) return;
    const overlay = await this.codeGraphAdapter.getDiffOverlay(repoId, baseRevision, headRevision);
    if (this.destroyed || navigationVersion !== this.codeGraphNavigationVersion || requestKey !== this.codeGraphRequestKey()) return;
    this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "DIFF_LOADED", overlay });
    this.invalidateCodeGraphLayout();
    this.render();
  }

  private codeGraphRequestKey(): string {
    const code = this.state.codeGraph;
    return JSON.stringify([
      code.repoId,
      code.mode,
      code.path,
      code.symbolKey,
      code.depth,
      code.baseRevision,
      code.headRevision,
      code.filters.freshness.includes("stale"),
    ]);
  }

  private visibleCodeGraphSnapshot(): ReturnType<typeof applyCodeGraphFilters> | null {
    const snapshot = currentCodeGraphSnapshot(this.state.codeGraph);
    return snapshot ? applyCodeGraphFilters(snapshot, this.state.codeGraph.filters, this.state.codeGraph.diffOverlay) : null;
  }

  private bindCodeGraph(): void {
    const root = this.options.root.querySelector<HTMLElement>("[data-testid='code-graph-renderer']");
    if (!root) return;
    bindKnowledgeGraphListNavigation(this.options.root, {
      onSelect: this.onCodeNodeSelected,
      onFocus: this.onCodeNodeFocused,
    });
    const snapshot = this.visibleCodeGraphSnapshot();
    const stageSize = measureKnowledgeGraphStage(root);
    if (snapshot && !this.codeGraphLayout && this.state.codeGraph.layoutStatus === "idle") {
      this.scheduleCodeGraphLayout(stageSize);
    }
    const renderSnapshot = snapshot ? codeGraphSnapshotForRendering(snapshot, this.state.codeGraph.diffOverlay) : null;
    mountCodeGraphRenderer(root, {
      snapshot: renderSnapshot,
      layout: this.codeGraphLayout,
      selectedNodeIds: this.state.codeGraph.selectedNodeIds,
      view: this.codeGraphView,
      onSelect: this.onCodeNodeSelected,
      onFocus: this.onCodeNodeFocused,
      onSelectArea: this.onCodeAggregateSelected,
      nodeStyle: (node) => {
        const codeNode = snapshot?.nodes.find((candidate) => candidate.id === node.nodeId);
        return codeNode ? codeNodeVisualStyle(codeNode) : undefined;
      },
      edgeStyle: (edge) => edge.confidence
        ? codeEdgeVisualStyle({ confidence: edge.confidence as "exact" | "syntactic" | "heuristic" })
        : undefined,
    });
    void this.ensureSelectedCodeDetail();
  }

  private scheduleCodeGraphLayout(size = measureKnowledgeGraphStage(this.options.root)): void {
    const snapshot = this.visibleCodeGraphSnapshot();
    if (!snapshot || this.state.codeGraph.layoutStatus === "loading") return;
    const run = ++this.codeGraphLayoutRun;
    const renderSnapshot = codeGraphSnapshotForRendering(snapshot, this.state.codeGraph.diffOverlay);
    this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "LAYOUT_STATUS_SET", status: "loading" });
    const layoutScale = Math.min(2, Math.max(1.4, Math.sqrt(Math.max(1, renderSnapshot.nodes.length) / 20)));
    void this.graphLayoutAdapter.layout(renderSnapshot, {
      kind: codeGraphLayoutKindForMode(this.state.codeGraph.mode),
      focusedNodeId: this.state.codeGraph.selectedNodeIds[0] ?? null,
      width: Math.round(Math.max(1280, size.width * layoutScale)),
      height: Math.round(Math.max(900, size.height * layoutScale)),
    }).then((layout) => {
      if (this.destroyed || run !== this.codeGraphLayoutRun) return;
      this.codeGraphLayout = layout;
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "LAYOUT_STATUS_SET", status: "ready" });
      this.render();
    }).catch((error) => {
      if (this.destroyed || run !== this.codeGraphLayoutRun) return;
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, {
        type: "LAYOUT_STATUS_SET",
        status: "failed",
        error: errorMessage(error),
      });
      this.render();
    });
  }

  private onCodeAggregateSelected = (nodeId: string): void => {
    const nodes = this.visibleCodeGraphSnapshot()?.nodes ?? [];
    const node = nodes.find((candidate) => candidate.id === nodeId)
      ?? nodes.find((candidate) => candidate.kind === "community" && candidate.metrics.community_id === nodeId);
    if (node) this.drillIntoCodeNode(node);
  };

  private onCodeNodeSelected = (nodeId: string): void => {
    const node = this.visibleCodeGraphSnapshot()?.nodes.find((candidate) => candidate.id === nodeId);
    if (!node) return;
    if (node.kind === "directory" || node.kind === "file" || node.kind === "community") {
      this.drillIntoCodeNode(node);
      return;
    }
    this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "NODE_SELECTED", nodeId });
    this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "TARGET_SET", symbolKey: node.symbol_key ?? null });
    this.render();
  };

  private onCodeNodeFocused = (nodeId: string): void => {
    const node = this.visibleCodeGraphSnapshot()?.nodes.find((candidate) => candidate.id === nodeId);
    if (!node) return;
    if (node.symbol_key) {
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, {
        type: "DRILL_IN",
        breadcrumb: { kind: "symbol", id: node.symbol_key, label: node.label },
        mode: "neighborhood",
        symbolKey: node.symbol_key,
      });
      this.invalidateCodeGraphNavigation();
      void this.loadCodeGraph();
    } else {
      this.drillIntoCodeNode(node);
    }
  };

  private drillIntoCodeNode(node: CodeGraphNode): void {
    if (node.kind === "directory" || node.kind === "community") {
      const prefixes = node.kind === "community"
        ? [node.label, node.path_display].filter((value): value is string => Boolean(value))
        : [node.path_display ?? node.label];
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, {
        type: "FILTERS_SET",
        filters: { pathPrefixes: prefixes },
      });
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, {
        type: "DRILL_IN",
        breadcrumb: { kind: "directory", id: prefixes[0], label: node.label },
        mode: "atlas",
        path: prefixes[0],
        symbolKey: null,
      });
      this.invalidateCodeGraphNavigation();
      void this.loadCodeGraph();
      return;
    }
    const path = node.path_display ?? null;
    const breadcrumb = {
      kind: "file" as const,
      id: path ?? node.id,
      label: node.label,
    };
    this.state.codeGraph = codeGraphReducer(this.state.codeGraph, {
      type: "DRILL_IN",
      breadcrumb,
      mode: "file",
      path,
      symbolKey: null,
    });
    this.invalidateCodeGraphNavigation();
    void this.loadCodeGraph();
  }

  private stepBackCodeGraphView(): void {
    const code = this.state.codeGraph;
    if (code.breadcrumbs.length > 0) {
      this.state.codeGraph = codeGraphReducer(code, {
        type: "BREADCRUMB_POP",
        index: code.breadcrumbs.length > 1 ? code.breadcrumbs.length - 2 : undefined,
      });
    } else if (code.mode === "neighborhood") {
      this.state.codeGraph = codeGraphReducer(code, { type: "MODE_SET", mode: code.path ? "file" : "atlas" });
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "TARGET_SET", symbolKey: null });
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "SELECTION_SET", nodeIds: [] });
    } else {
      this.state.codeGraph = codeGraphReducer(code, { type: "MODE_SET", mode: "atlas" });
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "TARGET_SET", symbolKey: null, path: null });
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "SELECTION_SET", nodeIds: [] });
    }
    this.invalidateCodeGraphNavigation();
    void this.loadCodeGraph();
  }

  private async ensureSelectedCodeDetail(): Promise<void> {
    const adapter = this.codeGraphAdapter;
    const snapshot = this.visibleCodeGraphSnapshot();
    const selected = snapshot?.nodes.find((node) => this.state.codeGraph.selectedNodeIds.includes(node.id));
    if (!adapter || !selected?.symbol_key) return;
    const detailKey = `${snapshot?.repo_id ?? this.state.codeGraph.repoId}:${selected.symbol_key}`;
    const key = `${detailKey}:${snapshot?.cursor.partition}:${snapshot?.cursor.sequence}`;
    if (this.codeGraphSymbolRequest === key || this.state.codeGraph.symbolDetails[detailKey]) return;
    this.codeGraphSymbolRequest = key;
    try {
      const detail = await adapter.getSymbolDetail(
        this.state.codeGraph.repoId ?? snapshot!.repo_id,
        selected.symbol_key,
        { includeStale: selected.freshness === "stale" || this.state.codeGraph.filters.freshness.includes("stale") },
      );
      if (!this.destroyed && this.codeGraphSymbolRequest === key) {
        this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "SYMBOL_DETAIL_LOADED", detail });
        this.render();
      }
    } catch {
      // The structure list remains useful when a detail endpoint is unavailable.
    } finally {
      if (this.codeGraphSymbolRequest === key) this.codeGraphSymbolRequest = null;
    }
  }

  private selectedCodeSymbolDetail(): CodeSymbolDetail | null {
    const snapshot = this.visibleCodeGraphSnapshot();
    const selected = snapshot?.nodes.find((node) => this.state.codeGraph.selectedNodeIds.includes(node.id));
    if (!selected?.symbol_key || !this.state.codeGraph.repoId) return null;
    return this.state.codeGraph.symbolDetails[`${this.state.codeGraph.repoId}:${selected.symbol_key}`] ?? null;
  }

  private startEventSubscription(): void {
    if (this.eventSubscription?.transport === this.transport || typeof this.transport.events !== "function") {
      return;
    }
    this.stopEventSubscription();
    const subscription = { active: true, transport: this.transport } as {
      active: boolean;
      transport: GatewayReader;
      iterator?: AsyncIterator<GatewayEnvelope>;
    };
    this.eventSubscription = subscription;
    void this.consumeGatewayEvents(subscription);
  }

  private startLiveRefreshTimer(): void {
    if (this.liveRefreshTimer) {
      return;
    }
    this.liveRefreshTimer = setInterval(() => {
      if (this.destroyed || this.state.connectionMode !== "connected" || !this.state.selectedProjectId) {
        return;
      }
      void this.requestLiveRefresh();
    }, liveRefreshPollIntervalMs);
  }

  private stopLiveRefreshTimer(): void {
    if (!this.liveRefreshTimer) {
      return;
    }
    clearInterval(this.liveRefreshTimer);
    this.liveRefreshTimer = null;
  }

  private stopEventSubscription(): void {
    const subscription = this.eventSubscription;
    if (!subscription) return;
    subscription.active = false;
    this.eventSubscription = null;
    const closed = subscription.iterator?.return?.();
    void closed?.catch(() => undefined);
  }

  private async consumeGatewayEvents(subscription: {
    active: boolean;
    transport: GatewayReader;
    iterator?: AsyncIterator<GatewayEnvelope>;
  }): Promise<void> {
    try {
      const iterator = subscription.transport.events!(
        this.latestGatewayEventCursor ?? undefined,
      )[Symbol.asyncIterator]();
      subscription.iterator = iterator;
      while (subscription.active && !this.destroyed) {
        const next = await iterator.next();
        if (next.done) break;
        this.latestGatewayEventCursor = next.value.cursor;
        try {
          await this.onGatewayEvent(next.value);
        } catch (error) {
          console.warn("[opensymphony] gateway event processing failed after cursor advance", {
            baseUri: subscription.transport.baseUri,
            error: errorMessage(error),
          });
        }
      }
    } catch (error) {
      console.warn("[opensymphony] gateway event stream unavailable; using periodic refresh fallback", {
        baseUri: subscription.transport.baseUri,
        error: errorMessage(error),
      });
    } finally {
      if (this.eventSubscription === subscription) {
        this.eventSubscription = null;
      }
    }
  }

  private async onGatewayEvent(envelope: GatewayEnvelope): Promise<void> {
    let handledMemoryGraphUpdate = false;
    let handledCodeGraphUpdate = false;
    if (envelope.event_kind === "memory_graph_updated") {
      if (isMemoryGraphUpdatedEvent(envelope.payload)) {
        handledMemoryGraphUpdate = true;
        const selectedBundleId = this.state.knowledgeGraph.selectedBundleId;
        if (!selectedBundleId || selectedBundleId === envelope.payload.bundle_id) {
          this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "GRAPH_UPDATED", event: envelope.payload });
          this.render();
        }
        // A memory update can add capsules or PR evidence for completed
        // tasks; refresh the Completed pane's page (seq-guarded, no-op
        // without an adapter).
        if (this.options.mode === "desktop") {
          void this.loadCompletedTasks();
        }
      }
    }
    if (envelope.event_kind === "code_graph_updated" && isCodeGraphUpdatedEvent(envelope.payload)) {
      handledCodeGraphUpdate = true;
      const selectedRepoId = this.state.codeGraph.repoId;
      if (!selectedRepoId || selectedRepoId === envelope.payload.repo_id) {
        this.state.codeGraph = codeGraphReducer(this.state.codeGraph, {
          type: "GRAPH_UPDATED",
          repoId: envelope.payload.repo_id,
          updatedAt: envelope.payload.updated_at,
        });
        if (this.state.graphPaneView === "code") {
          void this.loadCodeGraph();
        }
        this.render();
      }
    }
    if (!handledMemoryGraphUpdate && !handledCodeGraphUpdate && !this.eventAffectsCurrentView(envelope)) {
      return;
    }
    await this.requestLiveRefresh();
  }

  private async requestLiveRefresh(): Promise<void> {
    if (this.liveRefreshInFlight) {
      this.liveRefreshQueued = true;
      return;
    }
    this.liveRefreshInFlight = true;
    try {
      do {
        this.liveRefreshQueued = false;
        try {
          const hadLiveRefreshFailures = this.liveRefreshFailureCount > 0;
          await this.refreshLiveGatewayData();
          this.liveRefreshFailureCount = 0;
          this.state.connectionMode = "connected";
          this.state.connectionMessage = `Connected to ${this.transport.baseUri || "same-origin gateway"}`;
          if (hadLiveRefreshFailures) {
            this.render();
          }
        } catch (error) {
          this.liveRefreshFailureCount += 1;
          console.warn("[opensymphony] live gateway refresh failed; event stream remains active", {
            baseUri: this.transport.baseUri,
            error: errorMessage(error),
          });
          if (this.liveRefreshFailureCount >= liveRefreshFailureThreshold) {
            this.state.connectionMessage = `Live data stale: ${errorMessage(error)}`;
            this.render();
          }
        }
      } while (this.liveRefreshQueued && !this.destroyed);
    } finally {
      this.liveRefreshInFlight = false;
    }
  }

  private eventAffectsCurrentView(envelope: GatewayEnvelope): boolean {
    const projectId = this.state.selectedProjectId;
    const entity = envelope.entity_ref;
    if (entity?.kind === "project") {
      return !projectId || entity.id === projectId;
    }
    if (entity?.kind === "run") {
      return !this.state.runDetail || entity.id === this.state.runDetail.run_id;
    }
    if (entity?.kind === "issue" || entity?.kind === "sub_issue") {
      return this.state.taskGraph?.nodes.some((node) =>
        node.node_id === entity.id
        || node.identifier === entity.identifier
        || node.identifier === entity.id
      ) ?? true;
    }
    return false;
  }

  private async refreshLiveGatewayData(): Promise<void> {
    // Snapshot the epoch: if the user navigates (task click, project switch,
    // diff selection) while this refresh is in flight, its results are stale
    // by definition and must be dropped rather than applied over the user's
    // choice. The next poll tick picks up the fresh selection.
    const epoch = this.interactionEpoch;
    const abandoned = () => this.destroyed || epoch !== this.interactionEpoch;

    const snapshot = await this.transport.snapshot();
    if (abandoned()) return;

    const previousProjectId = this.state.selectedProjectId;
    const projectStillPresent = snapshot.projects.some((project) => project.project_id === previousProjectId);
    const nextProjectId = projectStillPresent
      ? previousProjectId
      : snapshot.projects[0]?.project_id ?? "default";

    if (!nextProjectId) {
      this.state.snapshot = snapshot;
      this.state.selectedProjectId = nextProjectId;
      this.render();
      return;
    }

    const taskGraph = await this.transport.taskGraph(nextProjectId);
    if (abandoned()) return;

    // Resolve the preserved selection against the *current* state, not
    // values captured before the awaits above.
    const currentNodeId = this.state.selectedNodeId;
    const currentRunId = this.state.runDetail?.run_id ?? null;
    const selectedNode = taskGraph.nodes.find((node) => node.node_id === currentNodeId)
      ?? taskGraph.nodes.find((node) => node.run_id === currentRunId)
      ?? initialSelectedTaskNode(taskGraph.nodes, taskGraph.root_ids);

    const [overlays, selectedRun] = await Promise.all([
      this.fetchRunOverlays(taskGraph),
      // Backlog selections have no run: probing /runs/{backlog id} would
      // only surface a spurious "Run unavailable" message.
      selectedNode && selectedNode.state_category !== "backlog"
        ? this.fetchSelectedRunRefresh(selectedNode)
        : Promise.resolve(null),
    ]);
    if (abandoned()) return;

    // A task finishing (or reopening) moves it between the Current pane and
    // the Completed table, whose data loads separately — refresh that page
    // whenever the completed set could have changed so it never goes stale
    // until a manual reload. The signature watches both the task graph's
    // done nodes (so a done node appearing/leaving triggers it) and the
    // control-plane completed count (so a completion whose issue is absent
    // from the task graph — e.g. no project metadata — still triggers it).
    const completedSetChanged = this.options.mode === "desktop"
      && completedTasksSignature(this.state.snapshot, this.state.taskGraph)
        !== completedTasksSignature(snapshot, taskGraph);

    // Apply everything atomically: the previous data stays on screen until
    // the replacement is fully loaded, so panels never flash empty.
    this.state.snapshot = snapshot;
    this.state.selectedProjectId = nextProjectId;
    this.state.taskGraph = taskGraph;
    this.state.runOverlays = overlays;
    this.state.selectedNodeId = selectedNode?.node_id ?? null;
    if (selectedRun) {
      this.state.runDetail = selectedRun.runDetail;
      this.state.runOverlays.set(selectedRun.runDetail.run_id, selectedRun.runDetail);
      this.applyRunDetailBundle(selectedRun.bundle);
    }
    if (completedSetChanged) {
      void this.loadCompletedTasks();
    }
    if (this.state.graphPaneView === "knowledge") {
      await this.loadKnowledgeGraph();
      if (abandoned()) return;
    } else if (this.state.graphPaneView === "code") {
      await this.loadCodeGraph();
      if (abandoned()) return;
    }
    this.render();
  }

  private async fetchSelectedRunRefresh(
    node: TaskGraphNode,
  ): Promise<{ runDetail: RunDetail; bundle: RunDetailBundle } | null> {
    const runId = runIdForNode(node);
    try {
      const [runDetail, bundle] = await Promise.all([
        this.transport.runDetail(runId),
        this.fetchRunDetailBundle(runId, this.state.selectedDiffPath),
      ]);
      return { runDetail, bundle };
    } catch (error) {
      // Same as openRun: an untracked active node has no run detail by
      // design, so a background refresh miss is not worth a banner.
      if (nodeHasRun(node)) {
        this.state.connectionMessage = `Run ${runId} unavailable: ${errorMessage(error)}`;
      }
      return null;
    }
  }

  private async fetchRunOverlays(taskGraph: TaskGraphSnapshot): Promise<Map<string, RunDetail>> {
    const runIds = new Set(taskGraph.nodes.map((node) => node.run_id).filter((id): id is string => Boolean(id)));
    const overlays = new Map<string, RunDetail>();
    if (runIds.size === 0) {
      return overlays;
    }
    await Promise.all(
      Array.from(runIds).map(async (runId) => {
        try {
          const run = await this.transport.runDetail(runId);
          overlays.set(runId, run);
        } catch {
          // Ignore missing runs; overlay will be absent.
        }
      }),
    );
    return overlays;
  }

  private async openRun(node: TaskGraphNode): Promise<void> {
    const runId = runIdForNode(node);
    this.interactionEpoch += 1;
    this.diffSelectSeq += 1;
    const openSeq = ++this.runOpenSeq;
    this.state.selectedNodeId = node.node_id;
    this.state.loading = true;
    this.render();
    try {
      // The run detail and every evidence pane load concurrently: the click
      // latency is one round trip, not six queued ones.
      const [runDetail, bundle] = await Promise.all([
        this.transport.runDetail(runId),
        this.fetchRunDetailBundle(runId, null),
      ]);
      if (this.destroyed || openSeq !== this.runOpenSeq) {
        return;
      }
      this.state.runDetail = runDetail;
      this.state.runOverlays.set(runId, runDetail);
      this.state.evidenceView = "diff";
      this.state.expandedActivityEvents = new Set();
      this.state.collapsedActivityEvents = new Set();
      this.applyRunDetailBundle(bundle);
    } catch (error) {
      if (this.destroyed || openSeq !== this.runOpenSeq) {
        return;
      }
      this.state.runDetail = null;
      this.state.runFiles = null;
      this.state.runDiff = null;
      this.state.evidenceView = "diff";
      this.state.runEvents = null;
      this.state.expandedActivityEvents = new Set();
      this.state.collapsedActivityEvents = new Set();
      this.state.runValidation = null;
      this.state.runApprovals = null;
      this.state.selectedDiffPath = null;
      // An active issue the control plane does not track yet (e.g. just
      // promoted from Backlog, no run dispatched) has no run detail to
      // serve — the miss is expected, so the selection stays graph-local
      // without a spurious "Run unavailable" banner. Real lookup failures
      // on run-carrying nodes still surface.
      if (nodeHasRun(node)) {
        this.state.connectionMessage = `Run ${runId} unavailable: ${errorMessage(error)}`;
      }
    }
    this.state.loading = false;
    this.render();
  }

  private async selectDiffFile(path: string): Promise<void> {
    // Invalidates background refreshes and older diff fetches, but not an
    // in-flight openRun: clicking a (possibly stale) file must never cancel
    // a task the user just opened.
    this.interactionEpoch += 1;
    const seq = ++this.diffSelectSeq;
    this.state.selectedDiffPath = path;
    this.state.evidenceView = "diff";
    this.render();
    const runId = this.state.runDetail?.run_id;
    if (runId && typeof this.transport.runDiffs === "function") {
      let runDiff: FileDiffPage | null = null;
      let warning: string | null = null;
      try {
        runDiff = await this.transport.runDiffs!(runId, path);
      } catch (error) {
        warning = `Diff unavailable: ${errorMessage(error)}`;
      }
      // Drop the result if a newer diff click or task open superseded this
      // fetch, or if the shown run changed while it was in flight.
      if (this.destroyed || seq !== this.diffSelectSeq || this.state.runDetail?.run_id !== runId) {
        return;
      }
      this.state.runDiff = runDiff;
      if (warning) {
        this.state.connectionMessage = warning;
      }
    } else if (runId) {
      this.state.runDiff = null;
      this.state.connectionMessage = "Diff endpoint unavailable for the active transport";
    }
    this.render();
  }

  private selectEvidenceView(view: AppState["evidenceView"]): void {
    this.state.evidenceView = view;
    this.render();
  }

  private selectGraphPaneView(view: GraphPaneView): void {
    this.state.graphPaneView = view;
    this.render();
    if (view === "knowledge") {
      void this.loadKnowledgeGraph();
    } else if (view === "code") {
      void this.loadCodeGraph();
    }
  }

  private bindKnowledgeGraph(): void {
    const root = this.options.root.querySelector<HTMLElement>("[data-testid='knowledge-graph-renderer']");
    if (!root) return;
    // Navigation affordances span the hero (breadcrumb) and the lower
    // columns (entity list, inspector capsule), so bind at the app root.
    this.bindKnowledgeGraphNavigation(this.options.root);
    bindKnowledgeGraphListNavigation(this.options.root, {
      onSelect: this.onKnowledgeNodeSelected,
      onFocus: this.onKnowledgeNodeFocused,
    });
    void this.ensureSelectedConceptDetail();
    const snapshot = visibleGraphSnapshot(this.state.knowledgeGraph);
    const stageSize = measureKnowledgeGraphStage(root);
    if (
      snapshot
      && this.state.knowledgeGraphLayout
      && this.state.knowledgeGraph.layoutStatus === "ready"
      && this.knowledgeGraphLayoutSize
      && stageSizeChanged(this.knowledgeGraphLayoutSize, stageSize)
    ) {
      this.state.knowledgeGraphLayout = null;
      this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "LAYOUT_STATUS_SET", status: "idle" });
      this.knowledgeGraphLayoutSize = null;
      this.scheduleKnowledgeGraphLayout(stageSize);
      return;
    }
    if (snapshot && !this.state.knowledgeGraphLayout && this.state.knowledgeGraph.layoutStatus === "idle") {
      this.scheduleKnowledgeGraphLayout(stageSize);
    }
    mountKnowledgeGraphRenderer(root, {
      snapshot,
      layout: this.state.knowledgeGraphLayout,
      selectedNodeIds: this.state.knowledgeGraph.selectedNodeIds,
      view: this.knowledgeGraphView,
      onSelect: this.onKnowledgeNodeSelected,
      onFocus: this.onKnowledgeNodeFocused,
      onSelectArea: (areaId) => {
        this.drillIntoKnowledgeArea(areaId);
      },
    });
  }

  private onKnowledgeNodeSelected = (nodeId: string): void => {
    this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "SELECTION_SET", nodeIds: [nodeId] });
    this.render();
  };

  private onKnowledgeNodeFocused = (nodeId: string): void => {
    this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "NODE_FOCUSED", nodeId });
    this.render();
    if (this.state.knowledgeGraph.mode !== "neighborhood") return;
    this.invalidateKnowledgeGraphLayout();
  };

  private installBrowserGraphLayoutAdapter(): void {
    if (typeof Worker === "undefined") return;
    void import("./graph-layout-worker-factory.js").then(({ createBrowserGraphLayoutAdapter }) => {
      if (this.destroyed) return;
      this.replaceGraphLayoutAdapter(createBrowserGraphLayoutAdapter());
    }).catch((error: unknown) => {
      console.warn("Knowledge graph layout worker unavailable; using synchronous fallback.", error);
      this.graphLayoutAdapter = createGraphLayoutAdapter(() => null);
    });
  }

  private replaceGraphLayoutAdapter(adapter: GraphLayoutAdapter): void {
    if (this.state.knowledgeGraph.layoutStatus === "loading") {
      this.pendingGraphLayoutAdapter?.dispose();
      this.pendingGraphLayoutAdapter = adapter;
      return;
    }
    this.graphLayoutAdapter.dispose();
    this.graphLayoutAdapter = adapter;
  }

  private applyPendingGraphLayoutAdapter(): void {
    if (!this.pendingGraphLayoutAdapter || this.state.knowledgeGraph.layoutStatus === "loading") return;
    const adapter = this.pendingGraphLayoutAdapter;
    this.pendingGraphLayoutAdapter = null;
    this.graphLayoutAdapter.dispose();
    this.graphLayoutAdapter = adapter;
  }

  private scheduleKnowledgeGraphLayout(size = measureKnowledgeGraphStage(this.options.root)): void {
    const snapshot = visibleGraphSnapshot(this.state.knowledgeGraph);
    if (!snapshot || this.state.knowledgeGraph.layoutStatus === "loading") return;
    const run = ++this.graphLayoutRun;
    this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "LAYOUT_STATUS_SET", status: "loading" });
    const { width, height } = size;
    // The layout runs on a canvas larger than the stage: the 3D camera fits
    // whatever extent the layout produces, and the extra room lets the force
    // layout separate clusters instead of packing them into the viewport.
    const layoutScale = Math.min(2, Math.max(1.4, Math.sqrt((visibleGraphSnapshot(this.state.knowledgeGraph)?.nodes.length ?? 60) / 40)));
    void this.graphLayoutAdapter.layout(snapshot, {
      kind: graphLayoutKindForMode(this.state.knowledgeGraph.mode),
      focusedNodeId: this.state.knowledgeGraph.focusedNodeId,
      width: Math.round(Math.max(1280, width * layoutScale)),
      height: Math.round(Math.max(900, height * layoutScale)),
    }).then((layout) => {
      if (this.destroyed || run !== this.graphLayoutRun) return;
      this.state.knowledgeGraphLayout = layout;
      this.knowledgeGraphLayoutSize = { width, height };
      this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "LAYOUT_STATUS_SET", status: "ready" });
      this.applyPendingGraphLayoutAdapter();
      this.render();
    }).catch((error) => {
      if (this.destroyed || run !== this.graphLayoutRun) return;
      this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, {
        type: "LAYOUT_STATUS_SET",
        status: "failed",
        error: errorMessage(error),
      });
      this.applyPendingGraphLayoutAdapter();
      this.render();
    });
  }

  private toggleActivityEvent(eventKey: string): void {
    const expanded = new Set(this.state.expandedActivityEvents);
    const collapsed = new Set(this.state.collapsedActivityEvents);
    const currentlyExpanded = this.options.root
      .querySelector<HTMLElement>(`[data-activity-toggle="${cssEscape(eventKey)}"]`)
      ?.getAttribute("aria-expanded") === "true";
    if (currentlyExpanded) {
      expanded.delete(eventKey);
      collapsed.add(eventKey);
      this.state.expandedActivityEvents = expanded;
      this.state.collapsedActivityEvents = collapsed;
      this.render();
      return;
    }
    if (expanded.has(eventKey)) {
      expanded.delete(eventKey);
    } else {
      expanded.add(eventKey);
    }
    collapsed.delete(eventKey);
    this.state.expandedActivityEvents = expanded;
    this.state.collapsedActivityEvents = collapsed;
    this.render();
  }

  private startPaneResize(handle: string | undefined, event: PointerEvent): void {
    if (!isWorkspacePaneResizeHandle(handle)) {
      return;
    }
    const shell = (event.currentTarget as HTMLElement).closest<HTMLElement>(".os-lower-columns");
    if (!shell) {
      return;
    }
    const width = shell.getBoundingClientRect().width;
    if (width <= 0) {
      return;
    }
    event.preventDefault();
    const startX = event.clientX;
    const surface = this.state.graphPaneView;
    const start = { ...this.currentWorkspacePaneSizes() };
    const move = (moveEvent: PointerEvent) => {
      const delta = ((moveEvent.clientX - startX) / width) * 100;
      const nextSizes = resizeWorkspacePanes(start, handle, delta);
      this.state.workspacePaneSizes = {
        ...this.state.workspacePaneSizes,
        [surface]: nextSizes,
      };
      applyWorkspacePaneStyle(shell, nextSizes);
    };
    const done = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", done);
      this.render();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", done, { once: true });
    (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  }

  private onPaneResizeKey(handle: string | undefined, event: KeyboardEvent): void {
    if (!isWorkspacePaneResizeHandle(handle)) {
      return;
    }
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") {
      return;
    }
    event.preventDefault();
    const delta = event.key === "ArrowRight" ? 2 : -2;
    this.state.workspacePaneSizes = {
      ...this.state.workspacePaneSizes,
      [this.state.graphPaneView]: resizeWorkspacePanes(this.currentWorkspacePaneSizes(), handle, delta),
    };
    this.render();
  }

  /** Drag a task-graph side pane (Completed or Backlog) to a new px width. */
  private startTaskPaneResize(pane: string | undefined, event: PointerEvent): void {
    if (!isTaskSidePane(pane)) {
      return;
    }
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = this.state.taskPaneSizes[pane];
    const bounds = taskPaneSizeBounds[pane];
    const paneEl = this.options.root.querySelector<HTMLElement>(`[data-tg-pane="${pane}"]`);
    const move = (moveEvent: PointerEvent) => {
      // Completed sits left of its handle (drag right grows it); Backlog sits
      // right of its handle (drag right shrinks it).
      const rawDelta = moveEvent.clientX - startX;
      const delta = pane === "done" ? rawDelta : -rawDelta;
      const next = clamp(startWidth + delta, bounds.min, bounds.max);
      this.state.taskPaneSizes = { ...this.state.taskPaneSizes, [pane]: next };
      if (paneEl) {
        paneEl.style.flex = `0 0 ${next}px`;
      }
      this.scheduleCrossLinksReposition();
    };
    const done = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", done);
      this.render();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", done, { once: true });
    (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  }

  private onTaskPaneResizeKey(pane: string | undefined, event: KeyboardEvent): void {
    if (!isTaskSidePane(pane)) {
      return;
    }
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") {
      return;
    }
    event.preventDefault();
    const bounds = taskPaneSizeBounds[pane];
    // Arrow keys mirror the divider's physical motion: Right widens the pane
    // left of the handle (Completed) and narrows the one right of it (Backlog).
    const rightward = event.key === "ArrowRight" ? taskPaneResizeStep : -taskPaneResizeStep;
    const delta = pane === "done" ? rightward : -rightward;
    const next = clamp(this.state.taskPaneSizes[pane] + delta, bounds.min, bounds.max);
    this.state.taskPaneSizes = { ...this.state.taskPaneSizes, [pane]: next };
    this.render();
  }

  /** Drag the divider between the graph pane and the lower row to a new px height. */
  private startLowerRowResize(handle: string | undefined, event: PointerEvent): void {
    if (handle !== "lower") {
      return;
    }
    event.preventDefault();
    const startY = event.clientY;
    const startHeight = this.state.lowerRowHeight;
    const shell = this.options.root.querySelector<HTMLElement>(".os-lower-columns");
    // The graph pane above is fixed-height, so a growing row extends downward
    // and the handle would drift away from the cursor. Scroll the page by the
    // same amount the row actually grows so the handle tracks the cursor and
    // the divider + panes visually move up as the row expands.
    const scroller = scrollContainerFor(shell);
    const startScrollTop = scroller.get();
    const move = (moveEvent: PointerEvent) => {
      // The divider sits above the row, so dragging up (clientY decreases)
      // grows the row below and dragging down shrinks it.
      const next = clamp(startHeight - (moveEvent.clientY - startY), lowerRowHeightBounds.min, lowerRowHeightBounds.max);
      this.state.lowerRowHeight = next;
      shell?.style.setProperty("--os-lower-row-height", `${next}px`);
      scroller.set(startScrollTop + (next - startHeight));
    };
    const done = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", done);
      this.render();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", done, { once: true });
    (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  }

  private onLowerRowResizeKey(handle: string | undefined, event: KeyboardEvent): void {
    if (handle !== "lower") {
      return;
    }
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") {
      return;
    }
    event.preventDefault();
    // The divider sits above the row: ArrowUp grows it, ArrowDown shrinks it,
    // matching the pointer drag direction.
    const delta = event.key === "ArrowUp" ? lowerRowResizeStep : -lowerRowResizeStep;
    this.state.lowerRowHeight = clamp(this.state.lowerRowHeight + delta, lowerRowHeightBounds.min, lowerRowHeightBounds.max);
    this.render();
  }

  private async dispatchRunAction(action: RunAction): Promise<void> {
    const runId = this.state.runDetail?.run_id;
    if (!runId) return;
    const transport = this.transport as unknown as {
      cancelRun?: (id: string) => Promise<ActionReceipt>;
      resumeRun?: (id: string) => Promise<ActionReceipt>;
      openWorkspace?: (id: string) => Promise<ActionReceipt>;
      debugRun?: (id: string) => Promise<ActionReceipt>;
    };
    let receipt: ActionReceipt | null = null;
    try {
      switch (action) {
        case "cancel":
          receipt = await (transport.cancelRun?.(runId) ?? unsupportedAction(action));
          break;
        case "resume":
          receipt = await (transport.resumeRun?.(runId) ?? unsupportedAction(action));
          break;
        case "open_workspace":
          receipt = await (transport.openWorkspace?.(runId) ?? unsupportedAction(action));
          break;
        case "debug":
          receipt = await (transport.debugRun?.(runId) ?? unsupportedAction(action));
          break;
        default:
          receipt = await unsupportedAction(action);
      }
      if (!receipt) return;
      this.state.lastActionReceipt = receipt;
      this.state.auditTrail.push({
        timestamp: new Date().toISOString(),
        actor: "operator",
        action,
        target: runId,
        status: receipt.status,
        details: receipt.reason,
      });
    } catch (error) {
      this.state.auditTrail.push({
        timestamp: new Date().toISOString(),
        actor: "operator",
        action,
        target: runId,
        status: "failed",
        details: errorMessage(error),
      });
    }
    this.render();
  }

  private async submitApprovalDecision(
    approvalId: string,
    decision: ApprovalDecision,
    explanation?: string,
  ): Promise<void> {
    const transport = this.transport as unknown as {
      approvalDecision?: (id: string, d: ApprovalDecision, exp?: string) => Promise<ActionReceipt>;
    };
    try {
      const receipt = await (transport.approvalDecision?.(approvalId, decision, explanation) ??
        unsupportedAction("approval_decision"));
      this.state.lastActionReceipt = receipt;
      this.state.auditTrail.push({
        timestamp: new Date().toISOString(),
        actor: "operator",
        action: `approval_${decision}`,
        target: approvalId,
        status: receipt.status,
        details: explanation,
      });
    } catch (error) {
      this.state.auditTrail.push({
        timestamp: new Date().toISOString(),
        actor: "operator",
        action: `approval_${decision}`,
        target: approvalId,
        status: "failed",
        details: errorMessage(error),
      });
    }
    this.render();
  }

  private async selectProject(projectId: string): Promise<void> {
    this.interactionEpoch += 1;
    this.runOpenSeq += 1;
    this.diffSelectSeq += 1;
    this.state.selectedProjectId = projectId;
    this.state.loading = true;
    this.render();
    await this.loadTaskGraph(projectId);
    this.state.loading = false;
    this.render();
  }

  private async selectProfile(profileId: string): Promise<void> {
    const controller = this.options.profileController;
    const profile = this.state.profiles.find((candidate) => candidate.id === profileId);
    if (!profile) {
      return;
    }
    const wasActive = profile.active || this.state.activeProfileId === profileId;
    this.state.activeProfileId = profileId;
    this.state.gatewayDraft = profile.gatewayUrl;
    this.state.profiles = this.state.profiles.map((candidate) => ({
      ...candidate,
      active: candidate.id === profileId,
    }));

    if (controller && !wasActive) {
      await controller.setActiveProfile(profileId).catch((error) => {
        this.state.connectionMessage = `Profile selection failed: ${errorMessage(error)}`;
      });
    }
    if (this.options.onGatewayUrlChanged) {
      this.stopEventSubscription();
      await this.transport.close().catch(() => undefined);
      this.transport = await this.options.onGatewayUrlChanged(profile.gatewayUrl);
      this.graphAdapter = this.options.onGraphGatewayUrlChanged?.(profile.gatewayUrl) ?? this.graphAdapter;
      this.codeGraphAdapter = this.options.onCodeGraphGatewayUrlChanged?.(profile.gatewayUrl) ?? this.codeGraphAdapter;
      this.resetKnowledgeGraph();
      this.resetCodeGraph();
    }
    await this.refresh();
  }

  private async saveProfile(): Promise<void> {
    const controller = this.options.profileController;
    if (!controller) {
      return;
    }
    const gatewayInput = this.options.root.querySelector<HTMLInputElement>("[data-profile-gateway]");
    const kindInput = this.options.root.querySelector<HTMLSelectElement>("[data-profile-kind]");
    const labelInput = this.options.root.querySelector<HTMLInputElement>("[data-profile-label]");
    const selectedProfileId = this.valueOf<HTMLSelectElement>("[data-profile-select]")
      || this.state.activeProfileId
      || undefined;
    const gatewayUrl = (gatewayInput?.value ?? "").trim();
    const activeProfile = this.state.profiles.find((profile) => profile.id === selectedProfileId);
    const label = (labelInput?.value ?? "").trim() || activeProfile?.label || "Local Gateway";
    const kind = editableProfileKindFromValue(kindInput?.value, this.options.mode);
    if (!gatewayUrl) {
      this.state.connectionMessage = "Profile URL is required";
      this.render();
      return;
    }

    try {
      const saved = await controller.storeProfile({
        id: selectedProfileId,
        label,
        kind,
        gatewayUrl,
      });
      await controller.setActiveProfile(saved.id);
      if (this.options.onGatewayUrlChanged) {
        this.stopEventSubscription();
        await this.transport.close().catch(() => undefined);
        this.transport = await this.options.onGatewayUrlChanged(saved.gatewayUrl);
        this.graphAdapter = this.options.onGraphGatewayUrlChanged?.(saved.gatewayUrl) ?? this.graphAdapter;
        this.codeGraphAdapter = this.options.onCodeGraphGatewayUrlChanged?.(saved.gatewayUrl) ?? this.codeGraphAdapter;
        this.resetKnowledgeGraph();
        this.resetCodeGraph();
      }
      await this.refresh();
    } catch (error) {
      this.state.connectionMode = "failed";
      this.state.connectionMessage = `Profile save failed: ${errorMessage(error)}`;
      this.render();
    }
  }

  private async createProfileDraft(): Promise<void> {
    const controller = this.options.profileController;
    if (!controller) {
      return;
    }
    const activeProfile = this.state.profiles.find((profile) => profile.id === this.state.activeProfileId)
      ?? defaultUiProfiles(this.transport.baseUri)[0];
    try {
      const saved = await controller.storeProfile({
        label: "New gateway",
        kind: activeProfile.kind,
        gatewayUrl: activeProfile.gatewayUrl || this.transport.baseUri,
      });
      const active = await controller.setActiveProfile(saved.id).catch(() => saved);
      this.state.profiles = [
        ...this.state.profiles.filter((profile) => profile.id !== active.id),
        active,
      ].map((profile) => ({
        ...profile,
        active: profile.id === active.id,
      }));
      this.state.activeProfileId = active.id;
      this.state.gatewayDraft = active.gatewayUrl;
      this.state.profilePanelExpanded = true;
      this.render();
    } catch (error) {
      this.state.connectionMessage = `Profile create failed: ${errorMessage(error)}`;
      this.render();
    }
  }

  private async removeProfile(): Promise<void> {
    const controller = this.options.profileController;
    if (!controller) {
      return;
    }
    const activeProfileId = this.valueOf<HTMLSelectElement>("[data-profile-select]")
      || this.state.activeProfileId;
    if (!activeProfileId) {
      return;
    }
    const profile = this.state.profiles.find((candidate) => candidate.id === activeProfileId);
    if (!this.confirmProfileRemoval(profile?.label ?? "this connection profile")) {
      return;
    }
    try {
      const profiles = await controller.removeProfile(activeProfileId);
      const active = profiles.find((profile) => profile.active) ?? profiles[0] ?? null;
      this.state.profiles = profiles;
      this.state.activeProfileId = active?.id ?? null;
      this.state.gatewayDraft = active?.gatewayUrl ?? this.transport.baseUri;
      if (active && this.options.onGatewayUrlChanged) {
        this.stopEventSubscription();
        await this.transport.close().catch(() => undefined);
        this.transport = await this.options.onGatewayUrlChanged(active.gatewayUrl);
        this.graphAdapter = this.options.onGraphGatewayUrlChanged?.(active.gatewayUrl) ?? this.graphAdapter;
        this.codeGraphAdapter = this.options.onCodeGraphGatewayUrlChanged?.(active.gatewayUrl) ?? this.codeGraphAdapter;
        this.resetKnowledgeGraph();
        this.resetCodeGraph();
      }
      this.render();
    } catch (error) {
      this.state.connectionMessage = `Profile delete failed: ${errorMessage(error)}`;
      this.render();
    }
  }

  private async selectModelProfile(profileId: string): Promise<void> {
    const profile = this.state.modelProfiles.find((candidate) => candidate.id === profileId);
    if (!profile) {
      return;
    }
    this.state.activeModelProfileId = profileId;
    this.state.modelProfileError = null;
    this.render();
  }

  private async saveModelProfile(): Promise<void> {
    const controller = this.options.modelProfileController;
    if (!controller) {
      return;
    }
    const profiles = modelProfilesWithDefaults(this.state.modelProfiles);
    const selectedProfileId = this.valueOf<HTMLSelectElement>("[data-model-profile-select]")
      || this.state.activeModelProfileId;
    const active = activeModelProfile(profiles, selectedProfileId) ?? profiles[0] ?? null;
    const mode = modelModeFromValue(this.valueOf<HTMLSelectElement>("[data-model-mode]"));
    const baseProfile = active ?? createModelProfile(mode);
    const label = this.valueOf<HTMLInputElement>("[data-model-label]").trim() || active?.label || "Model profile";
    const model = this.valueOf<HTMLInputElement>("[data-model-name]").trim();
    if (!model) {
      this.state.modelProfileError = "Model string is required";
      this.render();
      return;
    }

    const credentialInput = this.valueOf<HTMLInputElement>("[data-model-credential-ref]").trim();
    const apiKeyRef = credentialInput || null;
    const credentialStorage = mode === "subscription"
      ? "openhands_auth_directory"
      : baseProfile.credentialStorage;
    const subscriptionCredentialDefaults = defaultModelProfiles()
      .find((profile) => profile.mode === "subscription")!
      .subscriptionCredential!;
    const subscriptionCredential = mode === "subscription"
      ? {
          ...subscriptionCredentialDefaults,
          ...baseProfile.subscriptionCredential,
          provider: baseProfile.subscriptionCredential?.provider
            || subscriptionCredentialDefaults.provider,
          authDirectoryEnv: credentialInput || null,
        }
      : null;
    const credentialError = mode === "api_key"
      ? validateStoredCredentialRef(credentialInput, credentialStorage)
      : validateSubscriptionCredential(subscriptionCredential);
    if (credentialError) {
      this.state.modelProfileError = credentialError;
      this.render();
      return;
    }
    const activeFlag = this.options.root.querySelector<HTMLInputElement>("[data-model-active]")?.checked ?? baseProfile.active;
    const profile: ModelConfigurationProfile = {
      ...baseProfile,
      id: baseProfile.id,
      label,
      mode,
      owner: modelOwnerFromValue(this.valueOf<HTMLSelectElement>("[data-model-owner]")),
      baseUrl: this.valueOf<HTMLInputElement>("[data-model-base-url]").trim(),
      model,
      apiKeyRef: mode === "api_key" ? apiKeyRef : null,
      subscriptionCredential,
      credentialStorage,
      harnesses: splitList(this.valueOf<HTMLInputElement>("[data-model-harnesses]")),
      active: activeFlag,
    };

    try {
      const saved = await controller.storeProfile(profile);
      if (saved.active) {
        await controller.setActiveProfile(saved.id);
      }
      this.state.modelProfiles = upsertModelProfile(profiles, saved).map((profile) => {
        if (saved.active) {
          return { ...profile, active: profile.id === saved.id };
        }
        return profile.id === saved.id ? saved : profile;
      });
      this.state.activeModelProfileId = saved.active
        ? saved.id
        : this.state.modelProfiles.find((profile) => profile.active)?.id ?? null;
      this.state.modelProfileError = null;
      this.render();
    } catch (error) {
      this.state.modelProfileError = `Model profile save failed: ${errorMessage(error)}`;
      this.render();
    }
  }

  private toggleSettingsPanel(panel: "connection" | "model"): void {
    if (panel === "connection") {
      this.state.profilePanelExpanded = !this.state.profilePanelExpanded;
    } else {
      this.state.modelPanelExpanded = !this.state.modelPanelExpanded;
    }
    this.render();
  }

  private changeModelProfileMode(mode: ModelCredentialMode): void {
    const profiles = modelProfilesWithDefaults(this.state.modelProfiles);
    const selectedProfileId = this.valueOf<HTMLSelectElement>("[data-model-profile-select]")
      || this.state.activeModelProfileId;
    const current = activeModelProfile(profiles, selectedProfileId)
      ?? profiles[0]
      ?? createModelProfile(mode);
    const subscriptionCredentialDefaults = defaultModelProfiles()
      .find((profile) => profile.mode === "subscription")!
      .subscriptionCredential!;
    const nextProfile: ModelConfigurationProfile = {
      ...current,
      mode,
      apiKeyRef: null,
      subscriptionCredential: mode === "subscription"
        ? {
            ...subscriptionCredentialDefaults,
            ...current.subscriptionCredential,
            authDirectoryEnv: null,
          }
        : null,
      credentialStorage: mode === "subscription"
        ? "openhands_auth_directory"
        : current.credentialStorage,
    };
    this.state.modelProfiles = upsertModelProfile(profiles, nextProfile);
    this.state.activeModelProfileId = nextProfile.id;
    this.state.modelProfileError = null;
    this.render();
  }

  private confirmProfileRemoval(label: string): boolean {
    const view = this.options.root.ownerDocument.defaultView;
    if (!view?.confirm) {
      return true;
    }
    return view.confirm(`Delete profile "${label}"?`);
  }

  private async createModelProfileDraft(): Promise<void> {
    const controller = this.options.modelProfileController;
    if (!controller) {
      return;
    }
    const active = activeModelProfile(this.state.modelProfiles, this.state.activeModelProfileId);
    const draft = createModelProfile(active?.mode ?? "api_key");
    try {
      const saved = await controller.storeProfile(draft);
      this.state.modelProfiles = [
        ...this.state.modelProfiles.filter((profile) => profile.id !== saved.id),
        saved,
      ];
      this.state.activeModelProfileId = saved.id;
      this.state.modelProfileError = null;
      this.render();
    } catch (error) {
      this.state.modelProfileError = `Model profile create failed: ${errorMessage(error)}`;
      this.render();
    }
  }

  private async removeModelProfile(): Promise<void> {
    const controller = this.options.modelProfileController;
    if (!controller) {
      return;
    }
    const active = activeModelProfile(this.state.modelProfiles, this.state.activeModelProfileId);
    if (!active) {
      return;
    }
    if (!this.confirmProfileRemoval(active.label)) {
      return;
    }
    try {
      const profiles = await controller.removeProfile(active.id);
      const nextActive = profiles.find((profile) => profile.active) ?? profiles[0] ?? null;
      this.state.modelProfiles = profiles;
      this.state.activeModelProfileId = nextActive?.id ?? null;
      this.state.modelProfileError = null;
      this.render();
    } catch (error) {
      this.state.modelProfileError = `Model profile remove failed: ${errorMessage(error)}`;
      this.render();
    }
  }

  private valueOf<T extends HTMLInputElement | HTMLSelectElement>(selector: string): string {
    return this.options.root.querySelector<T>(selector)?.value ?? "";
  }

  private render(): void {
    if (this.destroyed) {
      return;
    }
    const scrollPositions = captureShellScrollPositions(this.options.root);
    const title = this.options.title ?? "OpenSymphony";
    // Dispose whenever the upcoming render will not include the Knowledge
    // Graph surface — not only on pane switches: the planning view and auth
    // placeholders also drop the canvas, and morphing it away without
    // disposal would leak the WebGL context and scheduled draws.
    const rendersKnowledgeGraph = this.state.graphPaneView === "knowledge"
      && this.state.activeView === "dashboard"
      && this.state.authState === "open";
    const rendersCodeGraph = this.state.graphPaneView === "code"
      && this.state.activeView === "dashboard"
      && this.state.authState === "open";
    if (!rendersKnowledgeGraph) {
      disposeKnowledgeGraphRenderer(this.options.root);
    }
    if (!rendersCodeGraph) {
      disposeCodeGraphRenderer(this.options.root);
    }
    // Morph the new markup into the live DOM instead of rebuilding it with
    // innerHTML: only changed nodes mutate, so focus, input state, scroll
    // offsets, the knowledge-graph canvas bitmap, and attached listeners all
    // survive background refreshes.
    morphChildren(this.options.root, `
      <style>${appShellStyles()}</style>
      <main class="os-app" data-opensymphony-app-shell="mounted" data-mode="${this.options.mode}" data-auth-state="${this.state.authState}">
        <header class="os-topbar">
          <div>
            <h1>${escapeHtml(title)}</h1>
            <p>${escapeHtml(this.state.connectionMessage)}</p>
          </div>
          <div class="os-view-tabs">
            <button type="button" class="os-view-tab ${this.state.activeView === "dashboard" ? "os-view-tab-active" : ""}" data-plan-view="dashboard">Dashboard</button>
            <button type="button" class="os-view-tab os-view-tab-preview ${this.state.activeView === "planning" ? "os-view-tab-active" : ""}" data-plan-view="planning" title="Planning is under construction">Planning<span class="os-tab-badge">WIP</span></button>
          </div>
          ${this.renderTopbarStatusStrip()}
        </header>
        <section class="os-grid">
          ${this.renderViewContent()}
        </section>
        ${this.renderGlobalModals()}
      </main>
    `);
    this.bindEvents();
    restoreShellScrollPositions(this.options.root, scrollPositions);
  }

  private renderViewContent(): string {
    if (this.state.authState !== "open") {
      return this.renderAuthPlaceholder();
    }
    if (this.state.activeView === "planning") {
      return `
        <div class="os-preview-banner" role="status" data-testid="planning-preview-banner">
          <strong>Under construction</strong>
          <span>This planning workspace shows fixture data for preview purposes — it is not wired to a live planning session yet.</span>
        </div>
        ${renderPlanningWorkspace(this.state.planningWorkspace, this.state.planningEdit)}
      `;
    }
    return `
      ${this.renderDashboardWorkspace()}
    `;
  }

  private renderGlobalModals(): string {
    return `
      ${this.renderProfiles()}
      ${this.renderModelProfiles()}
      ${this.renderEventLogModal()}
    `;
  }

  private renderTopbarStatusStrip(): string {
    const snapshot = this.state.snapshot;
    const totalTokens = snapshot
      ? snapshot.metrics.total_input_tokens
        + snapshot.metrics.total_cache_read_tokens
        + snapshot.metrics.total_output_tokens
      : 0;
    const metrics = snapshot
      ? `
        <span><strong>${snapshot.metrics.running_issue_count}</strong> running</span>
        <span><strong>${snapshot.metrics.retry_queue_depth}</strong> retry</span>
        <span><strong>${formatNumber(totalTokens)}</strong> tokens</span>
      `
      : `<span>status loading</span>`;
    const events = statusEvents(snapshot)
      .slice(0, 2)
      .map((event) => `
        <li>
          <time datetime="${escapeAttr(event.happened_at)}">${escapeHtml(formatEventTime(event.happened_at))}</time>
          <span>${escapeHtml(event.issue_identifier ?? "system")}</span>
          ${escapeHtml(event.summary)}
        </li>
      `)
      .join("");
    const profiles = this.state.profiles.length > 0
      ? this.state.profiles
      : defaultUiProfiles(this.transport.baseUri);
    const activeProfile = profiles.find((profile) => profile.id === this.state.activeProfileId)
      ?? profiles[0]
      ?? null;
    const modelProfiles = modelProfilesWithDefaults(this.state.modelProfiles);
    const activeModel = activeModelProfile(modelProfiles, this.state.activeModelProfileId)
      ?? modelProfiles[0]
      ?? null;
    const persistence = this.options.modelProfileController?.persistence;
    const persistenceMeta = persistence
      ? `<span class="os-model-persistence os-model-persistence-${escapeAttr(persistence.kind)}" data-testid="model-persistence-status">${escapeHtml(persistence.label)}</span>`
      : "";
    const modelProfileError = this.state.modelProfileError
      ? `<span class="os-model-error os-strip-alert" role="alert" data-testid="model-profile-error">${escapeHtml(this.state.modelProfileError)}</span>`
      : "";
    return `
      <div class="os-status-strip" data-testid="status-strip">
        <div class="os-status os-status-${this.state.connectionMode}">
          <span aria-hidden="true"></span>${escapeHtml(statusLabel(this.state.connectionMode))}
        </div>
        <div class="os-strip-metrics">${metrics}</div>
        <div class="os-strip-connection">
          <span>${escapeHtml(activeProfile?.label ?? "Connection")}</span>
          <button type="button" class="os-icon-button os-glyph-button" data-toggle-settings="connection" aria-expanded="${this.state.profilePanelExpanded ? "true" : "false"}" aria-label="Connection settings" title="Connection settings">${connectionIconSvg()}</button>
        </div>
        <div class="os-event-mini" data-testid="event-log-mini">
          <ol>${events || `<li>No recent events</li>`}</ol>
          <button type="button" class="os-icon-button" data-open-event-log aria-haspopup="dialog">Log</button>
        </div>
        <div class="os-strip-model">
          <span>${escapeHtml(activeModel?.model || activeModel?.label || "Model")}</span>
          ${persistenceMeta}
          ${modelProfileError}
          <button type="button" class="os-icon-button os-glyph-button os-model-gear" data-toggle-settings="model" aria-expanded="${this.state.modelPanelExpanded ? "true" : "false"}" aria-label="Model Configuration settings" title="Model Configuration settings">${gearIconSvg()}</button>
        </div>
      </div>
    `;
  }

  private renderDashboardWorkspace(): string {
    const sizes = this.currentWorkspacePaneSizes();
    return `
      <section class="os-workspace-shell" data-testid="workspace-pane-shell" data-graph-surface="${escapeAttr(this.state.graphPaneView)}">
        ${this.renderGraphPane()}
        ${renderLowerRowResizer(this.state.lowerRowHeight)}
        <section class="os-lower-columns" data-testid="workspace-lower-columns" style="--os-left-column: ${panePercent(sizes.left)}; --os-right-column: ${panePercent(sizes.right)}; --os-lower-row-height: ${this.state.lowerRowHeight}px;">
          ${this.renderLowerColumn("left")}
          ${renderPaneResizer("lower-columns", "Resize lower workspace columns", sizes.left)}
          ${this.renderLowerColumn("right")}
        </section>
      </section>
    `;
  }

  private currentWorkspacePaneSizes(): WorkspacePaneSizes {
    return this.state.workspacePaneSizes[this.state.graphPaneView] ?? defaultWorkspacePaneSizes;
  }

  private renderEventLogModal(): string {
    if (!this.state.eventLogModalOpen) {
      return "";
    }
    const events = statusEvents(this.state.snapshot);
    const items = events.map((event) => `
      <li>
        <time datetime="${escapeAttr(event.happened_at)}">${escapeHtml(formatEventTime(event.happened_at))}</time>
        <span>${escapeHtml(event.kind)}</span>
        <strong>${escapeHtml(event.issue_identifier ?? "system")}</strong>
        ${escapeHtml(event.summary)}
      </li>
    `).join("");
    return `
      <div class="os-modal-backdrop" data-event-log-modal>
        <section class="os-dialog os-event-log-modal" role="dialog" aria-modal="true" aria-labelledby="os-event-log-title">
          <div class="os-section-head">
            <div>
              <h2 id="os-event-log-title">Event Log</h2>
              <span>${events.length} recent event${events.length === 1 ? "" : "s"}</span>
            </div>
            <button type="button" class="os-activity-toggle os-panel-toggle" data-close-event-log aria-label="Close Event Log" title="Close Event Log">
              <span aria-hidden="true">x</span>
            </button>
          </div>
          <ol class="os-events os-events-full" data-testid="event-log-modal">${items || `<li>No recent events</li>`}</ol>
        </section>
      </div>
    `;
  }

  /**
   * Render the auth-aware placeholder shell.
   *
   * Hosted auth integration arrives in a follow-on task; these placeholders
   * keep the user-facing states stable so the real provider can slot in with
   * minimal UI churn. Local unauthenticated gateways (`auth_modes:["none"]`)
   * never reach this path because their reads succeed and `authState` stays
   * `"open"`.
   */
  private renderAuthPlaceholder(): string {
    const state = this.state.authState;
    // Org/project selection is only meaningful when the caller can still act
    // on it (sign in / switch workspace). A hard 403 `forbidden` deny means
    // the gateway refused the workspace outright, so tenant selectors would
    // be misleading there.
    const orgProject = state === "forbidden" ? "" : this.renderOrgProjectPlaceholder();
    if (state === "unauthenticated") {
      return `
        <section class="os-panel os-auth-panel" data-testid="auth-placeholder" data-auth-state="unauthenticated">
          <div class="os-section-head"><h2>Sign in</h2><span>hosted</span></div>
          <div class="os-auth-body">
            <p class="os-auth-message" data-testid="auth-message">Sign in required to view this OpenSymphony workspace.</p>
            <div class="os-auth-actions">
              <button type="button" data-auth-action="sign-in" data-testid="auth-sign-in">Sign in</button>
              <button type="button" data-auth-action="refresh" data-testid="auth-refresh">Retry</button>
            </div>
            <p class="os-auth-note" data-testid="auth-note">Hosted authentication is configured by your administrator. Local development gateways do not require sign-in.</p>
          </div>
          ${orgProject}
        </section>
      `;
    }
    if (state === "unauthorized") {
      return `
        <section class="os-panel os-auth-panel os-auth-denied" data-testid="auth-placeholder" data-auth-state="unauthorized">
          <div class="os-section-head"><h2>Access denied</h2><span>hosted</span></div>
          <div class="os-auth-body">
            <p class="os-auth-message" data-testid="auth-message">You are signed in but do not have permission to view this workspace.</p>
            <div class="os-auth-actions">
              <button type="button" data-auth-action="refresh" data-testid="auth-refresh">Retry</button>
            </div>
            <p class="os-auth-note" data-testid="auth-note">Request access from your organization administrator, or switch to a workspace you can access.</p>
          </div>
          ${orgProject}
        </section>
      `;
    }
    // forbidden
    return `
      <section class="os-panel os-auth-panel os-auth-denied" data-testid="auth-placeholder" data-auth-state="forbidden">
        <div class="os-section-head"><h2>Access forbidden</h2><span>hosted</span></div>
        <div class="os-auth-body">
          <p class="os-auth-message" data-testid="auth-message">Access to this workspace is forbidden.</p>
          <div class="os-auth-actions">
            <button type="button" data-auth-action="refresh" data-testid="auth-refresh">Retry</button>
          </div>
          <p class="os-auth-note" data-testid="auth-note">The gateway refused the request. If this is unexpected, contact your administrator.</p>
        </div>
        ${orgProject}
      </section>
    `;
  }

  /**
   * Organization/project selection placeholder for hosted contexts.
   *
   * Real tenant/org selection arrives with hosted auth; this surface keeps the
   * selector present so the data model and UI layout are stable. Rendered only
   * for `unauthenticated` and `unauthorized` auth states (see
   * `renderAuthPlaceholder`), where the caller can still act on a workspace
   * choice. It is intentionally omitted for `forbidden`, where the gateway has
   * hard-denied the workspace.
   */
  private renderOrgProjectPlaceholder(): string {
    return `
      <div class="os-auth-scope" data-testid="auth-scope">
        <div class="os-section-head"><h3>Workspace</h3></div>
        <div class="os-inline-fields">
          <label class="os-field">
            <span>Organization</span>
            <select data-auth-org data-testid="auth-org" disabled>
              <option value="">Select organization</option>
            </select>
          </label>
          <label class="os-field">
            <span>Project</span>
            <select data-auth-project data-testid="auth-project" disabled>
              <option value="">Select project</option>
            </select>
          </label>
        </div>
        <p class="os-auth-note">Organization and project selection is available after you sign in.</p>
      </div>
    `;
  }

  private renderProfiles(): string {
    if (!this.state.profilePanelExpanded) {
      return "";
    }
    const profiles = this.state.profiles.length > 0
      ? this.state.profiles
      : defaultUiProfiles(this.transport.baseUri);
    const options = profiles
      .map((profile) => {
        const selected = profile.id === this.state.activeProfileId ? "selected" : "";
        return `<option value="${escapeAttr(profile.id)}" ${selected}>${escapeHtml(profile.label)}</option>`;
      })
      .join("");
    const activeProfile = profiles.find((profile) => profile.id === this.state.activeProfileId)
      ?? profiles[0];
    const selectedKind = activeProfile?.kind ?? defaultProfileKindForMode(this.options.mode);
    const kindOptions = editableProfileKindOptions
      .map((option) => {
        const selected = option.value === selectedKind ? "selected" : "";
        return `<option value="${option.value}" ${selected}>${option.label}</option>`;
      })
      .join("");
    const capabilities = this.state.capabilities?.transports
      .map((transport) => transport.transport)
      .join(", ") ?? "unknown";
    const canRemoveProfile = profiles.length > 1;
    return `
      <div class="os-modal-backdrop" data-settings-modal="connection">
        <section class="os-dialog os-profile-panel" role="dialog" aria-modal="true" aria-labelledby="os-connection-settings-title">
          <div class="os-section-head">
            <div>
              <h2 id="os-connection-settings-title">Connection</h2>
              <span>${escapeHtml(activeProfile ? `${activeProfile.label} | ${activeProfile.gatewayUrl}` : this.transport.baseUri)}</span>
            </div>
            <button type="button" class="os-activity-toggle os-panel-toggle" data-toggle-settings="connection" aria-expanded="true" aria-label="Close Connection settings" title="Close Connection settings">
              <span aria-hidden="true">x</span>
            </button>
          </div>
          <label class="os-field">
            <span>Profile</span>
            <select data-profile-select>${options}</select>
          </label>
          <div class="os-inline-fields">
            <label class="os-field">
              <span>Label</span>
              <input data-profile-label value="${escapeAttr(activeProfile?.label ?? "Local Gateway")}" />
            </label>
            <label class="os-field">
              <span>Kind</span>
              <select data-profile-kind>${kindOptions}</select>
            </label>
            <label class="os-field">
              <span>Gateway URL</span>
              <input data-profile-gateway value="${escapeAttr(this.state.gatewayDraft)}" />
            </label>
            <div class="os-model-actions">
              <button type="button" data-save-profile ${this.options.profileController ? "" : "disabled"}>Save</button>
              <button type="button" data-new-profile ${this.options.profileController ? "" : "disabled"}>New</button>
              <button type="button" data-remove-profile ${this.options.profileController && canRemoveProfile ? "" : "disabled"}>Delete</button>
            </div>
          </div>
          <div class="os-meta">Transport: ${escapeHtml(capabilities)}</div>
        </section>
      </div>
    `;
  }

  private renderModelProfiles(): string {
    const profiles = modelProfilesWithDefaults(this.state.modelProfiles);
    const active = activeModelProfile(profiles, this.state.activeModelProfileId)
      ?? profiles[0]
      ?? null;
    if (!this.state.modelPanelExpanded) {
      return "";
    }
    if (!active) {
      return `
        <div class="os-modal-backdrop" data-settings-modal="model">
        <section class="os-dialog os-model-panel" role="dialog" aria-modal="true" aria-labelledby="os-model-settings-title" data-testid="model-profile-panel">
          <div class="os-section-head">
            <div>
              <h2 id="os-model-settings-title">Model Configuration</h2>
              <span>No model profiles</span>
            </div>
            <button type="button" class="os-activity-toggle os-panel-toggle" data-toggle-settings="model" aria-expanded="true" aria-label="Close Model Configuration settings" title="Close Model Configuration settings">
              <span aria-hidden="true">x</span>
            </button>
          </div>
        </section>
        </div>
      `;
    }
    const options = profiles
      .map((profile) => {
        const selected = profile.id === active.id ? "selected" : "";
        return `<option value="${escapeAttr(profile.id)}" ${selected}>${escapeHtml(profile.label)}</option>`;
      })
      .join("");
    const credentialRef = active.mode === "subscription"
      ? active.subscriptionCredential?.authDirectoryEnv
      : active.apiKeyRef;
    const credentialSummary = modelCredentialSummary(active);
    const credentialLabel = modelCredentialLabel(active);
    const credentialInputType = active.mode === "subscription" ? "text" : "password";
    const modelProfileError = this.state.modelProfileError
      ? `<div class="os-model-error" role="alert" data-testid="model-profile-error">${escapeHtml(this.state.modelProfileError)}</div>`
      : "";
    const persistence = this.options.modelProfileController?.persistence;
    const persistenceMeta = persistence
      ? `<div class="os-model-persistence os-model-persistence-${escapeAttr(persistence.kind)}" data-testid="model-persistence-status">${escapeHtml(persistence.label)}</div>`
      : "";
    const canRemoveProfile = profiles.length > 1;
    const summary = `${active.label} • ${active.model || "No model"}${active.baseUrl ? ` • ${active.baseUrl}` : ""}`;
    const header = `
      <div class="os-section-head">
        <div>
          <h2 id="os-model-settings-title">Model Configuration</h2>
          <span>${escapeHtml(summary)}</span>
        </div>
        <button type="button" class="os-activity-toggle os-panel-toggle" data-toggle-settings="model" aria-expanded="true" aria-label="Close Model Configuration settings" title="Close Model Configuration settings">
          <span aria-hidden="true">x</span>
        </button>
      </div>
    `;
    return `
      <div class="os-modal-backdrop" data-settings-modal="model">
      <section class="os-dialog os-model-panel" role="dialog" aria-modal="true" aria-labelledby="os-model-settings-title" data-testid="model-profile-panel">
        ${header}
        <div class="os-model-layout">
          <label class="os-field">
            <span>Profile</span>
            <select data-model-profile-select>${options}</select>
          </label>
          <label class="os-field">
            <span>Label</span>
            <input data-model-label value="${escapeAttr(active.label)}" />
          </label>
          <label class="os-field">
            <span>Mode</span>
            <select data-model-mode>
              ${option("api_key", "API-compatible key", active.mode)}
              ${option("subscription", "Subscription", active.mode)}
            </select>
          </label>
          <label class="os-field">
            <span>Base URL</span>
            <input data-model-base-url value="${escapeAttr(active.baseUrl)}" placeholder="Provider default or API-compatible URL" />
          </label>
          <label class="os-field">
            <span>Model ID</span>
            <input data-model-name value="${escapeAttr(active.model)}" />
          </label>
          <label class="os-field">
            <span>${credentialLabel}</span>
            <input data-model-credential-ref type="${credentialInputType}" autocomplete="off" value="${escapeAttr(credentialRef ?? "")}" />
          </label>
          <label class="os-check-field">
            <input data-model-active type="checkbox" ${active.active ? "checked" : ""} />
            <span>Active</span>
          </label>
          <details class="os-advanced-settings">
            <summary>Advanced</summary>
            <div class="os-advanced-grid">
              <label class="os-field">
                <span>Scope</span>
                <select data-model-owner>
                  ${option("user", "User", active.owner)}
                  ${option("organization", "Organization", active.owner)}
                  ${option("project", "Project", active.owner)}
                </select>
              </label>
              <label class="os-field">
                <span>Usable Harnesses</span>
                <input data-model-harnesses value="${escapeAttr(active.harnesses.join(", "))}" />
              </label>
            </div>
          </details>
          <div class="os-model-actions">
            <button type="button" data-save-model-profile ${this.options.modelProfileController ? "" : "disabled"}>Save</button>
            <button type="button" data-new-model-profile ${this.options.modelProfileController ? "" : "disabled"}>New</button>
            <button type="button" data-remove-model-profile ${this.options.modelProfileController && canRemoveProfile ? "" : "disabled"}>Delete</button>
          </div>
        </div>
        <div class="os-model-meta" data-testid="model-redacted-credential">
          Credential: ${escapeHtml(credentialSummary)}
        </div>
        ${persistenceMeta}
        ${modelProfileError}
      </section>
      </div>
    `;
  }

  private renderGraphPane(): string {
    const content = this.state.graphPaneView === "task"
      ? this.renderTaskGraph()
      : this.state.graphPaneView === "knowledge"
      ? renderKnowledgeGraphSurface({
          snapshot: visibleGraphSnapshot(this.state.knowledgeGraph),
          layout: this.state.knowledgeGraphLayout,
          state: this.state.knowledgeGraph,
        })
        : renderCodeGraphSurface({
          snapshot: this.visibleCodeGraphSnapshot(),
          layout: this.codeGraphLayout,
          state: this.state.codeGraph,
          symbolDetail: this.selectedCodeSymbolDetail(),
          rawRecord: this.codeGraphRawRecord,
        });
    return `
      <section class="os-panel os-graph-hero-panel" data-testid="graph-hero" data-active-graph-surface="${escapeAttr(this.state.graphPaneView)}">
        <div class="os-graph-hero-toolbar">
          <div>
            <h2>Graph Surface</h2>
            <span>${escapeHtml(graphSurfaceSummary(this.state.graphPaneView))}</span>
          </div>
          <div class="os-segmented" data-testid="graph-view-toggle">
            <button type="button" class="${this.state.graphPaneView === "task" ? "is-selected" : ""}" data-graph-view="task">Task Graph</button>
            <button type="button" class="${this.state.graphPaneView === "knowledge" ? "is-selected" : ""}" data-graph-view="knowledge">Knowledge Graph</button>
            <button type="button" class="${this.state.graphPaneView === "code" ? "is-selected" : ""}" data-graph-view="code">Code Graph</button>
          </div>
        </div>
        <div class="os-graph-hero-body">
          ${content}
        </div>
      </section>
    `;
  }

  private renderTaskGraph(): string {
    const taskGraph = this.state.taskGraph;
    if (!taskGraph) {
      return `<div class="os-empty">No task graph loaded</div>`;
    }
    const allDependencySignals = buildDependencySignals(taskGraph.nodes, taskGraph.nodes);
    const getOverlay = (node: TaskGraphNode) => {
      const run = node.run_id ? this.state.runOverlays.get(node.run_id) : undefined;
      return applyGraphRuntimeOverlay(
        node,
        allDependencySignals.get(node.node_id),
        buildRuntimeOverlay(node, run),
      );
    };
    const filtered = filterTaskGraphNodes(taskGraph.nodes, this.state.taskGraphFilter);
    if (this.options.mode === "web") {
      return this.renderEditableTaskGraph(taskGraph, filtered, getOverlay);
    }
    const filters = renderTaskGraphFilters(this.state.taskGraphFilter);
    return `${filters}${this.renderTaskGraphPanes(taskGraph, filtered, getOverlay)}`;
  }

  /**
   * Desktop task surface: three panes — Completed (memory-backed table),
   * Current (dispatchable dependency graph), Backlog (same grammar, faded
   * edges). Cross-pane dependency edges render in an absolutely positioned
   * overlay whose paths are measured after mount (positionTaskGraphCrossLinks).
   */
  private renderTaskGraphPanes(
    taskGraph: TaskGraphSnapshot,
    filtered: TaskGraphNode[],
    getOverlay: (node: TaskGraphNode) => ReturnType<typeof buildRuntimeOverlay>,
  ): string {
    const currentNodes = filtered.filter(isCurrentPaneTaskNode);
    const backlogNodes = filtered.filter((node) => node.state_category === "backlog");
    // Dependency suffixes treat both graph panes as visible: a backlog task
    // blocked by a Current task reads "blocked by VIZ-103", not "1 hidden".
    // In-pane edge building still only links nodes present in that pane;
    // cross-pane pairs render through the measured overlay instead.
    const visibleAcrossPanes = [...currentNodes, ...backlogNodes];
    const currentSignals = buildDependencySignals(taskGraph.nodes, visibleAcrossPanes);
    const backlogSignals = currentSignals;
    const collapsed = this.state.taskPaneCollapsed;

    const currentGraph = currentNodes.length > 0
      ? renderTaskGraphVisualization(
        currentNodes,
        this.state.selectedNodeId,
        getOverlay,
        currentSignals,
        this.state.collapsedProjectGroups,
        true,
      )
      : `<div class="os-empty">No dispatchable tasks match the current filters</div>`;
    const backlogGraph = backlogNodes.length > 0
      ? renderTaskGraphVisualization(
        backlogNodes,
        this.state.selectedNodeId,
        getOverlay,
        backlogSignals,
        new Set(),
        false,
        "backlog",
      )
      : `<div class="os-empty">No backlog tasks</div>`;

    const sizes = this.state.taskPaneSizes;
    const donePane = collapsed.done
      ? renderCollapsedTaskPane("done", "Completed", this.state.completedTasks?.total ?? null)
      : `
        <section class="os-tg-pane os-tg-pane-done" data-tg-pane="done" data-testid="task-pane-done" style="flex: 0 0 ${sizes.done}px;">
          ${renderTaskPaneHeader("done", "Completed", this.state.completedTasks?.total ?? null)}
          <div class="os-tg-pane-body" data-tg-pane-body="done">
            ${this.renderCompletedTasksBody()}
          </div>
        </section>
      `;
    const backlogPane = collapsed.backlog
      ? renderCollapsedTaskPane("backlog", "Backlog", backlogNodes.length)
      : `
        <section class="os-tg-pane os-tg-pane-backlog" data-tg-pane="backlog" data-testid="task-pane-backlog" style="flex: 0 0 ${sizes.backlog}px;">
          ${renderTaskPaneHeader("backlog", "Backlog", backlogNodes.length)}
          <div class="os-tg-pane-body" data-tg-pane-body="backlog">
            ${backlogGraph}
          </div>
        </section>
      `;

    const crossLinks = renderTaskGraphCrossLinks(taskGraph.nodes, currentNodes, backlogNodes);

    return `
      <div class="os-tg-panes" data-tg-panes>
        ${donePane}
        ${collapsed.done ? "" : renderTaskPaneResizer("done", "Resize Completed pane", sizes.done)}
        <section class="os-tg-pane os-tg-pane-current" data-tg-pane="current" data-testid="task-pane-current">
          <header class="os-tg-pane-head">
            <span class="os-tg-dot os-tg-dot-current" aria-hidden="true"></span>
            <strong>Current</strong>
            <span class="os-tg-count os-tg-count-current">${currentNodes.filter((node) => node.kind !== "milestone").length} current</span>
          </header>
          <div class="os-tg-pane-body" data-tg-pane-body="current">
            ${currentGraph}
          </div>
        </section>
        ${collapsed.backlog ? "" : renderTaskPaneResizer("backlog", "Resize Backlog pane", sizes.backlog)}
        ${backlogPane}
        ${crossLinks}
      </div>
    `;
  }

  /** Search box, sortable table, and pagination for the Completed pane. */
  private renderCompletedTasksBody(): string {
    if (!this.graphAdapter?.getCompletedTasks) {
      return `<div class="os-empty" data-testid="completed-tasks-unavailable">Completed tasks need a memory-server connection</div>`;
    }
    const page = this.state.completedTasks;
    const error = this.state.completedTasksError
      ? `<p class="os-tg-done-error" data-testid="completed-tasks-error" role="alert">Completed tasks unavailable: ${escapeHtml(this.state.completedTasksError)}</p>`
      : "";
    const params = this.state.completedTasksParams;
    const search = `
      <input
        type="search"
        class="os-tg-done-search"
        data-tg-done-search
        placeholder="Search completed tasks"
        aria-label="Search completed tasks"
        value="${escapeAttr(params.query)}"
      />
    `;
    if (!page) {
      return `${search}${error || `<div class="os-empty" data-testid="completed-tasks-loading">Loading completed tasks…</div>`}`;
    }
    const rows = page.tasks.map((task) => this.renderCompletedTaskRow(task)).join("");
    const table = page.tasks.length > 0
      ? `
        <table class="os-tg-done-table" data-testid="completed-tasks-table">
          <thead>
            <tr>
              ${renderCompletedSortHeader("id", "ID", params.sort)}
              ${renderCompletedSortHeader("title", "Title", params.sort)}
              ${renderCompletedSortHeader("pr", "PR", params.sort)}
              ${renderCompletedSortHeader("completed", "Done", params.sort)}
              <th scope="col">Capsule</th>
            </tr>
          </thead>
          <tbody>${rows}</tbody>
        </table>
      `
      : `<div class="os-empty">No completed tasks${params.query ? " match the search" : ""}</div>`;
    return `${search}${error}${table}${renderCompletedTasksPagination(page)}`;
  }

  private renderCompletedTaskRow(task: MemoryCompletedTask): string {
    const completed = task.completed_at ? formatCompletedDate(task.completed_at) : "—";
    const capsule = task.concept_id
      ? `<button
          type="button"
          class="os-tg-capsule-button"
          data-tg-capsule="${escapeAttr(formatMemoryDeepLink({ bundleId: task.bundle_id ?? "local-default", conceptId: task.concept_id }))}"
          title="Open memory capsule ${escapeAttr(task.concept_id)}"
          aria-label="Open memory capsule for ${escapeAttr(task.issue_key)}"
        >◈</button>`
      : `<span class="os-tg-capsule-missing" title="No memory capsule captured yet (${escapeAttr(task.source)})">—</span>`;
    const title = task.url
      ? `<a href="${escapeAttr(task.url)}" target="_blank" rel="noreferrer noopener" title="${escapeAttr(task.title)}">${escapeHtml(task.title)}</a>`
      : `<span title="${escapeAttr(task.title)}">${escapeHtml(task.title)}</span>`;
    return `
      <tr data-testid="completed-task-row" data-task-key="${escapeAttr(task.issue_key)}">
        <td class="os-tg-done-id">${escapeHtml(task.issue_key)}</td>
        <td class="os-tg-done-title">${title}</td>
        <td class="os-tg-done-prs">${renderCompletedTaskPrs(task.prs)}</td>
        <td class="os-tg-done-date">${escapeHtml(completed)}</td>
        <td class="os-tg-done-capsule">${capsule}</td>
      </tr>
    `;
  }

  private renderEditableTaskGraph(
    taskGraph: TaskGraphSnapshot,
    filtered: TaskGraphNode[],
    getOverlay: (node: TaskGraphNode) => ReturnType<typeof buildRuntimeOverlay>,
  ): string {
    const allNodes = new Map(taskGraph.nodes.map((node) => [node.node_id, node]));
    const nodes = filtered.map((node) => renderTaskGraphNode(
      node,
      this.state.selectedNodeId,
      this.state.inlineEdit,
      getOverlay(node),
    )).join("");
    const toolbar = renderTaskGraphToolbar();
    const filters = renderTaskGraphFilters(this.state.taskGraphFilter);
    const pendingBanner = this.state.pendingMutations.size > 0
      ? `<div class="os-pending-banner">${this.state.pendingMutations.size} change(s) pending server acknowledgement</div>`
      : "";
    const dependencyDialog = this.state.dependencyEdit.nodeId && allNodes.get(this.state.dependencyEdit.nodeId)
      ? renderDependencyEditor(allNodes.get(this.state.dependencyEdit.nodeId)!, allNodes, this.state.dependencyEdit)
      : "";
    const commentDialog = this.state.commentEdit.nodeId && allNodes.get(this.state.commentEdit.nodeId)
      ? renderCommentEditor(allNodes.get(this.state.commentEdit.nodeId)!, this.state.commentEdit)
      : "";
    const createDialog = renderCreateDialog(this.state.createDialog);
    const actions = (() => {
      if (!this.state.createDialog.open && !this.state.dependencyEdit.nodeId && !this.state.commentEdit.nodeId) return "";
      return `
        <div class="os-dialog-actions-bar">
          <span data-tg-active-action="true">editing ${
            this.state.createDialog.open ? "create" : this.state.dependencyEdit.nodeId ? "dependencies" : "comment"
          }</span>
        </div>
      `;
    })();

    return panel(
      "Task Graph",
      `${toolbar}${filters}${pendingBanner}<div class="os-node-list">${nodes || `<div class="os-empty">No tasks match the current filters</div>`}</div>${actions}${createDialog}${dependencyDialog}${commentDialog}`,
      "os-task-graph-panel",
    );
  }

  private renderLowerColumn(column: "left" | "right"): string {
    switch (this.state.graphPaneView) {
      case "task":
        return column === "left" ? this.renderRunDetail() : this.renderRunEvidence();
      case "knowledge":
        return column === "left" ? this.renderKnowledgeGraphListColumn() : this.renderKnowledgeGraphDetailColumn();
      case "code":
        return column === "left" ? this.renderCodeGraphStructureColumn() : this.renderCodeGraphDetailColumn();
    }
  }

  /**
   * Narrow lower-left column: the clickable entity list for the visible
   * graph (drilled views list only the drilled area's members). Lives beside
   * the inspector so the graph stage, the entities, and the selected
   * capsule all share the fold.
   */
  private renderKnowledgeGraphListColumn(): string {
    const snapshot = visibleGraphSnapshot(this.state.knowledgeGraph);
    return panel(
      "Entities",
      renderKnowledgeGraphNodeList(snapshot, this.state.knowledgeGraph.selectedNodeIds),
      "os-knowledge-lower-panel",
    );
  }

  /** Lower-right column: the selected node's inspector card and memory capsule. */
  private renderKnowledgeGraphDetailColumn(): string {
    return panel(
      "Inspector",
      renderKnowledgeGraphInspector({
        snapshot: visibleGraphSnapshot(this.state.knowledgeGraph),
        layout: this.state.knowledgeGraphLayout,
        state: this.state.knowledgeGraph,
        ...this.selectedKnowledgeCapsule(),
      }),
      "os-knowledge-lower-panel",
    );
  }

  private renderCodeGraphStructureColumn(): string {
    return panel(
      "Structure List",
      renderCodeGraphNodeList(this.visibleCodeGraphSnapshot(), this.state.codeGraph.selectedNodeIds, this.state.codeGraph.diffOverlay),
      "os-code-graph-lower-panel",
    );
  }

  private renderCodeGraphDetailColumn(): string {
    return panel(
      "Symbol Detail",
      renderCodeGraphInspector({
        snapshot: this.visibleCodeGraphSnapshot(),
        layout: this.codeGraphLayout,
        state: this.state.codeGraph,
        symbolDetail: this.selectedCodeSymbolDetail(),
        rawRecord: this.codeGraphRawRecord,
      }),
      "os-code-graph-lower-panel",
    );
  }

  private renderRunDetail(): string {
    const run = this.state.runDetail;
    if (!run) {
      return panel("Run Detail", `<div class="os-empty">Select an issue and open its run</div>`, "os-run-detail-panel");
    }
    const phase = run.liveness?.phase ?? statusToPhase(run.status, run.release_reason, run.detached);
    const stream = run.liveness?.stream ?? "healthy";
    const cancelState = run.cancel_failed
      ? "cancel-failed"
      : run.cancel_acknowledged
        ? "cancel-acknowledged"
        : undefined;
    const actionItems = buildActionBarItems(run);
    const actionBar = renderActionBar(actionItems);
    const files = renderChangedFileList(this.state.runFiles ?? [], this.state.selectedDiffPath ?? undefined);
    const selectedNode = this.selectedTaskNode();
    const dependencyDetail = selectedNode
      ? renderDependencyDetail(selectedNode, this.state.taskGraph?.nodes ?? [])
      : "";
    const validation = hasValidationSummary(this.state.runValidation)
      ? renderValidationSummary(this.state.runValidation)
      : "";
    const approvals = this.state.runApprovals?.length
      ? renderApprovalList(this.state.runApprovals, {
          onDecide: (id, decision, explanation) => {
            void this.submitApprovalDecision(id, decision, explanation);
          },
        })
      : "";
    const runtime = run.runtime_seconds > 0 || (run.started_at && run.status === "running")
      ? `${run.runtime_seconds}s`
      : "unknown";
    const detailRows = [
      `<div><span>Phase</span><strong>${escapeHtml(phase)}</strong></div>`,
      `<div><span>Stream</span><strong>${escapeHtml(stream)}</strong></div>`,
      `<div><span>Turns</span><strong>${formatNumber(run.turn_count)}</strong></div>`,
      `<div><span>Runtime</span><strong>${runtime}</strong></div>`,
      `<div><span>Input</span><strong>${formatNumber(run.input_tokens)}</strong></div>`,
      `<div><span>Cache</span><strong>${formatNumber(run.cache_read_tokens)}</strong></div>`,
      `<div><span>Output</span><strong>${formatNumber(run.output_tokens)}</strong></div>`,
      `<div><span>Total</span><strong>${formatNumber(run.input_tokens + run.cache_read_tokens + run.output_tokens)}</strong></div>`,
      run.diagnostics?.cancel_acknowledged ? `<div><span>Cancel</span><strong class="os-cancel-acknowledged" data-testid="cancel-acknowledged">acknowledged</strong></div>` : "",
      run.diagnostics?.cancel_failed ? `<div><span>Cancel</span><strong class="os-cancel-failed" data-testid="cancel-failed">failed</strong></div>` : "",
    ].filter(Boolean).join("");
    const runMeta = [
      run.branch_name ? `<div class="os-run-meta-row" data-testid="run-branch"><span>Branch</span><code>${escapeHtml(run.branch_name)}</code></div>` : "",
      (run.branch_name || run.pr_url) ? `<div class="os-run-meta-row" data-testid="run-pr"><span>Pull Request</span>${run.pr_url ? `<a href="${escapeAttr(run.pr_url)}" target="_blank" rel="noopener noreferrer">${escapeHtml(formatPrLinkLabel(run.pr_url))}</a>` : `<em>Not found</em>`}</div>` : "",
    ].filter(Boolean).join("");
    const receipt = this.state.lastActionReceipt
      ? renderActionReceipt(this.state.lastActionReceipt)
      : "";
    const audit = this.state.auditTrail.length
      ? `<div class="os-audit-trail" data-testid="audit-trail">${this.state.auditTrail.map(renderAuditTrailEntry).join("")}</div>`
      : "";
    return panel(
      "Run Detail",
      `
        <div class="os-run-head">
          <div>
            <strong>${escapeHtml(run.issue_identifier)}</strong>
            <span>${escapeHtml(run.run_id)}</span>
          </div>
          <div class="os-run-pills">
            <div class="os-pill">${escapeHtml(run.status)}</div>
            ${run.detached ? `<div class="os-pill os-pill-detached" data-testid="run-pill-detached">detached</div>` : ""}
            ${cancelState ? `<div class="os-pill os-pill-${cancelState}" data-testid="run-pill-cancel-state">${cancelState}</div>` : ""}
          </div>
        </div>
        ${dependencyDetail}
        <div class="os-run-grid">
          ${detailRows}
        </div>
        ${runMeta ? `<div class="os-run-meta-list">${runMeta}</div>` : ""}
        ${actionBar}
        ${receipt}
        <div class="os-run-section">
          <h3>Changed Files</h3>
          ${files}
        </div>
        ${validation || approvals ? `<div class="os-run-panels">${validation ? `<div class="os-validation-panel">${validation}</div>` : ""}${approvals ? `<div class="os-approval-panel">${approvals}</div>` : ""}</div>` : ""}
        ${audit}
        <pre>${escapeHtml(run.workspace_path ?? run.workspace_id ?? "workspace path unavailable")}</pre>
      `,
      "os-run-detail-panel",
    );
  }

  private selectedTaskNode(): TaskGraphNode | null {
    const selectedNodeId = this.state.selectedNodeId;
    const nodes = this.state.taskGraph?.nodes ?? [];
    if (selectedNodeId) {
      const selected = nodes.find((node) => node.node_id === selectedNodeId);
      if (selected) return selected;
    }
    const runIssue = this.state.runDetail?.issue_identifier ?? this.state.runDetail?.run_id ?? null;
    return runIssue ? (findNodeByRef(nodes, runIssue) ?? null) : null;
  }

  private renderRunEvidence(): string {
    const run = this.state.runDetail;
    if (!run) {
      return panel("Inspector", `<div class="os-empty">Select an issue to inspect a diff or activity</div>`, "os-run-evidence-panel");
    }
    const diff = this.state.runDiff ? renderFileDiff(this.state.runDiff) : "";
    const activity = renderRunActivity(
      this.state.runEvents,
      this.state.expandedActivityEvents,
      this.state.collapsedActivityEvents,
    );
    const showingDiff = this.state.evidenceView === "diff";
    const showingActivity = this.state.evidenceView === "activity";
    const content = showingDiff
      ? diff || `<div class="os-empty">Select a changed file to view its diff</div>`
      : activity;
    return panel(
      "Inspector",
      `
        <div class="os-segmented" data-testid="evidence-toggle">
          <button type="button" class="${showingDiff ? "is-selected" : ""}" data-evidence-view="diff">Diff</button>
          <button type="button" class="${showingActivity ? "is-selected" : ""}" data-evidence-view="activity">Activity</button>
        </div>
        <div class="os-run-section">
          <h3>${showingDiff ? "Selected Diff" : "Conversation Activity"}</h3>
          ${content}
        </div>
      `,
      "os-run-evidence-panel",
    );
  }

  /**
   * Attach a listener exactly once per (element, site, event type).
   *
   * render() morphs the DOM in place, so elements — and their listeners —
   * survive re-renders. bindEvents() still runs after every render to cover
   * newly created elements; this guard keeps it from stacking duplicate
   * listeners on the survivors. The site string identifies the bindEvents
   * call site so distinct handlers for the same element/event coexist.
   */
  private listen(
    element: Element | null | undefined,
    site: string,
    type: string,
    handler: (event: Event) => void,
  ): void {
    if (!element) {
      return;
    }
    let sites = this.boundListeners.get(element);
    if (!sites) {
      sites = new Set();
      this.boundListeners.set(element, sites);
    }
    const key = `${site}:${type}`;
    if (sites.has(key)) {
      return;
    }
    sites.add(key);
    element.addEventListener(type, handler);
  }

  /**
   * Return the Knowledge Graph to its home view: atlas mode, no focus, no
   * selection, camera reframed to the full layout. Bound to the
   * "Show full graph" toolbar button and Escape.
   */
  private resetKnowledgeGraphView(): void {
    this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "NODE_FOCUSED", nodeId: null });
    this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "MODE_SET", mode: "atlas" });
    this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "SELECTION_SET", nodeIds: [] });
    this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "FILTERS_SET", filters: { communities: [] } });
    this.state.knowledgeGraphLayout = null;
    this.knowledgeGraphLayoutSize = null;
    this.knowledgeGraphView.camera = null;
    this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "LAYOUT_STATUS_SET", status: "idle" });
    this.render();
  }

  private onCodeGraphFilterChange(control: HTMLElement): void {
    const key = control.dataset.codeFilter as keyof CodeGraphFilters | undefined;
    if (!key) return;
    const previousIncludeStale = this.state.codeGraph.filters.freshness.includes("stale");
    let shouldReload = false;
    if (key === "diagnostics") {
      const value = (control as HTMLSelectElement).value;
      if (value !== "all" && value !== "with_diagnostics" && value !== "without_diagnostics") return;
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, {
        type: "FILTERS_SET",
        filters: { diagnostics: value },
      });
    } else if (key === "pathPrefixes") {
      const values = (control as HTMLInputElement).value
        .split(",")
        .map((value) => value.trim())
        .filter(Boolean);
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, {
        type: "FILTERS_SET",
        filters: { pathPrefixes: values },
      });
    } else {
      const values = [...this.options.root.querySelectorAll<HTMLInputElement>(`[data-code-filter="${key}"][data-code-filter-value]`)]
        .filter((input) => input.checked)
        .map((input) => input.dataset.codeFilterValue)
        .filter((value): value is string => Boolean(value));
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, {
        type: "FILTERS_SET",
        filters: { [key]: values } as Partial<CodeGraphFilters>,
      });
      if (key === "repoIds") {
        const input = control as HTMLInputElement;
        const nextRepoId = input.checked
          ? input.dataset.codeFilterValue
          : values[0] ?? this.state.codeGraph.repoId;
        if (nextRepoId && nextRepoId !== this.state.codeGraph.repoId) {
          this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "REPO_SELECTED", repoId: nextRepoId });
          shouldReload = true;
        }
      }
    }
    shouldReload ||= previousIncludeStale !== this.state.codeGraph.filters.freshness.includes("stale");
    this.invalidateCodeGraphLayout();
    this.render();
    if (shouldReload) void this.loadCodeGraph();
  }

  private invalidateCodeGraphNavigation(): void {
    this.codeGraphNavigationVersion += 1;
    this.invalidateCodeGraphLayout();
  }

  private invalidateCodeGraphLayout(): void {
    this.codeGraphLayout = null;
    this.codeGraphLayoutRun += 1;
    this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "LAYOUT_STATUS_SET", status: "idle" });
  }

  private resetCodeGraphView(): void {
    this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "MODE_SET", mode: "atlas" });
    this.state.codeGraph = codeGraphReducer(this.state.codeGraph, {
      type: "TARGET_SET",
      symbolKey: null,
      path: null,
      runId: null,
    });
    this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "SELECTION_SET", nodeIds: [] });
    this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "BREADCRUMB_POP", index: -1 });
    this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "FILTERS_SET", filters: { pathPrefixes: [] } });
    this.invalidateCodeGraphNavigation();
    this.codeGraphView.camera = null;
    this.codeGraphView.overrides.clear();
    void this.loadCodeGraph();
  }

  private setCodeGraphMode(mode: CodeGraphMode): void {
    const currentBreadcrumb = this.state.codeGraph.breadcrumbs.at(-1);
    if (mode === "file" && (!this.state.codeGraph.path || currentBreadcrumb?.kind === "directory")) return;
    if (mode === "neighborhood" && !this.state.codeGraph.symbolKey) return;
    this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "MODE_SET", mode });
    if (mode === "atlas") {
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "TARGET_SET", symbolKey: null, path: null });
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "SELECTION_SET", nodeIds: [] });
    }
    this.invalidateCodeGraphNavigation();
    void this.loadCodeGraph();
  }

  /**
   * One drill level back: selected concept → its area, drilled area → the
   * atlas. Bound to Escape so the same key that narrows never strands the
   * operator; the "Show full graph" button still jumps straight home.
   */
  private stepBackKnowledgeGraphView(): void {
    const graph = this.state.knowledgeGraph;
    const drilled = graph.filters.communities.length > 0;
    if (graph.selectedNodeIds.length > 0 || graph.focusedNodeId !== null) {
      const wasNeighborhood = graph.mode === "neighborhood";
      this.state.knowledgeGraph = graphReducer(graph, { type: "NODE_FOCUSED", nodeId: null });
      this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "SELECTION_SET", nodeIds: [] });
      this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "MODE_SET", mode: drilled ? "community" : "atlas" });
      if (wasNeighborhood) {
        this.invalidateKnowledgeGraphLayout();
      }
      this.render();
      return;
    }
    if (drilled) {
      this.state.knowledgeGraph = graphReducer(graph, { type: "FILTERS_SET", filters: { communities: [] } });
      this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "MODE_SET", mode: "atlas" });
      this.invalidateKnowledgeGraphLayout();
      this.render();
      return;
    }
    this.resetKnowledgeGraphView();
  }

  /** Drill into an area cloud: community mode filtered to that area's members. */
  private drillIntoKnowledgeArea(areaId: string): void {
    const graph = this.state.knowledgeGraph;
    const alreadyDrilled = graph.mode === "community"
      && graph.filters.communities.length === 1
      && graph.filters.communities[0] === areaId;
    if (alreadyDrilled && graph.selectedNodeIds.length === 0 && graph.focusedNodeId === null) {
      return;
    }
    // Re-drilling into the current area still clears selection/focus (the
    // requested destination is the area view, not the open capsule) — it
    // just skips the redundant filter change and relayout.
    this.state.knowledgeGraph = graphReducer(graph, { type: "NODE_FOCUSED", nodeId: null });
    this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "SELECTION_SET", nodeIds: [] });
    if (!alreadyDrilled) {
      this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "COMMUNITY_SELECTED", communityId: areaId });
      this.invalidateKnowledgeGraphLayout();
    }
    this.render();
  }

  /**
   * Follow a capsule link (wiki link or the Linked concepts list) to its
   * node. Targets arrive as node ids ("tag:x"), concept ids
   * ("issues/COE-399"), bare issue keys, or labels — resolve in that order.
   * Following a link across areas re-drills into the target's community so
   * the selected node is always visible.
   */
  private openKnowledgeCapsuleLink(target: string): void {
    const snapshot = currentGraphSnapshot(this.state.knowledgeGraph);
    if (!snapshot) return;
    // OKF capsules store link targets verbatim, often as relative markdown
    // paths ("../issues/COE-124.md") that the snapshot has already resolved
    // to bare ids ("issues/COE-124") — normalize before matching.
    const normalized = target.replace(/^(\.\.?\/)+/, "").replace(/\.md$/i, "");
    const tail = normalized.split("/").at(-1) ?? normalized;
    const node = snapshot.nodes.find((candidate) => candidate.id === target)
      ?? snapshot.nodes.find((candidate) => candidate.concept_id === target)
      ?? snapshot.nodes.find((candidate) => candidate.concept_id === normalized)
      ?? snapshot.nodes.find((candidate) => candidate.concept_id?.split("/").at(-1) === tail)
      ?? snapshot.nodes.find((candidate) => candidate.label === target)
      ?? null;
    if (!node) return;
    this.selectKnowledgeNode(node);
  }

  /**
   * Select a node reached by navigation (capsule link, deep link) and land
   * where manual drilling would have: inside the node's area. From the
   * atlas this drills in; from another area it re-drills — unless the node
   * is already visible in the current area (secondary membership counts).
   * Nodes outside every area (e.g. the bundle node) widen back to the atlas.
   */
  private selectKnowledgeNode(node: MemoryGraphNode): void {
    const graph = this.state.knowledgeGraph;
    const targetCommunity = node.metrics?.community_id ?? null;
    const currentCommunity = graph.filters.communities[0] ?? null;
    const snapshot = currentGraphSnapshot(graph);
    const visibleInCurrentArea = currentCommunity !== null && (
      targetCommunity === currentCommunity
      || (snapshot?.communities.find((community) => community.id === currentCommunity)?.node_ids.includes(node.id) ?? false)
    );
    let relayout = false;
    this.state.knowledgeGraph = graphReducer(graph, { type: "NODE_FOCUSED", nodeId: null });
    if (targetCommunity !== null && !visibleInCurrentArea) {
      this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "COMMUNITY_SELECTED", communityId: targetCommunity });
      relayout = currentCommunity !== targetCommunity;
    } else if (targetCommunity === null && currentCommunity !== null) {
      this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "FILTERS_SET", filters: { communities: [] } });
      this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "MODE_SET", mode: "atlas" });
      relayout = true;
    }
    this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "SELECTION_SET", nodeIds: [node.id] });
    if (relayout) {
      this.invalidateKnowledgeGraphLayout();
    }
    this.render();
  }

  private invalidateKnowledgeGraphLayout(): void {
    this.state.knowledgeGraphLayout = null;
    this.knowledgeGraphLayoutSize = null;
    this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "LAYOUT_STATUS_SET", status: "idle" });
    this.scheduleKnowledgeGraphLayout();
  }

  /** Selected concept's capsule detail + error, for the inspector render. */
  private selectedKnowledgeCapsule(): { conceptDetail: MemoryConceptDetail | null; conceptDetailError: string | null } {
    const graph = this.state.knowledgeGraph;
    const bundleId = graph.selectedBundleId;
    const node = this.selectedKnowledgeConcept();
    if (!bundleId || !node?.concept_id) {
      return { conceptDetail: null, conceptDetailError: null };
    }
    const key = `${bundleId}:${node.concept_id}`;
    return {
      conceptDetail: cachedConceptDetail(graph, bundleId, node.concept_id),
      conceptDetailError: this.knowledgeCapsuleError?.key === key ? this.knowledgeCapsuleError.message : null,
    };
  }

  private selectedKnowledgeConcept(): MemoryGraphNode | null {
    const snapshot = currentGraphSnapshot(this.state.knowledgeGraph);
    if (!snapshot) return null;
    const selected = new Set(this.state.knowledgeGraph.selectedNodeIds);
    return snapshot.nodes.find((node) => selected.has(node.id) && node.kind === "concept" && node.concept_id) ?? null;
  }

  /**
   * Lazily fetch the selected concept's capsule. Idempotent per render:
   * cached details and in-flight or failed requests are not re-requested (the
   * failure is keyed, so selecting a different node retries cleanly and the
   * inline Retry button clears it explicitly).
   */
  private async ensureSelectedConceptDetail(): Promise<void> {
    const graph = this.state.knowledgeGraph;
    const bundleId = graph.selectedBundleId;
    const node = this.selectedKnowledgeConcept();
    if (!this.graphAdapter || !bundleId || !node?.concept_id) return;
    const conceptId = node.concept_id;
    // A cached-but-stale detail keeps rendering while we refetch it in the
    // background (a newer snapshot marked it stale); only a truly-cached
    // fresh detail skips the fetch. The refetch swaps in atomically via
    // CONCEPT_DETAIL_LOADED, so the open capsule never blanks or scrolls
    // back to the top on a background snapshot tick.
    if (cachedConceptDetail(graph, bundleId, conceptId) && !isConceptDetailStale(graph, bundleId, conceptId)) {
      return;
    }
    const key = `${bundleId}:${conceptId}`;
    if (this.knowledgeCapsuleRequest === key || this.knowledgeCapsuleError?.key === key) return;
    this.knowledgeCapsuleRequest = key;
    // Snapshot generation guard: an accepted refresh while this request is
    // in flight invalidates the bundle's capsule cache, and this response
    // may predate the refresh — writing it back would pin stale markdown
    // against the current graph. Discard it instead; the follow-up render
    // refetches against the new snapshot.
    const cursorBefore = graph.snapshots[bundleId]?.cursor;
    try {
      const detail = await this.graphAdapter.getConceptDetail(bundleId, conceptId);
      if (this.destroyed) return;
      const cursorAfter = this.state.knowledgeGraph.snapshots[bundleId]?.cursor;
      const superseded = cursorBefore === undefined
        || cursorAfter === undefined
        || cursorAfter.partition !== cursorBefore.partition
        || cursorAfter.sequence !== cursorBefore.sequence;
      if (!superseded) {
        // Cache under the id we asked for: a server echoing an alias id
        // would otherwise miss the cache and refetch on every render.
        const normalized = detail.concept_id === conceptId ? detail : { ...detail, concept_id: conceptId };
        this.state.knowledgeGraph = graphReducer(this.state.knowledgeGraph, { type: "CONCEPT_DETAIL_LOADED", detail: normalized });
      }
    } catch (error) {
      if (this.destroyed) return;
      this.knowledgeCapsuleError = { key, message: errorMessage(error) };
    } finally {
      if (this.knowledgeCapsuleRequest === key) {
        this.knowledgeCapsuleRequest = null;
      }
    }
    this.render();
  }

  /**
   * Idempotent property handlers (the DOM morph preserves elements across
   * renders) for the drill affordances that live outside the canvas:
   * breadcrumbs, capsule links, capsule retry, and the deep-link copy button.
   */
  private bindKnowledgeGraphNavigation(root: HTMLElement): void {
    root.querySelectorAll<HTMLElement>("[data-kg-crumb]").forEach((button) => {
      button.onclick = () => {
        if (button.dataset.kgCrumb === "atlas") {
          this.resetKnowledgeGraphView();
        } else {
          this.stepBackKnowledgeGraphView();
        }
      };
    });
    root.querySelectorAll<HTMLElement>("[data-kg-link-target]").forEach((button) => {
      button.onclick = () => {
        const target = button.dataset.kgLinkTarget;
        if (target) this.openKnowledgeCapsuleLink(target);
      };
    });
    const retry = root.querySelector<HTMLElement>("[data-kg-capsule-retry]");
    if (retry) {
      retry.onclick = () => {
        this.knowledgeCapsuleError = null;
        void this.ensureSelectedConceptDetail();
        this.render();
      };
    }
    const copy = root.querySelector<HTMLButtonElement>("[data-kg-copy-deeplink]");
    if (copy) {
      copy.onclick = () => {
        const link = copy.dataset.kgCopyDeeplink;
        if (!link) return;
        void navigator.clipboard?.writeText(link).catch(() => undefined);
        copy.textContent = "Copied";
        setTimeout(() => {
          if (copy.isConnected) copy.textContent = "Copy deep link";
        }, 1_200);
      };
    }
  }

  async openMemoryDeepLink(url: string): Promise<boolean> {
    const link = parseMemoryDeepLink(url);
    if (!link || !this.graphAdapter) return false;
    this.state.activeView = "dashboard";
    this.state.graphPaneView = "knowledge";
    await this.loadKnowledgeGraph(link.bundleId);
    if (this.destroyed) return false;
    const snapshot = currentGraphSnapshot(this.state.knowledgeGraph);
    if (!snapshot || snapshot.bundle_id !== link.bundleId) {
      this.render();
      return false;
    }
    if (link.communityId) {
      if (!snapshot.communities.some((community) => community.id === link.communityId)) {
        this.render();
        return false;
      }
      this.drillIntoKnowledgeArea(link.communityId);
      return true;
    }
    if (link.conceptId) {
      const node = resolveMemoryDeepLinkNode(snapshot, link);
      if (!node) {
        this.render();
        return false;
      }
      // Lands drilled into the concept's area with the capsule open, exactly
      // where manual navigation would have ended up.
      this.selectKnowledgeNode(node);
      return true;
    }
    this.resetKnowledgeGraphView();
    return true;
  }

  async openCodeDeepLink(url: string): Promise<boolean> {
    const link = parseCodeDeepLink(url);
    if (!link || !this.codeGraphAdapter) return false;
    this.state.activeView = "dashboard";
    this.state.graphPaneView = "code";
    this.state.codeGraph = codeGraphReducer(this.state.codeGraph, {
      type: "HISTORY_RESTORED",
      state: codeDeepLinkToGraphState(link),
    });
    this.state.codeGraph = { ...this.state.codeGraph, breadcrumbs: [] };
    this.invalidateCodeGraphNavigation();
    await this.loadCodeGraph();
    if (this.destroyed) return false;
    const snapshot = this.visibleCodeGraphSnapshot();
    if (!snapshot || snapshot.repo_id !== link.repoId) {
      this.render();
      return false;
    }
    if (link.symbolKey) {
      const node = snapshot.nodes.find((candidate) => candidate.symbol_key === link.symbolKey);
      if (!node) {
        this.render();
        return false;
      }
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "NODE_SELECTED", nodeId: node.id });
    } else if (link.path) {
      const node = snapshot.nodes.find((candidate) => candidate.path_display === link.path);
      if (!node) {
        this.render();
        return false;
      }
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "NODE_SELECTED", nodeId: node.id });
    }
    this.render();
    return true;
  }

  /**
   * Emphasize dependency edges for a hovered or selected task, across all
   * three panes. Pure class toggling on the already-rendered DOM — hover
   * must never trigger a re-render.
   *
   * - Current-pane focus: its incoming and outgoing edges (including
   *   cross-pane edges into the Backlog) bolden; everything else recedes.
   * - Backlog focus: the full ancestry critical path boldens — every
   *   unfinished upstream edge chain that must complete to unblock it —
   *   and unrelated backlog cards dim.
   */
  private applyTaskGraphEmphasis(focusId: string | null): void {
    const container = this.options.root.querySelector<HTMLElement>("[data-tg-panes]");
    if (!container) {
      return;
    }
    const paths = container.querySelectorAll<SVGPathElement>(".os-task-graph-link, .os-tg-cross-link");
    const cards = container.querySelectorAll<HTMLElement>("[data-node-id]");
    paths.forEach((path) => path.classList.remove("is-active", "is-ancestry"));
    cards.forEach((card) => card.classList.remove("os-tg-dim", "os-tg-ancestry"));
    const graph = this.state.taskGraph;
    const node = focusId ? graph?.nodes.find((candidate) => candidate.node_id === focusId) : undefined;
    // Only enter focused mode when the focused node actually has a rendered
    // card. A selection that transitioned to `done` during a live refresh
    // moved to the Completed table, so its node still exists in the snapshot
    // but no card is drawn — focusing it would dim every edge with nothing
    // highlighted until the user picks another visible task.
    const hasRenderedCard = Boolean(
      focusId && container.querySelector(`[data-node-id="${cssEscape(focusId)}"]`),
    );
    container.classList.toggle("os-tg-focused", Boolean(node) && hasRenderedCard);
    if (!graph || !node || !hasRenderedCard) {
      return;
    }
    if (node.state_category === "backlog") {
      const ancestry = collectAncestryEdges(graph.nodes, node);
      paths.forEach((path) => {
        const key = `${path.dataset.linkFrom}->${path.dataset.linkTo}`;
        if (ancestry.edges.has(key)) {
          path.classList.add("is-ancestry");
        }
      });
      cards.forEach((card) => {
        const id = card.dataset.nodeId ?? "";
        if (ancestry.members.has(id)) {
          card.classList.add("os-tg-ancestry");
        } else if (card.closest("[data-tg-pane='backlog']")) {
          card.classList.add("os-tg-dim");
        }
      });
      return;
    }
    paths.forEach((path) => {
      if (path.dataset.linkFrom === focusId || path.dataset.linkTo === focusId) {
        path.classList.add("is-active");
      }
    });
  }

  /** Coalesce cross-link repositioning to one measurement per frame. */
  private scheduleCrossLinksReposition(): void {
    if (this.destroyed || this.crossLinksFrame !== null) {
      return;
    }
    const raf = this.options.root.ownerDocument.defaultView?.requestAnimationFrame?.bind(
      this.options.root.ownerDocument.defaultView,
    );
    if (!raf) {
      this.positionTaskGraphCrossLinks();
      return;
    }
    this.crossLinksFrame = raf(() => {
      this.crossLinksFrame = null;
      this.positionTaskGraphCrossLinks();
    });
  }

  /**
   * Give every cross-pane edge its geometry: from the right edge of the
   * blocking card in the Current pane to the left edge of the blocked card
   * in the Backlog pane, measured against the live layout. Edges whose
   * endpoint scrolled out of its pane (or whose pane is collapsed) hide
   * instead of drawing across headers.
   */
  private positionTaskGraphCrossLinks(): void {
    const container = this.options.root.querySelector<HTMLElement>("[data-tg-panes]");
    const svg = container?.querySelector<SVGSVGElement>("[data-tg-cross-links]");
    if (!container || !svg) {
      return;
    }
    const containerRect = container.getBoundingClientRect();
    if (containerRect.width <= 0 || containerRect.height <= 0) {
      return;
    }
    svg.setAttribute("viewBox", `0 0 ${containerRect.width} ${containerRect.height}`);
    const paneBodyRect = (pane: string): DOMRect | null =>
      container.querySelector(`[data-tg-pane-body="${pane}"]`)?.getBoundingClientRect() ?? null;
    const currentBody = paneBodyRect("current");
    const backlogBody = paneBodyRect("backlog");
    svg.querySelectorAll<SVGPathElement>(".os-tg-cross-link").forEach((path) => {
      const fromId = path.dataset.linkFrom;
      const toId = path.dataset.linkTo;
      const from = fromId ? container.querySelector(`[data-node-id="${cssEscape(fromId)}"]`) : null;
      const to = toId ? container.querySelector(`[data-node-id="${cssEscape(toId)}"]`) : null;
      if (!from || !to || !currentBody || !backlogBody) {
        path.style.display = "none";
        return;
      }
      const fromRect = from.getBoundingClientRect();
      const toRect = to.getBoundingClientRect();
      const y1 = fromRect.top + fromRect.height / 2;
      const y2 = toRect.top + toRect.height / 2;
      const fromVisible = y1 >= currentBody.top - 4 && y1 <= currentBody.bottom + 4;
      const toVisible = y2 >= backlogBody.top - 4 && y2 <= backlogBody.bottom + 4;
      if (!fromVisible || !toVisible) {
        path.style.display = "none";
        return;
      }
      const x1 = Math.min(fromRect.right, currentBody.right) - containerRect.left;
      const x2 = toRect.left - containerRect.left;
      const bend = Math.max(28, (x2 - x1) * 0.45);
      path.setAttribute(
        "d",
        `M ${x1} ${y1 - containerRect.top} C ${x1 + bend} ${y1 - containerRect.top}, ${x2 - bend} ${y2 - containerRect.top}, ${x2} ${y2 - containerRect.top}`,
      );
      path.style.display = "";
    });
  }

  private bindEvents(): void {
    this.listen(this.options.root.querySelector("[data-save-profile]"), "save-profile", "click", () => {
      void this.saveProfile();
    });
    this.listen(this.options.root.querySelector("[data-new-profile]"), "new-profile", "click", () => {
      void this.createProfileDraft();
    });
    this.listen(this.options.root.querySelector("[data-remove-profile]"), "remove-profile", "click", () => {
      void this.removeProfile();
    });
    this.listen(this.options.root.querySelector("[data-save-model-profile]"), "save-model-profile", "click", () => {
      void this.saveModelProfile();
    });
    this.listen(this.options.root.querySelector("[data-new-model-profile]"), "new-model-profile", "click", () => {
      void this.createModelProfileDraft();
    });
    this.listen(this.options.root.querySelector("[data-remove-model-profile]"), "remove-model-profile", "click", () => {
      void this.removeModelProfile();
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-toggle-settings]").forEach((button) => {
      this.listen(button, "toggle-settings", "click", () => {
        const panel = button.dataset.toggleSettings;
        if (panel === "connection" || panel === "model") {
          this.toggleSettingsPanel(panel);
        }
      });
    });
    this.listen(this.options.root.querySelector("[data-open-event-log]"), "open-event-log", "click", () => {
      this.state.eventLogModalOpen = true;
      this.render();
    });
    this.listen(this.options.root.querySelector("[data-close-event-log]"), "close-event-log", "click", () => {
      this.state.eventLogModalOpen = false;
      this.render();
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-auth-action]").forEach((button) => {
      this.listen(button, "auth-action", "click", () => {
        const action = button.dataset.authAction;
        if (action === "sign-in") {
          // Hosted auth provider integration is a follow-on task; the
          // placeholder triggers a refresh so an operator-supplied session
          // (or a newly-permitted gateway) is re-evaluated.
          void this.refresh();
        } else if (action === "refresh") {
          void this.refresh();
        }
      });
    });
    this.listen(this.options.root.querySelector("[data-profile-select]"), "profile-select", "change", (event) => {
      const target = event.target as HTMLSelectElement;
      void this.selectProfile(target.value);
    });
    this.listen(this.options.root.querySelector("[data-model-profile-select]"), "model-profile-select", "change", (event) => {
      const target = event.target as HTMLSelectElement;
      void this.selectModelProfile(target.value);
    });
    this.listen(this.options.root.querySelector("[data-model-mode]"), "model-mode", "change", (event) => {
      const target = event.target as HTMLSelectElement;
      this.changeModelProfileMode(modelModeFromValue(target.value));
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-project-id]").forEach((button) => {
      this.listen(button, "project-id", "click", () => {
        const projectId = button.dataset.projectId;
        if (projectId) {
          void this.selectProject(projectId);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-node-id]").forEach((button) => {
      this.listen(button, "node-id", "click", (event) => {
        // Avoid selecting when clicking inline editor controls or action buttons.
        const target = event.target as HTMLElement;
        if (target.closest(".os-node-actions, .os-inline-input, .os-node-badges")) {
          return;
        }
        const node = this.state.taskGraph?.nodes.find(
          (candidate) => candidate.node_id === button.dataset.nodeId,
        );
        if (node) {
          if (this.options.mode === "desktop" && node.state_category !== "backlog") {
            void this.openRun(node);
          } else {
            // Backlog tasks have no run to open; selecting one pins its
            // ancestry critical path instead. Still a navigation: bump the
            // guards so an in-flight openRun or live refresh cannot land a
            // stale run detail (or probe /runs/{backlog id}) over this
            // selection.
            this.interactionEpoch += 1;
            this.runOpenSeq += 1;
            this.diffSelectSeq += 1;
            this.state.selectedNodeId = node.node_id;
            this.render();
          }
        }
      });
      // Hovering a task spotlights its dependency edges (and for backlog
      // tasks the full ancestry path): pure class toggling on the
      // already-rendered SVG, no re-render involved.
      this.listen(button, "node-id-hover", "pointerenter", () => {
        this.applyTaskGraphEmphasis(button.dataset.nodeId ?? null);
      });
      this.listen(button, "node-id-hover", "pointerleave", () => {
        this.applyTaskGraphEmphasis(this.state.selectedNodeId);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-project-group-toggle]").forEach((button) => {
      this.listen(button, "project-group-toggle", "click", () => {
        const projectKey = button.dataset.projectGroupToggle;
        if (!projectKey) return;
        if (this.state.collapsedProjectGroups.has(projectKey)) {
          this.state.collapsedProjectGroups.delete(projectKey);
        } else {
          this.state.collapsedProjectGroups.add(projectKey);
        }
        this.render();
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-open-run]").forEach((button) => {
      this.listen(button, "open-run", "click", () => {
        const node = this.state.taskGraph?.nodes.find(
          (candidate) => candidate.node_id === button.dataset.openRun,
        );
        if (node) {
          void this.openRun(node);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-testid='changed-file-item']").forEach((button) => {
      this.listen(button, "changed-file-item", "click", () => {
        const path = button.dataset.path;
        if (path) {
          void this.selectDiffFile(path);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-evidence-view]").forEach((button) => {
      this.listen(button, "evidence-view", "click", () => {
        const view = button.dataset.evidenceView;
        if (view === "diff" || view === "activity") {
          this.selectEvidenceView(view);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-graph-view]").forEach((button) => {
      this.listen(button, "graph-view", "click", () => {
        const view = button.dataset.graphView;
        if (view === "task" || view === "knowledge" || view === "code") {
          this.selectGraphPaneView(view);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-code-mode]").forEach((button) => {
      this.listen(button, "code-mode", "click", () => {
        const mode = button.dataset.codeMode;
        if (mode === "atlas" || mode === "file" || mode === "neighborhood" || mode === "diff") {
          this.setCodeGraphMode(mode);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-code-filter]").forEach((control) => {
      const eventType = control instanceof HTMLInputElement && control.type === "text" ? "input" : "change";
      this.listen(control, "code-filter", eventType, () => this.onCodeGraphFilterChange(control));
    });
    this.listen(this.options.root.querySelector("[data-code-filter-reset]"), "code-filter-reset", "click", () => {
      const hadStaleFilter = this.state.codeGraph.filters.freshness.includes("stale");
      this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "FILTERS_RESET" });
      this.invalidateCodeGraphLayout();
      this.render();
      if (hadStaleFilter) void this.loadCodeGraph();
    });
    this.listen(this.options.root.querySelector("[data-code-reset]"), "code-reset", "click", () => {
      this.resetCodeGraphView();
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-code-crumb]").forEach((button) => {
      this.listen(button, "code-crumb", "click", () => {
        const index = Number.parseInt(button.dataset.codeCrumb ?? "-1", 10);
        if (index < 0) {
          this.resetCodeGraphView();
        } else {
          this.state.codeGraph = codeGraphReducer(this.state.codeGraph, { type: "BREADCRUMB_POP", index });
          this.invalidateCodeGraphLayout();
          void this.loadCodeGraph();
        }
      });
    });
    this.listen(this.options.root.querySelector("[data-code-raw-toggle]"), "code-raw-toggle", "click", () => {
      this.codeGraphRawRecord = !this.codeGraphRawRecord;
      this.render();
    });
    this.listen(this.options.root.querySelector("[data-code-copy-deeplink]"), "code-copy-deeplink", "click", (event) => {
      const link = (event.currentTarget as HTMLElement).dataset.codeCopyDeeplink;
      if (link) void navigator.clipboard?.writeText(link).catch(() => undefined);
    });
    this.listen(this.options.root.querySelector("[data-kg-reset]"), "kg-reset", "click", () => {
      this.resetKnowledgeGraphView();
    });
    this.bindKnowledgeGraph();
    this.bindCodeGraph();
    this.options.root.querySelectorAll<HTMLElement>("[data-pane-resizer]").forEach((handle) => {
      this.listen(handle, "pane-resizer", "pointerdown", (event) => {
        this.startPaneResize(handle.dataset.paneResizer, event as PointerEvent);
      });
      this.listen(handle, "pane-resizer", "keydown", (event) => {
        this.onPaneResizeKey(handle.dataset.paneResizer, event as KeyboardEvent);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-resizer]").forEach((handle) => {
      this.listen(handle, "tg-resizer", "pointerdown", (event) => {
        this.startTaskPaneResize(handle.dataset.tgResizer, event as PointerEvent);
      });
      this.listen(handle, "tg-resizer", "keydown", (event) => {
        this.onTaskPaneResizeKey(handle.dataset.tgResizer, event as KeyboardEvent);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-row-resizer]").forEach((handle) => {
      this.listen(handle, "row-resizer", "pointerdown", (event) => {
        this.startLowerRowResize(handle.dataset.rowResizer, event as PointerEvent);
      });
      this.listen(handle, "row-resizer", "keydown", (event) => {
        this.onLowerRowResizeKey(handle.dataset.rowResizer, event as KeyboardEvent);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-activity-toggle]").forEach((button) => {
      this.listen(button, "activity-toggle", "click", () => {
        const eventKey = button.dataset.activityToggle;
        if (eventKey) {
          this.toggleActivityEvent(eventKey);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-testid='run-action-button']").forEach((button) => {
      this.listen(button, "run-action-button", "click", () => {
        const action = button.dataset.action as RunAction | undefined;
        if (action) {
          void this.dispatchRunAction(action);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-testid='approve-button']").forEach((button) => {
      this.listen(button, "approve-button", "click", () => {
        const approvalId = button.dataset.approvalId;
        if (!approvalId) return;
        const container = button.closest("[data-testid='approval-item']");
        const explanation = container?.querySelector<HTMLInputElement>("[data-testid='approval-explanation']")?.value;
        void this.submitApprovalDecision(approvalId, "approved", explanation);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-testid='deny-button']").forEach((button) => {
      this.listen(button, "deny-button", "click", () => {
        const approvalId = button.dataset.approvalId;
        if (!approvalId) return;
        const container = button.closest("[data-testid='approval-item']");
        const explanation = container?.querySelector<HTMLInputElement>("[data-testid='approval-explanation']")?.value;
        void this.submitApprovalDecision(approvalId, "rejected", explanation);
      });
    });


    // Three-pane task graph: collapse toggles, completed-tasks controls,
    // capsule deep links, and cross-pane edge geometry.
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-pane-toggle]").forEach((button) => {
      this.listen(button, "tg-pane-toggle", "click", () => {
        const pane = button.dataset.tgPaneToggle;
        if (pane !== "done" && pane !== "backlog") return;
        this.state.taskPaneCollapsed = {
          ...this.state.taskPaneCollapsed,
          [pane]: !this.state.taskPaneCollapsed[pane],
        };
        this.render();
      });
    });
    this.listen(this.options.root.querySelector("[data-tg-done-search]"), "tg-done-search", "input", (event) => {
      const value = (event.target as HTMLInputElement).value;
      if (this.completedTasksSearchTimer !== null) {
        clearTimeout(this.completedTasksSearchTimer);
      }
      // Debounced so every keystroke does not hit the memory server; the
      // seq guard in loadCompletedTasks handles any remaining races.
      this.completedTasksSearchTimer = setTimeout(() => {
        this.completedTasksSearchTimer = null;
        if (this.destroyed || this.state.completedTasksParams.query === value) return;
        this.state.completedTasksParams = { ...this.state.completedTasksParams, query: value, page: 1 };
        void this.loadCompletedTasks();
      }, 180);
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-done-sort]").forEach((button) => {
      this.listen(button, "tg-done-sort", "click", () => {
        const column = button.dataset.tgDoneSort;
        const sorts = column ? completedSortColumns[column] : undefined;
        if (!sorts) return;
        const active = this.state.completedTasksParams.sort;
        const next = active === sorts.first
          ? (sorts.first === sorts.asc ? sorts.desc : sorts.asc)
          : sorts.first;
        this.state.completedTasksParams = { ...this.state.completedTasksParams, sort: next, page: 1 };
        void this.loadCompletedTasks();
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-done-page]").forEach((button) => {
      this.listen(button, "tg-done-page", "click", () => {
        const value = button.dataset.tgDonePage;
        const params = this.state.completedTasksParams;
        const total = this.state.completedTasks?.total ?? 0;
        const pageCount = Math.max(1, Math.ceil(total / completedTasksPageSize));
        const nextPage = value === "prev"
          ? params.page - 1
          : value === "next"
            ? params.page + 1
            : Number.parseInt(value ?? "", 10);
        if (!Number.isFinite(nextPage) || nextPage < 1 || nextPage > pageCount || nextPage === params.page) return;
        this.state.completedTasksParams = { ...params, page: nextPage };
        void this.loadCompletedTasks();
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-capsule]").forEach((button) => {
      this.listen(button, "tg-capsule", "click", () => {
        const link = button.dataset.tgCapsule;
        if (link) {
          void this.openMemoryDeepLink(link);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-pane-body], .os-task-graph-stage").forEach((element) => {
      this.listen(element, "tg-cross-scroll", "scroll", () => this.scheduleCrossLinksReposition());
    });
    if (this.options.mode === "desktop" && this.state.graphPaneView === "task") {
      // Selection emphasis and cross-pane geometry re-apply after every
      // morph; hover emphasis stays purely event-driven.
      this.applyTaskGraphEmphasis(this.state.selectedNodeId);
      this.scheduleCrossLinksReposition();
    }

    // Task graph filters
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-filter]").forEach((control) => {
      this.listen(control, "tg-filter", "change", () => this.onFilterChange());
      this.listen(control, "tg-filter", "input", () => this.onFilterChange());
    });
    this.listen(this.options.root.querySelector("[data-tg-filter-reset]"), "tg-filter-reset", "click", () => {
      this.state.taskGraphFilter = { ...defaultTaskGraphFilter };
      this.render();
    });

    this.options.root.querySelectorAll<HTMLElement>("[data-tg-create]").forEach((button) => {
      this.listen(button, "tg-create", "click", () => {
        const kind = button.dataset.tgCreate as TaskGraphNodeKind;
        this.openCreateDialog(kind, null);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-create-child]").forEach((button) => {
      this.listen(button, "tg-create-child", "click", () => {
        const parentId = button.dataset.tgCreateChild;
        if (!parentId) return;
        const parent = this.state.taskGraph?.nodes.find((node) => node.node_id === parentId);
        if (!parent) return;
        const childKind: TaskGraphNodeKind = parent.kind === "milestone" ? "issue" : "sub_issue";
        this.openCreateDialog(childKind, parentId);
      });
    });
    this.listen(this.options.root.querySelector("[data-tg-create-save]"), "tg-create-save", "click", () => {
      void this.saveCreateDialog();
    });
    this.listen(this.options.root.querySelector("[data-tg-create-cancel]"), "tg-create-cancel", "click", () => {
      this.state.createDialog = { ...emptyEditorDialog };
      this.render();
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-edit]").forEach((button) => {
      this.listen(button, "tg-edit", "click", () => {
        const nodeId = button.dataset.tgEdit;
        if (nodeId) this.startInlineEdit(nodeId);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-inline-save]").forEach((button) => {
      this.listen(button, "tg-inline-save", "click", () => {
        const nodeId = button.dataset.tgInlineSave;
        if (nodeId) void this.saveInlineEdit(nodeId);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-inline-cancel]").forEach((button) => {
      this.listen(button, "tg-inline-cancel", "click", () => {
        this.state.inlineEdit = { ...emptyInlineEdit };
        this.render();
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-deps]").forEach((button) => {
      this.listen(button, "tg-deps", "click", () => {
        const nodeId = button.dataset.tgDeps;
        if (nodeId) this.openDependencyEditor(nodeId);
      });
    });
    this.listen(this.options.root.querySelector("[data-tg-deps-save]"), "tg-deps-save", "click", () => {
      void this.saveDependencyEdit();
    });
    this.listen(this.options.root.querySelector("[data-tg-deps-cancel]"), "tg-deps-cancel", "click", () => {
      this.state.dependencyEdit = { ...emptyDependencyEdit };
      this.render();
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-comment]").forEach((button) => {
      this.listen(button, "tg-comment", "click", () => {
        const nodeId = button.dataset.tgComment;
        if (nodeId) this.openCommentEditor(nodeId);
      });
    });
    this.listen(this.options.root.querySelector("[data-tg-comment-save]"), "tg-comment-save", "click", () => {
      void this.saveCommentEdit();
    });
    this.listen(this.options.root.querySelector("[data-tg-comment-cancel]"), "tg-comment-cancel", "click", () => {
      this.state.commentEdit = { ...emptyCommentEdit };
      this.render();
    });

    // Planning workspace view navigation
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-view]").forEach((button) => {
      this.listen(button, "plan-view", "click", () => {
        const view = button.dataset.planView as AppState["activeView"];
        if (view) {
          this.state.activeView = view;
          this.render();
        }
      });
    });

    // Planning workspace tabs
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-tab]").forEach((button) => {
      this.listen(button, "plan-tab", "click", () => {
        const tab = button.dataset.planTab;
        if (!tab) return;
        this.state.planningWorkspace = { ...this.state.planningWorkspace, activeTab: tab as typeof this.state.planningWorkspace.activeTab };
        this.render();
      });
    });

    // Planning conversation
    this.listen(this.options.root.querySelector("[data-plan-send-message]"), "plan-send-message", "click", () => {
      this.sendPlanMessage();
    });
    this.listen(this.options.root.querySelector("[data-plan-composer]"), "plan-composer", "keydown", (event) => {
      if ((event as KeyboardEvent).key === "Enter" && !(event as KeyboardEvent).shiftKey) {
        (event as KeyboardEvent).preventDefault();
        this.sendPlanMessage();
      }
    });
    this.listen(this.options.root.querySelector("[data-plan-composer]"), "plan-composer", "input", () => {
      this.state.planningWorkspace.composerDraft = this.options.root.querySelector<HTMLTextAreaElement>("[data-plan-composer]")?.value ?? "";
    });

    // Planning artifact editor
    this.listen(this.options.root.querySelector("[data-plan-artifact-select]"), "plan-artifact-select", "change", () => {
      const artifactId = this.options.root.querySelector<HTMLSelectElement>("[data-plan-artifact-select]")?.value ?? null;
      this.state.planningWorkspace = selectArtifact(this.state.planningWorkspace, artifactId);
      this.renderPreservingFocus();
    });
    this.listen(this.options.root.querySelector("[data-plan-revision-select]"), "plan-revision-select", "change", () => {
      const revisionId = this.options.root.querySelector<HTMLSelectElement>("[data-plan-revision-select]")?.value ?? null;
      this.state.planningWorkspace = selectRevision(this.state.planningWorkspace, revisionId);
      this.renderPreservingFocus();
    });
    this.listen(this.options.root.querySelector("[data-plan-save-artifact]"), "plan-save-artifact", "click", () => {
      this.savePlanArtifact();
    });
    this.listen(this.options.root.querySelector("[data-plan-add-artifact]"), "plan-add-artifact", "click", () => {
      this.addPlanArtifact();
    });
    this.listen(this.options.root.querySelector("[data-plan-artifact-content]"), "plan-artifact-content", "input", () => {
      // Content is not persisted continuously; only saved on explicit save.
    });

    // Planning hierarchy editor
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-node-select]").forEach((row) => {
      this.listen(row, "plan-node-select", "click", (event) => {
        if ((event.target as HTMLElement).closest(".os-node-actions, .os-plan-toggle, .os-plan-node-body input")) return;
        const nodeId = row.dataset.planNodeSelect;
        if (nodeId) {
          this.state.planningWorkspace = { ...this.state.planningWorkspace, selectedNodeId: nodeId };
          this.render();
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-node-toggle]").forEach((button) => {
      this.listen(button, "plan-node-toggle", "click", () => {
        const nodeId = button.dataset.planNodeToggle;
        if (nodeId) {
          this.state.planningWorkspace = toggleNodeExpanded(this.state.planningWorkspace, nodeId);
          this.render();
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-add-node]").forEach((button) => {
      this.listen(button, "plan-add-node", "click", () => {
        const kind = button.dataset.planAddNode as "milestone" | "issue" | "sub_issue";
        this.addPlanNode(kind, null);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-add-child]").forEach((button) => {
      this.listen(button, "plan-add-child", "click", () => {
        const parentId = button.dataset.planAddChild;
        if (!parentId) return;
        const parent = this.state.planningWorkspace.nodes.find((n) => n.node_id === parentId);
        if (!parent) return;
        const childKind: "milestone" | "issue" | "sub_issue" = parent.kind === "milestone" ? "issue" : "sub_issue";
        this.addPlanNode(childKind, parentId);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-node-edit]").forEach((button) => {
      this.listen(button, "plan-node-edit", "click", () => {
        const nodeId = button.dataset.planNodeEdit;
        if (!nodeId) return;
        const node = this.state.planningWorkspace.nodes.find((n) => n.node_id === nodeId);
        if (!node) return;
        this.state.planningEdit = { nodeId, title: node.title, state: node.state };
        this.render();
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-node-save]").forEach((button) => {
      this.listen(button, "plan-node-save", "click", () => {
        const nodeId = button.dataset.planNodeSave;
        if (nodeId) this.savePlanNodeEdit(nodeId);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-node-cancel]").forEach((button) => {
      this.listen(button, "plan-node-cancel", "click", () => {
        this.state.planningEdit = { ...emptyPlanningEditState };
        this.render();
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-remove-node]").forEach((button) => {
      this.listen(button, "plan-remove-node", "click", () => {
        const nodeId = button.dataset.planRemoveNode;
        if (nodeId) {
          this.state.planningWorkspace = removePlanningNode(this.state.planningWorkspace, nodeId);
          this.render();
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-graph-node]").forEach((node) => {
      this.listen(node, "plan-graph-node", "click", () => {
        const nodeId = node.dataset.planGraphNode;
        if (nodeId) {
          this.state.planningWorkspace = { ...this.state.planningWorkspace, selectedNodeId: nodeId };
          this.render();
        }
      });
    });

    // Planning dependency editor
    this.listen(this.options.root.querySelector("[data-plan-deps-node-select]"), "plan-deps-node-select", "change", () => {
      const nodeId = this.options.root.querySelector<HTMLSelectElement>("[data-plan-deps-node-select]")?.value ?? null;
      this.state.planningWorkspace = { ...this.state.planningWorkspace, selectedNodeId: nodeId };
      this.renderPreservingFocus();
    });
    this.listen(this.options.root.querySelector("[data-plan-deps-save]"), "plan-deps-save", "click", () => {
      this.savePlanDependencies();
    });

    // Planning acceptance criteria / verification editor
    this.listen(this.options.root.querySelector("[data-plan-criteria-add]"), "plan-criteria-add", "click", () => {
      const text = this.options.root.querySelector<HTMLInputElement>("[data-plan-criteria-new]")?.value ?? "";
      if (!text.trim()) return;
      this.state.planningWorkspace = addCriterion(this.state.planningWorkspace, text);
      this.render();
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-criteria-toggle]").forEach((checkbox) => {
      this.listen(checkbox, "plan-criteria-toggle", "change", () => {
        const id = checkbox.dataset.planCriteriaToggle;
        if (id) {
          this.state.planningWorkspace = toggleCriterion(this.state.planningWorkspace, id);
          this.render();
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-criteria-text]").forEach((input) => {
      this.listen(input, "plan-criteria-text", "input", () => {
        const id = input.dataset.planCriteriaText;
        const value = (input as HTMLInputElement).value;
        if (id) {
          this.state.planningWorkspace = updateCriterion(this.state.planningWorkspace, id, value);
          this.renderPreservingFocus();
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-criteria-remove]").forEach((button) => {
      this.listen(button, "plan-criteria-remove", "click", () => {
        const id = button.dataset.planCriteriaRemove;
        if (id) {
          this.state.planningWorkspace = removeCriterion(this.state.planningWorkspace, id);
          this.render();
        }
      });
    });
    this.listen(this.options.root.querySelector("[data-plan-verification-add]"), "plan-verification-add", "click", () => {
      const text = this.options.root.querySelector<HTMLInputElement>("[data-plan-verification-new]")?.value ?? "";
      if (!text.trim()) return;
      this.state.planningWorkspace = addVerification(this.state.planningWorkspace, text);
      this.render();
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-verification-toggle]").forEach((checkbox) => {
      this.listen(checkbox, "plan-verification-toggle", "change", () => {
        const id = checkbox.dataset.planVerificationToggle;
        if (id) {
          this.state.planningWorkspace = toggleVerification(this.state.planningWorkspace, id);
          this.render();
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-verification-text]").forEach((input) => {
      this.listen(input, "plan-verification-text", "input", () => {
        const id = input.dataset.planVerificationText;
        const value = (input as HTMLInputElement).value;
        if (id) {
          this.state.planningWorkspace = updateVerification(this.state.planningWorkspace, id, value);
          this.renderPreservingFocus();
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-verification-remove]").forEach((button) => {
      this.listen(button, "plan-verification-remove", "click", () => {
        const id = button.dataset.planVerificationRemove;
        if (id) {
          this.state.planningWorkspace = removeVerification(this.state.planningWorkspace, id);
          this.render();
        }
      });
    });

    // Planning validation links
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-validation-link]").forEach((link) => {
      this.listen(link, "plan-validation-link", "click", () => {
        this.followPlanValidationLink(link);
      });
    });

    // Planning diff revision selectors
    this.listen(this.options.root.querySelector("[data-plan-diff-left]"), "plan-diff-left", "change", () => {
      this.state.planningWorkspace = {
        ...this.state.planningWorkspace,
        diffLeftRevisionId: this.options.root.querySelector<HTMLSelectElement>("[data-plan-diff-left]")?.value ?? null,
      };
      this.renderPreservingFocus();
    });
    this.listen(this.options.root.querySelector("[data-plan-diff-right]"), "plan-diff-right", "change", () => {
      this.state.planningWorkspace = {
        ...this.state.planningWorkspace,
        diffRightRevisionId: this.options.root.querySelector<HTMLSelectElement>("[data-plan-diff-right]")?.value ?? null,
      };
      this.renderPreservingFocus();
    });
  }

  // -- Task graph filter handling --

  private onFilterChange(): void {
    const root = this.options.root;
    const state = (root.querySelector<HTMLSelectElement>("[data-tg-filter='state']")?.value ?? "all") as TaskGraphFilter["stateCategory"];
    const search = root.querySelector<HTMLInputElement>("[data-tg-filter='search']")?.value ?? "";
    this.state.taskGraphFilter = { stateCategory: state, search };
    this.renderPreservingFocus();
  }

  private renderPreservingFocus(): void {
    const root = this.options.root;
    const active = root.ownerDocument?.activeElement as HTMLElement | null;
    const tag = active?.tagName?.toLowerCase() ?? null;
    const dataAttrs = active
      ? Array.from(active.attributes)
          .filter((attr) => attr.name.startsWith("data-"))
          .map((attr) => ({ name: attr.name, value: attr.value }))
      : [];
    const input = active as HTMLInputElement | HTMLTextAreaElement | null;
    const selectionStart = input?.selectionStart ?? null;
    const selectionEnd = input?.selectionEnd ?? null;

    this.render();

    if (!tag || dataAttrs.length === 0) return;
    const candidates = Array.from(root.querySelectorAll<HTMLElement>(tag));
    const match = candidates.find((el) =>
      dataAttrs.every((attr) => el.getAttribute(attr.name) === attr.value),
    );
    if (match) {
      match.focus();
      if (selectionStart !== null && selectionEnd !== null && "setSelectionRange" in match) {
        (match as HTMLInputElement | HTMLTextAreaElement).setSelectionRange(selectionStart, selectionEnd);
      }
    }
  }

  private openCreateDialog(kind: TaskGraphNodeKind, parentId: string | null): void {
    this.state.createDialog = {
      open: true,
      kind,
      parentId,
      draftTitle: "",
      draftState: "Todo",
    };
    this.render();
  }

  private async saveCreateDialog(): Promise<void> {
    const dialog = this.state.createDialog;
    if (!dialog.open || !dialog.kind) return;
    const title = (this.options.root.querySelector<HTMLInputElement>("[data-tg-create-title]")?.value ?? "").trim();
    const state = (this.options.root.querySelector<HTMLInputElement>("[data-tg-create-state]")?.value ?? "Todo").trim() || "Todo";
    if (!title) return;

    const parentId = dialog.parentId ?? undefined;
    const nodeId = `new-${dialog.kind}-${generateId()}`;
    const newNode = buildCreatedNode({ parent_id: parentId, kind: dialog.kind, title, state }, nodeId);
    const taskGraph = this.state.taskGraph;
    if (taskGraph) {
      taskGraph.nodes.push(newNode);
      if (parentId) {
        const parent = taskGraph.nodes.find((node) => node.node_id === parentId);
        if (parent && !parent.children.includes(nodeId)) {
          parent.children.push(nodeId);
        }
      } else if (!taskGraph.root_ids.includes(nodeId)) {
        taskGraph.root_ids.push(nodeId);
      }
    }
    this.state.createDialog = { ...emptyEditorDialog };
    this.render();

    if (isActionCapable(this.transport)) {
      const correlationId = `tg-create-${parentId ?? "root"}-${dialog.kind}-${generateId()}`;
      this.state.pendingMutations.add(correlationId);
      this.state.pendingCreates.set(correlationId, nodeId);
      this.state.pendingSnapshots.set(correlationId, null);
      this.render();
      try {
        const receipt = await dispatchTaskGraphCreate(this.transport, {
          parent_id: parentId,
          kind: dialog.kind,
          title,
          state,
        }, correlationId);
        this.applyMutationReceipt(receipt);
      } catch (error) {
        this.rollbackOptimisticMutation(correlationId);
        this.state.connectionMessage = `Create failed: ${errorMessage(error)}`;
      }
      this.render();
    }
  }

  private startInlineEdit(nodeId: string): void {
    const node = this.state.taskGraph?.nodes.find((candidate) => candidate.node_id === nodeId);
    if (!node) return;
    this.state.inlineEdit = { nodeId, title: node.title, state: node.state };
    this.render();
  }

  private async saveInlineEdit(nodeId: string): Promise<void> {
    const title = Array.from(this.options.root.querySelectorAll<HTMLInputElement>("[data-tg-inline-title]")).find(
      (input) => input.dataset.tgInlineTitle === nodeId,
    )?.value.trim();
    const state = Array.from(this.options.root.querySelectorAll<HTMLInputElement>("[data-tg-inline-state]")).find(
      (input) => input.dataset.tgInlineState === nodeId,
    )?.value.trim();
    const node = this.state.taskGraph?.nodes.find((candidate) => candidate.node_id === nodeId);
    if (!node) return;
    const snapshot = { ...node };
    this.updateTaskGraphNode(applyNodeUpdate(node, { title, state }));
    this.state.inlineEdit = { ...emptyInlineEdit };
    this.render();

    if (isActionCapable(this.transport)) {
      const correlationId = `tg-update-${nodeId}-${generateId()}`;
      this.state.pendingMutations.add(correlationId);
      this.state.pendingSnapshots.set(correlationId, snapshot);
      this.render();
      try {
        const receipt = await dispatchTaskGraphUpdate(this.transport, { node_id: nodeId, title, state }, correlationId);
        this.applyMutationReceipt(receipt);
      } catch (error) {
        this.rollbackOptimisticMutation(correlationId);
        this.state.connectionMessage = `Update failed: ${errorMessage(error)}`;
      }
      this.render();
    }
  }

  private openDependencyEditor(nodeId: string): void {
    const node = this.state.taskGraph?.nodes.find((candidate) => candidate.node_id === nodeId);
    if (!node) return;
    this.state.dependencyEdit = { nodeId, blockedBy: [...node.blocked_by] };
    this.render();
  }

  private async saveDependencyEdit(): Promise<void> {
    const nodeId = this.state.dependencyEdit.nodeId;
    if (!nodeId) return;
    const select = this.options.root.querySelector<HTMLSelectElement>("[data-tg-deps-select]");
    const blockedBy = Array.from(select?.selectedOptions ?? []).map((option) => option.value);
    const node = this.state.taskGraph?.nodes.find((candidate) => candidate.node_id === nodeId);
    if (!node) return;
    const snapshot = { ...node };
    this.updateTaskGraphNode({ ...node, blocked_by: blockedBy });
    this.state.dependencyEdit = { ...emptyDependencyEdit };
    this.render();

    if (isActionCapable(this.transport)) {
      const correlationId = `tg-deps-${nodeId}-${generateId()}`;
      this.state.pendingMutations.add(correlationId);
      this.state.pendingSnapshots.set(correlationId, snapshot);
      this.render();
      try {
        const receipt = await dispatchTaskGraphDependencies(this.transport, { node_id: nodeId, blocked_by: blockedBy }, correlationId);
        this.applyMutationReceipt(receipt);
      } catch (error) {
        this.rollbackOptimisticMutation(correlationId);
        this.state.connectionMessage = `Dependency update failed: ${errorMessage(error)}`;
      }
      this.render();
    }
  }

  private openCommentEditor(nodeId: string): void {
    this.state.commentEdit = { nodeId, kind: "comment", body: "" };
    this.render();
  }

  private async saveCommentEdit(): Promise<void> {
    const nodeId = this.state.commentEdit.nodeId;
    if (!nodeId) return;
    const body = this.options.root.querySelector<HTMLTextAreaElement>("[data-tg-comment-body]")?.value.trim() ?? "";
    if (!body) return;
    const kind = (this.options.root.querySelector<HTMLSelectElement>("[data-tg-comment-kind]")?.value ?? "comment") as "comment" | "evidence";
    const node = this.state.taskGraph?.nodes.find((candidate) => candidate.node_id === nodeId);
    const snapshot = node ? { ...node } : null;
    if (node) {
      this.updateTaskGraphNode({ ...node, comment_count: (node.comment_count ?? 0) + 1 });
    }
    this.state.commentEdit = { ...emptyCommentEdit };
    this.render();

    if (isActionCapable(this.transport)) {
      const correlationId = `tg-comment-${nodeId}-${generateId()}`;
      this.state.pendingMutations.add(correlationId);
      if (snapshot) {
        this.state.pendingSnapshots.set(correlationId, snapshot);
      }
      this.render();
      try {
        const receipt = await dispatchTaskGraphComment(this.transport, { node_id: nodeId, body, kind }, correlationId);
        this.applyMutationReceipt(receipt);
      } catch (error) {
        this.rollbackOptimisticMutation(correlationId);
        this.state.connectionMessage = `Comment failed: ${errorMessage(error)}`;
      }
      this.render();
    }
  }

  private updateTaskGraphNode(updated: TaskGraphNode): void {
    const taskGraph = this.state.taskGraph;
    if (!taskGraph) return;
    const idx = taskGraph.nodes.findIndex((node) => node.node_id === updated.node_id);
    if (idx >= 0) {
      taskGraph.nodes[idx] = updated;
    }
  }

  private applyMutationReceipt(receipt: ActionReceipt): void {
    if (receipt.status !== "accepted") {
      this.rollbackOptimisticMutation(receipt.correlation_id);
      const detail = receipt.reason ? `: ${receipt.reason}` : "";
      this.state.connectionMessage = `Mutation ${receipt.status}${detail}`;
      return;
    }

    const result = receipt.result as { node_id?: string; updated_at?: string } | undefined;
    if (!result?.node_id || !result?.updated_at) {
      this.state.pendingMutations.delete(receipt.correlation_id);
      this.state.pendingCreates.delete(receipt.correlation_id);
      this.state.pendingSnapshots.delete(receipt.correlation_id);
      return;
    }

    const localNodeId = this.state.pendingCreates.get(receipt.correlation_id);
    if (localNodeId && localNodeId !== result.node_id) {
      this.reconcileNodeId(localNodeId, result.node_id);
    }
    this.state.pendingMutations.delete(receipt.correlation_id);
    this.state.pendingCreates.delete(receipt.correlation_id);
    this.state.pendingSnapshots.delete(receipt.correlation_id);

    const node = this.state.taskGraph?.nodes.find((candidate) => candidate.node_id === result.node_id);
    if (node) {
      this.updateTaskGraphNode({ ...node, updated_at: result.updated_at });
    }
  }

  private rollbackOptimisticMutation(correlationId: string): void {
    const snapshot = this.state.pendingSnapshots.get(correlationId);
    if (snapshot === undefined) {
      this.state.pendingMutations.delete(correlationId);
      this.state.pendingCreates.delete(correlationId);
      return;
    }

    const taskGraph = this.state.taskGraph;
    if (snapshot === null) {
      const localNodeId = this.state.pendingCreates.get(correlationId);
      if (taskGraph && localNodeId) {
        taskGraph.nodes = taskGraph.nodes.filter((node) => node.node_id !== localNodeId);
        taskGraph.root_ids = taskGraph.root_ids.filter((id) => id !== localNodeId);
        for (const node of taskGraph.nodes) {
          node.children = node.children.filter((id) => id !== localNodeId);
          if (node.parent_id === localNodeId) {
            node.parent_id = undefined;
          }
        }
      }
    } else if (taskGraph) {
      this.updateTaskGraphNode(snapshot);
    }

    this.state.pendingMutations.delete(correlationId);
    this.state.pendingCreates.delete(correlationId);
    this.state.pendingSnapshots.delete(correlationId);
  }

  private reconcileNodeId(oldId: string, newId: string): void {
    const taskGraph = this.state.taskGraph;
    if (!taskGraph) return;
    if (taskGraph.nodes.some((node) => node.node_id === newId && node.node_id !== oldId)) {
      this.state.connectionMessage = `Server returned a duplicate node ID (${newId}); optimistic ID not reconciled.`;
      return;
    }

    const node = taskGraph.nodes.find((candidate) => candidate.node_id === oldId);
    if (!node) return;
    node.node_id = newId;
    if (taskGraph.root_ids.includes(oldId)) {
      taskGraph.root_ids = taskGraph.root_ids.map((id) => (id === oldId ? newId : id));
    }
    for (const candidate of taskGraph.nodes) {
      if (candidate.parent_id === oldId) candidate.parent_id = newId;
      candidate.children = candidate.children.map((id) => (id === oldId ? newId : id));
      candidate.blocked_by = candidate.blocked_by.map((id) => (id === oldId ? newId : id));
    }
    if (this.state.selectedNodeId === oldId) this.state.selectedNodeId = newId;
    if (this.state.inlineEdit.nodeId === oldId) this.state.inlineEdit.nodeId = newId;
    if (this.state.dependencyEdit.nodeId === oldId) this.state.dependencyEdit.nodeId = newId;
    if (this.state.commentEdit.nodeId === oldId) this.state.commentEdit.nodeId = newId;
  }

  // -- Planning workspace handling --

  private sendPlanMessage(): void {
    const body = this.options.root.querySelector<HTMLTextAreaElement>("[data-plan-composer]")?.value ?? "";
    if (!body.trim()) return;
    this.state.planningWorkspace = addMessage(this.state.planningWorkspace, "user", body);
    this.state.planningWorkspace = addMessage(this.state.planningWorkspace, "assistant", "Acknowledged.");
    this.render();
  }

  private savePlanArtifact(): void {
    const artifactId = this.state.planningWorkspace.selectedArtifactId;
    if (!artifactId) return;
    const content = this.options.root.querySelector<HTMLTextAreaElement>("[data-plan-artifact-content]")?.value ?? "";
    this.state.planningWorkspace = updateArtifactContent(this.state.planningWorkspace, artifactId, content);
    this.renderPreservingFocus();
  }

  private addPlanArtifact(): void {
    const now = new Date().toISOString();
    const artifactId = `artifact-new-${generateId()}`;
    const revisionId = `rev-new-${generateId()}`;
    const newArtifact = {
      schema_version: schemaVersion,
      artifact_id: artifactId,
      session_id: this.state.planningWorkspace.session_id,
      kind: "intake" as const,
      title: "New Intake",
      created_at: now,
      updated_at: now,
      approved: false,
      published_to_tracker: false,
      revisions: [{ revision_id: revisionId, created_at: now, content: "" }],
    };
    this.state.planningWorkspace = {
      ...this.state.planningWorkspace,
      artifacts: [...this.state.planningWorkspace.artifacts, newArtifact],
      selectedArtifactId: artifactId,
      selectedRevisionId: revisionId,
    };
    this.render();
  }

  private addPlanNode(kind: "milestone" | "issue" | "sub_issue", parentId: string | null): void {
    this.state.planningWorkspace = addPlanningNode(
      this.state.planningWorkspace,
      kind,
      parentId,
      `New ${kind.replace(/_/g, " ")}`,
    );
    const newNodeId = this.state.planningWorkspace.selectedNodeId;
    const newNode = newNodeId
      ? this.state.planningWorkspace.nodes.find((n) => n.node_id === newNodeId)
      : undefined;
    this.state.planningEdit = newNode
      ? { nodeId: newNode.node_id, title: newNode.title, state: newNode.state }
      : { ...emptyPlanningEditState };
    this.render();
  }

  private savePlanNodeEdit(nodeId: string): void {
    const root = this.options.root;
    const title = Array.from(root.querySelectorAll<HTMLInputElement>("[data-plan-node-title]")).find(
      (el) => el.dataset.planNodeTitle === nodeId,
    )?.value.trim();
    const state = Array.from(root.querySelectorAll<HTMLInputElement>("[data-plan-node-state]")).find(
      (el) => el.dataset.planNodeState === nodeId,
    )?.value.trim();
    this.state.planningWorkspace = updatePlanningNode(this.state.planningWorkspace, nodeId, { title, state });
    this.state.planningEdit = { ...emptyPlanningEditState };
    this.render();
  }

  private savePlanDependencies(): void {
    const nodeId = this.state.planningWorkspace.selectedNodeId;
    if (!nodeId) return;
    const select = this.options.root.querySelector<HTMLSelectElement>("[data-plan-deps-select]");
    const blockedBy = Array.from(select?.selectedOptions ?? []).map((option) => option.value);
    this.state.planningWorkspace = updateNodeDependencies(this.state.planningWorkspace, nodeId, blockedBy);
    this.render();
  }

  private followPlanValidationLink(link: HTMLElement): void {
    const kind = link.dataset.planFieldKind;
    const id = link.dataset.planFieldId;
    if (!kind || !id) return;
    let planningWorkspace = this.state.planningWorkspace;
    if (kind === "artifact") {
      planningWorkspace = selectArtifact(planningWorkspace, id);
      planningWorkspace = { ...planningWorkspace, activeTab: "artifact" };
    } else if (kind === "node") {
      planningWorkspace = { ...planningWorkspace, selectedNodeId: id, activeTab: "hierarchy" };
      planningWorkspace = { ...planningWorkspace, expandedNodeIds: new Set(planningWorkspace.expandedNodeIds).add(id) };
    } else if (kind === "criteria" || kind === "verification") {
      planningWorkspace = { ...planningWorkspace, activeTab: "criteria" };
    } else if (kind === "dependency") {
      planningWorkspace = { ...planningWorkspace, selectedNodeId: id, activeTab: "dependencies" };
    }
    this.state.planningWorkspace = planningWorkspace;
    this.state.activeView = "planning";
    this.render();
  }
}

function unsupportedAction(action: string): never {
  throw new Error(`${action} is not supported by the active gateway transport`);
}

function panel(title: string, body: string, className = ""): string {
  const classes = `os-panel${className ? ` ${className}` : ""}`;
  return `
    <section class="${escapeAttr(classes)}">
      <div class="os-section-head"><h2>${escapeHtml(title)}</h2></div>
      ${body}
    </section>
  `;
}

const shellScrollSelectors = [
  ".os-graph-hero-panel",
  ".os-run-detail-panel",
  ".os-run-evidence-panel",
  ".os-knowledge-lower-panel",
  ".os-code-graph-lower-panel",
  ".os-task-graph-stage",
  '[data-tg-pane-body="done"]',
  '[data-tg-pane-body="current"]',
  '[data-tg-pane-body="backlog"]',
  ".os-run-activity",
] as const;

// Capture by (selector, index): some selectors match more than one element
// (the Knowledge Graph's entity list and capsule inspector both use
// `.os-knowledge-lower-panel`), so keying on the selector alone would only
// ever preserve the first — the capsule inspector's scroll would snap to the
// top on every background render. DOM order is stable across renders, so the
// positional index maps each element back to itself.
function captureShellScrollPositions(root: HTMLElement): Map<string, { left: number; top: number }> {
  const positions = new Map<string, { left: number; top: number }>();
  for (const selector of shellScrollSelectors) {
    root.querySelectorAll<HTMLElement>(selector).forEach((element, index) => {
      positions.set(`${selector}#${index}`, { left: element.scrollLeft, top: element.scrollTop });
    });
  }
  return positions;
}

function restoreShellScrollPositions(
  root: HTMLElement,
  positions: Map<string, { left: number; top: number }>,
): void {
  const elementsBySelector = new Map<string, NodeListOf<HTMLElement>>();
  for (const [key, position] of positions) {
    const separator = key.lastIndexOf("#");
    const selector = key.slice(0, separator);
    const index = Number(key.slice(separator + 1));
    let elements = elementsBySelector.get(selector);
    if (!elements) {
      elements = root.querySelectorAll<HTMLElement>(selector);
      elementsBySelector.set(selector, elements);
    }
    const element = elements[index];
    if (element) {
      element.scrollLeft = position.left;
      element.scrollTop = position.top;
    }
  }
}

function renderPaneResizer(handle: WorkspacePaneResizeHandle, label: string, value: number): string {
  return `
    <div class="os-pane-resizer" role="separator" tabindex="0" aria-orientation="vertical" aria-label="${escapeAttr(label)}" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${Math.round(value)}" data-pane-resizer="${handle}">
      <span aria-hidden="true"></span>
    </div>
  `;
}

/**
 * Nearest scrollable ancestor of `el` (vertical), falling back to the
 * document scrolling element. Used so the lower-row resizer can scroll the
 * page as the row grows, keeping the divider under the cursor.
 */
function scrollContainerFor(el: HTMLElement | null): { get(): number; set(value: number): void } {
  const doc = el?.ownerDocument ?? document;
  const view = doc.defaultView;
  let node: HTMLElement | null = el?.parentElement ?? null;
  while (node) {
    const overflowY = view?.getComputedStyle(node).overflowY;
    if ((overflowY === "auto" || overflowY === "scroll") && node.scrollHeight > node.clientHeight) {
      const target = node;
      return { get: () => target.scrollTop, set: (value) => { target.scrollTop = value; } };
    }
    node = node.parentElement;
  }
  const root = doc.scrollingElement ?? doc.documentElement;
  return { get: () => root.scrollTop, set: (value) => { root.scrollTop = value; } };
}

function renderLowerRowResizer(height: number): string {
  return `
    <div class="os-pane-resizer os-row-resizer" role="separator" tabindex="0" aria-orientation="horizontal" aria-label="Resize lower workspace row" aria-valuemin="${lowerRowHeightBounds.min}" aria-valuemax="${lowerRowHeightBounds.max}" aria-valuenow="${Math.round(height)}" data-row-resizer="lower">
      <span aria-hidden="true"></span>
    </div>
  `;
}

function renderTaskPaneResizer(pane: TaskSidePane, label: string, value: number): string {
  const bounds = taskPaneSizeBounds[pane];
  return `
    <div class="os-pane-resizer os-tg-resizer" role="separator" tabindex="0" aria-orientation="vertical" aria-label="${escapeAttr(label)}" aria-valuemin="${bounds.min}" aria-valuemax="${bounds.max}" aria-valuenow="${Math.round(value)}" data-tg-resizer="${pane}">
      <span aria-hidden="true"></span>
    </div>
  `;
}

function isTaskSidePane(value: string | undefined): value is TaskSidePane {
  return value === "done" || value === "backlog";
}

function panePercent(value: number): string {
  return `${Math.round(value * 100) / 100}%`;
}

function applyWorkspacePaneStyle(shell: HTMLElement, sizes: WorkspacePaneSizes): void {
  shell.style.setProperty("--os-left-column", panePercent(sizes.left));
  shell.style.setProperty("--os-right-column", panePercent(sizes.right));
}

function resizeWorkspacePanes(
  start: WorkspacePaneSizes,
  handle: WorkspacePaneResizeHandle,
  delta: number,
): WorkspacePaneSizes {
  if (handle === "lower-columns") {
    const [left, right] = resizePair(start.left, start.right, delta, minWorkspacePaneSizes.left, minWorkspacePaneSizes.right);
    return { left, right };
  }
  return start;
}

function resizePair(
  left: number,
  right: number,
  delta: number,
  minLeft: number,
  minRight: number,
): [number, number] {
  const total = left + right;
  const nextLeft = clamp(left + delta, minLeft, total - minRight);
  return [nextLeft, total - nextLeft];
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function isWorkspacePaneResizeHandle(value: string | undefined): value is WorkspacePaneResizeHandle {
  return value === "lower-columns";
}

function createDefaultWorkspacePaneSizes(): WorkspacePaneSizesBySurface {
  return {
    task: { ...defaultWorkspacePaneSizes },
    // Entity list left, inspector capsule right: the list only needs to fit
    // titles, the capsule carries the content.
    knowledge: { left: 34, right: 66 },
    code: { ...defaultWorkspacePaneSizes },
  };
}

function isMemoryGraphUpdatedEvent(value: unknown): value is MemoryGraphUpdatedEvent {
  if (!value || typeof value !== "object") return false;
  const event = value as { bundle_id?: unknown; cursor?: unknown; updated_at?: unknown };
  const cursor = event.cursor as { sequence?: unknown; partition?: unknown } | undefined;
  return typeof event.bundle_id === "string"
    && typeof event.updated_at === "string"
    && !!cursor
    && typeof cursor.sequence === "number"
    && typeof cursor.partition === "string";
}

function isCodeGraphUpdatedEvent(value: unknown): value is CodeGraphUpdatedEvent {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<CodeGraphUpdatedEvent>;
  return typeof candidate.repo_id === "string"
    && typeof candidate.updated_at === "string"
    && typeof candidate.cursor?.sequence === "number"
    && typeof candidate.cursor.partition === "string";
}

function sameCodeGraphTopology(a: CodeGraphSnapshot, b: CodeGraphSnapshot): boolean {
  if (a.repo_id !== b.repo_id || a.mode !== b.mode) return false;
  if (a.nodes.length !== b.nodes.length || a.edges.length !== b.edges.length) return false;
  const nodeIdsA = a.nodes.map((node) => `${node.id}:${node.kind}`).sort();
  const nodeIdsB = b.nodes.map((node) => `${node.id}:${node.kind}`).sort();
  const edgeIdsA = a.edges
    .map((edge) => `${edge.id}:${edge.source_id}:${edge.target_id}:${edge.kind}:${edge.confidence}`)
    .sort();
  const edgeIdsB = b.edges
    .map((edge) => `${edge.id}:${edge.source_id}:${edge.target_id}:${edge.kind}:${edge.confidence}`)
    .sort();
  return nodeIdsA.every((id, index) => id === nodeIdsB[index])
    && edgeIdsA.every((id, index) => id === edgeIdsB[index]);
}

function statusEvents(snapshot: DashboardSnapshot | null): DashboardSnapshot["recent_events"] {
  return snapshot?.recent_events.filter((event) => !isTelemetryEventKind(event.kind)) ?? [];
}

function graphSurfaceSummary(view: GraphPaneView): string {
  switch (view) {
    case "task":
      return "Task scheduling and run context";
    case "knowledge":
      return "Memory graph exploration";
    case "code":
      return "Repository symbols, structure, and diff context";
  }
}

function renderRunActivity(
  events: RunEvent[] | null,
  expandedActivityEvents: Set<string>,
  collapsedActivityEvents: Set<string>,
): string {
  if (events === null) {
    return `<div class="os-run-activity os-empty" data-testid="run-activity">Loading conversation activity</div>`;
  }
  if (events.length === 0) {
    return `<div class="os-run-activity os-empty" data-testid="run-activity">No recent activity</div>`;
  }
  const sorted = sortEventsNewestFirst(events);
  const lifecycle = renderActivityLifecycleIndicator(sorted);
  const visibleEvents = sorted.filter(shouldRenderActivityMessage);
  if (visibleEvents.length === 0) {
    return `<div class="os-run-activity" data-testid="run-activity">${lifecycle}<div class="os-empty">No message activity</div></div>`;
  }
  const items = visibleEvents
    .map((event, index) => {
      const eventKey = activityEventKey(event);
      const expanded = expandedActivityEvents.has(eventKey)
        || (index === 0 && !collapsedActivityEvents.has(eventKey));
      return `
      <div class="os-activity-entry os-activity-entry-${escapeAttr(activityClassName(event.kind))}" data-testid="run-activity-entry" data-event-kind="${escapeAttr(event.kind)}" data-event-id="${escapeAttr(event.event_id)}">
        ${renderActivityEvent(event, eventKey, expanded)}
      </div>
    `;
    })
    .join("");
  return `<div class="os-run-activity" data-testid="run-activity">${lifecycle}${items}</div>`;
}

function renderActivityEvent(event: RunEvent, eventKey: string, expanded: boolean): string {
  const body = eventDisplaySummary(event).trim();
  const preview = body ? compactActivityText(body) : "";
  return `
    <div class="os-activity-row">
      <div class="os-activity-meta">
        <span>${escapeHtml(formatEventTime(event.happened_at))}</span>
        <strong>${escapeHtml(event.kind)}</strong>
        ${preview ? `<span class="os-activity-separator">-</span><span class="os-activity-preview" title="${escapeAttr(preview)}">${escapeHtml(preview)}</span>` : ""}
      </div>
      ${body ? `
        <button type="button" class="os-activity-toggle" data-activity-toggle="${escapeAttr(eventKey)}" aria-expanded="${expanded ? "true" : "false"}" aria-label="${expanded ? "Collapse" : "Expand"} ${escapeAttr(event.kind)} event">
          <span aria-hidden="true">${expanded ? "v" : ">"}</span>
        </button>
      ` : ""}
    </div>
    ${body && expanded ? `<pre class="os-activity-detail">${escapeHtml(body)}</pre>` : ""}
  `;
}

function renderActivityLifecycleIndicator(events: RunEvent[]): string {
  const lifecycleEvents = events
    .map((event) => ({ event, state: activityLifecycleState(event) }))
    .filter((entry): entry is { event: RunEvent; state: "started" | "completed" } => Boolean(entry.state));
  if (lifecycleEvents.length === 0) {
    return "";
  }
  const latest = lifecycleEvents[0]!;
  const started = lifecycleEvents.filter((entry) => entry.state === "started").length;
  const completed = lifecycleEvents.filter((entry) => entry.state === "completed").length;
  const latestLabel = latest.state === "started" ? "Working" : "Settled";
  return `
    <div class="os-activity-lifecycle" data-testid="activity-lifecycle">
      <span>${escapeHtml(formatEventTime(latest.event.happened_at))}</span>
      <strong>${latestLabel}</strong>
      <em>${started} started, ${completed} completed</em>
    </div>
  `;
}

function shouldRenderActivityMessage(event: RunEvent): boolean {
  if (activityLifecycleState(event) !== null) return false;
  return !isTelemetryEventKind(event.kind);
}

function isTelemetryEventKind(kind: string): boolean {
  return kind === "codex.thread/tokenUsage/updated" || kind === "codex.turn/diff/updated";
}

function activityLifecycleState(event: RunEvent): "started" | "completed" | null {
  const normalized = `${event.kind} ${event.summary}`
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim();
  if (/\bitem started\b|\bturn started\b/.test(normalized)) {
    return "started";
  }
  if (/\bitem completed\b|\bturn completed\b/.test(normalized)) {
    return "completed";
  }
  return null;
}

function sortEventsNewestFirst(events: RunEvent[]): RunEvent[] {
  return [...events].sort((a, b) => {
    const timeDiff = eventTimeValue(b) - eventTimeValue(a);
    if (timeDiff !== 0) return timeDiff;
    const sequenceDiff = b.sequence - a.sequence;
    if (sequenceDiff !== 0) return sequenceDiff;
    return b.event_id.localeCompare(a.event_id);
  });
}

function eventTimeValue(event: RunEvent): number {
  const parsed = Date.parse(event.happened_at);
  return Number.isNaN(parsed) ? 0 : parsed;
}

function activityEventKey(event: RunEvent): string {
  return event.event_id || `${event.sequence}:${event.happened_at}:${event.kind}`;
}

function compactActivityText(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

function eventDisplaySummary(event: RunEvent): string {
  const payloadText = eventPayloadText(event);
  if (payloadText && isGenericEventSummary(event)) {
    return payloadText;
  }
  if (isGenericEventSummary(event)) {
    return "";
  }
  if (event.kind === "ActionEvent" && payloadText && payloadText !== event.summary) {
    return `${event.summary}: ${payloadText}`;
  }
  return event.summary;
}

function isGenericEventSummary(event: RunEvent): boolean {
  const summary = event.summary.trim().toLowerCase();
  return summary === ""
    || summary === event.kind.toLowerCase()
    || summary === "action"
    || summary === "tool call"
    || summary === "tool result";
}

function eventPayloadText(event: RunEvent): string | null {
  const payloads = [event.payload, event.raw_payload];
  for (const payload of payloads) {
    const text = actionPayloadText(payload) ?? observationPayloadText(payload);
    if (text) {
      return text;
    }
  }
  return null;
}

function actionPayloadText(value: unknown): string | null {
  const record = objectRecord(value);
  if (!record) {
    return null;
  }
  const action = objectRecord(record.action);
  const argumentsRecord = objectRecord(record.arguments);
  const summary = stringField(record, "summary");
  const tool = stringField(record, "tool_name") ?? stringField(action, "tool_name");
  const detail = stringField(record, "message")
    ?? stringField(action, "message")
    ?? stringField(record, "command")
    ?? stringField(action, "command")
    ?? stringField(argumentsRecord, "command")
    ?? stringField(record, "thought")
    ?? stringField(action, "thought");

  if (summary && detail && summary !== detail) {
    return `${summary}: ${detail}`;
  }
  if (detail && tool && detail !== tool) {
    return `${tool}: ${detail}`;
  }
  return detail ?? summary ?? tool ?? null;
}

function observationPayloadText(value: unknown): string | null {
  const record = objectRecord(value);
  if (!record) {
    return null;
  }
  const observation = objectRecord(record.observation);
  const content = stringField(record, "content") ?? stringField(observation, "content");
  const preview = stringField(record, "preview") ?? stringField(record, "summary");
  const tool = stringField(record, "tool_name") ?? stringField(observation, "tool_name");
  return content ?? preview ?? tool ?? null;
}

function objectRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function stringField(record: Record<string, unknown> | null, field: string): string | null {
  const value = record?.[field];
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function activityClassName(kind: string): string {
  return kind
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
    .replace(/[^a-zA-Z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .toLowerCase() || "event";
}

function formatEventTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function option(value: string, label: string, selectedValue: string | null | undefined): string {
  const selected = value === selectedValue ? "selected" : "";
  return `<option value="${escapeAttr(value)}" ${selected}>${escapeHtml(label)}</option>`;
}

function activeModelProfile(
  profiles: ModelConfigurationProfile[],
  profileId: string | null,
): ModelConfigurationProfile | null {
  return profiles.find((profile) => profile.id === profileId)
    ?? profiles.find((profile) => profile.active)
    ?? null;
}

function modelProfilesWithDefaults(
  profiles: ModelConfigurationProfile[],
): ModelConfigurationProfile[] {
  return profiles.length > 0 ? profiles : defaultModelProfiles();
}

function modelCredentialSummary(profile: ModelConfigurationProfile): string {
  if (profile.mode === "api_key") {
    return profile.apiKeyRef?.trim() ? "API key configured" : "API key not configured";
  }
  const authDirectoryEnv = profile.subscriptionCredential?.authDirectoryEnv?.trim();
  const codexReady = profile.harnesses.includes("codex_app_server")
    ? "Codex CLI login via gateway readiness"
    : null;
  const openhandsReady = authDirectoryEnv
    ? `OpenHands auth dir env ${authDirectoryEnv}`
    : "OpenHands auth dir env not configured";

  return codexReady ? `${codexReady}; ${openhandsReady}` : openhandsReady;
}

function modelCredentialLabel(profile: ModelConfigurationProfile): string {
  if (profile.mode === "api_key") {
    return "API Key Secret";
  }
  return profile.harnesses.includes("codex_app_server")
    ? "OpenHands Auth Directory Env (OpenHands only)"
    : "OpenHands Auth Directory Env";
}

function upsertModelProfile(
  profiles: ModelConfigurationProfile[],
  profile: ModelConfigurationProfile,
): ModelConfigurationProfile[] {
  const index = profiles.findIndex((candidate) => candidate.id === profile.id);
  if (index < 0) {
    return [...profiles, profile];
  }
  const next = [...profiles];
  next[index] = profile;
  return next;
}

function modelModeFromValue(value: string): ModelCredentialMode {
  return value === "subscription" ? "subscription" : "api_key";
}

function modelOwnerFromValue(value: string): ModelConfigurationProfile["owner"] {
  switch (value) {
    case "organization":
    case "project":
      return value;
    case "user":
    default:
      return "user";
  }
}

function splitList(value: string): string[] {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter((item) => item.length > 0);
}

const editableProfileKindOptions: Array<{
  value: ConnectionProfile["kind"];
  label: string;
}> = [
  { value: "local_daemon", label: "Local daemon" },
  { value: "external_gateway", label: "External gateway" },
  { value: "hosted_gateway", label: "Hosted gateway" },
];

function defaultProfileKindForMode(
  mode: OpenSymphonyAppOptions["mode"],
): ConnectionProfile["kind"] {
  return mode === "desktop" ? "local_daemon" : "external_gateway";
}

function editableProfileKindFromValue(
  value: string | undefined,
  mode: OpenSymphonyAppOptions["mode"],
): ConnectionProfile["kind"] {
  switch (value) {
    case "local_daemon":
    case "external_gateway":
    case "hosted_gateway":
      return value;
    default:
      return defaultProfileKindForMode(mode);
  }
}

function defaultUiProfiles(gatewayUrl: string): ConnectionProfile[] {
  return [
    {
      id: "local-daemon",
      label: "Local Daemon",
      kind: "local_daemon",
      active: true,
      gatewayUrl: gatewayUrl || "http://127.0.0.1:2468",
      transport: "loopback_http",
      managed: false,
    },
  ];
}

function statusToPhase(
  status: RunDetail["status"],
  releaseReason?: RunDetail["release_reason"],
  detached?: boolean,
): RunPhase {
  if (detached) {
    return "detached";
  }
  if (status === "retry_queued") {
    return "retry_queued";
  }
  if (status === "released") {
    if (releaseReason === "completed") return "completed";
    if (releaseReason === "cancel_failed") return "cancelled";
    return "cancelled";
  }
  return status === "running" || status === "claimed" ? "active" : "quiet";
}

function statusLabel(mode: ConnectionMode): string {
  switch (mode) {
    case "connected":
      return "Connected";
    case "failed":
      return "Failed";
    case "connecting":
      return "Connecting";
  }
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "object" && error !== null) {
    const record = error as Record<string, unknown>;
    if (typeof record.message === "string" && record.message.trim()) {
      return record.message;
    }
    if (typeof record.error === "string" && record.error.trim()) {
      return record.error;
    }
    try {
      return JSON.stringify(error);
    } catch {
      return String(error);
    }
  }
  return String(error);
}

function escapeHtml(value: unknown): string {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function escapeAttr(value: unknown): string {
  return escapeHtml(value).replace(/"/g, "&quot;");
}

function cssEscape(value: string): string {
  return globalThis.CSS?.escape?.(value) ?? value.replace(/["\\]/g, "\\$&");
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat("en-US", { notation: "compact" }).format(value);
}

function formatPrLinkLabel(url: string): string {
  try {
    const parsed = new URL(url);
    const parts = parsed.pathname.split("/").filter(Boolean);
    const pullIndex = parts.findIndex((part) => part === "pull");
    if (
      parsed.hostname === "github.com"
      && pullIndex === 2
      && parts.length > pullIndex + 1
    ) {
      return `${parts[0]}/${parts[1]}#${parts[pullIndex + 1]}`;
    }
  } catch {
    // Fall through to the original URL for non-URL values.
  }
  return url;
}

function hasValidationSummary(summary: RunValidationSummary | null): summary is RunValidationSummary {
  return Boolean(summary && (summary.commands.length > 0 || summary.evidence.length > 0));
}

interface DependencySignal {
  gutter: string;
  suffix: string;
  upstreamVisible: TaskGraphNode[];
  upstreamHiddenCount: number;
  downstreamVisible: TaskGraphNode[];
  downstreamHiddenCount: number;
  completedBlockers: TaskGraphNode[];
}

interface TaskGraphRenderModel {
  node: TaskGraphNode;
  signal: DependencySignal;
  lane: number;
  index: number;
}

interface TaskGraphLink {
  from: TaskGraphRenderModel;
  to: TaskGraphRenderModel;
  routeLane: number;
  span: number;
  hue: number;
}

const taskGraphRowHeight = 44;
const taskGraphRowGap = 8;
/** Radius of the connector circle (.os-node-gutter, 22px) centred on each row. */
const taskGraphNodeRadius = 11;
/**
 * Skip-level dependency arrows route through a dedicated left gutter, one
 * lane and one hue per blocker, so several long-range dependencies stay
 * readable instead of conflating into a single L-shaped rail. The gutter
 * widens with the number of distinct blockers so lanes are never reused
 * (hues cycle, lanes do not).
 */
const taskGraphMinRouteGutterWidth = 62;
const taskGraphSkipLaneGap = 11;
/** Arrow rail x-origin inside the routing gutter (see renderTaskGraphLink). */
const taskGraphRailBaseOffset = 21;
/** Slack past the deepest lane for the curve bend and arrowhead. */
const taskGraphArrowPad = 48;
const taskGraphLinkHues = ["#39708f", "#7c3aed", "#0f766e", "#b0762f", "#a05577"] as const;

function taskGraphRouteGutterWidth(links: readonly TaskGraphLink[]): number {
  const laneCount = links.reduce((max, link) => Math.max(max, link.routeLane + 1), 0);
  return Math.max(taskGraphMinRouteGutterWidth, 30 + laneCount * taskGraphSkipLaneGap);
}
const taskGraphLaneWidth = 34;

function buildDependencySignals(
  allNodes: TaskGraphNode[],
  visibleNodes: TaskGraphNode[],
): Map<string, DependencySignal> {
  const visibleIds = new Set(visibleNodes.map((node) => node.node_id));
  const downstream = new Map<string, TaskGraphNode[]>();

  for (const node of allNodes) {
    for (const blockerId of node.blocked_by) {
      const blocker = findNodeByRef(allNodes, blockerId);
      const downstreamKey = blocker?.node_id ?? normalizeNodeRef(blockerId);
      const entries = downstream.get(downstreamKey) ?? [];
      entries.push(node);
      downstream.set(downstreamKey, entries);
    }
  }

  const signals = new Map<string, DependencySignal>();
  for (const node of allNodes) {
    const knownBlockers = node.blocked_by.map((id) => findNodeByRef(allNodes, id)).filter((candidate): candidate is TaskGraphNode => Boolean(candidate));
    const unknownBlockerCount = node.blocked_by.length - knownBlockers.length;
    const unfinishedBlockers = knownBlockers.filter((candidate) => !isTerminalTaskNode(candidate));
    const upstreamVisible = unfinishedBlockers.filter((candidate) => visibleIds.has(candidate.node_id));
    const upstreamHiddenCount = unfinishedBlockers.length - upstreamVisible.length + unknownBlockerCount;
    const completedBlockers = knownBlockers.filter(isTerminalTaskNode);

    const downstreamNodes = downstream.get(node.node_id) ?? [];
    const unfinishedDownstream = downstreamNodes.filter((candidate) => !isTerminalTaskNode(candidate));
    const downstreamVisible = unfinishedDownstream.filter((candidate) => visibleIds.has(candidate.node_id));
    const downstreamHiddenCount = unfinishedDownstream.length - downstreamVisible.length;
    const suffix = dependencySuffix(upstreamVisible, upstreamHiddenCount, downstreamVisible, downstreamHiddenCount);
    const gutter = upstreamVisible.length > 0
      ? "|  "
      : downstreamVisible.length > 0 || downstreamHiddenCount > 0
        ? "+--"
        : "   ";

    signals.set(node.node_id, {
      gutter,
      suffix,
      upstreamVisible,
      upstreamHiddenCount,
      downstreamVisible,
      downstreamHiddenCount,
      completedBlockers,
    });
  }
  return signals;
}

function applyGraphRuntimeOverlay(
  node: TaskGraphNode,
  signal: DependencySignal | undefined,
  overlay: ReturnType<typeof buildRuntimeOverlay>,
): ReturnType<typeof buildRuntimeOverlay> {
  if (!signal) {
    return overlay;
  }
  const badges: ReturnType<typeof buildRuntimeOverlay>["badges"] = overlay.badges.filter((badge) => badge !== "blocker");
  if (isActivelyBlocking(node, signal)) {
    badges.push("blocker");
  }
  return {
    ...overlay,
    is_blocked: hasUnresolvedUpstream(signal),
    blocked_by_count: signal.upstreamVisible.length + signal.upstreamHiddenCount,
    badges: [...new Set(badges)],
  };
}

function isActivelyBlocking(node: TaskGraphNode, signal: DependencySignal): boolean {
  return node.kind !== "milestone"
    && isDispatchableTaskNode(node)
    && !hasUnresolvedUpstream(signal)
    && (signal.downstreamVisible.length > 0 || signal.downstreamHiddenCount > 0);
}

function hasUnresolvedUpstream(signal: DependencySignal): boolean {
  return signal.upstreamVisible.length > 0 || signal.upstreamHiddenCount > 0;
}

function isDispatchableTaskNode(node: TaskGraphNode): boolean {
  if (isTerminalTaskNode(node)) {
    return false;
  }
  if (node.state_category === "backlog" || node.state_category === "canceled") {
    return false;
  }
  if (node.state_category === "todo" || node.state_category === "in_progress") {
    return true;
  }
  const state = node.state.toLowerCase();
  return state.includes("todo")
    || state.includes("progress")
    || state.includes("human review")
    || state.includes("rework");
}

function renderTaskGraphVisualization(
  nodes: TaskGraphNode[],
  selectedNodeId: string | null,
  getOverlay: (node: TaskGraphNode) => ReturnType<typeof buildRuntimeOverlay>,
  signals: Map<string, DependencySignal>,
  collapsedProjectGroups = new Set<string>(),
  groupByProject = false,
  variant: "current" | "backlog" = "current",
): string {
  if (nodes.length === 0) {
    return `<div class="os-empty">No tasks match the current filters</div>`;
  }
  const projectGroups = groupByProject ? buildProjectGroups(nodes, signals) : [];
  if (projectGroups.length > 0) {
    return projectGroups.map((group) => {
      const collapsed = collapsedProjectGroups.has(group.key);
      const body = collapsed
        ? ""
        : renderTaskGraphVisualization(group.nodes, selectedNodeId, getOverlay, signals, collapsedProjectGroups, false, variant);
      const title = projectGroupTitle(group);
      return `
        <section class="os-project-group" id="${escapeAttr(projectGroupDomId(group.key))}" role="region" aria-label="${escapeAttr(title)}" data-project-group="${escapeAttr(group.key)}">
          ${renderProjectGroupHeader(group, collapsed)}
          ${body}
        </section>
      `;
    }).join("");
  }
  const models = buildTaskGraphRenderModels(nodes, signals);
  const links = buildTaskGraphLinks(models);
  const graphHeight = models.length * taskGraphRowHeight + Math.max(0, models.length - 1) * taskGraphRowGap;
  const maxLane = models.reduce((max, model) => Math.max(max, model.lane), 0);
  const gutterWidth = taskGraphRouteGutterWidth(links);
  // Only the arrow coordinate space needs a fixed width: every in-pane edge
  // is anchored to the left routing gutter and lane rails (see
  // renderTaskGraphLink), so the deepest lane plus a small pad for the bend
  // and arrowhead is all the SVG must span. Node bodies are fluid — they grow
  // to fill wider panes and shrink to fit narrow ones (the node list caps at
  // 100% of the pane), so this width no longer reserves body space and the
  // narrow side panes stop showing a horizontal scrollbar when nothing
  // actually overflows.
  const graphWidth = taskGraphRailBaseOffset + gutterWidth + maxLane * taskGraphLaneWidth + taskGraphArrowPad;
  const svgLinks = links.map((link) => renderTaskGraphLink(link, gutterWidth)).join("");
  const renderedNodes = models.map((model) => renderReadOnlyTaskGraphNode(
    model,
    selectedNodeId,
    getOverlay(model.node),
  )).join("");
  const stageClass = variant === "backlog" ? "os-task-graph-stage os-tg-stage-backlog" : "os-task-graph-stage";
  const stageTestId = variant === "backlog" ? "task-graph-backlog" : "task-graph-visualization";

  return `
    <div class="${stageClass}" data-testid="${stageTestId}" style="--os-graph-height: ${graphHeight}px; --os-graph-width: ${graphWidth}px; --os-tg-gutter: ${gutterWidth}px;">
      <svg class="os-task-graph-links" data-testid="task-graph-links" viewBox="0 0 ${graphWidth} ${graphHeight}" preserveAspectRatio="none" aria-hidden="true">
        <defs>
          <marker id="os-task-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto">
            <path d="M 0 0 L 8 4 L 0 8 z"></path>
          </marker>
          ${taskGraphLinkHues.map((hue, index) => `
          <marker id="os-task-arrow-${index}" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto">
            <path d="M 0 0 L 8 4 L 0 8 z" fill="${escapeAttr(hue)}"></path>
          </marker>`).join("")}
        </defs>
        ${svgLinks}
      </svg>
      <div class="os-node-list os-node-graph-list" style="min-height: ${graphHeight}px;">${renderedNodes}</div>
    </div>
  `;
}

/** Header shared by the collapsible Completed and Backlog panes. */
function renderTaskPaneHeader(pane: "done" | "backlog", label: string, count: number | null): string {
  const countChip = count !== null
    ? `<span class="os-tg-count os-tg-count-${pane}">${count} ${pane === "done" ? "done" : "backlog"}</span>`
    : "";
  return `
    <header class="os-tg-pane-head">
      <span class="os-tg-dot os-tg-dot-${pane}" aria-hidden="true"></span>
      <strong>${escapeHtml(label)}</strong>
      ${countChip}
      <button
        type="button"
        class="os-tg-pane-toggle"
        data-tg-pane-toggle="${pane}"
        aria-expanded="true"
        aria-label="Collapse ${escapeAttr(label)} pane"
        title="Collapse ${escapeAttr(label)} pane"
      >${pane === "done" ? "‹" : "›"}</button>
    </header>
  `;
}

/** Narrow vertical strip shown while a side pane is collapsed. */
function renderCollapsedTaskPane(pane: "done" | "backlog", label: string, count: number | null): string {
  return `
    <section class="os-tg-pane os-tg-pane-collapsed" data-tg-pane="${pane}" data-collapsed data-testid="task-pane-${pane}">
      <button
        type="button"
        class="os-tg-pane-toggle"
        data-tg-pane-toggle="${pane}"
        aria-expanded="false"
        aria-label="Expand ${escapeAttr(label)} pane"
        title="Expand ${escapeAttr(label)} pane"
      >${pane === "done" ? "›" : "‹"}</button>
      <span class="os-tg-dot os-tg-dot-${pane}" aria-hidden="true"></span>
      <span class="os-tg-pane-vertical-label">${escapeHtml(label)}</span>
      ${count !== null ? `<span class="os-tg-count os-tg-count-${pane}">${count}</span>` : ""}
    </section>
  `;
}

const completedSortColumns: Record<string, { asc: string; desc: string; first: string }> = {
  id: { asc: "id_asc", desc: "id_desc", first: "id_asc" },
  title: { asc: "title_asc", desc: "title_desc", first: "title_asc" },
  pr: { asc: "pr_asc", desc: "pr_desc", first: "pr_desc" },
  completed: { asc: "completed_asc", desc: "completed_desc", first: "completed_desc" },
};

function renderCompletedSortHeader(column: string, label: string, activeSort: string): string {
  const sorts = completedSortColumns[column];
  const direction = activeSort === sorts.asc ? "ascending" : activeSort === sorts.desc ? "descending" : null;
  const arrow = direction === "ascending" ? " ↑" : direction === "descending" ? " ↓" : " ↕";
  return `
    <th scope="col" ${direction ? `aria-sort="${direction}"` : ""}>
      <button type="button" class="os-tg-done-sort ${direction ? "is-active" : ""}" data-tg-done-sort="${escapeAttr(column)}">${escapeHtml(label)}<span aria-hidden="true">${arrow}</span></button>
    </th>
  `;
}

function renderCompletedTaskPrs(prs: MemoryTaskPullRequest[]): string {
  if (prs.length === 0) {
    return `<span class="os-tg-capsule-missing">—</span>`;
  }
  // Newest first by PR number — matching the gateway/fixture ordering — so
  // the bold "latest" chip is genuinely the newest PR even when it is a
  // later abandoned/unmerged attempt after an older merged one. Unmerged
  // PRs are struck through.
  const ordered = [...prs].sort((a, b) => b.number - a.number);
  const latest = ordered[0];
  return ordered.map((pr) => {
    const classes = [
      "os-tg-pr",
      pr === latest ? "os-tg-pr-latest" : "",
      pr.merged ? "" : "os-tg-pr-unmerged",
    ].filter(Boolean).join(" ");
    const title = `${pr.title || `PR #${pr.number}`}${pr.merged ? "" : " (not merged)"}`;
    return pr.url
      ? `<a class="${classes}" href="${escapeAttr(pr.url)}" target="_blank" rel="noreferrer noopener" title="${escapeAttr(title)}">#${pr.number}</a>`
      : `<span class="${classes}" title="${escapeAttr(title)}">#${pr.number}</span>`;
  }).join(" ");
}

function renderCompletedTasksPagination(page: MemoryCompletedTaskPage): string {
  const pageCount = Math.max(1, Math.ceil(page.total / Math.max(1, page.limit)));
  const currentPage = Math.floor(page.offset / Math.max(1, page.limit)) + 1;
  if (pageCount <= 1) {
    return `<div class="os-tg-done-pagination" data-testid="completed-tasks-pagination"><span class="os-tg-done-page-size">${page.total} task${page.total === 1 ? "" : "s"}</span></div>`;
  }
  // Window of at most 7 numbered buttons centered on the current page.
  const windowStart = Math.max(1, Math.min(currentPage - 3, pageCount - 6));
  const windowEnd = Math.min(pageCount, windowStart + 6);
  const numbers: string[] = [];
  for (let pageNumber = windowStart; pageNumber <= windowEnd; pageNumber += 1) {
    numbers.push(`
      <button
        type="button"
        class="os-tg-done-page ${pageNumber === currentPage ? "is-active" : ""}"
        data-tg-done-page="${pageNumber}"
        ${pageNumber === currentPage ? `aria-current="page"` : ""}
      >${pageNumber}</button>
    `);
  }
  return `
    <div class="os-tg-done-pagination" data-testid="completed-tasks-pagination">
      <button type="button" class="os-tg-done-page" data-tg-done-page="prev" ${currentPage <= 1 ? "disabled" : ""} aria-label="Previous page">‹</button>
      ${numbers.join("")}
      <button type="button" class="os-tg-done-page" data-tg-done-page="next" ${currentPage >= pageCount ? "disabled" : ""} aria-label="Next page">›</button>
      <span class="os-tg-done-page-size">${page.limit} / page</span>
    </div>
  `;
}

function formatCompletedDate(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return iso;
  }
  return date.toLocaleDateString(undefined, { month: "short", day: "2-digit" });
}

/**
 * Cross-pane dependency edges (Current → Backlog). Paths carry the same
 * data-link-from/to contract as in-pane edges but start with empty geometry:
 * positionTaskGraphCrossLinks measures the live card positions after mount
 * and on every scroll/resize/collapse.
 */
function renderTaskGraphCrossLinks(
  allNodes: TaskGraphNode[],
  currentNodes: TaskGraphNode[],
  backlogNodes: TaskGraphNode[],
): string {
  const currentIds = new Set(currentNodes.map((node) => node.node_id));
  const pairs: Array<{ from: string; to: string }> = [];
  for (const backlogNode of backlogNodes) {
    for (const blockerRef of backlogNode.blocked_by) {
      const blocker = findNodeByRef(allNodes, blockerRef);
      if (blocker && currentIds.has(blocker.node_id)) {
        pairs.push({ from: blocker.node_id, to: backlogNode.node_id });
      }
    }
  }
  if (pairs.length === 0) {
    return "";
  }
  const hueBySource = new Map<string, number>();
  for (const pair of pairs) {
    if (!hueBySource.has(pair.from)) {
      hueBySource.set(pair.from, hueBySource.size % taskGraphLinkHues.length);
    }
  }
  const paths = pairs.map((pair) => {
    const hue = hueBySource.get(pair.from) ?? 0;
    return `<path class="os-tg-cross-link os-tg-hue-${hue}" data-testid="task-graph-cross-link" data-link-from="${escapeAttr(pair.from)}" data-link-to="${escapeAttr(pair.to)}" d="" marker-end="url(#os-tg-cross-arrow-${hue})"></path>`;
  }).join("");
  return `
    <svg class="os-tg-cross-links" data-tg-cross-links aria-hidden="true">
      <defs>
        ${taskGraphLinkHues.map((hue, index) => `
        <marker id="os-tg-cross-arrow-${index}" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto">
          <path d="M 0 0 L 8 4 L 0 8 z" fill="${escapeAttr(hue)}"></path>
        </marker>`).join("")}
      </defs>
      ${paths}
    </svg>
  `;
}

/**
 * Edges on the "ancestry critical path" of a backlog task: every
 * non-terminal blocked_by edge reachable walking upstream from the task.
 * These are the tasks that must complete to unblock it.
 */
function collectAncestryEdges(
  nodes: TaskGraphNode[],
  target: TaskGraphNode,
): { edges: Set<string>; members: Set<string> } {
  const edges = new Set<string>();
  const members = new Set<string>([target.node_id]);
  const queue = [target];
  const visited = new Set<string>([target.node_id]);
  while (queue.length > 0) {
    const node = queue.shift()!;
    for (const blockerRef of node.blocked_by) {
      const blocker = findNodeByRef(nodes, blockerRef);
      if (!blocker || isTerminalTaskNode(blocker)) {
        continue;
      }
      edges.add(`${blocker.node_id}->${node.node_id}`);
      members.add(blocker.node_id);
      if (!visited.has(blocker.node_id)) {
        visited.add(blocker.node_id);
        queue.push(blocker);
      }
    }
  }
  return { edges, members };
}

interface ProjectGroup {
  key: string;
  slug: string;
  name: string;
  nodes: TaskGraphNode[];
  issueCount: number;
  runningCount: number;
  todoCount: number;
  blockedCount: number;
}

const unassignedProjectGroupKey = "__opensymphony_unassigned__";

function buildProjectGroups(
  nodes: TaskGraphNode[],
  signals: Map<string, DependencySignal>,
): ProjectGroup[] {
  if (!nodes.some(hasProjectMetadata)) {
    return [];
  }
  const groups = new Map<string, ProjectGroup>();
  for (const node of nodes) {
    const key = node.project_slug ?? node.project_id ?? node.project_name ?? unassignedProjectGroupKey;
    const group = groups.get(key) ?? {
      key,
      slug: node.project_slug ?? node.project_id ?? node.project_name ?? "unassigned",
      name: node.project_name ?? (key === unassignedProjectGroupKey ? "Unassigned" : ""),
      nodes: [],
      issueCount: 0,
      runningCount: 0,
      todoCount: 0,
      blockedCount: 0,
    };
    group.nodes.push(node);
    if (!group.name && key !== unassignedProjectGroupKey && node.project_name) {
      group.name = node.project_name;
    }
    if (node.kind !== "milestone") {
      group.issueCount += 1;
      if (node.state_category === "in_progress") group.runningCount += 1;
      if (node.state_category === "todo") group.todoCount += 1;
      const signal = signals.get(node.node_id);
      if (signal && hasUnresolvedUpstream(signal)) group.blockedCount += 1;
    }
    groups.set(key, group);
  }
  return Array.from(groups.values()).sort((left, right) =>
    left.slug.localeCompare(right.slug, undefined, { sensitivity: "base" })
      || left.name.localeCompare(right.name, undefined, { sensitivity: "base" }),
  );
}

function hasProjectMetadata(node: TaskGraphNode): boolean {
  return Boolean(node.project_id || node.project_slug || node.project_name);
}

function renderProjectGroupHeader(group: ProjectGroup, collapsed: boolean): string {
  const controlId = projectGroupDomId(group.key);
  const counts = [
    `issues=${group.issueCount}`,
    `running=${group.runningCount}`,
    `todo=${group.todoCount}`,
    `blocked=${group.blockedCount}`,
  ].join(" ");
  const title = projectGroupTitle(group);
  return `
    <button type="button" class="os-project-group-header" data-project-group-toggle="${escapeAttr(group.key)}" aria-expanded="${collapsed ? "false" : "true"}" aria-controls="${escapeAttr(controlId)}">
      <span aria-hidden="true">${collapsed ? "+" : "-"}</span>
      <strong>${escapeHtml(title)}</strong>
      <em>${escapeHtml(counts)}</em>
    </button>
  `;
}

function projectGroupDomId(key: string): string {
  return `os-project-group-${encodeURIComponent(key)}`;
}

function projectGroupTitle(group: ProjectGroup): string {
  return group.name && group.name.toLowerCase() !== group.slug.toLowerCase()
    ? `${group.slug} | ${group.name}`
    : group.slug;
}

function buildTaskGraphRenderModels(
  nodes: TaskGraphNode[],
  signals: Map<string, DependencySignal>,
): TaskGraphRenderModel[] {
  const laneById = new Map<string, number>();
  const visibleIds = new Set(nodes.map((node) => node.node_id));

  for (const node of nodes) {
    const signal = signals.get(node.node_id);
    const upstreamLanes = signal?.upstreamVisible
      .filter((upstream) => visibleIds.has(upstream.node_id))
      .map((upstream) => laneById.get(upstream.node_id) ?? 0) ?? [];
    const lane = upstreamLanes.length > 0 ? Math.min(4, Math.max(...upstreamLanes) + 1) : 0;
    laneById.set(node.node_id, lane);
  }

  return nodes.map((node, index) => ({
    node,
    signal: signals.get(node.node_id) ?? emptyDependencySignal(),
    lane: laneById.get(node.node_id) ?? 0,
    index,
  }));
}

function buildTaskGraphLinks(models: TaskGraphRenderModel[]): TaskGraphLink[] {
  const byId = new Map(models.map((model) => [model.node.node_id, model]));
  const links: Array<Omit<TaskGraphLink, "routeLane" | "hue">> = [];
  for (const model of models) {
    for (const upstream of model.signal.upstreamVisible) {
      const from = byId.get(upstream.node_id);
      if (from) {
        links.push({ from, to: model, span: Math.abs(model.index - from.index) });
      }
    }
  }
  // Every skip-level source gets its own gutter lane and hue, shared by all
  // of that blocker's arrows: same-source fans stay visually grouped while
  // different blockers never share a rail.
  //
  // Rails are ordered by span, not row position: the shortest hops take the
  // inner rails (hugging the node cards) and only genuinely long-range arcs
  // sweep out to the wide rails. Ordering by row instead pushed every deeper
  // blocker to an outer rail regardless of how close its dependency was, so a
  // locally-blocked but deeply-nested task's arrow drifted further left the
  // deeper it sat — the opposite of what the indentation implies.
  const maxSpanBySource = new Map<string, number>();
  const firstRowBySource = new Map<string, number>();
  for (const link of links) {
    if (link.span <= 1) {
      continue;
    }
    const sourceId = link.from.node.node_id;
    maxSpanBySource.set(sourceId, Math.max(maxSpanBySource.get(sourceId) ?? 0, link.span));
    firstRowBySource.set(sourceId, Math.min(firstRowBySource.get(sourceId) ?? Infinity, link.from.index));
  }
  const skipSources = [...maxSpanBySource.keys()].sort((a, b) => {
    const spanDelta = (maxSpanBySource.get(a) ?? 0) - (maxSpanBySource.get(b) ?? 0);
    // Shorter span → inner rail; ties fall back to row order for determinism.
    return spanDelta !== 0 ? spanDelta : (firstRowBySource.get(a) ?? 0) - (firstRowBySource.get(b) ?? 0);
  });
  // Lanes are never reused: every distinct blocker gets its own rail and
  // the gutter widens to fit (taskGraphRouteGutterWidth). Hues cycle.
  const laneBySource = new Map(skipSources.map((sourceId, index) => [sourceId, index]));
  return links.map((link) => {
    const lane = link.span > 1 ? laneBySource.get(link.from.node.node_id) ?? 0 : 0;
    return { ...link, routeLane: lane, hue: lane % taskGraphLinkHues.length };
  });
}

function renderTaskGraphLink(
  link: TaskGraphLink,
  gutterWidth: number,
): string {
  // Node rows sit right of the routing gutter (see .os-node-graph-list padding).
  const railX = 21 + gutterWidth;
  const x1 = railX + link.from.lane * taskGraphLaneWidth;
  const x2 = railX + link.to.lane * taskGraphLaneWidth;
  const y1 = link.from.index * (taskGraphRowHeight + taskGraphRowGap) + taskGraphRowHeight / 2;
  const y2 = link.to.index * (taskGraphRowHeight + taskGraphRowGap) + taskGraphRowHeight / 2;
  const linkAttrs = `data-testid="task-graph-link" data-link-from="${escapeAttr(link.from.node.node_id)}" data-link-to="${escapeAttr(link.to.node.node_id)}"`;
  const r = taskGraphNodeRadius;
  if (link.span > 1) {
    // Skip-level arrows sweep out through the left gutter and arrive
    // horizontally at the LEFT edge of the target circle (pointing right), so
    // several long-range blockers on one task stay individually legible.
    const routeX = Math.max(4, railX - 16 - link.routeLane * taskGraphSkipLaneGap);
    const turn = Math.min(14, Math.abs(y2 - y1) / 2, Math.max(4, x1 - routeX));
    const direction = y2 > y1 ? 1 : -1;
    const endX = x2 - r;
    const d = [
      `M ${x1} ${y1}`,
      `H ${routeX + turn}`,
      `Q ${routeX} ${y1} ${routeX} ${y1 + turn * direction}`,
      `V ${y2 - turn * direction}`,
      `Q ${routeX} ${y2} ${routeX + turn} ${y2}`,
      `H ${endX}`,
    ].join(" ");
    // The hue-specific marker is applied via CSS (see .os-tg-hue-* rules):
    // a `marker-end` presentation attribute here would lose to the base
    // `.os-task-graph-link` CSS rule, leaving every skip arrowhead default.
    return `<path class="os-task-graph-link os-task-graph-link-skip os-tg-hue-${link.hue}" ${linkAttrs} d="${escapeAttr(d)}"></path>`;
  }
  // Next-level arrows arrive vertically at the TOP of the target circle
  // (pointing down, orient=auto follows the tangent). Same lane → a straight
  // drop; one lane deeper → an S-curve that shifts right while still landing
  // vertically, which distinguishes it from the sideways skip-level arrows.
  const dir = y2 >= y1 ? 1 : -1;
  const endY = y2 - r * dir;
  const bend = Math.min(20, Math.max(8, Math.abs(endY - y1) * 0.5));
  const d = `M ${x1} ${y1} C ${x1} ${y1 + bend * dir}, ${x2} ${endY - bend * dir}, ${x2} ${endY}`;
  return `<path class="os-task-graph-link" ${linkAttrs} d="${escapeAttr(d)}"></path>`;
}

function dependencySuffix(
  upstreamVisible: TaskGraphNode[],
  upstreamHiddenCount: number,
  downstreamVisible: TaskGraphNode[],
  downstreamHiddenCount: number,
): string {
  const parts: string[] = [];
  if (upstreamVisible.length > 0) {
    parts.push(`blocked by ${upstreamVisible.slice(0, 2).map(nodeLabel).join(", ")}`);
    if (upstreamVisible.length > 2) {
      parts.push(`+${upstreamVisible.length - 2}`);
    }
  } else if (upstreamHiddenCount > 0) {
    parts.push(`blocked by ${upstreamHiddenCount} hidden`);
  }

  if (downstreamVisible.length > 0) {
    parts.push(`blocks ${downstreamVisible.slice(0, 3).map(nodeLabel).join(", ")}`);
    if (downstreamVisible.length > 3) {
      parts.push(`+${downstreamVisible.length - 3}`);
    }
  } else if (downstreamHiddenCount > 0) {
    parts.push(`blocks ${downstreamHiddenCount} hidden`);
  }

  return parts.join(" | ");
}

function renderReadOnlyTaskGraphNode(
  model: TaskGraphRenderModel,
  selectedNodeId: string | null,
  overlay: ReturnType<typeof buildRuntimeOverlay>,
): string {
  const { node, signal } = model;
  const isSelected = node.node_id === selectedNodeId;
  const stateTone = stateToneForTaskNode(node);
  const hasUpstream = signal.upstreamVisible.length > 0 || signal.upstreamHiddenCount > 0;
  const hasDownstream = signal.downstreamVisible.length > 0 || signal.downstreamHiddenCount > 0;
  // Dependencies read from the connector arrows now, so the card only keeps
  // the two badges that can't be inferred from position: the run status and
  // whether this task is actively blocking others.
  const blockerBadge = overlay.badges.includes("blocker") ? renderBadge("blocker") : "";
  const dependencyGlyph = hasUpstream && hasDownstream
    ? "<>"
    : hasUpstream
      ? "<"
      : hasDownstream
        ? ">"
        : "";

  return `
    <button type="button" class="os-node os-node-readonly ${isSelected ? "is-selected" : ""} ${hasUpstream ? "os-node-has-upstream" : ""} ${hasDownstream ? "os-node-has-downstream" : ""}" data-node-id="${escapeAttr(node.node_id)}" style="--os-lane: ${model.lane}; --os-node-indent: ${model.lane * taskGraphLaneWidth}px; --os-node-height: ${taskGraphRowHeight}px;">
      <span class="os-node-gutter" aria-hidden="true">${escapeHtml(dependencyGlyph)}</span>
      <span class="os-node-line">
        <strong>${escapeHtml(node.identifier)}</strong>
        <span>${escapeHtml(node.title)}</span>
      </span>
      <span class="os-node-tags">
        <em class="os-node-state os-node-state-${escapeAttr(stateTone)}">${escapeHtml(node.state)}</em>
        ${blockerBadge}
      </span>
    </button>
  `;
}

function renderDependencyDetail(node: TaskGraphNode, allNodes: TaskGraphNode[]): string {
  const signals = buildDependencySignals(allNodes, allNodes);
  const signal = signals.get(node.node_id);
  if (!signal) {
    return "";
  }
  const upstream = signal.upstreamVisible.length > 0
    ? `blocked by ${signal.upstreamVisible.map(nodeLabel).join(", ")}`
    : signal.upstreamHiddenCount > 0
      ? `blocked by ${signal.upstreamHiddenCount} hidden`
      : "ready";
  const completed = signal.completedBlockers.length > 0
    ? ` | completed blockers ${signal.completedBlockers.map(nodeLabel).join(", ")}`
    : "";
  const downstream = signal.downstreamVisible.length > 0
    ? ` | blocks ${signal.downstreamVisible.map(nodeLabel).join(", ")}`
    : signal.downstreamHiddenCount > 0
      ? ` | blocks ${signal.downstreamHiddenCount} hidden`
      : "";

  return `<div class="os-dependency-detail" data-testid="dependency-detail">deps: ${escapeHtml(upstream + completed + downstream)}</div>`;
}

function emptyDependencySignal(): DependencySignal {
  return {
    gutter: "   ",
    suffix: "",
    upstreamVisible: [],
    upstreamHiddenCount: 0,
    downstreamVisible: [],
    downstreamHiddenCount: 0,
    completedBlockers: [],
  };
}

function isTerminalTaskNode(node: TaskGraphNode): boolean {
  const state = `${node.state} ${node.state_category ?? ""}`.toLowerCase();
  return state.includes("done")
    || state.includes("complete")
    || state.includes("release")
    || state.includes("cancel");
}

function nodeLabel(node: TaskGraphNode): string {
  return node.identifier || node.node_id;
}

function runIdForNode(node: TaskGraphNode): string {
  return node.run_id || node.identifier || node.node_id;
}

/**
 * Whether the control plane is expected to serve a run detail for this
 * node. Backlog nodes and active issues the orchestrator has not picked up
 * yet arrive from the tracker scan alone (no run linkage, no runtime
 * overlay) — a /runs/{id} miss on them is by design, so it should not
 * raise a "Run unavailable" banner.
 */
function nodeHasRun(node: TaskGraphNode): boolean {
  return Boolean(node.run_id || node.runtime_overlay);
}

function findNodeByRef(nodes: TaskGraphNode[], ref: string): TaskGraphNode | undefined {
  const normalized = normalizeNodeRef(ref);
  return nodes.find((node) =>
    normalizeNodeRef(node.node_id) === normalized
    || normalizeNodeRef(node.identifier) === normalized,
  );
}

function normalizeNodeRef(ref: string): string {
  return ref.trim().toLowerCase();
}

function initialSelectedTaskNode(nodes: TaskGraphNode[], rootIds: string[]): TaskGraphNode | null {
  const ordered = [
    ...rootIds.map((id) => findNodeByRef(nodes, id)).filter((node): node is TaskGraphNode => Boolean(node)),
    ...nodes,
  ];
  // Side-pane nodes (backlog, done) have no run to open, so the initial
  // selection sticks to the Current pane and prefers non-terminal work —
  // a canceled node only wins when nothing else renders there.
  const current = ordered.filter(isCurrentPaneTaskNode);
  return current.find((node) => node.kind !== "milestone" && node.state_category === "in_progress")
    ?? current.find((node) => node.kind !== "milestone" && node.run_id && !isTerminalTaskNode(node))
    ?? current.find((node) => node.kind !== "milestone" && !isTerminalTaskNode(node))
    ?? current.find((node) => node.kind !== "milestone")
    ?? current[0]
    ?? null;
}

/**
 * Nodes rendered in the Current pane. Done nodes move to the Completed
 * pane and backlog nodes to the Backlog pane; canceled nodes stay here —
 * they have no other pane (the Completed endpoint serves done work only),
 * so dropping them would make the "Canceled" state filter show nothing.
 */
function isCurrentPaneTaskNode(node: TaskGraphNode): boolean {
  return node.state_category !== "backlog" && node.state_category !== "done";
}

/**
 * Fingerprint of a snapshot's done nodes; when it changes across a live
 * refresh, the Completed table (loaded separately) may be stale, so it
 * reloads. Includes the row-relevant fields — title, PR URL, timestamps —
 * not just IDs, so a completed issue whose PR or dates change while
 * staying done still triggers a reload.
 */
/**
 * Change signal for the separately-loaded Completed pane. Combines the
 * task graph's done nodes (a done node appearing or leaving) with the
 * control-plane completed count from the dashboard snapshot (a completion
 * whose issue never surfaces in the task graph — e.g. no project metadata —
 * still bumps this count). When it differs across a live refresh, the
 * Completed page is reloaded.
 */
function completedTasksSignature(
  snapshot: DashboardSnapshot | null,
  taskGraph: TaskGraphSnapshot | null,
): string {
  const completedCount = (snapshot?.projects ?? []).reduce(
    (sum, project) => sum + project.completed_count,
    0,
  );
  return `${completedCount}\n${doneTaskGraphKey(taskGraph)}`;
}

function doneTaskGraphKey(taskGraph: TaskGraphSnapshot | null): string {
  return (taskGraph?.nodes ?? [])
    .filter((node) => node.state_category === "done")
    .map((node) =>
      JSON.stringify([
        node.node_id,
        node.identifier,
        node.title,
        node.url ?? "",
        node.updated_at ?? "",
      ]),
    )
    .sort()
    .join("\n");
}

function stateToneForTaskNode(node: TaskGraphNode): string {
  const value = `${node.state} ${node.state_category ?? ""}`.toLowerCase();
  if (value.includes("human review") || value.includes("review")) return "review";
  if (value.includes("block")) return "blocked";
  if (value.includes("fail") || value.includes("cancel")) return "failed";
  if (value.includes("running") || value.includes("progress")) return "running";
  if (value.includes("done") || value.includes("complete") || value.includes("release")) return "done";
  if (value.includes("backlog")) return "backlog";
  if (value.includes("todo")) return "todo";
  if (value.includes("idle")) return "idle";
  return "neutral";
}

function measureKnowledgeGraphStage(root: ParentNode): { width: number; height: number } {
  const stage = root.querySelector<HTMLElement>("[data-kg-stage]");
  const rect = stage?.getBoundingClientRect();
  return {
    width: Math.max(360, Math.floor(rect?.width || 720)),
    height: Math.max(260, Math.floor(rect?.height || 420)),
  };
}

function stageSizeChanged(
  previous: { width: number; height: number },
  next: { width: number; height: number },
): boolean {
  return Math.abs(previous.width - next.width) > 32 || Math.abs(previous.height - next.height) > 32;
}

/** Feather-style "link" glyph used for the connection settings toggle. */
function connectionIconSvg(): string {
  return `<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"></path><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"></path></svg>`;
}

/** Feather-style "settings" gear glyph used for the model settings toggle. */
function gearIconSvg(): string {
  return `<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"></circle><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"></path></svg>`;
}

function appShellStyles(): string {
  return `
    :root { color-scheme: light dark; }
    html { -webkit-font-smoothing: antialiased; -moz-osx-font-smoothing: grayscale; }
    body { margin: 0; background: #f4f6f8; color: #17202a; font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
    .os-app { min-height: 100vh; display: flex; flex-direction: column; }
    .os-topbar { display: grid; grid-template-columns: minmax(180px, 0.9fr) auto minmax(420px, 2fr); align-items: center; gap: 18px; padding: 14px 18px; background: #ffffff; border-bottom: 1px solid #d8dee4; }
    .os-topbar h1 { margin: 0; font-size: 18px; line-height: 1.2; letter-spacing: 0; }
    .os-topbar p { margin: 5px 0 0; color: #5d6b78; font-size: 13px; }
    .os-status-strip { min-width: 0; display: grid; grid-template-columns: auto auto minmax(110px, 0.8fr) minmax(150px, 1.1fr) minmax(130px, 1fr); gap: 8px; align-items: center; }
    .os-status { display: inline-flex; align-items: center; gap: 8px; border: 1px solid #cad3dd; border-radius: 6px; padding: 7px 10px; background: #f8fafc; font-size: 13px; white-space: nowrap; }
    .os-status span { width: 9px; height: 9px; border-radius: 50%; background: #6b7280; }
    .os-status-connected span { background: #1f9d55; }
    .os-status-failed span { background: #c2410c; }
    .os-strip-metrics, .os-strip-connection, .os-strip-model, .os-event-mini { min-width: 0; min-height: 34px; display: flex; align-items: center; gap: 7px; border: 1px solid #d8dee4; border-radius: 6px; padding: 5px 7px; background: #fbfcfd; font-size: 12px; color: #536170; }
    .os-strip-metrics { flex-wrap: nowrap; }
    .os-strip-metrics span { white-space: nowrap; }
    .os-strip-metrics strong { color: #17202a; }
    .os-strip-connection > span, .os-strip-model > span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .os-event-mini ol { min-width: 0; display: grid; gap: 2px; margin: 0; padding: 0; list-style: none; }
    .os-event-mini li { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .os-event-mini time { color: #667788; font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; margin-right: 4px; }
    .os-event-mini span { color: #39708f; margin-right: 4px; }
    .os-icon-button { flex: 0 0 auto; min-height: 28px; padding: 4px 8px; font-size: 12px; }
    .os-glyph-button { width: 32px; min-height: 32px; margin-left: auto; padding: 0; display: inline-flex; align-items: center; justify-content: center; color: #536170; }
    .os-glyph-button svg { display: block; }
    .os-glyph-button:hover { color: #17202a; }
    .os-strip-alert { margin: 0; max-width: 240px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .os-grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 14px; padding: 14px; align-items: start; }
    .os-panel { background: #ffffff; border: 1px solid #d8dee4; border-radius: 8px; padding: 14px; min-width: 0; box-shadow: 0 1px 2px rgba(15, 23, 42, 0.05); }
    .os-run-detail-panel, .os-run-evidence-panel { grid-column: span 1; }
    .os-profile-panel { grid-column: span 3; }
    .os-model-panel { grid-column: 1 / -1; }
    .os-workspace-shell { grid-column: 1 / -1; display: grid; gap: 14px; min-height: 0; }
    .os-graph-hero-panel { min-height: 360px; }
    .os-graph-hero-toolbar { display: flex; justify-content: space-between; gap: 12px; align-items: center; margin-bottom: 12px; }
    .os-graph-hero-toolbar h2 { margin: 0; font-size: 15px; letter-spacing: 0; }
    .os-graph-hero-toolbar span { display: block; color: #667788; font-size: 12px; margin-top: 2px; }
    .os-graph-hero-body { min-width: 0; }
    .os-lower-columns { display: flex; align-items: stretch; gap: 0; height: var(--os-lower-row-height, 520px); min-height: 0; }
    .os-lower-columns > .os-panel { box-sizing: border-box; height: 100%; min-height: 0; overflow: auto; }
    .os-lower-columns > .os-panel:first-child { flex: 0 0 calc(var(--os-left-column, 50%) - 5px); }
    .os-lower-columns > .os-panel:last-child { flex: 0 0 calc(var(--os-right-column, 50%) - 5px); }
    .os-pane-resizer { flex: 0 0 10px; min-width: 10px; display: grid; place-items: center; cursor: col-resize; touch-action: none; outline: none; }
    .os-pane-resizer span { width: 2px; height: 100%; min-height: 44px; border-radius: 999px; background: #cad3dd; transition-property: background-color, width; transition-duration: 150ms; transition-timing-function: cubic-bezier(0.2, 0, 0, 1); }
    .os-pane-resizer:hover span, .os-pane-resizer:focus-visible span { width: 3px; background: #39708f; }
    .os-row-resizer { flex: none; width: 100%; height: 12px; cursor: row-resize; }
    .os-row-resizer span { width: 100%; min-width: 44px; height: 2px; min-height: 0; }
    .os-row-resizer:hover span, .os-row-resizer:focus-visible span { width: 100%; height: 3px; background: #39708f; }
    .os-panel-collapsed { padding-bottom: 12px; }
    .os-section-head > div { min-width: 0; }
    .os-section-head > div span { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .os-panel-toggle { flex: 0 0 auto; }
    .os-section-head .os-panel-toggle span { color: inherit; font-size: inherit; }
    .os-secondary-button { min-height: 30px; padding: 4px 10px; }
    .os-model-layout { display: grid; grid-template-columns: repeat(4, minmax(160px, 1fr)); gap: 10px; align-items: end; }
    .os-model-layout button { align-self: end; }
    .os-model-actions { display: flex; gap: 8px; align-items: end; flex-wrap: wrap; }
    .os-model-meta { margin-top: 10px; color: #667788; font-size: 12px; }
    .os-model-error { margin-top: 10px; border: 1px solid #f0b88e; border-radius: 6px; padding: 8px 10px; background: #fff7ed; color: #9a3412; font-size: 12px; }
    .os-advanced-settings { grid-column: 1 / -1; border-top: 1px solid #e5ebf0; padding-top: 8px; }
    .os-advanced-settings summary { cursor: pointer; color: #536170; font-size: 12px; }
    .os-advanced-grid { display: grid; grid-template-columns: repeat(3, minmax(160px, 1fr)); gap: 10px; margin-top: 8px; align-items: end; }
    .os-check-field { min-height: 34px; display: inline-flex; align-items: center; gap: 8px; color: #536170; font-size: 12px; }
    .os-check-field input { width: 16px; height: 16px; }
    .os-task-graph-panel { grid-column: span 2; }
    .os-section-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
    .os-section-head h2 { margin: 0; font-size: 15px; letter-spacing: 0; }
    .os-section-head span, .os-meta { color: #667788; font-size: 12px; }
    .os-inline-fields { display: grid; grid-template-columns: minmax(150px, 0.7fr) minmax(160px, 0.8fr) minmax(260px, 1.3fr) auto; gap: 10px; align-items: end; }
    .os-field { display: grid; gap: 5px; font-size: 12px; color: #536170; }
    .os-field input, .os-field select { min-height: 34px; border: 1px solid #cbd5df; border-radius: 6px; padding: 6px 8px; background: #ffffff; color: #17202a; font: inherit; }
    button { min-height: 34px; border: 1px solid #afbac5; border-radius: 6px; background: #eef3f8; color: #17202a; font: inherit; cursor: pointer; }
    button:disabled { opacity: 0.48; cursor: not-allowed; }
    button:hover:not(:disabled), .os-list-item:hover, .os-node:hover { border-color: #39708f; background: #e7f1f5; }
    .os-run-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 9px; margin-bottom: 12px; }
    .os-run-grid div { border: 1px solid #d8dee4; border-radius: 6px; padding: 10px; background: #f8fafc; }
    .os-run-grid strong { display: block; font-size: 18px; }
    .os-run-grid span { display: block; color: #667788; font-size: 12px; margin-top: 3px; }
    .os-list, .os-node-list { display: grid; gap: 8px; }
    .os-list-item, .os-node { width: 100%; text-align: left; display: grid; gap: 3px; padding: 10px; background: #ffffff; }
    .os-task-graph-stage { position: relative; min-width: min(100%, var(--os-graph-width)); overflow-x: auto; padding: 2px 0; }
    .os-task-graph-links { position: absolute; z-index: 3; inset: 2px auto auto 0; width: var(--os-graph-width); height: var(--os-graph-height); pointer-events: none; overflow: visible; }
    .os-task-graph-link { fill: none; stroke: #39708f; stroke-width: 1.9; stroke-linecap: round; stroke-linejoin: round; opacity: 0.9; marker-end: url(#os-task-arrow); transition: opacity 0.14s ease, stroke-width 0.14s ease; }
    .os-task-graph-link-skip { opacity: 0.72; }
    .os-task-graph-link.os-tg-hue-0 { stroke: #39708f; marker-end: url(#os-task-arrow-0); }
    .os-task-graph-link.os-tg-hue-1 { stroke: #7c3aed; marker-end: url(#os-task-arrow-1); }
    .os-task-graph-link.os-tg-hue-2 { stroke: #0f766e; marker-end: url(#os-task-arrow-2); }
    .os-task-graph-link.os-tg-hue-3 { stroke: #b0762f; marker-end: url(#os-task-arrow-3); }
    .os-task-graph-link.os-tg-hue-4 { stroke: #a05577; marker-end: url(#os-task-arrow-4); }
    .os-task-graph-links.os-links-hover .os-task-graph-link { opacity: 0.1; }
    .os-task-graph-links.os-links-hover .os-task-graph-link.is-active { opacity: 1; stroke-width: 2.5; }
    .os-task-graph-links marker path:not([fill]) { fill: #39708f; }
    /* Reserve the skip-arrow routing gutter (taskGraphRouteGutterWidth). */
    .os-node-graph-list { position: relative; z-index: 1; min-width: min(100%, var(--os-graph-width)); gap: 8px; padding-left: var(--os-tg-gutter, 62px); }
    /* Three-pane task graph: Completed | Current | Backlog. */
    /* gap: 0 — the resizers between expanded panes provide the separation
       (matching .os-lower-columns); collapsed strips carry their own margin. */
    .os-tg-panes { position: relative; display: flex; align-items: stretch; gap: 0; min-width: 0; }
    .os-tg-resizer { align-self: stretch; }
    .os-tg-pane { display: flex; flex-direction: column; min-width: 0; border: 1px solid #d8dee4; border-radius: 10px; background: #fbfcfe; }
    .os-tg-pane-done { flex: 0 0 clamp(300px, 30%, 460px); }
    .os-tg-pane-current { flex: 1 1 auto; }
    .os-tg-pane-backlog { flex: 0 0 clamp(260px, 26%, 420px); }
    .os-tg-pane-collapsed { flex: 0 0 44px; align-items: center; gap: 8px; padding: 8px 0; margin-inline: 5px; }
    .os-tg-pane-head { display: flex; align-items: center; gap: 8px; padding: 9px 12px; border-bottom: 1px solid #e3e8ee; }
    .os-tg-pane-head strong { font-size: 13px; }
    .os-tg-dot { width: 9px; height: 9px; border-radius: 999px; flex: 0 0 auto; }
    .os-tg-dot-done { background: #16a34a; }
    .os-tg-dot-current { background: #2563eb; }
    .os-tg-dot-backlog { background: #7c3aed; }
    .os-tg-count { border-radius: 999px; font-size: 11px; padding: 1px 8px; }
    .os-tg-count-done { background: #dcfce7; color: #166534; }
    .os-tg-count-current { background: #dbeafe; color: #1e40af; }
    .os-tg-count-backlog { background: #ede9fe; color: #5b21b6; }
    .os-tg-pane-toggle { margin-left: auto; min-height: 24px; min-width: 26px; padding: 0 6px; border-radius: 6px; font-size: 13px; line-height: 1; }
    .os-tg-pane-collapsed .os-tg-pane-toggle { margin-left: 0; }
    .os-tg-pane-vertical-label { writing-mode: vertical-rl; font-size: 12px; font-weight: 600; color: #405568; letter-spacing: 0.04em; }
    .os-tg-pane-body { min-height: 0; max-height: clamp(360px, 56vh, 720px); overflow: auto; padding: 10px; }
    /* Cross-pane dependency edges: measured overlay, never intercepts input. */
    .os-tg-cross-links { position: absolute; inset: 0; width: 100%; height: 100%; z-index: 4; pointer-events: none; overflow: visible; }
    .os-tg-cross-link { fill: none; stroke-width: 1.9; stroke-linecap: round; opacity: 0.34; transition: opacity 0.14s ease, stroke-width 0.14s ease; }
    .os-tg-cross-link.os-tg-hue-0 { stroke: #39708f; }
    .os-tg-cross-link.os-tg-hue-1 { stroke: #7c3aed; }
    .os-tg-cross-link.os-tg-hue-2 { stroke: #0f766e; }
    .os-tg-cross-link.os-tg-hue-3 { stroke: #b0762f; }
    .os-tg-cross-link.os-tg-hue-4 { stroke: #a05577; }
    /* Backlog edges read as "not yet actionable" until emphasized. */
    .os-tg-stage-backlog .os-task-graph-link { opacity: 0.26; }
    [data-tg-panes].os-tg-focused .os-task-graph-link:not(.is-active):not(.is-ancestry) { opacity: 0.1; }
    [data-tg-panes].os-tg-focused .os-tg-cross-link:not(.is-active):not(.is-ancestry) { opacity: 0.08; }
    [data-tg-panes] .os-task-graph-link.is-active,
    [data-tg-panes] .os-task-graph-link.is-ancestry,
    [data-tg-panes] .os-tg-cross-link.is-active,
    [data-tg-panes] .os-tg-cross-link.is-ancestry { opacity: 1; stroke-width: 2.8; }
    [data-tg-panes] .os-node.os-tg-dim { opacity: 0.42; }
    [data-tg-panes] .os-node.os-tg-ancestry { border-color: #7c3aed; box-shadow: 0 0 0 1px rgba(124, 58, 237, 0.25); }
    /* Completed pane: search + sortable table + pagination. */
    .os-tg-done-search { width: 100%; box-sizing: border-box; min-height: 32px; margin-bottom: 8px; border: 1px solid #cbd5df; border-radius: 6px; padding: 5px 9px; background: #ffffff; color: #17202a; font: inherit; font-size: 12px; }
    .os-tg-done-error { color: #b3372a; font-size: 12px; margin: 0 0 8px; }
    .os-tg-done-table { width: 100%; border-collapse: collapse; font-size: 12px; }
    .os-tg-done-table th { text-align: left; padding: 4px 6px; border-bottom: 1px solid #d8dee4; color: #536170; font-size: 11px; white-space: nowrap; }
    .os-tg-done-table td { padding: 6px; border-bottom: 1px solid #eef1f4; vertical-align: top; }
    .os-tg-done-sort { border: none; background: transparent; min-height: 22px; padding: 0; color: inherit; font: inherit; font-size: 11px; font-weight: 600; cursor: pointer; }
    .os-tg-done-sort span { color: #98a6b3; }
    .os-tg-done-sort.is-active, .os-tg-done-sort.is-active span { color: #23566f; }
    .os-tg-done-id { font-weight: 600; white-space: nowrap; color: #17202a; }
    .os-tg-done-title { max-width: 0; width: 55%; }
    .os-tg-done-title a, .os-tg-done-title span { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: inherit; text-decoration: none; }
    .os-tg-done-title a:hover { text-decoration: underline; }
    .os-tg-done-prs { white-space: nowrap; }
    .os-tg-pr { color: #23566f; text-decoration: none; }
    .os-tg-pr:hover { text-decoration: underline; }
    .os-tg-pr-latest { font-weight: 700; }
    .os-tg-pr-unmerged { text-decoration: line-through; color: #98a6b3; }
    .os-tg-pr-unmerged:hover { text-decoration: line-through underline; }
    .os-tg-done-date { white-space: nowrap; color: #536170; }
    .os-tg-capsule-button { min-height: 22px; min-width: 26px; padding: 0 4px; border: 1px solid #cdd6de; border-radius: 5px; background: #f4f7f9; color: #5b21b6; font-size: 13px; line-height: 1; cursor: pointer; }
    .os-tg-capsule-button:hover { border-color: #7c3aed; background: #ede9fe; }
    .os-tg-capsule-missing { color: #98a6b3; }
    .os-tg-done-pagination { display: flex; align-items: center; gap: 4px; margin-top: 10px; flex-wrap: wrap; }
    .os-tg-done-page { min-height: 26px; min-width: 28px; padding: 0 7px; font-size: 12px; border-radius: 6px; }
    .os-tg-done-page.is-active { border-color: #39708f; background: #e7f1f5; font-weight: 700; }
    .os-tg-done-page-size { margin-left: auto; color: #667788; font-size: 11px; }
    .os-knowledge-graph { display: grid; gap: 10px; min-width: 0; }
    .os-knowledge-toolbar { display: flex; justify-content: space-between; gap: 10px; align-items: center; min-width: 0; }
    .os-knowledge-toolbar div { display: grid; gap: 2px; min-width: 0; }
    .os-knowledge-toolbar strong { font-size: 13px; }
    .os-knowledge-toolbar span { color: #667788; font-size: 12px; }
    .os-kg-reset { flex: 0 0 auto; border-color: #39708f; color: #23566f; background: #e7f1f5; font-weight: 600; }
    .os-kg-status { flex: 0 0 auto; border: 1px solid #cbd5df; border-radius: 999px; padding: 3px 8px; color: #23566f; background: #e7f1f5; font-size: 11px; }
    .os-kg-status-failed { color: #991b1b; background: #fee2e2; border-color: #fecaca; }
    .os-code-filters { border: 1px solid #d8dee4; border-radius: 6px; padding: 5px 8px; background: #f8fafc; }
    .os-code-filters summary { cursor: pointer; color: #23566f; font-size: 11px; font-weight: 700; }
    .os-code-filter-grid { display: flex; flex-wrap: wrap; gap: 8px 12px; align-items: flex-start; padding-top: 8px; }
    .os-code-filter-group { display: grid; gap: 3px; margin: 0; padding: 4px 7px; border: 1px solid #d8dee4; border-radius: 5px; background: #ffffff; }
    .os-code-filter-group legend { padding: 0 3px; color: #667788; font-size: 10px; font-weight: 700; text-transform: uppercase; }
    .os-code-filter-group label, .os-code-filter-path, .os-code-filter-diagnostics { color: #405568; font-size: 11px; }
    .os-code-filter-path, .os-code-filter-diagnostics { display: grid; gap: 3px; }
    .os-code-filter-path input, .os-code-filter-diagnostics select { min-height: 24px; max-width: 220px; padding: 2px 5px; border: 1px solid #cbd5df; border-radius: 4px; background: #ffffff; color: #17202a; font-size: 11px; }
    .os-code-filter-grid > button { min-height: 24px; align-self: end; }
    .os-code-delta-badge { color: #92400e; font-size: 10px; font-weight: 700; text-transform: uppercase; }
    .os-knowledge-stage { position: relative; height: clamp(320px, 52vh, 680px); min-width: 0; overflow: hidden; border: 1px solid #d8dee4; border-radius: 6px; background: #eef1f4; }
    .os-knowledge-canvas { display: block; width: 100%; height: 100%; touch-action: none; outline: none; cursor: grab; }
    .os-knowledge-canvas[data-kg-pointer="pan"], .os-knowledge-canvas[data-kg-pointer="orbit"] { cursor: grabbing; }
    .os-knowledge-canvas[data-kg-pointer="drag-node"] { cursor: move; }
    .os-knowledge-labels { position: absolute; inset: 0; pointer-events: none; }
    .os-kg-label { position: absolute; transform: translate(-50%, 0); max-width: min(200px, 46%); min-height: 22px; padding: 2px 7px; border: 1px solid rgba(57, 112, 143, 0.24); border-radius: 999px; background: rgba(255, 255, 255, 0.92); color: #17202a; font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; pointer-events: auto; box-shadow: 0 3px 10px rgba(15, 23, 42, 0.08); transition: opacity 0.16s ease; }
    .os-kg-label.is-selected { border-color: #c2410c; color: #9a3412; background: #fff7ed; }
    .os-kg-label.is-hovered { border-color: #c2410c; color: #9a3412; }
    .os-kg-area-label { position: absolute; transform: translate(-50%, -50%); font-size: 17px; font-weight: 700; letter-spacing: 0.08em; text-transform: uppercase; opacity: 0.8; text-shadow: 0 1px 0 rgba(255, 255, 255, 0.75); transition: opacity 0.18s ease; pointer-events: none; white-space: nowrap; }
    .os-kg-tooltip { position: absolute; transform: translate(-50%, -100%); display: grid; gap: 2px; max-width: 260px; padding: 8px 10px; border: 1px solid #cbd5df; border-radius: 8px; background: rgba(255, 255, 255, 0.97); box-shadow: 0 8px 24px rgba(15, 23, 42, 0.16); pointer-events: none; z-index: 3; }
    .os-kg-tooltip strong { font-size: 12px; line-height: 1.25; white-space: normal; }
    .os-kg-tooltip span { color: #536170; font-size: 11px; }
    .os-kg-tooltip em { color: #23566f; font-size: 11px; font-style: normal; }
    .os-kg-controls-hint { position: absolute; right: 8px; bottom: 6px; color: #8a97a3; font-size: 10.5px; letter-spacing: 0.02em; pointer-events: none; user-select: none; }
    .os-kg-list { display: grid; gap: 5px; list-style: none; margin: 0; padding: 0; max-height: 160px; overflow: auto; }
    .os-kg-list li { position: relative; display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px; align-items: center; border: 1px solid #d8dee4; border-radius: 6px; padding: 5px 8px; background: #ffffff; }
    .os-kg-list li.is-selected { border-color: #c2410c; background: #fff7ed; }
    .os-kg-list button { min-width: 0; min-height: 22px; padding: 0; border: none; background: transparent; text-align: left; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 11.5px; }
    .os-kg-list span { color: #667788; font-size: 10px; text-transform: uppercase; }
    /* Truncated entity names surface instantly on hover (data-kg-overflow is
       set only for rows that actually ellipsize; title stays as fallback). */
    .os-kg-list button[data-kg-overflow]:hover::after { content: attr(data-kg-overflow); position: absolute; left: 4px; top: calc(100% + 2px); z-index: 40; max-width: min(320px, 90vw); white-space: normal; background: #1d2833; color: #f2f7fb; font-size: 11.5px; line-height: 1.35; padding: 4px 8px; border-radius: 5px; box-shadow: 0 6px 16px rgba(0, 0, 0, 0.25); pointer-events: none; }
    .os-kg-inspector { border: 1px solid #d8dee4; border-radius: 6px; padding: 8px 10px; background: #ffffff; }
    .os-kg-inspector h3 { margin: 0 0 8px; font-size: 13px; letter-spacing: 0; }
    .os-kg-inspector dl { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 6px 10px; margin: 0; }
    .os-kg-inspector dt { color: #667788; font-size: 11px; }
    .os-kg-inspector dd { margin: 2px 0 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; }
    .os-kg-breadcrumb { display: flex; align-items: center; gap: 6px; font-size: 12px; min-height: 20px; flex-wrap: wrap; }
    .os-kg-breadcrumb button { border: none; background: transparent; padding: 0; color: #23566f; font-size: 12px; font-weight: 600; cursor: pointer; text-decoration: underline; text-underline-offset: 2px; }
    .os-kg-breadcrumb span[aria-current] { color: #1d2833; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 32ch; }
    .os-kg-crumb-sep { color: #98a6b3; }
    .os-kg-inspector-header { display: flex; justify-content: space-between; align-items: baseline; gap: 8px; }
    .os-kg-inspector-header h3 { margin: 0 0 8px; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .os-kg-copy-deeplink { flex: 0 0 auto; border: 1px solid #cdd6de; border-radius: 5px; background: #f4f7f9; color: #2c4356; font-size: 11px; padding: 2px 8px; cursor: pointer; }
    .os-kg-capsule { margin-top: 8px; border-top: 1px solid #e3e8ee; padding-top: 8px; display: grid; gap: 6px; }
    .os-kg-capsule h4 { margin: 4px 0 0; font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em; color: #667788; }
    .os-kg-capsule-chips { display: flex; flex-wrap: wrap; gap: 4px; }
    .os-kg-chip { border: 1px solid #d8dee4; border-radius: 999px; background: #f4f7f9; color: #405568; font-size: 11px; padding: 1px 8px; }
    .os-kg-capsule-body { max-height: 220px; overflow: auto; font-size: 12px; line-height: 1.5; color: #2b3947; }
    .os-kg-capsule-body h4, .os-kg-capsule-body h5, .os-kg-capsule-body h6 { margin: 8px 0 2px; font-size: 12px; text-transform: none; letter-spacing: 0; color: #1d2833; }
    .os-kg-capsule-body p { margin: 4px 0; }
    .os-kg-capsule-body ul { margin: 4px 0; padding-left: 18px; }
    .os-kg-capsule-body code { background: #eef1f4; border-radius: 3px; padding: 0 3px; font-size: 11px; }
    .os-kg-capsule-links, .os-kg-capsule-sources { display: flex; flex-wrap: wrap; gap: 4px 8px; list-style: none; margin: 0; padding: 0; font-size: 12px; }
    .os-kg-capsule-link { border: none; background: transparent; padding: 0; color: #23566f; font-size: inherit; cursor: pointer; text-decoration: underline; text-underline-offset: 2px; }
    .os-kg-capsule-error { color: #b3372a; font-size: 12px; margin: 0; }
    /* Lower-column layout: the panel is the single scroll context — inner
       caps (sized for the old in-hero inspector) would nest scrollbars. */
    .os-knowledge-lower-panel .os-kg-list { max-height: none; overflow: visible; }
    .os-knowledge-lower-panel .os-kg-inspector { border: none; padding: 0; background: transparent; }
    .os-knowledge-lower-panel .os-kg-capsule-body { max-height: none; overflow: visible; }
    .os-surface-list { display: grid; gap: 6px; margin: 0; padding: 0; list-style: none; }
    .os-surface-list li { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px; align-items: center; border: 1px solid #d8dee4; border-radius: 6px; padding: 8px 9px; background: #ffffff; }
    .os-surface-list li.is-selected { border-color: #39708f; background: #e7f1f5; }
    .os-surface-list strong, .os-surface-detail strong { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .os-surface-list span, .os-surface-detail span { color: #667788; font-size: 11px; text-transform: uppercase; }
    .os-surface-detail { display: grid; gap: 5px; border: 1px solid #d8dee4; border-radius: 6px; padding: 10px; background: #f8fafc; }
    .os-project-group { display: grid; gap: 8px; margin-bottom: 10px; }
    .os-project-group-header { width: 100%; min-height: 32px; display: grid; grid-template-columns: 18px minmax(0, 1fr) auto; align-items: center; gap: 8px; padding: 6px 9px; border-radius: 6px; background: #f8fafc; text-align: left; }
    .os-project-group-header strong, .os-project-group-header em { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .os-project-group-header strong { color: #17202a; font-size: 12px; }
    .os-project-group-header em { color: #667788; font-size: 11px; font-style: normal; }
    .os-node-readonly { box-sizing: border-box; grid-template-columns: 28px minmax(0, 1fr) auto; column-gap: 10px; align-items: center; height: var(--os-node-height, 44px); width: calc(100% - var(--os-node-indent, 0px) - 8px); margin-left: var(--os-node-indent, 0px); margin-right: 8px; padding: 6px 10px; border-radius: 8px; font-size: 12px; overflow: hidden; transition-property: background-color, border-color, box-shadow, transform; transition-duration: 150ms; transition-timing-function: ease-out; }
    .os-node-readonly:active { transform: scale(0.996); }
    .os-node-gutter { width: 22px; height: 22px; display: inline-flex; align-items: center; justify-content: center; border-radius: 999px; font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; color: #39708f; font-size: 11px; font-weight: 800; white-space: pre; background: #e7f1f5; box-shadow: 0 0 0 1px rgba(57, 112, 143, 0.28); }
    .os-node-gutter:empty { background: transparent; box-shadow: 0 0 0 1px rgba(57, 112, 143, 0.18); }
    .os-node-has-upstream .os-node-gutter { background: #fff7ed; color: #92400e; box-shadow: 0 0 0 1px rgba(146, 64, 14, 0.32); }
    .os-node-has-downstream .os-node-gutter { background: #e7f1f5; color: #23566f; box-shadow: 0 0 0 1px rgba(57, 112, 143, 0.34); }
    .os-node-has-upstream.os-node-has-downstream .os-node-gutter { background: #fef3c7; color: #78350f; box-shadow: 0 0 0 1px rgba(146, 64, 14, 0.38); }
    .os-node-main, .os-node-line, .os-node-subline { min-width: 0; }
    .os-node-tags { display: inline-flex; align-items: center; gap: 6px; flex: 0 0 auto; margin-left: auto; }
    .os-node-line { display: flex; gap: 8px; align-items: baseline; flex-wrap: nowrap; }
    .os-node-line > span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .os-node-line strong { flex: 0 0 auto; font-size: 12px; font-variant-numeric: tabular-nums; }
    .os-node-dependency { display: block; margin-top: 3px; color: #92400e; font-size: 11px; line-height: 1.3; white-space: normal; overflow-wrap: anywhere; }
    .os-node-readonly .os-node-dependency { display: -webkit-box; -webkit-line-clamp: 1; -webkit-box-orient: vertical; overflow: hidden; }
    .os-node-subline { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; margin-top: 4px; }
    .os-node-readonly .os-node-subline { flex-wrap: nowrap; overflow: hidden; }
    .os-list-item span, .os-node span, .os-node em { color: #667788; font-size: 12px; font-style: normal; }
    .os-node .os-node-gutter { color: #39708f; }
    .os-node .os-node-dependency { color: #92400e; }
    .is-selected { border-color: #39708f; background: #e7f1f5; }
    .os-node-readonly.is-selected { box-shadow: 0 0 0 1px rgba(57, 112, 143, 0.32), 0 10px 24px rgba(15, 23, 42, 0.08); }
    .os-node-kind { text-transform: uppercase; letter-spacing: 0.08em; }
    .os-node-state { display: inline-flex; width: fit-content; border-radius: 999px; padding: 2px 8px; font-size: 11px; font-weight: 600; }
    .os-node-state-review { background: #fef3c7; color: #92400e; }
    .os-node-state-blocked, .os-node-state-failed { background: #fee2e2; color: #991b1b; }
    .os-node-state-running { background: #dcfce7; color: #166534; }
    .os-node-state-done { background: #dbeafe; color: #1e40af; }
    .os-node-state-backlog { background: #f1f5f9; color: #475569; }
    .os-node-state-todo, .os-node-state-idle { background: #e0f2fe; color: #0c4a6e; }
    .os-node-state-neutral { background: #f8fafc; color: #475569; border: 1px solid #d8dee4; }
    .os-detail-strip, .os-run-head { display: flex; align-items: center; justify-content: space-between; gap: 10px; margin-bottom: 10px; border: 1px solid #d8dee4; border-radius: 6px; padding: 10px; background: #fbfcfd; }
    .os-detail-strip span, .os-run-head span { color: #667788; font-size: 12px; }
    .os-events { margin: 0; padding-left: 18px; display: grid; gap: 5px; font-size: 12px; line-height: 1.35; }
    .os-events span { color: #39708f; margin-right: 5px; }
    .os-pill, .os-actions span { border-radius: 999px; background: #e7f1f5; color: #23566f; padding: 5px 9px; font-size: 12px; }
    .os-actions { display: flex; flex-wrap: wrap; gap: 6px; margin: 12px 0; }
    .os-run-detail-panel { font-size: 12px; }
    .os-run-detail-panel .os-section-head h2 { font-size: 14px; }
    .os-run-detail-panel button { min-height: 30px; padding: 5px 8px; font-size: 12px; }
    .os-run-detail-panel .os-run-head { padding: 8px 10px; margin-bottom: 8px; }
    .os-run-detail-panel .os-run-head strong { font-size: 14px; }
    .os-run-detail-panel .os-run-grid { grid-template-columns: repeat(auto-fit, minmax(82px, 1fr)); gap: 6px; margin-bottom: 8px; }
    .os-run-detail-panel .os-run-grid div { min-height: 42px; padding: 6px 7px; }
    .os-run-detail-panel .os-run-grid strong { font-size: 13px; line-height: 1.2; white-space: nowrap; }
    .os-run-detail-panel .os-run-grid span { font-size: 10px; line-height: 1.2; margin-top: 0; }
    .os-run-meta-list { display: grid; gap: 6px; margin-bottom: 8px; }
    .os-run-meta-row { display: grid; grid-template-columns: 48px minmax(0, 1fr); gap: 8px; align-items: center; border: 1px solid #d8dee4; border-radius: 6px; padding: 7px 8px; background: #f8fafc; }
    .os-run-meta-row span { color: #667788; font-size: 10px; line-height: 1.2; text-transform: uppercase; letter-spacing: 0.04em; }
    .os-run-meta-row code, .os-run-meta-row a, .os-run-meta-row em { min-width: 0; overflow-wrap: anywhere; color: #17202a; font-size: 12px; line-height: 1.3; }
    .os-run-meta-row a { color: #23566f; font-weight: 700; }
    .os-run-meta-row em { color: #667788; font-style: normal; }
    .os-run-action-bar { display: flex; flex-wrap: wrap; gap: 6px; margin: 8px 0; }
    .os-action-item { display: flex; align-items: center; gap: 8px; }
    .os-action-warning { color: #b45309; font-size: 12px; }
    .os-action-receipt { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; font-size: 12px; margin: 10px 0; padding: 8px; border: 1px solid #d8dee4; border-radius: 6px; background: #f8fafc; }
    .os-receipt-status-accepted { color: #1f9d55; }
    .os-receipt-status-rejected { color: #c2410c; }
    .os-dependency-detail { border: 1px solid #d8dee4; border-radius: 6px; padding: 8px 10px; background: #fbfcfd; color: #536170; font-size: 12px; margin-bottom: 10px; }
    .os-run-panels { display: grid; grid-template-columns: 1fr; gap: 12px; margin: 12px 0; }
    .os-run-section { display: grid; gap: 8px; }
    .os-run-section + .os-run-section { margin-top: 14px; padding-top: 12px; border-top: 1px solid #d8dee4; }
    .os-run-section h3 { margin: 0; font-size: 13px; letter-spacing: 0; color: #536170; }
    .os-segmented { display: inline-flex; width: fit-content; gap: 4px; padding: 3px; border: 1px solid #d8dee4; border-radius: 7px; background: #fbfcfd; margin-bottom: 12px; }
    .os-segmented button { min-height: 28px; padding: 4px 10px; border-color: transparent; background: transparent; }
    .os-segmented button.is-selected { border-color: #39708f; background: #e7f1f5; font-weight: 600; }
    .os-run-activity { display: grid; gap: 6px; }
    .os-activity-lifecycle { min-height: 34px; display: grid; grid-template-columns: auto auto minmax(0, 1fr); gap: 8px; align-items: center; border-radius: 6px; padding: 7px 9px; background: #eef3f8; color: #536170; font-size: 12px; }
    .os-activity-lifecycle span { font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; font-variant-numeric: tabular-nums; color: #667788; }
    .os-activity-lifecycle strong { color: #23566f; }
    .os-activity-lifecycle em { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-style: normal; }
    .os-activity-entry { display: grid; gap: 7px; align-items: start; border: 1px solid #d8dee4; border-radius: 6px; padding: 8px; background: #f8fafc; font-size: 12px; }
    .os-activity-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px; align-items: center; min-width: 0; }
    .os-activity-meta { display: flex; gap: 6px; align-items: baseline; min-width: 0; overflow: hidden; }
    .os-activity-entry span { color: #667788; white-space: nowrap; }
    .os-activity-entry strong { color: #39708f; white-space: nowrap; }
    .os-activity-preview { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: #27313a; }
    .os-activity-toggle { width: 24px; height: 24px; min-height: 24px; padding: 0; border-radius: 999px; display: inline-grid; place-items: center; font-size: 13px; line-height: 1; }
    .os-activity-detail { margin: 0; max-height: 260px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; border: 1px solid #d8dee4; border-radius: 5px; padding: 8px; background: #ffffff; font-size: 12px; line-height: 1.35; }
    .os-knowledge-graph { display: grid; gap: 10px; }
    .os-knowledge-status { display: grid; gap: 3px; border: 1px solid #d8dee4; border-radius: 6px; padding: 9px 10px; background: #f8fafc; }
    .os-knowledge-status strong { color: #23566f; }
    .os-knowledge-status span { color: #667788; font-size: 12px; overflow-wrap: anywhere; }
    .os-knowledge-status-stale { border-color: #f59e0b; background: #fffbeb; }
    .os-knowledge-status-warning { border-color: #d97706; background: #fff7ed; }
    .os-knowledge-status-failed { border-color: #ef4444; background: #fef2f2; }
    .os-knowledge-metrics { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 6px; }
    .os-knowledge-metrics div { min-height: 42px; display: grid; align-content: center; gap: 2px; border: 1px solid #d8dee4; border-radius: 6px; padding: 7px 8px; background: #fbfcfd; }
    .os-knowledge-metrics strong { color: #17202a; font-size: 14px; font-variant-numeric: tabular-nums; }
    .os-knowledge-metrics span { color: #667788; font-size: 10px; line-height: 1.2; }
    .os-knowledge-list { display: grid; gap: 6px; margin: 0; padding: 0; list-style: none; }
    .os-knowledge-list li { display: grid; gap: 2px; border: 1px solid #d8dee4; border-radius: 6px; padding: 8px 9px; background: #ffffff; min-width: 0; }
    .os-knowledge-list strong { color: #17202a; overflow-wrap: anywhere; font-size: 12px; }
    .os-knowledge-list span { color: #667788; font-size: 11px; }
    .os-knowledge-map { position: relative; min-height: 220px; border-radius: 8px; background: #fbfcfd; box-shadow: inset 0 0 0 1px rgba(57, 112, 143, 0.14); overflow: hidden; }
    .os-kg-node { position: absolute; z-index: 2; display: grid; place-items: center; width: 54px; height: 54px; border-radius: 999px; background: #e7f1f5; color: #23566f; box-shadow: 0 0 0 1px rgba(57, 112, 143, 0.24), 0 10px 24px rgba(15, 23, 42, 0.08); font-size: 11px; font-weight: 700; }
    .os-kg-node-main { left: calc(50% - 27px); top: calc(50% - 27px); background: #dcfce7; color: #166534; }
    .os-kg-node-a { left: 18%; top: 22%; }
    .os-kg-node-b { right: 14%; top: 24%; }
    .os-kg-node-c { left: 34%; bottom: 12%; }
    .os-kg-edge { position: absolute; z-index: 1; left: 50%; top: 50%; width: clamp(110px, 22%, 190px); height: 2px; transform-origin: left center; background: rgba(57, 112, 143, 0.35); border-radius: 999px; }
    .os-kg-edge-a { transform: rotate(210deg); }
    .os-kg-edge-b { transform: rotate(330deg); }
    .os-kg-edge-c { transform: rotate(108deg); width: 28%; }
    .os-knowledge-summary { display: grid; gap: 3px; padding: 10px; border-radius: 6px; background: #f8fafc; box-shadow: inset 0 0 0 1px rgba(216, 222, 228, 0.9); }
    .os-knowledge-summary strong { color: #23566f; }
    .os-knowledge-summary span, .os-knowledge-summary em { color: #667788; font-size: 12px; font-style: normal; text-wrap: pretty; }
    .os-changed-file-list { display: grid; gap: 6px; }
    .os-changed-file { width: 100%; text-align: left; display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 7px; align-items: center; padding: 7px 8px; background: #ffffff; font-size: 12px; line-height: 1.2; }
    .os-changed-file.os-selected { border-color: #39708f; background: #e7f1f5; }
    .os-file-path { min-width: 0; overflow-wrap: anywhere; }
    .os-file-stats { white-space: nowrap; font-size: 12px; font-variant-numeric: tabular-nums; }
    .os-lines-added { color: #1f9d55; }
    .os-lines-removed { color: #c2410c; }
    .os-change-kind { text-transform: uppercase; font-size: 10px; padding: 2px 5px; border-radius: 4px; }
    .os-change-kind-created { background: #dcfce7; color: #166534; }
    .os-change-kind-modified { background: #e0f2fe; color: #0c4a6e; }
    .os-change-kind-removed { background: #fee2e2; color: #991b1b; }
    .os-file-diff { border: 1px solid #d8dee4; border-radius: 6px; background: #f8fafc; }
    .os-diff-header { display: flex; justify-content: space-between; padding: 8px; border-bottom: 1px solid #d8dee4; background: #eef3f8; font-size: 12px; }
    .os-diff-hunk { padding: 8px; font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; font-size: 12px; }
    .os-diff-hunk-header { color: #667788; margin-bottom: 4px; }
    .os-diff-line { white-space: pre-wrap; }
    .os-diff-line-addition { color: #1f9d55; background: #dcfce7; }
    .os-diff-line-deletion { color: #c2410c; background: #fee2e2; }
    .os-diff-line-context { color: #334155; }
    .os-validation-header { display: flex; justify-content: space-between; padding: 8px; border-bottom: 1px solid #d8dee4; background: #eef3f8; }
    .os-validation-status-passed { color: #1f9d55; }
    .os-validation-status-failed { color: #c2410c; }
    .os-validation-status-error { color: #c2410c; }
    .os-validation-status-pending { color: #6b7280; }
    .os-validation-command, .os-validation-evidence-item { padding: 8px; border-bottom: 1px solid #eef3f8; }
    .os-approval-list { display: grid; gap: 10px; }
    .os-approval-item { border: 1px solid #d8dee4; border-radius: 6px; padding: 10px; }
    .os-approval-title { font-weight: 600; }
    .os-approval-explain { display: flex; gap: 8px; margin-top: 8px; }
    .os-approval-explain input { flex: 1; }
    .os-approval-risk-high { color: #c2410c; }
    .os-approval-risk-medium { color: #b45309; }
    .os-approval-risk-low { color: #1f9d55; }
    .os-audit-trail { display: grid; gap: 6px; margin-top: 12px; }
    .os-audit-trail-entry { display: grid; grid-template-columns: auto auto auto auto 1fr; gap: 8px; font-size: 12px; }

    .os-node-actions { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 6px; }
    .os-node-actions button { min-height: 26px; padding: 4px 8px; font-size: 11px; }
    .os-node-badges { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
    .os-node-readonly .os-node-badges { flex: 0 1 auto; flex-wrap: nowrap; min-width: 0; overflow: hidden; }
    .os-badge { border-radius: 999px; background: #e7f1f5; color: #23566f; padding: 3px 7px; font-size: 10px; text-transform: uppercase; letter-spacing: 0.04em; }
    .os-badge-failed, .os-badge-blocked, .os-badge-blocker { background: #fee2e2; color: #991b1b; }
    .os-badge-running { background: #dcfce7; color: #166534; }
    .os-badge-complete { background: #dbeafe; color: #1e40af; }
    .os-badge-stale { background: #fef3c7; color: #92400e; }
    .os-badge-queued, .os-badge-retry { background: #f3e8ff; color: #6b21a8; }
    .os-badge-workspace, .os-badge-harness, .os-badge-diff_summary, .os-badge-validation { background: #f1f5f9; color: #475569; }
    .os-run-meta { color: #94a3b3; font-size: 11px; margin-top: 2px; }
    .os-filter-bar { display: flex; flex-wrap: wrap; gap: 10px; align-items: end; margin-bottom: 12px; padding: 10px; border: 1px solid #d8dee4; border-radius: 6px; background: #fbfcfd; }
    .os-filter-bar .os-field { flex: 1 1 140px; }
    .os-tg-toolbar { display: flex; gap: 8px; margin-bottom: 10px; }
    .os-pending-banner { padding: 8px 10px; border-radius: 6px; background: #fef3c7; color: #92400e; font-size: 12px; margin-bottom: 10px; }
    .os-dialog-backdrop, .os-modal-backdrop { position: fixed; inset: 0; background: rgba(15, 23, 42, 0.45); display: flex; align-items: center; justify-content: center; z-index: 100; }
    .os-dialog { background: #ffffff; border: 1px solid #d8dee4; border-radius: 8px; padding: 18px; min-width: 320px; max-width: 90vw; box-shadow: 0 10px 25px rgba(15, 23, 42, 0.15); }
    .os-modal-backdrop .os-dialog { width: min(920px, calc(100vw - 32px)); max-height: calc(100vh - 32px); overflow: auto; }
    .os-event-log-modal { width: min(760px, calc(100vw - 32px)); }
    .os-events-full { max-height: min(62vh, 560px); overflow: auto; }
    .os-dialog .os-section-head { margin-bottom: 14px; }
    .os-dialog-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 14px; }
    .os-dialog-actions-bar { display: flex; justify-content: flex-end; gap: 8px; margin-top: 10px; }
    .os-inline-input { min-height: 28px; padding: 4px 6px; border: 1px solid #39708f; border-radius: 4px; font: inherit; }
    .os-inline-state { width: 120px; }
    pre { margin: 0; padding: 10px; border-radius: 6px; background: #17202a; color: #d7e4ee; overflow: auto; font-size: 12px; }
    .os-empty { color: #667788; font-size: 13px; border: 1px dashed #cbd5df; border-radius: 6px; padding: 14px; }
    .os-auth-panel { display: flex; flex-direction: column; gap: 14px; grid-column: 1 / -1; }
    .os-auth-panel .os-section-head span { text-transform: uppercase; font-size: 11px; letter-spacing: 0.04em; color: #667788; }
    .os-auth-body { display: flex; flex-direction: column; gap: 10px; }
    .os-auth-message { margin: 0; font-size: 14px; }
    .os-auth-actions { display: flex; gap: 8px; flex-wrap: wrap; }
    .os-auth-note { margin: 0; font-size: 12px; color: #667788; }
    .os-auth-denied .os-auth-message { color: #991b1b; }
    .os-auth-scope { border: 1px solid #d8dee4; border-radius: 6px; padding: 12px; display: flex; flex-direction: column; gap: 8px; background: #f8fafc; }
    .os-auth-scope .os-section-head h3 { margin: 0; font-size: 13px; }
    .os-auth-scope .os-auth-note { margin: 0; }
    .os-view-tabs { display: inline-flex; gap: 6px; }
    .os-view-tab { min-height: 32px; padding: 6px 12px; font-size: 13px; border-radius: 6px; background: #f8fafc; border: 1px solid #cad3dd; }
    .os-view-tab-active { background: #e7f1f5; border-color: #39708f; font-weight: 600; }
    .os-view-tab-preview { color: #8a97a3; }
    .os-view-tab-preview.os-view-tab-active { color: #536170; }
    .os-tab-badge { margin-left: 6px; padding: 1px 5px; font-size: 10px; font-weight: 600; letter-spacing: 0.04em; border-radius: 4px; background: #fef3c7; border: 1px solid #f0d48a; color: #92600a; vertical-align: 1px; }
    .os-preview-banner { grid-column: 1 / -1; display: flex; align-items: baseline; gap: 10px; padding: 10px 14px; border: 1px solid #f0d48a; border-radius: 8px; background: #fef8e7; color: #92600a; font-size: 13px; }
    .os-preview-banner strong { text-transform: uppercase; font-size: 11px; letter-spacing: 0.06em; white-space: nowrap; }
    .os-planning-panel { grid-column: 1 / -1; display: flex; flex-direction: column; gap: 14px; }
    .os-planning-head { display: flex; align-items: center; justify-content: space-between; gap: 14px; flex-wrap: wrap; }
    .os-planning-head h2 { margin: 0; font-size: 16px; }
    .os-plan-tabs { display: inline-flex; gap: 6px; flex-wrap: wrap; }
    .os-plan-tab { min-height: 30px; padding: 5px 10px; font-size: 12px; border-radius: 6px; background: #f8fafc; border: 1px solid #cad3dd; }
    .os-plan-tab-active { background: #e7f1f5; border-color: #39708f; font-weight: 600; }
    .os-planning-layout { display: flex; gap: 16px; min-height: 420px; }
    .os-planning-conversation { flex: 0 0 300px; display: flex; flex-direction: column; gap: 10px; }
    .os-planning-content { flex: 1 1 auto; min-width: 0; display: flex; flex-direction: column; gap: 10px; }
    .os-conversation-list { display: flex; flex-direction: column; gap: 8px; max-height: 320px; overflow: auto; }
    .os-conversation-message { border: 1px solid #d8dee4; border-radius: 6px; padding: 8px; background: #f8fafc; font-size: 13px; }
    .os-conversation-message p { margin: 0; }
    .os-conversation-role { display: block; font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em; color: #667788; margin-bottom: 4px; }
    .os-conversation-user { background: #eef3f8; border-color: #cbd5df; }
    .os-conversation-assistant { background: #f0fdf4; border-color: #bbf7d0; }
    .os-planning-actions { display: flex; gap: 8px; align-items: end; flex-wrap: wrap; }
    .os-planning-actions input { flex: 1 1 180px; }
    .os-plan-hierarchy { display: flex; flex-direction: column; gap: 4px; }
    .os-plan-hierarchy-row { display: flex; align-items: center; gap: 6px; border: 1px solid #d8dee4; border-radius: 6px; padding: 8px 10px; background: #ffffff; cursor: pointer; }
    .os-plan-hierarchy-row:hover { border-color: #39708f; background: #e7f1f5; }
    .os-plan-hierarchy-row.is-selected { border-color: #39708f; background: #e7f1f5; }
    .os-plan-toggle, .os-plan-toggle-spacer { width: 22px; height: 22px; display: inline-flex; align-items: center; justify-content: center; border: none; background: transparent; color: #667788; font-size: 12px; cursor: pointer; }
    .os-plan-node-body { flex: 1; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; min-width: 0; }
    .os-plan-node-body strong { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .os-plan-node-body span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .os-plan-checklist { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
    .os-plan-checklist-row { display: flex; align-items: center; gap: 8px; border: 1px solid #d8dee4; border-radius: 6px; padding: 6px 8px; background: #ffffff; }
    .os-plan-checklist-row input[type="checkbox"] { width: 18px; height: 18px; }
    .os-plan-checklist-row input[type="text"] { flex: 1; min-width: 0; border: 1px solid #cbd5df; border-radius: 4px; padding: 4px 6px; }
    .os-plan-validation-list { display: flex; flex-direction: column; gap: 6px; }
    .os-plan-validation-row { border-radius: 6px; padding: 8px 10px; font-size: 13px; }
    .os-plan-validation-error { background: #fee2e2; color: #991b1b; }
    .os-plan-validation-warning { background: #fef3c7; color: #92400e; }
    .os-plan-validation-info { background: #dbeafe; color: #1e40af; }
    .os-plan-validation-link { background: transparent; border: none; padding: 0; margin: 0; font: inherit; color: inherit; text-decoration: underline; cursor: pointer; text-align: left; }
    .os-plan-diff { border: 1px solid #d8dee4; border-radius: 6px; background: #ffffff; font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; font-size: 12px; max-height: 320px; overflow: auto; }
    .os-plan-diff-line { display: grid; grid-template-columns: 36px 36px 1fr; gap: 8px; padding: 2px 8px; white-space: pre-wrap; }
    .os-plan-diff-lnum, .os-plan-diff-rnum { color: #94a3b3; text-align: right; }
    .os-plan-diff-add { background: #dcfce7; color: #166534; }
    .os-plan-diff-remove { background: #fee2e2; color: #991b1b; }
    .os-plan-diff-unchanged { background: transparent; }
    .os-plan-graph { border: 1px solid #d8dee4; border-radius: 6px; background: #ffffff; overflow: auto; }
    .os-plan-graph svg { display: block; min-width: 100%; }
    .os-plan-graph-edge { stroke: #cbd5df; stroke-width: 2; }
    .os-plan-graph-dependency { stroke: #92400e; stroke-dasharray: 4 2; }
    .os-plan-graph-node rect { fill: #f8fafc; stroke: #d8dee4; stroke-width: 1; }
    .os-plan-graph-node text { font-size: 11px; fill: #17202a; }
    .os-plan-graph-node-sub { font-size: 10px; fill: #667788; }
    .os-plan-graph-node-selected rect { fill: #e7f1f5; stroke: #39708f; }
    @media (max-width: 980px) {
      .os-grid { grid-template-columns: 1fr; }
      .os-profile-panel, .os-model-panel, .os-task-graph-panel, .os-graph-hero-panel, .os-run-detail-panel, .os-run-evidence-panel, .os-planning-panel { grid-column: 1 / -1; }
      .os-workspace-shell, .os-lower-columns { display: grid; grid-template-columns: 1fr; gap: 14px; min-height: 0; }
      .os-lower-columns { height: auto; }
      .os-lower-columns > .os-panel { height: auto; max-height: none; min-height: 0; overflow: visible; }
      .os-pane-resizer { display: none; }
      .os-inline-fields, .os-model-layout, .os-advanced-grid, .os-run-grid { grid-template-columns: 1fr; }
      .os-topbar, .os-status-strip { grid-template-columns: 1fr; align-items: stretch; }
      .os-graph-hero-toolbar { align-items: flex-start; flex-direction: column; }
    }
    @media (prefers-color-scheme: dark) {
      body { background: #101418; color: #d9e2ea; }
      .os-topbar, .os-panel, .os-list-item, .os-node, .os-dialog { background: #171d23; border-color: #2a3440; }
      .os-topbar p, .os-section-head span, .os-meta, .os-model-meta, .os-check-field, .os-list-item span, .os-node span, .os-node em, .os-empty, .os-run-grid span, .os-run-meta, .os-run-meta-row span { color: #94a3b3; }
      .os-status, .os-strip-metrics, .os-strip-connection, .os-strip-model, .os-event-mini, .os-run-grid div, .os-run-meta-row, .os-detail-strip, .os-run-head, .os-filter-bar, .os-pending-banner { background: #111820; border-color: #2a3440; }
      .os-strip-metrics strong, .os-surface-list strong, .os-surface-detail strong { color: #d9e2ea; }
      .os-surface-detail { background: #111820; border-color: #344454; }
      .os-surface-list span, .os-surface-detail span { color: #94a3b3; }
      .os-surface-list li { background: #171d23; border-color: #2a3440; }
      .os-surface-list li.is-selected { background: #18303a; border-color: #5ca0b8; }
      .os-knowledge-stage { background: #e7ebef; border-color: #2a3440; }
      .os-kg-list li, .os-kg-inspector { background: #111820; border-color: #2a3440; }
      .os-kg-label { background: rgba(17, 24, 32, 0.92); color: #d9e2ea; border-color: #2a3440; }
      .os-kg-label.is-selected { background: rgba(47, 32, 23, 0.95); color: #fed7aa; border-color: #fb923c; box-shadow: 0 0 0 1px rgba(251, 146, 60, 0.22), 0 8px 18px rgba(0, 0, 0, 0.2); }
      .os-kg-list li.is-selected { background: #2a2118; border-color: #fb923c; box-shadow: inset 3px 0 0 rgba(251, 146, 60, 0.95); }
      .os-kg-list button { color: #d9e2ea; }
      .os-kg-list li.is-selected button, .os-kg-inspector h3, .os-kg-inspector dd { color: #f2f7fb; }
      .os-kg-list li.is-selected span { color: #fdba74; }
      .os-kg-inspector dt, .os-kg-inspector p { color: #94a3b3; }
      .os-kg-list button[data-kg-overflow]:hover::after { background: #2a3542; color: #f2f7fb; box-shadow: 0 6px 16px rgba(0, 0, 0, 0.55); }
      .os-kg-breadcrumb button, .os-kg-capsule-link { color: #8bd0e6; }
      .os-kg-breadcrumb span[aria-current] { color: #f2f7fb; }
      .os-kg-crumb-sep { color: #5b6b7a; }
      .os-kg-copy-deeplink { background: #111820; border-color: #2a3440; color: #d9e2ea; }
      .os-kg-capsule { border-top-color: #2a3440; }
      .os-kg-capsule h4 { color: #94a3b3; }
      .os-kg-chip { background: #111820; border-color: #2a3440; color: #b8c6d2; }
      .os-kg-capsule-body { color: #c4d0da; }
      .os-kg-capsule-body h4, .os-kg-capsule-body h5, .os-kg-capsule-body h6 { color: #f2f7fb; }
      .os-kg-capsule-body code { background: #1c2630; }
      .os-kg-capsule-error { color: #fca5a5; }
      .os-run-meta-row code { color: #d9e2ea; }
      .os-run-meta-row a { color: #8bd0e6; }
      .os-run-meta-row em { color: #94a3b3; }
      .os-model-error { background: #32180d; border-color: #7c2d12; color: #fed7aa; }
      .os-auth-panel .os-auth-message { color: #d9e2ea; }
      .os-auth-denied .os-auth-message { color: #fca5a5; }
      .os-auth-note { color: #94a3b3; }
      .os-auth-scope { background: #111820; border-color: #2a3440; }
      .os-segmented { background: #111820; border-color: #2a3440; }
      .os-segmented button.is-selected { background: #18303a; border-color: #5ca0b8; }
      .os-field input, .os-field select, .os-inline-input, .os-dialog textarea { background: #0f151b; color: #d9e2ea; border-color: #344454; }
      button { background: #1f2a35; color: #d9e2ea; border-color: #3b4c5e; }
      button:hover:not(:disabled), .os-list-item:hover, .os-node:hover, .os-changed-file:hover, .is-selected { background: #18303a; border-color: #5ca0b8; }
      .os-task-graph-link { stroke: #5ca0b8; opacity: 0.82; }
      .os-task-graph-links marker path { fill: #5ca0b8; }
      .os-tg-pane { background: #131920; border-color: #2a3440; }
      .os-tg-pane-head { border-color: #2a3440; }
      .os-tg-count-done { background: #14351f; color: #86efac; }
      .os-tg-count-current { background: #16283f; color: #93c5fd; }
      .os-tg-count-backlog { background: #271c3f; color: #c4b5fd; }
      .os-tg-pane-vertical-label { color: #b8c4cf; }
      .os-tg-stage-backlog .os-task-graph-link { opacity: 0.3; }
      [data-tg-panes] .os-node.os-tg-ancestry { border-color: #a78bfa; box-shadow: 0 0 0 1px rgba(167, 139, 250, 0.35); }
      .os-tg-done-search { background: #101720; border-color: #2a3440; color: #e6edf3; }
      .os-tg-done-table th { border-color: #2a3440; color: #94a3b3; }
      .os-tg-done-table td { border-color: #1e2731; }
      .os-tg-done-id { color: #e6edf3; }
      .os-tg-done-sort.is-active, .os-tg-done-sort.is-active span { color: #8bd0e6; }
      .os-tg-pr { color: #8bd0e6; }
      .os-tg-pr-unmerged { color: #64748b; }
      .os-tg-capsule-button { background: #1a2230; border-color: #2f3b4c; color: #c4b5fd; }
      .os-tg-capsule-button:hover { border-color: #a78bfa; background: #271c3f; }
      .os-tg-done-page.is-active { background: #18303a; border-color: #5ca0b8; }
      .os-node-gutter { background: #10232c; box-shadow: 0 0 0 1px rgba(92, 160, 184, 0.3); }
      .os-node-gutter:empty { background: transparent; box-shadow: 0 0 0 1px rgba(92, 160, 184, 0.22); }
      .os-node-has-upstream .os-node-gutter { background: #3a2414; color: #fbbf24; box-shadow: 0 0 0 1px rgba(251, 191, 36, 0.34); }
      .os-node-has-downstream .os-node-gutter { background: #102c34; color: #8bd0e6; box-shadow: 0 0 0 1px rgba(92, 160, 184, 0.38); }
      .os-node-has-upstream.os-node-has-downstream .os-node-gutter { background: #3f3215; color: #fde68a; box-shadow: 0 0 0 1px rgba(251, 191, 36, 0.42); }
      .os-node-readonly.is-selected { box-shadow: 0 0 0 1px rgba(92, 160, 184, 0.36), 0 14px 28px rgba(0, 0, 0, 0.18); }
      .os-view-tab, .os-plan-tab, .os-changed-file { background: #111820; color: #d9e2ea; border-color: #3b4c5e; }
      .os-view-tab-active, .os-plan-tab-active, .os-changed-file.os-selected { background: #18303a; color: #f2f7fb; border-color: #5ca0b8; }
      .os-changed-file .os-file-path { color: #e6edf3; }
      .os-changed-file .os-file-stats { color: #cbd5e1; }
      .os-lines-added { color: #86efac; }
      .os-lines-removed { color: #fecaca; }
      .os-file-diff, .os-approval-item, .os-validation-command, .os-validation-evidence-item { background: #111820; border-color: #2a3440; }
      .os-run-section + .os-run-section { border-color: #2a3440; }
      .os-run-section h3 { color: #94a3b3; }
      .os-diff-header, .os-validation-header { background: #1f2a35; border-color: #2a3440; }
      .os-diff-line-addition { background: #14532d; color: #86efac; }
      .os-diff-line-deletion { background: #7f1d1d; color: #fecaca; }
      .os-diff-line-context { color: #94a3b3; }
      .os-action-receipt { background: #111820; border-color: #2a3440; }
      .os-dependency-detail { background: #111820; border-color: #2a3440; color: #cbd5e1; }
      .os-node .os-node-dependency { color: #fbbf24; }

      button:hover:not(:disabled), .os-list-item:hover, .os-node:hover, .is-selected { background: #18303a; border-color: #5ca0b8; }
      .os-node-state-review { background: #451a03; color: #fcd34d; }
      .os-node-state-blocked, .os-node-state-failed { background: #451a1a; color: #fca5a5; }
      .os-node-state-running { background: #14532d; color: #86efac; }
      .os-node-state-done { background: #1e3a8a; color: #93c5fd; }
      .os-node-state-backlog { background: #1e293b; color: #cbd5e1; }
      .os-node-state-todo, .os-node-state-idle { background: #164e63; color: #a5f3fc; }
      .os-node-state-neutral { background: #111820; color: #cbd5e1; border-color: #2a3440; }
      .os-activity-entry { background: #111820; border-color: #2a3440; }
      .os-activity-lifecycle { background: #1f2a35; color: #94a3b3; }
      .os-activity-lifecycle span { color: #94a3b3; }
      .os-activity-lifecycle strong { color: #8bd0e6; }
      .os-activity-entry span { color: #94a3b3; }
      .os-activity-entry strong { color: #5ca0b8; }
      .os-activity-preview { color: #d9e2ea; }
      .os-activity-detail { background: #0c1116; border-color: #2a3440; color: #d9e2ea; }
      .os-pane-resizer span { background: #344454; }
      .os-pane-resizer:hover span, .os-pane-resizer:focus-visible span { background: #5ca0b8; }
      .os-knowledge-map { background: #111820; box-shadow: inset 0 0 0 1px rgba(92, 160, 184, 0.16); }
      .os-kg-node { background: #10232c; color: #8bd0e6; box-shadow: 0 0 0 1px rgba(92, 160, 184, 0.28), 0 10px 24px rgba(0, 0, 0, 0.18); }
      .os-kg-node-main { background: #14532d; color: #86efac; }
      .os-kg-edge { background: rgba(92, 160, 184, 0.34); }
      .os-knowledge-summary { background: #111820; box-shadow: inset 0 0 0 1px rgba(42, 52, 64, 0.9); }
      .os-knowledge-summary strong { color: #8bd0e6; }
      .os-knowledge-summary span, .os-knowledge-summary em { color: #94a3b3; }
      .os-badge-failed, .os-badge-blocked, .os-badge-blocker { background: #451a1a; color: #fca5a5; }
      .os-badge-running { background: #14532d; color: #86efac; }
      .os-badge-complete { background: #1e3a8a; color: #93c5fd; }
      .os-badge-stale { background: #451a03; color: #fcd34d; }
      .os-badge-queued, .os-badge-retry { background: #3b0764; color: #d8b4fe; }
      .os-badge-workspace, .os-badge-harness, .os-badge-diff_summary, .os-badge-validation { background: #1e293b; color: #cbd5e1; }
      pre { background: #0c1116; color: #d9e2ea; }
      .os-planning-panel, .os-plan-hierarchy-row, .os-plan-checklist-row, .os-plan-diff, .os-plan-graph, .os-conversation-message { background: #171d23; border-color: #2a3440; }
      .os-plan-hierarchy-row:hover, .os-plan-hierarchy-row.is-selected { background: #18303a; border-color: #5ca0b8; }
      .os-plan-diff-add { background: #14532d; color: #86efac; }
      .os-plan-diff-remove { background: #451a1a; color: #fca5a5; }
      .os-plan-graph-node rect { fill: #111820; stroke: #2a3440; }
      .os-plan-graph-node text { fill: #d9e2ea; }
      .os-plan-graph-node-sub { fill: #94a3b3; }
      .os-plan-graph-node-selected rect { fill: #18303a; stroke: #5ca0b8; }
    }
  `;
}
