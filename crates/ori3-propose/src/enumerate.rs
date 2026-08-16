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

use std::collections::{BTreeMap, BTreeSet};

use ori3_cp::{Face, extract_faces};
use ori3_layers::flat_state::FlatState;
use ori3_layers::precrease_collapse::{PrecreaseCollapseInput, collapse_precrease_network};
use ori3_layers::replay::replay;
use ori3_model::{CreasePattern, Document, EdgeId};
use ori3_rigid::{max_seam_gap, self_intersection_pairs};

use crate::plan::{CreaseLine, FoldedMask, crease_lines};
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
    /// [`FoldLine::id`]。
    pub id: usize,
    /// 閉じる直線の端から端(材料座標)。
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

    /// 確かめた手を1つ進める。
    ///
    /// # Errors
    ///
    /// その手がいまの状態で折れない場合(確かめた直後の状態から動いている場合など)。
    pub fn apply(&mut self, mv: &VerifiedMove) -> Result<(), String> {
        let (cp, step) = self.collapse(mv.line)?;
        let mut document = self.document.clone();
        document.cp = cp;
        let id = u32::try_from(document.sequence.len())
            .map_err(|_| "手順が多すぎて番号を振れない".to_string())?;
        let mut step = step;
        step.id = id;
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
        self.document = document;
        self.faces = faces;
        self.state = state;
        self.rebuild();
        Ok(())
    }

    /// この直線を閉じる操作を、複製の上で1回だけ行う。
    fn collapse(
        &self,
        line: [[f64; 2]; 2],
    ) -> Result<(CreasePattern, ori3_model::FoldStep), String> {
        let mut cp = self.document.cp.clone();
        let result = collapse_precrease_network(
            &mut cp,
            &self.faces,
            &self.state,
            &PrecreaseCollapseInput {
                lines: vec![line],
                target_layers: None,
            },
        )?;
        if !result.warnings.is_empty() {
            return Err(format!("折る手続きが警告を出した: {:?}", result.warnings));
        }
        Ok((cp, result.step))
    }

    /// 1つの候補を実際に折って、4つの条件をすべて見る。
    fn try_fold(&self, fold_line: &FoldLine, scan: PoseScan) -> Result<VerifiedMove, Unverified> {
        let (cp, mut step) = self
            .collapse([fold_line.a, fold_line.b])
            .map_err(|_| Unverified::CannotCollapse)?;
        let mut candidate = self.document.clone();
        candidate.cp = cp;
        let id = u32::try_from(candidate.sequence.len())
            .map_err(|_| Unverified::PoseFailed(PoseProblem::StepSkipped))?;
        step.id = id;
        candidate.sequence.push(step);
        let faces = extract_faces(&candidate.cp);
        if faces.len() != self.faces.len() {
            return Err(Unverified::PoseFailed(PoseProblem::FaceLost {
                expected: self.faces.len(),
                got: faces.len(),
            }));
        }

        let up_to = candidate.sequence.len();
        let mut worst_gap: f64 = 0.0;
        let mut worst_pairs = 0usize;
        for i in 0..scan.points() {
            let replayed = replay(&candidate, up_to, scan.at(i));
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
            let gap = max_seam_gap(&candidate.cp, &faces, &replayed.frame);
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
        Ok(VerifiedMove {
            id: fold_line.id,
            line: [fold_line.a, fold_line.b],
            closes: fold_line.closes.clone(),
            mask: self.folded | fold_line.mask,
            max_seam_gap: worst_gap,
            penetrations: worst_pairs,
            poses_checked: scan.points(),
        })
    }

    /// 展開図から、折り線のまとまり・一度に閉じる直線・折り終えた印を作り直す。
    fn rebuild(&mut self) {
        self.lines = crease_lines(&self.document.cp);
        self.fold_lines = build_fold_lines(&self.lines);
        let closed = closed_edges(&self.faces, &self.state);
        self.folded = 0;
        for line in &self.lines {
            if !line.edges.is_empty() && line.edges.iter().all(|e| closed.contains(e)) {
                self.folded |= 1 << line.id;
            }
        }
    }
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
                mask |= 1 << line.id;
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
