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
/// Gauss-Newtonの最大反復回数。
const MAX_ITER: u32 = 50;

/// `solve` の結果。anglesは全ヒンジの角度(度)で、次回のwarm_startに使える。
/// driverのない自由ヒンジのうちループに乗らないものは、拘束が働かないため
/// warm_start(なければ初期値0度)の値がそのまま載る。
#[derive(Clone, Debug, serde::Serialize)]
pub struct SolveResult {
    pub frame: Frame3D,
    pub converged: bool,
    pub angles: HashMap<EdgeId, f64>,
    /// 実行した反復回数(warm_startの効果確認・性能調整用)
    pub iterations: u32,
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
fn side_jacobian(ops: &[FoldOp], x: &[f64], sign: f64, mut emit: impl FnMut(usize, [f64; 12])) {
    let rots: Vec<DMat3> = ops
        .iter()
        .map(|op| DMat3::from_quat(DQuat::from_axis_angle(op.axis_u, x[op.hinge])))
        .collect();
    // suffix[i] = F_{i+1}∘…∘F_p(iより後ろの合成)
    let mut suffix = vec![(DMat3::IDENTITY, DVec3::ZERO); ops.len() + 1];
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
        emit(op.hinge, col);
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
/// - 結果の角度は±180°へ折り返すが、warm_startのある自由ヒンジは前回値に
///   最も近い同値角(±360°ずらし)を選ぶ。±180°をまたいだ瞬間に符号が反転して
///   山谷が逆に見えたり、次回のwarm start連鎖が反対枝へ飛ぶのを防ぐ
/// - 不収束時はpanicせず、反復中の最良解で `converged: false` のFrame3Dを返し、
///   warningsに「追従計算が収束していません」を追加する
pub fn solve(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    warm_start: Option<&HashMap<EdgeId, f64>>,
) -> SolveResult {
    let forest = tree::build_forest(cp, faces);
    let n = forest.hinges.len();
    let idx_of: HashMap<EdgeId, usize> = forest
        .hinges
        .iter()
        .enumerate()
        .map(|(i, &e)| (e, i))
        .collect();
    let mut warnings = Vec::new();

    // driver角(ラジアン)を固定値として登録
    let mut fixed: Vec<Option<f64>> = vec![None; n];
    for drv in drivers {
        match idx_of.get(&drv.hinge) {
            Some(&i) => fixed[i] = Some(drv.target_angle_deg.to_radians()),
            None => warnings.push(format!(
                "辺ID {} は折り線(2面の境)ではないため、角度指定を無視します",
                drv.hinge
            )),
        }
    }

    // 閉包拘束の閉路を作る(モジュール冒頭のコメント参照)。BFSの走査は
    // ヒンジ添字順なので決定的。木辺だけでも必ず届くため経路は常に存在する。
    let mut is_tree = vec![true; n];
    let mut loop_rank = vec![usize::MAX; n];
    for (li, l) in forest.loops.iter().enumerate() {
        is_tree[l.hinge] = false;
        loop_rank[l.hinge] = li;
    }
    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); faces.len()]; // (ヒンジ, 相手面)
    for (hi, occ) in forest.hinge_occ.iter().enumerate() {
        let (f, g) = (occ[0].0, occ[1].0);
        adj[f].push((hi, g));
        adj[g].push((hi, f));
    }
    let axis_on = |hi: usize, face: usize| {
        let occ = &forest.hinge_occ[hi];
        let o = if occ[0].0 == face { occ[0] } else { occ[1] };
        (o.1, o.2)
    };
    let loop_walks: Vec<LoopWalk> = forest
        .loops
        .iter()
        .enumerate()
        .map(|(li, l)| {
            // BFS: l.to から l.from へ(使える辺: 木辺と先順の非木辺)
            let mut prev: Vec<Option<(usize, usize)>> = vec![None; faces.len()]; // (元の面, ヒンジ)
            let mut visited = vec![false; faces.len()];
            let mut queue = VecDeque::new();
            visited[l.to] = true;
            queue.push_back(l.to);
            'bfs: while let Some(cur) = queue.pop_front() {
                for &(hi, nb) in &adj[cur] {
                    if visited[nb] || !(is_tree[hi] || loop_rank[hi] < li) {
                        continue;
                    }
                    visited[nb] = true;
                    prev[nb] = Some((cur, hi));
                    if nb == l.from {
                        break 'bfs;
                    }
                    queue.push_back(nb);
                }
            }
            // 経路を復元し、渡る元の面の側の軸で折り列にする。
            // 一周: まず非木辺を from→to へ渡り、経路で to→…→from と戻る
            let mut back = Vec::new(); // (渡る元の面, ヒンジ)。from側から遡った逆順
            let mut cur = l.from;
            while cur != l.to {
                let (p, hi) = prev[cur].expect("木辺だけでも必ず届く");
                back.push((p, hi));
                cur = p;
            }
            let mut ops = vec![FoldOp {
                hinge: l.hinge,
                axis_a: l.axis_a,
                axis_u: l.axis_u,
            }];
            for &(src, hi) in back.iter().rev() {
                let (a, u) = axis_on(hi, src);
                ops.push(FoldOp {
                    hinge: hi,
                    axis_a: a,
                    axis_u: u,
                });
            }
            LoopWalk { ops }
        })
        .collect();

    // ループ(閉路)上のヒンジ=初期値バイアスの適用範囲。
    // 閉路に乗らないヒンジ(ぶら下がりの面へ向かう木辺など)は残差に影響しない
    // =初期値がそのまま解として残るため、バイアスを掛けてはいけない。
    let mut on_loop = vec![false; n];
    for lw in &loop_walks {
        for op in &lw.ops {
            on_loop[op.hinge] = true;
        }
    }

    // driver角の平均の大きさ(ラジアン)と山谷符号による初期値バイアス
    let fixed_vals: Vec<f64> = fixed.iter().filter_map(|v| *v).collect();
    let mean_drive = if fixed_vals.is_empty() {
        0.0
    } else {
        fixed_vals.iter().map(|v| v.abs()).sum::<f64>() / fixed_vals.len() as f64
    };
    let kinds: HashMap<EdgeId, EdgeKind> = cp.edges.iter().map(|e| (e.id, e.kind)).collect();
    let kind_sign = |eid: EdgeId| -> f64 {
        match kinds.get(&eid) {
            Some(EdgeKind::Valley) => -1.0,
            _ => 1.0,
        }
    };

    // 初期値(全ヒンジ分。fixedは固定値で埋める)
    let mut x: Vec<f64> = (0..n)
        .map(|i| {
            if let Some(v) = fixed[i] {
                return v;
            }
            if let Some(w) = warm_start.and_then(|m| m.get(&forest.hinges[i])) {
                return w.to_radians();
            }
            if on_loop[i] {
                kind_sign(forest.hinges[i]) * mean_drive * 0.5
            } else {
                0.0
            }
        })
        .collect();

    // 変数 = driver固定でなく、かつ閉路上のヒンジ
    let vars: Vec<usize> = (0..n).filter(|&i| fixed[i].is_none() && on_loop[i]).collect();
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

    let mut iterations = 0u32;
    let mut r = vec![0.0; m];
    eval_all(&x, &mut r);
    let mut cost = sq_sum(&r);
    let mut converged = rms(cost) < TOL_RMS;
    let mut best_cost = cost;
    let mut best_x = x.clone();

    if k > 0 && m > 0 {
        let mut lambda = 1e-3;
        let mut vals = vec![0.0; col_idx.len()];
        let mut blocks: Vec<Vec<f64>> =
            loop_vars.iter().map(|vs| vec![0.0; 12 * vs.len()]).collect();
        let mut chol = EnvelopeCholesky::new(k, &row_ptr, &col_idx);
        while !converged && iterations < MAX_ITER {
            iterations += 1;
            // 疎ヤコビアン: 閉路ごとに、その閉路上の変数の列だけを解析微分で作る
            // (全域再伝播なし。閉路外のヒンジの列は零。driver固定ヒンジは捨てる)
            for (li, lw) in loop_walks.iter().enumerate() {
                let vs = &loop_vars[li];
                let w = vs.len();
                let block = &mut blocks[li];
                block.fill(0.0);
                let emit = |hinge: usize, col: [f64; 12]| {
                    let Some(vi) = var_of[hinge] else { return };
                    let c = vs.binary_search(&vi).expect("閉路上の変数");
                    for (row, v) in col.iter().enumerate() {
                        block[row * w + c] += v;
                    }
                };
                side_jacobian(&lw.ops, &x, 1.0, emit);
            }
            // 正規方程式の左辺JtJ(CSRへ加算)と右辺Jt・r
            vals.fill(0.0);
            let mut jtr = vec![0.0; k];
            for (li, vs) in loop_vars.iter().enumerate() {
                let w = vs.len();
                let block = &blocks[li];
                let rl = &r[12 * li..12 * li + 12];
                for (ci, &vi) in vs.iter().enumerate() {
                    jtr[vi] += (0..12).map(|row| block[row * w + ci] * rl[row]).sum::<f64>();
                    let (lo, hi2) = (row_ptr[vi], row_ptr[vi + 1]);
                    for (cj, &vj) in vs.iter().enumerate() {
                        let s: f64 = (0..12)
                            .map(|row| block[row * w + ci] * block[row * w + cj])
                            .sum();
                        let pos = lo
                            + col_idx[lo..hi2]
                                .binary_search(&vj)
                                .expect("疎パターンに含まれる");
                        vals[pos] += s;
                    }
                }
            }
            // Levenberg減衰: 残差が減るまでλを10倍しながら更新を試す
            let mut improved = false;
            for _ in 0..8 {
                if !chol.factor(&row_ptr, &col_idx, &vals, lambda) {
                    lambda *= 10.0;
                    continue;
                }
                let b: Vec<f64> = jtr.iter().map(|v| -v).collect();
                let delta = chol.solve(&b);
                if !delta.iter().all(|d| d.is_finite()) {
                    lambda *= 10.0;
                    continue;
                }
                let mut xt = x.clone();
                for (vi, &hi) in vars.iter().enumerate() {
                    xt[hi] += delta[vi];
                }
                let mut rt = vec![0.0; m];
                eval_all(&xt, &mut rt);
                let ct = sq_sum(&rt);
                if ct < cost {
                    x = xt;
                    r = rt;
                    cost = ct;
                    lambda = (lambda * 0.1).max(1e-12);
                    improved = true;
                    break;
                }
                lambda *= 10.0;
            }
            if cost < best_cost {
                best_cost = cost;
                best_x = x.clone();
            }
            converged = rms(cost) < TOL_RMS;
            if !improved && !converged {
                break; // 停滞: これ以上残差を減らせない
            }
        }
        x = best_x;
    }

    // 結果の角度(度)。±180°へ折り返すが、warm startのある自由ヒンジは
    // 前回値に最も近い同値角を選ぶ(±180°またぎの符号反転防止)。
    // driver固定ヒンジは指定値そのまま(折り返しのみ)。
    let angles: HashMap<EdgeId, f64> = forest
        .hinges
        .iter()
        .enumerate()
        .map(|(i, &e)| {
            let prev = if fixed[i].is_some() {
                None
            } else {
                warm_start.and_then(|w| w.get(&e).copied())
            };
            (e, unwrap_deg(x[i].to_degrees(), prev))
        })
        .collect();

    // 最終フレームは構築済みの森で一度だけ伝播する(build_forestの二重実行を回避)
    let folded = tree::fold_frame(&forest, faces, &x);
    let mut frame = tree::to_frame3d(cp, faces, &folded);
    frame.warnings.append(&mut warnings);
    if !converged {
        frame
            .warnings
            .push("追従計算が収束していません".to_string());
    }
    SolveResult {
        frame,
        converged,
        angles,
        iterations,
    }
}

