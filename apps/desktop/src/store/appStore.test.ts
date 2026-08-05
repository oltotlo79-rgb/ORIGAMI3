// appStoreのテスト:
//  - 直列化まわり:「成功したviewはisLatestに関わらず破棄されない」
//    (A成功→B失敗でも、画面はAのdocを保持しバックエンドと一致する)
//  - 折り角度の指定: 60ms間引き・全解除・展開図編集後の追従
//  - 手順の表示と再生: 手順選択・コマ送り・アニメーションの進行と停止・
//    最新1件だけを送る間引き(coalescing)

import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  DocumentView,
  Driver,
  FoldStep,
  ReplayResult,
  SolveResult,
  Vec2,
} from "../lib/types";
import { STEP_DURATION_MS } from "../lib/playback";

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
import {
  canFoldNow,
  isStepSkipped,
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

/** poseSolveへ渡された引数(呼び出し番号ごと) */
function poseCalls(): Driver[][] {
  return vi.mocked(ipc.poseSolve).mock.calls.map(([drivers]) => drivers);
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
    selection: { edgeIds: [], vertexIds: [] },
    errorMessage: null,
    docEpoch: 0,
    drivers: new Map(),
    poseAngles: new Map(),
    poseWarnings: [],
    poseConverged: true,
    frame3d: null,
    currentStep: null,
    playT: 1,
    playing: false,
    skipped: [],
    replaySkipped: [],
    replayWarnings: [],
    activeTool: "select",
    foldDraft: null,
  });
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

/** 偽タイマーを使うテストの準備。
 * 間引きの基準時刻はbeforeEachのresetPoseThrottle()で初期化済みなので、
 * ここでは偽タイマーへ切り替えるだけでよい(テストの並び順に依存しない)。 */
function primeFakeTimers(): void {
  vi.useFakeTimers();
}

describe("appStore 折り角度の指定", () => {
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

  it("連続操作は60msで間引かれ、最後の角度が必ず送られる", async () => {
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

    // 解除した5は0度(平ら)を明示。残りの7はそのまま
    expect(poseCalls()).toEqual([
      [
        { hinge: 7, target_angle_deg: 30 },
        { hinge: 5, target_angle_deg: 0 },
      ],
    ]);
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

    expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: 90 }]]);
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
    expect(s.poseWarnings).toEqual(["追従計算が収束していません"]);
    expect(s.poseAngles.get(5)).toBe(90);
    expect(s.errorMessage).toBeNull(); // 追従計算の警告はエラーにしない
  });
});

describe("appStore 手順の表示と再生", () => {
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

  it("平らに畳んだ状態のときだけ折れる", () => {
    seedFolded();
    expect(canFoldNow(useAppStore.getState())).toBe(true);

    useAppStore.setState({ playT: 0.5 });
    expect(canFoldNow(useAppStore.getState())).toBe(false);

    useAppStore.setState({ playT: 1, currentStep: 0 });
    expect(canFoldNow(useAppStore.getState())).toBe(false); // 折る前の途中表示

    useAppStore.setState({ currentStep: 1 });
    expect(canFoldNow(useAppStore.getState())).toBe(true); // 最終手順=最新の形

    useAppStore.setState({ drivers: new Map([[5, 90]]) });
    expect(canFoldNow(useAppStore.getState())).toBe(false); // 角度スライダーで変形中
  });

  it("引いた折り線の設定どおりにFoldThroughを送る(全ての層・向こうへ折る)", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(makeStepView(3000, 2));

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
    });
    store.updateFoldDraft({ direction: "Down" });
    await useAppStore.getState().commitFoldDraft();

    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    expect(op.type).toBe("FoldThrough");
    if (op.type !== "FoldThrough") throw new Error("FoldThroughでない");
    expect(op.up_to).toBe(1); // 手順の数(末尾へ足す)
    expect(op.line).toEqual(LINE);
    expect(op.direction).toBe("Down");
    expect(op.target_layers).toBeNull(); // 全ての層
    // 右側を動かすので、動かさない側の点は左(x<0.5)
    expect(op.keep_side_point[0]).toBeLessThan(0.5);
    // 折り終えたら折り線は捨て、最新の形を表示する
    expect(useAppStore.getState().foldDraft).toBeNull();
    expect(useAppStore.getState().currentStep).toBeNull();
  });

  it("「いちばん上の1枚」ではその層の面IDだけを対象にする", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply).mockResolvedValueOnce(makeStepView(3010, 2));

    useAppStore.getState().beginFoldDraft(LINE, "3d");
    useAppStore.getState().updateFoldDraft({ target: "top", movingSide: "left" });
    await useAppStore.getState().commitFoldDraft();

    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    if (op.type !== "FoldThrough") throw new Error("FoldThroughでない");
    expect(op.target_layers).toEqual([1]); // 重なり順がいちばん上の面
    expect(op.keep_side_point[0]).toBeGreaterThan(0.5); // 左を動かすので右が残る
  });

  it("折れなかったときは折り線を残し、やめると捨てる", async () => {
    seedFolded();
    vi.mocked(ipc.sequenceApply).mockRejectedValueOnce("折り線がどの層の面も横切っていません");

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
});
