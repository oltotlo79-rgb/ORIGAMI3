//! ori3-model: 作品データの型定義(紙・展開図・折り手順)と許容誤差定数。

pub mod clock;

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

/// 仕上げ手順に記録する、紙のたわみの再現値(SIM-015)。
///
/// 再生で再計算できる3値だけを保存し、細分数・反復回数・3D頂点は含めない。
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FinishSoftSettings {
    pub enabled: bool,
    pub stiffness: f64,
    pub pressure: f64,
}

impl Default for FinishSoftSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            stiffness: default_stiffness(),
            pressure: 0.0,
        }
    }
}

/// タイムラインへ出す「折り方の名前」。
///
/// [`TechniqueKind`] は再生の権限を持つ折り操作の種別であり、こちらは表示専用の
/// 呼び名である。両者を分けているのは、同じ再生結果でも利用者へ見せる名前だけを
/// 後から足せるようにするためである。
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum DisplayTechniqueKind {
    /// 層操作。手動の汎用操作を選んだときだけ付く。
    LayerOperation,
    Pleat,
    InsideReverse,
    OutsideReverse,
    Squash,
    Petal,
    OpenSink,
    Swivel,
    Twist,
    /// つかんで動かした折り。名前の付いた技法だと言い切れない動き。
    GrabMove,
}

/// [`TechniqueClassification`] の由来。
///
/// あとから自動で名前を付け直すとき、利用者が選んだ [`Self::Explicit`] を
/// 上書きしないために残す。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TechniqueClassificationOrigin {
    /// 折った結果の形から自動で判定した。
    Automatic,
    /// 利用者が折り方を選んだ。
    Explicit,
}

/// 手順1つに載せる、表示用の折り方の名前とその由来。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TechniqueClassification {
    pub kind: DisplayTechniqueKind,
    pub origin: TechniqueClassificationOrigin,
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
    /// この仕上げ位置で確定したたわみの3値。旧作品・通常の折り手順では任意。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_soft: Option<FinishSoftSettings>,
    pub note: String,
    /// タイムラインへ出す折り方の名前と、その由来。
    ///
    /// この項目を持たない旧作品は`None`として読め、`None`は保存でも書き出さない。
    /// 表示は`Some`を優先し、`None`のときだけ従来の[`FoldStep::kind`]へ戻る。
    /// 再生はこの値を一切参照しない。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technique_classification: Option<TechniqueClassification>,
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

/// SIM-011 の「つまんで動かす」操作で、つかんだ面からどの層の束を動かすか。
///
/// デスクトップとブラウザが同じ wire 値を使うため、host 固有の crate ではなく
/// 共有モデルに置く。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sim011LayerSelection {
    /// つかんだ面から連続する見えている束を動かす。
    #[default]
    Flap,
    /// つかみ点に重なる全層を動かす。
    All,
    /// 指定した一枚だけを動かす。
    Single,
}

/// SIM-011 の「つまんで動かす」操作の共有 wire 入力。
///
/// `grab` と `target` は畳んだ平面座標であり、`grab_face` は UI がつかんだ
/// 層を明示する。通信層だけがこの型を定義し、各 host が別々の入力形を持たない。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sim011MoveRequest {
    pub grab: [f64; 2],
    pub target: [f64; 2],
    pub grab_face: FaceId,
    #[serde(default)]
    pub selection: Sim011LayerSelection,
    pub direction: FoldDirection,
}

/// SIM-011 が求め、適用した折り線の共有 wire 出力。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sim011MoveResult {
    pub crease_lines: Vec<[[f64; 2]; 2]>,
    pub selected_layers: Vec<FaceId>,
}

/// 畳んだ形の上へ続けて折る直前に、利用者が指定した折り目の角度。
///
/// 画面上の計算結果ではなく、書類の折り目IDと利用者が指定した符号付き角度だけを
/// 渡す。`+180` と `-180` は別の指定としてそのまま保持する。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FoldPoseDriver {
    pub edge_id: EdgeId,
    pub target_angle_deg: f64,
}

/// [`SeqOp::FoldThrough`] の直前に書類から再現する折った形の指定。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FoldPoseInput {
    pub drivers: Vec<FoldPoseDriver>,
}

