//! 作業21「検査済みの次手を列挙する」。列挙した手を**実際に折って**確かめる。
//!
//! ## なぜこれが要るか
//!
//! 作業18の [`crate::GenericPlanner`] は展開図だけを見て「次に折れる手」を数えた。
//! そこでは**紙の重なり順・面のめり込み・折る途中の姿勢を一切見ていない**ので、
//! 数えた手は「折れるかもしれない手」の**上限側の見積もり**だった
//! (`scratchpad/propose-18-report.md`、判断6の注意点3)。
//!
//! このモジュールは、その候補を1つずつ**本当に折ってみて**、折れたものだけを返す。
//!
//! ## 「折れた」と判断する条件(4つ。1つでも欠けたら返さない)
//!
//! | # | 見ていること | 判断のもと | 合格の値 |
//! |---|---|---|---|
//! | 1 | 平らに畳めること・紙の重なり順が保てること | [`collapse_precrease_network`] が成功し、警告が0件 | エラー0・警告0 |
//! | 2 | 紙が**裂けない**こと | [`max_seam_gap`](ori3_rigid::max_seam_gap) を途中の姿勢すべてで測る | [`MAX_SEAM_GAP`] 未満 |
//! | 3 | 紙が**すり抜けない**(めり込まない)こと | [`self_intersection_pairs`](ori3_rigid::self_intersection_pairs) を途中の姿勢すべてで数える | **0組** |
//! | 4 | **途中の姿勢が成り立つ**こと | [`replay`] を `t = 0, 1/n, …, 1` で走らせる | 飛ばした手順0・警告0・面の欠損0・座標がすべて有限 |
//!
//! 条件2〜4は**折り終わった形だけでなく、折っている途中の姿勢すべて**で見る。
//! 終点だけを見ると、途中で紙を突き抜けてから正しい形に着地する手を
//! 「折れる」と数えてしまうためである。
//!
//! ## 手の単位: 「同じ直線に乗る折り目は一緒に閉じる」
//!
//! 紙は直線に沿ってしか折れない。1本の直線の上に山と谷が混ざっていても、
//! その直線で折る動作は**1回**である。実際に折る手続き
//! ([`collapse_precrease_network`]) も、渡した2点が決める**無限直線**に乗る
//! 折り目をまとめて閉じる。
//!
//! そこでこのモジュールは、作業18の [`CreaseLine`](crate::CreaseLine)
//! (1本の直線に並ぶ、**同じ山谷**の折り目のまとまり)を、
//! **山谷を問わず同じ直線に乗るものへまとめ直した** [`FoldLine`] を手の単位にする。
//! 作業18の数え方との対応は [`FoldLine::closes`] に残してあるので、
//! 「見積もりの何本ぶんが1手だったのか」をそのまま数えられる。
//!
//! 完成探索だけは、この単線候補に加えて、未閉鎖線の全網と、現在の畳み平面で
//! 正の長さを共有する複数の [`FoldLine`] を既存折り線網として同時に閉じる。
//! 局所候補は線分端点の間に実在するactive setと、それらが正の長さで連なる最大成分であり、
//! 部分集合の全組合せではない。
//! 花弁折りのように1本ずつ閉じられない形のためで、通常の
//! [`FoldSession::verified_moves`] と作業22の単線探索は変えない。同時折りも上の
//! 4条件をすべて通す。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

use ori3_cp::{Face, extract_faces};
use ori3_layers::flat_state::{FlatState, layers_at_point, point_in_face, representative_point};
use ori3_layers::fold_through::{resolve_driver_edges, warning_means_the_fold_was_not_as_requested};
use ori3_layers::pose_motion::{
    FlatPoseMotionInput, PoseAngleTarget, PoseEdgeActivation, solve_and_apply_flat_pose_step,
};
use ori3_layers::precrease_collapse::{PrecreaseCollapseInput, collapse_precrease_network};
use ori3_layers::replay::replay;
use ori3_layers::{FoldDirection, FoldThroughInput, TechniqueInput, fold_through, petal, squash};
use ori3_model::{CreasePattern, Document, Edge, EdgeId, EdgeKind, FaceId, VertexId};
use ori3_rigid::{max_seam_gap, self_intersection_pairs};

use crate::plan::{CreaseLine, FoldedMask, MAX_LINES, crease_lines};
use crate::plan_generic::GenericPlanner;

/// 裂けたと判断する量。共有する辺の離れ具合をこの値未満に保つ。
///
/// 実測(`crates/ori3-propose/tests/enumerate.rs`、debugビルド): 折り鶴・やっこさんで
/// 確かめられた手8件の裂けは、1手だけ折ったとき最大 `2.923e-14`、
/// 続けて3手折ったときでも最大 `1.188e-13` だった。上限に対して**7桁**の余裕がある。
/// 値そのものは `crates/ori3-layers/src/pose_step.rs` が持続手順の受け入れに使う
/// `1e-6` と同じで、こちらだけ緩めていない。
pub const MAX_SEAM_GAP: f64 = 1e-6;

/// 同じ直線とみなす許容誤差。作業18の [`crate::CreaseLine`] と同じ値。
const LINE_TOL: f64 = 1e-7;

/// 2つの面の置き方が同じかを見る許容誤差。折り目が閉じているかの判定に使う。
const PLACEMENT_TOL: f64 = 1e-9;

/// 方向付き単線・層packet・つぶし／花弁の候補を作るか。
///
/// **作る。** 利用者の指示(2026-08-23)である。
///
/// # なぜ作るのか
///
/// 作らないと、**利用者は花弁折りやつぶし折りを使う折り方を一度も提案されない**。
/// 花弁折り・つぶし折りは実際の紙で普通に折る操作なので、
/// `docs/requirements-definition.md` §2 の
/// 「**実際の紙で折れる操作はすべてアプリで表現できなければならない**」に反する。
///
/// 2026-08-22 の時点では「作っても3標本のどれも良くならず、時間だけが12.6倍になる」
/// という実測を根拠に作らない設定にしていた。**その実測は正しかったが、
/// 良くならなかった理由は2つの取り違えだった**(下記)。取り違えを直したので、
/// 作ったほうが良くなる。
///
/// # 直した2つの取り違え(2026-08-23。`scratchpad/petal-tear-cause-report.md`)
///
/// 1. [`PART_LAYER_SKIP_MARK`] — `flat_motion` が**動きの部品ごと**に出す
///    「その部品に掛からない層を外した」知らせを、「折り上がりが指定と違う」と
///    誤読して候補を捨てていた。**鳥の基本形を完成させる花弁折りが、これで消えていた。**
/// 2. [`crate::search`] の `PREPARATION_TURN` — 花弁折りでできた状態が
///    「準備手の状態」として**常に後回し**にされ、状態上限12に達するまで
///    一度も広げられなかった。**粗い順位では1位に付けていた。**
///
/// # 実測(2026-08-23、最適化あり、既定12状態・分岐3・深さ8、10回)
///
/// | 標本 | 作らない | **作る(現在)** |
/// |---|---|---|
/// | 折り鶴 | `GoalReached` 5手 `[16,3,28,31,32]` 長さ0.3591209848302908 | **`GoalReached`** 6手 `[16,29,30,3,28,32]` 長さ**0.3591209848302908** |
/// | やっこさん | `GoalReached` 1手 `[8]` | **`GoalReached`** 1手 `[8]` |
/// | 鳥の基本形 | `StateCap` 長さ **0.7071067811865483**(未完成) | **`GoalReached`** 5手 `[2,13,7,154,13]` 長さ **0.3535533905932740**(完成) |
///
/// **鳥の基本形が初めて完成する。** 4つの隔たりは
/// 数 `0.000000` / 長さ `0.3535533905932740` / 太さ `0.000000000000` / 位置 `0.125000` で、
/// すべて [`CompletionTolerance::DEFAULT`](crate::search::CompletionTolerance::DEFAULT) の内側。
/// 折り鶴・やっこさんの4つの隔たりは**16桁とも作らないときと同じ**である。
///
/// # 費用(実測。10回、最適化あり)
///
/// | 標本 | 1回の探索 |
/// |---|---|
/// | 折り鶴 | 平均 **92.939秒**(最小 89.975 / 最大 99.804) |
/// | 鳥の基本形 | 平均 **4.958秒**(最小 4.467 / 最大 5.496) |
/// | やっこさん | 平均 **0.204秒** |
///
/// 作らないときは折り鶴2.213秒・鳥0.579秒・やっこ0.094秒だったので、
/// **折り鶴で約42倍**重い。上限の見直しは
/// [`SearchWatchdog::MAX_MILLIS`](crate::search::SearchWatchdog::MAX_MILLIS) と
/// `apps/desktop/src-tauri/src/commands.rs::PLAN_BUDGET` のコメントに実測つきで残した。
const WITH_EXTRA_CANDIDATES: bool = true;

/// 折る途中の姿勢を何点見るか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoseScan {
    /// 区切りの数。`t = 0, 1/steps, …, 1` の `steps + 1` 点を見る。
    pub steps: usize,
}

impl PoseScan {
    /// 既定の刻み。`t = 0, 0.05, …, 1` の21点。
    ///
    /// 21点は `scratchpad/propose-design-report.md` §9 の作業23が定める走査点数で、
    /// 折る途中を同じ細かさで見るためにここでも同じ数にした。
    pub const DEFAULT: PoseScan = PoseScan { steps: 20 };

    /// 見る点の数。
    #[must_use]
    pub fn points(self) -> usize {
        self.steps + 1
    }

    fn at(self, index: usize) -> f64 {
        if self.steps == 0 {
            1.0
        } else {
            index as f64 / self.steps as f64
        }
    }
}

/// 一度に閉じる折り線。同じ直線に乗る折り目を、山谷を問わず1つにまとめたもの。
#[derive(Clone, Debug, PartialEq)]
pub struct FoldLine {
    /// 番号(直線の並び順。同じ展開図なら毎回同じ番号になる)。
    pub id: usize,
    /// この直線に乗る折り目全体の端から端(材料座標)。
    pub a: [f64; 2],
    pub b: [f64; 2],
    /// この直線が閉じる、作業18の [`CreaseLine`] の番号(昇順)。
    pub closes: Vec<usize>,
    /// 同じものをビットで表したもの。
    pub mask: FoldedMask,
    /// この直線に乗る展開図の辺(昇順)。
    pub edges: Vec<EdgeId>,
}

/// 折れると確かめられた手1つぶん。
#[derive(Clone, Debug, PartialEq)]
pub struct VerifiedMove {
    /// [`FoldLine::id`]。複数線の同時折りだけは、その状態の通常IDの直後。
    pub id: usize,
    /// 閉じる直線の端から端(材料座標)。複数線の同時折りでは決定的な代表1本。
    pub line: [[f64; 2]; 2],
    /// 作業18の数え方で何本ぶんにあたるか。
    pub closes: Vec<usize>,
    /// 折り終わったあとに折り終えている折り線のまとまり。
    pub mask: FoldedMask,
    /// 途中の姿勢すべてを通した、裂けの最大量。[`MAX_SEAM_GAP`] 未満。
    pub max_seam_gap: f64,
    /// 途中の姿勢すべてを通した、めり込みの最大件数。**確かめた手では必ず0**。
    pub penetrations: usize,
    /// 実際に見た途中の姿勢の数。
    pub poses_checked: usize,
}

/// 探索内部でだけ使う、検証済みの手と、その検証で作った終点の組。
///
/// 2つを別々に持つと、別候補の終点を細走査へ渡せてしまう。この型はfieldを非公開にし、
/// `Clone` も実装しないことで、粗走査を通った1候補を細走査で1回だけ消費させる。
pub(crate) struct PreparedMove {
    verified: VerifiedMove,
    successor: FoldSession,
}

impl PreparedMove {
    #[must_use]
    pub(crate) fn verified(&self) -> &VerifiedMove {
        &self.verified
    }

    #[must_use]
    pub(crate) fn successor(&self) -> &FoldSession {
        &self.successor
    }

    #[must_use]
    pub(crate) fn into_parts(self) -> (VerifiedMove, FoldSession) {
        (self.verified, self.successor)
    }
}

/// 途中の姿勢が成り立たなかった中身。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoseProblem {
    /// 折り線が見つからず、手順が飛ばされた。
    StepSkipped,
    /// 再生が警告を出した。
    ReplayWarned,
    /// 面が欠けた。
    FaceLost { expected: usize, got: usize },
    /// 有限でない座標が出た。
    NotFinite,
}

/// 「折れる」と確かめられなかった理由。
///
/// **文章をそのまま持たない。** 折る手続きが返す説明文には
/// 「辺38のまわりで食い違う」のように、実行のたびに変わる番号が入る
/// (中の処理が `HashMap` をたどる順に依存するため)。これを結果へ載せると、
/// 同じ入力で3回実行しても結果が一致しなくなる。落とした理由の**種類**は
/// 毎回同じなので、種類だけを持つ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Unverified {
    /// 平らに畳めない。紙の重なり順を保ったままでは、この直線を閉じられない。
    CannotCollapse,
    /// 紙が裂ける。
    Torn { max_seam_gap: f64 },
    /// 紙が紙をすり抜ける(めり込む)。
    PaperPassesThrough { pairs: usize },
    /// 途中の姿勢が成り立たない。
    PoseFailed(PoseProblem),
}

impl Unverified {
    /// 報告用の短い名前。
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Unverified::CannotCollapse => "平らに畳めない",
            Unverified::Torn { .. } => "紙が裂ける",
            Unverified::PaperPassesThrough { .. } => "紙がすり抜ける",
            Unverified::PoseFailed(_) => "途中の姿勢が成り立たない",
        }
    }
}

/// 確かめられなかった手1つぶん。
#[derive(Clone, Debug, PartialEq)]
pub struct RejectedMove {
    pub id: usize,
    pub line: [[f64; 2]; 2],
    pub closes: Vec<usize>,
    pub reason: Unverified,
}

/// 1つの状態から次に折れる手を数えた結果。
#[derive(Clone, Debug, PartialEq)]
pub struct MoveReport {
    /// **確かめる前**の手の数。作業18の [`GenericPlanner`] が数えた
    /// [`CreaseLine`](crate::CreaseLine) の本数で、上限側の見積もりにあたる。
    pub proposed_crease_lines: usize,
    /// 同じ直線に乗るものをまとめた後の、確かめる前の手の数。
    pub proposed_fold_lines: usize,
    /// **確かめた後**に残った手。
    pub verified: Vec<VerifiedMove>,
    /// 確かめられなかった手と、その理由。
    pub rejected: Vec<RejectedMove>,
    /// 見積もりには入っていないが、確かめたら折れた手の数。
    ///
    /// 作業18の規則が**取りこぼしている**ぶんで、
    /// 「見積もりが必ず上限側になる」とは限らないことを数字で残すために測る。
    pub verified_outside_estimate: usize,
}

impl MoveReport {
    /// 確かめられなかった手の数。
    #[must_use]
    pub fn unverified(&self) -> usize {
        self.rejected.len()
    }

    /// 理由ごとの件数(理由の名前順)。
    #[must_use]
    pub fn reasons(&self) -> Vec<(&'static str, usize)> {
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        for r in &self.rejected {
            *counts.entry(r.reason.label()).or_default() += 1;
        }
        counts.into_iter().collect()
    }
}

/// 折りかけの作品1つぶん。次に折れる手を確かめながら進める。
///
/// 展開図・面・重なり順・ここまでの手順をまとめて持ち、
/// [`Self::verified_moves`] で次の手を確かめ、[`Self::apply`] で1手進める。
#[derive(Clone, Debug)]
pub struct FoldSession {
    document: Document,
    faces: Vec<Face>,
    state: FlatState,
    lines: Vec<CreaseLine>,
    fold_lines: Vec<FoldLine>,
    folded: FoldedMask,
    /// いま閉じている(両側の面が重なって折り目の角が±180°になっている)展開図の辺。
    ///
    /// [`Self::rebuild`] が [`Self::folded`] を作るのに使うものと同じ集合である。
    /// 候補づくり・開閉の判定・[`closed_effect`] も同じ答えを要るので、
    /// 同じ計算を4か所で繰り返さず、ここに1つだけ持つ。
    closed: BTreeSet<EdgeId>,
    /// 同じ状態で粗走査・21姿勢再走査・applyが繰り返す候補記述を1回だけ作る。
    network_candidates: OnceLock<Arc<[NetworkCandidate]>>,
}

/// 同じ[`FoldSession`]で折り線の姿勢線分を作るときに共有する幾何index。
///
/// 頂点・辺・面の所有関係は1状態の候補生成中には変わらない。FoldLineごとに
/// 作り直すと、葉数が多い作品ほど同じ全走査を何十回も繰り返すため、候補生成1回に
/// 1つだけ作る。`edges`は従来の線形探索と同じく、同じIDがあれば先頭を保持する。
struct FoldedSegmentGeometry<'a> {
    positions: BTreeMap<VertexId, [f64; 2]>,
    owners: BTreeMap<EdgeId, Vec<FaceId>>,
    edges: BTreeMap<EdgeId, &'a Edge>,
}

/// 探索で同じ候補集合を持つ状態を1回だけ広げるための決定的な鍵。
///
/// 折り線IDはCPの山谷や分割によって再採番されるため使わない。現在のCP、面の配置、
/// 層順、保存済みの最終ヒンジ角を、それぞれ既存の幾何許容差で量子化して持つ。
/// final CPには未来のprecreaseも含まれるため、現在0°でも一度作動した辺は
/// open-panelの境界として扱う。その集合も候補可否の一部なので鍵へ含める。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SessionStateKey {
    vertices: Vec<(VertexId, i64, i64)>,
    edges: Vec<(EdgeId, VertexId, VertexId, u8)>,
    next_ids: (VertexId, EdgeId),
    order: Vec<FaceId>,
    placements: Vec<(FaceId, bool, i64, i64, i64, i64)>,
    targets: Vec<(EdgeId, i64)>,
    activated: Vec<EdgeId>,
}

impl FoldSession {
    /// まだ折っていない作品から始める。
    ///
    /// # Errors
    ///
    /// 展開図から面を取り出せない場合。
    pub fn new(document: &Document) -> Result<Self, String> {
        let faces = extract_faces(&document.cp);
        if faces.is_empty() {
            return Err("展開図から面を取り出せなかった".to_string());
        }
        let state = FlatState::initial(&document.cp, &faces);
        let mut session = Self {
            document: document.clone(),
            faces,
            state,
            lines: Vec::new(),
            fold_lines: Vec::new(),
            folded: 0,
            closed: BTreeSet::new(),
            network_candidates: OnceLock::new(),
        };
        session.rebuild();
        Ok(session)
    }

