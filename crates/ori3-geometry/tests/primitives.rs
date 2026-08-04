//! 幾何プリミティブ(交差・距離・鏡映・等長変換)の検証。

use glam::DVec2;
use ori3_geometry::{
    Isometry2, dist_point_segment, point_on_segment, reflect_across_line, seg_intersection,
};
use ori3_model::EPS;

fn approx(a: DVec2, b: DVec2) -> bool {
    (a - b).length() < 1e-7
}

// ---- seg_intersection ----

#[test]
fn test_seg_intersection_crossing() {
    // X字型の交差: (0,0)-(1,1) と (0,1)-(1,0) は (0.5,0.5) で交わる。
    let p = seg_intersection(
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 1.0),
        DVec2::new(0.0, 1.0),
        DVec2::new(1.0, 0.0),
    )
    .expect("must intersect");
    assert!(approx(p, DVec2::new(0.5, 0.5)), "got {p:?}");
}

#[test]
fn test_seg_intersection_none() {
    // 直線としては交わるが、線分の範囲外。
    let r = seg_intersection(
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
        DVec2::new(2.0, 1.0),
        DVec2::new(2.0, 2.0),
    );
    assert!(r.is_none(), "got {r:?}");
}

#[test]
fn test_seg_intersection_parallel() {
    // 平行(交点なし)。
    let r = seg_intersection(
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
        DVec2::new(0.0, 1.0),
        DVec2::new(1.0, 1.0),
    );
    assert!(r.is_none(), "got {r:?}");
    // 同一直線上で重なる場合もNone。
    let r = seg_intersection(
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
        DVec2::new(0.5, 0.0),
        DVec2::new(2.0, 0.0),
    );
    assert!(r.is_none(), "got {r:?}");
}

#[test]
fn test_seg_intersection_endpoint_touch() {
    // T字型: bの端点がaの中央に触れる。端点接触も交差として返す。
    let p = seg_intersection(
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
        DVec2::new(0.5, 0.0),
        DVec2::new(0.5, 1.0),
    )
    .expect("endpoint touch must count");
    assert!(approx(p, DVec2::new(0.5, 0.0)), "got {p:?}");

    // L字型: 端点同士の接触。
    let p = seg_intersection(
        DVec2::new(0.0, 0.0),
        DVec2::new(1.0, 0.0),
        DVec2::new(1.0, 0.0),
        DVec2::new(1.0, 1.0),
    )
    .expect("shared endpoint must count");
    assert!(approx(p, DVec2::new(1.0, 0.0)), "got {p:?}");
}

// ---- point_on_segment / dist_point_segment ----

#[test]
fn test_point_on_segment() {
    let a = DVec2::new(0.0, 0.0);
    let b = DVec2::new(1.0, 1.0);
    assert!(point_on_segment(DVec2::new(0.5, 0.5), a, b));
    assert!(point_on_segment(a, a, b)); // 端点
    assert!(point_on_segment(b, a, b));
    // EPS内のずれは許容。
    assert!(point_on_segment(DVec2::new(0.5, 0.5 + EPS * 0.5), a, b));
    // 線分の延長線上は不可。
    assert!(!point_on_segment(DVec2::new(1.5, 1.5), a, b));
    // 線分から離れた点は不可。
    assert!(!point_on_segment(DVec2::new(0.5, 0.6), a, b));
}

#[test]
fn test_dist_point_segment() {
    let a = DVec2::new(0.0, 0.0);
    let b = DVec2::new(2.0, 0.0);
    // 垂線の足が線分内。
    assert!((dist_point_segment(DVec2::new(1.0, 1.0), a, b) - 1.0).abs() < 1e-12);
    // 垂線の足が線分外→近い方の端点までの距離。
    assert!((dist_point_segment(DVec2::new(3.0, 4.0), a, b) - 17.0_f64.sqrt()).abs() < 1e-12);
    assert!((dist_point_segment(DVec2::new(-3.0, 4.0), a, b) - 5.0).abs() < 1e-12);
    // 線分上は0。
    assert!(dist_point_segment(DVec2::new(0.5, 0.0), a, b) < 1e-12);
}

// ---- reflect_across_line ----

