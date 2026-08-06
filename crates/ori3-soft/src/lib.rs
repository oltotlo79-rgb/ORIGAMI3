//! ori3-soft: 紙のたわみ表現(SIM-012〜015・要件§7.1c)。
//!
//! 剛体折り(`ori3-rigid`)が求めた「平らな板の集まり」の姿勢を**基準の形**として
//! 受け取り、面を細かい三角形へ分けて頂点を動かし、折り目以外の場所でも紙が
//! 滑らかに曲がった見た目を近似する後処理層。**層順序は既存の層モデルの値
//! (`Face3D::layer`)を拘束として使うだけ**で、折り操作・手順記録・折り図出力へは
//! 一切影響しない。
//!
//! # 範囲(§4.2 非目標)
//!
//! 物理的に正確な材質・重力・摩擦・皺の再現はしない。**見た目の近似**に留める。
//! たわみの状態は[`SoftSettings`]のパラメータとしてのみ扱い、頂点の位置そのものは
//! 保存しない(SIM-015)。

mod grid;
mod solve;
mod subdivide;

use glam::DVec3;
use ori3_cp::Face;
use ori3_model::{CreasePattern, FaceId, Frame3D};

/// 細分の上限(1辺 2^4 = 16等分)。これを超える指定は丸めて警告する。
const MAX_SUBDIVISION: u32 = 4;
/// 反復回数の上限。これを超える指定は丸めて警告する。
const MAX_ITERATIONS: u32 = 200;
/// 網の三角形数の上限。超える見込みなら細分を自動で落とす(NFR-002b の
/// 「大きな展開図では分割の細かさを自動で落として目標を保つ」)。
///
/// 実測(2026-08-06・開発機 Windows 11・release・反復20回・層16枚)では
/// 1フレームおよそ「三角形1,000枚あたり1.6ms」で、三角形12,800枚だと約20msと
/// 目標の16msを超える。8,000枚なら約12msに収まるのでこの値にしている。
const MAX_TRIANGLES: usize = 8_000;

/// たわみの設定。SIM-015 のとおり、たわみの状態はこの値だけで表す。
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SoftSettings {
    /// たわみ計算を行うか。**既定は false**(オフのときは剛体折りの多角形を
    /// そのまま三角形にしただけの網を返し、頂点は1ビットも動かさない)。
    pub enabled: bool,
    /// 面の分割の細かさ。0=分割しない、nなら各三角形の1辺を 2^n 等分する。
    /// 0〜[`MAX_SUBDIVISION`](=4)。既定2。大きな展開図では自動で落とす。
    pub subdivision: u32,
    /// 紙の硬さ。0.0〜1.0で、大きいほど面の中が平らに保たれる(既定0.5)。
    /// 折り目(面をまたぐ辺)の角度拘束はこの値に関係なく常に最強。
    pub stiffness: f64,
    /// 膨らみの強さ(空気圧)。0.0〜1.0で、0.0なら膨らませない(既定0.0)。
    pub pressure: f64,
    /// 反復回数。決定性のため固定回数だけ回す。1〜[`MAX_ITERATIONS`]。既定20。
    pub iterations: u32,
}

impl Default for SoftSettings {
    fn default() -> Self {
        SoftSettings {
            enabled: false,
            subdivision: 2,
            stiffness: 0.5,
            pressure: 0.0,
            iterations: 20,
        }
    }
}

/// たわませた結果。細かい三角形の網(3D表示・当たり判定はこれを使う)。
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SoftMesh {
    pub positions: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    /// 三角形→元の面(当たり判定・色分け用)
    pub triangle_faces: Vec<FaceId>,
    /// 三角形→層番号(下から0,1,2…)
    pub triangle_layers: Vec<u32>,
    /// 警告(日本語)
    pub warnings: Vec<String>,
}

/// 面を`div`等分したときの三角形数の見込み。
fn estimate(faces: &[Face], div: u32) -> usize {
    let per: usize = faces
        .iter()
        .map(|f| f.vertices.len().saturating_sub(2))
        .sum();
    per.saturating_mul((div as usize).saturating_mul(div as usize))
}

/// 剛体折りの結果(基準の形)と層順序から、たわませた三角形網を作る。
///
/// `settings.enabled` が false のときは細分も反復も行わず、`frame` の多角形を
/// 三角形へ分けただけの網を返す(呼び出し側が表示・当たり判定で常に同じ型を
/// 使えるようにするため。計算量は三角形分割のみ)。
pub fn relax(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &Frame3D,
    settings: &SoftSettings,
) -> SoftMesh {
    let mut warnings = Vec::new();
    let stiffness = settings.stiffness.clamp(0.0, 1.0);
    let pressure = settings.pressure.clamp(0.0, 1.0);
    if stiffness != settings.stiffness || pressure != settings.pressure {
        warnings.push("たわみの硬さ・膨らみの強さは0.0〜1.0に丸めました".to_string());
    }
    let iterations = settings.iterations.clamp(1, MAX_ITERATIONS);

    let mut sub = if settings.enabled {
        settings.subdivision.min(MAX_SUBDIVISION)
    } else {
        0
    };
    if settings.enabled && settings.subdivision > MAX_SUBDIVISION {
        warnings.push(format!("面の分割の細かさは{MAX_SUBDIVISION}までに丸めました"));
    }
    if settings.enabled && sub > 0 && estimate(faces, 1 << sub) > MAX_TRIANGLES {
        while sub > 0 && estimate(faces, 1 << sub) > MAX_TRIANGLES {
            sub -= 1;
        }
        warnings.push(format!(
            "展開図が大きいため、たわみの分割の細かさを{sub}へ自動で落としました"
        ));
    }

    let mut raw = subdivide::build_mesh(cp, faces, frame, 1 << sub);
    warnings.append(&mut raw.warnings);
    if settings.enabled {
        let c = solve::build(&raw, &raw.positions, stiffness, pressure, iterations);
        let broken = solve::run(&mut raw.positions, &c, iterations);
        if broken > 0 {
            warnings.push(format!(
                "たわみ計算で層の重なり順を{broken}箇所保てませんでした。いちばん近い形で表示します"
            ));
        }
    }
    SoftMesh {
        positions: raw.positions.iter().map(DVec3::to_array).collect(),
        triangles: raw.triangles,
        triangle_faces: raw.tri_face,
        triangle_layers: raw.tri_layer,
        warnings,
    }
}
