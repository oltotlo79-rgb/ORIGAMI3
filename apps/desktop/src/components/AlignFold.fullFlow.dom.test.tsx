// @vitest-environment jsdom
// 「合わせて折る」8方式を、2D/3Dの実pointer選択から折り上がりまで通す画面検査。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type React from "react";
import { ALIGN_LABELS, type AlignMode, type AlignTarget } from "../lib/alignFold";
import type {
  Document,
  DocumentView,
  EdgeKind,
  Face,
  Frame3D,
  SeqOp,
  Vec2,
} from "../lib/types";

const held = vi.hoisted(() => ({
  cpDocument: null as unknown,
  scene: null as unknown as Record<string, unknown>,
}));

vi.mock("./CpEditor/renderer", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./CpEditor/renderer")>();
  return {
    ...actual,
    render: vi.fn((...args: unknown[]) => {
      held.cpDocument = args[4];
    }),
  };
});

vi.mock("./Viewer3D/sceneBuilder", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./Viewer3D/sceneBuilder")>();
  const THREE = await import("three");
  return {
    ...actual,
    createScene: () => {
      const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
      camera.position.set(0.5, 0.5, 2);
      camera.lookAt(0.5, 0.5, 0);
      camera.updateMatrixWorld(true);
      camera.updateProjectionMatrix();
      const scene = {
        camera,
        contentGroup: new THREE.Group(),
        highlightGroup: new THREE.Group(),
        content: null as unknown,
        soft: null as unknown,
        render: vi.fn(),
        syncTheme: vi.fn(),
        resize: vi.fn(),
        resetCamera: vi.fn(),
        setContent: vi.fn((content: unknown) => {
          scene.content = content;
        }),
        setHighlight: vi.fn(),
        setSupplementalEdges: vi.fn(),
        setPreview: vi.fn(),
        setSoft: vi.fn((content: unknown) => {
          scene.soft = content;
        }),
        setDrawMode: vi.fn(),
        dispose: vi.fn(),
      };
      held.scene = scene as unknown as Record<string, unknown>;
      return scene;
    },
  };
});

vi.mock("../ipc/client", () => ({
  documentNew: vi.fn(),
  documentOpen: vi.fn(),
  documentSave: vi.fn(),
  editApply: vi.fn(),
  editUndo: vi.fn(),
  editRedo: vi.fn(),
  sequenceApply: vi.fn(),
  sequenceReplay: vi.fn(),
  poseSolve: vi.fn(),
  recoveryCheck: vi.fn(),
  recoveryRestore: vi.fn(),
  proposalGenerate: vi.fn(),
}));

import * as ipc from "../ipc/client";
import { useAppStore, type AlignCpPick } from "../store/appStore";
import { ContextPanel } from "./ContextPanel";
import { CpEditor } from "./CpEditor/CpEditor";
import { Timeline } from "./Timeline";
import { Viewer3D } from "./Viewer3D/Viewer3D";

type FoldThroughOp = Extract<SeqOp, { type: "FoldThrough" }>;
type Direction = "Up" | "Down";
type Vec3 = [number, number, number];

interface AlignFlowCase {
  mode: AlignMode;
  label: string;
  kinds: AlignTarget["kind"][];
  expectedPicks2d: AlignTarget[];
  expectedPicks3d: AlignTarget[];
  expectedCpPicks3d: (AlignCpPick | null)[];
  clicks2d: Vec2[];
  clicks3d: Vec2[];
  direction2d: Direction;
  direction3d: Direction;
}

interface FoldFixture {
  doc: Document;
  faces: Face[];
  frame: Frame3D;
  creaseAlreadyPresent: boolean;
}

interface ViewerContentProbe {
  positions: Float32Array;
  topology: {
    slots: Map<number, { offset: number; count: number }>;
  };
}

const FIRST_FOLD_STEP = {
  id: 0,
  kind: "Simple" as const,
  drivers: [{ a: [0.5, 0] as Vec2, b: [0.5, 1] as Vec2, target_angle_deg: -180 }],
  layer_order: [[0.25, 0.5], [0.75, 0.5]] as Vec2[],
  note: "",
};

