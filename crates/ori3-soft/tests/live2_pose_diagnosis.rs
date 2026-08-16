//! 利用者の画面(`live2`)の姿勢について、**測るだけ**の診断。
//!
//! 合否を付けない `#[ignore]` の測定用テストだけを置く。目的は次の2点を分けること。
//!
//! 1. 剛体折りが返す生の形は、平らに畳んだ束が本当に「同じ平面」に乗っているか。
//! 2. 束をばらけさせているのは、solveの残差か、それとも表示前の接触補正
//!    (`ori3_soft::prevent_overlap_with_order_authority`、`LAYER_GAP = 0.002`)か。
//!
//! 展開図と折り角は、`verification/` などの追跡対象外を読まずにここへ直接書く(CLAUDE.md §10.1)。
//! 展開図は `apps/desktop/src-tauri/src/surface_order_acceptance.rs::live_frame_cp` と同一で、
//! 折り角だけが違う(利用者が辺35を −1.73° から −35° へ動かした結果の姿勢)。

use std::collections::HashMap;

use ori3_cp::{Face, extract_faces};
use ori3_model::{CreasePattern, Edge, EdgeId, EdgeKind, Vertex};
use ori3_model::{FaceId, Frame3D};
use ori3_rigid::{propagate, to_frame3d};

fn vertex(id: u32, x: f64, y: f64) -> Vertex {
    Vertex { id, pos: [x, y] }
}

fn edge(id: EdgeId, v0: u32, v1: u32, kind: EdgeKind) -> Edge {
    Edge { id, v0, v1, kind }
}

/// 利用者の展開図(頂点13・辺28)。`live_frame_cp` と同じ。
fn live_cp() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 1.0, 0.0),
            vertex(2, 1.0, 1.0),
            vertex(3, 0.0, 1.0),
            vertex(4, 1.0, 0.5),
            vertex(5, 0.0, 0.5),
            vertex(6, 0.5, 1.0),
            vertex(7, 0.5, 0.0),
            vertex(8, 0.5, 0.5),
            vertex(9, 0.5, 0.792_893_218_813_452_5),
            vertex(10, 0.792_893_218_813_452_5, 0.5),
            vertex(11, 0.5, 0.207_106_781_186_547_52),
            vertex(12, 0.207_106_781_186_547_52, 0.5),
        ],
        edges: vec![
            edge(4, 1, 4, EdgeKind::Border),
            edge(5, 4, 2, EdgeKind::Border),
            edge(6, 3, 5, EdgeKind::Border),
            edge(7, 5, 0, EdgeKind::Border),
            edge(9, 2, 6, EdgeKind::Border),
            edge(10, 6, 3, EdgeKind::Border),
            edge(11, 0, 7, EdgeKind::Border),
            edge(12, 7, 1, EdgeKind::Border),
            edge(17, 0, 8, EdgeKind::Valley),
            edge(18, 8, 2, EdgeKind::Valley),
            edge(19, 6, 9, EdgeKind::Mountain),
            edge(20, 9, 8, EdgeKind::Mountain),
            edge(21, 2, 9, EdgeKind::Mountain),
            edge(22, 4, 10, EdgeKind::Mountain),
            edge(23, 10, 8, EdgeKind::Mountain),
            edge(24, 2, 10, EdgeKind::Mountain),
            edge(25, 10, 1, EdgeKind::Mountain),
            edge(26, 8, 11, EdgeKind::Mountain),
            edge(27, 11, 7, EdgeKind::Mountain),
            edge(28, 0, 11, EdgeKind::Mountain),
            edge(29, 11, 1, EdgeKind::Mountain),
            edge(30, 8, 12, EdgeKind::Mountain),
            edge(31, 12, 5, EdgeKind::Mountain),
            edge(32, 0, 12, EdgeKind::Mountain),
            edge(33, 12, 3, EdgeKind::Mountain),
            edge(34, 3, 9, EdgeKind::Mountain),
            edge(35, 12, 9, EdgeKind::Valley),
            edge(36, 10, 11, EdgeKind::Valley),
        ],
        next_vertex_id: 13,
        next_edge_id: 37,
    }
}

