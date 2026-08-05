//! 基本技法のマクロ(段折り・中割り折り・かぶせ折り)のテスト。
//!
//! 検証の方針:
//! - 形と層: 面の数・畳んだ位置・層順序
//! - 展開図: 追加された折り線の本数と山谷(裏返った部分の入れ替えを含む)
//! - 手順: FoldStepのdriversが技法の生成した折り線を全て含み、再生で同じ形に戻る
//! - 表示: 折り終わる直前(t=0.99)の高さから読んだ本当の重なりと層順序が一致する
//!   (`display_order.rs` と同じ考え方。技法では動いた層が山の途中に入るため、
//!   端にまとまるかではなく「重なった面どうしの上下」を1組ずつ突き合わせる)
//! - 折り目の向きと層順序の一致: 折り目でつながった2面の上下は、その折り目を表から
//!   見たときの山谷で決まる(谷なら相手は上、山なら下)。全ての折り目についてこれを
//!   確かめる([`assert_fold_senses`])
//!
//! 中割り折り・かぶせ折りに t=0.99 の高さ読みを使えない理由:
//! この2つは「フラップを開いて先端を層の間へ入れる/外へ回す」動きだが、
//! 開くにはフラップの背をいったん平らに戻す必要がある。手順再生は前の手順の
//! 折り目を180°に固定したまま補間するので、先端は紙全体の上か下を大回りして
//! 目的の位置へ入る。折り上がり(t=1)の形と重なりは正しいが、途中の高さからは
//! 「内側に入ったか外側を包んだか」を読めない。そこで折り目の向きとの一致
//! ([`assert_fold_senses`])で確かめる。

use std::collections::HashMap;

use glam::{DVec2, DVec3};
use ori3_cp::{Face, extract_faces};
use ori3_layers::flat_state::representative_point;
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, fold_through};
use ori3_layers::techniques::TechniqueInput;
use ori3_layers::{flat_state_at, inside_reverse, outside_reverse, pleat, replay};
use ori3_model::{CreasePattern, Document, EdgeKind, FaceId, Paper, TechniqueKind};

/// 技法1回ぶんの結果(検証しやすいようにまとめた)。
#[derive(Debug)]
struct Applied {
    faces: Vec<Face>,
    /// 面ID → 展開図上の代表点
    rep: HashMap<FaceId, [f64; 2]>,
    /// 面ID → 畳んだ平面での代表点の位置
    at: HashMap<FaceId, DVec2>,
    /// 層順序(下→上)
    order: Vec<FaceId>,
    warnings: Vec<String>,
}

fn square_doc() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

/// store.rs の SeqOp::FoldThrough と同じ手順で1手折る(下ごしらえ用)。
fn fold(doc: &mut Document, line: [[f64; 2]; 2], keep: [f64; 2], direction: FoldDirection) {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, _) = flat_state_at(doc, &faces, up_to).expect("平らな状態から折る");
    let mut cp = doc.cp.clone();
    let res = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line,
            keep_side_point: keep,
            target_layers: None,
            direction,
        },
    )
    .expect("折れる指定");
    let mut step = res.step;
    step.id = u32::try_from(doc.sequence.len()).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);
}

type Technique = fn(
    &mut CreasePattern,
    &[Face],
    &ori3_layers::FlatState,
    &TechniqueInput,
) -> Result<ori3_layers::FoldThroughResult, String>;

/// store.rs の SeqOp::Technique と同じ手順で技法を1回適用する。
fn apply(
    doc: &mut Document,
    technique: Technique,
    flap: Vec<FaceId>,
    line: [[f64; 2]; 2],
    reference_point: [f64; 2],
) -> Result<Applied, String> {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, _) = flat_state_at(doc, &faces, up_to)?;
    let mut cp = doc.cp.clone();
    let res = technique(
        &mut cp,
        &faces,
        &state,
        &TechniqueInput {
            flap,
            line,
            reference_point,
        },
    )?;
    let mut step = res.step;
    step.id = u32::try_from(up_to).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);

    let faces = extract_faces(&doc.cp);
    let rep: HashMap<FaceId, [f64; 2]> = faces
        .iter()
        .map(|f| (f.id, representative_point(&doc.cp, f)))
        .collect();
    let at: HashMap<FaceId, DVec2> = faces
        .iter()
        .map(|f| (f.id, res.state.placements[&f.id].apply(DVec2::from(rep[&f.id]))))
        .collect();
    Ok(Applied {
        faces,
        rep,
        at,
        order: res.state.order,
        warnings: res.warnings,
    })
}

