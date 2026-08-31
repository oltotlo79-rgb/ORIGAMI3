import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as ts from "typescript";
import { describe, expect, it } from "vitest";
import { useAppStore } from "./appStore";

const STORE_DIR = dirname(fileURLToPath(import.meta.url));
const SRC_DIR = dirname(STORE_DIR);
const DESKTOP_DIR = dirname(SRC_DIR);
const APP_STORE_PATH = join(STORE_DIR, "appStore.ts");
const IPC_QUEUE_PATH = join(STORE_DIR, "ipcQueue.ts");
const TSCONFIG_PATH = join(DESKTOP_DIR, "tsconfig.json");

function canonical(fileName: string): string {
  return ts.sys.resolvePath(fileName).replace(/\\/g, "/").toLowerCase();
}

function nameBelow(root: string, fileName: string): string {
  const normalizedRoot = ts.sys.resolvePath(root).replace(/\\/g, "/");
  const normalizedFile = ts.sys.resolvePath(fileName).replace(/\\/g, "/");
  if (
    !normalizedFile.toLowerCase().startsWith(`${normalizedRoot.toLowerCase()}/`)
  ) {
    throw new Error(`${fileName} is not below ${root}`);
  }
  return normalizedFile.slice(normalizedRoot.length + 1);
}

function sourceName(fileName: string): string {
  return nameBelow(SRC_DIR, fileName);
}

function storeModuleName(fileName: string): string {
  const name = nameBelow(STORE_DIR, fileName).replace(/\.tsx?$/, "");
  return name.startsWith(".") ? name : `./${name}`;
}

function isProductionSource(source: ts.SourceFile): boolean {
  const name = canonical(source.fileName);
  return (
    !source.isDeclarationFile &&
    name.startsWith(`${canonical(SRC_DIR)}/`) &&
    !/\.(?:test|spec)\.[cm]?[jt]sx?$/.test(name)
  );
}

function loadProgram(): {
  checker: ts.TypeChecker;
  options: ts.CompilerOptions;
  productSources: ts.SourceFile[];
  program: ts.Program;
  sourceByKey: Map<string, ts.SourceFile>;
} {
  const configFile = ts.readConfigFile(TSCONFIG_PATH, ts.sys.readFile);
  if (configFile.error) {
    throw new Error(ts.flattenDiagnosticMessageText(configFile.error.messageText, "\n"));
  }
  const parsed = ts.parseJsonConfigFileContent(
    configFile.config,
    ts.sys,
    DESKTOP_DIR,
    undefined,
    TSCONFIG_PATH,
  );
  if (parsed.errors.length > 0) {
    throw new Error(
      parsed.errors
        .map((diagnostic) =>
          ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n"),
        )
        .join("\n"),
    );
  }
  const program = ts.createProgram({
    rootNames: parsed.fileNames,
    options: parsed.options,
  });
  const productSources = program.getSourceFiles().filter(isProductionSource);
  return {
    checker: program.getTypeChecker(),
    options: parsed.options,
    productSources,
    program,
    sourceByKey: new Map(
      productSources.map((source) => [canonical(source.fileName), source]),
    ),
  };
}

const { checker, options, productSources, program, sourceByKey } = loadProgram();
function requiredSource(fileName: string): ts.SourceFile {
  const source = program.getSourceFile(fileName);
  if (!source) {
    throw new Error(`${fileName} was not included by ${TSCONFIG_PATH}`);
  }
  return source;
}
const appStoreSource = requiredSource(APP_STORE_PATH);

async function sha256(value: string | Uint8Array): Promise<string> {
  const bytes =
    typeof value === "string" ? new TextEncoder().encode(value) : Uint8Array.from(value);
  const digest = await globalThis.crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("")
    .toUpperCase();
}

function moduleSymbol(source: ts.SourceFile): ts.Symbol {
  const symbol = checker.getSymbolAtLocation(source);
  if (!symbol) {
    throw new Error(`module symbol not found: ${source.fileName}`);
  }
  return symbol;
}

