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

#[test]
fn surface_order_exact_user_zero_step_pose_reproduction() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    assert_eq!(cp.vertices.len(), 13);
    assert_eq!(cp.edges.len(), 28);
    assert_eq!(faces.len(), 16);
    assert_eq!(zero_back_user_angles().len(), 20);
    let diagram = diagram("exact-user-zero-step-pose", cp.clone(), 1.0, 1.0);
    let frame = zero_back_user_frame(&cp, &faces);
    let default_camera = camera(1.0, 1.0, 1.0);
    let image = visual_image(&diagram, &frame, VIEWPORT, default_camera);

    let mut calculated_faces = render_faces(&diagram, &frame, VIEWPORT, default_camera);
    calculated_faces.sort_by_key(render_face_owner_key);
    let calculation_faces = calculated_faces
        .iter()
        .enumerate()
        .map(|(draw_order, face)| {
            serde_json::json!({
                "face": face.face,
                "draw_order": draw_order,
                "surface_rank": face.surface_rank,
                "side": face.side,
                "material_orientation": face.material_orientation,
                "back_facing": face.triangles.iter().all(|triangle| triangle.back_facing),
                "triangle_back_facing": face
                    .triangles
                    .iter()
                    .map(|triangle| triangle.back_facing)
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let fixture = serde_json::json!({
        "document": {
            "schema_version": 1,
            "paper": { "width_mm": 150.0, "height_mm": 150.0 },
            "cp": cp,
            "sequence": [],
            "display": {
                "front_color": [237, 28, 36],
                "back_color": [255, 255, 255],
                "grid_divisions": 8,
                "soft_enabled": false,
                "soft_stiffness": 0.5,
                "soft_pressure": 0.0,
                "overlap_prevention_enabled": true,
                "penetration_prevention_enabled": true,
            },
        },
        "faces": faces
            .iter()
            .map(|face| serde_json::json!({
                "id": face.id,
                "vertices": face.vertices,
                "edges": face.edges,
            }))
            .collect::<Vec<_>>(),
        "frame": frame,
        "calculation_faces": calculation_faces,
    });
    println!(
        "ZERO_BACK_PIPELINE_FIXTURE {}",
        serde_json::to_string(&fixture).expect("pipeline fixture serializes")
    );

    // 利用者指定の相互排他的RGB判定を、既定色を持つCPU rasterへそのまま適用する。
    // 背景と黒線はfill-only rasterのNoneであり、紙画素の分母へ含めない。
    let mut front = 0_u64;
    let mut back = 0_u64;
    for pixel in image.pixels.iter().flatten() {
        let [r, g, b] = if pixel.back_facing {
            [255_i16, 255_i16, 255_i16]
        } else {
            [237_i16, 28_i16, 36_i16]
        };
        let is_front = r > 140 && r - g > 40 && r - b > 40;
        let is_background = (r - 205).abs() <= 12 && (g - 200).abs() <= 12 && (b - 193).abs() <= 12;
        let is_black = r < 90 && g < 90 && b < 90;
        let is_back = !is_front
            && !is_background
            && !is_black
            && r > 150
            && (r - g).abs() < 30
            && (g - b).abs() < 30;
        front += u64::from(is_front);
        back += u64::from(is_back);
    }
    assert_eq!(front, image.red_pixels);
    assert_eq!(back, image.light_pixels);
    let total = front + back;
    assert!(total > 0);
    let back_ratio = back as f64 / total as f64;
    println!(
        "ZERO_BACK_REPRO steps=0 vertices={} edges={} creases={} front={} back={} back_ratio={:.6}% visible={} back_faces={}",
        cp.vertices.len(),
        cp.edges.len(),
        zero_back_user_angles().len(),
        front,
        back,
        back_ratio * 100.0,
        ids(&image.visible_faces),
        ids(&image.visible_back_faces),
    );

    // bad-full.pngの3D canvasは925×536 CSS px。800角の測定bufferでも水平投影へ
    // 同じaspectを入れれば、色比を保ったまま画面と同じclipping範囲を測れる。
    let screen_aspect = 925.0 / 536.0;
    let screen_image = visual_image(
        &diagram,
        &frame,
        VIEWPORT,
        camera_with_aspect(1.0, 1.0, 1.0, screen_aspect),
    );
    let screen_total = screen_image.red_pixels + screen_image.light_pixels;
    let screen_back_ratio = screen_image.light_pixels as f64 / screen_total as f64;
    println!(
        "ZERO_BACK_SCREEN_ASPECT aspect={screen_aspect:.9} front={} back={} back_ratio={:.6}% visible={} back_faces={}",
        screen_image.red_pixels,
        screen_image.light_pixels,
        screen_back_ratio * 100.0,
        ids(&screen_image.visible_faces),
        ids(&screen_image.visible_back_faces),
    );
}

#[test]
fn surface_order_exact_user_warm_start_paths() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("exact-user-warm-start-paths", cp.clone(), 1.0, 1.0);
    let final_angles = zero_back_user_angles();
    let direct_frame = zero_back_user_frame(&cp, &faces);
    let default_camera = camera(1.0, 1.0, 1.0);
    let screen_camera = camera_with_aspect(1.0, 1.0, 1.0, 925.0 / 536.0);

    let face_pixels = |image: &VisualImage| {
        let mut counts = BTreeMap::<FaceId, [u64; 2]>::new();
        for pixel in image.pixels.iter().flatten() {
            let count = counts.entry(pixel.face).or_default();
            count[usize::from(pixel.back_facing)] += 1;
        }
        counts
    };
    let direct_image = visual_image(&diagram, &direct_frame, VIEWPORT, default_camera);
    let direct_total = direct_image.red_pixels + direct_image.light_pixels;
    println!(
        "WARMSTART_BASELINE {}",
        serde_json::to_string(&serde_json::json!({
            "method": "direct-final-angle-propagation",
            "stages": 1,
            "front": direct_image.red_pixels,
            "back": direct_image.light_pixels,
            "back_ratio_percent": direct_image.light_pixels as f64 / direct_total as f64 * 100.0,
            "face_pixels_front_back": face_pixels(&direct_image),
        }))
        .expect("warm-start baseline serializes")
    );

    let mut measured = 0_usize;
    for path in [
        ZeroBackWarmPath::Active36WithPreferred,
        ZeroBackWarmPath::Edge36Only,
        ZeroBackWarmPath::AllHardControl,
    ] {
        for stages in [1_usize, 10, 50, 200] {
            let result = zero_back_warm_solve(&cp, &faces, path, stages);
            let square = visual_image(&diagram, &result.frame, VIEWPORT, default_camera);
            let screen = visual_image(&diagram, &result.frame, VIEWPORT, screen_camera);
            let square_total = square.red_pixels + square.light_pixels;
            let screen_total = screen.red_pixels + screen.light_pixels;
            assert!(square_total > 0 && screen_total > 0);

            let mut maximum_angle_delta = 0.0_f64;
            let mut angle_comparison = final_angles
                .iter()
                .map(|(&hinge, &target)| {
                    let actual = result.angles.get(&hinge).copied().unwrap_or(0.0);
                    let delta = (actual - target + 180.0).rem_euclid(360.0) - 180.0;
                    maximum_angle_delta = maximum_angle_delta.max(delta.abs());
                    serde_json::json!({
                        "hinge": hinge,
                        "target": target,
                        "actual": actual,
                        "canonical_delta": delta,
                    })
                })
                .collect::<Vec<_>>();
            angle_comparison.sort_by_key(|entry| entry["hinge"].as_u64().unwrap_or_default());

            let square_face_pixels = face_pixels(&square);
            let mut maximum_vertex_delta = 0.0_f64;
            let mut face_comparison = Vec::with_capacity(direct_frame.faces.len());
            for reference in &direct_frame.faces {
                let candidate = result
                    .frame
                    .faces
                    .iter()
                    .find(|face| face.face == reference.face)
                    .expect("warm-start frame contains every reference face");
                assert_eq!(reference.polygon.len(), candidate.polygon.len());
                let face_vertex_delta = reference
                    .polygon
                    .iter()
                    .zip(&candidate.polygon)
                    .map(|(left, right)| {
                        left.iter()
                            .zip(right)
                            .map(|(a, b)| (a - b).powi(2))
                            .sum::<f64>()
                            .sqrt()
                    })
                    .fold(0.0_f64, f64::max);
                maximum_vertex_delta = maximum_vertex_delta.max(face_vertex_delta);
                face_comparison.push(serde_json::json!({
                    "face": reference.face,
                    "direct_surface_rank": reference.surface_rank,
                    "warm_surface_rank": candidate.surface_rank,
                    "direct_mirrored": reference.mirrored,
                    "warm_mirrored": candidate.mirrored,
                    "max_vertex_delta": face_vertex_delta,
                    "front_pixels": square_face_pixels
                        .get(&reference.face)
                        .map_or(0, |counts| counts[0]),
                    "back_pixels": square_face_pixels
                        .get(&reference.face)
                        .map_or(0, |counts| counts[1]),
                }));
            }

            println!(
                "WARMSTART_SUMMARY method={} stages={} square_front={} square_back={} square_back_ratio={:.9}% screen_front={} screen_back={} screen_back_ratio={:.9}% converged={} closure_rms={:.3e} seam={:.3e} intersects={} contact={} max_angle_delta={:.9} max_vertex_delta={:.9}",
                path.label(),
                stages,
                square.red_pixels,
                square.light_pixels,
                square.light_pixels as f64 / square_total as f64 * 100.0,
                screen.red_pixels,
                screen.light_pixels,
                screen.light_pixels as f64 / screen_total as f64 * 100.0,
                result.converged,
                result.closure_rms,
                result.max_seam_gap,
                result.self_intersects,
                result.contact_detected,
                maximum_angle_delta,
                maximum_vertex_delta,
            );
            println!(
                "WARMSTART_CASE {}",
                serde_json::to_string(&serde_json::json!({
                    "method": path.label(),
                    "stages": stages,
                    "square": {
                        "front": square.red_pixels,
                        "back": square.light_pixels,
                        "back_ratio_percent": square.light_pixels as f64 / square_total as f64 * 100.0,
                        "visible_faces": square.visible_faces,
                        "visible_back_faces": square.visible_back_faces,
                    },
                    "screen_aspect": {
                        "front": screen.red_pixels,
                        "back": screen.light_pixels,
                        "back_ratio_percent": screen.light_pixels as f64 / screen_total as f64 * 100.0,
                        "visible_faces": screen.visible_faces,
                        "visible_back_faces": screen.visible_back_faces,
                    },
                    "solve": {
                        "converged": result.converged,
                        "closure_rms": result.closure_rms,
                        "iterations": result.iterations,
                        "contact_detected": result.contact_detected,
                        "self_intersects": result.self_intersects,
                        "max_seam_gap": result.max_seam_gap,
                        "maximum_final_angle_delta_deg": maximum_angle_delta,
                        "maximum_direct_frame_vertex_delta": maximum_vertex_delta,
                    },
                    "angles": angle_comparison,
                    "faces": face_comparison,
                }))
                .expect("warm-start measurement serializes")
            );
            measured += 1;
        }
    }
    assert_eq!(measured, 12, "three paths times four stage counts");
}

/// 利用者が実際に表示する20角度の形を、固定の36方位角 x 17仰角で測る常設検査。
/// 旧検査はedge36だけをwarm solveしたほぼ展開状態を測り、裏53,762,620画素のうち
/// 46,271,965画素に表の覆いが無かったため、612方向で裏0という条件の対象として誤っていた。
/// この検査は `zero_back_user_frame` へ対象を直し、表triangleが同じpixelを覆う裏だけを
/// 所有者判定の失敗とする。表の覆いが無い2画素は幾何的露出として座標ごと固定する。
#[test]
fn surface_order_user_frame_has_only_expected_geometric_exposure_from_all_612_directions() {
    type Exposure = (i32, i32, usize, usize, FaceId);
    // (azimuth, elevation, x, y, owner face)。CPU rasterの座標原点は左上。
    const EXPECTED_GEOMETRIC_EXPOSURES: [Exposure; 2] =
        [(40, 60, 7, 370, 13), (250, -20, 673, 206, 2)];

    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("user-frame-zero-612", cp.clone(), 1.0, 1.0);
    let frame = zero_back_user_frame(&cp, &faces);
    assert_eq!(cp.vertices.len(), 13);
    assert_eq!(cp.edges.len(), 28);
    assert_eq!(faces.len(), 16);
    assert_eq!(frame.faces.len(), 16);

    let mut measured_directions = 0_usize;
    let mut total_front_pixels = 0_u64;
    let mut raw_directions_with_back = 0_usize;
    let mut raw_back_pixels = 0_u64;
    let mut directions_with_geometric_exposure = 0_usize;
    // 合否に使う `directions_with_back` は、表の覆いがあるのに裏ownerとなった方向数。
    let mut directions_with_back = 0_usize;
    let mut geometric_exposures = BTreeSet::<Exposure>::new();
    let mut unexpected_back_pixels = BTreeSet::<Exposure>::new();

    for elevation_deg in (-80_i32..=80).step_by(10) {
        for azimuth_deg in (0_i32..=350).step_by(10) {
            let view = camera_from_orbit_angles(1.0, 1.0, azimuth_deg, elevation_deg);
            let image = visual_image(&diagram, &frame, VIEWPORT, view);
            let (front, back) = classified_fill_counts(&image);
            assert_eq!(front, image.red_pixels);
            assert_eq!(back, image.light_pixels);
            measured_directions += 1;
            total_front_pixels += front;
            if back == 0 {
                continue;
            }

            raw_directions_with_back += 1;
            raw_back_pixels += back;
            let mut front_coverage = vec![false; VIEWPORT * VIEWPORT];
            for face in render_faces(&diagram, &frame, VIEWPORT, view) {
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

            let mut has_geometric_exposure = false;
            let mut has_unexpected_back = false;
            for (pixel_index, pixel) in image.pixels.iter().enumerate() {
                let Some(owner) = pixel.filter(|pixel| pixel.back_facing) else {
                    continue;
                };
                let exposure = (
                    azimuth_deg,
                    elevation_deg,
                    pixel_index % VIEWPORT,
                    pixel_index / VIEWPORT,
                    owner.face,
                );
                if front_coverage[pixel_index] {
                    unexpected_back_pixels.insert(exposure);
                    has_unexpected_back = true;
                } else {
                    geometric_exposures.insert(exposure);
                    has_geometric_exposure = true;
                }
            }
            directions_with_geometric_exposure += usize::from(has_geometric_exposure);
            directions_with_back += usize::from(has_unexpected_back);
        }
    }

    assert_eq!(measured_directions, 36 * 17);
    println!(
        "ZERO612_SUMMARY directions={} total_front_pixels={} raw_directions_with_back={} raw_back_pixels={} directions_with_geometric_exposure={} geometric_exposure_pixels={} directions_with_back={} unexpected_back_pixels={} geometric_exposures={geometric_exposures:?} unexpected={unexpected_back_pixels:?}",
        measured_directions,
        total_front_pixels,
        raw_directions_with_back,
        raw_back_pixels,
        directions_with_geometric_exposure,
        geometric_exposures.len(),
        directions_with_back,
        unexpected_back_pixels.len(),
    );
    assert_eq!(
        directions_with_back, 0,
        "all covered back pixels must be eliminated: {unexpected_back_pixels:?}"
    );
    assert!(unexpected_back_pixels.is_empty());
    assert_eq!(raw_back_pixels as usize, geometric_exposures.len());
    assert_eq!(geometric_exposures.len(), 2);
    assert_eq!(
        geometric_exposures,
        BTreeSet::from(EXPECTED_GEOMETRIC_EXPOSURES),
        "geometric exposure pixels changed"
    );
}

/// 利用者の画面で紙の裏が 66.88% 見えていた `live-frame` の姿勢を、同じ
/// 36方位角 x 17仰角 = 612方向で測る常設検査。
///
/// この姿勢では16面すべてがほぼ同じ平面に重なるため、どの面が見えるかは
/// `surface_rank` だけが決める。修正前は180°の折り目15本のうち7本が折り目の
/// 向きに反しており、612方向**すべて**で裏が出ていた(実測 裏40.38%)。
///
/// 修正後に残る裏の画素は、612方向 x 800x800 の合計 22,442,018 表画素に対して
/// **2画素だけ**で、いずれも面の輪郭上に孤立した1画素である(上下左右の隣接画素は
/// すべて表)。面11・面10の投影多角形の縁がその1画素にだけ掛かった、CPU rasterの
/// 境界画素であり、まとまった面が裏を向いている状態ではない。
/// 隣接画素まで含めて固定するので、面が1枚でも裏返れば必ず落ちる。
#[test]
fn surface_order_live_frame_has_no_back_pixels_from_all_612_directions() {
    type Exposure = (i32, i32, usize, usize, FaceId);
    // (方位角, 仰角, x, y, 面番号)。CPU rasterの座標原点は左上。
    const EXPECTED_BOUNDARY_PIXELS: [Exposure; 2] =
        [(20, 20, 730, 255, 11), (140, -70, 163, 249, 10)];

    let cp = live_frame_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("live-frame-612", cp.clone(), 1.0, 1.0);
    let frame = live_frame_frame(&cp, &faces);
    assert_eq!(cp.vertices.len(), 13);
    assert_eq!(cp.edges.len(), 28);
    assert_eq!(faces.len(), 16);
    assert_eq!(frame.faces.len(), 16);

    let mut measured_directions = 0_usize;
    let mut total_front_pixels = 0_u64;
    let mut raw_back_pixels = 0_u64;
    let mut raw_directions_with_back = 0_usize;
    let mut directions_with_back = 0_usize;
    let mut geometric_exposures = BTreeSet::<Exposure>::new();
    let mut unexpected_back_pixels = BTreeSet::<Exposure>::new();

    for elevation_deg in (-80_i32..=80).step_by(10) {
        for azimuth_deg in (0_i32..=350).step_by(10) {
            let view = camera_from_orbit_angles(1.0, 1.0, azimuth_deg, elevation_deg);
            let image = visual_image(&diagram, &frame, VIEWPORT, view);
            let (front, back) = classified_fill_counts(&image);
            assert_eq!(front, image.red_pixels);
            assert_eq!(back, image.light_pixels);
            measured_directions += 1;
            total_front_pixels += front;
            if back == 0 {
                continue;
            }
            raw_directions_with_back += 1;
            raw_back_pixels += back;
            directions_with_back += 1;
            for (pixel_index, pixel) in image.pixels.iter().enumerate() {
                let Some(owner) = pixel.filter(|pixel| pixel.back_facing) else {
                    continue;
                };
                let x = pixel_index % VIEWPORT;
                let y = pixel_index / VIEWPORT;
                let exposure = (azimuth_deg, elevation_deg, x, y, owner.face);
                // 上下左右の隣接画素に裏が1つでもあれば、輪郭の1画素ではなく
                // 面が裏返って見えている領域である。
                let isolated =
                    [(-1_i32, 0_i32), (1, 0), (0, -1), (0, 1)]
                        .into_iter()
                        .all(|(dx, dy)| {
                            let (Some(nx), Some(ny)) = (
                                x.checked_add_signed(dx as isize),
                                y.checked_add_signed(dy as isize),
                            ) else {
                                return true;
                            };
                            if nx >= VIEWPORT || ny >= VIEWPORT {
                                return true;
                            }
                            image.pixels[ny * VIEWPORT + nx]
                                .is_none_or(|neighbour| !neighbour.back_facing)
                        });
                if isolated {
                    geometric_exposures.insert(exposure);
                } else {
                    unexpected_back_pixels.insert(exposure);
                }
            }
        }
    }

    assert_eq!(measured_directions, 36 * 17);
    println!(
        "LIVE612_SUMMARY directions={measured_directions} total_front_pixels={total_front_pixels} raw_directions_with_back={raw_directions_with_back} raw_back_pixels={raw_back_pixels} isolated_boundary_pixels={} directions_with_back={directions_with_back} clustered_back_pixels={} boundary={geometric_exposures:?} clustered={unexpected_back_pixels:?}",
        geometric_exposures.len(),
        unexpected_back_pixels.len(),
    );
    assert!(total_front_pixels > 0, "紙が1画素も描かれていない");
    assert!(
        unexpected_back_pixels.is_empty(),
        "隣接画素まで裏になっている。面がまとまって裏返っている: {unexpected_back_pixels:?}"
    );
    assert_eq!(raw_back_pixels as usize, geometric_exposures.len());
    assert_eq!(
        geometric_exposures,
        BTreeSet::from(EXPECTED_BOUNDARY_PIXELS),
        "輪郭の裏画素が変わった"
    );
    // 修正前は612方向すべてで裏が出ていた。裏の割合の上限を実測値で固定する。
    assert!(
        raw_back_pixels * 1_000_000 < total_front_pixels,
        "裏 {raw_back_pixels} / 表 {total_front_pixels}"
    );
}

/// 上の612方向検査で裏になった2画素を、その画素を覆う全triangleと一緒に並べる。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn live_frame_boundary_back_pixel_detail() {
    let cp = live_frame_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("live-frame-612", cp.clone(), 1.0, 1.0);
    let frame = live_frame_frame(&cp, &faces);
    for (azimuth_deg, elevation_deg, target_x, target_y) in
        [(20_i32, 20_i32, 730_usize, 255_usize), (140, -70, 163, 249)]
    {
        let view = camera_from_orbit_angles(1.0, 1.0, azimuth_deg, elevation_deg);
        let mut rendered = render_faces(&diagram, &frame, VIEWPORT, view);
        rendered.sort_by_key(render_face_owner_key);
        let target = target_y * VIEWPORT + target_x;
        println!("--- az={azimuth_deg} el={elevation_deg} pixel=({target_x},{target_y}) ---");
        let image = visual_image(&diagram, &frame, VIEWPORT, view);
        for dy in -2_i32..=2 {
            let row = (-2_i32..=2)
                .map(|dx| {
                    let x = target_x as i32 + dx;
                    let y = target_y as i32 + dy;
                    match image.pixels[y as usize * VIEWPORT + x as usize] {
                        None => "----".to_string(),
                        Some(pixel) => format!(
                            "{}{}",
                            if pixel.back_facing { "B" } else { "F" },
                            pixel.face
                        ),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            println!("  neighbours dy={dy:+}: {row}");
        }
        for face in &rendered {
            for (index, triangle) in face.triangles.iter().enumerate() {
                let mut hit = None;
                rasterize(
                    |pixel, depth| {
                        if pixel == target {
                            hit = Some(depth);
                        }
                    },
                    triangle,
                    VIEWPORT,
                );
                if let Some(depth) = hit {
                    println!(
                        "  face={} rank={} side={} group={} back_facing={} tri={index} depth={depth:.9} key={:?}",
                        face.face,
                        face.surface_rank,
                        face.side,
                        face.coplanar_group,
                        triangle.back_facing,
                        render_face_owner_key(face),
                    );
                }
            }
        }
    }
}

#[test]
fn surface_order_user_pose_az320_el20_reports_owner_candidates() {
    #[derive(Clone, Copy)]
    struct Cover {
        draw_order: usize,
        triangle: usize,
        depth: f32,
        depth_code: u32,
        back_facing: bool,
    }

    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("user-pose-owner-diagnosis", cp.clone(), 1.0, 1.0);
    let frame = zero_back_user_frame(&cp, &faces);
    println!(
        "ZERO_OWNER_FRAME_LAYERS {:?}",
        frame
            .faces
            .iter()
            .map(|face| (face.face, face.layer, face.surface_rank))
            .collect::<Vec<_>>()
    );
    let view = camera_from_orbit_angles(1.0, 1.0, 320, 20);
    let image = visual_image(&diagram, &frame, VIEWPORT, view);
    let mut rendered = render_faces(&diagram, &frame, VIEWPORT, view);
    rendered.sort_by_key(render_face_owner_key);

    let max_depth_code = (1_u64 << DEPTH_BITS) - 1;
    let back_mask = image
        .pixels
        .iter()
        .map(|pixel| pixel.is_some_and(|pixel| pixel.back_facing))
        .collect::<Vec<_>>();
    let mut covers = (0..VIEWPORT * VIEWPORT)
        .map(|_| Vec::<Cover>::new())
        .collect::<Vec<_>>();
    for (draw_order, face) in rendered.iter().enumerate() {
        for (triangle_index, triangle) in face.triangles.iter().enumerate() {
            rasterize(
                |pixel, depth| {
                    if !back_mask[pixel] {
                        return;
                    }
                    covers[pixel].push(Cover {
                        draw_order,
                        triangle: triangle_index,
                        depth,
                        depth_code: (depth.clamp(0.0, 1.0) * max_depth_code as f32).round() as u32,
                        back_facing: triangle.back_facing,
                    });
                },
                triangle,
                VIEWPORT,
            );
        }
    }

    let mut diagnosed_back_pixels = 0_u64;
    let mut no_front_at_pixel = 0_u64;
    let mut front_exists_but_farther = 0_u64;
    let mut lost_to_eligible_front = 0_u64;
    let mut same_side = 0_u64;
    let mut split_side = 0_u64;
    let mut adjacent_front_covering_current = 0_u64;
    let mut adjacent_front_not_covering_current = 0_u64;
    let mut pixels_with_adjacent_front_covering_current = 0_u64;
    let mut pixels_with_only_adjacent_front_not_covering_current = 0_u64;
    let mut winner_front_pairs = BTreeMap::<(FaceId, FaceId, u32, u32, i64, i64), u64>::new();
    let mut nearest_front_pairs = BTreeMap::<(FaceId, FaceId, u32), u64>::new();
    let mut adjacent_cover_pairs = BTreeMap::<(FaceId, FaceId, u32), u64>::new();

    for (pixel_index, pixel) in image.pixels.iter().enumerate() {
        let Some(owner) = pixel.filter(|pixel| pixel.back_facing) else {
            continue;
        };
        diagnosed_back_pixels += 1;
        let pixel_covers = &covers[pixel_index];
        assert!(
            !pixel_covers.is_empty(),
            "owner pixel must have a raster cover"
        );
        let minimum_depth_code = pixel_covers
            .iter()
            .map(|cover| cover.depth_code)
            .min()
            .expect("a back pixel has at least one cover");
        let minimum_depth = minimum_depth_code as f32 / max_depth_code as f32;
        let tolerance = DEPTH_TIE_CODES as f32 / max_depth_code as f32;
        // productionのvisual_imageと同じく、量子化されたnearestに対して
        // fragmentのraw f32 depthを比較する。丸め後code差だけでは境界がずれる。
        let eligible = |cover: &Cover| cover.depth - minimum_depth <= tolerance;
        let expected_owner = pixel_covers
            .iter()
            .filter(|cover| eligible(cover))
            .max_by_key(|cover| cover.draw_order)
            .expect("the nearest cover is always eligible");
        assert_eq!(rendered[expected_owner.draw_order].face, owner.face);
        assert_eq!(expected_owner.back_facing, owner.back_facing);
        let best_front_any = pixel_covers
            .iter()
            .filter(|cover| !cover.back_facing)
            .max_by_key(|cover| cover.draw_order);
        let nearest_front = pixel_covers
            .iter()
            .filter(|cover| !cover.back_facing)
            .min_by(|left, right| {
                left.depth
                    .total_cmp(&right.depth)
                    .then(right.draw_order.cmp(&left.draw_order))
            });
        let best_front_eligible = pixel_covers
            .iter()
            .filter(|cover| !cover.back_facing && eligible(cover))
            .max_by_key(|cover| cover.draw_order);
        if let Some(front) = nearest_front {
            *nearest_front_pairs
                .entry((
                    owner.face,
                    rendered[front.draw_order].face,
                    front.depth_code.saturating_sub(minimum_depth_code),
                ))
                .or_default() += 1;
        }

        match (best_front_any, best_front_eligible) {
            (None, _) => no_front_at_pixel += 1,
            (Some(_), None) => front_exists_but_farther += 1,
            (Some(_), Some(front)) => {
                lost_to_eligible_front += 1;
                let winner = rendered
                    .iter()
                    .find(|face| face.face == owner.face)
                    .expect("the owner face remains in the rendered list");
                let front_face = &rendered[front.draw_order];
                if winner.side == front_face.side {
                    same_side += 1;
                } else {
                    split_side += 1;
                }
                *winner_front_pairs
                    .entry((
                        winner.face,
                        front_face.face,
                        winner.surface_rank,
                        front_face.surface_rank,
                        winner.side,
                        front_face.side,
                    ))
                    .or_default() += 1;
            }
        }

        let x = pixel_index % VIEWPORT;
        let y = pixel_index / VIEWPORT;
        let mut adjacent = Vec::new();
        let mut has_adjacent_front_covering_current = false;
        let mut has_adjacent_front_not_covering_current = false;
        for (dx, dy) in [(-1_isize, 0_isize), (1, 0), (0, -1), (0, 1)] {
            let nx = x.checked_add_signed(dx);
            let ny = y.checked_add_signed(dy);
            let Some((nx, ny)) = nx
                .zip(ny)
                .filter(|(nx, ny)| *nx < VIEWPORT && *ny < VIEWPORT)
            else {
                continue;
            };
            let neighbor_index = ny * VIEWPORT + nx;
            let neighbor = image.pixels[neighbor_index];
            let covers_current = neighbor.is_some_and(|neighbor| {
                pixel_covers
                    .iter()
                    .any(|cover| rendered[cover.draw_order].face == neighbor.face)
            });
            if let Some(neighbor) = neighbor.filter(|neighbor| !neighbor.back_facing) {
                if covers_current {
                    adjacent_front_covering_current += 1;
                    has_adjacent_front_covering_current = true;
                    if let Some(cover) = pixel_covers
                        .iter()
                        .filter(|cover| rendered[cover.draw_order].face == neighbor.face)
                        .min_by(|left, right| left.depth.total_cmp(&right.depth))
                    {
                        *adjacent_cover_pairs
                            .entry((
                                owner.face,
                                neighbor.face,
                                cover.depth_code.saturating_sub(minimum_depth_code),
                            ))
                            .or_default() += 1;
                    }
                } else {
                    adjacent_front_not_covering_current += 1;
                    has_adjacent_front_not_covering_current = true;
                }
                adjacent.push(serde_json::json!({
                    "dx": dx,
                    "dy": dy,
                    "face": neighbor.face,
                    "back_facing": neighbor.back_facing,
                    "covers_current_pixel": covers_current,
                }));
            }
        }
        if has_adjacent_front_covering_current {
            pixels_with_adjacent_front_covering_current += 1;
        } else if has_adjacent_front_not_covering_current {
            pixels_with_only_adjacent_front_not_covering_current += 1;
        }

        let candidate_json = pixel_covers
            .iter()
            .map(|cover| {
                let face = &rendered[cover.draw_order];
                serde_json::json!({
                    "face": face.face,
                    "surface_rank": face.surface_rank,
                    "side": face.side,
                    "side_times_surface_rank": face.side * i64::from(face.surface_rank),
                    "material_orientation": face.material_orientation,
                    "triangle": cover.triangle,
                    "back_facing": cover.back_facing,
                    "depth_code": cover.depth_code,
                    "minimum_depth_delta_codes": cover.depth_code.saturating_sub(minimum_depth_code),
                    "raw_minimum_depth_delta_codes":
                        (cover.depth - minimum_depth) * max_depth_code as f32,
                    "eligible": eligible(cover),
                    "draw_order": cover.draw_order,
                })
            })
            .collect::<Vec<_>>();
        let front_json = |cover: &Cover| {
            let face = &rendered[cover.draw_order];
            serde_json::json!({
                "face": face.face,
                "surface_rank": face.surface_rank,
                "side": face.side,
                "side_times_surface_rank": face.side * i64::from(face.surface_rank),
                "material_orientation": face.material_orientation,
                "depth_code": cover.depth_code,
                "minimum_depth_delta_codes": cover.depth_code.saturating_sub(minimum_depth_code),
                "raw_minimum_depth_delta_codes":
                    (cover.depth - minimum_depth) * max_depth_code as f32,
                "draw_order": cover.draw_order,
            })
        };
        if std::env::var_os("ZERO_OWNER_VERBOSE").is_some() {
            println!(
                "ZERO_OWNER_PIXEL {}",
                serde_json::to_string(&serde_json::json!({
                    "x": x,
                    "y": y,
                    "winner": {
                        "face": owner.face,
                        "back_facing": owner.back_facing,
                    },
                    "minimum_depth_code": minimum_depth_code,
                    "best_front_any": best_front_any.map(front_json),
                    "best_front_eligible": best_front_eligible.map(front_json),
                    "candidates": candidate_json,
                    "adjacent_front_winners": adjacent,
                }))
                .expect("owner pixel diagnosis serializes")
            );
        }
    }

    assert_eq!(diagnosed_back_pixels, image.light_pixels);
    assert_eq!(
        no_front_at_pixel + front_exists_but_farther + lost_to_eligible_front,
        diagnosed_back_pixels,
    );
    println!(
        "ZERO_OWNER_SUMMARY azimuth_deg=320 elevation_deg=20 viewport={} front_pixels={} back_pixels={} no_front_at_pixel={} front_exists_but_farther={} lost_to_eligible_front={} same_side={} split_side={} adjacent_front_covering_current={} adjacent_front_not_covering_current={} pixels_with_adjacent_front_covering_current={} pixels_with_only_adjacent_front_not_covering_current={} winner_front_pairs={winner_front_pairs:?} nearest_front_pairs={nearest_front_pairs:?} adjacent_cover_pairs={adjacent_cover_pairs:?}",
        VIEWPORT,
        image.red_pixels,
        image.light_pixels,
        no_front_at_pixel,
        front_exists_but_farther,
        lost_to_eligible_front,
        same_side,
        split_side,
        adjacent_front_covering_current,
        adjacent_front_not_covering_current,
        pixels_with_adjacent_front_covering_current,
        pixels_with_only_adjacent_front_not_covering_current,
    );
}

/// 裏画素を「表の覆いがある(所有者判定の失敗)」と「表の覆いが無い(幾何的露出)」へ分ける。
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

#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage1b_edge36_pose_back_pixels_are_tie_or_geometry() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage1b-edge36-pose", cp.clone(), 1.0, 1.0);
    let default_camera = camera(1.0, 1.0, 1.0);
    for (pose, final_angles) in [
        ("angles-now", zero_back_user_angles_now()),
        ("angles-old", zero_back_user_angles()),
    ] {
        let result = zero_back_warm_solve_to(
            &cp,
            &faces,
            &final_angles,
            ZeroBackWarmPath::Edge36Only,
            200,
        );
        let (front, back, covered) = covered_back_counts(&diagram, &result.frame, default_camera);
        let (strict_front, strict_back) =
            strict_nearest_counts(&diagram, &result.frame, default_camera);
        let angles = result
            .angles
            .iter()
            .map(|(&hinge, &angle)| (hinge, angle))
            .collect::<BTreeMap<_, _>>();
        println!(
            "STAGE1B pose={pose} rule_front={front} rule_back={back} rule_covered_back={covered} strict_front={strict_front} strict_back={strict_back} angles={:?}",
            angles
                .iter()
                .map(|(hinge, angle)| format!("{hinge}:{angle:.3}"))
                .collect::<Vec<_>>(),
        );
    }
}

/// `ORI3_STAGE1_IMAGE_DIR` を指定して実行したときだけ、CPU rasterの見た目をPPMで書き出す。
/// 既定では何も書かないので、追跡対象のファイルは変化しない。
#[test]
#[ignore = "調査用。ORI3_STAGE1_IMAGE_DIR を指定したときだけ画像を書く"]
fn stage1d_write_pose_images() {
    let Ok(directory) = std::env::var("ORI3_STAGE1_IMAGE_DIR") else {
        println!("STAGE1D skipped: ORI3_STAGE1_IMAGE_DIR is unset");
        return;
    };
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage1d-images", cp.clone(), 1.0, 1.0);
    let edge36 = zero_back_warm_solve_to(
        &cp,
        &faces,
        &zero_back_user_angles(),
        ZeroBackWarmPath::Edge36Only,
        200,
    );
    let user = zero_back_user_frame(&cp, &faces);
    for (name, frame) in [("edge36-only-200", &edge36.frame), ("user-pose", &user)] {
        for (view_name, view) in [
            ("front", camera(1.0, 1.0, 1.0)),
            ("back", camera(1.0, 1.0, -1.0)),
        ] {
            let image = visual_image(&diagram, frame, VIEWPORT, view);
            let mut bytes = format!("P6\n{VIEWPORT} {VIEWPORT}\n255\n").into_bytes();
            for pixel in &image.pixels {
                let rgb = match pixel {
                    None => [207_u8, 203, 194],
                    Some(pixel) if pixel.back_facing => [255, 255, 255],
                    Some(_) => [237, 28, 36],
                };
                bytes.extend_from_slice(&rgb);
            }
            let path = format!("{directory}/stage1d-{name}-{view_name}.ppm");
            std::fs::write(&path, bytes).expect("write diagnostic image");
            println!("STAGE1D wrote {path}");
        }
    }
}

#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage1c_edge36_pose_rule_versus_strict_over_612_directions() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage1c-edge36-pose-612", cp.clone(), 1.0, 1.0);
    let result = zero_back_warm_solve_to(
        &cp,
        &faces,
        &zero_back_user_angles(),
        ZeroBackWarmPath::Edge36Only,
        200,
    );
    let mut directions = 0_usize;
    let mut rule_back_total = 0_u64;
    let mut strict_back_total = 0_u64;
    let mut worse_directions = 0_usize;
    let mut worst = (0_i64, 0_i32, 0_i32);
    for elevation_deg in (-80_i32..=80).step_by(10) {
        for azimuth_deg in (0_i32..=350).step_by(10) {
            let view = camera_from_orbit_angles(1.0, 1.0, azimuth_deg, elevation_deg);
            let image = visual_image(&diagram, &result.frame, VIEWPORT, view);
            let (_, rule_back) = classified_fill_counts(&image);
            let (_, strict_back) = strict_nearest_counts(&diagram, &result.frame, view);
            directions += 1;
            rule_back_total += rule_back;
            strict_back_total += strict_back;
            let difference = rule_back as i64 - strict_back as i64;
            if difference > 0 {
                worse_directions += 1;
                if difference > worst.0 {
                    worst = (difference, azimuth_deg, elevation_deg);
                }
            }
        }
    }
    println!(
        "STAGE1C directions={directions} rule_back_total={rule_back_total} strict_back_total={strict_back_total} directions_rule_worse_than_strict={worse_directions} worst_excess={} at_az={} el={}",
        worst.0, worst.1, worst.2,
    );
}

#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage1_surface_rank_direct_versus_warm_start() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage1-rank-comparison", cp.clone(), 1.0, 1.0);
    let default_camera = camera(1.0, 1.0, 1.0);

    for (pose, final_angles) in [
        ("angles-now", zero_back_user_angles_now()),
        ("angles-old", zero_back_user_angles()),
    ] {
        let mut methods: Vec<(String, Frame3D, HashMap<EdgeId, f64>)> = Vec::new();

        // A0: 最終角をそのまま propagate して組み立てる基準形。
        let direct = zero_back_apply_overlap(&cp, &faces, {
            let folded = propagate(&cp, &faces, &final_angles);
            to_frame3d(&cp, &faces, &folded)
        });
        methods.push((
            "A0-propagate-direct".to_string(),
            direct,
            final_angles.clone(),
        ));

        // A1: 最終角をいきなり全部hardで1回solveする(warm無し)。
        let mut hard = final_angles
            .iter()
            .map(|(&hinge, &target_angle_deg)| Driver {
                hinge,
                target_angle_deg,
            })
            .collect::<Vec<_>>();
        hard.sort_unstable_by_key(|driver| driver.hinge);
        let cold = solve_motion(&cp, &faces, &hard, None, None, true);
        methods.push((
            "A1-solve-cold-one-shot".to_string(),
            zero_back_apply_overlap(&cp, &faces, cold.result.frame),
            cold.result.angles,
        ));

        // B: 0度から200段で少しずつ動かし、直前の解をwarm startにする。
        for path in [
            ZeroBackWarmPath::AllHardControl,
            ZeroBackWarmPath::Active36WithPreferred,
            ZeroBackWarmPath::Edge36Only,
        ] {
            let result = zero_back_warm_solve_to(&cp, &faces, &final_angles, path, 200);
            methods.push((
                format!("B-{}-200stages", path.label()),
                result.frame,
                result.angles,
            ));
        }

        let mut reference_ranks: Option<BTreeMap<FaceId, u32>> = None;
        for (label, frame, angles) in &methods {
            let ranks = frame
                .faces
                .iter()
                .map(|face| (face.face, face.surface_rank))
                .collect::<BTreeMap<_, _>>();
            let mismatched = reference_ranks.as_ref().map(|reference| {
                reference
                    .iter()
                    .filter(|(face, rank)| ranks.get(face) != Some(rank))
                    .map(|(&face, _)| face)
                    .collect::<Vec<_>>()
            });
            let (front, back, covered) = covered_back_counts(&diagram, frame, default_camera);
            let mut maximum_angle_delta = 0.0_f64;
            for (&hinge, &target) in &final_angles {
                let actual = angles.get(&hinge).copied().unwrap_or(0.0);
                let delta = (actual - target + 180.0).rem_euclid(360.0) - 180.0;
                maximum_angle_delta = maximum_angle_delta.max(delta.abs());
            }
            println!(
                "STAGE1 pose={pose} method={label} ranks={:?} mismatch_count={} mismatch_faces={:?} front={front} back={back} covered_back={covered} back_ratio={:.6}% max_angle_delta={maximum_angle_delta:.9}",
                ranks.values().copied().collect::<Vec<_>>(),
                mismatched.as_ref().map_or(0, Vec::len),
                mismatched.unwrap_or_default(),
                back as f64 / (front + back).max(1) as f64 * 100.0,
            );
            if reference_ranks.is_none() {
                reference_ranks = Some(ranks);
            }
        }
    }
}

#[test]
fn surface_order_minus_94_four_view_conditions() {
    let cp = angle_surface_cp();
    let faces = extract_faces(&cp);
    assert_eq!(cp.vertices.len(), 14);
    assert_eq!(cp.edges.len(), 29);
    assert_eq!(faces.len(), 16);
    let diagram = diagram("diagonal-midline-user-cp", cp.clone(), 1.0, 1.0);
    let mut valley_front = None;
    for (fold, angle) in [("mountain", 94.0), ("valley", -94.0)] {
        let folded = propagate(&cp, &faces, &angle_surface_angles(angle));
        let frame = to_frame3d(&cp, &faces, &folded);
        for (view_name, sign) in [("front", 1.0), ("back", -1.0)] {
            let image = visual_image(&diagram, &frame, VIEWPORT, camera(1.0, 1.0, sign));
            let (baseline_red, baseline_light) = match (fold, view_name) {
                ("mountain", "front") => (45_015_u64, 80_u64),
                ("mountain", "back") => (50_438, 6_244),
                ("valley", "front") => (45_777, 4_580),
                ("valley", "back") => (56_603, 79),
                _ => unreachable!("the four acceptance conditions are exhaustive"),
            };
            let baseline_ratio = baseline_red as f64 / (baseline_red + baseline_light) as f64;
            println!(
                "SURFACE_94 fold={fold} angle={angle:+.0} view={view_name} red={} light={} red_ratio={:.6}% baseline_red={} baseline_light={} baseline_red_ratio={:.6}% visible={} back_faces={}",
                image.red_pixels,
                image.light_pixels,
                image.red_ratio() * 100.0,
                baseline_red,
                baseline_light,
                baseline_ratio * 100.0,
                ids(&image.visible_faces),
                ids(&image.visible_back_faces),
            );
            assert!(image.red_pixels + image.light_pixels > 0);
            assert!(
                image.red_ratio() >= baseline_ratio,
                "{fold} {angle:+.0} degrees from the {view_name} must not regress below the measured baseline: red={} light={} ratio={:.9}% baseline={:.9}%",
                image.red_pixels,
                image.light_pixels,
                image.red_ratio() * 100.0,
                baseline_ratio * 100.0,
            );
            if fold == "valley" && view_name == "front" {
                valley_front = Some(image);
            }
        }
    }
    let valley_front = valley_front.expect("the four conditions include valley/front");
    assert!(
        valley_front.red_ratio() >= 0.909,
        "valley -94 degrees from the front must be at least 90.9% red: red={} light={}",
        valley_front.red_pixels,
        valley_front.light_pixels,
    );
}

#[test]
fn surface_order_same_input_is_deterministic_ten_times() {
    let cp = angle_surface_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("diagonal-midline-user-cp", cp.clone(), 1.0, 1.0);
    let angles = angle_surface_angles(-94.0);
    let mut expected = None::<(Vec<u8>, VisualImage)>;
    for run in 1..=10 {
        let frame = to_frame3d(&cp, &faces, &propagate(&cp, &faces, &angles));
        let serialized = serde_json::to_vec(&frame).expect("serialize deterministic frame");
        let image = visual_image(&diagram, &frame, VIEWPORT, camera(1.0, 1.0, 1.0));
        if let Some(reference) = &expected {
            assert_eq!(&serialized, &reference.0, "frame changed on run {run}");
            assert_eq!(&image, &reference.1, "visible result changed on run {run}");
        } else {
            expected = Some((serialized, image));
        }
    }
    println!("SURFACE_DETERMINISM identical_runs=10 input=edge43:-94");
}

#[test]
fn surface_order_user_pose_minus_180_is_deterministic_ten_times() {
    let cp = angle_surface_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("diagonal-midline-user-pose", cp.clone(), 1.0, 1.0);
    let angles = angle_surface_angles(-180.0);
    let mut expected = None::<(Vec<u8>, VisualImage)>;
    for run in 1..=10 {
        let frame = to_frame3d(&cp, &faces, &propagate(&cp, &faces, &angles));
        let serialized = serde_json::to_vec(&frame).expect("serialize deterministic user pose");
        let image = visual_image(&diagram, &frame, VIEWPORT, camera(1.0, 1.0, 1.0));
        if let Some(reference) = &expected {
            assert_eq!(
                &serialized, &reference.0,
                "user-pose frame changed on run {run}"
            );
            assert_eq!(
                &image, &reference.1,
                "user-pose visible result changed on run {run}"
            );
        } else {
            expected = Some((serialized, image));
        }
    }
    println!("SURFACE_DETERMINISM identical_runs=10 input=edge43:-180");
}

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

/// 段階1: 多数の姿勢で「現行規則の裏」と「重なり順を一切使わない最前面判定の裏」を比べる。
/// 合否は付けない(§10.7.7)。数を出して段階2の判断材料にする。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage1_random_pose_sweep_rule_versus_strict() {
    const SEED: u64 = 0x0123_4567_89AB_CDEF;
    const POSES: usize = 240;
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage1-sweep", cp.clone(), 1.0, 1.0);
    let views = [
        ("default", camera(1.0, 1.0, 1.0)),
        ("back", camera(1.0, 1.0, -1.0)),
        ("az0el0", camera_from_orbit_angles(1.0, 1.0, 0, 0)),
        ("az90el30", camera_from_orbit_angles(1.0, 1.0, 90, 30)),
        ("az200el-40", camera_from_orbit_angles(1.0, 1.0, 200, -40)),
        ("az285el55", camera_from_orbit_angles(1.0, 1.0, 285, 55)),
    ];
    let faceid_order = {
        let mut order = faces.iter().map(|face| face.id).collect::<Vec<_>>();
        order.sort_unstable();
        order
    };
    let poses = sweep_poses(&cp, &faces, SEED, POSES);
    let mut worse_poses = 0_usize;
    let mut clean_poses = 0_usize;
    let mut clean_worse_poses = 0_usize;
    let mut faceid_order_poses = 0_usize;
    let mut current_depth_branch = 0_usize;
    let mut rule_grand_total = 0_u64;
    let mut strict_grand_total = 0_u64;
    for pose in &poses {
        let order = surface_rank_order(&pose.frame);
        let is_faceid_order = order == faceid_order;
        // 裂けが小さく自己交差の無い形だけを「実際に折れる形」として別に数える。
        let is_clean = pose.max_seam_gap < 1e-6 && !pose.self_intersects;
        if is_faceid_order {
            faceid_order_poses += 1;
        }
        if is_clean {
            clean_poses += 1;
        }
        if !pose.needs_canonical_path && !pose.has_exact {
            current_depth_branch += 1;
        }
        let mut worst = (0_i64, "");
        let mut rule_total = 0_u64;
        let mut strict_total = 0_u64;
        for (view_name, view) in &views {
            let (_, rule_back, _) = covered_back_counts(&diagram, &pose.frame, *view);
            let (_, strict_back) = strict_nearest_counts(&diagram, &pose.frame, *view);
            rule_total += rule_back;
            strict_total += strict_back;
            let excess = rule_back as i64 - strict_back as i64;
            if excess > worst.0 {
                worst = (excess, view_name);
            }
        }
        rule_grand_total += rule_total;
        strict_grand_total += strict_total;
        if worst.0 > 0 {
            worse_poses += 1;
            if is_clean {
                clean_worse_poses += 1;
            }
            println!(
                "STAGE1SWEEP worse {} clean={is_clean} excess={} view={} rule_back={rule_total} strict_back={strict_total} faceid_order={is_faceid_order} needs_canonical={} has_exact={} seam={:.3e} intersects={} ranks={:?}",
                pose.label,
                worst.0,
                worst.1,
                pose.needs_canonical_path,
                pose.has_exact,
                pose.max_seam_gap,
                pose.self_intersects,
                order,
            );
        }
    }
    println!(
        "STAGE1SWEEP seed={SEED:#x} poses={POSES} views={} clean_poses={clean_poses} rule_worse_than_strict_poses={worse_poses} clean_rule_worse_than_strict_poses={clean_worse_poses} faceid_order_poses={faceid_order_poses} current_depth_branch_poses={current_depth_branch} rule_back_total={rule_grand_total} strict_back_total={strict_grand_total}",
        views.len(),
    );
}

#[path = "surface_order_sa_overlap.rs"]
mod surface_order_sa_overlap;
use surface_order_sa_overlap::*;
/// 段階2の測定。240通りの姿勢について
/// (1) 重なり順をどの経路で決めたかの件数
/// (2) 「上に来るべき面」を独立に求めた結果との食い違いの数
/// を出す。合否は付けない(§10.7.7)。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage2_random_pose_sweep_paths_and_top_faces() {
    const SEED: u64 = 0x0123_4567_89AB_CDEF;
    const POSES: usize = 240;
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage2-sweep", cp.clone(), 1.0, 1.0);
    let poses = sweep_poses(&cp, &faces, SEED, POSES);
    let mut sources = BTreeMap::<&'static str, usize>::new();
    let mut total = TopFaceAudit::default();
    let mut poses_with_mismatch = 0_usize;
    let mut poses_with_overlap = 0_usize;
    for pose in &poses {
        *sources.entry(pose.source_label).or_default() += 1;
        let mut lines = Vec::new();
        let audit = audit_top_faces(
            &cp,
            &faces,
            &diagram,
            &pose.frame,
            &pose.angles,
            |overlap, truth_left_above, rank_left_above, ladder| {
                // 梯子の高さの差と、その段の裂けの量を並べる。裂けのほうが大きければ
                // 「離れている」という判定は紙のちぎれに埋もれており、信用できない。
                let evidence = match ladder {
                    Some((height, seam)) => format!(
                        " ladder_height={height:.3e} ladder_seam={seam:.3e} height_over_seam={:.3}",
                        height.abs() / seam.max(f64::MIN_POSITIVE)
                    ),
                    None => " ladder=none".to_string(),
                };
                lines.push(format!(
                    "{}<->{} hinge={:?} truth_left_above={truth_left_above} rank_left_above={rank_left_above}{evidence}",
                    overlap.left, overlap.right, overlap.shared_hinge,
                ));
            },
        );
        total.overlaps += audit.overlaps;
        total.local_decided += audit.local_decided;
        total.ladder_decided += audit.ladder_decided;
        total.truth_disagreements += audit.truth_disagreements;
        total.rank_mismatches += audit.rank_mismatches;
        total.undecided += audit.undecided;
        if audit.overlaps > 0 {
            poses_with_overlap += 1;
        }
        if audit.rank_mismatches > 0 {
            poses_with_mismatch += 1;
            println!(
                "STAGE2MISMATCH {} source={} overlaps={} mismatches={} pairs=[{}] angles={:?}",
                pose.label,
                pose.source_label,
                audit.overlaps,
                audit.rank_mismatches,
                lines.join(" | "),
                pose.angles
                    .iter()
                    .map(|(&hinge, &angle)| (hinge, angle))
                    .collect::<BTreeMap<_, _>>()
                    .iter()
                    .map(|(hinge, angle)| format!("{hinge}:{angle:.6}"))
                    .collect::<Vec<_>>(),
            );
        }
    }
    for (label, count) in &sources {
        println!("STAGE2SOURCE {label}={count}");
    }
    println!(
        "STAGE2 seed={SEED:#x} poses={POSES} poses_with_overlap={poses_with_overlap} overlaps={} local_decided={} ladder_decided={} truth_disagreements={} undecided={} rank_mismatches={} poses_with_mismatch={poses_with_mismatch}",
        total.overlaps,
        total.local_decided,
        total.ladder_decided,
        total.truth_disagreements,
        total.undecided,
        total.rank_mismatches,
    );
}

