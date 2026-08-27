// 3D表示で拾った点・線を、一意な共通支持面の等長chartへ写して既存solveAlignで解く。
// 8方式の式はここへ複製せず、Viewer側は支持面の判定・座標変換・面内clipだけを担う。

import {
  ALIGN_EPS,
  alignRefPoint,
  movingSideOf,
  solveAlign,
  type FoldLine,
} from "../../lib/alignFold";
import type { AlignMode, AlignTarget, Vec2 } from "../../lib/types";
import type {
  SpatialAlignTarget,
  SpatialFoldTarget,
  SpatialSupportPlane,
  SpatialVec3,
} from "../../lib/spatialAlignTypes";
import type {
  SpatialMaterialFoldInput,
  SpatialMaterialForMovingSide,
} from "../../store/slices/documentSlice";
import {
  facePlaneNormal,
  mapPoint,
  pointInPolygon,
  unmapPoint,
  type FacePlacement,
} from "./edgeHighlight";
import {
  materialFoldLineFrom3D,
  type CpPick3D,
  type MaterialFoldLine,
} from "./cpPick3d";
import type { HingeSegment } from "./hingePicker";

/**
 * f64の単位正方形をFloat32表示頂点へ変換した固定seed 200,000姿勢で、
 * 面への再写像残差の実測最大は2.161808357136193e-7だった。境目2.7e-7では
 * 実測が80.07%になり、余裕0にしない。既存の最小表示層間隔2e-4はこの約741倍なので、
 * 別の支持面を同一視しない。座標は紙の長辺=1の正規化単位。
 */
export const SPATIAL_REPROJECTION_EPS = 2.7e-7;

const MATERIAL_LINE_EQ_EPS = 1e-8;
const CLIP_EPS = 1e-10;
const KEEP_MIN_OFFSET = 2e-9;

interface NormalizedPlane {
  point: SpatialVec3;
  normal: SpatialVec3;
  offset: number;
}

const GLOBAL_FOLDED_PLANE: NormalizedPlane = {
  point: [0, 0, 0],
  normal: [0, 0, 1],
  offset: 0,
};

interface PlaneChart {
  plane: NormalizedPlane;
  origin: SpatialVec3;
  u: SpatialVec3;
  v: SpatialVec3;
}

interface ClippedCandidate {
  target: SpatialFoldTarget;
  materialLine: MaterialFoldLine["material_line"];
  materialForMovingSide: SpatialMaterialForMovingSide;
}

interface ClippedSolution {
  target: SpatialFoldTarget;
  materialForMovingSide: SpatialMaterialForMovingSide;
}

export interface SolveSpatialAlignInput {
  mode: AlignMode;
  picks: readonly SpatialAlignTarget[];
  cursorWorld?: SpatialVec3 | null;
  placements: readonly FacePlacement[];
}

export interface SpatialAlignResult {
  status: "ready" | "unavailable";
  solutions: (SpatialFoldTarget | null)[];
  materialSolutions: (SpatialMaterialForMovingSide | null)[];
  reason: string | null;
  maxReprojectionResidual: number | null;
}

function finite3(point: readonly number[]): point is SpatialVec3 {
  return (
    point.length === 3 &&
    Number.isFinite(point[0]) &&
    Number.isFinite(point[1]) &&
    Number.isFinite(point[2])
  );
}

function vec3(point: readonly number[]): SpatialVec3 {
  return [point[0], point[1], point[2]];
}

function add3(a: SpatialVec3, b: SpatialVec3): SpatialVec3 {
  return [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
}

function sub3(a: SpatialVec3, b: SpatialVec3): SpatialVec3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
}

function mul3(a: SpatialVec3, factor: number): SpatialVec3 {
  return [a[0] * factor, a[1] * factor, a[2] * factor];
}

function dot3(a: SpatialVec3, b: SpatialVec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}

function cross3(a: SpatialVec3, b: SpatialVec3): SpatialVec3 {
  return [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ];
}

function length3(a: SpatialVec3): number {
  return Math.hypot(a[0], a[1], a[2]);
}

function distance3(a: SpatialVec3, b: SpatialVec3): number {
  return length3(sub3(a, b));
}

function unit3(a: SpatialVec3): SpatialVec3 | null {
  const length = length3(a);
  if (!Number.isFinite(length) || length <= ALIGN_EPS) return null;
  return mul3(a, 1 / length);
}