    /// いまの作品(ここまでの手順を含む)。
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.document
    }

    /// いまの展開図から取り出した面。
    #[must_use]
    pub fn faces(&self) -> &[Face] {
        &self.faces
    }

    /// 作業18の数え方での折り線のまとまり。
    #[must_use]
    pub fn crease_lines(&self) -> &[CreaseLine] {
        &self.lines
    }

    /// 一度に閉じる折り線(このモジュールの手の単位)。
    #[must_use]
    pub fn fold_lines(&self) -> &[FoldLine] {
        &self.fold_lines
    }

    /// もう閉じている折り線のまとまり。
    #[must_use]
    pub fn folded_mask(&self) -> FoldedMask {
        self.folded
    }

    /// CPや層順が同じ物理状態を、違う手順履歴から二重に探索しないための鍵。
    pub(crate) fn state_key(&self) -> SessionStateKey {
        let mut vertices = self
            .document
            .cp
            .vertices
            .iter()
            .map(|vertex| {
                (
                    vertex.id,
                    quantize_geometry(vertex.pos[0], LINE_TOL),
                    quantize_geometry(vertex.pos[1], LINE_TOL),
                )
            })
            .collect::<Vec<_>>();
        vertices.sort_unstable_by_key(|vertex| vertex.0);

        let mut edges = self
            .document
            .cp
            .edges
            .iter()
            .map(|edge| {
                let kind = match edge.kind {
                    EdgeKind::Border => 0,
                    EdgeKind::Mountain => 1,
                    EdgeKind::Valley => 2,
                    EdgeKind::Aux => 3,
                };
                (edge.id, edge.v0, edge.v1, kind)
            })
            .collect::<Vec<_>>();
        edges.sort_unstable_by_key(|edge| edge.0);

        let mut placements = self
            .state
            .placements
            .iter()
            .map(|(&face, placement)| {
                let (sin, cos) = placement.rotation.sin_cos();
                (
                    face,
                    placement.mirrored,
                    quantize_geometry(cos, PLACEMENT_TOL),
                    quantize_geometry(sin, PLACEMENT_TOL),
                    quantize_geometry(placement.translation.x, PLACEMENT_TOL),
                    quantize_geometry(placement.translation.y, PLACEMENT_TOL),
                )
            })
            .collect::<Vec<_>>();
        placements.sort_unstable_by_key(|placement| placement.0);

        SessionStateKey {
            vertices,
            edges,
            next_ids: (
                self.document.cp.next_vertex_id,
                self.document.cp.next_edge_id,
            ),
            order: self.state.order.clone(),
            placements,
            targets: saved_angle_targets(&self.document).into_iter().collect(),
            activated: activated_edges(&self.document).into_iter().collect(),
        }
    }

    /// ここまでに進めた手の数。
    #[must_use]
    pub fn applied_moves(&self) -> usize {
        self.document.sequence.len()
    }

    /// 次に折れる手を列挙し、1つずつ実際に折って確かめる。
    ///
    /// 返すのは**確かめられた手だけ**である。確かめられなかったものは
    /// [`MoveReport::rejected`] に理由とともに入り、[`MoveReport::verified`] には入らない。
    #[must_use]
    pub fn verified_moves(&self, scan: PoseScan) -> MoveReport {
        let planner = GenericPlanner::new(&self.document.cp);
        let proposed_bits = planner
            .next_moves(self.folded)
            .into_iter()
            .fold(0 as FoldedMask, |acc, bit| acc | bit);
        let proposed_crease_lines = proposed_bits.count_ones() as usize;

        let mut proposed_fold_lines = 0usize;
        let mut verified = Vec::new();
        let mut rejected = Vec::new();
        let mut verified_outside_estimate = 0usize;
        for fold_line in &self.fold_lines {
            if fold_line.mask & !self.folded == 0 {
                continue; // すべて折り終えている
            }
            let in_estimate = fold_line.mask & proposed_bits != 0;
            if in_estimate {
                proposed_fold_lines += 1;
            }
            match self.try_fold(fold_line, scan) {
                Ok(mv) => {
                    if in_estimate {
                        verified.push(mv);
                    } else {
                        verified_outside_estimate += 1;
                    }
                }
                Err(reason) => {
                    if in_estimate {
                        rejected.push(RejectedMove {
                            id: fold_line.id,
                            line: [fold_line.a, fold_line.b],
                            closes: fold_line.closes.clone(),
                            reason,
                        });
                    }
                }
            }
        }
        MoveReport {
            proposed_crease_lines,
            proposed_fold_lines,
            verified,
            rejected,
            verified_outside_estimate,
        }
    }

    /// 手を**1つだけ**、指定した細かさで確かめ直す。
    ///
    /// [`Self::verified_moves`] は候補を全部確かめるので、細かく見るほど重くなる。
    /// 「ざっと見て順位を付け、選んだ手だけを細かく確かめ直す」という使い方が
    /// できるように、1手だけを確かめる道を開けてある(作業22の探索が使う)。
    ///
    /// 見る条件は [`Self::verified_moves`] とまったく同じで、
    /// 確かめられなければ [`None`] を返す。
    #[must_use]
    pub fn verify_move(&self, id: usize, scan: PoseScan) -> Option<VerifiedMove> {
        self.prepare_move(id, scan)
            .map(|prepared| prepared.into_parts().0)
    }

    /// 探索内部向け。指定走査を通した手と、その検証で既に作った終点を一緒に返す。
    /// 同じsolverを直後の採点・子状態作成で再実行しないためのもので、公開手順は従来どおり
    /// [`VerifiedMove`] だけを持ち、最終検証では独立に再適用する。
    pub(crate) fn prepare_move(&self, id: usize, scan: PoseScan) -> Option<PreparedMove> {
        if id >= self.fold_lines.len() {
            return self
                .network_candidates()
                .iter()
                .find(|network| network.id == id)
                .and_then(|network| self.try_network_prepared(network, scan).ok());
        }
        let fold_line = self.fold_lines.iter().find(|line| line.id == id)?;
        if fold_line.mask & !self.folded == 0 {
            return None;
        }
        self.try_fold_prepared(fold_line, scan).ok()
    }

    /// 粗走査を通った候補の終点へ、細走査の全姿勢を最初から適用し直す。
    ///
    /// collapse・網実行・終点生成は走査点に依存しないため繰り返さない。一方、裂け・めり込み・
    /// 有限性は粗走査の結果を流用せず、`scan` が指定する全点で改めて確認する。
    pub(crate) fn reverify_prepared_move(
        &self,
        prepared: PreparedMove,
        scan: PoseScan,
    ) -> Result<PreparedMove, Unverified> {
        let (max_seam_gap, penetrations) =
            self.verify_successor_poses(&prepared.successor, scan)?;
        let PreparedMove {
            mut verified,
            successor,
        } = prepared;
        verified.max_seam_gap = max_seam_gap;
        verified.penetrations = penetrations;
        verified.poses_checked = scan.points();
        Ok(PreparedMove {
            verified,
            successor,
        })
    }

    /// まだ閉じていない複数の直線を、1つの既存折り線網として同時に閉じられるか確かめる。
    ///
    /// 花弁折りのように、交差する折り線を順番に1本ずつ閉じると途中で行き止まる形を
    /// [`collapse_precrease_network`] 本来の複数線入力で扱う。単一直線の候補が1本以下なら
    /// 同じ手を重複して返さない。返すIDは通常の直線IDの直後で、同じ状態なら決定的である。
    #[must_use]
    pub fn verify_network_move(&self, scan: PoseScan) -> Option<VerifiedMove> {
        self.remaining_network()
            .map(|network| self.try_network(&network, scan))
            .and_then(Result::ok)
    }

    /// 完成探索で使う、複数直線の候補をすべて確かめる。
    ///
    /// 先頭は従来どおり「未閉鎖線の全体」で、その後ろに、現在の畳み平面で
    /// 同一直線へ重なる折り線群を並べる。全網を先頭に保ち、通った候補をすべて返す。
    #[must_use]
    pub fn verified_network_moves(&self, scan: PoseScan) -> Vec<VerifiedMove> {
        self.prepared_network_moves_until(scan, || false)
            .0
            .into_iter()
            .map(|prepared| prepared.into_parts().0)
            .collect()
    }

    /// 複数直線候補を1件ずつ確かめ、候補間でだけ打ち切る。
    ///
    /// `should_stop` が真になっても、実行中の1候補を途中で捨てない。途中姿勢の確認を
    /// 中断して未確認の手を返すことを避け、完了した候補だけを返すためである。第2要素は
    /// 全候補を見る前に打ち切ったかを表す。
    pub(crate) fn prepared_network_moves_until(
        &self,
        scan: PoseScan,
        mut should_stop: impl FnMut() -> bool,
    ) -> (Vec<PreparedMove>, bool) {
        self.prepared_network_moves_filtered(scan, None, &mut should_stop)
    }

    /// 完成探索向け。向き付き単線は、同じ状態で通常単線を安全に閉じられた
    /// FoldLineを先に検証し、残りも期限まで検証する。
    ///
    /// 通常collapseと「残す側・上/下」を明示したfold-throughの安全性は同値ではない。
    /// 前者の失敗を後者の除外条件にはせず、ここでは高価な姿勢走査の順番だけを絞る。
    pub(crate) fn prepared_completion_moves_until(
        &self,
        scan: PoseScan,
        preferred_directional_parents: &BTreeSet<usize>,
        mut should_stop: impl FnMut() -> bool,
    ) -> (Vec<PreparedMove>, bool) {
        self.prepared_network_moves_filtered(
            scan,
            Some(preferred_directional_parents),
            &mut should_stop,
        )
    }

    fn prepared_network_moves_filtered(
        &self,
        scan: PoseScan,
        preferred_directional_parents: Option<&BTreeSet<usize>>,
        should_stop: &mut impl FnMut() -> bool,
    ) -> (Vec<PreparedMove>, bool) {
        if self.network_candidates.get().is_none() {
            let (candidates, timed_out) = self.build_network_candidates_until(should_stop);
            if timed_out {
                return (Vec::new(), true);
            }
            let _ = self.network_candidates.set(Arc::from(candidates));
        }
        let mut verified = Vec::new();
        // 優先親以外のdirectionalだけを第2巡へ送る。None候補・packet技法の既存順は
        // 変えず、通常collapseが失敗した親も時間が残れば必ず調べる。
        let passes = if preferred_directional_parents.is_some() {
            2
        } else {
            1
        };
        for deferred_pass in 0..passes {
            for network in self.network_candidates() {
                let deferred = preferred_directional_parents.is_some_and(|preferred| {
                    matches!(
                        &network.key,
                        CandidateKey::FoldThrough { line_id, .. }
                            if !preferred.contains(line_id)
                    )
                });
                if usize::from(deferred) != deferred_pass {
                    continue;
                }
                if should_stop() {
                    return (verified, true);
                }
                if let Ok(prepared) = self.try_network_prepared(network, scan) {
                    verified.push(prepared);
                }
                if should_stop() {
                    return (verified, true);
                }
            }
        }
        let timed_out = should_stop();
        (verified, timed_out)
    }

    /// 現在の状態で作る複数直線候補の数（全網1件を含む）。
    #[must_use]
    pub fn network_move_count(&self) -> usize {
        self.network_candidates().len()
    }

    /// 手を**1つだけ**確かめ、確かめられなかった**理由まで**返す。
    ///
    /// [`Self::verify_move`] は理由を捨てて [`None`] にしてしまう。
    /// 全手順を通した検証(作業23 [`crate::verify`])は
    /// 「何手目の、どの確認で落ちたか」を利用者へ伝える必要があるので、
    /// 理由を残したまま受け取る道を開ける。
    ///
    /// 返り値の外側の [`None`] は「その番号の折り線が、いまの展開図に無い」か
    /// 「その折り線はもう全部折り終えている」という**手そのものが選べない**場合で、
    /// 内側の [`Err`] は「手は選べるが折れない」場合である。
    /// 見る条件は [`Self::verified_moves`] とまったく同じ。
    #[must_use]
    pub fn check_move(
        &self,
        id: usize,
        scan: PoseScan,
    ) -> Option<Result<VerifiedMove, Unverified>> {
        if id >= self.fold_lines.len() {
            return self
                .network_candidates()
                .iter()
                .find(|network| network.id == id)
                .map(|network| {
                    self.try_network_prepared(network, scan)
                        .map(|prepared| prepared.into_parts().0)
                });
        }
        let fold_line = self.fold_lines.iter().find(|l| l.id == id)?;
        if fold_line.mask & !self.folded == 0 {
            return None; // すべて折り終えている
        }
        Some(
            self.try_fold_prepared(fold_line, scan)
                .map(|prepared| prepared.into_parts().0),
        )
    }

    /// その番号の折り線が、いまの展開図にあるか。
    ///
    /// [`Self::check_move`] が [`None`] を返したとき、
    /// 「番号が無い」のか「もう折り終えている」のかを見分けるために使う。
    #[must_use]
    pub fn has_fold_line(&self, id: usize) -> bool {
        self.fold_lines.iter().any(|l| l.id == id)
            || self
                .network_candidates()
                .iter()
                .any(|network| network.id == id)
    }

    /// 候補が局所層packetから作られた操作か。
    ///
    /// 完成検査で「全層の旧候補だけで偶然通った」のではなく、対象層の区別を実際に
    /// 使った手順であることを固定するための読み取り情報である。
    #[must_use]
    pub fn move_uses_layer_packet(&self, id: usize) -> bool {
        self.network_candidates()
            .iter()
            .find(|candidate| candidate.id == id)
            .is_some_and(|candidate| match &candidate.key {
                CandidateKey::Collapse {
                    line_ids,
                    packet_edges,
                } => !line_ids.is_empty() && packet_edges.is_some(),
                // FlatPoseは露出packetからdriverを導くが、実行入力にはFaceIdを渡さない。
                // 対象層そのものを使うCollapse/Techniqueだけをpacket利用として数える。
                CandidateKey::FlatPose { .. } => false,
                CandidateKey::FoldThrough { packet, .. } => packet.is_some(),
                CandidateKey::Technique { .. } => true,
            })
    }

    /// 候補が、既閉鎖線を0°へ開きながら別の線を±180°へ閉じる操作か。
    #[must_use]
    pub fn move_opens_and_closes(&self, id: usize) -> bool {
        self.network_candidates()
            .iter()
            .find(|candidate| candidate.id == id)
            .is_some_and(|candidate| match &candidate.key {
                CandidateKey::FlatPose { targets, .. } => {
                    targets.iter().any(|(_, target)| *target == 0)
                        && targets.iter().any(|(_, target)| *target != 0)
                }
                CandidateKey::Technique { kind, .. } => *kind == PacketTechnique::Petal,
                CandidateKey::Collapse { .. } | CandidateKey::FoldThrough { .. } => false,
            })
    }

    /// 候補が、同じ閉鎖maskの層を持ち替えるために既存ヒンジを再作動する操作か。
    ///
    /// 0°を含む開き直しとは分けて数える。参照のつぶし折りのように、幾何だけでは
    /// 点数が変わらない準備手を探索が実際に残したことを受け入れ検査で確かめるためである。
    #[must_use]
    pub fn move_reactivates_layer_packet(&self, id: usize) -> bool {
        self.network_candidates()
            .iter()
            .find(|candidate| candidate.id == id)
            .is_some_and(|candidate| match &candidate.key {
                CandidateKey::FlatPose { .. } => false,
                CandidateKey::Technique { kind, .. } => *kind == PacketTechnique::Squash,
                CandidateKey::Collapse { .. } | CandidateKey::FoldThrough { .. } => false,
            })
    }

    /// 生成済みの子が、閉じた材料ヒンジを開いたか／開いたヒンジを閉じたか。
    ///
    /// 候補の名前ではなく、保存される最後の手と現在の閉鎖辺を照合する。探索の
    /// `Reopen` 枠を、実際には開閉しないPetal入力が消費しないための内部判定である。
    pub(crate) fn transition_edge_changes(&self, successor: &Self) -> (bool, bool) {
        let before_closed = &self.closed;
        let Some(step) = successor.document.sequence.last() else {
            return (false, false);
        };
        let mut opens_closed = false;
        let mut closes_open = false;
        for driver in &step.drivers {
            if !driver.target_angle_deg.is_finite() {
                continue;
            }
            let before_edges = resolve_driver_edges(&self.document.cp, driver);
            let after_edges = resolve_driver_edges(&successor.document.cp, driver);
            if driver.target_angle_deg.abs() <= LINE_TOL {
                opens_closed |= before_edges.iter().any(|edge| before_closed.contains(edge));
            } else if (driver.target_angle_deg.abs() - 180.0).abs() <= LINE_TOL
                && !after_edges.is_empty()
            {
                closes_open |= before_edges.is_empty()
                    || before_edges
                        .iter()
                        .any(|edge| !before_closed.contains(edge));
            }
        }
        (opens_closed, closes_open)
    }

    /// 候補が、残す半平面と上／下を明示した方向付きfold-throughか。
    #[must_use]
    pub fn move_is_directional_fold(&self, id: usize) -> bool {
        self.network_candidates()
            .iter()
            .find(|candidate| candidate.id == id)
            .is_some_and(|candidate| {
                matches!(
                    &candidate.key,
                    CandidateKey::FoldThrough { .. }
                        | CandidateKey::FlatPose {
                            directional: true,
                            ..
                        }
                )
            })
    }

    /// 確かめた手を1つ進める。
    ///
    /// # Errors
    ///
    /// その手がいまの状態で折れない場合(確かめた直後の状態から動いている場合など)。
    pub fn apply(&mut self, mv: &VerifiedMove) -> Result<(), String> {
        let (collapsed, line, affected) = if mv.id >= self.fold_lines.len() {
            let network = self
                .network_candidates()
                .iter()
                .find(|network| network.id == mv.id)
                .ok_or_else(|| "同時に閉じる折り線網が現在の状態にない".to_string())?;
            let line = network.representative;
            let affected = network.affected_closes.clone();
            (self.execute_network(network)?, line, affected)
        } else {
            let fold_line = self
                .fold_lines
                .iter()
                .find(|line| line.id == mv.id)
                .ok_or_else(|| "確認した折り線が現在の状態にない".to_string())?;
            (
                self.collapse(mv.line)?,
                [fold_line.a, fold_line.b],
                fold_line.closes.clone(),
            )
        };
        let next = self.successor(collapsed)?;
        let closes = closed_effect(&self.lines, &affected, self.folded, &next.closed);
        if mv.line != line || mv.closes != closes || mv.mask != next.folded {
            return Err("確認後に折り操作の効果が変わっている".to_string());
        }
        *self = next;
        Ok(())
    }

    /// この直線を閉じる操作を、複製の上で1回だけ行う。
    fn collapse(
        &self,
        line: [[f64; 2]; 2],
    ) -> Result<(CreasePattern, ori3_model::FoldStep), String> {
        self.collapse_lines(vec![line], None)
    }

    /// 指定した既存折り線網を同時に閉じる操作を、複製の上で1回だけ行う。
    fn collapse_lines(
        &self,
        lines: Vec<[[f64; 2]; 2]>,
        target_layers: Option<Vec<FaceId>>,
    ) -> Result<(CreasePattern, ori3_model::FoldStep), String> {
        let mut cp = self.document.cp.clone();
        let result = collapse_precrease_network(
            &mut cp,
            &self.faces,
            &self.state,
            &PrecreaseCollapseInput {
                lines,
                target_layers,
            },
        )?;
        if !result.warnings.is_empty() {
            return Err(format!("折る手続きが警告を出した: {:?}", result.warnings));
        }
        Ok((cp, result.step))
    }

    /// 同じ単線でも「どちら側を残し、層山の上/下のどちらへ回すか」を区別して折る。
    fn fold_directionally(
        &self,
        line: [[f64; 2]; 2],
        keep_side_point: [f64; 2],
        direction: FoldDirection,
        target_layers: Option<Vec<FaceId>>,
    ) -> Result<(CreasePattern, ori3_model::FoldStep), String> {
        let mut cp = self.document.cp.clone();
        let result = fold_through(
            &mut cp,
            &self.faces,
            &self.state,
            &FoldThroughInput {
                line,
                keep_side_point,
                target_layers,
                direction,
            },
        )?;
        if !result.warnings.is_empty() {
            return Err(format!(
                "向き付き単線折りが警告を出した: {:?}",
                result.warnings
            ));
        }
        Ok((cp, result.step))
    }

    /// 既存のflat-poseソルバーで、0°へ開く線と±180°へ閉じる線を1手にする。
    fn solve_flat_pose(
        &self,
        activations: Vec<PoseEdgeActivation>,
        drivers: Vec<PoseAngleTarget>,
        branch_hints: Vec<PoseAngleTarget>,
    ) -> Result<(CreasePattern, ori3_model::FoldStep), String> {
        let original_steps = self.document.sequence.len();
        let mut candidate = self.document.clone();
        solve_and_apply_flat_pose_step(
            &mut candidate,
            FlatPoseMotionInput {
                activations,
                drivers,
                branch_hints,
                note: "露出した連続層を開いて折り直す".to_string(),
            },
        )?;
        if candidate.sequence.len() != original_steps + 1 {
            return Err("flat-poseソルバーが1手だけを追加しなかった".to_string());
        }
        let step = candidate
            .sequence
            .pop()
            .ok_or_else(|| "flat-poseソルバーの手順が見つからない".to_string())?;
        Ok((candidate.cp, step))
    }

    /// 既存の汎用flat-motionで実装済みのpacket技法を、現在の層だけへ適用する。
    fn apply_packet_technique(
        &self,
        kind: PacketTechnique,
        input: TechniqueInput,
    ) -> Result<(CreasePattern, ori3_model::FoldStep), String> {
        let mut cp = self.document.cp.clone();
        let result = match kind {
            PacketTechnique::Squash => squash(&mut cp, &self.faces, &self.state, &input),
            PacketTechnique::Petal => petal(&mut cp, &self.faces, &self.state, &input),
        }?;
        // 警告が出ただけで候補を捨てない(「止めずに警告する」= CLAUDE.md §8)。
        // 技法の警告のほとんどは「指定どおりに折ったうえでの注意」で、
        // たとえば「反対向きの折り線が既にあります(折り上がりは同じですが…)」は、
        // 折り筋を先に引いた紙では必ず出るが、実際の紙では普通に折れる。
        // これで捨てていたため、鳥の基本形に要る花弁折りの候補が0件になっていた。
        //
        // 捨てるのは「**折り上がりが指定と違ってしまう**」ものだけにする。
        // 実際に折れるかどうかは、この後の21姿勢の走査
        // ([`FoldSession::verify_successor`]: 面の欠け・非有限・裂け・すり抜け)と
        // [`FoldSession::successor`] の平坦状態の警告が判定する。
        // 文面ではなく、測った形で決める。
        if let Some(blocking) = result
            .warnings
            .iter()
            .find(|warning| packet_technique_warning_is_blocking(warning))
        {
            return Err(format!("packet技法が指定どおりに折れなかった: {blocking}"));
        }
        // 折り上がった形で紙が裂けている手は、実際には折れない。ここで落とす。
        let torn = torn_creases(&cp, &result.state);
        if !torn.is_empty() {
            return Err(format!(
                "packet技法の折り上がりで紙が裂ける(折り目 {}本、いちばん離れた距離 {:.3e}): {:?}",
                torn.len(),
                torn.iter().map(|&(_, gap)| gap).fold(0.0f64, f64::max),
                torn.iter().map(|&(edge, _)| edge).collect::<Vec<_>>(),
            ));
        }
        Ok((cp, result.step))
    }

    fn execute_network(
        &self,
        network: &NetworkCandidate,
    ) -> Result<(CreasePattern, ori3_model::FoldStep), String> {
        match &network.action {
            NetworkAction::Collapse {
                lines,
                target_layers,
            } => self.collapse_lines(lines.clone(), target_layers.clone()),
            NetworkAction::FlatPose {
                activations,
                drivers,
                branch_hints,
            } => self.solve_flat_pose(activations.clone(), drivers.clone(), branch_hints.clone()),
            NetworkAction::FoldThrough {
                line,
                keep_side_point,
                direction,
                target_layers,
            } => {
                self.fold_directionally(*line, *keep_side_point, *direction, target_layers.clone())
            }
            NetworkAction::Technique { kind, input } => {
                self.apply_packet_technique(*kind, input.clone())
            }
        }
    }

    /// 生成した1手を末尾へ加え、実際の終点から面・層順・閉鎖maskを作り直す。
    fn successor(
        &self,
        (cp, mut step): (CreasePattern, ori3_model::FoldStep),
    ) -> Result<Self, String> {
        let mut document = self.document.clone();
        document.cp = cp;
        step.id = u32::try_from(document.sequence.len())
            .map_err(|_| "手順が多すぎて番号を振れない".to_string())?;
        document.sequence.push(step);
        let faces = extract_faces(&document.cp);
        if faces.is_empty() {
            return Err("折った後の展開図から面を取り出せなかった".to_string());
        }
        let (state, warnings) =
            ori3_layers::replay::flat_state_at(&document, &faces, document.sequence.len())?;
        if !warnings.is_empty() {
            return Err(format!("折った後の重なり順に警告が出た: {warnings:?}"));
        }
        let mut next = Self {
            document,
            faces,
            state,
            lines: Vec::new(),
            fold_lines: Vec::new(),
            folded: 0,
            closed: BTreeSet::new(),
            network_candidates: OnceLock::new(),
        };
        next.rebuild();
        Ok(next)
    }

    /// 1つの候補を実際に折って、4つの条件をすべて見る。
    fn try_fold(&self, fold_line: &FoldLine, scan: PoseScan) -> Result<VerifiedMove, Unverified> {
        self.try_fold_prepared(fold_line, scan)
            .map(|prepared| prepared.into_parts().0)
    }

    fn try_fold_prepared(
        &self,
        fold_line: &FoldLine,
        scan: PoseScan,
    ) -> Result<PreparedMove, Unverified> {
        let collapsed = self
            .collapse([fold_line.a, fold_line.b])
            .map_err(|_| Unverified::CannotCollapse)?;
        let successor = self
            .successor(collapsed)
            .map_err(|_| Unverified::CannotCollapse)?;
        if !self.operation_changes_state(&successor) {
            return Err(Unverified::CannotCollapse);
        }
        self.verify_successor(
            fold_line.id,
            [fold_line.a, fold_line.b],
            fold_line.closes.clone(),
            successor,
            scan,
        )
    }

    /// 複数直線を同時に閉じる候補を、単一直線と同じ4条件で確かめる。
    fn try_network(
        &self,
        network: &NetworkCandidate,
        scan: PoseScan,
    ) -> Result<VerifiedMove, Unverified> {
        self.try_network_prepared(network, scan)
            .map(|prepared| prepared.into_parts().0)
    }

    fn try_network_prepared(
        &self,
        network: &NetworkCandidate,
        scan: PoseScan,
    ) -> Result<PreparedMove, Unverified> {
        let collapsed = self
            .execute_network(network)
            .map_err(|_| Unverified::CannotCollapse)?;
        let successor = self
            .successor(collapsed)
            .map_err(|_| Unverified::CannotCollapse)?;
        if !self.operation_changes_state(&successor) {
            return Err(Unverified::CannotCollapse);
        }
        self.verify_successor(
            network.id,
            network.representative,
            network.affected_closes.clone(),
            successor,
            scan,
        )
    }

    /// CP・平坦配置・層順・保存済みヒンジ目標のどれも変えないno-opを候補にしない。
    fn operation_changes_state(&self, successor: &FoldSession) -> bool {
        if self.document.cp != successor.document.cp
            || self.state.order != successor.state.order
            || self.state.placements.len() != successor.state.placements.len()
            || self.state.placements.iter().any(|(face, placement)| {
                successor
                    .state
                    .placements
                    .get(face)
                    .is_none_or(|next| !placement.approx_eq(next, PLACEMENT_TOL))
            })
        {
            return true;
        }
        saved_angle_targets(&self.document) != saved_angle_targets(&successor.document)
    }

    /// 平らに閉じられた候補について、途中姿勢の裂け・めり込み・有限性を確認する。
    fn verify_successor(
        &self,
        id: usize,
        line: [[f64; 2]; 2],
        affected: Vec<usize>,
        successor: FoldSession,
        scan: PoseScan,
    ) -> Result<PreparedMove, Unverified> {
        let (worst_gap, worst_pairs) = self.verify_successor_poses(&successor, scan)?;
        let closes = closed_effect(&self.lines, &affected, self.folded, &successor.closed);
        let verified = VerifiedMove {
            id,
            line,
            closes,
            mask: successor.folded,
            max_seam_gap: worst_gap,
            penetrations: worst_pairs,
            poses_checked: scan.points(),
        };
        Ok(PreparedMove {
            verified,
            successor,
        })
    }

    /// 作成済みの終点を変えず、指定された途中姿勢すべての安全性だけを確かめる。
    fn verify_successor_poses(
        &self,
        successor: &FoldSession,
        scan: PoseScan,
    ) -> Result<(f64, usize), Unverified> {
        let candidate = &successor.document;
        let faces = &successor.faces;
        if let Some(problem) = face_count_problem(self.faces.len(), faces.len()) {
            return Err(Unverified::PoseFailed(problem));
        }

        let up_to = candidate.sequence.len();
        let mut worst_gap: f64 = 0.0;
        let mut worst_pairs = 0usize;
        for i in 0..scan.points() {
            let replayed = replay(candidate, up_to, scan.at(i));
            if !replayed.skipped.is_empty() {
                return Err(Unverified::PoseFailed(PoseProblem::StepSkipped));
            }
            if !replayed.warnings.is_empty() {
                return Err(Unverified::PoseFailed(PoseProblem::ReplayWarned));
            }
            if replayed.frame.faces.len() != faces.len() {
                return Err(Unverified::PoseFailed(PoseProblem::FaceLost {
                    expected: faces.len(),
                    got: replayed.frame.faces.len(),
                }));
            }
            if !replayed
                .frame
                .faces
                .iter()
                .all(|f| f.polygon.iter().flatten().all(|v| v.is_finite()))
            {
                return Err(Unverified::PoseFailed(PoseProblem::NotFinite));
            }
            let gap = max_seam_gap(&candidate.cp, faces, &replayed.frame);
            if !gap.is_finite() {
                return Err(Unverified::PoseFailed(PoseProblem::NotFinite));
            }
            worst_gap = worst_gap.max(gap);
            worst_pairs = worst_pairs.max(self_intersection_pairs(&replayed.frame).len());
        }
        if worst_gap >= MAX_SEAM_GAP {
            return Err(Unverified::Torn {
                max_seam_gap: worst_gap,
            });
        }
        if worst_pairs > 0 {
            return Err(Unverified::PaperPassesThrough { pairs: worst_pairs });
        }
        Ok((worst_gap, worst_pairs))
    }

    /// 現在まだ閉じていない直線を、決定的な順番の1つの網にまとめる。
    fn remaining_network(&self) -> Option<NetworkCandidate> {
        let ids: Vec<usize> = self
            .fold_lines
            .iter()
            .filter(|line| line.mask & !self.folded != 0)
            .map(|line| line.id)
            .collect();
        if ids.len() < 2 {
            return None;
        }
        self.collapse_candidate(ids, None, None, self.fold_lines.len())
    }

    /// 全網・同じ位置へ重なる線群・その位置で露出した連続packetを作る。
    ///
    /// 1本折ると、材料上では離れていた折り線が層の上下で同じ場所へ来る。
    /// それらは実物では1回につまむ線なので、同じ正の長さを共有するactive setと、
    /// 正の長さで連なる最大成分を候補にする。packetはつまむ区間の局所stackについて、
    /// 上端・下端からのprefix/suffixだけである。線・面の冪集合は作らない。
    fn network_candidates(&self) -> &[NetworkCandidate] {
        self.network_candidates
            .get_or_init(|| {
                let mut never_stop = || false;
                Arc::from(self.build_network_candidates_until(&mut never_stop).0)
            })
            .as_ref()
    }

    fn build_network_candidates_until(
        &self,
        should_stop: &mut impl FnMut() -> bool,
    ) -> (Vec<NetworkCandidate>, bool) {
        if should_stop() {
            return (Vec::new(), true);
        }
        let remaining: Vec<usize> = self
            .fold_lines
            .iter()
            .filter(|line| line.mask & !self.folded != 0)
            .map(|line| line.id)
            .collect();
        let Some(segment_geometry) = self.folded_segment_geometry_until(should_stop) else {
            return (Vec::new(), true);
        };
        let mut folded_segments_by_line = Vec::with_capacity(self.fold_lines.len());
        for fold_line in &self.fold_lines {
            let Some(segments) =
                self.folded_segments_until(fold_line, &segment_geometry, should_stop)
            else {
                return (Vec::new(), true);
            };
            folded_segments_by_line.push(segments);
        }
        let mut folded_segments = Vec::new();
        for (fold_line, segments) in self.fold_lines.iter().zip(&folded_segments_by_line) {
            if should_stop() {
                return (Vec::new(), true);
            }
            if fold_line.mask & !self.folded != 0 {
                folded_segments.extend(
                    segments
                        .iter()
                        .copied()
                        .map(|segment| (fold_line.id, segment)),
                );
            }
        }
        if should_stop() {
            return (Vec::new(), true);
        }
        let Some(mut groups) = coincident_line_sets_until(&folded_segments, should_stop) else {
            return (Vec::new(), true);
        };
        // 1本の直線を区間ごとにつまむactive setに加え、正の長さで連なる
        // 最大成分も1回の同時折り候補として残す。A-BとB-Cのactive set自体は
        // 置換せず、追加候補も後段のcollapse＋全姿勢検査を必ず通す。
        let Some(legacy_components) =
            coincident_line_components_until(&folded_segments, should_stop)
        else {
            return (Vec::new(), true);
        };
        groups.extend(legacy_components);

        let mut out = self.remaining_network().into_iter().collect::<Vec<_>>();
        for ids in groups
            .into_iter()
            .filter(|ids| ids.len() >= 2 && ids.len() < remaining.len())
        {
            if should_stop() {
                return (Vec::new(), true);
            }
            if let Some(candidate) = self.collapse_candidate(ids, None, None, 0) {
                out.push(candidate);
            }
        }

        if !WITH_EXTRA_CANDIDATES {
            for (ordinal, candidate) in out.iter_mut().enumerate() {
                candidate.id = self.fold_lines.len() + ordinal;
            }
            return if should_stop() {
                (Vec::new(), true)
            } else {
                (out, false)
            };
        }

        // collapseは終点配置を解くが、同じ配置へ至る「どちら側を動かすか」と
        // 「層山の上/下へ回すか」は区別しない。つぶし折り前の2手のように層順が
        // 次の可否を決める場合があるため、既存fold_throughの4通りも候補にする。
        let Some(directional) =
            self.directional_fold_candidates_until(&folded_segments_by_line, should_stop)
        else {
            return (Vec::new(), true);
        };
        out.extend(directional);

        // まだ1本も閉じていない紙には物理的な重なり層packetが無い。初手の通常単線と
        // 全網候補を保ち、同じ平面を別FaceIdで分けただけの重複packetは作らない。
        // この判定を全線分の平坦化より先に置き、既に共有済みの線分を再走査しない。
        let closed = &self.closed;
        if closed.is_empty() {
            for (ordinal, candidate) in out.iter_mut().enumerate() {
                candidate.id = self.fold_lines.len() + ordinal;
            }
            return if should_stop() {
                (Vec::new(), true)
            } else {
                (out, false)
            };
        }

        // packet候補は、既存のNone候補をすべて並べた後ろへ足す。これにより従来候補の
        // IDと順位を保ったまま、閉じた線の再作動と開き直しだけを追加できる。
        let mut all_segments = Vec::new();
        for (fold_line, segments) in self.fold_lines.iter().zip(&folded_segments_by_line) {
            if should_stop() {
                return (Vec::new(), true);
            }
            all_segments.extend(
                segments
                    .iter()
                    .copied()
                    .map(|segment| (fold_line.id, segment)),
            );
        }
        if should_stop() {
            return (Vec::new(), true);
        }
        let Some(panel_of) = self.open_panel_map_until(closed, should_stop) else {
            return (Vec::new(), true);
        };
        let mut seen = out
            .iter()
            .map(|candidate| candidate.key.clone())
            .collect::<BTreeSet<_>>();
        let Some(mut seed_cells) = coincident_line_cells_until(&all_segments, should_stop) else {
            return (Vec::new(), true);
        };
        if should_stop() {
            return (Vec::new(), true);
        }
        seed_cells.extend(all_segments.iter().map(|(id, segment)| {
            (
                vec![*id],
                [
                    (segment[0][0] + segment[1][0]) * 0.5,
                    (segment[0][1] + segment[1][1]) * 0.5,
                ],
            )
        }));
        seed_cells.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then(left.1[0].total_cmp(&right.1[0]))
                .then(left.1[1].total_cmp(&right.1[1]))
        });
        if should_stop() {
            return (Vec::new(), true);
        }
        seed_cells.dedup_by(|left, right| left.0 == right.0 && point_near(left.1, right.1));

        // 正の長さで連なるcrease graphの各最大成分につき、実在するつまみ位置の
        // local stackだけを見る。stackの冪集合ではなく、上下端から露出する連続packet
        // だけを作り、そのpacketが実際に触る成分内の線へ操作集合を射影する。
        let Some(components) = coincident_line_components_until(&all_segments, should_stop) else {
            return (Vec::new(), true);
        };
        if should_stop() {
            return (Vec::new(), true);
        }
        let mut component_of = BTreeMap::<usize, Vec<usize>>::new();
        for component in components {
            if should_stop() {
                return (Vec::new(), true);
            }
            for id in &component {
                component_of.insert(*id, component.clone());
            }
        }
        let mut component_stacks = BTreeSet::<(Vec<usize>, Vec<FaceId>)>::new();
        for (seed_ids, point) in seed_cells {
            if should_stop() {
                return (Vec::new(), true);
            }
            let Some(&first) = seed_ids.first() else {
                continue;
            };
            let component = component_of
                .get(&first)
                .cloned()
                .unwrap_or_else(|| vec![first]);
            let stack = layers_at_point(&self.document.cp, &self.faces, &self.state, point);
            if !stack.is_empty() {
                component_stacks.insert((component, stack));
            }
        }
        let mut component_packets = BTreeSet::<(Vec<usize>, Vec<FaceId>)>::new();
        let mut exposed_panels = BTreeSet::<Vec<FaceId>>::new();
        for (component, stack) in component_stacks {
            if should_stop() {
                return (Vec::new(), true);
            }
            if let Some(bottom) = stack.first()
                && let Some(panel) = panel_of.get(bottom)
            {
                exposed_panels.insert(panel.clone());
            }
            if let Some(top) = stack.last()
                && let Some(panel) = panel_of.get(top)
            {
                exposed_panels.insert(panel.clone());
            }
            let Some(raw_packets) = exposed_packets_until(&stack, should_stop) else {
                return (Vec::new(), true);
            };
            for raw_packet in raw_packets {
                let packet = expand_packet(&raw_packet, &panel_of);
                if !packet.is_empty() {
                    component_packets.insert((component.clone(), packet));
                }
            }
        }
        // 折り線の中点は境界上なので、片側の面が層照会から落ちる場合がある。各atomic faceの
        // 内部代表点でも局所stackを取り、技法用の露出panelだけを補う。panel集合で重複排除し、
        // collapse用の線集合やprefix/suffixの数は増やさない。
        for face in &self.faces {
            if should_stop() {
                return (Vec::new(), true);
            }
            let Some(placement) = self.state.placements.get(&face.id) else {
                continue;
            };
            let material = representative_point(&self.document.cp, face);
            let point = transform_material_point(
                material,
                placement.mirrored,
                placement.rotation,
                [placement.translation.x, placement.translation.y],
            );
            let stack = layers_at_point(&self.document.cp, &self.faces, &self.state, point);
            for exposed in [stack.first(), stack.last()].into_iter().flatten() {
                if let Some(panel) = panel_of.get(exposed) {
                    exposed_panels.insert(panel.clone());
                }
            }
        }
        let mut component_packets = component_packets.into_iter().collect::<Vec<_>>();
        component_packets.sort_by(|left, right| {
            right
                .0
                .len()
                .cmp(&left.0.len())
                .then(left.0.cmp(&right.0))
                .then(left.1.len().cmp(&right.1.len()))
                .then(left.1.cmp(&right.1))
        });
        if should_stop() {
            return (Vec::new(), true);
        }
        let layer_index = self
            .state
            .order
            .iter()
            .enumerate()
            .map(|(index, face)| (*face, index))
            .collect::<BTreeMap<_, _>>();
        let mut exposed_panels = exposed_panels.into_iter().collect::<Vec<_>>();
        // Squashは現在つまめる外層を1枚ずつ持ち替える準備手である。FaceId順ではなく
        // 実際のbottom→top順に並べ、同点だけFaceIdで決める。これにより未来のprecrease
        // によるFaceId採番が、どの露出panelをbranch=3へ残すかへ影響しない。
        exposed_panels.sort_by(|left, right| {
            let position = |panel: &[FaceId]| {
                panel
                    .iter()
                    .filter_map(|face| layer_index.get(face))
                    .copied()
                    .min()
                    .unwrap_or(usize::MAX)
            };
            position(left).cmp(&position(right)).then(left.cmp(right))
        });
        if should_stop() {
            return (Vec::new(), true);
        }

        for (component, packet) in component_packets {
            if should_stop() {
                return (Vec::new(), true);
            }
            let Some(relations) = self.packet_edge_relations_until(&packet, should_stop) else {
                return (Vec::new(), true);
            };
            let ids = component
                .into_iter()
                .filter(|id| self.line_touches_packet(*id, &relations))
                .collect::<Vec<_>>();
            if ids.is_empty() {
                continue;
            }
            let collapse_ids = ids
                .iter()
                .copied()
                .filter(|id| {
                    self.fold_lines[*id].mask & !self.folded != 0
                        && self.line_has_internal_packet_edge(*id, &relations, closed)
                })
                .collect::<Vec<_>>();
            let packet_edges = relations
                .iter()
                .filter_map(|(&edge, relation)| relation.internal.then_some(edge))
                .collect::<Vec<_>>();
            if let Some(candidate) =
                self.collapse_candidate(collapse_ids, Some(packet.clone()), Some(packet_edges), 0)
                && seen.insert(candidate.key.clone())
            {
                out.push(candidate);
            }

            // 既存の山谷種と反対の±180°へ閉じる枝は、通常collapseや
            // fold-throughの「山谷を既存種へ合わせる」経路では表せない。露出packet
            // が一意に決めた線集合だけを既存flat-pose solverへ渡し、符号の冪集合は
            // 列挙しない。
            if let Some(candidate) =
                self.flat_pose_candidate(&ids, &relations, closed, false, true)
                && seen.insert(candidate.key.clone())
            {
                out.push(candidate);
            }

            for inverted in [false, true] {
                if let Some(candidate) =
                    self.flat_pose_candidate(&ids, &relations, closed, true, inverted)
                    && seen.insert(candidate.key.clone())
                {
                    out.push(candidate);
                }
            }
        }

        // 技法は任意の層部分集合ではなく、局所stackのいちばん上/下で実際につまめる
        // 1枚を、まだ一度も使っていないprecrease越しに元のpanelへ戻したものだけにする。
        // final CPでは後の技法が作る線で元の1面が細分されているため、この閉包が無いと
        // `packet.len() == 1` と「packet内部の折り線」が両立せずPetal候補が0件になる。
        let mut technique_panels = Vec::new();
        for panel in exposed_panels {
            if should_stop() {
                return (Vec::new(), true);
            }
            let Some(relations) = self.packet_edge_relations_until(&panel, should_stop) else {
                return (Vec::new(), true);
            };
            let ids = self
                .fold_lines
                .iter()
                .filter(|line| self.line_touches_packet(line.id, &relations))
                .map(|line| line.id)
                .collect::<Vec<_>>();
            if ids.is_empty() {
                continue;
            }
            let Some(supports) =
                self.panel_closed_support_lines_until(&panel, closed, should_stop)
            else {
                return (Vec::new(), true);
            };
            let Some(axes) = self.panel_petal_axes_until(&panel, closed, should_stop) else {
                return (Vec::new(), true);
            };
            technique_panels.push((panel, ids, supports, axes));
        }

        // 袋のrestackは、実際に外側へ露出しているbottomから順に試す。FaceIdではなく
        // 上で確定した層順を使うため、未来のprecreaseによる面番号へ順位が依存しない。
        for (panel, ids, supports, _) in &technique_panels {
            // `squash`は閉じた袋の背を開いてrestackする既存技法である。背はpanel境界の
            // 閉鎖辺からだけ取り、同じ支持直線へ分割された辺は代表1本へまとめる。
            for &line in supports {
                for (tip, pivot) in [(line[0], line[1]), (line[1], line[0])] {
                    let reference_point = [2.0 * tip[0] - pivot[0], 2.0 * tip[1] - pivot[1]];
                    for open_to_back in [false, true] {
                        if let Some(candidate) = self.packet_technique_candidate(
                            PacketTechnique::Squash,
                            ids,
                            panel,
                            line,
                            reference_point,
                            open_to_back,
                        ) && seen.insert(candidate.key.clone())
                        {
                            out.push(candidate);
                        }
                    }
                }
            }
        }

        // `petal`のlineは閉じる3本のどれかではなく、持ち上げる先端から閉じた角へ
        // 向かう対称軸である。前面(false)はtop→bottom、背面(true)はbottom→topの
        // 物理的な露出順で試す。全panel×前後2通りは保ち、順番だけを決定的にする。
        for open_to_back in [false, true] {
            for offset in 0..technique_panels.len() {
                let index = if open_to_back {
                    offset
                } else {
                    technique_panels.len() - 1 - offset
                };
                let (panel, ids, _, axes) = &technique_panels[index];
                for &(line, reference_point) in axes {
                    if let Some(candidate) = self.packet_technique_candidate(
                        PacketTechnique::Petal,
                        ids,
                        panel,
                        line,
                        reference_point,
                        open_to_back,
                    ) && seen.insert(candidate.key.clone())
                    {
                        out.push(candidate);
                    }
                }
            }
        }
        for (ordinal, candidate) in out.iter_mut().enumerate() {
            candidate.id = self.fold_lines.len() + ordinal;
        }
        if should_stop() {
            (Vec::new(), true)
        } else {
            (out, false)
        }
    }

    fn collapse_candidate(
        &self,
        mut ids: Vec<usize>,
        target_layers: Option<Vec<FaceId>>,
        packet_edges: Option<Vec<EdgeId>>,
        id: usize,
    ) -> Option<NetworkCandidate> {
        ids.sort_unstable();
        ids.dedup();
        let first = *ids.first()?;
        let lines = ids
            .iter()
            .map(|line| [self.fold_lines[*line].a, self.fold_lines[*line].b])
            .collect::<Vec<_>>();
        let affected_closes = affected_lines(&self.fold_lines, &ids);
        Some(NetworkCandidate {
            id,
            representative: [self.fold_lines[first].a, self.fold_lines[first].b],
            // FaceId列そのものではなく、そのpacketの内側にある辺集合で重複除去する。
            // 実際のnetwork解決は複製せず、既存collapse実装だけに任せる。
            key: CandidateKey::Collapse {
                line_ids: ids,
                packet_edges,
            },
            action: NetworkAction::Collapse {
                lines,
                target_layers,
            },
            affected_closes,
        })
    }

    /// 未閉鎖FoldLineを、可動半平面2通り×重なりの上/下2通りで折る。
    ///
    /// 線・層の部分集合は列挙しない。各実在segmentにつき4候補で、同じ入力は
    /// 整数量子化したkeyにより決定的に1件へまとめる。
    fn directional_fold_candidates_until(
        &self,
        folded_segments_by_line: &[Vec<[[f64; 2]; 2]>],
        should_stop: &mut impl FnMut() -> bool,
    ) -> Option<Vec<NetworkCandidate>> {
        debug_assert_eq!(self.fold_lines.len(), folded_segments_by_line.len());
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for (fold_line, segments) in self.fold_lines.iter().zip(folded_segments_by_line) {
            if should_stop() {
                return None;
            }
            if fold_line.mask & !self.folded == 0 {
                continue;
            }
            let mut supports: Vec<[[f64; 2]; 2]> = Vec::new();
            for &segment in segments {
                if should_stop() {
                    return None;
                }
                if let Some(existing) = supports
                    .iter_mut()
                    .find(|known| supporting_lines_match(**known, segment))
                {
                    let longer = segment_length(segment) > segment_length(*existing) + LINE_TOL;
                    let same_length =
                        (segment_length(segment) - segment_length(*existing)).abs() <= LINE_TOL;
                    if longer
                        || (same_length
                            && quantized_segment(segment) < quantized_segment(*existing))
                    {
                        *existing = segment;
                    }
                } else {
                    supports.push(segment);
                }
            }
            supports.sort_by_key(|segment| quantized_segment(*segment));
            for line in supports {
                if should_stop() {
                    return None;
                }
                let delta = [line[1][0] - line[0][0], line[1][1] - line[0][1]];
                let length = delta[0].hypot(delta[1]);
                if length <= LINE_TOL {
                    continue;
                }
                let midpoint = [
                    (line[0][0] + line[1][0]) * 0.5,
                    (line[0][1] + line[1][1]) * 0.5,
                ];
                let normal = [-delta[1] / length, delta[0] / length];
                let offset = length.max(1.0) * 0.25;
                // replayの基準面（最小FaceId）がある側を先に「残す側」とする。
                // これは全体座標を固定したまま紙の反対側を動かす選択で、参照手順の
                // 中央折りと一致する。同じ実操作の反対側も2番目に必ず残す。
                let preferred_side = self
                    .faces
                    .iter()
                    .min_by_key(|face| face.id)
                    .and_then(|face| {
                        let placement = self.state.placements.get(&face.id)?;
                        let material = representative_point(&self.document.cp, face);
                        let point = transform_material_point(
                            material,
                            placement.mirrored,
                            placement.rotation,
                            [placement.translation.x, placement.translation.y],
                        );
                        let signed = (point[0] - midpoint[0]) * normal[0]
                            + (point[1] - midpoint[1]) * normal[1];
                        (signed.abs() > LINE_TOL).then_some(signed.signum())
                    })
                    .unwrap_or(-1.0);
                for side in [preferred_side, -preferred_side] {
                    let keep_side_point = [
                        midpoint[0] + side * normal[0] * offset,
                        midpoint[1] + side * normal[1] * offset,
                    ];
                    for direction in [FoldDirection::Up, FoldDirection::Down] {
                        let key = CandidateKey::FoldThrough {
                            line_id: fold_line.id,
                            line: quantized_segment(line),
                            keep_side: quantized_point(keep_side_point),
                            down: direction == FoldDirection::Down,
                            packet: None,
                        };
                        if !seen.insert(key.clone()) {
                            continue;
                        }
                        out.push(NetworkCandidate {
                            id: 0,
                            representative: line,
                            key,
                            action: NetworkAction::FoldThrough {
                                line,
                                keep_side_point,
                                direction,
                                target_layers: None,
                            },
                            affected_closes: fold_line.closes.clone(),
                        });
                    }
                }
            }
        }
        Some(out)
    }

    fn line_touches_packet(
        &self,
        id: usize,
        relations: &BTreeMap<EdgeId, PacketEdgeRelation>,
    ) -> bool {
        self.fold_lines[id]
            .edges
            .iter()
            .any(|edge| relations.contains_key(edge))
    }

    fn line_has_internal_packet_edge(
        &self,
        id: usize,
        relations: &BTreeMap<EdgeId, PacketEdgeRelation>,
        closed: &BTreeSet<EdgeId>,
    ) -> bool {
        self.fold_lines[id].edges.iter().any(|edge| {
            relations
                .get(edge)
                .is_some_and(|relation| relation.internal && !closed.contains(edge))
        })
    }

    fn packet_edge_relations_until(
        &self,
        packet: &[FaceId],
        should_stop: &mut impl FnMut() -> bool,
    ) -> Option<BTreeMap<EdgeId, PacketEdgeRelation>> {
        if should_stop() {
            return None;
        }
        let selected = packet.iter().copied().collect::<BTreeSet<_>>();
        let owners = face_owners(&self.faces);
        let positions = self
            .document
            .cp
            .vertices
            .iter()
            .map(|vertex| (vertex.id, vertex.pos))
            .collect::<BTreeMap<_, _>>();
        let mut relations = BTreeMap::new();
        for edge in &self.document.cp.edges {
            if should_stop() {
                return None;
            }
            if edge.kind == EdgeKind::Border {
                continue;
            }
            if edge.kind == EdgeKind::Aux {
                let (Some(a), Some(b)) = (positions.get(&edge.v0), positions.get(&edge.v1)) else {
                    continue;
                };
                let midpoint = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
                let mut selected_here = false;
                for face in &self.faces {
                    if should_stop() {
                        return None;
                    }
                    if selected.contains(&face.id)
                        && point_in_face(&self.document.cp, face, midpoint)
                    {
                        selected_here = true;
                        break;
                    }
                }
                if selected_here {
                    relations.insert(
                        edge.id,
                        PacketEdgeRelation {
                            internal: true,
                            boundary: false,
                        },
                    );
                }
                continue;
            }
            let Some(incident) = owners.get(&edge.id).filter(|incident| incident.len() == 2) else {
                continue;
            };
            let inside = incident
                .iter()
                .filter(|owner| selected.contains(*owner))
                .count();
            if inside > 0 {
                relations.insert(
                    edge.id,
                    PacketEdgeRelation {
                        internal: inside == 2,
                        boundary: inside == 1,
                    },
                );
            }
        }
        (!should_stop()).then_some(relations)
    }

    /// packet内の材料ヒンジを±180°へ再作動するか、境界の旧閉鎖線を0°へ開く。
    fn flat_pose_candidate(
        &self,
        ids: &[usize],
        relations: &BTreeMap<EdgeId, PacketEdgeRelation>,
        closed: &BTreeSet<EdgeId>,
        reopen_boundary: bool,
        inverted: bool,
    ) -> Option<NetworkCandidate> {
        let mut covered = BTreeSet::new();
        let mut targets = BTreeMap::<EdgeId, i8>::new();
        // FoldLineは現在すでにM/Vである。既存辺のkindを候補ごとに書き換えると、
        // 作業18のCreaseLine番号まで再採番されるため、符号はdriverだけで指定する。
        // Auxの昇格はpacket collapse側の既存処理へ任せる。
        let activations = Vec::new();
        let mut has_closed = false;
        let mut opens_boundary = false;
        let mut closes_open = false;
        let saved_targets = saved_angle_targets(&self.document);
        let mut changes_angle = false;

        for id in ids {
            for edge_id in &self.fold_lines[*id].edges {
                let Some(relation) = relations.get(edge_id).copied() else {
                    continue;
                };
                let edge = self
                    .document
                    .cp
                    .edges
                    .iter()
                    .find(|edge| edge.id == *edge_id)?;
                covered.insert(*id);
                let was_closed = closed.contains(edge_id);
                has_closed |= was_closed;
                let target = if reopen_boundary && was_closed && relation.boundary {
                    opens_boundary = true;
                    0
                } else {
                    if !was_closed {
                        closes_open = true;
                    }
                    let ordinary = if edge.kind == EdgeKind::Mountain {
                        1
                    } else {
                        -1
                    };
                    if inverted { -ordinary } else { ordinary }
                };
                let quantized_target = i64::from(target) * 180_000_000;
                // 閉じているのに保存角が無い従属ヒンジを、同じ±180°へ明示しただけでは
                // 実際の動作を証明できない。0↔±180°か、保存済み符号の反転だけを
                // 「動く」と数え、静止したまま層順だけ変える偽の手を作らない。
                changes_angle |= if target == 0 {
                    was_closed
                } else if !was_closed {
                    true
                } else {
                    saved_targets
                        .get(edge_id)
                        .is_some_and(|saved| *saved != quantized_target)
                };
                targets.insert(*edge_id, target);
            }
        }
        if covered.len() != ids.len()
            || !changes_angle
            || (!reopen_boundary && !has_closed && !inverted)
            || (reopen_boundary && !(opens_boundary && closes_open))
        {
            return None;
        }

        let mut drivers = Vec::new();
        let mut branch_hints = Vec::new();
        for (&edge_id, &target) in &targets {
            let target_angle_deg = f64::from(target) * 180.0;
            drivers.push(PoseAngleTarget {
                edge_id,
                target_angle_deg,
            });
            // exact 0/±180°は剛体解の特異点なので、branch hintには1°手前の側を渡す。
            // flat-pose実装がhard driverへ使うapproachと同じ側であり、保存角から開く
            // 0°だけは現在いる側の±1°を選ぶ。exact値をseedにすると、参照のつぶし折りを
            // 作れるdriver集合でも特異点から出られずCannotCollapseになった実測を直す。
            let branch_angle_deg = match target {
                1 => 179.0,
                -1 => -179.0,
                0 => saved_targets
                    .get(&edge_id)
                    .map_or(0.0, |saved| saved.signum() as f64),
                _ => unreachable!("flat target is represented by -1, 0, or 1"),
            };
            branch_hints.push(PoseAngleTarget {
                edge_id,
                target_angle_deg: branch_angle_deg,
            });
        }
        let nonzero_edges = targets
            .iter()
            .filter_map(|(&edge, &target)| (target != 0).then_some(edge))
            .collect::<BTreeSet<_>>();
        let mut affected_closes = self
            .lines
            .iter()
            .filter(|line| line.edges.iter().any(|edge| nonzero_edges.contains(edge)))
            .map(|line| line.id)
            .collect::<Vec<_>>();
        affected_closes.sort_unstable();
        affected_closes.dedup();
        let key_targets = targets.into_iter().collect::<Vec<_>>();
        Some(NetworkCandidate {
            id: 0,
            representative: [self.fold_lines[ids[0]].a, self.fold_lines[ids[0]].b],
            // FlatPoseへpacket自体は渡らない。同じexact driverは同じsolver呼出しなので、
            // つまみ位置やFaceId列が違っても1候補へまとめる。
            key: CandidateKey::FlatPose {
                targets: key_targets,
                directional: !reopen_boundary && inverted,
            },
            action: NetworkAction::FlatPose {
                activations,
                drivers,
                branch_hints,
            },
            affected_closes,
        })
    }

    /// final CPで将来の折り線により分割された面を、現在つまめる元のpanelへ戻す。
    ///
    /// 共有辺が現在閉じておらず、かつ探索手順でまだ一度も作動していない場合だけ渡る。
    /// 一度閉じてから0°へ開いた線は次のpacket境界として残し、未来のprecreaseだけを
    /// 面分割として吸収する。各成分内の配置が同じことも確認する。
    fn open_panel_map_until(
        &self,
        closed: &BTreeSet<EdgeId>,
        should_stop: &mut impl FnMut() -> bool,
    ) -> Option<BTreeMap<FaceId, Vec<FaceId>>> {
        if should_stop() {
            return None;
        }
        let activated = activated_edges(&self.document);

        let owners = face_owners(&self.faces);
        let mut adjacent = self
            .faces
            .iter()
            .map(|face| (face.id, BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        for (edge, incident) in owners {
            if should_stop() {
                return None;
            }
            if incident.len() != 2 || closed.contains(&edge) || activated.contains(&edge) {
                continue;
            }
            let (left, right) = (incident[0], incident[1]);
            let (Some(left_placement), Some(right_placement)) = (
                self.state.placements.get(&left),
                self.state.placements.get(&right),
            ) else {
                continue;
            };
            if !left_placement.approx_eq(right_placement, PLACEMENT_TOL) {
                continue;
            }
            adjacent.entry(left).or_default().insert(right);
            adjacent.entry(right).or_default().insert(left);
        }

        let mut panel_of = BTreeMap::new();
        let mut visited = BTreeSet::new();
        for start in self.faces.iter().map(|face| face.id) {
            if should_stop() {
                return None;
            }
            if !visited.insert(start) {
                continue;
            }
            let mut pending = vec![start];
            let mut panel = BTreeSet::new();
            while let Some(face) = pending.pop() {
                if should_stop() {
                    return None;
                }
                panel.insert(face);
                if let Some(next) = adjacent.get(&face) {
                    for &other in next {
                        if should_stop() {
                            return None;
                        }
                        if visited.insert(other) {
                            pending.push(other);
                        }
                    }
                }
            }
            let panel = panel.into_iter().collect::<Vec<_>>();
            for face in &panel {
                if should_stop() {
                    return None;
                }
                panel_of.insert(*face, panel.clone());
            }
        }
        (!should_stop()).then_some(panel_of)
    }

    /// panel境界で現在閉じている辺を、畳み平面の支持直線ごとに1本へまとめる。
    fn panel_closed_support_lines_until(
        &self,
        panel: &[FaceId],
        closed: &BTreeSet<EdgeId>,
        should_stop: &mut impl FnMut() -> bool,
    ) -> Option<Vec<[[f64; 2]; 2]>> {
        if should_stop() {
            return None;
        }
        let selected = panel.iter().copied().collect::<BTreeSet<_>>();
        let owners = face_owners(&self.faces);
        let positions = self
            .document
            .cp
            .vertices
            .iter()
            .map(|vertex| (vertex.id, vertex.pos))
            .collect::<BTreeMap<_, _>>();
        let mut supports: Vec<[[f64; 2]; 2]> = Vec::new();
        for edge in &self.document.cp.edges {
            if should_stop() {
                return None;
            }
            if edge.kind == EdgeKind::Border || !closed.contains(&edge.id) {
                continue;
            }
            let Some(incident) = owners.get(&edge.id).filter(|faces| faces.len() == 2) else {
                continue;
            };
            let inside = incident
                .iter()
                .filter(|face| selected.contains(*face))
                .copied()
                .collect::<Vec<_>>();
            if inside.len() != 1 {
                continue;
            }
            let (Some(&a), Some(&b), Some(placement)) = (
                positions.get(&edge.v0),
                positions.get(&edge.v1),
                self.state.placements.get(&inside[0]),
            ) else {
                continue;
            };
            let segment = [
                transform_material_point(
                    a,
                    placement.mirrored,
                    placement.rotation,
                    [placement.translation.x, placement.translation.y],
                ),
                transform_material_point(
                    b,
                    placement.mirrored,
                    placement.rotation,
                    [placement.translation.x, placement.translation.y],
                ),
            ];
            if point_near(segment[0], segment[1]) {
                continue;
            }
            if let Some(existing) = supports
                .iter_mut()
                .find(|known| supporting_lines_match(**known, segment))
            {
                if segment_length(segment) > segment_length(*existing) {
                    *existing = segment;
                }
            } else {
                supports.push(segment);
            }
        }
        supports.sort_by_key(|segment| quantized_segment(*segment));
        (!should_stop()).then_some(supports)
    }

    /// 花弁折りの中心軸を、露出panelの閉じた角と自由端から作る。
    fn panel_petal_axes_until(
        &self,
        panel: &[FaceId],
        closed: &BTreeSet<EdgeId>,
        should_stop: &mut impl FnMut() -> bool,
    ) -> Option<Vec<PetalAxis>> {
        if should_stop() {
            return None;
        }
        let Some(first_face) = panel.first() else {
            return Some(Vec::new());
        };
        let Some(placement) = self.state.placements.get(first_face) else {
            return Some(Vec::new());
        };
        if panel.iter().any(|face| {
            self.state
                .placements
                .get(face)
                .is_none_or(|other| !placement.approx_eq(other, PLACEMENT_TOL))
        }) {
            return Some(Vec::new());
        }

        let selected = panel.iter().copied().collect::<BTreeSet<_>>();
        let owners = face_owners(&self.faces);
        let positions = self
            .document
            .cp
            .vertices
            .iter()
            .map(|vertex| (vertex.id, vertex.pos))
            .collect::<BTreeMap<_, _>>();
        let mut boundary_vertices = BTreeSet::new();
        let mut closed_at = BTreeMap::<VertexId, Vec<EdgeId>>::new();
        for edge in &self.document.cp.edges {
            if should_stop() {
                return None;
            }
            let Some(incident) = owners.get(&edge.id) else {
                continue;
            };
            let inside = incident
                .iter()
                .filter(|face| selected.contains(*face))
                .count();
            if inside == 0 {
                continue;
            }
            let boundary = edge.kind == EdgeKind::Border || inside < incident.len();
            if !boundary {
                continue;
            }
            boundary_vertices.insert(edge.v0);
            boundary_vertices.insert(edge.v1);
            if edge.kind != EdgeKind::Border && closed.contains(&edge.id) && inside == 1 {
                closed_at.entry(edge.v0).or_default().push(edge.id);
                closed_at.entry(edge.v1).or_default().push(edge.id);
            }
        }

        let edges = self
            .document
            .cp
            .edges
            .iter()
            .map(|edge| (edge.id, edge))
            .collect::<BTreeMap<_, _>>();
        let mut axes = BTreeMap::new();
        for (pivot_id, incident) in closed_at {
            if should_stop() {
                return None;
            }
            let Some(&pivot_material) = positions.get(&pivot_id) else {
                continue;
            };
            let mut has_corner = false;
            'corner: for (index, left_id) in incident.iter().enumerate() {
                for right_id in &incident[index + 1..] {
                    if should_stop() {
                        return None;
                    }
                    let (Some(left), Some(right)) = (edges.get(left_id), edges.get(right_id))
                    else {
                        continue;
                    };
                    if edge_directions_are_non_collinear(pivot_id, left, right, &positions) {
                        has_corner = true;
                        break 'corner;
                    }
                }
            }
            if !has_corner {
                continue;
            }
            let placement_translation = [placement.translation.x, placement.translation.y];
            let pivot = transform_material_point(
                pivot_material,
                placement.mirrored,
                placement.rotation,
                placement_translation,
            );
            let mut tips = Vec::new();
            for vertex in &boundary_vertices {
                if should_stop() {
                    return None;
                }
                let Some(material) = positions.get(vertex) else {
                    continue;
                };
                let point = transform_material_point(
                    *material,
                    placement.mirrored,
                    placement.rotation,
                    placement_translation,
                );
                let distance = (point[0] - pivot[0]).hypot(point[1] - pivot[1]);
                if distance > LINE_TOL {
                    tips.push((*vertex, point, distance));
                }
            }
            let Some(max_distance) = tips
                .iter()
                .map(|(_, _, distance)| *distance)
                .max_by(f64::total_cmp)
            else {
                continue;
            };
            tips.retain(|(_, _, distance)| max_distance - *distance <= LINE_TOL);
            tips.sort_by(|left, right| {
                quantized_point(left.1)
                    .cmp(&quantized_point(right.1))
                    .then(left.0.cmp(&right.0))
            });
            for (_, tip, _) in tips {
                if should_stop() {
                    return None;
                }
                if !axis_splits_panel(
                    tip,
                    pivot,
                    &boundary_vertices,
                    &positions,
                    (
                        placement.mirrored,
                        placement.rotation,
                        placement_translation,
                    ),
                ) {
                    continue;
                }
                let line = [tip, pivot];
                axes.insert((quantized_segment(line), quantized_point(tip)), (line, tip));
            }
        }
        (!should_stop()).then(|| axes.into_values().collect())
    }

    fn packet_technique_candidate(
        &self,
        kind: PacketTechnique,
        ids: &[usize],
        packet: &[FaceId],
        line: [[f64; 2]; 2],
        reference_point: [f64; 2],
        open_to_back: bool,
    ) -> Option<NetworkCandidate> {
        if ids.is_empty()
            || packet.is_empty()
            || !line.iter().flatten().all(|value| value.is_finite())
            || !reference_point.iter().all(|value| value.is_finite())
            || point_near(line[0], line[1])
        {
            return None;
        }
        let line_key = line.map(|point| point.map(|value| quantize_geometry(value, LINE_TOL)));
        let reference_key = reference_point.map(|value| quantize_geometry(value, LINE_TOL));
        let mut affected_closes = ids
            .iter()
            .flat_map(|id| self.fold_lines[*id].closes.iter().copied())
            .collect::<Vec<_>>();
        affected_closes.sort_unstable();
        affected_closes.dedup();
        Some(NetworkCandidate {
            id: 0,
            representative: line,
            key: CandidateKey::Technique {
                kind,
                packet: packet.to_vec(),
                line: line_key,
                reference: reference_key,
                open_to_back,
            },
            action: NetworkAction::Technique {
                kind,
                input: TechniqueInput {
                    flap: packet.to_vec(),
                    line,
                    reference_point,
                    open_to_back: Some(open_to_back),
                    polygon: None,
                    center: None,
                },
            },
            affected_closes,
        })
    }

    /// 折り線の姿勢線分を作る全FoldLine共通の幾何indexを1回だけ組み立てる。
    fn folded_segment_geometry_until(
        &self,
        should_stop: &mut impl FnMut() -> bool,
    ) -> Option<FoldedSegmentGeometry<'_>> {
        let mut positions = BTreeMap::new();
        for vertex in &self.document.cp.vertices {
            if should_stop() {
                return None;
            }
            // 従来の`collect::<BTreeMap<_, _>>()`と同じく、重複IDなら後ろを保持する。
            positions.insert(vertex.id, vertex.pos);
        }
        if should_stop() {
            return None;
        }
        let owners = face_owners(&self.faces);
        if should_stop() {
            return None;
        }
        let mut edges = BTreeMap::new();
        for edge in &self.document.cp.edges {
            if should_stop() {
                return None;
            }
            // 従来の`iter().find(...)`と同じく、重複IDなら先頭を保持する。
            edges.entry(edge.id).or_insert(edge);
        }
        if should_stop() {
            None
        } else {
            Some(FoldedSegmentGeometry {
                positions,
                owners,
                edges,
            })
        }
    }

    /// 1本の材料折り線が、現在の畳み平面で占める線分を集める。
    fn folded_segments_until(
        &self,
        line: &FoldLine,
        geometry: &FoldedSegmentGeometry<'_>,
        should_stop: &mut impl FnMut() -> bool,
    ) -> Option<Vec<[[f64; 2]; 2]>> {
        if should_stop() {
            return None;
        }
        let mut segments = Vec::new();
        for edge_id in &line.edges {
            if should_stop() {
                return None;
            }
            let Some(edge) = geometry.edges.get(edge_id) else {
                continue;
            };
            let (Some(&a), Some(&b)) = (
                geometry.positions.get(&edge.v0),
                geometry.positions.get(&edge.v1),
            ) else {
                continue;
            };
            for face_id in geometry.owners.get(edge_id).into_iter().flatten() {
                if should_stop() {
                    return None;
                }
                let Some(placement) = self.state.placements.get(face_id) else {
                    continue;
                };
                let transform = |point: [f64; 2]| {
                    let y = if placement.mirrored {
                        -point[1]
                    } else {
                        point[1]
                    };
                    let (sin, cos) = placement.rotation.sin_cos();
                    [
                        cos * point[0] - sin * y + placement.translation.x,
                        sin * point[0] + cos * y + placement.translation.y,
                    ]
                };
                let mut segment = [transform(a), transform(b)];
                if segment[1][0]
                    .total_cmp(&segment[0][0])
                    .then(segment[1][1].total_cmp(&segment[0][1]))
                    .is_lt()
                {
                    segment.swap(0, 1);
                }
                if !segments.iter().any(|known: &[[f64; 2]; 2]| {
                    point_near(known[0], segment[0]) && point_near(known[1], segment[1])
                }) {
                    segments.push(segment);
                }
            }
        }
        (!should_stop()).then_some(segments)
    }

    /// 展開図から、折り線のまとまり・一度に閉じる直線・折り終えた印を作り直す。
    fn rebuild(&mut self) {
        self.network_candidates = OnceLock::new();
        self.lines = crease_lines(&self.document.cp);
        self.fold_lines = build_fold_lines(&self.lines);
        self.closed = closed_edges(&self.faces, &self.state);
        self.folded = 0;
        for line in &self.lines {
            if !line.edges.is_empty()
                && line.edges.iter().all(|e| self.closed.contains(e))
            {
                self.folded |= folded_bit(line.id);
            }
        }
    }
}

