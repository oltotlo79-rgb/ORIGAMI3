# B1手動受入のCDP自動化仕様

対象はB1のX分類15件である。この文書は実装仕様であり、CDP接続、アプリ起動、ブラウザ起動は行っていない。

## 共通前提

- 新規の実行本体は `apps/desktop/tests-live/doc-link-b1-cdp.mjs` とする。
- CDP接続、起動済み同梱版とのPID照合、fixtureのhash照合、WebGL readbackは `apps/desktop/src/components/Viewer3D/hiddenCreaseOwner.cdp.mjs` を踏襲する。
- 既存のCDP接続・評価・結果保存の骨格は `apps/desktop/tests-live/backface-canonical-cdp.mjs` を踏襲する。
- 画素を測るケースは、既存検査と同じ `1280×860 @ 2` の固定viewportを使う。canvasとDPRが一致しなければ測定前に失敗にする。
- 画素数を合格条件に使うケースは、同一の追跡済みfixtureで3回測定して得る `n1`、`n2`、`n3` に対し、しきい値を `floor(0.8 × min(n1,n2,n3))` とする。実測値そのものを境目にせず、約20%の余裕を取る。現在はアプリを起動しない制約のため、この3値は未測定であり、数値を推測で固定しない。
- DOMに専用のtest IDがない箇所は、下表の現存selectorを最初に使う。テキスト又は装飾classに頼る箇所は、実装時に併記したsourceへ `data-testid` を追加してから恒久化する。

## 15件の仕様

