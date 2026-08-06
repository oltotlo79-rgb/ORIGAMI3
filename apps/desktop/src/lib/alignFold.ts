// 「合わせて折る」(折り紙の基準合わせ)の幾何計算。
// 目分量ではなく、選んだ点・線から折り線を厳密に決めるための純関数だけを置く。
// 座標は畳み平面(3D表示のxy = 平らに畳んだ紙の座標)。Three.jsには依存しない。
//
// 対応する3つの合わせ方(折り紙公理の2・3・5に相当):
//   点と点  → 2点の垂直二等分線(解は1本)
//   線と線  → 2直線の角の二等分線(交わるなら2本、平行なら中間線1本)
//   点を線へ(折り目が通る点を指定) → 指定点を中心にした円と線の交点から作る(0〜2本)

import type { Vec2 } from "./types";

/** 長さ・距離が0かどうかの判定に使う余裕(正規化座標。紙の長辺=1.0) */
export const ALIGN_EPS = 1e-9;

/** 折り線(畳み平面の2点。FoldThroughのlineと同じ形) */
export type FoldLine = [Vec2, Vec2];

/** 3つの合わせ方 */
export type AlignMode = "pointPoint" | "lineLine" | "pointLineThrough";

/** 合わせる対象(3D画面でクリックして選ぶ) */
export type AlignTarget =
  | { kind: "point"; p: Vec2 }
  | { kind: "line"; a: Vec2; b: Vec2 };

/** 合わせ方ごとに必要な選択の数と、順番どおりの対象の種類 */
export const ALIGN_STEPS: Record<AlignMode, ("point" | "line")[]> = {
  pointPoint: ["point", "point"],
  lineLine: ["line", "line"],
  pointLineThrough: ["point", "line", "point"],
};

/** 合わせ方の日本語名(画面のボタン・ヒントで使う) */
export const ALIGN_LABELS: Record<AlignMode, string> = {
  pointPoint: "点と点を合わせる",
  lineLine: "線と線を合わせる",
  pointLineThrough: "点を線に合わせる(折り目が通る点を指定)",
};

function sub(a: Vec2, b: Vec2): Vec2 {
  return [a[0] - b[0], a[1] - b[1]];
}
function dot(a: Vec2, b: Vec2): number {
  return a[0] * b[0] + a[1] * b[1];
}
function cross(a: Vec2, b: Vec2): number {
  return a[0] * b[1] - a[1] * b[0];
}

/** 単位方向ベクトル。長さ0ならnull */
export function unitDir(line: FoldLine): Vec2 | null {
  const d = sub(line[1], line[0]);
  const len = Math.hypot(d[0], d[1]);
  return len < ALIGN_EPS ? null : [d[0] / len, d[1] / len];
}

/**
 * 2点の垂直二等分線。この線で折ると p が q に重なる。
 * 返す線分は中点から左右へ |pq|/2 ずつ(例: (0,0)と(1,1) → [(1,0),(0,1)] = y=1-x)。
 * 2点が重なっていればnull。
 */
export function perpendicularBisector(p: Vec2, q: Vec2): FoldLine | null {
  const d = sub(q, p);
  const len = Math.hypot(d[0], d[1]);
  if (len < ALIGN_EPS) return null;
  const mid: Vec2 = [(p[0] + q[0]) / 2, (p[1] + q[1]) / 2];
  // 単位法線(pqを90度回したもの)
  const n: Vec2 = [-d[1] / len, d[0] / len];
  const h = len / 2;
  return [
    [mid[0] - n[0] * h, mid[1] - n[1] * h],
    [mid[0] + n[0] * h, mid[1] + n[1] * h],
  ];
}

/**
 * 2直線の角の二等分線。この線で折ると1本目の直線が2本目に重なる。
 * 交わるときは内角と外角の2本、平行なときは中間線1本。
 * どちらかの線の長さが0なら空。
 */
export function angleBisectors(l1: FoldLine, l2: FoldLine): FoldLine[] {
  const u1 = unitDir(l1);
  const u2 = unitDir(l2);
  if (!u1 || !u2) return [];
  const scale = Math.max(
    Math.hypot(l1[1][0] - l1[0][0], l1[1][1] - l1[0][1]),
    Math.hypot(l2[1][0] - l2[0][0], l2[1][1] - l2[0][1]),
  );
  const den = cross(u1, u2);
  if (Math.abs(den) < ALIGN_EPS) {
    // 平行: 1本目の始点から2本目へ下ろした足との中点を通る、同じ向きの線
    const foot = footOnLine(l1[0], l2, u2);
    const mid: Vec2 = [(l1[0][0] + foot[0]) / 2, (l1[0][1] + foot[1]) / 2];
    return [segmentAt(mid, u1, scale)];
  }
  const t = cross(sub(l2[0], l1[0]), u2) / den;
  const x: Vec2 = [l1[0][0] + u1[0] * t, l1[0][1] + u1[1] * t];
  const out: FoldLine[] = [];
  for (const v of [
    [u1[0] + u2[0], u1[1] + u2[1]] as Vec2,
    [u1[0] - u2[0], u1[1] - u2[1]] as Vec2,
  ]) {
    const len = Math.hypot(v[0], v[1]);
    if (len < ALIGN_EPS) continue;
    out.push(segmentAt(x, [v[0] / len, v[1] / len], scale));
  }
  return out;
}

/** 点から直線へ下ろした足(uはlineの単位方向) */
function footOnLine(p: Vec2, line: FoldLine, u: Vec2): Vec2 {
  const t = dot(sub(p, line[0]), u);
  return [line[0][0] + u[0] * t, line[0][1] + u[1] * t];
}

