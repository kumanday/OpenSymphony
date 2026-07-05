import type {
  GraphLayoutResult,
  GraphState,
  MemoryGraphSnapshot,
} from "@opensymphony/graph";
import * as THREE from "three";
import { escapeAttr, escapeHtml } from "./html.js";
import {
  advanceCameraToward,
  buildGraphScene,
  defaultCameraForLayout,
  dollyCamera,
  frameWorldPoints,
  hitTestHull,
  hitTestScene,
  nodeEmphasisColor,
  orbitCamera,
  panCamera,
  unprojectToPlaneThroughPoint,
  worldNodesFor,
  worldToLayout,
  type GraphCameraState,
  type GraphScene,
  type KnowledgeGraphViewState,
  type SceneViewport,
} from "./knowledge-graph-scene.js";

export { createKnowledgeGraphViewState } from "./knowledge-graph-scene.js";
export type { KnowledgeGraphViewState } from "./knowledge-graph-scene.js";

const graphSurfaceColor = "#eef1f4";
const graphSurfaceColorInt = 0xeef1f4;

export interface KnowledgeGraphSurface {
  snapshot: MemoryGraphSnapshot | null;
  layout: GraphLayoutResult | null;
  state: GraphState;
}

export interface KnowledgeGraphMountOptions {
  snapshot: MemoryGraphSnapshot | null;
  layout: GraphLayoutResult | null;
  selectedNodeIds: readonly string[];
  view: KnowledgeGraphViewState;
  onSelect(nodeId: string): void;
  onFocus(nodeId: string): void;
}

export function renderKnowledgeGraphSurface(surface: KnowledgeGraphSurface): string {
  const { snapshot, state } = surface;
  const status = state.layoutStatus;
  const metrics = snapshot?.metrics;
  const summary = snapshot
    ? `${snapshot.nodes.length} nodes / ${snapshot.edges.length} edges / ${metrics?.stale_concept_count ?? 0} stale / ${metrics?.warning_count ?? 0} warnings`
    : "No graph snapshot";
  // Home affordance: once the operator narrows the view (neighborhood focus,
  // a selection, or a non-atlas mode) there must always be a one-click path
  // back to the full graph.
  const narrowed = state.mode !== "atlas"
    || state.focusedNodeId !== null
    || state.selectedNodeIds.length > 0;
  const resetButton = narrowed
    ? `<button type="button" class="os-icon-button os-kg-reset" data-kg-reset data-testid="knowledge-graph-reset" title="Show the full graph (Esc)">Show full graph</button>`
    : "";
  return `
    <div class="os-knowledge-graph" data-testid="knowledge-graph-renderer" data-layout-status="${escapeAttr(status)}">
      <div class="os-knowledge-toolbar">
        <div>
          <strong>Knowledge Graph</strong>
          <span data-testid="knowledge-graph-metrics">${escapeHtml(summary)}</span>
        </div>
        ${resetButton}
        ${renderStatus(surface.state)}
      </div>
      <div class="os-knowledge-stage" data-kg-stage>
        <canvas class="os-knowledge-canvas" data-testid="knowledge-graph-canvas" aria-label="Knowledge Graph canvas"></canvas>
        <div class="os-knowledge-labels" data-kg-labels data-morph-ignore-children></div>
        <span class="os-kg-controls-hint" aria-hidden="true">drag pan &middot; &#8997;-drag orbit &middot; scroll zoom &middot; double-click frame</span>
      </div>
      ${renderSelectedInspector(snapshot, surface.state.selectedNodeIds)}
      ${renderFallbackList(snapshot, surface.state.selectedNodeIds)}
    </div>
  `;
}

// ─── Renderer state ─────────────────────────────────────────────────────────

interface RendererState {
  options: KnowledgeGraphMountOptions;
  camera: GraphCameraState;
  goal: GraphCameraState;
  hoveredNodeId: string | null;
  pointer: {
    mode: "idle" | "pan" | "orbit" | "drag-node";
    nodeId: string | null;
    lastX: number;
    lastY: number;
    moved: boolean;
  };
  rafHandle: number | ReturnType<typeof setTimeout> | null;
  rafKind: "animation-frame" | "timeout";
  lastFrameAt: number;
  animating: boolean;
  reducedMotion: boolean;
  canvasSize: { width: number; height: number; cssWidth: number; cssHeight: number };
  lastScene: GraphScene | null;
  layoutIdentity: string | null;
}

const rendererStates = new WeakMap<HTMLCanvasElement, RendererState>();

