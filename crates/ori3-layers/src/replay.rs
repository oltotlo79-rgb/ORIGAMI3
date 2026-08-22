//! 手順の再生: 展開図(CP)と手順列だけから立体の姿勢を求め直す。
//!
//! 3D状態は一切保存しない(SEQ-004「展開図を編集したら手順を自動で再生し直す」の
//! ための設計)。各ステップの [`DriverLine`](ori3_model::DriverLine) は
//! 「CP座標の線分+角度」なので、後続の折りや編集で辺IDが変わっても
//! [`resolve_driver_edges`] で現在の辺へ解決できる。
//!
//! # 求め方(折り上がり `t=1`)
//!
//! 折り畳んだ状態の上に次の折りを重ねるのではなく、平らな展開図に手順を解決して
//! 1回で解く。同じ辺を複数のステップが指定する場合は後のステップが勝つ。
//! 表示solveでは、現在の非Pose手順で実際に変えた角度だけをhard、過去手順とPoseを
//! preferred、未指定ヒンジをfreeにする。過去の希望が現在操作を妨げず、未指定角を
//! 0°へ引く人工的な抵抗も生じない。
//!
//! 後続の平坦操作が使う[`FlatState`]だけは表示とは分け、従来どおり全保存角をexact、
//! 未指定ヒンジを0°として[`ori3_rigid::propagate`]する。表示の自然追従によって
//! fold-throughの組合せ的な意味を変えないためである。
//!
//! # 折っている最中(`0 < t < 1`)
//!
//! **角度を線形補間した値をそのまま全ヒンジに固定してはいけない。** 内部頂点の
//! まわりにはループ閉包の拘束があり(自由度1の四折り頂点で2本以上を勝手な値に
//! すると閉じない)、破ると**面どうしが離れて紙がちぎれて見える**。
//!
//! 単一Simple DriverLineは利用者が直接指定した現在角としてhardにする。一方、
//! `flat_motion`由来の複数DriverLineは独立な入力角ではなく、完了形で変化した全角の
//! スナップショットである。この一様補間は一般に剛体経路ではないため、途中だけは
//! [`ori3_rigid::solve_near`] の共同path targetとして閉じた経路へ射影する。さらに
//! 一発で `t` まで飛ばすと対称な解のあいだで解が飛び移って紙が瞬間移動するため、目標を
//! [`SUBSTEPS`] 等分して少しずつ動かし、前の解を次の初期値にする(連続法)。
//! 分割点は `t` だけで決まるので結果は決定的(SYS-004)。
//! `t=0` は「直前の手順を折り終えた状態」として `t=1` と同じ厳密な道で解く。
//!
//! # 見つからない折り線の扱い(SEQ-004)
//!
//! - DriverLineが1本も辺に解決できないステップは飛ばし、`skipped` に載せて警告する。
//!   飛ばしたステップの層順序も使わない(直前の層順序を保つ)
//! - 一部だけ解決できた場合は解決できた分で続行し、警告だけ出す
//! - 折り線を持たないステップ(Pose等)は「見つからない」ではないので飛ばさない
//! - 層順序の代表点が1点も現在の面に解決できないときも直前の層順序を保つ
//! - 姿勢が求まらない(閉じない)場合も止めず、最良解を返して警告に載せる
//!
//! # 既知の制限
//!
//! 一部の層だけを折る手順(`fold_through` の `target_layers` 指定)は、平らな1枚の
//! 剛体折りとしては成立しない(折り線の端が紙の縁でも既存の折り線の交点でもない
//! 位置で終わるため、±180°まで折ると閉じない)。再生すると収束せず、いちばん近い形と
//! 警告を返す。
//!
//! つぶし折り・花弁折り・中割り折りのように**既にある折り目を開いて折り直す**手順は、
//! 折り角の線形補間が剛体折りの道筋と一致しない(道の途中に分岐点がある)。紙は
//! つながったままだが、折っている最中の姿勢が分岐点の前後で切り替わることがある。
//! 折り上がり(`t=1`)の形は正しい。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_geometry::Isometry2;
use ori3_model::{
    AlignmentTarget, CreasePattern, DisplaySettings, Document, Driver, DriverLine, Edge, EdgeId,
    FaceId, FinishSoftSettings, FoldAlignment, FoldStep, Frame3D, Paper, StepId, TechniqueKind,
    Vertex,
};

use crate::flat_state::FlatState;
use crate::fold_through::resolve_driver_edges;

/// 平坦判定の許容誤差。ソルバーの表示精度(座標誤差 1e-6 程度)に合わせる。
/// [`ori3_model::EPS`](1e-9)では厳しすぎて、正しく畳めた状態を弾いてしまう。
const FLAT_EPS: f64 = 1e-6;

/// 折り途中の接触補正に使う層順序の契約。
///
/// 表示用の [`Frame3D::faces`] は従来どおり、途中では `start`、完了時は `end`
/// の層番号を持つ。接触補正だけは両方と進行度を受け取り、上下が切り替わる折りでも
/// 突然向きが反転しないようにできる。
#[derive(Clone, Debug, PartialEq)]
pub struct LayerTransition {
    /// この手順に入る直前の層順序(下→上)。
    pub start: Vec<FaceId>,
    /// この手順を完了したときの層順序(下→上)。
    pub end: Vec<FaceId>,
    /// 現在の進行度(0.0〜1.0)。
    pub progress: f64,
    /// `start` / `end` の少なくとも一方が、解決済みの保存 `layer_order` を
    /// 経路に含む物理順か。正当な順がFaceId昇順でもfallbackと区別する。
    pub order_is_authoritative: bool,
}

/// [`replay`] の結果。
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReplayResult {
    /// 3D表示用フレーム(`Face3D.layer` は下から0,1,2…)
    pub frame: Frame3D,
    /// 折り線が見つからず飛ばしたステップのID
    pub skipped: Vec<StepId>,
    /// 再生中に出た警告(日本語)
    pub warnings: Vec<String>,
    /// 補正後にも残る食い込みの原因候補ヒンジ。コマンド層で判定して設定する。
    pub suspect_hinges: Vec<EdgeId>,
    /// 手順から実際に角度指定されたヒンジ。候補の優先順位付け専用。
    #[serde(skip)]
    pub driver_hinges: Vec<EdgeId>,
    /// 剛体ソルバーが返した最終姿勢の全ヒンジ角(度)。
    /// Poseの次の操作を同じ閉包解から続けるための内部状態で、IPCへは出さない。
    #[serde(skip)]
    pub hinge_angles: HashMap<EdgeId, f64>,
    /// completeなcanonical surface導出が証明した、正面積重なり対だけの上下。
    /// 次手順の幾何導出専用で、保存layerからは作らない。
    #[serde(skip)]
    pub surface_order_provenance: Option<ori3_rigid::SurfaceOrderProvenance>,
    /// 表示位置までに保存手順から解決できた明示角（辺ID昇順）。
    /// 未指定ヒンジを平らにするための0°は含めない。
    #[serde(skip)]
    pub sequence_targets: Vec<Driver>,
    /// 過去手順またはPoseの希望角を譲った診断（辺ID昇順）。
    #[serde(skip)]
    pub relaxations: Vec<ori3_rigid::AngleRelaxation>,
    /// 表示解の閉包残差RMS。
    #[serde(skip)]
    pub closure_rms: f64,
    /// 閉包収束前でも、現在手順を守った有限候補を表示しているか。
    #[serde(skip)]
    pub best_effort: bool,
    /// 表示解が閉包収束したか。
    #[serde(skip)]
    pub converged: bool,
    /// 接触補正専用の開始・完了層順序。IPCへは出さず、コマンド層でだけ使う。
    #[serde(skip)]
    pub layer_transition: LayerTransition,
}

/// 完了endpointだけを保持する、最後の再生入力1件分のキャッシュ。
///
/// 3D状態を作品へ保存するものではない。同じ展開図・手順・抽出面を連続して表示するとき、
/// `t < 1` の履歴復元が要求する同じ `t=1` 結果を数値的に解き直さないためだけに使う。
struct ReplayEndpointCache {
    state: Mutex<Option<ReplayEndpointCacheState>>,
    #[cfg(test)]
    lookups: AtomicUsize,
    #[cfg(test)]
    hits: AtomicUsize,
    #[cfg(test)]
    stores: AtomicUsize,
    #[cfg(test)]
    computed_bodies: AtomicUsize,
}

struct ReplayEndpointCacheState {
    input: ReplayInputSnapshot,
    endpoints: Vec<Option<Arc<ReplayResult>>>,
}

struct ReplayInputSnapshot {
    document: Document,
    faces: Vec<Face>,
}

impl ReplayInputSnapshot {
    fn new(document: &Document, faces: &[Face]) -> Self {
        Self {
            document: document.clone(),
            faces: faces.to_vec(),
        }
    }

    fn matches(&self, document: &Document, faces: &[Face]) -> bool {
        same_document_bits(&self.document, document) && same_faces(&self.faces, faces)
    }
}

impl ReplayEndpointCache {
    fn new() -> Self {
        Self {
            state: Mutex::new(None),
            #[cfg(test)]
            lookups: AtomicUsize::new(0),
            #[cfg(test)]
            hits: AtomicUsize::new(0),
            #[cfg(test)]
            stores: AtomicUsize::new(0),
            #[cfg(test)]
            computed_bodies: AtomicUsize::new(0),
        }
    }

    fn lookup(&self, document: &Document, faces: &[Face], up_to: usize) -> Option<ReplayResult> {
        #[cfg(test)]
        self.lookups.fetch_add(1, Ordering::Relaxed);
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cached = state
            .as_ref()
            .filter(|state| state.input.matches(document, faces))?
            .endpoints
            .get(up_to)?
            .as_ref()?
            .clone();
        #[cfg(test)]
        self.hits.fetch_add(1, Ordering::Relaxed);
        drop(state);
        Some((*cached).clone())
    }

    fn store(&self, document: &Document, faces: &[Face], up_to: usize, result: &ReplayResult) {
        #[cfg(test)]
        self.stores.fetch_add(1, Ordering::Relaxed);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state
            .as_ref()
            .is_some_and(|state| state.input.matches(document, faces))
        {
            *state = Some(ReplayEndpointCacheState {
                input: ReplayInputSnapshot::new(document, faces),
                endpoints: vec![None; document.sequence.len() + 1],
            });
        }
        let state = state.as_mut().expect("直前に再生cacheを初期化した");
        if up_to >= state.endpoints.len() {
            state.endpoints.resize(up_to + 1, None);
        }
        state.endpoints[up_to] = Some(Arc::new(result.clone()));
    }

    fn note_compute(&self) {
        #[cfg(test)]
        self.computed_bodies.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(test)]
    fn clear(&self) {
        *self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.lookups.store(0, Ordering::Relaxed);
        self.hits.store(0, Ordering::Relaxed);
        self.stores.store(0, Ordering::Relaxed);
        self.computed_bodies.store(0, Ordering::Relaxed);
    }

