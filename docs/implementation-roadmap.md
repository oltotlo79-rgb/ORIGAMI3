# ORIGAMI3 実装ロードマップ

> **実装エージェント(Codex等)へ:** 本書はタスク単位のチェックボックス(`- [ ]`)で進捗を管理する。タスクは上から順に実施する。各タスクは「テストを書く → 失敗を確認 → 実装 → 成功を確認 → コミット → プッシュ」のTDDサイクルで進めること。

**Goal:** 展開図を描きながら3Dで1折りずつ折り紙を折り、骨格指定から展開図を自動提案できるデスクトップアプリ(要件は `docs/requirements-definition.md`)。

**Architecture:** Tauri 2ホスト + React/TypeScriptフロント + Rust計算コア(cargo workspace、`ori3-soft`を含む8クレート)。折りエンジンは「剛体折りソルバー(表示)+ 平坦状態の層モデル(記録)」のハイブリッド。3D状態は保存せず「展開図 + 折り手順」から常に再生する。M3強化の折り手順計画は配置案P1として既存の`ori3-propose`内に置き、新クレートは作らない。

**Tech Stack:** Tauri 2 / React / TypeScript / Vite / Zustand / Three.js / Rust (glam, serde, thiserror, rand, resvg, svg2pdf)

---

## 0. 作業規律(全タスク共通)

### 0.1 Git

- リモート: `https://github.com/oltotlo79-rgb/ORIGAMI3.git`、ブランチ `main`
- **タスク完了ごとにコミットし、必ず `git push origin main` する**(ユーザー指示。プッシュ忘れ禁止)
- **コミットメッセージは日本語で、内容が具体的に分かるように書く(ユーザー指示)**
  - 1行目: 何を追加・変更したかの要約(体言止め可)
  - 本文: 「これで何ができるようになったか」「主な変更点」を箇条書きで2〜5行
  - 専門用語(クレート名・型名・アルゴリズム名・英語の略語)は極力使わない。避けられない場合は日本語の言い換えを添える
  - 例:

```
畳んだ紙に線を引いてまとめて折る操作を追加

- 折り紙を畳んだ状態の上に折り線を引くと、重なっている紙をまとめて折れるようになった
- 折った結果の折り線は展開図にも自動で書き加えられる
- 紙の重なり順も折った後の正しい順番に更新される
```

### 0.2 検査(タスク完了条件に常に含む)

```powershell
# 5つの検査をまとめて実行する。個別のコマンドを直に打たず、必ずこれを使う。
./scripts/check.ps1
```

**素の `cargo test --workspace` を直に打たないこと。** `CLAUDE.md` §10.6 の #18〜#20 は、
最適化なしでは現実的な時間で終わらない(いちばん重いもので約7.5時間)。
`scripts/check.ps1` はその3件だけを `--skip` し、残りはいままでどおり全部走らせる。
3件は CI の `performance` ジョブと §10.6 の表が**最適化ありで**走らせるので、
**どの検査も消えていない**。

`scripts/check.ps1` が実行する5つは次のとおり(内訳は同ファイルの先頭コメント)。

```text
cargo test --workspace -- --skip <§10.6 の #18〜#20 の3件>
cargo clippy --workspace --all-targets -- -D warnings
cd apps/desktop; npm run build; npm run lint; npm run test; cd ../..
```

**検査が通らない状態でコミットしない。**

### 0.3 規律(要件定義書§2より。違反する実装はレビューで差し戻し)

- f64 + 明示的ε。厳密有理数演算・証明機構を書かない
- 失敗時は「止めずに警告」。ユーザー操作をブロックするゲートを作らない
- Tauriコマンドは「コマンド1個+操作enum」への集約を優先する。個数上限は2026-08-08に撤廃(追加時は要件定義書§9.3の表を更新)
- 常設UI区画は4つ固定。固定パネル・常設セクションを追加しない
- コード全体・個別ファイルの行数上限は2026-08-08に撤廃。分割は読みやすさと責務の明確さのための任意の改善であり、合否条件にしない。ただし、新しく足すコードを1つのファイルへ無計画に積み増さない
- フロント状態はZustandストア1本。コンポーネント`useState`はホバー等の表示専用に限る

### 0.4 参考資料(読み取り専用)

`C:\Users\oltot\Documents\git-projects\ORIGAMI2` に前身の実装がある。**コードはコピーしない**(厳密有理数前提で設計が異なるため)。アルゴリズムの参考として以下のみ参照してよい:

- `crates/ori-topology/` — 面抽出・局所平坦折り判定(前川・川崎)の考え方
- `crates/ori-domain/src/beginner_generator.rs` — 骨格→円充填→展開図生成の手順
- `crates/ori-instructions/` — 折り図の記号・レイアウトの考え方

## 1. 最終ファイル構成マップ

```
ORIGAMI3/
├─ Cargo.toml                      # workspace定義
├─ scripts/check.ps1               # 一括検査
├─ docs/                           # 本書・要件定義書・進捗メモのみ
├─ crates/
│  ├─ ori3-model/src/lib.rs        # データ型 + serde + ε定数(依存なし)
│  ├─ ori3-geometry/src/
│  │   ├─ lib.rs                   # re-export
│  │   ├─ primitives.rs            # 交差・射影・鏡映
│  │   └─ isometry.rs              # 2D等長変換(回転+並進+鏡映)
│  ├─ ori3-cp/src/
│  │   ├─ lib.rs
│  │   ├─ graph.rs                 # 線分挿入(交点自動分割)・隣接構造
│  │   ├─ faces.rs                 # 面抽出(half-edge・最左回り)
│  │   ├─ snap.rs                  # グリッド/頂点/線上/交点スナップ
│  │   ├─ construct.rs             # 作図補助(二等分線・垂線・n等分・22.5°)
│  │   └─ flatfold.rs              # 前川・川崎の局所判定
│  ├─ ori3-rigid/src/
│  │   ├─ lib.rs
│  │   ├─ tree.rs                  # 面隣接グラフ・全域木・角度伝播
│  │   └─ solver.rs                # ループ閉包Gauss-Newton
│  ├─ ori3-layers/src/
│  │   ├─ lib.rs
│  │   ├─ flat_state.rs            # 平坦状態(面配置+層順序)
│  │   ├─ fold_through.rs          # 折り操作プリミティブ(重ね折り)
│  │   └─ techniques.rs            # 技法マクロ(中割り・かぶせ・花弁・段・つぶし・沈め・ひだ寄せ・ねじり)
│  ├─ ori3-soft/src/
│  │   └─ lib.rs                   # たわみ・膨らみの後処理(SIM-012〜015)
│  ├─ ori3-propose/src/
│  │   ├─ lib.rs
│  │   ├─ skeleton.rs              # 骨格(木構造)モデル
│  │   ├─ packing.rs               # 円・川充填の数値最適化
│  │   └─ generate.rs              # 充填→分子→展開図(P1の手順計画もこのクレート内。方式別ファイル名は作業18後に決定)
│  └─ ori3-export/src/
│      ├─ lib.rs
│      ├─ cp_svg.rs                # 展開図SVG
│      ├─ cp_png.rs                # 展開図PNG(resvgでラスタライズ)
│      ├─ diagram.rs               # 折り図(ステップ投影+矢印記号)
│      └─ pdf.rs                   # svg2pdfでA4組版
└─ apps/desktop/
   ├─ src-tauri/src/
   │   ├─ lib.rs                   # コマンド登録のみ(薄く保つ)
   │   ├─ store.rs                 # DocumentStore(現ドキュメント+Undo/Redo)
   │   ├─ commands.rs              # Tauriコマンド18個(全てstore/クレートへ委譲)
   │   └─ autosave.rs              # 自動保存+復旧
   └─ src/
       ├─ main.tsx / App.tsx       # 4区画レイアウト
       ├─ store/appStore.ts        # Zustandストア(唯一の状態置き場)
       ├─ ipc/client.ts            # invokeラッパー18関数(1関数=1コマンド)
       ├─ components/
       │   ├─ ToolRail.tsx         # ツールレール(関連する道具は整理して表示)
       │   ├─ CpEditor/            # 2D展開図エディタ(Canvas 2D)
       │   ├─ Viewer3D/            # Three.js 3Dビュー + 3D上の折り線描画
       │   ├─ Timeline.tsx         # 手順タイムライン
       │   ├─ ContextPanel.tsx     # コンテキストパネル(選択対象で切替)
       │   └─ dialogs/             # 新規作成/提案ウィザード/書き出し/復旧など。追加時は既存画面との統合を先に検討
       └─ lib/                     # 型定義・ユーティリティ
```

## 2. 共有データ型(ori3-model、全タスクの前提)

以降の全タスクはこの型を正とする。変更する場合は本書と要件定義書を先に改訂する。

```rust
pub const SCHEMA_VERSION: u32 = 1;
/// 幾何計算の許容誤差。座標は「紙の長辺 = 1.0」に正規化した系で扱い、
/// mm値は入出力時のみ使用する。
pub const EPS: f64 = 1e-9;

pub type VertexId = u32;
pub type EdgeId = u32;
pub type FaceId = u32;   // 面IDは面抽出のたびに再採番される導出値。永続化に使わない
pub type StepId = u32;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Paper { pub width_mm: f64, pub height_mm: f64 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EdgeKind { Border, Mountain, Valley, Aux }

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Vertex { pub id: VertexId, pub pos: [f64; 2] }

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Edge { pub id: EdgeId, pub v0: VertexId, pub v1: VertexId, pub kind: EdgeKind }

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CreasePattern {
    pub vertices: Vec<Vertex>,
    pub edges: Vec<Edge>,
    pub next_vertex_id: VertexId,
    pub next_edge_id: EdgeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TechniqueKind {
    Simple, Pleat, InsideReverse, OutsideReverse, Petal,
    Squash, OpenSink, Swivel, Twist, Pose,
}

/// ヒンジ角: 0=平ら, +180=完全な山折り, -180=完全な谷折り(度)
/// 注: EdgeId参照のDriverは pose_solve(スライダー操作)専用の一時指定。
/// 手順の永続化には使わない(辺IDは後続の折りの分割で無効化されるため)。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Driver { pub hinge: EdgeId, pub target_angle_deg: f64 }

/// 手順永続化用のdriver: 折り線をCP座標の線分で指定する。
/// 再生時は「この線分上に乗る折り辺すべて」(同一直線上・区間内・EPS許容)を
/// 対象角へ駆動する。後続の折りで辺が分割されても全断片が駆動されるため
/// ID無効化に耐える(層順序の代表点方式と同じ思想)。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DriverLine { pub a: [f64; 2], pub b: [f64; 2], pub target_angle_deg: f64 }

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FoldStep {
    pub id: StepId,
    pub kind: TechniqueKind,
    pub drivers: Vec<DriverLine>,
    /// 平坦到達時の層順序(下→上)。面IDは不安定なので、
    /// 各面を「CP座標系におけるその面の内部代表点」で参照する。
    /// 平坦にならないステップ(Pose)ではNone。
    pub layer_order: Option<Vec<[f64; 2]>>,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DisplaySettings {
    pub front_color: [u8; 3],
    pub back_color: [u8; 3],
    pub grid_divisions: u32,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Document {
    pub schema_version: u32,
    pub paper: Paper,
    pub cp: CreasePattern,
    pub sequence: Vec<FoldStep>,
    pub display: DisplaySettings,
}

/// edit_apply コマンドの操作enum(これ以外の編集操作を追加しない)
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum EditOp {
    AddSegment { a: [f64; 2], b: [f64; 2], kind: EdgeKind },
    RemoveEdges { ids: Vec<EdgeId> },
    SetEdgeKind { ids: Vec<EdgeId>, kind: EdgeKind },
    MoveVertex { id: VertexId, to: [f64; 2] },
    SetPaper { paper: Paper },
    /// 提案ウィザードの流し込み用
    ReplaceCreasePattern { cp: CreasePattern },
    /// 紙の色・方眼の分割数(作品ごとの設定。undo/redo対象。分割数2〜1024外は丸めて警告)
    SetDisplay { display: DisplaySettings },
}

/// sequence_apply コマンドの操作enum
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum SeqOp {
    PushStep { step: FoldStep },
    InsertStep { index: usize, step: FoldStep },
    RemoveStep { id: StepId },
    UpdateStep { step: FoldStep },
}

/// 3D表示用フレーム(IPC戻り値)
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Face3D { pub face: FaceId, pub polygon: Vec<[f64; 3]>, pub layer: u32 }

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Frame3D { pub faces: Vec<Face3D>, pub warnings: Vec<String> }
```

### IPCコマンド一覧(現在18個。個数上限なし)

`document_new / document_open / document_save / edit_apply / edit_apply_batch / edit_undo / edit_redo / sequence_apply / sequence_replay / pose_solve / fold_all_preview / recovery_check / recovery_restore / proposal_generate / proposal_progress / proposal_control / proposal_apply / document_export`

18個であること自体は違反ではない。追加時は既存コマンドの操作enumへ集約できないかを先に検討し、実装登録と本一覧を同時に更新する。

全コマンドの戻り値は `Result<T, String>` とし、内部panicは `std::panic::catch_unwind` で捕捉してErrに変換する(SYS-005)。

---

## M0: プロジェクト基盤

### Task 0-1: cargo workspace とクレート雛形

**Files:** `Cargo.toml`, `crates/ori3-{model,geometry,cp,rigid,layers,propose,export}/Cargo.toml`, 各 `src/lib.rs`, `.gitignore`

