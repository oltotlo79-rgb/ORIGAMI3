//! Browserless acceptance measurements for the rigid surface-order contract.
//!
//! The raster path intentionally follows Viewer3D's Float32 projection,
//! 24-bit depth target, two-code depth tolerance, and surface-owner ordering.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, Neg, Sub};

use ori3_cp::{Face, extract_faces};
use ori3_model::{
    CreasePattern, Document, Driver, Edge, EdgeId, EdgeKind, FaceId, Frame3D, Vertex,
};
use ori3_rigid::{
    MotionContactOptions, SurfaceOrderSource, propagate, solve_motion,
    solve_motion_with_contact_options, to_frame3d,
};

const CAMERA_FOV_DEG: f64 = 45.0;
const CAMERA_MARGIN: f64 = 1.35;
const CAMERA_NEAR: f64 = 0.01;
const CAMERA_FAR: f64 = 100.0;
const DEPTH_BITS: u32 = 24;
const DEPTH_TIE_CODES: u32 = 2;
const VIEWPORT: usize = 800;
const RASTER_EPS: f32 = 1e-10;
const SURFACE_OWNER_PLANARITY_EPSILON: f64 = 1e-6;
const SURFACE_OWNER_COPLANAR_EPSILON: f64 = 1e-6;
const SURFACE_OWNER_NORMAL_EPSILON: f64 = 1e-6;
const EXPLICIT_CONTACT_PREVENTION: MotionContactOptions = MotionContactOptions {
    detect: true,
    prevent: true,
};

const WARMUP_ABS: [f64; 19] = [
    0.0, 9.0, 19.0, 29.0, 39.0, 49.0, 59.0, 69.0, 79.0, 90.0, 101.0, 111.0, 121.0, 131.0, 141.0,
    151.0, 161.0, 171.0, 179.0,
];
const BOUNDARY_ABS: [f64; 5] = [179.5, 179.9, 179.99, 179.999, 180.0];

const FOLDED_SAMPLE: &str =
    include_str!("../../../../crates/ori3-layers/tests/fixtures/folded-sample.ori3");

#[derive(Clone, Copy, Debug, Default)]
struct V3 {
    x: f64,
    y: f64,
    z: f64,
}

impl V3 {
    const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    const Y: Self = Self::new(0.0, 1.0, 0.0);
    const Z: Self = Self::new(0.0, 0.0, 1.0);

    const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    fn length_squared(self) -> f64 {
        self.dot(self)
    }

    fn normalize(self) -> Self {
        self / self.length_squared().sqrt()
    }
}

impl Add for V3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl AddAssign for V3 {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Sub for V3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Mul<f64> for V3 {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Div<f64> for V3 {
    type Output = Self;

    fn div(self, rhs: f64) -> Self::Output {
        self * (1.0 / rhs)
    }
}

impl DivAssign<f64> for V3 {
    fn div_assign(&mut self, rhs: f64) {
        *self = *self / rhs;
    }
}

impl Neg for V3 {
    type Output = Self;

