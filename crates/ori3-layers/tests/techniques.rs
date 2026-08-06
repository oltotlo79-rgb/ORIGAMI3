//! 基本技法のマクロ(段折り・中割り折り・かぶせ折り・開いてつぶす・花弁折り)のテスト。
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
//! 中割り折りにだけ t=0.99 の高さ読みを使えない理由:
//! 中割り折りは「フラップを開いて先端を層の間へ入れる」動きだが、開くには
//! フラップの背をいったん平らに戻す必要がある。手順再生は前の手順の折り目を
//! 180°に固定したまま補間するので、先端は紙全体の外側を大回りして層の間へ入る。
//! 折り上がり(t=1)の形と重なりは正しいが、途中の高さは「外側」を指してしまう。
//! かぶせ折り(先端が外側を包む)と段折りは途中の高さと折り上がりが一致するので、
//! そのまま t=0.99 の検証を使う。中割り折りは折り目の向きとの一致
//! ([`assert_fold_senses`])で確かめる。

use std::collections::HashMap;

use glam::{DVec2, DVec3};
use ori3_cp::{Face, extract_faces};
use ori3_layers::flat_state::representative_point;
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, fold_through};
use ori3_layers::techniques::TechniqueInput;
use ori3_layers::{
    flat_state_at, inside_reverse, open_sink, outside_reverse, petal, pleat, replay, squash,
};
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

/// 再生一致テスト1件ぶんの指定(名前・技法・折り線・基準点)。
type ReplayCase = (&'static str, Technique, [[f64; 2]; 2], [f64; 2]);

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
    apply_input(
        doc,
        technique,
        TechniqueInput {
            flap,
            line,
            reference_point,
            open_to_back: None,
        },
    )
}

/// 入力をそのまま渡す版([`apply`] は既定の入力を組み立ててこれを呼ぶ)。
fn apply_input(
    doc: &mut Document,
    technique: Technique,
    input: TechniqueInput,
) -> Result<Applied, String> {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, _) = flat_state_at(doc, &faces, up_to)?;
    let mut cp = doc.cp.clone();
    let res = technique(&mut cp, &faces, &state, &input)?;
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
    // 先端が外側を包むかぶせ折りは、折り終わる直前の高さも折り上がりと一致する
    assert_display_order(&doc, "かぶせ折り");
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

    // 展開図に入る点の位置は同じ(IDの付き方は折る順で変わるので位置で比べる)
    let sorted_positions = |doc: &Document| {
        let mut v: Vec<[f64; 2]> = doc.cp.vertices.iter().map(|v| v.pos).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v
    };
    assert_eq!(
        sorted_positions(&inside_doc),
        sorted_positions(&outside_doc)
    );
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
    // 段折りの基準点は2本目の折り線の位置、中割り・かぶせは先端が向かう側
    let d = 0.5 * std::f64::consts::SQRT_2;
    let cases: [ReplayCase; 5] = [
        ("段折り", pleat as Technique, [[0.6, 0.0], [0.6, 1.0]], [0.7, 0.25]),
        // つぶし折りは背(y=0.5)を開いて右端まわりに45°回す
        (
            "つぶし折り",
            squash as Technique,
            [[0.0, 0.5], [1.0, 0.5]],
            [1.0 - d, 0.5 - d],
        ),
        (
            "中割り折り",
            inside_reverse as Technique,
            [[0.7, 0.5], [0.5, 0.0]],
            [0.2, 0.25],
        ),
        (
            "かぶせ折り",
            outside_reverse as Technique,
            [[0.7, 0.5], [0.5, 0.0]],
            [0.2, 0.25],
        ),
        // 花弁折りは背(y=0.5)を中心線に、右端の先端を持ち上げる
        ("花弁折り", petal as Technique, [[0.0, 0.5], [1.0, 0.5]], [1.0, 0.5]),
    ];
    for (label, technique, line, reference) in cases {
        let mut doc = two_layer_flap();
        let flap: Vec<FaceId> = extract_faces(&doc.cp).iter().map(|f| f.id).collect();
        let a = apply(&mut doc, technique, flap, line, reference)
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

/// 正方形を2回折った4層の紙(層順序は下→上)。
fn four_layer_stack() -> Document {
    let mut doc = square_doc();
    fold(&mut doc, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25], FoldDirection::Up);
    fold(&mut doc, [[0.5, 0.0], [0.5, 0.5]], [0.25, 0.25], FoldDirection::Up);
    doc
}

