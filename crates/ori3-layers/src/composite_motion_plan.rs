//! Structural validation for a multi-region SIM-011 motion lowered as one step.

use std::collections::{HashMap, HashSet, VecDeque};

use glam::DVec2;
use ori3_geometry::Isometry2;
use ori3_model::{FaceId, TechniqueKind};

use crate::flat_motion::{FlatMotionInput, LayerTurn, MotionPart, MotionTransform};
use crate::fold_through::FoldDirection;

const PLAN_EPS: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanVertexKind { End, Bend, Branch }

#[derive(Clone, Debug)]
pub struct PlanVertex { pub id: usize, pub point: [f64; 2], pub kind: PlanVertexKind, pub incident_seams: Vec<usize> }

#[derive(Clone, Debug)]
pub struct PlanRegion { pub id: usize, pub part: MotionPart }

#[derive(Clone, Debug)]
pub struct PlanSeam {
    pub id: usize,
    pub endpoints: [usize; 2],
    pub support: [[f64; 2]; 2],
    pub left_region: usize,
    pub right_region: usize,
    pub direction: FoldDirection,
}

#[derive(Clone, Debug)]
pub struct CompositeMotionPlan { pub regions: Vec<PlanRegion>, pub seams: Vec<PlanSeam>, pub vertices: Vec<PlanVertex> }

impl CompositeMotionPlan {
    /// Validates local graph structure then produces exactly one `FlatMotionInput`.
    pub fn lower(&self, selected: &[FaceId]) -> Result<FlatMotionInput, String> {
        let selected: HashSet<FaceId> = selected.iter().copied().collect();
        if selected.is_empty() { return Err("selected packet is empty".to_string()); }
        let regions = indexed(&self.regions, |region| region.id, "region")?;
        let seams = indexed(&self.seams, |seam| seam.id, "seam")?;
        let vertices = indexed(&self.vertices, |vertex| vertex.id, "vertex")?;
        if regions.is_empty() || seams.is_empty() || vertices.is_empty() {
            return Err("composite motion needs regions, seams, and vertices".to_string());
        }
        for region in regions.values() {
            if region.part.layers.is_empty() { return Err(format!("region {} has implicit all-layer selection", region.id)); }
            if region.part.layers.iter().any(|id| !selected.contains(id)) { return Err(format!("region {} includes an unselected layer", region.id)); }
        }
        for face in &selected {
            if !regions.values().any(|region| region.part.layers.contains(face)) {
                return Err(format!("selected layer {face} is not assigned to any region"));
            }
        }
        let mut graph: HashMap<usize, Vec<usize>> = HashMap::new();
        for seam in seams.values() {
            let left = regions.get(&seam.left_region).ok_or_else(|| format!("seam {} has no left region", seam.id))?;
            let right = regions.get(&seam.right_region).ok_or_else(|| format!("seam {} has no right region", seam.id))?;
            if left.id == right.id { return Err(format!("seam {} joins a region to itself", seam.id)); }
            let [a, b] = seam.endpoints;
            let va = vertices.get(&a).ok_or_else(|| format!("seam {} has no start vertex", seam.id))?;
            let vb = vertices.get(&b).ok_or_else(|| format!("seam {} has no end vertex", seam.id))?;
            if !va.incident_seams.contains(&seam.id) || !vb.incident_seams.contains(&seam.id) { return Err(format!("seam {} is absent from an endpoint", seam.id)); }
            if !same_point(va.point, seam.support[0]) || !same_point(vb.point, seam.support[1]) { return Err(format!("seam {} support and vertices disagree", seam.id)); }
            if !support_is_boundary(&left.part, seam.support) || !support_is_boundary(&right.part, seam.support) { return Err(format!("seam {} is not a boundary of both regions", seam.id)); }
            let relative = part_isometry(&right.part)?.compose(&part_isometry(&left.part)?.inverse());
            let expected = Isometry2::reflection(DVec2::from(seam.support[0]), DVec2::from(seam.support[1]));
            if !relative.approx_eq(&expected, PLAN_EPS) { return Err(format!("seam {} does not join its regions by one reflection", seam.id)); }
            if ![left, right].iter().filter_map(|region| turn_direction(region.part.turn)).any(|direction| direction == seam.direction) { return Err(format!("seam {} has no matching mountain/valley direction", seam.id)); }
            graph.entry(left.id).or_default().push(right.id);
            graph.entry(right.id).or_default().push(left.id);
        }
        for vertex in vertices.values() {
            let degree = vertex.incident_seams.len();
            if degree == 0 || matches!(vertex.kind, PlanVertexKind::End) && degree != 1 || matches!(vertex.kind, PlanVertexKind::Bend) && degree != 2 || matches!(vertex.kind, PlanVertexKind::Branch) && degree < 3 {
                return Err(format!("vertex {} has an invalid degree", vertex.id));
            }
        }
        connected(&regions, &graph)?;
        Ok(FlatMotionInput { parts: self.regions.iter().map(|region| region.part.clone()).collect(), kind: TechniqueKind::Simple })
    }
}

