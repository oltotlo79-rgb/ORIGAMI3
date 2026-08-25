# ORIGAMI3 総合コードレビュー・競合評価・外注再構築費試算

- 評価日: 2026-08-24
- 対象版: 0.4.5
- 対象: リポジトリに存在するRust、TypeScript/TSX/CSS、テスト、CI、設定、要件・進捗文書
- 比較基準: `docs/review-2026-08-20.md` の67/100、`docs/competitive-review-2026-08-20.md` の6.5/10、利用者提示の以前の外注費試算約7億円

## 1. 結論

### 1.1 総合判定

**現時点の総合点は78/100、競争力は6.9/10である。** 2026-08-20の67/100・6.5/10から、それぞれ**11ポイント・0.4ポイント上昇**した。

上昇の主因は、前回弱かった3D重なり順、接触後処理、仕上げ値の手順保存、提案結果の型安全性、既知作品の完成検証、起動・画面・説明書の実測証拠がコードとテストへ入ったことである。一方、2026-08-24の再実行ではRust通常テストが失敗した。特に「最も重い提案が時間制限へ当たらない」検査は単独でも4候補すべて `TimeCap` となった。このため、リリース判定を通過した状態、または完成度90点以上とは評価できない。

本プロジェクトの最も強い価値は、単なる展開図エディタでも単なる同時折りシミュレータでもなく、次の流れを一つのWindowsアプリ内でつないでいる点にある。

> 展開図編集 → 剛体3D姿勢 → 層・接触 → 名前付き折り技法 → 手順再生 → 完成候補提案 → 折り図PDF/SVG

この統合範囲は競合に対する明確な差別化になる。ただし、FOLD等の交換形式を持たないこと、Windows・日本語のみであること、提案の公開検証標本が3作品に留まること、実利用者テストが存在しないことは、技術の普及価値を大きく制限する。

### 1.2 外注再構築費

このコードとテストを、仕様化、設計、実装、検証、Windows配布まで含めてゼロから外注で同等再構築する費用は、**基本ケースで8.1億～9.2億円、中心値8.7億円**と試算する。

以前の約7億円を同じ費用範囲・同じ品質条件の基準値とみなす場合、再構築費ベースの価値上昇は**約1.1億～2.2億円、中心値約1.7億円（約24%）**である。

これは会社価値、売却価格、将来収益、知財プレミアムではない。あくまで、現在確認できる成果物を同程度まで作り直すための外注費である。また、以前の7億円の算定資料は本リポジトリ内に存在しないため、前回計算式そのものの監査はできない。比較は利用者提示の7億円を基準点として行った。

## 2. 調査方法と「事実」「評価」「試算」の区別

### 2.1 実際に確認したもの

1. `CLAUDE.md`、要件、ロードマップ、進捗文書、前回レビューを最後まで確認した。
2. 9個のRustワークスペース構成、Tauriホスト、Reactフロント、テスト、CI、リリース設定を確認した。
3. モデル、CP、幾何、剛体、層、soft、提案、書出し、保存、UIの主要経路をソースで追った。
4. 行数・ファイル数・テスト属性をリポジトリから再集計した。
5. 規約上git操作を実行できないため、gitを呼ぶ `scripts/check.ps1` は使用せず、その5工程を個別に実行した。
6. 競合の機能は各プロジェクトの公式サイト、公式リポジトリ、公式仕様のみを根拠にした。

### 2.2 判断の境界

- **確認済み事実**: コード、テスト、設定、実行結果、公式資料に直接存在するもの。
- **評価**: 下記の採点基準へ確認済み事実を当てはめたもの。
- **費用試算**: 正確な見積書ではなく、確認済み実装範囲をWBSへ分解し、人月単価を可変入力にした条件付き計算。単価を事実のようには扱わない。
- **未確認**: 実利用者の操作成功率、市場シェア、販売実績、過去の実工数、実際の外注契約単価。これらは点数へ推測で加算していない。

## 3. 実装規模の実測

| 項目 | 実測値 | 根拠 |
|---|---:|---|
| Rustファイル | 156 | `crates/**/*.rs` と `apps/desktop/src-tauri/**/*.rs` の列挙 |
| Rust物理行 | 100,742 | 上記156ファイルを `ReadAllLines` で集計 |
| フロントファイル | 223 | `apps/desktop/src/**/*.{ts,tsx,css}` |
| フロント物理行 | 78,772 | 上記223ファイルを集計 |
| フロント製品コード | 114ファイル、41,410行 | テスト名を除外して集計 |
| フロントテスト | 109ファイル、37,362行 | `*.test.*` / `*.dom.test.*` を集計 |
| 合計物理行 | 179,514 | Rust + フロント。自動生成物・依存物・文書は含めない |
| Rustテスト属性 | 858 | `#[test]` / `#[tokio::test]` を静的集計 |
| Vitest実行結果 | 1,719件中1,718合格、1スキップ | 2026-08-24 `npm run test` |
| `#[allow(...)]` | 0件 | Rust・フロントを検索 |
| Rust `unsafe` | 本番0件 | 5箇所は `#[cfg(test)]` の計測用アロケータのみ。`apps/desktop/src-tauri/src/lib.rs:24-58` |

