// 「合わせて折る」(折り紙の基準合わせ)の幾何計算。
// 目分量ではなく、選んだ点・線から折り線を厳密に決めるための純関数だけを置く。
// 座標は、全入力を載せた同じ等長2D chart。平らな紙では畳み平面xyを使い、
// 3Dでは一意な共通支持平面のchartを呼び出し側が作る。global XYや個別Face chartを
// 混ぜず、ここはThree.jsやFace IDに依存しない。
//
// 藤田・羽鳥の7基本作図をすべて扱う。加えて、折り図で頻出する
// 「既存の折り筋をそのまま使う」も、画面上で引き直さず選べるようにする。

import type { AlignMode, AlignTarget, Vec2 } from "./types";
export type { AlignMode, AlignTarget } from "./types";

/** 長さ・距離が0かどうかの判定に使う余裕(正規化座標。紙の長辺=1.0) */
export const ALIGN_EPS = 1e-9;

/** 折り線(畳み平面の2点。FoldThroughのlineと同じ形) */
export type FoldLine = [Vec2, Vec2];

/** 合わせ方ごとに必要な選択の数と、順番どおりの対象の種類 */
export const ALIGN_STEPS: Record<AlignMode, ("point" | "line")[]> = {
  throughTwoPoints: ["point", "point"],
  pointPoint: ["point", "point"],
  lineLine: ["line", "line"],
  pointPerpendicularLine: ["point", "line"],
  pointLineThrough: ["point", "line", "point"],
  pointToLinePointToLine: ["point", "line", "point", "line"],
  pointLinePerpendicular: ["point", "line", "line"],
  existingLine: ["line"],
};

/**
 * 合わせ方ごとの説明。何ができるかと、何をどの順にクリックするかを書く。
 *
 * ボタンの名前だけでは何が起きるか分からないため、マウスを乗せたときと
 * キーボードで選んだときに、この文をそのまま出す。
 */
export const ALIGN_HINTS: Record<AlignMode, string> = {
  throughTwoPoints:
    "選んだ2つの点を結ぶ位置に折り目を作ります。展開図または3D表示で点を2つクリックします",
  pointPoint:
    "一方の点をもう一方へぴったり重ねる折り目を作ります。展開図または3D表示で点を2つクリックします",
  lineLine:
    "一方の線をもう一方へぴったり重ねる折り目を作ります。展開図または3D表示で線を2つクリックします",
  pointPerpendicularLine:
    "選んだ点を通り、選んだ線と直角に交わる折り目を作ります。展開図または3D表示で点→線の順にクリックします",
  pointLineThrough:
    "点を線の上へ重ねる折り目のうち、指定した別の点を通るものを作ります。展開図または3D表示で点→線→通ってほしい点の順にクリックします",
  pointToLinePointToLine:
    "2つの点を、それぞれ別の線の上へ同時に重ねる折り目を作ります。展開図または3D表示で点→線→点→線の順にクリックします",
  pointLinePerpendicular:
    "点を線の上へ重ねつつ、もう1本の線と直角に交わる折り目を作ります。展開図または3D表示で点→線→線の順にクリックします",
  existingLine:
    "すでにある折り線に沿って折ります。展開図または3D表示でその線をクリックします",
};

/** 合わせ方の日本語名(画面のボタン・ヒントで使う) */
export const ALIGN_LABELS: Record<AlignMode, string> = {
  throughTwoPoints: "2点を通る",
  pointPoint: "点と点を合わせる",
  lineLine: "線と線を合わせる",
  pointPerpendicularLine: "点を通り線と垂直に",
  pointLineThrough: "点を線に合わせる(折り目が通る点を指定)",
  pointToLinePointToLine: "2組を同時に合わせる",
  pointLinePerpendicular: "点を線に合わせて別の線と垂直に",
  existingLine: "既存の線に沿って折る",
};

