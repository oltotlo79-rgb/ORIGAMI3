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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ori3_cp::Face;
use ori3_model::{
    CreasePattern, Document, EdgeId, EdgeKind, EditOp, Frame3D, MAX_GRID_DIVISIONS,
    MIN_GRID_DIVISIONS, Paper, SCHEMA_VERSION, SeqOp, StepId, TechniqueKind, VertexId,
};

/// undo履歴の最大件数。超過時は最古をFIFOで破棄する。
const MAX_UNDO: usize = 100;

/// 同じ場所に一時ファイルを書いてから名前を入れ替えるための連番。
static ATOMIC_WRITE_ID: AtomicU64 = AtomicU64::new(0);

/// 同一ディレクトリ内の一時ファイル経由でファイルを置き換える。
/// 書き込み中に止まっても、既存の完成ファイルを途中の内容で壊さない。
pub(crate) fn write_atomic(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("ori3");
    let id = ATOMIC_WRITE_ID.fetch_add(1, Ordering::Relaxed);
    let temp = dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), id));
    std::fs::write(&temp, bytes)?;
    if let Err(err) = std::fs::rename(&temp, target) {
        let _ = std::fs::remove_file(&temp);
        return Err(err);
    }
    Ok(())
}

/// フロントへ返す表示用ビュー(Document全体 + 導出情報)。
/// save以外の全コマンドの成功時戻り値。
#[derive(Clone, Debug, serde::Serialize)]
pub struct DocumentView {
    pub doc: Document,
    pub faces: Vec<Face>,
    /// 操作固有の警告 + `ori3_cp::validate` + 手順再生の警告(「止めずに警告」原則)
    pub warnings: Vec<String>,
    /// 平らに畳めない疑いのある点(前川定理・川崎定理を満たさない内部頂点)。
    /// 操作は止めず、2D画面で色を変えて知らせるだけ(CPE-009)
    pub violations: Vec<VertexId>,
    /// 最新ステップまで自動再生した立体(SEQ-004)。手順が空ならNone
    pub frame: Option<Frame3D>,
    /// 自動再生で折り線が見つからず飛ばされたステップのID
    pub skipped: Vec<StepId>,
}

pub struct DocumentStore {
    doc: Document,
    /// 現docに対応する導出faces(pose_solveが毎回extract_facesを再実行しない
    /// ためのキャッシュ)。docの変更経路はstore内(new_document/open/commit/
    /// undo/redo)に閉じているため、その全箇所で更新すれば整合が保たれる
    faces: Vec<Face>,
    undo_stack: Vec<Document>,
    redo_stack: Vec<Document>,
    dirty: bool,
    path: Option<PathBuf>,
    /// pose_solveの前回解(次回のwarm start用)。ソルバーは知らない辺IDを
    /// 無視するため、CP編集後に古い解が残っていても安全
    pose_angles: Option<HashMap<EdgeId, f64>>,
}