// x=.5で右半分を左へ折った、実backendと同じ2面・2層の開始状態。
// y=.5の2本はAuxなので選択には使えるが、面抽出には使われない。
const BASE_DOC: Document = {
  schema_version: 1,
  paper: { width_mm: 150, height_mm: 150 },
  cp: {
    vertices: [
      { id: 0, pos: [0, 0] },
      { id: 1, pos: [0.5, 0] },
      { id: 2, pos: [1, 0] },
      { id: 3, pos: [0, 0.5] },
      { id: 4, pos: [0.5, 0.5] },
      { id: 5, pos: [1, 0.5] },
      { id: 6, pos: [0, 1] },
      { id: 7, pos: [0.5, 1] },
      { id: 8, pos: [1, 1] },
    ],
    edges: [
      { id: 0, v0: 0, v1: 1, kind: "Border" },
      { id: 1, v0: 1, v1: 2, kind: "Border" },
      { id: 2, v0: 2, v1: 5, kind: "Border" },
      { id: 3, v0: 5, v1: 8, kind: "Border" },
      { id: 4, v0: 8, v1: 7, kind: "Border" },
      { id: 5, v0: 7, v1: 6, kind: "Border" },
      { id: 6, v0: 6, v1: 3, kind: "Border" },
      { id: 7, v0: 3, v1: 0, kind: "Border" },
      { id: 8, v0: 3, v1: 4, kind: "Aux" },
      { id: 9, v0: 4, v1: 5, kind: "Aux" },
      { id: 10, v0: 1, v1: 4, kind: "Valley" },
      { id: 11, v0: 4, v1: 7, kind: "Valley" },
    ],
    next_vertex_id: 9,
    next_edge_id: 12,
  },
  sequence: [FIRST_FOLD_STEP],
  display: {
    front_color: [237, 28, 36],
    back_color: [255, 255, 255],
    grid_divisions: 8,
  },
};

const BASE_FACES: Face[] = [
  { id: 0, vertices: [0, 1, 4, 7, 6, 3], edges: [0, 10, 11, 5, 6, 7] },
  { id: 1, vertices: [1, 2, 5, 8, 7, 4], edges: [1, 2, 3, 4, 11, 10] },
];

const BASE_FRAME: Frame3D = {
  faces: [
    {
      face: 0,
      polygon: [
        [0, 0, 0],
        [0.5, 0, 0],
        [0.5, 0.5, 0],
        [0.5, 1, 0],
        [0, 1, 0],
        [0, 0.5, 0],
      ],
      layer: 0,
    },
    {
      face: 1,
      polygon: [
        [0.5, 0, 0],
        [0, 0, 0],
        [0, 0.5, 0],
        [0, 1, 0],
        [0.5, 1, 0],
        [0.5, 0.5, 0],
      ],
      layer: 1,
      mirrored: true,
    },
  ],
  warnings: [],
};

const FINAL_FACES: Face[] = [
  { id: 0, vertices: [0, 1, 4, 3], edges: [0, 10, 8, 7] },
  { id: 1, vertices: [1, 2, 5, 4], edges: [1, 2, 9, 10] },
  { id: 2, vertices: [5, 8, 7, 4], edges: [3, 4, 11, 9] },
  { id: 3, vertices: [7, 6, 3, 4], edges: [5, 6, 8, 11] },
];

const FLOW_WARNING = "平坦条件を確認しながら、指定した折りは完了しました";

function creaseKind(direction: Direction): EdgeKind {
  return direction === "Up" ? "Valley" : "Mountain";
}

function oppositeKind(kind: EdgeKind): EdgeKind {
  return kind === "Valley" ? "Mountain" : "Valley";
}

function promoteAuxCrease(doc: Document, direction: Direction): Document {
  const direct = creaseKind(direction);
  const mirrored = oppositeKind(direct);
  return {
    ...doc,
    cp: {
      ...doc.cp,
      edges: doc.cp.edges.map((edge) =>
        edge.id === 8
          ? { ...edge, kind: direct }
          : edge.id === 9
            ? { ...edge, kind: mirrored }
            : edge,
      ),
    },
  };
}

function existingBaseFrame(): Frame3D {
  return {
    faces: [
      {
        face: 0,
        polygon: [[0, 0, 0], [0.5, 0, 0], [0.5, 0.5, 0], [0, 0.5, 0]],
        layer: 0,
      },
      {
        face: 1,
        polygon: [[0.5, 0, 0], [0, 0, 0], [0, 0.5, 0], [0.5, 0.5, 0]],
        layer: 3,
        mirrored: true,
      },
      {
        face: 2,
        polygon: [[0, 0.5, 0], [0, 1, 0], [0.5, 1, 0], [0.5, 0.5, 0]],
        layer: 2,
        mirrored: true,
      },
      {
        face: 3,
        polygon: [[0.5, 1, 0], [0, 1, 0], [0, 0.5, 0], [0.5, 0.5, 0]],
        layer: 1,
      },
    ],
    warnings: [],
  };
}

