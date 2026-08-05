// アプリ全体の状態を1本で管理するZustandストア(要件§2: フロント状態はストア1本)。
// IPC呼び出しはactionの中で行い、成功したらDocumentViewの内容を一括反映、
// 失敗(reject)はerrorMessageへ入れる(「止めずに警告」原則)。
// 全IPC要求は直列化キュー(ipcQueue.ts)を通し、連続操作でも適用順を発行順に
// 固定する。最新でない応答は破棄する(古いdocによる上書きを防ぐ)。

import { create } from "zustand";
import * as ipc from "../ipc/client";
import { createSerialQueue } from "./ipcQueue";
import { hingeEdgeIds } from "../lib/hinges";
import { advancePlayback, startPlayback } from "../lib/playback";
import type {
  Document,
  DocumentView,
  Driver,
  EditOp,
  Face,
  Frame3D,
  Paper,
  SeqOp,
} from "../lib/types";

/** ヒンジ角の連続操作(スライダー)を間引く間隔(ms) */
const POSE_THROTTLE_MS = 60;

/** 画面更新の仕組みが無い環境(テスト)で1コマを送る間隔(ms) */
const FALLBACK_FRAME_MS = 16;

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
  /** 折り角度を指定できる辺(ヒンジ)のID集合。doc/faces更新時に1度だけ導出する */
  hinges: ReadonlySet<number>;
  /** 表示中の折り手順番号(1始まり。0は折る前、nullは全手順を折った最新の状態) */
  currentStep: number | null;
  /** 表示中の手順の進み具合(0..1)。再生アニメーションの途中経過 */
  playT: number;
  /** 手順を自動再生中か */
  playing: boolean;
  /** 全手順を通した再生で飛ばされた手順のID(DocumentView由来。作品全体の結果)。
   * タイムラインの赤表示はこちらを見る */
  skipped: number[];
  /** 途中の手順までを再生し直したときに飛ばされた手順のID。
   * その手順までのことしか分からないので、全体のskippedとは別に持つ
   * (混ぜると、途中の手順を選んだだけで後ろの手順の赤表示が消えてしまう) */
  replaySkipped: number[];
  /** 手順再生からの警告(飛ばした理由など)。展開図・追従計算の警告とは別に持つ */
  replayWarnings: string[];
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
  /** 手順の追加・変更・削除(sequence_apply)。再生中なら止めてから送る */
  applySequenceOp: (op: SeqOp) => Promise<void>;
  /** 表示する手順を選ぶ(0=折る前、null=最新)。再生中なら止める */
  selectStep: (step: number | null) => void;
  /** 表示する手順を前後に動かす(コマ送り) */
  stepBy: (delta: number) => void;
  /** 再生と一時停止を切り替える */
  togglePlay: () => void;
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
    /** 予約と間隔の基準を完全に初期化する(テストの前処理用) */
    clearAll: () => {
      clear();
      lastRun = 0;
    },
  };
}

/** テスト用: 角度計算の間引き状態を初期化する。
 * 間引きの基準時刻はストア(アプリ全体で1個)が持ち続けるため、初期化できないと
 * 前のテストで進めた時計が次のテストの待ち時間に響く(テストの順序依存の原因)。*/
let resetThrottle: () => void = () => {};
export function resetPoseThrottle(): void {
  resetThrottle();
}

/** 中身が同じなら前の配列を使い回す。再生中は毎コマ結果が届くので、
 * 内容が変わらない限り同じ配列を返して画面の再描画を起こさない */
function keepIfSame<T>(prev: T[], next: T[]): T[] {
  const same = prev.length === next.length && prev.every((v, i) => v === next[i]);
  return same ? prev : next;
}

/** その手順が再生で飛ばされているか。作品全体の再生結果(DocumentView由来)を見て、
 * 途中まで再生し直したときの結果もあわせて見る。
 * 部分再生の結果だけで判断すると、途中の手順を選んだ瞬間に後ろの手順の印が消える */