export function mountKnowledgeGraphRenderer(
  root: HTMLElement,
  options: KnowledgeGraphMountOptions,
): void {
  const canvas = root.querySelector<HTMLCanvasElement>("[data-testid='knowledge-graph-canvas']");
  if (!canvas) return;
  if (!options.snapshot || !options.layout) {
    // The morphing render preserves the canvas node, so without drawable
    // data the previous mount's bitmap and handlers would otherwise stay
    // live and interactive against a stale layout. Always detach the
    // interaction handlers and clear the label/tooltip overlays (they are
    // positioned against the old layout and remain clickable otherwise);
    // drop the bitmap too once the snapshot itself is gone (bundle
    // switch/reset). During a pure re-layout the snapshot is still present
    // and keeping the bitmap avoids a blank flash.
    detachCanvasHandlers(canvas);
    clearOverlayContainer(root);
    if (!options.snapshot) {
      disposeKnowledgeGraphCanvas(canvas);
      canvas.width = canvas.width || 1;
      canvas.dataset.nonblank = "false";
    }
    return;
  }

  const stage = canvas.closest<HTMLElement>("[data-kg-stage]");
  const viewport = canvasViewport(stage);
  const layoutIdentity = `${options.layout.kind}:${options.layout.width}:${options.layout.height}:${options.layout.generatedAt}`;
  let state = rendererStates.get(canvas);
  if (!state) {
    const camera = options.view.camera ?? defaultCameraForLayout(options.layout, viewport);
    state = {
      options,
      camera,
      goal: camera,
      hoveredNodeId: null,
      pointer: { mode: "idle", nodeId: null, lastX: 0, lastY: 0, moved: false },
      rafHandle: null,
      rafKind: "animation-frame",
      lastFrameAt: 0,
      animating: false,
      reducedMotion: prefersReducedMotion(),
      canvasSize: { width: 0, height: 0, cssWidth: 0, cssHeight: 0 },
      lastScene: null,
      layoutIdentity,
    };
    rendererStates.set(canvas, state);
  } else {
    state.options = options;
    if (options.view.camera) {
      state.camera = options.view.camera;
      if (!state.animating) state.goal = options.view.camera;
    }
    if (state.layoutIdentity !== layoutIdentity) {
      state.layoutIdentity = layoutIdentity;
      state.hoveredNodeId = null;
      // A new layout means the content genuinely changed (identical
      // snapshots no longer trigger relayouts): mode switches, bundle
      // switches, and resize relayouts all reposition nodes, so glide the
      // camera toward framing the new layout instead of keeping a view
      // that may point entirely off-frame.
      state.goal = defaultCameraForLayout(options.layout, viewport);
    }
  }
  options.view.camera = state.camera;

  attachCanvasHandlers(canvas, root, stage, state);
  if (cameraDiffers(state.camera, state.goal)) {
    startAnimation(canvas, root, stage, state);
  } else {
    drawFrame(canvas, root, stage, state);
  }
  bindListNavigation(root, options);
}

function cameraDiffers(a: GraphCameraState, b: GraphCameraState): boolean {
  return a.targetX !== b.targetX
    || a.targetY !== b.targetY
    || a.targetZ !== b.targetZ
    || a.distance !== b.distance
    || a.yaw !== b.yaw
    || a.pitch !== b.pitch;
}

export function disposeKnowledgeGraphRenderer(root: ParentNode): void {
  root.querySelectorAll<HTMLCanvasElement>("[data-testid='knowledge-graph-canvas']").forEach((canvas) => {
    disposeKnowledgeGraphCanvas(canvas);
  });
}

export function disposeKnowledgeGraphCanvas(canvas: HTMLCanvasElement): void {
  const state = rendererStates.get(canvas);
  if (state?.rafHandle !== null && state?.rafHandle !== undefined) {
    if (state.rafKind === "animation-frame" && typeof cancelAnimationFrame === "function") {
      cancelAnimationFrame(state.rafHandle as number);
    } else {
      clearTimeout(state.rafHandle as ReturnType<typeof setTimeout>);
    }
    state.rafHandle = null;
  }
  rendererStates.delete(canvas);
  resetThreeCanvasState(canvas);
}

// ─── Interaction ────────────────────────────────────────────────────────────

function canvasPointFromEvent(
  canvas: HTMLCanvasElement,
  event: MouseEvent,
): { x: number; y: number } {
  // offsetX/offsetY are relative to the event *target*, which may be an
  // overlay label the pointer is passing over (events bubble to the canvas
  // handlers) — always resolve against the canvas box instead.
  const rect = canvas.getBoundingClientRect();
  return { x: event.clientX - rect.left, y: event.clientY - rect.top };
}

function detachCanvasHandlers(canvas: HTMLCanvasElement): void {
  canvas.onwheel = null;
  canvas.onpointerdown = null;
  canvas.onpointermove = null;
  canvas.onpointerup = null;
  canvas.onpointerleave = null;
  canvas.ondblclick = null;
  canvas.oncontextmenu = null;
}