function finalFrame(direction: Direction): Frame3D {
  const layers = direction === "Up" ? [0, 1, 2, 3] : [2, 3, 0, 1];
  return {
    faces: [
      {
        face: 0,
        polygon: [[0, 0, 0], [0.5, 0, 0], [0.5, 0.5, 0], [0, 0.5, 0]],
        layer: layers[0],
      },
      {
        face: 1,
        polygon: [[0.5, 0, 0], [0, 0, 0], [0, 0.5, 0], [0.5, 0.5, 0]],
        layer: layers[1],
        mirrored: true,
      },
      {
        face: 2,
        polygon: [[0, 0.5, 0], [0, 0, 0], [0.5, 0, 0], [0.5, 0.5, 0]],
        layer: layers[2],
      },
      {
        face: 3,
        polygon: [[0.5, 0, 0], [0, 0, 0], [0, 0.5, 0], [0.5, 0.5, 0]],
        layer: layers[3],
        mirrored: true,
      },
    ],
    warnings: [FLOW_WARNING],
  };
}

function foldDrivers(direction: Direction) {
  const angle = direction === "Up" ? -180 : 180;
  return [
    { a: [0, 0.5] as Vec2, b: [0.5, 0.5] as Vec2, target_angle_deg: angle },
    { a: [0.5, 0.5] as Vec2, b: [1, 0.5] as Vec2, target_angle_deg: -angle },
  ];
}

function baseFixture(direction: Direction, existing: boolean): FoldFixture {
  if (!existing) {
    return {
      doc: BASE_DOC,
      faces: BASE_FACES,
      frame: BASE_FRAME,
      creaseAlreadyPresent: false,
    };
  }
  const doc = promoteAuxCrease(BASE_DOC, direction);
  doc.sequence = [
    {
      ...FIRST_FOLD_STEP,
      layer_order: [[0.25, 0.25], [0.25, 0.75], [0.75, 0.75], [0.75, 0.25]],
    },
  ];
  return {
    doc,
    faces: FINAL_FACES,
    frame: existingBaseFrame(),
    creaseAlreadyPresent: true,
  };
}

function fixtureView(fixture: FoldFixture): DocumentView {
  return {
    doc: fixture.doc,
    faces: fixture.faces,
    warnings: [],
    violations: [],
    frame: fixture.frame,
    skipped: [],
  };
}

function foldedView(fixture: FoldFixture, operation: FoldThroughOp): DocumentView {
  // static backend snapshotを返す前に入力を検査し、operationから都合のよい結果を作らない。
  const horizontal =
    Math.abs(operation.line[0][1] - 0.5) < 1e-8 &&
    Math.abs(operation.line[1][1] - 0.5) < 1e-8 &&
    Math.hypot(
      operation.line[1][0] - operation.line[0][0],
      operation.line[1][1] - operation.line[0][1],
    ) > 1 &&
    Math.min(operation.line[0][0], operation.line[1][0]) <= 0 &&
    Math.max(operation.line[0][0], operation.line[1][0]) >= 0.5;
  if (!horizontal || operation.keep_side_point[1] >= 0.5 || operation.target_layers !== null) {
    throw new Error("通し検査のbackend snapshotと異なるFoldThrough入力");
  }
  const creased = promoteAuxCrease(fixture.doc, operation.direction);
  const layerOrder =
    operation.direction === "Up"
      ? [[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]]
      : [[0.75, 0.75], [0.25, 0.75], [0.25, 0.25], [0.75, 0.25]];
  const doc: Document = {
    ...creased,
    sequence: [
      ...fixture.doc.sequence,
      {
        id: 1,
        kind: "Simple",
        drivers: foldDrivers(operation.direction),
        layer_order: layerOrder as Vec2[],
        alignment: operation.alignment ?? null,
        note: "",
      },
    ],
  };
  return {
    doc,
    faces: FINAL_FACES,
    warnings: [FLOW_WARNING],
    violations: [],
    frame: finalFrame(operation.direction),
    skipped: [],
  };
}

// 2Dは元の正方形が20..380px。3Dは最初の折りでx=0..0.5へ重なっている。
const P2 = {
  bl: [20, 380] as Vec2,
  tl: [20, 20] as Vec2,
  bottomLine: [110, 380] as Vec2,
  topLine: [110, 20] as Vec2,
  rightLower: [200, 290] as Vec2,
  leftMiddle: [20, 200] as Vec2,
  rightMiddle: [200, 200] as Vec2,
  middleLine: [110, 200] as Vec2,
};
const P3 = {
  bl: [79, 321] as Vec2,
  tl: [79, 79] as Vec2,
  bottomLine: [140, 321] as Vec2,
  topLine: [140, 79] as Vec2,
  rightLower: [200, 260] as Vec2,
  leftMiddle: [79, 200] as Vec2,
  rightMiddle: [200, 200] as Vec2,
  middleLine: [140, 200] as Vec2,
};