/// 段階2で1件だけ残った食い違い(pose163 の面7と面13)を、
/// 独立判定と製品側の経路の両方から詳しく見る。合否は付けない。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage2_remaining_mismatch_detail() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage2-detail", cp.clone(), 1.0, 1.0);
    let poses = sweep_poses(&cp, &faces, 0x0123_4567_89AB_CDEF, 240);
    let wanted = std::env::var("ORI3_STAGE2_POSE").unwrap_or_else(|_| "pose163".to_string());
    let pose = poses
        .into_iter()
        .find(|pose| pose.label.starts_with(&wanted))
        .expect("the requested pose is in the fixed sweep");
    println!("DETAIL pose={} angles={:?}", pose.label, {
        let mut angles = pose
            .angles
            .iter()
            .map(|(&e, &a)| (e, a))
            .collect::<Vec<_>>();
        angles.sort_by_key(|pair| pair.0);
        angles
    });
    let exact = frame_polygons(&pose.frame);
    let overlaps = coincident_overlaps(&diagram, &pose.frame);
    let ranks = pose
        .frame
        .faces
        .iter()
        .map(|face| (face.face, face.surface_rank))
        .collect::<BTreeMap<_, _>>();
    println!(
        "DETAIL source={} overlaps={}",
        pose.source_label,
        overlaps.len()
    );
    for overlap in &overlaps {
        let local = local_hinge_truth(&cp, &faces, &pose.angles, &exact, overlap);
        println!(
            "DETAIL pair {}<->{} hinge={:?} rank_left={} rank_right={} local_truth={local:?}",
            overlap.left,
            overlap.right,
            overlap.shared_hinge,
            ranks[&overlap.left],
            ranks[&overlap.right],
        );
        // 全体を同じ割合で戻す梯子を、段ごとに裂けの量つきで出す。
        let mut warm = pose.angles.clone();
        for scale in [
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
        ] {
            let mut hard = pose
                .angles
                .iter()
                .map(|(&hinge, &angle)| Driver {
                    hinge,
                    target_angle_deg: angle * scale,
                })
                .collect::<Vec<_>>();
            hard.sort_unstable_by_key(|driver| driver.hinge);
            let motion = solve_motion(&cp, &faces, &hard, None, Some(&warm), false);
            warm = motion.result.angles.clone();
            let seam = ori3_rigid::max_seam_gap(&cp, &faces, &motion.result.frame);
            let probe = frame_polygons(&motion.result.frame);
            let difference = probe_height_difference(&exact, &probe, overlap);
            println!(
                "DETAIL   ladder scale={scale:.10} seam={seam:.3e} height_difference={difference:?}",
            );
        }
        // 製品側 canonical_motion_surface_order と同じ「共通の角度で頭打ちにして
        // 各点を解き直す」経路。
        let mut solved_warm: Option<HashMap<EdgeId, f64>> = None;
        for checkpoint in [9.0_f64, 90.0, 171.0, 179.0, 179.5, 179.9, 179.99, 179.999] {
            let mut hard = pose
                .angles
                .iter()
                .map(|(&hinge, &angle)| Driver {
                    hinge,
                    target_angle_deg: angle.signum() * angle.abs().min(checkpoint),
                })
                .collect::<Vec<_>>();
            hard.sort_unstable_by_key(|driver| driver.hinge);
            let motion = solve_motion(&cp, &faces, &hard, None, solved_warm.as_ref(), false);
            solved_warm = Some(motion.result.angles.clone());
            let seam = ori3_rigid::max_seam_gap(&cp, &faces, &motion.result.frame);
            let probe = frame_polygons(&motion.result.frame);
            let difference = probe_height_difference(&exact, &probe, overlap);
            println!(
                "DETAIL   solved-clamp checkpoint={checkpoint} seam={seam:.3e} height_difference={difference:?}",
            );
        }
        // 製品側と同じ「共通の角度で頭打ちにする」経路を、伝播だけで再現する。
        for checkpoint in [90.0_f64, 171.0, 179.0, 179.9, 179.99, 179.999] {
            let clamped = pose
                .angles
                .iter()
                .map(|(&hinge, &angle)| (hinge, angle.signum() * angle.abs().min(checkpoint)))
                .collect::<HashMap<_, _>>();
            let frame = to_frame3d(&cp, &faces, &propagate(&cp, &faces, &clamped));
            let seam = ori3_rigid::max_seam_gap(&cp, &faces, &frame);
            let probe = frame_polygons(&frame);
            let difference = probe_height_difference(&exact, &probe, overlap);
            println!(
                "DETAIL   clamp checkpoint={checkpoint} seam={seam:.3e} height_difference={difference:?}",
            );
        }
    }
}

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

