//! 手順の再生: 展開図(CP)と手順列だけから立体の姿勢を求め直す。
//!
//! 3D状態は一切保存しない(SEQ-004「展開図を編集したら手順を自動で再生し直す」の
//! ための設計)。各ステップの [`DriverLine`](ori3_model::DriverLine) は
//! 「CP座標の線分+角度」なので、後続の折りや編集で辺IDが変わっても
//! [`resolve_driver_edges`] で現在の辺へ解決できる。
//!
//! # 求め方(折り上がり `t=1`)
//!
//! 折り畳んだ状態の上に次の折りを重ねるのではなく、**平らな展開図に
//! 「そこまでの全ステップのdriver」をまとめて与えて1回で解く**。
//! 同じ辺を複数のステップが駆動する場合は後のステップが勝つ。
//!
//! **まだ折っていない折り線は0°(平ら)のdriverとして明示的に固定する。**
//! ソルバーは角度指定の無いヒンジを自由変数として扱い、初期値バイアス(山谷の向きへ
//! driver角の平均の半分)から別の枝へ収束させてしまうため、これを省くと
//! 「後続ステップの折り線まで曲がった、警告の出ない誤った形」が返る
//! (`ori3-rigid` の `solve` のdocが書いているとおり、平らに戻すには0°の明示が要る)。
//! 結果として全ヒンジが固定値になるので、ステップごとに解き直しても最後の1回と
//! 同じ姿勢になる。無駄を避けて1回だけ解く(warm startを使わないので決定的)。
//!
//! # 折っている最中(`0 < t < 1`)
//!
//! **角度を線形補間した値をそのまま全ヒンジに固定してはいけない。** 内部頂点の
//! まわりにはループ閉包の拘束があり(自由度1の四折り頂点で2本以上を勝手な値に
//! すると閉じない)、破ると**面どうしが離れて紙がちぎれて見える**。
//!
//! そこで補間値は「目標」として扱い、[`ori3_rigid::solve_near`] で
//! **閉包を満たす形のうち目標にいちばん近いもの**を求める。さらに一発で `t` まで
//! 飛ばすと対称な解のあいだで解が飛び移って紙が瞬間移動するため、目標を
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

use std::collections::{BTreeMap, HashMap};

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_geometry::Isometry2;
use ori3_model::{Document, Driver, EdgeId, FaceId, Frame3D, StepId};

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
    /// 接触補正専用の開始・完了層順序。IPCへは出さず、コマンド層でだけ使う。
    #[serde(skip)]
    pub layer_transition: LayerTransition,
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
    let plan = plan_steps(doc, faces, up_to, t);
    let layer_transition = LayerTransition {
        start: plan.order_start.clone(),
        end: plan.order_end.clone(),
        progress: t,
    };
    let mut warnings = plan.warnings;

    let result = match &plan.path {
        Some(path) => solve_along(doc, faces, path, t),
        None => ori3_rigid::solve(&doc.cp, faces, &plan.drivers, None),
    };
    if !result.converged {
        warnings.push(format!(
            "手順{up_to}までの形が展開図から求まりませんでした(いちばん近い形で表示します)。一部の層だけを折る手順は、展開図からの折り直しでは正確に再現できないことがあります"
        ));
    }

    let hinge_angles = result.angles;
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

    ReplayResult {
        frame,
        skipped: plan.skipped,
        warnings,
        suspect_hinges: Vec::new(),
        driver_hinges: plan.driver_hinges,
        hinge_angles,
        layer_transition,
    }
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
        .drivers
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

/// 折り道 `path`(ヒンジごとの 直前の角度→目標角)を `0..=t` までたどり、
/// 途中で紙がつながったままになる姿勢を求める。
///
/// 各分割点で「閉包を満たす形のうち、補間した角度にいちばん近いもの」を解き、
/// その解を次の分割点の初期値にする。分割点は `t` の等分なので `t` だけで
/// 決まり、同じ入力なら常に同じ結果になる(SYS-004)。
fn solve_along(
    doc: &Document,
    faces: &[Face],
    path: &[(EdgeId, f64, f64)],
    t: f64,
) -> ori3_rigid::SolveResult {
    let mut warm: Option<HashMap<EdgeId, f64>> = None;
    let mut result: Option<ori3_rigid::SolveResult> = None;
    let mut iterations = 0u32;
    for i in 1..=SUBSTEPS {
        let s = t * f64::from(i) / f64::from(SUBSTEPS);
        let targets: HashMap<EdgeId, f64> = path
            .iter()
            .map(|&(e, from, to)| (e, from + (to - from) * s))
            .collect();
        let mut candidate = ori3_rigid::solve_near(&doc.cp, faces, &[], &targets, warm.as_ref());
        iterations = iterations.saturating_add(candidate.iterations);
        if !candidate.converged {
            // 箱制約の境界では、次の補間点に閉じた解が存在しないことがある。
            // 不収束候補を次のwarm startへ昇格させず、直前の収束解を表示する。
            // 最初の補間点で失敗した場合だけ、s=0の開始姿勢を明示的に解く。
            let previous = if let Some(previous) = result {
                previous
            } else {
                let initial_targets: HashMap<EdgeId, f64> =
                    path.iter().map(|&(edge, from, _)| (edge, from)).collect();
                let initial = ori3_rigid::solve_near(&doc.cp, faces, &[], &initial_targets, None);
                iterations = iterations.saturating_add(initial.iterations);
                if !initial.converged {
                    candidate.iterations = iterations;
                    return candidate;
                }
                initial
            };
            return previous_replay_result(previous, candidate, iterations);
        }
        candidate.iterations = iterations;
        warm = Some(candidate.angles.clone());
        result = Some(candidate);
    }
    result.expect("SUBSTEPSは1以上")
}

