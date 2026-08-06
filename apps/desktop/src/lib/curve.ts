// 曲線の折り目(CPE-011)の計算。円弧・3次ベジェを、指定した誤差以内の折れ線にする。
//
// 展開図のデータ構造は直線の辺だけなので、曲線は「十分細かい折れ線」として入れる。
// 同じ計算はRust側(crates/ori3-cp/src/curve.rs)にもある。Tauriコマンドは
// 9個のまま増やさない約束なので、描いている最中の形はここで作り、確定したときに
// 既存の edit_apply(AddSegment)を折れ線の本数だけ送る。数式は両方に同じテストを置く。

import type { Document, Vec2 } from "./types";

/** 折れ線の分割数の上限(これ以上細かくしても見分けが付かず辺数だけ増える) */
export const MAX_CURVE_SEGMENTS = 200;
/** 既定の許容誤差(紙の長辺=1.0 に対する、曲線と弦の離れ方の上限) */
export const DEFAULT_CURVE_TOL = 0.005;

const EPS = 1e-12;
const TAU = Math.PI * 2;

const sub = (a: Vec2, b: Vec2): Vec2 => [a[0] - b[0], a[1] - b[1]];
const add = (a: Vec2, b: Vec2): Vec2 => [a[0] + b[0], a[1] + b[1]];
const mul = (a: Vec2, k: number): Vec2 => [a[0] * k, a[1] * k];
const len = (a: Vec2): number => Math.hypot(a[0], a[1]);
const cross = (a: Vec2, b: Vec2): number => a[0] * b[1] - a[1] * b[0];
const sq = (a: Vec2): number => a[0] * a[0] + a[1] * a[1];

/** 3点を通る円の中心。3点がほぼ一直線ならnull */
export function circumcenter(a: Vec2, b: Vec2, c: Vec2): Vec2 | null {
  const d = 2 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
  if (Math.abs(d) < EPS) return null;
  const [qa, qb, qc] = [sq(a), sq(b), sq(c)];
  return [
    (qa * (b[1] - c[1]) + qb * (c[1] - a[1]) + qc * (a[1] - b[1])) / d,
    (qa * (c[0] - b[0]) + qb * (a[0] - c[0]) + qc * (b[0] - a[0])) / d,
  ];
}

/** 円弧を折れ線にするときの分割数(1区間の弦の膨らみが tol 以下になる最小の数) */
export function arcSegmentCount(radius: number, sweep: number, tol: number): number {
  const t = Math.max(tol, 1e-9);
  if (radius <= t) return 1;
  const step = 2 * Math.acos(Math.min(1, Math.max(-1, 1 - t / radius)));
  if (step <= EPS) return MAX_CURVE_SEGMENTS;
  return Math.min(MAX_CURVE_SEGMENTS, Math.max(1, Math.ceil(Math.abs(sweep) / step)));
}

const mod = (a: number, m: number): number => ((a % m) + m) % m;

/**
 * 始点・通過点・終点で決まる円弧の折れ線(端点を含む)。
 * segmentsを指定するとその数で等分し、なければ tol から自動で決める。
 * 3点が一直線・退化していればただの線分を返す。
 */
export function arcPolyline(
  p0: Vec2,
  through: Vec2,
  p1: Vec2,
  tol: number,
  segments?: number,
): Vec2[] {
  const c = circumcenter(p0, through, p1);
  if (!c) return [p0, p1];
  const r = len(sub(p0, c));
  if (r < EPS) return [p0, p1];
  const ang = (p: Vec2) => Math.atan2(p[1] - c[1], p[0] - c[0]);
  const a0 = ang(p0);
  const d1 = mod(ang(p1) - a0, TAU);
  // 通過点が始点から見て終点より手前にあるなら反時計回り、そうでなければ時計回り
  const sweep = mod(ang(through) - a0, TAU) < d1 ? d1 : d1 - TAU;
  const n =
    segments === undefined
      ? arcSegmentCount(r, sweep, tol)
      : Math.min(MAX_CURVE_SEGMENTS, Math.max(1, Math.round(segments)));
  const pts: Vec2[] = [];
  for (let i = 0; i <= n; i++) {
    const t = a0 + (sweep * i) / n;
    pts.push([c[0] + r * Math.cos(t), c[1] + r * Math.sin(t)]);
  }
  // 端点は丸め誤差を入れず指定値そのものにする(既存頂点へ確実に吸着させる)
  pts[0] = p0;
  pts[n] = p1;
  return pts;
}

