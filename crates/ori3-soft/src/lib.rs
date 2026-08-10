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

mod cup;
mod curl;
mod grid;
mod solve;
mod subdivide;
mod symmetry;

pub use cup::{RadialCupError, RadialCupReport, RadialCupSettings, radial_cup_vertices};
pub use curl::{CurlError, CurlReport, CurlSettings, curl_vertices};
pub use symmetry::{
    HalfTurnSymmetryError, HalfTurnSymmetryReport, HalfTurnSymmetrySettings,
    enforce_half_turn_symmetry,
};

use std::collections::{BTreeMap, BTreeSet, HashMap};

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
/// 1フレームおよそ「三角形1,000枚あたり1.6ms」で、三角形12,800枚だと約21msと
/// 目標の16msを超える。8,000枚なら約13msに収まるのでこの値にしている。
const MAX_TRIANGLES: usize = 8_000;
/// 中間フレームの接触補正に使う既定の反復数。
pub const DEFAULT_OVERLAP_ITERATIONS: u32 = 4;
/// 接触補正はドラッグ中の1フレームへ入るよう、たわみより低い上限にする。
const MAX_OVERLAP_ITERATIONS: u32 = 20;

/// 剛体折りの後に掛ける、表示優先の重なり防止設定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlapSettings {
    /// falseならFrame3Dを1ビットも変更しない。
    pub enabled: bool,
    /// PBDの反復数。画面の既定は[`DEFAULT_OVERLAP_ITERATIONS`]。
    pub iterations: u32,
}

impl Default for OverlapSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            iterations: DEFAULT_OVERLAP_ITERATIONS,
        }
    }
}

/// 1フレームの接触補正結果。完全保証ではなく、近傍で見つかった接触の指標。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OverlapReport {
    pub applied: bool,
    pub corrected_vertices: usize,
    pub penetrations_before: usize,
    pub penetrations_after: usize,
    pub total_depth_before: f64,
    pub total_depth_after: f64,
    pub max_depth_before: f64,
    pub max_depth_after: f64,
    pub target_gap: f64,
}

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

/// 指定された面を全て1回ずつ含む、決定的な層順位を作る。
fn complete_ranks(ids: &BTreeSet<FaceId>, order: &[FaceId]) -> BTreeMap<FaceId, f64> {
    let mut complete = Vec::with_capacity(ids.len());
    let mut seen = BTreeSet::new();
    for &id in order {
        if ids.contains(&id) && seen.insert(id) {
            complete.push(id);
        }
    }
    complete.extend(ids.iter().copied().filter(|id| seen.insert(*id)));
    complete
        .into_iter()
        .enumerate()
        .map(|(rank, id)| (id, rank as f64))
        .collect()
}

/// 開始順と完了順を進行度で補間した面ごとの層スコア。
fn interpolated_layers(
    faces: &[Face],
    start_order: &[FaceId],
    end_order: &[FaceId],
    progress: f64,
) -> (BTreeMap<FaceId, f64>, bool) {
    let ids: BTreeSet<FaceId> = faces.iter().map(|face| face.id).collect();
    let start = complete_ranks(&ids, start_order);
    let end = complete_ranks(&ids, end_order);
    let progress = if progress.is_finite() {
        progress.clamp(0.0, 1.0)
    } else {
        1.0
    };
    // ease-in/outにして、層の上下が切り替わる前後の力の変化を急にしない。
    let t = progress * progress * (3.0 - 2.0 * progress);
    let scores = ids
        .iter()
        .map(|&id| {
            let a = start[&id];
            let b = end[&id];
            (id, a + (b - a) * t)
        })
        .collect();
    let ids: Vec<FaceId> = ids.into_iter().collect();
    let order_changes = (0..ids.len()).any(|i| {
        (i + 1..ids.len()).any(|j| {
            let (a, b) = (ids[i], ids[j]);
            (start[&a] < start[&b]) != (end[&a] < end[&b])
        })
    });
    (scores, order_changes)
}

