import { describe, expect, it } from "vitest";
import {
  captureViewer3DReadback,
  registerViewer3DReadback,
  waitForViewer3DReady,
  type Viewer3DReadback,
} from "./captureReadbackBridge";

function readback(marker: number): Viewer3DReadback {
  const data = btoa(String.fromCharCode(marker));
  return {
    version: 1,
    width: 1,
    height: 1,
    rowOrder: "bottom-to-top",
    owner: {
      encoding: "rgba8-base64",
      data,
      codeToFace: [[marker, marker]],
    },
    depth: { encoding: "rgba8-packed-depth-base64", data },
    final: { encoding: "rgba8-base64", data },
  };
}

describe("3D撮影の軽量bridge", () => {
  it("scene未登録では、従来と同じ理由で同期captureを拒否する", () => {
    expect(() => captureViewer3DReadback()).toThrow(
      "3D表示の描画資源がまだ用意されていません",
    );
  });

  it("登録したsourceの同じreadbackを、同期のまま返す", () => {
    const expected = readback(7);
    const unregister = registerViewer3DReadback(() => expected);
    try {
      expect(captureViewer3DReadback()).toBe(expected);
    } finally {
      unregister();
    }
  });

  it("古いsceneのcleanupは、後から登録したsceneを消さない", () => {
    const first = readback(1);
    const second = readback(2);
    const unregisterFirst = registerViewer3DReadback(() => first);
    const unregisterSecond = registerViewer3DReadback(() => second);

    unregisterFirst();
    expect(captureViewer3DReadback()).toBe(second);

    unregisterSecond();
    expect(() => captureViewer3DReadback()).toThrow(
      "3D表示の描画資源がまだ用意されていません",
    );
  });

  it("準備待ちは登録前には解決せず、source登録後だけ解決する", async () => {
    let resolved = false;
    const ready = waitForViewer3DReady().then(() => {
      resolved = true;
    });
    await Promise.resolve();
    expect(resolved).toBe(false);

    const unregister = registerViewer3DReadback(() => readback(9));
    try {
      await ready;
      expect(resolved).toBe(true);
    } finally {
      unregister();
    }
  });
});
