use ori3_layers::{ReplayResult, replay};
use ori3_model::{Document, TechniqueKind};

/// 曲がる中割りの任意欄だけを埋めても、全手順端点の再生値が1 bitも変わらないことを調べる。
pub fn assert_marker_preserves_all_step_endpoint_bits(
    document: &Document,
    label: &str,
) -> (usize, usize) {
    let mut marked = document.clone();
    let mut curved_steps = 0usize;
    for step in &mut marked.sequence {
        let is_curved_inside_reverse = step.kind == TechniqueKind::InsideReverse;
        curved_steps += usize::from(is_curved_inside_reverse);
        step.curved_inside_reverse = Some(is_curved_inside_reverse);
    }

    // 先に片方の全endpointを計算し切り、直前結果のcacheが比較対象を隠さない順にする。
    let baseline: Vec<_> = (0..=document.sequence.len())
        .map(|up_to| replay(document, up_to, 1.0))
        .collect();
    let with_marker: Vec<_> = (0..=marked.sequence.len())
        .map(|up_to| replay(&marked, up_to, 1.0))
        .collect();

    assert_eq!(baseline.len(), with_marker.len(), "{label}: 比較endpoint数");
    for (up_to, (left, right)) in baseline.iter().zip(&with_marker).enumerate() {
        assert_replay_result_bits_eq(left, right, label, up_to);
    }
    (baseline.len(), curved_steps)
}

fn assert_replay_result_bits_eq(
    left: &ReplayResult,
    right: &ReplayResult,
    label: &str,
    up_to: usize,
) {
    assert_eq!(left.skipped, right.skipped, "{label}/手{up_to}: skipped");
    assert_eq!(left.warnings, right.warnings, "{label}/手{up_to}: warnings");
    assert_eq!(
        left.suspect_hinges, right.suspect_hinges,
        "{label}/手{up_to}: suspect_hinges"
    );
    assert_eq!(
        left.driver_hinges, right.driver_hinges,
        "{label}/手{up_to}: driver_hinges"
    );
    assert_eq!(
        left.frame.warnings, right.frame.warnings,
        "{label}/手{up_to}: frame warnings"
    );
    assert_eq!(
        left.frame.faces.len(),
        right.frame.faces.len(),
        "{label}/手{up_to}: face数"
    );
    for (face_index, (left_face, right_face)) in
        left.frame.faces.iter().zip(&right.frame.faces).enumerate()
    {
        assert_eq!(
            left_face.face, right_face.face,
            "{label}/手{up_to}/面{face_index}: face ID"
        );
        assert_eq!(
            left_face.layer, right_face.layer,
            "{label}/手{up_to}/面{face_index}: layer"
        );
        assert_eq!(
            left_face.surface_rank, right_face.surface_rank,
            "{label}/手{up_to}/面{face_index}: surface rank"
        );
        assert_eq!(
            left_face.mirrored, right_face.mirrored,
            "{label}/手{up_to}/面{face_index}: mirrored"
        );
        assert_eq!(
            left_face.polygon.len(),
            right_face.polygon.len(),
            "{label}/手{up_to}/面{face_index}: 頂点数"
        );
        for (point_index, (left_point, right_point)) in left_face
            .polygon
            .iter()
            .zip(&right_face.polygon)
            .enumerate()
        {
            for axis in 0..3 {
                assert_eq!(
                    left_point[axis].to_bits(),
                    right_point[axis].to_bits(),
                    "{label}/手{up_to}/面{face_index}/点{point_index}/軸{axis}: 座標bit"
                );
            }
        }
    }

    assert_eq!(
        left.hinge_angles.len(),
        right.hinge_angles.len(),
        "{label}/手{up_to}: hinge angle数"
    );
    for (hinge, left_angle) in &left.hinge_angles {
        let right_angle = right
            .hinge_angles
            .get(hinge)
            .unwrap_or_else(|| panic!("{label}/手{up_to}: hinge {hinge}が右辺にない"));
        assert_eq!(
            left_angle.to_bits(),
            right_angle.to_bits(),
            "{label}/手{up_to}/hinge {hinge}: angle bit"
        );
    }
    assert_eq!(
        left.surface_order_provenance, right.surface_order_provenance,
        "{label}/手{up_to}: surface order provenance"
    );
    assert_eq!(
        left.sequence_targets.len(),
        right.sequence_targets.len(),
        "{label}/手{up_to}: sequence target数"
    );
    for (index, (left_driver, right_driver)) in left
        .sequence_targets
        .iter()
        .zip(&right.sequence_targets)
        .enumerate()
    {
        assert_eq!(
            left_driver.hinge, right_driver.hinge,
            "{label}/手{up_to}/target {index}: hinge"
        );
        assert_eq!(
            left_driver.target_angle_deg.to_bits(),
            right_driver.target_angle_deg.to_bits(),
            "{label}/手{up_to}/target {index}: angle bit"
        );
    }
    assert_eq!(
        left.relaxations.len(),
        right.relaxations.len(),
        "{label}/手{up_to}: relaxation数"
    );
    for (index, (left_relaxation, right_relaxation)) in
        left.relaxations.iter().zip(&right.relaxations).enumerate()
    {
        assert_eq!(
            left_relaxation.hinge, right_relaxation.hinge,
            "{label}/手{up_to}/relaxation {index}: hinge"
        );
        for (name, left_value, right_value) in [
            (
                "target",
                left_relaxation.target_angle_deg,
                right_relaxation.target_angle_deg,
            ),
            (
                "actual",
                left_relaxation.actual_angle_deg,
                right_relaxation.actual_angle_deg,
            ),
            (
                "delta",
                left_relaxation.delta_deg,
                right_relaxation.delta_deg,
            ),
        ] {
            assert_eq!(
                left_value.to_bits(),
                right_value.to_bits(),
                "{label}/手{up_to}/relaxation {index}/{name}: bit"
            );
        }
    }
    assert_eq!(
        left.closure_rms.to_bits(),
        right.closure_rms.to_bits(),
        "{label}/手{up_to}: closure_rms bit"
    );
    assert_eq!(
        left.best_effort, right.best_effort,
        "{label}/手{up_to}: best_effort"
    );
    assert_eq!(
        left.converged, right.converged,
        "{label}/手{up_to}: converged"
    );
    assert_eq!(
        left.layer_transition.start, right.layer_transition.start,
        "{label}/手{up_to}: layer transition start"
    );
    assert_eq!(
        left.layer_transition.end, right.layer_transition.end,
        "{label}/手{up_to}: layer transition end"
    );
    assert_eq!(
        left.layer_transition.progress.to_bits(),
        right.layer_transition.progress.to_bits(),
        "{label}/手{up_to}: layer transition progress bit"
    );
    assert_eq!(
        left.layer_transition.order_is_authoritative, right.layer_transition.order_is_authoritative,
        "{label}/手{up_to}: layer transition authority"
    );
}
