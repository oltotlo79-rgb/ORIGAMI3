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
//! 「そこまでの全ステップのdriver」をまとめて与えて解く**。
//! `up_to` 未満のステップは目標角そのまま、`up_to` ステップ目だけ角度を `t` 倍する
//! (0→目標への線形補間)。同じ辺を複数のステップが駆動する場合は後のステップが勝つ。
//!
//! ステップを1つずつ進めながら解き、前ステップの解を次のwarm startに渡す
//! (途中の形からの連続変化になるため、山谷の分岐を取り違えにくい)。
//! warm startの元は必ず同じ手順列から決まるので結果は決定的(SYS-004)。
//!
//! # 見つからない折り線の扱い(SEQ-004)
//!
//! - DriverLineが1本も辺に解決できないステップは飛ばし、`skipped` に載せて警告する。
//!   飛ばしたステップの層順序も使わない(直前の層順序を保つ)
//! - 一部だけ解決できた場合は解決できた分で続行し、警告だけ出す
//! - 折り線を持たないステップ(Pose等)は「見つからない」ではないので飛ばさない
//! - 姿勢計算が収束しない場合も止めず、最良解を返して警告に載せる

use std::collections::HashMap;

use ori3_cp::extract_faces;
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
    let faces = extract_faces(&doc.cp);
    let up_to = up_to.min(doc.sequence.len());
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        1.0
    };

    let mut warnings: Vec<String> = Vec::new();
    let mut skipped: Vec<StepId> = Vec::new();
    let mut diverged: Vec<usize> = Vec::new();
    // 現在の層順序(下→上)。初期状態は面ID昇順。
    let mut order = FlatState::initial(&doc.cp, &faces).order;
    // そこまでのステップの角度指定の累積(後から積んだものが優先される)
    let mut drivers: Vec<Driver> = Vec::new();
    let mut warm: Option<HashMap<EdgeId, f64>> = None;
    let mut frame: Option<Frame3D> = None;

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
        drivers.extend(step_drivers);

        let result = ori3_rigid::solve(&doc.cp, &faces, &drivers, warm.as_ref());
        if !result.converged {
            diverged.push(number);
        }
        warm = Some(result.angles);
        frame = Some(result.frame);

        // 層順序はステップ完了時にだけ更新する(Poseなど層順序を持たないステップと
        // 解決できる点が無いステップは直前の層順序を保つ)
        if scale >= 1.0
            && let Some(points) = &step.layer_order
            && !points.is_empty()
        {
            let (resolved, mut w) = FlatState::resolve_order(&doc.cp, &faces, points);
            order = resolved;
            warnings.append(&mut w);
        }
    }

    if !diverged.is_empty() {
        let list = diverged
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join("・");
        warnings.push(format!(
            "手順{list}の折り具合の計算が収束しませんでした(いちばん近い形で表示します)"
        ));
    }

    // 1ステップも適用しなかった場合(up_to=0・全ステップ飛ばし)は平らな姿勢
    let mut frame =
        frame.unwrap_or_else(|| ori3_rigid::solve(&doc.cp, &faces, &drivers, warm.as_ref()).frame);
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
