// アプリ全体の状態を1本で管理するZustandストア(要件§2: フロント状態はストア1本)。
// IPC呼び出しはactionの中で行い、成功したらDocumentViewの内容を一括反映、
// 失敗(reject)はerrorMessageへ入れる(「止めずに警告」原則)。
// 全IPC要求は直列化キュー(ipcQueue.ts)を通し、連続操作でも適用順を発行順に
// 固定する。最新でない応答は破棄する(古いdocによる上書きを防ぐ)。

import { create } from "zustand";
import * as ipc from "../ipc/client";
import { createSerialQueue } from "./ipcQueue";
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
  /** 作品の世代番号。新規/開くの成功で増える(エディタの表示リセット合図) */
  docEpoch: number;

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
  const queue = createSerialQueue();

  /** DocumentViewの内容で状態を一括更新する(成功時共通処理)。
   * isNewDocument=true(新規/開く)なら選択を解除しdocEpochを進める */
  const applyView = (view: DocumentView, isNewDocument: boolean) => {
    set((s) => ({
      doc: view.doc,
      faces: view.faces,
      warnings: view.warnings,
      violations: view.violations,
      selection: isNewDocument
        ? EMPTY_SELECTION
        : pruneSelection(s.selection, view.doc),
      errorMessage: null,
      docEpoch: isNewDocument ? s.docEpoch + 1 : s.docEpoch,
    }));
  };

  /** IPC失敗(reject)をerrorMessageへ反映する */
  const fail = (e: unknown) => {
    set({ errorMessage: typeof e === "string" ? e : String(e) });
  };

  /** DocumentViewを返すコマンドを直列化キュー経由で実行し、結果を反映する。
   * 直列化により適用順の逆転は起きないため、成功したviewは完了順に全て適用する
   * (途中の成功を捨てると、後続が失敗したときにバックエンドと画面が食い違う)。
   * 失敗の報告だけは最新要求に限る(古い失敗の直後には必ず新しい結果が続く) */
  const runViewCommand = async (
    task: () => Promise<DocumentView>,
    isNewDocument: boolean,
  ): Promise<void> => {
    const r = await queue.run(task);
    if (r.ok) {
      applyView(r.value, isNewDocument);
    } else if (r.isLatest) {
      fail(r.error);
    }
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
    docEpoch: 0,

    newDocument: (paper) => runViewCommand(() => ipc.documentNew(paper), true),

    openDocument: (path) => runViewCommand(() => ipc.documentOpen(path), true),

    saveDocument: async (path) => {
      // 保存も直列化する(直前の編集が確定してから保存されることを保証)。
      // 状態の更新はないので、応答の新旧に関わらず結果を報告する
      const r = await queue.run(() => ipc.documentSave(path));
      if (r.ok) {
        set({ errorMessage: null });
      } else {
        fail(r.error);
      }
    },

    applyEdit: (op) => runViewCommand(() => ipc.editApply(op), false),

    undo: () => runViewCommand(() => ipc.editUndo(), false),

    redo: () => runViewCommand(() => ipc.editRedo(), false),

    setTool: (tool) => {
      // ツール切替時は選択を保つ必要がないので解除する
      if (get().activeTool !== tool) {
        set({ activeTool: tool, selection: EMPTY_SELECTION });
      }
    },

    setSelection: (selection) => set({ selection }),
  };
});
