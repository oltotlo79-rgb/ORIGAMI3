// 通常の直線描画で使う方向吸着の純関数。
// 始点につながる既存線の延長方向と、線どうしが作る角の二等分方向を候補にする。

import type { Document, Vec2 } from "./types";

/** カーソル方向が候補からこの角度以内なら吸着する。 */
export const DIRECTION_SNAP_ANGLE_DEG = 5;

/** 始点が既存線上にあるかを調べる正規化座標上の許容誤差。 */
const CONNECT_TOLERANCE = 1e-7;
const EPS = 1e-9;
const SAME_DIRECTION_DOT = 1 - 1e-10;

export type DirectionSnapKind = "extension" | "bisector";

export interface DirectionCandidate {
  /** 始点から見た単位方向ベクトル。反対側は別候補として列挙する。 */
  direction: Vec2;
  kind: DirectionSnapKind;
}

export interface DirectionSnapResult extends DirectionCandidate {
  /** 向きだけを候補へ合わせ、カーソルまでの距離を保った終点。 */
  pos: Vec2;
}

function sub(a: Vec2, b: Vec2): Vec2 {
  return [a[0] - b[0], a[1] - b[1]];
}

function length(v: Vec2): number {
  return Math.hypot(v[0], v[1]);
}

function normalize(v: Vec2): Vec2 | null {
  const n = length(v);
  if (n <= EPS) return null;
  const x = v[0] / n;
  const y = v[1] / n;
  return [Math.abs(x) <= EPS ? 0 : x, Math.abs(y) <= EPS ? 0 : y];
}

function dot(a: Vec2, b: Vec2): number {
  return a[0] * b[0] + a[1] * b[1];
}

function cross(a: Vec2, b: Vec2): number {
  return a[0] * b[1] - a[1] * b[0];
}

function near(a: Vec2, b: Vec2, tolerance = CONNECT_TOLERANCE): boolean {
  return length(sub(a, b)) <= tolerance;
}

interface LineSegmentParameters {
  unit: Vec2;
  lineDistance: number;
  segmentRatio: number;
}

interface LineIntersectionParameters {
  firstUnit: Vec2;
  firstLength: number;
  firstDistance: number;
  secondLength: number;
  secondDistance: number;
}

/**
 * start + unit * lineDistance と a + (b-a) * segmentRatio が交わる値を返す。
 * 範囲の制限は呼び出し側で行い、平行・同一直線・長さ0は一意な交点なしとする。
 */
function lineSegmentParameters(
  start: Vec2,
  direction: Vec2,
  a: Vec2,
  b: Vec2,
): LineSegmentParameters | null {
  const unit = normalize(direction);
  if (!unit) return null;
  const segment = sub(b, a);
  const denominator = cross(unit, segment);
  if (Math.abs(denominator) <= EPS) return null;

  const fromStart = sub(a, start);
  return {
    unit,
    lineDistance: cross(fromStart, segment) / denominator,
    segmentRatio: cross(fromStart, unit) / denominator,
  };
}

/**
 * 2直線の交点を、両方の方向を単位化して対称に求める。
 * 通常描画用の半直線計算とは分け、線分長や引数順で平行判定が変わらないようにする。
 */
function lineIntersectionParameters(
  a: Vec2,
  b: Vec2,
  c: Vec2,
  d: Vec2,
): LineIntersectionParameters | null {
  const first = sub(b, a);
  const second = sub(d, c);
  const firstLength = length(first);
  const secondLength = length(second);
  if (firstLength <= EPS || secondLength <= EPS) return null;

  const firstUnit: Vec2 = [first[0] / firstLength, first[1] / firstLength];
  const secondUnit: Vec2 = [second[0] / secondLength, second[1] / secondLength];
  const denominator = cross(firstUnit, secondUnit);
  if (Math.abs(denominator) <= EPS) return null;

  const offset = sub(c, a);
  const firstDistance = cross(offset, secondUnit) / denominator;
  const secondDistance = cross(offset, firstUnit) / denominator;
  return {
    firstUnit,
    firstLength,
    firstDistance,
    secondLength,
    secondDistance,
  };
}

