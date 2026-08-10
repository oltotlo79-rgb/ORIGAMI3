//! 汎用の折り操作プリミティブ: 平坦状態から平坦状態への「任意の紙の動き」。
//!
//! 要件定義書の設計原則0(表現の完全性)とSIM-011の実現手段。実際の紙で折れる
//! 動きはすべてこの1つの関数([`flat_motion`])で表せるようにしてある。
//! 名前の付いた技法([`crate::techniques`])も、名前の無い自由な動きも、
//! これを呼ぶマクロとして書ける。
//!
//! # 考え方
//!
//! 畳んだ紙の動きは、数学的には次の2つだけで決まる。
//!
//! 1. **どの紙が、どの等長変換で動くか**(面の配置の変化)
//! 2. **動いた紙が重なりのどこへ入るか**(層順序の変化)
//!
//! 1つの動きを、いくつかの [`MotionPart`](動かす部分)で表す。各部分は
//!
//! - `layers`: 動かす層(現在の面ID)
//! - `region`: 畳み平面での領域(半平面の共通部分。空なら層まるごと)
//! - `transform`: 施す等長変換(鏡映の列/直接指定/動かさない)
//! - `turn`: 動いた紙を重なりのどこへ入れるか
//!
//! を持つ。**領域の境界線がそのまま新しい折り線になる**(各層の面へ引き戻して
//! 展開図へ追加する)。折り線が要らない動き(既存の折り目を開く・重なり順だけ
//! 変える)は `region` を空にすればよい。
//!
//! # 表せる動きの対応(設計フェーズの確認結果)
//!
//! | 紙の動き | 表し方 |
//! |---|---|
//! | 単純折り | 1部分・領域=折り線の可動側・変換=その線での鏡映・`Outside` |
//! | 一部の層だけ折る | 同上で `layers` を絞る(層の数の偶奇は問わない) |
//! | 折り目を開く | 1部分・領域=空・変換=その折り目での鏡映(配置が一致して角度0°になる) |
//! | つぶし折り | 複数部分。奥の紙は鏡映2回=回転になる |
//! | 紙が動かない遷移 | 変換=[`MotionTransform::Stay`]・`turn`で重なりだけ変える |
//! | 中割り・かぶせ | 2部分(同じ鏡映・逆向きの`turn`)。層の分割は紙のつながりから決める |
//! | 花弁折り | 複数部分(開く + 左右をたたむ)の組み合わせ |
//! | 沈め折り | 領域内の層を`Stay`+`reverse_layers`で裏返す(山谷は重なり順から決め直す) |
//! | ひだ寄せ・ねじり | 部分ごとに別々の等長変換を与える |
//!
//! # 断る入力
//!
//! 「止めずに警告」原則により、Errで断るのは**幾何的に定義できない入力だけ**
//! (退化した直線、内側を示す点が線の上にある、平坦状態に配置の無い面、
//! 折り線が面を横切っているのに面を分割できなかった)。紙が裂ける指定・
//! 重なり順と山谷が食い違う指定は、警告を付けてそのまま実行する。
//!
//! # 既知の制限
//!
//! - 領域に掛かるかどうかは面を領域で切り取った面積で判定する
//!   ([`overlaps_region`])。面積が [`REGION_AREA_EPS`] 以下の細い掛かり方は
//!   掛かっていない扱いになる
//! - 重なり順は各部分の `turn` から構成的に決める。物理的に成り立たない
//!   入れ方を指定しても断らない(山谷と重なり順の食い違いは警告になる)

use std::collections::{HashMap, HashSet};

use glam::DVec2;
use ori3_cp::{Face, extract_faces, insert_segment};
use ori3_geometry::{Isometry2, collinear_overlap, dist_point_segment, point_on_segment};
use ori3_model::{
    CreasePattern, DriverLine, EPS, EdgeId, EdgeKind, FaceId, FoldStep, TechniqueKind, VertexId,
};

use crate::flat_state::{FlatState, point_in_face, representative_point};
use crate::fold_through::{
    FoldDirection, FoldThroughResult, TEAR_MARK, angle_of, faces_by_edge, flip_kind,
    normalize_to_root, opposite_crease_warning, push_driver_line, vertex_positions,
};

/// 面のつながり(同じ点に写るか)を見る許容誤差。等長変換の積み重ねと
/// 剛体折りソルバー由来の誤差(1e-7程度)より十分大きく取る。
const JOIN_EPS: f64 = 1e-6;

/// 面が領域に掛かっているとみなす最小面積(紙は1辺1の正方形として扱う)。
const REGION_AREA_EPS: f64 = 1e-12;

/// 畳み平面の半平面。`inside_point` のある側が内側(動かす側)。
#[derive(Clone, Debug)]
pub struct HalfPlane {
    /// 境界線(2点。無限直線として扱う)。この線が新しい折り線になる
    pub line: [[f64; 2]; 2],
    /// 内側(動かす側)を示す点
    pub inside_point: [f64; 2],
}

/// 動かす紙に施す等長変換の指定。座標は畳み平面。
#[derive(Clone, Debug)]
pub enum MotionTransform {
    /// 紙を動かさない(重なり順・山谷だけを変える遷移に使う)
    Stay,
    /// 直線での鏡映を先頭から順に適用する(1本=単純折り、2本=回転)
    Reflect(Vec<[[f64; 2]; 2]>),
    /// 等長変換を直接指定する
    Isometry(Isometry2),
}

