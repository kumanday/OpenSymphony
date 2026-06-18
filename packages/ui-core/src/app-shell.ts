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
  TaskGraphSnapshot,
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
  initialProfiles?: ConnectionProfile[];
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
  runValidation: RunValidationSummary | null;
  runApprovals: ApprovalRequest[] | null;
  lastActionReceipt: ActionReceipt | null;
  auditTrail: AuditTrailEntry[];
  profiles: ConnectionProfile[];
  activeProfileId: string | null;
  gatewayDraft: string;
  loading: boolean;
  activeView: "dashboard" | "planning";
  // Task graph editor state
  taskGraphFilter: TaskGraphFilter;
  runOverlays: Map<string, RunDetail>;
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
    this.state = {
      connectionMode: "connecting",
      connectionMessage: "Connecting",
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
      runValidation: null,
      runApprovals: null,
      lastActionReceipt: null,
      auditTrail: [],
      profiles,
      activeProfileId: activeProfile?.id ?? null,
      gatewayDraft: activeProfile?.gatewayUrl ?? this.transport.baseUri,
      loading: true,
      activeView: "dashboard",
      taskGraphFilter: { ...defaultTaskGraphFilter },
      runOverlays: new Map(),
      planningWorkspace: emptyPlanningWorkspaceState(),
      planningEdit: { ...emptyPlanningEditState },
    };
    this.loadPlanningWorkspace("opensymphony-local");
  }

  private async loadRunDetails(runId: string): Promise<void> {
    this.state.runFiles = null;
    this.state.runDiff = null;
    this.state.runEvents = null;
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

  private async loadGatewayState(): Promise<void> {
    try {
      const [capabilities, snapshot] = await Promise.all([
        this.transport.health(),
        this.transport.snapshot(),
      ]);
      this.state.capabilities = capabilities;
      this.state.snapshot = snapshot;
      this.state.connectionMode = "connected";
      this.state.connectionMessage = `Connected to ${this.transport.baseUri || "same-origin gateway"}`;
      this.state.selectedProjectId = snapshot.projects[0]?.project_id ?? "default";
      await this.loadTaskGraph(this.state.selectedProjectId);
      this.loadPlanningWorkspace(this.state.selectedProjectId);
      this.state.planningWorkspace = {
        ...this.state.planningWorkspace,
        project_id: this.state.selectedProjectId,
      };
    } catch (error) {
      this.state.capabilities = null;
      this.state.snapshot = null;
      this.state.taskGraph = null;
      this.state.selectedProjectId = null;
      this.state.selectedNodeId = null;
      this.state.runDetail = null;
      this.state.runFiles = null;
      this.state.runDiff = null;
      this.state.evidenceView = "diff";
      this.state.runEvents = null;
      this.state.runValidation = null;
      this.state.runApprovals = null;
      this.state.connectionMode = "failed";
      this.state.connectionMessage = `Gateway unavailable: ${errorMessage(error)}`;
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
    try {
      const taskGraph = await this.transport.taskGraph(projectId);
      this.state.taskGraph = taskGraph;
      this.state.selectedNodeId =
        taskGraph.root_ids[0] ?? taskGraph.nodes[0]?.node_id ?? null;
      this.state.runDetail = null;
      this.state.runFiles = null;
      this.state.runDiff = null;
      this.state.evidenceView = "diff";
      this.state.runEvents = null;
      this.state.runValidation = null;
      this.state.runApprovals = null;
      this.state.selectedDiffPath = null;
      await this.loadRunOverlays(taskGraph);
    } catch (error) {
      this.state.taskGraph = null;
      this.state.selectedNodeId = null;
      this.state.runDetail = null;
      this.state.runFiles = null;
      this.state.runDiff = null;
      this.state.evidenceView = "diff";
      this.state.runEvents = null;
      this.state.runValidation = null;
      this.state.runApprovals = null;
      this.state.connectionMessage = `Task graph unavailable: ${errorMessage(error)}`;
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
    const gatewayUrl = (gatewayInput?.value ?? "").trim();
    const activeProfile = this.state.profiles.find((profile) => profile.id === this.state.activeProfileId);
    const label = activeProfile?.label ?? "Local Gateway";
    const kind = editableProfileKindFromValue(kindInput?.value, this.options.mode);
    if (!gatewayUrl) {
      this.state.connectionMessage = "Profile URL is required";
      this.render();
      return;
    }

    try {
      const saved = await controller.storeProfile({
        id: this.state.activeProfileId ?? undefined,
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

  private render(): void {
    if (this.destroyed) {
      return;
    }
    const title = this.options.title ?? "OpenSymphony";
    this.options.root.innerHTML = `
      <style>${appShellStyles()}</style>
      <main class="os-app" data-opensymphony-app-shell="mounted" data-mode="${this.options.mode}">
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
    if (this.state.activeView === "planning") {
      return `
        ${this.renderProfiles()}
        ${renderPlanningWorkspace(this.state.planningWorkspace, this.state.planningEdit)}
      `;
    }
    return `
      ${this.renderStatus()}
      ${this.renderProfiles()}
      ${this.renderTaskGraph()}
      ${this.renderRunDetail()}
      ${this.renderRunEvidence()}
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
    return `
      <section class="os-panel os-profile-panel">
        <div class="os-section-head">
          <h2>Connection</h2>
          <span>${escapeHtml(this.options.mode)}</span>
        </div>
        <label class="os-field">
          <span>Profile</span>
          <select data-profile-select>${options}</select>
        </label>
        <div class="os-inline-fields">
          <label class="os-field">
            <span>Kind</span>
            <select data-profile-kind>${kindOptions}</select>
          </label>
          <label class="os-field">
            <span>Gateway URL</span>
            <input data-profile-gateway value="${escapeAttr(this.state.gatewayDraft)}" />
          </label>
          <button type="button" data-save-profile ${this.options.profileController ? "" : "disabled"}>Save</button>
        </div>
        <div class="os-meta">Transport: ${escapeHtml(capabilities)}</div>
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
    const getOverlay = (node: TaskGraphNode) => {
      const run = node.run_id ? this.state.runOverlays.get(node.run_id) : undefined;
      return buildRuntimeOverlay(node, run);
    };
    const filtered = filterTaskGraphNodes(taskGraph.nodes, this.state.taskGraphFilter, getOverlay);
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
    const activity = renderRunActivity(this.state.runEvents);
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
    this.options.root.querySelector("[data-profile-select]")?.addEventListener("change", (event) => {
      const target = event.target as HTMLSelectElement;
      void this.selectProfile(target.value);
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
          void this.openRun(node);
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

function renderRunActivity(events: RunEvent[] | null): string {
  if (events === null) {
    return `<div class="os-run-activity os-empty" data-testid="run-activity">Loading conversation activity</div>`;
  }
  if (events.length === 0) {
    return `<div class="os-run-activity os-empty" data-testid="run-activity">No recent activity</div>`;
  }
  const items = [...events]
    .reverse()
    .map((event) => `
      <div class="os-activity-entry" data-testid="run-activity-entry">
        <span>${escapeHtml(formatEventTime(event.happened_at))}</span>
        <strong>${escapeHtml(event.kind)}</strong>
        <p>${escapeHtml(event.summary)}</p>
      </div>
    `)
    .join("");
  return `<div class="os-run-activity" data-testid="run-activity">${items}</div>`;
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
  return error instanceof Error ? error.message : String(error);
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
  const rowHeight = 76;
  const rowGap = 10;
  const laneWidth = 46;
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
        links.push({ from, to: model });
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
    parts.push(`<- ${nodeLabel(upstreamVisible[0])}`);
    if (upstreamVisible.length > 1) {
      parts.push(`+${upstreamVisible.length - 1}`);
    }
  } else if (upstreamHiddenCount > 0) {
    parts.push(`<- ${upstreamHiddenCount} hidden`);
  }

  if (downstreamVisible.length > 0) {
    const chain = downstreamVisible.slice(0, 2).map(nodeLabel).join(" -> ");
    parts.push(`-> ${chain}`);
    if (downstreamVisible.length > 2) {
      parts.push(`+${downstreamVisible.length - 2}`);
    }
  } else if (downstreamHiddenCount > 0) {
    parts.push(`-> ${downstreamHiddenCount} hidden`);
  }

  return parts.join(" ");
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

  return `
    <button type="button" class="os-node os-node-readonly ${isSelected ? "is-selected" : ""} ${hasUpstream ? "os-node-has-upstream" : ""} ${hasDownstream ? "os-node-has-downstream" : ""}" data-node-id="${escapeAttr(node.node_id)}" style="--os-lane: ${model.lane};">
      <span class="os-node-gutter" aria-hidden="true">${escapeHtml(signal.gutter)}</span>
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
    .os-task-graph-panel { grid-column: span 2; }
    .os-section-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
    .os-section-head h2 { margin: 0; font-size: 15px; letter-spacing: 0; }
    .os-section-head span, .os-meta { color: #667788; font-size: 12px; }
    .os-inline-fields { display: grid; grid-template-columns: minmax(150px, 0.7fr) minmax(260px, 1.3fr) auto; gap: 10px; align-items: end; }
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
    .os-task-graph-links { position: absolute; inset: 0 auto auto 0; width: var(--os-graph-width); height: var(--os-graph-height); pointer-events: none; overflow: visible; }
    .os-task-graph-link { fill: none; stroke: #39708f; stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; opacity: 0.78; marker-end: url(#os-task-arrow); }
    .os-task-graph-links marker path { fill: #39708f; }
    .os-node-graph-list { position: relative; z-index: 1; min-width: var(--os-graph-width); gap: 10px; }
    .os-node-readonly { grid-template-columns: 34px minmax(0, 1fr); align-items: center; min-height: 76px; margin-left: calc(var(--os-lane, 0) * 46px); margin-right: 10px; border-radius: 8px; transition-property: background-color, border-color, box-shadow, transform; transition-duration: 150ms; transition-timing-function: ease-out; }
    .os-node-readonly:active { transform: scale(0.996); }
    .os-node-gutter { width: 24px; height: 24px; display: inline-flex; align-items: center; justify-content: center; border-radius: 999px; font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; color: #39708f; font-size: 12px; font-weight: 700; white-space: pre; background: #e7f1f5; box-shadow: 0 0 0 1px rgba(57, 112, 143, 0.22); }
    .os-node-main, .os-node-line, .os-node-subline { min-width: 0; }
    .os-node-line { display: flex; gap: 8px; align-items: baseline; flex-wrap: nowrap; }
    .os-node-line > span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .os-node-line strong { flex: 0 0 auto; font-variant-numeric: tabular-nums; }
    .os-node-dependency { color: #92400e; font-size: 12px; white-space: nowrap; }
    .os-node-subline { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; margin-top: 5px; }
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
    .os-run-action-bar { display: flex; flex-wrap: wrap; gap: 10px; margin: 12px 0; }
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
    .os-activity-entry { display: grid; grid-template-columns: auto auto minmax(0, 1fr); gap: 8px; align-items: start; border: 1px solid #d8dee4; border-radius: 6px; padding: 8px; background: #f8fafc; font-size: 12px; }
    .os-activity-entry span { color: #667788; white-space: nowrap; }
    .os-activity-entry strong { color: #39708f; white-space: nowrap; }
    .os-activity-entry p { margin: 0; min-width: 0; overflow-wrap: anywhere; }
    .os-changed-file-list { display: grid; gap: 6px; }
    .os-changed-file { width: 100%; text-align: left; display: grid; grid-template-columns: auto 1fr auto; gap: 8px; align-items: center; padding: 8px; background: #ffffff; }
    .os-changed-file.os-selected { border-color: #39708f; background: #e7f1f5; }
    .os-file-path { min-width: 0; overflow-wrap: anywhere; }
    .os-file-stats { white-space: nowrap; }
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
      .os-status-panel, .os-profile-panel, .os-task-graph-panel, .os-run-detail-panel, .os-run-evidence-panel, .os-planning-panel { grid-column: 1 / -1; }
      .os-inline-fields, .os-metrics, .os-run-grid { grid-template-columns: 1fr; }
      .os-topbar { align-items: flex-start; flex-direction: column; }
    }
    @media (prefers-color-scheme: dark) {
      body { background: #101418; color: #d9e2ea; }
      .os-topbar, .os-panel, .os-list-item, .os-node, .os-dialog { background: #171d23; border-color: #2a3440; }
      .os-topbar p, .os-section-head span, .os-meta, .os-list-item span, .os-node span, .os-node em, .os-empty, .os-metrics span, .os-run-grid span, .os-run-meta, .os-event-time { color: #94a3b3; }
      .os-status, .os-metrics div, .os-run-grid div, .os-detail-strip, .os-run-head, .os-filter-bar, .os-pending-banner { background: #111820; border-color: #2a3440; }
      .os-segmented { background: #111820; border-color: #2a3440; }
      .os-segmented button.is-selected { background: #18303a; border-color: #5ca0b8; }
      .os-field input, .os-field select, .os-inline-input, .os-dialog textarea { background: #0f151b; color: #d9e2ea; border-color: #344454; }
      button { background: #1f2a35; color: #d9e2ea; border-color: #3b4c5e; }
      button:hover:not(:disabled), .os-list-item:hover, .os-node:hover, .os-changed-file:hover, .is-selected { background: #18303a; border-color: #5ca0b8; }
      .os-task-graph-link { stroke: #5ca0b8; opacity: 0.82; }
      .os-task-graph-links marker path { fill: #5ca0b8; }
      .os-node-gutter { background: #10232c; box-shadow: 0 0 0 1px rgba(92, 160, 184, 0.3); }
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
      .os-node .os-node-gutter, .os-node .os-node-dependency { color: #fbbf24; }

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
