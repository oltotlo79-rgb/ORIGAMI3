// 3Dビューのcamera回転とframing。sceneやcontentの資源を所有しない。
import * as THREE from "three";
import {
  cameraQuaternionLookingAt,
  orbitCameraOffset,
  transportCameraScreenUp,
} from "./viewCube";

/** カメラ画角(度) */
export const CAMERA_FOV = 45;
/** 初期カメラの向き(紙の中心から見た方向。斜め上=手前上から見下ろす) */
export const CAMERA_DIR = new THREE.Vector3(0.35, -0.85, 0.95).normalize();
/**
 * 直す前の「視点を戻す」が使っていた距離の係数。
 *
 * 紙の長辺/(2 tan(画角/2)) にこれを掛けた距離へカメラを置いていた。縦の画角しか
 * 見ていないので、区画が縦長になると紙が左右へはみ出した(実測: 区画200×600 CSS pxで
 * 左へ136px・右へ189px)。いまはこの式を「直す前より紙を小さくしない」下限としてだけ使う。
 */
const LEGACY_CAMERA_MARGIN = 1.35;
/** 強調表示の伸ばす向き(回転計算に使う) */
const AXIS_Y = new THREE.Vector3(0, 1, 0);

// ---------------------------------------------------------------------------
// 視点のドラッグ回転
// ---------------------------------------------------------------------------

/** 注視点からカメラへ向かうずれと、そのときの画面の上向き。 */
export interface CameraOrbitPose {
  /** 注視点からカメラへ向かうずれ。長さは注視点までの距離。 */
  offset: THREE.Vector3;
  /** 画面の上向き(世界座標の単位ベクトル)。視線と直角を保つ。 */
  screenUp: THREE.Vector3;
}

/** カメラの向きが定まらないほど注視点に近いときの下限。 */
export const MIN_ORBIT_RADIUS = 1e-9;
/**
 * 1段で回してよいドラッグ量の上限(画面高さに対する割合)。
 *
 * 回す量は「画面高さいっぱいで1回転(360度)」なので、1/8は45度にあたる。
 * orbitCameraOffset は上下の角を0〜180度へ収めるため、いまの向き(常に90度)から
 * 一度に90度を超えて動かすと頭打ちになる。素早いドラッグで1回の通知に
 * 大きな移動量がまとまっても頭打ちにしないよう、45度ずつに分けて回す。
 */
const MAX_ORBIT_STEP_RATIO = 1 / 8;

/** 現在の姿勢から読み取る画面の上向き。 */
export function cameraScreenUp(camera: THREE.Camera): THREE.Vector3 {
  return AXIS_Y.clone().applyQuaternion(camera.quaternion).normalize();
}

/**
 * ドラッグ量だけ視点を回した後のずれと画面の上向き。
 *
 * 回す量の決め方は視点立方体のドラッグと同じ viewCube.ts の orbitCameraOffset を、
 * 画面の上向きの持ち運びも同じ transportCameraScreenUp を使う。
 * 回し方を増やさないため、この関数以外に回転の計算を置かない。
 *
 * 違いは「どの枠で回すか」だけにした。世界の上下を軸にすると、真上・真下が
 * 可動域の端になり、そこで止まる(実測: 「視点を戻す」直後の向きからは
 * 下向きへ49.98度しか回らず、上向きへ130.02度で行き止まりになった)。
 * そこで、いまの画面の上向きが枠の上になるように移してから回す。
 * カメラは常に枠の赤道上(上下角90度)にいるので、どちらへ何周しても端に届かない。
 */