/// 調査用。梯子の実測で上下が決まった面対が何組あり、そのうち刻印された
/// `surface_rank` と食い違う組が両端点でいくつあるかを数える。合否は付けない。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_determined_stacks_versus_surface_rank() {
    let diagrams = boundary_diagrams();
    let mut total_pairs = 0_usize;
    let mut total_determined = 0_usize;
    let mut before_bad = 0_usize;
    let mut after_bad = 0_usize;
    for diagram in &diagrams {
        for &(hinge, _) in &diagram.hinges {
            for sign in [1.0_f64, -1.0] {
                let ladder = boundary_ladder(diagram, hinge, sign);
                let determined = determined_stacks(&diagram.cp, &diagram.faces, &ladder);
                let before = &ladder[ladder.len() - 2].1.frame;
                let after = &ladder[ladder.len() - 1].1.frame;
                let pairs = near_overlaps(before, f64::INFINITY).len();
                let before_disagreements = stack_disagreements(before, &determined)
                    .into_iter()
                    .map(|(left, right, _)| (left, right))
                    .collect::<BTreeSet<_>>();
                let after_disagreements = stack_disagreements(after, &determined)
                    .into_iter()
                    .map(|(left, right, _)| (left, right))
                    .collect::<BTreeSet<_>>();
                total_pairs += pairs;
                total_determined += determined.len();
                // 両端点で食い違いが同じ面対は、端点の差ではなく最初から
                // 実測と合っていない面対である。端点の差はその対称差で数える。
                let only_before = before_disagreements
                    .difference(&after_disagreements)
                    .copied()
                    .collect::<Vec<_>>();
                let only_after = after_disagreements
                    .difference(&before_disagreements)
                    .copied()
                    .collect::<Vec<_>>();
                before_bad += only_before.len();
                after_bad += only_after.len();
                if !only_before.is_empty() || !only_after.is_empty() {
                    println!(
                        "DETSTACK diagram={} edge={hinge} sign={sign:+} pairs={pairs} determined={} shared_bad={} only_before={only_before:?} only_after={only_after:?}",
                        diagram.name,
                        determined.len(),
                        before_disagreements
                            .intersection(&after_disagreements)
                            .count(),
                    );
                }
                if diagram.name == "folded-sample.ori3" && (hinge == 306 || hinge == 425) {
                    println!(
                        "DETSTACK_FOCUS edge={hinge} sign={sign:+} determined={} focus={:?}",
                        determined.len(),
                        determined
                            .iter()
                            .filter(|stack| {
                                [(31, 34), (30, 35), (6, 8), (7, 9)]
                                    .contains(&(stack.left, stack.right))
                            })
                            .map(|stack| (
                                stack.left,
                                stack.right,
                                stack.samples,
                                stack.smallest_gap,
                                stack.largest_gap
                            ))
                            .collect::<Vec<_>>(),
                    );
                }
            }
        }
    }
    println!(
        "DETSTACK_TOTAL near_pairs={total_pairs} determined={total_determined} before_disagreements={before_bad} after_disagreements={after_bad}"
    );
}

