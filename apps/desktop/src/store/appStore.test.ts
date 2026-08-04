// appStoreのテスト:
//  - 直列化まわり:「成功したviewはisLatestに関わらず破棄されない」
//    (A成功→B失敗でも、画面はAのdocを保持しバックエンドと一致する)
//  - 折り角度の指定: 60ms間引き・全解除・展開図編集後の追従

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DocumentView, Driver, SolveResult } from "../lib/types";

vi.mock("../ipc/client", () => ({
  documentNew: vi.fn(),
  documentOpen: vi.fn(),
  documentSave: vi.fn(),
  editApply: vi.fn(),
  editUndo: vi.fn(),
  editRedo: vi.fn(),
  sequenceApply: vi.fn(),
  poseSolve: vi.fn(),
}));

import * as ipc from "../ipc/client";
import { useAppStore } from "./appStore";

/** 角度の間引き間隔(appStore.tsのPOSE_THROTTLE_MS)より少し長く待つ時間(ms) */
const POSE_WAIT_MS = 100;

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

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(ipc.poseSolve).mockResolvedValue(makeSolveResult());
  useAppStore.setState({
    doc: null,
    faces: [],
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

describe("appStore 折り角度の指定", () => {
  it("連続操作は60msで間引かれ、最後の角度が必ず送られる", async () => {
    vi.useFakeTimers();
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

  it("「全て平らに戻す」は全ての折り線へ0度を指定して送る", async () => {
    const view = makeHingeView(500);
    useAppStore.setState({ doc: view.doc, faces: view.faces });

    const store = useAppStore.getState();
    store.setDriverAngle(5, 90);
    store.clearDrivers();
    await flush();

    // 間引き中の分は送られず、平ら指定だけが1回送られる
    expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: 0 }]]);
    expect(useAppStore.getState().drivers.size).toBe(0);
  });

  it("展開図を編集すると残った指定で計算し直し、折り線でなくなった指定は捨てる", async () => {
    const view = makeHingeView(600);
    useAppStore.setState({
      doc: view.doc,
      faces: view.faces,
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

  it("収束しなかった結果は警告と収束フラグに反映される", async () => {
    const result = makeSolveResult({ "5": 90 });
    result.converged = false;
    result.frame.warnings = ["追従計算が収束していません"];
    vi.mocked(ipc.poseSolve).mockResolvedValueOnce(result);
    const view = makeHingeView(700);
    useAppStore.setState({ doc: view.doc, faces: view.faces });

    useAppStore.getState().clearDrivers();
    await flush();

    const s = useAppStore.getState();
    expect(s.poseConverged).toBe(false);
    expect(s.poseWarnings).toEqual(["追従計算が収束していません"]);
    expect(s.poseAngles.get(5)).toBe(90);
    expect(s.errorMessage).toBeNull(); // 追従計算の警告はエラーにしない
  });
});
