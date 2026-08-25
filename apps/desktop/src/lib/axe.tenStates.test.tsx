// @vitest-environment jsdom

import { useRef, type ReactElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import axe, { type AxeResults, type Result } from "axe-core";
import { CpEditor } from "../components/CpEditor/CpEditor";
import { RecoveryDialog } from "../components/RecoveryDialog";
import { ExportDialog } from "../components/dialogs/ExportDialog";
import { HelpCenter } from "../components/dialogs/HelpCenter";
import { NewDocumentDialog } from "../components/dialogs/NewDocumentDialog";
import { ProposalWizard } from "../components/dialogs/ProposalWizard";
import {
  LENGTH_RANGE,
  ROOT_ID,
  WIDTH_RANGE,
  addLimb,
  defaultSkeleton,
  leafNodes,
} from "./skeleton";
import type {
  CreasePattern,
  Document,
  FoldStep,
  ProposalCandidate,
  Skeleton,
} from "./types";
import { useAppStore } from "../store/appStore";

vi.mock("../components/CpEditor/renderer", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("../components/CpEditor/renderer")>();
  return { ...actual, render: vi.fn() };
});

function makeCp(mark: number): CreasePattern {
  return {
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
    next_edge_id: mark,
  };
}

function makeSteps(count: number): FoldStep[] {
  return Array.from({ length: count }, (_, index) => ({
    id: index + 1,
    kind: "Simple",
    drivers: [],
    layer_order: null,
    note: "",
  }));
}

const BASE_DOCUMENT: Document = {
  schema_version: 1,
  paper: { width_mm: 150, height_mm: 150 },
  cp: makeCp(4),
  sequence: [],
  display: {
    front_color: [237, 28, 36],
    back_color: [255, 255, 255],
    grid_divisions: 8,
  },
};

const LONG_EXPORT_BASENAME =
  "折り鶴の展開図_利用者調査で文字切れを確認するための非常に長い作品名_最終版.png";
const LONG_EXPORT_ERROR =
  "選んだ場所へ保存できませんでした。空き容量と書き込みできる場所かを確かめてから、別の保存先でもう一度試してください。";

function makeCandidate(
  mark: number,
  leafIds: readonly number[] = [],
): ProposalCandidate {
  return {
    cp: makeCp(mark),
    scale: 0.4,
    violations: 0,
    warnings: [],
    fold_plan: null,
    sites: leafIds.map((leafId, index) => ({
      circle: {
        leaf_id: leafId,
        circle_index: index,
        center: [
          ((index % 4) + 0.5) / 4,
          (Math.floor(index / 4) + 0.5) /
            Math.max(1, Math.ceil(leafIds.length / 4)),
        ] as [number, number],
        radius: 0.04,
      },
      vertex: null,
      molecules: [],
    })),
  };
}

function withLargestParts(skeleton: Skeleton): Skeleton {
  return {
    nodes: skeleton.nodes.map((node) =>
      node.parent === null
        ? node
        : {
            ...node,
            length: LENGTH_RANGE.max,
            width_factor: WIDTH_RANGE.max,
          },
    ),
  };
}

/** 5-Aで固定した「深い形」: 深さ4、先端12本の最大標本。 */
function deepTwelveTipBranch(): Skeleton {
  let skeleton = defaultSkeleton();
  skeleton = addLimb(skeleton, 1);
  const second = skeleton.nodes.find((node) => node.parent === 1);
  if (second === undefined) throw new Error("深い形の2段目がありません");
  skeleton = addLimb(skeleton, second.id);
  const third = skeleton.nodes.find((node) => node.parent === second.id);
  if (third === undefined) throw new Error("深い形の3段目がありません");
  for (let index = 0; index < 9; index += 1) {
    skeleton = addLimb(skeleton, third.id);
  }
  expect(leafNodes(skeleton)).toHaveLength(12);
  return withLargestParts(skeleton);
}

function radialSkeleton(count: number): Skeleton {
  return {
    nodes: [
      { id: ROOT_ID, parent: null, length: 0, width_factor: 1 },
      ...Array.from({ length: count }, (_, index) => ({
        id: index + 1,
        parent: ROOT_ID,
        length: 1,
        width_factor: 1,
      })),
    ],
  };
}

