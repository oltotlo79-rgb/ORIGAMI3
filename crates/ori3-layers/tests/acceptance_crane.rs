//! Task 2-9: M2受け入れテスト — 折り鶴。
//!
//! **アプリが提供する折り操作の列だけ**で折り鶴を完成させ、回帰テストとして固定する。
//! 手作業の展開図編集は一切せず、次の操作だけを使う:
//!
//! - 重ね折り `fold_through`(半分に折る・羽を下げる)
//! - 開いてつぶす折り `squash`(予備基本形への組み替え)
//! - 花弁折り `petal`(前面と背面で1回ずつ→鶴の基本形)
//! - 中割り折り `inside_reverse`(首・尾・頭)
//!
//! # 折り順(標準的な折り鶴の手順そのまま)
//!
//! 1. 正方形を半分に2回折り、開いてつぶす折り2回で**予備基本形**(4層が輪につながる)
//! 2. 前面を花弁折り、背面を花弁折り(`open_to_back`)→ **鶴の基本形**
//! 3. 細い先を左右へ中割り折り → 首と尾
//! 4. 首の先をもう一度中割り折り → 頭
//! 5. 広いフラップを前後へ倒す → 羽
//!
//! # 座標について
//!
//! 畳み平面の座標は1手ごとに全体の等長変換だけずれる(根面をそろえ直すため)。
//! そのため完成形の検証は、紙の4隅と中心が畳み平面のどこに来たかを見て、
//! **中心から見た距離と角度**という座標系に依らない量で行う。
//!
//! # 表示上の重なり順
//!
//! 折り終わる直前(t=0.99)の高さから読んだ本当の上下と層順序が一致することを
//! [`assert_display_order`] で確かめる(`display_order.rs` と同じ方式)。予備基本形
//! までの工程(半分に折る・開いてつぶす)はこの方式で検証する。
//!
//! 花弁折りと中割り折りの後はこの方式が使えない。どちらも**既にある折り目を開く/
//! 裏返す**動きを含み、手順再生は前の手順の折り目を180°に固定したまま補間するため、
//! 開く途中の紙が高さ0をまたいで大回りする(理由の詳しい説明は `techniques.rs` の
//! テスト冒頭にある)。折り上がり(t=1)の形と重なりは正しいので、これらの工程は
//! 折り目の山谷と層順序の一致([`assert_fold_senses`])で確かめる。

use std::collections::HashMap;

use glam::{DVec2, DVec3};
use ori3_cp::{Face, extract_faces};
use ori3_layers::flat_state::representative_point;
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, fold_through};
use ori3_layers::techniques::TechniqueInput;
use ori3_layers::{
    FlatState, FoldThroughResult, flat_state_at, inside_reverse, petal, replay, squash,
};
use ori3_model::{CreasePattern, Document, EdgeKind, FaceId, Paper};

/// 紙の中心から細い先までの距離(鶴の基本形。1 - √2/2)。
const CORE: f64 = 1.0 - 0.5 * std::f64::consts::SQRT_2;

type Technique = fn(
    &mut CreasePattern,
    &[Face],
    &FlatState,
    &TechniqueInput,
) -> Result<FoldThroughResult, String>;

/// 単位正方形の紙。
fn square_doc() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

/// store.rs の SeqOp::FoldThrough と同じ手順で1手折る(重なりの一部だけを折るときは
/// `target_layers` を渡す)。戻り値は折った直後の平坦状態と警告。
fn fold_layers(
    doc: &mut Document,
    line: [[f64; 2]; 2],
    keep: [f64; 2],
    target_layers: Option<Vec<FaceId>>,
    direction: FoldDirection,
) -> (FlatState, Vec<String>) {
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
            target_layers,
            direction,
        },
    )
    .expect("折れる指定");
    let mut step = res.step;
    step.id = u32::try_from(up_to).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);
    (res.state, res.warnings)
}