/// 層の数が奇数のフラップでも、紙のつながりどおりに中割り折りできる。
///
/// 段折りでできた3層(もとの紙・段の中・その先が、2本の折り目でつながっている)を
/// まとめて中割り折りする。層の数を機械的に半分に割る決め方では扱えないが、
/// 「折り線が横切る折り目でつながった層は反対向きに回る」という紙のつながりから
/// 決めれば、奇数層でもそのまま折れる。
#[test]
fn inside_reverse_on_odd_number_of_layers() {
    let mut doc = square_doc();
    // 下ごしらえ: 段折りで3層にする(折り目は x=0.6 と x=0.7 の縦線)
    apply(
        &mut doc,
        pleat,
        Vec::new(),
        [[0.6, 0.0], [0.6, 1.0]],
        [0.7, 0.5],
    )
    .expect("段折りで3層にする");
    let faces = extract_faces(&doc.cp);
    assert_eq!(faces.len(), 3);
    let (state, _) = flat_state_at(&doc, &faces, doc.sequence.len()).unwrap();
    let flap: Vec<FaceId> = state.order.clone();

    // 3層が重なっている帯を、段の折り目2本を横切る横線で中割り折りする
    let a = apply(
        &mut doc,
        inside_reverse,
        flap,
        [[0.0, 0.5], [1.0, 0.5]],
        [0.5, 0.9],
    )
    .expect("3層のフラップも中割り折りできる");
    assert!(a.warnings.is_empty(), "紙は裂けない: {:?}", a.warnings);
    assert_eq!(a.faces.len(), 6, "3層それぞれが2つに分かれる");
    assert_fold_senses(&doc, "3層フラップの中割り折り");

    // 展開図と手順だけから同じ形に折り直せる
    let new_faces = extract_faces(&doc.cp);
    let (replayed, warnings) =
        flat_state_at(&doc, &new_faces, doc.sequence.len()).expect("平らに畳める");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(replayed.order, a.order, "層順序が再生結果と一致する");
    for f in &new_faces {
        assert!(
            a.at[&f.id]
                .abs_diff_eq(replayed.placements[&f.id].apply(DVec2::from(a.rep[&f.id])), 1e-6),
            "面 {} の位置が再生結果と一致する",
            f.id
        );
    }
}

/// 重なりの一部だけを選ぶと(選んだ層の先端が、選ばなかった層の先端と折り目で
/// つながっている場合)、そのままでは紙が裂ける。断らずに警告して続ける
/// (「止めずに警告」原則。実際に折れる指定を断らないため)。
#[test]
fn reverse_fold_warns_when_the_selected_layers_are_not_a_whole_flap() {
    let mut doc = four_layer_stack();
    let faces = extract_faces(&doc.cp);
    let (state, _) = flat_state_at(&doc, &faces, doc.sequence.len()).unwrap();
    assert_eq!(state.order.len(), 4);

    // 4層のうち下3層だけを選ぶ。この重なりでは4層が輪のようにつながっているため、
    // 3層だけを折り返すと、残した層とのつながりが切れる
    let flap: Vec<FaceId> = state.order[..3].to_vec();
    let a = apply(
        &mut doc,
        inside_reverse,
        flap,
        [[0.35, 0.5], [0.15, 0.0]],
        [0.05, 0.25],
    )
    .expect("紙が裂ける指定でも、折り自体は続行する");
    assert!(
        a.warnings.iter().any(|w| w.contains("裂けます")),
        "裂けることを知らせる: {:?}",
        a.warnings
    );
}

/// 4層のフラップ(奥2枚・手前2枚)でも中割り折りできる。
#[test]
fn inside_reverse_on_four_layer_flap() {
    let mut doc = four_layer_stack();
    let faces = extract_faces(&doc.cp);
    let (state, _) = flat_state_at(&doc, &faces, doc.sequence.len()).unwrap();
    let flap: Vec<FaceId> = state.order.clone();

    let a = apply(
        &mut doc,
        inside_reverse,
        flap,
        [[0.35, 0.5], [0.15, 0.0]],
        [0.05, 0.25],
    )
    .expect("4層のフラップも中割り折りできる");
    assert!(a.warnings.is_empty(), "{:?}", a.warnings);
    assert_eq!(a.faces.len(), 8, "4層それぞれが2つに分かれる");
    assert_fold_senses(&doc, "4層フラップの中割り折り");

    // 展開図と手順だけから同じ形に折り直せる
    let new_faces = extract_faces(&doc.cp);
    let (replayed, warnings) =
        flat_state_at(&doc, &new_faces, doc.sequence.len()).expect("平らに畳める");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(replayed.order, a.order);
}

// ---------------------------------------------------------------------------
// 開いてつぶす折り
// ---------------------------------------------------------------------------

