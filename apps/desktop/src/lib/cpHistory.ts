import type { Document, DriverLine, Edge, StepCreases, Vec2 } from "./types";

/** CP座標で、点が手順の線分上にあるとみなす距離。 */
export const CP_HISTORY_EPSILON = 1e-8;

/**
 * Documentはストアで更新のたびに参照が入れ替わるため、Document参照を世代として
 * 手順ごとの表示用Documentを覚える。古い作品はWeakMapから自然に解放される。
 */
interface CpHistoryCache {
  /** この結果を作ったときの来歴。別の来歴が来たら作り直す。 */
  creases: readonly StepCreases[] | null;
  /** 最終辺ID -> 初めて追加された1始まり手順。nullは事前折り線。 */
  introducedByEdge: ReadonlyMap<number, number | null>;
  documents: Map<number, Document>;
}

const historyCache = new WeakMap<Document, CpHistoryCache>();

function finitePoint(point: Vec2): boolean {
  return Number.isFinite(point[0]) && Number.isFinite(point[1]);
}

/** 端点の外側も距離EPS以内なら同じ有限線分上として扱う。 */
function pointOnSegment(point: Vec2, a: Vec2, b: Vec2): boolean {
  if (!finitePoint(point) || !finitePoint(a) || !finitePoint(b)) return false;

  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const lengthSquared = dx * dx + dy * dy;
  if (lengthSquared <= CP_HISTORY_EPSILON * CP_HISTORY_EPSILON) return false;

  const px = point[0] - a[0];
  const py = point[1] - a[1];
  const projection = (px * dx + py * dy) / lengthSquared;
  const along = Math.max(0, Math.min(1, projection));
  const nearestX = a[0] + along * dx;
  const nearestY = a[1] + along * dy;
  return Math.hypot(point[0] - nearestX, point[1] - nearestY) <= CP_HISTORY_EPSILON;
}

function usableDriver(driver: DriverLine): boolean {
  if (!Number.isFinite(driver.target_angle_deg) || driver.target_angle_deg === 0) {
    return false;
  }
  if (!finitePoint(driver.a) || !finitePoint(driver.b)) return false;
  return (
    Math.hypot(driver.b[0] - driver.a[0], driver.b[1] - driver.a[1]) >
    CP_HISTORY_EPSILON
  );
}

/** 手順IDごとの「その手順が新しく足した折り線」。 */
function recordedLines(
  creases: readonly StepCreases[] | null,
): ReadonlyMap<number, readonly [Vec2, Vec2][]> {
  const map = new Map<number, readonly [Vec2, Vec2][]>();
  for (const entry of creases ?? []) {
    if (!Array.isArray(entry.lines)) continue;
    map.set(entry.step, entry.lines);
  }
  return map;
}

/**
 * 最終CPの辺が現れる手順を求める。
 *
 * 来歴(step_creases)がある手順では、そこに記録された線だけがその手順で足された線
 * になる。記録が空の手順は「線を1本も足していない」ことの証拠なので、先に描いて
 * あった折り線をその手順のものと取り違えない。
 *
 * 来歴を持たない旧形式の作品だけ、これまでどおりdriver線分との一致から推測する。
 * 見つからない辺や壊れた辺は事前折り線として扱い、隠さない。
 */
function introducedAt(
  doc: Document,
  edge: Edge,
  vertices: ReadonlyMap<number, Vec2>,
  recorded: ReadonlyMap<number, readonly [Vec2, Vec2][]>,
): number | null {
  if (edge.kind !== "Mountain" && edge.kind !== "Valley") return null;
  const a = vertices.get(edge.v0);
  const b = vertices.get(edge.v1);
  if (!a || !b) return null;

  for (let index = 0; index < doc.sequence.length; index += 1) {
    const step = doc.sequence[index];
    const lines = recorded.get(step.id);
    if (lines) {
      const added = lines.some(
        (line) => pointOnSegment(a, line[0], line[1]) && pointOnSegment(b, line[0], line[1]),
      );
      if (added) return index + 1;
      continue;
    }
    if (step.kind === "Pose") continue;
    // 既にある折り筋で折った手順は、その線を足していない(旧形式で分かる唯一の証拠)
    if (step.alignment?.mode === "existingLine") continue;
    for (const driver of step.drivers) {
      if (
        usableDriver(driver) &&
        pointOnSegment(a, driver.a, driver.b) &&
        pointOnSegment(b, driver.a, driver.b)
      ) {
        return index + 1;
      }
    }
  }
  return null;
}

function normalizeStep(doc: Document, currentStep: number): number {
  const total = doc.sequence.length;
  if (Number.isNaN(currentStep)) return 0;
  return Math.max(0, Math.min(total, Math.trunc(currentStep)));
}

/** 辺と来歴の照合は作品ごとに1回だけ行い、コマ送り中は再利用する。 */
function historyFor(
  doc: Document,
  creases: readonly StepCreases[] | null,
): CpHistoryCache {
  const cached = historyCache.get(doc);
  if (cached && cached.creases === creases) return cached;

  const vertices = new Map(doc.cp.vertices.map((vertex) => [vertex.id, vertex.pos]));
  const recorded = recordedLines(creases);
  const introducedByEdge = new Map(
    doc.cp.edges.map((edge) => [edge.id, introducedAt(doc, edge, vertices, recorded)]),
  );
  const history = {
    creases,
    introducedByEdge,
    documents: new Map<number, Document>(),
  };
  historyCache.set(doc, history);
  return history;
}

/**
 * タイムライン位置で既に付いている折り線と点だけを持つ2D表示用Documentを返す。
 * 元Documentは変更せず、最新表示では余計な複製を作らず元参照を返す。
 *
 * `stepCreases` は手順ごとに新しく足した折り線の来歴(DocumentView由来)。
 * 省略した旧形式の作品では、これまでどおり折り手順から推測する。
 */
export function documentForCpStep(
  doc: Document,
  currentStep: number | null,
  stepCreases?: readonly StepCreases[] | null,
): Document {
  if (currentStep === null) return doc;
  const step = normalizeStep(doc, currentStep);
  if (step === doc.sequence.length) return doc;

  const history = historyFor(doc, stepCreases ?? null);
  const cached = history.documents.get(step);
  if (cached) return cached;

  const edges = doc.cp.edges.filter((edge) => {
    const added = history.introducedByEdge.get(edge.id) ?? null;
    return added === null || added <= step;
  });
  let result: Document = doc;
  if (edges.length !== doc.cp.edges.length) {
    // 見えている線の端点だけを残す。どの線にも使われていない点は、線の増減と
    // 関係なく置かれた点なので残す
    const shown = new Set<number>();
    for (const edge of edges) {
      shown.add(edge.v0);
      shown.add(edge.v1);
    }
    const used = new Set<number>();
    for (const edge of doc.cp.edges) {
      used.add(edge.v0);
      used.add(edge.v1);
    }
    const vertices = doc.cp.vertices.filter(
      (vertex) => shown.has(vertex.id) || !used.has(vertex.id),
    );
    result = { ...doc, cp: { ...doc.cp, edges, vertices } };
  }
  history.documents.set(step, result);
  return result;
}

/**
 * その手順の展開図に無い点の丸を出さないよう、畳めない点の一覧を絞る。
 * 減らす点が無ければ元の配列をそのまま返す。
 */
export function violationsForCpStep(
  stepDocument: Document,
  violations: number[],
): number[] {
  if (violations.length === 0) return violations;
  const shown = new Set(stepDocument.cp.vertices.map((vertex) => vertex.id));
  const kept = violations.filter((id) => shown.has(id));
  return kept.length === violations.length ? violations : kept;
}