    #[cfg(test)]
    fn stats(&self) -> ReplayEndpointCacheStats {
        ReplayEndpointCacheStats {
            lookups: self.lookups.load(Ordering::Relaxed),
            hits: self.hits.load(Ordering::Relaxed),
            stores: self.stores.load(Ordering::Relaxed),
            computed_bodies: self.computed_bodies.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
struct ReplayEndpointCacheStats {
    lookups: usize,
    hits: usize,
    stores: usize,
    computed_bodies: usize,
}

fn replay_endpoint_cache() -> &'static ReplayEndpointCache {
    static CACHE: OnceLock<ReplayEndpointCache> = OnceLock::new();
    CACHE.get_or_init(ReplayEndpointCache::new)
}

fn same_f64_bits(left: f64, right: f64) -> bool {
    left.to_bits() == right.to_bits()
}

fn same_face(left: &Face, right: &Face) -> bool {
    let Face {
        id: left_id,
        vertices: left_vertices,
        edges: left_edges,
    } = left;
    let Face {
        id: right_id,
        vertices: right_vertices,
        edges: right_edges,
    } = right;
    left_id == right_id && left_vertices == right_vertices && left_edges == right_edges
}

fn same_faces(left: &[Face], right: &[Face]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| same_face(left, right))
}

fn same_point2_bits(left: &[f64; 2], right: &[f64; 2]) -> bool {
    same_f64_bits(left[0], right[0]) && same_f64_bits(left[1], right[1])
}

fn same_paper_bits(left: &Paper, right: &Paper) -> bool {
    let Paper {
        width_mm: left_width,
        height_mm: left_height,
    } = left;
    let Paper {
        width_mm: right_width,
        height_mm: right_height,
    } = right;
    same_f64_bits(*left_width, *right_width) && same_f64_bits(*left_height, *right_height)
}

fn same_vertex_bits(left: &Vertex, right: &Vertex) -> bool {
    let Vertex {
        id: left_id,
        pos: left_pos,
    } = left;
    let Vertex {
        id: right_id,
        pos: right_pos,
    } = right;
    left_id == right_id && same_point2_bits(left_pos, right_pos)
}

fn same_edge(left: &Edge, right: &Edge) -> bool {
    let Edge {
        id: left_id,
        v0: left_v0,
        v1: left_v1,
        kind: left_kind,
    } = left;
    let Edge {
        id: right_id,
        v0: right_v0,
        v1: right_v1,
        kind: right_kind,
    } = right;
    left_id == right_id && left_v0 == right_v0 && left_v1 == right_v1 && left_kind == right_kind
}

fn same_crease_pattern_bits(left: &CreasePattern, right: &CreasePattern) -> bool {
    let CreasePattern {
        vertices: left_vertices,
        edges: left_edges,
        next_vertex_id: left_next_vertex,
        next_edge_id: left_next_edge,
    } = left;
    let CreasePattern {
        vertices: right_vertices,
        edges: right_edges,
        next_vertex_id: right_next_vertex,
        next_edge_id: right_next_edge,
    } = right;
    left_vertices.len() == right_vertices.len()
        && left_vertices
            .iter()
            .zip(right_vertices)
            .all(|(left, right)| same_vertex_bits(left, right))
        && left_edges.len() == right_edges.len()
        && left_edges
            .iter()
            .zip(right_edges)
            .all(|(left, right)| same_edge(left, right))
        && left_next_vertex == right_next_vertex
        && left_next_edge == right_next_edge
}

fn same_driver_line_bits(left: &DriverLine, right: &DriverLine) -> bool {
    let DriverLine {
        a: left_a,
        b: left_b,
        target_angle_deg: left_target,
    } = left;
    let DriverLine {
        a: right_a,
        b: right_b,
        target_angle_deg: right_target,
    } = right;
    same_point2_bits(left_a, right_a)
        && same_point2_bits(left_b, right_b)
        && same_f64_bits(*left_target, *right_target)
}

fn same_alignment_target_bits(left: &AlignmentTarget, right: &AlignmentTarget) -> bool {
    match (left, right) {
        (AlignmentTarget::Point { p: left }, AlignmentTarget::Point { p: right }) => {
            same_point2_bits(left, right)
        }
        (
            AlignmentTarget::Line {
                a: left_a,
                b: left_b,
            },
            AlignmentTarget::Line {
                a: right_a,
                b: right_b,
            },
        ) => same_point2_bits(left_a, right_a) && same_point2_bits(left_b, right_b),
        _ => false,
    }
}

fn same_alignment_bits(left: &FoldAlignment, right: &FoldAlignment) -> bool {
    let FoldAlignment {
        mode: left_mode,
        picks: left_picks,
    } = left;
    let FoldAlignment {
        mode: right_mode,
        picks: right_picks,
    } = right;
    left_mode == right_mode
        && left_picks.len() == right_picks.len()
        && left_picks
            .iter()
            .zip(right_picks)
            .all(|(left, right)| same_alignment_target_bits(left, right))
}

fn same_finish_soft_bits(left: &FinishSoftSettings, right: &FinishSoftSettings) -> bool {
    let FinishSoftSettings {
        enabled: left_enabled,
        stiffness: left_stiffness,
        pressure: left_pressure,
    } = left;
    let FinishSoftSettings {
        enabled: right_enabled,
        stiffness: right_stiffness,
        pressure: right_pressure,
    } = right;
    left_enabled == right_enabled
        && same_f64_bits(*left_stiffness, *right_stiffness)
        && same_f64_bits(*left_pressure, *right_pressure)
}

fn same_fold_step_bits(left: &FoldStep, right: &FoldStep) -> bool {
    let FoldStep {
        id: left_id,
        kind: left_kind,
        drivers: left_drivers,
        layer_order: left_order,
        alignment: left_alignment,
        finish_soft: left_finish_soft,
        note: left_note,
    } = left;
    let FoldStep {
        id: right_id,
        kind: right_kind,
        drivers: right_drivers,
        layer_order: right_order,
        alignment: right_alignment,
        finish_soft: right_finish_soft,
        note: right_note,
    } = right;
    left_id == right_id
        && left_kind == right_kind
        && left_drivers.len() == right_drivers.len()
        && left_drivers
            .iter()
            .zip(right_drivers)
            .all(|(left, right)| same_driver_line_bits(left, right))
        && match (left_order, right_order) {
            (Some(left), Some(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .zip(right)
                        .all(|(left, right)| same_point2_bits(left, right))
            }
            (None, None) => true,
            _ => false,
        }
        && match (left_alignment, right_alignment) {
            (Some(left), Some(right)) => same_alignment_bits(left, right),
            (None, None) => true,
            _ => false,
        }
        && match (left_finish_soft, right_finish_soft) {
            (Some(left), Some(right)) => same_finish_soft_bits(left, right),
            (None, None) => true,
            _ => false,
        }
        && left_note == right_note
}

fn same_display_settings_bits(left: &DisplaySettings, right: &DisplaySettings) -> bool {
    let DisplaySettings {
        front_color: left_front,
        back_color: left_back,
        grid_divisions: left_grid,
        soft_enabled: left_soft_enabled,
        soft_stiffness: left_soft_stiffness,
        soft_pressure: left_soft_pressure,
        overlap_prevention_enabled: left_overlap,
        penetration_prevention_enabled: left_penetration,
    } = left;
    let DisplaySettings {
        front_color: right_front,
        back_color: right_back,
        grid_divisions: right_grid,
        soft_enabled: right_soft_enabled,
        soft_stiffness: right_soft_stiffness,
        soft_pressure: right_soft_pressure,
        overlap_prevention_enabled: right_overlap,
        penetration_prevention_enabled: right_penetration,
    } = right;
    left_front == right_front
        && left_back == right_back
        && left_grid == right_grid
        && left_soft_enabled == right_soft_enabled
        && same_f64_bits(*left_soft_stiffness, *right_soft_stiffness)
        && same_f64_bits(*left_soft_pressure, *right_soft_pressure)
        && left_overlap == right_overlap
        && left_penetration == right_penetration
}

fn same_document_bits(left: &Document, right: &Document) -> bool {
    let Document {
        schema_version: left_schema,
        paper: left_paper,
        cp: left_cp,
        sequence: left_sequence,
        display: left_display,
    } = left;
    let Document {
        schema_version: right_schema,
        paper: right_paper,
        cp: right_cp,
        sequence: right_sequence,
        display: right_display,
    } = right;
    left_schema == right_schema
        && same_paper_bits(left_paper, right_paper)
        && same_crease_pattern_bits(left_cp, right_cp)
        && left_sequence.len() == right_sequence.len()
        && left_sequence
            .iter()
            .zip(right_sequence)
            .all(|(left, right)| same_fold_step_bits(left, right))
        && same_display_settings_bits(left_display, right_display)
}

/// ステップ列を順に適用する。
///
/// - `up_to`: 表示対象ステップ番号(0=初期状態=平ら、1=ステップ1適用後、…)。
///   手順の数を超える指定は手順の数に丸める
/// - `t`: 0..=1 の補間係数(`up_to` ステップ目の途中を表す。t=1で完了)。
///   範囲外は丸め、数値でない場合は1として扱う
///
/// `up_to` ステップ目の層順序は完了時(t=1)にだけ反映する
/// (折っている途中の紙はまだ新しい重なり順になっていないため)。
pub fn replay(doc: &Document, up_to: usize, t: f64) -> ReplayResult {
    replay_with_faces(doc, &extract_faces(&doc.cp), up_to, t)
}

/// 面抽出済みの呼び出し側(store等)のための [`replay`]。
///
/// `faces` は `doc.cp` から `extract_faces` で導出したものでなければならない
/// (別のCP由来の面を渡すと結果は意味を持たない)。
pub fn replay_with_faces(doc: &Document, faces: &[Face], up_to: usize, t: f64) -> ReplayResult {
    let up_to = up_to.min(doc.sequence.len());
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        1.0
    };
    replay_with_faces_impl(doc, faces, up_to, t, Some(replay_endpoint_cache()))
}

fn replay_with_faces_impl(
    doc: &Document,
    faces: &[Face],
    up_to: usize,
    t: f64,
    cache: Option<&ReplayEndpointCache>,
) -> ReplayResult {
    if t == 1.0
        && let Some(cache) = cache
        && let Some(cached) = cache.lookup(doc, faces, up_to)
    {
        return cached;
    }
    if let Some(cache) = cache {
        cache.note_compute();
    }
    // 現在手順がまだ始まっていないt=0は、直前手順の完了表示をそのまま再利用する。
    // 現在手順の解決警告や分類を先取りしないため、frame・角度・警告がビット一致する。
    if up_to > 0 && t <= 0.0 {
        return replay_with_faces_impl(doc, faces, up_to - 1, 1.0, cache);
    }
    let plan = plan_steps(doc, faces, up_to, t);
    let layer_transition = LayerTransition {
        start: plan.order_start.clone(),
        end: plan.order_end.clone(),
        progress: t,
        order_is_authoritative: plan.transition_order_is_authoritative,
    };
    let mut warnings = plan.warnings;

    // 直前endpointは、途中再生のwarm startか、現在姿勢だけではsurface順が不完全な
    // ときに限って遅延再生する。現在の実深度だけで完結する大きなCPを全履歴再生しない。
    let mut previous = None;
    let current_geometry_changes = plan.path.is_some();
    let (mut result, mut current_path) = if t < 1.0
        && let Some(path) = &plan.path
    {
        // 直接操作角があるときはt=0と同じ直前姿勢から始める。共同path targetだけの
        // 複合技法は、そのtarget自身を初期値にしないと平坦分岐の零勾配に留まるため、
        // 最初だけwarmを渡さず、2点目以降は有限な直前候補を引き継ぐ。
        let warm = if path.hard.is_empty() {
            None
        } else {
            if previous.is_none() && up_to > 0 {
                previous = Some(replay_with_faces_impl(doc, faces, up_to - 1, 1.0, cache));
            }
            previous
                .as_ref()
                .map(|previous| previous.hinge_angles.clone())
        };
        solve_along(doc, faces, path, t, warm)
    } else {
        // 完了形の物理解は従来どおりone-shotに保つ。surface順のためにsolver branchや
        // 収束結果を変えず、下でcurrent stepのnear-final probeだけを別に求める。
        let warm: HashMap<EdgeId, f64> = plan
            .flat_exact
            .iter()
            .map(|driver| (driver.hinge, driver.target_angle_deg))
            .collect();
        (
            solve_display_near(
                doc,
                faces,
                &plan.display_hard,
                &plan.display_preferred,
                Some(&warm),
            ),
            Vec::new(),
        )
    };
    if !result.converged {
        warnings.push(format!(
            "手順{up_to}までの形が展開図から求まりませんでした(いちばん近い形で表示します)。一部の層だけを折る手順は、展開図からの折り直しでは正確に再現できないことがあります"
        ));
    }

    // まず現在の実深度/exact制約だけを試す。completeなら履歴もprobeも不要。
    // 角度不変手順は幾何も不変なので、直前のcomplete順をそのまま保つ。
    let mut surface_order_provenance = if current_geometry_changes || up_to == 0 {
        stamp_canonical_surface_order_from_angles(doc, faces, &current_path, None, &mut result)
    } else {
        None
    };
    if surface_order_provenance.is_none() && up_to > 0 {
        if previous.is_none() {
            previous = Some(replay_with_faces_impl(doc, faces, up_to - 1, 1.0, cache));
        }
        if current_geometry_changes {
            // current depthだけで決まらない平坦な束に限り、直前warmから固定3点で
            // 終点直前まで追う。完了形のone-shot物理解は一切変更しない。
            if t >= 1.0
                && let Some(path) = &plan.path
            {
                current_path = solve_surface_approach(
                    doc,
                    faces,
                    path,
                    previous.as_ref().map(|previous| &previous.hinge_angles),
                );
            }
            surface_order_provenance = stamp_canonical_surface_order_from_angles(
                doc,
                faces,
                &current_path,
                previous
                    .as_ref()
                    .and_then(|previous| previous.surface_order_provenance.as_ref()),
                &mut result,
            );
            if surface_order_provenance.is_none()
                && t >= 1.0
                && let Some(path) = &plan.path
            {
                let full = solve_surface_approach_full(
                    doc,
                    faces,
                    path,
                    previous.as_ref().map(|previous| &previous.hinge_angles),
                );
                current_path = combine_surface_paths(current_path, full);
                surface_order_provenance = stamp_canonical_surface_order_from_angles(
                    doc,
                    faces,
                    &current_path,
                    previous
                        .as_ref()
                        .and_then(|previous| previous.surface_order_provenance.as_ref()),
                    &mut result,
                );
                if surface_order_provenance.is_none() {
                    surface_order_provenance =
                        stamp_canonical_surface_order_from_flat_path(doc, faces, &mut result);
                }
            }
        } else {
            surface_order_provenance = previous
                .as_ref()
                .and_then(|previous| preserve_complete_surface_order(previous, &mut result))
                .or_else(|| {
                    stamp_canonical_surface_order_from_angles(doc, faces, &[], None, &mut result)
                });
        }
    }
    if surface_order_provenance.is_none() && t >= 1.0 {
        surface_order_provenance =
            stamp_canonical_surface_order_from_motion(doc, faces, &mut result);
    }
    let hinge_angles = result.angles;
    let relaxations = result.relaxations;
    let closure_rms = result.closure_rms;
    let best_effort = result.best_effort;
    let converged = result.converged;
    let mut frame = result.frame;
    let layer_of: HashMap<FaceId, u32> = plan
        .order
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, u32::try_from(i).unwrap_or(u32::MAX)))
        .collect();
    for f in &mut frame.faces {
        f.layer = layer_of.get(&f.face).copied().unwrap_or(0);
    }

    let replayed = ReplayResult {
        frame,
        skipped: plan.skipped,
        warnings,
        suspect_hinges: Vec::new(),
        driver_hinges: plan.driver_hinges,
        hinge_angles,
        surface_order_provenance,
        sequence_targets: plan.sequence_targets,
        relaxations,
        closure_rms,
        best_effort,
        converged,
        layer_transition,
    };
    if t == 1.0
        && let Some(cache) = cache
    {
        cache.store(doc, faces, up_to, &replayed);
    }
    replayed
}

