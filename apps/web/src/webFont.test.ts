// ブラウザ版へ同梱する日本語フォントの約束。
//
// 日本語フォントを持たない機械でも画面の文字が四角い箱にならないよう、必要な字だけを
// 残したフォントを1本同梱している。次の4つが崩れると、その機械で文字が読めなくなる。
//   1. 画面やヘルプへ新しい字が増えたのに、フォントを作り直していない
//   2. 「増えたことに気づく検査」自体が壊れて、黙って通るようになった
//   3. フォントの読み込み(@font-face)や字体の並びへの結線が外れた
//   4. 同梱したファイルが差し替わった、またはライセンス原文が一緒に配られなくなった
// 出所・利用条件・作り直し方は apps/web/fonts/README.md に記す。

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  CHARSET_PATH,
  REPOSITORY_ROOT,
  charactersMissingFrom,
  collectCharacters,
} from "../fonts/collect-charset.mjs";

const FONT_URL = "/fonts/NotoSansJP-ORIGAMI3-subset.ttf";
const FONT_FAMILY = "ORIGAMI3 JP";
const FONT_PATH = resolve(REPOSITORY_ROOT, "apps/web/public/fonts/NotoSansJP-ORIGAMI3-subset.ttf");
const LICENSE_PATH = resolve(REPOSITORY_ROOT, "apps/web/public/fonts/OFL.txt");

/** 同梱物の照合値。差し替えたら README の表と一緒に直す。 */
// 2026-09-04: ヘルプ本文とRustへ増えた7字(α ぬ 恒 競 義 翼 飾)を足し、970字→977字で作り直した。
// 旧値は 470,716 B / 2FD9D3E23D647236DA46D5F2EA11C00838F798B44FD1F4F030D0A2B37CA511C5。
const FONT_SHA256 = "3F9B664FABBFA1BA4032A169A1B9736F8D9E5B8C0E4058ACC037E6A286C5E514";
const FONT_BYTES = 475_640;
const LICENSE_SHA256 = "1C05C68C34F9708415AADA51F17E1B0092D2CEA709BF4A94CD38114F9E73D7D9";

/**
 * 元のNoto Sans JP 2.004-H2(9,589,900 B)自体に入っていない4字。
 * 画面ではこの4字を使うのをやめ、components/UiIcon.tsx の図に置き換えた。
 * 字として戻ってくると、その機械のフォント次第で形が変わってしまう。
 */
const REPLACED_BY_ICONS = ["⌕", "⏮", "⏸", "✕"];

/** 総称名。字体の並びは必ずこのどれかを含む。 */
const GENERIC_FAMILIES = ["sans-serif", "serif", "monospace", "cursive", "fantasy"];

function sha256(bytes: Buffer): string {
  return createHash("sha256").update(bytes).digest("hex").toUpperCase();
}