/// 利用者の画面が実際に表示していた解(`live2-frame.json` の `poseAngles`)。
fn live2_solved_angles() -> HashMap<EdgeId, f64> {
    HashMap::from([
        (17, -179.957_082_142_683_45),
        (18, -179.995_417_140_379_54),
        (19, -162.504_692_427_436_82),
        (20, 179.999_999_999_999_97),
        (21, 180.0),
        (22, -0.232_786_692_480_804_14),
        (23, 179.966_411_921_299_28),
        (24, 179.972_443_823_207_39),
        (25, 179.998_151_008_391_03),
        (26, 179.984_999_262_888_55),
        (27, -0.252_528_328_276_677_54),
        (28, 179.988_178_110_990_08),
        (29, 179.999_659_174_636_66),
        (30, 179.981_412_657_370_6),
        (31, -162.486_104_949_067_94),
        (32, 179.985_781_176_608_84),
        (33, 179.999_992_332_109_06),
        (34, 179.999_999_999_999_97),
        (35, -17.495_307_572_563_128),
        (36, -179.732_210_079_595_26),
    ])
}

/// 利用者が要求した角(`live2-frame.json` の `drivers`)。
fn live2_requested_angles() -> HashMap<EdgeId, f64> {
    let mut angles = live2_solved_angles();
    angles.insert(17, -180.0);
    angles.insert(18, -180.0);
    angles.insert(19, -178.265_130_385_534_97);
    angles.insert(20, 180.0);
    angles.insert(22, -3.062_204_584_590_538_5e-15);
    angles.insert(23, 180.0);
    angles.insert(24, 180.0);
    angles.insert(25, 180.0);
    angles.insert(26, 180.0);
    angles.insert(27, -5.233_885_113_024_099e-15);
    angles.insert(28, 180.0);
    angles.insert(29, 180.0);
    angles.insert(30, 179.999_999_999_999_97);
    angles.insert(31, -178.265_130_385_534_97);
    angles.insert(32, 180.0);
    angles.insert(33, 180.0);
    angles.insert(34, 180.0);
    angles.insert(35, -35.0);
    angles.insert(36, -180.0);
    angles
}

/// 直前まで表示されていた、完全に畳んだ姿勢(`live-frame`)。辺35だけが −1.73°。
fn live_frame_angles() -> HashMap<EdgeId, f64> {
    HashMap::from([
        (17, -180.0),
        (18, -180.0),
        (19, -178.265_130_385_534_97),
        (20, 180.0),
        (21, 180.0),
        (22, -3.062_204_584_590_538_5e-15),
        (23, 180.0),
        (24, 180.0),
        (25, 180.0),
        (26, 180.0),
        (27, -5.233_885_113_024_099e-15),
        (28, 180.0),
        (29, 180.0),
        (30, 179.999_999_999_999_97),
        (31, -178.265_130_385_534_97),
        (32, 180.0),
        (33, 180.0),
        (34, 180.0),
        (35, -1.734_869_614_465_027),
        (36, -180.0),
    ])
}

fn frame_of(cp: &CreasePattern, faces: &[Face], angles: &HashMap<EdgeId, f64>) -> Frame3D {
    to_frame3d(cp, faces, &propagate(cp, faces, angles))
}

fn rank_order(frame: &Frame3D) -> Vec<FaceId> {
    let mut ranked = frame
        .faces
        .iter()
        .map(|face| (face.surface_rank, face.face))
        .collect::<Vec<_>>();
    ranked.sort_unstable();
    ranked.into_iter().map(|(_, face)| face).collect()
}

fn apply_overlap(cp: &CreasePattern, faces: &[Face], frame: &mut Frame3D, enabled: bool) {
    let order = rank_order(frame);
    ori3_soft::prevent_overlap_with_order_authority(
        cp,
        faces,
        frame,
        ori3_soft::OverlapOrderInput {
            start: &order,
            end: &order,
            progress: 0.5,
            authoritative: true,
        },
        &ori3_soft::OverlapSettings {
            enabled,
            ..Default::default()
        },
    );
}

