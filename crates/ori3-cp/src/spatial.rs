//! 一様格子(空間ハッシュ)による近傍探索。
//!
//! 全ペア総当たり(O(E²))を避けるための道具。線分・点をセルへ登録し、
//! 同じセルに載ったものだけを候補として返す。
//!
//! 設計上の約束:
//! - **決定的**: セルは`BTreeMap`で保持し、問い合わせ結果は必ず添字の昇順に
//!   そろえて返す。よって呼び出し側は「総当たりの二重ループ」と同じ順序で
//!   候補を処理でき、警告の内容・順序が変わらない。
//! - **取りこぼさない**: 登録も問い合わせもEPS分だけ広げた範囲で行う。
//!   セル境界をまたいでEPS以内にある相手は必ず同じセルに載る。

use std::collections::BTreeMap;

use glam::{DVec2, Vec2Swizzles};
use ori3_model::EPS;

/// 格子セルの座標。
type Cell = (i64, i64);

/// 一様格子。値はスライスへの添字(登録順=呼び出し側の走査順)。
pub struct Grid {
    h: f64,
    cells: BTreeMap<Cell, Vec<u32>>,
}

/// 座標をセル番号に落とす(`as i64`は飽和変換なので極端な値でも壊れない)。
fn cell_of(v: f64, h: f64) -> i64 {
    (v / h).floor() as i64
}

/// 線分群からセルの大きさを決める。
///
/// 外接矩形を√n分割した幅と、線分の平均的な広がりの大きい方を採る。
/// 前者だけだと長い線分が多数のセルに跨り、後者だけだと短い線分が同一セルへ
/// 集中し得るため、両者の大きい方で釣り合いを取る。
/// 全点が同一位置など格子が作れない場合はNone(呼び出し側は総当たりに戻す)。
fn choose_cell_size(segs: &[(DVec2, DVec2)]) -> Option<f64> {
    if segs.is_empty() {
        return None;
    }
    let mut lo = DVec2::splat(f64::INFINITY);
    let mut hi = DVec2::splat(f64::NEG_INFINITY);
    let mut span_sum = 0.0;
    for &(a, b) in segs {
        lo = lo.min(a).min(b);
        hi = hi.max(a).max(b);
        span_sum += (b - a).abs().max_element();
    }
    let extent = (hi - lo).max_element();
    if !extent.is_finite() || extent <= 0.0 {
        return None;
    }
    let n = segs.len() as f64;
    let h = (extent / n.sqrt()).max(span_sum / n);
    // セル数が発散しないよう下限を置く(1辺あたり最大10万分割)。
    Some(h.max(extent / 1e5).max(EPS))
}

impl Grid {
    /// 線分群(または`(p, p)`とした点群)を登録した格子を作る。
    /// 格子が作れない入力ではNoneを返す。
    pub fn build(segs: &[(DVec2, DVec2)]) -> Option<Self> {
        let h = choose_cell_size(segs)?;
        let mut g = Grid {
            h,
            cells: BTreeMap::new(),
        };
        for (i, &(a, b)) in segs.iter().enumerate() {
            g.insert(i as u32, a, b);
        }
        Some(g)
    }

    /// 線分(点なら`a == b`)を、EPS分広げた範囲が触れる全セルへ登録する。
    pub fn insert(&mut self, idx: u32, a: DVec2, b: DVec2) {
        let h = self.h;
        let cells = &mut self.cells;
        for_each_cell(a, b, h, |c| {
            let v = cells.entry(c).or_default();
            if v.last() != Some(&idx) {
                v.push(idx);
            }
        });
    }

    /// 線分の近傍にある候補の添字を、昇順・重複なしで`out`へ書き出す。
    pub fn near_into(&self, a: DVec2, b: DVec2, out: &mut Vec<u32>) {
        out.clear();
        let cells = &self.cells;
        for_each_cell(a, b, self.h, |c| {
            if let Some(v) = cells.get(&c) {
                out.extend_from_slice(v);
            }
        });
        out.sort_unstable();
        out.dedup();
    }

    /// 点の近傍にある候補の添字(昇順・重複なし)。
    pub fn near_point(&self, p: DVec2) -> Vec<u32> {
        let mut out = Vec::new();
        self.near_into(p, p, &mut out);
        out
    }
}

/// 線分をEPSだけ太らせた帯が触れるセルを列挙する。
///
/// 外接矩形を丸ごと舐めると長い斜め線でセル数が二乗に膨らむため、広がりの
/// 大きい軸に沿って走査し、各列(行)で線分が実際に取る範囲だけを拾う。
/// 帯の内側の点はどれも必ずいずれかのセルに含まれる(線分は各列内で単調)。
fn for_each_cell(a: DVec2, b: DVec2, h: f64, mut f: impl FnMut(Cell)) {
    let lo = a.min(b) - DVec2::splat(EPS);
    let hi = a.max(b) + DVec2::splat(EPS);
    let flip = (hi.x - lo.x) < (hi.y - lo.y);
    // 走査軸をxに寄せる(必要なら座標を入れ替えて最後に戻す)。
    let (a, b, lo, hi) = if flip {
        (a.yx(), b.yx(), lo.yx(), hi.yx())
    } else {
        (a, b, lo, hi)
    };
    let (u0, u1) = (a.x.min(b.x), a.x.max(b.x));
    let du = b.x - a.x;
    for cu in cell_of(lo.x, h)..=cell_of(hi.x, h) {
        let s0 = (cu as f64 * h - EPS).max(u0);
        let s1 = ((cu + 1) as f64 * h + EPS).min(u1);
        if s0 > s1 {
            continue; // 走査範囲の丸めで空になった列(通常は起きない)
        }
        let (v0, v1) = if du != 0.0 {
            let at = |x: f64| a.y + (x - a.x) * (b.y - a.y) / du;
            let (p, q) = (at(s0), at(s1));
            (p.min(q), p.max(q))
        } else {
            (a.y.min(b.y), a.y.max(b.y))
        };
        for cv in cell_of(v0 - EPS, h)..=cell_of(v1 + EPS, h) {
            f(if flip { (cv, cu) } else { (cu, cv) });
        }
    }
}