function largestAbsoluteAxis(vector: SpatialVec3): number {
  let axis = 0;
  for (let index = 1; index < 3; index++) {
    if (Math.abs(vector[index]) > Math.abs(vector[axis])) axis = index;
  }
  return axis;
}

function normalizePlane(plane: SpatialSupportPlane): NormalizedPlane | null {
  if (!finite3(plane.point) || !finite3(plane.normal)) return null;
  let normal = unit3(vec3(plane.normal));
  if (normal === null) return null;
  let offset = dot3(normal, vec3(plane.point));
  if (!Number.isFinite(offset)) return null;
  if (normal[largestAbsoluteAxis(normal)] < 0) {
    normal = mul3(normal, -1);
    offset = -offset;
  }
  const withoutNegativeZero = (value: number): number =>
    Object.is(value, -0) ? 0 : value;
  normal = normal.map(withoutNegativeZero) as SpatialVec3;
  offset = withoutNegativeZero(offset);
  return {
    point: mul3(normal, offset).map(withoutNegativeZero) as SpatialVec3,
    normal,
    offset,
  };
}

function planeResidual(plane: NormalizedPlane, point: SpatialVec3): number {
  return Math.abs(dot3(plane.normal, point) - plane.offset);
}

function equivalentPlanes(a: NormalizedPlane, b: NormalizedPlane): boolean {
  return (
    distance3(a.normal, b.normal) <= SPATIAL_REPROJECTION_EPS &&
    planeResidual(a, b.point) <= SPATIAL_REPROJECTION_EPS &&
    planeResidual(b, a.point) <= SPATIAL_REPROJECTION_EPS
  );
}

function comparePlanes(a: NormalizedPlane, b: NormalizedPlane): number {
  for (let axis = 0; axis < 3; axis++) {
    if (a.normal[axis] !== b.normal[axis]) return a.normal[axis] - b.normal[axis];
  }
  return a.offset - b.offset;
}

function targetWorldPoints(target: SpatialAlignTarget): SpatialVec3[] | null {
  if (target.kind === "point") {
    return finite3(target.world) ? [vec3(target.world)] : null;
  }
  if (!finite3(target.aWorld) || !finite3(target.bWorld)) return null;
  const a = vec3(target.aWorld);
  const b = vec3(target.bWorld);
  return distance3(a, b) > ALIGN_EPS ? [a, b] : null;
}

function validPlanesForTarget(target: SpatialAlignTarget): {
  planes: NormalizedPlane[];
  points: SpatialVec3[];
  maximumResidual: number;
} | null {
  const points = targetWorldPoints(target);
  if (points === null || target.supportPlanes.length === 0) return null;
  let maximumResidual = 0;
  const valid: NormalizedPlane[] = [];
  for (const raw of target.supportPlanes) {
    const plane = normalizePlane(raw);
    if (plane === null) continue;
    const residual = Math.max(...points.map((point) => planeResidual(plane, point)));
    maximumResidual = Math.max(maximumResidual, residual);
    if (residual <= SPATIAL_REPROJECTION_EPS) valid.push(plane);
  }
  return { planes: valid, points, maximumResidual };
}

function commonPlaneOf(picks: readonly SpatialAlignTarget[]): {
  plane: NormalizedPlane | null;
  maximumResidual: number;
} {
  if (picks.length === 0) return { plane: null, maximumResidual: 0 };
  const perTarget = picks.map(validPlanesForTarget);
  const observedMaximumResidual = Math.max(
    0,
    ...perTarget.map((entry) => entry?.maximumResidual ?? 0),
  );
  if (perTarget.some((entry) => entry === null || entry.planes.length === 0)) {
    return { plane: null, maximumResidual: observedMaximumResidual };
  }

  // ε近似は推移的ではない。A≈B・B≈C・A≉Cをgreedyに1面へ畳むと、
  // supportPlanesやpickの配列順でready/unavailableが変わる。各実在候補を全targetへ
  // 直接照合し、全targetに同値な支持面があり、かつ全world点が候補自身へ載るものだけを
  // 共通候補にする。target固有の外れ面が近似連鎖しても、真の共通面を巻き込まない。
  const entries = perTarget as Exclude<(typeof perTarget)[number], null>[];
  const common = entries
    .flatMap((entry) => entry.planes)
    .filter((candidate) =>
      entries.every(
        (entry) =>
          entry.planes.some((plane) => equivalentPlanes(candidate, plane)) &&
          entry.points.every(
            (point) =>
              planeResidual(candidate, point) <= SPATIAL_REPROJECTION_EPS,
          ),
      ),
    );
  if (
    common.length === 0 ||
    common.some((candidate, index) =>
      common
        .slice(index + 1)
        .some((other) => !equivalentPlanes(candidate, other)),
    )
  ) {
    return { plane: null, maximumResidual: observedMaximumResidual };
  }
  const plane = [...common].sort(comparePlanes)[0];
  const maximumResidual = Math.max(
    0,
    ...entries.flatMap((entry) =>
      entry.points.map((point) => planeResidual(plane, point)),
    ),
  );
  return {
    plane,
    maximumResidual,
  };
}

