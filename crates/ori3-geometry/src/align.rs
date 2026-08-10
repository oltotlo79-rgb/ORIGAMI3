//! Exact geometry for the eight origami alignment constructions.
//!
//! This is the Rust counterpart of `apps/desktop/src/lib/alignFold.ts`.  Keep
//! the equations, tolerances, candidate ordering, and final verification in
//! sync so an alignment selected in the UI can be reproduced in Rust without
//! an eyeballed coordinate.

use glam::DVec2;
use ori3_model::{AlignmentMode, AlignmentTarget};

/// Tolerance used for zero lengths and distances in normalized paper space.
pub const ALIGN_EPS: f64 = 1e-9;

/// A fold line represented by two points in the folded plane.
pub type FoldLine = [[f64; 2]; 2];

/// The kind of selection required by one stage of an alignment construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignmentTargetKind {
    Point,
    Line,
}

/// Which side of an oriented fold line contains the moving reference target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovingSide {
    Left,
    Right,
}

/// Result of the common eight-construction entry point.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignSolution {
    pub lines: Vec<FoldLine>,
    pub reason: Option<String>,
}

/// Required selections, in order, for an alignment construction.
#[must_use]
pub fn alignment_steps(mode: AlignmentMode) -> &'static [AlignmentTargetKind] {
    use AlignmentTargetKind::{Line, Point};

    match mode {
        AlignmentMode::ThroughTwoPoints | AlignmentMode::PointPoint => &[Point, Point],
        AlignmentMode::LineLine => &[Line, Line],
        AlignmentMode::PointPerpendicularLine => &[Point, Line],
        AlignmentMode::PointLineThrough => &[Point, Line, Point],
        AlignmentMode::PointToLinePointToLine => &[Point, Line, Point, Line],
        AlignmentMode::PointLinePerpendicular => &[Point, Line, Line],
        AlignmentMode::ExistingLine => &[Line],
    }
}

fn point(p: [f64; 2]) -> DVec2 {
    DVec2::from(p)
}

fn array(p: DVec2) -> [f64; 2] {
    [p.x, p.y]
}

fn line_length(line: FoldLine) -> f64 {
    (point(line[1]) - point(line[0])).length()
}

fn unit_vector(line: FoldLine) -> Option<DVec2> {
    let d = point(line[1]) - point(line[0]);
    let len = d.length();
    (len >= ALIGN_EPS).then(|| d / len)
}

/// Unit direction vector of a non-degenerate line.
#[must_use]
pub fn unit_dir(line: FoldLine) -> Option<[f64; 2]> {
    unit_vector(line).map(array)
}

/// Huzita-Hatori axiom 1: the line through two distinct points.
#[must_use]
pub fn line_through_points(p: [f64; 2], q: [f64; 2]) -> Option<FoldLine> {
    ((point(q) - point(p)).length() >= ALIGN_EPS).then_some([p, q])
}

fn segment_at(center: DVec2, direction: DVec2, half: f64) -> FoldLine {
    let half = half.max(ALIGN_EPS);
    [
        array(center - direction * half),
        array(center + direction * half),
    ]
}

/// Huzita-Hatori axiom 4: a line through `p`, perpendicular to `line`.
#[must_use]
pub fn perpendicular_through_point(p: [f64; 2], line: FoldLine) -> Option<FoldLine> {
    let u = unit_vector(line)?;
    let half = line_length(line).max(ALIGN_EPS);
    Some(segment_at(point(p), DVec2::new(-u.y, u.x), half))
}

fn line_intersection(first: FoldLine, second: FoldLine) -> Option<DVec2> {
    let r = point(first[1]) - point(first[0]);
    let s = point(second[1]) - point(second[0]);
    let denominator = r.perp_dot(s);
    let scale = r.length() * s.length();
    if scale < ALIGN_EPS || denominator.abs() <= ALIGN_EPS * scale {
        return None;
    }
    let t = (point(second[0]) - point(first[0])).perp_dot(s) / denominator;
    Some(point(first[0]) + r * t)
}

