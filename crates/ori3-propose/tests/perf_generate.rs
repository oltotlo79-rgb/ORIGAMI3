//! 折り線の分割方式を、現方式と接触関係を使う試作方式で同時測定する。
//!
//! 試作はこの検査ファイルだけに置く。製品コードへ方式を入れず、測定値からの
//! 採否もこの検査では決めない。試作で変えるのは三角形分割だけで、各三角形を
//! 埋めるウサギ耳分子、山谷、垂線先の規則は現方式と同じにする。

use std::collections::{BTreeMap, BTreeSet};
use std::hint::black_box;
use std::time::{Duration, Instant};

use ori3_cp::{insert_segment, local_violations, validate};
use ori3_model::{CreasePattern, EPS, Edge, EdgeKind, Vertex};
use ori3_propose::packing::PACK_TOL;
use ori3_propose::skeleton::{Skeleton, SkeletonNode};
use ori3_propose::triangulate::{MERGE_TOL, dedup, index_of, triangulate};
use ori3_propose::{Packing, generate, pack};

const PAPER_W: f64 = 1.0;
const PAPER_H: f64 = 1.0;
const STARTS: usize = 8;
/// 充填制約の必要距離との差が、配置側の制約許容差以内なら接触辺とみなす。
const CONTACT_TOL: f64 = PACK_TOL;
const ON_EDGE_TOL: f64 = 1e-9;
const PLANAR_TOL: f64 = 1e-12;
const TIMING_REPETITIONS: usize = 9;

type Point = [f64; 2];
type PointEdge = (usize, usize);