/// 動かした紙を重なりのどこへ入れるか。
#[derive(Clone, Copy, Debug)]
pub enum LayerTurn {
    /// 重なり順を変えない
    Keep,
    /// Leave the layer order unchanged while assigning the requested sense to
    /// a crease that is folded and immediately unfolded.
    CreaseOnly(FoldDirection),
    /// 重なり全体のいちばん上(Up)/いちばん下(Down)へ回す(普通の折り)
    Outside(FoldDirection),
    /// 分かれた元の紙のすぐ上(Up)/すぐ下(Down)へ差し込む(中割り・かぶせ)
    Inside(FoldDirection),
    /// 指定した面から分かれた紙のすぐ上(Up)/すぐ下(Down)へ入れる
    /// (重なり順を細かく決めたいとき)。同じ基準面へ続けて置くと、
    /// 先に置いた紙のさらに外側へ積まれる(花弁折りが袋ごとに使う)
    Beside {
        /// 基準にする面(この動きを始める時点の面ID)
        anchor: FaceId,
        direction: FoldDirection,
    },
}

/// 動きの一部分(まとめて同じように動く紙)。
#[derive(Clone, Debug)]
pub struct MotionPart {
    /// 動かす層(この動きを始める時点の面ID)。空なら全ての層
    pub layers: Vec<FaceId>,
    /// 動かす領域(畳み平面の半平面の共通部分)。空なら層まるごと。
    /// 境界線はそのまま新しい折り線になる
    pub region: Vec<HalfPlane>,
    /// 施す等長変換
    pub transform: MotionTransform,
    /// 動いた紙を重なりのどこへ入れるか
    pub turn: LayerTurn,
    /// この部分の中の重なり順を逆にするか。`None` なら自動
    /// (裏返る変換なら逆順、裏返らない変換ならそのまま)
    pub reverse_layers: Option<bool>,
}

impl MotionPart {
    /// 単純な折り: `line` の `movable_point` 側にある紙を折り返し、重なり全体の
    /// 上(Up)/下(Down)へ回す。
    pub fn fold(
        layers: Vec<FaceId>,
        line: [[f64; 2]; 2],
        movable_point: [f64; 2],
        direction: FoldDirection,
    ) -> MotionPart {
        MotionPart {
            layers,
            region: vec![HalfPlane {
                line,
                inside_point: movable_point,
            }],
            transform: MotionTransform::Reflect(vec![line]),
            turn: LayerTurn::Outside(direction),
            reverse_layers: None,
        }
    }

    /// 既存の折り目を開く: `crease` で折られている紙を平らに戻す。
    /// 新しい折り線は引かず、その折り目の角度が0°になる。
    pub fn open(layers: Vec<FaceId>, crease: [[f64; 2]; 2]) -> MotionPart {
        MotionPart {
            layers,
            region: Vec::new(),
            transform: MotionTransform::Reflect(vec![crease]),
            turn: LayerTurn::Keep,
            reverse_layers: None,
        }
    }

    /// Add a crease to the selected flap without changing its final placement
    /// or its position in the layer stack.
    pub fn crease_only(
        layers: Vec<FaceId>,
        line: [[f64; 2]; 2],
        movable_point: [f64; 2],
        direction: FoldDirection,
    ) -> MotionPart {
        MotionPart {
            layers,
            region: vec![HalfPlane {
                line,
                inside_point: movable_point,
            }],
            transform: MotionTransform::Stay,
            turn: LayerTurn::CreaseOnly(direction),
            reverse_layers: Some(false),
        }
    }

    /// 紙を動かさず、重なり順だけを変える。
    pub fn restack(layers: Vec<FaceId>, turn: LayerTurn) -> MotionPart {
        MotionPart {
            layers,
            region: Vec::new(),
            transform: MotionTransform::Stay,
            turn,
            reverse_layers: None,
        }
    }
}

/// [`flat_motion`] の入力。
#[derive(Clone, Debug)]
pub struct FlatMotionInput {
    /// 動かす部分。先に書いたものが優先(同じ紙が複数の部分に当たる場合)
    pub parts: Vec<MotionPart>,
    /// 記録する手順の技法種別
    pub kind: TechniqueKind,
}

/// 平坦状態から平坦状態への任意の紙の動きを適用する。
///
/// 1. 各部分の領域の境界線を、対象の層の面へ引き戻して展開図へ追加する
/// 2. 面を取り直し、各面がどの部分に属するかを代表点で決めて新しい配置を求める
/// 3. 各部分の `turn` から新しい重なり順を組み立てる
/// 4. 折り目の山谷を新しい重なり順に合わせ直し、この動きで角度が変わる折り目を
///    すべて [`DriverLine`] として記録する(開いた折り目は0°で記録する)
/// 5. 紙が裂ける指定を警告する
///
/// 展開図の更新は複製の上で行い、成功したときだけ `cp` へ反映する(原子性)。
pub fn flat_motion(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &FlatMotionInput,
) -> Result<FoldThroughResult, String> {
    let out = run_motion(cp, faces, state, input)?;
    *cp = out.cp;
    Ok(out.result)
}