/// Perpendicular bisector of two distinct points (axiom 2).
#[must_use]
pub fn perpendicular_bisector(p: [f64; 2], q: [f64; 2]) -> Option<FoldLine> {
    let p = point(p);
    let q = point(q);
    let d = q - p;
    let len = d.length();
    if len < ALIGN_EPS {
        return None;
    }
    let midpoint = (p + q) * 0.5;
    let normal = DVec2::new(-d.y / len, d.x / len);
    Some(segment_at(midpoint, normal, len * 0.5))
}

fn foot_on_line(p: DVec2, line: FoldLine, unit: DVec2) -> DVec2 {
    let start = point(line[0]);
    start + unit * (p - start).dot(unit)
}

/// Angle bisectors that reflect `first` onto `second` (axiom 3).
///
/// Intersecting lines return the internal and external bisectors. Parallel
/// lines return their single midway parallel.
#[must_use]
pub fn angle_bisectors(first: FoldLine, second: FoldLine) -> Vec<FoldLine> {
    let Some(u1) = unit_vector(first) else {
        return Vec::new();
    };
    let Some(u2) = unit_vector(second) else {
        return Vec::new();
    };
    let scale = line_length(first).max(line_length(second));
    let denominator = u1.perp_dot(u2);
    if denominator.abs() < ALIGN_EPS {
        let foot = foot_on_line(point(first[0]), second, u2);
        let midpoint = (point(first[0]) + foot) * 0.5;
        return vec![segment_at(midpoint, u1, scale)];
    }

    let t = (point(second[0]) - point(first[0])).perp_dot(u2) / denominator;
    let intersection = point(first[0]) + u1 * t;
    [u1 + u2, u1 - u2]
        .into_iter()
        .filter_map(|direction| {
            let len = direction.length();
            (len >= ALIGN_EPS).then(|| segment_at(intersection, direction / len, scale))
        })
        .collect()
}

/// Axiom 5: folds through `through` that place `p` onto `line` (zero to two).
#[must_use]
pub fn fold_point_onto_line(p: [f64; 2], line: FoldLine, through: [f64; 2]) -> Vec<FoldLine> {
    let Some(u) = unit_vector(line) else {
        return Vec::new();
    };
    let p_vec = point(p);
    let through_vec = point(through);
    let foot = foot_on_line(through_vec, line, u);
    let distance = (foot - through_vec).length();
    let radius = (p_vec - through_vec).length();
    if radius < ALIGN_EPS || distance > radius + ALIGN_EPS {
        return Vec::new();
    }
    let half_chord = (radius * radius - distance * distance).max(0.0).sqrt();
    let hits = if half_chord <= ALIGN_EPS {
        vec![foot]
    } else {
        vec![foot + u * half_chord, foot - u * half_chord]
    };
    hits.into_iter()
        .filter_map(|q| perpendicular_bisector(p, array(q)))
        .collect()
}

/// Reflect a point across a fold line.
#[must_use]
pub fn reflect_point_across_fold(p: [f64; 2], fold: FoldLine) -> Option<[f64; 2]> {
    let u = unit_vector(fold)?;
    let p = point(p);
    Some(array(foot_on_line(p, fold, u) * 2.0 - p))
}

#[derive(Clone, Copy)]
struct LineEquation {
    normal: DVec2,
    distance: f64,
}

fn line_equation(line: FoldLine) -> Option<LineEquation> {
    let u = unit_vector(line)?;
    let normal = DVec2::new(-u.y, u.x);
    Some(LineEquation {
        normal,
        distance: normal.dot(point(line[0])),
    })
}

type Polynomial = Vec<f64>;

fn poly_scale(poly: &[f64], scale: f64) -> Polynomial {
    poly.iter().map(|value| value * scale).collect()
}

fn poly_sub(first: &[f64], second: &[f64]) -> Polynomial {
    (0..first.len().max(second.len()))
        .map(|i| first.get(i).copied().unwrap_or(0.0) - second.get(i).copied().unwrap_or(0.0))
        .collect()
}

fn poly_mul(first: &[f64], second: &[f64]) -> Polynomial {
    let mut out = vec![0.0; first.len() + second.len() - 1];
    for (i, a) in first.iter().enumerate() {
        for (j, b) in second.iter().enumerate() {
            out[i + j] += a * b;
        }
    }
    out
}

