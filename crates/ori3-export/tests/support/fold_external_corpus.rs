use serde_json::{Map, Value};
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[path = "fold_sha256.rs"]
mod fold_sha256;

pub use fold_sha256::sha256_hex;

/// 公式リポジトリ`edemaine/FOLD`にはFOLD 1.1以上の見本が2件しかない
/// (残り3件はFOLD 1.0で、限定profileが読める版ではない)。実測どおりの2件にする。
const OFFICIAL_QUOTA: usize = 2;
/// 4番目の出所。利用者の決定(2026-08-29)でORIPAから`origamimagiro/flat-folder`へ
/// 差し替えた。ORIPAの配布物に`.fold`が無く(`.opx`のみ)、書き出すにはORIPA本体を
/// 動かす必要があるためである。公式が2件しか無い分をここで持ち、合計30件を保つ。
const FLAT_FOLDER_QUOTA: usize = 12;
const ORIEDITA_QUOTA: usize = 8;
const ORIGAMI_SIMULATOR_QUOTA: usize = 8;
const USER_AUTHORIZED_LICENSE: &str = "LicenseRef-ORIGAMI3-User-Authorized-Samples-2026-08-26";
const SYNTHETIC_TEST_LICENSE: &str = "LicenseRef-Synthetic-Test-Only";
/// Upstream licence reviewed on 2026-08-29 for `edemaine/FOLD` and
/// `origamimagiro/flat-folder`. Both ship the same MIT text, copied beside the
/// samples as `LICENSE-FOLD.txt` and `LICENSE-flat-folder.txt`.
const REVIEWED_UPSTREAM_LICENSE: &str = "MIT";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestRightsProfile {
    UserAuthorizedSamples20260826,
    SyntheticTestOnly,
}

