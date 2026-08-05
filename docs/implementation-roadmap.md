# ORIGAMI3 実装ロードマップ

> **実装エージェント(Codex等)へ:** 本書はタスク単位のチェックボックス(`- [ ]`)で進捗を管理する。タスクは上から順に実施する。各タスクは「テストを書く → 失敗を確認 → 実装 → 成功を確認 → コミット → プッシュ」のTDDサイクルで進めること。

**Goal:** 展開図を描きながら3Dで1折りずつ折り紙を折り、骨格指定から展開図を自動提案できるデスクトップアプリ(要件は `docs/requirements-definition.md`)。

**Architecture:** Tauri 2ホスト + React/TypeScriptフロント + Rust計算コア(cargo workspace、7クレート)。折りエンジンは「剛体折りソルバー(表示)+ 平坦状態の層モデル(記録)」のハイブリッド。3D状態は保存せず「展開図 + 折り手順」から常に再生する。

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
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cd apps/desktop; npm run build; npm run lint; npm run test; cd ../..
```

上記をまとめた `scripts/check.ps1` をM0で作成する。**検査が通らない状態でコミットしない。**

### 0.3 規律(要件定義書§2より。違反する実装はレビューで差し戻し)

- f64 + 明示的ε。厳密有理数演算・証明機構を書かない
- 失敗時は「止めずに警告」。ユーザー操作をブロックするゲートを作らない
- Tauriコマンドは本書に列挙した13個から増やさない
- 常設UI区画は4つ固定。固定パネル・常設セクションを追加しない
- Rust 50,000行 / TS 20,000行以内(テスト除く)。1ファイル1,000行以内を目安
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
│  ├─ ori3-propose/src/
│  │   ├─ lib.rs
│  │   ├─ skeleton.rs              # 骨格(木構造)モデル
│  │   ├─ packing.rs               # 円・川充填の数値最適化
│  │   └─ generate.rs              # 充填→分子→展開図
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
   │   ├─ commands.rs              # Tauriコマンド13個(全てstore/クレートへ委譲)
   │   └─ autosave.rs              # 自動保存+復旧
   └─ src/
       ├─ main.tsx / App.tsx       # 4区画レイアウトのみ(200行以内厳守)
       ├─ store/appStore.ts        # Zustandストア(唯一の状態置き場)
       ├─ ipc/client.ts            # invokeラッパー13関数(1関数=1コマンド)
       ├─ components/
       │   ├─ ToolRail.tsx         # ツールレール(ボタン10個以内)
       │   ├─ CpEditor/            # 2D展開図エディタ(Canvas 2D)
       │   ├─ Viewer3D/            # Three.js 3Dビュー + 3D上の折り線描画
       │   ├─ Timeline.tsx         # 手順タイムライン
       │   ├─ ContextPanel.tsx     # コンテキストパネル(選択対象で切替)
       │   └─ dialogs/             # 新規作成/提案ウィザード/書き出し/復旧 の4種のみ
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

### IPCコマンド一覧(13個で確定)

`document_new / document_open / document_save / document_export / edit_apply / edit_undo / edit_redo / sequence_apply / pose_solve / sequence_replay / proposal_generate / recovery_check / recovery_restore`

全コマンドの戻り値は `Result<T, String>` とし、内部panicは `std::panic::catch_unwind` で捕捉してErrに変換する(SYS-005)。

---

## M0: プロジェクト基盤

### Task 0-1: cargo workspace とクレート雛形

**Files:** `Cargo.toml`, `crates/ori3-{model,geometry,cp,rigid,layers,propose,export}/Cargo.toml`, 各 `src/lib.rs`, `.gitignore`

- [x] ルート`Cargo.toml`にworkspace(members = crates/* と apps/desktop/src-tauri)を定義
- [x] 各クレートを`cargo new --lib`で作成。依存: model(なし) / geometry(model, glam) / cp(geometry) / rigid(cp) / layers(cp) / propose(cp, rand) / export(layers, rigid, resvg, svg2pdf)
- [x] 共通依存(serde, serde_json, thiserror, glam)はworkspace.dependenciesで一元管理。バージョンは最新安定版を選び`Cargo.lock`で固定
- [x] `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings` が通ることを確認
- [x] コミット `計算部品を置くためのフォルダ構成と空の部品一式を作成` → プッシュ

### Task 0-2: Tauriアプリ雛形

**Files:** `apps/desktop/` 一式(Tauri 2 + React + TS + Viteテンプレート)

- [x] `npm create tauri-app@latest`(react-tsテンプレート)で`apps/desktop`を作成し、`three` `@types/three` `zustand` を追加
- [x] `src-tauri/Cargo.toml` をworkspaceメンバーに追加し、空の`greet`系サンプルコマンドを削除
- [x] `npm run tauri dev` でウィンドウが起動することを確認(タイトル: ORIGAMI3)
- [x] コミット `アプリの画面が起動する最小の土台を作成` → プッシュ

### Task 0-3: 検査スクリプト

**Files:** `scripts/check.ps1`

- [x] §0.2の4検査を順に実行し、いずれか失敗で非0終了するスクリプトを作成。手動実行で成功を確認
- [x] コミット `全ての自動チェックを一度に実行できる仕組みを追加` → プッシュ

## M1: 展開図エディタ + 剛体折り(受け入れ: やっこさん)

### Task 1-1: ori3-model 型定義

**Files:** `crates/ori3-model/src/lib.rs`, `tests/serde_roundtrip.rs`

- [x] テスト: `Document`を構築→JSONへserialize→deserializeで往復一致(`test_document_json_roundtrip`)。実行して失敗確認
- [x] §2の型定義を実装。`Document::new(paper: Paper) -> Document`(輪郭4辺入りのCP初期化)も実装
- [x] テスト成功確認 → コミット `作品データ(紙・展開図・折り手順)の保存形式を定義` → プッシュ

### Task 1-2: ori3-geometry 幾何プリミティブ

**Files:** `crates/ori3-geometry/src/{lib,primitives,isometry}.rs`, `tests/primitives.rs`

- [x] テストを先に書く: 交差あり/なし/平行/端点接触の`seg_intersection`、`point_on_segment`、`reflect_across_line`(点(1,0)を直線x=0で鏡映→(-1,0))
- [x] 実装:

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

- [x] テスト成功確認 → コミット `線の交わりや折り返し位置を計算する基本部品を追加` → プッシュ

### Task 1-3: ori3-cp 平面グラフと面抽出

**Files:** `crates/ori3-cp/src/{lib,graph,faces}.rs`, `tests/{graph,faces}.rs`

- [x] テストを先に書く:
  - `insert_segment`: 正方形に対角線1本→辺数5・頂点数4。交差する2本目→両線が交点で分割され頂点数5・辺数8。既存線と同一線分の重複挿入→変化なし
  - `extract_faces`: 正方形のみ→面1。対角線1本→面2。米字(対角線2本+十字)→面8
- [x] 実装:

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

- [x] テスト成功確認 → コミット `展開図の線の管理と、線で囲まれた面の検出を追加` → プッシュ

### Task 1-4: DocumentStore とIPCコマンド(編集系)

**Files:** `apps/desktop/src-tauri/src/{lib,store,commands}.rs`, `store.rs`のユニットテスト

- [x] テスト(storeはTauri非依存の純Rustとして書く): `apply_edit`でAddSegment→undo→redoの状態一致、Undo100段制限、`document_save`/`document_open`の往復一致
- [x] 実装:

```rust
pub struct DocumentStore {
    doc: Document,
    undo_stack: Vec<Document>,  // v1は「編集前スナップショット」方式(単純さ優先)。100件でFIFO破棄
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

- [x] Tauriコマンド `document_new/open/save`, `edit_apply`, `edit_undo`, `edit_redo`, `sequence_apply` を`commands.rs`に登録(各3〜10行、storeへ委譲)。panic捕捉ラッパー`fn guard<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String>`を全コマンドに適用
- [x] テスト成功確認 → コミット `作品データの保管と、編集・元に戻す・やり直しの機能を追加` → プッシュ

### Task 1-5: フロント基盤(ストア・IPCクライアント・4区画レイアウト)

**Files:** `apps/desktop/src/{App.tsx, store/appStore.ts, ipc/client.ts, lib/types.ts, components/{ToolRail,ContextPanel}.tsx}`

- [x] `lib/types.ts`: §2のRust型に対応するTS型を手書きで定義(Document, EditOp, SeqOp, Frame3D等。フィールド名はserde出力と一致させる)
- [x] `ipc/client.ts`: 型付きラッパー関数のみ(1関数5行以内)。実装済み7コマンド分を定義し、残り6コマンドは各実装タスクで追加する
- [x] `store/appStore.ts`(Zustand): 状態は `doc / faces / violations / selection / activeTool / frame3d / currentStep / warnings` と各action。IPC呼び出しはactionの中で行う
- [x] `App.tsx`: 4区画CSSグリッド(ツールレール64px / 2Dと3Dは1:1で可変 / 下部コンテキストパネル160px)。200行以内
- [x] `npm run build`成功 → コミット `画面の基本レイアウト(4区画)と画面側の土台を追加` → プッシュ

### Task 1-6: 2D展開図エディタ

**Files:** `apps/desktop/src/components/CpEditor/{CpEditor.tsx, renderer.ts, interaction.ts, snap.ts}`

- [x] スナップはフロントエンド(TypeScript)側で実装する: `snap(doc, cursorPos, radius): SnapResult | null`。優先順: 既存頂点 > グリッド交点 > 線分上(交点は挿入時に自動で頂点化されるため「既存頂点」に含まれる)。`SnapResult { pos: [x,y], kind: "vertex" | "grid" | "edge" }`
  - 理由: IPCコマンド13個にスナップ用はなく、マウス移動のたびのIPC往復は応答性が悪い。展開図データはフロントのストアに常にあるため、フロント側の純関数として実装しユニットテスト(vitest等)を付ける
- [x] Canvas描画(`renderer.ts`): 紙(白)・グリッド(薄灰)・輪郭(黒実線)・山(赤)・谷(青)・補助(灰)・選択強調(太線)・スナップ候補(丸マーカー)。線種の色分けは定数モジュールに集約
- [x] 操作(`interaction.ts`): ツール=選択/山/谷/補助/削除。2クリックで線分確定(スナップ適用)、Escでキャンセル、矩形選択、Delete削除、ホイールズーム、中ボタンパン
- [x] ツールレール接続(ボタン: 選択・山・谷・補助・削除・全体表示の6個)
- [x] 手動確認: グリッド8分割で鶴の基本形の展開図が描ける → コミット `展開図を描く画面(方眼・吸着・線の描画)を追加` → プッシュ

### Task 1-7: ori3-rigid 全域木の角度伝播(ループなしCP)

**Files:** `crates/ori3-rigid/src/{lib,tree}.rs`, `tests/tree.rs`

- [x] テストを先に書く:
  - 正方形+中央縦1本、ヒンジ角180°→2面が重なる(左面の頂点が右面へ鏡映された位置、z差はEPS以内)
  - ヒンジ角90°→2面のなす二面角が90°(法線の内積で検証)
- [x] 実装:

```rust
/// 面隣接グラフのBFS全域木を作り、根面をxy平面に固定、
/// 木辺のヒンジ角(未指定は0)で子面の姿勢(DMat3+DVec3)を伝播する。
pub struct FoldedFrame { pub transforms: HashMap<FaceId, (DMat3, DVec3)> }
pub fn propagate(cp: &CreasePattern, faces: &[Face], angles: &HashMap<EdgeId, f64>) -> FoldedFrame;
pub fn to_frame3d(cp: &CreasePattern, faces: &[Face], frame: &FoldedFrame) -> Frame3D;
```

- [x] テスト成功確認 → コミット `折り線の角度から紙の立体的な形を計算する機能を追加` → プッシュ

### Task 1-8: ori3-rigid ループ閉包ソルバー(内部頂点対応)

**Files:** `crates/ori3-rigid/src/solver.rs`, `tests/solver.rs`

- [x] テストを先に書く:
  - 次数4の内部頂点1個のCP(鳥の基本形の1頂点相当)で、driver1本を90°にしたとき、残り3ヒンジの角が閉包条件(ループ一周の回転合成=恒等、残差フロベニウスノルム<1e-6)を満たす
  - driverを±180°にすると全ヒンジが±180°に達し平坦になる
  - 不能な指定(矛盾するdriver2本)でも`converged: false`と直前解を返しpanicしない
- [x] 実装:

```rust
pub struct SolveResult { pub frame: Frame3D, pub converged: bool, pub angles: HashMap<EdgeId, f64> }
/// driver角を固定し、非木辺ヒンジごとのループ閉包残差を
/// Gauss-Newton(数値ヤコビアン+Levenberg減衰、最大50反復)で最小化。
/// warm_start: 前回解を初期値にする(連続的なスライダー操作で安定させる)
pub fn solve(cp: &CreasePattern, faces: &[Face], drivers: &[Driver],
             warm_start: Option<&HashMap<EdgeId, f64>>) -> SolveResult;
```

- [x] `pose_solve`コマンドをcommands.rsに追加(warm_startはstoreが保持)
- [x] テスト成功確認 → コミット `複雑な展開図でも折り角度のつじつまを自動で合わせる計算を追加` → プッシュ

### Task 1-9: 3Dビュー(Three.js)と角度操作

**Files:** `apps/desktop/src/components/Viewer3D/{Viewer3D.tsx, sceneBuilder.ts, hingePicker.ts}`, `ContextPanel.tsx`(ヒンジ選択時の内容)

- [x] `sceneBuilder.ts`: Frame3Dから面メッシュ生成(表=front_color/裏=back_color、DoubleSide不使用で2枚描き)、辺のライン表示、OrbitControls
- [x] `hingePicker.ts`: 3D上の辺クリックでヒンジ選択(画面距離しきい値で判定、選択中は黄色強調)
- [x] コンテキストパネル(ヒンジ選択時): 角度スライダー(−180〜+180)+数値入力。変更のたび`pose_solve`を呼びFrame3D更新(60ms間引き)
- [x] 不収束時: 3Dビュー右上に警告バッジ「⚠ 追従計算が収束していません」を表示(操作は継続)
- [x] 手動確認 → コミット `3D表示画面と、折り線ごとの角度操作を追加` → プッシュ

### Task 1-10: M1受け入れ(やっこさん)

**Files:** `crates/ori3-rigid/tests/acceptance_yakko.rs`

- [x] やっこさんの展開図(座布団折り2回相当の折り線)をコードで構築し、全driver±180°でsolveが収束し畳んだ位置(外形0.5角・8点が中心に重なる・内部頂点の写り先)が理論値と一致することを検証する回帰テスト(注: ±180°ではz座標は恒等的に0になるため、|z|<1e-6ではなく収束+位置一致を合格根拠とする)
- [ ] 手動確認: アプリでやっこさんを描いて折る。操作上の問題は`docs/progress.md`に記録(実機のGUI確認待ち。自動テスト側は完了済み)
- [x] コミット `やっこさんが折れることを確認する自動テストを追加` → プッシュ

## M2: 層順序 + 折り操作 + 手順(受け入れ: 折り鶴)

### Task 2-0: 剛体折りソルバーの性能・数値改修(M1品質レビューからの必須引き継ぎ)

**Files:** `crates/ori3-rigid/src/{tree,solver}.rs`, `apps/desktop/src-tauri/src/commands.rs`

M1の品質レビューで「面400・辺1,000でsolve 33ms以内(NFR-002)」に対し現行の密行列Gauss-Newtonは約30倍超過と判定された。M2の手順再生(毎ステップ±180°平坦到達の連続solve)に入る前に以下を改修する:

- [x] 疎ヤコビアン化: ヒンジhの列は「hを含む基本ループの残差12成分」のみ非零。ループ局所性を使いJtJ構築と数値微分の全域再伝播を排除(可能なら回転微分の解析式化)。目標: 面400でsolve 33ms以内をベンチテストで確認(実装: 閉包拘束を「非木辺を1回渡り木辺+先順の非木辺で戻る最短閉路」に置き換え+解析ヤコビアン+RCM順の帯コレスキー。20×20ミウラ折り(面400・辺840)でwarm start 1回あたりrelease約3〜6ms、tests/perf_miura.rsで回帰監視)
- [x] 収束判定を残差本数でスケールするRMS基準に変更(現行の絶対値1e-12は大規模でf64ノイズ床と衝突し、厳密解でもconverged=falseになり得る)
- [x] ±180°近傍の縮退対策: 前進差分h=1e-6を中心差分またはh適応に(平坦到達の収束減速防止)(解析微分の厳密ヤコビアンに置き換えたため差分幅の問題自体が消滅。中心差分との一致は単体テストで検証)
- [x] ±180°またぎのwrapで山谷符号が反転する問題: wrap前にwarm start前回値へ近い側を選ぶunwrap処理
- [x] 軽微: kind_signの線形走査をマップ化 / solve内のbuild_forest二重実行除去 / pose_solveのextract_faces毎回実行をstoreのキャッシュ流用に / driverを外した自由ヒンジがwarm start値のまま残る挙動をdocに明文化
- [x] コミット `折りの計算を大きな作品でも間に合う速さに改良` → プッシュ

あわせてフロント側もアニメーション(手順再生)に耐える構造へ改修する(M1品質レビューの引き継ぎ):

- [x] Viewer3D: トポロジとジオメトリの分離 — doc/faces変化時のみ三角形分割(スリット面の凹形状はShapeUtils.triangulateShape)とヒンジ集合を確定し、frame3d変化時はposition属性のin-place更新(DynamicDrawUsage)のみ。表裏は1ジオメトリ+addGroup+マテリアル配列。三角形index→面IDの対応表も作る(Task 2-5のraycastで必要)(実装: `buildTopology`(slots/indices/triangleFaceIds/lineIndices/hingeSlots/flatPositions)+ `createContent` + `updateFrame`。表裏は同じ三角形範囲へaddGroup×2、裏はBackSide指定でThree.jsが法線を反転。境界線はposition属性を面と共有)
- [x] 作り替え前にsceneBuilderのdispose回帰テストを1本追加(偽geometry/materialでdispose回数を数える)(`sceneBuilder.test.ts`のclearGroup 3件。マテリアル配列と非対象の子も確認)
- [x] pose_solve系のIPCをcoalescing方式に変更 — 実行中は保留1件を最新値で上書きし完了時に発行(FIFO積み上げによる表示遅延の防止)。編集系は従来のFIFOのまま(実装: `SerialQueue.runLatest`。追い越された要求は`{ok:false, error:SUPERSEDED, isLatest:false}`で返り、既存の破棄規約にそのまま乗る。ただし「その1回だけ0度を明示する」意味を持つ解除系pose_solveは追い越されると意味が失われるためFIFOのまま。runの後ろに積まれたrunLatestは追い越しの対象にならず順序も逆転しない)
- [x] 軽微: hingeEdgeIdsのuseMemo化(ストアの`hinges`としてdoc/faces更新時に1度だけ導出) / AngleNumberInputのdirtyフラグ(未編集blurでdriver化しない)+Escape取り消し / スロットルのテスト順序依存解消(`resetPoseThrottle`をexport) / コンテキストロスト復帰時の再描画 / setPixelRatioの追従 / render呼び出しのrAF集約 / ヒンジ選択の手前優先タイブレーク修正(0.5px刻み→手前の順で整列) / 3Dカメラのリセット手段(ツールレールの「全体」で2D・3D両方)
- [x] コミット `3D表示を手順再生に耐える作りに改良` → プッシュ

### Task 2-1: ori3-layers 平坦状態

**Files:** `crates/ori3-layers/src/{lib,flat_state}.rs`, `tests/flat_state.rs`

- [x] テスト: 正方形を半分に折った状態→2面の配置が鏡映関係、層順序が[下面, 上面]。層順序の代表点参照(layer_orderの[f64;2]→FaceId解決)が面の再抽出後も正しく対応する(テスト12件。境界・凹面(スリット・枝分かれ)・解決不能点の警告・重複点も網羅)
- [x] 実装(代表点は耳刈りで得た最初の三角形の重心、点の内外判定は境界EPS許容+交差数の偶奇):

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

- [x] テスト成功確認 → コミット `平らに畳んだときの紙の重なり順を管理する機能を追加` → プッシュ

### Task 2-2: 折り操作プリミティブ(fold_through)

**Files:** `crates/ori3-layers/src/fold_through.rs`, `tests/fold_through.rs`

- [x] テストを先に書く(10件):
  - 正方形を1回半分折り→層2枚。さらに直交方向に重ね折り→層4枚、CPに折り線が各層分(引き戻しで2本)追加され、山谷が層の向きに応じて正しく付く(mirrored反転の検証)
  - 段折り(同方向に2本、UpとDown)で層3枚・順序正しい
  - 対象層を「上1枚のみ」に指定した折りで、下層が動かない
  - 原子性(不正入力4種でErr・cpが完全無変更)/ layer_orderのresolve_order往復一致と決定性 / 紙が裂ける指定の警告
  - 折り線と重なる補助線が折り線へ昇格し、面が正しく分割され配置・層順序も半分折りと一致する(レビュー指摘の状態破壊の回帰テスト)
  - DriverLineの辺分割耐性: ステップ1の折り線が2回目の折りで2辺に分割された後も`resolve_driver_edges`が両断片を返し、全ステップのdriverをsolveに与えると畳んだ位置がFlatStateと一致する
- [x] 実装:

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
- [x] テスト成功確認 → コミット `畳んだ紙に線を引いてまとめて折る操作を追加` → プッシュ
- [x] レビュー指摘の修正 → コミット `補助線の上で折れない・手順が再生できなくなる2つの欠陥を修正` → プッシュ

### Task 2-3: 手順エンジン(記録・再生・決定性)

**Files:** `crates/ori3-layers/src/replay.rs`, `crates/ori3-layers/tests/replay.rs`, `apps/desktop/src-tauri/src/{commands,store}.rs`

- [x] テスト:
  - 手順3ステップの`Document`を`replay(doc, up_to, t)`で2回再生→Frame3Dがビット一致(SYS-004)
  - 展開図に無関係な補助線を追加後の再生→全ステップ成功(補助線が折り線を分割してもよい)
  - 手順が参照する折り線を削除後の再生→該当ステップがスキップされ警告リストに載り、以降のステップは続行(SEQ-004)
  - 一部だけ解決できない手順は残りで続行+警告 / 折り線を持たない手順(Pose)は飛ばさない / up_to・tの範囲外は丸める
  - 途中ステップ(up_to=k)の外形が期待値どおり=まだ折っていない折り線が曲がっていない / `replay(k, t=0)` が `replay(k-1, t=1)` とビット一致(非縮退のk≥2を含む)
  - 層順序の代表点が1点も解決できない手順は直前の層順序を保つ
  - 性能(NFR-002): 10ステップ・面400の全再生が3秒以内(蛇腹400面・辺1,201・層順序400点/ステップで debug実測 約0.7秒 / release実測 約23ms)
- [x] 実装:

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

- [x] `sequence_replay`コマンド追加(9個目。引数`up_to: usize, t: f64`)。`DocumentView`に`frame: Option<Frame3D>`・`skipped: Vec<StepId>`を追加し、ビューを返す全コマンドの成功後に最新ステップまで自動再生して載せる(手順が空なら`frame: None`)
- [x] ロック規約の徹底: 自動再生はstore内(ロック保持中)ではなく、コマンド層の`view_command`がロック解放後に`store::attach_replay`で行う。`sequence_replay`もロック下はDocument+facesの複製のみ
- [x] テスト成功確認 → コミット `折り手順の記録と再生(展開図を直したら自動で折り直す)を追加` → プッシュ
- [x] レビュー指摘の修正 → コミット `折り途中の手順を選んだときに違う形が表示される問題を修正` → プッシュ

### Task 2-4: タイムラインUI

**Files:** `apps/desktop/src/components/Timeline.tsx`, `ContextPanel.tsx`(ステップ選択時)

- [x] ステップ一覧(番号+技法名+警告アイコン)、クリックで選択→その時点の3D表示、◀▶コマ送り、▶再生(driver角補間アニメーション、320ms/ステップ)。タイムラインは3Dビュー区画の内側を上下分割して置く(常設区画は4つのまま)。技法名の日本語表は`lib/techniques.ts`、補間の進行計算は`lib/playback.ts`(純関数)
- [x] ステップ選択時のコンテキストパネル: 技法種別変更・注記編集・削除ボタン(`sequence_apply`)
- [x] スキップされたステップは赤表示+ツールチップで理由
- [x] 手動確認 → コミット `折り手順の一覧表示と再生・コマ送りの画面を追加` → プッシュ

### Task 2-5: 3Dビュー上の折り線描画と折り操作(SIM-005)

**Files:** `apps/desktop/src/components/Viewer3D/foldDraw.ts`, `apps/desktop/src/lib/planeProject.ts`, `ContextPanel.tsx`(折りツール時), `store/appStore.ts`(拡張), `crates/ori3-layers/src/replay.rs`(`flat_state_at`), `crates/ori3-model/src/lib.rs`(`SeqOp::FoldThrough`)

- [x] 手順から現在の平坦状態を導出する `flat_state_at(doc, faces, up_to) -> Result<FlatState, String>`(3D状態は保存しない設計のため、再生結果の3D姿勢からxy平面の等長変換を取り出す。平坦でなければErr)。座標系は3D表示と同じ(根面=最小面IDが恒等変換)
- [x] Tauriコマンドは増やさず、`SeqOp::FoldThrough { up_to, line, keep_side_point, target_layers, direction }` を追加して `sequence_apply` で実現(`FoldDirection`はori3-modelへ移動しserde対応、ori3-layersは再エクスポート)。v1は末尾(`up_to == sequence.len()`)のみ許可し、途中への挿入はErr
- [x] 3Dビューに「折る」ツールを追加(ツールレール7個目): 平坦状態の紙の上でドラッグ→画面座標をz=0平面へ投影(`lib/planeProject.ts`)→端点を紙の輪郭・既存頂点へスナップ(`foldDraw.ts`)→折り線と動く側のプレビュー表示(既存のハイライト機構を流用、動く側は半平面で切り取った輪郭)
- [x] 平坦でないとき(折り途中の手順・再生中・角度スライダー使用中)は3Dビュー右上に「平らに畳んだ状態で使えます」と出して描画させない
- [x] 確定UI(コンテキストパネル): 方向(手前へ折る(谷)/向こうへ折る(山))、対象層(全ての層/いちばん上の1枚)、動かす側(左/右)→「折る」で`SeqOp::FoldThrough`を送信→2D展開図に折り線が追記され、タイムラインに手順が1つ増える。「やめる」で破棄(v1では「選択した層」は出さない)
- [x] 2D側でも同じ折り操作を出せるようにする(2回クリックで線を引き、同じ確定UIを使用)。手順が1つ以上ある作品では展開図座標と畳み平面座標が食い違うため2D側からの折りは断り、「折る操作は3D画面から行ってください」と案内する
- [x] 手動確認: 座布団折り→観音折り(6手順)を3D側の線描画だけで完成できることを実機のスクリーンショットで確認。同じ手順の自動テストも追加(`cushion_then_cupboard_fold_only_with_fold_through`) → コミット `3D画面に直接線を引いて折る操作を追加(展開図へ自動反映)` → プッシュ

### Task 2-6: 技法マクロ(段・中割り・かぶせ / 花弁・開いてつぶすは残作業)

**Files:** `crates/ori3-layers/src/techniques.rs`, `tests/techniques.rs`

- [x] テスト: 2層・4層のフラップ(正方形を半分/4つ折りにしたもの)を下ごしらえとして、(a)中割り折りで首を折る→層数・層順序・CPへの追加線が期待値どおり (b)続けてもう一度中割り折りして頭にできる(鶴の首と頭の流れ)。折り目の向き(山谷)と層順序の一致検証を全技法に、t=0.99の高さからの重なり検証を段折り・かぶせ折りに適用(中割り折りだけは、フラップを開く動きを再生で表せないため高さからは判定できない)
- [x] 実装(全て「fold_throughと層順序操作の合成」として実装し、専用データ構造を持たない):

```rust
/// 対象フラップ(面集合)と折り線・基準点を受け取り、技法に必要な折り線群・
/// driver群・層順序変化を生成してFoldStepを返す。
pub fn pleat(cp, faces, state, input: &TechniqueInput) -> Result<FoldThroughResult, String>;
pub fn inside_reverse(...) -> Result<FoldThroughResult, String>;
pub fn outside_reverse(...) -> Result<FoldThroughResult, String>;
```

