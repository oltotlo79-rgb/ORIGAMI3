//! ループ閉包ソルバー: 内部頂点(展開図の内側で折り線が集まる点)があると
//! 面隣接グラフにループができ、全ヒンジ角を独立に指定できない。
//! driver角を固定し、残りのヒンジ角を変数として、非木辺ごとの閉包残差を
//! Gauss-Newton(疎ヤコビアン+Levenberg減衰、最大50反復)で最小化する。
//!
//! # 閉包拘束の取り方(疎性の根拠)
//!
//! 非木辺(全域木に入らなかったヒンジ)ごとに閉路を1本割り当てる:
//! その非木辺を1回だけ渡り、木辺と「より前に処理した非木辺」だけを使って
//! 元の面へ戻る最短閉路(BFSで探す)。閉路一周のヒンジ折りの合成が恒等に
//! なることが拘束で、残差は合成姿勢と恒等の差(回転行列9成分+並進3成分)。
//!
//! この閉路集合は全域木の基本ループ(両端をLCAで結ぶ木経路)と拘束として
//! 等価: 任意の閉路の一周合成は「各非木辺の基本ループの一周合成の共役」の積で、
//! 非木辺の処理順に帰納すれば、先順の非木辺の閉包が成り立つとき残りは
//! この非木辺の基本ループの共役だけになり、共役の恒等はもとの恒等と同値。
//! 基本ループは格子状のCPで長さO(格子幅)になるが、最短閉路は内部頂点を
//! 囲む長さ4〜8程度に収まるため、残差は少数のヒンジだけに依存し、
//! ヤコビアンも正規方程式も大幅に疎になる。
//!
//! ヤコビアンの各列は回転の軸角微分(dR/dθ = \[u\]×・R)を使った解析式で
//! 厳密に計算する(`side_jacobian`)。正規方程式 JtJ・δ=−Jt・r は疎パターン
//! (同じ閉路に現れる変数対のみ非零)で組み立て、帯幅を狭めるRCM順序付きの
//! 帯(エンベロープ)コレスキー分解で厳密に解く(`EnvelopeCholesky`)。
//! 座標は紙の長辺=1.0に正規化済みなので回転成分と並進成分のスケールは
//! そろっている。

use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};

use glam::{DMat3, DQuat, DVec3};
use ori3_cp::Face;
use ori3_model::{CreasePattern, Driver, EdgeId, EdgeKind, Frame3D};

use crate::tree;

/// 収束判定: 残差のRMS(成分あたりの2乗平均平方根)がこの値未満なら収束。
/// RMSは残差本数でスケールしないため、大規模CPでも厳密解がf64の丸め
/// (ノイズ床、成分あたり〜1e-14)と衝突して不収束扱いになることがない。
/// 完全に畳んだ状態(±180°)の近くでは残差が角度誤差の2乗オーダーになるため、
/// 表示に必要な精度(座標誤差1e-6)を確保できるようノイズ床の10倍に取る。
const TOL_RMS: f64 = 1e-13;
/// Gauss-Newtonの最大反復回数(1段あたり)。
const MAX_ITER: u32 = 50;
/// 零空間内の同順位選択は速く収束するため、対話性能を守る範囲へ制限する。
const NULLSPACE_MAX_ITER: u32 = 10;
/// 従属ヒンジが紙を通り抜けないための物理的な可動限界。
///
/// バリアでは境界へ厳密に到達できないため、Levenberg-Marquardt の各候補を
/// この区間へ射影する。driver は利用者が直接指定する固定値なので対象外。
const DEPENDENT_ANGLE_LIMIT: f64 = std::f64::consts::PI;
/// [`solve_near`] の「目標角へ引くばね」の重みの2乗。閉包残差(座標のスケールは
/// 紙の長辺=1.0)に対してこの重みで角度(ラジアン)のずれを罰する。大きすぎると
/// 閉包を犠牲にして目標へ張り付き、小さすぎると引きが効かず解が遠くへ飛ぶ。
const SPRING_W2: f64 = 1e-2;
/// 閉包精密化で、閉包ヤコビアンの零空間に残る同順位だけを決める数値重み。
/// 物理的な抵抗として閉包を緩めないよう、第1段より十分小さくする。
const NULLSPACE_KEEP_W2: f64 = 1e-14;
/// soft targetの診断へ残す最小偏差（度）。
const RELAXATION_EPS_DEG: f64 = 1e-6;

/// 中優先の希望角から実角が譲った量。永続化せず、1回のsolve結果だけに載せる。
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct AngleRelaxation {
    pub hinge: EdgeId,
    pub target_angle_deg: f64,
    pub actual_angle_deg: f64,
    pub delta_deg: f64,
}

/// `solve` の結果。anglesは全ヒンジの角度(度)で、次回のwarm_startに使える。
/// driverのない自由ヒンジのうちループに乗らないものは、拘束が働かないため
/// warm_start(なければ初期値0度)の値がそのまま載る。
#[derive(Clone, Debug, serde::Serialize)]
pub struct SolveResult {
    pub frame: Frame3D,
    pub converged: bool,
    pub angles: HashMap<EdgeId, f64>,
    /// 返した候補の閉包残差RMS。
    pub closure_rms: f64,
    /// 閉包未収束または接触不可避でも、hardを守った最良の有限候補を返しているか。
    pub best_effort: bool,
    /// 中優先角が希望値から譲った量（辺ID昇順）。
    pub relaxations: Vec<AngleRelaxation>,
    /// 実行した反復回数(warm_startの効果確認・性能調整用)
    pub iterations: u32,
}

#[derive(Clone)]
struct Candidate {
    x: Vec<f64>,
    closure_rms: f64,
    keep_energy: f64,
    warm_distance: f64,
}

#[derive(Clone, Copy)]
struct SolveOptions {
    wrap_updates: bool,
    finish_closure: bool,
    spring_w2: f64,
}

impl SolveOptions {
    const fn new(wrap_updates: bool, finish_closure: bool, spring_w2: f64) -> Self {
        Self {
            wrap_updates,
            finish_closure,
            spring_w2,
        }
    }
}

impl Candidate {
    fn update_if_better(
        best: &mut Option<Self>,
        x: &[f64],
        closure_rms: f64,
        keep_energy: f64,
        warm_seed: &[f64],
    ) {
        if !closure_rms.is_finite() || !keep_energy.is_finite() {
            return;
        }
        let mut warm_distance = 0.0;
        for (&angle, &seed) in x.iter().zip(warm_seed) {
            if !angle.is_finite() {
                return;
            }
            warm_distance += (angle - seed).powi(2);
        }
        if !warm_distance.is_finite() {
            return;
        }
        let candidate = Self {
            x: Vec::new(),
            closure_rms,
            keep_energy,
            warm_distance,
        };
        if best
            .as_ref()
            .is_some_and(|current| !candidate.is_better_than_with_x(current, x))
        {
            return;
        }
        match best {
            Some(current) => {
                current.x.clone_from_slice(x);
                current.closure_rms = closure_rms;
                current.keep_energy = keep_energy;
                current.warm_distance = warm_distance;
            }
            None => {
                *best = Some(Self {
                    x: x.to_vec(),
                    closure_rms,
                    keep_energy,
                    warm_distance,
                });
            }
        }
    }