/// 折り終えた印の、その折り線1本ぶんのビット。
///
/// [`FoldedMask`] は [`MAX_LINES`] 本ぶんしか幅が無い。折る途中で折り目が増える手
/// (花弁折りなど)を扱えるようにしたので、**折り線の本数は手を進めるほど増えうる**。
/// 見張りが無いと `1 << id` が桁あふれし、最適化ありでは別の折り線のビットへ
/// 化けて回り込む(Rustのシフトは最適化ありで幅の剰余を取る)。
///
/// 上限を超えた折り線は**どのビットも立てない**。「閉じ終えた」と誤って印を付けて
/// 完成扱いにするより、「まだ閉じていない」として扱うほうが安全側である
/// (印が立たなければ、その線は候補に残り続けるだけで、勝手に完成にはならない)。
/// 同じ見張りは `crate::plan::full_mask` にもある。
fn folded_bit(id: usize) -> FoldedMask {
    if id >= MAX_LINES {
        0
    } else {
        1 << id
    }
}

/// 折り終えた印に、その折り線のビットが立っているか。[`folded_bit`] と対になる。
fn folded_bit_is_set(mask: FoldedMask, id: usize) -> bool {
    let bit = folded_bit(id);
    bit != 0 && mask & bit != 0
}

/// 同時操作の種類。どちらもori3-layersの既存汎用機構へ渡すだけである。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PacketTechnique {
    /// 開いてつぶす折り。既閉鎖線のpacketだけを再作動して層を組み替える。
    Squash,
    /// 花弁折り。packet内の旧閉鎖線を開きながら別の線群を閉じる。
    Petal,
}

