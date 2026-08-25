use super::*;

// ===========================================================================
// 「上に来るべき面」を、アプリの重なり順とは独立に幾何から求めて突き合わせる。
//
// ここでは `surface_rank` を一切読まない。紙の位置と折り目のつながりだけから
// 「完全に重なった2枚のうち、どちらが上か」を決め、アプリが使っている順と比べる。
// ===========================================================================

/// 重なりを調べるための、いまの姿勢での面の平面。
pub(super) struct OverlapPlane {
    pub(super) origin: V3,
    pub(super) normal: V3,
    pub(super) u: V3,
    pub(super) v: V3,
}

pub(super) fn polygon_normal3(points: &[V3]) -> Option<V3> {
    if points.len() < 3 {
        return None;
    }
    let mut normal = V3::ZERO;
    for index in 0..points.len() {
        normal += points[index].cross(points[(index + 1) % points.len()]);
    }
    (normal.length_squared() > 1e-24).then(|| normal.normalize())
}

/// 法線の向きの符号を消す。上下の比較を面の巻き方向に依存させないため。
///
/// `ori3-rigid` の `surface_order::canonical` と、画面側の
/// `surfaceOwner.ts::canonicalize` と同じ式。重なり順の「上」がどちらを指すかは
/// この向きそのものなので、測定側もそろえないと比較できない。
pub(super) fn canonical3(mut normal: V3) -> V3 {
    let absolute = V3::new(normal.x.abs(), normal.y.abs(), normal.z.abs());
    let component = if absolute.x >= absolute.y && absolute.x >= absolute.z {
        normal.x
    } else if absolute.y >= absolute.z {
        normal.y
    } else {
        normal.z
    };
    if component < 0.0 {
        normal = -normal;
    }
    normal
}

pub(super) fn overlap_plane(points: &[V3]) -> Option<OverlapPlane> {
    let origin = *points.first()?;
    let normal = canonical3(polygon_normal3(points)?);
    let u = (0..points.len())
        .map(|index| points[(index + 1) % points.len()] - points[index])
        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))?
        .normalize();
    let v = normal.cross(u).normalize();
    Some(OverlapPlane {
        origin,
        normal,
        u,
        v,
    })
}

pub(super) fn project2(points: &[V3], plane: &OverlapPlane) -> Vec<[f64; 2]> {
    points
        .iter()
        .map(|point| {
            let relative = *point - plane.origin;
            [relative.dot(plane.u), relative.dot(plane.v)]
        })
        .collect()
}

pub(super) fn polygon_area2(polygon: &[[f64; 2]]) -> f64 {
    if polygon.len() < 3 {
        return 0.0;
    }
    0.5 * (0..polygon.len())
        .map(|index| {
            let a = polygon[index];
            let b = polygon[(index + 1) % polygon.len()];
            a[0] * b[1] - a[1] * b[0]
        })
        .sum::<f64>()
}

/// 凸多角形どうしのクリップ。三角形どうしにだけ使う。
pub(super) fn clip_convex(subject: &[[f64; 2]], clip: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut output = subject.to_vec();
    let side = |start: [f64; 2], end: [f64; 2], point: [f64; 2]| {
        (end[0] - start[0]) * (point[1] - start[1]) - (end[1] - start[1]) * (point[0] - start[0])
    };
    let winding = if polygon_area2(clip) < 0.0 { -1.0 } else { 1.0 };
    for index in 0..clip.len() {
        let start = clip[index];
        let end = clip[(index + 1) % clip.len()];
        let input = std::mem::take(&mut output);
        let Some(mut previous) = input.last().copied() else {
            break;
        };
        let mut previous_side = side(start, end, previous) * winding;
        for current in input {
            let current_side = side(start, end, current) * winding;
            if (previous_side >= 0.0) != (current_side >= 0.0) {
                let denominator = previous_side - current_side;
                if denominator.abs() > 1e-18 {
                    let ratio = previous_side / denominator;
                    output.push([
                        previous[0] + (current[0] - previous[0]) * ratio,
                        previous[1] + (current[1] - previous[1]) * ratio,
                    ]);
                }
            }
            if current_side >= 0.0 {
                output.push(current);
            }
            previous = current;
            previous_side = current_side;
        }
    }
    output
}

