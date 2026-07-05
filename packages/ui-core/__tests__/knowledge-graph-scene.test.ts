/**
 * @jest-environment jsdom
 *
 * Scene-model tests for the knowledge-graph command center: the software
 * projector must match THREE's perspective camera, hulls must hug their
 * clusters (including multi-area membership), and label visibility must
 * follow the zoom level.
 */

import * as THREE from "three";
import {
  advanceCameraToward,
  buildAreaHulls,
  buildGraphScene,
  cameraBasis,
  convexHull,
  defaultCameraForLayout,
  dollyCamera,
  frameWorldPoints,
  graphCameraFovDegrees,
  hitTestScene,
  orbitCamera,
  panCamera,
  projectWorldPoint,
  unprojectToPlaneThroughPoint,
  worldNodesFor,
  type GraphCameraState,
} from "../src/knowledge-graph-scene.js";
import { computeGraphLayout, graphVizFixtureSnapshot } from "@opensymphony/graph";

const viewport = { width: 1200, height: 700 };

function testCamera(overrides: Partial<GraphCameraState> = {}): GraphCameraState {
  return {
    targetX: 0,
    targetY: 0,
    targetZ: 0,
    distance: 900,
    yaw: 0.35,
    pitch: 0.2,
    ...overrides,
  };
}

describe("software projector parity with three.js", () => {
  it("matches THREE.PerspectiveCamera projection for sample points", () => {
    const camera = testCamera();
    const basis = cameraBasis(camera);
    const three = new THREE.PerspectiveCamera(
      graphCameraFovDegrees,
      viewport.width / viewport.height,
      6,
      12_000,
    );
    three.position.set(basis.position.x, basis.position.y, basis.position.z);
    three.up.set(0, 1, 0);
    three.lookAt(camera.targetX, camera.targetY, camera.targetZ);
    three.updateMatrixWorld();

    const samples = [
      { x: 0, y: 0, z: 0 },
      { x: 320, y: -140, z: 60 },
      { x: -510, y: 220, z: -90 },
      { x: 44, y: 380, z: 130 },
    ];
    for (const sample of samples) {
      const mine = projectWorldPoint(camera, viewport, sample);
      const vector = new THREE.Vector3(sample.x, sample.y, sample.z).project(three);
      const expectedX = ((vector.x + 1) / 2) * viewport.width;
      const expectedY = ((1 - vector.y) / 2) * viewport.height;
      expect(mine.visible).toBe(true);
      expect(Math.abs(mine.x - expectedX)).toBeLessThan(0.75);
      expect(Math.abs(mine.y - expectedY)).toBeLessThan(0.75);
    }
  });

  it("marks points behind the camera as not visible", () => {
    const camera = testCamera({ yaw: 0, pitch: 0 });
    const behind = projectWorldPoint(camera, viewport, { x: 0, y: 0, z: 5_000 });
    expect(behind.visible).toBe(false);
  });
});