/** 3次ベジェを折れ線にするときの分割数(誤差は max|B''|/(8n²) 以下) */
export function cubicSegmentCount(
  p0: Vec2,
  c1: Vec2,
  c2: Vec2,
  p1: Vec2,
  tol: number,
): number {
  const t = Math.max(tol, 1e-9);
  // 制御点が始点と終点を結ぶ直線に乗っていれば曲線もその直線上にある
  const chord = sub(p1, p0);
  if (len(chord) > EPS) {
    const u = mul(chord, 1 / len(chord));
    const off = (p: Vec2) => Math.abs(cross(sub(p, p0), u));
    if (off(c1) <= t && off(c2) <= t) return 1;
  }
  const v1 = add(sub(p0, mul(c1, 2)), c2);
  const v2 = add(sub(c1, mul(c2, 2)), p1);
  const m = 6 * Math.max(len(v1), len(v2));
  if (m <= EPS) return 1;
  return Math.min(MAX_CURVE_SEGMENTS, Math.max(1, Math.ceil(Math.sqrt(m / (8 * t)))));
}

/** 3次ベジェの折れ線(端点を含む)。S字も引けるので自由度が高い */
export function cubicPolyline(
  p0: Vec2,
  c1: Vec2,
  c2: Vec2,
  p1: Vec2,
  tol: number,
  segments?: number,
): Vec2[] {
  const n =
    segments === undefined
      ? cubicSegmentCount(p0, c1, c2, p1, tol)
      : Math.min(MAX_CURVE_SEGMENTS, Math.max(1, Math.round(segments)));
  const pts: Vec2[] = [];
  for (let i = 0; i <= n; i++) {
    const t = i / n;
    const u = 1 - t;
    pts.push(
      add(
        add(mul(p0, u * u * u), mul(c1, 3 * u * u * t)),
        add(mul(c2, 3 * u * t * t), mul(p1, t * t * t)),
      ),
    );
  }
  pts[0] = p0;
  pts[n] = p1;
  return pts;
}

/** 線分を紙の矩形(0,0)-(w,h)で切り取る(Liang-Barsky法)。掛からなければnull */
function clipToPaper(a: Vec2, d: Vec2, w: number, h: number): [Vec2, Vec2] | null {
  let t0 = -Infinity;
  let t1 = Infinity;
  const limits: [number, number][] = [
    [-d[0], a[0]],
    [d[0], w - a[0]],
    [-d[1], a[1]],
    [d[1], h - a[1]],
  ];
  for (const [p, q] of limits) {
    if (Math.abs(p) < EPS) {
      if (q < 0) return null;
      continue;
    }
    const r = q / p;
    if (p < 0) t0 = Math.max(t0, r);
    else t1 = Math.min(t1, r);
  }
  if (t1 - t0 < EPS) return null;
  return [add(a, mul(d, t0)), add(a, mul(d, t1))];
}

/** 曲線の折り目の両側に入れる「紙が曲がるための線」(ruling)の端点 */
export interface Ruling {
  /** 曲線の上の点(ここから両側へ伸びる) */
  at: Vec2;
  /** へこむ側(曲がりの内側)の端 */
  concave: Vec2;
  /** 膨らむ側(曲がりの外側)の端 */
  convex: Vec2;
}

/**
 * 曲線の折り目の両側に入れる「紙が曲がるための線」。
 *
 * 曲線の折り目は、両側の紙が曲がらないと折れない(平らな板2枚を曲線でつなぐと
 * 角度0以外では紙がちぎれる)。実際の紙では折り目の両側が円錐状に曲がっており、
 * その曲がりを表すのがこの線。折れ線の各内点で、折れ線に直角な向きへ
 * 紙の縁まで伸ばした線を返す。
 */
export function rulingLines(points: Vec2[], paper: Vec2): Ruling[] {
  const out: Ruling[] = [];
  for (let i = 1; i < points.length - 1; i++) {
    const [prev, cur, next] = [points[i - 1], points[i], points[i + 1]];
    const tan = sub(next, prev);
    if (len(tan) < EPS) continue;
    const n = mul([-tan[1], tan[0]], 1 / len(tan));
    const clipped = clipToPaper(cur, n, paper[0], paper[1]);
    if (!clipped) continue;
    const [p, q] = clipped;
    // 左へ曲がる(外積が正)なら、へこむ側は法線の正の向き(=q側)
    const left = cross(sub(cur, prev), sub(next, cur)) > 0;
    const [concave, convex] = left ? [q, p] : [p, q];
    if (len(sub(concave, cur)) < EPS || len(sub(convex, cur)) < EPS) continue;
    out.push({ at: cur, concave, convex });
  }
  return out;
}