/** TrueTypeのcmap(形式4)を読んで、収録しているUnicodeの集合を返す。 */
function cmapCodepoints(font: Buffer): Set<number> {
  const tableCount = font.readUInt16BE(4);
  let cmapOffset = -1;
  for (let index = 0; index < tableCount; index += 1) {
    const record = 12 + 16 * index;
    if (font.toString("latin1", record, record + 4) === "cmap") {
      cmapOffset = font.readUInt32BE(record + 8);
    }
  }
  if (cmapOffset < 0) throw new Error("cmap表がありません");

  const subtableCount = font.readUInt16BE(cmapOffset + 2);
  let bmpOffset = -1;
  for (let index = 0; index < subtableCount; index += 1) {
    const record = cmapOffset + 4 + 8 * index;
    if (font.readUInt16BE(record) === 3 && font.readUInt16BE(record + 2) === 1) {
      bmpOffset = cmapOffset + font.readUInt32BE(record + 4);
    }
  }
  if (bmpOffset < 0) throw new Error("Windows BMPのcmap副表がありません");
  const format = font.readUInt16BE(bmpOffset);
  if (format !== 4) throw new Error(`cmapの形式が4ではありません: ${format}`);

  const segmentCount = font.readUInt16BE(bmpOffset + 6) / 2;
  const endBase = bmpOffset + 14;
  const startBase = endBase + segmentCount * 2 + 2;
  const deltaBase = startBase + segmentCount * 2;
  const rangeBase = deltaBase + segmentCount * 2;

  const covered = new Set<number>();
  for (let segment = 0; segment < segmentCount; segment += 1) {
    const end = font.readUInt16BE(endBase + segment * 2);
    const start = font.readUInt16BE(startBase + segment * 2);
    if (start === 0xffff) continue;
    const delta = font.readInt16BE(deltaBase + segment * 2);
    const rangeOffset = font.readUInt16BE(rangeBase + segment * 2);
    for (let code = start; code <= end; code += 1) {
      let glyph: number;
      if (rangeOffset === 0) {
        glyph = (code + delta) & 0xffff;
      } else {
        const at = rangeBase + segment * 2 + rangeOffset + (code - start) * 2;
        if (at + 2 > font.length) continue;
        glyph = font.readUInt16BE(at);
        if (glyph !== 0) glyph = (glyph + delta) & 0xffff;
      }
      if (glyph !== 0) covered.add(code);
    }
  }
  return covered;
}

function readText(...relative: string[]): string {
  return readFileSync(resolve(REPOSITORY_ROOT, ...relative), "utf8").replaceAll("\r\n", "\n");
}

interface FontStack {
  property: string;
  value: string;
  families: string[];
}

/** 総称名を含む、実体のある字体の並びだけを取り出す。 */
function concreteFontStacks(css: string, property: string): FontStack[] {
  const found: FontStack[] = [];
  const pattern = new RegExp(`(${property})\\s*:\\s*([^;]+);`, "g");
  for (const match of css.matchAll(pattern)) {
    const value = match[2].replaceAll(/\s+/g, " ").trim();
    const families = value.split(",").map((one) => one.trim());
    if (!families.some((one) => GENERIC_FAMILIES.includes(one))) continue;
    found.push({ property: match[1], value, families });
  }
  return found;
}

const charsetText = readFileSync(CHARSET_PATH, "utf8");
const fontBytes = readFileSync(FONT_PATH);
const webShellCss = readText("apps/web/src/webShell.css");
const tokensCss = readText("apps/desktop/src/styles/tokens.css");
const themesCss = readText("apps/desktop/src/styles/themes.css");
const baseLayoutCss = readText("apps/desktop/src/styles/base-layout.css");
const diagramPdfSource = readText("crates/ori3-export/src/pdf.rs");
const tauriConfig = JSON.parse(readText("apps/desktop/src-tauri/tauri.conf.json")) as {
  bundle: { resources: Record<string, string> };
};