/// 同時操作の種類。すべてori3-layersの既存汎用機構へ渡すだけである。
#[derive(Clone, Debug)]
enum NetworkAction {
    Collapse {
        lines: Vec<[[f64; 2]; 2]>,
        target_layers: Option<Vec<FaceId>>,
    },
    FlatPose {
        activations: Vec<PoseEdgeActivation>,
        drivers: Vec<PoseAngleTarget>,
        branch_hints: Vec<PoseAngleTarget>,
    },
    FoldThrough {
        line: [[f64; 2]; 2],
        keep_side_point: [f64; 2],
        direction: FoldDirection,
        target_layers: Option<Vec<FaceId>>,
    },
    Technique {
        kind: PacketTechnique,
        input: TechniqueInput,
    },
}

/// f64を含めずに候補を重複排除する、決定的な内部identity。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum CandidateKey {
    Collapse {
        line_ids: Vec<usize>,
        packet_edges: Option<Vec<EdgeId>>,
    },
    FlatPose {
        /// -1=谷180°、0=開く、+1=山180°。
        targets: Vec<(EdgeId, i8)>,
        /// 開いた線を既存M/Vと反対符号へ閉じる、方向選択の準備枝か。
        directional: bool,
    },
    FoldThrough {
        line_id: usize,
        line: [[i64; 2]; 2],
        keep_side: [i64; 2],
        down: bool,
        packet: Option<Vec<FaceId>>,
    },
    Technique {
        kind: PacketTechnique,
        packet: Vec<FaceId>,
        line: [[i64; 2]; 2],
        reference: [i64; 2],
        open_to_back: bool,
    },
}

