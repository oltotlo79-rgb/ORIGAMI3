//! SIM-011 の表現完全性を、有限の表と構造検査で固定する。
//!
//! 標本は鳥の基本形と、その構築途中にある1/2/4層packetと局所5層stack。
//! 1つの連結領域と2つの非連結領域を
//! 直接指定した [`MotionTransform::Isometry`] で動かす。表の操作そのものは
//! 名前付き技法の関数を使わない。
//!
//! 全項目を1つの `#[test]` 内で順番に実行する。剛体再生、時計、fixture I/Oを
//! 使わないため、別担当の性能検査へ負荷を掛けない。

use std::collections::BTreeSet;

use glam::DVec2;
use ori3_cp::{Face, extract_faces, validate};
use ori3_geometry::{Isometry2, dist_point_segment};
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, FoldThroughResult, fold_through};
use ori3_layers::techniques::TechniqueInput;
use ori3_layers::{
    CompoundTechnique, FlatMotionInput, FlatState, HalfPlane, LayerTurn, MotionPart,
    MotionTransform, RabbitEarInput, flat_motion, flat_state_at, inside_reverse, layers_at_point,
    open_sink, outside_reverse, petal, pleat, point_in_face, rabbit_ear, representative_point,
    squash, swivel, twist,
};
use ori3_model::{CreasePattern, Document, FaceId, Paper, TechniqueKind};

const TECHNIQUES_SOURCE: &str = include_str!("../src/techniques.rs");
const FOLD_THROUGH_SOURCE: &str = include_str!("../src/fold_through.rs");
const FLAT_MOTION_SOURCE: &str = include_str!("../src/flat_motion.rs");
const RABBIT_EAR_SOURCE: &str = include_str!("../src/rabbit_ear.rs");

/// 10回の実測最大差は0。許容差はリポジトリの幾何判定と同じEPSとし、
/// 計算した座標・角度の厳密一致は使わない。各標本の境界からの余白は別途
/// [`local_stack_witness`] で正値かつEPSより大きいことを検査する。
const RESULT_EPS: f64 = ori3_model::EPS;

type NamedTechnique = fn(
    &mut CreasePattern,
    &[Face],
    &FlatState,
    &TechniqueInput,
) -> Result<FoldThroughResult, String>;

const NAMED_TECHNIQUES: [(&str, NamedTechnique); 8] = [
    ("pleat", pleat),
    ("inside_reverse", inside_reverse),
    ("outside_reverse", outside_reverse),
    ("squash", squash),
    ("petal", petal),
    ("open_sink", open_sink),
    ("swivel", swivel),
    ("twist", twist),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LayerClass {
    One,
    Two,
    ThreeOrMore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Parity {
    Odd,
    Even,
}

#[derive(Clone, Copy, Debug)]
struct CompletenessCase {
    id: &'static str,
    selected_layers: usize,
    class: LayerClass,
    parity: Parity,
    connected_regions: usize,
}

/// 検査の正本となる表。
///
/// | ID | 選択層 | 偶奇 | 動く連結領域 | 変換 |
/// |---|---:|---|---:|---|
/// | L1-R1 | 1 | 奇 | 1 | 名前なしIsometry |
/// | L1-R2 | 1 | 奇 | 2 | 名前なしIsometry |
/// | L2-R1 | 2 | 偶 | 1 | 名前なしIsometry |
/// | L2-R2 | 2 | 偶 | 2 | 名前なしIsometry |
/// | L5-R1 | 5(3以上) | 奇 | 1 | 名前なしIsometry |
/// | L5-R2 | 5(3以上) | 奇 | 2 | 名前なしIsometry |
/// | L4-R1 | 4(3以上) | 偶 | 1 | 名前なしIsometry |
/// | L4-R2 | 4(3以上) | 偶 | 2 | 名前なしIsometry |
const COMPLETENESS_CASES: [CompletenessCase; 8] = [
    CompletenessCase {
        id: "L1-R1",
        selected_layers: 1,
        class: LayerClass::One,
        parity: Parity::Odd,
        connected_regions: 1,
    },
    CompletenessCase {
        id: "L1-R2",
        selected_layers: 1,
        class: LayerClass::One,
        parity: Parity::Odd,
        connected_regions: 2,
    },
    CompletenessCase {
        id: "L2-R1",
        selected_layers: 2,
        class: LayerClass::Two,
        parity: Parity::Even,
        connected_regions: 1,
    },
    CompletenessCase {
        id: "L2-R2",
        selected_layers: 2,
        class: LayerClass::Two,
        parity: Parity::Even,
        connected_regions: 2,
    },
    CompletenessCase {
        id: "L5-R1",
        selected_layers: 5,
        class: LayerClass::ThreeOrMore,
        parity: Parity::Odd,
        connected_regions: 1,
    },
    CompletenessCase {
        id: "L5-R2",
        selected_layers: 5,
        class: LayerClass::ThreeOrMore,
        parity: Parity::Odd,
        connected_regions: 2,
    },
    CompletenessCase {
        id: "L4-R1",
        selected_layers: 4,
        class: LayerClass::ThreeOrMore,
        parity: Parity::Even,
        connected_regions: 1,
    },
    CompletenessCase {
        id: "L4-R2",
        selected_layers: 4,
        class: LayerClass::ThreeOrMore,
        parity: Parity::Even,
        connected_regions: 2,
    },
];

#[derive(Debug)]
struct CaseOutcome {
    document: Document,
    cp: CreasePattern,
    result: FoldThroughResult,
    target_layers: Vec<Vec<FaceId>>,
}

#[test]
fn sim011_completeness_table_and_generic_routes_are_permanent() {
    assert_all_named_techniques_reach_the_generic_core();
    assert_documented_undefined_generic_inputs_are_rejected();
    assert_case_table_is_complete();
    let generated_accepted = assert_defined_packet_inputs_are_not_rejected();

    let mut accepted = 0usize;
    let mut rejected = Vec::new();
    for case in COMPLETENESS_CASES {
        match run_case(case) {
            Ok(outcome) => {
                assert_valid_outcome(case, &outcome);
                accepted += 1;
            }
            Err(error) => rejected.push(format!("{}: {error}", case.id)),
        }
    }
    assert_eq!(
        accepted,
        COMPLETENESS_CASES.len(),
        "定義できる表の全8件を拒否しない(rejected={rejected:?})"
    );
    assert!(rejected.is_empty(), "扱えない表の行は0件: {rejected:?}");

    // 最難行(4層・偶数・2つの非連結領域・直接Isometry)を同じ初期入力から
    // ちょうど10回作り直す。1回目を基準とし、整数/ID/種類は完全一致、計算した
    // 小数だけはRESULT_EPSつきで比較する。
    let hardest = COMPLETENESS_CASES[7];
    let baseline = run_case(hardest).expect("決定性の基準入力は定義できる");
    assert_valid_outcome(hardest, &baseline);
    let mut deterministic_runs = 1usize;
    let mut observed_max_delta = 0.0_f64;
    for run in 2..=10 {
        let current = run_case(hardest)
            .unwrap_or_else(|error| panic!("決定性の{run}回目が拒否された: {error}"));
        assert_valid_outcome(hardest, &current);
        observed_max_delta =
            observed_max_delta.max(assert_outcomes_close(&baseline, &current, run));
        deterministic_runs += 1;
    }
    assert_eq!(deterministic_runs, 10, "同じ入力の決定性は10/10");
    assert!(
        observed_max_delta <= RESULT_EPS,
        "10回の最大数値差 {observed_max_delta:.3e} は許容差 {RESULT_EPS:.3e} 以下"
    );
    println!(
        "SIM-011: 表 {accepted}/{}件受理、定義可能な生成入力 {generated_accepted}/64件受理、決定性 {deterministic_runs}/10、最大数値差 {observed_max_delta:.3e}",
        COMPLETENESS_CASES.len()
    );
}

fn assert_all_named_techniques_reach_the_generic_core() {
    let source_names = public_function_names(TECHNIQUES_SOURCE);
    let expected_names = NAMED_TECHNIQUES
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect::<BTreeSet<_>>();
    let named_kind_names = [
        TechniqueKind::Simple,
        TechniqueKind::Pleat,
        TechniqueKind::InsideReverse,
        TechniqueKind::OutsideReverse,
        TechniqueKind::Petal,
        TechniqueKind::Squash,
        TechniqueKind::OpenSink,
        TechniqueKind::Swivel,
        TechniqueKind::Twist,
        TechniqueKind::Pose,
    ]
    .into_iter()
    .filter_map(named_technique_kind_name)
    .map(str::to_string)
    .collect::<BTreeSet<_>>();
    assert_eq!(
        named_kind_names, expected_names,
        "TechniqueKindの名前付き8件と公開技法関数が一致する"
    );
    assert_eq!(
        source_names, expected_names,
        "techniques.rsの公開技法は正本8件と同じ集合。追加時は経路表も同時に更新する"
    );
    let compound_names = [
        CompoundTechnique::Pleat,
        CompoundTechnique::InsideReverse,
        CompoundTechnique::OutsideReverse,
        CompoundTechnique::Petal,
        CompoundTechnique::Squash,
        CompoundTechnique::OpenSink,
        CompoundTechnique::Swivel,
        CompoundTechnique::Twist,
    ]
    .map(compound_technique_name)
    .into_iter()
    .map(str::to_string)
    .collect::<BTreeSet<_>>();
    assert_eq!(
        compound_names, expected_names,
        "CompoundTechniqueの正本8件と公開技法関数が一致する"
    );

    // 3件はSession::fold -> fold_through、5件はflat_motionへ直接委譲する。
    let pleat_body = function_code(TECHNIQUES_SOURCE, "pleat");
    assert!(
        call_count(&pleat_body, "fold") >= 2,
        "段折りはSession::foldを通る"
    );
    let inside_body = function_code(TECHNIQUES_SOURCE, "inside_reverse");
    assert!(call_count(&inside_body, "reverse_fold") == 1 && inside_body.contains("true"));
    let outside_body = function_code(TECHNIQUES_SOURCE, "outside_reverse");
    assert!(call_count(&outside_body, "reverse_fold") == 1 && outside_body.contains("false"));
    let reverse_body = function_code(TECHNIQUES_SOURCE, "reverse_fold");
    assert_eq!(call_count(&reverse_body, "fold"), 1);
    let session_fold_body = function_code(TECHNIQUES_SOURCE, "fold");
    assert_eq!(call_count(&session_fold_body, "fold_through"), 1);

    for name in ["squash", "petal", "open_sink", "swivel", "twist"] {
        assert!(
            call_count(&function_code(TECHNIQUES_SOURCE, name), "flat_motion") >= 1,
            "{name} は公開flat_motionへ到達する"
        );
    }

    let public_flat_motion = function_code(FLAT_MOTION_SOURCE, "flat_motion");
    assert_eq!(call_count(&public_flat_motion, "run_motion"), 1);
    let fold_through_body = function_code(FOLD_THROUGH_SOURCE, "fold_through");
    assert_eq!(call_count(&fold_through_body, "run_motion"), 1);

    // rabbit_earは独自のTechniqueKindを持たずPleatとして記録される公開補助技法。
    // 正本8件には足さないが、広義の公開名付き折りヘルパー9件目として同じcoreへの
    // 到達を固定する。
    type RabbitEarTechnique = fn(
        &mut CreasePattern,
        &[Face],
        &FlatState,
        &RabbitEarInput,
    ) -> Result<FoldThroughResult, String>;
    let _rabbit_ear_function: RabbitEarTechnique = rabbit_ear;
    let rabbit_ear_body = function_code(RABBIT_EAR_SOURCE, "rabbit_ear");
    assert_eq!(call_count(&rabbit_ear_body, "run_motion"), 1);
    assert!(
        rabbit_ear_body.contains("TechniqueKind::Pleat"),
        "rabbit_earは独自の9番目ではなくPleatとして記録する"
    );
}

fn named_technique_kind_name(kind: TechniqueKind) -> Option<&'static str> {
    // 網羅matchなので、新しい永続kindは名前付き/汎用/姿勢の分類を更新しないと
    // この検査を組み立てられない。
    match kind {
        TechniqueKind::Simple | TechniqueKind::Pose => None,
        TechniqueKind::Pleat => Some("pleat"),
        TechniqueKind::InsideReverse => Some("inside_reverse"),
        TechniqueKind::OutsideReverse => Some("outside_reverse"),
        TechniqueKind::Petal => Some("petal"),
        TechniqueKind::Squash => Some("squash"),
        TechniqueKind::OpenSink => Some("open_sink"),
        TechniqueKind::Swivel => Some("swivel"),
        TechniqueKind::Twist => Some("twist"),
    }
}

fn compound_technique_name(technique: CompoundTechnique) -> &'static str {
    // 網羅matchなので、正本enumへ技法を足したのに本検査を更新し忘れるとコンパイル時に落ちる。
    match technique {
        CompoundTechnique::Pleat => "pleat",
        CompoundTechnique::InsideReverse => "inside_reverse",
        CompoundTechnique::OutsideReverse => "outside_reverse",
        CompoundTechnique::Petal => "petal",
        CompoundTechnique::Squash => "squash",
        CompoundTechnique::OpenSink => "open_sink",
        CompoundTechnique::Swivel => "swivel",
        CompoundTechnique::Twist => "twist",
    }
}