/// つぶし折り: 背が開いて(角0°)、手前の層は新しい折り線で折り返し、
/// 奥の層は鏡映2回=回転で丸ごと回る。
#[test]
fn squash_opens_the_spine_and_spreads_the_flap() {
    let mut doc = two_layer_flap(); // 2層。背(開く折り目)は y=0.5
    let flap: Vec<FaceId> = extract_faces(&doc.cp).iter().map(|f| f.id).collect();
    let edges_before = doc.cp.edges.len();
    // 背の右端(1,0.5)を支点に、背を左(180°)から左下(225°)へ45°回す
    let d = 0.5 * std::f64::consts::SQRT_2;
    let a = apply(
        &mut doc,
        squash,
        flap,
        [[0.0, 0.5], [1.0, 0.5]],
        [1.0 - d, 0.5 - d],
    )
    .expect("つぶし折りできる");

    assert!(a.warnings.is_empty(), "紙は裂けない: {:?}", a.warnings);
    assert_eq!(a.faces.len(), 3, "手前の層が新しい折り線で2つに分かれて3面");
    assert_eq!(
        doc.cp.edges.len(),
        edges_before + 2,
        "追加線は1本(紙の縁が交点で2本に分かれるぶんを含めて+2)"
    );

    let step = &doc.sequence[doc.sequence.len() - 1];
    assert_eq!(step.kind, TechniqueKind::Squash);
    assert!(
        step.drivers.iter().any(|dr| dr.target_angle_deg == 0.0),
        "開いた背は0°で記録される: {:?}",
        step.drivers
    );
    assert!(
        step.drivers.iter().any(|dr| dr.target_angle_deg.abs() == 180.0),
        "新しい折り線は±180°で記録される: {:?}",
        step.drivers
    );

    // 背でつながる2面の向きがそろう(=背が開いて平ら)
    let faces = extract_faces(&doc.cp);
    let (state, _) = flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
    let pos: HashMap<u32, DVec2> =
        doc.cp.vertices.iter().map(|v| (v.id, DVec2::from(v.pos))).collect();
    let spine = doc
        .cp
        .edges
        .iter()
        .find(|e| {
            let (Some(&p0), Some(&p1)) = (pos.get(&e.v0), pos.get(&e.v1)) else {
                return false;
            };
            (p0.y - 0.5).abs() < 1e-9 && (p1.y - 0.5).abs() < 1e-9
        })
        .expect("背の辺がある");
    let across: Vec<FaceId> = faces
        .iter()
        .filter(|f| f.edges.contains(&spine.id))
        .map(|f| f.id)
        .collect();
    assert_eq!(across.len(), 2, "背は2面をつなぐ");
    assert_eq!(
        state.placements[&across[0]].mirrored, state.placements[&across[1]].mirrored,
        "背が開いて2面の向きがそろう"
    );
    assert_eq!(a.order.len(), 3, "層は3枚");
    assert_eq!(state.order, a.order, "層順序が再生と一致する");
    assert_fold_senses(&doc, "つぶし折り");
}

/// つぶし折りは「向こう側へ開く」指定もできる(実際の紙で奥へ開く動き)。
/// 紙の行き先は手前へ開いたときと同じで、つぶした紙が入る重なりの側だけが変わる。
#[test]
fn squash_can_open_the_flap_to_the_back() {
    let d = 0.5 * std::f64::consts::SQRT_2;
    let line = [[0.0, 0.5], [1.0, 0.5]];
    let reference_point = [1.0 - d, 0.5 - d];

    let mut front_doc = two_layer_flap();
    let flap: Vec<FaceId> = extract_faces(&front_doc.cp).iter().map(|f| f.id).collect();
    let front = apply(&mut front_doc, squash, flap.clone(), line, reference_point)
        .expect("手前へ開ける");

    let mut back_doc = two_layer_flap();
    let back = apply_input(
        &mut back_doc,
        squash,
        TechniqueInput {
            flap,
            line,
            reference_point,
            open_to_back: Some(true),
        },
    )
    .expect("向こうへ開ける");

    assert!(back.warnings.is_empty(), "紙は裂けない: {:?}", back.warnings);
    assert_eq!(back.faces.len(), front.faces.len(), "分かれる面の数は同じ");
    for f in &back.faces {
        assert!(
            back.at[&f.id].abs_diff_eq(front.at[&f.id], 1e-12),
            "面 {} の行き先は開く向きによらず同じ",
            f.id
        );
    }
    let reversed: Vec<FaceId> = front.order.iter().rev().copied().collect();
    assert_eq!(back.order, reversed, "重なりの上下だけが入れ替わる");
    assert_display_order(&back_doc, "向こうへ開くつぶし折り");
    assert_fold_senses(&back_doc, "向こうへ開くつぶし折り");
}

