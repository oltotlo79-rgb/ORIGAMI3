// 作図補助(CPE-005)の計算。角の二等分線・垂線・等分点の目印・角度刻みの方向線を、
// 引くべき補助線の並びとして返す(純関数。IPCは通さない)。
//
// 同じ計算はRust側(crates/ori3-cp/src/construct.rs)にもある。Tauriコマンドは
// 9個のまま増やさない約束なので、画面で使う分はここで計算し、結果を既存の
// edit_apply(AddSegment)で補助線として送る。数式は両方に同じテストを置いている。

import type { Vec2 } from "./types";

export type ConstructKind = "bisector" | "perpendicular" | "divide" | "angle";

/** 等分点の目印(短いひげ)の長さ(紙の長辺=1.0の座標系) */
const TICK_LEN = 0.03;
/** 方向線の片側の長さ。紙の内側のどこからでも反対の縁まで届く */
const RAY_LEN = 1.0;
/** 座標の許容誤差 */
const EPS = 1e-9;

const sub = (a: Vec2, b: Vec2): Vec2 => [a[0] - b[0], a[1] - b[1]];
const add = (a: Vec2, b: Vec2): Vec2 => [a[0] + b[0], a[1] + b[1]];
const mul = (a: Vec2, k: number): Vec2 => [a[0] * k, a[1] * k];
const len = (a: Vec2): number => Math.hypot(a[0], a[1]);
const perp = (a: Vec2): Vec2 => [-a[1], a[0]];

/** 角ABC(頂点B)の二等分線。長さは長い方の腕に合わせる。角が決まらなければnull */
export function bisector(a: Vec2, b: Vec2, c: Vec2): [Vec2, Vec2] | null {
  const u = sub(a, b);
  const v = sub(c, b);
  const lu = len(u);
  const lv = len(v);
  if (lu < EPS || lv < EPS) return null;
  const sum = add(mul(u, 1 / lu), mul(v, 1 / lv));
  // まっすぐ(180°)に開いた角では向きが定まらないので腕に垂直な向きを使う
  const dir = len(sum) < EPS ? perp(mul(u, 1 / lu)) : mul(sum, 1 / len(sum));
  return [b, add(b, mul(dir, Math.max(lu, lv)))];
}

/** 点pから線分segへ下ろした垂線(足は線分を延長した直線の上に取る) */
export function perpendicular(p: Vec2, seg: [Vec2, Vec2]): [Vec2, Vec2] | null {
  const d = sub(seg[1], seg[0]);
  const l2 = d[0] * d[0] + d[1] * d[1];
  if (l2 < EPS * EPS) return null;
  const t = (sub(p, seg[0])[0] * d[0] + sub(p, seg[0])[1] * d[1]) / l2;
  const foot = add(seg[0], mul(d, t));
  return len(sub(foot, p)) < EPS ? null : [p, foot];
}

/** 線分をn等分する点(両端を含まないn-1個)。nが2〜8の外なら空 */
export function dividePoints(seg: [Vec2, Vec2], n: number): Vec2[] {
  if (!Number.isInteger(n) || n < 2 || n > 8) return [];
  const d = sub(seg[1], seg[0]);
  if (len(d) < EPS) return [];
  return Array.from({ length: n - 1 }, (_, i) => add(seg[0], mul(d, (i + 1) / n)));
}

/** 等分点の目印。各点を端点にした短いひげを線分と直角に出す
 *  (端点にすると点そのものが頂点になり、次に線を引くとき正確に吸着できる) */
export function divideTicks(seg: [Vec2, Vec2], n: number): [Vec2, Vec2][] {
  const d = sub(seg[1], seg[0]);
  const l = len(d);
  if (l < EPS) return [];
  const nrm = mul(perp(mul(d, 1 / l)), TICK_LEN);
  return dividePoints(seg, n).map((p) => [p, add(p, nrm)] as [Vec2, Vec2]);
}

/** 点pを通る方向線を0°からstepDeg刻みで180°未満まで並べる(22.5°なら8本) */
export function directionLines(p: Vec2, stepDeg: number): [Vec2, Vec2][] {
  if (!(stepDeg > 0 && stepDeg <= 180)) return [];
  const lines: [Vec2, Vec2][] = [];
  for (let deg = 0; deg < 180 - 1e-9; deg += stepDeg) {
    const rad = (deg * Math.PI) / 180;
    const d: Vec2 = [Math.cos(rad) * RAY_LEN, Math.sin(rad) * RAY_LEN];
    lines.push([sub(p, d), add(p, d)]);
  }
  return lines;
}

