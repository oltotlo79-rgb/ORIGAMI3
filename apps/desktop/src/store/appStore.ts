// アプリ全体の状態を1本で管理するZustandストア(要件§2: フロント状態はストア1本)。
// IPC呼び出しはactionの中で行い、成功したらDocumentViewの内容を一括反映、
// 失敗(reject)はerrorMessageへ入れる(「止めずに警告」原則)。
// 全IPC要求は直列化キュー(ipcQueue.ts)を通し、連続操作でも適用順を発行順に
// 固定する。最新でない応答は破棄する(古いdocによる上書きを防ぐ)。

import { create } from "zustand";
import * as ipc from "../ipc/client";
import { createSerialQueue } from "./ipcQueue";
import { hingeEdgeIds } from "../lib/hinges";
import type {
  Document,
  DocumentView,
  Driver,
  EditOp,
  Face,
  Frame3D,
  Paper,
} from "../lib/types";

/** ヒンジ角の連続操作(スライダー)を間引く間隔(ms) */
const POSE_THROTTLE_MS = 60;

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
  /** 3D表示フレーム。nullなら平ら(展開図から直接描く) */
  frame3d: Frame3D | null;
  /** 表示中の折り手順番号(Task 1-9以降で使用) */
  currentStep: number | null;
  errorMessage: string | null;
  /** 作品の世代番号。新規/開くの成功で増える(エディタの表示リセット合図) */
  docEpoch: number;
  /** 利用者が指定した折り角度(度)。キーは辺ID */
  drivers: Map<number, number>;
  /** 直近のソルバー解(度)。キーは辺ID。未指定ヒンジの現在角の表示に使う */
  poseAngles: Map<number, number>;
  /** 追従計算からの警告(不収束など)。展開図の検査警告とは別に持つ */
  poseWarnings: string[];
  /** 追従計算が収束したか(falseなら3D区画のバッジで知らせる) */
  poseConverged: boolean;

  newDocument: (paper: Paper) => Promise<void>;
  openDocument: (path: string) => Promise<void>;
  saveDocument: (path: string | null) => Promise<void>;
  applyEdit: (op: EditOp) => Promise<void>;
  undo: () => Promise<void>;
  redo: () => Promise<void>;
  setTool: (tool: ToolId) => void;
  setSelection: (selection: Selection) => void;
  /** ヒンジの折り角度を指定する(60ms間引きで追従計算を呼ぶ) */
  setDriverAngle: (hinge: number, deg: number) => void;
  /** 1本の角度指定を解除する(形は残りの指定から計算し直す) */
  clearDriver: (hinge: number) => void;
  /** 全ての角度指定を解除して平らに戻す */
  clearDrivers: () => void;
}

const EMPTY_SELECTION: Selection = { edgeIds: [], vertexIds: [] };

/**
 * トレーリングエッジのスロットル。連続呼び出しをintervalMsごと1回に間引き、
 * 最後の呼び出しは必ず実行する(スライダーを離した位置の角度が捨てられない)。
 * reset()は予約を取り消し、「たった今実行した」ものとして間隔を測り直す
 * (別経路で要求を送った直後に、間引き分が二重に飛ぶのを防ぐ)。
 */
function createTrailingThrottle(intervalMs: number, fn: () => void) {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let lastRun = 0;
  const clear = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };
  return {
    schedule: () => {
      clear();
      const wait = Math.max(0, intervalMs - (Date.now() - lastRun));
      timer = setTimeout(() => {
        timer = null;
        lastRun = Date.now();
        fn();
      }, wait);
    },
    reset: () => {
      clear();
      lastRun = Date.now();
    },
  };
}

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
      if (isNewDocument) {
        // 別の作品になったので角度指定と立体形状は捨てる(平らから始める)
        pose.reset();
        set({
          drivers: new Map(),
          poseAngles: new Map(),
          poseWarnings: [],
          poseConverged: true,
          frame3d: null,
        });
      } else {
        await syncPose(r.value);
      }
    } else if (r.isLatest) {
      fail(r.error);
    }
  };

  /** driversの配列表現(IPCの引数) */
  const driverList = (drivers: Map<number, number>): Driver[] =>
    [...drivers].map(([hinge, deg]) => ({ hinge, target_angle_deg: deg }));

  /** 追従計算を直列化キュー経由で実行し、3D表示へ反映する。
   * 成功応答は完了順に全て適用する(runViewCommandと同じ規約)。
   * 成功時にerrorMessageは触らない(編集側のエラー報告を消さないため) */
  const runPoseSolve = async (drivers: Driver[]): Promise<void> => {
    pose.reset();
    const r = await queue.run(() => ipc.poseSolve(drivers));
    if (r.ok) {
      set({
        frame3d: r.value.frame,
        poseWarnings: r.value.frame.warnings,
        poseConverged: r.value.converged,
        poseAngles: new Map(
          Object.entries(r.value.angles).map(([id, deg]) => [Number(id), deg]),
        ),
      });
    } else if (r.isLatest) {
      fail(r.error);
    }
  };

  /** 展開図の更新後、残っている角度指定で立体形状を計算し直す。
   * 折り線でなくなった辺の指定は捨てる */
  const syncPose = async (view: DocumentView): Promise<void> => {
    const hinges = hingeEdgeIds(view.doc, view.faces);
    const before = get().drivers;
    const kept = new Map([...before].filter(([hinge]) => hinges.has(hinge)));
    if (kept.size !== before.size) set({ drivers: kept });
    // 平らのまま(指定も立体形状も無い)なら計算する必要はない
    if (kept.size === 0 && get().frame3d === null) return;
    await runPoseSolve(driverList(kept));
  };

  // スライダーの連続操作を間引く(実行時点の最新driversを送る)
  const pose = createTrailingThrottle(POSE_THROTTLE_MS, () => {
    void runPoseSolve(driverList(get().drivers));
  });

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
    drivers: new Map(),
    poseAngles: new Map(),
    poseWarnings: [],
    poseConverged: true,

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

    setDriverAngle: (hinge, deg) => {
      // 画面の反応を優先し、指定はその場で反映してから計算を間引いて依頼する
      const drivers = new Map(get().drivers);
      drivers.set(hinge, deg);
      set({ drivers });
      pose.schedule();
    },

    clearDriver: (hinge) => {
      const drivers = new Map(get().drivers);
      if (!drivers.delete(hinge)) return;
      set({ drivers });
      void runPoseSolve(driverList(drivers));
    },

    clearDrivers: () => {
      const { doc, faces } = get();
      set({ drivers: new Map() });
      // 平らに戻す: 何も指定せずに送ると前回の計算結果が引き継がれてしまうため、
      // 全ての折り線に0度(平ら)を明示して送る
      const flat = doc
        ? [...hingeEdgeIds(doc, faces)].map((hinge) => ({
            hinge,
            target_angle_deg: 0,
          }))
        : [];
      void runPoseSolve(flat);
    },
  };
});