describe("camera operations", () => {
  it("pans the target in the view plane", () => {
    const camera = testCamera({ yaw: 0, pitch: 0 });
    const panned = panCamera(camera, viewport, 120, -40);
    expect(panned.targetX).not.toBe(camera.targetX);
    expect(panned.targetY).not.toBe(camera.targetY);
    expect(panned.distance).toBe(camera.distance);
  });

  it("dollies toward the cursor anchor when zooming in", () => {
    const camera = testCamera({ yaw: 0, pitch: 0 });
    const zoomed = dollyCamera(camera, 0.5, { viewport, screenX: viewport.width * 0.85, screenY: viewport.height * 0.25 });
    expect(zoomed.distance).toBeCloseTo(camera.distance * 0.5, 5);
    // The target moves toward the anchored point (right of and above center).
    expect(zoomed.targetX).toBeGreaterThan(camera.targetX);
    expect(zoomed.targetY).toBeGreaterThan(camera.targetY);
  });

  it("clamps orbit pitch", () => {
    const camera = testCamera();
    const orbited = orbitCamera(camera, 0.4, 10);
    expect(orbited.yaw).toBeCloseTo(camera.yaw + 0.4, 5);
    expect(orbited.pitch).toBeLessThanOrEqual(1.15);
  });

  it("frames points so they fit the viewport", () => {
    const points = [
      { x: -400, y: -200, z: 0 },
      { x: 400, y: 200, z: 0 },
    ];
    const framed = frameWorldPoints(points, viewport, { yaw: 0, pitch: 0 }, 1.1);
    for (const point of points) {
      const projected = projectWorldPoint(framed, viewport, point);
      expect(projected.visible).toBe(true);
      expect(projected.x).toBeGreaterThanOrEqual(0);
      expect(projected.x).toBeLessThanOrEqual(viewport.width);
      expect(projected.y).toBeGreaterThanOrEqual(0);
      expect(projected.y).toBeLessThanOrEqual(viewport.height);
    }
  });

  it("unprojects a node's screen position back onto its drag plane", () => {
    const camera = testCamera();
    const world = { x: 150, y: -80, z: 40 };
    const projected = projectWorldPoint(camera, viewport, world);
    const roundTripped = unprojectToPlaneThroughPoint(camera, viewport, projected.x, projected.y, world);
    expect(roundTripped).not.toBeNull();
    expect(Math.abs(roundTripped!.x - world.x)).toBeLessThan(0.5);
    expect(Math.abs(roundTripped!.y - world.y)).toBeLessThan(0.5);
    expect(Math.abs(roundTripped!.z - world.z)).toBeLessThan(0.5);
  });

  it("eases toward a goal and settles", () => {
    let camera = testCamera({ distance: 900 });
    const goal = testCamera({ distance: 300, targetX: 200 });
    let done = false;
    for (let step = 0; step < 200 && !done; step += 1) {
      const advanced = advanceCameraToward(camera, goal, 1 / 60);
      camera = advanced.camera;
      done = advanced.done;
    }
    expect(done).toBe(true);
    expect(camera).toEqual(goal);
  });

  it("snaps immediately under reduced motion", () => {
    const goal = testCamera({ distance: 250 });
    const advanced = advanceCameraToward(testCamera(), goal, 1 / 60, true);
    expect(advanced.done).toBe(true);
    expect(advanced.camera).toEqual(goal);
  });
});

describe("area hulls", () => {
  it("computes convex hulls containing all inputs", () => {
    const points = [
      { x: 0, y: 0 },
      { x: 10, y: 0 },
      { x: 10, y: 10 },
      { x: 0, y: 10 },
      { x: 5, y: 5 },
    ];
    const hull = convexHull(points);
    expect(hull).toHaveLength(4);
    expect(hull).not.toContainEqual({ x: 5, y: 5 });
  });

  it("hugs the resident cluster and ignores remote multi-area members", () => {
    const cluster = Array.from({ length: 8 }, (_, index) => ({
      nodeId: `core-${index}`,
      x: (index % 3) * 40,
      y: Math.floor(index / 3) * 40,
      z: 0,
      radius: 9,
      label: `core ${index}`,
      kind: "concept",
      degree: 2,
    }));
    const remote = {
      nodeId: "remote",
      x: 4_000,
      y: 4_000,
      z: 0,
      radius: 9,
      label: "remote multi-area member",
      kind: "concept",
      degree: 1,
    };
    const hulls = buildAreaHulls(
      [...cluster, remote],
      [{ id: "area:x", label: "X", node_ids: [...cluster.map((node) => node.nodeId), "remote"] }],
    );
    expect(hulls).toHaveLength(1);
    const xs = hulls[0].outline.map((point) => point.x);
    // The outline stays near the cluster; the remote member cannot stretch
    // the hull across the canvas.
    expect(Math.max(...xs)).toBeLessThan(1_000);
    expect(hulls[0].memberNodeIds).toContain("remote");
  });

  it("feeds one node into every area hull it belongs to", () => {
    const shared = { nodeId: "shared", x: 0, y: 0, z: 0, radius: 9, label: "s", kind: "concept", degree: 1 };
    const hulls = buildAreaHulls(
      [shared],
      [
        { id: "area:a", label: "A", node_ids: ["shared"] },
        { id: "area:b", label: "B", node_ids: ["shared"] },
      ],
    );
    expect(hulls).toHaveLength(2);
    expect(hulls[0].memberNodeIds).toEqual(["shared"]);
    expect(hulls[1].memberNodeIds).toEqual(["shared"]);
  });
});