/// 展開図の座標から面を選ぶ(代表点の位置で見分ける)。
fn face_with_rep(a: &Applied, pred: impl Fn([f64; 2]) -> bool) -> Vec<FaceId> {
    let mut out: Vec<FaceId> = a
        .faces
        .iter()
        .map(|f| f.id)
        .filter(|id| pred(a.rep[id]))
        .collect();
    out.sort_unstable();
    out
}

fn edge_kind_at(cp: &CreasePattern, mid: [f64; 2]) -> Option<EdgeKind> {
    let pos: HashMap<u32, DVec2> = cp.vertices.iter().map(|v| (v.id, DVec2::from(v.pos))).collect();
    let m = DVec2::from(mid);
    cp.edges
        .iter()
        .find(|e| {
            let (Some(&a), Some(&b)) = (pos.get(&e.v0), pos.get(&e.v1)) else {
                return false;
            };
            ((a + b) * 0.5 - m).length() < 1e-9
        })
        .map(|e| e.kind)
}

/// 層順序での位置(下から0)。
fn layer_of(a: &Applied, id: FaceId) -> usize {
    a.order.iter().position(|&f| f == id).expect("層順序に面がある")
}

// ---------------------------------------------------------------------------
// 表示上の重なり順の検証(t=0.99の高さから読み取る)
// ---------------------------------------------------------------------------

/// 面の「展開図の点 → 折り終わる直前(t=0.99)の3D位置」を与える剛体変換。
/// 面は剛体なので、3頂点の対応からアフィン写像として復元できる。
struct FaceMap {
    p0: DVec2,
    e1: DVec2,
    e2: DVec2,
    q0: DVec3,
    f1: DVec3,
    f2: DVec3,
    det: f64,
}

impl FaceMap {
    fn new(cp: &CreasePattern, face: &Face, polygon: &[[f64; 3]]) -> Option<FaceMap> {
        let pos: HashMap<u32, DVec2> =
            cp.vertices.iter().map(|v| (v.id, DVec2::from(v.pos))).collect();
        let pts: Vec<DVec2> = face.vertices.iter().filter_map(|v| pos.get(v).copied()).collect();
        if pts.len() != face.vertices.len() || pts.len() != polygon.len() || pts.len() < 3 {
            return None;
        }
        let p0 = pts[0];
        let q0 = DVec3::from(polygon[0]);
        // 1点目から見て一次独立になる2点を選ぶ
        for i in 1..pts.len() {
            for j in (i + 1)..pts.len() {
                let e1 = pts[i] - p0;
                let e2 = pts[j] - p0;
                let det = e1.perp_dot(e2);
                if det.abs() > 1e-6 {
                    return Some(FaceMap {
                        p0,
                        e1,
                        e2,
                        q0,
                        f1: DVec3::from(polygon[i]) - q0,
                        f2: DVec3::from(polygon[j]) - q0,
                        det,
                    });
                }
            }
        }
        None
    }

    /// 展開図の点に対応する3D位置の高さ(z)。
    fn height(&self, p: DVec2) -> f64 {
        let d = p - self.p0;
        let a = d.perp_dot(self.e2) / self.det;
        let b = self.e1.perp_dot(d) / self.det;
        (self.q0 + self.f1 * a + self.f2 * b).z
    }
}

