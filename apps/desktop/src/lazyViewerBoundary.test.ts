import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { describe, expect, it } from "vitest";

function source(relativePath: string): string {
  return readFileSync(fileURLToPath(new URL(relativePath, import.meta.url).href), "utf8");
}

function parsed(relativePath: string): ts.SourceFile {
  return ts.createSourceFile(
    relativePath,
    source(relativePath),
    ts.ScriptTarget.Latest,
    true,
    relativePath.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
  );
}

function staticImports(relativePath: string): string[] {
  return parsed(relativePath).statements.flatMap((statement) =>
    ts.isImportDeclaration(statement) && ts.isStringLiteral(statement.moduleSpecifier)
      ? [statement.moduleSpecifier.text]
      : [],
  );
}

function dynamicImports(relativePath: string): string[] {
  const imports: string[] = [];
  const visit = (node: ts.Node) => {
    if (
      ts.isCallExpression(node) &&
      node.expression.kind === ts.SyntaxKind.ImportKeyword &&
      node.arguments.length === 1 &&
      ts.isStringLiteral(node.arguments[0])
    ) {
      imports.push(node.arguments[0].text);
    }
    ts.forEachChild(node, visit);
  };
  visit(parsed(relativePath));
  return imports;
}

describe("6-E 3D遅延読込の静的境界", () => {
  it("Appは軽量外枠だけを読み、外枠がViewer本体を1回だけdynamic importする", () => {
    expect(staticImports("./App.tsx")).not.toContain(
      "./components/Viewer3D/Viewer3D",
    );
    expect(staticImports("./App.tsx")).toContain(
      "./components/Viewer3D/DeferredViewer3D",
    );
    expect(dynamicImports("./App.tsx")).not.toContain(
      "./components/Viewer3D/Viewer3D",
    );
    expect(staticImports("./components/Viewer3D/DeferredViewer3D.tsx")).not.toContain(
      "./Viewer3D",
    );
    expect(
      dynamicImports("./components/Viewer3D/DeferredViewer3D.tsx").filter(
        (specifier) => specifier === "./Viewer3D",
      ),
    ).toHaveLength(1);
  });

  it("captureApiはThree側でなく軽量bridgeだけからreadbackを読む", () => {
    const imports = staticImports("./captureApi.ts");
    expect(imports).toContain("./captureReadbackBridge");
    expect(
      imports.filter(
        (specifier) =>
          specifier.includes("components/Viewer3D") ||
          specifier.includes("sceneBuilder") ||
          specifier.includes("sceneFacade") ||
          specifier === "three",
      ),
    ).toEqual([]);
  });

  it("軽量bridgeはViewer・scene・Threeへ逆参照しない", () => {
    expect(
      staticImports("./captureReadbackBridge.ts").filter(
        (specifier) =>
          specifier.includes("Viewer3D") ||
          specifier.includes("scene") ||
          specifier === "three" ||
          specifier.startsWith("three/"),
      ),
    ).toEqual([]);
  });

  it("製品のcreateScene呼出しはviewerLifecycleの1か所だけ", () => {
    const productSources = import.meta.glob(
      "./components/Viewer3D/*.{ts,tsx}",
      { query: "?raw", import: "default", eager: true },
    ) as Record<string, string>;
    const callOwners = Object.entries(productSources)
      .filter(([path]) => !/\.(?:test|spec)\./.test(path))
      .flatMap(([path, productSource]) => {
        let calls = 0;
        const visit = (node: ts.Node) => {
          if (
            ts.isCallExpression(node) &&
            ts.isIdentifier(node.expression) &&
            node.expression.text === "createScene"
          ) {
            calls += 1;
          }
          ts.forEachChild(node, visit);
        };
        visit(
          ts.createSourceFile(
            path,
            productSource,
            ts.ScriptTarget.Latest,
            true,
            path.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
          ),
        );
        const name = path.slice(path.lastIndexOf("/") + 1);
        return Array.from({ length: calls }, () => name);
      });
    expect(callOwners).toEqual(["viewerLifecycle.ts"]);
  });

  it("build自身が採用済みの二層bundle境界を検査する", () => {
    const config = source("../vite.config.ts");
    expect(config).toContain("const INITIAL_GZIP_LIMIT = 250_000;");
    expect(config).toContain("const MAX_RAW_CHUNK_LIMIT = 500_000;");
    expect(config).toContain("bundleBudget()");
    for (const chunk of [
      "three-core",
      "three-module",
      "three-examples",
      "react-vendor",
    ]) {
      expect(config).toContain(`return "${chunk}";`);
    }
    expect(config).toContain("/node_modules/three/");
    expect(config).toContain("/src/components/Viewer3D/");
  });
});
