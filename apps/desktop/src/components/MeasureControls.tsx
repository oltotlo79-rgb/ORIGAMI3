import { measureGuide } from "../lib/measureGuide";
import {
  formatAngleMeasurement,
  formatLengthMeasurement,
  measurePlanarAngle,
  measurePlanarDistance,
  measureSegmentLength,
  measureSpatialDistance,
  type MeasurementDisplay,
  type Segment2,
} from "../lib/measurements";
import type { Document } from "../lib/types";
import {
  useAppStore,
  type MeasureEdgePick,
  type MeasureMode,
  type MeasurePointPick,
} from "../store/appStore";
import { mapCpPointToFrame } from "./Viewer3D/edgeHighlight";

const MODE_LABEL: Record<MeasureMode, string> = {
  angle: "角度",
  length: "線の長さ",
  distance: "2点の距離",
};

const MODE_ORDER: MeasureMode[] = ["angle", "length", "distance"];

const REQUIRED_PICKS: Record<MeasureMode, number> = {
  angle: 2,
  length: 1,
  distance: 2,
};

interface ResultRow {
  label: string;
  display: MeasurementDisplay;
}

interface ResultView {
  rows: ResultRow[];
  exactAvailable: boolean;
  exactDefault: boolean;
  exactUnavailableReason: string | null;
  notes: string[];
  invalidReason: string | null;
}

function segmentForEdge(doc: Document, edgeId: number): Segment2 | null {
  const edge = doc.cp.edges.find((candidate) => candidate.id === edgeId);
  if (!edge) return null;
  const first = doc.cp.vertices.find((vertex) => vertex.id === edge.v0)?.pos;
  const second = doc.cp.vertices.find((vertex) => vertex.id === edge.v1)?.pos;
  return first && second ? [first, second] : null;
}

function emptyResult(invalidReason: string | null = null): ResultView {
  return {
    rows: [],
    exactAvailable: false,
    exactDefault: false,
    exactUnavailableReason: null,
    notes: [],
    invalidReason,
  };
}

function resultForAngle(
  doc: Document,
  picks: readonly MeasureEdgePick[],
  display: "decimal" | "exact" | null,
): ResultView {
  if (picks.length < 2) return emptyResult();
  const first = segmentForEdge(doc, picks[0].edgeId);
  const second = segmentForEdge(doc, picks[1].edgeId);
  const measurement = first && second ? measurePlanarAngle(first, second) : null;
  if (!measurement) {
    return emptyResult("この線では角度を測れません。別の線を選んでください。");
  }
  const exactAvailable = measurement.kind === "exact";
  const format = display === "exact" ? "fraction" : display ?? "default";
  return {
    rows: [
      {
        label: "角度",
        display: formatAngleMeasurement(measurement, format),
      },
    ],
    exactAvailable,
    exactDefault: exactAvailable && measurement.defaultKind === "fraction",
    exactUnavailableReason: exactAvailable
      ? null
      : "この角度は度を分数で正確に表せないため、小数で表示します。",
    notes: [],
    invalidReason: null,
  };
}

function resultForLength(
  doc: Document,
  picks: readonly MeasureEdgePick[],
  display: "decimal" | "exact" | null,
): ResultView {
  if (picks.length < 1) return emptyResult();
  const segment = segmentForEdge(doc, picks[0].edgeId);
  const scaleMm = Math.max(doc.paper.width_mm, doc.paper.height_mm);
  const measurement = segment ? measureSegmentLength(segment, scaleMm) : null;
  if (!measurement) {
    return emptyResult("この線では長さを測れません。別の線を選んでください。");
  }
  const exactAvailable = measurement.kind === "exact";
  return {
    rows: [
      {
        label: "線の長さ",
        display: formatLengthMeasurement(measurement, display ?? "default"),
      },
    ],
    exactAvailable,
    exactDefault: exactAvailable,
    exactUnavailableReason: exactAvailable
      ? null
      : "この長さは正確な形で表せないため、小数で表示します。",
    notes: [],
    invalidReason: null,
  };
}

function resultForDistance(
  doc: Document,
  faces: ReturnType<typeof useAppStore.getState>["faces"],
  frame3d: ReturnType<typeof useAppStore.getState>["frame3d"],
  picks: readonly MeasurePointPick[],
  display: "decimal" | "exact" | null,
): ResultView {
  if (picks.length < 2) return emptyResult();
  const scaleMm = Math.max(doc.paper.width_mm, doc.paper.height_mm);
  const planar = measurePlanarDistance(picks[0].cp, picks[1].cp, scaleMm);
  if (!planar) {
    return emptyResult("この2つの点では距離を測れません。別の点を選んでください。");
  }

  const firstWorld = mapCpPointToFrame(
    doc,
    faces,
    frame3d,
    picks[0].cp,
    picks[0].faceId,
  );
  const secondWorld = mapCpPointToFrame(
    doc,
    faces,
    frame3d,
    picks[1].cp,
    picks[1].faceId,
  );
  const spatial =
    firstWorld && secondWorld
      ? measureSpatialDistance(firstWorld, secondWorld, scaleMm)
      : null;
  const exactAvailable = planar.kind === "exact";
  const rows: ResultRow[] = [
    {
      label: "展開図での距離",
      display: formatLengthMeasurement(planar, display ?? "default"),
    },
  ];
  if (spatial) {
    rows.push({
      label: "3D図での距離",
      display: formatLengthMeasurement(spatial, "decimal"),
    });
  } else {
    rows.push({
      label: "3D図での距離",
      display: { primary: "測れません", secondary: null, approximate: true },
    });
  }
  return {
    rows,
    exactAvailable,
    exactDefault: exactAvailable,
    exactUnavailableReason: exactAvailable
      ? null
      : "展開図での距離は正確な形で表せないため、小数で表示します。",
    notes: [
      spatial
        ? "立体での距離は、およその値で表示します。"
        : "この位置では3D図での距離を測れません。別の点を選んでください。",
    ],
    invalidReason: null,
  };
}

