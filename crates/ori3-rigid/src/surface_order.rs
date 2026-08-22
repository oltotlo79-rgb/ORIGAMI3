//! 同一深度の紙面を、完全重なりへ入る直前の幾何から下→上へ並べる。

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::{DMat3, DVec2, DVec3};
use ori3_cp::Face;
use ori3_model::{CreasePattern, Driver, EPS, EdgeId, FaceId, Frame3D};

const COPLANAR_EPS: f64 = 1e-8;
/// 2枚の面を「同じ平面に乗っている」とみなす、頂点から平面までの距離の上限。
/// 法線の平行判定・深度順位の許容値とは別に置く。現行の裏表修正を含むA/B実測で、
/// 折り切った辺426の面対(6,8)/(7,9)は平面から最大 `1.253e-8` 離れた。
/// `1e-8` は両組を束から除外したが、`1e-6` は両組を拾い、同時にlive3の番号一致
/// 0/46・完全重なり9/9、8角度区分各20件の番号順0、110折り目のrank変更0、
/// 612方向の面状の裏0を維持した。この実測から恒久値を `1e-6` とする。
const COPLANAR_DISTANCE_EPS: f64 = 1e-6;
const DEPTH_ORDER_EPS: f64 = 1e-12;
const OVERLAP_AREA_EPS: f64 = 1e-14;
pub(crate) const EXACT_FLAT_EPS_RAD: f64 = 1e-8;
/// 平坦な展開図から最終角へ向かう決定的な重なり順経路。
///
/// 各値は全ヒンジに共通の角度checkpointである。各ヒンジは符号を保ったまま
/// `min(|最終角|, checkpoint)` まで動き、部分折りは最終角へ達した後固定される。
/// 終点だけで分離しない面対も、終点に最も近い分離点まで順に戻って物理的な上下を決める。
pub(crate) const SURFACE_PATH_CHECKPOINT_DEG: [f64; 22] = [
    9.0, 19.0, 29.0, 39.0, 49.0, 59.0, 69.0, 79.0, 90.0, 101.0, 111.0, 121.0, 131.0, 141.0, 151.0,
    161.0, 171.0, 179.0, 179.5, 179.9, 179.99, 179.999,
];

/// 重なり順を決めるとき「折り切った」とみなす角度(度)。最後から2つめのcheckpoint。
///
/// 経路の終点は**重なっている面対を選ぶための平らな束**を作るためだけに使い、
/// 上下そのものは経路上の実深度と折り目の向きが決める。したがってこの境目は、
/// 姿勢のわずかな違いでまたがれない程度に粗くなければならない。
///
/// 以前は `最後のcheckpoint − 1e-6`(= 179.998999)だった。実測では
/// `folded-sample.ori3` の辺125を送ったとき、連動する折り目185〜192が
/// **179.999°の姿勢で 179.998910、180°の姿勢で 179.999753** となり、
/// ちょうどこの境目をまたいでいた。またいだ側では終点が平らにならず、
/// `common_plane` の距離側の同一平面条件(現在の `COPLANAR_DISTANCE_EPS`、当時1e-8)を
/// 満たす面対が1組も無くなって、
/// 上下が1つも決まらないまま面の番号順が残っていた。
pub(crate) const STACK_FLAT_THRESHOLD_DEG: f64 =
    SURFACE_PATH_CHECKPOINT_DEG[SURFACE_PATH_CHECKPOINT_DEG.len() - 2];

/// 折り切ったとみなせる角度を ±180° へ寄せる。それ以外はそのまま返す。
pub(crate) fn snap_to_flat(angle_deg: f64) -> f64 {
    if angle_deg.abs() >= STACK_FLAT_THRESHOLD_DEG {
        angle_deg.signum() * 180.0
    } else {
        angle_deg
    }
}

type Transforms = HashMap<FaceId, (DMat3, DVec3)>;

/// 前の姿勢で、正面積を共有していた面対だけが持つ幾何由来の上下。
///
/// 中身はcompleteなsurface導出だけが作る。保存layerや全面total orderから構築する
/// public APIを持たせないことで、幾何が証明していないtieを次手順へ持ち込ませない。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceOrderProvenance {
    pub(crate) constraints: Vec<(FaceId, FaceId)>,
}

/// 重なり順の導出結果と、幾何からは決められなかった箇所の数。
///
/// 呼出し元は `unresolved_overlaps` を見て「幾何が答えを持っていない」ことを
/// 知る。面の番号順で当てずっぽうに埋めるのではなく、平らな状態からの経路など
/// 別の幾何へ切り替えるための判断材料である。
#[derive(Debug, Clone)]
pub(crate) struct SurfaceOrder {
    /// 下→上の面順。
    pub(crate) order: Vec<FaceId>,
    /// 実面積で重なっていて、上下を幾何から決められた面対の数。
    pub(crate) resolved_overlaps: usize,
    /// current depthや前姿勢を使わず、厳密折り目制約だけで比較できた重なり面対の数。
    pub(crate) exact_resolved_overlaps: usize,
    /// 経路または現在Frameで、許容値を越える実深度差を直接測れたoverlap対の数。
    pub(crate) sampled_depth_constraints: usize,
    /// 実面積で重なっているのに上下を決められなかった面対の数。
    pub(crate) unresolved_overlaps: usize,
    /// 多角形が退化していて比較そのものができなかった面対の数。
    pub(crate) skipped_pairs: usize,
    /// 上下の制約が輪になっていたため落とした制約の数。
    pub(crate) broken_constraints: usize,
    /// 折り目の向きが決める厳密な上下と食い違ったため捨てた、深度由来の制約の数。
    pub(crate) dropped_depth_constraints: usize,
    /// 実面積で重なる全ての面対が、幾何制約の推移閉包で比較可能か。
    pub(crate) complete: bool,
    /// completeなときだけ、次の手順へ渡せる正面積overlap制約を含む。
    pub(crate) provenance: SurfaceOrderProvenance,
}

#[derive(Debug)]
struct GeometricFaceKey {
    face: FaceId,
    centroid: DVec2,
    /// 展開図上のCCW境界を、辞書順最小の巡回表現へ正規化したもの。
    ring: Vec<DVec2>,
}

/// 面番号を一切使わない、制約付き安定ソートの種を作る。
///
/// 上下制約で比較できない面は実面積で重なっていないため、その全体順には物理的な
/// 意味が無い。それでも `surface_rank` は全面の順列を要求するので、展開図上の材質
/// 多角形だけから決まる順を使う。FaceId・VertexId・入力列は比較キーへ入れない。
pub(crate) fn geometric_seed_order(
    cp: &CreasePattern,
    faces: &[Face],
) -> Result<Vec<FaceId>, String> {
    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    if positions.len() != cp.vertices.len() {
        return Err("crease pattern contains duplicate vertex ids".to_string());
    }

    let mut keyed = Vec::with_capacity(faces.len());
    let mut face_ids = BTreeSet::new();
    for face in faces {
        if !face_ids.insert(face.id) {
            return Err("faces contain duplicate face ids".to_string());
        }
        let points = face
            .vertices
            .iter()
            .map(|vertex| {
                let point = positions
                    .get(vertex)
                    .copied()
                    .ok_or_else(|| "face refers to a missing material vertex".to_string())?;
                point
                    .is_finite()
                    .then_some(point)
                    .ok_or_else(|| "face contains a non-finite material vertex".to_string())
            })
            .collect::<Result<Vec<_>, String>>()?;
        if points.len() < 3 {
            return Err("face has fewer than three material vertices".to_string());
        }
        let start = (0..points.len())
            .min_by(|&left, &right| compare_cyclic_points(&points, left, right))
            .expect("a face with three vertices has a cyclic start");
        let ring = (0..points.len())
            .map(|offset| points[(start + offset) % points.len()])
            .collect::<Vec<_>>();
        let centroid = points.iter().copied().sum::<DVec2>() / points.len() as f64;
        keyed.push(GeometricFaceKey {
            face: face.id,
            centroid,
            ring,
        });
    }
    keyed.sort_by(compare_geometric_face_keys);
    if keyed
        .windows(2)
        .any(|pair| compare_geometric_face_keys(&pair[0], &pair[1]) == Ordering::Equal)
    {
        return Err("faces contain duplicate material polygons".to_string());
    }
    Ok(keyed.into_iter().map(|key| key.face).collect())
}

