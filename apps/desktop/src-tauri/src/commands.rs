//! IPCコマンド層: 各コマンドはDocumentStoreへ委譲するだけの薄いラッパー。
//! 全コマンドをpanic捕捉ラッパー`guard`で包み、アプリを落とさない(SYS-005)。
//! 全コマンドを`#[tauri::command(async)]`にしてスレッドプールで実行する
//! (同期fnはメインスレッド実行になり、validate等の計算でUIが引っかかるため)。
//!
//! 設計規約: ロック中に重い計算をしない(pose_solveなど他コマンドを待たせないため)。
//! ロック下ではstoreの状態更新と複製だけを行い、手順の再生や姿勢計算は
//! ロックを解放してから実行する(`view_command` / `pose_solve` / `sequence_replay`)。

use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::autosave;
use crate::store::{DocumentStore, DocumentView, add_penetration_warning, attach_replay};
use ori3_model::{CreasePattern, Driver, EditOp, Paper, SeqOp};
use ori3_propose::{Skeleton, generate, pack};

/// panicをErr文字列に変換する(SYS-005: アプリを落とさない)。
fn guard<T>(f: impl FnOnce() -> Result<T, String> + std::panic::UnwindSafe) -> Result<T, String> {
    std::panic::catch_unwind(f).unwrap_or_else(|payload| {
        let msg = if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "詳細不明".to_string()
        };
        Err(format!("内部エラーが発生しました: {msg}"))
    })
}

/// storeのロックを取る。過去のpanicで毒化されていても中身を取り出して続行する
/// (storeは「複製に適用→確定」方式のため、panic時も直前の整合状態を保っている)。
fn lock<'a>(state: &'a State<'_, Mutex<DocumentStore>>) -> MutexGuard<'a, DocumentStore> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// DocumentViewを返す操作の共通後処理: 手順を最新ステップまで自動再生して
/// 立体・飛ばした手順・警告をビューへ載せる(SEQ-004)。
///
/// 設計規約: ロック中に重い計算をしない。`f` の中で取ったロックは `f` を抜けた時点で
/// 解放されているので、再生(面400・10手順でrelease約23ms)はロックの外で走る。
fn view_command(f: impl FnOnce() -> Result<DocumentView, String>) -> Result<DocumentView, String> {
    let mut view = f()?; // ここでロックは解放済み
    attach_replay(&mut view);
    Ok(view)
}

#[tauri::command(async)]
pub fn document_new(
    state: State<'_, Mutex<DocumentStore>>,
    paper: Paper,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| {
        view_command(|| lock(&state).new_document(paper))
    }))
}

#[tauri::command(async)]
pub fn document_open(
    state: State<'_, Mutex<DocumentStore>>,
    path: String,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| {
        view_command(|| lock(&state).open(Path::new(&path)))
    }))
}

#[tauri::command(async)]
pub fn document_save(
    app: tauri::AppHandle,
    state: State<'_, Mutex<DocumentStore>>,
    path: Option<String>,
) -> Result<(), String> {
    guard(AssertUnwindSafe(|| {
        lock(&state).save(path.as_deref().map(Path::new))?;
        // 保存できた内容は自動保存から復元する必要がない(SYS-003)
        if let Ok(dir) = autosave::app_data_dir(&app) {
            autosave::discard(&dir);
        }
        Ok(())
    }))
}

/// 前回の異常終了で残った自動保存があるか調べる(SYS-003)。
/// あればその情報を返し、フロントが復旧ダイアログで復元するか尋ねる。
#[tauri::command(async)]
pub fn recovery_check(app: tauri::AppHandle) -> Result<Option<autosave::RecoveryInfo>, String> {
    guard(AssertUnwindSafe(|| {
        let dir = autosave::app_data_dir(&app)?;
        Ok(autosave::check(&dir))
    }))
}

/// 復旧ダイアログの答えを実行する。`accept`なら自動保存の内容を現在の作品にし、
/// そうでなければ自動保存ファイルを消す(以後は提案しない)。
///
/// 設計規約: 読み込みとJSON解釈はロックの外、状態の入れ替えだけロック下で行う。
#[tauri::command(async)]
pub fn recovery_restore(
    app: tauri::AppHandle,
    state: State<'_, Mutex<DocumentStore>>,
    accept: bool,
) -> Result<Option<DocumentView>, String> {
    guard(AssertUnwindSafe(|| {
        let dir = autosave::app_data_dir(&app)?;
        if !accept {
            autosave::discard(&dir);
            return Ok(None);
        }
        let Some(mut view) = autosave::restore(&state, &dir)? else {
            return Ok(None);
        };
        attach_replay(&mut view); // 重い再生はロック解放後(view_commandと同じ規約)
        Ok(Some(view))
    }))
}

