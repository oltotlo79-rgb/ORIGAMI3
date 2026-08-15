import * as THREE from "three";

/** 立方体を収める右上の外接枠。配置検査もこの大きさを使う。 */
export const VIEW_CUBE_SIZE_PX = 96;
/** 3D表示部分の上端・右端からの距離。 */
export const VIEW_CUBE_INSET_PX = 12;
/** 左側の重ね表示が立方体へ入らないために空ける幅。 */
export const VIEW_CUBE_CLEARANCE_PX = 120;
/** 面を選んだときの視点移動時間。既存に補間が無いため0.3秒とする。 */
export const VIEW_CUBE_TRANSITION_MS = 300;
/** これを超えて動いたら面クリックではなく視点ドラッグとみなす。 */
export const VIEW_CUBE_CLICK_MOVE_PX = 4;

export type ViewCubeFace = "front" | "back" | "left" | "right" | "top" | "bottom";

export interface ViewCubeFaceDefinition {
  id: ViewCubeFace;
  label: string;
  /** 紙の中心からカメラを置く向き。画面には出さない。 */
  cameraDirection: readonly [number, number, number];
}

/**
 * 紙の表側法線は+zなので、前/後は紙の表/裏に対応させる。
 * 残る4面は紙を正面から見たときの上下左右である。
 */
export const VIEW_CUBE_FACES: readonly ViewCubeFaceDefinition[] = [
  { id: "front", label: "前", cameraDirection: [0, 0, 1] },
  { id: "back", label: "後", cameraDirection: [0, 0, -1] },
  { id: "left", label: "左", cameraDirection: [-1, 0, 0] },
  { id: "right", label: "右", cameraDirection: [1, 0, 0] },
  { id: "top", label: "上", cameraDirection: [0, 1, 0] },
  { id: "bottom", label: "下", cameraDirection: [0, -1, 0] },
] as const;

const FACE_BY_ID = new Map(VIEW_CUBE_FACES.map((face) => [face.id, face]));
const MIN_CAMERA_DISTANCE = 1e-9;
const POLAR_EPSILON_RAD = 1e-6;

/** 選んだ面を見るため、紙の中心からカメラを置く単位方向。 */
export function cameraDirectionForFace(face: ViewCubeFace): THREE.Vector3 {
  const direction = FACE_BY_ID.get(face)?.cameraDirection;
  if (!direction) throw new Error(`Unknown view-cube face: ${face}`);
  return new THREE.Vector3(...direction);
}

/** 選んだ面から紙の中心へ向かう、期待される視線方向。 */
export function viewDirectionForFace(face: ViewCubeFace): THREE.Vector3 {
  return cameraDirectionForFace(face).multiplyScalar(-1);
}

/** 上下の真正面でも画面の上下が曖昧にならないための、終端だけの上向き。 */
export function cameraUpForFace(face: ViewCubeFace): THREE.Vector3 {
  if (face === "top") return new THREE.Vector3(0, 0, -1);
  if (face === "bottom") return new THREE.Vector3(0, 0, 1);
  return new THREE.Vector3(0, 1, 0);
}

/** 選んだ面を正面にする終端のカメラ位置。現在の距離は保つ。 */
export function cameraPositionForFace(
  face: ViewCubeFace,
  target: THREE.Vector3,
  distance: number,
): THREE.Vector3 {
  return target
    .clone()
    .addScaledVector(cameraDirectionForFace(face), Math.max(distance, MIN_CAMERA_DISTANCE));
}

/** カメラ位置から注視点へ向かう単位視線。 */
export function cameraViewDirection(
  position: THREE.Vector3,
  target: THREE.Vector3,
): THREE.Vector3 {
  return target.clone().sub(position).normalize();
}

/** 2方向の角度差を度で返す。受け入れ検査の0.5度判定に使う。 */
export function directionAngleDeg(a: THREE.Vector3, b: THREE.Vector3): number {
  return THREE.MathUtils.radToDeg(a.angleTo(b));
}

/** 0..1を、始点と終点で速度0になる滑らかな進み方へ変える。 */
export function smoothViewProgress(progress: number): number {
  const t = THREE.MathUtils.clamp(progress, 0, 1);
  return t * t * (3 - 2 * t);
}

/**
 * 距離を変えず、球面上の短い経路で2方向を補間する。
 * 真反対の面でも中心を通り抜けない。
 */