/// 面の法線を、絶対値が最大の成分が正になる側へそろえる(画面・rigidと同じ規約)。
fn canonical(normal: [f64; 3]) -> [f64; 3] {
    let a = [normal[0].abs(), normal[1].abs(), normal[2].abs()];
    let k = if a[0] >= a[1] && a[0] >= a[2] {
        0
    } else if a[1] >= a[2] {
        1
    } else {
        2
    };
    if normal[k] < 0.0 {
        [-normal[0], -normal[1], -normal[2]]
    } else {
        normal
    }
}

/// 面の平面(そろえた法線と原点からの符号付き距離)。
fn plane_of(polygon: &[[f64; 3]]) -> ([f64; 3], f64) {
    let mut n = [0.0f64; 3];
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    let n = canonical([n[0] / len, n[1] / len, n[2] / len]);
    let d = polygon
        .iter()
        .map(|p| n[0] * p[0] + n[1] * p[1] + n[2] * p[2])
        .sum::<f64>()
        / polygon.len() as f64;
    (n, d)
}

/// 「ほぼ平行(canonical法線の差 ≤ 1e-2)」な面どうしの、平面距離の最大値。
/// これが束(同じ紙の重なり)のばらけ幅である。
fn parallel_spread(frame: &Frame3D) -> (f64, String) {
    let planes = frame
        .faces
        .iter()
        .map(|face| (face.face, face.surface_rank, plane_of(&face.polygon)))
        .collect::<Vec<_>>();
    let mut worst = 0.0f64;
    let mut detail = String::new();
    for (i, (fa, ra, (na, da))) in planes.iter().enumerate() {
        for (fb, rb, (nb, db)) in planes.iter().skip(i + 1) {
            let dn = ((na[0] - nb[0]).powi(2) + (na[1] - nb[1]).powi(2) + (na[2] - nb[2]).powi(2))
                .sqrt();
            if dn > 1e-2 {
                continue;
            }
            let gap = (da - db).abs();
            if gap > worst {
                worst = gap;
                detail = format!("face{fa}(rank{ra}) - face{fb}(rank{rb})");
            }
        }
    }
    (worst, detail)
}

fn report_spread(label: &str, frame: &Frame3D) {
    let (spread, pair) = parallel_spread(frame);
    let mut heights = frame
        .faces
        .iter()
        .map(|face| (face.face, face.surface_rank, plane_of(&face.polygon).1))
        .collect::<Vec<_>>();
    heights.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());
    println!("LIVE2 {label} 平行な面の平面距離の最大差={spread:.6e} ({pair})");
    println!(
        "LIVE2 {label} 低い順 = {}",
        heights
            .iter()
            .map(|(f, r, d)| format!("face{f}(rank{r},{d:+.3e})"))
            .collect::<Vec<_>>()
            .join(" ")
    );
}

/// 段階A-1: solveの生の形と、接触補正を入れた形を並べて測る。
#[test]
#[ignore = "測定用。合否を付けない"]
fn live2_spread_before_and_after_overlap_correction() {
    let cp = live_cp();
    let faces = extract_faces(&cp);
    for (label, angles) in [
        ("live2(利用者の画面)", live2_solved_angles()),
        ("live-frame(直前の完全に畳んだ姿勢)", live_frame_angles()),
    ] {
        println!("---- {label} ----");
        let raw = frame_of(&cp, &faces, &angles);
        report_spread("補正なし", &raw);
        let flat = raw
            .faces
            .iter()
            .flat_map(|f| f.polygon.iter())
            .map(|p| p[2].abs())
            .fold(0.0f64, f64::max);
        println!("LIVE2 補正なし |z|の最大 = {flat:.6e} (1e-6未満なら接触補正はまるごと省かれる)");
        let mut corrected = raw.clone();
        apply_overlap(&cp, &faces, &mut corrected, true);
        report_spread("補正あり", &corrected);
        let moved = raw
            .faces
            .iter()
            .zip(&corrected.faces)
            .flat_map(|(a, b)| a.polygon.iter().zip(&b.polygon))
            .map(|(p, q)| {
                ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
            })
            .fold(0.0f64, f64::max);
        println!("LIVE2 補正が頂点を動かした最大距離 = {moved:.6e}");
    }
}