fn assert_documented_undefined_generic_inputs_are_rejected() {
    // line_dirを領域境界と鏡映軸から呼ぶため、入力クラスとしては下の動的検査7件。
    // 防御的な面分割不能Errは有効入力で再現できておらず、正常入力の期待Errにはしない。
    let flat_motion_code = rust_code_only(FLAT_MOTION_SOURCE);
    assert_eq!(
        call_count(&flat_motion_code, "Err"),
        6,
        "汎用核の明示的な拒否生成は既知6箇所だけ。追加時は定義不能かを分類する"
    );
    for allowed in [
        "の配置が平坦状態に見つかりません",
        "動かす紙が指定されていません",
        "動かす対象の層がありません",
        "折り線の2点が一致しています",
        "動かす側を示す点が折り線上にあります",
    ] {
        assert!(
            FLAT_MOTION_SOURCE.contains(allowed),
            "定義不能の許可済み拒否が残っている: {allowed}"
        );
    }
    assert!(
        FLAT_MOTION_SOURCE.contains("折り線が面を横切っているのに面を分割できませんでした"),
        "有効入力で起きれば欠陥になる防御的な面分割不能Errも監視する"
    );

    let document = square_document();
    let faces = extract_faces(&document.cp);
    let initial = FlatState::initial(&document.cp, &faces);
    let valid_axis = [[0.5, 0.0], [0.5, 1.0]];
    let valid_part = MotionPart {
        layers: Vec::new(),
        region: vec![HalfPlane {
            line: valid_axis,
            inside_point: [0.75, 0.5],
        }],
        transform: MotionTransform::Isometry(Isometry2::reflection(
            DVec2::from(valid_axis[0]),
            DVec2::from(valid_axis[1]),
        )),
        turn: LayerTurn::Outside(FoldDirection::Up),
        reverse_layers: None,
    };

    let mut missing_placement = initial.clone();
    missing_placement.placements.clear();
    assert_undefined_input(
        &document.cp,
        &faces,
        &missing_placement,
        vec![valid_part.clone()],
        "配置が平坦状態に見つかりません",
    );
    assert_undefined_input(
        &document.cp,
        &faces,
        &initial,
        Vec::new(),
        "動かす紙が指定されていません",
    );
    assert_undefined_input(
        &document.cp,
        &faces,
        &initial,
        vec![MotionPart {
            layers: Vec::new(),
            region: vec![HalfPlane {
                line: [[0.5, 0.5], [0.5, 0.5]],
                inside_point: [0.75, 0.5],
            }],
            transform: MotionTransform::Stay,
            turn: LayerTurn::Keep,
            reverse_layers: None,
        }],
        "折り線の2点が一致しています",
    );
    assert_undefined_input(
        &document.cp,
        &faces,
        &initial,
        vec![MotionPart {
            layers: Vec::new(),
            region: vec![HalfPlane {
                line: valid_axis,
                inside_point: [0.5, 0.5],
            }],
            transform: MotionTransform::Stay,
            turn: LayerTurn::Keep,
            reverse_layers: None,
        }],
        "動かす側を示す点が折り線上にあります",
    );
    assert_undefined_input(
        &document.cp,
        &faces,
        &initial,
        vec![MotionPart {
            layers: Vec::new(),
            region: Vec::new(),
            transform: MotionTransform::Reflect(vec![[[0.4, 0.4], [0.4, 0.4]]]),
            turn: LayerTurn::Keep,
            reverse_layers: None,
        }],
        "折り線の2点が一致しています",
    );
    assert_undefined_input(
        &document.cp,
        &faces,
        &initial,
        vec![MotionPart {
            layers: vec![FaceId::MAX],
            region: Vec::new(),
            transform: MotionTransform::Stay,
            turn: LayerTurn::Keep,
            reverse_layers: None,
        }],
        "動かす対象の層がありません",
    );
    let outside_axis = [[2.0, 0.0], [2.0, 1.0]];
    assert_undefined_input(
        &document.cp,
        &faces,
        &initial,
        vec![MotionPart {
            layers: Vec::new(),
            region: vec![HalfPlane {
                line: outside_axis,
                inside_point: [3.0, 0.5],
            }],
            transform: MotionTransform::Isometry(Isometry2::reflection(
                DVec2::from(outside_axis[0]),
                DVec2::from(outside_axis[1]),
            )),
            turn: LayerTurn::Outside(FoldDirection::Up),
            reverse_layers: None,
        }],
        "動かす対象の層がありません",
    );
}