    fn neg(self) -> Self::Output {
        self * -1.0
    }
}

#[derive(Clone)]
struct Diagram {
    name: &'static str,
    cp: CreasePattern,
    faces: Vec<Face>,
    hinges: Vec<(EdgeId, EdgeKind)>,
    paper_width: f64,
    paper_height: f64,
    triangles: BTreeMap<FaceId, Vec<[usize; 3]>>,
    owner_codes: BTreeMap<FaceId, i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct VisualImage {
    visible_faces: BTreeSet<FaceId>,
    visible_back_faces: BTreeSet<FaceId>,
    red_pixels: u64,
    light_pixels: u64,
    pixels: Vec<Option<VisualPixel>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VisualPixel {
    face: FaceId,
    back_facing: bool,
}

impl VisualImage {
    fn red_ratio(&self) -> f64 {
        let total = self.red_pixels + self.light_pixels;
        if total == 0 {
            0.0
        } else {
            self.red_pixels as f64 / total as f64
        }
    }

    fn difference(&self, other: &Self, viewport: usize) -> VisualDifference {
        assert_eq!(self.pixels.len(), other.pixels.len());
        assert_eq!(self.pixels.len(), viewport * viewport);
        let mut owner_pixels = 0;
        let mut color_pixels = 0;
        let mut coverage_pixels = 0;
        let mut side_pixels = 0;
        let mut face_only_pixels = 0;
        let mut min_x = viewport;
        let mut min_y = viewport;
        let mut max_x = 0;
        let mut max_y = 0;
        for (pixel, (before, after)) in self.pixels.iter().zip(&other.pixels).enumerate() {
            if before != after {
                owner_pixels += 1;
            }
            let color_changed = match (before, after) {
                (None, None) => false,
                (None, Some(_)) | (Some(_), None) => {
                    coverage_pixels += 1;
                    true
                }
                (Some(before), Some(after)) if before.back_facing != after.back_facing => {
                    side_pixels += 1;
                    true
                }
                (Some(before), Some(after)) => {
                    if before.face != after.face {
                        face_only_pixels += 1;
                    }
                    false
                }
            };
            if color_changed {
                color_pixels += 1;
                let x = pixel % viewport;
                let y = pixel / viewport;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
        VisualDifference {
            owner_pixels,
            color_pixels,
            coverage_pixels,
            side_pixels,
            face_only_pixels,
            color_bounds: (color_pixels > 0).then_some((min_x, min_y, max_x, max_y)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VisualDifference {
    owner_pixels: usize,
    color_pixels: usize,
    coverage_pixels: usize,
    side_pixels: usize,
    face_only_pixels: usize,
    color_bounds: Option<(usize, usize, usize, usize)>,
}

#[derive(Clone)]
struct EndpointState {
    frame: Frame3D,
    angles: HashMap<EdgeId, f64>,
}

#[derive(Clone, Copy)]
struct Camera {
    position: V3,
    view_x: [f32; 4],
    view_y: [f32; 4],
    view_depth: [f32; 4],
    projection_scale: f32,
    projection_depth_a: f32,
    projection_depth_b: f32,
}

#[derive(Clone, Copy)]
struct Projected {
    x: f32,
    y: f32,
    depth: f32,
}

struct RenderFace {
    face: FaceId,
    owner_code: i64,
    surface_rank: u32,
    side: i64,
    material_orientation: i64,
    plane_normal: V3,
    plane_distance: f64,
    planar: bool,
    /// 支持平面ごとに付ける組の符号。0は平面をまとめられなかったこと。
    /// Viewer3Dの `surfaceOwnerGroupToken` と同じ規則で付ける。
    coplanar_group: u32,
    points: Vec<V3>,
    triangles: Vec<RenderTriangle>,
}

struct RenderTriangle {
    projected: [Projected; 3],
    depth_plane: Option<[f32; 3]>,
    back_facing: bool,
}

#[path = "surface_order_sa_fixture.rs"]
mod surface_order_sa_fixture;
use surface_order_sa_fixture::*;
#[path = "surface_order_sa_raster.rs"]
mod surface_order_sa_raster;
use surface_order_sa_raster::*;
/// `BOUNDARY_ABS`(179.5°〜180°)を順に送った姿勢を全て返す。
///
/// 180°の手前の4点は面どうしが実際に離れているので、重なりの上下を
/// **姿勢そのものから測れる**。`endpoint_frames` はここから両端点だけを取り出す。
fn boundary_ladder(diagram: &Diagram, hinge: EdgeId, sign: f64) -> Vec<(f64, EndpointState)> {
    let mut warm = None::<HashMap<EdgeId, f64>>;
    for absolute in WARMUP_ABS {
        let motion = solve_motion_with_contact_options(
            &diagram.cp,
            &diagram.faces,
            &[Driver {
                hinge,
                target_angle_deg: sign * absolute,
            }],
            None,
            warm.as_ref(),
            MotionContactOptions {
                detect: true,
                prevent: true,
            },
        );
        warm = Some(motion.result.angles);
    }

    let mut ladder = Vec::with_capacity(BOUNDARY_ABS.len());
    for absolute in BOUNDARY_ABS {
        let motion = solve_motion_with_contact_options(
            &diagram.cp,
            &diagram.faces,
            &[Driver {
                hinge,
                target_angle_deg: sign * absolute,
            }],
            None,
            warm.as_ref(),
            MotionContactOptions {
                detect: true,
                prevent: true,
            },
        );
        assert!(
            motion
                .result
                .frame
                .faces
                .iter()
                .flat_map(|face| &face.polygon)
                .flatten()
                .all(|coordinate| coordinate.is_finite()),
            "{} edge {hinge} at {} degrees returned non-finite geometry",
            diagram.name,
            sign * absolute
        );
        warm = Some(motion.result.angles.clone());
        ladder.push((
            absolute,
            EndpointState {
                frame: motion.result.frame,
                angles: motion.result.angles,
            },
        ));
    }
    ladder
}

fn endpoint_frames(diagram: &Diagram, hinge: EdgeId, sign: f64) -> (EndpointState, EndpointState) {
    let mut ladder = boundary_ladder(diagram, hinge, sign);
    let after = ladder
        .pop()
        .expect("boundary samples include 180 degrees")
        .1;
    let before = ladder
        .pop()
        .expect("boundary samples include 179.999 degrees")
        .1;
    (before, after)
}

fn ids(values: &BTreeSet<FaceId>) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn surface_rank_order(frame: &Frame3D) -> Vec<FaceId> {
    let mut ranks = frame
        .faces
        .iter()
        .map(|face| (face.surface_rank, face.face))
        .collect::<Vec<_>>();
    ranks.sort_unstable();
    ranks.into_iter().map(|(_, face)| face).collect()
}

fn max_vertex_distance(before: &Frame3D, after: &Frame3D) -> f64 {
    let after_faces = after
        .faces
        .iter()
        .map(|face| (face.face, face))
        .collect::<BTreeMap<_, _>>();
    before
        .faces
        .iter()
        .flat_map(|before_face| {
            let after_face = after_faces
                .get(&before_face.face)
                .expect("endpoint frames contain the same faces");
            assert_eq!(before_face.polygon.len(), after_face.polygon.len());
            before_face.polygon.iter().zip(&after_face.polygon).map(
                |(before_vertex, after_vertex)| {
                    before_vertex
                        .iter()
                        .zip(after_vertex)
                        .map(|(before_coordinate, after_coordinate)| {
                            (before_coordinate - after_coordinate).powi(2)
                        })
                        .sum::<f64>()
                        .sqrt()
                },
            )
        })
        .fold(0.0, f64::max)
}

/// 面対の「上」を測る軸が、その姿勢で紙の**巻き方向**とどちらへそろっているか。
///
/// `surface_rank` は「その面対が乗る平面の正準法線の向きに、下から上へ」並べた順で
/// ある(製品側 `surface_order.rs::derive_surface_order_with` は、面対のうち
/// **先に来るほうの面**の平面を使う。`near_overlaps` も同じ並びで面対を作る)。
/// 正準法線は `n` と `−n` のうち絶対値が最大の成分が正になる側で、対称に折り切った
/// 形では2成分の絶対値が厳密に等しくなり、姿勢が 6.6e-6 動くだけで支配的な成分が
/// 入れ替わる。そのとき順位も入れ替わるが、**紙の重なり方は同じ**である。
///
/// 実測(`diag_kome_edge12_canonical_axis` / `diag_kome_edge12_float32_axis_choice`、
/// `diagonal-midline-square` の辺12、面3と面4):
///
/// | 姿勢 | 面3の法線 | \|x\|−\|y\| (f64) | \|x\|−\|y\| (Float32) |
/// |---|---|---:|---:|
/// | 179.999° | (0.598072673372, −0.598069066507, 0.533500205298) | +3.607e-6 | +3.603e-6 |
/// | 180° | (0.598072754669, −0.598072755698, 0.533495978442) | −1.028e-9 | −6.118e-8 |
///
/// 裂けはどちらも 3.1e-15 以下(許容 1e-6)で、面4は両方の姿勢で面3の紙の裏側に
/// あり(179°〜179.999°で −2.057e-3 〜 −2.057e-6、符号は一定)、物理的な上下は
/// 変わっていない。画面側(Float32でも符号は同じ)も同じ式で `side` を決めるので
/// 描かれる絵も変わらない。Rust側にだけ「ほぼ同じなら軸の優先順」の帯を入れると
/// 画面側と食い違い、裏が31,991画素増えることを実測しているので、正準法線の式は
/// 変えない。**代わりに上下を比べるときだけ、姿勢によらない巻き方向へ直す。**
/// 巻き方向の法線は展開図の面の定義だけで決まるので、姿勢が変わっても意味が
/// 変わらない。
fn winding_sign(polygons: &BTreeMap<FaceId, Vec<V3>>, left: FaceId) -> Option<f64> {
    let normal = polygon_normal3(polygons.get(&left)?)?;
    Some(if normal.dot(canonical3(normal)) >= 0.0 {
        1.0
    } else {
        -1.0
    })
}

/// 刻印された重なり順を、面対ごとに紙の巻き方向へそろえて読む。
/// 値が `true` なら `right` は `left` の巻き方向の側にある。
fn material_sides(frame: &Frame3D, stacks: &[DeterminedStack]) -> BTreeMap<(FaceId, FaceId), bool> {
    let polygons = frame_polygons(frame);
    let rank = frame
        .faces
        .iter()
        .map(|face| (face.face, face.surface_rank))
        .collect::<BTreeMap<_, _>>();
    stacks
        .iter()
        .filter_map(|stack| {
            let sign = winding_sign(&polygons, stack.left)?;
            let above = rank.get(&stack.right)? > rank.get(&stack.left)?;
            Some((
                (stack.left, stack.right),
                if sign >= 0.0 { above } else { !above },
            ))
        })
        .collect()
}

/// 2つの姿勢の間で、紙の重なり方そのものが入れ替わった面対を返す。
///
/// 上下は各姿勢の**巻き方向**で読むので、正準法線の反転だけでは入れ替わらない。
/// `situation` は不合格の説明に差し込む「どこの間で入れ替わったか」である。
fn stacks_that_differ(
    before: &Frame3D,
    after: &Frame3D,
    stacks: &[DeterminedStack],
    situation: &str,
) -> Vec<(FaceId, FaceId, String)> {
    let before_sides = material_sides(before, stacks);
    let after_sides = material_sides(after, stacks);
    stacks
        .iter()
        .filter_map(|stack| {
            let key = (stack.left, stack.right);
            let (Some(before_side), Some(after_side)) =
                (before_sides.get(&key), after_sides.get(&key))
            else {
                return None;
            };
            (before_side != after_side).then(|| {
                (
                    stack.left,
                    stack.right,
                    format!(
                        "実測{}段(隙間 {:.3e}〜{:.3e})では面{}が面{}の同じ側にあり続けるのに、\
                         {situation}で紙の重なり方が入れ替わった",
                        stack.samples,
                        stack.smallest_gap,
                        stack.largest_gap,
                        stack.right,
                        stack.left,
                    ),
                )
            })
        })
        .collect()
}

/// 179.999°と180°で、紙の重なり方そのものが入れ替わった面対を返す。
fn stacks_that_flip_between_endpoints(
    before: &Frame3D,
    after: &Frame3D,
    stacks: &[DeterminedStack],
) -> Vec<(FaceId, FaceId, String)> {
    stacks_that_differ(before, after, stacks, "179.999°と180°")
}

/// 入力の展開図の頂点座標を、全て1 ULPだけ大きい側へ動かした複製。
///
/// 作品ファイルの小数の読み方(`serde_json` の `float_roundtrip`)や計算機の違いで
/// 実際に起きる大きさの入力差である。同じ検査をこの複製でも走らせ、答えが変わる
/// 面対を主張の対象から外すために使う。
fn one_ulp_nudged(source: &Diagram) -> Diagram {
    let mut cp = source.cp.clone();
    for vertex in &mut cp.vertices {
        for coordinate in &mut vertex.pos {
            *coordinate = f64::from_bits(coordinate.to_bits() + 1);
        }
    }
    diagram(source.name, cp, source.paper_width, source.paper_height)
}

/// 梯子の実測で上下が決まり、**入力を1 ULP動かしても同じ答えになる**面対だけを返す。
///
/// 1 ULPで揺れる量の実測(`diag_gap_noise_from_one_ulp_of_input`。
/// `folded-sample.ori3` の6本×2方向、179.999°の姿勢で対応の取れた467組):
/// 隙間の差は中央値 **1.399e-10**、p90 **3.102e-7**、p99 **2.267e-5**、
/// 最大 **6.829e-5**。うち**6組は符号ごと反転**した(いちばん大きいもので
/// 3.359e-5 → −1.668e-5)。隙間の大きさだけでは丸めの揺れと区別できないので、
/// 「1 ULP動かしても答えが変わらないこと」を条件にする。
fn rounding_robust_stacks(
    base: &Diagram,
    base_ladder: &[(f64, EndpointState)],
    nudged: &Diagram,
    hinge: EdgeId,
    sign: f64,
) -> Vec<DeterminedStack> {
    let nudged_ladder = boundary_ladder(nudged, hinge, sign);
    let nudged_stacks = determined_stacks(&nudged.cp, &nudged.faces, &nudged_ladder)
        .into_iter()
        .map(|stack| ((stack.left, stack.right), stack.right_above_winding))
        .collect::<BTreeMap<_, _>>();
    determined_stacks(&base.cp, &base.faces, base_ladder)
        .into_iter()
        .filter(|stack| {
            nudged_stacks.get(&(stack.left, stack.right)) == Some(&stack.right_above_winding)
        })
        .collect()
}

// 3つの展開図の折り目110本を1本ずつ ±179.999° と ±180° へ送り、
// **紙の重なり方が同じである**ことと、見えている裏面が飛ばないことを検査する。
//
// **以前は両端点の `surface_rank` の並びをそのまま比べていた。** 完全に折った
// 状態のすぐ近くでは、解が近くの別の折り方へ移るだけで並びが入れ替わるため、
// この形は計算機や丸めの違いで落ち得る(CLAUDE.md §10.7.7 が禁じる「solveの
// 結果に期待値を結び付けた検査」)。実際、作品ファイルの小数を正確に読む
// `serde_json` の `float_roundtrip` を入れただけで、`folded-sample.ori3` の
// 辺306(面31と面34)が落ちるようになった。この面対は 179.999°の姿勢での
// 隙間が −1.902e-6 しかなく、入力を1 ULP動かすと +3.295e-5 へ**符号ごと**
// 変わる(`diag_gap_noise_from_one_ulp_of_input` の `ULPFLIP`)。
//
// いまは次の形にしてある。主張は弱めていない。
//
// 1. 180°の手前の4段(179.5 / 179.9 / 179.99 / 179.999)は面どうしが実際に
//    離れているので、**その姿勢そのものから**面対の上下を測る。紙はすり抜け
//    られないので、信号のある段が3段以上あり、すべて同じ側でなければならない。
// 2. さらに**入力の座標を1 ULP動かした複製**でも同じ梯子を作り、測った上下が
//    変わらない面対だけを主張の対象にする。
// 3. 対象の面対について、179.999°と180°の刻印が同じ紙の重なり方を表している
//    ことを主張する。上下は正準法線ではなく姿勢によらない**巻き方向**で読む
//    ので、軸の反転では入れ替わらない(`winding_sign`)。
//
// 実測: 179.999°の姿勢で法線が平行に重なる面対は 4989組、梯子で上下が決まった
// のは 4936組(98.9%)、そのうち1 ULPでも答えが変わらなかったのは **4888組**で
// ある。以前の形が主張していた「両端点の並びが完全に一致」は、重なっていない
include!("surface_order_sa_endpoint_heavy.rs");
//    - 180°の姿勢と、warm start無しで解き直した姿勢(同じ形へ収束したときだけ)

#[test]
fn surface_order_exact_endpoint_is_rank_stable_for_previous_19() {
    const PREVIOUS_RANK_CHANGES: [EdgeId; 19] = [
        125, 143, 181, 183, 297, 298, 309, 314, 352, 358, 362, 367, 380, 393, 394, 401, 402, 426,
        430,
    ];
    let diagrams = boundary_diagrams();
    let diagram = diagrams
        .iter()
        .find(|diagram| diagram.name == "folded-sample.ori3")
        .expect("the folded acceptance fixture exists");
    let nudged = one_ulp_nudged(diagram);
    let mut robust_stacks = 0_usize;
    let mut cold_compared = 0_usize;
    let mut flipped_directions = BTreeSet::<(EdgeId, i32)>::new();
    for hinge in PREVIOUS_RANK_CHANGES {
        assert!(diagram.hinges.iter().any(|&(edge, _)| edge == hinge));
        for sign in [1.0, -1.0] {
            let ladder = boundary_ladder(diagram, hinge, sign);
            let stacks = rounding_robust_stacks(diagram, &ladder, &nudged, hinge, sign);
            robust_stacks += stacks.len();
            let after = &ladder[ladder.len() - 1].1;
            let before = &ladder[ladder.len() - 2].1;
            let mut flipped =
                stacks_that_flip_between_endpoints(&before.frame, &after.frame, &stacks);

            let refreshed = solve_motion_with_contact_options(
                &diagram.cp,
                &diagram.faces,
                &[Driver {
                    hinge,
                    target_angle_deg: sign * 180.0,
                }],
                None,
                Some(&after.angles),
                MotionContactOptions {
                    detect: true,
                    prevent: true,
                },
            );
            flipped.extend(stacks_that_differ(
                &after.frame,
                &refreshed.result.frame,
                &stacks,
                "180°の姿勢とそこから解き直した姿勢",
            ));

            // warm start無しで解き直すと、同じ折り目を±180°にしても**別の形**へ収束する。
            // 実測(この19本×2方向=38件、`diag_cold_solve_reaches_the_same_pose`):
            // 他の折り目の角度は最大 **359.999900 度** ちがい、38件すべてで
            // 1e-6度を超えてちがった。刻印する重なり順は「いま表示している形」を
            // 説明するものなので、形がちがえば順がちがうのは正しい。
            // ここでは「同じ形へ収束したときは同じ重なり方になる」ことだけを検査する。
            let cold = solve_motion_with_contact_options(
                &diagram.cp,
                &diagram.faces,
                &[Driver {
                    hinge,
                    target_angle_deg: sign * 180.0,
                }],
                None,
                None,
                MotionContactOptions {
                    detect: true,
                    prevent: true,
                },
            );
            let cold_pose_difference = after
                .angles
                .iter()
                .map(|(edge, angle)| {
                    (angle - cold.result.angles.get(edge).copied().unwrap_or(f64::NAN)).abs()
                })
                .fold(0.0_f64, f64::max);
            if cold_pose_difference <= 1e-6 {
                cold_compared += 1;
                flipped.extend(stacks_that_differ(
                    &after.frame,
                    &cold.result.frame,
                    &stacks,
                    "180°の姿勢とwarm start無しで解き直した同じ形",
                ));
            }

            if !flipped.is_empty() {
                flipped_directions.insert((hinge, sign as i32));
                println!(
                    "SURFACE_19_RANK_CHANGE edge={hinge} direction={sign:+} robust_stacks={} flipped={flipped:?}",
                    stacks.len(),
                );
            }
        }
    }
    println!(
        "SURFACE_19_RANK_TOTAL hinges={} directions={} robust_stacks={robust_stacks} cold_compared={cold_compared} changed_directions={} changed_edges={flipped_directions:?}",
        PREVIOUS_RANK_CHANGES.len(),
        PREVIOUS_RANK_CHANGES.len() * 2,
        flipped_directions.len(),
    );
    assert!(
        flipped_directions.is_empty(),
        "the exact endpoint must stack the paper the same way for every pair whose stacking the geometry determines and one ULP of input does not change: {flipped_directions:?}"
    );
    // 主張の対象が空になっていないことの下限。梯子や1 ULPの選別が壊れて対象が
    // 消えると、この検査は何も主張しないまま緑になってしまう。
    //
    // 実測: 19本×2方向で `float_roundtrip` あり **1298組**、なし **1315組**
    // (どちらもこの計算機。1方向あたり約34組)。計算機が変わると外れる組は
    // 増え得るので、下限は「空回りかどうか」だけが分かる 1,000組に置く。
    // 実際に空回りすれば0に近い値まで落ちるので、これで検知できる。
    assert!(
        robust_stacks >= 1_000,
        "the measured stacking must still cover the paper: robust_stacks={robust_stacks}"
    );
}

include!("surface_order_sa_user_frame.rs");
include!("surface_order_sa_visual.rs");
include!("surface_order_sa_ignored_diagnostics_a.rs");

fn covered_back_counts(diagram: &Diagram, frame: &Frame3D, view: Camera) -> (u64, u64, u64) {
    let image = visual_image(diagram, frame, VIEWPORT, view);
    let (front, back) = classified_fill_counts(&image);
    if back == 0 {
        return (front, 0, 0);
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
    let mut covered = 0_u64;
    for (pixel_index, pixel) in image.pixels.iter().enumerate() {
        if pixel.is_some_and(|pixel| pixel.back_facing) && front_coverage[pixel_index] {
            covered += 1;
        }
    }
    (front, back, covered)
}

/// 深度のtie窓と組符号を一切使わず、純粋な最前深度だけで所有者を決めた場合の表・裏。
/// 現行規則との差が、tie解決(surface_rank)に由来する裏画素の量になる。
fn strict_nearest_counts(diagram: &Diagram, frame: &Frame3D, view: Camera) -> (u64, u64) {
    let mut faces = render_faces(diagram, frame, VIEWPORT, view);
    faces.sort_by_key(render_face_owner_key);
    let max_depth_code = (1_u64 << DEPTH_BITS) - 1;
    let mut nearest = vec![f32::INFINITY; VIEWPORT * VIEWPORT];
    let mut owner_back = vec![None::<bool>; VIEWPORT * VIEWPORT];
    for face in &faces {
        for triangle in &face.triangles {
            rasterize(
                |pixel, depth| {
                    let code = (depth.clamp(0.0, 1.0) * max_depth_code as f32).round();
                    if code < nearest[pixel] {
                        nearest[pixel] = code;
                        owner_back[pixel] = Some(triangle.back_facing);
                    }
                },
                triangle,
                VIEWPORT,
            );
        }
    }
    let mut front = 0_u64;
    let mut back = 0_u64;
    for owner in owner_back.into_iter().flatten() {
        if owner {
            back += 1;
        } else {
            front += 1;
        }
    }
    (front, back)
}




include!("surface_order_sa_determinism.rs");
/// 決定的な擬似乱数(SplitMix64)。種を固定すれば毎回同じ姿勢の並びを作る。
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// [0,1) の一様乱数。
    fn next_unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64
    }

    fn next_below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }
}

/// 折り目の目標角を1本ぶん引く。ほぼ平ら(±180°付近)を意図的に厚めに混ぜる。
fn sweep_angle(random: &mut SplitMix64) -> f64 {
    let sign = if random.next_unit() < 0.5 { -1.0 } else { 1.0 };
    let magnitude = match random.next_below(10) {
        0..=5 => random.next_unit() * 180.0,
        6 | 7 => 170.0 + random.next_unit() * 9.9,
        _ => {
            const NEAR_FLAT: [f64; 6] = [179.9, 179.99, 179.999, 179.999_9, 179.999_999, 180.0];
            NEAR_FLAT[random.next_below(NEAR_FLAT.len())]
        }
    };
    sign * magnitude
}

struct SweepPose {
    label: String,
    frame: Frame3D,
    /// 解けた全ヒンジ角。独立検証の探り姿勢を作るのに使う。
    angles: HashMap<EdgeId, f64>,
    /// 重なり順をどの幾何から決めたか。
    source_label: &'static str,
    /// `stamp_motion_surface_order` の canonical 経路条件を外から作り直したもの。
    needs_canonical_path: bool,
    /// `stamp_motion_surface_order` の has_exact 条件を外から作り直したもの。
    has_exact: bool,
    /// 紙が裂けていない形かどうかを分けるための実測値。
    max_seam_gap: f64,
    self_intersects: bool,
}

/// 種を固定して `count` 通りの姿勢を作る。全て `solve_motion` の製品経路を通す。
fn sweep_poses(cp: &CreasePattern, faces: &[Face], seed: u64, count: usize) -> Vec<SweepPose> {
    let mut hinges = fold_hinges(cp, faces)
        .into_iter()
        .map(|(hinge, _)| hinge)
        .collect::<Vec<_>>();
    hinges.sort_unstable();
    let mut random = SplitMix64(seed);
    let mut poses = Vec::with_capacity(count);
    for index in 0..count {
        let mode = index % 3;
        let targets = hinges
            .iter()
            .map(|&hinge| (hinge, sweep_angle(&mut random)))
            .collect::<HashMap<EdgeId, f64>>();
        let (hard, preferred) = match mode {
            // すべての折り目を hard で指定する。
            0 => {
                let mut hard = targets
                    .iter()
                    .map(|(&hinge, &target_angle_deg)| Driver {
                        hinge,
                        target_angle_deg,
                    })
                    .collect::<Vec<_>>();
                hard.sort_unstable_by_key(|driver| driver.hinge);
                (hard, None)
            }
            // 1〜4本だけ hard、残りを preferred にする。
            1 => {
                let driven = 1 + random.next_below(4);
                let mut chosen = BTreeSet::new();
                while chosen.len() < driven {
                    chosen.insert(hinges[random.next_below(hinges.len())]);
                }
                let hard = chosen
                    .iter()
                    .map(|&hinge| Driver {
                        hinge,
                        target_angle_deg: targets[&hinge],
                    })
                    .collect::<Vec<_>>();
                let rest = targets
                    .iter()
                    .filter(|(hinge, _)| !chosen.contains(hinge))
                    .map(|(&hinge, &angle)| (hinge, angle))
                    .collect::<HashMap<_, _>>();
                (hard, Some(rest))
            }
            // 1〜3本だけ hard にして残りは自由に追従させる(preferred 無し)。
            _ => {
                let driven = 1 + random.next_below(3);
                let mut chosen = BTreeSet::new();
                while chosen.len() < driven {
                    chosen.insert(hinges[random.next_below(hinges.len())]);
                }
                let hard = chosen
                    .iter()
                    .map(|&hinge| Driver {
                        hinge,
                        target_angle_deg: targets[&hinge],
                    })
                    .collect::<Vec<_>>();
                (hard, None)
            }
        };
        // 0度から段階的に近づけ、直前の解をwarm startにする。いきなり最終角へ
        // 解くと紙が裂けた形ばかりになり、実際の操作で現れる姿勢から遠ざかる。
        const RAMP_STAGES: usize = 8;
        let mut warm = hinges
            .iter()
            .map(|&hinge| (hinge, 0.0))
            .collect::<HashMap<EdgeId, f64>>();
        let mut motion = None;
        for stage in 1..=RAMP_STAGES {
            let progress = stage as f64 / RAMP_STAGES as f64;
            let stage_hard = hard
                .iter()
                .map(|driver| Driver {
                    hinge: driver.hinge,
                    target_angle_deg: driver.target_angle_deg * progress,
                })
                .collect::<Vec<_>>();
            let stage_preferred = preferred.as_ref().map(|rest| {
                rest.iter()
                    .map(|(&hinge, &angle)| (hinge, angle * progress))
                    .collect::<HashMap<_, _>>()
            });
            motion = Some(solve_motion(
                cp,
                faces,
                &stage_hard,
                stage_preferred.as_ref(),
                Some(&warm),
                true,
            ));
            warm = motion
                .as_ref()
                .expect("the ramp just solved a stage")
                .result
                .angles
                .clone();
        }
        let motion = motion.expect("the ramp runs at least one stage");
        let max_seam_gap = ori3_rigid::max_seam_gap(cp, faces, &motion.result.frame);
        let self_intersects = ori3_rigid::self_intersects(&motion.result.frame);
        let mut canonical = preferred.clone().unwrap_or_default();
        for driver in &hard {
            canonical.insert(driver.hinge, driver.target_angle_deg);
        }
        let needs_canonical_path = canonical
            .values()
            .any(|angle| angle.abs() >= 179.999 - 1e-6);
        let has_exact = motion
            .result
            .angles
            .values()
            .any(|angle| (angle.abs().to_radians() - std::f64::consts::PI).abs() <= 1e-8);
        let mut summary = hard
            .iter()
            .map(|driver| format!("{}:{:.4}", driver.hinge, driver.target_angle_deg))
            .collect::<Vec<_>>();
        summary.sort_unstable();
        poses.push(SweepPose {
            label: format!("pose{index:03}-mode{mode}-hard[{}]", summary.join(" ")),
            source_label: surface_order_source_label(
                motion.surface_order.map(|order| order.source),
            ),
            angles: motion.result.angles,
            frame: motion.result.frame,
            needs_canonical_path,
            has_exact,
            max_seam_gap,
            self_intersects,
        });
    }
    poses
}

#[path = "surface_order_sa_overlap.rs"]
mod surface_order_sa_overlap;
use surface_order_sa_overlap::*;

/// 「ほぼ同じ平面に乗っていて、投影で実面積が重なる」面対と、その平面間の隙間。
/// `coincident_overlaps` は隙間 1e-9 以下しか拾わないため、179.999°のように
/// 実際に離れている姿勢では0組になる。ここでは隙間を測って返す。
struct NearOverlap {
    left: FaceId,
    right: FaceId,
    /// 重なりの代表点で、`left` の面から正準法線方向に `right` の面まで進む符号付き距離。
    /// 正なら `right` が上。楔状に開いていても、この符号は代表点で一意に決まる。
    gap: f64,
    /// 正準法線が `left` の巻き方向の法線と同じ向きなら `+1`、逆なら `-1`。
    ///
    /// 巻き方向の法線は展開図の面の定義だけで決まる**材質側の向き**なので、
    /// 姿勢が変わっても連続に動く。正準法線は絶対値が最大の成分で決めるため、
    /// 対称な姿勢では 1e-9 の違いで反転する。梯子の各段の符号を突き合わせる
    /// ときは、まず `gap * winding_sign`(材質側の向き)へそろえる。
    winding_sign: f64,
}

fn near_overlaps(frame: &Frame3D, max_gap: f64) -> Vec<NearOverlap> {
    let polygons = frame_polygons(frame);
    // 面の並びは `Frame3D` が持つ順(= `extract_faces` の順)のままにする。
    // 製品側 `derive_surface_order_with` も同じ順で面対を作り、**先に来るほうの面**の
    // 正準法線を「上」の向きに使う。並びを変えると、同じ面対でも上下の向きの基準が
    // 製品と食い違う。
    let face_ids = frame.faces.iter().map(|face| face.face).collect::<Vec<_>>();
    let mut found = Vec::new();
    for (left_index, &left) in face_ids.iter().enumerate() {
        for &right in &face_ids[left_index + 1..] {
            let left_points = &polygons[&left];
            let right_points = &polygons[&right];
            let (Some(plane), Some(right_plane)) =
                (overlap_plane(left_points), overlap_plane(right_points))
            else {
                continue;
            };
            // 0.001°の傾きは 1.7e-5 rad。これを「平行」として拾う。
            if plane.normal.dot(right_plane.normal).abs() < 1.0 - 1e-4 {
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
            // 重なりの代表点で、左の面から正準法線方向に右の面まで進む距離。
            // 正なら右が上。楔状に開いていても、この符号は代表点で一意に決まる。
            let witness3 = plane.origin + plane.u * witness[0] + plane.v * witness[1];
            let denominator = right_plane.normal.dot(plane.normal);
            if denominator.abs() < 1e-6 {
                continue;
            }
            let gap = right_plane.normal.dot(right_plane.origin - witness3) / denominator;
            if !gap.is_finite() || gap.abs() > max_gap {
                continue;
            }
            let Some(winding) = polygon_normal3(left_points) else {
                continue;
            };
            let winding_sign = if winding.dot(plane.normal) >= 0.0 {
                1.0
            } else {
                -1.0
            };
            found.push(NearOverlap {
                left,
                right,
                gap,
                winding_sign,
            });
        }
    }
    found
}

/// 梯子の実測だけで上下が決まった面対。`surface_rank` は一度も読んでいない。
struct DeterminedStack {
    left: FaceId,
    right: FaceId,
    /// `left` の巻き方向の法線に対して `right` が上なら `true`。
    /// 姿勢が変わっても意味が変わらない材質側の向きで表す。
    right_above_winding: bool,
    /// 根拠に使えた段の数。
    samples: usize,
    /// 根拠に使えた段のうち、いちばん小さい隙間の大きさ。
    smallest_gap: f64,
    /// いちばん大きい隙間の大きさ。
    largest_gap: f64,
}

/// 梯子の各段で測った隙間の符号から、丸めに左右されない上下だけを取り出す。
///
/// 179.5°/179.9°/179.99°/179.999° の4段は、いずれも面どうしが実際に離れている
/// 姿勢である。紙はすり抜けられないので、**どの段でも同じ側**にいなければ
/// ならない。1段だけの符号は、解が近くの別の折り方へ移ると反転し得る
/// (実測: 入力座標を1 ULP動かしただけで、隙間 3.36e-5 の面対の符号が反転した。
/// `diag_gap_noise_from_one_ulp_of_input` の `ULPFLIP`)。3段以上で符号が
/// 一致していることを条件にすると、この揺れは根拠から外れる。
fn determined_stacks(
    cp: &CreasePattern,
    faces: &[Face],
    ladder: &[(f64, EndpointState)],
) -> Vec<DeterminedStack> {
    let separated = ladder
        .iter()
        .filter(|(absolute, _)| *absolute < 180.0)
        .map(|(_, state)| {
            let seam = ori3_rigid::max_seam_gap(cp, faces, &state.frame);
            let pairs = near_overlaps(&state.frame, f64::INFINITY)
                .into_iter()
                .map(|pair| ((pair.left, pair.right), pair.gap * pair.winding_sign))
                .collect::<BTreeMap<_, _>>();
            (seam, pairs)
        })
        .collect::<Vec<_>>();
    let Some((_, last)) = separated.last() else {
        return Vec::new();
    };
    let mut determined = Vec::new();
    for &(left, right) in last.keys() {
        let mut positives = 0_usize;
        let mut negatives = 0_usize;
        let mut smallest = f64::INFINITY;
        let mut largest = 0.0_f64;
        for (seam, pairs) in &separated {
            let Some(&oriented) = pairs.get(&(left, right)) else {
                continue;
            };
            // 面が離れている量より紙のちぎれのほうが大きい段には信号がない。
            if oriented.abs() <= seam.max(1e-9) {
                continue;
            }
            if oriented > 0.0 {
                positives += 1;
            } else {
                negatives += 1;
            }
            smallest = smallest.min(oriented.abs());
            largest = largest.max(oriented.abs());
        }
        let samples = positives + negatives;
        if samples < 3 || (positives != 0 && negatives != 0) {
            continue;
        }
        determined.push(DeterminedStack {
            left,
            right,
            right_above_winding: positives > 0,
            samples,
            smallest_gap: smallest,
            largest_gap: largest,
        });
    }
    determined
}

/// 刻印された `surface_rank` が、梯子の実測で決まった上下と合っているか。
///
/// `surface_rank` は「その面対が乗る平面の**正準法線**の向きに下から上へ」並べた
/// 順である。正準法線は対称な姿勢で反転し得るので、その姿勢での
/// `winding_sign` で材質側の向きへ直してから比べる。合わない面対を返す。
fn stack_disagreements(
    frame: &Frame3D,
    determined: &[DeterminedStack],
) -> Vec<(FaceId, FaceId, String)> {
    let sides = material_sides(frame, determined);
    determined
        .iter()
        .filter_map(|stack| {
            let side = sides.get(&(stack.left, stack.right))?;
            (*side != stack.right_above_winding).then(|| {
                (
                    stack.left,
                    stack.right,
                    format!(
                        "実測{}段(隙間 {:.3e}〜{:.3e})では面{}が面{}の反対側にあるのに、重なり順が逆である",
                        stack.samples,
                        stack.smallest_gap,
                        stack.largest_gap,
                        stack.right,
                        stack.left,
                    ),
                )
            })
        })
        .collect()
}
include!("surface_order_sa_ignored_diagnostics_b.rs");

/// `motion.rs::canonical_motion_surface_order` と同じ22点のcheckpoint。
/// 製品側は `surface_order.rs::SURFACE_PATH_CHECKPOINT_DEG`(crate内)にある。
const CANONICAL_CHECKPOINT_DEG: [f64; 22] = [
    9.0, 19.0, 29.0, 39.0, 49.0, 59.0, 69.0, 79.0, 90.0, 101.0, 111.0, 121.0, 131.0, 141.0, 151.0,
    161.0, 171.0, 179.0, 179.5, 179.9, 179.99, 179.999,
];
/// `surface_order.rs::STACK_FLAT_THRESHOLD_DEG` と同じ値。
const CANONICAL_STACK_FLAT_DEG: f64 = 179.99;

/// 製品の `canonical_motion_surface_order` が使う経路と終点を、公開APIだけで再現する。
fn canonical_path_frames(
    diagram: &Diagram,
    final_angles: &HashMap<EdgeId, f64>,
) -> (Vec<Frame3D>, Frame3D) {
    let mut sorted = final_angles
        .iter()
        .map(|(&hinge, &angle)| (hinge, angle.clamp(-180.0, 180.0)))
        .collect::<Vec<_>>();
    sorted.sort_by_key(|&(hinge, _)| hinge);
    let mut warm = None::<HashMap<EdgeId, f64>>;
    let mut frames = Vec::new();
    for checkpoint in CANONICAL_CHECKPOINT_DEG {
        let drivers = sorted
            .iter()
            .map(|&(hinge, angle)| Driver {
                hinge,
                target_angle_deg: angle.signum() * angle.abs().min(checkpoint),
            })
            .collect::<Vec<_>>();
        let solved = ori3_rigid::solve(&diagram.cp, &diagram.faces, &drivers, warm.as_ref());
        frames.push(solved.frame.clone());
        warm = Some(solved.angles);
    }
    let exact_drivers = sorted
        .iter()
        .map(|&(hinge, angle)| Driver {
            hinge,
            target_angle_deg: if angle.abs() >= CANONICAL_STACK_FLAT_DEG {
                angle.signum() * 180.0
            } else {
                angle
            },
        })
        .collect::<Vec<_>>();
    let exact = ori3_rigid::solve(&diagram.cp, &diagram.faces, &exact_drivers, warm.as_ref());
    (frames, exact.frame)
}

/// `diagonal-midline-square`(米印)の折り目12を ±180° まで送ったときの全8角。
/// 実測(`diag_remaining_two_creases_gap_sign_ladder` の `LADDER_ANGLES`)を
/// そのまま入力として埋め込む。閉じた頂点なのでこの8角だけで形が決まる。
fn kome_edge12_angles(sign: f64) -> HashMap<EdgeId, f64> {
    HashMap::from([
        (8, sign * 64.483_939_566_398_75),
        (9, sign * 45.635_164_732_595_49),
        (10, sign * 3.788_334_474_960_96),
        (11, sign * -39.409_800_747_860_28),
        (12, sign * 180.0),
        (13, sign * -39.409_800_658_124_02),
        (14, sign * 3.788_334_481_828_027_6),
        (15, sign * 45.635_164_727_441_044),
    ])
}

include!("surface_order_sa_gap_derivation.rs");
#[path = "surface_order_sa_warm_path_canonical.rs"]
mod surface_order_sa_warm_path_canonical;
