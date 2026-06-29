import type {
  GraphLayoutResult,
  GraphState,
  LayoutStatus,
  MemoryGraphSnapshot,
} from "@opensymphony/graph";
import * as THREE from "three";
import { escapeAttr, escapeHtml } from "./html.js";

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

export interface KnowledgeGraphViewState {
  scale: number;
  dx: number;
  dy: number;
}

export function renderKnowledgeGraphSurface(surface: KnowledgeGraphSurface): string {
  const { snapshot, layout, state } = surface;
  const status = state.layoutStatus;
  const summary = snapshot
    ? `${snapshot.nodes.length} nodes / ${snapshot.edges.length} edges`
    : "No graph snapshot";
  return `
    <div class="os-knowledge-graph" data-testid="knowledge-graph-renderer" data-layout-status="${escapeAttr(status)}">
      <div class="os-knowledge-toolbar">
        <div>
          <strong>Knowledge Graph</strong>
          <span>${escapeHtml(summary)}</span>
        </div>
        ${renderStatus(status, state.layoutError)}
      </div>
      <div class="os-knowledge-stage" data-kg-stage>
        <canvas class="os-knowledge-canvas" data-testid="knowledge-graph-canvas" aria-label="Knowledge Graph canvas"></canvas>
        <div class="os-knowledge-labels" data-kg-labels>${renderLabels(layout, state.selectedNodeIds)}</div>
      </div>
      ${renderFallbackList(snapshot, state.selectedNodeIds)}
    </div>
  `;
}

export function mountKnowledgeGraphRenderer(
  root: HTMLElement,
  options: KnowledgeGraphMountOptions,
): void {
  const canvas = root.querySelector<HTMLCanvasElement>("[data-testid='knowledge-graph-canvas']");
  if (!canvas || !options.snapshot || !options.layout) return;
  const stage = canvas.closest<HTMLElement>("[data-kg-stage]");
  const view = {
    scale: options.view.scale,
    dx: options.view.dx,
    dy: options.view.dy,
    dragging: false,
    moved: false,
    px: 0,
    py: 0,
  };
  let canvasSize = { width: 0, height: 0, cssWidth: 0, cssHeight: 0 };
  const draw = () => {
    const viewport = canvasViewport(stage);
    canvasSize = resizeCanvasIfNeeded(canvas, viewport, canvasSize);
    if (!drawThree(canvas, viewport, options.layout!, options.selectedNodeIds, view)) {
      drawCanvas2d(canvas, options.layout!, options.selectedNodeIds, view);
    }
    syncLabels(root, options.layout!, view, viewport);
    canvas.dataset.nonblank = options.layout!.nodes.length > 0 ? "true" : "false";
  };
  draw();
  canvas.onwheel = (event) => {
    event.preventDefault();
    view.scale = clamp(view.scale * (event.deltaY < 0 ? 1.08 : 0.92), 0.45, 3);
    syncViewState(options.view, view);
    draw();
  };
  canvas.onpointerdown = (event) => {
    canvas.setPointerCapture(event.pointerId);
    view.dragging = true;
    view.moved = false;
    view.px = event.clientX;
    view.py = event.clientY;
  };
  canvas.onpointermove = (event) => {
    if (!view.dragging) return;
    const dx = event.clientX - view.px;
    const dy = event.clientY - view.py;
    if (Math.abs(dx) + Math.abs(dy) > 2) view.moved = true;
    view.dx += dx;
    view.dy += dy;
    view.px = event.clientX;
    view.py = event.clientY;
    syncViewState(options.view, view);
    draw();
  };
  canvas.onpointerup = (event) => {
    view.dragging = false;
    canvas.releasePointerCapture(event.pointerId);
    if (!view.moved) {
      const nodeId = nearestNode(options.layout!, event.offsetX, event.offsetY, canvasViewport(stage), view);
      if (nodeId) options.onSelect(nodeId);
    }
  };
  root.querySelectorAll<HTMLElement>("[data-kg-node-id]").forEach((button) => {
    button.addEventListener("click", () => {
      const nodeId = button.dataset.kgNodeId;
      if (nodeId) options.onSelect(nodeId);
    });
    button.addEventListener("focus", () => {
      const nodeId = button.dataset.kgNodeId;
      if (nodeId) options.onFocus(nodeId);
    });
  });
}