    fn is_better_than_with_x(&self, other: &Self, x: &[f64]) -> bool {
        let self_closed = self.closure_rms < TOL_RMS;
        let other_closed = other.closure_rms < TOL_RMS;
        match self_closed.cmp(&other_closed) {
            Ordering::Greater => return true,
            Ordering::Less => return false,
            Ordering::Equal => {}
        }
        let primary = if self_closed {
            self.keep_energy.total_cmp(&other.keep_energy)
        } else {
            self.closure_rms.total_cmp(&other.closure_rms)
        };
        match primary {
            Ordering::Less => return true,
            Ordering::Greater => return false,
            Ordering::Equal => {}
        }
        match self.warm_distance.total_cmp(&other.warm_distance) {
            Ordering::Less => return true,
            Ordering::Greater => return false,
            Ordering::Equal => {}
        }
        x.iter()
            .zip(&other.x)
            .find_map(|(a, b)| match a.total_cmp(b) {
                Ordering::Equal => None,
                ordering => Some(ordering == Ordering::Less),
            })
            .unwrap_or(false)
    }
}

/// 閉路を渡り歩く1ステップ(渡る元の面の側の向き付き軸によるヒンジ折り)。
struct FoldOp {
    /// `Forest::hinges` 内の添字
    hinge: usize,
    axis_a: DVec3,
    axis_u: DVec3,
}

/// 非木辺1本の閉包拘束: 非木辺を1回だけ渡り、木辺と先順の非木辺だけで
/// 元の面へ戻る閉路のヒンジ折り列。一周の合成が恒等になることが拘束。
struct LoopWalk {
    ops: Vec<FoldOp>,
}

/// CPと面集合だけで決まる、solve間で共有できる閉包トポロジ。
///
/// 連続追従では開始姿勢の検証と要求姿勢のsolveが同じCPを使うため、この部分を
/// 一度だけ構築する。数値計算用の配列は含めず、各solveの状態は従来どおり独立する。
pub(crate) struct PreparedTopology {
    forest: tree::Forest,
    idx_of: HashMap<EdgeId, usize>,
    loop_walks: Vec<LoopWalk>,
    on_loop: Vec<bool>,
}

pub(crate) fn prepare_topology(cp: &CreasePattern, faces: &[Face]) -> PreparedTopology {
    let forest = tree::build_forest(cp, faces);
    let n = forest.hinges.len();
    let idx_of = forest
        .hinges
        .iter()
        .enumerate()
        .map(|(i, &edge)| (edge, i))
        .collect();

    // 閉包拘束の閉路を作る。BFSの走査はヒンジ添字順なので決定的。
    let mut is_tree = vec![true; n];
    let mut loop_rank = vec![usize::MAX; n];
    for (li, closure) in forest.loops.iter().enumerate() {
        is_tree[closure.hinge] = false;
        loop_rank[closure.hinge] = li;
    }
    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); faces.len()];
    for (hi, occ) in forest.hinge_occ.iter().enumerate() {
        let (f, g) = (occ[0].0, occ[1].0);
        adj[f].push((hi, g));
        adj[g].push((hi, f));
    }
    let axis_on = |hi: usize, face: usize| {
        let occ = &forest.hinge_occ[hi];
        let occurrence = if occ[0].0 == face { occ[0] } else { occ[1] };
        (occurrence.1, occurrence.2)
    };
    let mut loop_walks = Vec::with_capacity(forest.loops.len());
    let mut prev: Vec<Option<(usize, usize)>> = vec![None; faces.len()];
    let mut visited = vec![false; faces.len()];
    let mut queue = VecDeque::new();
    let mut back = Vec::new();
    for (li, closure) in forest.loops.iter().enumerate() {
        prev.fill(None);
        visited.fill(false);
        queue.clear();
        visited[closure.to] = true;
        queue.push_back(closure.to);
        'bfs: while let Some(cur) = queue.pop_front() {
            for &(hi, next) in &adj[cur] {
                if visited[next] || !(is_tree[hi] || loop_rank[hi] < li) {
                    continue;
                }
                visited[next] = true;
                prev[next] = Some((cur, hi));
                if next == closure.from {
                    break 'bfs;
                }
                queue.push_back(next);
            }
        }
        back.clear();
        let mut cur = closure.from;
        while cur != closure.to {
            let (previous, hi) = prev[cur].expect("木辺だけでも必ず届く");
            back.push((previous, hi));
            cur = previous;
        }
        let mut ops = Vec::with_capacity(back.len() + 1);
        ops.push(FoldOp {
            hinge: closure.hinge,
            axis_a: closure.axis_a,
            axis_u: closure.axis_u,
        });
        for &(source, hi) in back.iter().rev() {
            let (axis_a, axis_u) = axis_on(hi, source);
            ops.push(FoldOp {
                hinge: hi,
                axis_a,
                axis_u,
            });
        }
        loop_walks.push(LoopWalk { ops });
    }

    let mut on_loop = vec![false; n];
    for walk in &loop_walks {
        for op in &walk.ops {
            on_loop[op.hinge] = true;
        }
    }
    PreparedTopology {
        forest,
        idx_of,
        loop_walks,
        on_loop,
    }
}

/// opsの折りを恒等姿勢から順に合成した姿勢を返す。
fn chain_ops(ops: &[FoldOp], x: &[f64]) -> (DMat3, DVec3) {
    let mut cur = (DMat3::IDENTITY, DVec3::ZERO);
    for op in ops {
        cur = tree::fold_child(cur.0, cur.1, op.axis_a, op.axis_u, x[op.hinge]);
    }
    cur
}

/// 1閉路の閉包残差12成分(回転行列−恒等の9+並進3)を`out`へ書き込む。
fn eval_loop(lw: &LoopWalk, x: &[f64], out: &mut [f64]) {
    let (r, t) = chain_ops(&lw.ops, x);
    out[..9].copy_from_slice(&r.to_cols_array());
    out[0] -= 1.0;
    out[4] -= 1.0;
    out[8] -= 1.0;
    out[9..12].copy_from_slice(&[t.x, t.y, t.z]);
}

/// 単位軸uの外積行列 \[u\]×。軸角回転の解析微分 dR/dθ = \[u\]×・R に使う。
fn cross_matrix(u: DVec3) -> DMat3 {
    DMat3::from_cols(
        DVec3::new(0.0, u.z, -u.y),
        DVec3::new(-u.z, 0.0, u.x),
        DVec3::new(u.y, -u.x, 0.0),
    )
}