#[tauri::command(async)]
pub fn edit_apply(
    state: State<'_, Mutex<DocumentStore>>,
    op: EditOp,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| {
        view_command(|| lock(&state).apply_edit(op))
    }))
}

#[tauri::command(async)]
pub fn edit_undo(state: State<'_, Mutex<DocumentStore>>) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| view_command(|| lock(&state).undo())))
}

#[tauri::command(async)]
pub fn edit_redo(state: State<'_, Mutex<DocumentStore>>) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| view_command(|| lock(&state).redo())))
}

#[tauri::command(async)]
pub fn sequence_apply(
    state: State<'_, Mutex<DocumentStore>>,
    op: SeqOp,
) -> Result<DocumentView, String> {
    guard(AssertUnwindSafe(|| {
        view_command(|| lock(&state).apply_seq(op))
    }))
}

/// 折り角度の追従計算(Task 1-8)。driver角を固定して残りのヒンジ角を解き、
/// 3D表示用フレームを返す。前回解はstoreが保持し、warm startとして使う。
/// facesは編集時に導出済みのstoreのキャッシュを流用する(extract_faces再実行なし)。
///
/// 設計規約: ロック中に重い計算をしない(将来の自動保存スレッドとの共存のため)。
/// ロック下ではCP・faces・前回解の複製だけを行って即ロックを解放し、
/// solveはロックの外で実行し、結果の角度だけを短いロックで書き戻す。
#[tauri::command(async)]
pub fn pose_solve(
    state: State<'_, Mutex<DocumentStore>>,
    drivers: Vec<Driver>,
) -> Result<ori3_rigid::SolveResult, String> {
    guard(AssertUnwindSafe(|| {
        let (cp, faces, warm) = lock(&state).pose_inputs(); // 複製のみ、即ロック解放
        let mut result = ori3_rigid::solve(&cp, &faces, &drivers, warm.as_ref());
        add_penetration_warning(&mut result.frame); // 紙のめり込み(SIM-007)
        lock(&state).store_pose_angles(result.angles.clone()); // 短いロックで書き戻し
        Ok(result)
    }))
}

/// 手順の再生(Task 2-3)。展開図と手順列から `up_to` ステップ目(補間係数 `t`)の
/// 立体を求め直す。3D状態は保存しないので、展開図を編集した後でも再生できる。
///
/// 設計規約: ロック中に重い計算をしない。ロック下ではDocumentと導出済みfacesの複製
/// だけを行って即ロックを解放し、再生はロックの外で実行する(結果の書き戻しは不要)。
#[tauri::command(async)]
pub fn sequence_replay(
    state: State<'_, Mutex<DocumentStore>>,
    up_to: usize,
    t: f64,
) -> Result<ori3_layers::ReplayResult, String> {
    guard(AssertUnwindSafe(|| {
        let (doc, faces) = lock(&state).replay_inputs(); // 複製のみ、即ロック解放
        let mut result = ori3_layers::replay_with_faces(&doc, &faces, up_to, t);
        // 折る途中(t<1)は立体になるので、紙が食い込んでいないかを見る(SIM-007)。
        // 画面のバッジは ReplayResult.warnings を見るので両方へ同じ文言を載せる
        if add_penetration_warning(&mut result.frame) {
            result
                .warnings
                .push(ori3_rigid::PENETRATION_WARNING.to_string());
        }
        Ok(result)
    }))
}

/// 円充填のやり直し回数。多いほど良い配置に当たりやすいが時間もかかる。
/// 8回で12本の角でも数百ms以内に収まる(packing.rsの調整値と揃えてある)。
const PACK_STARTS: usize = 8;

/// 提案された展開図1つ分。`scale` は骨格の長さ1あたりが紙の何割になるか(大きいほど
/// 完成品が大きい)、`violations` は平坦に折りにくい頂点の数(0が理想)。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProposalCandidate {
    pub cp: CreasePattern,
    pub scale: f64,
    pub violations: usize,
    pub warnings: Vec<String>,
}

