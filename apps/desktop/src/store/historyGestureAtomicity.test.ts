// 下の2件は不具合D05を再現する検査で、現在は意図的に失敗する。
// 修正後は `it.fails` を `it` へ戻すこと。
// 修正には `apps/desktop/src/store/appStore.ts`、
// `apps/desktop/src/components/CpEditor/interaction.ts`、
// `apps/desktop/src-tauri/src/store.rs` の変更が必要。

import { beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_CONSTRUCT, type ConstructOptions } from "../lib/construct";
import { DEFAULT_CURVE, type CurveOptions } from "../lib/curve";
import type { Document, DocumentView, EdgeKind, EditOp, Vec2 } from "../lib/types";
import {
  initialEphemeralState,
  onMouseDown,
  type InteractionCtx,
} from "../components/CpEditor/interaction";

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
}));

import * as ipc from "../ipc/client";
import { useAppStore } from "./appStore";

const HISTORY_LIMIT = 100;

const ORIGINAL_DOCUMENT: Document = {
  schema_version: 1,
  paper: { width_mm: 100, height_mm: 100 },
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
    ],
    next_vertex_id: 4,
    next_edge_id: 4,
  },
  sequence: [],
  display: {
    front_color: [237, 28, 36],
    back_color: [255, 255, 255],
    grid_divisions: 8,
  },
};

const cloneDocument = (doc: Document): Document => structuredClone(doc);
const sameDocument = (left: Document | null, right: Document): boolean =>
  JSON.stringify(left) === JSON.stringify(right);

let backendDocument = cloneDocument(ORIGINAL_DOCUMENT);
let undoHistory: Document[] = [];
let redoHistory: Document[] = [];

function viewOf(doc: Document): DocumentView {
  return {
    doc: cloneDocument(doc),
    faces: [],
    warnings: [],
    violations: [],
    frame: null,
    skipped: [],
  };
}

/**
 * この検査で必要なのは、1回のeditApplyが履歴を1件増やすという境界だけ。
 * 線分交差などの幾何は対象外なので、各編集を識別できる辺1本として記録する。
 */
function applyBackendEdit(op: EditOp): DocumentView {
  undoHistory = [...undoHistory, cloneDocument(backendDocument)].slice(-HISTORY_LIMIT);
  redoHistory = [];
  const next = cloneDocument(backendDocument);
  const id = next.cp.next_edge_id;
  next.cp.edges.push({
    id,
    v0: 0,
    v1: 1,
    kind: op.type === "AddSegment" ? op.kind : "Aux",
  });
  next.cp.next_edge_id = id + 1;
  backendDocument = next;
  return viewOf(backendDocument);
}

function undoBackendEdit(): DocumentView {
  const previous = undoHistory.pop();
  if (!previous) throw new Error("元に戻せる操作がありません");
  redoHistory = [...redoHistory, cloneDocument(backendDocument)].slice(-HISTORY_LIMIT);
  backendDocument = previous;
  return viewOf(backendDocument);
}

function redoBackendEdit(): DocumentView {
  const next = redoHistory.pop();
  if (!next) throw new Error("やり直せる操作がありません");
  undoHistory = [...undoHistory, cloneDocument(backendDocument)].slice(-HISTORY_LIMIT);
  backendDocument = next;
  return viewOf(backendDocument);
}

const toScreen = (p: Vec2): Vec2 => [p[0] * 500, 500 - p[1] * 500];