function planeChart(plane: NormalizedPlane): PlaneChart | null {
  const axes: SpatialVec3[] = [
    [1, 0, 0],
    [0, 1, 0],
    [0, 0, 1],
  ];
  let axis = 0;
  for (let index = 1; index < axes.length; index++) {
    if (
      Math.abs(dot3(axes[index], plane.normal)) <
      Math.abs(dot3(axes[axis], plane.normal))
    ) {
      axis = index;
    }
  }
  const projected = sub3(
    axes[axis],
    mul3(plane.normal, dot3(axes[axis], plane.normal)),
  );
  const u = unit3(projected);
  if (u === null) return null;
  const v = unit3(cross3(plane.normal, u));
  if (v === null) return null;
  return { plane, origin: plane.point, u, v };
}

function worldToChart(chart: PlaneChart, point: SpatialVec3): Vec2 {
  const delta = sub3(point, chart.origin);
  return [dot3(delta, chart.u), dot3(delta, chart.v)];
}

function chartToWorld(chart: PlaneChart, point: Vec2): SpatialVec3 {
  return add3(
    chart.origin,
    add3(mul3(chart.u, point[0]), mul3(chart.v, point[1])),
  );
}

function targetToChart(target: SpatialAlignTarget, chart: PlaneChart): AlignTarget {
  return target.kind === "point"
    ? { kind: "point", p: worldToChart(chart, vec3(target.world)) }
    : {
        kind: "line",
        a: worldToChart(chart, vec3(target.aWorld)),
        b: worldToChart(chart, vec3(target.bWorld)),
      };
}

function placementPlane(placement: FacePlacement): NormalizedPlane | null {
  const normal = facePlaneNormal(placement);
  if (normal === null) return null;
  return normalizePlane({ point: vec3(placement.q0), normal: vec3(normal) });
}

function cross2(a: Vec2, b: Vec2): number {
  return a[0] * b[1] - a[1] * b[0];
}

function sub2(a: Vec2, b: Vec2): Vec2 {
  return [a[0] - b[0], a[1] - b[1]];
}

function addUniqueParameter(parameters: number[], value: number): void {
  if (!Number.isFinite(value)) return;
  if (!parameters.some((existing) => Math.abs(existing - value) <= CLIP_EPS)) {
    parameters.push(value);
  }
}