/// 179.999°の姿勢は面どうしが実際に離れているので、重なり順の正解は実測の隙間だけで
/// 一意に決まる(カメラも履歴も要らない)。その正解と、刻印した `surface_rank` が
/// 合っているかを数える。合否は付けず、数値を出すだけの測定である。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn surface_rank_against_the_measured_gap_at_179_999() {
    let diagrams = boundary_diagrams();
    let mut total_pairs = 0_usize;
    let mut total_mismatch = 0_usize;
    let mut total_mismatch_after = 0_usize;
    for diagram in &diagrams {
        for (hinge, _) in diagram.hinges.clone() {
            for sign in [1.0_f64, -1.0] {
                let (before, after) = endpoint_frames(diagram, hinge, sign);
                let ranks = |frame: &Frame3D| {
                    frame
                        .faces
                        .iter()
                        .map(|face| (face.face, face.surface_rank))
                        .collect::<BTreeMap<_, _>>()
                };
                let before_rank = ranks(&before.frame);
                let after_rank = ranks(&after.frame);
                let pairs = near_overlaps(&before.frame, 1e-3);
                let mut mismatch_before = Vec::new();
                let mut mismatch_after = Vec::new();
                for pair in &pairs {
                    // 隙間が丸めより十分大きい面対だけを正解の根拠にする。
                    if pair.gap.abs() < 1e-9 {
                        continue;
                    }
                    total_pairs += 1;
                    let truth_right_above = pair.gap > 0.0;
                    if (before_rank[&pair.right] > before_rank[&pair.left]) != truth_right_above {
                        mismatch_before.push((pair.left, pair.right, pair.gap));
                        total_mismatch += 1;
                    }
                    if (after_rank[&pair.right] > after_rank[&pair.left]) != truth_right_above {
                        mismatch_after.push((pair.left, pair.right, pair.gap));
                        total_mismatch_after += 1;
                    }
                }
                println!(
                    "GAPTRUTH diagram={} edge={hinge} sign={sign:+} near_pairs={} mismatch_at_179_999={} mismatch_at_180={} before_detail={:?} after_detail={:?}",
                    diagram.name,
                    pairs.len(),
                    mismatch_before.len(),
                    mismatch_after.len(),
                    mismatch_before,
                    mismatch_after,
                );
            }
        }
    }
    println!(
        "GAPTRUTH_TOTAL pairs={total_pairs} mismatch_at_179_999={total_mismatch} mismatch_at_180={total_mismatch_after}"
    );
}

