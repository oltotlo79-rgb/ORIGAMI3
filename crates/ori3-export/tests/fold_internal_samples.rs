//! `docs/improvement-roadmap-2026-08-24.md` §12.6-8 の受け入れ検査。
//!
//! 提案の探索が書き出した6手の文書・やっこさん・カエル・鳥の基本形・伝承の折り鶴の内部5標本で **100回連続の
//! import→export→import** を行い、§12.6 の2〜7の数値を100/100で満たすことを固定する。
//! 連鎖は `import#0 → export#1 → import#1 → export#2 → import#2 → …` で、
//! 隣り合う3つ組がそれぞれ1回の import→export→import になる。
//!
//! # 標本の出所（新しいfixtureを作らない）
//!
//! §10.1「追跡対象のfixtureだけを参照する」と§10.7.6「通常テストは記録を読んで
//! 照合するだけにする」に従い、**新しい標本ファイルを作らず**、既に追跡済みの正本を
//! `include_str!` で読む。複製を置くと正本と分岐して§10.7.6の再発になるためである。
//!
//! - 提案の探索が書き出した6手の文書 / やっこさん / 鳥の基本形:
//!   `crates/ori3-rigid/tests/fixtures/check-*.ori3`。
//!   同じ相対参照は `crates/ori3-layers/tests/flat_endpoint.rs` が既に使っている。
//! - カエル: `apps/desktop/src/lib/__fixtures__/frog.json`。
//!   `apps/desktop/src-tauri/tests/fold_all_frog.rs:20` と同じ入口で、
//!   `crates/ori3-layers/tests/acceptance_frog.rs` が正本から read-only 照合している。
//!
//! # 1つ目の標本が折り鶴ではない点（2026-09-05 実測）
//!
//! `check-proposal-6step.ori3`（旧名 `check-crane.ori3`）は、**提案の探索が返した6手を
//! そのまま適用して書き出した文書**であって、伝承の折り鶴ではない。完成形は凧形で終わる。
//! 旧名が「鶴」を名乗っていたため、この検査でも標本を折り鶴と取り違えていた。実体に合わせて
//! ファイル名・定数名・検査名から「鶴」を外した。伝承の折り鶴の正本は
//! `crates/ori3-layers/tests/fixtures/traditional-crane/traditional-crane-cp.ori3`、
//! そこから作った作品は `apps/desktop/tests-live/fixtures/traditional-crane-full.ori3` である。
//!
//! **正本由来の鶴は、5つ目の標本として別に足した**（2026-09-05）。
//! かつてはこの検査へ入れられなかった。`document_to_fold` が2手目（`Petal`）について
//! `UnsupportedGeometry`「step endpointを指定どおりの収束解として再生できません」
//! （`converged: false` / `best_effort: true`）を返し、往復に入る前に失敗したためである。
//! 伝承の折り鶴は紙を曲げずには折れないことが数値的に確定しており
//! （`docs/rules/03-品質ゲート.md` §7.1）、正本の受け入れ検査
//! `crates/ori3-layers/tests/acceptance_crane.rs` も剛体の収束解ではなく `flat_state_at` の
//! 平坦再生で確かめている。そこで製品側を直し、**終点が平坦な手順は剛体の収束ではなく
//! 宣言角の平坦再生で確かめる**ようにした（`crates/ori3-export/src/fold/conversion.rs` の
//! `validate_flat_endpoint`）。許容差は1つも緩めていない。裂け `1e-6`、すり抜け0、
//! 重なり順の矛盾なし、という終点の条件はそのままで、測る姿勢だけが変わっている。
//!
//! 鶴の終点比較も同じ理由で [`EndpointCheck::Flat`] を使う。既存4標本は
//! [`EndpointCheck::Rigid`] のままで、比較の中身は1つも変えていない。
//!
//! # カエルだけ数え方が違う点
//!
//! カエルの正本は展開図（頂点141・折り目280・面140）だけを持ち、折る手順を持たない。
//! したがってカエルでは step frame に関する§12.6-4・5の比較件数が0件になる。
//! 手順を伴う数値は残り3標本（合計12手順）と、既存の
//! `crates/ori3-export/tests/fold_document_roundtrip.rs` が非0件で担保する。
//! 手順を持たない標本へ手順を**作り足すことはしない**（記録の捏造になるため）。
//!
//! # 比較の型
//!
//! §10.7.7 に従い、**計算で出た座標・角度は許容差付き**（`1e-9`）で、
//! **件数・つながり・種類・ID・faceOrdersの整数三つ組は完全一致**で比べる。
//! JSON文字列そのものの完全一致は使わない（座標の最下位1桁で落ちた過去の失敗と同型のため）。

use glam::{DVec2, DVec3};
use ori3_cp::{Face, extract_faces};
use ori3_export::fold::{
    FoldAssignment, FoldFile, FoldIssue, FoldIssueCode, FoldIssueSeverity, document_to_fold,
    fold_to_document, parse_fold_1_2, unsupported_fields, write_fold_1_2,
};
use ori3_geometry::Isometry2;
use ori3_layers::{FlatState, flat_state_at, replay};
use ori3_model::{
    CreasePattern, DisplaySettings, Document, Edge, EdgeKind, Face3D, FaceId, Frame3D, Paper,
    SCHEMA_VERSION, Vertex,
};
use ori3_rigid::{max_seam_gap, self_intersection_pairs};

