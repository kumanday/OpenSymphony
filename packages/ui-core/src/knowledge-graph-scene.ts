import type {
  GraphLayoutResult,
  MemoryGraphSnapshot,
} from "@opensymphony/graph";

/**
 * Pure scene model for the knowledge-graph command center.
 *
 * Everything the renderer draws — node positions, hover emphasis, diffuse
 * area hulls, zoom-dependent label opacities — is computed here from an
 * orbital perspective camera, with no DOM or WebGL dependencies. The WebGL
 * backend mirrors this projection with a real THREE.PerspectiveCamera; a
 * parity unit test keeps the two in lockstep so hit-testing, HTML labels,
 * and the 2D fallback always agree with what the GPU rasterizes.
 *
 * Conventions: layout space is the 2D box produced by `@opensymphony/graph`
 * (x right, y down). World space re-centers it at the origin with y up and
 * spreads nodes in z by community so orbiting reveals genuine depth.
 */

export interface GraphCameraState {
  targetX: number;
  targetY: number;
  targetZ: number;
  /** Dolly distance from the target, world units. */
  distance: number;
  /** Orbit around the world Y axis, radians. */
  yaw: number;
  /** Orbit toward the world Y pole, radians, clamped. */
  pitch: number;
}

export interface KnowledgeGraphViewState {
  camera: GraphCameraState | null;
  /**
   * Node drag overrides keyed by node id: x/y in layout coordinates, plus
   * the world-space z of the drop point. Dragging happens on a camera-facing
   * plane, so under an orbited camera the dropped point's z differs from the
   * node's community depth — persisting it keeps the node under the cursor.
   */
  overrides: Map<string, { x: number; y: number; z?: number }>;
}

export function createKnowledgeGraphViewState(): KnowledgeGraphViewState {
  return { camera: null, overrides: new Map() };
}

export const graphCameraFovDegrees = 55;
export const graphCameraNear = 6;
export const graphCameraFar = 12_000;
const minCameraDistance = 90;
const maxCameraDistance = 7_000;
const maxCameraPitch = 1.15;

export interface SceneViewport {
  width: number;
  height: number;
}

export interface WorldPoint {
  x: number;
  y: number;
  z: number;
}

export interface ProjectedPoint {
  x: number;
  y: number;
  /** Distance along the camera forward axis; ≤ 0 means behind the camera. */
  depth: number;
  /** Screen pixels per world unit at this depth. */
  pixelsPerWorldUnit: number;
  visible: boolean;
}

// ─── Camera math ────────────────────────────────────────────────────────────

export interface CameraBasis {
  position: WorldPoint;
  right: WorldPoint;
  up: WorldPoint;
  forward: WorldPoint;
}

export function cameraBasis(camera: GraphCameraState): CameraBasis {
  const cosPitch = Math.cos(camera.pitch);
  const offset: WorldPoint = {
    x: Math.sin(camera.yaw) * cosPitch * camera.distance,
    y: Math.sin(camera.pitch) * camera.distance,
    z: Math.cos(camera.yaw) * cosPitch * camera.distance,
  };
  const position: WorldPoint = {
    x: camera.targetX + offset.x,
    y: camera.targetY + offset.y,
    z: camera.targetZ + offset.z,
  };
  const forward = normalize({
    x: camera.targetX - position.x,
    y: camera.targetY - position.y,
    z: camera.targetZ - position.z,
  });
  const worldUp: WorldPoint = { x: 0, y: 1, z: 0 };
  const right = normalize(cross(forward, worldUp));
  const up = cross(right, forward);
  return { position, right, up, forward };
}