impl Default for DocumentStore {
    /// 起動直後の初期状態(150mm正方形の新規作品)。
    fn default() -> Self {
        let doc = Document::new(Paper {
            width_mm: 150.0,
            height_mm: 150.0,
        });
        DocumentStore {
            faces: ori3_cp::extract_faces(&doc.cp),
            doc,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            dirty: false,
            path: None,
            pose_angles: None,
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
        self.faces = view.faces.clone();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.dirty = false;
        self.path = None;
        self.pose_angles = None;
        Ok(view)
    }

    /// `.ori3`ファイル(pretty JSON)を読み込む。schema_version不一致はErr。
    pub fn open(&mut self, path: &Path) -> Result<DocumentView, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("ファイルを開けませんでした: {e}"))?;
        let doc = parse_document(&text)?;
        // 導出を先に済ませ、成功した場合のみ状態を確定する
        let view = build_view(&doc, Vec::new());
        self.doc = doc;
        self.faces = view.faces.clone();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.dirty = false;
        self.path = Some(path.to_path_buf());
        self.pose_angles = None;
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
        write_atomic(&target, json.as_bytes()).map_err(|e| format!("保存に失敗しました: {e}"))?;
        self.path = Some(target);
        self.dirty = false;
        Ok(())
    }

    /// 編集操作を適用する。実際に変更が起きた場合のみundo履歴に積む。
    pub fn apply_edit(&mut self, op: EditOp) -> Result<DocumentView, String> {
        let replaced_crease_pattern = matches!(&op, EditOp::ReplaceCreasePattern { .. });
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
                // 別の展開図へ付けた手順を残すと、たまたま同じIDの無関係な線を
                // 折ってしまう。置換は新規作品と同じく手順を持たない状態から始める。
                doc.sequence.clear();
            }
            EditOp::SetDisplay { mut display } => {
                // 色は[u8;3]なので0〜255は型が保証する(範囲外はIPCの読み取りで弾かれる)。
                // 分割数だけは範囲外を丸めて続ける(止めずに警告)
                let n = display.grid_divisions;
                if !(MIN_GRID_DIVISIONS..=MAX_GRID_DIVISIONS).contains(&n) {
                    display.grid_divisions = n.clamp(MIN_GRID_DIVISIONS, MAX_GRID_DIVISIONS);
                    warnings.push(format!(
                        "方眼の数は{MIN_GRID_DIVISIONS}〜{MAX_GRID_DIVISIONS}の範囲で指定してください({n}は{}に丸めました)",
                        display.grid_divisions
                    ));
                }
                doc.display = display;
            }
        }
        let view = self.commit(doc, warnings);
        if replaced_crease_pattern {
            // CP全置換前の解は辺IDが偶然一致しても使ってはいけない。
            self.pose_angles = None;
        }
        Ok(view)
    }

    /// 折り手順操作を適用する。実際に変更が起きた場合のみundo履歴に積む。
    pub fn apply_seq(&mut self, op: SeqOp) -> Result<DocumentView, String> {
        let mut doc = self.doc.clone();
        let mut warnings: Vec<String> = Vec::new();
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
            SeqOp::FoldThrough {
                up_to,
                line,
                keep_side_point,
                target_layers,
                direction,
            } => {
                let mut insert_warnings = check_insert_point(&doc, up_to)?;
                // facesは現docから導出済みのキャッシュ(docはまだ複製したまま無変更)
                // 現在の状態を求め直すときの警告(飛ばした手順など)も利用者へ返す
                let (state, state_warnings) = ori3_layers::flat_state_at(&doc, &self.faces, up_to)?;
                let mut cp = doc.cp.clone();
                let result = ori3_layers::fold_through(
                    &mut cp,
                    &self.faces,
                    &state,
                    &ori3_layers::FoldThroughInput {
                        line,
                        keep_side_point,
                        target_layers,
                        direction,
                    },
                )?;
                let mut step = result.step;
                step.id = next_step_id(&doc);
                doc.cp = cp;
                doc.sequence.insert(up_to, step);
                warnings = state_warnings;
                warnings.append(&mut insert_warnings);
                warnings.extend(result.warnings);
            }
            SeqOp::Technique {
                up_to,
                kind,
                flap,
                line,
                reference_point,
                open_to_back,
                polygon,
                center,
            } => {
                // FoldThroughと同じ規約(途中への挿入も可。後続手順は再生時に検査される)
                let mut insert_warnings = check_insert_point(&doc, up_to)?;
                let technique = match kind {
                    TechniqueKind::Pleat => ori3_layers::pleat,
                    TechniqueKind::InsideReverse => ori3_layers::inside_reverse,
                    TechniqueKind::OutsideReverse => ori3_layers::outside_reverse,
                    TechniqueKind::Squash => ori3_layers::squash,
                    TechniqueKind::Petal => ori3_layers::petal,
                    TechniqueKind::OpenSink => ori3_layers::open_sink,
                    TechniqueKind::Swivel => ori3_layers::swivel,
                    TechniqueKind::Twist => ori3_layers::twist,
                    _ => {
                        return Err(
                            "この折り方はまだ選べません。手動の折り操作で代替してください"
                                .to_string(),
                        );
                    }
                };
                let (state, state_warnings) = ori3_layers::flat_state_at(&doc, &self.faces, up_to)?;
                let mut cp = doc.cp.clone();
                let result = technique(
                    &mut cp,
                    &self.faces,
                    &state,
                    &ori3_layers::TechniqueInput {
                        flap,
                        line,
                        reference_point,
                        open_to_back,
                        polygon,
                        center,
                    },
                )?;
                let mut step = result.step;
                step.id = next_step_id(&doc);
                doc.cp = cp;
                doc.sequence.insert(up_to, step);
                warnings = state_warnings;
                warnings.append(&mut insert_warnings);
                warnings.extend(result.warnings);
            }
        }
        Ok(self.commit(doc, warnings))
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
        self.faces = view.faces.clone();
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
        self.faces = view.faces.clone();
        self.dirty = true;
        Ok(view)
    }

    /// 未保存の変更があるか。
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// 現在の保存先(未保存の新規作品ならNone)。
    pub fn current_path(&self) -> Option<PathBuf> {
        self.path.clone()
    }

    /// 自動保存の材料(保存先と現Documentの複製)を取り出す。未保存の変更が
    /// 無ければNone(SYS-003: 変更が無いときは書かない)。
    ///
    /// 設計規約: `save` を自動保存に流用してはいけない。`save` は保存先パスと
    /// 未保存フラグを書き換えるため、自動保存ファイルへ書くと本来の保存先が
    /// 乗っ取られ、未保存の印も消えてしまう。ここは複製を返すだけで
    /// `path`/`dirty` を触らず、書き出しはロックの外(autosave.rs)で行う。
    pub fn autosave_snapshot(&self) -> Option<(Option<PathBuf>, Document)> {
        if !self.dirty {
            return None;
        }
        Some((self.path.clone(), self.doc.clone()))
    }

    /// 自動保存から読んだDocumentを現在の作品にする(復元)。
    /// 元の保存先を引き継ぎ、まだ書き出していない内容なので未保存扱いにする。
    pub fn restore(&mut self, doc: Document, path: Option<PathBuf>) -> DocumentView {
        // 導出を先に済ませてから状態を確定する(openと同じ規約)
        let view = build_view(&doc, Vec::new());
        self.doc = doc;
        self.faces = view.faces.clone();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.dirty = true;
        self.path = path;
        self.pose_angles = None;
        view
    }

    /// pose_solveの入力(CP・導出済みfaces・前回解・重なり防止設定)を取り出す。
    /// facesは編集時に導出済みのキャッシュの流用で、extract_facesを再実行しない。
    /// 設計規約: ロック中に重い計算をしないため、コマンド層はこの複製を取って
    /// 即ロックを解放し、solveはロックの外で実行する。
    pub fn pose_inputs(&self) -> (CreasePattern, Vec<Face>, Option<HashMap<EdgeId, f64>>, bool) {
        (
            self.doc.cp.clone(),
            self.faces.clone(),
            self.pose_angles.clone(),
            self.doc.display.overlap_prevention_enabled,
        )
    }

    /// sequence_replayの入力(Documentと導出済みfacesの複製)を取り出す。
    /// facesは編集時に導出済みのキャッシュの流用で、extract_facesを再実行しない。
    /// 設計規約: pose_inputsと同じく、コマンド層はこの複製を取って即ロックを解放し、
    /// 再生(重い計算)はロックの外で実行する。
    pub fn replay_inputs(&self) -> (Document, Vec<Face>) {
        (self.doc.clone(), self.faces.clone())
    }

    /// document_exportの入力(現Documentの複製)を取り出す。
    /// 設計規約: replay_inputsと同じく、コマンド層はこの複製を取って即ロックを解放し、
    /// 図の組み立てとファイル書き出し(I/O)はロックの外で行う。
    pub fn export_inputs(&self) -> Document {
        self.doc.clone()
    }

    /// pose_solveの結果角度を保存する(次回のwarm start用)。
    pub fn store_pose_angles(&mut self, angles: HashMap<EdgeId, f64>) {
        self.pose_angles = Some(angles);
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
            self.faces = view.faces.clone();
            self.redo_stack.clear();
            self.dirty = true;
        }
        view
    }
}