/// 多角形の内部(境界からeps以内を含む)に点があるか。
fn inside_polygon(poly: &[DVec2], p: DVec2, eps: f64) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let ab = b - a;
        let t = if ab.length_squared() == 0.0 {
            0.0
        } else {
            ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
        };
        if (p - (a + ab * t)).length() <= eps {
            return true;
        }
    }
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) {
            let t = (p.y - a.y) / (b.y - a.y);
            if p.x < a.x + t * (b.x - a.x) {
                inside = !inside;
            }
        }
    }
    inside
}

/// 記録した層順序が「画面に見える重なり」と一致することを確かめる。
///
/// 折り終わる直前(t=0.99)の高さは本当の上下そのものなので、平らな状態で重なって
/// いる面の組を選び、その重なり位置での高さの大小が層順序と同じ向きかを見る。
/// 高さの差がごく小さい組(まだ動いていない層どうし)は上下を読めないので飛ばす。
fn assert_display_order(doc: &Document, label: &str) {
    const READABLE: f64 = 1e-4;
    let up_to = doc.sequence.len();
    let faces = extract_faces(&doc.cp);
    let (state, warnings) = flat_state_at(doc, &faces, up_to)
        .unwrap_or_else(|e| panic!("{label}: 畳んだ状態が求まらない: {e}"));
    assert!(warnings.is_empty(), "{label}: 警告なしで再生できる: {warnings:?}");

    let pos: HashMap<u32, DVec2> = doc
        .cp
        .vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect();
    // 各面の畳み平面での境界多角形
    let plane_poly: HashMap<FaceId, Vec<DVec2>> = faces
        .iter()
        .map(|f| {
            let pl = state.placements[&f.id];
            (
                f.id,
                f.vertices
                    .iter()
                    .filter_map(|v| pos.get(v).copied())
                    .map(|p| pl.apply(p))
                    .collect(),
            )
        })
        .collect();

    let frame = replay(doc, up_to, 0.99).frame;
    let maps: HashMap<FaceId, FaceMap> = frame
        .faces
        .iter()
        .filter_map(|f3| {
            let face = faces.iter().find(|f| f.id == f3.face)?;
            FaceMap::new(&doc.cp, face, &f3.polygon).map(|m| (f3.face, m))
        })
        .collect();

    let mut compared = 0usize;
    for f in &faces {
        let rep = DVec2::from(representative_point(&doc.cp, f));
        let q = state.placements[&f.id].apply(rep);
        for g in &faces {
            if g.id == f.id {
                continue;
            }
            if !inside_polygon(&plane_poly[&g.id], q, -1e-9) {
                continue;
            }
            let rep_g = state.placements[&g.id].inverse().apply(q);
            let (Some(mf), Some(mg)) = (maps.get(&f.id), maps.get(&g.id)) else {
                continue;
            };
            let (zf, zg) = (mf.height(rep), mg.height(rep_g));
            if (zf - zg).abs() <= READABLE {
                continue;
            }
            compared += 1;
            let lf = state.order.iter().position(|&id| id == f.id).unwrap();
            let lg = state.order.iter().position(|&id| id == g.id).unwrap();
            assert_eq!(
                zf < zg,
                lf < lg,
                "{label}: 面{}(z={zf:.6}, 層{lf})と面{}(z={zg:.6}, 層{lg})の上下が層順序と食い違う",
                f.id,
                g.id
            );
        }
    }
    assert!(compared > 0, "{label}: 上下を読み取れる重なりが1組も無い");
}

