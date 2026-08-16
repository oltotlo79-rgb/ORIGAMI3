// appStoreのテスト:
//  - 直列化まわり:「成功したviewはisLatestに関わらず破棄されない」
//    (A成功→B失敗でも、画面はAのdocを保持しバックエンドと一致する)
//  - 折り角度の指定: 16ms間引き・全解除・展開図編集後の追従
//  - 手順の表示と再生: 手順選択・コマ送り・アニメーションの進行と停止・
//    最新1件だけを送る間引き(coalescing)

import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  DocumentView,
  Driver,
  FoldStep,
  ReplayResult,
  SolveResult,
  TechniqueKind,
  Vec2,
} from "../lib/types";
import { STEP_DURATION_MS } from "../lib/playback";

vi.mock("../ipc/client", () => ({
  documentNew: vi.fn(),
  documentOpen: vi.fn(),
  documentSave: vi.fn(),
  editApply: vi.fn(),
  editApplyBatch: vi.fn(),
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
import {
  canFoldNow,
  foldInsertAt,
  isStepSkipped,
  poseRecordReason,
  resetPoseThrottle,
  useAppStore,
} from "./appStore";

/** 角度の間引き間隔(appStore.tsのPOSE_THROTTLE_MS)より少し長く待つ時間(ms) */
const POSE_WAIT_MS = 100;
/** 間引きが追加で飛ばないことを確かめるために待つ時間(ms) */
const REAL_WAIT_MS = 200;

/** 手動でresolve/rejectできるPromise */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** markで区別できる最小のDocumentView(正方形・線なし) */
function makeView(mark: number): DocumentView {
  return {
    doc: {
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
        ],
        next_vertex_id: 4,
        next_edge_id: mark, // 区別用の印
      },
      sequence: [],
      display: {
        front_color: [237, 28, 36],
        back_color: [255, 255, 255],
        grid_divisions: 8,
      },
    },
    faces: [],
    warnings: [],
    violations: [],
    frame: null,
    skipped: [],
  };
}

/** 対角線(辺ID 5、山折り)で2つの面に分かれた正方形のview */
function makeHingeView(mark: number): DocumentView {
  const view = makeView(mark);
  view.doc.cp.edges.push({ id: 5, v0: 0, v1: 2, kind: "Mountain" });
  view.faces = [
    { id: 0, vertices: [0, 1, 2], edges: [0, 1, 5] },
    { id: 1, vertices: [0, 2, 3], edges: [5, 2, 3] },
  ];
  return view;
}

/** 平らなSolveResult(角度だけ差し替えられる) */
function makeSolveResult(angles: Record<string, number> = {}): SolveResult {
  return {
    frame: { faces: [], warnings: [] },
    converged: true,
    angles,
    iterations: 1,
  };
}

/** poseSolveへ渡された「固定する折り線」(呼び出し番号ごと) */
function poseCalls(): Driver[][] {
  return vi.mocked(ipc.poseSolve).mock.calls.map(([drivers]) => drivers);
}

/** poseSolveへ渡された「なるべく保ちたい折り線」(呼び出し番号ごと) */
function poseKeeps(): Driver[][] {
  return vi.mocked(ipc.poseSolve).mock.calls.map(([, keep]) => keep ?? []);
}

/** sequenceReplayへ渡された引数(呼び出し番号ごと) */
function replayCalls(): [number, number][] {
  return vi.mocked(ipc.sequenceReplay).mock.calls.map(([upTo, t]) => [upTo, t]);
}

/** 直近のsequenceReplay呼び出しの引数(まだ無ければundefined) */
function lastReplayCall(): [number, number] | undefined {
  const calls = replayCalls();
  return calls[calls.length - 1];
}

/** 単純折りの手順を1つ作る */
function makeStep(id: number): FoldStep {
  return {
    id,
    kind: "Simple",
    drivers: [{ a: [0, 0], b: [1, 1], target_angle_deg: 180 }],
    layer_order: null,
    note: "",
  };
}

/** 手順をcount個持つview(手順IDは1始まり) */
function makeStepView(mark: number, count: number): DocumentView {
  const view = makeHingeView(mark);
  view.doc.sequence = Array.from({ length: count }, (_, i) => makeStep(i + 1));
  view.frame = { faces: [], warnings: [] };
  return view;
}

/** 空のReplayResult(飛ばした手順・警告は差し替えられる) */
function makeReplayResult(): ReplayResult {
  return { frame: { faces: [], warnings: [] }, skipped: [], warnings: [] };
}

/** 手順count個の作品を表示中の状態にする(IPCは呼ばない) */
function seedSequence(count: number, currentStep: number | null = null): void {
  const view = makeStepView(1000, count);
  useAppStore.setState({
    doc: view.doc,
    faces: view.faces,
    hinges: new Set([5]),
    currentStep,
    playT: 1,
    playing: false,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  // 間引きの基準時刻はストア(アプリ全体で1個)が持ち続けるため、
  // テストごとに初期化して前のテストの時計を持ち込まない
  resetPoseThrottle();
  vi.mocked(ipc.poseSolve).mockResolvedValue(makeSolveResult());
  vi.mocked(ipc.sequenceReplay).mockResolvedValue(makeReplayResult());
  useAppStore.setState({
    doc: null,
    faces: [],
    hinges: new Set<number>(),
    warnings: [],
    violations: [],
    flatFoldViolations: [],
    selection: { edgeIds: [], vertexIds: [] },
    errorMessage: null,
    docEpoch: 0,
    drivers: new Map(),
    poseAngles: new Map(),
    poseWarnings: [],
    poseConverged: true,
    poseBestEffort: false,
    poseClosureRms: null,
    contactDetected: false,
    sequenceTargets: new Map(),
    relaxations: [],
    activeAngleIntent: null,
    angleIntentGeneration: 0,
    frame3d: null,
    suspectHinges: [],
    currentStep: null,
    playT: 1,
    playing: false,
    skipped: [],
    replaySkipped: [],
    replayWarnings: [],
    activeTool: "select",
    foldDraft: null,
    pendingFoldThrough: null,
    foldThroughBusy: false,
    techniqueDraft: null,
    recovery: null,
  });
  vi.mocked(ipc.recoveryCheck).mockResolvedValue(null);
});

describe("appStore 直列化と応答の反映", () => {
  it("後続の要求で最新でなくなっても、成功したviewは破棄されない", async () => {
    // A(成功・完了前にBが積まれる)→ B(失敗): 画面はAのdocを保持し、Bのエラーを報告
    const viewA = makeView(100);
    const slowA = deferred<DocumentView>();
    vi.mocked(ipc.editApply)
      .mockReturnValueOnce(slowA.promise)
      .mockRejectedValueOnce("Bは失敗しました");

    const store = useAppStore.getState();
    const pA = store.applyEdit({ type: "RemoveEdges", ids: [10] });
    const pB = store.applyEdit({ type: "RemoveEdges", ids: [11] });
    slowA.resolve(viewA);
    await Promise.all([pA, pB]);

    const s = useAppStore.getState();
    expect(s.doc).toEqual(viewA.doc); // Aの成功が適用されている(破棄されない)
    expect(s.errorMessage).toBe("Bは失敗しました");
  });

  it("成功が続く場合は完了順に適用され、最後のviewが残る", async () => {
    const slowA = deferred<DocumentView>();
    vi.mocked(ipc.editApply)
      .mockReturnValueOnce(slowA.promise)
      .mockResolvedValueOnce(makeView(200));

    const store = useAppStore.getState();
    const pA = store.applyEdit({ type: "RemoveEdges", ids: [10] });
    const pB = store.applyEdit({ type: "RemoveEdges", ids: [11] });
    slowA.resolve(makeView(100));
    await Promise.all([pA, pB]);

    const s = useAppStore.getState();
    expect(s.doc?.cp.next_edge_id).toBe(200); // 発行順=完了順で最後のBが残る
    expect(s.errorMessage).toBeNull();
  });

  it("新規作成のviewが最新でなくてもdocEpochが進む(表示リセットの合図)", async () => {
    const slowNew = deferred<DocumentView>();
    vi.mocked(ipc.documentNew).mockReturnValueOnce(slowNew.promise);
    vi.mocked(ipc.editApply).mockResolvedValueOnce(makeView(300));

    const store = useAppStore.getState();
    const p1 = store.newDocument({ width_mm: 150, height_mm: 150 });
    const p2 = store.applyEdit({ type: "RemoveEdges", ids: [10] });
    slowNew.resolve(makeView(100));
    await Promise.all([p1, p2]);

    const s = useAppStore.getState();
    expect(s.docEpoch).toBe(1); // 新規作成の適用は破棄されずepochが進む
    expect(s.doc?.cp.next_edge_id).toBe(300); // その後に編集結果が上書き
  });

  it("Document変更後は古い保存希望を捨て、再解決済みの辺ID順だけを持つ", async () => {
    useAppStore.setState({ sequenceTargets: new Map([[999, 120]]) });
    const view = makeView(301);
    view.sequence_targets = [
      { hinge: 9, target_angle_deg: 30 },
      { hinge: 5, target_angle_deg: 60 },
    ];
    vi.mocked(ipc.documentNew).mockResolvedValueOnce(view);

    await useAppStore.getState().newDocument({ width_mm: 150, height_mm: 150 });

    expect([...useAppStore.getState().sequenceTargets]).toEqual([
      [5, 60],
      [9, 30],
    ]);
    expect(useAppStore.getState().sequenceTargets.has(999)).toBe(false);
  });

  it("最新でない失敗は報告しない(直後に新しい結果が必ず続くため)", async () => {
    const slowFail = deferred<DocumentView>();
    vi.mocked(ipc.editApply)
      .mockReturnValueOnce(slowFail.promise)
      .mockResolvedValueOnce(makeView(400));

    const store = useAppStore.getState();
    const pA = store.applyEdit({ type: "RemoveEdges", ids: [10] });
    const pB = store.applyEdit({ type: "RemoveEdges", ids: [11] });
    // Aを失敗させる(この時点でBが積まれているのでAはisLatest=false)
    slowFail.reject("古い失敗");
    await Promise.all([pA, pB]);

    const s = useAppStore.getState();
    expect(s.doc?.cp.next_edge_id).toBe(400); // Bの成功が反映される
    expect(s.errorMessage).toBeNull(); // 古い失敗は報告されない
  });
});

/** 次のマクロタスクまで待ち、溜まっているPromiseの続きを流し切る */
function flush(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

/** 偽タイマーを進めず、Promiseの応答反映だけを流す。 */
async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 8; i++) await Promise.resolve();
}

/** 偽タイマーを使うテストの準備。
 * 間引きの基準時刻はbeforeEachのresetPoseThrottle()で初期化済みなので、
 * ここでは偽タイマーへ切り替えるだけでよい(テストの並び順に依存しない)。 */
function primeFakeTimers(): void {
  vi.useFakeTimers();
}

describe("appStore 折り角度の指定", () => {
  it("pose_solveへ現在の手順位置と途中進行度を渡す", async () => {
    seedSequence(3, 2);
    useAppStore.setState({ playT: 0.4 });

    useAppStore.getState().setDriverAngle(5, 75);
    await vi.waitFor(() => expect(ipc.poseSolve).toHaveBeenCalledTimes(1));

    const call = vi.mocked(ipc.poseSolve).mock.calls[0];
    expect(call[4]).toBe(2);
    expect(call[5]).toBe(0.4);
  });

  it("±180度で4点を知らせても角度を反映し、離した後も次の最新結果まで残す", async () => {
    const view = makeHingeView(444);
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
      hinges: new Set([5]),
    });
    vi.mocked(ipc.poseSolve)
      .mockResolvedValueOnce({
        ...makeSolveResult({ "5": 104.75 }),
        frame: { faces: [], warnings: ["180度へ動かした形"] },
        flat_fold_violations: [9, 10, 11, 12],
      })
      .mockResolvedValueOnce({
        ...makeSolveResult({ "5": 90 }),
        frame: { faces: [], warnings: ["次の形"] },
        flat_fold_violations: [],
      });

    useAppStore.getState().setDriverAngle(5, 180);
    await vi.waitFor(() =>
      expect(useAppStore.getState().flatFoldViolations).toEqual([9, 10, 11, 12]),
    );

    let state = useAppStore.getState();
    expect(state.drivers.get(5)).toBe(180);
    expect(state.poseAngles.get(5)).toBe(104.75);
    expect(state.frame3d?.warnings).toEqual(["180度へ動かした形"]);
    expect(state.errorMessage).toBeNull();

    await state.finishAngleIntent();
    expect(useAppStore.getState().activeAngleIntent).toBeNull();
    expect(useAppStore.getState().flatFoldViolations).toEqual([9, 10, 11, 12]);

    useAppStore.getState().setDriverAngle(5, 90);
    await vi.waitFor(() =>
      expect(useAppStore.getState().flatFoldViolations).toEqual([]),
    );
    state = useAppStore.getState();
    expect(state.drivers.get(5)).toBe(90);
    expect(state.poseAngles.get(5)).toBe(90);
    expect(state.frame3d?.warnings).toEqual(["次の形"]);
    expect(state.errorMessage).toBeNull();
  });

  it("接触を検出しても希望角を書き戻さず、警告状態だけを覚える", async () => {
    const view = makeHingeView(445);
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
      hinges: new Set([5]),
    });
    vi.mocked(ipc.poseSolve).mockResolvedValueOnce({
      ...makeSolveResult({ "5": 104.75 }),
      contact_detected: true,
    });

    useAppStore.getState().setDriverAngle(5, 110);
    await vi.waitFor(() => expect(useAppStore.getState().contactDetected).toBe(true));

    const state = useAppStore.getState();
    expect(state.drivers.get(5)).toBe(110);
    expect(state.poseAngles.get(5)).toBe(104.75);
    // 110度と指定したのに104.75度になったので、その差を利用者へ知らせる
    expect(state.poseWarnings).toHaveLength(1);
    expect(state.poseWarnings[0]).toContain("折り目 #5");
    expect(state.poseWarnings[0]).toContain("110.0°");
    expect(state.poseWarnings[0]).toContain("104.8°");
    expect(vi.mocked(ipc.poseSolve).mock.calls[0][3]).toEqual([]);
  });

  it("実際に待っても、連続変更は1回にまとまり最後の角度が送られる", async () => {
    const store = useAppStore.getState();
    store.setDriverAngle(5, 10);
    store.setDriverAngle(5, 20);
    store.setDriverAngle(5, 30);

    await vi.waitFor(
      () => expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: 30 }]]),
      { timeout: 2000, interval: 10 },
    );
    // その後しばらく待っても、間引かれた分が追加で飛ぶことはない
    await new Promise((resolve) => setTimeout(resolve, REAL_WAIT_MS));
    expect(poseCalls()).toHaveLength(1);
  });

  it("連続操作は16msで間引かれ、最後の角度が必ず送られる", async () => {
    primeFakeTimers();
    try {
      const store = useAppStore.getState();
      store.setDriverAngle(5, 10);
      store.setDriverAngle(5, 20);
      store.setDriverAngle(5, 30);
      expect(ipc.poseSolve).not.toHaveBeenCalled(); // まだ送っていない
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);
      expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: 30 }]]);
      expect(useAppStore.getState().drivers.get(5)).toBe(30);
    } finally {
      vi.useRealTimers();
    }
  });

  it("間隔を空けた変更はまとめられず、それぞれ送られる", async () => {
    primeFakeTimers();
    try {
      const store = useAppStore.getState();
      store.setDriverAngle(5, 10);
      await vi.advanceTimersByTimeAsync(70);
      store.setDriverAngle(5, 20);
      await vi.advanceTimersByTimeAsync(70);
      expect(poseCalls()).toEqual([
        [{ hinge: 5, target_angle_deg: 10 }],
        [{ hinge: 5, target_angle_deg: 20 }],
      ]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("計算中に角度が変わり続けても、待たせるのは最新の1件だけ", async () => {
    primeFakeTimers();
    try {
      const slow = deferred<SolveResult>();
      vi.mocked(ipc.poseSolve).mockReturnValueOnce(slow.promise);
      const store = useAppStore.getState();

      store.setDriverAngle(5, 10);
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS); // 1件目が実行中になる
      expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: 10 }]]);

      for (const deg of [20, 30, 40]) {
        store.setDriverAngle(5, deg);
        await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);
      }
      expect(poseCalls()).toHaveLength(1); // 1件目が終わるまで次は送られない

      slow.resolve(makeSolveResult());
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);

      // 待っていた3件は最新の1件にまとまる(20と30は送られない)
      expect(poseCalls()).toEqual([
        [{ hinge: 5, target_angle_deg: 10 }],
        [{ hinge: 5, target_angle_deg: 40 }],
      ]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("操作終了は予約中の末尾solveをflushし、完了後にactiveだけを解除する", async () => {
    const solving = deferred<SolveResult>();
    vi.mocked(ipc.poseSolve).mockReturnValueOnce(solving.promise);
    const store = useAppStore.getState();
    store.setDriverAngle(5, 72);
    const generation = useAppStore.getState().activeAngleIntent?.generation;

    const finishing = store.finishAngleIntent();
    await flush();
    expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: 72 }]]);
    expect(useAppStore.getState().activeAngleIntent?.generation).toBe(generation);

    solving.resolve(makeSolveResult({ "5": 72 }));
    await finishing;
    expect(useAppStore.getState().activeAngleIntent).toBeNull();
    expect(useAppStore.getState().drivers.get(5)).toBe(72);
  });

  it("次の16ms要求が未送信でも、新しい操作世代は旧solveの表示を破棄する", async () => {
    primeFakeTimers();
    try {
      const oldSolve = deferred<SolveResult>();
      const latestFrame = { faces: [], warnings: ["最新の形"] };
      vi.mocked(ipc.poseSolve)
        .mockReturnValueOnce(oldSolve.promise)
        .mockResolvedValueOnce({
          ...makeSolveResult({ "5": 20 }),
          frame: latestFrame,
          relaxations: [
            { hinge: 7, target_angle_deg: 45, actual_angle_deg: 40, delta_deg: -5 },
          ],
        });
      const view = makeHingeView(446);
      const beforeFrame = { faces: [], warnings: ["表示中の形"] };
      useAppStore.setState({
        doc: view.doc,
        faces: view.faces,
        hinges: new Set([5]),
        frame3d: beforeFrame,
        poseAngles: new Map([[5, 1]]),
        relaxations: [],
      });

      const store = useAppStore.getState();
      store.setDriverAngle(5, 10);
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS); // 旧solveを実行中にする
      store.setDriverAngle(5, 20); // 新要求はまだthrottle待ち

      oldSolve.resolve({
        ...makeSolveResult({ "5": 10 }),
        frame: { faces: [], warnings: ["旧世代の形"] },
        relaxations: [
          { hinge: 5, target_angle_deg: 10, actual_angle_deg: 8, delta_deg: -2 },
        ],
      });
      await flushMicrotasks();

      expect(poseCalls()).toHaveLength(1);
      expect(useAppStore.getState().frame3d).toBe(beforeFrame);
      expect(useAppStore.getState().poseAngles.get(5)).toBe(1);
      expect(useAppStore.getState().relaxations).toEqual([]);

      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);
      expect(poseCalls()).toHaveLength(2);
      expect(useAppStore.getState().frame3d).toBe(latestFrame);
      expect(useAppStore.getState().poseAngles.get(5)).toBe(20);
      expect(useAppStore.getState().relaxations[0].hinge).toBe(7);
    } finally {
      vi.useRealTimers();
    }
  });

  it("不収束中の連続100入力でも、最後の要求角と最良候補が表示される", async () => {
    primeFakeTimers();
    try {
      const slow = deferred<SolveResult>();
      const finalFrame = { faces: [], warnings: ["いちばん近い形"] };
      vi.mocked(ipc.poseSolve)
        .mockReturnValueOnce(slow.promise)
        .mockResolvedValueOnce({
          ...makeSolveResult({ "5": 100 }),
          converged: false,
          best_effort: true,
          frame: finalFrame,
        });
      const store = useAppStore.getState();

      store.setDriverAngle(5, 1);
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);
      for (let deg = 2; deg <= 100; deg++) {
        store.setDriverAngle(5, deg);
        await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);
      }
      expect(poseCalls()).toHaveLength(1);

      slow.resolve({
        ...makeSolveResult({ "5": 1 }),
        converged: false,
        best_effort: true,
      });
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);

      expect(poseCalls()).toEqual([
        [{ hinge: 5, target_angle_deg: 1 }],
        [{ hinge: 5, target_angle_deg: 100 }],
      ]);
      expect(useAppStore.getState().drivers.get(5)).toBe(100);
      expect(useAppStore.getState().frame3d).toEqual(finalFrame);
      expect(useAppStore.getState().poseBestEffort).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it("複数の折り線を同じ角度にして1回で送り、対象外の指定は保ちたい目標にする", async () => {
    primeFakeTimers();
    try {
      const view = makeHingeView(460);
      useAppStore.setState({
        doc: view.doc,
        faces: view.faces,
        hinges: new Set([5, 7, 9]),
        drivers: new Map([[9, 25]]),
      });

      // 重複と存在しない辺を含めても、有効な選択だけを番号順に1回へまとめる。
      useAppStore.getState().setDriverAngles([7, 5, 7, 999], 60);
      expect(ipc.poseSolve).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);

      // 固定するのは代表の1本だけ。残りは同じ角度の希望として送る
      // (全部を固定すると実際の紙では成り立たず、紙が閉じなくなる)。
      expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: 60 }]]);
      expect(poseKeeps()).toEqual([
        [
          { hinge: 7, target_angle_deg: 60 },
          { hinge: 9, target_angle_deg: 25 },
        ],
      ]);
      expect([...useAppStore.getState().drivers]).toEqual([
        [9, 25],
        [5, 60],
        [7, 60],
      ]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("まとめて動かすと、選んだ折り目は全部が動かし中(3Dで水色)になる", () => {
    const view = makeHingeView(462);
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
      hinges: new Set([5, 7, 9]),
      drivers: new Map(),
    });
    useAppStore.getState().setDriverAngles([9, 5, 7], 60);
    // 光るのは3本すべて。固定するのはそのうち1本だけ(splitDriversの役目)。
    expect(useAppStore.getState().activeAngleIntent?.hinges).toEqual([5, 7, 9]);
    expect(useAppStore.getState().activeAngleIntent?.fixAll).toBe(false);
  });

  it("選択を外すと、動かし中の水色も消える", () => {
    const view = makeHingeView(463);
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
      hinges: new Set([5, 7]),
      drivers: new Map(),
      selection: { edgeIds: [5, 7], vertexIds: [] },
    });
    useAppStore.getState().setDriverAngles([5, 7], 60);
    expect(useAppStore.getState().activeAngleIntent).not.toBeNull();

    // 実機で見つかった不具合: 角度を動かし終えても印が残り、何も選んでいないのに
    // 3Dの線が水色に光ったままになっていた。
    useAppStore.getState().setSelection({ edgeIds: [], vertexIds: [] });
    expect(useAppStore.getState().activeAngleIntent).toBeNull();
  });

  it("選んだままなら、動かし中の印は残る", () => {
    const view = makeHingeView(464);
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
      hinges: new Set([5, 7]),
      drivers: new Map(),
      selection: { edgeIds: [5, 7], vertexIds: [] },
    });
    useAppStore.getState().setDriverAngles([5, 7], 60);
    useAppStore.getState().setSelection({ edgeIds: [5], vertexIds: [] });
    expect(useAppStore.getState().activeAngleIntent?.hinges).toEqual([5, 7]);
  });

  it("複数折り線の連続変更も16ms間引きとrunLatestで最後だけを残し履歴は1件にする", async () => {
    primeFakeTimers();
    try {
      const slow = deferred<SolveResult>();
      vi.mocked(ipc.poseSolve).mockReturnValueOnce(slow.promise);
      const view = makeHingeView(470);
      useAppStore.setState({
        doc: view.doc,
        faces: view.faces,
        hinges: new Set([5, 7, 9, 11]),
        drivers: new Map([[9, 25]]),
        angleUndoStack: [],
        angleRedoStack: [new Map([[9, 10]])],
      });
      const store = useAppStore.getState();
      const selected = [11, 7, 5, 7, 999];

      store.setDriverAngles(selected, 10);
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS); // 1件目が実行中になる
      expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: 10 }]]);
      expect(poseKeeps()).toEqual([
        [
          { hinge: 7, target_angle_deg: 10 },
          { hinge: 9, target_angle_deg: 25 },
          { hinge: 11, target_angle_deg: 10 },
        ],
      ]);

      for (const deg of [20, 30, 40]) {
        store.setDriverAngles(selected, deg);
        await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);
      }
      expect(poseCalls()).toHaveLength(1); // 実行中は中間の20・30度を送らない

      // 同じ選択組のスライダー操作は、最初の状態だけを1件の履歴に残す。
      const during = useAppStore.getState();
      expect(during.angleUndoStack).toHaveLength(1);
      expect([...during.angleUndoStack[0]]).toEqual([[9, 25]]);
      expect(during.angleRedoStack).toEqual([]);

      slow.resolve(makeSolveResult());
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);

      expect(poseCalls()).toEqual([
        [{ hinge: 5, target_angle_deg: 10 }],
        [{ hinge: 5, target_angle_deg: 40 }],
      ]);
      expect(poseKeeps()).toEqual([
        [
          { hinge: 7, target_angle_deg: 10 },
          { hinge: 9, target_angle_deg: 25 },
          { hinge: 11, target_angle_deg: 10 },
        ],
        [
          { hinge: 7, target_angle_deg: 40 },
          { hinge: 9, target_angle_deg: 25 },
          { hinge: 11, target_angle_deg: 40 },
        ],
      ]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("追い越された追従計算の成功結果は3D表示へ反映しない", async () => {
    const first = deferred<SolveResult>();
    const second = deferred<SolveResult>();
    vi.mocked(ipc.poseSolve)
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    const view = makeHingeView(450);
    const before = { faces: [], warnings: ["表示中の形"] };
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
      hinges: new Set([5]),
      frame3d: before,
      suspectHinges: [99],
      relaxations: [
        {
          hinge: 99,
          target_angle_deg: 90,
          actual_angle_deg: 80,
          delta_deg: -10,
        },
      ],
      poseBestEffort: false,
    });

    const store = useAppStore.getState();
    store.clearDrivers();
    await flush();
    store.clearDrivers();
    await flush();
    first.resolve({
      ...makeSolveResult(),
      frame: { faces: [], warnings: ["古い形"] },
      suspect_hinges: [5],
      relaxations: [
        { hinge: 5, target_angle_deg: 90, actual_angle_deg: 70, delta_deg: -20 },
      ],
      best_effort: true,
    });
    await flush();
    expect(useAppStore.getState().frame3d).toEqual(before);
    expect(useAppStore.getState().suspectHinges).toEqual([99]);
    expect(useAppStore.getState().relaxations[0].hinge).toBe(99);
    expect(useAppStore.getState().poseBestEffort).toBe(false);

    const latest = { faces: [], warnings: ["新しい形"] };
    second.resolve({
      ...makeSolveResult(),
      frame: latest,
      suspect_hinges: [7],
      relaxations: [
        { hinge: 7, target_angle_deg: 90, actual_angle_deg: 72, delta_deg: -18 },
      ],
      best_effort: true,
    });
    await flush();
    expect(useAppStore.getState().frame3d).toEqual(latest);
    expect(useAppStore.getState().suspectHinges).toEqual([7]);
    expect(useAppStore.getState().relaxations[0].hinge).toBe(7);
    expect(useAppStore.getState().poseBestEffort).toBe(true);
  });

  it("追従計算の原因候補を反映し、次の空応答で消す", async () => {
    const view = makeHingeView(451);
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
      hinges: new Set([5, 7]),
    });
    vi.mocked(ipc.poseSolve)
      .mockResolvedValueOnce({ ...makeSolveResult(), suspect_hinges: [5, 7] })
      .mockResolvedValueOnce({ ...makeSolveResult(), suspect_hinges: [] });

    useAppStore.getState().clearDrivers();
    await flush();
    expect(useAppStore.getState().suspectHinges).toEqual([5, 7]);

    useAppStore.getState().clearDrivers();
    await flush();
    expect(useAppStore.getState().suspectHinges).toEqual([]);
  });

  it("角度を次々に指定しても、固定するのは操作中の1本だけ(紙が切れない)", async () => {
    primeFakeTimers();
    try {
      const store = useAppStore.getState();
      // 内部頂点のまわりでは折り角どうしに拘束があるので、指定済みを全部
      // 固定すると形が閉じず面が離れる。以前の指定は「保ちたい目標」で送る
      store.setDriverAngle(5, 30);
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);
      store.setDriverAngle(7, 60);
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);
      store.setDriverAngle(9, 90);
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);

      expect(poseCalls()).toEqual([
        [{ hinge: 5, target_angle_deg: 30 }],
        [{ hinge: 7, target_angle_deg: 60 }],
        [{ hinge: 9, target_angle_deg: 90 }],
      ]);
      expect(poseKeeps()).toEqual([
        [],
        [{ hinge: 5, target_angle_deg: 30 }],
        [
          { hinge: 5, target_angle_deg: 30 },
          { hinge: 7, target_angle_deg: 60 },
        ],
      ]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("「全て平らに戻す」は全ての折り線へ0度を指定して送る", async () => {
    const view = makeHingeView(500);
    useAppStore.setState({ doc: view.doc, faces: view.faces, hinges: new Set([5]) });

    const store = useAppStore.getState();
    store.setDriverAngle(5, 90);
    store.clearDrivers();
    await flush();

    // 間引き中の分は送られず、平ら指定だけが1回送られる
    expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: 0 }]]);
    expect(useAppStore.getState().drivers.size).toBe(0);
  });

  it("1本だけ解除したときは、その折り線に0度を明示して1回送る", async () => {
    const view = makeHingeView(800);
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
      hinges: new Set([5]),
      drivers: new Map([
        [5, 90],
        [7, 30],
      ]),
    });

    useAppStore.getState().clearDriver(5);
    await flush();

    // 解除した5は0度(平ら)を固定して明示。残りの7は「保ちたい目標」として送る
    // (7まで固定すると内部頂点まわりの拘束と両立せず紙が切れて見える)
    expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: 0 }]]);
    expect(poseKeeps()).toEqual([[{ hinge: 7, target_angle_deg: 30 }]]);
    expect([...useAppStore.getState().drivers]).toEqual([[7, 30]]);
  });

  it("展開図を編集すると残った指定で計算し直し、折り線でなくなった指定は捨てる", async () => {
    const view = makeHingeView(600);
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
      hinges: new Set([5]),
      drivers: new Map([
        [5, 90],
        [9, 45], // この辺は編集後のviewに存在しない
      ]),
    });
    vi.mocked(ipc.editApply).mockResolvedValueOnce(makeHingeView(601));

    await useAppStore.getState().applyEdit({ type: "RemoveEdges", ids: [9] });

    // 操作中の折り線が無いので、残った指定は全て「保ちたい目標」として送る
    expect(poseCalls()).toEqual([[]]);
    expect(poseKeeps()).toEqual([[{ hinge: 5, target_angle_deg: 90 }]]);
    expect([...useAppStore.getState().drivers]).toEqual([[5, 90]]);
  });

  it("編集で角度指定が全て無くなったら、全ての折り線へ0度を送って平らに戻す", async () => {
    const view = makeHingeView(900);
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
      hinges: new Set([5]),
      drivers: new Map([[9, 45]]), // 編集後のviewには残らない辺
      frame3d: { faces: [], warnings: [] }, // すでに折った状態
    });
    vi.mocked(ipc.editApply).mockResolvedValueOnce(makeHingeView(901));

    await useAppStore.getState().applyEdit({
      type: "SetEdgeKind",
      ids: [9],
      kind: "Aux",
    });

    // 空のまま送ると前回の計算結果を引き継いで折れたまま残るため0度を明示する
    expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: 0 }]]);
    expect(useAppStore.getState().drivers.size).toBe(0);
  });

  it("既に折り切ってある折り目は、譲れる希望ではなく厳密に保つ側へ回す", async () => {
    const view = makeHingeView(446);
    view.doc.cp.edges.push({ id: 6, v0: 1, v1: 3, kind: "Valley" });
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
      hinges: new Set([5, 6]),
      // 辺5は180度まで折り切ってある。辺6は途中の角度。
      sequenceTargets: new Map([
        [5, 180],
        [6, -178.265],
      ]),
    });

    useAppStore.getState().setDriverAngle(6, -35);
    await vi.waitFor(() => expect(poseCalls().length).toBeGreaterThan(0));

    // 折り切った辺5は「厳密に保つ」側、折り切っていない辺6の元の希望は残らない
    // (辺6はいま動かしているので厳密側)。
    expect(poseCalls()[0].map((d) => d.hinge).sort()).toEqual([5, 6]);
    expect(poseKeeps()[0]).toEqual([]);
  });

  it("折り切った折り目まで保つと解けない形では、操作を止めず希望へ戻して解き直す", async () => {
    const view = makeHingeView(447);
    view.doc.cp.edges.push({ id: 6, v0: 1, v1: 3, kind: "Valley" });
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
      hinges: new Set([5, 6]),
      sequenceTargets: new Map([[5, 180]]),
    });
    // 1回目(辺5も厳密)は収束しない。2回目(辺5は希望へ戻す)は収束する。
    vi.mocked(ipc.poseSolve)
      .mockResolvedValueOnce({
        ...makeSolveResult({ "5": 180, "6": -35 }),
        converged: false,
      })
      .mockResolvedValueOnce(makeSolveResult({ "5": 170, "6": -35 }));

    useAppStore.getState().setDriverAngle(6, -35);
    await vi.waitFor(() => expect(poseCalls().length).toBe(2));

    // 1回目は辺5と辺6を厳密に、2回目は辺6だけを厳密にして辺5を希望へ戻す
    expect(poseCalls()[0].map((d) => d.hinge).sort()).toEqual([5, 6]);
    expect(poseCalls()[1].map((d) => d.hinge)).toEqual([6]);
    expect(poseKeeps()[1].map((d) => d.hinge)).toEqual([5]);
    // 操作は止まらず、2回目の結果が表示に入る
    const s = useAppStore.getState();
    expect(s.poseConverged).toBe(true);
    expect(s.poseAngles.get(5)).toBe(170);
    // 指定どおりにならなかったことは利用者へ知らせる
    expect(s.poseWarnings.some((w) => w.includes("折り目 #5"))).toBe(true);
  });

  it("収束しなかった結果は警告と収束フラグに反映される(角度指定)", async () => {
    const result = makeSolveResult({ "5": 90 });
    result.converged = false;
    result.frame.warnings = ["追従計算が収束していません"];
    vi.mocked(ipc.poseSolve).mockResolvedValueOnce(result);
    const view = makeHingeView(700);
    useAppStore.setState({ doc: view.doc, faces: view.faces, hinges: new Set([5]) });

    useAppStore.getState().clearDrivers();
    await flush();

    const s = useAppStore.getState();
    expect(s.poseConverged).toBe(false);
    // 収束しない旨に加えて、0度と指定した折り目が90度のままであることも知らせる
    expect(s.poseWarnings[0]).toBe("追従計算が収束していません");
    expect(s.poseWarnings).toHaveLength(2);
    expect(s.poseWarnings[1]).toContain("折り目 #5");
    expect(s.poseAngles.get(5)).toBe(90);
    expect(s.errorMessage).toBeNull(); // 追従計算の警告はエラーにしない
  });
});