export function disposeKnowledgeGraphRenderer(root: ParentNode): void {
  root.querySelectorAll<HTMLCanvasElement>("[data-testid='knowledge-graph-canvas']").forEach((canvas) => {
    const state = threeCanvasState.get(canvas);
    if (!state) return;
    disposeObject3D(state.graph);
    state.renderer.dispose();
    state.renderer.forceContextLoss();
    threeCanvasState.delete(canvas);
  });
}

function syncViewState(
  target: KnowledgeGraphViewState,
  view: KnowledgeGraphViewState,
): void {
  target.scale = view.scale;
  target.dx = view.dx;
  target.dy = view.dy;
}

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

function renderStatus(status: LayoutStatus, error: string | null): string {
  if (status === "failed") {
    return `<span class="os-kg-status os-kg-status-failed">Error${error ? `: ${escapeHtml(error)}` : ""}</span>`;
  }
  if (status === "loading" || status === "stabilizing") {
    return `<span class="os-kg-status">Stabilizing</span>`;
  }
  if (status === "ready") return `<span class="os-kg-status">Ready</span>`;
  return `<span class="os-kg-status">Idle</span>`;
}

function renderLabels(layout: GraphLayoutResult | null, selectedNodeIds: readonly string[]): string {
  if (!layout) return "";
  const selected = new Set(selectedNodeIds);
  const showAll = layout.nodes.length <= 80;
  return layout.nodes
    .filter((node) => showAll || selected.has(node.nodeId) || node.kind === "concept")
    .map((node) => {
      const left = (node.x / layout.width) * 100;
      const top = (node.y / layout.height) * 100;
      const picked = selected.has(node.nodeId) ? " is-selected" : "";
      return `<button type="button" class="os-kg-label${picked}" style="left:${left.toFixed(2)}%;top:${top.toFixed(2)}%" data-kg-node-id="${escapeAttr(node.nodeId)}">${escapeHtml(shortLabel(node.label))}</button>`;
    })
    .join("");
}

function renderFallbackList(snapshot: MemoryGraphSnapshot | null, selectedNodeIds: readonly string[]): string {
  if (!snapshot || snapshot.nodes.length === 0) {
    return `<div class="os-empty">No graph data available.</div>`;
  }
  const selected = new Set(selectedNodeIds);
  return `
    <ul class="os-kg-list" aria-label="Visible graph nodes">
      ${snapshot.nodes.slice(0, 20).map((node) => `
        <li class="${selected.has(node.id) ? "is-selected" : ""}">
          <button type="button" data-kg-node-id="${escapeAttr(node.id)}">${escapeHtml(node.label)}</button>
          <span>${escapeHtml(node.kind)}</span>
        </li>
      `).join("")}
    </ul>
  `;
}

interface ThreeCanvasState {
  renderer: THREE.WebGLRenderer;
  scene: THREE.Scene;
  camera: THREE.OrthographicCamera;
  graph: THREE.Group;
  layoutKey: string | null;
  selectionKey: string | null;
}

const threeCanvasState = new WeakMap<HTMLCanvasElement, ThreeCanvasState>();

function drawThree(
  canvas: HTMLCanvasElement,
  viewport: { width: number; height: number; ratio: number },
  layout: GraphLayoutResult,
  selectedNodeIds: readonly string[],
  view: { scale: number; dx: number; dy: number },
): boolean {
  try {
    const state = threeStateFor(canvas);
    state.renderer.setPixelRatio(viewport.ratio);
    state.renderer.setSize(viewport.width, viewport.height, false);
    state.renderer.setClearColor(0xf8fafc, 1);
    state.renderer.clear(true, true, true);
    state.camera.left = -viewport.width / 2;
    state.camera.right = viewport.width / 2;
    state.camera.top = viewport.height / 2;
    state.camera.bottom = -viewport.height / 2;
    state.camera.updateProjectionMatrix();
    syncGraphObjects(state, layout, selectedNodeIds);
    state.graph.scale.set(view.scale, view.scale, 1);
    state.graph.position.set(view.dx, -view.dy, 0);
    state.renderer.render(state.scene, state.camera);
    return true;
  } catch {
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
  const camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 1, 2000);
  camera.position.set(0, 0, 800);
  const state = { renderer, scene, camera, graph, layoutKey: null, selectionKey: null };
  threeCanvasState.set(canvas, state);
  return state;
}

