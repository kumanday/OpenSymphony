import type {
  ActionDispatch,
  ActionReceipt,
  ChangedFileEntry,
  ConnectionProfile,
  DashboardSnapshot,
  FileDiffPage,
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
} from "@opensymphony/gateway-schema";
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
  listProfiles(): Promise<ModelConfigurationProfile[]>;
  storeProfile(profile: ModelConfigurationProfile): Promise<ModelConfigurationProfile>;
  setActiveProfile(profileId: string): Promise<ModelConfigurationProfile>;
  removeProfile(profileId: string): Promise<ModelConfigurationProfile[]>;
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
}

export interface OpenSymphonyAppHandle {
  refresh(): Promise<void>;
  destroy(): Promise<void>;
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
  runDetail: RunDetail | null;
  runFiles: ChangedFileEntry[] | null;
  selectedDiffPath: string | null;
  runDiff: FileDiffPage | null;
  evidenceView: "diff" | "activity";
  runEvents: RunEvent[] | null;
  expandedActivityEvents: Set<string>;
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

const schemaVersion = { major: 1, minor: 0, patch: 0 };

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
  private state: AppState;
  private destroyed = false;

  constructor(options: OpenSymphonyAppOptions) {
    this.options = options;
    this.transport = options.transport;
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
      runDetail: null,
      runFiles: null,
      selectedDiffPath: null,
      runDiff: null,
      evidenceView: "diff",
      runEvents: null,
      expandedActivityEvents: new Set(),
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
  }

  private async loadRunDetails(runId: string): Promise<void> {
    this.state.runFiles = null;
    this.state.runDiff = null;
    this.state.runEvents = null;
    this.state.expandedActivityEvents = new Set();
    this.state.runValidation = null;
    this.state.runApprovals = null;
    this.state.selectedDiffPath = null;
    this.state.evidenceView = "diff";
    try {
      this.state.runFiles = typeof this.transport.runFiles === "function"
        ? await this.transport.runFiles(runId)
        : [];
    } catch (error) {
      this.state.runFiles = [];
      this.state.connectionMessage = `Changed files unavailable: ${errorMessage(error)}`;
    }
    this.state.selectedDiffPath = this.state.runFiles[0]?.path ?? null;
    try {
      this.state.runDiff = this.state.selectedDiffPath
        && typeof this.transport.runDiffs === "function"
        ? await this.transport.runDiffs(runId, this.state.selectedDiffPath)
        : null;
    } catch (error) {
      this.state.runDiff = null;
      this.state.connectionMessage = `Diff unavailable: ${errorMessage(error)}`;
    }
    try {
      this.state.runEvents = typeof this.transport.runEvents === "function"
        ? (await this.transport.runEvents(runId)).events
        : [];
    } catch (error) {
      this.state.runEvents = [];
      this.state.connectionMessage = `Conversation activity unavailable: ${errorMessage(error)}`;
    }
    try {
      this.state.runValidation = typeof this.transport.runValidation === "function"
        ? await this.transport.runValidation(runId)
        : null;
    } catch (error) {
      this.state.runValidation = null;
      this.state.connectionMessage = `Validation summary unavailable: ${errorMessage(error)}`;
    }
    try {
      this.state.runApprovals = typeof this.transport.runApprovals === "function"
        ? await this.transport.runApprovals(runId)
        : [];
    } catch (error) {
      this.state.runApprovals = [];
      this.state.connectionMessage = `Approvals unavailable: ${errorMessage(error)}`;
    }
  }

