# 手動受入手順

この文書のIDは `roadmap-links.json` の手動証拠と1対1で対応する。実施者はID、日付、結果、確認した画面又は履歴を記録する。担当者はアプリを起動せず、画面確認は統括が同梱版で行う。

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
| `MANUAL.M2.T2-8.C02.SCREEN-ACCEPTANCE` | X | 復旧ダイアログ |
| `MANUAL.M3.T3-4.C01.SCREEN-ACCEPTANCE` | X | 提案ウィザード3画面 |
| `MANUAL.M3.T3-4.C02.SCREEN-ACCEPTANCE` | X | 提案ウィザードの起動位置 |
| `MANUAL.M4.T4-3.C02.SCREEN-ACCEPTANCE` | X | 展開図書き出しダイアログ |
| `MANUAL.M4.T4-5.C03.SCREEN-ACCEPTANCE` | X | 手順図書き出しダイアログ |

### X: CDP自動化の共通手順

1. 専用の検査環境で同梱版を1つだけ起動し、CDP接続後に該当IDの操作を再現する。
2. 指定された文字列、要素領域、状態ごとのスクリーンショットを取得し、期待する画素領域又は文字列と比較する。
3. ID、操作列、取得画像、比較結果を保存する。1つでも操作不能・表示欠落・期待外の画素差があれば不合格にする。

### Y: 人が判断する手順

#### `MANUAL.M2.T2-6c.C07.SCREEN-ACCEPTANCE`
1. 担当: 画面検査の担当者とは別のレビュー担当者。
2. `apps/desktop/src/lib/layerMotion.test.ts` とテスト設定を読み、jsdomとTesting Libraryの基盤、およびプレビュー・ヒント・ドラッグの主要経路を検査する実在testがあることを確認する。
3. 担当者が指定する検査名一覧の取得又は対象test実行の結果を確認し、ID、確認日、確認したtest名、結果を記録する。画面上の見た目だけ、又はtest名だけでは合格にしない。

## MANUAL.M1.T1-1.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M1.T1-1.C03` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M1.T1-10.C02.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

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
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

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
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-6c.C03.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

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
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-6c.C08.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

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
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-7.C04.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-7.C04` と同じTaskを確認する。
2. 明示対応commit: `dfd5ca03dce87fa2ae6cfff5cb05aba5b527d478`（題名: 作図の補助線・折りたたみ可否の注意表示・紙のめり込み警告を追加）。
3. `git merge-base --is-ancestor dfd5ca03dce87fa2ae6cfff5cb05aba5b527d478 origin/main` の確認結果: `True`。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M2.T2-8.C02.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M2.T2-8.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M2.T2-8.C03` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

## MANUAL.M2.T2-9.C02.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

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
1. `docs/implementation-roadmap.md` の `M3.T3-4.C04` と同じTaskを確認する。
2. 統括が指定されたコミット題名と進捗記録を履歴で照合し、リモート本線の祖先であることを確認する。
3. 題名・確認日・結果を記録し、確認不能なら合格にしない。

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

## MANUAL.M4.T4-5.C03.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M4.T4-5.C04.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M4.T4-5.C04` と同じTaskを確認する。
2. 明示対応commit: `eb1c2c5904ebe67c15d2e2331c9533cddf91705c`（題名: 折り図をPDFとして保存する機能を追加）。
3. `git merge-base --is-ancestor eb1c2c5904ebe67c15d2e2331c9533cddf91705c origin/main` の確認結果: `True`。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M4.T4-6.C02.SCREEN-ACCEPTANCE
1. 統括が画面を同梱した版を1つだけ起動し、checkbox本文の操作を行う。
2. 本文にある表示、操作結果、日本語の案内を目視し、画面又は撮影記録への参照を残す。
3. 操作不能、英語表示、表示崩れがあれば不合格として進捗を書き換えずに報告する。

## MANUAL.M4.T4-6.C03.COMMIT-PUSH
1. `docs/implementation-roadmap.md` の `M4.T4-6.C03` と同じTaskを確認する。
2. 明示対応commit: `7c49536e8807074751cebd7852f801b5f24dd79b`（題名: 伝承のカエルが完成形まで折れることを確認する自動テストを追加）。
3. `git merge-base --is-ancestor 7c49536e8807074751cebd7852f801b5f24dd79b origin/main` の確認結果: `True`。
4. この対応はTask番号だけで推測していない。題名・確認日・結果を記録し、祖先でなければ合格にしない。

## MANUAL.M6.ACCEPTANCE.C01.FULL-ACCEPTANCE
1. クリーンなcommit済みtreeで全品質ゲートを通す。
2. 統括が日本語ヘルプ、初回ガイドの再表示、5テーマの保存・復元を画面で確認する。
3. 各確認の画面又は記録参照を残し、1つでも不足ならM6を合格にしない。
