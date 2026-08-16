//! 「紙内包含」を中心だけで見るか(案A)、円全体で見るか(案B)を決めるための測定。
//!
//! 判断そのものはここでは行わない。測った値だけを残す。
//! 報告書は `scratchpad/containment-report.md`。
//!
//! # 案Bの充填を、案Aと1か所だけ変えて用意する
//!
//! `crates/ori3-propose/src/packing.rs` は製品コードなので変更しない。ここでは
//! 同じ手順(初期配置→押し離し掃引→縮尺の二分探索→揺さぶり直し)を写し取り、
//! **紙内包含の判定だけ**を「中心が紙内」から「円全体が紙内」へ替えた版を置く。
//! 変える量を1つに限るのは `CLAUDE.md` §10.7.4のため。
//!
//! # 折って測った値の出どころ
//!
//! `bird_base_*` と `frog_base_*` の実測値は、`crates/ori3-layers` の折り操作だけで
//! 折った鶴の基本形・カエルの基本形を、平らな折り上がりの上で測ったもの。
//! 折る手順と測り方は `crates/ori3-layers/tests/acceptance_crane.rs` の
//! `bird_base_lifts_two_slender_points` と、`crates/ori3-layers/tests/acceptance_frog.rs` の
//! `petal_folding_the_kite_makes_the_frog_base` /
//! `the_frog_base_is_a_45_degree_kite_of_half_diagonal` にそのまま入っている。
//! `ori3-propose` は `ori3-layers` に依存していないため(依存追加は `CLAUDE.md` §5で
//! 要承認)、ここでは実測値を定数として置き、充填の制約式との突き合わせだけを行う。

use ori3_propose::packing::{PACK_TOL, Packing, pack};
use ori3_propose::skeleton::{Skeleton, SkeletonNode};

use rand::{RngExt, SeedableRng, rngs::StdRng};

const SQRT2: f64 = std::f64::consts::SQRT_2;

/// 折って測った鶴の基本形の細い先(首・尾)の長さ。理論値 1-√2/2。
const BIRD_SLENDER: f64 = 1.0 - 0.5 * SQRT2;
/// 折って測った鶴の基本形の広いフラップ(羽)の長さ。理論値 √2/2。
const BIRD_WIDE: f64 = 0.5 * SQRT2;
/// 折って測ったカエルの基本形の出っぱりの長さ。理論値 √2/4。
const FROG_LIMB: f64 = 0.25 * SQRT2;

/// 根に葉を`n`本だけぶら下げた星形の骨格(各辺の長さは`len`、太さ1.0)。
fn star(n: u32, len: f64) -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    for i in 1..=n {
        nodes.push(SkeletonNode::new(i, Some(0), len));
    }
    Skeleton { nodes }
}

/// 根に、指定した長さの葉をぶら下げた骨格。
fn fan(lengths: &[f64]) -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    for (i, &len) in lengths.iter().enumerate() {
        nodes.push(SkeletonNode::new(1 + i as u32, Some(0), len));
    }
    Skeleton { nodes }
}

/// 紙に丸ごと収まる円の最大半径(短辺の半分)。
fn half_short_side(w: f64, h: f64) -> f64 {
    0.5 * w.min(h)
}

/// 円の中心だけを紙内に置く案Aの制約違反(`src/packing.rs` の `violation_of` と同じ式)。
fn violation_center(s: &Skeleton, p: &Packing, w: f64, h: f64) -> f64 {
    let mut v: f64 = 0.0;
    for (ia, &(id_a, a)) in p.centers.iter().enumerate() {
        v = v.max(-a[0]).max(a[0] - w).max(-a[1]).max(a[1] - h);
        for &(id_b, b) in &p.centers[ia + 1..] {
            let need = p.scale * s.leaf_distance(id_a, id_b);
            v = v.max(need - (a[0] - b[0]).hypot(a[1] - b[1]));
        }
    }
    v.max(0.0)
}

/// 円全体を紙内に収める案Bの制約違反。対の条件は案Aと同じで、境界だけが違う。
fn violation_full(s: &Skeleton, p: &Packing, w: f64, h: f64) -> f64 {
    let mut v: f64 = 0.0;
    for (ia, &(id_a, a)) in p.centers.iter().enumerate() {
        let r = p.scale * s.leaf_radius(id_a);
        v = v
            .max(r - a[0])
            .max(a[0] + r - w)
            .max(r - a[1])
            .max(a[1] + r - h);
        for &(id_b, b) in &p.centers[ia + 1..] {
            let need = p.scale * s.leaf_distance(id_a, id_b);
            v = v.max(need - (a[0] - b[0]).hypot(a[1] - b[1]));
        }
    }
    v.max(0.0)
}

