// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import type React from "react";

vi.mock("./ipc/client", () => ({}));

const bridge = vi.hoisted(() => ({
  capture: vi.fn(),
}));
vi.mock("./captureReadbackBridge", () => ({
  captureViewer3DReadback: bridge.capture,
}));

import { installCaptureApi } from "./captureApi";
import { useAppStore } from "./store/appStore";

function refs(ensureViewer3d: () => Promise<void> = async () => {}) {
  return {
    fit2d: { current: null } as React.RefObject<(() => void) | null>,
    fit3d: { current: null } as React.RefObject<(() => void) | null>,
    ensureViewer3d,
  };
}

afterEach(() => {
  delete window.__origami3Capture;
  document.documentElement.removeAttribute("data-origami3-capture-view");
  bridge.capture.mockReset();
  vi.unstubAllGlobals();
});

describe("ORIGAMI3撮影APIの生存識別", () => {
  it("同じ設置世代でheartbeatだけが進み、片付けるとAPIを外す", () => {
    document.title = "ORIGAMI3";
    const dispose = installCaptureApi(refs());
    const first = window.__origami3Capture?.getStatus();
    const second = window.__origami3Capture?.getStatus();

    expect(first).toMatchObject({
      version: 1,
      ready: true,
      heartbeat: 1,
      title: "ORIGAMI3",
      url: window.location.href,
    });
    expect(first?.generation).toBeTruthy();
    expect(second?.generation).toBe(first?.generation);
    expect(second?.heartbeat).toBe(2);

    dispose();
    expect(window.__origami3Capture).toBeUndefined();
  });

  it("再設置したAPIは前の世代と区別できる", () => {
    const disposeFirst = installCaptureApi(refs());
    const firstGeneration = window.__origami3Capture?.getStatus().generation;
    disposeFirst();

    const disposeSecond = installCaptureApi(refs());
    const secondGeneration = window.__origami3Capture?.getStatus().generation;

    expect(secondGeneration).toBeTruthy();
    expect(secondGeneration).not.toBe(firstGeneration);
    disposeSecond();
  });
});

describe("遅れて準備する3D表示と撮影APIの順序", () => {
  it("3Dの準備完了を待ってから、従来どおりpaint・fit・3paintの順で安定させる", async () => {
    const events: string[] = [];
    let resolveReady = () => {};
    const ready = new Promise<void>((resolve) => {
      resolveReady = resolve;
    });
    const ensureViewer3d = vi.fn(async () => {
      events.push("ready-start");
      await ready;
      events.push("ready-end");
    });
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      events.push("paint");
      callback(0);
      return events.length;
    });

    const captureRefs = refs(ensureViewer3d);
    captureRefs.fit2d.current = () => events.push("fit2d");
    captureRefs.fit3d.current = () => events.push("fit3d");
    const dispose = installCaptureApi(captureRefs);
    const pending = window.__origami3Capture?.setView("3d");

    expect(ensureViewer3d).toHaveBeenCalledTimes(1);
    expect(events).toEqual(["ready-start"]);
    resolveReady();
    await pending;

    expect(events).toEqual([
      "ready-start",
      "ready-end",
      "paint",
      "fit3d",
      "paint",
      "paint",
      "paint",
    ]);
    expect(captureRefs.fit2d.current).toBeTypeOf("function");
    dispose();
  });

  it("2Dだけの表示は3Dを読まず、both/normalは3D準備を待つ", async () => {
    const ensureViewer3d = vi.fn(async () => {});
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      callback(0);
      return 1;
    });
    const captureRefs = refs(ensureViewer3d);
    captureRefs.fit2d.current = vi.fn();
    captureRefs.fit3d.current = vi.fn();
    const dispose = installCaptureApi(captureRefs);

    await window.__origami3Capture?.setView("cp");
    expect(ensureViewer3d).not.toHaveBeenCalled();
    expect(captureRefs.fit2d.current).toHaveBeenCalledTimes(1);
    expect(captureRefs.fit3d.current).not.toHaveBeenCalled();

    await window.__origami3Capture?.setView("both");
    await window.__origami3Capture?.setView("normal");
    expect(ensureViewer3d).toHaveBeenCalledTimes(2);
    expect(captureRefs.fit2d.current).toHaveBeenCalledTimes(3);
    expect(captureRefs.fit3d.current).toHaveBeenCalledTimes(2);
    dispose();
  });

  it("captureCanonical3DはPromiseへ変えず、bridgeの同じreadbackを同期で返す", () => {
    const previousFrame = useAppStore.getState().frame3d;
    const expected = {
      version: 1 as const,
      width: 1,
      height: 1,
      rowOrder: "bottom-to-top" as const,
      owner: {
        encoding: "rgba8-base64" as const,
        data: "AAAAAA==",
        codeToFace: [] as const,
      },
      depth: { encoding: "rgba8-packed-depth-base64" as const, data: "AAAAAA==" },
      final: { encoding: "rgba8-base64" as const, data: "AAAAAA==" },
    };
    bridge.capture.mockReturnValue(expected);
    useAppStore.setState({ frame3d: { faces: [], warnings: [] } });
    const dispose = installCaptureApi(refs());
    try {
      const result = window.__origami3Capture?.captureCanonical3D();
      expect(result).not.toBeInstanceOf(Promise);
      expect(result?.readback).toBe(expected);
      expect(bridge.capture).toHaveBeenCalledTimes(1);
    } finally {
      dispose();
      useAppStore.setState({ frame3d: previousFrame });
    }
  });

  it("getStatus.readyとgetInteractionStateを3D準備状態へ流用しない", () => {
    const neverReady = new Promise<void>(() => {});
    const dispose = installCaptureApi(refs(() => neverReady));
    const api = window.__origami3Capture;
    const before = api?.getInteractionState();

    expect(api?.getStatus().ready).toBe(true);
    expect(before).not.toBeInstanceOf(Promise);
    expect(Object.keys(before ?? {})).toEqual([
      "version",
      "stepCount",
      "currentStep",
      "activeTool",
      "selectedEdgeCount",
      "selectedVertexCount",
      "document",
      "diagnosis",
      "pull",
      "fold",
      "technique",
    ]);
    expect(api?.getInteractionState()).toEqual(before);
    dispose();
  });
});