/** 作図補助の選択状態(どの作図か・等分数・角度の刻み) */
export interface ConstructOptions {
  kind: ConstructKind;
  /** 等分の数(2〜8) */
  divisions: number;
  /** 方向線の角度の刻み(度) */
  stepDeg: number;
}

/** 作図補助の初期設定。角度は折り紙でよく使う22.5°刻み */
export const DEFAULT_CONSTRUCT: ConstructOptions = {
  kind: "bisector",
  divisions: 4,
  stepDeg: 22.5,
};

/** 作図ごとに必要なクリックの順番("pt"=点、"line"=既にある線) */
export const CONSTRUCT_STEPS: Record<ConstructKind, ("pt" | "line")[]> = {
  bisector: ["pt", "pt", "pt"],
  perpendicular: ["line", "pt"],
  divide: ["pt", "pt"],
  angle: ["pt"],
};

/** ツールレールのサブメニューに出す名前(折り紙の言葉で短く) */
export const CONSTRUCT_LABEL: Record<ConstructKind, string> = {
  bisector: "二等分",
  perpendicular: "垂線",
  divide: "等分",
  angle: "角度線",
};

/** 次に何をすればよいかの案内(1行)。doneは済んだクリックの数 */
export function constructHint(kind: ConstructKind, done: number, divisions: number): string {
  const steps: Record<ConstructKind, string[]> = {
    bisector: [
      "角の1本目の先をクリック",
      "角の頂点(2本が集まる点)をクリック",
      "角のもう1本の先をクリック",
    ],
    perpendicular: ["直角に交わらせたい線をクリック", "線を通したい点をクリック"],
    divide: [`${divisions}等分したい区間の始点をクリック`, "区間の終点をクリック"],
    angle: ["方向線を出したい点をクリック"],
  };
  const list = steps[kind];
  const text = list[Math.min(done, list.length - 1)];
  return `${CONSTRUCT_LABEL[kind]}: ${text}(Escで中止)`;
}

/** 集めたクリックから引くべき補助線を作る。まだ足りなければ空を返す。
 *  結果は紙の内側だけに切り取る(紙からはみ出した線はCPに置けないため) */
export function constructLines(
  kind: ConstructKind,
  points: Vec2[],
  seg: [Vec2, Vec2] | null,
  opts: { divisions: number; stepDeg: number; paper: Vec2 },
): [Vec2, Vec2][] {
  let raw: ([Vec2, Vec2] | null)[] = [];
  if (kind === "bisector" && points.length >= 3) {
    raw = [bisector(points[0], points[1], points[2])];
  } else if (kind === "perpendicular" && seg && points.length >= 1) {
    raw = [perpendicular(points[0], seg)];
  } else if (kind === "divide" && points.length >= 2) {
    raw = divideTicks([points[0], points[1]], opts.divisions);
  } else if (kind === "angle" && points.length >= 1) {
    raw = directionLines(points[0], opts.stepDeg);
  }
  return raw
    .map((l) => (l ? clipToPaper(l, opts.paper[0], opts.paper[1]) : null))
    .filter((l): l is [Vec2, Vec2] => l !== null && len(sub(l[1], l[0])) > EPS);
}

/** 線分を紙の矩形(0,0)-(w,h)で切り取る。紙に掛からなければnull(Liang-Barsky法) */
export function clipToPaper(seg: [Vec2, Vec2], w: number, h: number): [Vec2, Vec2] | null {
  const d = sub(seg[1], seg[0]);
  let t0 = 0;
  let t1 = 1;
  const limits: [number, number][] = [
    [-d[0], seg[0][0]],
    [d[0], w - seg[0][0]],
    [-d[1], seg[0][1]],
    [d[1], h - seg[0][1]],
  ];
  for (const [p, q] of limits) {
    if (Math.abs(p) < EPS) {
      if (q < 0) return null; // 辺と平行で紙の外
      continue;
    }
    const r = q / p;
    if (p < 0) t0 = Math.max(t0, r);
    else t1 = Math.min(t1, r);
  }
  if (t1 - t0 < EPS) return null;
  return [add(seg[0], mul(d, t0)), add(seg[0], mul(d, t1))];
}