/// 複数線またはpacketをまとめた、内部用の同時操作候補。
#[derive(Clone, Debug)]
struct NetworkCandidate {
    id: usize,
    representative: [[f64; 2]; 2],
    key: CandidateKey,
    action: NetworkAction,
    /// 0°以外を指定した、作業18単位の線。終点で実際に閉じたものだけ結果へ載せる。
    affected_closes: Vec<usize>,
}

/// 花弁折りの中心軸1本。`(中心線, 持ち上げる先端)` の組である。
type PetalAxis = ([[f64; 2]; 2], [f64; 2]);

#[derive(Clone, Copy, Debug)]
struct PacketEdgeRelation {
    internal: bool,
    boundary: bool,
}

fn affected_lines(fold_lines: &[FoldLine], ids: &[usize]) -> Vec<usize> {
    let mut affected = ids
        .iter()
        .flat_map(|id| fold_lines[*id].closes.iter().copied())
        .collect::<Vec<_>>();
    affected.sort_unstable();
    affected.dedup();
    affected
}

/// 折った後に面が**減った**か。減ったときだけ「面が欠けた」とみなす。
///
/// **増えるのは欠けではない。** 実際の紙では、折る途中で新しい折り目が生まれる手
/// (花弁折り、層を貫く折り)が普通にある。折り目が1本増えれば、その折り目が横切る
/// 面は2枚に分かれるので、面の数は必ず増える。増加まで「面が欠けた」として捨てると、
/// **折り目が増える手を1件も表せない**(`docs/requirements-definition.md` §2
/// 「表現の完全性」)。
///
/// ## 増加を許しても、面の欠けは見落とさない
///
/// 面が本当に欠けたことは、この後の姿勢の走査でも
/// `replayed.frame.faces.len() != faces.len()` として**別に**見ている。
/// そちらは「その手の展開図から取り出した面」と「再生して出てきた面」を比べており、
/// 折り目が増えても正しく働く。ここで見るのは**手の前後**の比較だけである。
///
/// ## 実測(2026-08-23、最適化あり、`WITH_EXTRA_CANDIDATES = true`)
///
/// 変える前は「増えた」だけを理由に、姿勢を1点も見ないまま次の手が捨てられていた。
/// 鳥の基本形・深さ2の花弁折り2件(面 14 → 15)は、3姿勢すべてで飛ばした手順0・
/// 警告0だったのに、この1行だけで落ちていた。
/// 方向付き単線は、鳥の基本形の最初の状態で**49候補中20件(41%)**が同じ理由で落ちていた。
/// `flat_motion` が**動きの部品ごと**に出す「その部品の領域に掛からない層を外した」知らせ。
///
/// # なぜ、この1件だけを別扱いにするのか
///
/// まったく同じ文面が2か所から出るが、**意味が違う**。
///
/// | 出どころ | 意味 |
/// |---|---|
/// | `crates/ori3-layers/src/fold_through.rs:300` | **1回の折り**で、頼んだ層が折られなかった。本当に「頼んだ手と違う」 |
/// | `crates/ori3-layers/src/flat_motion.rs:604` | **動きの部品(`MotionPart`)ごと**に、その部品の領域に掛からない層を外した |
///
/// [`FoldSession::apply_packet_technique`] が呼ぶ技法(`squash`・`petal`)は、
/// **`flat_motion` しか呼ばない**(`crates/ori3-layers/src/techniques.rs` の
/// `squash`・`petal` は、どちらも `flat_motion` を1回呼ぶだけで `fold_through` を呼ばない)。
/// したがって、この入口へ届くこの文面は**必ず後者の意味**である。
///
/// 花弁折りの動きは「右の羽・左の羽・先端・袋」など**複数の部品**でできている。
/// 掴んだ紙(open panel)の一部の面が、ある部品の領域に掛からないのは**当たり前**で、
/// それを「折り上がりが指定と違う」と読むのは誤読である。
///
/// # 誤読していたことの実測(2026-08-23。`scratchpad/petal-tear-cause-report.md`)
///
/// 鳥の基本形で、予備基本形の状態(手順 `[2, 7]` の後)に現れる花弁折りの候補は4件で、
/// **4件とも裂け0本・折り線7本・そのうち `0°`(袋の口を開く)が2本**、
/// つまり**参照どおりの花弁折りとまったく同じ記録**だった。
/// ところが、そのうち2件(掴んだ紙が4面ある側)がこの知らせを出したため落とされていた。
/// **落とされていた2件が、鳥の基本形を完成させる手だった**
/// (その2件を通すと、花弁折り2回で4つの隔たりがすべて `0.000000` になる)。
///
/// # 取りこぼしを見落とさないための担保
///
/// - 「指定した層が**まったく動かなかった**」場合は、`petal` 自身が
///   **別の警告**を出す(`techniques.rs` の「…折り線の手前側に掛かっていないため動きません」)。
///   その文面は [`NOT_AS_REQUESTED_MARKS`](ori3_layers::fold_through::NOT_AS_REQUESTED_MARKS) に入っていないので、**もともと落とす理由にしていない**。
/// - 折り上がりが平らに畳めているかは [`torn_creases`] が**測って**判定する。
/// - 折り途中で本当に折れるかは [`FoldSession::verify_successor`] の21姿勢の走査
///   (面の欠け・非有限・裂け・すり抜け・再生の警告)が判定する。
///
/// # 文面に頼っていることの後始末
///
/// `ori3-layers` は「どの警告か」を型で区別せず、文字列だけを返す。
/// そのため、ここも文面で見分けるしかない。**文面が変わったときに黙って
/// 誤読へ戻らないよう**、この定数が [`NOT_AS_REQUESTED_MARKS`](ori3_layers::fold_through::NOT_AS_REQUESTED_MARKS) の中に
/// 実在し続けることを検査
/// (`the_part_layer_skip_notice_is_still_one_of_the_not_as_requested_marks`)で固定する。
/// 見つからなくなったら検査が落ちるので、そのときは文面ではなく
/// **`flat_motion` 側で知らせの種類を分ける**ことを検討する。
const PART_LAYER_SKIP_MARK: &str = "折り線の可動側に掛かっていないため除外しました";

