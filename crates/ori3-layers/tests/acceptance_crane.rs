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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use glam::{DVec2, DVec3};
use ori3_cp::{Face, extract_faces, insert_segment};
use ori3_geometry::Isometry2;
use ori3_layers::flat_state::representative_point;
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, fold_through};
use ori3_layers::techniques::TechniqueInput;
use ori3_layers::{
    FlatState, FoldThroughResult, PrecreaseCollapseInput, collapse_precrease_network,
    flat_state_at, inside_reverse, petal, replay, squash,
};
use ori3_model::{
    CreasePattern, Document, Driver, DriverLine, Edge, EdgeKind, Face3D, FaceId,
    FinishSoftSettings, FoldStep, Frame3D, Paper, TechniqueKind, Vertex,
};
use ori3_rigid::{max_seam_gap, self_intersection_pairs};

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

/// 折り上がりが平ら(全ての面がz=0に乗る)ことを確かめる。
fn assert_flat(doc: &Document, label: &str) {
    let result = replay(doc, doc.sequence.len(), 1.0);
    let (_, state) = state_of(doc);
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
    assert_eq!(
        result.frame.faces.len(),
        state.placements.len(),
        "{label}: 3D表示と平坦状態の面数"
    );
    for face in &result.frame.faces {
        assert_eq!(
            face.mirrored, state.placements[&face.face].mirrored,
            "{label}: 面{}の3D表示と平坦状態の鏡映偶奇",
            face.face
        );
    }
    assert!(
        state
            .placements
            .values()
            .any(|placement| placement.mirrored)
            && state
                .placements
                .values()
                .any(|placement| !placement.mirrored),
        "{label}: 表向き面と裏返った面の双方を検査する"
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

/// 鶴以外の既存作品にも同じ一般検証器を適用する回帰検査。
/// 作品の最終手に永続化された順序を通常replayした結果だけを候補にし、
/// Face ID順などで検査側から補完しない。
#[test]
#[ignore = "既知の欠陥: 花弁折り・つぶし折りが返す層順が一般制約（taco-tortilla/taco-taco）を満たさない。別単位で修正するまで。実測: 鳥 4/20・カエル 116/6664, 12/1826"]
fn bird_base_saved_layer_order_satisfies_general_constraints() {
    let document = bird_base();
    assert!(
        document
            .sequence
            .last()
            .and_then(|step| step.layer_order.as_ref())
            .is_some(),
        "鳥の基本形の最終手には保存layer_orderがある"
    );
    let (faces, state) = state_of(&document);
    let validation = ori3_layers::precrease_collapse::validate_precrease_layer_order(
        &document.cp,
        &faces,
        &state.placements,
        &state.order,
    )
    .expect("鳥の基本形の保存順を一般制約で検証できる");
    assert!(
        validation.is_valid(),
        "鳥の基本形の保存順は一般制約違反0: {:?}",
        validation.violations
    );
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
            let gap = max_seam_gap(&doc.cp, &faces, &frame);
            assert!(
                gap < 1e-6,
                "折り鶴(手順{up_to}, t={t}): 面が {gap:.9} 離れている"
            );
        }
    }
}

/// 裂け検査が壊れた入力を確実に検出することも、完成した鶴そのもので確かめる。
#[test]
fn seam_gap_detects_a_deliberately_broken_crane_frame() {
    let (doc, _) = crane();
    let faces = extract_faces(&doc.cp);
    let mut frame = replay(&doc, doc.sequence.len(), 1.0).frame;
    assert!(max_seam_gap(&doc.cp, &faces, &frame) < 1e-6);

    let mut edge_face_count: HashMap<u32, usize> = HashMap::new();
    for face in &faces {
        let mut edges = face.edges.clone();
        edges.sort_unstable();
        edges.dedup();
        for edge in edges {
            *edge_face_count.entry(edge).or_default() += 1;
        }
    }
    let moved_face = faces
        .iter()
        .find(|face| {
            face.edges
                .iter()
                .any(|edge| edge_face_count.get(edge) == Some(&2))
        })
        .expect("折り目を共有する鶴の面")
        .id;
    for point in &mut frame
        .faces
        .iter_mut()
        .find(|face| face.face == moved_face)
        .expect("3Dフレーム内の鶴の面")
        .polygon
    {
        point[2] += 0.01;
    }

    let gap = max_seam_gap(&doc.cp, &faces, &frame);
    assert!(gap > 1e-3, "壊した鶴の裂けを検出できない: gap={gap}");
}

/// 内部頂点(まわりの辺がすべて折り線=紙の縁に触れない点)のうち、折り線が
/// いちばん多く集まる点の折り線を辺ID順に返す。この点の折り角どうしには
/// 拘束があるので、複数を勝手な値に固定するとループ閉包が破れる。
fn hinges_at_inner_vertex(cp: &CreasePattern, faces: &[Face]) -> Vec<u32> {
    let mut share: HashMap<u32, usize> = HashMap::new();
    for f in faces {
        let mut ids = f.edges.clone();
        ids.sort_unstable();
        ids.dedup();
        for e in ids {
            *share.entry(e).or_default() += 1;
        }
    }
    let mut inc: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut on_border: HashMap<u32, bool> = HashMap::new();
    for e in &cp.edges {
        let hinge = share.get(&e.id).copied() == Some(2);
        for v in [e.v0, e.v1] {
            if hinge {
                inc.entry(v).or_default().push(e.id);
            } else {
                on_border.insert(v, true);
            }
        }
    }
    let mut cand: Vec<(usize, u32, Vec<u32>)> = inc
        .into_iter()
        .filter(|(v, _)| !on_border.contains_key(v))
        .map(|(v, mut es)| {
            es.sort_unstable();
            (es.len(), v, es)
        })
        .collect();
    // 本数の多い順、同数なら頂点ID順(HashMapの走査順に依存しない決定的な選択)
    cand.sort_by_key(|(n, v, _)| (std::cmp::Reverse(*n), *v));
    cand.into_iter()
        .next()
        .map(|(_, _, es)| es)
        .unwrap_or_default()
}

/// 角度スライダーで折り角を次々に指定していく操作の再現(実機で報告された不具合)。
///
/// 指定済みのヒンジを全部「固定」として渡すと、内部頂点まわりの拘束と両立せず
/// ループ閉包が破れて面が離れる(=紙が切れて見える)。いま操作しているヒンジ
/// だけを固定し、以前の指定は「なるべく保ちたい目標」として追従させると、
/// 閉包を満たす形が返る。
#[test]
fn crane_paper_stays_connected_while_angles_are_set_one_by_one() {
    let (doc, _) = crane();
    let faces = extract_faces(&doc.cp);
    let hinges = hinges_at_inner_vertex(&doc.cp, &faces);
    assert!(
        hinges.len() >= 5,
        "内部頂点に折り線が5本以上ある: {hinges:?}"
    );
    let kinds: HashMap<u32, EdgeKind> = doc.cp.edges.iter().map(|e| (e.id, e.kind)).collect();
    let want = |e: u32| {
        if kinds[&e] == EdgeKind::Valley {
            -70.0
        } else {
            70.0
        }
    };
    let picked: Vec<u32> = hinges.iter().copied().take(5).collect();

    // (1) 指定済みを全部固定する古いやり方は面が離れる(不具合の再現)
    let mut torn = 0.0f64;
    for i in 1..=picked.len() {
        let drivers: Vec<Driver> = picked[..i]
            .iter()
            .map(|&h| Driver {
                hinge: h,
                target_angle_deg: want(h),
            })
            .collect();
        let res = ori3_rigid::solve(&doc.cp, &faces, &drivers, None);
        torn = torn.max(max_seam_gap(&doc.cp, &faces, &res.frame));
    }
    assert!(torn > 1e-3, "全部固定だと面が離れるはず(実際 {torn:.9})");

    // (2) いま操作している1本だけを固定し、以前の指定は目標として追従させる
    let mut warm: Option<HashMap<u32, f64>> = None;
    for i in 1..=picked.len() {
        let h = picked[i - 1];
        let hard = vec![Driver {
            hinge: h,
            target_angle_deg: want(h),
        }];
        let targets: HashMap<u32, f64> = picked[..i].iter().map(|&e| (e, want(e))).collect();
        let res = ori3_rigid::solve_near(&doc.cp, &faces, &hard, &targets, warm.as_ref());
        let gap = max_seam_gap(&doc.cp, &faces, &res.frame);
        assert!(gap < 1e-6, "{i}本目まで指定: 面が {gap:.9} 離れている");
        assert!(
            (res.angles[&h] - want(h)).abs() < 1e-9,
            "操作中の折り線は指定どおり({}度)",
            res.angles[&h]
        );
        warm = Some(res.angles);
    }
}

/// 以前に指定した角度が「なるべく保たれる」ことの確認。
///
/// 目標どうしが両立するとき(同時に満たせる形が存在するとき)は、次のヒンジを
/// 指定しても前の指定がほとんど動かないこと。両立しない目標(上のテストの
/// ±70°など)は幾何的に保てないので、ここでは実際に折れる形を目標に使う。
#[test]
fn crane_keeps_previously_set_angles_while_setting_more() {
    let (doc, _) = crane();
    let faces = extract_faces(&doc.cp);
    let picked: Vec<u32> = hinges_at_inner_vertex(&doc.cp, &faces)
        .into_iter()
        .take(5)
        .collect();
    // 1本だけ固定して解いた形=5本すべてを同時に満たせる目標
    let base = ori3_rigid::solve(
        &doc.cp,
        &faces,
        &[Driver {
            hinge: picked[0],
            target_angle_deg: 70.0,
        }],
        None,
    );
    let goal = |e: u32| base.angles[&e];

    let mut warm: Option<HashMap<u32, f64>> = None;
    for i in 1..=picked.len() {
        let h = picked[i - 1];
        let hard = vec![Driver {
            hinge: h,
            target_angle_deg: goal(h),
        }];
        let targets: HashMap<u32, f64> = picked[..i].iter().map(|&e| (e, goal(e))).collect();
        let res = ori3_rigid::solve_near(&doc.cp, &faces, &hard, &targets, warm.as_ref());
        let gap = max_seam_gap(&doc.cp, &faces, &res.frame);
        assert!(gap < 1e-6, "{i}本目まで指定: 面が {gap:.9} 離れている");
        for &e in &picked[..i] {
            let d = (res.angles[&e] - goal(e)).abs();
            assert!(d < 0.01, "以前の指定(辺{e})が {d:.6}度ずれた");
        }
        warm = Some(res.angles);
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

fn explicit_flat_frame(document: &Document, faces: &[Face], state: &FlatState) -> Frame3D {
    let positions = vertex_pos(&document.cp);
    Frame3D {
        faces: faces
            .iter()
            .map(|face| {
                let rank = state
                    .order
                    .iter()
                    .position(|id| *id == face.id)
                    .expect("全ての面が層順序にある");
                Face3D {
                    face: face.id,
                    polygon: face
                        .vertices
                        .iter()
                        .map(|vertex| {
                            let point = state.placements[&face.id].apply(positions[vertex]);
                            [point.x, point.y, 0.0]
                        })
                        .collect(),
                    layer: u32::try_from(rank).expect("層順序はu32に収まる"),
                    surface_rank: u32::try_from(rank).expect("層順序はu32に収まる"),
                    mirrored: state.placements[&face.id].mirrored,
                }
            })
            .collect(),
        warnings: Vec::new(),
    }
}

fn finished_replay_coordinates(
    mut document: Document,
    enabled: bool,
    label: &str,
) -> Vec<(FaceId, [f64; 3])> {
    // 3回測定の最大gapは0。明示した平坦層の組立てで出る丸めだけを許すため、
    // モデル共通EPS(1e-9)を境界にする。可視の裂け(1e-6)より十分小さい。
    const FLAT_GAP_TOLERANCE: f64 = 1e-9;
    let settings = FinishSoftSettings {
        enabled,
        stiffness: 0.52,
        pressure: 0.41,
    };
    let step_id = u32::try_from(document.sequence.len()).expect("手順数はu32に収まる");
    document.sequence.push(FoldStep {
        id: step_id,
        kind: TechniqueKind::Pose,
        drivers: Vec::new(),
        layer_order: None,
        alignment: None,
        finish_soft: Some(settings),
        note: "SIM-015仕上げ確定".to_string(),
    });
    let up_to = document.sequence.len();
    assert_eq!(
        document.finish_soft_at(up_to, 1.0),
        Some(settings),
        "{label}: 完了位置で記録した仕上げ値を選ぶ"
    );

    let faces = extract_faces(&document.cp);
    let (flat, flat_warnings) =
        flat_state_at(&document, &faces, up_to).expect("仕上げPoseまで平坦に再生できる");
    assert!(
        flat_warnings.is_empty(),
        "{label}: 明示平坦層の再生警告なし: {flat_warnings:?}"
    );
    let flat_frame = explicit_flat_frame(&document, &faces, &flat);
    assert!(
        self_intersection_pairs(&flat_frame).is_empty(),
        "{label}: 明示した平坦層にすり抜けはない"
    );
    let flat_gap = max_seam_gap(&document.cp, &faces, &flat_frame);
    assert!(
        flat_gap < FLAT_GAP_TOLERANCE,
        "{label}: 明示した平坦層はつながる (gap={flat_gap:.3e})"
    );

    let replayed = replay(&document, up_to, 1.0);
    assert!(
        replayed.warnings.is_empty() && replayed.skipped.is_empty(),
        "{label}: 仕上げ{}で完全に再生する: warnings={:?}, skipped={:?}",
        if enabled { "on" } else { "off" },
        replayed.warnings,
        replayed.skipped
    );

    replayed
        .frame
        .faces
        .iter()
        .flat_map(|face| {
            face.polygon.iter().map(move |point| {
                assert!(
                    point.iter().all(|coordinate| coordinate.is_finite()),
                    "{label}: 再生座標は有限"
                );
                (face.face, *point)
            })
        })
        .collect()
}

/// SIM-015: 折り鶴は仕上げのon/offを最後の手順へ保存しても、展開図と手順だけで
/// 完全に再生できる。3回の位置差は実測0なので、丸めだけを許す1e-12を上限にした。
/// これは平坦層の判定EPS(1e-9)の1/1000であり、見える位置ずれを許容しない。
#[test]
fn crane_replays_with_finish_soft_on_and_off_three_times_without_penetration() {
    const POSITION_TOLERANCE: f64 = 1e-12;

    let mut configurations_replayed = 0usize;
    let mut observed_max_delta = 0.0_f64;
    for enabled in [false, true] {
        let mut baseline: Option<Vec<(FaceId, [f64; 3])>> = None;
        for run in 1..=3 {
            let (document, _) = crane();
            let label = format!(
                "折り鶴/仕上げ{}/{}回目",
                if enabled { "on" } else { "off" },
                run
            );
            let coordinates = finished_replay_coordinates(document, enabled, &label);
            if let Some(reference) = &baseline {
                assert_eq!(
                    coordinates.len(),
                    reference.len(),
                    "{label}: 再生した頂点数は基準と一致する"
                );
                for ((face, point), (reference_face, reference_point)) in
                    coordinates.iter().zip(reference)
                {
                    assert_eq!(face, reference_face, "{label}: 面IDは基準と一致する");
                    let delta = point
                        .iter()
                        .zip(reference_point)
                        .map(|(left, right)| (left - right).abs())
                        .fold(0.0_f64, f64::max);
                    observed_max_delta = observed_max_delta.max(delta);
                    assert!(
                        delta <= POSITION_TOLERANCE,
                        "{label}: 位置差 {delta:.3e} は許容値 {POSITION_TOLERANCE:.3e} 以下"
                    );
                }
            } else {
                baseline = Some(coordinates);
            }
        }
        configurations_replayed += 1;
    }
    assert_eq!(
        configurations_replayed, 2,
        "仕上げon/offの2/2で折り鶴を再生する"
    );
    assert!(
        observed_max_delta <= POSITION_TOLERANCE,
        "3回実測の最大位置差 {observed_max_delta:.3e} は許容値以下"
    );
}

// ---------------------------------------------------------------------------
// フロント側テスト用のフィクスチャ書き出し
// ---------------------------------------------------------------------------

/// 完成形の展開図と面を、フロント側(vitest)が読めるJSONの文字列にする。
/// 対称軸の判定(`apps/desktop/src/lib/grabDrive.ts`)を**実データ**で検証するため。
/// serde_jsonへ依存を増やさずに済むよう、必要な項目だけを手書きで出力する。
/// f64は `{:?}` で往復可能な最短表記になる。
fn front_fixture_json(doc: &Document, faces: &[Face]) -> String {
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
    s
}

fn crane_front_fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/desktop/src/lib/__fixtures__/crane.json")
}

/// 明示的な再生成専用: `cargo test -p ori3-layers --test acceptance_crane regenerate_crane_front_fixture -- --ignored --exact`
#[test]
#[ignore = "フロント用折り鶴fixtureを明示的に作り直すときだけ実行する"]
fn regenerate_crane_front_fixture() {
    let (doc, _) = crane();
    let faces = extract_faces(&doc.cp);
    let path = crane_front_fixture_path();
    std::fs::create_dir_all(path.parent().expect("置き場")).expect("フィクスチャ置き場を作る");
    std::fs::write(&path, front_fixture_json(&doc, &faces)).expect("フィクスチャを書き出す");
}

/// apps配下へ書き込まず、既存の折り鶴フィクスチャが現在の実データと一致するか調べる。
///
/// 以前は普通のテストが毎回このファイルを上書きしていた。テストを走らせるだけで
/// コミット済みのファイルが変わると、意図しない書き換えを一緒にコミットしてしまう。
/// カエル(`acceptance_frog.rs`)と同じ「読むだけで照合する」形にそろえる。
#[test]
fn crane_front_fixture_matches_read_only() {
    /// 座標の差の許容量。紙の一辺を1とした値なので、この差は表示にも計算にも影響しない。
    const COORD_TOLERANCE: f64 = 1e-9;

    let (doc, _) = crane();
    let faces = extract_faces(&doc.cp);
    let path = crane_front_fixture_path();
    let stored = std::fs::read_to_string(&path).expect("既存のフロント用折り鶴fixtureを読む");
    let generated = front_fixture_json(&doc, &faces);

    let (stored_shape, stored_numbers) = split_numbers(&stored.replace("\r\n", "\n"));
    let (generated_shape, generated_numbers) = split_numbers(&generated.replace("\r\n", "\n"));

    assert_eq!(
        stored_shape,
        generated_shape,
        "フロント用折り鶴fixtureの構造が現在の展開図と不一致: {}",
        path.display()
    );
    assert_eq!(
        stored_numbers.len(),
        generated_numbers.len(),
        "フロント用折り鶴fixtureの数値の個数が不一致: {}",
        path.display()
    );
    for (index, (stored_value, generated_value)) in
        stored_numbers.iter().zip(&generated_numbers).enumerate()
    {
        let scale = stored_value.abs().max(generated_value.abs()).max(1.0);
        assert!(
            (stored_value - generated_value).abs() <= COORD_TOLERANCE * scale,
            "フロント用折り鶴fixtureの{index}番目の数値が不一致: 保存 {stored_value} / 現在 {generated_value} ({})",
            path.display()
        );
    }
}

/// 数値を `#` へ置き換えた骨組みと、取り出した数値の並びに分ける。
///
/// 座標は計算機や数学ライブラリの違いで最下位の桁が変わることがあるため、
/// 文字列のまま厳密に比べると、どの計算機で作り直しても他方では一致しなくなる。
/// 骨組み(項目名・並び・整数のID)は厳密に、座標は許容差で比べるために分ける。
fn split_numbers(text: &str) -> (String, Vec<f64>) {
    let chars: Vec<char> = text.chars().collect();
    let mut skeleton = String::with_capacity(text.len());
    let mut numbers = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let head = chars[i];
        let starts_number = head.is_ascii_digit()
            || (head == '-' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit());
        if !starts_number {
            skeleton.push(head);
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < chars.len() {
            let c = chars[i];
            if c.is_ascii_digit() || c == '.' {
                i += 1;
            } else if c == 'e' || c == 'E' {
                i += 1;
                if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                    i += 1;
                }
            } else {
                break;
            }
        }
        let token: String = chars[start..i].iter().collect();
        match token.parse::<f64>() {
            Ok(value) => {
                numbers.push(value);
                skeleton.push('#');
            }
            Err(_) => skeleton.push_str(&token),
        }
    }
    (skeleton, numbers)
}

