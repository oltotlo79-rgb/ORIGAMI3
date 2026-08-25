// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

vi.mock("../ipc/client", () => ({
  documentSave: vi.fn(),
  foldAllPreview: vi.fn(),
  poseSolve: vi.fn(),
  sequenceReplay: vi.fn(),
}));

import * as ipc from "../ipc/client";
import type {
  Document,
  FoldAllPreviewOutcome,
  Frame3D,
} from "../lib/types";
import {
  resetFoldAllPreviewRuntime,
  resetPoseThrottle,
  useAppStore,
} from "../store/appStore";
import { ContextPanel } from "./ContextPanel";

function frameAt(percent: number): Frame3D {
  return {
    faces: [
      {
        face: 0,
        polygon: [
          [0, 0, percent],
          [1, 0, percent],
          [0, 1, percent],
        ],
        layer: 0,
        surface_rank: 0,
        mirrored: false,
      },
    ],
    warnings: [],
  };
}

function outcome(
  percent: number,
  patch: Partial<FoldAllPreviewOutcome> = {},
): FoldAllPreviewOutcome {
  return {
    frame: frameAt(percent),
    converged: true,
    angles: { "5": percent * 1.8 },
    iterations: 1,
    requested_percent: percent,
    requested_angles: [{ hinge: 5, target_angle_deg: percent * 1.8 }],
    next_warm_seed: [{ hinge: 5, target_angle_deg: percent * 1.8 }],
    suspect_hinges: [],
    contact_detected: false,
    flat_fold_violations: [],
    layer_order: "unavailable_without_sequence",
    ...patch,
  };
}

function makeDocument(): Document {
  return {
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
        { id: 5, v0: 0, v1: 2, kind: "Mountain" },
      ],
      next_vertex_id: 4,
      next_edge_id: 6,
    },
    sequence: [
      {
        id: 8,
        kind: "Simple",
        drivers: [{ a: [0, 0], b: [1, 1], target_angle_deg: 45 }],
        layer_order: null,
        note: "元の手順",
      },
    ],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
}