export function projectWorldPoint(
  camera: GraphCameraState,
  viewport: SceneViewport,
  point: WorldPoint,
  basis: CameraBasis = cameraBasis(camera),
): ProjectedPoint {
  const relative: WorldPoint = {
    x: point.x - basis.position.x,
    y: point.y - basis.position.y,
    z: point.z - basis.position.z,
  };
  const viewX = dot(relative, basis.right);
  const viewY = dot(relative, basis.up);
  const depth = dot(relative, basis.forward);
  if (depth <= graphCameraNear * 0.5) {
    return { x: Number.NaN, y: Number.NaN, depth, pixelsPerWorldUnit: 0, visible: false };
  }
  const f = 1 / Math.tan((graphCameraFovDegrees * Math.PI) / 360);
  const aspect = viewport.width / Math.max(1, viewport.height);
  const ndcX = (f / aspect) * (viewX / depth);
  const ndcY = f * (viewY / depth);
  const pixelsPerWorldUnit = (f * viewport.height) / (2 * depth);
  return {
    x: ((ndcX + 1) / 2) * viewport.width,
    y: ((1 - ndcY) / 2) * viewport.height,
    depth,
    pixelsPerWorldUnit,
    visible: true,
  };
}

/** Ray from the camera through a screen pixel, in world space. */
export function screenRay(
  camera: GraphCameraState,
  viewport: SceneViewport,
  screenX: number,
  screenY: number,
): { origin: WorldPoint; direction: WorldPoint } {
  const basis = cameraBasis(camera);
  const f = 1 / Math.tan((graphCameraFovDegrees * Math.PI) / 360);
  const aspect = viewport.width / Math.max(1, viewport.height);
  const ndcX = (screenX / viewport.width) * 2 - 1;
  const ndcY = 1 - (screenY / viewport.height) * 2;
  const direction = normalize(add(
    add(scale(basis.right, (ndcX * aspect) / f), scale(basis.up, ndcY / f)),
    basis.forward,
  ));
  return { origin: basis.position, direction };
}

/**
 * Intersect a screen ray with the camera-facing plane through `worldPoint`.
 * This is the drag plane: moving a node keeps it at its current depth.
 */
export function unprojectToPlaneThroughPoint(
  camera: GraphCameraState,
  viewport: SceneViewport,
  screenX: number,
  screenY: number,
  worldPoint: WorldPoint,
): WorldPoint | null {
  const basis = cameraBasis(camera);
  const ray = screenRay(camera, viewport, screenX, screenY);
  const planeNormal = basis.forward;
  const denominator = dot(ray.direction, planeNormal);
  if (Math.abs(denominator) < 1e-6) return null;
  const t = dot(
    { x: worldPoint.x - ray.origin.x, y: worldPoint.y - ray.origin.y, z: worldPoint.z - ray.origin.z },
    planeNormal,
  ) / denominator;
  if (t <= 0) return null;
  return add(ray.origin, scale(ray.direction, t));
}

export function panCamera(
  camera: GraphCameraState,
  viewport: SceneViewport,
  deltaXPixels: number,
  deltaYPixels: number,
): GraphCameraState {
  const basis = cameraBasis(camera);
  const worldPerPixel = (2 * camera.distance * Math.tan((graphCameraFovDegrees * Math.PI) / 360)) / Math.max(1, viewport.height);
  const shift = add(
    scale(basis.right, -deltaXPixels * worldPerPixel),
    scale(basis.up, deltaYPixels * worldPerPixel),
  );
  return {
    ...camera,
    targetX: camera.targetX + shift.x,
    targetY: camera.targetY + shift.y,
    targetZ: camera.targetZ + shift.z,
  };
}

/**
 * Dolly toward/away from the point under the cursor so zooming feels
 * anchored to what the operator is pointing at.
 */
export function dollyCamera(
  camera: GraphCameraState,
  factor: number,
  anchor?: { viewport: SceneViewport; screenX: number; screenY: number },
): GraphCameraState {
  const distance = clamp(camera.distance * factor, minCameraDistance, maxCameraDistance);
  const applied = distance / camera.distance;
  let { targetX, targetY, targetZ } = camera;
  if (anchor && applied !== 1) {
    const focus = unprojectToPlaneThroughPoint(
      camera,
      anchor.viewport,
      anchor.screenX,
      anchor.screenY,
      { x: camera.targetX, y: camera.targetY, z: camera.targetZ },
    );
    if (focus) {
      // Zooming in pulls the target toward the anchor point; zooming out
      // releases it back. `1 - applied` is the fraction of the remaining
      // gap the target travels.
      const blend = 1 - applied;
      targetX += (focus.x - targetX) * blend;
      targetY += (focus.y - targetY) * blend;
      targetZ += (focus.z - targetZ) * blend;
    }
  }
  return { ...camera, distance, targetX, targetY, targetZ };
}