/// 角度だけで折った形からでも、紙の重なり順を求められることを検査する。
///
/// 手順を記録せず角度で折ると重なり順が決まらず、同じ平面の面が完全に同じ位置へ
/// 描かれて裏面が見えたり貫通して見える(2026-08-12に利用者の画面で確認。
/// 16面すべてが同じ段、厚み0)。折り上がった形から順序を求めれば解消できる。
#[test]
fn derived_layer_order_matches_the_recorded_fold() {
    let document = bird_base();
    let faces = extract_faces(&document.cp);
    let replayed = replay(&document, document.sequence.len(), 1.0);

    let derived = ori3_rigid::derive_layer_order(&document.cp, &faces, &replayed.frame)
        .expect("平らに折り切った鳥の基本形から重なり順を求められる");
    assert_eq!(
        derived.len(),
        replayed.frame.faces.len(),
        "全ての面が順序に含まれる"
    );

    // 求めた順序を段として入れ直すと、形との矛盾が無くなること。
    let mut frame = replayed.frame.clone();
    let rank: HashMap<FaceId, u32> = derived
        .iter()
        .enumerate()
        .map(|(index, &id)| (id, u32::try_from(index).expect("面の数は段に収まる")))
        .collect();
    for face in &mut frame.faces {
        face.layer = rank[&face.face];
    }
    assert!(
        !ori3_rigid::layer_order_conflicts(&document.cp, &faces, &frame),
        "求めた重なり順が紙の形と矛盾している"
    );

    // 同じ入力なら必ず同じ順序になること。
    let again = ori3_rigid::derive_layer_order(&document.cp, &faces, &replayed.frame)
        .expect("2回目も求められる");
    assert_eq!(derived, again, "同じ入力で順序が変わってはいけない");
}

/// 完成した鶴の形を検査する。
///
/// 裂けや交差が0でも形が違うことがある(2026-08-12に折り鶴で確認)。
/// 数値だけを合格条件にせず、形そのものを条件に入れる。
///
/// 実測(2026-08-12): 外形 幅0.432 × 奥行0.432 × 高さ0.000、面29枚、交差0組。
/// 完成形は平らに畳まれ、対角線について左右対称になる。
#[test]
fn completed_crane_is_flat_and_symmetric() {
    let (doc, _) = crane();
    let replayed = replay(&doc, doc.sequence.len(), 1.0);
    let points: Vec<[f64; 3]> = replayed
        .frame
        .faces
        .iter()
        .flat_map(|face| face.polygon.iter().copied())
        .collect();
    assert!(!points.is_empty(), "面がある");

    let span = |axis: usize| {
        let lo = points.iter().map(|p| p[axis]).fold(f64::MAX, f64::min);
        let hi = points.iter().map(|p| p[axis]).fold(f64::MIN, f64::max);
        (lo, hi, hi - lo)
    };
    let (_, _, width) = span(0);
    let (_, _, depth) = span(1);
    let (_, _, height) = span(2);

    assert!(
        height < 1e-9,
        "完成した鶴は平らに畳まれるはずだが高さ{height:e}がある"
    );
    assert!(
        (width - depth).abs() < 1e-6,
        "外形が正方形にならない(幅{width:.6} 奥行{depth:.6})"
    );
    assert!(
        (0.3..0.6).contains(&width),
        "外形の大きさが想定外(幅{width:.6}、紙の一辺は1.0)"
    );
    assert!(
        ori3_rigid::self_intersection_pairs(&replayed.frame).is_empty(),
        "完成した鶴で紙が交差している"
    );

    // 鶴は首と尾を振り分ける対角線について、概ね左右対称になる。
    // 頭の中割り折りは片側だけなので完全な対称にはならない。
    // 実測(2026-08-12): 全118点のうち鏡像が無いのは18点(15.3%)で、これが頭の分。
    const MIRROR_TOL: f64 = 1e-6;
    const MAX_ASYMMETRIC_RATIO: f64 = 0.25;
    let mirrored_missing = points
        .iter()
        .filter(|p| {
            !points.iter().any(|q| {
                (q[0] - p[1]).abs() < MIRROR_TOL
                    && (q[1] - p[0]).abs() < MIRROR_TOL
                    && (q[2] - p[2]).abs() < MIRROR_TOL
            })
        })
        .count();
    let ratio = mirrored_missing as f64 / points.len() as f64;
    assert!(
        ratio < MAX_ASYMMETRIC_RATIO,
        "対角線について左右対称になっていない(鏡像が無い点が{mirrored_missing}/{}={:.1}%)",
        points.len(),
        ratio * 100.0
    );
}

#[derive(Debug)]
struct TraditionalCraneEdge {
    id: String,
    assignment: char,
    p0: DVec2,
    p1: DVec2,
}

/// 正本CSVを読む。JSONの `implicit.C` は端点との食い違いがあるため参照しない。
fn traditional_crane_edges() -> Vec<TraditionalCraneEdge> {
    const CSV: &str =
        include_str!("fixtures/traditional-crane/traditional_crane_edges_equations.csv");

    let mut lines = CSV.lines().filter(|line| !line.trim().is_empty());
    let header = lines.next().expect("折り鶴正本CSVの見出し");
    let columns: Vec<&str> = header
        .trim_start_matches('\u{feff}')
        .split(',')
        .map(str::trim)
        .collect();
    let column = |name: &str| {
        columns
            .iter()
            .position(|candidate| *candidate == name)
            .unwrap_or_else(|| panic!("折り鶴正本CSVに列{name}が無い"))
    };
    let edge_id = column("edge_id");
    let assignment = column("assignment");
    let x1 = column("x1");
    let y1 = column("y1");
    let x2 = column("x2");
    let y2 = column("y2");

    lines
        .enumerate()
        .map(|(row, line)| {
            let fields: Vec<&str> = line.split(',').map(str::trim).collect();
            let field = |index: usize, name: &str| {
                fields
                    .get(index)
                    .copied()
                    .unwrap_or_else(|| panic!("折り鶴正本CSVの{}行目に列{name}が無い", row + 2))
            };
            let number = |index: usize, name: &str| {
                field(index, name).parse::<f64>().unwrap_or_else(|error| {
                    panic!(
                        "折り鶴正本CSVの{}行目{name}を数値として読めない: {error}",
                        row + 2
                    )
                })
            };
            let assignment_text = field(assignment, "assignment");
            let mut assignment_chars = assignment_text.chars();
            let assignment = assignment_chars
                .next()
                .unwrap_or_else(|| panic!("折り鶴正本CSVの{}行目assignmentが空", row + 2));
            assert!(
                matches!(assignment, 'M' | 'V' | 'B') && assignment_chars.next().is_none(),
                "折り鶴正本CSVの{}行目assignmentが不正: {assignment_text}",
                row + 2
            );
            TraditionalCraneEdge {
                id: field(edge_id, "edge_id").to_owned(),
                assignment,
                p0: DVec2::new(number(x1, "x1"), number(y1, "y1")),
                p1: DVec2::new(number(x2, "x2"), number(y2, "y2")),
            }
        })
        .collect()
}