/// 退化ケース: 2回半分に折った正方形(4層)を、2回のつぶし折りで
/// 予備基本形の重なりへ組み替える。
///
/// つぶす方向が背の延長上にある(回転角0)ので紙は1mmも動かない。変わるのは
/// 層順序(いちばん下の層が上へ回る)と、その層に接する1組の折り目の山谷だけ。
/// 実際の紙で「畳んだ正方形を開いて反対側から畳み直す」動きにあたる。
#[test]
fn squash_reorders_layers_without_moving_paper() {
    let mut doc = four_layer_stack();
    let faces = extract_faces(&doc.cp);
    let (before, _) = flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
    let start_order = before.order.clone();
    assert_eq!(start_order.len(), 4, "2回半分に折って4層");
    let kinds_before = kinds(&doc.cp);

    // 1回目: いちばん下の層を、右端(x=0.5)の背を開いて上へ回す
    let bottom = start_order[0];
    let a = apply(&mut doc, squash, vec![bottom], [[0.5, 0.0], [0.5, 1.0]], [0.5, 0.1])
        .expect("つぶし折りできる");
    assert!(a.warnings.is_empty(), "{:?}", a.warnings);
    assert_eq!(a.faces.len(), 4, "面は分かれない(新しい折り線が要らない)");
    for f in &a.faces {
        assert!(
            a.at[&f.id].abs_diff_eq(
                before.placements[&f.id].apply(DVec2::from(a.rep[&f.id])),
                1e-12
            ),
            "面 {} は1mmも動かない",
            f.id
        );
    }
    let mut rotated: Vec<FaceId> = start_order[1..].to_vec();
    rotated.push(bottom);
    assert_eq!(a.order, rotated, "いちばん下の層が上へ回る");
    let changed = changed_kinds(&kinds_before, &kinds(&doc.cp));
    assert_eq!(changed, 2, "山谷が変わるのは1組(2本)の折り目だけ");
    let step = &doc.sequence[doc.sequence.len() - 1];
    assert_eq!(step.kind, TechniqueKind::Squash);
    assert_eq!(step.drivers.len(), 2, "その2本ぶんだけ手順に記録される");

    // 2回目: 新しくいちばん下になった層を、上端(y=0.5)の背を開いて上へ回す
    let kinds_mid = kinds(&doc.cp);
    let bottom2 = a.order[0];
    let b = apply(&mut doc, squash, vec![bottom2], [[0.0, 0.5], [1.0, 0.5]], [0.1, 0.5])
        .expect("2回目もつぶし折りできる");
    assert!(b.warnings.is_empty(), "{:?}", b.warnings);
    for f in &b.faces {
        assert!(
            b.at[&f.id].abs_diff_eq(
                before.placements[&f.id].apply(DVec2::from(b.rep[&f.id])),
                1e-12
            ),
            "面 {} は2回目も動かない",
            f.id
        );
    }
    assert_eq!(changed_kinds(&kinds_mid, &kinds(&doc.cp)), 2, "2回目も1組だけ");
    // 4層の並びは元の並びを2つずらしたもの(=予備基本形と同じつながり方)
    let expected: Vec<FaceId> = start_order[2..]
        .iter()
        .chain(start_order[..2].iter())
        .copied()
        .collect();
    assert_eq!(b.order, expected, "層順序が2つずれる");
    assert_fold_senses(&doc, "つぶし折り(退化)");
}

/// 展開図の折り目の線種(辺ID→線種)。
fn kinds(cp: &CreasePattern) -> HashMap<u32, EdgeKind> {
    cp.edges.iter().map(|e| (e.id, e.kind)).collect()
}

/// 線種が変わった辺の本数。
fn changed_kinds(before: &HashMap<u32, EdgeKind>, after: &HashMap<u32, EdgeKind>) -> usize {
    after.iter().filter(|(id, k)| before.get(id) != Some(k)).count()
}

/// つぶし折りが断るのは幾何的に決められない入力だけ。断ったときは文書を変えない。
#[test]
fn squash_rejects_undefined_input_without_touching_document() {
    let mut doc = two_layer_flap();
    let before = doc.clone();
    let flap: Vec<FaceId> = extract_faces(&doc.cp).iter().map(|f| f.id).collect();

    // 中心線が退化(2点が一致)
    let err = apply(&mut doc, squash, flap.clone(), [[0.5, 0.5], [0.5, 0.5]], [0.2, 0.2])
        .expect_err("退化した中心線はエラー");
    assert!(err.contains("2点が一致"), "{err}");
    assert_eq!(doc, before, "失敗時は文書を変更しない");

    // フラップの指定が無い
    let err = apply(&mut doc, squash, Vec::new(), [[0.0, 0.5], [1.0, 0.5]], [0.2, 0.2])
        .expect_err("フラップ未指定はエラー");
    assert!(err.contains("フラップ"), "{err}");
    assert_eq!(doc, before);

    // 無い層を指定した
    let missing = flap.iter().max().copied().unwrap_or(0) + 99;
    let err = apply(&mut doc, squash, vec![missing], [[0.0, 0.5], [1.0, 0.5]], [0.2, 0.2])
        .expect_err("無い層はエラー");
    assert!(err.contains("見つかりません"), "{err}");
    assert_eq!(doc, before);

    // 中心線に背が無い指定は、断らずに警告して続ける(「止めずに警告」原則)
    let a = apply(&mut doc, squash, flap, [[0.0, 0.2], [1.0, 0.2]], [0.2, 0.1])
        .expect("背が無くても折れる");
    assert!(
        a.warnings.iter().any(|w| w.contains("開ける折り目")),
        "背が見つからない警告が出る: {:?}",
        a.warnings
    );
}