/// 骨格から展開図の候補を作る(PRO-001/PRO-005、Task 3-4)。
/// 乱数の初期値違いで最大4つの候補を返し、どれを使うかは利用者が選ぶ。
///
/// 設計規約: ロック中に重い計算をしない。この処理は作品の状態を一切見ないので
/// storeのロックそのものを取らない(充填中も他のコマンドが普通に動く)。
#[tauri::command(async)]
pub fn proposal_generate(
    skeleton: Skeleton,
    paper: Paper,
    seed: u64,
) -> Result<Vec<ProposalCandidate>, String> {
    guard(AssertUnwindSafe(move || {
        skeleton.validate()?;
        let long = paper.width_mm.max(paper.height_mm);
        if !(long > 0.0 && long.is_finite()) {
            return Err("紙のサイズは正の値にしてください".to_string());
        }
        // CPの座標系は「紙の長辺=1.0」正規化(ori3_model::Document::new と同じ)
        let (w, h) = (paper.width_mm / long, paper.height_mm / long);
        let packings = pack(&skeleton, w, h, seed, PACK_STARTS);
        let mut out = Vec::new();
        let mut last_err = None;
        for p in &packings {
            match generate(&skeleton, p, w, h) {
                Ok(r) => out.push(ProposalCandidate {
                    cp: r.cp,
                    scale: p.scale,
                    violations: r.violations,
                    warnings: r.warnings,
                }),
                Err(e) => last_err = Some(e),
            }
        }
        if out.is_empty() {
            return Err(last_err.unwrap_or_else(|| {
                "この骨格を紙の上に配置できませんでした(角を減らすか短くしてみてください)"
                    .to_string()
            }));
        }
        Ok(out)
    }))
}

#[cfg(test)]
mod tests {
    use super::{guard, proposal_generate};
    use ori3_model::Paper;
    use ori3_propose::{Skeleton, SkeletonNode};
    use std::panic::AssertUnwindSafe;

    /// 根1つ+`leaves`本の角(すべて同じ長さ・太さ)の骨格。
    fn star(leaves: u32) -> Skeleton {
        let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
        nodes.extend((1..=leaves).map(|i| SkeletonNode::new(i, Some(0), 1.0)));
        Skeleton { nodes }
    }

    const A4ISH: Paper = Paper {
        width_mm: 150.0,
        height_mm: 150.0,
    };

    #[test]
    fn proposal_generate_returns_candidates() {
        let out = proposal_generate(star(4), A4ISH, 7).expect("候補が返るはず");
        assert!(!out.is_empty() && out.len() <= 4, "件数={}", out.len());
        for c in &out {
            assert!(c.scale > 0.0, "scale={}", c.scale);
            // 輪郭4辺だけ、ということはない(折り線が引かれている)
            assert!(c.cp.edges.len() > 4, "辺数={}", c.cp.edges.len());
        }
    }

    #[test]
    fn proposal_generate_is_deterministic() {
        let a = proposal_generate(star(3), A4ISH, 42).unwrap();
        let b = proposal_generate(star(3), A4ISH, 42).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn proposal_generate_rejects_broken_skeleton() {
        // 角が1本もない(根だけ)骨格は骨格側の検査で日本語のErrになる
        let only_root = Skeleton {
            nodes: vec![SkeletonNode::new(0, None, 0.0)],
        };
        let err = proposal_generate(only_root, A4ISH, 1).unwrap_err();
        assert!(err.contains("角"), "err={err}");

        // 紙のサイズが0以下でもErr(パニックにしない)
        let bad_paper = Paper {
            width_mm: 0.0,
            height_mm: 0.0,
        };
        assert!(proposal_generate(star(2), bad_paper, 1).is_err());
    }

    #[test]
    fn guard_converts_panic_to_err() {
        let r: Result<(), String> = guard(AssertUnwindSafe(|| panic!("爆発した")));
        let err = r.unwrap_err();
        assert!(err.contains("内部エラー"), "err={err}");
        assert!(err.contains("爆発した"), "err={err}");
    }

    #[test]
    fn guard_passes_through_ok_and_err() {
        assert_eq!(guard(AssertUnwindSafe(|| Ok(42))), Ok(42));
        assert_eq!(
            guard(AssertUnwindSafe(|| Err::<(), _>("だめ".to_string()))),
            Err("だめ".to_string())
        );
    }
}
