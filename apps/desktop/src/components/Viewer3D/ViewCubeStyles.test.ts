import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const appCss = readFileSync(new URL("../../App.css", import.meta.url), "utf8").replace(
  /\r\n/g,
  "\n",
);

function cssDeclarations(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return appCss.match(new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`, "s"))?.[1] ?? "";
}

describe("視点立方体のCSS契約", () => {
  it("外接枠を3D区画の右上12px、96px四方に固定する", () => {
    const declarations = cssDeclarations(".view-cube");
    expect(declarations).toContain("top: 12px;");
    expect(declarations).toContain("right: 12px;");
    expect(declarations).toContain("width: 96px;");
    expect(declarations).toContain("height: 96px;");
    expect(declarations).toContain("position: absolute;");
  });

  it("テーマの既存色を使い、面が押せることをカーソルとhoverで示す", () => {
    const face = cssDeclarations(".view-cube-face");
    expect(face).toContain("background: var(--viewer-hint-background);");
    expect(face).toContain("color: var(--color-text);");
    expect(face).toContain("cursor: pointer;");
    expect(appCss).toMatch(/\.view-cube-face:hover,[\s\S]*background:\s*var\(--color-accent-soft\)/);
  });

  it("左上案内と条件付きの右上通知も120pxを予約する", () => {
    expect(cssDeclarations(".viewer-operation-hint")).toContain("right: 120px;");
    expect(cssDeclarations(".status-badge")).toContain("right: 120px;");
    expect(cssDeclarations(".suspect-hinge-guide")).toContain("right: 120px;");
  });
});