/// 定義可能な非退化入力を、層数・軸方向・軸位置・動かす側の直積で走査する。
/// 外部乱数へ依存せず毎回同じ64件を生成し、表の代表8点だけに合わせた拒否が
/// 入っても見逃さない。1/2/4層packetと鳥の基本形の局所5層stackを同じ軸表で走査する。
fn assert_defined_packet_inputs_are_not_rejected() -> usize {
    let mut accepted = 0usize;
    for selected_layers in [1usize, 2, 4, 5] {
        let document = if selected_layers == 5 {
            bird_base_five_layer_stage_document()
        } else {
            bird_base_packet_after_folds(selected_layers)
        };
        let (faces, state) = state_of(&document);
        assert_eq!(
            state.order.len(),
            selected_layers,
            "生成入力の標本は表示どおり局所{selected_layers}層を全て指定する"
        );
        let (lo, hi) = folded_bounds(&document.cp, &faces, &state);
        let size = hi - lo;
        for vertical in [false, true] {
            for fraction in [0.2_f64, 0.35, 0.65, 0.8] {
                let coordinate = if vertical {
                    lo.x + size.x * fraction
                } else {
                    lo.y + size.y * fraction
                };
                let line = if vertical {
                    [[coordinate, lo.y], [coordinate, hi.y]]
                } else {
                    [[lo.x, coordinate], [hi.x, coordinate]]
                };
                for positive_side in [false, true] {
                    let offset = if positive_side { 0.1 } else { -0.1 };
                    let inside_point = if vertical {
                        [coordinate + size.x * offset, (lo.y + hi.y) * 0.5]
                    } else {
                        [(lo.x + hi.x) * 0.5, coordinate + size.y * offset]
                    };
                    let part = isometry_part(
                        &state.order,
                        vec![HalfPlane { line, inside_point }],
                        reflection(line),
                    );
                    let label = format!(
                        "property {selected_layers}層 {}軸 fraction={fraction} side={positive_side}",
                        if vertical { "縦" } else { "横" }
                    );
                    let _ = local_stack_witness(
                        &document.cp,
                        &faces,
                        &state,
                        &part.layers,
                        Some(&part.region),
                        &label,
                    );
                    let mut cp = document.cp.clone();
                    let result = flat_motion(
                        &mut cp,
                        &faces,
                        &state,
                        &FlatMotionInput {
                            parts: vec![part],
                            kind: TechniqueKind::Simple,
                        },
                    )
                    .unwrap_or_else(|error| panic!("{label}: 定義可能な入力を拒否した: {error}"));
                    assert!(
                        result.warnings.is_empty(),
                        "{label}: 全局所層を折る入力に警告なし: {:?}",
                        result.warnings
                    );
                    assert!(validate(&cp).is_empty(), "{label}: 出力展開図は有効");
                    accepted += 1;
                }
            }
        }
    }
    assert_eq!(accepted, 64, "定義可能な生成入力は64/64受理する");
    accepted
}

fn assert_case_table_is_complete() {
    let actual_pairs = COMPLETENESS_CASES
        .iter()
        .map(|case| (case.selected_layers, case.connected_regions))
        .collect::<BTreeSet<_>>();
    let expected_pairs = [1usize, 2, 4, 5]
        .into_iter()
        .flat_map(|layers| [(layers, 1), (layers, 2)])
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual_pairs, expected_pairs,
        "1層/2層/3以上の奇数5層・偶数4層×連結領域1/2の8組を重複なく持つ"
    );
    let ids = COMPLETENESS_CASES
        .iter()
        .map(|case| case.id)
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), COMPLETENESS_CASES.len(), "表のIDは全て一意");
    assert_eq!(
        COMPLETENESS_CASES
            .iter()
            .filter(|case| case.class == LayerClass::One)
            .count(),
        2,
        "1層は2件"
    );
    assert_eq!(
        COMPLETENESS_CASES
            .iter()
            .filter(|case| case.class == LayerClass::Two)
            .count(),
        2,
        "2層は2件"
    );
    assert_eq!(
        COMPLETENESS_CASES
            .iter()
            .filter(|case| case.class == LayerClass::ThreeOrMore)
            .count(),
        4,
        "3層以上は奇数・偶数×領域数の4件"
    );
    assert_eq!(
        COMPLETENESS_CASES
            .iter()
            .filter(|case| case.parity == Parity::Odd)
            .count(),
        4,
        "奇数層は4件"
    );
    assert_eq!(
        COMPLETENESS_CASES
            .iter()
            .filter(|case| case.parity == Parity::Even)
            .count(),
        4,
        "偶数層は4件"
    );
    assert_eq!(
        COMPLETENESS_CASES
            .iter()
            .filter(|case| case.connected_regions == 1)
            .count(),
        4,
        "1連結領域は4件"
    );
    assert_eq!(
        COMPLETENESS_CASES
            .iter()
            .filter(|case| case.connected_regions >= 2)
            .count(),
        4,
        "2つ以上の連結領域は4件"
    );
    for case in COMPLETENESS_CASES {
        let expected_parity = if case.selected_layers.is_multiple_of(2) {
            Parity::Even
        } else {
            Parity::Odd
        };
        assert_eq!(case.parity, expected_parity, "{}の偶奇", case.id);
        assert_eq!(
            case.class,
            match case.selected_layers {
                1 => LayerClass::One,
                2 => LayerClass::Two,
                _ => LayerClass::ThreeOrMore,
            },
            "{}の層区分",
            case.id
        );
    }
}