/// §12.6-8 が定める連続回数。減らさない。
const ROUND_TRIPS: usize = 100;
/// §12.6-3 の 2D座標と fold angle の境目。`FoldComparisonOptions::default()` と同値。
const COORDINATE_EPS: f64 = 1e-9;
const ANGLE_EPS_DEG: f64 = 1e-9;
/// §12.6-4 の終点距離・seam の境目。
const ENDPOINT_EPS: f64 = 1e-6;

const PROPOSAL_SIX_STEP: &str =
    include_str!("../../ori3-rigid/tests/fixtures/check-proposal-6step.ori3");
const YAKKO: &str = include_str!("../../ori3-rigid/tests/fixtures/check-yakko.ori3");
const BIRD_BASE: &str = include_str!("../../ori3-rigid/tests/fixtures/check-bird-base.ori3");
const FROG: &str = include_str!("../../../apps/desktop/src/lib/__fixtures__/frog.json");
/// 伝承の折り鶴の正本 `crates/ori3-layers/tests/fixtures/traditional-crane/traditional-crane-cp.ori3`
/// から作った、追跡済みの作品。単純折り・花弁折り・中割り折りの3手を持つ。
const TRADITIONAL_CRANE: &str =
    include_str!("../../../apps/desktop/tests-live/fixtures/traditional-crane-full.ori3");

/// 各手順の終点をどの姿勢として比べるか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EndpointCheck {
    /// 剛体solverが再生した姿勢どうしを比べる。
    Rigid,
    /// 宣言した折り角の平坦再生で決まる姿勢どうしを比べる。
    ///
    /// 終点が平坦（全ての山谷が0か±180度）な作品では、その姿勢は宣言角と
    /// 記録された重なり順だけで一意に決まり、剛体solverの収束に依存しない。
    Flat,
}

#[test]
fn proposal_six_step_document_survives_one_hundred_consecutive_import_export_imports() {
    let summary = assert_hundred_round_trips("提案の6手", document_fixture(PROPOSAL_SIX_STEP));
    assert_eq!(summary.steps, 6, "提案の6手の手順数");
    assert!(
        summary.endpoint_comparisons > 0,
        "提案の6手は終点比較を行う"
    );
    assert!(
        summary.face_order_triples > 0,
        "提案の6手はfaceOrdersを持つ"
    );
}

#[test]
fn yakko_survives_one_hundred_consecutive_import_export_imports() {
    let summary = assert_hundred_round_trips("やっこさん", document_fixture(YAKKO));
    assert_eq!(summary.steps, 1, "やっこさんの手順数");
    assert!(summary.endpoint_comparisons > 0, "やっこさんは終点比較を行う");
    assert!(summary.face_order_triples > 0, "やっこさんはfaceOrdersを持つ");
}

#[test]
fn bird_base_survives_one_hundred_consecutive_import_export_imports() {
    let summary = assert_hundred_round_trips("鳥の基本形", document_fixture(BIRD_BASE));
    assert_eq!(summary.steps, 5, "鳥の基本形の手順数");
    assert!(summary.endpoint_comparisons > 0, "鳥の基本形は終点比較を行う");
    assert!(summary.face_order_triples > 0, "鳥の基本形はfaceOrdersを持つ");
}

#[test]
fn frog_survives_one_hundred_consecutive_import_export_imports() {
    let summary = assert_hundred_round_trips("カエル", frog_document());
    // 正本が展開図だけを持つことを、黙って0にせずここで固定する。
    assert_eq!(summary.steps, 0, "カエルの正本は折る手順を持たない");
    assert_eq!(summary.vertices, 141, "カエルの頂点数");
    assert_eq!(summary.edges, 280, "カエルの折り目数");
}

/// 正本由来の伝承折り鶴。剛体では収束しない（`docs/rules/03-品質ゲート.md` §7.1）が、
/// 3手とも終点は平坦なので、平坦再生で終点を確かめれば100回の往復を通せる。
#[test]
fn traditional_crane_survives_one_hundred_consecutive_import_export_imports() {
    let seed = document_fixture(TRADITIONAL_CRANE);
    // 正本の実測値。取込・書出しのどこかで頂点や折り目が落ちれば、ここで落ちる。
    assert_eq!(seed.cp.vertices.len(), 56, "正本由来の鶴の頂点数");
    assert_eq!(seed.cp.edges.len(), 114, "正本由来の鶴の折り目数");
    assert_eq!(seed.sequence.len(), 3, "正本由来の鶴の手順数");
    let summary = assert_hundred_round_trips_with("伝承の折り鶴", seed, EndpointCheck::Flat);
    assert_eq!(summary.vertices, 56, "往復後も鶴の頂点数");
    assert_eq!(summary.edges, 114, "往復後も鶴の折り目数");
    assert_eq!(summary.steps, 3, "往復後も鶴の手順数");
    assert!(
        summary.endpoint_comparisons > 0,
        "伝承の折り鶴は終点比較を行う"
    );
    assert!(
        summary.face_order_triples > 0,
        "伝承の折り鶴はfaceOrdersを持つ"
    );
}

