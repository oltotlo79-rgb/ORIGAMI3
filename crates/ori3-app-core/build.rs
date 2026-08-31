use std::env;
use std::fs;
use std::path::{Path, PathBuf};

struct Contract {
    rust_name: &'static str,
    signature: &'static str,
    attribute: &'static str,
}

#[derive(Clone, Copy)]
enum WireSource {
    Commands,
    Store,
    Autosave,
}

struct WireType {
    name: &'static str,
    kind: &'static str,
    source: WireSource,
}

const CONTRACTS: [Contract; 18] = [
    Contract {
        rust_name: "document_new",
        signature: "pub fn document_new(app: tauri::AppHandle, state: State<'_, Mutex<DocumentStore>>, paper: Paper) -> Result<DocumentView, String>",
        attribute: "#[tauri::command(async)]",
    },
    Contract {
        rust_name: "document_open",
        signature: "pub fn document_open(app: tauri::AppHandle, state: State<'_, Mutex<DocumentStore>>, path: String) -> Result<DocumentView, String>",
        attribute: "#[tauri::command(async)]",
    },
    Contract {
        rust_name: "document_save",
        signature: "pub fn document_save(app: tauri::AppHandle, state: State<'_, Mutex<DocumentStore>>, path: Option<String>) -> Result<(), String>",
        attribute: "#[tauri::command(async)]",
    },
    Contract {
        rust_name: "edit_apply",
        signature: "pub fn edit_apply(state: State<'_, Mutex<DocumentStore>>, op: EditOp) -> Result<DocumentView, String>",
        attribute: "#[tauri::command(async)]",
    },
    Contract {
        rust_name: "edit_apply_batch",
        signature: "pub fn edit_apply_batch(state: State<'_, Mutex<DocumentStore>>, ops: Vec<EditOp>) -> Result<DocumentView, String>",
        attribute: "#[tauri::command(async)]",
    },
    Contract {
        rust_name: "edit_undo",
        signature: "pub fn edit_undo(state: State<'_, Mutex<DocumentStore>>) -> Result<DocumentView, String>",
        attribute: "#[tauri::command(async)]",
    },
    Contract {
        rust_name: "edit_redo",
        signature: "pub fn edit_redo(state: State<'_, Mutex<DocumentStore>>) -> Result<DocumentView, String>",
        attribute: "#[tauri::command(async)]",
    },
    Contract {
        rust_name: "sequence_apply",
        signature: "pub fn sequence_apply(state: State<'_, Mutex<DocumentStore>>, op: serde_json::Value) -> Result<DocumentView, String>",
        attribute: "#[tauri::command(async)]",
    },
    Contract {
        rust_name: "sequence_replay",
        signature: "pub fn sequence_replay(state: State<'_, Mutex<DocumentStore>>, up_to: usize, t: f64, soft: Option<SoftSettings>) -> Result<ReplayOutcome, String>",
        attribute: "#[tauri::command(async)]",
    },
    Contract {
        rust_name: "pose_solve",
        signature: "pub fn pose_solve(state: State<'_, Mutex<DocumentStore>>, request: PoseSolveRequest) -> Result<PoseOutcome, String>",
        attribute: "#[tauri::command(async)]",
    },
    Contract {
        rust_name: "fold_all_preview",
        signature: "pub fn fold_all_preview(state: State<'_, Mutex<DocumentStore>>, percent: f64, warm_seed: Option<Vec<Driver>>) -> Result<FoldAllPreviewOutcome, String>",
        attribute: "#[tauri::command(async)]",
    },
    Contract {
        rust_name: "recovery_check",
        signature: "pub fn recovery_check(app: tauri::AppHandle) -> Result<Option<autosave::RecoveryChoices>, String>",
        attribute: "#[tauri::command(async)]",
    },
    Contract {
        rust_name: "recovery_restore",
        signature: "pub fn recovery_restore(app: tauri::AppHandle, state: State<'_, Mutex<DocumentStore>>, accept: bool, candidate_id: u64) -> Result<Option<DocumentView>, String>",
        attribute: "#[tauri::command(async)]",
    },
    Contract {
        rust_name: "proposal_generate_job",
        signature: "pub fn proposal_generate_job(jobs: State<'_, ProposalJobs>, job_id: ProposalJobId, skeleton: Skeleton, paper: Paper, seed: u64, with_fold_plan: bool) -> Result<ProposalJobResult, String>",
        attribute: "#[tauri::command(async, rename = \"proposal_generate\")]",
    },
    Contract {
        rust_name: "proposal_progress",
        signature: "pub fn proposal_progress(jobs: State<'_, ProposalJobs>, job_id: ProposalJobId) -> Option<ProposalProgressSnapshot>",
        attribute: "#[tauri::command]",
    },
    Contract {
        rust_name: "proposal_control",
        signature: "pub fn proposal_control(jobs: State<'_, ProposalJobs>, operation: ProposalControl) -> Result<ProposalProgressSnapshot, String>",
        attribute: "#[tauri::command]",
    },
    Contract {
        rust_name: "proposal_apply",
        signature: "pub fn proposal_apply(state: State<'_, Mutex<DocumentStore>>, cp: CreasePattern, steps: Vec<FoldStep>) -> Result<DocumentView, String>",
        attribute: "#[tauri::command(async)]",
    },
    Contract {
        rust_name: "document_export",
        signature: "pub fn document_export(state: State<'_, Mutex<DocumentStore>>, kind: ExportKind, path: String, options: ExportOptions) -> Result<Vec<FoldIssue>, String>",
        attribute: "#[tauri::command(async)]",
    },
];

