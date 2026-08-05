//! 手順の再生: 展開図(CP)と手順列だけから立体の姿勢を求め直す。
//!
//! 3D状態は一切保存しない(SEQ-004「展開図を編集したら手順を自動で再生し直す」の
//! ための設計)。各ステップの [`DriverLine`](ori3_model::DriverLine) は
//! 「CP座標の線分+角度」なので、後続の折りや編集で辺IDが変わっても
//! [`resolve_driver_edges`] で現在の辺へ解決できる。
//!
//! # 求め方
//!
//! 折り畳んだ状態の上に次の折りを重ねるのではなく、**平らな展開図に
//! 「そこまでの全ステップのdriver」をまとめて与えて1回で解く**。
//! `up_to` 未満のステップは目標角そのまま、`up_to` ステップ目だけ角度を `t` 倍する
//! (0→目標への線形補間)。同じ辺を複数のステップが駆動する場合は後のステップが勝つ。
//!
//! **まだ折っていない折り線は0°(平ら)のdriverとして明示的に固定する。**
//! ソルバーは角度指定の無いヒンジを自由変数として扱い、初期値バイアス(山谷の向きへ
//! driver角の平均の半分)から別の枝へ収束させてしまうため、これを省くと
//! 「後続ステップの折り線まで曲がった、警告の出ない誤った形」が返る
//! (`ori3-rigid` の `solve` のdocが書いているとおり、平らに戻すには0°の明示が要る)。
//! 結果として全ヒンジが固定値になるので、ステップごとに解き直しても最後の1回と
//! 同じ姿勢になる。無駄を避けて1回だけ解く(warm startを使わないので決定的)。
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

use std::collections::{BTreeMap, HashMap, HashSet};

use ori3_cp::{Face, extract_faces};
use ori3_model::{Document, Driver, EdgeId, FaceId, Frame3D, StepId};

use crate::flat_state::FlatState;
use crate::fold_through::resolve_driver_edges;

/// [`replay`] の結果。
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReplayResult {
    /// 3D表示用フレーム(`Face3D.layer` は下から0,1,2…)
    pub frame: Frame3D,
    /// 折り線が見つからず飛ばしたステップのID
    pub skipped: Vec<StepId>,
    /// 再生中に出た警告(日本語)
    pub warnings: Vec<String>,
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

    let mut warnings: Vec<String> = Vec::new();
    let mut skipped: Vec<StepId> = Vec::new();
    // 現在の層順序(下→上)。初期状態は面ID昇順。
    let mut order = FlatState::initial(&doc.cp, faces).order;
    // そこまでのステップの角度指定の累積(後から積んだものが優先される)
    let mut drivers: Vec<Driver> = Vec::new();
    let mut driven: HashSet<EdgeId> = HashSet::new();

    for (i, step) in doc.sequence.iter().take(up_to).enumerate() {
        let number = i + 1; // 利用者向けの手順番号は1始まり
        let scale = if number == up_to { t } else { 1.0 };

        let mut resolved_lines = 0usize;
        let mut step_drivers: Vec<Driver> = Vec::new();
        for line in &step.drivers {
            let edges = resolve_driver_edges(&doc.cp, line);
            if edges.is_empty() {
                continue;
            }
            resolved_lines += 1;
            step_drivers.extend(edges.into_iter().map(|hinge| Driver {
                hinge,
                target_angle_deg: line.target_angle_deg * scale,
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
        driven.extend(step_drivers.iter().map(|d| d.hinge));
        drivers.extend(step_drivers);

        // 層順序はステップ完了時にだけ更新する。層順序を持たないステップ(Pose)と、
        // 代表点が1点も現在の面に解決できなかったステップは直前の層順序を保つ。
        if scale >= 1.0
            && let Some(points) = &step.layer_order
            && !points.is_empty()
        {
            let (resolved, mut w) = FlatState::resolve_order(&doc.cp, faces, points);
            // resolve_orderは解決できなかった点ごとにちょうど1件の警告を返すので、
            // 警告の数が点の数と同じなら1点も解決できていない
            if w.len() < points.len() {
                order = resolved;
            }
            warnings.append(&mut w);
        }
    }

    // まだ折っていない折り線は0°(平ら)に固定する。自由変数として残すと初期値
    // バイアスから別の枝へ収束し、警告なしで誤った形が返る(モジュールdoc参照)。
    let mut all: Vec<Driver> = hinge_edges(faces)
        .into_iter()
        .filter(|e| !driven.contains(e))
        .map(|hinge| Driver {
            hinge,
            target_angle_deg: 0.0,
        })
        .collect();
    all.extend(drivers);

    let result = ori3_rigid::solve(&doc.cp, faces, &all, None);
    if !result.converged {
        warnings.push(format!(
            "手順{up_to}までの形が展開図から求まりませんでした(いちばん近い形で表示します)。一部の層だけを折る手順は、展開図からの折り直しでは正確に再現できないことがあります"
        ));
    }

    let mut frame = result.frame;
    let layer_of: HashMap<FaceId, u32> = order
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, u32::try_from(i).unwrap_or(u32::MAX)))
        .collect();
    for f in &mut frame.faces {
        f.layer = layer_of.get(&f.face).copied().unwrap_or(0);
    }

    ReplayResult {
        frame,
        skipped,
        warnings,
    }
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
