import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as ts from "typescript";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  ALIGN_STEPS,
  type AlignMode,
  type AlignTarget,
} from "../../lib/alignFold";
import type { DocumentView, Vec2 } from "../../lib/types";

vi.mock("../../ipc/client", () => ({
  documentNew: vi.fn(),
  documentOpen: vi.fn(),
  documentSave: vi.fn(),
  editApply: vi.fn(),
  editApplyBatch: vi.fn(),
  editUndo: vi.fn(),
  editRedo: vi.fn(),
  sequenceApply: vi.fn(),
  sequenceReplay: vi.fn(),
  poseSolve: vi.fn(),
  recoveryCheck: vi.fn(),
  recoveryRestore: vi.fn(),
  proposalGenerate: vi.fn(),
  proposalProgress: vi.fn(),
  proposalControl: vi.fn(),
}));

import * as ipc from "../../ipc/client";
import { useAppStore } from "../../store/appStore";

const EXPECTED_ALIGN_MODES = [
  "throughTwoPoints",
  "pointPoint",
  "lineLine",
  "pointPerpendicularLine",
  "pointLineThrough",
  "pointToLinePointToLine",
  "pointLinePerpendicular",
  "existingLine",
] as const satisfies readonly AlignMode[];

const X_AXIS = {
  kind: "line" as const,
  a: [0, 0] as Vec2,
  b: [1, 0] as Vec2,
};
const Y_AXIS = {
  kind: "line" as const,
  a: [0, 0] as Vec2,
  b: [0, 1] as Vec2,
};
const TOP_AXIS = {
  kind: "line" as const,
  a: [0, 1] as Vec2,
  b: [1, 1] as Vec2,
};

/** 8つの合わせ方を、折り線が一意に求まる既存の代表入力で通す。 */
const ALIGN_CASES: readonly {
  mode: AlignMode;
  picks: readonly AlignTarget[];
}[] = [
  {
    mode: "throughTwoPoints",
    picks: [
      { kind: "point", p: [0, 0] },
      { kind: "point", p: [1, 0] },
    ],
  },
  {
    mode: "pointPoint",
    picks: [
      { kind: "point", p: [0, 0] },
      { kind: "point", p: [1, 0] },
    ],
  },
  { mode: "lineLine", picks: [X_AXIS, TOP_AXIS] },
  {
    mode: "pointPerpendicularLine",
    picks: [{ kind: "point", p: [0.25, 0.5] }, X_AXIS],
  },
  {
    mode: "pointLineThrough",
    picks: [
      { kind: "point", p: [0, 2] },
      X_AXIS,
      { kind: "point", p: [0, 1] },
    ],
  },
  {
    mode: "pointToLinePointToLine",
    picks: [
      { kind: "point", p: [0, 1] },
      X_AXIS,
      { kind: "point", p: [0, 0] },
      TOP_AXIS,
    ],
  },
  {
    mode: "pointLinePerpendicular",
    picks: [{ kind: "point", p: [0, 1] }, X_AXIS, Y_AXIS],
  },
  { mode: "existingLine", picks: [X_AXIS] },
];

const TEST_DIR = dirname(fileURLToPath(import.meta.url));

function source(relativePath: string): string {
  return readFileSync(join(TEST_DIR, relativePath), "utf8");
}

function visit(node: ts.Node, callback: (node: ts.Node) => void): void {
  callback(node);
  ts.forEachChild(node, (child) => visit(child, callback));
}