/// 予備基本形(4層。層のつながりが輪になっていて、どの向きにも開ける)。
fn preliminary_base() -> Document {
    let mut doc = four_layer_stack();
    let faces = extract_faces(&doc.cp);
    let (state, _) = flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
    let bottom = state.order[0];
    let a = apply(&mut doc, squash, vec![bottom], [[0.5, 0.0], [0.5, 1.0]], [0.5, 0.1])
        .expect("1回目の組み替え");
    apply(&mut doc, squash, vec![a.order[0]], [[0.0, 0.5], [1.0, 0.5]], [0.1, 0.5])
        .expect("2回目の組み替え");
    doc
}

// ---------------------------------------------------------------------------
// 花弁折り
// ---------------------------------------------------------------------------

/// M2の受け入れ条件(折り鶴の前提): 予備基本形の前面を花弁折りすると、
/// 先端が持ち上がって鶴の基本形の細い先(首・尾になる部分)ができる。
///
/// 予備基本形は footprint が [0,0.5]x[0.5,1.0] で、紙の4隅が (0,1)(開いた先端)、
/// 紙の中心が (0.5,0.5)(閉じた角)に来る。中心線はその2点を結ぶ線。
#[test]
fn petal_lifts_the_front_of_the_bird_base() {
    let mut doc = preliminary_base();
    let faces = extract_faces(&doc.cp);
    let (before, _) = flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
    let front = *before.order.last().expect("最前面");
    let edges_before = doc.cp.edges.len();

    let a = apply(&mut doc, petal, vec![front], [[0.0, 1.0], [0.5, 0.5]], [0.0, 1.0])
        .expect("前面を花弁折りできる");
    assert!(a.warnings.is_empty(), "紙は裂けない: {:?}", a.warnings);

    // 前面が「羽2枚・中央のくさび・動かない部分」の4面に、
    // 一緒に開く隣の2層がそれぞれ2面に分かれる(4+2+2+1=9面)
    assert_eq!(a.faces.len(), 9, "前面4面・隣の層2面ずつ・残り1層");
    assert_eq!(a.order.len(), 9);
    assert_eq!(doc.cp.edges.len(), edges_before + 7, "折り線5本と輪郭の分割2本");

    let step = &doc.sequence[doc.sequence.len() - 1];
    assert_eq!(step.kind, TechniqueKind::Petal);
    // 動かす折り線は7本: 前面の3本(斜め2本+ちょうつがい)、隣の層の斜め2本、
    // そして開く折り目2本(前面の羽と隣の層の羽をつないでいた縁)
    assert_eq!(step.drivers.len(), 7, "駆動する折り線: {:?}", step.drivers);
    let opened = step
        .drivers
        .iter()
        .filter(|dr| dr.target_angle_deg == 0.0)
        .count();
    assert_eq!(opened, 2, "開く折り目は0°で記録される: {:?}", step.drivers);
    assert_eq!(
        step.drivers
            .iter()
            .filter(|dr| dr.target_angle_deg.abs() == 180.0)
            .count(),
        5,
        "残りの5本は折り上がり180°: {:?}",
        step.drivers
    );

    // 先端(紙の4隅が来ていた (0,1))が中心線に沿って持ち上がり、
    // 閉じた角 (0.5,0.5) を越えて外へ出る(鶴の細い先)
    let tip = DVec2::new(0.0, 1.0);
    let apex = DVec2::new(0.5, 0.5);
    let axis = (apex - tip).normalize();
    let faces = extract_faces(&doc.cp);
    let (after, warnings) =
        flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
    assert!(warnings.is_empty(), "{warnings:?}");
    let pos: HashMap<u32, DVec2> =
        doc.cp.vertices.iter().map(|v| (v.id, DVec2::from(v.pos))).collect();
    let reach_of = |f: &Face| -> f64 {
        f.vertices
            .iter()
            .filter_map(|v| pos.get(v))
            .map(|&p| axis.dot(after.placements[&f.id].apply(p) - tip))
            .fold(f64::NEG_INFINITY, f64::max)
    };
    let far = faces.iter().map(reach_of).fold(f64::NEG_INFINITY, f64::max);
    assert!(
        (far - 1.0).abs() < 1e-6,
        "先端が中心線に沿って1.0まで持ち上がる(閉じた角は{}): {far}",
        (apex - tip).length()
    );
    // 持ち上がった紙は重なりのいちばん上に来る(鶴の基本形の前面)
    let top = *a.order.last().expect("最上層");
    let top_face = faces.iter().find(|f| f.id == top).expect("最上層の面");
    assert!(
        reach_of(top_face) > 0.5 + 1e-9,
        "最前面がちょうつがい(先端から0.5)より先へ出ている"
    );

    // 記録した手順から同じ形に折り直せる
    assert_eq!(after.order, a.order, "層順序が再生結果と一致する");
    for f in &faces {
        assert!(
            a.at[&f.id].abs_diff_eq(after.placements[&f.id].apply(DVec2::from(a.rep[&f.id])), 1e-6),
            "面 {} の位置が再生結果と一致する",
            f.id
        );
    }
    assert_fold_senses(&doc, "花弁折り");
}