/// 手順を `up_to` まで再生した結果が平坦(全ての面がz≈0の平面に乗る)なら、
/// その平坦状態([`FlatState`])を返す。
///
/// 3D状態は保存しない設計なので、畳んだ状態の上に折る([`crate::fold_through`])ための
/// 現在の平坦状態は手順から導出する。各面の3D姿勢(回転行列+並進)から
/// xy平面内の等長変換を取り出す(回転角は線形部分の第1列の偏角、`mirrored`は
/// xy成分の行列式が負かどうか、並進はxy成分)。層順序は手順の`layer_order`を
/// [`FlatState::resolve_order`] で現在の面IDへ解決する(無ければ面ID昇順)。
///
/// 座標系は3D表示と同じ(根面=最小面IDの面が恒等変換)。畳んだ紙の上に画面から
/// 引いた折り線をそのまま渡せる。[`crate::fold_through`] が返す状態も同じ座標系へ
/// そろえてあるので、層順序(下→上)の向きは常に一致する。
///
/// 平坦でない(折り途中の角度が残る)場合はErrを返す。
///
/// 戻り値には手順を読み直したときの警告(折り線が見つからない手順・解決できない
/// 層順序の代表点など)を添える。折れなくなるほどの問題ではないので止めはしないが、
/// 捨てると「知らないうちに一部の手順が無視された状態の上に折る」ことになるため、
/// 呼び出し側へ渡して利用者に見せること(「止めずに警告」原則)。
pub fn flat_state_at(
    doc: &Document,
    faces: &[Face],
    up_to: usize,
) -> Result<(FlatState, Vec<String>), String> {
    let up_to = up_to.min(doc.sequence.len());
    let plan = plan_steps(doc, faces, up_to, 1.0);
    // 後から積んだ指定が優先(HashMapへの順次挿入で後勝ちになる)
    let angles: HashMap<EdgeId, f64> = plan
        .flat_exact
        .iter()
        .map(|d| (d.hinge, d.target_angle_deg))
        .collect();
    let folded = ori3_rigid::propagate(&doc.cp, faces, &angles);

    let mut placements: HashMap<FaceId, Isometry2> = HashMap::with_capacity(faces.len());
    for f in faces {
        let (r, t) = folded
            .transforms
            .get(&f.id)
            .ok_or_else(|| format!("面 {} の姿勢が求まりませんでした", f.id))?;
        // 面が z=0 平面に乗るのは「線形部分がz成分を作らない」かつ「並進のzが0」のとき。
        // 面上の点 (x, y) の高さは x_axis.z·x + y_axis.z·y + t.z で決まる。
        if r.x_axis.z.abs() > FLAT_EPS || r.y_axis.z.abs() > FLAT_EPS || t.z.abs() > FLAT_EPS {
            return Err(
                "折り途中の状態では折れません。手順を完了した状態で折ってください".to_string(),
            );
        }
        // xy成分の行列式が負なら裏返っている。回転角はどちらの場合も第1列の偏角
        // (Isometry2は p' = R(θ)·M(mirrored)·p + t で、M は y の符号反転)。
        let det = r.x_axis.x * r.y_axis.y - r.y_axis.x * r.x_axis.y;
        placements.insert(
            f.id,
            Isometry2 {
                rotation: r
                    .x_axis
                    .y
                    .atan2(r.x_axis.x)
                    .rem_euclid(std::f64::consts::TAU),
                translation: DVec2::new(t.x, t.y),
                mirrored: det < 0.0,
            },
        );
    }
    Ok((
        FlatState {
            placements,
            order: plan.order,
        },
        plan.warnings,
    ))
}

/// 折り途中の姿勢を求める分割数。目標角を `SUBSTEPS` 等分して少しずつ動かし、
/// 前の解を次の初期値にする(連続法)。1回で `t` へ飛ばすと、対称な複数の解の
/// あいだで解が飛び移り、紙が瞬間移動したように見える。
const SUBSTEPS: u32 = 12;

/// 「開いている」か「閉じている」かを見分ける角度の幅(度)。
///
/// 記録した目標角は ±180 / 0 のような値なので、この幅は
/// **どちらでもない(角度が変わらない)線を外す**ためだけに使う。
/// 実測: 鳥の基本形の花弁折りで、開く2本は 180 → 0(差 180度)、
/// 閉じる5本は 0 → ±180(差 180度)で、いちばん小さい差でも 180度ある。
/// 1度はその **180分の1** で、丸めの雑音(1e-9度未満)より十分大きい。
const ANGLE_PHASE_EPS: f64 = 1.0;

/// 完了形と比較するcurrent-step probe。表示テストと同じく、平坦化直前の有限な高さを
/// 十分残しつつendpointに近い姿勢を使う。
const SURFACE_APPROACH_PROGRESS: [f64; 3] = [0.5, 0.9, 0.99];

/// 完了形の物理解を変えず、surface順のためだけにcurrent stepを固定3点で追う。
fn solve_surface_approach(
    doc: &Document,
    faces: &[Face],
    path: &StepPath,
    warm: Option<&HashMap<EdgeId, f64>>,
) -> Vec<Frame3D> {
    solve_surface_approach_at(doc, faces, path, warm, SURFACE_APPROACH_PROGRESS)
}

/// 粗い3点で上下がcompleteにならない複合経路だけ、実t=.99再生と同じ12分割へ精緻化。
fn solve_surface_approach_full(
    doc: &Document,
    faces: &[Face],
    path: &StepPath,
    warm: Option<&HashMap<EdgeId, f64>>,
) -> Vec<Frame3D> {
    solve_surface_approach_at(
        doc,
        faces,
        path,
        warm,
        (1..=SUBSTEPS)
            .map(|step| SURFACE_APPROACH_PROGRESS[2] * f64::from(step) / f64::from(SUBSTEPS)),
    )
}

/// 粗い3点と12分割の追従pathを、両方とも重なり順の証拠として残す。
///
/// どちらも同じ直前姿勢から同じ終点へ向かう追い方で、刻みの細かさだけが違う。
/// 片方を捨てる理由が無いので、上下を読み取る側が「終点にいちばん近い姿勢から順に、
/// 最初に上下が分かれた姿勢で決める」という規則をそのまま使えるように、
/// **細かい12分割を後ろへ**置く。決められた対は12分割が決め、12分割のどの姿勢でも
/// ぴったり重なったままだった対だけ、粗い3点側が決める。
///
/// 実測(2026-08-22、カエルの受け入れ検査9点): 12分割だけを証拠にすると
/// `up_to=9` で決められない重なりが 249組残ったが、3点だけなら 1組だった。
/// 逆に `up_to=7` は3点で11組、12分割を足すと1組になる。どちらか一方では足りない。
fn combine_surface_paths(coarse: Vec<Frame3D>, full: Vec<Frame3D>) -> Vec<Frame3D> {
    let mut combined = coarse;
    combined.extend(full);
    combined
}

/// 復元用の追従pathへ渡す**最初の**初期値。
///
/// 直接操作角(`hard`)があるときだけ直前完成角をwarmとして渡す。共同path targetだけの
/// 複合技法では、直前姿勢が新しい希望角に対しても停留点になり得る。そこから始めると
/// 最初のsolveが旧姿勢をそのまま返し、2点目以降もその解を引き継ぐため、経路全体が
/// 1度も動かない。動かない経路は重なりの上下を1対も分けられないので、
/// 「重なり順が現在の角度から決まらない」に必ず落ちる。
///
/// 途中再生の [`solve_along`] は同じ理由で最初のwarmを渡さない契約であり、
/// 復元側だけが違っていた。2点目以降は両方とも直前候補を引き継ぐ(連続法)。
fn initial_surface_approach_warm<'a>(
    path: &StepPath,
    warm: Option<&'a HashMap<EdgeId, f64>>,
) -> Option<&'a HashMap<EdgeId, f64>> {
    if path.hard.is_empty() { None } else { warm }
}

fn solve_surface_approach_at(
    doc: &Document,
    faces: &[Face],
    path: &StepPath,
    warm: Option<&HashMap<EdgeId, f64>>,
    progress: impl IntoIterator<Item = f64>,
) -> Vec<Frame3D> {
    let mut warm = initial_surface_approach_warm(path, warm).cloned();
    let mut frames = Vec::with_capacity(SURFACE_APPROACH_PROGRESS.len());
    for progress in progress {
        let drivers = path.drivers_at(progress);
        let targets = path.targets_at(progress);
        let candidate = solve_display_near(doc, faces, &drivers, &targets, warm.as_ref());
        if !is_finite_result(&candidate, faces.len()) {
            break;
        }
        warm = Some(candidate.angles);
        frames.push(candidate.frame);
    }
    frames
}

/// 折り道 `path`(ヒンジごとの 直前の角度→目標角)を `0..=t` までたどり、
/// 途中で紙がつながったままになる姿勢を求める。
///
/// 各分割点で「閉包を満たす形のうち、補間した角度にいちばん近いもの」を解き、
/// その解を次の分割点の初期値にする。分割点は `t` の等分なので `t` だけで
/// 決まり、同じ入力なら常に同じ結果になる(SYS-004)。
fn solve_along(
    doc: &Document,
    faces: &[Face],
    path: &StepPath,
    t: f64,
    mut warm: Option<HashMap<EdgeId, f64>>,
) -> (ori3_rigid::SolveResult, Vec<Frame3D>) {
    let mut last_finite: Option<ori3_rigid::SolveResult> = None;
    let mut final_failure: Option<ori3_rigid::SolveResult> = None;
    let mut surface_path = Vec::with_capacity(SUBSTEPS as usize);
    let mut iterations = 0u32;
    for i in 1..=SUBSTEPS {
        let s = t * f64::from(i) / f64::from(SUBSTEPS);
        let drivers = path.drivers_at(s);
        let targets = path.targets_at(s);
        let mut candidate = solve_display_near(doc, faces, &drivers, &targets, warm.as_ref());
        iterations = iterations.saturating_add(candidate.iterations);
        if is_finite_result(&candidate, faces.len()) {
            candidate.iterations = iterations;
            surface_path.push(candidate.frame.clone());
            warm = Some(candidate.angles.clone());
            last_finite = Some(candidate);
            final_failure = None;
        } else if i == SUBSTEPS {
            final_failure = Some(candidate);
        }
    }
    let result = match (last_finite, final_failure) {
        (Some(previous), Some(failed)) => previous_replay_result(previous, failed, iterations),
        (Some(mut result), None) => {
            result.iterations = iterations;
            result
        }
        (None, Some(mut failed)) => {
            failed.iterations = iterations;
            failed
        }
        (None, None) => unreachable!("SUBSTEPSは1以上"),
    };
    (result, surface_path)
}

/// soft抵抗を外した最終閉包段まで行う表示solve。
fn solve_display_near(
    doc: &Document,
    faces: &[Face],
    drivers: &[Driver],
    targets: &HashMap<EdgeId, f64>,
    warm: Option<&HashMap<EdgeId, f64>>,
) -> ori3_rigid::SolveResult {
    ori3_rigid::solve_near_exact_without_surface_order(&doc.cp, faces, drivers, targets, warm)
}

/// 有限なsolver結果の全ヒンジ角から、completeなcanonical surface順だけを刻印する。
///
/// 次手順へ返すprovenanceは、この導出時にも正面積で重なっていた面対だけを含む。
/// 不完全な導出ではseed順を物理順として刻まず、provenanceも発行しない。
fn stamp_canonical_surface_order_from_angles(
    doc: &Document,
    faces: &[Face],
    current_path: &[Frame3D],
    previous: Option<&ori3_rigid::SurfaceOrderProvenance>,
    result: &mut ori3_rigid::SolveResult,
) -> Option<ori3_rigid::SurfaceOrderProvenance> {
    let derived = ori3_rigid::surface_order_from_angles(
        &doc.cp,
        faces,
        &result.angles,
        current_path,
        previous,
    );
    let (order, provenance) = derived.ok()?;
    ori3_rigid::stamp_surface_order(&mut result.frame, &order).ok()?;
    Some(provenance)
}

