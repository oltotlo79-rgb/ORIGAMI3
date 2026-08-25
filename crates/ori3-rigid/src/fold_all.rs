//! 全ての有効な山谷ヒンジへ同じ割合の希望角を与える、一時表示専用の計算。
//!
//! 折り手順を持たないため、返す形へ物理的な重なり順を刻まない。展開図や手順を
//! 変更せず、呼出し側が明示した直前角だけをwarm startとして使う。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use ori3_cp::Face;
use ori3_model::{CreasePattern, Driver, EdgeId, EdgeKind, FaceId};

use crate::motion::solve_motion_without_surface_order;
use crate::{MotionContactOptions, MotionSolveResult};

/// 一斉折りには順次の折り手順がないため、物理的な重なり順を返さないことを示す警告。
pub const FOLD_ALL_LAYER_ORDER_WARNING: &str =
    "この一時表示には折る手順がないため、重なり順は確定していません";

/// 全ての希望角が閉包と両立せず、一部の角度を譲ったことを示す警告。
pub const FOLD_ALL_RELAXATION_WARNING: &str =
    "全部を同じ割合にはできないため、いちばん近い形を返しています";

/// 一斉折りの入力値が計算契約を満たさない場合だけ返すエラー。
///
/// 紙の不収束・平坦条件違反・貫通はここへ入れず、有限な結果の診断として返す。
#[derive(Clone, Debug, PartialEq)]
pub enum FoldAllPreviewError {
    /// 割合が有限な0%以上100以下ではない。
    InvalidPercent(f64),
    /// warm startの角度が有限な-180度以上180度以下ではない。
    InvalidWarmAngle { hinge: EdgeId, angle_deg: f64 },
}

impl fmt::Display for FoldAllPreviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPercent(percent) => write!(
                formatter,
                "全部の折り目を動かす割合は有限な0%以上100以下で指定してください（受取値: {percent}）"
            ),
            Self::InvalidWarmAngle { hinge, angle_deg } => write!(
                formatter,
                "一時表示の出発角は有限な-180度以上180度以下で指定してください（辺ID {hinge}: {angle_deg}度）"
            ),
        }
    }
}

impl std::error::Error for FoldAllPreviewError {}

/// 一斉折り1回の計算結果。作品・手順・Undoへは書き込まない。
#[derive(Clone, Debug)]
pub struct FoldAllPreviewResult {
    /// 呼出し側が指定した0〜100の割合。
    pub requested_percent: f64,
    /// 有効ヒンジへ実際に渡した希望角（辺ID昇順）。
    pub requested_angles: Vec<Driver>,
    /// 有限な最良姿勢と、不収束・角度譲歩・接触の診断。
    pub motion: MotionSolveResult,
}

/// ちょうど2つの異なる面をつなぐ山谷ヒンジだけを、辺ID昇順で返す。
fn active_hinges(cp: &CreasePattern, faces: &[Face]) -> Vec<(EdgeId, EdgeKind)> {
    let mut face_ids_by_edge: BTreeMap<EdgeId, BTreeSet<FaceId>> = BTreeMap::new();
    for face in faces {
        for &edge in &face.edges {
            face_ids_by_edge.entry(edge).or_default().insert(face.id);
        }
    }
    let kinds: HashMap<EdgeId, EdgeKind> =
        cp.edges.iter().map(|edge| (edge.id, edge.kind)).collect();
    face_ids_by_edge
        .into_iter()
        .filter(|(_, face_ids)| face_ids.len() == 2)
        .filter_map(|(edge, _)| match kinds.get(&edge).copied() {
            Some(kind @ (EdgeKind::Mountain | EdgeKind::Valley)) => Some((edge, kind)),
            Some(EdgeKind::Border | EdgeKind::Aux) | None => None,
        })
        .collect()
}

/// 全有効ヒンジへ、山`+180°`・谷`-180°`を100%とする希望角を作る。
///
/// 戻り値は辺ID昇順。同時には成立しない角度もあり得るため、これは厳密固定ではなく
/// [`solve_fold_all_preview`] が`preferred`として使う希望値である。
pub fn fold_all_targets(
    cp: &CreasePattern,
    faces: &[Face],
    percent: f64,
) -> Result<Vec<Driver>, FoldAllPreviewError> {
    if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
        return Err(FoldAllPreviewError::InvalidPercent(percent));
    }
    let fraction = percent / 100.0;
    Ok(active_hinges(cp, faces)
        .into_iter()
        .map(|(hinge, kind)| Driver {
            hinge,
            target_angle_deg: match kind {
                EdgeKind::Mountain => 180.0 * fraction,
                EdgeKind::Valley => -180.0 * fraction,
                EdgeKind::Border | EdgeKind::Aux => unreachable!("山谷だけに絞り込み済み"),
            },
        })
        .collect())
}

