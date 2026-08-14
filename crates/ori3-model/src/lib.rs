//! ori3-model: 作品データの型定義(紙・展開図・折り手順)と許容誤差定数。

pub const SCHEMA_VERSION: u32 = 1;
/// 幾何計算の許容誤差。座標は「紙の長辺 = 1.0」に正規化した系で扱い、
/// mm値は入出力時のみ使用する。
pub const EPS: f64 = 1e-9;

pub type VertexId = u32;
pub type EdgeId = u32;
pub type FaceId = u32; // 面IDは面抽出のたびに再採番される導出値。永続化に使わない
pub type StepId = u32;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Paper {
    pub width_mm: f64,
    pub height_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EdgeKind {
    Border,
    Mountain,
    Valley,
    Aux,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Vertex {
    pub id: VertexId,
    pub pos: [f64; 2],
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    pub v0: VertexId,
    pub v1: VertexId,
    pub kind: EdgeKind,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CreasePattern {
    pub vertices: Vec<Vertex>,
    pub edges: Vec<Edge>,
    pub next_vertex_id: VertexId,
    pub next_edge_id: EdgeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TechniqueKind {
    Simple,
    Pleat,
    InsideReverse,
    OutsideReverse,
    Petal,
    Squash,
    OpenSink,
    Swivel,
    Twist,
    Pose,
}

/// ヒンジ角: 0=平ら, +180=完全な山折り, -180=完全な谷折り(度)
///
/// 注: EdgeId参照のDriverは pose_solve(スライダー操作)専用の一時指定。
/// 手順の永続化には使わない(辺IDは後続の折りの分割で無効化されるため)。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Driver {
    pub hinge: EdgeId,
    pub target_angle_deg: f64,
}

/// 手順永続化用のdriver: 折り線をCP座標の線分で指定する。
/// 再生時は「この線分上に乗る折り辺すべて」(同一直線上・区間内・EPS許容)を
/// 対象角へ駆動する(ori3-layersの`resolve_driver_edges`で解決)。後続の折りで
/// 辺が分割されても全断片が駆動されるためID無効化に耐える
/// (層順序の代表点方式と同じ思想)。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DriverLine {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub target_angle_deg: f64,
}

/// 「合わせて折る」で選んだ基準の種類。
///
/// camelCase はデスクトップ側の `AlignMode` と同じ表記にしている。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AlignmentMode {
    ThroughTwoPoints,
    PointPoint,
    LineLine,
    PointPerpendicularLine,
    PointLineThrough,
    PointToLinePointToLine,
    PointLinePerpendicular,
    ExistingLine,
}

/// 合わせ折りの説明文を再現するため、選んだ点・線を畳み平面座標で保存する。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AlignmentTarget {
    Point { p: [f64; 2] },
    Line { a: [f64; 2], b: [f64; 2] },
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FoldAlignment {
    pub mode: AlignmentMode,
    pub picks: Vec<AlignmentTarget>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FoldStep {
    pub id: StepId,
    pub kind: TechniqueKind,
    pub drivers: Vec<DriverLine>,
    /// 平坦到達時の層順序(下→上)。面IDは不安定なので、
    /// 各面を「CP座標系におけるその面の内部代表点」で参照する。
    /// 平坦にならないステップ(Pose)ではNone。
    pub layer_order: Option<Vec<[f64; 2]>>,
    /// 合わせ折りで選んだ点・線。旧形式の作品では存在しないため任意。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alignment: Option<FoldAlignment>,
    pub note: String,
}

/// 折る向き(畳んだ状態の上に折り線を引いてまとめて折るときの向き)。
///
/// 折り操作の実装は `ori3-layers` にあるが、手順操作([`SeqOp::FoldThrough`])の
/// 引数として画面から送られてくるため、serde対応の型定義はここに置く。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FoldDirection {
    /// 動く側の層を反転して山の一番上に載せる(紙の表から見て谷折りに相当)。
    Up,
    /// 動く側の層を反転して山の一番下に入れる(山折りに相当)。
    Down,
}

/// 畳み平面上の半平面。`inside_point` がある側を操作対象にする。
///
/// 汎用層操作([`SeqOp::FlatMotion`])のIPC入力用で、境界線は必要に応じて
/// 新しい折り線になる。座標は「畳んだ平面座標」。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HalfPlane {
    pub line: [[f64; 2]; 2],
    pub inside_point: [f64; 2],
}

