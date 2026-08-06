//! 曲線の折り目(CPE-011)の分割・挿入のテスト。

use glam::DVec2;
use ori3_cp::curve::{
    DEFAULT_CURVE_TOL, MAX_CURVE_SEGMENTS, arc_polyline, arc_segment_count, cubic_polyline,
    insert_polyline, insert_rulings, ruling_lines,
};
use ori3_cp::{extract_faces, local_violations, validate};
use ori3_model::{Document, EdgeKind, Paper};

fn square() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

/// 点pから折れ線までの最短距離。
fn dist_to_polyline(p: DVec2, pts: &[[f64; 2]]) -> f64 {
    pts.windows(2)
        .map(|w| {
            let (a, b) = (DVec2::from(w[0]), DVec2::from(w[1]));
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
            (p - (a + ab * t)).length()
        })
        .fold(f64::MAX, f64::min)
}

#[test]
fn 円弧は指定した誤差以内の折れ線になる() {
    for tol in [0.02, 0.005, 0.001] {
        let pts = arc_polyline([0.0, 0.0], [0.5, 0.4], [1.0, 0.0], tol, None);
        // 真の円弧を細かくサンプルし、折れ線からの離れ方が tol 以内であること
        let fine = arc_polyline([0.0, 0.0], [0.5, 0.4], [1.0, 0.0], 1e-7, None);
        let worst = fine
            .iter()
            .map(|&q| dist_to_polyline(DVec2::from(q), &pts))
            .fold(0.0, f64::max);
        assert!(worst <= tol, "tol={tol} で誤差 {worst}");
    }
}

#[test]
fn 分割数は細かさの指定に応じて増える() {
    let n_coarse = arc_segment_count(0.5, std::f64::consts::PI, 0.02);
    let n_fine = arc_segment_count(0.5, std::f64::consts::PI, 0.001);
    assert!(n_coarse >= 1 && n_fine > n_coarse, "{n_coarse} < {n_fine}");
    // 半円・半径0.5・既定の誤差ならおよそ10〜20分割(辺数が爆発しない)
    let n = arc_segment_count(0.5, std::f64::consts::PI, DEFAULT_CURVE_TOL);
    assert!((8..=24).contains(&n), "分割数 {n}");
    assert_eq!(arc_segment_count(0.5, std::f64::consts::PI, 1e-12), MAX_CURVE_SEGMENTS);
}

#[test]
fn 端点は指定した座標そのものになる() {
    let pts = arc_polyline([0.1, 0.2], [0.5, 0.9], [0.8, 0.3], DEFAULT_CURVE_TOL, None);
    assert_eq!(pts[0], [0.1, 0.2]);
    assert_eq!(*pts.last().unwrap(), [0.8, 0.3]);
    // 通過点のそばを必ず通る
    assert!(dist_to_polyline(DVec2::new(0.5, 0.9), &pts) <= DEFAULT_CURVE_TOL);
}

#[test]
fn 一直線の3点はただの線分になる() {
    let pts = arc_polyline([0.0, 0.0], [0.5, 0.5], [1.0, 1.0], DEFAULT_CURVE_TOL, None);
    assert_eq!(pts, vec![[0.0, 0.0], [1.0, 1.0]]);
}

#[test]
fn 分割数を指定すればその数になり上限で頭打ちになる() {
    let pts = arc_polyline([0.0, 0.0], [0.5, 0.4], [1.0, 0.0], 0.005, Some(5));
    assert_eq!(pts.len(), 6);
    let many = arc_polyline([0.0, 0.0], [0.5, 0.4], [1.0, 0.0], 0.005, Some(10_000));
    assert_eq!(many.len(), MAX_CURVE_SEGMENTS as usize + 1);
}

#[test]
fn ベジェも指定した誤差以内の折れ線になる() {
    let (p0, c1, c2, p1) = ([0.0, 0.0], [0.0, 1.0], [1.0, -0.5], [1.0, 0.5]);
    for tol in [0.02, 0.005, 0.001] {
        let pts = cubic_polyline(p0, c1, c2, p1, tol, None);
        let fine = cubic_polyline(p0, c1, c2, p1, 1e-7, None);
        let worst = fine
            .iter()
            .map(|&q| dist_to_polyline(DVec2::from(q), &pts))
            .fold(0.0, f64::max);
        assert!(worst <= tol, "tol={tol} で誤差 {worst}");
    }
    // 制御点が一直線上ならただの線分でよい
    let straight = cubic_polyline([0.0, 0.0], [0.25, 0.25], [0.75, 0.75], [1.0, 1.0], 0.005, None);
    assert_eq!(straight, vec![[0.0, 0.0], [1.0, 1.0]]);
}

#[test]
fn 曲線は折れ線として展開図に入り面が取れる() {
    let mut doc = square();
    let pts = arc_polyline([0.0, 0.3], [0.5, 0.6], [1.0, 0.3], DEFAULT_CURVE_TOL, None);
    let ids = insert_polyline(&mut doc.cp, &pts, EdgeKind::Valley);
    assert_eq!(ids.len(), pts.len() - 1, "区間の数だけ辺ができる");
    let faces = extract_faces(&doc.cp);
    assert_eq!(faces.len(), 2, "曲線が紙を2つに分ける");
}