Rustは `ori3-model`、`ori3-geometry`、`ori3-cp`、`ori3-rigid`、`ori3-layers`、`ori3-soft`、`ori3-propose`、`ori3-export` とTauriホストの9単位である（`Cargo.toml:3-11`）。版は0.4.5、Rust edition 2024である（`Cargo.toml:16-17`）。フロントはReact 19.1、Three.js 0.185.1、Zustand 5.0.14、Tauri 2を使う（`apps/desktop/package.json:17-23`）。

行数は品質そのものではない。ここでは、費用試算の対象範囲と保守対象量を確定するためにだけ用いる。

## 4. 現行品質ゲートの実測

### 4.1 結果

| 検査 | 2026-08-24結果 | 証拠・解釈 |
|---|---|---|
| Rust workspace test | **不合格** | CI相当のskipを付けて実行。3件の失敗を確認後、残る長時間テストを中断 |
| Rust clippy | 合格 | `cargo clippy --workspace --all-targets -- -D warnings`、21.67秒 |
| フロントbuild | 合格、警告あり | 17.49秒。main JS 1,319.50kB、gzip 374.46kBで500kB警告 |
| フロントlint | 合格 | 13.17秒 |
| フロントtest | 合格 | 109ファイル、1,718合格、1スキップ、28.18秒 |

通常Rustテストで観測した失敗は次の3件だった。

1. `commands::tests::proposal_progress_counts_every_candidate`
2. `commands::tests::the_heaviest_proposal_never_hits_the_time_limit`
3. `commands::tests::proposal_candidates_are_the_same_computed_together_or_one_by_one`

切り分け結果は重要である。

- `proposal_progress_counts_every_candidate` は単独実行で合格した。進捗がプロセス共通の `PROPOSAL_DONE` / `PROPOSAL_TOTAL` に保存されるため（`apps/desktop/src-tauri/src/commands.rs:932-970`）、並列テストまたは複数要求が互いの値を上書きできる構造と整合する。
- `proposal_candidates_are_the_same_computed_together_or_one_by_one` は単独で合格したが70.91秒かかった。並列時の資源競合が壁時計上限へ影響する設計と整合する。
- `the_heaviest_proposal_never_hits_the_time_limit` は単独でも30.59秒後に不合格となり、4候補すべてが `TimeCap` だった。製品側 `PLAN_BUDGET.max_millis` は30,000msである（`apps/desktop/src-tauri/src/commands.rs:892-898`）。

CIの通常Rustテストはこの3件をskipしていない（`.github/workflows/ci.yml:60`、`scripts/check-ci.ps1:484`）。したがって「ローカルだけの参考テスト」ではなく、現在の品質ゲートを止める回帰である。

### 4.2 根本原因の評価

提案探索には状態数・分岐数・深さという決定的な上限がある一方、結果の停止理由へ壁時計 `max_millis` も混ぜている（`crates/ori3-propose/src/search.rs:392-418,990-1076`）。同じ入力でもCPU負荷により探索済み状態が変わり得る。コメント自身も壁時計を安全弁としてのみ使う意図を記している（`apps/desktop/src-tauri/src/commands.rs:869-879`）が、現在のコードでは利用者へ返す探索結果を `TimeCap` にしており、意図と結果が一致していない。

完全修正はテスト時間の延長だけではない。次が必要である。

1. 製品結果の決定は状態数・分岐数・深さ・候補順だけで行う。
2. 壁時計はハング監視またはキャンセルとして別経路にし、通常の「部分提案」の品質判定へ混ぜない。
3. 提案ジョブごとにIDと進捗状態を持ち、グローバル2原子変数を廃止する。
4. 同じ入力を低負荷・高負荷・並列要求で実行して、結果と停止理由が一致する受け入れテストを追加する。

## 5. 分野別採点

前回と比較できるよう、同じ12分野を各10点で採点し、合計を100点換算した。93/120 = 77.5を四捨五入して78/100とする。

| 分野 | 前回 | 現在 | 主な根拠 | 満点を阻むもの |
|---|---:|---:|---|---|
| 紙・作品モデル PAP | 9 | **9** | f64、EPS、紙・色・手順・表示設定の型、通常3D頂点を保存しない構造 | schema v1の移行戦略が薄い |
| CP・幾何 CPE | 9 | **9** | スナップ、交差分割、重複区間処理、面抽出、曲線・作図法 | 非連結の入れ子閉ループを穴として扱えない |
| 3D・剛体・soft SIM | 6 | **9** | 疎Gauss-Newton、RCM Cholesky、接触の検出/防止分離、権威ある面順、soft品質ゲート | exact物理ではない。一般の全構造への完全性証明はない |
| 手順・層・技法 SEQ | 9 | **8** | 座標ベースDriver、再導出、8技法、原子的技法処理 | 手順移動がRemove+Insertの2操作。複雑経路は近似 |
| 提案 PRO | 7 | **7** | tree/circle-river/CP生成/探索/21姿勢検証/型付き完成 | 現在のTimeCap回帰、3標本、壁時計依存 |
| 書出し・説明書 EXP | 9 | **9** | SVG/PNG、6コマA4 PDF、自動説明、82ページ説明書 | 交換形式と3D書出しがない |
| UI・UX | 8 | **8** | 4領域、直接操作、プレビュー、F1ヘルプ、テーマ、広いDOMテスト | ダイアログのfocus管理、実利用者検証、巨大bundle |
| 保存・履歴・デスクトップ SYS | 5 | **9** | 100履歴、メモリ実測、30秒autosave、原子的保存、復旧経路防御、配布設定 | 手順移動の原子性、schema移行、更新機構がない |
| 非機能 NFR | 4 | **6** | 起動・solver・replay・soft・画面状態の実測資産 | 現在の全ゲート不合格、bundle警告、提案時間依存 |
| アーキテクチャ | 5 | **6** | 9分割、型付きIPC、一元store、決定的BTree構造 | store/UI/CSS/replayに巨大集中 |
| テスト・CI | 7 | **7** | Rust 858属性、Vitest 1,719件、release性能・packageジョブ | 通常Rustゲート不合格、共有状態flaky、外部標本が少ない |
| 文書・進捗管理 | 2 | **6** | 詳細要件、進捗、コード内の数値根拠、同一ソースのヘルプ/説明書 | roadmapと実装が同期せず、コメントの値にも陳腐化あり |