/// `wanted` が現行展開図の辺区間の和集合に隙間なく含まれるかを調べる。
/// `allowed_kinds` に含まれる種類の現行辺だけを和集合へ入れる。
fn crane_cp_covers_segment(
    cp: &CreasePattern,
    wanted: &TraditionalCraneEdge,
    allowed_kinds: &[EdgeKind],
    tolerance: f64,
) -> bool {
    let positions = vertex_pos(cp);
    let delta = wanted.p1 - wanted.p0;
    let length = delta.length();
    assert!(length > tolerance, "正本の辺{}が退化している", wanted.id);
    let direction = delta / length;
    let distance_from_line = |point: DVec2| {
        let from_start = point - wanted.p0;
        (from_start.x * direction.y - from_start.y * direction.x).abs()
    };

    let mut intervals = Vec::<(f64, f64)>::new();
    for edge in &cp.edges {
        if !allowed_kinds.contains(&edge.kind) {
            continue;
        }
        let (Some(&p0), Some(&p1)) = (positions.get(&edge.v0), positions.get(&edge.v1)) else {
            continue;
        };
        if distance_from_line(p0) > tolerance || distance_from_line(p1) > tolerance {
            continue;
        }
        let t0 = (p0 - wanted.p0).dot(direction);
        let t1 = (p1 - wanted.p0).dot(direction);
        let lo = t0.min(t1).max(0.0);
        let hi = t0.max(t1).min(length);
        if hi + tolerance >= lo {
            intervals.push((lo, hi));
        }
    }
    intervals.sort_by(|left, right| left.0.total_cmp(&right.0));

    let mut covered_to = 0.0_f64;
    for (lo, hi) in intervals {
        if lo > covered_to + tolerance {
            break;
        }
        covered_to = covered_to.max(hi);
        if covered_to >= length - tolerance {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy)]
struct ActiveCraneSegment {
    p0: DVec2,
    p1: DVec2,
    kind: EdgeKind,
}

/// 同じ山谷・同一直線上で接している線分を1本へ戻す。
///
/// 0°の前折りを消したあと、その交点だけを理由に残る分割頂点まで除くために使う。
/// 別のactive線との交点は、下の`insert_segment`によるplanar graph再構成で復元される。
fn merge_active_crane_segment(
    left: ActiveCraneSegment,
    right: ActiveCraneSegment,
    tolerance: f64,
) -> Option<ActiveCraneSegment> {
    if left.kind != right.kind {
        return None;
    }
    let delta = left.p1 - left.p0;
    let length = delta.length();
    if length <= tolerance {
        return None;
    }
    let direction = delta / length;
    let line_distance = |point: DVec2| {
        let offset = point - left.p0;
        (offset.x * direction.y - offset.y * direction.x).abs()
    };
    if line_distance(right.p0) > tolerance || line_distance(right.p1) > tolerance {
        return None;
    }

    let project = |point: DVec2| (point - left.p0).dot(direction);
    let right_lo = project(right.p0).min(project(right.p1));
    let right_hi = project(right.p0).max(project(right.p1));
    if right_lo > length + tolerance || right_hi < -tolerance {
        return None;
    }

    let endpoints = [left.p0, left.p1, right.p0, right.p1];
    let p0 = *endpoints
        .iter()
        .min_by(|a, b| project(**a).total_cmp(&project(**b)))
        .expect("4端点がある");
    let p1 = *endpoints
        .iter()
        .max_by(|a, b| project(**a).total_cmp(&project(**b)))
        .expect("4端点がある");
    Some(ActiveCraneSegment {
        p0,
        p1,
        kind: left.kind,
    })
}

/// 完成状態で二面角が非0のM/Vだけを、用紙境界へ挿入し直したplanar graph。
fn active_final_crane_cp(
    doc: &Document,
    angle_tolerance_deg: f64,
    coordinate_tolerance: f64,
) -> (CreasePattern, Vec<ActiveCraneSegment>, usize) {
    let replayed = replay(doc, doc.sequence.len(), 1.0);
    assert!(
        replayed.warnings.is_empty() && replayed.skipped.is_empty(),
        "完成した折り鶴を最後まで再生できる: warnings={:?}, skipped={:?}",
        replayed.warnings,
        replayed.skipped
    );
    let positions = vertex_pos(&doc.cp);
    let mut active = Vec::<ActiveCraneSegment>::new();
    let mut zero_angle_edges = 0_usize;
    for edge in &doc.cp.edges {
        if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let angle = replayed
            .hinge_angles
            .get(&edge.id)
            .copied()
            .unwrap_or_else(|| panic!("完成状態に内部辺{}の二面角が無い", edge.id));
        if angle.abs() < angle_tolerance_deg {
            zero_angle_edges += 1;
            continue;
        }
        let p0 = positions
            .get(&edge.v0)
            .copied()
            .unwrap_or_else(|| panic!("辺{}の始点{}が無い", edge.id, edge.v0));
        let p1 = positions
            .get(&edge.v1)
            .copied()
            .unwrap_or_else(|| panic!("辺{}の終点{}が無い", edge.id, edge.v1));
        let mut pending = ActiveCraneSegment {
            p0,
            p1,
            kind: edge.kind,
        };
        loop {
            let Some(index) = active.iter().position(|existing| {
                merge_active_crane_segment(*existing, pending, coordinate_tolerance).is_some()
            }) else {
                active.push(pending);
                break;
            };
            let existing = active.swap_remove(index);
            pending = merge_active_crane_segment(existing, pending, coordinate_tolerance)
                .expect("直前に結合可能と判定した線分");
        }
    }

    let mut rebuilt = square_doc().cp;
    for segment in &active {
        insert_segment(
            &mut rebuilt,
            segment.p0.to_array(),
            segment.p1.to_array(),
            segment.kind,
        );
    }
    (rebuilt, active, zero_angle_edges)
}

/// 完成した折り鶴の展開図を、利用者提供の唯一の正本と照合する。
///
/// 正本の座標系もDocumentも `[0,1]²`・原点左下・y上なので、座標変換はしない。
/// 正本の異なる頂点間の最小距離は0.0411961001458665、その1/10は
/// 0.00411961001458665。3資料の共有点の最大差は4.5263792714e-13なので、
/// 別頂点を混同せず資料間の差を十分に吸収する1e-9を採用する。
#[test]
#[ignore = "決定B（伝統手順の再構成）未実装。CR2。手10の oracle 待ち"]
fn completed_crane_cp_matches_traditional_reference() {
    const TOLERANCE: f64 = 1e-9;
    const ACTIVE_ANGLE_TOLERANCE_DEG: f64 = 1e-9;
    // 2026-08-26の赤い基準はno-flip 11/102・全体反転18/102。どちらも低いため、
    // 表裏の約束が逆だとは判定せずfalseを維持する。線ごとの例外反転は認めない。
    const INVERT_ORACLE_MOUNTAIN_VALLEY: bool = false;

    let (doc, _) = crane();
    let oracle = traditional_crane_edges();
    let assigned_oracle: Vec<&TraditionalCraneEdge> = oracle
        .iter()
        .filter(|edge| matches!(edge.assignment, 'M' | 'V'))
        .collect();
    let border_oracle = oracle.iter().filter(|edge| edge.assignment == 'B').count();

    // 正本は「完成した鶴を開いた展開図」なので、最後に二面角0°へ戻った前折りは
    // 比較対象ではない。これは期待値の緩和ではなく、正本の定義に合わせた比較規則である。
    // 0°線が作った分割頂点も残さず、active M/Vと用紙境界からplanar graphを再構成する。
    let (active_cp, merged_active_segments, zero_angle_edges) =
        active_final_crane_cp(&doc, ACTIVE_ANGLE_TOLERANCE_DEG, TOLERANCE);
    let geometry_matches = assigned_oracle
        .iter()
        .filter(|edge| {
            crane_cp_covers_segment(
                &active_cp,
                edge,
                &[EdgeKind::Mountain, EdgeKind::Valley],
                TOLERANCE,
            )
        })
        .count();

    let assignment_matches_for = |invert: bool| {
        assigned_oracle
            .iter()
            .filter(|edge| {
                let mountain = edge.assignment == 'M';
                let kind = if mountain ^ invert {
                    EdgeKind::Mountain
                } else {
                    EdgeKind::Valley
                };
                crane_cp_covers_segment(&active_cp, edge, &[kind], TOLERANCE)
            })
            .count()
    };
    let assignment_matches_no_flip = assignment_matches_for(false);
    let assignment_matches_inverted = assignment_matches_for(true);
    let assignment_matches = assignment_matches_for(INVERT_ORACLE_MOUNTAIN_VALLEY);

    // 正本→activeだけでは、正本の端点を越えて延びたactive線分を見逃せる。
    // そこで正本M/Vからもplanar graphを作り、結合後のactive線分を逆向きに全区間照合する。
    let mut oracle_cp = square_doc().cp;
    for edge in &assigned_oracle {
        let kind = if edge.assignment == 'M' {
            EdgeKind::Mountain
        } else {
            EdgeKind::Valley
        };
        insert_segment(&mut oracle_cp, edge.p0.to_array(), edge.p1.to_array(), kind);
    }
    let active_as_reference: Vec<TraditionalCraneEdge> = merged_active_segments
        .iter()
        .enumerate()
        .map(|(index, segment)| TraditionalCraneEdge {
            id: format!("active-{index}"),
            assignment: if segment.kind == EdgeKind::Mountain {
                'M'
            } else {
                'V'
            },
            p0: segment.p0,
            p1: segment.p1,
        })
        .collect();
    let active_to_oracle_geometry = active_as_reference
        .iter()
        .filter(|edge| {
            crane_cp_covers_segment(
                &oracle_cp,
                edge,
                &[EdgeKind::Mountain, EdgeKind::Valley],
                TOLERANCE,
            )
        })
        .count();
    let active_assignment_matches_for = |invert: bool| {
        active_as_reference
            .iter()
            .filter(|edge| {
                let active_kind = if edge.assignment == 'M' {
                    EdgeKind::Mountain
                } else {
                    EdgeKind::Valley
                };
                let oracle_kind = if invert {
                    match active_kind {
                        EdgeKind::Mountain => EdgeKind::Valley,
                        EdgeKind::Valley => EdgeKind::Mountain,
                        _ => unreachable!("active線分はM/Vだけ"),
                    }
                } else {
                    active_kind
                };
                crane_cp_covers_segment(&oracle_cp, edge, &[oracle_kind], TOLERANCE)
            })
            .count()
    };
    let active_assignment_no_flip = active_assignment_matches_for(false);
    let active_assignment_inverted = active_assignment_matches_for(true);
    let active_assignment_matches = active_assignment_matches_for(INVERT_ORACLE_MOUNTAIN_VALLEY);
    let extra_active_geometry = active_as_reference.len() - active_to_oracle_geometry;

    let vertex_count = active_cp.vertices.len();
    let face_count = extract_faces(&active_cp).len();
    let active_internal_edges = active_cp
        .edges
        .iter()
        .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .count();
    if geometry_matches != 102
        || oracle.len() != 114
        || assignment_matches != 102
        || active_to_oracle_geometry != active_as_reference.len()
        || active_assignment_matches != active_as_reference.len()
        || assigned_oracle.len() != 102
        || border_oracle != 12
        || active_internal_edges != 102
        || vertex_count != 56
        || face_count != 59
    {
        panic!(
            "完成した折り鶴のactive展開図が正本と一致しない: coverage={geometry_matches}/{} (期待102/102; B除外), vertices={vertex_count}/56, faces={face_count}/59, mountain_valley={assignment_matches}/{} (期待102/102), no_flip={assignment_matches_no_flip}/{}, inverted={assignment_matches_inverted}/{}, active_to_oracle={active_to_oracle_geometry}/{}, extra_active_geometry={extra_active_geometry}, active_assignment={active_assignment_matches}/{}, active_assignment_no_flip={active_assignment_no_flip}/{}, active_assignment_inverted={active_assignment_inverted}/{}, active_internal_edges={active_internal_edges}/102, merged_active_segments={}, excluded_zero_angle_edges={zero_angle_edges}, angle_tolerance_deg={ACTIVE_ANGLE_TOLERANCE_DEG:e}, coordinate_tolerance={TOLERANCE:e}, invert_mountain_valley={INVERT_ORACLE_MOUNTAIN_VALLEY}",
            assigned_oracle.len(),
            assigned_oracle.len(),
            assigned_oracle.len(),
            assigned_oracle.len(),
            active_as_reference.len(),
            active_as_reference.len(),
            active_as_reference.len(),
            active_as_reference.len(),
            active_as_reference.len()
        );
    }
}

struct TraditionalCraneCollapseWork {
    document: Document,
    raw_vertex_coordinates: Vec<[String; 2]>,
    collapsed_cp: CreasePattern,
    result: FoldThroughResult,
    generated_order_before_oracle: Vec<FaceId>,
}

/// 正本CPのmaterial領域から導いた部位。Face IDは再抽出で変わり得るため、
/// 部位oracleには使わない。
struct TraditionalCraneMaterialParts {
    back_wing: BTreeSet<FaceId>,
    front_wing: BTreeSet<FaceId>,
    tail: BTreeSet<FaceId>,
    neck: BTreeSet<FaceId>,
}

/// 正本CPのmaterial境界ループ。座標そのものを複製せず、正本vertex IDから多角形を作る。
/// 代表点が境界上にない面だけを分類するため、共有境界の所属は曖昧にならない。
const TRADITIONAL_CRANE_BACK_WING_REGION: &[u32] = &[0, 46, 45, 44, 39, 40, 41, 42, 43];
const TRADITIONAL_CRANE_FRONT_WING_REGION: &[u32] = &[2, 21, 23, 25, 5, 16, 10, 8, 36, 37, 38];
const TRADITIONAL_CRANE_TAIL_REGION: &[u32] = &[1, 38, 37, 36, 35, 34, 33, 32, 24, 22];
const TRADITIONAL_CRANE_NECK_AND_HEAD_REGION: &[u32] =
    &[3, 55, 31, 30, 29, 28, 27, 26, 25, 23, 21, 47];
const TRADITIONAL_CRANE_HEAD_REGION: &[u32] = &[3, 55, 54, 53, 52, 51, 50, 49, 48, 47];

fn traditional_crane_material_region(cp: &CreasePattern, boundary: &[u32]) -> Vec<DVec2> {
    let positions = vertex_pos(cp);
    boundary
        .iter()
        .map(|vertex| {
            *positions
                .get(vertex)
                .unwrap_or_else(|| panic!("正本material領域の頂点{vertex}が無い"))
        })
        .collect()
}

fn traditional_crane_material_parts(
    cp: &CreasePattern,
    faces: &[Face],
) -> TraditionalCraneMaterialParts {
    let back_wing_region =
        traditional_crane_material_region(cp, TRADITIONAL_CRANE_BACK_WING_REGION);
    let front_wing_region =
        traditional_crane_material_region(cp, TRADITIONAL_CRANE_FRONT_WING_REGION);
    let tail_region = traditional_crane_material_region(cp, TRADITIONAL_CRANE_TAIL_REGION);
    let neck_and_head_region =
        traditional_crane_material_region(cp, TRADITIONAL_CRANE_NECK_AND_HEAD_REGION);
    let head_region = traditional_crane_material_region(cp, TRADITIONAL_CRANE_HEAD_REGION);

    let in_region = |face: &Face, region: &[DVec2]| {
        inside_polygon(region, DVec2::from(representative_point(cp, face)))
    };
    let back_wing = faces
        .iter()
        .filter(|face| in_region(face, &back_wing_region))
        .map(|face| face.id)
        .collect::<BTreeSet<_>>();
    let front_wing = faces
        .iter()
        .filter(|face| in_region(face, &front_wing_region))
        .map(|face| face.id)
        .collect::<BTreeSet<_>>();
    let tail = faces
        .iter()
        .filter(|face| in_region(face, &tail_region))
        .map(|face| face.id)
        .collect::<BTreeSet<_>>();
    let neck_and_head = faces
        .iter()
        .filter(|face| in_region(face, &neck_and_head_region))
        .map(|face| face.id)
        .collect::<BTreeSet<_>>();
    let head = faces
        .iter()
        .filter(|face| in_region(face, &head_region))
        .map(|face| face.id)
        .collect::<BTreeSet<_>>();
    let neck = neck_and_head
        .difference(&head)
        .copied()
        .collect::<BTreeSet<_>>();

    assert_eq!(back_wing.len(), 7, "material領域から後翼7面を導く");
    assert_eq!(front_wing.len(), 7, "material領域から前翼7面を導く");
    assert_eq!(tail.len(), 8, "material領域から尾8面を導く");
    assert_eq!(neck.len(), 8, "material領域から首8面を導く");
    assert_eq!(head.len(), 8, "material領域から頭8面を導く");
    TraditionalCraneMaterialParts {
        back_wing,
        front_wing,
        tail,
        neck,
    }
}

/// 利用者が与えた「首・尾は後翼と前翼の間」という部分順を、正本material領域と
/// 正の面積重なりから全数導出し、既存の`FoldStep.layer_order`へ保存する完全順を作る。
/// Face IDや旧rank列はoracleにせず、未指定の比較では自動collapse順を優先するだけとする。
fn traditional_crane_declared_layer_oracle(
    cp: &CreasePattern,
    faces: &[Face],
    placements: &HashMap<FaceId, Isometry2>,
    generated_order: &[FaceId],
) -> Vec<FaceId> {
    assert_eq!(generated_order.len(), faces.len());
    assert_eq!(generated_order.len(), 59);
    let general = ori3_layers::precrease_collapse::validate_precrease_layer_order(
        cp,
        faces,
        placements,
        generated_order,
    )
    .expect("自動collapse表示順とは独立に一般制約DAGを導ける");
    assert!(
        general.is_valid(),
        "自動collapseの表示継続順も一般制約上は有効: {:?}",
        general.violations
    );
    assert!(
        !general.unresolved_overlap_pairs.is_empty(),
        "展開図だけでは未決定の正面積重なりがある"
    );

    let parts = traditional_crane_material_parts(cp, faces);
    let generated_state = FlatState {
        placements: placements.clone(),
        order: generated_order.to_vec(),
    };
    let plane = faces
        .iter()
        .map(|face| (face.id, plane_poly(cp, face, &generated_state)))
        .collect::<HashMap<_, _>>();
    let mut constraints = BTreeSet::new();
    for middle_faces in [&parts.tail, &parts.neck] {
        for &middle in middle_faces {
            for &back in &parts.back_wing {
                if traditional_crane_positive_overlap_area(&plane[&middle], &plane[&back])
                    > TRADITIONAL_CRANE_OVERLAP_AREA_EPS
                {
                    constraints.insert((back, middle));
                }
            }
            for &front in &parts.front_wing {
                if traditional_crane_positive_overlap_area(&plane[&middle], &plane[&front])
                    > TRADITIONAL_CRANE_OVERLAP_AREA_EPS
                {
                    constraints.insert((middle, front));
                }
            }
        }
    }
    assert_eq!(
        constraints.len(),
        128,
        "正の面積で重なる後翼<首尾<前翼の全128比較を層oracleにする"
    );
    let constraints = constraints.into_iter().collect::<Vec<_>>();
    let declared = ori3_layers::precrease_collapse::resolve_precrease_layer_order_with_constraints(
        cp,
        faces,
        placements,
        generated_order,
        &constraints,
    )
    .expect("翼間層oracleを一般制約に反しない完全順へ延長できる");
    assert_eq!(declared.len(), generated_order.len());
    assert_eq!(
        declared.iter().copied().collect::<HashSet<_>>().len(),
        faces.len(),
        "保存層oracleは59面の完全permutation"
    );
    let declared_validation = ori3_layers::precrease_collapse::validate_precrease_layer_order(
        cp, faces, placements, &declared,
    )
    .expect("利用者層oracleを加えたtotal extensionを検証できる");
    assert!(
        declared_validation.is_valid(),
        "利用者層oracleを加えても一般制約違反0: {:?}",
        declared_validation.violations
    );
    let rank = declared
        .iter()
        .enumerate()
        .map(|(rank, &face)| (face, rank))
        .collect::<HashMap<_, _>>();
    assert!(
        constraints
            .iter()
            .all(|&(lower, upper)| rank[&lower] < rank[&upper]),
        "保存層oracleは正の面積で重なる翼間128比較をすべて満たす"
    );
    declared
}

const TRADITIONAL_CRANE_OVERLAP_AREA_EPS: f64 = 1e-12;
const TRADITIONAL_CRANE_POLYGON_EPS: f64 = 1e-12;

fn traditional_crane_polygon_area(polygon: &[DVec2]) -> f64 {
    if polygon.len() < 3 {
        return 0.0;
    }
    0.5 * (0..polygon.len())
        .map(|index| polygon[index].perp_dot(polygon[(index + 1) % polygon.len()]))
        .sum::<f64>()
}

fn traditional_crane_simple_polygon(boundary: &[DVec2]) -> Vec<DVec2> {
    let mut polygon = Vec::with_capacity(boundary.len());
    for &point in boundary {
        if polygon
            .last()
            .is_none_or(|previous: &DVec2| previous.distance(point) > TRADITIONAL_CRANE_POLYGON_EPS)
        {
            polygon.push(point);
        }
    }
    while polygon.len() > 1
        && polygon[0].distance(polygon[polygon.len() - 1]) <= TRADITIONAL_CRANE_POLYGON_EPS
    {
        polygon.pop();
    }
    polygon
}

fn traditional_crane_point_in_triangle(point: DVec2, a: DVec2, b: DVec2, c: DVec2) -> bool {
    (b - a).perp_dot(point - a) >= -TRADITIONAL_CRANE_POLYGON_EPS
        && (c - b).perp_dot(point - b) >= -TRADITIONAL_CRANE_POLYGON_EPS
        && (a - c).perp_dot(point - c) >= -TRADITIONAL_CRANE_POLYGON_EPS
}

fn traditional_crane_triangulate(boundary: &[DVec2]) -> Vec<Vec<DVec2>> {
    let mut polygon = traditional_crane_simple_polygon(boundary);
    assert!(
        polygon.len() >= 3
            && traditional_crane_polygon_area(&polygon).abs() > TRADITIONAL_CRANE_OVERLAP_AREA_EPS,
        "正本面の投影多角形は正面積"
    );
    if traditional_crane_polygon_area(&polygon) < 0.0 {
        polygon.reverse();
    }
    let mut triangles = Vec::with_capacity(polygon.len().saturating_sub(2));
    while polygon.len() > 3 {
        let count = polygon.len();
        let ear = (0..count)
            .find(|&index| {
                let a = polygon[(index + count - 1) % count];
                let b = polygon[index];
                let c = polygon[(index + 1) % count];
                (b - a).perp_dot(c - b) > TRADITIONAL_CRANE_POLYGON_EPS.powi(2)
                    && !polygon.iter().enumerate().any(|(other, &point)| {
                        other != index
                            && other != (index + count - 1) % count
                            && other != (index + 1) % count
                            && traditional_crane_point_in_triangle(point, a, b, c)
                    })
            })
            .expect("正本面の投影多角形を三角形分割できる");
        triangles.push(vec![
            polygon[(ear + count - 1) % count],
            polygon[ear],
            polygon[(ear + 1) % count],
        ]);
        polygon.remove(ear);
    }
    triangles.push(polygon);
    triangles
}

fn traditional_crane_deduplicate_polygon(points: Vec<DVec2>) -> Vec<DVec2> {
    let mut output = Vec::with_capacity(points.len());
    for point in points {
        if output
            .last()
            .is_none_or(|previous: &DVec2| previous.distance(point) > TRADITIONAL_CRANE_POLYGON_EPS)
        {
            output.push(point);
        }
    }
    if output.len() > 1
        && output[0].distance(output[output.len() - 1]) <= TRADITIONAL_CRANE_POLYGON_EPS
    {
        output.pop();
    }
    output
}

fn traditional_crane_intersect_convex(subject: &[DVec2], clip: &[DVec2]) -> Vec<DVec2> {
    let mut output = subject.to_vec();
    for index in 0..clip.len() {
        let clip_start = clip[index];
        let clip_end = clip[(index + 1) % clip.len()];
        let input = std::mem::take(&mut output);
        let Some(mut previous) = input.last().copied() else {
            break;
        };
        let mut previous_side = (clip_end - clip_start).perp_dot(previous - clip_start);
        for current in input {
            let current_side = (clip_end - clip_start).perp_dot(current - clip_start);
            let previous_inside = previous_side >= -TRADITIONAL_CRANE_POLYGON_EPS;
            let current_inside = current_side >= -TRADITIONAL_CRANE_POLYGON_EPS;
            if previous_inside != current_inside {
                let denominator = previous_side - current_side;
                if denominator.abs() > TRADITIONAL_CRANE_POLYGON_EPS.powi(2) {
                    output.push(previous + (current - previous) * (previous_side / denominator));
                }
            }
            if current_inside {
                output.push(current);
            }
            previous = current;
            previous_side = current_side;
        }
    }
    traditional_crane_deduplicate_polygon(output)
}

fn traditional_crane_positive_overlap_area(left: &[DVec2], right: &[DVec2]) -> f64 {
    let mut area = 0.0;
    for left_triangle in traditional_crane_triangulate(left) {
        for right_triangle in traditional_crane_triangulate(right) {
            let intersection = traditional_crane_intersect_convex(&left_triangle, &right_triangle);
            area += traditional_crane_polygon_area(&intersection).abs();
        }
    }
    area
}

#[derive(Debug)]
struct TraditionalCraneSandwichViolation {
    middle_part: &'static str,
    wing_part: &'static str,
    middle_face: FaceId,
    wing_face: FaceId,
    middle_rank: usize,
    wing_rank: usize,
    overlap_area: f64,
}

#[derive(Debug)]
struct TraditionalCraneSandwichAudit {
    overlap_counts: BTreeMap<&'static str, usize>,
    violations: Vec<TraditionalCraneSandwichViolation>,
    minimum_positive_overlap_area: f64,
}

fn traditional_crane_sandwich_audit(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
) -> TraditionalCraneSandwichAudit {
    let parts = traditional_crane_material_parts(cp, faces);
    let plane = faces
        .iter()
        .map(|face| (face.id, plane_poly(cp, face, state)))
        .collect::<HashMap<_, _>>();
    let ranks = state
        .order
        .iter()
        .enumerate()
        .map(|(rank, face)| (*face, rank))
        .collect::<HashMap<_, _>>();
    let middle_parts = [("tail", &parts.tail), ("neck", &parts.neck)];
    let wing_parts = [
        ("back_wing", &parts.back_wing, true),
        ("front_wing", &parts.front_wing, false),
    ];
    let mut overlap_counts = BTreeMap::new();
    let mut violations = Vec::new();
    let mut minimum_positive_overlap_area = f64::INFINITY;
    for (middle_name, middle_faces) in middle_parts {
        for &(wing_name, wing_faces, wing_must_be_below) in &wing_parts {
            let label = match (middle_name, wing_name) {
                ("tail", "back_wing") => "tail/back_wing",
                ("tail", "front_wing") => "tail/front_wing",
                ("neck", "back_wing") => "neck/back_wing",
                ("neck", "front_wing") => "neck/front_wing",
                _ => unreachable!(),
            };
            let mut overlaps = 0;
            for &middle_face in middle_faces {
                for &wing_face in wing_faces {
                    let overlap_area = traditional_crane_positive_overlap_area(
                        &plane[&middle_face],
                        &plane[&wing_face],
                    );
                    if overlap_area <= TRADITIONAL_CRANE_OVERLAP_AREA_EPS {
                        continue;
                    }
                    overlaps += 1;
                    minimum_positive_overlap_area = minimum_positive_overlap_area.min(overlap_area);
                    let middle_rank = ranks[&middle_face];
                    let wing_rank = ranks[&wing_face];
                    let valid = if wing_must_be_below {
                        wing_rank < middle_rank
                    } else {
                        middle_rank < wing_rank
                    };
                    if !valid {
                        violations.push(TraditionalCraneSandwichViolation {
                            middle_part: middle_name,
                            wing_part: wing_name,
                            middle_face,
                            wing_face,
                            middle_rank,
                            wing_rank,
                            overlap_area,
                        });
                    }
                }
            }
            overlap_counts.insert(label, overlaps);
        }
    }
    TraditionalCraneSandwichAudit {
        overlap_counts,
        violations,
        minimum_positive_overlap_area,
    }
}

fn traditional_crane_order_change_counts(before: &[FaceId], after: &[FaceId]) -> (usize, usize) {
    assert_eq!(before.len(), after.len());
    let before_rank = before
        .iter()
        .enumerate()
        .map(|(rank, face)| (*face, rank))
        .collect::<HashMap<_, _>>();
    let after_rank = after
        .iter()
        .enumerate()
        .map(|(rank, face)| (*face, rank))
        .collect::<HashMap<_, _>>();
    let rank_changed = before
        .iter()
        .filter(|face| before_rank[face] != after_rank[face])
        .count();
    let mut pair_changed = 0;
    for left in 0..before.len() {
        for right in (left + 1)..before.len() {
            let first = before[left];
            let second = before[right];
            if (before_rank[&first] < before_rank[&second])
                != (after_rank[&first] < after_rank[&second])
            {
                pair_changed += 1;
            }
        }
    }
    (rank_changed, pair_changed)
}

/// 正本CSVから、頂点ID・辺ID・端点・M/V/Bを一切作り替えずにCPを作る。
/// JSONの `implicit.C` は派生値の矛盾があるため、ここでも参照しない。
fn traditional_crane_reference_cp() -> (CreasePattern, Vec<[String; 2]>) {
    const VERTICES_CSV: &str =
        include_str!("fixtures/traditional-crane/traditional_crane_vertices.csv");
    const EDGES_CSV: &str =
        include_str!("fixtures/traditional-crane/traditional_crane_edges_equations.csv");

    let mut vertex_lines = VERTICES_CSV.lines().filter(|line| !line.trim().is_empty());
    let vertex_header = vertex_lines.next().expect("正本頂点CSVの見出し");
    assert_eq!(
        vertex_header.trim_start_matches('\u{feff}'),
        "vertex_id,x,y,scaled_x_for_side_L,scaled_y_for_side_L"
    );
    let mut vertices = Vec::new();
    let mut raw_vertex_coordinates = Vec::new();
    for (row, line) in vertex_lines.enumerate() {
        let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
        assert!(fields.len() >= 3, "正本頂点CSVの{}行目", row + 2);
        let id = fields[0]
            .parse::<u32>()
            .unwrap_or_else(|error| panic!("正本頂点IDを読めない: {error}"));
        assert_eq!(id as usize, row, "正本頂点IDは0から連続する");
        let x = fields[1]
            .parse::<f64>()
            .unwrap_or_else(|error| panic!("正本頂点xを読めない: {error}"));
        let y = fields[2]
            .parse::<f64>()
            .unwrap_or_else(|error| panic!("正本頂点yを読めない: {error}"));
        vertices.push(Vertex { id, pos: [x, y] });
        // 0/1も含めて、利用者資料の字面を作品ファイルへそのまま保存する。
        raw_vertex_coordinates.push([fields[1].to_owned(), fields[2].to_owned()]);
    }

    let mut edge_lines = EDGES_CSV.lines().filter(|line| !line.trim().is_empty());
    let edge_header = edge_lines.next().expect("正本辺CSVの見出し");
    let columns = edge_header
        .trim_start_matches('\u{feff}')
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();
    let column = |name: &str| {
        columns
            .iter()
            .position(|candidate| *candidate == name)
            .unwrap_or_else(|| panic!("正本辺CSVに列{name}が無い"))
    };
    let edge_id_column = column("edge_id");
    let assignment_column = column("assignment");
    let v_start_column = column("v_start");
    let v_end_column = column("v_end");
    let x1_column = column("x1");
    let y1_column = column("y1");
    let x2_column = column("x2");
    let y2_column = column("y2");

    let mut edges = Vec::new();
    for (row, line) in edge_lines.enumerate() {
        let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
        let field = |index: usize, name: &str| {
            fields
                .get(index)
                .copied()
                .unwrap_or_else(|| panic!("正本辺CSVの{}行目に列{name}が無い", row + 2))
        };
        let edge_id_text = field(edge_id_column, "edge_id").trim_start_matches('e');
        let id = edge_id_text
            .parse::<u32>()
            .unwrap_or_else(|error| panic!("正本辺IDを読めない: {error}"));
        assert_eq!(id as usize, row, "正本辺IDは0から連続する");
        let v0 = field(v_start_column, "v_start")
            .parse::<u32>()
            .unwrap_or_else(|error| panic!("正本辺の始点IDを読めない: {error}"));
        let v1 = field(v_end_column, "v_end")
            .parse::<u32>()
            .unwrap_or_else(|error| panic!("正本辺の終点IDを読めない: {error}"));
        let assignment = field(assignment_column, "assignment");
        let kind = match assignment {
            "M" => EdgeKind::Mountain,
            "V" => EdgeKind::Valley,
            "B" => EdgeKind::Border,
            other => panic!("正本辺{id}のassignmentが不正: {other}"),
        };
        let raw0 = raw_vertex_coordinates
            .get(v0 as usize)
            .unwrap_or_else(|| panic!("正本辺{id}の始点{v0}が無い"));
        let raw1 = raw_vertex_coordinates
            .get(v1 as usize)
            .unwrap_or_else(|| panic!("正本辺{id}の終点{v1}が無い"));
        assert_eq!(field(x1_column, "x1"), raw0[0], "正本辺{id}の始点x");
        assert_eq!(field(y1_column, "y1"), raw0[1], "正本辺{id}の始点y");
        assert_eq!(field(x2_column, "x2"), raw1[0], "正本辺{id}の終点x");
        assert_eq!(field(y2_column, "y2"), raw1[1], "正本辺{id}の終点y");
        edges.push(Edge { id, v0, v1, kind });
    }

    assert_eq!(vertices.len(), 56, "正本頂点56個");
    assert_eq!(edges.len(), 114, "正本辺114本");
    (
        CreasePattern {
            vertices,
            edges,
            next_vertex_id: 56,
            next_edge_id: 114,
        },
        raw_vertex_coordinates,
    )
}

/// `collapse_precrease_network` は入力を有限区間でなく支持直線として読む。
/// 同じ支持直線を複数回渡すと後続のlineがhitされないため、正本M/Vの支持直線を一意化する。
fn traditional_crane_unique_collapse_lines(cp: &CreasePattern) -> Vec<[[f64; 2]; 2]> {
    const LINE_TOLERANCE: f64 = 1e-9;
    let positions = vertex_pos(cp);
    let mut lines = Vec::<[[f64; 2]; 2]>::new();
    for edge in &cp.edges {
        if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let a = positions[&edge.v0];
        let b = positions[&edge.v1];
        let already_present = lines.iter().any(|line| {
            let line_a = DVec2::from(line[0]);
            let line_b = DVec2::from(line[1]);
            let direction = line_b - line_a;
            let length = direction.length();
            let distance = |point: DVec2| {
                let offset = point - line_a;
                (offset.x * direction.y - offset.y * direction.x).abs() / length
            };
            distance(a) <= LINE_TOLERANCE && distance(b) <= LINE_TOLERANCE
        });
        if !already_present {
            lines.push([a.to_array(), b.to_array()]);
        }
    }
    lines
}

fn traditional_crane_collapse_work() -> TraditionalCraneCollapseWork {
    let (oracle_cp, raw_vertex_coordinates) = traditional_crane_reference_cp();
    let faces = extract_faces(&oracle_cp);
    let initial = FlatState::initial(&oracle_cp, &faces);
    let collapse_lines = traditional_crane_unique_collapse_lines(&oracle_cp);
    let mut collapsed_cp = oracle_cp.clone();
    let mut result = collapse_precrease_network(
        &mut collapsed_cp,
        &faces,
        &initial,
        &PrecreaseCollapseInput {
            lines: collapse_lines,
            target_layers: None,
        },
    )
    .unwrap_or_else(|error| panic!("正本CPを一括collapseできない: {error}"));
    result.step.id = 0;

    // CP+M/Vだけでは決まらない枝は、利用者が与えた層oracleを既存の保存欄へ明示する。
    // collapseが返すFace ID tie-breakを作品oracleとして保存しない。
    let generated_order_before_oracle = result.state.order.clone();
    let declared_order = traditional_crane_declared_layer_oracle(
        &oracle_cp,
        &faces,
        &result.state.placements,
        &generated_order_before_oracle,
    );
    result.state.order = declared_order.clone();
    let material_faces = faces
        .iter()
        .map(|face| (face.id, face))
        .collect::<HashMap<_, _>>();
    result.step.layer_order = Some(
        declared_order
            .iter()
            .map(|face| representative_point(&oracle_cp, material_faces[face]))
            .collect(),
    );

    let mut document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    // 作品の展開図はcollapse内部の作業値でなく、入力された正本そのものを保持する。
    document.cp = oracle_cp;
    document.sequence = vec![result.step.clone()];
    TraditionalCraneCollapseWork {
        document,
        raw_vertex_coordinates,
        collapsed_cp,
        result,
        generated_order_before_oracle,
    }
}

/// serde_json依存を増やさず、正本の座標トークンをそのまま残すSCHEMA_VERSION=1 serializer。
fn traditional_crane_work_json(work: &TraditionalCraneCollapseWork) -> String {
    let document = &work.document;
    assert_eq!(
        document.cp.vertices.len(),
        work.raw_vertex_coordinates.len()
    );
    let mut output = String::from("{\n");
    output.push_str(&format!(
        "  \"schema_version\": {},\n  \"paper\": {{\"width_mm\": {:?}, \"height_mm\": {:?}}},\n",
        document.schema_version, document.paper.width_mm, document.paper.height_mm
    ));
    output.push_str("  \"cp\": {\n    \"vertices\": [\n");
    for (index, (vertex, raw)) in document
        .cp
        .vertices
        .iter()
        .zip(&work.raw_vertex_coordinates)
        .enumerate()
    {
        let comma = if index + 1 == document.cp.vertices.len() {
            ""
        } else {
            ","
        };
        output.push_str(&format!(
            "      {{\"id\": {}, \"pos\": [{}, {}]}}{comma}\n",
            vertex.id, raw[0], raw[1]
        ));
    }
    output.push_str("    ],\n    \"edges\": [\n");
    for (index, edge) in document.cp.edges.iter().enumerate() {
        let comma = if index + 1 == document.cp.edges.len() {
            ""
        } else {
            ","
        };
        output.push_str(&format!(
            "      {{\"id\": {}, \"v0\": {}, \"v1\": {}, \"kind\": \"{:?}\"}}{comma}\n",
            edge.id, edge.v0, edge.v1, edge.kind
        ));
    }
    output.push_str(&format!(
        "    ],\n    \"next_vertex_id\": {},\n    \"next_edge_id\": {}\n  }},\n",
        document.cp.next_vertex_id, document.cp.next_edge_id
    ));
    output.push_str("  \"sequence\": [\n");
    for (step_index, step) in document.sequence.iter().enumerate() {
        assert!(
            step.alignment.is_none(),
            "collapse stepにalignmentを合成しない"
        );
        assert!(
            step.finish_soft.is_none(),
            "collapse stepにfinish_softを合成しない"
        );
        assert!(step.note.is_empty(), "collapse stepのnoteは空");
        let step_comma = if step_index + 1 == document.sequence.len() {
            ""
        } else {
            ","
        };
        output.push_str(&format!(
            "    {{\"id\": {}, \"kind\": \"{:?}\", \"drivers\": [\n",
            step.id, step.kind
        ));
        for (driver_index, driver) in step.drivers.iter().enumerate() {
            let comma = if driver_index + 1 == step.drivers.len() {
                ""
            } else {
                ","
            };
            output.push_str(&format!(
                "      {{\"a\": [{:?}, {:?}], \"b\": [{:?}, {:?}], \"target_angle_deg\": {:?}}}{comma}\n",
                driver.a[0], driver.a[1], driver.b[0], driver.b[1], driver.target_angle_deg
            ));
        }
        output.push_str("    ], \"layer_order\": [\n");
        let layer_order = step
            .layer_order
            .as_ref()
            .expect("collapse stepには永続化したlayer_orderがある");
        for (point_index, point) in layer_order.iter().enumerate() {
            let comma = if point_index + 1 == layer_order.len() {
                ""
            } else {
                ","
            };
            output.push_str(&format!("      [{:?}, {:?}]{comma}\n", point[0], point[1]));
        }
        output.push_str(&format!("    ], \"note\": \"\"}}{step_comma}\n"));
    }
    let display = &document.display;
    output.push_str(&format!(
        "  ],\n  \"display\": {{\"front_color\": {:?}, \"back_color\": {:?}, \"grid_divisions\": {}, \"soft_enabled\": {}, \"soft_stiffness\": {:?}, \"soft_pressure\": {:?}, \"overlap_prevention_enabled\": {}, \"penetration_prevention_enabled\": {}}}\n}}\n",
        display.front_color,
        display.back_color,
        display.grid_divisions,
        display.soft_enabled,
        display.soft_stiffness,
        display.soft_pressure,
        display.overlap_prevention_enabled,
        display.penetration_prevention_enabled
    ));
    output
}

/// collapse結果を伝統手順の逆算に使えるよう、派生FaceIdでなく原紙の代表点と境界で保存する。
/// 1行が1面の1境界頂点で、rank順・面境界順に並ぶ。
fn traditional_crane_collapse_shape_csv(
    document: &Document,
    faces: &[Face],
    state: &FlatState,
) -> String {
    let frame = explicit_flat_frame(document, faces, state);
    let positions = vertex_pos(&document.cp);
    let material_faces = faces
        .iter()
        .map(|face| (face.id, face))
        .collect::<HashMap<_, _>>();
    let mut folded_faces = frame.faces.iter().collect::<Vec<_>>();
    folded_faces.sort_by_key(|face| face.surface_rank);

    let mut output = String::from(
        "# oracle_schema=traditional-crane-collapse-v1\n# coordinates=material:[0,1]^2-left-bottom-y-up;folded:[x,y,z]\nsurface_rank,material_rep_x,material_rep_y,mirrored,boundary_index,material_vertex_id,material_x,material_y,folded_x,folded_y,folded_z\n",
    );
    for folded in folded_faces {
        let material = material_faces[&folded.face];
        let representative = representative_point(&document.cp, material);
        assert_eq!(material.vertices.len(), folded.polygon.len());
        for (boundary_index, (&vertex_id, point)) in
            material.vertices.iter().zip(&folded.polygon).enumerate()
        {
            let material_point = positions[&vertex_id];
            output.push_str(&format!(
                "{},{:?},{:?},{},{},{},{:?},{:?},{:?},{:?},{:?}\n",
                folded.surface_rank,
                representative[0],
                representative[1],
                folded.mirrored,
                boundary_index,
                vertex_id,
                material_point.x,
                material_point.y,
                point[0],
                point[1],
                point[2]
            ));
        }
    }
    output
}

/// 1000x700の上面投影で、線を含まない紙面だけをface rankで所有者判定する。
/// 実機§10.7.10(1)と同じ視野を使い、M/V全反転の診断を画面へ触れずに行う。
fn traditional_crane_paper_pixels(
    document: &Document,
    faces: &[Face],
    state: &FlatState,
    view: &str,
) -> (usize, usize, BTreeMap<FaceId, usize>) {
    const WIDTH: usize = 1000;
    const HEIGHT: usize = 700;
    let frame = explicit_flat_frame(document, faces, state);
    let all_points = frame
        .faces
        .iter()
        .flat_map(|face| face.polygon.iter())
        .collect::<Vec<_>>();
    let minimum = [0, 1, 2].map(|axis| {
        all_points
            .iter()
            .map(|point| point[axis])
            .fold(f64::INFINITY, f64::min)
    });
    let maximum = [0, 1, 2].map(|axis| {
        all_points
            .iter()
            .map(|point| point[axis])
            .fold(f64::NEG_INFINITY, f64::max)
    });
    let center = [0, 1, 2].map(|axis| (minimum[axis] + maximum[axis]) * 0.5);
    let extent = [0, 1, 2].map(|axis| maximum[axis] - minimum[axis]);
    let aspect = WIDTH as f64 / HEIGHT as f64;
    let framing_extent = extent[1].max(extent[0] / aspect).max(0.1);
    let tangent = (45.0_f64.to_radians() * 0.5).tan();
    let distance = (framing_extent * 0.5 / tangent) * 1.3;
    let center3 = DVec3::from(center);
    let (camera, nominal_up) = match view {
        "top" => (center3 + DVec3::Z * distance, DVec3::Y),
        "isometric" => (center3 + DVec3::new(0.55, -0.55, 0.9) * distance, DVec3::Z),
        "side" => (center3 + DVec3::X * distance, DVec3::Z),
        other => panic!("未知の折り鶴診断視点: {other}"),
    };
    let direction = (center3 - camera).normalize();
    let right = direction.cross(nominal_up).normalize();
    let screen_up = right.cross(direction).normalize();

    let projected = frame
        .faces
        .iter()
        .map(|face| {
            let polygon = face
                .polygon
                .iter()
                .map(|point| {
                    let relative = DVec3::from(*point) - camera;
                    let forward = relative.dot(direction);
                    let ndc_x = relative.dot(right) / (forward * tangent * aspect);
                    let ndc_y = relative.dot(screen_up) / (forward * tangent);
                    [
                        (ndc_x * 0.5 + 0.5) * WIDTH as f64,
                        (ndc_y * 0.5 + 0.5) * HEIGHT as f64,
                    ]
                })
                .collect::<Vec<_>>();
            (face.face, face.surface_rank, face.mirrored, polygon)
        })
        .collect::<Vec<_>>();
    let contains = |polygon: &[[f64; 2]], x: f64, y: f64| {
        let mut inside = false;
        let mut previous = polygon.len() - 1;
        for current in 0..polygon.len() {
            let a = polygon[current];
            let b = polygon[previous];
            if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
                inside = !inside;
            }
            previous = current;
        }
        inside
    };

    let mut paper = 0usize;
    let mut back = 0usize;
    let mut back_by_face = BTreeMap::new();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let sample = [x as f64 + 0.5, y as f64 + 0.5];
            let top = projected
                .iter()
                .filter(|(_, _, _, polygon)| contains(polygon, sample[0], sample[1]))
                .max_by_key(|(face, rank, _, _)| (*rank, std::cmp::Reverse(*face)));
            let Some((face, _, mirrored, _)) = top else {
                continue;
            };
            paper += 1;
            if *mirrored {
                back += 1;
                *back_by_face.entry(*face).or_insert(0) += 1;
            }
        }
    }
    (paper, back, back_by_face)
}

