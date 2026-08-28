// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";
import * as THREE from "three";

const held = vi.hoisted(() => ({
  renderersCreated: 0,
  renderersDisposed: 0,
  rendererContextsLost: 0,
  controlsCreated: 0,
  controlsDisposed: 0,
  controlAdds: [] as Array<[string, EventListener]>,
  controlRemoves: [] as Array<[string, EventListener]>,
  ownerPassCreated: 0,
  ownerPassDisposed: 0,
  highlightCreated: 0,
  highlightDisposed: 0,
  supplementalCreated: 0,
  supplementalDisposed: 0,
  previewMaterialsCreated: 0,
  previewMaterialsDisposed: 0,
  geometriesCreated: 0,
  geometriesDisposed: 0,
}));

vi.mock("three", async (importOriginal) => {
  const actual = await importOriginal<typeof import("three")>();

  class CountingBufferGeometry extends actual.BufferGeometry {
    constructor() {
      super();
      held.geometriesCreated += 1;
    }

    override dispose(): void {
      held.geometriesDisposed += 1;
      super.dispose();
    }
  }

  class FakeWebGLRenderer {
    constructor() {
      held.renderersCreated += 1;
    }

    setPixelRatio() {}
    setSize() {}
    dispose() {
      held.renderersDisposed += 1;
    }

    forceContextLoss() {
      held.rendererContextsLost += 1;
    }
  }

  return {
    ...actual,
    BufferGeometry: CountingBufferGeometry,
    WebGLRenderer: FakeWebGLRenderer,
  };
});

vi.mock("three/examples/jsm/controls/OrbitControls.js", async () => {
  const THREE = await import("three");
  class FakeOrbitControls {
    readonly target = new THREE.Vector3();
    readonly mouseButtons = {
      LEFT: THREE.MOUSE.ROTATE as THREE.MOUSE | null,
      MIDDLE: THREE.MOUSE.DOLLY as THREE.MOUSE | null,
      RIGHT: THREE.MOUSE.PAN as THREE.MOUSE | null,
    };
    readonly touches = { ONE: THREE.TOUCH.ROTATE };
    enabled = true;
    enableDamping = false;
    enableRotate = false;

    constructor() {
      held.controlsCreated += 1;
    }

    addEventListener(type: string, listener: EventListener) {
      held.controlAdds.push([type, listener]);
    }

    removeEventListener(type: string, listener: EventListener) {
      held.controlRemoves.push([type, listener]);
    }

    update() {}

    dispose() {
      held.controlsDisposed += 1;
    }
  }
  return { OrbitControls: FakeOrbitControls };
});

vi.mock("./surfaceOwnerShader", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./surfaceOwnerShader")>();
  const THREE = await import("three");
  return {
    ...actual,
    createSurfaceOwnerPassResources: () => {
      held.ownerPassCreated += 1;
      return {
        depthTarget: {
          depthTexture: new THREE.DepthTexture(1, 1),
          width: 1,
          height: 1,
        },
        colorTarget: { width: 1, height: 1 },
        depthMaterial: new THREE.MeshBasicMaterial(),
        colorMaterial: new THREE.MeshBasicMaterial(),
      };
    },
    disposeSurfaceOwnerPassResources: () => {
      held.ownerPassDisposed += 1;
    },
    resizeSurfaceOwnerPassResources: vi.fn(),
  };
});

vi.mock("./sceneLayers", async () => {
  const THREE = await import("three");
  const disposeDrawable = (child: THREE.Object3D) => {
    if (!(child instanceof THREE.Mesh || child instanceof THREE.LineSegments)) return;
    child.geometry.dispose();
    const materials = Array.isArray(child.material) ? child.material : [child.material];
    for (const material of materials) material.dispose();
  };
  return {
    clearGroup: (group: THREE.Group) => {
      for (const child of [...group.children]) {
        group.remove(child);
        disposeDrawable(child);
      }
    },
    disposeDrawable,
    createHighlightLayer: () => {
      held.highlightCreated += 1;
      return {
        group: new THREE.Group(),
        setSegments: vi.fn(),
        setOwnerCodes: vi.fn(),
        dispose: () => {
          held.highlightDisposed += 1;
        },
      };
    },
    createSupplementalEdgeLayer: () => {
      held.supplementalCreated += 1;
      return {
        group: new THREE.Group(),
        setSegments: vi.fn(),
        clear: vi.fn(),
        dispose: () => {
          held.supplementalDisposed += 1;
        },
      };
    },
    createPreviewMaterial: () => {
      held.previewMaterialsCreated += 1;
      const material = new THREE.MeshBasicMaterial();
      const dispose = material.dispose.bind(material);
      material.dispose = () => {
        held.previewMaterialsDisposed += 1;
        dispose();
      };
      return material;
    },
  };
});

