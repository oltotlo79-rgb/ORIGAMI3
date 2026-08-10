use ori3_geometry::{
    MovingSide, angle_bisectors, distance_to_line, existing_line, fold_point_onto_line,
    fold_point_onto_line_perpendicular, fold_two_points_onto_two_lines, line_through_points,
    moving_side_of, perpendicular_bisector, perpendicular_through_point, reflect_point_across_fold,
    solve_align,
};
use ori3_model::{AlignmentMode, AlignmentTarget};

type FoldLine = [[f64; 2]; 2];

fn on_line(line: FoldLine, c0: f64, c1: f64, c2: f64) {
    for p in line {
        assert!((c0 * p[0] + c1 * p[1] - c2).abs() < 1e-9, "line={line:?}");
    }
}

fn point(p: [f64; 2]) -> AlignmentTarget {
    AlignmentTarget::Point { p }
}

fn line(a: [f64; 2], b: [f64; 2]) -> AlignmentTarget {
    AlignmentTarget::Line { a, b }
}

#[test]
fn axioms_one_to_four_match_the_desktop_constructions() {
    assert_eq!(
        line_through_points([0.0, 0.0], [2.0, 1.0]),
        Some([[0.0, 0.0], [2.0, 1.0]])
    );
    assert!(line_through_points([0.25, 0.75], [0.25, 0.75]).is_none());

    let bisector = perpendicular_bisector([0.0, 0.0], [1.0, 1.0]).unwrap();
    on_line(bisector, 1.0, 1.0, 1.0);
    assert!(perpendicular_bisector([0.5, 0.5], [0.5, 0.5]).is_none());

    let perpendicular = perpendicular_through_point([0.25, 0.5], [[0.0, 0.0], [1.0, 0.0]]).unwrap();
    on_line(perpendicular, 1.0, 0.0, 0.25);

    let bisectors = angle_bisectors([[0.0, 0.0], [1.0, 0.0]], [[0.0, 0.0], [0.0, 1.0]]);
    assert_eq!(bisectors.len(), 2);
    on_line(bisectors[0], 1.0, -1.0, 0.0);
    on_line(bisectors[1], 1.0, 1.0, 0.0);

    let parallel = angle_bisectors([[0.0, 0.0], [1.0, 0.0]], [[0.0, 2.0], [1.0, 2.0]]);
    assert_eq!(parallel.len(), 1);
    on_line(parallel[0], 0.0, 1.0, 1.0);
}

#[test]
fn axiom_five_returns_two_one_or_zero_solutions() {
    let x_axis = [[0.0, 0.0], [1.0, 0.0]];
    let two = fold_point_onto_line([0.0, 1.0], x_axis, [0.0, 0.0]);
    assert_eq!(two.len(), 2);
    for fold in two {
        assert!(distance_to_line(fold, [0.0, 0.0]) < 1e-9);
    }

    let one = fold_point_onto_line([0.0, 1.0], x_axis, [0.0, 0.5]);
    assert_eq!(one.len(), 1);
    on_line(one[0], 0.0, 1.0, 0.5);
    assert!(fold_point_onto_line([0.0, 1.0], x_axis, [0.0, 3.0]).is_empty());

    let stationary_removed = fold_point_onto_line([1.0, 0.0], x_axis, [0.0, 0.0]);
    assert_eq!(stationary_removed.len(), 1);
    on_line(stationary_removed[0], 1.0, 0.0, 0.0);
}

#[test]
fn axiom_six_finds_and_verifies_every_real_candidate() {
    let p1 = [0.0, 0.0];
    let line1 = [[1.0, -1.0], [1.0, 2.0]];
    let p2 = [-1.0, 1.0];
    let line2 = [[1.0, 0.0], [2.0, 1.0]];
    let folds = fold_two_points_onto_two_lines(p1, line1, p2, line2);
    assert!(!folds.is_empty() && folds.len() <= 3);
    assert!(
        folds
            .iter()
            .any(|fold| distance_to_line(*fold, [0.5, 0.0]) <= 1e-9)
    );
    for fold in folds {
        let q1 = reflect_point_across_fold(p1, fold).unwrap();
        let q2 = reflect_point_across_fold(p2, fold).unwrap();
        assert!(distance_to_line(line1, q1) < 1e-8);
        assert!(distance_to_line(line2, q2) < 1e-8);
    }

    let horizontal = fold_two_points_onto_two_lines(
        [0.0, 1.0],
        [[-1.0, -1.0], [1.0, -1.0]],
        [2.0, 2.0],
        [[2.0, -3.0], [2.0, 3.0]],
    );
    assert!(
        horizontal
            .iter()
            .any(|fold| distance_to_line(*fold, [0.0, 0.0]) <= 1e-9)
    );
}