fn poly_value(poly: &[f64], x: f64) -> f64 {
    poly.iter().rev().fold(0.0, |out, value| out * x + value)
}

fn poly_derivative_value(poly: &[f64], x: f64) -> f64 {
    (1..poly.len())
        .rev()
        .fold(0.0, |out, i| out * x + i as f64 * poly[i])
}

fn unique_roots(mut values: Vec<f64>) -> Vec<f64> {
    values.retain(|value| value.is_finite());
    values.sort_by(f64::total_cmp);
    let mut out: Vec<f64> = Vec::new();
    for value in values {
        if out
            .last()
            .is_none_or(|previous| (value - previous).abs() > 1e-8 * value.abs().max(1.0))
        {
            out.push(value);
        }
    }
    out
}

/// Real roots of a polynomial of degree at most three. This mirrors the
/// Cardano-plus-Newton implementation in the desktop alignment solver.
fn real_polynomial_roots(input: &[f64]) -> Vec<f64> {
    let mut coefficients = input.to_vec();
    // JavaScript Number.MIN_VALUE, i.e. the least positive subnormal f64.
    let min_value = f64::from_bits(1);
    let scale = coefficients
        .iter()
        .map(|value| value.abs())
        .fold(min_value, f64::max);
    while coefficients.len() > 1
        && coefficients.last().copied().unwrap_or(0.0).abs() <= 64.0 * f64::EPSILON * scale
    {
        coefficients.pop();
    }

    let degree = coefficients.len().saturating_sub(1);
    let roots = match degree {
        1 => vec![-coefficients[0] / coefficients[1]],
        2 => {
            let (c, b, a) = (coefficients[0], coefficients[1], coefficients[2]);
            let discriminant = b * b - 4.0 * a * c;
            let tolerance =
                64.0 * f64::EPSILON * min_value.max((b * b).abs()).max((4.0 * a * c).abs());
            if discriminant < -tolerance {
                Vec::new()
            } else {
                let root = discriminant.max(0.0).sqrt();
                if root <= tolerance.sqrt() {
                    vec![-b / (2.0 * a)]
                } else {
                    vec![(-b - root) / (2.0 * a), (-b + root) / (2.0 * a)]
                }
            }
        }
        3 => {
            let (d, c, b, a) = (
                coefficients[0],
                coefficients[1],
                coefficients[2],
                coefficients[3],
            );
            let aa = b / a;
            let bb = c / a;
            let cc = d / a;
            let p = bb - aa * aa / 3.0;
            let q = 2.0 * aa * aa * aa / 27.0 - aa * bb / 3.0 + cc;
            let q_term = q * q / 4.0;
            let p_term = p * p * p / 27.0;
            let discriminant = q_term + p_term;
            let tolerance = 64.0 * f64::EPSILON * min_value.max(q_term.abs()).max(p_term.abs());
            if discriminant > tolerance {
                let root = discriminant.sqrt();
                vec![(-q / 2.0 + root).cbrt() + (-q / 2.0 - root).cbrt() - aa / 3.0]
            } else if discriminant >= -tolerance {
                let u = (-q / 2.0).cbrt();
                vec![2.0 * u - aa / 3.0, -u - aa / 3.0]
            } else {
                let radius = 2.0 * (-p / 3.0).sqrt();
                let denominator = 2.0 * (-(p * p * p) / 27.0).sqrt();
                let angle = (-q / denominator).clamp(-1.0, 1.0).acos();
                (0..3)
                    .map(|k| {
                        radius * ((angle + 2.0 * std::f64::consts::PI * k as f64) / 3.0).cos()
                            - aa / 3.0
                    })
                    .collect()
            }
        }
        _ => Vec::new(),
    };

    let refined = roots
        .into_iter()
        .map(|initial| {
            let mut x = initial;
            for _ in 0..6 {
                let derivative = poly_derivative_value(&coefficients, x);
                if !derivative.is_finite() || derivative.abs() <= 1e-14 {
                    break;
                }
                let next = x - poly_value(&coefficients, x) / derivative;
                if !next.is_finite() {
                    break;
                }
                x = next;
            }
            x
        })
        .collect();
    unique_roots(refined)
}