#[test]
fn 曲がるための線は曲線に直角で両側へ伸びる() {
    let pts = arc_polyline([0.0, 0.1], [0.5, 0.4], [1.0, 0.1], DEFAULT_CURVE_TOL, None);
    let rulings = ruling_lines(&pts, [1.0, 1.0]);
    assert_eq!(rulings.len(), pts.len() - 2, "内側の点ごとに1本");
    for (i, [concave, cur, convex]) in rulings.iter().enumerate() {
        let tan = DVec2::from(pts[i + 2]) - DVec2::from(pts[i]);
        let dir = DVec2::from(*convex) - DVec2::from(*concave);
        assert!(tan.normalize().dot(dir.normalize()).abs() < 1e-9, "直角でない");
        // 上に膨らむ弧なので、へこむ側(円の中心側)は下
        assert!(concave[1] < cur[1] && convex[1] > cur[1], "向きが逆");
    }
    let mut doc = square();
    insert_polyline(&mut doc.cp, &pts, EdgeKind::Valley);
    insert_rulings(&mut doc.cp, &pts, [1.0, 1.0], EdgeKind::Valley);
    let faces = extract_faces(&doc.cp);
    assert_eq!(faces.len(), 2 * (pts.len() - 1), "区間ごとに表裏2枚ずつ");
}

#[test]
fn 曲線の折り目の警告は一続きにつき1つにまとまる() {
    let mut doc = square();
    let pts = arc_polyline([0.0, 0.3], [0.5, 0.6], [1.0, 0.3], DEFAULT_CURVE_TOL, None);
    insert_polyline(&mut doc.cp, &pts, EdgeKind::Valley);
    // 折れ線の内側の点はすべて川崎の条件を外れる(曲線折りは平らに畳めない)が、
    // 印は1つにまとまる
    assert!(pts.len() - 2 >= 5, "内側の点が十分ある");
    assert_eq!(local_violations(&doc.cp).len(), 1);

    // 曲がるための線を足して交点が4本になっても、まとまり方は変わらない
    insert_rulings(&mut doc.cp, &pts, [1.0, 1.0], EdgeKind::Valley);
    assert_eq!(local_violations(&doc.cp).len(), 1);
}

#[test]
fn まっすぐな折り目の警告は間引かれない() {
    // 中心で交わる十字(山1本・谷1本)は前川の条件を外れる点が1つ。
    // まっすぐな折り目が並ぶだけの点を曲線と取り違えないことの確認
    let mut doc = square();
    insert_polyline(&mut doc.cp, &[[0.0, 0.5], [1.0, 0.5]], EdgeKind::Mountain);
    insert_polyline(&mut doc.cp, &[[0.5, 0.0], [0.5, 1.0]], EdgeKind::Valley);
    assert_eq!(local_violations(&doc.cp).len(), 1);

    // 谷を3本平行に引き、それぞれを横切る山を1本引くと違反する点が3つ並ぶ。
    // まっすぐなので間引かれない
    let mut d2 = square();
    for y in [0.25, 0.5, 0.75] {
        insert_polyline(&mut d2.cp, &[[0.0, y], [1.0, y]], EdgeKind::Valley);
    }
    insert_polyline(&mut d2.cp, &[[0.5, 0.0], [0.5, 1.0]], EdgeKind::Mountain);
    assert_eq!(local_violations(&d2.cp).len(), 3);
}

#[test]
fn 曲線を10本引いても検査が実用的な速さで終わる() {
    // 曲線1本は数十本の折れ線になるので辺数が増える。10本引いた展開図で、
    // 描くたびに走る検査(面抽出・整合性・平坦折り)が目安16msに収まること
    let mut doc = square();
    let t0 = std::time::Instant::now();
    for i in 0..10 {
        let y = 0.05 + 0.09 * f64::from(i);
        let pts = arc_polyline([0.0, y], [0.5, y + 0.06], [1.0, y], DEFAULT_CURVE_TOL, None);
        insert_polyline(&mut doc.cp, &pts, EdgeKind::Valley);
        insert_rulings(&mut doc.cp, &pts, [1.0, 1.0], EdgeKind::Valley);
    }
    let insert_ms = t0.elapsed().as_millis();
    let t1 = std::time::Instant::now();
    let faces = extract_faces(&doc.cp);
    let warn = validate(&doc.cp);
    let violations = local_violations(&doc.cp);
    let check_ms = t1.elapsed().as_millis();
    println!(
        "辺{} 面{} 警告{} 印{} / 挿入{insert_ms}ms 検査{check_ms}ms",
        doc.cp.edges.len(),
        faces.len(),
        warn.len(),
        violations.len()
    );
    assert!(doc.cp.edges.len() > 200, "辺が十分ある: {}", doc.cp.edges.len());
    // デバッグビルドでも余裕を持って通る上限(実測はこの1/10以下)
    assert!(check_ms < 500, "検査が遅い: {check_ms}ms");
}