function resetVisibleState(): void {
  useAppStore.setState({
    doc: structuredClone(BASE_DOCUMENT),
    recovery: null,
    newDialogOpen: false,
    proposalStep: null,
    proposalSkeleton: defaultSkeleton(),
    proposalCandidates: [],
    proposalSelected: null,
    proposalPaperSource: null,
    proposalPaperPositions: [],
    proposalPaperSpecified: [],
    proposalPositionLastMoved: [],
    proposalPositionUndoStack: [],
    proposalPositionRedoStack: [],
    proposalBusy: false,
    proposalJobId: null,
    proposalProgress: null,
    proposalProgressWarning: null,
    proposalError: null,
    exportOpen: false,
    exportBusy: false,
    exportError: null,
    exportSavedPath: null,
    helpOpen: false,
    helpQuery: "",
    guideOpen: false,
    activeTool: "select",
    operationStage: 0,
    lineInputStart: null,
  });
}

function CpLineStartState(): ReactElement {
  const fitRef = useRef<(() => void) | null>(null);
  return (
    <section className="pane pane-2d" data-ten-state="cp-line-start">
      <CpEditor fitRef={fitRef} />
    </section>
  );
}

interface TenState {
  id: string;
  label: string;
  ownerFiles: readonly string[];
  floatingUi: string | null;
  prepare: () => void;
  view: () => ReactElement;
  afterMount?: (root: HTMLElement) => void;
}

