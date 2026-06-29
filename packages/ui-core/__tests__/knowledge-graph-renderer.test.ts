/**
 * @jest-environment jsdom
 */

import { jest } from "@jest/globals";
import {
  computeGraphLayout,
  createInitialGraphState,
  fixtureGraphSnapshot,
  graphReducer,
} from "@opensymphony/graph";
import {
  mountKnowledgeGraphRenderer,
  renderKnowledgeGraphSurface,
} from "../src/knowledge-graph-renderer.js";

const mockRenderCalls: unknown[] = [];

jest.mock("three", () => {
  class FakeObject3D {
    children: FakeObject3D[] = [];
    position = { set: jest.fn() };
    scale = { set: jest.fn() };

    add(...objects: FakeObject3D[]): void {
      this.children.push(...objects);
    }

    clear(): void {
      this.children = [];
    }

    traverse(callback: (object: FakeObject3D) => void): void {
      callback(this);
      for (const child of this.children) child.traverse(callback);
    }
  }
  class FakeRenderer {
    setPixelRatio = jest.fn();
    setSize = jest.fn();
    setClearColor = jest.fn();
    clear = jest.fn();
    dispose = jest.fn();
    forceContextLoss = jest.fn();

    render(scene: unknown, camera: unknown): void {
      mockRenderCalls.push({ scene, camera });
    }
  }
  class FakeCamera extends FakeObject3D {
    left = 0;
    right = 0;
    top = 0;
    bottom = 0;
    updateProjectionMatrix = jest.fn();
  }
  class FakeGeometry {
    setAttribute = jest.fn();
    dispose = jest.fn();
  }
  class FakeMaterial {
    dispose = jest.fn();
  }
  class FakeInstancedMesh extends FakeObject3D {
    instanceMatrix = { needsUpdate: false };
    constructor(public geometry: FakeGeometry, public material: FakeMaterial) {
      super();
    }
    setMatrixAt = jest.fn();
  }
  class FakeLineSegments extends FakeObject3D {
    constructor(public geometry: FakeGeometry, public material: FakeMaterial) {
      super();
    }
  }
  return {
    WebGLRenderer: FakeRenderer,
    Scene: class FakeScene extends FakeObject3D {},
    Group: class FakeGroup extends FakeObject3D {},
    OrthographicCamera: FakeCamera,
    BufferGeometry: FakeGeometry,
    Float32BufferAttribute: class FakeFloat32BufferAttribute {
      constructor(public values: number[], public itemSize: number) {}
    },
    LineSegments: FakeLineSegments,
    LineBasicMaterial: FakeMaterial,
    CircleGeometry: class FakeCircleGeometry extends FakeGeometry {},
    MeshBasicMaterial: FakeMaterial,
    InstancedMesh: FakeInstancedMesh,
    Matrix4: class FakeMatrix4 {
      compose = jest.fn();
    },
    Vector3: class FakeVector3 {
      constructor(public x: number, public y: number, public z: number) {}
    },
    Quaternion: class FakeQuaternion {},
  };
});

describe("Knowledge Graph renderer", () => {
  it("uses the WebGL/Three path when a WebGL context is available", () => {
    const layout = computeGraphLayout(fixtureGraphSnapshot, { kind: "force", width: 640, height: 360 });
    const state = graphReducer(createInitialGraphState(), {
      type: "SNAPSHOT_LOADED",
      snapshot: fixtureGraphSnapshot,
    });
    document.body.innerHTML = renderKnowledgeGraphSurface({
      snapshot: fixtureGraphSnapshot,
      layout,
      state: graphReducer(state, { type: "LAYOUT_STATUS_SET", status: "ready" }),
    });
    const canvas = document.querySelector<HTMLCanvasElement>("[data-testid='knowledge-graph-canvas']");
    expect(canvas).toBeTruthy();
    jest.spyOn(canvas!, "getContext").mockImplementation((contextId: string) => (
      contextId.startsWith("webgl") ? {} as RenderingContext : null
    ));

    mountKnowledgeGraphRenderer(document.body, {
      snapshot: fixtureGraphSnapshot,
      layout,
      selectedNodeIds: [],
      view: { scale: 1, dx: 0, dy: 0 },
      onSelect: jest.fn(),
      onFocus: jest.fn(),
    });

    expect(mockRenderCalls).toHaveLength(1);
    expect(canvas!.dataset.nonblank).toBe("true");
  });
});
