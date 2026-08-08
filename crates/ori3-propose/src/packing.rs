//! 円・川充填の数値最適化(PRO-002 / 要件§8-2)。
//!
//! 変数は各葉(角)の円中心と縮尺 `s`。目的は `s` の最大化、制約は
//! 「葉iとjの円中心の距離 ≥ s ×(円半径i + 経路上の川幅 + 円半径j)」と
//! 「円中心が紙の内側」。円そのものは紙からはみ出してよい(紙の角・辺に
//! 置いた角がそうなる)。
//!
//! 解き方は射影法(制約違反を押し戻す掃引)+ 段階的な拡大。乱数シードを
//! 変えたマルチスタートで局所解を避け、上位候補を返す(PRO-005)。

use crate::skeleton::Skeleton;
use rand::{RngExt, SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};

/// 返せる候補の最大数(PRO-005)。
pub const MAX_CANDIDATES: usize = 4;

/// 「制約を満たしている」とみなす違反量の上限。
pub const PACK_TOL: f64 = 1e-9;

/// 縮尺の二分探索の刻み数、1刻みあたりの押し離し掃引数、揺さぶり直しの回数。
/// 12葉・8スタートで数百ms以内に収まり、かつ質が頭打ちになる値に合わせた。
const BISECT_STEPS: usize = 28;
const RELAX_SWEEPS: usize = 48;
const SHAKE_ROUNDS: usize = 3;

/// 充填の結果。`violation` は制約違反の最大量で、0に近いほど良い。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Packing {
    pub scale: f64,
    pub centers: Vec<(u32, [f64; 2])>,
    pub violation: f64,
}

/// 最適化しやすい形にほぐした問題。
struct Problem {
    ids: Vec<u32>,
    radii: Vec<f64>,
    /// (葉の添字i, 葉の添字j, 必要距離d)。実距離が `s*d` 以上であればよい。
    pairs: Vec<(usize, usize, f64)>,
    w: f64,
    h: f64,
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

impl Problem {
    fn new(skeleton: &Skeleton, w: f64, h: f64) -> Self {
        let ids = skeleton.leaves();
        let radii = ids.iter().map(|&id| skeleton.leaf_radius(id)).collect();
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

    /// この配置で成り立つ最大の縮尺。制約が1つもなければ無限大。
    fn scale_of(&self, c: &[[f64; 2]]) -> f64 {
        let mut s = f64::INFINITY;
        for &(i, j, d) in &self.pairs {
            if d > 0.0 {
                s = s.min(dist(c[i], c[j]) / d);
            }
        }
        s
    }

    /// 指定縮尺での制約違反の最大量(円の重なり / 紙のはみ出し)。
    fn violation_of(&self, s: f64, c: &[[f64; 2]]) -> f64 {
        let mut v: f64 = 0.0;
        for &(i, j, d) in &self.pairs {
            v = v.max(s * d - dist(c[i], c[j]));
        }
        for p in c {
            v = v
                .max(-p[0])
                .max(p[0] - self.w)
                .max(-p[1])
                .max(p[1] - self.h);
        }
        v.max(0.0)
    }

    fn clamp(&self, p: &mut [f64; 2]) {
        p[0] = p[0].clamp(0.0, self.w);
        p[1] = p[1].clamp(0.0, self.h);
    }

    /// 目標縮尺を満たすよう、近すぎる対を押し離す掃引を繰り返す(射影)。
    /// 動かした量は関わった制約の数で割って平均化し、振動を抑える。
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
                // 中心がぴったり重なったときは添字で決まる向きへ逃がす(決定的)。
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
            for ((p, d), k) in c.iter_mut().zip(disp.iter()).zip(cnt.iter()) {
                let m = f64::from((*k).max(1));
                p[0] += d[0] / m;
                p[1] += d[1] / m;
                self.clamp(p);
            }
        }
    }

