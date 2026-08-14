//! めり込み診断と追従補正(SIM-007/SIM-016)のため、立体表示の面同士が
//! 突き抜けていないかを調べる。
//!
//! 代表接触点と侵入量を追従計算へ渡し、他の折り目を譲らせる目的に使う。
//! 完全に避けられない場合も操作は止めず、最小侵入の有限形と警告を返す。
//!
//! 判定の方針:
//! - 面を扇状に三角形へ割り、三角形の辺が相手の三角形の内部を貫くかを見る。
//!   同一平面上で重なっているだけの場合は「めり込み」としない(平らに畳んだ紙の
//!   層は必ず同一平面に重なるため。厚みは表示側のずらしで表現する)。
//! - 折り目でつながった面(頂点を2つ共有する面)は調べない。つながった2面は
//!   折り目のところで必ず接しており、完全に折り畳むと同一平面で重なるのが正常。
//! - 面数400を想定し、まず外接直方体(AABB)で組を絞ってから三角形を調べる。
//!
//! 凸でない面では扇分割の三角形が面の外へはみ出すため、まれに実際より広く
//! 見積もる(警告を出しすぎる=安全側)。

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::time::{Duration, Instant};

use glam::{DVec2, DVec3};
use ori3_cp::Face;
use ori3_model::{CreasePattern, EdgeId, EdgeKind, FaceId, Frame3D, VertexId};

/// めり込みを見つけたときの警告文(3D表示のバッジに出る)
pub const PENETRATION_WARNING: &str = "紙が重なって食い込んでいます";

/// 幾何の許容誤差(正規化座標。長辺=1.0)。接している程度は貫通としない。
const TOL: f64 = 1e-6;

/// 追従計算へ渡す接触候補の上限。
///
/// 全交差面ペアの数と侵入量は上限を掛けず集計し、線形補正に使う代表だけを
/// 深い順に絞る。面数が増えても接触補正の列数を決定的に抑えるための上限である。
pub const MAX_CONTACT_WITNESSES: usize = 32;

/// 交差する面ペアを離す方向と、交差面から浅い側の端点までの侵入深さ。
///
/// `normal` は線分が横切った三角形の頂点順に従う単位法線である。どちらを上層に
/// するかは層順序または直前の非交差姿勢で呼び出し側が決める。`penetration_depth`
/// は紙の長辺を1とした正規化座標で、接触許容内は0になる。
#[derive(Clone, Debug, PartialEq)]
pub struct ContactWitness {
    /// 小さいFaceId、大きいFaceIdの順。
    pub faces: (FaceId, FaceId),
    pub point: [f64; 3],
    pub normal: [f64; 3],
    pub penetration_depth: f64,
}

/// 全交差面ペアの接触指標。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContactMetrics {
    /// witness上限を掛ける前の全交差面ペア数。
    pub pair_count: usize,
    /// 全交差面ペアそれぞれの最深witnessの最大値。
    pub max_penetration: f64,
    /// 全交差面ペアそれぞれの最深witnessの合計値。
    pub total_penetration: f64,
}

/// 性能回帰テストで接触走査の段階別時間と候補数を測るための診断値。
/// 製品経路とは別に同じ絞り込みと狭域判定を1回だけ実行する。
#[doc(hidden)]
#[derive(Clone, Debug, Default)]
pub struct ContactScanProfile {
    pub flat_check_time: Duration,
    pub build_parts_time: Duration,
    pub broad_phase_time: Duration,
    pub narrow_phase_time: Duration,
    pub face_pairs: usize,
    pub aabb_overlaps: usize,
    pub shared_edge_pairs: usize,
    pub narrow_phase_pairs: usize,
    pub triangle_pair_tests: usize,
    pub segment_triangle_tests: usize,
    pub intersection_pairs: usize,
}