struct NormalFold {
    line: FoldLine,
    normal: DVec2,
    distance: f64,
}

fn canonical_normal(normal: DVec2, distance: f64) -> (DVec2, f64) {
    if normal.x < -ALIGN_EPS || (normal.x.abs() <= ALIGN_EPS && normal.y < 0.0) {
        (-normal, -distance)
    } else {
        (normal, distance)
    }
}

enum Offset {
    Impossible,
    Unconstrained,
    Value(f64),
}

fn fold_offset(p: DVec2, equation: LineEquation, normal: DVec2) -> Offset {
    let delta = equation.normal.dot(p) - equation.distance;
    let denominator = equation.normal.dot(normal);
    if denominator.abs() <= ALIGN_EPS {
        if delta.abs() <= ALIGN_EPS {
            Offset::Unconstrained
        } else {
            Offset::Impossible
        }
    } else {
        Offset::Value(normal.dot(p) - delta / (2.0 * denominator))
    }
}

fn simultaneous_candidate(
    normal: DVec2,
    p1: [f64; 2],
    line1: FoldLine,
    equation1: LineEquation,
    p2: [f64; 2],
    line2: FoldLine,
    equation2: LineEquation,
) -> Option<NormalFold> {
    let length = normal.length();
    if length < ALIGN_EPS {
        return None;
    }
    let normal = normal / length;
    let offset1 = fold_offset(point(p1), equation1, normal);
    let offset2 = fold_offset(point(p2), equation2, normal);
    let distance = match (offset1, offset2) {
        (Offset::Impossible, _) | (_, Offset::Impossible) => return None,
        (Offset::Unconstrained, Offset::Unconstrained) => return None,
        (Offset::Unconstrained, Offset::Value(value))
        | (Offset::Value(value), Offset::Unconstrained) => value,
        (Offset::Value(first), Offset::Value(second)) => {
            if (first - second).abs() > 2e-7 {
                return None;
            }
            (first + second) * 0.5
        }
    };

    let base = normal * distance;
    let fold = segment_at(base, DVec2::new(-normal.y, normal.x), 1.0);
    let reflected1 = reflect_point_across_fold(p1, fold)?;
    let reflected2 = reflect_point_across_fold(p2, fold)?;
    if distance_to_line(line1, reflected1) > 2e-7 || distance_to_line(line2, reflected2) > 2e-7 {
        return None;
    }
    if (point(reflected1) - point(p1)).length() <= ALIGN_EPS
        && (point(reflected2) - point(p2)).length() <= ALIGN_EPS
    {
        return None;
    }
    let (normal, distance) = canonical_normal(normal, distance);
    Some(NormalFold {
        line: fold,
        normal,
        distance,
    })
}

/// Axiom 6: folds placing two points onto two lines simultaneously (zero to
/// three solutions).
#[must_use]
pub fn fold_two_points_onto_two_lines(
    p1: [f64; 2],
    line1: FoldLine,
    p2: [f64; 2],
    line2: FoldLine,
) -> Vec<FoldLine> {
    let Some(equation1) = line_equation(line1) else {
        return Vec::new();
    };
    let Some(equation2) = line_equation(line2) else {
        return Vec::new();
    };
    let delta1 = equation1.normal.dot(point(p1)) - equation1.distance;
    let delta2 = equation2.normal.dot(point(p2)) - equation2.distance;
    let first = vec![equation1.normal.x, equation1.normal.y];
    let second = vec![equation2.normal.x, equation2.normal.y];
    let displacement = vec![p1[0] - p2[0], p1[1] - p2[1]];
    // 2(n·(p1-p2))(m1·n)(m2·n) - |n|²(δ1(m2·n)-δ2(m1·n)) = 0.
    let cubic = poly_sub(
        &poly_scale(&poly_mul(&poly_mul(&displacement, &first), &second), 2.0),
        &poly_mul(
            &[1.0, 0.0, 1.0],
            &poly_sub(&poly_scale(&second, delta1), &poly_scale(&first, delta2)),
        ),
    );

    let mut candidates: Vec<NormalFold> = Vec::new();
    let mut add_candidate = |normal: DVec2| {
        let Some(candidate) =
            simultaneous_candidate(normal, p1, line1, equation1, p2, line2, equation2)
        else {
            return;
        };
        let duplicate = candidates.iter().any(|other| {
            (other.normal - candidate.normal).length() <= 2e-7
                && (other.distance - candidate.distance).abs() <= 2e-7
        });
        if !duplicate {
            candidates.push(candidate);
        }
    };
    for root in real_polynomial_roots(&cubic) {
        add_candidate(DVec2::new(1.0, root));
    }
    // n.x=0 is the root at infinity and does not occur among finite cubic roots.
    add_candidate(DVec2::Y);
    candidates
        .into_iter()
        .map(|candidate| candidate.line)
        .collect()
}