export function interpolateCameraOffset(
  from: THREE.Vector3,
  to: THREE.Vector3,
  progress: number,
): THREE.Vector3 {
  const fromDistance = from.length();
  const toDistance = to.length();
  if (fromDistance <= MIN_CAMERA_DISTANCE || toDistance <= MIN_CAMERA_DISTANCE) {
    return from.clone().lerp(to, THREE.MathUtils.clamp(progress, 0, 1));
  }
  const t = THREE.MathUtils.clamp(progress, 0, 1);
  const rotation = new THREE.Quaternion().setFromUnitVectors(
    from.clone().normalize(),
    to.clone().normalize(),
  );
  const partial = new THREE.Quaternion().slerp(rotation, t);
  const distance = THREE.MathUtils.lerp(fromDistance, toDistance, t);
  return from.clone().normalize().applyQuaternion(partial).multiplyScalar(distance);
}

export interface InterpolatedCameraPose {
  offset: THREE.Vector3;
  /** カメラの画面上方向を表す世界座標の単位ベクトル。 */
  screenUp: THREE.Vector3;
}

/** 2つの単位ベクトル間で、axis周りの符号付き角度を返す。 */
function signedAngleAround(
  from: THREE.Vector3,
  to: THREE.Vector3,
  axis: THREE.Vector3,
): number {
  return Math.atan2(axis.dot(from.clone().cross(to)), from.dot(to));
}

/**
 * 位置と画面上向きを同じ球面回転で運び、最後のrollだけ視線軸周りで補う。
 * これにより反対面への移動中も紙の中心が画面中央から外れない。
 */
export function interpolateCameraPose(
  fromOffset: THREE.Vector3,
  toOffset: THREE.Vector3,
  fromScreenUp: THREE.Vector3,
  toScreenUp: THREE.Vector3,
  progress: number,
): InterpolatedCameraPose {
  const fromDistance = fromOffset.length();
  const toDistance = toOffset.length();
  if (fromDistance <= MIN_CAMERA_DISTANCE || toDistance <= MIN_CAMERA_DISTANCE) {
    return {
      offset: fromOffset.clone().lerp(toOffset, THREE.MathUtils.clamp(progress, 0, 1)),
      screenUp: fromScreenUp.clone().lerp(toScreenUp, progress).normalize(),
    };
  }

  const t = THREE.MathUtils.clamp(progress, 0, 1);
  const fromDirection = fromOffset.clone().normalize();
  const toDirection = toOffset.clone().normalize();
  const wholeRotation = new THREE.Quaternion().setFromUnitVectors(
    fromDirection,
    toDirection,
  );
  const partialRotation = new THREE.Quaternion().slerp(wholeRotation, t);
  const offset = fromDirection
    .clone()
    .applyQuaternion(partialRotation)
    .multiplyScalar(THREE.MathUtils.lerp(fromDistance, toDistance, t));

  const transportedUp = fromScreenUp.clone().normalize().applyQuaternion(partialRotation);
  const transportedEndUp = fromScreenUp.clone().normalize().applyQuaternion(wholeRotation);
  const endViewDirection = toDirection.clone().multiplyScalar(-1);
  const roll = signedAngleAround(
    transportedEndUp,
    toScreenUp.clone().normalize(),
    endViewDirection,
  );
  const currentViewDirection = offset.clone().normalize().multiplyScalar(-1);
  const screenUp = transportedUp
    .applyQuaternion(new THREE.Quaternion().setFromAxisAngle(currentViewDirection, roll * t))
    .normalize();
  return { offset, screenUp };
}

/**
 * ドラッグ前後のカメラ位置を結ぶ回転で、現在の画面上方向も一緒に運ぶ。
 * 面移動の途中からドラッグへ切り替えても、rollを急に失わない。
 */
export function transportCameraScreenUp(
  fromOffset: THREE.Vector3,
  toOffset: THREE.Vector3,
  fromScreenUp: THREE.Vector3,
): THREE.Vector3 {
  if (
    fromOffset.lengthSq() <= MIN_CAMERA_DISTANCE ** 2 ||
    toOffset.lengthSq() <= MIN_CAMERA_DISTANCE ** 2
  ) {
    return fromScreenUp.clone().normalize();
  }
  const rotation = new THREE.Quaternion().setFromUnitVectors(
    fromOffset.clone().normalize(),
    toOffset.clone().normalize(),
  );
  return fromScreenUp.clone().normalize().applyQuaternion(rotation).normalize();
}

/**
 * OrbitControlsと同じ画面高さ基準の角度でカメラ位置を回す。
 * 横は方位、縦は上下を変え、極を越えて反転しない。
 */
