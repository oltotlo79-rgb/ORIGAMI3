import type {
  AlignTarget,
  Document,
  FoldStep,
  StepCreases,
  Vec2,
} from "../../../desktop/src/lib/types";

/**
 * Rust の `FinishSoftSettings` と同じ、再生に必要な確定済み設定だけを表す。
 * 頂点位置・反復計算の途中値・warm start は含めない。
 */
export interface SavedFinishSoftSettings {
  enabled: boolean;
  stiffness: number;
  pressure: number;
}

export type SavedFoldStep = FoldStep & {
  finish_soft?: SavedFinishSoftSettings | null;
};

/** Rust の `SavedDocument` と同じ永続化境界。 */
export interface SavedDocumentSnapshot
  extends Omit<Document, "sequence"> {
  sequence: SavedFoldStep[];
  step_creases?: StepCreases[];
}

export interface SavedDocumentSource {
  doc: Document & { sequence: SavedFoldStep[] };
  step_creases?: readonly StepCreases[];
}

function copyVec2(value: Vec2): Vec2 {
  return [value[0], value[1]];
}

function copyAlignTarget(target: AlignTarget): AlignTarget {
  if (target.kind === "point") {
    return { kind: "point", p: copyVec2(target.p) };
  }
  return {
    kind: "line",
    a: copyVec2(target.a),
    b: copyVec2(target.b),
  };
}

function copyFoldStep(step: SavedFoldStep): SavedFoldStep {
  const copied: SavedFoldStep = {
    id: step.id,
    kind: step.kind,
    drivers: step.drivers.map((driver) => ({
      a: copyVec2(driver.a),
      b: copyVec2(driver.b),
      target_angle_deg: driver.target_angle_deg,
    })),
    layer_order: step.layer_order?.map(copyVec2) ?? null,
    note: step.note,
  };
  if (step.alignment !== undefined && step.alignment !== null) {
    copied.alignment = {
      mode: step.alignment.mode,
      picks: step.alignment.picks.map(copyAlignTarget),
    };
  }
  if (step.finish_soft !== undefined && step.finish_soft !== null) {
    copied.finish_soft = {
      enabled: step.finish_soft.enabled,
      stiffness: step.finish_soft.stiffness,
      pressure: step.finish_soft.pressure,
    };
  }
  return copied;
}

/**
 * 保存可能なフィールドを新しいオブジェクトへ一つずつ投影する。
 * 入力に `frame`、`angles`、`warm_seed`、`fold_all_preview` 等が混入しても
 * 出力へ到達しない。
 */
export function projectSavedDocument(
  source: SavedDocumentSource,
): SavedDocumentSnapshot {
  const { doc } = source;
  const display: SavedDocumentSnapshot["display"] = {
    front_color: [...doc.display.front_color],
    back_color: [...doc.display.back_color],
    grid_divisions: doc.display.grid_divisions,
  };
  if (doc.display.overlap_prevention_enabled !== undefined) {
    display.overlap_prevention_enabled =
      doc.display.overlap_prevention_enabled;
  }
  if (doc.display.penetration_prevention_enabled !== undefined) {
    display.penetration_prevention_enabled =
      doc.display.penetration_prevention_enabled;
  }
  if (doc.display.soft_enabled !== undefined) {
    display.soft_enabled = doc.display.soft_enabled;
  }
  if (doc.display.soft_stiffness !== undefined) {
    display.soft_stiffness = doc.display.soft_stiffness;
  }
  if (doc.display.soft_pressure !== undefined) {
    display.soft_pressure = doc.display.soft_pressure;
  }

  const snapshot: SavedDocumentSnapshot = {
    schema_version: doc.schema_version,
    paper: {
      width_mm: doc.paper.width_mm,
      height_mm: doc.paper.height_mm,
    },
    cp: {
      vertices: doc.cp.vertices.map((vertex) => ({
        id: vertex.id,
        pos: copyVec2(vertex.pos),
      })),
      edges: doc.cp.edges.map((edge) => ({
        id: edge.id,
        v0: edge.v0,
        v1: edge.v1,
        kind: edge.kind,
      })),
      next_vertex_id: doc.cp.next_vertex_id,
      next_edge_id: doc.cp.next_edge_id,
    },
    sequence: doc.sequence.map(copyFoldStep),
    display,
  };
  if (source.step_creases !== undefined && source.step_creases.length > 0) {
    snapshot.step_creases = source.step_creases.map((entry) => ({
      step: entry.step,
      lines: entry.lines.map(([a, b]) => [copyVec2(a), copyVec2(b)]),
    }));
  }
  return snapshot;
}

export function serializeSavedDocument(source: SavedDocumentSource): string {
  return JSON.stringify(projectSavedDocument(source));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 保存領域の破損を日本語エラーへ変換し、余分なフィールドを再度除外する。 */
export function parseSavedDocument(payload: string): SavedDocumentSnapshot {
  let parsed: unknown;
  try {
    parsed = JSON.parse(payload) as unknown;
  } catch {
    throw new Error("復旧データのJSONが壊れているため、作品を復元できません。");
  }
  if (
    !isRecord(parsed) ||
    !isRecord(parsed.paper) ||
    !isRecord(parsed.cp) ||
    !Array.isArray(parsed.sequence) ||
    !isRecord(parsed.display)
  ) {
    throw new Error("復旧データに作品として必要な項目がありません。");
  }
  try {
    return projectSavedDocument({
      doc: parsed as unknown as SavedDocumentSource["doc"],
      step_creases: Array.isArray(parsed.step_creases)
        ? (parsed.step_creases as unknown as StepCreases[])
        : undefined,
    });
  } catch {
    throw new Error("復旧データの作品形式を読み取れません。");
  }
}
