# 手動受入手順

この文書のIDは `roadmap-links.json` の手動証拠と1対1で対応する。実施者はID、日付、結果、確認した画面又は履歴を記録する。担当者はアプリを起動せず、画面確認は統括が同梱版で行う。

- ロードマップSHA-256: `fec483aee8b989489d3e25af7606a2b6c493465009cc59c73ce543b9ecc384f5`
- 検査名台帳SHA-256: `0dc1c0edf7347695a1d56211c59c56bd72fdb4dd9273f6a073f5ff9dc29251d9`（roadmap-mapped 61/61件、source 38/38ファイル、definition tree `32a80feacd71df87b47127d2604788d553ecc2985b3eae1887c58aba71022191`、リポジトリ全検査数は主張しない）

## B1未実施受入の自動化可否（2026-08-26）

XはCDPで画面操作、表示文字列、画素又は領域の有無を確認できる。Yは検査が主張する範囲を人が読んで判断する。Z（実際に紙を折る比較が必要な項目）は、この16件にはない。

| ID | 区分 | 確認対象 |
|---|---|---|
| `MANUAL.M2.T2-6b.C05.SCREEN-ACCEPTANCE` | X | つまんで動かす操作とツールレール |
| `MANUAL.M2.T2-6b.C06.SCREEN-ACCEPTANCE` | X | 技法サブメニュー9種 |
| `MANUAL.M2.T2-6c.C01.SCREEN-ACCEPTANCE` | X | 層のずらし表示 |
| `MANUAL.M2.T2-6c.C02.SCREEN-ACCEPTANCE` | X | つかんで動かす操作 |
| `MANUAL.M2.T2-6c.C03.SCREEN-ACCEPTANCE` | X | 実行前プレビュー |
| `MANUAL.M2.T2-6c.C04.SCREEN-ACCEPTANCE` | X | 状態と操作理由の表示 |
| `MANUAL.M2.T2-6c.C05.SCREEN-ACCEPTANCE` | X | 技法の自動判定と記録 |
| `MANUAL.M2.T2-6c.C07.SCREEN-ACCEPTANCE` | Y | DOM検査基盤と主要経路の検査 |
| `MANUAL.M2.T2-7.C01.SCREEN-ACCEPTANCE` | X | 4種類の作図補助 |
| `MANUAL.M2.T2-7.C02.SCREEN-ACCEPTANCE` | X | 局所平坦違反の橙表示 |
| `MANUAL.M2.T2-7.C03.SCREEN-ACCEPTANCE` | X | めり込み警告バッジ |
| `MANUAL.M3.T3-4.C01.SCREEN-ACCEPTANCE` | X | 提案ウィザード3画面 |
| `MANUAL.M3.T3-4.C02.SCREEN-ACCEPTANCE` | X | 提案ウィザードの起動位置 |
| `MANUAL.M4.T4-3.C02.SCREEN-ACCEPTANCE` | X | 展開図書き出しダイアログ |
| `MANUAL.M4.T4-5.C03.SCREEN-ACCEPTANCE` | X | 手順図書き出しダイアログ |
| `MANUAL.ADDITIONAL.FOLD-ALL.C02.SCREEN-ACCEPTANCE` | X | 一斉折りの仮表示の速さ(NFR-002) |

### X: CDP自動化の共通手順

1. 専用の検査環境で同梱版を1つだけ起動し、CDP接続後に該当IDの操作を再現する。
2. 指定された文字列、要素領域、状態ごとのスクリーンショットを取得し、期待する画素領域又は文字列と比較する。
3. ID、操作列、取得画像、比較結果を保存する。1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

### Y: 人が判断する手順