/// 2枚の面が実面積で重なる代表点(共通平面上の2D座標)と重なり面積を返す。
pub(super) fn overlap_witness(
    left: &[[f64; 2]],
    left_triangles: &[[usize; 3]],
    right: &[[f64; 2]],
    right_triangles: &[[usize; 3]],
) -> Option<([f64; 2], f64)> {
    let mut best: Option<([f64; 2], f64)> = None;
    for left_indices in left_triangles {
        let left_triangle = left_indices.map(|index| left[index]);
        for right_indices in right_triangles {
            let right_triangle = right_indices.map(|index| right[index]);
            let intersection = clip_convex(&left_triangle, &right_triangle);
            let area = polygon_area2(&intersection).abs();
            if area <= 1e-12 || intersection.len() < 3 {
                continue;
            }
            let sum = intersection.iter().fold([0.0, 0.0], |sum, point| {
                [sum[0] + point[0], sum[1] + point[1]]
            });
            let count = intersection.len() as f64;
            let center = [sum[0] / count, sum[1] / count];
            if best.is_none_or(|(_, best_area)| area > best_area) {
                best = Some((center, area));
            }
        }
    }
    best
}

/// `exact` の面上の点を材質座標へ戻し、`probe` の同じ面へ写す。
pub(super) fn map_material_point(exact: &[V3], probe: &[V3], point: V3) -> Option<V3> {
    if exact.len() != probe.len() || exact.len() < 3 {
        return None;
    }
    let origin = exact[0];
    let (first, second) = (1..exact.len()).find_map(|first| {
        (first + 1..exact.len())
            .find(|&second| {
                (exact[first] - origin)
                    .cross(exact[second] - origin)
                    .length_squared()
                    > 1e-24
            })
            .map(|second| (first, second))
    })?;
    let a = exact[first] - origin;
    let b = exact[second] - origin;
    let relative = point - origin;
    let aa = a.length_squared();
    let ab = a.dot(b);
    let bb = b.length_squared();
    let determinant = aa * bb - ab * ab;
    if determinant.abs() <= 1e-24 {
        return None;
    }
    let ra = relative.dot(a);
    let rb = relative.dot(b);
    let first_weight = (ra * bb - rb * ab) / determinant;
    let second_weight = (rb * aa - ra * ab) / determinant;
    let probe_origin = probe[0];
    Some(
        probe_origin
            + (probe[first] - probe_origin) * first_weight
            + (probe[second] - probe_origin) * second_weight,
    )
}

