// 操作案内の開閉部品が狭い区画でも崩れないためのApp.css契約。

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// vitestは.cssのimportを空にするため、既存のuiTokens.test.tsと同じく直に読む。
const baseLayoutCss = readFileSync(
  new URL("../styles/base-layout.css", import.meta.url),
  "utf8",
).replace(/\r\n/g, "\n");
const viewerCss = readFileSync(
  new URL("../styles/viewer.css", import.meta.url),
  "utf8",
).replace(/\r\n/g, "\n");
const contextCss = readFileSync(
  new URL("../styles/context.css", import.meta.url),
  "utf8",
).replace(/\r\n/g, "\n");
const responsiveCss = readFileSync(
  new URL("../styles/responsive.css", import.meta.url),
  "utf8",
).replace(/\r\n/g, "\n");
const viewerStatusOverlaysSource = readFileSync(
  new URL("./ViewerStatusOverlays.tsx", import.meta.url),
  "utf8",
);
const viewerSource = readFileSync(
  new URL("./Viewer3D/Viewer3D.tsx", import.meta.url),
  "utf8",
);
const overlayStackSource = readFileSync(
  new URL("./Viewer3D/ViewerOverlayStack.tsx", import.meta.url),
  "utf8",
);

const baseSelectors = new Set([
  ".viewer-overlay-scroll-controls > button",
  ".suspect-hinge-guide",
]);

