#[path = "support/fold_external_corpus.rs"]
mod fold_external_corpus;

use fold_external_corpus::{ManifestSummary, sha256_hex, validate_manifest};
use ori3_export::fold::{fold_to_document, parse_fold_1_2};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const PLAN: &str = include_str!("fixtures/fold/external-corpus-plan.json");
const TRACKED_MANIFEST: &str = include_str!("fixtures/fold/corpus/manifest.json");
const SOURCE_QUOTAS: [(&str, usize); 4] = [
    ("official", 6),
    ("oripa", 8),
    ("oriedita", 8),
    ("origami_simulator", 8),
];
const ACCEPTED_TRANCHE: [(&str, usize); 2] = [("oriedita", 8), ("origami_simulator", 8)];

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn tracked_corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fold/corpus")
}

fn tracked_manifest() -> Value {
    serde_json::from_str(TRACKED_MANIFEST).expect("追跡corpus manifestはvalid JSON")
}

struct SyntheticCorpus {
    root: PathBuf,
    manifest_path: PathBuf,
    manifest: Value,
}

impl SyntheticCorpus {
    fn write_manifest(&self) {
        let bytes = serde_json::to_vec_pretty(&self.manifest).expect("synthetic manifestをJSON化");
        fs::write(&self.manifest_path, bytes).expect("temp synthetic manifestを書ける");
    }

    fn entries_mut(&mut self) -> &mut Vec<Value> {
        self.manifest["entries"]
            .as_array_mut()
            .expect("synthetic entriesはarray")
    }

    fn entry_path(&self, index: usize) -> PathBuf {
        self.root.join(
            self.manifest["entries"][index]["path"]
                .as_str()
                .expect("synthetic pathはstring"),
        )
    }
}

impl Drop for SyntheticCorpus {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn synthetic_corpus() -> SyntheticCorpus {
    let unique = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("ori3-fold-corpus-{}-{unique}", std::process::id()));
    fs::create_dir(&root).expect("専用temp corpus rootを作れる");

    let mut entries = Vec::new();
    let mut global_index = 0_usize;
    for (source, quota) in ACCEPTED_TRANCHE {
        let source_dir_name = if source == "origami_simulator" {
            "origami-simulator"
        } else {
            source
        };
        let source_dir = root.join(source_dir_name);
        fs::create_dir_all(&source_dir).expect("temp source directoryを作れる");
        for source_index in 1..=quota {
            global_index += 1;
            let id_prefix = if source == "origami_simulator" {
                "origami-simulator"
            } else {
                source
            };
            let id = format!("{id_prefix}-{source_index:02}");
            let relative_path = format!("{source_dir_name}/{id}.fold");
            let raw = format!("{{\"file_spec\":1.2,\"synthetic_fixture\":{global_index}}}\n")
                .into_bytes();
            fs::write(root.join(&relative_path), &raw).expect("temp raw fixtureを書ける");
            let expected = if source == "oriedita" {
                "supported"
            } else {
                "unsupported"
            };
            let observed = if source == "oriedita" && source_index <= 7 {
                "supported"
            } else {
                "unsupported"
            };

            entries.push(json!({
                "id": id,
                "source": source,
                "generator": format!("synthetic-{source}"),
                "generator_version": "test-1.0",
                "source_uri": format!("https://example.invalid/fold/{id}"),
                "source_file_last_write_utc": "2026-08-26T00:00:00Z",
                "path": relative_path,
                "byte_length": raw.len() as u64,
                "sha256": sha256_hex(&raw),
                "classification": {
                    "expected": expected,
                    "frozen_at_utc": "2026-08-26T00:10:00Z",
                    "basis": "Synthetic validator contract input; not an external-corpus decision.",
                    "unsupported_paths": if expected == "unsupported" {
                        vec!["$.synthetic_unsupported_field"]
                    } else {
                        Vec::<&str>::new()
                    }
                },
                "observed": {
                    "result": observed,
                    "observed_at_utc": "2026-08-26T00:05:00Z",
                    "method": "Synthetic prior diagnostic kept separate from frozen expectation.",
                    "excluded_from_frozen_classification": true,
                    "warnings": [],
                    "errors": if observed == "unsupported" {
                        vec![json!({
                            "code": "synthetic_unsupported",
                            "path": "$.synthetic_unsupported_field"
                        })]
                    } else {
                        Vec::<Value>::new()
                    }
                },
                "rights": {
                    "content_spdx": "LicenseRef-Synthetic-Test-Only",
                    "content_evidence": "Generated inside this test and not an external rights claim.",
                    "rights_holder": "synthetic-validator-test",
                    "authorization_date": "2026-08-26",
                    "authorization_scope": "temporary test files only",
                    "generator_license_used_for_content": false,
                    "reviewer": "synthetic-validator-test",
                    "reviewed_on": "2026-08-26",
                    "redistribution_allowed": true
                }
            }));
        }
    }