/** 2つの線分の交点(交わらなければnull) */
function segIntersection(a: Vec2, b: Vec2, c: Vec2, d: Vec2): Vec2 | null {
  const r = sub(b, a);
  const s = sub(d, c);
  const denom = cross(r, s);
  if (Math.abs(denom) < 1e-15) return null;
  const t = cross(sub(c, a), s) / denom;
  const u = cross(sub(c, a), r) / denom;
  if (t < 0 || t > 1 || u < 0 || u > 1) return null;
  return add(a, mul(r, t));
}

/**
 * fromからtoへ向かって、最初にぶつかる既存の折り目までで切った終点。
 * 曲がるための線が他の折り目を突き抜けて関係のない場所まで伸びるのを防ぐ
 * (実際の紙でも紙が曲がる範囲は隣の折り目までで区切られる)。
 * Rust側の insert_rulings と同じ切り方(crates/ori3-cp/src/curve.rs)。
 */
export function firstCrossing(doc: Document, from: Vec2, to: Vec2): Vec2 {
  const total = len(sub(to, from));
  if (total < EPS) return to;
  const dir = mul(sub(to, from), 1 / total);
  const byId = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  let best = total;
  for (const e of doc.cp.edges) {
    if (e.kind === "Aux") continue;
    const p0 = byId.get(e.v0);
    const p1 = byId.get(e.v1);
    if (!p0 || !p1) continue;
    const q = segIntersection(from, to, p0, p1);
    if (!q) continue;
    const t = sub(q, from)[0] * dir[0] + sub(q, from)[1] * dir[1];
    // 出発点そのもの(曲線の上)での交わりは無視する
    if (t > 1e-6 && t < best) best = t;
  }
  return add(from, mul(dir, best));
}

/** 曲線の描き方(円弧=3点、ベジェ=4点) */
export type CurveShape = "arc" | "bezier";

/** 曲線の折り目の設定(コンテキストパネルで切り替える) */
export interface CurveOptions {
  /** 曲線モードか(オフなら今までどおり2クリックの直線) */
  enabled: boolean;
  shape: CurveShape;
  /** 分割数。null=自動(誤差 DEFAULT_CURVE_TOL 以内になるまで細かくする) */
  segments: number | null;
  /** 紙が曲がるための線も一緒に引くか(これが無いと曲線折りは折れない) */
  rulings: boolean;
}

export const DEFAULT_CURVE: CurveOptions = {
  enabled: false,
  shape: "arc",
  segments: null,
  rulings: true,
};

/** その描き方に必要なクリックの数 */
export const CURVE_STEPS: Record<CurveShape, number> = { arc: 3, bezier: 4 };

export const CURVE_LABEL: Record<CurveShape, string> = {
  arc: "円弧",
  bezier: "ベジェ",
};

/**
 * 集めたクリック(と今のカーソル位置)から曲線の折れ線を作る。
 * 点が足りなければ、足りない分をカーソル位置で補って描いている最中の形を返す。
 * 円弧は[始点, 終点, 通過点]、ベジェは[始点, 終点, 制御点1, 制御点2]の順。
 */
export function curvePolyline(
  shape: CurveShape,
  points: Vec2[],
  opts: { segments: number | null; tol?: number },
): Vec2[] | null {
  const tol = opts.tol ?? DEFAULT_CURVE_TOL;
  const n = opts.segments ?? undefined;
  if (points.length < 2) return null;
  if (shape === "arc") {
    const [p0, p1, through] = points;
    if (through === undefined) return [p0, p1];
    return arcPolyline(p0, through, p1, tol, n);
  }
  const [p0, p1, c1, c2] = points;
  if (c1 === undefined) return [p0, p1];
  return cubicPolyline(p0, c1, c2 ?? c1, p1, tol, n);
}

/** 次に何をすればよいかの案内(1行)。doneは済んだクリックの数 */
export function curveHint(shape: CurveShape, done: number, rulings: boolean): string {
  const steps: Record<CurveShape, string[]> = {
    arc: ["曲線の始点をクリック", "曲線の終点をクリック", "曲線を通したい点をクリック"],
    bezier: [
      "曲線の始点をクリック",
      "曲線の終点をクリック",
      "始点側の引っぱり先をクリック",
      "終点側の引っぱり先をクリック",
    ],
  };
  const list = steps[shape];
  const text = list[Math.min(done, list.length - 1)];
  const extra = rulings ? "" : "(曲がるための線なし)";
  return `${CURVE_LABEL[shape]}${extra}: ${text}(Escで中止)`;
}
