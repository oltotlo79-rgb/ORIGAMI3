//! 骨格モデル(PRO-001)。頭・尾・足などの「角」を根付き木で表す。
//!
//! 要件§8-1: 葉が角、内部辺が胴。各辺に長さ、各葉に太さ係数を持つ。

use serde::{Deserialize, Serialize};

/// 角(葉)の本数の上限。要件PRO-001。
pub const MAX_LEAVES: usize = 12;

/// 完成形の先端位置として指定できる各成分の下限。
pub const TIP_POS_MIN: f64 = -1.0;
/// 完成形の先端位置として指定できる各成分の上限。
pub const TIP_POS_MAX: f64 = 1.0;

/// 完成形における先端の位置(要件PRO-006 / PRO-007)。
///
/// 完成した作品を正面から見たときの**2D投影上の相対位置**で、奥行きは持たない。
///
/// - 原点 `(0.0, 0.0)` は胴の中心(根の節点)。
/// - `x` は右向きが正、`y` は上向きが正。
/// - 範囲は `TIP_POS_MIN`(-1.0) 以上 `TIP_POS_MAX`(1.0) 以下。
///   `1.0` は完成形を囲む正方形の中心から辺までの長さ(半辺)にあたる。
/// - 指すのは**完成した作品での頭・尾・足などの先端の位置**であり、
///   展開前の紙の上で材料を割り当てる位置ではない(PRO-007)。
///
/// 将来3D指定を足す場合は `tip_pos_3d` のような別の欄として追加する。
/// この型が読めるのは `{"x": 横, "y": 縦}` の形だけで、欄が多くても少なくても、
/// 値を並べて書いても読み込みエラーになる。したがって奥行き `z` を同じ欄へ
/// 紛れ込ませることはできず、既に保存された2Dの指定が3Dとして読み替えられることもない
/// (PRO-006)。
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct TipPos2d {
    pub x: f64,
    pub y: f64,
}

impl<'de> Deserialize<'de> for TipPos2d {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct XyOnly;

        impl<'de> serde::de::Visitor<'de> for XyOnly {
            type Value = TipPos2d;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("横xと縦yの2つの欄だけを持つ完成形の先端位置")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<TipPos2d, A::Error> {
                use serde::de::Error as _;
                let mut x: Option<f64> = None;
                let mut y: Option<f64> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "x" => {
                            if x.is_some() {
                                return Err(A::Error::duplicate_field("x"));
                            }
                            x = Some(map.next_value()?);
                        }
                        "y" => {
                            if y.is_some() {
                                return Err(A::Error::duplicate_field("y"));
                            }
                            y = Some(map.next_value()?);
                        }
                        other => return Err(A::Error::unknown_field(other, &["x", "y"])),
                    }
                }
                Ok(TipPos2d {
                    x: x.ok_or_else(|| A::Error::missing_field("x"))?,
                    y: y.ok_or_else(|| A::Error::missing_field("y"))?,
                })
            }
        }

        deserializer.deserialize_map(XyOnly)
    }
}

impl TipPos2d {
    /// 先端位置を作る。範囲の検査はしない(検査は `Skeleton::validate`)。
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// 有限で、かつ `-1.0`〜`1.0` の範囲に収まっているか。
    pub fn is_valid(&self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && (TIP_POS_MIN..=TIP_POS_MAX).contains(&self.x)
            && (TIP_POS_MIN..=TIP_POS_MAX).contains(&self.y)
    }

    /// 範囲内へ収めた値を返す。画面から先端を引っぱって動かすとき用で、
    /// データを読み込むときには使わない(読み込みは範囲外をエラーにする)。
    /// 数値にならない値(NaN)は原点の成分 `0.0` として扱う。
    pub fn clamped(self) -> Self {
        let fit = |v: f64| {
            if v.is_nan() {
                0.0
            } else {
                v.clamp(TIP_POS_MIN, TIP_POS_MAX)
            }
        };
        Self {
            x: fit(self.x),
            y: fit(self.y),
        }
    }
}

/// 骨格の節点。
///
/// - `parent` が `None` の節点が根(胴の中心)。根はちょうど1つ。
/// - `length` は「親へつながる辺の長さ」。根では使わない。
/// - `width_factor` は角の太さ(膨らみ)係数。葉の円半径は `length * width_factor`。
/// - `tip_pos_2d` は完成形における先端の位置。省略できる。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkeletonNode {
    pub id: u32,
    pub parent: Option<u32>,
    pub length: f64,
    pub width_factor: f64,
    /// 完成形における先端の位置(PRO-006 / PRO-007)。
    ///
    /// `None` は「位置を指定しない」で、置き場所は提案の計算が決める。
    /// JSONでは欄ごと省略されるため、位置を指定しない骨格の書き出しは
    /// この欄が無かったころと1文字も変わらない。
    /// 意味を持つのは葉(角)の欄だけで、`Skeleton::leaf_tip_positions` が取り出す。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip_pos_2d: Option<TipPos2d>,
}

impl SkeletonNode {
    /// 親へつながる辺を持つ節点を作る(太さ係数は既定の1.0、位置は未指定)。
    pub fn new(id: u32, parent: Option<u32>, length: f64) -> Self {
        Self {
            id,
            parent,
            length,
            width_factor: 1.0,
            tip_pos_2d: None,
        }
    }

    /// 完成形における先端の位置を付けた節点を返す。
    pub fn with_tip_pos(mut self, pos: TipPos2d) -> Self {
        self.tip_pos_2d = Some(pos);
        self
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

    /// 完成形の先端位置が指定されている葉の一覧。葉IDの昇順で、1つの葉IDは高々1回。
    ///
    /// 位置を指定していない葉は入らない。葉でない節点に位置が残っていても入らない
    /// (先へ枝を足して葉でなくなった場合に、古い指定を使わないため)。
    pub fn leaf_tip_positions(&self) -> Vec<(u32, TipPos2d)> {
        let mut out: Vec<(u32, TipPos2d)> = self
            .leaves()
            .into_iter()
            .filter_map(|id| self.node(id).and_then(|n| n.tip_pos_2d).map(|p| (id, p)))
            .collect();
        out.sort_by_key(|(id, _)| *id);
        out
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
            if let Some(p) = n.tip_pos_2d
                && !p.is_valid()
            {
                return Err(format!(
                    "節点{}の位置は横も縦も{TIP_POS_MIN}以上{TIP_POS_MAX}以下にしてください(現在 横{}, 縦{})",
                    n.id, p.x, p.y
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
