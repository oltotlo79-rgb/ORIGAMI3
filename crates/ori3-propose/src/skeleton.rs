//! 骨格モデル(PRO-001)。頭・尾・足などの「角」を根付き木で表す。
//!
//! 要件§8-1: 葉が角、内部辺が胴。各辺に長さ、各葉に太さ係数を持つ。

use serde::{Deserialize, Serialize};

/// 角(葉)の本数の上限。要件PRO-001。
pub const MAX_LEAVES: usize = 12;

/// 骨格の節点。
///
/// - `parent` が `None` の節点が根(胴の中心)。根はちょうど1つ。
/// - `length` は「親へつながる辺の長さ」。根では使わない。
/// - `width_factor` は角の太さ(膨らみ)係数。葉の円半径は `length * width_factor`。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkeletonNode {
    pub id: u32,
    pub parent: Option<u32>,
    pub length: f64,
    pub width_factor: f64,
}

impl SkeletonNode {
    /// 親へつながる辺を持つ節点を作る(太さ係数は既定の1.0)。
    pub fn new(id: u32, parent: Option<u32>, length: f64) -> Self {
        Self {
            id,
            parent,
            length,
            width_factor: 1.0,
        }
    }
}

/// 骨格全体。節点の並び順は自由(根が先頭でなくてもよい)。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Skeleton {
    pub nodes: Vec<SkeletonNode>,
}

impl Skeleton {
    /// IDから節点を引く。
    pub fn node(&self, id: u32) -> Option<&SkeletonNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// 根(親を持たない節点)のID。複数あるときは最初の1つ。
    pub fn root(&self) -> Option<u32> {
        self.nodes.iter().find(|n| n.parent.is_none()).map(|n| n.id)
    }

    /// 角(葉)のID一覧。子を持たず、かつ親へつながる辺を持つ節点。
    ///
    /// 節点が根1つだけの骨格には辺がないので、葉も0本になる。
    pub fn leaves(&self) -> Vec<u32> {
        self.nodes
            .iter()
            .filter(|n| n.parent.is_some() && !self.nodes.iter().any(|c| c.parent == Some(n.id)))
            .map(|n| n.id)
            .collect()
    }

    /// 葉に対応する円の半径(角の長さ × 太さ係数)。
    pub fn leaf_radius(&self, id: u32) -> f64 {
        self.node(id)
            .map(|n| n.length * n.width_factor)
            .unwrap_or(0.0)
    }

    /// 節点から根までの祖先列(自分自身を含み、根で終わる)。
    /// 循環している場合は打ち切って返す。
    fn ancestors(&self, id: u32) -> Vec<u32> {
        let mut path = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            if path.contains(&c) {
                break; // 循環保護
            }
            path.push(c);
            cur = self.node(c).and_then(|n| n.parent);
        }
        path
    }

    /// 2つの葉の間に必要な距離(縮尺1のとき)。
    /// = 円半径a + 木の上の経路にある川幅(内部辺の長さ)の合計 + 円半径b。
    ///
    /// 同じ葉、または木としてつながっていない場合は0を返す。
    pub fn leaf_distance(&self, a: u32, b: u32) -> f64 {
        if a == b {
            return 0.0;
        }
        let up_a = self.ancestors(a);
        let up_b = self.ancestors(b);
        let Some(lca) = up_a.iter().find(|x| up_b.contains(x)).copied() else {
            return 0.0;
        };
        // 経路上の各節点の`length`を足す(共通祖先そのものは足さない)。
        let sum = |path: &[u32]| -> f64 {
            path.iter()
                .take_while(|x| **x != lca)
                .filter_map(|x| self.node(*x))
                .map(|n| n.length)
                .sum()
        };
        // 端の2本は「辺の長さ」ではなく太さを含めた円半径で数える。
        sum(&up_a) + sum(&up_b) - self.len_of(a) - self.len_of(b)
            + self.leaf_radius(a)
            + self.leaf_radius(b)
    }

    fn len_of(&self, id: u32) -> f64 {
        self.node(id).map(|n| n.length).unwrap_or(0.0)
    }

    /// 骨格が木として正しいかを調べる。エラーは日本語1文で返す。
    pub fn validate(&self) -> Result<(), String> {
        if self.nodes.is_empty() {
            return Err("骨格に節点がありません".to_string());
        }
        for (i, n) in self.nodes.iter().enumerate() {
            if self.nodes[..i].iter().any(|m| m.id == n.id) {
                return Err(format!("節点IDが重複しています(ID {})", n.id));
            }
        }
        for n in &self.nodes {
            if let Some(p) = n.parent
                && self.node(p).is_none()
            {
                return Err(format!("節点{}の親(ID {p})が見つかりません", n.id));
            }
        }
        let roots = self.nodes.iter().filter(|n| n.parent.is_none()).count();
        if roots != 1 {
            return Err(format!(
                "根(親を持たない節点)はちょうど1つにしてください(現在{roots}個)"
            ));
        }
        let root = self.root().unwrap_or_default();
        for n in &self.nodes {
            if *self.ancestors(n.id).last().unwrap_or(&n.id) != root {
                return Err(format!(
                    "骨格に循環があります(節点{}が根までたどれません)",
                    n.id
                ));
            }
        }
        for n in &self.nodes {
            if n.parent.is_some() && !(n.length > 0.0 && n.length.is_finite()) {
                return Err(format!(
                    "節点{}の長さは0より大きい値にしてください(現在{})",
                    n.id, n.length
                ));
            }
            if !(n.width_factor > 0.0 && n.width_factor.is_finite()) {
                return Err(format!(
                    "節点{}の太さは0より大きい値にしてください(現在{})",
                    n.id, n.width_factor
                ));
            }
        }
        let leaves = self.leaves().len();
        if leaves == 0 {
            return Err("角(頭・尾・足など)が1本もありません".to_string());
        }
        if leaves > MAX_LEAVES {
            return Err(format!("角は{MAX_LEAVES}本までです(現在{leaves}本)"));
        }
        Ok(())
    }
}
