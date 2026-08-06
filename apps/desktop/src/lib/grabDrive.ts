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

import { MIRROR_EPS, isSameSegment, type Segment } from "./mirror";
import {
  buildSegmentIndex,
  findMirrorAxes,
  findSegment,
  reflectSegment,
  type MirrorLine,
  type SegmentIndex,
} from "./symmetry";
import type { Document, Face, Frame3D, Vec2 } from "./types";

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

/**
 * 根の面をつかんだときに動かせる折り線と、その回転軸(UI-007)。
 *
 * ソルバーは根の面をその場に固定するので、根の面をつかんでも「経路」が空になり、
 * これまでは動かせなかった。しかし実際の紙は**どこをつかんでも動かせる**——
 * 手で持った所が止まって見え、残りの紙が逆向きに動くだけである。
 * そこで根の面をつかんだときは、その面に接する折り線を駆動し、
 * 「相手側が逆に動く」ぶんをつかんだ点の見かけの速度として返す
 * (速度の符号を反転する)。こうすると引いた向きへ紙が開いて見える。
 */
export function rootAxes(
  doc: Document,
  faces: Face[],
  frame: Frame3D | null,
  faceId: number,
): DriveAxis[] {
  const polys = facePolygons3D(doc, faces, frame);
  const face = edgeIndex(faces).get(faceId);
  const poly = polys.get(faceId);
  if (!face || !poly || poly.length !== face.edges.length) return [];
  const owners = hingeOwners(faces);
  const out: DriveAxis[] = [];
  for (let j = 0; j < face.edges.length; j++) {
    const hinge = face.edges[j];
    if (!owners.has(hinge)) continue; // 輪郭(片側にしか面が無い辺)は動かせない
    const u = normalize(sub(poly[(j + 1) % poly.length], poly[j]));
    if (u) out.push({ hinge, a: poly[j], u });
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

/**
 * 対称軸を探すのに必要なものをまとめた索引(引き始めに1回だけ作る)。
 * 折り線(両側に面がある辺)の位置と、そこから見つけた対称軸を効かせたい順に持つ。
 */
export interface MirrorIndex {
  segs: SegmentIndex;
  axes: MirrorLine[];
}

/** 面の全域木で親を持たない面(=ソルバーが固定する基準の面)の展開図上の多角形 */
function rootPolygons(doc: Document, faces: Face[]): Vec2[] {
  const pos = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const parents = faceParents(faces);
  const out: Vec2[] = [];
  for (const f of faces) {
    if (parents.has(f.id)) continue;
    for (const v of f.vertices) {
      const p = pos.get(v);
      if (p) out.push(p);
    }
  }
  return out;
}

/**
 * 展開図から対称軸を見つけて索引にまとめる。引き始めに1回だけ呼べば足りるよう、
 * 同じ展開図に対する結果は使い回す(大きな展開図でもドラッグ中は再計算しない)。
 */
const mirrorCache = new WeakMap<Document, MirrorIndex>();

export function buildMirrorIndex(doc: Document, faces: Face[]): MirrorIndex {
  const cached = mirrorCache.get(doc);
  if (cached) return cached;
  const pos = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const owners = hingeOwners(faces);
  const items: [number, Segment][] = [];
  for (const e of doc.cp.edges) {
    const a = pos.get(e.v0);
    const b = pos.get(e.v1);
    if (owners.has(e.id) && a && b) items.push([e.id, [a, b]]);
  }
  const segs = buildSegmentIndex(items);
  const built = { segs, axes: findMirrorAxes(doc.paper, segs, rootPolygons(doc, faces)) };
  mirrorCache.set(doc, built);
  return built;
}

/**
 * 対称軸をはさんで、その折り線と対称の位置にある折り線を探す(UI-007)。
 * 鶴の羽のように左右対になった折り線を一緒に動かすために使う。
 *
 * 軸は紙の形から決め打ちせず、[`buildMirrorIndex`] が展開図そのものから見つける。
 * 軸が複数見つかったときは、効かせたい順に当てて最初に相手が見つかった軸を使う。
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
  const ix = buildMirrorIndex(doc, faces);
  const seg = ix.segs.items.find(([id]) => id === hinge)?.[1];
  if (!seg) return null;
  for (const ax of ix.axes) {
    const other = reflectSegment(seg, ax);
    // 軸の上に乗っている(折り返しても自分自身)なら、そこが体の真ん中。
    // 別の軸を当てると体の中心線を足の折り目と対にしてしまうので、ここで打ち切る
    if (isSameSegment(seg, other, eps)) return null;
    const found = findSegment(ix.segs, other, eps);
    if (found !== null && found !== hinge) return found;
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
 *
 * つかんだ面が根そのもので経路が空になるときは [`rootAxes`] へ切り替え、
 * その面に接する折り線を「相手側が逆に動く」向き(速度の符号を反転)で駆動する。
 * 実際の紙はどこをつかんでも動かせるので、折り線が1本でもつながっていれば
 * nullを返さない(まったく折り線の無い1枚きりの面のときだけnull)。
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
  // 経路が空(=つかんだ面が根)なら、根に接する折り線を「逆向き」に駆動する
  let axes = pathAxes(doc, faces, frame, faceId);
  const sign = axes.length > 0 ? 1 : -1;
  if (axes.length === 0) axes = rootAxes(doc, faces, frame, faceId);
  let best: PullPlan | null = null;
  let bestScore = 0;
  for (const ax of axes) {
    const v = cross(ax.u, sub(grabPoint, ax.a));
    const velocity: Vec3 = [sign * v[0], sign * v[1], sign * v[2]];
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