function linePolygonIntervals(
  line: FoldLine,
  polygon: readonly Vec2[],
): [number, number][] {
  if (polygon.length < 3) return [];
  const direction = sub2(line[1], line[0]);
  const lengthSquared =
    direction[0] * direction[0] + direction[1] * direction[1];
  if (lengthSquared <= ALIGN_EPS ** 2) return [];
  const parameters: number[] = [];
  for (const vertex of polygon) {
    addUniqueParameter(
      parameters,
      ((vertex[0] - line[0][0]) * direction[0] +
        (vertex[1] - line[0][1]) * direction[1]) /
        lengthSquared,
    );
  }
  for (let index = 0; index < polygon.length; index++) {
    const a = polygon[index];
    const b = polygon[(index + 1) % polygon.length];
    const edge = sub2(b, a);
    const denominator = cross2(direction, edge);
    const fromLine = sub2(a, line[0]);
    if (Math.abs(denominator) <= CLIP_EPS) {
      if (Math.abs(cross2(fromLine, direction)) <= CLIP_EPS) {
        addUniqueParameter(
          parameters,
          ((a[0] - line[0][0]) * direction[0] +
            (a[1] - line[0][1]) * direction[1]) /
            lengthSquared,
        );
        addUniqueParameter(
          parameters,
          ((b[0] - line[0][0]) * direction[0] +
            (b[1] - line[0][1]) * direction[1]) /
            lengthSquared,
        );
      }
      continue;
    }
    const t = cross2(fromLine, edge) / denominator;
    const edgeAt = cross2(fromLine, direction) / denominator;
    if (edgeAt >= -CLIP_EPS && edgeAt <= 1 + CLIP_EPS) {
      addUniqueParameter(parameters, t);
    }
  }
  parameters.sort((a, b) => a - b);
  const raw: [number, number][] = [];
  for (let index = 0; index + 1 < parameters.length; index++) {
    const a = parameters[index];
    const b = parameters[index + 1];
    if (b - a <= CLIP_EPS) continue;
    const middle = (a + b) / 2;
    const point: Vec2 = [
      line[0][0] + direction[0] * middle,
      line[0][1] + direction[1] * middle,
    ];
    if (pointInPolygon(polygon, point, CLIP_EPS)) raw.push([a, b]);
  }
  const merged: [number, number][] = [];
  for (const interval of raw) {
    const previous = merged[merged.length - 1];
    if (previous && interval[0] - previous[1] <= CLIP_EPS) {
      previous[1] = interval[1];
    } else {
      merged.push([interval[0], interval[1]]);
    }
  }
  return merged;
}

function pointLineDistance3(
  point: SpatialVec3,
  line: readonly [SpatialVec3, SpatialVec3],
): number {
  const direction = sub3(line[1], line[0]);
  const length = length3(direction);
  if (length <= ALIGN_EPS) return distance3(point, line[0]);
  return length3(cross3(sub3(point, line[0]), direction)) / length;
}

function clippedWorldSegments(
  placement: FacePlacement,
  lineWorld: readonly [SpatialVec3, SpatialVec3],
): [SpatialVec3, SpatialVec3][] {
  const materialA = unmapPoint(placement, lineWorld[0]);
  const materialB = unmapPoint(placement, lineWorld[1]);
  if (materialA === null || materialB === null) return [];
  const remappedA = mapPoint(placement, materialA);
  const remappedB = mapPoint(placement, materialB);
  if (remappedA === null || remappedB === null) return [];
  if (
    distance3(vec3(remappedA), lineWorld[0]) > SPATIAL_REPROJECTION_EPS ||
    distance3(vec3(remappedB), lineWorld[1]) > SPATIAL_REPROJECTION_EPS
  ) {
    return [];
  }
  const materialLine: FoldLine = [materialA, materialB];
  const materialDirection = sub2(materialB, materialA);
  const out: [SpatialVec3, SpatialVec3][] = [];
  for (const [from, to] of linePolygonIntervals(materialLine, placement.polygon)) {
    const aMaterial: Vec2 = [
      materialA[0] + materialDirection[0] * from,
      materialA[1] + materialDirection[1] * from,
    ];
    const bMaterial: Vec2 = [
      materialA[0] + materialDirection[0] * to,
      materialA[1] + materialDirection[1] * to,
    ];
    const a = mapPoint(placement, aMaterial);
    const b = mapPoint(placement, bMaterial);
    if (a === null || b === null) continue;
    const segment: [SpatialVec3, SpatialVec3] = [vec3(a), vec3(b)];
    if (distance3(segment[0], segment[1]) <= ALIGN_EPS) continue;
    if (
      pointLineDistance3(segment[0], lineWorld) > SPATIAL_REPROJECTION_EPS ||
      pointLineDistance3(segment[1], lineWorld) > SPATIAL_REPROJECTION_EPS
    ) {
      continue;
    }
    out.push(segment);
  }
  return out;
}

function materialLineKey(line: MaterialFoldLine["material_line"]): [number, number, number] | null {
  const dx = line[1][0] - line[0][0];
  const dy = line[1][1] - line[0][1];
  const length = Math.hypot(dx, dy);
  if (length <= ALIGN_EPS) return null;
  let a = -dy / length;
  let b = dx / length;
  let c = -(a * line[0][0] + b * line[0][1]);
  if (a < -MATERIAL_LINE_EQ_EPS || (Math.abs(a) <= MATERIAL_LINE_EQ_EPS && b < 0)) {
    a = -a;
    b = -b;
    c = -c;
  }
  return [a, b, c];
}