#[derive(Debug, Default)]
struct RoundTripSummary {
    vertices: usize,
    edges: usize,
    steps: usize,
    /// 隣り合う往復どうしの最大ずれ。
    coordinate_max_consecutive: f64,
    angle_max_consecutive: f64,
    /// 1回目の書出しと各回の書出しの最大ずれ（100回ぶんの累積）。
    coordinate_max_cumulative: f64,
    angle_max_cumulative: f64,
    coordinate_comparisons: usize,
    angle_comparisons: usize,
    /// 完全一致した B/M/V の数と、比べた総数。
    assignment_matches: usize,
    assignment_total: usize,
    /// step終点の3D頂点どうしの最大距離と比較件数。
    endpoint_max_distance: f64,
    endpoint_comparisons: usize,
    seam_max: f64,
    penetration_pairs: usize,
    face_order_triples: usize,
    aux_edges: usize,
    fu_downgrade_warnings: usize,
    unsupported_field_reports: usize,
    silently_dropped_fields: usize,
}

fn assert_hundred_round_trips(label: &str, seed: Document) -> RoundTripSummary {
    assert_hundred_round_trips_with(label, seed, EndpointCheck::Rigid)
}

fn assert_hundred_round_trips_with(
    label: &str,
    seed: Document,
    endpoint_check: EndpointCheck,
) -> RoundTripSummary {
    let mut summary = RoundTripSummary {
        vertices: seed.cp.vertices.len(),
        edges: seed.cp.edges.len(),
        steps: seed.sequence.len(),
        ..RoundTripSummary::default()
    };

    // ORIGAMI3 → FOLD。取込前の作品を1回書き出し、その結果から連鎖を始める。
    let seed_export = document_to_fold(&seed)
        .unwrap_or_else(|error| panic!("{label}: 正本をFOLDへ書き出せる: {error:?}"));
    assert_eq!(seed_export.file.file_spec, 1.2, "{label}: 書出し版は常に1.2");
    assert_seed_matches_export(label, &seed, &seed_export.file, &mut summary);

    let seed_json = write_fold_1_2(&seed_export.file)
        .unwrap_or_else(|error| panic!("{label}: 1回目のFOLD JSONを書ける: {error:?}"));
    let parsed = parse_fold_1_2(&seed_json)
        .unwrap_or_else(|error| panic!("{label}: 1回目のFOLD JSONを読める: {error}"));
    let first = fold_to_document(&parsed)
        .unwrap_or_else(|error| panic!("{label}: 1回目のFOLDを取込める: {error:?}"));
    assert_import_accounting(label, 0, &parsed, &first.warnings, &mut summary);

    let mut document = first.document;
    assert_document_matches_file(label, 0, &document, &parsed);
    let origin_endpoints = endpoint_frames(&document, endpoint_check);
    // 基準は「取込んだ作品からの1回目の書出し」にする。正本と取込後では頂点IDの
    // 並べ替えが1回入るため、面の並び方まで同一と決めつけないためである。
    let mut baseline_file: Option<FoldFile> = None;

    for round in 1..=ROUND_TRIPS {
        // export
        let exported = document_to_fold(&document)
            .unwrap_or_else(|error| panic!("{label}: {round}回目の書出し: {error:?}"));
        assert_eq!(
            exported.file.file_spec, 1.2,
            "{label}: {round}回目の書出し版は常に1.2"
        );
        assert_export_accounting(label, round, &document, &exported.warnings, &mut summary);
        if baseline_file.is_none() {
            baseline_file = Some(exported.file.clone());
        }
        let baseline = baseline_file
            .as_ref()
            .expect("1回目の書出しを基準にしている");
        assert_file_matches_baseline(label, round, &exported.file, baseline, &mut summary);

        let json = write_fold_1_2(&exported.file)
            .unwrap_or_else(|error| panic!("{label}: {round}回目のJSONを書ける: {error:?}"));

        // import
        let parsed = parse_fold_1_2(&json)
            .unwrap_or_else(|error| panic!("{label}: {round}回目のJSONを読める: {error}"));
        let imported = fold_to_document(&parsed)
            .unwrap_or_else(|error| panic!("{label}: {round}回目の取込: {error:?}"));
        assert_import_accounting(label, round, &parsed, &imported.warnings, &mut summary);
        let next = imported.document;
        assert_document_matches_file(label, round, &next, &parsed);

        let drift = measure_drift(&document, &next, &format!("{label}: {round}回目の連続往復"));
        summary.coordinate_max_consecutive =
            summary.coordinate_max_consecutive.max(drift.coordinate_max);
        summary.angle_max_consecutive = summary.angle_max_consecutive.max(drift.angle_max);
        summary.coordinate_comparisons += drift.coordinate_comparisons;
        summary.angle_comparisons += drift.angle_comparisons;
        assert!(
            drift.coordinate_max <= COORDINATE_EPS,
            "{label}: {round}回目の連続往復の座標ずれ {:e} が {COORDINATE_EPS:e} を超えた",
            drift.coordinate_max
        );
        assert!(
            drift.angle_max <= ANGLE_EPS_DEG,
            "{label}: {round}回目の連続往復の角度ずれ {:e} 度が {ANGLE_EPS_DEG:e} 度を超えた",
            drift.angle_max
        );

        assert_endpoints_match(
            label,
            round,
            &origin_endpoints,
            &next,
            endpoint_check,
            &mut summary,
        );
        document = next;
    }

    assert_eq!(
        summary.assignment_matches, summary.assignment_total,
        "{label}: B/M/Vの一致率は100%"
    );
    assert_eq!(
        summary.silently_dropped_fields, 0,
        "{label}: 無言で捨てたfieldは0"
    );
    assert_eq!(summary.penetration_pairs, 0, "{label}: penetrationは0");
    assert!(
        summary.seam_max <= ENDPOINT_EPS,
        "{label}: seam最大 {:e} が {ENDPOINT_EPS:e} を超えた",
        summary.seam_max
    );
    assert!(
        summary.endpoint_max_distance <= ENDPOINT_EPS,
        "{label}: 終点距離最大 {:e} が {ENDPOINT_EPS:e} を超えた",
        summary.endpoint_max_distance
    );
    assert!(
        summary.coordinate_max_cumulative <= COORDINATE_EPS
            && summary.angle_max_cumulative <= ANGLE_EPS_DEG,
        "{label}: 100回ぶんの累積ずれ 座標{:e} / 角度{:e}度",
        summary.coordinate_max_cumulative,
        summary.angle_max_cumulative
    );
    assert!(
        summary.coordinate_max_consecutive <= COORDINATE_EPS
            && summary.angle_max_consecutive <= ANGLE_EPS_DEG,
        "{label}: 隣り合う往復のずれ 座標{:e} / 角度{:e}度",
        summary.coordinate_max_consecutive,
        summary.angle_max_consecutive
    );
    assert!(
        summary.coordinate_comparisons > 0 && summary.assignment_total > 0,
        "{label}: 比較件数が0の空検査にしない"
    );
    assert_eq!(
        summary.angle_comparisons > 0,
        summary.steps > 0,
        "{label}: 手順があるときだけ角度を比べ、あるなら必ず比べる"
    );
    // 4標本はいずれも補助線(ORIGAMI3の`Aux`)を持たないので、F/U縮退の警告も0件になる。
    // 「0件」であること自体を固定して、数え漏れと本当に0件であることを区別する。
    assert_eq!(summary.aux_edges, 0, "{label}: 内部標本に補助線は無い");
    assert_eq!(
        summary.fu_downgrade_warnings, 0,
        "{label}: 補助線が無いのでF/U縮退の警告も0件"
    );
    // F/U縮退と未対応fieldの件数は各回で1対1に照合済み（`assert_*_accounting`）。
    // ここでは総数を残し、書出しが未対応fieldを持ち込まないことだけを固定する。
    assert_eq!(
        summary.unsupported_field_reports, 0,
        "{label}: 内部標本の書出しは未対応fieldを1件も持ち込まない"
    );
    summary
}