describe("appStore 手順の表示と再生", () => {
  it("4点を知らせても手順を再生し、次の最新再生結果が空なら解除する", async () => {
    seedSequence(3);
    vi.mocked(ipc.sequenceReplay)
      .mockResolvedValueOnce({
        ...makeReplayResult(),
        frame: { faces: [], warnings: ["手順2の形"] },
        flat_fold_violations: [9, 10, 11, 12],
      })
      .mockResolvedValueOnce({
        ...makeReplayResult(),
        frame: { faces: [], warnings: ["手順1の形"] },
        flat_fold_violations: [],
      });

    useAppStore.getState().selectStep(2);
    await vi.waitFor(() =>
      expect(useAppStore.getState().flatFoldViolations).toEqual([9, 10, 11, 12]),
    );

    let state = useAppStore.getState();
    expect(state.currentStep).toBe(2);
    expect(state.frame3d?.warnings).toEqual(["手順2の形"]);
    expect(state.errorMessage).toBeNull();

    state.selectStep(1);
    await vi.waitFor(() =>
      expect(useAppStore.getState().flatFoldViolations).toEqual([]),
    );
    state = useAppStore.getState();
    expect(state.currentStep).toBe(1);
    expect(state.frame3d?.warnings).toEqual(["手順1の形"]);
    expect(state.errorMessage).toBeNull();
  });

  it("手順を選ぶと、その手順まで折った形を表示する", async () => {
    seedSequence(3);
    vi.mocked(ipc.sequenceReplay).mockResolvedValueOnce({
      ...makeReplayResult(),
      skipped: [2],
      warnings: ["手順2の折り線が見つからないため、この手順を飛ばしました"],
    });

    useAppStore.getState().selectStep(2);
    await flush();

    expect(replayCalls()).toEqual([[2, 1]]);
    const s = useAppStore.getState();
    expect(s.currentStep).toBe(2);
    expect(s.replaySkipped).toEqual([2]);
    expect(s.replayWarnings).toHaveLength(1);
  });

  it("途中の手順を選んでも、その先の手順の飛ばした印は消えない", async () => {
    seedSequence(3);
    // 作品全体の再生では手順3が飛ばされている(DocumentView由来)
    useAppStore.setState({ skipped: [3] });
    // 手順1までの再生からは、手順3のことは分からない
    vi.mocked(ipc.sequenceReplay).mockResolvedValueOnce(makeReplayResult());

    useAppStore.getState().selectStep(1);
    await flush();

    const s = useAppStore.getState();
    expect(s.skipped).toEqual([3]); // タイムラインの赤表示は保たれる
    expect(s.replaySkipped).toEqual([]);
    expect(isStepSkipped(s, 3)).toBe(true);
  });

  it("最新を選ぶと全ての手順まで折った形を表示する", async () => {
    seedSequence(3, 1);

    useAppStore.getState().selectStep(null);
    await flush();

    expect(replayCalls()).toEqual([[3, 1]]);
    expect(useAppStore.getState().currentStep).toBeNull();
  });

  it("コマ送りは0(折る前)から手順数までの範囲に収まる", async () => {
    seedSequence(2);

    // 最新表示からの「前へ」は最終手順を基準に1つ戻る
    useAppStore.getState().stepBy(-1);
    await flush();
    expect(useAppStore.getState().currentStep).toBe(1);

    useAppStore.getState().stepBy(-1); // 0(折る前)
    useAppStore.getState().stepBy(-1); // これ以上は戻らない
    await flush();
    expect(useAppStore.getState().currentStep).toBe(0);

    useAppStore.getState().stepBy(1);
    useAppStore.getState().stepBy(1);
    useAppStore.getState().stepBy(1); // 手順数を超えない
    await flush();
    expect(useAppStore.getState().currentStep).toBe(2);
    // 端で行き止まりになったぶん(0での「前へ」・2での「次へ」)は同じ形なので
    // 描き直しを頼まない
    expect(replayCalls()).toEqual([
      [1, 1],
      [0, 1],
      [1, 1],
      [2, 1],
    ]);
  });

  it("再生は手順を順に進み、最終手順で止まる", async () => {
    primeFakeTimers();
    try {
      seedSequence(2);
      useAppStore.getState().togglePlay();
      expect(useAppStore.getState().playing).toBe(true);

      // 1手順目の途中(320msの半分)では、まだ1手順目を補間している
      await vi.advanceTimersByTimeAsync(STEP_DURATION_MS / 2);
      const mid = lastReplayCall();
      expect(mid?.[0]).toBe(1);
      expect(mid?.[1]).toBeGreaterThan(0);
      expect(mid?.[1]).toBeLessThan(1);

      // 2手順ぶん進めれば最後まで折り終えて止まる
      await vi.advanceTimersByTimeAsync(STEP_DURATION_MS * 3);
      const s = useAppStore.getState();
      expect(s.playing).toBe(false);
      expect(s.currentStep).toBe(2);
      expect(lastReplayCall()).toEqual([2, 1]);

      // 止まった後は描き直しを頼まない
      const count = replayCalls().length;
      await vi.advanceTimersByTimeAsync(STEP_DURATION_MS);
      expect(replayCalls()).toHaveLength(count);
    } finally {
      vi.useRealTimers();
    }
  });

  it("再生中に計算が追いつかなくても、待たせるのは最新の1件だけ", async () => {
    primeFakeTimers();
    try {
      const total = 3;
      seedSequence(total);
      const slow = deferred<ReplayResult>();
      vi.mocked(ipc.sequenceReplay).mockReturnValueOnce(slow.promise);

      useAppStore.getState().togglePlay();
      // 1件目が返らないまま、最後の手順まで再生しきる(60コマ以上進む)
      await vi.advanceTimersByTimeAsync(STEP_DURATION_MS * (total + 1));
      expect(replayCalls()).toHaveLength(1); // 1件目が終わるまで次は送らない
      expect(useAppStore.getState().playing).toBe(false); // 最後まで進んで停止

      slow.resolve(makeReplayResult());
      await vi.advanceTimersByTimeAsync(32);

      // 待っていた数十コマぶんは最新の1件にまとまり、最後の形だけが送られる
      expect(replayCalls()).toHaveLength(2);
      expect(lastReplayCall()).toEqual([total, 1]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("再生中に「全て平らに戻す」を押したら、後続の再生結果で折れ直さない", async () => {
    primeFakeTimers();
    try {
      const slowReplay = deferred<ReplayResult>();
      const foldedFrame = { faces: [], warnings: ["再生で折れた形"] };
      const flatFrame = { faces: [], warnings: ["0度で平らな形"] };
      vi.mocked(ipc.sequenceReplay)
        .mockReturnValueOnce(slowReplay.promise)
        .mockResolvedValue({
          frame: foldedFrame,
          skipped: [],
          warnings: [],
        });
      vi.mocked(ipc.poseSolve).mockResolvedValue({
        ...makeSolveResult({ "5": 0 }),
        frame: flatFrame,
      });
      seedSequence(2);
      useAppStore.setState({
        drivers: new Map([[5, 90]]),
        frame3d: foldedFrame,
      });

      useAppStore.getState().togglePlay();
      await vi.advanceTimersByTimeAsync(32);
      expect(replayCalls()).toHaveLength(1);

      // 実機でボタンを押した順序を再現する。0度計算が待っている間にも
      // 再生tickが続くと、正しい平坦結果が古い応答として捨てられてしまう。
      useAppStore.getState().clearDrivers();
      await vi.advanceTimersByTimeAsync(32);
      slowReplay.resolve({
        frame: foldedFrame,
        skipped: [],
        warnings: [],
      });
      await vi.runAllTimersAsync();

      expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: 0 }]]);
      expect(useAppStore.getState().drivers.size).toBe(0);
      expect(useAppStore.getState().playing).toBe(false);
      expect(useAppStore.getState().frame3d).toEqual(flatFrame);
    } finally {
      vi.useRealTimers();
    }
  });

  it("追い越された再生の成功結果は3D表示へ反映しない", async () => {
    const first = deferred<ReplayResult>();
    const second = deferred<ReplayResult>();
    vi.mocked(ipc.sequenceReplay)
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    const before = { faces: [], warnings: ["表示中の形"] };
    seedSequence(2);
    useAppStore.setState({
      frame3d: before,
      suspectHinges: [99],
      sequenceTargets: new Map([[99, 12]]),
      poseAngles: new Map([[99, 11]]),
      relaxations: [
        { hinge: 99, target_angle_deg: 12, actual_angle_deg: 11, delta_deg: -1 },
      ],
      poseBestEffort: false,
      poseClosureRms: 0.01,
      contactDetected: false,
    });

    useAppStore.getState().selectStep(1);
    await flush();
    useAppStore.getState().selectStep(2);
    await flush();
    first.resolve({
      frame: { faces: [], warnings: ["古い形"] },
      skipped: [1],
      warnings: [],
      suspect_hinges: [5],
      sequence_targets: [{ hinge: 5, target_angle_deg: 90 }],
      angles: { "5": 70 },
      relaxations: [
        { hinge: 5, target_angle_deg: 90, actual_angle_deg: 70, delta_deg: -20 },
      ],
      best_effort: true,
      closure_rms: 2,
      contact_detected: true,
    });
    await flush();
    expect(useAppStore.getState().frame3d).toEqual(before);
    expect(useAppStore.getState().suspectHinges).toEqual([99]);
    expect([...useAppStore.getState().sequenceTargets]).toEqual([[99, 12]]);
    expect([...useAppStore.getState().poseAngles]).toEqual([[99, 11]]);
    expect(useAppStore.getState().relaxations[0].hinge).toBe(99);
    expect(useAppStore.getState().poseBestEffort).toBe(false);
    expect(useAppStore.getState().poseClosureRms).toBe(0.01);
    expect(useAppStore.getState().contactDetected).toBe(false);

    const latest = { faces: [], warnings: ["新しい形"] };
    second.resolve({
      frame: latest,
      skipped: [2],
      warnings: [],
      suspect_hinges: [7],
      sequence_targets: [{ hinge: 7, target_angle_deg: 45 }],
      angles: { "7": 40 },
      relaxations: [
        { hinge: 7, target_angle_deg: 45, actual_angle_deg: 40, delta_deg: -5 },
      ],
      best_effort: true,
      closure_rms: 0.5,
      contact_detected: true,
    });
    await flush();
    expect(useAppStore.getState().frame3d).toEqual(latest);
    expect(useAppStore.getState().suspectHinges).toEqual([7]);
    expect([...useAppStore.getState().sequenceTargets]).toEqual([[7, 45]]);
    expect([...useAppStore.getState().poseAngles]).toEqual([[7, 40]]);
    expect(useAppStore.getState().relaxations[0].hinge).toBe(7);
    expect(useAppStore.getState().poseBestEffort).toBe(true);
    expect(useAppStore.getState().poseClosureRms).toBe(0.5);
    expect(useAppStore.getState().contactDetected).toBe(true);
  });

  it("角度操作の次要求がthrottle待ちでも、旧再生結果を表示しない", async () => {
    primeFakeTimers();
    try {
      const oldReplay = deferred<ReplayResult>();
      vi.mocked(ipc.sequenceReplay).mockReturnValueOnce(oldReplay.promise);
      const before = { faces: [], warnings: ["角度操作前の表示"] };
      seedSequence(2);
      useAppStore.setState({
        frame3d: before,
        sequenceTargets: new Map([[99, 12]]),
        poseAngles: new Map([[99, 11]]),
        relaxations: [
          { hinge: 99, target_angle_deg: 12, actual_angle_deg: 11, delta_deg: -1 },
        ],
      });

      useAppStore.getState().selectStep(1);
      await flushMicrotasks();
      expect(replayCalls()).toEqual([[1, 1]]);
      useAppStore.getState().setDriverAngle(5, 30); // pose要求はまだ未送信

      oldReplay.resolve({
        frame: { faces: [], warnings: ["旧再生"] },
        skipped: [1],
        warnings: ["旧警告"],
        sequence_targets: [{ hinge: 5, target_angle_deg: 90 }],
        angles: { "5": 90 },
        relaxations: [
          { hinge: 5, target_angle_deg: 90, actual_angle_deg: 70, delta_deg: -20 },
        ],
      });
      await flushMicrotasks();

      expect(poseCalls()).toHaveLength(0);
      expect(useAppStore.getState().frame3d).toBe(before);
      expect([...useAppStore.getState().sequenceTargets]).toEqual([[99, 12]]);
      expect([...useAppStore.getState().poseAngles]).toEqual([[99, 11]]);
      expect(useAppStore.getState().relaxations[0].hinge).toBe(99);

      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);
      expect(poseCalls()).toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("手順再生の原因候補を反映し、次の空応答で消す", async () => {
    seedSequence(2);
    vi.mocked(ipc.sequenceReplay)
      .mockResolvedValueOnce({ ...makeReplayResult(), suspect_hinges: [5, 7] })
      .mockResolvedValueOnce({ ...makeReplayResult(), suspect_hinges: [] });

    useAppStore.getState().selectStep(1);
    await flush();
    expect(useAppStore.getState().suspectHinges).toEqual([5, 7]);

    useAppStore.getState().selectStep(2);
    await flush();
    expect(useAppStore.getState().suspectHinges).toEqual([]);
  });

  it("再生中でも、飛ばした手順と警告の中身が同じなら配列を作り直さない", async () => {
    seedSequence(2);
    vi.mocked(ipc.sequenceReplay).mockResolvedValue({
      ...makeReplayResult(),
      skipped: [2],
      warnings: ["手順2の折り線が見つからないため、この手順を飛ばしました"],
    });

    useAppStore.getState().selectStep(1);
    await flush();
    const first = useAppStore.getState();

    useAppStore.getState().selectStep(2);
    await flush();
    const second = useAppStore.getState();

    // 内容が同じなら同じ配列を返す(毎コマの再描画を防ぐ)
    expect(second.replaySkipped).toBe(first.replaySkipped);
    expect(second.replayWarnings).toBe(first.replayWarnings);
  });

  it("再生中に手順を選ぶと再生は止まる", async () => {
    primeFakeTimers();
    try {
      seedSequence(3);
      useAppStore.getState().togglePlay();
      await vi.advanceTimersByTimeAsync(STEP_DURATION_MS / 2);

      useAppStore.getState().selectStep(1);
      expect(useAppStore.getState().playing).toBe(false);

      await vi.advanceTimersByTimeAsync(0); // 選んだ手順の描き直しを送り終える
      const count = replayCalls().length;
      await vi.advanceTimersByTimeAsync(STEP_DURATION_MS);
      expect(replayCalls()).toHaveLength(count); // 予約されていたコマも取り消す
      expect(useAppStore.getState().currentStep).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("再生に失敗したら止めて理由を知らせる", async () => {
    primeFakeTimers();
    try {
      seedSequence(3);
      vi.mocked(ipc.sequenceReplay).mockRejectedValue("再生に失敗しました");

      useAppStore.getState().togglePlay();
      await vi.advanceTimersByTimeAsync(64);

      const s = useAppStore.getState();
      expect(s.playing).toBe(false);
      expect(s.errorMessage).toBe("再生に失敗しました");
    } finally {
      vi.useRealTimers();
    }
  });

  it("最新表示中の編集は、viewに載っている自動再生の結果をそのまま使う", async () => {
    const view = makeStepView(2000, 2);
    view.frame = { faces: [], warnings: [] };
    view.skipped = [2];
    vi.mocked(ipc.editApply).mockResolvedValueOnce(view);
    useAppStore.setState({ hinges: new Set([5]), drivers: new Map([[5, 90]]) });

    await useAppStore.getState().applyEdit({ type: "RemoveEdges", ids: [9] });

    // 手順のある作品では、立体表示は角度指定ではなく手順の再生結果で作る
    expect(ipc.poseSolve).not.toHaveBeenCalled();
    // 自動再生済みの結果が載っているので、同じ内容を再生し直さない
    expect(replayCalls()).toEqual([]);
    const s = useAppStore.getState();
    expect(s.frame3d).toEqual(view.frame);
    expect(s.skipped).toEqual([2]);
    expect(s.replayWarnings).toEqual([]); // 警告はview.warnings側に入っている
  });

  it("途中の手順を表示中に編集したら、その手順まで再生し直す", async () => {
    seedSequence(3, 2);
    vi.mocked(ipc.editApply).mockResolvedValueOnce(makeStepView(2050, 3));
    vi.mocked(ipc.sequenceReplay).mockResolvedValueOnce({
      ...makeReplayResult(),
      skipped: [2],
      warnings: ["手順2の折り線が見つからないため、この手順を飛ばしました"],
    });

    await useAppStore.getState().applyEdit({ type: "RemoveEdges", ids: [9] });

    expect(replayCalls()).toEqual([[2, 1]]);
    const s = useAppStore.getState();
    expect(s.replaySkipped).toEqual([2]);
    expect(s.replayWarnings).toHaveLength(1);
  });

  it("一時停止中に展開図を編集しても、再生位置は折り終わりに揃う", async () => {
    // 手順2を途中(50%)まで折って一時停止している状態
    seedSequence(3, 2);
    useAppStore.setState({ playT: 0.5 });
    vi.mocked(ipc.editApply).mockResolvedValueOnce(makeStepView(2060, 3));

    await useAppStore.getState().applyEdit({ type: "RemoveEdges", ids: [9] });

    // 折り終わりの形(t=1)を描いたので、再生位置も折り終わりに合わせる。
    // 0.5のままだと、次に再生を押したとき表示が一度巻き戻ってしまう
    expect(replayCalls()).toEqual([[2, 1]]);
    expect(useAppStore.getState().playT).toBe(1);
  });

  it("再生中の編集・元に戻すは再生を止める", async () => {
    primeFakeTimers();
    try {
      seedSequence(3);
      vi.mocked(ipc.editUndo).mockResolvedValueOnce(makeStepView(2070, 3));

      useAppStore.getState().togglePlay();
      await vi.advanceTimersByTimeAsync(STEP_DURATION_MS / 2);
      expect(useAppStore.getState().playing).toBe(true);

      const p = useAppStore.getState().undo();
      expect(useAppStore.getState().playing).toBe(false); // 送る前に止まる
      await vi.advanceTimersByTimeAsync(0);
      await p;

      // 予約されていたコマも取り消されている(折り直した形が上書きされない)
      const count = replayCalls().length;
      await vi.advanceTimersByTimeAsync(STEP_DURATION_MS);
      expect(replayCalls()).toHaveLength(count);
    } finally {
      vi.useRealTimers();
    }
  });

  it("手順が減ったら、表示中の手順番号を手順数まで詰める", async () => {
    seedSequence(3, 3);
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(makeStepView(2100, 2));

    await useAppStore.getState().applySequenceOp({ type: "RemoveStep", id: 3 });

    expect(useAppStore.getState().currentStep).toBe(2);
    expect(replayCalls()).toEqual([[2, 1]]);
  });

  it("手順が全て無くなったら最新表示に戻り、再生は呼ばない", async () => {
    seedSequence(1, 1);
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(makeView(2200));

    await useAppStore.getState().applySequenceOp({ type: "RemoveStep", id: 1 });

    expect(useAppStore.getState().currentStep).toBeNull();
    expect(replayCalls()).toEqual([]);
  });
});

describe("展開図の置き換え", () => {
  it("手順・角度履歴・3D表示を新規作品と同じくリセットする", async () => {
    const before = makeStepView(2300, 1);
    const replacement = makeView(2301);
    vi.mocked(ipc.editApply).mockResolvedValueOnce(replacement);
    useAppStore.setState({
      doc: before.doc,
      faces: before.faces,
      hinges: new Set([5]),
      currentStep: 1,
      frame3d: { faces: [], warnings: ["前の手順の形"] },
      drivers: new Map([[5, 90]]),
      poseAngles: new Map([[5, 90]]),
      angleUndoStack: [new Map([[5, 45]])],
      angleRedoStack: [new Map([[5, 30]])],
      docUndoDepth: 1,
    });

    await useAppStore.getState().applyEdit({
      type: "ReplaceCreasePattern",
      cp: replacement.doc.cp,
    });

    const s = useAppStore.getState();
    expect(s.doc?.sequence).toEqual([]);
    expect(s.currentStep).toBeNull();
    expect(s.frame3d).toBeNull();
    expect(s.drivers.size).toBe(0);
    expect(s.poseAngles.size).toBe(0);
    expect(s.angleUndoStack).toEqual([]);
    expect(s.angleRedoStack).toEqual([]);
    expect(ipc.poseSolve).not.toHaveBeenCalled();
    expect(ipc.sequenceReplay).not.toHaveBeenCalled();
  });
});

describe("3D画面での折り操作(折り線を引いて折る)", () => {
  /** 同じ位置に2枚重なった平らな状態(下=面0、上=面1) */
  function stackedFrame() {
    const quad: [number, number, number][] = [
      [0, 0, 0],
      [1, 0, 0],
      [1, 1, 0],
      [0, 1, 0],
    ];
    return {
      faces: [
        { face: 0, polygon: quad, layer: 0 },
        { face: 1, polygon: quad, layer: 1 },
      ],
      warnings: [],
    };
  }

  /** 手順1つ・2層の畳んだ状態を表示中にする */
  function seedFolded(): void {
    seedSequence(1);
    useAppStore.setState({ activeTool: "fold", frame3d: stackedFrame() });
  }

  const LINE: [Vec2, Vec2] = [
    [0.5, 0],
    [0.5, 1],
  ];

  function foldThroughProposalView(mark: number): DocumentView {
    const view = makeStepView(mark, 1);
    view.fold_through_proposal = {
      folded_line: [
        [0.4, 0.2],
        [0.4, 0.8],
      ],
      crease_segments: [
        [
          [0.6, 0.2],
          [0.6, 0.8],
        ],
      ],
      message: "縁に沿う追加折り目を入れると貫通を避けられます。",
    };
    return view;
  }

  it("平らに畳んだ状態なら、途中の手順を見ていても折れる", () => {
    seedFolded();
    expect(canFoldNow(useAppStore.getState())).toBe(true);
    expect(foldInsertAt(useAppStore.getState())).toBe(1); // 最新=末尾へ足す

    useAppStore.setState({ playT: 0.5 });
    expect(canFoldNow(useAppStore.getState())).toBe(false);

    // 折る前の形を見ている間も折れる。折ると手順1の前に挟まる(SEQ-006)
    useAppStore.setState({ playT: 1, currentStep: 0 });
    expect(canFoldNow(useAppStore.getState())).toBe(true);
    expect(foldInsertAt(useAppStore.getState())).toBe(0);

    useAppStore.setState({ currentStep: 1 });
    expect(canFoldNow(useAppStore.getState())).toBe(true); // 最終手順=最新の形
    expect(foldInsertAt(useAppStore.getState())).toBe(1);

    useAppStore.setState({ drivers: new Map([[5, 90]]) });
    expect(canFoldNow(useAppStore.getState())).toBe(false); // 角度スライダーで変形中
  });

  it("引いた折り線の設定どおりにFoldThroughを送る(全ての層・向こうへ折る)", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply).mockResolvedValue(makeStepView(3000, 2));

    const store = useAppStore.getState();
    store.beginFoldDraft(LINE, "3d");
    expect(useAppStore.getState().foldDraft).toEqual({
      line: LINE,
      direction: "Up",
      target: "all",
      movingSide: "right",
      // 線を引いた時点の形を覚えておく(折るときに食い違いを見つけるため)
      docEpoch: useAppStore.getState().docEpoch,
      stepCount: 1,
      upTo: 1,
    });
    store.updateFoldDraft({ direction: "Down" });
    await useAppStore.getState().commitFoldDraft();

    const calls = vi.mocked(ipc.sequenceApply).mock.calls;
    expect(calls.map(([op]) => op.type)).toEqual([
      "PreviewFoldThrough",
      "FoldThrough",
    ]);
    const op = calls[1][0];
    expect(op.type).toBe("FoldThrough");
    if (op.type !== "FoldThrough") throw new Error("FoldThroughでない");
    expect(op.up_to).toBe(1); // 手順の数(末尾へ足す)
    expect(op.line).toEqual(LINE);
    expect(op.direction).toBe("Down");
    expect(op.target_layers).toBeNull(); // 全ての層
    expect(op.accept_additional_crease).toBe(false); // 提案なしなら従来どおり折る
    // 右側を動かすので、動かさない側の点は左(x<0.5)
    expect(op.keep_side_point[0]).toBeLessThan(0.5);
    // 折り終えたら折り線は捨て、最新の形を表示する
    expect(useAppStore.getState().foldDraft).toBeNull();
    expect(useAppStore.getState().currentStep).toBeNull();
  });

  it("途中の手順を見ているときは、その手順の前へ挟むFoldThroughを送る(SEQ-006)", async () => {
    seedSequence(3, 1); // 手順3つのうち、手順1まで折った形を表示中
    useAppStore.setState({ activeTool: "fold", frame3d: stackedFrame() });
    vi.mocked(ipc.sequenceApply).mockResolvedValue(makeStepView(3020, 4));

    useAppStore.getState().beginFoldDraft(LINE, "3d");
    expect(useAppStore.getState().foldDraft?.upTo).toBe(1);
    await useAppStore.getState().commitFoldDraft();

    const op = vi.mocked(ipc.sequenceApply).mock.calls[1][0];
    if (op.type !== "FoldThrough") throw new Error("FoldThroughでない");
    expect(op.up_to).toBe(1); // 手順2の位置へ挟む(後ろの手順2・3は残る)
    // 挟んだ手順(2番目)を表示したままにする。最新へ飛ばさない
    expect(useAppStore.getState().currentStep).toBe(2);
  });

  it("線を引いた後に別の手順へ移ったら、その線は捨てる", async () => {
    seedSequence(3, 1);
    useAppStore.setState({ activeTool: "fold", frame3d: stackedFrame() });

    useAppStore.getState().beginFoldDraft(LINE, "3d");
    // 別の位置の形へ移ると、線は「別の形の上」で引いたものになる
    useAppStore.setState({ currentStep: 2 });
    await useAppStore.getState().commitFoldDraft();

    expect(ipc.sequenceApply).not.toHaveBeenCalled();
    expect(useAppStore.getState().foldDraft).toBeNull();
    expect(useAppStore.getState().errorMessage).toContain("もう一度線を引いて");
  });

  it("「いちばん上の1枚」ではその層の面IDだけを対象にする", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply).mockResolvedValue(makeStepView(3010, 2));

    useAppStore.getState().beginFoldDraft(LINE, "3d");
    useAppStore.getState().updateFoldDraft({ target: "top", movingSide: "left" });
    await useAppStore.getState().commitFoldDraft();

    const op = vi.mocked(ipc.sequenceApply).mock.calls[1][0];
    if (op.type !== "FoldThrough") throw new Error("FoldThroughでない");
    expect(op.target_layers).toEqual([1]); // 重なり順がいちばん上の面
    expect(op.keep_side_point[0]).toBeGreaterThan(0.5); // 左を動かすので右が残る
  });

  it("折れなかったときは折り線を残し、やめると捨てる", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply)
      .mockResolvedValueOnce(makeStepView(3030, 1))
      .mockRejectedValueOnce("折り線がどの層の面も横切っていません");

    useAppStore.getState().beginFoldDraft(LINE, "3d");
    await useAppStore.getState().commitFoldDraft();

    expect(useAppStore.getState().errorMessage).toContain("横切っていません");
    expect(useAppStore.getState().foldDraft).not.toBeNull();

    useAppStore.getState().cancelFoldDraft();
    expect(useAppStore.getState().foldDraft).toBeNull();
  });

  it("手順がある作品では展開図側から折れない(3D画面へ案内する)", () => {
    seedFolded();
    useAppStore.getState().beginFoldDraft(LINE, "2d");
    expect(useAppStore.getState().foldDraft).toBeNull();
    expect(useAppStore.getState().errorMessage).toContain("3D画面から");

    // まだ1度も折っていない作品なら展開図側からも折れる
    useAppStore.setState({ doc: makeView(3020).doc, errorMessage: null });
    useAppStore.getState().beginFoldDraft(LINE, "2d");
    expect(useAppStore.getState().foldDraft?.line).toEqual(LINE);
  });

  it("ツールを切り替えると引きかけの折り線を捨てる", () => {
    seedFolded();
    useAppStore.getState().beginFoldDraft(LINE, "3d");
    useAppStore.getState().setTool("select");
    expect(useAppStore.getState().foldDraft).toBeNull();
  });

  it("元に戻すと引きかけの折り線を捨てる(別の形の上の線を折らない)", async () => {
    seedFolded();
    useAppStore.getState().beginFoldDraft(LINE, "3d");
    // 元に戻して手順が1つ減る = 線を引いた形とは別の形になる
    vi.mocked(ipc.editUndo).mockResolvedValueOnce(makeStepView(3030, 0));
    await useAppStore.getState().undo();

    expect(useAppStore.getState().foldDraft).toBeNull();
    // 折る要求は送られない
    await useAppStore.getState().commitFoldDraft();
    expect(vi.mocked(ipc.sequenceApply)).not.toHaveBeenCalled();
  });

  it("線を引いた後に手順が変わっていたら、折らずに捨てて知らせる", async () => {
    seedFolded();
    useAppStore.getState().beginFoldDraft(LINE, "3d");
    // ストアの外(応答の反映以外の経路)で形が変わった状況を作る
    useAppStore.setState({ doc: makeStepView(3040, 2).doc });

    await useAppStore.getState().commitFoldDraft();

    expect(vi.mocked(ipc.sequenceApply)).not.toHaveBeenCalled();
    expect(useAppStore.getState().foldDraft).toBeNull();
    expect(useAppStore.getState().errorMessage).toContain("もう一度線を引いて");
  });

  it("折れる状態でなくなったら(途中の手順を表示中)、折らずに捨てる", async () => {
    seedFolded();
    useAppStore.getState().beginFoldDraft(LINE, "3d");
    // 途中の手順を選ぶと畳み平面の座標と画面の形が食い違う
    useAppStore.setState({ currentStep: 0 });

    await useAppStore.getState().commitFoldDraft();

    expect(vi.mocked(ipc.sequenceApply)).not.toHaveBeenCalled();
    expect(useAppStore.getState().foldDraft).toBeNull();
    expect(useAppStore.getState().errorMessage).toContain("もう一度線を引いて");
  });

  it("手順を選ぶ・再生を始めると引きかけの折り線を捨てる", async () => {
    seedFolded();
    useAppStore.getState().beginFoldDraft(LINE, "3d");
    useAppStore.getState().selectStep(0);
    expect(useAppStore.getState().foldDraft).toBeNull();
    await Promise.resolve();

    seedFolded();
    useAppStore.getState().beginFoldDraft(LINE, "3d");
    useAppStore.getState().togglePlay();
    expect(useAppStore.getState().foldDraft).toBeNull();
  });

  it("事前確認中・提案中は別の折り入力で元の操作を上書きしない", async () => {
    seedFolded();
    const preview = deferred<DocumentView>();
    vi.mocked(ipc.sequenceApply).mockReset().mockReturnValueOnce(preview.promise);

    useAppStore.getState().beginFoldDraft(LINE, "3d");
    const commit = useAppStore.getState().commitFoldDraft();
    expect(useAppStore.getState().foldThroughBusy).toBe(true);

    const another: [Vec2, Vec2] = [
      [0.25, 0],
      [0.25, 1],
    ];
    useAppStore.getState().beginFoldDraft(another, "3d");
    await useAppStore.getState().foldByDrag([0.2, 0.5], [0.8, 0.5], "all");
    expect(useAppStore.getState().foldDraft?.line).toEqual(LINE);
    expect(ipc.sequenceApply).toHaveBeenCalledTimes(1);

    preview.resolve(foldThroughProposalView(3050));
    await commit;
    expect(useAppStore.getState().pendingFoldThrough).not.toBeNull();

    useAppStore.getState().beginFoldDraft(another, "3d");
    await useAppStore.getState().foldByDrag([0.2, 0.5], [0.8, 0.5], "all");
    expect(useAppStore.getState().foldDraft).toBeNull();
    expect(ipc.sequenceApply).toHaveBeenCalledTimes(1);
  });

  it("確認IPCの待機中にツールを変えたら、古い候補も元の折りも適用しない", async () => {
    seedFolded();
    const preview = deferred<DocumentView>();
    vi.mocked(ipc.sequenceApply).mockReset().mockReturnValueOnce(preview.promise);

    useAppStore.getState().beginFoldDraft(LINE, "3d");
    const commit = useAppStore.getState().commitFoldDraft();
    useAppStore.getState().setTool("select");
    preview.resolve(foldThroughProposalView(3051));
    await commit;

    expect(useAppStore.getState().pendingFoldThrough).toBeNull();
    expect(useAppStore.getState().foldThroughBusy).toBe(false);
    expect(ipc.sequenceApply).toHaveBeenCalledTimes(1);
  });

  it("提案後に再生状態へ変わっていたら、承認時にも古い折りを断る", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply)
      .mockReset()
      .mockResolvedValueOnce(foldThroughProposalView(3052));

    useAppStore.getState().beginFoldDraft(LINE, "3d");
    await useAppStore.getState().commitFoldDraft();
    expect(useAppStore.getState().pendingFoldThrough).not.toBeNull();

    // 通常のtogglePlayは提案を即時取消する。ここでは承認処理自身の最終防衛を検査する。
    useAppStore.setState({ playing: true });
    await useAppStore.getState().resolveFoldThroughProposal(true);

    expect(ipc.sequenceApply).toHaveBeenCalledTimes(1);
    expect(useAppStore.getState().pendingFoldThrough).toBeNull();
    expect(useAppStore.getState().errorMessage).toContain("もう一度線を引いて");
  });

  it("提案後に別の手順を選んだ時点で、古いプレビューをすぐ消す", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply)
      .mockReset()
      .mockResolvedValueOnce(foldThroughProposalView(3054));

    useAppStore.getState().beginFoldDraft(LINE, "3d");
    await useAppStore.getState().commitFoldDraft();
    expect(useAppStore.getState().pendingFoldThrough).not.toBeNull();

    useAppStore.getState().selectStep(0);
    expect(useAppStore.getState().pendingFoldThrough).toBeNull();
    await Promise.resolve();
  });

  it("後続の保存で最新でなくなった折りが失敗しても、提案のbusyを必ず戻す", async () => {
    seedFolded();
    const fold = deferred<DocumentView>();
    vi.mocked(ipc.sequenceApply)
      .mockReset()
      .mockResolvedValueOnce(foldThroughProposalView(3053))
      .mockReturnValueOnce(fold.promise);
    vi.mocked(ipc.documentSave).mockResolvedValue(undefined);

    useAppStore.getState().beginFoldDraft(LINE, "3d");
    await useAppStore.getState().commitFoldDraft();
    const accept = useAppStore.getState().resolveFoldThroughProposal(true);
    const save = useAppStore.getState().saveDocument(null);
    fold.reject("古い折りの失敗");
    await Promise.all([accept, save]);

    expect(useAppStore.getState().foldThroughBusy).toBe(false);
    expect(useAppStore.getState().pendingFoldThrough).not.toBeNull();
  });
});