/// 調査用。`surface_order_exact_endpoint_is_rank_stable_for_previous_19` が使う
/// 「warm start無しで解き直す」経路が、warm startで到達した姿勢と同じ形になるかを測る。
/// 形が違えば、重なり順が違うのは当然である(刻印する順は表示している形を説明する)。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_cold_solve_reaches_the_same_pose() {
    const PREVIOUS_RANK_CHANGES: [EdgeId; 19] = [
        125, 143, 181, 183, 297, 298, 309, 314, 352, 358, 362, 367, 380, 393, 394, 401, 402, 426,
        430,
    ];
    let diagrams = boundary_diagrams();
    let diagram = diagrams
        .iter()
        .find(|item| item.name == "folded-sample.ori3")
        .expect("fixture");
    let mut worst = 0.0_f64;
    let mut differing = 0_usize;
    for hinge in PREVIOUS_RANK_CHANGES {
        for sign in [1.0_f64, -1.0] {
            let (_, after) = endpoint_frames(diagram, hinge, sign);
            let cold = solve_motion_with_contact_options(
                &diagram.cp,
                &diagram.faces,
                &[Driver {
                    hinge,
                    target_angle_deg: sign * 180.0,
                }],
                None,
                None,
                EXPLICIT_CONTACT_PREVENTION,
            );
            let delta = after
                .angles
                .iter()
                .map(|(edge, angle)| {
                    (angle - cold.result.angles.get(edge).copied().unwrap_or(f64::NAN)).abs()
                })
                .fold(0.0_f64, f64::max);
            let same_rank =
                surface_rank_order(&after.frame) == surface_rank_order(&cold.result.frame);
            if !same_rank {
                differing += 1;
            }
            worst = worst.max(delta);
            println!(
                "COLDPOSE edge={hinge} sign={sign:+} max_angle_difference_deg={delta:.6e} same_rank={same_rank}"
            );
        }
    }
    println!(
        "COLDPOSE_TOTAL cases={} worst_angle_difference_deg={worst:.6e} rank_differs={differing}",
        PREVIOUS_RANK_CHANGES.len() * 2
    );
}

/// 段階1の測定(残った2本)。**規則をいっさい使わず**、裂けていない姿勢の幾何だけから
/// 上下を決める。折り目を 179.0 → 179.5 → 179.9 → 179.99 → 179.999 → 180.0 と送り、
/// 各姿勢で(a)裂けの量、(b)隣接2面の重心が基準面の平面からどちら側へ離れているか、
/// (c)ほぼ平行で実面積が重なる面対の隙間の符号、を並べて印字する。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_remaining_two_creases_gap_sign_ladder() {
    let diagrams = boundary_diagrams();
    for (name, hinge) in [
        ("diagonal-midline-square", 12_u32),
        ("folded-sample.ori3", 425_u32),
    ] {
        let diagram = diagrams
            .iter()
            .find(|diagram| diagram.name == name)
            .expect("diagram exists");
        // 折り目に接する2面(展開図の接続だけで決まる。上下の規則は使わない)
        let adjacent = diagram
            .faces
            .iter()
            .filter(|face| face.edges.contains(&hinge))
            .map(|face| face.id)
            .collect::<Vec<_>>();
        println!("LADDER_SETUP diagram={name} edge={hinge} adjacent_faces={adjacent:?}");
        for sign in [1.0_f64, -1.0] {
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
                    EXPLICIT_CONTACT_PREVENTION,
                );
                warm = Some(motion.result.angles);
            }
            for absolute in [179.0, 179.5, 179.9, 179.99, 179.999, 180.0] {
                let motion = solve_motion_with_contact_options(
                    &diagram.cp,
                    &diagram.faces,
                    &[Driver {
                        hinge,
                        target_angle_deg: sign * absolute,
                    }],
                    None,
                    warm.as_ref(),
                    EXPLICIT_CONTACT_PREVENTION,
                );
                let frame = motion.result.frame.clone();
                let seam = ori3_rigid::max_seam_gap(&diagram.cp, &diagram.faces, &frame);
                let ranks = frame
                    .faces
                    .iter()
                    .map(|face| (face.face, face.surface_rank))
                    .collect::<BTreeMap<_, _>>();
                let polygons = frame_polygons(&frame);
                // (b) 隣接2面: 基準面の平面から相手の重心までの符号付き高さ。
                // 折り切る手前ではこの符号が「紙としてどちらが上か」そのものである。
                let mut adjacent_detail = Vec::new();
                if adjacent.len() == 2 {
                    let (left, right) = (adjacent[0], adjacent[1]);
                    let left_points = &polygons[&left];
                    let right_points = &polygons[&right];
                    if let (Some(left_normal), Some(right_normal)) =
                        (polygon_normal3(left_points), polygon_normal3(right_points))
                    {
                        let up = canonical3(left_normal);
                        let centroid = |points: &[V3]| {
                            points.iter().fold(V3::ZERO, |sum, &p| sum + p) / points.len() as f64
                        };
                        let height = (centroid(right_points) - centroid(left_points)).dot(up);
                        adjacent_detail.push((
                            left,
                            right,
                            height,
                            ranks[&right] > ranks[&left],
                            left_normal.dot(up),
                            right_normal.dot(up),
                        ));
                        println!(
                            "LADDER_NORMAL diagram={name} edge={hinge} sign={sign:+} target={absolute} face{left}_normal=({:.12},{:.12},{:.12}) face{right}_normal=({:.12},{:.12},{:.12}) up=({:.12},{:.12},{:.12})",
                            left_normal.x,
                            left_normal.y,
                            left_normal.z,
                            right_normal.x,
                            right_normal.y,
                            right_normal.z,
                            up.x,
                            up.y,
                            up.z,
                        );
                    }
                }
                // (c) ほぼ平行で実面積が重なる面対の隙間
                let pairs = near_overlaps(&frame, 1e-3);
                let detail = pairs
                    .iter()
                    .filter(|pair| pair.gap.abs() >= 1e-9)
                    .map(|pair| {
                        (
                            pair.left,
                            pair.right,
                            pair.gap,
                            (ranks[&pair.right] > ranks[&pair.left]) == (pair.gap > 0.0),
                        )
                    })
                    .collect::<Vec<_>>();
                let mut near_flat = motion
                    .result
                    .angles
                    .iter()
                    .filter(|(_, angle)| angle.abs() >= 179.9)
                    .map(|(&edge, &angle)| (edge, angle))
                    .collect::<Vec<_>>();
                near_flat.sort_by_key(|&(edge, _)| edge);
                let near_flat_faces = near_flat
                    .iter()
                    .map(|&(edge, angle)| {
                        let touching = diagram
                            .faces
                            .iter()
                            .filter(|face| face.edges.contains(&edge))
                            .map(|face| face.id)
                            .collect::<Vec<_>>();
                        (edge, angle, touching)
                    })
                    .collect::<Vec<_>>();
                println!(
                    "LADDER_FLAT_FACES diagram={name} edge={hinge} sign={sign:+} target={absolute} {near_flat_faces:?}"
                );
                println!(
                    "LADDER diagram={name} edge={hinge} sign={sign:+} target={absolute} driver={:.9} seam={seam:.3e} source={:?} adjacent={adjacent_detail:?} mismatched_pairs={} near_pairs={} ranks={:?} detail={detail:?} near_flat={near_flat:?}",
                    motion
                        .result
                        .angles
                        .get(&hinge)
                        .copied()
                        .unwrap_or(f64::NAN),
                    motion.surface_order,
                    detail.iter().filter(|item| !item.3).count(),
                    detail.len(),
                    if diagram.faces.len() <= 16 {
                        format!("{ranks:?}")
                    } else {
                        String::from("-")
                    },
                );
                if diagram.faces.len() <= 16 {
                    let mut all = motion
                        .result
                        .angles
                        .iter()
                        .map(|(&edge, &angle)| (edge, angle))
                        .collect::<Vec<_>>();
                    all.sort_by_key(|&(edge, _)| edge);
                    println!(
                        "LADDER_ANGLES diagram={name} edge={hinge} sign={sign:+} target={absolute} angles={all:?}"
                    );
                }
                warm = Some(motion.result.angles);
            }
        }
    }
}

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

