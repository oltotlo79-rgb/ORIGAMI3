# ORIGAMI3に同梱する日本語フォントの出所

ブラウザ版の画面は、日本語フォントを持たない機械（Windows以外の環境、フォントを絞った端末など）でも
文字が四角い箱にならないように、日本語フォントを1本だけ同梱する。また、デスクトップ版と
ブラウザ版の折り図PDFは、機械の書体一覧に左右されず同じ字形・同じbytesになるよう、
この同じフォントをRustの実行ファイル/WASMへ埋め込んで使う。
同梱するのは元のフォントそのものではなく、この製品に実際に出る文字だけを残した縮小版である。

## 同梱物

| 配置 | 大きさ | SHA-256 | 役割 |
|---|---:|---|---|
| `apps/web/public/fonts/NotoSansJP-ORIGAMI3-subset.ttf` | 475,640 B | `3F9B664FABBFA1BA4032A169A1B9736F8D9E5B8C0E4058ACC037E6A286C5E514` | ブラウザ画面と両版の折り図PDFに使う縮小版 |
| `apps/web/public/fonts/OFL.txt` | 4,388 B | `1C05C68C34F9708415AADA51F17E1B0092D2CEA709BF4A94CD38114F9E73D7D9` | 配布条件の原文。同梱が条件なので必ず一緒に配る |
| `apps/web/fonts/charset.txt` | 2,740 B | `19BC79E9C35BAFCBA836BF00C70B1D0FBD5BFEA70AFE4440B82293704B5A989F` | 縮小版へ残した977文字。作り直しの入力 |

TTFとcharset.txtは2026-09-04に作り直した。2026-08-29版（`470,716 B` / `2FD9D3E2…`、970文字）から、
ヘルプ本文とRustの文言へ増えた7字（`α ぬ 恒 競 義 翼 飾`）を足したものである。

`public/` の2つはブラウザ版の組み立て後の `dist/fonts/` へそのまま入り、
`/fonts/…` で配られる。デスクトップ版はTTFを実行ファイルへ埋め込み、
`apps/desktop/src-tauri/tauri.conf.json` のresource指定で同じ `OFL.txt` を
`licenses/NotoSansJP-OFL.txt` として同梱する。`apps/web/fonts/` は配らない。
作り直しの記録と入力だけを置く。

## 元のフォント

| 項目 | 値 |
|---|---|
| 名前 | Noto Sans JP（可変フォント、太さの軸 `wght` 100〜900） |
| 取得元 | `https://raw.githubusercontent.com/google/fonts/main/ofl/notosansjp/NotoSansJP%5Bwght%5D.ttf` |
| 取得日 | 2026-08-29（利用者の許可を得て統括が取得） |
| 大きさ | 9,589,900 B |
| SHA-256 | `C2F3B4D463500A2DDCD3849CDED1FCEEB9FD6D1C32E6CBECD568453BA50FC68F` |
| 版（`name` 5番） | `Version 2.004-H2;hotconv 1.0.118;makeotfexe 2.5.65603` |
| 版（`head`） | fontRevision 2.004 |
| 著作権表示（`name` 0番） | `(c) 2014-2021 Adobe (http://www.adobe.com/), with Reserved Font Name 'Source'.` |
| 埋め込み許可（`OS/2` fsType） | 0（制限なし） |
| 収録字数 | 16,732 Unicode |
| 上流の出所（`METADATA.pb`） | `https://github.com/notofonts/noto-cjk` commit `523d033d6cb47f4a80c58a35753646f5c3608a78`、`license: "OFL"` |

同じ場所から取った `METADATA.pb` は 983 B / SHA-256 `5AA23F70FBA3EB1AFA6D92CB89DE3B5C4690018CADDBDDED678B5E595E56825B` だった。

`C:\Windows\Fonts\NotoSansJP-VF.ttf` に見た目の同じファイルがあるが、こちらは
`Version 2.04;241114210129;non-release` と書かれた別の組み立て物で、大きさも944 B違う。
公式配布物ではないので使わない。

## 利用条件

SIL Open Font License 1.1（`OFL.txt` が原文）。要点は次の3つ。