/// [`flat_motion`] の内部結果。展開図を書き戻す前の状態を返すので、呼び出し側で
/// さらに条件を検査してからErrにできる([`crate::fold_through`] の「再折り」判定)。
pub(crate) struct MotionOutcome {
    pub cp: CreasePattern,
    pub result: FoldThroughResult,
    /// 面の内部を横切って新しい折り線を引いたか、既存の折り筋を駆動したか。
    /// `fold_through` はどちらも有効な折り操作として受理する。
    pub crossed_any: bool,
    /// この動きで折り線へ昇格した既存の補助線断片の数。
    pub promoted_aux_edges: usize,
}

/// 半平面(内側が正になる符号付き距離を持つ)。
struct Plane {
    l0: DVec2,
    u: DVec2,
    sign: f64,
}

impl Plane {
    /// 内側が正の符号付き距離。
    fn signed(&self, q: DVec2) -> f64 {
        self.sign * self.u.perp_dot(q - self.l0)
    }
}

/// 検証済みの [`MotionPart`]。
struct ResolvedPart {
    layers: Vec<FaceId>,
    region: Vec<Plane>,
    iso: Isometry2,
    direction: Option<FoldDirection>,
    turn: LayerTurn,
    reverse: Option<bool>,
}

impl ResolvedPart {
    /// 畳み平面の点が領域の内側(境界を含まない)にあるか。
    fn contains(&self, q: DVec2) -> bool {
        self.region.iter().all(|p| p.signed(q) > EPS)
    }
}