function resolveAlias(symbol: ts.Symbol): ts.Symbol {
  return symbol.flags & ts.SymbolFlags.Alias
    ? checker.getAliasedSymbol(symbol)
    : symbol;
}

function walk(node: ts.Node, visit: (node: ts.Node) => void): void {
  visit(node);
  node.forEachChild((child) => walk(child, visit));
}

function resolvedProductModule(
  containingFile: ts.SourceFile,
  specifier: string,
): string | null {
  const resolvedModule = ts.resolveModuleName(
    specifier,
    containingFile.fileName,
    options,
    ts.sys,
  ).resolvedModule;
  if (!resolvedModule) return null;
  const key = canonical(resolvedModule.resolvedFileName);
  return sourceByKey.has(key) ? key : null;
}

function expressionPath(expression: ts.Expression): string | null {
  if (ts.isIdentifier(expression)) return expression.text;
  if (ts.isPropertyAccessExpression(expression)) {
    const owner = expressionPath(expression.expression);
    return owner ? `${owner}.${expression.name.text}` : null;
  }
  return null;
}

const FACADE_CONTRACT = [
  "ActiveAngleIntent|type|./slices/poseReplaySlice",
  "AlignCpPick|type|./slices/documentSlice",
  "AlignDraft|type|./slices/documentSlice",
  "AngleSnapshot|type|./slices/poseReplaySlice",
  "DEFAULT_NEW_PAPER|value|./slices/dialogSettingsSlice",
  "DEFAULT_PNG_LONG_SIDE|value|./slices/dialogSettingsSlice",
  "ExportSettings|type|./slices/dialogSettingsSlice",
  "FINISH_JUMP_NOTICE_THRESHOLD|value|./slices/poseReplaySlice",
  "FINISH_JUMP_NOTICE|value|./slices/poseReplaySlice",
  "FoldAllPreviewState|type|./slices/poseReplaySlice",
  "FoldAllReturnState|type|./slices/poseReplaySlice",
  "FoldDraft|type|./slices/documentSlice",
  "FoldTarget|type|./slices/documentSlice",
  "FoldTargetSelection|type|./slices/documentSlice",
  "GuideAction|type|./slices/dialogSettingsSlice",
  "GuideStep|type|./slices/dialogSettingsSlice",
  "MIRROR_AXIS_REMOVED_NOTICE|value|./services/commandService",
  "MeasureDisplay|type|./slices/documentSlice",
  "MeasureDraft|type|./slices/documentSlice",
  "MeasureEdgePick|type|./slices/documentSlice",
  "MeasureMode|type|./slices/documentSlice",
  "MeasurePick|type|./slices/documentSlice",
  "MeasurePointPick|type|./slices/documentSlice",
  "NewPaperDraft|type|./slices/dialogSettingsSlice",
  "PendingFoldThrough|type|./slices/documentSlice",
  "ProposalPositionSnapshot|type|./slices/proposalSlice",
  "ProposalStep|type|./slices/proposalSlice",
  "RELAX_NOTICE_EPS_DEG|value|./slices/poseReplaySlice",
  "Selection|type|./slices/documentSlice",
  "SpatialFoldDrag|type|./slices/documentSlice",
  "TechniqueDraft|type|./slices/documentSlice",
  "ToolId|type|./toolTypes",
  "alignFoldDraft|value|./slices/documentSlice",
  "automaticMovingSide|value|./slices/documentSlice",
  "canFoldNow|value|./slices/documentSlice",
  "draftToPaper|value|./slices/dialogSettingsSlice",
  "foldInsertAt|value|./slices/documentSlice",
  "inflateBlockReason|value|./slices/poseReplaySlice",
  "initialMovingSide|value|./slices/documentSlice",
  "isAlignComplete|value|./slices/documentSlice",
  "isSpatialFoldFrame|value|./slices/documentSlice",
  "isStepSkipped|value|./slices/poseReplaySlice",
  "maximumFrameVertexMovement|value|./slices/poseReplaySlice",
  "nextAlignKind|value|./slices/documentSlice",
  "poseRecordReason|value|./slices/poseReplaySlice",
  "pullBlockReason|value|./slices/poseReplaySlice",
  "pullBlockedOf|value|./slices/poseReplaySlice",
  "relaxationNotices|value|./slices/poseReplaySlice",
  "resetFoldAllPreviewRuntime|value|./services/foldAllRuntime",
  "resetPoseThrottle|value|./services/poseRuntime",
  "stepPanelSelected|value|./slices/poseReplaySlice",
  "useAppStore|value|./appStore",
].sort();

