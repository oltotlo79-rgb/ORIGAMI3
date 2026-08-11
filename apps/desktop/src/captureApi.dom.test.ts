// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import type React from "react";

vi.mock("./ipc/client", () => ({}));

import { installCaptureApi } from "./captureApi";

function refs() {
  return {
    fit2d: { current: null } as React.RefObject<(() => void) | null>,
    fit3d: { current: null } as React.RefObject<(() => void) | null>,
  };
}

afterEach(() => {
  delete window.__origami3Capture;
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