describe("技法(選ぶだけで折る)", () => {
  /** 同じ位置に2枚重なった平らな状態(下=面0、上=面1) */
  function stackedFrame() {
    const quad: [number, number, number][] = [
      [0, 0, 0],
      [1, 0, 0],
      [1, 1, 0],
      [0, 1, 0],
    ];
    return {
      faces: [
        { face: 0, polygon: quad, layer: 0 },
        { face: 1, polygon: quad, layer: 1 },
      ],
      warnings: [],
    };
  }

  /** 手順1つ・2層の畳んだ状態を表示中にする */
  function seedFolded(): void {
    seedSequence(1);
    useAppStore.setState({ activeTool: "technique", frame3d: stackedFrame() });
  }

  const LINE: [Vec2, Vec2] = [
    [0.5, 0],
    [0.5, 1],
  ];

  it("平らに畳む技法で4点を知らせても技法と最新の形を適用する", async () => {
    seedFolded();
    const applied = makeStepView(3999, 2);
    applied.frame = { faces: [], warnings: ["技法を適用した形"] };
    applied.flat_fold_violations = [9, 10, 11, 12];
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(applied);

    const store = useAppStore.getState();
    store.beginTechnique("InsideReverse");
    store.setTechniqueFlap([0, 1]);
    store.setTechniqueLine(LINE);
    await store.commitTechnique();

    expect(vi.mocked(ipc.sequenceApply)).toHaveBeenCalledTimes(1);
    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    expect(op.type).toBe("Technique");
    expect(op.type === "Technique" && op.kind).toBe("InsideReverse");
    const state = useAppStore.getState();
    expect(state.doc?.sequence).toHaveLength(2);
    expect(state.techniqueDraft).toBeNull();
    expect(state.currentStep).toBeNull();
    expect(state.frame3d?.warnings).toEqual(["技法を適用した形"]);
    expect(state.flatFoldViolations).toEqual([9, 10, 11, 12]);
    expect(state.errorMessage).toBeNull();
  });

  it("中割り折り: フラップと折り線を選んでTechniqueを送る", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(makeStepView(4000, 2));

    const store = useAppStore.getState();
    store.beginTechnique("InsideReverse");
    expect(useAppStore.getState().techniqueDraft).toEqual({
      kind: "InsideReverse",
      flap: [],
      flapCandidates: [],
      flapPickCount: 1,
      line: null,
      movingSide: "right",
      widthMm: 10,
      polygon: [],
      center: null,
      referencePoint: null,
      twistDeg: 30,
      openToBack: false,
      motionMode: "reflect",
      motionTurn: "Keep",
      motionDirection: "Up",
      motionAnchor: 0,
      motionReverseLayers: false,
      motionAxisEdgeId: null,
      motionParts: [],
      docEpoch: useAppStore.getState().docEpoch,
      stepCount: 1,
      upTo: 1,
    });
    useAppStore.getState().setTechniqueFlap([0, 1]);
    useAppStore.getState().setTechniqueLine(LINE);
    await useAppStore.getState().commitTechnique();

    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    expect(op.type).toBe("Technique");
    if (op.type !== "Technique") throw new Error("Techniqueでない");
    expect(op.up_to).toBe(1); // 手順の数(末尾へ足す)
    expect(op.kind).toBe("InsideReverse");
    expect(op.flap).toEqual([0, 1]);
    expect(op.line).toEqual(LINE);
    expect(op).not.toHaveProperty("open_to_back");
    // 「こちら側」が動くので、先端が向かう側(基準点)は反対側(x<0.5)
    expect(op.reference_point[0]).toBeLessThan(0.5);
    // 折り終えたら下ごしらえを捨て、最新の形を表示する
    expect(useAppStore.getState().techniqueDraft).toBeNull();
    expect(useAppStore.getState().currentStep).toBeNull();
  });

  it("層操作: 既存折り目の開閉と重ね替えを複数部分のFlatMotionで送る", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(makeStepView(4001, 2));

    useAppStore.getState().beginTechnique("Simple");
    useAppStore.getState().setTechniqueFlap([1]);
    useAppStore.getState().setLayerMotionAxis(12, LINE);
    useAppStore.getState().addLayerMotionPart();

    expect(useAppStore.getState().techniqueDraft).toMatchObject({
      flap: [],
      line: null,
      motionAxisEdgeId: null,
      motionParts: [
        {
          layers: [1],
          region: [],
          transform: { Reflect: [LINE] },
          turn: "Keep",
        },
      ],
    });

    useAppStore.getState().setTechniqueFlap([0]);
    useAppStore.getState().updateTechniqueDraft({
      motionMode: "stay",
      motionTurn: "Beside",
      motionDirection: "Down",
      motionAnchor: 1,
      motionReverseLayers: true,
    });
    await useAppStore.getState().commitTechnique();

    expect(vi.mocked(ipc.sequenceApply)).toHaveBeenCalledWith({
      type: "FlatMotion",
      up_to: 1,
      kind: "Simple",
      parts: [
        {
          layers: [1],
          region: [],
          transform: { Reflect: [LINE] },
          turn: "Keep",
        },
        {
          layers: [0],
          region: [],
          transform: "Stay",
          turn: { Beside: { anchor: 1, direction: "Down" } },
          reverse_layers: true,
        },
      ],
    });
    expect(useAppStore.getState().techniqueDraft).toBeNull();
  });

  it("層操作: Stay+Keepの無変更は送らず、選択層の山谷反転だけなら送る", async () => {
    seedFolded();
    useAppStore.getState().beginTechnique("Simple");
    useAppStore.getState().updateTechniqueDraft({ motionMode: "stay" });

    await useAppStore.getState().commitTechnique();
    expect(vi.mocked(ipc.sequenceApply)).not.toHaveBeenCalled();
    expect(useAppStore.getState().errorMessage).toContain("重ね方・山谷反転");

    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(makeStepView(4002, 2));
    useAppStore.getState().setTechniqueFlap([0]);
    useAppStore.getState().updateTechniqueDraft({ motionReverseLayers: true });
    await useAppStore.getState().commitTechnique();

    expect(vi.mocked(ipc.sequenceApply)).toHaveBeenCalledWith({
      type: "FlatMotion",
      up_to: 1,
      kind: "Simple",
      parts: [
        {
          layers: [0],
          region: [],
          transform: "Stay",
          turn: "Keep",
          reverse_layers: true,
        },
      ],
    });
  });

  it("層操作: ドラッグした目分量の線は既存折り目のReflect軸として送らない", async () => {
    seedFolded();
    useAppStore.getState().beginTechnique("Simple");
    useAppStore.getState().setTechniqueFlap([1]);
    useAppStore.getState().setTechniqueLine(LINE);

    await useAppStore.getState().commitTechnique();

    expect(vi.mocked(ipc.sequenceApply)).not.toHaveBeenCalled();
    expect(useAppStore.getState().errorMessage).toContain("正確な開閉軸");
    expect(useAppStore.getState().techniqueDraft?.motionAxisEdgeId).toBeNull();
  });

  it("段折り: フラップ指定なしで送れる。基準点は段の幅ぶん動く側へ離れる", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(makeStepView(4010, 2));

    useAppStore.getState().beginTechnique("Pleat");
    useAppStore.getState().setTechniqueLine(LINE);
    useAppStore.getState().updateTechniqueDraft({ widthMm: 15 });
    await useAppStore.getState().commitTechnique();

    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    if (op.type !== "Technique") throw new Error("Techniqueでない");
    expect(op.kind).toBe("Pleat");
    expect(op.flap).toEqual([]);
    // 紙の長辺150mmに対して段の幅15mm = 正規化座標で0.1。動く側(こちら側=x>0.5)
    expect(op.reference_point[0]).toBeCloseTo(0.6, 9);
    expect(op.reference_point[1]).toBeCloseTo(0.5, 9);
  });

  it("沈め折り: 自動の基準点は動かす側と同じ先端側に置く", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(makeStepView(4011, 2));

    useAppStore.getState().beginTechnique("OpenSink");
    useAppStore.getState().setTechniqueLine(LINE);
    await useAppStore.getState().commitTechnique();

    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    if (op.type !== "Technique") throw new Error("Techniqueでない");
    expect(op.kind).toBe("OpenSink");
    // LINEは下→上なのでrightはx>0.5。沈める先端側をそのままRustへ渡す。
    expect(op.reference_point[0]).toBeGreaterThan(0.5);
    expect(op.reference_point[1]).toBeCloseTo(0.5, 9);
  });

  it("任意の基準点を指定すると技法ごとの自動点より優先する", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(makeStepView(4012, 2));

    useAppStore.getState().beginTechnique("Swivel");
    useAppStore.getState().setTechniqueLine(LINE);
    useAppStore.getState().setTechniqueReferencePoint([0.87, 0.13]);
    await useAppStore.getState().commitTechnique();

    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    if (op.type !== "Technique") throw new Error("Techniqueでない");
    expect(op.kind).toBe("Swivel");
    expect(op.reference_point).toEqual([0.87, 0.13]);
  });

  it("フラップや折り線が足りないときは送らずに案内する", async () => {
    seedFolded();
    useAppStore.getState().beginTechnique("InsideReverse");

    // 折り線がまだ無い
    await useAppStore.getState().commitTechnique();
    expect(vi.mocked(ipc.sequenceApply)).not.toHaveBeenCalled();
    expect(useAppStore.getState().errorMessage).toContain("折り線");

    // 層が1枚しか選ばれていない
    useAppStore.getState().setTechniqueLine(LINE);
    useAppStore.getState().setTechniqueFlap([1]);
    await useAppStore.getState().commitTechnique();
    expect(vi.mocked(ipc.sequenceApply)).not.toHaveBeenCalled();
    expect(useAppStore.getState().errorMessage).toContain("層");
    expect(useAppStore.getState().techniqueDraft).not.toBeNull();

    // 層が2枚以上あれば送る(枚数が奇数でも、先端の向きは紙のつながりから決まる)
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(makeStepView(4030, 2));
    useAppStore.getState().setTechniqueFlap([0, 1, 2]);
    await useAppStore.getState().commitTechnique();
    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    if (op.type !== "Technique") throw new Error("Techniqueでない");
    expect(op.flap).toEqual([0, 1, 2]);
  });

  it.each<{
    kind: TechniqueKind;
    flap: number[];
    sendsOpenSide: boolean;
  }>([
    { kind: "Squash", flap: [1], sendsOpenSide: true },
    { kind: "Petal", flap: [1], sendsOpenSide: true },
    { kind: "OpenSink", flap: [], sendsOpenSide: false },
    { kind: "Swivel", flap: [], sendsOpenSide: true },
  ])("$kindはRust側と同じ最小層数で送れる", async ({ kind, flap, sendsOpenSide }) => {
    seedFolded();
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(makeStepView(4035, 2));
    useAppStore.getState().beginTechnique(kind);
    useAppStore.getState().setTechniqueLine(LINE);
    if (flap.length > 0) useAppStore.getState().setTechniqueFlap(flap);

    await useAppStore.getState().commitTechnique();

    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    if (op.type !== "Technique") throw new Error("Techniqueでない");
    expect(op.flap).toEqual(flap);
    if (sendsOpenSide) expect(op.open_to_back).toBe(false);
    else expect(op).not.toHaveProperty("open_to_back");
  });

  it("向こうへ開く指定をsnake_caseでRustへ送る", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(makeStepView(4036, 2));
    useAppStore.getState().beginTechnique("Petal");
    useAppStore.getState().setTechniqueFlap([1]);
    useAppStore.getState().setTechniqueLine(LINE);
    useAppStore.getState().updateTechniqueDraft({ openToBack: true });

    await useAppStore.getState().commitTechnique();

    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    if (op.type !== "Technique") throw new Error("Techniqueでない");
    expect(op.open_to_back).toBe(true);
  });

  it("クリック候補を全選択し、枚数・奥行き・個別チェックで部分集合にできる", () => {
    seedFolded();
    useAppStore.getState().beginTechnique("Squash");
    const candidates = Array.from({ length: 128 }, (_, i) => i);

    useAppStore.getState().setTechniqueFlap(candidates);
    expect(useAppStore.getState().techniqueDraft?.flapCandidates).toEqual(candidates);
    expect(useAppStore.getState().techniqueDraft?.flap).toEqual(candidates);

    useAppStore.getState().updateTechniqueDraft({ flapPickCount: 51 });
    useAppStore.getState().setTechniqueFlapPreset("front");
    expect(useAppStore.getState().techniqueDraft?.flap).toEqual(candidates.slice(77));

    useAppStore.getState().updateTechniqueDraft({ flapPickCount: 55 });
    useAppStore.getState().setTechniqueFlapPreset("back");
    expect(useAppStore.getState().techniqueDraft?.flap).toEqual(candidates.slice(0, 55));

    useAppStore.getState().updateTechniqueDraft({ flapPickCount: 128 });
    useAppStore.getState().setTechniqueFlapPreset("frontNth");
    expect(useAppStore.getState().techniqueDraft?.flap).toEqual([0]);
    useAppStore.getState().toggleTechniqueFlap(127);
    expect(useAppStore.getState().techniqueDraft?.flap).toEqual([0, 127]);
  });

  it("ねじり折り: 順にクリックした角がそのまま中央多角形として送られる", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(makeStepView(4040, 2));

    useAppStore.getState().beginTechnique("Twist");
    // 辺の長さがそろっていない三角形(正多角形では表せない形)
    const pts: Vec2[] = [
      [0.2, 0.2],
      [0.8, 0.2],
      [0.3, 0.9],
    ];
    for (const p of pts) useAppStore.getState().addTechniqueVertex(p);
    // 角を1つ余分に置いてから取り消せる
    useAppStore.getState().addTechniqueVertex([0.1, 0.5]);
    useAppStore.getState().undoTechniqueVertex();
    expect(useAppStore.getState().techniqueDraft?.polygon).toEqual(pts);

    await useAppStore.getState().commitTechnique();
    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    if (op.type !== "Technique") throw new Error("Techniqueでない");
    expect(op.kind).toBe("Twist");
    expect(op.polygon).toEqual(pts);
    // 中心は指定しなければ多角形の重心
    expect(op.center?.[0]).toBeCloseTo((0.2 + 0.8 + 0.3) / 3, 9);
    expect(op.center?.[1]).toBeCloseTo((0.2 + 0.2 + 0.9) / 3, 9);
    // 層を選ばなくても送れる(選ばなければ全ての層)
    expect(op.flap).toEqual([]);
    // 折り線は1辺目(エンジンは多角形を優先する)
    expect(op.line).toEqual([pts[0], pts[1]]);
    expect(useAppStore.getState().techniqueDraft).toBeNull();
  });

  it("ねじり折り: 中心を指した点にでき、角が3つ未満なら送らずに案内する", async () => {
    seedFolded();
    useAppStore.getState().beginTechnique("Twist");
    useAppStore.getState().addTechniqueVertex([0.2, 0.2]);
    useAppStore.getState().addTechniqueVertex([0.8, 0.2]);

    await useAppStore.getState().commitTechnique();
    expect(vi.mocked(ipc.sequenceApply)).not.toHaveBeenCalled();
    expect(useAppStore.getState().errorMessage).toContain("角を3つ以上");

    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(makeStepView(4050, 2));
    useAppStore.getState().addTechniqueVertex([0.3, 0.9]);
    useAppStore.getState().setTechniqueCenter([0.4, 0.4]);
    await useAppStore.getState().commitTechnique();
    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    if (op.type !== "Technique") throw new Error("Techniqueでない");
    expect(op.center).toEqual([0.4, 0.4]);
    // 基準点は1辺目の中点を中心のまわりに既定の30度回した点(ねじる角)
    const mid: Vec2 = [0.5, 0.2];
    const rad = (30 * Math.PI) / 180;
    expect(op.reference_point[0]).toBeCloseTo(
      0.4 + (mid[0] - 0.4) * Math.cos(rad) - (mid[1] - 0.4) * Math.sin(rad),
      9,
    );
  });

  it("折れなかったときは手動の折り操作への案内を添え、下ごしらえを残す", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply).mockRejectedValueOnce(
      "このフラップには中割り折りができません。折り線がフラップを横切っていないか確認してください",
    );

    useAppStore.getState().beginTechnique("InsideReverse");
    useAppStore.getState().setTechniqueFlap([0, 1]);
    useAppStore.getState().setTechniqueLine(LINE);
    await useAppStore.getState().commitTechnique();

    expect(useAppStore.getState().errorMessage).toContain("中割り折りができません");
    expect(useAppStore.getState().errorMessage).toContain("手動の折り操作で代替");
    expect(useAppStore.getState().techniqueDraft).not.toBeNull();

    useAppStore.getState().cancelTechnique();
    expect(useAppStore.getState().techniqueDraft).toBeNull();
  });

  it("形が変わったら折らずに捨てて知らせる", async () => {
    seedFolded();
    useAppStore.getState().beginTechnique("Pleat");
    useAppStore.getState().setTechniqueLine(LINE);
    // ストアの外(応答の反映以外の経路)で形が変わった状況を作る
    useAppStore.setState({ doc: makeStepView(4020, 2).doc });

    await useAppStore.getState().commitTechnique();

    expect(vi.mocked(ipc.sequenceApply)).not.toHaveBeenCalled();
    expect(useAppStore.getState().techniqueDraft).toBeNull();
    expect(useAppStore.getState().errorMessage).toContain("もう一度線を引いて");
  });

  it("ツールの切り替え・手順の選択で下ごしらえを捨てる", () => {
    seedFolded();
    useAppStore.getState().beginTechnique("Pleat");
    useAppStore.getState().setTool("select");
    expect(useAppStore.getState().techniqueDraft).toBeNull();

    seedFolded();
    useAppStore.getState().beginTechnique("Pleat");
    useAppStore.getState().selectStep(0);
    expect(useAppStore.getState().techniqueDraft).toBeNull();
  });
});