function pointOnUnitLine(start: Vec2, unit: Vec2, distance: number): Vec2 {
  return [start[0] + unit[0] * distance, start[1] + unit[1] * distance];
}

/**
 * 点を始点から direction へ伸びる半直線へ垂直投影する。
 * 投影先が始点より後ろにある場合と、方向が長さ0の場合は null を返す。
 */
export function projectPointToDirectionRay(
  start: Vec2,
  direction: Vec2,
  point: Vec2,
): Vec2 | null {
  const unit = normalize(direction);
  if (!unit) return null;
  const along = dot(sub(point, start), unit);
  if (along < -EPS) return null;
  const distance = Math.max(0, along);
  return [start[0] + unit[0] * distance, start[1] + unit[1] * distance];
}

/**
 * 始点から direction へ伸びる半直線と線分 ab の交点を返す。
 * 平行・同一直線、半直線の後方、線分の範囲外では交点なしとする。
 */
export function intersectDirectionRayWithSegment(
  start: Vec2,
  direction: Vec2,
  a: Vec2,
  b: Vec2,
): Vec2 | null {
  const intersection = lineSegmentParameters(start, direction, a, b);
  if (!intersection) return null;
  const { unit, lineDistance: rayDistance, segmentRatio } = intersection;
  if (
    rayDistance < -EPS ||
    segmentRatio < -EPS ||
    segmentRatio > 1 + EPS
  ) {
    return null;
  }

  const distance = Math.max(0, rayDistance);
  return pointOnUnitLine(start, unit, distance);
}

/**
 * 2本の線分ab・cdの交点を返す。
 * 線分の外側、平行・同一直線、長さ0では交点なしとする。
 */
export function intersectSegments(a: Vec2, b: Vec2, c: Vec2, d: Vec2): Vec2 | null {
  const intersection = lineIntersectionParameters(a, b, c, d);
  if (!intersection) return null;
  if (
    intersection.firstDistance < -EPS ||
    intersection.firstDistance > intersection.firstLength + EPS ||
    intersection.secondDistance < -EPS ||
    intersection.secondDistance > intersection.secondLength + EPS
  ) {
    return null;
  }
  return pointOnUnitLine(a, intersection.firstUnit, intersection.firstDistance);
}

/**
 * 線分ab・cdを両方向へ延ばした2直線の交点を返す。
 * 平行・同一直線、長さ0では一意な交点がないためnullとする。
 */
export function intersectInfiniteLines(
  a: Vec2,
  b: Vec2,
  c: Vec2,
  d: Vec2,
): Vec2 | null {
  const intersection = lineIntersectionParameters(a, b, c, d);
  return intersection
    ? pointOnUnitLine(a, intersection.firstUnit, intersection.firstDistance)
    : null;
}

/** startが線分ab上にあるか（端点を含む）。 */
function liesOnSegment(start: Vec2, a: Vec2, b: Vec2): boolean {
  const ab = sub(b, a);
  const abLength = length(ab);
  if (abLength <= EPS) return false;
  const ap = sub(start, a);
  const crossDistance = Math.abs(ab[0] * ap[1] - ab[1] * ap[0]) / abLength;
  if (crossDistance > CONNECT_TOLERANCE) return false;
  const t = dot(ap, ab) / (abLength * abLength);
  const tTolerance = CONNECT_TOLERANCE / abLength;
  return t >= -tTolerance && t <= 1 + tTolerance;
}

function addUniqueDirection(directions: Vec2[], direction: Vec2): void {
  const unit = normalize(direction);
  if (!unit) return;
  if (directions.some((existing) => dot(existing, unit) >= SAME_DIRECTION_DOT)) return;
  directions.push(unit);
}

function addUniqueCandidate(
  candidates: DirectionCandidate[],
  direction: Vec2,
  kind: DirectionSnapKind,
): void {
  const unit = normalize(direction);
  if (!unit) return;
  // 同じ向きが延長と二等分の両方に現れるときは、先に追加する延長として扱う。
  if (candidates.some((candidate) => dot(candidate.direction, unit) >= SAME_DIRECTION_DOT)) {
    return;
  }
  candidates.push({ direction: unit, kind });
}