    /// 初期配置。紙の角・辺・中心という有利な場所へ大きい円から貪欲に置き、
    /// シードごとに並びと揺らぎを変える(要件§8-2)。
    fn initial(&self, start: usize, rng: &mut StdRng) -> Vec<[f64; 2]> {
        let (w, h) = (self.w, self.h);
        // 角は対角どうしを先に使う(2本の角が同じ辺に並んで動けなくなるのを防ぐ)。
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
        // 偶数番のシードは角・辺を使わず完全な乱数配置にして候補を散らす。
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
            self.clamp(&mut p);
            pos[li] = p;
        }
        pos
    }

    /// 1スタート分の最適化。二分探索で縮尺を上げ、行き詰まったら少し揺さぶって
    /// やり直す(格子状の行き止まりから抜けるため)。
    /// 返す縮尺はその配置が実際に満たす値なので、制約違反は生じない。
    fn solve_one(&self, start: usize, rng: &mut StdRng) -> (f64, Vec<[f64; 2]>) {
        // 紙の対角線より遠くへは離せないので、これが縮尺の上界になる。
        let d_max = self.pairs.iter().map(|p| p.2).fold(0.0_f64, f64::max);
        let s_hi = if d_max > 0.0 {
            self.w.hypot(self.h) / d_max
        } else {
            0.0
        };
        let mut best = self.initial(start, rng);
        let mut best_s = self.scale_of(&best).min(s_hi);
        for round in 0..=SHAKE_ROUNDS {
            let mut work = best.clone();
            if round > 0 {
                let amp = 0.08 * self.w.min(self.h);
                for p in work.iter_mut() {
                    p[0] += rng.random_range(-amp..amp);
                    p[1] += rng.random_range(-amp..amp);
                    self.clamp(p);
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

    /// 与えた配置を出発点に、達成できる縮尺を二分探索で詰める。
    fn bisect(&self, start: Vec<[f64; 2]>, from: f64, s_hi: f64) -> (f64, Vec<[f64; 2]>) {
        let mut best = start;
        let mut best_s = self.scale_of(&best).min(s_hi);
        // 出発点が既に満たしている縮尺より下は探さない。
        let (mut lo, mut hi) = (best_s.max(from.min(s_hi)), s_hi);
        for _ in 0..BISECT_STEPS {
            if hi - lo <= 1e-12 * s_hi.max(1.0) {
                break;
            }
            let mid = 0.5 * (lo + hi);
            let mut c = best.clone();
            self.relax(&mut c, mid, RELAX_SWEEPS);
            let s = self.scale_of(&c);
            // 目標に届いたら下限を上げ、届かなければ上限を下げる。
            if s >= mid * (1.0 - 1e-9) {
                lo = mid;
            } else {
                hi = mid;
            }
            // 届かなかった配置でも、これまでより広ければ採用する。
            if s > best_s {
                best_s = s;
                best = c;
            }
        }
        (best_s.max(0.0), best)
    }
}

/// 骨格を紙の上に円・川充填する(PRO-002)。
///
/// `seed` と `starts` が同じなら結果も同じ(決定的)。スコア順に最大
/// [`MAX_CANDIDATES`] 件を返す。骨格や紙寸法が不正なときは空を返す。
pub fn pack(
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
    let p = Problem::new(skeleton, paper_w, paper_h);
    // 角が1本だけなら対の制約がない。円の直径が紙の短辺に収まる縮尺を返す。
    if p.pairs.is_empty() {
        let r = p.radii.first().copied().unwrap_or(0.0);
        let scale = if r > 0.0 {
            paper_w.min(paper_h) / (2.0 * r)
        } else {
            1.0
        };
        let center = (p.ids[0], [paper_w * 0.5, paper_h * 0.5]);
        return vec![Packing {
            scale,
            centers: vec![center],
            violation: 0.0,
        }];
    }
    let mut out: Vec<Packing> = Vec::new();
    for start in 0..starts.clamp(1, 64) {
        let mix = (start as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut rng = StdRng::seed_from_u64(seed ^ mix);
        let (scale, centers) = p.solve_one(start, &mut rng);
        out.push(Packing {
            violation: p.violation_of(scale, &centers),
            centers: p.ids.iter().copied().zip(centers).collect(),
            scale,
        });
    }
    // 制約を満たすものを先に、その中では縮尺の大きい順に並べる。
    let feasible = |x: &Packing| u8::from(x.violation > PACK_TOL);
    out.sort_by(|a, b| {
        feasible(a)
            .cmp(&feasible(b))
            .then(b.scale.total_cmp(&a.scale))
    });
    out.truncate(MAX_CANDIDATES);
    out
}
