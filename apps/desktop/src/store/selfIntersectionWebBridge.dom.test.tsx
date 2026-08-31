// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { Ori3WebBridge } from "../ipc/runtime";
import type { Document, FoldAllPreviewOutcome, Frame3D } from "../lib/types";

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
  isTauri: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => tauri);

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

function documentWithDetection(): Document {
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
      penetration_prevention_enabled: true,
    },
  };
}

function penetratingOutcome(): FoldAllPreviewOutcome {
  return {
    frame: frameAt(0),
    converged: true,
    angles: {},
    iterations: 1,
    requested_percent: 0,
    requested_angles: [],
    next_warm_seed: [],
    suspect_hinges: [],
    contact_detected: true,
    flat_fold_violations: [],
    layer_order: "unavailable_without_sequence",
    self_intersection_pairs: [[0, 2]],
  };
}

beforeEach(() => {
  vi.resetModules();
  tauri.invoke.mockReset();
  tauri.isTauri.mockReset();
  tauri.isTauri.mockReturnValue(false);
  delete window.__ori3Web;
});

afterEach(() => {
  cleanup();
  delete window.__ori3Web;
});

it("Web bridgeのfold_all_preview結果をstoreとめり込みバッジへ運ぶ", async () => {
  const webInvoke = vi.fn().mockResolvedValue(penetratingOutcome());
  window.__ori3Web = {
    invoke: webInvoke as Ori3WebBridge["invoke"],
  };

  const { useAppStore, resetFoldAllPreviewRuntime, resetPoseThrottle } =
    await import("./appStore");
  const { ViewerStatusOverlays } = await import(
    "../components/ViewerStatusOverlays"
  );
  const initialStoreState = useAppStore.getState();
  const doc = documentWithDetection();

  try {
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
      selfIntersectionPairs: [],
      focusedSelfIntersectionPairIndex: 0,
      currentStep: null,
      playT: 1,
      playing: false,
      activeTool: "select",
      selection: { edgeIds: [], vertexIds: [] },
      errorMessage: null,
    });
    render(<ViewerStatusOverlays />);

    await useAppStore.getState().enterFoldAllPreview();

    expect(webInvoke).toHaveBeenCalledWith("fold_all_preview", {
      percent: 0,
      warmSeed: null,
    });
    expect(useAppStore.getState().selfIntersectionPairs).toEqual([[0, 2]]);
    expect(
      screen.getByRole("button", { name: /Face ID 0.*2/ }),
    ).toBeTruthy();
    expect(tauri.invoke).not.toHaveBeenCalled();
  } finally {
    resetPoseThrottle();
    resetFoldAllPreviewRuntime();
    useAppStore.setState(initialStoreState, true);
  }
});
