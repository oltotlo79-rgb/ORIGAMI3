//! 充填の性能確認。骨格12葉・8スタートが実用的な時間で終わること。

use ori3_propose::skeleton::{Skeleton, SkeletonNode};
use std::time::Instant;

fn star(n: u32, len: f64) -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    for i in 1..=n {
        nodes.push(SkeletonNode::new(i, Some(0), len));
    }
    Skeleton { nodes }
}

#[test]
fn twelve_leaves_eight_starts_is_fast() {
    let s = star(12, 1.0);
    // 初回のウォームアップ(ページフォールト等を測定から除く)
    let _ = ori3_propose::pack(&s, 1.0, 1.0, 0, 8);

    let t = Instant::now();
    let out = ori3_propose::pack(&s, 1.0, 1.0, 1, 8);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    eprintln!("12葉・8スタート: {ms:.1}ms 最良縮尺={:.4}", out[0].scale);

    // 目安は数百ms以内。debugビルドでも余裕を見て1秒を上限にする。
    assert!(ms < 1000.0, "充填が遅すぎる: {ms:.1}ms");
    assert_eq!(out.len(), 4);
}
