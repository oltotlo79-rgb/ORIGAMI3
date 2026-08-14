//! 充填結果からの展開図生成(PRO-002/PRO-003 / 要件§8-3・§8-4)のテスト。

use std::collections::{BTreeMap, BTreeSet};

use ori3_cp::{extract_faces, validate};
use ori3_model::{CreasePattern, EdgeKind};
use ori3_propose::skeleton::{Skeleton, SkeletonNode};
use ori3_propose::{ProposalResult, generate, pack};

/// 根に葉を`n`本ぶら下げた星形の骨格。
fn star(n: u32, len: f64) -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    for i in 1..=n {
        nodes.push(SkeletonNode::new(i, Some(0), len));
    }
    Skeleton { nodes }
}

/// 「頭1・尾1・足4」の骨格。根0が胴、1=頭、2=尾、3〜6=足。
fn crane_like() -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    nodes.push(SkeletonNode::new(1, Some(0), 1.0));
    nodes.push(SkeletonNode::new(2, Some(0), 1.0));
    for i in 3..=6 {
        nodes.push(SkeletonNode::new(i, Some(0), 0.7));
    }
    Skeleton { nodes }
}

/// 骨格を充填して展開図まで作る一連の流れ。
fn build(s: &Skeleton, w: f64, h: f64, seed: u64) -> ProposalResult {
    let ps = pack(s, w, h, seed, 8);
    assert!(!ps.is_empty(), "充填に失敗した");
    generate(s, &ps[0], w, h).expect("生成に失敗した")
}

/// 頂点ごとの折り線の本数(補助線は無いので全辺を数える)。
fn degrees(cp: &CreasePattern) -> BTreeMap<u32, usize> {
    let mut d = BTreeMap::new();
    for e in &cp.edges {
        *d.entry(e.v0).or_default() += 1;
        *d.entry(e.v1).or_default() += 1;
    }
    d
}

/// 紙の縁に触れず、折り線が2本以上集まる点(局所平坦折り判定の対象)の数。
fn interior_vertex_count(cp: &CreasePattern) -> usize {
    let mut border = BTreeSet::new();
    for e in cp.edges.iter().filter(|e| e.kind == EdgeKind::Border) {
        border.insert(e.v0);
        border.insert(e.v1);
    }
    degrees(cp)
        .into_iter()
        .filter(|(v, d)| !border.contains(v) && *d >= 2)
        .count()
}

#[test]
fn four_leaves_and_a_body_give_a_valid_planar_graph() {
    // 要件§8の指定: 葉4+胴1の充填結果から作った展開図が妥当な平面グラフで、
    // 面が2つ以上に分かれ、違反数が結果として返ること。
    let r = build(&star(4, 1.0), 1.0, 1.0, 2026);
    let faces = extract_faces(&r.cp);
    assert!(faces.len() >= 2, "面が分かれていない: {}", faces.len());
    // オイラーの公式 V − E + F = 1(外周面を除く)で平面グラフの整合を検算する。
    let (v, e, f) = (r.cp.vertices.len(), r.cp.edges.len(), faces.len());
    assert_eq!(
        v + f,
        e + 1,
        "平面グラフとして辻褄が合わない: V{v} E{e} F{f}"
    );
    assert!(r.violations <= interior_vertex_count(&r.cp));
}

#[test]
fn generated_cp_passes_cp_validate() {
    // 参照切れ・潰れた辺・非連結・辺の交差のいずれも出ないこと。
    for n in [2u32, 4, 6, 9, 12] {
        let r = build(&star(n, 1.0), 1.0, 0.75, 11);
        let w = validate(&r.cp);
        assert!(w.is_empty(), "角{n}本で展開図の点検に引っかかった: {w:?}");
    }
}

#[test]
fn depth_three_branching_skeleton_packs_and_generates_valid_cp() {
    // 0=胴、1=途中、その先に3・6・7が分岐する深さ3の形。
    // 2・4・5は胴から直接出し、従来の放射状の部分も同じ入力に残す。
    let s = Skeleton {
        nodes: vec![
            SkeletonNode::new(0, None, 0.0),
            SkeletonNode::new(1, Some(0), 0.3),
            SkeletonNode::new(2, Some(0), 1.0),
            SkeletonNode::new(3, Some(1), 1.0),
            SkeletonNode::new(4, Some(0), 0.6),
            SkeletonNode::new(5, Some(0), 0.6),
            SkeletonNode::new(6, Some(1), 0.6),
            SkeletonNode::new(7, Some(1), 0.6),
        ],
    };
    assert_eq!(
        s.nodes.iter().filter(|node| node.parent == Some(1)).count(),
        3,
        "途中から出る先端が3本でない"
    );

    let candidates = pack(&s, 1.0, 1.0, 2026, 8);
    let candidate_count = candidates.len();
    assert!(candidate_count >= 1, "候補が{candidate_count}件しかない");

    let result = generate(&s, &candidates[0], 1.0, 1.0).expect("深さ3の形から生成できない");
    let fold_line_count = result
        .cp
        .edges
        .iter()
        .filter(|edge| edge.kind != EdgeKind::Border)
        .count();
    assert!(
        fold_line_count >= 1,
        "輪郭以外の折り線が{fold_line_count}本しかない"
    );

    let validation_warnings = validate(&result.cp);
    assert_eq!(
        validation_warnings.len(),
        0,
        "展開図の点検警告が{}件ある: {validation_warnings:?}",
        validation_warnings.len()
    );
}