/// 実current-step経路を固定12点まで追ってもcompleteにならない場合だけ、平坦な
/// 展開図からのcanonical経路を最後の幾何fallbackとして刻印する。
fn stamp_canonical_surface_order_from_flat_path(
    doc: &Document,
    faces: &[Face],
    result: &mut ori3_rigid::SolveResult,
) -> Option<ori3_rigid::SurfaceOrderProvenance> {
    let (order, provenance) =
        ori3_rigid::surface_order_from_angles_flat_path(&doc.cp, faces, &result.angles).ok()?;
    ori3_rigid::stamp_surface_order(&mut result.frame, &order).ok()?;
    Some(provenance)
}

/// replay固有の経路だけでcompleteにならない複合手順では、同じ終点角をwarmにした
/// `solve_motion` の既存canonical path authorityを再利用する。motionが証明を発行し、
/// 同じ終点角へ着いた場合だけ、その全面順を現在の表示frameへ刻印する。
fn stamp_canonical_surface_order_from_motion(
    doc: &Document,
    faces: &[Face],
    result: &mut ori3_rigid::SolveResult,
) -> Option<ori3_rigid::SurfaceOrderProvenance> {
    let solved = ori3_rigid::solve_authoritative_surface_order(&doc.cp, faces, &result.angles)?;
    if solved.frame().faces.len() != result.frame.faces.len()
        || solved
            .frame()
            .faces
            .iter()
            .zip(&result.frame.faces)
            .any(|(motion_face, replay_face)| {
                motion_face.face != replay_face.face
                    || motion_face.mirrored != replay_face.mirrored
                    || motion_face.polygon != replay_face.polygon
            })
    {
        return None;
    }
    let (_, order, provenance) = solved.into_parts();
    ori3_rigid::stamp_surface_order(&mut result.frame, &order).ok()?;
    Some(provenance)
}

/// 角度が変わらない手順では、直前のcompleteなcanonical順とprovenanceをそのまま保つ。
///
/// total rankから新しいprovenanceは作らず、opaqueな証明をcloneできる場合に限って、
/// 同じ幾何へ直前の全面順を再刻印する。保存layerは参照しない。
fn preserve_complete_surface_order(
    previous: &ReplayResult,
    result: &mut ori3_rigid::SolveResult,
) -> Option<ori3_rigid::SurfaceOrderProvenance> {
    let provenance = previous.surface_order_provenance.clone()?;
    let mut ranked = previous
        .frame
        .faces
        .iter()
        .map(|face| (face.surface_rank, face.face))
        .collect::<Vec<_>>();
    ranked.sort_by_key(|&(rank, _)| rank);
    if ranked
        .iter()
        .enumerate()
        .any(|(rank, &(stored, _))| usize::try_from(stored).ok() != Some(rank))
    {
        return None;
    }
    let order = ranked.into_iter().map(|(_, face)| face).collect::<Vec<_>>();
    ori3_rigid::stamp_surface_order(&mut result.frame, &order).ok()?;
    Some(provenance)
}

/// 保存手順から現在の再生位置で有効な層順序(下→上)を導出する。
///
/// 戻り値が`Some`なのは、現在位置までに非空の
/// [`FoldStep::layer_order`](ori3_model::FoldStep::layer_order)が1点以上、
/// 現在の面へ解決できた場合だけ。
/// 平坦でないPoseなど`layer_order=None`の手順と、代表点が全て未解決の手順は
/// 直前の保存順を保つ。現在手順の途中(`t < 1`)は開始前の順、完了時だけ終了順を返す。
/// 初期の面ID順は表示用fallbackであって保存手順の権威ではないため`None`を返す。
#[must_use]
pub fn saved_layer_order_at(
    doc: &Document,
    faces: &[Face],
    up_to: usize,
    t: f64,
) -> Option<Vec<FaceId>> {
    let up_to = up_to.min(doc.sequence.len());
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        1.0
    };
    plan_steps(doc, faces, up_to, t).saved_order
}

fn is_finite_result(result: &ori3_rigid::SolveResult, expected_faces: usize) -> bool {
    result.closure_rms.is_finite()
        && result.angles.values().all(|angle| angle.is_finite())
        && result.relaxations.iter().all(|relaxation| {
            relaxation.target_angle_deg.is_finite()
                && relaxation.actual_angle_deg.is_finite()
                && relaxation.delta_deg.is_finite()
        })
        && result.frame.faces.len() == expected_faces
        && result.frame.faces.iter().all(|face| {
            face.polygon
                .iter()
                .flatten()
                .all(|coordinate| coordinate.is_finite())
        })
}

/// 最終要求で有限形を作れなかった場合だけ、直前の有限姿勢へ警告を載せる。
fn previous_replay_result(
    mut previous: ori3_rigid::SolveResult,
    failed: ori3_rigid::SolveResult,
    iterations: u32,
) -> ori3_rigid::SolveResult {
    previous.converged = false;
    previous.best_effort = true;
    previous.iterations = iterations;
    for warning in failed.frame.warnings {
        if !previous.frame.warnings.contains(&warning) {
            previous.frame.warnings.push(warning);
        }
    }
    if !previous
        .frame
        .warnings
        .iter()
        .any(|warning| warning.contains("収束していません"))
    {
        previous
            .frame
            .warnings
            .push("追従計算が収束していません".to_string());
    }
    previous
}

/// 現在手順を補間する経路。hardとpreferredの出所を保ったまま連続法へ渡す。
struct StepPath {
    /// 現在の単一Simple直接操作。(辺ID, 開始角, 完了角)
    hard: Vec<(EdgeId, f64, f64)>,
    /// 過去手順、Pose、複数lineの共同path target。(辺ID, 開始角, 完了角)
    preferred: Vec<(EdgeId, f64, f64)>,
    /// **この手が動かさない**折り目と、その折り途中で押さえる角(辺ID昇順)。
    ///
    /// 入るのは次の2種類で、どちらも「この手が動かすものではない」という
    /// 同じ理由で押さえる。役割が同じなので1つの表にまとめてある。
    ///
    /// 1. **どの手順もまだ一度も指定していない折り目** → `0°`。
    ///    折り筋を先に全部引いてから畳む展開図では、まだ折っていない折り目も
    ///    展開図の上には存在する。`extract_faces` はそこで面を分けるので、
    ///    この辺は「2面が共有する辺」= ちょうつがいとして数えられてしまう。
    ///    補間の途中でこれを自由にすると、**利用者がまだ折っていない場所が勝手に折れ**、
    ///    紙が別の紙を突き抜ける。両端(`t=0` / `t=1`)は
    ///    [`StepPlan::flat_exact`] が0°で固定しているのに、折り途中だけ自由という
    ///    食い違いだった。
    /// 2. **前の手順が決めた角のうち、この手が動かす折り目と頂点を共有しないもの**
    ///    → その決まった角。**記録した技法の再生(共同path target)のときだけ**入れる。
    ///    技法の記録にはその手が変える折り目が全て書かれているので、そこに無い
    ///    折り目はこの手では動かない。実際の紙でも、花弁折りをしている最中に
    ///    模型の他の場所が勝手に開くことはない。
    ///
    ///    **この手が動かす折り目と同じ頂点に集まる折り目は入れない。**
    ///    頂点まわりの閉包(その頂点に集まる折り目の輪)は1本だけでは動けないので、
    ///    隣の折り目が譲らないと紙がまったく動かなくなる。実際の紙でも、
    ///    つぶし折りは袋を開くために隣の折り目がいったん戻る。
    ///    実測(予備基本形の4面、`t` を動かしたときの高さの幅、紙の幅は1):
    ///    隣まで押さえると `t=0.5` で既に **2.88e-14**(＝もう畳み切っていて
    ///    折り途中の動きが無い)。隣を譲らせると `t=0.99` で **1.570538e-2**、
    ///    `t=0.999` で **1.570795e-3** となり、残り角の正弦(`0.5·sin1.8°`)と一致する。
    ///
    ///    **利用者がいま1本を動かしている手(単一Simple)には入れない。**
    ///    2026-08-11の利用者決定「動かしている折り目が最優先・他の折り目は
    ///    紙のつながりが成立するよう自然に引っ張られて動く」に従い、
    ///    そのときの過去角は希望(soft)のままにして譲れるようにする。
    ///
    /// 実測(`crates/ori3-layers/tests/precrease_pose.rs`、鳥の基本形の参照手順を
    /// `t = 0, 0.05, …, 1` の21点で走査、紙の幅は1):
    ///
    /// | 手 | 技法 | どちらも押さえない | 1だけ押さえる | **1と2を押さえる** |
    /// |---:|---|---|---|---|
    /// | 3 | つぶし折り | 7組 / 2.850453e-1 | 0組 | **0組** |
    /// | 4 | つぶし折り | 9組 / 2.058969e-1 | 0組 | **0組** |
    /// | 5 | 花弁折り | 6組 / 2.046746e-1 | 1組 / 7.745151e-2 | **0組** |
    /// | 6 | 花弁折り | 6組 / 1.891851e-1 | 3組 / 2.351486e-2 | **0組** |
    ///
    /// 2を押さえないと、ソルバーは**前の手順で折った折り目のほうを開いて**
    /// 逃げ道にしていた(実測: 手6の折り途中で、この手が動かさない辺22・辺31が
    /// ±180°から∓150°まで戻り、動かすはずの辺26は±180°のまま動かなかった)。
    /// 辺22・辺31は、この手が動かす折り目とは頂点を共有しない。
    held: Vec<(EdgeId, f64)>,
    /// **先に開く**折り目(いま折れている角を0°へ戻すもの。辺ID昇順)。
    ///
    /// 花弁折りのような手は、**袋を開きながら別の線を閉じる**。実際の紙では
    /// 「先に開く、次に閉じる」の順で、同時ではない。全部の角を同じ `t` で
    /// 一様に動かすと、開き切る前に閉じ始めた架空の形になり、紙が紙を突き抜ける。
    open_first: Vec<EdgeId>,
    /// 開く区間の終わり(`0 < split <= 1`)。`1.0` は「区間を分けない」= 従来どおり。
    ///
    /// 開く線と閉じる線が**両方ある**手だけ 1.0 未満になる。値は
    /// **開く線の本数 / (開く線 + 閉じる線)の本数**で、`t` と手順だけから決まるので
    /// 結果は決定的(SYS-004)。
    ///
    /// **この値は測定に合わせて選んだものではない**(`CLAUDE.md` §10.7.9)。
    /// 「動かす線の本数の比で時間を配る」という素直な決め方で、
    /// 鳥の基本形では 開く2本 / 閉じる5本 → `2/7 = 0.2857…` になる。
    /// 参考までに `0.15 / 0.2857 / 0.4 / 0.5 / 0.6 / 0.75 / 0.9` を測った結果、
    /// 手5の自己交差はどの値でも 1組(区間を分けない従来は6組)、
    /// 手6は 0.2857 と 0.4 が最良の 3組・最深 2.3e-2 で、
    /// 0.5以上にすると 6組・11組へ増えた。件数比の値は最良の側にある。
    split: f64,
}

impl StepPath {
    /// 区間分けを反映した、この折り目の進み具合。
    ///
    /// `split == 1.0`(区間を分けない)なら、全ての折り目が `s` をそのまま使う。
    fn progress_of(&self, hinge: EdgeId, s: f64) -> f64 {
        if self.split >= 1.0 {
            return s;
        }
        if self.open_first.binary_search(&hinge).is_ok() {
            (s / self.split).min(1.0)
        } else {
            ((s - self.split) / (1.0 - self.split)).clamp(0.0, 1.0)
        }
    }

    /// 折り道の `progress` 地点でソルバーへ渡す固定角。
    ///
    /// 途中再生([`solve_along`])と、重なり順の復元
    /// ([`solve_surface_approach_at`])は**同じ道**をたどらなければならない
    /// (`the_recovery_path_matches_the_intermediate_pose_the_replay_shows` が
    /// その一致を検査している)。片方だけが固定角を足すと道が分かれるので、
    /// 作る場所を1つにまとめてある。
    fn drivers_at(&self, progress: f64) -> Vec<Driver> {
        self.hard
            .iter()
            .map(|&(hinge, from, to)| {
                let local = self.progress_of(hinge, progress);
                Driver {
                    hinge,
                    target_angle_deg: from + (to - from) * local,
                }
            })
            .chain(self.held.iter().map(|&(hinge, target_angle_deg)| Driver {
                hinge,
                target_angle_deg,
            }))
            .collect()
    }

    /// 折り道の `progress` 地点でソルバーへ渡す希望角。[`Self::drivers_at`] と対。
    fn targets_at(&self, progress: f64) -> HashMap<EdgeId, f64> {
        self.preferred
            .iter()
            .map(|&(hinge, from, to)| {
                let local = self.progress_of(hinge, progress);
                (hinge, from + (to - from) * local)
            })
            .collect()
    }
}