function syncGraphObjects(
  state: ThreeCanvasState,
  layout: GraphLayoutResult,
  selectedNodeIds: readonly string[],
): void {
  const layoutKey = graphLayoutKey(layout);
  const selectionKey = [...selectedNodeIds].sort().join("\u0000");
  if (state.layoutKey === layoutKey && state.selectionKey === selectionKey) return;
  disposeObject3D(state.graph);
  state.graph.clear();
  state.graph.add(edgeSegments(layout));
  state.graph.add(nodeInstances(layout, selectedNodeIds));
  state.layoutKey = layoutKey;
  state.selectionKey = selectionKey;
}

function disposeObject3D(object: THREE.Object3D): void {
  object.traverse((child) => {
    if (child === object) return;
    disposeRenderable(child);
  });
}

function disposeRenderable(object: THREE.Object3D): void {
    const mesh = object as THREE.Mesh | THREE.LineSegments;
    if ("geometry" in mesh) mesh.geometry.dispose();
    const materials = "material" in mesh
      ? Array.isArray(mesh.material) ? mesh.material : [mesh.material]
      : [];
    for (const material of materials) material.dispose();
}

function graphLayoutKey(layout: GraphLayoutResult): string {
  const nodes = layout.nodes
    .map((node) => `${node.nodeId}:${node.x.toFixed(2)}:${node.y.toFixed(2)}:${node.z.toFixed(2)}:${node.radius}`)
    .join("|");
  const edges = layout.edges
    .map((edge) => `${edge.edgeId}:${edge.sourceId}:${edge.targetId}`)
    .join("|");
  return `${layout.kind}:${layout.width}:${layout.height}:${nodes}:${edges}`;
}

function edgeSegments(layout: GraphLayoutResult): THREE.LineSegments {
  const byId = new Map(layout.nodes.map((node) => [node.nodeId, node]));
  const positions: number[] = [];
  for (const edge of layout.edges) {
    const source = byId.get(edge.sourceId);
    const target = byId.get(edge.targetId);
    if (!source || !target) continue;
    positions.push(...projectPoint(source.x, source.y, 0, layout, { scale: 1, dx: 0, dy: 0 }));
    positions.push(...projectPoint(target.x, target.y, 0, layout, { scale: 1, dx: 0, dy: 0 }));
  }
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  return new THREE.LineSegments(geometry, new THREE.LineBasicMaterial({ color: 0x8aa4b8, transparent: true, opacity: 0.62 }));
}

function nodeInstances(
  layout: GraphLayoutResult,
  selectedNodeIds: readonly string[],
): THREE.Group {
  const selected = new Set(selectedNodeIds);
  const grouped = new Map<string, GraphLayoutResult["nodes"]>();
  for (const node of layout.nodes) {
    const color = selected.has(node.nodeId) ? "#c2410c" : colorForKind(node.kind);
    const nodes = grouped.get(color);
    if (nodes) {
      nodes.push(node);
    } else {
      grouped.set(color, [node]);
    }
  }
  const group = new THREE.Group();
  for (const [color, nodes] of grouped) {
    const geometry = new THREE.CircleGeometry(1, 24);
    const material = new THREE.MeshBasicMaterial({ color });
    const mesh = new THREE.InstancedMesh(geometry, material, nodes.length);
    const matrix = new THREE.Matrix4();
    nodes.forEach((node, index) => {
      const [x, y, z] = projectPoint(node.x, node.y, node.z, layout, { scale: 1, dx: 0, dy: 0 });
      const scale = selected.has(node.nodeId) ? node.radius + 4 : node.radius;
      matrix.compose(new THREE.Vector3(x, y, z), new THREE.Quaternion(), new THREE.Vector3(scale, scale, 1));
      mesh.setMatrixAt(index, matrix);
    });
    mesh.instanceMatrix.needsUpdate = true;
    group.add(mesh);
  }
  return group;
}

