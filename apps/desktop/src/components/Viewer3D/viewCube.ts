import * as THREE from "three";

/** 立方体を収める右上の外接枠。配置検査もこの大きさを使う。 */
export const VIEW_CUBE_SIZE_PX = 96;
/** 3D表示部分の上端・右端からの距離。 */
export const VIEW_CUBE_INSET_PX = 12;
/** 左側の重ね表示が立方体へ入らないために空ける幅。 */
export const VIEW_CUBE_CLEARANCE_PX = 120;
/** 行き先を選んだときの視点移動時間。既存に補間が無いため0.3秒とする。 */
export const VIEW_CUBE_TRANSITION_MS = 300;
/** これを超えて動いたら選択ではなく視点ドラッグとみなす。 */
export const VIEW_CUBE_CLICK_MOVE_PX = 4;
/** 立方体1面の一辺(px)。 */
export const VIEW_CUBE_FACE_PX = 48;
/** 面の縁に置く帯の幅(px)。ここが辺と角の押し場所になる。 */
export const VIEW_CUBE_BAND_PX = 13;

export type ViewCubeFace = "front" | "back" | "left" | "right" | "top" | "bottom";

export type ViewCubeEdge =
  | "top-front"
  | "top-back"
  | "top-right"
  | "top-left"
  | "bottom-front"
  | "bottom-back"
  | "bottom-right"
  | "bottom-left"
  | "front-right"
  | "front-left"
  | "back-right"
  | "back-left";

export type ViewCubeCorner =
  | "top-front-right"
  | "top-front-left"
  | "top-back-right"
  | "top-back-left"
  | "bottom-front-right"
  | "bottom-front-left"
  | "bottom-back-right"
  | "bottom-back-left";

/** 面6 + 辺12 + 角8 = 26箇所の行き先。 */
export type ViewCubeTarget = ViewCubeFace | ViewCubeEdge | ViewCubeCorner;

export type ViewCubeTargetKind = "face" | "edge" | "corner";

export interface ViewCubeTargetDefinition {
  id: ViewCubeTarget;
  kind: ViewCubeTargetKind;
  /** 画面に出す短い呼び名。面だけが立方体の上に文字として出る。 */
  label: string;
  /** 読み上げ・吹き出し用の文。 */
  actionLabel: string;
  /** 紙の中心からカメラを置く向き(正規化前)。画面には出さない。 */
  direction: readonly [number, number, number];
}

interface AxisPart {
  readonly axis: "x" | "y" | "z";
  readonly positive: { readonly id: string; readonly label: string };
  readonly negative: { readonly id: string; readonly label: string };
}

/**
 * 呼び名を並べる順序。上下 → 前後 → 左右 の順に読むと日本語として自然になる。
 * 紙の表側法線は+zなので、前/後は紙の表/裏に対応する。
 */
const AXIS_PARTS: readonly AxisPart[] = [
  {
    axis: "y",
    positive: { id: "top", label: "上" },
    negative: { id: "bottom", label: "下" },
  },
  {
    axis: "z",
    positive: { id: "front", label: "前" },
    negative: { id: "back", label: "後" },
  },
  {
    axis: "x",
    positive: { id: "right", label: "右" },
    negative: { id: "left", label: "左" },
  },
];

const KIND_BY_PART_COUNT: Record<number, ViewCubeTargetKind> = {
  1: "face",
  2: "edge",
  3: "corner",
};

function describeTarget(x: number, y: number, z: number): ViewCubeTargetDefinition {
  const value = { x, y, z };
  const parts = AXIS_PARTS.filter((part) => value[part.axis] !== 0).map((part) =>
    value[part.axis] > 0 ? part.positive : part.negative,
  );
  const names = parts.map((part) => part.label);
  const kind = KIND_BY_PART_COUNT[parts.length];
  const actionLabel =
    kind === "face"
      ? `${names[0]}を正面にする`
      : kind === "edge"
        ? `${names.join("と")}の間の辺を正面にする`
        : `${names.join("と")}が集まる角を正面にする`;
  return {
    id: parts.map((part) => part.id).join("-") as ViewCubeTarget,
    kind,
    label: names.join(""),
    actionLabel,
    direction: [x, y, z],
  };
}

function buildTargets(): readonly ViewCubeTargetDefinition[] {
  const all: ViewCubeTargetDefinition[] = [];
  for (const y of [1, 0, -1]) {
    for (const z of [1, 0, -1]) {
      for (const x of [1, 0, -1]) {
        if (x === 0 && y === 0 && z === 0) continue;
        all.push(describeTarget(x, y, z));
      }
    }
  }
  const order: Record<ViewCubeTargetKind, number> = { face: 0, edge: 1, corner: 2 };
  return all.sort((a, b) => order[a.kind] - order[b.kind]);
}