pub(crate) fn run_motion(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &FlatMotionInput,
) -> Result<MotionOutcome, String> {
    let mut warnings: Vec<String> = Vec::new();

    for f in faces {
        if !state.placements.contains_key(&f.id) {
            return Err(format!("面 {} の配置が平坦状態に見つかりません", f.id));
        }
    }
    if input.parts.is_empty() {
        return Err("動かす紙が指定されていません".to_string());
    }

    let vpos = vertex_positions(cp);
    let polygon = |f: &Face| -> Vec<DVec2> {
        f.vertices
            .iter()
            .filter_map(|id| vpos.get(id).copied())
            .collect()
    };

    // 1. 入力の検証と対象層の絞り込み
    let mut parts: Vec<ResolvedPart> = Vec::with_capacity(input.parts.len());
    for spec in &input.parts {
        parts.push(resolve_part(spec, faces, state, &polygon, &mut warnings)?);
    }
    if parts.iter().all(|p| p.layers.is_empty()) {
        return Err("動かす対象の層がありません".to_string());
    }

    // 2. 領域の境界線を各層の面へ引き戻し、展開図へ挿入する
    let mut work = cp.clone();
    let mut added: Vec<EdgeId> = Vec::new();
    let mut drivers: Vec<DriverLine> = Vec::new();
    // 折り線を引いた区間(CP座標)と、そのとき決めた線種。角度は重なり順を決めてから
    // 最終的な線種で付け直すので、ここでは記録だけしておく。
    let mut cut_intervals: Vec<(DVec2, DVec2, EdgeKind)> = Vec::new();
    let mut warned_overlap: HashSet<EdgeId> = HashSet::new();
    let mut crossed_any = false;
    let mut promoted_aux_edges = 0;
    let mut wvpos = vertex_positions(&work);
    for part in &parts {
        for f in faces.iter().filter(|f| part.layers.contains(&f.id)) {
            let pl = state.placements[&f.id];
            let poly = polygon(f);
            if poly.len() < 3 {
                continue;
            }
            let base = match part.direction {
                Some(FoldDirection::Down) => EdgeKind::Mountain,
                _ => EdgeKind::Valley,
            };
            let kind = if pl.mirrored { flip_kind(base) } else { base };
            for (bi, boundary) in part.region.iter().enumerate() {
                let cut = CutLine::pull_back(&pl, boundary);
                for (q0, q1) in cut.intervals(&poly, part, bi, &pl) {
                    let mid = (q0 + q1) * 0.5;
                    if !point_in_face(cp, f, [mid.x, mid.y]) {
                        continue;
                    }
                    let n = poly.len();
                    let on_boundary =
                        (0..n).any(|i| dist_point_segment(mid, poly[i], poly[(i + 1) % n]) <= EPS);
                    if on_boundary {
                        // 面の縁に沿う区間は面を横切らないので線は引かない。既存の
                        // 折り目に沿っている場合だけ、再生でその断片群を駆動できる
                        // ようDriverLineを作る(角度は既存の線種に従う)。
                        let found = work.edges.iter().find_map(|e| {
                            if !matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley) {
                                return None;
                            }
                            let (p0, p1) = (wvpos.get(&e.v0)?, wvpos.get(&e.v1)?);
                            let (o0, o1) = collinear_overlap(q0, q1, *p0, *p1)?;
                            ((o1 - o0).length() > EPS).then_some((e.id, e.kind))
                        });
                        if let Some((eid, k)) = found {
                            // 面を新しく分割しなくても、境界にある既存の折り筋は
                            // この動きのヒンジになる。区間をDriverLineへ残すだけでなく、
                            // fold_throughの「有効な折り線」判定にも含める。
                            crossed_any = true;
                            cut_intervals.push((q0, q1, k));
                            if k != kind && warned_overlap.insert(eid) {
                                warnings.push(opposite_crease_warning(eid));
                            }
                        }
                        continue;
                    }
                    crossed_any = true;
                    cut_intervals.push((q0, q1, kind));
                    let mut has_aux_overlap = false;
                    for e in &work.edges {
                        let (Some(&p0), Some(&p1)) = (wvpos.get(&e.v0), wvpos.get(&e.v1)) else {
                            continue;
                        };
                        let Some((o0, o1)) = collinear_overlap(q0, q1, p0, p1) else {
                            continue;
                        };
                        if (o1 - o0).length() <= EPS {
                            continue;
                        }
                        if e.kind == EdgeKind::Aux {
                            has_aux_overlap = true;
                        } else if e.kind != kind && warned_overlap.insert(e.id) {
                            warnings.push(opposite_crease_warning(e.id));
                        }
                    }
                    added.extend(insert_segment(&mut work, [q0.x, q0.y], [q1.x, q1.y], kind));
                    wvpos = vertex_positions(&work);
                    if has_aux_overlap {
                        for e in work.edges.iter_mut() {
                            if e.kind == EdgeKind::Aux
                                && let (Some(&p0), Some(&p1)) = (wvpos.get(&e.v0), wvpos.get(&e.v1))
                                && (p1 - p0).length() >= EPS
                                && point_on_segment(p0, q0, q1)
                                && point_on_segment(p1, q0, q1)
                            {
                                e.kind = kind;
                                added.push(e.id);
                                promoted_aux_edges += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    added.sort_unstable();
    added.dedup();
    added.retain(|id| work.edges.iter().any(|e| e.id == *id));

    // 3. 面を取り直し、親面・所属する部分・新しい配置を決める
    let new_faces = extract_faces(&work);
    let wpos = wvpos;
    let old_rank: HashMap<FaceId, usize> = state
        .order
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, i))
        .collect();

    let mut parent_of: HashMap<FaceId, FaceId> = HashMap::with_capacity(new_faces.len());
    let mut part_of: HashMap<FaceId, usize> = HashMap::new();
    let mut placements: HashMap<FaceId, Isometry2> = HashMap::with_capacity(new_faces.len());
    for nf in &new_faces {
        let r = representative_point(&work, nf);
        let Some(pf) = faces.iter().find(|f| point_in_face(cp, f, r)) else {
            warnings.push(format!(
                "新しい面 {} の親面が特定できないため、動かさず元の配置のままにします",
                nf.id
            ));
            placements.insert(nf.id, Isometry2::identity());
            continue;
        };
        parent_of.insert(nf.id, pf.id);
        let ppl = state.placements[&pf.id];
        let q = ppl.apply(DVec2::from(r));
        // 防御: 領域の境界が面を横切っているのに面が分かれていない場合、
        // このまま進めると面全体が誤って動く。領域の一部にだけ掛かる面
        // (=領域で切ると面積が減る面)がそれにあたる。
        let poly: Vec<DVec2> = nf
            .vertices
            .iter()
            .filter_map(|vid| wpos.get(vid))
            .map(|&p| ppl.apply(p))
            .collect();
        for part in parts.iter().filter(|p| p.layers.contains(&pf.id)) {
            let all_in = poly
                .iter()
                .all(|&q| part.region.iter().all(|plane| plane.signed(q) >= -EPS));
            if !all_in && overlaps_region(&poly, &part.region) {
                return Err(
                    "折り線が面を横切っているのに面を分割できませんでした。折り線と重なる線の近くの展開図を確認してください"
                        .to_string(),
                );
            }
        }
        let hit = parts
            .iter()
            .position(|p| p.layers.contains(&pf.id) && p.contains(q));
        match hit {
            Some(i) => {
                part_of.insert(nf.id, i);
                placements.insert(nf.id, parts[i].iso.compose(&ppl));
            }
            None => {
                placements.insert(nf.id, ppl);
            }
        }
    }

    // 4. 新しい重なり順(下→上)を各部分の turn から組み立てる
    let order = build_order(&new_faces, &parts, &parent_of, &part_of, &old_rank);

    // 5. 表示座標系(根面=最小面IDが恒等)へそろえる
    let (placements, order) = normalize_to_root(placements, order);

    // 6. 折り目の山谷を新しい重なり順に合わせ直し、角度が変わる折り目を記録する
    let moved: HashSet<FaceId> = part_of.keys().copied().collect();
    let angles = settle_creases(
        &mut work,
        &new_faces,
        &wpos,
        state,
        &parent_of,
        &placements,
        &order,
        &old_rank,
        &added,
        &mut drivers,
    );
    // 引いた折り線の区間へ、決まった線種に合わせて角度を付ける
    for (q0, q1, kind) in cut_intervals {
        record_cut_driver(&work, &angles, q0, q1, kind, &mut drivers);
    }

    // 7. 紙が裂ける指定の検出
    warnings.extend(tear_warnings(
        &work,
        &new_faces,
        &wpos,
        &placements,
        &moved,
        &parent_of,
    ));

    let new_state = FlatState { placements, order };
    let layer_points = new_state.to_layer_points(&work, &new_faces);
    let step = FoldStep {
        id: 0,
        kind: input.kind,
        drivers,
        layer_order: Some(layer_points),
        alignment: None,
        note: String::new(),
    };
    Ok(MotionOutcome {
        cp: work,
        result: FoldThroughResult {
            state: new_state,
            added_edges: added,
            step,
            warnings,
        },
        crossed_any,
        promoted_aux_edges,
    })
}

// ---------------------------------------------------------------------------
// 入力の検証
// ---------------------------------------------------------------------------

fn line_dir(line: [[f64; 2]; 2]) -> Result<(DVec2, DVec2), String> {
    let a = DVec2::from(line[0]);
    let b = DVec2::from(line[1]);
    if (b - a).length() < EPS {
        return Err("折り線の2点が一致しています".to_string());
    }
    Ok((a, (b - a).normalize()))
}

fn resolve_part(
    spec: &MotionPart,
    faces: &[Face],
    state: &FlatState,
    polygon: &impl Fn(&Face) -> Vec<DVec2>,
    warnings: &mut Vec<String>,
) -> Result<ResolvedPart, String> {
    let mut region: Vec<Plane> = Vec::with_capacity(spec.region.len());
    for h in &spec.region {
        let (l0, u) = line_dir(h.line)?;
        let s = u.perp_dot(DVec2::from(h.inside_point) - l0);
        if s.abs() <= EPS {
            return Err("動かす側を示す点が折り線上にあります".to_string());
        }
        region.push(Plane {
            l0,
            u,
            sign: s.signum(),
        });
    }

    let iso = match &spec.transform {
        MotionTransform::Stay => Isometry2::identity(),
        MotionTransform::Isometry(m) => *m,
        MotionTransform::Reflect(lines) => {
            let mut acc = Isometry2::identity();
            for l in lines {
                let (a, u) = line_dir(*l)?;
                acc = Isometry2::reflection(a, a + u).compose(&acc);
            }
            acc
        }
    };

    // 対象の層。指定が空なら全ての面。
    let listed: Vec<FaceId> = if spec.layers.is_empty() {
        faces.iter().map(|f| f.id).collect()
    } else {
        let mut out: Vec<FaceId> = Vec::with_capacity(spec.layers.len());
        for &id in &spec.layers {
            if out.contains(&id) {
                continue;
            }
            if faces.iter().any(|f| f.id == id) {
                out.push(id);
            } else {
                warnings.push(format!(
                    "対象層 {id} は現在の面に存在しないため除外しました"
                ));
            }
        }
        out
    };

    // 領域に掛からない面は除く(切り取っても面積が残らない面)。
    let mut layers: Vec<FaceId> = Vec::with_capacity(listed.len());
    for id in listed {
        let f = faces.iter().find(|f| f.id == id).expect("検証済みの面ID");
        let pl = state.placements[&id];
        let poly: Vec<DVec2> = polygon(f).into_iter().map(|p| pl.apply(p)).collect();
        let hit = overlaps_region(&poly, &region);
        if hit {
            layers.push(id);
        } else if !spec.layers.is_empty() {
            warnings.push(format!(
                "対象層 {id} は折り線の可動側に掛かっていないため除外しました"
            ));
        }
    }

    let direction = match spec.turn {
        LayerTurn::Keep => None,
        LayerTurn::CreaseOnly(direction) => Some(direction),
        LayerTurn::Outside(d) | LayerTurn::Inside(d) => Some(d),
        LayerTurn::Beside { direction, .. } => Some(direction),
    };
    Ok(ResolvedPart {
        layers,
        region,
        iso,
        direction,
        turn: spec.turn,
        reverse: spec.reverse_layers,
    })
}

/// 面(畳み平面の多角形)が領域(凸=半平面の共通部分)に掛かるか。
///
/// 多角形を半平面で順に切り取り(Sutherland–Hodgman)、面積が残るかで判定する。
/// 頂点が内側にあるかだけを見ると、3本以上の半平面で囲んだ細い領域(花弁折りの
/// 先端の三角形など、頂点が全て領域の境界に乗る形)を取りこぼす。
fn overlaps_region(poly: &[DVec2], region: &[Plane]) -> bool {
    if region.is_empty() {
        return true;
    }
    let mut cur: Vec<DVec2> = poly.to_vec();
    for plane in region {
        if cur.len() < 3 {
            return false;
        }
        let mut next: Vec<DVec2> = Vec::with_capacity(cur.len() + 1);
        for i in 0..cur.len() {
            let (a, b) = (cur[i], cur[(i + 1) % cur.len()]);
            let (da, db) = (plane.signed(a), plane.signed(b));
            if da >= 0.0 {
                next.push(a);
            }
            if (da > 0.0 && db < 0.0) || (da < 0.0 && db > 0.0) {
                next.push(a + (b - a) * (da / (da - db)));
            }
        }
        cur = next;
    }
    if cur.len() < 3 {
        return false;
    }
    let area: f64 = (0..cur.len())
        .map(|i| cur[i].perp_dot(cur[(i + 1) % cur.len()]))
        .sum::<f64>()
        * 0.5;
    area.abs() > REGION_AREA_EPS
}

// ---------------------------------------------------------------------------
// 折り線の引き戻しと区間の切り出し
// ---------------------------------------------------------------------------

/// 領域の境界線を1つの面のCP座標へ引き戻した直線。
struct CutLine {
    a: DVec2,
    dir: DVec2,
}

impl CutLine {
    fn pull_back(pl: &Isometry2, boundary: &Plane) -> CutLine {
        let inv = pl.inverse();
        let a = inv.apply(boundary.l0);
        let b = inv.apply(boundary.l0 + boundary.u);
        CutLine {
            a,
            dir: (b - a).normalize(),
        }
    }

    /// 面の境界との交点(と、領域の他の境界線との交点)で区切った区間を返す。
    /// 領域の他の半平面から外れる区間は捨てる。
    fn intervals(
        &self,
        poly: &[DVec2],
        part: &ResolvedPart,
        self_index: usize,
        pl: &Isometry2,
    ) -> Vec<(DVec2, DVec2)> {
        let n = poly.len();
        let t_of = |q: DVec2| (q - self.a).dot(self.dir);
        let mut ts: Vec<f64> = Vec::new();
        for i in 0..n {
            let p0 = poly[i];
            let p1 = poly[(i + 1) % n];
            let s0 = self.dir.perp_dot(p0 - self.a);
            let s1 = self.dir.perp_dot(p1 - self.a);
            if s0.abs() <= EPS && s1.abs() <= EPS {
                ts.push(t_of(p0));
                ts.push(t_of(p1));
            } else if s0.abs() <= EPS {
                ts.push(t_of(p0));
            } else if s1.abs() <= EPS {
                ts.push(t_of(p1));
            } else if s0 * s1 < 0.0 {
                ts.push(t_of(p0 + (p1 - p0) * (s0 / (s0 - s1))));
            }
        }
        // 領域の他の境界線との交点でも区切る(凸領域の角で折り線を止めるため)。
        for (j, other) in part.region.iter().enumerate() {
            if j == self_index {
                continue;
            }
            let o = CutLine::pull_back(pl, other);
            let denom = self.dir.perp_dot(o.dir);
            if denom.abs() <= EPS {
                continue;
            }
            let t = (o.a - self.a).perp_dot(o.dir) / denom;
            ts.push(t);
        }
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|x, y| (*x - *y).abs() <= EPS);

        let mut out = Vec::new();
        for w in ts.windows(2) {
            let (t0, t1) = (w[0], w[1]);
            if t1 - t0 <= EPS {
                continue;
            }
            let mid = self.a + self.dir * (0.5 * (t0 + t1));
            // 領域の他の半平面から外れる区間は折り線にしない
            let midf = pl.apply(mid);
            if part
                .region
                .iter()
                .enumerate()
                .any(|(j, q)| j != self_index && q.signed(midf) < -EPS)
            {
                continue;
            }
            out.push((self.a + self.dir * t0, self.a + self.dir * t1));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// 新しい重なり順
// ---------------------------------------------------------------------------

/// 各部分の `turn` から新しい層順序(下→上)を組み立てる。
///
/// 出発点は「元の重なり順を、分かれた面へそのまま細かくしたもの」。そこから
/// 部分ごとに面を抜き出し、指定された入れ方で戻す。層の枚数や偶奇の仮定は置かない。
fn build_order(
    new_faces: &[Face],
    parts: &[ResolvedPart],
    parent_of: &HashMap<FaceId, FaceId>,
    part_of: &HashMap<FaceId, usize>,
    old_rank: &HashMap<FaceId, usize>,
) -> Vec<FaceId> {
    let key = |id: FaceId| -> (usize, FaceId) {
        let rank = parent_of
            .get(&id)
            .and_then(|p| old_rank.get(p))
            .copied()
            .unwrap_or(usize::MAX);
        (rank, id)
    };
    let mut order: Vec<FaceId> = new_faces.iter().map(|f| f.id).collect();
    order.sort_by_key(|&id| key(id));

    // 面のつながり(相手が見つからない先端の置き場所を探すのに使う)
    let mut neighbors: HashMap<FaceId, Vec<FaceId>> = HashMap::new();
    for (_, fs) in faces_by_edge(new_faces) {
        if fs.len() == 2 {
            neighbors.entry(fs[0]).or_default().push(fs[1]);
            neighbors.entry(fs[1]).or_default().push(fs[0]);
        }
    }

    // 既に `Beside` で置いた紙(面→基準面)。同じ基準面へ続けて置くとき、
    // 先に置いた紙の外側へ重なっていく(1つの袋の中で紙が順に積まれる)。
    let mut beside_of: HashMap<FaceId, FaceId> = HashMap::new();

    for (i, part) in parts.iter().enumerate() {
        let mut block: Vec<FaceId> = order
            .iter()
            .copied()
            .filter(|id| part_of.get(id) == Some(&i))
            .collect();
        if block.is_empty() {
            continue;
        }
        if part.reverse.unwrap_or(part.iso.mirrored) {
            block.reverse();
        }
        match part.turn {
            LayerTurn::Keep | LayerTurn::CreaseOnly(_) => {
                // 位置は変えず、部分の中の並びだけ差し替える
                let slots: Vec<usize> = order
                    .iter()
                    .enumerate()
                    .filter(|(_, id)| part_of.get(id) == Some(&i))
                    .map(|(k, _)| k)
                    .collect();
                for (k, &slot) in slots.iter().enumerate() {
                    order[slot] = block[k];
                }
            }
            LayerTurn::Outside(dir) => {
                order.retain(|id| part_of.get(id) != Some(&i));
                match dir {
                    FoldDirection::Up => order.extend(block),
                    FoldDirection::Down => {
                        block.extend(order.iter().copied());
                        order = block;
                    }
                }
            }
            LayerTurn::Inside(dir) => {
                order.retain(|id| part_of.get(id) != Some(&i));
                let mut placed: HashSet<FaceId> = HashSet::new();
                let mut out: Vec<FaceId> = Vec::with_capacity(order.len() + block.len());
                for &id in &order {
                    let mine: Vec<FaceId> = block
                        .iter()
                        .copied()
                        .filter(|m| {
                            !placed.contains(m)
                                && anchor_of(*m, id, parent_of, &neighbors, part_of, i)
                        })
                        .collect();
                    placed.extend(mine.iter().copied());
                    match dir {
                        FoldDirection::Up => {
                            out.push(id);
                            out.extend(mine);
                        }
                        FoldDirection::Down => {
                            out.extend(mine);
                            out.push(id);
                        }
                    }
                }
                for &m in &block {
                    if !placed.contains(&m) {
                        match dir {
                            FoldDirection::Up => out.push(m),
                            FoldDirection::Down => out.insert(0, m),
                        }
                    }
                }
                order = out;
            }
            LayerTurn::Beside { anchor, direction } => {
                order.retain(|id| part_of.get(id) != Some(&i));
                let slots: Vec<usize> = order
                    .iter()
                    .enumerate()
                    .filter(|(_, id)| {
                        parent_of.get(id) == Some(&anchor) || beside_of.get(id) == Some(&anchor)
                    })
                    .map(|(k, _)| k)
                    .collect();
                let at = match direction {
                    FoldDirection::Up => slots.last().map(|k| k + 1),
                    FoldDirection::Down => slots.first().copied(),
                };
                for &m in &block {
                    beside_of.insert(m, anchor);
                }
                match at {
                    Some(k) => {
                        order.splice(k..k, block);
                    }
                    // 基準面の紙が全部動いて残りが無いときも、置く側は direction で決まる
                    // (向こう側=Down なら重なりの下、手前側=Up なら上へ入る)
                    None => match direction {
                        FoldDirection::Up => order.extend(block),
                        FoldDirection::Down => {
                            block.extend(order.iter().copied());
                            order = block;
                        }
                    },
                }
            }
        }
    }
    order
}

/// 動いた面 `m` の置き場所が `host` かどうか。
/// まず「同じ面から分かれた残り」、無ければ「折り目でつながっている動かない面」。
fn anchor_of(
    m: FaceId,
    host: FaceId,
    parent_of: &HashMap<FaceId, FaceId>,
    neighbors: &HashMap<FaceId, Vec<FaceId>>,
    part_of: &HashMap<FaceId, usize>,
    part: usize,
) -> bool {
    let Some(&mine) = parent_of.get(&m) else {
        return false;
    };
    if parent_of.get(&host) == Some(&mine) {
        return true;
    }
    // 同じ面から分かれた残りが1つも無いときだけ、折り目でつながっている面を place 先にする
    let has_sibling = parent_of
        .iter()
        .any(|(&x, &p)| x != m && p == mine && part_of.get(&x) != Some(&part));
    if has_sibling {
        return false;
    }
    neighbors
        .get(&m)
        .is_some_and(|ns| ns.contains(&host) && part_of.get(&host) != Some(&part))
}

// ---------------------------------------------------------------------------
// 山谷の決め直しと手順の記録
// ---------------------------------------------------------------------------

/// 折り目でつながった2面の上下から、その折り目のあるべき線種を求める。
/// 表向き(mirroredでない)の面から見て、相手が上なら谷・下なら山。
fn want_kind(rank_a: usize, rank_b: usize, a_mirrored: bool) -> EdgeKind {
    if (rank_b > rank_a) == a_mirrored {
        EdgeKind::Mountain
    } else {
        EdgeKind::Valley
    }
}

/// 折り目の山谷を新しい重なり順に合わせ直し、角度の変わる折り目をDriverLineへ記録する。
/// 戻り値は「折り目(2面が共有する辺)ごとの、この動きのあとの角度」。
///
/// 動きの前からあった折り目に触るのは「動く前も後も紙がつながっている」場合だけ。
/// 裂けている(技法が複数回の折りを重ねる途中の)折り目に触ると山谷が壊れるため。
/// この動きで新しく引いた折り線は、重なり順から求めた山谷へそろえる
/// (折る向きだけでは、回転を含む動きで正しい山谷にならない)。
#[allow(clippy::too_many_arguments)]
fn settle_creases(
    work: &mut CreasePattern,
    new_faces: &[Face],
    wpos: &HashMap<VertexId, DVec2>,
    state: &FlatState,
    parent_of: &HashMap<FaceId, FaceId>,
    placements: &HashMap<FaceId, Isometry2>,
    order: &[FaceId],
    old_rank: &HashMap<FaceId, usize>,
    added: &[EdgeId],
    drivers: &mut Vec<DriverLine>,
) -> HashMap<EdgeId, f64> {
    let rank: HashMap<FaceId, usize> = order.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    // (辺ID, 新しい線種, 手順へ記録する角度)
    let mut fixes: Vec<(EdgeId, Option<EdgeKind>, Option<f64>)> = Vec::new();
    let mut angles: HashMap<EdgeId, f64> = HashMap::new();
    for (eid, fs) in faces_by_edge(new_faces) {
        if fs.len() != 2 {
            continue;
        }
        let (a, b) = (fs[0], fs[1]);
        let (Some(&pa), Some(&pb)) = (parent_of.get(&a), parent_of.get(&b)) else {
            continue;
        };
        let Some(e) = work.edges.iter().find(|e| e.id == eid) else {
            continue;
        };
        if !matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let (Some(&v0), Some(&v1)) = (wpos.get(&e.v0), wpos.get(&e.v1)) else {
            continue;
        };
        let (npa, npb) = (placements[&a], placements[&b]);
        let folded = npa.mirrored != npb.mirrored;
        angles.insert(eid, if folded { angle_of(e.kind) } else { 0.0 });
        if !joined(&npa, &npb, v0, v1) {
            continue; // 裂けている折り目は山谷を決められない
        }
        let (Some(&nra), Some(&nrb)) = (rank.get(&a), rank.get(&b)) else {
            continue;
        };
        let want = want_kind(nra, nrb, npa.mirrored);

        if pa == pb {
            // この動きで新しく引いた折り線。重なり順に合わせた山谷にそろえる
            // (もとからあった線に重ねただけの辺は書き換えない)。
            if folded && e.kind != want && added.contains(&eid) {
                fixes.push((eid, Some(want), None));
                angles.insert(eid, angle_of(want));
            }
            continue;
        }

        let (opa, opb) = (state.placements[&pa], state.placements[&pb]);
        if !joined(&opa, &opb, v0, v1) {
            continue;
        }
        let (Some(&ora), Some(&orb)) = (old_rank.get(&pa), old_rank.get(&pb)) else {
            continue;
        };
        let was_folded = opa.mirrored != opb.mirrored;
        if !folded {
            // 平らに開いた: 線種はそのままにして角度0°で記録する
            if was_folded {
                fixes.push((eid, None, Some(0.0)));
            }
            continue;
        }
        if !was_folded {
            fixes.push((eid, Some(want), Some(angle_of(want))));
            angles.insert(eid, angle_of(want));
            continue;
        }
        let before = want_kind(ora, orb, opa.mirrored);
        if before != want {
            // 重なり順か向きが変わったので山谷が入れ替わる。もとの線種が重なり順と
            // 食い違っていた場合もそのずれを保つよう、反転で書き換える。
            let now = flip_kind(e.kind);
            fixes.push((eid, Some(now), Some(angle_of(now))));
            angles.insert(eid, angle_of(now));
        }
    }
    for (eid, kind, angle) in fixes {
        let Some(e) = work.edges.iter_mut().find(|e| e.id == eid) else {
            continue;
        };
        if let Some(k) = kind {
            e.kind = k;
        }
        let (v0, v1) = (e.v0, e.v1);
        let (Some(&a), Some(&b)) = (wpos.get(&v0), wpos.get(&v1)) else {
            continue;
        };
        if let Some(deg) = angle {
            push_driver_line(drivers, a, b, deg);
        }
    }
    angles
}

/// 引いた折り線の区間へDriverLineを付ける。角度は最終的な線種([`settle_creases`]が
/// 決めたもの)に従う。区間の中で線種が食い違う場合だけ辺ごとに分けて記録する。
fn record_cut_driver(
    work: &CreasePattern,
    angles: &HashMap<EdgeId, f64>,
    q0: DVec2,
    q1: DVec2,
    fallback: EdgeKind,
    drivers: &mut Vec<DriverLine>,
) {
    let pos = vertex_positions(work);
    let on: Vec<(DVec2, DVec2, f64)> = work
        .edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .filter_map(|e| {
            let (&p0, &p1) = (pos.get(&e.v0)?, pos.get(&e.v1)?);
            ((p1 - p0).length() >= EPS
                && point_on_segment(p0, q0, q1)
                && point_on_segment(p1, q0, q1))
            .then(|| {
                (
                    p0,
                    p1,
                    angles
                        .get(&e.id)
                        .copied()
                        .unwrap_or_else(|| angle_of(e.kind)),
                )
            })
        })
        .collect();
    let Some(&(_, _, first)) = on.first() else {
        push_driver_line(drivers, q0, q1, angle_of(fallback));
        return;
    };
    if on.iter().all(|&(_, _, a)| a == first) {
        push_driver_line(drivers, q0, q1, first);
    } else {
        for (p0, p1, a) in on {
            push_driver_line(drivers, p0, p1, a);
        }
    }
}

/// 2つの配置が辺の両端点を同じ場所へ写すか(紙がつながっているか)。
fn joined(pa: &Isometry2, pb: &Isometry2, v0: DVec2, v1: DVec2) -> bool {
    (pa.apply(v0) - pb.apply(v0)).length() <= JOIN_EPS
        && (pa.apply(v1) - pb.apply(v1)).length() <= JOIN_EPS
}

/// 動いた紙とその隣の紙のつながりが切れている折り目の警告。
fn tear_warnings(
    work: &CreasePattern,
    new_faces: &[Face],
    wpos: &HashMap<VertexId, DVec2>,
    placements: &HashMap<FaceId, Isometry2>,
    moved: &HashSet<FaceId>,
    parent_of: &HashMap<FaceId, FaceId>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (eid, fs) in faces_by_edge(new_faces) {
        if fs.len() != 2 {
            continue;
        }
        let (a, b) = (fs[0], fs[1]);
        if !moved.contains(&a) && !moved.contains(&b) {
            continue;
        }
        if parent_of.get(&a).is_none() || parent_of.get(&b).is_none() {
            continue;
        }
        let Some(e) = work.edges.iter().find(|e| e.id == eid) else {
            continue;
        };
        let (Some(&v0), Some(&v1)) = (wpos.get(&e.v0), wpos.get(&e.v1)) else {
            continue;
        };
        if !joined(&placements[&a], &placements[&b], v0, v1) {
            out.push(format!(
                "折り目(辺ID {eid})の両側の紙が離れているため、このままでは{TEAR_MARK}(指定のまま続行します)"
            ));
        }
    }
    out
}