- [x] ルート`Cargo.toml`にworkspace(members = crates/* と apps/desktop/src-tauri)を定義 — [証拠:M0.T0-1.C01](#roadmap-evidence-m0-t0-1-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M0.T0-1.C01 evidence=CHECK.CURRENT-STATUS.WORKSPACE-MEMBERS -->
- [x] 各クレートを`cargo new --lib`で作成。依存: model(なし) / geometry(model, glam) / cp(geometry) / rigid(cp) / layers(cp) / propose(cp, rand) / export(layers, rigid, resvg, svg2pdf) — [証拠:M0.T0-1.C02](#roadmap-evidence-m0-t0-1-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M0.T0-1.C02 evidence=MANUAL.M0.T0-1.C02.CRATE-SCAFFOLD -->
- [x] 共通依存(serde, serde_json, thiserror, glam)はworkspace.dependenciesで一元管理。バージョンは最新安定版を選び`Cargo.lock`で固定 — [証拠:M0.T0-1.C03](#roadmap-evidence-m0-t0-1-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M0.T0-1.C03 evidence=MANUAL.M0.T0-1.C03.DEPENDENCY-BASELINE -->
- [x] `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings` が通ることを確認 — [証拠:M0.T0-1.C04](#roadmap-evidence-m0-t0-1-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M0.T0-1.C04 evidence=CHECK.LOCAL.RUST-WORKSPACE-TEST,CHECK.LOCAL.RUST-WORKSPACE-CLIPPY -->
- [x] コミット `計算部品を置くためのフォルダ構成と空の部品一式を作成` → プッシュ — [証拠:M0.T0-1.C05](#roadmap-evidence-m0-t0-1-c05) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M0.T0-1.C05 evidence=MANUAL.M0.T0-1.C05.COMMIT-PUSH -->

### Task 0-2: Tauriアプリ雛形

**Files:** `apps/desktop/` 一式(Tauri 2 + React + TS + Viteテンプレート)

- [x] `npm create tauri-app@latest`(react-tsテンプレート)で`apps/desktop`を作成し、`three` `@types/three` `zustand` を追加 — [証拠:M0.T0-2.C01](#roadmap-evidence-m0-t0-2-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M0.T0-2.C01 evidence=MANUAL.M0.T0-2.C01.TAURI-SCAFFOLD -->
- [x] `src-tauri/Cargo.toml` をworkspaceメンバーに追加し、空の`greet`系サンプルコマンドを削除 — [証拠:M0.T0-2.C02](#roadmap-evidence-m0-t0-2-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M0.T0-2.C02 evidence=CHECK.CURRENT-STATUS.WORKSPACE-MEMBERS,CHECK.CURRENT-STATUS.TAURI-COMMANDS -->
- [x] `npm run tauri dev` でウィンドウが起動することを確認(タイトル: ORIGAMI3) — [証拠:M0.T0-2.C03](#roadmap-evidence-m0-t0-2-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M0.T0-2.C03 evidence=MANUAL.M0.T0-2.C03.TAURI-LAUNCH-TITLE -->
- [x] コミット `アプリの画面が起動する最小の土台を作成` → プッシュ — [証拠:M0.T0-2.C04](#roadmap-evidence-m0-t0-2-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M0.T0-2.C04 evidence=MANUAL.M0.T0-2.C04.COMMIT-PUSH -->

### Task 0-3: 検査スクリプト

**Files:** `scripts/check.ps1`

- [x] §0.2の5検査を順に実行し、いずれか失敗で非0終了するスクリプトを作成。手動実行で成功を確認 — [証拠:M0.T0-3.C01](#roadmap-evidence-m0-t0-3-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M0.T0-3.C01 evidence=CHECK.LOCAL.ALL-FIVE,MANUAL.M0.T0-3.C01.ALL-FIVE-RUN -->
- [x] コミット `全ての自動チェックを一度に実行できる仕組みを追加` → プッシュ — [証拠:M0.T0-3.C02](#roadmap-evidence-m0-t0-3-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M0.T0-3.C02 evidence=MANUAL.M0.T0-3.C02.COMMIT-PUSH -->

### M0 証拠リンク

この表は7-D1の正本である。自動検査は実際の検査名、手動項目は一意な受入IDと確認条件を記し、履歴を現在値として誤判定しない。

<!-- ORIGAMI3-ROADMAP-EVIDENCE:BEGIN scope=M0 schema=1 -->
| link ID | evidence | authoritative source | acceptance | progress | unresolved |
|---|---|---|---|---|---|
| <a id="roadmap-evidence-m0-t0-1-c01"></a>`M0.T0-1.C01` | 自動 `CHECK.CURRENT-STATUS.WORKSPACE-MEMBERS` | `file:Cargo.toml` :: `section:[workspace]/field:members`<br>`file:scripts/generate-current-status.ps1` :: `function:Get-WorkspaceMembers` | collectorが追跡済みworkspace memberを列挙し、重複・欠損・未追跡member manifestをsource errorにする。 | historical-evolution — checkboxの`crates/*`表現に対し、Task 0-1の明示crate一覧へ後続のdesktop / `ori3-soft`を加えた現在manifestも明示member一覧である。 | none |
| <a id="roadmap-evidence-m0-t0-1-c02"></a>`M0.T0-1.C02` | 手動 `MANUAL.M0.T0-1.C02.CRATE-SCAFFOLD` | `git:19bc6ebd0009e62a15efc69f5eb17a7bdcfe6dbd` :: `tree:Cargo.toml+seven crate Cargo.toml/src/lib.rs`<br>`file:crates/ori3-export/Cargo.toml` :: `section:[dependencies]/fields:resvg,svg2pdf`<br>`file:docs/progress.md` :: `heading:## 2026-08-05 - Task 0-1 - 計算部品のフォルダ構成と空の部品一式を作成` | 初期commitでcrate雛形を照合し、M4で後追いした`resvg` / `svg2pdf`は現行export manifestで充足を確認する。`cargo new`の実行方法そのものは成果treeから推定しない。 | historical-evolution — 現在は`ori3-soft`を加えた計算crate集合である。さらに`resvg`・`svg2pdf`はTask 0-1当時未追加で、後続M4まで保留した履歴が正しい。 | none |
| <a id="roadmap-evidence-m0-t0-1-c03"></a>`M0.T0-1.C03` | 手動 `MANUAL.M0.T0-1.C03.DEPENDENCY-BASELINE` | `file:Cargo.toml` :: `section:[workspace.dependencies]`<br>`file:Cargo.lock` :: `lockfile`<br>`file:docs/progress.md` :: `heading:## 2026-08-05 - Task 0-1 - 計算部品のフォルダ構成と空の部品一式を作成` | 共通依存のworkspace一元化とlockfileを照合する。選定当時の「最新安定版」はofflineの現在treeから再判定しない。 | consistent — 現在sourceとTask 0-1の進捗は一元化・固定で一致する。 | none |
| <a id="roadmap-evidence-m0-t0-1-c04"></a>`M0.T0-1.C04` | 自動 `CHECK.LOCAL.RUST-WORKSPACE-TEST`<br>自動 `CHECK.LOCAL.RUST-WORKSPACE-CLIPPY` | `file:scripts/check.ps1` :: `Invoke-Check label:(1/5) cargo test --workspace`<br>`file:scripts/check.ps1` :: `Invoke-Check label:(2/5) cargo clippy --workspace --all-targets -- -D warnings`<br>`file:.github/workflows/ci.yml` :: `jobs:checks+performance` | debugから分離した重い検査をperformance側で補完したうえで、Rust workspace testとclippy警告0を確認する。素のdebug commandを直打ちしない。 | consistent — M0完了記録の全自動検査合格と現行gateが一致する。 | none |
| <a id="roadmap-evidence-m0-t0-1-c05"></a>`M0.T0-1.C05` | 手動 `MANUAL.M0.T0-1.C05.COMMIT-PUSH` | `git:19bc6ebd0009e62a15efc69f5eb17a7bdcfe6dbd` :: `subject:計算部品を置くためのフォルダ構成と空の部品一式を作成`<br>`git:19bc6ebd0009e62a15efc69f5eb17a7bdcfe6dbd` :: `ancestor-of:refs/remotes/origin/main`<br>`file:docs/progress.md` :: `heading:## 2026-08-05 - Task 0-1 - 計算部品のフォルダ構成と空の部品一式を作成` | 指定subjectのcommitと進捗記録を照合し、`origin/main`のancestorであることを手動受入時に確認する。 | consistent — 指定commitとTask 0-1記録が存在する。 | none |
| <a id="roadmap-evidence-m0-t0-2-c01"></a>`M0.T0-2.C01` | 手動 `MANUAL.M0.T0-2.C01.TAURI-SCAFFOLD` | `git:e231579ea7210b9f91a9a7e4987e389f78445acc` :: `tree:apps/desktop Tauri+React+TypeScript+Vite scaffold`<br>`file:apps/desktop/package.json` :: `dependencies:three,zustand+devDependencies:@types/three`<br>`file:docs/progress.md` :: `heading:## 2026-08-05 - Task 0-2 - アプリの画面が起動する最小の土台を作成` | 初期commitの雛形・依存と進捗を照合する。`npm create`の実行方法そのものは成果treeから推定しない。 | consistent — 現在sourceとTask 0-2記録が一致する。 | none |
| <a id="roadmap-evidence-m0-t0-2-c02"></a>`M0.T0-2.C02` | 自動 `CHECK.CURRENT-STATUS.WORKSPACE-MEMBERS`<br>自動 `CHECK.CURRENT-STATUS.TAURI-COMMANDS` | `file:Cargo.toml` :: `section:[workspace]/field:members`<br>`file:apps/desktop/src-tauri/src/lib.rs` :: `run/tauri::generate_handler!`<br>`file:scripts/generate-current-status.ps1` :: `functions:Get-WorkspaceMembers+Get-TauriCommands` | desktop memberがworkspaceにあり、handler inventoryにsample `greet`が無いことを確認する。 | consistent — 現在sourceとTask 0-2記録が一致する。 | none |
| <a id="roadmap-evidence-m0-t0-2-c03"></a>`M0.T0-2.C03` | 手動 `MANUAL.M0.T0-2.C03.TAURI-LAUNCH-TITLE` | `file:apps/desktop/src-tauri/tauri.conf.json` :: `/build/beforeDevCommand`<br>`file:apps/desktop/src-tauri/tauri.conf.json` :: `/app/windows/0/title`<br>`file:docs/progress.md` :: `heading:## 2026-08-05 - Task 0-2 - アプリの画面が起動する最小の土台を作成` | 当時のdev起動はprogressとconfigを照合する。現在再受入する場合は担当が`npm run tauri -- build --no-bundle`で組み立て、統括がその同梱版を起動してtitle bar `ORIGAMI3`を目視する。これは当時のdev command実行の代用とはしない。D1では組み立て・起動しない。 | consistent — 起動済み・title設定の記録はあるが当時のtitle目視の直接記録は弱いため、再受入手順を持つ手動IDで補う。 | none |
| <a id="roadmap-evidence-m0-t0-2-c04"></a>`M0.T0-2.C04` | 手動 `MANUAL.M0.T0-2.C04.COMMIT-PUSH` | `git:e231579ea7210b9f91a9a7e4987e389f78445acc` :: `subject:アプリの画面が起動する最小の土台を作成`<br>`git:e231579ea7210b9f91a9a7e4987e389f78445acc` :: `ancestor-of:refs/remotes/origin/main`<br>`file:docs/progress.md` :: `heading:## 2026-08-05 - Task 0-2 - アプリの画面が起動する最小の土台を作成` | 指定subjectのcommitと進捗記録を照合し、`origin/main`のancestorであることを手動受入時に確認する。 | consistent — 指定commitとTask 0-2記録が存在する。 | none |
| <a id="roadmap-evidence-m0-t0-3-c01"></a>`M0.T0-3.C01` | 自動 `CHECK.LOCAL.ALL-FIVE`<br>手動 `MANUAL.M0.T0-3.C01.ALL-FIVE-RUN` | `file:scripts/check.ps1` :: `function:Invoke-Check+labels:(1/5)..(5/5)`<br>`file:docs/progress.md` :: `heading:## 2026-08-05 - M0完了 - 基盤のレビューと修正が完了`<br>`file:docs/progress.md` :: `heading:## 2026-08-05 - Task 0-3 - 全ての自動チェックを一度に実行できる仕組みを追加`<br>`file:docs/progress.md` :: `heading:## 2026-08-05 - Task 1-6 - 展開図を描く画面(方眼・吸着・線の描画)を追加` | `scripts/check.ps1`を実行し、5段すべて成功でexit 0、いずれかの外部command非0・起動失敗でnon-zeroになることを確認する。 | historical-evolution — Task 0-3当時は4検査で、後続Task 1-6で画面testを足した現行5検査が正本である。 | none |
| <a id="roadmap-evidence-m0-t0-3-c02"></a>`M0.T0-3.C02` | 手動 `MANUAL.M0.T0-3.C02.COMMIT-PUSH` | `git:b89a9b6a5343715960cbc749ab0d876881438ffc` :: `subject:全ての自動チェックを一度に実行できる仕組みを追加`<br>`git:b89a9b6a5343715960cbc749ab0d876881438ffc` :: `ancestor-of:refs/remotes/origin/main`<br>`file:docs/progress.md` :: `heading:## 2026-08-05 - Task 0-3 - 全ての自動チェックを一度に実行できる仕組みを追加` | 指定subjectのcommitと進捗記録を照合し、`origin/main`のancestorであることを手動受入時に確認する。 | consistent — 指定commitとTask 0-3記録が存在する。 | none |
<!-- ORIGAMI3-ROADMAP-EVIDENCE:END scope=M0 -->

## M1: 展開図エディタ + 剛体折り(受け入れ: やっこさん)

### Task 1-1: ori3-model 型定義

**Files:** `crates/ori3-model/src/lib.rs`, `tests/serde_roundtrip.rs`

- [x] テスト: `Document`を構築→JSONへserialize→deserializeで往復一致(`test_document_json_roundtrip`)。実行して失敗確認 — [証拠:M1.T1-1.C01](traceability/roadmap-links.md#roadmap-evidence-m1-t1-1-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-1.C01 evidence=TEST.M1.T1-1.C01 -->
- [x] §2の型定義を実装。`Document::new(paper: Paper) -> Document`(輪郭4辺入りのCP初期化)も実装 — [証拠:M1.T1-1.C02](traceability/roadmap-links.md#roadmap-evidence-m1-t1-1-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-1.C02 evidence=TEST.M1.T1-1.C02 -->
- [x] テスト成功確認 → コミット `作品データ(紙・展開図・折り手順)の保存形式を定義` → プッシュ — [証拠:M1.T1-1.C03](traceability/roadmap-links.md#roadmap-evidence-m1-t1-1-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-1.C03 evidence=MANUAL.M1.T1-1.C03.COMMIT-PUSH -->

### Task 1-2: ori3-geometry 幾何プリミティブ

**Files:** `crates/ori3-geometry/src/{lib,primitives,isometry}.rs`, `tests/primitives.rs`

- [x] テストを先に書く: 交差あり/なし/平行/端点接触の`seg_intersection`、`point_on_segment`、`reflect_across_line`(点(1,0)を直線x=0で鏡映→(-1,0)) — [証拠:M1.T1-2.C01](traceability/roadmap-links.md#roadmap-evidence-m1-t1-2-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-2.C01 evidence=TEST.M1.T1-2.C01 -->
- [x] 実装: — [証拠:M1.T1-2.C02](traceability/roadmap-links.md#roadmap-evidence-m1-t1-2-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-2.C02 evidence=TEST.M1.T1-2.C02 -->

```rust
pub fn seg_intersection(a0: DVec2, a1: DVec2, b0: DVec2, b1: DVec2) -> Option<DVec2>;
pub fn collinear_overlap(a0: DVec2, a1: DVec2, b0: DVec2, b1: DVec2) -> Option<(DVec2, DVec2)>; // 同一直線上の重なり区間(点接触は同一点ペア)
pub fn point_on_segment(p: DVec2, a: DVec2, b: DVec2) -> bool;      // EPS許容
pub fn reflect_across_line(p: DVec2, l0: DVec2, l1: DVec2) -> DVec2;
pub fn dist_point_segment(p: DVec2, a: DVec2, b: DVec2) -> f64;

/// 2D等長変換: p' = R(θ)·M·p + t (M=鏡映フラグ)。合成・逆変換・線分への適用を持つ
pub struct Isometry2 { pub rotation: f64, pub translation: DVec2, pub mirrored: bool }
impl Isometry2 {
    pub fn identity() -> Self;
    pub fn reflection(l0: DVec2, l1: DVec2) -> Self;
    pub fn apply(&self, p: DVec2) -> DVec2;
    pub fn compose(&self, other: &Isometry2) -> Isometry2; // self ∘ other
    pub fn inverse(&self) -> Isometry2;
}
```

- [x] テスト成功確認 → コミット `線の交わりや折り返し位置を計算する基本部品を追加` → プッシュ — [証拠:M1.T1-2.C03](traceability/roadmap-links.md#roadmap-evidence-m1-t1-2-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-2.C03 evidence=MANUAL.M1.T1-2.C03.COMMIT-PUSH -->

### Task 1-3: ori3-cp 平面グラフと面抽出

**Files:** `crates/ori3-cp/src/{lib,graph,faces}.rs`, `tests/{graph,faces}.rs`

- [x] テストを先に書く: — [証拠:M1.T1-3.C01](traceability/roadmap-links.md#roadmap-evidence-m1-t1-3-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-3.C01 evidence=TEST.M1.T1-3.C01 -->
  - `insert_segment`: 正方形に対角線1本→辺数5・頂点数4。交差する2本目→両線が交点で分割され頂点数5・辺数8。既存線と同一線分の重複挿入→変化なし
  - `extract_faces`: 正方形のみ→面1。対角線1本→面2。米字(対角線2本+十字)→面8
- [x] 実装: — [証拠:M1.T1-3.C02](traceability/roadmap-links.md#roadmap-evidence-m1-t1-3-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-3.C02 evidence=TEST.M1.T1-3.C02 -->

```rust
/// 線分を挿入し、既存辺との交点で双方を自動分割する。追加された辺IDを返す。
/// 既存頂点・辺上の点へはEPSで吸着する。
pub fn insert_segment(cp: &mut CreasePattern, a: [f64; 2], b: [f64; 2], kind: EdgeKind) -> Vec<EdgeId>;
pub fn remove_edges(cp: &mut CreasePattern, ids: &[EdgeId]);      // 孤立頂点も掃除
pub fn move_vertex(cp: &mut CreasePattern, id: VertexId, to: [f64; 2]);

pub struct Face { pub id: FaceId, pub vertices: Vec<VertexId>, pub edges: Vec<EdgeId> }
/// half-edge構造を作り、各半辺から最左回りで面を辿る。外周面は除外。
pub fn extract_faces(cp: &CreasePattern) -> Vec<Face>;
```

- [x] テスト成功確認 → コミット `展開図の線の管理と、線で囲まれた面の検出を追加` → プッシュ — [証拠:M1.T1-3.C03](traceability/roadmap-links.md#roadmap-evidence-m1-t1-3-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-3.C03 evidence=MANUAL.M1.T1-3.C03.COMMIT-PUSH -->

### Task 1-4: DocumentStore とIPCコマンド(編集系)

**Files:** `apps/desktop/src-tauri/src/{lib,store,commands}.rs`, `store.rs`のユニットテスト

- [x] テスト(storeはTauri非依存の純Rustとして書く): `apply_edit`でAddSegment→undo→redoの状態一致、Undo100段制限、`document_save`/`document_open`の往復一致 — [証拠:M1.T1-4.C01](traceability/roadmap-links.md#roadmap-evidence-m1-t1-4-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-4.C01 evidence=TEST.M1.T1-4.C01 -->
- [x] 実装: — [証拠:M1.T1-4.C02](traceability/roadmap-links.md#roadmap-evidence-m1-t1-4-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-4.C02 evidence=TEST.M1.T1-4.C02 -->

```rust
pub struct DocumentStore {
    doc: Document,
    undo_stack: Vec<Document>,  // 編集前スナップショット方式。100件でFIFO破棄
    redo_stack: Vec<Document>,
    dirty: bool,
    path: Option<PathBuf>,
}
impl DocumentStore {
    pub fn apply_edit(&mut self, op: EditOp) -> Result<DocumentView, String>;
    pub fn apply_seq(&mut self, op: SeqOp) -> Result<DocumentView, String>;
    pub fn undo(&mut self) -> Result<DocumentView, String>;
    pub fn redo(&mut self) -> Result<DocumentView, String>;
}
/// フロントへ返す表示用ビュー(Document全体 + 導出情報)
#[derive(serde::Serialize)]
pub struct DocumentView {
    pub doc: Document,
    pub faces: Vec<Face>,
    pub warnings: Vec<String>,      // 操作固有の警告 + ori3_cp::validate の結果(「止めずに警告」原則)
    pub violations: Vec<VertexId>,  // 局所平坦折り判定(Task 2-7)。今は常に空
}
```

Undo/Redoは編集前スナップショット方式を正式採用する。100段の履歴と原子的な取り消しは検査で固定済みであり、`apply_edits` は1回のジェスチャーを1履歴として扱う。逆操作方式への作り替えは広範囲になる一方で利用者から見た違いがないためである。記憶使用量は、折り目の多い作品で100段積んだ実測から上限を定め、検査で固定する。実測が許容上限の3分の1以下に収まらない場合は逆操作方式を再検討する。

- [x] Tauriコマンド `document_new/open/save`, `edit_apply`, `edit_undo`, `edit_redo`, `sequence_apply` を`commands.rs`に登録(各3〜10行、storeへ委譲)。panic捕捉ラッパー`fn guard<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String>`を全コマンドに適用 — [証拠:M1.T1-4.C03](traceability/roadmap-links.md#roadmap-evidence-m1-t1-4-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-4.C03 evidence=TEST.M1.T1-4.C03 -->
- [x] テスト成功確認 → コミット `作品データの保管と、編集・元に戻す・やり直しの機能を追加` → プッシュ — [証拠:M1.T1-4.C04](traceability/roadmap-links.md#roadmap-evidence-m1-t1-4-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-4.C04 evidence=MANUAL.M1.T1-4.C04.COMMIT-PUSH -->

### Task 1-5: フロント基盤(ストア・IPCクライアント・4区画レイアウト)

**Files:** `apps/desktop/src/{App.tsx, store/appStore.ts, ipc/client.ts, lib/types.ts, components/{ToolRail,ContextPanel}.tsx}`

- [x] `lib/types.ts`: §2のRust型に対応するTS型を手書きで定義(Document, EditOp, SeqOp, Frame3D等。フィールド名はserde出力と一致させる) — [証拠:M1.T1-5.C01](traceability/roadmap-links.md#roadmap-evidence-m1-t1-5-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-5.C01 evidence=TEST.M1.T1-5.C01 -->
- [x] `ipc/client.ts`: 型付きラッパー関数のみ(1関数5行以内)。実装済み7コマンド分を定義し、残り6コマンドは各実装タスクで追加する — [証拠:M1.T1-5.C02](traceability/roadmap-links.md#roadmap-evidence-m1-t1-5-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-5.C02 evidence=TEST.M1.T1-5.C02 -->
- [x] `store/appStore.ts`(Zustand): 状態は `doc / faces / violations / selection / activeTool / frame3d / currentStep / warnings` と各action。IPC呼び出しはactionの中で行う — [証拠:M1.T1-5.C03](traceability/roadmap-links.md#roadmap-evidence-m1-t1-5-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-5.C03 evidence=TEST.M1.T1-5.C03 -->
- [x] `App.tsx`: 4区画CSSグリッド(ツールレール64px / 2Dと3Dは1:1で可変 / 下部コンテキストパネル160px) — [証拠:M1.T1-5.C04](traceability/roadmap-links.md#roadmap-evidence-m1-t1-5-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-5.C04 evidence=MANUAL.M1.T1-5.C04.SCREEN-ACCEPTANCE -->
- [x] `npm run build`成功 → コミット `画面の基本レイアウト(4区画)と画面側の土台を追加` → プッシュ — [証拠:M1.T1-5.C05](traceability/roadmap-links.md#roadmap-evidence-m1-t1-5-c05) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-5.C05 evidence=MANUAL.M1.T1-5.C05.COMMIT-PUSH -->

### Task 1-6: 2D展開図エディタ

**Files:** `apps/desktop/src/components/CpEditor/{CpEditor.tsx, renderer.ts, interaction.ts, snap.ts}`

- [x] スナップはフロントエンド(TypeScript)側で実装する: `snap(doc, cursorPos, radius): SnapResult | null`。優先順: 既存頂点 > グリッド交点 > 線分上(交点は挿入時に自動で頂点化されるため「既存頂点」に含まれる)。`SnapResult { pos: [x,y], kind: "vertex" | "grid" | "edge" }` — [証拠:M1.T1-6.C01](traceability/roadmap-links.md#roadmap-evidence-m1-t1-6-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-6.C01 evidence=TEST.M1.T1-6.C01 -->
  - 理由: IPCにスナップ専用コマンドはなく、マウス移動のたびのIPC往復は応答性が悪い。展開図データはフロントのストアに常にあるため、フロント側の純関数として実装しユニットテスト(vitest等)を付ける
- [x] Canvas描画(`renderer.ts`): 紙(白)・グリッド(薄灰)・輪郭(黒実線)・山(赤)・谷(青)・補助(灰)・選択強調(太線)・スナップ候補(丸マーカー)。線種の色分けは定数モジュールに集約 — [証拠:M1.T1-6.C02](traceability/roadmap-links.md#roadmap-evidence-m1-t1-6-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-6.C02 evidence=MANUAL.M1.T1-6.C02.SCREEN-ACCEPTANCE -->
- [x] 操作(`interaction.ts`): ツール=選択/山/谷/補助/削除。2クリックで線分確定(スナップ適用)、Escでキャンセル、矩形選択、Delete削除、ホイールズーム、中ボタンパン — [証拠:M1.T1-6.C03](traceability/roadmap-links.md#roadmap-evidence-m1-t1-6-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-6.C03 evidence=MANUAL.M1.T1-6.C03.SCREEN-ACCEPTANCE -->
- [x] ツールレール接続(ボタン: 選択・山・谷・補助・削除・全体表示の6個) — [証拠:M1.T1-6.C04](traceability/roadmap-links.md#roadmap-evidence-m1-t1-6-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-6.C04 evidence=MANUAL.M1.T1-6.C04.SCREEN-ACCEPTANCE -->
- [x] 手動確認: グリッド8分割で鶴の基本形の展開図が描ける → コミット `展開図を描く画面(方眼・吸着・線の描画)を追加` → プッシュ — [証拠:M1.T1-6.C05](traceability/roadmap-links.md#roadmap-evidence-m1-t1-6-c05) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-6.C05 evidence=MANUAL.M1.T1-6.C05.COMMIT-PUSH -->

### Task 1-7: ori3-rigid 全域木の角度伝播(ループなしCP)

**Files:** `crates/ori3-rigid/src/{lib,tree}.rs`, `tests/tree.rs`

- [x] テストを先に書く: — [証拠:M1.T1-7.C01](traceability/roadmap-links.md#roadmap-evidence-m1-t1-7-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-7.C01 evidence=TEST.M1.T1-7.C01 -->
  - 正方形+中央縦1本、ヒンジ角180°→2面が重なる(左面の頂点が右面へ鏡映された位置、z差はEPS以内)
  - ヒンジ角90°→2面のなす二面角が90°(法線の内積で検証)
- [x] 実装: — [証拠:M1.T1-7.C02](traceability/roadmap-links.md#roadmap-evidence-m1-t1-7-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-7.C02 evidence=TEST.M1.T1-7.C02 -->

```rust
/// 面隣接グラフのBFS全域木を作り、根面をxy平面に固定、
/// 木辺のヒンジ角(未指定は0)で子面の姿勢(DMat3+DVec3)を伝播する。
pub struct FoldedFrame { pub transforms: HashMap<FaceId, (DMat3, DVec3)> }
pub fn propagate(cp: &CreasePattern, faces: &[Face], angles: &HashMap<EdgeId, f64>) -> FoldedFrame;
pub fn to_frame3d(cp: &CreasePattern, faces: &[Face], frame: &FoldedFrame) -> Frame3D;
```

- [x] テスト成功確認 → コミット `折り線の角度から紙の立体的な形を計算する機能を追加` → プッシュ — [証拠:M1.T1-7.C03](traceability/roadmap-links.md#roadmap-evidence-m1-t1-7-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-7.C03 evidence=MANUAL.M1.T1-7.C03.COMMIT-PUSH -->

### Task 1-8: ori3-rigid ループ閉包ソルバー(内部頂点対応)

**Files:** `crates/ori3-rigid/src/solver.rs`, `tests/solver.rs`

- [x] テストを先に書く: — [証拠:M1.T1-8.C01](traceability/roadmap-links.md#roadmap-evidence-m1-t1-8-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-8.C01 evidence=TEST.M1.T1-8.C01 -->
  - 次数4の内部頂点1個のCP(鳥の基本形の1頂点相当)で、driver1本を90°にしたとき、残り3ヒンジの角が閉包条件(ループ一周の回転合成=恒等、残差フロベニウスノルム<1e-6)を満たす
  - driverを±180°にすると全ヒンジが±180°に達し平坦になる
  - 不能な指定(矛盾するdriver2本)でも`converged: false`と直前解を返しpanicしない
- [x] 実装: — [証拠:M1.T1-8.C02](traceability/roadmap-links.md#roadmap-evidence-m1-t1-8-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-8.C02 evidence=TEST.M1.T1-8.C02 -->

```rust
pub struct SolveResult { pub frame: Frame3D, pub converged: bool, pub angles: HashMap<EdgeId, f64> }
/// driver角を固定し、非木辺ヒンジごとのループ閉包残差を
/// Gauss-Newton(数値ヤコビアン+Levenberg減衰、最大50反復)で最小化。
/// warm_start: 前回解を初期値にする(連続的なスライダー操作で安定させる)
pub fn solve(cp: &CreasePattern, faces: &[Face], drivers: &[Driver],
             warm_start: Option<&HashMap<EdgeId, f64>>) -> SolveResult;
```

- [x] `pose_solve`コマンドをcommands.rsに追加(warm_startはstoreが保持) — [証拠:M1.T1-8.C03](traceability/roadmap-links.md#roadmap-evidence-m1-t1-8-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-8.C03 evidence=TEST.M1.T1-8.C03 -->
- [x] テスト成功確認 → コミット `複雑な展開図でも折り角度のつじつまを自動で合わせる計算を追加` → プッシュ — [証拠:M1.T1-8.C04](traceability/roadmap-links.md#roadmap-evidence-m1-t1-8-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-8.C04 evidence=MANUAL.M1.T1-8.C04.COMMIT-PUSH -->

### Task 1-9: 3Dビュー(Three.js)と角度操作

**Files:** `apps/desktop/src/components/Viewer3D/{Viewer3D.tsx, sceneBuilder.ts, hingePicker.ts}`, `ContextPanel.tsx`(ヒンジ選択時の内容)

- [x] `sceneBuilder.ts`: Frame3Dから面メッシュ生成(表=front_color/裏=back_color、DoubleSide不使用で2枚描き)、辺のライン表示、OrbitControls — [証拠:M1.T1-9.C01](traceability/roadmap-links.md#roadmap-evidence-m1-t1-9-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-9.C01 evidence=MANUAL.M1.T1-9.C01.SCREEN-ACCEPTANCE -->
- [x] `hingePicker.ts`: 3D上の辺クリックでヒンジ選択(画面距離しきい値で判定、選択中は黄色強調) — [証拠:M1.T1-9.C02](traceability/roadmap-links.md#roadmap-evidence-m1-t1-9-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-9.C02 evidence=MANUAL.M1.T1-9.C02.SCREEN-ACCEPTANCE -->
- [x] コンテキストパネル(ヒンジ選択時): 角度スライダー(−180〜+180)+数値入力。変更のたび`pose_solve`を呼びFrame3D更新(60ms間引き) — [証拠:M1.T1-9.C03](traceability/roadmap-links.md#roadmap-evidence-m1-t1-9-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-9.C03 evidence=MANUAL.M1.T1-9.C03.SCREEN-ACCEPTANCE -->
- [x] 不収束時: 3Dビュー右上に警告バッジ「⚠ 追従計算が収束していません」を表示(操作は継続) — [証拠:M1.T1-9.C04](traceability/roadmap-links.md#roadmap-evidence-m1-t1-9-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-9.C04 evidence=MANUAL.M1.T1-9.C04.SCREEN-ACCEPTANCE -->
- [x] 手動確認 → コミット `3D表示画面と、折り線ごとの角度操作を追加` → プッシュ — [証拠:M1.T1-9.C05](traceability/roadmap-links.md#roadmap-evidence-m1-t1-9-c05) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-9.C05 evidence=MANUAL.M1.T1-9.C05.COMMIT-PUSH -->

### Task 1-10: M1受け入れ(やっこさん)

**Files:** `crates/ori3-rigid/tests/acceptance_yakko.rs`

- [x] やっこさんの展開図(座布団折り2回相当の折り線)をコードで構築し、全driver±180°でsolveが収束し畳んだ位置(外形0.5角・8点が中心に重なる・内部頂点の写り先)が理論値と一致することを検証する回帰テスト(注: ±180°ではz座標は恒等的に0になるため、|z|<1e-6ではなく収束+位置一致を合格根拠とする) — [証拠:M1.T1-10.C01](traceability/roadmap-links.md#roadmap-evidence-m1-t1-10-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-10.C01 evidence=TEST.M1.T1-10.C01 -->
- [ ] 手動確認: アプリでやっこさんを描いて折る。操作上の問題は`docs/progress.md`に記録(実機のGUI確認待ち。自動テスト側は完了済み) — [証拠:M1.T1-10.C02](traceability/roadmap-links.md#roadmap-evidence-m1-t1-10-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-10.C02 evidence=MANUAL.M1.T1-10.C02.SCREEN-ACCEPTANCE -->
- [x] コミット `やっこさんが折れることを確認する自動テストを追加` → プッシュ — [証拠:M1.T1-10.C03](traceability/roadmap-links.md#roadmap-evidence-m1-t1-10-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M1.T1-10.C03 evidence=MANUAL.M1.T1-10.C03.COMMIT-PUSH -->

## M2: 層順序 + 折り操作 + 手順(受け入れ: 折り鶴)

### Task 2-0: 剛体折りソルバーの性能・数値改修(M1品質レビューからの必須引き継ぎ)

**Files:** `crates/ori3-rigid/src/{tree,solver}.rs`, `apps/desktop/src-tauri/src/commands.rs`

M1の品質レビューで「面400・辺1,000でsolve 33ms以内(NFR-002)」に対し現行の密行列Gauss-Newtonは約30倍超過と判定された。M2の手順再生(毎ステップ±180°平坦到達の連続solve)に入る前に以下を改修する:

- [x] 疎ヤコビアン化: ヒンジhの列は「hを含む基本ループの残差12成分」のみ非零。ループ局所性を使いJtJ構築と数値微分の全域再伝播を排除(可能なら回転微分の解析式化)。目標: 面400でsolve 33ms以内をベンチテストで確認(実装: 閉包拘束を「非木辺を1回渡り木辺+先順の非木辺で戻る最短閉路」に置き換え+解析ヤコビアン+RCM順の帯コレスキー。20×20ミウラ折り(面400・辺840)でwarm start 1回あたりrelease約3〜6ms、tests/perf_miura.rsで回帰監視) — [証拠:M2.T2-0.C01](traceability/roadmap-links.md#roadmap-evidence-m2-t2-0-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-0.C01 evidence=TEST.M2.T2-0.C01 -->
- [x] 収束判定を残差本数でスケールするRMS基準に変更(現行の絶対値1e-12は大規模でf64ノイズ床と衝突し、厳密解でもconverged=falseになり得る) — [証拠:M2.T2-0.C02](traceability/roadmap-links.md#roadmap-evidence-m2-t2-0-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-0.C02 evidence=TEST.M2.T2-0.C02 -->
- [x] ±180°近傍の縮退対策: 前進差分h=1e-6を中心差分またはh適応に(平坦到達の収束減速防止)(解析微分の厳密ヤコビアンに置き換えたため差分幅の問題自体が消滅。中心差分との一致は単体テストで検証) — [証拠:M2.T2-0.C03](traceability/roadmap-links.md#roadmap-evidence-m2-t2-0-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-0.C03 evidence=TEST.M2.T2-0.C03 -->
- [x] ±180°またぎのwrapで山谷符号が反転する問題: wrap前にwarm start前回値へ近い側を選ぶunwrap処理 — [証拠:M2.T2-0.C04](traceability/roadmap-links.md#roadmap-evidence-m2-t2-0-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-0.C04 evidence=TEST.M2.T2-0.C04 -->
- [x] 軽微: kind_signの線形走査をマップ化 / solve内のbuild_forest二重実行除去 / pose_solveのextract_faces毎回実行をstoreのキャッシュ流用に / driverを外した自由ヒンジがwarm start値のまま残る挙動をdocに明文化 — [証拠:M2.T2-0.C05](traceability/roadmap-links.md#roadmap-evidence-m2-t2-0-c05) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-0.C05 evidence=MANUAL.M2.T2-0.C05.SCREEN-ACCEPTANCE -->
- [x] コミット `折りの計算を大きな作品でも間に合う速さに改良` → プッシュ — [証拠:M2.T2-0.C06](traceability/roadmap-links.md#roadmap-evidence-m2-t2-0-c06) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-0.C06 evidence=MANUAL.M2.T2-0.C06.COMMIT-PUSH -->

あわせてフロント側もアニメーション(手順再生)に耐える構造へ改修する(M1品質レビューの引き継ぎ):

- [x] Viewer3D: トポロジとジオメトリの分離 — doc/faces変化時のみ三角形分割(スリット面の凹形状はShapeUtils.triangulateShape)とヒンジ集合を確定し、frame3d変化時はposition属性のin-place更新(DynamicDrawUsage)のみ。表裏は1ジオメトリ+addGroup+マテリアル配列。三角形index→面IDの対応表も作る(Task 2-5のraycastで必要)(実装: `buildTopology`(slots/indices/triangleFaceIds/lineIndices/hingeSlots/flatPositions)+ `createContent` + `updateFrame`。表裏は同じ三角形範囲へaddGroup×2、裏はBackSide指定でThree.jsが法線を反転。境界線はposition属性を面と共有) — [証拠:M2.T2-0.C07](traceability/roadmap-links.md#roadmap-evidence-m2-t2-0-c07) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-0.C07 evidence=MANUAL.M2.T2-0.C07.SCREEN-ACCEPTANCE -->
- [x] 作り替え前にsceneBuilderのdispose回帰テストを1本追加(偽geometry/materialでdispose回数を数える)(`sceneBuilder.test.ts`のclearGroup 3件。マテリアル配列と非対象の子も確認) — [証拠:M2.T2-0.C08](traceability/roadmap-links.md#roadmap-evidence-m2-t2-0-c08) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-0.C08 evidence=MANUAL.M2.T2-0.C08.SCREEN-ACCEPTANCE -->
- [x] pose_solve系のIPCをcoalescing方式に変更 — 実行中は保留1件を最新値で上書きし完了時に発行(FIFO積み上げによる表示遅延の防止)。編集系は従来のFIFOのまま(実装: `SerialQueue.runLatest`。追い越された要求は`{ok:false, error:SUPERSEDED, isLatest:false}`で返り、既存の破棄規約にそのまま乗る。ただし「その1回だけ0度を明示する」意味を持つ解除系pose_solveは追い越されると意味が失われるためFIFOのまま。runの後ろに積まれたrunLatestは追い越しの対象にならず順序も逆転しない) — [証拠:M2.T2-0.C09](traceability/roadmap-links.md#roadmap-evidence-m2-t2-0-c09) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-0.C09 evidence=MANUAL.M2.T2-0.C09.SCREEN-ACCEPTANCE -->
- [x] 軽微: hingeEdgeIdsのuseMemo化(ストアの`hinges`としてdoc/faces更新時に1度だけ導出) / AngleNumberInputのdirtyフラグ(未編集blurでdriver化しない)+Escape取り消し / スロットルのテスト順序依存解消(`resetPoseThrottle`をexport) / コンテキストロスト復帰時の再描画 / setPixelRatioの追従 / render呼び出しのrAF集約 / ヒンジ選択の手前優先タイブレーク修正(0.5px刻み→手前の順で整列) / 3Dカメラのリセット手段(ツールレールの「全体」で2D・3D両方) — [証拠:M2.T2-0.C10](traceability/roadmap-links.md#roadmap-evidence-m2-t2-0-c10) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-0.C10 evidence=MANUAL.M2.T2-0.C10.SCREEN-ACCEPTANCE -->
- [x] コミット `3D表示を手順再生に耐える作りに改良` → プッシュ — [証拠:M2.T2-0.C11](traceability/roadmap-links.md#roadmap-evidence-m2-t2-0-c11) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-0.C11 evidence=MANUAL.M2.T2-0.C11.COMMIT-PUSH -->

### Task 2-1: ori3-layers 平坦状態

**Files:** `crates/ori3-layers/src/{lib,flat_state}.rs`, `tests/flat_state.rs`

- [x] テスト: 正方形を半分に折った状態→2面の配置が鏡映関係、層順序が[下面, 上面]。層順序の代表点参照(layer_orderの[f64;2]→FaceId解決)が面の再抽出後も正しく対応する(テスト12件。境界・凹面(スリット・枝分かれ)・解決不能点の警告・重複点も網羅) — [証拠:M2.T2-1.C01](traceability/roadmap-links.md#roadmap-evidence-m2-t2-1-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-1.C01 evidence=TEST.M2.T2-1.C01 -->
- [x] 実装(代表点は耳刈りで得た最初の三角形の重心、点の内外判定は境界EPS許容+交差数の偶奇): — [証拠:M2.T2-1.C02](traceability/roadmap-links.md#roadmap-evidence-m2-t2-1-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-1.C02 evidence=TEST.M2.T2-1.C02 -->

```rust
pub struct FlatState {
    pub placements: HashMap<FaceId, Isometry2>, // CP座標→畳んだ平面座標
    pub order: Vec<FaceId>,                     // 下→上
}
impl FlatState {
    pub fn initial(cp: &CreasePattern, faces: &[Face]) -> FlatState; // 全面恒等・任意順
    /// layer_order(代表点リスト)を面IDへ解決する。解決不能な点は警告として返す
    pub fn resolve_order(cp: &CreasePattern, faces: &[Face], points: &[[f64; 2]])
        -> (Vec<FaceId>, Vec<String>);
    pub fn to_layer_points(&self, cp: &CreasePattern, faces: &[Face]) -> Vec<[f64; 2]>;
}
/// 面の内部代表点(凹面でも内部に落ちる。決定的)
pub fn representative_point(cp: &CreasePattern, face: &Face) -> [f64; 2];
/// 点が面の内部(境界EPS以内を含む)にあるか
pub fn point_in_face(cp: &CreasePattern, face: &Face, p: [f64; 2]) -> bool;
```

- [x] テスト成功確認 → コミット `平らに畳んだときの紙の重なり順を管理する機能を追加` → プッシュ — [証拠:M2.T2-1.C03](traceability/roadmap-links.md#roadmap-evidence-m2-t2-1-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-1.C03 evidence=MANUAL.M2.T2-1.C03.COMMIT-PUSH -->

### Task 2-2: 折り操作プリミティブ(fold_through)

**Files:** `crates/ori3-layers/src/fold_through.rs`, `tests/fold_through.rs`

- [x] テストを先に書く(10件): — [証拠:M2.T2-2.C01](traceability/roadmap-links.md#roadmap-evidence-m2-t2-2-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-2.C01 evidence=TEST.M2.T2-2.C01 -->
  - 正方形を1回半分折り→層2枚。さらに直交方向に重ね折り→層4枚、CPに折り線が各層分(引き戻しで2本)追加され、山谷が層の向きに応じて正しく付く(mirrored反転の検証)
  - 段折り(同方向に2本、UpとDown)で層3枚・順序正しい
  - 対象層を「上1枚のみ」に指定した折りで、下層が動かない
  - 原子性(不正入力4種でErr・cpが完全無変更)/ layer_orderのresolve_order往復一致と決定性 / 紙が裂ける指定の警告
  - 折り線と重なる補助線が折り線へ昇格し、面が正しく分割され配置・層順序も半分折りと一致する(レビュー指摘の状態破壊の回帰テスト)
  - DriverLineの辺分割耐性: ステップ1の折り線が2回目の折りで2辺に分割された後も`resolve_driver_edges`が両断片を返し、全ステップのdriverをsolveに与えると畳んだ位置がFlatStateと一致する
- [x] 実装: — [証拠:M2.T2-2.C02](traceability/roadmap-links.md#roadmap-evidence-m2-t2-2-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-2.C02 evidence=TEST.M2.T2-2.C02 -->

```rust
/// 折る向き: Up=動く側の層を反転して山の一番上に載せる(表から見て谷折りに相当)
///           Down=一番下に入れる(山折りに相当)
pub enum FoldDirection { Up, Down }
pub struct FoldThroughInput {
    pub line: [[f64; 2]; 2],          // 畳んだ平面座標での折り線(無限直線として扱う)
    pub keep_side_point: [f64; 2],    // 動かさない側を示す点(畳んだ平面座標)
    pub target_layers: Option<Vec<FaceId>>, // None=可動側に幾何が乗る全ての層
    pub direction: FoldDirection,
}
pub struct FoldThroughResult {
    pub state: FlatState,             // 折った後の平坦状態(新しい面ID体系)
    pub added_edges: Vec<EdgeId>,     // CPへ追記された折り線(昇格させた補助線の断片を含む)
    pub step: FoldStep,               // 記録用(kind=Simple、drivers(DriverLine)+layer_order設定済み、id=0)
    pub warnings: Vec<String>,
}
/// 折り線を各対象面へplacement逆変換で引き戻してCPに挿入し(横切らない対象面は
/// 丸ごと動く)、可動側の面へ折り線の鏡映を重ね、層順序を「動いた面を旧順の逆順で
/// 山全体の上(Up)/下(Down)へ」で更新する。山谷はUp=谷/Down=山を基準に
/// mirroredな層で反転。重なった補助線は折りの線種へ昇格させ(面が分割されるように)、
/// 既存の山/谷線は線種を維持したままDriverLineの駆動対象にする。
/// CPの更新は複製上で行い、成功時のみ反映(原子性)。折り線が横切ったのに面を
/// 分割できなかった場合は状態を壊す前にErrで止める。
pub fn fold_through(cp: &mut CreasePattern, faces: &[Face], state: &FlatState,
                    input: &FoldThroughInput) -> Result<FoldThroughResult, String>;
/// DriverLineの線分上に乗る折り辺(両端点が線分からEPS以内)を解決する。
/// 後続の折りで辺が分割されていても全断片が返る(手順再生で使う)
pub fn resolve_driver_edges(cp: &CreasePattern, line: &DriverLine) -> Vec<EdgeId>;
```

  DriverLineは「対象面ごとの引き戻し区間」単位で生成する(同一線分は重複排除)。既存の山/谷線と重なる区間・面の縁に沿う既存折り目の区間にも、既存の線種に従った角度でDriverLineを作る。
  既知の制限(v1、docコメントに明記): 部分的な折りの層順序は近似(物理的に厳密な挟み込み順にならないことがある)。折り線がどの面も横切らない指定(既存折り線での再折りを含む)はErr
- [x] テスト成功確認 → コミット `畳んだ紙に線を引いてまとめて折る操作を追加` → プッシュ — [証拠:M2.T2-2.C03](traceability/roadmap-links.md#roadmap-evidence-m2-t2-2-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-2.C03 evidence=MANUAL.M2.T2-2.C03.COMMIT-PUSH -->
- [x] レビュー指摘の修正 → コミット `補助線の上で折れない・手順が再生できなくなる2つの欠陥を修正` → プッシュ — [証拠:M2.T2-2.C04](traceability/roadmap-links.md#roadmap-evidence-m2-t2-2-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-2.C04 evidence=MANUAL.M2.T2-2.C04.COMMIT-PUSH -->

### Task 2-3: 手順エンジン(記録・再生・決定性)

**Files:** `crates/ori3-layers/src/replay.rs`, `crates/ori3-layers/tests/replay.rs`, `apps/desktop/src-tauri/src/{commands,store}.rs`

- [x] テスト: — [証拠:M2.T2-3.C01](traceability/roadmap-links.md#roadmap-evidence-m2-t2-3-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-3.C01 evidence=TEST.M2.T2-3.C01 -->
  - 手順3ステップの`Document`を`replay(doc, up_to, t)`で2回再生→Frame3Dがビット一致(SYS-004)
  - 展開図に無関係な補助線を追加後の再生→全ステップ成功(補助線が折り線を分割してもよい)
  - 手順が参照する折り線を削除後の再生→該当ステップがスキップされ警告リストに載り、以降のステップは続行(SEQ-004)
  - 一部だけ解決できない手順は残りで続行+警告 / 折り線を持たない手順(Pose)は飛ばさない / up_to・tの範囲外は丸める
  - 途中ステップ(up_to=k)の外形が期待値どおり=まだ折っていない折り線が曲がっていない / `replay(k, t=0)` が `replay(k-1, t=1)` とビット一致(非縮退のk≥2を含む)
  - 層順序の代表点が1点も解決できない手順は直前の層順序を保つ
  - 性能(NFR-002): 10ステップ・面400の全再生が3秒以内(蛇腹400面・辺1,201・層順序400点/ステップで debug実測 約0.7秒 / release実測 約23ms)
- [x] 実装: — [証拠:M2.T2-3.C02](traceability/roadmap-links.md#roadmap-evidence-m2-t2-3-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-3.C02 evidence=TEST.M2.T2-3.C02 -->

```rust
/// ステップ列を順に適用する。3D状態は保存せず、平らな展開図に「そこまでの全ステップの
/// driver」を累積して与えて1回で解く(折った状態の上に次を折るのではない)。
/// 各ステップのDriverLineは resolve_driver_edges で現在の辺IDへ解決し、
/// up_to未満は目標角そのまま、up_toステップ目だけ角度をt倍する。
/// まだ折っていない折り線(=手順1..up_toが駆動しないヒンジ)は0°のdriverとして
/// 明示的に固定する。自由変数のまま残すとソルバーの初期値バイアスから別の枝へ
/// 収束し、警告なしで誤った形(後続の折り線まで曲がった形)が返るため。
/// 結果として全ヒンジが固定値になるのでステップごとに解き直す必要はなく、
/// warm startも使わない(決定的)。
/// 層順序は各ステップのlayer_orderをresolve_orderで解決してFace3D.layerへ反映
/// (up_toステップ目は完了時t=1のみ。None・空・1点も解決できない・飛ばした手順では
/// 直前の層順序を保つ)。
/// up_to: 表示対象ステップ(0=初期状態)、t: 0..=1 の補間係数
pub fn replay(doc: &Document, up_to: usize, t: f64) -> ReplayResult;
/// 面抽出済みの呼び出し側(store等)向け。extract_facesの二重実行を避ける
pub fn replay_with_faces(doc: &Document, faces: &[Face], up_to: usize, t: f64) -> ReplayResult;
pub struct ReplayResult { pub frame: Frame3D, pub skipped: Vec<StepId>, pub warnings: Vec<String> }
```

- [x] `sequence_replay`コマンド追加(9個目。引数`up_to: usize, t: f64`)。`DocumentView`に`frame: Option<Frame3D>`・`skipped: Vec<StepId>`を追加し、ビューを返す全コマンドの成功後に最新ステップまで自動再生して載せる(手順が空なら`frame: None`) — [証拠:M2.T2-3.C03](traceability/roadmap-links.md#roadmap-evidence-m2-t2-3-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-3.C03 evidence=MANUAL.M2.T2-3.C03.SCREEN-ACCEPTANCE -->
- [x] ロック規約の徹底: 自動再生はstore内(ロック保持中)ではなく、コマンド層の`view_command`がロック解放後に`store::attach_replay`で行う。`sequence_replay`もロック下はDocument+facesの複製のみ — [証拠:M2.T2-3.C04](traceability/roadmap-links.md#roadmap-evidence-m2-t2-3-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-3.C04 evidence=TEST.M2.T2-3.C04 -->
- [x] テスト成功確認 → コミット `折り手順の記録と再生(展開図を直したら自動で折り直す)を追加` → プッシュ — [証拠:M2.T2-3.C05](traceability/roadmap-links.md#roadmap-evidence-m2-t2-3-c05) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-3.C05 evidence=MANUAL.M2.T2-3.C05.COMMIT-PUSH -->
- [x] レビュー指摘の修正 → コミット `折り途中の手順を選んだときに違う形が表示される問題を修正` → プッシュ — [証拠:M2.T2-3.C06](traceability/roadmap-links.md#roadmap-evidence-m2-t2-3-c06) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-3.C06 evidence=MANUAL.M2.T2-3.C06.COMMIT-PUSH -->

### Task 2-4: タイムラインUI

**Files:** `apps/desktop/src/components/Timeline.tsx`, `ContextPanel.tsx`(ステップ選択時)

- [x] ステップ一覧(番号+技法名+警告アイコン)、クリックで選択→その時点の3D表示、◀▶コマ送り、▶再生(driver角補間アニメーション、320ms/ステップ)。タイムラインは3Dビュー区画の内側を上下分割して置く(常設区画は4つのまま)。技法名の日本語表は`lib/techniques.ts`、補間の進行計算は`lib/playback.ts`(純関数) — [証拠:M2.T2-4.C01](traceability/roadmap-links.md#roadmap-evidence-m2-t2-4-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-4.C01 evidence=MANUAL.M2.T2-4.C01.SCREEN-ACCEPTANCE -->
- [x] ステップ選択時のコンテキストパネル: 技法種別変更・注記編集・削除ボタン(`sequence_apply`) — [証拠:M2.T2-4.C02](traceability/roadmap-links.md#roadmap-evidence-m2-t2-4-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-4.C02 evidence=MANUAL.M2.T2-4.C02.SCREEN-ACCEPTANCE -->
- [x] スキップされたステップは赤表示+ツールチップで理由 — [証拠:M2.T2-4.C03](traceability/roadmap-links.md#roadmap-evidence-m2-t2-4-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-4.C03 evidence=MANUAL.M2.T2-4.C03.SCREEN-ACCEPTANCE -->
- [x] 手動確認 → コミット `折り手順の一覧表示と再生・コマ送りの画面を追加` → プッシュ — [証拠:M2.T2-4.C04](traceability/roadmap-links.md#roadmap-evidence-m2-t2-4-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-4.C04 evidence=MANUAL.M2.T2-4.C04.COMMIT-PUSH -->

### Task 2-5: 3Dビュー上の折り線描画と折り操作(SIM-005)

**Files:** `apps/desktop/src/components/Viewer3D/foldDraw.ts`, `apps/desktop/src/lib/planeProject.ts`, `ContextPanel.tsx`(折りツール時), `store/appStore.ts`(拡張), `crates/ori3-layers/src/replay.rs`(`flat_state_at`), `crates/ori3-model/src/lib.rs`(`SeqOp::FoldThrough`)

- [x] 手順から現在の平坦状態を導出する `flat_state_at(doc, faces, up_to) -> Result<FlatState, String>`(3D状態は保存しない設計のため、再生結果の3D姿勢からxy平面の等長変換を取り出す。平坦でなければErr)。座標系は3D表示と同じ(根面=最小面IDが恒等変換) — [証拠:M2.T2-5.C01](traceability/roadmap-links.md#roadmap-evidence-m2-t2-5-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-5.C01 evidence=MANUAL.M2.T2-5.C01.SCREEN-ACCEPTANCE -->
- [x] Tauriコマンドは増やさず、`SeqOp::FoldThrough { up_to, line, keep_side_point, target_layers, direction }` を追加して `sequence_apply` で実現(`FoldDirection`はori3-modelへ移動しserde対応、ori3-layersは再エクスポート)。v1は末尾(`up_to == sequence.len()`)のみ許可し、途中への挿入はErr — [証拠:M2.T2-5.C02](traceability/roadmap-links.md#roadmap-evidence-m2-t2-5-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-5.C02 evidence=TEST.M2.T2-5.C02 -->
- [x] 3Dビューに「折る」ツールを追加(ツールレール7個目): 平坦状態の紙の上でドラッグ→画面座標をz=0平面へ投影(`lib/planeProject.ts`)→端点を紙の輪郭・既存頂点へスナップ(`foldDraw.ts`)→折り線と動く側のプレビュー表示(既存のハイライト機構を流用、動く側は半平面で切り取った輪郭) — [証拠:M2.T2-5.C03](traceability/roadmap-links.md#roadmap-evidence-m2-t2-5-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-5.C03 evidence=MANUAL.M2.T2-5.C03.SCREEN-ACCEPTANCE -->
- [x] 平坦でないとき(折り途中の手順・再生中・角度スライダー使用中)は3Dビュー右上に「平らに畳んだ状態で使えます」と出して描画させない — [証拠:M2.T2-5.C04](traceability/roadmap-links.md#roadmap-evidence-m2-t2-5-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-5.C04 evidence=MANUAL.M2.T2-5.C04.SCREEN-ACCEPTANCE -->
- [x] 確定UI(コンテキストパネル): 方向(手前へ折る(谷)/向こうへ折る(山))、対象層(全ての層/いちばん上の1枚)、動かす側(左/右)→「折る」で`SeqOp::FoldThrough`を送信→2D展開図に折り線が追記され、タイムラインに手順が1つ増える。「やめる」で破棄(v1では「選択した層」は出さない) — [証拠:M2.T2-5.C05](traceability/roadmap-links.md#roadmap-evidence-m2-t2-5-c05) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-5.C05 evidence=MANUAL.M2.T2-5.C05.SCREEN-ACCEPTANCE -->
- [x] 2D側でも同じ折り操作を出せるようにする(2回クリックで線を引き、同じ確定UIを使用)。手順が1つ以上ある作品では展開図座標と畳み平面座標が食い違うため2D側からの折りは断り、「折る操作は3D画面から行ってください」と案内する — [証拠:M2.T2-5.C06](traceability/roadmap-links.md#roadmap-evidence-m2-t2-5-c06) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-5.C06 evidence=MANUAL.M2.T2-5.C06.SCREEN-ACCEPTANCE -->
- [x] 手動確認: 座布団折り→観音折り(6手順)を3D側の線描画だけで完成できることを実機のスクリーンショットで確認。同じ手順の自動テストも追加(`cushion_then_cupboard_fold_only_with_fold_through`) → コミット `3D画面に直接線を引いて折る操作を追加(展開図へ自動反映)` → プッシュ — [証拠:M2.T2-5.C07](traceability/roadmap-links.md#roadmap-evidence-m2-t2-5-c07) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-5.C07 evidence=MANUAL.M2.T2-5.C07.COMMIT-PUSH -->

### Task 2-6: 技法マクロ(段・中割り・かぶせ / 花弁・開いてつぶすは残作業)

**Files:** `crates/ori3-layers/src/techniques.rs`, `tests/techniques.rs`

- [x] テスト: 2層・3層(段折りでできた奇数層)・4層のフラップを下ごしらえとして、(a)中割り折りで首を折る→層数・層順序・CPへの追加線が期待値どおり (b)続けてもう一度中割り折りして頭にできる(鶴の首と頭の流れ)。折り目の向き(山谷)と層順序の一致検証を全技法に、t=0.99の高さからの重なり検証を段折り・かぶせ折りに適用(中割り折りだけは、フラップを開く動きを再生で表せないため高さからは判定できない) — [証拠:M2.T2-6.C01](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6.C01 evidence=TEST.M2.T2-6.C01 -->
- [x] 実装(全て「fold_throughと層順序操作の合成」として実装し、専用データ構造を持たない): — [証拠:M2.T2-6.C02](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6.C02 evidence=TEST.M2.T2-6.C02 -->

```rust
/// 対象フラップ(面集合)と折り線・基準点を受け取り、技法に必要な折り線群・
/// driver群・層順序変化を生成してFoldStepを返す。
pub fn pleat(cp, faces, state, input: &TechniqueInput) -> Result<FoldThroughResult, String>;
pub fn inside_reverse(...) -> Result<FoldThroughResult, String>;
pub fn outside_reverse(...) -> Result<FoldThroughResult, String>;
```

  引数は共通で `(cp, faces, state, &TechniqueInput { flap, line, reference_point })`。生成不能な形状ではErrを返し、UI側は「手動の折り操作で代替してください」と案内(要件§12)
- [x] Tauriコマンドは増やさず、`SeqOp::Technique { up_to, kind, flap, line, reference_point }` を追加して `sequence_apply` で実現(末尾のみ許可) — [証拠:M2.T2-6.C03](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6.C03 evidence=TEST.M2.T2-6.C03 -->
- [x] ツールレールに「技法」ボタン(8個目・サブメニュー3種)を追加し、フラップクリック→線指定→適用の流れを実装 — [証拠:M2.T2-6.C04](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6.C04 evidence=MANUAL.M2.T2-6.C04.SCREEN-ACCEPTANCE -->
- [x] 先端をどちら向きに回すかは、層の数を機械的に半分に割るのではなく**紙のつながり**(折り線が横切る折り目でつながった層は反対向きに回る)から決める。奇数層のフラップや、重なりの一部だけを選んだフラップでも物理どおりに折れる。Errで断るのは幾何的に定まらない入力だけ(折り線がフラップを横切らない・つながりが奇数の輪になる)で、紙が裂ける指定・山谷と重なり順が食い違う指定は警告して続行する(「止めずに警告」原則) — [証拠:M2.T2-6.C05](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6-c05) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6.C05 evidence=TEST.M2.T2-6.C05 -->
- [x] テスト成功確認 → コミット `中割り折りなど基本の折り方を選ぶだけで折れる機能を追加` → プッシュ — [証拠:M2.T2-6.C06](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6-c06) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6.C06 evidence=MANUAL.M2.T2-6.C06.COMMIT-PUSH -->

### Task 2-6b: 汎用の折り操作基盤(重み18)

**Files:** `crates/ori3-layers/src/{flat_motion.rs, techniques.rs, fold_through.rs}`, `tests/`, UI

要件定義書の**設計原則0(表現の完全性)とSIM-011**を実現する中核タスク。「物理的には折れるのにアプリでは表現できない」状態を無くす。Task 2-6で判明した2つの不足(花弁折り・つぶし折りが作れない / 層数の偶奇による制限)を、個別対応ではなく**汎用プリミティブ**で根本解決する。

**設計の骨子(要件§7.1bと対応)**: 平坦状態から平坦状態への紙の動きは「動かす層の集合」と「それらに施す等長変換(反射の合成)」で表せる。反射の軸がそのまま折り線になる。

- [x] **設計フェーズ(実装前に必ず行う)**: 下記のケースを紙で折る思考実験で洗い出し、1つのプリミティブで表せることを確認してから実装する。表せないケースがあれば設計を見直す — [証拠:M2.T2-6b.C01](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6b-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6b.C01 evidence=TEST.M2.T2-6b.C01 -->
  - 単純折り(反射1回)/ 折り目を開く(既存の折り目を0°へ)/ つぶし折り(反射2回=回転。紙が動かない退化ケースを含む)/ 花弁折り / 中割り・かぶせ(層ごとに逆向き)/ 沈め折り(領域の山谷反転)/ ひだ寄せ / ねじり折り / 部分的な層だけを動かす操作 / 奇数層のフラップ

**設計フェーズの結果(2026-08-05)**

紙の動きは「(1) どの紙がどの等長変換で動くか」と「(2) 動いた紙が重なりのどこへ入るか」の2つだけで決まる。1つの動きを複数の **動かす部分(`MotionPart`)** の集まりとして表す:

| 項目 | 内容 |
|---|---|
| `layers` | 動かす層(現在の面ID)。空なら全ての層 |
| `region` | 畳み平面での領域(半平面の共通部分)。空なら層まるごと。**境界線がそのまま新しい折り線になる** |
| `transform` | `Stay`(動かさない)/ `Reflect(直線の列)`(鏡映を順に適用)/ `Isometry`(直接指定) |
| `turn` | `Keep`(重なり順そのまま)/ `Outside(上下)`(重なり全体の上/下へ回す)/ `Inside(上下)`(分かれた元の紙のすぐ隣へ差し込む)/ `Beside{基準面, 上下}` |
| `reverse_layers` | 部分の中の重なり順を逆にするか(既定は「裏返る変換なら逆順」) |

隣り合う部分の相対変換がその境界線の鏡映になっていれば紙はつながる。ならなければ「裂ける指定」として警告して続行する。山谷は最終的な重なり順から決め直し、角度が変わった折り目(開いた折り目は0°)だけをDriverLineに記録する。

| 紙の動き | 表し方 | 検証テスト(`tests/flat_motion.rs`) |
|---|---|---|
| 単純折り(反射1回) | 1部分・領域=折り線の可動側・`Reflect([線])`・`Outside` | `simple_fold_gives_the_same_result_as_fold_through` |
| 一部の層だけ折る / 奇数の重なり | 同上で `layers` を絞る(層数の偶奇を仮定しない) | `moving_only_some_layers_of_an_odd_stack_warns_but_continues` |
| 折り目を開く | 1部分・領域=空・`Reflect([その折り目])`(配置が一致して角度0°になる) | `opening_a_crease_brings_the_paper_back_flat` |
| つぶし折り(反射2回=回転) | 複数部分。奥の紙は `Reflect([線1, 線2])` = 回転 | `one_motion_can_use_a_rotation_made_of_two_reflections` |
| つぶし折りの退化(紙が動かない) | `Stay` + `turn` で重なり順だけ変える(山谷は重なり順から決め直される) | `restacking_without_moving_paper_only_changes_layers_and_creases` |
| 中割り折り・かぶせ折り(層ごとに逆向き) | 同じ鏡映の2部分に、逆向きの `Inside` を与える | `one_motion_can_turn_each_layer_the_opposite_way` |
| 花弁折り | 「開く部分」と「たたむ部分」を1回の動きにまとめる | `one_motion_can_open_one_crease_and_make_another` |
| 沈め折り(領域の山谷反転) | 領域を指定し `Stay` + `reverse_layers` | `layers_inside_a_region_can_be_turned_inside_out_without_moving` |
| ひだ寄せ・ねじり折り | 部分ごとに別々の等長変換(回転を含む)を与える | 上の回転テストと同じ仕組み |

- [x] **プリミティブの実装**: `pub fn flat_motion(cp, faces, state, input: &FlatMotionInput) -> Result<FoldThroughResult, String>`(`crates/ori3-layers/src/flat_motion.rs`) — [証拠:M2.T2-6b.C02](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6b-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6b.C02 evidence=TEST.M2.T2-6b.C02 -->
  - 処理: (a)領域の境界線を各層の面へ引き戻してCPへ追加 (b)**紙のつながりを検査**(辺の両端が両側の配置で同じ点に写るか)。切れていれば「裂ける指定」として**警告して続行** (c)新しい面配置と層順序を `turn` から構成的に決定 (d)角度の変わる折り線を全てDriverLineとして記録(開いた折り目は0°)
  - **層の分割は実際の紙のつながりから決める**。「層数を半分ずつ」のような仮定を置かない → 奇数層・部分フラップも自然に扱える
  - **手順再生との整合(検証済み)**: `DriverLine.target_angle_deg` は任意の値を取れ、`plan_steps`(replay.rs)・`flat_state_at`・ソルバー(solver.rs)がいずれも「後のステップが勝つ」ため、「既存の折り目を0°へ駆動する」は再生で正しく効く。土台の追加改修は不要
  - **Errで断るのは幾何的に定義不能な入力だけ**: 退化した直線 / 内側を示す点が線上 / 平坦状態に配置の無い面 / 動かす対象が1つも無い / 折り線が面を横切っているのに面を分割できなかった
- [x] **既存技法の作り直し**: `fold_through` を `flat_motion` への薄い委譲に再実装(`pleat`・`inside_reverse`・`outside_reverse` は `fold_through` 経由で自動的にその上に乗る)。層数の偶奇による制限は無し。既存テストは1件も変更せず全て合格 — [証拠:M2.T2-6b.C03](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6b-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6b.C03 evidence=TEST.M2.T2-6b.C03 -->
- [ ] **`squash`(開いてつぶす)・`petal`(花弁折り)を実装**。鶴の基本形の前面が持ち上がることをテスト — [証拠:M2.T2-6b.C04](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6b-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6b.C04 evidence=TEST.M2.T2-6b.C04 -->
- [ ] **UI: 「つまんで動かす」ツール**(SIM-011)。畳んだ状態で層を選び、目標位置へドラッグすると必要な折り線を自動で求めて折る。技法として名前が付いていない動きもこれで行える。関連する道具としてツールレールへ整理して追加する — [証拠:M2.T2-6b.C05](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6b-c05) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6b.C05 evidence=MANUAL.M2.T2-6b.C05.SCREEN-ACCEPTANCE -->
- [ ] UIの技法サブメニューを9種に(層操作/段/中割り/かぶせ/つぶし/花弁/沈め/ひだ寄せ/ねじり) — [証拠:M2.T2-6b.C06](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6b-c06) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6b.C06 evidence=MANUAL.M2.T2-6b.C06.SCREEN-ACCEPTANCE -->
- [x] テスト: 上記の設計フェーズで洗い出した全ケース、表示上の重なり順検証(t=0.99のz読み取り方式)、記録した手順からの再生一致、奇数層・部分フラップ、原子性(`crates/ori3-layers/tests/flat_motion.rs` 9件) — [証拠:M2.T2-6b.C07](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6b-c07) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6b.C07 evidence=MANUAL.M2.T2-6b.C07.SCREEN-ACCEPTANCE -->
- [x] コミット `どんな折り方でも表せる汎用の折り操作を追加` → プッシュ — [証拠:M2.T2-6b.C08](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6b-c08) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6b.C08 evidence=MANUAL.M2.T2-6b.C08.COMMIT-PUSH -->

### Task 2-6c: 直感的な折り操作UI(重み12)

**Files:** `apps/desktop/src/components/Viewer3D/*`, `components/{ContextPanel,ToolRail,Timeline}.tsx`, `store/appStore.ts`

要件定義書の**設計原則3b(直感的に触れること)とUI-007〜010**を実現する。現状は「折り線を引く→パネルで方向・対象層・動かす側を選ぶ→折るボタン」という手数の多い操作になっており、これを**紙を直接つかんで動かす**操作に置き換える。

- [ ] **層のずらし表示(UI-010 / SIM-004)**: 平坦状態では層ごとに微小オフセット(表示専用)を付けて重なりを見せる。層の枚数が多いときも潰れないよう、視点距離に応じてオフセット量を調整。**これが無いと畳んだ紙が1枚に見えて選択操作が理解できない**ため最初に実装する — [証拠:M2.T2-6c.C01](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6c-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6c.C01 evidence=MANUAL.M2.T2-6c.C01.SCREEN-ACCEPTANCE -->
- [ ] **つかんで動かす操作(UI-007)**: 3Dビューで紙の上をドラッグすると、(a)つかんだ点にある層のうち最も手前のフラップを自動選択 (b)ドラッグ方向から折り線を推定(つかんだ点と離した点の垂直二等分線、または既存の折り目・紙の縁へのスナップ)(c)離すと折れる。Shift等の修飾キーで「その点の全層」「1枚だけ」を切り替え — [証拠:M2.T2-6c.C02](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6c-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6c.C02 evidence=MANUAL.M2.T2-6c.C02.SCREEN-ACCEPTANCE -->
- [ ] **実行前プレビュー(UI-008)**: ドラッグ中に折った結果の形を半透明で重ねて表示。動く層を色分け、折り線を明示。プレビューは `flat_motion` を実際に呼んで得た結果を使う(見た目と実際が食い違わないようにする) — [証拠:M2.T2-6c.C03](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6c-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6c.C03 evidence=MANUAL.M2.T2-6c.C03.SCREEN-ACCEPTANCE -->
- [ ] **状態の可視化(UI-009)**: 3Dビュー上部に現在のモードと操作ヒントを1行で常時表示(例「紙をドラッグすると折れます / Shiftで1枚だけ」)。できない状態では理由を表示(例「折り途中では折れません。手順の最後に戻してください」)。ボタンは無効化しても消さず、理由をツールチップに出す — [証拠:M2.T2-6c.C04](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6c-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6c.C04 evidence=MANUAL.M2.T2-6c.C04.SCREEN-ACCEPTANCE -->
- [ ] **技法の選び方を簡素化**: 技法サブメニューを常時表示のパレットにせず、**つかんで動かした結果に応じて自動判定**した技法名を手順に記録する。手動で技法を指定したい場合のみサブメニューを使う — [証拠:M2.T2-6c.C05](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6c-c05) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6c.C05 evidence=MANUAL.M2.T2-6c.C05.SCREEN-ACCEPTANCE -->
- [ ] 既存の「折り線を引いてパネルで確定」する操作は残す(細かい指定をしたいとき用)が、主操作ではなくする — [証拠:M2.T2-6c.C06](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6c-c06) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6c.C06 evidence=TEST.M2.T2-6c.C06 -->
- [ ] **DOM環境のテスト基盤を導入**(jsdom + @testing-library/react)。プレビュー・ヒント表示・ドラッグ操作の主要経路にテストを付ける(これまで目視確認のみだった領域) — [証拠:M2.T2-6c.C07](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6c-c07) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6c.C07 evidence=MANUAL.M2.T2-6c.C07.SCREEN-ACCEPTANCE -->
- [ ] 実機確認: **説明なしで座布団折り→鶴の基本形まで折れるか**を操作しながら確認し、詰まった箇所をprogress.mdに記録 — [証拠:M2.T2-6c.C08](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6c-c08) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6c.C08 evidence=MANUAL.M2.T2-6c.C08.SCREEN-ACCEPTANCE -->
- [ ] コミット `紙をつかんで動かす直感的な折り操作に変更` → プッシュ — [証拠:M2.T2-6c.C09](traceability/roadmap-links.md#roadmap-evidence-m2-t2-6c-c09) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-6c.C09 evidence=MANUAL.M2.T2-6c.C09.COMMIT-PUSH -->

### Task 2-7: 作図補助・局所平坦判定・めり込み警告

**Files:** `crates/ori3-cp/src/{construct,flatfold}.rs`, `crates/ori3-rigid/src/lib.rs`(交差検査), 各tests

- [ ] 作図補助(テスト先行): `bisector(角の3点)` / `perpendicular(点, 辺)` / `divide_points(辺, n)` / `direction_lines(点, 22.5°刻み)`。ツールレールのサブメニューから利用 — [証拠:M2.T2-7.C01](traceability/roadmap-links.md#roadmap-evidence-m2-t2-7-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-7.C01 evidence=MANUAL.M2.T2-7.C01.SCREEN-ACCEPTANCE -->
- [ ] 局所平坦判定: 内部頂点ごとに前川(山−谷=±2)・川崎(交互角和=180°)を検査し違反頂点を返す→2Dで橙色表示(CPE-009) — [証拠:M2.T2-7.C02](traceability/roadmap-links.md#roadmap-evidence-m2-t2-7-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-7.C02 evidence=MANUAL.M2.T2-7.C02.SCREEN-ACCEPTANCE -->
- [ ] めり込み簡易警告: Frame3Dの面ペアの三角形交差を総当たり検査(面数400まで想定、rayonで並列化)→交差ありなら3Dビューに警告バッジ(SIM-007) — [証拠:M2.T2-7.C03](traceability/roadmap-links.md#roadmap-evidence-m2-t2-7-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-7.C03 evidence=MANUAL.M2.T2-7.C03.SCREEN-ACCEPTANCE -->
- [ ] テスト成功確認 → コミット `作図の補助線・折りたたみ可否の注意表示・紙のめり込み警告を追加` → プッシュ — [証拠:M2.T2-7.C04](traceability/roadmap-links.md#roadmap-evidence-m2-t2-7-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-7.C04 evidence=MANUAL.M2.T2-7.C04.COMMIT-PUSH -->

### Task 2-8: 自動保存と復旧

**Files:** `apps/desktop/src-tauri/src/autosave.rs`, `dialogs/RecoveryDialog.tsx`

- [ ] 30秒間隔+dirty時のみ`<保存先>.ori3.autosave`へ保存。正常終了時に削除 — [証拠:M2.T2-8.C01](traceability/roadmap-links.md#roadmap-evidence-m2-t2-8-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-8.C01 evidence=TEST.M2.T2-8.C01 -->
- [ ] 起動時`recovery_check`でautosaveの有無を返し、あれば復旧ダイアログ(復元する/破棄する) — [証拠:M2.T2-8.C02](traceability/roadmap-links.md#roadmap-evidence-m2-t2-8-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-8.C02 evidence=MANUAL.M2.T2-8.C02.SCREEN-ACCEPTANCE -->
- [ ] storeユニットテスト+手動確認(プロセスkill→再起動→復元) → コミット `30秒ごとの自動保存と、異常終了後の復元機能を追加` → プッシュ — [証拠:M2.T2-8.C03](traceability/roadmap-links.md#roadmap-evidence-m2-t2-8-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-8.C03 evidence=MANUAL.M2.T2-8.C03.COMMIT-PUSH -->

### Task 2-9: M2受け入れ(折り鶴)

**Files:** `crates/ori3-layers/tests/acceptance_crane.rs`

- [x] 折り鶴を「fold_through+技法マクロの列」でスクリプト構築し、最終状態の層数・外形寸法・決定性を検証する回帰テスト — [証拠:M2.T2-9.C01](traceability/roadmap-links.md#roadmap-evidence-m2-t2-9-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-9.C01 evidence=TEST.M2.T2-9.C01 --> <!-- 実行確認: completed_crane_is_flat_and_symmetric (1 passed; 0 failed) -->
- [ ] 手動確認: アプリで鶴を1折りずつ折って完成→展開図の一部を修正→自動再生で形が追従 — [証拠:M2.T2-9.C02](traceability/roadmap-links.md#roadmap-evidence-m2-t2-9-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-9.C02 evidence=MANUAL.M2.T2-9.C02.SCREEN-ACCEPTANCE -->
- [ ] コミット `折り鶴が折れることを確認する自動テストを追加` → プッシュ — [証拠:M2.T2-9.C03](traceability/roadmap-links.md#roadmap-evidence-m2-t2-9-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M2.T2-9.C03 evidence=MANUAL.M2.T2-9.C03.COMMIT-PUSH -->

## M3: 展開図自動提案(受け入れ: 頭1・尾1・足4の骨格)

### Task 3-1: 骨格モデル

**Files:** `crates/ori3-propose/src/skeleton.rs`, `tests/skeleton.rs`

```rust
pub struct SkeletonNode { pub id: u32, pub parent: Option<u32>, pub length: f64, pub width_factor: f64 }
pub struct Skeleton { pub nodes: Vec<SkeletonNode> }   // 根1つの木。葉=角(頭・尾・足)
impl Skeleton {
    pub fn validate(&self) -> Result<(), String>;      // 木であること・葉1〜12・length>0
    pub fn leaves(&self) -> Vec<u32>;
}
```

- [ ] テスト(validate正常系/異常系)→実装→コミット `頭・尾・足などの骨格を指定するためのデータ形式を追加` → プッシュ — [証拠:M3.T3-1.C01](traceability/roadmap-links.md#roadmap-evidence-m3-t3-1-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-1.C01 evidence=MANUAL.M3.T3-1.C01.COMMIT-PUSH -->

### Task 3-2: 円・川充填の数値最適化

**Files:** `crates/ori3-propose/src/packing.rs`, `tests/packing.rs`

- [x] テスト: (a)葉2(長さ1,1)を1×1紙に充填→縮尺≥0.5に到達 (b)葉5の充填で円非重複と中心包含の違反がEPS以内 (c)同一シード→同一結果(決定性)。既存検査は緩めない。紙内包含は「円の中心が紙内」(案A)を最終要件として確定した(判断4、2026-08-16) — [証拠:M3.T3-2.C01](traceability/roadmap-links.md#roadmap-evidence-m3-t3-2-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-2.C01 evidence=TEST.M3.T3-2.C01 -->
- [x] 実装: 変数=各葉の円中心+縮尺s。目的=s最大化。制約=|ci−cj| ≥ s·(li+lj+川幅)。紙内包含は「円の中心が紙内」(案A)を確定要件とする。円そのものが紙からはみ出すことは制約としない。射影勾配法(制約違反を射影で戻す)×乱数シード別マルチスタート(既定8スタート、上位4候補を返す)。`rand::rngs::StdRng::seed_from_u64`で決定的に — [証拠:M3.T3-2.C02](traceability/roadmap-links.md#roadmap-evidence-m3-t3-2-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-2.C02 evidence=TEST.M3.T3-2.C02 -->
- [ ] テスト成功確認 → コミット `骨格に合わせて紙の上に必要な領域を自動配置する計算を追加` → プッシュ — [証拠:M3.T3-2.C03](traceability/roadmap-links.md#roadmap-evidence-m3-t3-2-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-2.C03 evidence=MANUAL.M3.T3-2.C03.COMMIT-PUSH -->

### Task 3-3: 展開図生成(充填→分子→折り線)

**Files:** `crates/ori3-propose/src/generate.rs`, `tests/generate.rs`

- [ ] テスト: 葉4+胴1の充填結果から生成したCPが (a)妥当な平面グラフ(extract_faces成功) (b)軸線・稜線が揃い、局所平坦判定の違反頂点数を結果として返す — [証拠:M3.T3-3.C01](traceability/roadmap-links.md#roadmap-evidence-m3-t3-3-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-3.C01 evidence=TEST.M3.T3-3.C01 -->
- [ ] 実装手順: 円中心のドロネー三角形分割→各三角形をウサギ耳分子(3辺の二等分線+垂線)で充填→四角形以上は扇状分割→山谷割り当て(軸線=谷基調、稜線=山基調の既定則)→`ProposalResult { cp: CreasePattern, violations: usize }` — [証拠:M3.T3-3.C02](traceability/roadmap-links.md#roadmap-evidence-m3-t3-3-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-3.C02 evidence=TEST.M3.T3-3.C02 -->
- [ ] `proposal_generate`コマンド追加(Skeleton→候補最大4件のVec<ProposalResult>) — [証拠:M3.T3-3.C03](traceability/roadmap-links.md#roadmap-evidence-m3-t3-3-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-3.C03 evidence=TEST.M3.T3-3.C03 -->
- [ ] テスト成功確認 → コミット `自動配置の結果から展開図を組み立てる機能を追加` → プッシュ — [証拠:M3.T3-3.C04](traceability/roadmap-links.md#roadmap-evidence-m3-t3-3-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-3.C04 evidence=MANUAL.M3.T3-3.C04.COMMIT-PUSH -->

### Task 3-4: 提案ウィザードUI

**Files:** `apps/desktop/src/components/dialogs/ProposalWizard.tsx`

- [ ] 3画面構成: ①骨格編集(角の追加/削除ボタン+各角の長さ・太さスライダー+2D骨格プレビュー) ②候補選択(生成4候補の展開図サムネイル+違反数表示) ③確認→`edit_apply ReplaceCreasePattern`で流し込み、ダイアログを閉じる — [証拠:M3.T3-4.C01](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C01 evidence=MANUAL.M3.T3-4.C01.SCREEN-ACCEPTANCE -->
- [ ] ツールバーの「提案ウィザード」ボタンから起動。メイン画面に常設UIを追加しない(PRO-004) — [証拠:M3.T3-4.C02](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C02 evidence=MANUAL.M3.T3-4.C02.SCREEN-ACCEPTANCE -->
- [x] 選んだ展開図を使う前に、今の作品の折り手順が1件以上なら全て消えることと件数を確認画面内で伝える。0件では注意を出さず、注意があっても「この展開図を使う」と「選び直す」の両方を操作できる。0件/1件/100件の3/3表示と、選び直した場合の作品差分0を画面検査で固定する — [証拠:M3.T3-4.C03](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C03 evidence=MANUAL.M3.T3-4.C03.SCREEN-ACCEPTANCE -->
- [ ] 手動確認: 頭1・尾1・足4で鶴系の基本形が得られ、そのまま編集・折りに進める(M3受け入れ) → コミット `骨格を指定して展開図を提案してもらう画面を追加` → プッシュ — [証拠:M3.T3-4.C04](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C04 evidence=MANUAL.M3.T3-4.C04.COMMIT-PUSH -->

## M3強化: 完成形の位置指定と完成までの折り方(2026-08-14追加)

### 作業1: 先行設計判断の記録

- [x] **判断1 / PRO-006**: 完成位置は2D投影上の相対位置とし、今回の入力に奥行きを含めない。奥行き操作は説明なしで扱いにくいためで、入力型は2Dだけを持たせ、将来の3D形式を別に追加できる境界にする — [証拠:M3.T3-4.C05](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c05) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C05 evidence=TEST.M3.T3-4.C05 -->
- [x] **判断2 / PRO-007**: 「位置」は完成した動物で頭・尾・足などの先端がどこにあるかを意味する。展開前の紙上の割当位置ではない。利用者が指定したいのは完成した動物で頭・尾・足がどこにあるかだからである — [証拠:M3.T3-4.C06](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c06) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C06 evidence=TEST.M3.T3-4.C06 -->
- [x] **判断3 / PRO-008**: 完成形の絵で先端を直接動かすことを主にし、必要なときだけ紙上位置も調整する案Cへ2026-08-21に拡張した。動物の形を伝える意図を保ちながら、紙の使い方も調整できるため。2種類の位置の使い分けは作業13の規則に従う — [証拠:M3.T3-4.C07](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c07) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C07 evidence=TEST.M3.T3-4.C07 -->
- [x] **判断5 / PRO-009**: 手順計画は配置案P1として`crates/ori3-propose`内に置き、新クレートは作らない。新クレートは`Cargo.toml`・`Cargo.lock`の変更と承認を要するため、まず既存クレート内で作り、分離の必要性が実測で示された場合だけ再検討する。P1で`ori3-layers`依存を追加する場合の承認は別途必要 — [証拠:M3.T3-4.C08](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c08) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C08 evidence=TEST.M3.T3-4.C08 -->
  - **現状(2026-08-16)**: 作業17(追跡情報)・作業18(方式の比較)・作業20(完成の目標の4指標)まで、`Cargo.toml`・`Cargo.lock`・`vendor/`の変更は**0行**で済んでいる。折り順の探し方の比較は「どの手をどの順に並べられるか」の数え上げで、展開図(`ori3-cp`/`ori3-model`)だけで測れたため。**依存の追加は要らなかったので、承認の要求も0件。** `ori3-layers`が要るのは「実際に折れるか」を確かめる作業21以降で、そのときに§5のとおり事前に差分案を出す
- [x] **判断4**: 紙内包含は「円の中心が紙内にあればよい」(案A、現行実装のまま)に決定した(2026-08-16)。実際に折った鶴・カエルの基本形(計9本)で出っぱりの長さを測ると、円が紙をはみ出しても不足は9本すべて0(最大差6.8e-15)。案B(円全体包含)は鶴の基本形の配置を作れず、作れる場合も同じ紙でできる作品が17〜59%小さくなる。詳細は作業7と`scratchpad/containment-report.md` — [証拠:M3.T3-4.C09](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c09) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C09 evidence=TEST.M3.T3-4.C09 -->
- [x] **判断6**: 折り順は**任意の展開図の汎用探索を土台にし、提案が作った展開図で追跡情報があるときだけ枝を絞る**形に決定した(2026-08-16)。生成履歴だけに頼る方式は採らない — [証拠:M3.T3-4.C10](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c10) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C10 evidence=TEST.M3.T3-4.C10 -->
  - **根拠1(広さ)**: 同じ4つの展開図で測ったところ、汎用探索は**4/4**で次の手を列挙して最後まで折り切る順番を見つけたのに対し、生成履歴の方式は**2/4**だった。出せなかったのは提案が作ったものではない展開図で、**折り鶴では折り線のまとまり34本すべてに役目が付かず、1手も出せない**(最初の手0・たどった状態1)。やっこさんも同じで16本すべてが役目なし。追跡情報は提案の生成器が作るものなので、**読み込んだ展開図・手で描いた展開図・提案の展開図を利用者が編集した後**の3つでは存在しない
  - **根拠2(弱点の性質)**: 汎用探索の弱点は枝が広がることで、最初の4手までに行ける状態は最大**13,554**(頭1尾1足4、47ms)、折り鶴で**5,327**(23ms)だった。これは打ち切りと枝の絞り込みで扱える種類の問題である。一方、生成履歴側の弱点(追跡情報が無いと**0手**)は探索の工夫では埋まらない
  - **根拠3(要件)**: `CLAUDE.md` §8「実際の紙で折れる操作はすべてアプリで表現できなければならない」に照らすと、利用者の展開図で1手も動かない方式は採れない
  - **注意点1**: 汎用探索が4/4で通ったのは**規則をゆるめた結果**である。4標本で折り目の集まる点に条件を課したのは合計31点、そのうち**3点(9.7%)**は「どの組も作れない」状態として妨げから外している(頭1尾1足4で2点、折り鶴で1点)。ゆるめを外せば同じ標本でも行き詰まりうるので、**「汎用探索なら何でも折れる」とは読めない**
  - **注意点2**: 生成履歴側にだけ「1か所から外へ広げる」制約が入っており、**枝分かれの小ささ(深さ4で9状態)はその制約の影響かもしれない**。同じ制約を汎用探索へ入れた比較は未実施である
  - **注意点3**: 今回の測定は**紙の重なり順・面のめり込み・折る途中の姿勢を一切見ていない**。数えた手はすべて上限側の見積もりで、実際に折れるかの判定は作業21で入れる
  - 測定値は`crates/ori3-propose/tests/plan.rs`の出力(表A・B・C)。標本4は2026-08-16の利用者指示で「悪魔 手順24」から「やっこさん」へ差し替えており、判断の根拠には折り鶴・やっこさんの値を使っている

未決定の判断は0件になった。配置案P1の決定は、依存関係変更の承認を兼ねない。

| 要件ID | 受け入れ条件 |
|---|---|
| PRO-006 | 1〜12葉の位置入力が有限な2成分だけを持ち、奥行き成分を持たない。RustのJSON往復1件と画面側で同じ固定JSONを読む1件の2/2で一致し、Rust往復の絶対誤差は`<=1e-12`。将来3D用の別形式を加えても既存2D入力を再解釈しない |
| PRO-007 | 1〜12葉の各葉IDが完成形の測定点へちょうど1回対応する。完成位置を0.1動かした固定例で完成位置の評価値が`>1e-4`変わり、紙上の充填中心を利用者の完成位置入力として扱う経路は0件 |
| PRO-008 | 完成形プレビューの最大12個の先端をpointer操作20件・keyboard操作20件の40/40で動かせ、期待位置との差が`<=1e-6`。常設区画増加0、1000×700で横はみ出し`<=0px`、利用者向け表示の内部用語0件。紙上位置も別に調整でき、2種類の位置は作業13の規則で使い分ける |
| PRO-009 | 手順計画の製品コードは`crates/ori3-propose`内にあり、新規クレートとworkspace memberの追加0件、Tauriホスト内の探索本体0件。依存変更が必要な場合の事前承認1件を記録する。折り方は、作品を問わず候補を探し、提案が作った展開図で対応情報があるときだけ候補を絞る |

### 作業6: 配置品質の基準測定(判断4の前提)

- [x] 製品実装を変えず、seed 1でstarts=`1/8/16/32/64`の5実行と、starts 8でseed=`0..999`の1,000実行、合計1,005実行(重複を除く1,004組)を行う — [証拠:M3.T3-4.C11](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c11) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C11 evidence=TEST.M3.T3-4.C11 -->
- [x] 全1,005実行の出力について、縮尺・違反量・全中心座標が有限で、葉IDと中心の欠損が0であることを検査する。各seedの最良縮尺のmin/p50/p95/max、実行時間、4候補の重複率を`docs/progress.md`へ記録する。重複は同じ葉IDの全中心のユークリッド距離が`<=1e-7`と定義する — [証拠:M3.T3-4.C12](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c12) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C12 evidence=TEST.M3.T3-4.C12 -->
- [x] 中心だけの包含での12葉下限`0.194277036`と、円全体の包含での4×3格子下限`0.124999999`を固定配置から各1件検算する。**訂正(2026-08-16)**: 円全体包含は本測定で実測`0.139958812`(格子下限の1.1197倍)まで届くことを確認した。案Bは不採用のため、`0.124999999`は「採らなかった案の実行可能性の記録」として残す — [証拠:M3.T3-4.C13](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c13) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C13 evidence=TEST.M3.T3-4.C13 -->
- [x] 測定完了だけで包含方式を自動決定しない。判断4として中心包含/円全体包含のどちらを採るかを別途決め、要件・ロードマップ・検査・実装を同じ意味へそろえる → 作業7で決定した — [証拠:M3.T3-4.C14](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c14) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C14 evidence=TEST.M3.T3-4.C14 -->

### 作業7: 紙内包含の方式決定(判断4・完了、2026-08-16)

- [x] `crates/ori3-layers`の折り操作だけで実際に折った鶴の基本形(8手)・カエルの基本形(9手)を使い、平らな折り上がりの上で「軸の根元から出っぱりの先端までの距離」を測った。9本すべてで要求した長さ(=円の半径)に対する不足が0(最大差6.8e-15)だった — [証拠:M3.T3-4.C15](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c15) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C15 evidence=TEST.M3.T3-4.C15 -->
- [x] はみ出し量だけを0%→75%、はみ出す面積を5.83倍に変えて測り直したが、不足は0のまま変わらなかった。「円が紙をはみ出す→出っぱりが短くなる」という因果は測定で否定された — [証拠:M3.T3-4.C16](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c16) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C16 evidence=TEST.M3.T3-4.C16 -->
- [x] 同じ標本(骨格4種・合計1,600件、乱数シード列・探索手順は共通、紙内包含の判定だけを替えた)で案A/案Bの最良縮尺を比較した。案Bは鶴の基本形の配置自体を作れず(羽の円の半径0.7071が紙に収まる最大半径0.5を超える)、作れる場合も同じ紙でできる作品が17〜59%小さくなる(必要な紙の面積は1.47〜5.83倍) — [証拠:M3.T3-4.C17](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c17) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C17 evidence=TEST.M3.T3-4.C17 -->
- [x] **判断4を決定した**: 紙内包含は「円の中心が紙内にあればよい」(案A)とする。円そのものが紙からはみ出すことは制約としない。理由は上の3点(不足0の実測、案Bでは鶴の基本形が配置不能、案Bは作品を17〜59%小さくする) — [証拠:M3.T3-4.C18](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c18) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C18 evidence=TEST.M3.T3-4.C18 -->
- [x] 案Aを正としたことに伴い、円が紙からはみ出すことだけを理由にした「◯◯が紙からはみ出しています」警告(`crates/ori3-propose/src/generate.rs`)は誤警告になるため、円の中心が紙内にある限り出さないよう修正した。中心そのものが紙の外に出る場合(案Aの制約が破れる異常な入力。通常`pack()`からは起きない)は、展開図が壊れうる本当の問題として引き続き警告する — [証拠:M3.T3-4.C19](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c19) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C19 evidence=TEST.M3.T3-4.C19 -->
- [x] 測定記録は`scratchpad/containment-report.md`(製品コード変更0行の測定フェーズ)。判断の確定と警告修正は本ロードマップと`crates/ori3-propose/src/generate.rs`で行った(実装フェーズ) — [証拠:M3.T3-4.C20](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c20) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C20 evidence=TEST.M3.T3-4.C20 -->

### 作業13: 2種類の位置を統合(決定済み、2026-08-21)

- [x] 完成形の位置と紙の上の位置は先端ごとに独立して判定する。完成形だけなら完成形の位置、紙の上だけなら紙の上の位置、両方が食い違わなければ紙の上の位置を使う。 — [証拠:M3.T3-4.C21](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c21) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C21 evidence=TEST.M3.T3-4.C21 -->
- [x] 両方が食い違う先端では、利用者が最後に動かしたほうを使う。食い違いの許容差は`0.0032`とし、紙の上の編集画面でつまみを1画素動かした実測値`0.004`の約8割を採った。根拠は`CLAUDE.md` §10.7.9に従って要件書にも記録する。 — [証拠:M3.T3-4.C22](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c22) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C22 evidence=MANUAL.M3.T3-4.C22.SCREEN-ACCEPTANCE -->
- [x] 各先端で使っている位置を画面に示し、使われていないほうへ戻す操作を置く。片方を動かしても別の先端の指定を失わず、動かしたつまみが反応しない状態を作らない。 — [証拠:M3.T3-4.C23](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c23) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C23 evidence=MANUAL.M3.T3-4.C23.SCREEN-ACCEPTANCE -->

### 作業14・15: 分割方式の比較測定と判断(完了)

- [x] 頭1・尾1・足4(長さ`1,1,0.7×4`、紙1×1、`starts=8`)の固定seed `2026` 1件とseed `0..99` 100件、計101件で、現方式と接触関係を使う試作方式の局所違反・3叉・川崎残差max/中央値・時間を比較した — [証拠:M3.T3-4.C24](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c24) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C24 evidence=TEST.M3.T3-4.C24 -->
- [x] 試作方式は採用せず、現方式を維持する。局所違反と3叉は100/100件で同一、固定例の品質3指標も同一で、時間は82/100件で現方式が速く、測定で変更の利点が示されなかったためである — [証拠:M3.T3-4.C25](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c25) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C25 evidence=TEST.M3.T3-4.C25 -->
- [x] 今後の案は、同じ100件・同じ入力・同じ計測範囲で次の4条件をすべて満たした場合だけ採用する — [証拠:M3.T3-4.C26](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c26) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C26 evidence=TEST.M3.T3-4.C26 -->

| 指標 | 採用条件 |
|---|---|
| 局所違反の数 | seed `0..99` の100件で現方式より悪化0件、かつ現方式より10件以上で改善 |
| 3叉の数 | 100件で現方式より悪化0件 |
| 川崎残差 max | 100/100件で、それぞれ現方式の値以下 |
| 時間 | 100件の中央値が、同一実行で測った現方式の中央値の1.2倍以内 |

100件すべて非悪化だけでは採用条件を満たさない。局所違反10件以上の改善を必須とする。現方式を選んだため作業16の製品実装は行わず、上の4条件を満たす別案が測定された場合だけ再開する。判断6は2026-08-16に決定したので、未決定の判断は0件である。

### 作業18: 折り順の探し方を比較するスパイク(判断6の前提)

- [ ] 配置案P1の`crates/ori3-propose`内で、生成履歴方式と汎用探索方式の両方を同じ固定4展開図へ試す。新クレートは作らない — [証拠:M3.T3-4.C27](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c27) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C27 evidence=TEST.M3.T3-4.C27 -->
- [ ] 両方式が固定4展開図の4/4で次手候補を1件以上列挙し、展開状態数・最大分岐数・時間を方式×4件の24/24値で記録する。この作業では方式を決定しない — [証拠:M3.T3-4.C28](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c28) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C28 evidence=TEST.M3.T3-4.C28 -->
- [ ] `ori3-layers`への依存追加が必要な場合は、`crates/ori3-propose/Cargo.toml`と`Cargo.lock`を変更する前に承認を得る — [証拠:M3.T3-4.C29](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c29) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C29 evidence=TEST.M3.T3-4.C29 -->

### 作業19: 折り順の方式決定(判断6・完了、2026-08-16)

- [x] 作業18の測定を根拠に、**汎用探索を土台にし、追跡情報があるときだけ枝を絞る**方式へ決定した。根拠と注意点3つは上の判断6の欄に数値つきで記した — [証拠:M3.T3-4.C30](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c30) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C30 evidence=TEST.M3.T3-4.C30 -->
- [x] 生成履歴だけの方式を採らない理由を、検査として残した(`crates/ori3-propose/tests/plan.rs`の`the_history_plan_only_works_on_crease_patterns_the_proposal_made`)。提案が作った展開図では役目なし0本で最後まで到達し、作品の展開図(折り鶴34本・やっこさん16本)では最初の手が0であることを固定する — [証拠:M3.T3-4.C31](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c31) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C31 evidence=TEST.M3.T3-4.C31 -->
- [x] 標本4を「悪魔 手順24」から「やっこさん」へ差し替え、`tests/fixtures/cp-devil-024.json`を削除した(2026-08-16 利用者指示)。判断6の根拠に悪魔・ローズの数値は使わない — [証拠:M3.T3-4.C32](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c32) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C32 evidence=TEST.M3.T3-4.C32 -->

### 作業20: 完成の目標の4指標(完了、2026-08-16)

**Files:** `crates/ori3-propose/src/finish.rs`(新規), `tests/finish.rs`(新規)

- [x] 利用者が指定する**角の数・長さ・太さ・位置**の4つについて、「いまの形が完成形へどれだけ近づいたか」を測る物差しを1つずつ作る。`count_gap` / `length_gap` / `width_gap` / `position_gap` はどれも**0.0が最良**で、互いに独立して単独で使える。**4つを1つの順位へまとめる重み付けは作業22で決める**(`finish_gaps`は並べて返すだけで合成しない) — [証拠:M3.T3-4.C33](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c33) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C33 evidence=TEST.M3.T3-4.C33 -->
- [x] 位置は作業5の`TipPos2d`(完成形での先端の位置)を使い、先端と紙の場所の対応は作業9の`LeafSite`をそのまま使う。座標を突き合わせて後から推測する経路は0件。紙の上の充填中心を利用者の完成位置として扱う経路も0件(PRO-007) — [証拠:M3.T3-4.C34](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c34) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C34 evidence=TEST.M3.T3-4.C34 -->
- [x] 位置を指定していない先端は位置の物差しの母数から外し、指定があるのに測っていない先端はいちばん遠い(1.0)として数える。理由は`scratchpad/propose-20-report.md`と`finish.rs`のdocに記した — [証拠:M3.T3-4.C35](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c35) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C35 evidence=TEST.M3.T3-4.C35 -->
- [x] 葉1〜12本の12通りで4つとも有限(NaN・無限大0件)、完成形そのものを入れると4つとも0.0(12/12)、角を1本減らす・長さを半分・太さを半分・位置を0.1ずらすの4通りで対応する物差しが悪化、同じ入力10回で同じ値(10/10)、既存の提案結果は`tests/fixtures/cp-baseline-1-12.json`と12/12で一致 — [証拠:M3.T3-4.C36](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c36) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C36 evidence=TEST.M3.T3-4.C36 -->
- [x] 折り順の探索そのものはここでは作らない(作業21・22)。UI・常設区画・設定項目の追加0件 — [証拠:M3.T3-4.C37](traceability/roadmap-links.md#roadmap-evidence-m3-t3-4-c37) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M3.T3-4.C37 evidence=MANUAL.M3.T3-4.C37.SCREEN-ACCEPTANCE -->

### M3強化の実装順

```text
要件・判断:       作業1 → 作業6 → 作業7(包含を決定・完了) → 作業8(配置探索を実装)
完成位置(UI案A): 作業4 → 作業5(2D位置契約) → 作業11(直接操作)
案Cの位置調整:     作業10(紙上位置計算) → 作業12(任意の紙上調整) → 作業13(2種類の位置を統合)
折り線分割:       作業9 → 作業14(比較) → 作業15(現方式を維持) → 作業16(4条件を満たす別案が出た場合だけ)
手順計画(P1):    作業17 → 作業18(両方式を比較) → 作業19(方式を決定) → 作業20以降
```

同じファイルを触る作業は上記順序で直列に行う。作業番号と各作業の全数値条件は`scratchpad/propose-design-report.md` §9を正とし、作業18前に折り順方式を先取りしない。

## M4: 複雑技法 + 書き出し(受け入れ: 伝承のカエル)

### Task 4-1: 沈め折り(open sink)

**Files:** `crates/ori3-layers/src/techniques.rs`(追加), `tests/sink.rs`

- [x] テスト: 鶴の基本形の頂点を沈める→対象領域の全層で山谷が反転し、層順序が沈め込み後の入れ子順になる — [証拠:M4.T4-1.C01](traceability/roadmap-links.md#roadmap-evidence-m4-t4-1-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-1.C01 evidence=TEST.M4.T4-1.C01 --> <!-- 実行確認: open_sink_works_on_the_bird_base_apex (1 passed; 0 failed) -->
- [x] 実装: `pub fn open_sink(cp, faces, state, region_line: [[f64;2];2]) -> Result<FoldThroughResult, String>`。折り線より先端側の全層について、(a)折り線で各層を分割 (b)先端側の山谷を反転 (c)層順序を内外反転して再挿入 — [証拠:M4.T4-1.C02](traceability/roadmap-links.md#roadmap-evidence-m4-t4-1-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-1.C02 evidence=TEST.M4.T4-1.C02 --> <!-- 実行確認: open_sink_turns_the_tip_of_the_preliminary_base_inside_out (1 passed; 0 failed) -->
- [ ] テスト成功確認 → コミット `沈め折りを選ぶだけで折れる機能を追加` → プッシュ — [証拠:M4.T4-1.C03](traceability/roadmap-links.md#roadmap-evidence-m4-t4-1-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-1.C03 evidence=MANUAL.M4.T4-1.C03.COMMIT-PUSH -->

### Task 4-2: ひだ寄せ・ねじり折り

**Files:** `crates/ori3-layers/src/techniques.rs`(追加), `tests/{swivel,twist}.rs`

- [x] `pub fn swivel(...)`: 基準線+寄せ線の2線指定でひだを寄せる(fold_through2回+層併合) — [証拠:M4.T4-2.C01](traceability/roadmap-links.md#roadmap-evidence-m4-t4-2-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-2.C01 evidence=TEST.M4.T4-2.C01 --> <!-- 実行確認: swivel_works_on_stacked_layers_and_rejects_only_undefined_input (1 passed; 0 failed) -->
- [x] `pub fn twist(...)`: 多角形領域+周辺ひだ線の指定でねじる(領域回転配置+周辺ひだのfold_through列) — [証拠:M4.T4-2.C02](traceability/roadmap-links.md#roadmap-evidence-m4-t4-2-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-2.C02 evidence=TEST.M4.T4-2.C02 --> <!-- 実行確認: twist_works_on_a_triangle_and_rejects_only_undefined_input (1 passed; 0 failed) -->
- [ ] 各テスト(層数・順序・CP追加線の検証)→実装→コミット `ひだ寄せとねじり折りを選ぶだけで折れる機能を追加` → プッシュ — [証拠:M4.T4-2.C03](traceability/roadmap-links.md#roadmap-evidence-m4-t4-2-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-2.C03 evidence=MANUAL.M4.T4-2.C03.COMMIT-PUSH -->

### Task 4-3: 展開図SVG/PNG書き出し

**Files:** `crates/ori3-export/src/{cp_svg,cp_png}.rs`, `tests/export_cp.rs`, `dialogs/ExportDialog.tsx`

- [ ] テスト: 生成SVGに線種別スタイル(山=一点鎖線/谷=破線/輪郭=実線)が含まれ、viewBoxが実寸mm。PNGが指定解像度で非空 — [証拠:M4.T4-3.C01](traceability/roadmap-links.md#roadmap-evidence-m4-t4-3-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-3.C01 evidence=TEST.M4.T4-3.C01 -->
- [ ] 実装: SVGは文字列組み立て(`svg`クレート可)。PNGはresvgでSVGをラスタライズ。`document_export`コマンド追加(種別enum: CpSvg/CpPng/DiagramPdf/DiagramSvg)+書き出しダイアログ(補助線含む/含まない、PNG解像度) — [証拠:M4.T4-3.C02](traceability/roadmap-links.md#roadmap-evidence-m4-t4-3-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-3.C02 evidence=MANUAL.M4.T4-3.C02.SCREEN-ACCEPTANCE -->
- [ ] テスト成功確認 → コミット `展開図を画像ファイルとして保存する機能を追加` → プッシュ — [証拠:M4.T4-3.C03](traceability/roadmap-links.md#roadmap-evidence-m4-t4-3-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-3.C03 evidence=MANUAL.M4.T4-3.C03.COMMIT-PUSH -->

### Task 4-4: 折り図レンダラ(ステップ図)

**Files:** `crates/ori3-export/src/diagram.rs`, `tests/diagram.rs`

- [ ] テスト: 3ステップの手順から3コマのSVGが生成され、各コマに(a)折る前の平坦状態の正射影(可視輪郭+可視折線) (b)今回の折り線(山=一点鎖線/谷=破線) (c)技法別矢印(TechniqueKindごとに固定の記号パス)が含まれる — [証拠:M4.T4-4.C01](traceability/roadmap-links.md#roadmap-evidence-m4-t4-4-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-4.C01 evidence=TEST.M4.T4-4.C01 -->
- [ ] 実装: `pub fn render_step(doc: &Document, step_index: usize) -> String /* SVG */`。投影は該当ステップ直前のFlatStateの最上層から可視面を層順に描画。矢印記号はTechniqueKind→固定SVGパスのテーブル(谷矢印/山矢印/中割り/かぶせ/花弁/つぶし/沈め/ひだ/ねじり/ポーズの10種) — [証拠:M4.T4-4.C02](traceability/roadmap-links.md#roadmap-evidence-m4-t4-4-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-4.C02 evidence=TEST.M4.T4-4.C02 -->
- [ ] テスト成功確認 → コミット `折り手順を1コマずつ図にする機能を追加` → プッシュ — [証拠:M4.T4-4.C03](traceability/roadmap-links.md#roadmap-evidence-m4-t4-4-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-4.C03 evidence=MANUAL.M4.T4-4.C03.COMMIT-PUSH -->

### Task 4-5: 折り図PDF/SVG組版

**Files:** `crates/ori3-export/src/pdf.rs`, `tests/pdf.rs`

- [ ] テスト: 7ステップの手順→A4・2列×3コマで2ページのPDFが生成される(ページ数・非空を検証)。SVG版はページ単位のファイル群 — [証拠:M4.T4-5.C01](traceability/roadmap-links.md#roadmap-evidence-m4-t4-5-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-5.C01 evidence=TEST.M4.T4-5.C01 -->
- [ ] 実装: render_stepのSVGをA4(210×297mm)グリッドに配置(コマ番号+注記付き)、svg2pdfでPDF化。表紙(タイトル+完成図)を1ページ目に付ける — [証拠:M4.T4-5.C02](traceability/roadmap-links.md#roadmap-evidence-m4-t4-5-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-5.C02 evidence=TEST.M4.T4-5.C02 -->
- [ ] 書き出しダイアログにDiagramPdf/DiagramSvgを接続 — [証拠:M4.T4-5.C03](traceability/roadmap-links.md#roadmap-evidence-m4-t4-5-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-5.C03 evidence=MANUAL.M4.T4-5.C03.SCREEN-ACCEPTANCE -->
- [ ] テスト成功確認 → コミット `折り図をPDFとして保存する機能を追加` → プッシュ — [証拠:M4.T4-5.C04](traceability/roadmap-links.md#roadmap-evidence-m4-t4-5-c04) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-5.C04 evidence=MANUAL.M4.T4-5.C04.COMMIT-PUSH -->

### Task 4-6: M4受け入れ(伝承のカエル)

**Files:** `crates/ori3-layers/tests/acceptance_frog.rs`

- [ ] 伝承のカエル(花弁折り・中割り折り・段折りを含む)をスクリプト構築する回帰テスト(最終層数・決定性) — [証拠:M4.T4-6.C01](traceability/roadmap-links.md#roadmap-evidence-m4-t4-6-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-6.C01 evidence=TEST.M4.T4-6.C01 -->
- [ ] 手動確認: アプリでカエルを折って完成→折り図PDFを書き出し、内容を目視確認 — [証拠:M4.T4-6.C02](traceability/roadmap-links.md#roadmap-evidence-m4-t4-6-c02) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-6.C02 evidence=MANUAL.M4.T4-6.C02.SCREEN-ACCEPTANCE -->
- [ ] コミット `伝承のカエルが折れて折り図を出せることを確認する自動テストを追加` → プッシュ — [証拠:M4.T4-6.C03](traceability/roadmap-links.md#roadmap-evidence-m4-t4-6-c03) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M4.T4-6.C03 evidence=MANUAL.M4.T4-6.C03.COMMIT-PUSH -->

## M5: 紙のたわみ表現(受け入れ: 風船・折り鶴)

`ori3-soft` は8番目の計算クレートとして、たわみ・膨らみ・層順序拘束の計算を担う。表示のオン／オフと膨らませる操作の実装経路はあるが、完成判定はSIM-012〜015の受け入れ条件で行う。

### Task 5-1: たわみの手順記録と受け入れ

- [ ] SIM-015を満たすよう、仕上げ手順ごとの有効・硬さ・膨らみの強さを記録して再生し、頂点座標を保存しないこと、風船と折り鶴の受け入れ条件を検査で固定する。現在の作品全体設定だけではこの条件を満たさない。 — [証拠:M5.T5-1.C01](traceability/roadmap-links.md#roadmap-evidence-m5-t5-1-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M5.T5-1.C01 evidence=TEST.M5.T5-1.C01 -->

## M6: ヘルプ・初回ガイド・テーマ

UI-011〜013の実装経路と画面検査はある。M6の完成判定は、F1の日本語ヘルプ、再表示可能な初回ガイド、端末ごとに復元される5テーマを、クリーンなHEADで全品質ゲートおよび受け入れ条件とともに満たした時点に行う。 — [証拠:M6.ACCEPTANCE.C01](traceability/roadmap-links.md#roadmap-evidence-m6-acceptance-c01) <!-- ORIGAMI3-ROADMAP-LINK schema=1 id=M6.ACCEPTANCE.C01 evidence=MANUAL.M6.ACCEPTANCE.C01.FULL-ACCEPTANCE -->

## 追加目標(2026-08-24承認)

利用者の承認により、次の2項目を目標へ加える。実装順と数値による受け入れ条件は`docs/improvement-roadmap-2026-08-24.md`の§12および§15を正とする。

### FOLD 1.2 限定の読み書き

- [ ] FOLD 1.2限定で、対応する展開図・折り角・重なりの情報・線形の手順を読み書きする。対応外の内容は利用者に示し、FOLD-001〜006およびM7の受け入れ基準を満たす。

### 全部の折り目を一斉に折る一時表示

- [ ] 山折り・谷折りの全折り目を共通の0〜100%で一時表示する。これは記録された手順ではないことを常に示し、手順、保存、Undo/Redoには残さない。

## 3. マイルストーン完了時の共通チェック

各M完了時に以下を実施してからプッシュする:

1. `scripts/check.ps1` 全通過
2. NFR-004に従い、新しく足したコードが1ファイルへ無計画に積み増されていないことを確認する。行数は合否条件にしない
3. IPCの実装登録と§2の一覧が現在の18個で一致し、追加分を既存の操作enumへ集約できないかを確認する
4. `docs/progress.md` に完了内容・既知の問題を3〜10行で追記(長文の経緯記録は禁止。要件定義書NFR-006)
5. 要件定義書の該当要件IDに対する充足状況を確認し、未達があればタスク化

## 4. 実装順の依存関係

```
M0 → 1-1 → 1-2 → 1-3 → 1-4 → 1-5 → 1-6 ─┐
                   └→ 1-7 → 1-8 → 1-9 ──┴→ 1-10
M2: 2-1 → 2-2 → 2-3 → 2-4 → 2-5 → 2-6 → 2-9
    (2-7, 2-8 は 2-3 完了後いつでも並行可)
M3: 3-1 → 3-2 → 3-3 → 3-4(M2完了に依存しない。M1完了後なら並行可。ただし受け入れはM2の折り操作を使う)
M3強化: 作業1 → 作業5 / 作業6 → 作業7 → 作業8 / 作業17 → 作業18 → 作業19 → 作業20以降
M4: 4-1 → 4-2 → 4-6 / 4-3 → 4-4 → 4-5(4-3系は2-3完了後なら並行可)
M5: 2-3 → 5-1
M6: M1 → UI-011〜013の受け入れ
```