#[derive(Clone, Copy, Debug)]
struct Contact {
    edge: PointEdge,
    slack: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct PrototypeResult {
    cp: CreasePattern,
    violations: usize,
    validation_warnings: usize,
}

#[derive(Clone, Copy, Debug)]
struct Metrics {
    local_violations: usize,
    three_forks: usize,
    kawasaki_max_rad: f64,
    kawasaki_median_rad: f64,
    generation_ms: f64,
}

#[derive(Clone, Debug)]
struct CaseMeasurement {
    label: String,
    seed: u64,
    current: Metrics,
    prototype: Metrics,
}

#[derive(Clone, Copy, Debug)]
struct Distribution {
    min: f64,
    median: f64,
    p95: f64,
    max: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Wins {
    current: usize,
    prototype: usize,
    ties: usize,
}

fn crane_like() -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    nodes.push(SkeletonNode::new(1, Some(0), 1.0));
    nodes.push(SkeletonNode::new(2, Some(0), 1.0));
    for id in 3..=6 {
        nodes.push(SkeletonNode::new(id, Some(0), 0.7));
    }
    Skeleton { nodes }
}

fn canonical_edge(a: usize, b: usize) -> PointEdge {
    (a.min(b), a.max(b))
}

fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn cross(a: Point, b: Point, c: Point) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn on_segment(p: Point, a: Point, b: Point) -> bool {
    cross(a, b, p).abs() <= PLANAR_TOL
        && p[0] >= a[0].min(b[0]) - PLANAR_TOL
        && p[0] <= a[0].max(b[0]) + PLANAR_TOL
        && p[1] >= a[1].min(b[1]) - PLANAR_TOL
        && p[1] <= a[1].max(b[1]) + PLANAR_TOL
}

fn point_on_open_segment(p: Point, a: Point, b: Point) -> bool {
    on_segment(p, a, b) && distance(p, a) > MERGE_TOL && distance(p, b) > MERGE_TOL
}

fn segments_conflict(a: Point, b: Point, c: Point, d: Point) -> bool {
    let (ab_c, ab_d) = (cross(a, b, c), cross(a, b, d));
    let (cd_a, cd_b) = (cross(c, d, a), cross(c, d, b));
    let proper = ((ab_c > PLANAR_TOL && ab_d < -PLANAR_TOL)
        || (ab_c < -PLANAR_TOL && ab_d > PLANAR_TOL))
        && ((cd_a > PLANAR_TOL && cd_b < -PLANAR_TOL) || (cd_a < -PLANAR_TOL && cd_b > PLANAR_TOL));
    proper
        || (ab_c.abs() <= PLANAR_TOL && on_segment(c, a, b))
        || (ab_d.abs() <= PLANAR_TOL && on_segment(d, a, b))
        || (cd_a.abs() <= PLANAR_TOL && on_segment(a, c, d))
        || (cd_b.abs() <= PLANAR_TOL && on_segment(b, c, d))
}

fn can_add_edge(points: &[Point], edges: &BTreeSet<PointEdge>, edge: PointEdge) -> bool {
    let (a, b) = edge;
    if a == b
        || points
            .iter()
            .enumerate()
            .any(|(i, &p)| i != a && i != b && point_on_open_segment(p, points[a], points[b]))
    {
        return false;
    }
    edges.iter().all(|&(c, d)| {
        a == c
            || a == d
            || b == c
            || b == d
            || !segments_conflict(points[a], points[b], points[c], points[d])
    })
}

fn add_boundary_edges(
    points: &[Point],
    paper_w: f64,
    paper_h: f64,
    edges: &mut BTreeSet<PointEdge>,
) {
    let sides = [
        (0usize, 0.0, true),
        (0usize, paper_w, true),
        (1usize, 0.0, false),
        (1usize, paper_h, false),
    ];
    for (axis, value, sort_by_y) in sides {
        let mut on_side: Vec<usize> = points
            .iter()
            .enumerate()
            .filter(|(_, p)| (p[axis] - value).abs() <= ON_EDGE_TOL)
            .map(|(i, _)| i)
            .collect();
        on_side.sort_by(|&a, &b| {
            let coordinate = |p: Point| if sort_by_y { p[1] } else { p[0] };
            coordinate(points[a])
                .total_cmp(&coordinate(points[b]))
                .then(a.cmp(&b))
        });
        for pair in on_side.windows(2) {
            edges.insert(canonical_edge(pair[0], pair[1]));
        }
    }
}

fn contact_relations(skeleton: &Skeleton, packing: &Packing, points: &[Point]) -> Vec<Contact> {
    let mut contacts = Vec::new();
    for i in 0..packing.centers.len() {
        for j in (i + 1)..packing.centers.len() {
            let (id_a, a) = packing.centers[i];
            let (id_b, b) = packing.centers[j];
            let Some(point_a) = index_of(points, a) else {
                continue;
            };
            let Some(point_b) = index_of(points, b) else {
                continue;
            };
            if point_a == point_b {
                continue;
            }
            let actual = distance(a, b);
            let required = packing.scale * skeleton.leaf_distance(id_a, id_b);
            let slack = actual - required;
            if !slack.is_finite() || slack > CONTACT_TOL {
                continue;
            }
            contacts.push(Contact {
                edge: canonical_edge(point_a, point_b),
                slack,
            });
        }
    }
    contacts.sort_by(|a, b| a.slack.total_cmp(&b.slack).then(a.edge.cmp(&b.edge)));
    contacts
}

fn point_strictly_inside_triangle(p: Point, tri: [Point; 3]) -> bool {
    let signs = [
        cross(tri[0], tri[1], p),
        cross(tri[1], tri[2], p),
        cross(tri[2], tri[0], p),
    ];
    signs.iter().all(|value| *value > PLANAR_TOL) || signs.iter().all(|value| *value < -PLANAR_TOL)
}

fn contact_constrained_triangulation(
    points: &[Point],
    contacts: &[Contact],
    paper_w: f64,
    paper_h: f64,
) -> Result<Vec<[usize; 3]>, String> {
    let mut edges = BTreeSet::new();
    add_boundary_edges(points, paper_w, paper_h, &mut edges);

    for contact in contacts {
        if edges.contains(&contact.edge) || can_add_edge(points, &edges, contact.edge) {
            edges.insert(contact.edge);
        }
    }

    for triangle in triangulate(points) {
        for edge in [
            canonical_edge(triangle[0], triangle[1]),
            canonical_edge(triangle[1], triangle[2]),
            canonical_edge(triangle[2], triangle[0]),
        ] {
            if !edges.contains(&edge) && can_add_edge(points, &edges, edge) {
                edges.insert(edge);
            }
        }
    }

    let mut all_edges = Vec::new();
    for a in 0..points.len() {
        for b in (a + 1)..points.len() {
            all_edges.push((canonical_edge(a, b), distance(points[a], points[b])));
        }
    }
    all_edges.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    for (edge, _) in all_edges {
        if !edges.contains(&edge) && can_add_edge(points, &edges, edge) {
            edges.insert(edge);
        }
    }

    let mut triangles = Vec::new();
    for a in 0..points.len() {
        for b in (a + 1)..points.len() {
            for c in (b + 1)..points.len() {
                if !edges.contains(&canonical_edge(a, b))
                    || !edges.contains(&canonical_edge(b, c))
                    || !edges.contains(&canonical_edge(c, a))
                {
                    continue;
                }
                let area2 = cross(points[a], points[b], points[c]);
                if area2.abs() <= PLANAR_TOL {
                    continue;
                }
                let triangle = if area2 > 0.0 { [a, b, c] } else { [a, c, b] };
                let positions = [
                    points[triangle[0]],
                    points[triangle[1]],
                    points[triangle[2]],
                ];
                if (0..points.len()).any(|point| {
                    !triangle.contains(&point)
                        && point_strictly_inside_triangle(points[point], positions)
                }) {
                    continue;
                }
                triangles.push(triangle);
            }
        }
    }
    triangles.sort_unstable();

    let area: f64 = triangles
        .iter()
        .map(|t| cross(points[t[0]], points[t[1]], points[t[2]]).abs() * 0.5)
        .sum();
    let expected = paper_w * paper_h;
    if triangles.is_empty() || (area - expected).abs() > 1e-9 * expected.max(1.0) {
        return Err(format!(
            "接触辺を含む分割が紙を覆わない: triangles={} area={area:.15} expected={expected:.15}",
            triangles.len()
        ));
    }
    Ok(triangles)
}

fn border_cp(w: f64, h: f64) -> CreasePattern {
    let corners = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]];
    CreasePattern {
        vertices: corners
            .iter()
            .enumerate()
            .map(|(id, &pos)| Vertex { id: id as u32, pos })
            .collect(),
        edges: (0..4)
            .map(|id| Edge {
                id,
                v0: id,
                v1: (id + 1) % 4,
                kind: EdgeKind::Border,
            })
            .collect(),
        next_vertex_id: 4,
        next_edge_id: 4,
    }
}

