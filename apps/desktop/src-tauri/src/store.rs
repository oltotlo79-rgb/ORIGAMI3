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

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
use std::cell::Cell;

use ori3_cp::Face;
use ori3_export::fold::{FoldConversionError, FoldImport, FoldIssue, FoldParseError};
use ori3_model::{
    CreasePattern, Document, Driver, EdgeId, EdgeKind, EditOp, FaceId, FoldStep, FoldTargetInfo,
    FoldTargetStatus, FoldTargetTopAction, Frame3D, MAX_GRID_DIVISIONS, MIN_GRID_DIVISIONS, Paper,
    SCHEMA_VERSION, SavedDocument, SeqOp, StepCreases, StepId, TechniqueKind, VertexId,
};

/// undo履歴の最大件数。超過時は最古をFIFOで破棄する。
const MAX_UNDO: usize = 100;

#[cfg(test)]
thread_local! {
    /// 並列test同士を干渉させず、現在のtest threadだけのcommit呼出し回数を測る。
    static COMMIT_COUNT_FOR_TEST: Cell<usize> = const { Cell::new(0) };
    /// 次のMoveStep候補の最終導出後をpanicさせる、製品状態に入らない一回限りの注入口。
    static FAIL_NEXT_MOVE_STEP_DERIVATION_FOR_TEST: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn reset_commit_count_for_test() {
    COMMIT_COUNT_FOR_TEST.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn commit_count_for_test() -> usize {
    COMMIT_COUNT_FOR_TEST.with(Cell::get)
}

#[cfg(test)]
fn record_commit_for_test() {
    COMMIT_COUNT_FOR_TEST.with(|count| count.set(count.get() + 1));
}

#[cfg(not(test))]
fn record_commit_for_test() {}

#[cfg(test)]
fn fail_move_step_derivation_if_requested() {
    let should_fail = FAIL_NEXT_MOVE_STEP_DERIVATION_FOR_TEST.with(|flag| flag.replace(false));
    if should_fail {
        panic!("MoveStepの導出に失敗しました");
    }
}

#[cfg(not(test))]
fn fail_move_step_derivation_if_requested() {}

/// SYS-002: 折り目の多い作品(カエル、280辺・141頂点)で取り消し履歴を100段
/// 積んだときの、履歴がヒープに保持しているバイト数の上限。
///
/// 実測(2026-08-23、この作業機、debugビルド、
/// `undo_history_after_100_edits_of_a_crease_rich_document_stays_under_a_measured_budget`
/// が `undo_history_heap_bytes` で履歴の中身を直接数えた値):
/// **813,352バイト**(約794KiB)。単独実行でも `cargo test --workspace` の中でも、
/// 並列・直列どちらでも同じ値になることを確認済み(各5回以上)。
///
/// (以前はプロセス全体のアロケータ確保量で測っており、並列実行時に他のテストの
/// 確保量が混入して58,315,019〜78,370,317バイトという実行ごとに変わる値が出ていた。
/// 経緯と実測は `scratchpad/undo-memory-fix-report.md`。)
///
/// 実測をそのまま境目にせず(CLAUDE.md §10.7.9)、約1.35倍の余裕を掛けて
/// `1_100_000`(約1.05MiB)とする。実測/上限 ≈ 0.74 で、このリポジトリの
/// 他の上限(実測の75〜82%を境目にする慣例)と同じ考え方。
/// バイト数は整数なので、比較は許容差なしの厳密な比較でよい。
///
/// 要件書(`docs/requirements-definition.md` SYS-002)が定める「許容上限の
/// 3分の1以下」の判定: 取り消し履歴機能全体として許容できる上限を
/// 200MB(`200_000_000`)と置いても、実測800,384バイトはその1/3(約66.7MB)を
/// 大幅に下回る。したがって編集前スナップショット方式のままで要件を満たし、
/// 逆操作方式への作り替えは不要と判断する。
#[cfg(test)]
const UNDO_HISTORY_BUDGET_BYTES: i64 = 1_100_000;

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
    #[cfg(test)]
    if std::env::var_os("ORI3_TEST_PAUSE_PARTIAL_ATOMIC_TARGET")
        .is_some_and(|requested| Path::new(&requested) == target)
    {
        use std::io::Write;

        let split = bytes.len().div_ceil(2);
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(&bytes[..split])?;
        file.sync_all()?;
        if let Some(ready) = std::env::var_os("ORI3_TEST_PAUSE_PARTIAL_ATOMIC_READY") {
            std::fs::write(PathBuf::from(ready), b"ready")?;
        }
        // 親testがこのprocessをChild::killする。returnやDropによる後始末を通さず、
        // 実際の書込み途中と同じpartial tempを残すためのtest-build専用checkpoint。
        loop {
            std::thread::park_timeout(std::time::Duration::from_millis(10));
        }
    }
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
    /// 手順ごとに展開図へ新しく足した折り線(手順IDで結び付ける)。
    /// 2D画面が「その手順の時点の展開図」を推測せずに組み立てるための来歴で、
    /// 作品ファイルにも同じ形で保存する。
    pub step_creases: Vec<StepCreases>,
    /// FOLD 1.2 限定の読込で置き換えた内容。確認待ちのgateには使わず、
    /// 読込済みの作品と一緒に画面へ返す。
    pub fold_issues: Vec<FoldIssue>,
    pub faces: Vec<Face>,
    /// 操作固有の警告 + `ori3_cp::validate` + 手順再生の警告(「止めずに警告」原則)
    pub warnings: Vec<String>,
    /// 平らに畳めない疑いのある点(前川定理・川崎定理を満たさない内部頂点)。
    /// 操作は止めず、2D画面で色を変えて知らせるだけ(CPE-009)
    pub violations: Vec<VertexId>,
    /// 今回の平坦化指定に関係し、指定角まで届かなかったか紙が食い込んだときに
    /// 全体通知へ出す点。
    /// 生の `violations` とは分け、操作の可否には使わない。
    pub flat_fold_violations: Vec<VertexId>,
    /// 最新ステップまで自動再生した立体(SEQ-004)。手順が空ならNone
    pub frame: Option<Frame3D>,
    /// 自動再生で折り線が見つからず飛ばされたステップのID
    pub skipped: Vec<StepId>,
    /// 補正後にも残る食い込みの原因候補ヒンジ。
    pub suspect_hinges: Vec<EdgeId>,
    /// 手順再生の最終姿勢で、紙の面どうしの食い込みを検出したか。
    /// 診断結果を知らせるだけで、再生結果を止める条件には使わない。
    pub contact_detected: bool,
    /// 保存手順の線分を現在の辺IDへ解決した希望角（永続化しない導出結果）。
    pub sequence_targets: Vec<Driver>,
    /// 自動再生で得た全ヒンジの実角。次の操作のwarm startにも使う。
    pub angles: HashMap<EdgeId, f64>,
    /// 前の希望角を譲った診断（永続化しない導出結果）。
    pub relaxations: Vec<ori3_rigid::AngleRelaxation>,
    /// 自動再生結果の閉包残差RMS（永続化しない導出結果）。
    pub closure_rms: Option<f64>,
    /// 現在指定を守った有限の最良候補を表示しているか。
    pub best_effort: bool,
    /// 自動再生の追従計算が収束したか。
    pub converged: bool,
    /// 巻き込みで回避できる典型的な単一縁衝突の、非破壊プレビュー結果。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fold_through_proposal: Option<ori3_layers::FoldThroughProposal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fold_target_info: Option<FoldTargetInfo>,
}

/// FOLDの読込を確定する前に起きた失敗。
///
/// parse時はstoreへ触れず、変換時はそれまでに集めたwarningとblocking errorを
/// [`FoldConversionError`]のまま保持する。
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FoldImportError {
    Parse(FoldParseError),
    Conversion(FoldConversionError),
}

impl FoldImportError {
    #[cfg(test)]
    #[must_use]
    pub(crate) fn warnings(&self) -> &[FoldIssue] {
        match self {
            Self::Parse(_) => &[],
            Self::Conversion(error) => &error.warnings,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn errors(&self) -> &[FoldIssue] {
        match self {
            Self::Parse(_) => &[],
            Self::Conversion(error) => &error.errors,
        }
    }
}

impl fmt::Display for FoldImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(_) => write!(
                formatter,
                "ほかの折り紙ソフトのファイルを読み取れませんでした。ファイルの内容を確認してください。"
            ),
            Self::Conversion(_) => write!(
                formatter,
                "このファイルには、ORIGAMI3で扱えない内容があります。"
            ),
        }
    }
}

impl std::error::Error for FoldImportError {}

/// undo/redoで行き来する状態一式。展開図・手順と、その来歴は必ず一緒に戻す。
#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    doc: Document,
    step_creases: Vec<StepCreases>,
}

/// command境界のpanic検査で、永続状態と全履歴が一切変わらないことを比較する。
/// test-onlyのfailpointと観測counterは製品状態ではないため含めない。
#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AtomicityProbe {
    document_bytes: Vec<u8>,
    step_creases_bytes: Vec<u8>,
    faces: Vec<Face>,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    dirty: bool,
    path: Option<PathBuf>,
    pose_angles: Option<HashMap<EdgeId, f64>>,
}

pub struct DocumentStore {
    doc: Document,
    /// 手順ごとに展開図へ新しく足した折り線(手順IDで結び付ける)。
    /// 手順を消しても消さない(並べ替えは削除+挿入で行われるため)。
    /// 保存時だけ、存在しない手順の分を落として書き出す。
    step_creases: Vec<StepCreases>,
    /// 現docに対応する導出faces(pose_solveが毎回extract_facesを再実行しない
    /// ためのキャッシュ)。docの変更経路はstore内(new_document/open/commit/
    /// undo/redo)に閉じているため、その全箇所で更新すれば整合が保たれる
    faces: Vec<Face>,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    dirty: bool,
    path: Option<PathBuf>,
    /// pose_solveの前回解(次回のwarm start用)。ソルバーは知らない辺IDを
    /// 無視するため、CP編集後に古い解が残っていても安全
    pose_angles: Option<HashMap<EdgeId, f64>>,
}

