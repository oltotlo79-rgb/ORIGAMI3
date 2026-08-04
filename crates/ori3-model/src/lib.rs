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
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Driver {
    pub hinge: EdgeId,
    pub target_angle_deg: f64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FoldStep {
    pub id: StepId,
    pub kind: TechniqueKind,
    pub drivers: Vec<Driver>,
    /// 平坦到達時の層順序(下→上)。面IDは不安定なので、
    /// 各面を「CP座標系におけるその面の内部代表点」で参照する。
    /// 平坦にならないステップ(Pose)ではNone。
    pub layer_order: Option<Vec<[f64; 2]>>,
    pub note: String,
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
    pub fn new(paper: Paper) -> Document {
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
    PushStep { step: FoldStep },
    InsertStep { index: usize, step: FoldStep },
    RemoveStep { id: StepId },
    UpdateStep { step: FoldStep },
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