function drawCanvas2d(
  canvas: HTMLCanvasElement,
  layout: GraphLayoutResult,
  selectedNodeIds: readonly string[],
  view: { scale: number; dx: number; dy: number },
): void {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const ratio = canvas.width / Number.parseFloat(canvas.style.width || String(canvas.width));
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  ctx.fillStyle = "#f8fafc";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  const selected = new Set(selectedNodeIds);
  const byId = new Map(layout.nodes.map((node) => [node.nodeId, node]));
  ctx.strokeStyle = "#8aa4b8";
  ctx.lineWidth = 4;
  ctx.globalAlpha = 0.72;
  for (const edge of layout.edges) {
    const source = byId.get(edge.sourceId);
    const target = byId.get(edge.targetId);
    if (!source || !target) continue;
    const a = canvasPoint(source.x, source.y, layout, view);
    const b = canvasPoint(target.x, target.y, layout, view);
    ctx.beginPath();
    ctx.moveTo(a.x, a.y);
    ctx.lineTo(b.x, b.y);
    ctx.stroke();
  }
  ctx.globalAlpha = 1;
  for (const node of layout.nodes) {
    const p = canvasPoint(node.x, node.y, layout, view);
    ctx.beginPath();
    ctx.fillStyle = selected.has(node.nodeId) ? "#c2410c" : colorForKind(node.kind);
    ctx.arc(p.x, p.y, (selected.has(node.nodeId) ? node.radius + 12 : node.radius + 9) * view.scale, 0, Math.PI * 2);
    ctx.fill();
    ctx.strokeStyle = "#ffffff";
    ctx.lineWidth = 2;
    ctx.stroke();
  }
}

function nearestNode(
  layout: GraphLayoutResult,
  x: number,
  y: number,
  viewport: { width: number; height: number },
  view: { scale: number; dx: number; dy: number },
): string | null {
  let best: { id: string; distance: number } | null = null;
  for (const node of layout.nodes) {
    const p = canvasPoint(node.x, node.y, layout, view, viewport);
    const distance = Math.hypot(p.x - x, p.y - y);
    if (distance <= 24 && (!best || distance < best.distance)) best = { id: node.nodeId, distance };
  }
  return best?.id ?? null;
}

function syncLabels(
  root: HTMLElement,
  layout: GraphLayoutResult,
  view: { scale: number; dx: number; dy: number },
  viewport: { width: number; height: number },
): void {
  root.querySelectorAll<HTMLElement>(".os-kg-label[data-kg-node-id]").forEach((label) => {
    const nodeId = label.dataset.kgNodeId;
    const node = layout.nodes.find((candidate) => candidate.nodeId === nodeId);
    if (!node) return;
    const point = canvasPoint(node.x, node.y, layout, view, viewport);
    label.style.left = `${point.x}px`;
    label.style.top = `${point.y}px`;
  });
}

function canvasViewport(stage: HTMLElement | null): { width: number; height: number; ratio: number } {
  const rect = stage?.getBoundingClientRect();
  const width = Math.max(320, Math.floor(rect?.width || 640));
  const height = Math.max(260, Math.floor(rect?.height || 360));
  return { width, height, ratio: globalThis.devicePixelRatio || 1 };
}

function projectPoint(
  x: number,
  y: number,
  z: number,
  layout: GraphLayoutResult,
  view: { scale: number; dx: number; dy: number },
): [number, number, number] {
  return [
    (x - layout.width / 2) * view.scale + view.dx,
    (layout.height / 2 - y) * view.scale - view.dy,
    z,
  ];
}

function canvasPoint(
  x: number,
  y: number,
  layout: GraphLayoutResult,
  view: { scale: number; dx: number; dy: number },
  viewport = { width: Number.NaN, height: Number.NaN },
): { x: number; y: number } {
  const width = Number.isNaN(viewport.width) ? Math.max(320, layout.width) : viewport.width;
  const height = Number.isNaN(viewport.height) ? Math.max(260, layout.height) : viewport.height;
  return {
    x: width / 2 + (x - layout.width / 2) * view.scale + view.dx,
    y: height / 2 + (y - layout.height / 2) * view.scale + view.dy,
  };
}

function shortLabel(label: string): string {
  return label.length > 34 ? `${label.slice(0, 31)}...` : label;
}

function colorForKind(kind: string): string {
  switch (kind) {
    case "concept":
      return "#1f6f8b";
    case "bundle":
      return "#334155";
    case "tag":
      return "#7c3aed";
    case "source_ref":
      return "#0f766e";
    default:
      return "#64748b";
  }
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}
