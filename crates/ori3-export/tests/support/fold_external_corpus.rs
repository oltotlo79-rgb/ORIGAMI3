use serde_json::{Map, Value};
use std::collections::HashSet;
use std::fmt::{self, Write as _};
use std::fs;
use std::path::{Component, Path, PathBuf};

const OFFICIAL_QUOTA: usize = 6;
const ORIPA_QUOTA: usize = 8;
const ORIEDITA_QUOTA: usize = 8;
const ORIGAMI_SIMULATOR_QUOTA: usize = 8;
const USER_AUTHORIZED_LICENSE: &str = "LicenseRef-ORIGAMI3-User-Authorized-Samples-2026-08-26";
const SYNTHETIC_TEST_LICENSE: &str = "LicenseRef-Synthetic-Test-Only";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestRightsProfile {
    UserAuthorizedSamples20260826,
    SyntheticTestOnly,
}

impl ManifestRightsProfile {
    fn approved_content_license(self) -> &'static str {
        match self {
            Self::UserAuthorizedSamples20260826 => USER_AUTHORIZED_LICENSE,
            Self::SyntheticTestOnly => SYNTHETIC_TEST_LICENSE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestSummary {
    pub entries: usize,
    pub official: usize,
    pub oripa: usize,
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
        oripa: 0,
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
            "oripa" => summary.oripa += 1,
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
        let policy_frozen_at_utc = manifest["classification_policy"]["frozen_at_utc"]
            .as_str()
            .expect("classification policy was validated before entries");
        if frozen_at_utc != policy_frozen_at_utc {
            return Err(ManifestError::new(format!(
                "{context}.classification.frozen_at_utc must match classification_policy.frozen_at_utc"
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
    } else if reserved_index(id, "oripa-", ORIPA_QUOTA) {
        Some("oripa")
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
        "oripa" => "external/oripa",
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
    let approved_content_license = rights_profile.approved_content_license();
    if content_spdx != approved_content_license {
        return Err(ManifestError::new(format!(
            "{context}.rights.content_spdx must use the approved LicenseRef {approved_content_license}"
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

/// Dependency-free SHA-256 for test corpus byte verification.
pub fn sha256_hex(input: &[u8]) -> String {
    let digest = sha256(input);
    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn sha256(input: &[u8]) -> [u8; 32] {
    const INITIAL: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    const ROUND: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    let bit_length = (input.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(input.len() + 72);
    padded.extend_from_slice(input);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_be_bytes());

    let mut state = INITIAL;
    for chunk in padded.chunks_exact(64) {
        let mut words = [0_u32; 64];
        for (index, word) in words[..16].iter_mut().enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..64 {
            let sigma0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let sigma1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(sigma0)
                .wrapping_add(words[index - 7])
                .wrapping_add(sigma1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let choose = (e & f) ^ ((!e) & g);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let big_sigma0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let big_sigma1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let temporary1 = h
                .wrapping_add(big_sigma1)
                .wrapping_add(choose)
                .wrapping_add(ROUND[index])
                .wrapping_add(words[index]);
            let temporary2 = big_sigma0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temporary1);
            d = c;
            c = b;
            b = a;
            a = temporary1.wrapping_add(temporary2);
        }

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }

    let mut digest = [0_u8; 32];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}
