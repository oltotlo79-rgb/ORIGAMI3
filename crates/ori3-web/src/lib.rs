//! ORIGAMI3の計算coreをWebAssemblyから呼び出すJSON境界。

use ori3_app_core::Ori3AppCore;

/// Web Workerが所有する、1つのアプリケーションcoreへの入口。
#[derive(Debug, Default)]
pub struct Ori3WasmBackend {
    core: Ori3AppCore,
}

impl Ori3WasmBackend {
    /// 空のWebAssembly向けbackendを作る。
    #[must_use]
    pub fn new() -> Self {
        Self {
            core: Ori3AppCore::new(),
        }
    }

    /// JSON要求をアプリケーションcoreへ転送する。
    pub fn invoke_json(&mut self, request_json: &str) -> Result<String, String> {
        self.core.invoke_json(request_json)
    }
}

#[cfg(target_arch = "wasm32")]
mod binding {
    use super::Ori3WasmBackend as Backend;
    use wasm_bindgen::prelude::*;

    /// JavaScript側へ公開するWASM backend。
    #[wasm_bindgen(js_name = Ori3WasmBackend)]
    pub struct Ori3WasmBackend {
        backend: Backend,
    }

    impl Default for Ori3WasmBackend {
        fn default() -> Self {
            Self {
                backend: Backend::new(),
            }
        }
    }

    #[wasm_bindgen(js_class = Ori3WasmBackend)]
    impl Ori3WasmBackend {
        /// Workerごとに1つのbackendを作る。
        #[wasm_bindgen(constructor)]
        pub fn new() -> Self {
            Self::default()
        }

        /// JSON文字列1本でcore commandを呼ぶ。
        pub fn invoke_json(&mut self, request_json: &str) -> Result<String, JsValue> {
            self.backend
                .invoke_json(request_json)
                .map_err(|error| JsValue::from_str(&error))
        }
    }
}

#[cfg(test)]
mod tests {
    use ori3_app_core::BACKEND_COMMAND_NAMES;
    use serde_json::json;

    use super::Ori3WasmBackend;

    #[test]
    fn native_backend_returns_the_desktop_document_new_fixture() {
        let request = json!({
            "command": "document_new",
            "args": {
                "paper": {
                    "width_mm": 150.0,
                    "height_mm": 100.0
                }
            }
        })
        .to_string();
        let mut backend = Ori3WasmBackend::new();
        let first = backend
            .invoke_json(&request)
            .expect("native Web backendで新規作品を作れる");
        let second = backend
            .invoke_json(&request)
            .expect("同じbackendを続けて利用できる");
        let expected =
            include_str!("../../ori3-app-core/tests/fixtures/document-new-150x100.json").trim_end();

        assert_eq!(first, expected);
        assert_eq!(second, expected);
    }

    #[test]
    fn native_backend_matches_the_desktop_edit_roundtrip_fixtures() {
        let mut backend = Ori3WasmBackend::new();
        let invoke = |backend: &mut Ori3WasmBackend, command: &str, args: serde_json::Value| {
            backend
                .invoke_json(&json!({ "command": command, "args": args }).to_string())
                .unwrap_or_else(|error| panic!("{command} failed: {error}"))
        };
        let document_new =
            include_str!("../../ori3-app-core/tests/fixtures/document-new-150x100.json").trim_end();
        let diagonal =
            include_str!("../../ori3-app-core/tests/fixtures/edit-apply-diagonal-150x100.json")
                .trim_end();
        let removed = include_str!(
            "../../ori3-app-core/tests/fixtures/edit-apply-batch-remove-diagonal-150x100.json"
        )
        .trim_end();

        assert_eq!(
            invoke(
                &mut backend,
                "document_new",
                json!({ "paper": { "width_mm": 150.0, "height_mm": 100.0 } }),
            ),
            document_new
        );
        assert_eq!(
            invoke(
                &mut backend,
                "edit_apply",
                json!({
                    "op": {
                        "type": "AddSegment",
                        "a": [0.0, 0.0],
                        "b": [1.0, 2.0 / 3.0],
                        "kind": "Mountain"
                    }
                }),
            ),
            diagonal
        );
        assert_eq!(
            invoke(
                &mut backend,
                "edit_apply_batch",
                json!({
                    "ops": [
                        { "type": "SetEdgeKind", "ids": [4], "kind": "Valley" },
                        { "type": "RemoveEdges", "ids": [4] }
                    ]
                }),
            ),
            removed
        );
        assert_eq!(invoke(&mut backend, "edit_undo", json!({})), diagonal);
        assert_eq!(invoke(&mut backend, "edit_redo", json!({})), removed);
    }