export function orbitCamera(
  camera: GraphCameraState,
  deltaYaw: number,
  deltaPitch: number,
): GraphCameraState {
  return {
    ...camera,
    yaw: camera.yaw + deltaYaw,
    pitch: clamp(camera.pitch + deltaPitch, -maxCameraPitch, maxCameraPitch),
  };
}

export function frameWorldPoints(
  points: readonly WorldPoint[],
  viewport: SceneViewport,
  previous?: Pick<GraphCameraState, "yaw" | "pitch">,
  paddingFactor = 1.2,
): GraphCameraState {
  const yaw = previous?.yaw ?? 0;
  const pitch = previous?.pitch ?? 0;
  if (points.length === 0) {
    return { targetX: 0, targetY: 0, targetZ: 0, distance: 900, yaw, pitch };
  }
  // Extents are measured in the *active* camera basis (right/up/forward for
  // the preserved yaw/pitch), not along world axes: framing after an orbit
  // must still fit every requested point on screen.
  const basis = cameraBasis({ targetX: 0, targetY: 0, targetZ: 0, distance: 1, yaw, pitch });
  let minRight = Number.POSITIVE_INFINITY;
  let maxRight = Number.NEGATIVE_INFINITY;
  let minUp = Number.POSITIVE_INFINITY;
  let maxUp = Number.NEGATIVE_INFINITY;
  let minForward = Number.POSITIVE_INFINITY;
  let maxForward = Number.NEGATIVE_INFINITY;
  for (const point of points) {
    const alongRight = dot(point, basis.right);
    const alongUp = dot(point, basis.up);
    const alongForward = dot(point, basis.forward);
    minRight = Math.min(minRight, alongRight);
    maxRight = Math.max(maxRight, alongRight);
    minUp = Math.min(minUp, alongUp);
    maxUp = Math.max(maxUp, alongUp);
    minForward = Math.min(minForward, alongForward);
    maxForward = Math.max(maxForward, alongForward);
  }
  const centerRight = (minRight + maxRight) / 2;
  const centerUp = (minUp + maxUp) / 2;
  const centerForward = (minForward + maxForward) / 2;
  // The basis is orthonormal, so the view-space center maps back to world
  // space as a linear combination of the basis directions.
  const center: WorldPoint = {
    x: basis.right.x * centerRight + basis.up.x * centerUp + basis.forward.x * centerForward,
    y: basis.right.y * centerRight + basis.up.y * centerUp + basis.forward.y * centerForward,
    z: basis.right.z * centerRight + basis.up.z * centerUp + basis.forward.z * centerForward,
  };
  const halfWidth = Math.max(40, (maxRight - minRight) / 2);
  const halfHeight = Math.max(40, (maxUp - minUp) / 2);
  const halfDepth = (maxForward - minForward) / 2;
  // Box fit: distance where both the vertical and horizontal extents fill
  // the frustum, pushed back by the content's depth so nothing near-clips.
  const f = Math.tan((graphCameraFovDegrees * Math.PI) / 360);
  const aspect = viewport.width / Math.max(1, viewport.height);
  const fitHeight = halfHeight / f;
  const fitWidth = halfWidth / (f * aspect);
  return {
    targetX: center.x,
    targetY: center.y,
    targetZ: center.z,
    distance: clamp((Math.max(fitHeight, fitWidth) + halfDepth) * paddingFactor, minCameraDistance, maxCameraDistance),
    yaw,
    pitch,
  };
}