fn indexed<'a, T>(items: &'a [T], id: impl Fn(&T) -> usize, label: &str) -> Result<HashMap<usize, &'a T>, String> {
    let mut out = HashMap::with_capacity(items.len());
    for item in items { let key = id(item); if out.insert(key, item).is_some() { return Err(format!("duplicate {label} id {key}")); } }
    Ok(out)
}
fn part_isometry(part: &MotionPart) -> Result<Isometry2, String> {
    match &part.transform {
        MotionTransform::Stay => Ok(Isometry2::identity()),
        MotionTransform::Isometry(iso) => Ok(*iso),
        MotionTransform::Reflect(lines) => lines.iter().try_fold(Isometry2::identity(), |acc, line| {
            let a = DVec2::from(line[0]); let b = DVec2::from(line[1]);
            ((b - a).length() > PLAN_EPS).then(|| Isometry2::reflection(a, b).compose(&acc)).ok_or_else(|| "reflection support is degenerate".to_string())
        }),
    }
}
fn turn_direction(turn: LayerTurn) -> Option<FoldDirection> { match turn { LayerTurn::Keep => None, LayerTurn::CreaseOnly(d) | LayerTurn::Outside(d) | LayerTurn::Inside(d) => Some(d), LayerTurn::Beside { direction, .. } => Some(direction) } }
fn same_point(a: [f64; 2], b: [f64; 2]) -> bool { (DVec2::from(a) - DVec2::from(b)).length() <= PLAN_EPS }
fn support_is_boundary(part: &MotionPart, support: [[f64; 2]; 2]) -> bool {
    let a = DVec2::from(support[0]); let b = DVec2::from(support[1]);
    part.region.iter().any(|half| { let h0 = DVec2::from(half.line[0]); let h1 = DVec2::from(half.line[1]); let d = h1 - h0; d.length() > PLAN_EPS && d.perp_dot(a - h0).abs() <= PLAN_EPS && d.perp_dot(b - h0).abs() <= PLAN_EPS })
}
fn connected(regions: &HashMap<usize, &PlanRegion>, graph: &HashMap<usize, Vec<usize>>) -> Result<(), String> {
    let Some(&start) = regions.keys().next() else { return Err("no regions".to_string()) };
    let mut seen = HashSet::new(); let mut queue = VecDeque::from([start]);
    while let Some(region) = queue.pop_front() { if seen.insert(region) { queue.extend(graph.get(&region).into_iter().flatten().copied()); } }
    (seen.len() == regions.len()).then_some(()).ok_or_else(|| "regions are not one connected motion".to_string())
}

#[cfg(test)]
mod tests {
    use super::{CompositeMotionPlan, PlanRegion, PlanSeam, PlanVertex, PlanVertexKind};
    use crate::flat_motion::{HalfPlane, LayerTurn, MotionPart, MotionTransform};
    use crate::fold_through::FoldDirection;
    use ori3_model::TechniqueKind;

    fn part(transform: MotionTransform, turn: LayerTurn) -> MotionPart {
        MotionPart {
            layers: vec![1],
            region: vec![HalfPlane {
                line: [[0.5, 0.0], [0.5, 1.0]],
                inside_point: [0.25, 0.5],
            }],
            transform,
            turn,
            reverse_layers: None,
        }
    }

    fn two_region_plan() -> CompositeMotionPlan {
        CompositeMotionPlan {
            regions: vec![
                PlanRegion { id: 0, part: part(MotionTransform::Stay, LayerTurn::Keep) },
                PlanRegion {
                    id: 1,
                    part: part(
                        MotionTransform::Reflect(vec![[[0.5, 0.0], [0.5, 1.0]]]),
                        LayerTurn::Outside(FoldDirection::Up),
                    ),
                },
            ],
            seams: vec![PlanSeam {
                id: 0,
                endpoints: [0, 1],
                support: [[0.5, 0.0], [0.5, 1.0]],
                left_region: 0,
                right_region: 1,
                direction: FoldDirection::Up,
            }],
            vertices: vec![
                PlanVertex { id: 0, point: [0.5, 0.0], kind: PlanVertexKind::End, incident_seams: vec![0] },
                PlanVertex { id: 1, point: [0.5, 1.0], kind: PlanVertexKind::End, incident_seams: vec![0] },
            ],
        }
    }

    #[test]
    fn lowers_a_connected_two_region_motion_into_one_simple_step() {
        let lowered = two_region_plan().lower(&[1]).unwrap();
        assert_eq!(lowered.kind, TechniqueKind::Simple);
        assert_eq!(lowered.parts.len(), 2);
    }

    #[test]
    fn rejects_a_seam_whose_regions_are_not_one_reflection_apart() {
        let mut plan = two_region_plan();
        plan.regions[1].part.transform = MotionTransform::Stay;
        assert!(plan.lower(&[1]).unwrap_err().contains("does not join"));
    }

    #[test]
    fn rejects_any_region_that_moves_an_unselected_layer() {
        let mut plan = two_region_plan();
        plan.regions[1].part.layers = vec![2];
        assert!(plan.lower(&[1]).unwrap_err().contains("unselected"));
    }

    #[test]
    fn rejects_a_misdeclared_branch_vertex() {
        let mut plan = two_region_plan();
        plan.vertices[0].kind = PlanVertexKind::Branch;
        assert!(plan.lower(&[1]).unwrap_err().contains("invalid degree"));
    }

    #[test]
    fn rejects_a_mountain_valley_reversal_without_a_split_seam() {
        let mut plan = two_region_plan();
        plan.seams[0].direction = FoldDirection::Down;
        assert!(plan.lower(&[1]).unwrap_err().contains("matching mountain/valley"));
    }
}