/// 折り目の向き(山谷)と層順序が食い違わないことを確かめる。
///
/// 折り目でつながった2つの面は、折り目を「表から見たときの山谷」で上下が決まる:
/// 表向きの面から見て谷なら相手の面は上、山なら下(裏返っている面から見るときは逆)。
/// 畳んだ形が同じでも重なり順だけが違う中割り折りとかぶせ折りは、この向きで見分けられる。
fn assert_fold_senses(doc: &Document, label: &str) {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, warnings) = flat_state_at(doc, &faces, up_to)
        .unwrap_or_else(|e| panic!("{label}: 畳んだ状態が求まらない: {e}"));
    assert!(warnings.is_empty(), "{label}: 警告なしで再生できる: {warnings:?}");

    let mut edge_faces: HashMap<u32, Vec<FaceId>> = HashMap::new();
    for f in &faces {
        let mut ids = f.edges.clone();
        ids.sort_unstable();
        ids.dedup();
        for eid in ids {
            edge_faces.entry(eid).or_default().push(f.id);
        }
    }
    let mut checked = 0usize;
    for e in &doc.cp.edges {
        if !matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let Some(fs) = edge_faces.get(&e.id) else {
            continue;
        };
        if fs.len() != 2 {
            continue;
        }
        let (a, b) = (fs[0], fs[1]);
        let (pa, pb) = (state.placements[&a], state.placements[&b]);
        // 折られていない(平らにつながったまま)折り目は上下を決めない
        if pa.mirrored == pb.mirrored {
            continue;
        }
        let above = matches!(
            (e.kind, pa.mirrored),
            (EdgeKind::Valley, false) | (EdgeKind::Mountain, true)
        );
        let ia = state.order.iter().position(|&id| id == a).unwrap();
        let ib = state.order.iter().position(|&id| id == b).unwrap();
        checked += 1;
        assert_eq!(
            ib > ia,
            above,
            "{label}: 折り目(辺{} {:?})でつながる面{a}(層{ia})と面{b}(層{ib})の上下が向きと食い違う",
            e.id,
            e.kind
        );
    }
    assert!(checked > 0, "{label}: 折られた折り目が1本も無い");
}

// ---------------------------------------------------------------------------
// 段折り
// ---------------------------------------------------------------------------

/// 平らな紙への段折り: 折り線2本・3層・山谷が交互・先の紙が段幅の2倍ずれる。
#[test]
fn pleat_makes_two_alternating_creases_and_three_layers() {
    let mut doc = square_doc();
    let a = apply(
        &mut doc,
        pleat,
        Vec::new(),
        [[0.4, 0.0], [0.4, 1.0]],
        [0.5, 0.5],
    )
    .expect("平らな紙は段折りできる");
    assert!(a.warnings.is_empty(), "{:?}", a.warnings);

    // 面は3つ(もとの紙・段の中・その先)
    assert_eq!(a.faces.len(), 3);
    assert_eq!(a.order.len(), 3);
    // 展開図には x=0.4(谷)と x=0.5(山)が入る
    assert_eq!(edge_kind_at(&doc.cp, [0.4, 0.5]), Some(EdgeKind::Valley));
    assert_eq!(edge_kind_at(&doc.cp, [0.5, 0.5]), Some(EdgeKind::Mountain));

    // 手順は段折りとして記録され、折り線2本ぶんのdriverを持つ
    let step = &doc.sequence[0];
    assert_eq!(step.kind, TechniqueKind::Pleat);
    assert_eq!(step.drivers.len(), 2);
    assert!(step.drivers.iter().any(|d| d.target_angle_deg == -180.0));
    assert!(step.drivers.iter().any(|d| d.target_angle_deg == 180.0));
    assert_eq!(step.layer_order.as_ref().map(Vec::len), Some(3));

    // 段の先(x>0.5)は段幅0.1の2倍だけ左へずれる
    let body = face_with_rep(&a, |r| r[0] < 0.4);
    let band = face_with_rep(&a, |r| (0.4..0.5).contains(&r[0]));
    let tail = face_with_rep(&a, |r| r[0] > 0.5);
    assert_eq!((body.len(), band.len(), tail.len()), (1, 1, 1));
    let shift = a.at[&tail[0]] - DVec2::from(a.rep[&tail[0]]);
    let body_shift = a.at[&body[0]] - DVec2::from(a.rep[&body[0]]);
    assert!(
        ((shift - body_shift) - DVec2::new(-0.2, 0.0)).length() < 1e-9,
        "段の先は段幅の2倍だけずれる(実際 {:?})",
        shift - body_shift
    );

    // 重なりは下から「もとの紙・段の中・その先」(表示座標系では上下がそろって反転しうる)
    let layers = (
        layer_of(&a, body[0]),
        layer_of(&a, band[0]),
        layer_of(&a, tail[0]),
    );
    assert!(
        layers == (0, 1, 2) || layers == (2, 1, 0),
        "重なりは順に並ぶ(実際 {layers:?})"
    );
    assert_display_order(&doc, "平らな紙の段折り");
    assert_fold_senses(&doc, "平らな紙の段折り");
}

