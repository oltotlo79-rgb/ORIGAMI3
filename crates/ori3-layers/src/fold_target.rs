//! 「選んだ折り線の直下にあるひだ」を数えるための製品API骨格。
//!
//! 段階2では、書類から再現した符号付き角度と上からの重なり順だけを純粋に解析する。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use glam::DVec2;
use ori3_cp::Face;
use ori3_model::{CreasePattern, EPS, EdgeId, FaceId};

use crate::flat_state::FlatState;
use crate::folded_query::FoldedQuery;

/// 完全折りの終端を識別するときだけ使う許容差。
///
/// 利用者が指定した根拠は次のとおりである。
/// 1. ひだは、表面紙と裏紙が +180° または -180°まで完全に折られた組だけである。
/// 2. 実機で観測した終端誤差の最大は 1.0342061893570844e-13°、書類だけから
///    再現したときの最大は 4.1702290600902744e-13°だった。
/// 3. 保存済み6作品の292角は、すべて +180° / 0° / -180°の正確な値だった。
/// 4. 画面で確定する角度は1°刻みであり、179°は完全終端から1°離れた未完の指定である。
/// 5. 未完をひだと誤認する偽陽性は、重なっていない紙を動かして形の破綻や食い込みを
///    作るため、判定は広げず狭く保たなければならない。
/// 6. +180°と -180°は別の符号として保持し、絶対値や周期化で同一視しない。
///
/// `1e-9` は実測の8割を閾値にした値ではない。実測誤差とは独立して存在する
/// 「終端180°と未完179°の1°差」に対して十分狭く、かつ上の実測誤差を収める境界
/// として採用する。この値を広げて179°を終端へ丸めたり、角度へ `abs` や周期化を
/// 適用したりしてはならない。
pub const COMPLETE_FOLD_ENDPOINT_EPS_DEG: f64 = 1.0e-9;

/// 完全折りを作った符号付き終端。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullFoldSign {
    Positive180,
    Negative180,
}

/// 書類から再現した現在Edgeの、隣り合うFaceと符号付き角度。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HingeObservation {
    pub face_a: FaceId,
    pub face_b: FaceId,
    pub angle_deg: f64,
}

/// 新しい折り線を、その直下の重なりが変わらない区間に分けた入力。
#[derive(Clone, Debug, PartialEq)]
pub struct FoldLineSection {
    /// 新しい折り線から可動側へずらした内部点を覆う面。上から下の順。
    pub faces_top_to_bottom: Vec<FaceId>,
    /// 書類から再現した現在Edgeの符号付き角度。
    pub hinges: Vec<HingeObservation>,
}

/// 0°で連続する表面群どうしが完全折りで組になった1ひだ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PleatPair {
    /// 完全折りを担う、上側と下側の代表Face。
    pub hinge_faces: (FaceId, FaceId),
    /// 0°で連続する上側表面を構成するFace。上からの決定順を保つ。
    pub upper_surface_faces: Vec<FaceId>,
    /// 0°で連続する下側表面を構成するFace。上からの決定順を保つ。
    pub lower_surface_faces: Vec<FaceId>,
    pub sign: FullFoldSign,
}

/// 最上紙が完全なひだを作らないときに、この折り操作で行うこと。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TopAction {
    CreaseOnlyTop { surface_faces: Vec<FaceId> },
}

/// 上から数える処理を途中で止めた理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PleatCountLimit {
    IncompleteBoundaryAfter { count: usize },
}

/// 新しい折り線のうち、重なりが一定な1区間の解析結果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PleatSectionAnalysis {
    /// 上から貪欲に確定した、重複しない表裏の組。
    pub pairs_top_to_bottom: Vec<PleatPair>,
    /// 連続するひだpairの間で確認した完全折りの符号。
    pub boundary_signs_between_pairs: Vec<FullFoldSign>,
    /// 最上紙が完全な組を作らない場合の処置。
    pub top_action: Option<TopAction>,
    /// 最初の未完な境目で打ち切った場合の理由と、その直前までの枚数。
    pub count_limit: Option<PleatCountLimit>,
}