fn run_case(case: CompletenessCase) -> Result<CaseOutcome, String> {
    let (document, parts) = if case.selected_layers == 5 {
        bird_base_five_layer_motion(case.connected_regions)
    } else {
        bird_base_packet_motion(case.selected_layers, case.connected_regions)
    };
    let (faces, state) = state_of(&document);
    assert!(
        parts
            .iter()
            .all(|part| part.layers.len() == case.selected_layers),
        "{}の各連結領域は{}層を明示的に選ぶ: {:?}",
        case.id,
        case.selected_layers,
        parts.iter().map(|part| &part.layers).collect::<Vec<_>>()
    );
    assert!(
        parts.len() >= 2,
        "{}は単一の普通折りでなく、複数の直接等長変換からなる名前なし動作",
        case.id
    );
    assert!(
        parts
            .iter()
            .all(|part| matches!(part.transform, MotionTransform::Isometry(_))),
        "{}は正本8技法やReflect列でなく直接Isometryだけを使う",
        case.id
    );
    for (part_index, part) in parts.iter().enumerate() {
        let label = format!(
            "{} part {part_index} の局所{}層",
            case.id, case.selected_layers
        );
        let _ = local_stack_witness(
            &document.cp,
            &faces,
            &state,
            &part.layers,
            Some(&part.region),
            &label,
        );
    }
    let target_layers = parts.iter().map(|part| part.layers.clone()).collect();

    let mut cp = document.cp.clone();
    let result = flat_motion(
        &mut cp,
        &faces,
        &state,
        &FlatMotionInput {
            parts: parts.clone(),
            kind: TechniqueKind::Simple,
        },
    )?;
    assert_every_target_layer_and_region_moved(MotionApplication {
        case,
        before_cp: &document.cp,
        before_faces: &faces,
        before_state: &state,
        parts: &parts,
        after_cp: &cp,
        result: &result,
    });
    let mut completed = document;
    completed.cp = cp.clone();
    let mut saved_step = result.step.clone();
    saved_step.id = u32::try_from(completed.sequence.len()).expect("手順数はu32に収まる");
    completed.sequence.push(saved_step);
    Ok(CaseOutcome {
        document: completed,
        cp,
        result,
        target_layers,
    })
}

fn bird_base_packet_motion(
    selected_layers: usize,
    connected_regions: usize,
) -> (Document, Vec<MotionPart>) {
    let document = bird_base_packet_after_folds(selected_layers);
    let (faces, state) = state_of(&document);
    assert_eq!(
        state.order.len(),
        selected_layers,
        "鳥の基本形の構築途中に{selected_layers}面のpacketがある"
    );
    assert_eq!(faces.len(), selected_layers, "packetは層ごとに1面");
    let label = format!("鳥の基本形の構築途中の{selected_layers}層packet");
    let _ = local_stack_witness(&document.cp, &faces, &state, &state.order, None, &label);
    let bounds = folded_bounds(&document.cp, &faces, &state);
    let parts = match connected_regions {
        1 => one_connected_unnamed_motion(&state.order, bounds),
        2 => two_connected_unnamed_motion(&state.order, bounds),
        other => panic!("未定義の連結領域数: {other}"),
    };
    (document, parts)
}

fn bird_base_five_layer_motion(connected_regions: usize) -> (Document, Vec<MotionPart>) {
    let document = bird_base_five_layer_stage_document();
    let (faces, state) = state_of(&document);
    let (layers, bounds) = local_stack_of_depth(
        &document.cp,
        &faces,
        &state,
        5,
        "鳥の基本形に実在する局所5層stack",
    );
    // 5層の共通内部に境界を置く。1領域は3象限を2種類の鏡映とその合成回転で、
    // 2領域は静止帯を挟む左右を同時に動かす。どちらも単一の普通折りでなく、
    // 正本8技法の入口にも分類されない。
    let parts = match connected_regions {
        1 => one_connected_unnamed_motion(&layers, bounds),
        2 => two_connected_unnamed_motion(&layers, bounds),
        other => panic!("未定義の連結領域数: {other}"),
    };
    (document, parts)
}

/// 鳥の基本形を畳み平面上で走査し、ちょうど`depth`枚が共通の内部点を覆う
/// 局所stackを返す。材料上の面どうしの接続と、畳み平面上の層数は別概念なので、
/// ここでは同一点を覆うことだけを数える。
fn local_stack_of_depth(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    depth: usize,
    label: &str,
) -> (Vec<FaceId>, (DVec2, DVec2)) {
    let (lo, hi) = folded_bounds(cp, faces, state);
    let mut candidates = faces
        .iter()
        .map(|face| state.placements[&face.id].apply(DVec2::from(representative_point(cp, face))))
        .collect::<Vec<_>>();
    const GRID: usize = 64;
    for y in 0..GRID {
        for x in 0..GRID {
            let fraction = DVec2::new(
                (x as f64 + 0.5) / GRID as f64,
                (y as f64 + 0.5) / GRID as f64,
            );
            candidates.push(lo + (hi - lo) * fraction);
        }
    }

    let mut best: Option<(Vec<FaceId>, DVec2, f64)> = None;
    let mut maximum_candidate_clearance = f64::NEG_INFINITY;
    let mut observed_depths = BTreeSet::new();
    for point in candidates {
        let layers = layers_at_point(cp, faces, state, [point.x, point.y]);
        observed_depths.insert(layers.len());
        if layers.len() != depth {
            continue;
        }
        // 選択したdepth面だけでなく全面の境界までの距離を使う。この距離より
        // 小さい領域では局所層の集合が変わらず、本当に「ちょうどdepth層」である。
        let clearance = stack_clearance(cp, faces, state, &state.order, point);
        maximum_candidate_clearance = maximum_candidate_clearance.max(clearance);
        if clearance <= RESULT_EPS || !clearance.is_finite() {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|(_, _, best_clearance)| clearance > *best_clearance)
        {
            best = Some((layers, point, clearance));
        }
    }
    let (layers, point, clearance) = best.unwrap_or_else(|| {
        panic!(
            "{label}: 共通内部点を覆う{depth}層が存在する(走査で見つかった局所層数={observed_depths:?}, {depth}層候補の最大境界距離={maximum_candidate_clearance:e})"
        )
    });
    let _ = local_stack_witness(cp, faces, state, &layers, None, label);
    let half_extent = clearance * 0.4;
    assert!(
        half_extent > RESULT_EPS,
        "{label}: {depth}層だけが覆う領域に十分な内側余白がある"
    );
    let lo = point - DVec2::splat(half_extent);
    let hi = point + DVec2::splat(half_extent);
    for corner in [lo, DVec2::new(lo.x, hi.y), DVec2::new(hi.x, lo.y), hi] {
        assert_eq!(
            layers_at_point(cp, faces, state, [corner.x, corner.y]),
            layers,
            "{label}: 動作領域の四隅も同じ{depth}層"
        );
    }
    (layers, (lo, hi))
}

fn stack_clearance(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    selected: &[FaceId],
    point: DVec2,
) -> f64 {
    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<std::collections::HashMap<_, _>>();
    selected
        .iter()
        .map(|id| {
            let face = faces
                .iter()
                .find(|face| face.id == *id)
                .expect("局所層の面が存在する");
            let placement = state.placements[id];
            let polygon = face
                .vertices
                .iter()
                .filter_map(|vertex| positions.get(vertex))
                .map(|point| placement.apply(*point))
                .collect::<Vec<_>>();
            (0..polygon.len())
                .map(|index| {
                    dist_point_segment(point, polygon[index], polygon[(index + 1) % polygon.len()])
                })
                .fold(f64::INFINITY, f64::min)
        })
        .fold(f64::INFINITY, f64::min)
}