/// 段階A-2: 利用者が要求した角と、solveが返した角の食い違いを測る。
/// あわせて「要求どおりの角をそのまま伝播したら紙は閉じるのか」を測る。
#[test]
#[ignore = "測定用。合否を付けない"]
fn live2_requested_angles_versus_solved_angles() {
    let cp = live_cp();
    let faces = extract_faces(&cp);
    let requested = live2_requested_angles();
    let solved = live2_solved_angles();
    let mut keys = requested.keys().copied().collect::<Vec<_>>();
    keys.sort_unstable();
    for edge in keys {
        let r = requested[&edge];
        let s = solved[&edge];
        println!("LIVE2ANGLE edge{edge} 要求={r:+.6} 解={s:+.6} 差={:+.6}", s - r);
    }
    // 要求どおりの角をそのまま伝播したときの裂け(紙が閉じるか)。
    let requested_frame = frame_of(&cp, &faces, &requested);
    let solved_frame = frame_of(&cp, &faces, &solved);
    println!(
        "LIVE2SEAM 要求どおりの角の裂け = {:.6e}",
        ori3_rigid::max_seam_gap(&cp, &faces, &requested_frame)
    );
    println!(
        "LIVE2SEAM solveが返した角の裂け = {:.6e}",
        ori3_rigid::max_seam_gap(&cp, &faces, &solved_frame)
    );
}

/// 17本を厳密に ±180°/0° に固定したまま、残る3本(19・31・35)だけを動かして
/// 紙が閉じる組み合わせがあるかを掃く。あるなら「平らな束を厳密に保った解」が
/// 存在することになり、無いなら残差は避けられない。
#[test]
#[ignore = "測定用。合否を付けない"]
fn live2_exact_flat_seventeen_creases_search() {
    let cp = live_cp();
    let faces = extract_faces(&cp);
    let exact = |a: f64, b: f64| -> HashMap<EdgeId, f64> {
        let mut angles = HashMap::from([
            (17, -180.0),
            (18, -180.0),
            (20, 180.0),
            (21, 180.0),
            (22, 0.0),
            (23, 180.0),
            (24, 180.0),
            (25, 180.0),
            (26, 180.0),
            (27, 0.0),
            (28, 180.0),
            (29, 180.0),
            (30, 180.0),
            (32, 180.0),
            (33, 180.0),
            (34, 180.0),
            (36, -180.0),
        ]);
        angles.insert(19, a);
        angles.insert(31, a);
        angles.insert(35, b);
        angles
    };
    let seam = |a: f64, b: f64| {
        let angles = exact(a, b);
        ori3_rigid::max_seam_gap(&cp, &faces, &frame_of(&cp, &faces, &angles))
    };
    // 参考: live-frame が実際に取っていた組み合わせ
    println!(
        "FLAT17 live-frame の組 (19=31=-178.265, 35=-1.735) の裂け = {:.6e}",
        seam(-178.265_130_385_534_97, -1.734_869_614_465_027)
    );
    println!(
        "FLAT17 利用者の解の組 (19=31=-162.5, 35=-17.5) の裂け = {:.6e}",
        seam(-162.495, -17.495_307_572_563_128)
    );
    // 35 を刻み、その各点で 19=31 を細かく掃いて最小の裂けを探す。
    for b_i in 0..=8 {
        let b = -(b_i as f64) * 5.0;
        let mut best = (f64::INFINITY, 0.0);
        let mut a = -180.0;
        while a <= -100.0 {
            let g = seam(a, b);
            if g < best.0 {
                best = (g, a);
            }
            a += 0.05;
        }
        println!(
            "FLAT17 辺35={b:+.1}°: 19=31 を -180..-100 で掃いた最小の裂け = {:.6e} (そのときの19=31={:+.2}°)",
            best.0, best.1
        );
    }
}