#### `MANUAL.M2.T2-6c.C07.SCREEN-ACCEPTANCE`
1. 担当: 画面検査の担当者とは別のレビュー担当者。
2. `apps/desktop/src/lib/layerMotion.test.ts` とテスト設定を読み、jsdomとTesting Libraryの基盤、およびプレビュー・ヒント・ドラッグの主要経路を検査する実在testがあることを確認する。
3. 担当者が指定する検査名一覧の取得又は対象test実行の結果を確認し、ID、確認日、確認したtest名、結果を記録する。画面上の見た目だけ、又はtest名だけでは合格にしない。

## MANUAL.ADDITIONAL.FOLD-ALL.C02.SCREEN-ACCEPTANCE
1. 2026-09-05に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-fold-all-latency-cdp.mjs`（exit=0、`ADDITIONAL.FOLD-ALL.C02 VERIFY PASSED`）。ほかの測定を全て止めた静かな状態で3回実行した（同時に走る`cargo`・`rustc`・test実行ファイルはいずれも0件）。
2. 実測結果: NFR-002の2点を3回とも満たした。**ソルバー1回の最大は17.6 / 5.1 / 17.7 ms**（上限33 ms以内）、**3D更新は43.838 / 38.872 / 45.899 回/秒**（下限30回/秒以上）。「全部いっぺんに折ってみる」のつまみへ10・20…100%の10入力を送って往復時間を採り、続けて1秒間つまみを動かして`data-applied-percent`の変化回数を数えた。要件でない「入力から画面反映まで」の時間は合否に使わず参考値として出すだけにしている。
3. PID・実行ファイルSHA-256を照合した（実行ファイルSHA-256 `4BF0DC2268CB7001AED90F852EC5AF228A2EF365FAD82DB4347271EB20DE2FD6`、HEAD `dfd3c59`の同梱版）。測定のあいだだけ`window.fetch`を包み、終了時に必ず元へ戻す。記録と実測値は`scratchpad/acceptance-2026-09-05/M2.T2-6b.FOLD-ALL-LATENCY-quiet-1.log`〜`-quiet-3.log`にある（ID新設前の仮IDのままのファイル名）。
4. 同じ条件で再実行するときも、ソルバー1回が33 msを超えるか、更新が30回/秒を下回れば不合格にする。上限・下限はNFR-002の数値そのままで、緩めない。

## MANUAL.M1.T1-1.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M1.T1-1.C03` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M1.T1-10.C02.SCREEN-ACCEPTANCE
1. 2026-09-05に専用CDP枠で実行済み。実行本体: `scratchpad/acceptance-2026-09-05/driver-481-redo.mjs`（`apps/desktop/tests-live/doc-link-b1-cdp-support.mjs`の`connectDesktop`/`evaluate`/`restoreBlank`を使う。exit=0）。
2. 実測結果: 白紙へ**描いて**やっこさんを折った。正本`crates/ori3-rigid/tests/fixtures/check-yakko.ori3`と同じ折り目を谷8ストローク・山8ストロークで引き、頂点4→**20**・辺4→**36**（正本と一致）、平らにたためない点**0**・平坦条件の違反**0**・警告**0**。「全部いっぺんに折ってみる」100%で面17・外形0.5×0.5・**厚み0**（平らに畳めた）。表示はすべて日本語で、常設4区画（ツールレール1・展開図1・3D1・コンテキストパネル1）は不変。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に手順0・道具「選択」・開いているdialog 0の白紙へ復元した。記録と実測値は`scratchpad/acceptance-2026-09-05/MANUAL.M1.T1-10.C02-redo.stdout.log`、画像は同フォルダの`MANUAL.M1.T1-10.C02-redo-1-blank.png`〜`-4-folded.png`にある。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M1.T1-10.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M1.T1-10.C03` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M1.T1-2.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M1.T1-2.C03` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M1.T1-3.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M1.T1-3.C03` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M1.T1-4.C04.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M1.T1-4.C04` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M1.T1-5.C04.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M1.T1-5.C05.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M1.T1-5.C05` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M1.T1-6.C02.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M1.T1-6.C03.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M1.T1-6.C04.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M1.T1-6.C05.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M1.T1-6.C05` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M1.T1-7.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M1.T1-7.C03` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M1.T1-8.C04.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M1.T1-8.C04` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M1.T1-9.C01.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M1.T1-9.C02.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M1.T1-9.C03.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M1.T1-9.C04.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M1.T1-9.C05.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M1.T1-9.C05` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M2.T2-0.C05.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-0.C06.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-0.C06` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M2.T2-0.C07.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-0.C08.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-0.C09.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-0.C10.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-0.C11.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-0.C11` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M2.T2-1.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-1.C03` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M2.T2-2.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-2.C03` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M2.T2-2.C04.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-2.C04` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M2.T2-3.C03.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-3.C05.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-3.C05` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M2.T2-3.C06.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-3.C06` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M2.T2-4.C01.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-4.C02.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-4.C03.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-4.C04.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-4.C04` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M2.T2-5.C01.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-5.C03.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-5.C04.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-5.C05.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-5.C06.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-5.C07.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-5.C07` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M2.T2-6.C04.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-6.C06.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-6.C06` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M2.T2-6b.C05.SCREEN-ACCEPTANCE
1. 2026-09-05に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-pull-cdp.mjs`（exit=0、`M2.T2-6b.C05 VERIFY PASSED`）。
2. 実測結果: つかんで動かす操作で手順がちょうど1件増え（1→2）、折り目の辺が36→51へ増え、道具は「折る」のまま、離した後の掴みは解除（`grab.active=false`）、増えた手順「2 単純折り」がタイムラインにある。fixtureは`crates/ori3-rigid/tests/fixtures/check-yakko.ori3`、正規化座標(0.50,0.50)→(0.65,0.50)。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に手順0・道具「選択」・開いているdialog 0の白紙へ復元した。記録と実測値は`scratchpad/acceptance-2026-09-05/MANUAL.M2.T2-6b.C05.SCREEN-ACCEPTANCE.stdout.log`にある。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M2.T2-6b.C06.SCREEN-ACCEPTANCE
1. 2026-08-26に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-cdp.mjs`。
2. 実測結果: 技法9種の名称と順序が完全一致。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に指定作品、道具、dialog、capture属性、viewportを復元した。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M2.T2-6b.C07.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-6b.C08.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-6b.C08` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M2.T2-6c.C01.SCREEN-ACCEPTANCE
1. 2026-08-26に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-remaining-cdp.mjs`。
2. 実測結果: 固定1280×860で主要3層が各14,000物理画素以上、層重心間80画素以上。固定drag/wheel後も同条件、視点差50画素以上。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に指定作品、道具、dialog、capture属性、viewportを復元した。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M2.T2-6c.C02.SCREEN-ACCEPTANCE
1. 2026-08-26に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-grab-cdp.mjs`。
2. 実測結果: 通常dragの対象面13、Shift dragの対象面17、両方とも手順をちょうど1件追加。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に指定作品、道具、dialog、capture属性、viewportを復元した。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M2.T2-6c.C03.SCREEN-ACCEPTANCE
1. 2026-08-26に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-grab-cdp.mjs`。
2. 実測結果: 通常dragのプレビュー多角形13・線分49、Shift dragの多角形17・線分61、release後grab inactive。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に指定作品、道具、dialog、capture属性、viewportを復元した。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M2.T2-6c.C04.SCREEN-ACCEPTANCE
1. 2026-08-26に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-cdp.mjs`。
2. 実測結果: 通常時と途中step時の操作ヒント各1件、標準修飾キー名以外の英字語0。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に指定作品、道具、dialog、capture属性、viewportを復元した。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M2.T2-6c.C05.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-6c.C07.SCREEN-ACCEPTANCE
1. 担当: このDOM検査を実装していないレビュー担当者（画面の目視確認をした人とも別の人）。
2. 対象: `apps/desktop/src/components/Viewer3D/Viewer3D.dom.test.tsx`、`ViewerOperationHint.dom.test.tsx`、`PaperActionTip.dom.test.tsx`、`apps/desktop/src/components/OperationSteps.dom.test.tsx`、`apps/desktop/src/lib/layerMotion.test.ts`、およびテスト設定。プレビュー表示、操作理由、ドラッグの開始・移動・終了、手順表示のそれぞれが、画面部品を実際に組み立てた検査で確認されているかを読む。
3. 実行: `cd apps/desktop; npm.cmd run test -- --run src/components/Viewer3D/Viewer3D.dom.test.tsx src/components/Viewer3D/ViewerOperationHint.dom.test.tsx src/components/Viewer3D/PaperActionTip.dom.test.tsx src/components/OperationSteps.dom.test.tsx src/lib/layerMotion.test.ts`。実行結果で全対象がpassし、skipが0であることを確認する。
4. 合格: 各観点に対応するtest名・実行結果・確認日を記録する。入力変換だけ、test名だけ、又は画面の見た目だけでは合格にしない。少なくとも1本はDOM上のpointer down/move/upを通し、少なくとも1本はプレビュー又は操作理由のDOMを確認していなければ不合格とする。
5. 不合格: 対応する検査が無い、skipがある、又は上のコマンドが失敗したときは、文書の状態を変えず不足した観点と実パスを統括へ報告する。