/// 正本のM/V表裏契約だけを全体で反転した比較診断。通常実行では作品を変更しない。
#[test]
#[ignore]
fn diagnose_traditional_crane_global_mountain_valley_swap() {
    let original = traditional_crane_collapse_work();
    let original_faces = extract_faces(&original.document.cp);
    let original_pixels = traditional_crane_paper_pixels(
        &original.document,
        &original_faces,
        &original.result.state,
        "top",
    );

    let (mut swapped_cp, _) = traditional_crane_reference_cp();
    for edge in &mut swapped_cp.edges {
        edge.kind = match edge.kind {
            EdgeKind::Mountain => EdgeKind::Valley,
            EdgeKind::Valley => EdgeKind::Mountain,
            other => other,
        };
    }
    let swapped_faces = extract_faces(&swapped_cp);
    let initial = FlatState::initial(&swapped_cp, &swapped_faces);
    let collapse_lines = traditional_crane_unique_collapse_lines(&swapped_cp);
    let mut collapsed_cp = swapped_cp.clone();
    let swapped = collapse_precrease_network(
        &mut collapsed_cp,
        &swapped_faces,
        &initial,
        &PrecreaseCollapseInput {
            lines: collapse_lines,
            target_layers: None,
        },
    )
    .expect("M/V全反転CPを一括collapseできる");
    let mut swapped_document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    swapped_document.cp = swapped_cp;
    let swapped_pixels =
        traditional_crane_paper_pixels(&swapped_document, &swapped_faces, &swapped.state, "top");
    let changed_assignments = collapsed_cp
        .edges
        .iter()
        .zip(&swapped_document.cp.edges)
        .filter(|(actual, input)| actual.kind != input.kind)
        .count();
    let residual = traditional_crane_collapse_cycle_residual(
        &swapped_document.cp,
        &swapped_faces,
        &swapped.state,
    );
    let frame = explicit_flat_frame(&swapped_document, &swapped_faces, &swapped.state);
    println!(
        "original paper={} back={} ratio={:.12}% back_by_face={:?}",
        original_pixels.0,
        original_pixels.1,
        original_pixels.1 as f64 / original_pixels.0 as f64 * 100.0,
        original_pixels.2
    );
    let replayed_original = replay(&original.document, 1, 1.0);
    let replay_ranks = replayed_original
        .frame
        .faces
        .iter()
        .map(|face| (face.face, face.surface_rank))
        .collect::<BTreeMap<_, _>>();
    let mut replay_order = replayed_original
        .frame
        .faces
        .iter()
        .map(|face| (face.surface_rank, face.face))
        .collect::<Vec<_>>();
    replay_order.sort_unstable();
    let replay_state = FlatState {
        placements: original.result.state.placements.clone(),
        order: replay_order.into_iter().map(|(_, face)| face).collect(),
    };
    let replay_pixels =
        traditional_crane_paper_pixels(&original.document, &original_faces, &replay_state, "top");
    println!(
        "original order={:?} direct_ranks(face21,face57)=({:?},{:?}) replay_ranks(face21,face57)=({:?},{:?}) replay_visible paper={} back={} ratio={:.12}% back_by_face={:?}",
        original.result.state.order,
        original
            .result
            .state
            .order
            .iter()
            .position(|face| *face == 21),
        original
            .result
            .state
            .order
            .iter()
            .position(|face| *face == 57),
        replay_ranks.get(&21),
        replay_ranks.get(&57),
        replay_pixels.0,
        replay_pixels.1,
        replay_pixels.1 as f64 / replay_pixels.0 as f64 * 100.0,
        replay_pixels.2
    );
    println!(
        "swapped paper={} back={} ratio={:.12}% back_by_face={:?}",
        swapped_pixels.0,
        swapped_pixels.1,
        swapped_pixels.1 as f64 / swapped_pixels.0 as f64 * 100.0,
        swapped_pixels.2
    );
    println!(
        "swapped order={:?} residual={:.17e} warnings={} changed_assignments={} self_intersection_pairs={:?}",
        swapped.state.order,
        residual,
        swapped.warnings.len(),
        changed_assignments,
        self_intersection_pairs(&frame)
    );
}

