//! 近傍探索用の一様格子(開番地法のハッシュ表)。
//!
//! 層の貫通防止で「近くにいる別の層の頂点」を探すのに使う。頂点数6,000規模で
//! 1点あたり27セルを引くため、整列配列の二分探索(1回あたり十数回のランダム
//! アクセス)では性能目標(NFR-002b)に届かない。セル鍵を64ビットへ詰めて
//! 線形探査のハッシュ表で引くと、ほぼ1回のアクセスで済む。
//!
//! 走査順は「セル鍵→頂点番号」の昇順に固定されるので結果は決定的。

use glam::DVec3;

/// 空き印(このセル鍵は使われない値)。
const EMPTY: u64 = u64::MAX;
/// 座標をセル番号にしたときの下駄(負の番号を非負へ寄せる)。
const OFFSET: i64 = 1 << 20;

/// セル番号(x,y,z)を64ビットの鍵へ詰める。範囲外は端へ丸める(探索の取りこぼしは
/// 貫通防止が働かないだけで、形が壊れることはない)。
fn pack(c: [i64; 3]) -> u64 {
    let mut k = 0u64;
    for &v in &c {
        k = (k << 21) | ((v + OFFSET).clamp(0, (1 << 21) - 1) as u64);
    }
    k
}

/// セル鍵で引ける一様格子。
pub(crate) struct CellGrid {
    /// セル鍵の昇順(同じ鍵の中では頂点番号の昇順)に並べた頂点番号
    items: Vec<u32>,
    /// ハッシュ表: (セル鍵, `items`の開始位置, 個数)
    table: Vec<(u64, u32, u32)>,
    mask: u64,
    cell: f64,
}

impl CellGrid {
    /// 一辺`cell`の格子へ全頂点を入れる。
    pub fn new(pos: &[DVec3], cell: f64) -> CellGrid {
        let mut keyed: Vec<(u64, u32)> = pos
            .iter()
            .enumerate()
            .map(|(i, p)| (pack(cell_of(*p, cell)), i as u32))
            .collect();
        keyed.sort_unstable();
        let mut cap = 16usize;
        while cap < keyed.len() * 2 {
            cap *= 2;
        }
        let mask = (cap - 1) as u64;
        let mut table = vec![(EMPTY, 0u32, 0u32); cap];
        let mut items = Vec::with_capacity(keyed.len());
        let mut i = 0;
        while i < keyed.len() {
            let key = keyed[i].0;
            let start = items.len() as u32;
            while i < keyed.len() && keyed[i].0 == key {
                items.push(keyed[i].1);
                i += 1;
            }
            let mut slot = (hash(key) & mask) as usize;
            while table[slot].0 != EMPTY {
                slot = (slot + 1) & mask as usize;
            }
            table[slot] = (key, start, items.len() as u32 - start);
        }
        CellGrid {
            items,
            table,
            mask,
            cell,
        }
    }

    /// 点`p`が入るセルの番号。
    pub fn cell_of(&self, p: DVec3) -> [i64; 3] {
        cell_of(p, self.cell)
    }

    /// セル番号`c`にいる頂点番号を返す(頂点番号の昇順)。
    pub fn cell(&self, c: [i64; 3]) -> &[u32] {
        let key = pack(c);
        let mut slot = (hash(key) & self.mask) as usize;
        loop {
            let (k, start, len) = self.table[slot];
            if k == key {
                return &self.items[start as usize..(start + len) as usize];
            }
            if k == EMPTY {
                return &[];
            }
            slot = (slot + 1) & self.mask as usize;
        }
    }
}

fn cell_of(p: DVec3, cell: f64) -> [i64; 3] {
    [
        (p.x / cell).floor() as i64,
        (p.y / cell).floor() as i64,
        (p.z / cell).floor() as i64,
    ]
}

fn hash(key: u64) -> u64 {
    key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 29
}