/** 面・辺・角の26箇所。面 → 辺 → 角 の順に並ぶ。 */
export const VIEW_CUBE_TARGETS: readonly ViewCubeTargetDefinition[] = buildTargets();

const TARGET_BY_ID = new Map(VIEW_CUBE_TARGETS.map((target) => [target.id, target]));

const FACE_ORDER: readonly ViewCubeFace[] = [
  "front",
  "back",
  "left",
  "right",
  "top",
  "bottom",
];

/** 6面だけを、前・後・左・右・上・下の順で取り出したもの。 */
export const VIEW_CUBE_FACES: readonly ViewCubeTargetDefinition[] = FACE_ORDER.map(
  (id) => {
    const face = TARGET_BY_ID.get(id);
    if (!face) throw new Error(`Unknown view-cube face: ${id}`);
    return face;
  },
);

const MIN_CAMERA_DISTANCE = 1e-9;
const POLAR_EPSILON_RAD = 1e-6;
const WORLD_UP = new THREE.Vector3(0, 1, 0);
const CAMERA_BACKWARD = new THREE.Vector3(0, 0, 1);
const ORIGIN = new THREE.Vector3();

export function viewCubeTarget(id: ViewCubeTarget): ViewCubeTargetDefinition {
  const target = TARGET_BY_ID.get(id);
  if (!target) throw new Error(`Unknown view-cube target: ${id}`);
  return target;
}

export function isViewCubeTarget(value: unknown): value is ViewCubeTarget {
  return typeof value === "string" && TARGET_BY_ID.has(value as ViewCubeTarget);
}

/** 選んだ場所を見るため、紙の中心からカメラを置く単位方向。 */
export function cameraDirectionForTarget(id: ViewCubeTarget): THREE.Vector3 {
  return new THREE.Vector3(...viewCubeTarget(id).direction).normalize();
}

/** 選んだ場所から紙の中心へ向かう、期待される視線方向。 */
export function viewDirectionForTarget(id: ViewCubeTarget): THREE.Vector3 {
  return cameraDirectionForTarget(id).multiplyScalar(-1);
}

/**
 * 終端の画面上向き。真上・真下だけは紙の上下が決まらないため向きを決め打ちし、
 * 残る24箇所は世界の上向きを視線と直角な向きへ落とす。
 * 落とし方はドラッグ操作の上向きの決め方と同じなので、着いた後に姿勢が跳ねない。
 */
export function cameraUpForTarget(id: ViewCubeTarget): THREE.Vector3 {
  if (id === "top") return new THREE.Vector3(0, 0, -1);
  if (id === "bottom") return new THREE.Vector3(0, 0, 1);
  const direction = cameraDirectionForTarget(id);
  return WORLD_UP.clone().projectOnPlane(direction).normalize();
}

/** 選んだ場所を正面にする終端のカメラ位置。現在の距離は保つ。 */
export function cameraPositionForTarget(
  id: ViewCubeTarget,
  target: THREE.Vector3,
  distance: number,
): THREE.Vector3 {
  return target
    .clone()
    .addScaledVector(cameraDirectionForTarget(id), Math.max(distance, MIN_CAMERA_DISTANCE));
}

export interface ViewCubeFacePlate {
  id: ViewCubeFace;
  label: string;
  /** 面の外向き。 */
  normal: readonly [number, number, number];
  /** 面を正面から見たときに画面の右へ向かう向き。 */
  right: readonly [number, number, number];
  /** 面を正面から見たときに画面の上へ向かう向き。 */
  up: readonly [number, number, number];
}

/**
 * 6枚の板の向き。CSSの面の回転(App.css の .view-cube-face-*)と同じ姿勢を、
 * CSSの下向き+yを上向き+yへ直した世界の向きで持つ。
 */
export const VIEW_CUBE_PLATES: readonly ViewCubeFacePlate[] = FACE_ORDER.map((id) => {
  const label = viewCubeTarget(id).label;
  switch (id) {
    case "front":
      return { id, label, normal: [0, 0, 1], right: [1, 0, 0], up: [0, 1, 0] };
    case "back":
      return { id, label, normal: [0, 0, -1], right: [-1, 0, 0], up: [0, 1, 0] };
    case "left":
      return { id, label, normal: [-1, 0, 0], right: [0, 0, 1], up: [0, 1, 0] };
    case "right":
      return { id, label, normal: [1, 0, 0], right: [0, 0, -1], up: [0, 1, 0] };
    case "top":
      return { id, label, normal: [0, 1, 0], right: [1, 0, 0], up: [0, 0, -1] };
    default:
      return { id, label, normal: [0, -1, 0], right: [1, 0, 0], up: [0, 0, 1] };
  }
});