function facadeContract(): string[] {
  return checker
    .getExportsOfModule(moduleSymbol(appStoreSource))
    .map((exported) => {
      const target = resolveAlias(exported);
      const declaration = target.getDeclarations()?.[0];
      if (!declaration) throw new Error(`declaration not found: ${exported.name}`);
      const kind = target.flags & ts.SymbolFlags.Value ? "value" : "type";
      return `${exported.name}|${kind}|${storeModuleName(
        declaration.getSourceFile().fileName,
      )}`;
    })
    .sort();
}

function countCallsNamed(source: ts.SourceFile, name: string): number {
  let count = 0;
  walk(source, (node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === name
    ) {
      count += 1;
    }
  });
  return count;
}

function storeReturnShape(): string[] {
  let initializer: ts.Expression | undefined;
  for (const statement of appStoreSource.statements) {
    if (!ts.isVariableStatement(statement)) continue;
    for (const declaration of statement.declarationList.declarations) {
      if (
        ts.isIdentifier(declaration.name) &&
        declaration.name.text === "useAppStore"
      ) {
        initializer = declaration.initializer;
      }
    }
  }
  if (!initializer || !ts.isCallExpression(initializer)) {
    throw new Error("useAppStore create call not found");
  }
  const creator = initializer.arguments[0];
  if (
    !creator ||
    (!ts.isArrowFunction(creator) && !ts.isFunctionExpression(creator)) ||
    !ts.isBlock(creator.body)
  ) {
    throw new Error("useAppStore state creator not found");
  }
  const returned = creator.body.statements.find(ts.isReturnStatement)?.expression;
  if (!returned || !ts.isObjectLiteralExpression(returned)) {
    throw new Error("useAppStore object return not found");
  }
  return returned.properties.map((property) => {
    if (!ts.isSpreadAssignment(property)) {
      throw new Error(`unexpected return property: ${property.getText(appStoreSource)}`);
    }
    if (!ts.isObjectLiteralExpression(property.expression)) {
      const path = expressionPath(property.expression);
      if (!path) throw new Error("non-path slice spread");
      return `spread:${path}`;
    }
    if (property.expression.properties.length !== 1) {
      throw new Error("recovery reservation must contain exactly one property");
    }
    const reservation = property.expression.properties[0];
    if (
      !ts.isPropertyAssignment(reservation) ||
      !ts.isIdentifier(reservation.name)
    ) {
      throw new Error("invalid recovery reservation");
    }
    const value = expressionPath(reservation.initializer);
    if (!value) throw new Error("invalid recovery reservation source");
    return `reserve:${reservation.name.text}=${value}`;
  });
}

function literalModuleSpecifiers(source: ts.SourceFile): string[] {
  const specifiers: string[] = [];
  walk(source, (node) => {
    if (
      (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) &&
      node.moduleSpecifier &&
      ts.isStringLiteralLike(node.moduleSpecifier)
    ) {
      specifiers.push(node.moduleSpecifier.text);
    } else if (
      ts.isImportEqualsDeclaration(node) &&
      ts.isExternalModuleReference(node.moduleReference) &&
      node.moduleReference.expression &&
      ts.isStringLiteralLike(node.moduleReference.expression)
    ) {
      specifiers.push(node.moduleReference.expression.text);
    } else if (
      ts.isCallExpression(node) &&
      node.expression.kind === ts.SyntaxKind.ImportKeyword &&
      node.arguments[0] &&
      ts.isStringLiteralLike(node.arguments[0])
    ) {
      specifiers.push(node.arguments[0].text);
    } else if (
      ts.isImportTypeNode(node) &&
      ts.isLiteralTypeNode(node.argument) &&
      ts.isStringLiteralLike(node.argument.literal)
    ) {
      specifiers.push(node.argument.literal.text);
    }
  });
  return specifiers;
}