/// 2層のフラップへの段折り: 両方の層に折り線が入り、段の中の折り目はそのまま残る。
#[test]
fn pleat_on_two_layer_flap_folds_both_layers() {
    let mut doc = square_doc();
    // 下ごしらえ: y=0.5 で上半分を下へ折る(2層。折り目=谷)
    fold(&mut doc, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25], FoldDirection::Up);
    assert_eq!(edge_kind_at(&doc.cp, [0.5, 0.5]), Some(EdgeKind::Valley));

    let faces = extract_faces(&doc.cp);
    let flap: Vec<FaceId> = faces.iter().map(|f| f.id).collect();
    let a = apply(
        &mut doc,
        pleat,
        flap,
        [[0.6, 0.0], [0.6, 1.0]],
        [0.7, 0.25],
    )
    .expect("2層のフラップも段折りできる");
    assert!(a.warnings.is_empty(), "{:?}", a.warnings);

    // 2層それぞれが3つに分かれて6面
    assert_eq!(a.faces.len(), 6);
    // 段の中の折り目は谷のまま。段の中の紙は「まとめて折り返された」ので、
    // 裏返ると同時に重なり順も入れ替わり、差し引きで山谷は変わらない
    assert_eq!(edge_kind_at(&doc.cp, [0.65, 0.5]), Some(EdgeKind::Valley));
    assert_eq!(edge_kind_at(&doc.cp, [0.3, 0.5]), Some(EdgeKind::Valley));
    assert_eq!(edge_kind_at(&doc.cp, [0.85, 0.5]), Some(EdgeKind::Valley));

    // driver: 折り線2本×2層
    let step = doc.sequence.last().unwrap();
    assert_eq!(step.kind, TechniqueKind::Pleat);
    assert_eq!(step.drivers.len(), 4);
    assert_display_order(&doc, "2層フラップの段折り");
    assert_fold_senses(&doc, "2層フラップの段折り");
}

/// 段の幅0(基準点が折り線の上)はErrで、展開図も手順も変わらない。
#[test]
fn pleat_rejects_zero_gap_without_touching_document() {
    let mut doc = square_doc();
    let before = doc.clone();
    let err = apply(
        &mut doc,
        pleat,
        Vec::new(),
        [[0.4, 0.0], [0.4, 1.0]],
        [0.4, 0.5],
    )
    .expect_err("段の幅0はエラー");
    assert!(err.contains("段の幅"), "{err}");
    assert_eq!(doc, before, "失敗時は文書を変更しない");
}

// ---------------------------------------------------------------------------
// 中割り折り・かぶせ折り
// ---------------------------------------------------------------------------

/// 2層のフラップ(正方形を半分に折ったもの)。折り目(背)は y=0.5。
fn two_layer_flap() -> Document {
    let mut doc = square_doc();
    fold(&mut doc, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25], FoldDirection::Up);
    doc
}