/// 閉包を満たさない中間姿勢を見せず、直前の収束姿勢へ不収束警告だけを載せる。
fn previous_replay_result(
    mut previous: ori3_rigid::SolveResult,
    failed: ori3_rigid::SolveResult,
    iterations: u32,
) -> ori3_rigid::SolveResult {
    previous.converged = false;
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

/// `up_to` ステップまでの角度指定・層順序・警告(replayとflat_state_atの共通処理)。
struct StepPlan {
    /// ソルバーへ渡す角度指定。t=1のときは全ヒンジ(未駆動は0°)、
    /// 折り途中(t<1)は「そのステップが駆動するヒンジ」だけ
    drivers: Vec<Driver>,
    /// 手順内のDriverLineから解決できたヒンジ。未指定ヒンジの0度固定は含めない。
    driver_hinges: Vec<EdgeId>,
    /// 折り途中(t<1)の折り道: ヒンジごとの (辺ID, 直前の角度, 目標角) (度)。
    /// この間を少しずつ動かしながら「閉じた形」を追いかける。t=1ではNone。
    path: Option<Vec<(EdgeId, f64, f64)>>,
    /// 層順序(下→上)
    order: Vec<FaceId>,
    /// `up_to` 手順へ入る直前の層順序(接触補正用)。
    order_start: Vec<FaceId>,
    /// `up_to` 手順を完了したときの層順序(接触補正用)。
    order_end: Vec<FaceId>,
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
    let mut order_start = order.clone();
    let mut order_end = order.clone();
    // ヒンジごとの目標角(後から積んだステップが優先される)。BTreeMapなので
    // ソルバーへ渡す順序も辺ID昇順で決定的(SYS-004)。
    let mut angles: BTreeMap<EdgeId, f64> = BTreeMap::new();
    // `up_to` ステップ目に入る直前の角度(折り途中の補間の始点)
    let mut before: BTreeMap<EdgeId, f64> = BTreeMap::new();

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
        if last {
            if let Some(next) = resolved_order {
                order_end = next;
            }
            if t >= 1.0 {
                order.clone_from(&order_end);
            }
        } else if let Some(next) = resolved_order {
            order = next;
        }
    }

    let hinges = hinge_edges(faces);
    // t=0 は「直前の手順を折り終えた状態」そのもの。連続法を1歩も進めずに
    // 厳密解の道(下の全ヒンジ固定)へ回し、`replay(k, 0)` が `replay(k-1, 1)` と
    // ビット一致するようにする。
    if t <= 0.0 {
        angles = before.clone();
    }

    // 折り途中(t<1): 補間した角度は「目標」であって解ではない。全部を固定すると
    // 内部頂点まわりのループ閉包が破れ、面どうしが離れて紙がちぎれて見える
    // (自由度1の四折り頂点で2本以上を勝手な値にすると閉じない)。
    // 角度指定は置かず、閉包を満たす形のうち補間値にいちばん近いものを解かせる。
    if t > 0.0 && t < 1.0 {
        let path: Vec<(EdgeId, f64, f64)> = hinges
            .iter()
            .map(|&e| {
                let from = before.get(&e).copied().unwrap_or(0.0);
                (e, from, angles.get(&e).copied().unwrap_or(0.0))
            })
            .collect();
        return StepPlan {
            drivers: Vec::new(),
            driver_hinges: angles.keys().copied().collect(),
            path: Some(path),
            order,
            order_start,
            order_end,
            skipped,
            warnings,
        };
    }

    // 折り上がり(t=1)は全ヒンジを固定して厳密な形を出す。まだ折っていない
    // 折り線も0°(平ら)で明示する。自由変数として残すと初期値バイアスから
    // 別の枝へ収束し、警告なしで誤った形が返る(モジュールdoc参照)。
    let all: Vec<Driver> = hinges
        .into_iter()
        .map(|hinge| Driver {
            hinge,
            target_angle_deg: angles.get(&hinge).copied().unwrap_or(0.0),
        })
        .collect();

    StepPlan {
        drivers: all,
        driver_hinges: angles.keys().copied().collect(),
        path: None,
        order,
        order_start,
        order_end,
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