impl ManifestRightsProfile {
    /// Every SPDX id this profile accepts for sample content.
    ///
    /// The user-authorized LicenseRef covers samples the ORIGAMI3 user made and
    /// authorized. `MIT` covers upstream samples whose own licence permits
    /// redistribution; the licence text ships next to those samples so the
    /// required notice travels with them.
    fn approved_content_licenses(self) -> &'static [&'static str] {
        match self {
            Self::UserAuthorizedSamples20260826 => {
                &[USER_AUTHORIZED_LICENSE, REVIEWED_UPSTREAM_LICENSE]
            }
            Self::SyntheticTestOnly => &[SYNTHETIC_TEST_LICENSE],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestSummary {
    pub entries: usize,
    pub official: usize,
    pub flat_folder: usize,
    pub oriedita: usize,
    pub origami_simulator: usize,
    pub expected_supported: usize,
    pub expected_unsupported: usize,
    pub observed_supported: usize,
    pub observed_unsupported: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError {
    message: String,
}

impl ManifestError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ManifestError {}

/// Validate a frozen external-corpus tranche without modifying its manifest or
/// raw fixtures. The manifest reports actual counts; it does not force a target
/// supported/unsupported ratio after results are known.
pub fn validate_manifest(
    corpus_root: &Path,
    manifest_path: &Path,
    rights_profile: ManifestRightsProfile,
) -> Result<ManifestSummary, ManifestError> {
    validate_corpus_root(corpus_root)?;
    validate_manifest_path(corpus_root, manifest_path)?;
    let manifest_bytes = fs::read(manifest_path).map_err(|error| {
        ManifestError::new(format!(
            "manifest could not be read at {}: {error}",
            manifest_path.display()
        ))
    })?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| ManifestError::new(format!("manifest is not valid JSON: {error}")))?;
    let manifest = manifest
        .as_object()
        .ok_or_else(|| ManifestError::new("manifest root must be an object"))?;

    match manifest.get("schema_version").and_then(Value::as_u64) {
        Some(2) => {}
        _ => return Err(ManifestError::new("schema_version must be 2")),
    }
    validate_classification_policy(manifest)?;

    let entries = manifest
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| ManifestError::new("entries must be an array"))?;
    if entries.is_empty() {
        return Err(ManifestError::new(
            "manifest entries must contain at least one accepted sample",
        ));
    }

    let mut summary = ManifestSummary {
        entries: entries.len(),
        official: 0,
        flat_folder: 0,
        oriedita: 0,
        origami_simulator: 0,
        expected_supported: 0,
        expected_unsupported: 0,
        observed_supported: 0,
        observed_unsupported: 0,
    };
    let mut ids = HashSet::with_capacity(entries.len());
    let mut paths = HashSet::with_capacity(entries.len());
    let mut hashes = HashSet::with_capacity(entries.len());

    for (index, entry) in entries.iter().enumerate() {
        let context = format!("entries[{index}]");
        let entry = entry
            .as_object()
            .ok_or_else(|| ManifestError::new(format!("{context} must be an object")))?;

        let id = required_text(entry, "id", &context)?;
        if !ids.insert(id) {
            return Err(ManifestError::new(format!("duplicate id: {id}")));
        }

        let source = required_text(entry, "source", &context)?;
        match source {
            "official" => summary.official += 1,
            "flat_folder" => summary.flat_folder += 1,
            "oriedita" => summary.oriedita += 1,
            "origami_simulator" => summary.origami_simulator += 1,
            source => {
                return Err(ManifestError::new(format!(
                    "{context}.source is not a reserved source: {source}"
                )));
            }
        }
        validate_provenance(entry, &context)?;

        let path_text = required_text(entry, "path", &context)?;
        let relative_path = validate_relative_path(path_text, &context)?;
        if !paths.insert(path_text) {
            return Err(ManifestError::new(format!("duplicate path: {path_text}")));
        }
        let reserved_source = reserved_source_for_id(id).ok_or_else(|| {
            ManifestError::new(format!("{context}.id is not a reserved slot: {id}"))
        })?;
        if source != reserved_source {
            return Err(ManifestError::new(format!(
                "{context}.source is not the reserved source for {id}: expected {reserved_source}, found {source}"
            )));
        }
        let reserved_path = format!("{}/{id}.fold", source_directory(reserved_source));
        if path_text != reserved_path {
            return Err(ManifestError::new(format!(
                "{context}.path is not the reserved path for {id}: expected {reserved_path}, found {path_text}"
            )));
        }

        let byte_length = entry
            .get("byte_length")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                ManifestError::new(format!(
                    "{context}.byte_length must be a non-negative integer"
                ))
            })?;
        let expected_hash = required_text(entry, "sha256", &context)?;
        if expected_hash.len() != 64
            || !expected_hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ManifestError::new(format!(
                "{context}.sha256 must be 64 lowercase hexadecimal characters"
            )));
        }
        if !hashes.insert(expected_hash) {
            return Err(ManifestError::new(format!(
                "duplicate sha256: {expected_hash}"
            )));
        }

        let classification = entry
            .get("classification")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                ManifestError::new(format!("{context}.classification must be an object"))
            })?;
        for field in classification.keys() {
            if !matches!(
                field.as_str(),
                "expected" | "frozen_at_utc" | "basis" | "unsupported_paths" | "adjudication"
            ) {
                return Err(ManifestError::new(format!(
                    "{context}.classification contains an unsupported field: {field}"
                )));
            }
        }
        let expected = required_resolved_text(
            classification,
            "expected",
            &format!("{context}.classification"),
        )?;
        let frozen_at_utc = required_resolved_text(
            classification,
            "frozen_at_utc",
            &format!("{context}.classification"),
        )?;
        require_utc(
            frozen_at_utc,
            &format!("{context}.classification.frozen_at_utc"),
        )?;
        // An entry may only carry a freeze time that the policy itself declares.
        // Tranches arrive on different days, so the policy lists every freeze it
        // authorises; an entry still cannot invent a timestamp of its own.
        if !declared_freeze_times(manifest).iter().any(|declared| declared == frozen_at_utc) {
            return Err(ManifestError::new(format!(
                "{context}.classification.frozen_at_utc must match classification_policy.frozen_at_utc or one of classification_policy.additional_frozen_at_utc"
            )));
        }
        required_resolved_text(
            classification,
            "basis",
            &format!("{context}.classification"),
        )?;
        let unsupported_paths = classification
            .get("unsupported_paths")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ManifestError::new(format!(
                    "{context}.classification.unsupported_paths must be an array"
                ))
            })?;
        for (path_index, path) in unsupported_paths.iter().enumerate() {
            let path = path.as_str().filter(|path| !path.trim().is_empty()).ok_or_else(|| {
                ManifestError::new(format!(
                    "{context}.classification.unsupported_paths[{path_index}] must be a non-empty string"
                ))
            })?;
            if !path.starts_with('$') {
                return Err(ManifestError::new(format!(
                    "{context}.classification.unsupported_paths[{path_index}] must be a JSON path"
                )));
            }
        }
        match expected {
            "supported" => {
                if !unsupported_paths.is_empty() {
                    return Err(ManifestError::new(format!(
                        "{context}.classification supported entry must not list unsupported paths"
                    )));
                }
                summary.expected_supported += 1;
            }
            "unsupported" => {
                if unsupported_paths.is_empty() {
                    return Err(ManifestError::new(format!(
                        "{context}.classification requires at least one unsupported path"
                    )));
                }
                summary.expected_unsupported += 1;
            }
            classification => {
                return Err(ManifestError::new(format!(
                    "{context}.classification must be supported or unsupported, found {classification}"
                )));
            }
        }
        validate_classification_adjudication(
            classification,
            &format!("{context}.classification"),
            expected,
        )?;
        let observation = entry
            .get("observed")
            .and_then(Value::as_object)
            .ok_or_else(|| ManifestError::new(format!("{context}.observed must be an object")))?;
        validate_observation(observation, &context, &mut summary)?;
        validate_rights(entry, &context, rights_profile)?;

        let raw = read_regular_file_without_symlinks(corpus_root, &relative_path, &context)?;
        if raw.len() as u64 != byte_length {
            return Err(ManifestError::new(format!(
                "{context}.byte_length mismatch: manifest {byte_length}, raw {}",
                raw.len()
            )));
        }
        let actual_hash = sha256_hex(&raw);
        if actual_hash != expected_hash {
            return Err(ManifestError::new(format!(
                "{context}.sha256 mismatch: manifest {expected_hash}, raw {actual_hash}"
            )));
        }
    }

    Ok(summary)
}