/** Exponential approach toward a goal camera; returns done when settled. */
export function advanceCameraToward(
  current: GraphCameraState,
  goal: GraphCameraState,
  deltaSeconds: number,
  reducedMotion = false,
): { camera: GraphCameraState; done: boolean } {
  if (reducedMotion) return { camera: goal, done: true };
  const rate = 1 - Math.exp(-9 * Math.max(0, deltaSeconds));
  const next: GraphCameraState = {
    targetX: lerp(current.targetX, goal.targetX, rate),
    targetY: lerp(current.targetY, goal.targetY, rate),
    targetZ: lerp(current.targetZ, goal.targetZ, rate),
    distance: lerp(current.distance, goal.distance, rate),
    yaw: lerp(current.yaw, goal.yaw, rate),
    pitch: lerp(current.pitch, goal.pitch, rate),
  };
  const settled = Math.abs(next.distance - goal.distance) < 0.5
    && Math.hypot(next.targetX - goal.targetX, next.targetY - goal.targetY, next.targetZ - goal.targetZ) < 0.5
    && Math.abs(next.yaw - goal.yaw) < 0.002
    && Math.abs(next.pitch - goal.pitch) < 0.002;
  return { camera: settled ? goal : next, done: settled };
}

// ─── World model ────────────────────────────────────────────────────────────

export interface WorldNode extends WorldPoint {
  nodeId: string;
  radius: number;
  label: string;
  kind: string;
  communityId?: string;
  degree: number;
}

/** Map a layout node into centered, y-up world space with community depth. */
export function worldNodesFor(
  layout: GraphLayoutResult,
  overrides: ReadonlyMap<string, { x: number; y: number; z?: number }>,
): WorldNode[] {
  const degrees = new Map<string, number>();
  for (const edge of layout.edges) {
    degrees.set(edge.sourceId, (degrees.get(edge.sourceId) ?? 0) + 1);
    degrees.set(edge.targetId, (degrees.get(edge.targetId) ?? 0) + 1);
  }
  return layout.nodes.map((node) => {
    const override = overrides.get(node.nodeId);
    const layoutX = override?.x ?? node.x;
    const layoutY = override?.y ?? node.y;
    return {
      nodeId: node.nodeId,
      x: layoutX - layout.width / 2,
      y: layout.height / 2 - layoutY,
      z: override?.z ?? communityDepth(node.communityId, node.nodeId),
      radius: node.radius,
      label: node.label,
      kind: node.kind,
      communityId: node.communityId,
      degree: degrees.get(node.nodeId) ?? 0,
    };
  });
}

/**
 * Deterministic z per community band plus per-node jitter: clusters live on
 * nearby depth planes so orbiting the camera separates them visibly.
 */
export function communityDepth(communityId: string | undefined, nodeId: string): number {
  const band = communityId ? ((stringHash(communityId) % 7) - 3) * 42 : 0;
  const jitter = ((stringHash(nodeId) % 1000) / 1000 - 0.5) * 30;
  return band + jitter;
}

export function worldToLayout(layout: GraphLayoutResult, point: WorldPoint): { x: number; y: number } {
  return { x: point.x + layout.width / 2, y: layout.height / 2 - point.y };
}

export function defaultCameraForLayout(
  layout: GraphLayoutResult,
  viewport: SceneViewport,
): GraphCameraState {
  const corners: WorldPoint[] = [
    { x: -layout.width / 2, y: -layout.height / 2, z: 0 },
    { x: layout.width / 2, y: layout.height / 2, z: 0 },
  ];
  return frameWorldPoints(corners, viewport, { yaw: 0, pitch: 0 }, 1.06);
}

// ─── Area hulls ─────────────────────────────────────────────────────────────

export interface AreaHullModel {
  areaId: string;
  label: string;
  memberNodeIds: string[];
  /** Smooth closed outline in world space. */
  outline: WorldPoint[];
  centroid: WorldPoint;
  color: string;
}

interface CommunityLike {
  id: string;
  label: string;
  node_ids: readonly string[];
}

const hullPadding = 42;

/**
 * Diffuse cluster hulls. Membership comes from snapshot communities, which
 * may list one node under several areas — a node in two areas simply feeds
 * both hulls, so overlapping membership renders as overlapping translucent
 * blobs rather than needing exclusive geometry.
 */
