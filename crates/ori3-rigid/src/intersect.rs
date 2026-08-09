//! めり込み警告(SIM-007): 立体表示の面同士が突き抜けていないかを調べる。
//!
//! 厳密な防止はしない(剛体折りの計算を止めない)。「紙が食い込んでいる」ことを
//! 見つけて警告に載せるだけの検査で、実際の紙では起こり得ない形を利用者へ知らせる。
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

use glam::{DVec2, DVec3};
use ori3_cp::Face;
use ori3_model::{CreasePattern, EdgeId, EdgeKind, FaceId, Frame3D, VertexId};

/// めり込みを見つけたときの警告文(3D表示のバッジに出る)
pub const PENETRATION_WARNING: &str = "紙が重なって食い込んでいます";

/// 幾何の許容誤差(正規化座標。長辺=1.0)。接している程度は貫通としない。
const TOL: f64 = 1e-6;

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
            if a.tris
                .iter()
                .any(|t1| b.tris.iter().any(|t2| tris_pierce(t1, t2)))
            {
                pairs.push((frame.faces[i].face, frame.faces[j].face));
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
}

/// 2つの三角形が突き抜けているか(辺が相手の内部を貫くか)。
/// 同一平面の重なりは貫通としない(交点の計算が退化しNoneになる)。
fn tris_pierce(t1: &[DVec3; 3], t2: &[DVec3; 3]) -> bool {
    (0..3).any(|k| segment_pierces(t1[k], t1[(k + 1) % 3], t2))
        || (0..3).any(|k| segment_pierces(t2[k], t2[(k + 1) % 3], t1))
}

/// 線分が三角形の内部を貫くか(Möller–Trumbore法)。
/// 辺・頂点にちょうど触れるだけ、線分の端でちょうど接するだけの場合は含めない
/// (紙同士が触れているのは正常なので、はっきり食い込んだ場合だけを拾う)。
fn segment_pierces(p0: DVec3, p1: DVec3, tri: &[DVec3; 3]) -> bool {
    let dir = p1 - p0;
    let (e1, e2) = (tri[1] - tri[0], tri[2] - tri[0]);
    let h = dir.cross(e2);
    let det = e1.dot(h);
    // detは長さの3乗の量なので、絶対値ではなく辺の長さに対する相対値で平行を判定する
    if det.abs() < TOL * dir.length() * e1.length() * e2.length() {
        return false; // 線分が三角形の面と平行(同一平面の重なりを含む)
    }
    let s = p0 - tri[0];
    let u = s.dot(h) / det;
    let q = s.cross(e1);
    let v = dir.dot(q) / det;
    if u <= TOL || v <= TOL || u + v >= 1.0 - TOL {
        return false; // 三角形の内部でなければ貫通としない
    }
    let t = e2.dot(q) / det;
    t > TOL && t < 1.0 - TOL
}