/// 新しい折り線の直下にあるひだの全区間解析結果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PleatAnalysis {
    /// 全区間で同じ1値にできない場合はNone。
    pub scalar_count: Option<usize>,
    /// 新しい折り線を区切り、上からの順序と打切り理由を区間別に保った結果。
    pub sections: Vec<PleatSectionAnalysis>,
    /// 1値にできない場合に画面へ返す日本語の理由。
    pub reason: Option<String>,
}

/// Pleat analysis plus the complete ordered surface membership of every
/// geometric interval crossed by the requested new fold line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldTargetAnalysis {
    pub pleats: PleatAnalysis,
    pub section_surfaces_top_to_bottom: Vec<Vec<Vec<FaceId>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PleatAnalysisError {
    NotImplemented,
    /// The same two folded surfaces were joined by observations that did not
    /// all describe the same signed complete fold.
    AmbiguousRelation,
    InvalidGeometry,
    NoSections,
    RepeatedSurfaceInSection,
    InvalidPleatCount,
    UnsafeWholeFaceSelection,
}

/// Build fold-line sections from one canonical, document-derived flat state.
///
/// `hinge_angles` contains the signed declared angle for every material hinge.
/// It is deliberately separate from any live 3D frame or solver warm state.
pub fn analyze_fold_target_at_state(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    hinge_angles: &HashMap<EdgeId, f64>,
    line: [[f64; 2]; 2],
    keep_side_point: [f64; 2],
) -> Result<FoldTargetAnalysis, PleatAnalysisError> {
    let observations = hinge_observations(faces, hinge_angles)?;
    validate_fragment_observations(&observations, COMPLETE_FOLD_ENDPOINT_EPS_DEG)?;
    let zero_roots = zero_surface_roots(faces, &observations);
    validate_surface_observations(&observations, COMPLETE_FOLD_ENDPOINT_EPS_DEG, |face| {
        zero_roots.get(&face).copied()
    })?;
    let sections = fold_line_sections(cp, faces, state, &observations, line, keep_side_point)?;
    if sections.is_empty() {
        return Err(PleatAnalysisError::NoSections);
    }

    let section_surfaces_top_to_bottom = sections.iter().map(surface_groups).collect::<Vec<_>>();
    let pleats = analyze_pleats(&sections, COMPLETE_FOLD_ENDPOINT_EPS_DEG)?;
    Ok(FoldTargetAnalysis {
        pleats,
        section_surfaces_top_to_bottom,
    })
}