/// Axiom 7: place `p` onto `target` while making the fold perpendicular to
/// `perpendicular_to`.
#[must_use]
pub fn fold_point_onto_line_perpendicular(
    p: [f64; 2],
    target: FoldLine,
    perpendicular_to: FoldLine,
) -> Option<FoldLine> {
    let direction = unit_vector(perpendicular_to)?;
    let destination = line_intersection([p, array(point(p) + direction)], target)?;
    perpendicular_bisector(p, array(destination))
}

/// Validate and return an existing non-degenerate crease (the eighth mode).
#[must_use]
pub fn existing_line(line: FoldLine) -> Option<FoldLine> {
    unit_vector(line).map(|_| line)
}

/// Extend a line to length `2 * half`, preserving its supporting line.
#[must_use]
pub fn extend_line(line: FoldLine, half: f64) -> FoldLine {
    let Some(direction) = unit_vector(line) else {
        return line;
    };
    let midpoint = (point(line[0]) + point(line[1])) * 0.5;
    segment_at(midpoint, direction, half)
}

/// Representative point of a selected point or line.
#[must_use]
pub fn align_ref_point(target: &AlignmentTarget) -> [f64; 2] {
    match target {
        AlignmentTarget::Point { p } => *p,
        AlignmentTarget::Line { a, b } => [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5],
    }
}

/// Side of an oriented line containing `reference`; points on the line use
/// the same default (`Right`) as the desktop implementation.
#[must_use]
pub fn moving_side_of(line: FoldLine, reference: [f64; 2]) -> MovingSide {
    let Some(direction) = unit_vector(line) else {
        return MovingSide::Right;
    };
    if direction.perp_dot(point(reference) - point(line[0])) > ALIGN_EPS {
        MovingSide::Left
    } else {
        MovingSide::Right
    }
}

/// Distance from a point to an infinite line.
#[must_use]
pub fn distance_to_line(line: FoldLine, p: [f64; 2]) -> f64 {
    let Some(direction) = unit_vector(line) else {
        return (point(p) - point(line[0])).length();
    };
    direction.perp_dot(point(p) - point(line[0])).abs()
}

/// Sort candidate folds by distance from the cursor. No cursor preserves the
/// solver's candidate order.
#[must_use]
pub fn sort_by_cursor(mut lines: Vec<FoldLine>, cursor: Option<[f64; 2]>) -> Vec<FoldLine> {
    if let Some(cursor) = cursor {
        lines.sort_by(|first, second| {
            distance_to_line(*first, cursor).total_cmp(&distance_to_line(*second, cursor))
        });
    }
    lines
}

fn target_point(target: Option<&AlignmentTarget>) -> Option<[f64; 2]> {
    match target? {
        AlignmentTarget::Point { p } => Some(*p),
        AlignmentTarget::Line { .. } => None,
    }
}

fn target_line(target: Option<&AlignmentTarget>) -> Option<FoldLine> {
    match target? {
        AlignmentTarget::Line { a, b } => Some([*a, *b]),
        AlignmentTarget::Point { .. } => None,
    }
}

fn no_solution(reason: &str) -> AlignSolution {
    AlignSolution {
        lines: Vec::new(),
        reason: Some(reason.to_string()),
    }
}