const TEN_STATES: readonly TenState[] = [
  {
    id: "new-square",
    label: "新規作成・既定の正方形",
    ownerFiles: [
      "apps/desktop/src/components/dialogs/NewDocumentDialog.tsx",
      "apps/desktop/src/components/dialogs/ModalDialog.tsx",
    ],
    floatingUi: "new-document-dialog",
    prepare: () => {
      useAppStore.setState({
        newDialogOpen: true,
        newPaperDraft: { widthMm: 150, heightMm: 150, square: true },
      });
    },
    view: () => <NewDocumentDialog />,
  },
  {
    id: "export-png-long-messages",
    label: "書き出し・PNGと長い成功／失敗文",
    ownerFiles: [
      "apps/desktop/src/components/dialogs/ExportDialog.tsx",
      "apps/desktop/src/components/dialogs/ModalDialog.tsx",
    ],
    floatingUi: "export-dialog",
    prepare: () => {
      useAppStore.setState({
        doc: { ...structuredClone(BASE_DOCUMENT), sequence: makeSteps(1) },
        exportOpen: true,
        exportKind: "CpPng",
        exportIncludeAux: true,
        exportLongSide: 2048,
        exportSavedPath:
          `C:\\利用者\\折り鶴の作品\\とても長い名前の展開図を保存した場所\\${LONG_EXPORT_BASENAME}`,
        exportError: LONG_EXPORT_ERROR,
      });
    },
    view: () => <ExportDialog />,
  },
  {
    id: "proposal-skeleton-deep",
    label: "提案・形を決める（深い形）",
    ownerFiles: ["apps/desktop/src/components/dialogs/SkeletonPreview.tsx"],
    floatingUi: "proposal-dialog",
    prepare: () => {
      useAppStore.setState({
        proposalStep: "skeleton",
        proposalSkeleton: deepTwelveTipBranch(),
      });
    },
    view: () => <ProposalWizard />,
  },
  {
    id: "proposal-four-candidates",
    label: "提案・4候補",
    ownerFiles: [
      "apps/desktop/src/components/dialogs/ProposalWizard.tsx",
      "apps/desktop/src/components/dialogs/ModalDialog.tsx",
    ],
    floatingUi: "proposal-dialog",
    prepare: () => {
      const skeleton = radialSkeleton(4);
      const leaves = leafNodes(skeleton).map((node) => node.id);
      useAppStore.setState({
        proposalStep: "candidates",
        proposalSkeleton: skeleton,
        proposalCandidates: Array.from({ length: 4 }, (_, index) =>
          makeCandidate(20 + index, leaves),
        ),
        proposalSelected: 0,
      });
    },
    view: () => <ProposalWizard />,
  },
  {
    id: "proposal-paper-twelve-handles",
    label: "提案・紙上の12個の場所",
    ownerFiles: [
      "apps/desktop/src/components/dialogs/ProposalWizard.tsx",
      "apps/desktop/src/components/dialogs/ModalDialog.tsx",
    ],
    floatingUi: "proposal-dialog",
    prepare: () => {
      const skeleton = radialSkeleton(12);
      const leaves = leafNodes(skeleton).map((node) => node.id);
      const candidate = makeCandidate(30, leaves);
      useAppStore.setState({
        proposalStep: "candidates",
        proposalSkeleton: skeleton,
        proposalCandidates: [candidate],
        proposalSelected: 0,
      });
      useAppStore.getState().openProposalPaperPositionEditor();
    },
    view: () => <ProposalWizard />,
  },
  {
    id: "proposal-confirm-warning",
    label: "提案・確認と既存手順消去警告",
    ownerFiles: [
      "apps/desktop/src/components/dialogs/ProposalWizard.tsx",
      "apps/desktop/src/components/dialogs/ModalDialog.tsx",
    ],
    floatingUi: "proposal-dialog",
    prepare: () => {
      useAppStore.setState({
        doc: { ...structuredClone(BASE_DOCUMENT), sequence: makeSteps(3) },
        proposalStep: "confirm",
        proposalCandidates: [makeCandidate(40)],
        proposalSelected: 0,
      });
    },
    view: () => <ProposalWizard />,
  },
  {
    id: "proposal-busy",
    label: "提案・処理中",
    ownerFiles: [
      "apps/desktop/src/components/dialogs/ProposalWizard.tsx",
      "apps/desktop/src/components/dialogs/ModalDialog.tsx",
    ],
    floatingUi: "proposal-dialog",
    prepare: () => {
      useAppStore.setState({
        proposalStep: "skeleton",
        proposalSkeleton: defaultSkeleton(),
        proposalBusy: true,
        proposalJobId: "ten-state-busy",
        proposalProgress: {
          job_id: "ten-state-busy",
          done: 2,
          total: 4,
          phase: "Generating",
        },
      });
    },
    view: () => <ProposalWizard />,
  },
  {
    id: "recovery-long-path",
    label: "復旧・長い実パス",
    ownerFiles: [
      "apps/desktop/src/components/RecoveryDialog.tsx",
      "apps/desktop/src/components/dialogs/ModalDialog.tsx",
    ],
    floatingUi: "recovery-dialog",
    prepare: () => {
      useAppStore.setState({
        recovery: {
          autosave_path:
            "C:\\Users\\利用者\\AppData\\Local\\ORIGAMI3\\控え\\非常に長い控えの名前.ori3",
          document_path:
            "C:\\Users\\利用者\\Documents\\折り紙作品\\2026年の展示会\\利用者調査で確認する非常に長い作品名\\折り鶴の最終作品.ori3",
          saved_at_ms: Date.UTC(2026, 7, 24, 12, 34, 56),
        },
      });
    },
    view: () => <RecoveryDialog />,
  },
  {
    id: "help-all-thirteen-chapters",
    label: "ヘルプ・全13章",
    ownerFiles: [
      "apps/desktop/src/components/dialogs/HelpCenter.tsx",
      "apps/desktop/src/components/dialogs/ModalDialog.tsx",
    ],
    floatingUi: "help-dialog",
    prepare: () => {
      useAppStore.setState({
        helpOpen: true,
        helpChapterId: "overview",
        helpQuery: "",
      });
    },
    view: () => <HelpCenter />,
  },
  {
    id: "cp-keyboard-line-start",
    label: "展開図・キーボードで始点を決めた途中状態",
    ownerFiles: [
      "apps/desktop/src/App.tsx",
      "apps/desktop/src/components/CpEditor/CpEditor.tsx",
    ],
    floatingUi: null,
    prepare: () => {
      useAppStore.setState({
        doc: structuredClone(BASE_DOCUMENT),
        docEpoch: 1,
        faces: [],
        hinges: new Set(),
        selection: { edgeIds: [], vertexIds: [] },
        activeTool: "mountain",
        operationStage: 0,
        lineInputStart: null,
        currentStep: null,
      });
    },
    view: () => <CpLineStartState />,
    afterMount: (root) => {
      const canvas = root.querySelector<HTMLCanvasElement>("canvas.cp-canvas");
      if (canvas === null) throw new Error("展開図の操作画面がありません");
      act(() => canvas.focus());
      fireEvent.keyDown(canvas, { key: "Enter" });
      fireEvent.keyUp(canvas, { key: "Enter" });
      expect(useAppStore.getState().lineInputStart).not.toBeNull();
      expect(useAppStore.getState().operationStage).toBe(1);
    },
  },
] as const;

const initialStoreState = useAppStore.getState();
const originalResizeObserver = globalThis.ResizeObserver;
const originalCanvasContext = HTMLCanvasElement.prototype.getContext;
const originalDocumentLanguage = document.documentElement.lang;
const originalDocumentTitle = document.title;
const originalCanvasWidth = Object.getOwnPropertyDescriptor(
  HTMLCanvasElement.prototype,
  "clientWidth",
);
const originalCanvasHeight = Object.getOwnPropertyDescriptor(
  HTMLCanvasElement.prototype,
  "clientHeight",
);
const originalCanvasRect = HTMLCanvasElement.prototype.getBoundingClientRect;