function interactionContext(options: {
  construct?: Partial<ConstructOptions>;
  curve?: Partial<CurveOptions>;
}) {
  const pending: Promise<void>[] = [];
  const doc = cloneDocument(ORIGINAL_DOCUMENT);
  const ctx: InteractionCtx = {
    doc,
    view: { scale: 500, offsetX: 0, offsetY: 500 },
    tool: "construct",
    selection: { edgeIds: [], vertexIds: [] },
    alignDraft: null,
    alignPickDoc: doc,
    faces: [],
    frame3d: null,
    construct: { ...DEFAULT_CONSTRUCT, ...options.construct },
    curve: { ...DEFAULT_CURVE, ...options.curve },
    wheelBehavior: "scroll",
    violations: [],
    mirrorAxis: null,
    state: initialEphemeralState(),
    setView: vi.fn(),
    applyEdit: (op) => {
      pending.push(useAppStore.getState().applyEdit(op));
    },
    drawSegment: (a: Vec2, b: Vec2, kind: EdgeKind) => {
      pending.push(useAppStore.getState().drawSegment(a, b, kind));
    },
    drawCurve: (points: Vec2[], kind: EdgeKind) => {
      pending.push(useAppStore.getState().drawCurve(points, kind));
    },
    setSelection: vi.fn(),
    beginFoldDraft: vi.fn(),
    pickAlignTarget: vi.fn(),
  };
  return { ctx, pending };
}

beforeEach(() => {
  vi.clearAllMocks();
  backendDocument = cloneDocument(ORIGINAL_DOCUMENT);
  undoHistory = [];
  redoHistory = [];
  vi.mocked(ipc.editApply).mockImplementation(async (op) => applyBackendEdit(op));
  vi.mocked(ipc.editUndo).mockImplementation(async () => undoBackendEdit());
  vi.mocked(ipc.editRedo).mockImplementation(async () => redoBackendEdit());
  useAppStore.setState({
    doc: cloneDocument(ORIGINAL_DOCUMENT),
    faces: [],
    selection: { edgeIds: [], vertexIds: [] },
    errorMessage: null,
    mirrorDraw: false,
    construct: { ...DEFAULT_CONSTRUCT },
    curve: { ...DEFAULT_CURVE },
    angleUndoStack: [],
    angleRedoStack: [],
    docUndoDepth: 0,
  });
});

describe("D05: 1回の入力を1回で元に戻す", () => {
  it.fails(
    "D05: 22.5度刻みの角度線を1回作ると、元に戻す1回で作る前へ戻る",
    async () => {
      const { ctx, pending } = interactionContext({
        construct: { kind: "angle", stepDeg: 22.5 },
      });

      onMouseDown(ctx, toScreen([0.5, 0.5]), 0);
      await Promise.all(pending);
      const historyEntries = vi.mocked(ipc.editApply).mock.calls.length;
      expect(useAppStore.getState().doc).not.toEqual(ORIGINAL_DOCUMENT);

      await useAppStore.getState().undo();

      expect(
        sameDocument(useAppStore.getState().doc, ORIGINAL_DOCUMENT),
        `角度線1回が${historyEntries}件の履歴に分裂している`,
      ).toBe(true);
    },
  );

  it.fails(
    "D05: 曲線を1本引くと、元に戻す1回で引く前へ戻り、履歴上限でも欠けない",
    async () => {
      const { ctx, pending } = interactionContext({
        curve: { enabled: true, shape: "arc", segments: 200, rulings: true },
      });
      ctx.tool = "valley";

      onMouseDown(ctx, toScreen([0.1, 0.2]), 0);
      onMouseDown(ctx, toScreen([0.9, 0.2]), 0);
      onMouseDown(ctx, toScreen([0.5, 0.8]), 0);
      await Promise.all(pending);
      const historyEntries = vi.mocked(ipc.editApply).mock.calls.length;
      const retainedEntries = undoHistory.length;
      expect(useAppStore.getState().doc).not.toEqual(ORIGINAL_DOCUMENT);

      await useAppStore.getState().undo();
      expect.soft(
        sameDocument(useAppStore.getState().doc, ORIGINAL_DOCUMENT),
        `曲線1本が${historyEntries}件の履歴に分裂している`,
      ).toBe(true);

      // 変更前の全履歴を使い切る。上限を超えて古い履歴が欠けていれば、ここでも戻らない。
      for (let i = 1; i < retainedEntries; i += 1) {
        await useAppStore.getState().undo();
      }
      expect(
        sameDocument(useAppStore.getState().doc, ORIGINAL_DOCUMENT),
        `${historyEntries}件中${retainedEntries}件しか保持されず、作る前へ戻せない`,
      ).toBe(true);
    },
  );
});
