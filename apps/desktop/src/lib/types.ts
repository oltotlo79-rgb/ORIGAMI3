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

/**
 * ヒンジ角: 0=平ら, +180=完全な山折り, -180=完全な谷折り(度)
 * pose_solve(スライダー操作)専用の一時指定。手順の永続化には使わない
 */
export interface Driver {
  hinge: number;
  target_angle_deg: number;
}

/**
 * 手順永続化用のdriver: 折り線をCP座標の線分で指定する。
 * 再生時は線分上に乗る折り辺すべて(分割後の断片を含む)を対象角へ駆動する
 */
export interface DriverLine {
  a: Vec2;
  b: Vec2;
  target_angle_deg: number;
}

export interface FoldStep {
  id: number;
  kind: TechniqueKind;
  drivers: DriverLine[];
  /** 平坦到達時の層順序(下→上)。各面は内部代表点で参照。平坦にならない場合null */
  layer_order: Vec2[] | null;
  note: string;
}

/**
 * 折る向き。
 * Up = 動く側を反転して山の一番上に載せる(手前へ折る=谷折り)、
 * Down = 一番下に入れる(向こうへ折る=山折り)
 */
export type FoldDirection = "Up" | "Down";

export interface DisplaySettings {
  front_color: [number, number, number];
  back_color: [number, number, number];
  grid_divisions: number;
  /** 折り動作中の紙どうしの重なりを補正するか。既定はオン。
   * 古い作品ファイルには無いので省略可とし、falseのときだけ切る */
  overlap_prevention_enabled?: boolean;
  /** 紙のたわみを表現するか(SIM-012)。既定はオフ。
   * 古い作品ファイルには無いのでRust側が既定値で埋める(省略可) */
  soft_enabled?: boolean;
  /** 紙の硬さ(0.0〜1.0)。大きいほど面の中が平らに保たれる */
  soft_stiffness?: number;
  /** 膨らみの強さ(0.0〜1.0、SIM-013)。0.0なら膨らませない */
  soft_pressure?: number;
}

/** たわみ計算の指定(ori3-soft::SoftSettings)。
 * SIM-015のとおり、たわみの状態はこの値だけで表す(頂点の位置は保存しない) */
export interface SoftSettings {
  enabled: boolean;
  subdivision: number;
  stiffness: number;
  pressure: number;
  iterations: number;
}

/** たわませた三角形の網(ori3-soft::SoftMesh)。表示専用 */
export interface SoftMesh {
  positions: [number, number, number][];
  triangles: [number, number, number][];
  /** 三角形→元の面ID(表裏の色分け・当たり判定用) */
  triangle_faces: number[];
  /** 三角形→層番号(下から0,1,2…) */
  triangle_layers: number[];
  warnings: string[];
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
  /** 最新ステップまで自動再生した立体(SEQ-004)。手順が空ならnull */
  frame: Frame3D | null;
  /** 自動再生で折り線が見つからず飛ばされたステップID */
  skipped: number[];
  /**
   * 折り切る前の非破壊確認で見つかった、巻き込みに必要な追加折り目。
   * 通常のDocumentViewでは未指定またはnullになる。
   */
  fold_through_proposal?: FoldThroughProposal | null;
}

/**
 * 紙の縁へぶつかる単純な1か所を、巻き込み折りで避ける提案。
 * folded_lineは現在の畳み平面(3D表示)の座標、crease_segmentsは
 * 元の展開図(CP)へ写した線分で、同じ追加折り目を2つの区画に表示する。
 */
export interface FoldThroughProposal {
  folded_line: [Vec2, Vec2];
  crease_segments: [Vec2, Vec2][];
  message: string;
}

/** edit_apply の操作(serde内部タグ形式: { "type": "..." }) */
export type EditOp =
  | { type: "AddSegment"; a: Vec2; b: Vec2; kind: EdgeKind }
  | { type: "RemoveEdges"; ids: number[] }
  | { type: "SetEdgeKind"; ids: number[]; kind: EdgeKind }
  | { type: "MoveVertex"; id: number; to: Vec2 }
  | { type: "SetPaper"; paper: Paper }
  | { type: "ReplaceCreasePattern"; cp: CreasePattern }
  /** 紙の色と方眼の分割数(PAP-003 / CPE-003)。作品ごとの設定として
   * .ori3ファイルに保存され、元に戻す/やり直しの対象になる。
   * grid_divisionsが2〜64の外ならRust側が丸めて警告を返す */
  | { type: "SetDisplay"; display: DisplaySettings };