describe("自動保存からの復旧(SYS-003)", () => {
  const INFO = {
    autosave_path: "C:/作品/鶴.ori3.autosave",
    document_path: "C:/作品/鶴.ori3",
    saved_at_ms: 1_700_000_000_000,
  };

  it("起動時に残っていればダイアログの材料を持つ。無ければ何も出さない", async () => {
    await useAppStore.getState().checkRecovery();
    expect(useAppStore.getState().recovery).toBeNull();

    vi.mocked(ipc.recoveryCheck).mockResolvedValue(INFO);
    await useAppStore.getState().checkRecovery();
    expect(useAppStore.getState().recovery).toEqual(INFO);
  });

  it("復元すると作業中だった内容が画面に載り、提案は消える", async () => {
    const view = makeView(700);
    vi.mocked(ipc.recoveryRestore).mockResolvedValue(view);
    useAppStore.setState({ recovery: INFO });

    await useAppStore.getState().resolveRecovery(true);

    expect(vi.mocked(ipc.recoveryRestore)).toHaveBeenCalledWith(true);
    const s = useAppStore.getState();
    expect(s.doc).toEqual(view.doc);
    expect(s.recovery).toBeNull();
    expect(s.docEpoch).toBe(1); // 別の作品になったので世代が進む
    expect(s.errorMessage).toBeNull();
  });

  it("破棄すると内容は読み込まず、提案だけ消える", async () => {
    vi.mocked(ipc.recoveryRestore).mockResolvedValue(null);
    useAppStore.setState({ recovery: INFO, doc: makeView(701).doc });

    await useAppStore.getState().resolveRecovery(false);

    expect(vi.mocked(ipc.recoveryRestore)).toHaveBeenCalledWith(false);
    const s = useAppStore.getState();
    expect(s.recovery).toBeNull();
    expect(s.doc).toEqual(makeView(701).doc); // 今開いている作品はそのまま
    expect(s.errorMessage).toBeNull();
  });

  it("答えは1回きり(二度押しでも要求は1回)", async () => {
    vi.mocked(ipc.recoveryRestore).mockResolvedValue(null);
    useAppStore.setState({ recovery: INFO });

    await Promise.all([
      useAppStore.getState().resolveRecovery(false),
      useAppStore.getState().resolveRecovery(false),
    ]);

    expect(vi.mocked(ipc.recoveryRestore)).toHaveBeenCalledTimes(1);
  });

  it("復元できなかったときは理由を出す", async () => {
    vi.mocked(ipc.recoveryRestore).mockResolvedValue(null);
    useAppStore.setState({ recovery: INFO });

    await useAppStore.getState().resolveRecovery(true);

    expect(useAppStore.getState().errorMessage).toContain("見つかりませんでした");
  });
});