import {
  captureViewer3DReadback,
  createPackedDepthReadbackResources,
  createScene,
  disposePackedDepthReadbackResources,
} from "./sceneFacade";
import {
  buildTopology,
  createContent,
  createSoftContent,
  type SoftContent,
  type Viewer3DContent,
} from "./sceneContent";
import type { Document, Face, SoftMesh } from "../../lib/types";

function pointerEvent(
  type: string,
  values: Partial<PointerEvent> & { pointerId: number },
): Event {
  const event = new Event(type, { bubbles: true, cancelable: true });
  for (const [key, value] of Object.entries(values)) {
    Object.defineProperty(event, key, { configurable: true, value });
  }
  return event;
}

function canvasForScene(): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  Object.defineProperty(canvas, "clientHeight", { configurable: true, value: 400 });
  return canvas;
}

function documentFixture(): Document {
  return {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: {
      vertices: [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [1, 0] },
        { id: 2, pos: [1, 1] },
        { id: 3, pos: [0, 1] },
      ],
      edges: [
        { id: 0, v0: 0, v1: 1, kind: "Border" },
        { id: 1, v0: 1, v1: 2, kind: "Border" },
        { id: 2, v0: 2, v1: 3, kind: "Border" },
        { id: 3, v0: 3, v1: 0, kind: "Border" },
        { id: 5, v0: 0, v1: 2, kind: "Mountain" },
      ],
      next_vertex_id: 4,
      next_edge_id: 6,
    },
    sequence: [],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
}

const FACES: Face[] = [
  { id: 0, vertices: [0, 1, 2], edges: [0, 1, 5] },
  { id: 1, vertices: [0, 2, 3], edges: [5, 2, 3] },
];

const SOFT: SoftMesh = {
  positions: [
    [0, 0, 0],
    [1, 0, 0],
    [1, 1, 0],
    [0, 1, 0],
  ],
  triangles: [
    [0, 1, 2],
    [0, 2, 3],
  ],
  triangle_faces: [0, 1],
  triangle_layers: [0, 1],
  warnings: [],
};

function sixResources(content: Viewer3DContent | SoftContent): THREE.Material[] | THREE.BufferGeometry[] {
  const meshMaterials = Array.isArray(content.mesh.material)
    ? content.mesh.material
    : [content.mesh.material];
  return [
    content.mesh.geometry,
    content.owner.geometry,
    content.line.geometry,
    ...meshMaterials,
    content.line.material as THREE.Material,
  ] as THREE.Material[] | THREE.BufferGeometry[];
}

function disposalSpies(content: Viewer3DContent | SoftContent) {
  const resources = [...sixResources(content)] as Array<THREE.Material | THREE.BufferGeometry>;
  expect(new Set(resources)).toHaveLength(6);
  return resources.map((resource) => vi.spyOn(resource, "dispose"));
}

beforeEach(() => {
  for (const key of [
    "renderersCreated",
    "renderersDisposed",
    "rendererContextsLost",
    "controlsCreated",
    "controlsDisposed",
    "ownerPassCreated",
    "ownerPassDisposed",
    "highlightCreated",
    "highlightDisposed",
    "supplementalCreated",
    "supplementalDisposed",
    "previewMaterialsCreated",
    "previewMaterialsDisposed",
    "geometriesCreated",
    "geometriesDisposed",
  ] as const) {
    held[key] = 0;
  }
  held.controlAdds.length = 0;
  held.controlRemoves.length = 0;
});

