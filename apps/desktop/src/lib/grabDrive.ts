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

import type { Document, Face, Frame3D } from "./types";

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

/** つかんだ紙を引くときに動かす折り線と、その効き具合 */
export interface PullPlan {
  /** 角度を変える折り線の辺ID */
  hinge: number;
  /** 角度を1ラジアン変えたときにつかんだ点が動く速度(モーメントアーム) */
  velocity: Vec3;
  /** 今の角度(度)。ドラッグ量はここからの差として足す */
  baseDeg: number;
}

/**
 * つかんだ面を動かすのに使う折り線を選ぶ。
 * 経路上の各折り線について、角度を1ラジアン変えたときのつかんだ点の速度
 * v = u × (P − a) を求め、ドラッグの向きへの効き |v・d| がいちばん大きい1本を選ぶ
 * (回転軸から遠い=モーメントアームが大きく、かつ向きも合うものが選ばれる)。
 * まだ向きが分からない(つかんだ瞬間)ときは dragDir を省くと、単純に
 * モーメントアームがいちばん大きい1本を選ぶ。
 * 経路が無い(つかんだ面が根そのもの)ときは null。
 */
export function planPull(
  doc: Document,
  faces: Face[],
  frame: Frame3D | null,
  faceId: number,
  grabPoint: Vec3,
  dragDir: Vec3 = [0, 0, 0],
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
    best = { hinge: ax.hinge, velocity, baseDeg: angles.get(ax.hinge) ?? 0 };
  }
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
