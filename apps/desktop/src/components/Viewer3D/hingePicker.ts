// 3Dビュー上のヒンジ(折れる辺)の拾い上げ。
// カメラで辺を画面へ投影し、クリック位置との画面距離がしきい値以内で
// 最も近い辺を選ぶ(Raycasterの当たり判定は太さが世界座標基準になり、
// ズーム倍率によって拾いやすさが変わってしまうため画面距離で判定する)。
// 線分そのもの(edgeId・両端の座標)はsceneBuilderが立体形状から作って持つ。

import * as THREE from "three";

/** クリック位置から辺までの許容距離(px) */
export const PICK_THRESHOLD_PX = 10;
/** 「同じくらい近い」とみなす刻み(px)。この刻みで並べた上で手前を優先する */
const DISTANCE_BUCKET_PX = 0.5;

export interface HingeSegment {
  edgeId: number;
  a: THREE.Vector3;
  b: THREE.Vector3;
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

/**
 * クリック位置の真下にある面のうち、いちばん手前(視点に近い)のものの面ID。
 * 紙をつかむ操作で「どの層をつかんだか」を決めるのに使う。
 * 層のずらし表示で紙が持ち上がっていても、実際に描かれている三角形を当てるので
 * 平面へ投影する方法(最大で長辺の3%ずれる)より正確に拾える。
 */
export function pickFace(
  mesh: THREE.Mesh,
  triangleFaceIds: number[],
  camera: THREE.Camera,
  widthPx: number,
  heightPx: number,
  x: number,
  y: number,
): number | null {
  const raycaster = new THREE.Raycaster();
  raycaster.setFromCamera(
    new THREE.Vector2((x / widthPx) * 2 - 1, 1 - (y / heightPx) * 2),
    camera,
  );
  // 最初の交点がいちばん手前(Raycasterは距離順に返す)
  for (const hit of raycaster.intersectObject(mesh, false)) {
    const id = hit.faceIndex == null ? undefined : triangleFaceIds[hit.faceIndex];
    if (id !== undefined) return id;
  }
  return null;
}

/**
 * pickFaceと同じ拾い方で、当たった面IDと当たった位置(世界座標)を返す。
 * 紙をつかんで引く操作では、つかんだ位置が回転のモーメントアームになるので
 * 面だけでなく点も要る。
 */
export function pickPaper(
  mesh: THREE.Mesh,
  triangleFaceIds: number[],
  camera: THREE.Camera,
  widthPx: number,
  heightPx: number,
  x: number,
  y: number,
): { face: number; point: THREE.Vector3 } | null {
  const raycaster = new THREE.Raycaster();
  raycaster.setFromCamera(
    new THREE.Vector2((x / widthPx) * 2 - 1, 1 - (y / heightPx) * 2),
    camera,
  );
  for (const hit of raycaster.intersectObject(mesh, false)) {
    const id = hit.faceIndex == null ? undefined : triangleFaceIds[hit.faceIndex];
    if (id !== undefined) return { face: id, point: hit.point.clone() };
  }
  return null;
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
 * しきい値より遠ければnull(=選択解除)。
 * しきい値内の候補を全て集めてから「距離(0.5px刻み)→手前」の順に並べ替えて
 * 先頭を選ぶ(1本ずつ比べると、並び順によって奥の線が残ることがあるため)。
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
  const candidates: { edgeId: number; bucket: number; depth: number }[] = [];
  for (const seg of segments) {
    const a = project(seg.a, camera, widthPx, heightPx);
    const b = project(seg.b, camera, widthPx, heightPx);
    if (!a || !b) continue;
    const dist = distanceToSegment(x, y, a.x, a.y, b.x, b.y);
    if (dist > thresholdPx) continue;
    candidates.push({
      edgeId: seg.edgeId,
      bucket: Math.round(dist / DISTANCE_BUCKET_PX),
      depth: (a.depth + b.depth) / 2,
    });
  }
  if (candidates.length === 0) return null;
  candidates.sort((p, q) => p.bucket - q.bucket || p.depth - q.depth);
  return candidates[0].edgeId;
}