export function buildAreaHulls(
  worldNodes: readonly WorldNode[],
  communities: readonly CommunityLike[],
): AreaHullModel[] {
  const byId = new Map(worldNodes.map((node) => [node.nodeId, node]));
  const hulls: AreaHullModel[] = [];
  communities.forEach((community, index) => {
    const members = community.node_ids
      .map((nodeId) => byId.get(nodeId))
      .filter((node): node is WorldNode => Boolean(node));
    if (members.length === 0) return;
    // A node can belong to several areas but only sits in one place, so a
    // secondary area's hull would stretch across the canvas to reach it.
    // Trim spatial outliers: the hull hugs the area's resident cluster and
    // remote multi-area members surface through hover/tooltip instead.
    const core = trimSpatialOutliers(members);
    const centroid: WorldPoint = {
      x: core.reduce((sum, node) => sum + node.x, 0) / core.length,
      y: core.reduce((sum, node) => sum + node.y, 0) / core.length,
      z: core.reduce((sum, node) => sum + node.z, 0) / core.length,
    };
    const ring = core.length === 1
      ? circleOutline(core[0], hullPadding)
      : paddedHullOutline(core, hullPadding);
    hulls.push({
      areaId: community.id,
      label: community.label,
      memberNodeIds: members.map((node) => node.nodeId),
      outline: ring.map((point) => ({ ...point, z: centroid.z })),
      centroid,
      color: areaColor(community.id, index),
    });
  });
  return hulls;
}

function trimSpatialOutliers<T extends WorldPoint>(members: readonly T[]): T[] {
  if (members.length <= 3) return [...members];
  const center = {
    x: members.reduce((sum, point) => sum + point.x, 0) / members.length,
    y: members.reduce((sum, point) => sum + point.y, 0) / members.length,
  };
  const distances = members
    .map((point) => Math.hypot(point.x - center.x, point.y - center.y))
    .sort((a, b) => a - b);
  const median = distances[Math.floor(distances.length / 2)];
  const cutoff = Math.max(120, median * 2);
  const core = members.filter((point) => Math.hypot(point.x - center.x, point.y - center.y) <= cutoff);
  return core.length >= 2 ? core : [...members];
}

function circleOutline(center: WorldPoint, radius: number): Array<{ x: number; y: number }> {
  const points: Array<{ x: number; y: number }> = [];
  for (let step = 0; step < 18; step += 1) {
    const angle = (step / 18) * Math.PI * 2;
    points.push({ x: center.x + Math.cos(angle) * radius, y: center.y + Math.sin(angle) * radius });
  }
  return points;
}

function paddedHullOutline(
  members: readonly WorldPoint[],
  padding: number,
): Array<{ x: number; y: number }> {
  const hull = convexHull(members.map((point) => ({ x: point.x, y: point.y })));
  if (hull.length < 3) {
    // Two members or collinear members: build a stadium around ALL points
    // (hull of padded circles) so every member stays inside the blob.
    const samples = members.flatMap((point) => circleOutline({ x: point.x, y: point.y, z: 0 }, padding));
    return chaikinSmooth(convexHull(samples));
  }
  const centroid = {
    x: hull.reduce((sum, point) => sum + point.x, 0) / hull.length,
    y: hull.reduce((sum, point) => sum + point.y, 0) / hull.length,
  };
  const padded = hull.map((point) => {
    const dx = point.x - centroid.x;
    const dy = point.y - centroid.y;
    const length = Math.max(1e-3, Math.hypot(dx, dy));
    return {
      x: point.x + (dx / length) * padding,
      y: point.y + (dy / length) * padding,
    };
  });
  return chaikinSmooth(chaikinSmooth(padded));
}

