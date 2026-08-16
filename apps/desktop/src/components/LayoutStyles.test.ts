// 画面の崩れ(列のずれ・ボタンのくっつき・文字の詰まり)を防ぐ App.css の約束。
// 2026-08-17の点検で見つけた30件のうち、CSSで直したものが元へ戻らないようにする。
// 読み方は既存の OperationHelpStyles.test.ts / ViewCubeStyles.test.ts と同じ。

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { UI_THEMES } from "../lib/displayPrefs";

const appCss = readFileSync(new URL("../App.css", import.meta.url), "utf8").replace(
  /\r\n/g,
  "\n",
);

function cssDeclarations(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return appCss.match(new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`, "s"))?.[1] ?? "";
}

/** 注釈を外し、宣言の中に出てくる生のpx値だけを拾う。 */
function rawPixelDeclarations(property: string): string[] {
  const withoutComments = appCss.replace(/\/\*[\s\S]*?\*\//g, "\n");
  const found: string[] = [];
  for (const match of withoutComments.matchAll(
    new RegExp(`^\\s*${property}\\s*:\\s*([^;]+);`, "gm"),
  )) {
    const value = match[1].trim();
    if (value.includes("var(--")) continue;
    if (/^0(px)?$/.test(value)) continue;
    if (/-?\d+(\.\d+)?px/.test(value)) found.push(`${property}: ${value}`);
  }
  return found;
}

describe("行の列ぞろえ", () => {
  it("行の先頭ラベルは共通の列幅トークンで決める", () => {
    const root = cssDeclarations(":root");
    expect(root).toContain("--row-label-width:");
    expect(root).toContain("--tip-label-width:");

    const rowLabel = cssDeclarations(".row-label");
    expect(rowLabel).toContain("min-width: var(--row-label-width);");
    expect(rowLabel).toContain("flex: none;");

    // 3Dへ重ねる狭い札だけ専用の幅にする。
    expect(cssDeclarations(".fold-direction-tip-buttons > .row-label")).toContain(
      "min-width: var(--tip-label-width);",
    );
  });

  it("幅は1か所だけで決め、各所へ書き足さない", () => {
    const uses = appCss.match(/var\(--row-label-width\)/g) ?? [];
    expect(uses.length).toBe(1);
  });
});

describe("文字と枠・隣の要素の隙間", () => {
  it("丸印・チェックと言葉の間に隙間を入れる", () => {
    const declarations = cssDeclarations(
      '.app label > input\\[type="radio"\\]',
    );
    // 実際の宣言は checkbox と radio をまとめて書いてある。
    expect(appCss).toMatch(
      /\.app label > input\[type="checkbox"\],\s*\n\.app label > input\[type="radio"\] \{[^}]*margin-inline: 0 var\(--sp-3\);/s,
    );
    expect(declarations === "" || declarations.includes("margin-inline")).toBe(true);
  });

  it("重ねる札のボタンは下のパネルと同じ隙間にする", () => {
    expect(cssDeclarations(".paper-action-tip-buttons")).toContain("gap: var(--sp-3);");
    expect(cssDeclarations(".button-row")).toContain("gap: var(--sp-3);");
  });

  it("増減つまみの2つのボタンは入力欄との隙間と同じにする", () => {
    expect(cssDeclarations(".number-stepper")).toContain("gap: var(--sp-2);");
    expect(cssDeclarations(".number-stepper-controls")).toContain("gap: var(--sp-2);");
  });

  it("札の中の行は親のgapで隙間を作る", () => {
    const expanded = cssDeclarations(".paper-action-tip.expanded");
    expect(expanded).toContain("display: flex;");
    expect(expanded).toContain("flex-direction: column;");
    expect(expanded).toContain("gap: var(--sp-4);");
  });
});

describe("余白は設計トークンで決める", () => {
  it.each(["gap", "row-gap", "column-gap"])(
    "%s に生のpx値を書かない",
    (property) => {
      expect(rawPixelDeclarations(property)).toEqual([]);
    },
  );
});

describe("日本語の折り返し", () => {
  it("一括指定の text-wrap は使わない(white-space を打ち消すため)", () => {
    const withoutComments = appCss.replace(/\/\*[\s\S]*?\*\//g, "\n");
    expect(withoutComments).not.toMatch(/^\s*text-wrap\s*:/m);
  });

  it("説明文は pretty、短い名前は balance でそろえる", () => {
    expect(appCss).toMatch(/\.app p,[\s\S]*?text-wrap-style: pretty;/);
    expect(appCss).toMatch(/\.align-mode-buttons button,[\s\S]*?text-wrap-style: balance;/);
    // 日本語は語の間に空白が無いので、文節で折り返す指定も一緒に置く。
    expect(appCss).toMatch(/\.app p,[\s\S]*?word-break: auto-phrase;/);
  });
});

describe("3Dの操作案内はたためる", () => {
  it("たたむと中身の幅の1行になり、区画をほとんど取らない", () => {
    const collapsed = cssDeclarations(".viewer-operation-hint.collapsed");
    expect(collapsed).toContain("width: max-content;");
    expect(collapsed).toContain("right: auto;");
    expect(collapsed).toContain("padding: var(--sp-2) var(--sp-3);");
    expect(collapsed).toContain("border-radius: var(--radius-pill);");

    // たたんだ行は「絵・モード名・要点・開閉」の4列。
    expect(cssDeclarations(".viewer-operation-hint.collapsed .viewer-current-row")).toContain(
      "grid-template-columns: auto auto minmax(0, 1fr) auto;",
    );
  });

  it("開いた札では、いまできることを最後まで読める", () => {
    const expanded = cssDeclarations(".viewer-operation-hint.expanded .viewer-current-action");
    expect(expanded).toContain("white-space: normal;");
    expect(expanded).toContain("overflow: visible;");
  });

  it("たたんだ札にモード名の場所がある", () => {
    expect(cssDeclarations(".viewer-mode-name")).toContain("white-space: nowrap;");
    expect(cssDeclarations(".viewer-mode-icon.compact")).toContain("width: 24px;");
  });
});

describe("2Dの操作案内", () => {
  it("狭い区画でも中身の幅で止まり、右端まで伸びない", () => {
    expect(appCss).toMatch(
      /@container cp-operation-help \(max-width: 520px\) \{[\s\S]*?\.cp-operation-hint \{[^}]*width: max-content;/,
    );
  });
});

describe("道具レール", () => {
  it("窓が低いときは道具10個が収まる大きさへ詰める", () => {
    expect(appCss).toMatch(/@media \(max-height: 900px\) \{[\s\S]*?\.tool-button \{[^}]*min-height: 44px;/);
    expect(appCss).toMatch(/@media \(max-height: 760px\) \{[\s\S]*?\.tool-button \{[^}]*min-height: 36px;/);
  });

  it("サブメニューの道具はレールの道具と同じ「絵が上・言葉が下」にする", () => {
    const small = cssDeclarations(".tool-button.small");
    expect(small).toContain("flex-direction: column;");
    expect(cssDeclarations(".tool-submenu")).toContain("padding: var(--sp-2) 0;");
  });
});

describe("すべてのテーマで同じ約束が効く", () => {
  // ラベルの列幅はテーマごとに上書きしない。上書きすると崩れ方がテーマごとに
  // 変わり、1か所で直せなくなる。
  // 余白の段階(--sp-*)はモダンだけがわざと広くしており、これは意匠として認める。
  const columnTokens = ["--row-label-width", "--tip-label-width"];

  it.each(UI_THEMES.filter((theme) => theme !== "pop"))(
    "%s テーマはラベルの列幅を上書きしない",
    (theme) => {
      const block = cssDeclarations(`.app[data-theme="${theme}"]`);
      expect(block).not.toBe("");
      for (const token of columnTokens) {
        expect(block).not.toContain(`${token}:`);
      }
    },
  );

  it("ラベルの列幅は5テーマでいちばん広い文字でも収まる値にする", () => {
    // 実測(実機・各テーマの本文書体): いちばん長いラベル「紙の形を変える」は
    // ポップ80.5 / シンプル79.5 / 和風81.3 / モダン89.3 / クラシック80.5px。
    // いちばん広いモダンの89.3pxを下回ると、そのテーマだけ列がずれる。
    const root = cssDeclarations(":root");
    const rowWidth = Number(root.match(/--row-label-width:\s*(\d+)px/)?.[1]);
    expect(rowWidth).toBeGreaterThanOrEqual(90);

    // 3Dの札は「動かす側」が最長。実測の最大はモダンの51.2px。
    const tipWidth = Number(root.match(/--tip-label-width:\s*(\d+)px/)?.[1]);
    expect(tipWidth).toBeGreaterThanOrEqual(52);
  });
});