### 5.1 紙・作品モデル: 9/10

`SCHEMA_VERSION=1`、`EPS=1e-9` が明示されている（`crates/ori3-model/src/lib.rs:3-6`）。手順のドライバは不安定な辺IDではなく展開図上の線分座標を保存する `DriverLine` である（`crates/ori3-model/src/lib.rs:79-86`）。仕上げsoftは有効、硬さ、膨らみの設定だけを `FoldStep.finish_soft` に保存し、通常の3D頂点配列を作品へ永続化しない（`crates/ori3-model/src/lib.rs:139-149,248`）。これは「3DはCP+手順から再導出する」という要件に合う。

弱点はschemaがまだ1で、将来の破壊的変更に対する段階移行、旧版fixture群、ダウングレード方針が製品機能として十分に実証されていないことである。

### 5.2 CP・幾何: 9/10

辺挿入は既存辺との交差を分割し、同一直線の重複区間へ二重辺を作らない（`crates/ori3-cp/src/graph.rs:224-319`）。面抽出は決定的な半辺走査を使う。Maekawa/Kawasaki等の局所検査、円弧・三次曲線の分割、整列・二等分等の作図経路も実装されている。

既知制限はソースに明記されている。外周と接続しない入れ子閉ループは穴として扱われず、重なる面になり得る（`crates/ori3-cp/src/faces.rs:26-28`）。要件上は正方形・長方形の紙が中心なので直ちに失格ではないが、一般CP互換や切抜きを拡張する際の障害になる。

### 5.3 3D剛体・接触・soft: 9/10

剛体ソルバーは名称だけの近似ではない。閉路拘束を疎ヤコビアンで組み、Levenberg減衰付きGauss-Newtonを最大50反復、RCM順のエンベロープCholeskyで解く（`crates/ori3-rigid/src/solver.rs:4-24,42-44,928-929,1278-1339`）。解析Jacobianと中心差分の一致テストもある（`crates/ori3-rigid/src/solver.rs:1443`）。閉包許容はRMS `1e-13`（`crates/ori3-rigid/src/solver.rs:42`）。

重なり順は任意のface ID順へフォールバックして「物理順」と偽装しない。正面積overlapがすべて解決した場合だけ `complete` となり（`crates/ori3-rigid/src/surface_order.rs:568-603`）、外部から任意構築できない `AuthoritativeSurfaceOrder` を発行する（`crates/ori3-rigid/src/surface_order.rs:640-709`）。softの重なり補正も権威ある順序を要求し、finite、剛性、平面性、seam、交差が悪化しない候補だけ採用する（`crates/ori3-soft/src/lib.rs:346-371,434-571`、`crates/ori3-soft/src/quality.rs:24-63`）。

softは設計通り**見た目の近似**であり、材質、重力、摩擦、皺を再現しない（`crates/ori3-soft/src/lib.rs:5-11`）。最大8,000三角形を超えると細分を下げる（`crates/ori3-soft/src/lib.rs:46,605-607`）。したがって、Origami Simulatorのstrain表示や構造解析の代替とは評価しない。

### 5.4 手順・層・名前付き技法: 8/10

`SeqOp` は単純折り、合わせ折り、平坦モーション、名前付き技法を型で区別する。pleat、inside/outside reverse、squash、petal、open sink、swivel、twistは表示ラベルだけではなく、展開図更新と折り操作の組合せとして実装されている。技法処理はCPの複製を成功時だけ反映し、途中失敗で原本を変えない（`crates/ori3-layers/src/techniques.rs:945-984`）。

一方、フロントの `moveStep` は `RemoveStep` の後に `InsertStep` を別IPCとして実行する（`apps/desktop/src/store/appStore.ts:4645-4666`）。前半だけ成功すると手順を失い、Undoも2回必要になる。これは具体的な原子性欠陥である。backendへ `SeqOp::MoveStep { id, to }` を追加し、1ロック・1履歴・1再生検証で完了させるべきである。

また、途中再生の一様角補間は一般には剛体経路ではないことを実装自身が明記する（`crates/ori3-layers/src/replay.rs:28-52`）。終点の正しさと視覚的な途中経路を区別して扱っている点は誠実だが、任意複合技法の厳密モーションを保証するものではない。

### 5.5 完成候補提案: 7/10

提案は最大12葉のskeleton、長さ・太さ・完成位置、circle/river packing、CP生成、候補手、探索、最終検証まで実体がある。検証は各手を `t=0,0.05,...,1` の21姿勢で再生し、平坦性、seam、貫通、層警告、最終の長さ・太さ・位置gapを調べる（`crates/ori3-propose/src/verify.rs:27-35,342-427`）。