/// 新しい折り線の直下で、同時に折れるひだを数えた結果の状態。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoldTargetStatus {
    Ready,
    Limited,
    CreaseOnlyTop,
    Varies,
    Unavailable,
}

/// 一番上の紙が完全に折り重なっていないときの処置。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoldTargetTopAction {
    CreaseOnlyTop,
}

/// 書類と折り手順だけから再計算した、保存を伴わないひだ照会結果。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoldTargetInfo {
    pub status: FoldTargetStatus,
    /// Face数ではなく、上から連続して同時に折れるひだの枚数。
    pub available_count: Option<usize>,
    pub reason: Option<String>,
    pub top_action: Option<FoldTargetTopAction>,
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

/// 重なり防止の既定値。形を変える補正は利用者が明示的に選んだ場合だけ適用する。
fn default_overlap_prevention() -> bool {
    false
}

/// 食い込み検出の既定値。検出と警告は既定で有効だが、検出だけでは形を変えない。
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
    /// 現在の調整値と、手順別の値を持たない旧作品の再生用。仕上げ位置ごとの値は
    /// [`FoldStep::finish_soft`] に記録し、どちらにも頂点の位置そのものは保存しない。
    #[serde(default)]
    pub soft_enabled: bool,
    /// 紙の硬さ(0.0〜1.0)。大きいほど面の中が平らに保たれる。
    #[serde(default = "default_stiffness")]
    pub soft_stiffness: f64,
    /// 膨らみの強さ(0.0〜1.0、SIM-013)。0.0なら膨らませない。
    #[serde(default)]
    pub soft_pressure: f64,
    /// 折り途中の面どうしへ接触補正を掛けるか。**既定はオフ**。
    ///
    /// 利用者が明示的に有効化した場合だけ形を変える。
    /// 補正後の頂点そのものは保存せず、表示を求めるたびに剛体解へ後段適用する。
    #[serde(default = "default_overlap_prevention")]
    pub overlap_prevention_enabled: bool,
    /// 角度を動かした結果、紙どうしが交差したことを検出して警告するか。
    /// **既定はオン**。検出だけでは形を変えない。
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

impl From<&DisplaySettings> for FinishSoftSettings {
    fn from(display: &DisplaySettings) -> Self {
        Self {
            enabled: display.soft_enabled,
            stiffness: display.soft_stiffness,
            pressure: display.soft_pressure,
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

    /// 再生位置までに確定した最新の仕上げたわみを返す。
    ///
    /// `None` は全Pose手順に記録がない旧作品を表す。呼び出し側は従来どおり
    /// `DisplaySettings` 由来の設定を使う。記録を1件以上持つ新形式で、指定位置が
    /// 最初の記録より前なら、たわみを勝手に有効にしない既定値を返す。
    #[must_use]
    pub fn finish_soft_at(&self, up_to: usize, t: f64) -> Option<FinishSoftSettings> {
        let has_recorded_finish = self
            .sequence
            .iter()
            .any(|step| step.kind == TechniqueKind::Pose && step.finish_soft.is_some());
        if !has_recorded_finish {
            return None;
        }

        let up_to = up_to.min(self.sequence.len());
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let completed = if up_to > 0 && t < 1.0 {
            up_to - 1
        } else {
            up_to
        };
        Some(
            self.sequence[..completed]
                .iter()
                .rev()
                .find_map(|step| {
                    (step.kind == TechniqueKind::Pose)
                        .then_some(step.finish_soft)
                        .flatten()
                })
                .unwrap_or_default(),
        )
    }
}

/// 1つの手順が展開図へ**新しく足した**折り線(CP座標の線分)。
///
/// 「先に描いてあった折り線」と「その手順で足された折り線」は、最終展開図と
/// driver線分だけからは区別できない(同じ保存値になる)。区別を推測に頼らず
/// 記録するための来歴で、既にある折り筋で折った手順は `lines` が空になる。
///
/// 手順の並べ替え・削除で対応がずれないよう、配列の位置ではなく手順IDで結び付ける。
/// 線分は後続の折りで分割されるため、辺IDではなく座標で残す(層順序の代表点方式と
/// 同じ思想)。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StepCreases {
    pub step: StepId,
    pub lines: Vec<[[f64; 2]; 2]>,
}

/// `.ori3` ファイルの中身。[`Document`] に、手順ごとの追加折り線の来歴を足した形。
///
/// 来歴を持たない旧形式の作品も読めるよう、`step_creases` は既定で空にする。
/// 空のときは書き出さないので、来歴の無い作品のファイル内容は従来と同じになる。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SavedDocument {
    #[serde(flatten)]
    pub document: Document,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub step_creases: Vec<StepCreases>,
}

