import { readFileSync } from "node:fs";
import { describe, expect, expectTypeOf, it } from "vitest";
import type {
  Document,
  DocumentExportKind,
  FoldIssue,
} from "../lib/types";
import * as appStoreFacade from "./appStore";
import { useAppStore } from "./appStore";
import * as dialogSettingsModule from "./slices/dialogSettingsSlice";
import * as documentModule from "./slices/documentSlice";
import * as foldAllRuntimeModule from "./services/foldAllRuntime";
import * as poseRuntimeModule from "./services/poseRuntime";
import * as poseReplayModule from "./slices/poseReplaySlice";
import type {
  DialogSettingsSlice,
  DialogSettingsSliceActions,
  DialogSettingsSliceState,
  ExportSettings,
} from "./slices/dialogSettingsSlice";
import type {
  DocumentSlice,
  DocumentSliceActions,
  DocumentSliceState,
} from "./slices/documentSlice";
import type {
  FoldAllReturnState,
  PoseReplaySlice,
  PoseReplaySliceActions,
  PoseReplaySliceState,
} from "./slices/poseReplaySlice";
import type {
  ProposalSlice,
  ProposalSliceActions,
  ProposalSliceState,
} from "./slices/proposalSlice";

const PRODUCTION_SOURCES = import.meta.glob("../**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const B1_ACTIONS = [
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
] as const satisfies readonly (keyof DocumentSliceActions)[];

const B1_STATE = [
  "doc",
  "stepCreases",
  "faces",
  "warnings",
  "foldIssues",
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
] as const satisfies readonly (keyof DocumentSlice)[];

const B1_PUBLIC_VALUES = [
  "automaticMovingSide",
  "initialMovingSide",
  "alignFoldDraft",
  "isAlignComplete",
  "nextAlignKind",
  "canFoldNow",
  "foldInsertAt",
  "isSpatialFoldFrame",
] as const satisfies readonly (keyof typeof documentModule)[];

const B2_STATE = [
  "frame3d",
  "selfIntersectionPairs",
  "focusedSelfIntersectionPairIndex",
  "foldAllPreview",
  "suspectHinges",
  "sequenceTargets",
  "relaxations",
  "softMesh",
  "softWarnings",
  "hinges",
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
] as const satisfies readonly (keyof PoseReplaySliceState)[];

const B1_PUBLIC_TYPES = [
  "AlignCpPick",
  "AlignDraft",
  "FoldDraft",
  "FoldTarget",
  "FoldTargetSelection",
  "MeasureDisplay",
  "MeasureDraft",
  "MeasureEdgePick",
  "MeasureMode",
  "MeasurePick",
  "MeasurePointPick",
  "PendingFoldThrough",
  "Selection",
  "SpatialFoldDrag",
  "TechniqueDraft",
] as const;

const B2_ACTIONS = [
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
  "focusNextSelfIntersectionPair",
  "togglePinnedFold",
  "setPinnedFolds",
  "recordPoseStep",
  "moveStep",
] as const satisfies readonly (keyof PoseReplaySliceActions)[];

const B2_FOLD_ALL_RETURN_STATE = [
  "docEpoch",
  "currentStep",
  "playT",
  "activeTool",
  "selection",
] as const satisfies readonly (keyof FoldAllReturnState)[];

const B2_PUBLIC_VALUES = [
  "FINISH_JUMP_NOTICE",
  "FINISH_JUMP_NOTICE_THRESHOLD",
  "RELAX_NOTICE_EPS_DEG",
  "inflateBlockReason",
  "isStepSkipped",
  "maximumFrameVertexMovement",
  "poseRecordReason",
  "pullBlockReason",
  "pullBlockedOf",
  "relaxationNotices",
  "stepPanelSelected",
] as const satisfies readonly (keyof typeof poseReplayModule)[];

const B2_PUBLIC_TYPES = [
  "ActiveAngleIntent",
  "AngleSnapshot",
  "FoldAllPreviewState",
  "FoldAllReturnState",
] as const;

const B3_STATE = [
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
] as const satisfies readonly (keyof ProposalSliceState)[];

const B3_ACTIONS = [
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
] as const satisfies readonly (keyof ProposalSliceActions)[];

const B3_PUBLIC_TYPES = [
  "ProposalPositionSnapshot",
  "ProposalStep",
] as const;