fn validate_corpus_root(corpus_root: &Path) -> Result<(), ManifestError> {
    let metadata = fs::symlink_metadata(corpus_root).map_err(|error| {
        ManifestError::new(format!(
            "corpus root metadata could not be read at {}: {error}",
            corpus_root.display()
        ))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ManifestError::new("corpus root must not be a symlink"));
    }
    if !metadata.file_type().is_dir() {
        return Err(ManifestError::new("corpus root must be a directory"));
    }
    Ok(())
}

fn validate_manifest_path(corpus_root: &Path, manifest_path: &Path) -> Result<(), ManifestError> {
    let expected_path = corpus_root.join("manifest.json");
    if manifest_path != expected_path {
        return Err(ManifestError::new(format!(
            "manifest path must be exactly {}, found {}",
            expected_path.display(),
            manifest_path.display()
        )));
    }

    let metadata = fs::symlink_metadata(manifest_path).map_err(|error| {
        ManifestError::new(format!(
            "manifest path metadata could not be read at {}: {error}",
            manifest_path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ManifestError::new("manifest path must not be a symlink"));
    }
    if !metadata.file_type().is_file() {
        return Err(ManifestError::new("manifest path must name a regular file"));
    }
    Ok(())
}

/// Every freeze time the policy authorises, in declaration order. The policy is
/// validated before entries, so both fields are already known to be UTC strings.
fn declared_freeze_times(manifest: &Map<String, Value>) -> Vec<String> {
    let policy = &manifest["classification_policy"];
    let mut times = vec![
        policy["frozen_at_utc"]
            .as_str()
            .expect("classification policy was validated before entries")
            .to_string(),
    ];
    if let Some(additional) = policy.get("additional_frozen_at_utc").and_then(Value::as_array) {
        for value in additional {
            times.push(
                value
                    .as_str()
                    .expect("classification policy was validated before entries")
                    .to_string(),
            );
        }
    }
    times
}

fn validate_classification_policy(manifest: &Map<String, Value>) -> Result<(), ManifestError> {
    let context = "classification_policy";
    let policy = manifest
        .get(context)
        .and_then(Value::as_object)
        .ok_or_else(|| ManifestError::new("classification_policy must be an object"))?;
    match policy.get("frozen").and_then(Value::as_bool) {
        Some(true) => {}
        _ => {
            return Err(ManifestError::new(
                "classification_policy.frozen must be true before formal acceptance",
            ));
        }
    }
    let frozen_at_utc = required_resolved_text(policy, "frozen_at_utc", context)?;
    require_utc(frozen_at_utc, "classification_policy.frozen_at_utc")?;
    // Later tranches are frozen on later days. Each additional freeze must be
    // declared here, as an explicit UTC instant, before any entry may use it.
    if let Some(additional) = policy.get("additional_frozen_at_utc") {
        let additional = additional.as_array().ok_or_else(|| {
            ManifestError::new("classification_policy.additional_frozen_at_utc must be an array")
        })?;
        let mut seen = HashSet::with_capacity(additional.len() + 1);
        seen.insert(frozen_at_utc);
        for (index, value) in additional.iter().enumerate() {
            let context = format!("classification_policy.additional_frozen_at_utc[{index}]");
            let value = value
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    ManifestError::new(format!("{context} must be a non-empty string"))
                })?;
            require_utc(value, &context)?;
            if !seen.insert(value) {
                return Err(ManifestError::new(format!(
                    "{context} repeats a freeze time already declared"
                )));
            }
        }
    }
    required_resolved_text(policy, "independent_auditor", context)?;
    match policy
        .get("auditor_had_runtime_results")
        .and_then(Value::as_bool)
    {
        Some(false) => {}
        _ => {
            return Err(ManifestError::new(
                "classification_policy.auditor_had_runtime_results must be false",
            ));
        }
    }
    match policy
        .get("formal_acceptance_runs_after_freeze")
        .and_then(Value::as_bool)
    {
        Some(true) => {}
        _ => {
            return Err(ManifestError::new(
                "classification_policy.formal_acceptance_runs_after_freeze must be true",
            ));
        }
    }
    let rules = policy
        .get("rules")
        .and_then(Value::as_array)
        .ok_or_else(|| ManifestError::new("classification_policy.rules must be an array"))?;
    if rules.is_empty()
        || rules
            .iter()
            .any(|rule| rule.as_str().is_none_or(|rule| rule.trim().is_empty()))
    {
        return Err(ManifestError::new(
            "classification_policy.rules must contain non-empty strings",
        ));
    }
    Ok(())
}