const REGISTERED_FUNCTIONS: [&str; 18] = [
    "document_new",
    "document_open",
    "document_save",
    "edit_apply",
    "edit_apply_batch",
    "edit_undo",
    "edit_redo",
    "sequence_apply",
    "sequence_replay",
    "pose_solve",
    "fold_all_preview",
    "recovery_check",
    "recovery_restore",
    "proposal_generate_job",
    "proposal_progress",
    "proposal_control",
    "proposal_apply",
    "document_export",
];

const WIRE_TYPES: [WireType; 20] = [
    WireType {
        name: "DocumentView",
        kind: "struct",
        source: WireSource::Store,
    },
    WireType {
        name: "PoseSolveMode",
        kind: "enum",
        source: WireSource::Commands,
    },
    WireType {
        name: "PoseSolveRequest",
        kind: "struct",
        source: WireSource::Commands,
    },
    WireType {
        name: "PoseOutcome",
        kind: "struct",
        source: WireSource::Commands,
    },
    WireType {
        name: "FoldAllLayerOrder",
        kind: "enum",
        source: WireSource::Commands,
    },
    WireType {
        name: "FoldAllPreviewOutcome",
        kind: "struct",
        source: WireSource::Commands,
    },
    WireType {
        name: "ReplayOutcome",
        kind: "struct",
        source: WireSource::Commands,
    },
    WireType {
        name: "RecoveryInfo",
        kind: "struct",
        source: WireSource::Autosave,
    },
    WireType {
        name: "RecoveryChoices",
        kind: "struct",
        source: WireSource::Autosave,
    },
    WireType {
        name: "ProposalFoldPlanDetails",
        kind: "struct",
        source: WireSource::Commands,
    },
    WireType {
        name: "ProposalFoldPlan",
        kind: "struct",
        source: WireSource::Commands,
    },
    WireType {
        name: "ProposalFoldPlanState",
        kind: "enum",
        source: WireSource::Commands,
    },
    WireType {
        name: "ProposalCandidate",
        kind: "struct",
        source: WireSource::Commands,
    },
    WireType {
        name: "ProposalJobId",
        kind: "struct",
        source: WireSource::Commands,
    },
    WireType {
        name: "ProposalPhase",
        kind: "enum",
        source: WireSource::Commands,
    },
    WireType {
        name: "ProposalProgressSnapshot",
        kind: "struct",
        source: WireSource::Commands,
    },
    WireType {
        name: "ProposalControl",
        kind: "enum",
        source: WireSource::Commands,
    },
    WireType {
        name: "ProposalJobResult",
        kind: "struct",
        source: WireSource::Commands,
    },
    WireType {
        name: "ExportKind",
        kind: "enum",
        source: WireSource::Commands,
    },
    WireType {
        name: "ExportOptions",
        kind: "struct",
        source: WireSource::Commands,
    },
];