const pointTarget = (p: Vec2): AlignTarget => ({ kind: "point", p });
const lineTarget = (a: Vec2, b: Vec2): AlignTarget => ({ kind: "line", a, b });
const edgeCpPick = (id: number): AlignCpPick => ({ kind: "edge", id });
const BL: Vec2 = [0, 0];
const TL: Vec2 = [0, 1];
const LEFT_MIDDLE: Vec2 = [0, 0.5];
const RIGHT_MIDDLE: Vec2 = [0.5, 0.5];
const BOTTOM_LINE = lineTarget([0, 0], [0.5, 0]);
const TOP_LINE = lineTarget([0, 1], [0.5, 1]);
const RIGHT_LOWER_LINE = lineTarget([0.5, 0], [0.5, 0.5]);
const MIDDLE_LEFT_LINE = lineTarget([0, 0.5], [0.5, 0.5]);

const FLOW_CASES: AlignFlowCase[] = [
  {
    mode: "throughTwoPoints",
    label: "2点を通る",
    kinds: ["point", "point"],
    expectedPicks2d: [pointTarget(RIGHT_MIDDLE), pointTarget(LEFT_MIDDLE)],
    expectedPicks3d: [pointTarget(RIGHT_MIDDLE), pointTarget(LEFT_MIDDLE)],
    expectedCpPicks3d: [null, null],
    clicks2d: [P2.rightMiddle, P2.leftMiddle],
    clicks3d: [P3.rightMiddle, P3.leftMiddle],
    direction2d: "Up",
    direction3d: "Down",
  },
  {
    mode: "pointPoint",
    label: "点と点を合わせる",
    kinds: ["point", "point"],
    expectedPicks2d: [pointTarget(TL), pointTarget(BL)],
    expectedPicks3d: [pointTarget(TL), pointTarget(BL)],
    expectedCpPicks3d: [null, null],
    clicks2d: [P2.tl, P2.bl],
    clicks3d: [P3.tl, P3.bl],
    direction2d: "Down",
    direction3d: "Up",
  },
  {
    mode: "lineLine",
    label: "線と線を合わせる",
    kinds: ["line", "line"],
    expectedPicks2d: [TOP_LINE, BOTTOM_LINE],
    expectedPicks3d: [TOP_LINE, BOTTOM_LINE],
    expectedCpPicks3d: [edgeCpPick(4), edgeCpPick(1)],
    clicks2d: [P2.topLine, P2.bottomLine],
    clicks3d: [P3.topLine, P3.bottomLine],
    direction2d: "Up",
    direction3d: "Down",
  },
  {
    mode: "pointPerpendicularLine",
    label: "点を通り線と垂直に",
    kinds: ["point", "line"],
    expectedPicks2d: [pointTarget(RIGHT_MIDDLE), RIGHT_LOWER_LINE],
    expectedPicks3d: [pointTarget(RIGHT_MIDDLE), RIGHT_LOWER_LINE],
    expectedCpPicks3d: [null, edgeCpPick(10)],
    clicks2d: [P2.rightMiddle, P2.rightLower],
    clicks3d: [P3.rightMiddle, P3.rightLower],
    direction2d: "Down",
    direction3d: "Up",
  },
  {
    mode: "pointLineThrough",
    label: "点を線に合わせる(折り目が通る点を指定)",
    kinds: ["point", "line", "point"],
    expectedPicks2d: [pointTarget(TL), BOTTOM_LINE, pointTarget(LEFT_MIDDLE)],
    expectedPicks3d: [pointTarget(TL), BOTTOM_LINE, pointTarget(LEFT_MIDDLE)],
    expectedCpPicks3d: [null, edgeCpPick(1), null],
    clicks2d: [P2.tl, P2.bottomLine, P2.leftMiddle],
    clicks3d: [P3.tl, P3.bottomLine, P3.leftMiddle],
    direction2d: "Up",
    direction3d: "Down",
  },
  {
    mode: "pointToLinePointToLine",
    label: "2組を同時に合わせる",
    kinds: ["point", "line", "point", "line"],
    expectedPicks2d: [pointTarget(TL), BOTTOM_LINE, pointTarget(BL), TOP_LINE],
    expectedPicks3d: [pointTarget(TL), BOTTOM_LINE, pointTarget(BL), TOP_LINE],
    expectedCpPicks3d: [null, edgeCpPick(1), null, edgeCpPick(4)],
    clicks2d: [P2.tl, P2.bottomLine, P2.bl, P2.topLine],
    clicks3d: [P3.tl, P3.bottomLine, P3.bl, P3.topLine],
    direction2d: "Down",
    direction3d: "Up",
  },
  {
    mode: "pointLinePerpendicular",
    label: "点を線に合わせて別の線と垂直に",
    kinds: ["point", "line", "line"],
    expectedPicks2d: [pointTarget(TL), BOTTOM_LINE, RIGHT_LOWER_LINE],
    expectedPicks3d: [pointTarget(TL), BOTTOM_LINE, RIGHT_LOWER_LINE],
    expectedCpPicks3d: [null, edgeCpPick(1), edgeCpPick(10)],
    clicks2d: [P2.tl, P2.bottomLine, P2.rightLower],
    clicks3d: [P3.tl, P3.bottomLine, P3.rightLower],
    direction2d: "Up",
    direction3d: "Down",
  },
  {
    mode: "existingLine",
    label: "既存の線に沿って折る",
    kinds: ["line"],
    expectedPicks2d: [MIDDLE_LEFT_LINE],
    expectedPicks3d: [MIDDLE_LEFT_LINE],
    expectedCpPicks3d: [edgeCpPick(9)],
    clicks2d: [P2.middleLine],
    clicks3d: [P3.middleLine],
    direction2d: "Down",
    direction3d: "Up",
  },
];