fn reserved_source_for_id(id: &str) -> Option<&'static str> {
    if reserved_index(id, "official-", OFFICIAL_QUOTA) {
        Some("official")
    } else if reserved_index(id, "flat-folder-", FLAT_FOLDER_QUOTA) {
        Some("flat_folder")
    } else if reserved_index(id, "oriedita-", ORIEDITA_QUOTA) {
        Some("oriedita")
    } else if reserved_index(id, "origami-simulator-", ORIGAMI_SIMULATOR_QUOTA) {
        Some("origami_simulator")
    } else {
        None
    }
}

fn source_directory(source: &str) -> &'static str {
    match source {
        "official" => "external/official",
        "flat_folder" => "external/flat_folder",
        "oriedita" => "external/oriedita",
        "origami_simulator" => "external/origami_simulator",
        _ => unreachable!("source is checked before its directory is requested"),
    }
}

fn reserved_index(id: &str, prefix: &str, maximum: usize) -> bool {
    let Some(index) = id.strip_prefix(prefix) else {
        return false;
    };
    index.len() == 2
        && index.bytes().all(|byte| byte.is_ascii_digit())
        && index
            .parse::<usize>()
            .is_ok_and(|index| (1..=maximum).contains(&index))
}

fn required_text<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a str, ManifestError> {
    let value = object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ManifestError::new(format!("{context}.{field} must be a string")))?;
    if value.trim().is_empty() {
        return Err(ManifestError::new(format!(
            "{context}.{field} must not be empty"
        )));
    }
    Ok(value)
}

fn required_resolved_text<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a str, ManifestError> {
    let value = required_text(object, field, context)?;
    if value.trim().eq_ignore_ascii_case("NOASSERTION") {
        return Err(ManifestError::new(format!(
            "{context}.{field} must not be NOASSERTION"
        )));
    }
    Ok(value)
}

fn validate_provenance(entry: &Map<String, Value>, context: &str) -> Result<(), ManifestError> {
    required_resolved_text(entry, "generator", context)?;
    required_resolved_text(entry, "generator_version", context)?;
    required_resolved_text(entry, "source_uri", context)?;
    let source_file_last_write_utc =
        required_resolved_text(entry, "source_file_last_write_utc", context)?;
    require_utc(
        source_file_last_write_utc,
        &format!("{context}.source_file_last_write_utc"),
    )
}