  async refresh(): Promise<void> {
    if (this.destroyed) {
      return;
    }
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
        this.transport = await this.options.onGatewayUrlChanged(active.gatewayUrl);
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
      this.state.modelProfileError = null;
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
      this.state.selectedProjectId = snapshot.projects[0]?.project_id ?? "default";
      await this.loadTaskGraph(this.state.selectedProjectId);
      this.loadPlanningWorkspace(this.state.selectedProjectId);
      this.state.planningWorkspace = {
        ...this.state.planningWorkspace,
        project_id: this.state.selectedProjectId,
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
    this.state.runValidation = null;
    this.state.runApprovals = null;
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

  private async loadTaskGraph(projectId: string | null): Promise<void> {
    if (!projectId) {
      this.state.taskGraph = null;
      this.state.selectedNodeId = null;
      return;
    }
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
      this.state.runValidation = null;
      this.state.runApprovals = null;
      this.state.connectionMessage = `Task graph unavailable: ${errorMessage(error)}`;
      return;
    }

    this.state.taskGraph = taskGraph;
    const initialNode = initialSelectedTaskNode(taskGraph.nodes, taskGraph.root_ids);
    this.state.selectedNodeId = initialNode?.node_id ?? null;
    this.state.runDetail = null;
    this.state.runFiles = null;
    this.state.runDiff = null;
    this.state.evidenceView = "diff";
    this.state.runEvents = null;
    this.state.expandedActivityEvents = new Set();
    this.state.runValidation = null;
    this.state.runApprovals = null;
    this.state.selectedDiffPath = null;
    await this.loadRunOverlays(taskGraph);
    if (initialNode) {
      await this.openRun(initialNode);
    }
  }

  private async loadRunOverlays(taskGraph: TaskGraphSnapshot): Promise<void> {
    const runIds = new Set(taskGraph.nodes.map((node) => node.run_id).filter((id): id is string => Boolean(id)));
    if (runIds.size === 0) {
      this.state.runOverlays = new Map();
      return;
    }
    const overlays = new Map<string, RunDetail>();
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
    this.state.runOverlays = overlays;
  }

  private async openRun(node: TaskGraphNode): Promise<void> {
    const runId = node.run_id || node.identifier || node.node_id;
    this.state.selectedNodeId = node.node_id;
    this.state.loading = true;
    this.render();
    try {
      this.state.runDetail = await this.transport.runDetail(runId);
      this.state.runOverlays.set(runId, this.state.runDetail);
    } catch (error) {
      this.state.runDetail = null;
      this.state.runFiles = null;
      this.state.runDiff = null;
      this.state.evidenceView = "diff";
      this.state.runEvents = null;
      this.state.expandedActivityEvents = new Set();
      this.state.runValidation = null;
      this.state.runApprovals = null;
      this.state.connectionMessage = `Run ${runId} unavailable: ${errorMessage(error)}`;
      this.state.loading = false;
      this.render();
      return;
    }
    await this.loadRunDetails(runId);
    this.state.loading = false;
    this.render();
  }

  private async selectDiffFile(path: string): Promise<void> {
    this.state.selectedDiffPath = path;
    this.state.evidenceView = "diff";
    const runId = this.state.runDetail?.run_id;
    if (runId && typeof this.transport.runDiffs === "function") {
      try {
        this.state.runDiff = await this.transport.runDiffs!(runId, path);
      } catch (error) {
        this.state.runDiff = null;
        this.state.connectionMessage = `Diff unavailable: ${errorMessage(error)}`;
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

  private toggleActivityEvent(eventKey: string): void {
    const expanded = new Set(this.state.expandedActivityEvents);
    if (expanded.has(eventKey)) {
      expanded.delete(eventKey);
    } else {
      expanded.add(eventKey);
    }
    this.state.expandedActivityEvents = expanded;
    this.render();
  }

  private async dispatchRunAction(action: RunAction): Promise<void> {
    const runId = this.state.runDetail?.run_id;
    if (!runId) return;
    const transport = this.transport as unknown as {
      cancelRun?: (id: string) => Promise<ActionReceipt>;
      retryRun?: (id: string) => Promise<ActionReceipt>;
      rehydrateRun?: (id: string) => Promise<ActionReceipt>;
      resumeRun?: (id: string) => Promise<ActionReceipt>;
      commentRun?: (id: string, text: string) => Promise<ActionReceipt>;
      createFollowup?: (id: string, payload: unknown) => Promise<ActionReceipt>;
      openWorkspace?: (id: string) => Promise<ActionReceipt>;
      debugRun?: (id: string) => Promise<ActionReceipt>;
      dispatchAction?: (action: unknown) => Promise<ActionReceipt>;
    };
    let receipt: ActionReceipt | null = null;
    try {
      switch (action) {
        case "cancel":
          receipt = await (transport.cancelRun?.(runId) ?? unsupportedAction(action));
          break;
        case "retry":
          receipt = await (transport.retryRun?.(runId) ?? unsupportedAction(action));
          break;
        case "rehydrate":
          receipt = await (transport.rehydrateRun?.(runId) ?? unsupportedAction(action));
          break;
        case "resume":
          receipt = await (transport.resumeRun?.(runId) ?? unsupportedAction(action));
          break;
        case "detach":
          receipt = await (transport.dispatchAction?.({
            schema_version: schemaVersion,
            correlation_id: `detach-${runId}-${crypto.randomUUID()}`,
            action_kind: "transition_issue",
            target_entity: { entity_kind: "run", entity_id: runId },
            payload: { intent: "detach" },
          }) ?? unsupportedAction(action));
          break;
        case "comment":
          receipt = await (transport.commentRun?.(runId, "Operator comment") ?? unsupportedAction(action));
          break;
        case "create_followup":
          receipt = await (transport.createFollowup?.(runId, { title: "Follow-up from run" }) ?? unsupportedAction(action));
          break;
        case "open_workspace":
          receipt = await (transport.openWorkspace?.(runId) ?? unsupportedAction(action));
          break;
        case "debug":
          receipt = await (transport.debugRun?.(runId) ?? unsupportedAction(action));
          break;
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
      this.transport = await this.options.onGatewayUrlChanged(profile.gatewayUrl);
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
        this.transport = await this.options.onGatewayUrlChanged(saved.gatewayUrl);
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
        this.transport = await this.options.onGatewayUrlChanged(active.gatewayUrl);
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
    const title = this.options.title ?? "OpenSymphony";
    this.options.root.innerHTML = `
      <style>${appShellStyles()}</style>
      <main class="os-app" data-opensymphony-app-shell="mounted" data-mode="${this.options.mode}" data-auth-state="${this.state.authState}">
        <header class="os-topbar">
          <div>
            <h1>${escapeHtml(title)}</h1>
            <p>${escapeHtml(this.state.connectionMessage)}</p>
          </div>
          <div class="os-view-tabs">
            <button type="button" class="os-view-tab ${this.state.activeView === "dashboard" ? "os-view-tab-active" : ""}" data-plan-view="dashboard">Dashboard</button>
            <button type="button" class="os-view-tab ${this.state.activeView === "planning" ? "os-view-tab-active" : ""}" data-plan-view="planning">Planning</button>
          </div>
          <div class="os-status os-status-${this.state.connectionMode}">
            <span></span>${escapeHtml(statusLabel(this.state.connectionMode))}
          </div>
        </header>
        <section class="os-grid">
          ${this.renderViewContent()}
        </section>
      </main>
    `;
    this.bindEvents();
  }

  private renderViewContent(): string {
    if (this.state.authState !== "open") {
      return this.renderAuthPlaceholder();
    }
    if (this.state.activeView === "planning") {
      return `
        ${this.renderProfiles()}
        ${this.renderModelProfiles()}
        ${renderPlanningWorkspace(this.state.planningWorkspace, this.state.planningEdit)}
      `;
    }
    return `
      ${this.renderStatus()}
      ${this.renderProfiles()}
      ${this.renderModelProfiles()}
      ${this.renderTaskGraph()}
      ${this.renderRunDetail()}
      ${this.renderRunEvidence()}
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
        ${this.renderProfiles()}
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
        ${this.renderProfiles()}
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
      ${this.renderProfiles()}
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
    const summary = activeProfile
      ? `${activeProfile.label} • ${activeProfile.gatewayUrl}`
      : this.transport.baseUri;
    const toggleLabel = this.state.profilePanelExpanded ? "Collapse" : "Expand";
    const header = `
      <div class="os-section-head">
        <div>
          <h2>Connection</h2>
          <span>${escapeHtml(summary)}</span>
        </div>
        <button type="button" class="os-activity-toggle os-panel-toggle" data-toggle-settings="connection" aria-expanded="${this.state.profilePanelExpanded ? "true" : "false"}" aria-label="${toggleLabel} Connection settings" title="${toggleLabel} Connection settings">
          <span aria-hidden="true">${this.state.profilePanelExpanded ? "v" : ">"}</span>
        </button>
      </div>
    `;
    if (!this.state.profilePanelExpanded) {
      return `
        <section class="os-panel os-profile-panel os-panel-collapsed">
          ${header}
          <div class="os-meta">Transport: ${escapeHtml(capabilities)}</div>
        </section>
      `;
    }
    const canRemoveProfile = profiles.length > 1;
    return `
      <section class="os-panel os-profile-panel">
        ${header}
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
    `;
  }

  private renderModelProfiles(): string {
    const profiles = modelProfilesWithDefaults(this.state.modelProfiles);
    const active = activeModelProfile(profiles, this.state.activeModelProfileId)
      ?? profiles[0]
      ?? null;
    if (!active) {
      return `
        <section class="os-panel os-model-panel os-panel-collapsed" data-testid="model-profile-panel">
          <div class="os-section-head">
            <div>
              <h2>Model Configuration</h2>
              <span>No model profiles</span>
            </div>
          </div>
        </section>
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
    const credentialLabel = active.mode === "subscription" ? "OpenHands Auth Directory Env" : "API Key Secret";
    const credentialInputType = active.mode === "subscription" ? "text" : "password";
    const modelProfileError = this.state.modelProfileError
      ? `<div class="os-model-error" role="alert" data-testid="model-profile-error">${escapeHtml(this.state.modelProfileError)}</div>`
      : "";
    const canRemoveProfile = profiles.length > 1;
    const summary = `${active.label} • ${active.model || "No model"}${active.baseUrl ? ` • ${active.baseUrl}` : ""}`;
    const toggleLabel = this.state.modelPanelExpanded ? "Collapse" : "Expand";
    const header = `
      <div class="os-section-head">
        <div>
          <h2>Model Configuration</h2>
          <span>${escapeHtml(summary)}</span>
        </div>
        <button type="button" class="os-activity-toggle os-panel-toggle" data-toggle-settings="model" aria-expanded="${this.state.modelPanelExpanded ? "true" : "false"}" aria-label="${toggleLabel} Model Configuration settings" title="${toggleLabel} Model Configuration settings">
          <span aria-hidden="true">${this.state.modelPanelExpanded ? "v" : ">"}</span>
        </button>
      </div>
    `;
    if (!this.state.modelPanelExpanded) {
      return `
        <section class="os-panel os-model-panel os-panel-collapsed" data-testid="model-profile-panel">
          ${header}
          <div class="os-model-meta" data-testid="model-redacted-credential">
            Auth: ${escapeHtml(redactCredentialRef(credentialRef))}
          </div>
          ${modelProfileError}
        </section>
      `;
    }
    return `
      <section class="os-panel os-model-panel" data-testid="model-profile-panel">
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
          Auth: ${escapeHtml(redactCredentialRef(credentialRef))}
        </div>
        ${modelProfileError}
      </section>
    `;
  }

  private renderStatus(): string {
    const snapshot = this.state.snapshot;
    if (!snapshot) {
      return panel("Status", `<div class="os-empty">Loading status</div>`, "os-status-panel");
    }
    const events = snapshot.recent_events.slice(0, 3).map((event) => `
      <li>
        <time class="os-event-time" datetime="${escapeAttr(event.happened_at)}">${escapeHtml(formatEventTime(event.happened_at))}</time>
        <span>${escapeHtml(event.kind)}</span>
        <strong>${escapeHtml(event.issue_identifier ?? "system")}</strong>
        ${escapeHtml(event.summary)}
      </li>
    `).join("");
    return panel(
      "Status",
      `
        <div class="os-metrics">
          <div><strong>${snapshot.metrics.running_issue_count}</strong><span>Running</span></div>
          <div><strong>${snapshot.metrics.retry_queue_depth}</strong><span>Retry Queue</span></div>
          <div><strong>${formatNumber(snapshot.metrics.total_input_tokens + snapshot.metrics.total_output_tokens)}</strong><span>Tokens</span></div>
        </div>
        <ol class="os-events">${events || `<li>No recent events</li>`}</ol>
      `,
      "os-status-panel",
    );
  }

  private renderTaskGraph(): string {
    const taskGraph = this.state.taskGraph;
    if (!taskGraph) {
      return panel("Task Graph", `<div class="os-empty">No task graph loaded</div>`, "os-task-graph-panel");
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
    const filtered = filterTaskGraphNodes(taskGraph.nodes, this.state.taskGraphFilter, getOverlay);
    if (this.options.mode === "web") {
      return this.renderEditableTaskGraph(taskGraph, filtered, getOverlay);
    }
    const dependencySignals = buildDependencySignals(taskGraph.nodes, filtered);
    const graph = renderTaskGraphVisualization(
      filtered,
      this.state.selectedNodeId,
      getOverlay,
      dependencySignals,
    );

    const filters = renderTaskGraphFilters(this.state.taskGraphFilter);

    return panel(
      "Task Graph",
      `${filters}${graph}`,
      "os-task-graph-panel",
    );
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
    const validation = this.state.runValidation
      ? renderValidationSummary(this.state.runValidation)
      : "";
    const approvals = this.state.runApprovals
      ? renderApprovalList(this.state.runApprovals, {
          onDecide: (id, decision, explanation) => {
            void this.submitApprovalDecision(id, decision, explanation);
          },
        })
      : "";
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
          <div><span>Phase</span><strong>${escapeHtml(phase)}</strong></div>
          <div><span>Stream</span><strong>${escapeHtml(stream)}</strong></div>
          <div><span>Turns</span><strong>${run.turn_count} / ${run.max_turns}</strong></div>
          <div><span>Runtime</span><strong>${run.runtime_seconds}s</strong></div>
          ${run.diagnostics?.cancel_acknowledged ? `<div><span>Cancel</span><strong class="os-cancel-acknowledged" data-testid="cancel-acknowledged">acknowledged</strong></div>` : ""}
          ${run.diagnostics?.cancel_failed ? `<div><span>Cancel</span><strong class="os-cancel-failed" data-testid="cancel-failed">failed</strong></div>` : ""}
        </div>
        ${actionBar}
        ${receipt}
        <div class="os-run-section">
          <h3>Changed Files</h3>
          ${files}
        </div>
        <div class="os-run-panels">
          <div class="os-validation-panel">${validation}</div>
          <div class="os-approval-panel">${approvals}</div>
        </div>
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
    const activity = renderRunActivity(this.state.runEvents, this.state.expandedActivityEvents);
    const showingDiff = this.state.evidenceView === "diff";
    const content = showingDiff
      ? diff || `<div class="os-empty">Select a changed file to view its diff</div>`
      : activity;
    return panel(
      "Inspector",
      `
        <div class="os-segmented" data-testid="evidence-toggle">
          <button type="button" class="${showingDiff ? "is-selected" : ""}" data-evidence-view="diff">Diff</button>
          <button type="button" class="${!showingDiff ? "is-selected" : ""}" data-evidence-view="activity">Activity</button>
        </div>
        <div class="os-run-section">
          <h3>${showingDiff ? "Selected Diff" : "Conversation Activity"}</h3>
          ${content}
        </div>
      `,
      "os-run-evidence-panel",
    );
  }

  private bindEvents(): void {
    this.options.root.querySelector("[data-save-profile]")?.addEventListener("click", () => {
      void this.saveProfile();
    });
    this.options.root.querySelector("[data-new-profile]")?.addEventListener("click", () => {
      void this.createProfileDraft();
    });
    this.options.root.querySelector("[data-remove-profile]")?.addEventListener("click", () => {
      void this.removeProfile();
    });
    this.options.root.querySelector("[data-save-model-profile]")?.addEventListener("click", () => {
      void this.saveModelProfile();
    });
    this.options.root.querySelector("[data-new-model-profile]")?.addEventListener("click", () => {
      void this.createModelProfileDraft();
    });
    this.options.root.querySelector("[data-remove-model-profile]")?.addEventListener("click", () => {
      void this.removeModelProfile();
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-toggle-settings]").forEach((button) => {
      button.addEventListener("click", () => {
        const panel = button.dataset.toggleSettings;
        if (panel === "connection" || panel === "model") {
          this.toggleSettingsPanel(panel);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-auth-action]").forEach((button) => {
      button.addEventListener("click", () => {
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
    this.options.root.querySelector("[data-profile-select]")?.addEventListener("change", (event) => {
      const target = event.target as HTMLSelectElement;
      void this.selectProfile(target.value);
    });
    this.options.root.querySelector("[data-model-profile-select]")?.addEventListener("change", (event) => {
      const target = event.target as HTMLSelectElement;
      void this.selectModelProfile(target.value);
    });
    this.options.root.querySelector("[data-model-mode]")?.addEventListener("change", (event) => {
      const target = event.target as HTMLSelectElement;
      this.changeModelProfileMode(modelModeFromValue(target.value));
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-project-id]").forEach((button) => {
      button.addEventListener("click", () => {
        const projectId = button.dataset.projectId;
        if (projectId) {
          void this.selectProject(projectId);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-node-id]").forEach((button) => {
      button.addEventListener("click", (event) => {
        // Avoid selecting when clicking inline editor controls or action buttons.
        const target = event.target as HTMLElement;
        if (target.closest(".os-node-actions, .os-inline-input, .os-node-badges")) {
          return;
        }
        const node = this.state.taskGraph?.nodes.find(
          (candidate) => candidate.node_id === button.dataset.nodeId,
        );
        if (node) {
          if (this.options.mode === "desktop") {
            void this.openRun(node);
          } else {
            this.state.selectedNodeId = node.node_id;
            this.render();
          }
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-open-run]").forEach((button) => {
      button.addEventListener("click", () => {
        const node = this.state.taskGraph?.nodes.find(
          (candidate) => candidate.node_id === button.dataset.openRun,
        );
        if (node) {
          void this.openRun(node);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-testid='changed-file-item']").forEach((button) => {
      button.addEventListener("click", () => {
        const path = button.dataset.path;
        if (path) {
          void this.selectDiffFile(path);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-evidence-view]").forEach((button) => {
      button.addEventListener("click", () => {
        const view = button.dataset.evidenceView;
        if (view === "diff" || view === "activity") {
          this.selectEvidenceView(view);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-activity-toggle]").forEach((button) => {
      button.addEventListener("click", () => {
        const eventKey = button.dataset.activityToggle;
        if (eventKey) {
          this.toggleActivityEvent(eventKey);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-testid='run-action-button']").forEach((button) => {
      button.addEventListener("click", () => {
        const action = button.dataset.action as RunAction | undefined;
        if (action) {
          void this.dispatchRunAction(action);
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-testid='approve-button']").forEach((button) => {
      button.addEventListener("click", () => {
        const approvalId = button.dataset.approvalId;
        if (!approvalId) return;
        const container = button.closest("[data-testid='approval-item']");
        const explanation = container?.querySelector<HTMLInputElement>("[data-testid='approval-explanation']")?.value;
        void this.submitApprovalDecision(approvalId, "approved", explanation);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-testid='deny-button']").forEach((button) => {
      button.addEventListener("click", () => {
        const approvalId = button.dataset.approvalId;
        if (!approvalId) return;
        const container = button.closest("[data-testid='approval-item']");
        const explanation = container?.querySelector<HTMLInputElement>("[data-testid='approval-explanation']")?.value;
        void this.submitApprovalDecision(approvalId, "rejected", explanation);
      });
    });


    // Task graph filters
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-filter]").forEach((control) => {
      control.addEventListener("change", () => this.onFilterChange());
      control.addEventListener("input", () => this.onFilterChange());
    });
    this.options.root.querySelector("[data-tg-filter-reset]")?.addEventListener("click", () => {
      this.state.taskGraphFilter = { ...defaultTaskGraphFilter };
      this.render();
    });

    this.options.root.querySelectorAll<HTMLElement>("[data-tg-create]").forEach((button) => {
      button.addEventListener("click", () => {
        const kind = button.dataset.tgCreate as TaskGraphNodeKind;
        this.openCreateDialog(kind, null);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-create-child]").forEach((button) => {
      button.addEventListener("click", () => {
        const parentId = button.dataset.tgCreateChild;
        if (!parentId) return;
        const parent = this.state.taskGraph?.nodes.find((node) => node.node_id === parentId);
        if (!parent) return;
        const childKind: TaskGraphNodeKind = parent.kind === "milestone" ? "issue" : "sub_issue";
        this.openCreateDialog(childKind, parentId);
      });
    });
    this.options.root.querySelector("[data-tg-create-save]")?.addEventListener("click", () => {
      void this.saveCreateDialog();
    });
    this.options.root.querySelector("[data-tg-create-cancel]")?.addEventListener("click", () => {
      this.state.createDialog = { ...emptyEditorDialog };
      this.render();
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-edit]").forEach((button) => {
      button.addEventListener("click", () => {
        const nodeId = button.dataset.tgEdit;
        if (nodeId) this.startInlineEdit(nodeId);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-inline-save]").forEach((button) => {
      button.addEventListener("click", () => {
        const nodeId = button.dataset.tgInlineSave;
        if (nodeId) void this.saveInlineEdit(nodeId);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-inline-cancel]").forEach((button) => {
      button.addEventListener("click", () => {
        this.state.inlineEdit = { ...emptyInlineEdit };
        this.render();
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-deps]").forEach((button) => {
      button.addEventListener("click", () => {
        const nodeId = button.dataset.tgDeps;
        if (nodeId) this.openDependencyEditor(nodeId);
      });
    });
    this.options.root.querySelector("[data-tg-deps-save]")?.addEventListener("click", () => {
      void this.saveDependencyEdit();
    });
    this.options.root.querySelector("[data-tg-deps-cancel]")?.addEventListener("click", () => {
      this.state.dependencyEdit = { ...emptyDependencyEdit };
      this.render();
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-comment]").forEach((button) => {
      button.addEventListener("click", () => {
        const nodeId = button.dataset.tgComment;
        if (nodeId) this.openCommentEditor(nodeId);
      });
    });
    this.options.root.querySelector("[data-tg-comment-save]")?.addEventListener("click", () => {
      void this.saveCommentEdit();
    });
    this.options.root.querySelector("[data-tg-comment-cancel]")?.addEventListener("click", () => {
      this.state.commentEdit = { ...emptyCommentEdit };
      this.render();
    });

    // Planning workspace view navigation
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-view]").forEach((button) => {
      button.addEventListener("click", () => {
        const view = button.dataset.planView as AppState["activeView"];
        if (view) {
          this.state.activeView = view;
          this.render();
        }
      });
    });

    // Planning workspace tabs
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-tab]").forEach((button) => {
      button.addEventListener("click", () => {
        const tab = button.dataset.planTab;
        if (!tab) return;
        this.state.planningWorkspace = { ...this.state.planningWorkspace, activeTab: tab as typeof this.state.planningWorkspace.activeTab };
        this.render();
      });
    });

    // Planning conversation
    this.options.root.querySelector("[data-plan-send-message]")?.addEventListener("click", () => {
      this.sendPlanMessage();
    });
    this.options.root.querySelector("[data-plan-composer]")?.addEventListener("keydown", (event) => {
      if ((event as KeyboardEvent).key === "Enter" && !(event as KeyboardEvent).shiftKey) {
        (event as KeyboardEvent).preventDefault();
        this.sendPlanMessage();
      }
    });
    this.options.root.querySelector("[data-plan-composer]")?.addEventListener("input", () => {
      this.state.planningWorkspace.composerDraft = this.options.root.querySelector<HTMLTextAreaElement>("[data-plan-composer]")?.value ?? "";
    });

    // Planning artifact editor
    this.options.root.querySelector("[data-plan-artifact-select]")?.addEventListener("change", () => {
      const artifactId = this.options.root.querySelector<HTMLSelectElement>("[data-plan-artifact-select]")?.value ?? null;
      this.state.planningWorkspace = selectArtifact(this.state.planningWorkspace, artifactId);
      this.renderPreservingFocus();
    });
    this.options.root.querySelector("[data-plan-revision-select]")?.addEventListener("change", () => {
      const revisionId = this.options.root.querySelector<HTMLSelectElement>("[data-plan-revision-select]")?.value ?? null;
      this.state.planningWorkspace = selectRevision(this.state.planningWorkspace, revisionId);
      this.renderPreservingFocus();
    });
    this.options.root.querySelector("[data-plan-save-artifact]")?.addEventListener("click", () => {
      this.savePlanArtifact();
    });
    this.options.root.querySelector("[data-plan-add-artifact]")?.addEventListener("click", () => {
      this.addPlanArtifact();
    });
    this.options.root.querySelector("[data-plan-artifact-content]")?.addEventListener("input", () => {
      // Content is not persisted continuously; only saved on explicit save.
    });

    // Planning hierarchy editor
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-node-select]").forEach((row) => {
      row.addEventListener("click", (event) => {
        if ((event.target as HTMLElement).closest(".os-node-actions, .os-plan-toggle, .os-plan-node-body input")) return;
        const nodeId = row.dataset.planNodeSelect;
        if (nodeId) {
          this.state.planningWorkspace = { ...this.state.planningWorkspace, selectedNodeId: nodeId };
          this.render();
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-node-toggle]").forEach((button) => {
      button.addEventListener("click", () => {
        const nodeId = button.dataset.planNodeToggle;
        if (nodeId) {
          this.state.planningWorkspace = toggleNodeExpanded(this.state.planningWorkspace, nodeId);
          this.render();
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-add-node]").forEach((button) => {
      button.addEventListener("click", () => {
        const kind = button.dataset.planAddNode as "milestone" | "issue" | "sub_issue";
        this.addPlanNode(kind, null);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-add-child]").forEach((button) => {
      button.addEventListener("click", () => {
        const parentId = button.dataset.planAddChild;
        if (!parentId) return;
        const parent = this.state.planningWorkspace.nodes.find((n) => n.node_id === parentId);
        if (!parent) return;
        const childKind: "milestone" | "issue" | "sub_issue" = parent.kind === "milestone" ? "issue" : "sub_issue";
        this.addPlanNode(childKind, parentId);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-node-edit]").forEach((button) => {
      button.addEventListener("click", () => {
        const nodeId = button.dataset.planNodeEdit;
        if (!nodeId) return;
        const node = this.state.planningWorkspace.nodes.find((n) => n.node_id === nodeId);
        if (!node) return;
        this.state.planningEdit = { nodeId, title: node.title, state: node.state };
        this.render();
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-node-save]").forEach((button) => {
      button.addEventListener("click", () => {
        const nodeId = button.dataset.planNodeSave;
        if (nodeId) this.savePlanNodeEdit(nodeId);
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-node-cancel]").forEach((button) => {
      button.addEventListener("click", () => {
        this.state.planningEdit = { ...emptyPlanningEditState };
        this.render();
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-remove-node]").forEach((button) => {
      button.addEventListener("click", () => {
        const nodeId = button.dataset.planRemoveNode;
        if (nodeId) {
          this.state.planningWorkspace = removePlanningNode(this.state.planningWorkspace, nodeId);
          this.render();
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-graph-node]").forEach((node) => {
      node.addEventListener("click", () => {
        const nodeId = node.dataset.planGraphNode;
        if (nodeId) {
          this.state.planningWorkspace = { ...this.state.planningWorkspace, selectedNodeId: nodeId };
          this.render();
        }
      });
    });

    // Planning dependency editor
    this.options.root.querySelector("[data-plan-deps-node-select]")?.addEventListener("change", () => {
      const nodeId = this.options.root.querySelector<HTMLSelectElement>("[data-plan-deps-node-select]")?.value ?? null;
      this.state.planningWorkspace = { ...this.state.planningWorkspace, selectedNodeId: nodeId };
      this.renderPreservingFocus();
    });
    this.options.root.querySelector("[data-plan-deps-save]")?.addEventListener("click", () => {
      this.savePlanDependencies();
    });

    // Planning acceptance criteria / verification editor
    this.options.root.querySelector("[data-plan-criteria-add]")?.addEventListener("click", () => {
      const text = this.options.root.querySelector<HTMLInputElement>("[data-plan-criteria-new]")?.value ?? "";
      if (!text.trim()) return;
      this.state.planningWorkspace = addCriterion(this.state.planningWorkspace, text);
      this.render();
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-criteria-toggle]").forEach((checkbox) => {
      checkbox.addEventListener("change", () => {
        const id = checkbox.dataset.planCriteriaToggle;
        if (id) {
          this.state.planningWorkspace = toggleCriterion(this.state.planningWorkspace, id);
          this.render();
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-criteria-text]").forEach((input) => {
      input.addEventListener("input", () => {
        const id = input.dataset.planCriteriaText;
        const value = (input as HTMLInputElement).value;
        if (id) {
          this.state.planningWorkspace = updateCriterion(this.state.planningWorkspace, id, value);
          this.renderPreservingFocus();
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-criteria-remove]").forEach((button) => {
      button.addEventListener("click", () => {
        const id = button.dataset.planCriteriaRemove;
        if (id) {
          this.state.planningWorkspace = removeCriterion(this.state.planningWorkspace, id);
          this.render();
        }
      });
    });
    this.options.root.querySelector("[data-plan-verification-add]")?.addEventListener("click", () => {
      const text = this.options.root.querySelector<HTMLInputElement>("[data-plan-verification-new]")?.value ?? "";
      if (!text.trim()) return;
      this.state.planningWorkspace = addVerification(this.state.planningWorkspace, text);
      this.render();
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-verification-toggle]").forEach((checkbox) => {
      checkbox.addEventListener("change", () => {
        const id = checkbox.dataset.planVerificationToggle;
        if (id) {
          this.state.planningWorkspace = toggleVerification(this.state.planningWorkspace, id);
          this.render();
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-verification-text]").forEach((input) => {
      input.addEventListener("input", () => {
        const id = input.dataset.planVerificationText;
        const value = (input as HTMLInputElement).value;
        if (id) {
          this.state.planningWorkspace = updateVerification(this.state.planningWorkspace, id, value);
          this.renderPreservingFocus();
        }
      });
    });
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-verification-remove]").forEach((button) => {
      button.addEventListener("click", () => {
        const id = button.dataset.planVerificationRemove;
        if (id) {
          this.state.planningWorkspace = removeVerification(this.state.planningWorkspace, id);
          this.render();
        }
      });
    });

    // Planning validation links
    this.options.root.querySelectorAll<HTMLElement>("[data-plan-validation-link]").forEach((link) => {
      link.addEventListener("click", () => {
        this.followPlanValidationLink(link);
      });
    });

    // Planning diff revision selectors
    this.options.root.querySelector("[data-plan-diff-left]")?.addEventListener("change", () => {
      this.state.planningWorkspace = {
        ...this.state.planningWorkspace,
        diffLeftRevisionId: this.options.root.querySelector<HTMLSelectElement>("[data-plan-diff-left]")?.value ?? null,
      };
      this.renderPreservingFocus();
    });
    this.options.root.querySelector("[data-plan-diff-right]")?.addEventListener("change", () => {
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
    const kind = (root.querySelector<HTMLSelectElement>("[data-tg-filter='kind']")?.value ?? "all") as TaskGraphFilter["kind"];
    const runtime = (root.querySelector<HTMLSelectElement>("[data-tg-filter='runtime']")?.value ?? "all") as TaskGraphFilter["runtime"];
    const state = (root.querySelector<HTMLSelectElement>("[data-tg-filter='state']")?.value ?? "all") as TaskGraphFilter["stateCategory"];
    const search = root.querySelector<HTMLInputElement>("[data-tg-filter='search']")?.value ?? "";
    this.state.taskGraphFilter = { kind, runtime, stateCategory: state, search };
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

function renderRunActivity(events: RunEvent[] | null, expandedActivityEvents: Set<string>): string {
  if (events === null) {
    return `<div class="os-run-activity os-empty" data-testid="run-activity">Loading conversation activity</div>`;
  }
  if (events.length === 0) {
    return `<div class="os-run-activity os-empty" data-testid="run-activity">No recent activity</div>`;
  }
  const items = sortEventsNewestFirst(events)
    .map((event) => {
      const eventKey = activityEventKey(event);
      return `
      <div class="os-activity-entry os-activity-entry-${escapeAttr(activityClassName(event.kind))}" data-testid="run-activity-entry" data-event-kind="${escapeAttr(event.kind)}" data-event-id="${escapeAttr(event.event_id)}">
        ${renderActivityEvent(event, eventKey, expandedActivityEvents.has(eventKey))}
      </div>
    `;
    })
    .join("");
  return `<div class="os-run-activity" data-testid="run-activity">${items}</div>`;
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

function formatNumber(value: number): string {
  return new Intl.NumberFormat("en-US", { notation: "compact" }).format(value);
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
}

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
): string {
  if (nodes.length === 0) {
    return `<div class="os-empty">No tasks match the current filters</div>`;
  }
  const models = buildTaskGraphRenderModels(nodes, signals);
  const links = buildTaskGraphLinks(models);
  const rowHeight = 62;
  const rowGap = 8;
  const laneWidth = 34;
  const railX = 20;
  const graphHeight = models.length * rowHeight + Math.max(0, models.length - 1) * rowGap;
  const maxLane = models.reduce((max, model) => Math.max(max, model.lane), 0);
  const graphWidth = Math.max(620, 360 + maxLane * laneWidth);
  const svgLinks = links.map((link) => renderTaskGraphLink(link, rowHeight, rowGap, laneWidth, railX)).join("");
  const renderedNodes = models.map((model) => renderReadOnlyTaskGraphNode(
    model,
    selectedNodeId,
    getOverlay(model.node),
  )).join("");

  return `
    <div class="os-task-graph-stage" data-testid="task-graph-visualization" style="--os-graph-height: ${graphHeight}px; --os-graph-width: ${graphWidth}px;">
      <svg class="os-task-graph-links" data-testid="task-graph-links" viewBox="0 0 ${graphWidth} ${graphHeight}" preserveAspectRatio="none" aria-hidden="true">
        <defs>
          <marker id="os-task-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto">
            <path d="M 0 0 L 8 4 L 0 8 z"></path>
          </marker>
        </defs>
        ${svgLinks}
      </svg>
      <div class="os-node-list os-node-graph-list" style="min-height: ${graphHeight}px;">${renderedNodes}</div>
    </div>
  `;
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
  const links: TaskGraphLink[] = [];
  for (const model of models) {
    for (const upstream of model.signal.upstreamVisible) {
      const from = byId.get(upstream.node_id);
      if (from) {
        const span = Math.abs(model.index - from.index);
        links.push({
          from,
          to: model,
          span,
          routeLane: Math.max(0, Math.min(3, span - 2)),
        });
      }
    }
  }
  return links;
}

function renderTaskGraphLink(
  link: TaskGraphLink,
  rowHeight: number,
  rowGap: number,
  laneWidth: number,
  railX: number,
): string {
  const x1 = railX + link.from.lane * laneWidth;
  const x2 = railX + link.to.lane * laneWidth;
  const y1 = link.from.index * (rowHeight + rowGap) + rowHeight / 2;
  const y2 = link.to.index * (rowHeight + rowGap) + rowHeight / 2;
  if (link.span > 1) {
    const routeX = Math.max(4, railX - 9 - link.routeLane * 7);
    const d = `M ${x1} ${y1} H ${routeX} V ${y2} H ${x2}`;
    return `<path class="os-task-graph-link os-task-graph-link-skip" data-testid="task-graph-link" d="${escapeAttr(d)}"></path>`;
  }
  const bend = Math.max(34, Math.abs(y2 - y1) * 0.24);
  const d = `M ${x1} ${y1} C ${x1 + bend} ${y1}, ${Math.max(x2 - bend, x1 + 18)} ${y2}, ${x2} ${y2}`;
  return `<path class="os-task-graph-link" data-testid="task-graph-link" d="${escapeAttr(d)}"></path>`;
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
  const overlayBadges = overlay.badges.length ? overlay.badges.map(renderBadge).join("") : "";
  const runMeta = overlay.run_id
    ? `<span class="os-run-meta">run ${escapeHtml(overlay.run_id)}</span>`
    : "";
  const stateTone = stateToneForTaskNode(node);
  const hasUpstream = signal.upstreamVisible.length > 0 || signal.upstreamHiddenCount > 0;
  const hasDownstream = signal.downstreamVisible.length > 0 || signal.downstreamHiddenCount > 0;
  const dependencyMeta = signal.suffix
    ? `<span class="os-node-dependency" data-testid="dependency-suffix">${escapeHtml(signal.suffix)}</span>`
    : "";
  const dependencyGlyph = hasUpstream && hasDownstream
    ? "<>"
    : hasUpstream
      ? "<"
      : hasDownstream
        ? ">"
        : "";

  return `
    <button type="button" class="os-node os-node-readonly ${isSelected ? "is-selected" : ""} ${hasUpstream ? "os-node-has-upstream" : ""} ${hasDownstream ? "os-node-has-downstream" : ""}" data-node-id="${escapeAttr(node.node_id)}" style="--os-lane: ${model.lane};">
      <span class="os-node-gutter" aria-hidden="true">${escapeHtml(dependencyGlyph)}</span>
      <span class="os-node-main">
        <span class="os-node-line">
          <strong>${escapeHtml(node.identifier)}</strong>
          <span>${escapeHtml(node.title)}</span>
          ${dependencyMeta}
        </span>
        <span class="os-node-subline">
          <span class="os-node-kind">${escapeHtml(node.kind.replace(/_/g, " "))}</span>
          <em class="os-node-state os-node-state-${escapeAttr(stateTone)}">${escapeHtml(node.state)}</em>
          <span class="os-node-badges">${overlayBadges}</span>
          ${runMeta}
        </span>
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
  return ordered.find((node) => node.kind !== "milestone" && node.state_category === "in_progress")
    ?? ordered.find((node) => node.kind !== "milestone" && node.run_id)
    ?? ordered.find((node) => node.kind !== "milestone")
    ?? ordered[0]
    ?? null;
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

function appShellStyles(): string {
  return `
    :root { color-scheme: light dark; }
    body { margin: 0; background: #f4f6f8; color: #17202a; font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
    .os-app { min-height: 100vh; display: flex; flex-direction: column; }
    .os-topbar { display: flex; align-items: center; justify-content: space-between; gap: 24px; padding: 18px 22px; background: #ffffff; border-bottom: 1px solid #d8dee4; }
    .os-topbar h1 { margin: 0; font-size: 18px; line-height: 1.2; letter-spacing: 0; }
    .os-topbar p { margin: 5px 0 0; color: #5d6b78; font-size: 13px; }
    .os-status { display: inline-flex; align-items: center; gap: 8px; border: 1px solid #cad3dd; border-radius: 6px; padding: 7px 10px; background: #f8fafc; font-size: 13px; white-space: nowrap; }
    .os-status span { width: 9px; height: 9px; border-radius: 50%; background: #6b7280; }
    .os-status-connected span { background: #1f9d55; }
    .os-status-failed span { background: #c2410c; }
    .os-grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 14px; padding: 14px; align-items: start; }
    .os-panel { background: #ffffff; border: 1px solid #d8dee4; border-radius: 8px; padding: 14px; min-width: 0; box-shadow: 0 1px 2px rgba(15, 23, 42, 0.05); }
    .os-status-panel, .os-run-detail-panel, .os-run-evidence-panel { grid-column: span 1; }
    .os-profile-panel { grid-column: span 3; }
    .os-model-panel { grid-column: 1 / -1; }
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
    .os-metrics, .os-run-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 9px; margin-bottom: 12px; }
    .os-metrics div, .os-run-grid div { border: 1px solid #d8dee4; border-radius: 6px; padding: 10px; background: #f8fafc; }
    .os-metrics strong, .os-run-grid strong { display: block; font-size: 18px; }
    .os-metrics span, .os-run-grid span { display: block; color: #667788; font-size: 12px; margin-top: 3px; }
    .os-list, .os-node-list { display: grid; gap: 8px; }
    .os-list-item, .os-node { width: 100%; text-align: left; display: grid; gap: 3px; padding: 10px; background: #ffffff; }
    .os-task-graph-stage { position: relative; min-width: min(100%, var(--os-graph-width)); overflow-x: auto; padding: 2px 0; }
    .os-task-graph-links { position: absolute; z-index: 3; inset: 0 auto auto 0; width: var(--os-graph-width); height: var(--os-graph-height); pointer-events: none; overflow: visible; }
    .os-task-graph-link { fill: none; stroke: #39708f; stroke-width: 1.9; stroke-linecap: round; stroke-linejoin: round; opacity: 0.9; marker-end: url(#os-task-arrow); }
    .os-task-graph-link-skip { opacity: 0.78; }
    .os-task-graph-links marker path { fill: #39708f; }
    .os-node-graph-list { position: relative; z-index: 1; min-width: var(--os-graph-width); gap: 8px; }
    .os-node-readonly { grid-template-columns: 28px minmax(0, 1fr); align-items: center; min-height: 62px; margin-left: calc(var(--os-lane, 0) * 34px); margin-right: 8px; padding: 8px 10px; border-radius: 8px; font-size: 12px; transition-property: background-color, border-color, box-shadow, transform; transition-duration: 150ms; transition-timing-function: ease-out; }
    .os-node-readonly:active { transform: scale(0.996); }
    .os-node-gutter { width: 22px; height: 22px; display: inline-flex; align-items: center; justify-content: center; border-radius: 999px; font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; color: #39708f; font-size: 11px; font-weight: 800; white-space: pre; background: #e7f1f5; box-shadow: 0 0 0 1px rgba(57, 112, 143, 0.28); }
    .os-node-gutter:empty { background: transparent; box-shadow: 0 0 0 1px rgba(57, 112, 143, 0.18); }
    .os-node-has-upstream .os-node-gutter { background: #fff7ed; color: #92400e; box-shadow: 0 0 0 1px rgba(146, 64, 14, 0.32); }
    .os-node-has-downstream .os-node-gutter { background: #e7f1f5; color: #23566f; box-shadow: 0 0 0 1px rgba(57, 112, 143, 0.34); }
    .os-node-has-upstream.os-node-has-downstream .os-node-gutter { background: #fef3c7; color: #78350f; box-shadow: 0 0 0 1px rgba(146, 64, 14, 0.38); }
    .os-node-main, .os-node-line, .os-node-subline { min-width: 0; }
    .os-node-line { display: flex; gap: 8px; align-items: baseline; flex-wrap: nowrap; }
    .os-node-line > span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .os-node-line strong { flex: 0 0 auto; font-size: 12px; font-variant-numeric: tabular-nums; }
    .os-node-dependency { flex: 0 1 auto; color: #92400e; font-size: 11px; white-space: nowrap; }
    .os-node-subline { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; margin-top: 4px; }
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
    .os-event-time { color: #667788; font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; font-size: 11px; margin-right: 6px; }
    .os-pill, .os-actions span { border-radius: 999px; background: #e7f1f5; color: #23566f; padding: 5px 9px; font-size: 12px; }
    .os-actions { display: flex; flex-wrap: wrap; gap: 6px; margin: 12px 0; }
    .os-run-detail-panel { font-size: 12px; }
    .os-run-detail-panel .os-section-head h2 { font-size: 14px; }
    .os-run-detail-panel button { min-height: 30px; padding: 5px 8px; font-size: 12px; }
    .os-run-detail-panel .os-run-head { padding: 8px 10px; margin-bottom: 8px; }
    .os-run-detail-panel .os-run-head strong { font-size: 14px; }
    .os-run-detail-panel .os-run-grid { gap: 8px; margin-bottom: 8px; }
    .os-run-detail-panel .os-run-grid div { padding: 8px; }
    .os-run-detail-panel .os-run-grid strong { font-size: 15px; }
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
    .os-activity-entry { display: grid; gap: 7px; align-items: start; border: 1px solid #d8dee4; border-radius: 6px; padding: 8px; background: #f8fafc; font-size: 12px; }
    .os-activity-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px; align-items: center; min-width: 0; }
    .os-activity-meta { display: flex; gap: 6px; align-items: baseline; min-width: 0; overflow: hidden; }
    .os-activity-entry span { color: #667788; white-space: nowrap; }
    .os-activity-entry strong { color: #39708f; white-space: nowrap; }
    .os-activity-preview { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: #27313a; }
    .os-activity-toggle { width: 24px; height: 24px; min-height: 24px; padding: 0; border-radius: 999px; display: inline-grid; place-items: center; font-size: 13px; line-height: 1; }
    .os-activity-detail { margin: 0; max-height: 260px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; border: 1px solid #d8dee4; border-radius: 5px; padding: 8px; background: #ffffff; font-size: 12px; line-height: 1.35; }
    .os-changed-file-list { display: grid; gap: 6px; }
    .os-changed-file { width: 100%; text-align: left; display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 7px; align-items: center; padding: 7px 8px; background: #ffffff; font-size: 12px; line-height: 1.2; }
    .os-changed-file.os-selected { border-color: #39708f; background: #e7f1f5; }
    .os-file-path { min-width: 0; overflow-wrap: anywhere; }
    .os-file-stats { white-space: nowrap; font-size: 12px; font-variant-numeric: tabular-nums; }
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
    .os-dialog-backdrop { position: fixed; inset: 0; background: rgba(15, 23, 42, 0.45); display: flex; align-items: center; justify-content: center; z-index: 100; }
    .os-dialog { background: #ffffff; border: 1px solid #d8dee4; border-radius: 8px; padding: 18px; min-width: 320px; max-width: 90vw; box-shadow: 0 10px 25px rgba(15, 23, 42, 0.15); }
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
      .os-status-panel, .os-profile-panel, .os-model-panel, .os-task-graph-panel, .os-run-detail-panel, .os-run-evidence-panel, .os-planning-panel { grid-column: 1 / -1; }
      .os-inline-fields, .os-model-layout, .os-advanced-grid, .os-metrics, .os-run-grid { grid-template-columns: 1fr; }
      .os-topbar { align-items: flex-start; flex-direction: column; }
    }
    @media (prefers-color-scheme: dark) {
      body { background: #101418; color: #d9e2ea; }
      .os-topbar, .os-panel, .os-list-item, .os-node, .os-dialog { background: #171d23; border-color: #2a3440; }
      .os-topbar p, .os-section-head span, .os-meta, .os-model-meta, .os-check-field, .os-list-item span, .os-node span, .os-node em, .os-empty, .os-metrics span, .os-run-grid span, .os-run-meta, .os-event-time { color: #94a3b3; }
      .os-status, .os-metrics div, .os-run-grid div, .os-detail-strip, .os-run-head, .os-filter-bar, .os-pending-banner { background: #111820; border-color: #2a3440; }
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
      .os-activity-entry span { color: #94a3b3; }
      .os-activity-entry strong { color: #5ca0b8; }
      .os-activity-preview { color: #d9e2ea; }
      .os-activity-detail { background: #0c1116; border-color: #2a3440; color: #d9e2ea; }
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
