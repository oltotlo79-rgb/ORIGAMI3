//! 折り上がり(`t = 1.00`)の姿勢が求まる条件の検査。
//!
//! # 何を守っているか
//!
//! 平らに畳んだ形では、**1本の折り目が「折れている」かどうかは、
//! その折り目を挟む2つの面の裏表が食い違っているかどうかと必ず一致する**。
//! 折れている折り目を渡ると紙が裏返り、折れていない折り目を渡っても裏表は変わらないからである。
//!
//! したがって手順が記録した角(`DriverLine` の `target_angle_deg`)は、
//! 「折る/折らない」の並びとして**つじつまが合っていなければならない**。
//! 合っていない記録は、どんな解き方をしても紙がつながる形にはならない。
//! `replay` は `t = 1.00` でその形を1回で解くので、
//! つじつまの合わない記録は必ず「形が求まりませんでした」の警告になる。
//!
//! # 実測(2026-08-23。`scratchpad/flat-endpoint-converge-report.md`)
//!
//! 提案の探索が作った花弁折りの候補40件は、記録した角のつじつまが
//! **2〜4本ぶん合っておらず**、`t = 1.00` で閉包残差 `4.545e-1 〜 7.065e-1` を残して
//! 収束しなかった。全ヒンジを記録された角に固定し、初期値もその角そのものにしても
//! 残差は同じだったので、**探し方(初期値・刻み方)の問題ではない**。
//! 一方、参照どおりの折り方(この検査が作る鳥の基本形の6手)は、
//! どの手でも食い違い **0本**で、`t = 1.00` でも収束する。

use std::collections::{BTreeMap, HashMap};

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_layers::fold_through::{
    FoldDirection, FoldThroughInput, fold_through, resolve_driver_edges,
};
use ori3_layers::precrease_collapse::validate_precrease_layer_order;
use ori3_layers::techniques::TechniqueInput;
use ori3_layers::{FlatState, FoldThroughResult, flat_state_at, petal, replay, squash};
use ori3_model::{
    CreasePattern, Document, Driver, DriverLine, Edge, EdgeKind, FaceId, FoldStep, Paper,
    TechniqueKind, Vertex,
};

type Technique = fn(
    &mut CreasePattern,
    &[Face],
    &FlatState,
    &TechniqueInput,
) -> Result<FoldThroughResult, String>;

/// 折れているとみなす角の境目(度)。記録される角は `0` か `±180` なので、
/// その中間の `90` で分ける。刻み幅ではないので実測から決める値ではない。
const FOLDED_THRESHOLD_DEG: f64 = 90.0;

/// 記録した角と、その手順が到達する平らな形の裏表が食い違う折り目(辺ID昇順)。
///
/// 戻り値が空でないなら、その記録は平らに畳める形を指していない。
fn inconsistent_creases(doc: &Document, faces: &[Face]) -> Vec<u32> {
    let recorded = recorded_angles(doc);
    let (state, _) =
        flat_state_at(doc, faces, doc.sequence.len()).expect("平らに畳んだ状態が求まる");
    let mut owners: BTreeMap<u32, Vec<FaceId>> = BTreeMap::new();
    for face in faces {
        for &edge in &face.edges {
            owners.entry(edge).or_default().push(face.id);
        }
    }
    owners
        .into_iter()
        .filter(|(_, sharing)| sharing.len() == 2)
        .filter(|(hinge, sharing)| {
            let turned =
                state.placements[&sharing[0]].mirrored != state.placements[&sharing[1]].mirrored;
            let folded = recorded.get(hinge).copied().unwrap_or(0.0).abs() > FOLDED_THRESHOLD_DEG;
            turned != folded
        })
        .map(|(hinge, _)| hinge)
        .collect()
}

/// 手順が記録した角を、いまの展開図の辺へ解決したもの(後の手が勝つ)。
fn recorded_angles(doc: &Document) -> BTreeMap<u32, f64> {
    let mut recorded: BTreeMap<u32, f64> = BTreeMap::new();
    for step in &doc.sequence {
        for driver in &step.drivers {
            for hinge in resolve_driver_edges(&doc.cp, driver) {
                recorded.insert(hinge, driver.target_angle_deg);
            }
        }
    }
    recorded
}

