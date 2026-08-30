use super::*;

pub(super) fn vertex(id: u32, x: f64, y: f64) -> Vertex {
    Vertex { id, pos: [x, y] }
}

pub(super) fn edge(id: u32, v0: u32, v1: u32, kind: EdgeKind) -> Edge {
    Edge { id, v0, v1, kind }
}

pub(super) fn split_square() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 0.5, 0.0),
            vertex(2, 1.0, 0.0),
            vertex(3, 1.0, 1.0),
            vertex(4, 0.5, 1.0),
            vertex(5, 0.0, 1.0),
        ],
        edges: vec![
            edge(0, 0, 1, EdgeKind::Border),
            edge(1, 1, 2, EdgeKind::Border),
            edge(2, 2, 3, EdgeKind::Border),
            edge(3, 3, 4, EdgeKind::Border),
            edge(4, 4, 5, EdgeKind::Border),
            edge(5, 5, 0, EdgeKind::Border),
            edge(6, 1, 4, EdgeKind::Mountain),
        ],
        next_vertex_id: 6,
        next_edge_id: 7,
    }
}

pub(super) fn flat_foldable_kome() -> CreasePattern {
    let radial_kinds = [
        EdgeKind::Valley,
        EdgeKind::Valley,
        EdgeKind::Valley,
        EdgeKind::Mountain,
        EdgeKind::Valley,
        EdgeKind::Mountain,
        EdgeKind::Valley,
        EdgeKind::Mountain,
    ];
    let mut edges = vec![
        edge(0, 0, 1, EdgeKind::Border),
        edge(1, 1, 2, EdgeKind::Border),
        edge(2, 2, 3, EdgeKind::Border),
        edge(3, 3, 4, EdgeKind::Border),
        edge(4, 4, 5, EdgeKind::Border),
        edge(5, 5, 6, EdgeKind::Border),
        edge(6, 6, 7, EdgeKind::Border),
        edge(7, 7, 0, EdgeKind::Border),
    ];
    edges.extend(
        radial_kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| edge(8 + index as u32, 8, index as u32, kind)),
    );
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 0.5, 0.0),
            vertex(2, 1.0, 0.0),
            vertex(3, 1.0, 0.5),
            vertex(4, 1.0, 1.0),
            vertex(5, 0.5, 1.0),
            vertex(6, 0.0, 1.0),
            vertex(7, 0.0, 0.5),
            vertex(8, 0.5, 0.5),
        ],
        edges,
        next_vertex_id: 9,
        next_edge_id: 16,
    }
}

pub(super) fn angle_surface_cp() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 1.0, 0.0),
            vertex(2, 1.0, 1.0),
            vertex(3, 0.0, 1.0),
            vertex(4, 0.0, 0.5),
            vertex(5, 1.0, 0.5),
            vertex(6, 0.5, 1.0),
            vertex(7, 0.5, 0.0),
            vertex(8, 0.5, 0.5),
            vertex(9, 0.792_893_218_813_452_5, 0.5),
            vertex(10, 0.5, 0.207_106_781_186_547_52),
            vertex(11, 0.25, 0.5),
            vertex(12, 0.5, 0.792_893_218_813_452_5),
            vertex(13, 0.207_106_781_186_547_52, 0.5),
        ],
        edges: vec![
            edge(4, 3, 4, EdgeKind::Border),
            edge(5, 4, 0, EdgeKind::Border),
            edge(6, 1, 5, EdgeKind::Border),
            edge(7, 5, 2, EdgeKind::Border),
            edge(9, 2, 6, EdgeKind::Border),
            edge(10, 6, 3, EdgeKind::Border),
            edge(11, 0, 7, EdgeKind::Border),
            edge(12, 7, 1, EdgeKind::Border),
            edge(17, 0, 8, EdgeKind::Valley),
            edge(18, 8, 2, EdgeKind::Valley),
            edge(19, 8, 9, EdgeKind::Mountain),
            edge(20, 9, 5, EdgeKind::Mountain),
            edge(21, 2, 9, EdgeKind::Mountain),
            edge(22, 9, 1, EdgeKind::Mountain),
            edge(23, 8, 10, EdgeKind::Mountain),
            edge(24, 10, 7, EdgeKind::Mountain),
            edge(25, 0, 10, EdgeKind::Mountain),
            edge(26, 10, 1, EdgeKind::Mountain),
            edge(28, 11, 8, EdgeKind::Mountain),
            edge(31, 6, 12, EdgeKind::Mountain),
            edge(32, 12, 8, EdgeKind::Mountain),
            edge(33, 2, 12, EdgeKind::Mountain),
            edge(34, 12, 3, EdgeKind::Mountain),
            edge(37, 4, 13, EdgeKind::Mountain),
            edge(38, 13, 11, EdgeKind::Mountain),
            edge(39, 0, 13, EdgeKind::Mountain),
            edge(40, 13, 3, EdgeKind::Mountain),
            edge(42, 13, 12, EdgeKind::Valley),
            edge(43, 10, 9, EdgeKind::Valley),
        ],
        next_vertex_id: 14,
        next_edge_id: 44,
    }
}