function sameMaterialLine(
  a: MaterialFoldLine["material_line"],
  b: MaterialFoldLine["material_line"],
): boolean {
  const ka = materialLineKey(a);
  const kb = materialLineKey(b);
  return (
    ka !== null &&
    kb !== null &&
    ka.every((value, index) => Math.abs(value - kb[index]) <= MATERIAL_LINE_EQ_EPS)
  );
}

function sideWitness(
  lineWorld: [SpatialVec3, SpatialVec3],
  placement: FacePlacement,
  placements: readonly FacePlacement[],
  commonNormal: SpatialVec3,
  side: "left" | "right",
): { world: SpatialVec3; material: MaterialFoldLine } | null {
  const direction = unit3(sub3(lineWorld[1], lineWorld[0]));
  const normal = facePlaneNormal(placement);
  if (direction === null || normal === null) return null;
  let placementNormal = vec3(normal);
  if (dot3(placementNormal, commonNormal) < 0) placementNormal = mul3(placementNormal, -1);
  let left = unit3(cross3(placementNormal, direction));
  const commonLeft = unit3(cross3(commonNormal, direction));
  if (left === null || commonLeft === null) return null;
  if (dot3(left, commonLeft) < 0) left = mul3(left, -1);
  const sign = side === "left" ? 1 : -1;
  const lineLength = distance3(lineWorld[0], lineWorld[1]);
  const startingOffset = Math.max(KEEP_MIN_OFFSET * 2, lineLength * 0.25);
  for (const along of [0.5, 0.25, 0.75]) {
    const center = add3(
      lineWorld[0],
      mul3(sub3(lineWorld[1], lineWorld[0]), along),
    );
    for (let attempt = 0; attempt < 48; attempt++) {
      const offset = startingOffset / 2 ** attempt;
      if (offset <= KEEP_MIN_OFFSET) break;
      const candidate = add3(center, mul3(left, offset * sign));
      const material = materialFoldLineFrom3D(placements, lineWorld, candidate);
      if (material !== null) return { world: candidate, material };
    }
  }
  return null;
}

function spatialMaterialInput(
  material: MaterialFoldLine,
): SpatialMaterialFoldInput {
  return {
    materialLine: [
      [material.material_line[0][0], material.material_line[0][1]],
      [material.material_line[1][0], material.material_line[1][1]],
    ],
    materialKeepSidePoint: [
      material.material_keep_side_point[0],
      material.material_keep_side_point[1],
    ],
  };
}

function foldedPlaneCompanion(
  certified: boolean,
  lineWorld: [SpatialVec3, SpatialVec3],
  keepWorldForMovingSide: SpatialFoldTarget["keepWorldForMovingSide"],
): SpatialFoldTarget["foldedPlane"] {
  // raw Frame3Dでもz=0面だったという各入力の独立証拠がある場合だけ作る。
  // 表示層offsetや、端点が偶然z=0にある垂直面からは推測しない。
  if (!certified) return null;
  return {
    line: [
      [lineWorld[0][0], lineWorld[0][1]],
      [lineWorld[1][0], lineWorld[1][1]],
    ],
    keepPointForMovingSide: {
      left:
        keepWorldForMovingSide.left === null
          ? null
          : [keepWorldForMovingSide.left[0], keepWorldForMovingSide.left[1]],
      right:
        keepWorldForMovingSide.right === null
          ? null
          : [keepWorldForMovingSide.right[0], keepWorldForMovingSide.right[1]],
    },
  };
}

function foldedPointMatchesWorld(
  folded: Vec2,
  worldPoint: SpatialVec3,
): boolean {
  return Math.hypot(folded[0] - worldPoint[0], folded[1] - worldPoint[1]) <=
    SPATIAL_REPROJECTION_EPS;
}