fn on_paper_edge(a: Point, b: Point, paper_w: f64, paper_h: f64) -> bool {
    let same = |x: f64, y: f64, value: f64| {
        (x - value).abs() < ON_EDGE_TOL && (y - value).abs() < ON_EDGE_TOL
    };
    same(a[0], b[0], 0.0)
        || same(a[0], b[0], paper_w)
        || same(a[1], b[1], 0.0)
        || same(a[1], b[1], paper_h)
}

fn foot(p: Point, a: Point, b: Point) -> Point {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    if len2 <= 0.0 {
        return a;
    }
    let t = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0);
    [a[0] + ab[0] * t, a[1] + ab[1] * t]
}

fn fill_rabbit_ear(
    cp: &mut CreasePattern,
    triangle: [usize; 3],
    points: &[Point],
    paper_w: f64,
    paper_h: f64,
) {
    let tri = [
        points[triangle[0]],
        points[triangle[1]],
        points[triangle[2]],
    ];
    let opposite_length = |i: usize| distance(tri[(i + 1) % 3], tri[(i + 2) % 3]);
    let lengths = [opposite_length(0), opposite_length(1), opposite_length(2)];
    let sum = lengths.iter().sum::<f64>();
    if sum <= 0.0 || !sum.is_finite() {
        return;
    }
    let incenter = [
        (lengths[0] * tri[0][0] + lengths[1] * tri[1][0] + lengths[2] * tri[2][0]) / sum,
        (lengths[0] * tri[0][1] + lengths[1] * tri[1][1] + lengths[2] * tri[2][1]) / sum,
    ];

    for i in 0..3 {
        let (a, b) = (tri[i], tri[(i + 1) % 3]);
        if !on_paper_edge(a, b, paper_w, paper_h) {
            insert_segment(cp, a, b, EdgeKind::Valley);
        }
    }
    for point in tri {
        insert_segment(cp, point, incenter, EdgeKind::Mountain);
    }

    let key = |i: usize| {
        let a = triangle[(i + 1) % 3];
        let b = triangle[(i + 2) % 3];
        let border = on_paper_edge(points[a], points[b], paper_w, paper_h);
        (u8::from(border), lengths[i])
    };
    let pick = (0..3)
        .max_by(|&a, &b| {
            let (x, y) = (key(a), key(b));
            x.0.cmp(&y.0).then(x.1.total_cmp(&y.1))
        })
        .unwrap_or(0);
    let target = foot(incenter, tri[(pick + 1) % 3], tri[(pick + 2) % 3]);
    insert_segment(cp, incenter, target, EdgeKind::Valley);
}

