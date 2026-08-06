// 「3Dの紙をつかんで引く」操作の幾何(UI-007)。Three.jsに依存しない純粋な計算。
//
// つかんだ面を動かすには、その面と根の面をつなぐ折り線のどれかの角度を変えればよい。
// Rust側(ori3-rigid::tree)と同じ全域木を組み立て、つかんだ面から根までの経路の
// 折り線それぞれについて「角度を1ラジアン変えたときにつかんだ点が動く速度」
// v = 軸方向 × (つかんだ点 − 軸上の点) を求め、ドラッグの向きに最もよく効く
// (=モーメントアームが大きく向きも合う)1本を選ぶ。
//
// 折り上がった作品からも引けるように、今見えている立体の形から全ての折り線の
// 角度を読み取る関数も置く(ソルバーの出発点=warm startをその形に合わせるため)。

import { MIRROR_EPS, isSameSegment, mirrorAxisX, type Segment } from "./mirror";
import type { Document, Face, Frame3D, Paper, Vec2 } from "./types";

export type Vec3 = [number, number, number];

/** 面の法線が真逆(=平らに畳まれている)とみなす内積のしきい値 */
const FOLDED_DOT = -0.999999;

export function sub(a: Vec3, b: Vec3): Vec3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
}

export function cross(a: Vec3, b: Vec3): Vec3 {
  return [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ];
}

export function dot(a: Vec3, b: Vec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}

export function length(a: Vec3): number {
  return Math.hypot(a[0], a[1], a[2]);
}

/** 単位ベクトル(長さ0なら null) */
export function normalize(a: Vec3): Vec3 | null {
  const len = length(a);
  return len < 1e-12 ? null : [a[0] / len, a[1] / len, a[2] / len];
}

/**
 * 面ID → 今の立体での多角形(頂点は face.vertices と同順)。
 * frameがnull、または頂点数が合わない面は展開図どおり(z=0)の平らな形で埋める。
 */
export function facePolygons3D(
  doc: Document,
  faces: Face[],
  frame: Frame3D | null,
): Map<number, Vec3[]> {
  const pos = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const solid = new Map(frame?.faces.map((f) => [f.face, f.polygon]) ?? []);
  const out = new Map<number, Vec3[]>();
  for (const f of faces) {
    const p = solid.get(f.id);
    if (p && p.length === f.vertices.length) {
      out.set(f.id, p.map((q) => [q[0], q[1], q[2]] as Vec3));
      continue;
    }
    const flat: Vec3[] = [];
    for (const v of f.vertices) {
      const q = pos.get(v);
      if (q) flat.push([q[0], q[1], 0]);
    }
    if (flat.length === f.vertices.length) out.set(f.id, flat);
  }
  return out;
}

/** 折り線の辺ID → その辺を境界に持つ2つの面ID(辺ID昇順。Rust側と同じ定義) */
export function hingeOwners(faces: Face[]): Map<number, [number, number]> {
  const occ = new Map<number, number[]>();
  for (const f of faces) {
    for (const e of f.edges) {
      const list = occ.get(e);
      if (list) list.push(f.id);
      else occ.set(e, [f.id]);
    }
  }
  const out = new Map<number, [number, number]>();
  for (const [e, list] of [...occ].sort((a, b) => a[0] - b[0])) {
    if (list.length === 2 && list[0] !== list[1]) out.set(e, [list[0], list[1]]);
  }
  return out;
}

/** 多角形の法線(Newell法)。面積0なら null */
export function polygonNormal(poly: Vec3[]): Vec3 | null {
  let n: Vec3 = [0, 0, 0];
  for (let i = 0; i < poly.length; i++) {
    const a = poly[i];
    const b = poly[(i + 1) % poly.length];
    n = [
      n[0] + (a[1] * b[2] - a[2] * b[1]),
      n[1] + (a[2] * b[0] - a[0] * b[2]),
      n[2] + (a[0] * b[1] - a[1] * b[0]),
    ];
  }
  return normalize(n);
}

/** 1回のドラッグで動かせる角度の上限(度)。行き過ぎで形が飛ぶのを防ぐ */
export const MAX_PULL_DEG = 170;

/** 面ID → 親の面IDと、そこへつながる折り線 */
export interface FaceLink {
  parent: number;
  hinge: number;
}

/**
 * 面の隣接グラフのBFS全域木(Rust側 ori3-rigid::tree::build_forest と同じ組み立て)。
 * 根は連結成分ごとに最初に現れる面で、走査は折り線の辺ID昇順。根の面は
 * ソルバーがその場に固定するので、他の面は根からの経路の角度で位置が決まる。
 */