function cssDeclarations(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const ownerCss = baseSelectors.has(selector) ? baseLayoutCss : viewerCss;
  return (
    ownerCss.match(new RegExp(`(?:^|\\n)\\s*${escaped}\\s*\\{([^}]*)\\}`, "s"))?.[1] ??
    ""
  );
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
    expect(viewerCss).toMatch(
      /\.pane-3d-view\s*\{[^}]*container:\s*viewer-operation-help\s*\/\s*inline-size/s,
    );
    expect(viewerCss).toMatch(
      /\.cp-editor\s*\{[^}]*container:\s*cp-operation-help\s*\/\s*inline-size/s,
    );
    expect(contextCss).toMatch(
      /\.context-selection\s*\{[^}]*container:\s*context-operation-help\s*\/\s*inline-size/s,
    );
    expect(responsiveCss).toMatch(/@container\s+viewer-operation-help\s*\(max-width:/);
    expect(responsiveCss).toMatch(/@container\s+cp-operation-help\s*\(max-width:/);
    expect(responsiveCss).toMatch(/@container\s+context-operation-help\s*\(max-width:/);
  });

  it("3D幅にかかわらず通知・操作・紙・折り方の札を1本の縦列へ入れる", () => {
    expect(viewerSource).toContain("<ViewerOverlayStack>");
    expect(overlayStackSource).toContain('className="viewer-overlay-region"');
    expect(overlayStackSource).toContain('className="viewer-overlay-stack"');
    expect(viewerSource.indexOf("{statusOverlays}")).toBeLessThan(
      viewerSource.indexOf("<ViewerOperationHint"),
    );
    expect(viewerSource.indexOf("<ViewerOperationHint")).toBeLessThan(
      viewerSource.indexOf("<PaperActionTip"),
    );
    expect(viewerSource.indexOf("<PaperActionTip")).toBeLessThan(
      viewerSource.indexOf("<FoldDirectionTip"),
    );
    const region = cssDeclarations(".viewer-overlay-region");
    expect(region).toContain("width: min(430px, calc(100% - 164px));");
    expect(region).toContain("overflow: hidden;");
    expect(region).toContain("pointer-events: none;");
    const stack = cssDeclarations(".viewer-overlay-stack");
    expect(stack).toContain("display: flex;");
    expect(stack).toContain("inset: 0;");
    expect(stack).toContain("flex-direction: column;");
    expect(stack).toContain("overflow-y: auto;");
    expect(stack).toContain("pointer-events: none;");
    expect(responsiveCss).toMatch(
      /@container\s+viewer-operation-help\s*\(max-width:\s*520px\)[\s\S]*\.viewer-operation-hint\.collapsed \.viewer-current-row\s*\{[^}]*grid-template-columns:\s*auto minmax\(0, 1fr\) auto/,
    );
    expect(responsiveCss).toMatch(
      /\.viewer-operation-hint\.collapsed \.viewer-current-action\s*\{[^}]*grid-column:\s*1 \/ -1[^}]*white-space:\s*normal[^}]*overflow:\s*visible[^}]*text-overflow:\s*clip/,
    );
    expect(viewerCss).toMatch(
      /\.viewer-overlay-stack > \.viewer-operation-hint\.collapsed,[\s\S]*?\.viewer-overlay-stack > \.paper-action-tip\.expanded\s*\{[^}]*position:\s*relative[^}]*inset:\s*auto[^}]*width:\s*100%[^}]*max-width:\s*100%/,
    );
    expect(responsiveCss).toMatch(
      /@container\s+viewer-operation-help\s*\(max-width:\s*360px\)[\s\S]*\.viewer-overlay-region\s*\{[^}]*top:\s*148px[^}]*right:\s*var\(--sp-5\)[^}]*bottom:\s*56px/,
    );
    expect(viewerCss).toMatch(
      /\.viewer-overlay-stack > \.viewer-operation-hint::after\s*\{[^}]*display:\s*none/,
    );
    expect(viewerCss).toMatch(
      /\.viewer-overlay-stack > \.suspect-hinge-guide,[\s\S]*\.viewer-overlay-stack > \.paper-action-tip\s*\{[^}]*pointer-events:\s*auto/,
    );
    const stackedCards = viewerCss.match(
      /\.viewer-overlay-stack > \.status-badge,[\s\S]*?\.viewer-overlay-stack > \.paper-action-tip\.expanded\s*\{([^}]*)\}/,
    )?.[1];
    expect(stackedCards).toBeDefined();
    expect(stackedCards).not.toContain("pointer-events: auto");
  });

  it("高さ不足時だけ上下操作を出し、操作行は透過してボタンだけを押せる", () => {
    expect(overlayStackSource).toContain("new ResizeObserverClass(updateScrollState)");
    expect(overlayStackSource).toContain("new MutationObserverClass");
    expect(overlayStackSource).toContain("scrollState.overflowing ? (");
    expect(overlayStackSource).toContain('aria-label="3Dの案内を上へ送る"');
    expect(overlayStackSource).toContain('data-tooltip="3Dの案内を上へ送る"');
    expect(overlayStackSource).toContain('aria-label="3Dの案内を下へ送る"');
    expect(overlayStackSource).toContain('data-tooltip="3Dの案内を下へ送る"');
    expect(viewerCss).toMatch(
      /\.viewer-overlay-region\[data-overflow="true"\] \.viewer-overlay-stack\s*\{[^}]*bottom:\s*var\(--viewer-overlay-scroll-controls-height\)/,
    );
    expect(cssDeclarations(".viewer-overlay-scroll-controls")).toContain(
      "pointer-events: none;",
    );
    expect(
      cssDeclarations(".viewer-overlay-scroll-controls > button"),
    ).toContain("pointer-events: auto;");
  });

  it("上側の通知2種は補足を持ち、縦列では全文を折り返す", () => {
    const statusText = cssDeclarations(".status-badge > span");
    const suspect = cssDeclarations(".suspect-hinge-guide");
    for (const declarations of [statusText, suspect]) {
      expect(declarations).toContain("overflow: hidden;");
      expect(declarations).toContain("text-overflow: ellipsis;");
      expect(declarations).toContain("white-space: nowrap;");
    }
    expect(statusText).toContain("min-width: 0;");
    expect(viewerStatusOverlaysSource).toContain("data-tooltip={badgeText}");
    expect(viewerStatusOverlaysSource).toContain(
      'data-tooltip="赤く光る折り目の角度を見直してください。押すと原因候補を選びます"',
    );
    expect(viewerCss).toMatch(
      /\.viewer-overlay-stack > \.status-badge > span,[\s\S]*\.viewer-overlay-stack \.paper-action-tip-buttons > button\s*\{[^}]*overflow:\s*visible[^}]*text-overflow:\s*clip[^}]*white-space:\s*normal[^}]*overflow-wrap:\s*anywhere/,
    );
    expect(viewerCss).toMatch(
      /\.viewer-overlay-stack \.paper-action-tip\.expanded > strong\s*\{[^}]*padding-right:\s*calc\(28px \+ var\(--sp-3\)\)/,
    );
  });

});
