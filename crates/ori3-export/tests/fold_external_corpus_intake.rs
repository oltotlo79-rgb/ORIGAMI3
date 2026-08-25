mod support;

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use support::fold_external_corpus::{ManifestSummary, sha256_hex, validate_manifest};

const PLAN: &str = include_str!("fixtures/fold/external-corpus-plan.json");
const SOURCE_QUOTAS: [(&str, usize); 4] = [
    ("official", 6),
    ("oripa", 8),
    ("oriedita", 8),
    ("origami_simulator", 8),
];

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

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
    let root = std::env::temp_dir().join(format!(
        "ori3-fold-corpus-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).expect("専用temp corpus rootを作れる");

    let mut entries = Vec::new();
    let mut global_index = 0_usize;
    for (source, quota) in SOURCE_QUOTAS {
        let source_dir = root.join("raw").join(source);
        fs::create_dir_all(&source_dir).expect("temp source directoryを作れる");
        for source_index in 1..=quota {
            global_index += 1;
            let id = format!("{source}-{source_index:02}");
            let relative_path = format!("raw/{source}/{id}.fold");
            let raw = format!(
                "{{\"file_spec\":1.2,\"synthetic_fixture\":{global_index}}}\n"
            )
            .into_bytes();
            fs::write(root.join(&relative_path), &raw).expect("temp raw fixtureを書ける");

            entries.push(json!({
                "id": id,
                "source": source,
                "path": relative_path,
                "byte_length": raw.len() as u64,
                "sha256": sha256_hex(&raw),
                "classification": if global_index <= 20 { "supported" } else { "unsupported" },
                "rights": {
                    "source_url": format!("https://example.invalid/fold/{id}"),
                    "license_spdx": "CC0-1.0",
                    "license_url": "https://creativecommons.org/publicdomain/zero/1.0/",
                    "attribution": format!("Synthetic test fixture {id}"),
                    "redistribution_allowed": true
                }
            }));
        }
    }

    let manifest = json!({
        "schema_version": 1,
        "classification_frozen": true,
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
    assert_eq!(plan["status"], "reservations_only");

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

    let expected_classes = BTreeMap::from([("supported", 20_usize), ("unsupported", 10)]);
    let recorded_classes = plan["target_class_quotas"]
        .as_object()
        .expect("target_class_quotasはobject")
        .iter()
        .map(|(class, quota)| {
            (
                class.as_str(),
                quota.as_u64().expect("class quotaは非負整数") as usize,
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(recorded_classes, expected_classes);

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

        for unresolved_field in [
            "classification",
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
    let raw_before = corpus
        .manifest["entries"]
        .as_array()
        .expect("entriesはarray")
        .iter()
        .map(|entry| {
            let path = corpus.root.join(entry["path"].as_str().expect("pathはstring"));
            (path.clone(), fs::read(path).expect("raw fixtureをsnapshot"))
        })
        .collect::<Vec<_>>();

    let summary = validate_manifest(&corpus.root, &corpus.manifest_path)
        .expect("complete synthetic corpusはvalid");
    assert_eq!(
        summary,
        ManifestSummary {
            entries: 30,
            official: 6,
            oripa: 8,
            oriedita: 8,
            origami_simulator: 8,
            supported: 20,
            unsupported: 10,
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

    let mut missing = synthetic_corpus();
    missing.entries_mut()[0]["path"] = json!("raw/official/missing.fold");
    missing.write_manifest();
    assert_rejected_with(&missing, "metadata");

    let mut directory = synthetic_corpus();
    directory.entries_mut()[0]["path"] = json!("raw/official");
    directory.write_manifest();
    assert_rejected_with(&directory, "regular file");
}

#[test]
fn symlinks_are_rejected_when_the_platform_can_create_one() {
    let mut corpus = synthetic_corpus();
    let target = corpus.entry_path(0);
    let link = corpus.root.join("raw/official/symlink.fold");
    match try_symlink_file(&target, &link) {
        Ok(()) => {
            corpus.entries_mut()[0]["path"] = json!("raw/official/symlink.fold");
            corpus.write_manifest();
            assert_rejected_with(&corpus, "symlink");
        }
        Err(error) => {
            eprintln!("symlink creation is unavailable on this host; validator branch remains active: {error}");
        }
    }
}

#[test]
fn exact_source_and_classification_quotas_are_required() {
    let mut source = synthetic_corpus();
    source.entries_mut()[6]["source"] = json!("official");
    source.write_manifest();
    assert_rejected_with(&source, "source quota");

    let mut class = synthetic_corpus();
    class.entries_mut()[20]["classification"] = json!("supported");
    class.write_manifest();
    assert_rejected_with(&class, "classification quota");

    let mut total = synthetic_corpus();
    total.entries_mut().pop();
    total.write_manifest();
    assert_rejected_with(&total, "exactly 30");
}

#[test]
fn ids_paths_and_hashes_must_each_be_unique() {
    let mut id = synthetic_corpus();
    id.entries_mut()[1]["id"] = id.manifest["entries"][0]["id"].clone();
    id.write_manifest();
    assert_rejected_with(&id, "duplicate id");

    let mut path = synthetic_corpus();
    path.entries_mut()[1]["path"] = path.manifest["entries"][0]["path"].clone();
    path.write_manifest();
    assert_rejected_with(&path, "duplicate path");

    let mut hash = synthetic_corpus();
    hash.entries_mut()[1]["sha256"] = hash.manifest["entries"][0]["sha256"].clone();
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
        "source_url",
        "license_spdx",
        "license_url",
        "attribution",
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
    unresolved.entries_mut()[0]["rights"]["license_spdx"] = json!("NOASSERTION");
    unresolved.write_manifest();
    assert_rejected_with(&unresolved, "NOASSERTION");

    let mut forbidden = synthetic_corpus();
    forbidden.entries_mut()[0]["rights"]["redistribution_allowed"] = json!(false);
    forbidden.write_manifest();
    assert_rejected_with(&forbidden, "redistribution_allowed");
}

#[test]
fn classifications_must_be_present_and_frozen_at_twenty_ten() {
    let mut not_frozen = synthetic_corpus();
    not_frozen.manifest["classification_frozen"] = json!(false);
    not_frozen.write_manifest();
    assert_rejected_with(&not_frozen, "classification_frozen");

    let mut missing = synthetic_corpus();
    missing.entries_mut()[0]
        .as_object_mut()
        .expect("entryはobject")
        .remove("classification");
    missing.write_manifest();
    assert_rejected_with(&missing, "classification");

    let mut unknown = synthetic_corpus();
    unknown.entries_mut()[0]["classification"] = json!("pending");
    unknown.write_manifest();
    assert_rejected_with(&unknown, "supported or unsupported");
}