describe("scene facade の資源所有", () => {
  it("現在のsceneだけを撮影bridgeへ登録し、古い終了で新しいsceneを消さない", () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 1));
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    const first = createScene(canvasForScene());
    const second = createScene(canvasForScene());

    first.dispose();
    expect(() => captureViewer3DReadback()).toThrow("読み取れる紙面がありません");

    second.dispose();
    expect(() => captureViewer3DReadback()).toThrow(
      "3D表示の描画資源がまだ用意されていません",
    );
  });

  it("100回の生成・preview更新・終了でも全資源のcreate/dispose数が一致する", () => {
    let nextFrame = 1;
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => nextFrame++));
    vi.stubGlobal("cancelAnimationFrame", vi.fn());

    for (let i = 0; i < 100; i += 1) {
      const scene = createScene(canvasForScene());
      scene.setPreview([[[0, 0], [1, 0], [0, 1]]], 0.001);
      scene.setPreview([], 0);
      scene.dispose();
      scene.dispose();
    }

    expect(held.geometriesCreated).toBe(400);
    expect(held.geometriesDisposed).toBe(held.geometriesCreated);
    expect(held.previewMaterialsCreated).toBe(100);
    expect(held.previewMaterialsDisposed).toBe(held.previewMaterialsCreated);
    expect(held.renderersCreated).toBe(100);
    expect(held.renderersDisposed).toBe(held.renderersCreated);
    expect(held.rendererContextsLost).toBe(held.renderersCreated);
    expect(held.controlsCreated).toBe(100);
    expect(held.controlsDisposed).toBe(held.controlsCreated);
    expect(held.ownerPassCreated).toBe(100);
    expect(held.ownerPassDisposed).toBe(held.ownerPassCreated);
    expect(held.highlightCreated).toBe(100);
    expect(held.highlightDisposed).toBe(held.highlightCreated);
    expect(held.supplementalCreated).toBe(100);
    expect(held.supplementalDisposed).toBe(held.supplementalCreated);
    expect(held.controlAdds).toHaveLength(100);
    expect(held.controlRemoves).toEqual(held.controlAdds);
  });

  it("preview更新とscene終了でcreate/dispose数が一致し、二重終了でも増えない", () => {
    let nextFrame = 1;
    const requested: number[] = [];
    const cancelled: number[] = [];
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => {
      requested.push(nextFrame);
      return nextFrame++;
    }));
    vi.stubGlobal("cancelAnimationFrame", vi.fn((id: number) => cancelled.push(id)));

    const canvas = canvasForScene();
    const canvasAdd = vi.spyOn(canvas, "addEventListener");
    const canvasRemove = vi.spyOn(canvas, "removeEventListener");
    const documentAdd = vi.spyOn(document, "addEventListener");
    const documentRemove = vi.spyOn(document, "removeEventListener");
    const scene = createScene(canvas);

    scene.setPreview([[[0, 0], [1, 0], [0, 1]]], 0.001);
    scene.setPreview([], 0);
    scene.dispose();
    const afterFirstDispose = { ...held };
    scene.dispose();

    // facadeが直接作るBufferGeometryはempty owner 1、preview初期1、更新2。
    expect(held.geometriesCreated).toBe(4);
    expect(held.geometriesDisposed).toBe(held.geometriesCreated);
    expect(held.previewMaterialsCreated).toBe(1);
    expect(held.previewMaterialsDisposed).toBe(held.previewMaterialsCreated);
    expect(held.renderersCreated).toBe(1);
    expect(held.renderersDisposed).toBe(held.renderersCreated);
    expect(held.rendererContextsLost).toBe(held.renderersCreated);
    expect(held.controlsCreated).toBe(1);
    expect(held.controlsDisposed).toBe(held.controlsCreated);
    expect(held.ownerPassCreated).toBe(1);
    expect(held.ownerPassDisposed).toBe(held.ownerPassCreated);
    expect(held.highlightCreated).toBe(1);
    expect(held.highlightDisposed).toBe(held.highlightCreated);
    expect(held.supplementalCreated).toBe(1);
    expect(held.supplementalDisposed).toBe(held.supplementalCreated);
    expect(held).toEqual(afterFirstDispose);

    const canvasTypes = ["pointerdown", "wheel", "webglcontextrestored"];
    const documentTypes = ["pointermove", "pointerup", "pointercancel"];
    for (const type of canvasTypes) {
      const added = canvasAdd.mock.calls.find((call) => call[0] === type);
      const removed = canvasRemove.mock.calls.find((call) => call[0] === type);
      expect(removed).toEqual(added);
    }
    for (const type of documentTypes) {
      const added = documentAdd.mock.calls.find((call) => call[0] === type);
      const removed = documentRemove.mock.calls.find((call) => call[0] === type);
      expect(removed).toEqual(added);
    }
    expect(held.controlAdds).toHaveLength(1);
    expect(held.controlRemoves).toEqual(held.controlAdds);
    expect(requested).toEqual([1]);
    expect(cancelled).toEqual(requested);
  });

  it("packed-depth readbackの3資源を各1回だけ破棄する", () => {
    const resources = createPackedDepthReadbackResources(new THREE.DepthTexture(1, 1));
    const created = [resources.target, resources.material, resources.geometry];
    const disposes = created.map((resource) => vi.spyOn(resource, "dispose"));

    disposePackedDepthReadbackResources(resources);

    expect(created).toHaveLength(3);
    expect(disposes.filter((dispose) => dispose.mock.calls.length === 1)).toHaveLength(3);
  });

  it("rigid contentをsceneへ渡した後は置換・終了で6資源ずつ1回破棄する", () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 1));
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    const doc = documentFixture();
    const topology = buildTopology(doc, FACES, new Set([5]));
    const first = createContent(topology, doc.display);
    const second = createContent(topology, doc.display);
    const firstDisposes = disposalSpies(first);
    const secondDisposes = disposalSpies(second);
    const scene = createScene(canvasForScene());

    scene.setContent(first);
    scene.setContent(second);
    expect(firstDisposes.filter((dispose) => dispose.mock.calls.length === 1)).toHaveLength(6);
    expect(secondDisposes.every((dispose) => dispose.mock.calls.length === 0)).toBe(true);

    scene.dispose();
    expect(secondDisposes.filter((dispose) => dispose.mock.calls.length === 1)).toHaveLength(6);
  });

  it("soft contentをsceneへ渡した後は置換・終了で6資源ずつ1回破棄する", () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 1));
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    const display = documentFixture().display;
    const first = createSoftContent(SOFT, display);
    const second = createSoftContent(SOFT, display);
    const firstDisposes = disposalSpies(first);
    const secondDisposes = disposalSpies(second);
    const scene = createScene(canvasForScene());

    scene.setSoft(first);
    scene.setSoft(second);
    expect(firstDisposes.filter((dispose) => dispose.mock.calls.length === 1)).toHaveLength(6);
    expect(secondDisposes.every((dispose) => dispose.mock.calls.length === 0)).toBe(true);

    scene.dispose();
    expect(secondDisposes.filter((dispose) => dispose.mock.calls.length === 1)).toHaveLength(6);
  });
});