/// 実際のsolverを利用者と同じ入力で走らせ、収束の報告と実際の食い違いを並べる。
#[test]
#[ignore = "測定用。合否を付けない"]
fn live2_solver_reports_versus_actual_driver_error() {
    let cp = live_cp();
    let faces = extract_faces(&cp);
    let requested = live2_requested_angles();
    let mut hinges = requested.keys().copied().collect::<Vec<_>>();
    hinges.sort_unstable();
    let drivers = hinges
        .iter()
        .map(|&hinge| ori3_model::Driver {
            hinge,
            target_angle_deg: requested[&hinge],
        })
        .collect::<Vec<_>>();
    let warm = live_frame_angles();
    let motion = ori3_rigid::solve_motion(&cp, &faces, &drivers, None, Some(&warm), true);
    let result = motion.result;
    let worst = hinges
        .iter()
        .map(|h| (result.angles[h] - requested[h]).abs())
        .fold(0.0f64, f64::max);
    println!(
        "SOLVE converged={} best_effort={} closure_rms={:.6e} warnings={:?}",
        result.converged, result.best_effort, result.closure_rms, result.frame.warnings
    );
    println!("SOLVE 要求との最大の食い違い = {worst:.6}°");
    for h in &hinges {
        let d = result.angles[h] - requested[h];
        if d.abs() > 1e-3 {
            println!("SOLVE   edge{h} 要求={:+.6} 解={:+.6} 差={d:+.6}", requested[h], result.angles[h]);
        }
    }
    println!(
        "SOLVE 裂け = {:.6e}",
        ori3_rigid::max_seam_gap(&cp, &faces, &result.frame)
    );
    report_spread("solve_motionの出力", &result.frame);
}

/// 拘束の与え方(hard/preferred の分け方)を変えて、残差がどう変わるかを測る。
/// 1変数(どの折り目をhardにするか)だけを変えて比べる。
#[test]
#[ignore = "測定用。合否を付けない"]
fn live2_hard_preferred_split_changes_the_residual() {
    let cp = live_cp();
    let faces = extract_faces(&cp);
    let requested = live2_requested_angles();
    let mut hinges = requested.keys().copied().collect::<Vec<_>>();
    hinges.sort_unstable();
    let driver = |h: EdgeId| ori3_model::Driver {
        hinge: h,
        target_angle_deg: requested[&h],
    };
    let warm = live_frame_angles();
    // 「平らに畳む」ことを要求している17本。ここが厳密でないと表示が壊れる。
    let flat_creases: Vec<EdgeId> = hinges
        .iter()
        .copied()
        .filter(|h| ![19, 31, 35].contains(h))
        .collect();
    let cases: Vec<(&str, Vec<EdgeId>)> = vec![
        ("A 全20本がhard", hinges.clone()),
        ("B 辺35だけhard・残り19本preferred", vec![35]),
        ("C 平らな17本がhard・19/31/35がpreferred", flat_creases.clone()),
        ("D 平らな17本と35がhard・19/31がpreferred", {
            let mut v = flat_creases.clone();
            v.push(35);
            v
        }),
    ];
    for (label, hard_ids) in cases {
        let hard: Vec<_> = hard_ids.iter().copied().map(driver).collect();
        let preferred: Vec<_> = hinges
            .iter()
            .copied()
            .filter(|h| !hard_ids.contains(h))
            .map(driver)
            .collect();
        let targets = (!preferred.is_empty()).then(|| {
            preferred
                .iter()
                .map(|d| (d.hinge, d.target_angle_deg))
                .collect::<HashMap<_, _>>()
        });
        let motion =
            ori3_rigid::solve_motion(&cp, &faces, &hard, targets.as_ref(), Some(&warm), true);
        let r = motion.result;
        let flat_error = flat_creases
            .iter()
            .map(|h| (r.angles[h] - requested[h]).abs())
            .fold(0.0f64, f64::max);
        let (spread, _) = parallel_spread(&r.frame);
        println!(
            "SPLIT {label}: converged={} rms={:.3e} 裂け={:.3e} 平らな17本の最大誤差={flat_error:.4}° 束のばらけ={spread:.3e} 辺35={:+.3}° 辺19={:+.3}° 辺31={:+.3}°",
            r.converged,
            r.closure_rms,
            ori3_rigid::max_seam_gap(&cp, &faces, &r.frame),
            r.angles[&35],
            r.angles[&19],
            r.angles[&31],
        );
    }
}