/// 2つの面が共有している折り目(辺ID昇順)。
fn hinges(faces: &[Face]) -> Vec<u32> {
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for face in faces {
        for &edge in &face.edges {
            *counts.entry(edge).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter(|&(_, n)| n == 2)
        .map(|(edge, _)| edge)
        .collect()
}

/// 参照どおりの折り方は、どの手でも記録と平らな形が一致し、折り上がりも解ける。
#[test]
fn reference_folds_record_a_flat_state_the_solver_can_reach() {
    let doc = bird_base();
    let faces = extract_faces(&doc.cp);
    assert_eq!(doc.sequence.len(), 6, "鳥の基本形は6手");
    for up_to in 1..=doc.sequence.len() {
        let mut prefix = doc.clone();
        prefix.sequence.truncate(up_to);
        let bad = inconsistent_creases(&prefix, &faces);
        assert!(
            bad.is_empty(),
            "手{up_to}({:?}): 記録した角と平らな形の裏表が {}本 食い違う {bad:?}",
            doc.sequence[up_to - 1].kind,
            bad.len()
        );
        let replayed = replay(&prefix, up_to, 1.0);
        assert!(
            replayed.converged,
            "手{up_to}: 折り上がりが求まらない(閉包残差 {:.3e})",
            replayed.closure_rms
        );
        assert!(
            replayed.warnings.is_empty(),
            "手{up_to}: 警告が出た {:?}",
            replayed.warnings
        );
    }
}

/// 追跡済みのやっこさん作品に保存された完成順が、現在の面へ欠落なく解決され、
/// 作品固有情報を使わない重なり制約を全て満たすことを確かめる。
#[test]
fn yakko_saved_complete_layer_order_satisfies_general_constraints() {
    let document = parse_fixture(include_str!(
        "../../ori3-rigid/tests/fixtures/check-yakko.ori3"
    ));
    let faces = extract_faces(&document.cp);
    let (state, replay_warnings) = flat_state_at(&document, &faces, document.sequence.len())
        .expect("やっこさんを平坦に再生できる");
    assert!(
        replay_warnings.is_empty(),
        "やっこさんの再生警告なし: {replay_warnings:?}"
    );
    let saved_points = document
        .sequence
        .last()
        .and_then(|step| step.layer_order.as_deref())
        .expect("やっこさんの最終手に層順序が保存されている");
    assert_eq!(
        saved_points.len(),
        faces.len(),
        "保存順は最終展開図の全面を1回ずつ表す"
    );
    let (saved_order, resolution_warnings) =
        FlatState::resolve_order(&document.cp, &faces, saved_points);
    assert!(
        resolution_warnings.is_empty(),
        "保存代表点は補完なしで全面へ解決できる: {resolution_warnings:?}"
    );
    assert_eq!(
        saved_order, state.order,
        "検証対象はfixtureの最終手が保存した完成順そのもの"
    );

    let validation =
        validate_precrease_layer_order(&document.cp, &faces, &state.placements, &saved_order)
            .expect("やっこさんの一般層制約を導ける");
    let continuous_violations =
        validation.violations.continuous_crossings.len() + validation.violations.continuous.len();
    println!(
        "yakko saved layer order: adjacent violations={}/{}, taco-tortilla={}/{}, taco-taco={}/{}, continuous={}/{}, unresolved={}",
        validation.violations.adjacent_folds.len(),
        validation.counts.adjacent_folds,
        validation.violations.taco_tortilla.len(),
        validation.counts.taco_tortilla,
        validation.violations.taco_taco.len(),
        validation.counts.taco_taco,
        continuous_violations,
        validation.counts.continuous,
        validation.unresolved_overlap_pairs.len(),
    );
    assert!(
        validation.violations.adjacent_folds.is_empty(),
        "隣接M/V違反0: {:?}",
        validation.violations.adjacent_folds
    );
    assert!(
        validation.violations.taco_tortilla.is_empty(),
        "taco-tortilla違反0: {:?}",
        validation.violations.taco_tortilla
    );
    assert!(
        validation.violations.taco_taco.is_empty(),
        "taco-taco違反0: {:?}",
        validation.violations.taco_taco
    );
    assert_eq!(continuous_violations, 0, "0°連続面の違反0");
    assert!(
        validation.is_valid(),
        "やっこさんの保存順は一般制約を全て満たす: {validation:?}"
    );
}

/// 平らに畳めない記録は、黙って別の形に置き換えず、求まらなかったと知らせる。
///
/// 標本 `fixtures/petal-not-flat-foldable.ori3` は、提案の探索が実際に作った
/// 花弁折りの候補である(面21・折り目31本・最後の手は折り線7本の花弁折り)。
/// 記録された角は「折る/折らない」の並びとしてつじつまが合っておらず、
/// この数え方(平らな形の裏表と突き合わせる)では **7本**が食い違う。
/// つじつまの合う並びのうち記録にいちばん近いものを総当たりで求めると、
/// **最低4本**の「折る/折らない」を変えないと合わない。
/// どちらにしても、この形へ紙をつなげたまま到達することはできない。
///
/// この検査は次の3つを同時に固定する。
///
/// 1. 記録がつじつまの合わないものであること(**7本**)。個数なので厳密に比べてよい
/// 2. `replay` が `t = 1.00` でそれを黙って別の形に置き換えず、**警告で知らせる**こと
///    (`CLAUDE.md` §8「止めずに警告する」)
/// 3. **初期値をその角そのものに置いても残差が減らない**こと。
///    つまり収束しないのは探し方(初期値・刻み方)のせいではない
///
/// 記録の作り方が直って、この標本が平らに畳める角を持つようになったら、この検査は失敗する。
/// そのときは 1.〜3. を「食い違い0本・収束する」へ**書き直す**
/// (期待値を緩めるのではなく、直った事実に合わせて書き直す)。
#[test]
fn a_recorded_fold_that_cannot_lie_flat_is_reported_instead_of_silently_replaced() {
    // 実測(2026-08-23、最適化あり): 閉包残差RMSは再生でも、初期値をその角そのものに
    // した場合でも 7.065e-1 で変わらない。判定の境目は実測の約8割の 0.5 とする
    // (`CLAUDE.md` §10.7.9)。解けている状態の残差は 1e-13 以下なので、
    // この境目は「解けている」と「解けていない」を12桁の余裕で分ける。
    const STUCK_RESIDUAL: f64 = 0.5;

    let doc = parse_fixture(include_str!("fixtures/petal-not-flat-foldable.ori3"));
    let faces = extract_faces(&doc.cp);
    assert_eq!(faces.len(), 21, "標本の面の数");
    assert_eq!(doc.sequence.len(), 5, "標本の手の数");
    let last = doc.sequence.last().expect("最後の手");
    assert_eq!(last.kind, TechniqueKind::Petal, "最後の手は花弁折り");
    assert_eq!(last.drivers.len(), 7, "花弁折りの折り線の本数");

    let bad = inconsistent_creases(&doc, &faces);
    assert_eq!(
        bad.len(),
        7,
        "記録した角と平らな形の裏表が食い違う折り目 {bad:?}"
    );

    let up_to = doc.sequence.len();
    let replayed = replay(&doc, up_to, 1.0);
    assert!(
        !replayed.converged,
        "平らに畳めない記録なのに解けたことになっている"
    );
    assert!(
        replayed
            .warnings
            .iter()
            .any(|warning| warning.contains("形が展開図から求まりませんでした")),
        "求まらなかったことを知らせていない {:?}",
        replayed.warnings
    );

    // 全ヒンジを記録どおりに固定し、初期値もその角そのものにしても残差は減らない。
    let recorded = recorded_angles(&doc);
    let drivers: Vec<Driver> = hinges(&faces)
        .into_iter()
        .map(|hinge| Driver {
            hinge,
            target_angle_deg: recorded.get(&hinge).copied().unwrap_or(0.0),
        })
        .collect();
    let warm: HashMap<u32, f64> = drivers
        .iter()
        .map(|driver| (driver.hinge, driver.target_angle_deg))
        .collect();
    let exact = ori3_rigid::solve_near_exact_without_surface_order(
        &doc.cp,
        &faces,
        &drivers,
        &HashMap::new(),
        Some(&warm),
    );
    assert!(
        !exact.converged && exact.closure_rms > STUCK_RESIDUAL,
        "正解を初期値に置いても解けないはずだが、残差が {:.3e} まで減った",
        exact.closure_rms
    );
}

// ---- 標本の読み込み(ori3-layers は serde_json を持たないので最小限の読み取り) ----

fn parse_fixture(source: &str) -> Document {
    let paper_json = json_field(source, "paper");
    let mut document = Document::new(Paper {
        width_mm: json_f64(json_field(paper_json, "width_mm")),
        height_mm: json_f64(json_field(paper_json, "height_mm")),
    });
    document.schema_version = json_u32(json_field(source, "schema_version"));
    let cp_json = json_field(source, "cp");
    document.cp = CreasePattern {
        vertices: json_array_items(json_field(cp_json, "vertices"))
            .into_iter()
            .map(|vertex| Vertex {
                id: json_u32(json_field(vertex, "id")),
                pos: json_point(json_field(vertex, "pos")),
            })
            .collect(),
        edges: json_array_items(json_field(cp_json, "edges"))
            .into_iter()
            .map(|edge| Edge {
                id: json_u32(json_field(edge, "id")),
                v0: json_u32(json_field(edge, "v0")),
                v1: json_u32(json_field(edge, "v1")),
                kind: edge_kind(json_text(json_field(edge, "kind"))),
            })
            .collect(),
        next_vertex_id: json_u32(json_field(cp_json, "next_vertex_id")),
        next_edge_id: json_u32(json_field(cp_json, "next_edge_id")),
    };
    document.sequence = json_array_items(json_field(source, "sequence"))
        .into_iter()
        .map(|step| FoldStep {
            id: json_u32(json_field(step, "id")),
            kind: technique_kind(json_text(json_field(step, "kind"))),
            drivers: json_array_items(json_field(step, "drivers"))
                .into_iter()
                .map(|driver| DriverLine {
                    a: json_point(json_field(driver, "a")),
                    b: json_point(json_field(driver, "b")),
                    target_angle_deg: json_f64(json_field(driver, "target_angle_deg")),
                })
                .collect(),
            layer_order: Some(
                json_array_items(json_field(step, "layer_order"))
                    .into_iter()
                    .map(json_point)
                    .collect(),
            ),
            alignment: None,
            finish_soft: None,
            note: String::new(),
            technique_classification: None,
        })
        .collect();
    document
}

fn edge_kind(value: &str) -> EdgeKind {
    match value {
        "Border" => EdgeKind::Border,
        "Mountain" => EdgeKind::Mountain,
        "Valley" => EdgeKind::Valley,
        "Aux" => EdgeKind::Aux,
        other => panic!("知らない折り目の種類: {other}"),
    }
}

fn technique_kind(value: &str) -> TechniqueKind {
    match value {
        "Simple" => TechniqueKind::Simple,
        "InsideReverse" => TechniqueKind::InsideReverse,
        "Pleat" => TechniqueKind::Pleat,
        "OutsideReverse" => TechniqueKind::OutsideReverse,
        "Petal" => TechniqueKind::Petal,
        "Squash" => TechniqueKind::Squash,
        "OpenSink" => TechniqueKind::OpenSink,
        "Swivel" => TechniqueKind::Swivel,
        "Twist" => TechniqueKind::Twist,
        "Pose" => TechniqueKind::Pose,
        other => panic!("知らない技法: {other}"),
    }
}

fn json_field<'a>(source: &'a str, key: &str) -> &'a str {
    let marker = format!("\"{key}\"");
    let key_end = source
        .find(&marker)
        .map(|index| index + marker.len())
        .unwrap_or_else(|| panic!("項目 {key} がない"));
    json_value(
        source[key_end..]
            .trim_start()
            .strip_prefix(':')
            .unwrap_or_else(|| panic!("項目 {key} のあとにコロンがない"))
            .trim_start(),
    )
}