function inputsCertifyFoldedPlane(
  plane: NormalizedPlane,
  picks: readonly SpatialAlignTarget[],
): boolean {
  if (distance3(plane.normal, GLOBAL_FOLDED_PLANE.normal) > SPATIAL_REPROJECTION_EPS) {
    return false;
  }
  return picks.every((pick) => {
    if (pick.kind === "point") {
      return (
        pick.foldedPoint !== null &&
        foldedPointMatchesWorld(pick.foldedPoint, pick.world)
      );
    }
    if (pick.foldedLine === null) return false;
    const direct =
      foldedPointMatchesWorld(pick.foldedLine[0], pick.aWorld) &&
      foldedPointMatchesWorld(pick.foldedLine[1], pick.bWorld);
    const reversed =
      foldedPointMatchesWorld(pick.foldedLine[1], pick.aWorld) &&
      foldedPointMatchesWorld(pick.foldedLine[0], pick.bWorld);
    return direct || reversed;
  });
}

function automaticSideForFirstPick(
  line: FoldLine,
  first: AlignTarget | null | undefined,
): "left" | "right" | null {
  if (!first) return null;
  const dx = line[1][0] - line[0][0];
  const dy = line[1][1] - line[0][1];
  const length = Math.hypot(dx, dy);
  if (length < ALIGN_EPS) return null;
  const sideOf = (point: Vec2): "left" | "right" | null => {
    const distance =
      (dx * (point[1] - line[0][1]) - dy * (point[0] - line[0][0])) / length;
    if (distance > ALIGN_EPS) return "left";
    if (distance < -ALIGN_EPS) return "right";
    return null;
  };
  if (first.kind === "point") return sideOf(first.p);
  const a = sideOf(first.a);
  const b = sideOf(first.b);
  if (a === null) return b;
  if (b === null) return a;
  return a === b ? a : null;
}

function sideForFirstPick(
  line: FoldLine,
  first: AlignTarget | null | undefined,
): SpatialFoldTarget["sideForFirstPick"] {
  const automatic = automaticSideForFirstPick(line, first);
  return {
    automatic,
    initial:
      automatic ?? (first ? movingSideOf(line, alignRefPoint(first)) : "right"),
  };
}

function lexicographicNumbers(a: readonly number[], b: readonly number[]): number {
  for (let index = 0; index < Math.min(a.length, b.length); index++) {
    if (a[index] !== b[index]) return a[index] - b[index];
  }
  return a.length - b.length;
}

function candidateSortKey(candidate: ClippedCandidate): number[] {
  const [a, b] = candidate.target.lineWorld;
  const endpoints = lexicographicNumbers(a, b) <= 0 ? [...a, ...b] : [...b, ...a];
  const sideCount = Object.values(candidate.target.keepWorldForMovingSide).filter(
    (point) => point !== null,
  ).length;
  return [-sideCount, ...endpoints];
}

function clipSolution(
  line: FoldLine,
  chart: PlaneChart,
  placements: readonly FacePlacement[],
  firstPick: AlignTarget | null | undefined,
  foldedPlaneCertified: boolean,
): ClippedSolution | null {
  const infiniteWorld: [SpatialVec3, SpatialVec3] = [
    chartToWorld(chart, line[0]),
    chartToWorld(chart, line[1]),
  ];
  if (distance3(infiniteWorld[0], infiniteWorld[1]) <= ALIGN_EPS) return null;
  const candidates: ClippedCandidate[] = [];
  for (const placement of placements) {
    const support = placementPlane(placement);
    if (support === null || !equivalentPlanes(chart.plane, support)) continue;
    for (const lineWorld of clippedWorldSegments(placement, infiniteWorld)) {
      const leftKeep = sideWitness(
        lineWorld,
        placement,
        placements,
        chart.plane.normal,
        "right",
      );
      const rightKeep = sideWitness(
        lineWorld,
        placement,
        placements,
        chart.plane.normal,
        "left",
      );
      const material = leftKeep?.material ?? rightKeep?.material ?? null;
      if (material === null) continue;
      if (
        leftKeep !== null &&
        rightKeep !== null &&
        !sameMaterialLine(leftKeep.material.material_line, rightKeep.material.material_line)
      ) {
        continue;
      }
      const keepWorldForMovingSide: SpatialFoldTarget["keepWorldForMovingSide"] = {
        left: leftKeep?.world ?? null,
        right: rightKeep?.world ?? null,
      };
      candidates.push({
        target: {
          lineWorld,
          keepWorldForMovingSide,
          foldedPlane: foldedPlaneCompanion(
            foldedPlaneCertified,
            lineWorld,
            keepWorldForMovingSide,
          ),
          sideForFirstPick: sideForFirstPick(line, firstPick),
        },
        materialLine: material.material_line,
        materialForMovingSide: {
          left: leftKeep === null ? null : spatialMaterialInput(leftKeep.material),
          right: rightKeep === null ? null : spatialMaterialInput(rightKeep.material),
        },
      });
    }
  }
  if (candidates.length === 0) return null;
  const materialGroups: ClippedCandidate[][] = [];
  for (const candidate of candidates) {
    const group = materialGroups.find((entries) =>
      sameMaterialLine(entries[0].materialLine, candidate.materialLine),
    );
    if (group) group.push(candidate);
    else materialGroups.push([candidate]);
  }
  if (materialGroups.length !== 1) return null;
  const selected = [...materialGroups[0]].sort((a, b) =>
    lexicographicNumbers(candidateSortKey(a), candidateSortKey(b)),
  )[0];
  return {
    target: selected.target,
    materialForMovingSide: selected.materialForMovingSide,
  };
}