/// [`ContactScanProfile`] を取得する。時間制限の対象外で呼び出す診断専用API。
#[doc(hidden)]
#[must_use]
pub fn contact_scan_profile(frame: &Frame3D) -> ContactScanProfile {
    let mut profile = ContactScanProfile::default();

    let started = Instant::now();
    let flat = frame
        .faces
        .iter()
        .all(|face| face.polygon.iter().all(|point| point[2].abs() < TOL));
    profile.flat_check_time = started.elapsed();
    if flat {
        return profile;
    }

    let started = Instant::now();
    let parts: Vec<Part> = frame.faces.iter().map(Part::new).collect();
    profile.build_parts_time = started.elapsed();

    let started = Instant::now();
    let mut candidates = Vec::new();
    for i in 0..parts.len() {
        for j in (i + 1)..parts.len() {
            profile.face_pairs += 1;
            let (a, b) = (&parts[i], &parts[j]);
            if !a.aabb_overlaps(b) {
                continue;
            }
            profile.aabb_overlaps += 1;
            if a.shares_edge(b) {
                profile.shared_edge_pairs += 1;
                continue;
            }
            candidates.push((i, j));
        }
    }
    profile.broad_phase_time = started.elapsed();
    profile.narrow_phase_pairs = candidates.len();

    let started = Instant::now();
    for (i, j) in candidates {
        let (a, b) = (&parts[i], &parts[j]);
        let triangle_pairs = a.tris.len() * b.tris.len();
        profile.triangle_pair_tests += triangle_pairs;
        profile.segment_triangle_tests += triangle_pairs * 6;
        profile.intersection_pairs += usize::from(a.deepest_piercing(b).is_some());
    }
    profile.narrow_phase_time = started.elapsed();
    profile
}

/// 立体の面同士が食い込んでいるか。
///
/// 全ての点が z≒0(平らに畳んだ生データ)のときは常に false を返す。
/// 平らな状態では全ての層が同一平面に重なるため、交差の判定に意味がない
/// (重なりの上下は層番号で表し、表示側が層をずらして見せる)。
#[must_use]
pub fn self_intersects(frame: &Frame3D) -> bool {
    !find_self_intersections(frame, Some(1)).is_empty()
}

/// 立体表示で実際に食い込んでいる面の組を、フレーム内の順序で返す。
///
/// [`self_intersects`] と判定条件は同じ。折り目でつながる面や、平らに畳まれた
/// 状態は含めない。原因候補の折り目を案内したい呼び出し側はこの詳細版を使う。
#[must_use]
pub fn self_intersection_pairs(frame: &Frame3D) -> Vec<(FaceId, FaceId)> {
    find_self_intersections(frame, None)
}

fn find_self_intersections(frame: &Frame3D, limit: Option<usize>) -> Vec<(FaceId, FaceId)> {
    find_self_intersection_details(frame, limit)
        .into_iter()
        .map(|intersection| intersection.frame_faces)
        .collect()
}

/// 交差面ペアごとに最深のwitnessを返す。深い順、深さ差が幾何許容内なら
/// FaceId順で、接触補正の計算量を一定にするため最大32件に限る。
#[must_use]
pub fn contact_witnesses(frame: &Frame3D) -> Vec<ContactWitness> {
    let mut witnesses: Vec<ContactWitness> = find_self_intersection_details(frame, None)
        .into_iter()
        .map(PairIntersection::into_contact_witness)
        .collect();
    sort_contact_witnesses(&mut witnesses);
    witnesses.truncate(MAX_CONTACT_WITNESSES);
    witnesses
}

fn sort_contact_witnesses(witnesses: &mut [ContactWitness]) {
    witnesses.sort_by(|a, b| {
        b.penetration_depth
            .total_cmp(&a.penetration_depth)
            .then_with(|| compare_contact_witness_key(a, b))
    });
    // まず深さを厳密に並べることでsort comparatorの推移律を保つ。その後、
    // 各bucketの先頭深さをanchorにして幾何許容内を不変キー順へ並べ直す。
    // a≈b、b≈cでもa≉cになる近似比較をsort comparatorへ入れない。
    let mut bucket_start = 0;
    while bucket_start < witnesses.len() {
        let anchor = witnesses[bucket_start].penetration_depth;
        let mut bucket_end = bucket_start + 1;
        while bucket_end < witnesses.len()
            && anchor - witnesses[bucket_end].penetration_depth <= TOL
        {
            bucket_end += 1;
        }
        witnesses[bucket_start..bucket_end].sort_by(compare_contact_witness_key);
        bucket_start = bucket_end;
    }
}

fn compare_contact_witness_key(a: &ContactWitness, b: &ContactWitness) -> std::cmp::Ordering {
    // find_self_intersection_detailsは一つの面ペアにつきwitnessを一つだけ返すため、
    // CPUで最下位bitが変わり得るpoint/normalを使わず、この不変キーだけで一意になる。
    a.faces.cmp(&b.faces)
}