describe("立体的な仕上げの形を手順として残す(SIM-009)", () => {
  /** 対角線2本(辺ID 5・6)が折り線の正方形。どちらもヒンジ */
  function makePoseView(mark: number): DocumentView {
    const view = makeHingeView(mark);
    view.doc.cp.edges.push({ id: 6, v0: 1, v1: 3, kind: "Valley" });
    return view;
  }

  /** 角度のついた状態を作る(5は利用者指定、6はソルバーが求めた従属角度) */
  function seedPose(): void {
    const view = makePoseView(1200);
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
      hinges: new Set([5, 6]),
      drivers: new Map([[5, 90]]),
      poseAngles: new Map([
        [5, 89.9999],
        [6, -30.5],
      ]),
    });
  }

  it("今の全ての折り線の角度を「仕上げの角度」の手順として送る", async () => {
    seedPose();
    // 応答は手順が1つ増えた作品(立体は手順の再生結果で表す)
    const pushed = makePoseView(1201);
    pushed.doc.sequence = [makeStep(0)];
    pushed.frame = { faces: [], warnings: [] };
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(pushed);

    await useAppStore.getState().recordPoseStep();

    expect(vi.mocked(ipc.sequenceApply)).toHaveBeenCalledTimes(1);
    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    expect(op).toEqual({
      type: "PushStep",
      step: {
        id: 0,
        kind: "Pose",
        // 折り線は展開図座標の線分で残す(辺IDは編集で変わるため)。
        // 希望値ではなく、最新solveが返した全ヒンジの実角を保存する
        drivers: [
          { a: [0, 0], b: [1, 1], target_angle_deg: 89.9999 },
          { a: [1, 0], b: [0, 1], target_angle_deg: -30.5 },
        ],
        layer_order: null,
        note: "",
      },
    });
    // 手順として残ったので一時的な角度指定は消える(平ら計算は送らない)
    expect(useAppStore.getState().drivers.size).toBe(0);
    expect(poseCalls()).toEqual([]);
    expect(useAppStore.getState().errorMessage).toBeNull();
  });

  it("予約中の末尾solveを待ち、返った実角をPoseへ保存する", async () => {
    const view = makePoseView(1202);
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
      hinges: new Set([5, 6]),
      drivers: new Map(),
      poseAngles: new Map(),
    });
    const solving = deferred<SolveResult>();
    vi.mocked(ipc.poseSolve).mockReturnValueOnce(solving.promise);
    const pushed = makePoseView(1203);
    pushed.doc.sequence = [makeStep(0)];
    pushed.frame = { faces: [], warnings: [] };
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(pushed);

    useAppStore.getState().setDriverAngle(5, 90);
    const recording = useAppStore.getState().recordPoseStep();
    await flush();
    expect(ipc.poseSolve).toHaveBeenCalledTimes(1);
    expect(ipc.sequenceApply).not.toHaveBeenCalled();

    solving.resolve(
      makeSolveResult({
        "5": 89.123456789,
        "6": -30.5,
      }),
    );
    await recording;

    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    expect(op.type).toBe("PushStep");
    if (op.type !== "PushStep") throw new Error("Pose手順ではありません");
    expect(op.step.drivers.map((driver) => driver.target_angle_deg)).toEqual([
      89.123456789,
      -30.5,
    ]);
    expect(useAppStore.getState().activeAngleIntent).toBeNull();
  });

  it("保存待機中に積まれた後続solveまで待ち、最後の実角を保存する", async () => {
    primeFakeTimers();
    try {
      const view = makePoseView(1204);
      useAppStore.setState({
        doc: view.doc,
        faces: view.faces,
        hinges: new Set([5, 6]),
        drivers: new Map(),
        poseAngles: new Map(),
      });
      const firstSolve = deferred<SolveResult>();
      const latestSolve = deferred<SolveResult>();
      vi.mocked(ipc.poseSolve)
        .mockReturnValueOnce(firstSolve.promise)
        .mockReturnValueOnce(latestSolve.promise);
      const pushed = makePoseView(1205);
      pushed.doc.sequence = [makeStep(0)];
      pushed.frame = { faces: [], warnings: [] };
      vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(pushed);

      useAppStore.getState().setDriverAngle(5, 90);
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS); // 90°を実行中にする
      const recording = useAppStore.getState().recordPoseStep();
      await flushMicrotasks();

      useAppStore.getState().setDriverAngle(5, 100);
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS); // 100°を待機枠へ積む
      firstSolve.resolve(makeSolveResult({ "5": 89, "6": -20 }));
      await flushMicrotasks();

      expect(ipc.poseSolve).toHaveBeenCalledTimes(2);
      expect(ipc.sequenceApply).not.toHaveBeenCalled();

      latestSolve.resolve(
        makeSolveResult({
          "5": 72.123456789,
          "6": -30.5,
        }),
      );
      await recording;

      const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
      expect(op.type).toBe("PushStep");
      if (op.type !== "PushStep") throw new Error("Pose手順ではありません");
      expect(op.step.drivers.map((driver) => driver.target_angle_deg)).toEqual([
        72.123456789,
        -30.5,
      ]);
      expect(useAppStore.getState().activeAngleIntent).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("保存が待っていたsolve自体が追い越されても、後継の実角まで待つ", async () => {
    primeFakeTimers();
    try {
      const view = makePoseView(1206);
      useAppStore.setState({
        doc: view.doc,
        faces: view.faces,
        hinges: new Set([5, 6]),
        drivers: new Map(),
        poseAngles: new Map(),
      });
      // Aを実行中にして、90°(B)をrunLatestの待機枠へ置く。
      const blockingSave = deferred<void>();
      vi.mocked(ipc.documentSave).mockReturnValueOnce(blockingSave.promise);
      const unrelated = useAppStore.getState().saveDocument(null);
      await flushMicrotasks();

      useAppStore.getState().setDriverAngle(5, 90);
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);
      const recording = useAppStore.getState().recordPoseStep();
      await flushMicrotasks();

      // 保存処理が待つBを100°(C)でSUPERSEDEDにする。IPCへ届くのはCだけ。
      const latestSolve = deferred<SolveResult>();
      vi.mocked(ipc.poseSolve).mockReturnValueOnce(latestSolve.promise);
      useAppStore.getState().setDriverAngle(5, 100);
      await vi.advanceTimersByTimeAsync(POSE_WAIT_MS);
      blockingSave.resolve();
      await flushMicrotasks();

      expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: 100 }]]);
      expect(ipc.sequenceApply).not.toHaveBeenCalled();

      const pushed = makePoseView(1207);
      pushed.doc.sequence = [makeStep(0)];
      pushed.frame = { faces: [], warnings: [] };
      vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(pushed);
      latestSolve.resolve(
        makeSolveResult({
          "5": 72.123456789,
          "6": -30.5,
        }),
      );
      await Promise.all([recording, unrelated]);

      const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
      expect(op.type).toBe("PushStep");
      if (op.type !== "PushStep") throw new Error("Pose手順ではありません");
      expect(op.step.drivers.map((driver) => driver.target_angle_deg)).toEqual([
        72.123456789,
        -30.5,
      ]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("Pose保存中に始まった新しい角度指定を消さず、自動再生後に再計算する", async () => {
    seedPose();
    useAppStore.setState({
      display: { ...useAppStore.getState().doc!.display, soft_enabled: true },
    });
    const saving = deferred<DocumentView>();
    vi.mocked(ipc.sequenceApply).mockReturnValueOnce(saving.promise);
    const pushed = makePoseView(1206);
    pushed.doc.sequence = [makeStep(0)];
    pushed.doc.display.soft_enabled = true;
    pushed.frame = { faces: [], warnings: [] };

    const recording = useAppStore.getState().recordPoseStep();
    await vi.waitFor(() => expect(ipc.sequenceApply).toHaveBeenCalledTimes(1));
    useAppStore.getState().setDriverAngle(5, 100);
    const newGeneration = useAppStore.getState().activeAngleIntent?.generation;

    saving.resolve(pushed);
    await recording;

    expect(useAppStore.getState().drivers.get(5)).toBe(100);
    expect(useAppStore.getState().activeAngleIntent?.generation).toBe(newGeneration);
    expect(ipc.sequenceReplay).toHaveBeenCalledTimes(1);
    expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: 100 }]]);
  });

  it("手順IDは既にある手順の続きになる", async () => {
    seedPose();
    useAppStore.setState({
      doc: { ...useAppStore.getState().doc!, sequence: [makeStep(3)] },
    });
    const pushed2 = makePoseView(1202);
    pushed2.doc.sequence = [makeStep(3), makeStep(4)];
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(pushed2);

    await useAppStore.getState().recordPoseStep();

    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    expect(op.type === "PushStep" && op.step.id).toBe(4);
  });

  it("角度が全く付いていなければ残せず、理由を伝える", async () => {
    const view = makePoseView(1203);
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
      hinges: new Set([5, 6]),
      drivers: new Map(),
      poseAngles: new Map([[5, 0.0001]]),
    });

    expect(poseRecordReason(useAppStore.getState())).toContain(
      "まだ角度が付いていません",
    );
    await useAppStore.getState().recordPoseStep();

    expect(vi.mocked(ipc.sequenceApply)).not.toHaveBeenCalled();
    expect(useAppStore.getState().errorMessage).toContain(
      "まだ角度が付いていません",
    );
  });

  it("折り線が1本も無いとき・再生中は残せない", () => {
    expect(poseRecordReason(useAppStore.getState())).toBe("まだ紙がありません");
    seedPose();
    expect(poseRecordReason(useAppStore.getState())).toBeNull();
    useAppStore.setState({ playing: true });
    expect(poseRecordReason(useAppStore.getState())).toContain("再生を止めて");
    useAppStore.setState({ playing: false, hinges: new Set<number>() });
    expect(poseRecordReason(useAppStore.getState())).toBe(
      "折り線がまだありません",
    );
  });
});
