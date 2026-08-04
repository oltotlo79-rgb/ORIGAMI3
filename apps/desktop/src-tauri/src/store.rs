//! DocumentStore: 作品データの保持と、編集適用・undo/redo・保存/読み込み。
//!
//! Tauriに依存しない純Rustとして実装し、コマンド層(commands.rs)から委譲される。
//! undo/redoは「編集前スナップショット」方式(v1は単純さ優先)。
//! 変更が実際に起きた場合のみundo履歴に積み、100件を超えたら最古を破棄する。
//!
//! エラーと警告の使い分け規則:
//! - 複数対象の操作(RemoveEdges/SetEdgeKind)は部分成功+警告(できる分だけ行う)
//! - 単一対象の不能(SetPaperの折り線あり、SeqOpのID不在など)はErr(何も変更しない)
//! - 幾何的な壊れ(交差・参照切れなど)は警告(「止めずに警告」原則)。
//!   例外としてMoveVertexの頂点ID不在は、SetEdgeKindのID不在との整合を優先し
//!   Errではなく警告+無変更とする
//!
//! 導出(validate/extract_faces)は候補Documentに対して先に実行し、成功した場合のみ
//! 状態を確定する。導出がpanicしてもstoreは直前の整合状態を保つ(guardがErr化する)。

use std::path::{Path, PathBuf};

use ori3_cp::Face;
use ori3_model::{
    CreasePattern, Document, EdgeKind, EditOp, Paper, SCHEMA_VERSION, SeqOp, VertexId,
};

/// undo履歴の最大件数。超過時は最古をFIFOで破棄する。
const MAX_UNDO: usize = 100;

/// フロントへ返す表示用ビュー(Document全体 + 導出情報)。
/// save以外の全コマンドの成功時戻り値。
#[derive(Clone, Debug, serde::Serialize)]
pub struct DocumentView {
    pub doc: Document,
    pub faces: Vec<Face>,
    /// 操作固有の警告 + `ori3_cp::validate` の結果(「止めずに警告」原則)
    pub warnings: Vec<String>,
    /// 局所平坦折り判定の違反頂点(Task 2-7で実装)。今は常に空
    pub violations: Vec<VertexId>,
}

pub struct DocumentStore {
    doc: Document,
    undo_stack: Vec<Document>,
    redo_stack: Vec<Document>,
    dirty: bool,
    path: Option<PathBuf>,
}

impl Default for DocumentStore {
    /// 起動直後の初期状態(150mm正方形の新規作品)。
    fn default() -> Self {
        DocumentStore {
            doc: Document::new(Paper {
                width_mm: 150.0,
                height_mm: 150.0,
            }),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            dirty: false,
            path: None,
        }
    }
}

impl DocumentStore {
    /// 新規作品を作る。undo/redo履歴・保存先パスは破棄される。
    pub fn new_document(&mut self, paper: Paper) -> Result<DocumentView, String> {
        check_paper(&paper)?;
        let doc = Document::new(paper);
        let view = build_view(&doc, Vec::new());
        self.doc = doc;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.dirty = false;
        self.path = None;
        Ok(view)
    }