fn generation_points(
    skeleton: &Skeleton,
    packing: &Packing,
    paper_w: f64,
    paper_h: f64,
) -> Result<Vec<Point>, String> {
    skeleton.validate()?;
    if !(paper_w > 0.0 && paper_h > 0.0 && paper_w.is_finite() && paper_h.is_finite()) {
        return Err("紙寸法が不正".to_string());
    }
    if packing.centers.is_empty() {
        return Err("配置が空".to_string());
    }

    let mut raw_points: Vec<Point> = packing.centers.iter().map(|(_, center)| *center).collect();
    raw_points.extend([
        [0.0, 0.0],
        [paper_w, 0.0],
        [paper_w, paper_h],
        [0.0, paper_h],
    ]);
    Ok(dedup(&raw_points))
}

fn build_from_triangles(
    points: &[Point],
    triangles: &[[usize; 3]],
    paper_w: f64,
    paper_h: f64,
) -> PrototypeResult {
    let mut cp = border_cp(paper_w, paper_h);
    for &triangle in triangles {
        fill_rabbit_ear(&mut cp, triangle, points, paper_w, paper_h);
    }
    let violations = local_violations(&cp).len();
    let validation_warnings = validate(&cp).len();
    PrototypeResult {
        cp,
        violations,
        validation_warnings,
    }
}

/// 製品の現方式と同じ点準備・Delaunay分割・分子充填を、試作と同じ範囲で測る。
/// 各標本で製品の`generate`とCP・局所違反が一致することを計時前に照合する。
fn current_generate_for_measurement(
    skeleton: &Skeleton,
    packing: &Packing,
    paper_w: f64,
    paper_h: f64,
) -> Result<PrototypeResult, String> {
    let points = generation_points(skeleton, packing, paper_w, paper_h)?;
    let triangles = triangulate(&points);
    Ok(build_from_triangles(&points, &triangles, paper_w, paper_h))
}

fn prototype_generate(
    skeleton: &Skeleton,
    packing: &Packing,
    paper_w: f64,
    paper_h: f64,
) -> Result<PrototypeResult, String> {
    let points = generation_points(skeleton, packing, paper_w, paper_h)?;
    let contacts = contact_relations(skeleton, packing, &points);
    let triangles = contact_constrained_triangulation(&points, &contacts, paper_w, paper_h)?;
    Ok(build_from_triangles(&points, &triangles, paper_w, paper_h))
}

fn incident_directions(cp: &CreasePattern) -> (BTreeMap<u32, Vec<Point>>, BTreeSet<u32>) {
    let positions: BTreeMap<u32, Point> = cp.vertices.iter().map(|v| (v.id, v.pos)).collect();
    let mut incident: BTreeMap<u32, Vec<Point>> = BTreeMap::new();
    let mut border = BTreeSet::new();
    for edge in &cp.edges {
        let (Some(&a), Some(&b)) = (positions.get(&edge.v0), positions.get(&edge.v1)) else {
            continue;
        };
        if distance(a, b) < EPS {
            continue;
        }
        if edge.kind == EdgeKind::Border {
            border.insert(edge.v0);
            border.insert(edge.v1);
            continue;
        }
        if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        incident
            .entry(edge.v0)
            .or_default()
            .push([b[0] - a[0], b[1] - a[1]]);
        incident
            .entry(edge.v1)
            .or_default()
            .push([a[0] - b[0], a[1] - b[1]]);
    }
    (incident, border)
}

fn kawasaki_residual(directions: &[Point]) -> f64 {
    let mut angles: Vec<f64> = directions
        .iter()
        .map(|direction| direction[1].atan2(direction[0]))
        .collect();
    angles.sort_by(f64::total_cmp);
    let mut alternating_sum = 0.0;
    for i in (0..angles.len()).step_by(2) {
        let mut gap = angles[(i + 1) % angles.len()] - angles[i];
        if gap < 0.0 {
            gap += std::f64::consts::TAU;
        }
        alternating_sum += gap;
    }
    (alternating_sum - std::f64::consts::PI).abs()
}