describe("同梱する日本語フォント", () => {
  it("charset.txtが、いま画面へ出せる文字を全て挙げている", () => {
    const collected = collectCharacters(REPOSITORY_ROOT);
    expect(
      charactersMissingFrom(charsetText, collected.text),
      "画面かヘルプへ新しい字が増えました。" +
        "node apps/web/fonts/collect-charset.mjs を実行し、" +
        "apps/web/fonts/README.md の手順でフォントを作り直してください",
    ).toBe("");
    expect(charsetText).toBe(collected.text);
    expect(collected.fileCount).toBeGreaterThan(200);
  });

  it("「新しい字が増えたら気づく」検査そのものが働く", () => {
    // 上の検査が黙って通るようになると、四角い箱に気づけない。
    // わざと1字欠けた集合を渡し、その1字だけを言い当てることを確かめる。
    const collected = collectCharacters(REPOSITORY_ROOT);
    const sample = "鶴";
    expect(charsetText).toContain(sample);
    const holed = charsetText.replaceAll(sample, "");
    expect(charactersMissingFrom(holed, collected.text)).toBe(sample);
    expect(charactersMissingFrom(charsetText, collected.text)).toBe("");
    // 並び順ではなく集合で比べていること(同じ字を並べ替えても「増えた」と言わない)
    const reversed = [...charsetText].reverse().join("");
    expect(charactersMissingFrom(reversed, collected.text)).toBe("");
  });

  it("charset.txtは改行を含まない1行で、コードポイント順に並ぶ", () => {
    expect(charsetText).not.toMatch(/[\r\n]/);
    const sorted = [...charsetText].sort(
      (left, right) => (left.codePointAt(0) ?? 0) - (right.codePointAt(0) ?? 0),
    );
    expect(charsetText).toBe(sorted.join(""));
    expect(new Set(charsetText).size).toBe([...charsetText].length);
  });

  it("元フォントに無い4字は、字ではなく図で出している", () => {
    for (const one of REPLACED_BY_ICONS) {
      expect(charsetText, `${one} が画面へ戻っています`).not.toContain(one);
    }
    const icon = readText("apps/desktop/src/components/UiIcon.tsx");
    for (const name of ["search", "skip-to-start", "pause", "close"]) {
      expect(icon).toContain(name);
    }
    expect(icon).toContain('aria-hidden="true"');
    // 押しボタンの名前は日本語の文言か aria-label が持つ(読み上げが消えない)
    expect(readText("apps/desktop/src/components/Timeline.tsx")).toContain(
      '<UiIcon name="skip-to-start" /> 最初へ',
    );
    expect(readText("apps/desktop/src/components/dialogs/HelpCenter.tsx")).toContain(
      '<UiIcon name="search" />',
    );
    expect(readText("apps/desktop/src/components/dialogs/ProposalWizard.tsx")).toContain(
      '<UiIcon name="close" />',
    );
  });

  it("同梱フォントがcharset.txtの字を1つ残らず収録する", () => {
    const covered = cmapCodepoints(fontBytes);
    const uncovered = [...charsetText].filter((one) => !covered.has(one.codePointAt(0) ?? 0));
    expect(uncovered.join("")).toBe("");
  });

  it("同梱フォントとライセンス原文が、記録どおりのファイルである", () => {
    expect(fontBytes.byteLength).toBe(FONT_BYTES);
    expect(sha256(fontBytes)).toBe(FONT_SHA256);
    const license = readFileSync(LICENSE_PATH);
    expect(sha256(license)).toBe(LICENSE_SHA256);
    const licenseText = license.toString("utf8");
    expect(licenseText).toContain("SIL OPEN FONT LICENSE Version 1.1");
    expect(licenseText).toContain("Reserved Font Name");
  });

  it("ライセンス原文がフォントと同じ場所から配られる", () => {
    // OFL条件2は、どの複製にも著作権表示とライセンス本文が付くことを求める。
    // publicの中身は組み立て後 dist/fonts/ へそのまま入り、/fonts/OFL.txt で配られる。
    expect(LICENSE_PATH.replaceAll("\\", "/")).toContain("apps/web/public/fonts/OFL.txt");
    const readme = readText("apps/web/fonts/README.md");
    expect(readme).toContain(FONT_SHA256);
    expect(readme).toContain("C2F3B4D463500A2DDCD3849CDED1FCEEB9FD6D1C32E6CBECD568453BA50FC68F");
    expect(readme).toContain("SIL Open Font License 1.1");
  });

  it("両版の折り図PDFが同じ同梱書体だけを使い、desktopにもOFLを付ける", () => {
    expect(diagramPdfSource).toContain(
      "../../../apps/web/public/fonts/NotoSansJP-ORIGAMI3-subset.ttf",
    );
    expect(diagramPdfSource).not.toContain("load_system_fonts");
    expect(tauriConfig.bundle.resources["../../web/public/fonts/OFL.txt"]).toBe(
      "licenses/NotoSansJP-OFL.txt",
    );
  });

  it("ブラウザ版のCSSが、そのファイルを別名で読み込む", () => {
    expect(webShellCss).toContain("@font-face");
    expect(webShellCss).toContain(`font-family: "${FONT_FAMILY}";`);
    expect(webShellCss).toContain(`url("${FONT_URL}") format("truetype")`);
    // 画面が使う太さは400〜700だけ。可変フォントの軸もその範囲で絞ってある。
    expect(webShellCss).toContain("font-weight: 400 700;");
    expect(webShellCss).toContain("font-display: swap;");
  });

  it("字体の並びで、同梱フォントが総称名の正しい側に入る", () => {
    const stacks = [
      ...concreteFontStacks(tokensCss, "--[a-z-]*font[a-z-]*"),
      ...concreteFontStacks(themesCss, "--[a-z-]*font[a-z-]*"),
    ].filter(({ property }) => property !== "--font-user-text");
    expect(stacks.length).toBeGreaterThanOrEqual(8);
    for (const { property, value, families } of stacks) {
      const bundled = families.indexOf(`"${FONT_FAMILY}"`);
      const generic = families.findIndex((one) => GENERIC_FAMILIES.includes(one));
      expect(families.filter((one) => one === `"${FONT_FAMILY}"`)).toHaveLength(1);
      // 機械側のフォントを先に使う。同梱分は受け皿にする。
      expect(bundled, `${property}: ${value}`).toBeGreaterThan(0);
      if (families[generic] === "sans-serif") {
        // ゴシックの並びでは、総称のゴシックより先に同梱フォントを使う。
        expect(bundled, `${property}: ${value}`).toBe(generic - 1);
        expect(generic, `${property}: ${value}`).toBe(families.length - 1);
      } else {
        // 明朝の並びでは、総称の明朝を先に試し、同梱のゴシックは最後の受け皿にする。
        expect(bundled, `${property}: ${value}`).toBe(generic + 1);
        expect(bundled, `${property}: ${value}`).toBe(families.length - 1);
      }
    }
  });

  it("利用者が打ち込む文字と利用者由来の表示は、機械のフォントだけを使う", () => {
    for (const [name, css] of [
      ["tokens.css", tokensCss],
      ["themes.css", themesCss],
    ] as const) {
      const bodies = concreteFontStacks(css, "--font-body");
      const users = concreteFontStacks(css, "--font-user-text");
      expect(users.length, `${name} の --font-user-text の数`).toBe(bodies.length);
      for (const [index, body] of bodies.entries()) {
        const user = users[index];
        expect(user.families).not.toContain(`"${FONT_FAMILY}"`);
        // 同梱フォントを1つ抜いただけで、機械側の並びも順序も変えていない
        expect(user.value).toBe(
          body.families.filter((one) => one !== `"${FONT_FAMILY}"`).join(", "),
        );
      }
    }
    expect(baseLayoutCss).toContain("font-family: var(--font-user-text);");
    for (const selector of [
      'input[type="text"]',
      'input[type="search"]',
      "textarea",
      ".user-text",
    ]) {
      expect(baseLayoutCss).toContain(selector);
    }
    // 覚え書きの入力欄は input[type="text"] なので、上の並びに自動で乗る
    expect(readText("apps/desktop/src/components/contextAngleSteps.tsx")).toContain(
      'className="note-input"',
    );
    // ヘルプの検索欄も input[type="search"] で同じ並びに乗る
    expect(readText("apps/desktop/src/components/dialogs/HelpCenter.tsx")).toContain(
      'type="search"',
    );
  });

  it("同梱フォントの配布先が、内容保護方針(CSP)の font-src に収まる", () => {
    const headers = readText("apps/web/public/_headers");
    expect(headers).toContain("font-src 'self'");
    expect(FONT_URL.startsWith("/")).toBe(true);
  });
});