/// 正本CPの外から見える紙面は、表だけまたは裏だけでなければならない。
///
/// 1000x700の線なし紙面で直接collapseは少数側0/121,547画素だった。実測0を境界にせず、
/// zero-eventの95%上限2.4646394863e-5を約4倍へ上方丸めした1e-4を採る(§10.7.9)。
/// exact sideは平坦面が線へ退化して紙面0画素になるため、0/0を0%とは数えない。
#[test]
fn traditional_crane_replay_visible_surface_is_uniform() {
    const VISIBLE_SURFACE_MINORITY_RATIO_LIMIT: f64 = 1e-4;
    let work = traditional_crane_collapse_work();
    let faces = extract_faces(&work.document.cp);
    let direct_top =
        traditional_crane_paper_pixels(&work.document, &faces, &work.result.state, "top");
    let direct_isometric =
        traditional_crane_paper_pixels(&work.document, &faces, &work.result.state, "isometric");
    let direct_side =
        traditional_crane_paper_pixels(&work.document, &faces, &work.result.state, "side");
    let uniform = |pixels: &(usize, usize, BTreeMap<FaceId, usize>)| {
        pixels.0 > 0
            && pixels.1.min(pixels.0 - pixels.1) as f64 / pixels.0 as f64
                <= VISIBLE_SURFACE_MINORITY_RATIO_LIMIT
    };
    assert!(
        uniform(&direct_top)
            && uniform(&direct_isometric)
            && (direct_top.1 * 2 <= direct_top.0) == (direct_isometric.1 * 2 <= direct_isometric.0)
            && direct_side.0 == 0,
        "正本collapse自身の3方向表裏契約: top={direct_top:?} isometric={direct_isometric:?} side={direct_side:?}"
    );

    let replayed = replay(&work.document, 1, 1.0);
    let mut replay_order = replayed
        .frame
        .faces
        .iter()
        .map(|face| (face.surface_rank, face.face))
        .collect::<Vec<_>>();
    replay_order.sort_unstable();
    let replay_state = FlatState {
        placements: work.result.state.placements.clone(),
        order: replay_order.into_iter().map(|(_, face)| face).collect(),
    };
    let visible_top = traditional_crane_paper_pixels(&work.document, &faces, &replay_state, "top");
    let visible_isometric =
        traditional_crane_paper_pixels(&work.document, &faces, &replay_state, "isometric");
    let visible_side =
        traditional_crane_paper_pixels(&work.document, &faces, &replay_state, "side");
    let top_minority = visible_top.1.min(visible_top.0 - visible_top.1);
    let isometric_minority = visible_isometric
        .1
        .min(visible_isometric.0 - visible_isometric.1);
    let rank_mismatches = replayed
        .frame
        .faces
        .iter()
        .filter(|actual| {
            work.result
                .state
                .order
                .iter()
                .position(|face| *face == actual.face)
                != Some(actual.surface_rank as usize)
        })
        .count();
    assert_eq!(
        rank_mismatches, 0,
        "保存したlayer oracleとreplay frameのsurface_rankは59面すべて一致する"
    );
    assert!(
        uniform(&visible_top)
            && uniform(&visible_isometric)
            && (visible_top.1 * 2 <= visible_top.0)
                == (visible_isometric.1 * 2 <= visible_isometric.0)
            && visible_side.0 == 0,
        "保存collapseのreplayで見える表裏が混在: top paper={} front={} back={} minority_ratio={:.12}% back_by_face={:?}; isometric paper={} front={} back={} minority_ratio={:.12}% back_by_face={:?}; side paper={} back={}; limit={:.12}% surface_rank_mismatches={}/59",
        visible_top.0,
        visible_top.0 - visible_top.1,
        visible_top.1,
        top_minority as f64 / visible_top.0 as f64 * 100.0,
        visible_top.2,
        visible_isometric.0,
        visible_isometric.0 - visible_isometric.1,
        visible_isometric.1,
        isometric_minority as f64 / visible_isometric.0 as f64 * 100.0,
        visible_isometric.2,
        visible_side.0,
        visible_side.1,
        VISIBLE_SURFACE_MINORITY_RATIO_LIMIT * 100.0,
        rank_mismatches
    );
}