fn median(sorted: &[f64]) -> f64 {
    assert!(!sorted.is_empty());
    if sorted.len().is_multiple_of(2) {
        0.5 * (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2])
    } else {
        sorted[sorted.len() / 2]
    }
}

fn cp_metrics(cp: &CreasePattern, generation_ms: f64) -> Metrics {
    let (incident, border) = incident_directions(cp);
    let three_forks = incident
        .iter()
        .filter(|(vertex, directions)| !border.contains(vertex) && directions.len() == 3)
        .count();
    let mut residuals: Vec<f64> = incident
        .iter()
        .filter(|(vertex, directions)| {
            !border.contains(vertex) && directions.len() >= 2 && directions.len().is_multiple_of(2)
        })
        .map(|(_, directions)| kawasaki_residual(directions))
        .collect();
    residuals.sort_by(f64::total_cmp);
    let (kawasaki_max_rad, kawasaki_median_rad) = residuals
        .last()
        .map_or((0.0, 0.0), |last| (*last, median(&residuals)));
    Metrics {
        local_violations: local_violations(cp).len(),
        three_forks,
        kawasaki_max_rad,
        kawasaki_median_rad,
        generation_ms,
    }
}

fn duration_median(mut durations: Vec<Duration>) -> f64 {
    durations.sort_unstable();
    durations[durations.len() / 2].as_secs_f64() * 1000.0
}

fn accept_deterministic_result(
    slot: &mut Option<PrototypeResult>,
    result: PrototypeResult,
    method: &str,
) -> Result<(), String> {
    if result.validation_warnings != 0 {
        return Err(format!(
            "{method}の展開図点検に{}件の警告がある",
            result.validation_warnings
        ));
    }
    if let Some(expected) = slot {
        if expected != &result {
            return Err(format!("{method}が同じ配置から異なる結果を返した"));
        }
    } else {
        *slot = Some(result);
    }
    Ok(())
}

fn measure_case(
    skeleton: &Skeleton,
    packing: &Packing,
    case_index: usize,
) -> Result<(Metrics, Metrics), String> {
    let mut current_times = Vec::with_capacity(TIMING_REPETITIONS);
    let mut prototype_times = Vec::with_capacity(TIMING_REPETITIONS);
    let mut current_result = None;
    let mut prototype_result = None;

    // test-localの現方式が、その場で呼んだ製品baselineと同じであることを101件全てで照合する。
    let product = generate(skeleton, packing, PAPER_W, PAPER_H)?;
    let copied_current = current_generate_for_measurement(skeleton, packing, PAPER_W, PAPER_H)?;
    if product.cp != copied_current.cp || product.violations != copied_current.violations {
        return Err("計時用の現方式が製品generateと一致しない".to_string());
    }
    black_box(&product);
    accept_deterministic_result(&mut current_result, copied_current, "現方式")?;

    if case_index.is_multiple_of(2) {
        let current = current_generate_for_measurement(skeleton, packing, PAPER_W, PAPER_H)?;
        black_box(&current);
        accept_deterministic_result(&mut current_result, current, "現方式")?;
        let prototype = prototype_generate(skeleton, packing, PAPER_W, PAPER_H)?;
        black_box(&prototype);
        accept_deterministic_result(&mut prototype_result, prototype, "試作方式")?;
    } else {
        let prototype = prototype_generate(skeleton, packing, PAPER_W, PAPER_H)?;
        black_box(&prototype);
        accept_deterministic_result(&mut prototype_result, prototype, "試作方式")?;
        let current = current_generate_for_measurement(skeleton, packing, PAPER_W, PAPER_H)?;
        black_box(&current);
        accept_deterministic_result(&mut current_result, current, "現方式")?;
    }

    for repetition in 0..TIMING_REPETITIONS {
        let current_first = (case_index + repetition).is_multiple_of(2);
        if current_first {
            let started = Instant::now();
            let result = current_generate_for_measurement(skeleton, packing, PAPER_W, PAPER_H)?;
            current_times.push(started.elapsed());
            black_box(&result);
            accept_deterministic_result(&mut current_result, result, "現方式")?;

            let started = Instant::now();
            let result = prototype_generate(skeleton, packing, PAPER_W, PAPER_H)?;
            prototype_times.push(started.elapsed());
            black_box(&result);
            accept_deterministic_result(&mut prototype_result, result, "試作方式")?;
        } else {
            let started = Instant::now();
            let result = prototype_generate(skeleton, packing, PAPER_W, PAPER_H)?;
            prototype_times.push(started.elapsed());
            black_box(&result);
            accept_deterministic_result(&mut prototype_result, result, "試作方式")?;

            let started = Instant::now();
            let result = current_generate_for_measurement(skeleton, packing, PAPER_W, PAPER_H)?;
            current_times.push(started.elapsed());
            black_box(&result);
            accept_deterministic_result(&mut current_result, result, "現方式")?;
        }
    }

    let current_ms = duration_median(current_times);
    let prototype_ms = duration_median(prototype_times);
    let current = current_result.ok_or_else(|| "現方式の測定値がない".to_string())?;
    let prototype = prototype_result.ok_or_else(|| "試作方式の測定値がない".to_string())?;
    Ok((
        cp_metrics(&current.cp, current_ms),
        cp_metrics(&prototype.cp, prototype_ms),
    ))
}