function attachCanvasHandlers(
  canvas: HTMLCanvasElement,
  root: HTMLElement,
  stage: HTMLElement | null,
  state: RendererState,
): void {
  canvas.oncontextmenu = (event) => {
    event.preventDefault();
  };
  canvas.onwheel = (event) => {
    event.preventDefault();
    const viewport = canvasViewport(stage);
    const factor = event.deltaY < 0 ? 0.88 : 1.14;
    const point = canvasPointFromEvent(canvas, event);
    state.goal = dollyCamera(state.goal, factor, {
      viewport,
      screenX: point.x,
      screenY: point.y,
    });
    startAnimation(canvas, root, stage, state);
  };
  canvas.onpointerdown = (event) => {
    try {
      canvas.setPointerCapture(event.pointerId);
    } catch {
      // Pointer capture is an enhancement (keeps drags alive outside the
      // canvas); synthetic pointers in tests have no capturable id.
    }
    const viewport = canvasViewport(stage);
    const scene = state.lastScene ?? rebuildScene(state, viewport);
    const point = canvasPointFromEvent(canvas, event);
    const nodeId = event.button === 0 && !event.altKey && !event.metaKey
      ? hitTestScene(scene, point.x, point.y)
      : null;
    state.pointer = {
      mode: nodeId
        ? "drag-node"
        : event.button === 2 || event.altKey || event.metaKey
          ? "orbit"
          : "pan",
      nodeId,
      lastX: event.clientX,
      lastY: event.clientY,
      moved: false,
    };
    canvas.dataset.kgPointer = state.pointer.mode;
  };
  canvas.onpointermove = (event) => {
    const viewport = canvasViewport(stage);
    if (state.pointer.mode === "idle") {
      const scene = state.lastScene ?? rebuildScene(state, viewport);
      const point = canvasPointFromEvent(canvas, event);
      const nodeId = hitTestScene(scene, point.x, point.y);
      if (nodeId !== state.hoveredNodeId) {
        state.hoveredNodeId = nodeId;
        drawFrame(canvas, root, stage, state);
      }
      canvas.style.cursor = nodeId ? "pointer" : "grab";
      return;
    }
    const deltaX = event.clientX - state.pointer.lastX;
    const deltaY = event.clientY - state.pointer.lastY;
    if (Math.abs(deltaX) + Math.abs(deltaY) > 2) state.pointer.moved = true;
    state.pointer.lastX = event.clientX;
    state.pointer.lastY = event.clientY;
    if (state.pointer.mode === "pan") {
      state.camera = panCamera(state.camera, viewport, deltaX, deltaY);
      state.goal = state.camera;
    } else if (state.pointer.mode === "orbit") {
      state.camera = orbitCamera(state.camera, deltaX * 0.005, deltaY * 0.004);
      state.goal = state.camera;
    } else if (state.pointer.mode === "drag-node" && state.pointer.nodeId && state.options.layout) {
      const worldNode = worldNodesFor(state.options.layout, state.options.view.overrides)
        .find((node) => node.nodeId === state.pointer.nodeId);
      if (worldNode) {
        const dragPoint = canvasPointFromEvent(canvas, event);
        const dropped = unprojectToPlaneThroughPoint(
          state.camera,
          viewport,
          dragPoint.x,
          dragPoint.y,
          worldNode,
        );
        if (dropped) {
          state.options.view.overrides.set(state.pointer.nodeId, {
            ...worldToLayout(state.options.layout, dropped),
            z: dropped.z,
          });
        }
      }
    }
    state.options.view.camera = state.camera;
    drawFrame(canvas, root, stage, state);
  };
  canvas.onpointerup = (event) => {
    try {
      canvas.releasePointerCapture(event.pointerId);
    } catch {
      // Matching guard for synthetic pointers; see onpointerdown.
    }
    const { mode, nodeId, moved } = state.pointer;
    state.pointer = { mode: "idle", nodeId: null, lastX: 0, lastY: 0, moved: false };
    delete canvas.dataset.kgPointer;
    canvas.style.cursor = "grab";
    if (!moved && mode === "drag-node" && nodeId) {
      state.options.onSelect(nodeId);
    }
  };
  canvas.onpointerleave = () => {
    if (state.hoveredNodeId !== null && state.pointer.mode === "idle") {
      state.hoveredNodeId = null;
      drawFrame(canvas, root, stage, state);
    }
  };
  canvas.ondblclick = (event) => {
    event.preventDefault();
    const viewport = canvasViewport(stage);
    const scene = state.lastScene ?? rebuildScene(state, viewport);
    const layout = state.options.layout;
    if (!layout) return;
    const worldNodes = worldNodesFor(layout, state.options.view.overrides);
    const point = canvasPointFromEvent(canvas, event);
    const nodeId = hitTestScene(scene, point.x, point.y);
    if (nodeId) {
      // Frame the node and its direct neighborhood.
      const neighborhood = new Set([nodeId]);
      for (const edge of layout.edges) {
        if (edge.sourceId === nodeId) neighborhood.add(edge.targetId);
        if (edge.targetId === nodeId) neighborhood.add(edge.sourceId);
      }
      const points = worldNodes.filter((node) => neighborhood.has(node.nodeId));
      state.goal = frameWorldPoints(points, viewport, state.camera, 1.35);
      startAnimation(canvas, root, stage, state);
      state.options.onSelect(nodeId);
      return;
    }
    const hull = hitTestHull(scene, point.x, point.y);
    if (hull) {
      const members = new Set(
        state.options.snapshot?.communities.find((community) => community.id === hull.areaId)?.node_ids ?? [],
      );
      const points = worldNodes.filter((node) => members.has(node.nodeId));
      if (points.length > 0) {
        state.goal = frameWorldPoints(points, viewport, state.camera, 1.25);
        startAnimation(canvas, root, stage, state);
        return;
      }
    }
    state.goal = defaultCameraForLayout(layout, viewport);
    startAnimation(canvas, root, stage, state);
  };
}

