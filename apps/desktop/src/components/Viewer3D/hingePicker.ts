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
/** 実距離がこれ以下なら、共平面の候補として層順で決める */
export const SURFACE_HIT_DISTANCE_EPS = 1e-8;

export interface HingeSegment {
  edgeId: number;
  a: THREE.Vector3;
  b: THREE.Vector3;
  /** 太い強調線の中心が面の境界上にあるとき、紙の内側を示す点。 */
  surfaceProbe?: THREE.Vector3;
  /** この線分を持つ面。共有折り目は面ごとに1本ずつ持つ */
  ownerFace?: number;
  /** 所属面の層順。同一深度の可視面判定とそろえる */
  layer?: number;
}

/** 線を拾う前に、同じ画素の表面所有面を調べるための紙面 */
export interface PaperPickSurface {
  mesh: THREE.Mesh;
  triangleFaceIds: number[];
  triangleLayers: number[];
}

/** Raycasterの1交点を、描画と同じ表面所有順で比較できる形にしたもの */
export interface PaperHitCandidate {
  face: number;
  layer: number;
  distance: number;
  point: THREE.Vector3;
  normal: THREE.Vector3;
}

/**
 * 向きが反対でも同じ平面なら同じ法線にする。
 * 丸め誤差の小さな成分で符号が決まらないよう、絶対値最大の成分を正にする。
 */
export function canonicalizeHitNormal(normal: THREE.Vector3): THREE.Vector3 {
  const out = normal.clone().normalize();
  const components = [out.x, out.y, out.z];
  let axis = 0;
  for (let i = 1; i < components.length; i++) {
    if (Math.abs(components[i]) > Math.abs(components[axis])) axis = i;
  }
  if (components[axis] < 0) out.multiplyScalar(-1);
  return out;
}

/**
 * 同じ画素に当たった面から表面所有面を選ぶ純粋関数。
 * 実距離の最小を優先し、`SURFACE_HIT_DISTANCE_EPS`以内の共平面は
 * canonical法線の+側から見ると大きい層/面ID、-側なら小さい層/面IDを選ぶ。
 */
export function selectPaperHit(
  hits: readonly PaperHitCandidate[],
  cameraPosition: THREE.Vector3,
): PaperHitCandidate | null {
  if (hits.length === 0) return null;
  let minimumDistance = Number.POSITIVE_INFINITY;
  for (const hit of hits) minimumDistance = Math.min(minimumDistance, hit.distance);
  const tied = hits.filter(
    (hit) => hit.distance - minimumDistance <= SURFACE_HIT_DISTANCE_EPS,
  );
  const reference = tied[0];
  const normal = canonicalizeHitNormal(reference.normal);
  const positiveSide = normal.dot(cameraPosition.clone().sub(reference.point)) >= 0;
  let selected = reference;
  for (let i = 1; i < tied.length; i++) {
    const candidate = tied[i];
    const layerOrder = candidate.layer - selected.layer;
    const faceOrder = candidate.face - selected.face;
    const order = layerOrder || faceOrder;
    if ((positiveSide && order > 0) || (!positiveSide && order < 0)) {
      selected = candidate;
    } else if (order === 0 && candidate.distance < selected.distance) {
      selected = candidate;
    }
  }
  return selected;
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

/** Raycasterの全交点を面ID/層と結び、描画と同じ順序で所有面を1つに決める */
function pickPaperHit(
  mesh: THREE.Mesh,
  triangleFaceIds: number[],
  camera: THREE.Camera,
  widthPx: number,
  heightPx: number,
  x: number,
  y: number,
  triangleLayers?: number[],
): PaperHitCandidate | null {
  const raycaster = new THREE.Raycaster();
  raycaster.setFromCamera(
    new THREE.Vector2((x / widthPx) * 2 - 1, 1 - (y / heightPx) * 2),
    camera,
  );
  const normalMatrix = new THREE.Matrix3().getNormalMatrix(mesh.matrixWorld);
  const hits: PaperHitCandidate[] = [];
  for (const hit of raycaster.intersectObject(mesh, false)) {
    if (hit.faceIndex == null || hit.face == null) continue;
    const face = triangleFaceIds[hit.faceIndex];
    if (face === undefined) continue;
    hits.push({
      face,
      layer: triangleLayers?.[hit.faceIndex] ?? 0,
      distance: hit.distance,
      point: hit.point.clone(),
      normal: hit.face.normal.clone().applyNormalMatrix(normalMatrix).normalize(),
    });
  }
  return selectPaperHit(hits, camera.getWorldPosition(new THREE.Vector3()));
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
  triangleLayers?: number[],
): number | null {
  return pickPaperHit(
    mesh,
    triangleFaceIds,
    camera,
    widthPx,
    heightPx,
    x,
    y,
    triangleLayers,
  )?.face ?? null;
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
  triangleLayers?: number[],
): { face: number; point: THREE.Vector3 } | null {
  const hit = pickPaperHit(
    mesh,
    triangleFaceIds,
    camera,
    widthPx,
    heightPx,
    x,
    y,
    triangleLayers,
  );
  return hit ? { face: hit.face, point: hit.point.clone() } : null;
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
  surface?: PaperPickSurface,
): number | null {
  return pickHingeSegment(
    segments,
    camera,
    widthPx,
    heightPx,
    x,
    y,
    thresholdPx,
    surface,
  )?.edgeId ?? null;
}

/**
 * `pickHinge`と同じ優先順位で、IDだけでなく正確な表示線分を返す。
 * 汎用層操作で既存折り目を鏡映軸にするとき、手描きの近似線に戻さないために使う。
 */
export function pickHingeSegment(
  segments: HingeSegment[],
  camera: THREE.Camera,
  widthPx: number,
  heightPx: number,
  x: number,
  y: number,
  thresholdPx: number = PICK_THRESHOLD_PX,
  surface?: PaperPickSurface,
): HingeSegment | null {
  const surfaceFace = surface
    ? pickFace(
        surface.mesh,
        surface.triangleFaceIds,
        camera,
        widthPx,
        heightPx,
        x,
        y,
        surface.triangleLayers,
      )
    : null;
  const candidates: { segment: HingeSegment; bucket: number; depth: number }[] = [];
  for (const seg of segments) {
    // 所有面付きの新しい線だけを可視面で絞る。古いowner無し線と
    // 紙面に当たらない場所は、従来の距離判定をそのまま使う。
    if (
      surfaceFace !== null &&
      seg.ownerFace !== undefined &&
      seg.ownerFace !== surfaceFace
    ) {
      continue;
    }
    const a = project(seg.a, camera, widthPx, heightPx);
    const b = project(seg.b, camera, widthPx, heightPx);
    if (!a || !b) continue;
    const dist = distanceToSegment(x, y, a.x, a.y, b.x, b.y);
    if (dist > thresholdPx) continue;
    candidates.push({
      segment: seg,
      bucket: Math.round(dist / DISTANCE_BUCKET_PX),
      depth: (a.depth + b.depth) / 2,
    });
  }
  if (candidates.length === 0) return null;
  candidates.sort((p, q) => p.bucket - q.bucket || p.depth - q.depth);
  return candidates[0].segment;
}
