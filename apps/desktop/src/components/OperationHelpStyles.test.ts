// 操作案内の開閉部品が狭い区画でも崩れないためのApp.css契約。

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// vitestは.cssのimportを空にするため、既存のuiTokens.test.tsと同じく直に読む。
const appCss = readFileSync(new URL("../App.css", import.meta.url), "utf8");

function cssDeclarations(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return appCss.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`, "s"))?.[1] ?? "";
}

describe("操作案内の狭幅CSS", () => {
  it("開閉ボタンを縮ませず、文言を途中で折り返さない", () => {
    const declarations = cssDeclarations(".operation-detail-toggle");
    expect(declarations).toContain("white-space: nowrap;");
    expect(declarations).toContain("flex-shrink: 0;");
    expect(declarations).toContain("max-width: 100%;");
    expect(declarations).toContain("overflow: hidden;");
    expect(declarations).toContain("text-overflow: ellipsis;");
  });

  it("畳んだ後に残る要点を1行に保つ", () => {
    const declarations = cssDeclarations(".operation-summary-line");
    expect(declarations).toContain("min-width: 0;");
    expect(declarations).toContain("white-space: nowrap;");
    expect(declarations).toContain("overflow: hidden;");
    expect(declarations).toContain("text-overflow: ellipsis;");
  });

  it("紙を選んだときの小さな吹き出しも親幅へ収める", () => {
    const declarations = cssDeclarations(".paper-action-tip.compact");
    expect(declarations).toContain("box-sizing: border-box;");
    expect(declarations).toContain("max-width: calc(100% - 40px);");
    expect(declarations).toContain("white-space: nowrap;");
    expect(declarations).toContain("overflow: hidden;");
    expect(declarations).toContain("text-overflow: ellipsis;");
  });

  it("2D・3D・下部パネルそれぞれの実幅で省スペース表示へ切り替える", () => {
    expect(appCss).toMatch(
      /\.pane-3d-view\s*\{[^}]*container:\s*viewer-operation-help\s*\/\s*inline-size/s,
    );
    expect(appCss).toMatch(
      /\.cp-editor\s*\{[^}]*container:\s*cp-operation-help\s*\/\s*inline-size/s,
    );
    expect(appCss).toMatch(
      /\.context-selection\s*\{[^}]*container:\s*context-operation-help\s*\/\s*inline-size/s,
    );
    expect(appCss).toMatch(/@container\s+viewer-operation-help\s*\(max-width:/);
    expect(appCss).toMatch(/@container\s+cp-operation-help\s*\(max-width:/);
    expect(appCss).toMatch(/@container\s+context-operation-help\s*\(max-width:/);
  });

  it("最狭3Dでは展開した2つの案内に別々の縦領域を確保する", () => {
    expect(appCss).toMatch(
      /@container\s+viewer-operation-help[\s\S]*\.viewer-operation-hint\.paper-action-tip-open[\s\S]*\.paper-action-tip\.expanded\s*\{[^}]*right:\s*var\(--sp-5\)[^}]*max-height:\s*calc\(100%\s*-\s*222px\)/,
    );
    expect(
      cssDeclarations(
        ".viewer-operation-hint.paper-action-tip-open .viewer-current-row",
      ),
    ).toContain("margin-top: 0;");
    expect(
      cssDeclarations(
        ".viewer-operation-hint.paper-action-tip-open .viewer-operation-heading,\n  .viewer-operation-hint.paper-action-tip-open .viewer-mouse-assignments",
      ),
    ).toContain("display: none;");
  });
});