function bindListNavigation(root: HTMLElement, options: KnowledgeGraphMountOptions): void {
  // Handler properties (not addEventListener) so re-mounting after every
  // render stays idempotent: the DOM morph preserves these buttons across
  // renders, and stacked listeners would fire once per past render.
  root.querySelectorAll<HTMLElement>("[data-kg-node-id]").forEach((button) => {
    bindNodeButton(root, button, options);
  });
}

function bindNodeButton(root: HTMLElement, button: HTMLElement, options: KnowledgeGraphMountOptions): void {
  button.onclick = () => {
    const nodeId = button.dataset.kgNodeId;
    if (nodeId) options.onSelect(nodeId);
  };
  button.onkeydown = (event) => {
    const direction = graphListNavigationDirection(event.key);
    if (!direction) return;
    const buttons = Array.from(root.querySelectorAll<HTMLElement>(".os-kg-list [data-kg-node-id]"));
    const index = buttons.indexOf(button);
    if (index < 0) return;
    event.preventDefault();
    const nextIndex = direction === "first"
      ? 0
      : direction === "last"
        ? buttons.length - 1
        : (index + direction + buttons.length) % buttons.length;
    buttons[nextIndex]?.focus();
  };
  button.onfocus = () => {
    const nodeId = button.dataset.kgNodeId;
    // Focus-driven neighborhood preview is a keyboard affordance; a mouse
    // click also focuses the button, and jumping into neighborhood mode on
    // every click stranded users away from the full graph.
    if (nodeId && matchesFocusVisible(button)) options.onFocus(nodeId);
  };
}

// ─── Frame pipeline ─────────────────────────────────────────────────────────

function startAnimation(
  canvas: HTMLCanvasElement,
  root: HTMLElement,
  stage: HTMLElement | null,
  state: RendererState,
): void {
  if (state.reducedMotion) {
    state.camera = state.goal;
    state.options.view.camera = state.camera;
    drawFrame(canvas, root, stage, state);
    return;
  }
  state.animating = true;
  if (state.rafHandle !== null) return;
  state.lastFrameAt = now();
  const step = () => {
    state.rafHandle = null;
    if (!canvas.isConnected) return;
    const at = now();
    const deltaSeconds = Math.min(0.1, (at - state.lastFrameAt) / 1000);
    state.lastFrameAt = at;
    const advanced = advanceCameraToward(state.camera, state.goal, deltaSeconds);
    state.camera = advanced.camera;
    state.options.view.camera = state.camera;
    drawFrame(canvas, root, stage, state);
    if (!advanced.done) {
      schedule();
    } else {
      state.animating = false;
    }
  };
  const schedule = () => {
    if (typeof requestAnimationFrame === "function") {
      state.rafKind = "animation-frame";
      state.rafHandle = requestAnimationFrame(step);
    } else {
      state.rafKind = "timeout";
      state.rafHandle = setTimeout(step, 16);
    }
  };
  schedule();
}

function rebuildScene(state: RendererState, viewport: SceneViewport): GraphScene {
  const scene = buildGraphScene({
    layout: state.options.layout!,
    communities: state.options.snapshot?.communities ?? [],
    camera: state.camera,
    viewport,
    overrides: state.options.view.overrides,
    selectedNodeIds: state.options.selectedNodeIds,
    hoveredNodeId: state.hoveredNodeId,
  });
  state.lastScene = scene;
  return scene;
}

function drawFrame(
  canvas: HTMLCanvasElement,
  root: HTMLElement,
  stage: HTMLElement | null,
  state: RendererState,
): void {
  const viewport = canvasViewport(stage);
  state.canvasSize = resizeCanvasIfNeeded(canvas, viewport, state.canvasSize);
  const scene = rebuildScene(state, viewport);
  if (!drawThree(canvas, viewport, scene)) {
    drawCanvas2d(canvas, scene);
  }
  syncOverlay(root, state, scene);
  canvas.dataset.nonblank = scene.nodes.length > 0 ? "true" : "false";
  canvas.dataset.reducedMotion = state.reducedMotion ? "true" : "false";
  // Debug/E2E handle: the last drawn scene, camera, and viewport.
  (canvas as HTMLCanvasElement & { __kgDebug?: unknown }).__kgDebug = {
    camera: state.camera,
    viewport,
    nodes: scene.nodes.length,
    scene,
  };
}

// ─── HTML overlay: labels, area titles, tooltip ─────────────────────────────

function overlayContainer(root: HTMLElement): HTMLElement | null {
  return root.querySelector<HTMLElement>("[data-kg-labels]");
}

