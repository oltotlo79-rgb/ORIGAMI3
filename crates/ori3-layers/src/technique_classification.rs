//! Deterministic classification boundary for named folding techniques.
//!
//! 名前は「構造が完全に一致したときだけ」名乗る。判定は、正規化した動きの指定と、
//! その動きが残した離散的な由来([`TechniqueEvidence`])の構造一致だけで行い、
//! できあがりの座標の近さからは決して名前を決めない。証明できないものは
//! 折り操作の失敗にせず「つかんで動かした折り」へ落とす。
//!
//! 候補は固定順で全件評価し、最初に真になったものを採らない。ちょうど1件の
//! ときだけ名前が決まる(0件と複数件はどちらも
//! [`DisplayTechniqueKind::GrabMove`]として保存する)。

use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;

use glam::DVec2;
use ori3_geometry::Isometry2;
use ori3_model::{
    DisplayTechniqueKind, EPS, FaceId, FoldStep, TechniqueClassification,
    TechniqueClassificationOrigin, TechniqueKind,
};

use crate::flat_motion::{FlatMotionInput, LayerTurn, MotionPart, MotionTransform};
use crate::fold_through::FoldDirection;

/// Evidence emitted by a geometry-specific recognizer.
///
/// `LayerOperation` deliberately never produces an automatic name: it is a
/// broad interaction family and may only be named when the user selects it
/// manually. `Insufficient` records that a recognizer could not prove its
/// proposed name; it therefore prevents automatic classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TechniqueWitness {
    Pleat,
    InsideReverse,
    OutsideReverse,
    Squash,
    Petal,
    OpenSink,
    Swivel,
    Twist,
    LayerOperation,
    Insufficient,
}

impl TechniqueWitness {
    fn named_kind(self) -> Option<TechniqueKind> {
        match self {
            Self::Pleat => Some(TechniqueKind::Pleat),
            Self::InsideReverse => Some(TechniqueKind::InsideReverse),
            Self::OutsideReverse => Some(TechniqueKind::OutsideReverse),
            Self::Squash => Some(TechniqueKind::Squash),
            Self::Petal => Some(TechniqueKind::Petal),
            Self::OpenSink => Some(TechniqueKind::OpenSink),
            Self::Swivel => Some(TechniqueKind::Swivel),
            Self::Twist => Some(TechniqueKind::Twist),
            Self::LayerOperation | Self::Insufficient => None,
        }
    }
}

/// Classifies an aligned motion only when its witnesses prove one name.
///
/// `Some` is a named technique. `None` means "つかんで動かした折り": there
/// were no matches, more than one distinct match, a layer operation, or
/// insufficient proof. The result depends on neither witness order nor the
/// floating-point representation used by the motion.
pub fn classify_aligned_motion(
    motion: &FlatMotionInput,
    witnesses: &[TechniqueWitness],
) -> Option<TechniqueKind> {
    if motion.parts.is_empty()
        || witnesses.iter().any(|witness| {
            matches!(
                witness,
                TechniqueWitness::LayerOperation | TechniqueWitness::Insufficient
            )
        })
    {
        return None;
    }

    let mut candidate = None;
    for &witness in witnesses {
        let Some(kind) = witness.named_kind() else {
            continue;
        };
        match candidate {
            None => candidate = Some(kind),
            Some(existing) if existing == kind => {}
            Some(_) => return None,
        }
    }
    candidate
}

/// 手動で選んだ折り方を、タイムラインへ出す表示名へ写す。
///
/// 汎用の[`TechniqueKind::Simple`]と[`TechniqueKind::Pose`]には表示名を与えない。
/// 単純折りは「層操作」を含むあらゆる動きの入れ物であり、`Pose`は折り方ではない。
pub fn display_kind_for_technique(kind: TechniqueKind) -> Option<DisplayTechniqueKind> {
    match kind {
        TechniqueKind::Pleat => Some(DisplayTechniqueKind::Pleat),
        TechniqueKind::InsideReverse => Some(DisplayTechniqueKind::InsideReverse),
        TechniqueKind::OutsideReverse => Some(DisplayTechniqueKind::OutsideReverse),
        TechniqueKind::Squash => Some(DisplayTechniqueKind::Squash),
        TechniqueKind::Petal => Some(DisplayTechniqueKind::Petal),
        TechniqueKind::OpenSink => Some(DisplayTechniqueKind::OpenSink),
        TechniqueKind::Swivel => Some(DisplayTechniqueKind::Swivel),
        TechniqueKind::Twist => Some(DisplayTechniqueKind::Twist),
        TechniqueKind::Simple | TechniqueKind::Pose => None,
    }
}

/// 自動判定の結果。
///
/// 候補を固定順に全件評価した結果であり、最初に真になったものを採らない。
/// ちょうど1件のときだけ名前が決まる。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutomaticTechniqueMatch {
    /// 候補がちょうど1件だった。
    Unique(DisplayTechniqueKind),
    /// 候補が0件だった。
    NoMatch,
    /// 候補が2件以上だった。並びは表示名の定義順に整えてある。
    Ambiguous(Vec<DisplayTechniqueKind>),
}

/// 証拠の集まりから自動判定の結果を作る。
///
/// 「層操作」は残り8技法をすべて表せる汎用表現なので、自動の候補にしない。
/// 証拠が足りないものが1つでもあれば、似ているというだけで名前を付けない。
pub fn automatic_match_from_witnesses(witnesses: &[TechniqueWitness]) -> AutomaticTechniqueMatch {
    if witnesses.iter().any(|witness| {
        matches!(
            witness,
            TechniqueWitness::LayerOperation | TechniqueWitness::Insufficient
        )
    }) {
        return AutomaticTechniqueMatch::NoMatch;
    }

    let mut candidates = Vec::new();
    for &witness in witnesses {
        let Some(kind) = witness.named_kind().and_then(display_kind_for_technique) else {
            continue;
        };
        if !candidates.contains(&kind) {
            candidates.push(kind);
        }
    }
    candidates.sort_unstable();
    match candidates.len() {
        0 => AutomaticTechniqueMatch::NoMatch,
        1 => AutomaticTechniqueMatch::Unique(candidates[0]),
        _ => AutomaticTechniqueMatch::Ambiguous(candidates),
    }
}

// ---------------------------------------------------------------------------
// 面順序・浮動小数に左右されない正規化(設計§3)
// ---------------------------------------------------------------------------

/// 動きの指定どうしを構造として比べるときの許容差。
///
/// [`crate::composite_motion_plan`]が同じ動きの指定を検証するときに使う値と同じで、
/// 等長変換を積み重ねた誤差を吸収するために置いてある。**これより広げない。**
/// 退化(線の長さが0、内側を示す点が線の上)の判定は、動きの適用側と同じ
/// [`ori3_model::EPS`]で行う。
const PLAN_EPS: f64 = 1e-6;

/// 端点の順序に依らない無向直線の安定key。
///
/// 直線を「有限な単位法線と定数」へ直し、法線の符号を辞書順で一意化してある。
/// 定数は2点の中点から求めるので、線の2点を入れ替えても**1bitも変わらない**
/// (符号反転は浮動小数でも厳密な操作なので、符号の付け直しで元へ戻る)。
///
/// `PartialEq` は完全一致の比較である。幾何としての同一性には
/// [`CanonicalSupport::approx_eq`]を使う。許容差は [`PLAN_EPS`] から広げない。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalSupport {
    normal: [f64; 2],
    offset: f64,
}

impl CanonicalSupport {
    /// 2点が張る直線の安定key。退化した線と有限でない値は`None`。
    pub fn from_segment(a: [f64; 2], b: [f64; 2]) -> Option<CanonicalSupport> {
        let (a, b) = (DVec2::from(a), DVec2::from(b));
        if !a.is_finite() || !b.is_finite() {
            return None;
        }
        let direction = b - a;
        let length = direction.length();
        if length <= EPS {
            return None;
        }
        let normal = DVec2::new(-direction.y, direction.x) / length;
        let offset = normal.dot((a + b) * 0.5);
        // 法線の符号を辞書順で一意にする(x が0なら y で決める)。
        let flip = if normal.x != 0.0 {
            normal.x < 0.0
        } else {
            normal.y < 0.0
        };
        let (normal, offset) = if flip {
            (-normal, -offset)
        } else {
            (normal, offset)
        };
        (normal.is_finite() && offset.is_finite()).then_some(CanonicalSupport {
            normal: [normal.x, normal.y],
            offset,
        })
    }

    fn normal(&self) -> DVec2 {
        DVec2::from(self.normal)
    }

    /// 法線の向きを正とする符号付き距離。
    pub fn signed_distance(&self, point: DVec2) -> f64 {
        self.normal().dot(point) - self.offset
    }

    /// 同じ直線か。許容差は動きの指定を比べる [`PLAN_EPS`]。
    pub fn approx_eq(&self, other: &CanonicalSupport) -> bool {
        (self.normal() - other.normal()).length() <= PLAN_EPS
            && (self.offset - other.offset).abs() <= PLAN_EPS
    }

    /// 平行か(同じ直線も含む)。
    pub fn is_parallel_to(&self, other: &CanonicalSupport) -> bool {
        self.normal().perp_dot(other.normal()).abs() <= PLAN_EPS
    }

    /// 原点からいちばん近い、この直線上の点。
    fn point(&self) -> DVec2 {
        self.normal() * self.offset
    }

    fn endpoints(&self) -> (DVec2, DVec2) {
        let base = self.point();
        (base, base + DVec2::new(-self.normal[1], self.normal[0]))
    }

    /// この直線での鏡映。
    pub fn reflection(&self) -> Isometry2 {
        let (a, b) = self.endpoints();
        Isometry2::reflection(a, b)
    }

    /// 等長変換で写した直線。
    fn transformed(&self, iso: &Isometry2) -> Option<CanonicalSupport> {
        let (a, b) = self.endpoints();
        CanonicalSupport::from_segment(iso.apply(a).to_array(), iso.apply(b).to_array())
    }

    /// 2直線の交点。平行なら`None`。
    fn meeting_point(&self, other: &CanonicalSupport) -> Option<DVec2> {
        let (first, second) = (self.normal(), other.normal());
        let det = first.perp_dot(second);
        if det.abs() <= PLAN_EPS {
            return None;
        }
        let point = DVec2::new(
            (self.offset * second.y - other.offset * first.y) / det,
            (other.offset * first.x - self.offset * second.x) / det,
        );
        point.is_finite().then_some(point)
    }
}