/// witness上限を掛ける前の全交差面ペアについて、侵入深さを集計する。
#[must_use]
pub fn contact_metrics(frame: &Frame3D) -> ContactMetrics {
    find_self_intersection_details(frame, None)
        .into_iter()
        .fold(ContactMetrics::default(), |mut metrics, intersection| {
            let depth = intersection.witness.penetration_depth;
            metrics.pair_count += 1;
            metrics.max_penetration = metrics.max_penetration.max(depth);
            metrics.total_penetration += depth;
            metrics
        })
}

#[derive(Clone, Copy)]
struct PairIntersection {
    /// 既存APIとの互換のため、フレーム内で先に現れた面を先にする。
    frame_faces: (FaceId, FaceId),
    witness: SegmentTriangleWitness,
}

impl PairIntersection {
    fn into_contact_witness(self) -> ContactWitness {
        let (a, b) = self.frame_faces;
        ContactWitness {
            faces: (a.min(b), a.max(b)),
            point: self.witness.point.to_array(),
            normal: self.witness.normal.to_array(),
            penetration_depth: self.witness.penetration_depth,
        }
    }
}

fn find_self_intersection_details(frame: &Frame3D, limit: Option<usize>) -> Vec<PairIntersection> {
    let flat = frame
        .faces
        .iter()
        .all(|f| f.polygon.iter().all(|p| p[2].abs() < TOL));
    if flat {
        return Vec::new();
    }
    let parts: Vec<Part> = frame.faces.iter().map(Part::new).collect();
    let mut pairs = Vec::new();
    for i in 0..parts.len() {
        for j in (i + 1)..parts.len() {
            let (a, b) = (&parts[i], &parts[j]);
            if !a.aabb_overlaps(b) || a.shares_edge(b) {
                continue;
            }
            if let Some(witness) = a.deepest_piercing(b) {
                pairs.push(PairIntersection {
                    frame_faces: (frame.faces[i].face, frame.faces[j].face),
                    witness,
                });
                if limit.is_some_and(|max| pairs.len() >= max) {
                    return pairs;
                }
            }
        }
    }
    pairs
}

/// 食い込みの原因として利用者へ案内するヒンジを最大5本返す。
///
/// 交差面に直接隣接するdriverヒンジを最優先し、残りは各交差面ペアを面グラフ上で
/// 結ぶ最短経路から選ぶ。入力と辺IDだけで順序が決まるため、同じ形には常に同じ
/// 候補を返す。交差がなければ空になる。
#[must_use]
pub fn suspect_hinges(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &Frame3D,
    driver_hinges: &[EdgeId],
) -> Vec<EdgeId> {
    suspect_hinges_for_intersections(cp, faces, &self_intersection_pairs(frame), driver_hinges)
}

/// 交差面ペアを既に求めている呼び出し側のための [`suspect_hinges`]。
/// 16msごとの追従計算で同じ交差判定を警告用と候補用に二重実行しないために使う。
#[must_use]
pub fn suspect_hinges_for_intersections(
    cp: &CreasePattern,
    faces: &[Face],
    intersections: &[(FaceId, FaceId)],
    driver_hinges: &[EdgeId],
) -> Vec<EdgeId> {
    const MAX_SUSPECTS: usize = 5;

    if intersections.is_empty() {
        return Vec::new();
    }

    let fold_edges: BTreeSet<EdgeId> = cp
        .edges
        .iter()
        .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .map(|edge| edge.id)
        .collect();
    let mut edge_faces: BTreeMap<EdgeId, Vec<FaceId>> = BTreeMap::new();
    for face in faces {
        for &edge in &face.edges {
            if fold_edges.contains(&edge) {
                edge_faces.entry(edge).or_default().push(face.id);
            }
        }
    }
    for adjacent in edge_faces.values_mut() {
        adjacent.sort_unstable();
        adjacent.dedup();
    }
    edge_faces.retain(|_, adjacent| adjacent.len() == 2);

    let mut face_hinges: BTreeMap<FaceId, Vec<EdgeId>> = BTreeMap::new();
    for (&edge, adjacent) in &edge_faces {
        for &face in adjacent {
            face_hinges.entry(face).or_default().push(edge);
        }
    }

    let drivers: BTreeSet<EdgeId> = driver_hinges.iter().copied().collect();
    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();

    // 全交差ペアのdriver候補を先に集め、経路候補より必ず優先する。
    for &(a, b) in intersections {
        let incident = face_hinges
            .get(&a)
            .into_iter()
            .chain(face_hinges.get(&b))
            .flatten()
            .copied()
            .filter(|edge| drivers.contains(edge))
            .collect::<BTreeSet<_>>();
        for edge in incident {
            if selected.len() < MAX_SUSPECTS && seen.insert(edge) {
                selected.push(edge);
            }
        }
    }

    for &(start, goal) in intersections {
        if selected.len() >= MAX_SUSPECTS {
            break;
        }
        for edge in shortest_hinge_path(start, goal, &face_hinges, &edge_faces) {
            if selected.len() < MAX_SUSPECTS && seen.insert(edge) {
                selected.push(edge);
            }
        }
    }
    selected
}