function importGraph(): Map<string, Set<string>> {
  return new Map(
    productSources.map((source) => [
      canonical(source.fileName),
      new Set(
        literalModuleSpecifiers(source)
          .map((specifier) => resolvedProductModule(source, specifier))
          .filter((key): key is string => key !== null),
      ),
    ]),
  );
}

function cyclicComponents(graph: Map<string, Set<string>>): string[][] {
  let nextIndex = 0;
  const indices = new Map<string, number>();
  const lowLinks = new Map<string, number>();
  const stack: string[] = [];
  const onStack = new Set<string>();
  const cycles: string[][] = [];

  function connect(node: string): void {
    indices.set(node, nextIndex);
    lowLinks.set(node, nextIndex);
    nextIndex += 1;
    stack.push(node);
    onStack.add(node);

    for (const target of graph.get(node) ?? []) {
      if (!indices.has(target)) {
        connect(target);
        lowLinks.set(node, Math.min(lowLinks.get(node)!, lowLinks.get(target)!));
      } else if (onStack.has(target)) {
        lowLinks.set(node, Math.min(lowLinks.get(node)!, indices.get(target)!));
      }
    }
    if (lowLinks.get(node) !== indices.get(node)) return;

    const component: string[] = [];
    let member: string;
    do {
      member = stack.pop()!;
      onStack.delete(member);
      component.push(member);
    } while (member !== node);
    if (
      component.length > 1 ||
      (component.length === 1 && graph.get(node)?.has(node))
    ) {
      cycles.push(
        component
          .map((key) => sourceName(sourceByKey.get(key)!.fileName))
          .sort(),
      );
    }
  }

  for (const node of graph.keys()) {
    if (!indices.has(node)) connect(node);
  }
  return cycles.sort((left, right) => left.join("|").localeCompare(right.join("|")));
}

interface ImportedBinding {
  file: string;
  imported: string;
  local: string;
  symbol: ts.Symbol | undefined;
}

function runtimeImportsFrom(modulePrefix: string): ImportedBinding[] {
  const bindings: ImportedBinding[] = [];
  for (const source of productSources) {
    for (const statement of source.statements) {
      if (
        !ts.isImportDeclaration(statement) ||
        !ts.isStringLiteralLike(statement.moduleSpecifier) ||
        !(
          statement.moduleSpecifier.text === modulePrefix ||
          statement.moduleSpecifier.text.startsWith(`${modulePrefix}/`)
        )
      ) {
        continue;
      }
      const clause = statement.importClause;
      if (!clause || clause.isTypeOnly) continue;
      if (clause.name) {
        bindings.push({
          file: sourceName(source.fileName),
          imported: "default",
          local: clause.name.text,
          symbol: checker.getSymbolAtLocation(clause.name),
        });
      }
      if (clause.namedBindings && ts.isNamespaceImport(clause.namedBindings)) {
        bindings.push({
          file: sourceName(source.fileName),
          imported: "*",
          local: clause.namedBindings.name.text,
          symbol: checker.getSymbolAtLocation(clause.namedBindings.name),
        });
      } else if (clause.namedBindings) {
        for (const element of clause.namedBindings.elements) {
          if (element.isTypeOnly) continue;
          bindings.push({
            file: sourceName(source.fileName),
            imported: element.propertyName?.text ?? element.name.text,
            local: element.name.text,
            symbol: checker.getSymbolAtLocation(element.name),
          });
        }
      }
    }
  }
  return bindings;
}