/// 段階1(その2): `folded-sample.ori3` の辺425で食い違う面対が、経路のどの段で
/// どちらへ決まったのかを追う。製品コードは読むだけで、経路は公開APIで再現する。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_edge425_canonical_path_decision() {
    let diagrams = boundary_diagrams();
    let diagram = diagrams
        .iter()
        .find(|diagram| diagram.name == "folded-sample.ori3")
        .expect("fixture");
    let hinge = 425_u32;
    for sign in [-1.0_f64, 1.0] {
        for target in [179.999_f64, 180.0] {
            let mut warm = None::<HashMap<EdgeId, f64>>;
            for absolute in WARMUP_ABS.iter().copied().chain(
                BOUNDARY_ABS
                    .iter()
                    .copied()
                    .take_while(|&value| value <= target),
            ) {
                let motion = solve_motion_with_contact_options(
                    &diagram.cp,
                    &diagram.faces,
                    &[Driver {
                        hinge,
                        target_angle_deg: sign * absolute,
                    }],
                    None,
                    warm.as_ref(),
                    EXPLICIT_CONTACT_PREVENTION,
                );
                warm = Some(motion.result.angles);
            }
            let displayed_angles = warm.expect("ladder ran");
            let (path, exact) = canonical_path_frames(diagram, &displayed_angles);
            let exact_polygons = frame_polygons(&exact);
            let path_polygons = path.iter().map(frame_polygons).collect::<Vec<_>>();
            for (left, right) in [(6_u32, 8_u32), (7_u32, 9_u32)] {
                let left_points = &exact_polygons[&left];
                let right_points = &exact_polygons[&right];
                let (Some(plane), Some(right_normal)) = (
                    overlap_plane(left_points),
                    polygon_normal3(right_points).map(canonical3),
                ) else {
                    println!(
                        "PATHDEC sign={sign:+} target={target} pair=({left},{right}) no_plane"
                    );
                    continue;
                };
                let parallel = plane.normal.dot(right_normal);
                let coplanar_error = left_points
                    .iter()
                    .chain(right_points)
                    .map(|point| plane.normal.dot(*point - plane.origin).abs())
                    .fold(0.0_f64, f64::max);
                let left_2d = project2(left_points, &plane);
                let right_2d = project2(right_points, &plane);
                let left_triangles = triangulate_polygon(&left_2d);
                let right_triangles = triangulate_polygon(&right_2d);
                let Some((witness, area)) =
                    overlap_witness(&left_2d, &left_triangles, &right_2d, &right_triangles)
                else {
                    println!(
                        "PATHDEC sign={sign:+} target={target} pair=({left},{right}) no_overlap"
                    );
                    continue;
                };
                let overlap = CoincidentOverlap {
                    left,
                    right,
                    normal: plane.normal,
                    witness: plane.origin + plane.u * witness[0] + plane.v * witness[1],
                    shared_hinge: None,
                };
                let heights = path_polygons
                    .iter()
                    .enumerate()
                    .map(|(index, probe)| {
                        (
                            CANONICAL_CHECKPOINT_DEG[index],
                            probe_height_difference(&exact_polygons, probe, &overlap),
                        )
                    })
                    .collect::<Vec<_>>();
                let displayed = frame_polygons(
                    &solve_motion_with_contact_options(
                        &diagram.cp,
                        &diagram.faces,
                        &[Driver {
                            hinge,
                            target_angle_deg: sign * target,
                        }],
                        None,
                        Some(&displayed_angles),
                        EXPLICIT_CONTACT_PREVENTION,
                    )
                    .result
                    .frame,
                );
                let displayed_difference =
                    probe_height_difference(&exact_polygons, &displayed, &overlap);
                println!(
                    "PATHDEC sign={sign:+} target={target} pair=({left},{right}) parallel={parallel:.12} coplanar_error={coplanar_error:.3e} area={area:.3e} exact_normal=({:.12},{:.12},{:.12}) displayed_height_difference={displayed_difference:?} path={heights:?}",
                    plane.normal.x, plane.normal.y, plane.normal.z,
                );
            }
        }
    }
}

/// 段階1(その3): 米印の辺12で、上下の向きを決める「正準法線」がどの姿勢で
/// どちらを向くかを並べる。折り目の向きの規則は
/// **snapした表示姿勢を伝播した形**の面法線を、深度の規則は
/// **経路の終点(exact frame)**の面法線を使うため、両方を出す。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_kome_edge12_canonical_axis() {
    let diagrams = boundary_diagrams();
    let diagram = diagrams
        .iter()
        .find(|diagram| diagram.name == "diagonal-midline-square")
        .expect("fixture");
    let hinge = 12_u32;
    for sign in [1.0_f64, -1.0] {
        for target in [179.999_f64, 180.0] {
            let mut warm = None::<HashMap<EdgeId, f64>>;
            for absolute in WARMUP_ABS.iter().copied().chain(
                BOUNDARY_ABS
                    .iter()
                    .copied()
                    .take_while(|&value| value <= target),
            ) {
                let motion = solve_motion_with_contact_options(
                    &diagram.cp,
                    &diagram.faces,
                    &[Driver {
                        hinge,
                        target_angle_deg: sign * absolute,
                    }],
                    None,
                    warm.as_ref(),
                    EXPLICIT_CONTACT_PREVENTION,
                );
                warm = Some(motion.result.angles);
            }
            let displayed_angles = warm.expect("ladder ran");
            let snapped = displayed_angles
                .iter()
                .map(|(&edge, &angle)| {
                    (
                        edge,
                        if angle.abs() >= CANONICAL_STACK_FLAT_DEG {
                            angle.signum() * 180.0
                        } else {
                            angle
                        },
                    )
                })
                .collect::<HashMap<_, _>>();
            let propagated = to_frame3d(
                &diagram.cp,
                &diagram.faces,
                &propagate(&diagram.cp, &diagram.faces, &snapped),
            );
            let (_, exact) = canonical_path_frames(diagram, &displayed_angles);
            for (label, frame) in [
                ("snapped_propagated", &propagated),
                ("canonical_exact", &exact),
            ] {
                let polygons = frame_polygons(frame);
                for face in [3_u32, 4_u32] {
                    let normal = polygon_normal3(&polygons[&face]).expect("normal");
                    let up = canonical3(normal);
                    println!(
                        "AXIS sign={sign:+} target={target} {label} face={face} normal=({:.12},{:.12},{:.12}) up_is_normal={} abs_x_minus_abs_y={:.6e}",
                        normal.x,
                        normal.y,
                        normal.z,
                        normal.dot(up) > 0.0,
                        normal.x.abs() - normal.y.abs(),
                    );
                }
            }
        }
    }
}

/// 段階1(その4): 経路の終点(exact frame)で「法線が平行で投影が重なる」面対の
/// 平面からのずれが、どの桁に分布しているかを数える。
/// `surface_order.rs::COPLANAR_EPS`(1e-8)がこの分布のどこにあるかを見るための測定。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_exact_frame_coplanarity_histogram() {
    let diagrams = boundary_diagrams();
    let mut histogram = BTreeMap::<i32, usize>::new();
    let mut worst_same_stack = 0.0_f64;
    let mut worst_seam = 0.0_f64;
    for diagram in &diagrams {
        for &(hinge, _) in &diagram.hinges {
            for sign in [1.0_f64, -1.0] {
                let mut warm = None::<HashMap<EdgeId, f64>>;
                for absolute in WARMUP_ABS.iter().copied().chain(BOUNDARY_ABS) {
                    let motion = solve_motion_with_contact_options(
                        &diagram.cp,
                        &diagram.faces,
                        &[Driver {
                            hinge,
                            target_angle_deg: sign * absolute,
                        }],
                        None,
                        warm.as_ref(),
                        EXPLICIT_CONTACT_PREVENTION,
                    );
                    warm = Some(motion.result.angles);
                }
                let displayed_angles = warm.expect("ladder ran");
                let (_, exact) = canonical_path_frames(diagram, &displayed_angles);
                worst_seam = worst_seam.max(ori3_rigid::max_seam_gap(
                    &diagram.cp,
                    &diagram.faces,
                    &exact,
                ));
                let polygons = frame_polygons(&exact);
                let mut face_ids = polygons.keys().copied().collect::<Vec<_>>();
                face_ids.sort_unstable();
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
                        if plane.normal.dot(right_normal).abs() < 1.0 - 1e-8 {
                            continue;
                        }
                        let left_2d = project2(left_points, &plane);
                        let right_2d = project2(right_points, &plane);
                        let left_triangles = triangulate_polygon(&left_2d);
                        let right_triangles = triangulate_polygon(&right_2d);
                        if overlap_witness(&left_2d, &left_triangles, &right_2d, &right_triangles)
                            .is_none()
                        {
                            continue;
                        }
                        let error = left_points
                            .iter()
                            .chain(right_points)
                            .map(|point| plane.normal.dot(*point - plane.origin).abs())
                            .fold(0.0_f64, f64::max);
                        let decade = if error <= 0.0 {
                            -99
                        } else {
                            error.log10().floor() as i32
                        };
                        *histogram.entry(decade).or_default() += 1;
                        if error < 1e-4 {
                            worst_same_stack = worst_same_stack.max(error);
                        }
                    }
                }
            }
        }
    }
    println!(
        "COPLANARITY_HISTOGRAM worst_exact_seam={worst_seam:.3e} worst_error_below_1e-4={worst_same_stack:.3e} decades={histogram:?}"
    );
}

/// 段階1(その5): 画面側 (`surfaceOwner.ts::updateBatchViewOrder`) は
/// **Float32のposition bufferから読んだ頂点**で法線を組み立て、その絶対値を
/// 比べて正準法線の向きを決める。対称に折り切った形で \|x\| と \|y\| が
/// どれだけ違うかを、Float32へ丸めた頂点で測る。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_kome_edge12_float32_axis_choice() {
    let diagrams = boundary_diagrams();
    let diagram = diagrams
        .iter()
        .find(|diagram| diagram.name == "diagonal-midline-square")
        .expect("fixture");
    let hinge = 12_u32;
    for sign in [1.0_f64, -1.0] {
        for target in [179.999_f64, 180.0] {
            let mut warm = None::<HashMap<EdgeId, f64>>;
            for absolute in WARMUP_ABS.iter().copied().chain(
                BOUNDARY_ABS
                    .iter()
                    .copied()
                    .take_while(|&value| value <= target),
            ) {
                let motion = solve_motion_with_contact_options(
                    &diagram.cp,
                    &diagram.faces,
                    &[Driver {
                        hinge,
                        target_angle_deg: sign * absolute,
                    }],
                    None,
                    warm.as_ref(),
                    EXPLICIT_CONTACT_PREVENTION,
                );
                warm = Some(motion.result.angles);
            }
            let displayed = solve_motion_with_contact_options(
                &diagram.cp,
                &diagram.faces,
                &[Driver {
                    hinge,
                    target_angle_deg: sign * target,
                }],
                None,
                warm.as_ref(),
                EXPLICIT_CONTACT_PREVENTION,
            )
            .result
            .frame;
            let polygons = frame_polygons(&displayed);
            for face in [3_u32, 4_u32] {
                let exact_points = &polygons[&face];
                // 画面側と同じ: 頂点はFloat32、そこから先の掛け算・足し算はf64。
                let float32_points = exact_points
                    .iter()
                    .map(|point| {
                        V3::new(
                            f64::from(point.x as f32),
                            f64::from(point.y as f32),
                            f64::from(point.z as f32),
                        )
                    })
                    .collect::<Vec<_>>();
                let triangle_normal = |points: &[V3]| {
                    let mut normal = V3::ZERO;
                    for indices in &diagram.triangles[&face] {
                        let a = points[indices[0]];
                        let b = points[indices[1]];
                        let c = points[indices[2]];
                        normal += (b - a).cross(c - a);
                    }
                    normal.normalize()
                };
                let exact_normal = triangle_normal(exact_points);
                let float32_normal = triangle_normal(&float32_points);
                println!(
                    "F32AXIS sign={sign:+} target={target} face={face} f64=({:.12},{:.12},{:.12}) f64_abs_x_minus_abs_y={:.6e} f32=({:.12},{:.12},{:.12}) f32_abs_x_minus_abs_y={:.6e} f64_axis={} f32_axis={}",
                    exact_normal.x,
                    exact_normal.y,
                    exact_normal.z,
                    exact_normal.x.abs() - exact_normal.y.abs(),
                    float32_normal.x,
                    float32_normal.y,
                    float32_normal.z,
                    float32_normal.x.abs() - float32_normal.y.abs(),
                    if exact_normal.x.abs() >= exact_normal.y.abs() {
                        "x"
                    } else {
                        "y"
                    },
                    if float32_normal.x.abs() >= float32_normal.y.abs() {
                        "x"
                    } else {
                        "y"
                    },
                );
            }
        }
    }
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

/// 段階3: **折り目の向きの規則を使わずに**、裂けていない姿勢の隙間の符号だけから
/// 上下を決め、刻印された `surface_rank` と突き合わせる。
///
/// `crates/ori3-rigid/tests/surface_order.rs` の
/// `exact_folds_agree_with_the_surface_rank` は、`tree::exact_stack_constraints` が
/// 決めた `surface_rank` を**同じ規則**で照合しているため、規則そのものが誤って
/// いても違反0になる。ここでは規則をいっさい呼ばず、
///
/// 1. 折り切った折り目を**1本だけ**少し戻した姿勢を伝播で作り、
/// 2. その姿勢が**裂けていない**こと(`max_seam_gap < 1e-6`)を確かめ、
/// 3. 折り目に接する2面の重心が、表示姿勢の正準法線のどちら側にあるかを測り、
/// 4. その符号と `surface_rank` の上下が一致すること
///
/// だけを検査する。展開図と角度は検査コードへ埋め込んであり、`verification/` などの
/// 追跡対象外のファイルは読まない。期待値は数値ではなく「2つの実測量が一致する」と
/// いう関係なので、計算機が変わっても成り立つ(CLAUDE.md §10.7.7)。
#[test]
fn surface_rank_agrees_with_the_measured_gap_without_using_the_crease_rule() {
    let cases: [(&str, CreasePattern, HashMap<EdgeId, f64>); 4] = [
        ("live-frame", live_frame_cp(), live_frame_angles()),
        (
            "zero-back-user",
            zero_back_user_cp(),
            zero_back_user_angles(),
        ),
        (
            "diagonal-midline-square-edge12-mountain",
            flat_foldable_kome(),
            kome_edge12_angles(1.0),
        ),
        (
            "diagonal-midline-square-edge12-valley",
            flat_foldable_kome(),
            kome_edge12_angles(-1.0),
        ),
    ];
    let mut checked = 0_usize;
    for (name, cp, angles) in cases {
        let faces = extract_faces(&cp);
        let folded = to_frame3d(&cp, &faces, &propagate(&cp, &faces, &angles));
        checked +=
            assert_exact_creases_follow_the_measured_gap(name, &cp, &faces, &angles, &folded);
    }
    // この作業で直した `folded-sample.ori3` の辺425を含む3本。101本の角度はsolveで
    // 作るが、期待値は「実測の隙間」と「刻印された順位」の一致だけなので、solveの
    // 結果に数値を結び付けてはいない。
    let diagrams = boundary_diagrams();
    let diagram = diagrams
        .iter()
        .find(|diagram| diagram.name == "folded-sample.ori3")
        .expect("the folded acceptance fixture exists");
    for hinge in [425_u32, 12, 306] {
        for sign in [1.0_f64, -1.0] {
            let (_, folded) = endpoint_frames(diagram, hinge, sign);
            checked += assert_exact_creases_follow_the_measured_gap(
                &format!("folded-sample.ori3 edge{hinge} {sign:+}"),
                &diagram.cp,
                &diagram.faces,
                &folded.angles,
                &folded.frame,
            );
        }
    }
    println!("GAPRULEFREE checked_creases={checked}");
    // 主張の対象が空になっていないことの下限。裂ける探りや別の折り方へ飛んだ探りは
    // 根拠に使えないので、測れない折り目が残る。梯子の選別が壊れて対象が消えると、
    // この検査は何も主張しないまま緑になってしまう。
    //
    // **測れる本数そのものが計算機で変わる。** 探りは `ori3_rigid::solve` /
    // `solve_near` の結果であり、完全に折った状態のすぐ近くでは丸めの違いで解が
    // 近くの別の折り方へ移る(CLAUDE.md §10.7.7)。実測は
    // **この計算機で 50本、CI(GitHub Actions)の計算機で 49本**で、1本ずれた。
    // 以前は実測値 50 をそのまま下限にしていたため余裕が0で、CIが赤になった。
    //
    // そこで同じファイルの `robust_stacks`(実測4888に対し下限4,000 = 約82%)と
    // 同じ取り方にそろえ、実測の約8割にあたる **40本**を下限にする。実測49に対して
    // 18%、実測50に対して20%の余裕がある。空回りすれば0に近い値まで落ちるので、
    // この値でも検知できる。
    assert!(
        checked >= 40,
        "±180度の折り目をほとんど測れていない: checked={checked}"
    );
}