/// `up_to` ステップまでの角度指定・層順序・警告(replayとflat_state_atの共通処理)。
struct StepPlan {
    /// 表示solveで固定する現在の直接操作角。
    display_hard: Vec<Driver>,
    /// 表示solveでなるべく保つ過去手順、Pose、共同path target。
    display_preferred: HashMap<EdgeId, f64>,
    /// 表示位置までに解決できた全明示角。未指定ヒンジの0°は含めない。
    sequence_targets: Vec<Driver>,
    /// FlatState専用のexact角。従来どおり未指定ヒンジも0°で固定する。
    flat_exact: Vec<Driver>,
    /// 手順内のDriverLineから解決できたヒンジ。未指定ヒンジの0度固定は含めない。
    driver_hinges: Vec<EdgeId>,
    /// 現在手順のhard/preferredを保った折り道。完了時も実際の経路をsurface順へ使う。
    path: Option<StepPath>,
    /// 層順序(下→上)
    order: Vec<FaceId>,
    /// 保存済みlayer_orderを1点以上解決して得た、現在位置の権威ある順序。
    /// 初期の面ID順しか無い場合はNone。
    saved_order: Option<Vec<FaceId>>,
    /// `up_to` 手順へ入る直前の層順序(接触補正用)。
    order_start: Vec<FaceId>,
    /// `up_to` 手順を完了したときの層順序(接触補正用)。
    order_end: Vec<FaceId>,
    /// 現在transitionまでに解決済みの保存順を1つ以上含むか。
    /// 現在手順の途中では表示authorityより先にendだけ解決される場合がある。
    transition_order_is_authoritative: bool,
    skipped: Vec<StepId>,
    warnings: Vec<String>,
}

/// 手順を順に読み、角度指定と層順序を積み上げる。`up_to` は `doc.sequence` の
/// 範囲に収まっていること(呼び出し側で丸める)。
fn plan_steps(doc: &Document, faces: &[Face], up_to: usize, t: f64) -> StepPlan {
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        1.0
    };

    let mut warnings: Vec<String> = Vec::new();
    let mut skipped: Vec<StepId> = Vec::new();
    // 現在の層順序(下→上)。初期状態は面ID昇順。
    let mut order = FlatState::initial(&doc.cp, faces).order;
    // 初期の面ID順は決定的fallbackであり、折り手順が記録した順序ではない。
    let mut saved_order: Option<Vec<FaceId>> = None;
    let mut transition_order_is_authoritative = false;
    let mut order_start = order.clone();
    let mut order_end = order.clone();
    // ヒンジごとの目標角(後から積んだステップが優先される)。BTreeMapなので
    // ソルバーへ渡す順序も辺ID昇順で決定的(SYS-004)。
    let mut angles: BTreeMap<EdgeId, f64> = BTreeMap::new();
    // `up_to` ステップ目に入る直前の角度(折り途中の補間の始点)
    let mut before: BTreeMap<EdgeId, f64> = BTreeMap::new();
    // 現在手順で解決できた角度。過去角と混ぜず、表示solveのhard判定に使う。
    let mut current: BTreeMap<EdgeId, f64> = BTreeMap::new();
    let mut current_kind: Option<TechniqueKind> = None;
    let mut current_driver_lines = 0usize;

    for (i, step) in doc.sequence.iter().take(up_to).enumerate() {
        let number = i + 1; // 利用者向けの手順番号は1始まり
        let last = number == up_to;
        if last {
            // 飛ばした場合も「直前の状態」を正しく指すよう、解決の前に控える
            before.clone_from(&angles);
            order_start.clone_from(&order);
            order_end.clone_from(&order);
        }

        let mut resolved_lines = 0usize;
        let mut step_drivers: Vec<Driver> = Vec::new();
        for line in &step.drivers {
            let edges = resolve_driver_edges(&doc.cp, line);
            if edges.is_empty() {
                continue;
            }
            resolved_lines += 1;
            if last {
                let mut line_changes_angle = false;
                for &hinge in &edges {
                    let from = before.get(&hinge).copied().unwrap_or(0.0);
                    if !same_step_angle(from, line.target_angle_deg) {
                        line_changes_angle = true;
                    }
                }
                if line_changes_angle {
                    current_driver_lines += 1;
                }
            }
            step_drivers.extend(edges.into_iter().map(|hinge| Driver {
                hinge,
                target_angle_deg: line.target_angle_deg,
            }));
        }
        if resolved_lines == 0 && !step.drivers.is_empty() {
            skipped.push(step.id);
            warnings.push(format!(
                "手順{number}の折り線が見つからないため、この手順を飛ばしました"
            ));
            continue;
        }
        if resolved_lines < step.drivers.len() {
            warnings.push(format!("手順{number}の折り線の一部が見つかりません"));
        }
        if last {
            current_kind = Some(step.kind);
            let final_step_angles: BTreeMap<EdgeId, f64> = step_drivers
                .iter()
                .map(|driver| (driver.hinge, driver.target_angle_deg))
                .collect();
            current = final_step_angles
                .into_iter()
                .filter(|(hinge, target)| {
                    !same_step_angle(before.get(hinge).copied().unwrap_or(0.0), *target)
                })
                .collect();
        }
        for d in step_drivers {
            angles.insert(d.hinge, d.target_angle_deg);
        }

        // 表示の層順序はステップ完了時にだけ更新する。一方、接触補正は折っている
        // 最中にも完了順を使うため、最後の手順だけは先読みして別に保持する。
        // 層順序を持たないPoseと、代表点が1点も解決できない手順は直前順を保つ。
        let mut resolved_order = None;
        if let Some(points) = &step.layer_order
            && !points.is_empty()
        {
            let (resolved, mut w) = FlatState::resolve_order(&doc.cp, faces, points);
            // resolve_orderは解決できなかった点ごとにちょうど1件の警告を返すので、
            // 警告の数が点の数と同じなら1点も解決できていない
            if w.len() < points.len() {
                resolved_order = Some(resolved);
            }
            // 途中フレームの先読みだけで、従来より早く警告を見せない。
            if !last || t >= 1.0 {
                warnings.append(&mut w);
            }
        }
        transition_order_is_authoritative |= resolved_order.is_some();
        if last {
            if let Some(next) = resolved_order {
                order_end = next.clone();
                if t >= 1.0 {
                    saved_order = Some(next);
                }
            }
            if t >= 1.0 {
                order.clone_from(&order_end);
            }
        } else if let Some(next) = resolved_order {
            order = next.clone();
            saved_order = Some(next);
        }
    }

    let hinges = hinge_edges(faces);

    // IPCと表示solveへ渡す明示角は、現在の再生位置まで補間する。未指定ヒンジは
    // この集合へ0°として足さない。それらはlow/freeであり、warm startだけで枝を保つ。
    let mut display_angles = if t < 1.0 {
        before.clone()
    } else {
        angles.clone()
    };
    if t > 0.0 && t < 1.0 {
        for (&hinge, &target) in &current {
            let start = before.get(&hinge).copied().unwrap_or(0.0);
            display_angles.insert(hinge, start + (target - start) * t);
        }
    }

    // flat_motion由来の複数DriverLineは、利用者が直接指定した独立角ではなく、
    // 完了形から記録した全変化ヒンジである。元のMotionPartやactive折り目は保存されず、
    // 途中の一様な角度補間は一般に剛体経路にならないため、共同path targetとして
    // 閉じた経路へ射影する。現行形式で直接操作と確定できる単一Simple lineだけは
    // 補間中もhardにし、閉じることが既知のt=1では全ての非Pose currentをhardに戻す。
    let current_is_direct_angle =
        current_kind == Some(TechniqueKind::Simple) && current_driver_lines == 1;
    let current_is_path_target =
        current_kind == Some(TechniqueKind::Pose) || (t < 1.0 && !current_is_direct_angle);
    let current_is_hard = |hinge: EdgeId| !current_is_path_target && current.contains_key(&hinge);
    let display_hard: Vec<Driver> = if current_is_path_target {
        Vec::new()
    } else {
        current
            .keys()
            .filter(|&&hinge| current_is_hard(hinge))
            .filter_map(|hinge| {
                display_angles.get(hinge).map(|&target_angle_deg| Driver {
                    hinge: *hinge,
                    target_angle_deg,
                })
            })
            .collect()
    };
    let display_preferred: HashMap<EdgeId, f64> = display_angles
        .iter()
        .filter(|(hinge, _)| !current_is_hard(**hinge))
        .map(|(&hinge, &angle)| (hinge, angle))
        .collect();

    // 連続法でも出所を混ぜない。直接操作角だけがhard、過去角はその場に留まる
    // soft target、Poseと共同path targetは補間するsoft targetになる。
    let path = if t > 0.0 && !current.is_empty() {
        // 完了形のone-shot solveでは複合技法をhardに戻すが、surface probeはt=.99の
        // 実再生と同じ契約を使う。単一Simpleの直接操作だけがhardである。
        let path_is_hard = |hinge: EdgeId| current_is_direct_angle && current.contains_key(&hinge);
        let hard = current
            .iter()
            .filter(|(hinge, _)| path_is_hard(**hinge))
            .map(|(&hinge, &target)| (hinge, before.get(&hinge).copied().unwrap_or(0.0), target))
            .collect();
        // 開きながら閉じる手は、「先に開く・次に閉じる」の2区間に分ける。
        // 開く線か閉じる線の片方しか無い手は、従来どおり1区間のまま(`split = 1.0`)。
        let mut open_first: Vec<EdgeId> = Vec::new();
        let mut closing = 0usize;
        for (&hinge, &target) in &current {
            let start = before.get(&hinge).copied().unwrap_or(0.0);
            if target.abs() + ANGLE_PHASE_EPS < start.abs() {
                open_first.push(hinge);
            } else if start.abs() + ANGLE_PHASE_EPS < target.abs() {
                closing += 1;
            }
        }
        open_first.sort_unstable();
        let split = if open_first.is_empty() || closing == 0 {
            1.0
        } else {
            open_first.len() as f64 / (open_first.len() + closing) as f64
        };
        // この手が動かさない折り目は、折り途中でも動かさない。
        //
        // - まだ一度も指定されていない折り目は0°。両端と同じ扱いにそろえ、
        //   折り途中だけ勝手に折れないようにする。
        // - **すでに折ってある折り目を開きながら別の折り目を閉じる手**では、
        //   前の手順が決めた角のうちこの手が動かさないものも、その角で押さえる。
        //   開く手だけがこの押さえを要る理由は、**ソルバーが「頼まれた折り目を開く」
        //   代わりに「別の折り目を開く」ほうへ逃げられるのは、開く手だけ**だからである
        //   (実測: 鳥の基本形の花弁折りで、動かすはずの辺26が±180°のまま動かず、
        //   動かさないはずの辺22・辺31が±180°から∓150°まで戻っていた)。
        //   閉じるだけの手にはその逃げ道が無く、押さえると重なり順の探り経路が
        //   動かなくなる(実測: `folded-sample.ori3` の完全重なり23組のうち、
        //   閉じるだけの手まで押さえると再生と直接伝播の一致が 23組 → 12組 へ落ちた)。
        //
        // 利用者がいま1本を動かしている手(単一Simple)では、前の手順の角を
        // 押さえない。2026-08-11の利用者決定「動かしている折り目が最優先・
        // 他は自然に譲る」に従い、過去角は希望のままにして譲れるようにする。
        let hold_previous_angles =
            !current_is_direct_angle && !open_first.is_empty() && closing > 0;
        // この手が動かす折り目が集まる頂点に触れる折り目は、押さえてはいけない。
        // 頂点まわりの閉包(その頂点に集まる折り目の輪)は1本だけでは動けないので、
        // 隣の折り目が譲らないと紙がまったく動かなくなる。実際の紙でも、
        // つぶし折りは袋を開くために隣の折り目がいったん戻る。
        let active_vertices: BTreeSet<u32> = doc
            .cp
            .edges
            .iter()
            .filter(|edge| current.contains_key(&edge.id))
            .flat_map(|edge| [edge.v0, edge.v1])
            .collect();
        let touches_active_vertex: BTreeSet<EdgeId> = doc
            .cp
            .edges
            .iter()
            .filter(|edge| {
                active_vertices.contains(&edge.v0) || active_vertices.contains(&edge.v1)
            })
            .map(|edge| edge.id)
            .collect();
        let held: Vec<(EdgeId, f64)> = hinges
            .iter()
            .copied()
            .filter(|hinge| !current.contains_key(hinge))
            .filter_map(|hinge| match angles.get(&hinge) {
                None => Some((hinge, 0.0)),
                Some(&decided)
                    if hold_previous_angles && !touches_active_vertex.contains(&hinge) =>
                {
                    Some((hinge, decided))
                }
                Some(_) => None,
            })
            .collect();
        let held_hinges: BTreeSet<EdgeId> = held.iter().map(|&(hinge, _)| hinge).collect();
        let preferred = angles
            .iter()
            .filter(|(hinge, _)| !path_is_hard(**hinge) && !held_hinges.contains(hinge))
            .map(|(&hinge, &target)| {
                let start = if current.contains_key(&hinge) {
                    before.get(&hinge).copied().unwrap_or(0.0)
                } else {
                    target
                };
                (hinge, start, target)
            })
            .collect();
        Some(StepPath {
            hard,
            preferred,
            held,
            open_first,
            split,
        })
    } else {
        None
    };

    let sequence_targets: Vec<Driver> = display_angles
        .iter()
        .map(|(&hinge, &target_angle_deg)| Driver {
            hinge,
            target_angle_deg,
        })
        .collect();
    let driver_hinges = display_angles.keys().copied().collect();

    // FlatStateは平坦操作の基礎状態なので、従来どおり保存手順をexactに再生する。
    // 表示solveだけを優先度付きに変え、ここでは未指定ヒンジも0°へ固定する。
    let flat_exact = hinges
        .into_iter()
        .map(|hinge| Driver {
            hinge,
            target_angle_deg: angles.get(&hinge).copied().unwrap_or(0.0),
        })
        .collect();

    StepPlan {
        display_hard,
        display_preferred,
        sequence_targets,
        flat_exact,
        driver_hinges,
        path,
        order,
        saved_order,
        order_start,
        order_end,
        transition_order_is_authoritative,
        skipped,
        warnings,
    }
}