/// 指定Faceが同じ畳み平面の内部点を本当に覆うことを確認する。
/// `FlatState::order` は全Faceの大域順なので、その長さを局所層数の代わりにしない。
fn local_stack_witness(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    selected: &[FaceId],
    region: Option<&[HalfPlane]>,
    label: &str,
) -> (DVec2, f64) {
    assert!(!selected.is_empty(), "{label}: 層を1枚以上指定する");
    let selected_set = selected.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        selected_set.len(),
        selected.len(),
        "{label}: 指定層に重複がない"
    );
    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<std::collections::HashMap<_, _>>();
    let selected_faces = selected
        .iter()
        .map(|id| {
            faces
                .iter()
                .find(|face| face.id == *id)
                .unwrap_or_else(|| panic!("{label}: 指定面{id}が存在する"))
        })
        .collect::<Vec<_>>();

    let mut lo = DVec2::splat(f64::INFINITY);
    let mut hi = DVec2::splat(f64::NEG_INFINITY);
    let mut candidates = Vec::new();
    for face in &selected_faces {
        let placement = state.placements[&face.id];
        candidates.push(placement.apply(DVec2::from(representative_point(cp, face))));
        for point in face.vertices.iter().filter_map(|id| positions.get(id)) {
            let folded = placement.apply(*point);
            lo = lo.min(folded);
            hi = hi.max(folded);
        }
    }
    assert!(
        lo.is_finite() && hi.is_finite() && hi.x > lo.x && hi.y > lo.y,
        "{label}: 探索範囲は有限で正の面積"
    );
    const GRID: usize = 40;
    for y in 0..GRID {
        for x in 0..GRID {
            let fraction = DVec2::new(
                (x as f64 + 0.5) / GRID as f64,
                (y as f64 + 0.5) / GRID as f64,
            );
            candidates.push(lo + (hi - lo) * fraction);
        }
    }

    let mut best: Option<(DVec2, f64)> = None;
    for point in candidates {
        if region.is_some_and(|planes| !strictly_inside_region(point, planes)) {
            continue;
        }
        let actual = layers_at_point(cp, faces, state, [point.x, point.y])
            .into_iter()
            .collect::<BTreeSet<_>>();
        if actual != selected_set {
            continue;
        }
        let clearance = selected_faces
            .iter()
            .map(|face| {
                let placement = state.placements[&face.id];
                let polygon = face
                    .vertices
                    .iter()
                    .filter_map(|id| positions.get(id))
                    .map(|point| placement.apply(*point))
                    .collect::<Vec<_>>();
                (0..polygon.len())
                    .map(|index| {
                        dist_point_segment(
                            point,
                            polygon[index],
                            polygon[(index + 1) % polygon.len()],
                        )
                    })
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(f64::INFINITY, f64::min);
        assert!(clearance.is_finite(), "{label}: 境界距離は有限");
        if best.is_none_or(|(_, best_clearance)| clearance > best_clearance) {
            best = Some((point, clearance));
        }
    }
    let (point, clearance) =
        best.unwrap_or_else(|| panic!("{label}: 指定Faceだけが重なる共通内部点がある"));
    assert!(
        clearance > RESULT_EPS,
        "{label}: 共通点は全指定層の境界から離れた内部点(clearance={clearance:.3e})"
    );
    assert_eq!(
        layers_at_point(cp, faces, state, [point.x, point.y])
            .into_iter()
            .collect::<BTreeSet<_>>(),
        selected_set,
        "{label}: 共通内部点の局所層集合"
    );
    (point, clearance)
}

/// 3象限を同時に畳む名前なし動作。3つの部分は折り線でつながるL字形なので、
/// 動く紙の閉包は1つの連結領域。右上だけは鏡映2回を合成した180度回転になる。
fn one_connected_unnamed_motion(layers: &[FaceId], (lo, hi): (DVec2, DVec2)) -> Vec<MotionPart> {
    let mid = (lo + hi) * 0.5;
    let width = hi.x - lo.x;
    let height = hi.y - lo.y;
    let vertical = [[mid.x, lo.y], [mid.x, hi.y]];
    let horizontal = [[lo.x, mid.y], [hi.x, mid.y]];
    let right_bottom = [hi.x - width * 0.1, lo.y + height * 0.1];
    let right_top = [hi.x - width * 0.1, hi.y - height * 0.1];
    let left_top = [lo.x + width * 0.1, hi.y - height * 0.1];
    let reflect_vertical = reflection(vertical);
    let reflect_horizontal = reflection(horizontal);

    let parts = vec![
        isometry_part(
            layers,
            vec![
                HalfPlane {
                    line: vertical,
                    inside_point: right_bottom,
                },
                HalfPlane {
                    line: horizontal,
                    inside_point: right_bottom,
                },
            ],
            reflect_vertical,
        ),
        isometry_part(
            layers,
            vec![
                HalfPlane {
                    line: vertical,
                    inside_point: right_top,
                },
                HalfPlane {
                    line: horizontal,
                    inside_point: right_top,
                },
            ],
            reflect_horizontal.compose(&reflect_vertical),
        ),
        isometry_part(
            layers,
            vec![
                HalfPlane {
                    line: vertical,
                    inside_point: left_top,
                },
                HalfPlane {
                    line: horizontal,
                    inside_point: left_top,
                },
            ],
            reflect_horizontal,
        ),
    ];
    assert!(
        parts
            .iter()
            .all(|part| inside_or_on_region(mid, &part.region)),
        "3つの部分は同じ中心点でつながり、動く領域の閉包は1成分"
    );
    parts
}

/// 左右の端を、中央に正の幅を残したまま同時に折る名前なし動作。
/// 2つの帯の間には幅の半分の静止領域があり、動く領域は明確に2成分。
fn two_connected_unnamed_motion(layers: &[FaceId], (lo, hi): (DVec2, DVec2)) -> Vec<MotionPart> {
    let width = hi.x - lo.x;
    let height = hi.y - lo.y;
    let left_x = lo.x + width * 0.25;
    let right_x = hi.x - width * 0.25;
    let left = [[left_x, lo.y], [left_x, hi.y]];
    let right = [[right_x, lo.y], [right_x, hi.y]];
    let left_inside = [lo.x + width * 0.1, lo.y + height * 0.5];
    let right_inside = [hi.x - width * 0.1, lo.y + height * 0.5];
    assert!(right_x - left_x > RESULT_EPS, "2領域の間に正の隙間がある");

    let parts = vec![
        isometry_part(
            layers,
            vec![HalfPlane {
                line: left,
                inside_point: left_inside,
            }],
            reflection(left),
        ),
        isometry_part(
            layers,
            vec![HalfPlane {
                line: right,
                inside_point: right_inside,
            }],
            reflection(right),
        ),
    ];
    let gap_midpoint = DVec2::new((left_x + right_x) * 0.5, lo.y + height * 0.5);
    assert!(
        parts
            .iter()
            .all(|part| !inside_or_on_region(gap_midpoint, &part.region)),
        "左右の動く領域の間には静止領域がある"
    );
    parts
}

fn isometry_part(layers: &[FaceId], region: Vec<HalfPlane>, isometry: Isometry2) -> MotionPart {
    isometry_part_with_turn(layers, region, isometry, FoldDirection::Up)
}

fn isometry_part_with_turn(
    layers: &[FaceId],
    region: Vec<HalfPlane>,
    isometry: Isometry2,
    direction: FoldDirection,
) -> MotionPart {
    MotionPart {
        layers: layers.to_vec(),
        region,
        transform: MotionTransform::Isometry(isometry),
        turn: LayerTurn::Outside(direction),
        reverse_layers: None,
    }
}

fn reflection(line: [[f64; 2]; 2]) -> Isometry2 {
    Isometry2::reflection(DVec2::from(line[0]), DVec2::from(line[1]))
}

struct MotionApplication<'a> {
    case: CompletenessCase,
    before_cp: &'a CreasePattern,
    before_faces: &'a [Face],
    before_state: &'a FlatState,
    parts: &'a [MotionPart],
    after_cp: &'a CreasePattern,
    result: &'a FoldThroughResult,
}