さらに、完成と途中案をbool一つで曖昧にせず、privateな `CheckedToFinish` と `VerifiedPlan::CheckedToFinish/Partial` に分ける（`crates/ori3-propose/src/verify.rs:240-302`）。完成条件を通らずに「完成済み」を構築しにくい型設計は強い。

しかし、前節の30秒回帰は提案機能の中核を直接傷つける。既知完成標本も現在3件であり、鶴、カエル、昆虫、動物、多葉・非対称等へ一般化できることを示す公開corpusとしては不足する。よって、実装量が増えても前回7点から上げない。

### 5.6 書出し・説明書: 9/10

実寸mmのCP SVG、PNG、各手SVG、A4 2列×3コマの折り図PDF、自動日本語説明、アプリと同一内容源から生成する82ページの説明書がある。単なる3Dビューアより「人が折るための成果物」へ到達している点は競争上強い。

競争上の弱点はFOLD、CP、DXF、OBJ、STL、glTFを扱わないことである。これは現在の要件で明示的非目標（`docs/requirements-definition.md:57-72`）なので要件違反ではないが、他アプリとの往復、既存CP資産の取込み、研究用途の再利用性を失う。

### 5.7 UI・UX・アクセシビリティ: 8/10

4領域レイアウト、1000×700下限、直接操作、折る前のプレビュー、利用不能理由、F1ヘルプ、初回ガイド、5テーマを実装する。`aria-label` は143箇所、`aria-live` は11箇所あり、ダイアログも `role=dialog` と `aria-modal` を持つ。

ただし、Escape、open時focusを実装するのは主にHelpCenterである（`apps/desktop/src/components/dialogs/HelpCenter.tsx:106-116`）。New、Export、Proposal、Recoveryには共通focus trap、初期focus、閉じた後のfocus復帰が見当たらない。`aria-modal=true` だけではキーボード操作を完成させない。

また `App.tsx` は「200行以内」の自己規約に対し290行（`apps/desktop/src/App.tsx:3,290`）。`appStore.ts` 4,670行、`App.css` 5,452行、`Viewer3D.tsx` 2,248行、`sceneBuilder.ts` 2,102行、`ContextPanel.tsx` 1,871行である。main JS 1.32MB警告もあり、画面単位lazy loadがない。現在の機能は動いても、変更影響範囲と初期読込コストが増えている。

実利用者テストは見つからなかった。DOMのはみ出し0や文言検査は価値があるが、「説明なしで初心者が目的を達成できるか」はコードから確認できないため加点しない。

### 5.8 保存・履歴・デスクトップ: 9/10

Undoは最大100スナップショット（`apps/desktop/src-tauri/src/store.rs:29,829`）、密なfrog fixtureで約813KB、上限1.1MBの計測契約を持つ（`apps/desktop/src-tauri/src/store.rs:45-55`）。30秒ごとにdirtyな作品だけautosaveし（`apps/desktop/src-tauri/src/autosave.rs:21,132-151`）、一時ファイルからrenameする原子的保存を行う（`apps/desktop/src-tauri/src/store.rs:62-71`）。復旧markerから任意ファイルを削除しないようpathを検証する（`apps/desktop/src-tauri/src/autosave.rs:71-105`）。

Tauri設定は1280×860、最小1000×700、CSP、current-user installerを持つ（`apps/desktop/src-tauri/tauri.conf.json:16-23,48`）。一方、自動更新経路、schema移行マトリクス、FOLD等との災害復旧用交換経路は確認できない。

### 5.9 非機能・性能: 6/10

進捗文書には起動中央値1,356ms、最大1,385ms、100画面状態、23/23 overlap対、既知3/3提案、10/10決定性等の具体的測定がある。CIにもsolver、replay、soft、curve、proposalのrelease性能ジョブがある（`.github/workflows/ci.yml:104-140`）。

しかし最新の通常ゲートが失敗しているため、過去の合格記録だけで現在を合格とはしない。さらに、コードコメントには `PLAN_BUDGET` が6,000msと書かれている箇所がある一方（`crates/ori3-propose/src/search.rs:501`）、実値は30,000msである。性能契約と文書の同期も必要である。

### 5.10 アーキテクチャ: 6/10

Rustの責務分割、Zustandへの状態一元化、型付きIPC、直列queueと `runLatest`、generation tokenによる古い応答破棄、BTree系の決定性、WebGL資源の明示解放は良い。

ただし、巨大storeと巨大UIファイルは境界を弱める。`appStore.ts` は状態定義、IPC調停、履歴、提案、replay、UI actionを同居させている。`surface_order_acceptance.rs` 5,918行、`replay.rs` 2,799行等、テスト・中核の失敗局所化も難しい。機能別slice、command service、selector、test fixture builderへ分割し、外部公開APIを狭くする必要がある。

### 5.11 テスト・CI: 7/10

テスト量と種類は強い。前回のRust 758属性から858へ100増え、フロントテストファイルも103から109へ増えた。単体だけでなく、作品fixture、決定性、面順、seam、penetration、メモリ、性能、DOM、packageを扱う。

一方、テスト数の多さは現在の赤いゲートを相殺しない。グローバル進捗値により並列テストが干渉し、壁時計で結果が変わる。依存脆弱性監査、CodeQL、SBOM、ライセンス検査、axe等の自動アクセシビリティ検査は設定から確認できなかった。

### 5.12 文書・進捗: 6/10