fn json_array_items(array: &str) -> Vec<&str> {
    let inner = array
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .expect("JSONの配列");
    let mut rest = inner.trim();
    let mut items = Vec::new();
    while !rest.is_empty() {
        let value = json_value(rest);
        items.push(value);
        rest = rest[value.len()..].trim_start();
        if let Some(after_comma) = rest.strip_prefix(',') {
            rest = after_comma.trim_start();
        } else {
            assert!(rest.is_empty(), "配列の要素のあいだにコンマがない");
        }
    }
    items
}

fn json_value(source: &str) -> &str {
    let source = source.trim_start();
    let first = *source.as_bytes().first().expect("空でない値");
    match first {
        b'[' | b'{' => json_container(source, first),
        b'"' => json_quoted(source),
        _ => {
            let end = source.find([',', ']', '}']).unwrap_or(source.len());
            source[..end].trim_end()
        }
    }
}

fn json_container(source: &str, opening: u8) -> &str {
    let closing = if opening == b'[' { b']' } else { b'}' };
    let mut depth = 0_u32;
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in source.bytes().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        if byte == b'"' {
            in_string = true;
        } else if byte == opening {
            depth += 1;
        } else if byte == closing {
            depth -= 1;
            if depth == 0 {
                return &source[..=index];
            }
        }
    }
    panic!("閉じていないJSON")
}