fn shortest_hinge_path(
    start: FaceId,
    goal: FaceId,
    face_hinges: &BTreeMap<FaceId, Vec<EdgeId>>,
    edge_faces: &BTreeMap<EdgeId, Vec<FaceId>>,
) -> Vec<EdgeId> {
    if start == goal {
        return Vec::new();
    }
    let mut queue = VecDeque::from([start]);
    let mut visited = BTreeSet::from([start]);
    let mut previous: BTreeMap<FaceId, (FaceId, EdgeId)> = BTreeMap::new();
    while let Some(face) = queue.pop_front() {
        for &edge in face_hinges.get(&face).into_iter().flatten() {
            let Some(adjacent) = edge_faces.get(&edge) else {
                continue;
            };
            for &next in adjacent {
                if next == face || !visited.insert(next) {
                    continue;
                }
                previous.insert(next, (face, edge));
                if next == goal {
                    let mut path = Vec::new();
                    let mut cursor = goal;
                    while cursor != start {
                        let &(parent, via) = previous
                            .get(&cursor)
                            .expect("訪問済みの面には直前のヒンジがある");
                        path.push(via);
                        cursor = parent;
                    }
                    path.reverse();
                    return path;
                }
                queue.push_back(next);
            }
        }
    }
    Vec::new()
}

/// 平らに折り切った面の層順序が、共有折り目の山谷と矛盾しているか。
///
/// 表向きの面`a`から見て谷なら隣の面`b`は上、山なら下に来る。`a`が裏返って
/// いればこの関係も反転する。これは層モデルが折り目を通って紙を突き抜けた順序を
/// 記録していないかを調べる局所検査で、折り途中・投影が退化した面・未折りの辺は
/// 判定しない。
#[must_use]
pub fn layer_order_conflicts(cp: &CreasePattern, faces: &[Face], frame: &Frame3D) -> bool {
    let all_points = frame.faces.iter().flat_map(|face| &face.polygon);
    let (min_z, max_z) = all_points.fold((f64::MAX, f64::MIN), |(lo, hi), point| {
        (lo.min(point[2]), hi.max(point[2]))
    });
    if min_z == f64::MAX || max_z - min_z > TOL {
        return false; // 平らに折り切った状態だけを検査する
    }

    let positions: HashMap<VertexId, DVec2> = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect();
    let output: HashMap<FaceId, &ori3_model::Face3D> =
        frame.faces.iter().map(|face| (face.face, face)).collect();
    let mut mirrored: HashMap<FaceId, bool> = HashMap::new();
    for face in faces {
        let Some(folded) = output.get(&face.id) else {
            continue;
        };
        let original: Vec<DVec2> = face
            .vertices
            .iter()
            .filter_map(|vertex| positions.get(vertex).copied())
            .collect();
        let projected: Vec<DVec2> = folded
            .polygon
            .iter()
            .map(|point| DVec2::new(point[0], point[1]))
            .collect();
        let original_area = signed_area(&original);
        let projected_area = signed_area(&projected);
        if original_area.abs() <= TOL * TOL || projected_area.abs() <= TOL * TOL {
            continue;
        }
        mirrored.insert(face.id, original_area.signum() != projected_area.signum());
    }

    let mut edge_faces: BTreeMap<u32, Vec<FaceId>> = BTreeMap::new();
    for face in faces {
        let mut edges = face.edges.clone();
        edges.sort_unstable();
        edges.dedup();
        for edge in edges {
            edge_faces.entry(edge).or_default().push(face.id);
        }
    }
    for edge in &cp.edges {
        if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let Some(adjacent) = edge_faces.get(&edge.id) else {
            continue;
        };
        if adjacent.len() != 2 {
            continue;
        }
        let (a, b) = (adjacent[0], adjacent[1]);
        let (Some(&a_mirrored), Some(&b_mirrored)) = (mirrored.get(&a), mirrored.get(&b)) else {
            continue;
        };
        if a_mirrored == b_mirrored {
            continue; // この折り目は平らなままで、上下を拘束しない
        }
        let (Some(face_a), Some(face_b)) = (output.get(&a), output.get(&b)) else {
            continue;
        };
        let b_should_be_above = matches!(
            (edge.kind, a_mirrored),
            (EdgeKind::Valley, false) | (EdgeKind::Mountain, true)
        );
        if (face_b.layer > face_a.layer) != b_should_be_above {
            return true;
        }
    }
    false
}

