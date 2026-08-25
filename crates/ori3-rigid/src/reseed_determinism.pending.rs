// Temporarily paste these tests into motion.rs::tests only while measuring red/green.

#[test]
fn reseed_distance_bits_do_not_depend_on_hashmap_order() {
    let document = sa_document();
    let faces = extract_faces(&document.cp);
    let topology = solver::prepare_topology(&document.cp, &faces);
    let empty = HashMap::new();
    let reseed = Reseed {
        cp: &document.cp,
        faces: &faces,
        drivers: &[],
        targets: None,
        start_angles: &empty,
        warm: None,
        topology: &topology,
        prevent_contact: false,
    };
    let entries: Vec<_> = (0..=256)
        .map(|hinge| (hinge, if hinge == 0 { 180.0 } else { 1e-6 }))
        .collect();
    for _ in 0..32 {
        let forward: HashMap<_, _> = entries.iter().copied().collect();
        let reverse: HashMap<_, _> = entries.iter().rev().copied().collect();
        assert_eq!(
            reseed.distance_from_previous(&forward).to_bits(),
            reseed.distance_from_previous(&reverse).to_bits()
        );
    }
}

#[test]
fn sa_follow_pose_bits_do_not_depend_on_hashmap_order() {
    let document = sa_document();
    let faces = extract_faces(&document.cp);
    let document_seed: HashMap<_, _> = document
        .cp
        .edges
        .iter()
        .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .map(|edge| (edge.id, 0.0))
        .collect();
    let contact = super::MotionContactOptions {
        detect: false,
        prevent: false,
    };
    let desired_17 = HashMap::from([(17, -90.0)]);
    let finish_17 = solve_canonical_motion_prepared(
        &document.cp,
        &faces,
        &[],
        Some(&desired_17),
        Some(&document_seed),
        contact,
    )
    .0;
    let desired = HashMap::from([(17, -90.0), (21, 90.0)]);
    let finish_21 = solve_canonical_motion_prepared(
        &document.cp,
        &faces,
        &[],
        Some(&desired),
        Some(&document_seed),
        contact,
    )
    .0;
    assert!(max_vertex_delta(&finish_17.result.frame, &finish_21.result.frame) > 0.0);

    let mut ordered_warm: Vec<_> = finish_21
        .result
        .angles
        .iter()
        .map(|(&hinge, &angle)| (hinge, angle))
        .collect();
    ordered_warm.sort_unstable_by_key(|&(hinge, _)| hinge);
    let requested = [(17, -90.0), (21, 90.0)];
    let driver = [Driver {
        hinge: 19,
        target_angle_deg: 90.0,
    }];
    let mut baseline = None;
    for iteration in 0..16 {
        let warm: HashMap<_, _> = if iteration % 2 == 0 {
            ordered_warm.iter().copied().collect()
        } else {
            ordered_warm.iter().rev().copied().collect()
        };
        let preferred: HashMap<_, _> = if iteration % 2 == 0 {
            requested.into_iter().collect()
        } else {
            requested.into_iter().rev().collect()
        };
        let follow = solve_motion_with_contact_options(
            &document.cp,
            &faces,
            &driver,
            Some(&preferred),
            Some(&warm),
            contact,
        );
        if let Some(baseline) = &baseline {
            assert_pose_bits_eq(baseline, &follow.result);
        } else {
            baseline = Some(follow.result);
        }
    }
}