impl SavedDocument {
    /// 来歴を持たない作品として包む(新規作品・旧形式の読み込み結果)。
    pub fn new(document: Document) -> SavedDocument {
        SavedDocument {
            document,
            step_creases: Vec::new(),
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
#[derive(Clone, Debug, serde::Serialize)]
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
    MoveStep {
        id: StepId,
        to_index: usize,
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
        /// 上から同時に折るひだの枚数。Face IDは送らず、Rustが書類から再計算する。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_pleat_count: Option<usize>,
        direction: FoldDirection,
        /// 合わせ折りで選んだ点・線。説明文生成用で、折り計算には影響しない。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alignment: Option<FoldAlignment>,
        /// 衝突する縁が1本に定まる場合、誘導折り目を加えて紙を巻き込む。
        /// 古い作品・画面から省略された場合は従来どおり警告だけで折る。
        #[serde(default, skip_serializing_if = "is_false")]
        accept_additional_crease: bool,
        /// この折りの直前に、書類から再現する折った形の利用者指定。
        /// 旧画面から省略された場合は、従来どおり手順だけを再生する。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pose_before: Option<FoldPoseInput>,
    },
    /// 非平坦な形のいちばん上の紙へ、立体を動かさず折り目だけを付ける。
    ///
    /// `material_line` と `material_keep_side_point` は展開図（材料）座標であり、
    /// 畳み平面座標を受け取る [`SeqOp::FoldThrough`] とは座標の意味が異なる。
    CreaseOnlyTop {
        /// この操作の直前までの手順数。
        up_to: usize,
        /// 展開図（材料）座標の折り線。2点を通る無限直線として扱う。
        material_line: [[f64; 2]; 2],
        /// 展開図（材料）座標で、動かさず残す側を示す紙内の点。
        material_keep_side_point: [f64; 2],
        /// 折り目の山谷。折り角を0度にしても説明上の向きは失わない。
        direction: FoldDirection,
        /// この折りの直前に、書類から再現する利用者指定の非平坦な形。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pose_before: Option<FoldPoseInput>,
        /// 合わせ折りで選んだ点・線。説明用だけに保存し、計算には使わない。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alignment: Option<FoldAlignment>,
    },
    /// [`SeqOp::FoldThrough`] を変更せずに調べ、巻き込み用の追加折り目を提案する。
    ///
    /// コマンドの戻り値だけに提案を載せ、展開図・手順・undo履歴は変更しない。
    PreviewFoldThrough {
        up_to: usize,
        line: [[f64; 2]; 2],
        keep_side_point: [f64; 2],
        target_layers: Option<Vec<FaceId>>,
        /// 確定時と同じ、上から同時に折るひだの枚数。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_pleat_count: Option<usize>,
        direction: FoldDirection,
        /// 確定時と同じ直前形状を、書類から再現して非破壊で調べるための指定。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pose_before: Option<FoldPoseInput>,
    },
    /// 新しい折り線の直下で同時に折れるひだを、作品を変更せずに数える。
    PreviewFoldTargets {
        up_to: usize,
        line: [[f64; 2]; 2],
        keep_side_point: [f64; 2],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pose_before: Option<FoldPoseInput>,
    },
    /// 材料座標の新しい折り線について、作品を変更せず折り対象を調べる。
    ///
    /// 3D表示から材料座標へ引き戻した入力専用で、既存の
    /// [`SeqOp::PreviewFoldTargets`] の座標の意味は変更しない。
    PreviewFoldTargetsOnMaterial {
        up_to: usize,
        material_line: [[f64; 2]; 2],
        material_keep_side_point: [f64; 2],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pose_before: Option<FoldPoseInput>,
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

// Serdeの `deny_unknown_fields` はenum全体には指定できてもvariant単位には
// 指定できない。既存操作が受け取る `spatial` envelopeを保つため、MoveStepの
// payloadだけを厳密なstructとして読み、他のvariantは従来どおり余剰fieldを許す。
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveStepFields {
    id: StepId,
    to_index: usize,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type")]
enum SeqOpDeserialize {
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
    MoveStep(MoveStepFields),
    FoldThrough {
        up_to: usize,
        line: [[f64; 2]; 2],
        keep_side_point: [f64; 2],
        target_layers: Option<Vec<FaceId>>,
        #[serde(default)]
        target_pleat_count: Option<usize>,
        direction: FoldDirection,
        #[serde(default)]
        alignment: Option<FoldAlignment>,
        #[serde(default)]
        accept_additional_crease: bool,
        #[serde(default)]
        pose_before: Option<FoldPoseInput>,
    },
    CreaseOnlyTop {
        up_to: usize,
        material_line: [[f64; 2]; 2],
        material_keep_side_point: [f64; 2],
        direction: FoldDirection,
        #[serde(default)]
        pose_before: Option<FoldPoseInput>,
        #[serde(default)]
        alignment: Option<FoldAlignment>,
    },
    PreviewFoldThrough {
        up_to: usize,
        line: [[f64; 2]; 2],
        keep_side_point: [f64; 2],
        target_layers: Option<Vec<FaceId>>,
        #[serde(default)]
        target_pleat_count: Option<usize>,
        direction: FoldDirection,
        #[serde(default)]
        pose_before: Option<FoldPoseInput>,
    },
    PreviewFoldTargets {
        up_to: usize,
        line: [[f64; 2]; 2],
        keep_side_point: [f64; 2],
        #[serde(default)]
        pose_before: Option<FoldPoseInput>,
    },
    PreviewFoldTargetsOnMaterial {
        up_to: usize,
        material_line: [[f64; 2]; 2],
        material_keep_side_point: [f64; 2],
        #[serde(default)]
        pose_before: Option<FoldPoseInput>,
    },
    FlatMotion {
        up_to: usize,
        parts: Vec<MotionPart>,
        kind: TechniqueKind,
    },
    Technique {
        up_to: usize,
        kind: TechniqueKind,
        flap: Vec<FaceId>,
        line: [[f64; 2]; 2],
        reference_point: [f64; 2],
        #[serde(default)]
        open_to_back: Option<bool>,
        #[serde(default)]
        polygon: Option<Vec<[f64; 2]>>,
        #[serde(default)]
        center: Option<[f64; 2]>,
    },
}

impl From<SeqOpDeserialize> for SeqOp {
    fn from(op: SeqOpDeserialize) -> Self {
        match op {
            SeqOpDeserialize::PushStep { step } => Self::PushStep { step },
            SeqOpDeserialize::InsertStep { index, step } => Self::InsertStep { index, step },
            SeqOpDeserialize::RemoveStep { id } => Self::RemoveStep { id },
            SeqOpDeserialize::UpdateStep { step } => Self::UpdateStep { step },
            SeqOpDeserialize::MoveStep(fields) => Self::MoveStep {
                id: fields.id,
                to_index: fields.to_index,
            },
            SeqOpDeserialize::FoldThrough {
                up_to,
                line,
                keep_side_point,
                target_layers,
                target_pleat_count,
                direction,
                alignment,
                accept_additional_crease,
                pose_before,
            } => Self::FoldThrough {
                up_to,
                line,
                keep_side_point,
                target_layers,
                target_pleat_count,
                direction,
                alignment,
                accept_additional_crease,
                pose_before,
            },
            SeqOpDeserialize::CreaseOnlyTop {
                up_to,
                material_line,
                material_keep_side_point,
                direction,
                pose_before,
                alignment,
            } => Self::CreaseOnlyTop {
                up_to,
                material_line,
                material_keep_side_point,
                direction,
                pose_before,
                alignment,
            },
            SeqOpDeserialize::PreviewFoldThrough {
                up_to,
                line,
                keep_side_point,
                target_layers,
                target_pleat_count,
                direction,
                pose_before,
            } => Self::PreviewFoldThrough {
                up_to,
                line,
                keep_side_point,
                target_layers,
                target_pleat_count,
                direction,
                pose_before,
            },
            SeqOpDeserialize::PreviewFoldTargets {
                up_to,
                line,
                keep_side_point,
                pose_before,
            } => Self::PreviewFoldTargets {
                up_to,
                line,
                keep_side_point,
                pose_before,
            },
            SeqOpDeserialize::PreviewFoldTargetsOnMaterial {
                up_to,
                material_line,
                material_keep_side_point,
                pose_before,
            } => Self::PreviewFoldTargetsOnMaterial {
                up_to,
                material_line,
                material_keep_side_point,
                pose_before,
            },
            SeqOpDeserialize::FlatMotion { up_to, parts, kind } => {
                Self::FlatMotion { up_to, parts, kind }
            }
            SeqOpDeserialize::Technique {
                up_to,
                kind,
                flap,
                line,
                reference_point,
                open_to_back,
                polygon,
                center,
            } => Self::Technique {
                up_to,
                kind,
                flap,
                line,
                reference_point,
                open_to_back,
                polygon,
                center,
            },
        }
    }
}

impl<'de> serde::Deserialize<'de> for SeqOp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        SeqOpDeserialize::deserialize(deserializer).map(Into::into)
    }
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

#[cfg(test)]
mod technique_classification_tests {
    use super::{
        DisplayTechniqueKind, FoldStep, TechniqueClassification, TechniqueClassificationOrigin,
        TechniqueKind,
    };

    const ALL_DISPLAY_KINDS: [DisplayTechniqueKind; 10] = [
        DisplayTechniqueKind::LayerOperation,
        DisplayTechniqueKind::Pleat,
        DisplayTechniqueKind::InsideReverse,
        DisplayTechniqueKind::OutsideReverse,
        DisplayTechniqueKind::Squash,
        DisplayTechniqueKind::Petal,
        DisplayTechniqueKind::OpenSink,
        DisplayTechniqueKind::Swivel,
        DisplayTechniqueKind::Twist,
        DisplayTechniqueKind::GrabMove,
    ];

    fn step_without_a_display_name() -> FoldStep {
        FoldStep {
            id: 3,
            kind: TechniqueKind::Simple,
            drivers: Vec::new(),
            layer_order: None,
            alignment: None,
            finish_soft: None,
            note: "説明".to_owned(),
            technique_classification: None,
        }
    }

    /// 表示名の項目を持たない旧作品は`None`として読め、書き出しにも項目が出ない。
    #[test]
    fn an_old_step_without_the_field_reads_as_none_and_is_not_written_back() {
        let text = r#"{"id":3,"kind":"Simple","drivers":[],"layer_order":null,"note":"説明"}"#;
        let step: FoldStep = serde_json::from_str(text).expect("旧形式の手順を読める");
        assert_eq!(step.technique_classification, None);
        assert_eq!(step.kind, TechniqueKind::Simple, "既存の折り方は変えない");
        assert_eq!(step.note, "説明", "利用者の説明文は変えない");

        let written = serde_json::to_string(&step).expect("手順を書き出せる");
        assert!(
            !written.contains("technique_classification"),
            "`None`は保存に出さない: {written}"
        );
        let reread: FoldStep = serde_json::from_str(&written).expect("書き出した手順を読み直せる");
        assert_eq!(reread.technique_classification, None);
    }

    /// 付けた表示名と由来は、保存して読み直しても同じ値のまま残る。
    #[test]
    fn every_display_name_and_origin_survives_a_save_and_open_round_trip() {
        for kind in ALL_DISPLAY_KINDS {
            for origin in [
                TechniqueClassificationOrigin::Automatic,
                TechniqueClassificationOrigin::Explicit,
            ] {
                let expected = TechniqueClassification { kind, origin };
                let mut step = step_without_a_display_name();
                step.technique_classification = Some(expected);

                let written = serde_json::to_string(&step).expect("手順を書き出せる");
                assert!(
                    written.contains("technique_classification"),
                    "`Some`は保存に出す: {written}"
                );
                let reread: FoldStep =
                    serde_json::from_str(&written).expect("書き出した手順を読み直せる");
                assert_eq!(reread.technique_classification, Some(expected));
                assert_eq!(reread.kind, step.kind, "表示名は折り方を書き換えない");
                assert_eq!(reread.note, step.note, "表示名は説明文を書き換えない");
            }
        }
    }
}