/// 全内部辺を一周した反射配置と、collapseが保存した配置との差の最大値。
/// 角度差(rad)と平行移動距離の大きい方を採る。これはAPI内部の`approx_eq`と同じ二量である。
fn traditional_crane_collapse_cycle_residual(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
) -> f64 {
    let positions = vertex_pos(cp);
    let mut owners = BTreeMap::<u32, Vec<FaceId>>::new();
    for face in faces {
        for &edge_id in &face.edges {
            owners.entry(edge_id).or_default().push(face.id);
        }
    }
    let mut checked = 0_usize;
    let mut worst = 0.0_f64;
    for edge in &cp.edges {
        if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let incident = owners
            .get(&edge.id)
            .unwrap_or_else(|| panic!("正本辺{}に所属面が無い", edge.id));
        assert_eq!(incident.len(), 2, "正本M/V辺{}は内部辺", edge.id);
        let reflection = Isometry2::reflection(positions[&edge.v0], positions[&edge.v1]);
        let candidate = state.placements[&incident[0]].compose(&reflection);
        let existing = state.placements[&incident[1]];
        assert_eq!(
            candidate.mirrored, existing.mirrored,
            "正本辺{}を越えた鏡映の表裏",
            edge.id
        );
        let turn = std::f64::consts::TAU;
        let angle = (candidate.rotation - existing.rotation).rem_euclid(turn);
        let angle = angle.min(turn - angle);
        let translation = (candidate.translation - existing.translation).length();
        worst = worst.max(angle).max(translation);
        checked += 1;
    }
    assert_eq!(checked, 102, "B12を除く正本M/V102辺でcycleを検査する");
    worst
}

fn assert_traditional_crane_shape_snapshot(stored: &str, generated: &str) {
    const COORDINATE_TOLERANCE: f64 = 1e-9;
    const EXACT_COLUMNS: [usize; 4] = [0, 3, 4, 5];
    const FLOAT_COLUMNS: [usize; 7] = [1, 2, 6, 7, 8, 9, 10];
    let stored_lines = stored.lines().collect::<Vec<_>>();
    let generated_lines = generated.lines().collect::<Vec<_>>();
    assert_eq!(stored_lines.len(), 219, "shape oracleは見出し3行+境界216行");
    assert_eq!(stored_lines.len(), generated_lines.len());
    for (line_index, (stored_line, generated_line)) in
        stored_lines.iter().zip(&generated_lines).enumerate()
    {
        if line_index < 3 {
            assert_eq!(stored_line, generated_line, "shape oracle見出し");
            continue;
        }
        let stored_fields = stored_line.split(',').collect::<Vec<_>>();
        let generated_fields = generated_line.split(',').collect::<Vec<_>>();
        assert_eq!(
            stored_fields.len(),
            11,
            "shape oracle {}行目",
            line_index + 1
        );
        assert_eq!(stored_fields.len(), generated_fields.len());
        for column in EXACT_COLUMNS {
            assert_eq!(
                stored_fields[column],
                generated_fields[column],
                "shape oracle {}行目{}列目の識別値",
                line_index + 1,
                column + 1
            );
        }
        for column in FLOAT_COLUMNS {
            let stored_value = stored_fields[column]
                .parse::<f64>()
                .unwrap_or_else(|error| panic!("shape oracle数値を読めない: {error}"));
            let generated_value = generated_fields[column]
                .parse::<f64>()
                .unwrap_or_else(|error| panic!("生成shape数値を読めない: {error}"));
            assert!(
                (stored_value - generated_value).abs() <= COORDINATE_TOLERANCE,
                "shape oracle {}行目{}列目: 保存{stored_value:e} / 生成{generated_value:e}",
                line_index + 1,
                column + 1
            );
        }
    }
}

fn traditional_crane_fixture_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/traditional-crane")
        .join(name)
}