/// 手順が記録した折り目の目標角。
///
/// 支持線は**展開図の座標**である(手順は展開図の線として折り目を記録する)。
/// 畳み平面の plan と直接は比べられないので、判定では角度だけを使う。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalDriver {
    pub support: CanonicalSupport,
    pub target_degrees: f64,
}

/// 面の安定key。面IDの列挙順に依らないよう、開始材料面の由来へ引き戻してある。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CanonicalFaceKey {
    /// 分かれる前の、動き始めの時点の面ID。
    pub source: FaceId,
    /// この面を動かした部分の番号。動かない紙は`None`。
    pub part: Option<usize>,
}

/// できあがりの重なりで、すぐ下の紙とすぐ上の紙。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanonicalAdjacency {
    pub lower: CanonicalFaceKey,
    pub upper: CanonicalFaceKey,
}

/// 動きが残した**離散的な由来**(設計§2)。
///
/// 分類はこれと plan の構造だけで行い、できあがりの座標から意味を推測し直さない。
/// 証拠が無い動き(いまの単一反射の「つかんで動かす」など)では、それを必要とする
/// 判定器は名前を返さない。この値は保存せず、分類のときだけ使う。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TechniqueEvidence {
    /// 分かれた後の面 → 動き始めの面。
    pub parent_face_of: BTreeMap<FaceId, FaceId>,
    /// 重なりの中へ置き直した層=先端(動き始めの面ID)。
    /// 重なりを変えない指定(`Keep`・`CreaseOnly`)の層は入らない。
    pub tip_faces: BTreeSet<FaceId>,
    /// 袋ごとの分け方(動き始めの面ID → 袋の番号)。
    pub pocket_of: BTreeMap<FaceId, usize>,
    /// この動きで角度0°まで開いた既存の折り目(畳み平面の座標)。
    pub opened_spines: Vec<CanonicalSupport>,
    /// 紙を動かさない部分の番号。
    pub stationary_regions: BTreeSet<usize>,
    /// この手順が記録した折り目の目標角。
    pub target_drivers: Vec<CanonicalDriver>,
    /// できあがりの重なり(下→上)の隣り合わせ。
    pub final_adjacency: Vec<CanonicalAdjacency>,
    /// 配置が動き始めから変わった面(分かれた後の面ID)。許容差0で比べる。
    pub moved_faces: BTreeSet<FaceId>,
    /// 動きの折り線が横切っている、既存の折り目でつながった面の組。
    pub spine_pairs: Vec<[FaceId; 2]>,
}

impl TechniqueEvidence {
    /// できあがりの重なりを下から上へ並べた安定key。
    fn stack_keys(&self) -> Vec<CanonicalFaceKey> {
        let mut out: Vec<CanonicalFaceKey> = Vec::with_capacity(self.final_adjacency.len() + 1);
        for (index, pair) in self.final_adjacency.iter().enumerate() {
            if index == 0 {
                out.push(pair.lower);
            }
            out.push(pair.upper);
        }
        out
    }