/// 全ての層をまとめて折る版。
fn fold(doc: &mut Document, line: [[f64; 2]; 2], keep: [f64; 2], direction: FoldDirection) {
    let (_, warnings) = fold_layers(doc, line, keep, None, direction);
    assert!(warnings.is_empty(), "警告なしで折れる: {warnings:?}");
}

/// store.rs の SeqOp::Technique と同じ手順で技法を1回適用する。
/// `open_to_back` は動かした紙を重なりのどちら側へ回すか(つぶし折り・花弁折り)。
fn apply(
    doc: &mut Document,
    technique: Technique,
    flap: Vec<FaceId>,
    line: [[f64; 2]; 2],
    reference_point: [f64; 2],
    open_to_back: Option<bool>,
) -> FlatState {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, _) = flat_state_at(doc, &faces, up_to).expect("平らな状態から折る");
    let mut cp = doc.cp.clone();
    let res = technique(
        &mut cp,
        &faces,
        &state,
        &TechniqueInput {
            flap,
            line,
            reference_point,
            open_to_back,
            polygon: None,
            center: None,
        },
    )
    .expect("折れる指定");
    assert!(
        res.warnings.is_empty(),
        "警告なしで折れる: {:?}",
        res.warnings
    );
    let mut step = res.step;
    step.id = u32::try_from(up_to).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);
    res.state
}

/// 現在の畳んだ状態(面・配置・層順序)。警告が出たら失敗させる。
fn state_of(doc: &Document) -> (Vec<Face>, FlatState) {
    let faces = extract_faces(&doc.cp);
    let (state, warnings) = flat_state_at(doc, &faces, doc.sequence.len()).expect("平らに畳める");
    assert!(warnings.is_empty(), "再生の警告: {warnings:?}");
    (faces, state)
}

/// 展開図の頂点の位置(頂点ID→座標)。
fn vertex_pos(cp: &CreasePattern) -> HashMap<u32, DVec2> {
    cp.vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect()
}

/// 面が畳み平面で占める多角形。
fn plane_poly(cp: &CreasePattern, f: &Face, state: &FlatState) -> Vec<DVec2> {
    let pos = vertex_pos(cp);
    let pl = state.placements[&f.id];
    f.vertices
        .iter()
        .filter_map(|v| pos.get(v).copied())
        .map(|p| pl.apply(p))
        .collect()
}

/// 畳んだ形の外形(minx, miny, maxx, maxy)。
fn bbox(doc: &Document) -> [f64; 4] {
    let (faces, state) = state_of(doc);
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for f in &faces {
        for p in plane_poly(&doc.cp, f, &state) {
            b[0] = b[0].min(p.x);
            b[1] = b[1].min(p.y);
            b[2] = b[2].max(p.x);
            b[3] = b[3].max(p.y);
        }
    }
    b
}