/// 中割り折り: 先端が折り線で折り返され、フラップの層の間に挟まる。
#[test]
fn inside_reverse_tucks_the_tip_between_the_flap_layers() {
    let mut doc = two_layer_flap();
    let faces = extract_faces(&doc.cp);
    let flap: Vec<FaceId> = faces.iter().map(|f| f.id).collect();
    assert_eq!(flap.len(), 2, "下ごしらえは2層");

    // 背(y=0.5)を x=0.7 で横切る斜めの線。先端(右下)を左側へ折り込む
    let a = apply(
        &mut doc,
        inside_reverse,
        flap,
        [[0.7, 0.5], [0.5, 0.0]],
        [0.2, 0.25],
    )
    .expect("2層のフラップは中割り折りできる");
    assert!(a.warnings.is_empty(), "{:?}", a.warnings);

    // 各層が2つに分かれて4面
    assert_eq!(a.faces.len(), 4);
    assert_eq!(a.order.len(), 4);

    // 展開図: 背(y=0.5)を挟んでV字に折り線が2本入り、線種は同じ(谷)。
    // 手前の層は裏返っているので、同じ「先端を内側へ回す」動きが同じ線種で表される
    let upper = edge_kind_at(&doc.cp, [0.6, 0.75]).expect("上半分の折り線");
    let lower = edge_kind_at(&doc.cp, [0.6, 0.25]).expect("下半分の折り線");
    assert_eq!(upper, lower, "中割り折りの2本は同じ線種になる");
    assert_eq!(upper, EdgeKind::Valley);
    // 先端の中の背(x>0.7)は山谷が入れ替わる。手前の区間はそのまま
    assert_eq!(edge_kind_at(&doc.cp, [0.85, 0.5]), Some(EdgeKind::Mountain));
    assert_eq!(edge_kind_at(&doc.cp, [0.35, 0.5]), Some(EdgeKind::Valley));

    // 手順: 中割り折りとして記録され、折り線2本+入れ替えた背1本のdriverを持つ
    let step = doc.sequence.last().unwrap();
    assert_eq!(step.kind, TechniqueKind::InsideReverse);
    assert_eq!(step.drivers.len(), 3);
    assert_eq!(step.layer_order.as_ref().map(Vec::len), Some(4));

    // 層: 先端の2枚がフラップの残り2枚の内側に挟まる
    let tip = face_with_rep(&a, |r| r[0] > 0.7);
    let rest = face_with_rep(&a, |r| r[0] <= 0.7);
    assert_eq!((tip.len(), rest.len()), (2, 2));
    let tip_layers = [layer_of(&a, tip[0]), layer_of(&a, tip[1])];
    assert!(
        tip_layers.contains(&1) && tip_layers.contains(&2),
        "先端は内側(層1と層2)に入る(実際 {tip_layers:?})"
    );
    assert!(
        rest.iter().all(|id| {
            let l = layer_of(&a, *id);
            l == 0 || l == 3
        }),
        "フラップの残りは外側(層0と層3)"
    );
    assert_fold_senses(&doc, "中割り折り");
}

/// かぶせ折り: 先端がフラップの層を外から包む(中割り折りと層の位置が逆)。
#[test]
fn outside_reverse_wraps_the_tip_around_the_flap() {
    let mut doc = two_layer_flap();
    let faces = extract_faces(&doc.cp);
    let flap: Vec<FaceId> = faces.iter().map(|f| f.id).collect();

    let a = apply(
        &mut doc,
        outside_reverse,
        flap,
        [[0.7, 0.5], [0.5, 0.0]],
        [0.2, 0.25],
    )
    .expect("2層のフラップはかぶせ折りできる");
    assert!(a.warnings.is_empty(), "{:?}", a.warnings);
    assert_eq!(a.faces.len(), 4);

    // 折り線の線種は中割り折りと逆(山)になる。先端が裏返る点は同じなので、
    // 先端の中の背は中割り折りと同じく山へ入れ替わる
    let upper = edge_kind_at(&doc.cp, [0.6, 0.75]).expect("上半分の折り線");
    let lower = edge_kind_at(&doc.cp, [0.6, 0.25]).expect("下半分の折り線");
    assert_eq!(upper, EdgeKind::Mountain);
    assert_eq!(lower, EdgeKind::Mountain);
    assert_eq!(edge_kind_at(&doc.cp, [0.85, 0.5]), Some(EdgeKind::Mountain));

    let step = doc.sequence.last().unwrap();
    assert_eq!(step.kind, TechniqueKind::OutsideReverse);
    assert_eq!(step.drivers.len(), 3);

    // 層: 先端の2枚が外側(いちばん下といちばん上)へ回る
    let tip = face_with_rep(&a, |r| r[0] > 0.7);
    assert_eq!(tip.len(), 2);
    let tip_layers = [layer_of(&a, tip[0]), layer_of(&a, tip[1])];
    assert!(
        tip_layers.contains(&0) && tip_layers.contains(&3),
        "先端は外側(層0と層3)へ回る(実際 {tip_layers:?})"
    );
    assert_fold_senses(&doc, "かぶせ折り");
}

