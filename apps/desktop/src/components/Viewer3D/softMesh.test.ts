// たわみの網(SIM-012)を表示用の頂点配列へ組み替える計算のテスト。

import { describe, expect, it } from "vitest";
import type { SoftMesh } from "../../lib/types";
import type { Vec3 } from "../../lib/layerOffset";
import { buildSoftLayout, fillSoftPositions, softSignature } from "./softMesh";

/** 面0と面1が真ん中の辺(頂点1・2)を共有する、四角2枚ぶんの網 */
function twoFaces(): SoftMesh {
  return {
    positions: [
      [0, 0, 0],
      [1, 0, 0],
      [1, 1, 0],
      [0, 1, 0],
      [2, 0, 0],
      [2, 1, 0],
    ],
    triangles: [
      [0, 1, 2],
      [0, 2, 3],
      [1, 4, 5],
      [1, 5, 2],
    ],
    triangle_faces: [0, 0, 1, 1],
    triangle_layers: [0, 0, 1, 1],
    warnings: [],
  };
}

describe("たわみの網の組み替え", () => {
  it("折り目の頂点は面ごとに複製される(層ごとに別の高さへ持ち上げるため)", () => {
    const layout = buildSoftLayout(twoFaces());
    // 面0は4頂点、面1も4頂点。共有していた2頂点が複製されて合計8になる
    expect(layout.vertexCount).toBe(8);
    expect(layout.indices.length).toBe(4 * 3);
    expect(layout.triangleFaceIds).toEqual([0, 0, 1, 1]);
    expect(layout.triangleLayers).toEqual([0, 0, 1, 1]);
    expect(layout.triangleSources).toEqual([0, 1, 2, 3]);
    // 面をまたいで同じ複製後の番号が使われることはない
    for (let i = 0; i < layout.vertexCount; i++) {
      expect([0, 1]).toContain(layout.faceOf[i]);
    }
  });

  it("境界線は面ごとの輪郭になる(面の中の分割線は引かない)", () => {
    const layout = buildSoftLayout(twoFaces());
    // 四角1枚の輪郭は4本。2枚で8本(共有する辺も面ごとに1本ずつ出る)
    expect(layout.lineIndices.length / 2).toBe(8);
    expect(layout.lineProbeIndices).toHaveLength(layout.lineIndices.length / 2);
    // 面0の対角線(頂点0-2)は2つの三角形が使うので線にならない
    const pairs = new Set<string>();
    for (let i = 0; i < layout.lineIndices.length; i += 2) {
      const a = layout.source[layout.lineIndices[i]];
      const b = layout.source[layout.lineIndices[i + 1]];
      pairs.add([a, b].sort((p, q) => p - q).join("-"));

      // 各輪郭辺のprobeは、その辺を使う唯一の三角形の第3display頂点。
      const displayA = layout.lineIndices[i];
      const displayB = layout.lineIndices[i + 1];
      const probe = layout.lineProbeIndices[i / 2];
      const adjacent = Array.from({ length: layout.indices.length / 3 }, (_, triangle) =>
        layout.indices.slice(triangle * 3, triangle * 3 + 3),
      ).filter((indices) => indices.includes(displayA) && indices.includes(displayB));
      expect(adjacent).toHaveLength(1);
      expect(adjacent[0]).toContain(probe);
      expect(probe).not.toBe(displayA);
      expect(probe).not.toBe(displayB);
    }
    expect(pairs.has("0-2")).toBe(false);
    expect(pairs.has("0-1")).toBe(true);
    // 共有する辺(1-2)は両方の面の輪郭として残る
    expect(pairs.has("1-2")).toBe(true);
  });

  it("面ごとのずらし量を足した座標を書き込む(層の重なりが見える)", () => {
    const soft = twoFaces();
    const layout = buildSoftLayout(soft);
    const lifts = new Map<number, Vec3>([
      [0, [0, 0, 0]],
      [1, [0, 0, 0.5]],
    ]);
    const out = new Float32Array(layout.vertexCount * 3);
    fillSoftPositions(soft, layout, lifts, out);
    for (let i = 0; i < layout.vertexCount; i++) {
      const src = soft.positions[layout.source[i]];
      expect(out[i * 3]).toBeCloseTo(src[0]);
      expect(out[i * 3 + 2]).toBeCloseTo(layout.faceOf[i] === 1 ? 0.5 : 0);
    }
  });

  it("参照切れの三角形があっても止まらない", () => {
    const soft = twoFaces();
    soft.triangles.push([0, 1, 99]);
    soft.triangle_faces.push(0);
    soft.triangle_layers.push(0);
    const layout = buildSoftLayout(soft);
    expect(layout.indices.length).toBe(4 * 3); // 壊れた1枚は描かない
    expect(layout.triangleLayers).toEqual([0, 0, 1, 1]);
    expect(layout.triangleSources).toEqual([0, 1, 2, 3]);
  });

  it("形が同じかどうかを短い文字列で見分けられる", () => {
    const a = twoFaces();
    const b = twoFaces();
    expect(softSignature(a)).toBe(softSignature(b));
    b.triangles.pop();
    expect(softSignature(a)).not.toBe(softSignature(b));
  });
});