/**
 * 始点で使える方向候補を列挙する。
 *
 * - 始点を通る各既存線から、線の延長を両方向へ列挙する。
 * - 始点を端点に持つ既存線の各ペアから、角の二等分を両方向へ列挙する。
 * - 180°の角は、その直線に垂直な方向が二等分方向になる。
 * - 壊れた参照、長さ0の線、重複方向は無視する。
 */
export function directionCandidatesAt(doc: Document, start: Vec2): DirectionCandidate[] {
  const byId = new Map(doc.cp.vertices.map((vertex) => [vertex.id, vertex.pos]));
  const extensionDirections: Vec2[] = [];
  const connectedRays: Vec2[] = [];

  for (const edge of doc.cp.edges) {
    const a = byId.get(edge.v0);
    const b = byId.get(edge.v1);
    if (!a || !b) continue;
    const axis = normalize(sub(b, a));
    if (!axis) continue;

    // 線の途中を始点にした場合にも、その線の延長方向は利用できる。
    if (liesOnSegment(start, a, b)) {
      addUniqueDirection(extensionDirections, axis);
      addUniqueDirection(extensionDirections, [-axis[0], -axis[1]]);
    }

    // 二等分は「始点に集まっている線」だけを対象にする。線の途中に置いた
    // 1点から同じ線の両端を2本と数えて、不要な垂線を作らないため。
    if (near(start, a)) addUniqueDirection(connectedRays, sub(b, a));
    if (near(start, b)) addUniqueDirection(connectedRays, sub(a, b));
  }

  const candidates: DirectionCandidate[] = [];
  for (const direction of extensionDirections) {
    addUniqueCandidate(candidates, direction, "extension");
  }

  for (let i = 0; i < connectedRays.length; i += 1) {
    for (let j = i + 1; j < connectedRays.length; j += 1) {
      const a = connectedRays[i];
      const b = connectedRays[j];
      const sum: Vec2 = [a[0] + b[0], a[1] + b[1]];
      // 反対向きの2本は180°の角を作るので、垂直方向が二等分になる。
      const bisector: Vec2 = length(sum) > EPS ? sum : [-a[1], a[0]];
      addUniqueCandidate(candidates, bisector, "bisector");
      addUniqueCandidate(candidates, [-bisector[0], -bisector[1]], "bisector");
    }
  }

  return candidates;
}

/**
 * カーソル方向に最も近い候補へ向きだけを吸着させる。
 * 長さは始点から元のカーソルまでの距離を保つので、終点は自由に決められる。
 */
export function snapToDirection(
  start: Vec2,
  cursor: Vec2,
  candidates: DirectionCandidate[],
  maxAngleDeg = DIRECTION_SNAP_ANGLE_DEG,
): DirectionSnapResult | null {
  if (maxAngleDeg < 0) return null;
  const delta = sub(cursor, start);
  const distance = length(delta);
  const cursorDirection = normalize(delta);
  if (!cursorDirection || candidates.length === 0) return null;

  let best: DirectionCandidate | null = null;
  let bestDot = -Infinity;
  for (const candidate of candidates) {
    const similarity = dot(cursorDirection, candidate.direction);
    if (similarity > bestDot) {
      best = candidate;
      bestDot = similarity;
    }
  }

  const threshold = Math.cos((Math.min(maxAngleDeg, 180) * Math.PI) / 180);
  if (!best || bestDot + EPS < threshold) return null;
  return {
    ...best,
    pos: [
      start[0] + best.direction[0] * distance,
      start[1] + best.direction[1] * distance,
    ],
  };
}

/** ドキュメントから候補を作り、カーソル方向への吸着までまとめて行う。 */
export function snapLineDirection(
  doc: Document,
  start: Vec2,
  cursor: Vec2,
  maxAngleDeg = DIRECTION_SNAP_ANGLE_DEG,
): DirectionSnapResult | null {
  return snapToDirection(start, cursor, directionCandidatesAt(doc, start), maxAngleDeg);
}