pub(super) fn angle_surface_angles(edge_43: f64) -> HashMap<EdgeId, f64> {
    HashMap::from([
        (17, -180.0),
        (18, -180.0),
        (20, -95.0),
        (21, 180.0),
        (22, 180.0),
        (23, 180.0),
        (24, -95.0),
        (25, 180.0),
        (31, -123.0),
        (32, 180.0),
        (34, 180.0),
        (37, -123.0),
        (39, 180.0),
        (40, 180.0),
        (42, -57.0),
        (43, edge_43),
    ])
}

/// `verification/angles.json` の展開図を丸めず、追跡対象の検査コード内へ写したもの。
pub(super) fn zero_back_user_cp() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 1.0, 0.0),
            vertex(2, 1.0, 1.0),
            vertex(3, 0.0, 1.0),
            vertex(4, 0.0, 0.5),
            vertex(5, 1.0, 0.5),
            vertex(6, 0.5, 1.0),
            vertex(7, 0.5, 0.0),
            vertex(8, 0.5, 0.5),
            vertex(9, 0.792_893_218_813_452_5, 0.5),
            vertex(10, 0.5, 0.207_106_781_186_547_52),
            vertex(11, 0.207_106_781_186_547_52, 0.5),
            vertex(12, 0.5, 0.792_893_218_813_452_5),
        ],
        edges: vec![
            edge(4, 3, 4, EdgeKind::Border),
            edge(5, 4, 0, EdgeKind::Border),
            edge(6, 1, 5, EdgeKind::Border),
            edge(7, 5, 2, EdgeKind::Border),
            edge(9, 2, 6, EdgeKind::Border),
            edge(10, 6, 3, EdgeKind::Border),
            edge(11, 0, 7, EdgeKind::Border),
            edge(12, 7, 1, EdgeKind::Border),
            edge(17, 0, 8, EdgeKind::Valley),
            edge(18, 8, 2, EdgeKind::Valley),
            edge(19, 8, 9, EdgeKind::Mountain),
            edge(20, 9, 5, EdgeKind::Mountain),
            edge(21, 2, 9, EdgeKind::Mountain),
            edge(22, 9, 1, EdgeKind::Mountain),
            edge(23, 8, 10, EdgeKind::Mountain),
            edge(24, 10, 7, EdgeKind::Mountain),
            edge(25, 0, 10, EdgeKind::Mountain),
            edge(26, 10, 1, EdgeKind::Mountain),
            edge(27, 4, 11, EdgeKind::Mountain),
            edge(28, 11, 8, EdgeKind::Mountain),
            edge(29, 0, 11, EdgeKind::Mountain),
            edge(30, 11, 3, EdgeKind::Mountain),
            edge(31, 6, 12, EdgeKind::Mountain),
            edge(32, 12, 8, EdgeKind::Mountain),
            edge(33, 2, 12, EdgeKind::Mountain),
            edge(34, 12, 3, EdgeKind::Mountain),
            edge(35, 11, 12, EdgeKind::Valley),
            edge(36, 9, 10, EdgeKind::Valley),
        ],
        next_vertex_id: 13,
        next_edge_id: 37,
    }
}

/// `verification/angles.json` の20角度。JSONのf64値をそのまま使い、丸めない。
pub(super) fn zero_back_user_angles() -> HashMap<EdgeId, f64> {
    HashMap::from([
        (17, -179.999_999_999_999_97),
        (18, -179.999_999_999_999_97),
        (19, 180.0),
        (20, -92.999_999_999_999_99),
        (21, 180.0),
        (22, 180.0),
        (23, 180.0),
        (24, -92.999_999_999_999_99),
        (25, 180.0),
        (26, 180.0),
        (27, -178.693_545_755_435_96),
        (28, 180.0),
        (29, 180.0),
        (30, 180.0),
        (31, -178.693_545_755_435_96),
        (32, 179.999_999_999_999_97),
        (33, 180.0),
        (34, 180.0),
        (35, -1.306_454_244_564_035_5),
        (36, -87.0),
    ])
}