function clearOverlayContainer(root: ParentNode): void {
  const container = (root as HTMLElement).querySelector?.<HTMLElement>("[data-kg-labels]");
  container?.replaceChildren();
}

function syncOverlay(
  root: HTMLElement,
  state: RendererState,
  scene: GraphScene,
): void {
  const container = overlayContainer(root);
  if (!container) return;
  const document = container.ownerDocument;
  const seen = new Set<Element>();

  // Area titles surface when zoomed out, replacing the node-label noise.
  // A light declutter pass nudges titles apart when neighboring clusters
  // overlap so two areas never render on top of each other.
  const placedAreaLabels: Array<{ x: number; y: number; halfWidth: number }> = [];
  for (const hull of scene.hulls) {
    if (hull.labelAlpha <= 0.02) continue;
    let label = container.querySelector<HTMLElement>(`[data-kg-area-label="${cssEscape(hull.areaId)}"]`);
    if (!label) {
      label = document.createElement("div");
      label.className = "os-kg-area-label";
      label.dataset.kgAreaLabel = hull.areaId;
      container.appendChild(label);
    }
    if (label.textContent !== hull.label) label.textContent = hull.label;
    const halfWidth = hull.label.length * 5.4;
    let labelY = hull.labelY;
    for (let attempt = 0; attempt < 3; attempt += 1) {
      const collision = placedAreaLabels.find((placed) =>
        Math.abs(placed.y - labelY) < 24
        && Math.abs(placed.x - hull.labelX) < placed.halfWidth + halfWidth,
      );
      if (!collision) break;
      labelY += 26;
    }
    placedAreaLabels.push({ x: hull.labelX, y: labelY, halfWidth });
    label.style.left = `${hull.labelX.toFixed(1)}px`;
    label.style.top = `${labelY.toFixed(1)}px`;
    label.style.opacity = hull.labelAlpha.toFixed(2);
    label.style.color = hull.color;
    seen.add(label);
  }

  for (const node of scene.nodes) {
    if (node.labelAlpha <= 0.05) continue;
    let label = container.querySelector<HTMLElement>(`.os-kg-label[data-kg-node-id="${cssEscape(node.nodeId)}"]`);
    if (!label) {
      label = document.createElement("button");
      (label as HTMLButtonElement).type = "button";
      label.className = "os-kg-label";
      label.dataset.kgNodeId = node.nodeId;
      container.appendChild(label);
      bindNodeButton(root, label, state.options);
    }
    const text = shortLabel(node.labelText);
    if (label.textContent !== text) label.textContent = text;
    label.style.left = `${node.x.toFixed(1)}px`;
    label.style.top = `${(node.y + node.screenRadius + 3).toFixed(1)}px`;
    label.style.opacity = node.labelAlpha.toFixed(2);
    label.classList.toggle("is-selected", node.emphasis === "selected");
    label.classList.toggle("is-hovered", node.emphasis === "hovered");
    seen.add(label);
  }

  const tooltip = syncTooltip(container, state, scene);
  if (tooltip) seen.add(tooltip);

  for (const child of Array.from(container.children)) {
    if (!seen.has(child) && child !== document.activeElement) {
      child.remove();
    }
  }
}

function syncTooltip(
  container: HTMLElement,
  state: RendererState,
  scene: GraphScene,
): HTMLElement | null {
  const hovered = scene.hoveredNodeId
    ? scene.nodes.find((node) => node.nodeId === scene.hoveredNodeId)
    : null;
  let tooltip = container.querySelector<HTMLElement>("[data-kg-tooltip]");
  if (!hovered) {
    tooltip?.remove();
    return null;
  }
  const document = container.ownerDocument;
  if (!tooltip) {
    tooltip = document.createElement("div");
    tooltip.className = "os-kg-tooltip";
    tooltip.dataset.kgTooltip = "true";
    tooltip.setAttribute("role", "status");
    container.appendChild(tooltip);
  }
  const snapshotNode = state.options.snapshot?.nodes.find((node) => node.id === hovered.nodeId);
  const areas = state.options.snapshot?.communities
    .filter((community) => community.node_ids.includes(hovered.nodeId))
    .map((community) => community.label)
    ?? [];
  const degree = (snapshotNode?.metrics?.indegree ?? 0) + (snapshotNode?.metrics?.outdegree ?? 0);
  tooltip.innerHTML = `
    <strong>${escapeHtml(hovered.labelText)}</strong>
    <span>${escapeHtml(snapshotNode?.kind ?? "node")}${degree ? ` &middot; ${degree} links` : ""}</span>
    ${areas.length > 0 ? `<em>${escapeHtml(areas.join(" · "))}</em>` : ""}
  `;
  positionTooltip(tooltip, hovered.x, hovered.y - hovered.screenRadius - 10);
  return tooltip;
}

function positionTooltip(tooltip: HTMLElement, x: number, y: number): void {
  tooltip.style.left = `${x.toFixed(1)}px`;
  tooltip.style.top = `${Math.max(8, y).toFixed(1)}px`;
}

