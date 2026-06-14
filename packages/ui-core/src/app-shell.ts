import type {
  ActionDispatch,
  ActionReceipt,
  ConnectionProfile,
  DashboardSnapshot,
  GatewayCapabilities,
  RunDetail,
  RunPhase,
  RunStreamLiveness,
  TaskGraphNode,
  TaskGraphNodeKind,
  TaskGraphSnapshot,
} from "@opensymphony/gateway-schema";
import {
  buildRuntimeOverlay,
  defaultTaskGraphFilter,
  filterTaskGraphNodes,
  renderTaskGraphFilters,
  type TaskGraphFilter,
} from "./task-graph-editor.js";
import {
  emptyEditorDialog,
  emptyInlineEdit,
  emptyDependencyEdit,
  emptyCommentEdit,
  renderCommentEditor,
  renderCreateDialog,
  renderDependencyEditor,
  renderSelectedNodeDetail,
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

export interface GatewayReader {
  readonly baseUri: string;
  health(): Promise<GatewayCapabilities>;
  snapshot(): Promise<DashboardSnapshot>;
  taskGraph(projectId: string): Promise<TaskGraphSnapshot>;
  runDetail(runId: string): Promise<RunDetail>;
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

type ConnectionMode = "connecting" | "connected" | "fixture" | "failed";

interface AppState {
  connectionMode: ConnectionMode;
  connectionMessage: string;
  capabilities: GatewayCapabilities | null;
  snapshot: DashboardSnapshot | null;
  taskGraph: TaskGraphSnapshot | null;
  selectedProjectId: string | null;
  selectedNodeId: string | null;
  runDetail: RunDetail | null;
  profiles: ConnectionProfile[];
  activeProfileId: string | null;
  gatewayDraft: string;
  loading: boolean;
  // Task graph editor state
  taskGraphFilter: TaskGraphFilter;
  inlineEdit: InlineEditState;
  createDialog: EditorDialogState;
  dependencyEdit: DependencyEditState;
  commentEdit: CommentEditState;
  runOverlays: Map<string, RunDetail>;
  pendingMutations: Set<string>;
  pendingCreates: Map<string, string>;
  /** Snapshot of the task graph node before each optimistic mutation, keyed by correlation id. `null` means a new node was created. */
  pendingSnapshots: Map<string, TaskGraphNode | null>;
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
      profiles,
      activeProfileId: activeProfile?.id ?? null,
      gatewayDraft: activeProfile?.gatewayUrl ?? this.transport.baseUri,
      loading: true,
      taskGraphFilter: { ...defaultTaskGraphFilter },
      inlineEdit: { ...emptyInlineEdit },
      createDialog: { ...emptyEditorDialog },
      dependencyEdit: { ...emptyDependencyEdit },
      commentEdit: { ...emptyCommentEdit },
      runOverlays: new Map(),
      pendingMutations: new Set(),
      pendingCreates: new Map(),
      pendingSnapshots: new Map(),
    };
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
    } catch (error) {
      this.state.capabilities = alphaCapabilities();
      this.state.snapshot = alphaSnapshot();
      this.state.taskGraph = alphaTaskGraph();
      this.state.selectedProjectId = this.state.snapshot.projects[0]?.project_id ?? "opensymphony-local";
      this.state.selectedNodeId = this.state.taskGraph.nodes[1]?.node_id ?? null;
      this.state.runDetail = alphaRunDetail("desktop-alpha");
      this.state.connectionMode = "fixture";
      this.state.connectionMessage = `Gateway unavailable, showing desktop-alpha fixture data: ${errorMessage(error)}`;
    }
  }

  private async loadTaskGraph(projectId: string | null): Promise<void> {
    if (!projectId) {
      this.state.taskGraph = alphaTaskGraph();
      this.state.selectedNodeId = this.state.taskGraph.nodes[1]?.node_id ?? null;
      return;
    }
    try {
      const taskGraph = await this.transport.taskGraph(projectId);
      this.state.taskGraph = taskGraph;
      this.state.selectedNodeId =
        taskGraph.root_ids[0] ?? taskGraph.nodes[0]?.node_id ?? null;
      this.state.runDetail = null;
      await this.loadRunOverlays(taskGraph);
    } catch {
      this.state.taskGraph = alphaTaskGraph(projectId);
      this.state.selectedNodeId = this.state.taskGraph.nodes[1]?.node_id ?? null;
      this.state.runDetail = alphaRunDetail("desktop-alpha");
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
    const runId = node.run_id || node.node_id;
    this.state.selectedNodeId = node.node_id;
    this.state.loading = true;
    this.render();
    try {
      this.state.runDetail = await this.transport.runDetail(runId);
      this.state.runOverlays.set(runId, this.state.runDetail);
    } catch {
      this.state.runDetail = alphaRunDetail(runId, node.identifier);
    } finally {
      this.state.loading = false;
      this.render();
    }
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
    const labelInput = this.options.root.querySelector<HTMLInputElement>("[data-profile-label]");
    const kindInput = this.options.root.querySelector<HTMLSelectElement>("[data-profile-kind]");
    const gatewayUrl = (gatewayInput?.value ?? "").trim();
    const label = (labelInput?.value ?? "Local Gateway").trim() || "Local Gateway";
    const kind = editableProfileKindFromValue(kindInput?.value, this.options.mode);
    if (!gatewayUrl) {
      this.state.connectionMessage = "Profile URL is required";
      this.render();
      return;
    }

    try {
      const saved = await controller.storeProfile({
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
    const selectedNode = this.state.taskGraph?.nodes.find(
      (node) => node.node_id === this.state.selectedNodeId,
    );
    this.options.root.innerHTML = `
      <style>${appShellStyles()}</style>
      <main class="os-app" data-opensymphony-app-shell="mounted" data-mode="${this.options.mode}">
        <header class="os-topbar">
          <div>
            <h1>${escapeHtml(title)}</h1>
            <p>${escapeHtml(this.state.connectionMessage)}</p>
          </div>
          <div class="os-status os-status-${this.state.connectionMode}">
            <span></span>${escapeHtml(statusLabel(this.state.connectionMode))}
          </div>
        </header>
        <section class="os-grid">
          ${this.renderProfiles()}
          ${this.renderDashboard()}
          ${this.renderTaskGraph(selectedNode)}
          ${this.renderRunDetail()}
        </section>
      </main>
    `;
    this.bindEvents();
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
            <span>Label</span>
            <input data-profile-label value="Local Gateway" />
          </label>
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

  private renderDashboard(): string {
    const snapshot = this.state.snapshot;
    if (!snapshot) {
      return panel("Dashboard", `<div class="os-empty">Loading dashboard</div>`);
    }
    const projectButtons = snapshot.projects.map((project) => `
      <button type="button" class="os-list-item ${project.project_id === this.state.selectedProjectId ? "is-selected" : ""}" data-project-id="${escapeAttr(project.project_id)}">
        <strong>${escapeHtml(project.name)}</strong>
        <span>${project.running_count} running, ${project.completed_count} done, ${project.failed_count} failed</span>
      </button>
    `).join("");
    const events = snapshot.recent_events.slice(0, 5).map((event) => `
      <li>
        <span>${escapeHtml(event.kind)}</span>
        <strong>${escapeHtml(event.issue_identifier ?? "system")}</strong>
        ${escapeHtml(event.summary)}
      </li>
    `).join("");
    return panel(
      "Dashboard",
      `
        <div class="os-metrics">
          <div><strong>${snapshot.metrics.running_issue_count}</strong><span>Running</span></div>
          <div><strong>${snapshot.metrics.retry_queue_depth}</strong><span>Retry Queue</span></div>
          <div><strong>${formatNumber(snapshot.metrics.total_input_tokens + snapshot.metrics.total_output_tokens)}</strong><span>Tokens</span></div>
        </div>
        <div class="os-list">${projectButtons || `<div class="os-empty">No projects</div>`}</div>
        <ol class="os-events">${events || `<li>No recent events</li>`}</ol>
      `,
    );
  }

  private renderTaskGraph(selectedNode: TaskGraphNode | undefined): string {
    const taskGraph = this.state.taskGraph;
    if (!taskGraph) {
      return panel("Task Graph", `<div class="os-empty">No task graph loaded</div>`);
    }
    const allNodes = new Map(taskGraph.nodes.map((node) => [node.node_id, node]));
    const getOverlay = (node: TaskGraphNode) => {
      const run = node.run_id ? this.state.runOverlays.get(node.run_id) : undefined;
      return buildRuntimeOverlay(node, run);
    };
    const filtered = filterTaskGraphNodes(taskGraph.nodes, this.state.taskGraphFilter, getOverlay);
    const nodes = filtered.map((node) => renderTaskGraphNode(
      node,
      this.state.selectedNodeId,
      this.state.inlineEdit,
      getOverlay(node),
    )).join("");

    const selectedStrip = selectedNode ? renderSelectedNodeDetail(selectedNode) : "";
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
      `${toolbar}${filters}${pendingBanner}${selectedStrip}<div class="os-node-list">${nodes || `<div class="os-empty">No tasks match the current filters</div>`}</div>${actions}${createDialog}${dependencyDialog}${commentDialog}`,
    );
  }

  private renderRunDetail(): string {
    const run = this.state.runDetail;
    if (!run) {
      return panel("Run Detail", `<div class="os-empty">Select an issue and open its run</div>`);
    }
    const phase = run.liveness?.phase ?? statusToPhase(run.status, run.release_reason);
    const stream = run.liveness?.stream ?? "healthy";
    const actions = run.safe_actions ?? {
      retry: false,
      cancel: run.status === "running" || run.status === "claimed",
      rehydrate: false,
      detach: false,
    };
    const actionLabels = Object.entries(actions)
      .filter(([, enabled]) => enabled)
      .map(([name]) => `<span>${escapeHtml(name)}</span>`)
      .join("") || "<span>none</span>";
    return panel(
      "Run Detail",
      `
        <div class="os-run-head">
          <div>
            <strong>${escapeHtml(run.issue_identifier)}</strong>
            <span>${escapeHtml(run.run_id)}</span>
          </div>
          <div class="os-pill">${escapeHtml(run.status)}</div>
        </div>
        <div class="os-run-grid">
          <div><span>Phase</span><strong>${escapeHtml(phase)}</strong></div>
          <div><span>Stream</span><strong>${escapeHtml(stream)}</strong></div>
          <div><span>Turns</span><strong>${run.turn_count} / ${run.max_turns}</strong></div>
          <div><span>Runtime</span><strong>${run.runtime_seconds}s</strong></div>
        </div>
        <div class="os-actions">${actionLabels}</div>
        <pre>${escapeHtml(run.workspace_path ?? "workspace path unavailable")}</pre>
      `,
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
          this.state.selectedNodeId = node.node_id;
          this.render();
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

    // Task graph filters
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-filter]").forEach((control) => {
      control.addEventListener("change", () => this.onFilterChange());
      control.addEventListener("input", () => this.onFilterChange());
    });
    this.options.root.querySelector("[data-tg-filter-reset]")?.addEventListener("click", () => {
      this.state.taskGraphFilter = { ...defaultTaskGraphFilter };
      this.render();
    });

    // Toolbar create buttons
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-create]").forEach((button) => {
      button.addEventListener("click", () => {
        const kind = button.dataset.tgCreate as TaskGraphNodeKind;
        this.openCreateDialog(kind, null);
      });
    });

    // Create child buttons
    this.options.root.querySelectorAll<HTMLElement>("[data-tg-create-child]").forEach((button) => {
      button.addEventListener("click", () => {
        const parentId = button.dataset.tgCreateChild;
        if (!parentId) return;
        const parent = this.state.taskGraph?.nodes.find((n) => n.node_id === parentId);
        if (!parent) return;
        const childKind: TaskGraphNodeKind = parent.kind === "milestone" ? "issue" : "sub_issue";
        this.openCreateDialog(childKind, parentId);
      });
    });

    // Create dialog actions
    this.options.root.querySelector("[data-tg-create-save]")?.addEventListener("click", () => {
      void this.saveCreateDialog();
    });
    this.options.root.querySelector("[data-tg-create-cancel]")?.addEventListener("click", () => {
      this.state.createDialog = { ...emptyEditorDialog };
      this.render();
    });

    // Inline edit actions
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

    // Dependency editor actions
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

    // Comment editor actions
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

  // -- Create dialog handling --

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
    const root = this.options.root;
    const title = (root.querySelector<HTMLInputElement>("[data-tg-create-title]")?.value ?? "").trim();
    const state = (root.querySelector<HTMLInputElement>("[data-tg-create-state]")?.value ?? "Todo").trim() || "Todo";
    if (!title) return;

    const parentId = dialog.parentId ?? undefined;
    const nodeId = `new-${dialog.kind}-${generateId()}`;
    const newNode = buildCreatedNode(
      { parent_id: parentId, kind: dialog.kind, title, state },
      nodeId,
    );
    const taskGraph = this.state.taskGraph;
    if (taskGraph) {
      taskGraph.nodes.push(newNode);
      if (parentId) {
        const parent = taskGraph.nodes.find((n) => n.node_id === parentId);
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

  // -- Inline edit handling --

  private startInlineEdit(nodeId: string): void {
    const node = this.state.taskGraph?.nodes.find((n) => n.node_id === nodeId);
    if (!node) return;
    this.state.inlineEdit = { nodeId, title: node.title, state: node.state };
    this.render();
  }

  private async saveInlineEdit(nodeId: string): Promise<void> {
    const root = this.options.root;
    const title = Array.from(root.querySelectorAll<HTMLInputElement>("[data-tg-inline-title]")).find(
      (el) => el.dataset.tgInlineTitle === nodeId,
    )?.value.trim();
    const state = Array.from(root.querySelectorAll<HTMLInputElement>("[data-tg-inline-state]")).find(
      (el) => el.dataset.tgInlineState === nodeId,
    )?.value.trim();
    const node = this.state.taskGraph?.nodes.find((n) => n.node_id === nodeId);
    if (!node) return;
    const updated = applyNodeUpdate(node, { title, state });
    this.updateTaskGraphNode(updated);
    this.state.inlineEdit = { ...emptyInlineEdit };
    this.render();

    if (isActionCapable(this.transport)) {
      const correlationId = `tg-update-${nodeId}-${generateId()}`;
      this.state.pendingMutations.add(correlationId);
      this.state.pendingSnapshots.set(correlationId, { ...node });
      this.render();
      try {
        const receipt = await dispatchTaskGraphUpdate(this.transport, {
          node_id: nodeId,
          title,
          state,
        }, correlationId);
        this.applyMutationReceipt(receipt);
      } catch (error) {
        this.rollbackOptimisticMutation(correlationId);
        this.state.connectionMessage = `Update failed: ${errorMessage(error)}`;
      }
      this.render();
    }
  }

  // -- Dependency editor handling --

  private openDependencyEditor(nodeId: string): void {
    const node = this.state.taskGraph?.nodes.find((n) => n.node_id === nodeId);
    if (!node) return;
    this.state.dependencyEdit = { nodeId, blockedBy: [...node.blocked_by] };
    this.render();
  }

  private async saveDependencyEdit(): Promise<void> {
    const nodeId = this.state.dependencyEdit.nodeId;
    if (!nodeId) return;
    const select = this.options.root.querySelector<HTMLSelectElement>("[data-tg-deps-select]");
    const blockedBy = Array.from(select?.selectedOptions ?? []).map((option) => option.value);
    const node = this.state.taskGraph?.nodes.find((n) => n.node_id === nodeId);
    if (!node) return;
    const updated = { ...node, blocked_by: blockedBy };
    this.updateTaskGraphNode(updated);
    this.state.dependencyEdit = { ...emptyDependencyEdit };
    this.render();

    if (isActionCapable(this.transport)) {
      const correlationId = `tg-deps-${nodeId}-${generateId()}`;
      this.state.pendingMutations.add(correlationId);
      this.state.pendingSnapshots.set(correlationId, { ...node });
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

  // -- Comment editor handling --

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

    const node = this.state.taskGraph?.nodes.find((n) => n.node_id === nodeId);
    if (node) {
      const updated = { ...node, comment_count: (node.comment_count ?? 0) + 1 };
      this.updateTaskGraphNode(updated);
    }
    this.state.commentEdit = { ...emptyCommentEdit };
    this.render();

    if (isActionCapable(this.transport)) {
      const correlationId = `tg-comment-${nodeId}-${generateId()}`;
      this.state.pendingMutations.add(correlationId);
      if (node) {
        this.state.pendingSnapshots.set(correlationId, { ...node });
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

  // -- Local state mutation helpers --

  private updateTaskGraphNode(updated: TaskGraphNode): void {
    const taskGraph = this.state.taskGraph;
    if (!taskGraph) return;
    const idx = taskGraph.nodes.findIndex((n) => n.node_id === updated.node_id);
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

    // Reconcile optimistic create ids with the server-assigned id from the receipt.
    const localNodeId = this.state.pendingCreates.get(receipt.correlation_id);
    if (localNodeId && localNodeId !== result.node_id) {
      this.reconcileNodeId(localNodeId, result.node_id);
    }
    this.state.pendingMutations.delete(receipt.correlation_id);
    this.state.pendingCreates.delete(receipt.correlation_id);
    this.state.pendingSnapshots.delete(receipt.correlation_id);

    const node = this.state.taskGraph?.nodes.find((n) => n.node_id === result.node_id);
    if (node) {
      this.updateTaskGraphNode({ ...node, updated_at: result.updated_at });
    }
  }

  private rollbackOptimisticMutation(correlationId: string): void {
    const snapshot = this.state.pendingSnapshots.get(correlationId);
    if (snapshot === undefined) {
      // No snapshot recorded; nothing to roll back.
      this.state.pendingMutations.delete(correlationId);
      this.state.pendingCreates.delete(correlationId);
      return;
    }

    const taskGraph = this.state.taskGraph;
    if (snapshot === null) {
      // Optimistic create: remove the temporary node.
      const localNodeId = this.state.pendingCreates.get(correlationId);
      if (taskGraph && localNodeId) {
        taskGraph.nodes = taskGraph.nodes.filter((n) => n.node_id !== localNodeId);
        taskGraph.root_ids = taskGraph.root_ids.filter((id) => id !== localNodeId);
        for (const node of taskGraph.nodes) {
          node.children = node.children.filter((id) => id !== localNodeId);
          if (node.parent_id === localNodeId) {
            node.parent_id = undefined;
          }
        }
      }
    } else if (taskGraph) {
      // Restore the node from the snapshot.
      this.updateTaskGraphNode(snapshot);
    }

    this.state.pendingMutations.delete(correlationId);
    this.state.pendingCreates.delete(correlationId);
    this.state.pendingSnapshots.delete(correlationId);
  }

  private reconcileNodeId(oldId: string, newId: string): void {
    const taskGraph = this.state.taskGraph;
    if (!taskGraph) return;

    if (taskGraph.nodes.some((n) => n.node_id === newId && n.node_id !== oldId)) {
      this.state.connectionMessage = `Server returned a duplicate node ID (${newId}); optimistic ID not reconciled.`;
      return;
    }

    const node = taskGraph.nodes.find((n) => n.node_id === oldId);
    if (!node) return;
    node.node_id = newId;

    if (taskGraph.root_ids.includes(oldId)) {
      taskGraph.root_ids = taskGraph.root_ids.map((id) => (id === oldId ? newId : id));
    }
    for (const n of taskGraph.nodes) {
      if (n.parent_id === oldId) n.parent_id = newId;
      n.children = n.children.map((id) => (id === oldId ? newId : id));
      n.blocked_by = n.blocked_by.map((id) => (id === oldId ? newId : id));
    }

    if (this.state.selectedNodeId === oldId) this.state.selectedNodeId = newId;
    if (this.state.inlineEdit.nodeId === oldId) this.state.inlineEdit.nodeId = newId;
    if (this.state.dependencyEdit.nodeId === oldId) this.state.dependencyEdit.nodeId = newId;
    if (this.state.commentEdit.nodeId === oldId) this.state.commentEdit.nodeId = newId;
  }
}

function panel(title: string, body: string): string {
  return `
    <section class="os-panel">
      <div class="os-section-head"><h2>${escapeHtml(title)}</h2></div>
      ${body}
    </section>
  `;
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
      gatewayUrl: gatewayUrl || "http://127.0.0.1:8000",
      transport: "loopback_http",
      managed: false,
    },
  ];
}

function alphaCapabilities(): GatewayCapabilities {
  return {
    schema_version: schemaVersion,
    gateway_version: "desktop-alpha-fixture",
    supported_api_versions: ["1.0.0"],
    transports: [
      {
        transport: "loopback_http",
        modes: ["json"],
        supported_encodings: ["utf-8"],
        bidirectional: false,
      },
    ],
    features: [
      { feature: "task_graph", available: true, requires_auth: false },
      { feature: "terminal_stream", available: false, requires_auth: false },
    ],
    auth_modes: ["none"],
    max_event_page_size: 1000,
    max_terminal_frame_batch: 500,
  };
}

function alphaSnapshot(): DashboardSnapshot {
  return {
    schema_version: schemaVersion,
    generated_at: new Date(1_700_000_000_000).toISOString(),
    sequence: 1,
    health: "degraded",
    metrics: {
      running_issue_count: 1,
      retry_queue_depth: 0,
      total_input_tokens: 12000,
      total_output_tokens: 6200,
      total_cache_read_tokens: 1800,
      total_cost_micros: 0,
    },
    projects: [
      {
        project_id: "opensymphony-local",
        name: "OpenSymphony",
        milestone_count: 1,
        issue_count: 3,
        running_count: 1,
        completed_count: 2,
        failed_count: 0,
      },
    ],
    recent_events: [
      {
        happened_at: new Date(1_700_000_000_000).toISOString(),
        issue_identifier: "DESKTOP-ALPHA",
        kind: "client_attached",
        summary: "Desktop alpha shell mounted",
      },
    ],
  };
}

function alphaTaskGraph(projectId = "opensymphony-local"): TaskGraphSnapshot {
  return {
    schema_version: schemaVersion,
    project_id: projectId,
    generated_at: new Date(1_700_000_000_000).toISOString(),
    root_ids: ["m7"],
    nodes: [
      {
        schema_version: schemaVersion,
        node_id: "m7",
        kind: "milestone",
        identifier: "M7",
        title: "Shared Client And Desktop Alpha",
        state: "Backlog",
        state_category: "backlog",
        children: ["desktop-alpha", "coe-410"],
        blocked_by: [],
        labels: ["desktop"],
      },
      {
        schema_version: schemaVersion,
        node_id: "desktop-alpha",
        kind: "issue",
        identifier: "DESKTOP-ALPHA",
        title: "Desktop alpha recovery",
        state: "Backlog",
        state_category: "backlog",
        parent_id: "m7",
        children: [],
        blocked_by: [],
        labels: ["desktop", "recovery"],
      },
      {
        schema_version: schemaVersion,
        node_id: "coe-410",
        kind: "issue",
        identifier: "COE-410",
        title: "Desktop local stream optimization",
        state: "Done",
        state_category: "done",
        parent_id: "m7",
        children: [],
        blocked_by: [],
        labels: ["transport"],
      },
    ],
  };
}

function alphaRunDetail(runId: string, issueIdentifier = runId): RunDetail {
  return {
    schema_version: schemaVersion,
    run_id: runId,
    issue_id: issueIdentifier,
    issue_identifier: issueIdentifier,
    worker_id: "desktop-alpha",
    status: "running",
    claimed_at: new Date(1_700_000_000_000).toISOString(),
    started_at: new Date(1_700_000_030_000).toISOString(),
    turn_count: 1,
    max_turns: 8,
    input_tokens: 12000,
    output_tokens: 6200,
    cache_read_tokens: 1800,
    runtime_seconds: 90,
    workspace_path: "/tmp/opensymphony/desktop-alpha",
    liveness: {
      phase: "quiet",
      stream: "stale",
      latest_progress: {
        sequence: 1,
        event_id: "fixture-progress-1",
        happened_at: new Date(1_700_000_060_000).toISOString(),
        kind: "snapshot_published",
        summary: "Fixture run detail available",
      },
    },
    safe_actions: {
      retry: false,
      cancel: true,
      rehydrate: true,
      detach: false,
    },
  };
}

function statusToPhase(
  status: RunDetail["status"],
  releaseReason?: RunDetail["release_reason"],
): RunPhase {
  if (status === "retry_queued") {
    return "retry_queued";
  }
  if (status === "released") {
    return releaseReason === "completed" ? "completed" : "cancelled";
  }
  return status === "running" || status === "claimed" ? "active" : "quiet";
}

function statusLabel(mode: ConnectionMode): string {
  switch (mode) {
    case "connected":
      return "Connected";
    case "fixture":
      return "Fixture";
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
    .os-status-fixture span { background: #d97706; }
    .os-status-failed span { background: #c2410c; }
    .os-grid { display: grid; grid-template-columns: minmax(260px, 0.75fr) minmax(320px, 1fr) minmax(360px, 1.15fr); gap: 14px; padding: 14px; align-items: start; }
    .os-panel { background: #ffffff; border: 1px solid #d8dee4; border-radius: 8px; padding: 14px; min-width: 0; box-shadow: 0 1px 2px rgba(15, 23, 42, 0.05); }
    .os-profile-panel { grid-column: 1 / -1; }
    .os-section-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
    .os-section-head h2 { margin: 0; font-size: 15px; letter-spacing: 0; }
    .os-section-head span, .os-meta { color: #667788; font-size: 12px; }
    .os-inline-fields { display: grid; grid-template-columns: minmax(150px, 0.75fr) minmax(140px, 0.65fr) minmax(220px, 1.2fr) auto; gap: 10px; align-items: end; }
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
    .os-list-item span, .os-node span, .os-node em { color: #667788; font-size: 12px; font-style: normal; }
    .is-selected { border-color: #39708f; background: #e7f1f5; }
    .os-node-kind { text-transform: uppercase; letter-spacing: 0.08em; }
    .os-detail-strip, .os-run-head { display: flex; align-items: center; justify-content: space-between; gap: 10px; margin-bottom: 10px; border: 1px solid #d8dee4; border-radius: 6px; padding: 10px; background: #fbfcfd; }
    .os-detail-strip span, .os-run-head span { color: #667788; font-size: 12px; }
    .os-events { margin: 0; padding-left: 18px; display: grid; gap: 7px; font-size: 13px; }
    .os-events span { color: #39708f; margin-right: 6px; }
    .os-pill, .os-actions span { border-radius: 999px; background: #e7f1f5; color: #23566f; padding: 5px 9px; font-size: 12px; }
    .os-actions { display: flex; flex-wrap: wrap; gap: 6px; margin: 12px 0; }
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
    @media (max-width: 980px) {
      .os-grid { grid-template-columns: 1fr; }
      .os-inline-fields, .os-metrics, .os-run-grid { grid-template-columns: 1fr; }
      .os-topbar { align-items: flex-start; flex-direction: column; }
    }
    @media (prefers-color-scheme: dark) {
      body { background: #101418; color: #d9e2ea; }
      .os-topbar, .os-panel, .os-list-item, .os-node, .os-dialog { background: #171d23; border-color: #2a3440; }
      .os-topbar p, .os-section-head span, .os-meta, .os-list-item span, .os-node span, .os-node em, .os-empty, .os-metrics span, .os-run-grid span, .os-run-meta { color: #94a3b3; }
      .os-status, .os-metrics div, .os-run-grid div, .os-detail-strip, .os-run-head, .os-filter-bar, .os-pending-banner { background: #111820; border-color: #2a3440; }
      .os-field input, .os-field select, .os-inline-input, .os-dialog textarea { background: #0f151b; color: #d9e2ea; border-color: #344454; }
      button { background: #1f2a35; color: #d9e2ea; border-color: #3b4c5e; }
      button:hover:not(:disabled), .os-list-item:hover, .os-node:hover, .is-selected { background: #18303a; border-color: #5ca0b8; }
      .os-badge-failed, .os-badge-blocked, .os-badge-blocker { background: #451a1a; color: #fca5a5; }
      .os-badge-running { background: #14532d; color: #86efac; }
      .os-badge-complete { background: #1e3a8a; color: #93c5fd; }
      .os-badge-stale { background: #451a03; color: #fcd34d; }
      .os-badge-queued, .os-badge-retry { background: #3b0764; color: #d8b4fe; }
      .os-badge-workspace, .os-badge-harness, .os-badge-diff_summary, .os-badge-validation { background: #1e293b; color: #cbd5e1; }
      pre { background: #0c1116; color: #d9e2ea; }
    }
  `;
}