/// この警告を理由に、packet技法の候補を落とすか。
///
/// [`NOT_AS_REQUESTED_MARKS`](ori3_layers::fold_through::NOT_AS_REQUESTED_MARKS) のうち、[`PART_LAYER_SKIP_MARK`] だけを除く。
/// 理由は [`PART_LAYER_SKIP_MARK`] のコメントに書いた。
fn packet_technique_warning_is_blocking(warning: &str) -> bool {
    warning_means_the_fold_was_not_as_requested(warning) && !warning.contains(PART_LAYER_SKIP_MARK)
}

/// 折り目の両側の紙が「同じ場所にある」とみなす距離(紙の長辺=1)。
///
/// `ori3-layers` が技法の中で使う `JOIN_EPS` と同じ値である。
///
/// 実測(2026-08-23、鳥の基本形と折り鶴、花弁折り1080回。
/// `scratchpad/flat-endpoint-converge-report.md` §11.4):
///
/// - 裂けていない手では、両端点のずれは丸め誤差の範囲(`1e-16` 台)だった
/// - 裂けている手のずれは **最小 0.03108 / 中央 0.2242 / 最大 1.275**
///
/// つまり「裂けている」と「裂けていない」のあいだは13桁以上あいており、
/// この境目 `1e-6` は**実測の最小の裂けの3万分の1**、
/// **丸め誤差の1e10倍**である。どちらの側にも十分な余裕がある。
const JOINED_TOLERANCE: f64 = 1e-6;

/// 技法が作った平らな形で、**両側の紙が離れてしまった折り目**とその距離(辺ID昇順)。
///
/// # なぜ候補の段階で見るのか
///
/// 紙が裂ける手は、実際には折れない。`ori3-layers` の技法は
/// 「止めずに警告する」(`CLAUDE.md` §8)ので裂けても手順を返すが、
/// **裂けている折り目には山谷も角度も決められない**ため、
/// その折り目は手順に1本も記録されない
/// (`crates/ori3-layers/src/flat_motion.rs::settle_creases` の `joined` の判定)。
/// 記録が欠けた手順は「平らに畳める形」を指さないので、
/// `replay` は折り上がり(`t = 1.00`)で必ず収束しない。
///
/// 実測(2026-08-23、`WITH_EXTRA_CANDIDATES = true`、花弁折り1080回):
/// **裂けが無かった40回はすべて「開く」を `0°` として記録し、
/// 裂けた1040回は1本も記録しなかった。例外0件。**
/// 以前はこの1040回を、21姿勢すべて再生したあげく
/// いちばん遠い `t = 1.00` の収束判定で捨てていた。
///
/// # 数え方
///
/// 平らに畳んだ形では、折り目を挟む2つの面は**その折り目の両端点を同じ場所へ写す**。
/// 面ごとの置き方(`FlatState::placements`)で両端点を写し、
/// 離れた距離が [`JOINED_TOLERANCE`] を超えたら裂けているとみなす。
/// 山谷や角度の記録は一切見ない(**文面ではなく、測った形で決める**)。
fn torn_creases(cp: &CreasePattern, state: &FlatState) -> Vec<(EdgeId, f64)> {
    let faces = extract_faces(cp);
    let positions: BTreeMap<VertexId, glam::DVec2> = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, glam::DVec2::from(vertex.pos)))
        .collect();
    let mut sharing: BTreeMap<EdgeId, Vec<FaceId>> = BTreeMap::new();
    for face in &faces {
        for &edge in &face.edges {
            sharing.entry(edge).or_default().push(face.id);
        }
    }
    let mut torn: Vec<(EdgeId, f64)> = Vec::new();
    for (edge_id, owners) in sharing {
        if owners.len() != 2 {
            continue;
        }
        let Some(edge) = cp.edges.iter().find(|edge| edge.id == edge_id) else {
            continue;
        };
        if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let (Some(&v0), Some(&v1)) = (positions.get(&edge.v0), positions.get(&edge.v1)) else {
            continue;
        };
        let (Some(&a), Some(&b)) = (
            state.placements.get(&owners[0]),
            state.placements.get(&owners[1]),
        ) else {
            continue;
        };
        let gap = (a.apply(v0) - b.apply(v0))
            .length()
            .max((a.apply(v1) - b.apply(v1)).length());
        if gap > JOINED_TOLERANCE {
            torn.push((edge_id, gap));
        }
    }
    torn
}

fn face_count_problem(before: usize, after: usize) -> Option<PoseProblem> {
    (after < before).then_some(PoseProblem::FaceLost {
        expected: before,
        got: after,
    })
}

/// この手で新たに閉じ終えた、作業18の数え方の折り線。
///
/// ## なぜ番号ではなく**辺**で数えるか
///
/// [`CreaseLine`] の番号は端点の座標順で付け直される。折り目が増えれば以降の番号が
/// すべてずれるし、増えなくても山谷が変わればまとまりが割れたり繋がったりする
/// (実測: 折り鶴の探索経路で 34 → 33 → 32 → 30 → 31 → 28 → 28 と動く)。
/// したがって**親の番号と後継の番号を同じビット列として比べてはいけない**。
///
/// そこで、親の折り線が閉じ終えたかどうかを、**辺のID**で判定する。
/// 展開図の辺のIDは、折っても付け直されない(増えた折り目には新しいIDが付くだけ)ので、
/// 親と後継のあいだで確実に対応が取れる。
///
/// 返すのは**親の番号**の昇順で、`affected`(この手が0°以外を指定した線)のうち
/// 閉じ終えたものと、この手で新たに閉じ終えたものの和である。
fn closed_effect(
    lines: &[CreaseLine],
    affected: &[usize],
    before: FoldedMask,
    closed_after: &BTreeSet<EdgeId>,
) -> Vec<usize> {
    let closed_now = |id: usize| {
        lines.get(id).is_some_and(|line: &CreaseLine| {
            !line.edges.is_empty() && line.edges.iter().all(|edge| closed_after.contains(edge))
        })
    };
    let mut closes = affected
        .iter()
        .copied()
        .filter(|id| closed_now(*id))
        .collect::<BTreeSet<_>>();
    closes.extend(
        (0..lines.len()).filter(|id| closed_now(*id) && !folded_bit_is_set(before, *id)),
    );
    closes.into_iter().collect()
}

fn saved_angle_targets(document: &Document) -> BTreeMap<EdgeId, i64> {
    let mut targets = BTreeMap::new();
    for step in &document.sequence {
        for driver in &step.drivers {
            if !driver.target_angle_deg.is_finite() {
                continue;
            }
            let quantized = (driver.target_angle_deg * 1_000_000.0).round() as i64;
            for edge in resolve_driver_edges(&document.cp, driver) {
                targets.insert(edge, quantized);
            }
        }
    }
    // 最新の明示0°は、それ以前の±180°を上書きした「現在開いている」状態である。
    // 最後にだけ除くことで古い閉鎖角を復活させず、初期の暗黙0°と同じ物理状態へ戻す。
    targets.retain(|_, target| *target != 0);
    targets
}

/// final CPのうち、ここまでの手順で実際に0°以外へ作動したことのある辺。
///
/// 未来のprecreaseと、いったん折ってから開いたヒンジは現在角だけでは区別できない。
/// open-panel候補の境界と探索状態の同一性で同じ集合を使う。
fn activated_edges(document: &Document) -> BTreeSet<EdgeId> {
    let mut activated = BTreeSet::new();
    for step in &document.sequence {
        for driver in &step.drivers {
            if driver.target_angle_deg.is_finite() && driver.target_angle_deg.abs() > LINE_TOL {
                activated.extend(resolve_driver_edges(&document.cp, driver));
            }
        }
    }
    activated
}

fn quantize_geometry(value: f64, tolerance: f64) -> i64 {
    if !value.is_finite() {
        return if value.is_sign_negative() {
            i64::MIN
        } else {
            i64::MAX
        };
    }
    let scaled = (value / tolerance).round();
    if scaled >= i64::MAX as f64 {
        i64::MAX
    } else if scaled <= i64::MIN as f64 {
        i64::MIN
    } else {
        scaled as i64
    }
}

fn quantized_point(point: [f64; 2]) -> [i64; 2] {
    point.map(|value| quantize_geometry(value, LINE_TOL))
}

fn quantized_segment(mut segment: [[f64; 2]; 2]) -> [[i64; 2]; 2] {
    if quantized_point(segment[1]) < quantized_point(segment[0]) {
        segment.swap(0, 1);
    }
    segment.map(quantized_point)
}

fn segment_length(segment: [[f64; 2]; 2]) -> f64 {
    (segment[1][0] - segment[0][0]).hypot(segment[1][1] - segment[0][1])
}

fn transform_material_point(
    point: [f64; 2],
    mirrored: bool,
    rotation: f64,
    translation: [f64; 2],
) -> [f64; 2] {
    let y = if mirrored { -point[1] } else { point[1] };
    let (sin, cos) = rotation.sin_cos();
    [
        cos * point[0] - sin * y + translation[0],
        sin * point[0] + cos * y + translation[1],
    ]
}

fn expand_packet(packet: &[FaceId], panel_of: &BTreeMap<FaceId, Vec<FaceId>>) -> Vec<FaceId> {
    packet
        .iter()
        .flat_map(|face| panel_of.get(face).cloned().unwrap_or_else(|| vec![*face]))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn edge_directions_are_non_collinear(
    pivot: VertexId,
    left: &Edge,
    right: &Edge,
    positions: &BTreeMap<VertexId, [f64; 2]>,
) -> bool {
    let other = |edge: &Edge| {
        if edge.v0 == pivot {
            Some(edge.v1)
        } else if edge.v1 == pivot {
            Some(edge.v0)
        } else {
            None
        }
    };
    let (Some(&center), Some(left_id), Some(right_id)) =
        (positions.get(&pivot), other(left), other(right))
    else {
        return false;
    };
    let (Some(&left_point), Some(&right_point)) =
        (positions.get(&left_id), positions.get(&right_id))
    else {
        return false;
    };
    let a = [left_point[0] - center[0], left_point[1] - center[1]];
    let b = [right_point[0] - center[0], right_point[1] - center[1]];
    let scale = a[0].hypot(a[1]) * b[0].hypot(b[1]);
    scale > LINE_TOL && (a[0] * b[1] - a[1] * b[0]).abs() > LINE_TOL * scale
}

fn axis_splits_panel(
    tip: [f64; 2],
    pivot: [f64; 2],
    vertices: &BTreeSet<VertexId>,
    positions: &BTreeMap<VertexId, [f64; 2]>,
    placement: (bool, f64, [f64; 2]),
) -> bool {
    let axis = [pivot[0] - tip[0], pivot[1] - tip[1]];
    let length = axis[0].hypot(axis[1]);
    if length <= LINE_TOL {
        return false;
    }
    let mut left = false;
    let mut right = false;
    for vertex in vertices {
        let Some(&material) = positions.get(vertex) else {
            continue;
        };
        let point = transform_material_point(material, placement.0, placement.1, placement.2);
        let offset = [point[0] - tip[0], point[1] - tip[1]];
        let signed_distance = (axis[0] * offset[1] - axis[1] * offset[0]) / length;
        left |= signed_distance > LINE_TOL;
        right |= signed_distance < -LINE_TOL;
    }
    left && right
}

/// 局所stackから紙の外側へ露出している上下連続packetだけを作る。
#[cfg(test)]
fn exposed_packets(stack: &[FaceId]) -> Vec<Vec<FaceId>> {
    let mut never_stop = || false;
    exposed_packets_until(stack, &mut never_stop).unwrap_or_default()
}

fn exposed_packets_until(
    stack: &[FaceId],
    should_stop: &mut impl FnMut() -> bool,
) -> Option<Vec<Vec<FaceId>>> {
    let mut packets = BTreeSet::new();
    for size in 1..=stack.len() {
        if should_stop() {
            return None;
        }
        packets.insert(stack[..size].to_vec());
        packets.insert(stack[stack.len() - size..].to_vec());
    }
    (!should_stop()).then(|| packets.into_iter().collect())
}

fn face_owners(faces: &[Face]) -> BTreeMap<EdgeId, Vec<FaceId>> {
    let mut owners: BTreeMap<EdgeId, Vec<FaceId>> = BTreeMap::new();
    for face in faces {
        for &edge in &face.edges {
            owners.entry(edge).or_default().push(face.id);
        }
    }
    owners
}

fn point_near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) <= LINE_TOL
}

fn supporting_lines_match(a: [[f64; 2]; 2], b: [[f64; 2]; 2]) -> bool {
    let direction = [a[1][0] - a[0][0], a[1][1] - a[0][1]];
    let length = direction[0].hypot(direction[1]);
    if length <= LINE_TOL {
        return false;
    }
    let distance = |point: [f64; 2]| {
        (direction[0] * (point[1] - a[0][1]) - direction[1] * (point[0] - a[0][0])).abs() / length
    };
    distance(b[0]) <= LINE_TOL && distance(b[1]) <= LINE_TOL
}

fn segments_share_positive_length(a: [[f64; 2]; 2], b: [[f64; 2]; 2]) -> bool {
    if !supporting_lines_match(a, b) {
        return false;
    }
    let direction = [a[1][0] - a[0][0], a[1][1] - a[0][1]];
    let length = direction[0].hypot(direction[1]);
    if length <= LINE_TOL {
        return false;
    }
    let unit = [direction[0] / length, direction[1] / length];
    let project = |point: [f64; 2]| point[0] * unit[0] + point[1] * unit[1];
    let interval = |segment: [[f64; 2]; 2]| {
        let start = project(segment[0]);
        let end = project(segment[1]);
        (start.min(end), start.max(end))
    };
    let (a0, a1) = interval(a);
    let (b0, b1) = interval(b);
    a1.min(b1) - a0.max(b0) > LINE_TOL
}