/// 変化しない再指定を現在操作から除く。
///
/// +180°と-180°は終点の頂点が一致しても、山谷を反転するにはいったん開いて
/// 反対側へ折り直す必要がある。途中経路と重なり順が違うため周期同値にしない。
fn same_step_angle(left: f64, right: f64) -> bool {
    if !left.is_finite() || !right.is_finite() {
        return false;
    }
    (right - left).abs() <= 1e-9
}

/// ヒンジ(ちょうど2つの異なる面が共有する辺)を辺ID昇順で返す。
/// `ori3-rigid` の `build_forest` と同じ定義。ここに載らない辺へ角度を指定すると
/// ソルバーが「折り線(2面の境)ではない」警告を出すため、0°固定の対象も同じ集合に絞る。
fn hinge_edges(faces: &[Face]) -> Vec<EdgeId> {
    let mut occ: BTreeMap<EdgeId, Vec<usize>> = BTreeMap::new();
    for (fi, f) in faces.iter().enumerate() {
        for &eid in &f.edges {
            occ.entry(eid).or_default().push(fi);
        }
    }
    occ.into_iter()
        .filter(|(_, list)| list.len() == 2 && list[0] != list[1])
        .map(|(eid, _)| eid)
        .collect()
}

#[cfg(test)]
mod tests {
    use ori3_model::{CreasePattern, DriverLine, Edge, EdgeKind, FoldStep, Paper, Vertex};

    use super::*;

    fn degree_four_document(current_kind: TechniqueKind) -> Document {
        let ray_50_x = 0.5 + 0.5 * 50f64.to_radians().cos() / 50f64.to_radians().sin();
        let ray_110_x = 0.5 + 0.5 * 110f64.to_radians().cos() / 110f64.to_radians().sin();
        let ray_240_x = 0.5 + 0.5 / 240f64.to_radians().sin().abs() * 240f64.to_radians().cos();
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        document.cp = CreasePattern {
            vertices: vec![
                Vertex {
                    id: 0,
                    pos: [0.0, 0.0],
                },
                Vertex {
                    id: 1,
                    pos: [ray_240_x, 0.0],
                },
                Vertex {
                    id: 2,
                    pos: [1.0, 0.0],
                },
                Vertex {
                    id: 3,
                    pos: [1.0, 0.5],
                },
                Vertex {
                    id: 4,
                    pos: [1.0, 1.0],
                },
                Vertex {
                    id: 5,
                    pos: [ray_50_x, 1.0],
                },
                Vertex {
                    id: 6,
                    pos: [ray_110_x, 1.0],
                },
                Vertex {
                    id: 7,
                    pos: [0.0, 1.0],
                },
                Vertex {
                    id: 8,
                    pos: [0.5, 0.5],
                },
            ],
            edges: vec![
                Edge {
                    id: 0,
                    v0: 0,
                    v1: 1,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 1,
                    v0: 1,
                    v1: 2,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 2,
                    v0: 2,
                    v1: 3,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 3,
                    v0: 3,
                    v1: 4,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 4,
                    v0: 4,
                    v1: 5,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 5,
                    v0: 5,
                    v1: 6,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 6,
                    v0: 6,
                    v1: 7,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 7,
                    v0: 7,
                    v1: 0,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 8,
                    v0: 8,
                    v1: 3,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 9,
                    v0: 8,
                    v1: 5,
                    kind: EdgeKind::Valley,
                },
                Edge {
                    id: 10,
                    v0: 8,
                    v1: 6,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 11,
                    v0: 8,
                    v1: 1,
                    kind: EdgeKind::Mountain,
                },
            ],
            next_vertex_id: 9,
            next_edge_id: 12,
        };
        document.sequence = vec![
            FoldStep {
                id: 0,
                kind: TechniqueKind::Simple,
                drivers: vec![DriverLine {
                    a: [0.5, 0.5],
                    b: [ray_50_x, 1.0],
                    target_angle_deg: 0.0,
                }],
                layer_order: None,
                alignment: None,
                finish_soft: None,
                note: String::new(),
            },
            FoldStep {
                id: 1,
                kind: current_kind,
                drivers: vec![DriverLine {
                    a: [0.5, 0.5],
                    b: [1.0, 0.5],
                    target_angle_deg: 90.0,
                }],
                layer_order: None,
                alignment: None,
                finish_soft: None,
                note: String::new(),
            },
        ];
        document
    }

    #[test]
    fn replay_plan_separates_current_previous_and_unmentioned_angles() {
        let document = degree_four_document(TechniqueKind::Simple);
        let faces = extract_faces(&document.cp);
        let plan = plan_steps(&document, &faces, 2, 1.0);

        assert_eq!(
            plan.display_hard,
            vec![Driver {
                hinge: 8,
                target_angle_deg: 90.0,
            }]
        );
        assert_eq!(plan.display_preferred, HashMap::from([(9, 0.0)]));
        assert_eq!(
            plan.sequence_targets,
            vec![
                Driver {
                    hinge: 8,
                    target_angle_deg: 90.0,
                },
                Driver {
                    hinge: 9,
                    target_angle_deg: 0.0,
                },
            ]
        );
        assert!(
            [10, 11].into_iter().all(|hinge| {
                !plan.display_preferred.contains_key(&hinge)
                    && !plan.display_hard.iter().any(|driver| driver.hinge == hinge)
            }),
            "一度も明示していないヒンジはfree"
        );
        let simple_path = plan.path.as_ref().expect("角度が変わるcurrent pathがある");
        assert_eq!(simple_path.hard, vec![(8, 0.0, 90.0)]);

        let pose_document = degree_four_document(TechniqueKind::Pose);
        let pose_faces = extract_faces(&pose_document.cp);
        let pose = plan_steps(&pose_document, &pose_faces, 2, 1.0);
        assert!(pose.display_hard.is_empty());
        assert_eq!(pose.display_preferred, HashMap::from([(8, 90.0), (9, 0.0)]));
        assert!(pose.path.as_ref().unwrap().hard.is_empty());

        let petal_document = degree_four_document(TechniqueKind::Petal);
        let petal_faces = extract_faces(&petal_document.cp);
        let petal = plan_steps(&petal_document, &petal_faces, 2, 1.0);
        assert!(
            petal.path.as_ref().unwrap().hard.is_empty(),
            "複合技法のsurface probeは完了時も途中再生と同じpreferred契約"
        );
    }

    /// **この手が動かさない折り目**は、折り途中でも動かさない。
    ///
    /// 折り筋を先に全部引いてから畳む展開図では、まだ折っていない折り目も
    /// 展開図の上に存在する。`extract_faces` はそこで面を分けるので、放っておくと
    /// その辺が曲がるちょうつがいとして数えられ、**まだ折っていない場所が
    /// 折り途中だけ勝手に折れて**紙が別の紙を突き抜ける。両端(`t=0` / `t=1`)は
    /// [`StepPlan::flat_exact`] が0°で固定しているので、折り途中だけが食い違っていた。
    ///
    /// 記録した技法の再生では、**前の手順が決めた角のうちこの手が動かさないもの**も
    /// 同じ理由で押さえる。押さえないと、ソルバーは動かすはずの折り目ではなく
    /// 前の手順で折った折り目のほうを開いて逃げ道にしていた。
    ///
    /// solveを通さない分類だけの検査で、角度は引数で明示して与えている。
    /// 実際の姿勢での効き目は
    /// `crates/ori3-layers/tests/precrease_pose.rs` が確かめる。
    #[test]
    fn creases_this_step_does_not_fold_are_held_while_folding() {
        let document = degree_four_document(TechniqueKind::Squash);
        let faces = extract_faces(&document.cp);
        let plan = plan_steps(&document, &faces, 2, 0.5);
        let path = plan.path.expect("角度が変わるcurrent pathがある");

        assert_eq!(
            path.held,
            vec![(10, 0.0), (11, 0.0)],
            "押さえるのは、どの手順も指定していない辺10・辺11だけ。             辺9は手順1が決めた角だが、動かす辺8と同じ頂点に集まるので押さえない"
        );
        assert!(
            path.preferred.iter().any(|&(hinge, _, _)| hinge == 9),
            "動かす折り目と同じ頂点に集まる折り目は、譲れるように希望のまま残す"
        );
        let held_now: Vec<(EdgeId, f64)> = path
            .drivers_at(0.5)
            .into_iter()
            .filter(|driver| path.held.iter().any(|&(id, _)| id == driver.hinge))
            .map(|driver| (driver.hinge, driver.target_angle_deg))
            .collect();
        assert_eq!(
            held_now,
            path.held,
            "折り道のどこでも同じ角のまま(押さえる角は`t`で動かない)"
        );
        assert_eq!(
            path.drivers_at(0.5).len(),
            path.hard.len() + path.held.len(),
            "押さえる角は、手が動かす角に足すだけで、置き換えない"
        );
        assert!(
            path.held
                .iter()
                .all(|(hinge, _)| !path.hard.iter().any(|(id, _, _)| id == hinge)),
            "押さえる角と、手が動かす角は重ならない"
        );
        assert!(
            path.held
                .iter()
                .all(|(hinge, _)| !path.preferred.iter().any(|(id, _, _)| id == hinge)),
            "押さえる角は希望角と重ならない(同じ折り目を2通りの優先度で渡さない)"
        );
    }

    /// **利用者がいま1本を動かしている手**では、前の手順の角を押さえない。
    ///
    /// 2026-08-11の利用者決定:「いま動かしている折り目は指定どおりの角になることを
    /// 最優先し、前の手順で決めた角を含む他の折り目は、紙のつながりが成立するよう
    /// 自然に引っ張られて動く」。前の手順の角を「動かせない条件」にすると、
    /// 両立しなくなったときに解が消えて3Dが動かなくなる不具合を利用者が実機で踏んだ。
    ///
    /// 押さえてよいのは、**どの手順もまだ一度も指定していない折り目**だけである。
    /// solveを通さない分類だけの検査で、角度は引数で明示して与えている。
    #[test]
    fn a_single_line_drag_still_lets_earlier_angles_yield() {
        let document = degree_four_document(TechniqueKind::Simple);
        let faces = extract_faces(&document.cp);
        let plan = plan_steps(&document, &faces, 2, 0.5);
        let path = plan.path.expect("角度が変わるcurrent pathがある");

        assert_eq!(path.hard, vec![(8, 0.0, 90.0)], "動かしている折り目はhard");
        assert_eq!(
            path.held,
            vec![(10, 0.0), (11, 0.0)],
            "押さえるのは、どの手順も指定していない辺10・辺11だけ"
        );
        assert!(
            path.preferred.iter().any(|&(hinge, _, _)| hinge == 9),
            "手順1が決めた辺9の角は希望のまま。譲れるようにしておく"
        );
    }

    /// 縦の折り目3本が、頂点を1つも共有しない展開図。
    ///
    /// 手順1が左(辺10)と真ん中(辺11)を180°へ折り、
    /// 手順2が真ん中(辺11)を0°へ**開きながら**右(辺12)を180°へ**閉じる**。
    /// 3本とも端点を共有しないので、手順2から見た辺10は
    /// 「この手が動かす頂点に触れない、前の手順が決めた角」になる。
    fn three_separate_creases_document(current_kind: TechniqueKind) -> Document {
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        document.cp = CreasePattern {
            vertices: vec![
                Vertex {
                    id: 0,
                    pos: [0.0, 0.0],
                },
                Vertex {
                    id: 1,
                    pos: [0.25, 0.0],
                },
                Vertex {
                    id: 2,
                    pos: [0.5, 0.0],
                },
                Vertex {
                    id: 3,
                    pos: [0.75, 0.0],
                },
                Vertex {
                    id: 4,
                    pos: [1.0, 0.0],
                },
                Vertex {
                    id: 5,
                    pos: [1.0, 1.0],
                },
                Vertex {
                    id: 6,
                    pos: [0.75, 1.0],
                },
                Vertex {
                    id: 7,
                    pos: [0.5, 1.0],
                },
                Vertex {
                    id: 8,
                    pos: [0.25, 1.0],
                },
                Vertex {
                    id: 9,
                    pos: [0.0, 1.0],
                },
            ],
            edges: vec![
                Edge {
                    id: 0,
                    v0: 0,
                    v1: 1,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 1,
                    v0: 1,
                    v1: 2,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 2,
                    v0: 2,
                    v1: 3,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 3,
                    v0: 3,
                    v1: 4,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 4,
                    v0: 4,
                    v1: 5,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 5,
                    v0: 5,
                    v1: 6,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 6,
                    v0: 6,
                    v1: 7,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 7,
                    v0: 7,
                    v1: 8,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 8,
                    v0: 8,
                    v1: 9,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 9,
                    v0: 9,
                    v1: 0,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 10,
                    v0: 1,
                    v1: 8,
                    kind: EdgeKind::Valley,
                },
                Edge {
                    id: 11,
                    v0: 2,
                    v1: 7,
                    kind: EdgeKind::Valley,
                },
                Edge {
                    id: 12,
                    v0: 3,
                    v1: 6,
                    kind: EdgeKind::Valley,
                },
            ],
            next_vertex_id: 10,
            next_edge_id: 13,
        };
        document.sequence = vec![
            FoldStep {
                id: 0,
                kind: TechniqueKind::Simple,
                drivers: vec![
                    DriverLine {
                        a: [0.25, 0.0],
                        b: [0.25, 1.0],
                        target_angle_deg: 180.0,
                    },
                    DriverLine {
                        a: [0.5, 0.0],
                        b: [0.5, 1.0],
                        target_angle_deg: 180.0,
                    },
                ],
                layer_order: None,
                alignment: None,
                finish_soft: None,
                note: String::new(),
            },
            FoldStep {
                id: 1,
                kind: current_kind,
                drivers: vec![
                    DriverLine {
                        a: [0.5, 0.0],
                        b: [0.5, 1.0],
                        target_angle_deg: 0.0,
                    },
                    DriverLine {
                        a: [0.75, 0.0],
                        b: [0.75, 1.0],
                        target_angle_deg: 180.0,
                    },
                ],
                layer_order: None,
                alignment: None,
                finish_soft: None,
                note: String::new(),
            },
        ];
        document
    }