// ─── WebGL backend ──────────────────────────────────────────────────────────

interface ThreeCanvasState {
  renderer: THREE.WebGLRenderer;
  scene: THREE.Scene;
  camera: THREE.OrthographicCamera;
  graph: THREE.Group;
  viewportKey: string | null;
  contentKey: string | null;
}

const threeCanvasState = new WeakMap<HTMLCanvasElement, ThreeCanvasState>();

function resetThreeCanvasState(canvas: HTMLCanvasElement): void {
  const state = threeCanvasState.get(canvas);
  if (!state) return;
  try {
    disposeObject3D(state.graph);
    state.renderer.dispose();
    state.renderer.forceContextLoss();
  } catch {
    // Fall back to 2D drawing; the next WebGL attempt starts from a fresh state.
  }
  threeCanvasState.delete(canvas);
}

/**
 * GPU rasterization of the already-projected scene. The scene is authored in
 * screen space by the shared projector, so the WebGL path draws with a
 * simple orthographic screen-space camera and can never disagree with the
 * hit-testing, labels, or the 2D fallback.
 */
function drawThree(
  canvas: HTMLCanvasElement,
  viewport: { width: number; height: number; ratio: number },
  scene: GraphScene,
): boolean {
  try {
    const three = threeStateFor(canvas);
    const viewportKey = `${viewport.width}:${viewport.height}:${viewport.ratio}`;
    if (three.viewportKey !== viewportKey) {
      three.renderer.setPixelRatio(viewport.ratio);
      three.renderer.setSize(viewport.width, viewport.height, false);
      three.camera.left = 0;
      three.camera.right = viewport.width;
      three.camera.top = 0;
      three.camera.bottom = viewport.height;
      three.camera.updateProjectionMatrix();
      three.viewportKey = viewportKey;
    }
    syncThreeScene(three, scene);
    three.renderer.setClearColor(graphSurfaceColorInt, 1);
    three.renderer.clear(true, true, true);
    three.renderer.render(three.scene, three.camera);
    return true;
  } catch {
    resetThreeCanvasState(canvas);
    return false;
  }
}

function threeStateFor(canvas: HTMLCanvasElement): ThreeCanvasState {
  const existing = threeCanvasState.get(canvas);
  if (existing) return existing;
  const contextAttributes = { alpha: false, antialias: true, preserveDrawingBuffer: true };
  const context = (canvas.getContext("webgl2", contextAttributes) as WebGL2RenderingContext | null)
    ?? (canvas.getContext("webgl", contextAttributes) as WebGLRenderingContext | null);
  if (!context) throw new Error("WebGL is unavailable");
  const renderer = new THREE.WebGLRenderer({
    canvas,
    context,
    alpha: false,
    antialias: true,
    preserveDrawingBuffer: true,
  });
  const scene = new THREE.Scene();
  const graph = new THREE.Group();
  scene.add(graph);
  // Screen-space orthographic camera: with top=0 and bottom=height the
  // default -Z view maps our y-down screen coordinates directly; no up-flip
  // or lookAt (those would mirror x out of the clip volume).
  const camera = new THREE.OrthographicCamera(0, 1, 0, 1, -1000, 1000);
  camera.position.set(0, 0, 500);
  const state: ThreeCanvasState = { renderer, scene, camera, graph, viewportKey: null, contentKey: null };
  threeCanvasState.set(canvas, state);
  return state;
}