function functionBody(relativePath: string, name: string): string {
  const text = source(relativePath);
  const file = ts.createSourceFile(
    relativePath,
    text,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let body: ts.Block | null = null;
  visit(file, (node) => {
    if (
      body === null &&
      ts.isFunctionDeclaration(node) &&
      node.name?.text === name &&
      node.body
    ) {
      body = node.body;
    }
  });
  if (body === null) throw new Error(`missing function: ${name}`);
  return (body as ts.Block).getText(file);
}

function arrayInitializerStrings(relativePath: string, name: string): string[] {
  const text = source(relativePath);
  const file = ts.createSourceFile(
    relativePath,
    text,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let values: string[] | null = null;
  visit(file, (node) => {
    if (
      values === null &&
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      node.name.text === name &&
      node.initializer &&
      ts.isArrayLiteralExpression(node.initializer)
    ) {
      values = node.initializer.elements.map((element) => {
        if (!ts.isStringLiteral(element)) {
          throw new Error(`${name} contains a non-string member`);
        }
        return element.text;
      });
    }
  });
  if (values === null) throw new Error(`missing array: ${name}`);
  return values;
}

function rustStructBlock(rust: string, name: string): string {
  const start = rust.indexOf(`pub struct ${name} {`);
  if (start < 0) throw new Error(`missing Rust struct: ${name}`);
  const end = rust.indexOf("\n}", start);
  if (end < 0) throw new Error(`unterminated Rust struct: ${name}`);
  return rust.slice(start, end + 2);
}

function rustPublicFields(block: string): string[] {
  return [...block.matchAll(/^\s*pub\s+([A-Za-z_][A-Za-z0-9_]*)\s*:/gm)].map(
    (match) => match[1],
  );
}

function topPleatTypeDiagnostics(): string[] {
  const probePath = join(
    TEST_DIR,
    "../../__topPleatSelectionContractProbe.ts",
  );
  const probe = `
import type { GrabSelection } from "./components/Viewer3D/grabFold";
import type { FoldTargetSelection } from "./store/appStore";

const validGrab: GrabSelection = { mode: "topPleats", topPleatCount: 2 };
const validFold: FoldTargetSelection = { target: "topPleats", topPleatCount: 2 };
const missingGrabCount: GrabSelection = { mode: "topPleats" };
const missingFoldCount: FoldTargetSelection = { target: "topPleats" };
void [validGrab, validFold, missingGrabCount, missingFoldCount];
`;
  const options: ts.CompilerOptions = {
    target: ts.ScriptTarget.ES2020,
    module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    jsx: ts.JsxEmit.ReactJSX,
    strict: true,
    noEmit: true,
    skipLibCheck: true,
  };
  const host = ts.createCompilerHost(options, true);
  const originalFileExists = host.fileExists.bind(host);
  const originalReadFile = host.readFile.bind(host);
  const originalGetSourceFile = host.getSourceFile.bind(host);
  const pathKey = (fileName: string): string =>
    fileName.replace(/\\/g, "/").toLowerCase();
  const isProbe = (fileName: string) => pathKey(fileName) === pathKey(probePath);
  host.fileExists = (fileName) => isProbe(fileName) || originalFileExists(fileName);
  host.readFile = (fileName) => (isProbe(fileName) ? probe : originalReadFile(fileName));
  host.getSourceFile = (fileName, languageVersion, onError, shouldCreateNewSourceFile) =>
    isProbe(fileName)
      ? ts.createSourceFile(fileName, probe, languageVersion, true, ts.ScriptKind.TS)
      : originalGetSourceFile(
          fileName,
          languageVersion,
          onError,
          shouldCreateNewSourceFile,
        );
  const program = ts.createProgram({ rootNames: [probePath], options, host });
  return ts
    .getPreEmitDiagnostics(program)
    .filter((diagnostic) => diagnostic.file && isProbe(diagnostic.file.fileName))
    .map(
      (diagnostic) =>
        `${diagnostic.code}: ${ts.flattenDiagnosticMessageText(diagnostic.messageText, " ")}`,
    );
}

/** 単位正方形1枚・手順1つの、合わせ折りを確定できる状態。 */
function seedFlat(): void {
  const doc: DocumentView["doc"] = {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: {
      vertices: [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [1, 0] },
        { id: 2, pos: [1, 1] },
        { id: 3, pos: [0, 1] },
      ],
      edges: [
        { id: 0, v0: 0, v1: 1, kind: "Border" },
        { id: 1, v0: 1, v1: 2, kind: "Border" },
        { id: 2, v0: 2, v1: 3, kind: "Border" },
        { id: 3, v0: 3, v1: 0, kind: "Border" },
      ],
      next_vertex_id: 4,
      next_edge_id: 4,
    },
    sequence: [
      { id: 1, kind: "Simple", drivers: [], layer_order: null, note: "" },
    ],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
  useAppStore.setState({
    doc,
    faces: [{ id: 0, vertices: [0, 1, 2, 3], edges: [0, 1, 2, 3] }],
    hinges: new Set<number>(),
    activeTool: "fold",
    currentStep: null,
    playT: 1,
    playing: false,
    drivers: new Map(),
    foldDraft: null,
    pendingFoldThrough: null,
    foldThroughBusy: false,
    alignDraft: null,
    errorMessage: null,
    frame3d: {
      faces: [
        {
          face: 0,
          polygon: [
            [0, 0, 0],
            [1, 0, 0],
            [1, 1, 0],
            [0, 1, 0],
          ],
          layer: 0,
        },
      ],
      warnings: [],
    },
  });
  vi.mocked(ipc.sequenceApply).mockResolvedValue({
    doc,
    faces: [],
    warnings: [],
    violations: [],
    frame: { faces: [], warnings: [] },
    skipped: [],
    contact_detected: false,
  });
}

function completeAlign(mode: AlignMode, picks: readonly AlignTarget[]): void {
  useAppStore.getState().beginAlign(mode);
  for (const pick of picks) useAppStore.getState().pickAlignTarget(pick);
  expect(useAppStore.getState().foldDraft, `${mode}で折り線が求まる`).not.toBeNull();
}

function hasOwn(value: object, key: PropertyKey): boolean {
  return Object.prototype.hasOwnProperty.call(value, key);
}

beforeEach(() => {
  vi.clearAllMocks();
  seedFlat();
});

describe("上からKひだを選ぶ境界", () => {
  it("topPleatsはK無しで組み立てられず、K付きだけを型が受け入れる", () => {
    expect(topPleatTypeDiagnostics()).toEqual([
      expect.stringMatching(/^\d+: .*topPleatCount/),
      expect.stringMatching(/^\d+: .*topPleatCount/),
    ]);
  });

  it("K指定はPreviewとApplyへKだけを送り、面IDを送らない", async () => {
    completeAlign("pointPoint", [
      { kind: "point", p: [0, 0] },
      { kind: "point", p: [1, 0] },
    ]);
    // 実装前にもwire契約を赤くできるよう、型の合否自体は上のcompiler検査へ分離する。
    useAppStore.getState().updateFoldDraft({
      target: "topPleats",
      topPleatCount: 2,
    } as never);

    await useAppStore.getState().commitFoldDraft();

    const operations = vi.mocked(ipc.sequenceApply).mock.calls.map(([op]) => op);
    expect(operations.map((operation) => operation.type)).toEqual([
      "PreviewFoldThrough",
      "FoldThrough",
    ]);
    for (const operation of operations) {
      if (operation.type !== "PreviewFoldThrough" && operation.type !== "FoldThrough") {
        throw new Error(`unexpected operation: ${operation.type}`);
      }
      const wire = operation as unknown as Record<string, unknown>;
      expect(wire.target_pleat_count, operation.type).toBe(2);
      expect(wire.target_layers, operation.type).toBeNull();
      expect(Array.isArray(wire.target_layers), operation.type).toBe(false);
    }
  });

  it("8つの合わせ方は同じFoldDraft経路を通り、既定allを変えない", async () => {
    expect(ALIGN_CASES.map(({ mode }) => mode)).toEqual(EXPECTED_ALIGN_MODES);
    expect(Object.keys(ALIGN_STEPS)).toEqual(EXPECTED_ALIGN_MODES);
    expect(
      arrayInitializerStrings("../contextAlignFold.tsx", "ALIGN_MODES"),
    ).toEqual(EXPECTED_ALIGN_MODES);
    const contextSource = source("../contextAlignFold.tsx");
    expect(
      contextSource.match(
        /<FoldDraftContent\s+draft=\{foldDraft\}\s+showPleatTargets\s*\/>/g,
      ),
    ).toHaveLength(1);

    for (const { mode, picks } of ALIGN_CASES) {
      vi.clearAllMocks();
      seedFlat();
      completeAlign(mode, picks);
      const draft = useAppStore.getState().foldDraft;
      expect(draft?.target, mode).toBe("all");
      expect(hasOwn(draft!, "topPleatCount"), mode).toBe(false);

      await useAppStore.getState().commitFoldDraft();
      const operations = vi.mocked(ipc.sequenceApply).mock.calls.map(([op]) => op);
      expect(operations.map((operation) => operation.type), mode).toEqual([
        "PreviewFoldThrough",
        "FoldThrough",
      ]);
      for (const operation of operations) {
        const wire = operation as unknown as Record<string, unknown>;
        expect(wire.target_layers, mode).toBeNull();
        expect(hasOwn(wire, "target_pleat_count"), mode).toBe(false);
      }
    }
  });

  it("生の3Dドラッグは無修飾flap・Shift all・Alt singleのまま", () => {
    const body = functionBody("viewerPointer.ts", "grabMode");
    expect(body.match(/return\s+"(?:flap|all|single|topPleats)"/g)).toEqual([
      'return "all"',
      'return "single"',
      'return "flap"',
    ]);
    expect(body).not.toContain("topPleats");
  });

  it("top-Kは作品へ保存せず、schema 1・FoldStep・step_creasesの形を保つ", () => {
    const model = source("../../../../../crates/ori3-model/src/lib.rs");
    expect(model).toMatch(/pub const SCHEMA_VERSION:\s*u32\s*=\s*1\s*;/);

    const foldStep = rustStructBlock(model, "FoldStep");
    const stepCreases = rustStructBlock(model, "StepCreases");
    const savedDocument = rustStructBlock(model, "SavedDocument");
    expect(rustPublicFields(foldStep)).toEqual([
      "id",
      "kind",
      "drivers",
      "layer_order",
      "alignment",
      "finish_soft",
      "note",
    ]);
    expect(rustPublicFields(stepCreases)).toEqual(["step", "lines"]);
    expect(rustPublicFields(savedDocument)).toEqual(["document", "step_creases"]);
    for (const persisted of [foldStep, stepCreases, savedDocument]) {
      expect(persisted).not.toMatch(/top_?pleat|target_?pleat/i);
    }
  });
});