describe("scene facade のpointer契約", () => {
  it("同じpointerだけをdown→move→upで回し、終了後はdocument moveへ反応しない", () => {
    vi.stubGlobal("requestAnimationFrame", vi.fn(() => 1));
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    const canvas = canvasForScene();
    const scene = createScene(canvas);
    const before = scene.camera.position.clone();

    canvas.dispatchEvent(pointerEvent("pointerdown", {
      pointerId: 7,
      pointerType: "mouse",
      button: 0,
      clientX: 10,
      clientY: 20,
      ctrlKey: false,
      metaKey: false,
      shiftKey: false,
    }));
    document.dispatchEvent(pointerEvent("pointermove", {
      pointerId: 8,
      clientX: 60,
      clientY: 70,
    }));
    expect(scene.camera.position.toArray()).toEqual(before.toArray());

    document.dispatchEvent(pointerEvent("pointermove", {
      pointerId: 7,
      clientX: 60,
      clientY: 70,
    }));
    expect(scene.camera.position.toArray()).not.toEqual(before.toArray());
    const afterMove = scene.camera.position.clone();

    document.dispatchEvent(pointerEvent("pointerup", { pointerId: 7 }));
    document.dispatchEvent(pointerEvent("pointermove", {
      pointerId: 7,
      clientX: 90,
      clientY: 100,
    }));
    expect(scene.camera.position.toArray()).toEqual(afterMove.toArray());

    scene.dispose();
    document.dispatchEvent(pointerEvent("pointermove", {
      pointerId: 7,
      clientX: 120,
      clientY: 140,
    }));
    expect(scene.camera.position.toArray()).toEqual(afterMove.toArray());
  });
});