fn compare_scalar(left: f64, right: f64) -> Ordering {
    // `-0.0` と `+0.0` は同じ材質座標なので、bit表現ではなく数値として同一視する。
    if left == right {
        Ordering::Equal
    } else {
        left.total_cmp(&right)
    }
}

fn compare_point(left: DVec2, right: DVec2) -> Ordering {
    compare_scalar(left.x, right.x).then_with(|| compare_scalar(left.y, right.y))
}

fn compare_cyclic_points(points: &[DVec2], left: usize, right: usize) -> Ordering {
    (0..points.len())
        .map(|offset| {
            compare_point(
                points[(left + offset) % points.len()],
                points[(right + offset) % points.len()],
            )
        })
        .find(|ordering| *ordering != Ordering::Equal)
        .unwrap_or(Ordering::Equal)
}

fn compare_geometric_face_keys(left: &GeometricFaceKey, right: &GeometricFaceKey) -> Ordering {
    compare_scalar(left.centroid.y, right.centroid.y)
        .then_with(|| compare_scalar(left.centroid.x, right.centroid.x))
        .then_with(|| left.ring.len().cmp(&right.ring.len()))
        .then_with(|| {
            left.ring
                .iter()
                .copied()
                .zip(right.ring.iter().copied())
                .map(|(left, right)| compare_point(left, right))
                .find(|ordering| *ordering != Ordering::Equal)
                .unwrap_or(Ordering::Equal)
        })
}

/// `path` の実深度を制約、材質多角形の順を非重複面のtie-breakとして、下→上を返す。
///
/// `exact_frame` で同一平面かつ実面積が重なる面対だけを比較する。exact上の同一点を
/// それぞれの材質座標へ戻してから経路上の各姿勢へ写すため、祖先面の共通剛体運動は
/// 相殺される。終点に近い点から調べ、まだ同じ深度なら前の点へ戻る。画面履歴や
/// カメラを入力にせず、同じ4入力には常に同じ順を返す。
pub(crate) fn derive_surface_order(
    cp: &CreasePattern,
    faces: &[Face],
    path: &[Transforms],
    exact: &Transforms,
    exact_frame: &Frame3D,
    exact_constraints: &[(FaceId, FaceId)],
    previous: Option<&SurfaceOrderProvenance>,
) -> Result<SurfaceOrder, String> {
    let seed_order = geometric_seed_order(cp, faces)?;
    validate_order(faces, exact_frame, &seed_order)?;
    derive_surface_order_with(
        faces,
        exact_frame,
        &seed_order,
        DeriveSurfaceOptions {
            exact_constraints,
            previous,
            require_coplanar: true,
            sample_count: path.len(),
        },
        |sample, face, point, normal| approached_height(face, point, normal, &path[sample], exact),
    )
}

/// Order nearly parallel overlapping faces by their current physical separation.
pub(crate) fn derive_surface_order_from_current_depths(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &Frame3D,
    exact_constraints: &[(FaceId, FaceId)],
) -> Result<SurfaceOrder, String> {
    let seed_order = geometric_seed_order(cp, faces)?;
    validate_order(faces, frame, &seed_order)?;
    let frame_faces = frame
        .faces
        .iter()
        .map(|face| (face.face, face))
        .collect::<HashMap<_, _>>();
    derive_surface_order_with(
        faces,
        frame,
        &seed_order,
        DeriveSurfaceOptions {
            exact_constraints,
            previous: None,
            require_coplanar: false,
            sample_count: 1,
        },
        |_sample, face, point, normal| {
            approached_frame_height(face, point, normal, &frame_faces, &frame_faces)
        },
    )
}

/// `Frame3D` で再生した全ヒンジ経路を、完全折りendpointの同じ材質点で比較する。
/// solverの各checkpointに含まれる従属ヒンジも、そのまま上下制約へ参加する。
pub(crate) fn derive_surface_order_from_frame_path(
    cp: &CreasePattern,
    faces: &[Face],
    path: &[Frame3D],
    exact_frame: &Frame3D,
    exact_constraints: &[(FaceId, FaceId)],
) -> Result<SurfaceOrder, String> {
    derive_surface_order_from_frame_path_with_previous(
        cp,
        faces,
        path,
        exact_frame,
        exact_constraints,
        None,
    )
}

/// 2つの姿勢の、同じ面の同じ頂点どうしの最大の離れ。
fn frame_distance(left: &Frame3D, right: &Frame3D) -> f64 {
    let right_faces = right
        .faces
        .iter()
        .map(|face| (face.face, face))
        .collect::<HashMap<_, _>>();
    left.faces
        .iter()
        .flat_map(|face| {
            right_faces
                .get(&face.face)
                .into_iter()
                .flat_map(move |other| {
                    face.polygon
                        .iter()
                        .zip(&other.polygon)
                        .map(|(&here, &there)| (DVec3::from(here) - DVec3::from(there)).length())
                })
        })
        .fold(0.0_f64, f64::max)
}

/// 経路が「終点へ向かう途中の姿勢の並び」になっているか。
///
/// 重なり順は、終点の直前の姿勢の高さの差から読む。この読み方が成り立つのは、
/// 渡された並びが**終点へ向かう途中の姿勢**であるときだけである。
///
/// 終点との離れを**1度も変えない**並びは、同じ姿勢を繰り返しているだけで、
/// 途中の姿勢ではない。その高さの差は、その1つの姿勢の話であって、終点の上下の
/// 証拠にはならない。**ただし、その姿勢が終点そのもの(離れが 0)なら**、
/// 読み取る高さの差は終点の実深度そのものなので、これまでどおり証拠になる。
///
/// 判定に許容値を置かない。使うのは「終点との離れが変わったか」と
/// 「終点に着いているか」という、同じ単位の実測どうしの比較だけである。
/// 離れが変わる並びは、近づく形も遠ざかる形も、これまでどおり使う。
///
/// **この判定が捕まえた不具合(2026-08-22)**:
/// `crates/ori3-layers/tests/fixtures/folded-sample.ori3`(46面・101折り目・8手、
/// 紙の長辺 = 1.0)の手順4〜7で、手順の再生が渡す3姿勢の並びは、終点から
/// それぞれ 1.365 / 0.07405 / 0.2717 / **0.5756** 離れた**同じ姿勢**の繰り返しだった。
/// 手順7ではその止まった姿勢の高さの差が、完全に重なる4組
/// (24,29)・(25,28)・(27,33)・(39,43) の上下を、**実際に表示する動きと逆**に決めていた。
/// 表示する動き(`replay(doc, 7, t)`)では面24が面29より t=0.5 で +6.678e-3、
/// t=0.9999 で +3.101e-2 だけ**上**にあるのに、止まった姿勢では **−1.816e-2**(下)だった。
/// 手順8は角度を1本も変えないので、その誤りが完成形にそのまま残っていた。
fn path_approaches_endpoint(path: &[Frame3D], exact_frame: &Frame3D) -> bool {
    // 姿勢が1つしかない並びには「変わったか」を問えない。これまでどおり使う。
    if path.len() < 2 {
        return true;
    }
    let mut distances = path.iter().map(|frame| frame_distance(frame, exact_frame));
    let Some(first) = distances.next() else {
        return true;
    };
    let (mut nearest, mut farthest, mut last) = (first, first, first);
    for distance in distances {
        nearest = nearest.min(distance);
        farthest = farthest.max(distance);
        last = distance;
    }
    // 終点に着いている並びは、そこで読む高さの差が終点の実深度そのものである。
    // 頂点座標がそのまま一致したときだけ 0 になるので、丸めの許容値は要らない。
    last == 0.0 || nearest < farthest
}

