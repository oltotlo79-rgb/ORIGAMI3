// 3Dビュー上のヒンジ(折れる辺)の拾い上げ。
// カメラで辺を画面へ投影し、クリック位置との画面距離がしきい値以内で
// 最も近い辺を選ぶ(Raycasterの当たり判定は太さが世界座標基準になり、
// ズーム倍率によって拾いやすさが変わってしまうため画面距離で判定する)。

import * as THREE from "three";
import { hingeEdgeIds } from "../../lib/hinges";
import type { Document, Face, Frame3D } from "../../lib/types";

/** クリック位置から辺までの許容距離(px) */
export const PICK_THRESHOLD_PX = 10;

export interface HingeSegment {
  edgeId: number;
  a: THREE.Vector3;
  b: THREE.Vector3;
}

/** 現在の立体形状におけるヒンジの線分一覧(辺IDごとに1本) */
export function collectHingeSegments(
  doc: Document,
  faces: Face[],
  frame: Frame3D,
): HingeSegment[] {
  const hinges = hingeEdgeIds(doc, faces);
  const faceById = new Map(faces.map((f) => [f.id, f]));
  const found = new Map<number, HingeSegment>();
  for (const f3 of frame.faces) {
    const face = faceById.get(f3.face);
    if (!face) continue;
    const n = face.vertices.length;
    // 参照切れで頂点が欠けた面は境界と対応が取れないので拾わない
    if (f3.polygon.length !== n) continue;
    for (let i = 0; i < n; i++) {
      const edgeId = face.edges[i];
      if (!hinges.has(edgeId) || found.has(edgeId)) continue;
      const p = f3.polygon[i];
      const q = f3.polygon[(i + 1) % n];
      found.set(edgeId, {
        edgeId,
        a: new THREE.Vector3(p[0], p[1], p[2]),
        b: new THREE.Vector3(q[0], q[1], q[2]),
      });
    }
  }
  return [...found.values()];
}

/** 世界座標を画面座標(px)へ。カメラの後ろ側ならnull */
function project(
  point: THREE.Vector3,
  camera: THREE.Camera,
  widthPx: number,
  heightPx: number,
): { x: number; y: number; depth: number } | null {
  const v = point.clone().project(camera);
  if (v.z < -1 || v.z > 1) return null;
  return {
    x: ((v.x + 1) / 2) * widthPx,
    y: ((1 - v.y) / 2) * heightPx,
    depth: v.z,
  };
}

/** 点(px, py)から線分(ax,ay)-(bx,by)までの距離(px) */
function distanceToSegment(
  px: number,
  py: number,
  ax: number,
  ay: number,
  bx: number,
  by: number,
): number {
  const dx = bx - ax;
  const dy = by - ay;
  const len2 = dx * dx + dy * dy;
  const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / len2));
  return Math.hypot(px - (ax + dx * t), py - (ay + dy * t));
}

/**
 * クリック位置(canvas左上基準のpx)に最も近いヒンジの辺IDを返す。
 * しきい値より遠ければnull(=選択解除)。同じくらい近い辺は手前のものを選ぶ。
 */
export function pickHinge(
  segments: HingeSegment[],
  camera: THREE.Camera,
  widthPx: number,
  heightPx: number,
  x: number,
  y: number,
  thresholdPx: number = PICK_THRESHOLD_PX,
): number | null {
  let bestId: number | null = null;
  let bestDist = thresholdPx;
  let bestDepth = Infinity;
  for (const seg of segments) {
    const a = project(seg.a, camera, widthPx, heightPx);
    const b = project(seg.b, camera, widthPx, heightPx);
    if (!a || !b) continue;
    const dist = distanceToSegment(x, y, a.x, a.y, b.x, b.y);
    const depth = (a.depth + b.depth) / 2;
    if (dist > bestDist) continue;
    // ほぼ同じ距離なら手前(depthが小さい)を優先する
    if (dist > bestDist - 0.5 && depth >= bestDepth) continue;
    bestId = seg.edgeId;
    bestDist = dist;
    bestDepth = depth;
  }
  return bestId;
}
