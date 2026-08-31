// @vitest-environment jsdom

import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/client", () => ({
  documentExport: vi.fn(),
  documentSave: vi.fn(),
  documentNew: vi.fn(),
  documentOpen: vi.fn(),
  editApply: vi.fn(),
  editRedo: vi.fn(),
  editUndo: vi.fn(),
  foldAllPreview: vi.fn(),
  poseSolve: vi.fn(),
  sequenceApply: vi.fn(),
  sequenceReplay: vi.fn(),
}));

import { ViewerStatusOverlays } from "../components/ViewerStatusOverlays";
import * as ipc from "../ipc/client";
import type {
  Document,
  FoldAllPreviewOutcome,
  Frame3D,
  SelfIntersectionPair,
} from "../lib/types";
import {
  resetFoldAllPreviewRuntime,
  resetPoseThrottle,
  useAppStore,
} from "./appStore";

const initialStoreState = useAppStore.getState();

function frameAt(marker: number): Frame3D {
  return {
    faces: [
      {
        face: 0,
        polygon: [
          [0, 0, marker],
          [1, 0, marker],
          [0, 1, marker],
        ],
        layer: 0,
        surface_rank: 0,
      },
    ],
    warnings: [],
  };
}

function documentWithDetection(enabled: boolean): Document {
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
    sequence: [],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
      penetration_prevention_enabled: enabled,
    },
  };
}

function outcome(
  pairs?: SelfIntersectionPair[],
): FoldAllPreviewOutcome {
  return {
    frame: frameAt(0),
    converged: true,
    angles: {},
    iterations: 1,
    requested_percent: 0,
    requested_angles: [],
    next_warm_seed: [],
    suspect_hinges: [],
    contact_detected: pairs !== undefined && pairs.length > 0,
    flat_fold_violations: [],
    layer_order: "unavailable_without_sequence",
    ...(pairs === undefined ? {} : { self_intersection_pairs: pairs }),
  };
}

function seed(
  detectionEnabled: boolean,
  pairs: readonly SelfIntersectionPair[] = [],
): void {
  const doc = documentWithDetection(detectionEnabled);
  useAppStore.setState({
    doc,
    display: doc.display,
    docEpoch: 73,
    faces: [
      { id: 0, vertices: [0, 1, 2], edges: [0, 1, 5] },
      { id: 2, vertices: [0, 2, 3], edges: [5, 2, 3] },
    ],
    hinges: new Set([5]),
    frame3d: frameAt(-1),
    foldAllPreview: null,
    selfIntersectionPairs: pairs,
    focusedSelfIntersectionPairIndex: 0,
    currentStep: null,
    playT: 1,
    playing: false,
    activeTool: "select",
    selection: { edgeIds: [], vertexIds: [] },
    errorMessage: null,
  });
}

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
} {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

beforeEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
  vi.clearAllMocks();
  resetPoseThrottle();
  resetFoldAllPreviewRuntime();
});

afterEach(() => {
  cleanup();
  resetFoldAllPreviewRuntime();
  useAppStore.setState(initialStoreState, true);
});

describe("一斉折りとめり込みバッジ", () => {
  it("検出設定OFFは入口から旧ペアを消し、応答後もバッジを出さない", async () => {
    const pending = deferred<FoldAllPreviewOutcome>();
    vi.mocked(ipc.foldAllPreview).mockReturnValueOnce(pending.promise);
    seed(false, [[0, 2]]);
    render(<ViewerStatusOverlays />);

    expect(
      screen.queryByRole("button", { name: /紙のめり込み/ }),
    ).toBeNull();
    const entering = useAppStore.getState().enterFoldAllPreview();
    await waitFor(() => expect(ipc.foldAllPreview).toHaveBeenCalledTimes(1));
    expect(useAppStore.getState().selfIntersectionPairs).toEqual([]);
    expect(
      screen.queryByRole("button", { name: /紙のめり込み/ }),
    ).toBeNull();

    pending.resolve(outcome());
    await entering;
    expect(useAppStore.getState().selfIntersectionPairs).toEqual([]);
    expect(
      screen.queryByRole("button", { name: /紙のめり込み/ }),
    ).toBeNull();
  });

  it("検出設定ONは計算済み面ペアを採用してバッジへ出す", async () => {
    vi.mocked(ipc.foldAllPreview).mockResolvedValueOnce(
      outcome([[0, 2]]),
    );
    seed(true);
    render(<ViewerStatusOverlays />);

    await useAppStore.getState().enterFoldAllPreview();

    expect(useAppStore.getState().selfIntersectionPairs).toEqual([[0, 2]]);
    expect(
      screen.getByRole("button", {
        name: /紙のめり込み 1組（1\/1、Face ID 0 ↔ 2）/,
      }),
    ).toBeTruthy();
  });
});
