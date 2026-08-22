//! 紙上の太さ係数と、折り上がり探索で測る太さの目盛りを結ぶ検査。
//!
//! 骨格・画面の `width_factor` は先端の半開き角を `theta` とした `tan(theta)`、
//! [`FoldGoal::measure`] が返す太さは `2 sin(theta)` である。折り上がり探索の入口だけが
//! `2w / sqrt(1 + w^2)` へ変換し、紙上測定は生の `w` を保つことを固定する。

use ori3_model::{CreasePattern, Document, Edge, EdgeKind, Paper, Vertex};
use ori3_propose::skeleton::{Skeleton, SkeletonNode};
use ori3_propose::{FLAP_RADIUS, FinishTarget, FoldGoal, TipSite, width_gap};

/// 以前の30度・45度くさびで測った変換後gapは最大`8.89e-16`だった。
/// `1e-9`はその100万倍以上の余裕があり、利用者の指定と実測が同じ目盛りかだけを見る。
const WIDTH_GAP_TOL: f64 = 1e-9;
/// 独立な三角関数による換算値との照合。計算値を厳密一致では比べない。
const CONVERSION_TOL: f64 = 1e-12;

fn one_tip_skeleton(width_factor: f64) -> Skeleton {
    let mut tip = SkeletonNode::new(1, Some(0), 1.0);
    tip.width_factor = width_factor;
    Skeleton {
        nodes: vec![SkeletonNode::new(0, None, 0.0), tip],
    }
}

fn shifted(point: [f64; 2], direction: [f64; 2], distance: f64) -> [f64; 2] {
    [
        point[0] + distance * direction[0],
        point[1] + distance * direction[1],
    ]
}

/// 紙上の半径/長さが`width_factor`になる、平らな対称くさびを作る。
///
/// `FoldGoal`は先端の周囲を15度刻みで測る。任意の`width_factor`で量子化誤差を
/// 混ぜないよう、くさびの片側を常に180度の測定方向へ合わせ、くさび全体を回す。
/// 形は変えず向きだけを変えるので、半開き角は`atan(width_factor)`のままである。
fn wedge(width_factor: f64) -> (Document, [f64; 2], [f64; 2]) {
    let theta = width_factor.atan();
    let inward = [-theta.cos(), theta.sin()];
    let sideways = [-inward[1], inward[0]];
    let tip = [0.5, 0.5];
    let base_center = shifted(tip, inward, FLAP_RADIUS);
    let side = FLAP_RADIUS * width_factor;
    let aligned_edge = shifted(base_center, sideways, side);
    let other_edge = shifted(base_center, sideways, -side);
    let body = shifted(tip, inward, 0.5 * FLAP_RADIUS);

    let cp = CreasePattern {
        // 反時計回り。3辺とも紙の輪郭で、折り目は無い。
        vertices: vec![
            Vertex {
                id: 0,
                pos: aligned_edge,
            },
            Vertex { id: 1, pos: tip },
            Vertex {
                id: 2,
                pos: other_edge,
            },
        ],
        edges: vec![
            Edge {
                id: 0,
                v0: 0,
                v1: 1,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 1,
                v0: 1,
                v1: 2,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 2,
                v0: 2,
                v1: 0,
                kind: EdgeKind::Border,
            },
        ],
        next_vertex_id: 3,
        next_edge_id: 3,
    };
    let mut document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    document.cp = cp;
    (document, body, tip)
}

#[test]
fn five_width_factors_match_the_same_artificial_wedge_measurement() {
    for width_factor in [0.25, 0.5, 1.0, 2.0, 4.0] {
        let skeleton = one_tip_skeleton(width_factor);
        let target = FinishTarget::from_skeleton(&skeleton);
        let target_width = target.tip(1).expect("先端1の目標が無い").width;

        // 実装の代数式を繰り返さず、w=tan(theta)から独立に2sin(theta)を求める。
        let independent = 2.0 * width_factor.atan().sin();
        let conversion_delta = (target_width - independent).abs();
        assert!(
            conversion_delta < CONVERSION_TOL,
            "width_factor={width_factor}: 折り上がり目標{target_width:.15}が独立換算{independent:.15}と違う"
        );

        let (document, body, material) = wedge(width_factor);
        let goal = FoldGoal {
            target,
            body,
            sites: vec![TipSite {
                leaf_id: 1,
                material,
            }],
        };
        let measured = goal.measure(&document);
        let measured_width = measured.tip(1).expect("先端1を測れなかった").width;
        let gap = width_gap(&goal.target, &measured);
        println!(
            "width_factor={width_factor:.2} target={target_width:.15} measured={measured_width:.15} conversion_delta={conversion_delta:.3e} width_gap={gap:.3e}"
        );
        assert!(
            gap < WIDTH_GAP_TOL,
            "width_factor={width_factor}: 変換済み目標{target_width:.15}と実測{measured_width:.15}のgap={gap:.3e}"
        );
    }
}

#[test]
fn paper_target_keeps_the_unconverted_width_factor() {
    for width_factor in [0.25, 0.5, 1.0, 2.0, 4.0] {
        let skeleton = one_tip_skeleton(width_factor);
        let paper_width = FinishTarget::from_skeleton_on_paper(&skeleton)
            .tip(1)
            .expect("先端1の紙上目標が無い")
            .width;
        assert!(
            (paper_width - width_factor).abs() < CONVERSION_TOL,
            "紙上目標まで変換された: 指定{width_factor:.15}、目標{paper_width:.15}"
        );
    }
}

#[test]
fn the_largest_finite_width_factor_stays_finite_and_below_two() {
    let skeleton = one_tip_skeleton(f64::MAX);
    skeleton
        .validate_structure()
        .expect("有限な太さ係数に新しい上限を作ってはならない");
    let folded = FinishTarget::from_skeleton(&skeleton)
        .tip(1)
        .expect("先端1の折り上がり目標が無い")
        .width;
    println!("width_factor=f64::MAX folded_target={folded:.17}");

    assert!(
        folded.is_finite(),
        "最大の有限入力から有限値が返らない: {folded}"
    );
    assert!(folded > 0.0, "最大の有限入力から正の値が返らない: {folded}");
    assert!(folded < 2.0, "有限入力の漸近上限2へ到達した: {folded}");
    assert!(
        2.0 - folded < CONVERSION_TOL,
        "極端に太い入力が漸近上限2へ近づかない: {folded}"
    );
}