/// 比べたい各姿勢のフレームを、画面側の測定器へ渡せる形で標準出力へ書く。
/// (ファイルへは書かない。§10.7.6)
#[test]
#[ignore = "測定用。合否を付けない"]
fn live2_dump_frames_for_the_screen_measurement() {
    let cp = live_cp();
    let faces = extract_faces(&cp);
    let requested = live2_requested_angles();
    let mut hinges = requested.keys().copied().collect::<Vec<_>>();
    hinges.sort_unstable();
    let driver = |h: EdgeId| ori3_model::Driver {
        hinge: h,
        target_angle_deg: requested[&h],
    };
    let warm = live_frame_angles();
    let flat_creases: Vec<EdgeId> = hinges
        .iter()
        .copied()
        .filter(|h| ![19, 31, 35].contains(h))
        .collect();
    let mut split_d = flat_creases.clone();
    split_d.push(35);

    let mut dumps: Vec<(String, Frame3D)> = Vec::new();
    dumps.push(("live2-actual".to_string(), frame_of(&cp, &faces, &live2_solved_angles())));
    dumps.push(("live-frame".to_string(), frame_of(&cp, &faces, &live_frame_angles())));
    for (label, hard_ids) in [("split-C", flat_creases.clone()), ("split-D", split_d)] {
        let hard: Vec<_> = hard_ids.iter().copied().map(driver).collect();
        let preferred: Vec<_> = hinges
            .iter()
            .copied()
            .filter(|h| !hard_ids.contains(h))
            .map(driver)
            .collect();
        let targets = preferred
            .iter()
            .map(|d| (d.hinge, d.target_angle_deg))
            .collect::<HashMap<_, _>>();
        let motion =
            ori3_rigid::solve_motion(&cp, &faces, &hard, Some(&targets), Some(&warm), true);
        dumps.push((label.to_string(), motion.result.frame));
    }
    for (label, frame) in dumps {
        let faces_json = frame
            .faces
            .iter()
            .map(|f| {
                let polygon = f
                    .polygon
                    .iter()
                    .map(|p| format!("[{:.17e},{:.17e},{:.17e}]", p[0], p[1], p[2]))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "{{\"face\":{},\"polygon\":[{}],\"layer\":{},\"surface_rank\":{},\"mirrored\":{}}}",
                    f.face, polygon, f.layer, f.surface_rank, f.mirrored
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        println!("FRAMEDUMP {label} {{\"faces\":[{faces_json}]}}");
    }
}

/// **恒久の検査**: 利用者が要求した形へ、実際に到達できることを主張する。
///
/// 画面側 `apps/desktop/src/lib/settledFolds.ts` が「0°/±180° まで折り切ってある
/// 折り目は厳密に保つ側へ回す」ようにしたので、追従計算へ渡る分け方はここと同じになる。
/// 展開図と折り角はこのファイルへ直接書いており、`verification/` を読まない(§10.1)。
///
/// 直す前の実測(同じ入力で、折り切った17本も譲れる側にしていたとき):
///   辺35 = −17.495°(要求 −35°)/ 平らな17本の誤差 0.268° / 束のばらけ 2.473e-3
#[test]
fn live2_requested_pose_is_reached_when_settled_creases_are_kept_exact() {
    let cp = live_cp();
    let faces = extract_faces(&cp);
    let requested = live2_requested_angles();
    let mut hinges = requested.keys().copied().collect::<Vec<_>>();
    hinges.sort_unstable();

    // 画面側 `splitSettledFolds` と同じ判定(0° または ±180° から 1e-6 度以内)。
    let settled = |deg: f64| deg.abs() <= 1e-6 || (deg.abs() - 180.0).abs() <= 1e-6;
    // 利用者がいま動かしている折り目は辺35。
    let dragged: EdgeId = 35;
    let hard_ids = hinges
        .iter()
        .copied()
        .filter(|h| *h == dragged || settled(requested[h]))
        .collect::<Vec<_>>();
    assert_eq!(
        hard_ids.len(),
        18,
        "折り切った17本と、動かしている辺35の合計18本を厳密に保つ"
    );
    assert!(
        !hard_ids.contains(&19) && !hard_ids.contains(&31),
        "−178.265° は折り切っていないので譲れる側に残す"
    );

    let driver = |h: EdgeId| ori3_model::Driver {
        hinge: h,
        target_angle_deg: requested[&h],
    };
    let hard = hard_ids.iter().copied().map(driver).collect::<Vec<_>>();
    let targets = hinges
        .iter()
        .copied()
        .filter(|h| !hard_ids.contains(h))
        .map(|h| (h, requested[&h]))
        .collect::<HashMap<_, _>>();
    let motion = ori3_rigid::solve_motion(
        &cp,
        &faces,
        &hard,
        Some(&targets),
        Some(&live_frame_angles()),
        true,
    );
    let result = motion.result;

    assert!(result.converged, "追従計算が収束すること");
    // 1. 利用者が指定した角度へ到達する。
    assert!(
        (result.angles[&dragged] - (-35.0)).abs() <= 1e-9,
        "辺35は要求どおり −35.000° になること (実際 {})",
        result.angles[&dragged]
    );
    // 2. 折り切ってある17本は1本も開かない。
    let flat_error = hinges
        .iter()
        .filter(|h| settled(requested[h]))
        .map(|h| (result.angles[h] - requested[h]).abs())
        .fold(0.0f64, f64::max);
    assert!(
        flat_error <= 1e-9,
        "折り切った17本の誤差が0であること (実際 {flat_error} 度)"
    );
    // 3. 重なった紙の束がばらけない。ばらけると重なり順が幾何から決まらず、
    //    見えないはずの紙の裏が出る。3D表示が同じ高さとみなせる幅は 2.78e-5。
    let (spread, pair) = parallel_spread(&result.frame);
    assert!(
        spread <= 1e-15,
        "平行な面どうしの高さの差が 1e-15 以下であること (実際 {spread:.3e}, {pair})"
    );
    // 4. 紙が裂けていない。
    let seam = ori3_rigid::max_seam_gap(&cp, &faces, &result.frame);
    assert!(seam < 1e-6, "紙が裂けていないこと (実際 {seam:.3e})");
}

/// **恒久の検査**: 同じ入力を10回解いて同じ結果になること(§10.6 の決定性)。
#[test]
fn live2_settled_crease_pose_is_deterministic_ten_times() {
    let cp = live_cp();
    let faces = extract_faces(&cp);
    let requested = live2_requested_angles();
    let mut hinges = requested.keys().copied().collect::<Vec<_>>();
    hinges.sort_unstable();
    let settled = |deg: f64| deg.abs() <= 1e-6 || (deg.abs() - 180.0).abs() <= 1e-6;
    let hard_ids = hinges
        .iter()
        .copied()
        .filter(|h| *h == 35 || settled(requested[h]))
        .collect::<Vec<_>>();
    let driver = |h: EdgeId| ori3_model::Driver {
        hinge: h,
        target_angle_deg: requested[&h],
    };
    let hard = hard_ids.iter().copied().map(driver).collect::<Vec<_>>();
    let targets = hinges
        .iter()
        .copied()
        .filter(|h| !hard_ids.contains(h))
        .map(|h| (h, requested[&h]))
        .collect::<HashMap<_, _>>();
    let run = || {
        let motion = ori3_rigid::solve_motion(
            &cp,
            &faces,
            &hard,
            Some(&targets),
            Some(&live_frame_angles()),
            true,
        );
        let mut ranks = motion
            .result
            .frame
            .faces
            .iter()
            .map(|f| (f.face, f.surface_rank))
            .collect::<Vec<_>>();
        ranks.sort_unstable();
        let mut angles = motion.result.angles.into_iter().collect::<Vec<_>>();
        angles.sort_by_key(|(h, _)| *h);
        (ranks, format!("{angles:?}"))
    };
    let first = run();
    for attempt in 1..10 {
        assert_eq!(run(), first, "{attempt}回目が1回目と違う");
    }
}
