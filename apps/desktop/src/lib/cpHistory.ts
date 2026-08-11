import type { Document, DriverLine, Edge, Vec2 } from "./types";

/** CP座標で、点が手順のdriver線分上にあるとみなす距離。 */
export const CP_HISTORY_EPSILON = 1e-8;

/**
 * Documentはストアで更新のたびに参照が入れ替わるため、Document参照を世代として
 * 手順ごとの表示用Documentを覚える。古い作品はWeakMapから自然に解放される。
 */
interface CpHistoryCache {
  /** 最終辺ID -> 初めて追加された1始まり手順。nullは事前折り線。 */
  introducedByEdge: ReadonlyMap<number, number | null>;
  documents: Map<number, Document>;
}

const historyCache = new WeakMap<Document, CpHistoryCache>();

function finitePoint(point: Vec2): boolean {
  return Number.isFinite(point[0]) && Number.isFinite(point[1]);
}

/** 端点の外側も距離EPS以内なら同じ有限線分上として扱う。 */
function pointOnDriverSegment(point: Vec2, driver: DriverLine): boolean {
  if (!finitePoint(point) || !finitePoint(driver.a) || !finitePoint(driver.b)) {
    return false;
  }

  const dx = driver.b[0] - driver.a[0];
  const dy = driver.b[1] - driver.a[1];
  const lengthSquared = dx * dx + dy * dy;
  if (lengthSquared <= CP_HISTORY_EPSILON * CP_HISTORY_EPSILON) return false;

  const px = point[0] - driver.a[0];
  const py = point[1] - driver.a[1];
  const projection = (px * dx + py * dy) / lengthSquared;
  const along = Math.max(0, Math.min(1, projection));
  const nearestX = driver.a[0] + along * dx;
  const nearestY = driver.a[1] + along * dy;
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

/**
 * 最終CPの辺が初めて使われる手順を求める。
 * 見つからない辺や壊れた辺は事前折り線として扱い、隠さない。
 */
function introducedAt(
  doc: Document,
  edge: Edge,
  vertices: ReadonlyMap<number, Vec2>,
): number | null {
  if (edge.kind !== "Mountain" && edge.kind !== "Valley") return null;
  const a = vertices.get(edge.v0);
  const b = vertices.get(edge.v1);
  if (!a || !b) return null;

  for (let index = 0; index < doc.sequence.length; index += 1) {
    const step = doc.sequence[index];
    if (step.kind === "Pose") continue;
    for (const driver of step.drivers) {
      if (
        usableDriver(driver) &&
        pointOnDriverSegment(a, driver) &&
        pointOnDriverSegment(b, driver)
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

/** 辺と全driverの照合は作品ごとに1回だけ行い、コマ送り中は再利用する。 */
function historyFor(doc: Document): CpHistoryCache {
  const cached = historyCache.get(doc);
  if (cached) return cached;

  const vertices = new Map(doc.cp.vertices.map((vertex) => [vertex.id, vertex.pos]));
  const introducedByEdge = new Map(
    doc.cp.edges.map((edge) => [edge.id, introducedAt(doc, edge, vertices)]),
  );
  const history = { introducedByEdge, documents: new Map<number, Document>() };
  historyCache.set(doc, history);
  return history;
}

/**
 * タイムライン位置で既に付いている折り線だけを持つ2D表示用Documentを返す。
 * 元Documentは変更せず、最新表示では余計な複製を作らず元参照を返す。
 */
export function documentForCpStep(
  doc: Document,
  currentStep: number | null,
): Document {
  if (currentStep === null) return doc;
  const step = normalizeStep(doc, currentStep);
  if (step === doc.sequence.length) return doc;

  const history = historyFor(doc);
  const cached = history.documents.get(step);
  if (cached) return cached;

  const edges = doc.cp.edges.filter((edge) => {
    const added = history.introducedByEdge.get(edge.id) ?? null;
    return added === null || added <= step;
  });
  const result: Document =
    edges.length === doc.cp.edges.length
      ? doc
      : { ...doc, cp: { ...doc.cp, edges } };
  history.documents.set(step, result);
  return result;
}