fn assert_every_target_layer_and_region_moved(application: MotionApplication<'_>) {
    let MotionApplication {
        case,
        before_cp,
        before_faces,
        before_state,
        parts,
        after_cp,
        result,
    } = application;
    let after_faces = extract_faces(after_cp);
    let normalization = after_faces
        .iter()
        .find_map(|child| {
            let local = representative_point(after_cp, child);
            let parent = parent_face_at(before_cp, before_faces, local);
            let prior = before_state.placements[&parent.id];
            let folded_before = prior.apply(DVec2::from(local));
            let is_moving = parts.iter().any(|part| {
                part.layers.contains(&parent.id)
                    && strictly_inside_region(folded_before, &part.region)
            });
            if is_moving {
                None
            } else {
                let actual = result.state.placements[&child.id];
                Some(actual.compose(&prior.inverse()))
            }
        })
        .unwrap_or_else(|| panic!("{}: 全体座標系を測る静止領域がある", case.id));

    let expected_pairs = parts
        .iter()
        .enumerate()
        .flat_map(|(part_index, part)| {
            part.layers
                .iter()
                .copied()
                .map(move |face| (part_index, face))
        })
        .collect::<BTreeSet<_>>();
    let mut observed_pairs = BTreeSet::new();
    let mut moved_children = BTreeSet::new();
    let mut moved_children_by_parent = std::collections::HashMap::<FaceId, BTreeSet<FaceId>>::new();
    for child in &after_faces {
        let local = representative_point(after_cp, child);
        let parent = parent_face_at(before_cp, before_faces, local);
        let prior = before_state.placements[&parent.id];
        let folded_before = prior.apply(DVec2::from(local));
        for (part_index, part) in parts.iter().enumerate() {
            if !part.layers.contains(&parent.id)
                || !strictly_inside_region(folded_before, &part.region)
            {
                continue;
            }
            let MotionTransform::Isometry(isometry) = &part.transform else {
                panic!("{}: 表の変換は全て名前のない直接Isometry", case.id);
            };
            let expected = normalization.compose(&isometry.compose(&prior));
            let actual = result.state.placements[&child.id];
            assert!(
                actual.approx_eq(&expected, RESULT_EPS),
                "{}: part {part_index} の元面{}へ期待した等長変換を適用する\n期待={expected:?}\n実際={actual:?}",
                case.id,
                parent.id
            );
            observed_pairs.insert((part_index, parent.id));
            moved_children.insert(child.id);
            moved_children_by_parent
                .entry(parent.id)
                .or_default()
                .insert(child.id);
        }
    }
    assert_eq!(
        observed_pairs, expected_pairs,
        "{}: 各MotionPart×各指定層に内部領域があり、全て実際に動く",
        case.id
    );
    let selected_parents = parts
        .iter()
        .flat_map(|part| part.layers.iter().copied())
        .collect::<BTreeSet<_>>();
    for parent in selected_parents {
        let children = &moved_children_by_parent[&parent];
        assert_eq!(
            edge_connected_component_count(after_cp, &after_faces, children),
            case.connected_regions,
            "{}: 元面{parent}の動く領域を正の長さの共有辺で数えた連結成分",
            case.id
        );
    }
    assert!(
        parts
            .iter()
            .all(|part| matches!(part.turn, LayerTurn::Outside(FoldDirection::Up))),
        "{}: 表の動作は全て外側・上へ載せる指定",
        case.id
    );
    let mut local_overlap_witnesses = 0usize;
    let (folded_lo, folded_hi) = folded_bounds(after_cp, &after_faces, &result.state);
    let mut overlap_candidates = moved_children
        .iter()
        .map(|moved| {
            let face = after_faces
                .iter()
                .find(|face| face.id == *moved)
                .expect("動いた面が存在する");
            let local = representative_point(after_cp, face);
            result.state.placements[moved].apply(DVec2::from(local))
        })
        .collect::<Vec<_>>();
    const ORDER_GRID: usize = 64;
    for y in 0..ORDER_GRID {
        for x in 0..ORDER_GRID {
            let fraction = DVec2::new(
                (x as f64 + 0.5) / ORDER_GRID as f64,
                (y as f64 + 0.5) / ORDER_GRID as f64,
            );
            overlap_candidates.push(folded_lo + (folded_hi - folded_lo) * fraction);
        }
    }
    for folded in overlap_candidates {
        let local_layers =
            layers_at_point(after_cp, &after_faces, &result.state, [folded.x, folded.y]);
        let lowest_moved = local_layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| moved_children.contains(*layer))
            .map(|(rank, _)| rank)
            .min();
        let highest_moved = local_layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| moved_children.contains(*layer))
            .map(|(rank, _)| rank)
            .max();
        let lowest_stationary = local_layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| !moved_children.contains(*layer))
            .map(|(rank, _)| rank)
            .min();
        let highest_stationary = local_layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| !moved_children.contains(*layer))
            .map(|(rank, _)| rank)
            .max();
        if let (
            Some(lowest_moved),
            Some(highest_moved),
            Some(lowest_stationary),
            Some(highest_stationary),
        ) = (
            lowest_moved,
            highest_moved,
            lowest_stationary,
            highest_stationary,
        ) {
            local_overlap_witnesses += 1;
            if normalization.mirrored {
                // 根面が裏返った場合、normalize_to_rootは紙全体を裏から見る座標へ
                // そろえるため、下→上の順序も反転する。
                assert!(
                    highest_moved < lowest_stationary,
                    "{}: 裏返した表示座標ではOutside(Up)の可動層を静止層より下へ反転表示する: 点{folded:?} {local_layers:?}",
                    case.id
                );
            } else {
                assert!(
                    lowest_moved > highest_stationary,
                    "{}: 点{folded:?}の移動先でOutside(Up)の全可動層を静止層より上へ載せる: {local_layers:?}",
                    case.id
                );
            }
        }
    }
    assert!(
        local_overlap_witnesses > 0,
        "{}: 移動先で可動層と静止層が重なる局所層順序を検査する",
        case.id
    );
}

fn parent_face_at<'a>(
    before_cp: &CreasePattern,
    before_faces: &'a [Face],
    local: [f64; 2],
) -> &'a Face {
    before_faces
        .iter()
        .find(|face| point_in_face(before_cp, face, local))
        .expect("分割後の面には分割前の親面がある")
}

fn strictly_inside_region(point: DVec2, region: &[HalfPlane]) -> bool {
    region.iter().all(|plane| {
        let a = DVec2::from(plane.line[0]);
        let direction = DVec2::from(plane.line[1]) - a;
        let reference = direction.perp_dot(DVec2::from(plane.inside_point) - a);
        let signed_distance =
            reference.signum() * direction.perp_dot(point - a) / direction.length();
        signed_distance > RESULT_EPS
    })
}

fn inside_or_on_region(point: DVec2, region: &[HalfPlane]) -> bool {
    region.iter().all(|plane| {
        let a = DVec2::from(plane.line[0]);
        let direction = DVec2::from(plane.line[1]) - a;
        let reference = direction.perp_dot(DVec2::from(plane.inside_point) - a);
        reference.signum() * direction.perp_dot(point - a) / direction.length() >= -RESULT_EPS
    })
}