pub(super) fn frame_polygons(frame: &Frame3D) -> BTreeMap<FaceId, Vec<V3>> {
    frame
        .faces
        .iter()
        .map(|face| {
            (
                face.face,
                face.polygon
                    .iter()
                    .map(|point| V3::new(point[0], point[1], point[2]))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

/// いまの姿勢で「同じ平面に乗っていて、実面積で重なっている」面対。
/// ここが、カメラからの深度が並んで重なり順が表示を決める場所である。
pub(super) struct CoincidentOverlap {
    pub(super) left: FaceId,
    pub(super) right: FaceId,
    /// 共通平面の正準法線。この向きに高いほうが「上」。
    pub(super) normal: V3,
    /// 重なり領域の代表点(3D、いまの姿勢)。
    pub(super) witness: V3,
    /// 2面が直接つながっている折り目(あれば)。
    pub(super) shared_hinge: Option<EdgeId>,
}

/// `surface_rank` を一切読まずに、完全に重なっている面対を全て挙げる。
pub(super) fn coincident_overlaps(diagram: &Diagram, frame: &Frame3D) -> Vec<CoincidentOverlap> {
    let polygons = frame_polygons(frame);
    let mut face_ids = polygons.keys().copied().collect::<Vec<_>>();
    face_ids.sort_unstable();
    let edges_of = diagram
        .faces
        .iter()
        .map(|face| (face.id, face.edges.iter().copied().collect::<BTreeSet<_>>()))
        .collect::<BTreeMap<_, _>>();
    let mut overlaps = Vec::new();
    for (left_index, &left) in face_ids.iter().enumerate() {
        for &right in &face_ids[left_index + 1..] {
            let left_points = &polygons[&left];
            let right_points = &polygons[&right];
            let (Some(plane), Some(right_normal)) = (
                overlap_plane(left_points),
                polygon_normal3(right_points).map(canonical3),
            ) else {
                continue;
            };
            // 法線が平行で、かつ両面の全頂点が同じ平面に乗っているときだけ。
            if plane.normal.dot(right_normal).abs() < 1.0 - 1e-9 {
                continue;
            }
            let coincident = right_points
                .iter()
                .chain(left_points)
                .all(|point| plane.normal.dot(*point - plane.origin).abs() <= 1e-9);
            if !coincident {
                continue;
            }
            let left_2d = project2(left_points, &plane);
            let right_2d = project2(right_points, &plane);
            let left_triangles = triangulate_polygon(&left_2d);
            let right_triangles = triangulate_polygon(&right_2d);
            let Some((witness, _)) =
                overlap_witness(&left_2d, &left_triangles, &right_2d, &right_triangles)
            else {
                continue;
            };
            let shared_hinge = edges_of[&left]
                .intersection(&edges_of[&right])
                .copied()
                .find(|edge| diagram.hinges.iter().any(|(hinge, _)| hinge == edge));
            overlaps.push(CoincidentOverlap {
                left,
                right,
                normal: plane.normal,
                witness: plane.origin + plane.u * witness[0] + plane.v * witness[1],
                shared_hinge,
            });
        }
    }
    overlaps
}

/// 重なっている2面の、指定姿勢での高さの差(左 − 右)。決められないとき `None`。
pub(super) fn probe_height_difference(
    exact: &BTreeMap<FaceId, Vec<V3>>,
    probe: &BTreeMap<FaceId, Vec<V3>>,
    overlap: &CoincidentOverlap,
) -> Option<f64> {
    let left = map_material_point(
        exact.get(&overlap.left)?,
        probe.get(&overlap.left)?,
        overlap.witness,
    )?;
    let right = map_material_point(
        exact.get(&overlap.right)?,
        probe.get(&overlap.right)?,
        overlap.witness,
    )?;
    Some(left.dot(overlap.normal) - right.dot(overlap.normal))
}

/// 紙が裂けたとみなす辺の離れ。製品側 `motion.rs` と同じ値。
pub(super) const SEAM_TEAR_TOLERANCE: f64 = 1e-6;

/// 折り目1本だけを少し戻したときに、どちらが上へ出るかで決める判定。
///
/// 2面が1本の折り目でつながっているとき、その折り目の角度を少し戻すと、
/// 紙はすり抜けられないので必ず片方が上へ出る。他の折り目は一切動かさないので、
/// 経路の刻み方にも22点のcheckpointにも依存しない。
///
/// ただし折り目1本だけを戻すと紙が裂ける展開図もある。裂けた形は実際には
/// 作れないので、裂けの量が `1e-6` 以上になった探りは答えとして採らない。
pub(super) fn local_hinge_truth(
    cp: &CreasePattern,
    faces: &[Face],
    angles: &HashMap<EdgeId, f64>,
    exact: &BTreeMap<FaceId, Vec<V3>>,
    overlap: &CoincidentOverlap,
) -> Option<bool> {
    let hinge = overlap.shared_hinge?;
    let angle = angles.get(&hinge).copied()?;
    if angle == 0.0 {
        return None;
    }
    for relief in [1e-7_f64, 1e-6, 1e-5, 1e-4, 1e-3, 1e-2, 1e-1, 1.0, 5.0] {
        let relaxed = angle.abs() - relief;
        if relaxed <= 0.0 {
            break;
        }
        let mut probe_angles = angles.clone();
        probe_angles.insert(hinge, angle.signum() * relaxed);
        let probe_frame3d = to_frame3d(cp, faces, &propagate(cp, faces, &probe_angles));
        if ori3_rigid::max_seam_gap(cp, faces, &probe_frame3d) >= SEAM_TEAR_TOLERANCE {
            return None;
        }
        let probe = frame_polygons(&probe_frame3d);
        let Some(difference) = probe_height_difference(exact, &probe, overlap) else {
            continue;
        };
        if difference.abs() > 1e-12 {
            return Some(difference > 0.0);
        }
    }
    None
}

/// いまの姿勢から平らな状態へ向けて、全ての折り目を同じ割合で戻しながら
/// 実際に解いた姿勢の梯子。裂けた段は捨てるので、残るのは実際に作れる形だけ。
///
/// 製品側は「全ヒンジを共通の角度で頭打ちにする」経路を使う。ここは
/// 「全ヒンジを共通の割合で縮める」経路で、刻み方が違う独立の道である。
pub(super) fn solved_unfold_ladder(
    cp: &CreasePattern,
    faces: &[Face],
    angles: &HashMap<EdgeId, f64>,
) -> Vec<(f64, BTreeMap<FaceId, Vec<V3>>)> {
    const SCALES: [f64; 12] = [
        1.0 - 1e-9,
        1.0 - 1e-8,
        1.0 - 1e-7,
        1.0 - 1e-6,
        1.0 - 1e-5,
        1.0 - 1e-4,
        1.0 - 1e-3,
        0.99,
        0.95,
        0.9,
        0.8,
        0.5,
    ];
    let mut warm = angles.clone();
    let mut ladder = Vec::with_capacity(SCALES.len());
    for scale in SCALES {
        let mut hard = angles
            .iter()
            .map(|(&hinge, &angle)| Driver {
                hinge,
                target_angle_deg: angle * scale,
            })
            .collect::<Vec<_>>();
        hard.sort_unstable_by_key(|driver| driver.hinge);
        // 重なり順は刻印させない。独立検証がアプリの重なり順に触れないため。
        let motion = solve_motion(cp, faces, &hard, None, Some(&warm), false);
        let seam = ori3_rigid::max_seam_gap(cp, faces, &motion.result.frame);
        let torn = seam >= SEAM_TEAR_TOLERANCE;
        let finite = motion
            .result
            .frame
            .faces
            .iter()
            .flat_map(|face| &face.polygon)
            .flatten()
            .all(|coordinate| coordinate.is_finite());
        warm = motion.result.angles.clone();
        if !torn && finite {
            ladder.push((seam, frame_polygons(&motion.result.frame)));
        }
    }
    ladder
}

/// 梯子を「いまの姿勢に近い段」から順に見て、最初に離れた段の上下で決める。
///
/// 返すのは (上下, 高さの差, その段の裂けの量)。**高さの差がその段の裂けの量より
/// 小さい場合、その答えは信用できない。** 面が離れている量より、紙がちぎれている
/// 量のほうが大きいためである。呼出し元が比を見られるよう、判定は変えずに両方返す。
pub(super) fn ladder_truth(
    exact: &BTreeMap<FaceId, Vec<V3>>,
    ladder: &[(f64, BTreeMap<FaceId, Vec<V3>>)],
    overlap: &CoincidentOverlap,
) -> Option<(bool, f64, f64)> {
    for (seam, probe) in ladder {
        let Some(difference) = probe_height_difference(exact, probe, overlap) else {
            continue;
        };
        if difference.abs() > 1e-12 {
            return Some((difference > 0.0, difference, *seam));
        }
    }
    None
}

#[derive(Default, Debug, Clone, Copy)]
pub(super) struct TopFaceAudit {
    /// 完全に重なっている面対の数。
    pub(super) overlaps: usize,
    /// 折り目1本を戻す判定で上下が決まった面対の数。
    pub(super) local_decided: usize,
    /// 全体を同じ割合で戻して解き直す判定で上下が決まった面対の数。
    pub(super) ladder_decided: usize,
    /// 2つの独立判定が食い違った面対の数。
    pub(super) truth_disagreements: usize,
    /// 独立判定とアプリの重なり順が食い違った面対の数。
    pub(super) rank_mismatches: usize,
    /// どちらの独立判定でも決められなかった面対の数。
    pub(super) undecided: usize,
}

/// 完全に重なっている面対それぞれについて「上に来るべき面」を独立に求め、
/// アプリの `surface_rank` が選ぶ面と突き合わせる。
pub(super) fn audit_top_faces(
    cp: &CreasePattern,
    faces: &[Face],
    diagram: &Diagram,
    frame: &Frame3D,
    angles: &HashMap<EdgeId, f64>,
    mut report: impl FnMut(&CoincidentOverlap, bool, bool, Option<(f64, f64)>),
) -> TopFaceAudit {
    let exact = frame_polygons(frame);
    let ranks = frame
        .faces
        .iter()
        .map(|face| (face.face, face.surface_rank))
        .collect::<BTreeMap<_, _>>();
    let mut audit = TopFaceAudit::default();
    let overlaps = coincident_overlaps(diagram, frame);
    if overlaps.is_empty() {
        return audit;
    }
    let ladder = solved_unfold_ladder(cp, faces, angles);
    for overlap in overlaps {
        audit.overlaps += 1;
        let local = local_hinge_truth(cp, faces, angles, &exact, &overlap);
        let ladder_answer = ladder_truth(&exact, &ladder, &overlap);
        if let (Some(local), Some((ladder_above, _, _))) = (local, ladder_answer)
            && local != ladder_above
        {
            audit.truth_disagreements += 1;
        }
        if local.is_some() {
            audit.local_decided += 1;
        }
        if ladder_answer.is_some() {
            audit.ladder_decided += 1;
        }
        // 実際に解いた梯子を正とする。決められなかった面対だけ、裂けていない
        // 折り目1本の探りを使う。
        let Some(truth_left_above) = ladder_answer.map(|(above, _, _)| above).or(local) else {
            audit.undecided += 1;
            continue;
        };
        let rank_left_above = ranks[&overlap.left] > ranks[&overlap.right];
        if rank_left_above != truth_left_above {
            audit.rank_mismatches += 1;
            report(
                &overlap,
                truth_left_above,
                rank_left_above,
                ladder_answer.map(|(_, height, seam)| (height, seam)),
            );
        }
    }
    audit
}

/// 重なり順をどの幾何から決めたかの名前。番号順の経路が残っていれば
/// `face-index` として数えられる。
pub(super) fn surface_order_source_label(source: Option<SurfaceOrderSource>) -> &'static str {
    match source {
        Some(SurfaceOrderSource::SolvedMotionPath) => "solved-motion-path",
        Some(SurfaceOrderSource::SolvedFlatPath) => "solved-flat-path",
        Some(SurfaceOrderSource::FoldFramePath) => "fold-frame-path",
        Some(SurfaceOrderSource::CurrentDepths) => "current-depths",
        Some(SurfaceOrderSource::PropagatedFlatPath) => "propagated-flat-path",
        Some(SurfaceOrderSource::NoOverlap) => "no-overlap",
        None => "not-stamped",
    }
}