要件は数値条件まで詳細で、進捗には失敗と測定が豊富に残る。コードコメントも「なぜその上限か」を多く記録する。ヘルプとPDFを同じ型付き内容源から生成する設計も二重管理を減らす。

弱点は、ロードマップの未完了checkbox、実装済み記述、実際のコードが同期していないこと、巨大な時系列進捗から現在の契約を探しにくいこと、前述の6秒/30秒のようにコメントが陳腐化していることである。`docs/current-status.md` のような1ページの機械生成サマリをCIで更新・検証すべきである。

## 6. 競合比較

### 6.1 比較に使った公式情報

- [Oriedita Getting Started](https://oriedita.github.io/getting-started.html): `.ori`、FOLD、`.cp` の保存、SVG/PNG/JPEG/CP等の書出し。
- [Oriedita Download](https://oriedita.github.io/download.html): Windows、Linux、macOS、Java jarの配布。
- [ORIPA公式リポジトリ](https://github.com/oripa/oripa): CP入力、折り上がり計算、OPX/CP/FOLD変換、CLI、plugin。
- [Origami Simulator公式リポジトリ](https://github.com/amandaghassaei/OrigamiSimulator): 全折り目の同時折り、GPU計算、curved crease、SVG/FOLD入力、FOLD/STL/OBJ出力、strain表示。
- [TreeMaker公式ページ](https://langorigami.com/article/treemaker/): tree、flap長・接続・制約・対称性からbase CPを計算し、手順は利用者が考える必要がある。
- [Box Pleating Studio公式サイト](https://bp-studio.github.io/) / [公式manual](https://bp-studio.github.io/manual.html): box pleating/GOPS設計支援。出力CPはflat-foldableを保証する目的ではない。
- [Freeform Origami公式ページ](https://origami.c.u-tokyo.ac.jp/~tachi/software/): developability、flat-foldability、facet planarity、点一致、紙サイズを保つ対話設計。商用利用は別許諾。
- [FOLD 1.2公式仕様](https://github.com/edemaine/fold/blob/main/doc/spec.md): CP、folded form、層順、複数frame、animation/diagramを表現できる共通JSON形式。

### 6.2 機能軸比較

「強い/弱い」は上記公式機能とORIGAMI3コードの対応から行った比較評価であり、市場シェアの推測ではない。

競争力の総合点は**6.9/10**とする。前回6.5/10から0.4上昇した。内訳はCP編集8、剛体・層9、手順9、提案7、成果物8、交換形式2、対応環境3、UX7、品質証拠7、商用展開性9の単純平均である。3D・手順・検証証拠は上がったが、FOLD非対応とWindows/日本語限定が全体点を強く抑えている。市場採用数は確認できないため採点軸に含めていない。

| 軸 | ORIGAMI3 | 主な競合 | 判定 |
|---|---|---|---|
| CP編集・作図 | 交差分割、重複処理、曲線、作図法、flatfold警告 | Oriedita/ORIPAは長期のCP編集と交換形式を持つ | 幾何中核は強いが互換性と成熟実績で劣る |
| 平坦折り結果 | 剛体3D、閉路、層順、seam、接触警告 | ORIPA/OrieditaはCPからfolded formを計算 | ORIGAMI3は連続姿勢・接触まで広い |
| 動的シミュレーション | 手順単位の剛体motion、見た目soft | Origami SimulatorはGPU同時折り・strain・曲線折り | 順次折りはORIGAMI3、物理的視覚化とWeb即時性は競合 |
| 手順編集 | 手順記録、並替え、途中再生、8名前付き技法 | TreeMakerはCPを出すが手順は利用者が考える | **明確な優位**。ただしmoveStep原子性が弱い |
| 折り図生成 | 6コマPDF/SVG、自動日本語説明、注記 | 比較対象の主目的はCP/シミュレーションが中心 | **明確な優位**。実用成果物まで一体化 |
| 逆設計・提案 | skeleton→packing→CP→探索→21姿勢検証 | TreeMakerはtree theory、BP Studioはbox pleatingに特化 | 統合検証は独自。専門領域の蓄積・標本数は競合 |
| 自由形状拘束 | CPを動かしてsolve/replay | Freeformは可展性・平坦可折性・平面性等を保って形状編集 | 自由形状設計はFreeformが明確に強い |
| 交換形式 | `.ori3` と画像/PDF中心 | Oriedita/ORIPA/Simulator/FreeformはFOLD等で連携 | **最大の競争劣位** |
| 対応環境 | Windows 10/11、日本語 | OrieditaはWin/Linux/macOS/jar、Simulator/BPはWeb | 導入可能範囲で劣る |
| 品質証拠 | 大量の作品・決定性・性能・DOMテスト | 競合の公式説明だけでは同じ粒度を比較不能 | 内部証拠は強いが、現在のgate失敗が信用を損なう |
| ライセンス | MIT | ORIPA GPL、Freeformは商用別許諾 | 組込み・商用展開では有利 |

### 6.3 競争優位

#### 優位1: 手順を第一級データとして扱う

Origami Simulatorは公式に全折り目を同時に折る方式で、順次手順ではない。TreeMaker公式も、出力CPから手順を考えるのは利用者だと説明する。ORIGAMI3は `FoldStep`、`SeqOp`、replay、途中姿勢、名前付き技法、PDFを同じモデルでつなぐ。これは比較対象の空白を埋める機能である。

#### 優位2: 「完成案」の誤表示を型で抑える

提案はpacking結果をそのまま完成と表示せず、各手21姿勢と最終gapを再検査し、`CheckedToFinish` をprivate型で発行する。TreeMaker/BP Studioの設計支援に対し、「生成後に実際の手順として検査する」統合は強い。ただし現在のTimeCap回帰を直すことが前提である。

#### 優位3: 物理順を証拠付きにする

面IDや描画順を層順とみなさず、全overlapが解決した場合だけopaqueな権威型を作る。soft補正も品質悪化時に破棄する。赤い裏面、紙の突抜け、層順の誤表示を局所patchではなく型と幾何で抑える設計は技術的に価値が高い。

#### 優位4: 日本語の一体型デスクトップ製品

CP、3D、手順、提案、保存復旧、説明書まで同一UIにあり、Tauriのinstaller/MSI/portableと82ページ説明書を持つ。研究用prototypeを利用者向け配布物へ仕上げる費用が既に投入されている。

### 6.4 競争上の弱い部分

#### 弱点1: FOLD非対応

FOLDはCP、folded form、層順、frame列を表せ、Oriedita、ORIPA、Origami Simulator、Freeform等が連携に利用する。ORIGAMI3がこれを扱わないため、既存資産の入口と外部検証の出口がない。現在の要件では非目標だが、競争上は最優先の欠落である。

#### 弱点2: 提案の一般化証拠が狭い

既知3作品の3/3成功は前回より大きな進歩だが、tree theoryまたはbox pleatingの設計道具として信頼を得るには少ない。少なくとも30作品を、葉数、対称性、目標位置、技法、難度で層別し、成功率、gap、時間、停止理由を公開する必要がある。

#### 弱点3: 実行結果が機械負荷に依存する

壁時計30秒で `TimeCap` にする現仕様は、同じ入力に同じ答えという製品の主張を弱める。特に提案は長時間計算なので、CPU差・並列負荷差が利用者に見える。これは競合比較以前の製品信頼性問題である。

#### 弱点4: 配布範囲

Windows 10/11・日本語のみは要件通りだが、Orieditaの複数OS、Simulator/BP StudioのWeb実行に比べ入口が狭い。研究者や海外設計者から検証データを得にくく、FOLD非対応と相乗してecosystem形成を阻む。

#### 弱点5: 物理・自由形状・専門設計では専用品に及ばない

ORIGAMI3のsoftは視覚近似であり、GPU strain表示や材質解析ではない。Freeformのような可展性等を保持した自由形状変形でもない。TreeMakerの長期tree theory、BP StudioのGOPS/box pleating専門性と同一の深さを実証したわけでもない。ORIGAMI3の勝ち筋はそれら全てを浅く模倣することではなく、手順・検証・折り図までの一貫性である。

## 7. 外注再構築費の詳細

### 7.1 見積範囲

含めるもの:

- 要件整理、数学・折り紙ドメイン設計
- 現在存在する製品コードと同等機能
- 現在存在する自動テスト、性能試験、CI、Windows配布
- UI、ヘルプ、82ページ説明書生成
- PM、レビュー、統合、不具合修正

含めないもの:

- 販売、広告、顧客獲得、問い合わせ運用
- 今後の保守、クラウド費、利用者が判断する有料サービスの費用、法務
- 会社・ブランド・特許・将来収益の価値
- 現コードにないFOLD、macOS正式対応、多言語、exact物理

### 7.2 人月の積上げ

| WBS | 人月範囲 | コードに基づく対象 |
|---|---:|---|
| 要件・折り紙ドメイン・全体設計 | 30～35 | 詳細要件、操作モデル、数値契約、9クレート境界 |
| model / geometry / CP | 42～48 | f64モデル、graph、交差・重複、面、曲線、作図、検証 |
| rigid / contact / surface order | 68～78 | 閉路solver、Jacobian、疎直接法、motion、交差、seam、権威順 |
| layers / replay / 8技法 | 60～68 | step再生、flat/spatial fold、compound、層、技法、cache |
| propose | 72～82 | skeleton、packing、CP生成、列挙、探索、21姿勢検証、UI変換 |
| soft表示 | 18～23 | mesh細分、PBD近似、curl/cup/symmetry、overlap品質ゲート |
| フロントUI・2D・3D・状態 | 58～66 | React、Zustand、CP操作、Three.js、dialog、テーマ、help |
| export / 保存 / Tauri / 説明書 / 配布 | 35～40 | SVG/PNG/PDF、autosave、履歴、installer、manual、release |
| QA・fixture・性能・CI | 62～68 | 100,742行に含まれるRustテスト、37,362行のフロントテスト、CI |
| PM・統合・レビュー・文書 | 30～35 | 横断調整、受入、リリース、進捗・仕様同期 |
| **合計** | **475～543人月** | IPA換算の1人月160時間なら76,000～86,880人時 |

この人月は行数を単純除算したものではない。数値solver、surface order、提案探索は少ない行でも設計・検証費が大きく、CSSやfixtureは行当たり費用が低いので、責務単位で積み上げた。1人月=160時間は[IPAのデータ白書FAQ](https://www.ipa.go.jp/archive/publish/wp-sd/qa.html)の換算に合わせた。IPAの[ソフトウェア開発分析データ集2022](https://www.ipa.go.jp/digital/software-survey/metrics/metrics2022.html)も、工数、規模、生産性、信頼性を別指標として扱うため、SLOCだけの価格化はしていない。

### 7.3 単価感度

実契約単価は発注規模、請負責任、数値計算人材、元請階層、知財帰属で変わる。したがって1本の単価を事実とせず、感度を示す。

| 混成チーム単価 | 475人月 | 543人月 | 中央509人月 |
|---:|---:|---:|---:|
| 150万円/人月 | 7.13億円 | 8.15億円 | 7.64億円 |
| **170万円/人月（基本ケース）** | **8.08億円** | **9.23億円** | **8.65億円** |
| 200万円/人月 | 9.50億円 | 10.86億円 | 10.18億円 |

170万円は相場の断定ではなく、この試算の比較用入力である。外部整合性の確認として、JISAの[2024年版情報サービス産業基本統計調査](https://www.jisa.or.jp/Portals/0/report/basic2024report.pdf)は、調査295社の従業員1人当たり年間売上高を2,951.3万円、すなわち月換算約245.9万円と報告する。ただしこれは会社全体の売上指標であり、技術者の請求単価ではないため、そのまま単価には使っていない。

### 7.4 前回7億円からの上昇

基本ケースとの差は次の通りである。

| 比較 | 金額 |
|---|---:|
| 前回提示値 | 約7.0億円 |
| 現在下限 | 8.08億円（+1.08億円、+15.4%） |
| 現在中心 | 8.65億円（+1.65億円、+23.6%） |
| 現在上限 | 9.23億円（+2.23億円、+31.9%） |

前回レビューから現在までに、コードで確認できる価値上昇要因は次である。

| 上昇要因 | 前回状態 | 現在確認できる証拠 |
|---|---|---|
| 3D接触と層順 | SIM 6/10。接触補正・順序の信頼性が課題 | opaqueな権威順、全overlap解決条件、soft候補品質ゲート |
| 仕上げ値保存 | 手順単位保存が不足 | `FoldStep.finish_soft` と旧形式default、再生テスト |
| 3D保存境界 | 公開製品APIにgeometry snapshot懸念 | 通常Documentは設定だけ保存。頂点fixtureはテスト支援へ限定 |
| 提案完成表示 | bool/途中案の境界が課題 | private `CheckedToFinish` と21姿勢・最終gap検査 |
| 既知作品提案 | 完成保証が弱い | 進捗記録上3/3到達、10/10決定性。今回TimeCap回帰は残る |
| surface order | 個別の赤面・裏表問題 | 23/23 overlap対と多数視点の受入資産 |
| 製品仕上げ | 起動・画面・manual証拠不足 | 起動実測、100画面状態、82ページmanual、Windows package |
| テスト量 | Rust 758、フロント103ファイル | Rust 858、フロント109ファイル、実行1,719件 |

上昇はコード量の増加だけでなく、「完成と途中」「物理順と便宜順」「設定と頂点」「警告と補正」の境界を型と検査で固定したことによる。反対に、TimeCap回帰、巨大ファイル、交換形式欠如は増価を抑える要因として既に控除した。

## 8. 100点へ向けた具体的改善計画

100点は「バグが未来永劫0」という意味ではない。現在の要件を満たし、主要主張が再現可能な検査で裏付けられ、重大な既知欠陥と文書不整合がない状態と定義する。

### P0: リリースを止める問題

#### 1. 提案探索を負荷非依存にする

- `SearchDeadline` を製品の通常停止理由から分離する。
- 決定的budgetで同じ候補・順序・停止理由を返す。
- watchdog超過はキャンセル/内部エラーとして別型にする。
- proposal progressを `{job_id, done, total, phase}` のTauri managed stateへ移す。
- 同時2要求、高CPU負荷、1候補/4候補、debug/releaseで結果一致を検査する。

合格条件:

- 現在失敗する3テストが100回連続で合格。
- `cargo test --workspace` のCI相当コマンドが合格。
- 同入力10回、並列/直列、低負荷/高負荷で候補JSONと停止理由が一致。

期待点: PRO +1、NFR +1、Test +1。

#### 2. 手順移動を原子的にする

- `SeqOp::MoveStep { id, to_index }` をmodel/backend/frontendへ追加。
- remove/insert、再生検証、履歴pushを1トランザクションにする。
- 失敗時document、undo/redo、dirty、step_creasesがbit同一であることを検査する。

合格条件:

- 1操作=Undo 1回。
- IPC失敗を注入しても手順が消えない。
- 移動後の全step creaseとreplay終点が一致。

期待点: SEQ +1、SYS +0.5。

### P1: 品質の再現性と保守性

#### 3. 提案benchmark corpusを30作品へ拡張

- 3～12葉、対称/非対称、位置制約あり/なし、simple/compound技法を層別する。
- 入力、期待gap、停止理由、最大時間、決定性hashをversion管理する。
- 「完成率」だけでなくpartial時の改善率と安全性も出す。

合格条件:

- 30作品全てでfinite、裂け `<=1e-6`、penetration 0。
- 対象と定めた完成標本は10/10で `CheckedToFinish`。
- release CIの中央値・P95と基準機情報を公開。

#### 4. 巨大境界を分割

- `appStore.ts`: document、cp、pose/replay、proposal、dialogs/settingsのsliceへ分ける。
- IPC queue、generation token、historyをservice化する。
- `Viewer3D` をscene lifecycle、interaction、overlay、cameraへ分ける。
- `App.css` をcomponent/theme/layoutへ分け、CSS layerまたはmoduleで所有者を明示する。
- `surface_order_acceptance.rs` を契約別fixtureへ分割する。

合格条件:

- 製品TS/TSXの単一ファイル上限1,500行、store slice上限1,000行。
- 循環import 0、公開selectorの型テスト、全既存テスト維持。
- `App.tsx` 自己規約どおり200行以内。

期待点: Architecture +2、Docs +0.5。

#### 5. ダイアログの共通アクセシビリティ基盤

- `ModalDialog` にinitial focus、Tab循環、Escape、focus return、背景inertを実装。
- New/Export/Proposal/Recovery/Helpで共用する。
- axe-coreとkeyboard-only DOM検査をCIへ追加する。

合格条件:

- 自動axe重大違反0。
- mouseなしで全dialogを開く、操作する、閉じる、元要素へ戻る。
- 200%拡大、1000×700で操作要素欠落0。

期待点: UI +1。

#### 6. bundleを分割

- Proposal、Help、Export、manual previewをdynamic importする。
- Three.js周辺を独立chunkにし、起動直後不要な生成器を遅延する。
- CIへgzip budgetを追加する。

合格条件:

- initial JS gzip 250kB以下、単一minified chunk 500kB以下。
- 起動中央値・P95が現状を悪化させない。

期待点: NFR +0.5、UI +0.5。

#### 7. 文書を実装から機械検証する

- version、workspace数、Tauri command数、テスト数、budget、manualページ数をスクリプト生成する。
- roadmap checkboxを現在の受入testへリンクする。
- code commentに測定値を複製せず、benchmark JSONまたはdocsの一箇所へ寄せる。

合格条件:

- 6,000ms/30,000msのような不一致0。
- CIで文書生成差分0を検査。

期待点: Docs +2。

### P2: 競争力を満点へ近づける拡張

#### 8. FOLD 1.2のimport/export

現要件の非目標を正式に改訂した上で実施する。最初はCP、M/V/B/F/U、2D座標、fold angle、face order、step frameに範囲を限定し、unsupported fieldは警告する。ORIPA/Oriedita/Origami Simulatorとのround trip corpusを持つ。

合格条件:

- 公式sampleと競合出力30件をimportしてpanic 0。
- ORIGAMI3→FOLD→ORIGAMI3でCP topology、M/V、step終点、層制約が許容差内一致。
- 未対応fieldを黙って捨てず一覧表示。

これは現要件の点数というより、最大の競争劣位を解消する施策である。

#### 9. 実利用者検証

初心者5名、CP経験者5名、設計経験者5名を最低構成とし、新規作成、折り線追加、3D折り、手順記録、PDF出力のタスク成功率、時間、誤操作、質問回数を測る。

合格条件:

- 基本5タスクの初回成功率90%以上。
- 重大な行き止まり0、専門語で止まる操作0。
- 観測結果と修正を匿名化してdocsへ残す。

期待点: UI +0.5、NFR/製品証拠 +0.5。

#### 10. supply-chainと配布保守

- Dependabot/Renovate、`cargo audit`、npm audit方針、CodeQL、SBOM、license allowlistを追加。
- updaterを導入する場合は署名、rollback、失敗時継続を受入条件にする。

合格条件:

- release artifactごとのSBOMとhashを公開。
- critical/high脆弱性0、例外は期限・理由付き。

期待点: Test +1、SYS +0.5。

## 9. 推奨実施順と到達点

| 順序 | 施策 | 完了後の目安 | 理由 |
|---:|---|---:|---|
| 1 | 提案の決定性・job別progress | 82点 | 現在のrelease blockerを除く |
| 2 | 原子的MoveStep | 84点 | データ消失可能性とUndo不整合を除く |
| 3 | 30作品corpus | 87点 | 提案の主張を一般化可能な証拠へ変える |
| 4 | store/UI分割、bundle分割 | 92点 | 保守性と起動資産を改善する |
| 5 | modal a11y、利用者テスト | 95点 | コードで未確認だった操作性を実証する |
| 6 | 文書自動同期、security CI | 98点 | 現状と説明のずれ、供給網の空白を塞ぐ |
| 7 | FOLD round trip | **100点相当の競争状態** | ecosystem上の最大弱点を解消する |

点数は各施策の実装着手ではなく、記載した合格条件を満たした時だけ上げる。

## 10. 最終評価

ORIGAMI3は、約18万物理行と非常に大きな自動テスト資産を持つだけでなく、折り紙固有の難所である閉路拘束、面の上下、部分層、複合技法、完成提案の検証を、型と数値条件で扱っている。特に「手順を保存し、途中を再生し、人向けの折り図へ出す」一本の流れは、CP編集、同時折りシミュレーション、tree設計へ分かれがちな競合群に対する本物の差別化である。

一方、現時点で最優先すべきなのは新機能ではない。提案結果を壁時計とグローバル進捗から切り離し、同じ入力が機械負荷に関係なく同じ結果になるよう直すことである。その次が手順移動の原子化、30作品corpus、巨大境界分割、アクセシビリティ、FOLD互換である。

したがって、現在は「技術的に独自で高価値だが、品質ゲートが赤い高度なβ製品」と評価する。**78/100、外注再構築費8.1億～9.2億円、前回7億円比の中心増加約1.7億円**が、確認できたコード・テスト・公式競合情報から導ける妥当な結論である。