/// 中割り折りとかぶせ折りは、折り上がりの位置(畳んだ平面)が同じで重なり順だけ違う。
#[test]
fn inside_and_outside_reverse_differ_only_in_layer_order() {
    let line = [[0.7, 0.5], [0.5, 0.0]];
    let reference = [0.2, 0.25];
    let mut inside_doc = two_layer_flap();
    let mut outside_doc = two_layer_flap();
    let flap: Vec<FaceId> = extract_faces(&inside_doc.cp).iter().map(|f| f.id).collect();

    let i = apply(&mut inside_doc, inside_reverse, flap.clone(), line, reference).unwrap();
    let o = apply(&mut outside_doc, outside_reverse, flap, line, reference).unwrap();

    // 展開図の頂点は同じ(線種だけが違う)
    assert_eq!(inside_doc.cp.vertices, outside_doc.cp.vertices);
    for f in &i.faces {
        assert!(
            (i.at[&f.id] - o.at[&f.id]).length() < 1e-9,
            "面 {} の畳んだ位置は同じ",
            f.id
        );
    }
    assert_ne!(i.order, o.order, "重なり順は違う");
}

/// 折り線がフラップを横切っていない指定はErr(理由を日本語で返し、文書は無変更)。
#[test]
fn reverse_fold_rejects_a_line_that_does_not_cross_the_flap() {
    let mut doc = two_layer_flap();
    let before = doc.clone();
    let faces = extract_faces(&doc.cp);
    let flap: Vec<FaceId> = faces.iter().map(|f| f.id).collect();

    // 紙の外を通る線(どの層も横切らない)
    let err = apply(
        &mut doc,
        inside_reverse,
        flap.clone(),
        [[1.5, 0.0], [1.5, 1.0]],
        [0.5, 0.25],
    )
    .expect_err("横切らない線はエラー");
    assert!(err.contains("中割り折りができません"), "{err}");
    assert_eq!(doc, before, "失敗時は文書を変更しない");

    // フラップが1層だけの指定もエラー
    let err = apply(
        &mut doc,
        inside_reverse,
        vec![flap[0]],
        [[0.7, 0.5], [0.5, 0.0]],
        [0.2, 0.25],
    )
    .expect_err("1層のフラップはエラー");
    assert!(err.contains("2層以上"), "{err}");
    assert_eq!(doc, before);

    // 折り込む先を示す点が折り線の上にある指定もエラー
    let err = apply(
        &mut doc,
        inside_reverse,
        flap,
        [[0.7, 0.5], [0.5, 0.0]],
        [0.6, 0.25],
    )
    .expect_err("折り線上の基準点はエラー");
    assert!(err.contains("折り線の上"), "{err}");
    assert_eq!(doc, before);
}