| ID | CDP操作（現存DOMの目印と入力） | 測定値 | 数値の合格条件 | 実装先 / 参考 |
|---|---|---|---|---|
| `M2.T2-6b.C05` | `nav.tool-rail`内で表示文字列`引く`のbutton（現行の「つまんで動かす」に対応）を押す。`canvas.viewer3d-canvas`の中央から横幅の15%右へ左ドラッグする。 | pointer down中のcursor、操作後の手順数、3D readbackの変更画素数。 | 手順数がちょうど+1、変更画素数は共通式以上、cursorが`grabbing`を1回以上観測。 | 新規`apps/desktop/tests-live/doc-link-b1-cdp.mjs`。恒久selector追加先`apps/desktop/src/components/ToolRail.tsx`、参照`apps/desktop/src/components/Viewer3D/grabFold.test.ts`。 |
| `M2.T2-6b.C06` | 表示文字列`技法`のbuttonを押し、`[role="group"][aria-label="技法を選ぶ"]`を読む。 | `button[aria-label]`の個数と名称。 | 9個ちょうどで、順に層操作、段折り、中割り折り、かぶせ折り、開いてつぶす、花弁折り、沈め折り、ひだ寄せ、ねじり折りであること。 | 新規CDP script。selector追加先`apps/desktop/src/components/ToolRail.tsx`。参照`apps/desktop/src/lib/techniques.ts`。 |
| `M2.T2-6c.C01` | 多層fixtureを開き、`canvas.viewer3d-canvas`へ固定した視点ドラッグとホイール入力を送る。 | `window.__origami3Capture.captureCanonical3D().readback`上の層ごとの可視色領域・重心間距離。 | 可視層が3以上、隣接重心距離が1物理画素以上、各層領域が共通式以上。 | 新規CDP script。fixtureとreadback追加先`apps/desktop/tests-live/fixtures/`、必要なら`apps/desktop/src/captureApi.ts`。参照`apps/desktop/src/lib/layerOffset.test.ts`、`apps/desktop/src/components/Viewer3D/sceneBuilder.test.ts`。 |
| `M2.T2-6c.C02` | `canvas.viewer3d-canvas`で、fixtureが定める最前面フラップの正規化座標`(0.50,0.50)`から`(0.65,0.50)`へドラッグし、同じ操作をShift付きで行う。 | 手順数、3D readback、選択層数を返すCDP専用読取値。 | 各操作で手順数がちょうど+1、通常/Shiftの選択層数が異なり、変更画素数が共通式以上。 | 新規CDP script。読取値追加先`apps/desktop/src/captureApi.ts`、操作本体の参照`apps/desktop/src/components/Viewer3D/viewerPointer.ts`。 |
| `M2.T2-6c.C03` | C02と同じ開始点でpointer down→move後、pointer up前にreadbackを取る。 | 半透明プレビュー、動く層、折り線の各色の画素数と、up後の最終readbackとの差。 | 3種類の色領域が各1以上かつ共通式以上。up後の最終形との差分画素数が共通式以下（プレビューと実行結果の許容差）。 | 新規CDP script。必要な読取口`apps/desktop/src/captureApi.ts`、参照`apps/desktop/src/components/Viewer3D/viewerHighlight.ts`。 |
| `M2.T2-6c.C04` | `aside[data-floating-ui="viewer-operation-hint"]`と内部`p[role="status"]`を読む。`折る`を選び、折り途中fixtureへ移し、無効理由も読む。 | status要素数、表示文字列、英字だけの語の件数。 | status要素は常に1、通常時と無効時の文が各1、英字だけの利用者向け語が0。 | 新規CDP script。参照`apps/desktop/src/components/Viewer3D/ViewerOperationHint.dom.test.tsx`。 |
| `M2.T2-6c.C05` | `技法`→`[aria-label="段折り"]`を選び、3D canvasでfixtureの指定線をドラッグする。 | `section.operation-steps`の進行表示とTimelineの`button`本文。 | 操作前後で手順数がちょうど+1、追加行が`段折り`を1回だけ含む。 | 新規CDP script。必要ならtimeline selectorを`apps/desktop/src/components/Timeline.tsx`へ追加。参照`apps/desktop/src/components/OperationSteps.dom.test.tsx`。 |
| `M2.T2-7.C01` | `作図`button→`[role="group"][aria-label="作図の種類を選ぶ"]`を開く。`二等分`、`垂線`、`等分`、`角度`を各1回選び、`[aria-label="いくつに等分するか"]`へ4、`[aria-label="角度の刻み"]`へ22.5を入れる。 | 子button数、select値、`canvas.cp-canvas`の追加線数。 | button数4、等分値4、角度値22.5、各作図後の追加線数が1以上。 | 新規CDP script。selector追加先`apps/desktop/src/components/ToolRail.tsx`、参照`apps/desktop/src/lib/construct.ts`。 |
| `M2.T2-7.C02` | 前川・川崎違反を持つfixtureを開き、`canvas.cp-canvas`を固定表示する。 | RGBが`#ff8c00`（距離12以下）の画素数と違反頂点数。 | 違反頂点数1以上、橙画素数が共通式以上。 | 新規CDP script。fixture追加先`apps/desktop/tests-live/fixtures/`、参照`apps/desktop/src/components/CpEditor/renderer.ts`、`CpEditor`のDOM test群。 |
| `M2.T2-7.C03` | 面交差fixtureを開き、3D表示へ切替える。`[data-floating-ui="status-badge"]`と`[data-floating-ui="suspect-hinge-guide"]`を読む。 | badge/guidanceの個数、`#ff2438`近傍画素数。 | badge=1、guide=1、赤画素数が共通式以上。 | 新規CDP script。参照`apps/desktop/src/components/ViewerStatusOverlays.tsx`、`apps/desktop/src/components/Viewer3D/sceneLayers.ts`。 |
| `M2.T2-8.C02` | 事前に回復fixtureを配置して起動し、`[data-floating-ui="recovery-dialog"]`を待つ。`button`の`復元する`又は`破棄する`を1回押す。 | dialog数、button名、押下後のdialog数。 | 起動時dialog=1、選択肢はちょうど2、押下後dialog=0。 | 新規CDP script。fixture作成先`apps/desktop/tests-live/fixtures/`、参照`apps/desktop/src/components/RecoveryDialog.dom.test.tsx`。 |
| `M3.T3-4.C01` | toolbarの表示文字列`提案`を押す。`[data-floating-ui="proposal-dialog"]`で`data-proposal-step`を読み、`展開図を作ってもらう`→`[aria-label="候補1"]`→`これにする`→`この展開図を使う`を順に押す。 | 通過したstep種類、候補button数、`候補n:`の違反数文、適用後dialog数。 | 必須3画面（skeleton/candidates/confirm）を各1回、候補=4、違反数文=4、適用後dialog=0。 | 新規CDP script。参照`apps/desktop/src/components/dialogs/ProposalWizard.dom.test.tsx`。 |
| `M3.T3-4.C02` | toolbarの`提案`を押してから閉じる。閉じた前後で`.tool-rail`、`.cp-editor`、`canvas.viewer3d-canvas`、`#context-panel`を数える。 | proposal dialog数と4区画の要素数。 | 開く前0、開いた時1、閉じた後0。4区画は各ちょうど1で、開閉前後とも合計4。 | 新規CDP script。selector追加先`apps/desktop/src/components/AppToolbar.tsx`、参照`apps/desktop/src/components/dialogs/ProposalWizard.dom.test.tsx`。 |
| `M4.T4-3.C02` | toolbarの`書き出し`を押す。`[data-floating-ui="export-dialog"]`で`展開図(PNG)` radioを選び、`[aria-label="画像の大きさ（長辺の点数）"]`へ1024、`補助線(下書きの線)も含める`を切替える。 | radio数、数値input値、checkbox値。 | radio=4、input値=1024、checkboxのtrue/false切替が各1回反映。 | 新規CDP script。参照`apps/desktop/src/components/dialogs/ExportDialog.dom.test.tsx`。 |
| `M4.T4-5.C03` | toolbarの`書き出し`→`折り図(PDF)`と`折り図(ページごとのSVG)` radioを各1回選び、手順ありfixtureで保存操作へ進む。 | radioの選択値、保存成功文、出力ファイル数・サイズ。 | radio=4、対象2種の選択各1回、保存成功文各1、PDF=1ファイル・SVG=1ページ以上、各size>0。 | 新規CDP script。参照`apps/desktop/src/components/dialogs/ExportDialog.dom.test.tsx`、`apps/desktop/src/components/dialogs/exportChoices.ts`。 |

## 実装前に解消する点

1. `M2.T2-6b.C06`は、ロードマップを現行実装どおり9種へ更新済みである。CDP検査は9個と名称・順序を完全一致で確認する。
2. canvas操作の対象位置、層数、警告を安定させる追跡済みfixtureがまだ決まっていない。各fixtureのhashと正規化座標を検査コードへ固定する。
3. `M2.T2-6c.C01`、`C02`、`C03`は、現在の`window.__origami3Capture`が選択層数・プレビューを返さない。画素だけで推測せず、必要な読取専用値を`apps/desktop/src/captureApi.ts`へ追加する。
4. toolbarとtool railの主要buttonには専用test IDがない。実装時に`AppToolbar.tsx`と`ToolRail.tsx`へ追加し、表示文言やCSS classだけを恒久selectorにしない。
