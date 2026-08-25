//! `sa-warm-path.ori3` の実 solver 出力を、Viewer3D と同じ CPU raster で測る契約。
//! 段階1で赤を確認し、案1で緑になったため通常suiteへ登録する。
//!
//! ```text
//! #[path = "surface_order_sa_warm_path_canonical.rs"]
//! mod surface_order_sa_warm_path_canonical;
//! ```

use super::*;
use crate::commands::{PoseSolveInput, PoseSolveMode, pose_solve_core, pose_solve_core_with_mode};
use crate::store::DocumentStore;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SA_WARM_PATH: &str =
    include_str!("../../../../crates/ori3-rigid/tests/fixtures/sa-warm-path.ori3");
const ALL_UI_PATHS: [[EdgeId; 3]; 6] = [
    [17, 19, 21],
    [17, 21, 19],
    [19, 17, 21],
    [19, 21, 17],
    [21, 17, 19],
    [21, 19, 17],
];
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[derive(Debug)]
struct SolvedPath {
    order: [EdgeId; 3],
    frame: Frame3D,
    angles: Vec<(EdgeId, f64)>,
}

#[derive(PartialEq, Eq)]
struct ExposureImage {
    /// 0=裏ではない、1=表triangleにも覆われた裏、2=幾何的に露出した裏。
    pixels: Vec<u8>,
    covered_back_pixels: u64,
    geometric_back_pixels: u64,
}

struct DirectionRaster {
    image: VisualImage,
    exposure: ExposureImage,
}

#[derive(Debug, PartialEq, Eq)]
struct DirectionSummary {
    visible_faces: Vec<FaceId>,
    visible_back_faces: Vec<FaceId>,
    front_pixels: u64,
    back_pixels: u64,
    covered_back_pixels: u64,
    geometric_back_pixels: u64,
    owner_hash: u64,
    exposure_hash: u64,
}

#[derive(Debug)]
struct PathAggregate {
    order: [EdgeId; 3],
    directions: usize,
    front_pixels: u64,
    back_pixels: u64,
    covered_back_pixels: u64,
    geometric_back_pixels: u64,
    signature_hash: u64,
}

impl PathAggregate {
    fn new(order: [EdgeId; 3]) -> Self {
        Self {
            order,
            directions: 0,
            front_pixels: 0,
            back_pixels: 0,
            covered_back_pixels: 0,
            geometric_back_pixels: 0,
            signature_hash: FNV_OFFSET_BASIS,
        }
    }

    fn absorb(&mut self, azimuth_deg: i32, elevation_deg: i32, summary: &DirectionSummary) {
        self.directions += 1;
        self.front_pixels += summary.front_pixels;
        self.back_pixels += summary.back_pixels;
        self.covered_back_pixels += summary.covered_back_pixels;
        self.geometric_back_pixels += summary.geometric_back_pixels;
        hash_bytes(&mut self.signature_hash, &azimuth_deg.to_le_bytes());
        hash_bytes(&mut self.signature_hash, &elevation_deg.to_le_bytes());
        hash_bytes(&mut self.signature_hash, &summary.owner_hash.to_le_bytes());
        hash_bytes(
            &mut self.signature_hash,
            &summary.exposure_hash.to_le_bytes(),
        );
    }
}

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("crates/ori3-rigid/tests/fixtures/sa-warm-path.ori3")
}

fn fresh_store() -> Mutex<DocumentStore> {
    let store = Mutex::new(DocumentStore::default());
    store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .open(&fixture_path())
        .expect("リポジトリ内の sa-warm-path fixture を開けるはず");
    store
}

fn driver(hinge: EdgeId) -> Driver {
    Driver {
        hinge,
        target_angle_deg: match hinge {
            17 => -90.0,
            19 | 21 => 90.0,
            _ => panic!("検査対象でない hinge {hinge}"),
        },
    }
}

/// UI と同じく、現在の1本だけを hard、既に指定済みの希望値を preferred にする。
/// `warm_seed=None` なので、製品 command が保持する直前actualの経路もそのまま通る。
fn solve_ui_gesture(
    store: &Mutex<DocumentStore>,
    hard: Driver,
    preferred: Vec<Driver>,
) -> crate::commands::PoseOutcome {
    pose_solve_core(
        store,
        vec![hard],
        (!preferred.is_empty()).then_some(preferred),
        None,
        None,
        0,
        1.0,
    )
    .expect("sa の通常UI姿勢計算は有限の応答を返すはず")
}