## MANUAL.M2.T2-6c.C08.SCREEN-ACCEPTANCE
1. 2026-09-05に専用CDP枠で実行済み。このcheckboxの成果物は「詰まった箇所を`docs/progress.md`に記録」であり、**所見は`docs/progress.md`の「2026-09-05 - 説明なしで座布団折りから鶴の基本形まで折れるかを実機で確かめ、詰まった箇所を記録した」の節**にある。
2. 実測結果の要約: 座布団折りは説明なしで折れた（頂点8・辺12、平らにたためない点0・警告0、100%で面5・厚み0）。詰まりは2件で、①次に折る技法を画面が案内しない（技法一覧は9種の名前とヒント「左の一覧から技法を選んでください」だけ）②平らに畳めないときに、山谷を変えるべき折り目を画面が名指ししない（案内は「平らにたためない場所があります」だけ）。鶴の基本形には到達しなかった。操作不能・英語表示・表示崩れは0件。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に手順0・道具「選択」・開いているdialog 0の白紙へ復元した。記録と実測値は`scratchpad/acceptance-2026-09-05/MANUAL.M2.T2-6c.C08.stdout.log`、画像は同フォルダの`MANUAL.M2.T2-6c.C08-A1-zabuton-cp.png`〜`-C1-technique-menu.png`にある。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M2.T2-6c.C09.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-6c.C09` と同じTaskを確認する。
2. 明示対応commit: `85b8ca42b473f16312c4431b880bf569f48538f9`（題名: 紙をつかんで動かす直感的な折り操作に変更）。
3. `git merge-base --is-ancestor 85b8ca42b473f16312c4431b880bf569f48538f9 origin/main` の確認結果: `True`。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M2.T2-7.C01.SCREEN-ACCEPTANCE
1. 2026-08-26に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-remaining-cdp.mjs`。
2. 実測結果: 作図4種各1、等分4、角度22.5°、補助線画素の増分が角度4,000・垂線45・等分55・二等分20以上。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に指定作品、道具、dialog、capture属性、viewportを復元した。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M2.T2-7.C02.SCREEN-ACCEPTANCE
1. 2026-08-26に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-remaining-cdp.mjs`。
2. 実測結果: 違反fixtureの橙(#ff8c00、RGB距離12以内)画素412、合格境界320以上。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に指定作品、道具、dialog、capture属性、viewportを復元した。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M2.T2-7.C03.SCREEN-ACCEPTANCE
1. 2026-09-05に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-penetration-cdp.mjs`（exit=0、`M2.T2-7.C03 VERIFY PASSED`）。
2. 実測結果: 面が交差するfixture（`crates/ori3-layers/tests/fixtures/penetration-warning.ori3`）で、警告バッジがちょうど1個、疑わしい折り目の案内がちょうど1個、capture APIの警告数1、バッジのclassは`status-badge`だけで`error`を含まず、バッジの文言は日本語の「警告 1」。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に手順0・道具「選択」・開いているdialog 0の白紙へ復元した。記録と実測値は`scratchpad/acceptance-2026-09-05/MANUAL.M2.T2-7.C03.SCREEN-ACCEPTANCE.stdout.log`にある。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M2.T2-7.C04.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-7.C04` と同じTaskを確認する。
2. 明示対応commit: `dfd5ca03dce87fa2ae6cfff5cb05aba5b527d478`（題名: 作図の補助線・折りたたみ可否の注意表示・紙のめり込み警告を追加）。
3. `git merge-base --is-ancestor dfd5ca03dce87fa2ae6cfff5cb05aba5b527d478 origin/main` の確認結果: `True`。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M2.T2-8.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-8.C03` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M2.T2-9.C02.SCREEN-ACCEPTANCE
1. 2026-09-05に専用CDP枠で実行済み。実行本体: `scratchpad/acceptance-2026-09-05/driver-752-redo.mjs`（exit=0）。作品は`apps/desktop/tests-live/fixtures/traditional-crane-full.ori3`（正本CP 頂点56・辺114、手順3。`crates/ori3-layers/tests/acceptance_crane.rs`の`crane()`と同じ辺ID群で正本の一括collapse 1手を3手へ分けたもの）。
2. 実測結果: 手順0〜3を1つずつ進めて**鶴が完成した**（札は「折る前」「1 単純折り」「2 花弁折り」「3 中割り折り」で全て日本語。面59、3D画像に翼・首・頭・尾が見える）。展開図の内側の頂点id=10を(0.5,0.6659)→(0.56,0.7059)へドラッグして修正すると、「再生」の後に完成形が変わり（面座標のチェックサム96091.5→91406.8、外接箱も変化）、画面上部に日本語で「指定を優先し、いちばん近い形で追従中」と出て**追従した**。操作不能・英語表示・表示崩れは0件で、常設4区画は不変。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に手順0・道具「選択」・開いているdialog 0の白紙へ復元した。記録と実測値は`scratchpad/acceptance-2026-09-05/MANUAL.M2.T2-9.C02-redo.stdout.log`、画像は同フォルダの`MANUAL.M2.T2-9.C02-redo-1-opened.png`〜`-4-after-replay.png`にある。手順2（鳥の基本形）では日本語で「この折り方だと紙が突き抜けています」「指定した角度に近い形を表示しています（閉包RMS 1.655e-12）」と出て操作は止まらない（`-redo-chip-step-2.png`）。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M2.T2-9.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-9.C03` と同じTaskを確認する。
2. 明示対応commit: `f00628a8d365a01a71421cfeb32467e77bb75ebd`（題名: 折り鶴が折れることを確認する自動テストを追加）。
3. `git merge-base --is-ancestor f00628a8d365a01a71421cfeb32467e77bb75ebd origin/main` の確認結果: `True`。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M3.T3-1.C01.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M3.T3-1.C01` と同じTaskを確認する。
2. 明示対応commit: `6ce06fb3ac7cb21bf694a12af2db7a1871710f67`（題名: 頭・尾・足などの骨格を指定するためのデータ形式を追加）。
3. `git merge-base --is-ancestor 6ce06fb3ac7cb21bf694a12af2db7a1871710f67 origin/main` の確認結果: `True`。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M3.T3-2.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M3.T3-2.C03` と同じTaskを確認する。
2. 明示対応commit: `8532fb2dc74fb8cf606569ca6a00ca212677c1c5`（題名: 骨格に合わせて紙の上に必要な領域を自動配置する計算を追加）。
3. `git merge-base --is-ancestor 8532fb2dc74fb8cf606569ca6a00ca212677c1c5 origin/main` の確認結果: `True`。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M3.T3-3.C04.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M3.T3-3.C04` と同じTaskを確認する。
2. 明示対応commit: `e66e15152b347e7e0db1a77e7927fa7c83cc5d5d`（題名: 自動配置の結果から展開図を組み立てる機能を追加）。
3. `git merge-base --is-ancestor e66e15152b347e7e0db1a77e7927fa7c83cc5d5d origin/main` の確認結果: `True`。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M3.T3-4.C01.SCREEN-ACCEPTANCE
1. 2026-08-26に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-cdp.mjs`。
2. 実測結果: skeleton/candidates/confirm各1回、候補4件、違反数文4件、適用後dialog 0。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に指定作品、道具、dialog、capture属性、viewportを復元した。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M3.T3-4.C02.SCREEN-ACCEPTANCE
1. 2026-08-26に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-cdp.mjs`。
2. 実測結果: 提案前後でツールレール・展開図・3D・下部パネルが各1。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に指定作品、道具、dialog、capture属性、viewportを復元した。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M3.T3-4.C03.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M3.T3-4.C04.COMMIT-PUSH
1. 2026-09-05に実行済み。`docs/implementation-roadmap.md` の `M3.T3-4.C04` と同じTaskを確認した。
2. 明示対応commit: `dbb2a6b`（題名: 骨格を指定して展開図を提案してもらう画面を追加）。統括が`git log --grep`で実測し、リモート本線`origin/main`（当時のHEAD `dfd3c59`）の祖先であることを確認した。
3. 画面部分も確認済み: 出っぱりを既定4本から6本へ増やして頭1・尾1・足4を指定でき、提案の3画面（骨格→候補→確認）を通り、候補4件の説明は日本語、「この展開図を使う」で適用してdialog 0・展開図（頂点29・辺64・手順1）が入り、そのまま「谷」「折る」道具へ進めた。常設4区画は提案中も適用後も不変。記録は`scratchpad/claude-acceptance-report.md`の796の節と`scratchpad/acceptance-2026-09-05/MANUAL.M3.T3-4.C04.stdout.log`、画像は同フォルダの`MANUAL.M3.T3-4.C04-1-skeleton.png`〜`-5-editable.png`にある。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M3.T3-4.C22.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M3.T3-4.C23.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M3.T3-4.C37.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M4.T4-1.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M4.T4-1.C03` と同じTaskを確認する。
2. 明示対応commit: `e2a4dff1ce092417e3ff722a082d010a738efabf`（題名: 沈め折りを追加）。
3. `git merge-base --is-ancestor e2a4dff1ce092417e3ff722a082d010a738efabf origin/main` の確認結果: `True`。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M4.T4-2.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M4.T4-2.C03` と同じTaskを確認する。
2. 明示対応commit: `98e94ad293beb1b81c4c66acead9ff8d47248171`（題名: ひだ寄せとねじり折りを追加）。
3. `git merge-base --is-ancestor 98e94ad293beb1b81c4c66acead9ff8d47248171 origin/main` の確認結果: `True`。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M4.T4-3.C02.SCREEN-ACCEPTANCE
1. 2026-08-26に専用CDP枠で実行済み。実行本体: `apps/desktop/tests-live/doc-link-b1-cdp.mjs`。
2. 実測結果: 書出しradio 4、PNG長辺1024、補助線checkboxを両状態へ切替。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に指定作品、道具、dialog、capture属性、viewportを復元した。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M4.T4-3.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M4.T4-3.C03` と同じTaskを確認する。
2. 明示対応commit: `8ad7be3511f64dce20c8aaa8b4b1a897e4d5d656`（題名: 展開図を画像ファイルとして保存する機能を追加）。
3. `git merge-base --is-ancestor 8ad7be3511f64dce20c8aaa8b4b1a897e4d5d656 origin/main` の確認結果: `True`。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M4.T4-4.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M4.T4-4.C03` と同じTaskを確認する。
2. 明示対応commit: `1b1a0e650cd373fc0a877d7fb133452f767739ba`（題名: 折り手順を1コマずつ図にする機能を追加）。
3. `git merge-base --is-ancestor 1b1a0e650cd373fc0a877d7fb133452f767739ba origin/main` の確認結果: `True`。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M4.T4-5.C04.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M4.T4-5.C04` と同じTaskを確認する。
2. 明示対応commit: `eb1c2c5904ebe67c15d2e2331c9533cddf91705c`（題名: 折り図をPDFとして保存する機能を追加）。
3. `git merge-base --is-ancestor eb1c2c5904ebe67c15d2e2331c9533cddf91705c origin/main` の確認結果: `True`。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M4.T4-6.C02.SCREEN-ACCEPTANCE
1. 2026-09-05に専用CDP枠で実行済み。実行本体: `scratchpad/acceptance-2026-09-05/driver-945-redo.mjs`（exit=0）。作品は`apps/desktop/tests-live/fixtures/frog.ori3`（頂点141・辺280、手順14。`crates/ori3-layers/tests/acceptance_frog.rs`の`frog()`が折る伝承のカエル）。
2. 実測結果: 手順0〜14を1つずつ進めて**カエルが完成した**（札は「1 単純折り」「2 単純折り」「3〜8 開いてつぶす」「9 花弁折り」「10〜13 中割り折り」「14 段折り」で全て日本語、面140、警告0）。「書き出し」→「折り図(PDF)」→「保存先を選んで書き出す」でPDFを書き出し、画面に日本語で「保存しました:frog-diagram.pdf」と出た。書き出したPDFを開いて目視した: **4ページ・A4（595.28×841.89pt＝210×297mm）・427,473バイト**、1ページ目は表紙「折り図／できあがりの形(全14手順)／紙の大きさ 100×100mm」でカエルの完成形（足4本）の絵、2〜4ページは1ページ6コマ・番号1〜14の日本語の手順（山は赤・谷は青・矢印つき、ページ番号「2ページ」〜「4ページ」）。操作不能・英語表示・表示崩れは0件。
3. PID・実行ファイルSHA-256・fixture SHA-256を照合し、終了時に手順0・道具「選択」・開いているdialog 0の白紙へ復元した。記録と実測値は`scratchpad/acceptance-2026-09-05/MANUAL.M4.T4-6.C02-redo.stdout.log`、書き出したPDFは`%TEMP%\ori3-acceptance-2026-09-05\frog-diagram.pdf`（SHA-256 `AD4CCBD41DD0E219D1B68053C8A18C9759E7BA289A4DD263B4C91AC638D22536`）、画像は同フォルダの`MANUAL.M4.T4-6.C02-redo-1-opened.png`〜`-4-export-saved.png`と`-pdf-page1.png`〜`-pdf-page4.png`にある。
4. 同じ条件で再実行するときも、1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

## MANUAL.M4.T4-6.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M4.T4-6.C03` と同じTaskを確認する。
2. 明示対応commit: `7c49536e8807074751cebd7852f801b5f24dd79b`（題名: 伝承のカエルが完成形まで折れることを確認する自動テストを追加）。
3. `git merge-base --is-ancestor 7c49536e8807074751cebd7852f801b5f24dd79b origin/main` の確認結果: `True`。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M6.ACCEPTANCE.C01.FULL-ACCEPTANCE
1. クリーンなcommit済みtreeで全品質ゲートを通す。
2. 統括が日本語ヘルプ、初回ガイドの再表示、5テーマの保存・復元を画面で確認する。
3. 各確認の画面又は記録参照を残し、1つでも不足ならM6を合格にしない。