/// 汎用層操作で紙へ施す平面等長変換。
///
/// UIから必要になる基本形だけを永続コマンド型に公開する。任意の等長変換は
/// 鏡映の列で表せる(1本なら折り返し、2本なら回転)。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MotionTransform {
    /// 配置を動かさず、重なり順や山谷だけを変更する。
    Stay,
    /// 指定した直線での鏡映を先頭から順に適用する。
    Reflect(Vec<[[f64; 2]; 2]>),
}

/// 動かした紙を重なりのどこへ入れるか。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LayerTurn {
    /// 現在の重なり順を保つ。
    Keep,
    /// 重なり全体の外側へ回す。
    Outside(FoldDirection),
    /// 分かれた元の紙のすぐ内側へ差し込む。
    Inside(FoldDirection),
    /// 指定面のすぐ隣へ差し込む。
    Beside {
        anchor: FaceId,
        direction: FoldDirection,
    },
}

/// 汎用層操作のうち、同じ変換と重ね方で動く一部分。
///
/// `layers` の面IDは操作開始時(`up_to`)の導出面を指す一時入力であり、作品には
/// 保存しない。実行結果は安定な座標参照を持つ [`FoldStep`] へ変換して保存する。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MotionPart {
    /// 対象面。空なら全ての面。
    pub layers: Vec<FaceId>,
    /// 半平面の共通部分。空なら対象面の全域。
    pub region: Vec<HalfPlane>,
    pub transform: MotionTransform,
    pub turn: LayerTurn,
    /// 対象部分内の層順を反転するか。`None`なら変換から自動決定する。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse_layers: Option<bool>,
}

/// 方眼の分割数の下限・上限(CPE-003)。範囲外の指定は丸めて警告する
/// (「止めずに警告」原則)。色は `u8` なので0〜255は型が保証する。
pub const MIN_GRID_DIVISIONS: u32 = 2;
pub const MAX_GRID_DIVISIONS: u32 = 1024;

/// 紙の硬さの既定値(SIM-012)。古い作品ファイルにはこの項目が無いので既定で読む。
fn default_stiffness() -> f64 {
    0.5
}

/// 重なり防止の既定値。古い作品には項目が無いため、明示的にオンで補う。
fn default_overlap_prevention() -> bool {
    true
}