#[test]
fn axiom_six_keeps_three_nearby_real_roots() {
    let p1 = [0.16620390651305184, 0.05606810980014021];
    let line1 = [
        [0.7233129972797999, 0.08141874087277112],
        [0.7403485898762333, 0.08278827472852669],
    ];
    let p2 = [0.15681505806280727, 0.9080062872025711];
    let line2 = [
        [0.6552902502134852, 0.9306888477719542],
        [0.6689570877774842, 0.9317875760888786],
    ];
    let known = [
        [0.49241791908431437, -0.9786278355761696],
        [0.4015042125210822, 1.019304770384035],
    ];

    let folds = fold_two_points_onto_two_lines(p1, line1, p2, line2);
    assert_eq!(folds.len(), 3);
    assert!(folds.iter().any(|fold| {
        distance_to_line(*fold, known[0]) < 1e-9 && distance_to_line(*fold, known[1]) < 1e-9
    }));
    for fold in folds {
        let q1 = reflect_point_across_fold(p1, fold).unwrap();
        let q2 = reflect_point_across_fold(p2, fold).unwrap();
        assert!(distance_to_line(line1, q1) < 1e-8);
        assert!(distance_to_line(line2, q2) < 1e-8);
    }
}

#[test]
fn axiom_seven_and_existing_line_match_the_desktop_constructions() {
    let fold = fold_point_onto_line_perpendicular(
        [0.0, 2.0],
        [[-1.0, 0.0], [1.0, 0.0]],
        [[0.0, -1.0], [0.0, 1.0]],
    )
    .unwrap();
    on_line(fold, 0.0, 1.0, 1.0);
    assert!(
        fold_point_onto_line_perpendicular(
            [0.0, 1.0],
            [[0.0, 0.0], [1.0, 0.0]],
            [[0.0, 2.0], [1.0, 2.0]],
        )
        .is_none()
    );

    assert_eq!(
        existing_line([[0.0, 0.4], [1.0, 0.4]]),
        Some([[0.0, 0.4], [1.0, 0.4]])
    );
    assert!(existing_line([[0.0, 0.4], [0.0, 0.4]]).is_none());
}

#[test]
fn common_entry_point_supports_all_eight_modes() {
    let cases = [
        (
            AlignmentMode::ThroughTwoPoints,
            vec![point([0.1, 0.2]), point([0.8, 0.6])],
        ),
        (
            AlignmentMode::PointPoint,
            vec![point([0.0, 0.0]), point([1.0, 1.0])],
        ),
        (
            AlignmentMode::LineLine,
            vec![line([0.0, 0.0], [1.0, 0.0]), line([0.0, 0.0], [0.0, 1.0])],
        ),
        (
            AlignmentMode::PointPerpendicularLine,
            vec![point([0.25, 0.75]), line([0.0, 0.0], [1.0, 0.0])],
        ),
        (
            AlignmentMode::PointLineThrough,
            vec![
                point([0.0, 1.0]),
                line([0.0, 0.0], [1.0, 0.0]),
                point([0.0, 0.0]),
            ],
        ),
        (
            AlignmentMode::PointToLinePointToLine,
            vec![
                point([0.0, 0.0]),
                line([1.0, -1.0], [1.0, 2.0]),
                point([-1.0, 1.0]),
                line([1.0, 0.0], [2.0, 1.0]),
            ],
        ),
        (
            AlignmentMode::PointLinePerpendicular,
            vec![
                point([0.0, 2.0]),
                line([-1.0, 0.0], [1.0, 0.0]),
                line([0.0, -1.0], [0.0, 1.0]),
            ],
        ),
        (
            AlignmentMode::ExistingLine,
            vec![line([0.0, 0.4], [1.0, 0.4])],
        ),
    ];

    for (mode, picks) in cases {
        let result = solve_align(mode, &picks, Some([1.0, 1.0]));
        assert!(
            !result.lines.is_empty(),
            "mode={mode:?}, reason={:?}",
            result.reason
        );
        assert!(result.reason.is_none());
        for fold in result.lines {
            let length =
                ((fold[1][0] - fold[0][0]).powi(2) + (fold[1][1] - fold[0][1]).powi(2)).sqrt();
            assert!((length - 2.0).abs() < 1e-9);
        }
    }

    assert_eq!(
        moving_side_of([[0.0, 0.0], [1.0, 0.0]], [0.5, 1.0]),
        MovingSide::Left
    );
    assert_eq!(
        moving_side_of([[0.0, 0.0], [1.0, 0.0]], [0.5, 0.0]),
        MovingSide::Right
    );
}