function callsOfBindings(bindings: ImportedBinding[]): string[] {
  const calls: string[] = [];
  for (const source of productSources) {
    walk(source, (node) => {
      if (!ts.isCallExpression(node) || !ts.isIdentifier(node.expression)) return;
      const symbol = checker.getSymbolAtLocation(node.expression);
      for (const binding of bindings) {
        if (binding.symbol && symbol === binding.symbol) {
          calls.push(`${sourceName(source.fileName)}:${binding.local}`);
        }
      }
    });
  }
  return calls.sort();
}

function serialQueueBindings(): ImportedBinding[] {
  const queueKey = canonical(IPC_QUEUE_PATH);
  const bindings: ImportedBinding[] = [];
  for (const source of productSources) {
    for (const statement of source.statements) {
      if (
        !ts.isImportDeclaration(statement) ||
        !ts.isStringLiteralLike(statement.moduleSpecifier) ||
        resolvedProductModule(source, statement.moduleSpecifier.text) !== queueKey
      ) {
        continue;
      }
      const named = statement.importClause?.namedBindings;
      if (!named || !ts.isNamedImports(named)) continue;
      for (const element of named.elements) {
        if (
          !element.isTypeOnly &&
          (element.propertyName?.text ?? element.name.text) === "createSerialQueue"
        ) {
          bindings.push({
            file: sourceName(source.fileName),
            imported: "createSerialQueue",
            local: element.name.text,
            symbol: checker.getSymbolAtLocation(element.name),
          });
        }
      }
    }
  }
  return bindings;
}

function explicitNamedConsumers(): Map<string, Set<string>> {
  const consumers = new Map<string, Set<string>>();
  for (const source of productSources) {
    for (const statement of source.statements) {
      if (
        (!ts.isImportDeclaration(statement) && !ts.isExportDeclaration(statement)) ||
        !statement.moduleSpecifier ||
        !ts.isStringLiteralLike(statement.moduleSpecifier)
      ) {
        continue;
      }
      const target = resolvedProductModule(source, statement.moduleSpecifier.text);
      if (!target) continue;
      const elements = ts.isImportDeclaration(statement)
        ? statement.importClause?.namedBindings &&
          ts.isNamedImports(statement.importClause.namedBindings)
          ? statement.importClause.namedBindings.elements
          : []
        : statement.exportClause && ts.isNamedExports(statement.exportClause)
          ? statement.exportClause.elements
          : [];
      for (const element of elements) {
        const imported = element.propertyName?.text ?? element.name.text;
        const key = `${target}|${imported}`;
        const current = consumers.get(key) ?? new Set<string>();
        current.add(canonical(source.fileName));
        consumers.set(key, current);
      }
    }
  }
  return consumers;
}

function unusedServiceTypeExports(): string[] {
  const consumers = explicitNamedConsumers();
  const stale: string[] = [];
  for (const source of productSources) {
    const file = sourceName(source.fileName);
    if (!/^store\/services\/[^/]+\.ts$/.test(file)) continue;
    const sourceKey = canonical(source.fileName);
    for (const exported of checker.getExportsOfModule(moduleSymbol(source))) {
      const target = resolveAlias(exported);
      const isTypeDeclaration = target
        .getDeclarations()
        ?.some(
          (declaration) =>
            ts.isInterfaceDeclaration(declaration) ||
            ts.isTypeAliasDeclaration(declaration),
        );
      if (!isTypeDeclaration) continue;
      const otherConsumers = [...(consumers.get(`${sourceKey}|${exported.name}`) ?? [])]
        .filter((consumer) => consumer !== sourceKey);
      if (otherConsumers.length === 0) stale.push(`${file}:${exported.name}`);
    }
  }
  return stale.sort();
}

const REVIEWED_SLICE_INTERNAL_EXPORTS = [
  "store/slices/dialogSettingsSlice.ts:DialogSettingsPoseState",
  "store/slices/documentSlice.ts:DocumentSliceExternalActions",
  "store/slices/documentSlice.ts:DocumentSliceExternalState",
  "store/slices/documentSlice.ts:DocumentSliceInternals",
] as const;

