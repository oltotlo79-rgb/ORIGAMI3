// 対称に線をそろえるための純粋な2D計算(CPE-010)。
// JavaScriptのnumber(IEEE 754倍精度=f64相当)と、正規化座標で明示した許容誤差を使う。
// 基準線は「通る点pと向きd」で表し、紙の縦・横中心線と、作品内の選んだ線を
// 同じ計算へ渡せるようにする。画面・IPC・保存形式には依存しない。

import type { Document, Paper, Vec2 } from "./types";

/** 同じ場所とみなす距離(正規化座標。紙の長辺=1.0) */
export const MIRROR_EPS = 1e-6;

/** 線分(始点・終点) */
export type Segment = [Vec2, Vec2];

/** 基準線(点pを通り、ベクトルdの向きへ延びる直線)。dは単位長でなくてよい。 */
export interface MirrorLine {
  p: Vec2;
  d: Vec2;
}

/** 端末へ保存できる紙の基準線。作品内の線はここへ入れない。 */
export type MirrorAxisPreset = "paperVertical" | "paperHorizontal";

/** 現在使っている基準線。選んだ線は作品内の辺IDだけを一時的に覚える。 */
export type MirrorAxisChoice =
  | { kind: "paperVertical" }
  | { kind: "paperHorizontal" }
  | { kind: "selectedLine"; edgeId: number };

export const DEFAULT_MIRROR_AXIS: MirrorAxisChoice = { kind: "paperVertical" };

/** 公開APIへ渡された許容誤差を、有限な0以上の値へそろえる。 */
function toleranceOf(eps: number): number {
  return Number.isFinite(eps) && eps >= 0 ? eps : MIRROR_EPS;
}

/** 紙の正規化された横・縦の長さ(長辺=1.0)。 */
export function normalizedPaperSize(paper: Paper): Vec2 {
  if (
    !Number.isFinite(paper.width_mm) ||
    !Number.isFinite(paper.height_mm) ||
    paper.width_mm < 0 ||
    paper.height_mm < 0
  ) {
    return [0, 0];
  }
  const long = Math.max(paper.width_mm, paper.height_mm);
  return long > 0
    ? [paper.width_mm / long, paper.height_mm / long]
    : [0, 0];
}

/** 紙の縦の中心線のx座標。旧来の呼び出しと既定動作の確認用に残す。 */
export function mirrorAxisX(paper: Paper): number {
  return normalizedPaperSize(paper)[0] / 2;
}

/** 紙の縦または横の中心線。 */
export function paperMirrorLine(
  paper: Paper,
  preset: MirrorAxisPreset,
): MirrorLine {
  const [width, height] = normalizedPaperSize(paper);
  return preset === "paperHorizontal"
    ? { p: [width / 2, height / 2], d: [1, 0] }
    : { p: [width / 2, height / 2], d: [0, 1] };
}

/** 2点を通る基準線。2点が同じ場所なら基準線を作れない。 */
export function mirrorLineThrough(
  a: Vec2,
  b: Vec2,
  eps = MIRROR_EPS,
): MirrorLine | null {
  const tolerance = toleranceOf(eps);
  const d: Vec2 = [b[0] - a[0], b[1] - a[1]];
  const length = Math.hypot(d[0], d[1]);
  if (
    !Number.isFinite(a[0]) ||
    !Number.isFinite(a[1]) ||
    !Number.isFinite(b[0]) ||
    !Number.isFinite(b[1]) ||
    !Number.isFinite(length) ||
    length <= tolerance
  ) {
    return null;
  }
  return { p: [a[0], a[1]], d };
}

/** 辺IDから端点を得る。参照切れや非有限座標の辺は線分として扱わない。 */
function edgeSegment(doc: Document, edgeId: number): Segment | null {
  const edge = doc.cp.edges.find((candidate) => candidate.id === edgeId);
  if (!edge) return null;
  const a = doc.cp.vertices.find((vertex) => vertex.id === edge.v0)?.pos;
  const b = doc.cp.vertices.find((vertex) => vertex.id === edge.v1)?.pos;
  if (
    !a ||
    !b ||
    !Number.isFinite(a[0]) ||
    !Number.isFinite(a[1]) ||
    !Number.isFinite(b[0]) ||
    !Number.isFinite(b[1])
  ) {
    return null;
  }
  return [a, b];
}