    #[test]
    fn native_backend_matches_the_desktop_sequence_fixtures() {
        let mut backend = Ori3WasmBackend::new();
        let invoke = |backend: &mut Ori3WasmBackend, command: &str, args: serde_json::Value| {
            backend
                .invoke_json(&json!({ "command": command, "args": args }).to_string())
                .unwrap_or_else(|error| panic!("{command} failed: {error}"))
        };
        let document_new =
            include_str!("../../ori3-app-core/tests/fixtures/document-new-150x100.json").trim_end();
        let preview = include_str!(
            "../../ori3-app-core/tests/fixtures/sequence-preview-fold-through-150x100.json"
        )
        .trim_end();
        let applied = include_str!(
            "../../ori3-app-core/tests/fixtures/sequence-apply-fold-through-150x100.json"
        )
        .trim_end();
        let replayed = include_str!(
            "../../ori3-app-core/tests/fixtures/sequence-replay-fold-through-half-150x100.json"
        )
        .trim_end();
        let fold_input = json!({
            "up_to": 0,
            "line": [[0.0, 0.0], [1.0, 2.0 / 3.0]],
            "keep_side_point": [0.0, 2.0 / 3.0],
            "target_layers": null,
            "direction": "Up"
        });

        assert_eq!(
            invoke(
                &mut backend,
                "document_new",
                json!({ "paper": { "width_mm": 150.0, "height_mm": 100.0 } }),
            ),
            document_new
        );
        let mut preview_op = fold_input.clone();
        preview_op["type"] = json!("PreviewFoldThrough");
        assert_eq!(
            invoke(&mut backend, "sequence_apply", json!({ "op": preview_op }),),
            preview
        );
        let mut apply_op = fold_input;
        apply_op["type"] = json!("FoldThrough");
        apply_op["accept_additional_crease"] = json!(false);
        assert_eq!(
            invoke(&mut backend, "sequence_apply", json!({ "op": apply_op }),),
            applied
        );
        assert_eq!(
            invoke(
                &mut backend,
                "sequence_replay",
                json!({ "upTo": 1, "t": 0.5, "soft": null }),
            ),
            replayed
        );
    }

    #[test]
    fn native_backend_matches_the_desktop_pose_fixtures() {
        let mut backend = Ori3WasmBackend::new();
        let invoke = |backend: &mut Ori3WasmBackend, command: &str, args: serde_json::Value| {
            backend
                .invoke_json(&json!({ "command": command, "args": args }).to_string())
                .unwrap_or_else(|error| panic!("{command} failed: {error}"))
        };
        let document_new =
            include_str!("../../ori3-app-core/tests/fixtures/document-new-150x100.json").trim_end();
        let diagonal =
            include_str!("../../ori3-app-core/tests/fixtures/edit-apply-diagonal-150x100.json")
                .trim_end();
        let pose =
            include_str!("../../ori3-app-core/tests/fixtures/pose-solve-diagonal-150x100.json")
                .trim_end();
        let fold_all_zero = include_str!(
            "../../ori3-app-core/tests/fixtures/fold-all-preview-diagonal-0-150x100.json"
        )
        .trim_end();
        let fold_all = include_str!(
            "../../ori3-app-core/tests/fixtures/fold-all-preview-diagonal-50-150x100.json"
        )
        .trim_end();

        assert_eq!(
            invoke(
                &mut backend,
                "document_new",
                json!({ "paper": { "width_mm": 150.0, "height_mm": 100.0 } }),
            ),
            document_new
        );
        assert_eq!(
            invoke(
                &mut backend,
                "edit_apply",
                json!({
                    "op": {
                        "type": "AddSegment",
                        "a": [0.0, 0.0],
                        "b": [1.0, 2.0 / 3.0],
                        "kind": "Mountain"
                    }
                }),
            ),
            diagonal
        );
        assert_eq!(
            invoke(
                &mut backend,
                "pose_solve",
                json!({
                    "request": {
                        "hard": [{ "hinge": 4, "target_angle_deg": 90.0 }],
                        "preferred": null,
                        "soft": null,
                        "warmSeed": null,
                        "upTo": 0,
                        "t": 1.0,
                        "mode": "Follow"
                    }
                }),
            ),
            pose
        );
        assert_eq!(
            invoke(
                &mut backend,
                "pose_solve",
                json!({
                    "request": {
                        "hard": [],
                        "preferred": [{ "hinge": 4, "target_angle_deg": 90.0 }],
                        "soft": null,
                        "warmSeed": [{ "hinge": 4, "target_angle_deg": 90.0 }],
                        "upTo": 0,
                        "t": 1.0,
                        "mode": "Canonical"
                    }
                }),
            ),
            pose
        );
        assert_eq!(
            invoke(
                &mut backend,
                "fold_all_preview",
                json!({ "percent": 0.0, "warmSeed": null }),
            ),
            fold_all_zero
        );
        assert_eq!(
            invoke(
                &mut backend,
                "fold_all_preview",
                json!({
                    "percent": 50.0,
                    "warmSeed": [{ "hinge": 4, "target_angle_deg": 0.0 }]
                }),
            ),
            fold_all
        );
    }

    #[test]
    fn native_backend_exposes_all_eighteen_commands_and_recovery_after_host_staging() {
        let mut backend = Ori3WasmBackend::new();
        assert_eq!(BACKEND_COMMAND_NAMES.len(), 18);
        assert_eq!(
            backend
                .invoke_json(
                    &json!({
                        "command": "__web_recovery_set_choices",
                        "args": { "choices": null }
                    })
                    .to_string()
                )
                .expect("browser host can stage the absence of recovery candidates"),
            "null"
        );
        assert_eq!(
            backend
                .invoke_json(&json!({ "command": "recovery_check", "args": {} }).to_string())
                .expect("staged recovery_check is implemented"),
            "null"
        );
        assert_eq!(
            backend
                .invoke_json(
                    &json!({
                        "command": "recovery_restore",
                        "args": { "accept": false, "candidateId": 1 }
                    })
                    .to_string()
                )
                .expect("rejecting a selected recovery candidate is implemented"),
            "null"
        );
    }

    #[test]
    fn native_backend_preserves_invalid_json_errors() {
        let error = Ori3WasmBackend::new()
            .invoke_json("not json")
            .expect_err("不正JSONを受理しない");

        assert!(error.starts_with("コマンド要求のJSONを解析できません"));
    }
}
