import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as ts from "typescript";

const DESKTOP_DIR = dirname(dirname(fileURLToPath(import.meta.url)));
const SOURCE_DIR = join(DESKTOP_DIR, "src");
const validProbePath = join(SOURCE_DIR, "__topPleatSelectionValidContractProbe.ts");
const missingProbePath = join(
  SOURCE_DIR,
  "__topPleatSelectionMissingContractProbe.ts",
);
const probes = new Map([
  [
    validProbePath,
    `
import type { GrabSelection } from "./components/Viewer3D/grabFold";
import type { FoldTargetSelection } from "./store/appStore";

const validGrab: GrabSelection = { mode: "topPleats", topPleatCount: 2 };
const validFold: FoldTargetSelection = { target: "topPleats", topPleatCount: 2 };
void [validGrab, validFold];
`,
  ],
  [
    missingProbePath,
    `
import type { GrabSelection } from "./components/Viewer3D/grabFold";
import type { FoldTargetSelection } from "./store/appStore";

const missingGrabCount: GrabSelection = { mode: "topPleats" };
const missingFoldCount: FoldTargetSelection = { target: "topPleats" };
void [missingGrabCount, missingFoldCount];
`,
  ],
]);

const options = {
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
const pathKey = (fileName) => fileName.replace(/\\/g, "/").toLowerCase();
const probeByKey = new Map(
  [...probes].map(([fileName, source]) => [pathKey(fileName), source]),
);
host.fileExists = (fileName) =>
  probeByKey.has(pathKey(fileName)) || originalFileExists(fileName);
host.readFile = (fileName) =>
  probeByKey.get(pathKey(fileName)) ?? originalReadFile(fileName);
host.getSourceFile = (
  fileName,
  languageVersion,
  onError,
  shouldCreateNewSourceFile,
) => {
  const probe = probeByKey.get(pathKey(fileName));
  return probe === undefined
    ? originalGetSourceFile(
        fileName,
        languageVersion,
        onError,
        shouldCreateNewSourceFile,
      )
    : ts.createSourceFile(
        fileName,
        probe,
        languageVersion,
        true,
        ts.ScriptKind.TS,
      );
};

const program = ts.createProgram({
  rootNames: [...probes.keys()],
  options,
  host,
});
const diagnostics = ts.getPreEmitDiagnostics(program);
const diagnosticsFor = (probePath) =>
  diagnostics.filter(
    (diagnostic) =>
      diagnostic.file && pathKey(diagnostic.file.fileName) === pathKey(probePath),
  );
const diagnosticText = (diagnostic) =>
  `${diagnostic.code}: ${ts.flattenDiagnosticMessageText(
    diagnostic.messageText,
    " ",
  )}`;
const fail = (message, found) => {
  const details = found.length === 0 ? "(診断なし)" : found.map(diagnosticText).join("\n");
  throw new Error(`${message}\n${details}`);
};

const validDiagnostics = diagnosticsFor(validProbePath);
if (validDiagnostics.length !== 0) {
  fail("K付き topPleats は診断0件でなければなりません", validDiagnostics);
}

const missingDiagnostics = diagnosticsFor(missingProbePath);
if (
  missingDiagnostics.length !== 2 ||
  missingDiagnostics.some(
    (diagnostic) => !diagnosticText(diagnostic).includes("topPleatCount"),
  )
) {
  fail(
    "K無し topPleats は topPleatCount に言及する診断2件でなければなりません",
    missingDiagnostics,
  );
}

console.log(
  `topPleats type contract: K missing diagnostics=${missingDiagnostics.length}; ` +
    `K present diagnostics=${validDiagnostics.length}`,
);