/// 平らに折り切った形から、紙の重なり順を求める。
///
/// 折り目を1本ずつ見ると、山谷と面の裏返りから「どちらの面が上か」が一つに決まる。
/// その上下関係を並べ替えて順序(下から上)を作る。同じ高さに置ける面は面IDの
/// 小さい方を下にして、同じ入力なら必ず同じ順序になるようにする。
///
/// 平らに折り切っていない形、または上下関係が循環して成り立たない場合は `None`。
///
/// 手順を記録せず角度だけで折ると重なり順が決まらず、同じ平面の面が完全に同じ位置へ
/// 描かれて裏面が見えたり貫通して見えるため、その場合の順序決めに使う。
#[must_use]
pub fn derive_layer_order(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &Frame3D,
) -> Option<Vec<FaceId>> {
    let constraints = folded_pair_constraints(cp, faces, frame)?;
    let ids: BTreeSet<FaceId> = frame.faces.iter().map(|face| face.face).collect();

    // 「下の面 → 上の面」の向きで数え上げる。
    let mut above_of: BTreeMap<FaceId, BTreeSet<FaceId>> = BTreeMap::new();
    let mut below_count: BTreeMap<FaceId, usize> = ids.iter().map(|&id| (id, 0)).collect();
    for &(lower, upper) in &constraints {
        if above_of.entry(lower).or_default().insert(upper) {
            *below_count.entry(upper).or_default() += 1;
        }
    }

    let mut ready: BTreeSet<FaceId> = below_count
        .iter()
        .filter_map(|(&id, &count)| (count == 0).then_some(id))
        .collect();
    let mut order: Vec<FaceId> = Vec::with_capacity(ids.len());
    while let Some(&id) = ready.iter().next() {
        ready.remove(&id);
        order.push(id);
        for &upper in above_of.get(&id).into_iter().flatten() {
            let count = below_count.get_mut(&upper)?;
            *count -= 1;
            if *count == 0 {
                ready.insert(upper);
            }
        }
    }
    (order.len() == ids.len()).then_some(order)
}

