// 実データ(折り鶴・カエル)での対称軸の検証(UI-007)。
//
// 展開図は `crates/ori3-layers/tests/acceptance_crane.rs` /
// `acceptance_frog.rs` が折り操作の列だけで折り上げた**完成形そのもの**で、
// 同テストが `__fixtures__/*.json` へ書き出したものを読む(合成した展開図では
// 対称軸の決め方を確かめられないため)。
//
// 折り鶴では、羽をつかんで引くときに使われる折り線(羽の面から根の面までの
// 経路にある折り線)が、左右で1本ずつ対応する。ここではその対応を確かめる。

import { describe, expect, it } from "vitest";
import craneJson from "./__fixtures__/crane.json";
import frogJson from "./__fixtures__/frog.json";
import { faceParents, mirrorHingeOf } from "./grabDrive";
import type { Document, Edge, Face, Paper, Vertex } from "./types";

interface Fixture {
  paper: Paper;
  vertices: Vertex[];
  edges: Edge[];
  faces: Face[];
}

const FIXTURES: Record<string, unknown> = { crane: craneJson, frog: frogJson };

function load(name: string): { doc: Document; faces: Face[] } {
  const fx = FIXTURES[name] as Fixture;
  return {
    doc: {
      schema_version: 1,
      paper: fx.paper,
      cp: {
        vertices: fx.vertices,
        edges: fx.edges,
        next_vertex_id: fx.vertices.length,
        next_edge_id: fx.edges.length,
      },
      sequence: [],
      display: { front_color: [1, 1, 1], back_color: [1, 1, 1], grid_divisions: 8 },
    },
    faces: fx.faces,
  };
}

/** その面をつかんで引くときに使われる折り線(根の面までの経路) */
function driveHinges(faces: Face[], faceId: number): number[] {
  const parents = faceParents(faces);
  const out: number[] = [];
  for (let cur = faceId, link = parents.get(cur); link; link = parents.get(cur)) {
    out.push(link.hinge);
    cur = link.parent;
  }
  return out;
}

/** 展開図の四角い範囲にすっぽり入っている面(=紙の1/4=1枚の羽) */
function facesInQuarter(doc: Document, faces: Face[], b: number[]): number[] {
  const pos = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  return faces
    .filter((f) =>
      f.vertices.every((v) => {
        const p = pos.get(v);
        return (
          p !== undefined &&
          p[0] >= b[0] - 1e-9 &&
          p[0] <= b[2] + 1e-9 &&
          p[1] >= b[1] - 1e-9 &&
          p[1] <= b[3] + 1e-9
        );
      }),
    )
    .map((f) => f.id);
}

describe("折り鶴(実データ)", () => {
  const { doc, faces } = load("crane");

  it("左右の羽をつかむときの折り線が1本ずつ対になる", () => {
    // 羽は紙の1/4ずつ。B=右下の1/4、D=左上の1/4(残りの2隅から首と尾が出る)
    const wingB = facesInQuarter(doc, faces, [0.5, 0, 1, 0.5]);
    const wingD = facesInQuarter(doc, faces, [0, 0.5, 0.5, 1]);
    expect(wingB.length).toBeGreaterThan(0);
    expect(wingD.length).toBe(wingB.length);
    const hingesB = new Set(wingB.flatMap((f) => driveHinges(faces, f)));
    const hingesD = new Set(wingD.flatMap((f) => driveHinges(faces, f)));
    expect(hingesB.size).toBe(6);
    // 羽Bの経路の折り線は、1本残らず羽Dの経路の折り線と対になる
    for (const h of hingesB) {
      const other = mirrorHingeOf(doc, faces, h);
      expect(hingesD.has(other as number), `辺${h}の相手 ${other}`).toBe(true);
      expect(mirrorHingeOf(doc, faces, other as number)).toBe(h);
    }
  });
});

describe("伝承のカエル(実データ)", () => {
  const { doc, faces } = load("frog");

  /** 紙の隅を頂点に持つ面(=足の先)をつかむときに使われる折り線 */
  function legHinges(corner: number[]): Set<number> {
    const pos = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
    const tips = faces.filter((f) =>
      f.vertices.some((v) => {
        const p = pos.get(v);
        return p !== undefined && Math.hypot(p[0] - corner[0], p[1] - corner[1]) < 1e-9;
      }),
    );
    expect(tips.length).toBeGreaterThan(0);
    return new Set(tips.flatMap((f) => driveHinges(faces, f.id)));
  }

  it("左右の足をつかむときの折り線が対になる", () => {
    // 足は紙の4隅から出る。紙の縦の中心線をはさんで (0,0) と (1,0) が左右の対
    const left = legHinges([0, 0]);
    const right = legHinges([1, 0]);
    // 胴を通る共通の折り線は左右の区別が付かないので、片側だけのものを見る
    const own = [...left].filter((h) => !right.has(h));
    let matched = 0;
    for (const h of own) {
      const other = mirrorHingeOf(doc, faces, h);
      if (other === null) continue; // 段折りなど片側にしかない折り目
      matched++;
      expect(right.has(other), `辺${h}の相手 ${other} は右足側`).toBe(true);
      expect(mirrorHingeOf(doc, faces, other)).toBe(h);
    }
    expect(matched).toBeGreaterThanOrEqual(20);
  });

  it("対称な相手が無い折り線もある(見つからないものは1本だけ動かす)", () => {
    // 体の根元の段折りは左右の対にならない。誤った相手を返さずnullになること
    const lonely = [...legHinges([0, 0])].filter(
      (h) => mirrorHingeOf(doc, faces, h) === null,
    );
    expect(lonely.length).toBeGreaterThan(0);
  });
});