/// 折り列の合成姿勢のヤコビアン列(各ヒンジ12成分)を解析微分で生成し、
/// `emit(ヒンジ添字, 列)`へ順に渡す。合成 A = P∘F_i∘S(P=前置、S=後置)の
/// ヒンジiによる微分は、F_i=(R_i, a−R_i・a)、dR_i/dθ=\[u\]×・R_i から
///   dA.R = P.R・dR_i・S.R
///   dA.t = P.R・dR_i・(S.t − a)
/// となる(Pの並進は定数なので消える)。前進差分と違い厳密なので、±180°近傍の
/// 縮退(残差が角度誤差の2乗)でも勾配を誤らず、平坦到達時の収束減速がない。
/// 計算量は折り列の長さLに対しO(L)(数値微分のO(L²)から改善)。
#[cfg(test)]
fn side_jacobian(ops: &[FoldOp], x: &[f64], sign: f64, emit: impl FnMut(usize, [f64; 12])) {
    let mut rots = Vec::new();
    let mut suffix = Vec::new();
    let mut emit = emit;
    side_jacobian_with_scratch(ops, x, sign, &mut rots, &mut suffix, |_, hinge, col| {
        emit(hinge, col);
    });
}

/// 全閉路で作業領域を使い回す [`side_jacobian`] の実行経路。
/// 面400では1回のsolve中に数千回呼ぶため、閉路ごとの小さなVec確保を避ける。
fn side_jacobian_with_scratch(
    ops: &[FoldOp],
    x: &[f64],
    sign: f64,
    rots: &mut Vec<DMat3>,
    suffix: &mut Vec<(DMat3, DVec3)>,
    mut emit: impl FnMut(usize, usize, [f64; 12]),
) {
    rots.clear();
    for op in ops {
        rots.push(DMat3::from_quat(DQuat::from_axis_angle(
            op.axis_u,
            x[op.hinge],
        )));
    }
    // suffix[i] = F_{i+1}∘…∘F_p(iより後ろの合成)
    suffix.clear();
    suffix.resize(ops.len() + 1, (DMat3::IDENTITY, DVec3::ZERO));
    for (i, op) in ops.iter().enumerate().rev() {
        let (rs, ts) = suffix[i + 1];
        let ti = op.axis_a - rots[i] * op.axis_a;
        suffix[i] = (rots[i] * rs, ti + rots[i] * ts);
    }
    let mut rp = DMat3::IDENTITY; // prefixの回転(並進は微分に不要)
    for (i, op) in ops.iter().enumerate() {
        let dr = cross_matrix(op.axis_u) * rots[i];
        let (rs, ts) = suffix[i + 1];
        let dlin = rp * dr * rs * sign;
        let dtr = rp * (dr * (ts - op.axis_a)) * sign;
        let mut col = [0.0; 12];
        col[..9].copy_from_slice(&dlin.to_cols_array());
        col[9..].copy_from_slice(&dtr.to_array());
        emit(i, op.hinge, col);
        rp *= rots[i];
    }
}

/// driver角を固定し、残りのヒンジ角を変数としてループ閉包残差を最小化する。
///
/// - `warm_start`: 前回解(度)を初期値にする(スライダー連続操作の安定用)。
///   知らない辺IDの項目は無視されるので、CP編集後の古い解を渡しても安全
/// - 初期値の選び方: warm_startがあればそれを最優先。なければ、ループに関与する
///   自由ヒンジ(非木辺と、その閉包閉路を構成するヒンジ)はdriver角の平均の
///   大きさの半分に山谷の符号(山=+、谷=−)を付けた値、それ以外は0度。
///   全部0度(平坦)は山にも谷にも折れ始められる分岐点で勾配がほぼ消え
///   収束が不安定になるため、描かれた山谷の向きへ少し寄せた点から始める
/// - driverを外した自由ヒンジの挙動(意図した仕様): ループに乗らないヒンジには
///   拘束が働かないため変数にならず、warm_startの値(なければ0度)がそのまま
///   解として残る。スライダーからdriverを外した直後も形が跳ねないための挙動で、
///   平らに戻すには0度のdriverを明示するか、warm_startなしで呼ぶ
/// - driver以外の従属ヒンジは初期値と反復ごとの候補を±180°へ射影する。
///   他の折り目へ追従しても紙を通り抜けず、境界の180°で止まる
/// - 不収束時はpanicせず、反復中の最良解で `converged: false` のFrame3Dを返し、
///   warningsに「追従計算が収束していません」を追加する
pub fn solve(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    warm_start: Option<&HashMap<EdgeId, f64>>,
) -> SolveResult {
    let topology = prepare_topology(cp, faces);
    solve_prepared(cp, faces, drivers, warm_start, &topology)
}

pub(crate) fn solve_prepared(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    warm_start: Option<&HashMap<EdgeId, f64>>,
    topology: &PreparedTopology,
) -> SolveResult {
    let clamped = solve_impl_prepared(
        cp,
        faces,
        drivers,
        warm_start,
        None,
        SolveOptions::new(false, false, SPRING_W2),
        topology,
    );
    if clamped.converged {
        clamped
    } else {
        solve_impl_prepared(
            cp,
            faces,
            drivers,
            warm_start,
            None,
            SolveOptions::new(true, false, SPRING_W2),
            topology,
        )
    }
}

/// 「`targets` の角度にいちばん近い、閉じた(紙がつながったままの)形」を求める。
///
/// 折り途中の姿勢を出すための入口。折り角を線形補間しただけの値は、内部頂点の
/// まわりのループ閉包を満たさない(自由度1の四折り頂点で2本以上を勝手な値に
/// すると閉じない)ため、そのまま姿勢にすると面どうしが離れて紙がちぎれて見える。
/// かといって閉包だけを解くと、目標から遠く離れた別の形へ落ちて紙が飛び跳ねる。
///
/// そこで2段階で解く:
/// 1. 中優先の変数だけを `targets` へ長さ比例の抵抗([`SPRING_W2`])で引きながら閉包を解く
/// 2. 閉包を厳密に詰めつつ、数値的な零空間内だけで希望エネルギーを下げる
///
/// `targets` は全ヒンジの角度(度)。ループに乗らないヒンジは変数にならないので
/// 指定した値がそのまま残る。角度指定([`Driver`])を併用してもよい。
///
/// `warm_start` は初期値(なければ `targets`)。目標を少しずつ動かしながら前の解を
/// 渡していくと、対称な2つの解のあいだで解が飛び移らない連続した動きになる。
pub fn solve_near(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: &HashMap<EdgeId, f64>,
    warm_start: Option<&HashMap<EdgeId, f64>>,
) -> SolveResult {
    solve_near_with_spring_weight(cp, faces, drivers, targets, warm_start, SPRING_W2)
}

pub(crate) fn solve_near_prepared(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: &HashMap<EdgeId, f64>,
    warm_start: Option<&HashMap<EdgeId, f64>>,
    topology: &PreparedTopology,
) -> SolveResult {
    solve_near_with_spring_weight_prepared(
        cp, faces, drivers, targets, warm_start, SPRING_W2, topology,
    )
}

pub(crate) fn solve_near_with_spring_weight(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: &HashMap<EdgeId, f64>,
    warm_start: Option<&HashMap<EdgeId, f64>>,
    spring_w2: f64,
) -> SolveResult {
    let topology = prepare_topology(cp, faces);
    solve_near_with_spring_weight_prepared(
        cp, faces, drivers, targets, warm_start, spring_w2, &topology,
    )
}