export function isStepSkipped(
  s: { skipped: number[]; replaySkipped: number[] },
  stepId: number,
): boolean {
  return s.skipped.includes(stepId) || s.replaySkipped.includes(stepId);
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
   * isNewDocument=true(新規/開く)なら選択を解除しdocEpochを進める。
   * 手順が減ったときは表示中の手順番号を手順数まで詰める */
  const applyView = (view: DocumentView, isNewDocument: boolean) => {
    const total = view.doc.sequence.length;
    set((s) => ({
      doc: view.doc,
      faces: view.faces,
      hinges: hingeEdgeIds(view.doc, view.faces),
      warnings: view.warnings,
      violations: view.violations,
      skipped: view.skipped,
      currentStep:
        total === 0 || s.currentStep === null
          ? null
          : Math.min(s.currentStep, total),
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
        // 別の作品になったので角度指定・立体形状・再生位置は捨てる
        stopPlayback();
        pose.reset();
        set({
          drivers: new Map(),
          poseAngles: new Map(),
          poseWarnings: [],
          poseConverged: true,
          frame3d: r.value.frame,
          currentStep: null,
          playT: 1,
          replaySkipped: [],
          replayWarnings: [],
        });
      }
      if (r.value.doc.sequence.length > 0) {
        await syncSequence(r.value);
      } else if (!isNewDocument) {
        await syncPose();
      }
    } else if (r.isLatest) {
      fail(r.error);
    }
  };

  /** driversの配列表現(IPCの引数) */
  const driverList = (drivers: Map<number, number>): Driver[] =>
    [...drivers].map(([hinge, deg]) => ({ hinge, target_angle_deg: deg }));

  /** 全ての折り線に0度(平ら)を指定したdriver列。
   * 何も指定せずに送るとRust側が前回解(warm start)を引き継ぎ、折れたままの
   * 形が返る。平らに戻したいときは必ず0度を明示する必要がある */
  const flatDrivers = (hinges: ReadonlySet<number>): Driver[] =>
    [...hinges].map((hinge) => ({ hinge, target_angle_deg: 0 }));

  /** 追従計算を直列化キュー経由で実行し、3D表示へ反映する。
   * coalesce=true(スライダーの連続操作・手順再生)は「最新の形が出れば良い」
   * ので待ち行列に最新1件だけを置く(runLatest)。追い越された要求は実行されない。
   * 一方、解除操作のように「その1回だけ0度を明示する」意味を持つ要求は、
   * 追い越されると意味が失われるのでFIFO(run)で必ず送る。
   * 実行された成功応答は完了順に全て適用する(runViewCommandと同じ規約)。
   * 成功時にerrorMessageは触らない(編集側のエラー報告を消さないため) */
  const runPoseSolve = async (
    drivers: Driver[],
    coalesce = false,
  ): Promise<void> => {
    pose.reset();
    const call = () => ipc.poseSolve(drivers);
    const r = await (coalesce ? queue.runLatest(call) : queue.run(call));
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
  const syncPose = async (): Promise<void> => {
    // applyViewの直後に呼ばれるので、hingesは更新後の展開図から導出済み
    const hinges = get().hinges;
    const before = get().drivers;
    const kept = new Map([...before].filter(([hinge]) => hinges.has(hinge)));
    if (kept.size !== before.size) set({ drivers: kept });
    // 平らのまま(指定も立体形状も無い)なら計算する必要はない
    if (kept.size === 0 && get().frame3d === null) return;
    if (kept.size === 0) {
      // 指定が全て無くなった(線の種類変更などで折り線でなくなった)場合、
      // 空のまま送ると前回の計算結果を引き継いで折れたまま残り、画面には
      // 平らへ戻す操作も出なくなる。全ての折り線へ0度を明示して平らに戻す
      await runPoseSolve(flatDrivers(hinges));
      return;
    }
    await runPoseSolve(driverList(kept));
  };

  // スライダーの連続操作を間引く(実行時点の最新driversを送る)。
  // 間引いてもなお計算が追いつかない場合に備え、待ち行列は最新1件だけにする
  const pose = createTrailingThrottle(POSE_THROTTLE_MS, () => {
    void runPoseSolve(driverList(get().drivers), true);
  });
  resetThrottle = pose.clearAll;

  /** 手順の再生結果を3D表示へ反映する。
   * coalesce=true(再生アニメーション)は「最新の形が出れば良い」ので
   * 待ち行列に最新1件だけを置く(runLatest)。追い越された要求は実行されない */
  const runReplay = async (
    upTo: number,
    t: number,
    coalesce = false,
  ): Promise<void> => {
    const call = () => ipc.sequenceReplay(upTo, t);
    const r = await (coalesce ? queue.runLatest(call) : queue.run(call));
    if (r.ok) {
      const s = get();
      set({
        frame3d: r.value.frame,
        // upToまでの再生結果なので、作品全体のskippedは上書きしない
        replaySkipped: keepIfSame(s.replaySkipped, r.value.skipped),
        replayWarnings: keepIfSame(s.replayWarnings, r.value.warnings),
      });
    } else if (r.isLatest) {
      // 再生できない状態のままでは毎コマ失敗するので、止めて理由を知らせる
      stopPlayback();
      fail(r.error);
    }
  };

  /** 手順のある作品では、立体表示は手順の再生結果で表す(角度スライダーは
   * 手順の無い作品の確認用)。
   * 最新表示中(currentStep=null)はviewに自動再生の結果(立体・飛ばした手順・
   * 警告はview.warningsへ合流済み)が載っているので、再生は呼ばない。
   * 途中の手順を表示しているときだけ、その手順までを再生し直す */
  const syncSequence = async (view: DocumentView): Promise<void> => {
    // 表示中の手順番号はapplyViewで手順数まで詰めてある
    const step = get().currentStep;
    if (step === null) {
      set({ frame3d: view.frame, replaySkipped: [], replayWarnings: [] });
      return;
    }
    // 描き直すのはその手順を折り終えた形(t=1)。一時停止していた途中の進み具合を
    // 残すと、次に再生したとき表示が一度巻き戻ってから折り直される
    set({ playT: 1 });
    await runReplay(step, 1, true);
  };

  /** 次のコマを予約し、取り消す手続きを返す。画面のある環境では画面更新に
   * 合わせ、無い環境(テスト)ではタイマーで代用する */
  const scheduleFrame = (cb: (ts: number) => void): (() => void) => {
    if (typeof requestAnimationFrame === "function") {
      const id = requestAnimationFrame(cb);
      return () => cancelAnimationFrame(id);
    }
    const timer = setTimeout(() => cb(Date.now()), FALLBACK_FRAME_MS);
    return () => clearTimeout(timer);
  };

  /** 予約中のコマの取り消し手続き(nullなら予約していない) */
  let cancelFrame: (() => void) | null = null;
  /** 前のコマの時刻(0なら1コマ目。経過時間0として扱う) */
  let lastTs = 0;

  /** 再生を止める(予約中のコマも取り消す) */
  const stopPlayback = (): void => {
    cancelFrame?.();
    cancelFrame = null;
    if (get().playing) set({ playing: false });
  };

  /** 再生の1コマ。進み具合を計算し、その時点の形を(最新1件だけの)要求で描く */
  const tick = (ts: number): void => {
    cancelFrame = null;
    const s = get();
    if (!s.playing) return;
    const total = s.doc?.sequence.length ?? 0;
    const dt = lastTs === 0 ? 0 : ts - lastTs;
    lastTs = ts;
    const next = advancePlayback(
      { step: s.currentStep ?? 0, t: s.playT, playing: true },
      dt,
      total,
    );
    set({ currentStep: next.step, playT: next.t, playing: next.playing });
    void runReplay(next.step, next.t, true);
    if (next.playing) cancelFrame = scheduleFrame(tick);
  };

  return {
    doc: null,
    faces: [],
    hinges: new Set<number>(),
    warnings: [],
    violations: [],
    selection: EMPTY_SELECTION,
    activeTool: "select",
    frame3d: null,
    currentStep: null,
    playT: 1,
    playing: false,
    skipped: [],
    replaySkipped: [],
    replayWarnings: [],
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

    // 展開図が変わると再生中の形も変わるので、編集系はいずれも先に再生を止める
    // (止めないと、折り直した形が次のコマですぐ上書きされて一瞬跳ねて見える)
    applyEdit: (op) => {
      stopPlayback();
      return runViewCommand(() => ipc.editApply(op), false);
    },

    undo: () => {
      stopPlayback();
      return runViewCommand(() => ipc.editUndo(), false);
    },

    redo: () => {
      stopPlayback();
      return runViewCommand(() => ipc.editRedo(), false);
    },

    applySequenceOp: (op) => {
      // 手順が入れ替わると再生位置の意味が変わるので、先に止める
      stopPlayback();
      return runViewCommand(() => ipc.sequenceApply(op), false);
    },

    selectStep: (step) => {
      stopPlayback();
      const s = get();
      const total = s.doc?.sequence.length ?? 0;
      if (total === 0) {
        set({ currentStep: null, playT: 1 });
        return;
      }
      const upTo = step === null ? total : Math.max(0, Math.min(step, total));
      const next = step === null ? null : upTo;
      // すでに同じ形を表示しているなら描き直しを頼まない
      // (端で「次へ」を連打したときに、同じ要求が何度も飛ぶのを防ぐ)
      if (s.currentStep === next && s.playT === 1) return;
      set({ currentStep: next, playT: 1 });
      // 最新(null)の形も再生で作る(DocumentViewのframeと同じ内容になる)。
      // 途中まで折った表示から戻すときに、必ず最新の形へ描き直すため
      void runReplay(upTo, 1);
    },

    stepBy: (delta) => {
      const s = get();
      const total = s.doc?.sequence.length ?? 0;
      if (total === 0) return;
      // 最新表示(null)からのコマ送りは、最終手順を基準に数える
      const from = s.currentStep ?? total;
      s.selectStep(Math.max(0, Math.min(from + delta, total)));
    },

    togglePlay: () => {
      const s = get();
      if (s.playing) {
        stopPlayback();
        return;
      }
      const total = s.doc?.sequence.length ?? 0;
      const next = startPlayback(s.currentStep, s.playT, total);
      if (!next.playing) return;
      set({ currentStep: next.step, playT: next.t, playing: true });
      lastTs = 0; // 止めていた間の時間は進めない(1コマ目の経過時間は0)
      cancelFrame = scheduleFrame(tick);
    },

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
      // 指定を消しただけだと、この折り線は前回の計算結果(warm start)を
      // 引き継いで折れたまま残る。1回だけ0度(平ら)を明示して送り、
      // 次回以降は残りの指定だけで計算する(「全て平らに戻す」と同じ考え方)
      void runPoseSolve([
        ...driverList(drivers),
        { hinge, target_angle_deg: 0 },
      ]);
    },

    clearDrivers: () => {
      const hinges = get().hinges;
      set({ drivers: new Map() });
      void runPoseSolve(flatDrivers(hinges));
    },
  };
});