function presentReviewedSliceInternalExports(): string[] {
  return REVIEWED_SLICE_INTERNAL_EXPORTS.filter((entry) => {
    const separator = entry.lastIndexOf(":");
    const file = entry.slice(0, separator);
    const name = entry.slice(separator + 1);
    const source = productSources.find((candidate) => sourceName(candidate.fileName) === file);
    if (!source) throw new Error(`reviewed slice missing: ${file}`);
    return checker.getExportsOfModule(moduleSymbol(source)).some(
      (exported) => exported.name === name,
    );
  });
}

const EXPECTED_RUNTIME_STORE_KEY_ORDER = [
  "doc",
  "stepCreases",
  "faces",
  "warnings",
  "foldIssues", // 旧: なし → 新: foldIssues。ほかの折り紙ソフトのファイルの読込注意。意図した変更に対する照合値の更新であり、緩和ではない。
  "flatFoldViolations",
  "violations",
  "selection",
  "hoveredHinge",
  "activeTool",
  "measureDraft",
  "foldDraft",
  "pendingFoldThrough",
  "foldThroughBusy",
  "alignDraft",
  "techniqueDraft",
  "construct",
  "curve",
  "errorMessage",
  "documentSavedPath",
  "docEpoch",
  "newDocument",
  "openDocument",
  "saveDocument",
  "applyEdit",
  "drawSegment",
  "drawCurve",
  "setMirrorDraw",
  "setMirrorAxisPreset",
  "setSelectedLineAsMirrorAxis",
  "setTool",
  "setMeasureMode",
  "setMeasureDisplay",
  "pickMeasureEdge",
  "pickMeasurePoint",
  "clearMeasurement",
  "setSelection",
  "setHoveredHinge",
  "beginFoldDraft",
  "updateFoldDraft",
  "setFoldTarget",
  "requestFoldTargetInfo",
  "cancelFoldDraft",
  "commitFoldDraft",
  "resolveFoldThroughProposal",
  "beginAlign",
  "pickAlignTarget",
  "nextAlignSolution",
  "undoAlignPick",
  "cancelAlign",
  "foldByDrag",
  "beginTechnique",
  "setTechniqueFlap",
  "setTechniqueFlapPreset",
  "toggleTechniqueFlap",
  "setTechniqueLine",
  "setLayerMotionAxis",
  "addLayerMotionPart",
  "undoLayerMotionPart",
  "addTechniqueVertex",
  "undoTechniqueVertex",
  "setTechniqueCenter",
  "setTechniqueReferencePoint",
  "updateTechniqueDraft",
  "setConstruct",
  "setCurve",
  "cancelTechnique",
  "commitTechnique",
  "hinges",
  "frame3d",
  "foldAllPreview",
  "suspectHinges",
  "sequenceTargets",
  "relaxations",
  "softMesh",
  "softWarnings",
  "currentStep",
  "playT",
  "playing",
  "skipped",
  "replaySkipped",
  "replayWarnings",
  "drivers",
  "pinnedFolds",
  "releasedPins",
  "releasedPinHinges",
  "angleUndoStack",
  "angleRedoStack",
  "docUndoDepth",
  "poseAngles",
  "poseWarnings",
  "poseConverged",
  "poseBestEffort",
  "poseClosureRms",
  "contactDetected",
  "activeAngleIntent",
  "angleIntentGeneration",
  "pullHinge",
  "pullMirrorHinge",
  "undo",
  "redo",
  "applySequenceOp",
  "selectStep",
  "selectStepForCapture",
  "stepBy",
  "togglePlay",
  "beginPull",
  "pullTo",
  "endPull",
  "setDriverAngle",
  "setDriverAngles",
  "finishAngleIntent",
  "clearDriver",
  "clearDrivers",
  "enterFoldAllPreview",
  "setFoldAllPercent",
  "finishFoldAllPercent",
  "leaveFoldAllPreview",
  "togglePinnedFold",
  "setPinnedFolds",
  "recordPoseStep",
  "moveStep",
  "recovery",
  "proposalStep",
  "proposalSkeleton",
  "proposalCandidates",
  "proposalSelected",
  "proposalPaperSource",
  "proposalPaperPositions",
  "proposalPaperSpecified",
  "proposalPositionLastMoved",
  "proposalPositionUndoStack",
  "proposalPositionRedoStack",
  "proposalBusy",
  "proposalJobId",
  "proposalProgress",
  "proposalProgressWarning",
  "proposalError",
  "proposalSeed",
  "openProposal",
  "closeProposal",
  "setProposalStep",
  "setProposalSkeleton",
  "setProposalTipPosition",
  "generateProposal",
  "selectProposalCandidate",
  "openProposalPaperPositionEditor",
  "setProposalPaperPosition",
  "resetProposalPaperPositions",
  "restoreOtherProposalPosition",
  "undoProposalPosition",
  "redoProposalPosition",
  "generateProposalFromPaperPositions",
  "applyProposalCandidate",
  "recoveryChoices", // 旧: なし → 新: recoveryChoices。復旧候補の複数化。意図した変更に対する照合値の更新であり、緩和ではない。
  "recoveryDismissed", // 旧: なし → 新: recoveryDismissed。復旧候補を残したまま「あとで確認する」を選ぶ状態。意図した変更に対する照合値の更新であり、緩和ではない。
  "recoveryOverflowNotice", // 旧: なし → 新: recoveryOverflowNotice。4件以上の復旧候補を知らせる注意。意図した変更に対する照合値の更新であり、緩和ではない。
  "recoveryBusy", // 旧: なし → 新: recoveryBusy。復旧・破棄の二度押し防止。意図した変更に対する照合値の更新であり、緩和ではない。
  "exportOpen",
  "exportKind",
  "exportIncludeAux",
  "exportLongSide",
  "exportBusy",
  "exportError",
  "exportSavedPath",
  "exportDeliveryNotice",
  "exportFoldIssues", // 旧: なし → 新: exportFoldIssues。書き出しは続行できても利用者へ伝える注意。意図した変更に対する照合値の更新であり、緩和ではない。
  "newDialogOpen",
  "newPaperDraft",
  "display",
  "splitRatio",
  "contextPanelRatio",
  "mirrorDraw",
  "mirrorAxis",
  "mirrorAxisNotice",
  "pullMirror",
  "wheelBehavior",
  "uiTheme",
  "contextHelpExpanded",
  "viewerHintExpanded",
  "cpHelpExpanded",
  "paperHelpExpanded",
  "paperColorExpanded",
  "guideOpen",
  "guideStep",
  "helpOpen",
  "helpChapterId",
  "helpQuery",
  "operationStage",
  "lineInputStart",
  "paperActionTipVisible",
  "paperActionTipExpanded",
  "setPullMirror",
  "setWheelBehavior",
  "setUiTheme",
  "toggleContextHelp",
  "toggleViewerHint",
  "toggleCpHelp",
  "togglePaperHelp",
  "togglePaperColor",
  "openGuide",
  "openHelp",
  "closeHelp",
  "selectHelpChapter",
  "setHelpQuery",
  "dismissGuide",
  "completeGuideAction",
  "setOperationStage",
  "setLineInputStart",
  "showPaperActionTip",
  "collapsePaperActionTip",
  "expandPaperActionTip",
  "hidePaperActionTip",
  "checkRecovery",
  "resolveRecovery",
  "dismissRecovery", // 旧: なし → 新: dismissRecovery。復旧候補を残してダイアログを閉じる操作。意図した変更に対する照合値の更新であり、緩和ではない。
  "openRecovery", // 旧: なし → 新: openRecovery。保留した復旧候補を再表示する操作。意図した変更に対する照合値の更新であり、緩和ではない。
  "openExport",
  "closeExport",
  "setExportOption",
  "runExport",
  "openNewDialog",
  "closeNewDialog",
  "setNewPaperDraft",
  "confirmNewDocument",
  "setDisplay",
  "setSoft",
  "setSplitRatio",
  "setContextPanelRatio",
  "resetPaneSizes",
] as const;