fn compact(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .replace(",)", ")")
}

fn function_start(source: &str, name: &str) -> Result<usize, String> {
    let marker = format!("pub fn {name}");
    source
        .find(&marker)
        .ok_or_else(|| format!("desktop commands.rs に pub fn {name} がありません"))
}

fn function_signature(source: &str, name: &str) -> Result<String, String> {
    let start = function_start(source, name)?;
    let tail = &source[start..];
    let mut parenthesis_depth = 0_u32;
    let mut saw_parenthesis = false;
    for (offset, character) in tail.char_indices() {
        match character {
            '(' => {
                saw_parenthesis = true;
                parenthesis_depth += 1;
            }
            ')' => {
                parenthesis_depth = parenthesis_depth
                    .checked_sub(1)
                    .ok_or_else(|| format!("{name} の括弧が対応していません"))?;
            }
            '{' if saw_parenthesis && parenthesis_depth == 0 => {
                return Ok(compact(&tail[..offset]));
            }
            _ => {}
        }
    }
    Err(format!("{name} の関数本体の開始位置を読めません"))
}

fn command_attribute(source: &str, name: &str) -> Result<String, String> {
    let start = function_start(source, name)?;
    let prefix = &source[..start];
    let attribute_start = prefix
        .rfind("#[tauri::command")
        .ok_or_else(|| format!("{name} に #[tauri::command] がありません"))?;
    let attribute = &prefix[attribute_start..];
    let attribute_end = attribute
        .find(']')
        .ok_or_else(|| format!("{name} の #[tauri::command] が閉じていません"))?;
    let between_attribute_and_function = &attribute[attribute_end + 1..];
    if between_attribute_and_function.contains('{') || between_attribute_and_function.contains('}')
    {
        return Err(format!("{name} の直前に #[tauri::command] がありません"));
    }
    Ok(compact(&attribute[..=attribute_end]))
}

fn registered_functions(source: &str) -> Result<Vec<String>, String> {
    let marker = "tauri::generate_handler![";
    let start = source
        .find(marker)
        .ok_or_else(|| "desktop lib.rs に tauri::generate_handler! がありません".to_owned())?;
    let tail = &source[start + marker.len()..];
    let end = tail
        .find("])")
        .ok_or_else(|| "desktop lib.rs の generate_handler! が閉じていません".to_owned())?;
    Ok(tail[..end]
        .split("commands::")
        .skip(1)
        .map(|part| {
            part.chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect()
        })
        .collect())
}

/// wireに出ない `#[serde(skip)]` fieldと行commentを取り除く。
fn wire_source(source: &str) -> String {
    let mut output = String::new();
    let mut skip_next_field = false;
    for line in source.lines() {
        let code = line.split_once("//").map_or(line, |(code, _)| code);
        if code.trim().starts_with("#[serde(skip)]") {
            skip_next_field = true;
            continue;
        }
        if skip_next_field {
            if code.trim().is_empty() {
                continue;
            }
            skip_next_field = false;
            continue;
        }
        output.push_str(code);
        output.push('\n');
    }
    output
}