function sub(a: Vec2, b: Vec2): Vec2 {
  return [a[0] - b[0], a[1] - b[1]];
}
function add(a: Vec2, b: Vec2): Vec2 {
  return [a[0] + b[0], a[1] + b[1]];
}
function mul(a: Vec2, k: number): Vec2 {
  return [a[0] * k, a[1] * k];
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

/** 藤田・羽鳥1: 異なる2点を通る折り線。選んだ座標をそのまま端点に使う。 */
export function lineThroughPoints(p: Vec2, q: Vec2): FoldLine | null {
  return Math.hypot(q[0] - p[0], q[1] - p[1]) < ALIGN_EPS ? null : [p, q];
}

/** 藤田・羽鳥4: 点pを通り、lineに垂直な折り線。 */
export function perpendicularThroughPoint(p: Vec2, line: FoldLine): FoldLine | null {
  const u = unitDir(line);
  if (!u) return null;
  const half = Math.max(
    Math.hypot(line[1][0] - line[0][0], line[1][1] - line[0][1]),
    ALIGN_EPS,
  );
  return segmentAt(p, [-u[1], u[0]], half);
}

/** 2本の無限直線の交点。平行ならnull。 */
function lineIntersection(l1: FoldLine, l2: FoldLine): Vec2 | null {
  const r = sub(l1[1], l1[0]);
  const s = sub(l2[1], l2[0]);
  const den = cross(r, s);
  const scale = Math.hypot(r[0], r[1]) * Math.hypot(s[0], s[1]);
  if (scale < ALIGN_EPS || Math.abs(den) <= ALIGN_EPS * scale) return null;
  const t = cross(sub(l2[0], l1[0]), s) / den;
  return add(l1[0], mul(r, t));
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

/** 点を折り線で鏡映した位置。長さ0の折り線ならnull。 */
export function reflectPointAcrossFold(p: Vec2, fold: FoldLine): Vec2 | null {
  const u = unitDir(fold);
  if (!u) return null;
  const foot = footOnLine(p, fold, u);
  return [2 * foot[0] - p[0], 2 * foot[1] - p[1]];
}

interface LineEquation {
  /** 単位法線 */
  n: Vec2;
  /** n・x = d */
  d: number;
}

function lineEquation(line: FoldLine): LineEquation | null {
  const u = unitDir(line);
  if (!u) return null;
  const n: Vec2 = [-u[1], u[0]];
  return { n, d: dot(n, line[0]) };
}

/** 係数を定数項から昇順に並べた多項式。 */
type Polynomial = number[];

function polyScale(a: Polynomial, k: number): Polynomial {
  return a.map((v) => v * k);
}

function polySub(a: Polynomial, b: Polynomial): Polynomial {
  return Array.from({ length: Math.max(a.length, b.length) }, (_, i) =>
    (a[i] ?? 0) - (b[i] ?? 0),
  );
}

function polyMul(a: Polynomial, b: Polynomial): Polynomial {
  const out = Array.from({ length: a.length + b.length - 1 }, () => 0);
  for (let i = 0; i < a.length; i++) {
    for (let j = 0; j < b.length; j++) out[i + j] += a[i] * b[j];
  }
  return out;
}

function polyValue(a: Polynomial, x: number): number {
  let out = 0;
  for (let i = a.length - 1; i >= 0; i--) out = out * x + a[i];
  return out;
}

function polyDerivativeValue(a: Polynomial, x: number): number {
  let out = 0;
  for (let i = a.length - 1; i >= 1; i--) out = out * x + i * a[i];
  return out;
}

function uniqueRoots(values: number[]): number[] {
  const out: number[] = [];
  for (const value of values.filter(Number.isFinite).sort((a, b) => a - b)) {
    const prev = out[out.length - 1];
    if (prev === undefined || Math.abs(value - prev) > 1e-8 * Math.max(1, Math.abs(value))) {
      out.push(value);
    }
  }
  return out;
}

/** 3次以下の実係数多項式の実根。Cardanoの式の後にNewton法で残差を詰める。 */
function realPolynomialRoots(input: Polynomial): number[] {
  const coeff = [...input];
  const scale = Math.max(Number.MIN_VALUE, ...coeff.map(Math.abs));
  // 係数全体に対する相対値だけで次数を判定する。絶対値1を下限にすると、
  // 紙内のごく近い線から生じる小さい（ただし有効な）3次項を消してしまう。
  while (
    coeff.length > 1 &&
    Math.abs(coeff[coeff.length - 1]) <= 64 * Number.EPSILON * scale
  ) {
    coeff.pop();
  }
  const degree = coeff.length - 1;
  let roots: number[] = [];
  if (degree === 1) {
    roots = [-coeff[0] / coeff[1]];
  } else if (degree === 2) {
    const [c, b, a] = coeff;
    const disc = b * b - 4 * a * c;
    const tol =
      64 * Number.EPSILON *
      Math.max(Number.MIN_VALUE, Math.abs(b * b), Math.abs(4 * a * c));
    if (disc >= -tol) {
      const s = Math.sqrt(Math.max(0, disc));
      roots = s <= Math.sqrt(tol) ? [-b / (2 * a)] : [(-b - s) / (2 * a), (-b + s) / (2 * a)];
    }
  } else if (degree === 3) {
    const [d, c, b, a] = coeff;
    const aa = b / a;
    const bb = c / a;
    const cc = d / a;
    const p = bb - (aa * aa) / 3;
    const q = (2 * aa * aa * aa) / 27 - (aa * bb) / 3 + cc;
    const disc = (q * q) / 4 + (p * p * p) / 27;
    // 判別式は近接した3実根で非常に小さくなる。ここへ絶対値1の床を置くと、
    // 負の判別式を0とみなして2解を落とすため、構成項の丸め誤差だけを許容する。
    const tol =
      64 * Number.EPSILON *
      Math.max(Number.MIN_VALUE, Math.abs((q * q) / 4), Math.abs((p * p * p) / 27));
    if (disc > tol) {
      const s = Math.sqrt(disc);
      roots = [Math.cbrt(-q / 2 + s) + Math.cbrt(-q / 2 - s) - aa / 3];
    } else if (disc >= -tol) {
      const u = Math.cbrt(-q / 2);
      roots = [2 * u - aa / 3, -u - aa / 3];
    } else {
      const radius = 2 * Math.sqrt(-p / 3);
      const den = 2 * Math.sqrt(-(p * p * p) / 27);
      const angle = Math.acos(Math.max(-1, Math.min(1, -q / den)));
      roots = [0, 1, 2].map(
        (k) => radius * Math.cos((angle + 2 * Math.PI * k) / 3) - aa / 3,
      );
    }
  }

  // 閉形式で得た値を元の係数へ戻して磨く。選択座標自体は一切丸めない。
  const refined = roots.map((initial) => {
    let x = initial;
    for (let i = 0; i < 6; i++) {
      const dy = polyDerivativeValue(coeff, x);
      if (!Number.isFinite(dy) || Math.abs(dy) <= 1e-14) break;
      const next = x - polyValue(coeff, x) / dy;
      if (!Number.isFinite(next)) break;
      x = next;
    }
    return x;
  });
  return uniqueRoots(refined);
}

interface NormalFold {
  line: FoldLine;
  n: Vec2;
  c: number;
}

function canonicalNormal(n: Vec2, c: number): { n: Vec2; c: number } {
  if (n[0] < -ALIGN_EPS || (Math.abs(n[0]) <= ALIGN_EPS && n[1] < 0)) {
    return { n: [-n[0], -n[1]], c: -c };
  }
  return { n, c };
}

/**
 * 法線nの向きを固定したとき、2組の「点→線」を同時に満たす折り線を作る。
 * 各条件から折り線 n・x=c のcを独立に求め、一致と反射後の距離を再検算する。
 */
function simultaneousCandidate(
  normal: Vec2,
  p1: Vec2,
  l1: FoldLine,
  e1: LineEquation,
  p2: Vec2,
  l2: FoldLine,
  e2: LineEquation,
): NormalFold | null {
  const length = Math.hypot(normal[0], normal[1]);
  if (length < ALIGN_EPS) return null;
  const n: Vec2 = [normal[0] / length, normal[1] / length];

  const offset = (p: Vec2, e: LineEquation): number | null | undefined => {
    const delta = dot(e.n, p) - e.d;
    const den = dot(e.n, n);
    if (Math.abs(den) <= ALIGN_EPS) {
      // 焦点が既に線上ならこの条件はcを拘束しない。それ以外はこの向きでは不可能。
      return Math.abs(delta) <= ALIGN_EPS ? null : undefined;
    }
    return dot(n, p) - delta / (2 * den);
  };

  const c1 = offset(p1, e1);
  const c2 = offset(p2, e2);
  if (c1 === undefined || c2 === undefined || (c1 === null && c2 === null)) return null;
  if (c1 !== null && c2 !== null && Math.abs(c1 - c2) > 2e-7) return null;
  const c = c1 === null ? c2! : c2 === null ? c1 : (c1 + c2) / 2;
  const base = mul(n, c);
  const fold = segmentAt(base, [-n[1], n[0]], 1);
  const q1 = reflectPointAcrossFold(p1, fold);
  const q2 = reflectPointAcrossFold(p2, fold);
  if (!q1 || !q2 || distanceToLine(l1, q1) > 2e-7 || distanceToLine(l2, q2) > 2e-7) {
    return null;
  }
  if (
    Math.hypot(q1[0] - p1[0], q1[1] - p1[1]) <= ALIGN_EPS &&
    Math.hypot(q2[0] - p2[0], q2[1] - p2[1]) <= ALIGN_EPS
  ) {
    return null;
  }
  const canonical = canonicalNormal(n, c);
  return { line: fold, ...canonical };
}

/**
 * 藤田・羽鳥6: p1をl1へ、同時にp2をl2へ重ねる折り線(0〜3本)。
 *
 * 折り線を n・x=c、各合わせ先を mi・x=di とすると、反射条件は
 *   δi|n|² - 2(mi・n)(n・pi-c)=0,  δi=mi・pi-di
 * になる。n=(1,t)として2条件からcを消去すると3次方程式なので、その全実根と
 * n=(0,1)(無限遠の根)を調べる。最後に実際に2点を反射して両直線への距離を検算する。
 */
export function foldTwoPointsOntoTwoLines(
  p1: Vec2,
  l1: FoldLine,
  p2: Vec2,
  l2: FoldLine,
): FoldLine[] {
  const e1 = lineEquation(l1);
  const e2 = lineEquation(l2);
  if (!e1 || !e2) return [];
  const delta1 = dot(e1.n, p1) - e1.d;
  const delta2 = dot(e2.n, p2) - e2.d;
  const a: Polynomial = [e1.n[0], e1.n[1]];
  const b: Polynomial = [e2.n[0], e2.n[1]];
  const displacement: Polynomial = [p1[0] - p2[0], p1[1] - p2[1]];
  // 2(n・(p1-p2))(m1・n)(m2・n) - |n|²(δ1(m2・n)-δ2(m1・n)) = 0
  const cubic = polySub(
    polyScale(polyMul(polyMul(displacement, a), b), 2),
    polyMul([1, 0, 1], polySub(polyScale(b, delta1), polyScale(a, delta2))),
  );

  const candidates: NormalFold[] = [];
  const addCandidate = (normal: Vec2) => {
    const candidate = simultaneousCandidate(normal, p1, l1, e1, p2, l2, e2);
    if (!candidate) return;
    const duplicate = candidates.some(
      (other) =>
        Math.hypot(other.n[0] - candidate.n[0], other.n[1] - candidate.n[1]) <= 2e-7 &&
        Math.abs(other.c - candidate.c) <= 2e-7,
    );
    if (!duplicate) candidates.push(candidate);
  };
  for (const t of realPolynomialRoots(cubic)) addCandidate([1, t]);
  // n.x=0 は t=∞に当たり、通常の3次方程式の有限根には現れない。
  addCandidate([0, 1]);
  return candidates.map((candidate) => candidate.line);
}

/**
 * 藤田・羽鳥7: pをtargetへ重ね、折り目をperpendicularToに垂直にする。
 * pの移動先は「pを通りperpendicularToと平行な線」とtargetの交点で一意に決まる。
 */
export function foldPointOntoLinePerpendicular(
  p: Vec2,
  target: FoldLine,
  perpendicularTo: FoldLine,
): FoldLine | null {
  const u = unitDir(perpendicularTo);
  if (!u) return null;
  const destination = lineIntersection([p, add(p, u)], target);
  if (!destination) return null;
  return perpendicularBisector(p, destination);
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
 * 全pickとcursorは同じ等長2D chartにあり、返す線もそのchart上にある。chartの
 * 平行移動・回転・鏡映に対して同じworld直線へ戻るため、3D入力のzを捨てたり、
 * 対象ごとに別chartを使ったりしてはならない。
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
  if (
    mode === "throughTwoPoints" &&
    picks[0].kind === "point" &&
    picks[1].kind === "point"
  ) {
    const line = lineThroughPoints(picks[0].p, picks[1].p);
    if (!line)
      return { lines: [], reason: "2つの点が同じ位置です。別の点を選んでください" };
    lines = [line];
  } else if (
    mode === "pointPoint" &&
    picks[0].kind === "point" &&
    picks[1].kind === "point"
  ) {
    const b = perpendicularBisector(picks[0].p, picks[1].p);
    if (!b) return { lines: [], reason: "2つの点が同じ位置です。別の点を選んでください" };
    lines = [b];
  } else if (mode === "lineLine" && picks[0].kind === "line" && picks[1].kind === "line") {
    lines = angleBisectors([picks[0].a, picks[0].b], [picks[1].a, picks[1].b]);
    if (lines.length === 0)
      return { lines: [], reason: "選んだ線の長さが0です。別の線を選んでください" };
  } else if (
    mode === "pointPerpendicularLine" &&
    picks[0].kind === "point" &&
    picks[1].kind === "line"
  ) {
    const line = perpendicularThroughPoint(picks[0].p, [picks[1].a, picks[1].b]);
    if (!line)
      return { lines: [], reason: "選んだ線の長さが0です。別の線を選んでください" };
    lines = [line];
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
  } else if (
    mode === "pointToLinePointToLine" &&
    picks[0].kind === "point" &&
    picks[1].kind === "line" &&
    picks[2].kind === "point" &&
    picks[3].kind === "line"
  ) {
    lines = foldTwoPointsOntoTwoLines(
      picks[0].p,
      [picks[1].a, picks[1].b],
      picks[2].p,
      [picks[3].a, picks[3].b],
    );
    if (lines.length === 0)
      return {
        lines: [],
        reason: "この2組を同時に合わせる折り目はありません。別の点や線を選んでください",
      };
  } else if (
    mode === "pointLinePerpendicular" &&
    picks[0].kind === "point" &&
    picks[1].kind === "line" &&
    picks[2].kind === "line"
  ) {
    const line = foldPointOntoLinePerpendicular(
      picks[0].p,
      [picks[1].a, picks[1].b],
      [picks[2].a, picks[2].b],
    );
    if (!line)
      return {
        lines: [],
        reason: "点を合わせながら垂直にできません。別の点や線を選んでください",
      };
    lines = [line];
  } else if (mode === "existingLine" && picks[0].kind === "line") {
    const line: FoldLine = [picks[0].a, picks[0].b];
    if (!unitDir(line))
      return { lines: [], reason: "選んだ線の長さが0です。別の線を選んでください" };
    lines = [line];
  } else {
    return { lines: [], reason: "選んだ対象の種類が合いません。やり直してください" };
  }
  return { lines: sortByCursor(lines, cursor).map((l) => extendLine(l)), reason: null };
}
