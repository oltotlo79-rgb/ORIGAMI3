// アプリ全体の状態を1本で管理するZustandストア(要件§2: フロント状態はストア1本)。
// IPC呼び出しはactionの中で行い、成功したらDocumentViewの内容を一括反映、
// 失敗(reject)はerrorMessageへ入れる(「止めずに警告」原則)。

import { create } from "zustand";
import * as ipc from "../ipc/client";
import type {
  Document,
  DocumentView,
  EditOp,
  Face,
  Frame3D,
  Paper,
} from "../lib/types";

export type ToolId = "select" | "mountain" | "valley" | "aux" | "delete";

/** 選択中の線・頂点(ID)。DOMのSelectionと紛れないよう注意 */
export interface Selection {
  edgeIds: number[];
  vertexIds: number[];
}

interface AppState {
  doc: Document | null;
  faces: Face[];
  warnings: string[];
  violations: number[];
  selection: Selection;
  activeTool: ToolId;
  /** 3D表示フレーム(Task 1-9で使用) */
  frame3d: Frame3D | null;
  /** 表示中の折り手順番号(Task 1-9以降で使用) */
  currentStep: number | null;
  errorMessage: string | null;

  newDocument: (paper: Paper) => Promise<void>;
  openDocument: (path: string) => Promise<void>;
  saveDocument: (path: string | null) => Promise<void>;
  applyEdit: (op: EditOp) => Promise<void>;
  undo: () => Promise<void>;
  redo: () => Promise<void>;
  setTool: (tool: ToolId) => void;
  setSelection: (selection: Selection) => void;
}

const EMPTY_SELECTION: Selection = { edgeIds: [], vertexIds: [] };

/** ドキュメント更新後、存在しなくなったIDを選択から取り除く */
function pruneSelection(selection: Selection, doc: Document): Selection {
  const edgeIds = new Set(doc.cp.edges.map((e) => e.id));
  const vertexIds = new Set(doc.cp.vertices.map((v) => v.id));
  return {
    edgeIds: selection.edgeIds.filter((id) => edgeIds.has(id)),
    vertexIds: selection.vertexIds.filter((id) => vertexIds.has(id)),
  };
}

export const useAppStore = create<AppState>((set, get) => {
  /** DocumentViewの内容で状態を一括更新する(成功時共通処理) */
  const applyView = (view: DocumentView, clearSelection: boolean) => {
    set((s) => ({
      doc: view.doc,
      faces: view.faces,
      warnings: view.warnings,
      violations: view.violations,
      selection: clearSelection
        ? EMPTY_SELECTION
        : pruneSelection(s.selection, view.doc),
      errorMessage: null,
    }));
  };

  /** IPC失敗(reject)をerrorMessageへ反映する */
  const fail = (e: unknown) => {
    set({ errorMessage: typeof e === "string" ? e : String(e) });
  };

  return {
    doc: null,
    faces: [],
    warnings: [],
    violations: [],
    selection: EMPTY_SELECTION,
    activeTool: "select",
    frame3d: null,
    currentStep: null,
    errorMessage: null,

    newDocument: async (paper) => {
      try {
        applyView(await ipc.documentNew(paper), true);
      } catch (e) {
        fail(e);
      }
    },

    openDocument: async (path) => {
      try {
        applyView(await ipc.documentOpen(path), true);
      } catch (e) {
        fail(e);
      }
    },

    saveDocument: async (path) => {
      try {
        await ipc.documentSave(path);
        set({ errorMessage: null });
      } catch (e) {
        fail(e);
      }
    },

    applyEdit: async (op) => {
      try {
        applyView(await ipc.editApply(op), false);
      } catch (e) {
        fail(e);
      }
    },

    undo: async () => {
      try {
        applyView(await ipc.editUndo(), false);
      } catch (e) {
        fail(e);
      }
    },

    redo: async () => {
      try {
        applyView(await ipc.editRedo(), false);
      } catch (e) {
        fail(e);
      }
    },

    setTool: (tool) => {
      // ツール切替時は選択を保つ必要がないので解除する
      if (get().activeTool !== tool) {
        set({ activeTool: tool, selection: EMPTY_SELECTION });
      }
    },

    setSelection: (selection) => set({ selection }),
  };
});