export function orbitCameraOffset(
  initialOffset: THREE.Vector3,
  dragX: number,
  dragY: number,
  canvasHeight: number,
): THREE.Vector3 {
  const height = Math.max(canvasHeight, 1);
  const spherical = new THREE.Spherical().setFromVector3(initialOffset);
  spherical.theta -= (2 * Math.PI * dragX) / height;
  spherical.phi -= (2 * Math.PI * dragY) / height;
  spherical.phi = THREE.MathUtils.clamp(
    spherical.phi,
    POLAR_EPSILON_RAD,
    Math.PI - POLAR_EPSILON_RAD,
  );
  return new THREE.Vector3().setFromSpherical(spherical);
}

/**
 * OrbitControlsが公開しない注視点を、直前の注視点と更新後のカメラから追跡する。
 * 回転・拡大縮小では同じ点を保ち、画面内の平行移動ではカメラと同じだけ動く。
 */
export function trackedOrbitTarget(
  previousTarget: THREE.Vector3,
  cameraPosition: THREE.Vector3,
  cameraViewDirection: THREE.Vector3,
): THREE.Vector3 {
  const forward = cameraViewDirection.clone();
  if (forward.lengthSq() <= MIN_CAMERA_DISTANCE ** 2) return previousTarget.clone();
  forward.normalize();
  const distanceAlongView = previousTarget.clone().sub(cameraPosition).dot(forward);
  if (!Number.isFinite(distanceAlongView) || distanceAlongView <= MIN_CAMERA_DISTANCE) {
    return previousTarget.clone();
  }
  return cameraPosition.clone().addScaledVector(forward, distanceAlongView);
}

/** 指定位置から注視点を見るカメラ姿勢。元のupは変えない。 */
export function cameraQuaternionLookingAt(
  position: THREE.Vector3,
  target: THREE.Vector3,
  up: THREE.Vector3,
): THREE.Quaternion {
  return new THREE.Quaternion().setFromRotationMatrix(
    new THREE.Matrix4().lookAt(position, target, up),
  );
}

/**
 * 世界に固定した立方体を、現在のカメラから見た向きへ回すCSS行列。
 * CSSの下向きYとThree.jsの上向きYを入口と出口で反転する。
 */
export function cubeCssMatrixElements(cameraQuaternion: THREE.Quaternion): number[] {
  const flipY = new THREE.Matrix4().makeScale(1, -1, 1);
  const viewRotation = new THREE.Matrix4().makeRotationFromQuaternion(
    cameraQuaternion.clone().invert(),
  );
  const matrix = flipY.clone().multiply(viewRotation).multiply(flipY);
  return matrix.elements.map((value) => (Math.abs(value) < 1e-12 ? 0 : value));
}

export function cubeCssTransform(cameraQuaternion: THREE.Quaternion): string {
  return `matrix3d(${cubeCssMatrixElements(cameraQuaternion).join(",")})`;
}

export interface OverlayRect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface ViewCubeOverlayRects {
  cube: OverlayRect;
  /** 左上の「いまのモード」。高さは変わるため、下端まで予約して検査する。 */
  modeHint: OverlayRect;
  /** 文字幅より大きい保守的な幅で置いた右下のボタン。 */
  resetButton: OverlayRect;
}

/** 画面描画に依存せず、重ね表示の予約領域を数値で検査する。 */
export function viewCubeOverlayRects(width: number, height: number): ViewCubeOverlayRects {
  return {
    cube: {
      left: width - VIEW_CUBE_INSET_PX - VIEW_CUBE_SIZE_PX,
      top: VIEW_CUBE_INSET_PX,
      right: width - VIEW_CUBE_INSET_PX,
      bottom: VIEW_CUBE_INSET_PX + VIEW_CUBE_SIZE_PX,
    },
    modeHint: {
      left: VIEW_CUBE_INSET_PX,
      top: VIEW_CUBE_INSET_PX,
      right: width - VIEW_CUBE_CLEARANCE_PX,
      bottom: height,
    },
    resetButton: {
      left: width - 120,
      top: height - 38,
      right: width - 8,
      bottom: height - 8,
    },
  };
}

export function overlayRectsOverlap(a: OverlayRect, b: OverlayRect): boolean {
  return a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
}

/** 正ならそのpx数だけ横にはみ出している。0なら左右とも画面内。 */
export function horizontalOverflowPx(rect: OverlayRect, width: number): number {
  return Math.max(0, -rect.left, rect.right - width);
}
