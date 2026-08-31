// ブラウザ版へ同梱する日本語フォントに残す文字を、原典から機械的に集める。
//
// 規則は「コメントを取り除いたソースに残る文字」。コメントを外せば、非ASCIIとして
// 残るのは文字列リテラルとJSXの本文だけになる。正規表現でJSXの本文を切り出す方式は
// {" "} を挟む書き方(apps/desktop/src/components/contextAngleSteps.tsx の
// 「※折り線が見つからないため飛ばされています」)を取りこぼしたので使わない。
//
// アプリ内ヘルプと取扱説明書の文言は apps/desktop/src/help/ の文字列リテラルが元なので、
// 画面のTS/TSXを走査すれば取扱説明書の字形も入る(実測: ヘルプJSON708字の全てが入る)。
//
// 使い方:
//   node apps/web/fonts/collect-charset.mjs          … 集めた文字を charset.txt へ書く
//   node apps/web/fonts/collect-charset.mjs --check  … charset.txt と一致するか調べる
//
// 書き直したら apps/web/fonts/README.md の手順でフォントを作り直すこと。

import { readFileSync, readdirSync, writeFileSync, statSync } from "node:fs";
import { join, extname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** 走査から外すもの。テスト・自動生成物・取り込み物・組み立て時だけの物は画面に出ない。 */
const EXCLUDED = [
  "node_modules",
  "/dist/",
  "/target/",
  "/__fixtures__/",
  "/generated/",
  "/tests/",
  "/fixtures/",
  ".test.",
  ".spec.",
  "/build.rs",
];

/** 走査するもの。第2要素はコメントの外し方。 */
const SOURCES = [
  ["apps/desktop/src", [".ts", ".tsx"], "js"],
  ["apps/desktop/src", [".css"], "css"],
  ["apps/web/src", [".ts", ".tsx"], "js"],
  ["apps/web/src", [".css"], "css"],
  ["apps/desktop/index.html", [".html"], "html"],
  ["apps/web/index.html", [".html"], "html"],
  ["apps/desktop/src-tauri/src", [".rs"], "rust"],
  ["crates", [".rs"], "rust"],
];

const QUOTES = new Set(['"', "'", "`"]);

/** JS/TSのコメントを外す。文字列の中の // や /* は外さない。 */
export function stripJsComments(source) {
  let out = "";
  let i = 0;
  while (i < source.length) {
    const c = source[i];
    if (QUOTES.has(c)) {
      let j = i + 1;
      while (j < source.length) {
        if (source[j] === "\\") {
          j += 2;
          continue;
        }
        if (source[j] === c) break;
        j += 1;
      }
      out += source.slice(i, j + 1);
      i = j + 1;
      continue;
    }
    if (c === "/" && source[i + 1] === "/") {
      const j = source.indexOf("\n", i);
      i = j < 0 ? source.length : j;
      continue;
    }
    if (c === "/" && source[i + 1] === "*") {
      const j = source.indexOf("*/", i + 2);
      i = j < 0 ? source.length : j + 2;
      continue;
    }
    out += c;
    i += 1;
  }
  return out;
}

/** Rustのコメントを外す。生文字列 r#"…"# も文字列として残す。 */
export function stripRustComments(source) {
  let out = "";
  let i = 0;
  while (i < source.length) {
    const c = source[i];
    if (c === "r" && (source[i + 1] === "#" || source[i + 1] === '"')) {
      const opener = /^r(#*)"/.exec(source.slice(i, i + 16));
      if (opener) {
        const closer = `"${opener[1]}`;
        const start = i + opener[0].length;
        const j = source.indexOf(closer, start);
        const end = j < 0 ? source.length : j + closer.length;
        out += source.slice(i, end);
        i = end;
        continue;
      }
    }
    if (c === '"') {
      let j = i + 1;
      while (j < source.length) {
        if (source[j] === "\\") {
          j += 2;
          continue;
        }
        if (source[j] === '"') break;
        j += 1;
      }
      out += source.slice(i, j + 1);
      i = j + 1;
      continue;
    }
    if (c === "/" && source[i + 1] === "/") {
      const j = source.indexOf("\n", i);
      i = j < 0 ? source.length : j;
      continue;
    }
    if (c === "/" && source[i + 1] === "*") {
      const j = source.indexOf("*/", i + 2);
      i = j < 0 ? source.length : j + 2;
      continue;
    }
    out += c;
    i += 1;
  }
  return out;
}

const STRIPPERS = {
  js: stripJsComments,
  rust: stripRustComments,
  css: (source) => source.replaceAll(/\/\*[\s\S]*?\*\//g, " "),
  html: (source) => source.replaceAll(/<!--[\s\S]*?-->/g, " "),
};

function excluded(path) {
  const normalized = path.replaceAll("\\", "/");
  return EXCLUDED.some((mark) => normalized.includes(mark));
}

function walk(path, suffixes, found) {
  if (excluded(path)) return;
  let info;
  try {
    info = statSync(path);
  } catch {
    return;
  }
  if (info.isDirectory()) {
    for (const name of readdirSync(path)) walk(join(path, name), suffixes, found);
    return;
  }
  if (suffixes.includes(extname(path).toLowerCase())) found.push(path);
}

/** 字形の要らない文字(制御文字・空白)を落とす。 */
function displayable(character) {
  const code = character.codePointAt(0) ?? 0;
  if (code < 0x20 || code === 0x7f) return false;
  if (character === " " || code === 0x00a0 || code === 0x200b) return false;
  if (code === 0xfeff) return false; // ファイル先頭のBOM。幅も字形も無い
  return true;
}

/** 原典を走査して、必要な文字を1行の文字列(コードポイント順)で返す。 */
export function collectCharacters(repositoryRoot) {
  const characters = new Set();
  const scanned = [];
  for (const [relative, suffixes, kind] of SOURCES) {
    const files = [];
    walk(resolve(repositoryRoot, relative), suffixes, files);
    files.sort();
    for (const file of files) {
      const text = STRIPPERS[kind](readFileSync(file, "utf8"));
      for (const character of text) {
        if (displayable(character)) characters.add(character);
      }
    }
    scanned.push(...files);
  }
  const sorted = [...characters].sort(
    (left, right) => (left.codePointAt(0) ?? 0) - (right.codePointAt(0) ?? 0),
  );
  return { text: sorted.join(""), fileCount: scanned.length };
}

/**
 * 集めた文字のうち、charset.txt に入っていないものを返す(コードポイント順)。
 * 空文字列なら足りない字は無い。ここを独立した関数にしてあるのは、
 * 「新しい字が増えたら気づけること」自体を検査で固定するためである
 * (apps/web/src/webFont.test.ts が、わざと1字欠けた集合を渡して確かめる)。
 */
export function charactersMissingFrom(storedCharset, collectedText) {
  const stored = new Set(storedCharset);
  const missing = [...new Set(collectedText)].filter((one) => !stored.has(one));
  missing.sort((left, right) => (left.codePointAt(0) ?? 0) - (right.codePointAt(0) ?? 0));
  return missing.join("");
}

const HERE = fileURLToPath(new URL(".", import.meta.url));
export const REPOSITORY_ROOT = resolve(HERE, "../../..");
export const CHARSET_PATH = resolve(HERE, "charset.txt");

function main() {
  const check = process.argv.includes("--check");
  const { text, fileCount } = collectCharacters(REPOSITORY_ROOT);
  if (check) {
    const stored = readFileSync(CHARSET_PATH, "utf8");
    if (stored === text) {
      console.log(`一致しました: ${text.length}字 / 走査${fileCount}ファイル`);
      return;
    }
    const missing = charactersMissingFrom(stored, text);
    const extra = charactersMissingFrom(text, stored);
    console.error(`charset.txt と一致しません。足りない字: 「${missing}」 余分な字: 「${extra}」`);
    process.exitCode = 1;
    return;
  }
  writeFileSync(CHARSET_PATH, text, { encoding: "utf8" });
  console.log(`${CHARSET_PATH} へ ${text.length}字を書きました / 走査${fileCount}ファイル`);
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))) {
  main();
}