/// Common entry point for all eight exact constructions.
#[must_use]
pub fn solve_align(
    mode: AlignmentMode,
    picks: &[AlignmentTarget],
    cursor: Option<[f64; 2]>,
) -> AlignSolution {
    if picks.len() < alignment_steps(mode).len() {
        return AlignSolution {
            lines: Vec::new(),
            reason: None,
        };
    }

    let lines = match mode {
        AlignmentMode::ThroughTwoPoints => {
            let (Some(first), Some(second)) =
                (target_point(picks.first()), target_point(picks.get(1)))
            else {
                return no_solution("選んだ対象の種類が合いません。やり直してください");
            };
            let Some(line) = line_through_points(first, second) else {
                return no_solution("2つの点が同じ位置です。別の点を選んでください");
            };
            vec![line]
        }
        AlignmentMode::PointPoint => {
            let (Some(first), Some(second)) =
                (target_point(picks.first()), target_point(picks.get(1)))
            else {
                return no_solution("選んだ対象の種類が合いません。やり直してください");
            };
            let Some(line) = perpendicular_bisector(first, second) else {
                return no_solution("2つの点が同じ位置です。別の点を選んでください");
            };
            vec![line]
        }
        AlignmentMode::LineLine => {
            let (Some(first), Some(second)) =
                (target_line(picks.first()), target_line(picks.get(1)))
            else {
                return no_solution("選んだ対象の種類が合いません。やり直してください");
            };
            let lines = angle_bisectors(first, second);
            if lines.is_empty() {
                return no_solution("選んだ線の長さが0です。別の線を選んでください");
            }
            lines
        }
        AlignmentMode::PointPerpendicularLine => {
            let (Some(p), Some(source)) = (target_point(picks.first()), target_line(picks.get(1)))
            else {
                return no_solution("選んだ対象の種類が合いません。やり直してください");
            };
            let Some(line) = perpendicular_through_point(p, source) else {
                return no_solution("選んだ線の長さが0です。別の線を選んでください");
            };
            vec![line]
        }
        AlignmentMode::PointLineThrough => {
            let (Some(p), Some(target), Some(through)) = (
                target_point(picks.first()),
                target_line(picks.get(1)),
                target_point(picks.get(2)),
            ) else {
                return no_solution("選んだ対象の種類が合いません。やり直してください");
            };
            let lines = fold_point_onto_line(p, target, through);
            if lines.is_empty() {
                return no_solution(
                    "この点を通る折り方では届きません(折り目が通る点をもっと線の近くに選んでください)",
                );
            }
            lines
        }
        AlignmentMode::PointToLinePointToLine => {
            let (Some(p1), Some(line1), Some(p2), Some(line2)) = (
                target_point(picks.first()),
                target_line(picks.get(1)),
                target_point(picks.get(2)),
                target_line(picks.get(3)),
            ) else {
                return no_solution("選んだ対象の種類が合いません。やり直してください");
            };
            let lines = fold_two_points_onto_two_lines(p1, line1, p2, line2);
            if lines.is_empty() {
                return no_solution(
                    "この2組を同時に合わせる折り目はありません。別の点や線を選んでください",
                );
            }
            lines
        }
        AlignmentMode::PointLinePerpendicular => {
            let (Some(p), Some(target), Some(perpendicular_to)) = (
                target_point(picks.first()),
                target_line(picks.get(1)),
                target_line(picks.get(2)),
            ) else {
                return no_solution("選んだ対象の種類が合いません。やり直してください");
            };
            let Some(line) = fold_point_onto_line_perpendicular(p, target, perpendicular_to) else {
                return no_solution("点を合わせながら垂直にできません。別の点や線を選んでください");
            };
            vec![line]
        }
        AlignmentMode::ExistingLine => {
            let Some(selected) = target_line(picks.first()) else {
                return no_solution("選んだ対象の種類が合いません。やり直してください");
            };
            let Some(line) = existing_line(selected) else {
                return no_solution("選んだ線の長さが0です。別の線を選んでください");
            };
            vec![line]
        }
    };

    AlignSolution {
        lines: sort_by_cursor(lines, cursor)
            .into_iter()
            .map(|line| extend_line(line, 1.0))
            .collect(),
        reason: None,
    }
}