export function rotateCameraByDrag(
  offset: THREE.Vector3,
  screenUp: THREE.Vector3,
  dragX: number,
  dragY: number,
  canvasHeight: number,
): CameraOrbitPose {
  const height = Math.max(canvasHeight, 1);
  const limit = height * MAX_ORBIT_STEP_RATIO;
  const steps = Math.max(
    1,
    Math.ceil(Math.max(Math.abs(dragX), Math.abs(dragY)) / limit),
  );
  let currentOffset = offset.clone();
  let currentUp = screenUp.clone().normalize();
  for (let i = 0; i < steps; i += 1) {
    // 画面の上向きを枠の上へそろえる回転。枠を戻せば世界の向きに戻る。
    const toFrame = new THREE.Quaternion().setFromUnitVectors(currentUp, AXIS_Y);
    const fromFrame = toFrame.clone().invert();
    const nextOffset = orbitCameraOffset(
      currentOffset.clone().applyQuaternion(toFrame),
      dragX / steps,
      dragY / steps,
      height,
    ).applyQuaternion(fromFrame);
    currentUp = transportCameraScreenUp(currentOffset, nextOffset, currentUp);
    currentOffset = nextOffset;
    // 積み重ねた丸めで視線と直角からずれないよう、毎段そろえ直す。
    const forward = currentOffset.clone().normalize();
    const orthogonal = currentUp.clone().projectOnPlane(forward);
    if (orthogonal.lengthSq() > MIN_ORBIT_RADIUS ** 2) {
      currentUp = orthogonal.normalize();
    }
  }
  return { offset: currentOffset, screenUp: currentUp };
}

/**
 * 注視点を中心に、ドラッグ量だけカメラを回す。
 * 注視点と距離は変えないので、寄る・平行移動には影響しない。
 */
export function applyCameraDragRotation(
  camera: THREE.PerspectiveCamera,
  target: THREE.Vector3,
  dragX: number,
  dragY: number,
  canvasHeight: number,
): void {
  const offset = camera.position.clone().sub(target);
  if (offset.lengthSq() <= MIN_ORBIT_RADIUS ** 2) return;
  const pose = rotateCameraByDrag(
    offset,
    cameraScreenUp(camera),
    dragX,
    dragY,
    canvasHeight,
  );
  camera.position.copy(target).add(pose.offset);
  // 上向きを姿勢に合わせて持ち歩くと、寄る・平行移動のあとの
  // OrbitControls の向き直し(lookAt)でも画面の傾きが失われない。
  camera.up.copy(pose.screenUp);
  camera.quaternion.copy(
    cameraQuaternionLookingAt(camera.position, target, pose.screenUp),
  );
  camera.updateMatrixWorld(true);
}

/** OrbitControls のボタン割り当て(ツールごとに setDrawMode が入れ替える)。 */
export interface ViewRotationMouseButtons {
  LEFT?: THREE.MOUSE | null;
  MIDDLE?: THREE.MOUSE | null;
  RIGHT?: THREE.MOUSE | null;
}

/**
 * 押したボタンが視点回転を始めるかどうか。
 * OrbitControls の判定(回転ボタン+修飾キーなしで回転、
 * 平行移動ボタン+修飾キーで回転)をそのまま写す。
 */
export function viewRotationStarts(
  buttons: ViewRotationMouseButtons,
  button: number,
  panModifierHeld: boolean,
): boolean {
  const action =
    button === 0
      ? buttons.LEFT
      : button === 1
        ? buttons.MIDDLE
        : button === 2
          ? buttons.RIGHT
          : null;
  if (action === THREE.MOUSE.ROTATE) return !panModifierHeld;
  if (action === THREE.MOUSE.PAN) return panModifierHeld;
  return false;
}

// ---------------------------------------------------------------------------
// 紙を3D区画へ収める視点合わせ
// ---------------------------------------------------------------------------