  引数は共通で `(cp, faces, state, &TechniqueInput { flap, line, reference_point })`。生成不能な形状ではErrを返し、UI側は「手動の折り操作で代替してください」と案内(要件§12)
- [x] Tauriコマンドは増やさず、`SeqOp::Technique { up_to, kind, flap, line, reference_point }` を追加して `sequence_apply` で実現(末尾のみ許可)
- [x] ツールレールに「技法」ボタン(8個目・サブメニュー3種)を追加し、フラップクリック→線指定→適用の流れを実装
- [x] 層の数が奇数のフラップ(奥と手前に半分ずつ分けられない選び方)と、折り上がりの山谷が重なり順と食い違う場合はErrで断る(壊れた紙を作らない)
- [x] テスト成功確認 → コミット `中割り折りなど基本の折り方を選ぶだけで折れる機能を追加` → プッシュ

#### Task 2-6b(引き継ぎ): 花弁折り(petal)と開いてつぶす(squash)

Task 2-6では作れなかったため、次のタスクとして残す(重み5)。

- [ ] 「畳んだ状態で指定した折り目を開く(角度を0°へ戻す)」プリミティブを追加する。層順序の決定を含む。この2種は既存の折り目を開く操作を含むため、折り線を足すことしかできない `fold_through` の合成では作れない
- [ ] その上で `petal` / `squash` を実装し、ツールレールのサブメニューに追加する(それまではサブメニューに出さず、手動の折り操作で作る)
- [ ] 鶴の基本形(前面が持ち上がる花弁折り)のテストはここで行う

### Task 2-6b: 折り目を開くプリミティブと、花弁折り・開いてつぶす折り(重み10)

**Files:** `crates/ori3-layers/src/{open_fold.rs, techniques.rs}`, `tests/`, UI

Task 2-6で判明した構造的な不足への対応。**折り鶴(M2受け入れ)には花弁折りが必須**であり、M4の沈め折りも同じ「開く」動きを含むため、ここで土台を作る。

- [ ] **開くプリミティブの設計**: 畳んだ状態で「既存の折り目を開いて、開いた面を新しい位置へ平らに置き直す」操作。折り線の追加だけでなく、(a)対象面群の新しい配置(等長変換)(b)新しい層順序 を明示的に決める必要がある。`fold_through` と同じ「原子的にCPと平坦状態を更新しFoldStepを生成する」契約に揃える
  - 想定シグネチャ: `pub fn open_fold(cp, faces, state, input: &OpenFoldInput) -> Result<FoldThroughResult, String>`
  - `OpenFoldInput`: 開く折り目(畳み平面の線分または既存辺の指定)、開いた後に新しく入る折り線群、対象層、開いた層を置く先(参照点または目標配置)
  - **手順再生との整合**: 生成するFoldStepのdriversは「このステップが動かす全ての折り線」を含むこと(未指定の折り線は0°固定されるため)。**レビューで検証済みの事実**: `DriverLine.target_angle_deg` は任意の値を取れ、`plan_steps`(replay.rs:212-234)・`flat_state_at`(replay.rs:137-142)・ソルバー(solver.rs:168-177)がいずれも「後のステップが勝つ」ので、**「後のステップで既存の折り目を0°へ駆動する」は再生で正しく効く**。この土台は追加改修なしで使える
  - **設計上の注意(レビューの指摘)**: 開く操作が必ず紙を動かすとは限らない。古典的な「2回半分に折った正方形→予備基本形」の2回のつぶし折りは、**全ての面の配置が変わらず、変わるのは層順序と1組の折り目の山谷だけ**(4つの四半分が等長に重なるため対角線は0°のまま)。この退化ケース(開いた折り目が0°で紙が動かない)を一級市民として扱う設計にすること
  - **不可能性の裏付けをテストで固定**: 「既存の折り目そのものを `fold_through` に渡すとErrになる」ことを確認するテストを2-6bの最初に置く(現状は主張だけで失敗する実験が残っていない)
- [ ] `squash`(開いてつぶす)を open_fold の合成として実装
- [ ] `petal`(花弁折り)を実装(鶴の基本形の前面が持ち上がることをテスト)
- [ ] UIのサブメニューに2種を追加(5種になる)
- [ ] テスト: 各技法の層数・層順序・追加線・drivers、表示上の重なり順検証(t=0.99のz読み取り方式)、失敗時のErrと原子性
- [ ] コミット `花弁折りと開いてつぶす折りを追加(折り目を開く操作の土台つき)` → プッシュ

### Task 2-7: 作図補助・局所平坦判定・めり込み警告

**Files:** `crates/ori3-cp/src/{construct,flatfold}.rs`, `crates/ori3-rigid/src/lib.rs`(交差検査), 各tests

- [ ] 作図補助(テスト先行): `bisector(角の3点)` / `perpendicular(点, 辺)` / `divide_points(辺, n)` / `direction_lines(点, 22.5°刻み)`。ツールレールのサブメニューから利用
- [ ] 局所平坦判定: 内部頂点ごとに前川(山−谷=±2)・川崎(交互角和=180°)を検査し違反頂点を返す→2Dで橙色表示(CPE-009)
- [ ] めり込み簡易警告: Frame3Dの面ペアの三角形交差を総当たり検査(面数400まで想定、rayonで並列化)→交差ありなら3Dビューに警告バッジ(SIM-007)
- [ ] テスト成功確認 → コミット `作図の補助線・折りたたみ可否の注意表示・紙のめり込み警告を追加` → プッシュ

### Task 2-8: 自動保存と復旧

**Files:** `apps/desktop/src-tauri/src/autosave.rs`, `dialogs/RecoveryDialog.tsx`

- [ ] 30秒間隔+dirty時のみ`<保存先>.ori3.autosave`へ保存。正常終了時に削除
- [ ] 起動時`recovery_check`でautosaveの有無を返し、あれば復旧ダイアログ(復元する/破棄する)
- [ ] storeユニットテスト+手動確認(プロセスkill→再起動→復元) → コミット `30秒ごとの自動保存と、異常終了後の復元機能を追加` → プッシュ

### Task 2-9: M2受け入れ(折り鶴)

**Files:** `crates/ori3-layers/tests/acceptance_crane.rs`

- [ ] 折り鶴を「fold_through+技法マクロの列」でスクリプト構築し、最終状態の層数・外形寸法・決定性を検証する回帰テスト
- [ ] 手動確認: アプリで鶴を1折りずつ折って完成→展開図の一部を修正→自動再生で形が追従
- [ ] コミット `折り鶴が折れることを確認する自動テストを追加` → プッシュ

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

- [ ] テスト(validate正常系/異常系)→実装→コミット `頭・尾・足などの骨格を指定するためのデータ形式を追加` → プッシュ

### Task 3-2: 円・川充填の数値最適化

**Files:** `crates/ori3-propose/src/packing.rs`, `tests/packing.rs`

- [ ] テスト: (a)葉2(長さ1,1)を1×1紙に充填→縮尺≥0.5に到達 (b)葉5の充填で全制約(円非重複・紙内)違反がEPS以内 (c)同一シード→同一結果(決定性)
- [ ] 実装: 変数=各葉の円中心+縮尺s。目的=s最大化。制約=|ci−cj| ≥ s·(li+lj+川幅), 円中心は紙内。射影勾配法(制約違反を射影で戻す)×乱数シード別マルチスタート(既定8スタート、上位4候補を返す)。`rand::rngs::StdRng::seed_from_u64`で決定的に
- [ ] テスト成功確認 → コミット `骨格に合わせて紙の上に必要な領域を自動配置する計算を追加` → プッシュ

### Task 3-3: 展開図生成(充填→分子→折り線)

**Files:** `crates/ori3-propose/src/generate.rs`, `tests/generate.rs`

- [ ] テスト: 葉4+胴1の充填結果から生成したCPが (a)妥当な平面グラフ(extract_faces成功) (b)軸線・稜線が揃い、局所平坦判定の違反頂点数を結果として返す
- [ ] 実装手順: 円中心のドロネー三角形分割→各三角形をウサギ耳分子(3辺の二等分線+垂線)で充填→四角形以上は扇状分割→山谷割り当て(軸線=谷基調、稜線=山基調の既定則)→`ProposalResult { cp: CreasePattern, violations: usize }`
- [ ] `proposal_generate`コマンド追加(Skeleton→候補最大4件のVec<ProposalResult>)
- [ ] テスト成功確認 → コミット `自動配置の結果から展開図を組み立てる機能を追加` → プッシュ

### Task 3-4: 提案ウィザードUI

**Files:** `apps/desktop/src/components/dialogs/ProposalWizard.tsx`

- [ ] 3画面構成: ①骨格編集(角の追加/削除ボタン+各角の長さ・太さスライダー+2D骨格プレビュー) ②候補選択(生成4候補の展開図サムネイル+違反数表示) ③確認→`edit_apply ReplaceCreasePattern`で流し込み、ダイアログを閉じる
- [ ] ツールバーの「提案ウィザード」ボタンから起動。メイン画面に常設UIを追加しない(PRO-004)
- [ ] 手動確認: 頭1・尾1・足4で鶴系の基本形が得られ、そのまま編集・折りに進める(M3受け入れ) → コミット `骨格を指定して展開図を提案してもらう画面を追加` → プッシュ

## M4: 複雑技法 + 書き出し(受け入れ: 伝承のカエル)

### Task 4-1: 沈め折り(open sink)

**Files:** `crates/ori3-layers/src/techniques.rs`(追加), `tests/sink.rs`

- [ ] テスト: 鶴の基本形の頂点を沈める→対象領域の全層で山谷が反転し、層順序が沈め込み後の入れ子順になる
- [ ] 実装: `pub fn open_sink(cp, faces, state, region_line: [[f64;2];2]) -> Result<FoldThroughResult, String>`。折り線より先端側の全層について、(a)折り線で各層を分割 (b)先端側の山谷を反転 (c)層順序を内外反転して再挿入
- [ ] テスト成功確認 → コミット `沈め折りを選ぶだけで折れる機能を追加` → プッシュ

### Task 4-2: ひだ寄せ・ねじり折り

**Files:** `crates/ori3-layers/src/techniques.rs`(追加), `tests/{swivel,twist}.rs`

- [ ] `pub fn swivel(...)`: 基準線+寄せ線の2線指定でひだを寄せる(fold_through2回+層併合)
- [ ] `pub fn twist(...)`: 多角形領域+周辺ひだ線の指定でねじる(領域回転配置+周辺ひだのfold_through列)
- [ ] 各テスト(層数・順序・CP追加線の検証)→実装→コミット `ひだ寄せとねじり折りを選ぶだけで折れる機能を追加` → プッシュ

### Task 4-3: 展開図SVG/PNG書き出し

**Files:** `crates/ori3-export/src/{cp_svg,cp_png}.rs`, `tests/export_cp.rs`, `dialogs/ExportDialog.tsx`

- [ ] テスト: 生成SVGに線種別スタイル(山=一点鎖線/谷=破線/輪郭=実線)が含まれ、viewBoxが実寸mm。PNGが指定解像度で非空
- [ ] 実装: SVGは文字列組み立て(`svg`クレート可)。PNGはresvgでSVGをラスタライズ。`document_export`コマンド追加(種別enum: CpSvg/CpPng/DiagramPdf/DiagramSvg)+書き出しダイアログ(補助線含む/含まない、PNG解像度)
- [ ] テスト成功確認 → コミット `展開図を画像ファイルとして保存する機能を追加` → プッシュ

### Task 4-4: 折り図レンダラ(ステップ図)

**Files:** `crates/ori3-export/src/diagram.rs`, `tests/diagram.rs`

- [ ] テスト: 3ステップの手順から3コマのSVGが生成され、各コマに(a)折る前の平坦状態の正射影(可視輪郭+可視折線) (b)今回の折り線(山=一点鎖線/谷=破線) (c)技法別矢印(TechniqueKindごとに固定の記号パス)が含まれる
- [ ] 実装: `pub fn render_step(doc: &Document, step_index: usize) -> String /* SVG */`。投影は該当ステップ直前のFlatStateの最上層から可視面を層順に描画。矢印記号はTechniqueKind→固定SVGパスのテーブル(谷矢印/山矢印/中割り/かぶせ/花弁/つぶし/沈め/ひだ/ねじり/ポーズの10種)
- [ ] テスト成功確認 → コミット `折り手順を1コマずつ図にする機能を追加` → プッシュ

### Task 4-5: 折り図PDF/SVG組版

**Files:** `crates/ori3-export/src/pdf.rs`, `tests/pdf.rs`

- [ ] テスト: 7ステップの手順→A4・2列×3コマで2ページのPDFが生成される(ページ数・非空を検証)。SVG版はページ単位のファイル群
- [ ] 実装: render_stepのSVGをA4(210×297mm)グリッドに配置(コマ番号+注記付き)、svg2pdfでPDF化。表紙(タイトル+完成図)を1ページ目に付ける
- [ ] 書き出しダイアログにDiagramPdf/DiagramSvgを接続
- [ ] テスト成功確認 → コミット `折り図をPDFとして保存する機能を追加` → プッシュ

### Task 4-6: M4受け入れ(伝承のカエル)

**Files:** `crates/ori3-layers/tests/acceptance_frog.rs`

- [ ] 伝承のカエル(花弁折り・中割り折り・段折りを含む)をスクリプト構築する回帰テスト(最終層数・決定性)
- [ ] 手動確認: アプリでカエルを折って完成→折り図PDFを書き出し、内容を目視確認
- [ ] コミット `伝承のカエルが折れて折り図を出せることを確認する自動テストを追加` → プッシュ

## 3. マイルストーン完了時の共通チェック

各M完了時に以下を実施してからプッシュする:

1. `scripts/check.ps1` 全通過
2. 行数計測(`tokei`等)し、Rust 50,000行 / TS 20,000行以内を確認。超過傾向なら次Mの前に削減タスクを積む
3. Tauriコマンド数が13個のままであることを確認
4. `docs/progress.md` に完了内容・既知の問題を3〜10行で追記(長文の経緯記録は禁止。要件定義書NFR-006)
5. 要件定義書の該当要件IDに対する充足状況を確認し、未達があればタスク化

## 4. 実装順の依存関係

```
M0 → 1-1 → 1-2 → 1-3 → 1-4 → 1-5 → 1-6 ─┐
                   └→ 1-7 → 1-8 → 1-9 ──┴→ 1-10
M2: 2-1 → 2-2 → 2-3 → 2-4 → 2-5 → 2-6 → 2-9
    (2-7, 2-8 は 2-3 完了後いつでも並行可)
M3: 3-1 → 3-2 → 3-3 → 3-4(M2完了に依存しない。M1完了後なら並行可。ただし受け入れはM2の折り操作を使う)
M4: 4-1 → 4-2 → 4-6 / 4-3 → 4-4 → 4-5(4-3系は2-3完了後なら並行可)
```