    /// `.ori3`ファイル(pretty JSON)を読み込む。schema_version不一致はErr。
    pub fn open(&mut self, path: &Path) -> Result<DocumentView, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("ファイルを開けませんでした: {e}"))?;
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| format!("ファイルの内容を読み取れませんでした: {e}"))?;
        match value.get("schema_version").and_then(|v| v.as_u64()) {
            None => return Err("作品ファイルの形式ではありません".to_string()),
            Some(v) if v > u64::from(SCHEMA_VERSION) => {
                return Err(
                    "このファイルは新しい版のアプリで作られています。アプリを更新してください"
                        .to_string(),
                );
            }
            Some(v) if v < u64::from(SCHEMA_VERSION) => {
                return Err(format!("このファイルの形式(版{v})には対応していません"));
            }
            Some(_) => {}
        }
        let doc: Document = serde_json::from_value(value)
            .map_err(|e| format!("ファイルの内容を読み取れませんでした: {e}"))?;
        // 導出を先に済ませ、成功した場合のみ状態を確定する
        let view = build_view(&doc, Vec::new());
        self.doc = doc;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.dirty = false;
        self.path = Some(path.to_path_buf());
        Ok(view)
    }

    /// `.ori3`ファイル(pretty JSON)へ保存する。`None`なら前回のパスへ上書き。
    pub fn save(&mut self, path: Option<&Path>) -> Result<(), String> {
        let target = match path {
            Some(p) => p.to_path_buf(),
            None => self
                .path
                .clone()
                .ok_or_else(|| "保存先が指定されていません".to_string())?,
        };
        let json = serde_json::to_string_pretty(&self.doc)
            .map_err(|e| format!("保存データの作成に失敗しました: {e}"))?;
        std::fs::write(&target, json).map_err(|e| format!("保存に失敗しました: {e}"))?;
        self.path = Some(target);
        self.dirty = false;
        Ok(())
    }

    /// 編集操作を適用する。実際に変更が起きた場合のみundo履歴に積む。
    pub fn apply_edit(&mut self, op: EditOp) -> Result<DocumentView, String> {
        let mut doc = self.doc.clone();
        let mut warnings = Vec::new();
        match op {
            EditOp::AddSegment { a, b, kind } => {
                // 追加辺ゼロ(既存線と完全重複)でも成功扱い
                ori3_cp::insert_segment(&mut doc.cp, a, b, kind);
            }
            EditOp::RemoveEdges { ids } => {
                let removable: Vec<_> = ids
                    .iter()
                    .copied()
                    .filter(|id| !is_border(&doc.cp, *id))
                    .collect();
                if removable.len() != ids.len() {
                    warnings.push("輪郭線は削除できません".to_string());
                }
                ori3_cp::remove_edges(&mut doc.cp, &removable);
            }
            EditOp::SetEdgeKind { ids, kind } => {
                let mut warned_from_border = false;
                let mut warned_to_border = false;
                for id in ids {
                    let Some(e) = doc.cp.edges.iter_mut().find(|e| e.id == id) else {
                        warnings.push(format!("辺ID {id} が存在しません"));
                        continue;
                    };
                    if e.kind == EdgeKind::Border {
                        if !warned_from_border {
                            warnings.push("輪郭線の種類は変更できません".to_string());
                            warned_from_border = true;
                        }
                    } else if kind == EdgeKind::Border {
                        if !warned_to_border {
                            warnings.push("輪郭線へ変更することはできません".to_string());
                            warned_to_border = true;
                        }
                    } else {
                        e.kind = kind;
                    }
                }
            }
            EditOp::MoveVertex { id, to } => {
                if doc.cp.vertices.iter().any(|v| v.id == id) {
                    // 移動によりCPが壊れても止めない(validateの警告で知らせる)
                    ori3_cp::move_vertex(&mut doc.cp, id, to);
                } else {
                    warnings.push(format!("頂点ID {id} が存在しません"));
                }
            }
            EditOp::SetPaper { paper } => {
                check_paper(&paper)?;
                if doc.cp.edges.iter().any(|e| e.kind != EdgeKind::Border) {
                    return Err("折り線がある状態では紙サイズを変更できません".to_string());
                }
                // 輪郭のみなら作り直し(display/sequenceは維持)
                let fresh = Document::new(paper);
                doc.paper = fresh.paper;
                doc.cp = fresh.cp;
            }
            EditOp::ReplaceCreasePattern { cp } => {
                // 提案ウィザード用のCP全置換。妥当性はvalidateの警告として返すのみ
                doc.cp = cp;
            }
        }
        Ok(self.commit(doc, warnings))
    }

    /// 折り手順操作を適用する。実際に変更が起きた場合のみundo履歴に積む。
    pub fn apply_seq(&mut self, op: SeqOp) -> Result<DocumentView, String> {
        let mut doc = self.doc.clone();
        match op {
            SeqOp::PushStep { step } => doc.sequence.push(step),
            SeqOp::InsertStep { index, step } => {
                if index > doc.sequence.len() {
                    return Err(format!("挿入位置 {index} が手順の数を超えています"));
                }
                doc.sequence.insert(index, step);
            }
            SeqOp::RemoveStep { id } => {
                let before = doc.sequence.len();
                doc.sequence.retain(|s| s.id != id);
                if doc.sequence.len() == before {
                    return Err(format!("手順ID {id} が見つかりません"));
                }
            }
            SeqOp::UpdateStep { step } => {
                let Some(slot) = doc.sequence.iter_mut().find(|s| s.id == step.id) else {
                    return Err(format!("手順ID {} が見つかりません", step.id));
                };
                *slot = step;
            }
        }
        Ok(self.commit(doc, Vec::new()))
    }

    /// 直前の変更を取り消す。
    pub fn undo(&mut self) -> Result<DocumentView, String> {
        // 導出を先に済ませてからpop・swapする(導出panic時にstoreを変えない)
        let prev = self
            .undo_stack
            .last()
            .ok_or_else(|| "これ以上元に戻せません".to_string())?;
        let view = build_view(prev, Vec::new());
        let prev = self.undo_stack.pop().expect("直前にlastで確認済み");
        self.redo_stack.push(std::mem::replace(&mut self.doc, prev));
        self.dirty = true;
        Ok(view)
    }

    /// 取り消した変更をやり直す。
    pub fn redo(&mut self) -> Result<DocumentView, String> {
        let next = self
            .redo_stack
            .last()
            .ok_or_else(|| "これ以上やり直せません".to_string())?;
        let view = build_view(next, Vec::new());
        let next = self.redo_stack.pop().expect("直前にlastで確認済み");
        self.undo_stack.push(std::mem::replace(&mut self.doc, next));
        self.dirty = true;
        Ok(view)
    }

    /// 未保存の変更があるか。
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// 変更後Documentを確定する。変更が実際に起きた場合のみundo履歴に積む。
    ///
    /// 導出(validate/extract_faces)を候補docに対して先に実行し、成功した場合のみ
    /// 状態を入れ替える。導出がpanicしてもstoreは無変更のまま(guardがErr化し、
    /// 「Err⇒無変更」の不変条件を保つ)。
    fn commit(&mut self, doc: Document, warnings: Vec<String>) -> DocumentView {
        let view = build_view(&doc, warnings);
        if doc != self.doc {
            if self.undo_stack.len() >= MAX_UNDO {
                self.undo_stack.remove(0);
            }
            self.undo_stack.push(std::mem::replace(&mut self.doc, doc));
            self.redo_stack.clear();
            self.dirty = true;
        }
        view
    }
}