/// 左右の縁の長さが違うフラップでも花弁折りできる。
///
/// 長方形の紙(2:1)を1回折った2層。畳んだ形は [0,1]x[0,0.25] の長方形で、
/// 背(開く折り目)は y=0.25、残りの3辺は紙の縁。
fn rectangular_two_layer_flap() -> Document {
    let mut doc = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 50.0,
    });
    fold(&mut doc, [[0.0, 0.25], [1.0, 0.25]], [0.5, 0.125], FoldDirection::Up);
    doc
}

/// 先端から見た左右の縁の長さが違うフラップでも花弁折りできる。
///
/// 先端は角 (0,0)、中心線はそこから対角の (1,0.25) へ引く。先端から見た縁は
/// 片側が長さ 1.0(y=0 の辺)、もう片側が 0.25(x=0 の辺)と違う。
/// 実際の紙では左右で別の折り線(中心線に直交しない1本の斜めのちょうつがい)に
/// なるだけで平らに畳めるので、裂ける警告は出ない。
#[test]
fn petal_on_a_flap_with_different_edge_lengths() {
    let mut doc = rectangular_two_layer_flap();
    let flap: Vec<FaceId> = extract_faces(&doc.cp).iter().map(|f| f.id).collect();
    let a = apply(&mut doc, petal, flap, [[0.0, 0.0], [1.0, 0.25]], [0.0, 0.0])
        .expect("非対称なフラップでも花弁折りできる");
    assert!(a.warnings.is_empty(), "紙は裂けない: {:?}", a.warnings);

    let faces = extract_faces(&doc.cp);
    let (after, warnings) =
        flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(after.order, a.order, "層順序が再生結果と一致する");
    for f in &faces {
        assert!(
            a.at[&f.id].abs_diff_eq(after.placements[&f.id].apply(DVec2::from(a.rep[&f.id])), 1e-6),
            "面 {} の位置が再生結果と一致する",
            f.id
        );
    }
    assert_fold_senses(&doc, "非対称な花弁折り");
}

/// 花弁折りが断るのは幾何的に決められない入力だけ。断ったときは文書を変えない。
#[test]
fn petal_rejects_undefined_input_without_touching_document() {
    let mut doc = preliminary_base();
    let before = doc.clone();
    let faces = extract_faces(&doc.cp);
    let (state, _) = flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
    let front = *state.order.last().expect("最前面");

    // 中心線が退化(2点が一致)
    let err = apply(&mut doc, petal, vec![front], [[0.5, 0.5], [0.5, 0.5]], [0.0, 1.0])
        .expect_err("退化した中心線はエラー");
    assert!(err.contains("2点が一致"), "{err}");
    assert_eq!(doc, before, "失敗時は文書を変更しない");

    // フラップの指定が無い
    let err = apply(&mut doc, petal, Vec::new(), [[0.0, 1.0], [0.5, 0.5]], [0.0, 1.0])
        .expect_err("フラップ未指定はエラー");
    assert!(err.contains("フラップ"), "{err}");
    assert_eq!(doc, before);

    // 無い層を指定した
    let missing = faces.iter().map(|f| f.id).max().unwrap_or(0) + 99;
    let err = apply(&mut doc, petal, vec![missing], [[0.0, 1.0], [0.5, 0.5]], [0.0, 1.0])
        .expect_err("無い層はエラー");
    assert!(err.contains("見つかりません"), "{err}");
    assert_eq!(doc, before);

    // 中心線がフラップの縁に重なっていて片側にしか紙が無い指定は、断らずに
    // 警告して続ける(「止めずに警告」原則)
    let a = apply(&mut doc, petal, vec![front], [[0.0, 0.5], [0.5, 0.5]], [0.0, 0.5])
        .expect("片側だけでも折れる");
    assert!(
        a.warnings.iter().any(|w| w.contains("中心線の横まで広がって")),
        "無理のある指定は警告になる: {:?}",
        a.warnings
    );
    assert_ne!(doc, before, "警告付きでも折りは適用される");
}



// ---------------------------------------------------------------------------
// 沈め折り
// ---------------------------------------------------------------------------

/// 鶴の基本形。予備基本形の前面と背面を1回ずつ花弁折りする
/// (`acceptance_crane.rs` の `bird_base` と同じ手順)。
fn bird_base() -> Document {
    let mut doc = preliminary_base();
    let center_line = [[0.0, 1.0], [0.5, 0.5]];
    let tip = [0.0, 1.0];
    let faces = extract_faces(&doc.cp);
    let (state, _) = flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
    let front = vec![*state.order.last().expect("最前面")];
    apply(&mut doc, petal, front, center_line, tip).expect("前面の花弁折り");

    // 背面 = 前面の花弁折りで分かれなかった層(畳んだ正方形の両脇の角を両方持つ層)
    let faces = extract_faces(&doc.cp);
    let (state, _) = flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
    let has = |f: &Face, p: DVec2| {
        let pl = state.placements[&f.id];
        let pos: HashMap<u32, DVec2> =
            doc.cp.vertices.iter().map(|v| (v.id, DVec2::from(v.pos))).collect();
        f.vertices
            .iter()
            .filter_map(|v| pos.get(v))
            .any(|&q| (pl.apply(q) - p).length() < 1e-9)
    };
    let back: Vec<FaceId> = faces
        .iter()
        .filter(|f| has(f, DVec2::new(0.5, 1.0)) && has(f, DVec2::new(0.0, 0.5)))
        .map(|f| f.id)
        .collect();
    assert_eq!(back.len(), 1, "背面はまだ1枚(実際 {back:?})");
    apply_input(
        &mut doc,
        petal,
        TechniqueInput {
            flap: back,
            line: center_line,
            reference_point: tip,
            open_to_back: Some(true),
        },
    )
    .expect("背面の花弁折り");
    doc
}