1. 条件2 — 元のままでも変更版でも、ソフトウェアに同梱して再配布してよい。ただし
   **どの複製にも著作権表示とこのライセンス本文が入っていること**。
   縮小版の `name` 表には0番（著作権）・7番（商標）・9番（制作者）・13番（ライセンス説明）・
   14番（ライセンスの場所）を残してあり、加えて `OFL.txt` を同じ場所へ置いて配る。
2. 条件3 — 予約名（Reserved Font Name）は変更版に使えない。このフォントが宣言する予約名は
   `Source` であり `Noto` ではない。製品では `Source` を表示名に使わない。
   CSSでは別名 `"ORIGAMI3 JP"` で読み込むので、利用者の機械に入っている本物の
   `Noto Sans JP` と取り違えることもない。
3. 条件5 — フォント自体はこのライセンスのままで配る。フォントを使って作った文書
   （書き出したPDFなど）や、ORIGAMI3本体までがこのライセンスになるわけではない。

字形を削る操作はライセンス原文の定義でいう「変更版（Modified Version）」に当たる。
上の1・2はそれを踏まえた扱いである。

## 残した文字の決め方

`charset.txt` は次の原典から**機械的に**集めた。手で並べた文字は1つも無い。
規則は「コメントを取り除いたソースに残る文字」で、コメントを外せば非ASCIIとして残るのは
文字列リテラルとJSXの本文だけになる。正規表現でJSXの本文を切り出す方式は
`{" "}` を挟む書き方（`apps/desktop/src/components/contextAngleSteps.tsx` の
「※折り線が見つからないため飛ばされています」）を取りこぼしたため使わない。

| 原典 | 対象 |
|---|---|
| ヘルプ内容JSON | `npm run export-help` の出力の全文字列。図はSVGの `<text>` `<tspan>` `<title>` `<desc>` だけ |
| 画面のTS/TSX | `apps/desktop/src/**` と `apps/web/src/**`（`*.test.*` `__fixtures__` `generated` を除く170ファイル） |
| 画面のCSS | 同10ファイル |
| HTML | `apps/desktop/index.html`、`apps/web/index.html` |
| 画面へ返るRust | `apps/desktop/src-tauri/src/**`（17ファイル）と `crates/*/src/**`（81ファイル）。`tests` `fixtures` は除く |
| 説明書の組版 | `crates/ori3-export/src/**`（16ファイル） |

集まったのは977字（うち日本語の字形853字）。追跡中の説明書PDF（82ページ）から取り出した699字も
全て含む（977字は2026-08-29の970字を1字も減らさずに含む）。
集め直しは `node apps/web/fonts/collect-charset.mjs`、照合だけなら `--check` を付ける。
Python版とNode版の独立2実装が同じ集合を出すことを確認済みである。

`#[cfg(test)]` の中の文言も `crates/*/src/**` や `apps/desktop/src-tauri/src/**` に
書かれていれば集まる。2026-09-04に増えた7字のうち6字（`α` `恒` `競` `義` `翼` `飾`）は
この経路で、残る1字（`ぬ`）はヘルプ本文から入った。画面へ出ない字が少し混じるが、
足りないより安全なので、この走査規則は変えていない。

**利用者が打ち込んだ文字は、はじめからこの集合の対象外である。** 覚え書きの入力欄と
ヘルプの検索欄には、同梱フォントを含まない別の並び `--font-user-text` を当ててある
（`apps/desktop/src/styles/base-layout.css` の `input[type="text"]` /
`input[type="search"]` / `textarea` / `.user-text`）。こうしておくと、1つの文字列の中で
同梱フォントと機械のフォントが混ざらない。`--font-user-text` は各テーマの `--font-body`
から同梱フォント1つを抜いただけの値で、機械側の並びも順序も変えていない。

`.user-text` は、利用者由来の**表示**（保存したファイル名など）へ後から付けるための受け口で、
いまはまだどの要素にも付けていない。文中のファイル名へ付けると文字列が要素で分かれ、
文をひとかたまりで探している既存の検査5件（`ContextPanel.dom` 2件、
`platformFileGateway.dom` 1件、`keyboardOnly.tenStates` 2件）が落ちる。実測で確かめてある。

同梱フォントは画面の枠組み（ボタン・見出し・案内文）のためのもので、
利用者の文字まで覆おうとするとJIS X 0208相当まで広げる必要があり、8.2倍の大きさになる。

## 字体の並びへの入れ方

