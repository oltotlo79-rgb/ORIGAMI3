// appStoreの直列化まわりのテスト: 「成功したviewはisLatestに関わらず破棄されない」
// ことを固定する(A成功→B失敗でも、画面はAのdocを保持しバックエンドと一致する)。

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DocumentView } from "../lib/types";

vi.mock("../ipc/client", () => ({
  documentNew: vi.fn(),
  documentOpen: vi.fn(),
  documentSave: vi.fn(),
  editApply: vi.fn(),
  editUndo: vi.fn(),
  editRedo: vi.fn(),
  sequenceApply: vi.fn(),
}));

import * as ipc from "../ipc/client";
import { useAppStore } from "./appStore";

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

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({
    doc: null,
    faces: [],
    warnings: [],
    violations: [],
    selection: { edgeIds: [], vertexIds: [] },
    errorMessage: null,
    docEpoch: 0,
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