/// 展開図の点 `cp` が畳み平面のどこへ来たか(重なった同じ位置は1つにまとめる)。
fn folded_to(doc: &Document, cp: [f64; 2]) -> Vec<DVec2> {
    let (faces, state) = state_of(doc);
    let pos = vertex_pos(&doc.cp);
    let target = DVec2::from(cp);
    let mut out: Vec<DVec2> = Vec::new();
    for f in &faces {
        for v in f.vertices.iter().filter_map(|v| pos.get(v)) {
            if (*v - target).length() < 1e-9 {
                let q = state.placements[&f.id].apply(*v);
                if !out.iter().any(|p| (*p - q).length() < 1e-6) {
                    out.push(q);
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 層の選び方(UIで紙をつかむ操作に対応する)
// ---------------------------------------------------------------------------

/// 多角形の内部(境界を含む)に点があるか。
fn inside_polygon(poly: &[DVec2], p: DVec2) -> bool {
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let ab = b - a;
        let t = if ab.length_squared() == 0.0 {
            0.0
        } else {
            ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
        };
        if (p - (a + ab * t)).length() <= 1e-9 {
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

/// 畳み平面の形についての条件で層を選ぶ(下→上)。
fn pick(doc: &Document, want: impl Fn(&[DVec2]) -> bool) -> Vec<FaceId> {
    let (faces, state) = state_of(doc);
    state
        .order
        .iter()
        .copied()
        .filter(|id| {
            let f = faces.iter().find(|f| f.id == *id).expect("層順序の面");
            want(&plane_poly(&doc.cp, f, &state))
        })
        .collect()
}

/// 畳み平面で点 `p` を角に持つ層(=その先端を作っている紙)を下から順に返す。
fn layers_tipped_at(doc: &Document, p: DVec2) -> Vec<FaceId> {
    pick(doc, |poly| poly.iter().any(|q| (*q - p).length() < 1e-6))
}

/// 展開図の四角い範囲にすっぽり入っている紙(=紙の1/4=1枚の羽)の層を下から順に返す。
fn layers_in_quarter(doc: &Document, b: [f64; 4]) -> Vec<FaceId> {
    let (faces, state) = state_of(doc);
    let pos = vertex_pos(&doc.cp);
    state
        .order
        .iter()
        .copied()
        .filter(|id| {
            let f = faces.iter().find(|f| f.id == *id).expect("層順序の面");
            f.vertices.iter().filter_map(|v| pos.get(v)).all(|p| {
                p.x >= b[0] - 1e-9 && p.x <= b[2] + 1e-9 && p.y >= b[1] - 1e-9 && p.y <= b[3] + 1e-9
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 表示上の重なり順の検証(t=0.99の高さから読み取る。display_order.rs と同じ方式)
// ---------------------------------------------------------------------------

/// 面の「展開図の点 → 折り終わる直前(t=0.99)の3D位置」を与える剛体変換。
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
        let pos = vertex_pos(cp);
        let pts: Vec<DVec2> = face
            .vertices
            .iter()
            .filter_map(|v| pos.get(v).copied())
            .collect();
        if pts.len() != face.vertices.len() || pts.len() != polygon.len() || pts.len() < 3 {
            return None;
        }
        let p0 = pts[0];
        let q0 = DVec3::from(polygon[0]);
        for i in 1..pts.len() {
            for j in (i + 1)..pts.len() {
                let (e1, e2) = (pts[i] - p0, pts[j] - p0);
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

/// 記録した層順序が「画面に見える重なり」と一致することを確かめる。
///
/// 平らな状態で重なっている面の組を選び、折り終わる直前(t=0.99)の高さの大小が
/// 層順序と同じ向きかを1組ずつ突き合わせる。高さの差がごく小さい組(まだ動いて
/// いない層どうし)は上下を読めないので飛ばす。
fn assert_display_order(doc: &Document, label: &str) {
    const READABLE: f64 = 1e-4;
    let (faces, state) = state_of(doc);
    let plane: HashMap<FaceId, Vec<DVec2>> = faces
        .iter()
        .map(|f| (f.id, plane_poly(&doc.cp, f, &state)))
        .collect();
    let frame = replay(doc, doc.sequence.len(), 0.99).frame;
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
        for g in faces.iter().filter(|g| g.id != f.id) {
            if !inside_polygon(&plane[&g.id], q) {
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
/// 折り目でつながった2面の上下は、その折り目を表から見たときの山谷で決まる
/// (表向きの面から見て谷なら相手は上、山なら下)。
fn assert_fold_senses(doc: &Document, label: &str) {
    let (faces, state) = state_of(doc);
    let mut edge_faces: HashMap<u32, Vec<FaceId>> = HashMap::new();
    for f in faces {
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

/// 折り途中(t<1)も含めて、折り目でつながった面が離れていないことを確かめる。
///
/// 折り目の両端の頂点を、その折り目を共有する2つの面それぞれの3D多角形から読み、
/// 同じ位置に来ているかを見る。離れていたら紙がちぎれている(内部頂点まわりの
/// ループ閉包が破れている)。
/// 折り目でつながった2面の、その折り目の端点の3D位置のずれの最大値。
pub fn tear_of(doc: &Document, faces: &[Face], frame: &ori3_model::Frame3D) -> f64 {
    let poly: HashMap<FaceId, &Vec<[f64; 3]>> =
        frame.faces.iter().map(|f| (f.face, &f.polygon)).collect();
    let mut edge_faces: HashMap<u32, Vec<&Face>> = HashMap::new();
    for f in faces {
        let mut ids = f.edges.clone();
        ids.sort_unstable();
        ids.dedup();
        for eid in ids {
            edge_faces.entry(eid).or_default().push(f);
        }
    }
    let mut worst = 0.0f64;
    for e in &doc.cp.edges {
        let Some(fs) = edge_faces.get(&e.id) else {
            continue;
        };
        if fs.len() != 2 || fs[0].id == fs[1].id {
            continue;
        }
        for v in [e.v0, e.v1] {
            let pts: Vec<DVec3> = fs
                .iter()
                .filter_map(|f| {
                    let i = f.vertices.iter().position(|&x| x == v)?;
                    Some(DVec3::from(poly.get(&f.id)?[i]))
                })
                .collect();
            if pts.len() == 2 {
                worst = worst.max((pts[0] - pts[1]).length());
            }
        }
    }
    worst
}

/// 折り上がりが平ら(全ての面がz=0に乗る)ことを確かめる。
fn assert_flat(doc: &Document, label: &str) {
    let result = replay(doc, doc.sequence.len(), 1.0);
    assert!(
        result.warnings.is_empty(),
        "{label}: 再生の警告 {:?}",
        result.warnings
    );
    assert!(
        result.skipped.is_empty(),
        "{label}: 飛ばした手順 {:?}",
        result.skipped
    );
    assert!(
        result
            .frame
            .faces
            .iter()
            .all(|f| f.polygon.iter().all(|p| p[2].abs() < 1e-6)),
        "{label}: 折り上がりは平ら"
    );
}

// ---------------------------------------------------------------------------
// 折り鶴を折る
// ---------------------------------------------------------------------------

/// 予備基本形。正方形を半分に2回折り、開いてつぶす折り2回で層のつながりを
/// 輪へ組み替える(`techniques.rs` の `squash_reorders_layers_without_moving_paper`
/// と同じ手順)。畳んだ形は [0,0.5]x[0.5,1] の正方形で、紙の4隅が開いた先端
/// (0,1) に、紙の中心が閉じた角 (0.5,0.5) に来る。
fn preliminary_base() -> Document {
    let mut doc = square_doc();
    fold(
        &mut doc,
        [[0.0, 0.5], [1.0, 0.5]],
        [0.5, 0.25],
        FoldDirection::Up,
    );
    fold(
        &mut doc,
        [[0.5, 0.0], [0.5, 0.5]],
        [0.25, 0.25],
        FoldDirection::Up,
    );
    for (line, reference) in [
        ([[0.5, 0.0], [0.5, 1.0]], [0.5, 0.1]),
        ([[0.0, 0.5], [1.0, 0.5]], [0.1, 0.5]),
    ] {
        let (_, state) = state_of(&doc);
        let bottom = vec![state.order[0]];
        apply(&mut doc, squash, bottom, line, reference, None);
    }
    doc
}

/// 鶴の基本形。予備基本形の前面と背面を1回ずつ花弁折りする。
///
/// 背面は `open_to_back` で向こう側へ開く(実際の紙で裏返して折るのと同じ)。
/// 手前に決め打ちすると、持ち上げた紙が前面の花弁の上に回ってしまい、
/// 紙の重なりとして成り立たない。
fn bird_base() -> Document {
    let mut doc = preliminary_base();
    let center_line = [[0.0, 1.0], [0.5, 0.5]];
    let tip = [0.0, 1.0];

    let (_, state) = state_of(&doc);
    let front = vec![*state.order.last().expect("最前面")];
    apply(&mut doc, petal, front, center_line, tip, None);

    // 背面=前面の花弁折りで分かれなかった層。畳んだ正方形の両脇の角
    // (0,0.5) と (0.5,1) の両方を持つのはこの層だけ
    let side_b = layers_tipped_at(&doc, DVec2::new(0.5, 1.0));
    let back: Vec<FaceId> = layers_tipped_at(&doc, DVec2::new(0.0, 0.5))
        .into_iter()
        .filter(|id| side_b.contains(id))
        .collect();
    assert_eq!(back.len(), 1, "背面はまだ1枚のまま(実際 {back:?})");
    apply(&mut doc, petal, back, center_line, tip, Some(true));
    doc
}

/// 折り鶴。鶴の基本形の細い先を首と尾にし、首の先を頭にして、羽を下げる。
/// 戻り値は文書と、最後の操作が返した平坦状態(再生一致の検証に使う)。
fn crane() -> (Document, FlatState) {
    let mut doc = bird_base();
    let center = DVec2::new(0.0, CORE); // 閉じた角(紙の中心)=首と尾の付け根
    let (down15, right15) = (-15.0_f64).to_radians().sin_cos();

    // 首と尾: 重なった2本の細い先を、付け根を通る折り線で左右へ振り分ける。
    // 折り線が中心線と±15°をなすので、先端は真下(-90°)から60°/120°へ回る
    let points = layers_tipped_at(&doc, DVec2::ZERO);
    assert_eq!(points.len(), 6, "細い先は2本×2層=6面(実際 {points:?})");
    apply(
        &mut doc,
        inside_reverse,
        points[3..].to_vec(),
        [[center.x, center.y], [right15, center.y + down15]],
        [0.2, 0.5],
        None,
    );
    let tail = layers_tipped_at(&doc, DVec2::ZERO);
    assert_eq!(tail.len(), 3, "残った細い先は1本=3面(実際 {tail:?})");
    apply(
        &mut doc,
        inside_reverse,
        tail,
        [[center.x, center.y], [right15, center.y - down15]],
        [-0.2, 0.5],
        None,
    );

    // 頭: 首の先(60°方向、付け根から CORE)をもう一度中割り折りして-30°へ向ける
    let up60 = DVec2::new(0.5, 0.75_f64.sqrt());
    let hinge = center + up60 * (0.75 * CORE);
    let head = layers_tipped_at(&doc, center + up60 * CORE);
    assert_eq!(head.len(), 3, "首の先は2層=3面(実際 {head:?})");
    apply(
        &mut doc,
        inside_reverse,
        head,
        [[hinge.x, hinge.y], [hinge.x + right15, hinge.y - down15]],
        [hinge.x + 0.0866, hinge.y - 0.05],
        None,
    );

    // 羽を下げる: 広いフラップ(紙の1/4ずつ)を肩の少し上で前後へ倒す。
    // 肩の線(y=0.5)そのものは羽を体につないでいる折り目なので折り直せない
    let (mut state, mut warnings) = (None, Vec::new());
    for (quarter, direction) in [
        ([0.5, 0.0, 1.0, 0.5], FoldDirection::Down),
        ([0.0, 0.5, 0.5, 1.0], FoldDirection::Up),
    ] {
        let wing = layers_in_quarter(&doc, quarter);
        assert_eq!(wing.len(), 3, "羽は紙の1/4=3面(実際 {wing:?})");
        let (s, w) = fold_layers(
            &mut doc,
            [[-1.0, 0.6], [1.0, 0.6]],
            [0.0, 0.3],
            Some(wing),
            direction,
        );
        state = Some(s);
        warnings = w;
    }
    assert!(
        warnings.is_empty(),
        "羽は警告なしで下げられる: {warnings:?}"
    );
    (doc, state.expect("最後の操作の平坦状態"))
}

/// 単位ベクトル(度)。
fn dir(deg: f64) -> DVec2 {
    let (s, c) = deg.to_radians().sin_cos();
    DVec2::new(c, s)
}

/// 2つのベクトルのなす角(度。0〜180)。
fn angle_between(a: DVec2, b: DVec2) -> f64 {
    a.angle_to(b).abs().to_degrees()
}

/// 展開図の点1つが畳み平面の1点へ来ていることを確かめ、その点を返す。
fn only(doc: &Document, cp: [f64; 2], label: &str) -> DVec2 {
    let ps = folded_to(doc, cp);
    assert_eq!(ps.len(), 1, "{label}: 1点に重なる(実際 {ps:?})");
    ps[0]
}

// ---------------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------------

/// 予備基本形: 4層が輪につながり、畳んだ形は紙の1/4の正方形。
#[test]
fn preliminary_base_folds_the_square_into_four_layers() {
    let doc = preliminary_base();
    let (faces, state) = state_of(&doc);
    assert_eq!(faces.len(), 4, "紙の1/4ずつの4層");
    assert_eq!(state.order.len(), 4);
    let b = bbox(&doc);
    for (got, want) in b.iter().zip([0.0, 0.5, 0.5, 1.0]) {
        assert!((got - want).abs() < 1e-9, "畳んだ形は正方形(実際 {b:?})");
    }
    // 紙の4隅は開いた先端に、紙の中心は閉じた角に集まる
    for corner in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
        let p = only(&doc, corner, "紙の隅");
        assert!(
            (p - DVec2::new(0.0, 1.0)).length() < 1e-9,
            "4隅は先端 (0,1)(実際 {p:?})"
        );
    }
    let apex = only(&doc, [0.5, 0.5], "紙の中心");
    assert!(
        (apex - DVec2::new(0.5, 0.5)).length() < 1e-9,
        "中心は閉じた角(実際 {apex:?})"
    );
    assert_display_order(&doc, "予備基本形");
    assert_fold_senses(&doc, "予備基本形");
    assert_flat(&doc, "予備基本形");
}

/// 鶴の基本形: 前後の花弁折りで細い先が2本でき、外形は45°のひし形になる。
#[test]
fn bird_base_lifts_two_slender_points() {
    let doc = bird_base();
    let (faces, state) = state_of(&doc);
    // 前面4面・背面4面・残り2層が3面ずつ
    assert_eq!(faces.len(), 14);
    assert_eq!(state.order.len(), 14);

    // 花弁折りした2隅(向かい合う隅)が細い先に、残る2隅は広いフラップの先に来る
    let slender = [only(&doc, [0.0, 0.0], "隅A"), only(&doc, [1.0, 1.0], "隅C")];
    assert!(
        (slender[0] - slender[1]).length() < 1e-9,
        "細い先2本は重なる"
    );
    let wide = [only(&doc, [1.0, 0.0], "隅B"), only(&doc, [0.0, 1.0], "隅D")];
    assert!(
        (wide[0] - wide[1]).length() < 1e-9,
        "広いフラップ2枚も重なる"
    );
    let apex = only(&doc, [0.5, 0.5], "紙の中心");

    // 中心から見て、細い先は CORE、広いフラップは √2/2 の距離で正反対を向く
    let a = slender[0] - apex;
    let b = wide[0] - apex;
    assert!(
        (a.length() - CORE).abs() < 1e-9,
        "細い先の長さ(実際 {})",
        a.length()
    );
    assert!(
        (b.length() - 0.5 * std::f64::consts::SQRT_2).abs() < 1e-9,
        "広いフラップの長さ(実際 {})",
        b.length()
    );
    assert!(
        (angle_between(a, b) - 180.0).abs() < 1e-6,
        "細い先と広いフラップは逆向き"
    );

    // 外形は先端の角が45°のひし形(全長は紙の1辺と同じ1.0)
    let bb = bbox(&doc);
    assert!((bb[3] - bb[1] - 1.0).abs() < 1e-9, "全長1.0(実際 {bb:?})");
    let half = 0.5 * (2.0_f64.sqrt() - 1.0);
    assert!(
        (bb[2] - half).abs() < 1e-9 && (bb[0] + half).abs() < 1e-9,
        "幅(実際 {bb:?})"
    );

    assert_fold_senses(&doc, "鶴の基本形");
    assert_flat(&doc, "鶴の基本形");
}

/// M2受け入れ条件: 折り鶴が折り操作の列だけで完成し、首・尾・頭・羽が
/// 期待した位置に来る。
#[test]
fn crane_is_folded_only_with_fold_operations() {
    let (doc, _) = crane();
    let (faces, state) = state_of(&doc);
    assert_eq!(faces.len(), 29);
    assert_eq!(state.order.len(), 29);
    // 手順は9手(半分2回・組み替え2回・花弁2回・中割り3回)+羽2回
    assert_eq!(doc.sequence.len(), 11, "折り操作は11手");

    let body = only(&doc, [0.5, 0.5], "紙の中心(首と尾の付け根)");
    let head = only(&doc, [0.0, 0.0], "頭の先") - body;
    let tail = only(&doc, [1.0, 1.0], "尾の先") - body;
    let wing_b = only(&doc, [1.0, 0.0], "羽の先B") - body;
    let wing_d = only(&doc, [0.0, 1.0], "羽の先D") - body;

    // 期待する形(付け根から見た向きと長さ)。細い先は真下(-90°)を向いていて、
    // 付け根を通る±15°の折り線で中割り折りすると 60°(首)と 120°(尾)へ回る。
    // 頭は首の 3/4 の位置でもう一度折り返して -30° へ向く。
    // 羽は y=0.6 の折り線で倒れるので、先端は付け根から 1.0-2*0.6 の側へ来る
    let want_tail = dir(120.0) * CORE;
    let want_head = (dir(60.0) * 0.75 + dir(-30.0) * 0.25) * CORE;
    let want_wing = dir(-90.0) * (CORE - 0.2);
    for (got, want, label) in [
        (tail, want_tail, "尾"),
        (head, want_head, "頭"),
        (wing_b, want_wing, "羽B"),
        (wing_d, want_wing, "羽D"),
    ] {
        assert!(
            (got.length() - want.length()).abs() < 1e-9,
            "{label}の長さ(期待 {}, 実際 {})",
            want.length(),
            got.length()
        );
    }
    assert!((wing_b - wing_d).length() < 1e-9, "左右の羽の先は重なる");
    // 向きの関係(座標系に依らない量)
    for (a, b, wa, wb, label) in [
        (head, tail, want_head, want_tail, "頭と尾"),
        (tail, wing_b, want_tail, want_wing, "尾と羽"),
        (head, wing_b, want_head, want_wing, "頭と羽"),
    ] {
        let (got, want) = (angle_between(a, b), angle_between(wa, wb));
        assert!(
            (got - want).abs() < 1e-6,
            "{label}のなす角(期待 {want:.4}°, 実際 {got:.4}°)"
        );
    }

    // 首と尾は付け根から左右に開き、羽は反対側へ倒れている
    assert!(
        (angle_between(head, tail) - 78.4356).abs() < 1e-3,
        "首と尾はV字に開く(実際 {:.4}°)",
        angle_between(head, tail)
    );

    // 中割り折りを含む形なので、表示の重なりは折り目の向きとの一致で確かめる
    // (t=0.99の高さ読みが使えない理由は techniques.rs のテスト冒頭を参照)
    assert_fold_senses(&doc, "折り鶴");
    assert_flat(&doc, "折り鶴");
}

/// 展開図と手順だけから同じ折り鶴に折り直せる(3D状態を保存しない設計の検証)。
#[test]
fn crane_replays_from_the_crease_pattern() {
    let (doc, built) = crane();
    let faces = extract_faces(&doc.cp);
    let (replayed, warnings) =
        flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
    assert!(warnings.is_empty(), "再生の警告: {warnings:?}");
    assert_eq!(replayed.order, built.order, "層順序が構築時と一致する");
    let pos = vertex_pos(&doc.cp);
    for f in &faces {
        let built_pl = built.placements[&f.id];
        let replay_pl = replayed.placements[&f.id];
        for p in f.vertices.iter().filter_map(|v| pos.get(v)) {
            assert!(
                (built_pl.apply(*p) - replay_pl.apply(*p)).length() < 1e-9,
                "面 {} の位置が構築時と一致する",
                f.id
            );
        }
    }
}

/// 折っている最中(t=0.25/0.5/0.75)も紙がつながったままであること。
/// 全ヒンジ角を線形補間しただけの値は内部頂点まわりのループ閉包を満たさず、
/// 面どうしが離れて紙がちぎれて見える(実機で報告された不具合の回帰テスト)。
#[test]
fn crane_paper_stays_connected_while_folding() {
    let (doc, _) = crane();
    let faces = extract_faces(&doc.cp);
    for up_to in 1..=doc.sequence.len() {
        for k in 1..=8 {
            let t = f64::from(k) / 8.0;
            let frame = replay(&doc, up_to, t).frame;
            let gap = tear_of(&doc, &faces, &frame);
            assert!(
                gap < 1e-6,
                "折り鶴(手順{up_to}, t={t}): 面が {gap:.9} 離れている"
            );
        }
    }
}

/// 同じ操作列は何度実行しても同じ結果になる(決定性)。
#[test]
fn crane_is_deterministic() {
    let (a, _) = crane();
    let (b, _) = crane();
    assert_eq!(a.cp, b.cp, "展開図が一致する");
    assert_eq!(a.sequence, b.sequence, "手順が一致する");
    let frame = |doc: &Document| format!("{:?}", replay(doc, doc.sequence.len(), 1.0).frame);
    assert_eq!(frame(&a), frame(&b), "折り上がりの3D姿勢がビット一致する");
}

// ---------------------------------------------------------------------------
// フロント側テスト用のフィクスチャ書き出し
// ---------------------------------------------------------------------------

/// 完成形の展開図と面を、フロント側(vitest)が読めるJSONとして書き出す。
/// 対称軸の判定(`apps/desktop/src/lib/grabDrive.ts`)を**実データ**で検証するため。
/// serde_jsonへ依存を増やさずに済むよう、必要な項目だけを手書きで出力する。
/// f64は `{:?}` で往復可能な最短表記になる。
pub fn write_fixture(doc: &Document, faces: &[Face], name: &str) {
    let mut s = String::from("{\n");
    let (w, h) = (doc.paper.width_mm, doc.paper.height_mm);
    s.push_str(&format!(
        "  \"paper\": {{ \"width_mm\": {w:?}, \"height_mm\": {h:?} }},\n"
    ));
    s.push_str("  \"vertices\": [\n");
    for (i, v) in doc.cp.vertices.iter().enumerate() {
        let (id, x, y) = (v.id, v.pos[0], v.pos[1]);
        let comma = if i + 1 < doc.cp.vertices.len() {
            ","
        } else {
            ""
        };
        s.push_str(&format!(
            "    {{ \"id\": {id}, \"pos\": [{x:?}, {y:?}] }}{comma}\n"
        ));
    }
    s.push_str("  ],\n  \"edges\": [\n");
    for (i, e) in doc.cp.edges.iter().enumerate() {
        let (id, v0, v1, kind) = (e.id, e.v0, e.v1, e.kind);
        let comma = if i + 1 < doc.cp.edges.len() { "," } else { "" };
        s.push_str(&format!(
            "    {{ \"id\": {id}, \"v0\": {v0}, \"v1\": {v1}, \"kind\": \"{kind:?}\" }}{comma}\n"
        ));
    }
    s.push_str("  ],\n  \"faces\": [\n");
    for (i, f) in faces.iter().enumerate() {
        let (id, vs, es) = (f.id, &f.vertices, &f.edges);
        let comma = if i + 1 < faces.len() { "," } else { "" };
        s.push_str(&format!(
            "    {{ \"id\": {id}, \"vertices\": {vs:?}, \"edges\": {es:?} }}{comma}\n"
        ));
    }
    s.push_str("  ]\n}\n");
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../apps/desktop/src/lib/__fixtures__"
    );
    std::fs::create_dir_all(dir).expect("フィクスチャ置き場を作る");
    std::fs::write(format!("{dir}/{name}.json"), s).expect("フィクスチャを書き出す");
}

/// 折り鶴の完成形をフィクスチャとして書き出す。
#[test]
fn crane_is_written_as_a_fixture() {
    let (doc, _) = crane();
    let faces = extract_faces(&doc.cp);
    write_fixture(&doc, &faces, "crane");
}