fn validate_warm_start(
    warm_start: Option<&HashMap<EdgeId, f64>>,
) -> Result<(), FoldAllPreviewError> {
    let Some(warm_start) = warm_start else {
        return Ok(());
    };
    let mut angles: Vec<_> = warm_start.iter().collect();
    angles.sort_unstable_by_key(|(hinge, _)| **hinge);
    for (&hinge, &angle_deg) in angles {
        if !angle_deg.is_finite() || !(-180.0..=180.0).contains(&angle_deg) {
            return Err(FoldAllPreviewError::InvalidWarmAngle { hinge, angle_deg });
        }
    }
    Ok(())
}

fn append_warning_once(warnings: &mut Vec<String>, warning: &str) {
    if !warnings.iter().any(|present| present == warning) {
        warnings.push(warning.to_owned());
    }
}

fn finalize_fold_all_preview(
    requested_percent: f64,
    requested_angles: Vec<Driver>,
    mut motion: MotionSolveResult,
) -> FoldAllPreviewResult {
    // 手順のない同時姿勢から一意な物理的上下は決められない。全順位を同値にし、
    // 構造化されたIPC fieldとこの警告の両方で画面側へ明示する。
    for face in &mut motion.result.frame.faces {
        face.layer = 0;
        face.surface_rank = 0;
    }
    motion.surface_order = None;
    motion.surface_order_authoritative = false;
    append_warning_once(
        &mut motion.result.frame.warnings,
        FOLD_ALL_LAYER_ORDER_WARNING,
    );
    if !motion.result.relaxations.is_empty() {
        append_warning_once(
            &mut motion.result.frame.warnings,
            FOLD_ALL_RELAXATION_WARNING,
        );
    }

    FoldAllPreviewResult {
        requested_percent,
        requested_angles,
        motion,
    }
}