/** Andrew's monotone chain; returns counter-clockwise hull. */
export function convexHull(points: readonly { x: number; y: number }[]): Array<{ x: number; y: number }> {
  const sorted = [...points].sort((a, b) => a.x - b.x || a.y - b.y);
  if (sorted.length <= 2) return sorted;
  const crossProduct = (o: { x: number; y: number }, a: { x: number; y: number }, b: { x: number; y: number }) =>
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
  const lower: Array<{ x: number; y: number }> = [];
  for (const point of sorted) {
    while (lower.length >= 2 && crossProduct(lower[lower.length - 2], lower[lower.length - 1], point) <= 0) {
      lower.pop();
    }
    lower.push(point);
  }
  const upper: Array<{ x: number; y: number }> = [];
  for (const point of [...sorted].reverse()) {
    while (upper.length >= 2 && crossProduct(upper[upper.length - 2], upper[upper.length - 1], point) <= 0) {
      upper.pop();
    }
    upper.push(point);
  }
  lower.pop();
  upper.pop();
  return [...lower, ...upper];
}

/** One round of Chaikin corner cutting over a closed polygon. */
function chaikinSmooth(points: readonly { x: number; y: number }[]): Array<{ x: number; y: number }> {
  if (points.length < 3) return [...points];
  const smoothed: Array<{ x: number; y: number }> = [];
  for (let index = 0; index < points.length; index += 1) {
    const current = points[index];
    const next = points[(index + 1) % points.length];
    smoothed.push(
      { x: current.x * 0.75 + next.x * 0.25, y: current.y * 0.75 + next.y * 0.25 },
      { x: current.x * 0.25 + next.x * 0.75, y: current.y * 0.25 + next.y * 0.75 },
    );
  }
  return smoothed;
}

const areaPalette = [
  "#4c7fb0",
  "#7c66b8",
  "#2f8f83",
  "#b0762f",
  "#a05577",
  "#5f8f3d",
  "#3f7fa8",
  "#8a6b4f",
];

export function areaColor(areaId: string, index: number): string {
  return areaPalette[(stringHash(areaId) + index) % areaPalette.length];
}

// ─── Scene assembly ─────────────────────────────────────────────────────────

export interface SceneNode {
  nodeId: string;
  x: number;
  y: number;
  screenRadius: number;
  depth: number;
  color: string;
  alpha: number;
  labelText: string;
  labelAlpha: number;
  emphasis: "hovered" | "selected" | "neighbor" | "none" | "dimmed";
  world: WorldPoint;
}

export interface SceneEdge {
  edgeId: string;
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  alpha: number;
  emphasized: boolean;
  depth: number;
}

export interface SceneHull {
  areaId: string;
  label: string;
  color: string;
  alpha: number;
  labelAlpha: number;
  outline: Array<{ x: number; y: number }>;
  labelX: number;
  labelY: number;
  depth: number;
}

export interface GraphScene {
  nodes: SceneNode[];
  edges: SceneEdge[];
  hulls: SceneHull[];
  /** 0..1 — how "zoomed in" the camera is relative to the framed layout. */
  zoomLevel: number;
  hoveredNodeId: string | null;
}

export interface SceneBuildInput {
  layout: GraphLayoutResult;
  communities: readonly CommunityLike[];
  camera: GraphCameraState;
  viewport: SceneViewport;
  overrides: ReadonlyMap<string, { x: number; y: number; z?: number }>;
  selectedNodeIds: readonly string[];
  hoveredNodeId: string | null;
  maxLabels?: number;
}