/// pointer-up相当。明示seedを添えてもCanonical modeがそれを候補生成へ使わず、
/// 書類＋希望値から同じ形を再導出する契約を実solverと612方向で通す。
fn solve_canonical(
    store: &Mutex<DocumentStore>,
    mut desired: Vec<Driver>,
) -> crate::commands::PoseOutcome {
    desired.sort_unstable_by_key(|item| item.hinge);
    let seed = (17..=34)
        .map(|hinge| Driver {
            hinge,
            target_angle_deg: desired
                .iter()
                .find(|item| item.hinge == hinge)
                .map_or(0.0, |item| item.target_angle_deg),
        })
        .collect();
    pose_solve_core_with_mode(
        store,
        PoseSolveInput {
            hard: Vec::new(),
            preferred: Some(desired),
            soft: None,
            warm_seed: Some(seed),
            up_to: 0,
            t: 1.0,
            mode: PoseSolveMode::Canonical,
        },
    )
    .expect("sa のcanonical姿勢計算は有限の応答を返すはず")
}

fn solve_path(order: [EdgeId; 3]) -> SolvedPath {
    let store = fresh_store();
    let mut desired = Vec::<Driver>::new();
    for hinge in order {
        let hard = driver(hinge);
        let preferred = desired
            .iter()
            .filter(|item| item.hinge != hinge)
            .cloned()
            .collect();
        let _ = solve_ui_gesture(&store, hard.clone(), preferred);
        if let Some(existing) = desired.iter_mut().find(|item| item.hinge == hinge) {
            *existing = hard;
        } else {
            desired.push(hard);
            desired.sort_unstable_by_key(|item| item.hinge);
        }
        let _ = solve_canonical(&store, desired.clone());
    }

    let outcome = solve_canonical(&store, desired);
    let mut angles: Vec<_> = outcome
        .result
        .angles
        .iter()
        .map(|(&hinge, &angle)| (hinge, angle))
        .collect();
    angles.sort_unstable_by_key(|&(hinge, _)| hinge);
    SolvedPath {
        order,
        frame: outcome.result.frame,
        angles,
    }
}

fn sa_diagram() -> Diagram {
    let fixture: Document =
        serde_json::from_str(SA_WARM_PATH).expect("sa-warm-path fixture is a Document");
    let long = fixture.paper.width_mm.max(fixture.paper.height_mm);
    diagram(
        "sa-warm-path.ori3",
        fixture.cp,
        fixture.paper.width_mm / long,
        fixture.paper.height_mm / long,
    )
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn owner_hash(image: &VisualImage) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for pixel in &image.pixels {
        match pixel {
            None => hash_bytes(&mut hash, &[0]),
            Some(pixel) => {
                hash_bytes(&mut hash, &[1]);
                hash_bytes(&mut hash, &pixel.face.to_le_bytes());
                hash_bytes(&mut hash, &[u8::from(pixel.back_facing)]);
            }
        }
    }
    hash
}

fn exposure_hash(exposure: &ExposureImage) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    hash_bytes(&mut hash, &exposure.pixels);
    hash
}

fn exposure_image(
    diagram: &Diagram,
    frame: &Frame3D,
    view: Camera,
    image: &VisualImage,
) -> ExposureImage {
    let mut pixels = vec![0_u8; VIEWPORT * VIEWPORT];
    if image.light_pixels == 0 {
        return ExposureImage {
            pixels,
            covered_back_pixels: 0,
            geometric_back_pixels: 0,
        };
    }

    let mut front_coverage = vec![false; VIEWPORT * VIEWPORT];
    for face in render_faces(diagram, frame, VIEWPORT, view) {
        for triangle in face
            .triangles
            .iter()
            .filter(|triangle| !triangle.back_facing)
        {
            rasterize(
                |pixel, _depth| front_coverage[pixel] = true,
                triangle,
                VIEWPORT,
            );
        }
    }

    let mut covered_back_pixels = 0_u64;
    let mut geometric_back_pixels = 0_u64;
    for (pixel_index, owner) in image.pixels.iter().enumerate() {
        if !owner.is_some_and(|owner| owner.back_facing) {
            continue;
        }
        if front_coverage[pixel_index] {
            pixels[pixel_index] = 1;
            covered_back_pixels += 1;
        } else {
            pixels[pixel_index] = 2;
            geometric_back_pixels += 1;
        }
    }
    assert_eq!(
        covered_back_pixels + geometric_back_pixels,
        image.light_pixels,
        "全裏画素を covered/geometric のどちらかへ分類する"
    );
    ExposureImage {
        pixels,
        covered_back_pixels,
        geometric_back_pixels,
    }
}

fn direction_raster(
    diagram: &Diagram,
    frame: &Frame3D,
    azimuth_deg: i32,
    elevation_deg: i32,
) -> DirectionRaster {
    let view = camera_from_orbit_angles(
        diagram.paper_width,
        diagram.paper_height,
        azimuth_deg,
        elevation_deg,
    );
    let image = visual_image(diagram, frame, VIEWPORT, view);
    let (front, back) = classified_fill_counts(&image);
    assert_eq!((front, back), (image.red_pixels, image.light_pixels));
    let exposure = exposure_image(diagram, frame, view, &image);
    DirectionRaster { image, exposure }
}

