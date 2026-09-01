import { describe, expect, it } from "vitest";
import * as THREE from "three";

import type { Face, Frame3D } from "../../lib/types";
import {
  deriveViewer3DInteractionCapture,
  spatialFoldPreviewPlan,
  type GrabState,
} from "./viewerHighlight";

/**
 * ori3-layers::spatial_fold::tests::disconnected_ninety_degree_fixture と同じ形。
 * 中央ヒンジで90度に起こした4本のstripを、中央ヒンジより手前の平面で切る。
 * 全4面が動く半空間へ届く一方、中央ヒンジは届かないためflapだけ2面になる。
 */
const FACES: Face[] = [
  { id: 0, vertices: [0, 1, 2, 3], edges: [100, 10, 101, 102] },
  { id: 1, vertices: [1, 4, 5, 2], edges: [103, 11, 104, 10] },
  { id: 2, vertices: [4, 6, 7, 5], edges: [105, 12, 106, 11] },
  { id: 3, vertices: [6, 8, 9, 7], edges: [107, 108, 109, 12] },
];

const FRAME: Frame3D = {
  faces: [
    {
      face: 0,
      polygon: [
        [0, 0, 0],
        [0.25, 0, 0],
        [0.25, 1, 0],
        [0, 1, 0],
      ],
      layer: 0,
    },
    {
      face: 1,
      polygon: [
        [0.25, 0, 0],
        [0.5, 0, 0],
        [0.5, 1, 0],
        [0.25, 1, 0],
      ],
      layer: 1,
    },
    {
      face: 2,
      polygon: [
        [0.5, 0, 0],
        [0.5, 0, 0.25],
        [0.5, 1, 0.25],
        [0.5, 1, 0],
      ],
      layer: 2,
    },
    {
      face: 3,
      polygon: [
        [0.5, 0, 0.25],
        [0.5, 0, 0.5],
        [0.5, 1, 0.5],
        [0.5, 1, 0.25],
      ],
      layer: 3,
    },
  ],
  warnings: [],
};

function spatialGrab(mode: "single" | "flap" | "all"): Extract<GrabState, { spatial: true }> {
  return {
    spatial: true,
    face: 2,
    mode,
    a: [0.5, 0.5, 0.125],
    b: [0.5625, 0.5, 0.0625],
    origin: new THREE.Vector3(0.5, 0.5, 0.125),
    ndc: new THREE.Vector3(),
    x: 0,
    y: 0,
    direction: "Up",
  };
}

describe("90度姿勢の空間grab下見", () => {
  it.each([
    { mode: "single" as const, expectedFaces: [2] },
    { mode: "flap" as const, expectedFaces: [2, 3] },
    { mode: "all" as const, expectedFaces: [0, 1, 2, 3] },
  ])("$mode は実操作と同じ対象面だけを光らせ、その面数を表示する", ({
    mode,
    expectedFaces,
  }) => {
    const grab = spatialGrab(mode);
    const preview = spatialFoldPreviewPlan(FRAME, FACES, grab);
    const capture = deriveViewer3DInteractionCapture({
      grab,
      frame: FRAME,
      doc: null,
      faces: FACES,
    });

    expect(preview.faces.map((face) => face.face)).toEqual(expectedFaces);
    expect(
      preview.faces.every((face) => face.segments.some((segment) => segment.role === "active")),
    ).toBe(true);
    expect(capture.grab.selectedLayerCount).toBe(preview.faces.length);
    expect(capture.preview.segmentCount).toBe(preview.segments.length);
  });
});