const B4_STATE = [
  "recovery",
  "recoveryChoices",
  "recoveryDismissed",
  "recoveryOverflowNotice",
  "recoveryBusy",
  "exportOpen",
  "exportKind",
  "exportIncludeAux",
  "exportLongSide",
  "exportBusy",
  "exportError",
  "exportSavedPath",
  "exportDeliveryNotice",
  "exportFoldIssues",
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
] as const satisfies readonly (keyof DialogSettingsSliceState)[];

const B4_ACTIONS = [
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
  "dismissRecovery",
  "openRecovery",
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
] as const satisfies readonly (keyof DialogSettingsSliceActions)[];

const B4_PUBLIC_VALUES = [
  "DEFAULT_NEW_PAPER",
  "draftToPaper",
  "DEFAULT_PNG_LONG_SIDE",
] as const satisfies readonly (keyof typeof dialogSettingsModule)[];

const B4_PUBLIC_TYPES = [
  "GuideStep",
  "GuideAction",
  "NewPaperDraft",
  "ExportSettings",
] as const;

function source(relativePath: string): string {
  return readFileSync(new URL(relativePath, import.meta.url), "utf8");
}

describe("store split boundary", () => {
  it("B1 owns document/CP state outside the compatibility facade", () => {
    const documentSlice = source("./slices/documentSlice.ts");
    const documentActions = source("./services/documentActions.ts");
    const facade = source("./appStore.ts");
    const normalizedFacade = facade.replace(/\r\n/g, "\n");

    for (const name of [
      "automaticMovingSide",
      "alignFoldDraft",
      "foldInsertAt",
    ]) {
      expect(documentSlice, name).toMatch(
        new RegExp(`(?:function|const)\\s+${name}\\b`),
      );
    }
    expect(facade).not.toMatch(/function\s+automaticMovingSide\b/);
    expect(facade).not.toMatch(/function\s+alignFoldDraft\b/);
    expect(documentActions).toMatch(/function\s+createDocumentSlice\b/);
    expect(facade).toContain("interface AppState extends DocumentSlice");
    expect(facade.match(/createDocumentSlice<AppState>\(/g)).toHaveLength(1);
    expect(facade.match(/\.\.\.documentSlice\.slice/g)).toHaveLength(1);
    const composedReturnStart = normalizedFacade.indexOf(
      "  return {\n    ...documentSlice.slice",
    );
    expect(composedReturnStart).toBeGreaterThanOrEqual(0);
    const composedReturn = normalizedFacade.slice(composedReturnStart);
    const appStateMatch = normalizedFacade.match(
      /interface\s+AppState\s+extends\s+DocumentSlice[^{]*\{\s*\}/,
    );
    expect(appStateMatch).not.toBeNull();
    const appStateDeclaration = appStateMatch![0];

    for (const name of B1_STATE) {
      expect(documentActions, name).toMatch(new RegExp(`^    ${name}:`, "m"));
      expect(composedReturn, name).not.toMatch(new RegExp(`^    ${name}:`, "m"));
      expect(appStateDeclaration, name).not.toMatch(
        new RegExp(`^  ${name}:`, "m"),
      );
    }
    expectTypeOf<DocumentSliceState["foldIssues"]>().toEqualTypeOf<
      FoldIssue[]
    >();
    expect(useAppStore.getState().foldIssues).toEqual([]);

    for (const name of B1_ACTIONS) {
      expect(documentActions, name).toMatch(
        new RegExp(`^    ${name}:`, "m"),
      );
      expect(facade, name).not.toMatch(new RegExp(`^    ${name}:`, "m"));
      expect(appStateDeclaration, name).not.toMatch(
        new RegExp(`^  ${name}:`, "m"),
      );
      expect(useAppStore.getState()[name], name).toBeTypeOf("function");
    }

    type MissingAction = Exclude<
      keyof DocumentSliceActions,
      (typeof B1_ACTIONS)[number]
    >;
    type ExtraAction = Exclude<
      (typeof B1_ACTIONS)[number],
      keyof DocumentSliceActions
    >;
    type MissingState = Exclude<
      keyof DocumentSliceState,
      (typeof B1_STATE)[number]
    >;
    type ExtraState = Exclude<
      (typeof B1_STATE)[number],
      keyof DocumentSliceState
    >;
    expectTypeOf<MissingAction>().toEqualTypeOf<never>();
    expectTypeOf<ExtraAction>().toEqualTypeOf<never>();
    expectTypeOf<MissingState>().toEqualTypeOf<never>();
    expectTypeOf<ExtraState>().toEqualTypeOf<never>();
    type StoreState = ReturnType<typeof useAppStore.getState>;
    type ComposedDocumentSlice = Pick<StoreState, keyof DocumentSlice>;
    type StoreExtendsSlice = ComposedDocumentSlice extends DocumentSlice
      ? true
      : false;
    type SliceExtendsStore = DocumentSlice extends ComposedDocumentSlice
      ? true
      : false;
    expectTypeOf<StoreExtendsSlice>().toEqualTypeOf<true>();
    expectTypeOf<SliceExtendsStore>().toEqualTypeOf<true>();

    for (const name of B1_PUBLIC_VALUES) {
      expect(appStoreFacade[name], name).toBe(documentModule[name]);
    }
    const publicTypeReexport = facade.match(
      /export\s+type\s*\{([^}]*)\}\s+from\s+"\.\/slices\/documentSlice"/,
    );
    expect(publicTypeReexport).not.toBeNull();
    const reexportedDocumentTypes = publicTypeReexport![1]
      .split(",")
      .map((name) => name.trim())
      .filter(Boolean)
      .sort();
    expect(reexportedDocumentTypes).toEqual([...B1_PUBLIC_TYPES].sort());
    for (const name of B1_PUBLIC_TYPES) {
      expect(facade, name).not.toMatch(
        new RegExp(`export\\s+(?:interface|type)\\s+${name}\\b`),
      );
    }

    // 行数上限はCLAUDE.md §9で撤廃済み。分割境界は上の所有・型・再公開契約で検査する。
  });

  it("B1 uses the existing serial queue through the command service", async () => {
    const commandService = source("./services/commandService.ts");
    const documentActions = source("./services/documentActions.ts");
    const generationGate = source("./services/generationGate.ts");
    const documentSlice = source("./slices/documentSlice.ts");

    expect(commandService).toContain('from "../ipcQueue"');
    expect(commandService).toMatch(/\bcreateSerialQueue\(\)/);
    expect(commandService).toMatch(/\bcreateCommandService\b/);
    expect(documentActions).toMatch(/\bcreateDocumentSlice\b/);
    expect(generationGate).toMatch(/\bcreateGenerationGate\b/);
    expect(documentSlice).not.toMatch(/function\s+createSerialQueue\b/);
    expect(documentActions).not.toMatch(/function\s+createSerialQueue\b/);

    const queueCallSites = Object.entries(PRODUCTION_SOURCES).flatMap(
      ([path, text]) => {
        if (/\.test\.(?:ts|tsx)$/.test(path)) return [];
        if (!/from\s+["'][^"']*ipcQueue["']/.test(text)) return [];
        return [...text.matchAll(/\bcreateSerialQueue\(\)/g)].map(() => path);
      },
    );
    expect(queueCallSites.sort()).toEqual([
      "./services/commandService.ts",
      "./services/foldAllRuntime.ts",
    ]);

    const ipcQueue = source("./ipcQueue.ts");
    const bytes = new TextEncoder().encode(ipcQueue);
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    const sha256 = [...new Uint8Array(digest)]
      .map((byte) => byte.toString(16).padStart(2, "0"))
      .join("")
      .toUpperCase();
    expect(bytes).toHaveLength(4_627);
    expect(sha256).toBe(
      "783DF611311B2A5436DD5D636214428AC0C96DE5AD03883B3CCB9E84A4C1CC79",
    );
  });

  it("B1 removes the viewerHint/appStore cycle through a zero-import ToolId leaf", () => {
    const toolTypes = source("./toolTypes.ts");
    const viewerHint = source("../lib/viewerHint.ts");

    expect(toolTypes).toMatch(/export\s+type\s+ToolId\b/);
    expect(toolTypes).not.toMatch(/^\s*import\b/m);
    expect(viewerHint).toContain('from "../store/toolTypes"');
    expect(viewerHint).not.toContain('from "../store/appStore"');
    expect(source("./appStore.ts")).toContain(
      'export type { ToolId } from "./toolTypes"',
    );
  });

  it("B2 owns pose, replay, fold-all preview, and their histories", () => {
    const slice = source("./slices/poseReplaySlice.ts");
    const actions = source("./services/poseReplayActions.ts");
    const poseRuntime = source("./services/poseRuntime.ts");
    const replayRuntime = source("./services/replayRuntime.ts");
    const foldAllRuntime = source("./services/foldAllRuntime.ts");
    const implementation = [
      actions,
      poseRuntime,
      replayRuntime,
      foldAllRuntime,
    ].join("\n");
    const facade = source("./appStore.ts");

    expect(actions).toMatch(/function\s+createPoseReplaySlice\b/);
    expect(facade).toContain(
      "interface AppState extends DocumentSlice, PoseReplaySlice",
    );
    expect(facade.match(/createPoseReplaySlice<AppState>\(/g)).toHaveLength(1);
    expect(facade.match(/\.\.\.poseReplay\.slice/g)).toHaveLength(1);
    for (const name of B2_STATE) {
      expect(implementation, name).toMatch(
        new RegExp(`^    ${name}:`, "m"),
      );
      expect(facade, name).not.toMatch(new RegExp(`^    ${name}:`, "m"));
    }
    for (const name of B2_ACTIONS) {
      expect(implementation, name).toMatch(
        new RegExp(`(?:\\b(?:const|function)\\s+${name}\\b|^    ${name}:)`, "m"),
      );
      expect(facade, name).not.toMatch(new RegExp(`^    ${name}:`, "m"));
      expect(useAppStore.getState()[name], name).toBeTypeOf("function");
    }

    for (const name of [
      "runPoseSolve",
      "runReplay",
      "runFoldAllPreview",
      "restoreAfterFoldAllPreviewOnce",
      "restoreAfterFoldAllPreview",
    ]) {
      expect(implementation, name).toMatch(
        new RegExp(`(?:function|const)\\s+${name}\\b`),
      );
    }
    expect(slice).toMatch(/FOLD_ALL_THROTTLE_MS\s*=\s*16\b/);
    expect(foldAllRuntime).toMatch(/runLatest\s*\(/);
    expect(foldAllRuntime).toContain("next_warm_seed");
    expect(foldAllRuntime).toMatch(/docEpoch/);
    expect(foldAllRuntime).toMatch(/foldAllSessionGeneration/);
    expect(foldAllRuntime).toMatch(/foldAllRequestGeneration/);
    expect(foldAllRuntime).toMatch(/foldAllExitGeneration/);
    expect(foldAllRuntime).toMatch(/foldAllEnterGeneration/);
    expect(foldAllRuntime).toMatch(
      /const finishFoldAllPercent[\s\S]*?active\.percent === 0[\s\S]*?restoreAfterFoldAllPreview\(true\)/,
    );
    const returnStateDeclaration = slice.match(
      /export interface FoldAllReturnState\s*\{([\s\S]*?)\n\}/,
    );
    expect(returnStateDeclaration).not.toBeNull();
    for (const name of B2_FOLD_ALL_RETURN_STATE) {
      expect(returnStateDeclaration![1], name).toMatch(
        new RegExp(`^  ${name}:`, "m"),
      );
    }
    expect(slice).not.toMatch(/from\s+["']zustand["']/);
    for (const line of implementation
      .split(/\r?\n/)
      .filter((candidate) => /from\s+["']zustand["']/.test(candidate))) {
      expect(line.trim()).toMatch(/^import type\b/);
    }

    type MissingAction = Exclude<
      keyof PoseReplaySliceActions,
      (typeof B2_ACTIONS)[number]
    >;
    type ExtraAction = Exclude<
      (typeof B2_ACTIONS)[number],
      keyof PoseReplaySliceActions
    >;
    type MissingState = Exclude<
      keyof PoseReplaySliceState,
      (typeof B2_STATE)[number]
    >;
    type ExtraState = Exclude<
      (typeof B2_STATE)[number],
      keyof PoseReplaySliceState
    >;
    expectTypeOf<MissingAction>().toEqualTypeOf<never>();
    expectTypeOf<ExtraAction>().toEqualTypeOf<never>();
    expectTypeOf<MissingState>().toEqualTypeOf<never>();
    expectTypeOf<ExtraState>().toEqualTypeOf<never>();
    type MissingFoldAllReturnState = Exclude<
      keyof FoldAllReturnState,
      (typeof B2_FOLD_ALL_RETURN_STATE)[number]
    >;
    type ExtraFoldAllReturnState = Exclude<
      (typeof B2_FOLD_ALL_RETURN_STATE)[number],
      keyof FoldAllReturnState
    >;
    expectTypeOf<MissingFoldAllReturnState>().toEqualTypeOf<never>();
    expectTypeOf<ExtraFoldAllReturnState>().toEqualTypeOf<never>();

    type StoreState = ReturnType<typeof useAppStore.getState>;
    type ComposedPoseReplaySlice = Pick<StoreState, keyof PoseReplaySlice>;
    type StoreExtendsSlice = ComposedPoseReplaySlice extends PoseReplaySlice
      ? true
      : false;
    type SliceExtendsStore = PoseReplaySlice extends ComposedPoseReplaySlice
      ? true
      : false;
    expectTypeOf<StoreExtendsSlice>().toEqualTypeOf<true>();
    expectTypeOf<SliceExtendsStore>().toEqualTypeOf<true>();

    for (const name of B2_PUBLIC_VALUES) {
      expect(appStoreFacade[name], name).toBe(poseReplayModule[name]);
    }
    expect(appStoreFacade.resetPoseThrottle).toBe(
      poseRuntimeModule.resetPoseThrottle,
    );
    expect(appStoreFacade.resetFoldAllPreviewRuntime).toBe(
      foldAllRuntimeModule.resetFoldAllPreviewRuntime,
    );
    const publicTypeReexport = facade.match(
      /export\s+type\s*\{([^}]*)\}\s+from\s+"\.\/slices\/poseReplaySlice"/,
    );
    expect(publicTypeReexport).not.toBeNull();
    const reexportedPoseTypes = publicTypeReexport![1]
      .split(",")
      .map((name) => name.trim())
      .filter(Boolean)
      .sort();
    expect(reexportedPoseTypes).toEqual([...B2_PUBLIC_TYPES].sort());
    for (const name of B2_PUBLIC_TYPES) {
      expect(facade, name).not.toMatch(
        new RegExp(`export\\s+(?:interface|type)\\s+${name}\\b`),
      );
    }

    const lineCount = (text: string): number => {
      const lines = text.replace(/\r\n/g, "\n").split("\n");
      if (lines[lines.length - 1] === "") lines.pop();
      return lines.length;
    };
    expect(lineCount(slice)).toBeLessThanOrEqual(1_000);
    for (const service of [
      actions,
      poseRuntime,
      replayRuntime,
      foldAllRuntime,
    ]) {
      expect(lineCount(service)).toBeLessThanOrEqual(1_500);
    }
  });

  it("B3 owns proposal state, job generations, timers, and position history", () => {
    const slice = source("./slices/proposalSlice.ts");
    const actions = source("./services/proposalActions.ts");
    const facade = source("./appStore.ts");

    expect(actions).toMatch(/function\s+createProposalSlice\b/);
    expect(facade).toContain(
      "interface AppState extends DocumentSlice, PoseReplaySlice, ProposalSlice",
    );
    expect(facade.match(/createProposalSlice<AppState>\(/g)).toHaveLength(1);
    expect(facade.match(/\.\.\.proposalSlice\.slice/g)).toHaveLength(1);
    for (const name of B3_STATE) {
      expect(actions, name).toMatch(new RegExp(`^    ${name}:`, "m"));
      expect(facade, name).not.toMatch(new RegExp(`^    ${name}:`, "m"));
    }
    for (const name of B3_ACTIONS) {
      expect(actions, name).toMatch(
        new RegExp(`(?:\\b(?:const|function)\\s+${name}\\b|^    ${name}:)`, "m"),
      );
      expect(facade, name).not.toMatch(new RegExp(`^    ${name}:`, "m"));
      expect(useAppStore.getState()[name], name).toBeTypeOf("function");
    }
    for (const name of [
      "isCurrentProposalJob",
      "stopProposalJobTimers",
      "requestProposalCancel",
      "invalidateProposalJob",
      "startProposalJob",
      "warnProposalProgress",
      "watchProposalProgress",
      "holdFullProposalBar",
      "runProposalGeneration",
      "proposalPositionSnapshot",
      "pushProposalPositionUndo",
      "undoProposalPositionState",
      "redoProposalPositionState",
    ]) {
      expect(actions, name).toMatch(
        new RegExp(`(?:function|const)\\s+${name}\\b`),
      );
    }
    expect(actions).toMatch(/proposalGeneration/);
    expect(actions).toMatch(/activeProposalJob/);
    expect(actions).toMatch(/result\.job_id\s*!==\s*job\.jobId/);
    expect(actions).toMatch(/isCurrentProposalJob\(job\)/);
    expect(slice).not.toMatch(/from\s+["']zustand["']/);
    for (const line of actions
      .split(/\r?\n/)
      .filter((candidate) => /from\s+["']zustand["']/.test(candidate))) {
      expect(line.trim()).toMatch(/^import type\b/);
    }

    type MissingAction = Exclude<
      keyof ProposalSliceActions,
      (typeof B3_ACTIONS)[number]
    >;
    type ExtraAction = Exclude<
      (typeof B3_ACTIONS)[number],
      keyof ProposalSliceActions
    >;
    type MissingState = Exclude<
      keyof ProposalSliceState,
      (typeof B3_STATE)[number]
    >;
    type ExtraState = Exclude<
      (typeof B3_STATE)[number],
      keyof ProposalSliceState
    >;
    expectTypeOf<MissingAction>().toEqualTypeOf<never>();
    expectTypeOf<ExtraAction>().toEqualTypeOf<never>();
    expectTypeOf<MissingState>().toEqualTypeOf<never>();
    expectTypeOf<ExtraState>().toEqualTypeOf<never>();

    type StoreState = ReturnType<typeof useAppStore.getState>;
    type ComposedProposalSlice = Pick<StoreState, keyof ProposalSlice>;
    type StoreExtendsSlice = ComposedProposalSlice extends ProposalSlice
      ? true
      : false;
    type SliceExtendsStore = ProposalSlice extends ComposedProposalSlice
      ? true
      : false;
    expectTypeOf<StoreExtendsSlice>().toEqualTypeOf<true>();
    expectTypeOf<SliceExtendsStore>().toEqualTypeOf<true>();

    const publicTypeReexport = facade.match(
      /export\s+type\s*\{([^}]*)\}\s+from\s+"\.\/slices\/proposalSlice"/,
    );
    expect(publicTypeReexport).not.toBeNull();
    const reexportedProposalTypes = publicTypeReexport![1]
      .split(",")
      .map((name) => name.trim())
      .filter(Boolean)
      .sort();
    expect(reexportedProposalTypes).toEqual([...B3_PUBLIC_TYPES].sort());
    for (const name of B3_PUBLIC_TYPES) {
      expect(facade, name).not.toMatch(
        new RegExp(`export\\s+(?:interface|type)\\s+${name}\\b`),
      );
    }

    const lineCount = (text: string): number => {
      const lines = text.replace(/\r\n/g, "\n").split("\n");
      if (lines[lines.length - 1] === "") lines.pop();
      return lines.length;
    };
    expect(lineCount(slice)).toBeLessThanOrEqual(1_000);
    expect(lineCount(actions)).toBeLessThanOrEqual(1_500);
  });

  it("B4 owns dialogs, settings, onboarding, help, and transient UI state", () => {
    const slice = source("./slices/dialogSettingsSlice.ts");
    const actions = source("./services/dialogSettingsActions.ts");
    const facade = source("./appStore.ts");

    expect(actions).toMatch(/function\s+createDialogSettingsSlice\b/);
    expect(facade).toMatch(
      /interface\s+AppState\s+extends[^{]*\bDialogSettingsSlice\b[^{]*\{/,
    );
    expect(facade.match(/createDialogSettingsSlice<AppState>\(/g)).toHaveLength(1);
    expect(facade.match(/\.\.\.dialogSettingsSlice\.slice/g)).toHaveLength(1);

    for (const name of B4_STATE) {
      expect(actions, name).toMatch(new RegExp(`^    ${name}:`, "m"));
      expect(facade, name).not.toMatch(new RegExp(`^    ${name}:`, "m"));
      expect(facade, name).not.toMatch(new RegExp(`^  ${name}:`, "m"));
    }
    expectTypeOf<DialogSettingsSliceState["exportKind"]>().toEqualTypeOf<
      DocumentExportKind
    >();
    expectTypeOf<ExportSettings["exportKind"]>().toEqualTypeOf<
      DocumentExportKind
    >();
    expectTypeOf<DialogSettingsSliceState["exportFoldIssues"]>().toEqualTypeOf<
      FoldIssue[]
    >();
    expect(useAppStore.getState().exportFoldIssues).toEqual([]);
    for (const name of B4_ACTIONS) {
      expect(actions, name).toMatch(
        new RegExp(`(?:\\b(?:const|function)\\s+${name}\\b|^    ${name}:)`, "m"),
      );
      expect(facade, name).not.toMatch(new RegExp(`^    ${name}:`, "m"));
      expect(facade, name).not.toMatch(new RegExp(`^  ${name}:`, "m"));
      expect(useAppStore.getState()[name], name).toBeTypeOf("function");
    }

    type MissingAction = Exclude<
      keyof DialogSettingsSliceActions,
      (typeof B4_ACTIONS)[number]
    >;
    type ExtraAction = Exclude<
      (typeof B4_ACTIONS)[number],
      keyof DialogSettingsSliceActions
    >;
    type MissingState = Exclude<
      keyof DialogSettingsSliceState,
      (typeof B4_STATE)[number]
    >;
    type ExtraState = Exclude<
      (typeof B4_STATE)[number],
      keyof DialogSettingsSliceState
    >;
    expectTypeOf<MissingAction>().toEqualTypeOf<never>();
    expectTypeOf<ExtraAction>().toEqualTypeOf<never>();
    expectTypeOf<MissingState>().toEqualTypeOf<never>();
    expectTypeOf<ExtraState>().toEqualTypeOf<never>();

    type StoreState = ReturnType<typeof useAppStore.getState>;
    type ComposedDialogSettingsSlice = Pick<
      StoreState,
      keyof DialogSettingsSlice
    >;
    type StoreExtendsSlice =
      ComposedDialogSettingsSlice extends DialogSettingsSlice ? true : false;
    type SliceExtendsStore =
      DialogSettingsSlice extends ComposedDialogSettingsSlice ? true : false;
    expectTypeOf<StoreExtendsSlice>().toEqualTypeOf<true>();
    expectTypeOf<SliceExtendsStore>().toEqualTypeOf<true>();
    const selectExportOpen = (state: StoreState) => state.exportOpen;
    expectTypeOf(selectExportOpen).returns.toEqualTypeOf<boolean>();

    for (const name of B4_PUBLIC_VALUES) {
      expect(appStoreFacade[name], name).toBe(dialogSettingsModule[name]);
      expect(facade, name).not.toMatch(
        new RegExp(`export\\s+(?:const|function)\\s+${name}\\b`),
      );
    }
    const publicTypeReexport = facade.match(
      /export\s+type\s*\{([^}]*)\}\s+from\s+"\.\/slices\/dialogSettingsSlice"/,
    );
    expect(publicTypeReexport).not.toBeNull();
    const reexportedDialogTypes = publicTypeReexport![1]
      .split(",")
      .map((name) => name.trim())
      .filter(Boolean)
      .sort();
    expect(reexportedDialogTypes).toEqual([...B4_PUBLIC_TYPES].sort());
    for (const name of B4_PUBLIC_TYPES) {
      expect(slice, name).toMatch(
        new RegExp(`export\\s+(?:interface|type)\\s+${name}\\b`),
      );
      expect(facade, name).not.toMatch(
        new RegExp(`export\\s+(?:interface|type)\\s+${name}\\b`),
      );
    }

    expect(slice).not.toMatch(/from\s+["']zustand["']/);
    for (const line of actions
      .split(/\r?\n/)
      .filter((candidate) => /from\s+["']zustand["']/.test(candidate))) {
      expect(line.trim()).toMatch(/^import type\b/);
    }
    const lineCount = (text: string): number => {
      const lines = text.replace(/\r\n/g, "\n").split("\n");
      if (lines[lines.length - 1] === "") lines.pop();
      return lines.length;
    };
    expect(lineCount(slice)).toBeLessThanOrEqual(1_000);
    expect(lineCount(actions)).toBeLessThanOrEqual(1_500);
  });

  it("keeps useAppStore selectors typed and the production Zustand store count at one", () => {
    type StoreState = ReturnType<typeof useAppStore.getState>;
    const selectDocument = (state: StoreState) => state.doc;
    expectTypeOf(selectDocument).returns.toEqualTypeOf<Document | null>();

    const zustandFactories = Object.entries(PRODUCTION_SOURCES).flatMap(
      ([path, text]) => {
      if (/\.test\.(?:ts|tsx)$/.test(path)) return [];
      if (!/from\s+["']zustand["']/.test(text)) return [];
      return [...text.matchAll(/\bcreate\s*</g)].map(() => path);
      },
    );
    expect(zustandFactories).toHaveLength(1);
    expect(zustandFactories[0]).toBe("./appStore.ts");
  });
});
