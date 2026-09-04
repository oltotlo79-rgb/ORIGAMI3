// 実データ(折り鶴・カエル)での対称軸の検証(UI-007)。
//
// 展開図は `crates/ori3-layers/tests/acceptance_crane.rs` /
// `acceptance_frog.rs` が折り操作の列だけで折り上げた**完成形そのもの**で、
// 同テストが `__fixtures__/*.json` へ書き出したものを読む(合成した展開図では
// 対称軸の決め方を確かめられないため)。
//
// 折り鶴の正本CPが持つ鏡は反対角線 (x,y)->(1-y,1-x) で、頭の角(0,0)と尾の角(1,1)を
// 入れ替える(2枚の羽はそれぞれ自分自身へ写るので、羽どうしは対にならない)。
// ここでは尾側の面と折り線が、その鏡をはさんで頭側に相手を持つことを確かめる。

import { describe, expect, it } from "vitest";
import craneJson from "./__fixtures__/crane.json";
import frogJson from "./__fixtures__/frog.json";
import { faceParents, mirrorHingeOf } from "./grabDrive";
import type { Document, Edge, Face, Paper, Vec2, Vertex } from "./types";

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

/** 反対角線 y=1-x をはさんだ鏡像。折り鶴の正本CPが実際に持っている対称。 */
function mirrorAcrossAntiDiagonal(p: Vec2): Vec2 {
  return [1 - p[1], 1 - p[0]];
}

/** 展開図の面を、頂点の位置の列にする */
function polygonOf(doc: Document, face: Face): Vec2[] {
  const pos = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  return face.vertices.map((v) => pos.get(v) as Vec2);
}

/** 多角形の面積(向きに依らない) */
function polygonArea(p: Vec2[]): number {
  let sum = 0;
  for (let i = 0; i < p.length; i += 1) {
    const a = p[i];
    const b = p[(i + 1) % p.length];
    sum += a[0] * b[1] - b[0] * a[1];
  }
  return Math.abs(sum) / 2;
}

/** 多角形の重心 */
function polygonCentroid(p: Vec2[]): Vec2 {
  let twice = 0;
  let x = 0;
  let y = 0;
  for (let i = 0; i < p.length; i += 1) {
    const a = p[i];
    const b = p[(i + 1) % p.length];
    const cross = a[0] * b[1] - b[0] * a[1];
    twice += cross;
    x += (a[0] + b[0]) * cross;
    y += (a[1] + b[1]) * cross;
  }
  return [x / (3 * twice), y / (3 * twice)];
}

/** 点が多角形の内側にあるか(半直線と辺の交差数で判定) */
function pointInPolygon(pt: Vec2, p: Vec2[]): boolean {
  let inside = false;
  for (let i = 0; i < p.length; i += 1) {
    const a = p[i];
    const b = p[(i + 1) % p.length];
    if (a[1] > pt[1] !== b[1] > pt[1]) {
      const cut = ((b[0] - a[0]) * (pt[1] - a[1])) / (b[1] - a[1]) + a[0];
      if (pt[0] < cut) inside = !inside;
    }
  }
  return inside;
}

