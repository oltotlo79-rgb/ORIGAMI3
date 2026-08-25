# B2 検査範囲の補完仕様

対象は、進捗の完了記録に結ばれた既存検査が細目全体を直接立証していない13件である。これは実装計画であり、ここに書く「追加」はまだ行わない。実在しない検査名を証拠として扱わないため、新設する検査名は固定せず、配置先と確認内容だけを定める。

## 数値条件の扱い

ここでの数値は、件数・頁数・時刻・有無といった離散的な製品契約である。座標差、画像差、実行時間など実測が必要なしきい値は、実装時に同一入力を3回測定し、良い側の最小実測値の80%以下（上限型なら80%）又は悪い側の最大実測値の125%以上（下限型なら80%）へ余裕を取る。単発の実測値をそのまま境目にしない。

## 13件の仕様

| ID | 現在の検査が確かめること | 確かめていないこと | 追加又は再接続する検査と実パス | 合格条件 |
|---|---|---|---|---|
| `M2.T2-6b.C04` | `sim011_completeness_table_and_generic_routes_are_permanent` は名前付き技法が汎用経路へ到達することを確認する。 | 鶴の基本形の前面を `squash` と `petal` が実際に持ち上げる事例。 | `crates/ori3-layers/tests/sim011_completeness.rs` に、鶴の基本形から各技法を適用し、前面の対象面・折り線・再生結果を比べる検査を追加する。 | `squash` と `petal` の2/2が成功、各1面以上の前面頂点が開始位置と異なる、再生結果2/2一致、未定義入力以外の`Err` 0。 |
| `M2.T2-6c.C06` | `apps/desktop/src/lib/layerMotion.test.ts` の「既存折り目のReflectをregionなし・Keepへ変換する」は入力変換だけを確認する。 | 従来のパネル確定を残しつつ、通常操作の主経路から外したこと。 | `apps/desktop/src/components/ContextPanel.dom.test.tsx` と `apps/desktop/src/App.dom.test.tsx` に、主操作と詳細指定の到達経路を分けるDOM検査を追加する。必要なら状態の組立てを `apps/desktop/src/store/appStore.test.ts` に追加する。 | 通常操作3経路でパネル確定なしに記録済み手順が3/3、詳細指定では旧パネルが1/1で利用可能、両経路とも手順記録1件以上、例外0。 |
| `M2.T2-8.C01` | `autosave_skips_clean_document_and_writes_untitled_to_app_data` はdirty時だけの書込み、`clean_exit_discards_but_dirty_exit_keeps_the_autosave` は保存済み終了時の削除を確認する。 | 30秒周期を時計で確認していない。 | `apps/desktop/src-tauri/src/autosave.rs` の既存test moduleへ、注入時計又はタイマーの境界を通す検査を追加する。 | dirty作品は29,999msまで書込み0、30,000msで書込み1、clean作品は30,000ms後も書込み0、保存済み正常終了後のautosave/markerは0件。 |
| `M3.T3-3.C01` | `depth_three_branching_skeleton_packs_and_generates_valid_cp` は深さ3入力のpack/generate/validateを確認する。 | 葉4+胴1、面抽出、軸線・稜線、違反数返却を細目どおり一つの入力で結んでいない。 | `crates/ori3-propose/tests/generate.rs` に、葉4・胴1の固定fixtureから生成する検査を追加する。 | 入力node 5、`extract_faces`成功1/1、軸線1本以上・稜線1本以上、違反数の返却1値、生成候補1件以上、検証エラー0。 |
| `M3.T3-3.C02` | 同じ既存検査はCPを生成できることを確認する。 | ドロネー分割、ウサギ耳分子、扇状分割、山谷既定則、`ProposalResult`の組合せ。 | `crates/ori3-propose/tests/generate.rs` に、三角形・ウサギ耳・4辺以上扇状の3固定入力を追加する。 | 入力3/3でCP生成・検証成功、各入力で`ProposalResult` 1件以上、軸線の谷割当1本以上、稜線の山割当1本以上、違反数欠落0。 |
| `M3.T3-3.C03` | `apps/desktop/src-tauri/src/commands.rs` の実在検査 `proposal_generate_returns_candidates` は `proposal_generate(star(4), ...)` を直接呼び、候補が1～4件で、各候補にCPの折り線があることを確認済み。 | 現在の証拠リンクがこの検査でなく、ライブラリ側の生成検査を指している。振る舞いの未検査ではない。 | 新設不要。`docs/traceability/roadmap-links.json` と `.md` の当該IDを、上記実在検査へ再接続する。Tauri JSON境界まで別途保証したい場合だけ、同じ `apps/desktop/src-tauri/src/commands.rs` のtest moduleへ往復検査を追加する。 | 既存検査の候補数 `1 <= n <= 4`、候補ごとの`scale > 0`、折り線数 `> 4`、失敗0。 |
| `M4.T4-3.C01` | `cp_svg::tests::viewbox_is_paper_size_in_mm`、`each_edge_kind_has_its_own_style`、`cp_png::tests::png_matches_requested_long_side` が、実寸viewBox、3線種、指定PNG寸法を別々に確認している。 | 現在の証拠リンクが線種検査1本だけで、他の2条件を同じ細目へ結んでいない。 | 新設不要。複数証拠を持てる台帳形式へ拡張して、`crates/ori3-export/src/cp_svg.rs` の2検査と `crates/ori3-export/src/cp_png.rs` の1検査を同じ細目へ接続する。台帳が単一名だけなら3条件をまとめる回帰検査を同2ファイルへ追加する。 | viewBox `150×100mm` 1/1、山・谷・輪郭のスタイル3/3、要求256pxに対してPNG `256×171px` 1/1、PNG非空（`>100` byte）。 |
| `M4.T4-4.C01` | `manual::tests::representative_json_makes_four_page_pdf_and_two_toc_items` は取扱説明書PDFの頁数と目次を確認する。 | 3手順の折り図それぞれに、正射影、可視輪郭・折線、今回の折線、技法別矢印があること。 | `crates/ori3-export/src/diagram.rs` のtest moduleへ、3手順fixtureの `render_step` 出力を調べる検査を追加する。 | SVG 3/3が生成、各SVGで可視輪郭1本以上・可視折線1本以上・今回の折線1本以上・矢印1本以上、空SVG 0。 |
| `M4.T4-4.C02` | 既存の矢印検査は一般の矢印数を確認する。 | `render_step`、最上層からの描画、10種の技法から固定矢印パスへの対応表全体。 | `crates/ori3-export/src/diagram.rs` に、10 `TechniqueKind` を通すtable-driven検査を追加する。 | 10/10で`render_step`成功、各SVGに対応する矢印パス1個以上、最上層を隠す塗りつぶし0、矢印数は各 `1..=6`。 |
| `M4.T4-5.C01` | `seven_steps_make_a_cover_and_two_pages` は7手順を表紙+手順2頁へ並べ、6+1コマを確認する。`pdf_has_one_a4_page_per_svg_page` はPDF化とA4 3頁を確認する。 | SVG版をページ別の成果物として書出す経路との結合を台帳で示していない。 | まず上記2検査を細目へ再接続する。必要なら `crates/ori3-export/src/pdf.rs` へ、SVGページ群のファイル名・頁順を検査する1件を追加する。 | 7手順で手順頁2、コマ `6+1`、表紙を含む総頁3、各頁 `210×297mm`、PDF先頭`%PDF-`、PDF `>1000` byte、SVG頁3/3。 |
| `M4.T4-5.C02` | 同PDF検査はA4と表紙の存在を確認する。 | `svg2pdf`変換後の余白と表紙レイアウトの下限。 | `crates/ori3-export/src/pdf.rs` のtest moduleで、既存の`LEFT=19mm`、`TOP=22mm`、`GAP=6mm`を出力SVGへ照合し、同じ頁群をPDF化する検査を追加する。 | A4 `210×297mm` 3/3、左右余白19mm、上余白22mm、列間隔6mm、表紙題1件・完成図1件、PDFのMediaBox 3件、変換エラー0。 |
| `M4.T4-6.C01` | `the_frog_is_deterministic` は同じカエルが2回で同一になることを確認する。 | 花弁・中割り・段折りを含む伝承カエルの最終層数と手順内訳。 | `crates/ori3-layers/tests/acceptance_frog.rs` に、既存の`frog()`構築結果を直接調べる受入検査を追加する。 | 花弁・中割り・段折り各1手以上、最終層数1以上、再生2/2一致、連結違反0、決定性2/2一致。 |
| `M5.T5-1.C01` | `finish_soft_round_trips_three_values_only_with_measured_tolerance` は3値の保存往復を、`finish_soft_replay_uses_the_latest_completed_pose_at_each_position` は手順位置での選択を確認する。 | 頂点座標を保存しないことと、風船・折り鶴でのSIM-015受入を一体で確認していない。 | `crates/ori3-model/tests/serde_roundtrip.rs` に保存JSONの座標非保存検査、`crates/ori3-layers/tests/acceptance_crane.rs` に鶴、**新規** `crates/ori3-layers/tests/acceptance_balloon.rs` に風船の受入検査を追加する。 | 設定値3/3往復、保存JSON内の仕上げ頂点座標0件、風船・鶴2/2で仕上げon/offを再生、各3回の位置差は前記80%余裕を取った許容値以内、層すり抜け0。 |

## 実装順

1. 既存検査だけで細目を満たす `M3.T3-3.C03`、`M4.T4-3.C01`、`M4.T4-5.C01` は、検査を重複追加せず証拠を複数本へ接続できる台帳形式を決める。
2. その後、同じcrate内で完結する M2 autosave、M3 generator、M4 diagram/PDF、M4 frog、M5 serde/受入を、製品コードが空いた順に追加する。
3. UIをまたぐ `M2.T2-6c.C06` は、`ContextPanel.tsx` だけでは完結しないため、App・store・ContextPanelの担当が同時に空いた時に扱う。