fn require_utc(value: &str, context: &str) -> Result<(), ManifestError> {
    if value.contains('T') && value.ends_with('Z') {
        Ok(())
    } else {
        Err(ManifestError::new(format!(
            "{context} must be an explicit UTC timestamp"
        )))
    }
}

fn validate_rights(
    entry: &Map<String, Value>,
    context: &str,
    rights_profile: ManifestRightsProfile,
) -> Result<(), ManifestError> {
    let rights = entry
        .get("rights")
        .and_then(Value::as_object)
        .ok_or_else(|| ManifestError::new(format!("{context}.rights must be an object")))?;
    for field in [
        "content_spdx",
        "content_evidence",
        "rights_holder",
        "authorization_date",
        "authorization_scope",
        "reviewer",
        "reviewed_on",
    ] {
        required_resolved_text(rights, field, &format!("{context}.rights"))?;
    }
    let content_spdx =
        required_resolved_text(rights, "content_spdx", &format!("{context}.rights"))?;
    let approved = rights_profile.approved_content_licenses();
    if !approved.contains(&content_spdx) {
        return Err(ManifestError::new(format!(
            "{context}.rights.content_spdx must use the approved LicenseRef or a reviewed upstream licence, one of: {}",
            approved.join(", ")
        )));
    }
    match rights
        .get("generator_license_used_for_content")
        .and_then(Value::as_bool)
    {
        Some(false) => {}
        _ => {
            return Err(ManifestError::new(format!(
                "{context}.rights.generator_license_used_for_content must be false"
            )));
        }
    }
    match rights
        .get("redistribution_allowed")
        .and_then(Value::as_bool)
    {
        Some(true) => Ok(()),
        _ => Err(ManifestError::new(format!(
            "{context}.rights.redistribution_allowed must be true"
        ))),
    }
}

fn validate_observation(
    observation: &Map<String, Value>,
    entry_context: &str,
    summary: &mut ManifestSummary,
) -> Result<(), ManifestError> {
    let context = format!("{entry_context}.observed");
    required_resolved_text(observation, "method", &context)?;
    let observed_at_utc = required_resolved_text(observation, "observed_at_utc", &context)?;
    require_utc(observed_at_utc, &format!("{context}.observed_at_utc"))?;
    match observation
        .get("excluded_from_frozen_classification")
        .and_then(Value::as_bool)
    {
        Some(true) => {}
        _ => {
            return Err(ManifestError::new(format!(
                "{context}.excluded_from_frozen_classification must be true"
            )));
        }
    }

    validate_observed_issues(observation, "warnings", &context)?;
    let errors = validate_observed_issues(observation, "errors", &context)?;
    let result = required_resolved_text(observation, "result", &context)?;
    match result {
        "supported" => {
            if !errors.is_empty() {
                return Err(ManifestError::new(format!(
                    "{context} supported result must not contain errors"
                )));
            }
            summary.observed_supported += 1;
        }
        "unsupported" => {
            if errors.is_empty() {
                return Err(ManifestError::new(format!(
                    "{context} unsupported result requires at least one error"
                )));
            }
            summary.observed_unsupported += 1;
        }
        result => {
            return Err(ManifestError::new(format!(
                "{context}.result must be supported or unsupported, found {result}"
            )));
        }
    }
    Ok(())
}