beforeEach(() => {
  // 製品のindex.htmlと同じ文書情報を置き、component断片ではなく画面全体を走査する。
  document.documentElement.lang = "ja";
  document.title = "ORIGAMI3";
  Object.defineProperties(HTMLCanvasElement.prototype, {
    clientWidth: { configurable: true, value: 400 },
    clientHeight: { configurable: true, value: 400 },
  });
  HTMLCanvasElement.prototype.getContext = vi.fn(() => ({})) as never;
  HTMLCanvasElement.prototype.getBoundingClientRect = vi.fn(
    () =>
      ({
        x: 0,
        y: 0,
        width: 400,
        height: 400,
        top: 0,
        right: 400,
        bottom: 400,
        left: 0,
        toJSON: () => ({}),
      }) as DOMRect,
  );
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  resetVisibleState();
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
  document.documentElement.lang = originalDocumentLanguage;
  document.title = originalDocumentTitle;
  HTMLCanvasElement.prototype.getContext = originalCanvasContext;
  HTMLCanvasElement.prototype.getBoundingClientRect = originalCanvasRect;
  if (originalCanvasWidth !== undefined) {
    Object.defineProperty(
      HTMLCanvasElement.prototype,
      "clientWidth",
      originalCanvasWidth,
    );
  }
  if (originalCanvasHeight !== undefined) {
    Object.defineProperty(
      HTMLCanvasElement.prototype,
      "clientHeight",
      originalCanvasHeight,
    );
  }
  globalThis.ResizeObserver = originalResizeObserver;
});

function mountState(state: TenState): HTMLElement {
  state.prepare();
  render(state.view());

  let root: HTMLElement | null;
  if (state.floatingUi === null) {
    root = document.querySelector(
      '[data-ten-state="cp-line-start"] .cp-editor',
    );
  } else {
    root = document.querySelector(`[data-floating-ui="${state.floatingUi}"]`);
  }
  if (root === null) throw new Error(`${state.label}の製品画面がありません`);
  state.afterMount?.(root);
  return root;
}

function relevant(results: readonly Result[]): Result[] {
  return results.filter(
    (result) => result.impact === "critical" || result.impact === "serious",
  );
}

function formatResults(
  state: TenState,
  kind: "violations" | "incomplete",
  results: readonly Result[],
): string {
  if (results.length === 0) return `${state.label}: ${kind}=0`;
  const details = results.flatMap((result) =>
    result.nodes.map((node, nodeIndex) =>
      [
        `[${state.label}] ${kind} ${result.impact ?? "impact不明"} ${result.id}`,
        `説明: ${result.help}`,
        `対象${nodeIndex + 1}: ${node.target.join(" -> ")}`,
        `HTML: ${node.html}`,
        `要約: ${node.failureSummary ?? "要約なし"}`,
        `必要な製品ファイル: ${state.ownerFiles.join(", ")}`,
      ].join("\n"),
    ),
  );
  return details.join("\n\n");
}

async function runAllRules(): Promise<AxeResults> {
  // runOnly・rules・excludeを渡さず、4.12.1で既定有効の全規則を実行する。
  return axe.run(document, {
    resultTypes: ["violations", "incomplete", "passes", "inapplicable"],
  });
}

describe("5-E: axe-core 4.12.1による固定10状態の既定有効全規則監査", () => {
  it("固定した10状態を増減させない", () => {
    expect(TEN_STATES.map((state) => state.id)).toEqual([
      "new-square",
      "export-png-long-messages",
      "proposal-skeleton-deep",
      "proposal-four-candidates",
      "proposal-paper-twelve-handles",
      "proposal-confirm-warning",
      "proposal-busy",
      "recovery-long-path",
      "help-all-thirteen-chapters",
      "cp-keyboard-line-start",
    ]);
  });

  it.each(TEN_STATES)(
    "$label: critical／serious違反が0件",
    async (state) => {
      const root = mountState(state);
      if (state.id === "help-all-thirteen-chapters") {
        expect(screen.getByText("全13章")).not.toBeNull();
        expect(root.querySelectorAll(".help-toc > button")).toHaveLength(13);
      }
      if (state.id === "proposal-paper-twelve-handles") {
        expect(root.querySelectorAll("[data-paper-position-handle]")).toHaveLength(
          12,
        );
      }

      const results = await runAllRules();
      expect(results.testEngine.version).toBe("4.12.1");

      const highViolations = relevant(results.violations);
      const incomplete = relevant(results.incomplete);
      const incompleteReport = formatResults(state, "incomplete", incomplete);

      expect(
        highViolations,
        `${formatResults(state, "violations", highViolations)}\n\n` +
          `jsdomで判定未完了（合格扱いに含めない）:\n${incompleteReport}`,
      ).toEqual([]);
    },
  );
});