    /// **すでに折ってある折り目を開きながら閉じる手**では、
    /// この手が動かす折り目と頂点を共有しない前の手順の角を押さえる。
    ///
    /// 押さえないと、ソルバーは頼まれた折り目を開く代わりに
    /// **前の手順で折った別の折り目のほうを開いて**逃げ道にしていた
    /// (実測: 鳥の基本形の花弁折りで、動かすはずの辺26が±180°のまま動かず、
    /// 動かさないはずの辺22・辺31が±180°から∓150°まで戻り、
    /// 折り途中の自己交差が1組・3組 残っていた)。
    ///
    /// solveを通さない分類だけの検査で、角度は引数で明示して与えている。
    #[test]
    fn earlier_angles_away_from_an_opening_step_are_held_while_folding() {
        let document = three_separate_creases_document(TechniqueKind::Petal);
        let faces = extract_faces(&document.cp);
        assert_eq!(faces.len(), 4, "縦の折り目3本で4面に分かれる");
        let plan = plan_steps(&document, &faces, 2, 0.5);
        let path = plan.path.expect("角度が変わるcurrent pathがある");

        assert_eq!(
            path.open_first,
            vec![11],
            "手順2が開くのは、手順1が180°にした真ん中の辺11"
        );
        assert_eq!(
            path.held,
            vec![(10, 180.0)],
            "手順1が180°と決めた辺10は、手順2が動かす辺11・辺12と頂点を共有しないので押さえる"
        );
        assert_eq!(
            path.preferred
                .iter()
                .map(|&(hinge, _, _)| hinge)
                .collect::<Vec<_>>(),
            vec![11, 12],
            "希望角に残るのは、この手が動かす辺11・辺12だけ"
        );
        assert_eq!(
            path.drivers_at(0.5),
            vec![Driver {
                hinge: 10,
                target_angle_deg: 180.0,
            }],
            "折り道のどこでも、押さえる角は決まった180°のまま"
        );
    }

    /// **閉じるだけの手**では、前の手順の角を押さえない。
    ///
    /// 閉じるだけの手には「別の折り目を開いて逃げる」道が無いので押さえる必要が無く、
    /// 押さえると重なり順の探り経路が動かなくなる。実測(`folded-sample.ori3`、
    /// 完全に重なる23組): 閉じるだけの手まで押さえると、手順の再生と直接伝播の
    /// 一致が **23組 → 12組** へ落ちた。
    #[test]
    fn a_step_that_only_closes_lets_earlier_angles_yield() {
        let mut document = three_separate_creases_document(TechniqueKind::Petal);
        // 真ん中を開く指定を外し、右を閉じるだけの手にする。
        document.sequence[1].drivers.remove(0);
        let faces = extract_faces(&document.cp);
        let plan = plan_steps(&document, &faces, 2, 0.5);
        let path = plan.path.expect("角度が変わるcurrent pathがある");

        assert!(path.open_first.is_empty(), "開く折り目が無い手である");
        assert!(
            path.held.is_empty(),
            "押さえるのは0本。前の手順の辺10・辺11の角は希望のまま残す(実際 {:?})",
            path.held
        );
        assert!(
            [10, 11]
                .into_iter()
                .all(|hinge| path.preferred.iter().any(|&(id, _, _)| id == hinge)),
            "前の手順が決めた辺10・辺11は希望に残り、譲れるようにしておく"
        );
    }

    /// 復元用の追従pathの初期値の契約。solveを通さない分類だけの検査。
    ///
    /// 直接操作角(`hard`)があるときだけ直前完成角をwarmにする。共同path targetだけの
    /// 複合手順で直前姿勢をwarmにすると、直前姿勢が新しい希望角に対しても停留点に
    /// なり得るため、経路が1度も動かないまま重なり順を決められなくなる。
    #[test]
    fn recovery_path_seeds_the_previous_pose_only_when_a_direct_angle_drives_it() {
        let previous: HashMap<EdgeId, f64> = HashMap::from([(8, 90.0)]);
        let composite = StepPath {
            hard: Vec::new(),
            preferred: vec![(8, 0.0, 180.0)],
            held: Vec::new(),
            open_first: Vec::new(),
            split: 1.0,
        };
        let direct = StepPath {
            hard: vec![(8, 0.0, 180.0)],
            preferred: vec![(9, 0.0, 0.0)],
            held: Vec::new(),
            open_first: Vec::new(),
            split: 1.0,
        };

        assert!(
            initial_surface_approach_warm(&composite, Some(&previous)).is_none(),
            "共同path targetだけの手順は、直前姿勢を最初の初期値にしない"
        );
        assert_eq!(
            initial_surface_approach_warm(&direct, Some(&previous)),
            Some(&previous),
            "直接操作角があるときは、従来どおり直前姿勢から連続に追う"
        );
        assert!(initial_surface_approach_warm(&composite, None).is_none());
        assert!(initial_surface_approach_warm(&direct, None).is_none());
    }

    /// 12分割まで進んでも、粗い3点で得た姿勢を捨てない。
    ///
    /// 上下を読み取る側は「終点にいちばん近い姿勢から順に見る」ので、
    /// 細かい12分割が後ろに来ていなければならない。
    #[test]
    fn the_finer_recovery_path_is_added_after_the_coarse_one_without_dropping_it() {
        let frame = |warning: &str| Frame3D {
            faces: Vec::new(),
            warnings: vec![warning.to_string()],
        };
        let coarse = vec![frame("coarse0"), frame("coarse1")];
        let full = vec![frame("full0"), frame("full1"), frame("full2")];
        let combined = combine_surface_paths(coarse, full);
        assert_eq!(
            combined
                .iter()
                .map(|frame| frame.warnings[0].as_str())
                .collect::<Vec<_>>(),
            vec!["coarse0", "coarse1", "full0", "full1", "full2"],
            "粗い3点を捨てず、細かい12分割を後ろへ置く"
        );
    }

    /// 複合手順の復元pathは、**画面に実際に出る折り途中の姿勢と同じ道**をたどる。
    ///
    /// 重なり順の復元に使う12分割のpath(`solve_surface_approach_full`)は、
    /// 途中再生 `replay(doc, up_to, 0.99)` が使う `solve_along` と、分割点
    /// (`0.99*i/12`)も折り道も同じである。従って初期値の契約が同じなら、
    /// 最後の点の姿勢は**同じ計算**になり、どの計算機でも一致する。
    /// 記録した数値と比べるのではなく、その場で計算した2つを比べるので、
    /// 計算機ごとの差は入らない。
    ///
    /// 復元側だけが直前完成角をwarmにしていたとき、この2つは食い違っていた。
    /// 2026-08-22の実測(度は手順1・手順2の指定角、距離は紙の幅1に対する最大頂点距離):
    ///
    /// | 手順1 / 手順2 | 直前姿勢をwarmにする(不具合) | 初期値を渡さない(修正後) |
    /// |---|---:|---:|
    /// | 135° / 180° | 2.23614e-4 | **0** |
    /// | 180° / 180° | 1.6047e-5 | **0** |
    /// | 90° / 180° | 1.51e-7 | **0** |
    ///
    /// 判定の境目 `1e-9` は、修正後の実測 0 と、最小の食い違い 1.51e-7 の
    /// あいだに 150倍の余裕を取った値である。
    #[test]
    fn the_recovery_path_matches_the_intermediate_pose_the_replay_shows() {
        // 実測した食い違いが大きい順。1件でも同じ道から外れたら不合格にする。
        for (first, current) in [(135.0, 180.0), (180.0, 180.0), (90.0, 180.0)] {
            let mut document = degree_four_document(TechniqueKind::Petal);
            document.sequence[0].drivers[0].target_angle_deg = first;
            document.sequence[1].drivers[0].target_angle_deg = current;
            let faces = extract_faces(&document.cp);
            let plan = plan_steps(&document, &faces, 2, 1.0);
            let path = plan.path.expect("角度が変わるcurrent pathがある");
            assert!(
                path.hard.is_empty(),
                "複合技法の経路は共同path targetだけ({first}° / {current}°)"
            );
            let previous = replay(&document, 1, 1.0);

            let shown = replay(&document, 2, SURFACE_APPROACH_PROGRESS[2]);
            let recovery = solve_surface_approach_full(
                &document,
                &faces,
                &path,
                Some(&previous.hinge_angles),
            );
            assert_eq!(
                recovery.len(),
                SUBSTEPS as usize,
                "12分割の全点で有限な姿勢が返る({first}° / {current}°)"
            );
            let last = recovery.last().expect("12点ある");
            let distance = max_vertex_distance(&shown.frame, last);
            assert!(
                distance < 1e-9,
                "重なり順の復元pathが、画面に出る折り途中の姿勢と別の道になっている({first}° / {current}°、最大頂点距離 {distance:.9e}、紙の幅は1)"
            );
        }
    }