#[test]
fn test_reflect_across_line() {
    // 点(1,0)を直線x=0(=(0,0)-(0,1))で鏡映→(-1,0)。
    let p = reflect_across_line(
        DVec2::new(1.0, 0.0),
        DVec2::new(0.0, 0.0),
        DVec2::new(0.0, 1.0),
    );
    assert!(approx(p, DVec2::new(-1.0, 0.0)), "got {p:?}");

    // 原点を通らない斜めの直線 y = x + 1 で (1,0) を鏡映 → (-1,2)。
    let p = reflect_across_line(
        DVec2::new(1.0, 0.0),
        DVec2::new(0.0, 1.0),
        DVec2::new(1.0, 2.0),
    );
    assert!(approx(p, DVec2::new(-1.0, 2.0)), "got {p:?}");

    // 直線上の点は動かない。
    let p = reflect_across_line(
        DVec2::new(0.5, 1.5),
        DVec2::new(0.0, 1.0),
        DVec2::new(1.0, 2.0),
    );
    assert!(approx(p, DVec2::new(0.5, 1.5)), "got {p:?}");
}

// ---- Isometry2 ----

#[test]
fn test_isometry_identity() {
    let iso = Isometry2::identity();
    let p = DVec2::new(0.3, -0.7);
    assert!(approx(iso.apply(p), p));
}

#[test]
fn test_isometry_reflection_matches_reflect_across_line() {
    let l0 = DVec2::new(0.2, 1.0);
    let l1 = DVec2::new(1.3, 2.5);
    let iso = Isometry2::reflection(l0, l1);
    assert!(iso.mirrored);
    for p in [
        DVec2::new(1.0, 0.0),
        DVec2::new(-0.4, 2.0),
        DVec2::new(0.2, 1.0),
    ] {
        let expected = reflect_across_line(p, l0, l1);
        assert!(
            approx(iso.apply(p), expected),
            "p={p:?}: got {:?}, expected {expected:?}",
            iso.apply(p)
        );
    }
    // 鏡映を2回合成すると恒等変換。
    let twice = iso.compose(&iso);
    let p = DVec2::new(0.9, -0.3);
    assert!(approx(twice.apply(p), p), "got {:?}", twice.apply(p));
}

#[test]
fn test_isometry_compose_matches_sequential_apply() {
    use rand::RngExt;
    let mut rng = rand::rng();
    for _ in 0..100 {
        let a = Isometry2 {
            rotation: rng.random_range(-6.0..6.0),
            translation: DVec2::new(rng.random_range(-2.0..2.0), rng.random_range(-2.0..2.0)),
            mirrored: rng.random_bool(0.5),
        };
        let b = Isometry2 {
            rotation: rng.random_range(-6.0..6.0),
            translation: DVec2::new(rng.random_range(-2.0..2.0), rng.random_range(-2.0..2.0)),
            mirrored: rng.random_bool(0.5),
        };
        let p = DVec2::new(rng.random_range(-2.0..2.0), rng.random_range(-2.0..2.0));
        // (a ∘ b).apply(p) == a.apply(b.apply(p)) (bを先に適用)。
        let composed = a.compose(&b).apply(p);
        let sequential = a.apply(b.apply(p));
        assert!(
            approx(composed, sequential),
            "a={a:?} b={b:?} p={p:?}: composed={composed:?} sequential={sequential:?}"
        );
    }
}

#[test]
fn test_isometry_inverse_roundtrip() {
    use rand::RngExt;
    let mut rng = rand::rng();
    for _ in 0..100 {
        let iso = Isometry2 {
            rotation: rng.random_range(-6.0..6.0),
            translation: DVec2::new(rng.random_range(-2.0..2.0), rng.random_range(-2.0..2.0)),
            mirrored: rng.random_bool(0.5),
        };
        let p = DVec2::new(rng.random_range(-2.0..2.0), rng.random_range(-2.0..2.0));
        let back = iso.inverse().apply(iso.apply(p));
        assert!(
            approx(back, p),
            "iso={iso:?} p={p:?}: roundtrip gave {back:?}"
        );
        // inverse ∘ iso も恒等変換として振る舞う。
        let ident = iso.inverse().compose(&iso);
        assert!(approx(ident.apply(p), p), "compose-identity failed: {ident:?}");
    }
}