fn assert_valid_outcome(case: CompletenessCase, outcome: &CaseOutcome) {
    assert!(
        outcome
            .target_layers
            .iter()
            .all(|layers| layers.len() == case.selected_layers),
        "{}は各領域の指定層数を汎用核へ渡す",
        case.id
    );
    assert_eq!(
        outcome.result.step.kind,
        TechniqueKind::Simple,
        "{}は名前付き技法に化けない",
        case.id
    );
    assert!(
        !outcome.result.added_edges.is_empty(),
        "{}は少なくとも1本の折り線を作る",
        case.id
    );
    assert!(
        !outcome.result.step.drivers.is_empty(),
        "{}は再生可能なDriverLineを記録する",
        case.id
    );
    assert!(
        outcome.result.step.layer_order.is_some(),
        "{}は平坦な層順序を記録する",
        case.id
    );
    assert!(
        outcome.result.warnings.is_empty(),
        "{}は実際の紙で裂けず、対象層の除外もない: {:?}",
        case.id,
        outcome.result.warnings
    );
    assert!(
        validate(&outcome.cp).is_empty(),
        "{}の展開図は有効",
        case.id
    );

    let faces = extract_faces(&outcome.cp);
    let layer_points = outcome
        .result
        .step
        .layer_order
        .as_ref()
        .expect("平坦な層順序を記録する");
    assert_eq!(
        layer_points.len(),
        faces.len(),
        "{}は全ての面を安定な代表点で層順序へ記録する",
        case.id
    );
    assert!(
        layer_points.iter().flatten().copied().all(f64::is_finite),
        "{}の保存層参照点は有限",
        case.id
    );
    assert!(
        outcome
            .cp
            .vertices
            .iter()
            .flat_map(|vertex| vertex.pos)
            .all(f64::is_finite),
        "{}の展開図頂点は有限",
        case.id
    );
    let expected = faces.iter().map(|face| face.id).collect::<BTreeSet<_>>();
    let placements = outcome
        .result
        .state
        .placements
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    let order = outcome
        .result
        .state
        .order
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    assert_eq!(placements, expected, "{}は全ての面を配置する", case.id);
    assert_eq!(
        order, expected,
        "{}は全ての面を層順へ1回ずつ載せる",
        case.id
    );
    assert_eq!(
        outcome.result.state.order.len(),
        faces.len(),
        "{}の層順序に重複がない",
        case.id
    );
    for (face, placement) in &outcome.result.state.placements {
        assert!(
            placement.rotation.is_finite()
                && placement.translation.x.is_finite()
                && placement.translation.y.is_finite(),
            "{}の面{face}は有限な等長変換を持つ",
            case.id
        );
    }
    for driver in &outcome.result.step.drivers {
        assert!(
            driver.a.into_iter().chain(driver.b).all(f64::is_finite)
                && driver.target_angle_deg.is_finite(),
            "{}のDriverLineは有限",
            case.id
        );
    }

    let saved = outcome
        .document
        .sequence
        .last()
        .expect("汎用動作の保存stepがある");
    assert_eq!(
        saved.kind,
        TechniqueKind::Simple,
        "{}の保存技法種別",
        case.id
    );
    let (replayed, replay_warnings) =
        flat_state_at(&outcome.document, &faces, outcome.document.sequence.len())
            .unwrap_or_else(|error| panic!("{}の保存手順を再生できる: {error}", case.id));
    assert!(
        replay_warnings.is_empty(),
        "{}の保存手順は警告なしで再生できる: {replay_warnings:?}",
        case.id
    );
    assert_eq!(
        replayed.order, outcome.result.state.order,
        "{}の保存層順序をそのまま再現する",
        case.id
    );
    for face in &faces {
        let expected_placement = outcome.result.state.placements[&face.id];
        let actual_placement = replayed.placements[&face.id];
        assert!(
            actual_placement.approx_eq(&expected_placement, RESULT_EPS),
            "{}の保存driverが面{}の配置を再現する: 期待={expected_placement:?} 実際={actual_placement:?}",
            case.id,
            face.id
        );
    }
}

fn assert_outcomes_close(baseline: &CaseOutcome, current: &CaseOutcome, run: usize) -> f64 {
    assert_eq!(
        baseline.target_layers, current.target_layers,
        "{run}回目: 入力層"
    );
    assert_eq!(
        baseline.cp.next_vertex_id, current.cp.next_vertex_id,
        "{run}回目: 次の頂点ID"
    );
    assert_eq!(
        baseline.cp.next_edge_id, current.cp.next_edge_id,
        "{run}回目: 次の辺ID"
    );
    assert_eq!(baseline.cp.edges, current.cp.edges, "{run}回目: 辺構造");
    assert_eq!(
        baseline.cp.vertices.len(),
        current.cp.vertices.len(),
        "{run}回目: 頂点数"
    );

    let mut maximum = 0.0_f64;
    for (left, right) in baseline.cp.vertices.iter().zip(&current.cp.vertices) {
        assert_eq!(left.id, right.id, "{run}回目: 頂点ID");
        record_delta(
            &mut maximum,
            (DVec2::from(left.pos) - DVec2::from(right.pos)).length(),
            &format!("{run}回目: 頂点{}", left.id),
        );
    }

    assert_eq!(
        baseline.result.added_edges, current.result.added_edges,
        "{run}回目: 追加辺"
    );
    assert_eq!(
        baseline.result.state.order, current.result.state.order,
        "{run}回目: 層順序"
    );
    assert_eq!(
        baseline.result.warnings, current.result.warnings,
        "{run}回目: 警告"
    );
    let baseline_keys = baseline
        .result
        .state
        .placements
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    let current_keys = current
        .result
        .state
        .placements
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    assert_eq!(baseline_keys, current_keys, "{run}回目: 配置する面");
    for face in baseline_keys {
        let left = baseline.result.state.placements[&face];
        let right = current.result.state.placements[&face];
        assert!(
            left.approx_eq(&right, RESULT_EPS),
            "{run}回目: 面{face}の配置 {left:?} / {right:?}"
        );
        record_delta(
            &mut maximum,
            isometry_delta(left, right),
            &format!("{run}回目: 面{face}の配置"),
        );
    }

    let left_step = &baseline.result.step;
    let right_step = &current.result.step;
    assert_eq!(left_step.id, right_step.id, "{run}回目: step ID");
    assert_eq!(left_step.kind, right_step.kind, "{run}回目: 技法種別");
    assert_eq!(left_step.note, right_step.note, "{run}回目: 注記");
    assert!(
        left_step.alignment.is_none() && right_step.alignment.is_none(),
        "{run}回目: 汎用平坦動作に位置合わせ入力は無い"
    );
    assert!(
        left_step.finish_soft.is_none() && right_step.finish_soft.is_none(),
        "{run}回目: 汎用平坦動作にたわみ値は無い"
    );
    assert_eq!(
        left_step.drivers.len(),
        right_step.drivers.len(),
        "{run}回目: DriverLine数"
    );
    for (left, right) in left_step.drivers.iter().zip(&right_step.drivers) {
        record_delta(
            &mut maximum,
            point_delta(left.a, right.a),
            &format!("{run}回目: DriverLine始点"),
        );
        record_delta(
            &mut maximum,
            point_delta(left.b, right.b),
            &format!("{run}回目: DriverLine終点"),
        );
        record_delta(
            &mut maximum,
            (left.target_angle_deg - right.target_angle_deg).abs(),
            &format!("{run}回目: DriverLine角度"),
        );
    }
    let left_order = left_step.layer_order.as_ref().expect("平坦な層順序");
    let right_order = right_step.layer_order.as_ref().expect("平坦な層順序");
    assert_eq!(left_order.len(), right_order.len(), "{run}回目: 層参照数");
    for (&left, &right) in left_order.iter().zip(right_order) {
        record_delta(
            &mut maximum,
            point_delta(left, right),
            &format!("{run}回目: 層参照点"),
        );
    }
    assert!(maximum.is_finite(), "{run}回目の最大数値差は有限");
    assert!(
        maximum <= RESULT_EPS,
        "{run}回目の最大数値差 {maximum:.3e} は {RESULT_EPS:.3e} 以下"
    );
    maximum
}

fn record_delta(maximum: &mut f64, delta: f64, label: &str) {
    assert!(delta.is_finite(), "{label}の差は有限: {delta:?}");
    *maximum = maximum.max(delta);
}

fn isometry_delta(left: Isometry2, right: Isometry2) -> f64 {
    let raw_angle = (left.rotation - right.rotation).rem_euclid(std::f64::consts::TAU);
    let angle = raw_angle.min(std::f64::consts::TAU - raw_angle);
    angle.max((left.translation - right.translation).length())
}

fn point_delta(left: [f64; 2], right: [f64; 2]) -> f64 {
    (DVec2::from(left) - DVec2::from(right)).length()
}

fn faces_share_positive_edge(cp: &CreasePattern, left: &Face, right: &Face) -> bool {
    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<std::collections::HashMap<_, _>>();
    left.edges.iter().any(|edge_id| {
        if !right.edges.contains(edge_id) {
            return false;
        }
        let Some(edge) = cp.edges.iter().find(|edge| edge.id == *edge_id) else {
            return false;
        };
        let (Some(a), Some(b)) = (positions.get(&edge.v0), positions.get(&edge.v1)) else {
            return false;
        };
        (*b - *a).length() > RESULT_EPS
    })
}