fn item_fingerprint(source: &str, kind: &str, name: &str) -> Result<String, String> {
    let source = wire_source(source);
    let public_marker = format!("pub {kind} {name}");
    let private_marker = format!("{kind} {name}");
    let (item_start, marker_length) = find_item_marker(&source, &public_marker)
        .map(|start| (start, public_marker.len()))
        .or_else(|| {
            find_item_marker(&source, &private_marker).map(|start| (start, private_marker.len()))
        })
        .ok_or_else(|| format!("{kind} {name} がありません"))?;
    let start = item_start + marker_length;
    let outer_serde = outer_serde_fingerprint(&source, item_start);
    let tail = &source[start..];
    let (opening_offset, opening) = tail
        .char_indices()
        .find(|(_, character)| matches!(character, '{' | '(' | ';'))
        .ok_or_else(|| format!("{kind} {name} の本体を読めません"))?;
    if opening == ';' {
        return Ok(format!("{outer_serde};"));
    }
    let closing = if opening == '{' { '}' } else { ')' };
    let body = &tail[opening_offset..];
    let mut depth = 0_u32;
    for (offset, character) in body.char_indices() {
        if character == opening {
            depth += 1;
        } else if character == closing {
            depth = depth
                .checked_sub(1)
                .ok_or_else(|| format!("{kind} {name} の括弧が対応していません"))?;
            if depth == 0 {
                return Ok(format!("{outer_serde}{}", compact(&body[..=offset])));
            }
        }
    }
    Err(format!("{kind} {name} の本体が閉じていません"))
}

fn find_item_marker(source: &str, marker: &str) -> Option<usize> {
    source.match_indices(marker).find_map(|(start, _)| {
        let before_is_identifier = source[..start]
            .chars()
            .next_back()
            .is_some_and(|character| character.is_alphanumeric() || character == '_');
        let end = start + marker.len();
        let after_is_identifier = source[end..]
            .chars()
            .next()
            .is_some_and(|character| character.is_alphanumeric() || character == '_');
        (!before_is_identifier && !after_is_identifier).then_some(start)
    })
}

fn outer_serde_fingerprint(source: &str, item_start: usize) -> String {
    let mut tokens = Vec::new();
    for line in source[..item_start].lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !line.starts_with("#[") {
            break;
        }
        let Some(inner) = line
            .strip_prefix("#[serde(")
            .and_then(|value| value.strip_suffix(")]"))
        else {
            continue;
        };
        tokens.extend(
            inner
                .split(',')
                .map(compact)
                .filter(|token| token != "deny_unknown_fields"),
        );
    }
    tokens.sort_unstable();
    if tokens.is_empty() {
        String::new()
    } else {
        format!("serde({})", tokens.join(","))
    }
}

fn scanner_self_check(errors: &mut Vec<String>) {
    let collision = r#"
        pub struct ProposalFoldPlanDetails { pub wrong: u8 }
        pub struct ProposalFoldPlan { pub right: u16 }
    "#;
    match item_fingerprint(collision, "struct", "ProposalFoldPlan") {
        Ok(fingerprint) if fingerprint.contains("right:u16") && !fingerprint.contains("wrong") => {}
        Ok(fingerprint) => errors.push(format!(
            "wire scannerがprefix衝突を区別できません: {fingerprint}"
        )),
        Err(error) => errors.push(format!("wire scanner自己検査: {error}")),
    }

    let transparent = "#[serde(transparent)]\npub struct JobId(String);";
    let plain = "pub struct JobId(String);";
    match (
        item_fingerprint(transparent, "struct", "JobId"),
        item_fingerprint(plain, "struct", "JobId"),
    ) {
        (Ok(with_attribute), Ok(without_attribute)) if with_attribute != without_attribute => {}
        _ => errors.push("wire scannerがouter serde属性の差を検出できません".to_owned()),
    }

    let strict = "#[serde(tag = \"type\", deny_unknown_fields)]\npub enum Op { Cancel }";
    let desktop = "#[serde(tag = \"type\")]\npub enum Op { Cancel }";
    match (
        item_fingerprint(strict, "enum", "Op"),
        item_fingerprint(desktop, "enum", "Op"),
    ) {
        (Ok(core), Ok(host)) if core == host => {}
        _ => errors
            .push("wire scannerがdeserialize専用deny_unknown_fieldsを正規化できません".to_owned()),
    }
}

fn read(path: &Path, label: &str, errors: &mut Vec<String>) -> String {
    match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            errors.push(format!("{label} を読めません: {error}"));
            String::new()
        }
    }
}

