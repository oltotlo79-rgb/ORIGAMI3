//! ループ閉包ソルバー: 内部頂点(展開図の内側で折り線が集まる点)があると
//! 面隣接グラフにループができ、全ヒンジ角を独立に指定できない。
//! driver角を固定し、残りのヒンジ角(非木辺を含む全折りヒンジ)を変数として、
//! 各非木辺のループ閉包残差を Gauss-Newton(数値ヤコビアン+Levenberg減衰、
//! 最大50反復)で最小化する。
//!
//! 残差は非木辺ごとに「木経路で伝播した相手面の姿勢」と「from面からその非木辺の
//! ヒンジ角で折って予測した相手面の姿勢」の差(回転行列9成分+並進3成分)で
//! 構成する。座標は紙の長辺=1.0に正規化済みなので回転成分と並進成分の
//! スケールはそろっており、これはループ一周の回転合成が恒等になる拘束と等価。

use std::collections::HashMap;

use ori3_cp::Face;
use ori3_model::{CreasePattern, Driver, EdgeId, EdgeKind, Frame3D};

use crate::tree;

/// 収束判定: 残差ベクトルの2乗和の平方根がこの値未満なら収束。
/// 完全に畳んだ状態(±180°)の近くでは残差が角度誤差の2乗オーダーになるため、
/// 表示に必要な精度(座標誤差1e-6)を確保できるよう厳しめに取る。
const TOL: f64 = 1e-12;
/// Gauss-Newtonの最大反復回数。
const MAX_ITER: u32 = 50;
/// 数値ヤコビアンの前進差分幅(ラジアン)。
const DIFF_H: f64 = 1e-6;

/// `solve` の結果。anglesは全ヒンジの角度(度)で、次回のwarm_startに使える。
#[derive(Clone, Debug, serde::Serialize)]
pub struct SolveResult {
    pub frame: Frame3D,
    pub converged: bool,
    pub angles: HashMap<EdgeId, f64>,
    /// 実行した反復回数(warm_startの効果確認・性能調整用)
    pub iterations: u32,
}