読み込みは `apps/web/src/webShell.css` の `@font-face` だけで、別名 `"ORIGAMI3 JP"` を使う。
並びは `apps/desktop/src/styles/tokens.css` と `themes.css` の実体のある8か所へ1つ挿す。
入れる位置は総称名の種類で変える。

| 並びの種類 | 入れる位置 | 例 |
|---|---|---|
| ゴシック（`sans-serif`） | 総称名の**直前** | `"Yu Gothic UI", "Meiryo", "ORIGAMI3 JP", sans-serif` |
| 明朝（`serif`） | 総称名の**直後**（いちばん最後） | `Georgia, "Yu Mincho", serif, "ORIGAMI3 JP"` |

同梱フォントはゴシックなので、明朝の並びでは総称の `serif` を先に試させる。
ブラウザは字ごとに後ろへ落ちるため、明朝のある機械は明朝のまま出て、同梱フォントは
「どの明朝にも無い字」の最後の受け皿になる。追加の同梱は要らない。
どちらの場合も機械側のフォントより後ろに置くので、日本語フォントのある機械では
これまでどおりの見た目になる。デスクトップ版の画面はこのWeb用 `@font-face` を
読み込まないため表示は変わらない。一方、両版の折り図PDFは環境差を避けるため、
機械側のフォントを読まず、このsubsetだけを使う。

## 縮小版の作り直し方

道具はInkscape同梱のHarfBuzz 10.4.0（`C:\Program Files\Inkscape\bin\libharfbuzz-subset-0.dll`）。
`hb_subset_or_fail` に次を渡す。

- 残す文字: `apps/web/fonts/charset.txt` の977字
- 太さの軸: `hb_subset_input_set_axis_range(wght, 400, 700, 既定400)`
  （画面が使う太さは400・500・600・700の4つだけなので、100〜900は要らない）
- 残す名前: 既定の0〜6番に加えて7・8・9・11・13・14・16・17・25番

結果は977/977が入り、**欠落は0**である。FreeTypeで1字ずつ32pxへ実描画して、
空白字U+3000を除く黒画素0の字0件・送り幅0の字0件も確かめてある。
`name` 表には0・1・2・3・4・5・6・7・8・9・11・13・14・16・17・25番が残る。

上の3点をそのまま実行する道具を置いてある。元のフォントの実パスだけを渡す
（この作業機では `C:\Users\oltot\Documents\git-projects\ORIGAMI2\crates\ori-formats\assets\fonts\NotoSansJP-Variable.ttf`
が上の表と同じ9,589,900 B / `C2F3B4D4…` である。無ければ上の取得元から取り、SHA-256を確かめる）。

```powershell
node apps/web/fonts/collect-charset.mjs                                  # charset.txt を集め直す
python apps/web/fonts/make-subset.py --source <元のフォント>              # 縮小版を作り直す
python apps/web/fonts/verify-subset.py --source <元のフォント>            # 欠落0・実描画・名前を確かめる
```

`make-subset.py --check` は書き出さずに、いまの同梱物と一致するかだけを調べる。
2026-08-29の970字を渡すと `470,716 B` / `2FD9D3E2…` をbyteまで再現できることを確認済みで、
この手順が記録どおりであることの裏付けにしている。
`verify-subset.py` は描画に `freetype-py` を使う（無ければ `pip install --user freetype-py`）。
Inkscape同梱の `libfreetype-6.dll` を `ctypes` で直に叩くと `FT_FaceRec` の並びを手で写すことになり、
ずれても静かに違う結果が出るため使わない。

以前は `⌕`（U+2315）`⏮`（U+23EE）`⏸`（U+23F8）`✕`（U+2715）の4字を画面で使っていたが、
この4字は**元の9,589,900 Bのフォントにも入っていない**ため、機械のフォント次第で形が変わり、
記号フォントの無い機械では四角い箱になっていた。いまは字をやめて
`apps/desktop/src/components/UiIcon.tsx` の図（インラインSVG）で描いている。
読み上げのための名前は、隣の日本語の文言か `aria-label` がそのまま持っている。

太さの軸を100〜900のまま残すと464,204 B（名前を足す前の実測）になる。
文字を日本語の常用範囲まで広げた場合の目安は、957字にJIS X 0208相当の6,879字を足して
太さ400〜700で3,797,304 Bだった。どちらも今は採らない。