/// 正の長さの重なりを辺とするFoldLineグラフの最大連結成分。
///
/// 区間ごとのactive setは別の走査で保持する。この関数は
/// 推移的に連なる直線全体を追加候補にするだけで、部分集合の冪集合を作らない。
#[cfg(test)]
fn coincident_line_components(segments: &[(usize, [[f64; 2]; 2])]) -> BTreeSet<Vec<usize>> {
    let mut never_stop = || false;
    coincident_line_components_until(segments, &mut never_stop).unwrap_or_default()
}

fn coincident_line_components_until(
    segments: &[(usize, [[f64; 2]; 2])],
    should_stop: &mut impl FnMut() -> bool,
) -> Option<BTreeSet<Vec<usize>>> {
    let mut neighbors: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (index, (left_id, left)) in segments.iter().enumerate() {
        for (right_id, right) in &segments[index + 1..] {
            if should_stop() {
                return None;
            }
            if left_id == right_id || !segments_share_positive_length(*left, *right) {
                continue;
            }
            neighbors.entry(*left_id).or_default().insert(*right_id);
            neighbors.entry(*right_id).or_default().insert(*left_id);
        }
    }

    let mut visited = BTreeSet::new();
    let mut components = BTreeSet::new();
    for &start in neighbors.keys() {
        if should_stop() {
            return None;
        }
        if !visited.insert(start) {
            continue;
        }
        let mut stack = vec![start];
        let mut component = BTreeSet::new();
        while let Some(id) = stack.pop() {
            if should_stop() {
                return None;
            }
            component.insert(id);
            if let Some(next) = neighbors.get(&id) {
                for &candidate in next {
                    if should_stop() {
                        return None;
                    }
                    if visited.insert(candidate) {
                        stack.push(candidate);
                    }
                }
            }
        }
        if component.len() >= 2 {
            components.insert(component.into_iter().collect());
        }
    }
    Some(components)
}

/// 現在の平面で、正の長さを共有する線分ごとのFoldLine集合を作る。
///
/// 重なりは推移的ではない。A-BとB-Cが別区間で重なるときに`{A,B,C}`へ
/// 連結してしまわないよう、同じ支持直線上の端点間を走査し、その区間を実際に
/// 覆うIDだけを集める。端点区間は線分数に対して線形なので、部分集合の冪集合には
/// ならない。同じ集合は決定的な順序の1件へまとめる。
#[cfg(test)]
fn coincident_line_sets(segments: &[(usize, [[f64; 2]; 2])]) -> BTreeSet<Vec<usize>> {
    let mut never_stop = || false;
    coincident_line_sets_until(segments, &mut never_stop).unwrap_or_default()
}

fn coincident_line_sets_until(
    segments: &[(usize, [[f64; 2]; 2])],
    should_stop: &mut impl FnMut() -> bool,
) -> Option<BTreeSet<Vec<usize>>> {
    Some(
        coincident_line_cells_until(segments, should_stop)?
            .into_iter()
            .map(|(ids, _)| ids)
            .collect(),
    )
}

/// 正の長さを共有するactive setと、その区間の内部にある決定的なつまみ点。
fn coincident_line_cells_until(
    segments: &[(usize, [[f64; 2]; 2])],
    should_stop: &mut impl FnMut() -> bool,
) -> Option<Vec<(Vec<usize>, [f64; 2])>> {
    let mut cells = Vec::new();
    for (_, reference) in segments {
        if should_stop() {
            return None;
        }
        let direction = [
            reference[1][0] - reference[0][0],
            reference[1][1] - reference[0][1],
        ];
        let length = direction[0].hypot(direction[1]);
        if length <= LINE_TOL {
            continue;
        }
        let unit = [direction[0] / length, direction[1] / length];
        let project = |point: [f64; 2]| point[0] * unit[0] + point[1] * unit[1];
        let mut intervals = Vec::new();
        for (id, segment) in segments {
            if should_stop() {
                return None;
            }
            if supporting_lines_match(*reference, *segment) {
                let a = project(segment[0]);
                let b = project(segment[1]);
                intervals.push((*id, a.min(b), a.max(b)));
            }
        }
        let mut endpoints: Vec<f64> = intervals.iter().flat_map(|(_, a, b)| [*a, *b]).collect();
        endpoints.sort_by(f64::total_cmp);
        if should_stop() {
            return None;
        }
        for window in endpoints.windows(2) {
            if should_stop() {
                return None;
            }
            if window[1] - window[0] <= LINE_TOL {
                continue;
            }
            let midpoint = (window[0] + window[1]) * 0.5;
            let ids: Vec<usize> = intervals
                .iter()
                .filter(|(_, a, b)| *a < midpoint && midpoint < *b)
                .map(|(id, _, _)| *id)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            if ids.len() >= 2 {
                let origin_projection = project(reference[0]);
                let point = [
                    reference[0][0] + unit[0] * (midpoint - origin_projection),
                    reference[0][1] + unit[1] * (midpoint - origin_projection),
                ];
                cells.push((ids, point));
            }
        }
    }
    cells.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then(left.1[0].total_cmp(&right.1[0]))
            .then(left.1[1].total_cmp(&right.1[1]))
    });
    if should_stop() {
        return None;
    }
    cells.dedup_by(|left, right| left.0 == right.0 && point_near(left.1, right.1));
    (!should_stop()).then_some(cells)
}

/// 作業18の折り線のまとまりを、山谷を問わず同じ直線に乗るものへまとめ直す。
fn build_fold_lines(lines: &[CreaseLine]) -> Vec<FoldLine> {
    let mut groups: Vec<(f64, f64, f64, Vec<usize>)> = Vec::new(); // (dx, dy, offset, 番号)
    for line in lines {
        let d = [line.b[0] - line.a[0], line.b[1] - line.a[1]];
        let len = d[0].hypot(d[1]);
        if !len.is_finite() || len <= LINE_TOL {
            continue;
        }
        let mut dir = [d[0] / len, d[1] / len];
        if dir[0] < -LINE_TOL || (dir[0].abs() <= LINE_TOL && dir[1] < 0.0) {
            dir = [-dir[0], -dir[1]];
        }
        let off = dir[0] * line.a[1] - dir[1] * line.a[0];
        match groups.iter_mut().find(|g| {
            (g.0 - dir[0]).abs() <= LINE_TOL
                && (g.1 - dir[1]).abs() <= LINE_TOL
                && (g.2 - off).abs() <= LINE_TOL
        }) {
            Some(g) => g.3.push(line.id),
            None => groups.push((dir[0], dir[1], off, vec![line.id])),
        }
    }

    let mut out: Vec<FoldLine> = groups
        .into_iter()
        .map(|(dx, dy, _, members)| {
            let along = |p: [f64; 2]| p[0] * dx + p[1] * dy;
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            let mut a = [0.0, 0.0];
            let mut b = [0.0, 0.0];
            let mut mask: FoldedMask = 0;
            let mut edges: Vec<EdgeId> = Vec::new();
            for &m in &members {
                let line = &lines[m];
                mask |= folded_bit(line.id);
                edges.extend_from_slice(&line.edges);
                for p in [line.a, line.b] {
                    let t = along(p);
                    if t < lo {
                        lo = t;
                        a = p;
                    }
                    if t > hi {
                        hi = t;
                        b = p;
                    }
                }
            }
            edges.sort_unstable();
            edges.dedup();
            FoldLine {
                id: 0,
                a,
                b,
                closes: members,
                mask,
                edges,
            }
        })
        .collect();
    out.sort_by(|x, y| {
        x.a[0]
            .total_cmp(&y.a[0])
            .then(x.a[1].total_cmp(&y.a[1]))
            .then(x.b[0].total_cmp(&y.b[0]))
            .then(x.b[1].total_cmp(&y.b[1]))
    });
    for (i, line) in out.iter_mut().enumerate() {
        line.id = i;
        line.closes.sort_unstable();
    }
    out
}

