#[test]
fn surface_order_same_input_is_deterministic_ten_times() {
    let cp = angle_surface_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("diagonal-midline-user-cp", cp.clone(), 1.0, 1.0);
    let angles = angle_surface_angles(-94.0);
    let mut expected = None::<(Vec<u8>, VisualImage)>;
    for run in 1..=10 {
        let frame = to_frame3d(&cp, &faces, &propagate(&cp, &faces, &angles));
        let serialized = serde_json::to_vec(&frame).expect("serialize deterministic frame");
        let image = visual_image(&diagram, &frame, VIEWPORT, camera(1.0, 1.0, 1.0));
        if let Some(reference) = &expected {
            assert_eq!(&serialized, &reference.0, "frame changed on run {run}");
            assert_eq!(&image, &reference.1, "visible result changed on run {run}");
        } else {
            expected = Some((serialized, image));
        }
    }
    println!("SURFACE_DETERMINISM identical_runs=10 input=edge43:-94");
}

#[test]
fn surface_order_user_pose_minus_180_is_deterministic_ten_times() {
    let cp = angle_surface_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("diagonal-midline-user-pose", cp.clone(), 1.0, 1.0);
    let angles = angle_surface_angles(-180.0);
    let mut expected = None::<(Vec<u8>, VisualImage)>;
    for run in 1..=10 {
        let frame = to_frame3d(&cp, &faces, &propagate(&cp, &faces, &angles));
        let serialized = serde_json::to_vec(&frame).expect("serialize deterministic user pose");
        let image = visual_image(&diagram, &frame, VIEWPORT, camera(1.0, 1.0, 1.0));
        if let Some(reference) = &expected {
            assert_eq!(
                &serialized, &reference.0,
                "user-pose frame changed on run {run}"
            );
            assert_eq!(
                &image, &reference.1,
                "user-pose visible result changed on run {run}"
            );
        } else {
            expected = Some((serialized, image));
        }
    }
    println!("SURFACE_DETERMINISM identical_runs=10 input=edge43:-180");
}