fn direction_summary(raster: &DirectionRaster) -> DirectionSummary {
    DirectionSummary {
        visible_faces: raster.image.visible_faces.iter().copied().collect(),
        visible_back_faces: raster.image.visible_back_faces.iter().copied().collect(),
        front_pixels: raster.image.red_pixels,
        back_pixels: raster.image.light_pixels,
        covered_back_pixels: raster.exposure.covered_back_pixels,
        geometric_back_pixels: raster.exposure.geometric_back_pixels,
        owner_hash: owner_hash(&raster.image),
        exposure_hash: exposure_hash(&raster.exposure),
    }
}

fn first_owner_difference(
    expected: &VisualImage,
    actual: &VisualImage,
) -> Option<(usize, usize, Option<VisualPixel>, Option<VisualPixel>)> {
    expected
        .pixels
        .iter()
        .zip(&actual.pixels)
        .enumerate()
        .find_map(|(index, (&before, &after))| {
            (before != after).then_some((index % VIEWPORT, index / VIEWPORT, before, after))
        })
}

fn first_exposure_difference(
    expected: &ExposureImage,
    actual: &ExposureImage,
) -> Option<(usize, usize, u8, u8)> {
    expected
        .pixels
        .iter()
        .zip(&actual.pixels)
        .enumerate()
        .find_map(|(index, (&before, &after))| {
            (before != after).then_some((index % VIEWPORT, index / VIEWPORT, before, after))
        })
}

fn assert_same_direction(
    expected_path: &SolvedPath,
    expected: &DirectionRaster,
    actual_path: &SolvedPath,
    actual: &DirectionRaster,
    azimuth_deg: i32,
    elevation_deg: i32,
) {
    if expected.image == actual.image && expected.exposure == actual.exposure {
        return;
    }
    panic!(
        "同じ希望角でも612方向owner/露出が操作経路で変わった: azimuth={azimuth_deg} elevation={elevation_deg}\nexpected_order={:?} actual_order={:?}\nexpected_angles={:?}\nactual_angles={:?}\nexpected_summary={:?}\nactual_summary={:?}\nvisual_difference={:?}\nfirst_owner_difference={:?}\nfirst_exposure_difference={:?}",
        expected_path.order,
        actual_path.order,
        expected_path.angles,
        actual_path.angles,
        direction_summary(expected),
        direction_summary(actual),
        expected.image.difference(&actual.image, VIEWPORT),
        first_owner_difference(&expected.image, &actual.image),
        first_exposure_difference(&expected.exposure, &actual.exposure),
    );
}

#[test]
fn sa_real_solver_six_ui_paths_have_the_same_canonical_612_owner_exposure_signature() {
    let diagram = sa_diagram();
    let solved: Vec<_> = ALL_UI_PATHS.into_iter().map(solve_path).collect();
    assert_eq!(solved.len(), 6);
    assert!(
        solved
            .iter()
            .all(|path| path.frame.faces.len() == diagram.faces.len()),
        "全solver frameがfixtureの全faceを含む"
    );

    let mut aggregates: Vec<_> = solved
        .iter()
        .map(|path| PathAggregate::new(path.order))
        .collect();
    assert_eq!(
        aggregates.iter().map(|item| item.order).collect::<Vec<_>>(),
        ALL_UI_PATHS.to_vec(),
        "集計順も6つのUI経路順と一致する"
    );
    let mut measured_directions = 0_usize;
    for elevation_deg in (-80_i32..=80).step_by(10) {
        for azimuth_deg in (0_i32..=350).step_by(10) {
            let expected = direction_raster(&diagram, &solved[0].frame, azimuth_deg, elevation_deg);
            aggregates[0].absorb(azimuth_deg, elevation_deg, &direction_summary(&expected));
            for path_index in 1..solved.len() {
                let actual = direction_raster(
                    &diagram,
                    &solved[path_index].frame,
                    azimuth_deg,
                    elevation_deg,
                );
                aggregates[path_index].absorb(
                    azimuth_deg,
                    elevation_deg,
                    &direction_summary(&actual),
                );
                assert_same_direction(
                    &solved[0],
                    &expected,
                    &solved[path_index],
                    &actual,
                    azimuth_deg,
                    elevation_deg,
                );
            }
            measured_directions += 1;
        }
    }

    assert_eq!(measured_directions, 36 * 17);
    assert!(aggregates.iter().all(|item| item.directions == 36 * 17));
    println!("SA_REAL_SOLVER_612_CANONICAL aggregates={aggregates:#?}");
}