#[test]
fn one_to_twelve_leaves_do_not_panic() {
    for n in 1..=12u32 {
        for (w, h) in [(1.0, 1.0), (1.0, 0.6)] {
            let r = build(&star(n, 1.0), w, h, 5);
            assert!(!r.cp.edges.is_empty(), "角{n}本・紙{w}x{h}で辺が無い");
            assert!(extract_faces(&r.cp).len() >= 2);
        }
    }
}

#[test]
fn same_input_gives_same_cp() {
    let s = crane_like();
    let p = pack(&s, 1.0, 1.0, 2026, 8).remove(0);
    let a = generate(&s, &p, 1.0, 1.0).unwrap();
    let b = generate(&s, &p, 1.0, 1.0).unwrap();
    assert_eq!(a, b, "同じ入力から違う展開図ができた");
}

#[test]
fn broken_input_is_rejected_with_japanese_message() {
    let s = star(3, 1.0);
    let p = pack(&s, 1.0, 1.0, 1, 4).remove(0);
    assert!(generate(&s, &p, 0.0, 1.0).is_err(), "紙幅0が通ってしまった");
    let broken = Skeleton { nodes: Vec::new() };
    let e = generate(&broken, &p, 1.0, 1.0).unwrap_err();
    assert!(
        !e.is_empty() && !e.is_ascii(),
        "日本語のエラー文になっていない"
    );
}

#[test]
fn oversized_circle_is_reported_as_a_warning_not_an_error() {
    // 紙に対して角が長すぎる骨格。円は紙からはみ出すが、生成は続行する。
    let r = build(&star(2, 5.0), 1.0, 1.0, 3);
    assert!(
        r.warnings.iter().any(|w| w.contains("はみ出")),
        "はみ出しの警告が出ていない: {:?}",
        r.warnings
    );
    assert!(extract_faces(&r.cp).len() >= 2);
}

/// 要件§8の精度目標「頭1・尾1・足4の骨格で鶴系の基本形に相当する折り可能な
/// 展開図が得られること」の検証。
///
/// 「相当する」の判定根拠(厳密な折りたたみ試行はM2の層計算の仕事なので、
/// ここでは展開図の構造で代用する):
/// (a) 6本の角すべてが、円中心から3本以上の折り線が放射状に出る点として
///     展開図に現れる。= 6枚のフラップの先端が確保されている。
/// (b) 紙が「角の本数×2」以上の面に分かれている。鶴の基本形(正方基本形から
///     の花弁折り)と同程度以上に細分されていること。
/// (c) 展開図が平面グラフとして妥当(オイラーの公式が成り立ち、ori3-cpの
///     点検で参照切れ・交差・非連結が出ない)。
/// (d) 局所平坦折り判定の違反が内部頂点の半数以下に収まっている。
///     (完全な平坦折り可能性は要件で保証しないため「0」は求めない)
#[test]
fn crane_like_skeleton_yields_a_crane_class_base() {
    let s = crane_like();
    let r = build(&s, 1.0, 1.0, 2026);
    let ps = pack(&s, 1.0, 1.0, 2026, 8);
    let deg = degrees(&r.cp);

    // (a) 6本の角それぞれの円中心から折り線が放射状に出ている。
    for &(id, c) in &ps[0].centers {
        let v =
            r.cp.vertices
                .iter()
                .find(|v| (v.pos[0] - c[0]).hypot(v.pos[1] - c[1]) < 1e-7)
                .unwrap_or_else(|| panic!("角{id}の円中心が展開図の頂点になっていない"));
        let d = deg.get(&v.id).copied().unwrap_or(0);
        assert!(d >= 3, "角{id}から出る折り線が{d}本しかない");
    }

    // (b) 面の細分。
    let faces = extract_faces(&r.cp);
    assert!(faces.len() >= 12, "面が{}枚しかない", faces.len());

    // (c) 平面グラフとしての妥当性。
    let (v, e, f) = (r.cp.vertices.len(), r.cp.edges.len(), faces.len());
    assert_eq!(
        v + f,
        e + 1,
        "平面グラフとして辻褄が合わない: V{v} E{e} F{f}"
    );
    assert!(validate(&r.cp).is_empty(), "点検: {:?}", validate(&r.cp));

    // (d) 平坦折り違反が内部頂点の半数以下。
    let interior = interior_vertex_count(&r.cp);
    assert!(
        r.violations * 2 <= interior,
        "違反{}個 / 内部頂点{interior}個は多すぎる",
        r.violations
    );
}

#[test]
fn four_corner_circles_fold_flat_locally() {
    // 角4本を正方形の四隅へ置いた最も素直な配置。鶴の基本形の土台にあたる
    // 「対角線1本 + 各半分のウサギ耳」になり、局所平坦折り違反は0になる。
    let r = build(&star(4, 1.0), 1.0, 1.0, 7);
    assert_eq!(r.violations, 0, "警告: {:?}", r.warnings);
    assert!(
        r.cp.edges.iter().any(|e| e.kind == EdgeKind::Mountain)
            && r.cp.edges.iter().any(|e| e.kind == EdgeKind::Valley),
        "山と谷が両方できていない"
    );
}