/// 折り目ごとの「下の面, 上の面」。平らに折り切っていない形では `None`。
fn folded_pair_constraints(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &Frame3D,
) -> Option<Vec<(FaceId, FaceId)>> {
    let all_points = frame.faces.iter().flat_map(|face| &face.polygon);
    let (min_z, max_z) = all_points.fold((f64::MAX, f64::MIN), |(lo, hi), point| {
        (lo.min(point[2]), hi.max(point[2]))
    });
    if min_z == f64::MAX || max_z - min_z > TOL {
        return None;
    }

    let positions: HashMap<VertexId, DVec2> = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect();
    let output: HashMap<FaceId, &ori3_model::Face3D> =
        frame.faces.iter().map(|face| (face.face, face)).collect();
    let mut mirrored: HashMap<FaceId, bool> = HashMap::new();
    for face in faces {
        let Some(folded) = output.get(&face.id) else {
            continue;
        };
        let original: Vec<DVec2> = face
            .vertices
            .iter()
            .filter_map(|vertex| positions.get(vertex).copied())
            .collect();
        let projected: Vec<DVec2> = folded
            .polygon
            .iter()
            .map(|point| DVec2::new(point[0], point[1]))
            .collect();
        let original_area = signed_area(&original);
        let projected_area = signed_area(&projected);
        if original_area.abs() <= TOL * TOL || projected_area.abs() <= TOL * TOL {
            continue;
        }
        mirrored.insert(face.id, original_area.signum() != projected_area.signum());
    }

    let mut edge_faces: BTreeMap<u32, Vec<FaceId>> = BTreeMap::new();
    for face in faces {
        let mut edges = face.edges.clone();
        edges.sort_unstable();
        edges.dedup();
        for edge in edges {
            edge_faces.entry(edge).or_default().push(face.id);
        }
    }

    let mut pairs = Vec::new();
    for edge in &cp.edges {
        if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let Some(adjacent) = edge_faces.get(&edge.id) else {
            continue;
        };
        if adjacent.len() != 2 {
            continue;
        }
        let (a, b) = (adjacent[0], adjacent[1]);
        let (Some(&a_mirrored), Some(&b_mirrored)) = (mirrored.get(&a), mirrored.get(&b)) else {
            continue;
        };
        if a_mirrored == b_mirrored {
            continue; // この折り目は平らなままで、上下を拘束しない
        }
        let b_should_be_above = matches!(
            (edge.kind, a_mirrored),
            (EdgeKind::Valley, false) | (EdgeKind::Mountain, true)
        );
        pairs.push(if b_should_be_above { (a, b) } else { (b, a) });
    }
    Some(pairs)
}

fn signed_area(poly: &[DVec2]) -> f64 {
    if poly.len() < 3 {
        return 0.0;
    }
    0.5 * (0..poly.len())
        .map(|index| poly[index].perp_dot(poly[(index + 1) % poly.len()]))
        .sum::<f64>()
}

/// 1つの面の三角形列と外接直方体
struct Part {
    tris: Vec<[DVec3; 3]>,
    lo: DVec3,
    hi: DVec3,
    points: Vec<DVec3>,
}

impl Part {
    fn new(face: &ori3_model::Face3D) -> Part {
        let points: Vec<DVec3> = face.polygon.iter().map(|p| DVec3::from_array(*p)).collect();
        let tris = (1..points.len().saturating_sub(1))
            .map(|k| [points[0], points[k], points[k + 1]])
            .collect();
        let lo = points
            .iter()
            .copied()
            .fold(DVec3::splat(f64::MAX), DVec3::min);
        let hi = points
            .iter()
            .copied()
            .fold(DVec3::splat(f64::MIN), DVec3::max);
        Part {
            tris,
            lo,
            hi,
            points,
        }
    }

    fn aabb_overlaps(&self, other: &Part) -> bool {
        (0..3).all(|k| self.lo[k] <= other.hi[k] + TOL && other.lo[k] <= self.hi[k] + TOL)
    }

    /// 折り目でつながった面か(同じ位置の頂点を2つ以上共有する)
    fn shares_edge(&self, other: &Part) -> bool {
        let shared = self
            .points
            .iter()
            .filter(|p| other.points.iter().any(|q| (**p - *q).length() <= TOL))
            .count();
        shared >= 2
    }

    /// この面ペアの全三角形を調べ、最も深い線分―三角形貫通を返す。
    fn deepest_piercing(&self, other: &Part) -> Option<SegmentTriangleWitness> {
        let mut deepest: Option<SegmentTriangleWitness> = None;
        for first in &self.tris {
            for second in &other.tris {
                let Some(witness) = tris_piercing(first, second) else {
                    continue;
                };
                if deepest
                    .as_ref()
                    .is_none_or(|current| witness.penetration_depth > current.penetration_depth)
                {
                    deepest = Some(witness);
                }
            }
        }
        deepest
    }
}

#[derive(Clone, Copy)]
struct SegmentTriangleWitness {
    point: DVec3,
    normal: DVec3,
    penetration_depth: f64,
}

/// 2つの三角形を横切る全辺のうち、最も深いwitnessを返す。
/// 同一平面の重なりは交点計算が退化するため含めない。
fn tris_piercing(t1: &[DVec3; 3], t2: &[DVec3; 3]) -> Option<SegmentTriangleWitness> {
    let mut deepest: Option<SegmentTriangleWitness> = None;
    for (segment, triangle) in [(t1, t2), (t2, t1)] {
        for k in 0..3 {
            let Some(witness) = segment_piercing(segment[k], segment[(k + 1) % 3], triangle) else {
                continue;
            };
            if deepest
                .as_ref()
                .is_none_or(|current| witness.penetration_depth > current.penetration_depth)
            {
                deepest = Some(witness);
            }
        }
    }
    deepest
}