const initialStoreState = useAppStore.getState();
let activeFixture = baseFixture("Up", false);

function stubLayout(): void {
  Object.defineProperty(HTMLCanvasElement.prototype, "clientWidth", {
    configurable: true,
    value: 400,
  });
  Object.defineProperty(HTMLCanvasElement.prototype, "clientHeight", {
    configurable: true,
    value: 400,
  });
  HTMLCanvasElement.prototype.getBoundingClientRect = () =>
    ({
      width: 400,
      height: 400,
      left: 0,
      top: 0,
      right: 400,
      bottom: 400,
      x: 0,
      y: 0,
    }) as DOMRect;
  HTMLCanvasElement.prototype.getContext = vi.fn(() => ({})) as never;
  Element.prototype.setPointerCapture = vi.fn();
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

function seedFixture(fixture: FoldFixture): void {
  activeFixture = fixture;
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({
    doc: fixture.doc,
    display: fixture.doc.display,
    faces: fixture.faces,
    hinges: new Set<number>(),
    frame3d: fixture.frame,
    activeTool: "fold",
    selection: { edgeIds: [], vertexIds: [] },
    currentStep: null,
    playT: 1,
    playing: false,
    drivers: new Map(),
    poseAngles: new Map(),
    foldDraft: null,
    pendingFoldThrough: null,
    foldThroughBusy: false,
    alignDraft: null,
    techniqueDraft: null,
    warnings: [],
    poseWarnings: [],
    replayWarnings: [],
    flatFoldViolations: [],
    violations: [],
    suspectHinges: [],
    relaxations: [],
    errorMessage: null,
    softMesh: null,
  });
  vi.mocked(ipc.sequenceApply).mockImplementation(async (operation) =>
    operation.type === "FoldThrough"
      ? foldedView(activeFixture, operation)
      : fixtureView(activeFixture),
  );
}

function pointerClick(canvas: HTMLCanvasElement, at: Vec2): void {
  fireEvent.pointerDown(canvas, {
    button: 0,
    pointerId: 1,
    clientX: at[0],
    clientY: at[1],
  });
  fireEvent.pointerUp(canvas, {
    button: 0,
    pointerId: 1,
    clientX: at[0],
    clientY: at[1],
  });
}

function expectVec2(actual: Vec2, expected: Vec2): void {
  expect(actual[0]).toBeCloseTo(expected[0], 8);
  expect(actual[1]).toBeCloseTo(expected[1], 8);
}

function orderedLine([a, b]: [Vec2, Vec2]): [Vec2, Vec2] {
  return a[0] < b[0] || (a[0] === b[0] && a[1] <= b[1]) ? [a, b] : [b, a];
}

function expectLineEquivalent(actual: [Vec2, Vec2], expected: [Vec2, Vec2]): void {
  const [actualA, actualB] = orderedLine(actual);
  const [expectedA, expectedB] = orderedLine(expected);
  expectVec2(actualA, expectedA);
  expectVec2(actualB, expectedB);
}

function expectTargetEquivalent(actual: AlignTarget, expected: AlignTarget): void {
  expect(actual.kind).toBe(expected.kind);
  if (actual.kind === "point" && expected.kind === "point") {
    expectVec2(actual.p, expected.p);
    return;
  }
  if (actual.kind === "line" && expected.kind === "line") {
    expectLineEquivalent([actual.a, actual.b], [expected.a, expected.b]);
    return;
  }
  throw new Error("選択対象の種類が期待と異なる");
}

function expectSolutionPosition(line: [Vec2, Vec2]): void {
  expect(line[0][1]).toBeCloseTo(0.5, 8);
  expect(line[1][1]).toBeCloseTo(0.5, 8);
  expect(Math.hypot(line[1][0] - line[0][0], line[1][1] - line[0][1])).toBeGreaterThan(
    1,
  );
  expect(Math.min(line[0][0], line[1][0])).toBeLessThanOrEqual(0);
  expect(Math.max(line[0][0], line[1][0])).toBeGreaterThanOrEqual(0.5);
}

function footprintKey(polygon: Vec3[]): string {
  return polygon
    .map(([x, y]) => `${x.toFixed(8)},${y.toFixed(8)}`)
    .sort()
    .join("|");
}

function footprintGroups(frame: Frame3D): Map<string, Frame3D["faces"]> {
  const groups = new Map<string, Frame3D["faces"]>();
  for (const face of frame.faces) {
    const key = footprintKey(face.polygon);
    groups.set(key, [...(groups.get(key) ?? []), face]);
  }
  return groups;
}

function footprintArea(frame: Frame3D): number {
  const points = frame.faces.flatMap((face) => face.polygon);
  const xs = points.map((point) => point[0]);
  const ys = points.map((point) => point[1]);
  return (Math.max(...xs) - Math.min(...xs)) * (Math.max(...ys) - Math.min(...ys));
}

function renderWholeFoldUi(): {
  cpCanvas: HTMLCanvasElement;
  viewerCanvas: HTMLCanvasElement;
} {
  const cpFitRef = { current: null } as React.RefObject<(() => void) | null>;
  const viewerFitRef = { current: null } as React.RefObject<(() => void) | null>;
  const view = render(
    <>
      <CpEditor fitRef={cpFitRef} />
      <Viewer3D fitRef={viewerFitRef} />
      <Timeline />
      <ContextPanel />
    </>,
  );
  const cpCanvas = view.container.querySelector<HTMLCanvasElement>(".cp-canvas");
  const viewerCanvas = view.container.querySelector<HTMLCanvasElement>(".viewer3d-canvas");
  if (!cpCanvas || !viewerCanvas) throw new Error("2Dまたは3Dのcanvasがない");
  return { cpCanvas, viewerCanvas };
}

function chooseModeAndTargets(
  testCase: AlignFlowCase,
  canvas: HTMLCanvasElement,
  source: "2d" | "3d",
): { picks: AlignTarget[]; line: [Vec2, Vec2] } {
  const clicks = source === "2d" ? testCase.clicks2d : testCase.clicks3d;
  const expectedPicks =
    source === "2d" ? testCase.expectedPicks2d : testCase.expectedPicks3d;
  fireEvent.click(screen.getByRole("button", { name: ALIGN_LABELS[testCase.mode] }));
  expect(useAppStore.getState().alignDraft?.mode).toBe(testCase.mode);

  for (const [index, at] of clicks.entries()) {
    pointerClick(canvas, at);
    const draft = useAppStore.getState().alignDraft;
    expect(draft?.picks.map((pick) => pick.kind)).toEqual(
      testCase.kinds.slice(0, index + 1),
    );
    const actual = draft?.picks[index];
    if (!actual) throw new Error(`${index + 1}番目の選択が記録されていない`);
    expectTargetEquivalent(actual, expectedPicks[index]);
  }
  const draft = useAppStore.getState().alignDraft;
  expect(draft?.solutions.length).toBeGreaterThan(0);
  const foldDraft = useAppStore.getState().foldDraft;
  if (!foldDraft) throw new Error("折り線が確定していない");
  expectSolutionPosition(foldDraft.line);
  if (source === "2d") {
    expect(draft?.cpPicks).toHaveLength(testCase.kinds.length);
    expect(draft?.cpPicks?.every((pick) => pick !== null)).toBe(true);
  } else {
    // 重なった線では、3Dで実際に見えている前面ownerのCP辺IDを保持する。
    // 点には3D上の頂点ID対応をまだ持たせないため、従来どおりnullのまま。
    expect(draft?.cpPicks).toEqual(testCase.expectedCpPicks3d);
  }
  return {
    picks: [...(draft?.picks ?? [])],
    line: [
      [...foldDraft.line[0]] as Vec2,
      [...foldDraft.line[1]] as Vec2,
    ],
  };
}

function chooseDirection(direction: Direction): void {
  if (direction === "Up") {
    fireEvent.click(screen.getByLabelText("向こうへ折る(山)"));
    expect(useAppStore.getState().foldDraft?.direction).toBe("Down");
    fireEvent.click(screen.getByLabelText("手前へ折る(谷)"));
  } else {
    fireEvent.click(screen.getByLabelText("手前へ折る(谷)"));
    expect(useAppStore.getState().foldDraft?.direction).toBe("Up");
    fireEvent.click(screen.getByLabelText("向こうへ折る(山)"));
  }
  expect(useAppStore.getState().foldDraft?.direction).toBe(direction);
}

function chooseUpperMovingSide(line: [Vec2, Vec2]): void {
  const upper: Vec2 = [0.25, 0.75];
  const side =
    (line[1][0] - line[0][0]) * (upper[1] - line[0][1]) -
    (line[1][1] - line[0][1]) * (upper[0] - line[0][0]);
  const desired = side > 0 ? "left" : "right";
  const desiredLabel = desired === "right" ? "こちら側" : "反対側";
  const otherLabel = desired === "right" ? "反対側" : "こちら側";
  fireEvent.click(screen.getByLabelText(otherLabel));
  fireEvent.click(screen.getByLabelText(desiredLabel));
  expect(useAppStore.getState().foldDraft?.movingSide).toBe(desired);
}

function expectViewerMatchesFrame(content: ViewerContentProbe, frame: Frame3D): void {
  expect(content.topology.slots.size).toBe(frame.faces.length);
  for (const face of frame.faces) {
    const slot = content.topology.slots.get(face.face);
    expect(slot).toBeTruthy();
    expect(slot?.count).toBe(face.polygon.length);
    if (!slot) continue;
    for (const [index, point] of face.polygon.entries()) {
      const offset = (slot.offset + index) * 3;
      expect(content.positions[offset]).toBeCloseTo(point[0], 7);
      expect(content.positions[offset + 1]).toBeCloseTo(point[1], 7);
    }
  }
}

async function chooseSettingsFoldAndAssert(
  testCase: AlignFlowCase,
  direction: Direction,
  selectedPicks: AlignTarget[],
  selectedLine: [Vec2, Vec2],
): Promise<void> {
  chooseUpperMovingSide(selectedLine);
  chooseDirection(direction);

  fireEvent.click(screen.getByLabelText("いちばん上の1枚"));
  expect(useAppStore.getState().foldDraft?.target).toBe("top");
  fireEvent.click(screen.getByLabelText("全ての層"));
  expect(useAppStore.getState().foldDraft?.target).toBe("all");

  const scene = held.scene;
  const setContent = scene.setContent as ReturnType<typeof vi.fn>;
  const contentCallsBeforeFold = setContent.mock.calls.length;
  const positionsBeforeFold = Array.from(
    (scene.content as ViewerContentProbe).positions,
  );
  fireEvent.click(screen.getByRole("button", { name: "折る" }));

  const beforeStepCount = activeFixture.doc.sequence.length;
  await waitFor(() =>
    expect(useAppStore.getState().doc?.sequence).toHaveLength(beforeStepCount + 1),
  );
  expect(vi.mocked(ipc.sequenceApply).mock.calls.map(([op]) => op.type)).toEqual([
    "PreviewFoldThrough",
    "FoldThrough",
  ]);
  const operation = vi
    .mocked(ipc.sequenceApply)
    .mock.calls.map(([op]) => op)
    .find((op): op is FoldThroughOp => op.type === "FoldThrough");
  if (!operation) throw new Error("FoldThroughが送られていない");
  expect(operation.direction).toBe(direction);
  expect(operation.up_to).toBe(beforeStepCount);
  expectLineEquivalent(operation.line, selectedLine);
  expect(operation.keep_side_point[1]).toBeLessThan(0.5);
  expect(operation.target_layers).toBeNull();
  expect(operation.accept_additional_crease).toBe(false);
  expect(operation.alignment).toEqual({ mode: testCase.mode, picks: selectedPicks });

  const state = useAppStore.getState();
  const directKind = creaseKind(direction);
  const mirroredKind = oppositeKind(directKind);
  expect(state.doc?.cp.edges.find((edge) => edge.id === 8)?.kind).toBe(directKind);
  expect(state.doc?.cp.edges.find((edge) => edge.id === 9)?.kind).toBe(mirroredKind);
  expect(state.faces).toEqual(FINAL_FACES);
  expect(state.doc?.cp.edges).toHaveLength(activeFixture.doc.cp.edges.length);
  if (activeFixture.creaseAlreadyPresent) {
    expect(activeFixture.doc.cp.edges.find((edge) => edge.id === 8)?.kind).toBe(
      directKind,
    );
  } else {
    expect(activeFixture.doc.cp.edges.find((edge) => edge.id === 8)?.kind).toBe("Aux");
    expect(activeFixture.doc.cp.edges.find((edge) => edge.id === 9)?.kind).toBe("Aux");
  }

  const appendedStep = state.doc?.sequence[beforeStepCount];
  expect(appendedStep?.alignment).toEqual(operation.alignment);
  expect(appendedStep?.drivers.map((driver) => driver.target_angle_deg)).toEqual(
    direction === "Up" ? [-180, 180] : [180, -180],
  );
  expect(appendedStep?.layer_order).toHaveLength(4);
  expect(screen.getByRole("button", { name: /^2 単純折り/ })).toBeTruthy();

  const finalFrame = state.frame3d;
  if (!finalFrame) throw new Error("折り上がりのFrame3Dがない");
  expect(finalFrame).not.toEqual(activeFixture.frame);
  expect(finalFrame.faces).toHaveLength(4);
  expect(new Set(finalFrame.faces.map((face) => face.layer))).toEqual(
    new Set([0, 1, 2, 3]),
  );
  expect(finalFrame.faces.map((face) => face.layer)).toEqual(
    direction === "Up" ? [0, 1, 2, 3] : [2, 3, 0, 1],
  );
  expect(finalFrame.faces.map((face) => face.mirrored ?? false)).toEqual([
    false,
    true,
    false,
    true,
  ]);
  const baseGroups = footprintGroups(activeFixture.frame);
  expect(baseGroups.size).toBe(activeFixture.creaseAlreadyPresent ? 2 : 1);
  for (const faces of baseGroups.values()) expect(faces).toHaveLength(2);
  const finalGroups = footprintGroups(finalFrame);
  expect(finalGroups.size).toBe(1);
  expect([...finalGroups.values()][0]).toHaveLength(4);
  expect(footprintArea(finalFrame)).toBeLessThan(footprintArea(activeFixture.frame));

  expect(state.warnings).toContain(FLOW_WARNING);
  expect(state.errorMessage).toBeNull();
  expect(state.foldThroughBusy).toBe(false);
  expect(state.pendingFoldThrough).toBeNull();
  expect(state.alignDraft).toBeNull();
  expect(state.foldDraft).toBeNull();

  await waitFor(() => {
    const drawn = held.cpDocument as Document;
    expect(drawn.cp.edges.find((edge) => edge.id === 8)?.kind).toBe(directKind);
    expect(drawn.cp.edges.find((edge) => edge.id === 9)?.kind).toBe(mirroredKind);
    expect(setContent.mock.calls.length).toBeGreaterThan(contentCallsBeforeFold);
    const content = scene.content as ViewerContentProbe;
    expect(Array.from(content.positions)).not.toEqual(positionsBeforeFold);
    expectViewerMatchesFrame(content, finalFrame);
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  held.cpDocument = null;
  held.scene = null as unknown as Record<string, unknown>;
  stubLayout();
  useAppStore.setState(initialStoreState, true);
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("合わせて折る: 選択から折り上がりまで", () => {
  it.each(FLOW_CASES)(
    "$label: 2Dで選び、指定方向・全層で2D/3D/手順へ反映する",
    async (testCase) => {
      seedFixture(
        baseFixture(testCase.direction2d, testCase.mode === "existingLine"),
      );
      const { cpCanvas } = renderWholeFoldUi();
      const selected = chooseModeAndTargets(testCase, cpCanvas, "2d");
      await chooseSettingsFoldAndAssert(
        testCase,
        testCase.direction2d,
        selected.picks,
        selected.line,
      );
    },
  );

  it.each(FLOW_CASES)(
    "$label: 3Dで選び、指定方向・全層で2D/3D/手順へ反映する",
    async (testCase) => {
      seedFixture(
        baseFixture(testCase.direction3d, testCase.mode === "existingLine"),
      );
      const { viewerCanvas } = renderWholeFoldUi();
      const selected = chooseModeAndTargets(testCase, viewerCanvas, "3d");
      await chooseSettingsFoldAndAssert(
        testCase,
        testCase.direction3d,
        selected.picks,
        selected.line,
      );
    },
  );
});