/// Resolve a one-based pleat count to the whole Face set that the existing
/// fold engine accepts. The conversion remains internal to Rust.
pub fn target_faces_for_pleat_count(
    analysis: &FoldTargetAnalysis,
    count: usize,
) -> Result<Vec<FaceId>, PleatAnalysisError> {
    let Some(available) = analysis.pleats.scalar_count else {
        return Err(PleatAnalysisError::InvalidPleatCount);
    };
    if count == 0 || count > available {
        return Err(PleatAnalysisError::InvalidPleatCount);
    }
    if analysis.pleats.sections.len() != analysis.section_surfaces_top_to_bottom.len() {
        return Err(PleatAnalysisError::UnsafeWholeFaceSelection);
    }
    let Some(first_section) = analysis.pleats.sections.first() else {
        return Err(PleatAnalysisError::UnsafeWholeFaceSelection);
    };
    if analysis
        .pleats
        .sections
        .iter()
        .skip(1)
        .any(|section| !same_pleat_pair_identity(first_section, section))
    {
        return Err(PleatAnalysisError::UnsafeWholeFaceSelection);
    }

    let selected_by_section = analysis
        .pleats
        .sections
        .iter()
        .map(|section| {
            if section.pairs_top_to_bottom.len() < count {
                return Err(PleatAnalysisError::InvalidPleatCount);
            }
            Ok(section
                .pairs_top_to_bottom
                .iter()
                .take(count)
                .flat_map(|pair| {
                    pair.upper_surface_faces
                        .iter()
                        .chain(&pair.lower_surface_faces)
                        .copied()
                })
                .collect::<BTreeSet<_>>())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let selected_union = selected_by_section
        .iter()
        .flat_map(|selected| selected.iter().copied())
        .collect::<BTreeSet<_>>();

    for (selected, surfaces) in selected_by_section
        .iter()
        .zip(&analysis.section_surfaces_top_to_bottom)
    {
        let faces_in_section = surfaces
            .iter()
            .flat_map(|surface| surface.iter().copied())
            .collect::<BTreeSet<_>>();
        let whole_face_effect = selected_union
            .intersection(&faces_in_section)
            .copied()
            .collect::<BTreeSet<_>>();
        if &whole_face_effect != selected {
            return Err(PleatAnalysisError::UnsafeWholeFaceSelection);
        }
    }

    Ok(selected_union.into_iter().collect())
}

fn same_pleat_pair_identity(first: &PleatSectionAnalysis, second: &PleatSectionAnalysis) -> bool {
    first.boundary_signs_between_pairs == second.boundary_signs_between_pairs
        && first.pairs_top_to_bottom.len() == second.pairs_top_to_bottom.len()
        && first
            .pairs_top_to_bottom
            .iter()
            .zip(&second.pairs_top_to_bottom)
            .all(|(left, right)| {
                left.sign == right.sign
                    && left
                        .upper_surface_faces
                        .iter()
                        .copied()
                        .collect::<BTreeSet<_>>()
                        == right
                            .upper_surface_faces
                            .iter()
                            .copied()
                            .collect::<BTreeSet<_>>()
                    && left
                        .lower_surface_faces
                        .iter()
                        .copied()
                        .collect::<BTreeSet<_>>()
                        == right
                            .lower_surface_faces
                            .iter()
                            .copied()
                            .collect::<BTreeSet<_>>()
            })
}

#[derive(Clone, Copy, Debug)]
struct FaceLineInterval {
    face: FaceId,
    start: f64,
    end: f64,
}

fn hinge_observations(
    faces: &[Face],
    hinge_angles: &HashMap<EdgeId, f64>,
) -> Result<Vec<HingeObservation>, PleatAnalysisError> {
    let mut owners = BTreeMap::<EdgeId, BTreeSet<FaceId>>::new();
    for face in faces {
        for &edge in &face.edges {
            owners.entry(edge).or_default().insert(face.id);
        }
    }

    let mut observations = Vec::new();
    for (edge, owners) in owners {
        if owners.len() < 2 {
            continue;
        }
        if owners.len() != 2 {
            return Err(PleatAnalysisError::InvalidGeometry);
        }
        let angle_deg = hinge_angles
            .get(&edge)
            .copied()
            .ok_or(PleatAnalysisError::InvalidGeometry)?;
        let mut owners = owners.into_iter();
        observations.push(HingeObservation {
            face_a: owners.next().expect("two owners"),
            face_b: owners.next().expect("two owners"),
            angle_deg,
        });
    }
    Ok(observations)
}

fn fold_line_sections(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    observations: &[HingeObservation],
    line: [[f64; 2]; 2],
    keep_side_point: [f64; 2],
) -> Result<Vec<FoldLineSection>, PleatAnalysisError> {
    let line_start = DVec2::from(line[0]);
    let line_end = DVec2::from(line[1]);
    let keep = DVec2::from(keep_side_point);
    if !line_start.is_finite() || !line_end.is_finite() || !keep.is_finite() {
        return Err(PleatAnalysisError::InvalidGeometry);
    }
    let delta = line_end - line_start;
    if delta.length() < EPS {
        return Err(PleatAnalysisError::InvalidGeometry);
    }
    let direction = delta.normalize();
    let keep_side = direction.perp_dot(keep - line_start);
    if keep_side.abs() <= EPS {
        return Err(PleatAnalysisError::InvalidGeometry);
    }
    let moving_sign = -keep_side.signum();
    let query =
        FoldedQuery::new(cp, faces, state).map_err(|_| PleatAnalysisError::InvalidGeometry)?;

    let mut intervals = Vec::new();
    for geometry in query.face_geometries() {
        let polygon = geometry
            .polygon
            .iter()
            .copied()
            .map(DVec2::from)
            .collect::<Vec<_>>();
        intervals.extend(intervals_on_moving_side(
            geometry.face_id,
            &polygon,
            line_start,
            direction,
            moving_sign,
        ));
    }
    if intervals.is_empty() {
        return Err(PleatAnalysisError::NoSections);
    }

    let mut endpoints = intervals
        .iter()
        .flat_map(|interval| [interval.start, interval.end])
        .collect::<Vec<_>>();
    endpoints.sort_by(f64::total_cmp);
    endpoints.dedup_by(|left, right| (*left - *right).abs() <= EPS);

    let zero_roots = zero_surface_roots(faces, observations);
    let mut sections = Vec::new();
    let mut previous_window_had_paper = false;
    for window in endpoints.windows(2) {
        if window[1] - window[0] <= EPS {
            continue;
        }
        let midpoint = (window[0] + window[1]) * 0.5;
        let active = intervals
            .iter()
            .filter(|interval| midpoint > interval.start - EPS && midpoint < interval.end + EPS)
            .map(|interval| interval.face)
            .collect::<HashSet<_>>();
        if active.is_empty() {
            previous_window_had_paper = false;
            continue;
        }
        let faces_top_to_bottom = state
            .order
            .iter()
            .rev()
            .copied()
            .filter(|face| active.contains(face))
            .collect::<Vec<_>>();
        if faces_top_to_bottom.is_empty() {
            previous_window_had_paper = false;
            continue;
        }
        let mut seen_surfaces = HashSet::new();
        for face in &faces_top_to_bottom {
            let root = zero_roots
                .get(face)
                .copied()
                .ok_or(PleatAnalysisError::InvalidGeometry)?;
            if !seen_surfaces.insert(root) {
                return Err(PleatAnalysisError::RepeatedSurfaceInSection);
            }
        }
        if previous_window_had_paper
            && sections.last().is_some_and(|previous: &FoldLineSection| {
                previous.faces_top_to_bottom == faces_top_to_bottom
            })
        {
            continue;
        }
        sections.push(FoldLineSection {
            faces_top_to_bottom,
            hinges: observations.to_vec(),
        });
        previous_window_had_paper = true;
    }
    Ok(sections)
}

fn intervals_on_moving_side(
    face: FaceId,
    polygon: &[DVec2],
    line_start: DVec2,
    direction: DVec2,
    moving_sign: f64,
) -> Vec<FaceLineInterval> {
    if polygon.len() < 3 {
        return Vec::new();
    }
    let t_of = |point: DVec2| (point - line_start).dot(direction);
    let mut endpoints = Vec::new();
    for index in 0..polygon.len() {
        let first = polygon[index];
        let second = polygon[(index + 1) % polygon.len()];
        let first_signed = direction.perp_dot(first - line_start);
        let second_signed = direction.perp_dot(second - line_start);
        if first_signed.abs() <= EPS && second_signed.abs() <= EPS {
            endpoints.push(t_of(first));
            endpoints.push(t_of(second));
        } else if first_signed.abs() <= EPS {
            endpoints.push(t_of(first));
        } else if second_signed.abs() <= EPS {
            endpoints.push(t_of(second));
        } else if first_signed * second_signed < 0.0 {
            let crossing =
                first + (second - first) * (first_signed / (first_signed - second_signed));
            endpoints.push(t_of(crossing));
        }
    }
    endpoints.sort_by(f64::total_cmp);
    endpoints.dedup_by(|left, right| (*left - *right).abs() <= EPS);

    // FoldThrough moves only the strict half-plane farther than EPS. Probe the
    // same side at 2*EPS instead of clipping the concave polygon: clipping can
    // create an artificial line segment across a disconnected U-shaped gap.
    let moving_normal = DVec2::new(-direction.y, direction.x) * moving_sign;
    let mut intervals =
        endpoints
            .windows(2)
            .filter_map(|window| {
                let (start, end) = (window[0], window[1]);
                if end - start <= EPS {
                    return None;
                }
                let midpoint = line_start + direction * ((start + end) * 0.5);
                let moving_probe = midpoint + moving_normal * (2.0 * EPS);
                crate::flat_state::point_in_polygon(polygon, moving_probe)
                    .then_some(FaceLineInterval { face, start, end })
            })
            .collect::<Vec<_>>();
    intervals.sort_by(|left, right| {
        left.start
            .total_cmp(&right.start)
            .then_with(|| left.end.total_cmp(&right.end))
    });
    let mut merged: Vec<FaceLineInterval> = Vec::new();
    for interval in intervals {
        if let Some(previous) = merged.last_mut()
            && interval.start <= previous.end + EPS
        {
            previous.end = previous.end.max(interval.end);
        } else {
            merged.push(interval);
        }
    }
    merged
}

fn zero_surface_roots(
    faces: &[Face],
    observations: &[HingeObservation],
) -> HashMap<FaceId, FaceId> {
    let ids = faces.iter().map(|face| face.id).collect::<Vec<_>>();
    zero_surface_roots_for_ids(&ids, observations)
}

fn zero_surface_roots_for_ids(
    ids: &[FaceId],
    observations: &[HingeObservation],
) -> HashMap<FaceId, FaceId> {
    let mut parent = ids
        .iter()
        .copied()
        .map(|face| (face, face))
        .collect::<HashMap<_, _>>();
    for observation in observations {
        if observation.angle_deg == 0.0 {
            union_face_roots(&mut parent, observation.face_a, observation.face_b);
        }
    }
    let all_ids = parent.keys().copied().collect::<Vec<_>>();
    all_ids
        .into_iter()
        .map(|face| {
            let root = find_face_root(&mut parent, face);
            (face, root)
        })
        .collect()
}

fn find_face_root(parent: &mut HashMap<FaceId, FaceId>, face: FaceId) -> FaceId {
    let mut root = face;
    while parent.get(&root).copied().is_some_and(|next| next != root) {
        root = parent[&root];
    }
    let mut current = face;
    while parent
        .get(&current)
        .copied()
        .is_some_and(|next| next != root)
    {
        let next = parent[&current];
        parent.insert(current, root);
        current = next;
    }
    root
}

fn union_face_roots(parent: &mut HashMap<FaceId, FaceId>, left: FaceId, right: FaceId) {
    let left_root = find_face_root(parent, left);
    let right_root = find_face_root(parent, right);
    if left_root != right_root {
        let (root, child) = if left_root < right_root {
            (left_root, right_root)
        } else {
            (right_root, left_root)
        };
        parent.insert(child, root);
    }
}

#[derive(Clone, Copy, Debug)]
struct ObservedRelation {
    upper_face: FaceId,
    lower_face: FaceId,
    angle_deg: f64,
}

fn classify_complete_fold(angle_deg: f64, endpoint_epsilon_deg: f64) -> Option<FullFoldSign> {
    if !angle_deg.is_finite() || !endpoint_epsilon_deg.is_finite() || endpoint_epsilon_deg < 0.0 {
        return None;
    }

    let positive_delta = angle_deg - 180.0;
    if positive_delta >= -endpoint_epsilon_deg && positive_delta <= endpoint_epsilon_deg {
        return Some(FullFoldSign::Positive180);
    }

    let negative_delta = angle_deg + 180.0;
    if negative_delta >= -endpoint_epsilon_deg && negative_delta <= endpoint_epsilon_deg {
        return Some(FullFoldSign::Negative180);
    }

    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FragmentClassification {
    Zero,
    Complete(FullFoldSign),
    Incomplete,
}

fn classify_fragment(angle_deg: f64, endpoint_epsilon_deg: f64) -> FragmentClassification {
    if angle_deg == 0.0 {
        FragmentClassification::Zero
    } else if let Some(sign) = classify_complete_fold(angle_deg, endpoint_epsilon_deg) {
        FragmentClassification::Complete(sign)
    } else {
        FragmentClassification::Incomplete
    }
}

fn validate_fragment_observations(
    observations: &[HingeObservation],
    endpoint_epsilon_deg: f64,
) -> Result<(), PleatAnalysisError> {
    let mut by_face_pair = BTreeMap::<(FaceId, FaceId), Vec<FragmentClassification>>::new();
    for observation in observations {
        let key = if observation.face_a <= observation.face_b {
            (observation.face_a, observation.face_b)
        } else {
            (observation.face_b, observation.face_a)
        };
        by_face_pair.entry(key).or_default().push(classify_fragment(
            observation.angle_deg,
            endpoint_epsilon_deg,
        ));
    }
    for classifications in by_face_pair.values().filter(|values| values.len() > 1) {
        if !fragment_classifications_agree(classifications) {
            return Err(PleatAnalysisError::AmbiguousRelation);
        }
    }
    Ok(())
}

fn fragment_classifications_agree(classifications: &[FragmentClassification]) -> bool {
    let Some(&first) = classifications.first() else {
        return true;
    };
    first != FragmentClassification::Incomplete
        && classifications.iter().all(|&value| value == first)
}

fn validate_surface_observations<K, F>(
    observations: &[HingeObservation],
    endpoint_epsilon_deg: f64,
    mut surface_of: F,
) -> Result<(), PleatAnalysisError>
where
    K: Copy + Ord,
    F: FnMut(FaceId) -> Option<K>,
{
    let mut by_surface_pair = BTreeMap::<(K, K), Vec<FragmentClassification>>::new();
    for observation in observations {
        let first = surface_of(observation.face_a).ok_or(PleatAnalysisError::InvalidGeometry)?;
        let second = surface_of(observation.face_b).ok_or(PleatAnalysisError::InvalidGeometry)?;
        if first == second {
            if observation.angle_deg != 0.0 {
                return Err(PleatAnalysisError::AmbiguousRelation);
            }
            continue;
        }
        let key = if first <= second {
            (first, second)
        } else {
            (second, first)
        };
        by_surface_pair
            .entry(key)
            .or_default()
            .push(classify_fragment(
                observation.angle_deg,
                endpoint_epsilon_deg,
            ));
    }
    for classifications in by_surface_pair.values().filter(|values| values.len() > 1) {
        if !fragment_classifications_agree(classifications) {
            return Err(PleatAnalysisError::AmbiguousRelation);
        }
    }
    Ok(())
}

fn find_root(parent: &mut [usize], index: usize) -> usize {
    let mut root = index;
    while parent[root] != root {
        root = parent[root];
    }

    let mut current = index;
    while parent[current] != current {
        let next = parent[current];
        parent[current] = root;
        current = next;
    }
    root
}

fn union_roots(parent: &mut [usize], left: usize, right: usize) {
    let left_root = find_root(parent, left);
    let right_root = find_root(parent, right);
    if left_root != right_root {
        parent[right_root] = left_root;
    }
}

fn surface_groups(section: &FoldLineSection) -> Vec<Vec<FaceId>> {
    let mut all_faces = Vec::new();
    let mut seen_faces = HashSet::new();
    for &face in &section.faces_top_to_bottom {
        if seen_faces.insert(face) {
            all_faces.push(face);
        }
    }
    for hinge in &section.hinges {
        for face in [hinge.face_a, hinge.face_b] {
            if seen_faces.insert(face) {
                all_faces.push(face);
            }
        }
    }

    let face_indices: HashMap<FaceId, usize> = all_faces
        .iter()
        .copied()
        .enumerate()
        .map(|(index, face)| (face, index))
        .collect();
    let mut parent: Vec<usize> = (0..all_faces.len()).collect();

    for hinge in &section.hinges {
        if hinge.angle_deg == 0.0 {
            let Some(&left) = face_indices.get(&hinge.face_a) else {
                continue;
            };
            let Some(&right) = face_indices.get(&hinge.face_b) else {
                continue;
            };
            union_roots(&mut parent, left, right);
        }
    }

    let mut members_by_root: HashMap<usize, Vec<FaceId>> = HashMap::new();
    for (index, face) in all_faces.iter().copied().enumerate() {
        let root = find_root(&mut parent, index);
        members_by_root.entry(root).or_default().push(face);
    }

    let mut ordered_groups = Vec::new();
    let mut seen_roots = HashSet::new();
    for &face in &section.faces_top_to_bottom {
        let Some(&index) = face_indices.get(&face) else {
            continue;
        };
        let root = find_root(&mut parent, index);
        if seen_roots.insert(root) {
            ordered_groups.push(members_by_root.get(&root).cloned().unwrap_or_default());
        }
    }
    ordered_groups
}

fn relation_between_groups(
    upper_surface: &[FaceId],
    lower_surface: &[FaceId],
    hinges: &[HingeObservation],
    endpoint_epsilon_deg: f64,
) -> Result<Option<ObservedRelation>, PleatAnalysisError> {
    let mut matching = hinges.iter().filter_map(|hinge| {
        if upper_surface.contains(&hinge.face_a) && lower_surface.contains(&hinge.face_b) {
            Some(ObservedRelation {
                upper_face: hinge.face_a,
                lower_face: hinge.face_b,
                angle_deg: hinge.angle_deg,
            })
        } else if upper_surface.contains(&hinge.face_b) && lower_surface.contains(&hinge.face_a) {
            Some(ObservedRelation {
                upper_face: hinge.face_b,
                lower_face: hinge.face_a,
                angle_deg: hinge.angle_deg,
            })
        } else {
            None
        }
    });
    let Some(first) = matching.next() else {
        return Ok(None);
    };
    let rest = matching.collect::<Vec<_>>();
    if rest.is_empty() {
        return Ok(Some(first));
    }
    let Some(first_sign) = classify_complete_fold(first.angle_deg, endpoint_epsilon_deg) else {
        return Err(PleatAnalysisError::AmbiguousRelation);
    };
    if rest.iter().any(|relation| {
        classify_complete_fold(relation.angle_deg, endpoint_epsilon_deg) != Some(first_sign)
    }) {
        return Err(PleatAnalysisError::AmbiguousRelation);
    }
    let representative = rest.into_iter().fold(first, |best, candidate| {
        if (candidate.upper_face, candidate.lower_face) < (best.upper_face, best.lower_face) {
            candidate
        } else {
            best
        }
    });
    Ok(Some(representative))
}

fn pleat_pair(
    upper_surface: &[FaceId],
    lower_surface: &[FaceId],
    relation: ObservedRelation,
    sign: FullFoldSign,
) -> PleatPair {
    PleatPair {
        hinge_faces: (relation.upper_face, relation.lower_face),
        upper_surface_faces: upper_surface.to_vec(),
        lower_surface_faces: lower_surface.to_vec(),
        sign,
    }
}

fn analyze_ordered_surfaces<F>(
    surfaces_top_to_bottom: &[Vec<FaceId>],
    endpoint_epsilon_deg: f64,
    mut relation: F,
) -> Result<PleatSectionAnalysis, PleatAnalysisError>
where
    F: FnMut(&[FaceId], &[FaceId]) -> Result<Option<ObservedRelation>, PleatAnalysisError>,
{
    let mut analysis = PleatSectionAnalysis::default();
    let Some(top_surface) = surfaces_top_to_bottom.first() else {
        return Ok(analysis);
    };
    let Some(second_surface) = surfaces_top_to_bottom.get(1) else {
        return Ok(analysis);
    };

    let first_relation = relation(top_surface, second_surface)?;
    let first_complete = first_relation.and_then(|observed| {
        classify_complete_fold(observed.angle_deg, endpoint_epsilon_deg)
            .map(|sign| (observed, sign))
    });
    let Some((observed, sign)) = first_complete else {
        analysis.top_action = Some(TopAction::CreaseOnlyTop {
            surface_faces: top_surface.clone(),
        });
        return Ok(analysis);
    };
    analysis
        .pairs_top_to_bottom
        .push(pleat_pair(top_surface, second_surface, observed, sign));

    let mut next_pair_top = 2;
    while next_pair_top < surfaces_top_to_bottom.len() {
        let previous_lower = &surfaces_top_to_bottom[next_pair_top - 1];
        let next_upper = &surfaces_top_to_bottom[next_pair_top];
        let connector = relation(previous_lower, next_upper)?;
        let connector_sign = connector
            .and_then(|observed| classify_complete_fold(observed.angle_deg, endpoint_epsilon_deg));
        let Some(connector_sign) = connector_sign else {
            analysis.count_limit = Some(PleatCountLimit::IncompleteBoundaryAfter {
                count: analysis.pairs_top_to_bottom.len(),
            });
            break;
        };

        let Some(next_lower) = surfaces_top_to_bottom.get(next_pair_top + 1) else {
            break;
        };
        let next_relation = relation(next_upper, next_lower)?;
        let next_complete = next_relation.and_then(|observed| {
            classify_complete_fold(observed.angle_deg, endpoint_epsilon_deg)
                .map(|sign| (observed, sign))
        });
        let Some((observed, sign)) = next_complete else {
            analysis.count_limit = Some(PleatCountLimit::IncompleteBoundaryAfter {
                count: analysis.pairs_top_to_bottom.len(),
            });
            break;
        };

        analysis.boundary_signs_between_pairs.push(connector_sign);
        analysis
            .pairs_top_to_bottom
            .push(pleat_pair(next_upper, next_lower, observed, sign));
        next_pair_top += 2;
    }

    Ok(analysis)
}

fn combine_sections(sections: Vec<PleatSectionAnalysis>) -> PleatAnalysis {
    let first_count = sections
        .first()
        .map_or(0, |section| section.pairs_top_to_bottom.len());
    let all_counts_match = sections
        .iter()
        .all(|section| section.pairs_top_to_bottom.len() == first_count);

    PleatAnalysis {
        scalar_count: all_counts_match.then_some(first_count),
        reason: (!all_counts_match)
            .then(|| "折り線の場所によって、同時に折れるひだの枚数が異なります".to_owned()),
        sections,
    }
}

/// 区間ごとの入力からひだを解析する。
///
/// 利用者決定により、組は上から貪欲に調べる。1つの表面を2組へ再利用せず、
/// 完全な組を見つけたら、その表裏2つを消費して次の紙へ進む。
/// 上から連続して調べ、ひだpairどうしの境目も同じ許容差で完全折りと確認できた
/// ときだけ次へ進む。+180°と -180°は別の符号として比較・保持する。
/// 最初の未完で打ち切り、それより下の関係は問い合わせない。
pub fn analyze_pleats(
    sections: &[FoldLineSection],
    endpoint_epsilon_deg: f64,
) -> Result<PleatAnalysis, PleatAnalysisError> {
    for section in sections {
        validate_fragment_observations(&section.hinges, endpoint_epsilon_deg)?;
    }
    let analyses = sections
        .iter()
        .map(|section| {
            let groups = surface_groups(section);
            let all_faces = section
                .faces_top_to_bottom
                .iter()
                .copied()
                .chain(
                    section
                        .hinges
                        .iter()
                        .flat_map(|hinge| [hinge.face_a, hinge.face_b]),
                )
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let surface_roots = zero_surface_roots_for_ids(&all_faces, &section.hinges);
            validate_surface_observations(&section.hinges, endpoint_epsilon_deg, |face| {
                surface_roots.get(&face).copied()
            })?;
            analyze_ordered_surfaces(&groups, endpoint_epsilon_deg, |upper, lower| {
                relation_between_groups(upper, lower, &section.hinges, endpoint_epsilon_deg)
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(combine_sections(analyses))
}

/// 単一区間を上から必要なところまで問い合わせて解析する。
///
/// 最上紙が完全な組を作らない場合は、その紙へ折り目だけを付けて探索を止め、
/// 下に完全な組が残っていても、この操作では問い合わせず、数えない。
pub fn analyze_single_section_from_top<F>(
    faces_top_to_bottom: &[FaceId],
    endpoint_epsilon_deg: f64,
    mut relation: F,
) -> Result<PleatAnalysis, PleatAnalysisError>
where
    F: FnMut(FaceId, FaceId) -> Option<f64>,
{
    let surfaces: Vec<Vec<FaceId>> = faces_top_to_bottom
        .iter()
        .copied()
        .map(|face| vec![face])
        .collect();
    let analysis = analyze_ordered_surfaces(&surfaces, endpoint_epsilon_deg, |upper, lower| {
        let upper_face = upper[0];
        let lower_face = lower[0];
        Ok(
            relation(upper_face, lower_face).map(|angle_deg| ObservedRelation {
                upper_face,
                lower_face,
                angle_deg,
            }),
        )
    })?;
    Ok(combine_sections(vec![analysis]))
}