/// 沈め折りの基本: 予備基本形の閉じた角(紙の中心が来ている (0.5,0.5))を沈める。
///
/// 紙は1mmも動かず、折り線より先端側の4層が入れ子の内外を入れ替える。
/// 先端側の折り目は山谷が反転し、外側の紙は元のまま。
#[test]
fn open_sink_turns_the_tip_of_the_preliminary_base_inside_out() {
    let mut doc = preliminary_base();
    let faces = extract_faces(&doc.cp);
    let (before, _) = flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
    let before_kinds = kinds(&doc.cp);
    let edges_before = doc.cp.edges.len();

    // 閉じた角 (0.5,0.5) を切り取る線(先端側は小さな三角形)
    let line = [[0.5, 0.6], [0.4, 0.5]];
    let a = apply(&mut doc, open_sink, Vec::new(), line, [0.47, 0.53])
        .expect("閉じた角を沈められる");
    assert!(a.warnings.is_empty(), "紙は裂けない: {:?}", a.warnings);

    // 4層それぞれが先端の三角形と残りに分かれる
    assert_eq!(a.faces.len(), 8, "4層が先端と胴に分かれる");
    assert_eq!(a.order.len(), 8);
    assert_eq!(doc.cp.edges.len(), edges_before + 8, "折り線4本と輪郭の分割4本");
    let step = &doc.sequence[doc.sequence.len() - 1];
    assert_eq!(step.kind, TechniqueKind::OpenSink);

    // 紙は1mmも動かない(全ての新しい面が親の配置のまま)
    let after_faces = extract_faces(&doc.cp);
    for f in &after_faces {
        let r = representative_point(&doc.cp, f);
        let parent = faces
            .iter()
            .find(|p| ori3_layers::point_in_face(&doc.cp, p, r))
            .map(|p| p.id);
        if let Some(p) = parent {
            assert!(
                a.at[&f.id].abs_diff_eq(before.placements[&p].apply(DVec2::from(r)), 1e-12),
                "面 {} は動かない",
                f.id
            );
        }
    }

    // 先端側の4枚は重なり順が逆(沈め込み後の入れ子順)になり、胴側は元の順のまま
    let parent_of = |id: FaceId| -> FaceId {
        let f = after_faces.iter().find(|f| f.id == id).expect("面がある");
        let r = representative_point(&doc.cp, f);
        faces
            .iter()
            .find(|p| ori3_layers::point_in_face(&doc.cp, p, r))
            .expect("親の面")
            .id
    };
    let side = |id: FaceId| -> f64 {
        let d = DVec2::from(line[1]) - DVec2::from(line[0]);
        d.perp_dot(a.at[&id] - DVec2::from(line[0]))
    };
    let tip: Vec<FaceId> = a.order.iter().copied().filter(|&id| side(id) > 0.0).collect();
    let body: Vec<FaceId> = a.order.iter().copied().filter(|&id| side(id) < 0.0).collect();
    assert_eq!(tip.len(), 4, "先端側は4枚");
    assert_eq!(body.len(), 4, "胴側は4枚");
    let rank = |id: FaceId| before.order.iter().position(|&p| p == parent_of(id)).unwrap();
    let tip_ranks: Vec<usize> = tip.iter().map(|&id| rank(id)).collect();
    let body_ranks: Vec<usize> = body.iter().map(|&id| rank(id)).collect();
    assert_eq!(body_ranks, vec![0, 1, 2, 3], "胴側は元の重なり順のまま");
    assert_eq!(tip_ranks, vec![3, 2, 1, 0], "先端側は内外が入れ替わる");

    // 山谷も反転する
    let after_kinds = kinds(&doc.cp);
    assert!(
        changed_kinds(&before_kinds, &after_kinds) > 0,
        "先端側の折り目の山谷が入れ替わる"
    );
    // 折り終わる直前(t=0.99)の高さによる検証は沈め折りには使えない:
    // 沈め折りは紙が1mmも動かない遷移(重なり順と山谷だけが変わる)なので、
    // 補間の途中でも層は浮かず、高さの差が読み取れない。代わりに、折り目の
    // 向き(山谷)と層順序の一致 `assert_fold_senses` で重なりを確かめる。
    assert_fold_senses(&doc, "沈め折り");

    // 記録した手順から同じ形に折り直せる
    let (again, warnings) =
        flat_state_at(&doc, &after_faces, doc.sequence.len()).expect("平らに畳める");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(again.order, a.order, "層順序が再生結果と一致する");
    for f in &after_faces {
        assert!(
            a.at[&f.id].abs_diff_eq(again.placements[&f.id].apply(DVec2::from(a.rep[&f.id])), 1e-6),
            "面 {} の位置が再生結果と一致する",
            f.id
        );
    }
}