export function faceParents(faces: Face[]): Map<number, FaceLink> {
  const adj = new Map<number, { hinge: number; other: number }[]>();
  const link = (from: number, hinge: number, other: number) => {
    const list = adj.get(from);
    if (list) list.push({ hinge, other });
    else adj.set(from, [{ hinge, other }]);
  };
  for (const [hinge, [a, b]] of hingeOwners(faces)) {
    link(a, hinge, b);
    link(b, hinge, a);
  }
  const parents = new Map<number, FaceLink>();
  const visited = new Set<number>();
  for (const start of faces) {
    if (visited.has(start.id)) continue;
    visited.add(start.id);
    const queue = [start.id];
    while (queue.length > 0) {
      const cur = queue.shift()!;
      for (const { hinge, other } of adj.get(cur) ?? []) {
        if (visited.has(other)) continue;
        visited.add(other);
        parents.set(other, { parent: cur, hinge });
        queue.push(other);
      }
    }
  }
  return parents;
}

/** 折り線の回転軸(今の立体での位置)。向きは親面の反時計回り境界に合わせる */
export interface DriveAxis {
  hinge: number;
  a: Vec3;
  u: Vec3;
}

/** つかんだ面から根の面までの経路にある折り線と、その回転軸 */
export function pathAxes(
  doc: Document,
  faces: Face[],
  frame: Frame3D | null,
  faceId: number,
): DriveAxis[] {
  const polys = facePolygons3D(doc, faces, frame);
  const byId = edgeIndex(faces);
  const parents = faceParents(faces);
  const out: DriveAxis[] = [];
  const seen = new Set<number>([faceId]);
  let cur = faceId;
  for (let link = parents.get(cur); link; link = parents.get(cur)) {
    const face = byId.get(link.parent);
    const poly = polys.get(link.parent);
    const j = face?.edges.indexOf(link.hinge) ?? -1;
    if (face && poly && j >= 0 && poly.length === face.edges.length) {
      const u = normalize(sub(poly[(j + 1) % poly.length], poly[j]));
      if (u) out.push({ hinge: link.hinge, a: poly[j], u });
    }
    cur = link.parent;
    if (seen.has(cur)) break; // 念のための無限ループ止め
    seen.add(cur);
  }
  return out;
}

/** 面ID → 境界の辺IDの並び(頂点列と同順) */
function edgeIndex(faces: Face[]): Map<number, Face> {
  return new Map(faces.map((f) => [f.id, f]));
}

/**
 * 今見えている立体の形から、全ての折り線の角度(度)を読み取る。
 * 角度は隣り合う2面の法線のなす符号付き角(軸まわり)で、+180=山/−180=谷。
 *
 * 平らに畳まれた所(法線が真逆)は幾何だけでは±180の符号が決まらないので、
 * 展開図の線種で決める(Rust側 fold_through::angle_of と同じ対応)。
 * この読み取りは、折り上がった作品からでも引けるようにソルバーの出発点
 * (warm start)を今の形へ合わせるために使う。
 */
export function hingeAnglesFromFrame(
  doc: Document,
  faces: Face[],
  frame: Frame3D | null,
): Map<number, number> {
  const polys = facePolygons3D(doc, faces, frame);
  const byId = edgeIndex(faces);
  const kinds = new Map(doc.cp.edges.map((e) => [e.id, e.kind]));
  const out = new Map<number, number>();
  for (const [edgeId, [fa, fb]] of hingeOwners(faces)) {
    const pa = polys.get(fa);
    const pb = polys.get(fb);
    const face = byId.get(fa);
    if (!pa || !pb || !face) continue;
    const na = polygonNormal(pa);
    const nb = polygonNormal(pb);
    const j = face.edges.indexOf(edgeId);
    if (!na || !nb || j < 0) continue;
    const u = normalize(sub(pa[(j + 1) % pa.length], pa[j]));
    if (!u) continue;
    const d = Math.min(1, Math.max(-1, dot(na, nb)));
    out.set(
      edgeId,
      d < FOLDED_DOT
        ? kinds.get(edgeId) === "Valley"
          ? -180
          : 180
        : (Math.atan2(dot(cross(na, nb), u), d) * 180) / Math.PI,
    );
  }
  return out;
}

/** 展開図の対称軸(点pを通り、単位ベクトルdの向きの直線) */
interface MirrorLine {
  p: Vec2;
  d: Vec2;
}

/** 直線axで折り返した点 */
function reflectPoint(q: Vec2, ax: MirrorLine): Vec2 {
  const w: Vec2 = [q[0] - ax.p[0], q[1] - ax.p[1]];
  const t = w[0] * ax.d[0] + w[1] * ax.d[1];
  return [ax.p[0] + 2 * t * ax.d[0] - w[0], ax.p[1] + 2 * t * ax.d[1] - w[1]];
}

/**
 * 「左右」を折り返す軸の候補(効かせたい順)。
 *
 * まずは紙の縦の中心線(左右対称に線を引く CPE-010 と同じ軸)。ただし、この
 * アプリの折り操作で折った作品は紙の対角線が体の軸になることが多い(折り鶴は
 * 正方形を半分に2回折って組み立てるので、首と尾が向かい合う2隅、羽が残りの
 * 2隅から出る)。そこで正方形の紙では対角線も候補に入れる。
 * 縦・横に長い紙では対角線は対称軸になり得ないので入れない。
 */