/// `verification/angles-now.json` の20角度。JSONのf64値をそのまま使い、丸めない。
/// `zero_back_user_angles` との違いは辺20/24/27/28/31/32/35/36の8本。
pub(super) fn zero_back_user_angles_now() -> HashMap<EdgeId, f64> {
    HashMap::from([
        (17, -179.999_999_999_999_97),
        (18, -179.999_999_999_999_97),
        (19, 180.0),
        (20, -97.999_999_999_999_97),
        (21, 180.0),
        (22, 180.0),
        (23, 180.0),
        (24, -98.000_000_000_000_01),
        (25, 180.0),
        (26, 180.0),
        (27, -179.128_990_221_440_23),
        (28, 179.999_999_999_999_97),
        (29, 180.0),
        (30, 180.0),
        (31, -179.128_990_221_440_23),
        (32, 180.0),
        (33, 180.0),
        (34, 180.0),
        (35, -0.871_009_778_559_739_8),
        (36, -82.0),
    ])
}

/// 手順0件のFrameへ、既定オンの重なり補正をproductionと同じ順で適用する。
/// 動作中の `desktop.exe` から読み取った展開図(頂点13・辺28)。
/// `zero_back_user_cp` と同じ正方形だが、辺と頂点の番号は実機のものをそのまま使う。
/// `.gitignore` 対象の `verification/` を検査から読まないため値を埋め込んでいる
/// (CLAUDE.md §10.1)。
pub(super) fn live_frame_cp() -> CreasePattern {
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

/// 実機の `poseAngles` 20本をそのまま写した。f64の値は丸めない。
/// このうち ±180°(誤差 `1e-6` 以内)は15本ある。
pub(super) fn live_frame_angles() -> HashMap<EdgeId, f64> {
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

/// 実機が表示していた形を、productionと同じ順で組み立てる。
pub(super) fn live_frame_frame(cp: &CreasePattern, faces: &[Face]) -> Frame3D {
    let folded = propagate(cp, faces, &live_frame_angles());
    zero_back_apply_overlap(cp, faces, to_frame3d(cp, faces, &folded))
}

pub(super) fn zero_back_apply_overlap(
    cp: &CreasePattern,
    faces: &[Face],
    mut frame: Frame3D,
) -> Frame3D {
    let order = crate::store::frame_surface_rank_order(&frame)
        .expect("the zero-back frame has a complete unique surface order");
    ori3_soft::prevent_overlap_with_order_authority(
        cp,
        faces,
        &mut frame,
        ori3_soft::OverlapOrderInput {
            start: &order,
            end: &order,
            progress: 0.5,
            authoritative: true,
        },
        &ori3_soft::OverlapSettings {
            enabled: true,
            ..Default::default()
        },
    );
    frame
}

/// 手順0件で保持されている全ヒンジ角から表示Frameを再構成し、既定オンの
/// 重なり補正までproductionと同じ順で適用する。
pub(super) fn zero_back_user_frame(cp: &CreasePattern, faces: &[Face]) -> Frame3D {
    let folded = propagate(cp, faces, &zero_back_user_angles());
    zero_back_apply_overlap(cp, faces, to_frame3d(cp, faces, &folded))
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ZeroBackWarmPath {
    /// 選択中の辺36をhard、残る19角をpreferredとして同時に最終値へ近づける。
    Active36WithPreferred,
    /// 利用者の操作仮説どおり、辺36だけをhardにして残る19本を自由追従させる。
    Edge36Only,
    /// 全20角をhardにする対照。自由変数が無いためwarm startで枝を選べない。
    AllHardControl,
}

impl ZeroBackWarmPath {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Active36WithPreferred => "edge36-hard-plus-19-preferred",
            Self::Edge36Only => "edge36-hard-only",
            Self::AllHardControl => "all-20-hard-control",
        }
    }
}

#[derive(Debug)]
pub(super) struct ZeroBackWarmResult {
    pub(super) frame: Frame3D,
    pub(super) angles: HashMap<EdgeId, f64>,
    pub(super) converged: bool,
    pub(super) closure_rms: f64,
    pub(super) iterations: u32,
    pub(super) contact_detected: bool,
    pub(super) self_intersects: bool,
    pub(super) max_seam_gap: f64,
}

pub(super) fn zero_back_warm_solve(
    cp: &CreasePattern,
    faces: &[Face],
    path: ZeroBackWarmPath,
    stages: usize,
) -> ZeroBackWarmResult {
    zero_back_warm_solve_to(cp, faces, &zero_back_user_angles(), path, stages)
}

pub(super) fn zero_back_warm_solve_to(
    cp: &CreasePattern,
    faces: &[Face],
    final_angles: &HashMap<EdgeId, f64>,
    path: ZeroBackWarmPath,
    stages: usize,
) -> ZeroBackWarmResult {
    assert!(stages > 0);
    let final_angles = final_angles.clone();
    let mut warm = final_angles
        .keys()
        .copied()
        .map(|hinge| (hinge, 0.0))
        .collect::<HashMap<_, _>>();
    let mut last = None;

    for stage in 1..=stages {
        let progress = stage as f64 / stages as f64;
        let scaled = |hinge: EdgeId| final_angles[&hinge] * progress;
        let (hard, preferred) = match path {
            ZeroBackWarmPath::Active36WithPreferred => (
                vec![Driver {
                    hinge: 36,
                    target_angle_deg: scaled(36),
                }],
                Some(
                    final_angles
                        .keys()
                        .copied()
                        .filter(|&hinge| hinge != 36)
                        .map(|hinge| (hinge, scaled(hinge)))
                        .collect::<HashMap<_, _>>(),
                ),
            ),
            ZeroBackWarmPath::Edge36Only => (
                vec![Driver {
                    hinge: 36,
                    target_angle_deg: scaled(36),
                }],
                None,
            ),
            ZeroBackWarmPath::AllHardControl => {
                let mut hard = final_angles
                    .keys()
                    .copied()
                    .map(|hinge| Driver {
                        hinge,
                        target_angle_deg: scaled(hinge),
                    })
                    .collect::<Vec<_>>();
                hard.sort_unstable_by_key(|driver| driver.hinge);
                (hard, None)
            }
        };
        let motion = solve_motion(cp, faces, &hard, preferred.as_ref(), Some(&warm), true);
        assert!(
            motion
                .result
                .frame
                .faces
                .iter()
                .flat_map(|face| &face.polygon)
                .flatten()
                .all(|coordinate| coordinate.is_finite()),
            "{} with {stages} external stages returned non-finite geometry at stage {stage}",
            path.label(),
        );
        assert_eq!(motion.result.frame.faces.len(), faces.len());
        assert_eq!(motion.result.angles.len(), final_angles.len());
        warm = motion.result.angles.clone();
        last = Some(motion);
    }

    let motion = last.expect("at least one external warm-start stage ran");
    let self_intersects = ori3_rigid::self_intersects(&motion.result.frame);
    let max_seam_gap = ori3_rigid::max_seam_gap(cp, faces, &motion.result.frame);
    ZeroBackWarmResult {
        frame: zero_back_apply_overlap(cp, faces, motion.result.frame),
        angles: motion.result.angles,
        converged: motion.result.converged,
        closure_rms: motion.result.closure_rms,
        iterations: motion.result.iterations,
        contact_detected: motion.contact_detected,
        self_intersects,
        max_seam_gap,
    }
}

pub(super) fn fold_hinges(cp: &CreasePattern, faces: &[Face]) -> Vec<(EdgeId, EdgeKind)> {
    let mut owners = BTreeMap::<EdgeId, usize>::new();
    for face in faces {
        for &edge_id in &face.edges {
            *owners.entry(edge_id).or_default() += 1;
        }
    }
    cp.edges
        .iter()
        .filter(|item| {
            matches!(item.kind, EdgeKind::Mountain | EdgeKind::Valley)
                && owners.get(&item.id) == Some(&2)
        })
        .map(|item| (item.id, item.kind))
        .collect()
}

pub(super) fn diagram(
    name: &'static str,
    cp: CreasePattern,
    paper_width: f64,
    paper_height: f64,
) -> Diagram {
    let faces = extract_faces(&cp);
    let hinges = fold_hinges(&cp, &faces);
    let triangles = triangulations(&cp, &faces);
    let mut face_ids = faces.iter().map(|face| face.id).collect::<Vec<_>>();
    face_ids.sort_unstable();
    let owner_codes = face_ids
        .into_iter()
        .enumerate()
        .map(|(index, face)| (face, index as i64 + 1))
        .collect();
    Diagram {
        name,
        cp,
        faces,
        hinges,
        paper_width,
        paper_height,
        triangles,
        owner_codes,
    }
}

pub(super) fn boundary_diagrams() -> Vec<Diagram> {
    let fixture: Document =
        serde_json::from_str(FOLDED_SAMPLE).expect("folded-sample fixture is a Document");
    let long = fixture.paper.width_mm.max(fixture.paper.height_mm);
    let paper_width = fixture.paper.width_mm / long;
    let paper_height = fixture.paper.height_mm / long;
    let diagrams = vec![
        diagram("split-square", split_square(), 1.0, 1.0),
        diagram("diagonal-midline-square", flat_foldable_kome(), 1.0, 1.0),
        diagram("folded-sample.ori3", fixture.cp, paper_width, paper_height),
    ];
    assert_eq!(diagrams[0].hinges.len(), 1);
    assert_eq!(diagrams[1].hinges.len(), 8);
    assert_eq!(diagrams[2].hinges.len(), 101);
    diagrams
}