function syncThreeScene(three: ThreeCanvasState, scene: GraphScene): void {
  disposeObject3D(three.graph);
  three.graph.clear();

  let order = 0;
  for (const hull of scene.hulls) {
    if (hull.outline.length < 3) continue;
    const shape = new THREE.Shape(hull.outline.map((point) => new THREE.Vector2(point.x, point.y)));
    const mesh = new THREE.Mesh(
      new THREE.ShapeGeometry(shape),
      new THREE.MeshBasicMaterial({
        color: new THREE.Color(hull.color),
        transparent: true,
        opacity: hull.alpha,
        depthWrite: false,
        side: THREE.DoubleSide,
      }),
    );
    mesh.renderOrder = order++;
    three.graph.add(mesh);
  }

  const edgeGroups = new Map<string, { positions: number[]; color: string; alpha: number }>();
  for (const edge of scene.edges) {
    const alphaBucket = Math.round(edge.alpha * 20) / 20;
    const color = edge.emphasized ? nodeEmphasisColor : "#7d94a8";
    const key = `${color}:${alphaBucket}`;
    const group = edgeGroups.get(key) ?? { positions: [], color, alpha: alphaBucket };
    group.positions.push(edge.x1, edge.y1, 0, edge.x2, edge.y2, 0);
    edgeGroups.set(key, group);
  }
  for (const group of edgeGroups.values()) {
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.Float32BufferAttribute(group.positions, 3));
    const lines = new THREE.LineSegments(
      geometry,
      new THREE.LineBasicMaterial({
        color: new THREE.Color(group.color),
        transparent: true,
        opacity: group.alpha,
        depthWrite: false,
      }),
    );
    lines.renderOrder = order++;
    three.graph.add(lines);
  }

  const nodeGroups = new Map<string, { nodes: Array<{ x: number; y: number; r: number }>; color: string; alpha: number }>();
  for (const node of scene.nodes) {
    const color = node.emphasis === "hovered" || node.emphasis === "selected" ? nodeEmphasisColor : node.color;
    const alphaBucket = Math.round(node.alpha * 10) / 10;
    const key = `${color}:${alphaBucket}`;
    const radius = node.emphasis === "hovered" || node.emphasis === "selected"
      ? node.screenRadius * 1.3
      : node.screenRadius;
    const group = nodeGroups.get(key) ?? { nodes: [], color, alpha: alphaBucket };
    group.nodes.push({ x: node.x, y: node.y, r: radius });
    nodeGroups.set(key, group);
  }
  for (const group of nodeGroups.values()) {
    const geometry = new THREE.CircleGeometry(1, 22);
    const material = new THREE.MeshBasicMaterial({
      color: new THREE.Color(group.color),
      transparent: group.alpha < 1,
      opacity: group.alpha,
      depthWrite: false,
    });
    const mesh = new THREE.InstancedMesh(geometry, material, group.nodes.length);
    const matrix = new THREE.Matrix4();
    group.nodes.forEach((node, index) => {
      matrix.compose(
        new THREE.Vector3(node.x, node.y, 0),
        new THREE.Quaternion(),
        new THREE.Vector3(node.r, node.r, 1),
      );
      mesh.setMatrixAt(index, matrix);
    });
    mesh.instanceMatrix.needsUpdate = true;
    mesh.renderOrder = order++;
    three.graph.add(mesh);
  }
}

function disposeObject3D(object: THREE.Object3D): void {
  object.traverse((child) => {
    if (child === object) return;
    disposeRenderable(child);
  });
}

function disposeRenderable(object: THREE.Object3D): void {
  const renderable = object as THREE.Object3D & {
    geometry?: { dispose(): void };
    material?: THREE.Material | THREE.Material[];
  };
  renderable.geometry?.dispose();
  const materials = renderable.material
    ? Array.isArray(renderable.material) ? renderable.material : [renderable.material]
    : [];
  for (const material of materials) material.dispose();
}

// ─── 2D fallback ────────────────────────────────────────────────────────────

function drawCanvas2d(canvas: HTMLCanvasElement, scene: GraphScene): void {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const ratio = canvas.width / Number.parseFloat(canvas.style.width || String(canvas.width));
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  ctx.fillStyle = graphSurfaceColor;
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  for (const hull of scene.hulls) {
    if (hull.outline.length < 3) continue;
    ctx.beginPath();
    ctx.moveTo(hull.outline[0].x, hull.outline[0].y);
    for (const point of hull.outline.slice(1)) ctx.lineTo(point.x, point.y);
    ctx.closePath();
    ctx.globalAlpha = hull.alpha;
    ctx.fillStyle = hull.color;
    ctx.fill();
    ctx.globalAlpha = Math.min(1, hull.alpha * 2);
    ctx.lineWidth = 10;
    ctx.strokeStyle = hull.color;
    ctx.stroke();
  }

  ctx.lineWidth = 1;
  for (const edge of scene.edges) {
    ctx.globalAlpha = edge.alpha;
    ctx.strokeStyle = edge.emphasized ? nodeEmphasisColor : "#7d94a8";
    ctx.lineWidth = edge.emphasized ? 1.6 : 1;
    ctx.beginPath();
    ctx.moveTo(edge.x1, edge.y1);
    ctx.lineTo(edge.x2, edge.y2);
    ctx.stroke();
  }

  for (const node of scene.nodes) {
    const emphasized = node.emphasis === "hovered" || node.emphasis === "selected";
    ctx.globalAlpha = node.alpha;
    ctx.beginPath();
    ctx.fillStyle = emphasized ? nodeEmphasisColor : node.color;
    ctx.arc(node.x, node.y, emphasized ? node.screenRadius * 1.3 : node.screenRadius, 0, Math.PI * 2);
    ctx.fill();
    ctx.lineWidth = emphasized ? 2.4 : 1.4;
    ctx.strokeStyle = "#ffffff";
    ctx.stroke();
  }
  ctx.globalAlpha = 1;
}

// ─── Shared helpers ─────────────────────────────────────────────────────────

function resizeCanvasIfNeeded(
  canvas: HTMLCanvasElement,
  viewport: { width: number; height: number; ratio: number },
  previous: { width: number; height: number; cssWidth: number; cssHeight: number },
): { width: number; height: number; cssWidth: number; cssHeight: number } {
  const width = Math.floor(viewport.width * viewport.ratio);
  const height = Math.floor(viewport.height * viewport.ratio);
  if (
    previous.width === width
    && previous.height === height
    && previous.cssWidth === viewport.width
    && previous.cssHeight === viewport.height
  ) {
    return previous;
  }
  canvas.width = width;
  canvas.height = height;
  canvas.style.width = `${viewport.width}px`;
  canvas.style.height = `${viewport.height}px`;
  return { width, height, cssWidth: viewport.width, cssHeight: viewport.height };
}