describe("store composition boundary", () => {
  it("keeps the exact 51-name appStore facade without export-star", () => {
    expect(facadeContract()).toEqual(FACADE_CONTRACT);
    const nonNamedExports = appStoreSource.statements
      .filter(ts.isExportDeclaration)
      .filter(
        (statement) =>
          !statement.exportClause || !ts.isNamedExports(statement.exportClause),
      )
      .map((statement) => statement.getText(appStoreSource));
    expect(nonNamedExports).toEqual([]);
  });

  it("composes AppState from exactly four slices and spreads each factory once", () => {
    const appState = appStoreSource.statements.find(
      (statement): statement is ts.InterfaceDeclaration =>
        ts.isInterfaceDeclaration(statement) && statement.name.text === "AppState",
    );
    expect(appState).toBeDefined();
    expect(appState?.members).toHaveLength(0);
    expect(
      (appState?.heritageClauses ?? []).flatMap((clause) =>
        clause.types.map((type) => type.expression.getText(appStoreSource)),
      ),
    ).toEqual([
      "DocumentSlice",
      "PoseReplaySlice",
      "ProposalSlice",
      "DialogSettingsSlice",
    ]);

    for (const factory of [
      "createCommandService",
      "createDialogSettingsSlice",
      "createProposalSlice",
      "createDocumentSlice",
      "createPoseReplaySlice",
    ]) {
      expect(countCallsNamed(appStoreSource, factory), factory).toBe(1);
    }
    expect(storeReturnShape()).toEqual([
      "spread:documentSlice.slice",
      "spread:poseReplay.slice",
      "reserve:recovery=dialogSettingsSlice.slice.recovery",
      "spread:proposalSlice.slice",
      "spread:dialogSettingsSlice.slice",
    ]);
  });

  it("keeps the exact runtime store key order", () => {
    const keys = Object.keys(useAppStore.getInitialState());
    expect(keys).toEqual(EXPECTED_RUNTIME_STORE_KEY_ORDER);
  });

  it("keeps all production TypeScript imports and re-exports acyclic", () => {
    expect(cyclicComponents(importGraph())).toEqual([]);
  });

  it("creates exactly one Zustand store in appStore", () => {
    const imports = runtimeImportsFrom("zustand");
    expect(
      imports
        .map(({ file, imported, local }) => `${file}:${imported}->${local}`)
        .sort(),
    ).toEqual(["store/appStore.ts:create->create"]);
    expect(callsOfBindings(imports)).toEqual(["store/appStore.ts:create"]);
  });

  it("uses the existing serial queue in exactly two production services", () => {
    const bindings = serialQueueBindings();
    expect(
      bindings.map(({ file, imported, local }) => `${file}:${imported}->${local}`).sort(),
    ).toEqual([
      "store/services/commandService.ts:createSerialQueue->createSerialQueue",
      "store/services/foldAllRuntime.ts:createSerialQueue->createSerialQueue",
    ]);
    expect(callsOfBindings(bindings)).toEqual([
      "store/services/commandService.ts:createSerialQueue",
      "store/services/foldAllRuntime.ts:createSerialQueue",
    ]);
  });

  it("keeps ipcQueue byte-for-byte unchanged", async () => {
    const bytes = readFileSync(IPC_QUEUE_PATH);
    const text = new TextDecoder().decode(bytes);
    expect(bytes.byteLength).toBe(4627);
    expect(text.match(/\n/g)?.length ?? 0).toBe(105);
    expect(await sha256(bytes)).toBe(
      "783DF611311B2A5436DD5D636214428AC0C96DE5AD03883B3CCB9E84A4C1CC79",
    );
  });

  // 行数上限は CLAUDE.md §9 で撤廃済み。分割境界は所有・型・再公開の契約で検査する。

  it("does not publicly expose reviewed internal-only service and slice types", () => {
    expect([
      ...unusedServiceTypeExports(),
      ...presentReviewedSliceInternalExports(),
    ].sort()).toEqual([]);
  });
});