/// 保存されたJSONをDocumentへ戻す。schema_versionが合わなければErr。
/// `open` と自動保存の復元(autosave.rs)で共通に使う。
pub fn parse_document(text: &str) -> Result<Document, String> {
    let value: serde_json::Value = serde_json::from_str(text)
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
    serde_json::from_value(value).map_err(|e| format!("ファイルの内容を読み取れませんでした: {e}"))
}

/// Documentから表示用ビューを作る(faces/warningsは毎回導出)。
/// 立体(`frame`)は入れない。重い手順再生はロックの外で `attach_replay` が行う。
fn build_view(doc: &Document, mut warnings: Vec<String>) -> DocumentView {
    warnings.extend(ori3_cp::validate(&doc.cp));
    DocumentView {
        doc: doc.clone(),
        faces: ori3_cp::extract_faces(&doc.cp),
        warnings,
        violations: ori3_cp::local_violations(&doc.cp),
        frame: None,
        skipped: Vec::new(),
    }
}

/// ビューへ手順の自動再生結果(立体・飛ばした手順・警告)を載せる
/// (SEQ-004「展開図編集後、手順を自動再生して最新状態を表示」)。
/// 手順が空のときは再生するものが無いので `frame: None` のまま
/// (平らな姿勢はフロントが展開図から直接描ける)。
///
/// 設計規約: これは重い計算(面400・10手順でrelease約23ms)なので、
/// storeのロックを取らないコマンド層から、ロック解放後に呼ぶ。
/// 再生には `view.faces`(同じdocから導出済み)を渡し、面抽出を二重に行わない。
pub fn attach_replay(view: &mut DocumentView) {
    if view.doc.sequence.is_empty() {
        return;
    }
    let up_to = view.doc.sequence.len();
    let mut replayed = ori3_layers::replay_with_faces(&view.doc, &view.faces, up_to, 1.0);
    let transition = replayed.layer_transition.clone();
    ori3_soft::prevent_overlap(
        &view.doc.cp,
        &view.faces,
        &mut replayed.frame,
        &transition.start,
        &transition.end,
        transition.progress,
        &ori3_soft::OverlapSettings {
            enabled: view.doc.display.overlap_prevention_enabled,
            ..Default::default()
        },
    );
    // 紙のめり込み(SIM-007)。折り上がりは平ら(z≒0)なので通常は出ないが、
    // 平らに畳みきれない形では立体のまま返るため、そのときに知らせる
    if add_penetration_warning(&mut replayed.frame) {
        replayed
            .warnings
            .push(ori3_rigid::PENETRATION_WARNING.to_string());
    }
    view.warnings.extend(replayed.warnings);
    view.skipped = replayed.skipped;
    view.frame = Some(replayed.frame);
}

/// 立体の面同士が食い込んでいれば、フレームの警告に一文を足す(SIM-007)。
/// 厳密な防止はせず、気づけるようにするだけ(「止めずに警告」原則)。
/// 平らに畳んだ状態(z≒0)では層が同一平面に重なるのが正常なので何もしない。
pub fn add_penetration_warning(frame: &mut Frame3D) -> bool {
    if ori3_rigid::self_intersects(frame) {
        frame
            .warnings
            .push(ori3_rigid::PENETRATION_WARNING.to_string());
        return true;
    }
    false
}

fn check_paper(paper: &Paper) -> Result<(), String> {
    if paper.width_mm > 0.0 && paper.height_mm > 0.0 {
        Ok(())
    } else {
        Err("紙のサイズは正の値で指定してください".to_string())
    }
}

/// 折り操作([`SeqOp::FoldThrough`] / [`SeqOp::Technique`])の挿入位置を検査する。
///
/// `up_to` は「この折りの直前までの手順数」で、途中の値も許す(手順の途中へ折りを
/// 挟める)。挟めるのは、手順の永続化が面IDや辺IDではなく幾何(折り線の線分・層順序の
/// 代表点)で行われているため。挿入で既存の折り線が分割されても
/// `resolve_driver_edges` が断片を全て拾い、層順序の代表点も現在の面へ解決し直される。
/// それでも後続の手順が成り立たなくなることはあり得るが、その場合は再生側が
/// 手順を飛ばして警告を出す(「止めずに警告」原則)。
///
/// 戻り値は利用者へ返す警告(末尾への追加なら空)。
fn check_insert_point(doc: &Document, up_to: usize) -> Result<Vec<String>, String> {
    let len = doc.sequence.len();
    if up_to > len {
        return Err(format!("挿入位置 {up_to} が手順の数を超えています"));
    }
    if up_to == len {
        return Ok(Vec::new());
    }
    Ok(vec![format!(
        "手順{}の前に折りを挟みました。後ろの手順{}個は折り直した形の上で再生し直しています(合わなくなった手順は飛ばして知らせます)",
        up_to + 1,
        len - up_to
    )])
}