/// 折り切った折り目に接する2面が、少し戻した姿勢でどちら側へ離れるかを測って
/// `surface_rank` と突き合わせる。測れた折り目の本数を返す。
///
/// 戻した姿勢は `solved_unfold_ladder`(全ての折り目を同じ割合で縮めてsolveし、
/// **裂けた段は捨てる**梯子)を使う。製品の重なり順が使う「全ヒンジを共通の角度で
/// 頭打ちにする」経路とは刻み方が違う独立の道で、重なり順も刻印させない。
///
/// 「上」の向きは**表示する姿勢**での基準面の正準法線に取る。刻印された
/// `surface_rank` はその向きに下→上で並んでいるからである。戻した姿勢では基準面も
/// 少し動くので、平面の向きが 8°(内積0.99)以上ずれた段は使わない。また
/// **高さの差がその段の裂けの量以下**なら、離れている量より紙のちぎれのほうが
/// 大きいので答えの根拠にしない。
fn assert_exact_creases_follow_the_measured_gap(
    name: &str,
    cp: &CreasePattern,
    faces: &[Face],
    angles: &HashMap<EdgeId, f64>,
    folded: &Frame3D,
) -> usize {
    let ranks = folded
        .faces
        .iter()
        .map(|face| (face.face, face.surface_rank))
        .collect::<BTreeMap<_, _>>();
    let folded_polygons = frame_polygons(folded);
    let mut ladder = Vec::new();
    let mut warm = angles.clone();
    for probe_abs in [179.999_f64, 179.99, 179.9, 179.0, 175.0] {
        let mut drivers = angles
            .iter()
            .filter(|(_, angle)| angle.abs() > probe_abs)
            .map(|(&hinge, &angle)| Driver {
                hinge,
                target_angle_deg: angle.signum() * probe_abs,
            })
            .collect::<Vec<_>>();
        drivers.sort_unstable_by_key(|driver| driver.hinge);
        let solved = ori3_rigid::solve(cp, faces, &drivers, Some(&warm));
        let seam = ori3_rigid::max_seam_gap(cp, faces, &solved.frame);
        warm = solved.angles.clone();
        if seam < SEAM_TEAR_TOLERANCE
            && solved
                .frame
                .faces
                .iter()
                .flat_map(|face| &face.polygon)
                .flatten()
                .all(|coordinate| coordinate.is_finite())
        {
            ladder.push((seam, frame_polygons(&solved.frame)));
        }
    }
    let mut hinges = angles
        .iter()
        .filter(|(_, angle)| (angle.abs() - 180.0).abs() <= 1e-9)
        .map(|(&hinge, &angle)| (hinge, angle))
        .collect::<Vec<_>>();
    hinges.sort_by_key(|&(hinge, _)| hinge);
    let mut checked = 0_usize;
    for (hinge, angle) in hinges {
        let touching = faces
            .iter()
            .filter(|face| face.edges.contains(&hinge))
            .map(|face| face.id)
            .collect::<Vec<_>>();
        let [reference, other] = touching[..] else {
            continue;
        };
        let Some(axis) = folded_polygons
            .get(&reference)
            .and_then(|points| polygon_normal3(points))
            .map(canonical3)
        else {
            continue;
        };
        // 全ての折り切った折り目をまとめて戻した梯子で足りないときは、
        // **この折り目だけ**を戻した姿勢をsolveで作って探る。
        let mut probes = ladder.clone();
        if !probes.iter().any(|(seam, polygons)| {
            usable_probe_height(polygons, reference, other, axis, *seam).is_some()
        }) {
            let mut warm = angles.clone();
            for probe_abs in [179.999_f64, 179.99, 179.9, 179.0, 175.0] {
                let solved = ori3_rigid::solve_near(
                    cp,
                    faces,
                    &[Driver {
                        hinge,
                        target_angle_deg: angle.signum() * probe_abs,
                    }],
                    angles,
                    Some(&warm),
                );
                let seam = ori3_rigid::max_seam_gap(cp, faces, &solved.frame);
                let moved = max_vertex_distance(folded, &solved.frame);
                warm = solved.angles.clone();
                // 戻した角度が生む動きより桁違いに大きく動いた探りは、別の折り方へ
                // 飛んでいる。r度戻すと紙は高々 r ラジアン×紙の大きさ(≦1)だけ動く。
                if seam < SEAM_TEAR_TOLERANCE
                    && moved <= 20.0 * (angle.abs() - probe_abs).to_radians()
                    && solved
                        .frame
                        .faces
                        .iter()
                        .flat_map(|face| &face.polygon)
                        .flatten()
                        .all(|coordinate| coordinate.is_finite())
                {
                    probes.push((seam, frame_polygons(&solved.frame)));
                }
            }
        }
        for (seam, polygons) in &probes {
            let Some(height) = usable_probe_height(polygons, reference, other, axis, *seam) else {
                continue;
            };
            checked += 1;
            assert_eq!(
                ranks[&other] > ranks[&reference],
                height > 0.0,
                "{name}: 折り目{hinge}({angle:+.3}度)で、面{other}は面{reference}の\
                 実測で{height:.6e}(その段の裂け{seam:.3e})の側にあるのに、重なり順が逆である"
            );
            break;
        }
    }
    checked
}

/// 調査用。入力座標を1 ULPだけ動かしたときに、179.999°の姿勢で測った面対の
/// 隙間がどれだけ動くか(＝丸めの雑音の大きさ)を測る。合否は付けない。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_gap_noise_from_one_ulp_of_input() {
    let diagrams = boundary_diagrams();
    let base = diagrams
        .iter()
        .find(|item| item.name == "folded-sample.ori3")
        .expect("the folded acceptance fixture exists");
    // 入力の展開図の頂点座標を全て1 ULPだけ大きい側へ動かした複製。
    let mut nudged_cp = base.cp.clone();
    for vertex in &mut nudged_cp.vertices {
        for coordinate in &mut vertex.pos {
            *coordinate = f64::from_bits(coordinate.to_bits() + 1);
        }
    }
    let nudged = diagram(
        "folded-sample.ori3",
        nudged_cp,
        base.paper_width,
        base.paper_height,
    );

    let mut noise = Vec::<f64>::new();
    let mut sign_flips = 0_usize;
    let mut compared = 0_usize;
    for hinge in [306_u32, 425, 125, 12, 181, 297] {
        for sign in [1.0_f64, -1.0] {
            let (base_before, base_after) = endpoint_frames(base, hinge, sign);
            let (nudged_before, _) = endpoint_frames(&nudged, hinge, sign);
            let base_gaps = near_overlaps(&base_before.frame, f64::INFINITY)
                .into_iter()
                .map(|pair| ((pair.left, pair.right), pair.gap))
                .collect::<BTreeMap<_, _>>();
            let nudged_gaps = near_overlaps(&nudged_before.frame, f64::INFINITY)
                .into_iter()
                .map(|pair| ((pair.left, pair.right), pair.gap))
                .collect::<BTreeMap<_, _>>();
            for (key, base_gap) in &base_gaps {
                let Some(nudged_gap) = nudged_gaps.get(key) else {
                    continue;
                };
                compared += 1;
                noise.push((base_gap - nudged_gap).abs());
                if (*base_gap > 0.0) != (*nudged_gap > 0.0) {
                    sign_flips += 1;
                    println!(
                        "ULPFLIP edge={hinge} sign={sign:+} pair={key:?} base={base_gap:.6e} nudged={nudged_gap:.6e}"
                    );
                }
            }
            println!(
                "ULPNOISE edge={hinge} sign={sign:+} base_pairs={} nudged_pairs={} seam_before={:.3e} seam_after={:.3e}",
                base_gaps.len(),
                nudged_gaps.len(),
                ori3_rigid::max_seam_gap(&base.cp, &base.faces, &base_before.frame),
                ori3_rigid::max_seam_gap(&base.cp, &base.faces, &base_after.frame),
            );
        }
    }
    noise.sort_by(f64::total_cmp);
    let quantile = |ratio: f64| {
        noise
            .get(((noise.len() as f64 - 1.0) * ratio) as usize)
            .copied()
            .unwrap_or(f64::NAN)
    };
    println!(
        "ULPNOISE_TOTAL compared={compared} sign_flips={sign_flips} median={:.6e} p90={:.6e} p99={:.6e} max={:.6e}",
        quantile(0.5),
        quantile(0.9),
        quantile(0.99),
        noise.last().copied().unwrap_or(f64::NAN),
    );
}

/// 調査用。`surface_order_179_999_to_180_all_110_creases` が落ちる折り目について、
/// 順位が入れ替わった面対の隙間の実測値を出す。合否は付けない。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_rank_flip_pairs_measured_gap() {
    let diagrams = boundary_diagrams();
    for (name, hinge, sign) in [
        ("folded-sample.ori3", 306_u32, 1.0_f64),
        ("folded-sample.ori3", 306, -1.0),
        ("diagonal-midline-square", 12, 1.0),
        ("diagonal-midline-square", 12, -1.0),
    ] {
        let diagram = diagrams
            .iter()
            .find(|diagram| diagram.name == name)
            .expect("diagram exists");
        let (before, after) = endpoint_frames(diagram, hinge, sign);
        let rank_of = |frame: &Frame3D| {
            frame
                .faces
                .iter()
                .map(|face| (face.face, face.surface_rank))
                .collect::<BTreeMap<_, _>>()
        };
        let before_rank = rank_of(&before.frame);
        let after_rank = rank_of(&after.frame);
        let before_gaps = near_overlaps(&before.frame, f64::INFINITY)
            .into_iter()
            .map(|pair| ((pair.left, pair.right), pair.gap))
            .collect::<BTreeMap<_, _>>();
        let after_gaps = near_overlaps(&after.frame, f64::INFINITY)
            .into_iter()
            .map(|pair| ((pair.left, pair.right), pair.gap))
            .collect::<BTreeMap<_, _>>();
        println!(
            "FLIPSETUP diagram={name} edge={hinge} sign={sign:+} seam_before={:.3e} seam_after={:.3e} before_pairs={} after_pairs={}",
            ori3_rigid::max_seam_gap(&diagram.cp, &diagram.faces, &before.frame),
            ori3_rigid::max_seam_gap(&diagram.cp, &diagram.faces, &after.frame),
            before_gaps.len(),
            after_gaps.len(),
        );
        let mut faces = before_rank.keys().copied().collect::<Vec<_>>();
        faces.sort_unstable();
        let exact_error = |state: &EndpointState, left: FaceId, right: FaceId| {
            let (_, exact) = canonical_path_frames(diagram, &state.angles);
            let polygons = frame_polygons(&exact);
            let left_points = polygons.get(&left)?.clone();
            let right_points = polygons.get(&right)?.clone();
            let plane = overlap_plane(&left_points)?;
            let right_normal = polygon_normal3(&right_points).map(canonical3)?;
            let error = left_points
                .iter()
                .chain(&right_points)
                .map(|point| plane.normal.dot(*point - plane.origin).abs())
                .fold(0.0_f64, f64::max);
            Some((plane.normal.dot(right_normal), error))
        };
        for (left_index, &left) in faces.iter().enumerate() {
            for &right in &faces[left_index + 1..] {
                if (before_rank[&right] > before_rank[&left])
                    == (after_rank[&right] > after_rank[&left])
                {
                    continue;
                }
                println!(
                    "FLIPPAIR diagram={name} edge={hinge} sign={sign:+} pair=({left},{right}) before_gap={:?} after_gap={:?} vertex_move={:.3e} before_exact={:?} after_exact={:?}",
                    before_gaps
                        .get(&(left, right))
                        .map(|gap| format!("{gap:.6e}")),
                    after_gaps
                        .get(&(left, right))
                        .map(|gap| format!("{gap:.6e}")),
                    max_vertex_distance(&before.frame, &after.frame),
                    exact_error(&before, left, right).map(|(parallel, error)| format!(
                        "parallel={parallel:.9} coplanar_error={error:.6e}"
                    )),
                    exact_error(&after, left, right).map(|(parallel, error)| format!(
                        "parallel={parallel:.9} coplanar_error={error:.6e}"
                    )),
                );
            }
        }
    }
}