fn solve_near_with_spring_weight_prepared(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: &HashMap<EdgeId, f64>,
    warm_start: Option<&HashMap<EdgeId, f64>>,
    spring_w2: f64,
    topology: &PreparedTopology,
) -> SolveResult {
    let clamped = solve_impl_prepared(
        cp,
        faces,
        drivers,
        warm_start,
        Some(targets),
        SolveOptions::new(false, false, spring_w2),
        topology,
    );
    if clamped.converged {
        clamped
    } else {
        solve_impl_prepared(
            cp,
            faces,
            drivers,
            warm_start,
            Some(targets),
            SolveOptions::new(true, false, spring_w2),
            topology,
        )
    }
}

/// [`solve_near`] と同じ優先度で解き、soft抵抗を完全に外す最終閉包段も行う。
///
/// 保存された複合手順の補間値のように、soft目標自体が閉包多様体上にない場合の
/// 表示再生用。soft付きの2段で収束閾値へ届かなければ、その最良角を保ったまま
/// 純粋な閉包残差を詰める。通常の対話solveは従来の[`solve_near`]を使う。
pub fn solve_near_exact(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: &HashMap<EdgeId, f64>,
    warm_start: Option<&HashMap<EdgeId, f64>>,
) -> SolveResult {
    let topology = prepare_topology(cp, faces);
    solve_near_exact_prepared(cp, faces, drivers, targets, warm_start, &topology)
}

pub(crate) fn solve_near_exact_prepared(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: &HashMap<EdgeId, f64>,
    warm_start: Option<&HashMap<EdgeId, f64>>,
    topology: &PreparedTopology,
) -> SolveResult {
    let clamped = solve_impl_prepared(
        cp,
        faces,
        drivers,
        warm_start,
        Some(targets),
        SolveOptions::new(false, true, SPRING_W2),
        topology,
    );
    if clamped.converged {
        clamped
    } else {
        solve_impl_prepared(
            cp,
            faces,
            drivers,
            warm_start,
            Some(targets),
            SolveOptions::new(true, true, SPRING_W2),
            topology,
        )
    }
}