/// 3D画面で実際に当たった点から作る、立体折り専用の一時入力。
/// 作品には保存せず、同じFoldThroughコマンドの処理中だけ使う。
#[derive(Clone, Debug, serde::Deserialize)]
pub struct SpatialFoldSpec {
    pub from: [f64; 3],
    pub to: [f64; 3],
    #[serde(alias = "grabFace")]
    pub grab_face: FaceId,
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
            step_creases: Vec::new(),
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
        let view = build_view(&doc, &[], Vec::new());
        self.doc = doc;
        self.step_creases = Vec::new();
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
        let saved = parse_document(&text)?;
        // 導出を先に済ませ、成功した場合のみ状態を確定する
        let view = build_view(&saved.document, &saved.step_creases, Vec::new());
        self.doc = saved.document;
        self.step_creases = saved.step_creases;
        self.faces = view.faces.clone();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.dirty = false;
        self.path = Some(path.to_path_buf());
        self.pose_angles = None;
        Ok(view)
    }

    /// FOLD 1.2 限定を、既存undoで1回に戻せる未保存作品として読み込む。
    ///
    /// command層がlock外でparse・限定profile変換を終えた候補値を受け取り、
    /// [`Self::commit_prebuilt`]を1回だけ通す。warningは確認gateにせず、
    /// [`DocumentView::fold_issues`]へ全件載せて読込結果と同時に返す。
    pub(crate) fn import_fold(&mut self, import: FoldImport) -> DocumentView {
        let FoldImport { document, warnings } = import;
        // FOLD frameは同じ紙・同じedge topologyの角度snapshotであり、各手順で
        // 展開図へ新しい線を足した来歴はない。旧`.ori3`と同じ空の来歴で保持する。
        let step_creases = Vec::new();
        let mut view = build_view(&document, &step_creases, Vec::new());
        view.fold_issues = warnings;

        let view = self.commit_prebuilt(document, step_creases, view);
        self.dirty = true;
        self.path = None;
        self.pose_angles = None;
        view
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
        let json = serde_json::to_string_pretty(&self.saved_document())
            .map_err(|e| format!("保存データの作成に失敗しました: {e}"))?;
        write_atomic(&target, json.as_bytes()).map_err(|e| format!("保存に失敗しました: {e}"))?;
        self.path = Some(target);
        self.dirty = false;
        Ok(())
    }

    /// ファイルへ書き出す形(作品 + 手順ごとの追加折り線の来歴)を作る。
    ///
    /// 今は存在しない手順の来歴は書き出さない。書き出さないことで、読み直した
    /// 作品の新しい手順IDが古い来歴と衝突しなくなる。
    pub(crate) fn saved_document(&self) -> SavedDocument {
        SavedDocument {
            document: self.doc.clone(),
            step_creases: retain_existing_steps(&self.doc, &self.step_creases),
        }
    }

    /// 編集操作を1つ適用する。実際に変更が起きた場合のみundo履歴に積む。
    pub fn apply_edit(&mut self, op: EditOp) -> Result<DocumentView, String> {
        self.apply_edits(vec![op])
    }

    /// 複数の編集操作を「利用者の1操作」として適用する。
    ///
    /// 元に戻せる履歴は最後に1件だけ積む。曲線1本や左右対称の2本のように、
    /// 画面では1回の入力でも内部で複数の線になる操作を、元に戻す1回で
    /// 引く前へ戻せるようにするため(不具合D05)。
    /// 途中の操作が断られたら何も適用しない(片側だけ引かれた形にしない)。
    pub fn apply_edits(&mut self, ops: Vec<EditOp>) -> Result<DocumentView, String> {
        if ops.is_empty() {
            return Err("編集する内容がありません".to_string());
        }
        let replaced_crease_pattern = ops
            .iter()
            .any(|op| matches!(op, EditOp::ReplaceCreasePattern { .. }));
        let mut doc = self.doc.clone();
        let mut warnings = Vec::new();
        for op in ops {
            Self::edit_document(&mut doc, op, &mut warnings)?;
        }
        // 展開図の編集は手順を増やさない。線を引いた時点の来歴は変わらない
        let step_creases = self.step_creases.clone();
        let view = self.commit(doc, step_creases, warnings);
        if replaced_crease_pattern {
            // CP全置換前の解は辺IDが偶然一致しても使ってはいけない。
            self.pose_angles = None;
        }
        Ok(view)
    }

    /// 提案の展開図と折り手順を、利用者の1操作としてまとめて入れる(作業28)。
    ///
    /// [`EditOp::ReplaceCreasePattern`] は展開図だけを差し替え、折り手順を必ず空にする。
    /// 折り方まで付いた提案では、展開図と手順が**そろって初めて意味を持つ**ので、
    /// 別々の操作にはしない。
    ///
    /// - 断る場合は**確定の前**に断るので、途中まで入った状態が残らない。
    /// - 確定は [`Self::commit`] の1回だけなので、**元に戻す1回**で入れる前へ戻る。
    ///
    /// # Errors
    ///
    /// 展開図から紙の面を取り出せない場合と、折り手順の番号が重なっている場合。
    /// どちらも数を数えるだけの確認で、計算した小数を比べていない。
    pub fn apply_proposal(
        &mut self,
        cp: CreasePattern,
        steps: Vec<FoldStep>,
    ) -> Result<DocumentView, String> {
        if ori3_cp::extract_faces(&cp).is_empty() {
            return Err("この展開図では紙の面が作れませんでした".to_string());
        }
        let mut seen = HashSet::new();
        for step in &steps {
            if !seen.insert(step.id) {
                return Err("同じ折り手順が二重に入っています".to_string());
            }
        }
        let mut doc = self.doc.clone();
        doc.cp = cp;
        doc.sequence = steps;
        // 前の展開図に付いていた来歴は、番号がぶつかると無関係の線を指してしまう。
        // 置き換えは新規作品と同じく、来歴を持たない状態から始める。
        let view = self.commit(doc, Vec::new(), Vec::new());
        // 展開図が変わった後は、辺IDが偶然一致しても前の解を使ってはいけない。
        self.pose_angles = None;
        Ok(view)
    }

    /// 編集操作1つを候補の作品へ反映する(履歴には積まない)。
    fn edit_document(
        doc: &mut Document,
        op: EditOp,
        warnings: &mut Vec<String>,
    ) -> Result<(), String> {
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
        Ok(())
    }

    /// 折り手順操作を適用する。実際に変更が起きた場合のみundo履歴に積む。
    pub fn apply_seq(&mut self, op: SeqOp) -> Result<DocumentView, String> {
        self.apply_seq_with_spatial(op, None)
    }

    /// 3D画面の当たり点を伴う折り手順操作。
    /// 平坦時は追加情報を見ず、従来の処理をそのまま使う。
    pub fn apply_seq_with_spatial(
        &mut self,
        op: SeqOp,
        spatial: Option<SpatialFoldSpec>,
    ) -> Result<DocumentView, String> {
        let mut doc = self.doc.clone();
        let mut step_creases = self.step_creases.clone();
        let mut warnings: Vec<String> = Vec::new();
        let mut fold_through_proposal = None;
        match op {
            SeqOp::PushStep { step } => {
                record_frontend_step(&mut step_creases, &step);
                doc.sequence.push(step);
            }
            SeqOp::InsertStep { index, step } => {
                if index > doc.sequence.len() {
                    return Err(format!("挿入位置 {index} が手順の数を超えています"));
                }
                record_frontend_step(&mut step_creases, &step);
                doc.sequence.insert(index, step);
            }
            SeqOp::RemoveStep { id } => {
                // 来歴は消さない。手順の並べ替えは削除+挿入で行われるため、
                // ここで消すと並べ替えただけで折り線の来歴が失われる
                let before = doc.sequence.len();
                doc.sequence.retain(|s| s.id != id);
                if doc.sequence.len() == before {
                    return Err(format!("手順ID {id} が見つかりません"));
                }
            }
            SeqOp::MoveStep { id, to_index } => {
                // step_creasesをIDへ一意に対応させられないDocumentでは、対象IDに
                // 重複が無くてもMoveStep全体を拒否する。異常が重なった場合の契約は
                // duplicate -> missing ID -> range -> no-op の順で固定する。
                let mut seen = HashSet::with_capacity(doc.sequence.len());
                if doc.sequence.iter().any(|step| !seen.insert(step.id)) {
                    return Err("同じ折り手順が二重に入っています".to_string());
                }
                let Some(from) = doc.sequence.iter().position(|step| step.id == id) else {
                    return Err(format!("手順ID {id} が見つかりません"));
                };
                if to_index >= doc.sequence.len() {
                    return Err(format!("移動先 {to_index} が手順の数を超えています"));
                }

                if from != to_index {
                    let moved = doc.sequence.remove(from);
                    // to_indexはremove前の隙間ではなく、移動後sequenceの最終index。
                    // remove後の長さは元のlen-1なので、契約上の最大len-1にもinsert可能。
                    doc.sequence.insert(to_index, moved);
                }

                // MoveStepだけは、commandがErrを返したのにstoreだけ確定済みになる
                // post-commit replayを作らない。候補cloneの最終view/replayを先に導出し、
                // 実移動ならその同じviewとDocumentを1回だけ確定する。
                let view = build_move_step_view(&doc, &step_creases);
                if from == to_index {
                    return Ok(view);
                }
                return Ok(self.commit_prebuilt(doc, step_creases, view));
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
                target_pleat_count,
                direction,
                alignment,
                accept_additional_crease,
                pose_before,
            } => {
                let mut insert_warnings = check_insert_point(&doc, up_to)?;
                if target_layers.is_some() && target_pleat_count.is_some() {
                    return Err("折るひだの枚数と個別の紙を同時には指定できません".to_string());
                }
                if target_pleat_count.is_some() && spatial.is_some() {
                    return Err(
                        "折るひだの枚数と3D上のつかみ位置を同時には指定できません".to_string()
                    );
                }
                let target_layers = if let Some(count) = target_pleat_count {
                    Some(fold_target_faces_at(
                        &doc,
                        &self.faces,
                        up_to,
                        line,
                        keep_side_point,
                        pose_before.as_ref(),
                        count,
                    )?)
                } else {
                    target_layers
                };
                if pose_before.is_some() && spatial.is_some() {
                    return Err(
                        "折った形の再現と3D上のつかみ位置を、同時には指定できません".to_string()
                    );
                }
                if let Some(pose_input) = pose_before {
                    let pose = ori3_layers::replay::canonical_flat_pose_at(
                        &doc,
                        &self.faces,
                        up_to,
                        &pose_input,
                    )?;
                    let mut pose_step = pose.step;
                    pose_step.id = next_step_id(&doc, &step_creases);
                    record_frontend_step(&mut step_creases, &pose_step);
                    doc.sequence.insert(up_to, pose_step);

                    let mut cp = doc.cp.clone();
                    let result = ori3_layers::fold_through_with_additional_crease(
                        &mut cp,
                        &self.faces,
                        &pose.state,
                        &ori3_layers::FoldThroughInput {
                            line,
                            keep_side_point,
                            target_layers,
                            direction,
                        },
                        accept_additional_crease,
                    )?;
                    let mut step = result.step;
                    step.id = next_step_id(&doc, &step_creases);
                    step.alignment = alignment;
                    let lines = added_crease_lines(&doc.cp, &cp, &result.added_edges);
                    record_step_creases(&mut step_creases, step.id, lines);
                    doc.cp = cp;
                    doc.sequence.insert(up_to + 1, step);
                    warnings = pose.warnings;
                    warnings.append(&mut insert_warnings);
                    warnings.extend(result.warnings);
                } else {
                    // facesは現docから導出済みのキャッシュ(docはまだ複製したまま無変更)
                    // 現在の状態を求め直すときの警告(飛ばした手順など)も利用者へ返す
                    // 3D画面が実際の当たり点を渡した場合は、画面と同じFrame3Dの
                    // z座標判定を先に行う。行列だけを見る平坦判定との差で、立体用
                    // 入力が従来の2D入力として処理されることを防ぐ。
                    let current = spatial
                        .as_ref()
                        .map(|_| ori3_layers::replay_with_faces(&doc, &self.faces, up_to, 1.0));
                    if let Some(current) = current
                        .as_ref()
                        .filter(|current| frame_is_nonflat(&current.frame))
                    {
                        let input = spatial_fold_input(
                            spatial.as_ref(),
                            &current.frame,
                            &self.faces,
                            line,
                            keep_side_point,
                            target_layers.as_deref(),
                            direction,
                        );
                        let mut result =
                            ori3_layers::fold_from_plane_3d(&doc, &self.faces, up_to, &input);
                        if let Some(mut step) = result.step.take() {
                            step.id = next_step_id(&doc, &step_creases);
                            step.alignment = alignment;
                            let lines =
                                added_crease_lines(&doc.cp, &result.cp, &result.added_edges);
                            record_step_creases(&mut step_creases, step.id, lines);
                            doc.cp = result.cp;
                            doc.sequence.insert(up_to, step);
                        }
                        warnings.append(&mut insert_warnings);
                        warnings.append(&mut result.warnings);
                    } else {
                        match ori3_layers::flat_state_at(&doc, &self.faces, up_to) {
                            Ok((state, state_warnings)) => {
                                let mut cp = doc.cp.clone();
                                let result = ori3_layers::fold_through_with_additional_crease(
                                    &mut cp,
                                    &self.faces,
                                    &state,
                                    &ori3_layers::FoldThroughInput {
                                        line,
                                        keep_side_point,
                                        target_layers,
                                        direction,
                                    },
                                    accept_additional_crease,
                                )?;
                                let mut step = result.step;
                                step.id = next_step_id(&doc, &step_creases);
                                step.alignment = alignment;
                                let lines = added_crease_lines(&doc.cp, &cp, &result.added_edges);
                                record_step_creases(&mut step_creases, step.id, lines);
                                doc.cp = cp;
                                doc.sequence.insert(up_to, step);
                                warnings = state_warnings;
                                warnings.append(&mut insert_warnings);
                                warnings.extend(result.warnings);
                            }
                            Err(flat_error) => {
                                // 旧呼び出しにはspatialが無い。その場合も従来どおり、
                                // 平坦判定が拒否した立体姿勢を2D入力から続けて折る。
                                let current = current.unwrap_or_else(|| {
                                    ori3_layers::replay_with_faces(&doc, &self.faces, up_to, 1.0)
                                });
                                if !frame_is_nonflat(&current.frame) {
                                    return Err(flat_error);
                                }
                                let input = spatial_fold_input(
                                    spatial.as_ref(),
                                    &current.frame,
                                    &self.faces,
                                    line,
                                    keep_side_point,
                                    target_layers.as_deref(),
                                    direction,
                                );
                                let mut result = ori3_layers::fold_from_plane_3d(
                                    &doc,
                                    &self.faces,
                                    up_to,
                                    &input,
                                );
                                if let Some(mut step) = result.step.take() {
                                    step.id = next_step_id(&doc, &step_creases);
                                    step.alignment = alignment;
                                    let lines = added_crease_lines(
                                        &doc.cp,
                                        &result.cp,
                                        &result.added_edges,
                                    );
                                    record_step_creases(&mut step_creases, step.id, lines);
                                    doc.cp = result.cp;
                                    doc.sequence.insert(up_to, step);
                                }
                                warnings.append(&mut insert_warnings);
                                warnings.append(&mut result.warnings);
                            }
                        }
                    }
                }
            }
            SeqOp::CreaseOnlyTop {
                up_to,
                material_line,
                material_keep_side_point,
                direction,
                pose_before,
                alignment,
            } => {
                let mut insert_warnings = check_insert_point(&doc, up_to)?;
                let pose = ori3_layers::replay::canonical_nonflat_pose_at(
                    &doc,
                    &self.faces,
                    up_to,
                    pose_before.as_ref(),
                )?;
                let mut provider = material_top_surface_provider(
                    &doc.cp,
                    &self.faces,
                    &pose,
                    material_keep_side_point,
                )?;
                let result = ori3_layers::crease_only_top_from_material_line(
                    &doc.cp,
                    &self.faces,
                    &pose,
                    &ori3_layers::SpatialCreaseOnlyInput {
                        material_line,
                        material_keep_side_point,
                        direction,
                    },
                    &mut provider,
                )
                .map_err(material_crease_only_error_message)?;
                verify_direct_crease_only_pose_is_unchanged(&pose, &result)?;

                let mut insertion_index = up_to;
                let mut inserted_step_ids = Vec::with_capacity(2);
                if let Some(pose_input) = pose_before.as_ref() {
                    let mut pose_step = nonflat_pose_step_from_input(&doc.cp, pose_input)?;
                    pose_step.id = next_step_id(&doc, &step_creases);
                    inserted_step_ids.push(pose_step.id);
                    record_frontend_step(&mut step_creases, &pose_step);
                    doc.sequence.insert(insertion_index, pose_step);
                    insertion_index += 1;
                }

                let mut crease_step = result.step.clone();
                crease_step.id = next_step_id(&doc, &step_creases);
                crease_step.alignment = alignment;
                inserted_step_ids.push(crease_step.id);
                let lines = added_crease_lines(&doc.cp, &result.cp, &result.added_edges);
                record_step_creases(&mut step_creases, crease_step.id, lines);
                doc.cp = result.cp;
                doc.sequence.insert(insertion_index, crease_step);
                let replay_up_to_crease = insertion_index + 1;

                warnings = pose.frame.warnings.clone();
                warnings.append(&mut insert_warnings);
                filter_penetration_warnings(
                    &mut warnings,
                    doc.display.penetration_prevention_enabled,
                );
                let mut view = build_view(&doc, &step_creases, warnings);
                if replay_up_to_crease > view.doc.sequence.len() {
                    return Err(
                        "保存した折った形の位置を読み直せないため、変更しませんでした。"
                            .to_string(),
                    );
                }
                let cold = ori3_layers::replay::replay_endpoint_with_faces_uncached(
                    &view.doc,
                    &view.faces,
                    replay_up_to_crease,
                );
                verify_cold_crease_only_replay(
                    &pose,
                    &result.material_vertices,
                    &inserted_step_ids,
                    &view.faces,
                    &cold,
                )?;
                attach_replay(&mut view);
                return Ok(self.commit_prebuilt(doc, step_creases, view));
            }
            SeqOp::PreviewFoldThrough {
                up_to,
                line,
                keep_side_point,
                target_layers,
                target_pleat_count,
                direction,
                pose_before,
            } => {
                check_insert_point(&doc, up_to)?;
                if target_layers.is_some() && target_pleat_count.is_some() {
                    return Err("折るひだの枚数と個別の紙を同時には指定できません".to_string());
                }
                if target_pleat_count.is_some() && spatial.is_some() {
                    return Err(
                        "折るひだの枚数と3D上のつかみ位置を同時には指定できません".to_string()
                    );
                }
                let target_layers = if let Some(count) = target_pleat_count {
                    Some(fold_target_faces_at(
                        &doc,
                        &self.faces,
                        up_to,
                        line,
                        keep_side_point,
                        pose_before.as_ref(),
                        count,
                    )?)
                } else {
                    target_layers
                };
                if pose_before.is_some() && spatial.is_some() {
                    return Err(
                        "折った形の再現と3D上のつかみ位置を、同時には指定できません".to_string()
                    );
                }
                if let Some(pose_input) = pose_before {
                    let pose = ori3_layers::replay::canonical_flat_pose_at(
                        &doc,
                        &self.faces,
                        up_to,
                        &pose_input,
                    )?;
                    fold_through_proposal = ori3_layers::propose_fold_through(
                        &doc.cp,
                        &self.faces,
                        &pose.state,
                        &ori3_layers::FoldThroughInput {
                            line,
                            keep_side_point,
                            target_layers,
                            direction,
                        },
                    )?;
                    warnings = pose.warnings;
                } else {
                    let current = spatial
                        .as_ref()
                        .map(|_| ori3_layers::replay_with_faces(&doc, &self.faces, up_to, 1.0));
                    if let Some(current) = current
                        .as_ref()
                        .filter(|current| frame_is_nonflat(&current.frame))
                    {
                        let input = spatial_fold_input(
                            spatial.as_ref(),
                            &current.frame,
                            &self.faces,
                            line,
                            keep_side_point,
                            target_layers.as_deref(),
                            direction,
                        );
                        warnings =
                            ori3_layers::fold_from_plane_3d(&doc, &self.faces, up_to, &input)
                                .warnings;
                    } else {
                        match ori3_layers::flat_state_at(&doc, &self.faces, up_to) {
                            Ok((state, state_warnings)) => {
                                fold_through_proposal = ori3_layers::propose_fold_through(
                                    &doc.cp,
                                    &self.faces,
                                    &state,
                                    &ori3_layers::FoldThroughInput {
                                        line,
                                        keep_side_point,
                                        target_layers,
                                        direction,
                                    },
                                )?;
                                warnings = state_warnings;
                            }
                            Err(flat_error) => {
                                let current = current.unwrap_or_else(|| {
                                    ori3_layers::replay_with_faces(&doc, &self.faces, up_to, 1.0)
                                });
                                if !frame_is_nonflat(&current.frame) {
                                    return Err(flat_error);
                                }
                                let input = spatial_fold_input(
                                    spatial.as_ref(),
                                    &current.frame,
                                    &self.faces,
                                    line,
                                    keep_side_point,
                                    target_layers.as_deref(),
                                    direction,
                                );
                                warnings = ori3_layers::fold_from_plane_3d(
                                    &doc,
                                    &self.faces,
                                    up_to,
                                    &input,
                                )
                                .warnings;
                            }
                        }
                    }
                }
                filter_penetration_warnings(
                    &mut warnings,
                    doc.display.penetration_prevention_enabled,
                );
                let mut view = build_view(&doc, &step_creases, warnings);
                view.fold_through_proposal = fold_through_proposal;
                return Ok(view);
            }
            SeqOp::PreviewFoldTargets {
                up_to,
                line,
                keep_side_point,
                pose_before,
            } => {
                check_insert_point(&doc, up_to)?;
                let lookup = fold_target_info_at(
                    &doc,
                    &self.faces,
                    up_to,
                    line,
                    keep_side_point,
                    pose_before.as_ref(),
                );
                let mut view = build_view(&doc, &step_creases, lookup.warnings);
                view.fold_target_info = Some(lookup.info);
                return Ok(view);
            }
            SeqOp::PreviewFoldTargetsOnMaterial {
                up_to,
                material_line,
                material_keep_side_point,
                pose_before,
            } => {
                check_insert_point(&doc, up_to)?;
                let lookup = material_fold_target_info_at(
                    &doc,
                    &self.faces,
                    up_to,
                    material_line,
                    material_keep_side_point,
                    pose_before.as_ref(),
                );
                let mut view = build_view(&doc, &step_creases, lookup.warnings);
                view.fold_target_info = Some(lookup.info);
                return Ok(view);
            }
            SeqOp::FlatMotion { up_to, parts, kind } => {
                // FoldThrough/Techniqueと同じ挿入・警告規約。面IDはこの時点の
                // 導出値だが、結果は座標参照のFoldStepへ変換されて保存される。
                let mut insert_warnings = check_insert_point(&doc, up_to)?;
                let (state, state_warnings) = ori3_layers::flat_state_at(&doc, &self.faces, up_to)?;
                let mut cp = doc.cp.clone();
                let result = ori3_layers::flat_motion(
                    &mut cp,
                    &self.faces,
                    &state,
                    &ori3_layers::FlatMotionInput {
                        parts: parts.into_iter().map(to_layer_motion_part).collect(),
                        kind,
                    },
                )?;
                let mut step = result.step;
                step.id = next_step_id(&doc, &step_creases);
                let lines = added_crease_lines(&doc.cp, &cp, &result.added_edges);
                record_step_creases(&mut step_creases, step.id, lines);
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
                step.id = next_step_id(&doc, &step_creases);
                let lines = added_crease_lines(&doc.cp, &cp, &result.added_edges);
                record_step_creases(&mut step_creases, step.id, lines);
                doc.cp = cp;
                doc.sequence.insert(up_to, step);
                warnings = state_warnings;
                warnings.append(&mut insert_warnings);
                warnings.extend(result.warnings);
            }
        }
        filter_penetration_warnings(&mut warnings, doc.display.penetration_prevention_enabled);
        let mut view = self.commit(doc, step_creases, warnings);
        view.fold_through_proposal = fold_through_proposal;
        Ok(view)
    }

    /// 直前の変更を取り消す。
    pub fn undo(&mut self) -> Result<DocumentView, String> {
        // 導出を先に済ませてからpop・swapする(導出panic時にstoreを変えない)
        let prev = self
            .undo_stack
            .last()
            .ok_or_else(|| "これ以上元に戻せません".to_string())?;
        let view = build_view(&prev.doc, &prev.step_creases, Vec::new());
        let prev = self.undo_stack.pop().expect("直前にlastで確認済み");
        self.redo_stack.push(Snapshot {
            doc: std::mem::replace(&mut self.doc, prev.doc),
            step_creases: std::mem::replace(&mut self.step_creases, prev.step_creases),
        });
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
        let view = build_view(&next.doc, &next.step_creases, Vec::new());
        let next = self.redo_stack.pop().expect("直前にlastで確認済み");
        self.undo_stack.push(Snapshot {
            doc: std::mem::replace(&mut self.doc, next.doc),
            step_creases: std::mem::replace(&mut self.step_creases, next.step_creases),
        });
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
    pub fn autosave_snapshot(&self) -> Option<(Option<PathBuf>, SavedDocument)> {
        if !self.dirty {
            return None;
        }
        Some((self.path.clone(), self.saved_document()))
    }

    /// 自動保存から読んだ作品を現在の作品にする(復元)。
    /// 元の保存先を引き継ぎ、まだ書き出していない内容なので未保存扱いにする。
    pub fn restore(&mut self, saved: SavedDocument, path: Option<PathBuf>) -> DocumentView {
        // 導出を先に済ませてから状態を確定する(openと同じ規約)
        let view = build_view(&saved.document, &saved.step_creases, Vec::new());
        self.doc = saved.document;
        self.step_creases = saved.step_creases;
        self.faces = view.faces.clone();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.dirty = true;
        self.path = path;
        self.pose_angles = None;
        view
    }

    /// pose_solveの入力(Document・導出済みfaces・前回解・2種類の接触設定)を取り出す。
    /// Documentは現在の再生位置に有効な保存済みlayer_orderを導出するために使う。
    /// facesは編集時に導出済みのキャッシュの流用で、extract_facesを再実行しない。
    /// 設計規約: ロック中に重い計算をしないため、コマンド層はこの複製を取って
    /// 即ロックを解放し、solveはロックの外で実行する。
    pub fn pose_inputs(
        &self,
    ) -> (
        Document,
        Vec<Face>,
        Option<HashMap<EdgeId, f64>>,
        bool,
        bool,
    ) {
        (
            self.doc.clone(),
            self.faces.clone(),
            self.pose_angles.clone(),
            self.doc.display.overlap_prevention_enabled,
            self.doc.display.penetration_prevention_enabled,
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

    /// 次のMoveStep候補の最終導出後だけを失敗させる、thread-local注入口。
    #[cfg(test)]
    pub(crate) fn fail_next_move_step_derivation_for_test(&mut self) {
        FAIL_NEXT_MOVE_STEP_DERIVATION_FOR_TEST.with(|flag| flag.set(true));
    }

    /// commandのpanic変換をまたいで、storeの全製品状態を比較するtest-only snapshot。
    #[cfg(test)]
    pub(crate) fn atomicity_probe_for_test(&self) -> AtomicityProbe {
        AtomicityProbe {
            document_bytes: serde_json::to_vec(&self.doc).expect("Documentをbytes化できる"),
            step_creases_bytes: serde_json::to_vec(&self.step_creases)
                .expect("step_creasesをbytes化できる"),
            faces: self.faces.clone(),
            undo_stack: self.undo_stack.clone(),
            redo_stack: self.redo_stack.clone(),
            dirty: self.dirty,
            path: self.path.clone(),
            pose_angles: self.pose_angles.clone(),
        }
    }

    /// 変更後Documentを確定する。変更が実際に起きた場合のみundo履歴に積む。
    ///
    /// 導出(validate/extract_faces)を候補docに対して先に実行し、成功した場合のみ
    /// 状態を入れ替える。導出がpanicしてもstoreは無変更のまま(guardがErr化し、
    /// 「Err⇒無変更」の不変条件を保つ)。
    fn commit(
        &mut self,
        doc: Document,
        step_creases: Vec<StepCreases>,
        warnings: Vec<String>,
    ) -> DocumentView {
        let view = build_view(&doc, &step_creases, warnings);
        self.commit_prebuilt(doc, step_creases, view)
    }

    /// すでに候補Documentから導出し終えたviewと状態を、不可分に確定する。
    /// MoveStepは重いreplayもここへ来る前に済ませ、確定後に失敗点を残さない。
    fn commit_prebuilt(
        &mut self,
        doc: Document,
        step_creases: Vec<StepCreases>,
        view: DocumentView,
    ) -> DocumentView {
        record_commit_for_test();
        if doc != self.doc || step_creases != self.step_creases {
            if self.undo_stack.len() >= MAX_UNDO {
                self.undo_stack.remove(0);
            }
            let next = Snapshot { doc, step_creases };
            let prev = Snapshot {
                doc: std::mem::replace(&mut self.doc, next.doc),
                step_creases: std::mem::replace(&mut self.step_creases, next.step_creases),
            };
            self.undo_stack.push(prev);
            self.faces = view.faces.clone();
            self.redo_stack.clear();
            self.dirty = true;
        }
        view
    }
}

/// 保存されたJSONを作品へ戻す。schema_versionが合わなければErr。
/// `open` と自動保存の復元(autosave.rs)で共通に使う。
///
/// 手順ごとの追加折り線の来歴を持たない旧形式のファイルも、来歴なしとして読める。
pub fn parse_document(text: &str) -> Result<SavedDocument, String> {
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
fn build_view(
    doc: &Document,
    step_creases: &[StepCreases],
    mut warnings: Vec<String>,
) -> DocumentView {
    warnings.extend(ori3_cp::validate(&doc.cp));
    DocumentView {
        doc: doc.clone(),
        step_creases: retain_existing_steps(doc, step_creases),
        fold_issues: Vec::new(),
        faces: ori3_cp::extract_faces(&doc.cp),
        warnings,
        violations: ori3_cp::local_violations(&doc.cp),
        flat_fold_violations: Vec::new(),
        frame: None,
        skipped: Vec::new(),
        suspect_hinges: Vec::new(),
        contact_detected: false,
        sequence_targets: Vec::new(),
        angles: HashMap::new(),
        relaxations: Vec::new(),
        closure_rms: None,
        best_effort: false,
        converged: true,
        fold_through_proposal: None,
        fold_target_info: None,
    }
}

struct FoldTargetLookup {
    info: FoldTargetInfo,
    analysis: Option<ori3_layers::FoldTargetAnalysis>,
    warnings: Vec<String>,
}

fn unavailable_fold_target_info() -> FoldTargetInfo {
    FoldTargetInfo {
        status: FoldTargetStatus::Unavailable,
        available_count: None,
        reason: Some("この折り線で同時に折れるひだを確認できません。".to_string()),
        top_action: None,
    }
}

fn fold_target_info_at(
    doc: &Document,
    faces: &[Face],
    up_to: usize,
    line: [[f64; 2]; 2],
    keep_side_point: [f64; 2],
    pose_before: Option<&ori3_model::FoldPoseInput>,
) -> FoldTargetLookup {
    let Ok((analysis, warnings)) =
        ori3_layers::fold_target_analysis_at(doc, faces, up_to, line, keep_side_point, pose_before)
    else {
        return FoldTargetLookup {
            info: unavailable_fold_target_info(),
            analysis: None,
            warnings: Vec::new(),
        };
    };

    let info = fold_target_info_from_analysis(&analysis);
    let keep_analysis = info.status != FoldTargetStatus::Unavailable;
    FoldTargetLookup {
        info,
        analysis: keep_analysis.then_some(analysis),
        warnings,
    }
}

fn fold_target_info_from_analysis(analysis: &ori3_layers::FoldTargetAnalysis) -> FoldTargetInfo {
    let sections = &analysis.pleats.sections;
    let Some(available_count) = analysis.pleats.scalar_count else {
        return FoldTargetInfo {
            status: FoldTargetStatus::Varies,
            available_count: None,
            reason: Some("折り線の場所によって、同時に折れるひだの枚数が異なります。".to_string()),
            top_action: None,
        };
    };
    if sections.is_empty() {
        return unavailable_fold_target_info();
    }

    let all_crease_only = available_count == 0
        && sections.iter().all(|section| {
            section.pairs_top_to_bottom.is_empty()
                && section.count_limit.is_none()
                && matches!(
                    section.top_action,
                    Some(ori3_layers::TopAction::CreaseOnlyTop { .. })
                )
        });
    if all_crease_only {
        return FoldTargetInfo {
            status: FoldTargetStatus::CreaseOnlyTop,
            available_count: Some(0),
            reason: Some("いちばん上の紙が最後まで折り重なっていないため、今回はひだをまとめて折りません。いちばん上の紙に折り目だけを付け、下の紙と3Dの形は動かしません。".to_string()),
            top_action: Some(FoldTargetTopAction::CreaseOnlyTop),
        };
    }
    if available_count == 0 {
        return unavailable_fold_target_info();
    }

    let section_is_limited = |section: &ori3_layers::PleatSectionAnalysis| {
        section.top_action.is_none()
            && section.pairs_top_to_bottom.len() == available_count
            && section.count_limit
                == Some(ori3_layers::PleatCountLimit::IncompleteBoundaryAfter {
                    count: available_count,
                })
    };
    let section_is_ready = |section: &ori3_layers::PleatSectionAnalysis| {
        section.top_action.is_none()
            && section.count_limit.is_none()
            && section.pairs_top_to_bottom.len() == available_count
    };
    let all_ready_or_limited = sections
        .iter()
        .all(|section| section_is_ready(section) || section_is_limited(section));
    let any_limited = sections.iter().any(section_is_limited);
    let targets_are_safe = (1..=available_count)
        .all(|count| ori3_layers::target_faces_for_pleat_count(analysis, count).is_ok());
    if !targets_are_safe || !all_ready_or_limited {
        return unavailable_fold_target_info();
    }

    let (status, reason) = if any_limited {
        (
            FoldTargetStatus::Limited,
            Some(format!(
                "上から{available_count}枚まで選べます。{available_count}枚目の下は、まだ最後まで折り重なっていません。"
            )),
        )
    } else {
        (FoldTargetStatus::Ready, None)
    };
    FoldTargetInfo {
        status,
        available_count: Some(available_count),
        reason,
        top_action: None,
    }
}

fn fold_target_faces_at(
    doc: &Document,
    faces: &[Face],
    up_to: usize,
    line: [[f64; 2]; 2],
    keep_side_point: [f64; 2],
    pose_before: Option<&ori3_model::FoldPoseInput>,
    count: usize,
) -> Result<Vec<FaceId>, String> {
    let lookup = fold_target_info_at(doc, faces, up_to, line, keep_side_point, pose_before);
    let reason = lookup
        .info
        .reason
        .clone()
        .unwrap_or_else(|| "この枚数のひだを折れません。".to_string());
    let analysis = lookup.analysis.ok_or(reason.clone())?;
    if !matches!(
        lookup.info.status,
        FoldTargetStatus::Ready | FoldTargetStatus::Limited
    ) {
        return Err(reason);
    }
    ori3_layers::target_faces_for_pleat_count(&analysis, count).map_err(|_| reason)
}

struct OneShotMaterialTopProvider {
    observation: Option<ori3_layers::TopSurfaceObservation>,
}

impl ori3_layers::TopSurfaceProvider for OneShotMaterialTopProvider {
    fn observe_from_top(
        &mut self,
        depth: usize,
    ) -> Result<ori3_layers::TopSurfaceObservation, ori3_layers::SpatialCreaseOnlyError> {
        // 利用者の決定: 最上紙が未完なら、その下に完全なひだがあっても探索しない。
        if depth != 0 {
            return Err(ori3_layers::SpatialCreaseOnlyError::AmbiguousTopSurface);
        }
        self.observation
            .take()
            .ok_or(ori3_layers::SpatialCreaseOnlyError::AmbiguousTopSurface)
    }
}

fn material_top_surface_provider(
    cp: &CreasePattern,
    faces: &[Face],
    pose: &ori3_layers::CanonicalNonflatPose,
    material_point: [f64; 2],
) -> Result<OneShotMaterialTopProvider, String> {
    let observation = material_top_surface_observation(cp, faces, pose, material_point)
        .map_err(material_crease_only_error_message)?;
    Ok(OneShotMaterialTopProvider {
        observation: Some(observation),
    })
}

fn material_top_surface_observation(
    cp: &CreasePattern,
    faces: &[Face],
    pose: &ori3_layers::CanonicalNonflatPose,
    material_point: [f64; 2],
) -> Result<ori3_layers::TopSurfaceObservation, ori3_layers::SpatialCreaseOnlyError> {
    use ori3_layers::{SpatialCreaseOnlyError, SurfaceRelationFromTop, TopSurfaceObservation};

    if material_point
        .iter()
        .any(|coordinate| !coordinate.is_finite())
    {
        return Err(SpatialCreaseOnlyError::NonFiniteInput);
    }

    let mut angle_by_edge = HashMap::new();
    for &(edge, angle) in &pose.signed_hinge_angles {
        if !angle.is_finite() {
            return Err(SpatialCreaseOnlyError::InvalidTopRelation);
        }
        if let Some(previous) = angle_by_edge.insert(edge, angle)
            && previous.to_bits() != angle.to_bits()
        {
            return Err(SpatialCreaseOnlyError::InvalidTopRelation);
        }
    }

    let mut owners = HashMap::<EdgeId, Vec<FaceId>>::new();
    for face in faces {
        for &edge in &face.edges {
            owners.entry(edge).or_default().push(face.id);
        }
    }
    if owners.values().any(|edge_owners| edge_owners.len() > 2) {
        return Err(SpatialCreaseOnlyError::AmbiguousTopSurface);
    }

    let candidates = faces
        .iter()
        .filter(|face| ori3_layers::point_in_face(cp, face, material_point))
        .map(|face| face.id)
        .collect::<Vec<_>>();
    let Some(&seed) = candidates.first() else {
        return Err(SpatialCreaseOnlyError::MaterialKeepSidePointOutsidePaper);
    };

    let mut zero_neighbors = HashMap::<FaceId, Vec<FaceId>>::new();
    for (&edge, edge_owners) in &owners {
        if edge_owners.len() != 2
            || !angle_by_edge
                .get(&edge)
                .copied()
                .is_some_and(|angle| angle == 0.0)
        {
            continue;
        }
        zero_neighbors
            .entry(edge_owners[0])
            .or_default()
            .push(edge_owners[1]);
        zero_neighbors
            .entry(edge_owners[1])
            .or_default()
            .push(edge_owners[0]);
    }

    let mut selected = HashSet::from([seed]);
    let mut queue = VecDeque::from([seed]);
    while let Some(face) = queue.pop_front() {
        for &neighbor in zero_neighbors.get(&face).into_iter().flatten() {
            if selected.insert(neighbor) {
                queue.push_back(neighbor);
            }
        }
    }
    if candidates.iter().any(|face| !selected.contains(face)) {
        return Err(SpatialCreaseOnlyError::AmbiguousTopSurface);
    }

    let mut relation = None;
    for (&edge, edge_owners) in &owners {
        if edge_owners.len() != 2 {
            continue;
        }
        let first_selected = selected.contains(&edge_owners[0]);
        let second_selected = selected.contains(&edge_owners[1]);
        if first_selected == second_selected {
            continue;
        }
        let next = angle_by_edge
            .get(&edge)
            .copied()
            .map(classify_material_top_relation)
            .unwrap_or(SurfaceRelationFromTop::Missing);
        if let Some(current) = relation {
            if !same_material_top_relation(current, next) {
                return Err(SpatialCreaseOnlyError::AmbiguousTopSurface);
            }
        } else {
            relation = Some(next);
        }
    }

    // facesの材料順を保つだけで、面IDの大小を最上面の選択には使わない。
    let surface_faces = faces
        .iter()
        .filter(|face| selected.contains(&face.id))
        .map(|face| face.id)
        .collect();
    Ok(TopSurfaceObservation {
        surface_faces,
        relation_to_next: relation.unwrap_or(SurfaceRelationFromTop::Missing),
    })
}

fn classify_material_top_relation(angle: f64) -> ori3_layers::SurfaceRelationFromTop {
    use ori3_layers::SurfaceRelationFromTop;

    if angle == 0.0 {
        return SurfaceRelationFromTop::Zero;
    }
    let positive_delta = angle - 180.0;
    if (-ori3_layers::COMPLETE_FOLD_ENDPOINT_EPS_DEG..=ori3_layers::COMPLETE_FOLD_ENDPOINT_EPS_DEG)
        .contains(&positive_delta)
    {
        return SurfaceRelationFromTop::CompletePositive180;
    }
    let negative_delta = angle + 180.0;
    if (-ori3_layers::COMPLETE_FOLD_ENDPOINT_EPS_DEG..=ori3_layers::COMPLETE_FOLD_ENDPOINT_EPS_DEG)
        .contains(&negative_delta)
    {
        return SurfaceRelationFromTop::CompleteNegative180;
    }
    SurfaceRelationFromTop::Incomplete {
        signed_angle_deg: angle,
    }
}

fn same_material_top_relation(
    first: ori3_layers::SurfaceRelationFromTop,
    second: ori3_layers::SurfaceRelationFromTop,
) -> bool {
    use ori3_layers::SurfaceRelationFromTop;

    match (first, second) {
        (
            SurfaceRelationFromTop::Incomplete {
                signed_angle_deg: first,
            },
            SurfaceRelationFromTop::Incomplete {
                signed_angle_deg: second,
            },
        ) => {
            let delta = first - second;
            (-ori3_layers::COMPLETE_FOLD_ENDPOINT_EPS_DEG
                ..=ori3_layers::COMPLETE_FOLD_ENDPOINT_EPS_DEG)
                .contains(&delta)
        }
        _ => first == second,
    }
}

fn nonflat_pose_step_from_input(
    cp: &CreasePattern,
    input: &ori3_model::FoldPoseInput,
) -> Result<FoldStep, String> {
    let vertices = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.pos))
        .collect::<HashMap<_, _>>();
    let edges = cp
        .edges
        .iter()
        .map(|edge| (edge.id, edge))
        .collect::<HashMap<_, _>>();
    let mut drivers = Vec::with_capacity(input.drivers.len());
    for requested in &input.drivers {
        let edge = edges
            .get(&requested.edge_id)
            .ok_or_else(|| "折った形を再現する折り目が見つかりません".to_string())?;
        let a = vertices
            .get(&edge.v0)
            .copied()
            .ok_or_else(|| "折った形を再現する折り目の端が見つかりません".to_string())?;
        let b = vertices
            .get(&edge.v1)
            .copied()
            .ok_or_else(|| "折った形を再現する折り目の端が見つかりません".to_string())?;
        drivers.push(ori3_model::DriverLine {
            a,
            b,
            // 利用者の宣言を生のまま保存する。+180/-180や+90/-90を周期化しない。
            target_angle_deg: requested.target_angle_deg,
        });
    }
    Ok(FoldStep {
        id: 0,
        kind: TechniqueKind::Pose,
        drivers,
        layer_order: None,
        alignment: None,
        finish_soft: None,
        note: "折った形を再現してから折り目を付ける".to_string(),
    })
}

fn material_crease_only_error_message(error: ori3_layers::SpatialCreaseOnlyError) -> String {
    use ori3_layers::SpatialCreaseOnlyError;

    match error {
        SpatialCreaseOnlyError::DegenerateMaterialLine => {
            "折り線の2点が同じため、折り目を付けられません。".to_string()
        }
        SpatialCreaseOnlyError::NonFiniteInput => {
            "折り線の位置を読み取れないため、折り目を付けられません。".to_string()
        }
        SpatialCreaseOnlyError::MaterialKeepSidePointOnBoundary => {
            "残す側の点が紙のふちにあるため、どちら側か決められません。".to_string()
        }
        SpatialCreaseOnlyError::MaterialKeepSidePointOutsidePaper => {
            "残す側の点が紙の外にあるため、折り目を付けられません。".to_string()
        }
        SpatialCreaseOnlyError::MaterialLineMismatchAcrossSurfaceFaces => {
            "折り線がいちばん上の紙で1本につながらないため、折り目を付けられません。".to_string()
        }
        SpatialCreaseOnlyError::PartialInsertion => {
            "いちばん上の紙の全体へ折り目を付けられないため、変更しませんでした。".to_string()
        }
        SpatialCreaseOnlyError::AmbiguousTopSurface
        | SpatialCreaseOnlyError::MissingTopRelation
        | SpatialCreaseOnlyError::ZeroTopRelation
        | SpatialCreaseOnlyError::CompleteTopRelation
        | SpatialCreaseOnlyError::InvalidTopRelation
        | SpatialCreaseOnlyError::NotImplemented => {
            "いちばん上の紙が折り途中だと確認できないため、折り目を付けられません。".to_string()
        }
    }
}

fn material_fold_target_info_at(
    doc: &Document,
    faces: &[Face],
    up_to: usize,
    material_line: [[f64; 2]; 2],
    material_keep_side_point: [f64; 2],
    pose_before: Option<&ori3_model::FoldPoseInput>,
) -> FoldTargetLookup {
    let Ok(pose) = ori3_layers::replay::canonical_nonflat_pose_at(doc, faces, up_to, pose_before)
    else {
        return FoldTargetLookup {
            info: unavailable_fold_target_info(),
            analysis: None,
            warnings: Vec::new(),
        };
    };
    let warnings = pose.frame.warnings.clone();
    let Ok(mut provider) =
        material_top_surface_provider(&doc.cp, faces, &pose, material_keep_side_point)
    else {
        return FoldTargetLookup {
            info: unavailable_fold_target_info(),
            analysis: None,
            warnings,
        };
    };
    let result = ori3_layers::crease_only_top_from_material_line(
        &doc.cp,
        faces,
        &pose,
        &ori3_layers::SpatialCreaseOnlyInput {
            material_line,
            material_keep_side_point,
            direction: ori3_model::FoldDirection::Up,
        },
        &mut provider,
    );
    let info = if result.is_ok() {
        FoldTargetInfo {
            status: FoldTargetStatus::CreaseOnlyTop,
            available_count: Some(0),
            reason: Some("いちばん上の紙が最後まで折り重なっていないため、今回はひだをまとめて折りません。いちばん上の紙に折り目だけを付け、下の紙と3Dの形は動かしません。".to_string()),
            top_action: Some(FoldTargetTopAction::CreaseOnlyTop),
        }
    } else {
        unavailable_fold_target_info()
    };
    FoldTargetLookup {
        info,
        analysis: None,
        warnings,
    }
}

fn verify_direct_crease_only_pose_is_unchanged(
    pose: &ori3_layers::CanonicalNonflatPose,
    result: &ori3_layers::SpatialCreaseOnlyResult,
) -> Result<(), String> {
    let result_vertices = result
        .material_vertices
        .iter()
        .map(|vertex| (vertex.vertex, vertex.position.map(f64::to_bits)))
        .collect::<HashMap<_, _>>();
    for vertex in &pose.material_vertices {
        if result_vertices.get(&vertex.vertex) != Some(&vertex.position.map(f64::to_bits)) {
            return Err("折り目を付ける前後で3Dの形が動いたため、変更しませんでした。".to_string());
        }
    }
    Ok(())
}

fn verify_cold_crease_only_replay(
    pose: &ori3_layers::CanonicalNonflatPose,
    direct_vertices: &[ori3_layers::MaterialVertex3D],
    inserted_step_ids: &[StepId],
    material_faces: &[Face],
    cold: &ori3_layers::ReplayResult,
) -> Result<(), String> {
    // 2026-08-26実測: +90°/-90°、0°で連結した3面、保存prefix由来の非平坦姿勢、
    // 後続手順を持つ途中挿入の5標本で、旧Vertexの最大ずれは
    // 7.85046229341887583e-17。この実測が上限の約78.5%(およそ8割)になる
    // 1.0e-16を境目とする。標本の最短材料線0.25に対して2.5e15分の1であり、
    // 少なくとも実測した材料形状の位置差を吸収しない桁に留める。
    const MAX_COLD_REPLAY_OLD_VERTEX_DRIFT: f64 = 1.0e-16;

    if cold
        .skipped
        .iter()
        .any(|step| inserted_step_ids.contains(step))
    {
        return Err("保存した折った形と折り目を読み直せないため、変更しませんでした。".to_string());
    }
    if !cold.converged || cold.best_effort {
        return Err("保存した3Dの形を最後まで読み直せないため、変更しませんでした。".to_string());
    }
    let frame = &cold.frame;
    let material_face_ids = material_faces
        .iter()
        .map(|face| face.id)
        .collect::<HashSet<_>>();
    if material_face_ids.len() != material_faces.len() || frame.faces.len() != material_faces.len()
    {
        return Err("保存した3Dの紙面が過不足なく揃わないため、変更しませんでした。".to_string());
    }
    let expected = pose
        .material_vertices
        .iter()
        .map(|vertex| (vertex.vertex, vertex.position))
        .collect::<HashMap<_, _>>();
    let direct = direct_vertices
        .iter()
        .map(|vertex| (vertex.vertex, vertex.position))
        .collect::<HashMap<_, _>>();
    let mut seen_faces = HashSet::new();
    let mut seen_vertices = HashSet::new();
    let mut max_drift = 0.0_f64;
    for spatial_face in &frame.faces {
        if !seen_faces.insert(spatial_face.face) {
            return Err("保存した3Dの紙面が重複しているため、変更しませんでした。".to_string());
        }
        let material_face = material_faces
            .iter()
            .find(|face| face.id == spatial_face.face)
            .ok_or_else(|| {
                "保存した3Dの紙面を読み直せないため、変更しませんでした。".to_string()
            })?;
        if material_face.vertices.len() != spatial_face.polygon.len() {
            return Err("保存した3Dの紙面を読み直せないため、変更しませんでした。".to_string());
        }
        for (&vertex, &actual) in material_face.vertices.iter().zip(&spatial_face.polygon) {
            let Some(expected_position) = expected.get(&vertex) else {
                continue;
            };
            let Some(direct_position) = direct.get(&vertex) else {
                return Err(
                    "折り目を付けた直後の3Dの形を確認できないため、変更しませんでした。"
                        .to_string(),
                );
            };
            if direct_position.map(f64::to_bits) != expected_position.map(f64::to_bits) {
                return Err(
                    "折り目を付ける処理で3Dの形が動いたため、変更しませんでした。".to_string(),
                );
            }
            let squared = actual
                .into_iter()
                .zip(*expected_position)
                .map(|(left, right)| {
                    let delta = left - right;
                    delta * delta
                })
                .sum::<f64>();
            if !squared.is_finite() {
                return Err(
                    "保存した3Dの形を有限な位置で読み直せないため、変更しませんでした。"
                        .to_string(),
                );
            }
            max_drift = max_drift.max(squared.sqrt());
            seen_vertices.insert(vertex);
        }
    }
    if seen_faces != material_face_ids {
        return Err("保存した3Dの紙面が元の紙と一致しないため、変更しませんでした。".to_string());
    }
    if expected
        .keys()
        .any(|vertex| !seen_vertices.contains(vertex))
    {
        return Err("保存した3Dの形に元の紙の点が揃わないため、変更しませんでした。".to_string());
    }

    #[cfg(test)]
    eprintln!("stage3 cold replay max old-vertex drift = {max_drift:.17e}");
    if max_drift > MAX_COLD_REPLAY_OLD_VERTEX_DRIFT {
        return Err("保存した3Dの形で元の紙の点が動いたため、変更しませんでした。".to_string());
    }
    Ok(())
}

/// MoveStep候補の完全な返却viewを、store確定より前に導出する。
///
/// 通常操作のreplayはcommandがロック解放後に付けるが、MoveStepではその順序だと
/// replay panic後に「commandはErr、storeは変更済み」になり得る。この操作だけは
/// 同じ候補cloneからreplayまで作り終え、そのviewを`commit_prebuilt`へ渡す。
fn build_move_step_view(doc: &Document, step_creases: &[StepCreases]) -> DocumentView {
    let mut view = build_view(doc, step_creases, Vec::new());
    attach_replay(&mut view);
    // replayを含む全候補導出の直後、commit直前の最終境界へ置く。
    // panicしてもviewは局所値で、まだselfへ一切触れていない。
    fail_move_step_derivation_if_requested();
    view
}

/// ±180°の指定と実角を照合するときの許容差(度)。
/// 既存の角度緩和診断が記録を始める `1e-6°` と同じ値にそろえる。
const FLAT_TARGET_EPS_DEG: f64 = 1e-6;

/// 姿勢計算で既に得た接触情報を、平坦折り通知に使う条件へ絞る。
///
/// `contact_detected` は補正前の途中姿勢も含む累積値である。全折り目を一度に完成角へ
/// 補間すると、正しく畳める作品でも途中だけ接触して最終交差0組になることがあるため、
/// その値は一部の折り目をまとめて動かした操作だけで採用する。補正後の最終姿勢に
/// 交差が残る場合は、要求範囲によらず常に採用する。
pub(crate) fn pose_flat_fold_notice_intersects(
    cp: &CreasePattern,
    targets: &[Driver],
    contact_detected: bool,
    final_intersects: bool,
) -> bool {
    if final_intersects {
        return true;
    }
    if !contact_detected {
        return false;
    }

    let latest_targets: HashMap<EdgeId, f64> = targets
        .iter()
        .map(|target| (target.hinge, target.target_angle_deg))
        .collect();
    let mut crease_edges = cp
        .edges
        .iter()
        .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .peekable();
    let requests_every_crease_flat = crease_edges.peek().is_some()
        && crease_edges.all(|edge| {
            latest_targets.get(&edge.id).is_some_and(|target| {
                target.is_finite() && (target.abs() - 180.0).abs() <= FLAT_TARGET_EPS_DEG
            })
        });
    !requests_every_crease_flat
}

/// 今回±180°を求めた折り目だけで局所平坦折り条件を検査し、かつそのうち1本でも
/// 指定角へ届かなかったか、紙の食い込みが検出された場合に通知対象の点を返す。
///
/// 検査用CPは複製して作り、元のCPも操作結果も変更しない。通知は表示情報だけであり、
/// 呼び出し側が姿勢計算や手順再生を止める条件にはしない。
pub(crate) fn flat_fold_notice_violations(
    cp: &CreasePattern,
    targets: &[Driver],
    angles: &HashMap<EdgeId, f64>,
    paper_intersects: bool,
) -> Vec<VertexId> {
    // 先に全要求を辺ごとにまとめてから±180°へ絞る。同じ辺が複数回現れた場合は
    // 後の要求を優先するため、pose_solveで後置したhardがpreferredを確実に上書きする。
    let mut flat_targets: HashMap<EdgeId, f64> = targets
        .iter()
        .map(|target| (target.hinge, target.target_angle_deg))
        .collect();
    flat_targets.retain(|_, target| {
        target.is_finite() && (target.abs() - 180.0).abs() <= FLAT_TARGET_EPS_DEG
    });
    let edge_kinds: HashMap<EdgeId, EdgeKind> =
        cp.edges.iter().map(|edge| (edge.id, edge.kind)).collect();
    flat_targets.retain(|hinge, _| {
        edge_kinds
            .get(hinge)
            .is_some_and(|kind| *kind != EdgeKind::Border)
    });
    if flat_targets.is_empty() {
        return Vec::new();
    }

    let mut requested_cp = cp.clone();
    for edge in &mut requested_cp.edges {
        if edge.kind == EdgeKind::Border {
            continue;
        }
        edge.kind = match flat_targets.get(&edge.id) {
            Some(target) if target.is_sign_positive() => EdgeKind::Mountain,
            Some(_) => EdgeKind::Valley,
            None => EdgeKind::Aux,
        };
    }
    let violations = ori3_cp::local_violations(&requested_cp);
    if violations.is_empty() {
        return violations;
    }

    let missed = flat_targets.iter().any(|(hinge, target)| {
        angles.get(hinge).is_none_or(|actual| {
            !actual.is_finite() || (actual - target).abs() > FLAT_TARGET_EPS_DEG
        })
    });
    if missed || paper_intersects {
        violations
    } else {
        Vec::new()
    }
}

/// 手順再生で既に計算した最終フレームの交差組を、そのまま通知条件へ接続する。
/// 呼び出し側で自己交差判定を増やさず、姿勢・手順は結果にかかわらず返す。
pub(crate) fn replay_flat_fold_notice_violations(
    cp: &CreasePattern,
    targets: &[Driver],
    angles: &HashMap<EdgeId, f64>,
    intersections: &[(FaceId, FaceId)],
) -> Vec<VertexId> {
    flat_fold_notice_violations(cp, targets, angles, !intersections.is_empty())
}

/// 手順再生で既に得た最終交差組を、DocumentViewの診断値へ運ぶ。
/// 自己交差判定は呼び出し側の1回だけとし、ここでは結果の有無だけを写す。
fn attach_replay_contact_diagnostic(view: &mut DocumentView, intersections: &[(FaceId, FaceId)]) {
    view.contact_detected = !intersections.is_empty();
}

/// `surface_rank` が全面の0始まり連番なら、下→上の面順へ戻す。
///
/// 古いsnapshotの全0や、欠落・重複faceを物理順として信頼しないため、face IDとrankの
/// 両方が完全に一意で、rankがちょうど`0..faces.len()`のときだけ返す。
pub(crate) fn frame_surface_rank_order(frame: &Frame3D) -> Option<Vec<FaceId>> {
    let mut face_ids = HashSet::with_capacity(frame.faces.len());
    let mut ranked = Vec::with_capacity(frame.faces.len());
    for face in &frame.faces {
        if !face_ids.insert(face.face) {
            return None;
        }
        ranked.push((face.surface_rank, face.face));
    }
    ranked.sort_unstable();
    if ranked
        .iter()
        .enumerate()
        .any(|(index, (rank, _))| u32::try_from(index).ok() != Some(*rank))
    {
        return None;
    }
    Some(ranked.into_iter().map(|(_, face)| face).collect())
}

/// completeな幾何導出のproofと完全rankが両方ある再生結果だけを物理順へ戻す。
///
/// `frame_surface_rank_order` は古いframeを弾く構造検査にすぎない。canonical導出に
/// 失敗したframeにもmaterial seedの完全順列は入るため、provenance無しではPBDや
/// softのauthorityにしてはならない。
pub(crate) fn replay_surface_rank_order(
    replayed: &ori3_layers::ReplayResult,
) -> Option<Vec<FaceId>> {
    replayed.surface_order_provenance.as_ref()?;
    frame_surface_rank_order(&replayed.frame)
}

/// proof付き再生rankがあるときだけ、表示用の重なり補正を適用する。
/// 保存layer/orderは編集用の論理状態であり、このfallbackには使わない。
pub(crate) fn prevent_replay_overlap_if_authoritative(
    cp: &CreasePattern,
    faces: &[Face],
    replayed: &mut ori3_layers::ReplayResult,
    settings: &ori3_soft::OverlapSettings,
) -> Option<ori3_soft::OverlapReport> {
    let order = replay_surface_rank_order(replayed)?;
    let progress = replayed.layer_transition.progress;
    Some(ori3_soft::prevent_overlap_with_order_authority(
        cp,
        faces,
        &mut replayed.frame,
        ori3_soft::OverlapOrderInput {
            start: &order,
            end: &order,
            progress,
            authoritative: true,
        },
        settings,
    ))
}

/// ビューへ手順の自動再生結果(立体・飛ばした手順・警告)を載せる
/// (SEQ-004「展開図編集後、手順を自動再生して最新状態を表示」)。
/// 手順が空のときは再生するものが無いので `frame: None` のまま
/// (平らな姿勢はフロントが展開図から直接描ける)。
///
/// 設計規約: これは重い計算(面400・10手順でrelease約23ms)なので、通常操作は
/// storeのロックを取らないコマンド層から、ロック解放後に呼ぶ。例外はMoveStepで、
/// Err時の原子性を守るため候補cloneへcommit前に呼び、導出済みviewをそのまま返す。
/// 再生には `view.faces`(同じdocから導出済み)を渡し、面抽出を二重に行わない。
pub fn attach_replay(view: &mut DocumentView) {
    attach_replay_contact_diagnostic(view, &[]);
    if view.doc.sequence.is_empty() {
        return;
    }
    let up_to = view.doc.sequence.len();
    let mut replayed = ori3_layers::replay_with_faces(&view.doc, &view.faces, up_to, 1.0);
    let saved_order = ori3_layers::saved_layer_order_at(&view.doc, &view.faces, up_to, 1.0);
    let canonical_order = replay_surface_rank_order(&replayed);
    view.sequence_targets = replayed.sequence_targets.clone();
    view.angles = replayed.hinge_angles.clone();
    view.relaxations = replayed.relaxations.clone();
    view.closure_rms = replayed
        .closure_rms
        .is_finite()
        .then_some(replayed.closure_rms);
    view.best_effort = replayed.best_effort;
    view.converged = replayed.converged;
    let mut penetration_warnings: Vec<&'static str> = Vec::new();
    if saved_order.is_none() {
        let warning = apply_layer_order_display_settings(
            &view.doc.cp,
            &view.faces,
            &mut replayed.frame,
            canonical_order.as_deref(),
            view.doc.display.overlap_prevention_enabled,
            view.doc.display.penetration_prevention_enabled,
        );
        if let Some(warning) = warning {
            penetration_warnings.push(warning);
        }
    }
    // 検出と補正は独立した設定である。両方が有効な場合も、補正で消える前の
    // 利用者指定の姿勢を診断し、その結果を警告と原因候補へ残す。
    let intersections = if view.doc.display.penetration_prevention_enabled {
        ori3_rigid::self_intersection_pairs(&replayed.frame)
    } else {
        Vec::new()
    };
    attach_replay_contact_diagnostic(view, &intersections);
    let overlap_settings = ori3_soft::OverlapSettings {
        enabled: view.doc.display.overlap_prevention_enabled,
        ..Default::default()
    };
    let _ = prevent_replay_overlap_if_authoritative(
        &view.doc.cp,
        &view.faces,
        &mut replayed,
        &overlap_settings,
    );
    view.flat_fold_violations = replay_flat_fold_notice_violations(
        &view.doc.cp,
        &replayed.sequence_targets,
        &replayed.hinge_angles,
        &intersections,
    );
    replayed.suspect_hinges = ori3_rigid::suspect_hinges_for_intersections(
        &view.doc.cp,
        &view.faces,
        &intersections,
        &replayed.driver_hinges,
    );
    // 紙のめり込み(SIM-007)。折り上がりは平ら(z≒0)なので通常は出ないが、
    // 平らに畳みきれない形では立体のまま返るため、そのときに知らせる
    penetration_warnings.extend(add_penetration_warning_for_intersections(
        &view.doc.cp,
        &view.faces,
        &mut replayed.frame,
        false,
        &intersections,
    ));
    for warning in penetration_warnings {
        if !replayed.warnings.iter().any(|existing| existing == warning) {
            replayed.warnings.push(warning.to_string());
        }
    }
    filter_penetration_warnings(
        &mut view.warnings,
        view.doc.display.penetration_prevention_enabled,
    );
    filter_penetration_warnings(
        &mut replayed.warnings,
        view.doc.display.penetration_prevention_enabled,
    );
    filter_penetration_warnings(
        &mut replayed.frame.warnings,
        view.doc.display.penetration_prevention_enabled,
    );
    view.warnings.extend(replayed.warnings);
    view.skipped = replayed.skipped;
    view.suspect_hinges = replayed.suspect_hinges;
    view.frame = Some(replayed.frame);
}

/// 立体交差、または折り切り時の山谷と層順序の矛盾をフレームへ足す。
/// 厳密な防止はせず、気づけるようにするだけ(「止めずに警告」原則)。
/// `check_layer_order` はt=1の平坦状態だけでtrueにする。
pub fn add_penetration_warning(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &mut Frame3D,
    check_layer_order: bool,
) -> Vec<&'static str> {
    let intersections = ori3_rigid::self_intersection_pairs(frame);
    add_penetration_warning_for_intersections(cp, faces, frame, check_layer_order, &intersections)
}