/// 新しい手順に振るID(既存の最大+1)。手順を消しても再利用しない。
fn next_step_id(doc: &Document) -> StepId {
    doc.sequence
        .iter()
        .map(|s| s.id)
        .max()
        .map_or(0, |m| m.saturating_add(1))
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

    /// 紙の色と方眼の数は作品(Document::display)に保存され、undo/redoで戻せる。
    /// 方眼の数が範囲外なら丸めて警告する(止めない)。
    #[test]
    fn set_display_saves_into_document_and_is_undoable() {
        let mut store = square_store();
        let before = store.doc.display.clone();

        let view = store
            .apply_edit(EditOp::SetDisplay {
                display: ori3_model::DisplaySettings {
                    front_color: [0, 128, 255],
                    back_color: [16, 16, 16],
                    grid_divisions: 16,
                    ..Default::default()
                },
            })
            .unwrap();
        assert_eq!(view.doc.display.front_color, [0, 128, 255]);
        assert_eq!(view.doc.display.back_color, [16, 16, 16]);
        assert_eq!(view.doc.display.grid_divisions, 16);
        assert!(view.warnings.is_empty(), "warnings={:?}", view.warnings);
        assert!(store.is_dirty(), "作品が変わったので未保存になる");

        // 範囲外(0と100)は丸めて警告する
        let view = store
            .apply_edit(EditOp::SetDisplay {
                display: ori3_model::DisplaySettings {
                    grid_divisions: 100,
                    ..view.doc.display.clone()
                },
            })
            .unwrap();
        assert_eq!(view.doc.display.grid_divisions, MAX_GRID_DIVISIONS);
        assert!(
            view.warnings.iter().any(|w| w.contains("方眼の数")),
            "warnings={:?}",
            view.warnings
        );
        let view = store
            .apply_edit(EditOp::SetDisplay {
                display: ori3_model::DisplaySettings {
                    grid_divisions: 0,
                    ..view.doc.display.clone()
                },
            })
            .unwrap();
        assert_eq!(view.doc.display.grid_divisions, MIN_GRID_DIVISIONS);

        // undoを3回で元の見た目へ戻る
        for _ in 0..3 {
            store.undo().unwrap();
        }
        assert_eq!(store.doc.display, before);
        assert_eq!(store.redo().unwrap().doc.display.front_color, [0, 128, 255]);
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
    fn pose_angles_roundtrip_and_cleared_on_new() {
        let mut store = square_store();
        assert_eq!(store.pose_inputs().2, None);

        store.store_pose_angles(HashMap::from([(6u32, 90.0f64)]));
        let (cp, faces, warm, overlap_enabled) = store.pose_inputs();
        assert_eq!(cp, store.doc.cp);
        assert_eq!(faces.len(), 1, "正方形1面のはず");
        assert_eq!(warm, Some(HashMap::from([(6u32, 90.0f64)])));
        assert!(overlap_enabled, "重なり防止は既定オン");

        // 新規作成で前回解は破棄される(別のCPに古い解を引き継がない)
        store
            .new_document(Paper {
                width_mm: 100.0,
                height_mm: 100.0,
            })
            .unwrap();
        assert_eq!(store.pose_inputs().2, None);
    }

    #[test]
    fn replacing_crease_pattern_clears_steps_and_warm_start() {
        let mut store = square_store();
        store.apply_edit(diagonal()).unwrap();
        store.apply_seq(SeqOp::PushStep { step: step(7) }).unwrap();
        store.store_pose_angles(HashMap::from([(4u32, 120.0f64)]));

        let replacement = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        })
        .cp;
        let view = store
            .apply_edit(EditOp::ReplaceCreasePattern { cp: replacement })
            .unwrap();

        assert!(view.doc.sequence.is_empty(), "手順が残っている");
        assert!(store.doc.sequence.is_empty(), "storeにも手順が残っている");
        assert!(view.frame.is_none(), "手順の3D結果を持ち越さない");
        assert_eq!(store.pose_inputs().2, None, "暖機用の角度を持ち越さない");
    }

    /// facesキャッシュがdocの全変更経路(編集・undo・redo)で追従することの検証。
    /// pose_inputsの返すfacesは常に現doc由来のextract_faces結果と一致する。
    #[test]
    fn pose_inputs_faces_cache_follows_all_doc_changes() {
        let mut store = square_store();
        let fresh = |s: &DocumentStore| ori3_cp::extract_faces(&s.doc.cp);

        store.apply_edit(diagonal()).unwrap();
        assert_eq!(store.pose_inputs().1, fresh(&store), "編集後");

        store.undo().unwrap();
        assert_eq!(store.pose_inputs().1, fresh(&store), "undo後");

        store.redo().unwrap();
        assert_eq!(store.pose_inputs().1, fresh(&store), "redo後");

        store
            .new_document(Paper {
                width_mm: 100.0,
                height_mm: 100.0,
            })
            .unwrap();
        assert_eq!(store.pose_inputs().1, fresh(&store), "新規作成後");
    }

    /// SEQ-004: 編集後のビューに手順の自動再生結果が載る(コマンド層がロック解放後に
    /// `attach_replay` を呼ぶ想定)。手順が参照する折り線を消すとそのステップは飛ばされる。
    #[test]
    fn attach_replay_adds_frame_and_skipped_steps() {
        let mut store = square_store();
        // 手順が無いうちは再生するものが無い
        let mut view = store.apply_edit(diagonal()).unwrap();
        attach_replay(&mut view);
        assert!(view.frame.is_none());
        assert!(view.skipped.is_empty());

        // 対角線(山)を±180°まで折る手順を1つ積む
        let mut folding = step(0);
        folding.drivers = vec![ori3_model::DriverLine {
            a: [0.0, 0.0],
            b: [1.0, 1.0],
            target_angle_deg: 180.0,
        }];
        let mut view = store.apply_seq(SeqOp::PushStep { step: folding }).unwrap();
        // storeはロック内で呼ばれるので立体を載せない(重い計算はロックの外)
        assert!(view.frame.is_none(), "storeは再生しない");
        attach_replay(&mut view);
        let frame = view.frame.clone().expect("手順があれば自動再生される");
        assert_eq!(frame.faces.len(), 2);
        assert!(view.skipped.is_empty(), "warnings={:?}", view.warnings);
        let mut layers: Vec<u32> = frame.faces.iter().map(|f| f.layer).collect();
        layers.sort_unstable();
        assert_eq!(layers, vec![0, 1]);

        // 手順が参照する折り線を消すと、そのステップは飛ばされる(以降も止めない)
        let mountain = store
            .doc
            .cp
            .edges
            .iter()
            .find(|e| e.kind == EdgeKind::Mountain)
            .unwrap()
            .id;
        let mut view = store
            .apply_edit(EditOp::RemoveEdges {
                ids: vec![mountain],
            })
            .unwrap();
        attach_replay(&mut view);
        assert_eq!(view.skipped, vec![0]);
        assert!(
            view.warnings.iter().any(|w| w.contains("飛ばしました")),
            "warnings={:?}",
            view.warnings
        );
        assert!(view.frame.is_some(), "飛ばしても平らな立体は返す");
    }

    /// 畳んだ状態への折り操作(SeqOp::FoldThrough)で、展開図・手順・層が更新される。
    /// 2回目は1回目の結果(畳み平面)の上に折る。
    #[test]
    fn fold_through_updates_cp_sequence_and_layers() {
        let mut store = square_store();
        let view = store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .unwrap();
        // 層番号(下から0)が画面の重なりと一致している(根面が動く1手目)
        assert_display_layers(&store, "1手目");

        // 展開図に谷折り線が1本増え、面が2つに分かれる
        // (横切られた輪郭線2本も分割されるので、辺の総数は4→7)
        assert_eq!(view.doc.cp.edges.len(), 7);
        assert_eq!(
            view.doc
                .cp
                .edges
                .iter()
                .filter(|e| e.kind == EdgeKind::Valley)
                .count(),
            1
        );
        assert_eq!(view.faces.len(), 2);
        // 手順が1つ増え、折り線と層順序が記録されている
        assert_eq!(view.doc.sequence.len(), 1);
        let step = &view.doc.sequence[0];
        assert_eq!(step.id, 0);
        assert_eq!(step.kind, TechniqueKind::Simple);
        assert!(!step.drivers.is_empty());
        assert_eq!(step.layer_order.as_ref().map(Vec::len), Some(2));

        // 2回目: 畳んだ紙(半分の大きさ)を横線で折ると4層になる
        let mut view = store
            .apply_seq(fold_op(1, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]))
            .unwrap();
        assert_eq!(view.doc.sequence.len(), 2);
        assert_eq!(view.doc.sequence[1].id, 1, "手順IDは既存の最大+1");
        assert_eq!(view.faces.len(), 4);
        attach_replay(&mut view);
        let frame = view.frame.clone().expect("手順があるので立体が返る");
        let mut layers: Vec<u32> = frame.faces.iter().map(|f| f.layer).collect();
        layers.sort_unstable();
        assert_eq!(layers, vec![0, 1, 2, 3], "層番号は下から0,1,2,3");
        assert!(view.skipped.is_empty(), "warnings={:?}", view.warnings);
        assert_display_layers(&store, "2手目");

        // undoで折る前(手順1つ・面2つ)へ戻る
        let view = store.undo().unwrap();
        assert_eq!(view.doc.sequence.len(), 1);
        assert_eq!(view.faces.len(), 2);
    }

    /// 手順の途中(末尾以外)へも折り操作を挟める。挟んだ折りはその位置に入り、
    /// 後続の手順は残ったまま再生し直される(SEQ-006)。
    #[test]
    fn fold_through_inserts_in_the_middle_and_keeps_later_steps() {
        let mut store = square_store();
        // 手順1: 縦半分に折る
        store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .unwrap();
        let first_id = store.doc.sequence[0].id;

        // 手順1の前(up_to=0)へ横半分の折りを挟む
        let mut view = store
            .apply_seq(fold_op(0, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]))
            .unwrap();
        assert_eq!(view.doc.sequence.len(), 2, "手順が1つ増える");
        assert_eq!(
            view.doc.sequence[1].id, first_id,
            "元の手順は後ろへ押し出される"
        );
        assert!(
            view.warnings
                .iter()
                .any(|w| w.contains("前に折りを挟みました")),
            "途中挿入は警告で知らせる: {:?}",
            view.warnings
        );
        // 後続の手順は消えない(再生できなければ飛ばして警告するだけ)
        attach_replay(&mut view);
        assert_eq!(view.doc.sequence.len(), 2);

        // undoで挟む前へ戻る
        let view = store.undo().unwrap();
        assert_eq!(view.doc.sequence.len(), 1);
        assert_eq!(view.doc.sequence[0].id, first_id);
    }

    /// 手順の数を超える挿入位置はErr(文書は無変更)。
    #[test]
    fn fold_through_rejects_out_of_range_insert_point() {
        let mut store = square_store();
        let before = store.doc.clone();
        let err = store
            .apply_seq(fold_op(3, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]))
            .unwrap_err();
        assert!(err.contains("手順の数を超えています"), "err={err}");
        assert_eq!(store.doc, before, "Errのとき文書は変わらない");
    }

    /// 折れない指定(どの層も横切らない線)は理由を返し、展開図も手順も変えない。
    #[test]
    fn fold_through_failure_leaves_document_unchanged() {
        let mut store = square_store();
        let before = store.doc.clone();
        let err = store
            .apply_seq(fold_op(0, [[2.0, 0.0], [2.0, 1.0]], [1.5, 0.5]))
            .unwrap_err();
        assert!(err.contains("折り線"), "err={err}");
        assert_eq!(store.doc, before);
        assert!(store.undo_stack.is_empty(), "Errはundo履歴に積まれない");
    }

    /// 技法(SeqOp::Technique)で段折りができ、展開図・手順・層が更新される。
    #[test]
    fn technique_pleat_updates_cp_sequence_and_layers() {
        let mut store = square_store();
        let mut view = store
            .apply_seq(SeqOp::Technique {
                up_to: 0,
                kind: TechniqueKind::Pleat,
                flap: Vec::new(),
                line: [[0.4, 0.0], [0.4, 1.0]],
                reference_point: [0.5, 0.5],
                open_to_back: None,
                polygon: None,
                center: None,
            })
            .unwrap();
        // 折り線2本で面が3つに分かれ、手順が1つ増える
        assert_eq!(view.faces.len(), 3);
        assert_eq!(view.doc.sequence.len(), 1);
        let step = &view.doc.sequence[0];
        assert_eq!(step.kind, TechniqueKind::Pleat);
        assert_eq!(step.drivers.len(), 2);
        assert_eq!(step.layer_order.as_ref().map(Vec::len), Some(3));
        // 山と谷が1本ずつ(段折りは山谷が交互になる)
        let kinds: Vec<EdgeKind> = view
            .doc
            .cp
            .edges
            .iter()
            .map(|e| e.kind)
            .filter(|k| *k != EdgeKind::Border)
            .collect();
        assert_eq!(
            kinds.iter().filter(|k| **k == EdgeKind::Mountain).count(),
            1
        );
        assert_eq!(kinds.iter().filter(|k| **k == EdgeKind::Valley).count(), 1);
        attach_replay(&mut view);
        assert!(view.skipped.is_empty(), "warnings={:?}", view.warnings);

        // undoで折る前へ戻る
        let view = store.undo().unwrap();
        assert!(view.doc.sequence.is_empty());
        assert_eq!(view.faces.len(), 1);
    }

    /// 中割り折りは畳んだ状態(2層のフラップ)に対して適用できる。
    #[test]
    fn technique_inside_reverse_folds_a_two_layer_flap() {
        let mut store = square_store();
        // 下ごしらえ: 半分に折って2層にする
        store
            .apply_seq(fold_op(0, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]))
            .unwrap();
        let flap: Vec<u32> = store.faces.iter().map(|f| f.id).collect();
        assert_eq!(flap.len(), 2);

        let view = store
            .apply_seq(SeqOp::Technique {
                up_to: 1,
                kind: TechniqueKind::InsideReverse,
                flap,
                line: [[0.7, 0.5], [0.5, 0.0]],
                reference_point: [0.2, 0.25],
                open_to_back: None,
                polygon: None,
                center: None,
            })
            .unwrap();
        assert_eq!(view.faces.len(), 4, "2層が4層になる");
        assert_eq!(view.doc.sequence.len(), 2);
        assert_eq!(view.doc.sequence[1].kind, TechniqueKind::InsideReverse);
        assert_eq!(view.doc.sequence[1].drivers.len(), 3);
    }

    /// 開いてつぶす折りは畳んだ状態(2層のフラップ)に対して適用できる。
    #[test]
    fn technique_squash_opens_and_flattens_a_two_layer_flap() {
        let mut store = square_store();
        // 下ごしらえ: 半分に折って2層にする(背は y=0.5)
        store
            .apply_seq(fold_op(0, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]))
            .unwrap();
        let flap: Vec<u32> = store.faces.iter().map(|f| f.id).collect();
        assert_eq!(flap.len(), 2);

        // 背の右端(1,0.5)を支点に、背を左から左下へ45°回してつぶす
        let d = 0.5 * std::f64::consts::SQRT_2;
        let view = store
            .apply_seq(SeqOp::Technique {
                up_to: 1,
                kind: TechniqueKind::Squash,
                flap,
                line: [[0.0, 0.5], [1.0, 0.5]],
                reference_point: [1.0 - d, 0.5 - d],
                open_to_back: None,
                polygon: None,
                center: None,
            })
            .unwrap();
        assert_eq!(view.faces.len(), 3, "手前の層が分かれて3層になる");
        assert_eq!(view.doc.sequence.len(), 2);
        assert_eq!(view.doc.sequence[1].kind, TechniqueKind::Squash);
        assert!(
            view.doc.sequence[1]
                .drivers
                .iter()
                .any(|d| d.target_angle_deg == 0.0),
            "開いた背が0°で記録される"
        );
    }

    /// 花弁折りは畳んだ状態(2層のフラップ)に対して適用できる。
    #[test]
    fn technique_petal_lifts_the_tip_of_a_two_layer_flap() {
        let mut store = square_store();
        // 下ごしらえ: 半分に折って2層にする(背は y=0.5 = 中心線)
        store
            .apply_seq(fold_op(0, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]))
            .unwrap();
        let flap: Vec<u32> = store.faces.iter().map(|f| f.id).collect();
        assert_eq!(flap.len(), 2);

        // 中心線 y=0.5 の右端 (1,0.5) の先端を持ち上げる
        let view = store
            .apply_seq(SeqOp::Technique {
                up_to: 1,
                kind: TechniqueKind::Petal,
                flap,
                line: [[0.0, 0.5], [1.0, 0.5]],
                reference_point: [1.0, 0.5],
                open_to_back: None,
                polygon: None,
                center: None,
            })
            .unwrap();
        assert!(view.faces.len() > 2, "羽と中央のくさびに分かれる");
        assert_eq!(view.doc.sequence.len(), 2);
        assert_eq!(view.doc.sequence[1].kind, TechniqueKind::Petal);
        assert!(
            !view.doc.sequence[1].drivers.is_empty(),
            "動かす折り線が手順に記録される"
        );
    }

    /// 沈め折りは畳んだ状態の先端(角)に対して適用できる。
    #[test]
    fn technique_open_sink_turns_the_tip_inside_out() {
        let mut store = square_store();
        // 下ごしらえ: 半分に折って2層にする
        store
            .apply_seq(fold_op(0, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]))
            .unwrap();

        // 右下の角 (1,0) を切り取る線の先端側を沈める
        let view = store
            .apply_seq(SeqOp::Technique {
                up_to: 1,
                kind: TechniqueKind::OpenSink,
                flap: Vec::new(),
                line: [[0.8, 0.0], [1.0, 0.2]],
                reference_point: [0.97, 0.03],
                open_to_back: None,
                polygon: None,
                center: None,
            })
            .unwrap();
        assert!(view.faces.len() > 2, "先端側で各層が分かれる");
        assert_eq!(view.doc.sequence.len(), 2);
        assert_eq!(view.doc.sequence[1].kind, TechniqueKind::OpenSink);
        assert!(
            !view.doc.sequence[1].drivers.is_empty(),
            "沈めた折り線が手順に記録される"
        );
    }

    /// ひだ寄せとねじり折りも1枚の紙に対して適用できる。
    #[test]
    fn technique_swivel_and_twist_apply_to_a_flat_sheet() {
        let mut store = square_store();
        let view = store
            .apply_seq(SeqOp::Technique {
                up_to: 0,
                kind: TechniqueKind::Swivel,
                flap: Vec::new(),
                line: [[0.0, 0.5], [1.0, 0.5]],
                reference_point: [1.0, 0.8],
                open_to_back: None,
                polygon: None,
                center: None,
            })
            .unwrap();
        assert_eq!(view.faces.len(), 3, "くさび・その先・基準線の向こう");
        assert_eq!(view.doc.sequence[0].kind, TechniqueKind::Swivel);

        let mut store = square_store();
        let view = store
            .apply_seq(SeqOp::Technique {
                up_to: 0,
                kind: TechniqueKind::Twist,
                flap: Vec::new(),
                line: [[0.4, 0.4], [0.6, 0.4]],
                reference_point: [0.6, 0.327],
                open_to_back: None,
                polygon: None,
                center: None,
            })
            .unwrap();
        assert_eq!(view.faces.len(), 9, "中央1面+ひだ4面+腕4面");
        assert_eq!(view.doc.sequence[0].kind, TechniqueKind::Twist);
        assert!(!view.doc.sequence[0].drivers.is_empty());
    }

    /// ねじり折りは、中央多角形と中心を直接渡せる(辺の長さが違う多角形も折れる)。
    /// `polygon`/`center` は省略できる項目なので、古い作品ファイル(この2つが無い
    /// JSON)もそのまま読める。
    #[test]
    fn technique_twist_takes_a_polygon_with_unequal_sides() {
        let mut store = square_store();
        let view = store
            .apply_seq(SeqOp::Technique {
                up_to: 0,
                kind: TechniqueKind::Twist,
                flap: Vec::new(),
                line: [[0.0, 0.0], [1.0, 0.0]],
                reference_point: [0.72, 0.75],
                open_to_back: None,
                polygon: Some(vec![[0.85, 0.50], [0.45, 0.75], [0.40, 0.45]]),
                center: Some([0.55, 0.56]),
            })
            .unwrap();
        assert_eq!(view.faces.len(), 7, "中央1面+ひだ3面+腕3面");
        assert_eq!(view.doc.sequence[0].kind, TechniqueKind::Twist);
        assert!(
            view.warnings.is_empty(),
            "紙は裂けない: {:?}",
            view.warnings
        );

        // 省略した形のJSONも読める(#[serde(default)])
        let old = r#"{"type":"Technique","up_to":0,"kind":"Twist","flap":[],
            "line":[[0.4,0.4],[0.6,0.4]],"reference_point":[0.6,0.327]}"#;
        let op: SeqOp = serde_json::from_str(old).expect("古い形のJSONも読める");
        let mut store = square_store();
        let view = store.apply_seq(op).unwrap();
        assert_eq!(view.faces.len(), 9, "中央1面+ひだ4面+腕4面(従来の指し方)");
    }

    /// 未実装の技法・折れない指定・範囲外の挿入位置はErr(文書は無変更)。
    #[test]
    fn technique_rejects_unsupported_kind_and_bad_input() {
        let mut store = square_store();
        let before = store.doc.clone();

        let err = store
            .apply_seq(SeqOp::Technique {
                up_to: 0,
                kind: TechniqueKind::Pose,
                flap: Vec::new(),
                line: [[0.4, 0.0], [0.4, 1.0]],
                reference_point: [0.5, 0.5],
                open_to_back: None,
                polygon: None,
                center: None,
            })
            .unwrap_err();
        assert!(err.contains("まだ選べません"), "err={err}");
        assert_eq!(store.doc, before);

        // 段の幅0(基準点が折り線の上)
        let err = store
            .apply_seq(SeqOp::Technique {
                up_to: 0,
                kind: TechniqueKind::Pleat,
                flap: Vec::new(),
                line: [[0.4, 0.0], [0.4, 1.0]],
                reference_point: [0.4, 0.5],
                open_to_back: None,
                polygon: None,
                center: None,
            })
            .unwrap_err();
        assert!(err.contains("段の幅"), "err={err}");
        assert_eq!(store.doc, before);
        assert!(store.undo_stack.is_empty(), "Errはundo履歴に積まれない");

        // 手順の数を超える挿入位置
        let err = store
            .apply_seq(SeqOp::Technique {
                up_to: 2,
                kind: TechniqueKind::Pleat,
                flap: Vec::new(),
                line: [[0.4, 0.0], [0.4, 1.0]],
                reference_point: [0.5, 0.5],
                open_to_back: None,
                polygon: None,
                center: None,
            })
            .unwrap_err();
        assert!(err.contains("手順の数を超えています"), "err={err}");
        assert_eq!(store.doc, before);
    }

    /// 受け入れ確認(Task 2-5): 座布団折り(4隅を中心へ)→観音折り(左右を中心線へ)を
    /// 画面の折り操作(SeqOp::FoldThrough)だけで作れる。
    /// 折り線は画面に見えている外形(立体表示の外接矩形)から決める=画面で線を引くのと同じ。
    #[test]
    fn cushion_then_cupboard_fold_only_with_fold_through() {
        let mut store = square_store();

        // 座布団折り: 隣り合う辺の中点を結ぶ線で4隅を中心へ折る(いずれも谷=手前へ)
        for k in 0..4 {
            let ([x0, y0], [x1, y1]) = outline_bbox(&store);
            let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            let mids = [[cx, y0], [x1, cy], [cx, y1], [x0, cy]];
            let line = [mids[k], mids[(k + 1) % 4]];
            let view = store
                .apply_seq(fold_op(store.doc.sequence.len(), line, [cx, cy]))
                .unwrap_or_else(|e| panic!("座布団折り{}回目が折れない: {e}", k + 1));
            assert!(
                view.warnings.iter().all(|w| !w.contains("裂けます")),
                "座布団折り{}回目の警告: {:?}",
                k + 1,
                view.warnings
            );
            // 折るたびに層番号(下から0)が画面の重なりと一致している
            assert_display_layers(&store, &format!("座布団折り{}回目", k + 1));
        }
        assert_eq!(store.doc.sequence.len(), 4);
        // 外形は辺の中点を結ぶ正方形(対角線=1.0)になる
        let ([x0, y0], [x1, y1]) = outline_bbox(&store);
        assert!((x1 - x0 - 1.0).abs() < 1e-6 && (y1 - y0 - 1.0).abs() < 1e-6);
        assert_eq!(layer_count(&store), 5, "中央1枚+4隅のフラップ");

        // 観音折り: 左右を中心の縦線へ折る(全ての層をまとめて折る)
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let quarter = (x1 - x0) / 4.0;
        for dx in [-quarter, quarter] {
            let x = cx + dx;
            store
                .apply_seq(fold_op(
                    store.doc.sequence.len(),
                    [[x, y0], [x, y1]],
                    [cx, cy],
                ))
                .unwrap_or_else(|e| panic!("観音折り(x={x})が折れない: {e}"));
            assert_display_layers(&store, &format!("観音折り(x={x})"));
        }
        assert_eq!(store.doc.sequence.len(), 6);
        // 幅は半分になり、紙は平らに畳まれたまま
        let ([nx0, _], [nx1, _]) = outline_bbox(&store);
        assert!(
            (nx1 - nx0 - 0.5).abs() < 1e-6,
            "観音折り後の横幅={}",
            nx1 - nx0
        );
        // 手順を最初から再生し直しても同じ形になる(3D状態を保存しない設計の確認)
        let replayed = ori3_layers::replay(&store.doc, 6, 1.0);
        assert!(replayed.skipped.is_empty(), "警告={:?}", replayed.warnings);
        assert!(replayed.warnings.is_empty(), "警告={:?}", replayed.warnings);
        assert!(
            replayed
                .frame
                .faces
                .iter()
                .all(|f| f.polygon.iter().all(|p| p[2].abs() < 1e-6)),
            "折り上がりは平ら"
        );
    }

    /// `Frame3D.layer`(「下から0」)が本当に画面の重なりと一致するか確かめる。
    ///
    /// 完全に畳んだ状態(t=1)では全ての面がz=0に重なって上下を読めないので、
    /// 折り終わる直前(t=0.99)の高さ(z)から本当の重なりを読み取る。そのとき
    /// 浮いている(動いている)層は、折り上がりでは高さの符号の側=上か下の端に
    /// まとまって並ぶはず。ここが食い違うと、層ずらし表示・書き出し順・
    /// 「いちばん上の1枚」の指定がすべて逆さまになる。
    fn assert_display_layers(store: &DocumentStore, label: &str) {
        let up_to = store.doc.sequence.len();
        let mid = ori3_layers::replay(&store.doc, up_to, 0.99).frame;
        let mut lifted: Vec<u32> = Vec::new();
        let mut sign = 0.0f64;
        for f in &mid.faces {
            let z = f
                .polygon
                .iter()
                .map(|p| p[2])
                .max_by(|a, b| a.abs().total_cmp(&b.abs()))
                .unwrap_or(0.0);
            if z.abs() > 1e-3 {
                lifted.push(f.face);
                if sign == 0.0 {
                    sign = z.signum();
                }
                assert_eq!(z.signum(), sign, "{label}: 動いた層は同じ向きへ浮く");
            }
        }
        assert!(
            !lifted.is_empty(),
            "{label}: 折る直前には動いた層が浮いている"
        );

        let frame = ori3_layers::replay(&store.doc, up_to, 1.0).frame;
        let total = frame.faces.len();
        let mut want: Vec<u32> = lifted.clone();
        want.sort_unstable();
        // 上へ浮いたなら層番号の大きい方、下へ潜ったなら小さい方にまとまる
        let mut got: Vec<u32> = frame
            .faces
            .iter()
            .filter(|f| {
                let l = usize::try_from(f.layer).unwrap();
                if sign > 0.0 {
                    l >= total - lifted.len()
                } else {
                    l < lifted.len()
                }
            })
            .map(|f| f.face)
            .collect();
        got.sort_unstable();
        assert_eq!(
            got,
            want,
            "{label}: 浮いた層が層番号の端にまとまっていない(layer={:?})",
            frame
                .faces
                .iter()
                .map(|f| (f.face, f.layer))
                .collect::<Vec<_>>()
        );
    }

    /// 現在の手順まで折った立体の外接矩形(x,y)。画面に見えている外形にあたる。
    fn outline_bbox(store: &DocumentStore) -> ([f64; 2], [f64; 2]) {
        let frame = ori3_layers::replay(&store.doc, store.doc.sequence.len(), 1.0).frame;
        let mut lo = [f64::MAX; 2];
        let mut hi = [f64::MIN; 2];
        for f in &frame.faces {
            for p in &f.polygon {
                for i in 0..2 {
                    lo[i] = lo[i].min(p[i]);
                    hi[i] = hi[i].max(p[i]);
                }
            }
        }
        (lo, hi)
    }

    /// 重なりの枚数(層の数)。
    fn layer_count(store: &DocumentStore) -> usize {
        ori3_cp::extract_faces(&store.doc.cp).len()
    }

    fn fold_op(up_to: usize, line: [[f64; 2]; 2], keep_side_point: [f64; 2]) -> SeqOp {
        SeqOp::FoldThrough {
            up_to,
            line,
            keep_side_point,
            target_layers: None,
            direction: ori3_model::FoldDirection::Up,
        }
    }

    /// CPE-009: 平らに畳めない点がビューに載る(操作は止めない)。
    #[test]
    fn view_reports_flat_fold_violations_without_blocking() {
        let mut store = square_store();
        // 2本の対角線を山で引くと、中心は山4本(山−谷=4)で前川定理を満たさない
        let view = store.apply_edit(diagonal()).unwrap();
        assert!(view.violations.is_empty(), "対角線1本だけなら違反なし");
        let view = store
            .apply_edit(EditOp::AddSegment {
                a: [1.0, 0.0],
                b: [0.0, 1.0],
                kind: EdgeKind::Mountain,
            })
            .unwrap();
        // 操作は成功したうえで、中心の1点が「畳めない点」として返る
        assert_eq!(view.violations.len(), 1, "violations={:?}", view.violations);
        let id = view.violations[0];
        let pos = view
            .doc
            .cp
            .vertices
            .iter()
            .find(|v| v.id == id)
            .unwrap()
            .pos;
        assert!((pos[0] - 0.5).abs() < 1e-9 && (pos[1] - 0.5).abs() < 1e-9);

        // 1本を谷に変えれば違反は消える(山3・谷1)
        let mountain = view
            .doc
            .cp
            .edges
            .iter()
            .find(|e| e.kind == EdgeKind::Mountain && (e.v0 == id || e.v1 == id))
            .unwrap()
            .id;
        let view = store
            .apply_edit(EditOp::SetEdgeKind {
                ids: vec![mountain],
                kind: EdgeKind::Valley,
            })
            .unwrap();
        assert!(
            view.violations.is_empty(),
            "violations={:?}",
            view.violations
        );
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