describe("scene assembly on the viz fixture", () => {
  const layout = computeGraphLayout(graphVizFixtureSnapshot, { kind: "force", width: 1400, height: 900 });

  function buildScene(overrides: Partial<Parameters<typeof buildGraphScene>[0]> = {}) {
    return buildGraphScene({
      layout,
      communities: graphVizFixtureSnapshot.communities,
      camera: defaultCameraForLayout(layout, viewport),
      viewport,
      overrides: new Map(),
      selectedNodeIds: [],
      hoveredNodeId: null,
      ...overrides,
    });
  }

  it("shows area labels and hides node labels at the framed overview", () => {
    const scene = buildScene();
    expect(scene.hulls.length).toBe(graphVizFixtureSnapshot.communities.length);
    expect(scene.hulls.every((hull) => hull.labelAlpha > 0.5)).toBe(true);
    expect(scene.nodes.every((node) => node.labelAlpha < 0.05)).toBe(true);
  });

  it("fades node labels in and area labels out when zoomed in", () => {
    const overview = defaultCameraForLayout(layout, viewport);
    const scene = buildScene({ camera: { ...overview, distance: overview.distance / 3 } });
    expect(scene.nodes.some((node) => node.labelAlpha > 0.5)).toBe(true);
    expect(scene.hulls.every((hull) => hull.labelAlpha < 0.05)).toBe(true);
  });

  it("dims non-neighbors and emphasizes connected edges on hover", () => {
    const edge = layout.edges[0];
    const scene = buildScene({ hoveredNodeId: edge.sourceId });
    const hovered = scene.nodes.find((node) => node.nodeId === edge.sourceId);
    const neighbor = scene.nodes.find((node) => node.nodeId === edge.targetId);
    expect(hovered?.emphasis).toBe("hovered");
    expect(neighbor?.emphasis).toBe("neighbor");
    expect(scene.nodes.some((node) => node.emphasis === "dimmed")).toBe(true);
    const emphasized = scene.edges.filter((candidate) => candidate.emphasized);
    expect(emphasized.length).toBeGreaterThan(0);
    expect(scene.edges.filter((candidate) => !candidate.emphasized).every((candidate) => candidate.alpha < 0.1)).toBe(true);
  });

  it("hit-tests the node under a screen point", () => {
    const scene = buildScene();
    const probe = scene.nodes[Math.floor(scene.nodes.length / 2)];
    expect(hitTestScene(scene, probe.x, probe.y)).toBe(probe.nodeId);
    expect(hitTestScene(scene, -50, -50)).toBeNull();
  });

  it("applies drag overrides to world positions", () => {
    const nodeId = layout.nodes[0].nodeId;
    const overrides = new Map([[nodeId, { x: 77, y: 88 }]]);
    const moved = worldNodesFor(layout, overrides).find((node) => node.nodeId === nodeId)!;
    expect(moved.x).toBeCloseTo(77 - layout.width / 2, 5);
    expect(moved.y).toBeCloseTo(layout.height / 2 - 88, 5);
  });
});

describe("graph viz fixture", () => {
  it("is deterministic and dense enough to exercise the visualization", () => {
    expect(graphVizFixtureSnapshot.nodes.length).toBeGreaterThan(80);
    expect(graphVizFixtureSnapshot.edges.length).toBeGreaterThan(100);
    expect(graphVizFixtureSnapshot.communities.length).toBe(7);
    const multiArea = graphVizFixtureSnapshot.communities.flatMap((community) => community.node_ids)
      .reduce((counts, nodeId) => counts.set(nodeId, (counts.get(nodeId) ?? 0) + 1), new Map<string, number>());
    const inTwoAreas = [...multiArea.values()].filter((count) => count > 1).length;
    expect(inTwoAreas).toBeGreaterThan(5);
    // Determinism: rebuilding module state elsewhere must not shift ids.
    expect(graphVizFixtureSnapshot.nodes[0].id).toBe("bundle:viz-workbench");
  });
});