/** 中心cを通り向きuの線分(左右へhalfずつ) */
function segmentAt(c: Vec2, u: Vec2, half: number): FoldLine {
  const h = Math.max(half, ALIGN_EPS);
  return [
    [c[0] - u[0] * h, c[1] - u[1] * h],
    [c[0] + u[0] * h, c[1] + u[1] * h],
  ];
}

/**
 * 点pを直線lineへ乗せる折りのうち、点throughを通るもの(0〜2本)。
 * throughを中心・半径|through-p|の円と直線の交点p'について、
 * 折り線はpp'の垂直二等分線になる(|through-p|=|through-p'|なのでthroughを通る)。
 * 円が直線に届かなければ0本、接していれば1本、交われば2本。
 * pが直線上にあるときの「動かない解」(p'=p)は折りにならないので除く。
 */
export function foldPointOntoLine(
  p: Vec2,
  line: FoldLine,
  through: Vec2,
): FoldLine[] {
  const u = unitDir(line);
  if (!u) return [];
  const foot = footOnLine(through, line, u);
  const d = Math.hypot(foot[0] - through[0], foot[1] - through[1]);
  const r = Math.hypot(p[0] - through[0], p[1] - through[1]);
  if (r < ALIGN_EPS || d > r + ALIGN_EPS) return [];
  const h = Math.sqrt(Math.max(0, r * r - d * d));
  const hits: Vec2[] =
    h <= ALIGN_EPS
      ? [foot]
      : [
          [foot[0] + u[0] * h, foot[1] + u[1] * h],
          [foot[0] - u[0] * h, foot[1] - u[1] * h],
        ];
  const out: FoldLine[] = [];
  for (const q of hits) {
    const bisector = perpendicularBisector(p, q);
    if (bisector) out.push(bisector);
  }
  return out;
}

/** 折り線を長さ2*halfの線分へ伸ばす(向きと乗っている直線は変えない)。
 * 下見の表示と可動側の判定を安定させるため、解はすべて紙より長くしておく */
export function extendLine(line: FoldLine, half = 1): FoldLine {
  const u = unitDir(line);
  if (!u) return line;
  const mid: Vec2 = [
    (line[0][0] + line[1][0]) / 2,
    (line[0][1] + line[1][1]) / 2,
  ];
  return segmentAt(mid, u, half);
}

/** 対象を代表する1点(線なら中点)。どちら側が動くかを決めるのに使う */
export function alignRefPoint(t: AlignTarget): Vec2 {
  return t.kind === "point" ? t.p : [(t.a[0] + t.b[0]) / 2, (t.a[1] + t.b[1]) / 2];
}

/**
 * 点refが折り線のどちら側にあるか(線の進行方向に対する左右)。
 * 1つ目に選んだ対象がある側が動く側になるので、その判定に使う。
 * 線上(判定できない)ときは既定の"right"を返す。
 */
export function movingSideOf(line: FoldLine, ref: Vec2): "left" | "right" {
  const u = unitDir(line);
  if (!u) return "right";
  const s = cross(u, sub(ref, line[0]));
  return s > ALIGN_EPS ? "left" : "right";
}

/** 点から直線までの距離(解をカーソルに近い順へ並べるのに使う) */
export function distanceToLine(line: FoldLine, p: Vec2): number {
  const u = unitDir(line);
  if (!u) return Math.hypot(p[0] - line[0][0], p[1] - line[0][1]);
  return Math.abs(cross(u, sub(p, line[0])));
}

/** 解をカーソルに近い順へ並べ替える(既定の解をカーソル寄りにするため) */
export function sortByCursor(lines: FoldLine[], cursor: Vec2 | null): FoldLine[] {
  if (!cursor) return lines;
  return [...lines].sort(
    (a, b) => distanceToLine(a, cursor) - distanceToLine(b, cursor),
  );
}

/** 合わせ方の計算結果。linesが空のときはreasonに日本語の理由が入る */
export interface AlignSolution {
  lines: FoldLine[];
  reason: string | null;
}

/**
 * 選んだ対象から折り線を求める。picksはALIGN_STEPSの順に揃っている前提。
 * 解が0本のときは、なぜ折れないかを日本語で返す(止めずに警告の原則)。
 */
export function solveAlign(
  mode: AlignMode,
  picks: AlignTarget[],
  cursor: Vec2 | null = null,
): AlignSolution {
  const need = ALIGN_STEPS[mode].length;
  if (picks.length < need) return { lines: [], reason: null };
  let lines: FoldLine[];
  if (mode === "pointPoint" && picks[0].kind === "point" && picks[1].kind === "point") {
    const b = perpendicularBisector(picks[0].p, picks[1].p);
    if (!b) return { lines: [], reason: "2つの点が同じ位置です。別の点を選んでください" };
    lines = [b];
  } else if (mode === "lineLine" && picks[0].kind === "line" && picks[1].kind === "line") {
    lines = angleBisectors([picks[0].a, picks[0].b], [picks[1].a, picks[1].b]);
    if (lines.length === 0)
      return { lines: [], reason: "選んだ線の長さが0です。別の線を選んでください" };
  } else if (
    mode === "pointLineThrough" &&
    picks[0].kind === "point" &&
    picks[1].kind === "line" &&
    picks[2].kind === "point"
  ) {
    lines = foldPointOntoLine(picks[0].p, [picks[1].a, picks[1].b], picks[2].p);
    if (lines.length === 0)
      return {
        lines: [],
        reason:
          "この点を通る折り方では届きません(折り目が通る点をもっと線の近くに選んでください)",
      };
  } else {
    return { lines: [], reason: "選んだ対象の種類が合いません。やり直してください" };
  }
  return { lines: sortByCursor(lines, cursor).map((l) => extendLine(l)), reason: null };
}