/// Documentから表示用ビューを作る(faces/warningsは毎回導出)。
fn build_view(doc: &Document, mut warnings: Vec<String>) -> DocumentView {
    warnings.extend(ori3_cp::validate(&doc.cp));
    DocumentView {
        doc: doc.clone(),
        faces: ori3_cp::extract_faces(&doc.cp),
        warnings,
        violations: Vec::new(),
    }
}

fn check_paper(paper: &Paper) -> Result<(), String> {
    if paper.width_mm > 0.0 && paper.height_mm > 0.0 {
        Ok(())
    } else {
        Err("紙のサイズは正の値で指定してください".to_string())
    }
}

fn is_border(cp: &CreasePattern, id: u32) -> bool {
    cp.edges
        .iter()
        .any(|e| e.id == id && e.kind == EdgeKind::Border)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ori3_model::{FoldStep, TechniqueKind};

    fn square_store() -> DocumentStore {
        let mut store = DocumentStore::default();
        store
            .new_document(Paper {
                width_mm: 150.0,
                height_mm: 150.0,
            })
            .unwrap();
        store
    }

    fn diagonal() -> EditOp {
        EditOp::AddSegment {
            a: [0.0, 0.0],
            b: [1.0, 1.0],
            kind: EdgeKind::Mountain,
        }
    }

    fn step(id: u32) -> FoldStep {
        FoldStep {
            id,
            kind: TechniqueKind::Simple,
            drivers: Vec::new(),
            layer_order: None,
            note: String::new(),
        }
    }

    #[test]
    fn add_segment_undo_redo_roundtrip() {
        let mut store = square_store();
        let initial = store.doc.clone();

        let view = store.apply_edit(diagonal()).unwrap();
        assert_eq!(view.doc.cp.edges.len(), 5);
        assert_eq!(view.faces.len(), 2);
        assert!(view.warnings.is_empty());
        let edited = store.doc.clone();
        assert_ne!(initial, edited);

        let view = store.undo().unwrap();
        assert_eq!(view.doc, initial);
        assert_eq!(store.doc, initial);

        let view = store.redo().unwrap();
        assert_eq!(view.doc, edited);
        assert_eq!(store.doc, edited);
    }

    #[test]
    fn undo_stack_capped_at_100_oldest_dropped() {
        let mut store = square_store();

        // 101回の編集(頂点0を毎回別の位置へ移動)
        let mut states = vec![store.doc.clone()]; // states[i] = i回編集後
        for i in 1..=101u32 {
            store
                .apply_edit(EditOp::MoveVertex {
                    id: 0,
                    to: [-0.001 * f64::from(i), -0.001 * f64::from(i)],
                })
                .unwrap();
            states.push(store.doc.clone());
        }
        assert_eq!(store.undo_stack.len(), MAX_UNDO);

        // 100回まで戻れる。最古(初期状態)は破棄済みなので、1回編集後の状態で止まる
        for _ in 0..MAX_UNDO {
            store.undo().unwrap();
        }
        assert_eq!(store.doc, states[1]);
        assert!(store.undo().is_err());
    }

    #[test]
    fn save_open_roundtrip() {
        let mut store = square_store();
        store.apply_edit(diagonal()).unwrap();
        store.apply_seq(SeqOp::PushStep { step: step(0) }).unwrap();
        let saved_doc = store.doc.clone();

        let path = std::env::temp_dir().join(format!(
            "ori3_store_test_{}_roundtrip.ori3",
            std::process::id()
        ));
        store.save(Some(&path)).unwrap();
        assert!(!store.is_dirty());

        let mut other = DocumentStore::default();
        let view = other.open(&path).unwrap();
        assert_eq!(view.doc, saved_doc);
        assert_eq!(other.doc, saved_doc);
        assert!(!other.is_dirty());

        // pathを覚えているのでNoneで上書き保存できる
        other.apply_edit(diagonal_reverse()).unwrap();
        other.save(None).unwrap();

        std::fs::remove_file(&path).ok();
    }

    fn diagonal_reverse() -> EditOp {
        EditOp::AddSegment {
            a: [1.0, 0.0],
            b: [0.0, 1.0],
            kind: EdgeKind::Valley,
        }
    }

    #[test]
    fn open_rejects_newer_schema_version() {
        let mut store = square_store();
        let path =
            std::env::temp_dir().join(format!("ori3_store_test_{}_newer.ori3", std::process::id()));
        store.save(Some(&path)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let bumped = text.replacen(
            &format!("\"schema_version\": {SCHEMA_VERSION}"),
            &format!("\"schema_version\": {}", SCHEMA_VERSION + 1),
            1,
        );
        assert_ne!(text, bumped, "schema_versionの置換に失敗");
        std::fs::write(&path, bumped).unwrap();

        let err = store.open(&path).unwrap_err();
        assert!(err.contains("新しい版"), "err={err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn duplicate_add_segment_does_not_push_undo() {
        let mut store = square_store();
        store.apply_edit(diagonal()).unwrap();
        assert_eq!(store.undo_stack.len(), 1);

        // 完全重複の再挿入: 成功扱いだが無変更なのでundo履歴に積まない
        let view = store.apply_edit(diagonal()).unwrap();
        assert_eq!(view.doc.cp.edges.len(), 5);
        assert_eq!(store.undo_stack.len(), 1);
    }

    #[test]
    fn new_edit_clears_redo_stack() {
        let mut store = square_store();
        store.apply_edit(diagonal()).unwrap();
        store.undo().unwrap();
        assert_eq!(store.redo_stack.len(), 1);
        store.apply_edit(diagonal_reverse()).unwrap();
        assert!(store.redo_stack.is_empty());
        assert!(store.redo().is_err());
    }

    #[test]
    fn remove_edges_keeps_border_with_warning() {
        let mut store = square_store();
        let view = store.apply_edit(diagonal()).unwrap();
        let mountain_id = view
            .doc
            .cp
            .edges
            .iter()
            .find(|e| e.kind == EdgeKind::Mountain)
            .unwrap()
            .id;

        // 輪郭辺(id=0)と山折り線を同時に削除指定 → 輪郭は残り、警告が出る
        let view = store
            .apply_edit(EditOp::RemoveEdges {
                ids: vec![0, mountain_id],
            })
            .unwrap();
        assert!(view.warnings.iter().any(|w| w.contains("輪郭線")));
        assert_eq!(view.doc.cp.edges.len(), 4);
        assert!(view.doc.cp.edges.iter().all(|e| e.kind == EdgeKind::Border));
    }

    #[test]
    fn set_edge_kind_protects_border_both_ways() {
        let mut store = square_store();
        let view = store.apply_edit(diagonal()).unwrap();
        let mountain_id = view
            .doc
            .cp
            .edges
            .iter()
            .find(|e| e.kind == EdgeKind::Mountain)
            .unwrap()
            .id;

        // 輪郭→山はwarningで無変更、山→谷は変更される
        let view = store
            .apply_edit(EditOp::SetEdgeKind {
                ids: vec![0, mountain_id],
                kind: EdgeKind::Valley,
            })
            .unwrap();
        assert!(view.warnings.iter().any(|w| w.contains("輪郭線")));
        assert_eq!(kind_of(&view, 0), EdgeKind::Border);
        assert_eq!(kind_of(&view, mountain_id), EdgeKind::Valley);

        // 谷→輪郭への変更は不可(warning)
        let view = store
            .apply_edit(EditOp::SetEdgeKind {
                ids: vec![mountain_id],
                kind: EdgeKind::Border,
            })
            .unwrap();
        assert!(view.warnings.iter().any(|w| w.contains("輪郭線")));
        assert_eq!(kind_of(&view, mountain_id), EdgeKind::Valley);
    }

    fn kind_of(view: &DocumentView, id: u32) -> EdgeKind {
        view.doc.cp.edges.iter().find(|e| e.id == id).unwrap().kind
    }

    #[test]
    fn set_paper_rejected_when_fold_lines_exist() {
        let mut store = square_store();
        store.apply_edit(diagonal()).unwrap();
        let err = store
            .apply_edit(EditOp::SetPaper {
                paper: Paper {
                    width_mm: 150.0,
                    height_mm: 100.0,
                },
            })
            .unwrap_err();
        assert!(err.contains("紙サイズを変更できません"), "err={err}");
        // Errはundo履歴に積まれない
        assert_eq!(store.undo_stack.len(), 1);
    }

    #[test]
    fn set_paper_rebuilds_when_border_only() {
        let mut store = square_store();
        store.apply_seq(SeqOp::PushStep { step: step(7) }).unwrap();
        let view = store
            .apply_edit(EditOp::SetPaper {
                paper: Paper {
                    width_mm: 150.0,
                    height_mm: 100.0,
                },
            })
            .unwrap();
        // 長辺=1.0の正規化で作り直され、sequenceは維持される
        assert_eq!(view.doc.paper.height_mm, 100.0);
        let ymax = view
            .doc
            .cp
            .vertices
            .iter()
            .map(|v| v.pos[1])
            .fold(0.0, f64::max);
        assert!((ymax - 100.0 / 150.0).abs() < 1e-12);
        assert_eq!(view.doc.sequence.len(), 1);
    }

    #[test]
    fn seq_ops_apply_and_undo() {
        let mut store = square_store();
        store.apply_seq(SeqOp::PushStep { step: step(0) }).unwrap();
        store
            .apply_seq(SeqOp::InsertStep {
                index: 0,
                step: step(1),
            })
            .unwrap();
        let mut updated = step(1);
        updated.note = "折り筋をつける".to_string();
        let view = store
            .apply_seq(SeqOp::UpdateStep { step: updated })
            .unwrap();
        assert_eq!(view.doc.sequence[0].note, "折り筋をつける");

        let view = store.apply_seq(SeqOp::RemoveStep { id: 0 }).unwrap();
        assert_eq!(view.doc.sequence.len(), 1);

        assert!(store.apply_seq(SeqOp::RemoveStep { id: 99 }).is_err());
        assert!(
            store
                .apply_seq(SeqOp::InsertStep {
                    index: 5,
                    step: step(2)
                })
                .is_err()
        );

        // undo 3回で空のsequenceまで戻る
        store.undo().unwrap();
        store.undo().unwrap();
        store.undo().unwrap();
        let view = store.undo().unwrap();
        assert!(view.doc.sequence.is_empty());
    }

    #[test]
    fn broken_cp_replace_does_not_poison_store_or_file() {
        let mut store = square_store();
        // 参照切れ辺(存在しない頂点999を参照)を含むCPを流し込む
        let mut broken = store.doc.cp.clone();
        broken.edges.push(ori3_model::Edge {
            id: broken.next_edge_id,
            v0: 0,
            v1: 999,
            kind: EdgeKind::Mountain,
        });
        broken.next_edge_id += 1;

        // 成功し、参照切れの警告が返る(panicしない)
        let view = store
            .apply_edit(EditOp::ReplaceCreasePattern { cp: broken })
            .unwrap();
        assert!(
            view.warnings.iter().any(|w| w.contains("存在しない点")),
            "warnings={:?}",
            view.warnings
        );

        // 続けて別の編集も成功する
        store.apply_edit(diagonal()).unwrap();

        // save→openの往復も成功する(二度と開けないファイルを作らない)
        let path = std::env::temp_dir().join(format!(
            "ori3_store_test_{}_broken.ori3",
            std::process::id()
        ));
        store.save(Some(&path)).unwrap();
        let mut other = DocumentStore::default();
        let view = other.open(&path).unwrap();
        assert_eq!(view.doc, store.doc);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn open_reports_clear_errors_for_bad_input() {
        let mut store = square_store();
        let before = store.doc.clone();

        // ファイル不在
        let missing = std::env::temp_dir().join("ori3_store_test_no_such_file.ori3");
        let err = store.open(&missing).unwrap_err();
        assert!(err.contains("ファイルを開けませんでした"), "err={err}");

        // 不正JSON
        let path = std::env::temp_dir().join(format!(
            "ori3_store_test_{}_badjson.ori3",
            std::process::id()
        ));
        std::fs::write(&path, "これはJSONではない{{{").unwrap();
        let err = store.open(&path).unwrap_err();
        assert!(err.contains("読み取れませんでした"), "err={err}");

        // 失敗したopenはstoreを変えない
        assert_eq!(store.doc, before);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn move_vertex_missing_id_warns_without_change() {
        let mut store = square_store();
        let view = store
            .apply_edit(EditOp::MoveVertex {
                id: 999,
                to: [0.5, 0.5],
            })
            .unwrap();
        assert!(
            view.warnings.iter().any(|w| w.contains("頂点ID 999")),
            "warnings={:?}",
            view.warnings
        );
        // 無変更なのでundo履歴に積まれない
        assert!(store.undo_stack.is_empty());
    }

    #[test]
    fn move_vertex_reports_validate_warnings() {
        let mut store = square_store();
        store.apply_edit(diagonal()).unwrap();
        // 頂点0を対辺の外へ動かして交差を作っても、Errにはならず警告が返る
        let view = store
            .apply_edit(EditOp::MoveVertex {
                id: 0,
                to: [1.5, 0.5],
            })
            .unwrap();
        assert!(!view.warnings.is_empty());
    }
}