// ---------------------------------------------------------------------------
// 案Bの充填(製品の `pack` を写し、紙内包含だけを替えたもの)
// ---------------------------------------------------------------------------

const BISECT_STEPS: usize = 28;
const RELAX_SWEEPS: usize = 48;
const SHAKE_ROUNDS: usize = 3;
const MAX_CANDIDATES: usize = 4;

struct FullProblem {
    ids: Vec<u32>,
    radii: Vec<f64>,
    pairs: Vec<(usize, usize, f64)>,
    w: f64,
    h: f64,
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

impl FullProblem {
    fn new(skeleton: &Skeleton, w: f64, h: f64) -> Self {
        let ids = skeleton.leaves();
        let radii: Vec<f64> = ids.iter().map(|&id| skeleton.leaf_radius(id)).collect();
        let mut pairs = Vec::new();
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                pairs.push((i, j, skeleton.leaf_distance(ids[i], ids[j])));
            }
        }
        Self {
            ids,
            radii,
            pairs,
            w,
            h,
        }
    }

    /// この配置で成り立つ最大の縮尺。対の条件に加えて、円全体が紙に収まる条件も見る。
    fn scale_of(&self, c: &[[f64; 2]]) -> f64 {
        let mut s = f64::INFINITY;
        for &(i, j, d) in &self.pairs {
            if d > 0.0 {
                s = s.min(dist(c[i], c[j]) / d);
            }
        }
        for (p, &r) in c.iter().zip(self.radii.iter()) {
            if r > 0.0 {
                let margin = p[0].min(self.w - p[0]).min(p[1]).min(self.h - p[1]);
                s = s.min(margin / r);
            }
        }
        s.max(0.0)
    }

    /// 目標縮尺 `s` のときに、円全体が紙へ収まる範囲へ中心を押し戻す。
    fn clamp(&self, i: usize, p: &mut [f64; 2], s: f64) {
        let r = self.radii[i] * s;
        let (lo_x, hi_x) = (r, self.w - r);
        let (lo_y, hi_y) = (r, self.h - r);
        p[0] = if lo_x <= hi_x {
            p[0].clamp(lo_x, hi_x)
        } else {
            0.5 * self.w
        };
        p[1] = if lo_y <= hi_y {
            p[1].clamp(lo_y, hi_y)
        } else {
            0.5 * self.h
        };
    }

    fn relax(&self, c: &mut [[f64; 2]], target: f64, sweeps: usize) {
        let mut disp = vec![[0.0f64; 2]; c.len()];
        let mut cnt = vec![0u32; c.len()];
        for _ in 0..sweeps {
            disp.fill([0.0, 0.0]);
            cnt.fill(0);
            let mut moved = false;
            for &(i, j, d) in &self.pairs {
                let need = target * d;
                let cur = dist(c[i], c[j]);
                if cur >= need {
                    continue;
                }
                moved = true;
                let dir = if cur > 1e-12 {
                    [(c[j][0] - c[i][0]) / cur, (c[j][1] - c[i][1]) / cur]
                } else {
                    let a = (i * 7 + j * 13) as f64 * 0.7;
                    [a.cos(), a.sin()]
                };
                let push = (need - cur) * 0.5;
                for (k, sign) in [(i, -1.0), (j, 1.0)] {
                    disp[k][0] += sign * dir[0] * push;
                    disp[k][1] += sign * dir[1] * push;
                    cnt[k] += 1;
                }
            }
            if !moved {
                break;
            }
            for (i, ((p, d), k)) in c.iter_mut().zip(disp.iter()).zip(cnt.iter()).enumerate() {
                let m = f64::from((*k).max(1));
                p[0] += d[0] / m;
                p[1] += d[1] / m;
                self.clamp(i, p, target);
            }
        }
    }

    fn initial(&self, start: usize, rng: &mut StdRng, s: f64) -> Vec<[f64; 2]> {
        let (w, h) = (self.w, self.h);
        let anchors = [
            [0.0, 0.0],
            [w, h],
            [w, 0.0],
            [0.0, h],
            [w * 0.5, 0.0],
            [w * 0.5, h],
            [0.0, h * 0.5],
            [w, h * 0.5],
            [w * 0.5, h * 0.5],
        ];
        let n = self.ids.len();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| self.radii[b].total_cmp(&self.radii[a]));
        if start > 0 {
            for k in (1..n).rev() {
                order.swap(k, rng.random_range(0..=k));
            }
        }
        let anchor_count = match start {
            0 => anchors.len(),
            s if s % 2 == 1 => 4,
            _ => 0,
        };
        let jitter = if start == 0 { 0.0 } else { 0.15 * w.min(h) };
        let mut pos = vec![[0.0; 2]; n];
        for (k, &li) in order.iter().enumerate() {
            let mut p = match anchors.get(k) {
                Some(a) if k < anchor_count && jitter <= 0.0 => *a,
                Some(a) if k < anchor_count => [
                    a[0] + rng.random_range(-jitter..jitter),
                    a[1] + rng.random_range(-jitter..jitter),
                ],
                _ => [rng.random_range(0.0..w), rng.random_range(0.0..h)],
            };
            self.clamp(li, &mut p, s);
            pos[li] = p;
        }
        pos
    }

    fn upper_scale(&self) -> f64 {
        let d_max = self.pairs.iter().map(|p| p.2).fold(0.0_f64, f64::max);
        let by_pair = if d_max > 0.0 {
            self.w.hypot(self.h) / d_max
        } else {
            0.0
        };
        // 円全体が紙に収まるので、半径は紙の短辺の半分を超えられない。
        let r_max = self.radii.iter().fold(0.0_f64, |a, &b| a.max(b));
        let by_paper = if r_max > 0.0 {
            0.5 * self.w.min(self.h) / r_max
        } else {
            by_pair
        };
        by_pair.min(by_paper)
    }

    fn solve_one(&self, start: usize, rng: &mut StdRng) -> (f64, Vec<[f64; 2]>) {
        let s_hi = self.upper_scale();
        let mut best = self.initial(start, rng, s_hi);
        let mut best_s = self.scale_of(&best).min(s_hi);
        for round in 0..=SHAKE_ROUNDS {
            let mut work = best.clone();
            if round > 0 {
                let amp = 0.08 * self.w.min(self.h);
                for (i, p) in work.iter_mut().enumerate() {
                    p[0] += rng.random_range(-amp..amp);
                    p[1] += rng.random_range(-amp..amp);
                    self.clamp(i, p, best_s);
                }
            }
            let (s, c) = self.bisect(work, self.scale_of(&best).min(s_hi), s_hi);
            if s > best_s {
                best_s = s;
                best = c;
            }
        }
        (best_s.max(0.0), best)
    }

    fn bisect(&self, start: Vec<[f64; 2]>, from: f64, s_hi: f64) -> (f64, Vec<[f64; 2]>) {
        let mut best = start;
        let mut best_s = self.scale_of(&best).min(s_hi);
        let (mut lo, mut hi) = (best_s.max(from.min(s_hi)), s_hi);
        for _ in 0..BISECT_STEPS {
            if hi - lo <= 1e-12 * s_hi.max(1.0) {
                break;
            }
            let mid = 0.5 * (lo + hi);
            let mut c = best.clone();
            self.relax(&mut c, mid, RELAX_SWEEPS);
            let s = self.scale_of(&c);
            if s >= mid * (1.0 - 1e-9) {
                lo = mid;
            } else {
                hi = mid;
            }
            if s > best_s {
                best_s = s;
                best = c;
            }
        }
        (best_s.max(0.0), best)
    }
}