/// driver角を固定し、残りのヒンジ角を変数としてループ閉包残差を最小化する。
///
/// - `warm_start`: 前回解(度)を初期値にする(スライダー連続操作の安定用)。
///   知らない辺IDの項目は無視されるので、CP編集後の古い解を渡しても安全
/// - 初期値の選び方: warm_startがあればそれを最優先。なければ、ループに関与する
///   自由ヒンジ(非木辺と、その基本ループを構成する木経路上のヒンジ)は
///   driver角の平均の大きさの半分に山谷の符号(山=+、谷=−)を付けた値、
///   それ以外は0度。全部0度(平坦)は山にも谷にも折れ始められる分岐点で
///   勾配がほぼ消え収束が不安定になるため、描かれた山谷の向きへ少し寄せた
///   点から始める。ループに乗らない自由ヒンジは拘束がなく初期値のまま
///   残るので、必ず0度にする
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

    // ループに関与するヒンジ(初期値バイアスの適用範囲)をヒンジ単位で調べる:
    // 各非木辺と、その基本ループ(非木辺の両端の面を結ぶ木経路)上の木辺だけが対象。
    // 同じ連結成分でもループに乗らないヒンジ(ぶら下がりの面へ向かう木辺など)は
    // 残差に影響しない=初期値がそのまま解として残るため、バイアスを掛けてはいけない。
    let mut parent_step: Vec<Option<(usize, usize)>> = vec![None; faces.len()];
    let mut depth = vec![0usize; faces.len()];
    for s in &forest.steps {
        // stepsはBFS順なので親のdepthは確定済み
        parent_step[s.child] = Some((s.parent, s.hinge));
        depth[s.child] = depth[s.parent] + 1;
    }
    let mut on_loop = vec![false; n];
    for l in &forest.loops {
        on_loop[l.hinge] = true;
        // 両端から共通の祖先(LCA)まで木を昇り、経路上のヒンジを印付けする
        let (mut a, mut b) = (l.from, l.to);
        while depth[a] > depth[b] {
            let (p, h) = parent_step[a].expect("depth>0の面には親がある");
            on_loop[h] = true;
            a = p;
        }
        while depth[b] > depth[a] {
            let (p, h) = parent_step[b].expect("depth>0の面には親がある");
            on_loop[h] = true;
            b = p;
        }
        while a != b {
            let (pa, ha) = parent_step[a].expect("LCA到達前の面には親がある");
            on_loop[ha] = true;
            a = pa;
            let (pb, hb) = parent_step[b].expect("LCA到達前の面には親がある");
            on_loop[hb] = true;
            b = pb;
        }
    }

    // driver角の平均の大きさ(ラジアン)と山谷符号による初期値バイアス
    let fixed_vals: Vec<f64> = fixed.iter().filter_map(|v| *v).collect();
    let mean_drive = if fixed_vals.is_empty() {
        0.0
    } else {
        fixed_vals.iter().map(|v| v.abs()).sum::<f64>() / fixed_vals.len() as f64
    };
    let kind_sign = |eid: EdgeId| -> f64 {
        match cp.edges.iter().find(|e| e.id == eid).map(|e| e.kind) {
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

    let residual = |x: &[f64]| -> Vec<f64> {
        let tf = tree::propagate_with(&forest, faces.len(), x);
        let mut r = Vec::with_capacity(forest.loops.len() * 12);
        for l in &forest.loops {
            let (rf, tf_from) = tf[l.from];
            let (pr, pt) = tree::fold_child(rf, tf_from, l.axis_a, l.axis_u, x[l.hinge]);
            let (rt, tt) = tf[l.to];
            r.extend_from_slice(&(pr - rt).to_cols_array());
            let dt = pt - tt;
            r.extend_from_slice(&[dt.x, dt.y, dt.z]);
        }
        r
    };

    let free: Vec<usize> = (0..n).filter(|&i| fixed[i].is_none()).collect();
    let mut iterations = 0u32;
    let mut r = residual(&x);
    let mut cost = sq_sum(&r);
    let mut converged = cost.sqrt() < TOL;
    let mut best_cost = cost;
    let mut best_x = x.clone();

    if !free.is_empty() && !forest.loops.is_empty() {
        let m = r.len();
        let k = free.len();
        let mut lambda = 1e-3;
        while !converged && iterations < MAX_ITER {
            iterations += 1;
            // 数値ヤコビアン(前進差分)
            let mut jac = vec![vec![0.0; k]; m];
            for (j, &fi) in free.iter().enumerate() {
                let orig = x[fi];
                x[fi] = orig + DIFF_H;
                let rj = residual(&x);
                x[fi] = orig;
                for (row, (&a, &b)) in jac.iter_mut().zip(rj.iter().zip(r.iter())) {
                    row[j] = (a - b) / DIFF_H;
                }
            }
            // 正規方程式 JtJ・δ = −Jt・r
            let mut jtj = vec![vec![0.0; k]; k];
            let mut jtr = vec![0.0; k];
            for i in 0..k {
                for l in i..k {
                    let s: f64 = jac.iter().map(|row| row[i] * row[l]).sum();
                    jtj[i][l] = s;
                    jtj[l][i] = s;
                }
                jtr[i] = jac.iter().zip(&r).map(|(row, &rv)| row[i] * rv).sum();
            }
            // Levenberg減衰: 残差が減るまでλを10倍しながら更新を試す
            let mut improved = false;
            for _ in 0..8 {
                let mut a = jtj.clone();
                for (dd, row) in a.iter_mut().enumerate() {
                    row[dd] += lambda;
                }
                let mut delta: Vec<f64> = jtr.iter().map(|v| -v).collect();
                if !solve_linear(&mut a, &mut delta) {
                    lambda *= 10.0;
                    continue;
                }
                let mut xt = x.clone();
                for (j, &fi) in free.iter().enumerate() {
                    xt[fi] += delta[j];
                }
                let rt = residual(&xt);
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
            converged = cost.sqrt() < TOL;
            if !improved && !converged {
                break; // 停滞: これ以上残差を減らせない
            }
        }
        x = best_x;
    }

    // 結果の角度(度)。±180°の範囲へ折り返す(回転として同値)
    let angles: HashMap<EdgeId, f64> = forest
        .hinges
        .iter()
        .enumerate()
        .map(|(i, &e)| (e, wrap_deg(x[i].to_degrees())))
        .collect();
    let folded = tree::propagate(cp, faces, &angles);
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

/// 連立一次方程式 A・x = b を部分ピボット付きガウス消去で解く(bへ上書き)。
/// Aが特異なら false(呼び出し側はλを上げて再試行する)。
fn solve_linear(a: &mut [Vec<f64>], b: &mut [f64]) -> bool {
    let n = b.len();
    for col in 0..n {
        let piv = (col..n)
            .max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))
            .expect("colからnの範囲は空でない");
        if a[piv][col].abs() < 1e-300 {
            return false;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        let (upper, lower) = a.split_at_mut(col + 1);
        let pivot_row = &upper[col];
        for (off, row) in lower.iter_mut().enumerate() {
            let f = row[col] / pivot_row[col];
            if f == 0.0 {
                continue;
            }
            for (rc, &pc) in row[col..].iter_mut().zip(&pivot_row[col..]) {
                *rc -= f * pc;
            }
            b[col + 1 + off] -= f * b[col];
        }
    }
    for col in (0..n).rev() {
        let mut s = b[col];
        for c in col + 1..n {
            s -= a[col][c] * b[c];
        }
        b[col] = s / a[col][col];
    }
    true
}
