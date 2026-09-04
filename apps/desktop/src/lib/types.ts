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
 * 前の希望角を、紙のつながりを保つために譲った量。
 * 1回の再生・追従計算だけに属する導出情報で、作品ファイルへは保存しない。
 */
export interface AngleRelaxation {
  hinge: number;
  target_angle_deg: number;
  actual_angle_deg: number;
  delta_deg: number;
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

/**
 * 「合わせて折る」の基準。RustのFoldAlignmentと同じJSON形。
 *
 * throughTwoPoints〜pointLinePerpendicular は藤田・羽鳥の7基本作図の順に対応する。
 * existingLine は、折り図で頻出する「付いている折り筋で折る」を座標の引き直しなしで
 * 指定するための追加モード。
 */
export type AlignMode =
  | "throughTwoPoints"
  | "pointPoint"
  | "lineLine"
  | "pointPerpendicularLine"
  | "pointLineThrough"
  | "pointToLinePointToLine"
  | "pointLinePerpendicular"
  | "existingLine";

export type AlignTarget =
  | { kind: "point"; p: Vec2 }
  | { kind: "line"; a: Vec2; b: Vec2 };

export interface FoldAlignment {
  mode: AlignMode;
  picks: AlignTarget[];
}

/**
 * 手順の表示に使う技法名の種類(ori3-layers::technique_classification)。
 * `TechniqueKind`とは別の集合で、`Simple`は`LayerOperation`(層操作)、
 * `Pose`は分類対象外(このkindがclassificationを持つことはない)に対応し、
 * 一意な技法として名前が付かなかった動きは`GrabMove`(つかんで動かした折り)になる。
 */
export type DisplayTechniqueKind =
  | "LayerOperation"
  | "Pleat"
  | "InsideReverse"
  | "OutsideReverse"
  | "Squash"
  | "Petal"
  | "OpenSink"
  | "Swivel"
  | "Twist"
  | "GrabMove";

/** classificationの由来。自動判定か、利用者が明示的に付けたか。 */
export type TechniqueClassificationOrigin = "Automatic" | "Explicit";

/** 手順に記録された技法名。RustのTechniqueClassificationと同じJSON形。 */
export interface TechniqueClassification {
  kind: DisplayTechniqueKind;
  origin: TechniqueClassificationOrigin;
}

export interface FoldStep {
  id: number;
  kind: TechniqueKind;
  drivers: DriverLine[];
  /** 平坦到達時の層順序(下→上)。各面は内部代表点で参照。平坦にならない場合null */
  layer_order: Vec2[] | null;
  /** 合わせ折りで選んだ点・線。旧形式の作品では省略される */
  alignment?: FoldAlignment | null;
  /**
   * 手順に記録された技法名。旧作品・Pose・分類対象外の手順では項目そのものが無い
   * (undefined)。利用者が「折り方」selectでkindを明示的に選び直したときだけ、
   * 送るUpdateStepの手順からこの項目を落とす(この場合はnullまたは項目なしになる)。
   */
  technique_classification?: TechniqueClassification | null;
  note: string;
}

/**
 * 折る向き。
 * Up = 動く側を反転して山の一番上に載せる(手前へ折る=谷折り)、
 * Down = 一番下に入れる(向こうへ折る=山折り)
 */
export type FoldDirection = "Up" | "Down";

/** 汎用層操作の対象領域。inside_point側の半平面を使う。 */
export interface HalfPlane {
  line: [Vec2, Vec2];
  inside_point: Vec2;
}

/** Rustの外部タグenumと同じJSON形。鏡映は先頭から順に適用する。 */
export type MotionTransform =
  | "Stay"
  | { Reflect: [Vec2, Vec2][] };

/** 動かした層を重なりのどこへ置くか。 */
export type LayerTurn =
  | "Keep"
  | { Outside: FoldDirection }
  | { Inside: FoldDirection }
  | { Beside: { anchor: number; direction: FoldDirection } };

/** 1回の汎用層操作の中で、同じように動く紙の一部分。 */
export interface MotionPart {
  /** 操作開始時点の面ID。空なら全層。 */
  layers: number[];
  /** 空なら選んだ層の全域。既存折り目の開閉・重ね替えでは空を使う。 */
  region: HalfPlane[];
  transform: MotionTransform;
  turn: LayerTurn;
  /** 省略時は変換から自動決定。 */
  reverse_layers?: boolean;
}

export interface DisplaySettings {
  front_color: [number, number, number];
  back_color: [number, number, number];
  grid_divisions: number;
  /** 紙どうしの食い込みを減らすように形を補正するか。既定はオフ。
   * 古い作品ファイルには無いので省略可とし、trueのときだけ使う */
  overlap_prevention_enabled?: boolean;
  /** 紙どうしの食い込みを検出して警告するか。形は変えず、既定はオン。
   * 古い作品ファイルには無いので省略可とし、falseのときだけ切る */
  penetration_prevention_enabled?: boolean;
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

/**
 * 1つの手順が展開図へ新しく足した折り線(CP座標の線分)。
 *
 * 先に描いてあった折り線と、その手順で足された折り線は、最終展開図と手順の
 * 線分だけからは区別できない。区別を推測に頼らないための来歴で、既にある
 * 折り筋で折った手順は `lines` が空になる。並べ替え・削除でずれないよう、
 * 手順の位置ではなく手順IDで結び付ける。
 */
export interface StepCreases {
  step: number;
  lines: [Vec2, Vec2][];
}

export interface Document {
  schema_version: number;
  paper: Paper;
  cp: CreasePattern;
  sequence: FoldStep[];
  display: DisplaySettings;
}

/** ほかの折り紙ソフトのファイルを読み書きしたときの注意の種類。 */
export const FOLD_ISSUE_CODES = [
  "assignment_downgraded_to_aux",
  "unsupported_field",
  "unsupported_geometry",
  "non_linear_frames",
  "unrepresentable_face_orders",
  "invalid_topology",
  "missing_required_field",
  "invalid_value",
] as const;

export type FoldIssueCode = (typeof FOLD_ISSUE_CODES)[number];
export type FoldIssueSeverity = "warning" | "error";

/** Rust側のFoldIssue。画面にはraw値を出さず、codeを安全な日本語へ変換する。 */
export interface FoldIssue {
  severity: FoldIssueSeverity;
  code: FoldIssueCode;
  path: string;
  message: string;
  original_value?: unknown;
}

/** 展開図から導出される面(ori3-cp::Face) */
export interface Face {
  id: number;
  /** 境界を反時計回りに一周する頂点ID列 */
  vertices: number[];
  /** verticesと同順の境界辺ID列 */
  edges: number[];
}

/** 紙を実際に突き抜けた2面。backendの決定順を画面でも維持する。 */
export type SelfIntersectionPair = readonly [number, number];

/** save以外の全コマンド成功時の戻り値(store.rs::DocumentView) */
export interface DocumentView {
  doc: Document;
  /** 手順ごとに新しく足した折り線。作品ファイルにも保存される。
   * 来歴を持たない旧形式の作品では空になる */
  step_creases?: StepCreases[];
  /** 他形式を読み込んだ際の注意。状態接続まではIPC結果内にだけ保持する。 */
  fold_issues?: FoldIssue[];
  faces: Face[];
  warnings: string[];
  /** 今回の平らに畳む操作で知らせる点。rawのviolationsとは別の絞り込み済み結果 */
  flat_fold_violations?: number[];
  violations: number[];
  /** 最新ステップまで自動再生した立体(SEQ-004)。手順が空ならnull */
  frame: Frame3D | null;
  /** 自動再生で折り線が見つからず飛ばされたステップID */
  skipped: number[];
  /** 最終形で紙どうしの接触を検出したか。接触しても操作は止めない */
  contact_detected: boolean;
  /** 補正後にも残る食い込みの原因候補ヒンジ */
  suspect_hinges?: number[];
  /** 最終姿勢で紙を実際に突き抜けた面IDの組。0件では旧backendとの互換上省略される。 */
  self_intersection_pairs?: SelfIntersectionPair[];
  /** 手順から現在の辺IDへ解決した希望角。保存データではなく再生の導出結果 */
  sequence_targets?: Driver[];
  /** 自動再生で得た全ヒンジの実角。次の操作のwarm startにも使う */
  angles?: Record<string, number>;
  /** 前の希望角を譲った診断（辺ID昇順） */
  relaxations?: AngleRelaxation[];
  /** 自動再生結果の閉包残差RMS */
  closure_rms?: number;
  /** 収束前でも現在指定を守った最良の有限候補を表示しているか */
  best_effort?: boolean;
  /** 自動再生の追従計算が収束したか */
  converged?: boolean;
  /**
   * 折り切る前の非破壊確認で見つかった、巻き込みに必要な追加折り目。
   * 通常のDocumentViewでは未指定またはnullになる。
   */
  fold_through_proposal?: FoldThroughProposal | null;
  /** 新しい折り線の直下で、同時に折れるひだを数えた非破壊照会結果。 */
  fold_target_info?: FoldTargetInfo | null;
}

/** document_openはRust側でfold_issuesを常に配列として返す。 */
export type DocumentOpenResult = DocumentView & { fold_issues: FoldIssue[] };

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

/** 新しい折り線の直下で、上から何枚のひだを同時に折れるか。 */
export type FoldTargetStatus =
  | "ready"
  | "limited"
  | "crease_only_top"
  | "varies"
  | "unavailable";

/** 一番上の紙が完全に折り重なっていないときの処置。 */
export type FoldTargetTopAction = "crease_only_top";

/**
 * 書類と折り手順だけから再計算した、保存を伴わないひだ照会結果。
 * availableCountはFace数ではなく、上から連続して同時に折れるひだの枚数。
 */
export interface FoldTargetInfo {
  status: FoldTargetStatus;
  availableCount: number | null;
  reason: string | null;
  topAction: FoldTargetTopAction | null;
}

/**
 * FoldThroughの直前に、書類から再現する平坦姿勢の宣言。
 * 画面の計算結果ではなく、利用者が指定した符号付き角度だけを送る。
 */
export interface FoldPoseDriver {
  edge_id: number;
  target_angle_deg: number;
}

export interface FoldPoseInput {
  drivers: FoldPoseDriver[];
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
   * grid_divisionsが2〜1024の外ならRust側が丸めて警告を返す */
  | { type: "SetDisplay"; display: DisplaySettings };

/** sequence_apply の操作(serde内部タグ形式) */
export type SeqOp =
  | { type: "PushStep"; step: FoldStep }
  | { type: "InsertStep"; index: number; step: FoldStep }
  | { type: "RemoveStep"; id: number }
  | { type: "MoveStep"; id: number; to_index: number }
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
      /** 上から同時に折るひだの枚数。Face IDは送らず、Rustが書類から再計算する。 */
      target_pleat_count?: number | null;
      direction: FoldDirection;
      /** 省略時はup_toの保存済み姿勢をそのまま使う。 */
      pose_before?: FoldPoseInput | null;
      /** 合わせ折りの説明文に使う点・線。折り計算には影響しない。 */
      alignment?: FoldAlignment | null;
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
      /** Applyと同じK。省略時は従来どおりtarget_layersを使う。 */
      target_pleat_count?: number | null;
      direction: FoldDirection;
      /** Applyと同じ不変な姿勢宣言。Preview自体は作品を変更しない。 */
      pose_before?: FoldPoseInput | null;
    }
  /**
   * 新しい折り線の直下で同時に折れるひだを、作品を変更せずに数える。
   * 結果はDocumentView.fold_target_infoに入る。
   */
  | {
      type: "PreviewFoldTargets";
      up_to: number;
      line: [Vec2, Vec2];
      keep_side_point: Vec2;
      /** FoldThroughと同じ、利用者が指定した符号付き角度だけを送る。 */
      pose_before?: FoldPoseInput | null;
    }
  /**
   * 非平坦な最上紙へ、3Dの形を動かさず材料上の折り目だけを付ける。
   * 表示座標・Face ID・K=0ではなく、CP材料座標と利用者のsigned角度だけを送る。
   */
  | {
      type: "CreaseOnlyTop";
      up_to: number;
      material_line: [Vec2, Vec2];
      /** 対象材料面の厳密な内部点で、折り目の残す側も示す。 */
      material_keep_side_point: Vec2;
      direction: FoldDirection;
      pose_before?: FoldPoseInput | null;
      /** 合わせ折りの説明だけに使い、Rustの折り計算には使わない。 */
      alignment?: FoldAlignment | null;
    }
  /** 非平坦な材料直線の最上紙だけ処理を、作品を変えずに照会する。 */
  | {
      type: "PreviewFoldTargetsOnMaterial";
      up_to: number;
      material_line: [Vec2, Vec2];
      material_keep_side_point: Vec2;
      pose_before?: FoldPoseInput | null;
    }
  /**
   * 名前付き技法に閉じない層操作。複数partを1手で同時に適用する。
   * 結果はRust側で通常のFoldStepへ変換され、作品に保存・再生される。
   */
  | {
      type: "FlatMotion";
      up_to: number;
      parts: MotionPart[];
      kind: TechniqueKind;
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
      /** つぶし・花弁・ひだ寄せ・ねじりで、動かした紙を向こう側へ入れる。
       * 省略/falseは手前。ほかの技法では送らない */
      open_to_back?: boolean;
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
  /** 同一深度の紙面だけに使う、下から上への重なり順位。 */
  surface_rank?: number;
  /** 折り木で面が鏡映された回数の偶奇。旧mockは省略できるがRust IPCは常に返す。 */
  mirrored?: boolean;
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
  /** 今回の平らに畳む操作で知らせる点 */
  flat_fold_violations?: number[];
  /** 補正後にも残る食い込みの原因候補ヒンジ */
  suspect_hinges?: number[];
  /** 最終姿勢で紙を実際に突き抜けた面IDの組。 */
  self_intersection_pairs?: SelfIntersectionPair[];
  /** 手順から現在の辺IDへ解決した希望角（保存しない導出結果） */
  sequence_targets?: Driver[];
  /** 再生で得た全ヒンジの実角（JSONでは辺IDが文字列キーになる） */
  angles?: Record<string, number>;
  /** 前の希望角を譲った診断（辺ID昇順） */
  relaxations?: AngleRelaxation[];
  closure_rms?: number;
  best_effort?: boolean;
  converged?: boolean;
  /** 紙どうしの接触を検出したか。接触しても要求角まで進む */
  contact_detected?: boolean;
  /** たわみを指定したときだけ入る三角形の網(SIM-012) */
  soft?: SoftMesh | null;
}

/** pose_solve の戻り値(ori3-rigid::SolveResult) */
export interface SolveResult {
  frame: Frame3D;
  converged: boolean;
  /** 今回の平らに畳む操作で知らせる点 */
  flat_fold_violations?: number[];
  /** 全ヒンジの角度(度)。キーは辺ID(JSONでは文字列になる) */
  angles: Record<string, number>;
  /** 実行した反復回数(warm start効果の確認用) */
  iterations: number;
  /** 返した候補の閉包残差RMS */
  closure_rms?: number;
  /** 収束前でも現在指定を守った最良の有限候補を返しているか */
  best_effort?: boolean;
  /** 前の希望角を譲った診断（辺ID昇順） */
  relaxations?: AngleRelaxation[];
  /** 補正後にも残る食い込みの原因候補ヒンジ */
  suspect_hinges?: number[];
  /** 最終姿勢で紙を実際に突き抜けた面IDの組。 */
  self_intersection_pairs?: SelfIntersectionPair[];
  /** 紙どうしの接触を検出したか。接触しても要求角まで進む */
  contact_detected?: boolean;
  /** たわみを指定したときだけ入る三角形の網(SIM-012) */
  soft?: SoftMesh | null;
}

/**
 * 折る手順を持たない一斉表示では、紙の上下を確定できない。
 * 画面がこの事実を黙って隠さないための、fold_all_preview専用wire値。
 */
export type FoldAllLayerOrder = "unavailable_without_sequence";

/** fold_all_previewの戻り値。一時表示だけに使い、Documentへは入れない。 */
export interface FoldAllPreviewOutcome extends SolveResult {
  /** 画面が要求した割合。0..=100。 */
  requested_percent: number;
  /** 山谷の符号を含む今回の希望角。表示や次回seedには使わない。 */
  requested_angles: Driver[];
  /** 次回計算の出発角。requested_anglesではなく、必ずこちらを渡す。 */
  next_warm_seed: Driver[];
  suspect_hinges: number[];
  contact_detected: boolean;
  flat_fold_violations: number[];
  layer_order: FoldAllLayerOrder;
}

/**
 * 完成形における先端の位置(ori3-propose::TipPos2d)。
 * 完成した作品を正面から見たときの2D投影上の相対位置で、奥行きは持たない。
 * 原点(0,0)は胴の中心、xは右が正、yは上が正。範囲は-1.0以上1.0以下で、
 * 1.0は完成形を囲む正方形の中心から辺までの長さ。
 * 将来3Dを足す場合は別の欄として追加し、この2つの欄の意味は変えない。
 */
export interface TipPos2d {
  x: number;
  y: number;
}

/**
 * 選んだ候補について、紙の上で使いたい場所。
 *
 * 完成形の位置(`TipPos2d`)とは別の一時入力で、紙の中心を(0,0)、右と上を正にし、
 * 紙の長辺の半分を1.0として表す。短辺方向の端は縦横比に応じて1.0未満になる。
 * 作品へ保存せず、提案ウィザードを閉じると捨てる。
 */
export interface PaperPosition2d {
  x: number;
  y: number;
}

/** 紙の上の場所1件。leaf_idで完成形の先端と取り違えずに対応させる。 */
export interface PaperTipPosition {
  leaf_id: number;
  position: PaperPosition2d;
}

/**
 * 骨格の節点(ori3-propose::SkeletonNode)。
 * parentがnullの節点が根(胴の中心)で、ちょうど1つだけ置く。
 * lengthは親へつながる辺の長さ(根では使わない)、width_factorは太さ(膨らみ)。
 * tip_pos_2dは完成形での先端の位置。省略でき、省略時は置き場所を提案の計算が決める。
 */
export interface SkeletonNode {
  id: number;
  parent: number | null;
  length: number;
  width_factor: number;
  tip_pos_2d?: TipPos2d | null;
}

/** 骨格全体(ori3-propose::Skeleton) */
export interface Skeleton {
  nodes: SkeletonNode[];
}

/**
 * 先端1本ぶんの紙の上の円(ori3-propose::LeafCircle)。
 * どの先端がどの円になったかを番号で名指しできるようにしたもの。
 */
export interface LeafCircle {
  /** この円を使う先端(骨格の葉)のID */
  leaf_id: number;
  /** 円の番号。同じ候補の中で0から順に振られる */
  circle_index: number;
  /** 紙の上での円の中心 */
  center: Vec2;
  /** 紙の上での円の半径 */
  radius: number;
}

/**
 * 先端の材料になる、展開図の上の点(ori3-propose::LeafVertex)。
 */
export interface LeafVertex {
  /** 展開図の頂点ID */
  id: number;
  /** その頂点の座標 */
  pos: Vec2;
  /** 円の中心とのずれ。0に近いほど置き場所どおり */
  gap: number;
}

/**
 * 先端1本ぶんの、置き場所から展開図までの対応(ori3-propose::LeafSite)。
 * 先端 → 円 → 展開図の材料点 → その先端を囲む分子、がひとつながりになる。
 */
export interface LeafSite {
  /** 配置で決まった、この先端の円 */
  circle: LeafCircle;
  /** 展開図でこの先端の材料になる点。折り線を1本も引けなかったときだけnull */
  vertex: LeafVertex | null;
  /** この先端のまわりを埋めた分子の番号(昇順) */
  molecules: number[];
}

/** 提案された折り方に共通する、確かめ済みの手順(commands.rs::ProposalFoldPlanDetails)。 */
interface ProposalFoldPlanData {
  /** 確かめられた折り手順。先頭から順に折る */
  steps: FoldStep[];
  /** その手順を折り込んだ展開図(折る過程で線の種類が決まる) */
  cp: CreasePattern;
  /** 見つかった手の数 */
  planned: number;
  /** そのうち、通して確かめられた手の数(steps.length と同じ) */
  checked: number;
}

/**
 * 提案された折り方1つ分(commands.rs::ProposalFoldPlan)。
 *
 * 完成まで確認できた手順と途中までの手順は、真偽値ではなく判別できる型で分ける。
 * どちらも `steps` に入るのは通して確かめられた手だけなので、そのまま作品へ入れられる。
 */
export type ProposalFoldPlan =
  | (ProposalFoldPlanData & { status: "checked_to_finish" })
  | (ProposalFoldPlanData & { status: "partial" });

/** proposal_generate が返す展開図の候補1つ分(commands.rs::ProposalCandidate) */
export interface ProposalCandidate {
  cp: CreasePattern;
  /** 骨格の長さ1あたりが紙の何割になるか(大きいほど完成品が大きい) */
  scale: number;
  /** 平坦に折りにくい頂点の数(0が理想。0でなくても使える) */
  violations: number;
  warnings: string[];
  /**
   * 先端1本ずつが、この展開図のどの点・どの分子になったかの対応。
   * 先端1本につきちょうど1件入る。この欄を読まない今までの画面の処理は
   * そのまま動くよう、省略できる形にしてある。
   */
  sites?: LeafSite[];
  /**
   * この展開図の折り方。1手も見つからなかったときはnull。
   * この欄を読まない今までの画面の処理はそのまま動くよう、省略できる形にしてある。
   */
  fold_plan?: ProposalFoldPlan | null;
}

/** 提案計算1件を区別する不透明な値。画面表示には使わない。 */
export type ProposalJobId = string;

/** backendが返す提案計算の閉じた状態。画面にはこの内部名を直接出さない。 */
export type ProposalPhase =
  | "Queued"
  | "Generating"
  | "Verifying"
  | "Finished"
  | "Cancelled"
  | "Failed";

/** proposal_generateのjob別戻り値。要求時と同じjob_idをechoする。 */
export interface ProposalJobResult {
  job_id: ProposalJobId;
  candidates: ProposalCandidate[];
}

/** proposal_progressの1回のlockから得た一貫したsnapshot。 */
export interface ProposalProgressSnapshot {
  job_id: ProposalJobId;
  done: number;
  total: number;
  phase: ProposalPhase;
}

/** proposal_controlへ渡す同種操作の閉じた型。 */
export type ProposalControl = {
  type: "Cancel";
  job_id: ProposalJobId;
};

/** recovery_check の戻り値。前回の異常終了で残った自動保存の情報(SYS-003) */
export interface RecoveryInfo {
  /** 自動保存ファイルの場所 */
  autosave_path: string;
  /** 元の保存先(保存したことがない作品ならnull) */
  document_path: string | null;
  /** 最後に自動保存した時刻(1970年からのミリ秒)。分からなければnull */
  saved_at_ms: number | null;
  /** 復旧する内容を選ぶための番号。画面には表示しない。 */
  candidate_id: number;
  /** 控えた時点の折り手順数。 */
  step_count: number | null;
}

/** recovery_check の戻り値。利用者が選べる前回までの作業と、超過件数。 */
export interface RecoveryChoices {
  choices: RecoveryInfo[];
  overflow_count: number;
}

/** document_export の書き出しの種類(commands.rs::ExportKind)。
 * 展開図の画像はSVG(実寸mm)とPNGの2つ。折り図はPDF(1ファイル)と
 * ページごとのSVG(複数ファイル)の2つ */
export type ExportKind = "CpSvg" | "CpPng" | "DiagramPdf" | "DiagramSvg";

/** 状態接続前からIPC境界で受け付ける、ほかの折り紙ソフト向け形式。 */
export type FoldExportKind = "FoldJson";
export type DocumentExportKind = ExportKind | FoldExportKind;

/** document_export の細かい指定(commands.rs::ExportOptions) */
export interface ExportOptions {
  /** 補助線(下書きの線)も一緒に書き出すか */
  include_aux: boolean;
  /** PNGのときの長いほうの辺の点数 */
  png_long_side: number;
}
