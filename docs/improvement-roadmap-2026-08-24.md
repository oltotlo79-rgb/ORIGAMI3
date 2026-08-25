# ORIGAMI3 改善ロードマップ（2026-08-24）

対象: ORIGAMI3 0.5.0（Rust + Tauri 2 + React 19）  
根拠レビュー: `docs/comprehensive-review-2026-08-24.md`（対象版0.4.5）  
作成時HEAD: `eb909d3`  
行番号: 特記がなければ作成時HEADのUTF-8ファイルを1始まりで数えたもの  
作業範囲: 調査と実行計画の作成のみ。本書作成時には実装、検査実行、ブラウザ起動、`desktop.exe`起動、git書込みを行っていない。

## 0. レビュー後に既に解決した項目

直近5件は、`eb909d3`、`728389c`、`4f35294`、`b7fadf1`、`85b7f0d` である。0.4.5レビュー後の中心修正は `4f35294 提案が3つの標本すべてで目標の形に届くようになり、待ち時間と表示の不具合を直した` である。今回は再検査を求められていないため、次の「合格」は現行ソース、コミット記録、CI定義、`docs/progress.md` の実測を照合した判定であり、この調査中にテストを再実行したという意味ではない。

### 0.1 解決済み。これからの実装項目へ戻さないもの

| # | 0.4.5レビューの指摘 | 0.5.0で確認した実体 | 判定 |
|---:|---|---|---|
| 1 | `commands::tests::proposal_progress_counts_every_candidate` が失敗 | `apps/desktop/src-tauri/src/commands.rs:947-983` の `ProposalProgressCell::{new,start,finish_one,snapshot}` が進捗を1セルへ集約し、同ファイル `:998-1003` の `CandidateTicket::drop` が成功・失敗・panicを問わず候補1件を加算する。`generate_candidates` は進捗セルを引数で受ける（`:1154-1209`）。対象テストは製品大域値でなく専用セルを作る（`:1584-1598`）。panic経路も別検査がある（`:1606-1617`）。 | **テスト干渉の原因は解決済み。** 施策1ではこの仕組みを作り直さず、製品のjob別状態だけを扱う。 |
| 2 | `commands::tests::the_heaviest_proposal_never_hits_the_time_limit` が失敗 | テストは12先端・`PLAN_BUDGET`で `SearchStop::TimeCap` 0件を検査する（`apps/desktop/src-tauri/src/commands.rs:1766-1777`）。通常の `checks` ではこの1件だけを除外し（`.github/workflows/ci.yml:60-67`）、`performance` job（`:84-86`）が `cargo test --release -p desktop --lib the_heaviest_proposal_never_hits_the_time_limit -- --nocapture` を実行する（`:149-150`）。`CLAUDE.md:189,193` の表#21とも一致する。最適化ありの記録は10回中TimeCap 0、最大13.851秒である。 | **所定のrelease性能ゲートとして解決済み。** debugでも30秒以内になった、という判定ではない。通常ジョブへ戻したり、最適化を外したりしない。 |
| 3 | `commands::tests::proposal_candidates_are_the_same_computed_together_or_one_by_one` が失敗 | `apps/desktop/src-tauri/src/commands.rs:1448-1451` の `TIME_FREE_PLAN_BUDGET` は `max_millis=3_600_000`、その他を `PLAN_BUDGET`から継承する。並列版と直列版は同じbudgetを使い（`:1545-1558`）、件数と候補の完全一致を検査する（`:1559-1569`）。 | **壁時計による検査flakeは解決済み。** このテスト専用budgetを製品budgetへ流用しない。 |

これら3件は、将来変更による後退を検出する回帰ゲートとして100回確認の対象には残すが、「未修正の3不具合」として工数を計上しない。

### 0.2 まだ残っている項目

| # | 現在の状態 | 利用者に届く影響 | 本書での扱い |
|---:|---|---|---|
| 4 | `ProposalProgressCell` の中には `done` と `total` の2つの `AtomicUsize` があり（`apps/desktop/src-tauri/src/commands.rs:947-952`）、製品用 `static PROPOSAL_PROGRESS` は依然1個だけである（`:985-989`）。`proposal_generate` が同じセルへ書き（`:1124-1145`）、引数なしの `proposal_progress` が読む（`:1030-1033`）。`apps/desktop/src/ipc/client.ts:119-129` も `done/total` だけで、対象ソースに `job_id` / `jobId` / `phase` はない。 | 同時2要求を識別できず、一方の進捗を他方が読める。2原子値を別々に読むため、同一時点の一貫したsnapshotも型では保証されない。 | 施策1のjob別Tauri managed state、`{job_id, done, total, phase}`、終了後の回収へ限定して実施する。 |
| 5 | `PLAN_BUDGET` は `max_states=2`、`branch=2`、`max_millis=30_000`（`apps/desktop/src-tauri/src/commands.rs:892-899`）。意図は実測最大13.851秒に対する約2.2倍のハング用安全弁（`:869-887`）である。一方、`crates/ori3-propose/src/search.rs:990-993` は `SearchDeadline` を作り、期限到達で `SearchStop::TimeCap` にする（`:1017-1019,1034-1038,1073-1082`）。`crates/ori3-propose/src/verify.rs:437-452` がそれを `VerifiedPlan::Partial` へ入れ、`ProposalFoldPlan::from_verified` が画面用 `partial` へ変換する（`apps/desktop/src-tauri/src/commands.rs:802-815`）。 | `SearchStop::TimeCap` という理由名自体はIPCへSerializeされないが、壁時計が手順内容と `partial` / 提案なしを変える。したがって製品結果はまだ負荷非依存ではない。 | 施策1で、決定的探索結果とwatchdog/cancel/internal errorを別の型・別の経路にする。単に30,000を延ばす修正にはしない。 |

追加の同期不良として記録した、`crates/ori3-propose/src/search.rs` の画面用budget説明は現在30,000 msへ同期済みである。旧6,000 msと実値30,000 msの不一致は施策7で解消した。

## 0A. 利用者が決めたこと（2026-08-24・25）

次の6件は、**2026-08-24に利用者が決定した**。判断7は**2026-08-25に利用者が決定した**。以後の委譲では再び「判断待ち」へ戻さず、承認範囲だけを実装候補にする。決定は本書へ記録するものであり、本書作成時に要件正本、作業規約、コード、workflowを変更したという意味ではない。

| # | 項目 | 決めた範囲 | 決めなかった／範囲外としたもの |
|---:|---|---|---|
| 1 | FOLD 1.2 | **限定profileを承認。** 2D `vertices_coords`、edge topology、B/M/V、`edges_foldAngle`、表現可能な非循環 `faceOrders`、線形step frameを対象にする。F/Uは`Aux`へ縮退し、元指定を警告として残す。 | **F/U完全往復の41～62人日案は不採用。** 3D座標、枝分かれした手順、動画、名前付き技法の意味、注記、仕上げの丸み、FOLDのF/U区別は対象外。画面と文書で必ず「FOLD 1.2 限定」と表示し、「FOLD対応」「FOLD完全対応」とは表示しない。 |
| 2 | 実利用者15名 | **準備だけを承認。** 課題文、同意文、集計様式、観察手順、keyboard-only自動検査、拡大200%自動検査までを作る。 | 15名sessionの実施は利用者の都合がつく時まで保留する。準備完了や自動検査を15名の実施に代えず、`session 0/15`の間はUI・NFRの点数を上げない。 |
| 3 | supply-chainと配布保守 | **監視と公開までを承認。** 脆弱性の自動監視、依存更新の提案、静的解析、4配布物のSBOMとSHA-256の公開、license allowlistを対象にする。依存更新は人がreviewし、自動で取り込まない。 | **アプリの自動更新機構は範囲外。** 署名鍵管理、rollback、更新失敗時の継続を別途設計しない限り導入しない。本施策のN/A条件や将来オプションにも含めない。 |
| 4 | Linux対応 | **見送る。** GUIを使えるLinux環境（実機またはVM）を用意できず、全機能を確認できない。利用者基準「確実にLinuxで全ての機能が使えないなら対応しない」に従う。 | 技術的に不可能だからではない。再開条件を満たすまで、実装・要件改訂・工数計上をしない。対応環境3/10は据え置く。 |
| 5 | 全部の折り目を一斉に折る | **案Bを承認。** つまみ1本で0～100%を動かす一時表示を実装候補とする（8～12人日）。 | **案A（100%だけ）と案C（形を手順として残す）は不採用。** 通常の手順を形から起こす案も採らない。重なり順の無い形を、通常の手順と誤認させる危険があるため。 |
| 6 | リリースの範囲 | **施策9は準備までを完了とみなして今版をリリースする。** 施策1〜8、10、11は全単位の合格が必要であり、施策12は見送りのまま条件外とする。 | 施策9の外部実施5単位・7～12人日は**次の版で必ず実施する繰越**であり、やらないことにはしない。外部参加者15名が完了するまで、施策9ぶんのUI・NFR加点は0とする。 |
| 7 | コード署名 | **2026-08-25から当面は未署名のまま配布する。** Windowsの警告は、理由と安全な導入手順を丁寧に案内する。公開実績が積み上がった時点で、無料の署名プログラムへ改めて申請する。 | 有料の署名（サブスクリプション、証明書の購入、他社サービス）は採らない。費用が発生する判断は利用者が行う。時期は約束しない。 |

### 0A.1 FOLD 1.2 限定表示の利用者約束

施策8の全ての画面、警告、Help、書出し選択、利用者向け文書は、機能名を**「FOLD 1.2 限定」**とする。利用者から見える対応外一覧には、3D座標、枝分かれした手順、動画、名前付き技法の意味、注記、仕上げの丸み、FOLDのF/U区別の7項目を省略なく出す。F/Uは`Aux`へ縮退し、JSON pathと元assignmentを警告へ残す。黙って捨てる経路を1件も許さない。

### 0A.2 Linux対応を見送る

利用者の判断基準は「**確実にLinuxで全ての機能が使えないなら対応しなくていい。全て確認までできるなら対応したい**」である。GUIを使えるLinux環境（実機またはVM）を用意できないため、全機能の確認はできない。したがってLinux対応は**見送る**。これは技術的に不可能だからではない。

- 調査で分かった良い材料: 製品コードにWindows専用API、レジストリ、Windows固定パス、Windows専用crateの直接依存は見つからなかった。保存処理はTauriとRustのOS非依存APIを使う。Tauri 2はdeb / rpm / AppImageを正式なbundle targetとして持つ。
- 実行確認は**1件もしていない**。未確認はLinuxでのbuild、起動、3D表示、入力、保存、フォント、性能である。
- 再開条件は、Linux対応可否調査§1の5条件を全て満たすこととする。(1) 実装前に要件正本を改訂し、対象distribution、CPU、配布形式、検査条件、対象外を固定する。(2) Linux対応の施策と別工数をロードマップへ置く。(3) Linux runnerとGUI付きLinux実機またはVMを用意し、WebKitGTK、Three.js/WebGL、日本語フォント、保存・自動保存・復旧、配布物起動を実測する。(4) 施策4・施策6の後に正式認定し、同じreleaseへFOLDを含める場合はFOLD実装凍結後にも最終回帰を行う。(5) 施策10のSBOM/SHA-256をLinux成果物を含む確定済み成果物集合へ作り直す。
- 再開時の参考見積は、案A（AppImage 1形式）が27～46人日、案B（deb + AppImage）が37～60人日である。見送り中は総見積へ加えない。
- 競争評価の**対応環境3/10は据え置き**とする。

### 0A.3 全部の折り目を一斉に折る案Bの利用者約束

承認範囲は、つまみ1本で0～100%を動かす案Bの一時表示だけである。案A（100%だけ）と案C（形を手順として残す）は採らない。さらに、重なり順の無い形を通常の手順と誤認させる危険があるため、形から通常手順を起こす案も採らない。

- 一斉折りは手順に記録せず、保存せず、Undo/Redoの対象にしない。割合、一時角、診断、3D状態も保存しない。
- 一斉折り中の画面には、**「これは記録された手順ではない」**ことを利用者が一目で分かる表示として常に示す。
- §4.2の非目標には抵触しない。ただし実装前に、§4.1へ正の要件を追加する必要がある。

### 0A.4 リリースの範囲

利用者の「リリースは全ての修正が終わってから」という指示について、参加者募集によって無期限に停止しないよう、今版では次をリリース完了条件とする。

1. 施策1〜8、10、11の**全単位**が合格条件を満たすこと。
2. 施策9は、準備3単位・6～9人日（課題文、同意文、集計様式、観察手順、keyboard-only/200%自動検査）までを完了とみなすこと。
3. 施策9の外部実施5単位・7～12人日は**次の版で必ず実施する**。今版で外部参加者15名を完了できなくても「やらない」とは扱わない。
4. 施策12（Linux）は見送りのまま、今版のリリース条件に含めない。

施策9は、外部参加者15名が完了するまでUI・NFRの点数を上げない。したがって、今版のリリース時点で施策9ぶんの加点は**0**である。

### 0A.5 コード署名を当面見送る

**判断日: 2026-08-25。** SignPath Foundationの無料署名プログラムは不承認だった。不承認の理由は品質ではなく、公開実績の不足である。先方は「これは仕事の質や可能性への評価ではない」と明記している。

- 先方が確認する材料は、GitHubのstar・fork・contributor、外部記事、第三者の言及、機関の後ろ盾、継続的な活動の証跡である。
- 再申請は可能である。これらの公開実績が積み上がった時点で改めて申請するが、時期は約束しない。
- 当面の配布物（`setup.exe`、`.msi`、`portable.exe`）は未署名のままとする。Windowsの警告が出ることと、安全に導入する手順をREADMEとリリース本文へ明記する。
- 有料の署名（サブスクリプション、証明書の購入、他社サービス）はいまは採らない。費用が発生する判断は利用者が行う。

## 0B. 正本へ反映する改訂案（本書作成時には変更しない）

**適用済み（2026-08-24、受入基準追記）**: §0B.2および§0B.3を、利用者承認済みの改訂案どおり要件正本へ反映した。同じ改訂に、§12.2のFOLD-001～006およびM7を反映した。

### 0B.1 `CLAUDE.md` §2の改訂案

実該当箇所は `CLAUDE.md:28-40` であり、モデルを必ずSol Ultraとする文と起動例は同 `:30-34` である。**利用者が正本改訂を行うときは、`:28-40`を次の実文で置き換える。** 本書作成時には`CLAUDE.md`を変更しない。

````markdown
## 2. Codexの起動と使い方

- 委譲する各作業単位について、次の判定表で担当モデルを選ぶ。

| 判定 | モデル | 選ぶ基準 |
|---|---|---|
| **単純作業** | **terra** | 合格条件が数値で決まっていて、手順どおりに実行すれば終わる作業。一括置換、文言の差し替え、機械的なファイル分割、検査の実行、集計、体裁そろえ、画像の撮り直し、文書の生成を含む。 |
| **判断が要る作業** | **sol ultra** | 何が正しいかを決める必要がある作業。設計変更、幾何や数値の正しさ、原因の特定、複数ファイルにまたがる仕様変更を含む。 |

- **迷った場合はterraを選ぶ。** 委譲ごとに、選んだモデルと理由を「`terra: 対象の関数名と数値条件が確定しており判断が不要`」または「`sol ultra: 複数の表現から正しいデータ契約を決める必要がある`」のような1行で指示書と利用者への報告に記す。
- 1回の委譲に単純作業と判断が要る作業が混在する場合は、別の達成単位へ分ける。分けられない場合だけ委譲全体をsol ultraとする。terraの作業中に未定義の設計判断、幾何・数値差、原因不明の回帰が生じた場合は勝手に判断せず停止し、その判断部分だけをsol ultraの別作業として切り出す。
- terraはリポジトリのルートで次の形で起動する。

```text
codex exec --model gpt-5.6-terra -c model_reasoning_effort=medium --sandbox workspace-write --skip-git-repo-check - < 指示ファイル
```

- sol ultraはリポジトリのルートで次の形で起動する。

```text
codex exec --model gpt-5.6-sol -c model_reasoning_effort=ultra --sandbox workspace-write --skip-git-repo-check - < 指示ファイル
```

- 作業フォルダを一時フォルダへ移さない。リポジトリ外から起動すると、対象ファイルへの書き込み権限を失うことがある。
- 可能な限り複数エージェントを並列で起動する。ただし、工程や検査を飛ばしたり、品質を落として速度を稼いだりしない。
- 並列実行する各エージェントの指示には、**触ってよいファイル**と**絶対に触らないファイル**を実パスで明記する。作業ツリーを共有するため、担当の重複も避ける。
- Codexにcommit、push、checkout、merge等のgit操作をさせない。変更内容と対象ファイルだけをClaudeへ報告させる。
- 理由: 作業内容に合うモデルを使い、誤った作業場所による書き込み失敗、共有ツリー上の変更衝突、Codexの`.git`書き込み失敗を避けるため。
````

なお、`CLAUDE.md:483-485`には旧モデル名と「迷ったらSonnet」が残る。§2だけを変えると矛盾するため、同じ正本改訂で`:483-485`も次の実文へ置き換える。

```markdown
     **必要になった作業を、§2の判定表に従って選んだサブエージェントへ出す。**
  3. **サブエージェントを起動するたびに、§2の判定表でモデル（terra / sol ultra）を選び、
     選んだ理由を利用者への報告に1行で書く。書けないならterraを選ぶ。**
```

### 0B.2 `docs/requirements-definition.md` §4.2の改訂案

`docs/requirements-definition.md:386`は、§4.2の非目標リストを変える場合に要件書自体の改訂を必須とする。これに従い、利用者が正本改訂を行うときは次の**追加1箇所・置換1箇所**を同じ改訂として適用する。本書作成時には要件正本を変更しない。

1. **追加:** 現在の `docs/requirements-definition.md:55`（§4.1最後の`.ori3`項目）の直後へ、次の1項目を追加する。

```markdown
- **FOLD 1.2 限定**profileの入出力。対象は2D頂点座標、edge topology、B/M/V、`edges_foldAngle`、表現可能な非循環`faceOrders`、線形step frameとする。画面と文書には必ず「FOLD 1.2 限定」と表示し、「FOLD対応」「FOLD完全対応」と表示しない。3D座標、枝分かれした手順、動画、名前付き技法の意味、注記、仕上げの丸み、FOLDの「平ら(F)」「未指定(U)」の区別は対応外一覧として利用者から見える場所に示す。F/Uは`Aux`へ縮退し、元の指定を警告として残して黙って捨てない。
```

2. **削除と置換:** 現在の `docs/requirements-definition.md:66` の1行を削除し、次の1行を同じ位置へ入れる。FOLDだけを非目標から外し、DXF / OBJ / STL / glTFは非目標のまま残す。

```diff
-- FOLD / DXF / OBJ / STL / glTF の入出力
+- DXF / OBJ / STL / glTF の入出力
```

この2変更を行うまでは、承認済みであっても要件正本上のFOLD非目標は解消していない。コード着手条件は、この改訂と、施策8のFOLD-001～006およびM7受入基準を同じ要件改訂へ追加することである。

### 0B.3 要件定義§4.1への一斉折り追加案

§4.2の非目標を変えないので削除・置換は不要である。ただし、§4.1の目標を増やす正本改訂は、要件書:386の手続きに従いコード着手前に行う。本書作成時には要件正本を変更しない。

1. **追加:** 現在の要件書:55（§4.1最後の独自形式項目）の直後へ、次の1項目を追加する。§0B.2のFOLD追加も同じ改訂に入れる場合は、FOLD項目の直後へ続けて置く。挿入後は後続行番号がずれる。

~~~markdown
- 山折り・谷折りの全折り目へ共通の0～100%を希望として与え、手順とは別に一時的な3D形状を表示する一斉折り。全目標角を強制せず、紙のつながりを優先した最寄りの有限形を表示する。画面には常に「これは記録された手順ではない」と示し、手順への追加、作品への保存、Undo/Redoの対象化をしない。一時表示の割合、角度、診断、3D状態は保存しない。不収束、希望角との差、紙の突き抜けは日本語で警告して表示操作を止めず、重なり順を確定したものとして表示しない。常設UIは既存の4区画内に置く。
~~~

2. **置換なし:** §4.2の非目標は変更しない。この追加は、既存の3D表示に対する正の受入契約である。

## 目次