/** 選んだ辺を通る基準線。存在しない・潰れた・輪郭の辺なら選べない。 */
export function selectedEdgeMirrorLine(
  doc: Document,
  edgeId: number,
  eps = MIRROR_EPS,
): MirrorLine | null {
  const edge = doc.cp.edges.find((candidate) => candidate.id === edgeId);
  if (!edge || edge.kind === "Border") return null;
  const segment = edgeSegment(doc, edgeId);
  return segment ? mirrorLineThrough(segment[0], segment[1], eps) : null;
}

/** 現在の選択から実際の基準線を求める。無効な「選んだ線」ならnull。 */
export function mirrorLineForChoice(
  doc: Document,
  choice: MirrorAxisChoice,
  eps = MIRROR_EPS,
): MirrorLine | null {
  if (choice.kind === "selectedLine") {
    return selectedEdgeMirrorLine(doc, choice.edgeId, eps);
  }
  return paperMirrorLine(doc.paper, choice.kind);
}

/** 画面へ出す基準線の短い名前。 */
export function mirrorAxisLabel(choice: MirrorAxisChoice): string {
  if (choice.kind === "paperHorizontal") return "紙の横の中心線";
  if (choice.kind === "selectedLine") return "選んだ線";
  return "紙の縦の中心線";
}

/**
 * 基準線の向きを単位ベクトルにする。
 * MirrorLineのdは倍率自由なので、位置の許容誤差で長さを判定してはいけない。
 */
function unitDirection(line: MirrorLine): Vec2 | null {
  const length = Math.hypot(line.d[0], line.d[1]);
  if (
    !Number.isFinite(line.p[0]) ||
    !Number.isFinite(line.p[1]) ||
    !Number.isFinite(line.d[0]) ||
    !Number.isFinite(line.d[1]) ||
    !Number.isFinite(length) ||
    length === 0
  ) {
    return null;
  }
  return [line.d[0] / length, line.d[1] / length];
}

/** 点と向きが有限で、向きが0でない基準線か。 */
export function isValidMirrorLine(line: MirrorLine): boolean {
  return unitDirection(line) !== null;
}

/**
 * 編集後も「選んだ線」を指し続けられる辺IDへ結び直す。
 *
 * 交差する線を追加すると、基準辺が複数の辺へ分割されて元のIDが消えることがある。
 * その場合でも、旧線分の全長が同一直線上の新しい非輪郭辺で隙間なく覆われていれば
 * 線は残っているとみなし、旧始点を含む片へIDを付け替える。明示削除や一部欠落なら
 * nullを返し、呼び出し側が紙の縦中心線へ戻して知らせられるようにする。
 */
export function rebindSelectedMirrorAxis(
  previousDoc: Document,
  nextDoc: Document,
  choice: MirrorAxisChoice,
  eps = MIRROR_EPS,
): MirrorAxisChoice | null {
  if (choice.kind !== "selectedLine") return choice;
  if (selectedEdgeMirrorLine(nextDoc, choice.edgeId, eps) !== null) {
    return choice;
  }

  const tolerance = toleranceOf(eps);
  const previousSegment = edgeSegment(previousDoc, choice.edgeId);
  const previousAxis = selectedEdgeMirrorLine(
    previousDoc,
    choice.edgeId,
    tolerance,
  );
  const direction = previousAxis ? unitDirection(previousAxis) : null;
  if (!previousSegment || !previousAxis || !direction) return null;
  const length = Math.hypot(
    previousSegment[1][0] - previousSegment[0][0],
    previousSegment[1][1] - previousSegment[0][1],
  );

  const intervals: { id: number; start: number; end: number }[] = [];
  for (const edge of nextDoc.cp.edges) {
    if (edge.kind === "Border") continue;
    const segment = edgeSegment(nextDoc, edge.id);
    if (
      !segment ||
      mirrorLineThrough(segment[0], segment[1], tolerance) === null ||
      !isOnMirrorAxis(segment, previousAxis, tolerance)
    ) {
      continue;
    }
    const projected = segment.map((point) =>
      (point[0] - previousSegment[0][0]) * direction[0] +
      (point[1] - previousSegment[0][1]) * direction[1]
    );
    const start = Math.max(0, Math.min(length, Math.min(...projected)));
    const end = Math.max(0, Math.min(length, Math.max(...projected)));
    if (end - start > tolerance) intervals.push({ id: edge.id, start, end });
  }
  intervals.sort(
    (a, b) => a.start - b.start || b.end - a.end || a.id - b.id,
  );

  let coveredUntil = 0;
  let reboundId: number | null = null;
  for (const interval of intervals) {
    if (interval.end <= coveredUntil + tolerance) continue;
    if (interval.start > coveredUntil + tolerance) return null;
    reboundId ??= interval.id;
    coveredUntil = interval.end;
    if (coveredUntil >= length - tolerance) {
      return { kind: "selectedLine", edgeId: reboundId };
    }
  }
  return null;
}