/// 正本CPをそのまま持ち、既存の一括collapse 1手を保存した作品の受け入れ検査。
#[test]
fn traditional_crane_cp_work_matches_reference() {
    const GEOMETRY_TOLERANCE: f64 = 1e-9;
    // 2026-08-26の反射cycle最大残差は4.518468932346309e-11。
    // 実測値を境界にせず、約22倍の余裕を持つモデル共通EPS 1e-9を上限にする(§10.7.9)。
    const COLLAPSE_RESIDUAL_LIMIT: f64 = 1e-9;
    // 保存stepの通常replayはclosure_rms=7.218742174998615e-12を実測した。
    // こちらも実測値を境界にせず、約139倍の1e-9を上限にする。
    const REPLAY_CLOSURE_LIMIT: f64 = 1e-9;

    let work = traditional_crane_collapse_work();
    let document = &work.document;
    let stored_work =
        std::fs::read_to_string(traditional_crane_fixture_path("traditional-crane-cp.ori3"))
            .expect("正本CP作品fixtureを読む");
    let generated_work = traditional_crane_work_json(&work);
    assert_eq!(
        stored_work.replace("\r\n", "\n"),
        generated_work,
        "作品fixtureは正本CSV・既存collapse API・利用者layer oracleから生成したSCHEMA_VERSION=1と全文一致する"
    );

    assert_eq!(document.schema_version, ori3_model::SCHEMA_VERSION);
    assert_eq!(document.cp.vertices.len(), 56);
    assert_eq!(document.cp.edges.len(), 114);
    assert_eq!(document.cp.next_vertex_id, 56);
    assert_eq!(document.cp.next_edge_id, 114);
    let mountain = document
        .cp
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Mountain)
        .count();
    let valley = document
        .cp
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Valley)
        .count();
    let border = document
        .cp
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Border)
        .count();
    assert_eq!((mountain, valley, border), (61, 41, 12));
    let faces = extract_faces(&document.cp);
    assert_eq!(faces.len(), 59);

    let oracle_edges = traditional_crane_edges();
    let positions = vertex_pos(&document.cp);
    let exact_matches = oracle_edges
        .iter()
        .filter(|oracle| {
            let id = oracle
                .id
                .trim_start_matches('e')
                .parse::<u32>()
                .expect("正本edge_id");
            let edge = document
                .cp
                .edges
                .iter()
                .find(|edge| edge.id == id)
                .expect("作品内の正本辺");
            let kind_matches = matches!(
                (oracle.assignment, edge.kind),
                ('M', EdgeKind::Mountain) | ('V', EdgeKind::Valley) | ('B', EdgeKind::Border)
            );
            kind_matches && positions[&edge.v0] == oracle.p0 && positions[&edge.v1] == oracle.p1
        })
        .count();
    assert_eq!(
        exact_matches, 114,
        "正本114辺を端点・向き・M/V/Bまで完全注入する"
    );

    assert_eq!(document.sequence.len(), 1, "一括collapse 1手だけを保存する");
    assert_eq!(document.sequence[0].kind, TechniqueKind::Twist);
    assert_eq!(document.sequence[0].drivers.len(), 102);
    assert_eq!(
        document.sequence[0]
            .layer_order
            .as_ref()
            .expect("collapse層順")
            .len(),
        59
    );
    assert!(work.result.added_edges.is_empty(), "正本CPへ辺を追加しない");
    assert_eq!(
        work.result.warnings.len(),
        1,
        "展開図だけで決まらない重なりを1件の警告へ集約する"
    );
    assert!(
        work.result.warnings[0].starts_with(
            ori3_layers::precrease_collapse::PRECREASE_ORDER_UNDETERMINED_WARNING_PREFIX
        ),
        "内部用語を出さず、未決定の面組を既存warning経路へ出す: {:?}",
        work.result.warnings
    );
    assert_eq!(
        work.collapsed_cp, document.cp,
        "collapseは正本CPを変更しない"
    );
    assert_eq!(work.result.state.order.len(), 59);
    assert_eq!(work.result.state.placements.len(), 59);

    let cycle_residual =
        traditional_crane_collapse_cycle_residual(&document.cp, &faces, &work.result.state);
    assert!(
        cycle_residual <= COLLAPSE_RESIDUAL_LIMIT,
        "collapse反射cycle残差{cycle_residual:e}が上限{COLLAPSE_RESIDUAL_LIMIT:e}を超えた"
    );
    let direct_frame = explicit_flat_frame(document, &faces, &work.result.state);
    assert!(
        !ori3_rigid::layer_order_conflicts(&document.cp, &faces, &direct_frame),
        "collapseのsurface_rankが正本M/Vと矛盾しない"
    );
    assert_eq!(
        self_intersection_pairs(&direct_frame),
        Vec::<(FaceId, FaceId)>::new(),
        "collapse直後のself_intersection_pairs=[]"
    );

    // 保存したDriverLine+layer_orderを平坦状態として読み戻す経路は、APIの直接結果と一致する。
    // 3D solverのsurface_rankは、互いに重ならない面どうしを別の順に並べ得るため、
    // 作品に保存した全順序との比較にはFace3D.layerを使う。
    let (saved_flat_state, flat_warnings) =
        flat_state_at(document, &faces, 1).expect("保存collapse stepの平坦状態");
    assert!(flat_warnings.is_empty(), "平坦状態の読戻し警告なし");
    assert_eq!(saved_flat_state.order, work.result.state.order);
    for face in &faces {
        assert!(
            saved_flat_state.placements[&face.id]
                .approx_eq(&work.result.state.placements[&face.id], GEOMETRY_TOLERANCE),
            "面{}の保存collapse配置",
            face.id
        );
    }

    let replayed = replay(document, 1, 1.0);
    assert!(replayed.skipped.is_empty(), "保存collapse stepを飛ばさない");
    assert!(
        replayed.warnings.iter().all(|warning| {
            !warning.starts_with(
                ori3_layers::precrease_collapse::PRECREASE_ORDER_UNDETERMINED_WARNING_PREFIX,
            ) && !(warning.starts_with("保存された紙の重なり順")
                && warning.contains("採用しません"))
        }),
        "一般制約を満たす保存oracleの通常replayには未決定・不採用警告を出さない: {:?}",
        replayed.warnings
    );
    assert!(
        replayed.converged || replayed.best_effort,
        "有限な再生結果がある"
    );
    assert!(
        replayed.closure_rms <= REPLAY_CLOSURE_LIMIT,
        "保存作品の再生closure_rms={}が上限{}を超えた; warnings={:?}",
        replayed.closure_rms,
        REPLAY_CLOSURE_LIMIT,
        replayed.warnings
    );
    let max_z = replayed
        .frame
        .faces
        .iter()
        .flat_map(|face| &face.polygon)
        .map(|point| point[2].abs())
        .fold(0.0_f64, f64::max);
    assert!(max_z <= GEOMETRY_TOLERANCE, "collapse後のmax|z|={max_z:e}");
    assert_eq!(
        self_intersection_pairs(&replayed.frame),
        Vec::<(FaceId, FaceId)>::new(),
        "保存作品の再生後もself_intersection_pairs=[]"
    );

    let replay_faces = replayed
        .frame
        .faces
        .iter()
        .map(|face| (face.face, face))
        .collect::<HashMap<_, _>>();
    for expected in &direct_frame.faces {
        let actual = replay_faces[&expected.face];
        assert_eq!(actual.layer, expected.surface_rank);
        assert_eq!(actual.mirrored, expected.mirrored);
        assert_eq!(actual.polygon.len(), expected.polygon.len());
        for (actual_point, expected_point) in actual.polygon.iter().zip(&expected.polygon) {
            let delta = DVec3::from(*actual_point).distance(DVec3::from(*expected_point));
            assert!(
                delta <= GEOMETRY_TOLERANCE,
                "面{}のcollapse保存形との差{delta:e}",
                expected.face
            );
        }
    }

    let generated_shape =
        traditional_crane_collapse_shape_csv(document, &faces, &work.result.state);
    let stored_shape = std::fs::read_to_string(traditional_crane_fixture_path(
        "traditional-crane-collapse-oracle.csv",
    ))
    .expect("collapse shape oracleを読む");
    assert_traditional_crane_shape_snapshot(&stored_shape, &generated_shape);
    println!(
        "traditional crane oracle: edges=114/114 M/V/B={mountain}/{valley}/{border} faces={} cycle_residual={cycle_residual:.17e} replay_closure_rms={:.17e} replay_converged={} replay_best_effort={} replay_warnings={} max_z={max_z:.17e}",
        faces.len(),
        replayed.closure_rms,
        replayed.converged,
        replayed.best_effort,
        replayed.warnings.len()
    );
}

/// 保存した層oracleが、正の面積で重なる首・尾を後翼と前翼の間へ置く。
///
/// 部位はFace IDでなく正本material領域への代表点包含で導く。2026-08-27の全数実測は
/// 4分類が各32組、最小正面積0.00232128742142812だった。面積境目1e-12はその約23億分の1で、
/// 接触だけの0面積と十分に分離する(§10.7.9)。旧順は尾/前翼の12組が違反していた。
#[test]
fn traditional_crane_saved_layer_oracle_places_neck_and_tail_between_wings() {
    let work = traditional_crane_collapse_work();
    let faces = extract_faces(&work.document.cp);
    let stored_work =
        std::fs::read_to_string(traditional_crane_fixture_path("traditional-crane-cp.ori3"))
            .expect("正本CP作品fixtureを読む");
    assert_eq!(
        stored_work.replace("\r\n", "\n"),
        traditional_crane_work_json(&work),
        "この検査でreplayするDocumentは保存fixtureと全字段一致する"
    );
    let previous_state = FlatState {
        placements: work.result.state.placements.clone(),
        order: work.generated_order_before_oracle.clone(),
    };
    let previous = traditional_crane_sandwich_audit(&work.document.cp, &faces, &previous_state);
    let corrected = traditional_crane_sandwich_audit(&work.document.cp, &faces, &work.result.state);
    for label in [
        "tail/back_wing",
        "tail/front_wing",
        "neck/back_wing",
        "neck/front_wing",
    ] {
        assert_eq!(
            previous.overlap_counts.get(label),
            Some(&32),
            "旧順の{label}は正面積で重なる全32組"
        );
        assert_eq!(
            corrected.overlap_counts.get(label),
            Some(&32),
            "訂正後の{label}も同じ正面積32組"
        );
    }
    assert_eq!(
        previous.violations.len(),
        12,
        "Face ID tie-breakだった旧順の翼間違反は実測12組"
    );
    assert!(
        previous
            .violations
            .iter()
            .all(|violation| violation.middle_part == "tail"
                && violation.wing_part == "front_wing"),
        "旧12違反は全て尾が前翼より上: {:?}",
        previous
            .violations
            .iter()
            .map(|violation| (
                violation.middle_face,
                violation.wing_face,
                violation.middle_rank,
                violation.wing_rank,
                violation.overlap_area,
            ))
            .collect::<Vec<_>>()
    );
    assert!(
        corrected.violations.is_empty(),
        "訂正後はrank(後翼)<rank(尾/首)<rank(前翼): {:?}",
        corrected
            .violations
            .iter()
            .map(|violation| (
                violation.middle_part,
                violation.wing_part,
                violation.middle_face,
                violation.wing_face,
                violation.middle_rank,
                violation.wing_rank,
                violation.overlap_area,
            ))
            .collect::<Vec<_>>()
    );
    assert!(
        (corrected.minimum_positive_overlap_area - 0.00232128742142812).abs() <= 1e-12,
        "正面積の最小実測値が変わった: {}",
        corrected.minimum_positive_overlap_area
    );

    let (rank_changed, pair_changed) = traditional_crane_order_change_counts(
        &work.generated_order_before_oracle,
        &work.result.state.order,
    );
    assert_eq!(rank_changed, 20, "訂正でsurface_rankが変わる面は20/59");
    assert_eq!(pair_changed, 49, "訂正で上下関係が反転する面対は49組");

    // 直接collapseのstateだけでなく、保存fixtureを通常の1手replayへ通したframeに
    // 刻印されたsurface_rankを監査する。上の全字段一致により、replay対象はfixtureと同一である。
    let replayed = replay(&work.document, 1, 1.0);
    let mut replay_ranks = replayed
        .frame
        .faces
        .iter()
        .map(|face| face.surface_rank)
        .collect::<Vec<_>>();
    replay_ranks.sort_unstable();
    assert_eq!(
        replay_ranks,
        (0..faces.len() as u32).collect::<Vec<_>>(),
        "fixture replayのsurface_rankは0..58の順列"
    );
    let replay_faces = replayed
        .frame
        .faces
        .iter()
        .map(|face| face.face)
        .collect::<HashSet<_>>();
    assert_eq!(
        replay_faces,
        faces.iter().map(|face| face.id).collect::<HashSet<_>>(),
        "fixture replayは正本59面を重複・欠落なく持つ"
    );
    let mut replay_order = replayed
        .frame
        .faces
        .iter()
        .map(|face| (face.surface_rank, face.face))
        .collect::<Vec<_>>();
    replay_order.sort_unstable();
    let replay_state = FlatState {
        placements: work.result.state.placements.clone(),
        order: replay_order.into_iter().map(|(_, face)| face).collect(),
    };
    let replay_audit = traditional_crane_sandwich_audit(&work.document.cp, &faces, &replay_state);
    for label in [
        "tail/back_wing",
        "tail/front_wing",
        "neck/back_wing",
        "neck/front_wing",
    ] {
        assert_eq!(
            replay_audit.overlap_counts.get(label),
            Some(&32),
            "fixture replayの{label}も正面積で重なる全32組"
        );
    }
    assert_eq!(
        replay_audit.overlap_counts.values().sum::<usize>(),
        128,
        "fixture replayでも尾/首と前後翼が正面積で重なる全128組を監査する"
    );
    assert!(
        replay_audit.violations.is_empty(),
        "fixture replayのsurface_rankもrank(後翼)<rank(尾/首)<rank(前翼): {:?}",
        replay_audit.violations
    );
    assert_eq!(
        self_intersection_pairs(&replayed.frame),
        Vec::<(FaceId, FaceId)>::new(),
        "fixture replayもself_intersection_pairs=[]"
    );

    let corrected_frame = explicit_flat_frame(&work.document, &faces, &work.result.state);
    assert_eq!(
        self_intersection_pairs(&corrected_frame),
        Vec::<(FaceId, FaceId)>::new(),
        "層oracle訂正後もself_intersection_pairs=[]"
    );
    println!(
        "traditional crane layer oracle: previous_violations={:?} corrected_violations=0/128 rank_changed={rank_changed}/59 pair_changed={pair_changed} minimum_positive_overlap_area={:.17e}",
        previous
            .violations
            .iter()
            .map(|violation| format!(
                "{} Face{} rank{} / {} Face{} rank{} area={:.17e}",
                violation.middle_part,
                violation.middle_face,
                violation.middle_rank,
                violation.wing_part,
                violation.wing_face,
                violation.wing_rank,
                violation.overlap_area,
            ))
            .collect::<Vec<_>>(),
        corrected.minimum_positive_overlap_area,
    );
}