    let manifest = json!({
        "schema_version": 2,
        "classification_policy": {
            "frozen": true,
            "frozen_at_utc": "2026-08-26T00:10:00Z",
            "independent_auditor": "synthetic-validator-test",
            "auditor_had_runtime_results": false,
            "formal_acceptance_runs_after_freeze": true,
            "rules": ["synthetic non-empty rule"]
        },
        "entries": entries
    });
    let manifest_path = root.join("manifest.json");
    let corpus = SyntheticCorpus {
        root,
        manifest_path,
        manifest,
    };
    corpus.write_manifest();
    corpus
}

fn rejection(corpus: &SyntheticCorpus) -> String {
    validate_manifest(&corpus.root, &corpus.manifest_path)
        .expect_err("invalid synthetic manifestは拒否される")
        .to_string()
}

fn assert_rejected_with(corpus: &SyntheticCorpus, expected: &str) {
    let message = rejection(corpus);
    assert!(
        message.contains(expected),
        "errorに {expected:?} が必要: {message}"
    );
}

#[cfg(unix)]
fn try_symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn try_symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[test]
fn tracked_plan_reserves_exact_quotas_without_preclassifying_entries() {
    let plan: Value = serde_json::from_str(PLAN).expect("予約計画はvalid JSON");
    assert_eq!(plan["schema_version"], 1);
    assert_eq!(plan["status"], "reservations_with_partial_acceptance");

    let expected_sources = BTreeMap::from(SOURCE_QUOTAS);
    let recorded_sources = plan["source_quotas"]
        .as_object()
        .expect("source_quotasはobject")
        .iter()
        .map(|(source, quota)| {
            (
                source.as_str(),
                quota.as_u64().expect("source quotaは非負整数") as usize,
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(recorded_sources, expected_sources);

    assert!(
        plan.get("target_class_quotas").is_none(),
        "対応/非対応の件数は結果を見た後に目標値へ合わせず、manifestで実数を報告する"
    );

    let slots = plan["slots"].as_array().expect("slotsはarray");
    assert_eq!(slots.len(), 30, "予約枠は過不足なく30件");
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    let mut counts = BTreeMap::<&str, usize>::new();
    for slot in slots {
        let slot = slot.as_object().expect("slotはobject");
        let id = slot["id"].as_str().expect("予約idはstring");
        let source = slot["source"].as_str().expect("予約sourceはstring");
        let path = slot["reserved_path"]
            .as_str()
            .expect("reserved_pathはstring");
        assert!(ids.insert(id), "予約idは一意: {id}");
        assert!(paths.insert(path), "予約pathは一意: {path}");
        *counts.entry(source).or_default() += 1;

        let reserved_source = if id.starts_with("official-") {
            "official"
        } else if id.starts_with("oripa-") {
            "oripa"
        } else if id.starts_with("oriedita-") {
            "oriedita"
        } else if id.starts_with("origami-simulator-") {
            "origami_simulator"
        } else {
            panic!("予約外のid: {id}");
        };
        assert_eq!(source, reserved_source, "予約idとsourceを固定する: {id}");
        assert_eq!(
            path,
            format!("external/{reserved_source}/{id}.fold"),
            "raw到着前に固定した予約pathを変えない: {id}"
        );

        for unresolved_field in [
            "classification",
            "observed",
            "target_class",
            "sha256",
            "byte_length",
            "rights",
        ] {
            assert!(
                !slot.contains_key(unresolved_field),
                "raw到着前のslotに {unresolved_field} を捏造しない: {id}"
            );
        }
    }
    assert_eq!(counts, expected_sources);
}

#[test]
fn tracked_sixteen_sample_tranche_reports_frozen_and_observed_counts_separately() {
    let root = tracked_corpus_root();
    let summary = validate_manifest(&root, &root.join("manifest.json"))
        .expect("追跡16件のprovenance・rights・SHA-256・byte数は完全");
    assert_eq!(
        summary,
        ManifestSummary {
            entries: 16,
            official: 0,
            oripa: 0,
            oriedita: 8,
            origami_simulator: 8,
            expected_supported: 8,
            expected_unsupported: 8,
            observed_supported: 7,
            observed_unsupported: 9,
        }
    );

    let plan: Value = serde_json::from_str(PLAN).expect("予約計画はvalid JSON");
    let reserved = plan["slots"]
        .as_array()
        .expect("plan slotsはarray")
        .iter()
        .map(|slot| {
            let id = slot["id"].as_str().expect("予約idはstring");
            let source = slot["source"].as_str().expect("予約sourceはstring");
            let path = slot["reserved_path"]
                .as_str()
                .expect("予約pathはstring");
            (id, (source, path))
        })
        .collect::<BTreeMap<_, _>>();
    for entry in tracked_manifest()["entries"]
        .as_array()
        .expect("tracked entriesはarray")
    {
        let id = entry["id"].as_str().expect("accepted idはstring");
        let source = entry["source"].as_str().expect("accepted sourceはstring");
        let path = entry["path"].as_str().expect("accepted pathはstring");
        assert_eq!(
            reserved.get(id),
            Some(&(source, path)),
            "受入sampleのid・source・pathはraw到着前に予約済み: {id}"
        );
    }
}

#[test]
fn tracked_external_samples_match_blind_frozen_expectations() {
    let root = tracked_corpus_root();
    let manifest = tracked_manifest();
    let mut mismatches = Vec::new();

    for entry in manifest["entries"]
        .as_array()
        .expect("tracked entriesはarray")
    {
        let id = entry["id"].as_str().expect("entry idはstring");
        let relative_path = entry["path"].as_str().expect("entry pathはstring");
        let expected = entry["classification"]["expected"]
            .as_str()
            .expect("frozen expectedはstring");
        let raw = fs::read_to_string(root.join(relative_path))
            .unwrap_or_else(|error| panic!("追跡raw {id}を読める: {error}"));

        let frozen_unsupported_paths = entry["classification"]["unsupported_paths"]
            .as_array()
            .expect("frozen unsupported_pathsはarray")
            .iter()
            .map(|path| path.as_str().expect("frozen unsupported pathはstring"))
            .collect::<BTreeSet<_>>();
        let observed_warning_keys = entry["observed"]["warnings"]
            .as_array()
            .expect("observed warningsはarray")
            .iter()
            .map(|issue| {
                (
                    issue["code"]
                        .as_str()
                        .expect("observed warning codeはstring")
                        .to_string(),
                    issue["path"]
                        .as_str()
                        .expect("observed warning pathはstring")
                        .to_string(),
                )
            })
            .collect::<BTreeSet<_>>();

        let (actual, detail) = match parse_fold_1_2(&raw) {
            Err(error) => {
                let reason_matches = error.path != "$.file_spec"
                    && frozen_unsupported_paths.contains(error.path.as_str());
                (
                    "unsupported",
                    format!(
                        "parse {:?} @ {}: {}; frozen_reason_matches={reason_matches}",
                        error.kind, error.path, error.message
                    ),
                )
            }
            Ok(file) => match fold_to_document(&file) {
                Ok(imported) => {
                    let actual_warning_keys = imported
                        .warnings
                        .iter()
                        .map(|issue| {
                            (
                                serde_json::to_value(issue.code)
                                    .expect("warning codeをserialize")
                                    .as_str()
                                    .expect("serialized warning codeはstring")
                                    .to_string(),
                                issue.path.clone(),
                            )
                        })
                        .collect::<BTreeSet<_>>();
                    let warnings_match = actual_warning_keys == observed_warning_keys;
                    (
                        if warnings_match {
                            "supported"
                        } else {
                            "supported_with_warning_mismatch"
                        },
                        format!(
                            "parse/validation/conversion succeeded; warnings_match={warnings_match}; expected={observed_warning_keys:?}; actual={actual_warning_keys:?}"
                        ),
                    )
                }
                Err(error) => {
                    let actual_error_paths = error
                        .errors
                        .iter()
                        .map(|issue| issue.path.as_str())
                        .collect::<BTreeSet<_>>();
                    let reason_matches = !actual_error_paths.contains("$.file_spec")
                        && frozen_unsupported_paths
                            .iter()
                            .all(|path| actual_error_paths.contains(path));
                    let details = error
                        .errors
                        .iter()
                        .map(|issue| {
                            format!("{:?} @ {}: {}", issue.code, issue.path, issue.message)
                        })
                        .collect::<Vec<_>>()
                        .join(" | ");
                    (
                        if reason_matches {
                            "unsupported"
                        } else {
                            "unsupported_with_reason_mismatch"
                        },
                        format!("{details}; frozen_reason_matches={reason_matches}"),
                    )
                }
            },
        };
        if actual != expected {
            mismatches.push(format!(
                "{id}: frozen expected={expected}, actual={actual}; {detail}"
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "独立raw規則で凍結した期待を、実行後に変更しない:\n{}",
        mismatches.join("\n")
    );
}

#[test]
fn pure_rust_sha256_matches_nist_empty_and_abc_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn complete_temp_manifest_is_accepted_without_rewriting_inputs() {
    let corpus = synthetic_corpus();
    let manifest_before = fs::read(&corpus.manifest_path).expect("manifestをsnapshot");
    let raw_before = corpus.manifest["entries"]
        .as_array()
        .expect("entriesはarray")
        .iter()
        .map(|entry| {
            let path = corpus
                .root
                .join(entry["path"].as_str().expect("pathはstring"));
            (path.clone(), fs::read(path).expect("raw fixtureをsnapshot"))
        })
        .collect::<Vec<_>>();

    let summary = validate_manifest(&corpus.root, &corpus.manifest_path)
        .expect("complete synthetic corpusはvalid");
    assert_eq!(
        summary,
        ManifestSummary {
            entries: 16,
            official: 0,
            oripa: 0,
            oriedita: 8,
            origami_simulator: 8,
            expected_supported: 8,
            expected_unsupported: 8,
            observed_supported: 7,
            observed_unsupported: 9,
        }
    );

    assert_eq!(
        fs::read(&corpus.manifest_path).expect("manifestを再読込"),
        manifest_before,
        "validatorはmanifestを書き換えない"
    );
    for (path, before) in raw_before {
        assert_eq!(
            fs::read(&path).expect("raw fixtureを再読込"),
            before,
            "validatorはraw fixtureを書き換えない: {}",
            path.display()
        );
    }
}

#[test]
fn paths_must_be_relative_parent_free_regular_files() {
    let mut absolute = synthetic_corpus();
    let absolute_file = absolute.root.join("absolute.fold");
    fs::write(&absolute_file, b"absolute").expect("temp absolute targetを書ける");
    absolute.entries_mut()[0]["path"] = json!(absolute_file.to_string_lossy());
    absolute.write_manifest();
    assert_rejected_with(&absolute, "relative");

    let mut parent = synthetic_corpus();
    parent.entries_mut()[0]["path"] = json!("raw/../escape.fold");
    parent.write_manifest();
    assert_rejected_with(&parent, "parent");

    let missing = synthetic_corpus();
    fs::remove_file(missing.entry_path(0)).expect("reserved temp fixtureを除ける");
    assert_rejected_with(&missing, "metadata");

    let directory = synthetic_corpus();
    let directory_path = directory.entry_path(0);
    fs::remove_file(&directory_path).expect("reserved temp fixtureを除ける");
    fs::create_dir(&directory_path).expect("reserved pathへdirectoryを作れる");
    assert_rejected_with(&directory, "regular file");
}

#[test]
fn entries_must_keep_the_reserved_id_source_and_fold_path() {
    let mut id = synthetic_corpus();
    id.entries_mut()[0]["id"] = json!("official-07");
    id.write_manifest();
    assert_rejected_with(&id, "reserved slot");

    let mut source = synthetic_corpus();
    source.entries_mut()[0]["source"] = json!("oripa");
    source.write_manifest();
    assert_rejected_with(&source, "reserved source");

    let mut path = synthetic_corpus();
    path.entries_mut()[0]["path"] = json!("raw/official/renamed.json");
    path.write_manifest();
    assert_rejected_with(&path, "reserved path");
}

#[test]
fn symlinks_are_rejected_when_the_platform_can_create_one() {
    let corpus = synthetic_corpus();
    let link = corpus.entry_path(0);
    let target = corpus.root.join("symlink-target.fold");
    fs::write(&target, b"symlink target").expect("temp symlink targetを書ける");
    fs::remove_file(&link).expect("reserved temp fixtureを除ける");
    match try_symlink_file(&target, &link) {
        Ok(()) => {
            assert_rejected_with(&corpus, "symlink");
        }
        Err(error) => {
            eprintln!(
                "symlink creation is unavailable on this host; validator branch remains active: {error}"
            );
        }
    }
}

#[test]
fn partial_tranches_report_actual_counts_without_a_fixed_classification_quota() {
    let mut source = synthetic_corpus();
    source.entries_mut()[6]["source"] = json!("official");
    source.write_manifest();
    assert_rejected_with(&source, "reserved source");

    let mut partial = synthetic_corpus();
    let removed_path = partial.entry_path(15);
    partial.entries_mut().pop();
    fs::remove_file(removed_path).expect("removed manifest entryのrawもtempから除ける");
    partial.write_manifest();
    let summary = validate_manifest(&partial.root, &partial.manifest_path)
        .expect("予約30件が未完成でも、受入済みtrancheの実数を検査できる");
    assert_eq!(summary.entries, 15);
    assert_eq!(summary.origami_simulator, 7);
    assert_eq!(summary.expected_supported, 8);
    assert_eq!(summary.expected_unsupported, 7);
    assert_eq!(summary.observed_supported, 7);
    assert_eq!(summary.observed_unsupported, 8);
}

#[test]
fn ids_paths_and_hashes_must_each_be_unique() {
    let mut id = synthetic_corpus();
    let duplicate_id = id.manifest["entries"][0]["id"].clone();
    id.entries_mut()[1]["id"] = duplicate_id;
    id.write_manifest();
    assert_rejected_with(&id, "duplicate id");

    let mut path = synthetic_corpus();
    let duplicate_path = path.manifest["entries"][0]["path"].clone();
    path.entries_mut()[1]["path"] = duplicate_path;
    path.write_manifest();
    assert_rejected_with(&path, "duplicate path");

    let mut hash = synthetic_corpus();
    let duplicate_hash = hash.manifest["entries"][0]["sha256"].clone();
    hash.entries_mut()[1]["sha256"] = duplicate_hash;
    hash.write_manifest();
    assert_rejected_with(&hash, "duplicate sha256");
}

#[test]
fn raw_byte_lengths_and_sha256_are_checked_without_normalization() {
    let corpus = synthetic_corpus();
    let path = corpus.entry_path(0);
    let mut bytes = fs::read(&path).expect("raw bytesを読める");
    bytes[0] ^= 1;
    fs::write(&path, bytes).expect("temp raw bytesだけを変えられる");
    assert_rejected_with(&corpus, "sha256 mismatch");

    let mut length = synthetic_corpus();
    let recorded = length.manifest["entries"][0]["byte_length"]
        .as_u64()
        .expect("byte_lengthはu64");
    length.entries_mut()[0]["byte_length"] = json!(recorded + 1);
    length.write_manifest();
    assert_rejected_with(&length, "byte_length mismatch");

    let mut format = synthetic_corpus();
    let uppercase = format.manifest["entries"][0]["sha256"]
        .as_str()
        .expect("sha256はstring")
        .to_ascii_uppercase();
    format.entries_mut()[0]["sha256"] = json!(uppercase);
    format.write_manifest();
    assert_rejected_with(&format, "lowercase hexadecimal");
}

#[test]
fn rights_must_be_complete_resolved_and_redistributable() {
    for field in [
        "content_spdx",
        "content_evidence",
        "rights_holder",
        "authorization_date",
        "authorization_scope",
        "generator_license_used_for_content",
        "reviewer",
        "reviewed_on",
        "redistribution_allowed",
    ] {
        let mut corpus = synthetic_corpus();
        corpus.manifest["entries"][0]["rights"]
            .as_object_mut()
            .expect("rightsはobject")
            .remove(field);
        corpus.write_manifest();
        assert_rejected_with(&corpus, field);
    }

    let mut unresolved = synthetic_corpus();
    unresolved.entries_mut()[0]["rights"]["content_spdx"] = json!("NOASSERTION");
    unresolved.write_manifest();
    assert_rejected_with(&unresolved, "NOASSERTION");

    let mut not_license_ref = synthetic_corpus();
    not_license_ref.entries_mut()[0]["rights"]["content_spdx"] = json!("CC0-1.0");
    not_license_ref.write_manifest();
    assert_rejected_with(&not_license_ref, "LicenseRef");

    let mut unapproved_license_ref = synthetic_corpus();
    unapproved_license_ref.entries_mut()[0]["rights"]["content_spdx"] =
        json!("LicenseRef-Not-The-Approved-Synthetic-License");
    unapproved_license_ref.write_manifest();
    assert_rejected_with(&unapproved_license_ref, "approved LicenseRef");

    let mut generator_claim = synthetic_corpus();
    generator_claim.entries_mut()[0]["rights"]["generator_license_used_for_content"] = json!(true);
    generator_claim.write_manifest();
    assert_rejected_with(&generator_claim, "generator_license_used_for_content");

    let mut forbidden = synthetic_corpus();
    forbidden.entries_mut()[0]["rights"]["redistribution_allowed"] = json!(false);
    forbidden.write_manifest();
    assert_rejected_with(&forbidden, "redistribution_allowed");
}

#[test]
fn manifest_must_be_the_regular_non_symlink_file_at_the_corpus_root() {
    let corpus = synthetic_corpus();
    let alternate_manifest = corpus.root.join("alternate-manifest.json");
    fs::copy(&corpus.manifest_path, &alternate_manifest)
        .expect("alternate manifestをtemp rootへcopyできる");
    let error = validate_manifest(&corpus.root, &alternate_manifest)
        .expect_err("root直下の正規manifest以外は拒否される")
        .to_string();
    assert!(
        error.contains("manifest path"),
        "manifest位置の拒否理由が必要: {error}"
    );
}

#[test]
fn classifications_are_blindly_frozen_and_observations_stay_separate() {
    let mut not_frozen = synthetic_corpus();
    not_frozen.manifest["classification_policy"]["frozen"] = json!(false);
    not_frozen.write_manifest();
    assert_rejected_with(&not_frozen, "classification_policy.frozen");

    let mut saw_results = synthetic_corpus();
    saw_results.manifest["classification_policy"]["auditor_had_runtime_results"] = json!(true);
    saw_results.write_manifest();
    assert_rejected_with(&saw_results, "auditor_had_runtime_results");

    let mut missing = synthetic_corpus();
    missing.entries_mut()[0]
        .as_object_mut()
        .expect("entryはobject")
        .remove("classification");
    missing.write_manifest();
    assert_rejected_with(&missing, "classification");

    let mut unknown = synthetic_corpus();
    unknown.entries_mut()[0]["classification"]["expected"] = json!("pending");
    unknown.write_manifest();
    assert_rejected_with(&unknown, "supported or unsupported");

    let mut supported_with_unsupported_path = synthetic_corpus();
    supported_with_unsupported_path.entries_mut()[0]["classification"]["unsupported_paths"] =
        json!(["$.unexpected"]);
    supported_with_unsupported_path.write_manifest();
    assert_rejected_with(&supported_with_unsupported_path, "supported entry");

    let mut non_json_path = synthetic_corpus();
    non_json_path.entries_mut()[8]["classification"]["unsupported_paths"] =
        json!(["/not-a-json-path"]);
    non_json_path.write_manifest();
    assert_rejected_with(&non_json_path, "JSON path");

    let mut changed_freeze = synthetic_corpus();
    changed_freeze.entries_mut()[0]["classification"]["frozen_at_utc"] =
        json!("2026-08-26T00:11:00Z");
    changed_freeze.write_manifest();
    assert_rejected_with(&changed_freeze, "must match classification_policy");

    let mismatch = synthetic_corpus();
    let summary = validate_manifest(&mismatch.root, &mismatch.manifest_path)
        .expect("frozen expectedとexcluded prior observationは一致を強制しない");
    assert_eq!(summary.expected_supported, 8);
    assert_eq!(summary.observed_supported, 7);
}

#[test]
fn prior_observations_require_explicit_exclusion_and_machine_readable_issues() {
    let mut missing = synthetic_corpus();
    missing.entries_mut()[0]
        .as_object_mut()
        .expect("entryはobject")
        .remove("observed");
    missing.write_manifest();
    assert_rejected_with(&missing, "observed");

    let mut not_excluded = synthetic_corpus();
    not_excluded.entries_mut()[0]["observed"]["excluded_from_frozen_classification"] = json!(false);
    not_excluded.write_manifest();
    assert_rejected_with(&not_excluded, "excluded_from_frozen_classification");

    let mut unsupported_without_error = synthetic_corpus();
    unsupported_without_error.entries_mut()[8]["observed"]["errors"] = json!([]);
    unsupported_without_error.write_manifest();
    assert_rejected_with(&unsupported_without_error, "requires at least one error");

    let mut invalid_path = synthetic_corpus();
    invalid_path.entries_mut()[8]["observed"]["errors"][0]["path"] = json!("/not-json-path");
    invalid_path.write_manifest();
    assert_rejected_with(&invalid_path, "must be a JSON path");

    let mut freeform_message = synthetic_corpus();
    freeform_message.entries_mut()[8]["observed"]["errors"][0]["message"] =
        json!("do not preserve runtime prose in corpus evidence");
    freeform_message.write_manifest();
    assert_rejected_with(
        &freeform_message,
        "only code, path, and optional numeric value",
    );
}

#[test]
fn provenance_and_classification_evidence_are_required() {
    for field in [
        "generator",
        "generator_version",
        "source_uri",
        "source_file_last_write_utc",
    ] {
        let mut corpus = synthetic_corpus();
        corpus.entries_mut()[0]
            .as_object_mut()
            .expect("entryはobject")
            .remove(field);
        corpus.write_manifest();
        assert_rejected_with(&corpus, field);
    }

    for field in ["expected", "frozen_at_utc", "basis", "unsupported_paths"] {
        let mut corpus = synthetic_corpus();
        corpus.entries_mut()[8]["classification"]
            .as_object_mut()
            .expect("classificationはobject")
            .remove(field);
        corpus.write_manifest();
        assert_rejected_with(&corpus, field);
    }

    let mut unsupported_without_path = synthetic_corpus();
    unsupported_without_path.entries_mut()[8]["classification"]["unsupported_paths"] = json!([]);
    unsupported_without_path.write_manifest();
    assert_rejected_with(&unsupported_without_path, "at least one unsupported path");
}