fn distribution(mut values: Vec<f64>) -> Distribution {
    assert!(!values.is_empty() && values.iter().all(|value| value.is_finite()));
    values.sort_by(f64::total_cmp);
    let p95_rank = (values.len() * 95).div_ceil(100);
    Distribution {
        min: values[0],
        median: median(&values),
        p95: values[p95_rank - 1],
        max: *values.last().unwrap_or(&values[0]),
    }
}

fn wins(current: impl Iterator<Item = f64>, prototype: impl Iterator<Item = f64>) -> Wins {
    let mut result = Wins::default();
    for (current, prototype) in current.zip(prototype) {
        match current.total_cmp(&prototype) {
            std::cmp::Ordering::Less => result.current += 1,
            std::cmp::Ordering::Greater => result.prototype += 1,
            std::cmp::Ordering::Equal => result.ties += 1,
        }
    }
    result
}

fn print_summary(name: &str, current: Vec<f64>, prototype: Vec<f64>) {
    let result_wins = wins(current.iter().copied(), prototype.iter().copied());
    let current_distribution = distribution(current);
    let prototype_distribution = distribution(prototype);
    if name.starts_with("kawasaki_") {
        eprintln!(
            "P14_SUMMARY metric={name} method=current min={:.15e} median={:.15e} p95={:.15e} max={:.15e}",
            current_distribution.min,
            current_distribution.median,
            current_distribution.p95,
            current_distribution.max
        );
        eprintln!(
            "P14_SUMMARY metric={name} method=prototype min={:.15e} median={:.15e} p95={:.15e} max={:.15e}",
            prototype_distribution.min,
            prototype_distribution.median,
            prototype_distribution.p95,
            prototype_distribution.max
        );
    } else {
        eprintln!(
            "P14_SUMMARY metric={name} method=current min={:.12} median={:.12} p95={:.12} max={:.12}",
            current_distribution.min,
            current_distribution.median,
            current_distribution.p95,
            current_distribution.max
        );
        eprintln!(
            "P14_SUMMARY metric={name} method=prototype min={:.12} median={:.12} p95={:.12} max={:.12}",
            prototype_distribution.min,
            prototype_distribution.median,
            prototype_distribution.p95,
            prototype_distribution.max
        );
    }
    eprintln!(
        "P14_WINS metric={name} current={} prototype={} ties={} total={}",
        result_wins.current,
        result_wins.prototype,
        result_wins.ties,
        result_wins.current + result_wins.prototype + result_wins.ties
    );
}