function ResultCard({ rows }: { rows: readonly ResultRow[] }) {
  return (
    <section className="measure-result-card" aria-label="測定結果">
      {rows.map(({ label, display }) => (
        <div className="measure-result-row" key={label}>
          <strong>{label}</strong>
          <output aria-live="polite">
            <span className="measure-result-primary">{display.primary}</span>
            {display.secondary !== null && (
              <small className="measure-result-secondary">
                （<span>{display.secondary}</span>）
              </small>
            )}
          </output>
        </div>
      ))}
    </section>
  );
}

export function MeasureControls() {
  const doc = useAppStore((state) => state.doc);
  const faces = useAppStore((state) => state.faces);
  const frame3d = useAppStore((state) => state.frame3d);
  const draft = useAppStore((state) => state.measureDraft);
  const setMeasureMode = useAppStore((state) => state.setMeasureMode);
  const setMeasureDisplay = useAppStore((state) => state.setMeasureDisplay);
  const clearMeasurement = useAppStore((state) => state.clearMeasurement);

  if (!doc) return <p>読み込み中…</p>;

  const edgePicks = draft.picks.filter(
    (pick): pick is MeasureEdgePick => pick.kind === "edge",
  );
  const pointPicks = draft.picks.filter(
    (pick): pick is MeasurePointPick => pick.kind === "point",
  );
  const pickCount = draft.mode === "distance" ? pointPicks.length : edgePicks.length;
  const required = REQUIRED_PICKS[draft.mode];
  const result =
    draft.mode === "angle"
      ? resultForAngle(doc, edgePicks, draft.display)
      : draft.mode === "length"
        ? resultForLength(doc, edgePicks, draft.display)
        : resultForDistance(doc, faces, frame3d, pointPicks, draft.display);
  const exactSelected =
    result.exactAvailable &&
    (draft.display === "exact" ||
      (draft.display === null && result.exactDefault));
  const hasResult = result.rows.length > 0;

  return (
    <section className="measure-controls" aria-label="測る">
      <div className="button-row measure-mode-row">
        <strong className="row-label">測り方</strong>
        <div className="measure-mode-buttons" role="group" aria-label="測り方">
          {MODE_ORDER.map((mode) => (
            <button
              key={mode}
              type="button"
              aria-pressed={draft.mode === mode}
              onClick={() => setMeasureMode(mode)}
            >
              {MODE_LABEL[mode]}
            </button>
          ))}
        </div>
      </div>

      <div className="measure-progress" aria-live="polite">
        <strong>{measureGuide(draft.mode, pickCount)}</strong>
        <span>
          選択 {Math.min(pickCount, required)} / {required}
        </span>
      </div>
      <p className="hint measure-cancel-hint">Escでも途中の指定をやめられます</p>

      {result.invalidReason !== null && (
        <p className="warning-text">{result.invalidReason}</p>
      )}
      {hasResult && (
        <>
          <ResultCard rows={result.rows} />
          <div className="button-row measure-display-row">
            <span className="row-label">表示</span>
            <div
              className="measure-display-buttons"
              role="group"
              aria-label={`${MODE_LABEL[draft.mode]}の表示`}
            >
              <button
                type="button"
                aria-pressed={!exactSelected}
                onClick={() => setMeasureDisplay("decimal")}
              >
                小数
              </button>
              <button
                type="button"
                aria-pressed={exactSelected}
                disabled={!result.exactAvailable}
                data-tooltip={result.exactUnavailableReason ?? undefined}
                onClick={() => setMeasureDisplay("exact")}
              >
                {draft.mode === "angle" ? "度を分数で" : "正確な形"}
              </button>
            </div>
          </div>
          {result.exactUnavailableReason !== null && (
            <p className="hint measure-result-note">{result.exactUnavailableReason}</p>
          )}
          {result.notes.map((note) => (
            <p className="hint measure-result-note" key={note}>
              {note}
            </p>
          ))}
        </>
      )}

      {pickCount > 0 && (
        <div className="button-row measure-clear-row">
          <button type="button" onClick={() => clearMeasurement()}>
            選び直す
          </button>
        </div>
      )}
    </section>
  );
}