describe("折り鶴(実データ)", () => {
  const { doc, faces } = load("crane");

  // 2026-09-04に、旧11手の台本の鶴から正本CP(56頂点・114辺・59面)の粗い3手の鶴へ差し替えた。
  // 旧の主張「左右の羽をつかむときの折り線が1本ずつ対になる」は正本の鶴では成り立たない。
  // 正本の鶴の2枚の羽は鏡像でないからで、畳んだ形の羽の長さも羽B 0.3306769613787994 /
  // 羽D 0.3082014620276746 と違う(`crates/ori3-layers/tests/acceptance_crane.rs` の
  // `crane_is_folded_only_with_fold_operations` の実測)。展開図でも、2枚の羽を入れ替える写像
  // (主対角線 (x,y)->(y,x)) について正本CPは対称でなく、角(0,1)にある折り線8本
  // (辺91〜96,111,112。頂点v47〜v55)に、角(1,0)側の相手が無い。
  //
  // 正本CPが実際に持っている鏡は反対角線 (x,y)->(1-y,1-x) だけで、これは頭の角(0,0)と
  // 尾の角(1,1)を入れ替え、2枚の羽はそれぞれ自分自身へ写す。そこで主張を頭と尾で組み替える。
  it("尾側の面と折り線は、反対角線をはさんで頭側に相手を持つ", () => {
    // 対称の許容差。座標は長辺=1.0に正規化されており、実測の食い違いは面積で6.07e-14。
    // 実測値を境目にせず、モデル共通EPS 1e-9を上限にする(§10.7.7)。
    const AREA_TOL = 1e-9;

    // 頭は角(0,0)、尾は角(1,1)から出る(羽は残りの2隅)。
    const head = facesInQuarter(doc, faces, [0, 0, 0.5, 0.5]);
    const tail = facesInQuarter(doc, faces, [0.5, 0.5, 1, 1]);
    expect(head.length).toBe(13);
    expect(tail.length).toBe(6);

    // (1) 尾側の面は、反対角線で写すと頭側の面の集まりでちょうど覆える。
    // 頭側のほうが枚数が多いのは、頭の折りが同じ場所をさらに細かく割っているからで、
    // 尾側の1枚が頭側の1枚または2枚に対応する。
    const byId = new Map(faces.map((f) => [f.id, f]));
    const used = new Set<number>();
    for (const t of tail) {
      const mirrored = polygonOf(doc, byId.get(t) as Face).map(mirrorAcrossAntiDiagonal);
      const cover = head.filter((h) =>
        pointInPolygon(polygonCentroid(polygonOf(doc, byId.get(h) as Face)), mirrored),
      );
      expect(cover.length, `尾側の面${t}を覆う頭側の面`).toBeGreaterThan(0);
      const covered = cover.reduce(
        (sum, h) => sum + polygonArea(polygonOf(doc, byId.get(h) as Face)),
        0,
      );
      expect(
        Math.abs(covered - polygonArea(mirrored)),
        `尾側の面${t}の鏡像と、それを覆う頭側の面${cover}の面積差`,
      ).toBeLessThan(AREA_TOL);
      for (const h of cover) used.add(h);
    }
    // 覆いに使われる頭側の面は9枚。残る4枚(14,19,46,47)は頭の折りで増えた面で、
    // その鏡像は尾側の1/4に収まる面の中には無い。
    expect([...used].sort((a, b) => a - b)).toEqual([6, 7, 15, 17, 41, 42, 43, 44, 45]);
    expect(head.filter((h) => !used.has(h)).sort((a, b) => a - b)).toEqual([14, 19, 46, 47]);

    // (2) 尾側だけをつかむときの折り線は、1本残らず頭側に相手を持つ。
    // 逆向き(頭側→尾側)は主張しない。`driveHinges` は「根の面までの経路」を返し、
    // 根は `faceParents` が面の配列の最初の面(面0)から作るので尾側の1/4に入っている。
    // そのため頭側の面は経路が長く(13枚で折り線25本)、尾側は短い(6枚で折り線5本)。
    // この差は展開図の対称性ではなく全域木の根の位置で決まるので、片方向だけを主張する。
    const headHinges = new Set(head.flatMap((f) => driveHinges(faces, f)));
    const tailHinges = new Set(tail.flatMap((f) => driveHinges(faces, f)));
    expect(headHinges.size).toBe(25);
    expect(tailHinges.size).toBe(5);
    // 胴を通る共通の折り線(辺3・辺22)は頭と尾の区別が付かないので、尾側だけのものを見る。
    const tailOnly = [...tailHinges].filter((h) => !headHinges.has(h)).sort((a, b) => a - b);
    expect(tailOnly).toEqual([4, 10, 86]);
    for (const h of tailOnly) {
      const other = mirrorHingeOf(doc, faces, h);
      expect(headHinges.has(other as number), `辺${h}の相手 ${other}`).toBe(true);
      expect(mirrorHingeOf(doc, faces, other as number)).toBe(h);
    }

    // (3) 折り線102本のうち、相手が見つからないのは6本だけ。
    // 29,50,51,107,108 は反対角線そのものの上に乗る折り線で、`mirrorHingeOf` は
    // 「折り返しても自分自身」になるものを設計どおりnullにする。
    // 59 は頭の折りの7本のうちの1本で、尾側に対応する折り線が無い。
    const creases = doc.cp.edges.filter((e) => e.kind !== "Border");
    expect(creases.length).toBe(102);
    const lonely = creases
      .filter((e) => mirrorHingeOf(doc, faces, e.id) === null)
      .map((e) => e.id)
      .sort((a, b) => a - b);
    expect(lonely).toEqual([29, 50, 51, 59, 107, 108]);
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