/** 画面上の四角形(3D区画の左上を原点としたCSS px)。 */
export interface ScreenBounds {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

/** 視点合わせの結果。カメラの距離と、区画へ当てはめる投影の枠。 */
export interface PaperFraming {
  /** 紙の中心からカメラまでの距離 */
  distance: number;
  /** camera.setViewOffset へ渡す仮想の枠の大きさ */
  fullWidth: number;
  fullHeight: number;
  /** 仮想の枠の中で3D区画が占める位置 */
  offsetX: number;
  offsetY: number;
  /** このとき紙の四隅が画面上で作る四角形 */
  bounds: ScreenBounds;
}

/** 紙と3D区画の縁との間に必ず残す隙間(CSS px)。狭い区画では区画の5%まで詰める。 */
const VIEW_EDGE_PADDING_PX = 8;
/** 案内の札の下端から、さらに空けたい隙間(CSS px)。 */
const HINT_CLEARANCE_PX = 8;
/**
 * 案内の札の下から紙を出すためだけに許す縮小の下限(直す前の紙の高さに対する割合)。
 *
 * 実測(説明書を撮ったときの区画605×439 CSS px): 札の下端107pxより下へ紙の上端を
 * 逃がすには、紙の高さを326→316px(3.1%減)まで詰める必要があった。0.95を下限にすると
 * この区画では札を避けられる。札が区画の4割を占める605×261では、いくら詰めても
 * 避けられないので、この判定で「避けない」側に倒れ、紙は直す前の大きさのまま残る。
 */
const HINT_AVOID_MIN_HEIGHT_RATIO = 0.95;
/** 画角の半分の正接。距離と画面上の大きさを結ぶ係数。 */
const HALF_FOV_TAN = Math.tan((CAMERA_FOV * Math.PI) / 360);

/** 紙の四隅を、カメラの右向き・上向き・視線方向の成分へ分けた値。 */
interface PaperCorner {
  /** 視線方向の成分(手前ほど大きい)。距離から引くと奥行きになる */
  along: number;
  /** 画面の右向きの成分 */
  right: number;
  /** 画面の上向きの成分 */
  up: number;
}

/** 直す前の「視点を戻す」が置いていたカメラ距離相当。立体の大きさの下限に使う。 */
export function legacyBoxDistance(size: THREE.Vector3): number {
  return (
    (Math.max(size.x, size.y, size.z) / (2 * HALF_FOV_TAN)) * LEGACY_CAMERA_MARGIN
  );
}

/** 直す前の「視点を戻す」が置いていたカメラ距離。いまは紙の大きさの下限に使う。 */
export function legacyPaperDistance(paperWidth: number, paperHeight: number): number {
  return legacyBoxDistance(new THREE.Vector3(paperWidth, paperHeight, 0));
}

/**
 * 立体(bounding box)の8頂点を、その中心を原点としてカメラの向きの成分へ分ける。
 *
 * 折り上がった立体は展開図の(0,0)〜(紙の幅,紙の高さ)の範囲や中心には収まらない
 * (折る・技法で座標が動くため)。視点合わせは常に「実際にいま表示している形の
 * 広がり」を基準にする必要があり、この関数がその基準になる。
 */
export function boxCorners(
  box: THREE.Box3,
  dir: THREE.Vector3,
  right: THREE.Vector3,
  up: THREE.Vector3,
): PaperCorner[] {
  const center = box.getCenter(new THREE.Vector3());
  const xs = [box.min.x, box.max.x];
  const ys = [box.min.y, box.max.y];
  const zs = [box.min.z, box.max.z];
  const corners: PaperCorner[] = [];
  for (const x of xs) {
    for (const y of ys) {
      for (const z of zs) {
        const v = new THREE.Vector3(x - center.x, y - center.y, z - center.z);
        corners.push({ along: v.dot(dir), right: v.dot(right), up: v.dot(up) });
      }
    }
  }
  return corners;
}

/** 紙(平らな展開図)の四隅を、紙の中心を原点としてカメラの向きの成分へ分ける。 */
export function paperCorners(
  paperWidth: number,
  paperHeight: number,
  dir: THREE.Vector3,
  right: THREE.Vector3,
  up: THREE.Vector3,
): PaperCorner[] {
  return boxCorners(
    new THREE.Box3(
      new THREE.Vector3(0, 0, 0),
      new THREE.Vector3(paperWidth, paperHeight, 0),
    ),
    dir,
    right,
    up,
  );
}

/**
 * カメラの軸を区画の上から axisY の高さに置くための、仮想の枠の高さ。
 * 区画がこの枠の中へ収まる最小の大きさにすると、画面上の倍率が一つに決まる。
 */
function framingFullHeight(axisY: number, viewHeight: number): number {
  return 2 * Math.max(axisY, viewHeight - axisY);
}

/** 仮想の枠の高さから、世界の長さ1が画面上で何画素になるかの係数を出す。 */
function framingScale(axisY: number, viewHeight: number): number {
  return framingFullHeight(axisY, viewHeight) / (2 * HALF_FOV_TAN);
}

/** 軸を axisY に置いたとき、四隅が区画へ隙間つきで収まる最小の距離。 */
function framingDistance(
  corners: readonly PaperCorner[],
  axisY: number,
  viewWidth: number,
  viewHeight: number,
  padding: number,
): number {
  const scale = framingScale(axisY, viewHeight);
  const roomX = Math.max(viewWidth / 2 - padding, 1e-6);
  const roomUp = Math.max(axisY - padding, 1e-6);
  const roomDown = Math.max(viewHeight - padding - axisY, 1e-6);
  let distance = 0;
  for (const c of corners) {
    distance = Math.max(distance, c.along + (Math.abs(c.right) * scale) / roomX);
    distance = Math.max(
      distance,
      c.along + (Math.abs(c.up) * scale) / (c.up > 0 ? roomUp : roomDown),
    );
  }
  return distance;
}

/** 軸を axisY・距離を distance にしたときの、紙の四隅が作る画面上の四角形。 */
function framingBounds(
  corners: readonly PaperCorner[],
  axisY: number,
  distance: number,
  viewWidth: number,
  viewHeight: number,
): ScreenBounds {
  const scale = framingScale(axisY, viewHeight);
  let left = Infinity;
  let right = -Infinity;
  let top = Infinity;
  let bottom = -Infinity;
  for (const c of corners) {
    const depth = Math.max(distance - c.along, 1e-6);
    const x = viewWidth / 2 + (c.right * scale) / depth;
    const y = axisY - (c.up * scale) / depth;
    left = Math.min(left, x);
    right = Math.max(right, x);
    top = Math.min(top, y);
    bottom = Math.max(bottom, y);
  }
  return { left, right, top, bottom };
}

/**
 * 紙の上端が targetTop まで下がるように軸を下げる。ただし紙の高さが minHeight を
 * 割る手前で止める。下げるほど紙は小さくなるので、二分探索で境目を求める。
 */
function lowerAxis(
  corners: readonly PaperCorner[],
  viewWidth: number,
  viewHeight: number,
  padding: number,
  minHeight: number,
  targetTop: number,
): number {
  let lo = viewHeight / 2;
  let hi = viewHeight - padding;
  for (let i = 0; i < 60; i++) {
    const mid = (lo + hi) / 2;
    const bounds = framingBounds(
      corners,
      mid,
      framingDistance(corners, mid, viewWidth, viewHeight, padding),
      viewWidth,
      viewHeight,
    );
    if (bounds.bottom - bounds.top >= minHeight && bounds.top <= targetTop) lo = mid;
    else hi = mid;
  }
  return lo;
}

/**
 * 立体全体が3D区画へ収まり、できるだけ大きく、できるだけ左上の案内の札に
 * 隠れない視点を求める。
 *
 * 直す前は縦の画角だけで距離を決めていたため、区画の縦横比を無視していた。
 * ここでは四隅を実際に投影して左右・上下の4方向すべてを見るので、区画が
 * 縦長でも横長でもはみ出さない。
 *
 * 札を避けるのは「立体を小さくしすぎない」範囲だけにする。札が区画の高さの
 * 大部分を占める低い区画では避けきれないので、そのときは直す前の大きさを保ったまま、
 * 空いている下側の余りぶんだけ立体を下げる。
 *
 * @param box 実際にいま表示している立体(または紙)が占める範囲。展開図の大きさや
 *   中心ではなく、常にこの実測の範囲を基準にする(折る・技法で座標が動くため)。
 * @param hintBottomPx 左上の案内の札の下端(区画の上からのCSS px)。0なら札なし。
 */
export function boxFraming(
  box: THREE.Box3,
  viewWidth: number,
  viewHeight: number,
  hintBottomPx: number,
  dir: THREE.Vector3,
  right: THREE.Vector3,
  up: THREE.Vector3,
): PaperFraming {
  const corners = boxCorners(box, dir, right, up);
  const size = box.getSize(new THREE.Vector3());
  const padding = Math.min(
    VIEW_EDGE_PADDING_PX,
    viewWidth * 0.05,
    viewHeight * 0.05,
  );
  // 直す前の大きさ。ここを下回らないようにする
  const legacyBounds = framingBounds(
    corners,
    viewHeight / 2,
    legacyBoxDistance(size),
    viewWidth,
    viewHeight,
  );
  const legacyHeight = legacyBounds.bottom - legacyBounds.top;

  const centeredAxis = viewHeight / 2;
  const centeredDistance = framingDistance(
    corners,
    centeredAxis,
    viewWidth,
    viewHeight,
    padding,
  );
  const centeredBounds = framingBounds(
    corners,
    centeredAxis,
    centeredDistance,
    viewWidth,
    viewHeight,
  );
  const centeredHeight = centeredBounds.bottom - centeredBounds.top;
  // 札の下端より下へ紙の上端を置きたい。区画の半分より下げることはしない
  const targetTop = Math.min(hintBottomPx + HINT_CLEARANCE_PX, viewHeight / 2);

  let axisY = centeredAxis;
  if (centeredBounds.top < targetTop) {
    // まず少しだけ縮めてよい条件で避けられるか試す
    const withShrink = lowerAxis(
      corners,
      viewWidth,
      viewHeight,
      padding,
      Math.min(centeredHeight, legacyHeight * HINT_AVOID_MIN_HEIGHT_RATIO),
      targetTop,
    );
    const shrunkTop = framingBounds(
      corners,
      withShrink,
      framingDistance(corners, withShrink, viewWidth, viewHeight, padding),
      viewWidth,
      viewHeight,
    ).top;
    axisY =
      shrunkTop >= hintBottomPx
        ? withShrink // 縮めた甲斐があった(札の下から出た)
        : lowerAxis(
            // 避けきれないので縮めない。空いているぶんだけ下げる
            corners,
            viewWidth,
            viewHeight,
            padding,
            Math.min(centeredHeight, legacyHeight),
            targetTop,
          );
  }

  const distance = framingDistance(corners, axisY, viewWidth, viewHeight, padding);
  const fullHeight = framingFullHeight(axisY, viewHeight);
  return {
    distance,
    fullWidth: viewWidth,
    fullHeight,
    offsetX: 0,
    offsetY: fullHeight / 2 - axisY,
    bounds: framingBounds(corners, axisY, distance, viewWidth, viewHeight),
  };
}

/**
 * 紙(平らな展開図)全体を基準にした視点合わせ。{@link boxFraming} の薄い包み。
 * 折り上がった立体には使わない(実際の広がりと合わないため)。テストと
 * 「まだ立体が無い」ときのフォールバックのために残す。
 */
export function paperFraming(
  paperWidth: number,
  paperHeight: number,
  viewWidth: number,
  viewHeight: number,
  hintBottomPx: number,
  dir: THREE.Vector3,
  right: THREE.Vector3,
  up: THREE.Vector3,
): PaperFraming {
  return boxFraming(
    new THREE.Box3(
      new THREE.Vector3(0, 0, 0),
      new THREE.Vector3(paperWidth, paperHeight, 0),
    ),
    viewWidth,
    viewHeight,
    hintBottomPx,
    dir,
    right,
    up,
  );
}

/**
 * 求めた枠をカメラの投影へ入れる。
 * setViewOffset は「仮想の枠のうち、この四角形だけを区画へ描く」指定で、
 * 縦横比も同時に決まる。区画の大きさが変わるたびに入れ直す。
 */
export function applyPaperFraming(
  camera: THREE.PerspectiveCamera,
  framing: PaperFraming,
  viewWidth: number,
  viewHeight: number,
): void {
  camera.setViewOffset(
    framing.fullWidth,
    framing.fullHeight,
    framing.offsetX,
    framing.offsetY,
    viewWidth,
    viewHeight,
  );
}


