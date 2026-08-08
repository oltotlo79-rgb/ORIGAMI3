//! 曲線の折り目(CPE-011)が実際に折れることの受け入れテスト。
//!
//! 曲線折りは平らに畳めない(円弧を折ると紙は必ず立体になる)ので、平坦折りを
//! 前提にした確かめ方は使えない。ここでは角度指定(driver)で折り、
//! - 紙がつながったまま解けること(閉包が満たされる=converged)
//! - 平らでない立体になること(zの広がり)
//! - 分割を細かくするほど滑らかな曲面になること
//!
//! を確かめる。

use glam::DVec3;
use ori3_cp::curve::{arc_polyline, insert_polyline, insert_rulings};
use ori3_cp::{Face, extract_faces};
use ori3_model::{CreasePattern, Document, Driver, EdgeKind, Frame3D, Paper};
use ori3_rigid::solve;
use ori3_soft::{SoftMesh, SoftSettings, relax};

const PAPER: [f64; 2] = [1.0, 1.0];
/// 検証に使う円弧(紙の左端から右端へ、上に膨らむ)。
const ARC: ([f64; 2], [f64; 2], [f64; 2]) = ([0.0, 0.25], [0.5, 0.55], [1.0, 0.25]);

/// 正方形の紙に円弧の折り目を1本引いた展開図と、円弧の折り目の辺ID。
fn arc_cp(with_rulings: bool) -> (CreasePattern, Vec<u32>) {
    let mut doc = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    let pts = arc_polyline(ARC.0, ARC.1, ARC.2, 0.005, None);
    let ids = insert_polyline(&mut doc.cp, &pts, EdgeKind::Valley);
    if with_rulings {
        insert_rulings(&mut doc.cp, &pts, PAPER, EdgeKind::Valley);
    }
    (doc.cp, ids)
}

/// 姿勢の高さの広がり(0なら平らなまま)。
fn z_span(frame: &Frame3D) -> f64 {
    let zs: Vec<f64> = frame
        .faces
        .iter()
        .flat_map(|f| f.polygon.iter().map(|p| p[2]))
        .collect();
    zs.iter().copied().fold(f64::MIN, f64::max) - zs.iter().copied().fold(f64::MAX, f64::min)
}

/// 面の中(同じ面に属する三角形どうし)の折れ角の合計。
/// 剛体折りは面を平らな板として扱うのでこれは0。たわみが効くと0より大きくなる。
fn inner_bend(mesh: &SoftMesh) -> f64 {
    let p = |i: u32| DVec3::from(mesh.positions[i as usize]);
    let normal = |t: [u32; 3]| {
        (p(t[1]) - p(t[0]))
            .cross(p(t[2]) - p(t[0]))
            .normalize_or_zero()
    };
    let mut seen: std::collections::BTreeMap<(u32, u32), usize> = std::collections::BTreeMap::new();
    let mut inner = 0.0;
    for (ti, t) in mesh.triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if let Some(t0) = seen.insert((a.min(b), a.max(b)), ti)
                && mesh.triangle_faces[t0] == mesh.triangle_faces[ti]
            {
                inner += normal(mesh.triangles[t0])
                    .dot(normal(*t))
                    .clamp(-1.0, 1.0)
                    .acos();
            }
        }
    }
    inner
}

fn drive(cp: &CreasePattern, faces: &[Face], hinge: u32, deg: f64) -> ori3_rigid::SolveResult {
    solve(
        cp,
        faces,
        &[Driver {
            hinge,
            target_angle_deg: deg,
        }],
        None,
    )
}

#[test]
fn 円弧の折り目は曲がるための線があれば立体に折れる() {
    let (cp, arc) = arc_cp(true);
    let faces = extract_faces(&cp);
    assert!(
        faces.len() > 2,
        "曲がるための線で面が分かれる: {}",
        faces.len()
    );
    for deg in [-30.0, -60.0, -120.0] {
        let r = drive(&cp, &faces, arc[arc.len() / 2], deg);
        assert!(r.converged, "{deg}度で紙がつながったまま解けない");
        assert!(
            r.frame.warnings.is_empty(),
            "{deg}度で警告: {:?}",
            r.frame.warnings
        );
        assert!(z_span(&r.frame) > 0.2, "{deg}度で立体になっていない");
    }
}