/**
 * 任意の直線をはさんで反対側へ移した点。
 * 点から直線へ下ろした垂線の足をfootとし、2*foot-pointで求める。
 * 基準線からeps以内の点は、丸め誤差でも動かさないよう元の座標をそのまま返す。
 */
export function mirrorPoint(
  point: Vec2,
  line: MirrorLine,
  eps = MIRROR_EPS,
): Vec2 {
  const tolerance = toleranceOf(eps);
  const direction = unitDirection(line);
  if (direction === null || !Number.isFinite(point[0]) || !Number.isFinite(point[1])) {
    return [point[0], point[1]];
  }
  const fromLine: Vec2 = [point[0] - line.p[0], point[1] - line.p[1]];
  const t = fromLine[0] * direction[0] + fromLine[1] * direction[1];
  const foot: Vec2 = [
    line.p[0] + t * direction[0],
    line.p[1] + t * direction[1],
  ];
  if (!Number.isFinite(foot[0]) || !Number.isFinite(foot[1])) {
    return [point[0], point[1]];
  }
  if (
    Math.hypot(point[0] - foot[0], point[1] - foot[1]) <=
    tolerance
  ) {
    return [point[0], point[1]];
  }
  const reflected: Vec2 = [
    2 * foot[0] - point[0],
    2 * foot[1] - point[1],
  ];
  return Number.isFinite(reflected[0]) && Number.isFinite(reflected[1])
    ? reflected
    : [point[0], point[1]];
}

/** 任意の基準線をはさんで反対側へ移した線分。 */
export function mirrorSegment(
  segment: Segment,
  line: MirrorLine,
  eps = MIRROR_EPS,
): Segment {
  return [
    mirrorPoint(segment[0], line, eps),
    mirrorPoint(segment[1], line, eps),
  ];
}

/** その線分の両端が基準線の上に乗っているか。 */
export function isOnMirrorAxis(
  segment: Segment,
  line: MirrorLine,
  eps = MIRROR_EPS,
): boolean {
  const tolerance = toleranceOf(eps);
  const direction = unitDirection(line);
  if (direction === null) return false;
  return segment.every((point) => {
    if (!Number.isFinite(point[0]) || !Number.isFinite(point[1])) return false;
    const x = point[0] - line.p[0];
    const y = point[1] - line.p[1];
    return Math.abs(x * direction[1] - y * direction[0]) <= tolerance;
  });
}

function samePoint(a: Vec2, b: Vec2, eps: number): boolean {
  return Math.abs(a[0] - b[0]) <= eps && Math.abs(a[1] - b[1]) <= eps;
}

/** 同じ線分か(向きが逆でも同じとみなす)。 */
export function isSameSegment(x: Segment, y: Segment, eps = MIRROR_EPS): boolean {
  const tolerance = toleranceOf(eps);
  return (
    (samePoint(x[0], y[0], tolerance) &&
      samePoint(x[1], y[1], tolerance)) ||
    (samePoint(x[0], y[1], tolerance) &&
      samePoint(x[1], y[0], tolerance))
  );
}