export function buildGraphScene(input: SceneBuildInput): GraphScene {
  const basis = cameraBasis(input.camera);
  const worldNodes = worldNodesFor(input.layout, input.overrides);
  const hullModels = buildAreaHulls(worldNodes, input.communities);
  const framedDistance = defaultCameraForLayout(input.layout, input.viewport).distance;
  const zoomLevel = clamp(framedDistance / Math.max(1, input.camera.distance), 0.2, 8);

  const selected = new Set(input.selectedNodeIds);
  const neighborIds = new Set<string>();
  if (input.hoveredNodeId) {
    for (const edge of input.layout.edges) {
      if (edge.sourceId === input.hoveredNodeId) neighborIds.add(edge.targetId);
      if (edge.targetId === input.hoveredNodeId) neighborIds.add(edge.sourceId);
    }
  }

  const projectedById = new Map<string, { node: WorldNode; point: ProjectedPoint }>();
  for (const node of worldNodes) {
    projectedById.set(node.nodeId, {
      node,
      point: projectWorldPoint(input.camera, input.viewport, node, basis),
    });
  }

  const nodeLabelBudget = labelBudget(worldNodes, selected, input.hoveredNodeId, input.maxLabels ?? 80);
  // At the framed default (zoomLevel 1) only area titles show; node labels
  // fade in once the operator dollies past ~1.2x and take over by ~2x.
  const nodeLabelFade = smoothstep(1.18, 2.1, zoomLevel);
  const areaLabelFade = 1 - smoothstep(1.35, 2.2, zoomLevel);
  const hoverActive = Boolean(input.hoveredNodeId);

  const nodes: SceneNode[] = [];
  for (const { node, point } of projectedById.values()) {
    if (!point.visible) continue;
    const emphasis = node.nodeId === input.hoveredNodeId
      ? "hovered"
      : selected.has(node.nodeId)
        ? "selected"
        : neighborIds.has(node.nodeId)
          ? "neighbor"
          : hoverActive
            ? "dimmed"
            : "none";
    const fog = depthFog(point.depth, input.camera.distance);
    const alpha = emphasis === "dimmed" ? 0.18 : fog;
    // Degree feeds size so well-connected concepts read as landmarks at a
    // glance, mirroring Obsidian's graph view.
    const worldRadius = node.radius + Math.min(10, node.degree * 1.1);
    const screenRadius = Math.max(3, worldRadius * point.pixelsPerWorldUnit * 1.45);
    const labelEligible = nodeLabelBudget.has(node.nodeId) || emphasis === "hovered" || emphasis === "selected" || emphasis === "neighbor";
    const labelAlpha = emphasis === "hovered" || emphasis === "selected"
      ? 1
      : emphasis === "neighbor"
        ? Math.max(0.85, nodeLabelFade)
        : hoverActive
          ? 0
          : labelEligible
            ? nodeLabelFade * fog
            : 0;
    nodes.push({
      nodeId: node.nodeId,
      x: point.x,
      y: point.y,
      screenRadius,
      depth: point.depth,
      color: nodeColor(node.kind),
      alpha,
      labelText: node.label,
      labelAlpha,
      emphasis,
      world: { x: node.x, y: node.y, z: node.z },
    });
  }
  nodes.sort((a, b) => b.depth - a.depth);

  const edges: SceneEdge[] = [];
  for (const edge of input.layout.edges) {
    const source = projectedById.get(edge.sourceId);
    const target = projectedById.get(edge.targetId);
    if (!source?.point.visible || !target?.point.visible) continue;
    const emphasized = hoverActive
      && (edge.sourceId === input.hoveredNodeId || edge.targetId === input.hoveredNodeId);
    const depth = (source.point.depth + target.point.depth) / 2;
    const alpha = emphasized
      ? 0.92
      : hoverActive
        ? 0.05
        : 0.32 * depthFog(depth, input.camera.distance);
    edges.push({
      edgeId: edge.edgeId,
      x1: source.point.x,
      y1: source.point.y,
      x2: target.point.x,
      y2: target.point.y,
      alpha,
      emphasized,
      depth,
    });
  }
  edges.sort((a, b) => b.depth - a.depth);

  const hulls: SceneHull[] = hullModels.map((hull) => {
    const outline = hull.outline
      .map((point) => projectWorldPoint(input.camera, input.viewport, point, basis))
      .filter((point) => point.visible)
      .map((point) => ({ x: point.x, y: point.y }));
    const labelPoint = projectWorldPoint(input.camera, input.viewport, hull.centroid, basis);
    return {
      areaId: hull.areaId,
      label: hull.label,
      color: hull.color,
      alpha: hoverActive ? 0.04 : 0.07 + 0.07 * areaLabelFade,
      labelAlpha: hoverActive ? 0 : areaLabelFade,
      outline,
      labelX: labelPoint.x,
      labelY: labelPoint.y,
      depth: labelPoint.depth,
      };
  }).filter((hull) => hull.outline.length >= 3);
  hulls.sort((a, b) => b.depth - a.depth);

  return { nodes, edges, hulls, zoomLevel, hoveredNodeId: input.hoveredNodeId };
}