/// 鶴の基本形の頂点(首・尾の付け根になる閉じた角)を沈める。
///
/// 層の数が予備基本形より多く、層のつながりも複雑だが、同じ1回の動きで沈む。
#[test]
fn open_sink_works_on_the_bird_base_apex() {
    let mut doc = bird_base();
    let faces = extract_faces(&doc.cp);
    let (before, _) = flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
    let layers_before = before.order.len();

    // 閉じた角 (0.5,0.5) を切り取る線(鶴の基本形でも紙の中心はここにある)
    let line = [[0.5, 0.6], [0.4, 0.5]];
    let a = apply(&mut doc, open_sink, Vec::new(), line, [0.47, 0.53])
        .expect("鶴の基本形の頂点を沈められる");

    assert!(a.order.len() > layers_before, "先端側の層が分割される");
    let step = &doc.sequence[doc.sequence.len() - 1];
    assert_eq!(step.kind, TechniqueKind::OpenSink);
    assert!(!step.drivers.is_empty(), "沈めた折り線が手順に記録される");

    // 先端側は重なり順が逆(内外反転)になる
    let side = |id: FaceId| -> f64 {
        let d = DVec2::from(line[1]) - DVec2::from(line[0]);
        d.perp_dot(a.at[&id] - DVec2::from(line[0]))
    };
    let parent_rank = |id: FaceId| -> usize {
        let f = a.faces.iter().find(|f| f.id == id).expect("面がある");
        let r = representative_point(&doc.cp, f);
        let p = faces
            .iter()
            .find(|p| ori3_layers::point_in_face(&doc.cp, p, r))
            .expect("親の面");
        before.order.iter().position(|&q| q == p.id).unwrap()
    };
    let tip: Vec<usize> = a
        .order
        .iter()
        .copied()
        .filter(|&id| side(id) > 0.0)
        .map(parent_rank)
        .collect();
    assert!(tip.len() >= 2, "先端側に複数の層がある");
    let mut sorted = tip.clone();
    sorted.sort_unstable();
    sorted.reverse();
    assert_eq!(tip, sorted, "先端側は内外が入れ替わる(実際 {tip:?})");
    assert_fold_senses(&doc, "鶴の基本形の沈め折り");

    // 記録した手順から同じ形に折り直せる
    let after_faces = extract_faces(&doc.cp);
    let (again, _) =
        flat_state_at(&doc, &after_faces, doc.sequence.len()).expect("平らに畳める");
    assert_eq!(again.order, a.order, "層順序が再生結果と一致する");
}

/// 沈め折りは重なりの一部の層だけでもできる(層の数の偶奇も問わない)。
/// 定義できない入力だけをErrで断り、そのとき文書は一切変わらない。
#[test]
fn open_sink_accepts_partial_flaps_and_rejects_only_undefined_input() {
    let mut doc = preliminary_base();
    let before_doc = doc.clone();
    let faces = extract_faces(&doc.cp);
    let (state, _) = flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
    let line = [[0.5, 0.6], [0.4, 0.5]];

    // 退化した折り線
    let err = apply(&mut doc, open_sink, Vec::new(), [[0.5, 0.5], [0.5, 0.5]], [0.47, 0.53])
        .expect_err("退化した折り線はエラー");
    assert!(err.contains("2点が一致"), "{err}");
    assert_eq!(doc, before_doc, "失敗時は文書を変更しない");

    // 基準点が折り線の上
    let err = apply(&mut doc, open_sink, Vec::new(), line, [0.45, 0.55])
        .expect_err("沈める側が決まらない指定はエラー");
    assert!(err.contains("折り線の上"), "{err}");
    assert_eq!(doc, before_doc);

    // 無い層
    let missing = faces.iter().map(|f| f.id).max().unwrap_or(0) + 99;
    let err = apply(&mut doc, open_sink, vec![missing], line, [0.47, 0.53])
        .expect_err("無い層はエラー");
    assert!(err.contains("見つかりません"), "{err}");
    assert_eq!(doc, before_doc);

    // 一部の層(奇数枚)だけを沈める: 断らずに折れる
    let some: Vec<FaceId> = state.order[..3].to_vec();
    let a = apply(&mut doc, open_sink, some, line, [0.47, 0.53])
        .expect("3層(奇数)だけでも沈められる");
    assert_eq!(a.faces.len(), 7, "選んだ3層だけが先端と胴に分かれる");
    assert_ne!(doc, before_doc, "折りは適用される");
}