/// 食い込み防止の既定値。古い作品には項目が無いため、明示的にオンで補う。
fn default_penetration_prevention() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DisplaySettings {
    pub front_color: [u8; 3],
    pub back_color: [u8; 3],
    pub grid_divisions: u32,
    /// 紙のたわみを表現するか(SIM-012)。**既定はオフ**。
    ///
    /// たわみは「パラメータだけを残して、頂点の位置そのものは保存しない」決まり
    /// (SIM-015)なので、作品ごとの見た目の設定としてここに置く。
    #[serde(default)]
    pub soft_enabled: bool,
    /// 紙の硬さ(0.0〜1.0)。大きいほど面の中が平らに保たれる。
    #[serde(default = "default_stiffness")]
    pub soft_stiffness: f64,
    /// 膨らみの強さ(0.0〜1.0、SIM-013)。0.0なら膨らませない。
    #[serde(default)]
    pub soft_pressure: f64,
    /// 折り途中の面どうしへ接触補正を掛けるか。**既定はオン**。
    ///
    /// 補正後の頂点そのものは保存せず、表示を求めるたびに剛体解へ後段適用する。
    #[serde(default = "default_overlap_prevention")]
    pub overlap_prevention_enabled: bool,
    /// 角度を動かす途中で紙どうしが交差したとき、ぶつかる直前で止めるか。
    /// **既定はオン**。複雑な形では高速な簡易判定が見逃すことがある。
    #[serde(default = "default_penetration_prevention")]
    pub penetration_prevention_enabled: bool,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        DisplaySettings {
            front_color: [237, 28, 36],
            back_color: [255, 255, 255],
            grid_divisions: 8,
            soft_enabled: false,
            soft_stiffness: default_stiffness(),
            soft_pressure: 0.0,
            overlap_prevention_enabled: default_overlap_prevention(),
            penetration_prevention_enabled: default_penetration_prevention(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Document {
    pub schema_version: u32,
    pub paper: Paper,
    pub cp: CreasePattern,
    pub sequence: Vec<FoldStep>,
    pub display: DisplaySettings,
}

impl Document {
    /// 紙サイズから初期ドキュメントを作る。CPには輪郭4辺のみが入る。
    ///
    /// 座標系は「紙の長辺 = 1.0」に正規化する
    /// (例: 150×100mm なら 幅1.0×高さ2/3、正方形なら1.0×1.0)。
    /// 頂点は左下(0,0)起点の反時計回り、辺は4本すべて `EdgeKind::Border`。
    ///
    /// # Panics
    ///
    /// 紙の幅・高さが正の値でない場合はパニックする。
    pub fn new(paper: Paper) -> Document {
        assert!(
            paper.width_mm > 0.0 && paper.height_mm > 0.0,
            "紙のサイズは正の値でなければならない: width_mm={}, height_mm={}",
            paper.width_mm,
            paper.height_mm
        );
        let long_edge = paper.width_mm.max(paper.height_mm);
        let w = paper.width_mm / long_edge;
        let h = paper.height_mm / long_edge;
        let vertices = vec![
            Vertex {
                id: 0,
                pos: [0.0, 0.0],
            },
            Vertex {
                id: 1,
                pos: [w, 0.0],
            },
            Vertex { id: 2, pos: [w, h] },
            Vertex {
                id: 3,
                pos: [0.0, h],
            },
        ];
        let edges = (0..4)
            .map(|i| Edge {
                id: i,
                v0: i,
                v1: (i + 1) % 4,
                kind: EdgeKind::Border,
            })
            .collect();
        Document {
            schema_version: SCHEMA_VERSION,
            paper,
            cp: CreasePattern {
                vertices,
                edges,
                next_vertex_id: 4,
                next_edge_id: 4,
            },
            sequence: Vec::new(),
            display: DisplaySettings::default(),
        }
    }
}

/// edit_apply コマンドの操作enum(これ以外の編集操作を追加しない)
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum EditOp {
    AddSegment {
        a: [f64; 2],
        b: [f64; 2],
        kind: EdgeKind,
    },
    RemoveEdges {
        ids: Vec<EdgeId>,
    },
    SetEdgeKind {
        ids: Vec<EdgeId>,
        kind: EdgeKind,
    },
    MoveVertex {
        id: VertexId,
        to: [f64; 2],
    },
    SetPaper {
        paper: Paper,
    },
    /// 提案ウィザードの流し込み用
    ReplaceCreasePattern {
        cp: CreasePattern,
    },
    /// 紙の色と方眼の分割数を変える(PAP-003 / CPE-003)。
    /// 作品ごとの設定として `Document::display` に保存され、undo/redoの対象になる。
    /// `grid_divisions` が [`MIN_GRID_DIVISIONS`]〜[`MAX_GRID_DIVISIONS`] の外なら
    /// 丸めて警告する(止めない)。
    SetDisplay {
        display: DisplaySettings,
    },
}

/// sequence_apply コマンドの操作enum
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum SeqOp {
    PushStep {
        step: FoldStep,
    },
    InsertStep {
        index: usize,
        step: FoldStep,
    },
    RemoveStep {
        id: StepId,
    },
    UpdateStep {
        step: FoldStep,
    },
    /// 畳んだ状態の上に折り線を引いてまとめて折る(3D画面・展開図画面のどちらからも使う)。
    ///
    /// 座標は全て「畳んだ平面座標」(手順を `up_to` まで再生した3D表示の座標系)。
    /// 折り線はCPへ引き戻して追記され、生成された手順が末尾に足される。
    FoldThrough {
        /// この折りの直前までの手順数(通常は現在の手順数)
        up_to: usize,
        /// 折り線(2点。無限直線として扱う)
        line: [[f64; 2]; 2],
        /// 動かさない側を示す点
        keep_side_point: [f64; 2],
        /// 折る対象の層。None = 折り線の可動側に掛かる全ての層
        target_layers: Option<Vec<FaceId>>,
        direction: FoldDirection,
        /// 合わせ折りで選んだ点・線。説明文生成用で、折り計算には影響しない。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alignment: Option<FoldAlignment>,
        /// 衝突する縁が1本に定まる場合、誘導折り目を加えて紙を巻き込む。
        /// 古い作品・画面から省略された場合は従来どおり警告だけで折る。
        #[serde(default, skip_serializing_if = "is_false")]
        accept_additional_crease: bool,
    },
    /// [`SeqOp::FoldThrough`] を変更せずに調べ、巻き込み用の追加折り目を提案する。
    ///
    /// コマンドの戻り値だけに提案を載せ、展開図・手順・undo履歴は変更しない。
    PreviewFoldThrough {
        up_to: usize,
        line: [[f64; 2]; 2],
        keep_side_point: [f64; 2],
        target_layers: Option<Vec<FaceId>>,
        direction: FoldDirection,
    },
    /// 平坦状態から別の平坦状態へ、複数部分を同時に動かす汎用層操作。
    ///
    /// 開く・重ね替え・選択領域だけの山谷反転・複数ヒンジの同時移動を、
    /// 名前付き技法を追加せずに表現する。結果は通常の [`FoldStep`] として記録される。
    FlatMotion {
        /// この操作の直前までの手順数。
        up_to: usize,
        /// 先に置いた部分を優先する。
        parts: Vec<MotionPart>,
        /// 手順一覧に記録する技法種別。
        kind: TechniqueKind,
    },
    /// 基本技法(段折り・中割り折り・かぶせ折り・開いてつぶす)をまとめて折る。
    ///
    /// 座標は [`SeqOp::FoldThrough`] と同じ「畳んだ平面座標」。技法は
    /// 折り操作の合成として実装され(`ori3-layers`)、生成された折り線を
    /// まとめた手順が末尾に足される。
    Technique {
        /// この折りの直前までの手順数(通常は現在の手順数)
        up_to: usize,
        /// 技法の種類。Pleat/InsideReverse/OutsideReverse/Squash/Petal/OpenSink/
        /// Swivel/Twist のみ受け付ける
        kind: TechniqueKind,
        /// 対象フラップ(畳み平面で選んだ層の面ID)。段折りでは空を許す
        flap: Vec<FaceId>,
        /// 折り線(2点。無限直線として扱う)
        line: [[f64; 2]; 2],
        /// 技法ごとに意味の変わる基準点(段折り=2本目の折り線の位置、
        /// 中割り・かぶせ=先端が向かう側、開いてつぶす=つぶす方向)
        reference_point: [f64; 2],
        /// 開いてつぶす折りで、つぶした紙を向こう側(重なりのいちばん下)へ開くか。
        /// 省略時は手前へ開く。実際の紙ではどちらへも開けるため選べるようにしてある
        #[serde(default)]
        open_to_back: Option<bool>,
        /// ねじり折りの中央多角形(畳み平面の頂点を順に並べる)。省略時は
        /// `line` を1辺として中心のまわりに回した正多角形を使う。辺ごとに
        /// 長さの違う多角形は線1本では指せないので、この項目で直接渡す
        #[serde(default)]
        polygon: Option<Vec<[f64; 2]>>,
        /// ねじり折りの中心。省略時は選んだ層の重心
        #[serde(default)]
        center: Option<[f64; 2]>,
    },
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// 3D表示用フレーム(IPC戻り値)
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Face3D {
    pub face: FaceId,
    pub polygon: Vec<[f64; 3]>,
    pub layer: u32,
    /// 同一深度の面を決める、紙の下から上への重なり順位。
    #[serde(default)]
    pub surface_rank: u32,
    /// 平坦折りで面の材質座標が鏡映されているか。
    ///
    /// 表示用の導出値であり、古い検証用soft geometry snapshotにも読み込めるよう
    /// 未指定時は表向き(false)として扱う。
    #[serde(default)]
    pub mirrored: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Frame3D {
    pub faces: Vec<Face3D>,
    pub warnings: Vec<String>,
}