function unavailable(
  reason: string,
  maximumResidual: number | null,
): SpatialAlignResult {
  return {
    status: "unavailable",
    solutions: [],
    materialSolutions: [],
    reason,
    maxReprojectionResidual: maximumResidual,
  };
}

/**
 * 全入力が一意な共通3D平面に載る場合だけ、その面の等長chartで既存solveAlignを1回解く。
 * 非共面・支持面非一意・再写像残差超過・材料面へ一意にclipできない場合は、
 * global XYやFace ID順へfallbackせずunavailableを返す。
 */
export function solveSpatialAlignOnCommonPlane(
  input: SolveSpatialAlignInput,
): SpatialAlignResult {
  const common = commonPlaneOf(input.picks);
  if (common.plane === null) {
    const reason =
      common.maximumResidual > SPATIAL_REPROJECTION_EPS
        ? "3D面への再写像残差が許容範囲を超えました"
        : "選んだ点・線に一意な共通3D平面がありません";
    return unavailable(reason, common.maximumResidual);
  }
  const chart = planeChart(common.plane);
  if (chart === null) return unavailable("共通3D平面のchartを作れません", null);

  let maximumResidual = common.maximumResidual;
  for (const target of input.picks) {
    const points = targetWorldPoints(target);
    if (points === null) return unavailable("3Dの点または線が縮退しています", null);
    for (const point of points) {
      const remapped = chartToWorld(chart, worldToChart(chart, point));
      maximumResidual = Math.max(maximumResidual, distance3(remapped, point));
    }
  }
  const cursorWorld = input.cursorWorld ?? null;
  let cursor: Vec2 | null = null;
  if (cursorWorld !== null) {
    if (!finite3(cursorWorld)) return unavailable("3Dカーソル座標が不正です", null);
    cursor = worldToChart(chart, vec3(cursorWorld));
    maximumResidual = Math.max(
      maximumResidual,
      distance3(chartToWorld(chart, cursor), vec3(cursorWorld)),
    );
  }
  if (maximumResidual > SPATIAL_REPROJECTION_EPS) {
    return unavailable(
      "3D面への再写像残差が許容範囲を超えました",
      maximumResidual,
    );
  }

  const picks = input.picks.map((target) => targetToChart(target, chart));
  const foldedPlaneCertified = inputsCertifyFoldedPlane(chart.plane, input.picks);
  // 8方式の幾何正本はこの1回だけ。Viewer側へ各方式の式を複製しない。
  const solved = solveAlign(input.mode, picks, cursor);
  if (solved.lines.length === 0) {
    return unavailable(
      solved.reason ?? "共通3D平面内に折り線の解がありません",
      maximumResidual,
    );
  }
  const clipped = solved.lines.map((line) =>
    clipSolution(
      line,
      chart,
      input.placements,
      picks[0],
      foldedPlaneCertified,
    ),
  );
  const solutions = clipped.map((solution) => solution?.target ?? null);
  const materialSolutions = clipped.map(
    (solution) => solution?.materialForMovingSide ?? null,
  );
  if (solutions.every((solution) => solution === null)) {
    return unavailable(
      "共通3D平面内で正の長さの折り線を材料面へ一意に戻せません",
      maximumResidual,
    );
  }
  return {
    status: "ready",
    solutions,
    materialSolutions,
    reason: solved.reason,
    maxReprojectionResidual: maximumResidual,
  };
}