0A. [利用者が決めたこと（2026-08-24）](#0a-利用者が決めたこと2026-08-24)  
0B. [正本へ反映する改訂案](#0b-正本へ反映する改訂案本書作成時には変更しない)  
1. [この計画の前提と非緩和事項](#1-この計画の前提と非緩和事項)
2. [採点の基準と点数見込み](#2-採点の基準と点数見込み)
3. [並べ直した実施順](#3-並べ直した実施順)
4. [委譲・中間報告・完了判定の共通規約](#4-委譲中間報告完了判定の共通規約)
5. [施策1 提案探索を負荷非依存にする](#5-施策1-提案探索を負荷非依存にする)
6. [施策2 手順移動を原子的にする](#6-施策2-手順移動を原子的にする)
7. [施策3 提案benchmark corpusを30作品へ拡張する](#7-施策3-提案benchmark-corpusを30作品へ拡張する)
8. [施策4 巨大境界を分割する](#8-施策4-巨大境界を分割する)
9. [施策5 共通アクセシビリティ基盤を作る](#9-施策5-共通アクセシビリティ基盤を作る)
10. [施策6 bundleを分割する](#10-施策6-bundleを分割する)
11. [施策7 文書を実装から機械検証する](#11-施策7-文書を実装から機械検証する)
12. [施策8 FOLD 1.2 限定profileを実施する](#12-施策8-fold-12-限定profileを実施する)
13. [施策9 実利用者15名で検証する](#13-施策9-実利用者15名で検証する)
14. [施策10 supply-chainと配布を保守する](#14-施策10-supply-chainと配布を保守する)
15. [施策11 全部の折り目を一斉に折る一時表示](#15-施策11-全部の折り目を一斉に折る一時表示)
16. [施策12 Linux対応を見送る](#16-施策12-linux対応を見送る)
17. [全体見積・依存関係・停止条件](#17-全体見積依存関係停止条件)
18. [実施時コマンドの共通前置き](#18-実施時コマンドの共通前置き)

## 1. この計画の前提と非緩和事項

### 1.1 要件§2を全施策より優先する

要件の正本は `docs/requirements-definition.md:23-33` である。全施策で次を維持する。

- 表現の完全性（`:25`）: 層数、偶奇、特定形状を理由に汎用操作を狭めない。
- 数値（`:26`）: `f64` と明示的epsilonを維持し、計算結果の小数を完全一致へ変更しない。
- 警告（`:27`）: 原則として操作を止めず理由を示す。ただしデータ破壊、対応外形式、内部watchdogは成功した部分結果に偽装しない。
- UI（`:28-29`）: 常設4区画を増やさず、直接操作、事前プレビュー、状態表示、できない理由、日本語の折り紙用語を保つ。
- IPC（`:30`）: 同種操作は既存コマンドと操作enumへ集約する。Tauriコマンド数に上限を置くという意味ではない。
- 責務（`:31`）: 行数を理由に機能を削らない。一方、責務を考えず既存1ファイルへ積み増さない。
- 状態（`:32`）: Zustandストアは論理的に1本のままとし、slice分割を複数の独立store化と混同しない。
- 永続化（`:33`）: 通常の3D頂点状態を保存せず、展開図と手順から再導出する。

### 1.2 数値上限の区別

`CLAUDE.md:104-110` と `docs/requirements-definition.md:30-31` が禁じるのは、行数、Tauriコマンド数、ツールボタン数、ダイアログ数などを**機能制限の根拠**にすることである。本書の「単一ファイル1,500行以下」「store slice 1,000行以下」「`App.tsx` 200行以下」は、レビューが提示した今回の分割完了を測る**保守性の目標値**であり、将来必要な機能を拒否する恒久上限ではない。達成のために機能、受入検査、数値精度、表示状態を削ってはならない。

### 1.3 正本と現状の扱い

- `docs/requirements-definition.md` が要件の正本、`docs/implementation-roadmap.md` が既存実装順、`docs/progress.md` が到達点と実測の記録である。
- `docs/implementation-roadmap.md:712-730,748,762-774,877-924` の未チェック欄には、`docs/progress.md:513-620` で実装済みと記録された項目がある。チェックだけを未実装一覧として複写しない。
- `docs/requirements-definition.md:365` の文書構成に対し、本書は利用者が明示的に指定した例外である。今後さらに恒久文書を増やす場合は利用者判断を得る。機械生成の一時証拠は `verification/improvement-roadmap/`、正本へ残す要約は原則 `docs/progress.md` に置く。
- 本書の将来コマンドにgitのcommit、push、checkout、stash、reset、tagは含めない。実装担当Codexがgit操作を担当しない規約を変えない。

### 1.4 標本とfixture

標本、検査、目標に使う名前付き作品は折り鶴、やっこさん、カエル、鳥の基本形とする。その他は `leaves-03-symmetric-01` のように構造・出所が分かる中立IDにする。fixtureは出所、作成方法、ライセンス、checksumをmanifestへ記録し、検査実行中に追跡fixtureを書き換えない。

## 2. 採点の基準と点数見込み

レビュー§5（`docs/comprehensive-review-2026-08-24.md:105-122`）の基準値は93/120、100点換算78/100である。

| 分野 | 現在 |
|---|---:|
| 紙・作品モデル PAP | 9/10 |
| CP・幾何 CPE | 9/10 |
| 3D・剛体・soft SIM | 9/10 |
| 手順・層・技法 SEQ | 8/10 |
| 提案 PRO | 7/10 |
| 書出し・説明書 EXP | 9/10 |
| UI・UX | 8/10 |
| 保存・履歴・デスクトップ SYS | 9/10 |
| 非機能 NFR | 6/10 |
| アーキテクチャ | 6/10 |
| テスト・CI | 7/10 |
| 文書・進捗管理 | 6/10 |

次表は実装着手時ではなく、各節の合格条件を全て満たした後の再評価材料である。明記されたレビュー期待と、本調査で0.5.0の解決分を差し引いた推測を分ける。単純加算や分野10点超えはせず、最終点は同じ12分野で再レビューする。

| 施策 | レビュー記載 | 0.5.0からの限界効果見込み |
|---:|---|---|
| 1 | PRO +1、NFR +1、Test/CI +1 | **推測:** PRO +0.5～1、NFR +1、Test/CI +0.5。3検査の直接原因は解決済みなのでTestの全+1を再計上しない。 |
| 2 | SEQ +1、SYS +0.5 | 同じ。原子性とUndo契約が直接対応する。 |
| 3 | 個別記載なし | **推測:** PRO +1、Test/CI +0.5、文書 +0.5。30作品の公開証拠まで揃った場合。 |
| 4 | Architecture +2、Docs +0.5 | 同じ。ただしファイルを移しただけでなく循環0・契約維持まで合格した場合。 |
| 5 | UI +1 | 同じ。5ダイアログとkeyboard/axe/拡大検査が全て合格した場合。 |
| 6 | NFR +0.5、UI +0.5 | 同じ。gzipだけでなく起動回帰なしを含む。 |
| 7 | Docs +2 | 同じ。6指標、roadmapリンク、CI差分0が揃った場合。 |
| 8 | 現要件点ではなく競争劣位解消 | 2026-08-24に限定profile承認済み。ただし承認だけの現在は+0。**要件改訂と全実装・検査後の推測:** EXP +0.5、SYS +0.5、Docs +0.5。 |
| 9 | UI +0.5、NFR/製品証拠 +0.5 | 準備のみ承認済み。`session 0/15`の間はUI +0、NFR +0。将来15/15完了後だけ各+0.5。 |
| 10 | Test/CI +1、SYS +0.5 | 同じ。承認済み5項目の監視・静的解析・公開が通った場合。自動更新は範囲外で加点条件にしない。 |
| 11 | 個別記載なし | **推測:** SIM +0.5、UI +0.5。一斉折りの一時表示、非保存、警告、重なり順未確定表示の受入条件が全て合格した場合。 |

## 3. 並べ直した実施順

### 3.1 決定済み範囲と残る開始gate

利用者は2026-08-24に6件の範囲を決めた。以後は採否を再質問せず、次の開始gateだけを確認する。

1. FOLD 1.2 限定profileは承認済み。コード前に、§0B.2/§12.2を要件正本へ正式反映する。
2. 実利用者テストは準備3単位だけ承認済み。外部session 5単位は利用者が日程を決めるまで実施しない。
3. supply-chainは承認済み5項目だけを進める。具体的tool/version/pinと依存差分を導入前に提示し、自動更新を加えない。
4. 一斉折りは案Bだけを承認済み。コード前に§0B.3の追加1項目を要件正本へ正式反映する。
5. Linux対応は見送り。§0A.2の再開条件を全て満たすまで、施策12を実装順へ入れず工数も加えない。
6. 今版は施策9の準備までを完了とみなしてリリースし、外部実施5単位は次の版で必ず実施する。施策9の外部実施が未完了でも、今版のリリースを止めない。

### 3.2 実装・検証の順序

| 順序 | 施策 | 先に置く理由 | 完了後に解禁するもの |
|---:|---|---|---|
| 1 | 施策2 原子的 `MoveStep` | 0.5.0で現在も残る具体的な手順消失・Undo 2回の危険で、対象が小さく早く閉じられる。レビュー時の提案3検査は既に赤ではないため順番を入れ替える。 | 手順・storeの大規模分割 |
| 2 | 施策11 一斉折りの一時表示（案B） | 設計文書が安全とする「施策2の画面側完了後」の位置である。画面側は完了済みなので、施策4が未着手の間に単独変更として閉じる。 | 一斉折りの受入を施策4のpure move基準へ含められる |
| 3 | 施策1 提案の残る負荷依存とjob別進捗 | release検査配置は直ったが、製品結果へ壁時計が混ざり、大域progressも残る。corpusの基準を固定する前に意味を安定させる。 | 30作品の決定性hashと性能baseline |
| 4 | 施策7 機械検証付き文書 | `search.rs`/製品budgetは30,000/30,000 msへ同期済みである。roadmap checkboxの不一致を含む後続の大変更で再び陳腐化しない生成境界を先に置く。 | 後続施策の自動status反映 |
| 5 | 施策3 30作品corpus | 提案の意味が安定した後、大規模分割前の振る舞い安全網を作る。 | store/Viewer分割の一般化証拠 |
| 6 | 施策5 共通 `AccessibleModal` | 実利用者検証の前提。5ダイアログの責務境界も作り、後のUI分割を助ける。 | keyboard-only受入、利用者試験 |
| 7 | 施策4 巨大境界分割 | 原子性、提案corpus、Modal契約という安全網を得てから22,589行の主要境界を分ける。 | 安定したlazy-load境界 |
| 8 | 施策6 bundle分割 | 分割後のcomponent/service境界でdynamic importし、手作業のchunk指定だけに依存しない。 | 起動資産の数値gate |
| 9 | 施策10 supply-chain／配布保守 | 独立性が高く並行可能だが、配布物の形が安定した時点でSBOM/hashを確定する。承認済みpolicy作成は順序3以降に並行してよい。 | 監査付きrelease |
| 10 | 施策9 実利用者15名検証 | 準備3単位は先行可能。外部sessionはModal、構造、bundle変更後のUIを対象にし、同じ参加者で再試験する無駄を減らす。 | 15/15完了時だけUI/NFRの実利用証拠 |
| 11 | 施策8 FOLD 1.2 限定profile | 承認済みだが要件正本改訂が必要で、30外部fixture、model/IPC/UIを横断する最大拡張。中核品質を閉じた後に行う。 | 競争上の交換形式 |

施策10のpolicy・監査CI、FOLDの要件正本改訂、施策9の準備は順序3～8と並行できる。これは担当を増やすだけで、同じファイルを同時編集してよいという意味ではない。施策12は見送りのため実装・検証順へ入れない。

## 4. 委譲・中間報告・完了判定の共通規約

### 4.1 1回の委譲の大きさ

- 原則は1責務、現行コード1～5ファイル、検査1～3群、1～4人日とする。
- 5ファイルを超える場合も、同じ型をRust/TypeScript/IPCへ通す1つの垂直sliceなら1委譲にできる。
- 「30作品全部」「巨大store全部」「5ダイアログ全部」「配布保守全部」のような依頼は禁止する。`CLAUDE.md:42-53` の過去の大目標一括依頼の失敗を繰り返さない。
- 各委譲には、許可ファイル、禁止ファイル、契約、検査、合格数値、成果物、git操作禁止の7項目を必ず渡す。

### 4.2 中間報告の共通内容

各段階の完了時、次を報告してから次段階へ進む。

1. 変更ファイルと実関数、追加・更新した検査名。
2. 変更前後の数値。速度、gzip、行数などは同じ経路・同じ単位・同じ機械条件で比較する。
3. 合格、未実行、失敗を分けたコマンド一覧。未実行を合格と書かない。
4. 非緩和事項が保たれた証拠。削除した検査、緩めたepsilon、増やした無視項目が0であること。
5. 残リスクと次段階へ進める／止める判断。

### 4.3 数値境界の決め方

- レビューが指定した値はそのまま受入値にする。
- 新しい性能境界は、同じ製品経路を複数回測り、基準実測が上限の80%以下になる余裕を取る。手元実測ぴったりをCI上限にしない（`CLAUDE.md:341-363`）。
- 計算小数はepsilonで比べる。CPU、runner、debug/releaseの違いを「同じ機械」と偽らない（`CLAUDE.md:302-317`）。
- 点数は全合格後の再レビューでのみ上げる。

### 4.4 担当モデルの判定と集計

各表の「担当モデル」は、2026-08-24の利用者判断に従う。数値と手順が確定した置換・移動・検査・集計はterra、正しい契約・設計・幾何・数値・原因を決める作業はsol ultraとした。迷う作業はterraへ置き、terra作業で未定義の設計判断や結果差が生じた時点で停止し、その判断だけをsol ultraの別委譲へ切り出す。担当モデルは作業の品質条件を変えず、どちらも同じ非緩和事項と検査を満たす。

| 施策 | terra | sol ultra | 合計 |
|---:|---:|---:|---:|
| 1 | 2 | 3 | 5 |
| 2 | 0 | 3 | 3 |
| 3 | 2 | 6 | 8 |
| 4 | 29 | 7 | 36 |
| 5 | 5 | 1 | 6 |
| 6 | 5 | 1 | 6 |
| 7 | 3 | 10 | 13 |
| 8（承認済み限定profile） | 8 | 7 | 15 |
| 9（準備3単位、実施保留5単位を含む） | 6 | 2 | 8 |
| 10（自動更新なし） | 5 | 1 | 6 |
| 11（一斉折り案B） | 3 | 2 | 5 |
| 12（Linux対応見送り） | 0 | 0 | 0 |
| **合計** | **68** | **43** | **111** |

施策8の不採用案は単位数に含めない。施策9の実施保留5単位には将来再判断時の担当をあらかじめ割り当てるが、これは実施承認ではない。施策12は見送り記録だけであり、担当単位・工数を持たない。

## 5. 施策1 提案探索を負荷非依存にする

### 5.1 目的と現在地

解決済みの3テスト修正を再実装せず、残る2問題だけを閉じる。

1. 状態数・分岐・深さ・候補順で決まる探索結果から、壁時計watchdogを分離する。
2. 製品の大域 `PROPOSAL_PROGRESS` をjob別managed stateへ移し、同時要求を識別する。

watchdogに当たった場合は「通常の部分提案」ではなく、キャンセルまたは内部エラーとして別型で返す。遅い機械で候補が少なくなる挙動を許さない。

### 5.2 対象ファイルと関数

| 実パス | 現行／計画する実関数・型 | 役割 |
|---|---|---|
| `crates/ori3-propose/src/search.rs` | `SearchBudget`（`:390-418`）、`SearchStop`（`:540-553`）、`search_to_finish`、`search_to_completion`、`search`（`:958-1082`）、`SearchDeadline`（`:1109-1124`）、`expand`（`:1160`以降） | 決定的budgetと壁時計を分離する。通常の `SearchStop` から `TimeCap` を外すか、製品結果へ入れられないprivate型へ隔離する。 |
| `crates/ori3-propose/src/verify.rs` | `VerifiedPlan`、`verify_search_completion`（`:437-461`） | watchdog結果を `VerifiedPlan::Partial` に変換しない契約にする。 |
| `apps/desktop/src-tauri/src/commands.rs` | `PLAN_BUDGET`、`ProposalProgressCell`、`CandidateTicket`、`proposal_progress`、`plan_folds`、`proposal_generate`、`generate_candidates` | 解決済みticketは維持し、job registryを渡す。`PLAN_BUDGET.max_millis`を通常結果のbudgetとして使わない。 |
| `apps/desktop/src-tauri/src/lib.rs` | `run`（`:77-116`） | `ProposalJobs` を `tauri::Builder::manage` へ登録する。 |
| `apps/desktop/src/ipc/client.ts` | `proposalGenerate`、`ProposalProgress`、`proposalProgress`（`:104-129`） | `{job_id,done,total,phase}` を型付きで通す。 |
| `apps/desktop/src/store/appStore.ts` | `watchProposalProgress`（`:1318-1334`）、`holdFullProposalBar`、`generateProposal`（`:4260-4310`）、`generateProposalFromPaperPositions`（`:4483-4528`） | 現在のjobだけをpollし、古いjobの応答を破棄する。 |
| `.github/workflows/ci.yml` | 関数名は**該当なし**。YAMLの `checks` / `performance` jobであり関数を持たない。 | debug/release・通常/性能検査の配置を固定する。 |

計画上の新しい実名は、backend `ProposalJobId`、`ProposalPhase`、`ProposalJobs::{start,snapshot,finish,cancel,prune}`、IPC `proposal_progress(job_id)` とする。契約は次へ固定する。frontendが開始直前に Web Crypto の `crypto.randomUUID()` で不透明な `job_id` を1個作り、`proposal_generate(job_id, ...)` へ渡す。backendは重複IDを拒否してTauri managed stateの `ProposalJobs` へ登録し、結果型 `ProposalJobResult` も同じIDを返す。進捗は `proposal_progress(job_id)`、取消しは同種操作をまとめた `proposal_control({ type: "Cancel", job_id })` とし、開始用コマンドを増やさない。各jobの `done`、`total`、`phase`、取消し状態は単一 `Mutex<ProposalProgressSnapshot>` の1回のlockで読み書きし、別々のatomic読取りをスナップショットと呼ばない。`ProposalPhase` は `Queued / Generating / Verifying / Finished / Cancelled / Failed` の閉じたenumとする。取消しは探索結果の `SearchStop` に混ぜず、実行制御の結果として返す。

### 5.3 段階、委譲、中間報告

| 段階 | 1回の委譲 | 担当モデル（理由） | 作業 | 中間報告 |
|---|---|---|---|---|
| 1-A | 決定的停止契約（3～4人日） | **sol ultra:** `SearchStop`、`VerifiedPlan`、watchdog、cancelの正しい意味を再設計する必要がある。 | `SearchBudget` の状態・分岐・深さとwatchdog/cancelを型で分け、`SearchStop`、`VerifiedPlan`の到達経路を更新する。 | 同じ入力の候補JSON・通常停止理由hash、watchdog時にpartialを返さない型経路、削除／維持したvariant一覧。 |
| 1-B | job別managed state（3～4人日） | **sol ultra:** 同時job、取消し、panic、回収を含む状態一貫性の設計判断が要る。 | `ProposalJobs` とjob lifecycleを追加し、2要求の進捗を独立させ、完了・cancel後に回収する。 | job A/Bの `done/total/phase` 表、入替り0件、完了後registry件数0、panic時ticket完了数。 |
| 1-C | frontend job伝播（2～3人日） | **sol ultra:** Rust IPC、TypeScript、store間でjob IDと古い応答の契約を決める複数ファイル仕様変更である。 | `apps/desktop/src/ipc/client.ts` と `apps/desktop/src/store/appStore.ts` をjob ID対応にし、閉じた／再生成した画面へ古いpollを戻さない。 | 150 ms pollの呼出し先job、古い応答破棄のDOM/store検査、画面を閉じた後のtimer 0件。 |
| 1-D | 受入matrix runner（2～3人日） | **terra:** matrix、反復数、hash、profileが数値で固定され、runner作成と集計は手順化できる。 | 1/4候補、同時2要求、直列/並列、負荷有無、debug/release、100回回帰を、CI配置とは分けて自動化する。 | 全matrixの回数・hash、runner情報、fixture境界、未実行項目。 |
| 1-E | CI/profile配置（1～2人日） | **terra:** 配置先と責務が既存の`checks`／`performance`へ確定した設定作業である。 | 1-Dのrunnerを `checks` と `performance` へ役割別に置き、表#21のrelease条件を維持する。 | 各jobの実コマンド、所要時間、skip一覧、通常/性能の責務、未実行項目。 |

全体は1回の委譲では終わらない。5委譲に分け、1-Aの型契約を1-B/1-Cより先に固定する。

### 5.4 数値の合格条件

1. 解決済み3検査を、それぞれ100回連続で合格させる。これは再修正ではなく後退0の確認である。
2. 折り鶴、やっこさん、鳥の基本形と12先端入力について、同一seedを10回、候補1件／4件、直列／並列、負荷スレッド0／論理CPU数と同数で実行し、正規化した候補JSON、候補順、通常停止理由のhash不一致を0件にする。
3. debugとreleaseが完了した入力について、同じ契約hashの不一致を0件にする。watchdogに達した実行は候補比較へ混ぜず、専用error/cancelとして数える。
4. 同時2要求を100組実行し、job ID衝突0、進捗の相互混入0、`done > total` 0、完了後に残るjob 0とする。
5. candidate workerの成功、通常Err、panicの3経路で、各100回 `done == total` とする。
6. watchdog注入100回で `VerifiedPlan::Partial` または `CheckedToFinish` が返る件数0、専用error/cancel 100件とする。
7. `cargo test --workspace` 相当の通常CIと、表#21を含むrelease `performance` jobが1回以上合格する。表#21から `--release` を外さない。

### 5.5 変更・緩和してはいけないもの

- `ProposalProgressCell` と `CandidateTicket::drop` の成功・失敗・panic全件加算。
- `TIME_FREE_PLAN_BUDGET.max_millis=3_600_000` を使う比較検査の目的。
- 候補最大4件、先端最大12本という現要件の製品機能。テストを通すため候補や先端を減らさない。
- `CheckedToFinish` のprivate構築と21姿勢、finite、seam、penetration、終点gap検証。
- f64、明示epsilon、候補順の決定性。小数の完全一致へ変えない。
- 画面がbusyなら二度押しを止める既存UXは残す。ただしそれをbackend同時要求安全性の代用にしない。
- 30,000 msを単に延長して通常結果を得る修正、または `TimeCap` の表示名だけ変える修正を完了扱いしない。

### 5.6 過去の失敗と原因

- `CLAUDE.md:160-195`（§10.6）: 通常jobだけを確認し、最適化ありで走る性能jobを見落とした。原因はCIを1本とみなしたこと。全job表と実YAMLを同時に確認する。
- `CLAUDE.md:302-317`（§10.6.1）: CI runnerのCPUが特定できていない状態で、手元値から所要時間を推測してよいかが問題になり、正本も原因未確認と記録している。未確認のCPU像を断定せず、hash契約と性能契約を分け、実測したCPU/OS/profileを記録する。
- `CLAUDE.md:341-369`（§10.7.9）: 手元実測値を上限へ直置きしCIを落とした。原因は余裕のない境界と浮動小数の完全一致。新しい時間境界は基準を80%以内に置く。
- `docs/progress.md:9`: 機械の混み具合で答えが変わり、製品と違う経路を測って待ち時間を誤評価した。原因は意味と性能の測定経路が一致していなかったこと。

### 5.7 道具、コマンド、確認方法

実施時はPowerShellで次を使う。今回は実行していない。

```powershell
$env:CARGO_TARGET_DIR = "C:\Users\oltot\AppData\Local\Temp\ori3-target-codexroadmap"
cargo test -p ori3-propose
cargo test -p desktop --lib proposal_progress_counts_every_candidate
cargo test -p desktop --lib proposal_candidates_are_the_same_computed_together_or_one_by_one
cargo test --release -p desktop --lib the_heaviest_proposal_never_hits_the_time_limit -- --nocapture
npm --prefix apps/desktop test -- src/store/appStore.test.ts
```

100回とprofile横断は、新規 `scripts/check-proposal-determinism.ps1` に固定し、手入力loopを成果物にしない。確認値は `verification/improvement-roadmap/01-proposal/determinism.json` に、入力hash、profile、worker数、候補hash、stop、時間、CPU情報を保存する。`rg -n "PROPOSAL_PROGRESS|SearchDeadline|TimeCap|job_id|jobId|ProposalJobs"` で到達経路を数え、製品用大域 `static PROPOSAL_PROGRESS` が0件であることを確認する。

### 5.8 成果物、見積、依存、リスク、点数

- **保存先:** 上表の7実装ファイル、`scripts/check-proposal-determinism.ps1`、`verification/improvement-roadmap/01-proposal/determinism.json`、要約を `docs/progress.md`。CIを変える委譲だけ `.github/workflows/ci.yml` を許可する。
- **見積:** 11～16人日。現行7ファイル・合計10,403行（`search.rs`、`verify.rs`、`commands.rs`、Tauri `lib.rs`、`client.ts`、`appStore.ts`、CI YAML）を到達経路として調べ、型/探索、job lifecycle、frontend stale response、matrix runner、CI profileの5検査群を閉じる。差分行数ではなく、10,403行のうち進捗・探索に到達する関数を全て再確認する工数を含む。
- **依存:** 施策2とはコード依存なし。施策3は本施策の停止理由とhash契約確定後。施策7は本施策のbudget正本を読む。
- **主リスク:** watchdogを外して無限探索にすること。決定的budgetだけで終了が証明できない入力が1件でも出たらreleaseせず、探索の状態上限を製品契約として見直す。watchdogをpartialへ戻して回避しない。
- **副リスク:** job registryの回収漏れ。100同時組後にregistryが1件以上残れば1-Cへ進まない。
- **点数:** レビューはPRO +1、NFR +1、Test/CI +1。0.5.0からは直接テスト修正分を除き、**推測**でPRO +0.5～1、NFR +1、Test/CI +0.5。

## 6. 施策2 手順移動を原子的にする

### 6.1 目的と現在地

現在の `apps/desktop/src/store/appStore.ts:4683-4705` の `moveStep` は、`RemoveStep` と `InsertStep` を別IPCで待つ。前半成功後に後半が失敗すれば手順が消え、成功してもUndoが2回必要になる。`docs/progress.md:501` に既知制約として記録済みである。

`SeqOp::MoveStep { id, to_index }` を既存 `sequence_apply` のenumへ追加し、対象ID確認、行先確認、remove/insert、step crease保持、再生用view導出、履歴pushを1ロック・1commitで行う。`to_index` は移動後の0始まりindexと定義する。

### 6.2 対象ファイルと関数

| 実パス | 実関数・型 | 作業 |
|---|---|---|
| `crates/ori3-model/src/lib.rs` | `SeqOp`（`:473-489`） | `MoveStep { id: StepId, to_index: usize }` を追加する。 |
| `crates/ori3-model/tests/serde_roundtrip.rs` | `SeqOp`のserde検査群 | JSON契約の往復と未知・欠落fieldの失敗を検査する。 |
| `apps/desktop/src-tauri/src/store.rs` | `DocumentStore::apply_seq`、`DocumentStore::apply_seq_with_spatial`（`:400-436`）、`DocumentStore::commit`（`:816-842`） | 変更候補をclone上で検証し、1回だけcommitする。 |
| `apps/desktop/src-tauri/src/commands.rs` | `parse_sequence_operation`、`sequence_apply`（`:431-455`） | 新variantを既存コマンドで受ける。新Tauriコマンドは作らない。 |
| `apps/desktop/src/lib/types.ts` | TypeScript `SeqOp`（`:281-286`） | Rustと同じtag/field名を追加する。 |
| `apps/desktop/src/ipc/client.ts` | `sequenceApply`（`:54-56`） | wrapperは1回invokeのまま維持する。 |
| `apps/desktop/src/store/appStore.ts` | `applySequenceOp`、`moveStep`（`:4683-4705`） | 最新IDを取り直した後、`MoveStep`を1回送る。 |
| `apps/desktop/src/store/appSettings.test.ts` | `describe("手順の並べ替え")`（`:835-930`） | 2 call期待を1 call、Undo 1回、失敗時不変へ更新する。 |

### 6.3 段階、委譲、中間報告

| 段階 | 1回の委譲 | 担当モデル（理由） | 作業 | 中間報告 |
|---|---|---|---|---|
| 2-A | 契約と失敗matrix（1～2人日） | **sol ultra:** index、同一位置、異常入力、serde、履歴の正しい契約を決める必要がある。 | `to_index`、同一位置、存在しないID、範囲外、2手、重複ID防御、serdeを表にし、先に失敗検査を置く。 | 例ごとの変更前後sequence、期待history増分、dirty、error文、未決定事項0。 |
| 2-B | Rust原子操作（2～3人日） | **sol ultra:** Rustの複数層にまたがる原子transactionとreplay・幾何結果の正しさを保証する必要がある。 | model/store/commandを実装し、候補view導出成功時だけcommitする。 | store snapshotの比較、1操作history 1、Err時0、step_creases同一、replay終点差。 |
| 2-C | frontend単一IPC（2～3人日） | **sol ultra:** TypeScript型、IPC、store、mock、Undo/Redoをまたぐ仕様変更と順序判断が要る。 | TypeScript型、`moveStep`、mock、DOM/store検査を更新する。 | `ipc.sequenceApply` call数、pending UpdateStepとの順序、reject時表示、Undo/Redo往復。 |

全体一括はRustとfrontendの失敗位置を混同しやすいため不可。3委譲に分ける。

### 6.4 数値の合格条件

1. 有効な1回の移動につき `sequence_apply` invoke 1回、backend commit 1回、undo stack増分1、Undo 1回、Redo 1回とする。
2. 移動後の全 `step_creases` をstep IDで正規化し、移動前とbit同一にする。欠落・重複0件。
3. 存在しないID、`to_index > len-1`、JSON欠落、導出失敗を各100回注入し、`Document`、`step_creases`、undo/redo stack長、dirty、facesが変更前と一致する。手順消失0件。
4. frontend IPC rejectを100回注入し、画面側sequenceの楽観的削除0件、2回目invoke 0件とする。
5. 2～100手のsequenceについて、先頭・中央・末尾を前後へ移す全合法caseを検査し、IDの集合・件数が100%一致する。
6. 各合法caseでは、履歴を積まず期待する最終手順列を直接組み立てた独立cloneをoracleとする。原子的MoveStep後のreplay終点とoracleの対応頂点距離を `<=1e-9`、seamを `<=1e-6`、penetrationを0、全頂点をfiniteとする。手順順序で終点が変わり得るため、移動前の終点との一致は要求しない。
7. 同じ位置へのmoveは成功・無変更とし、history増分0、dirty変化0とする。

### 6.5 変更・緩和してはいけないもの

- `SYS-002` の「1ジェスチャー=1履歴」と最大100履歴。最大件数を減らさない。
- `InsertStep` / `RemoveStep` の既存用途とserde互換。新variant追加のため既存tagを改名しない。
- `step_creases` はstep IDに追従し、並べ替えで消さない（`apps/desktop/src-tauri/src/store.rs:428-435,3568-3594`）。
- frontendでpending `UpdateStep` の完了を待ち、IDから最新stepを取り直す契約（`apps/desktop/src/store/appStore.ts:4690-4698`）。
- Errまたはpanicならstore不変という `DocumentStore::commit` の先行導出契約。
- 移動を可能にするため途中への新規折り操作を無制限に許すなど、SEQ-005の別仕様を変更しない。

### 6.6 過去の失敗と原因

- `docs/progress.md:501`: 削除+挿入を「並べ替え」として組み合わせ、Undo 2回を既知制約にした。原因はUIジェスチャーとbackendトランザクションの境界が一致しなかったこと。
- `CLAUDE.md:269-283`（§10.7.5）: 変更したcrateだけを検査し、model、simulation、UIの層をまたぐ問題を見落とした。Rust enum、IPC JSON、frontend mock、replayを同じ縦sliceで確認する。
- `CLAUDE.md:319-339`（§10.7.8）: 同じ設定の複数箇所の一部だけを直し再発した。`rg`で `InsertStep` / `RemoveStep` / `moveStep` の全利用箇所を数える。

### 6.7 道具、コマンド、確認方法

```powershell
$env:CARGO_TARGET_DIR = "C:\Users\oltot\AppData\Local\Temp\ori3-target-codexroadmap"
cargo test -p ori3-model --test serde_roundtrip
cargo test -p desktop --lib store::tests::seq_ops_apply_and_undo
cargo test -p desktop --lib reordering_steps_keeps_the_crease_history
npm --prefix apps/desktop test -- src/store/appSettings.test.ts src/store/appStore.test.ts
rg -n "MoveStep|RemoveStep|InsertStep|moveStep" crates/ori3-model apps/desktop/src-tauri apps/desktop/src
```

内部snapshot比較はテスト専用helper `assert_store_unchanged` とし、製品APIへprivate状態を公開しない。結果は `verification/improvement-roadmap/02-move-step/atomicity.json` にcase数、failure注入数、history差、replay最大差を保存する。

### 6.8 成果物、見積、依存、リスク、点数

- **保存先:** 上表の8ファイル、`verification/improvement-roadmap/02-move-step/atomicity.json`、完了要約を `docs/progress.md`。
- **見積:** 5～8人日。Rust model/store/command/testとTypeScript型/IPC/store/testの現行8ファイル・合計15,626行を到達範囲として確認し、serde、store原子性、failure注入、replay、frontend IPC/Undoの5検査群を3委譲で閉じることが根拠である。
- **依存:** 先行施策なし。施策4のstore分割より前に完了する。
- **リスク:** `to_index` をremove前indexと解釈してoff-by-oneになること。2～100手の全合法caseで1件でも順序差があれば仕様を変えず2-Aへ戻る。
- **リスク:** `attach_replay` がcommit後に失敗してIPCだけErrになること。注入で再現した場合、MoveStepだけの局所patchにせず、view導出をcommit前へ移す共通トランザクション設計を別小作業として追加する。
- **点数:** SEQ +1、SYS +0.5（レビュー記載。推測ではない）。

## 7. 施策3 提案benchmark corpusを30作品へ拡張する

### 7.1 目的と範囲

現在の端から端の根拠は3標本（`docs/progress.md:5-9`）であり、1,005回の記録（`:75-84`）は同一骨格のseed測定で1,005作品ではない。異なる30作品を、葉数、対称性、位置制約、simple/compound、完成期待/partial期待で層別し、提案の一般化、安全性、決定性、速度を同じ製品経路で記録する。

30作品は次の5組に固定する。

| 組 | 葉数 | 件数 | 必須構成 |
|---|---:|---:|---|
| A | 3～4 | 6 | 対称3/非対称3、位置制約あり3/なし3 |
| B | 5～6 | 6 | 同上、simple/compoundを各3 |
| C | 7～8 | 6 | 同上 |
| D | 9～10 | 6 | 同上 |
| E | 11～12 | 6 | 同上 |

折り鶴、やっこさん、カエル、鳥の基本形をanchor fixtureとし、残り26件は構造を示す中立IDを使う。少なくとも12件を事前に「完成期待」、残りを「安全なpartialでも可」に分類し、結果を見て期待classを書き換えない。

### 7.2 対象ファイルと関数

| 実パス | 実関数・型 | 作業 |
|---|---|---|
| `crates/ori3-propose/tests/acceptance.rs` | `crane_sample`、`yakko_sample`、`bird_base_sample`、`samples`、`run_to_completion`、`completion_search_uses_safe_subsets_and_is_deterministic_ten_out_of_ten` | 既存3標本をanchorとして残す。大きな30件runnerは別ファイルへ出す。 |
| `crates/ori3-propose/tests/end_to_end.rs` | `run_named_candidate`、`run_named_candidates`、`assert_named_completion`、`named_sample_completes_end_to_end_and_is_deterministic_ten_out_of_ten` | 製品相当のend-to-end契約を共通runnerへ切り出す。 |
| `crates/ori3-propose/tests/support/mod.rs` | `read_baseline`、`assert_cp_matches_baseline` | manifest、hash、fixture read-only helperを追加する。 |
| `crates/ori3-propose/tests/corpus.rs` | 計画する `load_corpus_manifest`、`run_corpus_case`、`assert_corpus_safety`、`corpus_release_metrics` | 30作品専用runner。 |
| `crates/ori3-propose/tests/fixtures/corpus/manifest.json` | 関数名は**該当なし**。JSON manifestのため。 | source、license、checksum、層別、期待class、gap、停止理由、hash欄。 |
| `crates/ori3-propose/tests/fixtures/corpus/*.json` | 関数名は**該当なし**。入力fixtureのため。 | 30件を追跡し、テストから書き換えない。 |
| `scripts/generate-proposal-corpus-report.ps1` | 計画する `Invoke-CorpusRun`、`Get-Percentile`、`Write-CorpusReport` | release複数回の集計専用。fixture生成とは分ける。 |
| `.github/workflows/ci.yml` | 関数名は**該当なし**。YAML jobのため。 | 通常安全性とrelease性能集計を分ける。 |

### 7.3 段階、委譲、中間報告

| 段階 | 委譲単位 | 担当モデル（理由） | 作業 | 中間報告 |
|---|---|---|---|---|
| 3-A | schema/runner 1件（3～4人日） | **sol ultra:** corpus schema、期待class、安全性指標、製品相当経路の正しい定義を決める必要がある。 | manifest schema、正規化hash、6指標、read-only fixture規約、1件のpilotを作る。 | schema全field、pilotの入力hash/結果hash、fixture変更0、製品経路のcall graph。 |
| 3-B | 6作品×5委譲（各2～3人日） | **sol ultra（B-A～B-Eの5件）:** 各葉数層の入力妥当性、完成／partial期待、安全性を結果を見る前に判断する必要がある。 | A～Eを別担当・別回に追加し、各組で層別条件を満たす。 | 各6件の出所/license/checksum、葉数分布、完成/partial期待、全安全指標。 |
| 3-C | release集計 1件（3～4人日） | **terra:** 既定runnerを10回・5回実行し、hash、中央値、P95を集計する数値作業である。 | 同一runnerで10回決定性、5回性能を集計し、中央値/P95と機械情報を出す。 | 30×10のhash不一致数、30×5の時間、median/P95、CPU/OS/profile、外れ値。 |
| 3-D | CI/公開要約 1件（2～3人日） | **terra:** 通常／性能jobの役割と公開項目が確定した設定・文書生成作業である。 | 通常jobは安全性、performance jobはrelease時間に分け、`docs/progress.md`へ生成要約を置く。 | job所要時間、skip一覧、生成差分、30件のpass/partial表。 |

合計8委譲。30件一括委譲は禁止する。

### 7.4 数値の合格条件

1. manifestに異なる30作品があり、葉数3～12の5帯に各6件、対称/非対称各15件、位置制約あり/なし各15件、simple/compound各15件を含む。
2. 30/30で全座標・角・gapがfinite、最大seam `<=1e-6`、penetration 0、自己交差0とする。
3. 事前指定した完成期待12件以上は、各10/10で `CheckedToFinish`。途中で期待classを下げた件数0。
4. partial許容ケースは全件 `final_weighted_gap < initial_weighted_gap`、集合中央値の改善率20%以上、安全性違反0とする。満たせないcaseは原因とともに失敗として残し、fixtureを削除しない。
5. 各caseを同じseedで10回実行し、候補JSON、候補順、停止理由、決定性hashの不一致0件。
6. releaseで各caseを5回測り、全150値から全体median/P95、葉数帯別median/P95、基準機CPU/論理CPU/メモリ/OS/Rust/profileを保存する。
7. 回帰上限は基準P95を上限の80%以下に置く。すなわち初回安定baselineの1.25倍を超えない値を次回gateとし、測定ぴったりの値を上限にしない。
8. test実行前後の30 fixture checksum不一致0、追跡fixtureへのwrite call 0とする。

### 7.5 変更・緩和してはいけないもの

- 既存3標本の3/3、10/10、21姿勢、seam、penetration契約。30件で置換しない。
- 完成期待を結果に合わせてpartial期待へ下げない。
- 失敗fixtureを削除、ignore、葉数減少、位置制約解除して分母から外さない。
- 裂けだけを見て完成とせず、外形gap、自己交差、層警告、penetration、finiteを全て取る。
- fixture生成と検査を同じコマンドにしない。テストはread-only。
- 時間上限を機械間で同値とみなさず、結果hashと性能値を別契約にする。

### 7.6 過去の失敗と原因

- `CLAUDE.md:147-158`（§10.5）: PDF生成・ファイルサイズだけで内容を確認済みとした。原因は生成成功を意味の正しさと混同したこと。corpusも件数でなく6安全指標を見る。
- `CLAUDE.md:250-255`: 出所不明の未完成標本を完成作品と誤認した。原因はmanifestと由来の欠落。source/license/checksumを必須にする。
- `CLAUDE.md:285-300`（§10.7.6）: テストがfixtureを再生成し、追跡ファイルを書き換えた。原因は生成と検査の兼用。別scriptへ分ける。
- `docs/progress.md:9`: 製品と異なる経路の時間を測った。`proposal_generate`相当の候補生成と検証をrunnerの正本にする。

### 7.7 道具、コマンド、確認方法

```powershell
$env:CARGO_TARGET_DIR = "C:\Users\oltot\AppData\Local\Temp\ori3-target-codexroadmap"
cargo test -p ori3-propose --test acceptance
cargo test -p ori3-propose --test end_to_end
cargo test -p ori3-propose --test corpus
cargo test --release -p ori3-propose --test corpus -- --nocapture
powershell -ExecutionPolicy Bypass -File scripts/generate-proposal-corpus-report.ps1
```

fixtureの前後checksumをmanifestと照合し、`rg -n "write|create_dir|File::create" crates/ori3-propose/tests/corpus.rs crates/ori3-propose/tests/support` でrunner内の書込み経路0を確認する。性能出力は `verification/improvement-roadmap/03-proposal-corpus/release-metrics.json`、人向け要約は `docs/progress.md` の生成領域へ保存する。

### 7.8 成果物、見積、依存、リスク、点数

- **保存先:** `crates/ori3-propose/tests/acceptance.rs`、`crates/ori3-propose/tests/end_to_end.rs`、`crates/ori3-propose/tests/support/mod.rs`、`crates/ori3-propose/tests/corpus.rs`、`crates/ori3-propose/tests/fixtures/corpus/manifest.json`、同ディレクトリの30 fixture、`scripts/generate-proposal-corpus-report.ps1`、`verification/improvement-roadmap/03-proposal-corpus/release-metrics.json`、`docs/progress.md`、`.github/workflows/ci.yml`。
- **見積:** 18～26人日。既存runner/CIの4ファイル・合計1,912行、新規runner/manifest/reportの3ファイル、30 fixtureを対象にし、manifest/read-only、30件安全性、10回決定性、5回release性能、CI生成差分の5検査群を8委譲で閉じる。段階和は3-A 3～4、3-B 10～15、3-C 3～4、3-D 2～3人日である。
- **依存:** 施策1の停止理由、job、候補hash確定後。施策4より前に安全網として完了する。
- **リスク:** 30件が同じ骨格のseed違いになること。正規化した入力構造hashが重複したら別作品として数えない。
- **リスク:** release CI時間が長すぎること。通常 `checks` を同条件で3回測り、corpus追加による中央値増分が10分超、またはjob全体の中央値が20分超なら、30×10を通常jobへ一括投入せず、決定性を複数jobまたはscheduled `performance` へ分ける。検査、fixture、反復数は削らない。この値はCI資源の配置判断であり製品機能の上限ではない。
- **点数:** レビューに個別明記なし。**推測**でPRO +1、Test/CI +0.5、Docs +0.5。

## 8. 施策4 巨大境界を分割する

### 8.1 目的と現行実測

現行の主要境界をUTF-8の物理行で数えると次のとおりである。

| 実パス | 行数 | 主責務 |
|---|---:|---|
| `apps/desktop/src/store/appStore.ts` | 4,708 | 状態型、Zustand store、document/CP、pose/replay、proposal、dialogs/settings、履歴、世代管理 |
| `apps/desktop/src/App.css` | 5,452 | token/theme、base、layout、Viewer、Context、dialog、responsive |
| `apps/desktop/src/components/Viewer3D/Viewer3D.tsx` | 2,248 | scene lifecycle、highlight、camera、pick、pointer interaction、render |
| `apps/desktop/src/components/Viewer3D/sceneBuilder.ts` | 2,102 | topology、content、soft、camera、framing、highlight、scene作成 |
| `apps/desktop/src/components/ContextPanel.tsx` | 1,871 | angle、step、align、fold、technique、paper、curve、selection、message |
| `apps/desktop/src/App.tsx` | 290 | toolbar、4区画、status、dialog mount |
| `apps/desktop/src-tauri/src/surface_order_acceptance.rs` | 5,918 | fixture、camera/raster、endpoint、visual、random、gap、diagnostic、契約検査 |

合計22,589行である。`apps/desktop/src/store/ipcQueue.ts` の `createSerialQueue` は既に分離済みで、専用テスト `apps/desktop/src/store/ipcQueue.test.ts` もある。レビューの「IPC queueをservice化」を未着手として二重実装しない。また `ViewerOverlayStack` 等も既に別componentなので、戻してから再分割しない。

### 8.2 対象ファイルと関数

| 実パス | 現行の実関数・領域 | 計画する境界 |
|---|---|---|
| `apps/desktop/src/store/appStore.ts` | `AppState`、`useAppStore`（`:1279`）、`applyView`、`runViewCommand`、`applyDocChange`、`foldThroughRevision`、`foldThroughBusyToken`、`proposalGeneration`、pose/replay/history helper、proposal helper | `store/slices/documentCpSlice.ts`、`poseReplaySlice.ts`、`proposalSlice.ts`、`dialogSettingsSlice.ts`、`store/services/documentCommandService.ts`、計画する `generationGate.ts::{createGenerationGate,issue,isCurrent}`。全sliceを1つの `useAppStore`へ合成する。 |
| `apps/desktop/src/store/ipcQueue.ts` | `createSerialQueue` | **変更不要を基本**とし、公開契約を利用する。 |
| `apps/desktop/src/components/Viewer3D/sceneBuilder.ts` | `buildTopology`、`createContent`、`updateFrame`、`createSoftContent`、`updateSoftContent`、camera/framing群、highlight群、`createScene` | topology/content、camera/framing、highlight/layers、scene facadeへ分ける。 |
| `apps/desktop/src/components/Viewer3D/Viewer3D.tsx` | `Viewer3D`、`fitCamera`、`drawHighlight`、`handlePointerDown`、`handlePointerMove`、pointer up経路 | scene lifecycle hook、camera hook、pick/projection、pointer state/handlers、highlight derivationへ分ける。 |
| `apps/desktop/src/components/ContextPanel.tsx` | `HingeAngle`、`StepContent`、`AlignDraftContent`、`FoldDraftContent`、`TechniqueDraftContent`、`PullContent`、`SelectionContent`、`RelaxationMessages`、`ContextPanel` | angle/step、align/fold、layer motion/named technique、paper/curve/selection/messageのcomponentへ分ける。 |
| `apps/desktop/src/App.css` | 関数名は**該当なし**。CSS selector、theme、cascadeの集合である。 | `styles/index.css` で `@layer tokens, themes, base, layout, viewer, context, dialogs, responsive` の順を1回だけ宣言し、`tokens.css`、`themes.css`、`base-layout.css`、`viewer.css`、`context.css`、`dialogs.css`、`responsive.css` を所有layerへ分ける。 |
| `apps/desktop/src/App.tsx` | `ExportButton`、`relaxationStatus`、`App` | toolbar/statusを別componentへ出し、`App`は4区画compositionと最上位状態だけにする。 |
| `apps/desktop/src-tauri/src/surface_order_acceptance.rs` | `surface_order_179_999_to_180_all_110_creases`、`surface_order_exact_endpoint_is_rank_stable_for_previous_19`、user-frame/visual/determinism、`coincident_overlaps`、`audit_top_faces`、gap/derivation検査、ignored diagnostics | support fixture/raster、endpoint heavy contracts、visual/determinism、gap/derivation、diagnosticsのtest moduleへ分ける。テスト名は維持する。 |

### 8.3 段階と36個の委譲単位

段階2の最低見積16単位から、全製品TS/TSXとimport graphを追加走査した結果、`ContextPanel.tsx`、Viewer interaction、surface-order supportの競合回避が必要と分かった。1,871行のContextPanelを4責務一括、fixture/raster/overlapを一括、user-frame/visual/determinismを一括では委譲しない。最終計画では36単位まで細分化する。

| 段階 | 委譲IDと内容 | 担当モデル（単位ごとの理由） | 中間報告 |
|---|---|---|---|
| 4-A 契約固定 | **1件、計2～3人日:** A1 import graph、現行selector/public export、主要テスト名、7ファイル行数をJSON化する。 | **A1 terra:** import、selector、export、test名、行数を既定形式で棚卸しする機械的作業である。 | 循環数、公開selector数、現行test数、行数、移動中に変えてはいけないAPI一覧。 |
| 4-B store | **5件、計10～15人日:** B1 document/CP + command service + generation gate、B2 pose/replay/history、B3 proposal、B4 dialogs/settings、B5 store統合と旧内部export整理。既存 `ipcQueue.ts` は再実装しない。 | **B1 sol ultra:** command serviceとgeneration gateの責務・非同期一貫性を設計する。<br>**B2 terra:** pose/replay/historyを結果差0で移す。<br>**B3 terra:** proposal責務の移動境界が確定している。<br>**B4 terra:** dialogs/settingsの移動先が確定している。<br>**B5 terra:** A1の公開契約どおり再合成・内部export整理を行う。 | 各回、移した関数名、旧/新行数、selector型検査、focused test、full frontend test、Zustand store数。 |
| 4-C Viewer/Context | **13件、計20～30人日:** C1 scene topology/content、C2 camera/framing、C3 highlight/layers、C4 scene facade、C5 Viewer lifecycle、C6 Viewer camera、C7 pick/projection、C8 pointer interaction（state + handlers）、C9 Viewer highlight、C10 Context angle/step、C11 Context align/fold、C12 Context technique、C13 Context paper/curve/selection/message。 | **C1 terra:** topology/contentの対象関数と移動先が確定している。<br>**C2 terra:** camera/framingを結果差0で移す。<br>**C3 terra:** highlight/layersを既定境界へ移す。<br>**C4 sol ultra:** scene facadeのAPIと資源所有を設計する。<br>**C5 sol ultra:** lifecycleのmount/dispose責務を決める。<br>**C6 sol ultra:** camera座標・framing・状態同期の正しさを保つ。<br>**C7 sol ultra:** pick/projectionの座標変換と幾何を判断する。<br>**C8 sol ultra:** pointer state machine、capture、取消し、順序を設計する。<br>**C9 terra:** highlight derivationを結果差0で切り出す。<br>**C10 terra:** angle/stepの対象componentが確定している。<br>**C11 terra:** align/foldの対象componentが確定している。<br>**C12 terra:** technique責務の移動先が確定している。<br>**C13 terra:** paper/curve/selection/messageの移動範囲が確定している。 | 各回、WebGL資源のcreate/dispose数、pointer契約、camera差、既存overlay再利用、関連DOM/scene test、最大ファイル行数。 |
| 4-D CSS/App | **8件、計12～18人日:** D1 `@layer`/import契約、D2 token/theme、D3 base/layout、D4 Viewer、D5 Context、D6 dialogs/help/proposal、D7 responsive、D8 App toolbar/status切出し。 | **D1 sol ultra:** cascade順とselector所有契約の正しさを決める。<br>**D2 terra:** token/themeを確定layerへ移す。<br>**D3 terra:** base/layoutを確定layerへ移す。<br>**D4 terra:** Viewer selectorの所有先が確定している。<br>**D5 terra:** Context selectorの所有先が確定している。<br>**D6 terra:** dialogs/help/proposal selectorの所有先が確定している。<br>**D7 terra:** responsive規則を移し既定寸法で検査する。<br>**D8 terra:** toolbar/statusの移動対象が確定している。 | selector数の増減、CSS直読4検査の参照先、5 theme、1000×700、`App.tsx`行数、未所有selector数。 |
| 4-E surface order | **9件、計12～16人日:** E1 fixture、E2 raster、E3 overlap、E4 endpoint heavy、E5 user-frame、E6 visual、E7 determinism、E8 gap/derivation、E9 ignored diagnostics。 | **E1～E9 terra:** 9件とも既存検査を名前・期待値・実行分類・epsilon不変で指定moduleへ移すpure moveであり、各単位の対象群が確定している。 | 移動前後のテスト名集合、通常/release/ignored分類、個別heavy command、各module行数、数値期待値差0。 |

全体を1回で委譲することは明確に不可。同じ巨大ファイルを触るB、C、D、E内の委譲は原則逐次にし、別ファイル群だけを並行する。

### 8.4 数値の合格条件

1. 対象となる製品 `.ts` / `.tsx` の単一ファイルを全て1,500行以下、store sliceを各1,000行以下、`apps/desktop/src/App.tsx` を200行以下にする。
2. 分割後の対象CSS所有ファイルと `surface_order_acceptance` 分割moduleも各1,500行以下を今回の保守性目標とする。機能削減で達成した件数0。
3. Zustandの製品storeは `useAppStore` 1本。独立store追加0、同じ状態の二重保持0。
4. TypeScript/Rustのimport/module循環0。計画する `check-import-cycles` が全対象nodeを100%走査する。
5. 分割前の公開selector/exportをmanifest化し、意図したdeprecated削除を除いて型検査対応率100%、未説明削除0。
6. 分割前後で既存テスト名の集合差0、既存合格検査の削除・ignore追加0。テスト総数は減少0。
7. `surface_order_179_999_to_180_all_110_creases` と `surface_order_exact_endpoint_is_rank_stable_for_previous_19` は、名称とrelease個別実行を維持して各1回以上合格する。
8. WebGL sceneを100回mount/unmountし、renderer、geometry、material、listenerの未解放増分0。
9. 5 theme × 100画面状態で、既存CSS契約の欠落selector 0、常設4区画の増減0、1000×700で操作要素欠落0。
10. 各pure-move委譲で機能結果hash差0。差が出た場合は「分割だから同じはず」と進めず、その委譲だけを戻して原因を特定する。
11. `styles/index.css` のlayer順宣言は1件、製品selectorは100%が `tokens/themes/base/layout/viewer/context/dialogs/responsive` のいずれか1所有者に入り、未所有0、複数所有0とする。
12. ブラウザまたはアプリ起動が明示許可された別工程で、単一委譲以外の変更が混ざっていない状態をread-only `git status --short`で確認し、5 theme × 10代表状態 × 2寸法（1000×700、1280×860）=100枚の変更後画像を目視する。100/100にreviewer、判定、build IDがあり、未確認0、操作要素欠落0、意図しない差0とする。本書作成中はこの表示QAを実行しない。

### 8.5 変更・緩和してはいけないもの

- 行数達成のための機能、Tauriコマンド、ツール、ダイアログ、検査、fixture削除。
- Zustand store 1本、typed IPC、直列queue、`runLatest`、generation token、1ジェスチャー1履歴。
- `apps/desktop/src/store/ipcQueue.ts` の発行順とlatest集約。既に分離済みである。
- `ViewerOverlayStack` 等の既存component境界とWebGL dispose。
- 4区画、5 theme、1000×700、pointer/keyboard各20件、全画面状態検査。
- surface-orderのactive/release/ignored分類、テスト名、角度、視点数、epsilon。分割に合わせて数値を緩めない。
- CSS cascade順。単に複数ファイルへ切って見た目を変えない。

### 8.6 過去の失敗と原因

- `docs/requirements-definition.md:12-20`: ORIGAMI2では `editor.rs` 21,736行、`App.tsx` 5,963行と66 `useState`、IPC 191個へ集中した。原因は責務境界より機能追加を優先したこと。
- `CLAUDE.md:42-53`（§3）: 巨大な全手順を1回で依頼し、何度も失敗した。36個の責務単位へ分ける。
- `CLAUDE.md:319-339`（§10.7.8）: 同じ設定の5箇所中1箇所だけを直した。分割前後の全export/import/selectorを数える。
- `CLAUDE.md:371-397`（§10.7.10）: 見た目を確認せず次へ進み、壊れた表示を積み重ねた。各CSS/Viewer委譲で既存画面契約を閉じてから次へ進む。

### 8.7 道具、コマンド、確認方法

```powershell
$env:CARGO_TARGET_DIR = "C:\Users\oltot\AppData\Local\Temp\ori3-target-codexroadmap"
npm --prefix apps/desktop run lint
npm --prefix apps/desktop test
npm --prefix apps/desktop run build
npm --prefix apps/desktop run check:import-cycles
cargo test -p desktop --lib surface_order
cargo test --release -p desktop --lib surface_order_179_999_to_180_all_110_creases -- --nocapture
cargo test --release -p desktop --lib surface_order_exact_endpoint_is_rank_stable_for_previous_19 -- --nocapture
```

行数は `rg -n "^" <実パス>` の最終行番号で数え、改行解釈の異なる手段を混ぜない。import循環は既存依存を増やさず、`apps/desktop/scripts/check-import-cycles.ts` を既存 `tsx` で実行する案を第一候補にする。外部package導入が必要なら別承認を得る。結果は `verification/improvement-roadmap/04-boundaries/before.json` と `after.json` に保存する。

画像目視は `CLAUDE.md` §10.7.10の再発防止gateである。明示許可を得た実装工程だけで同一buildから100枚を取得し、`verification/improvement-roadmap/04-boundaries/visual-qa/after/` に置く。基準画像が同一機・同一寸法で取得できる場合は `before/` にも置き、`review.json` に100件のpath、theme、state、寸法、reviewer、判定を保存する。画像取得不能な環境では「未実行」とし、DOM検査だけで施策4を完了扱いにしない。

### 8.8 成果物、見積、依存、リスク、点数

- **保存先:** `apps/desktop/src/store/slices/`、`apps/desktop/src/store/services/`、`apps/desktop/src/components/Viewer3D/`、`apps/desktop/src/components/ContextPanel/`、`apps/desktop/src/styles/`、`apps/desktop/src/App.tsx`、`apps/desktop/src-tauri/src/surface_order_acceptance/`、`apps/desktop/scripts/check-import-cycles.ts`、`verification/improvement-roadmap/04-boundaries/before.json`、`verification/improvement-roadmap/04-boundaries/after.json`、`verification/improvement-roadmap/04-boundaries/visual-qa/before/`、`verification/improvement-roadmap/04-boundaries/visual-qa/after/`、`verification/improvement-roadmap/04-boundaries/visual-qa/review.json`、`docs/progress.md`。
- **見積:** 56～82人日。主要7ファイル・合計22,589行、関連frontend test約1万行、36委譲を対象にし、store selector、import cycle、Viewer scene/DOM、Context DOM、CSS直読4契約、surface通常、release heavy 2件、100枚目視の8検査群を閉じる。段階和は4-A 2～3、4-B 10～15、4-C 20～30、4-D 12～18、4-E 12～16人日である。
- **依存:** 施策2、施策3、施策5の契約を先に閉じる。施策6は本施策後。
- **リスク:** sliceを複数storeにして状態一貫性を失うこと。製品store数が2になった時点で停止する。
- **リスク:** CSS import順で見た目が変わること。selector差または100画面の欠落が1件でも出た委譲は次へ進めない。
- **リスク:** pure moveと機能修正を混ぜること。必要な機能修正を発見したら別施策として記録し、同じ委譲へ混ぜない。
- **点数:** Architecture +2、Docs +0.5（レビュー記載）。

## 9. 施策5 共通アクセシビリティ基盤を作る

### 9.1 目的と現在地

5画面は全て `.dialog-backdrop`、`role="dialog"`、`aria-modal="true"` を個別に持つが、共通focus trap、初期focus、閉じた後のfocus復帰、背景 `inert` はない。HelpだけがEscapeと初期検索focusを持つ（`apps/desktop/src/components/dialogs/HelpCenter.tsx:101-117`）。

共通 `ModalDialog` にfocus lifecycleを集める。ただし復旧画面のEscapeを「破棄する」に対応付けてはならない。復旧はkeyboardで復元または破棄を選べることを合格とし、中立dismissを新設する場合は復旧データを保持する別要件として利用者判断を得る。

### 9.2 対象ファイルと関数

| 実パス | 実関数 | 作業 |
|---|---|---|
| `apps/desktop/src/components/dialogs/ModalDialog.tsx`（新規） | 計画する `ModalDialog`、`focusableElements`、`handleDialogKeyDown`、`restoreFocus`、`useModalStack` | initial focus、Tab循環、Escape、focus return、topmost、背景inert、cleanup。 |
| `apps/desktop/src/components/dialogs/NewDocumentDialog.tsx` | `NewDocumentDialog`（`:18-138`） | primitiveへ移行し、最初の有効入力へfocus。 |
| `apps/desktop/src/components/dialogs/ExportDialog.tsx` | `ExportDialog`、`handleSave`（`:56-151`） | primitiveへ移行し、選択中の書出し種類へfocus。 |
| `apps/desktop/src/components/dialogs/ProposalWizard.tsx` | `ProposalWizard`（`:805-841`）と4 step component | stepごとの見出し/initial focusとbusy中Escape方針を固定する。 |
| `apps/desktop/src/components/dialogs/HelpCenter.tsx` | `HelpCenter`（`:90-243`） | F1入口は維持し、Escape/初期focusの重複をprimitiveへ移す。 |
| `apps/desktop/src/components/RecoveryDialog.tsx` | `RecoveryDialog`（`:25-75`） | 復元/破棄のkeyboard到達、破壊的Escape 0。 |
| `apps/desktop/src/lib/allScreenScenarios.ts` | dialog scenario群（`:415-500`） | 1000×700、代表状態、操作要素一覧を拡張する。 |
| `apps/desktop/package.json` / `apps/desktop/package-lock.json` | 関数名は**該当なし**。依存manifest/lockfileのため。 | 利用者承認後だけaxe系dev dependencyを追加する。 |
| `.github/workflows/ci.yml` | 関数名は**該当なし**。YAML jobのため。 | keyboard/axe DOM検査を通常frontend gateへ入れる。 |

### 9.3 段階と6個の委譲単位

| 段階 | 委譲 | 担当モデル（理由） | 作業 | 中間報告 |
|---|---|---|---|---|
| 5-A | primitive 1件（2～3人日） | **sol ultra:** focus trap、inert、topmost、多重mountの正しい共通Modal契約を設計する必要がある。 | `ModalDialog` と専用DOM test。focusable 0件、disabled、topmost、多重mountを含む。 | initial focus、Tab/Shift+Tab wrap、Escape、return、inert、listener残留の件数。 |
| 5-B | 小画面 2件（各1～2人日） | **B1/B2 terra:** 確定済みprimitiveへRecovery/New/Exportを移す機械的作業である。 | B1 Recovery+New、B2 Export。破壊操作とOS保存dialogの前後focusを分ける。 | 移行数3/5、各trigger、Escape方針、既存機能test。 |
| 5-C | Proposal 1件（2～3人日） | **terra:** 4状態とbusy規則を確定済みprimitiveへ載せ替え、固定DOM条件で判定できる。 | 4 stepとbusy stateを単独移行する。 | 4 stepのinitial target、閉じ方、busy中のkeyboard、2,400行超の既存DOM test結果。 |
| 5-D | Help 1件（1～2人日） | **terra:** F1/Escape/focus復帰を既定契約へ合わせる機械的移行である。 | F1監視を維持し、primitiveと二重Escapeをなくす。 | F1→open→search focus→Escape→trigger復帰を100回。 |
| 5-E | axe/全画面 1件（2～3人日） | **terra:** 10状態のaxe、keyboard-only、1000×700/200%を所定matrixで実行・集計する作業である。 | 依存承認後、10代表状態のaxeとkeyboard-only統合、1000×700/200%手順をCI化する。 | axe severity表、10状態×操作一覧、依存差分、CI時間。 |

5画面一括は不可。特にProposal、Help、Recoveryのclose意味を同時に変えない。

### 9.4 数値の合格条件

1. New、Export、Proposal、Recovery、Helpの5/5が共通 `ModalDialog` を使い、個別focus trap実装0件。
2. 5/5でopen時のinitial focusがちょうど1要素に入り、closeまたは選択完了後、元triggerが接続中なら5/5で復帰する。
3. 各画面でTab末尾→先頭、Shift+Tab先頭→末尾を100回反復し、dialog外へfocus流出0件。
4. dismiss可能なNew/Export/Proposal/HelpはEscape 100/100で安全に閉じる。RecoveryはEscapeによる `resolveRecovery(false)` 呼出し0、自動保存削除0で、復元/破棄ボタンへkeyboardだけで到達できる。
5. open中は背景4区画が100% `inert`、close後は100%解除。100 mount/unmount後のkeydown listener、inert属性、focus sentinel残留0。
6. axeを10代表状態で実行し、critical 0、serious 0。導入依存が未承認ならこの施策は完了扱いにしない。
7. mouseを使わず、5 dialogを開く、主要入力を行う、決定または安全に閉じる、元要素へ戻る一連の成功率5/5。
8. 1000×700かつ200%相当の10代表状態で、主要操作要素のDOM欠落0、横方向でしか到達できない決定button 0、focus対象の画面外固定0。

### 9.5 変更・緩和してはいけないもの

- `role="dialog"`、`aria-modal`、accessible name、`aria-live`、既存143 aria-label/11 aria-live相当の支援情報。
- HelpのF1、Escape、検索focus。共通化でF1入口を失わない。
- Recoveryの保存内容。Escapeを破棄へ割り当てない。
- Proposalのbusy中二重実行防止、4 step、1000×700 layout。
- 背景をinertにする際、dialog自身を同じinert subtreeへ入れない。
- 既存5 theme、focus outline、pointer操作。keyboardを足す代わりにpointerを削らない。
- ブラウザ窓を開く検査を必須化しない。DOM/CSS自動検査と、明示許可された別環境での表示確認を分ける。

### 9.6 過去の失敗と原因

- `docs/progress.md:280-286`: 1000×700で表示が隠れた。原因は個別画面の寸法だけを見て全状態を通さなかったこと。
- `docs/progress.md:29`: 100画面状態の確認で9件の欠陥が見つかった。代表1画面だけでは足りない。
- `CLAUDE.md:371-397`（§10.7.10）: 表示確認前に次へ進み、壊れた状態を積み重ねた。移行1件ごとにkeyboardと既存DOM検査を閉じる。
- `docs/progress.md:334-336`: themeの山線コントラストが1.06:1だった。aria属性だけで可視focus/contrastを代替しない。

### 9.7 道具、コマンド、確認方法

```powershell
$env:CARGO_TARGET_DIR = "C:\Users\oltot\AppData\Local\Temp\ori3-target-codexroadmap"
npm --prefix apps/desktop test -- src/components/dialogs/ModalDialog.dom.test.tsx
npm --prefix apps/desktop test -- src/components/dialogs/NewDocumentDialog.dom.test.tsx src/components/dialogs/ExportDialog.dom.test.tsx
npm --prefix apps/desktop test -- src/components/dialogs/ProposalWizard.dom.test.tsx src/components/dialogs/HelpCenter.dom.test.tsx src/components/RecoveryDialog.dom.test.tsx
npm --prefix apps/desktop test -- src/lib/allScreenScenarios.test.ts
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run build
```

axe依存追加はネットワークとlockfile変更を伴うため利用者承認後に行う。確認記録は `verification/improvement-roadmap/05-modal/keyboard-axe.json`。1000×700/200%の外部表示確認が未実行なら、DOM合格と分けて「未実行」と記録する。

### 9.8 成果物、見積、依存、リスク、点数

- **保存先:** `apps/desktop/src/components/dialogs/ModalDialog.tsx` と `.dom.test.tsx`、5画面と各DOM test、`apps/desktop/src/lib/allScreenScenarios.ts` とtest、承認時のみpackage/lock/CI、`verification/improvement-roadmap/05-modal/keyboard-axe.json`、`docs/progress.md`。
- **見積:** 9～15人日。現行6製品ファイル・合計1,971行（5 dialog + `allScreenScenarios.ts`）と既存5 DOM test・合計2,947行を対象にし、primitive lifecycle、5画面移行、Tab/Escape/focus return、inert cleanup、axe 10状態、keyboard 5経路、layout 10状態の7検査群を6委譲で閉じる。段階和は5-A 2～3、5-B 2～4、5-C 2～3、5-D 1～2、5-E 2～3人日である。
- **依存:** 施策2後、施策4のdialog/CSS分割前。施策9の必須前提。
- **リスク:** 同時に複数modalがopenしfocus/inertを奪い合うこと。topmost判定が100反復で1回でも破れたらstack managerを先に設計する。
- **リスク:** triggerがunmount済みでfocus復帰できないこと。`isConnected`を確認し、明示fallbackを1つ定める。bodyへ黙って落とさない。
- **点数:** UI +1（レビュー記載）。

## 10. 施策6 bundleを分割する

### 10.1 目的と現在地

レビュー時のmain JSは1,319.50 kB、gzip 374.46 kBで、500 kB警告が出ている（`docs/comprehensive-review-2026-08-24.md:72-78`）。`apps/desktop/src/App.tsx` は `Viewer3D`、`ProposalWizard`、`ExportDialog`、`HelpCenter` をstatic importし、`apps/desktop/vite.config.ts` に `manualChunks` やgzip budgetはない。

単に `React.lazy` へ変えて常時renderすると初回renderでimportされる。App側のopen状態でmountをgateする。また `EXPORT_CHOICES` をAppがstatic importするため小さいpure moduleへ移す。Help内のF1 listenerもlazyにすると入口が消えるため、F1だけ軽量常駐componentへ先に分ける。

レビューの「manual preview」に対応する独立runtime componentは現行ソースにない。`apps/desktop/src/help/manualExport.ts` の `buildManualExportContent` はbuild-time/test経路であり初期runtime graphへ入らない。Help表示を意味するならHelp lazyで包含し、別物なら実在する実パスを利用者が指定するまで対象を捏造しない。

### 10.2 対象ファイルと関数

| 実パス | 実関数・領域 | 作業 |
|---|---|---|
| `apps/desktop/src/App.tsx` | `App`、`ExportButton` | `lazy` / `Suspense` とopen状態gate。Viewer fallbackの寸法を固定。 |
| `apps/desktop/src/components/dialogs/ExportDialog.tsx` | `EXPORT_CHOICES`、`ExportDialog` | 定数を `exportChoices.ts` へ移し、dialog payloadだけlazyにする。 |
| `apps/desktop/src/components/dialogs/ProposalWizard.tsx` | `ProposalWizard` | `proposalStep !== null` のときだけloadする。 |
| `apps/desktop/src/components/dialogs/HelpCenter.tsx` | `HelpCenter`、現行F1 listener | F1を `HelpShortcut.tsx` へ移し、Help contentをlazyにする。 |
| `apps/desktop/src/components/Viewer3D/Viewer3D.tsx` | `Viewer3D` | Three.js到達graphを独立chunkへ置き、同寸法fallbackを用意する。 |
| `apps/desktop/vite.config.ts` | `defineConfig` | Vite manifestと必要最小限の `manualChunks`。 |
| `apps/desktop/scripts/check-bundle-budget.mjs`（新規） | 計画する `entryChunks`、`gzipBytes`、`assertBundleBudget` | manifest/indexからeager到達chunkを計算し、byte単位で検査する。 |
| `apps/desktop/package.json` / `.github/workflows/ci.yml` | 関数名は**該当なし**。script manifest/YAML jobのため。 | `check:bundle` とCI gate。 |

### 10.3 段階と6個の委譲単位

| 段階 | 委譲 | 担当モデル（理由） | 作業 | 中間報告 |
|---|---|---|---|---|
| 6-A | 計測1件（1～2人日） | **terra:** byte/gzip/chunkを指定境界で計測・記録する数値作業である。 | entry eager graph、gzip、最大raw chunk、chunk数を丸めず測る。250,000/250,001、500,000/500,001境界test。 | build ID、eager raw/gzip、全chunk表、最大、baselineの出所。 |
| 6-B | Export 1件（1～2人日） | **terra:** 確定したopen gateへdynamic importを置く機械的変更である。 | choices pure module、open gate、lazy、App test。 | closed時load 0、open時1、entry gzip差、export全機能test。 |
| 6-C | Proposal 1件（1～2人日） | **terra:** 確定したopen gateへdynamic importを置く機械的変更である。 | open gate、lazy、loading fallback、既存DOM test。 | closed時load 0、open 100回のmodule load数、候補UI回帰。 |
| 6-D | Help 1件（2人日） | **terra:** F1入口をeagerに残す境界と検査回数が確定している。 | F1入口をeager分離してHelp/chapter/image payloadをlazyにする。 | F1 100/100、focus、entryから消えたmodule一覧、Help test。 |
| 6-E | Viewer/Three 1件（2～3人日） | **sol ultra:** Viewer/Threeの初期化、WebGL資源寿命、fallbackを壊さないlazy境界の設計判断が要る。 | Viewer境界、fallback、Three chunk、load時期を固定する。 | fallback寸法差、初回3D表示、Three chunk、scene/DOM test、起動差。 |
| 6-F | 統合/CI 1件（1～2人日） | **terra:** byte gateと30回起動比較を既定手順へ固定・集計する作業である。 | byte budgetと変更前後各30回の起動比較をCI/release手順へ固定する。 | initial gzip、最大chunk、median/P95、要件3秒、全CI結果。 |

manual previewは実在確認後だけ7件目として追加する。現時点では0件として完了条件の分母へ入れない。

### 10.4 数値の合格条件

1. Vite manifestから静的に到達するinitial JSのgzip合計を250,000 bytes以下にする。
2. 全minified JS chunkをraw byteで測り、単一chunk最大500,000 bytes以下にする。
3. Proposal、Export、Helpはclosed初回render 100回でdynamic import開始0、各open 100回でload成功100、module評価は各session 1回以下。
4. F1を100回のopen/close反復で100回Help表示へつなぎ、入口消失0、二重dialog 0。
5. Viewer fallbackと実Viewerの割当区画寸法差を縦横各1 px以下とし、4区画のlayout shiftによる操作要素欠落0。
6. 同一release build・同一機・同じ測定器で変更前後各30回cold startし、変更後median `<=` 変更前median、変更後P95 `<=` 変更前P95、全実行3,000 ms以内とする。P95 baselineは実装前に新規取得する。
7. 起動値がtimer分解能内で逆転した場合だけ30回を1セット追加し、合計60回で再判定する。上限を緩めない。
8. build-time `manualExport.ts` をinitial graphへ新たに入れない。runtime manual previewと誤認した追加chunk 0。

### 10.5 変更・緩和してはいけないもの

- F1、Export tooltip、Proposalの4 step、Viewerの常設3D区画。
- lazy化のため機能や画像を削除、圧縮品質を無断低下、Help章を減らすこと。
- initial gzipを全outputサイズ、Vite表示の丸めkB、unminifiedサイズと混同しない。
- Three.jsを分けるため複数versionをbundleしない。
- 3秒要件と現行起動中央値の証拠。gzipだけ通して起動回帰を無視しない。
- `manualExport.ts` の説明書生成/test経路。

### 10.6 過去の失敗と原因

- `docs/progress.md:9`: 製品と異なる経路の時間を測った。bundleもbuild全容量、eager gzip、起動を別々に同じrelease成果物から測る。
- `CLAUDE.md:341-363`（§10.7.9）: 実測ぴったりの境界と小数完全一致でCIを落とした。byte境界は正確にし、時間は30回median/P95で判定する。
- `CLAUDE.md:319-339`（§10.7.8）: 関連箇所の一部だけを直した。Exportの定数import、HelpのF1、Appのmount gateまでimport graphで確認する。

### 10.7 道具、コマンド、確認方法

```powershell
$env:CARGO_TARGET_DIR = "C:\Users\oltot\AppData\Local\Temp\ori3-target-codexroadmap"
npm --prefix apps/desktop run build
npm --prefix apps/desktop run check:bundle
npm --prefix apps/desktop test -- src/App.dom.test.tsx
npm --prefix apps/desktop test -- src/components/dialogs/ExportDialog.dom.test.tsx src/components/dialogs/ProposalWizard.dom.test.tsx src/components/dialogs/HelpCenter.dom.test.tsx
npm --prefix apps/desktop test -- src/components/Viewer3D/Viewer3D.dom.test.tsx
npm --prefix apps/desktop run lint
```

`check-bundle-budget.mjs` はgzipをmemory上で計算し、Viteのwarning文字列をgrepしない。起動計測は明示許可された別工程でreleaseアプリを使い、本書作成中は実行しない。結果を `verification/improvement-roadmap/06-bundle/manifest.json`、`verification/improvement-roadmap/06-bundle/before-startup.json`、`verification/improvement-roadmap/06-bundle/after-startup.json` に保存する。

### 10.8 成果物、見積、依存、リスク、点数

- **保存先:** 上表の実装ファイル、`apps/desktop/src/components/dialogs/exportChoices.ts`、`HelpShortcut.tsx`、`apps/desktop/scripts/check-bundle-budget.mjs`、package/CI、`verification/improvement-roadmap/06-bundle/`、`docs/progress.md`。
- **見積:** 8～13人日。現行6製品/configファイル・合計3,806行（`App.tsx`、3 dialog、`Viewer3D.tsx`、`vite.config.ts`）と既存5 DOM test・合計4,703行を対象にし、byte境界、Export、Proposal、Help/F1、Viewer/Three、30+30起動の6検査群を6委譲で閉じる。段階和は6-A 1～2、6-B 1～2、6-C 1～2、6-D 2、6-E 2～3、6-F 1～2人日である。
- **依存:** 施策4のimport境界、施策5のModal/F1 focus契約後。
- **リスク:** lazy componentを無条件renderして初期loadすること。closed 100回でloadが1回でも始まればgate設計へ戻る。
- **リスク:** Viewerを遅延して初期3D操作を欠落させること。fallbackから実体への移行で寸法差>1 pxまたは最初の操作欠落1件ならViewerだけ差し戻す。
- **点数:** NFR +0.5、UI +0.5（レビュー記載）。

## 11. 施策7 文書を実装から機械検証する

### 11.1 目的と方針

version、workspace数、Tauri command数、テスト数、提案budget、manualページ数の6種類を実装・成果物から生成し、数値の手複写を止める。`crates/ori3-propose/src/search.rs` の画面用budget説明は現在30,000 msで、`PLAN_BUDGET.max_millis=30_000` と一致する。この一致を機械検証する。

新しい恒久 `docs/current-status.md` は、現要件の文書構成を利用者判断なしに増やすため採らない。`docs/progress.md` の先頭に機械生成領域を置き、詳細JSONは `verification/improvement-roadmap/07-docs/current-status.json` に置く。利用者が別文書を承認した場合だけ保存先を変更する。さらに、`docs/requirements-definition.md` の表はTauri commandを18個と記し、現行 `apps/desktop/src-tauri/src/lib.rs` のhandlerも `fold_all_preview`、`proposal_progress`、`proposal_control` を含む18個で一致する。この一致をdrift fixtureで継続検査する。

### 11.2 対象ファイルと関数

| 実パス | 実関数・領域 | 作業 |
|---|---|---|
| `scripts/generate-current-status.ps1`（新規） | 計画する `Get-WorkspaceVersion`、`Get-WorkspaceMembers`、`Get-TauriCommands`、`Get-TestInventory`、`Get-ProposalBudgets`、`Get-ManualPageCount`、`Write-GeneratedBlock` | 6指標を構造化して生成する。 |
| `scripts/check-ci.ps1` | `Assert-CiStepsMatch` とexpected step表 | 生成差分検査を通常gateへ追加する。 |
| `scripts/check-release-ready.ps1` | `Get-CargoWorkspaceVersion`、version照合 | 既存version正本を再利用し二重実装しない。 |
| `docs/progress.md` | 関数名は**該当なし**。時系列/現状文書のため。 | marker内だけを生成し、手書き履歴を保持する。 |
| `docs/implementation-roadmap.md` | 関数名は**該当なし**。計画文書のため。 | checkboxへ受入test名または明示的手動条件をリンクする。 |
| `Cargo.toml` / `apps/desktop/package.json` | 関数名は**該当なし**。manifestのため。 | version/workspace/scriptの読取元。 |
| `apps/desktop/src-tauri/src/lib.rs` | `run` と `tauri::generate_handler!`（`:77-99`） | Tauri command数の正本。 |
| `apps/desktop/src-tauri/src/commands.rs` | `PLAN_BUDGET` | 製品budgetの正本。 |
| `crates/ori3-propose/src/search.rs` | `SearchBudget`、既定値 | library defaultと製品overrideを役割別に生成する。 |
| `.github/workflows/ci.yml` | 関数名は**該当なし**。YAML jobのため。 | 生成差分0を検査する。 |

### 11.3 段階と13個の委譲単位

| 段階 | 委譲 | 担当モデル（理由） | 作業 | 中間報告 |
|---|---|---|---|---|
| 7-A | schema/正本 1件（1～2人日） | **sol ultra:** 6指標の正本source、数え方、profile、markerという証拠契約を決める必要がある。 | 6指標それぞれのsource、数え方、profile、表示形式、markerを決める。 | source path/function、現値、重複記述一覧、曖昧なtest countの定義。 |
| 7-B | generator 1件（2～3人日） | **terra:** 7-Aで確定したcollectorと生成物を実装しidempotenceを測る手順作業である。 | 6 collectorとJSON/Markdown生成、2回実行のidempotence test。 | 6/6値、同一hash、手書き領域差0、error時exit code。 |
| 7-C | CI差分gate 1件（1～2人日） | **terra:** 確定済みmarkerと一時出力の差分gateをCIへ置く機械的作業である。 | 一時出力と正本markerを比較し、差分時に原因sourceを示す。 | clean差分0、故意に6値をずらした6/6 fixtureでfail、job時間。 |
| 7-D1 | M0 link（0.5～1人日） | **sol ultra:** M0 checkboxを何が立証するか意味判断が要る。 | M0の11 checkboxを実検査名または手動受入IDへ結ぶ。 | 11/11のlink、正本source、未対応、進捗との矛盾。 |
| 7-D2 | M1 link（1～2人日） | **sol ultra:** M1 checkboxを何が立証するか意味判断が要る。 | M1の38 checkboxを同じ規則で結ぶ。 | 38/38のlink、自動/手動内訳、未対応、矛盾。 |
| 7-D3 | M2 core link（1～2人日） | **sol ultra:** solver/layers/sequence証拠の妥当性判断が要る。 | Task 2-0～2-3の24 checkboxを結ぶ。 | 24/24のlink、solver/layers/sequence検査、未対応、矛盾。 |
| 7-D4 | M2 UI/technique link（1～2人日） | **sol ultra:** UI/技法証拠の妥当性判断が要る。 | Task 2-4～2-6cの34 checkboxを結ぶ。 | 34/34のlink、UI/技法検査、未対応、矛盾。 |
| 7-D5 | M2 acceptance/recovery link（0.5～1人日） | **sol ultra:** 受入/復旧証拠の妥当性判断が要る。 | Task 2-7～2-9の10 checkboxを結ぶ。 | 10/10のlink、受入/復旧検査、未対応、矛盾。 |
| 7-D6 | M3 link（0.5～1人日） | **sol ultra:** 提案証拠の妥当性判断が要る。 | M3の12 checkboxを結ぶ。 | 12/12のlink、提案検査、未対応、矛盾。 |
| 7-D7 | M3強化 link（1～2人日） | **sol ultra:** 判断記録・実測・testのどれを証拠とするか決める必要がある。 | M3強化の33 checkboxを結ぶ。 | 33/33のlink、判断記録/実測/testの内訳、未対応、矛盾。 |
| 7-D8 | M4 link（1～2人日） | **sol ultra:** 技法・書出し・受入の正しい証拠対応を決める必要がある。 | M4の19 checkboxを結ぶ。 | 19/19のlink、技法/書出し/受入検査、未対応、矛盾。 |
| 7-D9 | M5/M6 link（0.5～1人日） | **sol ultra:** checkboxのないM6受入基準へ正しい受入IDを与える仕様判断が要る。 | M5/M6の1 checkboxと、見出しだけでcheckboxがないM6の受入基準を結ぶ。 | checkbox 1/1、M6受入ID 1/1、未対応、矛盾。 |
| 7-D10 | 182件統合監査（1～2人日） | **terra:** D1～D9の確定JSONを結合して重複・欠番・不一致を機械集計する作業である。 | D1～D9のJSONを結合し、重複、欠番、進捗との不一致を機械判定する。個別linkの意味をこの委譲で再調査しない。 | 182/182、重複0、未対応0、既存実装を未着手へ戻した件数0、生成hash。 |

13単位への分割が必要。特に182 checkbox全部を1回へ戻さず、M2もcore、UI/technique、acceptance/recoveryの3群に分ける。script、CI、2文書を1回で直さない。

### 11.4 数値の合格条件

1. version、workspace数、Tauri command数、Rust/frontend test数、提案budget、manualページ数の6/6を生成する。
2. 現行のTauri command 18個を18個として生成し、要件表の記載数・行数との不一致を0にする。追加時は表も同時更新する。
3. 6,000/30,000 msのように同じ役割を異なる値で記す不一致0。library default、製品override、test-only値は役割名を付けて別値として表示する。
4. generatorを連続2回実行し、JSON hashとMarkdown markerが100%同一、marker外差分0。
5. CIが一時生成した内容と正本の差分0。6指標それぞれの故意のdrift fixture 6/6でnon-zero終了する。
6. `docs/implementation-roadmap.md` の全checkboxについて、少なくとも1つの自動test名または一意な手動受入IDへの対応率100%、未対応0。
7. `docs/progress.md` で実装済み、`docs/implementation-roadmap.md` で未完了となる既知不一致0。単に全checkboxを完了へするのではなく証拠linkで判定する。
8. 版番号の既存4ファイル5箇所、manual鮮度、Help鮮度、CI job表の既存release-ready検査を全て維持する。

### 11.5 変更・緩和してはいけないもの

- 要件正本、実装ロードマップ、進捗の役割。生成器が要件を自動変更しない。
- `docs/progress.md` の過去失敗・実測履歴。marker外を書き換えない。
- test数を増やすため空検査を足す、減少を隠すため数え方を変えること。
- 製品budgetとlibrary default/test-only budgetの区別。
- manualのページ数だけで内容を正しいと判定すること。既存Help/manual内容・画像鮮度検査を残す。
- 利用者向け文書へ内部アルゴリズム詳細を露出しない（`CLAUDE.md:219-224`）。

### 11.6 過去の失敗と原因

- `CLAUDE.md:207-217`: 版更新後の機能がHelp/PDFへ載らない期間があった。原因は複数の手更新と鮮度検査不足。
- `CLAUDE.md:147-158`: PDFが生成できたことを内容確認と誤認した。ページ数は6指標の1つに過ぎず、内容検査を置換しない。
- `CLAUDE.md:116-124`（§10.1）: 手元とCIで入力集合が違った。generatorは追跡sourceだけを読み、CIと同じcommandを使う。
- 現在の `search.rs` の画面用budget説明は30,000 msで製品30,000 msと一致する。旧6,000 ms記述は、測定値をcode commentへ複製したことで陳腐化した過去の失敗である。

### 11.7 道具、コマンド、確認方法

```powershell
$env:CARGO_TARGET_DIR = "C:\Users\oltot\AppData\Local\Temp\ori3-target-codexroadmap"
powershell -ExecutionPolicy Bypass -File scripts/generate-current-status.ps1 -Check
powershell -ExecutionPolicy Bypass -File scripts/check-ci.ps1
powershell -ExecutionPolicy Bypass -File scripts/check-release-ready.ps1
rg -n "6,000|6000|30,000|30000|max_millis|PLAN_BUDGET" crates apps docs/requirements-definition.md docs/implementation-roadmap.md docs/progress.md
```

generatorの通常実行は一時ファイルへ出し、`-Check` がmarkerと比較する。実行時にCargoを呼ぶ場合だけ指定の一時targetを使う。結果は `verification/improvement-roadmap/07-docs/current-status.json` と `links.json`。

### 11.8 成果物、見積、依存、リスク、点数

- **保存先:** `scripts/generate-current-status.ps1`、`scripts/check-ci.ps1`、`scripts/check-release-ready.ps1`、`docs/progress.md`、`docs/implementation-roadmap.md`、`.github/workflows/ci.yml`、`verification/improvement-roadmap/07-docs/current-status.json`、`verification/improvement-roadmap/07-docs/links.json`。
- **見積:** 12～23人日。新規generator 1ファイルと、現行10 source/document/configファイル・合計8,228行を対象にし、6 collector、JSON/Markdown 2出力、drift fixture 6件、roadmap checkbox 182件、release-ready既存4項目の5検査群を13委譲で閉じる。段階和は7-A 1～2、7-B 2～3、7-C 1～2、7-D1～D10 8～16人日である。
- **依存:** 施策1/2の実関数とbudget確定後。後続全施策は生成領域を更新する。
- **リスク:** 正規表現でRust/TSを誤集計すること。6 collectorに最小fixtureを持たせ、構文が変わったとき黙って0を返さずfailする。
- **リスク:** 新しい恒久文書を無断追加すること。利用者承認がなければ `docs/progress.md` markerを使う。
- **点数:** Docs +2（レビュー記載）。

## 12. 施策8 FOLD 1.2 限定profileを実施する

### 12.1 現要件と判断

要件正本上、FOLDは現時点で明示的な非目標である。

- `docs/requirements-definition.md:57`: `### 4.2 やらないこと(明示的な非目標)`
- 同 `:59`: v1では実装せず、**設計の余地も残さない**。将来行う場合は要件から再定義する。
- 同 `:66`: `FOLD / DXF / OBJ / STL / glTF の入出力`
- 同 `:386`: §4.2の非目標を変える場合は要件書改訂を必須とする。

2026-08-24、利用者は§0Aの**FOLD 1.2 限定profileを承認した**。したがって施策8は利用者判断待ちではなく、承認済み範囲の実行計画とする。ただしこの決定だけでは要件正本の`:66`は変わらない。`:386`の手続きに従い、§0B.2と次節の条文を同じ要件書改訂へ反映してからコードへ着手する。F/U完全往復の41～62人日案は不採用であり、将来分岐・追加工数・代替profileとしても本ロードマップへ含めない。

### 12.2 承認済み要件改訂案（今回は正本を変更しない）

#### §4.1へ追加する案

> - **FOLD 1.2 限定**profileの読込/書出し（2D頂点、edge topology、B/M/V、`edges_foldAngle`、表現可能な非循環 `faceOrders`、線形step frame）。対応範囲、対応外の内容、縮退時の警告は§6.9 FOLD-001～006に従う。

#### §4.2の66行を置き換える案

> - DXF / OBJ / STL / glTF の入出力

#### §6へ新設する案

現行§6の表形式に合わせ、6件すべてを同じ優先度・同じ時期にする。安全項目だけMUST、coreだけSHOULDという混在は採らない。

| ID | 要件案 | 優先度 | 時期 |
|---|---|---|---|
| FOLD-001 | FOLD 1.2 JSONの2成分 `vertices_coords`、`edges_vertices`、`edges_assignment` を読み書きし、正方形または長方形の単一紙へ変換する。edge topologyとB/M/Vを保持する。3成分の頂点座標は対応外とする。 | MUST | M7 |
| FOLD-002 | 対応edgeの `edges_foldAngle` をf64で読み書きし、B/M/VとORIGAMI3の線種・driver角の対応および角度の符号規則を1つの変換表で固定する。 | MUST | M7 |
| FOLD-003 | 現行 `FoldStep.layer_order` へ損失なく変換できる非循環 `faceOrders` だけを読み書きする。変換不能な制約を近似して成功扱いにしない。 | MUST | M7 |
| FOLD-004 | 線形なstep frameだけを読み書きする。枝分かれした手順、動画、任意の継承関係は対応外として警告する。 | MUST | M7 |
| FOLD-005 | 画面と利用者向け文書には対応名称を必ず「FOLD 1.2 限定」と表示し、「FOLD対応」「FOLD完全対応」と表示しない。3D座標、枝分かれした手順、動画、名前付き技法の意味、注記、仕上げの丸み、FOLDの「平ら(F)」「未指定(U)」の区別を、利用者から見える対応外一覧へ表示する。入力中のF/Uは`Aux`へ縮退し、どのedgeがFまたはUだったかをpath付き警告として残す。無言で捨てる件数を0とする。 | MUST | M7 |
| FOLD-006 | FOLD取込は元ファイルを`.ori3`の上書き先にせず、未保存の新規ORIGAMI3作品として扱う。対応外fieldと表現不能構造をpath付き警告一覧で返し、利用者が取込中止または明示的な限定取込を選べるようにする。通常の3D頂点は保存しない。 | MUST | M7 |

#### §9.2 Rustクレート表を変える案

- `ori3-export` の責務を「SVG/PNG/折り図PDF生成、およびFOLD 1.2 限定profileの中立なimport/export変換」へ改訂する。既存crateへimport責務を加える以上、この行を変えずに実装しない。
- FOLD formatが増えて依存や責務の分離根拠が得られた場合だけ、`ori3-interchange` の新設を別の要件・Cargo承認として判断する。最小profileでは新crateを前提にしない。

#### §9.3 IPC表を変える案

- `document_open` を `.ori3` とFOLD 1.2 限定profileの読込（形式enumまたは確実な拡張子/内容判定）へ拡張する。FOLD時は `DocumentStore::import_fold` を呼び、dirtyな未保存作品として返す。
- `document_export` の既存 `ExportKind` に `FoldJson` を追加する。
- import/exportごとにTauriコマンドを増やす前に既存enumへ集約する。コマンド数自体に上限は置かない。

#### §11マイルストーン表へ追加する案

| MS | 内容 | 受け入れ基準 |
|---|---|---|
| M7 | FOLD 1.2 限定profileのimport/export | 4出所30 fixtureでpanic 0、限定profile 20件のtopology・B/M/V・折り角・step終点・層制約が許容差内で一致し、画面と利用者向け文書の名称表示率100%、対応外7項目の表示7/7、F/U入力の縮退警告率100%、無言の破棄0 |

この改訂では任意多角形の紙（現行 `docs/requirements-definition.md:65`）や通常3D頂点保存（§2設計原則7）まで解除しない。

### 12.3 最小範囲でできること／できないこと

| 区分 | できること | できないこと |
|---|---|---|
| 実装途中のCP交換pilot | 2D頂点、edge topology、B/M/Vのexact往復、F/Uの警告付き`Aux`縮退、正方形・長方形、外部FOLD 1.2 toolとの基本CP交換 | `edges_foldAngle`、`faceOrders`、step frameをまだ検査しないため、施策8の完成や製品公開にはならない。 |
| **承認済みのレビュー適合最小profile** | 上記に`edges_foldAngle`、表現可能な非循環`faceOrders`、線形step frameを加え、step終点まで往復する。 | 3D座標、枝分かれした手順、動画、名前付き技法の意味、注記、仕上げの丸み、FOLDのF/U区別。これら7項目は必ず利用者へ表示する。 |
| その他の形式構造 | 承認範囲内の正方形・長方形、B/M/V、generic driver角、層制約 | 非多様体、穴/切抜き、任意多角形、曲線制御点、独自extension、承認範囲外metadataの損失なし往復。対応外はpath付きで示す。 |

現行 `crates/ori3-model/src/lib.rs:20-25` の `EdgeKind` は `Border/Mountain/Valley/Aux` の4種だけで、FOLDのF（flat）とU（unassigned）を別々に保存できない。承認済みprofileではF/Uを`Aux`へ縮退し、元assignmentとJSON pathを警告に残し、M/V/Bだけをexact往復対象にする。**F/U完全往復案は2026-08-24に不採用となったため、variant追加、`.ori3` schema移行、追加工数は本計画に含めない。** 新しい利用者決定なしに再導入しない。

同様に、`faceOrders` と現行 `FoldStep.layer_order` は一般には同型ではない。限定profileで変換できないものが出た場合は、警告だけで意味を変えて取込むのではなく「対応外」と判定する。完全対応を望む場合はmodel schemaの追加と旧`.ori3`移行が別の要件変更になる。

### 12.4 対象ファイルと関数

最小の依存変更に留めるため、最初は既存 `ori3-export` に中立なFOLD変換moduleを置く案を採る。formatが増え、責務分離の実測根拠ができた場合だけ新crateを別承認する。

| 実パス | 実関数・計画関数 | 作業 |
|---|---|---|
| `docs/requirements-definition.md` | 関数名は**該当なし**。要件正本の条文・表・milestoneであり実行コードではない。 | 承認済み範囲を反映する正本改訂工程で§4.1、§4.2、§6、§9.2、§9.3、§11を同時改訂する。 |
| `docs/implementation-roadmap.md` | 関数名は**該当なし**。実装順・受入checkboxの文書であり実行コードではない。 | 承認されたM7を、下記15委譲と実検査名へ結ぶ。 |
| `docs/progress.md` | 関数名は**該当なし**。到達点・実測の時系列文書であり実行コードではない。 | 採否、限定profile、30件結果、対応外fieldを要約する。 |
| `crates/ori3-export/src/fold.rs`（新規） | `parse_fold_1_2`、`write_fold_1_2`、`unsupported_fields`、`fold_to_document`、`document_to_fold` | typed JSON、限定profile検証、変換、警告。 |
| `crates/ori3-export/src/lib.rs` | module/public export（現行`:1-14`） | FOLD APIを公開する。 |
| `crates/ori3-model/src/lib.rs` | `EdgeKind`（`:20-25`）、`CreasePattern`、`FoldStep`、`Document` | F/U縮退を明文化する。`Flat` / `Unassigned` variantとschema変更は行わない。 |
| `apps/desktop/src-tauri/src/store.rs` | `DocumentStore::open`（`:196-211`）、計画する `DocumentStore::import_fold` | 成功時だけ新規dirty documentへ入替え、失敗時不変。 |
| `apps/desktop/src-tauri/src/commands.rs` | `document_open`、`ExportKind`、`document_export`、`export_files` | 既存open/export境界へformat enumを通す。 |
| `apps/desktop/src/lib/types.ts` | `ExportKind`、`DocumentView`のwarning | `FoldJson` と警告型。 |
| `apps/desktop/src/ipc/client.ts` | `documentOpen`、`documentExport` | formatを型付きで渡す。 |
| `apps/desktop/src/App.tsx` | `App`内のopen/file filter経路 | `.fold` filterと未保存扱い。 |
| `apps/desktop/src/components/dialogs/ExportDialog.tsx` と、施策6後の `apps/desktop/src/components/dialogs/exportChoices.ts` | 現行 `EXPORT_CHOICES`、`ExportDialog` | FOLD選択、対応範囲、警告。施策6で定数を分離済みなら元moduleへ戻さない。 |
| `crates/ori3-export/tests/fold_roundtrip.rs`（新規） | `import_corpus_never_panics`、`supported_profile_round_trips`、`unsupported_fields_are_reported` | 30件外部corpusと内部roundtrip。 |

`Cargo.toml`、`Cargo.lock`、`vendor/` は本施策の承認と依存方針を得るまで変更しない。serde/serde_jsonの既存依存だけで実装できるかを最初に確認する。

### 12.5 段階と15個の作業単位

| 段階 | 作業単位 | 担当モデル（理由） | 作業 | 中間報告 |
|---|---|---|---|---|
| 8-A 承認反映 | **1件、非コード**（1～2人日） | **terra:** 利用者が決めたprofileを要件・roadmap・progress差分へそのまま反映する文書作業で、新しい範囲判断はしない。 | §0B.2と§12.2の確定条文、対応外、4出所corpus条件を正本へ反映する。 | 採用profile、条文差分、FOLD-001～006/M7の100%対応、未決事項0。 |
| 8-B core | **4委譲**（各2～3人日） | **B1 sol ultra:** parserの欠落・不正値契約を決める。<br>**B2 sol ultra:** 表現可否とpath付きwarning/reject境界を決める。<br>**B3 sol ultra:** writerのfield/frame/assignment表現の正しさを決める。<br>**B4 sol ultra:** canonical JSONとroundtrip比較の意味を決める。 | B1 typed parser、B2限定profile validator/unsupported path、B3 writer、B4 canonicalizer。parser+validator、writer+canonicalizerを一括にしない。 | 各回のfield対応表、malformed case、canonical JSON、silent drop数、公開API。 |
| 8-C backend | **2委譲**（各3～4人日） | **C1 sol ultra:** B/M/V、F/U→Aux+警告、角度、step frame、faceOrdersの幾何・数値変換を決める。<br>**C2 sol ultra:** store/import/open/exportの原子transactionと失敗契約を設計する。 | C1 model/converterのB/M/V/F/U、angle、step frame、face order対応、C2 `DocumentStore::import_fold` + 既存open/export IPCの原子transaction。 | C1は変換表と終点/層制約、C2は取込前後document、dirty/path、warning、失敗時store不変。 |
| 8-D frontend | **1委譲**（3～4人日） | **sol ultra:** filter、警告、限定名称、対応外一覧をfrontend全体で一貫させる仕様変更である。 | filter、export choice、警告一覧、対応範囲説明。 | keyboard操作、4区画不変、名称表示、対応外7/7、既存open/export回帰。 |
| 8-E corpus | **6委譲**（各2～3人日） | **E1～E6 terra:** 各5 fixtureを既定quota/schemaに従い取得・checksum・license・分類する反復作業である。 | E1～E6が各5 fixtureを取得・正規化・manifest化する。4出所の最終quotaは8-E開始前に予約し、各fixtureのsource/license/checksumを持つ。 | 各5件の出所、supported/unsupported分類、panic、roundtrip差、未対応field、累計quota。 |
| 8-F 統合 | **1委譲**（3～4人日） | **terra:** 30外部・4内部・100 malformedと全gateを実行、集計、文書生成する手順作業である。 | 30件、内部4件、100 malformed、UI/backend/full gateを同一buildで集計し、要件・roadmap・progressの差分を閉じる。 | 4出所quota、全数値、license未決0、生成report、既存形式回帰、未実行0。 |

全体一括不可。承認済み限定profileの15単位は、どれも4人日を超える1回の委譲にしない。F/U完全往復案は不採用なので、追加委譲は0件である。

### 12.6 数値の合格条件

1. FOLD 1.2公式sample 6件、ORIPA出力8件、Oriedita出力8件、Origami Simulator出力8件の4出所・合計30件を、限定profile内20件・意図的な対応外10件としてmanifest化し、30/30でpanic 0、分類結果の10回不一致0とする。出所/licenseを確認できないquotaを別出所の水増しで埋めない。
2. profile内20件は20/20で取込成功。対応外10件は10/10で少なくとも1つのfield pathを表示し、無言の成功0。
3. ORIGAMI3→FOLD→ORIGAMI3で、canonicalized vertex/edge topology完全一致、B/M/V一致率100%、2D座標最大誤差 `<=1e-9`、fold angle最大誤差 `<=1e-9` degree。F/Uは`Aux`へ縮退し、入力中のF/U edge全件で元assignmentとpathを警告へ残す率100%、無言の破棄0。
4. 線形step frameの各終点で全頂点finite、対応終点距離 `<=1e-6`、seam `<=1e-6`、penetration 0。step数・順序一致率100%。
5. 対応対象 `faceOrders` はcanonical triple集合の一致率100%、循環または表現不能制約は成功扱い0。
6. 未対応field総数に対する表示率100%、path欠落0、silent drop 0。未知extensionを最低20種類含む。
7. malformed/巨大でない不正JSON 100件でpanic 0、現Document、step_creases、history、dirty、pathの変更0。
8. 折り鶴、やっこさん、カエル、鳥の基本形の内部4 fixtureで100回連続のimport→export→importを行い、上記全数値を100/100で満たす。
9. file-open filter、書出し選択、取込結果/警告、Help、要件文、利用者向け説明書の6/6で名称を正確に**「FOLD 1.2 限定」**と表示する。単独の「FOLD対応」または「FOLD完全対応」という名称は0件。
10. 利用者から見える対応外一覧に、3D座標、枝分かれした手順、動画、名前付き技法の意味、注記、仕上げの丸み、FOLDの「平ら(F)」「未指定(U)」の区別を7/7表示し、Helpと取込/書出し入口の双方から1操作以内で到達できる。
11. F/Uを含む外部・内部fixtureではF/U edgeの`Aux`縮退率100%、元のF/U値とJSON pathを持つ警告率100%、警告なしの取込成功0。対応外一覧でもF/Uの区別を保持できないことを表示する。

### 12.7 変更・緩和してはいけないもの

- §0B.2の要件正本改訂前のコード、Cargo構成、file filter、ExportKind変更。
- f64、明示epsilon、通常3D状態を保存しない原則。
- 任意多角形、3D座標、branch frame等を「だいたい読めた」と成功扱いすること。
- 未対応fieldを削ってwarning 0に見せること。
- `.fold` を元の保存先として上書きすること。import後は未保存作品。
- 既存 `.ori3` schema、SVG/PNG/PDF/SVG pagesの往復とfile dialog。
- 外部fixtureの出所/license/checksum。取得できないファイルを競合出力と推測して置かない。
- 不採用となったF/U完全往復、`EdgeKind::Flat` / `EdgeKind::Unassigned`、そのためのschema移行を承認範囲へ戻さない。

### 12.8 過去の失敗と原因

- `CLAUDE.md:285-300`（§10.7.6）: fixture生成と検査を兼ね追跡fileを書き換えた。FOLD corpusもread-onlyにし取得/正規化scriptを分ける。
- `CLAUDE.md:269-283`（§10.7.5）: 変更したcrateだけを検査して他層を見落とした。model、converter、store、IPC、UI、roundtripを縦に確認する。
- `CLAUDE.md:341-366`（§10.7.9）: 計算小数の完全一致でCIを落とした。座標・角・終点は明示epsilon、topology/assignmentだけexactにする。
- `docs/requirements-definition.md:59,386`: 非目標をコードだけで越えるとスコープ肥大を再発する。要件決定を最初の停止gateにする。
- `docs/progress.md:5-9`: 現行の端から端の公開証拠は3標本で、製品と違う経路を測った誤りも記録されている。内部fixtureだけを外部互換30件と数えず、実際の4出所fileを同じimport経路へ通す。

### 12.9 道具、コマンド、確認方法

```powershell
$env:CARGO_TARGET_DIR = "C:\Users\oltot\AppData\Local\Temp\ori3-target-codexroadmap"
cargo test -p ori3-export --test fold_roundtrip
cargo test -p desktop --lib fold
npm --prefix apps/desktop test -- src/App.dom.test.tsx src/components/dialogs/ExportDialog.dom.test.tsx
rg -n "FoldJson|parse_fold_1_2|write_fold_1_2|unsupported_fields|import_fold|faceOrders|edges_foldAngle|file_frames" crates apps
```

外部corpus取得にはnetworkとライセンス確認が必要なので、利用者承認を別に得る。結果は `verification/improvement-roadmap/08-fold/compatibility.json` と `field-matrix.json`、要約は `docs/progress.md`。本書作成中は取得も実行も行わない。

### 12.10 成果物、見積、依存、リスク、点数

- **保存先:** 上表の実装/検査ファイル、`crates/ori3-export/tests/fixtures/fold/manifest.json` と30 fixture、`verification/improvement-roadmap/08-fold/compatibility.json`、`verification/improvement-roadmap/08-fold/field-matrix.json`、正本改訂時の `docs/requirements-definition.md`、`docs/implementation-roadmap.md`、`docs/progress.md`。
- **見積:** **承認済み限定profileへ33～48人日で確定する。** 現行11ファイル・合計11,908行（要件/roadmap/progress 2,335行、converter到達先8コードファイル9,573行）と新規2コード/testファイル、30外部fixtureを対象にし、parser/malformed、writer/canonical、model変換、store原子性、UI、4出所30件、内部4件×100回の7検査群を15委譲で閉じる。段階和は8-A 1～2、8-B 8～12、8-C 6～8、8-D 3～4、8-E 12～18、8-F 3～4人日である。利用者が不採用としたF/U完全往復41～62人日は足さず、追加工数0とする。
- **依存:** 利用者の範囲承認は2026-08-24に完了した。コード着手前に§0B.2/§12.2の要件正本改訂を完了し、その後は施策1、3、4、6完了後に実装する。
- **リスク:** `faceOrders` と現行modelが非同型。表現不能なfileは対応外としてpath付きで拒否し、意味を近似しない。schema拡張や承認profile縮小が必要になった場合は停止し、新しい利用者判断へ戻す。
- **リスク:** 限定名称が全仕様を扱うように誤読されること。6表示面の名称100%、対応外7/7、F/U警告100%のいずれか1件でも欠けたら公開しない。
- **点数:** 利用者承認だけ、または要件正本未改訂の現在は+0。要件改訂と全実装・検査後は**推測**でEXP +0.5、SYS +0.5、Docs +0.5。レビュー上は12分野点より競争劣位解消の意味が大きい。

## 13. 施策9 実利用者15名で検証する

### 13.1 この作業機で実施可能か

この作業機とこのCodex作業だけでは、15名の独立した実利用者テストを実施できない。参加者募集、同意、日程調整、観察者が存在せず、本書作成中はブラウザと `desktop.exe` の起動も禁止されているため、実施済みsessionは0/15である。エージェントが自動操作を15回行っても15名とは数えない。

2026-08-24、利用者は**準備だけ**を承認した。今進める範囲は、課題文、同意文、集計様式、観察手順、keyboard-only自動検査、拡大200%自動検査である。15名sessionは利用者の都合がつく時まで実施保留とし、準備が全て合格しても`session 0/15`の間はUI・NFRとも加点0とする。自動操作を人の理解の代替にしない。

### 13.2 対象ファイルと関数

| 実パス | 実関数 | 作業 |
|---|---|---|
| `verification/improvement-roadmap/09-usability/test-plan.md` | 関数名は**該当なし**。人を対象とする観察計画であり製品関数ではない。 | cohort、進行、成功定義、分析式を固定する。 |
| `verification/improvement-roadmap/09-usability/test-script.md` | 関数名は**該当なし**。課題文と観察手順を含む読み上げscriptのため。 | 5課題、追加説明禁止、観察者発話、介入条件を固定する。 |
| `verification/improvement-roadmap/09-usability/consent.md` | 関数名は**該当なし**。参加者向け同意文であり実行コードではない。 | 目的、記録項目、任意参加、撤回、匿名化、保存期間を明記する。 |
| `verification/improvement-roadmap/09-usability/session-schema.json` | 関数名は**該当なし**。匿名観察record schemaのため。 | 時刻、成否、誤操作、質問、help、重大行止まり。 |
| `verification/improvement-roadmap/09-usability/results.json` | 関数名は**該当なし**。匿名集計様式のJSONであり実行コードではない。 | 0/15の空template、cohort/課題別分母、欠測、重大問題欄を固定する。 |
| `apps/desktop/src/lib/allScreenScenarios.ts` | 関数名は**該当なし**。画面状態を宣言するdata moduleである。対象exportは `ALL_SCREEN_SCENARIOS`、`ScreenScenario`、`ScreenScenarioCoverage`。 | 5課題のkeyboard-only補助検査へ必要状態を追加する。 |
| `apps/desktop/src/App.dom.test.tsx` | 関数名は**該当なし**。VitestのApp DOM test moduleであり製品関数を定義しない。 | 新規作成、折り線、3D、手順、PDFの5 keyboard入口と、1000×700を200%拡大した相当viewportを検査する。 |
| `apps/desktop/src/lib/allScreenScenarios.test.ts` | 関数名は**該当なし**。画面scenarioのVitest test moduleであり製品関数を定義しない。 | 5課題の必要状態、10代表状態の操作要素、拡大200%時のfocus到達を検査する。 |
| `apps/desktop/src/components/dialogs/ModalDialog.dom.test.tsx`（施策5で新規） | 関数名は**該当なし**。共通modalのVitest test moduleであり製品関数を定義しない。 | keyboard-only時のfocus lifecycleを検査する。 |

NFR-006により恒久文書を無断で増やさず、protocol/raw evidenceは `verification/`、匿名要約は `docs/progress.md` に置く。恒久的な利用者調査文書群を `docs/` へ増やす場合は利用者承認を得る。

### 13.3 参加者と5課題

- 初心者5名: 折り紙設計/CADの継続利用経験なし。
- CP経験者5名: 展開図を読み書きした経験あり。
- 設計経験者5名: CPまたは折り手順を自作した経験あり。
- 同一人物を複数cohortへ重複計上しない。開発者、実装担当、事前に課題を見た人は15名へ入れない。

5課題は、(1) 新しい紙を作る、(2) 山/谷の折り線を追加する、(3) 3Dで紙を折る、(4) 手順を記録して再生する、(5) 折り図PDFを書き出す、である。課題用作品は折り鶴、やっこさん、カエル、鳥の基本形から選び、cohort間で同じ初期fileと説明を使う。

### 13.4 段階と8個の作業単位

| 段階 | 作業単位 | 担当モデル（理由） | 作業 | 中間報告 |
|---|---|---|---|---|
| 9-A protocol（**準備承認済み**） | **1件、非コード**（2～3人日） | **sol ultra:** cohort、同意、5課題、成功/停止定義、観察者介入、分析式という調査契約を決める必要がある。 | 課題文、同意文、集計様式、観察手順を作り、cohort、成功/行止まり/専門語停止の操作定義、分析式を固定する。 | 確保人数0/15、4成果物、除外条件、75試行の母数、観察者の介入規則。 |
| 9-B1 keyboard（**準備承認済み**） | **1委譲**（2～3人日） | **terra:** keyboard-onlyの5経路を確定手順で自動検査する作業である。 | 5課題をpointerなしで開始・完了できるDOM検査を作る。 | 5/5経路、focus順、modal復帰、到達不能0、既存DOM回帰。 |
| 9-B2 200%拡大（**準備承認済み**） | **1委譲**（2～3人日） | **terra:** 確定viewportと10状態の操作matrixを実行・集計する数値作業である。 | 1000×700を200%拡大した相当の500×350 CSS pxで、10代表状態のcritical actionとfocus到達を自動検査する。 | 10/10状態、critical action 50/50、画面外focus 0、操作要素欠落0。jsdomで視覚寸法を実測できない項目は別記する。 |
| 9-C1 pilot（**実施保留**） | **外部1件**（2～3人日相当） | **terra:** 固定protocolどおり3名へ実施・記録する手順作業である。 | 各cohort 1名ずつ計3名でprotocol、計測、匿名化をpilotする。課題/成功定義を変えなければ15名へ算入し、変えた場合は3名とも無効として補充する。 | 有効0～3名、15試行、介入、欠測、protocol変更有無、続行/再pilot判断。 |
| 9-C2 初心者wave（**実施保留**） | **外部1件**（1～2人日相当） | **terra:** 同一build・課題で残り4名へ実施・集計する手順作業である。 | pilot後の初心者4名を同一build/課題で実施する。 | 初心者累計5/5、25試行、成功、時間、誤操作、質問、行止まり。 |
| 9-C3 CP経験者wave（**実施保留**） | **外部1件**（1～2人日相当） | **terra:** 同一build・課題で残り4名へ実施・集計する手順作業である。 | pilot後のCP経験者4名を実施する。 | CP経験者累計5/5、25試行、同じ6指標。 |
| 9-C4 設計経験者wave（**実施保留**） | **外部1件**（1～2人日相当） | **terra:** 同一build・課題で残り4名へ実施・集計する手順作業である。 | pilot後の設計経験者4名を実施する。 | 設計経験者累計5/5、25試行、同じ6指標。 |
| 9-C5 集計/triage（**実施保留**） | **1件**（2～3人日） | **sol ultra:** 重大問題のseverity、原因、再現、再確認対象を決める判断が要る。 | 有効15名・75試行を匿名集計し、重大問題のseverity、再現、再確認対象を決める。 | 15/15、75/75、cohort別/課題別結果、欠測、重大問題、再確認待ち。 |

現在着手してよいのは9-A、9-B1、9-B2の3単位（6～9人日）だけである。9-C1～C5の5単位（7～12人日相当）は、利用者がsession日程を決めるまで開始しない。

### 13.5 数値の合格条件

1. **承認済み準備:** `test-plan.md`、課題文と観察手順を持つ`test-script.md`、`consent.md`、`session-schema.json`、0/15の`results.json`の5/5を作り、未記入の必須節0、schema fixture 10/10合格とする。
2. **承認済み準備:** keyboard-only自動検査は5課題の5/5経路で合格し、pointer入力0、focus到達不能0、modalを閉じた後のfocus復帰5/5とする。
3. **承認済み準備:** 1000×700を200%拡大した相当の500×350 CSS pxで10/10代表状態、critical action 50/50、画面外focus 0、操作要素欠落0とする。jsdomで実寸・clipを算出できない表示は「自動検査対象外」と列挙し、将来の許可された表示検査へ残す。
4. **点数gate:** 準備が1～3を満たしても実施済みsessionは0/15のまま記録し、UI加点0、NFR加点0とする。次の5～12は実施保留中は「未実施」であって「合格」ではない。
5. **将来の実施条件:** 初心者5、CP経験者5、設計経験者5の独立15名。欠席・無効sessionは補充し、有効15名未満で完了としない。
6. 5課題×15名=75試行を記録し、全体初回成功68/75以上（90.67%以上）。さらに各課題14/15以上（93.33%以上）とする。
7. 重大な行き止まり0。重大とは、facilitatorの操作介入、app再起動、作品消失、別課題へ継続不能のいずれかと定義する。
8. 専門語に起因する質問、操作中断、別画面への迷走を合計0件とする。停止時間に閾値を置かず、1件でも主条件は不合格とする。30秒以上の停止件数と最長停止秒は副指標として別に記録し、質問がなかった29秒停止を合格へ隠さない。用語起因かは観察発話、観察record、直後質問のいずれかで判定する。
9. 参加者/課題ごとに完了時間、Undo回数、同一操作3回以上の反復、error/warning、Help表示、質問回数を75/75記録する。欠測0。
10. 個人名、メール、音声、作品本文、CP座標を成果JSONへ保存する件数0。匿名session IDのみ。
11. 自動検査だけで15名条件を合格扱いにする件数0。
12. 発見した重大問題は全件severityと再現手順を持ち、修正後に影響cohort最低3名で再確認する。未再確認の重大問題1件以上なら点数を上げない。

### 13.6 変更・緩和してはいけないもの

- 15名を15自動run、同じ人の反復、開発者、Codexで代用しない。
- 90%の分母を成功した課題だけに減らさない。75試行と課題別15を両方示す。
- facilitatorが答えを教えた試行を初回成功に数えない。
- keyboard-onlyと200%拡大の自動検査を、人の理解、初回成功、15名sessionとして数えない。
- 初心者向けに機能を削るのではなく、4区画、直接操作、理由表示を維持して改善する。
- 参加者結果を実装者の印象だけで要約せず、匿名recordを残す。

### 13.7 過去の失敗と原因

- `CLAUDE.md:147-158`: 出力fileの生成を内容の正しさと誤認した。DOM testも「初見で理解できる」証拠ではない。
- `CLAUDE.md:371-397`: 表示確認なしに次へ進んだ。実利用者課題も各sessionの観察recordなしに成功としない。
- `docs/progress.md:29`: 100画面状態で9件見つかったが、これは機械的状態検査で人の理解とは別の証拠である。
- 現行要件の対象利用者は開発者自身（`docs/requirements-definition.md:37`）で、外部15名は新しい証拠範囲である。

### 13.8 道具、コマンド、確認方法

この機械で可能な補助検査だけを示す。人対象部分にはshell commandはない。

```powershell
$env:CARGO_TARGET_DIR = "C:\Users\oltot\AppData\Local\Temp\ori3-target-codexroadmap"
npm --prefix apps/desktop test -- src/App.dom.test.tsx src/lib/allScreenScenarios.test.ts
npm --prefix apps/desktop test -- src/components/dialogs/ModalDialog.dom.test.tsx
```

9-B2はブラウザを起動せず、1000×700 physical pxを200%拡大した相当の500×350 CSS pxをDOM環境へ設定する。10状態ごとに5 critical actionをqueryしてfocus・keyboard発火まで確認し、50件のstate ID、action ID、存在、focus、発火結果を`verification/improvement-roadmap/09-usability/results.json`の`automated_zoom_200`へ保存する。jsdomが実際のbox寸法やclipを算出しないことを明記し、これは将来の表示目視や15名sessionの代替ではない。

外部sessionは利用者が許可したstudy端末のrelease installerを使い、build IDとscreen sizeをrecordする。本書作成中はappを起動していない。匿名集計は `verification/improvement-roadmap/09-usability/results.json`、人向け要約は `docs/progress.md`。

### 13.9 成果物、見積、依存、リスク、点数

- **保存先:** `verification/improvement-roadmap/09-usability/test-plan.md`、`verification/improvement-roadmap/09-usability/test-script.md`、`verification/improvement-roadmap/09-usability/consent.md`、`verification/improvement-roadmap/09-usability/session-schema.json`、`verification/improvement-roadmap/09-usability/results.json`、`apps/desktop/src/lib/allScreenScenarios.ts`、`apps/desktop/src/lib/allScreenScenarios.test.ts`、`apps/desktop/src/App.dom.test.tsx`、`apps/desktop/src/components/dialogs/ModalDialog.dom.test.tsx`、実施後だけ`docs/progress.md`。
- **見積:** 全体は13～21人日相当だが、**今回承認された準備は6～9人日**、実施保留部分は7～12人日相当である。現行automation 3ファイル・合計906行、新規protocol/consent/schema/result 5ファイルを対象にし、keyboard 5経路、200%の10状態、privacy schema、pilot 3名、残り3 wave各4名、75試行集計の6検査群を8単位で閉じる。段階和は9-A 2～3、9-B1/B2 4～6、9-C1～C5 7～12人日である。募集待ちの暦日と謝礼は含まない。
- **依存:** 9-A/B1/B2の準備は承認済みで並行開始できる。9-C1～C5の実施は施策5、4、6完了と、利用者による日程決定の両方が必要である。
- **リスク:** 募集できないこと。有効15名を確保できなければ「未実施」とし、自動検査で置換しない。
- **リスク:** cohort間で課題難度が違うこと。同一build、初期file、文言、時間起点を使い、順序だけcounterbalanceする。
- **点数:** 実利用者15/15完了後にUI +0.5、NFR +0.5（レビュー記載）。**準備だけ、またはsession 0/15の現在はUI +0、NFR +0。**

## 14. 施策10 supply-chainと配布を保守する

### 14.1 目的と現在地

現在の設定には継続的依存更新、`cargo audit`、npm audit方針、CodeQL、SBOM、license allowlistが見当たらない。releaseはsetup.exe、x64.msi、portable.exe、取扱説明書PDFの4成果物を公開する（`.github/workflows/release.yml:224-228`）。2026-08-24の利用者承認に従い、本施策は次の5項目だけを対象にする。

1. 脆弱性の自動監視。
2. Cargo、npm、GitHub Actionsの依存更新提案。自動取込みは行わない。
3. コードの静的解析。
4. 4配布物のSBOMとSHA-256の公開。
5. license allowlist。

**アプリの自動更新機構は範囲外である。** updater plugin、endpoint、公開鍵/署名鍵設定、更新download・適用・rollback処理を本施策へ追加しない。これは判断待ちや条件付きN/Aではなく、利用者が採らないと決めた範囲である。

### 14.2 対象ファイルと関数

| 実パス | 実関数・領域 | 作業 |
|---|---|---|
| `.github/dependabot.yml`（新規候補） | 関数名は**該当なし**。依存更新設定のため。 | Cargo/npm/GitHub Actionsの更新頻度、group、同時PR。Renovateと併用しない。 |
| `.github/workflows/security.yml`（新規） | 関数名は**該当なし**。YAML workflowのため。 | audit、license、CodeQL、SBOM検証。 |
| `.github/workflows/release.yml` | 関数名は**該当なし**。YAML workflowであり関数を持たない。対象は `release-windows` jobと成果物収集/公開（`:18,123-228`）。 | 4成果物ごとのSBOM、hash manifest、sidecarを公開する。 |
| `.github/workflows/ci.yml` | 関数名は**該当なし**。YAML workflowであり関数を持たない。対象は `checks` / `performance` job。 | 軽量security gateを通常CIへ置く。性能jobを置換しない。 |
| `Cargo.toml` / `Cargo.lock` | 関数名は**該当なし**。Rust manifest/lockfileのため。 | audit/SBOM入力。変更は承認後。 |
| `apps/desktop/package.json` / `apps/desktop/package-lock.json` | 関数名は**該当なし**。npm manifest/lockfileのため。 | production/dev依存を分けて監査する。 |
| `.github/security-policy.json`（新規候補） | 関数名は**該当なし**。機械可読policyのため。 | severity、例外owner/reason/expiry、license allowlist。 |
| `scripts/check-supply-chain.ps1`（新規） | 計画する `Test-Advisories`、`Test-Licenses`、`Test-Exceptions`、`Test-ArtifactSbom` | local/CIで同じpolicyを実行する。 |

CodeQLは実施時点の正式対応言語を確認し、少なくともJavaScript/TypeScriptを対象にする。Rust対応を確認できない場合は、対応していると推測せず `cargo clippy`、`cargo audit`、別の承認済みSASTで補う。

### 14.3 段階と6個の委譲単位

| 段階 | 委譲 | 担当モデル（理由） | 作業 | 中間報告 |
|---|---|---|---|---|
| 10-A policy/license | 1件（2～3人日） | **sol ultra:** license allowlist、severity、期限付き例外、production/dev区分、頻度を決めるpolicy判断が要る。 | allow/deny license、advisory severity、例外schema、監視頻度、tool/version pinを決める。 | allow/deny一覧、例外field/期限、production/dev境界、頻度、未決事項0。 |
| 10-B1 脆弱性監視 | 1件（1～2人日） | **terra:** Cargo/npmの監視を10-Aのpolicyどおり定期実行・通知するworkflow設定である。 | Cargo/npm advisoryをscheduleとPR/pushで検査し、期限付き例外だけを許す。 | 実行契機、2 ecosystem結果、通知先、未説明critical/high、所要時間。 |
| 10-B2 依存更新提案 | 1件（1～2人日） | **terra:** 選んだbotで更新提案だけを作り、自動取込み0を検査する設定作業である。 | DependabotかRenovateの一方をCargo/npm/GitHub Actionsへ設定し、merge/applyは必ず人のreview後にする。 | 3 ecosystem、提案作成3/3、自動merge/apply 0、二重bot 0。 |
| 10-C 静的解析 | 1件（2～3人日） | **terra:** 10-AでpinしたCodeQL/SAST queryを実行・公開する機械設定で、例外をこの単位で勝手に決めない。 | 正式対応言語のworkflow、結果upload、既存lint/clippyとの重複整理。 | 言語、query suite、検出件数、10-A例外との照合、floating reference 0。 |
| 10-D SBOM公開 | 1件（2～3人日） | **terra:** 4配布物へpin済みtoolでSBOMを生成しasset公開・対応表を集計する作業である。 | 4成果物ごとに同じpin済みtoolでSBOMを生成し、artifact名/version/build IDへ結ぶ。 | 4×SBOM対応表、component数、tool/version、欠落、公開asset 4/4。 |
| 10-E hash公開 | 1件（3～4人日） | **terra:** 4配布物のSHA-256 sidecar/manifestを生成・公開しdownload後に再照合する数値作業である。 | 4 SHA-256 sidecarと1 manifestをrelease workflowへ結び、download後の再計算を検査する。 | 4×hash、asset名、version/build ID、download照合、公開asset 4/4。 |

6単位を一括にしない。SBOMとhashも、生成物・失敗位置・公開照合が異なるため別委譲にする。自動更新の委譲は0件である。

### 14.4 数値の合格条件

1. setup.exe、x64.msi、portable.exe、取扱説明書PDFの4/4にCycloneDXまたはSPDX SBOMとSHA-256を1対1で公開し、manifest欠落0。
2. download後に4/4のSHA-256を再計算し、sidecar/manifest一致4/4。artifact名・version・build IDの不一致0。
3. production依存のcritical/high未解決0。dev/build依存もcritical/high未説明0。
4. 例外は100% owner、advisory ID、理由、影響範囲、作成日、期限を持ち、期限は承認日から最大90日、期限切れ例外0。
5. license allowlistにないlicense 0、deny license 0、UNKNOWN 0。dual licenseは実際に採用する側を記録する。
6. DependabotまたはRenovateをCargo、npm、GitHub Actionsの3 ecosystemで有効化し、同じecosystemへの二重bot 0。
7. 依存更新は3 ecosystemすべてで提案作成までとし、自動merge、自動approve、自動lockfile取込み、自動releaseを合計0件、review経由100%とする。
8. 脆弱性監視はCargo/npmの2/2 ecosystemで少なくとも週1回とPR/push時に自動実行し、連続4週のschedule欠落0、失敗通知欠落0。
9. CodeQL/SASTの対象にJavaScript/TypeScript 1言語以上を含み、検出critical/high 0または上記期限付き例外100%。Rustの静的解析は既存`cargo clippy`を維持し、追加SASTの正式対応可否を明記する。
10. security workflowのaction/tool referenceを100% immutable versionまたは承認済みpinで固定し、unreviewed floating `@main` 0。
11. updater plugin、更新endpoint、更新公開鍵/署名鍵、更新download/apply/rollbackコードの追加を全て0件とする。

### 14.5 変更・緩和してはいけないもの

- 既存 `checks`、`performance`、Windows package、release-ready検査。security jobで置換しない。
- critical/highをseverity変更やdev分類だけで隠さない。
- 例外の無期限化、owner/reasonなし、期限更新だけの反復。
- SBOM生成action自体のversion pinとhash検証。
- `Cargo.toml`、`Cargo.lock`、`vendor/`、package lockを承認前に変更しない。
- アプリの自動更新機構、updater plugin、endpoint、更新鍵、更新適用コードを追加しない。
- Windows 10/11正式対象と4release成果物。

### 14.6 過去の失敗と原因

- `CLAUDE.md:67-68`: 依存patch変更で起動crash、build途中の強制終了で実行file破損が起きた。更新PRでもpackage起動/成果物検査を省かない。
- `CLAUDE.md:160-195`（§10.6）: CIの片方だけを見てperformance jobを落とした。security追加後も全job表を正本にする。
- `CLAUDE.md:398-459`（§10.7.11～13）: target directoryが39個230 GB、その後53個179 GBまで増えdiskを圧迫した。security/SBOM buildも指定一時targetを使い、`verification/`へbuildしない。
- `docs/progress.md:984-989`: toolchainがSmart App Controlで拒否され再導入が必要になった。toolの入手元、version、checksumを記録する。

### 14.7 道具、コマンド、確認方法

次はtool導入とnetworkの承認後に使う候補であり、今回は実行していない。

```powershell
$env:CARGO_TARGET_DIR = "C:\Users\oltot\AppData\Local\Temp\ori3-target-codexroadmap"
cargo audit
npm --prefix apps/desktop audit --omit=dev
npm --prefix apps/desktop audit
powershell -ExecutionPolicy Bypass -File scripts/check-supply-chain.ps1
```

SBOM toolは10-Aで1つに決めversion pinし、複数toolを無根拠に追加しない。結果は `verification/improvement-roadmap/10-supply-chain/` の `audit.json`、`licenses.json`、`artifacts.json`。release時は4 SBOMとhash sidecarをrelease assetsへ置く。

### 14.8 成果物、見積、依存、リスク、点数

- **保存先:** 上表のconfig/workflow/script、`verification/improvement-roadmap/10-supply-chain/`、release assetの4 SBOM・4 hash sidecar・1 manifest、要約を `docs/progress.md`。
- **見積:** **承認された5項目へ11～17人日。** 現行6 manifest/workflowファイル・合計10,588行と、新規候補4ファイル（`.github/dependabot.yml`、`.github/workflows/security.yml`、`.github/security-policy.json`、`scripts/check-supply-chain.ps1`）を対象にし、2 ecosystem脆弱性監視、3 ecosystem更新提案、SAST、4 SBOM、4 hash再計算、license/例外の6検査群を6委譲で閉じる。段階和は10-A 2～3、10-B1 1～2、10-B2 1～2、10-C 2～3、10-D 2～3、10-E 3～4人日である。自動更新の工数は0であり見積へ含めない。
- **依存:** policyは施策7後に並行可。最終SBOM/hashは施策6とrelease成果物安定後。承認済み範囲内でも、具体的な外部tool、network取得、依存追加は導入前に差分とpinを提示する。
- **リスク:** security tool自身が供給網を増やすこと。action/toolをpinできない場合は導入を止め、代替を比較する。
- **リスク:** advisoryでCIが長期停止すること。無期限skipではなく最大90日の例外と修正ownerを作る。
- **点数:** 承認された5項目を全て公開・運用できた後にTest/CI +1、SYS +0.5（レビュー記載）。自動更新は加点条件にしない。

## 15. 施策11 全部の折り目を一斉に折る一時表示

### 15.1 承認範囲と前提

実装候補は案Bだけである。山折りと谷折りの全有効ヒンジへ、共通の0～100%を希望として与え、つまみ1本で一時表示を動かす。これは通常の折り手順を生成する機能ではない。

- 案A（100%だけ）と案C（形を手順として残す）は不採用である。
- 重なり順の無い形を通常の手順と誤認させる危険があるため、形から通常手順を起こす機能は含めない。
- 一斉折りの割合、実角、診断、3D状態は、手順、作品保存、Undo/Redoのどれにも残さない。
- 常設4区画を増やさず、既存コンテキストパネル内で切り替える。画面には常に「これは記録された手順ではない」と示す。
- 不収束、希望角との差、紙の突き抜けを日本語で警告して操作を止めず、重なり順を確定したものとして表示しない。
- 実装の開始前に、§0B.3の要件定義§4.1追加案を正本へ反映する。§4.2の改訂は不要である。

### 15.2 実装順と他施策・編集中作業との衝突

§3.2の順序2に置く。設計文書が安全とする「施策2の画面側完了後」の位置であり、その画面側は完了済みである。施策4が未着手の間に、施策11を単独変更として完結する。

初版の主な変更候補は、apps/desktop/src/store/appStore.ts、apps/desktop/src/components/ContextPanel.tsx、apps/desktop/src/components/Timeline.tsx、apps/desktop/src/components/Viewer3D/Viewer3D.tsx、apps/desktop/src/components/Viewer3D/sceneBuilder.ts、apps/desktop/src/App.cssと、その専用の表示・検査ファイルである。

| 相手 | 衝突 | 扱い |
|---|---|---|
| 施策2 | 直接 | appStore.ts、ContextPanelとそのDOM検査が重なる。施策2の画面側完了後に始め、同じファイルを同時編集しない。 |
| 施策1 | 直接 | appStore.tsが重なる。一斉折りの単独変更を先に閉じ、提案のfrontend作業とは重ねない。 |
| 施策4 | 最大の直接衝突 | appStore.ts、ContextPanel.tsx、App.css、Viewer3D.tsx、sceneBuilder.tsが重なる。施策4が着手済みなら、分割後のpose/replay slice、ContextPanel子部品、viewer/style境界ができるまで待つ。 |
| 施策5・施策9 | 検査で条件付き | 全画面シナリオ、ContextPanel DOM、keyboard受入が重なる。一斉折りの状態を受入対象へ追加する。 |
| 施策6 | 3D表示で直接 | Viewer3D.tsxが重なる。App.tsxに入口を置かず、bundle分割とは同時編集しない。 |
| 現在の別担当 | 変更対象外だが共有ツリー上で進行中 | apps/desktop/src-tauri/src/commands.rsとcrates/ori3-propose/は別担当が編集中である。案B初版は既存のpose solveを再利用し、両方を変更しない。検査・画像の証拠は、これらの未確定変更を混ぜず、単独変更の作業ツリーで取る。 |

### 15.3 5個の委譲単位と担当モデル

| 段階 | 1回で終わる作業単位 | 担当モデル（理由） | 人日 |
|---|---|---|---:|
| 11-A | 山谷別の共通割合から希望角を作る純粋処理と単体検査 | terra（割合、符号、入力出力が確定している） | 1～2 |
| 11-B | 一時プレビュー状態、最新要求優先、通常手順への復帰、保存非対象の契約 | sol ultra（既存replay・一時姿勢・非永続化の正しい境界を決める） | 2～3 |
| 11-C | コンテキストパネル、つまみ、タイムラインの一時表示、キーボード導線 | terra（文言、配置、状態遷移、数値条件が確定している） | 1～2 |
| 11-D | 重なり順を確定と見せない3D表示、警告、表裏・輪郭の整合 | sol ultra（3D表示と重なり順の意味を保つ判断が要る） | 2～3 |
| 11-E | 保存・Undo/Redo・応答順・性能・画像の回帰検査 | terra（検査項目と合格数値が確定している） | 2 |
| **合計** | **5単位** | **terra 3、sol ultra 2** | **8～12** |

### 15.4 受入の要点

1. 山・谷の目標角は0/25/50/75/100%で符号誤り0とする。折り鶴、やっこさん、カエル、鳥の基本形の4標本を0～100%で掃引し、有限値、返却面欠落0、panic/IPC error 0とする。
2. 1秒120入力でも16ms間引き後のIPC呼出しは65回以下、同時実行最大1、最後の入力採用120/120とする。古い応答を最後の表示へ戻す件数は0とする。
3. 保存前後の正規化Documentは20/20で一致し、一斉折り専用fieldのJSON出現0、作品Undo・角度Undoの増分0とする。
4. 1000×700で横はみ出し0px、キーボードで0/100%と1%刻みを操作可能、設定キー増加0、Tauriコマンド増加0、常設区画増加0とする。
5. 一斉折り中の中立表示、表裏、輪郭、警告を、4標本×5割合の画像20/20で単独変更の作業ツリーから確認する。未実行を合格と書かない。

### 15.5 見積・依存・停止

見積は**8～12人日**であり、施策11として全体見積へ加える。既存IPCを使い、初版でcommands.rs、store.rs、types.ts、client.ts、ori3-model、ori3-rigidを変更しない前提である。既存のpose solveで必要な契約を満たせない、3D状態の保存または手順への記録が必要になる、重なり順を確定と見せずに受入を満たせない場合は、範囲を拡張せず利用者判断へ戻す。

## 16. 施策12 Linux対応を見送る

### 16.1 判断日・理由・再開条件

- **判断日:** 2026-08-24。
- **判断:** 実施しない（見送り）。工数は総見積へ加えない。
- **理由:** GUIを使えるLinux環境（実機またはVM）を用意できず、利用者の基準である「全ての機能の確認」を満たせないためである。技術的に不可能だからではない。
- **良い材料:** Windows専用API・レジストリ・Windows固定パス・Windows専用crateの直接依存は見つからず、保存はOS非依存APIを使う。Tauri 2はdeb / rpm / AppImageを正式bundle targetとして持つ。
- **未確認:** Linuxでのbuild、起動、3D表示、入力、保存、フォント、性能は1件も実行確認していない。
- **再開条件:** §0A.2に記録した5条件、すなわち要件の対象固定、専用施策と別工数、Linux runnerとGUI環境での実測、施策4・6および必要ならFOLD凍結後の認定、Linux成果物を含むSBOM/SHA-256の作り直しを全て満たす。
- **参考見積:** 案A（AppImage 1形式）は27～46人日、案B（deb + AppImage）は37～60人日。再開時に採用範囲と共に改めて判断する。
- **評価:** 対応環境3/10は据え置く。

## 17. 全体見積・依存関係・停止条件

### 17.1 最終の委譲数と人日

全製品ファイル走査と独立監査後の最終値である。段階2の最低61単位から、`ContextPanel.tsx`、Viewer/surface-order、F1/Export lazy前処理、Modal/axe、roadmap 182 checkbox、FOLD converter/corpus、15名wave、SBOM/hash、一斉折りの5単位を独立させ、合計111単位へ細分化した。

| 施策 | 段階 | 1回で終わる作業単位 | 人日 |
|---:|---:|---:|---:|
| 1 提案負荷非依存 | 5 | 5 | 11～16 |
| 2 原子的MoveStep | 3 | 3 | 5～8 |
| 3 30作品corpus | 4 | 8 | 18～26 |
| 4 巨大境界 | 5 | 36 | 56～82 |
| 5 Modal a11y | 5 | 6 | 9～15 |
| 6 bundle | 6 | 6 | 8～13 |
| 7 文書機械検証 | 4 | 13 | 12～23 |
| 8 FOLD 1.2 限定profile | 6 | 15 | 33～48 |
| 9 実利用者 | 3 | 8 | 13～21 |
| 10 supply-chain | 5 | 6 | 11～17 |
| 11 一斉折り案B | 5 | 5 | 8～12 |
| **合計（承認済み限定profile・一斉折り案B）** | **51** | **111** | **184～281** |

2026-08-24の決定により、FOLDは15単位・33～48人日の限定profileへ確定し、F/U完全往復の追加単位・追加人日は0である。一斉折り案Bは5単位・8～12人日へ確定し、案A・案C・通常手順の自動生成の追加単位・追加人日は0である。全体111単位・184～281人日は、次の版へ繰り越す施策9の外部実施5単位・7～12人日も含む。判断6により、**今版のリリースに必要な作業は106単位・177～269人日**とする。施策12のLinux対応見送りは0単位・0人日であり、いずれの総見積にも加えない。人日は実作業量で、参加者募集、要件正本反映待ち、CI待ち、外部reviewの暦日は含まない。

担当モデルは§4.4のとおり、全111単位で**terra 68件、sol ultra 43件**である。現在承認されている106単位では、保留5単位（terra 4、sol ultra 1）を除き、terra 64件、sol ultra 42件となる。

### 17.2 今版のリリース範囲・完了条件・見積

今版をリリースできるのは、次をすべて満たしたときである。

1. 施策1〜8、10、11の全103単位が各合格条件を満たす。
2. 施策9は準備3単位・6～9人日（課題文、同意文、集計様式、観察手順、keyboard-only/200%自動検査）を完了とみなす。
3. 施策9の外部実施5単位・7～12人日は、**次の版で必ず実施する繰越**とする。今版のリリース条件からは外すが、実施しないものにはしない。
4. 施策12（Linux）は見送りのまま、今版のリリース条件に含めない。

したがって、今版のリリースに必要な作業量は**106単位・177～269人日**である。次の版へ繰り越す作業量は**施策9の外部実施5単位・7～12人日**である。外部参加者15名が完了するまで、施策9によるUI・NFRの加点は0とする。

### 17.3 進捗の見える化（2026-08-24時点）

この表は今版のリリース範囲を母数とする。**以後、担当の完了報告ごとに、その施策の「完了した単位」と「状態」を同じ更新で直し、合計と完了率を再計算する。**

| 施策 | 見積(人日) | 単位数 | 完了した単位 | 状態 |
|---|---:|---:|---:|---|
| 1 提案負荷非依存 | 11～16 | 5 | 2 | 1-A・1-B完了、1-C実行中、1-D・1-E未着手 |
| 2 原子的MoveStep | 5～8 | 3 | 3 | **全3単位完了。** 実機確認済み、画像8枚を目視、手順の消失0件 |
| 3 30作品corpus | 18～26 | 8 | 0 | 未着手 |
| 4 巨大境界 | 56～82 | 36 | 0 | 未着手 |
| 5 Modal a11y | 9～15 | 6 | 0 | 未着手 |
| 6 bundle | 8～13 | 6 | 0 | 未着手 |
| 7 文書機械検証 | 12～23 | 13 | 0 | 未着手 |
| 8 FOLD 1.2 限定profile | 33～48 | 15 | 0 | 未着手 |
| 9 実利用者（今版の準備） | 6～9 | 3 | 0 | 未着手 |
| 10 supply-chain | 11～17 | 6 | 0 | 未着手 |
| 11 一斉折り案B | 8～12 | 5 | 0 | Rust側実行中、画面側未着手。完了単位はまだ0 |
| 12 Linux対応 | 0 | 0 | 0 | 見送り確定。リリース条件外 |
| **今版リリース合計** | **177～269** | **106** | **5** | **実行中** |
| 次の版へ繰越: 施策9の外部実施 | 7～12 | 5 | 0 | 次の版で必ず実施 |

単位ベースの完了率は**5 / 106 = 4.7%**である。人日ベースでは、完了した1-A・1-B（6～8人日）と施策2（5～8人日）の合計**11～16人日相当**を、今版177～269人日で割るため、**4.1～9.0%**（中点の参考値 **6.1%**）である。実行中の1-Cと施策11 Rust側は、完了報告が出るまで完了量へ加えない。

### 17.4 依存関係

| 施策 | 必須の先行 | 並行できるもの |
|---:|---|---|
| 2 | なし | FOLDの承認済み要件差分、施策9の承認済み準備、security policy |
| 11 | §0B.3の要件正本改訂、施策2の画面側完了。施策4が未着手なら単独完結、着手済みなら分割後のpose/replay・ContextPanel・viewer/style境界 | policy作成と文書改訂だけ。施策1のfrontend、施策4、施策6とは同じファイルを並行編集しない |
| 1 | なし（順序は2の後） | security policy |
| 7 | 1、2の契約確定 | 課題文・同意文・集計様式・観察手順・keyboard-only/200%自動検査の準備 |
| 3 | 1 | security audit準備 |
| 5 | 2 | corpusのfixture batch |
| 4 | 2、3、5 | surface-order分割はstore分割と別fileなら並行可 |
| 6 | 4、5 | release SBOM設計 |
| 10 | 7、最終成果物は6 | 4/5の実装とpolicy/CodeQL |
| 9 | 準備はなし。外部実施は5、4、6と利用者の日程決定 | 承認済みの課題・同意・集計・観察・自動検査準備 |
| 8 | 要件正本改訂、1、3。実装推奨は4/6後 | corpus取得/license確認 |
| 12 | 見送り。§0A.2の再開条件が全て満たされるまで着手しない | なし |

### 17.5 全体停止条件

次のいずれかなら、自動的に次へ進まず利用者へ判断を返す。

1. §0B.2/§12.2に記録した承認済みFOLD要件改訂を超えて、要件§2、§4.2、NFR-006を変えないと合格できない。
2. `Cargo.toml`、`Cargo.lock`、`vendor/`、外部依存、network、署名secretの新しい承認が必要。
3. 既存検査の削除、ignore追加、epsilon緩和、候補/機能/画面削減を提案しないと数値を通せない。
4. 限定profile内と判定したFOLDの`faceOrders`やstep frameが現行modelへ損失なく対応しない、または承認範囲の拡張が必要になる。
5. 次の版へ繰り越した施策9の外部実施で、実利用者が15名に達しない、同意が取れない、privacy条件を満たせない。これは今版のリリース停止条件ではなく、次の版での施策9完了を止める条件である。
6. performance値が機械情報なし、製品と違う経路、1回測定しかない。
7. 同じ巨大ファイルを2委譲が同時に変更する必要が生じる。
8. 一斉折り案Bが、手順への記録、保存、Undo/Redo対象化、常設5区画目、または重なり順を確定と見せる表示なしには成立しない。

### 17.6 全体完了の定義

- 改善ロードマップ全体（今版と次の版を合わせた完了）は、各施策の数値条件が全て合格し、未実行0となったときである。施策9は次の版で外部参加者15名を完了するまで、ロードマップ全体の完了にしない。ただし今版のリリース条件は§17.2に定めた準備完了までとする。施策11は案Bの一時表示をN/Aにせず、手順非記録・非保存・非Undoと画面表示を含めて合格させる。FOLD 1.2 限定profileは採用済みでN/Aにせず、自動更新は施策10の初めから範囲外なのでN/A項目を作らない。施策12は見送り中のため完了対象に数えない。
- 既存5検査と通常/性能/package/security jobが全て合格。
- 変更・緩和禁止の違反0、削除検査0、追加ignore 0。
- `docs/requirements-definition.md`、`docs/implementation-roadmap.md`、`docs/progress.md` と生成statusの不一致0。
- 12分野を同じ尺度で再採点し、推測点を事実点へ置き換える。

## 18. 実施時コマンドの共通前置き

本書作成中は検査を実行していない。将来Rustを組み立てる各PowerShell sessionでは、最初に必ず次を設定する。

```powershell
$env:CARGO_TARGET_DIR = "C:\Users\oltot\AppData\Local\Temp\ori3-target-codexroadmap"
```

確認規約:

- `verification/` をCargo target、npm cache、Tauri bundleの組み立て先にしない。ここには小さいJSON/Markdown証拠だけを置く。
- build/testの前後で `$env:CARGO_TARGET_DIR` の実値と対象pathを中間報告に記す。
- 一時targetの片付けは別の明示許可された作業にし、未確認pathへ再帰削除を行わない。
- npm testはjsdom/Vitestを優先し、利用者の画面へブラウザ窓を出す自動化を既定にしない。
- release性能は `--release`、通常正当性は通常profileというCI表の役割を守る。
- コマンド成功だけで「完成」とせず、各節の数値出力、成果物、非緩和事項を同時に確認する。

---

本書は0.5.0の現行ソースを起点にした実行計画である。0.4.5レビューの解決済み3件を再作業へ戻さず、残る大域progressと壁時計結果経路、原子的MoveStep、一般化証拠、保守境界、利用者・配布証拠を順に閉じる。2026-08-24の利用者決定により、FOLD 1.2 限定profileと供給網の監視・公開は承認済み、実利用者検証は準備のみ承認済みであり、自動更新とF/U完全往復は範囲外である。