    /// 重なりを下から上へ見たときの、部分の番号の並び(続きは1つにまとめる)。
    fn stack_part_runs(&self) -> Vec<Option<usize>> {
        let mut out: Vec<Option<usize>> = Vec::new();
        for key in self.stack_keys() {
            if out.last() != Some(&key.part) {
                out.push(key.part);
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// 正規化した plan
// ---------------------------------------------------------------------------

/// 畳み平面の半平面。`inside_is_positive`は法線の向きが内側かどうか。
#[derive(Clone, Copy, Debug, PartialEq)]
struct CanonicalHalfPlane {
    support: CanonicalSupport,
    inside_is_positive: bool,
}

/// 変換の**由来**。鏡映の列で書かれた変換は、その列のまま残す。
/// 等長変換を直接指定されたものは由来が消えているので`Direct`にする。
#[derive(Clone, Debug, PartialEq)]
enum CanonicalTransform {
    Stay,
    Reflections(Vec<CanonicalSupport>),
    Direct,
}

#[derive(Clone, Debug)]
struct CanonicalPart {
    index: usize,
    layers: Vec<FaceId>,
    region: Vec<CanonicalHalfPlane>,
    transform: CanonicalTransform,
    iso: Isometry2,
    turn: LayerTurn,
    reverse_layers: Option<bool>,
}

#[derive(Clone, Debug)]
struct CanonicalPlan {
    parts: Vec<CanonicalPart>,
}

impl CanonicalPlan {
    /// 動きの指定を正規化する。退化・非有限・重複supportがあれば`None`
    /// (別の名前へ丸めず、どの技法にも一致しない扱いにする)。
    fn from_motion(motion: &FlatMotionInput) -> Option<CanonicalPlan> {
        if motion.parts.is_empty() {
            return None;
        }
        let mut parts = Vec::with_capacity(motion.parts.len());
        for (index, spec) in motion.parts.iter().enumerate() {
            parts.push(canonical_part(index, spec)?);
        }
        Some(CanonicalPlan { parts })
    }
}

fn canonical_part(index: usize, spec: &MotionPart) -> Option<CanonicalPart> {
    let mut region: Vec<CanonicalHalfPlane> = Vec::with_capacity(spec.region.len());
    for half in &spec.region {
        let support = CanonicalSupport::from_segment(half.line[0], half.line[1])?;
        let inside = DVec2::from(half.inside_point);
        if !inside.is_finite() {
            return None;
        }
        let signed = support.signed_distance(inside);
        if signed.abs() <= EPS {
            // 内側を示す点が線の上にある。どちら側が動くのか一意に言えない。
            return None;
        }
        region.push(CanonicalHalfPlane {
            support,
            inside_is_positive: signed > 0.0,
        });
    }
    // 同じ領域に同じ支持線が2度出る(重複support)なら判定しない。
    for (position, half) in region.iter().enumerate() {
        if region[position + 1..]
            .iter()
            .any(|other| half.support.approx_eq(&other.support))
        {
            return None;
        }
    }

    let (transform, iso) = match &spec.transform {
        MotionTransform::Stay => (CanonicalTransform::Stay, Isometry2::identity()),
        MotionTransform::Isometry(iso) => {
            if !iso.rotation.is_finite() || !iso.translation.is_finite() {
                return None;
            }
            (CanonicalTransform::Direct, *iso)
        }
        MotionTransform::Reflect(lines) => {
            let mut supports = Vec::with_capacity(lines.len());
            let mut accumulated = Isometry2::identity();
            for line in lines {
                let support = CanonicalSupport::from_segment(line[0], line[1])?;
                let start = DVec2::from(line[0]);
                let direction = (DVec2::from(line[1]) - start).normalize();
                accumulated =
                    Isometry2::reflection(start, start + direction).compose(&accumulated);
                supports.push(support);
            }
            (CanonicalTransform::Reflections(supports), accumulated)
        }
    };

    let mut layers = spec.layers.clone();
    layers.sort_unstable();
    layers.dedup();
    Some(CanonicalPart {
        index,
        layers,
        region,
        transform,
        iso,
        turn: spec.turn,
        reverse_layers: spec.reverse_layers,
    })
}

fn reflections(part: &CanonicalPart) -> Option<&[CanonicalSupport]> {
    match &part.transform {
        CanonicalTransform::Reflections(supports) => Some(supports),
        _ => None,
    }
}

fn reflection_count(part: &CanonicalPart) -> Option<usize> {
    reflections(part).map(<[CanonicalSupport]>::len)
}

fn outside_direction(turn: LayerTurn) -> Option<FoldDirection> {
    match turn {
        LayerTurn::Outside(direction) => Some(direction),
        _ => None,
    }
}

fn inside_direction(turn: LayerTurn) -> Option<FoldDirection> {
    match turn {
        LayerTurn::Inside(direction) => Some(direction),
        _ => None,
    }
}

fn beside_anchor(turn: LayerTurn) -> Option<(FaceId, FoldDirection)> {
    match turn {
        LayerTurn::Beside { anchor, direction } => Some((anchor, direction)),
        _ => None,
    }
}

fn keeps_the_stack(turn: LayerTurn) -> bool {
    matches!(turn, LayerTurn::Keep)
}

/// 半平面の内側を正とする符号付き距離。
fn side_of(half: &CanonicalHalfPlane, point: DVec2) -> f64 {
    let signed = half.support.signed_distance(point);
    if half.inside_is_positive { signed } else { -signed }
}

/// 部分`later`が、部分`earlier`と支持線`support`で折り目としてつながっているか。
///
/// 折り目でつながったままなら、後の紙の変換は「前の紙が支持線を写した先での鏡映」を
/// 前の紙の変換に重ねたものになる。この形は plan の座標だけで確かめられるので、
/// できあがりの座標から意味を推測し直さずに済む。
fn joined_across(
    earlier: &CanonicalPart,
    later: &CanonicalPart,
    support: &CanonicalSupport,
) -> bool {
    let Some(image) = support.transformed(&earlier.iso) else {
        return false;
    };
    later
        .iso
        .approx_eq(&image.reflection().compose(&earlier.iso), PLAN_EPS)
}

// ---------------------------------------------------------------------------
// 8技法の保守的な判定条件(設計§4)
// ---------------------------------------------------------------------------

/// 段折り: 平行な2本の折り線と3つの帯。
fn matches_pleat(plan: &CanonicalPlan, evidence: &TechniqueEvidence) -> bool {
    if plan.parts.len() != 2 {
        return false;
    }
    let (middle, tip) = match (
        reflection_count(&plan.parts[0]),
        reflection_count(&plan.parts[1]),
    ) {
        (Some(1), Some(2)) => (&plan.parts[0], &plan.parts[1]),
        (Some(2), Some(1)) => (&plan.parts[1], &plan.parts[0]),
        _ => return false,
    };
    // 余分なrestack/reverseが無い。
    if middle.reverse_layers.is_some() || tip.reverse_layers.is_some() {
        return false;
    }
    if middle.layers != tip.layers || middle.layers.is_empty() {
        return false;
    }
    let (Some(way), Some(tip_way)) = (
        outside_direction(middle.turn),
        outside_direction(tip.turn),
    ) else {
        return false;
    };
    if way != tip_way {
        return false;
    }
    if middle.region.len() != 2 || tip.region.len() != 1 {
        return false;
    }
    let first = reflections(middle).expect("1鏡映")[0];
    let Some(near) = middle
        .region
        .iter()
        .find(|half| half.support.approx_eq(&first))
    else {
        return false;
    };
    let Some(far) = middle
        .region
        .iter()
        .find(|half| !half.support.approx_eq(&first))
    else {
        return false;
    };
    if !near.support.is_parallel_to(&far.support) {
        return false;
    }
    // 正幅(段の幅が0でない)。
    let gap = near.support.signed_distance(far.support.point());
    if gap.abs() <= PLAN_EPS {
        return false;
    }
    // 中帯は2本の折り線に挟まれている。
    if (gap > 0.0) != near.inside_is_positive {
        return false;
    }
    let back = far.support.signed_distance(near.support.point());
    if (back > 0.0) != far.inside_is_positive {
        return false;
    }
    // 先端帯は2本目の、1本目と反対側。
    let beyond = &tip.region[0];
    if !beyond.support.approx_eq(&far.support)
        || beyond.inside_is_positive == far.inside_is_positive
    {
        return false;
    }
    // 先端帯は平行2鏡映の合成で、中帯とは2本目の折り線でつながっている。
    let tip_lines = reflections(tip).expect("2鏡映");
    if !tip_lines[0].approx_eq(&first) || !tip_lines[1].is_parallel_to(&first) {
        return false;
    }
    if !joined_across(middle, tip, &far.support) {
        return false;
    }
    // できあがりが「元の紙・中帯・先端帯」のZ字。
    let expected = match way {
        FoldDirection::Up => vec![None, Some(middle.index), Some(tip.index)],
        FoldDirection::Down => vec![Some(tip.index), Some(middle.index), None],
    };
    evidence.stack_part_runs() == expected
}

/// 中割り折り/かぶせ折りの共通条件。違いは先端を層の内側へ入れるか外側へ出すか。
fn matches_reverse(plan: &CanonicalPlan, evidence: &TechniqueEvidence, inside: bool) -> bool {
    if plan.parts.len() != 2 {
        return false;
    }
    let (first, second) = (&plan.parts[0], &plan.parts[1]);
    if first.reverse_layers.is_some() || second.reverse_layers.is_some() {
        return false;
    }
    // 同じ折り線で1鏡映ずつ、領域はその折り線の同じ側の半平面1枚。
    let (Some(one), Some(other)) = (reflections(first), reflections(second)) else {
        return false;
    };
    if one.len() != 1 || other.len() != 1 || !one[0].approx_eq(&other[0]) {
        return false;
    }
    let support = one[0];
    if first.region.len() != 1 || second.region.len() != 1 {
        return false;
    }
    if !first.region[0].support.approx_eq(&support)
        || !second.region[0].support.approx_eq(&support)
        || first.region[0].inside_is_positive != second.region[0].inside_is_positive
    {
        return false;
    }
    // つながった先端どうしは反対向きへ回る。
    let turn_of = if inside {
        inside_direction
    } else {
        outside_direction
    };
    let (Some(up), Some(down)) = (turn_of(first.turn), turn_of(second.turn)) else {
        return false;
    };
    if up == down {
        return false;
    }
    // 2層以上で、2つの群は互いに素。
    if first.layers.is_empty()
        || second.layers.is_empty()
        || first.layers.len() + second.layers.len() < 2
        || first.layers.iter().any(|id| second.layers.contains(id))
    {
        return false;
    }
    // 親/tip由来と最終stackが無ければ判定不能。
    if evidence.final_adjacency.is_empty() || evidence.parent_face_of.is_empty() {
        return false;
    }
    for id in first.layers.iter().chain(second.layers.iter()) {
        if !evidence.tip_faces.contains(id) {
            return false;
        }
        // 折り線が全ての層を横切る(どの層も、動く子と動かない子に分かれている)。
        let children: Vec<FaceId> = evidence
            .parent_face_of
            .iter()
            .filter(|(_, parent)| *parent == id)
            .map(|(child, _)| *child)
            .collect();
        let moved = children
            .iter()
            .filter(|child| evidence.moved_faces.contains(child))
            .count();
        if children.len() < 2 || moved == 0 || moved == children.len() {
            return false;
        }
    }
    // spine接続graphが二部に分かれ、2つの群とちょうど一致する。
    let group_of = |id: &FaceId| {
        if first.layers.contains(id) {
            Some(false)
        } else if second.layers.contains(id) {
            Some(true)
        } else {
            None
        }
    };
    let mut linked = false;
    for [left, right] in &evidence.spine_pairs {
        let (Some(a), Some(b)) = (group_of(left), group_of(right)) else {
            continue;
        };
        if a == b {
            return false;
        }
        linked = true;
    }
    if !linked {
        return false;
    }
    // 先端の入り先。内側なら自分と同じ元の面の隣、外側なら選んだ束のいちばん外。
    let tips = [first.index, second.index];
    let keys = evidence.stack_keys();
    if !keys
        .iter()
        .any(|key| key.part.is_some_and(|part| tips.contains(&part)))
    {
        return false;
    }
    if inside {
        keys.iter().enumerate().all(|(at, key)| {
            if !key.part.is_some_and(|part| tips.contains(&part)) {
                return true;
            }
            let beside = |other: Option<&CanonicalFaceKey>| {
                other.is_some_and(|near| near.source == key.source && near.part != key.part)
            };
            beside(at.checked_sub(1).and_then(|below| keys.get(below))) || beside(keys.get(at + 1))
        })
    } else {
        let runs = evidence.stack_part_runs();
        !runs.iter().enumerate().any(|(at, part)| {
            part.is_some_and(|part| tips.contains(&part)) && at > 0 && at + 1 < runs.len()
        })
    }
}

/// 開いてつぶす折り: 既存の背が0°まで開き、支点を共有する二等分線で折る。
fn matches_squash(plan: &CanonicalPlan, evidence: &TechniqueEvidence) -> bool {
    if plan.parts.len() != 2 {
        return false;
    }
    let (near, far) = match (
        reflection_count(&plan.parts[0]),
        reflection_count(&plan.parts[1]),
    ) {
        (Some(1), Some(2)) => (&plan.parts[0], &plan.parts[1]),
        (Some(2), Some(1)) => (&plan.parts[1], &plan.parts[0]),
        _ => return false,
    };
    if near.reverse_layers.is_some() || far.reverse_layers.is_some() {
        return false;
    }
    // 手前寄りの半分は二等分線で折り返し、奥の半分は層まるごと回る。
    if near.region.len() != 1 || !far.region.is_empty() {
        return false;
    }
    let (Some(way), Some(far_way)) =
        (outside_direction(near.turn), outside_direction(far.turn))
    else {
        return false;
    };
    if way != far_way {
        return false;
    }
    let bisector = reflections(near).expect("1鏡映")[0];
    if !near.region[0].support.approx_eq(&bisector) {
        return false;
    }
    let far_lines = reflections(far).expect("2鏡映");
    if !far_lines[0].approx_eq(&bisector) {
        return false;
    }
    let closing = far_lines[1];
    // α=0(紙が1mmも動かない)は単なる重なり替えと区別できない。
    if bisector.approx_eq(&closing) {
        return false;
    }
    // 背を開いた由来が無ければ名前を付けない(設計§4)。
    let Some(spine) = evidence.opened_spines.iter().find(|spine| {
        spine
            .transformed(&bisector.reflection())
            .is_some_and(|image| image.approx_eq(&closing))
    }) else {
        return false;
    };
    if spine.approx_eq(&closing) {
        return false;
    }
    // 背・二等分線・行き先が同じ支点で交わる。
    let Some(pivot) = spine.meeting_point(&bisector) else {
        return false;
    };
    if closing.signed_distance(pivot).abs() > PLAN_EPS {
        return false;
    }
    joined_across(near, far, spine)
}

/// 花弁折り: 先端から出る二等分線2本と、それらを結ぶちょうつがい。
fn matches_petal(plan: &CanonicalPlan, evidence: &TechniqueEvidence) -> bool {
    if plan.parts.len() < 3 || evidence.pocket_of.is_empty() {
        return false;
    }
    if plan.parts.iter().any(|part| part.reverse_layers.is_some()) {
        return false;
    }
    // 持ち上げた紙は袋ごとに、その袋のいちばん外側の層の隣へ、同じ側へ入る。
    let mut way: Option<FoldDirection> = None;
    for part in &plan.parts {
        let Some((anchor, direction)) = beside_anchor(part.turn) else {
            return false;
        };
        if !evidence.pocket_of.contains_key(&anchor) {
            return false;
        }
        match way {
            None => way = Some(direction),
            Some(known) if known == direction => {}
            Some(_) => return false,
        }
    }
    let middles: Vec<&CanonicalPart> = plan
        .parts
        .iter()
        .filter(|part| reflection_count(part) == Some(1))
        .collect();
    let Some(middle) = middles.first().copied() else {
        return false;
    };
    let hinge = reflections(middle).expect("1鏡映")[0];
    if middles
        .iter()
        .any(|part| !reflections(part).expect("1鏡映")[0].approx_eq(&hinge))
    {
        return false;
    }
    if middle.region.len() < 3 {
        return false;
    }
    let Some(near) = middle
        .region
        .iter()
        .find(|half| half.support.approx_eq(&hinge))
    else {
        return false;
    };
    let bisectors: Vec<CanonicalHalfPlane> = middle
        .region
        .iter()
        .copied()
        .filter(|half| !half.support.approx_eq(&hinge))
        .collect();
    // 片翼だけでは他技法と見分けが付かないので名前を付けない。
    if bisectors.len() < 2 {
        return false;
    }
    let Some(tip) = bisectors[0].support.meeting_point(&bisectors[1].support) else {
        return false;
    };
    if bisectors
        .iter()
        .any(|half| half.support.signed_distance(tip).abs() > PLAN_EPS)
    {
        return false;
    }
    // ちょうつがいは先端を通らない(通ると折り返す紙が無くなる)。
    if hinge.signed_distance(tip).abs() <= PLAN_EPS {
        return false;
    }
    let wings: Vec<&CanonicalPart> = plan
        .parts
        .iter()
        .filter(|part| reflection_count(part) == Some(2))
        .collect();
    for bisector in &bisectors {
        let found = wings.iter().any(|wing| {
            let lines = reflections(wing).expect("2鏡映");
            lines[0].approx_eq(&bisector.support)
                && lines[1].approx_eq(&hinge)
                && wing.region.len() == 2
                && wing.region.iter().any(|half| {
                    half.support.approx_eq(&hinge)
                        && half.inside_is_positive == near.inside_is_positive
                })
                && wing.region.iter().any(|half| {
                    half.support.approx_eq(&bisector.support)
                        && half.inside_is_positive != bisector.inside_is_positive
                })
                && joined_across(middle, wing, &bisector.support)
        });
        if !found {
            return false;
        }
    }
    plan.parts.len() == wings.len() + middles.len()
}

/// 沈め折り: 紙は1mmも動かず、領域の中の重なりだけが裏返る。
fn matches_open_sink(plan: &CanonicalPlan, evidence: &TechniqueEvidence) -> bool {
    if plan.parts.len() != 1 {
        return false;
    }
    let part = &plan.parts[0];
    if part.transform != CanonicalTransform::Stay
        || !keeps_the_stack(part.turn)
        || part.reverse_layers != Some(true)
        || part.region.is_empty()
    {
        return false;
    }
    // 最終stackが無ければ判定不能。
    if evidence.final_adjacency.is_empty() {
        return false;
    }
    // 配置は1つも変わらない。
    if !evidence.moved_faces.is_empty() {
        return false;
    }
    // 境界で折り目を作り、その領域の紙がちゃんと重なりの中にある。
    !evidence.target_drivers.is_empty()
        && evidence
            .stack_keys()
            .iter()
            .any(|key| key.part == Some(part.index))
}

/// ひだ寄せ: 共通の支点で交わる基準線と二等分線。
fn matches_swivel(plan: &CanonicalPlan, _evidence: &TechniqueEvidence) -> bool {
    if plan.parts.len() != 2 {
        return false;
    }
    let (wedge, beyond) = match (
        reflection_count(&plan.parts[0]),
        reflection_count(&plan.parts[1]),
    ) {
        (Some(1), Some(2)) => (&plan.parts[0], &plan.parts[1]),
        (Some(2), Some(1)) => (&plan.parts[1], &plan.parts[0]),
        _ => return false,
    };
    if wedge.reverse_layers.is_some() || beyond.reverse_layers.is_some() {
        return false;
    }
    if outside_direction(wedge.turn).is_none() || !keeps_the_stack(beyond.turn) {
        return false;
    }
    if wedge.layers != beyond.layers {
        return false;
    }
    let bisector = reflections(wedge).expect("1鏡映")[0];
    if wedge.region.len() != 2 || beyond.region.len() != 1 {
        return false;
    }
    if !wedge
        .region
        .iter()
        .any(|half| half.support.approx_eq(&bisector))
    {
        return false;
    }
    let Some(base) = wedge
        .region
        .iter()
        .find(|half| !half.support.approx_eq(&bisector))
    else {
        return false;
    };
    // 反対側は基準線→二等分線の2鏡映=支点まわりの回転。
    let lines = reflections(beyond).expect("2鏡映");
    if !lines[0].approx_eq(&base.support) || !lines[1].approx_eq(&bisector) {
        return false;
    }
    if !beyond.region[0].support.approx_eq(&base.support)
        || beyond.region[0].inside_is_positive == base.inside_is_positive
    {
        return false;
    }
    // 共通の支点で交わり、角が0とπの近くで退化していない。
    if base.support.meeting_point(&bisector).is_none() {
        return false;
    }
    if base.support.normal().dot(bisector.normal()).abs() <= PLAN_EPS {
        return false;
    }
    joined_across(wedge, beyond, &base.support)
}

/// 回転の不動点(中心)。回転でない、または角が0なら`None`。
fn rotation_center(iso: &Isometry2) -> Option<DVec2> {
    if iso.mirrored {
        return None;
    }
    let angle = iso.rotation.rem_euclid(TAU);
    if angle.min(TAU - angle) <= PLAN_EPS {
        return None;
    }
    let (sin, cos) = angle.sin_cos();
    let det = 2.0 - 2.0 * cos;
    if det.abs() <= PLAN_EPS {
        return None;
    }
    let t = iso.translation;
    let center = DVec2::new(
        (t.x * (1.0 - cos) - sin * t.y) / det,
        ((1.0 - cos) * t.y + sin * t.x) / det,
    );
    center.is_finite().then_some(center)
}

/// ねじり折り: 中央の多角形・辺ごとのひだ・頂点ごとの腕で `2n+1` 部分。
fn matches_twist(plan: &CanonicalPlan, _evidence: &TechniqueEvidence) -> bool {
    let total = plan.parts.len();
    if total < 7 || total.is_multiple_of(2) {
        return false;
    }
    let sides = (total - 1) / 2;
    if plan.parts.iter().any(|part| part.reverse_layers.is_some()) {
        return false;
    }
    let centers: Vec<&CanonicalPart> = plan
        .parts
        .iter()
        .filter(|part| {
            part.region.len() == sides
                && outside_direction(part.turn).is_some()
                && rotation_center(&part.iso).is_some()
        })
        .collect();
    if centers.len() != 1 {
        return false;
    }
    let center = centers[0];
    let way = outside_direction(center.turn).expect("外側へ回す");
    let hub = rotation_center(&center.iso).expect("回転の中心");
    let edges = &center.region;

    // 中央多角形の頂点: 辺どうしの交点のうち、全ての半平面の内側にあるもの。
    let mut corners: Vec<(usize, usize, DVec2)> = Vec::new();
    for (first, one) in edges.iter().enumerate() {
        for (second, other) in edges.iter().enumerate().skip(first + 1) {
            let Some(point) = one.support.meeting_point(&other.support) else {
                continue;
            };
            if edges.iter().all(|half| side_of(half, point) >= -PLAN_EPS) {
                corners.push((first, second, point));
            }
        }
    }
    if corners.len() != sides {
        return false;
    }
    // 単純な閉じた多角形: どの辺もちょうど2つの頂点を持つ。
    for edge in 0..sides {
        let touching = corners
            .iter()
            .filter(|(first, second, _)| *first == edge || *second == edge)
            .count();
        if touching != 2 {
            return false;
        }
    }
    // 中心は多角形の内側。
    if !edges.iter().all(|half| side_of(half, hub) > PLAN_EPS) {
        return false;
    }

    // ひだ: 辺ごとに1つ、重なりを変えず、辺の外側にある。
    let pleats: Vec<&CanonicalPart> = plan
        .parts
        .iter()
        .filter(|part| keeps_the_stack(part.turn) && part.region.len() == 3)
        .collect();
    if pleats.len() != sides {
        return false;
    }
    let mut used = vec![false; sides];
    for pleat in &pleats {
        let found = (0..sides).find(|&edge| {
            !used[edge]
                && pleat.region.iter().any(|half| {
                    half.support.approx_eq(&edges[edge].support)
                        && half.inside_is_positive != edges[edge].inside_is_positive
                })
                && joined_across(center, pleat, &edges[edge].support)
        });
        let Some(edge) = found else {
            return false;
        };
        used[edge] = true;
    }

    // 腕: 頂点ごとに1つ、隣のひだと折り目でつながる。
    let arms: Vec<&CanonicalPart> = plan
        .parts
        .iter()
        .filter(|part| outside_direction(part.turn) == Some(way) && part.region.len() == 2)
        .collect();
    if arms.len() != sides {
        return false;
    }
    let mut taken = vec![false; corners.len()];
    for arm in &arms {
        let found = (0..corners.len()).find(|&corner| {
            !taken[corner]
                && arm
                    .region
                    .iter()
                    .all(|half| half.support.signed_distance(corners[corner].2).abs() <= PLAN_EPS)
        });
        let Some(corner) = found else {
            return false;
        };
        taken[corner] = true;
        let joined = pleats.iter().any(|pleat| {
            arm.region.iter().any(|half| {
                pleat
                    .region
                    .iter()
                    .any(|other| other.support.approx_eq(&half.support))
                    && joined_across(pleat, arm, &half.support)
            })
        });
        if !joined {
            return false;
        }
    }

    // 全頂点に4本の折り線が集まる。
    let mut supports: Vec<CanonicalSupport> = Vec::new();
    for part in &plan.parts {
        for half in &part.region {
            if !supports.iter().any(|known| known.approx_eq(&half.support)) {
                supports.push(half.support);
            }
        }
    }
    for (_, _, corner) in &corners {
        let touching = supports
            .iter()
            .filter(|support| support.signed_distance(*corner).abs() <= PLAN_EPS)
            .count();
        if touching != 4 {
            return false;
        }
    }
    total == 1 + pleats.len() + arms.len()
}

// ---------------------------------------------------------------------------
// 判定器の入口
// ---------------------------------------------------------------------------

/// 候補を評価する固定の順序。表示名の定義順にそろえてある。
type Matcher = fn(&CanonicalPlan, &TechniqueEvidence) -> bool;

const MATCHERS: [(TechniqueWitness, Matcher); 8] = [
    (TechniqueWitness::Pleat, matches_pleat),
    (TechniqueWitness::InsideReverse, matches_inside_reverse),
    (TechniqueWitness::OutsideReverse, matches_outside_reverse),
    (TechniqueWitness::Squash, matches_squash),
    (TechniqueWitness::Petal, matches_petal),
    (TechniqueWitness::OpenSink, matches_open_sink),
    (TechniqueWitness::Swivel, matches_swivel),
    (TechniqueWitness::Twist, matches_twist),
];

fn matches_inside_reverse(plan: &CanonicalPlan, evidence: &TechniqueEvidence) -> bool {
    matches_reverse(plan, evidence, true)
}

fn matches_outside_reverse(plan: &CanonicalPlan, evidence: &TechniqueEvidence) -> bool {
    matches_reverse(plan, evidence, false)
}

/// 動きと証拠から、名乗れる技法の証拠を集める。
///
/// 候補は固定順で**全件**評価し、最初に真になったものを採らない。
/// 正規化できない plan(退化・非有限・重複support)は1つも候補を出さない。
fn sim011_witnesses(
    motion: &FlatMotionInput,
    evidence: &TechniqueEvidence,
) -> Vec<TechniqueWitness> {
    let Some(plan) = CanonicalPlan::from_motion(motion) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (witness, matches) in MATCHERS {
        if matches(&plan, evidence) {
            found.push(witness);
        }
    }
    found
}

/// 動きと、その動きが残した離散的な由来から自動判定する。
///
/// 一致がちょうど1件のときだけ名前が決まる。0件・複数件の保存値は
/// [`DisplayTechniqueKind::GrabMove`]である。
pub fn classify_motion_plan(
    motion: &FlatMotionInput,
    evidence: &TechniqueEvidence,
) -> AutomaticTechniqueMatch {
    automatic_match_from_witnesses(&sim011_witnesses(motion, evidence))
}

/// SIM-011「つかんで動かす」で作った動きを、証拠なしで自動判定する。
///
/// 証拠を必要とする判定器は名前を返さないので、いまの単一反射の経路は
/// ここでは必ず「一致なし」になる。証拠がある呼び出しは
/// [`classify_motion_plan`]を使う。
pub fn classify_sim011_motion(motion: &FlatMotionInput) -> AutomaticTechniqueMatch {
    classify_motion_plan(motion, &TechniqueEvidence::default())
}

/// 手順へ載せる分類の指定。
///
/// 由来を決めるのはbackendだけで、画面から送られてくるJSONからは受け取らない。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TechniqueClassificationRequest {
    /// 利用者が折り方を選んだ。
    Explicit(DisplayTechniqueKind),
    /// 折った結果から自動で判定した。
    Automatic(AutomaticTechniqueMatch),
}

/// 直接折り([`ori3_model::SeqOp::FoldThrough`])を、利用者のどの操作から適用したか。
///
/// 画面が伝えるのは「どの操作をしたか」だけで、表示名を選ばせない。どの操作が
/// どの分類になるかは[`Self::classification_request`]だけが決める。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FoldThroughOrigin {
    /// 折り線を引いてから折った。利用者が折り線を決めた折りなので、
    /// 表示名は手順の折り方(`kind`)のままにする。
    #[default]
    DrawnFoldLine,
    /// 3D表示で紙をつかんでドラッグした(道具「折る」の掴み移動)。
    GrabMove,
}

impl FoldThroughOrigin {
    /// この操作に応じた分類の指定を作る。`None`は「分類を載せない」を表す。
    ///
    /// 掴み移動が作るのは折り線1本の直接折りであり、8技法の判定器が読む
    /// [`FlatMotionInput`]の形を持たない。したがって判定器に掛けられる候補は
    /// 0件で、[`AutomaticTechniqueMatch::NoMatch`]から
    /// [`DisplayTechniqueKind::GrabMove`]へ落ちる。名前を推測で付けない。
    #[must_use]
    pub fn classification_request(self) -> Option<TechniqueClassificationRequest> {
        match self {
            Self::DrawnFoldLine => None,
            Self::GrabMove => Some(TechniqueClassificationRequest::Automatic(
                AutomaticTechniqueMatch::NoMatch,
            )),
        }
    }
}

/// 直接折りで増えた手順へ、その操作に応じた分類を載せる。
///
/// 折り線を引いた折りには何も載せず、`kind`の表示名のままにする。載せる場合も
/// 唯一の入口[`assign_technique_classification`]を通る。
pub fn classify_fold_through_step(step: &mut FoldStep, origin: FoldThroughOrigin) {
    if let Some(request) = origin.classification_request() {
        assign_technique_classification(step, &request);
    }
}

/// 手順へ最終的な分類を載せる唯一の入口。
///
/// 自動・手動のどちらもここを通る。自動判定で名前が決まらなかった動きは、
/// 折り操作そのものを失敗させず[`DisplayTechniqueKind::GrabMove`]へ落とす。
/// [`FoldStep`]の`kind`・`drivers`・`layer_order`・`alignment`・`finish_soft`・
/// `note`には一切触らない。
pub fn assign_technique_classification(
    step: &mut FoldStep,
    request: &TechniqueClassificationRequest,
) {
    let classification = match request {
        TechniqueClassificationRequest::Explicit(kind) => TechniqueClassification {
            kind: *kind,
            origin: TechniqueClassificationOrigin::Explicit,
        },
        TechniqueClassificationRequest::Automatic(matched) => TechniqueClassification {
            kind: match matched {
                AutomaticTechniqueMatch::Unique(kind) => *kind,
                AutomaticTechniqueMatch::NoMatch | AutomaticTechniqueMatch::Ambiguous(_) => {
                    DisplayTechniqueKind::GrabMove
                }
            },
            origin: TechniqueClassificationOrigin::Automatic,
        },
    };
    step.technique_classification = Some(classification);
}

/// 差し替えの手順を書類へ入れる直前に、保存済みの表示名を引き継ぐ。
///
/// 利用者が折り方(`kind`)を選び直したときだけ、新しい折り方の表示へ戻すために
/// 分類を消す。同じ折り方のまま説明文などを直しただけの差し替えでは、
/// 保存済みの分類をそのまま残す。差し替え側が分類を明示していれば、それを使う。
pub fn carry_over_technique_classification(stored: &FoldStep, updated: &mut FoldStep) {
    if updated.technique_classification.is_none() && updated.kind == stored.kind {
        updated.technique_classification = stored.technique_classification;
    }
}

#[cfg(test)]
mod tests {
    use ori3_model::{
        DisplayTechniqueKind, FoldDirection, FoldStep, TechniqueClassification,
        TechniqueClassificationOrigin, TechniqueKind,
    };

    use super::{
        AutomaticTechniqueMatch, CanonicalAdjacency, CanonicalDriver, CanonicalFaceKey,
        CanonicalSupport, FoldThroughOrigin, TechniqueClassificationRequest, TechniqueEvidence,
        TechniqueWitness, assign_technique_classification, automatic_match_from_witnesses,
        carry_over_technique_classification, classify_aligned_motion, classify_fold_through_step,
        classify_motion_plan, classify_sim011_motion, display_kind_for_technique, sim011_witnesses,
    };
    use crate::flat_motion::{
        FlatMotionInput, HalfPlane, LayerTurn, MotionPart, MotionTransform,
    };
    use crate::fold_through::FoldDirection as Turn;
    use crate::techniques::twist_parts;
    use glam::DVec2;
    use ori3_model::FaceId;
    use std::collections::{BTreeMap, BTreeSet};

    fn aligned_motion() -> FlatMotionInput {
        FlatMotionInput {
            parts: vec![MotionPart::fold(
                vec![0],
                [[0.0, 0.0], [0.0, 1.0]],
                [1.0, 0.0],
                FoldDirection::Up,
            )],
            kind: TechniqueKind::Simple,
        }
    }

    macro_rules! names_exactly_one_witness {
        ($name:ident, $witness:expr, $kind:expr) => {
            #[test]
            fn $name() {
                let motion = aligned_motion();
                assert_eq!(classify_aligned_motion(&motion, &[$witness]), Some($kind));
            }
        };
    }

    names_exactly_one_witness!(
        classifies_pleat,
        TechniqueWitness::Pleat,
        TechniqueKind::Pleat
    );
    names_exactly_one_witness!(
        classifies_inside_reverse,
        TechniqueWitness::InsideReverse,
        TechniqueKind::InsideReverse
    );
    names_exactly_one_witness!(
        classifies_outside_reverse,
        TechniqueWitness::OutsideReverse,
        TechniqueKind::OutsideReverse
    );
    names_exactly_one_witness!(
        classifies_squash,
        TechniqueWitness::Squash,
        TechniqueKind::Squash
    );
    names_exactly_one_witness!(
        classifies_petal,
        TechniqueWitness::Petal,
        TechniqueKind::Petal
    );
    names_exactly_one_witness!(
        classifies_open_sink,
        TechniqueWitness::OpenSink,
        TechniqueKind::OpenSink
    );
    names_exactly_one_witness!(
        classifies_swivel,
        TechniqueWitness::Swivel,
        TechniqueKind::Swivel
    );
    names_exactly_one_witness!(
        classifies_twist,
        TechniqueWitness::Twist,
        TechniqueKind::Twist
    );

    #[test]
    fn returns_grabbed_move_for_zero_matches_or_a_layer_operation() {
        let motion = aligned_motion();
        assert_eq!(classify_aligned_motion(&motion, &[]), None);
        assert_eq!(
            classify_aligned_motion(&motion, &[TechniqueWitness::LayerOperation]),
            None
        );
    }

    #[test]
    fn returns_grabbed_move_for_ambiguous_matches() {
        let motion = aligned_motion();
        assert_eq!(
            classify_aligned_motion(
                &motion,
                &[TechniqueWitness::Pleat, TechniqueWitness::InsideReverse],
            ),
            None
        );
    }

    #[test]
    fn returns_grabbed_move_when_proof_is_insufficient() {
        let motion = aligned_motion();
        assert_eq!(
            classify_aligned_motion(
                &motion,
                &[TechniqueWitness::Pleat, TechniqueWitness::Insufficient]
            ),
            None
        );
    }

    #[test]
    fn classification_is_repeatable_and_independent_of_witness_order() {
        let motion = aligned_motion();
        let witnesses = [TechniqueWitness::Pleat, TechniqueWitness::Pleat];
        let first = classify_aligned_motion(&motion, &witnesses);
        assert_eq!(first, Some(TechniqueKind::Pleat));
        for _ in 0..32 {
            assert_eq!(classify_aligned_motion(&motion, &witnesses), first);
            assert_eq!(
                classify_aligned_motion(
                    &motion,
                    &[TechniqueWitness::Pleat, TechniqueWitness::Pleat]
                ),
                first
            );
        }
    }

    fn step_without_a_display_name() -> FoldStep {
        FoldStep {
            id: 7,
            kind: TechniqueKind::Simple,
            drivers: Vec::new(),
            layer_order: None,
            alignment: None,
            finish_soft: None,
            note: "説明".to_owned(),
            technique_classification: None,
        }
    }

    /// 利用者が選んだ折り方は、そのまま「利用者が選んだ」由来で載る。
    #[test]
    fn an_explicit_request_stores_the_chosen_name_as_chosen_by_the_user() {
        for kind in [
            DisplayTechniqueKind::LayerOperation,
            DisplayTechniqueKind::Pleat,
            DisplayTechniqueKind::Twist,
        ] {
            let mut step = step_without_a_display_name();
            assign_technique_classification(
                &mut step,
                &TechniqueClassificationRequest::Explicit(kind),
            );
            assert_eq!(
                step.technique_classification,
                Some(TechniqueClassification {
                    kind,
                    origin: TechniqueClassificationOrigin::Explicit,
                })
            );
            assert_eq!(step.kind, TechniqueKind::Simple, "折り方は変えない");
            assert_eq!(step.note, "説明", "説明文は変えない");
        }
    }

    /// 自動判定は、一意なときだけ名前を採り、0件・複数件は「つかんで動かした折り」にする。
    #[test]
    fn an_automatic_request_names_only_a_unique_match() {
        let cases = [
            (
                AutomaticTechniqueMatch::Unique(DisplayTechniqueKind::Squash),
                DisplayTechniqueKind::Squash,
            ),
            (
                AutomaticTechniqueMatch::NoMatch,
                DisplayTechniqueKind::GrabMove,
            ),
            (
                AutomaticTechniqueMatch::Ambiguous(vec![
                    DisplayTechniqueKind::Pleat,
                    DisplayTechniqueKind::Twist,
                ]),
                DisplayTechniqueKind::GrabMove,
            ),
        ];
        for (matched, expected) in cases {
            let mut step = step_without_a_display_name();
            assign_technique_classification(
                &mut step,
                &TechniqueClassificationRequest::Automatic(matched),
            );
            assert_eq!(
                step.technique_classification,
                Some(TechniqueClassification {
                    kind: expected,
                    origin: TechniqueClassificationOrigin::Automatic,
                })
            );
        }
    }

    /// 直接折りは、紙をつかんでドラッグした操作のときだけ表示名が載る。
    ///
    /// 折り線を引いた折りには何も載せず、`kind`の表示名のままにする。
    #[test]
    fn only_a_grabbed_fold_through_takes_an_automatic_name() {
        let mut grabbed = step_without_a_display_name();
        classify_fold_through_step(&mut grabbed, FoldThroughOrigin::GrabMove);
        assert_eq!(
            grabbed.technique_classification,
            Some(TechniqueClassification {
                kind: DisplayTechniqueKind::GrabMove,
                origin: TechniqueClassificationOrigin::Automatic,
            }),
            "つかんで動かした折りは、判定器が名前を決められない自動判定として載る"
        );
        assert_eq!(grabbed.kind, TechniqueKind::Simple, "折り方は変えない");
        assert_eq!(grabbed.note, "説明", "説明文は変えない");

        let mut drawn = step_without_a_display_name();
        classify_fold_through_step(&mut drawn, FoldThroughOrigin::DrawnFoldLine);
        assert_eq!(
            drawn.technique_classification, None,
            "折り線を引いた折りは表示名を載せない"
        );

        assert_eq!(
            FoldThroughOrigin::default(),
            FoldThroughOrigin::DrawnFoldLine,
            "印の無い旧入力は折り線を引いた折りとして扱う"
        );
        assert_eq!(FoldThroughOrigin::DrawnFoldLine.classification_request(), None);
        assert_eq!(
            FoldThroughOrigin::GrabMove.classification_request(),
            Some(TechniqueClassificationRequest::Automatic(
                AutomaticTechniqueMatch::NoMatch
            )),
            "掴み移動は8技法の判定器に掛けられる形を持たないので候補0件になる"
        );
    }

    /// 候補は固定順で全件評価する。0件・1件・2件以上を別々に固定する。
    #[test]
    fn candidates_are_counted_before_a_name_is_taken() {
        assert_eq!(
            automatic_match_from_witnesses(&[]),
            AutomaticTechniqueMatch::NoMatch
        );
        assert_eq!(
            automatic_match_from_witnesses(&[TechniqueWitness::Petal]),
            AutomaticTechniqueMatch::Unique(DisplayTechniqueKind::Petal)
        );
        // 層操作は残り8技法を表せる汎用表現なので、自動の候補にしない。
        assert_eq!(
            automatic_match_from_witnesses(&[TechniqueWitness::LayerOperation]),
            AutomaticTechniqueMatch::NoMatch
        );
        let ambiguous = AutomaticTechniqueMatch::Ambiguous(vec![
            DisplayTechniqueKind::Pleat,
            DisplayTechniqueKind::Twist,
        ]);
        assert_eq!(
            automatic_match_from_witnesses(&[TechniqueWitness::Twist, TechniqueWitness::Pleat]),
            ambiguous,
            "候補の並びは証拠の順に左右されない"
        );
        assert_eq!(
            automatic_match_from_witnesses(&[TechniqueWitness::Pleat, TechniqueWitness::Twist]),
            ambiguous
        );
    }

    /// 判定器が入るまで、SIM-011の動きは名前を持たない。
    #[test]
    fn a_grabbed_move_has_no_automatic_name_until_a_recognizer_exists() {
        assert_eq!(
            classify_sim011_motion(&aligned_motion()),
            AutomaticTechniqueMatch::NoMatch
        );
    }

    /// 汎用の`Simple`と`Pose`には表示名を与えない。
    #[test]
    fn the_generic_kinds_have_no_display_name() {
        assert_eq!(display_kind_for_technique(TechniqueKind::Simple), None);
        assert_eq!(display_kind_for_technique(TechniqueKind::Pose), None);
        assert_eq!(
            display_kind_for_technique(TechniqueKind::OpenSink),
            Some(DisplayTechniqueKind::OpenSink)
        );
    }

    /// 差し替えでは、折り方を選び直したときだけ表示名を消す。
    #[test]
    fn an_update_keeps_the_stored_name_unless_the_user_picks_another_technique() {
        let mut stored = step_without_a_display_name();
        assign_technique_classification(
            &mut stored,
            &TechniqueClassificationRequest::Explicit(DisplayTechniqueKind::LayerOperation),
        );

        let mut same_technique = step_without_a_display_name();
        same_technique.note = "説明を直した".to_owned();
        carry_over_technique_classification(&stored, &mut same_technique);
        assert_eq!(
            same_technique.technique_classification, stored.technique_classification,
            "同じ折り方のままの差し替えでは表示名を落とさない"
        );

        let mut another_technique = step_without_a_display_name();
        another_technique.kind = TechniqueKind::Pleat;
        carry_over_technique_classification(&stored, &mut another_technique);
        assert_eq!(
            another_technique.technique_classification, None,
            "折り方を選び直したときは選んだ折り方の表示へ戻す"
        );

        let mut already_named = step_without_a_display_name();
        assign_technique_classification(
            &mut already_named,
            &TechniqueClassificationRequest::Automatic(AutomaticTechniqueMatch::NoMatch),
        );
        let expected = already_named.technique_classification;
        carry_over_technique_classification(&stored, &mut already_named);
        assert_eq!(
            already_named.technique_classification, expected,
            "差し替え側が表示名を持つならそれを使う"
        );
    }

    // -----------------------------------------------------------------------
    // 8技法の正規な動きの指定(設計§4・§5)
    // -----------------------------------------------------------------------

    fn half(line: [[f64; 2]; 2], inside: [f64; 2]) -> HalfPlane {
        HalfPlane {
            line,
            inside_point: inside,
        }
    }

    fn part(
        layers: Vec<FaceId>,
        region: Vec<HalfPlane>,
        transform: MotionTransform,
        turn: LayerTurn,
    ) -> MotionPart {
        MotionPart {
            layers,
            region,
            transform,
            turn,
            reverse_layers: None,
        }
    }

    fn plan(parts: Vec<MotionPart>) -> FlatMotionInput {
        FlatMotionInput {
            parts,
            kind: TechniqueKind::Simple,
        }
    }

    fn line_key(line: [[f64; 2]; 2]) -> CanonicalSupport {
        CanonicalSupport::from_segment(line[0], line[1]).expect("退化していない線")
    }

    fn face(source: FaceId, part: Option<usize>) -> CanonicalFaceKey {
        CanonicalFaceKey { source, part }
    }

    /// 下から上へ並べた重なりを、隣り合わせの一覧へ直す。
    fn stack(keys: &[CanonicalFaceKey]) -> Vec<CanonicalAdjacency> {
        keys.windows(2)
            .map(|pair| CanonicalAdjacency {
                lower: pair[0],
                upper: pair[1],
            })
            .collect()
    }

    const SEAM_ONE: [[f64; 2]; 2] = [[0.0, 0.0], [0.0, 1.0]];
    const SEAM_TWO: [[f64; 2]; 2] = [[0.2, 0.0], [0.2, 1.0]];
    /// 1本目で折り返したあとの2本目(段折りの先端帯は、この2本で平行2鏡映になる)。
    const SEAM_TWO_TURNED: [[f64; 2]; 2] = [[-0.2, 0.0], [-0.2, 1.0]];
    const SPINE: [[f64; 2]; 2] = [[0.0, 0.0], [1.0, 0.0]];
    const BISECTOR: [[f64; 2]; 2] = [[0.0, 0.0], [1.0, 1.0]];
    const CLOSING: [[f64; 2]; 2] = [[0.0, 0.0], [0.0, 1.0]];
    const HINGE: [[f64; 2]; 2] = [[0.5, -1.0], [0.5, 1.0]];
    const RIGHT_BISECTOR: [[f64; 2]; 2] = [[0.0, 0.0], [0.866_025_403_784_438_6, -0.5]];
    const LEFT_BISECTOR: [[f64; 2]; 2] = [[0.0, 0.0], [0.866_025_403_784_438_6, 0.5]];
    const SINK_LINE: [[f64; 2]; 2] = [[0.5, 0.0], [0.5, 1.0]];

    /// 段折り: 平行な2本の折り線と、元の紙・中帯・先端帯の3つの帯。
    fn pleat_plan() -> FlatMotionInput {
        plan(vec![
            part(
                vec![1],
                vec![half(SEAM_ONE, [0.1, 0.5]), half(SEAM_TWO, [0.1, 0.5])],
                MotionTransform::Reflect(vec![SEAM_ONE]),
                LayerTurn::Outside(Turn::Up),
            ),
            part(
                vec![1],
                vec![half(SEAM_TWO, [0.5, 0.5])],
                MotionTransform::Reflect(vec![SEAM_ONE, SEAM_TWO_TURNED]),
                LayerTurn::Outside(Turn::Up),
            ),
        ])
    }

    fn pleat_evidence() -> TechniqueEvidence {
        TechniqueEvidence {
            final_adjacency: stack(&[face(1, None), face(1, Some(0)), face(1, Some(1))]),
            ..TechniqueEvidence::default()
        }
    }

    /// 中割り折り(先端を層の内側へ)/かぶせ折り(外側へ)の正規な動き。
    fn reverse_plan(tucks_inside: bool) -> FlatMotionInput {
        let turn = |direction| {
            if tucks_inside {
                LayerTurn::Inside(direction)
            } else {
                LayerTurn::Outside(direction)
            }
        };
        plan(vec![
            part(
                vec![1],
                vec![half(SEAM_ONE, [0.5, 0.5])],
                MotionTransform::Reflect(vec![SEAM_ONE]),
                turn(Turn::Up),
            ),
            part(
                vec![2],
                vec![half(SEAM_ONE, [0.5, 0.5])],
                MotionTransform::Reflect(vec![SEAM_ONE]),
                turn(Turn::Down),
            ),
        ])
    }

    fn reverse_evidence(tucks_inside: bool) -> TechniqueEvidence {
        let keys = if tucks_inside {
            // 先端はそれぞれ自分と同じ元の紙の隣へ入る。
            vec![
                face(1, None),
                face(1, Some(0)),
                face(2, Some(1)),
                face(2, None),
            ]
        } else {
            // 先端は選んだ束のいちばん外側を包む。
            vec![
                face(2, Some(1)),
                face(1, None),
                face(2, None),
                face(1, Some(0)),
            ]
        };
        TechniqueEvidence {
            parent_face_of: BTreeMap::from([(10, 1), (11, 1), (20, 2), (21, 2)]),
            moved_faces: BTreeSet::from([10, 20]),
            tip_faces: BTreeSet::from([1, 2]),
            spine_pairs: vec![[1, 2]],
            final_adjacency: stack(&keys),
            ..TechniqueEvidence::default()
        }
    }

    /// 開いてつぶす折り: 背が0°まで開き、支点を共有する二等分線で折る。
    fn squash_plan() -> FlatMotionInput {
        plan(vec![
            part(
                vec![1],
                vec![half(BISECTOR, [1.0, 0.0])],
                MotionTransform::Reflect(vec![BISECTOR]),
                LayerTurn::Outside(Turn::Up),
            ),
            part(
                vec![2],
                Vec::new(),
                MotionTransform::Reflect(vec![BISECTOR, CLOSING]),
                LayerTurn::Outside(Turn::Up),
            ),
        ])
    }

    fn squash_evidence() -> TechniqueEvidence {
        TechniqueEvidence {
            opened_spines: vec![line_key(SPINE)],
            ..TechniqueEvidence::default()
        }
    }

    /// 花弁折り: 先端から出る二等分線2本と、それらを結ぶちょうつがい。
    fn petal_plan() -> FlatMotionInput {
        let pocket = LayerTurn::Beside {
            anchor: 7,
            direction: Turn::Up,
        };
        plan(vec![
            part(
                vec![1],
                vec![
                    half(HINGE, [0.0, 0.0]),
                    half(RIGHT_BISECTOR, [0.25, -0.433_012_701_892_219_3]),
                ],
                MotionTransform::Reflect(vec![RIGHT_BISECTOR, HINGE]),
                pocket,
            ),
            part(
                vec![2],
                vec![
                    half(HINGE, [0.0, 0.0]),
                    half(LEFT_BISECTOR, [0.25, 0.433_012_701_892_219_3]),
                ],
                MotionTransform::Reflect(vec![LEFT_BISECTOR, HINGE]),
                pocket,
            ),
            part(
                vec![3],
                vec![
                    half(HINGE, [0.0, 0.0]),
                    half(RIGHT_BISECTOR, [0.25, 0.0]),
                    half(LEFT_BISECTOR, [0.25, 0.0]),
                ],
                MotionTransform::Reflect(vec![HINGE]),
                pocket,
            ),
        ])
    }

    fn petal_evidence() -> TechniqueEvidence {
        TechniqueEvidence {
            pocket_of: BTreeMap::from([(7, 0), (1, 0), (2, 0), (3, 0)]),
            ..TechniqueEvidence::default()
        }
    }

    /// 沈め折り: 紙は1mmも動かず、領域の中の重なりだけが裏返る。
    fn open_sink_plan() -> FlatMotionInput {
        plan(vec![MotionPart {
            layers: vec![1],
            region: vec![half(SINK_LINE, [0.8, 0.5])],
            transform: MotionTransform::Stay,
            turn: LayerTurn::Keep,
            reverse_layers: Some(true),
        }])
    }

    fn open_sink_evidence() -> TechniqueEvidence {
        TechniqueEvidence {
            target_drivers: vec![CanonicalDriver {
                support: line_key(SINK_LINE),
                target_degrees: 180.0,
            }],
            final_adjacency: stack(&[face(1, Some(0)), face(2, Some(0))]),
            ..TechniqueEvidence::default()
        }
    }

    /// ひだ寄せ: 共通の支点で交わる基準線と二等分線。
    fn swivel_plan() -> FlatMotionInput {
        plan(vec![
            part(
                vec![1],
                vec![half(SPINE, [1.0, 0.3]), half(BISECTOR, [1.0, 0.3])],
                MotionTransform::Reflect(vec![BISECTOR]),
                LayerTurn::Outside(Turn::Up),
            ),
            part(
                vec![1],
                vec![half(SPINE, [1.0, -0.3])],
                MotionTransform::Reflect(vec![SPINE, BISECTOR]),
                LayerTurn::Keep,
            ),
        ])
    }

    /// ねじり折り: 実物の生成器が作る `2n+1` 部分をそのまま使う(正五角形)。
    fn twist_plan() -> FlatMotionInput {
        let corners: Vec<DVec2> = (0..5)
            .map(|k| {
                let angle = std::f64::consts::TAU * f64::from(k) / 5.0;
                DVec2::new(angle.cos(), angle.sin())
            })
            .collect();
        plan(twist_parts(&[1], &[1], DVec2::ZERO, &corners, 0.4, Turn::Up))
    }

    fn named(motion: &FlatMotionInput, evidence: &TechniqueEvidence) -> AutomaticTechniqueMatch {
        classify_motion_plan(motion, evidence)
    }

    /// 手順へ実際に保存された表示名(故障注入はこの値で捕まえる)。
    fn stored(
        motion: &FlatMotionInput,
        evidence: &TechniqueEvidence,
    ) -> Option<TechniqueClassification> {
        let mut step = step_without_a_display_name();
        assign_technique_classification(
            &mut step,
            &TechniqueClassificationRequest::Automatic(classify_motion_plan(motion, evidence)),
        );
        step.technique_classification
    }

    fn automatic(kind: DisplayTechniqueKind) -> Option<TechniqueClassification> {
        Some(TechniqueClassification {
            kind,
            origin: TechniqueClassificationOrigin::Automatic,
        })
    }

    macro_rules! names_exactly_one_technique {
        ($name:ident, $motion:expr, $evidence:expr, $kind:expr) => {
            #[test]
            fn $name() {
                let (motion, evidence) = ($motion, $evidence);
                assert_eq!(
                    named(&motion, &evidence),
                    AutomaticTechniqueMatch::Unique($kind),
                    "正規な動きの指定はちょうど1つの技法に一致する"
                );
                assert_eq!(stored(&motion, &evidence), automatic($kind));
            }
        };
    }

    names_exactly_one_technique!(
        a_pleat_is_named_by_its_two_parallel_creases,
        pleat_plan(),
        pleat_evidence(),
        DisplayTechniqueKind::Pleat
    );
    names_exactly_one_technique!(
        an_inside_reverse_is_named_by_the_tips_it_tucks_in,
        reverse_plan(true),
        reverse_evidence(true),
        DisplayTechniqueKind::InsideReverse
    );
    names_exactly_one_technique!(
        an_outside_reverse_is_named_by_the_tips_it_wraps_around,
        reverse_plan(false),
        reverse_evidence(false),
        DisplayTechniqueKind::OutsideReverse
    );
    names_exactly_one_technique!(
        a_squash_is_named_by_the_spine_it_opens,
        squash_plan(),
        squash_evidence(),
        DisplayTechniqueKind::Squash
    );
    names_exactly_one_technique!(
        a_petal_is_named_by_its_two_bisectors_and_hinge,
        petal_plan(),
        petal_evidence(),
        DisplayTechniqueKind::Petal
    );
    names_exactly_one_technique!(
        an_open_sink_is_named_by_the_stack_it_turns_inside_out,
        open_sink_plan(),
        open_sink_evidence(),
        DisplayTechniqueKind::OpenSink
    );
    names_exactly_one_technique!(
        a_swivel_is_named_by_its_shared_pivot,
        swivel_plan(),
        TechniqueEvidence::default(),
        DisplayTechniqueKind::Swivel
    );
    names_exactly_one_technique!(
        a_twist_is_named_by_its_central_polygon,
        twist_plan(),
        TechniqueEvidence::default(),
        DisplayTechniqueKind::Twist
    );

    /// 何も証明できない動きは名前を持たず「つかんで動かした折り」になる。
    #[test]
    fn a_move_that_proves_no_technique_is_stored_as_a_grabbed_move() {
        let motion = aligned_motion();
        let evidence = TechniqueEvidence {
            // 由来はそろっているのに、動きの形がどの技法とも一致しない。
            parent_face_of: BTreeMap::from([(10, 1), (11, 1)]),
            moved_faces: BTreeSet::from([10]),
            tip_faces: BTreeSet::from([1]),
            spine_pairs: vec![[1, 2]],
            final_adjacency: stack(&[face(1, None), face(1, Some(0))]),
            ..TechniqueEvidence::default()
        };
        assert!(
            sim011_witnesses(&motion, &evidence).is_empty(),
            "候補は0件である"
        );
        assert_eq!(named(&motion, &evidence), AutomaticTechniqueMatch::NoMatch);
        assert_eq!(
            stored(&motion, &evidence),
            automatic(DisplayTechniqueKind::GrabMove)
        );
    }

    /// 2つの名前が同時に証明された動きも、名前を1つに決めずGrabMoveにする。
    ///
    /// 8つの判定条件は領域と重なりの入れ方で互いに排他になっており、成り立つ紙で
    /// 同時に2つを満たす動きは作れない。無効な作品を捏造せず、設計§5のとおり
    /// 候補集合から一意値を選ぶ純粋helperへ2候補を渡して固定する。
    #[test]
    fn two_proven_techniques_are_ambiguous_and_stored_as_a_grabbed_move() {
        let both = [TechniqueWitness::Squash, TechniqueWitness::Pleat];
        let matched = automatic_match_from_witnesses(&both);
        assert_eq!(
            matched,
            AutomaticTechniqueMatch::Ambiguous(vec![
                DisplayTechniqueKind::Pleat,
                DisplayTechniqueKind::Squash,
            ]),
            "候補の並びは証拠の順に左右されず、表示名の定義順になる"
        );
        let mut step = step_without_a_display_name();
        assign_technique_classification(
            &mut step,
            &TechniqueClassificationRequest::Automatic(matched),
        );
        assert_eq!(
            step.technique_classification,
            automatic(DisplayTechniqueKind::GrabMove)
        );
    }

    /// 線の端点を入れ替え、面と領域の並びも入れ替える。
    fn with_reversed_lines(motion: &FlatMotionInput) -> FlatMotionInput {
        let flip = |line: [[f64; 2]; 2]| [line[1], line[0]];
        FlatMotionInput {
            kind: motion.kind,
            parts: motion
                .parts
                .iter()
                .map(|part| MotionPart {
                    layers: part.layers.iter().copied().rev().collect(),
                    region: part
                        .region
                        .iter()
                        .rev()
                        .map(|half| HalfPlane {
                            line: flip(half.line),
                            inside_point: half.inside_point,
                        })
                        .collect(),
                    transform: match &part.transform {
                        MotionTransform::Reflect(lines) => {
                            MotionTransform::Reflect(lines.iter().copied().map(flip).collect())
                        }
                        other => other.clone(),
                    },
                    turn: part.turn,
                    reverse_layers: part.reverse_layers,
                })
                .collect(),
        }
    }

    /// 部分の並びを入れ替え、証拠の部分番号も同じように付け替える。
    fn with_swapped_parts(
        motion: &FlatMotionInput,
        evidence: &TechniqueEvidence,
    ) -> (FlatMotionInput, TechniqueEvidence) {
        let count = motion.parts.len();
        let renumber = |part: Option<usize>| part.map(|index| count - 1 - index);
        let swapped = FlatMotionInput {
            kind: motion.kind,
            parts: motion.parts.iter().rev().cloned().collect(),
        };
        let moved = TechniqueEvidence {
            stationary_regions: evidence
                .stationary_regions
                .iter()
                .map(|index| count - 1 - index)
                .collect(),
            final_adjacency: evidence
                .final_adjacency
                .iter()
                .map(|pair| CanonicalAdjacency {
                    lower: CanonicalFaceKey {
                        source: pair.lower.source,
                        part: renumber(pair.lower.part),
                    },
                    upper: CanonicalFaceKey {
                        source: pair.upper.source,
                        part: renumber(pair.upper.part),
                    },
                })
                .collect(),
            spine_pairs: evidence.spine_pairs.iter().rev().copied().collect(),
            ..evidence.clone()
        };
        (swapped, moved)
    }

    /// 同じ動きは、繰り返しても、線の端点を入れ替えても、面や部分や領域の順を
    /// 入れ替えても、まったく同じ候補集合と保存値になる。
    #[test]
    fn the_same_move_is_named_the_same_however_it_is_written_down() {
        let cases: Vec<(&str, FlatMotionInput, TechniqueEvidence)> = vec![
            ("段折り", pleat_plan(), pleat_evidence()),
            ("中割り折り", reverse_plan(true), reverse_evidence(true)),
            ("かぶせ折り", reverse_plan(false), reverse_evidence(false)),
            ("開いてつぶす折り", squash_plan(), squash_evidence()),
            ("花弁折り", petal_plan(), petal_evidence()),
            ("沈め折り", open_sink_plan(), open_sink_evidence()),
            ("ひだ寄せ", swivel_plan(), TechniqueEvidence::default()),
            ("ねじり折り", twist_plan(), TechniqueEvidence::default()),
        ];
        for (name, motion, evidence) in cases {
            let candidates = sim011_witnesses(&motion, &evidence);
            assert_eq!(candidates.len(), 1, "{name}: 候補はちょうど1件");
            let saved = stored(&motion, &evidence);
            for _ in 0..8 {
                assert_eq!(
                    sim011_witnesses(&motion, &evidence),
                    candidates,
                    "{name}: 同じ指定を何度読んでも同じ候補集合になる"
                );
            }
            let reversed = with_reversed_lines(&motion);
            assert_eq!(
                sim011_witnesses(&reversed, &evidence),
                candidates,
                "{name}: 線の端点と面・領域の順を入れ替えても候補集合は変わらない"
            );
            assert_eq!(
                stored(&reversed, &evidence),
                saved,
                "{name}: 線の端点を入れ替えても保存値は変わらない"
            );
            let (swapped, renumbered) = with_swapped_parts(&motion, &evidence);
            assert_eq!(
                sim011_witnesses(&swapped, &renumbered),
                candidates,
                "{name}: 部分の並びを入れ替えても候補集合は変わらない"
            );
            assert_eq!(
                stored(&swapped, &renumbered),
                saved,
                "{name}: 部分の並びを入れ替えても保存値は変わらない"
            );
        }
    }

    /// 設計§4が「自動判定しない」とした動きは、似ていても名前を付けない。
    #[test]
    fn variants_that_cannot_be_proved_stay_grabbed_moves() {
        // 1. 層操作(紙を動かさず重なりだけ変える)は残り8技法をすべて表せる。
        let layer_operation = plan(vec![part(
            vec![1],
            Vec::new(),
            MotionTransform::Stay,
            LayerTurn::Outside(Turn::Up),
        )]);
        // 2. つぶし折りのα=0(紙が1mmも動かない退化)。
        let degenerate_squash = plan(vec![MotionPart::restack(
            vec![1],
            LayerTurn::Outside(Turn::Up),
        )]);
        // 3. 外の紙へ固定されたつぶし折り。両側とも二等分線で折り返して内側へ入る
        //    形は中割り折りと構造が同じで、背が開かないので離散の由来が残らない。
        let anchored_squash = plan(vec![
            part(
                vec![1],
                vec![half(BISECTOR, [1.0, 0.0])],
                MotionTransform::Reflect(vec![BISECTOR]),
                LayerTurn::Inside(Turn::Up),
            ),
            part(
                vec![2],
                vec![half(BISECTOR, [1.0, 0.0])],
                MotionTransform::Reflect(vec![BISECTOR]),
                LayerTurn::Inside(Turn::Down),
            ),
        ]);
        // 4. 片翼だけの花弁折り。
        let one_winged_petal = plan(vec![
            part(
                vec![1],
                vec![
                    half(HINGE, [0.0, 0.0]),
                    half(RIGHT_BISECTOR, [0.25, -0.433_012_701_892_219_3]),
                ],
                MotionTransform::Reflect(vec![RIGHT_BISECTOR, HINGE]),
                LayerTurn::Beside {
                    anchor: 7,
                    direction: Turn::Up,
                },
            ),
            part(
                vec![3],
                vec![half(HINGE, [0.0, 0.0]), half(RIGHT_BISECTOR, [0.25, 0.0])],
                MotionTransform::Reflect(vec![HINGE]),
                LayerTurn::Beside {
                    anchor: 7,
                    direction: Turn::Up,
                },
            ),
        ]);
        // 5. 鏡映の列という由来が消えた指定(等長変換を直接渡したひだ寄せ)。
        let lost_provenance = plan(vec![
            part(
                vec![1],
                vec![half(SPINE, [1.0, 0.3]), half(BISECTOR, [1.0, 0.3])],
                MotionTransform::Isometry(line_key(BISECTOR).reflection()),
                LayerTurn::Outside(Turn::Up),
            ),
            part(
                vec![1],
                vec![half(SPINE, [1.0, -0.3])],
                MotionTransform::Isometry(
                    line_key(BISECTOR)
                        .reflection()
                        .compose(&line_key(SPINE).reflection()),
                ),
                LayerTurn::Keep,
            ),
        ]);
        // 6. 退化した線(2点が一致)はどの名前にも丸めない。
        let degenerate_line = plan(vec![part(
            vec![1],
            vec![half([[0.0, 0.0], [0.0, 0.0]], [1.0, 1.0])],
            MotionTransform::Stay,
            LayerTurn::Keep,
        )]);

        // 外の紙へ固定されたつぶし折りが実際に残す由来。二等分線は背の**支点**を
        // 通るだけで背を横切らないので、中割り折りが要る `spine_pairs` が出ない。
        // 背も開かないので `opened_spines` も出ない。名乗れる根拠が1つも無い。
        let anchored_evidence = TechniqueEvidence {
            parent_face_of: BTreeMap::from([(10, 1), (11, 1), (20, 2), (21, 2)]),
            moved_faces: BTreeSet::from([10, 20]),
            tip_faces: BTreeSet::from([1, 2]),
            final_adjacency: stack(&[
                face(1, None),
                face(1, Some(0)),
                face(2, Some(1)),
                face(2, None),
            ]),
            ..TechniqueEvidence::default()
        };
        // 残りの variant は、名乗る根拠をすべて渡しても形が一致しない。
        let generous = TechniqueEvidence {
            pocket_of: BTreeMap::from([(7, 0), (1, 0), (3, 0)]),
            spine_pairs: vec![[1, 2]],
            opened_spines: vec![line_key(SPINE)],
            target_drivers: vec![CanonicalDriver {
                support: line_key(SINK_LINE),
                target_degrees: 180.0,
            }],
            ..anchored_evidence.clone()
        };
        for (name, motion, evidence) in [
            ("層操作", layer_operation, &generous),
            ("つぶし折りのα=0", degenerate_squash, &generous),
            (
                "外の紙へ固定されたつぶし折り",
                anchored_squash,
                &anchored_evidence,
            ),
            ("片翼だけの花弁折り", one_winged_petal, &generous),
            ("鏡映の由来が消えた指定", lost_provenance, &generous),
            ("退化した線", degenerate_line, &generous),
        ] {
            assert!(
                sim011_witnesses(&motion, evidence).is_empty(),
                "{name}: 証明できないので候補を出さない"
            );
            assert_eq!(
                stored(&motion, evidence),
                automatic(DisplayTechniqueKind::GrabMove),
                "{name}: 保存値は「つかんで動かした折り」"
            );
        }
    }

    /// 中割り折りは「折り線が背を横切る」ことを証拠で確かめてから名乗る。
    ///
    /// 外の紙へ固定されたつぶし折りは同じ形の動きになるので、横切った証拠が
    /// 無いままでは名前を付けない。ここを緩めると、つぶし折りに中割り折りという
    /// 嘘の名前が付く。
    #[test]
    fn a_reverse_fold_is_not_named_unless_the_crease_line_crosses_the_spine() {
        let motion = reverse_plan(true);
        let crossing = reverse_evidence(true);
        assert_eq!(
            named(&motion, &crossing),
            AutomaticTechniqueMatch::Unique(DisplayTechniqueKind::InsideReverse)
        );
        let without_crossing = TechniqueEvidence {
            spine_pairs: Vec::new(),
            ..crossing
        };
        assert_eq!(
            named(&motion, &without_crossing),
            AutomaticTechniqueMatch::NoMatch,
            "背を横切った証拠が無ければ中割り折りと名乗らない"
        );
    }

    /// 証拠を取り上げると、証拠を必要とする技法は名前を返さなくなる。
    #[test]
    fn a_technique_that_needs_provenance_is_not_named_without_it() {
        for (name, motion) in [
            ("段折り", pleat_plan()),
            ("中割り折り", reverse_plan(true)),
            ("かぶせ折り", reverse_plan(false)),
            ("開いてつぶす折り", squash_plan()),
            ("花弁折り", petal_plan()),
            ("沈め折り", open_sink_plan()),
        ] {
            assert_eq!(
                named(&motion, &TechniqueEvidence::default()),
                AutomaticTechniqueMatch::NoMatch,
                "{name}: 由来が無ければ名乗らない"
            );
        }
    }
}
