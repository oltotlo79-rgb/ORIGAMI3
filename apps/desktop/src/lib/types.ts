// Rust側(ori3-model / ori3-cp / store.rs)のserde出力に対応するTS型。
// フィールド名はserdeのJSON表現と厳密に一致させる(snake_caseのまま)。

/** 正規化座標 [x, y](紙の長辺 = 1.0、y軸上向き) */
export type Vec2 = [number, number];

export type EdgeKind = "Border" | "Mountain" | "Valley" | "Aux";

export interface Paper {
  width_mm: number;
  height_mm: number;
}

export interface Vertex {
  id: number;
  pos: Vec2;
}

export interface Edge {
  id: number;
  v0: number;
  v1: number;
  kind: EdgeKind;
}

export interface CreasePattern {
  vertices: Vertex[];
  edges: Edge[];
  next_vertex_id: number;
  next_edge_id: number;
}

export type TechniqueKind =
  | "Simple"
  | "Pleat"
  | "InsideReverse"
  | "OutsideReverse"
  | "Petal"
  | "Squash"
  | "OpenSink"
  | "Swivel"
  | "Twist"
  | "Pose";

/** ヒンジ角: 0=平ら, +180=完全な山折り, -180=完全な谷折り(度) */
export interface Driver {
  hinge: number;
  target_angle_deg: number;
}

export interface FoldStep {
  id: number;
  kind: TechniqueKind;
  drivers: Driver[];
  /** 平坦到達時の層順序(下→上)。各面は内部代表点で参照。平坦にならない場合null */
  layer_order: Vec2[] | null;
  note: string;
}

export interface DisplaySettings {
  front_color: [number, number, number];
  back_color: [number, number, number];
  grid_divisions: number;
}

export interface Document {
  schema_version: number;
  paper: Paper;
  cp: CreasePattern;
  sequence: FoldStep[];
  display: DisplaySettings;
}

/** 展開図から導出される面(ori3-cp::Face) */
export interface Face {
  id: number;
  /** 境界を反時計回りに一周する頂点ID列 */
  vertices: number[];
  /** verticesと同順の境界辺ID列 */
  edges: number[];
}

/** save以外の全コマンド成功時の戻り値(store.rs::DocumentView) */
export interface DocumentView {
  doc: Document;
  faces: Face[];
  warnings: string[];
  violations: number[];
}

/** edit_apply の操作(serde内部タグ形式: { "type": "..." }) */
export type EditOp =
  | { type: "AddSegment"; a: Vec2; b: Vec2; kind: EdgeKind }
  | { type: "RemoveEdges"; ids: number[] }
  | { type: "SetEdgeKind"; ids: number[]; kind: EdgeKind }
  | { type: "MoveVertex"; id: number; to: Vec2 }
  | { type: "SetPaper"; paper: Paper }
  | { type: "ReplaceCreasePattern"; cp: CreasePattern };

/** sequence_apply の操作(serde内部タグ形式) */
export type SeqOp =
  | { type: "PushStep"; step: FoldStep }
  | { type: "InsertStep"; index: number; step: FoldStep }
  | { type: "RemoveStep"; id: number }
  | { type: "UpdateStep"; step: FoldStep };

/** 3D表示用フレーム(Task 1-9で使用) */
export interface Face3D {
  face: number;
  polygon: [number, number, number][];
  layer: number;
}

export interface Frame3D {
  faces: Face3D[];
  warnings: string[];
}

/** pose_solve の戻り値(ori3-rigid::SolveResult) */
export interface SolveResult {
  frame: Frame3D;
  converged: boolean;
  /** 全ヒンジの角度(度)。キーは辺ID(JSONでは文字列になる) */
  angles: Record<string, number>;
  /** 実行した反復回数(warm start効果の確認用) */
  iterations: number;
}
