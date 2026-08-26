import { describe, expect, expectTypeOf, it, vi } from "vitest";
import {
  EMPTY_VIEWER3D_INTERACTION_CAPTURE,
  captureViewer3DInteraction,
  registerViewer3DInteractionReader,
  type Viewer3DInteractionCapture,
} from "../../captureApi";

const INACTIVE_CAPTURE = {
  grab: {
    active: false,
    spatial: null,
    face: null,
    mode: null,
    selectedLayerCount: 0,
  },
  preview: {
    visible: false,
    polygonCount: 0,
    segmentCount: 0,
  },
} as const satisfies Viewer3DInteractionCapture;

describe("Viewer3D interaction capture の軽量registry", () => {
  it("指定されたreadonly型とexact nested shapeを公開し、未登録時はinactive / null / 0を返す", () => {
    expectTypeOf<Viewer3DInteractionCapture>().toEqualTypeOf<{
      readonly grab: {
        readonly active: boolean;
        readonly spatial: boolean | null;
        readonly face: number | null;
        readonly mode: "flap" | "all" | "single" | null;
        readonly selectedLayerCount: number;
      };
      readonly preview: {
        readonly visible: boolean;
        readonly polygonCount: number;
        readonly segmentCount: number;
      };
    }>();
    expectTypeOf(EMPTY_VIEWER3D_INTERACTION_CAPTURE).toEqualTypeOf<
      Viewer3DInteractionCapture
    >();

    const capture = captureViewer3DInteraction();
    expect(capture).toEqual(INACTIVE_CAPTURE);
    expect(capture).toEqual(EMPTY_VIEWER3D_INTERACTION_CAPTURE);
    expect(Object.keys(capture)).toEqual(["grab", "preview"]);
    expect(Object.keys(capture.grab)).toEqual([
      "active",
      "spatial",
      "face",
      "mode",
      "selectedLayerCount",
    ]);
    expect(Object.keys(capture.preview)).toEqual([
      "visible",
      "polygonCount",
      "segmentCount",
    ]);
  });

  it("登録だけではreaderを呼ばず、captureを読んだ時だけ1回ずつ呼ぶ", () => {
    const value = {
      grab: {
        active: true,
        spatial: false,
        face: 12,
        mode: "flap",
        selectedLayerCount: 4,
      },
      preview: {
        visible: true,
        polygonCount: 3,
        segmentCount: 5,
      },
    } as const satisfies Viewer3DInteractionCapture;
    const reader = vi.fn(() => value);
    const cleanup = registerViewer3DInteractionReader(reader);

    try {
      expect(reader).not.toHaveBeenCalled();
      expect(captureViewer3DInteraction()).toEqual(value);
      expect(reader).toHaveBeenCalledTimes(1);
      expect(captureViewer3DInteraction()).toEqual(value);
      expect(reader).toHaveBeenCalledTimes(2);
    } finally {
      cleanup();
    }

    expect(captureViewer3DInteraction()).toEqual(INACTIVE_CAPTURE);
  });

  it("古いcleanupは後から登録されたreaderを消さない", () => {
    const oldReader = vi.fn(() => INACTIVE_CAPTURE);
    const nextCapture = {
      ...INACTIVE_CAPTURE,
      grab: {
        ...INACTIVE_CAPTURE.grab,
        active: true,
        face: 8,
        mode: "all",
        selectedLayerCount: 6,
      },
    } as const satisfies Viewer3DInteractionCapture;
    const nextReader = vi.fn(() => nextCapture);
    const cleanupOld = registerViewer3DInteractionReader(oldReader);
    const cleanupNext = registerViewer3DInteractionReader(nextReader);
    let oldWasCleaned = false;

    try {
      cleanupOld();
      oldWasCleaned = true;

      expect(captureViewer3DInteraction()).toEqual(nextCapture);
      expect(oldReader).not.toHaveBeenCalled();
      expect(nextReader).toHaveBeenCalledTimes(1);
    } finally {
      if (!oldWasCleaned) cleanupOld();
      cleanupNext();
    }

    expect(captureViewer3DInteraction()).toEqual(INACTIVE_CAPTURE);
  });

  it("selectedLayerCountは通常grabで実際に選ばれた層数であり、完全折りのひだ数ではない", () => {
    const selectedLayerCount = 5;
    const fullyFoldedPleatCount = 2;
    const value = {
      grab: {
        active: true,
        spatial: true,
        face: 17,
        mode: "single",
        selectedLayerCount,
      },
      preview: {
        visible: true,
        polygonCount: 5,
        segmentCount: 8,
      },
    } as const satisfies Viewer3DInteractionCapture;
    const cleanup = registerViewer3DInteractionReader(() => value);

    try {
      const captured = captureViewer3DInteraction();
      expect(captured.grab.selectedLayerCount).toBe(selectedLayerCount);
      expect(captured.grab.selectedLayerCount).not.toBe(fullyFoldedPleatCount);
      expect("pleatCount" in captured.grab).toBe(false);
    } finally {
      cleanup();
    }
  });
});