/// 保存oracleは、候補順自身を証拠にせず、CP・M/V・鏡映・紙の連続性から独立に検証する。
#[test]
fn traditional_crane_saved_layer_oracle_satisfies_all_general_constraints() {
    let work = traditional_crane_collapse_work();
    let faces = extract_faces(&work.document.cp);
    let validation = ori3_layers::precrease_collapse::validate_precrease_layer_order(
        &work.document.cp,
        &faces,
        &work.result.state.placements,
        &work.result.state.order,
    )
    .expect("保存した正本鶴layer oracleの一般制約を検証できる");
    assert!(
        validation.is_valid(),
        "保存oracleは一般制約違反0: {:?}",
        validation.violations
    );
    assert_eq!(
        validation.counts.adjacent_folds, 102,
        "正本B12を除く隣接M/V 102辺を全数検証する"
    );
    assert_eq!(
        validation.counts.taco_tortilla, 987,
        "sampled taco-tortilla 987条件を全数検証する"
    );
    assert_eq!(
        validation.counts.taco_taco, 196,
        "same-side taco-taco 196条件を全数検証する"
    );
    assert_eq!(
        validation.counts.continuous, 0,
        "このCPには0°連続面どうしの平行対応候補が無い"
    );
    assert!(validation.violations.duplicate_faces.is_empty());
    assert!(validation.violations.missing_faces.is_empty());
    assert!(validation.violations.unexpected_faces.is_empty());
    assert!(validation.violations.adjacent_folds.is_empty());
    assert!(validation.violations.taco_tortilla.is_empty());
    assert!(validation.violations.taco_taco.is_empty());
    assert!(validation.violations.continuous_crossings.is_empty());
    assert!(validation.violations.continuous.is_empty());
    assert!(validation.discarded_relations.is_empty());
    assert!(
        !validation.unresolved_overlap_pairs.is_empty(),
        "CPだけでは決まらない面対があるからこそ、明示layer oracleが必要"
    );
    println!(
        "traditional crane general layer constraints: adjacent=0/{} taco_tortilla=0/{} taco_taco=0/{} continuous=0/{} mandatory={} unresolved={} discarded=0",
        validation.counts.adjacent_folds,
        validation.counts.taco_tortilla,
        validation.counts.taco_taco,
        validation.counts.continuous,
        validation.mandatory_constraints.len(),
        validation.unresolved_overlap_pairs.len(),
    );
}

/// 明示的なfixture再生成専用。通常検査は作品とshape oracleを読むだけで上書きしない。
#[test]
#[ignore = "正本CP作品fixtureを明示的に再生成するときだけ実行する"]
fn regenerate_traditional_crane_cp_work_fixture() {
    let work = traditional_crane_collapse_work();
    let faces = extract_faces(&work.document.cp);
    let (rank_changed, pair_changed) = traditional_crane_order_change_counts(
        &work.generated_order_before_oracle,
        &work.result.state.order,
    );
    let work_path = traditional_crane_fixture_path("traditional-crane-cp.ori3");
    let shape_path = traditional_crane_fixture_path("traditional-crane-collapse-oracle.csv");
    std::fs::write(&work_path, traditional_crane_work_json(&work))
        .expect("正本CP作品fixtureを書き出す");
    std::fs::write(
        &shape_path,
        traditional_crane_collapse_shape_csv(&work.document, &faces, &work.result.state),
    )
    .expect("collapse shape oracleを書き出す");
    println!("wrote {}", work_path.display());
    println!("wrote {}", shape_path.display());
    println!(
        "layer oracle correction: changed_surface_ranks={rank_changed}/59 changed_face_pairs={pair_changed}/1711"
    );
}

/// 画面確認用に、完成した鶴の作品ファイル(.ori3)を書き出す。
///
/// 保存先は環境変数 `ORI3_CRANE_OUT` で渡す。依存を増やさないよう、
/// `write_fixture` と同じく必要な項目だけを手書きで出力する。
#[test]
#[ignore = "画面確認用。ORI3_CRANE_OUT=<保存先> を指定して実行する"]
fn write_crane_document_for_screen_check() {
    let Ok(path) = std::env::var("ORI3_CRANE_OUT") else {
        panic!("保存先を ORI3_CRANE_OUT で渡してください");
    };
    let (doc, _) = crane();
    let mut s = String::from("{\n  \"schema_version\":1,\n");
    s.push_str(&format!(
        "  \"paper\":{{\"width_mm\":{:?},\"height_mm\":{:?}}},\n",
        doc.paper.width_mm, doc.paper.height_mm
    ));
    s.push_str("  \"cp\":{\n    \"vertices\":[");
    for (i, v) in doc.cp.vertices.iter().enumerate() {
        s.push_str(&format!(
            "{}{{\"id\":{},\"pos\":[{:?},{:?}]}}",
            if i == 0 { "" } else { "," },
            v.id,
            v.pos[0],
            v.pos[1]
        ));
    }
    s.push_str("],\n    \"edges\":[");
    for (i, e) in doc.cp.edges.iter().enumerate() {
        s.push_str(&format!(
            "{}{{\"id\":{},\"v0\":{},\"v1\":{},\"kind\":\"{:?}\"}}",
            if i == 0 { "" } else { "," },
            e.id,
            e.v0,
            e.v1,
            e.kind
        ));
    }
    s.push_str(&format!(
        "],\n    \"next_vertex_id\":{},\n    \"next_edge_id\":{}\n  }},\n",
        doc.cp.next_vertex_id, doc.cp.next_edge_id
    ));
    s.push_str("  \"sequence\":[");
    for (i, step) in doc.sequence.iter().enumerate() {
        s.push_str(&format!(
            "{}{{\"id\":{},\"kind\":\"{:?}\",\"drivers\":[",
            if i == 0 { "" } else { "," },
            step.id,
            step.kind
        ));
        for (j, d) in step.drivers.iter().enumerate() {
            s.push_str(&format!(
                "{}{{\"a\":[{:?},{:?}],\"b\":[{:?},{:?}],\"target_angle_deg\":{:?}}}",
                if j == 0 { "" } else { "," },
                d.a[0],
                d.a[1],
                d.b[0],
                d.b[1],
                d.target_angle_deg
            ));
        }
        s.push(']');
        if let Some(order) = &step.layer_order {
            s.push_str(",\"layer_order\":[");
            for (j, p) in order.iter().enumerate() {
                s.push_str(&format!(
                    "{}[{:?},{:?}]",
                    if j == 0 { "" } else { "," },
                    p[0],
                    p[1]
                ));
            }
            s.push(']');
        }
        s.push_str(",\"note\":\"\"}");
    }
    s.push_str("]\n}\n");
    std::fs::write(&path, s).expect("作品ファイルを書き出す");
    println!("書き出しました: {path}(手順{}件)", doc.sequence.len());
}

/// 折り目1本を谷折りで−180°まで送っても、紙が閉じたままであること。
///
/// 前の姿勢から連続に追うだけでは閉じた形へ辿り着けない角度があり、−147°〜−162°
/// あたりで閉包RMSが 2.835e-3〜9.177e-3 になっていた(紙が裂けて見える大きさ)。
/// 刻みを5°/2°/1°/0.5°と変えても同じ範囲で起きるため、分割を細かくしても直らない。
/// 閉じた形自体は存在するので、最終要求で閉じなかったときだけ初期値を変えて
/// 解き直すようにした。上限 1e-9 は、この修正後の実測 最悪 3.692e-14 を根拠にする。
#[test]
fn valley_folding_one_crease_to_180_keeps_the_paper_closed() {
    use ori3_rigid::motion::solve_motion;

    let (doc, _) = crane();
    let cp = &doc.cp;
    let faces = extract_faces(cp);
    let creases: Vec<u32> = cp
        .edges
        .iter()
        .filter(|edge| edge.kind != EdgeKind::Border)
        .map(|edge| edge.id)
        .collect();
    // 利用者が実際に選んだ2本と同じ位置づけ(操作中の1本＝hard、もう1本＝希望)
    let (driven, wanted) = (creases[17], creases[20]);
    let mut warm: HashMap<u32, f64> = creases.iter().map(|&edge| (edge, 0.0)).collect();
    let mut worst_rms = 0.0_f64;

    for step in 1..=36u32 {
        let angle = -5.0 * f64::from(step);
        let drivers = vec![Driver {
            hinge: driven,
            target_angle_deg: angle,
        }];
        let targets: HashMap<u32, f64> = HashMap::from([(wanted, angle)]);
        let solved = solve_motion(cp, &faces, &drivers, Some(&targets), Some(&warm), true).result;
        assert!(
            solved.converged,
            "{angle}°で紙が閉じない(閉包RMS {:.3e}、警告 {:?})",
            solved.closure_rms, solved.frame.warnings
        );
        worst_rms = worst_rms.max(solved.closure_rms);
        warm = solved.angles.clone();
    }

    assert!(
        worst_rms < 1e-9,
        "36段すべてで閉じるが、最悪の閉包RMSが大きすぎる: {worst_rms:.3e}"
    );
}

#[test]
fn saved_order_never_overrides_geometric_rank_across_angle_buckets() {
    let (base, _) = crane();
    let faces = extract_faces(&base.cp);
    let positions = base
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.pos))
        .collect::<HashMap<_, _>>();
    let samples = base
        .cp
        .edges
        .iter()
        .filter(|edge| edge.kind != EdgeKind::Border)
        .take(20)
        .map(|edge| DriverLine {
            a: positions[&edge.v0],
            b: positions[&edge.v1],
            target_angle_deg: 0.0,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        samples.len(),
        20,
        "angle sweep needs twenty distinct crane hinges"
    );

    let other = [
        -170.0, -150.0, -135.0, -120.0, -75.0, -60.0, -45.0, -30.0, -15.0, -1.0, 1.0, 15.0, 30.0,
        45.0, 60.0, 75.0, 120.0, 135.0, 150.0, 170.0,
    ];
    let fixed = [
        ("0", 0.0),
        ("+90", 90.0),
        ("-90", -90.0),
        ("+179", 179.0),
        ("-179", -179.0),
        ("+180", 180.0),
        ("-180", -180.0),
    ];
    for bucket_index in 0..=fixed.len() {
        let label = fixed.get(bucket_index).map_or("other", |(label, _)| *label);
        let mut numbered_cases = 0usize;
        let mut forced_faces = Vec::new();
        let mut rank_diff_faces = Vec::new();
        let mut converged = 0usize;
        let mut seam_ok = 0usize;
        let mut authoritative = 0usize;
        let mut face_id_order_cases = 0usize;
        let mut max_vertex_delta = 0.0_f64;
        for (sample_index, template) in samples.iter().enumerate() {
            let angle = fixed
                .get(bucket_index)
                .map_or(other[sample_index], |(_, angle)| *angle);
            let mut saved_doc = base.clone();
            let mut driver = template.clone();
            driver.target_angle_deg = angle;
            saved_doc.sequence.push(FoldStep {
                id: u32::try_from(saved_doc.sequence.len()).expect("step count fits"),
                kind: TechniqueKind::Simple,
                drivers: vec![driver],
                layer_order: None,
                alignment: None,
                finish_soft: None,
                note: String::new(),
            });
            let mut geometric_doc = saved_doc.clone();
            for step in &mut geometric_doc.sequence {
                step.layer_order = None;
            }
            let saved = replay(&saved_doc, saved_doc.sequence.len(), 1.0);
            let geometric = replay(&geometric_doc, geometric_doc.sequence.len(), 1.0);
            assert_eq!(
                saved.surface_order_provenance.is_some(),
                geometric.surface_order_provenance.is_some(),
                "{label} sample {sample_index}: 保存順の有無で幾何authorityが変わった"
            );
            authoritative += usize::from(saved.surface_order_provenance.is_some());
            assert!(
                saved.skipped.is_empty() && geometric.skipped.is_empty(),
                "{label} sample {sample_index}: 比較対象の手順を飛ばした"
            );
            let saved_ranks = saved
                .frame
                .faces
                .iter()
                .map(|face| (face.face, face.surface_rank))
                .collect::<BTreeMap<_, _>>();
            let geometric_ranks = geometric
                .frame
                .faces
                .iter()
                .map(|face| (face.face, face.surface_rank))
                .collect::<BTreeMap<_, _>>();
            let complete_ranks = |ranks: &BTreeMap<u32, u32>| {
                let mut values = ranks.values().copied().collect::<Vec<_>>();
                values.sort_unstable();
                values
                    .iter()
                    .enumerate()
                    .all(|(rank, &value)| u32::try_from(rank).ok() == Some(value))
            };
            assert!(
                complete_ranks(&saved_ranks) && complete_ranks(&geometric_ranks),
                "{label} sample {sample_index}: surface_rankが完全順列でない"
            );
            let forced = saved_ranks
                .iter()
                .filter(|&(face, rank)| *rank == *face && geometric_ranks[face] != *face)
                .count();
            let differences = saved_ranks
                .iter()
                .filter(|&(face, rank)| geometric_ranks[face] != *rank)
                .count();
            face_id_order_cases +=
                usize::from(saved_ranks.iter().all(|(face, rank)| *face == *rank));
            numbered_cases += usize::from(forced > 0);
            forced_faces.push(forced);
            rank_diff_faces.push(differences);
            converged += usize::from(saved.converged && geometric.converged);
            let saved_seam = max_seam_gap(&base.cp, &faces, &saved.frame);
            let geometric_seam = max_seam_gap(&base.cp, &faces, &geometric.frame);
            seam_ok += usize::from(saved_seam < 1e-6 && geometric_seam < 1e-6);
            for (left, right) in saved.frame.faces.iter().zip(&geometric.frame.faces) {
                assert_eq!(left.face, right.face);
                for (a, b) in left.polygon.iter().zip(&right.polygon) {
                    max_vertex_delta = max_vertex_delta.max(
                        a.iter()
                            .zip(b)
                            .map(|(x, y)| (x - y).abs())
                            .fold(0.0_f64, f64::max),
                    );
                }
            }
        }
        println!(
            "SURFACE_ORDER_CRANE bucket={label} total=20 numbered_cases={numbered_cases} face_id_order_cases={face_id_order_cases} authoritative={authoritative} forced_faces_min={} forced_faces_max={} forced_faces_sum={} rank_diff_min={} rank_diff_max={} converged={converged} seam_ok={seam_ok} max_vertex_delta={max_vertex_delta:.3e}",
            forced_faces.iter().copied().min().unwrap_or(0),
            forced_faces.iter().copied().max().unwrap_or(0),
            forced_faces.iter().sum::<usize>(),
            rank_diff_faces.iter().copied().min().unwrap_or(0),
            rank_diff_faces.iter().copied().max().unwrap_or(0),
        );
        assert_eq!(
            numbered_cases, 0,
            "{label}: 保存順が幾何には無い面番号一致を作った姿勢がある"
        );
        assert_eq!(
            face_id_order_cases, 0,
            "{label}: surface_rank全体がFaceId順になった姿勢がある"
        );
        assert!(
            forced_faces.iter().all(|&count| count == 0),
            "{label}: 保存順が面番号一致を強制した: {forced_faces:?}"
        );
        assert!(
            rank_diff_faces.iter().all(|&count| count == 0),
            "{label}: 保存順あり/なしでsurface rankが変わった: {rank_diff_faces:?}"
        );
        assert_eq!(
            max_vertex_delta, 0.0,
            "rank authority must not change geometry"
        );
    }
}