/// ORIGAMI3の正本と、そこから作ったFOLDが、折り目の種類と2D座標で一致することを固定する。
fn assert_seed_matches_export(
    label: &str,
    seed: &Document,
    file: &FoldFile,
    summary: &mut RoundTripSummary,
) {
    let assignments = file
        .root
        .edges_assignment
        .as_ref()
        .unwrap_or_else(|| panic!("{label}: 書出しはedges_assignmentを持つ"));
    let coordinates = file
        .root
        .vertices_coords
        .as_ref()
        .unwrap_or_else(|| panic!("{label}: 書出しはvertices_coordsを持つ"));
    assert_eq!(
        coordinates.len(),
        seed.cp.vertices.len(),
        "{label}: 頂点数を保つ"
    );
    assert_eq!(
        assignments.len(),
        seed.cp.edges.len(),
        "{label}: 折り目数を保つ"
    );

    let topology = file
        .root
        .edges_vertices
        .as_ref()
        .unwrap_or_else(|| panic!("{label}: 書出しはedges_verticesを持つ"));
    let positions = seed
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.pos))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut sorted = seed.cp.edges.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|edge| edge.id);
    for (index, (edge, assignment)) in sorted.iter().zip(assignments).enumerate() {
        assert_eq!(
            expected_assignment(edge.kind),
            *assignment,
            "{label}: edges_assignment[{index}]は正本の折り目の種類と一致する"
        );
        // 頂点IDはcanonical並べ替えでindexへ振り直されるので、つながりは
        // 「その折り目の両端がどこにあるか」で照合する。
        let ends = &topology[index];
        assert_eq!(ends.len(), 2, "{label}: edges_vertices[{index}]は2頂点");
        for (side, (expected, actual_index)) in [
            (positions[&edge.v0], ends[0]),
            (positions[&edge.v1], ends[1]),
        ]
        .into_iter()
        .enumerate()
        {
            let actual = &coordinates[actual_index];
            for component in 0..2 {
                let difference = (expected[component] - actual[component]).abs();
                summary.coordinate_comparisons += 1;
                assert!(
                    difference <= COORDINATE_EPS,
                    "{label}: edges_vertices[{index}]の端点{side}のずれ {difference:e}"
                );
            }
        }
    }

    // canonical並べ替えは置換だけで算術を含まないので、座標の集合は差0で一致する。
    let mut before = seed
        .cp
        .vertices
        .iter()
        .map(|vertex| vertex.pos)
        .collect::<Vec<_>>();
    let mut after = coordinates
        .iter()
        .map(|point| {
            assert_eq!(point.len(), 2, "{label}: 書出しは2成分座標だけを使う");
            [point[0], point[1]]
        })
        .collect::<Vec<_>>();
    before.sort_by(|left, right| left[0].total_cmp(&right[0]).then(left[1].total_cmp(&right[1])));
    after.sort_by(|left, right| left[0].total_cmp(&right[0]).then(left[1].total_cmp(&right[1])));
    let mut maximum = 0.0_f64;
    for (left, right) in before.iter().zip(&after) {
        maximum = maximum.max((left[0] - right[0]).abs());
        maximum = maximum.max((left[1] - right[1]).abs());
        summary.coordinate_comparisons += 2;
    }
    assert!(
        maximum <= COORDINATE_EPS,
        "{label}: 正本→FOLDの座標ずれ {maximum:e} が {COORDINATE_EPS:e} を超えた"
    );
    summary.coordinate_max_cumulative = summary.coordinate_max_cumulative.max(maximum);
}

