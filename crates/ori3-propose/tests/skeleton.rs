//! 骨格モデル(PRO-001)のテスト。

use ori3_propose::skeleton::{Skeleton, SkeletonNode};

/// 頭1・尾1・足4 + 胴の骨格(要件§8の精度目標に出てくる形)。
fn bird_base() -> Skeleton {
    // 0=根(胸) 1=胴(腰へ) 2=頭 3=尾 4..7=足
    let mut nodes = vec![
        SkeletonNode::new(0, None, 0.0),
        SkeletonNode::new(1, Some(0), 0.4),
        SkeletonNode::new(2, Some(0), 1.0),
        SkeletonNode::new(3, Some(1), 1.0),
    ];
    for i in 0..4u32 {
        nodes.push(SkeletonNode::new(4 + i, Some(if i < 2 { 0 } else { 1 }), 0.7));
    }
    Skeleton { nodes }
}

#[test]
fn valid_skeleton_passes_and_lists_leaves() {
    let s = bird_base();
    assert_eq!(s.validate(), Ok(()));
    assert_eq!(s.root(), Some(0));
    let mut leaves = s.leaves();
    leaves.sort_unstable();
    assert_eq!(leaves, vec![2, 3, 4, 5, 6, 7]);
}

#[test]
fn leaf_distance_includes_river_widths_on_path() {
    let s = bird_base();
    // 頭(2)と尾(3): 1.0 + 胴0.4 + 1.0
    assert!((s.leaf_distance(2, 3) - 2.4).abs() < 1e-12);
    // 前足(4)と後足(6): 0.7 + 胴0.4 + 0.7
    assert!((s.leaf_distance(4, 6) - 1.8).abs() < 1e-12);
    // 同じ親を持つ前足2本: 0.7 + 0.7
    assert!((s.leaf_distance(4, 5) - 1.4).abs() < 1e-12);
    assert_eq!(s.leaf_distance(2, 2), 0.0);
}

#[test]
fn width_factor_enlarges_circle_radius() {
    let mut s = bird_base();
    s.nodes[2].width_factor = 1.5; // 頭を太くする
    assert!((s.leaf_radius(2) - 1.5).abs() < 1e-12);
    assert!((s.leaf_distance(2, 3) - 2.9).abs() < 1e-12);
    assert_eq!(s.validate(), Ok(()));
}

#[test]
fn cyclic_skeleton_is_rejected() {
    let s = Skeleton {
        nodes: vec![
            SkeletonNode::new(0, None, 0.0),
            SkeletonNode::new(1, Some(2), 1.0),
            SkeletonNode::new(2, Some(1), 1.0),
        ],
    };
    let err = s.validate().unwrap_err();
    assert!(err.contains("循環"), "{err}");
}

#[test]
fn missing_parent_is_rejected() {
    let s = Skeleton {
        nodes: vec![
            SkeletonNode::new(0, None, 0.0),
            SkeletonNode::new(1, Some(99), 1.0),
        ],
    };
    let err = s.validate().unwrap_err();
    assert!(err.contains("見つかりません"), "{err}");
}

#[test]
fn too_many_leaves_is_rejected() {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    for i in 1..=13u32 {
        nodes.push(SkeletonNode::new(i, Some(0), 1.0));
    }
    let err = Skeleton { nodes }.validate().unwrap_err();
    assert!(err.contains("12本まで"), "{err}");
}

#[test]
fn non_positive_length_or_width_is_rejected() {
    let bad_len = Skeleton {
        nodes: vec![
            SkeletonNode::new(0, None, 0.0),
            SkeletonNode::new(1, Some(0), 0.0),
        ],
    };
    assert!(bad_len.validate().unwrap_err().contains("長さ"));

    let mut bad_width = bird_base();
    bad_width.nodes[2].width_factor = -1.0;
    assert!(bad_width.validate().unwrap_err().contains("太さ"));
}

#[test]
fn bad_node_or_root_count_is_rejected() {
    assert!(Skeleton::default().validate().unwrap_err().contains("節点"));

    let two_roots = Skeleton {
        nodes: vec![
            SkeletonNode::new(0, None, 0.0),
            SkeletonNode::new(1, None, 0.0),
        ],
    };
    assert!(two_roots.validate().unwrap_err().contains("根"));

    let dup = Skeleton {
        nodes: vec![
            SkeletonNode::new(0, None, 0.0),
            SkeletonNode::new(0, Some(0), 1.0),
        ],
    };
    assert!(dup.validate().unwrap_err().contains("重複"));

    let lone_root = Skeleton {
        nodes: vec![SkeletonNode::new(0, None, 0.0)],
    };
    assert!(lone_root.validate().unwrap_err().contains("角"));
}

#[test]
fn serde_roundtrip_preserves_skeleton() {
    let s = bird_base();
    let json = serde_json::to_string(&s).unwrap();
    let back: Skeleton = serde_json::from_str(&json).unwrap();
    assert_eq!(s, back);
}