fn solve_impl_prepared(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    warm_start: Option<&HashMap<EdgeId, f64>>,
    targets: Option<&HashMap<EdgeId, f64>>,
    options: SolveOptions,
    topology: &PreparedTopology,
) -> SolveResult {
    let SolveOptions {
        wrap_updates,
        finish_closure,
        spring_w2,
    } = options;
    let PreparedTopology {
        forest,
        idx_of,
        loop_walks,
        on_loop,
    } = topology;
    let n = forest.hinges.len();
    let mut warnings = Vec::new();

    // driver角(ラジアン)を固定値として登録
    let mut fixed: Vec<Option<f64>> = vec![None; n];
    for drv in drivers {
        if !drv.target_angle_deg.is_finite() || !(-180.0..=180.0).contains(&drv.target_angle_deg) {
            warnings.push(format!(
                "辺ID {} の角度指定は有限な-180°以上180°以下ではないため、無視します",
                drv.hinge
            ));
            continue;
        }
        match idx_of.get(&drv.hinge) {
            Some(&i) => fixed[i] = Some(drv.target_angle_deg.to_radians()),
            None => warnings.push(format!(
                "辺ID {} は折り線(2面の境)ではないため、角度指定を無視します",
                drv.hinge
            )),
        }
    }

    // driver角の平均の大きさ(ラジアン)と山谷符号による初期値バイアス
    let (fixed_abs_sum, fixed_count) = fixed
        .iter()
        .filter_map(|value| *value)
        .fold((0.0, 0usize), |(sum, count), value| {
            (sum + value.abs(), count + 1)
        });
    let mean_drive = if fixed_count == 0 {
        0.0
    } else {
        fixed_abs_sum / fixed_count as f64
    };
    let needs_kind_bias = (0..n).any(|i| {
        fixed[i].is_none()
            && on_loop[i]
            && !warm_start
                .and_then(|values| values.get(&forest.hinges[i]))
                .is_some_and(|value| value.is_finite())
            && !targets
                .and_then(|values| values.get(&forest.hinges[i]))
                .is_some_and(|value| value.is_finite())
    });
    let kinds: Option<HashMap<EdgeId, EdgeKind>> =
        needs_kind_bias.then(|| cp.edges.iter().map(|e| (e.id, e.kind)).collect());
    let kind_sign = |eid: EdgeId| -> f64 {
        match kinds.as_ref().and_then(|values| values.get(&eid)) {
            Some(EdgeKind::Valley) => -1.0,
            _ => 1.0,
        }
    };

    // soft targetは辺ID昇順の配列へ一度だけ移し、hardと重なる項を除外する。
    // HashMapの列挙順は、警告・診断・最良候補の決定順に使わない。
    let mut target_deg = vec![None; n];
    let mut target_rad = vec![None; n];
    for (i, &hinge) in forest.hinges.iter().enumerate() {
        if fixed[i].is_some() {
            continue;
        }
        let Some(&target) = targets.and_then(|values| values.get(&hinge)) else {
            continue;
        };
        if !target.is_finite() {
            warnings.push(format!(
                "辺ID {hinge} の希望角は有限値ではないため、無視します"
            ));
            continue;
        }
        target_deg[i] = Some(target);
        target_rad[i] = Some(target.clamp(-180.0, 180.0).to_radians());
    }

    // 単位紙長当たりの折り目抵抗。soft targetが無い通常solveでは使わないため、
    // 頂点・辺HashMapの構築も省く（400面追従では毎フレームの固定費になる）。
    let target_weight: Vec<f64> = if target_rad.iter().any(Option::is_some) {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        let vertex_positions: HashMap<_, _> = cp
            .vertices
            .iter()
            .map(|vertex| {
                for axis in 0..2 {
                    if vertex.pos[axis].is_finite() {
                        min[axis] = min[axis].min(vertex.pos[axis]);
                        max[axis] = max[axis].max(vertex.pos[axis]);
                    }
                }
                (vertex.id, vertex.pos)
            })
            .collect();
        let paper_length = (max[0] - min[0]).max(max[1] - min[1]).max(1e-12);
        let edge_length_ratio: HashMap<EdgeId, f64> = cp
            .edges
            .iter()
            .filter_map(|edge| {
                let (a, b) = (
                    vertex_positions.get(&edge.v0)?,
                    vertex_positions.get(&edge.v1)?,
                );
                let length = (b[0] - a[0]).hypot(b[1] - a[1]);
                length
                    .is_finite()
                    .then_some((edge.id, length / paper_length))
            })
            .collect();
        forest
            .hinges
            .iter()
            .map(|hinge| edge_length_ratio.get(hinge).copied().unwrap_or(0.0))
            .collect()
    } else {
        vec![0.0; n]
    };

    // 初期値(全ヒンジ分。fixedは固定値で埋める)。従属ヒンジは前回解が
    // 範囲外だった古い状態から始める場合も、最初に物理限界へ戻す。
    let mut x: Vec<f64> = (0..n)
        .map(|i| {
            if let Some(v) = fixed[i] {
                return v;
            }
            if let Some(w) = warm_start.and_then(|m| m.get(&forest.hinges[i]))
                && w.is_finite()
            {
                return w.to_radians();
            }
            if let Some(target) = target_rad[i] {
                return target;
            }
            if on_loop[i] {
                kind_sign(forest.hinges[i]) * mean_drive * 0.5
            } else {
                0.0
            }
        })
        .collect();
    clamp_dependent_angles(&mut x, &fixed);
    let warm_seed = x.clone();

    // 変数 = driver固定でなく、かつ閉路上のヒンジ
    let vars: Vec<usize> = (0..n)
        .filter(|&i| fixed[i].is_none() && on_loop[i])
        .collect();
    let mut var_of: Vec<Option<usize>> = vec![None; n];
    for (vi, &hi) in vars.iter().enumerate() {
        var_of[hi] = Some(vi);
    }
    let k = vars.len();

    // 閉路ごとの変数列(ヤコビアンの非零列パターン)
    let loop_vars: Vec<Vec<usize>> = loop_walks
        .iter()
        .map(|lw| {
            let mut vs: Vec<usize> = lw.ops.iter().filter_map(|op| var_of[op.hinge]).collect();
            vs.sort_unstable();
            vs.dedup();
            vs
        })
        .collect();
    let loop_op_cols: Vec<Vec<Option<usize>>> = loop_walks
        .iter()
        .zip(&loop_vars)
        .map(|(walk, vars)| {
            walk.ops
                .iter()
                .map(|op| var_of[op.hinge].map(|vi| vars.binary_search(&vi).expect("閉路上の変数")))
                .collect()
        })
        .collect();

    // JtJの疎パターン(CSR): 同じ閉路に現れる変数対の位置だけ非零
    let mut nbrs: Vec<Vec<usize>> = vec![Vec::new(); k];
    for vs in &loop_vars {
        for &i in vs {
            nbrs[i].extend(vs.iter().copied());
        }
    }
    let mut row_ptr = Vec::with_capacity(k + 1);
    row_ptr.push(0usize);
    let mut col_idx: Vec<usize> = Vec::new();
    for l in &mut nbrs {
        l.sort_unstable();
        l.dedup();
        col_idx.extend(l.iter().copied());
        row_ptr.push(col_idx.len());
    }

    let m = loop_walks.len() * 12;
    let eval_all = |x: &[f64], r: &mut [f64]| {
        for (li, lw) in loop_walks.iter().enumerate() {
            eval_loop(lw, x, &mut r[12 * li..12 * li + 12]);
        }
    };
    let rms = |cost: f64| {
        if m == 0 {
            0.0
        } else {
            (cost / m as f64).sqrt()
        }
    };

    // 変数にならない自由ヒンジ(閉路に乗らない=拘束が働かない)は目標角そのもの。
    // targetの無いlowはwarm seedのままで、希望ばねを一切持たない。
    for (i, xi) in x.iter_mut().enumerate() {
        if fixed[i].is_none()
            && var_of[i].is_none()
            && let Some(target) = target_rad[i]
        {
            *xi = target;
        }
    }
    let soft_vars: Vec<(usize, usize, f64, usize)> = vars
        .iter()
        .enumerate()
        .filter_map(|(vi, &hi)| {
            target_rad[hi].map(|_| {
                let (lo, end) = (row_ptr[vi], row_ptr[vi + 1]);
                let diagonal = lo + col_idx[lo..end].binary_search(&vi).expect("対角は必ずある");
                (vi, hi, target_weight[hi], diagonal)
            })
        })
        .collect();
    let spring_cost = |x: &[f64], w2: f64| -> f64 {
        if w2 == 0.0 {
            return 0.0;
        }
        let mut sum = 0.0;
        for &(_, hi, length_ratio, _) in &soft_vars {
            sum += length_ratio * (x[hi] - target_rad[hi].expect("soft変数には目標がある")).powi(2);
        }
        w2 * sum
    };
    let mut iterations = 0u32;
    let mut r = vec![0.0; m];
    eval_all(&x, &mut r);
    let mut best_x = x.clone();
    let mut best_candidate = None;
    Candidate::update_if_better(
        &mut best_candidate,
        &x,
        rms(sq_sum(&r)),
        spring_cost(&x, spring_w2),
        &warm_seed,
    );

    if k > 0 && m > 0 {
        let mut vals = vec![0.0; col_idx.len()];
        let mut blocks: Vec<Vec<[f64; 12]>> = loop_vars
            .iter()
            .map(|vs| vec![[0.0; 12]; vs.len()])
            .collect();
        let mut chol = EnvelopeCholesky::new(k, &row_ptr, &col_idx);
        // factorが読むのはRCM順の下三角だけ。同じ対称成分を両側で計算したり、
        // 反復ごとにCSR位置を二分探索したりせず、閉路内の列対と格納先を固定する。
        let normal_slots: Vec<Vec<(usize, usize, usize)>> = loop_vars
            .iter()
            .map(|vs| {
                let mut slots = Vec::with_capacity(vs.len() * (vs.len() + 1) / 2);
                for (ci, &vi) in vs.iter().enumerate() {
                    let (lo, hi) = (row_ptr[vi], row_ptr[vi + 1]);
                    for (cj, &vj) in vs.iter().enumerate() {
                        if chol.pos[vi] < chol.pos[vj] {
                            continue;
                        }
                        let slot = lo
                            + col_idx[lo..hi]
                                .binary_search(&vj)
                                .expect("疎パターンに含まれる");
                        slots.push((ci, cj, slot));
                    }
                }
                slots
            })
            .collect();
        let max_loop_ops = loop_walks
            .iter()
            .map(|walk| walk.ops.len())
            .max()
            .unwrap_or(0);
        let mut jacobian_rots = Vec::with_capacity(max_loop_ops);
        let mut jacobian_suffix = Vec::with_capacity(max_loop_ops + 1);
        let mut jtr = vec![0.0; k];
        let mut b = vec![0.0; k];
        let mut delta = vec![0.0; k];
        let mut solve_work = vec![0.0; k];
        let mut xt = x.clone();
        let mut rt = vec![0.0; m];
        // medium保持→零空間内の同順位選択。表示再生ではさらにsoft抵抗を
        // 完全に外す閉包段を足す。soft targetなしは純粋な閉包段だけを行う。
        let phases = if soft_vars.is_empty() {
            vec![0.0]
        } else if finish_closure {
            vec![spring_w2, NULLSPACE_KEEP_W2, 0.0]
        } else {
            vec![spring_w2, NULLSPACE_KEEP_W2]
        };
        for w2 in phases {
            let phase_limit = if w2 == NULLSPACE_KEEP_W2 {
                NULLSPACE_MAX_ITER
            } else {
                MAX_ITER
            };
            let use_continuation_lambda = w2 != NULLSPACE_KEEP_W2
                && targets.is_none()
                && warm_start.is_some()
                && faces.len() > 100;
            let mut lambda = if w2 == NULLSPACE_KEEP_W2 {
                1e-15
            } else if use_continuation_lambda {
                // 追従中は既に直前の閉包解の近傍にいる。強い初期減衰で小刻みに
                // 7回進む代わりにNewton stepから始め、悪化時は従来どおり10倍して戻す。
                1e-6
            } else {
                1e-1
            };
            let mut cost = sq_sum(&r) + spring_cost(&x, w2);
            let mut best_cost = cost;
            best_x.clone_from(&x);
            let mut phase_converged = rms(sq_sum(&r)) < TOL_RMS;
            let mut it = 0u32;
            // ばね付きの段は「閉包が0でも目標へ引く仕事が残っている」ので、
            // 改善が止まる(またはMAX_ITER)まで回す。ばね無しの段は残差0で終わり。
            while it < phase_limit && !(w2 == 0.0 && phase_converged) {
                it += 1;
                iterations += 1;
                // 疎ヤコビアン: 閉路ごとに、その閉路上の変数の列だけを解析微分で作る
                // (全域再伝播なし。閉路外のヒンジの列は零。driver固定ヒンジは捨てる)
                for (li, lw) in loop_walks.iter().enumerate() {
                    let block = &mut blocks[li];
                    block.fill([0.0; 12]);
                    let emit = |op_index: usize, _hinge: usize, col: [f64; 12]| {
                        let Some(c) = loop_op_cols[li][op_index] else {
                            return;
                        };
                        for (row, v) in col.iter().enumerate() {
                            block[c][row] += v;
                        }
                    };
                    side_jacobian_with_scratch(
                        &lw.ops,
                        &x,
                        1.0,
                        &mut jacobian_rots,
                        &mut jacobian_suffix,
                        emit,
                    );
                }
                // 正規方程式の左辺JtJ(CSRへ加算)と右辺Jt・r
                vals.fill(0.0);
                jtr.fill(0.0);
                for (li, vs) in loop_vars.iter().enumerate() {
                    let block = &blocks[li];
                    let rl = &r[12 * li..12 * li + 12];
                    for (ci, &vi) in vs.iter().enumerate() {
                        let mut dot = 0.0;
                        for row in 0..12 {
                            dot += block[ci][row] * rl[row];
                        }
                        jtr[vi] += dot;
                    }
                    for &(ci, cj, slot) in &normal_slots[li] {
                        let mut dot = 0.0;
                        for row in 0..12 {
                            dot += block[ci][row] * block[cj][row];
                        }
                        vals[slot] += dot;
                    }
                }
                // ばね(対角のみ): 目標角からのずれを罰する項を正規方程式へ足す
                if w2 > 0.0 {
                    for &(vi, hi, length_ratio, diagonal) in &soft_vars {
                        let coefficient = w2 * length_ratio;
                        jtr[vi] +=
                            coefficient * (x[hi] - target_rad[hi].expect("soft変数には目標がある"));
                        vals[diagonal] += coefficient;
                    }
                }
                // Levenberg減衰: 残差が減るまでλを10倍しながら更新を試す
                let mut improved = false;
                // The continuation path starts five decades below the legacy damping.  Keep the
                // cheap near-solution case, but retain the same maximum damping for arbitrary
                // public warm starts that are not actually close to a solution.
                let trial_limit = if use_continuation_lambda { 13 } else { 8 };
                for _ in 0..trial_limit {
                    if !chol.factor(&row_ptr, &col_idx, &vals, lambda) {
                        lambda *= 10.0;
                        continue;
                    }
                    for (to, from) in b.iter_mut().zip(&jtr) {
                        *to = -*from;
                    }
                    chol.solve_into(&b, &mut solve_work, &mut delta);
                    if !delta.iter().all(|d| d.is_finite()) {
                        lambda *= 10.0;
                        continue;
                    }
                    if wrap_updates {
                        // rem_euclid is not bit-idempotent.  A rejected LM trial must therefore
                        // start from x again, including free hinges outside every closure loop.
                        xt.clone_from(&x);
                    }
                    for (vi, &hi) in vars.iter().enumerate() {
                        xt[hi] = x[hi] + delta[vi];
                    }
                    if wrap_updates {
                        // Fold angles live on a circle: +π and -π are the same flat geometry.
                        // A second full solve may cross that identified boundary when the usual
                        // physical-interval projection cannot close.  Keeping this as a fallback
                        // preserves the branch selected by legacy clamped continuation whenever
                        // that branch already converges.
                        wrap_dependent_updates(&mut xt, &fixed);
                    } else {
                        clamp_dependent_angles(&mut xt, &fixed);
                    }
                    eval_all(&xt, &mut rt);
                    let closure_cost = sq_sum(&rt);
                    Candidate::update_if_better(
                        &mut best_candidate,
                        &xt,
                        rms(closure_cost),
                        spring_cost(&xt, spring_w2),
                        &warm_seed,
                    );
                    let ct = closure_cost + spring_cost(&xt, w2);
                    if ct < cost {
                        x.clone_from(&xt);
                        r.clone_from(&rt);
                        cost = ct;
                        lambda = (lambda * 0.1).max(1e-18);
                        improved = true;
                        break;
                    }
                    lambda *= 10.0;
                }
                if cost < best_cost {
                    best_cost = cost;
                    best_x.clone_from(&x);
                }
                phase_converged = rms(sq_sum(&r)) < TOL_RMS;
                if !improved {
                    break; // 停滞: これ以上残差を減らせない
                }
            }
            // 次の段(および最終姿勢)は、この段でいちばん良かった点から始める
            x.clone_from(&best_x);
            eval_all(&x, &mut r);
        }
    }

    let mut converged = if let Some(candidate) = best_candidate {
        x = candidate.x;
        candidate.closure_rms < TOL_RMS
    } else {
        false
    };
    eval_all(&x, &mut r);
    let closure_rms = rms(sq_sum(&r));

    // 結果の角度(度)。従属ヒンジは反復中から箱制約内にあり、丸め誤差も
    // 最後に詰めて必ず[-180, 180]を返す。driver固定ヒンジだけは従来どおり
    // 指定値と同値な±180°表現へ折り返す。
    let angles: HashMap<EdgeId, f64> = forest
        .hinges
        .iter()
        .enumerate()
        .map(|(i, &e)| {
            let deg = if fixed[i].is_some() {
                wrap_deg(x[i].to_degrees())
            } else {
                x[i].to_degrees().clamp(-180.0, 180.0)
            };
            (e, deg)
        })
        .collect();
    let relaxations: Vec<AngleRelaxation> = forest
        .hinges
        .iter()
        .enumerate()
        .filter_map(|(i, &hinge)| {
            let target = target_deg[i]?;
            let actual = angles[&hinge];
            let delta = canonical_delta_deg(actual, target);
            (delta.abs() >= RELAXATION_EPS_DEG).then_some(AngleRelaxation {
                hinge,
                target_angle_deg: target,
                actual_angle_deg: actual,
                delta_deg: delta,
            })
        })
        .collect();

    // 最終フレームは構築済みの森で一度だけ伝播する(build_forestの二重実行を回避)
    let folded = tree::fold_frame(&forest, faces, &x);
    let mut frame = tree::to_frame3d(cp, faces, &folded);
    frame.warnings.append(&mut warnings);
    let finite_frame = frame_is_finite_and_complete(&frame, faces.len());
    if !finite_frame {
        converged = false;
        frame
            .warnings
            .push("有限な立体形状を生成できませんでした".to_string());
    } else if !converged {
        let location = r
            .chunks_exact(12)
            .enumerate()
            .map(|(li, residual)| (li, sq_sum(residual)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(li, _)| forest.hinges[forest.loops[li].hinge]);
        let detail = location.map_or_else(
            || format!("閉包RMS {closure_rms:.3e}"),
            |hinge| format!("閉包RMS {closure_rms:.3e}、折り目 #{hinge} 付近"),
        );
        frame
            .warnings
            .push(format!("追従計算が収束していません（{detail}）"));
    }
    SolveResult {
        frame,
        converged,
        angles,
        closure_rms,
        best_effort: !converged,
        relaxations,
        iterations,
    }
}

fn sq_sum(v: &[f64]) -> f64 {
    let mut sum = 0.0;
    for &value in v {
        sum += value * value;
    }
    sum
}

fn frame_is_finite_and_complete(frame: &Frame3D, expected_faces: usize) -> bool {
    frame.faces.len() == expected_faces
        && frame.faces.iter().all(|face| {
            face.polygon
                .iter()
                .flatten()
                .all(|coordinate| coordinate.is_finite())
        })
}

/// driverでない全ヒンジを物理的な可動範囲へ射影する。
fn clamp_dependent_angles(x: &mut [f64], fixed: &[Option<f64>]) {
    for (angle, driver) in x.iter_mut().zip(fixed) {
        if driver.is_none() {
            *angle = angle.clamp(-DEPENDENT_ANGLE_LIMIT, DEPENDENT_ANGLE_LIMIT);
        }
    }
}

/// Normalize non-driver LM candidates to the physical angle interval.
///
/// Unlike an out-of-range persisted warm start or requested soft target, an optimizer update may
/// cross the identified +π/-π flat boundary.  Wrapping that update preserves exactly the same
/// rigid geometry while allowing the next iteration to move away from the singular boundary.
fn wrap_dependent_updates(x: &mut [f64], fixed: &[Option<f64>]) {
    for (angle, driver) in x.iter_mut().zip(fixed) {
        if driver.is_none() {
            let original = *angle;
            let wrapped = (original + DEPENDENT_ANGLE_LIMIT)
                .rem_euclid(2.0 * DEPENDENT_ANGLE_LIMIT)
                - DEPENDENT_ANGLE_LIMIT;
            *angle = if wrapped == -DEPENDENT_ANGLE_LIMIT && original > 0.0 {
                DEPENDENT_ANGLE_LIMIT
            } else {
                wrapped
            };
        }
    }
}

/// 角度(度)を[−180, 180]へ折り返す。+180ちょうど(および+180と同値の正の角)は
/// −180ではなく+180を返し、符号を保つ。
fn wrap_deg(d: f64) -> f64 {
    let w = (d + 180.0).rem_euclid(360.0) - 180.0;
    if w == -180.0 && d > 0.0 { 180.0 } else { w }
}

/// 角度差を同値な回転のうち絶対値が最小の `[-180, 180]` へ正規化する。
///
/// 折り角では +180° と -180° が同じ平坦姿勢を表すため、生の差を診断へ載せると
/// 360°譲ったという誤った通知になる。ちょうど180°の差だけは元の向きを保つ。
pub(crate) fn canonical_delta_deg(actual: f64, target: f64) -> f64 {
    let raw = actual - target;
    if (-180.0..=180.0).contains(&raw) {
        return raw;
    }
    let wrapped = (raw + 180.0).rem_euclid(360.0) - 180.0;
    if wrapped == -180.0 && raw > 0.0 {
        180.0
    } else {
        wrapped
    }
}

/// (JtJ + λI)・δ = b の直接法ソルバー: RCM順序付きの帯(エンベロープ)
/// コレスキー分解。順序と帯構造(パターン依存)はsolveごとに1回だけ作り、
/// λを変えた再分解は値の詰め直しと分解のみ行う。
/// JtJはグラム行列でλ>0なら正定値なので、分解は通常成功する
/// (丸めで対角が非正になったら呼び出し側がλを上げて再試行する)。
struct EnvelopeCholesky {
    /// 新添字→旧添字(RCM順)
    order: Vec<usize>,
    /// 旧添字→新添字
    pos: Vec<usize>,
    /// 新行iの帯開始列(この行の非零・フィルインは first[i]..=i に収まる)
    first: Vec<usize>,
    /// 行iの帯の格納開始位置(長さk+1)
    offset: Vec<usize>,
    /// 帯内の値(分解後は下三角L。対角含む)
    data: Vec<f64>,
}

impl EnvelopeCholesky {
    /// CSRパターンからRCM順序と帯構造を作る(値はまだ入れない)。
    fn new(k: usize, row_ptr: &[usize], col_idx: &[usize]) -> Self {
        // RCM: 次数最小の頂点から成分ごとにBFSし、近傍を次数順に並べ、全体を逆順に
        let deg = |i: usize| row_ptr[i + 1] - row_ptr[i];
        let mut order = Vec::with_capacity(k);
        let mut visited = vec![false; k];
        while let Some(start) = (0..k).filter(|&i| !visited[i]).min_by_key(|&i| (deg(i), i)) {
            visited[start] = true;
            let mut queue = VecDeque::from([start]);
            while let Some(cur) = queue.pop_front() {
                order.push(cur);
                let mut nb: Vec<usize> = col_idx[row_ptr[cur]..row_ptr[cur + 1]]
                    .iter()
                    .copied()
                    .filter(|&j| !visited[j])
                    .collect();
                nb.sort_unstable_by_key(|&j| (deg(j), j));
                for j in nb {
                    visited[j] = true;
                    queue.push_back(j);
                }
            }
        }
        order.reverse();
        let mut pos = vec![0usize; k];
        for (ni, &oi) in order.iter().enumerate() {
            pos[oi] = ni;
        }
        // 帯開始列: 行iの非零列(新添字)の最小。コレスキーのフィルインは
        // エンベロープの内側に収まる(古典的な性質)ため、これで格納が足りる
        let mut first: Vec<usize> = (0..k).collect();
        for (ni, &oi) in order.iter().enumerate() {
            for &oj in &col_idx[row_ptr[oi]..row_ptr[oi + 1]] {
                first[ni] = first[ni].min(pos[oj].min(ni));
            }
        }
        let mut offset = Vec::with_capacity(k + 1);
        offset.push(0usize);
        for (i, &f) in first.iter().enumerate() {
            offset.push(offset[i] + (i - f + 1));
        }
        let data = vec![0.0; offset[k]];
        EnvelopeCholesky {
            order,
            pos,
            first,
            offset,
            data,
        }
    }

    #[inline]
    fn at(&self, i: usize, j: usize) -> f64 {
        self.data[self.offset[i] + (j - self.first[i])]
    }

    /// JtJ(CSR)+λIを帯へ詰め直してコレスキー分解する。
    /// 丸めで対角が非正になったらfalse(呼び出し側はλを上げて再試行する)。
    fn factor(&mut self, row_ptr: &[usize], col_idx: &[usize], vals: &[f64], lambda: f64) -> bool {
        self.data.fill(0.0);
        let k = self.order.len();
        for (ni, &oi) in self.order.iter().enumerate() {
            let row_start = row_ptr[oi];
            let row_end = row_ptr[oi + 1];
            for t in 0..row_end - row_start {
                let oj = col_idx[row_start + t];
                let nj = self.pos[oj];
                if nj <= ni {
                    self.data[self.offset[ni] + (nj - self.first[ni])] = vals[row_start + t];
                }
            }
            self.data[self.offset[ni] + (ni - self.first[ni])] += lambda;
        }
        for i in 0..k {
            let i_offset = self.offset[i];
            let i_first = self.first[i];
            for j in self.first[i]..=i {
                let j_offset = self.offset[j];
                let j_first = self.first[j];
                let start = i_first.max(j_first);
                let mut sum = self.data[i_offset + (j - i_first)];
                for t in start..j {
                    sum -=
                        self.data[i_offset + (t - i_first)] * self.data[j_offset + (t - j_first)];
                }
                let v = if j < i {
                    sum / self.data[j_offset + (j - j_first)]
                } else {
                    if sum <= 1e-300 {
                        return false;
                    }
                    sum.sqrt()
                };
                self.data[i_offset + (j - i_first)] = v;
            }
        }
        true
    }

    /// 分解済みのLで (JtJ+λI)・x = b を解く(前進・後退代入)。
    fn solve_into(&self, b: &[f64], work: &mut Vec<f64>, out: &mut Vec<f64>) {
        let k = self.order.len();
        // 前進代入 L・y = P・b(行iの帯の対角を除く部分が列 first[i]..i に対応)
        work.clear();
        for &oi in &self.order {
            work.push(b[oi]);
        }
        for i in 0..k {
            let f = self.first[i];
            let row = &self.data[self.offset[i]..self.offset[i + 1] - 1];
            let mut s = 0.0;
            for (l, v) in row.iter().zip(&work[f..i]) {
                s += l * v;
            }
            work[i] = (work[i] - s) / self.at(i, i);
        }
        // 後退代入 Lᵀ・z = y(行方向の走査で列アクセスを避ける)
        for i in (0..k).rev() {
            let zi = work[i] / self.at(i, i);
            work[i] = zi;
            let f = self.first[i];
            let row = &self.data[self.offset[i]..self.offset[i + 1] - 1];
            for (l, v) in row.iter().zip(work[f..i].iter_mut()) {
                *v -= l * zi;
            }
        }
        // 順序を元に戻す
        out.clear();
        out.resize(k, 0.0);
        for (ni, &oi) in self.order.iter().enumerate() {
            out[oi] = work[ni];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEPENDENT_ANGLE_LIMIT, FoldOp, LoopWalk, canonical_delta_deg, eval_loop, side_jacobian,
        wrap_deg, wrap_dependent_updates,
    };
    use glam::DVec3;

    /// 解析ヤコビアン(side_jacobian)が数値微分(中心差分)と一致することの
    /// 検証。折り軸が平行でない3ヒンジの折り列で全列を突き合わせる。
    #[test]
    fn analytic_jacobian_matches_central_difference() {
        let op = |hinge: usize, a: [f64; 3], u: [f64; 3]| FoldOp {
            hinge,
            axis_a: DVec3::from(a),
            axis_u: DVec3::from(u).normalize(),
        };
        let lw = LoopWalk {
            ops: vec![
                op(0, [0.2, 0.1, 0.0], [1.0, 0.0, 0.0]),
                op(1, [0.5, 0.3, 0.0], [0.6, 0.8, 0.0]),
                op(2, [0.7, 0.2, 0.0], [0.0, 1.0, 0.0]),
            ],
        };
        let x = [0.7, -1.2, 2.9];
        let mut jac = [[0.0f64; 12]; 3];
        side_jacobian(&lw.ops, &x, 1.0, |hinge, col| {
            for (dst, v) in jac[hinge].iter_mut().zip(&col) {
                *dst += v;
            }
        });

        let h = 1e-6;
        for (hi, cols) in jac.iter().enumerate() {
            let (mut xp, mut xm) = (x, x);
            xp[hi] += h;
            xm[hi] -= h;
            let (mut rp, mut rm) = ([0.0; 12], [0.0; 12]);
            eval_loop(&lw, &xp, &mut rp);
            eval_loop(&lw, &xm, &mut rm);
            for (row, expect) in cols.iter().enumerate() {
                let num = (rp[row] - rm[row]) / (2.0 * h);
                assert!(
                    (num - expect).abs() < 1e-8,
                    "ヒンジ{hi}行{row}: 数値={num} 解析={expect}"
                );
            }
        }
    }

    #[test]
    fn wrap_deg_keeps_sign_at_exact_180() {
        assert_eq!(wrap_deg(180.0), 180.0);
        assert_eq!(wrap_deg(-180.0), -180.0);
        assert_eq!(wrap_deg(540.0), 180.0);
        assert_eq!(wrap_deg(-190.0), 170.0);
    }

    #[test]
    fn angle_relaxation_wraps_plus_minus_180() {
        assert_eq!(canonical_delta_deg(180.0, -180.0), 0.0);
        assert_eq!(canonical_delta_deg(-180.0, 180.0), 0.0);
        assert_eq!(canonical_delta_deg(180.0, 540.0), 0.0);
        assert!((canonical_delta_deg(-179.75, 179.75) - 0.5).abs() < 1e-12);
        assert!((canonical_delta_deg(179.75, -179.75) + 0.5).abs() < 1e-12);
    }

    #[test]
    fn dependent_lm_updates_cross_the_identified_flat_boundary() {
        let delta = 0.125;
        let mut angles = [
            DEPENDENT_ANGLE_LIMIT + delta,
            -DEPENDENT_ANGLE_LIMIT - delta,
            DEPENDENT_ANGLE_LIMIT + delta,
        ];
        let fixed = [None, None, Some(0.0)];

        wrap_dependent_updates(&mut angles, &fixed);

        assert!((angles[0] - (-DEPENDENT_ANGLE_LIMIT + delta)).abs() < 1e-15);
        assert!((angles[1] - (DEPENDENT_ANGLE_LIMIT - delta)).abs() < 1e-15);
        assert_eq!(angles[2], DEPENDENT_ANGLE_LIMIT + delta);
    }
}