/// 全ての有効な折り目を同じ割合で動かした、一時的な姿勢を求める。
///
/// 全角度は`preferred`として与え、`hard`は0本にする。閉包と両立しない場合は
/// 既存の継続計算が希望角を譲って最良の有限形を返す。不収束・貫通でも停止しない。
/// `warm_start`は呼出し側が直前応答の実角を明示した場合だけ使い、永続化しない。
///
/// 折り手順がないので、幾何から推測された表示順位は返却前に全て消す。紙の表裏を
/// 表す`mirrored`と頂点座標はそのまま返す。
pub fn solve_fold_all_preview(
    cp: &CreasePattern,
    faces: &[Face],
    percent: f64,
    warm_start: Option<&HashMap<EdgeId, f64>>,
) -> Result<FoldAllPreviewResult, FoldAllPreviewError> {
    validate_warm_start(warm_start)?;
    let requested_angles = fold_all_targets(cp, faces, percent)?;
    let targets: HashMap<EdgeId, f64> = requested_angles
        .iter()
        .map(|driver| (driver.hinge, driver.target_angle_deg))
        .collect();
    let motion = solve_motion_without_surface_order(
        cp,
        faces,
        &[],
        Some(&targets),
        warm_start,
        MotionContactOptions {
            detect: true,
            prevent: false,
        },
    );

    Ok(finalize_fold_all_preview(percent, requested_angles, motion))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        FOLD_ALL_LAYER_ORDER_WARNING, FoldAllPreviewError, finalize_fold_all_preview,
        fold_all_targets, solve_fold_all_preview,
    };
    use ori3_cp::{extract_faces, insert_segment};
    use ori3_model::{CreasePattern, Document, Edge, EdgeKind, Paper, Vertex};

    use crate::{PENETRATION_WARNING, self_intersection_pairs};

    fn vertex(id: u32, x: f64, y: f64) -> Vertex {
        Vertex { id, pos: [x, y] }
    }

    fn edge(id: u32, v0: u32, v1: u32, kind: EdgeKind) -> Edge {
        Edge { id, v0, v1, kind }
    }

    fn three_strips(left: EdgeKind, right: EdgeKind) -> CreasePattern {
        CreasePattern {
            vertices: vec![
                vertex(0, 0.0, 0.0),
                vertex(1, 1.0 / 3.0, 0.0),
                vertex(2, 2.0 / 3.0, 0.0),
                vertex(3, 1.0, 0.0),
                vertex(4, 1.0, 1.0),
                vertex(5, 2.0 / 3.0, 1.0),
                vertex(6, 1.0 / 3.0, 1.0),
                vertex(7, 0.0, 1.0),
            ],
            edges: vec![
                edge(0, 0, 1, EdgeKind::Border),
                edge(1, 1, 2, EdgeKind::Border),
                edge(2, 2, 3, EdgeKind::Border),
                edge(3, 3, 4, EdgeKind::Border),
                edge(4, 4, 5, EdgeKind::Border),
                edge(5, 5, 6, EdgeKind::Border),
                edge(6, 6, 7, EdgeKind::Border),
                edge(7, 7, 0, EdgeKind::Border),
                edge(8, 1, 6, left),
                edge(9, 2, 5, right),
            ],
            next_vertex_id: 8,
            next_edge_id: 10,
        }
    }

    fn divided_square() -> ori3_model::CreasePattern {
        let mut cp = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        })
        .cp;
        insert_segment(
            &mut cp,
            [1.0 / 3.0, 0.0],
            [1.0 / 3.0, 1.0],
            EdgeKind::Mountain,
        );
        insert_segment(
            &mut cp,
            [2.0 / 3.0, 0.0],
            [2.0 / 3.0, 1.0],
            EdgeKind::Valley,
        );
        insert_segment(&mut cp, [0.0, 0.5], [0.25, 0.5], EdgeKind::Aux);
        cp
    }

    #[test]
    fn targets_use_mountain_positive_valley_negative_and_skip_non_hinges() {
        let cp = divided_square();
        let faces = extract_faces(&cp);
        let kinds = cp
            .edges
            .iter()
            .map(|edge| (edge.id, edge.kind))
            .collect::<HashMap<_, _>>();
        for percent in [0.0, 25.0, 50.0, 75.0, 100.0] {
            let targets = fold_all_targets(&cp, &faces, percent).expect("割合は有効");
            assert_eq!(targets.len(), 2);
            assert!(targets.windows(2).all(|pair| pair[0].hinge < pair[1].hinge));
            for target in targets {
                let expected = match kinds[&target.hinge] {
                    EdgeKind::Mountain => 180.0 * percent / 100.0,
                    EdgeKind::Valley => -180.0 * percent / 100.0,
                    EdgeKind::Border | EdgeKind::Aux => panic!("輪郭・補助線を返した"),
                };
                assert_eq!(target.target_angle_deg, expected);
            }
        }
    }

    #[test]
    fn invalid_inputs_are_errors_but_a_calculated_pose_has_no_layer_order() {
        let cp = divided_square();
        let faces = extract_faces(&cp);
        assert!(matches!(
            fold_all_targets(&cp, &faces, f64::NAN),
            Err(FoldAllPreviewError::InvalidPercent(value)) if value.is_nan()
        ));
        assert_eq!(
            fold_all_targets(&cp, &faces, 100.01),
            Err(FoldAllPreviewError::InvalidPercent(100.01))
        );

        let preview = solve_fold_all_preview(&cp, &faces, 25.0, None).expect("姿勢を返す");
        assert!(preview.motion.result.closure_rms.is_finite());
        assert!(preview.motion.result.frame.faces.iter().all(|face| {
            face.layer == 0
                && face.surface_rank == 0
                && face.polygon.iter().flatten().all(|value| value.is_finite())
        }));
        assert!(preview.motion.surface_order.is_none());
        assert!(!preview.motion.surface_order_authoritative);
        assert!(
            preview
                .motion
                .result
                .frame
                .warnings
                .iter()
                .any(|warning| warning == FOLD_ALL_LAYER_ORDER_WARNING)
        );
    }

    #[test]
    fn a_finite_nonconverged_result_stays_successful_and_keeps_its_warning() {
        let cp = divided_square();
        let faces = extract_faces(&cp);
        let solved = solve_fold_all_preview(&cp, &faces, 75.0, None).expect("有限姿勢を返す");
        let mut motion = solved.motion;
        motion.result.converged = false;
        motion.result.best_effort = true;
        motion
            .result
            .frame
            .warnings
            .push("追従計算が収束していません（有限な最良形）".to_string());

        let preview = finalize_fold_all_preview(75.0, solved.requested_angles, motion);
        assert!(!preview.motion.result.converged);
        assert!(preview.motion.result.best_effort);
        assert!(
            preview
                .motion
                .result
                .frame
                .warnings
                .iter()
                .any(|warning| warning.contains("収束していません"))
        );
        assert!(preview.motion.result.frame.faces.iter().all(|face| {
            face.polygon
                .iter()
                .flatten()
                .all(|coordinate| coordinate.is_finite())
        }));
    }

    #[test]
    fn a_penetrating_fold_all_pose_stays_successful_and_keeps_its_warning() {
        let mut found = None;
        for (left, right) in [
            (EdgeKind::Mountain, EdgeKind::Mountain),
            (EdgeKind::Mountain, EdgeKind::Valley),
            (EdgeKind::Valley, EdgeKind::Mountain),
            (EdgeKind::Valley, EdgeKind::Valley),
        ] {
            let cp = three_strips(left, right);
            let faces = extract_faces(&cp);
            for percent in (5..=95).step_by(5) {
                let preview = solve_fold_all_preview(&cp, &faces, f64::from(percent), None)
                    .expect("貫通しても有限姿勢を返す");
                let intersections = self_intersection_pairs(&preview.motion.result.frame);
                if intersections.is_empty() {
                    continue;
                }
                assert!(preview.motion.contact_detected);
                assert!(!preview.motion.contact_stopped);
                assert!(
                    preview
                        .motion
                        .result
                        .frame
                        .warnings
                        .iter()
                        .any(|warning| warning == PENETRATION_WARNING)
                );
                assert!(preview.motion.result.frame.faces.iter().all(|face| {
                    face.polygon
                        .iter()
                        .flatten()
                        .all(|coordinate| coordinate.is_finite())
                }));
                found = Some((left, right, percent, intersections));
                break;
            }
            if found.is_some() {
                break;
            }
        }
        assert!(
            found.is_some(),
            "3短冊の山谷4通り×5〜95%に貫通姿勢がなく、検査標本になっていない"
        );
    }
}