/// replayの実current-step経路を使い、前endpointの重なり制約で同深度対だけを補う。
pub(crate) fn derive_surface_order_from_frame_path_with_previous(
    cp: &CreasePattern,
    faces: &[Face],
    path: &[Frame3D],
    exact_frame: &Frame3D,
    exact_constraints: &[(FaceId, FaceId)],
    previous: Option<&SurfaceOrderProvenance>,
) -> Result<SurfaceOrder, String> {
    let seed_order = geometric_seed_order(cp, faces)?;
    validate_order(faces, exact_frame, &seed_order)?;
    for frame in path {
        validate_order(faces, frame, &seed_order)?;
    }
    // 同じ姿勢を繰り返すだけの並びは、終点の重なり順の証拠にならない
    // (`path_approaches_endpoint` の説明を参照)。
    let path: &[Frame3D] = if path_approaches_endpoint(path, exact_frame) {
        path
    } else {
        &[]
    };
    let exact_faces = exact_frame
        .faces
        .iter()
        .map(|face| (face.face, face))
        .collect::<HashMap<_, _>>();
    let path_faces = path
        .iter()
        .map(|frame| {
            frame
                .faces
                .iter()
                .map(|face| (face.face, face))
                .collect::<HashMap<_, _>>()
        })
        .collect::<Vec<_>>();
    derive_surface_order_with(
        faces,
        exact_frame,
        &seed_order,
        DeriveSurfaceOptions {
            exact_constraints,
            previous,
            require_coplanar: true,
            sample_count: path.len(),
        },
        |sample, face, point, normal| {
            approached_frame_height(face, point, normal, &path_faces[sample], &exact_faces)
        },
    )
}

struct DeriveSurfaceOptions<'a> {
    exact_constraints: &'a [(FaceId, FaceId)],
    previous: Option<&'a SurfaceOrderProvenance>,
    require_coplanar: bool,
    sample_count: usize,
}

fn derive_surface_order_with(
    faces: &[Face],
    exact_frame: &Frame3D,
    previous_order: &[FaceId],
    options: DeriveSurfaceOptions<'_>,
    mut height: impl FnMut(usize, FaceId, DVec3, DVec3) -> Result<f64, String>,
) -> Result<SurfaceOrder, String> {
    let DeriveSurfaceOptions {
        exact_constraints,
        previous,
        require_coplanar,
        sample_count,
    } = options;
    let frame_faces = exact_frame
        .faces
        .iter()
        .map(|face| (face.face, face))
        .collect::<HashMap<_, _>>();
    // Each face participates in up to F-1 comparisons. Its normal, local plane,
    // and projection onto that plane do not depend on the other face.
    let geometries = faces
        .iter()
        .map(|face| {
            let polygon = frame_faces[&face.id]
                .polygon
                .iter()
                .copied()
                .map(DVec3::from)
                .collect::<Vec<_>>();
            let plane = face_plane(&polygon);
            let projected = plane.map(|plane| project_polygon(&polygon, plane));
            FaceGeometry {
                id: face.id,
                plane,
                projected,
                normal: polygon_normal(&polygon),
                polygon,
            }
        })
        .collect::<Vec<_>>();
    // 折り目の向きが決める厳密な上下を先に入れる。深度の差は丸めで壊れ得るが、
    // 折り目の向きは壊れない。あとから来る深度由来の制約がこれと食い違ったら、
    // 深度側を捨てる(下の `dropped_depth_constraints`)。
    let known_faces = faces.iter().map(|face| face.id).collect::<BTreeSet<_>>();
    let exact_pairs = exact_constraints
        .iter()
        .copied()
        .filter(|(below, above)| {
            below != above && known_faces.contains(below) && known_faces.contains(above)
        })
        .collect::<BTreeSet<(FaceId, FaceId)>>();
    let mut constraints = exact_pairs.clone();
    // 大きな完全平坦stackでは全深度が同値なので、閉包は実際に有限な深度制約を
    // 1本でも得た場合だけ作る。400面のexact-only経路へ不要なO(F^3/word)を足さない。
    let mut accepted_constraints = None;
    let mut dropped_depth_constraints = 0_usize;
    let mut sampled_depth_constraints = 0_usize;
    let mut skipped_pairs = 0_usize;
    let mut overlap_pairs = BTreeSet::new();

    for left_index in 0..faces.len() {
        for right_index in left_index + 1..faces.len() {
            let left_geometry = &geometries[left_index];
            let right_geometry = &geometries[right_index];
            let left = left_geometry.id;
            let right = right_geometry.id;
            let Some(plane) = common_plane(left_geometry, right_geometry, require_coplanar) else {
                continue;
            };
            let left_2d = left_geometry
                .projected
                .as_deref()
                .expect("a face plane always has a projection");
            let right_2d = project_polygon(&right_geometry.polygon, plane);
            // Polygon clipping is only needed when the projected axis intervals
            // can meet. Expanding by EPS keeps boundary-tolerance cases on the
            // exact path while rejecting spatially separated faces cheaply.
            if !projected_bounds_overlap(left_2d, &right_2d) {
                continue;
            }
            // 1対の多角形が退化していても、他の面対から得た上下は捨てない。
            // 以前はここで全体をErrにしており、呼出し元が全16面を面の番号順へ
            // 落としていた。1対の失敗で紙全体の重なり順を失わせない。
            let Ok(witnesses) = overlap_witnesses(left_2d, &right_2d) else {
                skipped_pairs += 1;
                continue;
            };
            if witnesses.is_empty() {
                continue;
            }
            overlap_pairs.insert((left, right));
            let mut sampling_failed = false;
            for sample in (0..sample_count).rev() {
                let mut left_above = false;
                let mut right_above = false;
                for &witness in &witnesses {
                    let point = plane.origin + plane.u * witness.x + plane.v * witness.y;
                    let (Ok(left_height), Ok(right_height)) = (
                        height(sample, left, point, plane.normal),
                        height(sample, right, point, plane.normal),
                    ) else {
                        sampling_failed = true;
                        break;
                    };
                    let difference = left_height - right_height;
                    left_above |= difference > DEPTH_ORDER_EPS;
                    right_above |= difference < -DEPTH_ORDER_EPS;
                }
                if sampling_failed {
                    break;
                }
                // 同じ深度なら、経路上の1つ前の姿勢まで戻る。1つの面対が重なり領域内で
                // 交差する点では面単位rankを決めず、さらに前の非交差姿勢を探す。
                if left_above == right_above {
                    continue;
                }
                let depth_constraint = if left_above {
                    (right, left)
                } else {
                    (left, right)
                };
                let accepted_constraints = accepted_constraints
                    .get_or_insert_with(|| ConstraintClosure::new(&known_faces, &constraints));
                if accepted_constraints.reaches(depth_constraint.1, depth_constraint.0) {
                    // 180°の折り目が直接または推移的に決める上下と逆。折り目の向きは
                    // 丸めでは壊れないが、経路上の深度差は丸めや、経路を作れなかった
                    // 中間形で壊れる。既に採用した幾何制約を残し、深度由来のこの1件だけを
                    // 捨てる。直接の逆辺だけを見ると長いexact chainへ循環を作ってしまう。
                    dropped_depth_constraints += 1;
                } else {
                    // loop endpointのauthorityは、exactと両立して実際に採用できた
                    // 経路深度が少なくとも1本ある場合だけ発行する。捨てた逆向き
                    // sampleを「経路を測れた」証拠へ数えない。
                    sampled_depth_constraints += 1;
                    if !accepted_constraints.reaches(depth_constraint.0, depth_constraint.1) {
                        constraints.insert(depth_constraint);
                        accepted_constraints.insert(depth_constraint.0, depth_constraint.1);
                    }
                }
                break;
            }
            if sampling_failed {
                skipped_pairs += 1;
            }
        }
    }

    if let Some(previous) = previous {
        merge_previous_overlap_constraints(&mut constraints, &overlap_pairs, previous);
    }
    let (order, broken_constraints) = stable_topological_order(previous_order, &constraints);
    // 個々の深度比較が同値でも、別の面を介した幾何制約で上下が決まることがある。
    // total orderのtie-breakを答えと誤認せず、制約グラフそのものの推移閉包で、実面積が
    // 重なる全ての対が比較可能かを確かめる。
    let reachable = constraint_reachability(&overlap_pairs, &constraints);
    let resolved_overlaps = overlap_pairs
        .iter()
        .filter(|&&(left, right)| {
            reachable.contains(&(left, right)) || reachable.contains(&(right, left))
        })
        .count();
    let exact_resolved_overlaps = if constraints == exact_pairs {
        resolved_overlaps
    } else {
        let exact_reachable = constraint_reachability(&overlap_pairs, &exact_pairs);
        overlap_pairs
            .iter()
            .filter(|&&(left, right)| {
                exact_reachable.contains(&(left, right)) || exact_reachable.contains(&(right, left))
            })
            .count()
    };
    let unresolved_overlaps = overlap_pairs.len().saturating_sub(resolved_overlaps);
    let complete = skipped_pairs == 0 && broken_constraints == 0 && unresolved_overlaps == 0;
    let provenance = if complete {
        provenance_from_overlap_order(&overlap_pairs, &order)
    } else {
        SurfaceOrderProvenance {
            constraints: Vec::new(),
        }
    };
    Ok(SurfaceOrder {
        order,
        resolved_overlaps,
        exact_resolved_overlaps,
        sampled_depth_constraints,
        unresolved_overlaps,
        skipped_pairs,
        broken_constraints,
        dropped_depth_constraints,
        complete,
        provenance,
    })
}