fn json_quoted(source: &str) -> &str {
    let mut escaped = false;
    for (index, byte) in source.bytes().enumerate().skip(1) {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            return &source[..=index];
        }
    }
    panic!("閉じていない文字列")
}

fn json_text(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .expect("JSONの文字列")
}

fn json_u32(value: &str) -> u32 {
    value.parse().expect("JSONの整数")
}

fn json_f64(value: &str) -> f64 {
    value.parse().expect("JSONの小数")
}

fn json_point(value: &str) -> [f64; 2] {
    let coordinates = json_array_items(value);
    assert_eq!(coordinates.len(), 2, "2次元の点");
    [json_f64(coordinates[0]), json_f64(coordinates[1])]
}

// ---- 参照の折り方(鳥の基本形) ----

fn square_doc() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

fn fold(doc: &mut Document, line: [[f64; 2]; 2], keep: [f64; 2]) {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, _) = flat_state_at(doc, &faces, up_to).expect("平らな状態から折る");
    let mut cp = doc.cp.clone();
    let res = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line,
            keep_side_point: keep,
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("折れる指定");
    let mut step = res.step;
    step.id = u32::try_from(up_to).expect("手順番号");
    doc.cp = cp;
    doc.sequence.push(step);
}

fn apply(
    doc: &mut Document,
    technique: Technique,
    flap: Vec<FaceId>,
    line: [[f64; 2]; 2],
    reference_point: [f64; 2],
    open_to_back: Option<bool>,
) {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, _) = flat_state_at(doc, &faces, up_to).expect("平らな状態から折る");
    let mut cp = doc.cp.clone();
    let res = technique(
        &mut cp,
        &faces,
        &state,
        &TechniqueInput {
            flap,
            line,
            reference_point,
            open_to_back,
            polygon: None,
            center: None,
        },
    )
    .expect("折れる指定");
    assert!(
        res.warnings.is_empty(),
        "警告なしで折れる: {:?}",
        res.warnings
    );
    let mut step = res.step;
    step.id = u32::try_from(up_to).expect("手順番号");
    doc.cp = cp;
    doc.sequence.push(step);
}