/// 取込んだDocumentが、読んだFOLD fileの topology と種類をそのまま持つことを固定する。
fn assert_document_matches_file(label: &str, round: usize, document: &Document, file: &FoldFile) {
    let coordinates = file
        .root
        .vertices_coords
        .as_ref()
        .unwrap_or_else(|| panic!("{label}: {round}回目のfileはvertices_coordsを持つ"));
    let topology = file
        .root
        .edges_vertices
        .as_ref()
        .unwrap_or_else(|| panic!("{label}: {round}回目のfileはedges_verticesを持つ"));
    let assignments = file
        .root
        .edges_assignment
        .as_ref()
        .unwrap_or_else(|| panic!("{label}: {round}回目のfileはedges_assignmentを持つ"));

    assert_eq!(
        document.cp.vertices.len(),
        coordinates.len(),
        "{label}: {round}回目の頂点数"
    );
    assert_eq!(
        document.cp.edges.len(),
        topology.len(),
        "{label}: {round}回目の折り目数"
    );
    for (index, vertex) in document.cp.vertices.iter().enumerate() {
        assert_eq!(
            u32::try_from(index).expect("頂点indexはu32で表せる"),
            vertex.id,
            "{label}: {round}回目の頂点IDはFOLD index"
        );
        let coordinate = &coordinates[index];
        // 成分数を先に固定しておく。zipで短い方に合わせて比較件数が黙って
        // 減ることを防ぐためで、0..2 の直接添字より条件は強い。
        assert_eq!(
            coordinate.len(),
            2,
            "{label}: {round}回目 vertices_coords[{index}] は2成分"
        );
        for (component, (actual, expected)) in vertex.pos.iter().zip(coordinate).enumerate() {
            let difference = (actual - expected).abs();
            assert!(
                difference <= COORDINATE_EPS,
                "{label}: {round}回目 vertices[{index}][{component}] のずれ {difference:e}"
            );
        }
    }
    for (index, edge) in document.cp.edges.iter().enumerate() {
        assert_eq!(
            u32::try_from(index).expect("折り目indexはu32で表せる"),
            edge.id,
            "{label}: {round}回目の折り目IDはFOLD index"
        );
        let ends = [
            usize::try_from(edge.v0).expect("v0はusizeで表せる"),
            usize::try_from(edge.v1).expect("v1はusizeで表せる"),
        ];
        assert_eq!(
            ends.as_slice(),
            topology[index].as_slice(),
            "{label}: {round}回目 edges_vertices[{index}] のつながり"
        );
        assert_eq!(
            expected_assignment(edge.kind),
            assignments[index],
            "{label}: {round}回目 edges_assignment[{index}] の種類"
        );
    }
}

/// 各回の書出しを1回目の書出しと比べ、topology・種類・faceOrdersの完全一致と
/// 座標・角度の許容差内一致を固定する。
fn assert_file_matches_baseline(
    label: &str,
    round: usize,
    actual: &FoldFile,
    baseline: &FoldFile,
    summary: &mut RoundTripSummary,
) {
    let (actual_root, baseline_root) = (&actual.root, &baseline.root);
    assert_eq!(
        actual_root.edges_vertices, baseline_root.edges_vertices,
        "{label}: {round}回目のcanonical edge topologyは完全一致"
    );
    assert_eq!(
        actual_root.faces_vertices, baseline_root.faces_vertices,
        "{label}: {round}回目のfaces_verticesは完全一致"
    );
    assert_eq!(
        actual.file_frames.len(),
        baseline.file_frames.len(),
        "{label}: {round}回目のstep数と順序"
    );

    let actual_assignments = actual_root
        .edges_assignment
        .as_ref()
        .unwrap_or_else(|| panic!("{label}: {round}回目のedges_assignment"));
    let baseline_assignments = baseline_root
        .edges_assignment
        .as_ref()
        .unwrap_or_else(|| panic!("{label}: 基準のedges_assignment"));
    assert_eq!(
        actual_assignments.len(),
        baseline_assignments.len(),
        "{label}: {round}回目のassignment件数"
    );
    for (left, right) in actual_assignments.iter().zip(baseline_assignments) {
        summary.assignment_total += 1;
        if left == right {
            summary.assignment_matches += 1;
        }
    }

    let actual_coordinates = actual_root
        .vertices_coords
        .as_ref()
        .unwrap_or_else(|| panic!("{label}: {round}回目のvertices_coords"));
    let baseline_coordinates = baseline_root
        .vertices_coords
        .as_ref()
        .unwrap_or_else(|| panic!("{label}: 基準のvertices_coords"));
    assert_eq!(
        actual_coordinates.len(),
        baseline_coordinates.len(),
        "{label}: {round}回目の頂点件数"
    );
    for (index, (left, right)) in actual_coordinates
        .iter()
        .zip(baseline_coordinates)
        .enumerate()
    {
        assert_eq!(left.len(), right.len(), "{label}: 座標の成分数");
        for (component, (left, right)) in left.iter().zip(right).enumerate() {
            let difference = (left - right).abs();
            summary.coordinate_comparisons += 1;
            summary.coordinate_max_cumulative = summary.coordinate_max_cumulative.max(difference);
            assert!(
                difference <= COORDINATE_EPS,
                "{label}: {round}回目 vertices_coords[{index}][{component}] の累積ずれ {difference:e}"
            );
        }
    }

    for (index, (left, right)) in actual
        .file_frames
        .iter()
        .zip(&baseline.file_frames)
        .enumerate()
    {
        assert_eq!(
            left.frame_parent, right.frame_parent,
            "{label}: {round}回目 file_frames[{index}].frame_parent"
        );
        assert_eq!(
            left.face_orders, right.face_orders,
            "{label}: {round}回目 file_frames[{index}].faceOrdersのcanonical三つ組"
        );
        summary.face_order_triples += left.face_orders.as_ref().map_or(0, Vec::len);

        let left_angles = left
            .edges_fold_angle
            .as_ref()
            .unwrap_or_else(|| panic!("{label}: {round}回目 frame {index} の角度"));
        let right_angles = right
            .edges_fold_angle
            .as_ref()
            .unwrap_or_else(|| panic!("{label}: 基準 frame {index} の角度"));
        assert_eq!(
            left_angles.len(),
            right_angles.len(),
            "{label}: {round}回目 frame {index} の角度件数"
        );
        for (edge_index, (left, right)) in left_angles.iter().zip(right_angles).enumerate() {
            match (left, right) {
                (None, None) => {}
                (Some(left), Some(right)) => {
                    let difference = (left - right).abs();
                    summary.angle_comparisons += 1;
                    summary.angle_max_cumulative = summary.angle_max_cumulative.max(difference);
                    assert!(
                        difference <= ANGLE_EPS_DEG,
                        "{label}: {round}回目 frame {index} 折り目{edge_index} の累積角度ずれ {difference:e} 度"
                    );
                }
                _ => panic!("{label}: {round}回目 frame {index} 折り目{edge_index} の角度の有無を保つ"),
            }
        }
    }
}