fn write_result(out_dir: &Path, errors: &[String]) {
    let output = out_dir.join("desktop_contract_check.rs");
    let contents = if errors.is_empty() {
        "// desktop command contract: 18件一致\n".to_owned()
    } else {
        let message = format!(
            "デスクトップ版18コマンドとori3-app-coreの契約が一致しません:\n{}",
            errors.join("\n")
        );
        format!("compile_error!({message:?});\n")
    };
    fs::write(&output, contents).unwrap_or_else(|error| {
        panic!(
            "desktop契約検査の出力 {} を書けません: {error}",
            output.display()
        )
    });
}

fn main() {
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIRが設定されていません"),
    );
    let commands_path = manifest_dir.join("../../apps/desktop/src-tauri/src/commands.rs");
    let desktop_lib_path = manifest_dir.join("../../apps/desktop/src-tauri/src/lib.rs");
    let store_path = manifest_dir.join("../../apps/desktop/src-tauri/src/store.rs");
    let autosave_path = manifest_dir.join("../../apps/desktop/src-tauri/src/autosave.rs");
    let app_core_path = manifest_dir.join("src/lib.rs");
    println!("cargo:rerun-if-changed={}", commands_path.display());
    println!("cargo:rerun-if-changed={}", desktop_lib_path.display());
    println!("cargo:rerun-if-changed={}", store_path.display());
    println!("cargo:rerun-if-changed={}", autosave_path.display());
    println!("cargo:rerun-if-changed={}", app_core_path.display());

    let mut errors = Vec::new();
    scanner_self_check(&mut errors);
    let commands = read(&commands_path, "desktop commands.rs", &mut errors);
    let desktop_lib = read(&desktop_lib_path, "desktop lib.rs", &mut errors);
    let store = read(&store_path, "desktop store.rs", &mut errors);
    let autosave = read(&autosave_path, "desktop autosave.rs", &mut errors);
    let app_core = read(&app_core_path, "ori3-app-core lib.rs", &mut errors);

    if !commands.is_empty() {
        for contract in CONTRACTS {
            match function_signature(&commands, contract.rust_name) {
                Ok(actual) if actual == compact(contract.signature) => {}
                Ok(actual) => errors.push(format!(
                    "{} の署名が変わりました: {}",
                    contract.rust_name, actual
                )),
                Err(error) => errors.push(error),
            }
            match command_attribute(&commands, contract.rust_name) {
                Ok(actual) if actual == compact(contract.attribute) => {}
                Ok(actual) => errors.push(format!(
                    "{} のTauri公開属性が変わりました: {}",
                    contract.rust_name, actual
                )),
                Err(error) => errors.push(error),
            }
        }
    }

    if !desktop_lib.is_empty() {
        match registered_functions(&desktop_lib) {
            Ok(actual)
                if actual
                    == REGISTERED_FUNCTIONS
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>() => {}
            Ok(actual) => errors.push(format!(
                "generate_handler! の18件が変わりました: {}",
                actual.join(", ")
            )),
            Err(error) => errors.push(error),
        }
    }

    if !app_core.is_empty() && !commands.is_empty() && !store.is_empty() && !autosave.is_empty() {
        for wire_type in WIRE_TYPES {
            let desktop_source = match wire_type.source {
                WireSource::Commands => &commands,
                WireSource::Store => &store,
                WireSource::Autosave => &autosave,
            };
            match (
                item_fingerprint(desktop_source, wire_type.kind, wire_type.name),
                item_fingerprint(&app_core, wire_type.kind, wire_type.name),
            ) {
                (Ok(desktop), Ok(core)) if desktop == core => {}
                (Ok(desktop), Ok(core)) => errors.push(format!(
                    "wire型 {} のfield/serde形が変わりました: desktop={} core={}",
                    wire_type.name, desktop, core
                )),
                (Err(error), _) => errors.push(format!("desktop {error}")),
                (_, Err(error)) => errors.push(format!("ori3-app-core {error}")),
            }
        }
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIRが設定されていません"));
    write_result(&out_dir, &errors);
}
