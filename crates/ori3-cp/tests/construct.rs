//! 作図補助(CPE-005)の検査: 二等分線・垂線・等分点・方向線。

use ori3_cp::construct::{bisector, direction_lines, divide_points, perpendicular};

/// 2点の距離がほぼ等しいことを確かめる補助
fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
}

#[test]
fn bisector_of_right_angle_points_diagonally() {
    // 角ABC: Bを頂点に、+x方向と+y方向へ伸びる直角
    let line = bisector([1.0, 0.0], [0.0, 0.0], [0.0, 1.0]);
    assert!(close(line[0], [0.0, 0.0]), "始点は角の頂点: {line:?}");
    let d = [line[1][0] - line[0][0], line[1][1] - line[0][1]];
    assert!((d[0] - d[1]).abs() < 1e-9, "45°方向へ伸びる: {d:?}");
    assert!(d[0] > 0.0);
    // 長さは腕の長い方に合わせる(画面で見える長さ)
    assert!((d[0].hypot(d[1]) - 1.0).abs() < 1e-9);
}

#[test]
fn bisector_of_straight_angle_is_perpendicular() {
    // 180°(まっすぐ)の角では、二等分線は腕に垂直な線になる
    let line = bisector([-1.0, 0.0], [0.0, 0.0], [1.0, 0.0]);
    let d = [line[1][0] - line[0][0], line[1][1] - line[0][1]];
    assert!(d[0].abs() < 1e-9 && d[1].abs() > 0.5, "d={d:?}");
}

#[test]
fn bisector_with_degenerate_arm_returns_degenerate_line() {
    // 腕の長さがゼロなら角が決まらない。潰れた線を返し、呼び出し側が捨てられるようにする
    let line = bisector([0.0, 0.0], [0.0, 0.0], [1.0, 0.0]);
    assert!(close(line[0], line[1]), "{line:?}");
}

#[test]
fn perpendicular_drops_foot_on_the_supporting_line() {
    let line = perpendicular([0.5, 0.7], [[0.0, 0.0], [1.0, 0.0]]);
    assert!(close(line[0], [0.5, 0.7]));
    assert!(close(line[1], [0.5, 0.0]), "{line:?}");
    // 線分の外側へ落ちる足も、線分を延長した直線の上に返す(作図の補助として使うため)
    let out = perpendicular([2.0, 1.0], [[0.0, 0.0], [1.0, 0.0]]);
    assert!(close(out[1], [2.0, 0.0]), "{out:?}");
}

#[test]
fn divide_points_returns_n_minus_1_inner_points() {
    let pts = divide_points([[0.0, 0.0], [1.0, 0.0]], 4);
    assert_eq!(pts.len(), 3);
    assert!(close(pts[0], [0.25, 0.0]));
    assert!(close(pts[1], [0.5, 0.0]));
    assert!(close(pts[2], [0.75, 0.0]));
    assert_eq!(divide_points([[0.0, 0.0], [1.0, 1.0]], 8).len(), 7);
    // 2〜8の外は作らない(空を返す)
    assert!(divide_points([[0.0, 0.0], [1.0, 0.0]], 1).is_empty());
    assert!(divide_points([[0.0, 0.0], [1.0, 0.0]], 9).is_empty());
    // 潰れた線分も作らない
    assert!(divide_points([[0.5, 0.5], [0.5, 0.5]], 4).is_empty());
}

#[test]
fn direction_lines_cover_a_half_turn_at_the_given_step() {
    let lines = direction_lines([0.5, 0.5], 22.5);
    assert_eq!(lines.len(), 8, "22.5°刻みなら180°を8本で分ける");
    // 全ての線が指定した点を通る(中点が指定点)
    for l in &lines {
        let mid = [(l[0][0] + l[1][0]) / 2.0, (l[0][1] + l[1][1]) / 2.0];
        assert!(close(mid, [0.5, 0.5]), "{l:?}");
    }
    // 1本目は水平
    assert!((lines[0][0][1] - 0.5).abs() < 1e-9 && (lines[0][1][1] - 0.5).abs() < 1e-9);
    assert_eq!(direction_lines([0.0, 0.0], 45.0).len(), 4);
    // 刻みが不正なら作らない
    assert!(direction_lines([0.0, 0.0], 0.0).is_empty());
    assert!(direction_lines([0.0, 0.0], 200.0).is_empty());
}