function mirrorLines(paper: Paper): MirrorLine[] {
  const long = Math.max(paper.width_mm, paper.height_mm);
  const w = long > 0 ? paper.width_mm / long : 0;
  const h = long > 0 ? paper.height_mm / long : 0;
  const lines: MirrorLine[] = [{ p: [mirrorAxisX(paper), 0], d: [0, 1] }];
  if (Math.abs(w - h) <= MIRROR_EPS) {
    lines.push({ p: [0, 0], d: [Math.SQRT1_2, Math.SQRT1_2] });
    lines.push({ p: [0, h], d: [Math.SQRT1_2, -Math.SQRT1_2] });
  }
  return lines;
}

/**
 * 対称軸をはさんで、その折り線と対称の位置にある折り線を探す(UI-007)。
 * 鶴の羽のように左右対になった折り線を一緒に動かすために使う。
 *
 * 次のときは「もう1本動かす意味がない」のでnullを返す(呼ぶ側は1本だけ動かす):
 *   - その折り線が軸の上に乗っている(折り返しても自分自身)
 *   - 折り返した線がもとの線と同じ(軸に直交して軸をまたぐ線)
 *   - 折り返した位置に折り線が無い(左右対称でない展開図)
 * 相手は「両側に面がある折り線」に限る(輪郭を動かしても形は変わらないため)。
 */
export function mirrorHingeOf(
  doc: Document,
  faces: Face[],
  hinge: number,
  eps = MIRROR_EPS,
): number | null {
  const pos = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const owners = hingeOwners(faces);
  const segs: [number, Segment][] = [];
  for (const e of doc.cp.edges) {
    const a = pos.get(e.v0);
    const b = pos.get(e.v1);
    if (owners.has(e.id) && a && b) segs.push([e.id, [a, b]]);
  }
  const seg = segs.find(([id]) => id === hinge)?.[1];
  if (!seg) return null;
  for (const ax of mirrorLines(doc.paper)) {
    const other: Segment = [reflectPoint(seg[0], ax), reflectPoint(seg[1], ax)];
    if (isSameSegment(seg, other, eps)) continue; // 軸の上・軸に対称な線
    const found = segs.find(([, s]) => isSameSegment(s, other, eps));
    if (found) return found[0];
  }
  return null;
}

/** つかんだ紙を引くときに動かす折り線と、その効き具合 */
export interface PullPlan {
  /** 角度を変える折り線の辺ID */
  hinge: number;
  /** 角度を1ラジアン変えたときにつかんだ点が動く速度(モーメントアーム) */
  velocity: Vec3;
  /** 今の角度(度)。ドラッグ量はここからの差として足す */
  baseDeg: number;
  /** 左右対称の相手の折り線(同じ角度で一緒に動かす)。無ければnull */
  mirrorHinge: number | null;
}

/**
 * つかんだ面を動かすのに使う折り線を選ぶ。
 * 経路上の各折り線について、角度を1ラジアン変えたときのつかんだ点の速度
 * v = u × (P − a) を求め、ドラッグの向きへの効き |v・d| がいちばん大きい1本を選ぶ
 * (回転軸から遠い=モーメントアームが大きく、かつ向きも合うものが選ばれる)。
 * まだ向きが分からない(つかんだ瞬間)ときは dragDir を省くと、単純に
 * モーメントアームがいちばん大きい1本を選ぶ。
 * 経路が無い(つかんだ面が根そのもの)ときは null。
 * mirror=trueなら、選んだ折り線と左右対称の相手も一緒に動かす相手として返す。
 */
export function planPull(
  doc: Document,
  faces: Face[],
  frame: Frame3D | null,
  faceId: number,
  grabPoint: Vec3,
  dragDir: Vec3 = [0, 0, 0],
  mirror = false,
): PullPlan | null {
  const angles = hingeAnglesFromFrame(doc, faces, frame);
  const dir = normalize(dragDir);
  let best: PullPlan | null = null;
  let bestScore = 0;
  for (const ax of pathAxes(doc, faces, frame, faceId)) {
    const velocity = cross(ax.u, sub(grabPoint, ax.a));
    const score = dir ? Math.abs(dot(velocity, dir)) : length(velocity);
    if (score <= bestScore) continue;
    bestScore = score;
    best = {
      hinge: ax.hinge,
      velocity,
      baseDeg: angles.get(ax.hinge) ?? 0,
      mirrorHinge: null,
    };
  }
  if (best && mirror) best.mirrorHinge = mirrorHingeOf(doc, faces, best.hinge);
  return best;
}

/**
 * ドラッグ(3Dのベクトル)を、駆動する折り線の角度の変化(度)に直す。
 * つかんだ点が指の動きにいちばん近づく回転量(最小二乗解)を使う。
 */
export function pullDeltaDeg(velocity: Vec3, drag: Vec3): number {
  const v2 = dot(velocity, velocity);
  if (v2 < 1e-12) return 0;
  const deg = ((dot(drag, velocity) / v2) * 180) / Math.PI;
  return Math.max(-MAX_PULL_DEG, Math.min(MAX_PULL_DEG, deg));
}
