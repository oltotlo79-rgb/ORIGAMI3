use ori3_app_core::{Ori3AppCore, ProposalJobId, TIME_FREE_PROPOSAL_PLAN_BUDGET};
use ori3_model::Paper;
use ori3_propose::Skeleton;

#[derive(serde::Deserialize)]
struct ProposalCorpus {
    paper: Paper,
    skeleton: Skeleton,
    seed: u64,
    with_fold_plan: bool,
}

fn bird_base_corpus() -> ProposalCorpus {
    serde_json::from_str(include_str!(
        "../../ori3-propose/tests/fixtures/corpus/bird-base.json"
    ))
    .expect("鳥の基本形corpusを読める")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
fn bird_base_product_json_contract() {
    let corpus = bird_base_corpus();
    let job_id: ProposalJobId =
        serde_json::from_value(serde_json::Value::String("bird-base-product".to_owned()))
            .expect("不透明なjob IDを同じwire形から作れる");
    // 壁時計の打ち切り(製品30,000ms)は、debug構成では探索が16.8〜20.5倍遅いため
    // 機械の混み具合だけで当たる(規約03 §10.6)。この契約が見たいのは候補とJSONの形で
    // あって探索の速さではないので、検査からだけ打ち切りなしの予算を渡す。
    // 探索の中身も期待値も製品と同じである。
    let result = Ori3AppCore::new()
        .proposal_generate_with_budget(
            job_id,
            corpus.skeleton,
            corpus.paper,
            corpus.seed,
            corpus.with_fold_plan,
            TIME_FREE_PROPOSAL_PLAN_BUDGET,
        )
        .expect("鳥の基本形の候補と折り方を生成できる");
    assert_eq!(result.candidates.len(), 4);
    assert!(
        corpus.with_fold_plan,
        "この契約は折り方を求める製品経路を対象にする"
    );
    for (index, candidate) in result.candidates.iter().enumerate() {
        assert!(
            candidate.fold_plan.is_some(),
            "候補 {index} に折り方がありません"
        );
        let candidate_json =
            serde_json::to_value(candidate).expect("候補をJSON値へ直列化できる");
        let steps = candidate_json
            .get("fold_plan")
            .and_then(serde_json::Value::as_object)
            .and_then(|plan| plan.get("steps"))
            .and_then(serde_json::Value::as_array)
            .expect("候補の折り方に手順配列がありません");
        assert!(
            !steps.is_empty(),
            "候補 {index} の折り方に手順がありません"
        );
    }
    let json = serde_json::to_string(&result).expect("候補をJSONへ直列化できる");
    // 2026-09-07: 旧 length=32_344 / FNV=5036_9e78_f6bd_bfa4 →
    // 新 length=31_748 / FNV=60e6_f600_438f_d2c4。
    // 解析交差が旧9点標本の見落としを補い、候補0/1の新2手目で
    // 辺17・面(8,9,18)の連続条件（交差幅約0.012616）が上下(8,18)を確定した。
    // 旧手の不正による更新ではなく、警告なしで検証可能になった手の採用による更新。
    // 旧/新の4候補全8手はcheck_moveの21姿勢検査を通り、終点はSAT・違反0。
    assert_eq!(json.len(), 31_748);
    assert_eq!(fnv1a64(json.as_bytes()), 0x60e6_f600_438f_d2c4);
}