fn validate_classification_adjudication(
    classification: &Map<String, Value>,
    context: &str,
    expected: &str,
) -> Result<(), ManifestError> {
    let Some(adjudication) = classification.get("adjudication") else {
        return Ok(());
    };
    let adjudication = adjudication
        .as_object()
        .ok_or_else(|| ManifestError::new(format!("{context}.adjudication must be an object")))?;
    const FIELDS: [&str; 7] = [
        "from_expected",
        "to_expected",
        "authorized_by",
        "authorized_at_utc",
        "runtime_results_known",
        "raw_geometry_basis",
        "product_threshold_changed",
    ];
    for field in adjudication.keys() {
        if !FIELDS.contains(&field.as_str()) {
            return Err(ManifestError::new(format!(
                "{context}.adjudication contains an unsupported field: {field}"
            )));
        }
    }
    let from_expected = required_resolved_text(adjudication, "from_expected", context)?;
    let to_expected = required_resolved_text(adjudication, "to_expected", context)?;
    if from_expected != "supported" || to_expected != "unsupported" || expected != to_expected {
        return Err(ManifestError::new(format!(
            "{context}.adjudication may only record an authorized supported-to-unsupported correction"
        )));
    }
    let authorized_by = required_resolved_text(adjudication, "authorized_by", context)?;
    if authorized_by != "統括（Claude）" {
        return Err(ManifestError::new(format!(
            "{context}.adjudication.authorized_by must be 統括（Claude）"
        )));
    }
    let authorized_at_utc = required_resolved_text(adjudication, "authorized_at_utc", context)?;
    require_utc(
        authorized_at_utc,
        &format!("{context}.adjudication.authorized_at_utc"),
    )?;
    required_resolved_text(adjudication, "raw_geometry_basis", context)?;
    if adjudication
        .get("runtime_results_known")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(ManifestError::new(format!(
            "{context}.adjudication.runtime_results_known must be true"
        )));
    }
    if adjudication
        .get("product_threshold_changed")
        .and_then(Value::as_bool)
        != Some(false)
    {
        return Err(ManifestError::new(format!(
            "{context}.adjudication.product_threshold_changed must be false"
        )));
    }
    Ok(())
}

fn validate_observed_issues<'a>(
    observation: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a Vec<Value>, ManifestError> {
    let issues = observation
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| ManifestError::new(format!("{context}.{field} must be an array")))?;
    for (index, issue) in issues.iter().enumerate() {
        let issue_context = format!("{context}.{field}[{index}]");
        let issue = issue
            .as_object()
            .ok_or_else(|| ManifestError::new(format!("{issue_context} must be an object")))?;
        for field in issue.keys() {
            if !matches!(field.as_str(), "code" | "path" | "value") {
                return Err(ManifestError::new(format!(
                    "{issue_context} may contain only code, path, and optional numeric value"
                )));
            }
        }
        required_resolved_text(issue, "code", &issue_context)?;
        let path = required_resolved_text(issue, "path", &issue_context)?;
        if !path.starts_with('$') {
            return Err(ManifestError::new(format!(
                "{issue_context}.path must be a JSON path"
            )));
        }
        if issue.get("value").is_some_and(|value| !value.is_number()) {
            return Err(ManifestError::new(format!(
                "{issue_context}.value must be numeric when present"
            )));
        }
    }
    Ok(issues)
}

fn validate_relative_path(path_text: &str, context: &str) -> Result<PathBuf, ManifestError> {
    let path = Path::new(path_text);
    if path.is_absolute() {
        return Err(ManifestError::new(format!(
            "{context}.path must be relative"
        )));
    }

    let mut has_component = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => has_component = true,
            Component::ParentDir => {
                return Err(ManifestError::new(format!(
                    "{context}.path must not contain a parent component"
                )));
            }
            Component::CurDir => {
                return Err(ManifestError::new(format!(
                    "{context}.path must not contain a current-directory component"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ManifestError::new(format!(
                    "{context}.path must be relative"
                )));
            }
        }
    }
    if !has_component {
        return Err(ManifestError::new(format!(
            "{context}.path must not be empty"
        )));
    }
    Ok(path.to_path_buf())
}

fn read_regular_file_without_symlinks(
    corpus_root: &Path,
    relative_path: &Path,
    context: &str,
) -> Result<Vec<u8>, ManifestError> {
    let mut components = relative_path.components().peekable();
    let mut current = corpus_root.to_path_buf();
    while let Some(component) = components.next() {
        let Component::Normal(component) = component else {
            return Err(ManifestError::new(format!(
                "{context}.path contains an invalid component"
            )));
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            ManifestError::new(format!(
                "{context}.path metadata could not be read at {}: {error}",
                current.display()
            ))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ManifestError::new(format!(
                "{context}.path must not contain a symlink: {}",
                current.display()
            )));
        }
        let is_last = components.peek().is_none();
        if is_last {
            if !metadata.file_type().is_file() {
                return Err(ManifestError::new(format!(
                    "{context}.path must name a regular file: {}",
                    current.display()
                )));
            }
        } else if !metadata.file_type().is_dir() {
            return Err(ManifestError::new(format!(
                "{context}.path ancestor must be a directory: {}",
                current.display()
            )));
        }
    }

    fs::read(&current).map_err(|error| {
        ManifestError::new(format!(
            "{context}.raw bytes could not be read at {}: {error}",
            current.display()
        ))
    })
}