/** sequence_apply の操作(serde内部タグ形式) */
export type SeqOp =
  | { type: "PushStep"; step: FoldStep }
  | { type: "InsertStep"; index: number; step: FoldStep }
  | { type: "RemoveStep"; id: number }
  | { type: "UpdateStep"; step: FoldStep }
  /**
   * 畳んだ状態の上に折り線を引いてまとめて折る。座標は畳み平面(3D表示のxy)。
   * up_toはこの折りの直前までの手順数(末尾でも途中でもよい)。
   * target_layersがnullなら可動側に掛かる全ての層を折る
   */
  | {
      type: "FoldThrough";
      /** この折りの直前までの手順数。手順数と同じなら末尾へ足し、
       * 途中の値ならその位置へ挟む(後続の手順はそのまま残る) */
      up_to: number;
      line: [Vec2, Vec2];
      keep_side_point: Vec2;
      target_layers: number[] | null;
      direction: FoldDirection;
      /** trueなら事前提案された追加折り目を入れて、巻き込みながら折る。 */
      accept_additional_crease: boolean;
    }
  /**
   * FoldThroughをまだ作品へ適用せず、巻き込み折り目が必要かだけ調べる。
   * 結果はDocumentView.fold_through_proposalに入る。
   */
  | {
      type: "PreviewFoldThrough";
      up_to: number;
      line: [Vec2, Vec2];
      keep_side_point: Vec2;
      target_layers: number[] | null;
      direction: FoldDirection;
    }
  /**
   * 基本技法(段折り・中割り折り・かぶせ折り)をまとめて折る。
   * 座標はFoldThroughと同じ畳み平面。flapは対象の層(段折りでは空を許す)。
   * reference_pointの意味は技法ごとに違う(段折り=2本目の折り線の位置、
   * 中割り・かぶせ=先端が向かう側)
   */
  | {
      type: "Technique";
      up_to: number;
      kind: TechniqueKind;
      flap: number[];
      line: [Vec2, Vec2];
      reference_point: Vec2;
      /** ねじり折りの中央多角形(畳み平面の頂点を順に並べる。3点以上)。
       * 省略するとlineを1辺として中心のまわりに回した正多角形になる。
       * 辺の数も長さも仮定しないので、任意の形の中央多角形を指定できる */
      polygon?: Vec2[];
      /** ねじり折りの中心。省略すると選んだ層の重心 */
      center?: Vec2;
    };

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

/** sequence_replay の戻り値(ori3-layers::ReplayResult) */
export interface ReplayResult {
  /** 3D表示用フレーム(Face3D.layerは下から0,1,2…) */
  frame: Frame3D;
  /** 折り線が見つからず飛ばされたステップID */
  skipped: number[];
  warnings: string[];
  /** たわみを指定したときだけ入る三角形の網(SIM-012) */
  soft?: SoftMesh | null;
}

/** pose_solve の戻り値(ori3-rigid::SolveResult) */
export interface SolveResult {
  frame: Frame3D;
  converged: boolean;
  /** 全ヒンジの角度(度)。キーは辺ID(JSONでは文字列になる) */
  angles: Record<string, number>;
  /** 実行した反復回数(warm start効果の確認用) */
  iterations: number;
  /** たわみを指定したときだけ入る三角形の網(SIM-012) */
  soft?: SoftMesh | null;
}

/**
 * 骨格の節点(ori3-propose::SkeletonNode)。
 * parentがnullの節点が根(胴の中心)で、ちょうど1つだけ置く。
 * lengthは親へつながる辺の長さ(根では使わない)、width_factorは太さ(膨らみ)。
 */
export interface SkeletonNode {
  id: number;
  parent: number | null;
  length: number;
  width_factor: number;
}

/** 骨格全体(ori3-propose::Skeleton) */
export interface Skeleton {
  nodes: SkeletonNode[];
}

/** proposal_generate が返す展開図の候補1つ分(commands.rs::ProposalCandidate) */
export interface ProposalCandidate {
  cp: CreasePattern;
  /** 骨格の長さ1あたりが紙の何割になるか(大きいほど完成品が大きい) */
  scale: number;
  /** 平坦に折りにくい頂点の数(0が理想。0でなくても使える) */
  violations: number;
  warnings: string[];
}

/** recovery_check の戻り値。前回の異常終了で残った自動保存の情報(SYS-003) */
export interface RecoveryInfo {
  /** 自動保存ファイルの場所 */
  autosave_path: string;
  /** 元の保存先(保存したことがない作品ならnull) */
  document_path: string | null;
  /** 最後に自動保存した時刻(1970年からのミリ秒)。分からなければnull */
  saved_at_ms: number | null;
}

/** document_export の書き出しの種類(commands.rs::ExportKind)。
 * 展開図の画像はSVG(実寸mm)とPNGの2つ。折り図はPDF(1ファイル)と
 * ページごとのSVG(複数ファイル)の2つ */
export type ExportKind = "CpSvg" | "CpPng" | "DiagramPdf" | "DiagramSvg";

/** document_export の細かい指定(commands.rs::ExportOptions) */
export interface ExportOptions {
  /** 補助線(下書きの線)も一緒に書き出すか */
  include_aux: boolean;
  /** PNGのときの長いほうの辺の点数 */
  png_long_side: number;
}