/// 101配置を2方式で作り直し、生成時間も同じプロセス内で交互に測るため、通常の
/// `cargo test` に毎回含めるには重い。作業14を再測定するときだけ明示実行する。
#[test]
#[ignore = "101配置×2方式の全件測定で通常検査を遅くするため、作業14の再測定時だけ実行する"]
fn compare_current_and_contact_constrained_generation_for_101_cases() {
    let skeleton = crane_like();
    let mut cases = Vec::with_capacity(101);
    cases.push(("fixed".to_string(), 2026u64));
    cases.extend((0..100u64).map(|seed| (format!("seed-{seed}"), seed)));
    assert_eq!(cases.len(), 101);

    let mut measurements = Vec::with_capacity(cases.len());
    let mut failed = Vec::new();
    for (case_index, (label, seed)) in cases.into_iter().enumerate() {
        let candidates = pack(&skeleton, PAPER_W, PAPER_H, seed, STARTS);
        let Some(packing) = candidates.first() else {
            failed.push(format!("{label}: 配置候補0件"));
            continue;
        };
        match measure_case(&skeleton, packing, case_index) {
            Ok((current, prototype)) => {
                eprintln!(
                    "P14_ROW sample={label} seed={seed} current_local={} prototype_local={} current_three_forks={} prototype_three_forks={} current_kawasaki_max_rad={:.15e} prototype_kawasaki_max_rad={:.15e} current_kawasaki_median_rad={:.15e} prototype_kawasaki_median_rad={:.15e} current_generation_ms={:.9} prototype_generation_ms={:.9}",
                    current.local_violations,
                    prototype.local_violations,
                    current.three_forks,
                    prototype.three_forks,
                    current.kawasaki_max_rad,
                    prototype.kawasaki_max_rad,
                    current.kawasaki_median_rad,
                    prototype.kawasaki_median_rad,
                    current.generation_ms,
                    prototype.generation_ms
                );
                measurements.push(CaseMeasurement {
                    label,
                    seed,
                    current,
                    prototype,
                });
            }
            Err(error) => failed.push(format!("{label}: {error}")),
        }
    }

    assert!(failed.is_empty(), "測定失敗{}件: {failed:?}", failed.len());
    assert_eq!(measurements.len(), 101, "測定済み件数が101でない");
    assert!(
        measurements.iter().all(|measurement| {
            [measurement.current, measurement.prototype]
                .iter()
                .all(|metrics| {
                    metrics.kawasaki_max_rad.is_finite()
                        && metrics.kawasaki_median_rad.is_finite()
                        && metrics.generation_ms.is_finite()
                })
        }),
        "非有限な測定値がある"
    );
    let fixed = &measurements[0];
    assert_eq!(fixed.label, "fixed");
    assert_eq!(fixed.seed, 2026);
    eprintln!(
        "P14_FIXED current_local={} prototype_local={} current_three_forks={} prototype_three_forks={} current_kawasaki_max_rad={:.15e} prototype_kawasaki_max_rad={:.15e} current_kawasaki_median_rad={:.15e} prototype_kawasaki_median_rad={:.15e} current_generation_ms={:.9} prototype_generation_ms={:.9}",
        fixed.current.local_violations,
        fixed.prototype.local_violations,
        fixed.current.three_forks,
        fixed.prototype.three_forks,
        fixed.current.kawasaki_max_rad,
        fixed.prototype.kawasaki_max_rad,
        fixed.current.kawasaki_median_rad,
        fixed.prototype.kawasaki_median_rad,
        fixed.current.generation_ms,
        fixed.prototype.generation_ms
    );

    let sweep = &measurements[1..];
    assert_eq!(sweep.len(), 100);
    print_summary(
        "local_violations",
        sweep
            .iter()
            .map(|measurement| measurement.current.local_violations as f64)
            .collect(),
        sweep
            .iter()
            .map(|measurement| measurement.prototype.local_violations as f64)
            .collect(),
    );
    print_summary(
        "three_forks",
        sweep
            .iter()
            .map(|measurement| measurement.current.three_forks as f64)
            .collect(),
        sweep
            .iter()
            .map(|measurement| measurement.prototype.three_forks as f64)
            .collect(),
    );
    print_summary(
        "kawasaki_max_rad",
        sweep
            .iter()
            .map(|measurement| measurement.current.kawasaki_max_rad)
            .collect(),
        sweep
            .iter()
            .map(|measurement| measurement.prototype.kawasaki_max_rad)
            .collect(),
    );
    print_summary(
        "kawasaki_median_rad",
        sweep
            .iter()
            .map(|measurement| measurement.current.kawasaki_median_rad)
            .collect(),
        sweep
            .iter()
            .map(|measurement| measurement.prototype.kawasaki_median_rad)
            .collect(),
    );
    print_summary(
        "generation_ms",
        sweep
            .iter()
            .map(|measurement| measurement.current.generation_ms)
            .collect(),
        sweep
            .iter()
            .map(|measurement| measurement.prototype.generation_ms)
            .collect(),
    );
    eprintln!(
        "P14_COMPLETE cases={} methods=2 indicators=4 numeric_values_per_method=5 missing=0 prototype_failures=0 timing_repetitions={TIMING_REPETITIONS}",
        measurements.len()
    );
}