/// 線分が三角形の内部を貫く位置・三角形法線・侵入深さ(Möller–Trumbore法)。
/// 辺・頂点にちょうど触れるだけ、線分の端でちょうど接するだけの場合は含めない
/// (紙同士が触れているのは正常なので、はっきり食い込んだ場合だけを拾う)。
fn segment_piercing(p0: DVec3, p1: DVec3, tri: &[DVec3; 3]) -> Option<SegmentTriangleWitness> {
    let dir = p1 - p0;
    let (e1, e2) = (tri[1] - tri[0], tri[2] - tri[0]);
    let h = dir.cross(e2);
    let det = e1.dot(h);
    // detは長さの3乗の量なので、絶対値ではなく辺の長さに対する相対値で平行を判定する
    if det.abs() < TOL * dir.length() * e1.length() * e2.length() {
        return None; // 線分が三角形の面と平行(同一平面の重なりを含む)
    }
    let s = p0 - tri[0];
    let u = s.dot(h) / det;
    let q = s.cross(e1);
    let v = dir.dot(q) / det;
    if u <= TOL || v <= TOL || u + v >= 1.0 - TOL {
        return None; // 三角形の内部でなければ貫通としない
    }
    let t = e2.dot(q) / det;
    if !(t > TOL && t < 1.0 - TOL) {
        return None;
    }

    let raw_normal = e1.cross(e2);
    let normal_length = raw_normal.length();
    if !normal_length.is_finite() || normal_length == 0.0 {
        return None;
    }
    let normal = raw_normal / normal_length;
    let point = p0 + dir * t;
    let endpoint_depth = (p0 - tri[0])
        .dot(normal)
        .abs()
        .min((p1 - tri[0]).dot(normal).abs());
    let penetration_depth = (endpoint_depth - TOL).max(0.0);
    if !point.is_finite() || !normal.is_finite() || !penetration_depth.is_finite() {
        return None;
    }
    Some(SegmentTriangleWitness {
        point,
        normal,
        penetration_depth,
    })
}

#[cfg(test)]
mod tests {
    use super::{ContactWitness, MAX_CONTACT_WITNESSES, TOL, sort_contact_witnesses};

    use ori3_model::{Face3D, Frame3D};

    /// 立体的な姿勢(平らに折り切っていない形)では重なり順を決めない。
    /// 重なった面が同じ高さに無いので、上下は見る向きで変わるため。
    #[test]
    fn solid_pose_has_no_single_layer_order() {
        let frame = Frame3D {
            faces: vec![
                Face3D {
                    face: 0,
                    polygon: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
                    layer: 0,
                    mirrored: false,
                },
                Face3D {
                    face: 1,
                    polygon: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.5], [1.0, 1.0, 0.5]],
                    layer: 0,
                    mirrored: false,
                },
            ],
            warnings: Vec::new(),
        };
        let cp = ori3_model::Document::new(ori3_model::Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        })
        .cp;
        let faces = ori3_cp::extract_faces(&cp);
        assert!(super::derive_layer_order(&cp, &faces, &frame).is_none());
    }

    #[test]
    fn witness_cutoff_uses_face_ids_for_depths_within_tolerance() {
        let denominator = (MAX_CONTACT_WITNESSES * 2) as f64;
        let mut witnesses = (0..=MAX_CONTACT_WITNESSES)
            .rev()
            .map(|index| ContactWitness {
                faces: (index as u32 * 2, index as u32 * 2 + 1),
                point: [index as f64, 0.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                // 全候補を最深値からTOL/2内に置く。exact depth順だと大きい
                // FaceIdが残るが、同深度bucketでは小さいFaceIdが残る。
                penetration_depth: 0.25 + index as f64 * TOL / denominator,
            })
            .collect::<Vec<_>>();

        sort_contact_witnesses(&mut witnesses);
        witnesses.truncate(MAX_CONTACT_WITNESSES);

        let actual = witnesses
            .iter()
            .map(|witness| witness.faces)
            .collect::<Vec<_>>();
        let expected = (0..MAX_CONTACT_WITNESSES)
            .map(|index| (index as u32 * 2, index as u32 * 2 + 1))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}