/// 書出しの警告が、失う内容の実数と1対1で対応することを固定する（無言の破棄0）。
fn assert_export_accounting(
    label: &str,
    round: usize,
    document: &Document,
    warnings: &[FoldIssue],
    summary: &mut RoundTripSummary,
) {
    for warning in warnings {
        assert_eq!(
            warning.severity,
            FoldIssueSeverity::Warning,
            "{label}: {round}回目の書出し警告はwarning severityだけ"
        );
        assert!(
            warning.path.starts_with('$'),
            "{label}: {round}回目の書出し警告にJSON pathがある: {warning:?}"
        );
    }
    let aux_edges = document
        .cp
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Aux)
        .count();
    let downgrades = warnings
        .iter()
        .filter(|warning| warning.code == FoldIssueCode::AssignmentDowngradedToAux)
        .count();
    assert_eq!(
        downgrades, aux_edges,
        "{label}: {round}回目、補助線1本ごとに元の種類とpathつきの警告を1件残す"
    );
    summary.aux_edges += aux_edges;
}

/// 取込の警告が、読んだFOLDのF/Uと未対応fieldの実数と1対1で対応することを固定する。
fn assert_import_accounting(
    label: &str,
    round: usize,
    file: &FoldFile,
    warnings: &[FoldIssue],
    summary: &mut RoundTripSummary,
) {
    let flat_or_unassigned = file.root.edges_assignment.as_ref().map_or(0, |assignments| {
        assignments
            .iter()
            .filter(|assignment| {
                matches!(
                    assignment,
                    FoldAssignment::Flat | FoldAssignment::Unassigned
                )
            })
            .count()
    });
    let downgrades = warnings
        .iter()
        .filter(|warning| warning.code == FoldIssueCode::AssignmentDowngradedToAux)
        .count();
    assert_eq!(
        downgrades, flat_or_unassigned,
        "{label}: {round}回目、F/U 1本ごとに元の値とpathつきの警告を1件残す"
    );
    for warning in warnings {
        assert!(
            warning.path.starts_with('$'),
            "{label}: {round}回目の取込警告にJSON pathがある: {warning:?}"
        );
        if warning.code == FoldIssueCode::AssignmentDowngradedToAux {
            assert!(
                warning.original_value.is_some(),
                "{label}: {round}回目、縮退の警告は元のassignmentを持つ: {warning:?}"
            );
        }
    }
    summary.fu_downgrade_warnings += downgrades;

    // 未対応fieldは「持っている数」と「表示した数」を突き合わせる（silent drop 0）。
    let reported = unsupported_fields(file);
    let carried = carried_unsupported_fields(file);
    assert_eq!(
        reported.len(),
        carried,
        "{label}: {round}回目、未対応fieldの表示率100%（表示 {} 件 / 実数 {carried} 件）",
        reported.len()
    );
    for issue in &reported {
        assert!(
            issue.path.starts_with('$'),
            "{label}: {round}回目、未対応fieldの報告にpathがある: {issue:?}"
        );
    }
    summary.unsupported_field_reports += reported.len();
    summary.silently_dropped_fields += carried.saturating_sub(reported.len());
}