/// 全面を一度ずつ含む下→上順を `surface_rank` へ刻印する。
pub fn stamp_surface_order(frame: &mut Frame3D, order: &[FaceId]) -> Result<(), String> {
    let frame_ids = frame
        .faces
        .iter()
        .map(|face| face.face)
        .collect::<BTreeSet<_>>();
    let order_ids = order.iter().copied().collect::<BTreeSet<_>>();
    if frame.faces.len() != order.len()
        || frame_ids.len() != frame.faces.len()
        || order_ids.len() != order.len()
        || frame_ids != order_ids
    {
        return Err("surface order does not contain every frame face exactly once".to_string());
    }
    let ranks = order
        .iter()
        .enumerate()
        .map(|(rank, &face)| {
            Ok((
                face,
                u32::try_from(rank).map_err(|_| "surface rank exceeds u32".to_string())?,
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    for face in &mut frame.faces {
        face.surface_rank = ranks[&face.face];
    }
    Ok(())
}

/// このmodule内で実行したgenuine `solve_motion` からだけ作れるsurface順の証明。
/// private fieldなので、公開boolや任意rankからopaque provenanceを組み立てられない。
pub struct AuthoritativeSurfaceOrder {
    frame: Frame3D,
    order: Vec<FaceId>,
    provenance: SurfaceOrderProvenance,
}

impl AuthoritativeSurfaceOrder {
    #[must_use]
    pub fn frame(&self) -> &Frame3D {
        &self.frame
    }

    #[must_use]
    pub fn into_parts(self) -> (Frame3D, Vec<FaceId>, SurfaceOrderProvenance) {
        (self.frame, self.order, self.provenance)
    }
}

/// 同じ終点角をwarmにした既存motion canonical pathがcompleteな場合だけ、次手順へ
/// 渡せるopaque provenanceを返す。外から構築された`MotionSolveResult`は受け取らない。
#[must_use]
pub fn solve_authoritative_surface_order(
    cp: &CreasePattern,
    faces: &[Face],
    angles: &HashMap<EdgeId, f64>,
) -> Option<AuthoritativeSurfaceOrder> {
    let mut drivers = angles
        .iter()
        .map(|(&hinge, &target_angle_deg)| Driver {
            hinge,
            target_angle_deg,
        })
        .collect::<Vec<_>>();
    drivers.sort_unstable_by_key(|driver| driver.hinge);
    let motion = crate::motion::solve_motion(cp, faces, &drivers, None, Some(angles), false);
    if !motion.surface_order_authoritative
        || motion.surface_order.is_none()
        || !signed_angles_match(angles, &motion.result.angles)
    {
        return None;
    }
    let mut ranked = motion
        .result
        .frame
        .faces
        .iter()
        .map(|face| (face.surface_rank, face.face))
        .collect::<Vec<_>>();
    ranked.sort_unstable();
    if ranked
        .iter()
        .enumerate()
        .any(|(rank, &(stored, _))| usize::try_from(stored).ok() != Some(rank))
    {
        return None;
    }
    let order = ranked.into_iter().map(|(_, face)| face).collect::<Vec<_>>();
    validate_order(faces, &motion.result.frame, &order).ok()?;

    // motion側で証明済みのtotal orderをchainへ戻し、このframeで実際に重なる面対だけを
    // provenanceへ収める。同じ役割の別順位導出を作らず、既存deriveのoverlap判定、
    // 推移閉包、skip/cycle検査をそのまま使う。
    let chain = order
        .windows(2)
        .map(|pair| (pair[0], pair[1]))
        .collect::<Vec<_>>();
    let derived =
        derive_surface_order_from_current_depths(cp, faces, &motion.result.frame, &chain).ok()?;
    (derived.complete && derived.order == order).then_some(AuthoritativeSurfaceOrder {
        frame: motion.result.frame,
        order,
        provenance: derived.provenance,
    })
}

fn signed_angles_match(expected: &HashMap<EdgeId, f64>, actual: &HashMap<EdgeId, f64>) -> bool {
    expected.len() == actual.len()
        && expected.iter().all(|(hinge, expected)| {
            actual
                .get(hinge)
                .is_some_and(|actual| (actual - expected).abs() <= 1e-9)
        })
}

fn validate_order(faces: &[Face], frame: &Frame3D, order: &[FaceId]) -> Result<(), String> {
    let face_ids = faces.iter().map(|face| face.id).collect::<BTreeSet<_>>();
    let frame_ids = frame
        .faces
        .iter()
        .map(|face| face.face)
        .collect::<BTreeSet<_>>();
    let order_ids = order.iter().copied().collect::<BTreeSet<_>>();
    if face_ids.len() != faces.len()
        || frame_ids.len() != frame.faces.len()
        || order_ids.len() != order.len()
        || face_ids != frame_ids
        || face_ids != order_ids
    {
        return Err("surface order inputs do not contain the same unique faces".to_string());
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Plane {
    origin: DVec3,
    normal: DVec3,
    u: DVec3,
    v: DVec3,
}

struct FaceGeometry {
    id: FaceId,
    polygon: Vec<DVec3>,
    normal: Option<DVec3>,
    plane: Option<Plane>,
    projected: Option<Vec<DVec2>>,
}

fn face_plane(polygon: &[DVec3]) -> Option<Plane> {
    let origin = *polygon.first()?;
    let raw_normal = polygon_normal(polygon)?;
    let normal = canonical(raw_normal);
    let u = polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .map(|(&a, &b)| b - a)
        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))?
        .normalize();
    let v = normal.cross(u).normalize();
    Some(Plane {
        origin,
        normal,
        u,
        v,
    })
}

fn common_plane(
    left: &FaceGeometry,
    right: &FaceGeometry,
    require_coplanar: bool,
) -> Option<Plane> {
    let plane = left.plane?;
    let right_normal = right.normal?;
    if plane.normal.dot(right_normal).abs() < 1.0 - COPLANAR_EPS
        || (require_coplanar
            && left
                .polygon
                .iter()
                .chain(&right.polygon)
                .any(|point| plane.normal.dot(*point - plane.origin).abs() > COPLANAR_DISTANCE_EPS))
    {
        return None;
    }
    Some(plane)
}

fn polygon_normal(points: &[DVec3]) -> Option<DVec3> {
    if points.len() < 3 {
        return None;
    }
    let normal = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(&a, &b)| a.cross(b))
        .sum::<DVec3>();
    (normal.length_squared() > EPS * EPS).then(|| normal.normalize())
}

/// 平面の2つの法線のうち、絶対値が最大の成分を正にした側を返す。`n` と `-n` に
/// 同じ向きを返すので、面の並び順によらず同じ「上」の向きを与える。
///
/// **画面側 `surfaceOwner.ts::canonicalize` と同じ式でなければならない。**
/// 画面はこの向きから面の表裏(`side`)を決め、`side * surface_rank` の順に描く。
/// ここだけ別の規則にすると、重なり順は「上」を+n方向で数えているのに画面は
/// −n方向で数える、という食い違いが起き、束の中で描く面が入れ替わる。
///
/// 実測(`diag_kome_edge12_float32_axis_choice`): 絶対値が最大の成分を「ほぼ同じ
/// なら軸の優先順(x→y→z)」に変える帯(相対1e-7)を **Rust側だけ**に入れたところ、
/// `diagonal-midline-square` の辺12を+180°へ送った姿勢で、面4の裏が
/// **31,991画素**新たに見えるようになった。画面側は頂点をFloat32で読むが、
/// この姿勢での \|x\|−\|y\| はFloat32でも **−6.118e-8** と符号が変わらないため、
/// 画面は帯を入れないRustと同じ軸を選ぶ。したがって帯は入れない。
pub(crate) fn canonical(mut normal: DVec3) -> DVec3 {
    let absolute = normal.abs();
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

fn project_polygon(points: &[DVec3], plane: Plane) -> Vec<DVec2> {
    points
        .iter()
        .map(|point| {
            let relative = *point - plane.origin;
            DVec2::new(relative.dot(plane.u), relative.dot(plane.v))
        })
        .collect()
}

fn projected_bounds_overlap(left: &[DVec2], right: &[DVec2]) -> bool {
    let bounds = |polygon: &[DVec2]| {
        polygon.iter().copied().fold(
            (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
            |(minimum, maximum), point| (minimum.min(point), maximum.max(point)),
        )
    };
    let (left_minimum, left_maximum) = bounds(left);
    let (right_minimum, right_maximum) = bounds(right);
    left_maximum.x + EPS >= right_minimum.x
        && right_maximum.x + EPS >= left_minimum.x
        && left_maximum.y + EPS >= right_minimum.y
        && right_maximum.y + EPS >= left_minimum.y
}

fn approached_height(
    face: FaceId,
    point: DVec3,
    normal: DVec3,
    approached: &Transforms,
    exact: &Transforms,
) -> Result<f64, String> {
    let (exact_rotation, exact_translation) = exact
        .get(&face)
        .ok_or_else(|| format!("exact transform lost face {face}"))?;
    let (approached_rotation, approached_translation) = approached
        .get(&face)
        .ok_or_else(|| format!("approach transform lost face {face}"))?;
    let material = exact_rotation.transpose() * (point - *exact_translation);
    let approached_point = *approached_rotation * material + *approached_translation;
    if !approached_point.is_finite() {
        return Err(format!("face {face} produced a non-finite depth sample"));
    }
    Ok(approached_point.dot(normal))
}

fn approached_frame_height(
    face: FaceId,
    point: DVec3,
    normal: DVec3,
    approached_faces: &HashMap<FaceId, &ori3_model::Face3D>,
    exact_faces: &HashMap<FaceId, &ori3_model::Face3D>,
) -> Result<f64, String> {
    let approached = approached_faces
        .get(&face)
        .ok_or_else(|| format!("approach frame lost face {face}"))?;
    let exact = exact_faces
        .get(&face)
        .ok_or_else(|| format!("exact frame lost face {face}"))?;
    if approached.polygon.len() != exact.polygon.len() || exact.polygon.len() < 3 {
        return Err(format!("face {face} changed its polygon topology"));
    }

    let exact_points = exact
        .polygon
        .iter()
        .copied()
        .map(DVec3::from)
        .collect::<Vec<_>>();
    let approached_points = approached
        .polygon
        .iter()
        .copied()
        .map(DVec3::from)
        .collect::<Vec<_>>();
    let exact_origin = exact_points[0];
    let Some((first, second)) = (1..exact_points.len()).find_map(|first| {
        (first + 1..exact_points.len())
            .find(|&second| {
                (exact_points[first] - exact_origin)
                    .cross(exact_points[second] - exact_origin)
                    .length_squared()
                    > EPS * EPS
            })
            .map(|second| (first, second))
    }) else {
        return Err(format!("face {face} has no non-collinear material basis"));
    };

    let exact_first = exact_points[first] - exact_origin;
    let exact_second = exact_points[second] - exact_origin;
    let relative = point - exact_origin;
    let first_squared = exact_first.length_squared();
    let cross = exact_first.dot(exact_second);
    let second_squared = exact_second.length_squared();
    let determinant = first_squared * second_squared - cross * cross;
    if determinant.abs() <= EPS * EPS {
        return Err(format!("face {face} has a singular material basis"));
    }
    let relative_first = relative.dot(exact_first);
    let relative_second = relative.dot(exact_second);
    let first_weight = (relative_first * second_squared - relative_second * cross) / determinant;
    let second_weight = (relative_second * first_squared - relative_first * cross) / determinant;

    let approached_origin = approached_points[0];
    let approached_point = approached_origin
        + (approached_points[first] - approached_origin) * first_weight
        + (approached_points[second] - approached_origin) * second_weight;
    if !approached_point.is_finite() {
        return Err(format!(
            "face {face} produced a non-finite frame depth sample"
        ));
    }
    Ok(approached_point.dot(normal))
}

fn overlap_witnesses(left: &[DVec2], right: &[DVec2]) -> Result<Vec<DVec2>, String> {
    let mut witnesses = Vec::new();
    for left_triangle in triangulate_polygon(left)? {
        for right_triangle in triangulate_polygon(right)? {
            let intersection = intersect_convex_polygons(&left_triangle, &right_triangle);
            if polygon_area(&intersection).abs() <= OVERLAP_AREA_EPS {
                continue;
            }
            let center = intersection.iter().copied().sum::<DVec2>() / intersection.len() as f64;
            witnesses.push(center);
            witnesses.extend(
                intersection
                    .iter()
                    .copied()
                    .map(|point| (point + center) * 0.5),
            );
        }
    }
    Ok(witnesses)
}

fn triangulate_polygon(boundary: &[DVec2]) -> Result<Vec<Vec<DVec2>>, String> {
    let mut polygon = simple_polygon(boundary);
    if polygon.len() < 3 || polygon_area(&polygon).abs() <= OVERLAP_AREA_EPS {
        return Err("surface order encountered a degenerate face polygon".to_string());
    }
    if polygon_area(&polygon) < 0.0 {
        polygon.reverse();
    }
    let mut triangles = Vec::with_capacity(polygon.len().saturating_sub(2));
    while polygon.len() > 3 {
        let count = polygon.len();
        let Some(ear) = (0..count).find(|&index| {
            let a = polygon[(index + count - 1) % count];
            let b = polygon[index];
            let c = polygon[(index + 1) % count];
            (b - a).perp_dot(c - b) > EPS * EPS
                && !polygon.iter().enumerate().any(|(other, &point)| {
                    other != index
                        && other != (index + count - 1) % count
                        && other != (index + 1) % count
                        && point_in_triangle(point, a, b, c)
                })
        }) else {
            return Err("surface order could not triangulate a face polygon".to_string());
        };
        triangles.push(vec![
            polygon[(ear + count - 1) % count],
            polygon[ear],
            polygon[(ear + 1) % count],
        ]);
        polygon.remove(ear);
    }
    triangles.push(polygon);
    Ok(triangles)
}

fn simple_polygon(boundary: &[DVec2]) -> Vec<DVec2> {
    let mut polygon = Vec::with_capacity(boundary.len());
    for &point in boundary {
        if polygon
            .last()
            .is_none_or(|previous: &DVec2| (*previous - point).length() > EPS)
        {
            polygon.push(point);
        }
    }
    while polygon.len() > 1 && (polygon[0] - polygon[polygon.len() - 1]).length() <= EPS {
        polygon.pop();
    }
    polygon
}

fn point_in_triangle(point: DVec2, a: DVec2, b: DVec2, c: DVec2) -> bool {
    (b - a).perp_dot(point - a) >= -EPS
        && (c - b).perp_dot(point - b) >= -EPS
        && (a - c).perp_dot(point - c) >= -EPS
}

fn intersect_convex_polygons(subject: &[DVec2], clip: &[DVec2]) -> Vec<DVec2> {
    let mut output = subject.to_vec();
    for index in 0..clip.len() {
        let clip_start = clip[index];
        let clip_end = clip[(index + 1) % clip.len()];
        let input = std::mem::take(&mut output);
        let Some(mut previous) = input.last().copied() else {
            break;
        };
        let mut previous_side = (clip_end - clip_start).perp_dot(previous - clip_start);
        for current in input {
            let current_side = (clip_end - clip_start).perp_dot(current - clip_start);
            let previous_inside = previous_side >= -EPS;
            let current_inside = current_side >= -EPS;
            if previous_inside != current_inside {
                let denominator = previous_side - current_side;
                if denominator.abs() > EPS * EPS {
                    output.push(previous + (current - previous) * (previous_side / denominator));
                }
            }
            if current_inside {
                output.push(current);
            }
            previous = current;
            previous_side = current_side;
        }
    }
    deduplicate_polygon(output)
}

fn deduplicate_polygon(points: Vec<DVec2>) -> Vec<DVec2> {
    let mut output = Vec::with_capacity(points.len());
    for point in points {
        if output
            .last()
            .is_none_or(|previous: &DVec2| (*previous - point).length() > EPS)
        {
            output.push(point);
        }
    }
    if output.len() > 1 && (output[0] - output[output.len() - 1]).length() <= EPS {
        output.pop();
    }
    output
}

fn polygon_area(polygon: &[DVec2]) -> f64 {
    if polygon.len() < 3 {
        return 0.0;
    }
    0.5 * (0..polygon.len())
        .map(|index| polygon[index].perp_dot(polygon[(index + 1) % polygon.len()]))
        .sum::<f64>()
}

/// 前姿勢で重なっていた面対の上下を、現姿勢でも重なる未比較の対だけへ引き継ぐ。
///
/// `constraints` には現手順のexact/depth制約が先に入っている。どちら向きでも既に
/// 推移的に比較できる対は触らないため、現手順の幾何が常に優先される。逆向きの到達
/// 経路が無いことも確認してから追加し、前姿勢由来の制約で循環を作らない。
fn merge_previous_overlap_constraints(
    constraints: &mut BTreeSet<(FaceId, FaceId)>,
    overlap_pairs: &BTreeSet<(FaceId, FaceId)>,
    previous: &SurfaceOrderProvenance,
) {
    let known_faces = constraints
        .iter()
        .chain(overlap_pairs)
        .flat_map(|&(below, above)| [below, above])
        .chain(
            previous
                .constraints
                .iter()
                .flat_map(|&(below, above)| [below, above]),
        )
        .collect::<BTreeSet<_>>();
    let mut closure = ConstraintClosure::new(&known_faces, constraints);
    for &(below, above) in &previous.constraints {
        if below == above
            || !(overlap_pairs.contains(&(below, above)) || overlap_pairs.contains(&(above, below)))
            || closure.reaches(below, above)
            || closure.reaches(above, below)
        {
            continue;
        }
        constraints.insert((below, above));
        closure.insert(below, above);
    }
}

/// 制約の推移閉包。前provenanceの各辺ごとに全制約をDFSし直さず、bitsetで関係を保つ。
struct ConstraintClosure {
    index: HashMap<FaceId, usize>,
    rows: Vec<Vec<u64>>,
}

impl ConstraintClosure {
    fn new(faces: &BTreeSet<FaceId>, constraints: &BTreeSet<(FaceId, FaceId)>) -> Self {
        let index = faces
            .iter()
            .copied()
            .enumerate()
            .map(|(index, face)| (face, index))
            .collect::<HashMap<_, _>>();
        let words = faces.len().div_ceil(u64::BITS as usize);
        let mut rows = vec![vec![0_u64; words]; faces.len()];
        {
            let mut set = |from: usize, to: usize| {
                rows[from][to / u64::BITS as usize] |= 1_u64 << (to % u64::BITS as usize);
            };
            for position in 0..faces.len() {
                set(position, position);
            }
            for &(below, above) in constraints {
                if let (Some(&from), Some(&to)) = (index.get(&below), index.get(&above)) {
                    set(from, to);
                }
            }
        }
        for through in 0..faces.len() {
            let through_row = rows[through].clone();
            let word = through / u64::BITS as usize;
            let bit = 1_u64 << (through % u64::BITS as usize);
            for row in &mut rows {
                if row[word] & bit != 0 {
                    for (cell, &reachable) in row.iter_mut().zip(&through_row) {
                        *cell |= reachable;
                    }
                }
            }
        }
        Self { index, rows }
    }

    fn reaches(&self, from: FaceId, to: FaceId) -> bool {
        let (Some(&from), Some(&to)) = (self.index.get(&from), self.index.get(&to)) else {
            return false;
        };
        self.rows[from][to / u64::BITS as usize] & (1_u64 << (to % u64::BITS as usize)) != 0
    }

    fn insert(&mut self, from: FaceId, to: FaceId) {
        let (Some(&from), Some(&to)) = (self.index.get(&from), self.index.get(&to)) else {
            return;
        };
        let successors = self.rows[to].clone();
        let word = from / u64::BITS as usize;
        let bit = 1_u64 << (from % u64::BITS as usize);
        for row in &mut self.rows {
            if row[word] & bit != 0 {
                for (cell, &reachable) in row.iter_mut().zip(&successors) {
                    *cell |= reachable;
                }
            }
        }
    }
}

/// completeな導出結果から、正面積で重なる面対だけを次手順用に取り出す。
///
/// `order` のrankだけで向きを決め、そのrank順で格納する。FaceIdは向き・列順の
/// tie-breakに使わない。
fn provenance_from_overlap_order(
    overlap_pairs: &BTreeSet<(FaceId, FaceId)>,
    order: &[FaceId],
) -> SurfaceOrderProvenance {
    let rank = order
        .iter()
        .copied()
        .enumerate()
        .map(|(rank, face)| (face, rank))
        .collect::<HashMap<_, _>>();
    let mut constraints = overlap_pairs
        .iter()
        .map(|&(left, right)| {
            if rank[&left] < rank[&right] {
                (left, right)
            } else {
                (right, left)
            }
        })
        .collect::<Vec<_>>();
    constraints.sort_by_key(|&(below, above)| {
        (
            rank[&above].saturating_sub(rank[&below]),
            rank[&below],
            rank[&above],
        )
    });
    SurfaceOrderProvenance { constraints }
}

/// 実面積で重なる面だけを始点に、上下制約の推移閉包を作る。
fn constraint_reachability(
    overlap_pairs: &BTreeSet<(FaceId, FaceId)>,
    constraints: &BTreeSet<(FaceId, FaceId)>,
) -> BTreeSet<(FaceId, FaceId)> {
    let sources = overlap_pairs
        .iter()
        .flat_map(|&(left, right)| [left, right])
        .collect::<BTreeSet<_>>();
    let mut outgoing = BTreeMap::<FaceId, Vec<FaceId>>::new();
    for &(below, above) in constraints {
        outgoing.entry(below).or_default().push(above);
    }
    let mut reachable = BTreeSet::new();
    for source in sources {
        let mut visited = BTreeSet::new();
        let mut pending = vec![source];
        while let Some(face) = pending.pop() {
            let Some(above) = outgoing.get(&face) else {
                continue;
            };
            for &next in above {
                if visited.insert(next) {
                    reachable.insert((source, next));
                    pending.push(next);
                }
            }
        }
    }
    reachable
}

/// 下→上の制約を満たす順を返す。制約が輪になっていても止まらず、落とした制約の
/// 数を返す。輪は「紙がすり抜けている」形でだけ起きるので、そこで全体を面の番号順へ
/// 捨てるより、残りの制約を全て活かした順を返すほうが実際の重なりに近い。
fn stable_topological_order(
    previous_order: &[FaceId],
    constraints: &BTreeSet<(FaceId, FaceId)>,
) -> (Vec<FaceId>, usize) {
    if constraints.is_empty() {
        return (previous_order.to_vec(), 0);
    }
    let mut outgoing = previous_order
        .iter()
        .copied()
        .map(|face| (face, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut indegree = previous_order
        .iter()
        .copied()
        .map(|face| (face, 0usize))
        .collect::<BTreeMap<_, _>>();
    for &(below, above) in constraints {
        let (Some(neighbors), true) = (outgoing.get_mut(&below), indegree.contains_key(&above))
        else {
            // 呼出し元の面集合に無い面を指す制約は作れない(`validate_order` 済み)。
            continue;
        };
        if neighbors.insert(above) {
            *indegree.get_mut(&above).expect("checked above face") += 1;
        }
    }
    let mut emitted = BTreeSet::new();
    let mut order = Vec::with_capacity(previous_order.len());
    let mut broken = 0_usize;
    while order.len() < previous_order.len() {
        let ready = previous_order
            .iter()
            .copied()
            .find(|face| !emitted.contains(face) && indegree[face] == 0);
        let next = match ready {
            Some(face) => face,
            // 輪になった。まだ出していない面のうち、下から押さえる制約が最も少ない
            // 面を出す。同数なら `previous_order` の並びで決めるので結果は決定的。
            None => {
                let Some(face) = previous_order
                    .iter()
                    .copied()
                    .filter(|face| !emitted.contains(face))
                    .min_by_key(|face| indegree[face])
                else {
                    break;
                };
                broken += indegree[&face];
                face
            }
        };
        emitted.insert(next);
        order.push(next);
        for &above in &outgoing[&next] {
            if !emitted.contains(&above) {
                let degree = indegree.get_mut(&above).expect("known above face");
                *degree = degree.saturating_sub(1);
            }
        }
    }
    (order, broken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ori3_model::{Face3D, Vertex};

    fn material_cp(vertices: &[(u32, [f64; 2])]) -> CreasePattern {
        CreasePattern {
            vertices: vertices
                .iter()
                .map(|&(id, pos)| Vertex { id, pos })
                .collect(),
            edges: Vec::new(),
            next_vertex_id: vertices
                .iter()
                .map(|(id, _)| *id)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
            next_edge_id: 0,
        }
    }

    fn face(id: FaceId, vertices: &[u32]) -> Face {
        Face {
            id,
            vertices: vertices.to_vec(),
            edges: Vec::new(),
        }
    }

    #[test]
    fn geometric_seed_uses_material_polygons_instead_of_face_ids() {
        let cp = material_cp(&[
            (0, [2.0, 0.0]),
            (1, [3.0, 0.0]),
            (2, [3.0, 1.0]),
            (3, [2.0, 1.0]),
            (4, [0.0, 0.0]),
            (5, [1.0, 0.0]),
            (6, [1.0, 1.0]),
            (7, [0.0, 1.0]),
        ]);
        let faces = vec![face(1, &[0, 1, 2, 3]), face(99, &[6, 7, 4, 5])];

        assert_eq!(geometric_seed_order(&cp, &faces), Ok(vec![99, 1]));
    }

    #[test]
    fn geometric_seed_rejects_duplicate_material_polygons() {
        let cp = material_cp(&[
            (0, [0.0, 0.0]),
            (1, [1.0, 0.0]),
            (2, [0.0, 1.0]),
            (3, [0.0, 0.0]),
            (4, [1.0, 0.0]),
            (5, [0.0, 1.0]),
        ]);
        let faces = vec![face(2, &[0, 1, 2]), face(1, &[4, 5, 3])];

        assert!(geometric_seed_order(&cp, &faces).is_err());
    }

    #[test]
    fn overlap_completeness_uses_the_constraint_transitive_closure() {
        let cp = material_cp(&[
            (0, [4.0, 0.0]),
            (1, [5.0, 0.0]),
            (2, [4.0, 1.0]),
            (3, [2.0, 0.0]),
            (4, [3.0, 0.0]),
            (5, [2.0, 1.0]),
            (6, [0.0, 0.0]),
            (7, [1.0, 0.0]),
            (8, [0.0, 1.0]),
        ]);
        let faces = vec![
            face(30, &[0, 1, 2]),
            face(10, &[3, 4, 5]),
            face(20, &[6, 7, 8]),
        ];
        let polygon = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let frame = Frame3D {
            faces: faces
                .iter()
                .map(|face| Face3D {
                    face: face.id,
                    polygon: polygon.clone(),
                    layer: 0,
                    surface_rank: 0,
                    mirrored: false,
                })
                .collect(),
            warnings: Vec::new(),
        };

        let complete =
            derive_surface_order_from_current_depths(&cp, &faces, &frame, &[(30, 10), (10, 20)])
                .expect("material faces and frame contain the same ids");
        assert!(complete.complete);
        assert_eq!(complete.resolved_overlaps, 3);
        assert_eq!(complete.unresolved_overlaps, 0);
        assert_eq!(complete.order, vec![30, 10, 20]);
        assert_eq!(
            complete.provenance.constraints,
            vec![(30, 10), (10, 20), (30, 20)]
        );

        let incomplete = derive_surface_order_from_current_depths(&cp, &faces, &frame, &[(30, 10)])
            .expect("material faces and frame contain the same ids");
        assert!(!incomplete.complete);
        assert_eq!(incomplete.resolved_overlaps, 1);
        assert_eq!(incomplete.unresolved_overlaps, 2);
        assert!(
            incomplete.provenance.constraints.is_empty(),
            "不完全な導出は次手順用provenanceを発行しない"
        );
    }

    #[test]
    fn current_constraint_wins_over_reverse_previous_provenance() {
        let overlaps = BTreeSet::from([(10, 20)]);
        let previous = SurfaceOrderProvenance {
            constraints: vec![(10, 20)],
        };
        let mut current = BTreeSet::from([(20, 10)]);

        merge_previous_overlap_constraints(&mut current, &overlaps, &previous);

        assert_eq!(current, BTreeSet::from([(20, 10)]));

        let mut unresolved = BTreeSet::new();
        merge_previous_overlap_constraints(&mut unresolved, &overlaps, &previous);
        assert_eq!(unresolved, BTreeSet::from([(10, 20)]));
    }

    #[test]
    fn actual_frame_path_wins_when_flat_reconstruction_has_the_reverse_depth() {
        let cp = material_cp(&[
            (0, [0.0, 0.0]),
            (1, [1.0, 0.0]),
            (2, [0.0, 1.0]),
            (3, [2.0, 0.0]),
            (4, [3.0, 0.0]),
            (5, [2.0, 1.0]),
        ]);
        let faces = vec![face(10, &[0, 1, 2]), face(20, &[3, 4, 5])];
        let polygon_at = |z| vec![[0.0, 0.0, z], [1.0, 0.0, z], [0.0, 1.0, z]];
        let frame_at = |left_z, right_z| Frame3D {
            faces: vec![
                Face3D {
                    face: 10,
                    polygon: polygon_at(left_z),
                    layer: 0,
                    surface_rank: 0,
                    mirrored: false,
                },
                Face3D {
                    face: 20,
                    polygon: polygon_at(right_z),
                    layer: 0,
                    surface_rank: 0,
                    mirrored: false,
                },
            ],
            warnings: Vec::new(),
        };
        let exact_frame = frame_at(0.0, 0.0);
        let exact = HashMap::from([
            (10, (DMat3::IDENTITY, DVec3::ZERO)),
            (20, (DMat3::IDENTITY, DVec3::new(-2.0, 0.0, 0.0))),
        ]);
        let reconstructed = HashMap::from([
            (10, (DMat3::IDENTITY, DVec3::new(0.0, 0.0, -1.0))),
            (20, (DMat3::IDENTITY, DVec3::new(-2.0, 0.0, 1.0))),
        ]);

        let generic = derive_surface_order(
            &cp,
            &faces,
            &[reconstructed],
            &exact,
            &exact_frame,
            &[],
            None,
        )
        .expect("synthetic flat reconstruction is valid");
        let actual = derive_surface_order_from_frame_path_with_previous(
            &cp,
            &faces,
            &[frame_at(1.0, -1.0)],
            &exact_frame,
            &[],
            None,
        )
        .expect("synthetic current-step path is valid");

        assert!(generic.complete && actual.complete);
        assert_eq!(generic.order, vec![10, 20]);
        assert_eq!(actual.order, vec![20, 10]);

        let dropped = derive_surface_order_from_frame_path(
            &cp,
            &faces,
            &[frame_at(1.0, -1.0)],
            &exact_frame,
            &[(10, 20)],
        )
        .expect("exactと逆向きの実深度も診断できる");
        assert_eq!(dropped.dropped_depth_constraints, 1);
        assert_eq!(
            dropped.sampled_depth_constraints, 0,
            "exactと逆で捨てた深度をloop経路authorityへ数えない"
        );
        assert_eq!(dropped.order, vec![10, 20]);
    }

    #[test]
    fn transitive_exact_order_drops_reverse_depth_before_it_forms_a_cycle() {
        let cp = material_cp(&[
            (0, [0.0, 0.0]),
            (1, [1.0, 0.0]),
            (2, [0.0, 1.0]),
            (3, [2.0, 0.0]),
            (4, [3.0, 0.0]),
            (5, [2.0, 1.0]),
            (6, [4.0, 0.0]),
            (7, [5.0, 0.0]),
            (8, [4.0, 1.0]),
        ]);
        let faces = vec![
            face(10, &[0, 1, 2]),
            face(20, &[3, 4, 5]),
            face(30, &[6, 7, 8]),
        ];
        let polygon_at = |z| vec![[0.0, 0.0, z], [1.0, 0.0, z], [0.0, 1.0, z]];
        let frame_at = |heights: [f64; 3]| Frame3D {
            faces: faces
                .iter()
                .zip(heights)
                .map(|(face, z)| Face3D {
                    face: face.id,
                    polygon: polygon_at(z),
                    layer: 0,
                    surface_rank: 0,
                    mirrored: false,
                })
                .collect(),
            warnings: Vec::new(),
        };
        let exact_frame = frame_at([0.0, 0.0, 0.0]);
        let reverse_path = frame_at([1.0, 0.0, -1.0]);

        let derived = derive_surface_order_from_frame_path(
            &cp,
            &faces,
            &[reverse_path],
            &exact_frame,
            &[(10, 20), (20, 30)],
        )
        .expect("推移的exact chainと逆向きの深度も診断できる");

        assert!(derived.complete);
        assert_eq!(derived.order, vec![10, 20, 30]);
        assert_eq!(derived.resolved_overlaps, 3);
        assert_eq!(derived.broken_constraints, 0);
        assert_eq!(derived.dropped_depth_constraints, 3);
        assert_eq!(derived.sampled_depth_constraints, 0);
    }

    #[test]
    fn signed_endpoint_match_keeps_plus_and_minus_180_distinct() {
        let plus = HashMap::from([(12, 180.0)]);
        let minus = HashMap::from([(12, -180.0)]);

        assert!(signed_angles_match(&plus, &plus));
        assert!(!signed_angles_match(&plus, &minus));
        assert!(!signed_angles_match(&minus, &plus));
    }
}