/// もう閉じている(180度まで折ってある)折り目を集める。
///
/// 折り目が開いていれば両側の面の置き方は同じで、閉じていれば鏡映のぶんだけ違う。
/// 手順を何手進めても、いまの重なり順から同じ規則で数え直せる。
fn closed_edges(faces: &[Face], state: &FlatState) -> BTreeSet<EdgeId> {
    let mut owners: BTreeMap<EdgeId, Vec<ori3_model::FaceId>> = BTreeMap::new();
    for face in faces {
        for &edge in &face.edges {
            owners.entry(edge).or_default().push(face.id);
        }
    }
    let mut out = BTreeSet::new();
    for (edge, ids) in owners {
        if ids.len() != 2 {
            continue;
        }
        let (Some(a), Some(b)) = (state.placements.get(&ids[0]), state.placements.get(&ids[1]))
        else {
            continue;
        };
        if !a.approx_eq(b, PLACEMENT_TOL) {
            out.insert(edge);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use ori3_cp::insert_segment;
    use ori3_model::{Document, DriverLine, EdgeKind, FaceId, FoldStep, Paper, TechniqueKind};

    use super::{
        CandidateKey, FoldSession, MAX_LINES, PART_LAYER_SKIP_MARK, PacketEdgeRelation,
        PacketTechnique, PoseProblem, PoseScan, PreparedMove, activated_edges, closed_effect,
        coincident_line_components, coincident_line_sets, exposed_packets, exposed_packets_until,
        face_count_problem, folded_bit, folded_bit_is_set, packet_technique_warning_is_blocking,
        resolve_driver_edges, saved_angle_targets, torn_creases,
    };

    use std::collections::BTreeSet;
    use std::sync::OnceLock;

    use ori3_cp::{Face, extract_faces};
    use ori3_layers::flat_state::FlatState;
    use ori3_layers::replay::flat_state_at;
    use ori3_layers::{
        FoldDirection, FoldThroughInput, TechniqueInput, fold_through, petal, squash,
    };

    /// 予備基本形(正方形を半分に2回折り、つぶし折りを2回)。
    ///
    /// 実際の紙で花弁折りの土台に使う形で、
    /// `crates/ori3-layers/tests/flat_endpoint.rs` が作る参照の鳥の基本形の
    /// 前半4手と同じものである。
    fn preliminary_base() -> (Document, Vec<Face>, FlatState) {
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        for (line, keep_side_point) in [
            ([[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]),
            ([[0.5, 0.0], [0.5, 0.5]], [0.25, 0.25]),
        ] {
            let faces = extract_faces(&document.cp);
            let up_to = document.sequence.len();
            let (state, _) = flat_state_at(&document, &faces, up_to).expect("平らな状態から折る");
            let mut cp = document.cp.clone();
            let result = fold_through(
                &mut cp,
                &faces,
                &state,
                &FoldThroughInput {
                    line,
                    keep_side_point,
                    target_layers: None,
                    direction: FoldDirection::Up,
                },
            )
            .expect("半分に折れる");
            let mut step = result.step;
            step.id = u32::try_from(up_to).expect("手順番号");
            document.cp = cp;
            document.sequence.push(step);
        }
        for (line, reference_point) in [
            ([[0.5, 0.0], [0.5, 1.0]], [0.5, 0.1]),
            ([[0.0, 0.5], [1.0, 0.5]], [0.1, 0.5]),
        ] {
            let faces = extract_faces(&document.cp);
            let up_to = document.sequence.len();
            let (state, _) = flat_state_at(&document, &faces, up_to).expect("平らな状態から折る");
            let mut cp = document.cp.clone();
            let result = squash(
                &mut cp,
                &faces,
                &state,
                &TechniqueInput {
                    flap: vec![state.order[0]],
                    line,
                    reference_point,
                    open_to_back: None,
                    polygon: None,
                    center: None,
                },
            )
            .expect("つぶし折りできる");
            let mut step = result.step;
            step.id = u32::try_from(up_to).expect("手順番号");
            document.cp = cp;
            document.sequence.push(step);
        }
        let faces = extract_faces(&document.cp);
        let (state, _) =
            flat_state_at(&document, &faces, document.sequence.len()).expect("平らに畳める");
        (document, faces, state)
    }

    /// この手が `0°`(=開く)として記録した折り線の本数。
    fn opened_lines(step: &FoldStep) -> usize {
        step.drivers
            .iter()
            .filter(|driver| driver.target_angle_deg.abs() < 90.0)
            .count()
    }

    fn assert_same_fine_result(reverified: &PreparedMove, fresh: &PreparedMove) {
        assert_eq!(
            reverified.verified(),
            fresh.verified(),
            "粗走査の終点を再検証した手と、細走査で折り直した手が違う"
        );
        assert_eq!(
            reverified.successor().document().cp,
            fresh.successor().document().cp,
            "粗走査と細走査で終点の展開図が違う"
        );
        assert_eq!(
            reverified.successor().document().sequence,
            fresh.successor().document().sequence,
            "粗走査と細走査で終点の手順が違う"
        );
        assert_eq!(
            reverified.successor().state_key(),
            fresh.successor().state_key(),
            "粗走査と細走査で終点の物理状態が違う"
        );
        assert_eq!(reverified.verified().poses_checked, 21);
        assert_eq!(reverified.verified().id, fresh.verified().id);
        assert_eq!(reverified.verified().mask, fresh.verified().mask);
    }

    #[test]
    fn fine_reverification_of_a_single_line_matches_a_fresh_full_scan() {
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        insert_segment(&mut document.cp, [0.0, 0.5], [1.0, 0.5], EdgeKind::Mountain);
        let session = FoldSession::new(&document).expect("中央折りのsessionを作れない");
        let id = session.fold_lines.first().expect("中央折り線がない").id;
        let coarse = session
            .prepare_move(id, PoseScan { steps: 2 })
            .expect("3姿勢の粗走査を通らない");
        assert_eq!(coarse.verified().poses_checked, 3);

        let reverified = session
            .reverify_prepared_move(coarse, PoseScan::DEFAULT)
            .expect("粗走査の終点が21姿勢を通らない");
        let fresh = session
            .prepare_move(id, PoseScan::DEFAULT)
            .expect("同じ手を折り直した21姿勢検査が通らない");
        assert_same_fine_result(&reverified, &fresh);
    }

    #[test]
    fn fine_reverification_of_a_network_move_matches_a_fresh_full_scan() {
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        for x in [0.25, 0.75] {
            insert_segment(&mut document.cp, [x, 0.0], [x, 1.0], EdgeKind::Valley);
        }
        let session = FoldSession::new(&document).expect("平行な2本折りのsessionを作れない");
        let network_id = session.fold_lines.len();
        let coarse = session
            .prepare_move(network_id, PoseScan { steps: 2 })
            .expect("2本を同時に閉じる網が3姿勢の粗走査を通らない");
        assert!(coarse.verified().closes.len() >= 2);
        assert_eq!(coarse.verified().poses_checked, 3);

        let reverified = session
            .reverify_prepared_move(coarse, PoseScan::DEFAULT)
            .expect("粗走査の網終点が21姿勢を通らない");
        let fresh = session
            .prepare_move(network_id, PoseScan::DEFAULT)
            .expect("同じ網を折り直した21姿勢検査が通らない");
        assert_same_fine_result(&reverified, &fresh);
    }

    /// 実際に折れる花弁折りは紙を裂かず、開く袋の口を `0°` として記録する。
    ///
    /// 裂けを理由に候補を落とす仕組み([`torn_creases`])が、
    /// **正しい花弁折りまで落としてしまわない**ことを固定する。
    #[test]
    fn a_petal_fold_that_lies_flat_records_the_pocket_it_opens() {
        let (document, faces, state) = preliminary_base();
        let mut cp = document.cp.clone();
        let result = petal(
            &mut cp,
            &faces,
            &state,
            &TechniqueInput {
                flap: vec![*state.order.last().expect("最前面")],
                line: [[0.0, 1.0], [0.5, 0.5]],
                reference_point: [0.0, 1.0],
                open_to_back: None,
                polygon: None,
                center: None,
            },
        )
        .expect("参照どおりの花弁折り");
        let torn = torn_creases(&cp, &result.state);
        assert!(
            torn.is_empty(),
            "折れる花弁折りを「紙が裂ける」と数えている: {torn:?}"
        );
        assert_eq!(
            result.step.drivers.len(),
            7,
            "花弁折りの折り線(斜め2本 + ちょうつがい + 新しく引く線 + 開く袋の口2本)"
        );
        assert_eq!(
            opened_lines(&result.step),
            2,
            "開く袋の口2本を0°として記録する(実測 {:?})",
            result
                .step
                .drivers
                .iter()
                .map(|driver| driver.target_angle_deg)
                .collect::<Vec<_>>()
        );
    }

    /// 紙が裂ける技法の手は、姿勢を1つも見ないうちに候補から落とす。
    ///
    /// # なぜこの検査が要るか
    ///
    /// 裂けている折り目には山谷も角度も決められないので、
    /// `crates/ori3-layers/src/flat_motion.rs::settle_creases` はその折り目を
    /// **1本も手順へ記録しない**。記録の欠けた手順は平らに畳める形を指さないため、
    /// `replay` は折り上がり(`t = 1.00`)で必ず収束しない。
    ///
    /// 以前はその収束判定という**いちばん遠い場所**で気づいていた。実測(2026-08-23、
    /// `scratchpad/flat-endpoint-converge-report.md` §11.4)では、探索が作る
    /// 花弁折り1080回のうち **1040回が裂けており**、
    /// **裂けなかった40回はすべて開く動きを `0°` として記録していた(例外0件)**。
    ///
    /// # 標本の作り方
    ///
    /// 中心線は参照と同じ `[[0,1],[0.5,0.5]]` のまま、**先端の位置だけを
    /// 中心線の反対の端 `[0.5, 0.5]` にする**と、持ち上げる先が紙の外を向くので裂ける。
    /// 実測: 裂けた折り目 **2本**、いちばん離れた距離 **1.0**(紙の長辺=1)。
    /// 判定の境目 `0.8` は実測の約8割(`CLAUDE.md` §10.7.9)で、
    /// 裂けていない側の実測(`1e-16` 台)とは15桁以上離れている。
    #[test]
    fn a_packet_technique_that_tears_the_paper_is_rejected_before_the_pose_scan() {
        let (document, faces, state) = preliminary_base();
        let input = TechniqueInput {
            flap: vec![state.order[0]],
            line: [[0.0, 1.0], [0.5, 0.5]],
            reference_point: [0.5, 0.5],
            open_to_back: None,
            polygon: None,
            center: None,
        };

        // 技法そのものは止めずに手順を返す(`CLAUDE.md` §8「止めずに警告する」)。
        let mut cp = document.cp.clone();
        let result = petal(&mut cp, &faces, &state, &input).expect("技法は止めずに続ける");
        let torn = torn_creases(&cp, &result.state);
        assert_eq!(torn.len(), 2, "裂けた折り目 {torn:?}");
        let widest = torn.iter().map(|&(_, gap)| gap).fold(0.0f64, f64::max);
        assert!(widest > 0.8, "裂けた距離 {widest:.4e}(紙の長辺は1)");
        assert_eq!(
            opened_lines(&result.step),
            0,
            "裂けている折り目には角度を決められないので、開く動きが1本も記録されない"
        );

        // 探索は、姿勢を1つも見ないうちにこの手を落とす。
        let mut session = FoldSession {
            document,
            faces,
            state,
            lines: Vec::new(),
            fold_lines: Vec::new(),
            folded: 0,
            closed: BTreeSet::new(),
            network_candidates: OnceLock::new(),
        };
        session.rebuild();
        let error = session
            .apply_packet_technique(PacketTechnique::Petal, input)
            .expect_err("紙が裂ける手は候補にしない");
        assert!(
            error.contains("紙が裂ける"),
            "落とした理由が「紙が裂ける」でない: {error}"
        );
    }

    #[test]
    fn inverted_pure_close_flat_pose_is_a_directional_preparation() {
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        insert_segment(&mut document.cp, [0.0, 0.5], [1.0, 0.5], EdgeKind::Mountain);
        let session = FoldSession::new(&document).expect("中央折りのsessionを作れない");
        let line = session.fold_lines.first().expect("中央折り線がない");
        let relations = line
            .edges
            .iter()
            .map(|edge| {
                (
                    *edge,
                    PacketEdgeRelation {
                        internal: true,
                        boundary: false,
                    },
                )
            })
            .collect();
        let candidate = session
            .flat_pose_candidate(
                &[line.id],
                &relations,
                &std::collections::BTreeSet::new(),
                false,
                true,
            )
            .expect("開いた線を反対符号へ閉じる候補がない");

        assert!(matches!(
            candidate.key,
            CandidateKey::FlatPose {
                directional: true,
                ..
            }
        ));
    }

    #[test]
    fn latest_explicit_zero_target_is_the_same_current_angle_as_implicit_open() {
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        insert_segment(&mut document.cp, [0.0, 0.5], [1.0, 0.5], EdgeKind::Mountain);
        insert_segment(&mut document.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Valley);
        let horizontal = DriverLine {
            a: [0.0, 0.5],
            b: [1.0, 0.5],
            target_angle_deg: 180.0,
        };
        let vertical = DriverLine {
            a: [0.5, 0.0],
            b: [0.5, 1.0],
            target_angle_deg: -180.0,
        };
        let step = |id, drivers| FoldStep {
            id,
            kind: TechniqueKind::Simple,
            drivers,
            layer_order: None,
            alignment: None,
            finish_soft: None,
            note: String::new(),
        };
        document.sequence.push(step(0, vec![horizontal.clone()]));
        let mut opened = horizontal;
        opened.target_angle_deg = 0.0;
        document
            .sequence
            .push(step(1, vec![opened.clone(), vertical.clone()]));

        let targets = saved_angle_targets(&document);
        let expected = resolve_driver_edges(&document.cp, &vertical)
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            targets
                .keys()
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            expected
        );
        assert!(targets.values().all(|target| *target == -180_000_000));

        let mut ever_activated = resolve_driver_edges(&document.cp, &opened)
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        ever_activated.extend(resolve_driver_edges(&document.cp, &vertical));
        assert_eq!(activated_edges(&document), ever_activated);
    }

    #[test]
    fn exposed_layer_packets_are_linear_prefixes_and_suffixes_not_a_power_set() {
        for depth in [1usize, 2, 8, 64] {
            let stack = (0..u32::try_from(depth).expect("depth fits u32")).collect::<Vec<_>>();
            let packets = exposed_packets(&stack);
            assert_eq!(packets.len(), 2 * depth - 1);
            for packet in packets {
                assert!(
                    stack.starts_with(&packet) || stack.ends_with(&packet),
                    "露出していない中間層packetを列挙した: {packet:?}"
                );
            }
        }
    }

    #[test]
    fn exposed_packet_generation_polls_before_finishing_a_deep_stack() {
        let stack = (0..64).collect::<Vec<FaceId>>();
        let mut polls = 0usize;
        let stopped = exposed_packets_until(&stack, &mut || {
            polls += 1;
            polls >= 7
        });
        assert!(stopped.is_none());
        assert_eq!(polls, 7, "深い層山のprefix生成中に期限を見ていない");
    }

    #[test]
    fn stopped_candidate_build_is_not_cached_and_can_be_retried() {
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        insert_segment(&mut document.cp, [0.0, 0.5], [1.0, 0.5], EdgeKind::Mountain);
        let session = FoldSession::new(&document).expect("中央折りのsessionを作れない");

        let mut total_polls = 0usize;
        let (_, timed_out) = session.build_network_candidates_until(&mut || {
            total_polls += 1;
            false
        });
        assert!(!timed_out);
        assert!(
            total_polls >= 4,
            "候補生成が期限を十分な間隔で確認していない"
        );

        let cutoff = (total_polls / 2).max(1);
        let mut polls = 0usize;
        let (partial, timed_out) =
            session.prepared_network_moves_until(PoseScan { steps: 0 }, || {
                polls += 1;
                polls >= cutoff
            });
        assert!(timed_out);
        assert!(partial.is_empty(), "中途構築した候補を公開した");
        assert!(
            session.network_candidates.get().is_none(),
            "中途候補をcacheした"
        );

        let (_, retried_timeout) =
            session.prepared_network_moves_until(PoseScan { steps: 0 }, || false);
        assert!(!retried_timeout);
        assert!(
            session.network_candidates.get().is_some(),
            "完全な再試行をcacheしなかった"
        );
    }

    #[test]
    fn coincident_sets_keep_both_sides_of_a_non_transitive_overlap() {
        let segments = [
            (0, [[0.0, 0.0], [2.0, 0.0]]),
            (1, [[1.0, 0.0], [3.0, 0.0]]),
            (2, [[2.0, 0.0], [4.0, 0.0]]),
        ];
        let sets = coincident_line_sets(&segments);
        assert_eq!(
            sets.into_iter().collect::<Vec<_>>(),
            vec![vec![0, 1], vec![1, 2]]
        );
        assert_eq!(
            coincident_line_components(&segments)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![vec![0, 1, 2]],
            "active setを置換せず、連なる全直線も追加候補にする"
        );
    }

    #[test]
    fn coincident_sets_accept_reversed_diagonals_but_not_point_contact() {
        let diagonal =
            coincident_line_sets(&[(0, [[0.0, 0.0], [2.0, 2.0]]), (1, [[3.0, 3.0], [1.0, 1.0]])]);
        assert_eq!(diagonal.into_iter().collect::<Vec<_>>(), vec![vec![0, 1]]);

        let touching =
            coincident_line_sets(&[(0, [[0.0, 0.0], [1.0, 0.0]]), (1, [[1.0, 0.0], [2.0, 0.0]])]);
        assert!(touching.is_empty());
        let touching_component = coincident_line_components(&[
            (0, [[0.0, 0.0], [1.0, 0.0]]),
            (1, [[1.0, 0.0], [2.0, 0.0]]),
        ]);
        assert!(touching_component.is_empty());
    }

    /// 折った後に面が**増える**のは「面が欠けた」ではない。
    ///
    /// 実際の紙では、折る途中で新しい折り目が生まれる手(花弁折りなど)が普通にある。
    /// 折り目が1本増えれば、その折り目が横切る面は2枚に分かれるので面の数は必ず増える。
    /// 増加まで捨てていたときは、姿勢の検査をすべて通る手でも、姿勢を1点も見ないまま
    /// 落ちていた(実測: 鳥の基本形・深さ2の花弁折り2件が面 14 → 15 で落ちた。
    /// 方向付き単線は最初の状態で49候補中20件)。
    #[test]
    fn only_a_smaller_face_count_means_a_face_was_lost() {
        assert_eq!(face_count_problem(14, 13), Some(PoseProblem::FaceLost {
            expected: 14,
            got: 13,
        }));
        assert_eq!(face_count_problem(14, 0), Some(PoseProblem::FaceLost {
            expected: 14,
            got: 0,
        }));
        assert_eq!(face_count_problem(14, 14), None, "変わらないのは欠けではない");
        assert_eq!(face_count_problem(14, 15), None, "1枚増えるのは欠けではない");
        assert_eq!(face_count_problem(14, 25), None, "11枚増えるのは欠けではない");
        assert_eq!(face_count_problem(29, 47), None, "18枚増えるのは欠けではない");
    }

    /// 折り終えた印は [`MAX_LINES`] 本ぶんしか幅が無い。上限を超えた折り線は
    /// **どのビットも立てない**。見張りが無いと `1 << id` が回り込んで、
    /// 別の折り線を「閉じ終えた」と誤って印を付ける。
    ///
    /// 折り目が増える手を扱えるようにしたので、折り線の本数は手を進めるほど増えうる。
    #[test]
    fn the_folded_mark_never_wraps_around_past_the_bit_width() {
        assert_eq!(folded_bit(0), 1);
        assert_eq!(folded_bit(MAX_LINES - 1), 1 << (MAX_LINES - 1));
        assert_eq!(folded_bit(MAX_LINES), 0, "上限ちょうどで回り込んだ");
        assert_eq!(folded_bit(MAX_LINES + 1), 0, "上限の1本先で回り込んだ");
        assert_eq!(folded_bit(MAX_LINES + 3), 0, "折り線0番のビットへ化けた");

        assert!(folded_bit_is_set(0b101, 0));
        assert!(!folded_bit_is_set(0b101, 1));
        assert!(folded_bit_is_set(0b101, 2));
        assert!(
            !folded_bit_is_set(u128::MAX, MAX_LINES),
            "全ビットが立っていても、幅の外の折り線は閉じ終えていない"
        );
    }

    /// 「この手で閉じ終えた折り線」は、**番号のビット列ではなく辺**で数える。
    ///
    /// 折り線の番号は展開図が変わるたびに付け直されるので、親の番号と後継の番号を
    /// 同じビット列として比べてはいけない。展開図の辺のIDは付け直されないので、
    /// 親の折り線が閉じ終えたかどうかを辺で判定できる。
    #[test]
    fn the_closed_effect_is_counted_by_edges_not_by_line_numbers() {
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        insert_segment(&mut document.cp, [0.0, 0.5], [1.0, 0.5], EdgeKind::Mountain);
        insert_segment(&mut document.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Valley);
        let session = FoldSession::new(&document).expect("折り筋のある紙を読み込めない");
        let lines = session.crease_lines();
        assert_eq!(lines.len(), 2, "折り線のまとまりが2本になっていない");

        let all_edges = lines
            .iter()
            .flat_map(|line| line.edges.iter().copied())
            .collect::<std::collections::BTreeSet<_>>();
        let first_only = lines[0].edges.iter().copied().collect();
        let nothing = std::collections::BTreeSet::new();

        assert_eq!(
            closed_effect(lines, &[0, 1], 0, &all_edges),
            vec![0, 1],
            "2本とも閉じ終えたのに数えられていない"
        );
        assert_eq!(
            closed_effect(lines, &[0, 1], 0, &first_only),
            vec![0],
            "辺が全部閉じていない折り線を閉じ終えたと数えた"
        );
        assert_eq!(
            closed_effect(lines, &[0, 1], 0, &nothing),
            Vec::<usize>::new(),
            "1本も閉じていないのに数えた"
        );

        // 手の前に既に閉じていた線も、`affected`(この手が動かした線)に入っていれば数える。
        // 入っていなければ、この手で新たに閉じたものだけを数える。
        assert_eq!(
            closed_effect(lines, &[0], folded_bit(0), &all_edges),
            vec![0, 1],
            "この手で新たに閉じた線を取りこぼした"
        );
        assert_eq!(
            closed_effect(lines, &[], folded_bit(0) | folded_bit(1), &all_edges),
            Vec::<usize>::new(),
            "この手より前から閉じていた線を、この手の効果として数えた"
        );
    }

    /// `flat_motion` が部品ごとに出す層の除外は、候補を落とす理由にしない。
    ///
    /// 残る4つの目印は、いままでどおり落とす理由にする。
    /// **どの文面をどう扱うかの一覧**であり、片方だけを直すと落ちる。
    #[test]
    fn only_the_notices_that_change_the_finished_shape_stop_a_packet_technique() {
        // 落とす: 折り上がりが指定と違ってしまうもの。
        for blocking in [
            "山谷と重なり順が食い違うため、展開図から折り直したときに形が変わります",
            "中心線から支点が決められません",
            "新しくできた面の親面が特定できないため、置き去りにしました",
            "対象層 7 は現在の面に存在しないため除外しました",
        ] {
            assert!(
                packet_technique_warning_is_blocking(blocking),
                "折り上がりが変わる知らせを通してしまった: {blocking}"
            );
        }

        // 落とさない: 動きの部品ごとに、その部品へ掛からない層を外しただけのもの。
        let skipped = format!("対象層 10 は{PART_LAYER_SKIP_MARK}");
        assert!(
            !packet_technique_warning_is_blocking(&skipped),
            "部品ごとの層の除外で候補を落としている(鳥の基本形が完成しなくなる): {skipped}"
        );

        // 「指定どおりに折ったうえでの注意」は、もともと落とす理由にしていない。
        for passing in [
            "折り線の一部に反対向きの折り線(山/谷)が既にあります(辺ID 29)。折り上がりは同じです",
            "この花弁折りでは、指定した層 8 が折り線の手前側に掛かっていないため動きません(指定のまま続行します)",
            "折り目(辺ID 21)の両側の紙が離れているため、このままでは紙が裂けます(指定のまま続行します)",
        ] {
            assert!(
                !packet_technique_warning_is_blocking(passing),
                "折れるはずの手を文面だけで落としている: {passing}"
            );
        }
    }

    /// [`PART_LAYER_SKIP_MARK`] が `ori3-layers` の一覧に実在し続けること。
    ///
    /// `ori3-layers` は警告の種類を型で分けず、文字列だけを返す。文面が変わると
    /// この目印は誰にも当たらなくなり、**黙って元の誤読へ戻る**。
    /// そうなったらここで落として気づけるようにする。
    #[test]
    fn the_part_layer_skip_notice_is_still_one_of_the_not_as_requested_marks() {
        assert!(
            ori3_layers::fold_through::NOT_AS_REQUESTED_MARKS.contains(&PART_LAYER_SKIP_MARK),
            "`ori3-layers` の文面が変わった。{PART_LAYER_SKIP_MARK:?} が {:?} の中に無い。\
             文面で見分けるのをやめ、`flat_motion` 側で知らせの種類を分けることを検討する",
            ori3_layers::fold_through::NOT_AS_REQUESTED_MARKS
        );
    }

    /// 掴んだ紙の一部が動かない花弁折りでも、折り上がりが裂けないなら候補にする。
    ///
    /// # なぜこの検査が要るか
    ///
    /// 鳥の基本形を完成させる花弁折りは、掴んだ紙(open panel)が4面あり、
    /// そのうち1面が動きの部品の領域に掛からない。`flat_motion` はその面を外して
    /// 知らせを出すが、**折り上がりは参照どおりの花弁折りとまったく同じ**である
    /// (裂け0本・折り線7本・袋の口を `0°` で記録)。
    /// この知らせで候補を落としていたため、**鳥の基本形が完成しなかった**
    /// (`scratchpad/petal-tear-cause-report.md`)。
    ///
    /// # 標本
    ///
    /// `tests/fixtures/cp-bird-base.json`(コミット済み)を、記録の手順の先頭2手
    /// `[2, 7]` だけ進めた予備基本形の状態。そこで露出した紙の panel をそのまま
    /// フラップにして、参照と同じ中心線 `[[0,1],[0.5,0.5]]` で花弁折りする。
    #[test]
    fn a_petal_that_leaves_part_of_the_panel_still_is_kept_as_a_candidate() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/cp-bird-base.json");
        let text = std::fs::read_to_string(path).expect("鳥の基本形の展開図を読めない");
        let cp: ori3_model::CreasePattern =
            serde_json::from_str(&text).expect("鳥の基本形の展開図を解釈できない");
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        document.cp = cp;

        let mut session = FoldSession::new(&document).expect("鳥の基本形を読み込めない");
        for id in [2usize, 7] {
            let mv = session
                .verify_move(id, PoseScan { steps: 1 })
                .unwrap_or_else(|| panic!("記録の手順の {id} 手目が折れない"));
            session.apply(&mv).expect("記録の手順を進められない");
        }

        // 露出している紙のまとまり(panel)のうち、2面以上のものを1つ選ぶ。
        let closed = session.closed.clone();
        let panels = session
            .open_panel_map_until(&closed, &mut || false)
            .expect("紙のまとまりを作れない");
        let mut panel = panels
            .values()
            .filter(|panel| panel.len() >= 2)
            .cloned()
            .collect::<Vec<_>>();
        panel.sort();
        panel.dedup();
        let flap = panel
            .into_iter()
            .find(|panel| {
                let input = TechniqueInput {
                    flap: panel.clone(),
                    line: [[0.0, 1.0], [0.5, 0.5]],
                    reference_point: [0.0, 1.0],
                    open_to_back: Some(false),
                    polygon: None,
                    center: None,
                };
                let mut cp = session.document.cp.clone();
                petal(&mut cp, &session.faces, &session.state, &input).is_ok_and(|result| {
                    result
                        .warnings
                        .iter()
                        .any(|warning| warning.contains(PART_LAYER_SKIP_MARK))
                })
            })
            .expect("掴んだ紙の一部が動かない花弁折りが1つも無い(標本の前提が崩れた)");

        let input = TechniqueInput {
            flap: flap.clone(),
            line: [[0.0, 1.0], [0.5, 0.5]],
            reference_point: [0.0, 1.0],
            open_to_back: Some(false),
            polygon: None,
            center: None,
        };
        let mut cp = session.document.cp.clone();
        let result = petal(&mut cp, &session.faces, &session.state, &input)
            .expect("参照と同じ中心線の花弁折り");

        // 1. 掴んだ紙の一部が動かないという知らせが、確かに出ている。
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains(PART_LAYER_SKIP_MARK)),
            "この標本ではもう部品ごとの層の除外が起きない(検査の前提が崩れた): {:?}",
            result.warnings
        );

        // 2. それでも折り上がりは裂けていない(**測った形**で判定する)。
        let torn = torn_creases(&cp, &result.state);
        assert!(
            torn.is_empty(),
            "掴んだ紙の一部が動かない花弁折りで紙が裂けた: {torn:?}"
        );

        // 3. 袋の口を開く動きが記録されている(＝本物の花弁折りである)。
        assert!(
            opened_lines(&result.step) >= 1,
            "袋の口を開く動きが1本も記録されていない(折り線 {}本、角 {:?})",
            result.step.drivers.len(),
            result
                .step
                .drivers
                .iter()
                .map(|driver| driver.target_angle_deg)
                .collect::<Vec<_>>()
        );

        // 4. だから、姿勢を見る前に候補から落とさない。
        session
            .apply_packet_technique(PacketTechnique::Petal, input)
            .expect("掴んだ紙の一部が動かないだけの花弁折りを落としている");
    }
}