/// 技法で作った形は、展開図と手順だけから同じ形に折り直せる(3D状態を保存しない設計)。
#[test]
fn techniques_replay_from_the_crease_pattern() {
    for (label, technique) in [
        ("中割り折り", inside_reverse as Technique),
        ("かぶせ折り", outside_reverse as Technique),
    ] {
        let mut doc = two_layer_flap();
        let flap: Vec<FaceId> = extract_faces(&doc.cp).iter().map(|f| f.id).collect();
        let a = apply(
            &mut doc,
            technique,
            flap,
            [[0.7, 0.5], [0.5, 0.0]],
            [0.2, 0.25],
        )
        .unwrap_or_else(|e| panic!("{label}が折れない: {e}"));

        let faces = extract_faces(&doc.cp);
        let (replayed, warnings) =
            flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
        assert!(warnings.is_empty(), "{label}: {warnings:?}");
        assert_eq!(replayed.order, a.order, "{label}: 層順序が再生結果と一致する");
        for f in &faces {
            assert!(
                a.at[&f.id]
                    .abs_diff_eq(replayed.placements[&f.id].apply(DVec2::from(a.rep[&f.id])), 1e-6),
                "{label}: 面 {} の位置が再生結果と一致する",
                f.id
            );
        }
        // 折り上がりは平ら(全ての面がz=0に乗る)
        let frame = replay(&doc, doc.sequence.len(), 1.0).frame;
        assert!(
            frame
                .faces
                .iter()
                .all(|f| f.polygon.iter().all(|p| p[2].abs() < 1e-6)),
            "{label}: 折り上がりは平ら"
        );
    }
}

/// 鶴の首を折る場面の再現: 半分に折った紙の端を中割り折りして首を立てる。
/// 層数・層順序・展開図への追加線が期待どおりで、続けてもう一度中割り折りできる
/// (首の先を頭にする=鶴の折り方と同じ流れ)。
#[test]
fn inside_reverse_twice_makes_a_neck_and_a_head() {
    let mut doc = two_layer_flap();
    let flap: Vec<FaceId> = extract_faces(&doc.cp).iter().map(|f| f.id).collect();
    let edges_before = doc.cp.edges.len();

    // 1回目: 首を立てる
    let a = apply(
        &mut doc,
        inside_reverse,
        flap,
        [[0.7, 0.5], [0.5, 0.0]],
        [0.2, 0.25],
    )
    .expect("首の中割り折り");
    assert_eq!(a.faces.len(), 4, "2層が4層になる");
    // 追加された線: 各層1本ずつ(横切られた輪郭線と背も分割される)
    assert!(
        doc.cp.edges.len() > edges_before,
        "展開図に折り線が増える"
    );
    // 中割り折りの2本は谷、入れ替わった背の先だけが山になる
    let mountains = doc
        .cp
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Mountain)
        .count();
    assert_eq!(mountains, 1, "入れ替わった背の先だけが山");
    let valleys = doc
        .cp
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Valley)
        .count();
    assert_eq!(valleys, 3, "背の手前側1本と中割り折りの2本");

    // 2回目: 首の先を中割り折りして頭にする(首の4層のうち先端側の2層を使う)
    let faces = extract_faces(&doc.cp);
    let (state, _) = flat_state_at(&doc, &faces, doc.sequence.len()).unwrap();
    // 先端(展開図で x>0.7 の部分)から来た層を選ぶ
    let neck: Vec<FaceId> = faces
        .iter()
        .filter(|f| representative_point(&doc.cp, f)[0] > 0.7)
        .map(|f| f.id)
        .collect();
    assert_eq!(neck.len(), 2);
    // 首がどこに立っているかを見て、その先端を折り返す線を引く
    let tips: Vec<DVec2> = neck
        .iter()
        .map(|id| state.placements[id].apply(DVec2::from(representative_point(
            &doc.cp,
            faces.iter().find(|f| f.id == *id).unwrap(),
        ))))
        .collect();
    let center = (tips[0] + tips[1]) * 0.5;
    assert!(center.x < 0.7, "首は折り線の反対側へ倒れている");

    let b = apply(
        &mut doc,
        inside_reverse,
        neck,
        [[0.45, 0.5], [0.35, 0.0]],
        [0.6, 0.25],
    )
    .expect("頭の中割り折り");
    assert_eq!(b.faces.len(), 6, "首の2層がさらに分かれて6層になる");
    assert_eq!(doc.sequence.len(), 3);
    assert_fold_senses(&doc, "鶴の首と頭");
}
