// 骨格の編集(PRO-001)の純関数テスト:
// 本数の増減が上下限で止まること、長さ・太さの書き換え、プレビュー配置。

import { describe, expect, it } from "vitest";
import {
  MAX_LIMBS,
  MIN_LIMBS,
  addLimb,
  defaultSkeleton,
  limbLabel,
  limbs,
  previewLayout,
  removeLimb,
  setLimb,
} from "./skeleton";

describe("骨格の編集", () => {
  it("初期状態は根1つ+出っぱり4本", () => {
    const s = defaultSkeleton();
    expect(limbs(s)).toHaveLength(4);
    expect(s.nodes.filter((n) => n.parent === null)).toHaveLength(1);
  });

  it("増やすと本数が1つ増え、IDは重ならない", () => {
    const s = addLimb(defaultSkeleton());
    expect(limbs(s)).toHaveLength(5);
    expect(new Set(s.nodes.map((n) => n.id)).size).toBe(s.nodes.length);
  });

  it("上限12本を超えて増えない", () => {
    let s = defaultSkeleton();
    for (let i = 0; i < 20; i++) s = addLimb(s);
    expect(limbs(s)).toHaveLength(MAX_LIMBS);
  });

  it("減らすと本数が1つ減り、下限1本で止まる", () => {
    let s = defaultSkeleton();
    s = removeLimb(s, limbs(s)[0].id);
    expect(limbs(s)).toHaveLength(3);
    for (let i = 0; i < 10; i++) s = removeLimb(s, limbs(s)[0].id);
    expect(limbs(s)).toHaveLength(MIN_LIMBS);
    // 根は消さない(消すとRust側の検査で「根がちょうど1つ」に反する)
    expect(s.nodes.filter((n) => n.parent === null)).toHaveLength(1);
  });

  it("根は削除できない", () => {
    const s = defaultSkeleton();
    expect(removeLimb(s, 0)).toBe(s);
  });

  it("長さと太さを書き換えられる(他の出っぱりは変わらない)", () => {
    const s0 = defaultSkeleton();
    const id = limbs(s0)[1].id;
    const s = setLimb(s0, id, { length: 2.5, width_factor: 0.4 });
    const changed = s.nodes.find((n) => n.id === id);
    expect(changed?.length).toBe(2.5);
    expect(changed?.width_factor).toBe(0.4);
    expect(limbs(s)[0]).toEqual(limbs(s0)[0]);
  });

  it("プレビューは根から放射状に並び、太いほど先端の丸が大きい", () => {
    const s = setLimb(defaultSkeleton(), limbs(defaultSkeleton())[0].id, {
      width_factor: 2,
    });
    const layout = previewLayout(s);
    expect(layout).toHaveLength(4);
    // 先頭は真上(y>0, x≒0)
    expect(layout[0].end[1]).toBeGreaterThan(0);
    expect(Math.abs(layout[0].end[0])).toBeLessThan(1e-9);
    expect(layout[0].radius).toBeGreaterThan(layout[1].radius);
  });

  it("呼び名は専門用語を使わない", () => {
    expect(limbLabel(0)).toBe("頭");
    expect(limbLabel(11)).toBe("出っぱり12");
  });
});