/// 交差面ペアを既に求めた追従・再生経路向けの警告付与。
/// 候補抽出と同じ結果を共有し、重い三角形交差判定を二重に走らせない。
pub fn add_penetration_warning_for_intersections(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &mut Frame3D,
    check_layer_order: bool,
    intersections: &[(FaceId, FaceId)],
) -> Vec<&'static str> {
    let mut added = Vec::new();
    if !intersections.is_empty()
        && !frame
            .warnings
            .iter()
            .any(|warning| warning == ori3_rigid::PENETRATION_WARNING)
    {
        frame
            .warnings
            .push(ori3_rigid::PENETRATION_WARNING.to_string());
        added.push(ori3_rigid::PENETRATION_WARNING);
    }
    if check_layer_order && let Some(warning) = add_layer_order_warning(cp, faces, frame) {
        added.push(warning);
    }
    added
}

/// 食い込み検出がOFFなら、下位層が操作診断として作った貫通警告も画面へ漏らさない。
/// 収束・裂けなど別種の警告はそのまま残す。
pub(crate) fn filter_penetration_warnings(warnings: &mut Vec<String>, detect: bool) {
    if detect {
        return;
    }
    warnings.retain(|warning| {
        warning != ori3_rigid::PENETRATION_WARNING
            && warning != ori3_layers::FOLD_PENETRATION_WARNING
    });
}

/// 接触補正でzが動く前の平坦フレームで、紙の重なり順を形に合わせる。
///
/// 手順を記録せず角度だけで折ると重なり順が決まらず、同じ平面の面が完全に同じ位置へ
/// 描かれて裏面が見えたり貫通して見える。まず折り上がった形から重なり順を求めて直し、
/// それでも矛盾が残るときだけ警告を足す。
pub(crate) fn add_layer_order_warning(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &mut Frame3D,
) -> Option<&'static str> {
    correct_layer_order(cp, faces, frame);
    add_layer_order_warning_only(cp, faces, frame)
}

/// 形から導いた層順序を後続手順用の`layer`へ反映する。警告は追加しない。
fn correct_layer_order(cp: &CreasePattern, faces: &[Face], frame: &mut Frame3D) {
    if ori3_rigid::layer_order_conflicts(cp, faces, frame)
        && let Some(order) = ori3_rigid::derive_layer_order(cp, faces, frame)
    {
        let rank: HashMap<FaceId, u32> = order
            .iter()
            .enumerate()
            .map(|(index, &id)| (id, u32::try_from(index).unwrap_or(u32::MAX)))
            .collect();
        for face in &mut frame.faces {
            if let Some(&layer) = rank.get(&face.face) {
                face.layer = layer;
            }
        }
    }
}

/// 現在の層順序の矛盾を警告するだけで、frameの形・layer・surface_rankを変えない。
fn add_layer_order_warning_only(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &mut Frame3D,
) -> Option<&'static str> {
    if !ori3_rigid::layer_order_conflicts(cp, faces, frame)
        || frame
            .warnings
            .iter()
            .any(|warning| warning == ori3_layers::FOLD_PENETRATION_WARNING)
    {
        return None;
    }
    frame
        .warnings
        .push(ori3_layers::FOLD_PENETRATION_WARNING.to_string());
    Some(ori3_layers::FOLD_PENETRATION_WARNING)
}

/// 画面の既存2設定を層順序へ適用する。
///
/// `prevent`だけがlayer/surface_rankを補正し、`detect`は警告を追加するだけである。
/// 検証済みcanonical順の刻印も、利用者が補正を明示した場合に限る。
pub(crate) fn apply_layer_order_display_settings(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &mut Frame3D,
    canonical_order: Option<&[FaceId]>,
    prevent: bool,
    detect: bool,
) -> Option<&'static str> {
    if prevent {
        correct_layer_order(cp, faces, frame);
        if let Some(order) = canonical_order {
            ori3_rigid::stamp_surface_order(frame, order)
                .expect("検証済みcanonical順は同じframeへ刻印できる");
        }
    }
    detect
        .then(|| add_layer_order_warning_only(cp, faces, frame))
        .flatten()
}

/// 形からの旧layer補助を適用し、検証済みのsurface authorityを維持する。
///
/// 保存順が無い自由角度では、rigidが最終状態から導出した`surface_rank`が正本。
/// [`add_layer_order_warning`] は後続手順用の`layer`だけを補助する。ここでも呼出し側が
/// 検証したcanonical順を再確認して刻み、両フィールドの契約を混同させない。
#[cfg(test)]
pub(crate) fn add_layer_order_warning_preserving_surface_authority(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &mut Frame3D,
    canonical_order: &[FaceId],
) -> Option<&'static str> {
    let warning = add_layer_order_warning(cp, faces, frame);
    ori3_rigid::stamp_surface_order(frame, canonical_order)
        .expect("検証済みcanonical順は同じframeへ刻印できる");
    warning
}

fn check_paper(paper: &Paper) -> Result<(), String> {
    if paper.width_mm > 0.0 && paper.height_mm > 0.0 {
        Ok(())
    } else {
        Err("紙のサイズは正の値で指定してください".to_string())
    }
}

/// IPC用のserde型を、汎用層演算の内部型へ移す。
fn to_layer_motion_part(part: ori3_model::MotionPart) -> ori3_layers::MotionPart {
    ori3_layers::MotionPart {
        layers: part.layers,
        region: part
            .region
            .into_iter()
            .map(|half_plane| ori3_layers::HalfPlane {
                line: half_plane.line,
                inside_point: half_plane.inside_point,
            })
            .collect(),
        transform: match part.transform {
            ori3_model::MotionTransform::Stay => ori3_layers::MotionTransform::Stay,
            ori3_model::MotionTransform::Reflect(lines) => {
                ori3_layers::MotionTransform::Reflect(lines)
            }
        },
        turn: match part.turn {
            ori3_model::LayerTurn::Keep => ori3_layers::LayerTurn::Keep,
            ori3_model::LayerTurn::Outside(direction) => ori3_layers::LayerTurn::Outside(direction),
            ori3_model::LayerTurn::Inside(direction) => ori3_layers::LayerTurn::Inside(direction),
            ori3_model::LayerTurn::Beside { anchor, direction } => {
                ori3_layers::LayerTurn::Beside { anchor, direction }
            }
        },
        reverse_layers: part.reverse_layers,
    }
}

const NONFLAT_EPS: f64 = 1e-6;

fn frame_is_nonflat(frame: &Frame3D) -> bool {
    frame.faces.iter().any(|face| {
        face.polygon
            .iter()
            .any(|point| point[2].abs() > NONFLAT_EPS)
    })
}

fn spatial_fold_input(
    spatial: Option<&SpatialFoldSpec>,
    frame: &Frame3D,
    faces: &[Face],
    line: [[f64; 2]; 2],
    keep_side_point: [f64; 2],
    target_layers: Option<&[FaceId]>,
    direction: ori3_model::FoldDirection,
) -> ori3_layers::SpatialFoldInput {
    if let Some(spatial) = spatial {
        return ori3_layers::SpatialFoldInput {
            plane: ori3_layers::FoldPlane3D {
                origin: [
                    (spatial.from[0] + spatial.to[0]) * 0.5,
                    (spatial.from[1] + spatial.to[1]) * 0.5,
                    (spatial.from[2] + spatial.to[2]) * 0.5,
                ],
                normal: [
                    spatial.to[0] - spatial.from[0],
                    spatial.to[1] - spatial.from[1],
                    spatial.to[2] - spatial.from[2],
                ],
            },
            grab_point: spatial.from,
            grab_face: spatial.grab_face,
            direction,
        };
    }

    // 展開図画面など旧入力だけの呼び出しも、立体姿勢なら従来の2D折り線を
    // z方向へ延ばした平面として扱い、止めずに続ける。
    let dx = line[1][0] - line[0][0];
    let dy = line[1][1] - line[0][1];
    let normal = [-dy, dx, 0.0];
    let length = dx.hypot(dy);
    let keep_signed = normal[0] * (keep_side_point[0] - line[0][0])
        + normal[1] * (keep_side_point[1] - line[0][1]);
    let keep_sign = if keep_signed < 0.0 { -1.0 } else { 1.0 };
    let unit = if length > 0.0 {
        [normal[0] / length, normal[1] / length, 0.0]
    } else {
        [0.0, 0.0, 0.0]
    };
    let grab_point = [
        line[0][0] - keep_sign * unit[0] * 0.25,
        line[0][1] - keep_sign * unit[1] * 0.25,
        -keep_sign * unit[2] * 0.25,
    ];
    let requested = target_layers
        .into_iter()
        .flatten()
        .copied()
        .find(|id| faces.iter().any(|face| face.id == *id));
    let geometric = frame.faces.iter().find_map(|face| {
        face.polygon
            .iter()
            .any(|point| {
                -keep_sign
                    * (normal[0] * (point[0] - line[0][0]) + normal[1] * (point[1] - line[0][1]))
                    > ori3_model::EPS
            })
            .then_some(face.face)
    });
    ori3_layers::SpatialFoldInput {
        plane: ori3_layers::FoldPlane3D {
            origin: [line[0][0], line[0][1], 0.0],
            normal,
        },
        grab_point,
        grab_face: requested.or(geometric).unwrap_or(0),
        direction,
    }
}

/// 折り操作([`SeqOp::FoldThrough`] / [`SeqOp::FlatMotion`] / [`SeqOp::Technique`])の
/// 挿入位置を検査する。
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
/// 次の手順IDを決める。今ある手順だけでなく、消した手順の来歴が持つIDも避ける。
/// 避けないと、消した手順の来歴が新しい手順の来歴として読まれてしまう。
fn next_step_id(doc: &Document, step_creases: &[StepCreases]) -> StepId {
    doc.sequence
        .iter()
        .map(|s| s.id)
        .chain(step_creases.iter().map(|c| c.step))
        .max()
        .map_or(0, |m| m.saturating_add(1))
}

/// 今ある手順の分だけを残す(保存とフロントへの受け渡し用)。
fn retain_existing_steps(doc: &Document, step_creases: &[StepCreases]) -> Vec<StepCreases> {
    step_creases
        .iter()
        .filter(|creases| doc.sequence.iter().any(|step| step.id == creases.step))
        .cloned()
        .collect()
}

/// この折りで展開図へ**新しく足した**折り線を、CP座標の線分として取り出す。
///
/// 折る前から在った辺は除く。補助線から折り線へ昇格した辺は辺IDが変わらないので、
/// 「先に描いてあった線」として折る前の展開図にも残る。
fn added_crease_lines(
    before: &CreasePattern,
    after: &CreasePattern,
    added: &[EdgeId],
) -> Vec<[[f64; 2]; 2]> {
    let existing: HashSet<EdgeId> = before.edges.iter().map(|e| e.id).collect();
    let pos: HashMap<VertexId, [f64; 2]> = after.vertices.iter().map(|v| (v.id, v.pos)).collect();
    added
        .iter()
        .filter(|id| !existing.contains(id))
        .filter_map(|id| after.edges.iter().find(|e| e.id == *id))
        .filter_map(|e| Some([*pos.get(&e.v0)?, *pos.get(&e.v1)?]))
        .collect()
}

/// 画面から送られてきた手順の来歴を整える。
///
/// 画面が新しく作る手順は「仕上げの角度」(Pose)だけで、折り線は1本も足さない。
/// 空の来歴を残して、同じIDだった古い手順の来歴を引き継がないようにする。
/// 折りの手順が送られてくるのは並べ替え(削除+挿入)のときなので、
/// その場合は既にある来歴をそのまま残す。
fn record_frontend_step(list: &mut Vec<StepCreases>, step: &FoldStep) {
    if step.kind == TechniqueKind::Pose {
        record_step_creases(list, step.id, Vec::new());
    }
}

/// 手順1つ分の来歴を記録する。線を1本も足していない手順は空で記録し、
/// 「この手順は線を足していない」ことを証拠として残す(推測へ落とさない)。
fn record_step_creases(list: &mut Vec<StepCreases>, step: StepId, lines: Vec<[[f64; 2]; 2]>) {
    list.retain(|creases| creases.step != step);
    list.push(StepCreases { step, lines });
}