/// 案B(円全体を紙内)の充填。`pack` と同じ引数・同じ返し方。
fn pack_full(
    skeleton: &Skeleton,
    paper_w: f64,
    paper_h: f64,
    seed: u64,
    starts: usize,
) -> Vec<Packing> {
    let ok_paper = paper_w > 0.0 && paper_h > 0.0 && paper_w.is_finite() && paper_h.is_finite();
    if !ok_paper || skeleton.validate().is_err() {
        return Vec::new();
    }
    let p = FullProblem::new(skeleton, paper_w, paper_h);
    if p.pairs.is_empty() {
        let r = p.radii.first().copied().unwrap_or(0.0);
        let scale = if r > 0.0 {
            paper_w.min(paper_h) / (2.0 * r)
        } else {
            1.0
        };
        return vec![Packing {
            scale,
            centers: vec![(p.ids[0], [paper_w * 0.5, paper_h * 0.5])],
            violation: 0.0,
            circles: Vec::new(),
        }];
    }
    let mut out: Vec<Packing> = Vec::new();
    for start in 0..starts.clamp(1, 64) {
        let mix = (start as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut rng = StdRng::seed_from_u64(seed ^ mix);
        let (scale, centers) = p.solve_one(start, &mut rng);
        let packing = Packing {
            scale,
            centers: p.ids.iter().copied().zip(centers).collect(),
            violation: 0.0,
            circles: Vec::new(),
        };
        out.push(Packing {
            violation: violation_full(skeleton, &packing, paper_w, paper_h),
            ..packing
        });
    }
    let feasible = |x: &Packing| u8::from(x.violation > PACK_TOL);
    out.sort_by(|a, b| {
        feasible(a)
            .cmp(&feasible(b))
            .then(b.scale.total_cmp(&a.scale))
    });
    out.truncate(MAX_CANDIDATES);
    out
}

// ---------------------------------------------------------------------------
// 1. 折って測った出っぱりの長さと、2つの包含条件の突き合わせ
// ---------------------------------------------------------------------------

/// 鶴の基本形: 4つの出っぱりの中心はすべて紙の隅にあり、円は紙から大きくはみ出す。
/// それでも折り上がりの長さは要求どおりで、案Aの制約はぴったり満たされる。
#[test]
fn bird_base_measured_flaps_exactly_saturate_center_containment() {
    // 折って測った長さ(scratchpadの測定と acceptance_crane.rs の検査が同じ値)。
    let skeleton = fan(&[BIRD_SLENDER, BIRD_SLENDER, BIRD_WIDE, BIRD_WIDE]);
    let packing = Packing {
        scale: 1.0,
        centers: vec![
            (1, [0.0, 0.0]), // 細い先(首)
            (2, [1.0, 1.0]), // 細い先(尾)
            (3, [1.0, 0.0]), // 広いフラップ(羽)
            (4, [0.0, 1.0]), // 広いフラップ(羽)
        ],
        violation: 0.0,
        circles: Vec::new(),
    };

    // 案A: 違反0。しかも6対のうち5対がぴったり(=これ以上大きくできない)。
    assert_eq!(violation_center(&skeleton, &packing, 1.0, 1.0), 0.0);
    let mut tight = 0;
    for (ia, &(id_a, a)) in packing.centers.iter().enumerate() {
        for &(id_b, b) in &packing.centers[ia + 1..] {
            let slack = (a[0] - b[0]).hypot(a[1] - b[1]) - skeleton.leaf_distance(id_a, id_b);
            if slack.abs() <= 1e-12 {
                tight += 1;
            }
        }
    }
    assert_eq!(tight, 5, "6対のうち5対がぴったり");

    // 案B: 同じ配置は成り立たない。羽の円は紙の半分より大きく、置く場所が無い。
    let largest_circle_in_paper = half_short_side(1.0, 1.0);
    assert!(
        BIRD_WIDE > largest_circle_in_paper,
        "羽の円の半径{BIRD_WIDE}は1×1の紙に収まる最大半径{largest_circle_in_paper}を超える"
    );
    let v = violation_full(&skeleton, &packing, 1.0, 1.0);
    assert!(v > 0.2, "案Bでは大きく違反する(実測 {v})");
}

/// カエルの基本形: 5本の出っぱりの長さは全部同じ √2/4。4本の円は紙の隅にあり
/// 3/4がはみ出し、1本は紙の中心にあり全部が紙の中にある。はみ出し量を
/// 0%と75%に変えても、折って測った長さは同じだった(`CLAUDE.md` §10.7.4)。
#[test]
fn frog_base_measured_flaps_are_equal_whether_the_circle_overflows_or_not() {
    let skeleton = star(5, FROG_LIMB);
    let packing = Packing {
        scale: 1.0,
        centers: vec![
            (1, [0.0, 0.0]),
            (2, [1.0, 0.0]),
            (3, [1.0, 1.0]),
            (4, [0.0, 1.0]),
            (5, [0.5, 0.5]),
        ],
        violation: 0.0,
        circles: Vec::new(),
    };
    assert_eq!(violation_center(&skeleton, &packing, 1.0, 1.0), 0.0);

    // 隅の4本は円の3/4が紙の外。中心の1本は全部が紙の中(余裕0.1464)。
    let margin_corner = 0.0_f64;
    let margin_center = 0.5;
    assert!(margin_corner < FROG_LIMB, "隅の円ははみ出す");
    assert!(margin_center > FROG_LIMB, "中心の円ははみ出さない");

    // 案Bで同じ配置を採ると、隅の4本が半径ぶん丸ごと違反する。
    let v = violation_full(&skeleton, &packing, 1.0, 1.0);
    assert!(
        (v - FROG_LIMB).abs() < 1e-12,
        "案Bの違反量は隅の半径そのもの(実測 {v})"
    );

    // 案Bで実際に置ける上限の具体例(4隅を内側へ寄せ、1本を中心に置く)。
    // 中心と隅の距離 √2(0.5-r) が 2r 以上、という条件から r <= √2/2 - 0.5。
    let r_b = 0.5 * SQRT2 - 0.5;
    let inner = Packing {
        scale: r_b / FROG_LIMB,
        centers: vec![
            (1, [r_b, r_b]),
            (2, [1.0 - r_b, r_b]),
            (3, [1.0 - r_b, 1.0 - r_b]),
            (4, [r_b, 1.0 - r_b]),
            (5, [0.5, 0.5]),
        ],
        violation: 0.0,
        circles: Vec::new(),
    };
    assert!(
        violation_full(&skeleton, &inner, 1.0, 1.0) <= 1e-15,
        "案Bで成り立つ具体例のはず"
    );
    // 同じ紙で出っぱりは 2-√2 倍(=41.42%短い)になる。
    let ratio = r_b / FROG_LIMB;
    assert!((ratio - (2.0 - SQRT2)).abs() < 1e-12, "実測 {ratio}");
}

// ---------------------------------------------------------------------------
// 2. 同じ標本での案A / 案Bの縮尺くらべ
// ---------------------------------------------------------------------------

struct Stats {
    min: f64,
    p50: f64,
    p95: f64,
    max: f64,
}

/// nearest-rank(昇順で ceil(p*n) 番目)。作業6の統計と同じ取り方。
fn stats(values: &mut [f64]) -> Stats {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    let at = |p: f64| values[((p * n as f64).ceil() as usize).clamp(1, n) - 1];
    Stats {
        min: values[0],
        p50: at(0.50),
        p95: at(0.95),
        max: values[n - 1],
    }
}

/// 同じ骨格・同じ紙・同じ乱数シード列で、案Aと案Bの最良縮尺を並べて測る。
fn compare(label: &str, skeleton: &Skeleton, seeds: u64) -> (Stats, Stats) {
    let (w, h) = (1.0, 1.0);
    let mut a_values = Vec::with_capacity(seeds as usize);
    let mut b_values = Vec::with_capacity(seeds as usize);
    let mut a_on_border = 0usize;
    let mut b_on_border = 0usize;
    let mut a_centers = 0usize;
    for seed in 0..seeds {
        let a = pack(skeleton, w, h, seed, 8);
        let b = pack_full(skeleton, w, h, seed, 8);
        assert!(!a.is_empty() && !b.is_empty(), "{label}: 候補が出ない");
        assert!(
            violation_center(skeleton, &a[0], w, h) <= PACK_TOL,
            "{label}: 案Aの検算違反"
        );
        assert!(
            violation_full(skeleton, &b[0], w, h) <= PACK_TOL,
            "{label}: 案Bの検算違反 {}",
            violation_full(skeleton, &b[0], w, h)
        );
        for &(_, c) in &a[0].centers {
            a_centers += 1;
            let margin = c[0].min(w - c[0]).min(c[1]).min(h - c[1]);
            if margin <= 1e-9 {
                a_on_border += 1;
            }
        }
        for &(id, c) in &b[0].centers {
            let margin = c[0].min(w - c[0]).min(c[1]).min(h - c[1]);
            if margin <= b[0].scale * skeleton.leaf_radius(id) + 1e-9 {
                b_on_border += 1;
            }
        }
        a_values.push(a[0].scale);
        b_values.push(b[0].scale);
    }
    let (sa, sb) = (stats(&mut a_values), stats(&mut b_values));
    println!(
        "[{label}] 標本{seeds}件 (starts=8, 紙1×1)\n  案A(中心包含)  min={:.9} p50={:.9} p95={:.9} max={:.9}\n  案B(円全体)    min={:.9} p50={:.9} p95={:.9} max={:.9}\n  中央値の比 案B/案A = {:.6}  紙の縁に接する中心 案A {a_on_border}/{a_centers}・案B(円が縁に接する) {b_on_border}/{a_centers}",
        sa.min, sa.p50, sa.p95, sa.max, sb.min, sb.p50, sb.p95, sb.max, sb.p50 / sa.p50,
    );
    (sa, sb)
}

/// 素早く回る標本での確認。案Bが案Aを上回らないこと(案Bは案Aの制約を強めた形)。
#[test]
fn full_circle_containment_never_beats_center_containment_on_twenty_seeds() {
    let skeleton = star(12, 1.0);
    for seed in 0..20 {
        let a = pack(&skeleton, 1.0, 1.0, seed, 8);
        let b = pack_full(&skeleton, 1.0, 1.0, seed, 8);
        assert!(!a.is_empty() && !b.is_empty());
        assert!(a[0].scale.is_finite() && b[0].scale.is_finite());
        assert!(violation_center(&skeleton, &a[0], 1.0, 1.0) <= PACK_TOL);
        assert!(violation_full(&skeleton, &b[0], 1.0, 1.0) <= PACK_TOL);
        assert!(
            b[0].scale <= a[0].scale + PACK_TOL,
            "seed {seed}: 案B {} が案A {} を上回った",
            b[0].scale,
            a[0].scale
        );
    }
}

/// 案Bの充填が、円全体を紙に収める配置を本当に返していること。
#[test]
fn full_circle_packing_keeps_every_circle_inside_the_paper() {
    for n in 1..=12u32 {
        let skeleton = star(n, 1.0);
        let out = pack_full(&skeleton, 1.0, 1.0, u64::from(n), 8);
        assert!(!out.is_empty(), "葉{n}本で候補が出ない");
        for p in &out {
            assert!(p.scale.is_finite() && p.scale > 0.0, "葉{n}本: {p:?}");
            for &(id, c) in &p.centers {
                let r = p.scale * skeleton.leaf_radius(id);
                let margin = c[0].min(1.0 - c[0]).min(c[1]).min(1.0 - c[1]);
                assert!(
                    margin >= r - PACK_TOL,
                    "葉{n}本・葉{id}の円がはみ出す: margin={margin}, r={r}"
                );
            }
        }
    }
}

/// 作業6と同じ標本(seed 0..999、starts 8、等長12本、紙1×1)での本測定。
///
/// 案Aと案Bで各1,000回、合計2,000回の充填を回すため5分前後かかる。
/// 毎回の `cargo test` を遅くしないよう `#[ignore]` にしてある。
/// 実行: `cargo test -p ori3-propose --test containment -- --ignored --nocapture`
#[test]
#[ignore = "案A・案Bを各1,000回ずつ充填するため5分前後かかる。判断材料の本測定用"]
fn measure_center_and_full_containment_on_the_same_thousand_seeds() {
    let (a12, b12) = compare("等長12本", &star(12, 1.0), 1000);
    assert!(b12.max <= a12.max + PACK_TOL);

    // 折って測った2作品と同じ骨格。
    compare("カエルの基本形の骨格(等長5本)", &star(5, 1.0), 200);
    compare(
        "鶴の基本形の骨格(細2本・広2本)",
        &fan(&[BIRD_SLENDER, BIRD_SLENDER, BIRD_WIDE, BIRD_WIDE]),
        200,
    );
    // 要件§8の精度目標に使われている骨格(頭1・尾1・足4)。
    let mut nodes = vec![
        SkeletonNode::new(0, None, 0.0),
        SkeletonNode::new(1, Some(0), 0.3),
        SkeletonNode::new(2, Some(0), 1.0),
        SkeletonNode::new(3, Some(1), 1.0),
    ];
    for i in 0..4u32 {
        nodes.push(SkeletonNode::new(4 + i, Some(if i < 2 { 0 } else { 1 }), 0.6));
    }
    compare("頭1・尾1・足4", &Skeleton { nodes }, 200);
}