function supportPlaneFromPlacement(
  placement: FacePlacement,
): SpatialSupportPlane | null {
  const normal = facePlaneNormal(placement);
  if (normal === null) return null;
  return { point: vec3(placement.q0), normal: vec3(normal) };
}

function worldPointMatchesPlacement(
  placement: FacePlacement,
  material: Vec2,
  worldPoint: SpatialVec3,
): boolean {
  if (!pointInPolygon(placement.polygon, material, SPATIAL_REPROJECTION_EPS)) {
    return false;
  }
  const mapped = mapPoint(placement, material);
  return mapped !== null && distance3(vec3(mapped), worldPoint) <= SPATIAL_REPROJECTION_EPS;
}

/** `CpPick3D.world`を失う前に、保存しないspatial pointへ変換する。 */
export function spatialPointTargetFromPick(
  pick: CpPick3D,
  placements: readonly FacePlacement[],
  foldedPoint: Vec2 | null = null,
): SpatialAlignTarget | null {
  if (!finite3(pick.world)) return null;
  const worldPoint = vec3(pick.world);
  const supportPlanes = placements
    .filter((placement) =>
      worldPointMatchesPlacement(placement, pick.cp, worldPoint),
    )
    .map(placementPlane)
    .filter((plane): plane is NormalizedPlane => plane !== null)
    .map((plane): SpatialSupportPlane => ({
      point: plane.point,
      normal: plane.normal,
    }));
  return supportPlanes.length === 0
    ? null
    : {
        kind: "point",
        world: worldPoint,
        supportPlanes,
        foldedPoint: foldedPoint === null ? null : [foldedPoint[0], foldedPoint[1]],
      };
}

function hingeMatchesPlacement(
  placement: FacePlacement,
  aWorld: SpatialVec3,
  bWorld: SpatialVec3,
): boolean {
  const a = unmapPoint(placement, aWorld);
  const b = unmapPoint(placement, bWorld);
  if (a === null || b === null) return false;
  if (
    !pointInPolygon(placement.polygon, a, SPATIAL_REPROJECTION_EPS) ||
    !pointInPolygon(placement.polygon, b, SPATIAL_REPROJECTION_EPS)
  ) {
    return false;
  }
  const remappedA = mapPoint(placement, a);
  const remappedB = mapPoint(placement, b);
  return (
    remappedA !== null &&
    remappedB !== null &&
    distance3(vec3(remappedA), aWorld) <= SPATIAL_REPROJECTION_EPS &&
    distance3(vec3(remappedB), bWorld) <= SPATIAL_REPROJECTION_EPS
  );
}

/** `HingeSegment.a/b`を失う前に、保存しないspatial lineへ変換する。 */
export function spatialLineTargetFromHinge(
  hinge: HingeSegment,
  placements: readonly FacePlacement[],
  foldedLine: [Vec2, Vec2] | null = null,
): SpatialAlignTarget | null {
  const aWorld: SpatialVec3 = [hinge.a.x, hinge.a.y, hinge.a.z];
  const bWorld: SpatialVec3 = [hinge.b.x, hinge.b.y, hinge.b.z];
  if (!finite3(aWorld) || !finite3(bWorld) || distance3(aWorld, bWorld) <= ALIGN_EPS) {
    return null;
  }
  const candidates =
    hinge.ownerFace === undefined
      ? placements
      : placements.filter((placement) => placement.faceId === hinge.ownerFace);
  const supportPlanes = candidates
    .filter((placement) => hingeMatchesPlacement(placement, aWorld, bWorld))
    .map(placementPlane)
    .filter((plane): plane is NormalizedPlane => plane !== null)
    .map((plane): SpatialSupportPlane => ({
      point: plane.point,
      normal: plane.normal,
    }));
  return supportPlanes.length === 0
    ? null
    : {
        kind: "line",
        aWorld,
        bWorld,
        supportPlanes,
        foldedLine:
          foldedLine === null
            ? null
            : [
                [foldedLine[0][0], foldedLine[0][1]],
                [foldedLine[1][0], foldedLine[1][1]],
              ],
      };
}

/** FacePlacementから保存しない支持面だけを得る。Viewer外へFace IDは出さない。 */
export function spatialSupportPlaneOf(
  placement: FacePlacement,
): SpatialSupportPlane | null {
  return supportPlaneFromPlacement(placement);
}