function seed() {
  useAppStore.setState({
    doc: makeDocument(),
    docEpoch: 4,
    faces: [
      { id: 0, vertices: [0, 1, 2], edges: [0, 1, 5] },
      { id: 1, vertices: [0, 2, 3], edges: [5, 2, 3] },
    ],
    hinges: new Set([5]),
    frame3d: frameAt(-1),
    foldAllPreview: null,
    selection: { edgeIds: [], vertexIds: [] },
    activeTool: "select",
    currentStep: null,
    playT: 1,
    playing: false,
    foldDraft: null,
    pendingFoldThrough: null,
    foldThroughBusy: false,
    alignDraft: null,
    techniqueDraft: null,
    drivers: new Map(),
    pinnedFolds: new Map(),
    sequenceTargets: new Map([[5, 45]]),
    poseAngles: new Map([[5, 45]]),
    relaxations: [],
    warnings: [],
    poseWarnings: [],
    replayWarnings: [],
    flatFoldViolations: [],
    errorMessage: null,
    documentSavedPath: null,
    mirrorAxisNotice: null,
    contextHelpExpanded: false,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  resetPoseThrottle();
  resetFoldAllPreviewRuntime();
  seed();
  vi.mocked(ipc.foldAllPreview).mockImplementation(async (percent) =>
    outcome(percent),
  );
  vi.mocked(ipc.sequenceReplay).mockResolvedValue({
    frame: frameAt(-1),
    skipped: [],
    warnings: [],
    sequence_targets: [{ hinge: 5, target_angle_deg: 45 }],
    angles: { "5": 45 },
    converged: true,
  });
  vi.mocked(ipc.poseSolve).mockResolvedValue({
    frame: frameAt(0),
    converged: true,
    angles: { "5": 0 },
    iterations: 1,
  });
});

afterEach(() => {
  cleanup();
  resetFoldAllPreviewRuntime();
  useAppStore.setState({ doc: null, frame3d: null, foldAllPreview: null });
});

async function enter() {
  fireEvent.click(
    screen.getByRole("button", { name: /全部いっぺんに折ってみる/ }),
  );
  await screen.findByText("これは仮の形です");
  await waitFor(() =>
    expect(
      document.querySelector("[data-fold-all-active]")?.getAttribute(
        "data-applied-percent",
      ),
    ).toBe("0"),
  );
}

describe("全部いっぺんに折ってみる画面", () => {
  it("既存パネルの入口から0〜100%のつまみと記録でない約束を常時表示する", async () => {
    render(<ContextPanel />);

    await enter();

    expect(document.querySelectorAll("#context-panel")).toHaveLength(1);
    const slider = screen.getByRole("slider", {
      name: "全部の折り目を動かす割合",
    });
    expect(slider).toHaveProperty("min", "0");
    expect(slider).toHaveProperty("max", "100");
    expect(slider).toHaveProperty("value", "0");
    expect(screen.getByText("元に戻る 0%")).toBeTruthy();
    expect(screen.getByText("できるところまで 100%")).toBeTruthy();
    expect(
      screen.getByText(
        "山折りと谷折りを同じ割合で動かして、形だけを見ます。手順には記録されません。",
      ),
    ).toBeTruthy();
    expect(
      screen.getByText(
        "紙を順番に折った形ではないため、どの紙が上になるかは決まっていません。",
      ),
    ).toBeTruthy();
  });

  it("不収束・平坦条件・貫通を知らせてもつまみを止めない", async () => {
    vi.mocked(ipc.foldAllPreview).mockImplementation(async (percent) =>
      outcome(percent, {
        converged: false,
        best_effort: true,
        relaxations: [
          {
            hinge: 5,
            target_angle_deg: 90,
            actual_angle_deg: 70,
            delta_deg: -20,
          },
        ],
        flat_fold_violations: [2],
        suspect_hinges: [5],
        contact_detected: true,
      }),
    );
    render(<ContextPanel />);
    await enter();

    const slider = screen.getByRole("slider", {
      name: "全部の折り目を動かす割合",
    });
    expect(slider).toHaveProperty("disabled", false);
    expect(screen.getByText(/形を最後まで合わせきれませんでした/)).toBeTruthy();
    expect(screen.getByText(/全部の折り目を同じ割合にできないため/)).toBeTruthy();
    expect(screen.getByText(/平らにたためない場所があります/)).toBeTruthy();
    expect(screen.getByText(/紙が突き抜けているところがあります/)).toBeTruthy();

    fireEvent.change(slider, { target: { value: "75" } });
    fireEvent.pointerUp(slider);
    await waitFor(() =>
      expect(
        document.querySelector("[data-fold-all-active]")?.getAttribute(
          "data-applied-percent",
        ),
      ).toBe("75"),
    );
    expect(slider).toHaveProperty("disabled", false);
  });

  it("いつもの表示へ戻ると専用表示を閉じて元の手順位置を再計算する", async () => {
    render(<ContextPanel />);
    await enter();

    fireEvent.click(screen.getByRole("button", { name: "いつもの表示に戻る" }));

    await waitFor(() =>
      expect(screen.queryByText("これは仮の形です")).toBeNull(),
    );
    expect(ipc.sequenceReplay).toHaveBeenCalledWith(1, 1, null);
    expect(
      screen.getByRole("button", { name: /全部いっぺんに折ってみる/ }),
    ).toBeTruthy();
  });

  it("50%から0%へ戻して操作を終えると、いつもの表示へ戻る", async () => {
    render(<ContextPanel />);
    await enter();
    const slider = screen.getByRole("slider", {
      name: "全部の折り目を動かす割合",
    });

    fireEvent.change(slider, { target: { value: "50" } });
    fireEvent.pointerUp(slider);
    await waitFor(() =>
      expect(
        document.querySelector("[data-fold-all-active]")?.getAttribute(
          "data-applied-percent",
        ),
      ).toBe("50"),
    );

    fireEvent.change(slider, { target: { value: "0" } });
    fireEvent.pointerUp(slider);

    await waitFor(() =>
      expect(screen.queryByText("これは仮の形です")).toBeNull(),
    );
    expect(ipc.sequenceReplay).toHaveBeenCalledWith(1, 1, null);
    expect(
      screen.getByRole("button", { name: /全部いっぺんに折ってみる/ }),
    ).toBeTruthy();
  });

  it("専用表示の本文・読み上げ名・説明に内部向けの語を出さない", async () => {
    useAppStore.setState({
      poseWarnings: ["ソルバーの古い通知"],
      replayWarnings: ["剛体の古い通知"],
    });
    render(<ContextPanel />);
    await enter();

    const panel = document.querySelector("#context-panel") as HTMLElement;
    const visible = [panel.textContent ?? ""];
    for (const element of panel.querySelectorAll("[aria-label], [data-tooltip], [title]")) {
      visible.push(
        element.getAttribute("aria-label") ?? "",
        element.getAttribute("data-tooltip") ?? "",
        element.getAttribute("title") ?? "",
      );
    }
    const text = visible.join("\n").toLowerCase();
    for (const forbidden of [
      "ソルバー",
      "剛体",
      "シミュレーション",
      "探索",
      "プレビュー",
      "layer_order",
      "preview",
      "solver",
      "target",
    ]) {
      expect(text).not.toContain(forbidden.toLowerCase());
    }
  });

  it("通常形を作り直している間も約束表示を残し、終わるまで操作を切り替えない", async () => {
    render(<ContextPanel />);
    await enter();
    let release!: (value: Awaited<ReturnType<typeof ipc.sequenceReplay>>) => void;
    vi.mocked(ipc.sequenceReplay).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );

    fireEvent.click(screen.getByRole("button", { name: "いつもの表示に戻る" }));
    await waitFor(() =>
      expect(
        document.querySelector("[data-fold-all-active]")?.getAttribute("data-returning"),
      ).toBe("true"),
    );
    expect(screen.getByText("これは仮の形です")).toBeTruthy();
    expect(screen.getByRole("slider")).toHaveProperty("disabled", true);

    release({
      frame: frameAt(-1),
      skipped: [],
      warnings: [],
      sequence_targets: [{ hinge: 5, target_angle_deg: 45 }],
      angles: { "5": 45 },
      converged: true,
    });
    await waitFor(() =>
      expect(screen.queryByText("これは仮の形です")).toBeNull(),
    );
  });

  it("いつもの表示へ戻せなくても約束表示を残し、つまみを再び動かせる", async () => {
    render(<ContextPanel />);
    await enter();
    vi.mocked(ipc.sequenceReplay).mockRejectedValueOnce(new Error("replay failed"));

    fireEvent.click(screen.getByRole("button", { name: "いつもの表示に戻る" }));

    expect(
      await screen.findByText(
        "いつもの表示へ戻せませんでした。仮の形を表示したままです。",
      ),
    ).toBeTruthy();
    expect(screen.getByText("これは仮の形です")).toBeTruthy();
    expect(screen.getByRole("slider")).toHaveProperty("disabled", false);
  });

  it("専用表示のまま保存したとき、いまの形は保存されないと知らせる", async () => {
    vi.mocked(ipc.documentSave).mockResolvedValue(undefined);
    render(<ContextPanel />);
    await enter();

    await useAppStore.getState().saveDocument("C:\\work\\sample.ori3");

    expect(
      await screen.findByText("作品を保存しました。いま見ている形は保存されません。"),
    ).toBeTruthy();
    expect(screen.getByText("これは仮の形です")).toBeTruthy();
  });

  it("入力からReact反映まで10回の平均・最大が33ms以内", async () => {
    render(<ContextPanel />);
    await enter();
    const slider = screen.getByRole("slider", {
      name: "全部の折り目を動かす割合",
    });
    const active = document.querySelector("[data-fold-all-active]") as HTMLElement;
    const durations: number[] = [];

    for (const percent of [10, 20, 30, 40, 50, 60, 70, 80, 90, 100]) {
      const changed = new Promise<void>((resolve) => {
        const observer = new MutationObserver(() => {
          if (active.getAttribute("data-applied-percent") === String(percent)) {
            observer.disconnect();
            resolve();
          }
        });
        observer.observe(active, { attributes: true });
      });
      const started = performance.now();
      fireEvent.change(slider, { target: { value: String(percent) } });
      await changed;
      durations.push(performance.now() - started);
    }

    const average = durations.reduce((sum, value) => sum + value, 0) / durations.length;
    const maximum = Math.max(...durations);
    console.info(
      `fold-all frontend input-to-React: average=${average.toFixed(3)}ms max=${maximum.toFixed(3)}ms`,
    );
    expect(average).toBeLessThanOrEqual(33);
    expect(maximum).toBeLessThanOrEqual(33);
  });
});
