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

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FoldStep {
    pub id: StepId,
    pub kind: TechniqueKind,
    pub drivers: Vec<DriverLine>,
    /// 平坦到達時の層順序(下→上)。面IDは不安定なので、
    /// 各面を「CP座標系におけるその面の内部代表点」で参照する。
    /// 平坦にならないステップ(Pose)ではNone。
    pub layer_order: Option<Vec<[f64; 2]>>,
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

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DisplaySettings {
    pub front_color: [u8; 3],
    pub back_color: [u8; 3],
    pub grid_divisions: u32,
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
            Vertex {
                id: 2,
                pos: [w, h],
            },
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
            display: DisplaySettings {
                front_color: [237, 28, 36],
                back_color: [255, 255, 255],
                grid_divisions: 8,
            },
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
    },
    /// 基本技法(段折り・中割り折り・かぶせ折り)をまとめて折る。
    ///
    /// 座標は [`SeqOp::FoldThrough`] と同じ「畳んだ平面座標」。技法は
    /// 折り操作の合成として実装され(`ori3-layers`)、生成された折り線を
    /// まとめた手順が末尾に足される。
    Technique {
        /// この折りの直前までの手順数(通常は現在の手順数)
        up_to: usize,
        /// 技法の種類。Pleat/InsideReverse/OutsideReverse のみ受け付ける
        kind: TechniqueKind,
        /// 対象フラップ(畳み平面で選んだ層の面ID)。段折りでは空を許す
        flap: Vec<FaceId>,
        /// 折り線(2点。無限直線として扱う)
        line: [[f64; 2]; 2],
        /// 技法ごとに意味の変わる基準点(段折り=2本目の折り線の位置、
        /// 中割り・かぶせ=先端が向かう側)
        reference_point: [f64; 2],
    },
}

/// 3D表示用フレーム(IPC戻り値)
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Face3D {
    pub face: FaceId,
    pub polygon: Vec<[f64; 3]>,
    pub layer: u32,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Frame3D {
    pub faces: Vec<Face3D>,
    pub warnings: Vec<String>,
}