/// 探る姿勢での「基準面から相手面の重心までの符号付き高さ」。使えないとき `None`。
///
/// 使えない条件は2つ。基準面の平面が表示姿勢から8度(内積0.99)以上傾いた探りと、
/// **高さがその探りの裂けの量以下**のもの。後者は、面が離れている量より紙が
/// ちぎれている量のほうが大きく、符号に信号が無いためである。
fn usable_probe_height(
    polygons: &BTreeMap<FaceId, Vec<V3>>,
    reference: FaceId,
    other: FaceId,
    axis: V3,
    seam: f64,
) -> Option<f64> {
    let reference_points = polygons.get(&reference)?;
    let other_points = polygons.get(&other)?;
    if polygon_normal3(reference_points)?.dot(axis).abs() < 0.99 {
        return None;
    }
    let centroid = |points: &[V3]| {
        points.iter().fold(V3::ZERO, |sum, &point| sum + point) / points.len() as f64
    };
    let height = (centroid(other_points) - centroid(reference_points)).dot(axis);
    (height.abs() > seam.max(1e-9)).then_some(height)
}

/// 保存した層順序は編集用の `layer` にだけ残し、表示用 `surface_rank` は同じ
/// 8手を幾何だけで再生した順位と一致させる。面ID順を表示順位へ戻さない回帰検査。
#[test]
fn saved_layer_order_does_not_override_geometric_surface_rank() {
    const EXACT_OVERLAPS: [(FaceId, FaceId); 9] = [
        (4, 5),
        (8, 9),
        (14, 15),
        (24, 29),
        (25, 28),
        (26, 32),
        (27, 33),
        (30, 35),
        (31, 34),
    ];

    let saved_doc: Document =
        serde_json::from_str(FOLDED_SAMPLE).expect("folded-sample fixture is a Document");
    assert_eq!(saved_doc.sequence.len(), 8, "保存標本は8手である");
    let faces = extract_faces(&saved_doc.cp);
    assert_eq!(faces.len(), 46, "保存標本は46面である");

    let mut geometric_doc = saved_doc.clone();
    for step in &mut geometric_doc.sequence {
        step.layer_order = None;
    }
    assert_eq!(geometric_doc.sequence.len(), 8, "幾何版も同じ8手を再生する");

    let saved = ori3_layers::replay(&saved_doc, saved_doc.sequence.len(), 1.0);
    let geometric = ori3_layers::replay(&geometric_doc, geometric_doc.sequence.len(), 1.0);
    assert!(
        saved.surface_order_provenance.is_some(),
        "保存順版のsurface順位はcompleteな幾何導出である"
    );
    assert!(
        geometric.surface_order_provenance.is_some(),
        "幾何版のsurface順位はcompleteな幾何導出である"
    );
    assert!(
        saved.skipped.is_empty() && geometric.skipped.is_empty(),
        "8手を1つも飛ばさず再生する"
    );
    assert_eq!(saved.frame.faces.len(), 46, "保存順版の表示は46面である");
    assert_eq!(geometric.frame.faces.len(), 46, "幾何版の表示は46面である");
    assert_eq!(
        saved.hinge_angles, geometric.hinge_angles,
        "保存層順序の有無で再生角を変えない"
    );

    let ranks_by_face = |frame: &Frame3D| {
        frame
            .faces
            .iter()
            .map(|face| (face.face, face.surface_rank))
            .collect::<BTreeMap<_, _>>()
    };
    let saved_ranks = ranks_by_face(&saved.frame);
    let geometric_ranks = ranks_by_face(&geometric.frame);
    assert_eq!(
        saved_ranks, geometric_ranks,
        "保存層順序が表示用surface_rankを上書きした"
    );
    let numbered = saved_ranks
        .iter()
        .filter(|&(face, rank)| face == rank)
        .count();
    assert_eq!(
        numbered, 0,
        "surface_rankが面IDと同じ面が残っている: {saved_ranks:?}"
    );

    let saved_order =
        ori3_layers::saved_layer_order_at(&saved_doc, &faces, saved_doc.sequence.len(), 1.0)
            .expect("8手目の保存layer_orderを解決できる");
    let expected_layers = saved_order
        .iter()
        .enumerate()
        .map(|(layer, &face)| (u32::try_from(layer).expect("46層はu32に収まる"), face))
        .collect::<Vec<_>>();
    let mut actual_layers = saved
        .frame
        .faces
        .iter()
        .map(|face| (face.layer, face.face))
        .collect::<Vec<_>>();
    actual_layers.sort_unstable();
    assert_eq!(
        actual_layers, expected_layers,
        "編集用layerは保存layer_orderを保つ"
    );

    let long = saved_doc.paper.width_mm.max(saved_doc.paper.height_mm);
    let folded_sample = diagram(
        "folded-sample-saved-rank",
        saved_doc.cp.clone(),
        saved_doc.paper.width_mm / long,
        saved_doc.paper.height_mm / long,
    );
    let exact_pairs = coincident_overlaps(&folded_sample, &geometric.frame)
        .into_iter()
        .map(|pair| (pair.left.min(pair.right), pair.left.max(pair.right)))
        .collect::<BTreeSet<_>>();
    let required_exact_pairs = EXACT_OVERLAPS.into_iter().collect::<BTreeSet<_>>();
    assert!(
        required_exact_pairs.is_subset(&exact_pairs),
        "指定した完全重なり9組が欠けた: {:?}",
        required_exact_pairs
            .difference(&exact_pairs)
            .collect::<Vec<_>>()
    );
    for (left, right) in EXACT_OVERLAPS {
        assert_ne!(saved_ranks[&left], saved_ranks[&right]);
        assert_eq!(
            saved_ranks[&left] < saved_ranks[&right],
            geometric_ranks[&left] < geometric_ranks[&right],
            "完全重なり面({left}, {right})の上下が保存層順序で変わった"
        );
    }

    // `replay` と独立な直接伝播でも、指定9組の幾何的な上下が一致する。
    let canonical = to_frame3d(
        &saved_doc.cp,
        &faces,
        &propagate(&saved_doc.cp, &faces, &saved.hinge_angles),
    );
    let canonical_ranks = ranks_by_face(&canonical);
    for (left, right) in EXACT_OVERLAPS {
        assert_eq!(
            canonical_ranks[&left] < canonical_ranks[&right],
            geometric_ranks[&left] < geometric_ranks[&right],
            "直接伝播と幾何再生で完全重なり面({left}, {right})の上下が違う"
        );
    }

    let mut hard = saved
        .hinge_angles
        .iter()
        .map(|(&hinge, &target_angle_deg)| Driver {
            hinge,
            target_angle_deg,
        })
        .collect::<Vec<_>>();
    hard.sort_unstable_by_key(|driver| driver.hinge);
    let motion = solve_motion(
        &saved_doc.cp,
        &faces,
        &hard,
        None,
        Some(&saved.hinge_angles),
        true,
    );
    assert!(
        motion.surface_order_authoritative,
        "FOLDED_SAMPLEのmotion順はcompleteな幾何導出を刻印できる"
    );
    let diagnostics = motion
        .surface_order
        .expect("surface順を刻印するmotionは診断を返す");
    println!("FOLDED_SAMPLE_MOTION_DIAGNOSTICS {diagnostics:?}");
    // 現在の継続法はlive3の実motion pathだけで23重なり対をcompleteにできる。
    // flatからのcanonical fallbackへ進む前に、表示した運動そのものを採ることを固定する。
    assert_eq!(diagnostics.source, SurfaceOrderSource::SolvedMotionPath);
    // 以前は2件だった。捨てていた2件は「面7が面6より上」「面9が面8より上」で、
    // すき間の実測(`solved_unfold_ladder`)が示す上下そのものだった。捨てていた理由は、
    // 主対角 `x = y` 上の11本の折り目のうち7本が `−180°`、4本が `+180°` と
    // **割れて記録されていた**ことにある。主対角以外の90本はすべて 0.0° なので
    // 紙は両半分とも1枚の硬い板であり、同じ板どうしを折り重ねる折り目が
    // 山と谷に割れることは実際の紙では起こらない。
    // `tree::exact_stack_constraints` が割れを見つけてその板の組の厳密な拘束を
    // 出さなくなったので、上下は実測の深度が決め、**捨てる理由そのものが消えた**。
    // 期待値を緩めたのではなく、捨てる対象が無くなった結果である。
    assert_eq!(diagnostics.dropped_depth_constraints, 0);
    assert_eq!(diagnostics.unresolved_overlaps, 0);
    assert_eq!(diagnostics.broken_constraints, 0);
    let motion_ranks = ranks_by_face(&motion.result.frame);
    for (left, right) in EXACT_OVERLAPS {
        assert_eq!(
            motion_ranks[&left] < motion_ranks[&right],
            geometric_ranks[&left] < geometric_ranks[&right],
            "直接solveと幾何再生で完全重なり面({left}, {right})の上下が違う"
        );
    }

    println!(
        "FOLDED_SAMPLE_SAVED_RANK faces={} numbered={} overlaps={} saved_ranks={saved_ranks:?}",
        saved.frame.faces.len(),
        numbered,
        exact_pairs.len(),
    );
}

/// 完全に重なっている面対の上下が、**導出の経路によって変わらない**ことを固定する。
///
/// 重なり順を出す道は3つあり、どれも同じ答えでなければならない。
///
/// 1. `replay`(手順を再生し、その手の追従経路の深度で決める)
/// 2. `propagate` + `to_frame3d`(手順を通さず、最終角の伝播経路の深度で決める)
/// 3. `solve_motion`(平らな紙から最終角まで解いた運動の深度で決める)
///
/// さらに、この3つと独立した**すき間の実測**(`solved_unfold_ladder` は全ての折り目を
/// 同じ割合で縮めながら実際に解き、裂けた段を捨てる。重なり順は刻印させない)と
/// 突き合わせる。
///
/// **この検査が捕まえた不具合(2026-08-22)**: `folded-sample.ori3` の手順7で、
/// `replay` が渡す探り経路の3姿勢が**すべて同じ姿勢**で、終点から 0.5756(紙の長辺=1.0)
/// 離れたまま1度も動いていなかった。その止まった姿勢の高さの差が、完全重なり4組
/// (24,29)・(25,28)・(27,33)・(39,43) の上下を、**実際に表示する動きと逆**に決めていた。
/// 手順8は角度を1本も変えないので手順7の順をそのまま引き継ぎ、誤りが最終形へ残っていた。
#[test]
fn coincident_overlap_order_is_the_same_for_every_derivation() {
    let saved_doc: Document =
        serde_json::from_str(FOLDED_SAMPLE).expect("folded-sample fixture is a Document");
    let faces = extract_faces(&saved_doc.cp);
    let mut geometric_doc = saved_doc.clone();
    for step in &mut geometric_doc.sequence {
        step.layer_order = None;
    }
    let replayed = ori3_layers::replay(&geometric_doc, geometric_doc.sequence.len(), 1.0);
    assert!(
        replayed.surface_order_provenance.is_some(),
        "再生した重なり順はcompleteな幾何導出である"
    );
    let angles = replayed.hinge_angles.clone();

    let propagated = to_frame3d(
        &saved_doc.cp,
        &faces,
        &propagate(&saved_doc.cp, &faces, &angles),
    );

    let mut hard = angles
        .iter()
        .map(|(&hinge, &target_angle_deg)| Driver {
            hinge,
            target_angle_deg,
        })
        .collect::<Vec<_>>();
    hard.sort_unstable_by_key(|driver| driver.hinge);
    let motion = solve_motion(&saved_doc.cp, &faces, &hard, None, Some(&angles), true);
    assert!(
        motion.surface_order_authoritative,
        "同じ最終角のmotionはcompleteな幾何導出を刻印できる"
    );

    let ranks_of = |frame: &Frame3D| {
        frame
            .faces
            .iter()
            .map(|face| (face.face, face.surface_rank))
            .collect::<BTreeMap<_, _>>()
    };
    let replayed_ranks = ranks_of(&replayed.frame);
    let propagated_ranks = ranks_of(&propagated);
    let motion_ranks = ranks_of(&motion.result.frame);

    let long = saved_doc.paper.width_mm.max(saved_doc.paper.height_mm);
    let folded_sample = diagram(
        "folded-sample-every-derivation",
        saved_doc.cp.clone(),
        saved_doc.paper.width_mm / long,
        saved_doc.paper.height_mm / long,
    );
    let overlaps = coincident_overlaps(&folded_sample, &replayed.frame);
    // 主張の対象が空にならないことの下限。実測は23組で、その約8割を下限にする
    // (CLAUDE.md §10.7.9)。重なりの拾い方が壊れて対象が消えると、この検査は
    // 何も主張しないまま緑になってしまう。
    assert!(
        overlaps.len() >= 18,
        "完全に重なる面対をほとんど拾えていない: {}",
        overlaps.len()
    );

    let exact_polygons = frame_polygons(&replayed.frame);
    let ladder = solved_unfold_ladder(&saved_doc.cp, &faces, &angles);
    let mut measured = 0_usize;
    let mut measured_agrees = 0_usize;
    for overlap in &overlaps {
        let (left, right) = (overlap.left, overlap.right);
        let replayed_above = replayed_ranks[&left] > replayed_ranks[&right];
        let propagated_above = propagated_ranks[&left] > propagated_ranks[&right];
        let motion_above = motion_ranks[&left] > motion_ranks[&right];
        assert_eq!(
            replayed_above, propagated_above,
            "手順の再生と直接伝播で、完全重なり面({left}, {right})の上下が違う"
        );
        assert_eq!(
            replayed_above, motion_above,
            "手順の再生と運動の解で、完全重なり面({left}, {right})の上下が違う"
        );
        if let Some((above, _, _)) = ladder_truth(&exact_polygons, &ladder, overlap) {
            measured += 1;
            if above == replayed_above {
                measured_agrees += 1;
            }
        }
    }
    println!(
        "EVERY_DERIVATION overlaps={} measured={measured} measured_agrees={measured_agrees}",
        overlaps.len()
    );
    // すき間の実測で上下を読めた面対のうち、いくつが刻印された順位と一致したか。
    // 実測は 23組中23組を測れて 21組が一致する(残る2組は、折り切った折り目の向きが
    // 決める上下と実測が逆になる面対 (6,7)・(8,9) で、製品は丸めで壊れない
    // 折り目の向きを優先し、深度側2件を捨てている。`dropped_depth_constraints = 2`)。
    // 下限は実測21の約8割にあたる17とする(CLAUDE.md §10.7.9)。
    assert!(
        measured >= 18,
        "すき間から上下を読めた面対が少なすぎる: {measured}"
    );
    assert!(
        measured_agrees >= 17,
        "刻印された順位が、すき間の実測とほとんど一致していない: {measured_agrees}/{measured}"
    );
}

#[path = "surface_order_sa_warm_path_canonical.rs"]
mod surface_order_sa_warm_path_canonical;