/// 180度付近で初めて確定する層順序は、完了直前の15%で隙間を0へ滑らかに減らす。
/// 完了フレームは既存のstackLiftsが厚みを付けるため、ここで逆向きへ押し返さない。
fn completion_gap_scale(progress: f64, order_changes: bool) -> f64 {
    if !order_changes {
        return 1.0;
    }
    let progress = if progress.is_finite() {
        progress.clamp(0.0, 1.0)
    } else {
        1.0
    };
    const TAPER_START: f64 = 0.85;
    if progress <= TAPER_START {
        return 1.0;
    }
    let t = (progress - TAPER_START) / (1.0 - TAPER_START);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// 剛体解の1フレームへ、層順序ベースの面間分離と頂点-三角形接触を後段適用する。
///
/// 網のCP頂点は面をまたいで共有されるため、補正で面が多少たわんでも折り目の接続は
/// 切れない。近傍探索で見つかる接触だけを扱う表示優先の近似で、完全な無貫通は保証
/// しない。`start_order` / `end_order` はどちらも下から上の面ID列。
pub fn prevent_overlap(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &mut Frame3D,
    start_order: &[FaceId],
    end_order: &[FaceId],
    progress: f64,
    settings: &OverlapSettings,
) -> OverlapReport {
    if !settings.enabled || settings.iterations == 0 || faces.len() < 2 {
        return OverlapReport::default();
    }
    let (scores, order_changes) = interpolated_layers(faces, start_order, end_order, progress);
    let target_gap = solve::LAYER_GAP * completion_gap_scale(progress, order_changes);
    if target_gap <= ori3_model::EPS {
        return OverlapReport {
            target_gap,
            ..OverlapReport::default()
        };
    }

    // 細分なし(元の面の三角形だけ)で、ドラッグ中にも収まる軽い後処理にする。
    let mut raw = subdivide::build_mesh(cp, faces, frame, 1);
    if raw.positions.is_empty() || raw.triangles.is_empty() {
        return OverlapReport {
            target_gap,
            ..OverlapReport::default()
        };
    }
    let iterations = settings.iterations.min(MAX_OVERLAP_ITERATIONS);
    // 面内・折り目とも基準角を強く戻し、最後の接触射影だけに見た目上必要な
    // ごく小さい変形を許す。共有頂点なので折り目の接続自体は常に保たれる。
    let mut constraints = solve::build(&raw, &raw.positions, 1.0, 0.0, iterations);

    // 整数の表示層とは別に、開始→完了を補間した連続スコアを接触へ渡す。
    constraints.tri_layer = raw
        .tri_face
        .iter()
        .map(|face| scores.get(face).copied().unwrap_or(0.0))
        .collect();
    let mut sums = vec![0.0; raw.positions.len()];
    let mut counts = vec![0u32; raw.positions.len()];
    for (triangle, &layer) in raw.triangles.iter().zip(&constraints.tri_layer) {
        for &vertex in triangle {
            sums[vertex as usize] += layer;
            counts[vertex as usize] += 1;
        }
    }
    constraints.layer = sums
        .iter()
        .zip(&counts)
        .map(|(&sum, &count)| {
            if count == 0 {
                0.0
            } else {
                sum / f64::from(count)
            }
        })
        .collect();
    constraints.min_layer_diff = ori3_model::EPS;
    constraints.scale_gap_by_layer_diff = true;

    let before = solve::measure_penetration(&raw.positions, &raw.triangles, &constraints);
    let original = raw.positions.clone();
    solve::run_with_gap(
        &mut raw.positions,
        &raw.triangles,
        &constraints,
        iterations,
        target_gap,
    );
    let after = solve::measure_penetration(&raw.positions, &raw.triangles, &constraints);

    // CP頂点の共有網位置を各Face3Dへ戻す。同じ折り目の両面には必ず同じ値が入る。
    let face_by_id: HashMap<FaceId, &Face> = faces.iter().map(|face| (face.id, face)).collect();
    for output in &mut frame.faces {
        let Some(face) = face_by_id.get(&output.face) else {
            continue;
        };
        if output.polygon.len() != face.vertices.len() {
            continue;
        }
        for (point, vertex) in output.polygon.iter_mut().zip(&face.vertices) {
            if let Some(&index) = raw.corners.get(vertex) {
                *point = raw.positions[index as usize].to_array();
            }
        }
    }
    OverlapReport {
        applied: true,
        corrected_vertices: original
            .iter()
            .zip(&raw.positions)
            .filter(|(a, b)| a.to_array() != b.to_array())
            .count(),
        penetrations_before: before.count,
        penetrations_after: after.count,
        total_depth_before: before.total_depth,
        total_depth_after: after.total_depth,
        max_depth_before: before.max_depth,
        max_depth_after: after.max_depth,
        target_gap,
    }
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
        warnings.push(format!(
            "面の分割の細かさは{MAX_SUBDIVISION}までに丸めました"
        ));
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
    // 全三角形が同じ層なら、初期位置は伸び・曲げ拘束を既に満たし、層接触も
    // 袋の空気圧も発生しない。細分網は返しつつ、結果を変えない拘束構築と反復を省く。
    let has_multiple_layers = raw
        .tri_layer
        .windows(2)
        .any(|layers| layers[0] != layers[1]);
    if settings.enabled && has_multiple_layers {
        let c = solve::build(&raw, &raw.positions, stiffness, pressure, iterations);
        let broken = solve::run(&mut raw.positions, &raw.triangles, &c, iterations);
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

#[cfg(test)]
mod overlap_tests {
    use super::*;
    use ori3_model::{Edge, EdgeKind, Face3D, Vertex};

    /// 1本の折り目を共有する2三角形を、上下が逆になるほど浅く重ねた姿勢。
    fn penetrating_crease() -> (CreasePattern, Vec<Face>, Frame3D) {
        let vertices = vec![
            Vertex {
                id: 0,
                pos: [0.0, 0.0],
            },
            Vertex {
                id: 1,
                pos: [1.0, 0.0],
            },
            Vertex {
                id: 2,
                pos: [0.0, 1.0],
            },
            Vertex {
                id: 3,
                pos: [0.0, -1.0],
            },
        ];
        let edge = |id, v0, v1, kind| Edge { id, v0, v1, kind };
        let edges = vec![
            edge(0, 0, 1, EdgeKind::Mountain),
            edge(1, 1, 2, EdgeKind::Border),
            edge(2, 2, 0, EdgeKind::Border),
            edge(3, 0, 3, EdgeKind::Border),
            edge(4, 3, 1, EdgeKind::Border),
        ];
        let cp = CreasePattern {
            vertices,
            edges,
            next_vertex_id: 4,
            next_edge_id: 5,
        };
        let faces = vec![
            Face {
                id: 0,
                vertices: vec![0, 1, 2],
                edges: vec![0, 1, 2],
            },
            Face {
                id: 1,
                vertices: vec![1, 0, 3],
                edges: vec![0, 3, 4],
            },
        ];
        let shared0 = [0.0, 0.0, 0.0];
        let shared1 = [1.0, 0.0, 0.0];
        let frame = Frame3D {
            faces: vec![
                Face3D {
                    face: 0,
                    polygon: vec![shared0, shared1, [0.0, 1.0, 0.0005]],
                    layer: 0,
                },
                Face3D {
                    face: 1,
                    polygon: vec![shared1, shared0, [0.0, 1.0, -0.0005]],
                    layer: 1,
                },
            ],
            warnings: Vec::new(),
        };
        (cp, faces, frame)
    }

    #[test]
    fn overlap_correction_reduces_penetration_depth() {
        let (cp, faces, mut frame) = penetrating_crease();
        let report = prevent_overlap(
            &cp,
            &faces,
            &mut frame,
            &[0, 1],
            &[0, 1],
            0.5,
            &OverlapSettings::default(),
        );
        assert!(report.applied);
        assert!(
            report.penetrations_before > 0,
            "補正前に食い込みがある: {report:?}"
        );
        assert!(
            report.total_depth_after <= report.total_depth_before * 0.5,
            "食い込み量が半分以下へ減る: {report:?}"
        );
        assert!(
            report.max_depth_after < report.max_depth_before,
            "{report:?}"
        );
    }

    #[test]
    fn overlap_correction_keeps_the_crease_connected() {
        let (cp, faces, mut frame) = penetrating_crease();
        prevent_overlap(
            &cp,
            &faces,
            &mut frame,
            &[0, 1],
            &[0, 1],
            0.5,
            &OverlapSettings::default(),
        );
        // 面0の頂点0/1と、逆向きに並ぶ面1の頂点1/0は同じCP頂点。
        assert_eq!(frame.faces[0].polygon[0], frame.faces[1].polygon[1]);
        assert_eq!(frame.faces[0].polygon[1], frame.faces[1].polygon[0]);
    }

    #[test]
    fn disabled_overlap_correction_is_a_bitwise_passthrough() {
        let (cp, faces, mut frame) = penetrating_crease();
        let before: Vec<Vec<[f64; 3]>> = frame.faces.iter().map(|f| f.polygon.clone()).collect();
        let report = prevent_overlap(
            &cp,
            &faces,
            &mut frame,
            &[0, 1],
            &[0, 1],
            0.5,
            &OverlapSettings {
                enabled: false,
                ..OverlapSettings::default()
            },
        );
        let after: Vec<Vec<[f64; 3]>> = frame.faces.iter().map(|f| f.polygon.clone()).collect();
        assert!(!report.applied);
        assert_eq!(after, before, "OFFなら従来の剛体フレームをそのまま返す");
    }

    #[test]
    fn changing_layer_order_is_interpolated_and_tapers_near_completion() {
        let (_, faces, _) = penetrating_crease();
        let (scores, changes) = interpolated_layers(&faces, &[0, 1], &[1, 0], 0.5);
        assert!(changes);
        assert_eq!(
            scores[&0], scores[&1],
            "上下の切替点では分離方向を確定しない"
        );
        let near_end = completion_gap_scale(0.9, changes);
        assert!(
            (0.0..1.0).contains(&near_end),
            "完了へ向けて隙間を漸減: {near_end}"
        );
        assert_eq!(completion_gap_scale(1.0, changes), 0.0);
    }
}