/// `unsupported_fields` が数えるべき対象の実数を、テスト側で独立に数える。
fn carried_unsupported_fields(file: &FoldFile) -> usize {
    let mut count = file.extra_fields.len();
    count += usize::from(file.file_creator.is_some());
    count += usize::from(file.file_author.is_some());
    count += usize::from(file.file_title.is_some());
    count += usize::from(file.file_description.is_some());
    count += file
        .file_classes
        .iter()
        .filter(|class| class.as_str() != "singleModel")
        .count();
    for frame in std::iter::once(&file.root).chain(&file.file_frames) {
        count += frame.extra_fields.len();
        count += usize::from(frame.frame_title.is_some());
        count += usize::from(frame.frame_description.is_some());
        count += frame
            .frame_classes
            .iter()
            .filter(|class| !matches!(class.as_str(), "creasePattern" | "foldedForm"))
            .count();
        count += frame
            .frame_attributes
            .iter()
            .filter(|attribute| !matches!(attribute.as_str(), "2D" | "manifold" | "orientable"))
            .count();
    }
    count
}

#[derive(Debug, Default)]
struct Drift {
    coordinate_max: f64,
    angle_max: f64,
    coordinate_comparisons: usize,
    angle_comparisons: usize,
}

/// 座標・角度を許容差で、それ以外のDocument全項目を完全一致で比べる。
///
/// 測った成分は複製の中で0にしてから `assert_eq!` するので、ID・種類・件数・手順の
/// 有無・表示設定などは1つでも違えばここで落ちる。
fn measure_drift(before: &Document, after: &Document, label: &str) -> Drift {
    let mut before = before.clone();
    let mut after = after.clone();
    let mut drift = Drift::default();

    measure(
        &mut before.paper.width_mm,
        &mut after.paper.width_mm,
        &mut drift.coordinate_max,
        &mut drift.coordinate_comparisons,
    );
    measure(
        &mut before.paper.height_mm,
        &mut after.paper.height_mm,
        &mut drift.coordinate_max,
        &mut drift.coordinate_comparisons,
    );

    assert_eq!(
        before.cp.vertices.len(),
        after.cp.vertices.len(),
        "{label}: 頂点数"
    );
    for (before, after) in before.cp.vertices.iter_mut().zip(&mut after.cp.vertices) {
        for component in 0..2 {
            measure(
                &mut before.pos[component],
                &mut after.pos[component],
                &mut drift.coordinate_max,
                &mut drift.coordinate_comparisons,
            );
        }
    }

    assert_eq!(
        before.sequence.len(),
        after.sequence.len(),
        "{label}: 手順数と順序"
    );
    for (index, (before, after)) in before
        .sequence
        .iter_mut()
        .zip(&mut after.sequence)
        .enumerate()
    {
        assert_eq!(
            before.drivers.len(),
            after.drivers.len(),
            "{label}: 手順{index}の折り線数"
        );
        for (before, after) in before.drivers.iter_mut().zip(&mut after.drivers) {
            for component in 0..2 {
                measure(
                    &mut before.a[component],
                    &mut after.a[component],
                    &mut drift.coordinate_max,
                    &mut drift.coordinate_comparisons,
                );
                measure(
                    &mut before.b[component],
                    &mut after.b[component],
                    &mut drift.coordinate_max,
                    &mut drift.coordinate_comparisons,
                );
            }
            measure(
                &mut before.target_angle_deg,
                &mut after.target_angle_deg,
                &mut drift.angle_max,
                &mut drift.angle_comparisons,
            );
        }
        match (before.layer_order.as_mut(), after.layer_order.as_mut()) {
            (None, None) => {}
            (Some(before), Some(after)) => {
                assert_eq!(before.len(), after.len(), "{label}: 手順{index}の層順の面数");
                for (before, after) in before.iter_mut().zip(after) {
                    for component in 0..2 {
                        measure(
                            &mut before[component],
                            &mut after[component],
                            &mut drift.coordinate_max,
                            &mut drift.coordinate_comparisons,
                        );
                    }
                }
            }
            _ => panic!("{label}: 手順{index}の層順の有無を保つ"),
        }
    }

    assert_eq!(
        before, after,
        "{label}: 座標・角度以外のDocument全項目を完全一致で保持する"
    );
    drift
}

fn measure(before: &mut f64, after: &mut f64, maximum: &mut f64, comparisons: &mut usize) {
    assert!(before.is_finite(), "往復前の値は有限");
    assert!(after.is_finite(), "往復後の値は有限");
    *maximum = maximum.max((*before - *after).abs());
    *comparisons += 1;
    *before = 0.0;
    *after = 0.0;
}

/// 各手順の終点を1度だけ再生して保存する（比較の基準）。
fn endpoint_frames(document: &Document, check: EndpointCheck) -> Vec<Vec<Vec<[f64; 3]>>> {
    (1..=document.sequence.len())
        .map(|up_to| {
            endpoint_frame(document, up_to, check)
                .faces
                .into_iter()
                .map(|face| face.polygon)
                .collect()
        })
        .collect()
}

/// 指定した手順までの終点姿勢を1つ求める。
fn endpoint_frame(document: &Document, up_to: usize, check: EndpointCheck) -> Frame3D {
    match check {
        EndpointCheck::Rigid => replay(document, up_to, 1.0).frame,
        EndpointCheck::Flat => {
            let faces = extract_faces(&document.cp);
            let (state, warnings) = flat_state_at(document, &faces, up_to)
                .unwrap_or_else(|error| panic!("手順{up_to}を宣言角のまま平坦に再生できる: {error}"));
            assert!(
                warnings.is_empty(),
                "手順{up_to}の平坦再生に警告がある: {warnings:?}"
            );
            flat_endpoint_frame(document, &faces, &state)
        }
    }
}