/** 面を3×3に割ったときの列。-1が画面の左、+1が画面の右。 */
export const VIEW_CUBE_ZONE_COLUMNS = [-1, 0, 1] as const;
/** 面を3×3に割ったときの行。+1が画面の上、-1が画面の下。 */
export const VIEW_CUBE_ZONE_ROWS = [1, 0, -1] as const;

export function viewCubeZoneKind(column: number, row: number): ViewCubeTargetKind {
  const away = (column === 0 ? 0 : 1) + (row === 0 ? 0 : 1);
  return away === 0 ? "face" : away === 1 ? "edge" : "corner";
}

/** 面の3×3の1区画が指す行き先。縁の帯が辺、四隅が角、中央が面になる。 */
export function viewCubeZoneTarget(
  plate: ViewCubeFacePlate,
  column: number,
  row: number,
): ViewCubeTarget {
  const direction = new THREE.Vector3(...plate.normal)
    .addScaledVector(new THREE.Vector3(...plate.right), column)
    .addScaledVector(new THREE.Vector3(...plate.up), row);
  const id = [
    Math.round(direction.y),
    Math.round(direction.z),
    Math.round(direction.x),
  ];
  const parts = AXIS_PARTS.map((part, index) => {
    const sign = id[index];
    return sign === 0 ? null : sign > 0 ? part.positive.id : part.negative.id;
  }).filter((part): part is string => part !== null);
  const target = parts.join("-");
  if (!isViewCubeTarget(target)) {
    throw new Error(`Unknown view-cube zone: ${plate.id} ${column},${row}`);
  }
  return target;
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

/** 2つのカメラ姿勢の間の回転角を度で返す。最短経路の検査に使う。 */
export function attitudeAngleDeg(a: THREE.Quaternion, b: THREE.Quaternion): number {
  return THREE.MathUtils.radToDeg(a.angleTo(b));
}

/** 0..1を、始点と終点で速度0になる滑らかな進み方へ変える。 */
export function smoothViewProgress(progress: number): number {
  const t = THREE.MathUtils.clamp(progress, 0, 1);
  return t * t * (3 - 2 * t);
}

/** 注視点からのずれと画面上向きから、カメラの姿勢を1つ作る。 */
export function cameraAttitude(
  offset: THREE.Vector3,
  screenUp: THREE.Vector3,
): THREE.Quaternion {
  return new THREE.Quaternion().setFromRotationMatrix(
    new THREE.Matrix4().lookAt(offset, ORIGIN, screenUp),
  );
}

export interface InterpolatedCameraPose {
  offset: THREE.Vector3;
  /** カメラの画面上方向を表す世界座標の単位ベクトル。 */
  screenUp: THREE.Vector3;
  /** 補間の途中のカメラ姿勢。 */
  attitude: THREE.Quaternion;
}

/**
 * 姿勢そのものを最短の回り方で補間し、位置は姿勢から導く。
 *
 * 以前は「位置を球面で運ぶ」と「最後に画面の傾きを直す」を別々に足していた。
 * 別々の軸まわりの回転を重ねるため、合計の回転が最短より大きくなる。
 * 実測(scratchpad/viewcube2-report.md)では上→下で74.6度、後→上で21.3度余分に回っていた。
 * 姿勢を1本の回転で補間すれば、回る量は始点と終点の姿勢差そのものに一致し、
 * 途中で画面の上下が裏返ることもない。
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
  const t = THREE.MathUtils.clamp(progress, 0, 1);
  if (fromDistance <= MIN_CAMERA_DISTANCE || toDistance <= MIN_CAMERA_DISTANCE) {
    const screenUp = fromScreenUp.clone().lerp(toScreenUp, t).normalize();
    const offset = fromOffset.clone().lerp(toOffset, t);
    return { offset, screenUp, attitude: cameraAttitude(offset, screenUp) };
  }

  const attitude = cameraAttitude(fromOffset, fromScreenUp).slerp(
    cameraAttitude(toOffset, toScreenUp),
    t,
  );
  const distance = THREE.MathUtils.lerp(fromDistance, toDistance, t);
  return {
    offset: CAMERA_BACKWARD.clone().applyQuaternion(attitude).multiplyScalar(distance),
    screenUp: WORLD_UP.clone().applyQuaternion(attitude),
    attitude,
  };
}

/**
 * ドラッグ前後のカメラ位置を結ぶ回転で、現在の画面上方向も一緒に運ぶ。
 * 移動の途中からドラッグへ切り替えても、画面の傾きを急に失わない。
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