/**
 * 実際に引く線分を返す。通常は元と反対側の2本、基準線上の線・もともと
 * 対称な線・無効な基準線は、同じ線を二重に引かず1本だけにする。
 */
export function mirrorSegments(
  segment: Segment,
  line: MirrorLine,
  eps = MIRROR_EPS,
): Segment[] {
  if (unitDirection(line) === null) return [segment];
  const other = mirrorSegment(segment, line, eps);
  if (isOnMirrorAxis(segment, line, eps) || isSameSegment(segment, other, eps)) {
    return [segment];
  }
  return [segment, other];
}

/** 線分に、同じ位置でも別物として扱う区分(線種など)を添えたもの。 */
export interface KeyedSegment<K = string> {
  segment: Segment;
  key: K;
}

/**
 * 1回の入力で引く線をまとめて反対側へ広げる(不具合D13)。
 *
 * 1本ずつ `mirrorSegments` を呼ぶと、別々の線どうしが互いの鏡像になっている集合
 * (基準線をはさんで並ぶ等分の目印や、1点を通る角度線の一式)で、同じ線を二重に
 * 引いてしまう。ここでは入力の線をそのまま残したうえで、鏡像は集合にまだ無い
 * ものだけを足す。基準線の上に乗る線には鏡像を作らない。
 *
 * 入力がすでに反対側の線を含んでいても結果は変わらない(何回掛けても同じ)ので、
 * 引く前に自分で反対側を作る経路と重ねて呼んでも線は増えない。
 *
 * keyが違う線は、位置が同じでも別の線として扱う(線種の違う線を消さないため)。
 */
export function mirrorSegmentSet<K>(
  items: readonly KeyedSegment<K>[],
  line: MirrorLine,
  eps = MIRROR_EPS,
): KeyedSegment<K>[] {
  const result: KeyedSegment<K>[] = items.map((item) => ({
    key: item.key,
    segment: item.segment,
  }));
  if (unitDirection(line) === null) return result;
  for (const item of items) {
    if (isOnMirrorAxis(item.segment, line, eps)) continue;
    const reflected = mirrorSegment(item.segment, line, eps);
    const duplicate = result.some(
      (other) =>
        other.key === item.key && isSameSegment(other.segment, reflected, eps),
    );
    if (!duplicate) result.push({ key: item.key, segment: reflected });
  }
  return result;
}

/** 紙の範囲内に見える基準線の両端。Canvasのガイド描画で縦・横・斜めを共用する。 */
export function mirrorLineInsidePaper(
  paper: Paper,
  line: MirrorLine,
  eps = MIRROR_EPS,
): Segment | null {
  const tolerance = toleranceOf(eps);
  const direction = unitDirection(line);
  if (direction === null) return null;
  const [width, height] = normalizedPaperSize(paper);
  const candidates: { t: number; point: Vec2 }[] = [];
  const add = (t: number, point: Vec2) => {
    if (
      !Number.isFinite(t) ||
      !Number.isFinite(point[0]) ||
      !Number.isFinite(point[1]) ||
      point[0] < -tolerance ||
      point[0] > width + tolerance ||
      point[1] < -tolerance ||
      point[1] > height + tolerance ||
      candidates.some(
        (candidate) =>
          Math.abs(candidate.point[0] - point[0]) <= tolerance &&
          Math.abs(candidate.point[1] - point[1]) <= tolerance,
      )
    ) {
      return;
    }
    candidates.push({
      t,
      point: [
        Math.max(0, Math.min(width, point[0])),
        Math.max(0, Math.min(height, point[1])),
      ],
    });
  };
  if (direction[0] !== 0) {
    for (const x of [0, width]) {
      const t = (x - line.p[0]) / direction[0];
      add(t, [x, line.p[1] + t * direction[1]]);
    }
  }
  if (direction[1] !== 0) {
    for (const y of [0, height]) {
      const t = (y - line.p[1]) / direction[1];
      add(t, [line.p[0] + t * direction[0], y]);
    }
  }
  if (candidates.length < 2) return null;
  candidates.sort((a, b) => a.t - b.t);
  return [candidates[0].point, candidates[candidates.length - 1].point];
}
