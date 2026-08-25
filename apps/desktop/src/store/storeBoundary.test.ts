import { readFileSync } from "node:fs";
import { describe, expect, expectTypeOf, it } from "vitest";
import type { Document } from "../lib/types";
import * as appStoreFacade from "./appStore";
import { useAppStore } from "./appStore";
import * as documentModule from "./slices/documentSlice";
import type {
  DocumentSlice,
  DocumentSliceActions,
  DocumentSliceState,
} from "./slices/documentSlice";
import type {
  PoseReplaySlice,
  PoseReplaySliceActions,
  PoseReplaySliceState,
} from "./slices/poseReplaySlice";

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
] as const satisfies readonly (keyof PoseReplaySliceActions)[];

const B2_STATE = [
  "frame3d",
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
  "togglePinnedFold",
  "setPinnedFolds",
  "recordPoseStep",
  "moveStep",
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
    expect(facade).toContain("interface AppState extends DocumentSlice {");
    expect(facade.match(/createDocumentSlice<AppState>\(/g)).toHaveLength(1);
    expect(facade.match(/\.\.\.documentSlice\.slice/g)).toHaveLength(1);
    const composedReturnStart = normalizedFacade.indexOf(
      "  return {\n    ...documentSlice.slice",
    );
    expect(composedReturnStart).toBeGreaterThanOrEqual(0);
    const composedReturn = normalizedFacade.slice(composedReturnStart);
    const appStateStart = normalizedFacade.indexOf(
      "interface AppState extends DocumentSlice {",
    );
    const appStateEnd = normalizedFacade.indexOf(
      "\n}\n\n/** 書き出しダイアログ",
      appStateStart,
    );
    expect(appStateStart).toBeGreaterThanOrEqual(0);
    expect(appStateEnd).toBeGreaterThan(appStateStart);
    const appStateDeclaration = normalizedFacade.slice(
      appStateStart,
      appStateEnd,
    );

    for (const name of B1_STATE) {
      expect(documentActions, name).toMatch(new RegExp(`^    ${name}:`, "m"));
      expect(composedReturn, name).not.toMatch(new RegExp(`^    ${name}:`, "m"));
      expect(appStateDeclaration, name).not.toMatch(
        new RegExp(`^  ${name}:`, "m"),
      );
    }

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

    const lineCount = (text: string): number => {
      const lines = text.replace(/\r\n/g, "\n").split("\n");
      if (lines[lines.length - 1] === "") lines.pop();
      return lines.length;
    };
    expect(lineCount(documentSlice)).toBeLessThanOrEqual(1_000);
    expect(lineCount(documentActions)).toBeLessThanOrEqual(1_500);
    expect(lineCount(source("./services/commandService.ts"))).toBeLessThanOrEqual(
      1_500,
    );
    expect(lineCount(source("./services/generationGate.ts"))).toBeLessThanOrEqual(
      1_500,
    );
    expect(lineCount(source("./toolTypes.ts"))).toBeLessThanOrEqual(1_500);
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
      "./appStore.ts",
      "./services/commandService.ts",
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
    const facade = source("./appStore.ts");

    expect(actions).toMatch(/function\s+createPoseReplaySlice\b/);
    expect(facade.match(/createPoseReplaySlice<AppState>\(/g)).toHaveLength(1);
    expect(facade.match(/\.\.\.poseReplaySlice\.slice/g)).toHaveLength(1);
    for (const name of B2_ACTIONS) {
      expect(actions, name).toMatch(new RegExp(`^    ${name}:`, "m"));
      expect(facade, name).not.toMatch(new RegExp(`^    ${name}:`, "m"));
    }

    for (const name of [
      "runPoseSolve",
      "runReplay",
      "runFoldAllPreview",
      "restoreAfterFoldAllPreviewOnce",
      "restoreAfterFoldAllPreview",
    ]) {
      expect(actions, name).toMatch(new RegExp(`(?:function|const)\\s+${name}\\b`));
    }
    expect(actions).toMatch(/FOLD_ALL_THROTTLE_MS\s*=\s*16\b/);
    expect(actions).toContain("next_warm_seed");
    expect(actions).toMatch(/docEpoch/);
    expect(actions).toMatch(/foldAllSessionGeneration/);
    expect(actions).toMatch(/foldAllRequestGeneration/);
    expect(slice).not.toMatch(/from\s+["']zustand["']/);
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