fn edge_connected_component_count(
    cp: &CreasePattern,
    faces: &[Face],
    selected: &BTreeSet<FaceId>,
) -> usize {
    assert!(!selected.is_empty(), "動いた紙の面が1枚以上ある");
    let mut remaining = selected.clone();
    let mut components = 0usize;
    while let Some(seed) = remaining.iter().next().copied() {
        components += 1;
        remaining.remove(&seed);
        let mut frontier = vec![seed];
        while let Some(current) = frontier.pop() {
            let current_face = faces
                .iter()
                .find(|face| face.id == current)
                .expect("動いた面が存在する");
            let neighbours = remaining
                .iter()
                .copied()
                .filter(|candidate| {
                    let candidate_face = faces
                        .iter()
                        .find(|face| face.id == *candidate)
                        .expect("連結候補の面が存在する");
                    faces_share_positive_edge(cp, current_face, candidate_face)
                })
                .collect::<Vec<_>>();
            for neighbour in neighbours {
                remaining.remove(&neighbour);
                frontier.push(neighbour);
            }
        }
    }
    components
}

fn assert_undefined_input(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    parts: Vec<MotionPart>,
    expected: &str,
) {
    let before = cp.clone();
    let mut candidate = cp.clone();
    let error = flat_motion(
        &mut candidate,
        faces,
        state,
        &FlatMotionInput {
            parts,
            kind: TechniqueKind::Simple,
        },
    )
    .expect_err("この既知の定義不能入力は拒否する");
    assert!(
        error.contains(expected),
        "期待 {expected:?} / 実際 {error:?}"
    );
    // エラー経路では計算結果を比べているのではなく、入力CPが一切書き換わって
    // いないという原子性を構造と値の完全一致で確認する。
    assert_eq!(candidate, before, "拒否時は展開図を変更しない");
}

fn square_document() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

/// 鳥の基本形の予備基本形へ進む途中にある1/2/4層packet。
fn bird_base_packet_after_folds(layers: usize) -> Document {
    let mut document = square_document();
    match layers {
        1 => {}
        2 => fold_for_sample(&mut document, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]),
        4 => {
            fold_for_sample(&mut document, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]);
            fold_for_sample(&mut document, [[0.5, 0.0], [0.5, 0.5]], [0.25, 0.25]);
        }
        other => panic!("鳥の基本形のpacketで使う層数は1/2/4、実際 {other}"),
    }
    document
}

/// 鳥の基本形の4層packetで、開いた先端側の表1枚を対角線で開く工程。
/// この対角線は表層の接続辺を裂かず、既存の `fold_through`
/// 受入れ検査でも警告0と固定済み。折り重ねた先の三角形が正面積の実5層になる。
fn bird_base_five_layer_stage_document() -> Document {
    let mut document = square_document();
    fold_for_sample(&mut document, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]);
    fold_for_sample(&mut document, [[0.5, 0.5], [1.0, 0.5]], [0.75, 0.25]);
    let (_, state) = state_of(&document);
    let top = *state.order.last().expect("4層packetの表層");
    fold_selected_for_sample(
        &mut document,
        [[0.0, 0.5], [0.5, 0.0]],
        [0.4, 0.4],
        vec![top],
    );
    document
}

fn fold_selected_for_sample(
    document: &mut Document,
    line: [[f64; 2]; 2],
    keep: [f64; 2],
    target_layers: Vec<FaceId>,
) {
    let (faces, state) = state_of(document);
    let mut cp = document.cp.clone();
    let result = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line,
            keep_side_point: keep,
            target_layers: Some(target_layers),
            direction: FoldDirection::Up,
        },
    )
    .expect("鳥の基本形の表層を対角線で開ける");
    assert!(
        result.warnings.is_empty(),
        "鳥の基本形の表層は裂けずに開ける: {:?}",
        result.warnings
    );
    let mut step = result.step;
    step.id = u32::try_from(document.sequence.len()).expect("手順数はu32に収まる");
    document.cp = cp;
    document.sequence.push(step);
}

fn fold_for_sample(document: &mut Document, line: [[f64; 2]; 2], keep: [f64; 2]) {
    let (faces, state) = state_of(document);
    let mut cp = document.cp.clone();
    let result = fold_through(
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
    .expect("鳥の基本形の下ごしらえは折れる");
    assert!(
        result.warnings.is_empty(),
        "鳥の基本形の下ごしらえに警告なし: {:?}",
        result.warnings
    );
    let mut step = result.step;
    step.id = u32::try_from(document.sequence.len()).expect("手順数はu32に収まる");
    document.cp = cp;
    document.sequence.push(step);
}

fn state_of(document: &Document) -> (Vec<Face>, FlatState) {
    let faces = extract_faces(&document.cp);
    let (state, warnings) =
        flat_state_at(document, &faces, document.sequence.len()).expect("標本は平らに畳める");
    assert!(
        warnings.is_empty(),
        "標本の平坦再生に警告なし: {warnings:?}"
    );
    (faces, state)
}

fn folded_bounds(cp: &CreasePattern, faces: &[Face], state: &FlatState) -> (DVec2, DVec2) {
    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<std::collections::HashMap<_, _>>();
    let mut lo = DVec2::splat(f64::INFINITY);
    let mut hi = DVec2::splat(f64::NEG_INFINITY);
    for face in faces {
        let placement = state.placements[&face.id];
        for point in face.vertices.iter().filter_map(|id| positions.get(id)) {
            let folded = placement.apply(*point);
            lo = lo.min(folded);
            hi = hi.max(folded);
        }
    }
    assert!(
        (hi.x - lo.x).is_finite()
            && (hi.y - lo.y).is_finite()
            && hi.x - lo.x > RESULT_EPS
            && hi.y - lo.y > RESULT_EPS,
        "標本のfootprintは有限で正の面積"
    );
    (lo, hi)
}

fn public_function_names(source: &str) -> BTreeSet<String> {
    let code = rust_code_only(source);
    code.match_indices("pub fn ")
        .filter_map(|(start, _)| {
            let declaration = &code[start + "pub fn ".len()..];
            let end = declaration
                .bytes()
                .position(|byte| !(byte.is_ascii_alphanumeric() || byte == b'_'))?;
            (declaration[end..]
                .chars()
                .find(|character| !character.is_whitespace())
                == Some('('))
            .then(|| declaration[..end].to_string())
        })
        .collect()
}

fn call_count(code: &str, name: &str) -> usize {
    code.match_indices(name)
        .filter(|(start, _)| {
            let before_is_identifier = code.as_bytes()[..*start]
                .last()
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
            let after = &code[*start + name.len()..];
            let followed_by_call =
                after.chars().find(|character| !character.is_whitespace()) == Some('(');
            !before_is_identifier && followed_by_call
        })
        .count()
}

fn function_code(source: &str, name: &str) -> String {
    let code = rust_code_only(source);
    let needle = format!("fn {name}");
    let signature = code
        .match_indices(&needle)
        .map(|(start, _)| start)
        .find(|start| {
            code[start + needle.len()..]
                .chars()
                .find(|character| !character.is_whitespace())
                == Some('(')
        })
        .unwrap_or_else(|| panic!("構造検査の関数が無い: {name}"));
    let open = code[signature..]
        .find('{')
        .map(|offset| signature + offset)
        .unwrap_or_else(|| panic!("構造検査の関数本体が無い: {name}"));
    let mut depth = 0usize;
    for (offset, byte) in code.as_bytes()[open..].iter().copied().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return code[open..=open + offset].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("構造検査の関数本体が閉じていない: {name}")
}

/// コメントと文字列を空白にしてから構造を読む。呼出し名がコメントに残っただけで
/// 経路検査が通ることを防ぎ、改行位置や関数の並べ替えには依存しない。
fn rust_code_only(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut code = vec![b' '; bytes.len()];
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"//") {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if bytes[index..].starts_with(b"/*") {
            index += 2;
            let mut depth = 1usize;
            while index < bytes.len() && depth > 0 {
                if bytes[index..].starts_with(b"/*") {
                    depth += 1;
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            continue;
        }
        if bytes[index] == b'"' {
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index = (index + 2).min(bytes.len());
                } else if bytes[index] == b'"' {
                    index += 1;
                    break;
                } else {
                    index += 1;
                }
            }
            continue;
        }
        code[index] = bytes[index];
        index += 1;
    }
    String::from_utf8(code).expect("Rustソースのコード抽出はUTF-8")
}