    /// 同じ面の頂点どうしの、2つのFrame3D間の最大距離。
    fn max_vertex_distance(left: &Frame3D, right: &Frame3D) -> f64 {
        let right_faces = right
            .faces
            .iter()
            .map(|face| (face.face, face))
            .collect::<HashMap<_, _>>();
        left.faces
            .iter()
            .map(|face| {
                let Some(other) = right_faces.get(&face.face) else {
                    return f64::INFINITY;
                };
                if other.polygon.len() != face.polygon.len() {
                    return f64::INFINITY;
                }
                face.polygon
                    .iter()
                    .zip(&other.polygon)
                    .map(|(a, b)| {
                        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2))
                            .sqrt()
                    })
                    .fold(0.0_f64, f64::max)
            })
            .fold(0.0_f64, f64::max)
    }

    #[test]
    fn opposite_flat_endpoints_keep_the_opening_replay_path_and_reverse_the_stack() {
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        ori3_cp::insert_segment(&mut document.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Mountain);
        let step = |id, target_angle_deg| FoldStep {
            id,
            kind: TechniqueKind::Simple,
            drivers: vec![DriverLine {
                a: [0.5, 0.0],
                b: [0.5, 1.0],
                target_angle_deg,
            }],
            layer_order: None,
            alignment: None,
            finish_soft: None,
            note: String::new(),
        };
        document.sequence = vec![step(0, 180.0), step(1, -180.0)];
        let faces = extract_faces(&document.cp);
        let hinge = hinge_edges(&faces)[0];

        let endpoint_plan = plan_steps(&document, &faces, 2, 1.0);
        assert_eq!(
            endpoint_plan.display_hard,
            vec![Driver {
                hinge,
                target_angle_deg: -180.0,
            }]
        );
        assert_eq!(
            endpoint_plan.path.expect("±180°の間にも実経路がある").hard,
            vec![(hinge, 180.0, -180.0)],
            "+180°と-180°を同じ姿勢として省略しない"
        );

        let quarter_plan = plan_steps(&document, &faces, 2, 0.25);
        assert_eq!(
            quarter_plan.display_hard,
            vec![Driver {
                hinge,
                target_angle_deg: 90.0,
            }],
            "180°から-180°への1/4地点は、開く途中の+90°である"
        );

        let order = |result: &ReplayResult| {
            let mut ranked = result
                .frame
                .faces
                .iter()
                .map(|face| (face.surface_rank, face.face))
                .collect::<Vec<_>>();
            ranked.sort_unstable();
            ranked.into_iter().map(|(_, face)| face).collect::<Vec<_>>()
        };
        let positive = replay_with_faces(&document, &faces, 1, 1.0);
        let negative = replay_with_faces(&document, &faces, 2, 1.0);
        assert!(positive.surface_order_provenance.is_some());
        assert!(negative.surface_order_provenance.is_some());
        let mut reversed = order(&positive);
        reversed.reverse();
        assert_eq!(
            order(&negative),
            reversed,
            "山谷を反転した完全折りは上下も反転する"
        );
    }

    #[test]
    fn unchanged_step_preserves_complete_surface_order_and_provenance() {
        let document = degree_four_document(TechniqueKind::Simple);
        let before = replay(&document, 0, 1.0);
        let unchanged = replay(&document, 1, 1.0);
        let by_face = |result: &ReplayResult| {
            result
                .frame
                .faces
                .iter()
                .map(|face| (face.face, (face.surface_rank, face.polygon.clone())))
                .collect::<BTreeMap<_, _>>()
        };

        assert!(before.surface_order_provenance.is_some());
        assert_eq!(
            unchanged.surface_order_provenance,
            before.surface_order_provenance
        );
        assert_eq!(by_face(&unchanged), by_face(&before));
    }

    #[test]
    fn exact_endpoint_proves_canonical_surface_order_without_a_synthetic_history() {
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        ori3_cp::insert_segment(&mut document.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Mountain);
        let faces = extract_faces(&document.cp);
        let hinge = hinge_edges(&faces)[0];
        let rank_order = |frame: &Frame3D| {
            let mut ranked = frame
                .faces
                .iter()
                .map(|face| (face.surface_rank, face.face))
                .collect::<Vec<_>>();
            ranked.sort_unstable();
            ranked.into_iter().map(|(_, face)| face).collect::<Vec<_>>()
        };
        let mut observed_change = false;

        for angle in [180.0, -180.0] {
            let drivers = [Driver {
                hinge,
                target_angle_deg: angle,
            }];
            let mut previous = ori3_rigid::solve_near_exact_without_surface_order(
                &document.cp,
                &faces,
                &drivers,
                &HashMap::new(),
                None,
            );
            let before_order = rank_order(&previous.frame);
            let before_geometry = previous
                .frame
                .faces
                .iter()
                .map(|face| (face.face, face.polygon.clone(), face.layer, face.mirrored))
                .collect::<Vec<_>>();
            let expected = ori3_rigid::to_frame3d(
                &document.cp,
                &faces,
                &ori3_rigid::propagate(&document.cp, &faces, &previous.angles),
            );

            let provenance = stamp_canonical_surface_order_from_angles(
                &document,
                &faces,
                &[],
                None,
                &mut previous,
            );
            assert!(
                provenance.is_some(),
                "厳密折り目だけで唯一の重なり対を比較できるendpointはauthorityを発行する"
            );
            assert_eq!(rank_order(&previous.frame), rank_order(&expected));
            assert_eq!(
                previous
                    .frame
                    .faces
                    .iter()
                    .map(|face| (face.face, face.polygon.clone(), face.layer, face.mirrored))
                    .collect::<Vec<_>>(),
                before_geometry,
                "厳密制約の刻印はrank以外の有限候補を変えない"
            );
            observed_change |= before_order != rank_order(&expected);
        }
        assert!(
            observed_change,
            "検査fixtureの少なくとも一方向はFaceId fallbackとcanonicalを区別できる"
        );
    }

    fn assert_replay_result_bits_eq(left: &ReplayResult, right: &ReplayResult) {
        // 論理フィールド追加時に比較漏れをコンパイルエラーで検出するため`..`を使わない。
        let ReplayResult {
            frame: _,
            skipped: _,
            warnings: _,
            suspect_hinges: _,
            driver_hinges: _,
            hinge_angles: _,
            surface_order_provenance: _,
            sequence_targets: _,
            relaxations: _,
            closure_rms: _,
            best_effort: _,
            converged: _,
            layer_transition: _,
        } = left;
        let Frame3D {
            faces: _,
            warnings: _,
        } = &left.frame;
        assert_eq!(left.frame.faces.len(), right.frame.faces.len());
        for (left_face, right_face) in left.frame.faces.iter().zip(&right.frame.faces) {
            let ori3_model::Face3D {
                face: _,
                polygon: _,
                layer: _,
                surface_rank: _,
                mirrored: _,
            } = left_face;
            assert_eq!(left_face.face, right_face.face);
            assert_eq!(left_face.layer, right_face.layer);
            assert_eq!(left_face.surface_rank, right_face.surface_rank);
            assert_eq!(left_face.mirrored, right_face.mirrored);
            assert_eq!(left_face.polygon.len(), right_face.polygon.len());
            for (left_point, right_point) in left_face.polygon.iter().zip(&right_face.polygon) {
                assert_eq!(
                    left_point.map(f64::to_bits),
                    right_point.map(f64::to_bits),
                    "face {} の座標bitがcache有無で変わった",
                    left_face.face
                );
            }
        }
        assert_eq!(left.frame.warnings, right.frame.warnings);
        assert_eq!(left.skipped, right.skipped);
        assert_eq!(left.warnings, right.warnings);
        assert_eq!(left.suspect_hinges, right.suspect_hinges);
        assert_eq!(left.driver_hinges, right.driver_hinges);
        let angle_bits = |angles: &HashMap<EdgeId, f64>| {
            angles
                .iter()
                .map(|(&hinge, &angle)| (hinge, angle.to_bits()))
                .collect::<BTreeMap<_, _>>()
        };
        assert_eq!(
            angle_bits(&left.hinge_angles),
            angle_bits(&right.hinge_angles)
        );
        assert_eq!(
            left.surface_order_provenance,
            right.surface_order_provenance
        );
        assert_eq!(left.sequence_targets.len(), right.sequence_targets.len());
        for (left_driver, right_driver) in left.sequence_targets.iter().zip(&right.sequence_targets)
        {
            let Driver {
                hinge: _,
                target_angle_deg: _,
            } = left_driver;
            assert_eq!(left_driver.hinge, right_driver.hinge);
            assert_eq!(
                left_driver.target_angle_deg.to_bits(),
                right_driver.target_angle_deg.to_bits()
            );
        }
        assert_eq!(left.relaxations.len(), right.relaxations.len());
        for (left_relaxation, right_relaxation) in left.relaxations.iter().zip(&right.relaxations) {
            let ori3_rigid::AngleRelaxation {
                hinge: _,
                target_angle_deg: _,
                actual_angle_deg: _,
                delta_deg: _,
            } = left_relaxation;
            assert_eq!(left_relaxation.hinge, right_relaxation.hinge);
            assert_eq!(
                left_relaxation.target_angle_deg.to_bits(),
                right_relaxation.target_angle_deg.to_bits()
            );
            assert_eq!(
                left_relaxation.actual_angle_deg.to_bits(),
                right_relaxation.actual_angle_deg.to_bits()
            );
            assert_eq!(
                left_relaxation.delta_deg.to_bits(),
                right_relaxation.delta_deg.to_bits()
            );
        }
        assert_eq!(left.closure_rms.to_bits(), right.closure_rms.to_bits());
        assert_eq!(left.best_effort, right.best_effort);
        assert_eq!(left.converged, right.converged);
        let LayerTransition {
            start: _,
            end: _,
            progress: _,
            order_is_authoritative: _,
        } = &left.layer_transition;
        assert_eq!(left.layer_transition.start, right.layer_transition.start);
        assert_eq!(left.layer_transition.end, right.layer_transition.end);
        assert_eq!(
            left.layer_transition.progress.to_bits(),
            right.layer_transition.progress.to_bits()
        );
        assert_eq!(
            left.layer_transition.order_is_authoritative,
            right.layer_transition.order_is_authoritative
        );
    }

    #[test]
    fn completed_endpoint_cache_reuses_nested_replay_without_changing_any_result_bits() {
        let mut document = degree_four_document(TechniqueKind::Simple);
        let unchanged = document.sequence[0].clone();
        document.sequence = (0..4)
            .map(|id| {
                let mut step = unchanged.clone();
                step.id = id;
                step
            })
            .collect();
        let faces = extract_faces(&document.cp);
        let requests = [
            (document.sequence.len(), 0.25),
            (document.sequence.len(), 0.75),
        ];
        let uncached =
            requests.map(|(up_to, t)| replay_with_faces_impl(&document, &faces, up_to, t, None));

        let cache = ReplayEndpointCache::new();
        cache.clear();
        let cached_first = replay_with_faces_impl(
            &document,
            &faces,
            requests[0].0,
            requests[0].1,
            Some(&cache),
        );
        let after_first = cache.stats();
        assert_eq!(
            after_first.hits, 0,
            "cold呼出しには再利用できるendpointが無い"
        );
        assert!(
            after_first.lookups > 0,
            "途中再生が直前endpointを要求するfixture"
        );
        assert_eq!(
            after_first.stores,
            document.sequence.len(),
            "cold呼出しがup_to-1から0までの入れ子endpointを全て保存する"
        );
        let cached_second = replay_with_faces_impl(
            &document,
            &faces,
            requests[1].0,
            requests[1].1,
            Some(&cache),
        );
        let after_second = cache.stats();
        assert_eq!(
            after_second.hits,
            after_first.hits + 1,
            "別のpublic相当呼出しが同じ完成endpointを再利用する"
        );
        assert_eq!(
            after_second.stores, after_first.stores,
            "warm呼出しは完成endpointを再計算・再保存しない"
        );
        assert_eq!(
            after_second.computed_bodies - after_first.computed_bodies,
            1,
            "warm呼出しではpartial本体だけを計算し、完成endpointを解き直さない"
        );
        assert!(
            after_second.computed_bodies - after_first.computed_bodies
                < after_first.computed_bodies,
            "warm呼出しの実計算回数がcold呼出しより減る"
        );

        assert_replay_result_bits_eq(&uncached[0], &cached_first);
        assert_replay_result_bits_eq(&uncached[1], &cached_second);

        let uncached_endpoint =
            replay_with_faces_impl(&document, &faces, document.sequence.len() - 1, 1.0, None);
        let before_caller_copy = cache.stats();
        let mut caller_copy = replay_with_faces_impl(
            &document,
            &faces,
            document.sequence.len() - 1,
            1.0,
            Some(&cache),
        );
        let after_caller_copy = cache.stats();
        assert_eq!(after_caller_copy.hits, before_caller_copy.hits + 1);
        assert_eq!(
            after_caller_copy.computed_bodies, before_caller_copy.computed_bodies,
            "返却cloneの独立性検査は実際のcache hitを使う"
        );
        caller_copy.frame.faces[0].polygon[0][0] =
            f64::from_bits(caller_copy.frame.faces[0].polygon[0][0].to_bits() ^ 1);
        caller_copy.warnings.push("呼出し側だけの変更".to_owned());
        assert_ne!(
            caller_copy.frame.faces[0].polygon[0][0].to_bits(),
            uncached_endpoint.frame.faces[0].polygon[0][0].to_bits()
        );
        let endpoint_again = replay_with_faces_impl(
            &document,
            &faces,
            document.sequence.len() - 1,
            1.0,
            Some(&cache),
        );
        assert_replay_result_bits_eq(&uncached_endpoint, &endpoint_again);
    }

    #[test]
    fn completed_endpoint_cache_key_distinguishes_signed_zero_and_nan_payloads() {
        let mut document = degree_four_document(TechniqueKind::Simple);
        document.display.soft_pressure = 0.0;
        let faces = extract_faces(&document.cp);
        let snapshot = ReplayInputSnapshot::new(&document, &faces);
        assert!(snapshot.matches(&document, &faces));

        let mut negative_zero = document.clone();
        negative_zero.display.soft_pressure = -0.0;
        assert!(!snapshot.matches(&negative_zero, &faces));

        let nan_bits = 0x7ff8_0000_0000_0123;
        let mut nan_document = document.clone();
        nan_document.display.soft_pressure = f64::from_bits(nan_bits);
        let nan_snapshot = ReplayInputSnapshot::new(&nan_document, &faces);
        assert!(
            nan_snapshot.matches(&nan_document, &faces),
            "同じNaN payloadは同じ入力bitである"
        );
        let mut different_nan = nan_document;
        different_nan.display.soft_pressure = f64::from_bits(nan_bits ^ 1);
        assert!(!nan_snapshot.matches(&different_nan, &faces));

        let mut changed_faces = faces.clone();
        changed_faces.reverse();
        assert!(!snapshot.matches(&document, &changed_faces));
    }

    #[test]
    fn completed_endpoint_cache_does_not_store_zero_progress_as_current_endpoint() {
        let document = degree_four_document(TechniqueKind::Simple);
        let faces = extract_faces(&document.cp);
        let cache = ReplayEndpointCache::new();
        let zero = replay_with_faces_impl(&document, &faces, 2, 0.0, Some(&cache));
        let previous = replay_with_faces_impl(&document, &faces, 1, 1.0, None);
        assert_replay_result_bits_eq(&zero, &previous);

        let before_endpoint = cache.stats();
        let endpoint = replay_with_faces_impl(&document, &faces, 2, 1.0, Some(&cache));
        let after_endpoint = cache.stats();
        assert_eq!(
            after_endpoint.computed_bodies,
            before_endpoint.computed_bodies + 1,
            "t=0の直前結果を現在手のt=1結果として誤hitさせない"
        );
        let uncached_endpoint = replay_with_faces_impl(&document, &faces, 2, 1.0, None);
        assert_replay_result_bits_eq(&endpoint, &uncached_endpoint);
    }
}