fn is_border(cp: &CreasePattern, id: u32) -> bool {
    cp.edges
        .iter()
        .any(|e| e.id == id && e.kind == EdgeKind::Border)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    use ori3_export::fold::{fold_to_document, parse_fold_1_2};
    use ori3_model::{
        AlignmentTarget, DisplaySettings, DriverLine, EPS, Edge, Face3D, FinishSoftSettings,
        FoldAlignment, FoldStep, TechniqueKind, Vertex,
    };
    use serde::Deserialize;

    mod movestep_contract;

    const FOLD_IMPORT_SUCCESS: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../crates/ori3-export/tests/fixtures/fold/flat-face-orders.fold"
    ));
    const FOLD_IMPORT_WARNINGS_20: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../crates/ori3-export/tests/fixtures/fold/unsupported-extensions.fold"
    ));
    const FOLD_IMPORT_WARNING_AND_ERROR: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../crates/ori3-export/tests/fixtures/fold/fu-assignments.fold"
    ));
    const FOLD_MALFORMED_100: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../crates/ori3-export/tests/fixtures/fold/malformed-store-cases.json"
    ));

    /// 利用者が2026-08-26に「線と線を合わせる」で折ろうとした13頂点・28辺の作品。
    /// 読み取り専用証拠 `03-store-state-readonly.json` のDocumentをそのまま固定した。
    /// 証拠SHA-256: B3EA53213BFC152D13C1A8173112B6A5E3DFEF2EF5773D9D5932DFF39C4BC4B7
    const CRANE_HEAD_DOCUMENT_JSON: &str = r#"{"schema_version":1,"paper":{"width_mm":150,"height_mm":150},"cp":{"vertices":[{"id":0,"pos":[0,0]},{"id":1,"pos":[1,0]},{"id":2,"pos":[1,1]},{"id":3,"pos":[0,1]},{"id":4,"pos":[0,0.5]},{"id":5,"pos":[1,0.5]},{"id":6,"pos":[0.5,0]},{"id":7,"pos":[0.5,1]},{"id":8,"pos":[0.5,0.5]},{"id":9,"pos":[0.7928932188134525,0.5]},{"id":10,"pos":[0.5,0.20710678118654752]},{"id":11,"pos":[0.20710678118654752,0.5]},{"id":12,"pos":[0.5,0.7928932188134525]}],"edges":[{"id":4,"v0":3,"v1":4,"kind":"Border"},{"id":5,"v0":4,"v1":0,"kind":"Border"},{"id":6,"v0":1,"v1":5,"kind":"Border"},{"id":7,"v0":5,"v1":2,"kind":"Border"},{"id":9,"v0":0,"v1":6,"kind":"Border"},{"id":10,"v0":6,"v1":1,"kind":"Border"},{"id":11,"v0":2,"v1":7,"kind":"Border"},{"id":12,"v0":7,"v1":3,"kind":"Border"},{"id":17,"v0":0,"v1":8,"kind":"Valley"},{"id":18,"v0":8,"v1":2,"kind":"Valley"},{"id":19,"v0":8,"v1":9,"kind":"Mountain"},{"id":20,"v0":9,"v1":5,"kind":"Mountain"},{"id":21,"v0":2,"v1":9,"kind":"Mountain"},{"id":22,"v0":9,"v1":1,"kind":"Mountain"},{"id":23,"v0":6,"v1":10,"kind":"Mountain"},{"id":24,"v0":10,"v1":8,"kind":"Mountain"},{"id":25,"v0":0,"v1":10,"kind":"Mountain"},{"id":26,"v0":10,"v1":1,"kind":"Mountain"},{"id":27,"v0":4,"v1":11,"kind":"Mountain"},{"id":28,"v0":11,"v1":8,"kind":"Mountain"},{"id":29,"v0":0,"v1":11,"kind":"Mountain"},{"id":30,"v0":11,"v1":3,"kind":"Mountain"},{"id":31,"v0":8,"v1":12,"kind":"Mountain"},{"id":32,"v0":12,"v1":7,"kind":"Mountain"},{"id":33,"v0":2,"v1":12,"kind":"Mountain"},{"id":34,"v0":12,"v1":3,"kind":"Mountain"},{"id":35,"v0":11,"v1":12,"kind":"Valley"},{"id":36,"v0":9,"v1":10,"kind":"Valley"}],"next_vertex_id":13,"next_edge_id":37},"sequence":[],"display":{"front_color":[237,28,36],"back_color":[255,255,255],"grid_divisions":8,"soft_enabled":false,"soft_stiffness":0.5,"soft_pressure":0,"overlap_prevention_enabled":false,"penetration_prevention_enabled":true}}"#;
    const CRANE_HEAD_POSE_DRIVERS: &[(EdgeId, f64)] = &[
        (17, -180.0),
        (18, -180.0),
        (21, 180.0),
        (22, 180.0),
        (25, 180.0),
        (26, 180.0),
        (29, 180.0),
        (30, 180.0),
        (33, 180.0),
        (34, 180.0),
        (35, -180.0),
        (36, -180.0),
    ];
    /// 読み取り専用証拠の完成frameから得た全ヒンジの平坦終点。
    /// 期待結果を独立に組み立てるtest oracle専用で、製品要求へ渡すのは上の12本だけ。
    const CRANE_HEAD_ORACLE_HINGE_ANGLES: &[(EdgeId, f64)] = &[
        (17, -180.0),
        (18, -180.0),
        (19, 180.0),
        (20, 0.0),
        (21, 180.0),
        (22, 180.0),
        (23, 0.0),
        (24, 180.0),
        (25, 180.0),
        (26, 180.0),
        (27, 0.0),
        (28, 180.0),
        (29, 180.0),
        (30, 180.0),
        (31, 180.0),
        (32, 0.0),
        (33, 180.0),
        (34, 180.0),
        (35, -180.0),
        (36, -180.0),
    ];
    const CRANE_HEAD_LINE: [[f64; 2]; 2] = [
        [0.19509032201612797, -0.9807852804032304],
        [-0.19509032201612797, 0.9807852804032304],
    ];
    const CRANE_HEAD_KEEP_POINT: [f64; 2] = [0.4903926402016152, 0.09754516100806399];
    const CRANE_HEAD_MOVING_FACES: &[FaceId] = &[2, 3, 6, 7, 10, 11, 12, 13, 14, 15];
    const CRANE_HEAD_CAPTURED_LAYER_ORDER: &[FaceId] =
        &[4, 5, 2, 13, 12, 9, 8, 1, 3, 11, 10, 14, 15, 0, 7, 6];
    // 以前の `CRANE_HEAD_CANONICAL_LAYER_ORDER` は2026-08-26のcanonical solve 1回が
    // 返した全順序だった。solve出力をgoldenにせず、下の検査で独立捕捉した物理順と
    // 正面積で重なる面対だけを比較する。重ならない面対のtotal-order tieは検査しない。
    // `pose_motion::overlap_witnesses` と同じ正面積閾値。
    const CRANE_HEAD_OVERLAP_AREA_EPS: f64 = 1e-14;

    const MALFORMED_CATEGORY_COUNT: usize = 10;
    const MALFORMED_CASES_PER_CATEGORY: usize = 10;
    const MALFORMED_TOTAL: usize = MALFORMED_CATEGORY_COUNT * MALFORMED_CASES_PER_CATEGORY;
    // 追跡manifestの実測最大は526 bytes。実測を境目にせず約7.8倍の余裕を取り、
    // 巨大入力検査と混ざらない4 KiB未満をこの検査の上限にする。
    const MALFORMED_MAX_CASE_BYTES: usize = 4 * 1024;

    #[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
    #[serde(rename_all = "snake_case")]
    enum FoldRejectAt {
        Parse,
        Convert,
    }

    #[derive(Debug, Deserialize)]
    struct FoldMalformedCase {
        category: String,
        name: String,
        expected: FoldRejectAt,
        source: String,
    }

    #[derive(Debug, Deserialize)]
    struct FoldMalformedManifest {
        schema: u32,
        cases: Vec<FoldMalformedCase>,
    }

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

    fn one_pleat_square_store() -> (DocumentStore, ori3_model::FoldPoseInput) {
        let mut store = square_store();
        ori3_cp::insert_segment(&mut store.doc.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Valley);
        store.faces = ori3_cp::extract_faces(&store.doc.cp);
        let hinge = store
            .doc
            .cp
            .edges
            .iter()
            .find(|edge| edge.kind == EdgeKind::Valley)
            .expect("inserted hinge")
            .id;
        (
            store,
            ori3_model::FoldPoseInput {
                drivers: vec![ori3_model::FoldPoseDriver {
                    edge_id: hinge,
                    target_angle_deg: 180.0,
                }],
            },
        )
    }

    fn crane_head_document() -> Document {
        serde_json::from_str(CRANE_HEAD_DOCUMENT_JSON).expect("利用者の固定標本を読める")
    }

    fn crane_head_store() -> DocumentStore {
        let mut store = DocumentStore::default();
        store.doc = crane_head_document();
        store.faces = ori3_cp::extract_faces(&store.doc.cp);
        store
    }

    fn crane_head_pose_json() -> serde_json::Value {
        serde_json::json!({
            "drivers": CRANE_HEAD_POSE_DRIVERS
                .iter()
                .map(|&(edge_id, target_angle_deg)| serde_json::json!({
                    "edge_id": edge_id,
                    "target_angle_deg": target_angle_deg,
                }))
                .collect::<Vec<_>>(),
        })
    }

    /// 利用者標本と同じ `pose_before` を、画面と同じJSON境界から読み込む。
    /// serdeで姿勢指定を落とす退行も、FoldThroughへ渡さない退行も同じ検査で捕まえる。
    fn crane_head_fold_op(preview: bool) -> SeqOp {
        let mut json = serde_json::json!({
            "type": if preview { "PreviewFoldThrough" } else { "FoldThrough" },
            "up_to": 0,
            "line": CRANE_HEAD_LINE,
            "keep_side_point": CRANE_HEAD_KEEP_POINT,
            "target_layers": null,
            "direction": "Up",
            "pose_before": crane_head_pose_json(),
        });
        if !preview {
            json["alignment"] = serde_json::Value::Null;
            json["accept_additional_crease"] = serde_json::Value::Bool(false);
        }
        serde_json::from_value(json).expect("FoldThrough要求を読める")
    }

    fn crane_head_target_query() -> SeqOp {
        serde_json::from_value(serde_json::json!({
            "type": "PreviewFoldTargets",
            "up_to": 0,
            "line": CRANE_HEAD_LINE,
            "keep_side_point": CRANE_HEAD_KEEP_POINT,
            "pose_before": crane_head_pose_json(),
        }))
        .expect("query uses the same persisted pose declaration as the fold")
    }

    fn crane_head_moving_faces(
        document: &Document,
        faces: &[Face],
        state: &ori3_layers::FlatState,
    ) -> Vec<FaceId> {
        let [l0, l1] = CRANE_HEAD_LINE;
        let dx = l1[0] - l0[0];
        let dy = l1[1] - l0[1];
        let length = dx.hypot(dy);
        let (ux, uy) = (dx / length, dy / length);
        let signed_side = |point: [f64; 2]| ux * (point[1] - l0[1]) - uy * (point[0] - l0[0]);
        let keep_sign = signed_side(CRANE_HEAD_KEEP_POINT).signum();
        let vertices = document
            .cp
            .vertices
            .iter()
            .map(|vertex| (vertex.id, vertex.pos))
            .collect::<HashMap<_, _>>();

        faces
            .iter()
            .filter(|face| {
                let placement = &state.placements[&face.id];
                face.vertices.iter().any(|vertex| {
                    let folded = placement.apply(vertices[vertex].into());
                    keep_sign * signed_side([folded.x, folded.y]) < -EPS
                })
            })
            .map(|face| face.id)
            .collect()
    }

    fn crane_head_moving_frame_faces(frame: &Frame3D) -> Vec<FaceId> {
        let [l0, l1] = CRANE_HEAD_LINE;
        let dx = l1[0] - l0[0];
        let dy = l1[1] - l0[1];
        let length = dx.hypot(dy);
        let (ux, uy) = (dx / length, dy / length);
        let signed_side = |x: f64, y: f64| ux * (y - l0[1]) - uy * (x - l0[0]);
        let keep_sign = signed_side(CRANE_HEAD_KEEP_POINT[0], CRANE_HEAD_KEEP_POINT[1]).signum();
        let mut moving = frame
            .faces
            .iter()
            .filter(|face| {
                face.polygon
                    .iter()
                    .any(|point| keep_sign * signed_side(point[0], point[1]) < -EPS)
            })
            .map(|face| face.face)
            .collect::<Vec<_>>();
        moving.sort_unstable();
        moving
    }

    fn solve_crane_head_pose(
        document: &Document,
        faces: &[Face],
    ) -> ori3_layers::replay::CanonicalFlatPose {
        ori3_layers::replay::canonical_flat_pose_at(
            document,
            faces,
            0,
            &ori3_model::FoldPoseInput {
                drivers: CRANE_HEAD_POSE_DRIVERS
                    .iter()
                    .map(|&(edge_id, target_angle_deg)| ori3_model::FoldPoseDriver {
                        edge_id,
                        target_angle_deg,
                    })
                    .collect(),
            },
        )
        .expect("書類と12本の符号だけから現在の平坦姿勢を再現できる")
    }

    fn crane_head_oracle_state(document: &Document, faces: &[Face]) -> ori3_layers::FlatState {
        let vertices = document
            .cp
            .vertices
            .iter()
            .map(|vertex| (vertex.id, vertex.pos))
            .collect::<HashMap<_, _>>();
        let edges = document
            .cp
            .edges
            .iter()
            .map(|edge| (edge.id, edge))
            .collect::<HashMap<_, _>>();
        let mut observed = ori3_layers::FlatState::initial(&document.cp, faces);
        observed.order = CRANE_HEAD_CAPTURED_LAYER_ORDER.to_vec();
        let layer_order = observed.to_layer_points(&document.cp, faces);
        let drivers = CRANE_HEAD_ORACLE_HINGE_ANGLES
            .iter()
            .map(|&(edge_id, target_angle_deg)| {
                let edge = edges[&edge_id];
                DriverLine {
                    a: vertices[&edge.v0],
                    b: vertices[&edge.v1],
                    target_angle_deg,
                }
            })
            .collect();
        let mut oracle = document.clone();
        oracle.sequence.push(FoldStep {
            id: 0,
            kind: TechniqueKind::Simple,
            drivers,
            layer_order: Some(layer_order),
            alignment: None,
            finish_soft: None,
            note: "固定標本の期待平坦姿勢".to_string(),
        });
        let (state, warnings) =
            ori3_layers::flat_state_at(&oracle, faces, 1).expect("期待姿勢を再生できる");
        assert!(warnings.is_empty(), "期待姿勢の警告={warnings:?}");
        state
    }

    type CraneHeadPoint2 = [f64; 2];

    fn crane_head_point_subtract(left: CraneHeadPoint2, right: CraneHeadPoint2) -> CraneHeadPoint2 {
        [left[0] - right[0], left[1] - right[1]]
    }

    fn crane_head_point_add(left: CraneHeadPoint2, right: CraneHeadPoint2) -> CraneHeadPoint2 {
        [left[0] + right[0], left[1] + right[1]]
    }

    fn crane_head_point_scale(point: CraneHeadPoint2, scale: f64) -> CraneHeadPoint2 {
        [point[0] * scale, point[1] * scale]
    }

    fn crane_head_perp_dot(left: CraneHeadPoint2, right: CraneHeadPoint2) -> f64 {
        left[0] * right[1] - left[1] * right[0]
    }

    fn crane_head_point_distance(left: CraneHeadPoint2, right: CraneHeadPoint2) -> f64 {
        (left[0] - right[0]).hypot(left[1] - right[1])
    }

    fn crane_head_polygon_area(polygon: &[CraneHeadPoint2]) -> f64 {
        if polygon.len() < 3 {
            return 0.0;
        }
        0.5 * (0..polygon.len())
            .map(|index| crane_head_perp_dot(polygon[index], polygon[(index + 1) % polygon.len()]))
            .sum::<f64>()
    }

    fn crane_head_simple_polygon(boundary: &[CraneHeadPoint2]) -> Vec<CraneHeadPoint2> {
        let mut polygon = Vec::with_capacity(boundary.len());
        for &point in boundary {
            if polygon
                .last()
                .is_none_or(|previous| crane_head_point_distance(*previous, point) > EPS)
            {
                polygon.push(point);
            }
        }
        while polygon.len() > 1
            && crane_head_point_distance(polygon[0], polygon[polygon.len() - 1]) <= EPS
        {
            polygon.pop();
        }
        polygon
    }

    fn crane_head_point_in_triangle(
        point: CraneHeadPoint2,
        a: CraneHeadPoint2,
        b: CraneHeadPoint2,
        c: CraneHeadPoint2,
    ) -> bool {
        crane_head_perp_dot(
            crane_head_point_subtract(b, a),
            crane_head_point_subtract(point, a),
        ) >= -EPS
            && crane_head_perp_dot(
                crane_head_point_subtract(c, b),
                crane_head_point_subtract(point, b),
            ) >= -EPS
            && crane_head_perp_dot(
                crane_head_point_subtract(a, c),
                crane_head_point_subtract(point, c),
            ) >= -EPS
    }

    fn crane_head_triangulate_polygon(
        boundary: &[CraneHeadPoint2],
    ) -> Result<Vec<Vec<CraneHeadPoint2>>, String> {
        let mut polygon = crane_head_simple_polygon(boundary);
        if polygon.len() < 3
            || crane_head_polygon_area(&polygon).abs() <= CRANE_HEAD_OVERLAP_AREA_EPS
        {
            return Err("crane-head投影多角形が退化しています".to_string());
        }
        if crane_head_polygon_area(&polygon) < 0.0 {
            polygon.reverse();
        }
        let mut triangles = Vec::with_capacity(polygon.len().saturating_sub(2));
        while polygon.len() > 3 {
            let count = polygon.len();
            let Some(ear) = (0..count).find(|&index| {
                let a = polygon[(index + count - 1) % count];
                let b = polygon[index];
                let c = polygon[(index + 1) % count];
                crane_head_perp_dot(
                    crane_head_point_subtract(b, a),
                    crane_head_point_subtract(c, b),
                ) > EPS * EPS
                    && !polygon.iter().enumerate().any(|(other, &point)| {
                        other != index
                            && other != (index + count - 1) % count
                            && other != (index + 1) % count
                            && crane_head_point_in_triangle(point, a, b, c)
                    })
            }) else {
                return Err("crane-head投影多角形を三角形分割できません".to_string());
            };
            triangles.push(vec![
                polygon[(ear + count - 1) % count],
                polygon[ear],
                polygon[(ear + 1) % count],
            ]);
            polygon.remove(ear);
        }
        triangles.push(polygon);
        Ok(triangles)
    }

    fn crane_head_deduplicate_polygon(points: Vec<CraneHeadPoint2>) -> Vec<CraneHeadPoint2> {
        let mut output = Vec::with_capacity(points.len());
        for point in points {
            if output
                .last()
                .is_none_or(|previous| crane_head_point_distance(*previous, point) > EPS)
            {
                output.push(point);
            }
        }
        if output.len() > 1 && crane_head_point_distance(output[0], output[output.len() - 1]) <= EPS
        {
            output.pop();
        }
        output
    }

    fn crane_head_intersect_convex_polygons(
        subject: &[CraneHeadPoint2],
        clip: &[CraneHeadPoint2],
    ) -> Vec<CraneHeadPoint2> {
        let mut output = subject.to_vec();
        for index in 0..clip.len() {
            let clip_start = clip[index];
            let clip_end = clip[(index + 1) % clip.len()];
            let clip_direction = crane_head_point_subtract(clip_end, clip_start);
            let input = std::mem::take(&mut output);
            let Some(mut previous) = input.last().copied() else {
                break;
            };
            let mut previous_side = crane_head_perp_dot(
                clip_direction,
                crane_head_point_subtract(previous, clip_start),
            );
            for current in input {
                let current_side = crane_head_perp_dot(
                    clip_direction,
                    crane_head_point_subtract(current, clip_start),
                );
                let previous_inside = previous_side >= -EPS;
                let current_inside = current_side >= -EPS;
                if previous_inside != current_inside {
                    let denominator = previous_side - current_side;
                    if denominator.abs() > EPS * EPS {
                        output.push(crane_head_point_add(
                            previous,
                            crane_head_point_scale(
                                crane_head_point_subtract(current, previous),
                                previous_side / denominator,
                            ),
                        ));
                    }
                }
                if current_inside {
                    output.push(current);
                }
                previous = current;
                previous_side = current_side;
            }
        }
        crane_head_deduplicate_polygon(output)
    }

    /// `pose_motion::overlap_witnesses` と同じear clipping・凸clip・面積閾値。
    /// desktopは同helperのcrate外なので、製品可視性を広げずtest内で同じ計算を行う。
    fn crane_head_overlap_witnesses(
        left: &[CraneHeadPoint2],
        right: &[CraneHeadPoint2],
    ) -> Result<Vec<CraneHeadPoint2>, String> {
        let left_triangles = crane_head_triangulate_polygon(left)?;
        let right_triangles = crane_head_triangulate_polygon(right)?;
        let mut witnesses = Vec::new();
        for left_triangle in &left_triangles {
            for right_triangle in &right_triangles {
                let intersection =
                    crane_head_intersect_convex_polygons(left_triangle, right_triangle);
                if crane_head_polygon_area(&intersection).abs() <= CRANE_HEAD_OVERLAP_AREA_EPS {
                    continue;
                }
                let sum = intersection
                    .iter()
                    .copied()
                    .fold([0.0, 0.0], crane_head_point_add);
                let center = crane_head_point_scale(sum, 1.0 / intersection.len() as f64);
                witnesses.push(center);
                witnesses.extend(
                    intersection.iter().copied().map(|point| {
                        crane_head_point_scale(crane_head_point_add(point, center), 0.5)
                    }),
                );
            }
        }
        Ok(witnesses)
    }

    fn crane_head_overlap_order_mismatches(
        document: &Document,
        faces: &[Face],
        state: &ori3_layers::FlatState,
        oracle_order: &[FaceId],
    ) -> Result<(usize, Vec<(FaceId, FaceId)>), String> {
        let vertices = document
            .cp
            .vertices
            .iter()
            .map(|vertex| (vertex.id, vertex.pos))
            .collect::<HashMap<_, _>>();
        let polygons = faces
            .iter()
            .map(|face| {
                let placement = state
                    .placements
                    .get(&face.id)
                    .ok_or_else(|| format!("crane-head面{}の平坦配置がありません", face.id))?;
                let polygon = face
                    .vertices
                    .iter()
                    .map(|vertex| {
                        let material = vertices.get(vertex).copied().ok_or_else(|| {
                            format!("crane-head面{}の頂点{vertex}がありません", face.id)
                        })?;
                        let point = placement.apply(material.into());
                        let point = [point.x, point.y];
                        point
                            .iter()
                            .all(|coordinate| coordinate.is_finite())
                            .then_some(point)
                            .ok_or_else(|| {
                                format!("crane-head面{}の座標が有限ではありません", face.id)
                            })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok((face.id, polygon))
            })
            .collect::<Result<HashMap<_, _>, String>>()?;
        let pose_rank = state
            .order
            .iter()
            .copied()
            .enumerate()
            .map(|(rank, face)| (face, rank))
            .collect::<HashMap<_, _>>();
        let oracle_rank = oracle_order
            .iter()
            .copied()
            .enumerate()
            .map(|(rank, face)| (face, rank))
            .collect::<HashMap<_, _>>();
        let mut compared_pairs = 0;
        let mut mismatches = Vec::new();
        for left_index in 0..faces.len() {
            for right_index in left_index + 1..faces.len() {
                let left = faces[left_index].id;
                let right = faces[right_index].id;
                if crane_head_overlap_witnesses(&polygons[&left], &polygons[&right])?.is_empty() {
                    continue;
                }
                compared_pairs += 1;
                let pose_left_is_below = pose_rank
                    .get(&left)
                    .zip(pose_rank.get(&right))
                    .is_some_and(|(left, right)| left < right);
                let oracle_left_is_below = oracle_rank
                    .get(&left)
                    .zip(oracle_rank.get(&right))
                    .is_some_and(|(left, right)| left < right);
                if pose_left_is_below != oracle_left_is_below {
                    mismatches.push((left, right));
                }
            }
        }
        Ok((compared_pairs, mismatches))
    }

    fn seeded_fold_import_store() -> DocumentStore {
        let mut store = square_store();
        let mut recorded_step = step(40);
        recorded_step.kind = TechniqueKind::Pose;
        store
            .apply_seq(SeqOp::PushStep {
                step: recorded_step,
            })
            .expect("seedの手順を通常操作で追加できる");
        store
            .apply_edit(diagonal())
            .expect("seedの展開図を通常操作で変更できる");
        store.undo().expect("redo内容を通常操作で作れる");
        store.dirty = false;
        store.path = Some(PathBuf::from("seed-before-fold.ori3"));
        assert!(!store.doc.sequence.is_empty());
        assert!(!store.step_creases.is_empty());
        assert!(!store.undo_stack.is_empty());
        assert!(!store.redo_stack.is_empty());
        store
    }

    fn malformed_fold_cases() -> Vec<FoldMalformedCase> {
        let manifest: FoldMalformedManifest =
            serde_json::from_str(FOLD_MALFORMED_100).expect("追跡済み100件manifestを読める");
        assert_eq!(manifest.schema, 1, "malformed manifest schema");
        manifest.cases
    }

    #[test]
    fn fold_import_is_one_undoable_dirty_unsaved_operation() {
        let file = parse_fold_1_2(FOLD_IMPORT_SUCCESS).expect("成功fixtureをparseできる");
        let expected = fold_to_document(&file).expect("成功fixtureを変換できる");
        assert!(expected.warnings.is_empty());

        let mut store = seeded_fold_import_store();
        let before_document = store.doc.clone();
        let before_step_creases = store.step_creases.clone();
        let before_undo_len = store.undo_stack.len();
        reset_commit_count_for_test();

        let view = store.import_fold(expected.clone());
        assert_eq!(view.doc, expected.document);
        assert!(
            view.step_creases.is_empty(),
            "FOLDは追加折り線の来歴を持たない"
        );
        assert!(store.step_creases.is_empty());
        assert!(view.fold_issues.is_empty());
        assert_eq!(commit_count_for_test(), 1, "FOLD読込のcommitはexactに1回");
        assert_eq!(store.undo_stack.len(), before_undo_len + 1);
        assert!(store.redo_stack.is_empty(), "新しい読込でredoを消す");
        assert!(store.is_dirty(), "FOLD読込後は未保存");
        assert_eq!(store.current_path(), None, "読込元.foldを保存先にしない");

        let imported_document = store.doc.clone();
        let imported_step_creases = store.step_creases.clone();
        store.undo().expect("既存undoでFOLD読込を1回で戻せる");
        assert_eq!(store.doc, before_document);
        assert_eq!(store.step_creases, before_step_creases);
        assert_eq!(store.current_path(), None, "既存undoは保存先を履歴化しない");
        store.redo().expect("既存redoでFOLD読込をやり直せる");
        assert_eq!(store.doc, imported_document);
        assert_eq!(store.step_creases, imported_step_creases);
    }

    #[test]
    fn fold_import_with_warnings_commits_without_a_confirmation_gate() {
        let file = parse_fold_1_2(FOLD_IMPORT_WARNINGS_20).expect("警告fixtureをparseできる");
        let expected = fold_to_document(&file).expect("警告だけなら変換できる");
        assert_eq!(expected.warnings.len(), 20);
        assert!(expected.warnings.iter().all(|issue| !issue.path.is_empty()));
        assert!(
            expected
                .warnings
                .iter()
                .all(|issue| issue.original_value.is_some())
        );

        let mut store = seeded_fold_import_store();
        let before = store.atomicity_probe_for_test();
        reset_commit_count_for_test();
        let view = store.import_fold(expected.clone());

        assert_ne!(
            store.atomicity_probe_for_test(),
            before,
            "警告をgateにしない"
        );
        assert_eq!(view.doc, expected.document);
        assert_eq!(view.fold_issues, expected.warnings);
        assert_eq!(view.fold_issues.len(), 20);
        assert_eq!(commit_count_for_test(), 1, "警告付き読込もcommitは1回");
        assert!(store.is_dirty());
        assert_eq!(store.current_path(), None);
    }

    #[test]
    fn fold_conversion_failure_keeps_the_complete_store_and_all_issues() {
        let file = parse_fold_1_2(FOLD_IMPORT_WARNING_AND_ERROR).expect("変換段階まで進むfixture");
        let expected = fold_to_document(&file).expect_err("警告とerrorが同居するfixture");
        assert!(!expected.warnings.is_empty());
        assert!(!expected.errors.is_empty());

        let store = seeded_fold_import_store();
        let before = store.atomicity_probe_for_test();
        reset_commit_count_for_test();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fold_to_document(&file).map_err(FoldImportError::Conversion)
        }));
        let error = caught
            .expect("変換拒否でpanicしない")
            .expect_err("意味を変えるFOLDは読み込まない");

        assert_eq!(store.atomicity_probe_for_test(), before);
        assert_eq!(commit_count_for_test(), 0, "拒否時commitは0回");
        assert_eq!(error.warnings(), expected.warnings);
        assert_eq!(error.errors(), expected.errors);
    }

    #[test]
    fn fold_import_errors_use_only_user_facing_text() {
        let parse_error = parse_fold_1_2("{").expect_err("壊れたJSONは拒否する");
        assert_eq!(
            FoldImportError::Parse(parse_error).to_string(),
            "ほかの折り紙ソフトのファイルを読み取れませんでした。ファイルの内容を確認してください。"
        );

        let file = parse_fold_1_2(FOLD_IMPORT_WARNING_AND_ERROR).expect("変換段階まで進むfixture");
        let conversion_error = fold_to_document(&file).expect_err("対応範囲外は拒否する");
        assert_eq!(
            FoldImportError::Conversion(conversion_error).to_string(),
            "このファイルには、ORIGAMI3で扱えない内容があります。"
        );
    }

    #[test]
    fn fold_import_rejects_malformed_100_without_panic_or_store_change() {
        let cases = malformed_fold_cases();
        assert_eq!(cases.len(), MALFORMED_TOTAL);

        let mut category_counts = BTreeMap::<String, usize>::new();
        let mut expected_stage_counts = BTreeMap::<FoldRejectAt, usize>::new();
        let mut names = BTreeSet::new();
        let mut identities = BTreeSet::new();
        let mut sources = BTreeSet::new();
        for case in &cases {
            *category_counts.entry(case.category.clone()).or_default() += 1;
            *expected_stage_counts.entry(case.expected).or_default() += 1;
            assert!(names.insert(case.name.clone()), "name重複: {}", case.name);
            assert!(identities.insert((case.category.clone(), case.name.clone())));
            assert!(
                sources.insert(case.source.clone()),
                "source重複: {}",
                case.name
            );
            assert!(
                case.source.len() < MALFORMED_MAX_CASE_BYTES,
                "巨大caseは禁止: {} bytes ({})",
                case.source.len(),
                case.name
            );
        }
        assert_eq!(category_counts.len(), MALFORMED_CATEGORY_COUNT);
        assert_eq!(
            category_counts
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                "01_invalid_json",
                "02_root_and_version",
                "03_known_field_types",
                "04_array_element_types",
                "05_required_and_cardinality",
                "06_topology",
                "07_unsupported_profile",
                "08_frame_sequence",
                "09_assignment_and_angle",
                "10_face_orders",
            ]
        );
        assert!(
            category_counts
                .values()
                .all(|&count| count == MALFORMED_CASES_PER_CATEGORY)
        );
        assert_eq!(expected_stage_counts.get(&FoldRejectAt::Parse), Some(&40));
        assert_eq!(expected_stage_counts.get(&FoldRejectAt::Convert), Some(&60));

        let mut actual_stage_counts = BTreeMap::<FoldRejectAt, usize>::new();
        let mut panic_count = 0_usize;
        for case in &cases {
            let actual_stage = match parse_fold_1_2(&case.source) {
                Err(_) => FoldRejectAt::Parse,
                Ok(file) => {
                    let typed_before = file.clone();
                    let converted = fold_to_document(&file);
                    assert_eq!(file, typed_before, "typed入力を変更した: {}", case.name);
                    assert!(converted.is_err(), "不正入力が変換成功: {}", case.name);
                    FoldRejectAt::Convert
                }
            };
            assert_eq!(actual_stage, case.expected, "拒否段階: {}", case.name);
            *actual_stage_counts.entry(actual_stage).or_default() += 1;

            let mut store = seeded_fold_import_store();
            let before = store.atomicity_probe_for_test();
            reset_commit_count_for_test();
            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                parse_fold_1_2(&case.source)
                    .map_err(FoldImportError::Parse)
                    .and_then(|file| fold_to_document(&file).map_err(FoldImportError::Conversion))
                    .map(|import| store.import_fold(import))
            }));
            let result = match caught {
                Ok(result) => result,
                Err(_) => {
                    panic_count += 1;
                    assert_eq!(
                        store.atomicity_probe_for_test(),
                        before,
                        "panic時にもstoreを部分更新しない: {}",
                        case.name
                    );
                    continue;
                }
            };
            assert!(result.is_err(), "不正入力が成功: {}", case.name);
            assert_eq!(
                store.atomicity_probe_for_test(),
                before,
                "拒否時store完全不変: {}",
                case.name
            );
            assert_eq!(commit_count_for_test(), 0, "拒否時commit: {}", case.name);
        }

        assert_eq!(actual_stage_counts.get(&FoldRejectAt::Parse), Some(&40));
        assert_eq!(actual_stage_counts.get(&FoldRejectAt::Convert), Some(&60));
        assert_eq!(panic_count, 0, "malformed 100件のpanicは0");
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
            alignment: None,
            finish_soft: None,
            note: String::new(),
        }
    }

    fn frame_with_z(z: f64) -> Frame3D {
        Frame3D {
            faces: vec![Face3D {
                face: 0,
                polygon: vec![[0.0, 0.0, z]],
                layer: 0,
                surface_rank: 0,
                mirrored: false,
            }],
            warnings: Vec::new(),
        }
    }

    fn material_vertex_world_position(
        faces: &[Face],
        frame: &Frame3D,
        vertex: VertexId,
    ) -> Option<[f64; 3]> {
        frame.faces.iter().find_map(|spatial_face| {
            let material_face = faces.iter().find(|face| face.id == spatial_face.face)?;
            let index = material_face
                .vertices
                .iter()
                .position(|candidate| *candidate == vertex)?;
            spatial_face.polygon.get(index).copied()
        })
    }

    #[test]
    fn spatial_nonflat_threshold_matches_viewer_contract() {
        assert!(!frame_is_nonflat(&frame_with_z(0.0)));
        assert!(!frame_is_nonflat(&frame_with_z(NONFLAT_EPS)));
        assert!(!frame_is_nonflat(&frame_with_z(-NONFLAT_EPS)));
        assert!(frame_is_nonflat(&frame_with_z(NONFLAT_EPS + 1e-12)));
        assert!(frame_is_nonflat(&frame_with_z(-NONFLAT_EPS - 1e-12)));
    }

    /// 利用者の画面から取り出した展開図(2026-08-13)。元の受け入れテストは
    /// `crates/ori3-rigid/` にあるため変更せず、通知側の恒久検査用に同じ入力を置く。
    fn flat_fold_notice_user_cp() -> CreasePattern {
        CreasePattern {
            vertices: vec![
                Vertex {
                    id: 0,
                    pos: [0.0, 0.0],
                },
                Vertex {
                    id: 1,
                    pos: [1.0, 0.0],
                },
                Vertex {
                    id: 2,
                    pos: [1.0, 1.0],
                },
                Vertex {
                    id: 3,
                    pos: [0.0, 1.0],
                },
                Vertex {
                    id: 4,
                    pos: [0.0, 0.5],
                },
                Vertex {
                    id: 5,
                    pos: [1.0, 0.5],
                },
                Vertex {
                    id: 6,
                    pos: [0.5, 1.0],
                },
                Vertex {
                    id: 7,
                    pos: [0.5, 0.0],
                },
                Vertex {
                    id: 8,
                    pos: [0.5, 0.5],
                },
                Vertex {
                    id: 9,
                    pos: [0.7928932188134525, 0.5],
                },
                Vertex {
                    id: 10,
                    pos: [0.5, 0.20710678118654752],
                },
                Vertex {
                    id: 11,
                    pos: [0.25, 0.5],
                },
                Vertex {
                    id: 12,
                    pos: [0.5, 0.7928932188134525],
                },
            ],
            edges: vec![
                Edge {
                    id: 4,
                    v0: 3,
                    v1: 4,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 5,
                    v0: 4,
                    v1: 0,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 6,
                    v0: 1,
                    v1: 5,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 7,
                    v0: 5,
                    v1: 2,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 9,
                    v0: 2,
                    v1: 6,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 10,
                    v0: 6,
                    v1: 3,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 11,
                    v0: 0,
                    v1: 7,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 12,
                    v0: 7,
                    v1: 1,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 17,
                    v0: 0,
                    v1: 8,
                    kind: EdgeKind::Valley,
                },
                Edge {
                    id: 18,
                    v0: 8,
                    v1: 2,
                    kind: EdgeKind::Valley,
                },
                Edge {
                    id: 19,
                    v0: 8,
                    v1: 9,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 20,
                    v0: 9,
                    v1: 5,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 21,
                    v0: 2,
                    v1: 9,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 22,
                    v0: 9,
                    v1: 1,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 23,
                    v0: 8,
                    v1: 10,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 24,
                    v0: 10,
                    v1: 7,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 25,
                    v0: 0,
                    v1: 10,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 26,
                    v0: 10,
                    v1: 1,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 27,
                    v0: 4,
                    v1: 11,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 28,
                    v0: 11,
                    v1: 8,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 29,
                    v0: 0,
                    v1: 11,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 30,
                    v0: 11,
                    v1: 3,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 31,
                    v0: 6,
                    v1: 12,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 32,
                    v0: 12,
                    v1: 8,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 33,
                    v0: 2,
                    v1: 12,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 34,
                    v0: 12,
                    v1: 3,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 35,
                    v0: 11,
                    v1: 12,
                    kind: EdgeKind::Valley,
                },
                Edge {
                    id: 36,
                    v0: 10,
                    v1: 9,
                    kind: EdgeKind::Valley,
                },
            ],
            next_vertex_id: 13,
            next_edge_id: 37,
        }
    }

    fn user_flat_targets() -> Vec<Driver> {
        [21, 22, 25, 26, 29, 30, 33, 34]
            .into_iter()
            .map(|hinge| Driver {
                hinge,
                target_angle_deg: 180.0,
            })
            .collect()
    }

    fn reached_angles(targets: &[Driver]) -> HashMap<EdgeId, f64> {
        targets
            .iter()
            .map(|target| (target.hinge, target.target_angle_deg))
            .collect()
    }

    fn all_crease_flat_targets(cp: &CreasePattern) -> Vec<Driver> {
        cp.edges
            .iter()
            .filter_map(|edge| {
                let target_angle_deg = match edge.kind {
                    EdgeKind::Mountain => 180.0,
                    EdgeKind::Valley => -180.0,
                    EdgeKind::Border | EdgeKind::Aux => return None,
                };
                Some(Driver {
                    hinge: edge.id,
                    target_angle_deg,
                })
            })
            .collect()
    }

    /// 保存手順の明示角を、姿勢計算を行わず現在の辺IDへ解決する。
    fn sequence_targets(doc: &Document) -> Vec<Driver> {
        let mut targets = std::collections::BTreeMap::new();
        for step in &doc.sequence {
            for line in &step.drivers {
                for hinge in ori3_layers::resolve_driver_edges(&doc.cp, line) {
                    targets.insert(hinge, line.target_angle_deg);
                }
            }
        }
        targets
            .into_iter()
            .map(|(hinge, target_angle_deg)| Driver {
                hinge,
                target_angle_deg,
            })
            .collect()
    }

    #[derive(serde::Deserialize)]
    struct FrontCpFixture {
        vertices: Vec<Vertex>,
        edges: Vec<Edge>,
    }

    /// フロントで実際に表示しているtracked fixtureからCPを復元する。
    fn front_fixture_cp(text: &str) -> CreasePattern {
        let fixture: FrontCpFixture = serde_json::from_str(text).expect("展開図fixtureを読む");
        let next_vertex_id = fixture
            .vertices
            .iter()
            .map(|vertex| vertex.id)
            .max()
            .map_or(0, |id| id.saturating_add(1));
        let next_edge_id = fixture
            .edges
            .iter()
            .map(|edge| edge.id)
            .max()
            .map_or(0, |id| id.saturating_add(1));
        CreasePattern {
            vertices: fixture.vertices,
            edges: fixture.edges,
            next_vertex_id,
            next_edge_id,
        }
    }

    /// 既存のやっこさん受け入れテストと同じ線を、通常の作図APIで作る。
    fn yakko_cp() -> CreasePattern {
        let mut cp = Document::new(Paper {
            width_mm: 150.0,
            height_mm: 150.0,
        })
        .cp;
        let (m1, m2, m3, m4) = ([0.5, 0.0], [1.0, 0.5], [0.5, 1.0], [0.0, 0.5]);
        for (a, b, kind) in [
            (m1, m2, EdgeKind::Valley),
            (m2, m3, EdgeKind::Valley),
            (m3, m4, EdgeKind::Valley),
            (m4, m1, EdgeKind::Valley),
        ] {
            ori3_cp::insert_segment(&mut cp, a, b, kind);
        }
        for t in [0.25, 0.75] {
            for (a, b, kind) in [
                ([t, 0.0], [t, 0.25], EdgeKind::Mountain),
                ([t, 0.25], [t, 0.75], EdgeKind::Valley),
                ([t, 0.75], [t, 1.0], EdgeKind::Mountain),
                ([0.0, t], [0.25, t], EdgeKind::Mountain),
                ([0.25, t], [0.75, t], EdgeKind::Valley),
                ([0.75, t], [1.0, t], EdgeKind::Mountain),
            ] {
                ori3_cp::insert_segment(&mut cp, a, b, kind);
            }
        }
        cp
    }

    fn flat_fold_rule_counts(cp: &CreasePattern, targets: &[Driver]) -> (usize, usize, usize) {
        let raw = ori3_cp::local_violations(cp).len();
        let filtered = flat_fold_notice_violations(cp, targets, &HashMap::new(), false).len();
        let notice =
            flat_fold_notice_violations(cp, targets, &reached_angles(targets), false).len();
        (raw, filtered, notice)
    }

    #[test]
    fn flat_fold_notice_reports_only_missed_signed_targets_without_mutating_cp() {
        let cp = flat_fold_notice_user_cp();
        let original = cp.clone();
        let targets = user_flat_targets();
        let missed: HashMap<EdgeId, f64> =
            targets.iter().map(|target| (target.hinge, 90.0)).collect();

        assert_eq!(
            flat_fold_notice_violations(&cp, &targets, &missed, false),
            vec![9, 10, 11, 12],
            "利用者の図では指定角未到達の4点を知らせる"
        );
        assert_eq!(
            flat_fold_notice_violations(&cp, &targets, &reached_angles(&targets), false),
            Vec::<VertexId>::new(),
            "全て指定角へ届き、紙も食い込んでいなければ知らせない"
        );
        let opposite: HashMap<EdgeId, f64> = targets
            .iter()
            .map(|target| (target.hinge, -target.target_angle_deg))
            .collect();
        assert_eq!(
            flat_fold_notice_violations(&cp, &targets, &opposite, false),
            vec![9, 10, 11, 12],
            "+180°と-180°を指定角の到達判定では同一視しない"
        );

        // pose_solveはpreferredを先、hardを後に並べる。同じ辺をhardで90°へ
        // 動かした最新要求は、古いpreferredの180°を残してはならない。
        let mut overridden = targets.clone();
        overridden.extend(targets.iter().map(|target| Driver {
            hinge: target.hinge,
            target_angle_deg: 90.0,
        }));
        assert!(
            flat_fold_notice_violations(&cp, &overridden, &missed, true).is_empty(),
            "後置したhardがpreferredを上書きする"
        );
        assert_eq!(cp, original, "通知検査は元の展開図を変更しない");
    }

    #[test]
    fn replay_flat_fold_notice_reuses_existing_intersections() {
        let cp = flat_fold_notice_user_cp();
        let targets = user_flat_targets();
        let angles = reached_angles(&targets);

        assert!(
            replay_flat_fold_notice_violations(&cp, &targets, &angles, &[]).is_empty(),
            "最終交差がなければ、指定角到達だけでは再生通知を出さない"
        );
        assert_eq!(
            replay_flat_fold_notice_violations(&cp, &targets, &angles, &[(0, 1)]),
            vec![9, 10, 11, 12],
            "再生側で既に得た交差組があれば4点を知らせる"
        );
    }

    #[test]
    fn replay_contact_diagnostic_reuses_existing_intersections_and_serializes() {
        let mut store = square_store();
        let mut view = store.apply_edit(diagonal()).unwrap();

        attach_replay_contact_diagnostic(&mut view, &[(3, 7)]);
        assert!(view.contact_detected, "明示した交差組を診断値へ運ぶ");
        let detected = serde_json::to_value(&view).expect("DocumentViewをJSONへ運べる");
        assert_eq!(detected["contact_detected"], true);

        attach_replay_contact_diagnostic(&mut view, &[]);
        assert!(!view.contact_detected, "交差0組なら診断値を戻す");
        let clear = serde_json::to_value(&view).expect("DocumentViewをJSONへ運べる");
        assert_eq!(clear["contact_detected"], false);
    }

    #[test]
    fn pose_flat_fold_notice_filters_only_transient_full_sweep_contacts() {
        let cp = flat_fold_notice_user_cp();
        let partial_targets = user_flat_targets();
        assert!(pose_flat_fold_notice_intersects(
            &cp,
            &partial_targets,
            true,
            false
        ));
        assert!(!pose_flat_fold_notice_intersects(
            &cp,
            &partial_targets,
            false,
            false
        ));

        let all_targets = all_crease_flat_targets(&cp);
        assert_eq!(all_targets.len(), 20);
        assert!(
            !pose_flat_fold_notice_intersects(&cp, &all_targets, true, false),
            "全折り目を完成角へ補間した途中だけの接触は通知へ上げない"
        );
        assert!(
            pose_flat_fold_notice_intersects(&cp, &all_targets, false, true),
            "補正後の最終交差は全折り目一括でも必ず通知へ上げる"
        );

        let mut aux_only = Document::new(Paper {
            width_mm: 150.0,
            height_mm: 150.0,
        })
        .cp;
        ori3_cp::insert_segment(&mut aux_only, [0.0, 0.0], [1.0, 1.0], EdgeKind::Aux);
        let aux_target = aux_only
            .edges
            .iter()
            .find(|edge| edge.kind == EdgeKind::Aux)
            .map(|edge| Driver {
                hinge: edge.id,
                target_angle_deg: 180.0,
            })
            .expect("補助線を追加する");
        assert!(
            pose_flat_fold_notice_intersects(&aux_only, &[aux_target], true, false),
            "山谷の折り目が0本でも空集合を全折り目一括とはみなさない"
        );
    }

    #[test]
    fn flat_fold_notice_reports_four_reached_points_when_paper_intersects() {
        let cp = flat_fold_notice_user_cp();
        let targets = user_flat_targets();
        let angles = reached_angles(&targets);
        assert_eq!(targets.len(), 8, "利用者がまとめて動かした折り目は8本");
        assert_eq!(
            flat_fold_notice_violations(&cp, &targets, &angles, true),
            vec![9, 10, 11, 12],
            "利用者の展開図では、8本が指定角へ届いていても食い込みがあれば4点を知らせる"
        );
    }

    #[test]
    fn flat_fold_motion_reaches_requested_angle_without_stopping() {
        let cp = flat_fold_notice_user_cp();
        let faces = ori3_cp::extract_faces(&cp);
        let targets = user_flat_targets();
        let mut warm: HashMap<EdgeId, f64> = cp
            .edges
            .iter()
            .filter(|edge| edge.kind != EdgeKind::Border)
            .map(|edge| (edge.id, 0.0))
            .collect();
        let mut final_result = None;
        for step in 1..=36u32 {
            let angle = 5.0 * f64::from(step);
            let hard = vec![Driver {
                hinge: targets[0].hinge,
                target_angle_deg: angle,
            }];
            let preferred: HashMap<EdgeId, f64> = targets[1..]
                .iter()
                .map(|target| (target.hinge, angle))
                .collect();
            let solved =
                ori3_rigid::solve_motion(&cp, &faces, &hard, Some(&preferred), Some(&warm), true)
                    .result;
            warm = solved.angles.clone();
            final_result = Some(solved);
        }
        let solved = final_result.expect("180°までの操作結果を返す");

        assert!(!solved.frame.faces.is_empty(), "平坦折り操作でも立体を返す");
        assert!(
            solved.angles.values().all(|angle| angle.is_finite()),
            "平坦折り操作でも有限の角度を返す"
        );
        assert!(
            (solved.angles[&targets[0].hinge] - targets[0].target_angle_deg).abs()
                <= FLAT_TARGET_EPS_DEG,
            "操作中の折り目は指定した180°まで動く"
        );
    }

    #[test]
    fn crane_closure_rescue_is_independent_of_contact_detection() {
        let cp = front_fixture_cp(include_str!("../../src/lib/__fixtures__/crane.json"));
        let faces = ori3_cp::extract_faces(&cp);
        let creases = cp
            .edges
            .iter()
            .filter(|edge| edge.kind != EdgeKind::Border)
            .map(|edge| edge.id)
            .collect::<Vec<_>>();
        let (driven, wanted) = (creases[17], creases[20]);
        let initial = creases
            .iter()
            .map(|&hinge| (hinge, 0.0))
            .collect::<HashMap<_, _>>();
        let mut detected_warm = initial.clone();
        let mut silent_warm = initial.clone();
        let mut direct_warm = initial;
        let mut final_detected = None;
        let mut final_silent = None;
        let mut final_direct = None;

        for step in 1..=30_u32 {
            let angle = -5.0 * f64::from(step);
            let targets = HashMap::from([(wanted, angle)]);
            let driver = [Driver {
                hinge: driven,
                target_angle_deg: angle,
            }];
            let detected = ori3_rigid::solve_motion(
                &cp,
                &faces,
                &driver,
                Some(&targets),
                Some(&detected_warm),
                true,
            );
            let silent = ori3_rigid::solve_motion(
                &cp,
                &faces,
                &driver,
                Some(&targets),
                Some(&silent_warm),
                false,
            );
            let direct = ori3_rigid::solve_motion_once(
                &cp,
                &faces,
                &driver,
                Some(&targets),
                Some(&direct_warm),
            );

            assert!(!detected.contact_stopped, "{angle}°");
            assert!(!silent.contact_stopped, "{angle}°");
            assert_eq!(detected.result.angles, silent.result.angles, "{angle}°");
            for (detected_face, silent_face) in detected
                .result
                .frame
                .faces
                .iter()
                .zip(&silent.result.frame.faces)
            {
                assert_eq!(detected_face.face, silent_face.face, "{angle}°");
                assert_eq!(detected_face.polygon, silent_face.polygon, "{angle}°");
            }
            assert!(detected.result.closure_rms.is_finite(), "{angle}°");
            assert!(
                detected
                    .result
                    .angles
                    .values()
                    .all(|value| value.is_finite())
            );
            assert!(detected.result.frame.faces.iter().all(|face| {
                face.polygon
                    .iter()
                    .flatten()
                    .all(|coordinate| coordinate.is_finite())
            }));
            detected_warm = detected.result.angles.clone();
            silent_warm = silent.result.angles.clone();
            direct_warm = direct.angles.clone();
            final_detected = Some(detected);
            final_silent = Some(silent);
            final_direct = Some(direct);
        }

        let detected = final_detected.expect("-150°の検出あり結果を返す");
        let silent = final_silent.expect("-150°の検出なし結果を返す");
        assert!(detected.result.converged);
        assert!(detected.result.closure_rms < 1e-9);
        assert!((detected.result.angles[&driven] + 150.0).abs() < 1e-9);
        assert!(!detected.contact_detected);
        assert!(!silent.contact_detected);
        assert!(ori3_rigid::contact_witnesses(&detected.result.frame).is_empty());
        assert!(
            detected
                .result
                .frame
                .warnings
                .iter()
                .all(|warning| warning != ori3_rigid::PENETRATION_WARNING)
        );

        // 旧 `detect=false` が選んでいた単発solveは、同じ入力で紙を閉じられず、
        // その裂けたFrameに7組の交差を作っていた。これは有効な紙のすり抜けではなく、
        // 接触設定にclosure rescueまで結合していた不具合である。
        let direct = final_direct.expect("旧単発経路の-150°結果を返す");
        assert!(direct.closure_rms > 1e-3, "rms={:.15e}", direct.closure_rms);
        let witnesses = ori3_rigid::contact_witnesses(&direct.frame);
        assert_eq!(witnesses.len(), 7, "witnesses={witnesses:?}");
        let mut reported_pairs = ori3_rigid::self_intersection_pairs(&direct.frame);
        let mut witness_pairs = witnesses
            .iter()
            .map(|witness| witness.faces)
            .collect::<Vec<_>>();
        reported_pairs.sort_unstable();
        witness_pairs.sort_unstable();
        assert_eq!(reported_pairs, witness_pairs);
        println!("crane direct -150° closure_rms={:.15e}", direct.closure_rms);
        for witness in &witnesses {
            assert!(witness.penetration_depth.is_finite());
            assert!(witness.penetration_depth > 0.0);
            println!(
                "crane -150° faces={}/{} depth={:.15e}",
                witness.faces.0, witness.faces.1, witness.penetration_depth
            );
        }
    }

    #[test]
    fn five_works_have_expected_flat_fold_rule_counts_for_reached_targets() {
        let folded_sample: Document = serde_json::from_str(include_str!(
            "../../../../crates/ori3-layers/tests/fixtures/folded-sample.ori3"
        ))
        .expect("折り上がりの標本を読む");
        let folded_sample_targets = sequence_targets(&folded_sample);
        let folded_sample_counts = flat_fold_rule_counts(&folded_sample.cp, &folded_sample_targets);

        let crane = front_fixture_cp(include_str!("../../src/lib/__fixtures__/crane.json"));
        let crane_targets = all_crease_flat_targets(&crane);
        let crane_counts = flat_fold_rule_counts(&crane, &crane_targets);

        let frog = front_fixture_cp(include_str!("../../src/lib/__fixtures__/frog.json"));
        let frog_targets = all_crease_flat_targets(&frog);
        let frog_counts = flat_fold_rule_counts(&frog, &frog_targets);

        let yakko = yakko_cp();
        let yakko_targets = all_crease_flat_targets(&yakko);
        let yakko_counts = flat_fold_rule_counts(&yakko, &yakko_targets);

        let mut cushion_flower = square_store().doc.cp;
        for (a, b, kind) in [
            ([0.0, 0.0], [1.0, 1.0], EdgeKind::Valley),
            ([0.0, 1.0], [1.0, 0.0], EdgeKind::Valley),
            ([0.5, 0.0], [0.5, 1.0], EdgeKind::Mountain),
            ([0.0, 0.5], [1.0, 0.5], EdgeKind::Mountain),
        ] {
            ori3_cp::insert_segment(&mut cushion_flower, a, b, kind);
        }
        let cushion_flower_targets = all_crease_flat_targets(&cushion_flower);
        let cushion_flower_counts = flat_fold_rule_counts(&cushion_flower, &cushion_flower_targets);

        // (生の局所違反, ±180°候補, 通知点)。通知規則を姿勢解から切り離し、
        // 指定角到達済み・食い込みなしを入力として明示する。
        assert_eq!(folded_sample_counts, (6, 2, 0), "折り上がりの標本");
        assert_eq!(crane_counts, (3, 3, 0), "鶴");
        assert_eq!(frog_counts, (3, 3, 0), "カエル");
        assert_eq!(yakko_counts, (0, 0, 0), "やっこさん");
        assert_eq!(cushion_flower_counts, (1, 1, 0), "八枚花弁の座布団花");

        let total = [
            folded_sample_counts,
            crane_counts,
            frog_counts,
            yakko_counts,
            cushion_flower_counts,
        ]
        .into_iter()
        .fold((0, 0, 0), |sum, counts| {
            (sum.0 + counts.0, sum.1 + counts.1, sum.2 + counts.2)
        });
        assert_eq!(total, (13, 9, 0));
    }

    fn current_flat_state(store: &DocumentStore) -> ori3_layers::FlatState {
        let (state, warnings) =
            ori3_layers::flat_state_at(&store.doc, &store.faces, store.doc.sequence.len())
                .expect("現在の手順は平坦状態");
        assert!(warnings.is_empty(), "再生警告={warnings:?}");
        state
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

    /// 画面での1回の入力から出た複数の線を、履歴1件として確定する(不具合D05)。
    /// 曲線1本の最大は201点・598本なので、上限100を超える本数で確かめる。
    #[test]
    fn apply_edits_records_one_history_entry_for_one_gesture() {
        let mut store = square_store();
        let before = store.doc.clone();
        let lines: Vec<EditOp> = (0..598)
            .map(|i| {
                let y = 0.001 + 0.001 * f64::from(i);
                EditOp::AddSegment {
                    a: [0.0, y],
                    b: [1.0, y],
                    kind: EdgeKind::Valley,
                }
            })
            .collect();

        let view = store.apply_edits(lines).unwrap();
        assert!(view.doc.cp.edges.len() > 598, "598本すべてが入る");
        assert_eq!(store.undo_stack.len(), 1, "履歴は1件だけ");

        let after = store.doc.clone();
        let view = store.undo().unwrap();
        assert_eq!(view.doc, before, "元に戻す1回で引く前へ戻る");
        assert!(store.undo().is_err(), "1回で戻り切っている");

        let view = store.redo().unwrap();
        assert_eq!(view.doc, after, "やり直し1回で引いた後へ戻る");
    }

    /// 途中の操作が断られたら1つも適用しない(片側だけ引かれた形にしない)。
    #[test]
    fn apply_edits_rejected_partway_changes_nothing() {
        let mut store = square_store();
        let before = store.doc.clone();

        let err = store
            .apply_edits(vec![
                diagonal(),
                // 折り線がある状態では紙サイズを変更できない
                EditOp::SetPaper {
                    paper: Paper {
                        width_mm: 100.0,
                        height_mm: 100.0,
                    },
                },
            ])
            .unwrap_err();

        assert!(err.contains("紙サイズ"), "err={err}");
        assert_eq!(store.doc, before, "断られたら1本も引かれない");
        assert!(store.undo_stack.is_empty(), "履歴も積まない");
    }

    /// 提案が返す展開図の代わり。折り線が1本入っていて、面を2つ取り出せる。
    fn proposal_cp() -> CreasePattern {
        let mut store = square_store();
        store.apply_edit(diagonal()).unwrap();
        store.doc.cp.clone()
    }

    /// 面を1つも取り出せない展開図(わざと失敗させるための入力)。
    fn cp_without_faces() -> CreasePattern {
        CreasePattern {
            vertices: Vec::new(),
            edges: Vec::new(),
            next_vertex_id: 0,
            next_edge_id: 0,
        }
    }

    /// 展開図と折り手順が、利用者の1操作としてまとめて入る(作業28)。
    #[test]
    fn apply_proposal_puts_the_crease_pattern_and_the_fold_order_in_together() {
        let mut store = square_store();
        let before = store.doc.clone();
        let cp = proposal_cp();
        let steps = vec![step(0), step(1), step(2)];

        let view = store.apply_proposal(cp.clone(), steps.clone()).unwrap();

        assert_eq!(view.doc.cp, cp, "展開図が入る");
        assert_eq!(view.doc.sequence, steps, "同じ1回で折り手順も入る");
        assert_eq!(view.doc.sequence.len(), 3, "手順の数は提案どおり");
        assert_eq!(store.doc.cp, cp);
        assert_eq!(store.doc.sequence, steps);
        assert_eq!(store.undo_stack.len(), 1, "履歴は1件だけ");
        assert_ne!(store.doc, before);
    }

    /// 断られたら展開図も折り手順も変わらない(片方だけ入った形にしない)。
    #[test]
    fn apply_proposal_rejected_changes_nothing() {
        let mut store = square_store();
        store.apply_seq(SeqOp::PushStep { step: step(7) }).unwrap();
        let before = store.doc.clone();
        let history = store.undo_stack.len();

        // 折り手順の番号が重なっている
        let err = store
            .apply_proposal(proposal_cp(), vec![step(0), step(0)])
            .unwrap_err();
        assert!(err.contains("二重"), "err={err}");
        assert_eq!(store.doc, before, "展開図も手順も入る前のまま");
        assert_eq!(store.undo_stack.len(), history, "履歴も積まない");

        // 面を1つも取り出せない展開図
        let err = store
            .apply_proposal(cp_without_faces(), vec![step(0)])
            .unwrap_err();
        assert!(err.contains("面"), "err={err}");
        assert_eq!(store.doc, before);
        assert_eq!(store.undo_stack.len(), history);
    }

    /// 元に戻す1回で、展開図も折り手順も入れる前へ戻る。
    #[test]
    fn one_undo_puts_back_both_the_crease_pattern_and_the_fold_order() {
        let mut store = square_store();
        store.apply_seq(SeqOp::PushStep { step: step(7) }).unwrap();
        let before = store.doc.clone();

        store
            .apply_proposal(proposal_cp(), vec![step(0), step(1)])
            .unwrap();
        assert_eq!(store.doc.sequence.len(), 2);
        assert_ne!(store.doc.cp, before.cp);

        let view = store.undo().unwrap();
        assert_eq!(view.doc.cp, before.cp, "展開図が戻る");
        assert_eq!(view.doc.sequence, before.sequence, "折り手順も戻る");
        assert_eq!(store.doc, before, "1回で入れる前へ戻り切っている");
    }

    /// 作業30: 名前付き「頭1・尾1・足4」の提案を端から端まで適用し、
    /// 元に戻す1回で元の作品一式へ戻す。
    ///
    /// # 探索の当たり外れへ主張をぶら下げない(`CLAUDE.md` §10.7.9 / §10.7.7)
    ///
    /// **2026-08-23に書き直した。** 以前はこの検査の入口に
    /// 「最後まで確認できた候補が1件以上ある」という下限があり、
    /// **折り方の探索が壁時計の打ち切りまでに完成へ届いたかどうか**を前提にしていた。
    /// その打ち切りは `crate::commands` の `PLAN_BUDGET` の `max_millis = 6_000`(6秒)で、
    /// **最適化ありの実測から決めた値**である。ところがこの検査は
    /// `cargo test -p desktop --lib`、つまり**最適化なし**で走る。
    /// 同じ骨格・同じ紙・同じ種で、組み立て方と候補の作り方だけを変えて測ると:
    ///
    /// | 組み立て | 候補#0の探索 | 候補#2の探索 | 6,000msに対して | 前提を満たすか |
    /// |---|---:|---:|---|---|
    /// | 最適化なし | 1,596 ms | 1,169 ms | 3.8倍の余裕 | 満たす |
    /// | 最適化なし・候補の作り方を広げたとき | **9,111 ms** | **12,388 ms** | **1.5〜2.1倍の超過** | **満たさない** |
    /// | 最適化あり | 74 ms | 73 ms | 81倍の余裕 | 満たす |
    /// | 最適化あり・候補の作り方を広げたとき | 449 ms | 617 ms | 9.7倍の余裕 | 満たす |
    ///
    /// 最適化なしは最適化ありより **16.8〜20.5倍**遅い(上表の実測から)。
    /// つまり前提が成り立つかどうかは、**折り方が正しいかではなく、
    /// どの組み立てで、どれだけ速い計算機で走らせたか**で反転する。
    /// CIの計算機は手元より約3.6倍遅いので、候補の作り方を広げる前の 1,596ms でも
    /// CI換算 5,746ms、6,000msまでの余裕は **1.04倍**しか無かった
    /// (§10.7.9が禁じる「余裕0の境目」そのものだった)。
    ///
    /// そこで**「提案が1操作で入り、取り消し1回で元の作品一式へ戻る」という主張は
    /// 一切弱めず**、その主張を確かめる相手を探索の当たり外れから切り離した。
    ///
    /// - 探索が何を返しても成り立つ**形の条件**は、返ってきた折り方**全件**にかける。
    ///   以前の「0手の候補が1件以上ある」という下限は、
    ///   「**どの候補も0手の折り方を運ばない**」という全件の不変条件へ置き換えた。
    ///   下限と違い、探索が何手見つけたかに左右されない。
    /// - 適用と取り消しは、**探索を通らない相手を必ず1件混ぜて**確かめる。
    ///   これで、探索が完成手順を返せなかった実行でも
    ///   「取り消し1回で元へ戻る」を必ず1回は通す(空回りしない)。
    /// - 探索が完成手順を返した候補は、いままでどおり
    ///   手順の数・再生・飛ばされた手まで端から端まで確かめる。
    #[test]
    fn checked_head_tail_four_legs_proposal_is_consumed_and_one_undo_restores_the_work() {
        use ori3_propose::skeleton::{Skeleton, SkeletonNode};

        // 正本の標本: 胴0から頭1・尾1（長さ1）と足4（長さ0.7）を出す。
        let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
        nodes.push(SkeletonNode::new(1, Some(0), 1.0));
        nodes.push(SkeletonNode::new(2, Some(0), 1.0));
        for id in 3..=6 {
            nodes.push(SkeletonNode::new(id, Some(0), 0.7));
        }
        let candidates = crate::commands::proposal_generate(
            Skeleton { nodes },
            Paper {
                width_mm: 150.0,
                height_mm: 150.0,
            },
            2026,
            true,
        )
        .expect("頭1・尾1・足4の候補を作れるはず");
        assert!(!candidates.is_empty(), "提案が候補を1件も返していない");

        // 探索が何手見つけたかに関係なく成り立つ形の条件を、返ってきた折り方の全件へかける。
        // 「0手を折り方ありとして運ばない」を、以前の「0手の候補が1件以上ある」という
        // 探索まかせの下限ではなく、全件の不変条件として言う。
        for (index, candidate) in candidates.iter().enumerate() {
            let candidate_no = index + 1;
            let Some(plan) = &candidate.fold_plan else {
                continue;
            };
            let details = plan.details();
            assert!(
                !details.steps.is_empty(),
                "候補{candidate_no}: 0手の折り方を運んでいる"
            );
            assert_eq!(
                details.checked,
                details.steps.len(),
                "候補{candidate_no}: 確かめた手数と運んだ手順の数が食い違う"
            );
            assert!(
                details.checked <= details.planned,
                "候補{candidate_no}: 確かめた手数{}が見つけた手数{}を超えている",
                details.checked,
                details.planned
            );
            if plan.checked_to_finish() {
                assert!(
                    details.planned > 0,
                    "候補{candidate_no}: 完成手順が0手になっている"
                );
                assert_eq!(
                    details.checked, details.planned,
                    "候補{candidate_no}: 全手順を確認していない"
                );
            }
        }

        // 適用と取り消しを確かめる相手を並べる。
        //
        // 先頭の1件は**探索を通らない**。名前付き骨格から実際に生成された展開図に、
        // 番号だけの手順を載せたものである(`apply_proposal` が求めるのは
        // 「面が取り出せる展開図」と「番号が重ならない手順」だけ)。
        // 探索が完成手順を返せなかった実行でも、この1件があるので
        // 「取り消し1回で元の作品一式へ戻る」の確認が空回りしない。
        let generated_cp = candidates[0].cp.clone();
        assert!(
            !ori3_cp::extract_faces(&generated_cp).is_empty(),
            "生成された展開図から紙の面を取り出せない"
        );
        // (名前, 展開図, 手順, 探索が完成まで確認した折り方か)
        let mut proposals: Vec<(String, CreasePattern, Vec<FoldStep>, bool)> = vec![(
            "生成された展開図と、番号だけの手順".to_string(),
            generated_cp,
            vec![step(0), step(1)],
            false,
        )];
        for (index, candidate) in candidates.iter().enumerate() {
            let Some(plan) = &candidate.fold_plan else {
                continue;
            };
            if !plan.checked_to_finish() {
                continue;
            }
            let details = plan.details();
            proposals.push((
                format!("候補{}の、完成まで確認できた折り方", index + 1),
                details.cp.clone(),
                details.steps.clone(),
                true,
            ));
        }
        // 先頭の1件は上で必ず入れているので、この下限は探索の結果に依存しない。
        assert!(
            !proposals.is_empty(),
            "確かめる相手が0件(先頭の1件を入れ損ねている)"
        );

        // 並べた相手を最後まで走査するため、各相手が以下のassertを1件ずつ全て通ることが
        // 「対象全件100%」の証拠になる。
        for (name, cp, steps, from_search) in &proposals {
            // 相手ごとに新しいstoreを使う。提案を入れる前にも展開図と折り手順を
            // 持つ作品を用意し、Document全体の一致で紙・表示・展開図・手順を
            // まとめて検査する。
            let mut store = square_store();
            store.apply_edit(diagonal()).unwrap();
            store.apply_seq(SeqOp::PushStep { step: step(7) }).unwrap();
            let before = store.doc.clone();
            let history_before = store.undo_stack.len();

            let mut applied = store
                .apply_proposal(cp.clone(), steps.clone())
                .unwrap_or_else(|error| panic!("{name}: 提案を適用できない: {error}"));
            assert_eq!(
                store.undo_stack.len(),
                history_before + 1,
                "{name}: 適用が1操作になっていない"
            );
            assert_eq!(applied.doc.cp, *cp, "{name}: 展開図を全て格納");
            assert_eq!(
                applied.doc.sequence, *steps,
                "{name}: 全手順を同じ操作で格納"
            );

            // 再生して確かめるのは、探索が「最後まで確認できた」と言った折り方だけ。
            // 番号だけの手順に同じことを求めるのは筋が違う。
            if *from_search {
                attach_replay(&mut applied);
                assert!(
                    applied.frame.is_some(),
                    "{name}: 手順を最後まで再生した立体が無い"
                );
                assert!(
                    applied.skipped.is_empty(),
                    "{name}: 飛ばされた手順がある: {:?}",
                    applied.skipped
                );
            }

            // ここが弱めてはいけない主張。相手が探索由来かどうかに関わらず全件でかける。
            let restored = store
                .undo()
                .unwrap_or_else(|error| panic!("{name}: 元に戻す1回が失敗した: {error}"));
            assert_eq!(restored.doc, before, "{name}: 元の作品一式へ戻らない");
            assert_eq!(
                store.doc, before,
                "{name}: store内部が元の作品一式へ戻らない"
            );
            assert_eq!(
                store.undo_stack.len(),
                history_before,
                "{name}: 提案適用の履歴1件だけを消費していない"
            );
        }
    }

    #[test]
    fn apply_edits_rejects_empty_request() {
        let mut store = square_store();
        assert!(store.apply_edits(Vec::new()).is_err());
        assert!(store.undo_stack.is_empty());
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

    /// 取り消し履歴(`undo_stack`)がヒープに保持しているバイト数を**直接**数える。
    ///
    /// プロセス全体の確保量ではなく、履歴そのものの大きさだけを数える。
    /// 数える対象が `undo_stack` の中身に限られるので、他のテストと同時に走らせても
    /// 値は変わらない(以前のプロセス共通カウンタ方式は、並列実行時に他のテストの
    /// 確保量が混入し、同じ検査で800,384バイト↔58,315,019〜78,370,317バイトと
    /// 65〜98倍の食い違いを出していた。実測は `scratchpad/undo-memory-fix-report.md`)。
    ///
    /// `Vec`・`String` は `len` ではなく `capacity` で数える。アロケータへ実際に
    /// 要求している大きさは `capacity` の分だからである。
    ///
    /// 数え漏れが黙って起きないよう、構造体は**全フィールドを並べた `let` 分解**、
    /// 列挙は**全変種を並べた `match`** で書く。`ori3-model` 側にフィールドや変種が
    /// 増えたら、この関数のコンパイルが通らなくなる。
    fn undo_history_heap_bytes(store: &DocumentStore) -> usize {
        let mut total = store.undo_stack.capacity() * size_of::<Snapshot>();
        for snapshot in &store.undo_stack {
            let Snapshot { doc, step_creases } = snapshot;
            total += document_heap_bytes(doc);
            total += step_creases.capacity() * size_of::<StepCreases>();
            for step_crease in step_creases {
                let StepCreases { step: _, lines } = step_crease;
                total += lines.capacity() * size_of::<[[f64; 2]; 2]>();
            }
        }
        total
    }

    fn document_heap_bytes(doc: &Document) -> usize {
        let Document {
            schema_version: _,
            paper,
            cp,
            sequence,
            display,
        } = doc;
        let CreasePattern {
            vertices,
            edges,
            next_vertex_id: _,
            next_edge_id: _,
        } = cp;
        let mut total = vertices.capacity() * size_of::<Vertex>()
            + vertices.iter().map(vertex_heap_bytes).sum::<usize>()
            + edges.capacity() * size_of::<Edge>()
            + edges.iter().map(edge_heap_bytes).sum::<usize>()
            + sequence.capacity() * size_of::<FoldStep>()
            + paper_heap_bytes(paper)
            + display_heap_bytes(display);
        for fold_step in sequence {
            total += fold_step_heap_bytes(fold_step);
        }
        total
    }

    fn fold_step_heap_bytes(fold_step: &FoldStep) -> usize {
        let FoldStep {
            id: _,
            kind: _, // TechniqueKind: 値を持たない列挙(ヒープ無し)
            drivers,
            layer_order,
            alignment,
            finish_soft,
            note,
        } = fold_step;
        let mut total = drivers.capacity() * size_of::<DriverLine>()
            + drivers.iter().map(driver_line_heap_bytes).sum::<usize>()
            + note.capacity();
        if let Some(layer_order) = layer_order {
            total += layer_order.capacity() * size_of::<[f64; 2]>();
        }
        if let Some(FoldAlignment { mode: _, picks }) = alignment {
            total += picks.capacity() * size_of::<AlignmentTarget>()
                + picks.iter().map(alignment_target_heap_bytes).sum::<usize>();
        }
        total += finish_soft.as_ref().map_or(0, finish_soft_heap_bytes);
        total
    }

    /// 以下の5つは「この型はヒープを使わない」ことをコンパイル時に確かめるための関数。
    /// 全フィールド・全変種を並べているので、ヒープを持つフィールドが増えたら
    /// 分解に失敗してコンパイルが通らなくなる。
    fn vertex_heap_bytes(vertex: &Vertex) -> usize {
        let Vertex { id: _, pos: _ } = vertex;
        0
    }

    fn edge_heap_bytes(edge: &Edge) -> usize {
        let Edge {
            id: _,
            v0: _,
            v1: _,
            kind: _, // EdgeKind: 値を持たない列挙
        } = edge;
        0
    }

    fn driver_line_heap_bytes(driver: &DriverLine) -> usize {
        let DriverLine {
            a: _,
            b: _,
            target_angle_deg: _,
        } = driver;
        0
    }

    fn alignment_target_heap_bytes(target: &AlignmentTarget) -> usize {
        match target {
            AlignmentTarget::Point { p: _ } => 0,
            AlignmentTarget::Line { a: _, b: _ } => 0,
        }
    }

    fn finish_soft_heap_bytes(finish_soft: &FinishSoftSettings) -> usize {
        let FinishSoftSettings {
            enabled: _,
            stiffness: _,
            pressure: _,
        } = finish_soft;
        0
    }

    fn paper_heap_bytes(paper: &Paper) -> usize {
        let Paper {
            width_mm: _,
            height_mm: _,
        } = paper;
        0
    }

    fn display_heap_bytes(display: &DisplaySettings) -> usize {
        let DisplaySettings {
            front_color: _,
            back_color: _,
            grid_divisions: _,
            soft_enabled: _,
            soft_stiffness: _,
            soft_pressure: _,
            overlap_prevention_enabled: _,
            penetration_prevention_enabled: _,
        } = display;
        0
    }

    #[test]
    fn undo_history_after_100_edits_of_a_crease_rich_document_stays_under_a_measured_budget() {
        // SYS-002(記憶使用量): 折り目の多い作品(カエル、`frog.json`、280辺・141頂点。
        // 標本4件(鶴61辺・やっこさん・カエル・鳥の基本形)のうち辺数が最大)を土台に、
        // 取り消し履歴を100段積んだ直後に、その履歴がヒープに保持している量を実測する。
        // 1頂点をわずかに動かすだけの編集を100回繰り返し、undo_stackへ本当に
        // 100個の異なるSnapshot(展開図まるごとの複製)を積ませる。
        //
        // 測るのは`undo_history_heap_bytes`が数える「履歴そのものの大きさ」で、
        // プロセス全体の確保量ではない。前者は同時に走る他のテストの影響を受けないが、
        // 後者は受ける(実測: 並列実行で65〜98倍に膨れた。
        // `scratchpad/undo-memory-fix-report.md` §段階1)。
        let mut store = DocumentStore::default();
        store.doc.cp = front_fixture_cp(include_str!("../../src/lib/__fixtures__/frog.json"));
        store.faces = ori3_cp::extract_faces(&store.doc.cp);

        let before_thread_bytes = crate::alloc_probe::current_thread_bytes();
        for i in 1..=100u32 {
            store
                .apply_edit(EditOp::MoveVertex {
                    id: 0,
                    to: [-0.0001 * f64::from(i), -0.0001 * f64::from(i)],
                })
                .expect("頂点をわずかに動かす編集は100回とも成功する");
        }
        assert_eq!(
            store.undo_stack.len(),
            MAX_UNDO,
            "100段分の取り消し履歴が積まれている"
        );
        let allocated_on_this_thread =
            crate::alloc_probe::current_thread_bytes() - before_thread_bytes;

        // 本体の測定: 取り消し履歴そのものがヒープに保持している量を直接数える。
        // プロセス全体の確保量に頼らないので、他のテストと同時に走らせても値は同じ。
        let measured_bytes = undo_history_heap_bytes(&store) as i64;
        eprintln!(
            "[SYS-002実測] 100段の取り消し履歴 = {measured_bytes} バイト \
             (この検査スレッドの正味確保量 = {allocated_on_this_thread} バイト)"
        );

        assert!(
            measured_bytes > 0,
            "100段の履歴を積んだのに、履歴の大きさが0以下({measured_bytes}バイト)"
        );
        // ここはバイト数(整数)どうしの比較なので、厳密に比べてよい
        // (CLAUDE.md §10.7.9が許容差を求めるのは、計算で出た小数の場合)。
        // 上限は`UNDO_HISTORY_BUDGET_BYTES`のコメントに記録した実測へ余裕を掛けた値。
        // 実測そのものを境目にしない。
        assert!(
            measured_bytes < UNDO_HISTORY_BUDGET_BYTES,
            "取り消し履歴100段の大きさが{measured_bytes}バイトで、\
             余裕を取った上限{UNDO_HISTORY_BUDGET_BYTES}バイトを超えた"
        );

        // 裏取り: 直接数えた大きさが、アロケータへ実際に要求された量と食い違っていないか。
        // 100回のループで正味残るのはほぼ履歴の分だけなので、両者は近い値になる。
        // 実測(2026-08-23、この作業機、debugビルド): 直接数え=813,352バイト、
        // スレッドの正味確保量=800,384バイト、差は12,968バイト(直接数えの1.6%)。
        // 差は履歴以外(`self.faces`の作り直し等)の増減が入るため0にはならない。
        // 幅は実測の差の桁に対して十分な余裕を取り、直接数えの±25%とする
        // (数え漏れがあれば必ず気づく程度には狭く、環境差では落ちない程度には広い)。
        let gap = (allocated_on_this_thread - measured_bytes).abs();
        assert!(
            gap * 4 < measured_bytes,
            "直接数えた履歴の大きさ{measured_bytes}バイトと、\
             アロケータへ要求された{allocated_on_this_thread}バイトが\
             {gap}バイトも食い違っている(数え漏れの疑い)"
        );
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
                    grid_divisions: 1024,
                    ..Default::default()
                },
            })
            .unwrap();
        assert_eq!(view.doc.display.front_color, [0, 128, 255]);
        assert_eq!(view.doc.display.back_color, [16, 16, 16]);
        assert_eq!(view.doc.display.grid_divisions, 1024);
        assert!(view.warnings.is_empty(), "warnings={:?}", view.warnings);
        assert!(store.is_dirty(), "作品が変わったので未保存になる");

        // 範囲外(0と上限超過)は丸めて警告する
        let view = store
            .apply_edit(EditOp::SetDisplay {
                display: ori3_model::DisplaySettings {
                    grid_divisions: MAX_GRID_DIVISIONS + 1,
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

        // 上限超過はすでに1024なので履歴を増やさず、実際に変わった2回を戻す
        for _ in 0..2 {
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
        let (doc, faces, warm, overlap_enabled, penetration_enabled) = store.pose_inputs();
        assert_eq!(doc, store.doc);
        assert_eq!(faces.len(), 1, "正方形1面のはず");
        assert_eq!(warm, Some(HashMap::from([(6u32, 90.0f64)])));
        // 形を変える補正は、利用者が明示的に選んだ場合だけ有効にする。
        assert!(!overlap_enabled, "重なり防止は既定オフ");
        // 形を変えない食い込みの検出と警告は既定で有効。
        assert!(penetration_enabled, "食い込み検出は既定オン");

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
        view.contact_detected = true;
        attach_replay(&mut view);
        assert!(view.frame.is_none());
        assert!(view.skipped.is_empty());
        assert!(!view.contact_detected, "手順なしは食い込み検出なしに戻す");

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
        assert_eq!(view.sequence_targets.len(), 1);
        assert_eq!(view.sequence_targets[0].target_angle_deg, 180.0);
        assert!(!view.angles.is_empty(), "実角度がビューへ運ばれる");
        assert!(view.relaxations.is_empty());
        assert!(view.closure_rms.is_some_and(f64::is_finite));
        assert!(!view.best_effort);
        assert!(view.converged);
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

    /// 利用者の固定標本が、12本の符号だけから同じ平坦姿勢へ戻ることを確かめる。
    /// 初期平面では対象0面だが、再現した姿勢では対象10面になるため、入力姿勢を
    /// FoldThroughへ渡さず元平面で計算する退行を数値で区別できる。
    #[test]
    fn crane_head_fixture_pose_has_ten_moving_faces_and_non_id_layer_order() {
        let document = crane_head_document();
        let faces = ori3_cp::extract_faces(&document.cp);
        assert_eq!(document.cp.vertices.len(), 13);
        assert_eq!(document.cp.edges.len(), 28);
        assert_eq!(faces.len(), 16);
        assert_eq!(
            CRANE_HEAD_POSE_DRIVERS
                .iter()
                .filter(|(_, angle)| *angle == 180.0)
                .count(),
            8
        );
        assert_eq!(
            CRANE_HEAD_POSE_DRIVERS
                .iter()
                .filter(|(_, angle)| *angle == -180.0)
                .count(),
            4
        );

        let (initial, warnings) =
            ori3_layers::flat_state_at(&document, &faces, 0).expect("初期平面を得る");
        assert!(warnings.is_empty(), "初期平面の警告={warnings:?}");
        assert_eq!(
            crane_head_moving_faces(&document, &faces, &initial),
            Vec::<FaceId>::new(),
            "元の展開状態へ同じ線を当てると対象は0面"
        );

        let oracle = crane_head_oracle_state(&document, &faces);
        assert_eq!(oracle.order, CRANE_HEAD_CAPTURED_LAYER_ORDER);
        assert_ne!(
            oracle.order,
            (0..16).collect::<Vec<FaceId>>(),
            "証拠の層順はFaceId順ではない"
        );
        assert_eq!(
            crane_head_moving_faces(&document, &faces, &oracle),
            CRANE_HEAD_MOVING_FACES,
            "証拠から独立に組み立てた現在姿勢では可動側に10面ある"
        );

        let pose = solve_crane_head_pose(&document, &faces);
        let mut saved_angles = HashMap::new();
        for driver in &pose.step.drivers {
            for edge in ori3_layers::resolve_driver_edges(&document.cp, driver) {
                saved_angles.insert(edge, driver.target_angle_deg);
            }
        }
        for &(edge, expected) in CRANE_HEAD_POSE_DRIVERS {
            assert_eq!(
                saved_angles.get(&edge).copied(),
                Some(expected),
                "edge {edge}は+180/-180の符号まで保つ"
            );
        }
        let material_faces = faces.iter().map(|face| face.id).collect::<BTreeSet<_>>();
        let pose_faces = pose.state.order.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(pose.state.order.len(), faces.len(), "全16面を1回ずつ並べる");
        assert_eq!(pose_faces, material_faces, "層順は材料面の完全順列である");
        let mut face_id_order = faces.iter().map(|face| face.id).collect::<Vec<_>>();
        face_id_order.sort_unstable();
        assert_ne!(
            pose.state.order, face_id_order,
            "層順をFaceId順で代用しない"
        );
        let (compared_pairs, mismatches) = crane_head_overlap_order_mismatches(
            &document,
            &faces,
            &pose.state,
            CRANE_HEAD_CAPTURED_LAYER_ORDER,
        )
        .expect("正面積で重なる全ての面対を比較できる");
        eprintln!(
            "crane_head_positive_overlap_pairs={compared_pairs} mismatches={}",
            mismatches.len()
        );
        assert!(compared_pairs > 0, "物理的な上下を比較する重なり面対がある");
        assert!(
            mismatches.is_empty(),
            "独立捕捉した層順と上下が異なる正面積重なり面対={mismatches:?}"
        );
        let validation = ori3_layers::precrease_collapse::validate_precrease_layer_order(
            &document.cp,
            &faces,
            &pose.state.placements,
            &pose.state.order,
        )
        .expect("canonical層順の一般制約を検査できる");
        assert!(
            validation.is_valid(),
            "canonical層順が一般制約に違反する: violations={:?}, discarded_relations={:?}",
            validation.violations,
            validation.discarded_relations
        );
        assert_eq!(
            crane_head_moving_faces(&document, &faces, &pose.state),
            CRANE_HEAD_MOVING_FACES,
            "12本だけから導出した姿勢も、証拠と同じ10面を選ぶ"
        );

        let reversed = ori3_layers::replay::canonical_flat_pose_at(
            &document,
            &faces,
            0,
            &ori3_model::FoldPoseInput {
                drivers: CRANE_HEAD_POSE_DRIVERS
                    .iter()
                    .rev()
                    .map(|&(edge_id, target_angle_deg)| ori3_model::FoldPoseDriver {
                        edge_id,
                        target_angle_deg,
                    })
                    .collect(),
            },
        )
        .expect("同じ書類と指定なら、入力順を変えても再現できる");
        assert_eq!(reversed.state, pose.state, "操作順を隠れた入力にしない");
        assert_eq!(
            serde_json::to_vec(&reversed.step).expect("逆順の保存手順を比較できる"),
            serde_json::to_vec(&pose.step).expect("正順の保存手順を比較できる"),
            "入力順に関係なく同じPose手順を保存する"
        );
    }

    /// 段階2のread-only設計probe。canonical入口がlive/warm無しで返す候補を測る。
    #[test]
    fn probe_crane_head_canonical_preferred_pose() {
        let document = crane_head_document();
        let faces = ori3_cp::extract_faces(&document.cp);
        let preferred = CRANE_HEAD_POSE_DRIVERS
            .iter()
            .copied()
            .collect::<HashMap<_, _>>();
        let document_seed = document
            .cp
            .edges
            .iter()
            .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
            .map(|edge| (edge.id, 0.0))
            .collect::<HashMap<_, _>>();
        let solved = ori3_rigid::motion::solve_canonical_motion_with_contact_options(
            &document.cp,
            &faces,
            &[],
            Some(&preferred),
            Some(&document_seed),
            ori3_rigid::MotionContactOptions {
                detect: true,
                prevent: false,
            },
        );
        let explicit = CRANE_HEAD_POSE_DRIVERS
            .iter()
            .map(|&(edge, target)| (edge, target, solved.result.angles.get(&edge).copied()))
            .collect::<Vec<_>>();
        let mut all_angles = solved
            .result
            .angles
            .iter()
            .map(|(&edge, &angle)| (edge, angle, angle.to_bits()))
            .collect::<Vec<_>>();
        all_angles.sort_unstable_by_key(|&(edge, _, _)| edge);
        let mut ranks = solved
            .result
            .frame
            .faces
            .iter()
            .map(|face| (face.face, face.surface_rank))
            .collect::<Vec<_>>();
        ranks.sort_unstable_by_key(|&(face, _)| face);
        let max_abs_z = solved
            .result
            .frame
            .faces
            .iter()
            .flat_map(|face| face.polygon.iter())
            .map(|point| point[2].abs())
            .fold(0.0, f64::max);
        let moving = crane_head_moving_frame_faces(&solved.result.frame);
        eprintln!(
            "canonical crane-head probe: converged={} best_effort={} closure_rms={:.17e} contact_detected={} frame_warnings={:?} relaxations={:?} explicit={:?} all_angles(edge,value,bits)={:?} surface_order_authoritative={} surface_order={:?} ranks={:?} max_abs_z={:.17e} moving={:?}",
            solved.result.converged,
            solved.result.best_effort,
            solved.result.closure_rms,
            solved.contact_detected,
            solved.result.frame.warnings,
            solved.result.relaxations,
            explicit,
            all_angles,
            solved.surface_order_authoritative,
            solved.surface_order,
            ranks,
            max_abs_z,
            moving,
        );
        assert_eq!(solved.result.frame.faces.len(), 16);
        assert!(max_abs_z.is_finite());
        assert_eq!(explicit.len(), 12);
        assert_eq!(all_angles.len(), 20);
    }

    /// PreviewもApplyと同じ不変なpose_beforeを使い、Document・Undo・warm値を変えない。
    #[test]
    fn crane_head_pose_preview_is_non_destructive() {
        let mut store = crane_head_store();
        let before = store.atomicity_probe_for_test();

        store
            .apply_seq(crane_head_fold_op(true))
            .expect("12本の符号で再現した現在姿勢から折り候補を調べられる");

        assert_eq!(
            store.atomicity_probe_for_test(),
            before,
            "PreviewはDocument・履歴・表示用warm値を変更しない"
        );
    }

    #[test]
    fn crane_head_target_query_is_non_destructive_and_reports_varying_sections() {
        let mut store = crane_head_store();
        let before = store.atomicity_probe_for_test();
        reset_commit_count_for_test();

        let view = store
            .apply_seq(crane_head_target_query())
            .expect("fold-target query returns a normal response");

        assert_eq!(store.atomicity_probe_for_test(), before);
        assert_eq!(commit_count_for_test(), 0, "a query does not enter commit");
        assert_eq!(
            view.fold_target_info,
            Some(FoldTargetInfo {
                status: FoldTargetStatus::Varies,
                available_count: None,
                reason: Some(
                    "折り線の場所によって、同時に折れるひだの枚数が異なります。".to_string(),
                ),
                top_action: None,
            }),
            "the captured line has two, two and one complete pleats in its three intervals",
        );
    }

    #[test]
    fn invalid_fold_target_query_is_unavailable_and_non_destructive() {
        let mut store = square_store();
        let before = store.atomicity_probe_for_test();
        reset_commit_count_for_test();

        let view = store
            .apply_seq(SeqOp::PreviewFoldTargets {
                up_to: 0,
                line: [[0.5, 0.5], [0.5, 0.5]],
                keep_side_point: [0.0, 0.0],
                pose_before: None,
            })
            .expect("an unavailable query is a normal response");

        assert_eq!(store.atomicity_probe_for_test(), before);
        assert_eq!(commit_count_for_test(), 0);
        assert_eq!(
            view.fold_target_info,
            Some(FoldTargetInfo {
                status: FoldTargetStatus::Unavailable,
                available_count: None,
                reason: Some("この折り線で同時に折れるひだを確認できません。".to_string()),
                top_action: None,
            }),
        );
    }

    #[test]
    fn one_complete_pleat_query_reports_ready_without_mutation() {
        let (mut store, pose_before) = one_pleat_square_store();
        let before = store.atomicity_probe_for_test();
        reset_commit_count_for_test();

        let view = store
            .apply_seq(SeqOp::PreviewFoldTargets {
                up_to: 0,
                line: [[0.0, 0.5], [0.5, 0.5]],
                keep_side_point: [0.25, 0.75],
                pose_before: Some(pose_before),
            })
            .expect("a document-derived complete pair is queryable");

        assert_eq!(store.atomicity_probe_for_test(), before);
        assert_eq!(commit_count_for_test(), 0);
        assert_eq!(
            view.fold_target_info,
            Some(FoldTargetInfo {
                status: FoldTargetStatus::Ready,
                available_count: Some(1),
                reason: None,
                top_action: None,
            }),
        );
    }

    #[test]
    fn a_single_surface_query_stays_unavailable_to_preserve_existing_fold_modes() {
        let mut store = square_store();
        let before = store.atomicity_probe_for_test();
        reset_commit_count_for_test();

        let view = store
            .apply_seq(SeqOp::PreviewFoldTargets {
                up_to: 0,
                line: [[0.0, 0.5], [1.0, 0.5]],
                keep_side_point: [0.5, 0.75],
                pose_before: None,
            })
            .expect("a single-surface query returns a normal fallback response");

        assert_eq!(store.atomicity_probe_for_test(), before);
        assert_eq!(commit_count_for_test(), 0);
        assert_eq!(
            view.fold_target_info,
            Some(FoldTargetInfo {
                status: FoldTargetStatus::Unavailable,
                available_count: None,
                reason: Some("この折り線で同時に折れるひだを確認できません。".to_string()),
                top_action: None,
            }),
        );
    }

    #[test]
    fn incomplete_top_pair_maps_to_the_exact_crease_only_response() {
        let analysis = ori3_layers::FoldTargetAnalysis {
            pleats: ori3_layers::PleatAnalysis {
                scalar_count: Some(0),
                sections: vec![ori3_layers::PleatSectionAnalysis {
                    top_action: Some(ori3_layers::TopAction::CreaseOnlyTop {
                        surface_faces: vec![1],
                    }),
                    ..ori3_layers::PleatSectionAnalysis::default()
                }],
                reason: None,
            },
            section_surfaces_top_to_bottom: vec![vec![vec![1], vec![2]]],
        };

        assert_eq!(
            fold_target_info_from_analysis(&analysis),
            FoldTargetInfo {
                status: FoldTargetStatus::CreaseOnlyTop,
                available_count: Some(0),
                reason: Some("いちばん上の紙が最後まで折り重なっていないため、今回はひだをまとめて折りません。いちばん上の紙に折り目だけを付け、下の紙と3Dの形は動かしません。".to_string()),
                top_action: Some(FoldTargetTopAction::CreaseOnlyTop),
            },
        );
    }

    #[test]
    fn ready_and_limited_sections_with_the_same_safe_count_report_limited() {
        let pair = |upper, lower| ori3_layers::PleatPair {
            hinge_faces: (upper, lower),
            upper_surface_faces: vec![upper],
            lower_surface_faces: vec![lower],
            sign: ori3_layers::FullFoldSign::Positive180,
        };
        let analysis = ori3_layers::FoldTargetAnalysis {
            pleats: ori3_layers::PleatAnalysis {
                scalar_count: Some(1),
                sections: vec![
                    ori3_layers::PleatSectionAnalysis {
                        pairs_top_to_bottom: vec![pair(1, 2)],
                        ..ori3_layers::PleatSectionAnalysis::default()
                    },
                    ori3_layers::PleatSectionAnalysis {
                        pairs_top_to_bottom: vec![pair(1, 2)],
                        count_limit: Some(ori3_layers::PleatCountLimit::IncompleteBoundaryAfter {
                            count: 1,
                        }),
                        ..ori3_layers::PleatSectionAnalysis::default()
                    },
                ],
                reason: None,
            },
            section_surfaces_top_to_bottom: vec![vec![vec![1], vec![2]], vec![vec![1], vec![2]]],
        };

        assert_eq!(
            fold_target_info_from_analysis(&analysis),
            FoldTargetInfo {
                status: FoldTargetStatus::Limited,
                available_count: Some(1),
                reason: Some(
                    "上から1枚まで選べます。1枚目の下は、まだ最後まで折り重なっていません。"
                        .to_string(),
                ),
                top_action: None,
            },
            "one incomplete interval limits the shared safe count even when another interval ends ready",
        );
    }

    #[test]
    fn equal_counts_with_different_surface_pair_identity_are_unavailable() {
        let pair = |upper, lower| ori3_layers::PleatPair {
            hinge_faces: (upper, lower),
            upper_surface_faces: vec![upper],
            lower_surface_faces: vec![lower],
            sign: ori3_layers::FullFoldSign::Positive180,
        };
        let analysis = ori3_layers::FoldTargetAnalysis {
            pleats: ori3_layers::PleatAnalysis {
                scalar_count: Some(1),
                sections: vec![
                    ori3_layers::PleatSectionAnalysis {
                        pairs_top_to_bottom: vec![pair(1, 2)],
                        ..ori3_layers::PleatSectionAnalysis::default()
                    },
                    ori3_layers::PleatSectionAnalysis {
                        pairs_top_to_bottom: vec![pair(3, 4)],
                        ..ori3_layers::PleatSectionAnalysis::default()
                    },
                ],
                reason: None,
            },
            section_surfaces_top_to_bottom: vec![vec![vec![1], vec![2]], vec![vec![3], vec![4]]],
        };

        assert_eq!(
            fold_target_info_from_analysis(&analysis),
            unavailable_fold_target_info(),
            "equal counts are insufficient when the surface pair identity differs by interval",
        );
    }

    #[test]
    fn pleat_count_and_explicit_face_targets_are_mutually_exclusive_and_atomic() {
        let mut store = crane_head_store();
        let before = store.atomicity_probe_for_test();
        let mut operation = crane_head_fold_op(false);
        let SeqOp::FoldThrough {
            target_layers,
            target_pleat_count,
            ..
        } = &mut operation
        else {
            panic!("fixture is FoldThrough");
        };
        *target_layers = Some(vec![CRANE_HEAD_MOVING_FACES[0]]);
        *target_pleat_count = Some(1);
        reset_commit_count_for_test();

        let error = store
            .apply_seq(operation)
            .expect_err("the two target models must not be mixed");

        assert!(error.contains("同時には指定できません"), "{error}");
        assert_eq!(store.atomicity_probe_for_test(), before);
        assert_eq!(commit_count_for_test(), 0);

        let mut preview = crane_head_fold_op(true);
        let SeqOp::PreviewFoldThrough {
            target_layers,
            target_pleat_count,
            ..
        } = &mut preview
        else {
            panic!("fixture is PreviewFoldThrough");
        };
        *target_layers = Some(vec![CRANE_HEAD_MOVING_FACES[0]]);
        *target_pleat_count = Some(1);
        reset_commit_count_for_test();
        let error = store
            .apply_seq(preview)
            .expect_err("preview must reject the two target models too");
        assert!(error.contains("同時には指定できません"), "{error}");
        assert_eq!(store.atomicity_probe_for_test(), before);
        assert_eq!(commit_count_for_test(), 0);
    }

    #[test]
    fn invalid_or_varying_pleat_count_leaves_fold_and_preview_unchanged() {
        for preview in [false, true] {
            let (mut store, pose_before) = one_pleat_square_store();
            let operation = if preview {
                SeqOp::PreviewFoldThrough {
                    up_to: 0,
                    line: [[0.0, 0.5], [0.5, 0.5]],
                    keep_side_point: [0.25, 0.75],
                    target_layers: None,
                    target_pleat_count: Some(2),
                    direction: ori3_model::FoldDirection::Up,
                    pose_before: Some(pose_before),
                }
            } else {
                SeqOp::FoldThrough {
                    up_to: 0,
                    line: [[0.0, 0.5], [0.5, 0.5]],
                    keep_side_point: [0.25, 0.75],
                    target_layers: None,
                    target_pleat_count: Some(2),
                    direction: ori3_model::FoldDirection::Up,
                    alignment: None,
                    accept_additional_crease: false,
                    pose_before: Some(pose_before),
                }
            };
            let before = store.atomicity_probe_for_test();
            reset_commit_count_for_test();

            let error = store
                .apply_seq(operation)
                .expect_err("only one pleat is available");

            assert!(error.contains("この枚数のひだを折れません"), "{error}");
            assert_eq!(store.atomicity_probe_for_test(), before);
            assert_eq!(commit_count_for_test(), 0);
        }

        for preview in [false, true] {
            let mut store = crane_head_store();
            let mut operation = crane_head_fold_op(preview);
            match &mut operation {
                SeqOp::FoldThrough {
                    target_pleat_count, ..
                }
                | SeqOp::PreviewFoldThrough {
                    target_pleat_count, ..
                } => *target_pleat_count = Some(1),
                _ => panic!("fixture is a fold request"),
            }
            let before = store.atomicity_probe_for_test();
            reset_commit_count_for_test();

            let error = store
                .apply_seq(operation)
                .expect_err("a varying line has no single selectable count");

            assert_eq!(
                error,
                "折り線の場所によって、同時に折れるひだの枚数が異なります。"
            );
            assert_eq!(store.atomicity_probe_for_test(), before);
            assert_eq!(commit_count_for_test(), 0);
        }
    }

    #[test]
    fn pleat_count_and_spatial_grab_are_rejected_for_fold_and_preview() {
        for preview in [false, true] {
            let (mut store, pose_before) = one_pleat_square_store();
            let operation = if preview {
                SeqOp::PreviewFoldThrough {
                    up_to: 0,
                    line: [[0.0, 0.5], [0.5, 0.5]],
                    keep_side_point: [0.25, 0.75],
                    target_layers: None,
                    target_pleat_count: Some(1),
                    direction: ori3_model::FoldDirection::Up,
                    pose_before: Some(pose_before),
                }
            } else {
                SeqOp::FoldThrough {
                    up_to: 0,
                    line: [[0.0, 0.5], [0.5, 0.5]],
                    keep_side_point: [0.25, 0.75],
                    target_layers: None,
                    target_pleat_count: Some(1),
                    direction: ori3_model::FoldDirection::Up,
                    alignment: None,
                    accept_additional_crease: false,
                    pose_before: Some(pose_before),
                }
            };
            let before = store.atomicity_probe_for_test();
            reset_commit_count_for_test();

            let error = store
                .apply_seq_with_spatial(
                    operation,
                    Some(SpatialFoldSpec {
                        from: [0.0, 0.0, 0.0],
                        to: [1.0, 0.0, 0.0],
                        grab_face: 0,
                    }),
                )
                .expect_err("K selection and a live 3D grab must not be mixed");

            assert_eq!(
                error,
                "折るひだの枚数と3D上のつかみ位置を同時には指定できません"
            );
            assert_eq!(store.atomicity_probe_for_test(), before);
            assert_eq!(commit_count_for_test(), 0);
        }
    }

    #[test]
    fn one_pleat_is_recomputed_and_committed_as_one_undo_operation() {
        let (mut store, pose_before) = one_pleat_square_store();
        let expected_doc = store.doc.clone();
        let before = store.atomicity_probe_for_test();
        let operation = SeqOp::FoldThrough {
            up_to: 0,
            line: [[0.0, 0.5], [0.5, 0.5]],
            keep_side_point: [0.25, 0.75],
            target_layers: None,
            target_pleat_count: Some(1),
            direction: ori3_model::FoldDirection::Up,
            alignment: None,
            accept_additional_crease: false,
            pose_before: Some(pose_before),
        };
        reset_commit_count_for_test();

        let view = store
            .apply_seq(operation)
            .expect("Rust recomputes the top pleat and applies it");

        assert_eq!(commit_count_for_test(), 1);
        assert_eq!(store.undo_stack.len(), 1);
        assert_ne!(store.atomicity_probe_for_test(), before);
        assert_eq!(view.doc.sequence.len(), 2, "pose plus fold are one commit");
        let undone = store.undo().expect("one undo restores the document");
        assert_eq!(undone.doc, expected_doc);
    }

    #[test]
    fn one_pleat_preview_recomputes_k_without_committing() {
        let (mut store, pose_before) = one_pleat_square_store();
        let before = store.atomicity_probe_for_test();
        reset_commit_count_for_test();

        let view = store
            .apply_seq(SeqOp::PreviewFoldThrough {
                up_to: 0,
                line: [[0.0, 0.5], [0.5, 0.5]],
                keep_side_point: [0.25, 0.75],
                target_layers: None,
                target_pleat_count: Some(1),
                direction: ori3_model::FoldDirection::Up,
                pose_before: Some(pose_before),
            })
            .expect("preview recomputes the same top pleat as apply");

        assert_eq!(store.atomicity_probe_for_test(), before);
        assert_eq!(commit_count_for_test(), 0, "preview never enters commit");
        assert!(view.doc.sequence.is_empty());
    }

    #[test]
    fn explicit_top_face_path_still_works_without_a_pleat_count() {
        let (mut store, pose_before) = one_pleat_square_store();
        let pose =
            ori3_layers::replay::canonical_flat_pose_at(&store.doc, &store.faces, 0, &pose_before)
                .expect("derive the persisted top face");
        let top = *pose.state.order.last().expect("one top face");
        reset_commit_count_for_test();

        let view = store
            .apply_seq(SeqOp::FoldThrough {
                up_to: 0,
                line: [[0.0, 0.5], [0.5, 0.5]],
                keep_side_point: [0.25, 0.75],
                target_layers: Some(vec![top]),
                target_pleat_count: None,
                direction: ori3_model::FoldDirection::Up,
                alignment: None,
                accept_additional_crease: false,
                pose_before: Some(pose_before),
            })
            .expect("the pre-existing explicit top-face path remains available");

        assert_eq!(commit_count_for_test(), 1);
        assert_eq!(store.undo_stack.len(), 1);
        assert_eq!(view.doc.sequence.len(), 2);
    }

    #[test]
    fn single_surface_all_layers_fold_still_works_without_a_pleat_count() {
        let mut store = square_store();
        reset_commit_count_for_test();

        let view = store
            .apply_seq(SeqOp::FoldThrough {
                up_to: 0,
                line: [[0.0, 0.5], [1.0, 0.5]],
                keep_side_point: [0.5, 0.75],
                target_layers: None,
                target_pleat_count: None,
                direction: ori3_model::FoldDirection::Up,
                alignment: None,
                accept_additional_crease: false,
                pose_before: None,
            })
            .expect("the pre-existing all-layers fold handles one unfolded sheet");

        assert_eq!(commit_count_for_test(), 1);
        assert_eq!(store.undo_stack.len(), 1);
        assert_eq!(view.doc.sequence.len(), 1);
    }

    /// Poseを候補Documentへ作れた後でFoldThroughが失敗しても、
    /// 実Document・全履歴・warm値を一切変更せず、commitもしない。
    #[test]
    fn crane_head_pose_fold_failure_is_atomic() {
        let mut store = crane_head_store();
        let before = store.atomicity_probe_for_test();
        let mut operation = crane_head_fold_op(false);
        let SeqOp::FoldThrough { line, .. } = &mut operation else {
            panic!("確定用FoldThroughである");
        };
        *line = [[2.0, 0.0], [2.0, 1.0]];
        reset_commit_count_for_test();

        let error = store
            .apply_seq(operation)
            .expect_err("紙の外の折り線ではFoldThroughだけが失敗する");

        assert!(error.contains("折る対象の層がありません"), "{error}");
        assert_eq!(commit_count_for_test(), 0, "失敗した操作を確定しない");
        assert_eq!(
            store.atomicity_probe_for_test(),
            before,
            "Pose計算後の失敗でも全状態を元のまま保つ"
        );
    }

    /// 利用者の1操作は、pose_beforeを文書から再現してからFoldThroughを適用し、
    /// Pose+折りの2手を1回だけcommitする。保存後のcold replayも一致し、Undoは1回。
    #[test]
    fn crane_head_pose_fold_applies_replays_and_undoes_as_one_operation() {
        let mut store = crane_head_store();
        let original = store.doc.clone();
        reset_commit_count_for_test();

        let applied = store
            .apply_seq(crane_head_fold_op(false))
            .expect("12本の符号で再現した現在姿勢の10面をまとめて折れる");

        assert_eq!(commit_count_for_test(), 1, "利用者の1操作を1回だけ確定する");
        assert_eq!(store.undo_stack.len(), 1, "Undo履歴は1件だけ増える");
        assert_eq!(
            applied.doc.sequence.len(),
            2,
            "再現用PoseとFoldThroughを連続する2手として保存する"
        );
        assert_eq!(
            applied.doc.sequence[0].layer_order.as_ref().map(Vec::len),
            Some(16),
            "再現した16面の層順を最初の手へ保存する"
        );

        let replayed = ori3_layers::replay(&store.doc, store.doc.sequence.len(), 1.0);
        assert!(replayed.skipped.is_empty(), "保存直前の再生={replayed:?}");
        let expected_frame = serde_json::to_vec(&replayed.frame).expect("frameを比較できる");
        let path = std::env::temp_dir().join(format!(
            "ori3_store_test_{}_crane_head_pose.ori3",
            std::process::id()
        ));
        store.save(Some(&path)).expect("利用者標本を保存できる");
        let mut reopened = DocumentStore::default();
        let reopened_view = reopened.open(&path).expect("保存作品を読み直せる");
        assert_eq!(reopened_view.doc, applied.doc, "保存した全手順を失わない");
        let cold = ori3_layers::replay(&reopened.doc, reopened.doc.sequence.len(), 1.0);
        assert!(cold.skipped.is_empty(), "読み直した作品の再生={cold:?}");
        assert_eq!(
            serde_json::to_vec(&cold.frame).expect("cold frameを比較できる"),
            expected_frame,
            "保存後のcold replayも同じ3D結果になる"
        );
        std::fs::remove_file(&path).ok();

        let undone = store.undo().expect("1回で利用者操作の前へ戻る");
        assert_eq!(undone.doc, original);
        assert!(store.undo_stack.is_empty(), "2回目のUndoを必要としない");
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

    /// D16: 折りが展開図へ新しく足した折り線を、手順ごとの来歴として記録する。
    #[test]
    fn fold_records_the_crease_it_adds_to_the_cp() {
        let mut store = square_store();

        let view = store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .unwrap();

        let step_id = view.doc.sequence[0].id;
        let creases = view
            .step_creases
            .iter()
            .find(|c| c.step == step_id)
            .expect("折りの来歴を記録する");
        assert_eq!(creases.lines.len(), 1, "lines={:?}", creases.lines);
        let [a, b] = creases.lines[0];
        assert!(
            (a[0] - 0.5).abs() < 1e-9 && (b[0] - 0.5).abs() < 1e-9,
            "a={a:?} b={b:?}"
        );
        assert!(
            ((a[1] - b[1]).abs() - 1.0).abs() < 1e-9,
            "紙の端から端までの1本: a={a:?} b={b:?}"
        );
    }

    /// D16: 先に描いておいた折り線で折っても、その手順が線を足したことにはならない。
    /// 来歴が空になることで、2D画面はその線を「折る前」から出し続けられる。
    #[test]
    fn folding_along_a_predrawn_crease_records_no_added_line() {
        let mut store = square_store();
        store
            .apply_edit(EditOp::AddSegment {
                a: [0.5, 0.0],
                b: [0.5, 1.0],
                kind: EdgeKind::Valley,
            })
            .expect("先に折り線を描く");
        let drawn: Vec<EdgeId> = store
            .doc
            .cp
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Valley)
            .map(|e| e.id)
            .collect();
        assert_eq!(drawn.len(), 1, "描いた折り線は1本");

        let view = store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .unwrap();

        let step_id = view.doc.sequence[0].id;
        let creases = view
            .step_creases
            .iter()
            .find(|c| c.step == step_id)
            .expect("来歴は必ず記録する");
        assert!(
            creases.lines.is_empty(),
            "既にある折り線で折った手順は線を足していない: {:?}",
            creases.lines
        );
        assert!(
            view.doc.cp.edges.iter().any(|e| e.id == drawn[0]),
            "描いた折り線はそのまま残る"
        );
    }

    /// 元に戻す・やり直しで、展開図と一緒に来歴も行き来する。
    #[test]
    fn undo_and_redo_move_the_crease_history_with_the_document() {
        let mut store = square_store();
        store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .unwrap();
        assert_eq!(store.step_creases.len(), 1);

        let view = store.undo().unwrap();
        assert!(view.step_creases.is_empty(), "折る前に来歴は無い");
        assert!(store.step_creases.is_empty());

        let view = store.redo().unwrap();
        assert_eq!(view.step_creases.len(), 1, "やり直しで来歴も戻る");
    }

    /// 手順の並べ替え(削除+挿入)で来歴を落とさない。
    #[test]
    fn reordering_steps_keeps_the_crease_history() {
        let mut store = square_store();
        store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .unwrap();
        store
            .apply_seq(fold_op(1, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]))
            .unwrap();
        let before = store.step_creases.clone();
        assert_eq!(before.len(), 2);

        let moved = store.doc.sequence[1].clone();
        store.apply_seq(SeqOp::RemoveStep { id: moved.id }).unwrap();
        let view = store
            .apply_seq(SeqOp::InsertStep {
                index: 0,
                step: moved,
            })
            .unwrap();

        assert_eq!(view.doc.sequence.len(), 2);
        let mut after = view.step_creases.clone();
        after.sort_by_key(|c| c.step);
        assert_eq!(after, before, "並べ替えても各手順の来歴は変わらない");
    }

    /// 来歴を持たない旧形式の作品も、これまでどおり開ける。
    #[test]
    fn old_files_without_crease_history_still_open() {
        let mut store = square_store();
        store.apply_edit(diagonal()).unwrap();
        let path = std::env::temp_dir().join(format!(
            "ori3_store_test_{}_old_format.ori3",
            std::process::id()
        ));
        // 旧形式 = Documentだけを書き出したファイル(step_creasesの項目が無い)
        let text = serde_json::to_string_pretty(&store.doc).unwrap();
        assert!(!text.contains("step_creases"), "旧形式に来歴は入らない");
        std::fs::write(&path, &text).unwrap();
        let expected = store.doc.clone();

        let view = store.open(&path).expect("旧形式の作品を開ける");

        assert_eq!(view.doc, expected, "作品の内容は変わらない");
        assert!(view.step_creases.is_empty(), "来歴は空として読む");
        std::fs::remove_file(&path).ok();
    }

    /// 保存して開き直すと、手順ごとの来歴も元どおりになる。
    #[test]
    fn saving_and_opening_keeps_the_crease_history() {
        let mut store = square_store();
        store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .unwrap();
        let expected = store.step_creases.clone();
        let path = std::env::temp_dir().join(format!(
            "ori3_store_test_{}_history.ori3",
            std::process::id()
        ));
        store.save(Some(&path)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("step_creases"), "来歴も書き出す");

        let view = store.open(&path).expect("保存した作品を開ける");

        assert_eq!(view.step_creases, expected);
        std::fs::remove_file(&path).ok();
    }

    /// 消した手順の来歴は書き出さず、新しい手順IDとも衝突させない。
    #[test]
    fn removed_steps_do_not_leave_history_in_the_saved_file() {
        let mut store = square_store();
        store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .unwrap();
        let removed = store.doc.sequence[0].id;
        store.apply_seq(SeqOp::RemoveStep { id: removed }).unwrap();

        let view = store
            .apply_seq(fold_op(0, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]))
            .unwrap();

        assert_ne!(
            view.doc.sequence[0].id, removed,
            "消した手順の来歴が残る間は同じIDを使わない"
        );
        assert_eq!(
            view.step_creases.len(),
            1,
            "書き出す来歴は今ある手順の分だけ"
        );
        assert_eq!(view.step_creases[0].step, view.doc.sequence[0].id);
    }

    /// 「この形で仕上げる」に相当する90°のPoseを記録した後も、同じFoldThrough入口で
    /// 次の折りを受け付け、展開図・立体・手順をまとめて更新する。
    #[test]
    fn fold_through_after_a_ninety_degree_pose_updates_cp_frame_and_sequence() {
        let mut store = square_store();
        store
            .apply_edit(EditOp::AddSegment {
                a: [0.5, 0.0],
                b: [0.5, 1.0],
                kind: EdgeKind::Mountain,
            })
            .expect("中央へ折り目を入れる");
        let mut pose = step(0);
        pose.kind = TechniqueKind::Pose;
        pose.drivers = vec![ori3_model::DriverLine {
            a: [0.5, 0.0],
            b: [0.5, 1.0],
            target_angle_deg: 90.0,
        }];
        store
            .apply_seq(SeqOp::PushStep { step: pose })
            .expect("90度の立体姿勢を記録する");

        let before_edges = store.doc.cp.edges.len();
        let before = ori3_layers::replay(&store.doc, 1, 1.0);
        let z_span = before
            .frame
            .faces
            .iter()
            .flat_map(|face| face.polygon.iter().map(|point| point[2]))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), z| {
                (lo.min(z), hi.max(z))
            });
        assert!(
            z_span.1 - z_span.0 > 0.4,
            "90度の立体姿勢: z範囲={z_span:?}"
        );

        let grabbed = before
            .frame
            .faces
            .iter()
            .find(|face| {
                let (lo, hi) = face
                    .polygon
                    .iter()
                    .map(|point| point[2])
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), z| {
                        (lo.min(z), hi.max(z))
                    });
                hi - lo > 0.4
            })
            .expect("90度で立った面");
        let center_x = grabbed.polygon.iter().map(|point| point[0]).sum::<f64>()
            / grabbed.polygon.len() as f64;
        let center_z = grabbed.polygon.iter().map(|point| point[2]).sum::<f64>()
            / grabbed.polygon.len() as f64;
        let spatial = SpatialFoldSpec {
            from: [center_x, 0.25, center_z],
            to: [center_x, 0.5, center_z],
            grab_face: grabbed.face,
        };
        let grabbed_material = store
            .faces
            .iter()
            .find(|face| face.id == grabbed.face)
            .expect("当たり面の展開図を得る");
        let moving_vertex = grabbed_material
            .vertices
            .iter()
            .zip(&grabbed.polygon)
            .min_by(|(_, a), (_, b)| {
                let distance = |point: &&[f64; 3]| {
                    point
                        .iter()
                        .zip(spatial.from)
                        .map(|(value, target)| (value - target).powi(2))
                        .sum::<f64>()
                };
                distance(a).total_cmp(&distance(b))
            })
            .map(|(vertex, _)| *vertex)
            .expect("当たり面の材質頂点を得る");
        let fixed_vertex = store
            .doc
            .cp
            .vertices
            .iter()
            .max_by(|left, right| {
                left.pos[1]
                    .total_cmp(&right.pos[1])
                    .then_with(|| left.pos[0].total_cmp(&right.pos[0]))
            })
            .expect("動かさない側の材質頂点を得る")
            .id;
        let before_world =
            material_vertex_world_position(&store.faces, &before.frame, moving_vertex)
                .expect("折る前の材質頂点位置を得る");
        let before_fixed =
            material_vertex_world_position(&store.faces, &before.frame, fixed_vertex)
                .expect("折る前の固定頂点位置を得る");

        let preview = store
            .apply_seq_with_spatial(
                SeqOp::PreviewFoldThrough {
                    up_to: 1,
                    line: [[0.0, 0.375], [1.0, 0.375]],
                    keep_side_point: [0.5, 0.75],
                    target_layers: None,
                    target_pleat_count: None,
                    direction: ori3_model::FoldDirection::Up,
                    pose_before: None,
                },
                Some(spatial.clone()),
            )
            .expect("立体姿勢からの折りを変更せず下見できる");
        assert_eq!(
            preview.doc.cp.edges.len(),
            before_edges,
            "下見は展開図を変えない"
        );
        assert_eq!(preview.doc.sequence.len(), 1, "下見は手順を増やさない");

        let mut view = store
            .apply_seq_with_spatial(
                fold_op(1, [[0.0, 0.375], [1.0, 0.375]], [0.5, 0.75]),
                Some(spatial),
            )
            .expect("立体姿勢から続けた折りを受け付ける");
        attach_replay(&mut view);

        assert!(view.doc.cp.edges.len() > before_edges, "展開図の辺が増える");
        let after_world = material_vertex_world_position(
            &view.faces,
            view.frame.as_ref().expect("更新後の立体を返す"),
            moving_vertex,
        )
        .expect("折った後も同じ材質頂点を得る");
        let after_fixed = material_vertex_world_position(
            &view.faces,
            view.frame.as_ref().expect("更新後の立体を返す"),
            fixed_vertex,
        )
        .expect("折った後も固定頂点を得る");
        let distance = |a: [f64; 3], b: [f64; 3]| {
            a.iter()
                .zip(b)
                .map(|(left, right)| (left - right).powi(2))
                .sum::<f64>()
                .sqrt()
        };
        let displacement = distance(before_world, after_world);
        assert!(
            displacement > 1e-9,
            "同じ材質頂点{moving_vertex}が3Dで移動する: 距離={displacement:.17e}"
        );
        let relative_change =
            (distance(after_world, after_fixed) - distance(before_world, before_fixed)).abs();
        assert!(
            relative_change > 1e-9,
            "全体の置き直しではなく材質頂点間の相対形が変わる: 差={relative_change:.17e}"
        );
        assert_eq!(view.doc.sequence.len(), 2, "手順が1件から2件へ増える");
        assert!(
            view.warnings.iter().all(|warning| {
                !warning.contains("折り途中の状態では折れません")
                    && !warning.contains("折れる紙がありません")
            }),
            "拒否文言を返さない: {:?}",
            view.warnings
        );
        let frame = view.frame.as_ref().expect("更新後の立体を返す");
        let gap = ori3_rigid::max_seam_gap(&view.doc.cp, &view.faces, frame);
        assert!(gap < 1e-9, "共有辺の隙間={gap:.17e}");
    }

    /// 既存折り目を、領域を追加せず鏡映する汎用操作で0°まで開ける。
    #[test]
    fn flat_motion_opens_existing_crease_and_replays() {
        let mut store = square_store();
        store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .expect("半分に折る");
        let before = current_flat_state(&store);
        let folded = store
            .faces
            .iter()
            .find(|face| before.placements[&face.id].mirrored)
            .expect("折り返された層")
            .id;
        let ([x0, y0], [_, y1]) = outline_bbox(&store);
        let seam = [[x0, y0], [x0, y1]];
        let cp_before = store.doc.cp.clone();

        let view = store
            .apply_seq(SeqOp::FlatMotion {
                up_to: 1,
                parts: vec![ori3_model::MotionPart {
                    layers: vec![folded],
                    region: Vec::new(),
                    transform: ori3_model::MotionTransform::Reflect(vec![seam]),
                    turn: ori3_model::LayerTurn::Keep,
                    reverse_layers: None,
                }],
                kind: TechniqueKind::Simple,
            })
            .expect("既存折り目を開く");

        assert_eq!(view.doc.cp, cp_before, "開く操作はCPへ線を追加しない");
        assert_eq!(view.doc.sequence.len(), 2);
        assert!(
            view.doc.sequence[1]
                .drivers
                .iter()
                .any(|driver| driver.target_angle_deg.abs() <= ori3_model::EPS),
            "開いた折り目は0°で記録される"
        );
        let replayed = current_flat_state(&store);
        let first = replayed.placements[&store.faces[0].id];
        for face in &store.faces {
            assert!(
                replayed.placements[&face.id].approx_eq(&first, 1e-6),
                "面{}が開いた平面へ戻る",
                face.id
            );
        }
    }

    /// `Stay`は紙を動かさず、指定層の重なり順だけを変更して再生できる。
    #[test]
    fn flat_motion_restacks_without_moving_and_replays() {
        let mut store = square_store();
        store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .expect("半分に折る");
        let before = current_flat_state(&store);
        let bottom = before.order[0];
        let expected = vec![before.order[1], bottom];
        let cp_before = store.doc.cp.clone();

        let view = store
            .apply_seq(SeqOp::FlatMotion {
                up_to: 1,
                parts: vec![ori3_model::MotionPart {
                    layers: vec![bottom],
                    region: Vec::new(),
                    transform: ori3_model::MotionTransform::Stay,
                    turn: ori3_model::LayerTurn::Outside(ori3_model::FoldDirection::Up),
                    reverse_layers: None,
                }],
                kind: TechniqueKind::Pose,
            })
            .expect("層を最上面へ重ね替える");

        assert_eq!(
            view.doc.cp.edges.len(),
            cp_before.edges.len(),
            "重ね替えはCPへ線を追加しない"
        );
        assert_eq!(view.doc.cp.vertices.len(), cp_before.vertices.len());
        assert_eq!(view.doc.cp.next_edge_id, cp_before.next_edge_id);
        assert_eq!(view.doc.cp.next_vertex_id, cp_before.next_vertex_id);
        assert_ne!(
            view.doc.cp, cp_before,
            "新しい層順に合わせて既存折り目の山谷は更新される"
        );
        let after = current_flat_state(&store);
        assert_eq!(after.order, expected);
        for face in &store.faces {
            assert!(
                after.placements[&face.id].approx_eq(&before.placements[&face.id], 1e-9),
                "面{}の配置は動かない",
                face.id
            );
        }
    }

    /// `reverse_layers`は選んだ層だけを裏返し、他の層順を保つ。
    #[test]
    fn flat_motion_reverses_only_selected_layers_and_replays() {
        let mut store = square_store();
        store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .expect("1回目");
        store
            .apply_seq(fold_op(1, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]))
            .expect("2回目");
        let before = current_flat_state(&store);
        assert_eq!(before.order.len(), 4);
        let selected = vec![before.order[1], before.order[2]];
        let mut expected = before.order.clone();
        expected.swap(1, 2);

        store
            .apply_seq(SeqOp::FlatMotion {
                up_to: 2,
                parts: vec![ori3_model::MotionPart {
                    layers: selected,
                    region: Vec::new(),
                    transform: ori3_model::MotionTransform::Stay,
                    turn: ori3_model::LayerTurn::Keep,
                    reverse_layers: Some(true),
                }],
                kind: TechniqueKind::OpenSink,
            })
            .expect("選択層だけ山谷を反転する");

        let after = current_flat_state(&store);
        assert_eq!(after.order, expected);
        for face in &store.faces {
            assert!(
                after.placements[&face.id].approx_eq(&before.placements[&face.id], 1e-9),
                "面{}の配置は動かない",
                face.id
            );
        }
    }

    /// 複数partを1手として処理し、不在層は警告して有効な部分を続行する。
    #[test]
    fn flat_motion_runs_multiple_parts_and_continues_with_warning() {
        let mut store = square_store();
        store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .expect("半分に折る");
        let before = current_flat_state(&store);
        let folded = before
            .placements
            .iter()
            .find_map(|(&id, placement)| placement.mirrored.then_some(id))
            .expect("折り返された層");
        let stationary = before
            .placements
            .iter()
            .find_map(|(&id, placement)| (!placement.mirrored).then_some(id))
            .expect("動いていない層");
        let ([x0, y0], [_, y1]) = outline_bbox(&store);
        let seam = [[x0, y0], [x0, y1]];

        let view = store
            .apply_seq(SeqOp::FlatMotion {
                up_to: 1,
                parts: vec![
                    ori3_model::MotionPart {
                        layers: vec![999],
                        region: Vec::new(),
                        transform: ori3_model::MotionTransform::Stay,
                        turn: ori3_model::LayerTurn::Keep,
                        reverse_layers: None,
                    },
                    ori3_model::MotionPart {
                        layers: vec![folded],
                        region: Vec::new(),
                        transform: ori3_model::MotionTransform::Reflect(vec![seam]),
                        turn: ori3_model::LayerTurn::Keep,
                        reverse_layers: None,
                    },
                    ori3_model::MotionPart {
                        layers: vec![stationary],
                        region: Vec::new(),
                        transform: ori3_model::MotionTransform::Stay,
                        turn: ori3_model::LayerTurn::Outside(ori3_model::FoldDirection::Up),
                        reverse_layers: None,
                    },
                ],
                kind: TechniqueKind::Pose,
            })
            .expect("有効な複数部分は続行する");

        assert_eq!(view.doc.sequence.len(), 2, "複数partでも1手だけ増える");
        assert!(
            view.warnings
                .iter()
                .any(|warning| warning.contains("対象層 999")),
            "warnings={:?}",
            view.warnings
        );
        let replayed = current_flat_state(&store);
        let first = replayed.placements[&store.faces[0].id];
        assert!(
            store
                .faces
                .iter()
                .all(|face| replayed.placements[&face.id].approx_eq(&first, 1e-6))
        );
    }

    #[test]
    fn fold_through_keeps_alignment_metadata_on_the_new_step() {
        let mut store = square_store();
        let alignment = ori3_model::FoldAlignment {
            mode: ori3_model::AlignmentMode::PointPoint,
            picks: vec![
                ori3_model::AlignmentTarget::Point { p: [1.0, 0.0] },
                ori3_model::AlignmentTarget::Point { p: [0.0, 1.0] },
            ],
        };
        let view = store
            .apply_seq(SeqOp::FoldThrough {
                up_to: 0,
                line: [[0.0, 0.0], [1.0, 1.0]],
                keep_side_point: [0.0, 1.0],
                target_layers: None,
                target_pleat_count: None,
                direction: ori3_model::FoldDirection::Up,
                alignment: Some(alignment.clone()),
                accept_additional_crease: false,
                pose_before: None,
            })
            .expect("合わせ折りを適用する");

        assert_eq!(view.doc.sequence[0].alignment, Some(alignment));
    }

    #[test]
    fn preview_fold_through_is_non_destructive_and_acceptance_adds_the_guide() {
        let mut store = square_store();
        store
            .apply_seq(SeqOp::FoldThrough {
                up_to: 0,
                line: [[0.25, 0.0], [0.25, 1.0]],
                keep_side_point: [0.5, 0.5],
                target_layers: None,
                target_pleat_count: None,
                direction: ori3_model::FoldDirection::Up,
                alignment: None,
                accept_additional_crease: false,
                pose_before: None,
            })
            .expect("左端を折る");
        let before = store.doc.clone();
        let undo_count = store.undo_stack.len();
        let preview = store
            .apply_seq(SeqOp::PreviewFoldThrough {
                up_to: 1,
                line: [[0.7, 0.0], [0.7, 1.0]],
                keep_side_point: [0.6, 0.5],
                target_layers: None,
                target_pleat_count: None,
                direction: ori3_model::FoldDirection::Up,
                pose_before: None,
            })
            .expect("巻き込み候補を調べる");
        let proposal = preview.fold_through_proposal.expect("単一衝突縁の提案");
        assert!(
            proposal
                .folded_line
                .iter()
                .all(|point| (point[0] - 0.9).abs() < 1e-9)
        );
        assert_eq!(store.doc, before, "プレビューは文書を変更しない");
        assert_eq!(store.undo_stack.len(), undo_count, "undo履歴も増やさない");

        let accepted = store
            .apply_seq(SeqOp::FoldThrough {
                up_to: 1,
                line: [[0.7, 0.0], [0.7, 1.0]],
                keep_side_point: [0.6, 0.5],
                target_layers: None,
                target_pleat_count: None,
                direction: ori3_model::FoldDirection::Up,
                alignment: None,
                accept_additional_crease: true,
                pose_before: None,
            })
            .expect("提案を承諾して折る");
        assert!(
            accepted
                .warnings
                .iter()
                .all(|warning| warning != ori3_layers::FOLD_PENETRATION_WARNING),
            "承諾後は貫通警告が消える: {:?}",
            accepted.warnings
        );
        assert!(accepted.doc.cp.edges.iter().any(|edge| {
            if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
                return false;
            }
            let p0 = accepted
                .doc
                .cp
                .vertices
                .iter()
                .find(|vertex| vertex.id == edge.v0)
                .expect("端点")
                .pos;
            let p1 = accepted
                .doc
                .cp
                .vertices
                .iter()
                .find(|vertex| vertex.id == edge.v1)
                .expect("端点")
                .pos;
            ((p0[0] + p1[0]) * 0.5 - 0.9).abs() < 1e-9
        }));
    }

    #[test]
    fn fold_penetration_warning_follows_the_detection_setting() {
        for detect in [false, true] {
            let mut store = square_store();
            store.doc.display.penetration_prevention_enabled = detect;
            store
                .apply_seq(SeqOp::FoldThrough {
                    up_to: 0,
                    line: [[0.25, 0.0], [0.25, 1.0]],
                    keep_side_point: [0.5, 0.5],
                    target_layers: None,
                    target_pleat_count: None,
                    direction: ori3_model::FoldDirection::Up,
                    alignment: None,
                    accept_additional_crease: false,
                    pose_before: None,
                })
                .expect("左端を折る");
            let view = store
                .apply_seq(SeqOp::FoldThrough {
                    up_to: 1,
                    line: [[0.7, 0.0], [0.7, 1.0]],
                    keep_side_point: [0.6, 0.5],
                    target_layers: None,
                    target_pleat_count: None,
                    direction: ori3_model::FoldDirection::Up,
                    alignment: None,
                    accept_additional_crease: false,
                    pose_before: None,
                })
                .expect("衝突する単純折りも止めずに適用する");
            assert_eq!(
                view.warnings
                    .iter()
                    .any(|warning| warning == ori3_layers::FOLD_PENETRATION_WARNING),
                detect,
                "食い込み検出={detect}と画面へ出す貫通警告を一致させる"
            );
        }
    }

    /// 重なり順が紙の形と食い違っているときは、警告を出す前に正しい順序へ直す。
    ///
    /// 以前は警告を出すだけだったが、角度だけで折ると重なり順が決まらず、同じ平面の
    /// 面が完全に同じ位置へ描かれて裏面が見えたり貫通して見えた(2026-08-12に
    /// 利用者の画面で確認)。折り上がった形から順序を求めて直すようにした。
    #[test]
    fn flat_layer_order_contradiction_is_corrected_instead_of_warned() {
        let mut store = square_store();
        store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .unwrap();
        let faces = ori3_cp::extract_faces(&store.doc.cp);
        let mut frame = ori3_layers::replay(&store.doc, 1, 1.0).frame;
        assert!(!ori3_rigid::layer_order_conflicts(
            &store.doc.cp,
            &faces,
            &frame
        ));
        for face in &mut frame.faces {
            face.layer = 1 - face.layer;
        }
        let added = add_penetration_warning(&store.doc.cp, &faces, &mut frame, true);
        assert!(
            added.is_empty(),
            "直せる食い違いなのに警告を出した: {added:?}"
        );
        assert!(
            !ori3_rigid::layer_order_conflicts(&store.doc.cp, &faces, &frame),
            "重なり順が紙の形と食い違ったまま残った"
        );
        assert!(
            !frame
                .warnings
                .iter()
                .any(|warning| warning == ori3_layers::FOLD_PENETRATION_WARNING),
            "直したのに貫通の警告が残っている: {:?}",
            frame.warnings
        );
    }

    #[test]
    fn final_state_layer_heuristic_preserves_rigid_surface_authority() {
        let mut store = square_store();
        store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .unwrap();
        let faces = ori3_cp::extract_faces(&store.doc.cp);
        let mut frame = ori3_layers::replay(&store.doc, 1, 1.0).frame;

        let mut canonical_order = frame_surface_rank_order(&frame).expect("完全なrank順");
        canonical_order.reverse();
        ori3_rigid::stamp_surface_order(&mut frame, &canonical_order).unwrap();
        for face in &mut frame.faces {
            face.layer = 1 - face.layer;
        }
        assert!(
            ori3_rigid::layer_order_conflicts(&store.doc.cp, &faces, &frame),
            "旧heuristicが実際にlayerを直すfixture"
        );

        let _ = add_layer_order_warning_preserving_surface_authority(
            &store.doc.cp,
            &faces,
            &mut frame,
            &canonical_order,
        );

        assert!(
            !ori3_rigid::layer_order_conflicts(&store.doc.cp, &faces, &frame),
            "stack lift用layerの補助は残す"
        );
        assert_eq!(
            frame_surface_rank_order(&frame),
            Some(canonical_order),
            "最終状態heuristicがrigid canonical surface rankを消さない"
        );
    }

    #[test]
    fn display_contact_flags_separate_layer_warning_from_correction() {
        let mut store = square_store();
        store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .unwrap();
        let faces = ori3_cp::extract_faces(&store.doc.cp);
        let mut conflicted = ori3_layers::replay(&store.doc, 1, 1.0).frame;
        let canonical_order = frame_surface_rank_order(&conflicted).expect("完全なrank順");
        for face in &mut conflicted.faces {
            face.layer = 1 - face.layer;
        }
        assert!(
            ori3_rigid::layer_order_conflicts(&store.doc.cp, &faces, &conflicted),
            "設定の分離を測れる層矛盾fixture"
        );
        let face_state = |frame: &Frame3D| {
            frame
                .faces
                .iter()
                .map(|face| {
                    (
                        face.face,
                        face.polygon.clone(),
                        face.layer,
                        face.surface_rank,
                        face.mirrored,
                    )
                })
                .collect::<Vec<_>>()
        };
        let before = face_state(&conflicted);

        let mut detection_only = conflicted.clone();
        let warning = apply_layer_order_display_settings(
            &store.doc.cp,
            &faces,
            &mut detection_only,
            Some(&canonical_order),
            false,
            true,
        );
        assert_eq!(warning, Some(ori3_layers::FOLD_PENETRATION_WARNING));
        assert_eq!(
            face_state(&detection_only),
            before,
            "食い込み検出はlayer・surface_rank・形を変えない"
        );

        let mut prevention_only = conflicted.clone();
        let warning = apply_layer_order_display_settings(
            &store.doc.cp,
            &faces,
            &mut prevention_only,
            Some(&canonical_order),
            true,
            false,
        );
        assert_eq!(warning, None, "検出OFFでは貫通警告を足さない");
        assert!(
            !ori3_rigid::layer_order_conflicts(&store.doc.cp, &faces, &prevention_only),
            "重なり防止を明示したときだけlayerを補正する"
        );
        assert_eq!(
            frame_surface_rank_order(&prevention_only),
            Some(canonical_order.clone()),
            "明示した補正だけがcanonical surface authorityを刻む"
        );

        let mut disabled = conflicted;
        let warning = apply_layer_order_display_settings(
            &store.doc.cp,
            &faces,
            &mut disabled,
            Some(&canonical_order),
            false,
            false,
        );
        assert_eq!(warning, None);
        assert_eq!(face_state(&disabled), before, "両方OFFはframeを変えない");
    }

    #[test]
    fn frame_surface_rank_order_rejects_non_permutations() {
        let mut frame = Frame3D {
            faces: vec![
                Face3D {
                    face: 10,
                    polygon: Vec::new(),
                    layer: 0,
                    surface_rank: 1,
                    mirrored: false,
                },
                Face3D {
                    face: 20,
                    polygon: Vec::new(),
                    layer: 0,
                    surface_rank: 0,
                    mirrored: false,
                },
            ],
            warnings: Vec::new(),
        };
        assert_eq!(frame_surface_rank_order(&frame), Some(vec![20, 10]));

        frame.faces[0].surface_rank = 0;
        assert_eq!(frame_surface_rank_order(&frame), None, "rank重複を拒否");
        frame.faces[0].surface_rank = 1;
        frame.faces[1].face = 10;
        assert_eq!(frame_surface_rank_order(&frame), None, "face重複を拒否");
    }

    #[test]
    fn replay_physical_order_requires_geometry_proof_in_addition_to_complete_rank() {
        let mut store = square_store();
        store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .unwrap();
        let faces = ori3_cp::extract_faces(&store.doc.cp);
        let mut replayed = ori3_layers::replay_with_faces(&store.doc, &faces, 1, 1.0);
        assert!(
            replayed.surface_order_provenance.is_some(),
            "fixtureはcompleteな幾何導出を持つ"
        );
        assert!(frame_surface_rank_order(&replayed.frame).is_some());
        assert!(
            replay_surface_rank_order(&replayed).is_some(),
            "proofと完全rankがそろえば物理順を使える"
        );

        replayed.surface_order_provenance = None;
        assert!(
            frame_surface_rank_order(&replayed.frame).is_some(),
            "material seed自体は完全順列に見える"
        );
        assert_eq!(
            replay_surface_rank_order(&replayed),
            None,
            "完全順列だけをauthorityへ昇格しない"
        );
        let before_faces = replayed
            .frame
            .faces
            .iter()
            .map(|face| {
                (
                    face.face,
                    face.polygon.clone(),
                    face.layer,
                    face.surface_rank,
                    face.mirrored,
                )
            })
            .collect::<Vec<_>>();
        let before_warnings = replayed.frame.warnings.clone();
        let report = prevent_replay_overlap_if_authoritative(
            &store.doc.cp,
            &faces,
            &mut replayed,
            &ori3_soft::OverlapSettings {
                enabled: true,
                ..Default::default()
            },
        );
        assert!(
            report.is_none(),
            "proofなしではsequence/attach PBDを呼ばない"
        );
        let after_faces = replayed
            .frame
            .faces
            .iter()
            .map(|face| {
                (
                    face.face,
                    face.polygon.clone(),
                    face.layer,
                    face.surface_rank,
                    face.mirrored,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            after_faces, before_faces,
            "PBDをskipして論理layerを含むframeを変えない"
        );
        assert_eq!(replayed.frame.warnings, before_warnings);
    }

    #[test]
    fn attach_replay_keeps_saved_layer_but_uses_geometric_surface_rank() {
        let mut store = square_store();
        store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .unwrap();
        store.doc.sequence[0]
            .layer_order
            .as_mut()
            .expect("平坦折りは保存順を持つ")
            .reverse();
        let faces = ori3_cp::extract_faces(&store.doc.cp);
        let expected = ori3_layers::saved_layer_order_at(&store.doc, &faces, 1, 1.0)
            .expect("反転した保存順を解決できる");
        let mut geometric_doc = store.doc.clone();
        geometric_doc.sequence[0].layer_order = None;
        let expected_surface = frame_surface_rank_order(
            &ori3_layers::replay_with_faces(&geometric_doc, &faces, 1, 1.0).frame,
        )
        .expect("同じ幾何から完全なsurface rankを導出できる");
        let mut view = build_view(&store.doc, &store.step_creases, Vec::new());

        attach_replay(&mut view);

        let frame = view.frame.expect("手順を自動再生する");
        let mut actual_rank = frame
            .faces
            .iter()
            .map(|face| (face.surface_rank, face.face))
            .collect::<Vec<_>>();
        actual_rank.sort_unstable();
        let mut actual_layer = frame
            .faces
            .iter()
            .map(|face| (face.layer, face.face))
            .collect::<Vec<_>>();
        actual_layer.sort_unstable();
        assert_eq!(
            actual_rank
                .into_iter()
                .map(|(_, face)| face)
                .collect::<Vec<_>>(),
            expected_surface,
            "保存済みlayer_orderで幾何由来のowner rankを上書きしない"
        );
        assert_eq!(
            actual_layer
                .into_iter()
                .map(|(_, face)| face)
                .collect::<Vec<_>>(),
            expected,
            "保存済みlayer_orderは後続手順用layerとして維持する"
        );
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
            target_pleat_count: None,
            direction: ori3_model::FoldDirection::Up,
            alignment: None,
            accept_additional_crease: false,
            pose_before: None,
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

    fn stage3_nonflat_material_request(
        operation_type: &str,
        material_line: [[f64; 2]; 2],
        material_keep_side_point: [f64; 2],
    ) -> (DocumentStore, serde_json::Value) {
        let (store, mut pose_before) = one_pleat_square_store();
        pose_before.drivers[0].target_angle_deg = 90.0;
        let request = stage3_material_request_for_store(
            operation_type,
            0,
            material_line,
            material_keep_side_point,
            Some(pose_before),
        );
        (store, request)
    }

    fn stage3_material_request_for_store(
        operation_type: &str,
        up_to: usize,
        material_line: [[f64; 2]; 2],
        material_keep_side_point: [f64; 2],
        pose_before: Option<ori3_model::FoldPoseInput>,
    ) -> serde_json::Value {
        let mut request = serde_json::json!({
            "type": operation_type,
            "up_to": up_to,
            "material_line": material_line,
            "material_keep_side_point": material_keep_side_point,
        });
        if let Some(pose_before) = pose_before {
            request["pose_before"] = serde_json::json!(pose_before);
        }
        if operation_type == "CreaseOnlyTop" {
            request["direction"] = serde_json::json!("Up");
            request["alignment"] = serde_json::Value::Null;
        }
        assert!(
            request.get("target_pleat_count").is_none() && request.get("target_layers").is_none(),
            "折り目だけをK=0やFace列へ代用しない"
        );
        request
    }

    fn stage3_parallel_hinge_store(
        boundary_angle_deg: f64,
        use_saved_prefix: bool,
    ) -> (DocumentStore, Option<ori3_model::FoldPoseInput>, usize) {
        let mut store = square_store();
        let mut drivers = Vec::new();
        for (x, target_angle_deg) in [
            (0.0625, 0.0),
            (0.125, 0.0),
            (0.25, boundary_angle_deg),
            (0.5, -37.0),
        ] {
            let edges =
                ori3_cp::insert_segment(&mut store.doc.cp, [x, 0.0], [x, 1.0], EdgeKind::Valley);
            assert_eq!(edges.len(), 1, "parallel hinge x={x} is one material edge");
            drivers.push(ori3_model::FoldPoseDriver {
                edge_id: edges[0],
                target_angle_deg,
            });
        }
        store.faces = ori3_cp::extract_faces(&store.doc.cp);
        let pose_before = ori3_model::FoldPoseInput { drivers };
        if !use_saved_prefix {
            return (store, Some(pose_before), 0);
        }

        let mut pose_step = nonflat_pose_step_from_input(&store.doc.cp, &pose_before)
            .expect("prefix Pose is valid");
        pose_step.id = next_step_id(&store.doc, &store.step_creases);
        record_frontend_step(&mut store.step_creases, &pose_step);
        store.doc.sequence.push(pose_step);
        (store, None, 1)
    }

    fn stage3_apply_json(
        store: &mut DocumentStore,
        request: serde_json::Value,
    ) -> Result<DocumentView, String> {
        serde_json::from_value::<SeqOp>(request)
            .map_err(|error| error.to_string())
            .and_then(|operation| store.apply_seq(operation))
    }

    #[derive(Debug, PartialEq)]
    struct Stage3ApplyObservation {
        ok: bool,
        error: Option<String>,
        commit_count: usize,
        undo_depth: usize,
        sequence_len: usize,
        changed_after_apply: bool,
        one_undo_restored: bool,
    }

    /// 非平坦Poseと折り目stepは利用者の1操作として1回だけ確定し、Undoも1回。
    /// 製品variant実装前はunknown variantとなるが、compile errorではなく全観測値の差でREDにする。
    #[test]
    fn stage3_crease_only_top_commits_pose_and_crease_as_one_undo_operation() {
        let (plus_store, mut plus_pose) = one_pleat_square_store();
        plus_pose.drivers[0].target_angle_deg = 90.0;
        let (minus_store, mut minus_pose) = one_pleat_square_store();
        minus_pose.drivers[0].target_angle_deg = -90.0;
        let (split_store, split_pose, split_up_to) = stage3_parallel_hinge_store(90.0, false);
        let (prefix_store, prefix_pose, prefix_up_to) = stage3_parallel_hinge_store(-90.0, true);
        let (mut middle_store, middle_pose, middle_up_to) = stage3_parallel_hinge_store(90.0, true);
        let mut trailing_step = middle_store.doc.sequence[0].clone();
        trailing_step.id = next_step_id(&middle_store.doc, &middle_store.step_creases);
        trailing_step.drivers[2].target_angle_deg = 45.0;
        trailing_step.drivers[3].target_angle_deg = 25.0;
        trailing_step.note = "挿入位置より後で動く既存手順".to_string();
        record_frontend_step(&mut middle_store.step_creases, &trailing_step);
        middle_store.doc.sequence.push(trailing_step);

        for (name, mut store, pose_before, up_to, material_line, keep_point, expected_len) in [
            (
                "explicit +90 degrees",
                plus_store,
                Some(plus_pose),
                0,
                [[0.0, 0.25], [0.5, 0.25]],
                [0.25, 0.75],
                2,
            ),
            (
                "explicit -90 degrees",
                minus_store,
                Some(minus_pose),
                0,
                [[0.0, 0.25], [0.5, 0.25]],
                [0.25, 0.75],
                2,
            ),
            (
                "two explicit zero-degree subdivisions",
                split_store,
                split_pose,
                split_up_to,
                [[0.0, 0.25], [0.25, 0.25]],
                [0.125, 0.75],
                2,
            ),
            (
                "nonflat pose derived from saved prefix",
                prefix_store,
                prefix_pose,
                prefix_up_to,
                [[0.0, 0.25], [0.25, 0.25]],
                [0.125, 0.75],
                2,
            ),
            (
                "saved prefix with a later moving step",
                middle_store,
                middle_pose,
                middle_up_to,
                [[0.0, 0.25], [0.25, 0.25]],
                [0.125, 0.75],
                3,
            ),
        ] {
            eprintln!("stage3 cold replay sample: {name}");
            let request = stage3_material_request_for_store(
                "CreaseOnlyTop",
                up_to,
                material_line,
                keep_point,
                pose_before,
            );
            let original = store.doc.clone();
            let before = store.atomicity_probe_for_test();
            reset_commit_count_for_test();

            let applied = stage3_apply_json(&mut store, request);
            if name == "saved prefix with a later moving step" {
                let view = applied
                    .as_ref()
                    .expect("途中挿入後も後続手順まで再生できる");
                for expected in [45.0_f64, 25.0] {
                    assert!(
                        view.angles
                            .values()
                            .any(|actual| (*actual - expected).abs() <= 1.0e-9),
                        "後続手順の{expected}°が返却された実角へ反映される: {:?}",
                        view.angles
                    );
                }
            }
            let observation = Stage3ApplyObservation {
                ok: applied.is_ok(),
                error: applied.as_ref().err().cloned(),
                commit_count: commit_count_for_test(),
                undo_depth: store.undo_stack.len(),
                sequence_len: applied
                    .as_ref()
                    .map_or(store.doc.sequence.len(), |view| view.doc.sequence.len()),
                changed_after_apply: store.atomicity_probe_for_test() != before,
                one_undo_restored: if applied.is_ok() {
                    store
                        .undo()
                        .is_ok_and(|view| view.doc == original && store.undo_stack.is_empty())
                } else {
                    false
                },
            };

            assert_eq!(
                observation,
                Stage3ApplyObservation {
                    ok: true,
                    error: None,
                    commit_count: 1,
                    undo_depth: 1,
                    sequence_len: expected_len,
                    changed_after_apply: true,
                    one_undo_restored: true,
                },
                "{name}: signed Poseと0°折り目を1 commit・1 Undoにする"
            );
        }
    }

    #[derive(Debug, PartialEq)]
    struct Stage3FailureObservation {
        recognized_variant: bool,
        failed: bool,
        state_unchanged: bool,
        commit_count: usize,
    }

    /// 材料線が退化、または材料面の証人が紙外なら、推測せず全状態を保つ。
    #[test]
    fn stage3_invalid_material_requests_fail_atomically_without_falling_back_to_k_or_faces() {
        for (name, line, keep) in [
            (
                "degenerate material line",
                [[0.25, 0.25], [0.25, 0.25]],
                [0.25, 0.75],
            ),
            (
                "material keep point outside paper",
                [[0.0, 0.25], [0.5, 0.25]],
                [2.0, 2.0],
            ),
        ] {
            let (mut store, request) = stage3_nonflat_material_request("CreaseOnlyTop", line, keep);
            let before = store.atomicity_probe_for_test();
            reset_commit_count_for_test();

            let result = stage3_apply_json(&mut store, request);
            let error = result.as_ref().err().cloned().unwrap_or_default();
            let observation = Stage3FailureObservation {
                recognized_variant: !error.contains("unknown variant"),
                failed: result.is_err(),
                state_unchanged: store.atomicity_probe_for_test() == before,
                commit_count: commit_count_for_test(),
            };
            assert_eq!(
                observation,
                Stage3FailureObservation {
                    recognized_variant: true,
                    failed: true,
                    state_unchanged: true,
                    commit_count: 0,
                },
                "{name}"
            );
        }
    }

    #[derive(Debug, PartialEq)]
    struct Stage3QueryObservation {
        both_ok: bool,
        same_response: bool,
        status: Option<FoldTargetStatus>,
        state_unchanged: bool,
        commit_count: usize,
    }

    /// 材料座標の照会は保存を伴わず、同じ入力を2回呼んでも同じ結果になる。
    #[test]
    fn stage3_material_target_query_is_repeatable_and_non_destructive() {
        let (mut store, request) = stage3_nonflat_material_request(
            "PreviewFoldTargetsOnMaterial",
            [[0.0, 0.25], [0.5, 0.25]],
            [0.25, 0.75],
        );
        let before = store.atomicity_probe_for_test();
        reset_commit_count_for_test();

        let first = stage3_apply_json(&mut store, request.clone());
        let second = stage3_apply_json(&mut store, request);
        let first_bytes = first
            .as_ref()
            .ok()
            .and_then(|view| serde_json::to_vec(view).ok());
        let second_bytes = second
            .as_ref()
            .ok()
            .and_then(|view| serde_json::to_vec(view).ok());
        let observation = Stage3QueryObservation {
            both_ok: first.is_ok() && second.is_ok(),
            same_response: first_bytes.is_some() && first_bytes == second_bytes,
            status: first
                .as_ref()
                .ok()
                .and_then(|view| view.fold_target_info.as_ref())
                .map(|info| info.status),
            state_unchanged: store.atomicity_probe_for_test() == before,
            commit_count: commit_count_for_test(),
        };

        assert_eq!(
            observation,
            Stage3QueryObservation {
                both_ok: true,
                same_response: true,
                status: Some(FoldTargetStatus::CreaseOnlyTop),
                state_unchanged: true,
                commit_count: 0,
            },
            "材料座標照会はDocument・履歴・dirty・warmを変えない"
        );
    }
}