/// 平坦な終点の3D姿勢を、宣言角から決まる面の配置と記録された重なり順で組み立てる。
///
/// 平坦な状態は全ての面が同じ平面に乗るので、上下は幾何ではなく作品が持つ重なり順が
/// 決める。`crates/ori3-layers/tests/acceptance_crane.rs` の `explicit_flat_frame` と
/// 同じ組み立て方である。
fn flat_endpoint_frame(document: &Document, faces: &[Face], state: &FlatState) -> Frame3D {
    let positions = document
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<std::collections::HashMap<_, _>>();
    let ranks = state
        .order
        .iter()
        .enumerate()
        .map(|(rank, id)| (*id, rank))
        .collect::<std::collections::HashMap<FaceId, usize>>();
    assert_eq!(
        ranks.len(),
        faces.len(),
        "平坦な終点の重なり順は全ての面をちょうど1回ずつ含む"
    );
    Frame3D {
        faces: faces
            .iter()
            .map(|face| {
                let placement: Isometry2 = state.placements[&face.id];
                let rank = u32::try_from(ranks[&face.id]).expect("重なり順はu32に収まる");
                Face3D {
                    face: face.id,
                    polygon: face
                        .vertices
                        .iter()
                        .map(|vertex| {
                            let moved = placement.apply(positions[vertex]);
                            [moved.x, moved.y, 0.0]
                        })
                        .collect(),
                    layer: rank,
                    surface_rank: rank,
                    mirrored: placement.mirrored,
                }
            })
            .collect(),
        warnings: Vec::new(),
    }
}

/// §12.6-4: 各終点で全頂点finite、対応終点距離 `<=1e-6`、seam `<=1e-6`、penetration 0。
fn assert_endpoints_match(
    label: &str,
    round: usize,
    origin: &[Vec<Vec<[f64; 3]>>],
    document: &Document,
    check: EndpointCheck,
    summary: &mut RoundTripSummary,
) {
    let faces = extract_faces(&document.cp);
    for (step_index, expected) in origin.iter().enumerate() {
        if check == EndpointCheck::Rigid {
            let skipped = replay(document, step_index + 1, 1.0).skipped;
            assert!(
                skipped.is_empty(),
                "{label}: {round}回目 手順{step_index}で飛ばした折りがある: {skipped:?}"
            );
        }
        let frame = endpoint_frame(document, step_index + 1, check);
        assert_eq!(
            frame.faces.len(),
            expected.len(),
            "{label}: {round}回目 手順{step_index}の面数"
        );
        for (face_index, (face, expected)) in frame.faces.iter().zip(expected).enumerate() {
            assert_eq!(
                face.polygon.len(),
                expected.len(),
                "{label}: {round}回目 手順{step_index} 面{face_index}の頂点数"
            );
            for (&actual, &expected) in face.polygon.iter().zip(expected) {
                assert!(
                    actual.into_iter().all(f64::is_finite),
                    "{label}: {round}回目 手順{step_index}の終点に非有限の座標がある"
                );
                let distance = (DVec3::from(actual) - DVec3::from(expected)).length();
                summary.endpoint_max_distance = summary.endpoint_max_distance.max(distance);
                summary.endpoint_comparisons += 1;
                assert!(
                    distance <= ENDPOINT_EPS,
                    "{label}: {round}回目 手順{step_index} 面{face_index} の終点距離 {distance:e}"
                );
            }
        }
        let seam = max_seam_gap(&document.cp, &faces, &frame);
        assert!(
            seam.is_finite() && seam <= ENDPOINT_EPS,
            "{label}: {round}回目 手順{step_index} のseam {seam:e}"
        );
        summary.seam_max = summary.seam_max.max(seam);
        let intersections = self_intersection_pairs(&frame);
        summary.penetration_pairs += intersections.len();
        assert!(
            intersections.is_empty(),
            "{label}: {round}回目 手順{step_index} のpenetration {}件",
            intersections.len()
        );
    }
}

fn expected_assignment(kind: EdgeKind) -> FoldAssignment {
    match kind {
        EdgeKind::Border => FoldAssignment::Border,
        EdgeKind::Mountain => FoldAssignment::Mountain,
        EdgeKind::Valley => FoldAssignment::Valley,
        EdgeKind::Aux => FoldAssignment::Unassigned,
    }
}

fn document_fixture(source: &str) -> Document {
    serde_json::from_str(source).expect("追跡済みの `.ori3` 正本を読める")
}

/// カエルの正本は展開図だけを持つので、同じ紙・空の手順でDocumentにする。
fn frog_document() -> Document {
    #[derive(serde::Deserialize)]
    struct FrogFixture {
        paper: Paper,
        vertices: Vec<Vertex>,
        edges: Vec<Edge>,
    }

    let fixture: FrogFixture =
        serde_json::from_str(FROG).expect("追跡済みのカエル展開図の正本を読める");
    let next_vertex_id = fixture
        .vertices
        .iter()
        .map(|vertex| vertex.id)
        .max()
        .expect("カエルに頂点がある")
        + 1;
    let next_edge_id = fixture
        .edges
        .iter()
        .map(|edge| edge.id)
        .max()
        .expect("カエルに折り目がある")
        + 1;
    Document {
        schema_version: SCHEMA_VERSION,
        paper: fixture.paper,
        cp: CreasePattern {
            vertices: fixture.vertices,
            edges: fixture.edges,
            next_vertex_id,
            next_edge_id,
        },
        sequence: Vec::new(),
        display: DisplaySettings::default(),
    }
}