fn state_of(doc: &Document) -> FlatState {
    let faces = extract_faces(&doc.cp);
    flat_state_at(doc, &faces, doc.sequence.len())
        .expect("平らに畳める")
        .0
}

fn layers_tipped_at(doc: &Document, p: DVec2) -> Vec<FaceId> {
    let faces = extract_faces(&doc.cp);
    let state = state_of(doc);
    let pos: HashMap<u32, DVec2> = doc
        .cp
        .vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect();
    state
        .order
        .iter()
        .copied()
        .filter(|id| {
            let face = faces
                .iter()
                .find(|face| face.id == *id)
                .expect("層順序の面");
            let placement = state.placements[&face.id];
            face.vertices
                .iter()
                .filter_map(|v| pos.get(v))
                .any(|&q| (placement.apply(q) - p).length() < 1e-6)
        })
        .collect()
}

fn bird_base() -> Document {
    let mut doc = square_doc();
    fold(&mut doc, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]);
    fold(&mut doc, [[0.5, 0.0], [0.5, 0.5]], [0.25, 0.25]);
    for (line, reference) in [
        ([[0.5, 0.0], [0.5, 1.0]], [0.5, 0.1]),
        ([[0.0, 0.5], [1.0, 0.5]], [0.1, 0.5]),
    ] {
        let bottom = vec![state_of(&doc).order[0]];
        apply(&mut doc, squash, bottom, line, reference, None);
    }
    let center_line = [[0.0, 1.0], [0.5, 0.5]];
    let tip = [0.0, 1.0];
    let front = vec![*state_of(&doc).order.last().expect("最前面")];
    apply(&mut doc, petal, front, center_line, tip, None);
    let side_b = layers_tipped_at(&doc, DVec2::new(0.5, 1.0));
    let back: Vec<FaceId> = layers_tipped_at(&doc, DVec2::new(0.0, 0.5))
        .into_iter()
        .filter(|id| side_b.contains(id))
        .collect();
    assert_eq!(back.len(), 1, "背面はまだ1枚のまま(実際 {back:?})");
    apply(&mut doc, petal, back, center_line, tip, Some(true));
    doc
}