function labelBudget(
  worldNodes: readonly WorldNode[],
  selected: ReadonlySet<string>,
  hoveredNodeId: string | null,
  maxLabels: number,
): Set<string> {
  const prioritized = [...worldNodes].sort((a, b) =>
    priorityRank(b, selected, hoveredNodeId) - priorityRank(a, selected, hoveredNodeId)
    || b.degree - a.degree
    || a.nodeId.localeCompare(b.nodeId),
  );
  return new Set(prioritized.slice(0, maxLabels).map((node) => node.nodeId));
}

function priorityRank(
  node: WorldNode,
  selected: ReadonlySet<string>,
  hoveredNodeId: string | null,
): number {
  if (node.nodeId === hoveredNodeId) return 3;
  if (selected.has(node.nodeId)) return 2;
  if (node.kind === "bundle" || node.kind === "community") return 1;
  return 0;
}

function depthFog(depth: number, cameraDistance: number): number {
  const near = cameraDistance * 0.55;
  const far = cameraDistance * 1.9;
  return 1 - 0.5 * smoothstep(near, far, depth);
}

export function hitTestScene(scene: GraphScene, x: number, y: number): string | null {
  let best: { nodeId: string; distance: number; depth: number } | null = null;
  for (const node of scene.nodes) {
    const distance = Math.hypot(node.x - x, node.y - y);
    const hitRadius = Math.max(9, node.screenRadius + 5);
    if (distance > hitRadius) continue;
    if (!best || distance < best.distance - 2 || (Math.abs(distance - best.distance) <= 2 && node.depth < best.depth)) {
      best = { nodeId: node.nodeId, distance, depth: node.depth };
    }
  }
  return best?.nodeId ?? null;
}

export function hitTestHull(scene: GraphScene, x: number, y: number): SceneHull | null {
  // Front-most hull whose polygon contains the point.
  for (let index = scene.hulls.length - 1; index >= 0; index -= 1) {
    const hull = scene.hulls[index];
    if (pointInPolygon(hull.outline, x, y)) return hull;
  }
  return null;
}

function pointInPolygon(polygon: readonly { x: number; y: number }[], x: number, y: number): boolean {
  let inside = false;
  for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i, i += 1) {
    const a = polygon[i];
    const b = polygon[j];
    if ((a.y > y) !== (b.y > y) && x < ((b.x - a.x) * (y - a.y)) / (b.y - a.y) + a.x) {
      inside = !inside;
    }
  }
  return inside;
}

export function nodeColor(kind: string): string {
  switch (kind) {
    case "concept":
      return "#2c7da0";
    case "bundle":
      return "#334155";
    case "tag":
      return "#7c3aed";
    case "source_ref":
      return "#0f766e";
    case "community":
      return "#475569";
    default:
      return "#64748b";
  }
}

export const nodeEmphasisColor = "#c2410c";

// ─── Small vector helpers ───────────────────────────────────────────────────

function dot(a: WorldPoint, b: WorldPoint): number {
  return a.x * b.x + a.y * b.y + a.z * b.z;
}

function cross(a: WorldPoint, b: WorldPoint): WorldPoint {
  return {
    x: a.y * b.z - a.z * b.y,
    y: a.z * b.x - a.x * b.z,
    z: a.x * b.y - a.y * b.x,
  };
}

function add(a: WorldPoint, b: WorldPoint): WorldPoint {
  return { x: a.x + b.x, y: a.y + b.y, z: a.z + b.z };
}

function scale(a: WorldPoint, factor: number): WorldPoint {
  return { x: a.x * factor, y: a.y * factor, z: a.z * factor };
}

function normalize(a: WorldPoint): WorldPoint {
  const length = Math.hypot(a.x, a.y, a.z) || 1;
  return { x: a.x / length, y: a.y / length, z: a.z / length };
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function smoothstep(edge0: number, edge1: number, value: number): number {
  const t = clamp((value - edge0) / (edge1 - edge0), 0, 1);
  return t * t * (3 - 2 * t);
}

function stringHash(value: string): number {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return Math.abs(hash >>> 0);
}