function renderStatus(state: GraphState): string {
  const { layoutError: error, layoutStatus: status } = state;
  if (status === "failed") {
    return `<span class="os-kg-status os-kg-status-failed" data-testid="knowledge-graph-status">Graph unavailable${error ? `: ${escapeHtml(error)}` : ""}</span>`;
  }
  if (state.freshnessStatus === "stale") {
    return `<span class="os-kg-status os-kg-status-stale" data-testid="knowledge-graph-status">Graph stale: ${escapeHtml(state.staleBundleIds.join(", ") || "pending refresh")}</span>`;
  }
  if (state.freshnessStatus === "warning") {
    return `<span class="os-kg-status os-kg-status-warning" data-testid="knowledge-graph-status">Graph warnings: ${escapeHtml(state.warningBundleIds.join(", ") || "review graph metrics")}</span>`;
  }
  if (status === "loading" || status === "stabilizing") {
    return `<span class="os-kg-status" data-testid="knowledge-graph-status">Stabilizing</span>`;
  }
  if (status === "ready") return `<span class="os-kg-status" data-testid="knowledge-graph-status">Ready</span>`;
  return `<span class="os-kg-status" data-testid="knowledge-graph-status">Idle</span>`;
}

function renderSelectedInspector(snapshot: MemoryGraphSnapshot | null, selectedNodeIds: readonly string[]): string {
  const selected = new Set(selectedNodeIds);
  const node = snapshot?.nodes.find((candidate) => selected.has(candidate.id)) ?? null;
  if (!node) {
    return `<section class="os-kg-inspector" data-testid="knowledge-graph-inspector"><h3>Inspector</h3><p>No node selected</p></section>`;
  }
  const community = node.metrics?.community_id
    ? snapshot?.communities.find((candidate) => candidate.id === node.metrics.community_id)?.label ?? node.metrics.community_id
    : "None";
  return `
    <section class="os-kg-inspector" data-testid="knowledge-graph-inspector">
      <h3>${escapeHtml(node.label)}</h3>
      <dl>
        <dt>Kind</dt><dd>${escapeHtml(node.kind)}</dd>
        <dt>Visibility</dt><dd>${escapeHtml(node.visibility ?? "unknown")}</dd>
        <dt>Community</dt><dd>${escapeHtml(community)}</dd>
        <dt>Tags</dt><dd>${escapeHtml(node.tags.join(", ") || "None")}</dd>
      </dl>
    </section>
  `;
}

function renderFallbackList(snapshot: MemoryGraphSnapshot | null, selectedNodeIds: readonly string[]): string {
  if (!snapshot || snapshot.nodes.length === 0) {
    return `<div class="os-empty">No graph data available.</div>`;
  }
  const selected = new Set(selectedNodeIds);
  return `
    <ul class="os-kg-list" data-testid="knowledge-graph-node-list" aria-label="Visible graph nodes">
      ${snapshot.nodes.map((node) => `
        <li class="${selected.has(node.id) ? "is-selected" : ""}">
          <button type="button" data-kg-node-id="${escapeAttr(node.id)}" ${selected.has(node.id) ? `aria-current="true"` : ""}>${escapeHtml(node.label)}</button>
          <span>${escapeHtml(node.kind)}</span>
        </li>
      `).join("")}
    </ul>
  `;
}

function prefersReducedMotion(): boolean {
  if (typeof globalThis.matchMedia !== "function") return false;
  try {
    return globalThis.matchMedia("(prefers-reduced-motion: reduce)").matches;
  } catch {
    return false;
  }
}

function graphListNavigationDirection(key: string): -1 | 1 | "first" | "last" | null {
  switch (key) {
    case "ArrowUp":
    case "ArrowLeft":
      return -1;
    case "ArrowDown":
    case "ArrowRight":
      return 1;
    case "Home":
      return "first";
    case "End":
      return "last";
    default:
      return null;
  }
}

function canvasViewport(stage: HTMLElement | null): { width: number; height: number; ratio: number } {
  const rect = stage?.getBoundingClientRect();
  const width = Math.max(320, Math.floor(rect?.width || 640));
  const height = Math.max(260, Math.floor(rect?.height || 360));
  return { width, height, ratio: globalThis.devicePixelRatio || 1 };
}

function now(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}

function shortLabel(label: string): string {
  return label.length > 34 ? `${label.slice(0, 31)}...` : label;
}

function matchesFocusVisible(element: Element): boolean {
  try {
    return element.matches(":focus-visible");
  } catch {
    // Environments without :focus-visible (older jsdom) keep the previous
    // behavior of treating any focus as intentional.
    return true;
  }
}

function cssEscape(value: string): string {
  if (typeof CSS !== "undefined" && typeof CSS.escape === "function") {
    return CSS.escape(value);
  }
  return value.replace(/["\\\]]/g, "\\$&");
}