fn sq_sum(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum()
}

/// 角度(度)を[−180, 180]へ折り返す。+180ちょうど(および+180と同値の正の角)は
/// −180ではなく+180を返し、符号を保つ。
fn wrap_deg(d: f64) -> f64 {
    let w = (d + 180.0).rem_euclid(360.0) - 180.0;
    if w == -180.0 && d > 0.0 { 180.0 } else { w }
}

/// 折り返した角度のうち、前回値`prev`に最も近い同値角(±360°ずらし)を返す。
/// warm start連鎖で±180°をまたいだとき、同じ回転なのに符号が反転した表現
/// (山谷が逆に見える反対枝)へ飛ぶのを防ぐ。`prev`がなければ通常の折り返し。
fn unwrap_deg(deg: f64, prev: Option<f64>) -> f64 {
    let w = wrap_deg(deg);
    let Some(p) = prev else { return w };
    let mut best = w;
    for cand in [w - 360.0, w + 360.0] {
        if (cand - p).abs() < (best - p).abs() {
            best = cand;
        }
    }
    best
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
        while let Some(start) = (0..k)
            .filter(|&i| !visited[i])
            .min_by_key(|&i| (deg(i), i))
        {
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
            for (t, &oj) in col_idx[row_ptr[oi]..row_ptr[oi + 1]].iter().enumerate() {
                let nj = self.pos[oj];
                if nj <= ni {
                    self.data[self.offset[ni] + (nj - self.first[ni])] = vals[row_ptr[oi] + t];
                }
            }
            self.data[self.offset[ni] + (ni - self.first[ni])] += lambda;
        }
        for i in 0..k {
            for j in self.first[i]..=i {
                let start = self.first[i].max(self.first[j]);
                let mut sum = self.at(i, j);
                for t in start..j {
                    sum -= self.at(i, t) * self.at(j, t);
                }
                let v = if j < i {
                    sum / self.at(j, j)
                } else {
                    if sum <= 1e-300 {
                        return false;
                    }
                    sum.sqrt()
                };
                self.data[self.offset[i] + (j - self.first[i])] = v;
            }
        }
        true
    }

    /// 分解済みのLで (JtJ+λI)・x = b を解く(前進・後退代入)。
    fn solve(&self, b: &[f64]) -> Vec<f64> {
        let k = self.order.len();
        // 前進代入 L・y = P・b(行iの帯の対角を除く部分が列 first[i]..i に対応)
        let mut y: Vec<f64> = self.order.iter().map(|&oi| b[oi]).collect();
        for i in 0..k {
            let f = self.first[i];
            let row = &self.data[self.offset[i]..self.offset[i + 1] - 1];
            let s: f64 = row.iter().zip(&y[f..i]).map(|(l, v)| l * v).sum();
            y[i] = (y[i] - s) / self.at(i, i);
        }
        // 後退代入 Lᵀ・z = y(行方向の走査で列アクセスを避ける)
        for i in (0..k).rev() {
            let zi = y[i] / self.at(i, i);
            y[i] = zi;
            let f = self.first[i];
            let row = &self.data[self.offset[i]..self.offset[i + 1] - 1];
            for (l, v) in row.iter().zip(y[f..i].iter_mut()) {
                *v -= l * zi;
            }
        }
        // 順序を元に戻す
        let mut x = vec![0.0; k];
        for (ni, &oi) in self.order.iter().enumerate() {
            x[oi] = y[ni];
        }
        x
    }
}

#[cfg(test)]
mod tests {
    use super::{FoldOp, LoopWalk, eval_loop, side_jacobian, unwrap_deg, wrap_deg};
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
    fn unwrap_prefers_side_near_previous() {
        // ±180°またぎ: 181°はwrapで−179°になるが、前回179°なら181°を保つ
        assert!((unwrap_deg(181.0, Some(179.0)) - 181.0).abs() < 1e-12);
        assert!((unwrap_deg(-181.0, Some(-179.0)) + 181.0).abs() < 1e-12);
        // 前回値が反対側なら通常どおり折り返す
        assert!((unwrap_deg(181.0, Some(-170.0)) + 179.0).abs() < 1e-12);
        // 前回値なしは折り返しのみ
        assert!((unwrap_deg(181.0, None) + 179.0).abs() < 1e-12);
    }
}