#[test]
fn 折り角度は曲線1本ぜんたいに伝わる() {
    // 円弧の折り目は1本の連続した折り目なので、1か所を折れば全体が同じ角度になる
    let (cp, arc) = arc_cp(true);
    let faces = extract_faces(&cp);
    let r = drive(&cp, &faces, arc[arc.len() / 2], -60.0);
    for id in &arc {
        let a = r.angles[id];
        assert!((a + 60.0).abs() < 1e-3, "辺{id}の角度が {a}");
    }
}

#[test]
fn 曲がるための線がないと円弧の折り目は折れない() {
    // 既知の限界: 曲線の両側を「平らな板」のままにすると、角度0以外では
    // 閉包が満たせない(実際の紙でも両側が曲がらなければ折れない)。
    let (cp, arc) = arc_cp(false);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 2);
    let r = drive(&cp, &faces, arc[arc.len() / 2], -60.0);
    assert!(!r.converged, "板2枚のままでは折れないはず");
}

#[test]
fn 曲線を細かく分けるほど曲面が滑らかになる() {
    // 曲線折りの滑らかさは分割の細かさで決まる。折れ線を細かくするほど、
    // 曲がるための線をまたぐ折れ角(=見た目のカクつき)が小さくなる。
    let mut prev = f64::MAX;
    for tol in [0.02, 0.005, 0.001] {
        let mut doc = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        let pts = arc_polyline(ARC.0, ARC.1, ARC.2, tol, None);
        let ids = insert_polyline(&mut doc.cp, &pts, EdgeKind::Valley);
        insert_rulings(&mut doc.cp, &pts, PAPER, EdgeKind::Valley);
        let faces = extract_faces(&doc.cp);
        let r = drive(&doc.cp, &faces, ids[ids.len() / 2], -90.0);
        assert!(r.converged, "誤差{tol}で折れない");
        let worst = r
            .angles
            .iter()
            .filter(|(k, _)| !ids.contains(k))
            .map(|(_, v)| v.abs())
            .fold(0.0, f64::max);
        assert!(
            worst < prev,
            "誤差{tol}で滑らかにならない({worst}度 >= {prev}度)"
        );
        prev = worst;
    }
    assert!(prev < 10.0, "いちばん細かい分割でも折れ角が {prev} 度");
}

#[test]
fn たわみ計算を通しても曲線の折り目の形は保たれる() {
    let (cp, arc) = arc_cp(true);
    let faces = extract_faces(&cp);
    let r = drive(&cp, &faces, arc[arc.len() / 2], -90.0);
    assert!(r.converged);
    let s = |enabled| SoftSettings {
        enabled,
        subdivision: 2,
        stiffness: 0.3,
        pressure: 0.0,
        iterations: 20,
    };
    let off = relax(&cp, &faces, &r.frame, &s(false));
    let on = relax(&cp, &faces, &r.frame, &s(true));
    assert!(on.triangles.len() > off.triangles.len(), "細分される");
    // 面をまたぐ折り目の角度はたわみ計算でも最強の拘束なので、曲線折りの形は
    // そのまま保たれる(重なった層がないため、面の中もほとんど動かない)。
    // 既知の限界: 曲がるための線のところの折れは、たわみ計算では丸まらない。
    // 滑らかさは折れ線を細かくして得る(下の「細かく分けるほど滑らか」を参照)。
    assert!(inner_bend(&off) < 1e-6, "たわみオフでは面は平らな板のまま");
    assert!(inner_bend(&on) < 1e-3, "重なりがなければ形は変わらない");
    let span = |m: &SoftMesh| {
        let zs: Vec<f64> = m.positions.iter().map(|p| p[2]).collect();
        zs.iter().copied().fold(f64::MIN, f64::max) - zs.iter().copied().fold(f64::MAX, f64::min)
    };
    assert!(
        (span(&on) - z_span(&r.frame)).abs() < 1e-3,
        "立体の高さが変わらない"
    );
    assert!(on.positions.iter().all(|p| p.iter().all(|v| v.is_finite())));
}
