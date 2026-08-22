import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { EXPORT_CHOICES } from "../components/dialogs/ExportDialog";
import { HELP_CHAPTERS } from "../help/helpContent";
import { SUPPORTED_TECHNIQUES } from "./techniques";
import {
  ALIGN_MODES_ARE_EXHAUSTIVE,
  ALL_SCREEN_SCENARIOS,
  AUDITED_ALIGN_MODES,
  AUDITED_CONSTRUCT_KINDS,
  AUDITED_EXPORT_KINDS,
  AUDITED_FLOATING_UI_IDS,
  AUDITED_GUIDE_STEPS,
  AUDITED_HELP_CHAPTER_IDS,
  AUDITED_MEASURE_MODES,
  AUDITED_PROPOSAL_STEPS,
  AUDITED_TECHNIQUE_KINDS,
  AUDITED_TOOL_IDS,
  CONSTRUCT_KINDS_ARE_EXHAUSTIVE,
  EXPORT_KINDS_ARE_EXHAUSTIVE,
  GUIDE_STEPS_ARE_EXHAUSTIVE,
  HELP_CHAPTER_IDS_ARE_EXHAUSTIVE,
  MEASURE_MODES_ARE_EXHAUSTIVE,
  MINIMUM_APP_VIEWPORT,
  PROPOSAL_STEPS_ARE_EXHAUSTIVE,
  TECHNIQUE_KINDS_ARE_EXHAUSTIVE,
  TOOL_IDS_ARE_EXHAUSTIVE,
  type ScreenLayoutContract,
  type ScreenScenarioCoverage,
} from "./allScreenScenarios";

const css = readFileSync(new URL("../App.css", import.meta.url), "utf8");
const tooltipSource = readFileSync(
  new URL("../components/Tooltip.tsx", import.meta.url),
  "utf8",
);

const tsxModules = import.meta.glob<string>(
  ["../App.tsx", "../components/**/*.tsx"],
  { eager: true, query: "?raw", import: "default" },
);
const productTsxSources = Object.entries(tsxModules)
  .filter(([path]) => !path.includes(".test."))
  .map(([, source]) => source);

type CoverageArrayKey = Exclude<keyof ScreenScenarioCoverage, "branches">;

function covered(key: CoverageArrayKey): (string | number)[] {
  return ALL_SCREEN_SCENARIOS.flatMap((item) => {
    const values = item.coverage[key];
    return values === undefined
      ? []
      : Array.from(values as readonly (string | number)[]);
  });
}

function uniqueSorted(values: readonly (string | number)[]): (string | number)[] {
  return [...new Set(values)].sort((a, b) => String(a).localeCompare(String(b)));
}

function expectSameMembers(
  actual: readonly (string | number)[],
  expected: readonly (string | number)[],
): void {
  expect(uniqueSorted(actual)).toEqual(uniqueSorted(expected));
}

function declarationBlock(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const matches = [
    ...css.matchAll(new RegExp(`${escaped}\\s*\\{([\\s\\S]*?)\\}`, "gu")),
  ];
  if (matches.length === 0) throw new Error(`CSSブロックがありません: ${selector}`);
  // 同じselectorを後ろで上書きするCSSもある。両方を結合し、最終的な契約を
  // 構成する宣言を取りこぼさない。
  return matches.map((match) => match[1]).join("\n");
}

function rootViewportBlock(): string {
  const match = /html,\s*body,\s*#root\s*\{([\s\S]*?)\}/u.exec(css);
  if (match === null) throw new Error("html/body/#rootのCSSブロックがありません");
  return match[1];
}

function expectTokens(block: string, tokens: readonly string[]): void {
  for (const token of tokens) expect(block).toContain(token);
}

interface AxisContractCheck {
  horizontal: () => void;
  vertical: () => void;
}

const layoutChecks: Record<ScreenLayoutContract, AxisContractCheck> = {
  workspace: {
    horizontal: () => {
      expectTokens(rootViewportBlock(), ["overflow: hidden"]);
      expectTokens(declarationBlock(".pane"), ["min-width: 0", "overflow: hidden"]);
      expectTokens(declarationBlock(".tool-rail"), ["overflow-x: hidden"]);
      expectTokens(declarationBlock(".timeline"), ["min-width: 0", "overflow: hidden"]);
      expectTokens(declarationBlock(".timeline-controls"), ["min-width: 0", "overflow-x: auto"]);
      expectTokens(declarationBlock(".timeline-steps"), ["overflow-x: auto", "overflow-y: hidden"]);
      expectTokens(declarationBlock(".context-selection"), ["min-width: 0"]);
      expectTokens(declarationBlock(".context-messages"), ["min-width: 0"]);
    },
    vertical: () => {
      expectTokens(rootViewportBlock(), ["height: 100%", "overflow: hidden"]);
      expectTokens(declarationBlock(".app"), ["minmax(0, var(--main-row-share", "minmax(0, var(--context-panel-share", "height: 100%"]);
      expectTokens(declarationBlock(".main-row"), ["min-height: 0"]);
      expectTokens(declarationBlock(".pane"), ["min-height: 0", "overflow: hidden"]);
      expectTokens(declarationBlock(".tool-rail"), ["min-height: 0", "overflow-y: auto"]);
      expectTokens(declarationBlock(".context-panel"), ["min-height: 0", "overflow: hidden"]);
      expectTokens(declarationBlock(".context-selection"), ["min-height: 0", "overflow-y: auto"]);
      expectTokens(declarationBlock(".context-messages"), ["min-height: 0", "overflow-y: auto"]);
    },
  },
  "viewer-overlay": {
    horizontal: () => {
      expectTokens(declarationBlock(".viewer-overlay-region"), ["width: min(430px, calc(100% - 164px))", "min-width: 0", "overflow: hidden"]);
      expectTokens(declarationBlock(".viewer-overlay-stack"), ["min-width: 0", "overflow-x: hidden"]);
    },
    vertical: () => {
      expectTokens(declarationBlock(".viewer-overlay-region"), ["top: var(--sp-5)", "bottom: var(--sp-5)", "min-height: 0", "overflow: hidden"]);
      expectTokens(declarationBlock(".viewer-overlay-stack"), ["min-height: 0", "overflow-y: auto"]);
    },
  },
  tooltip: {
    horizontal: () => {
      expect(tooltipSource).toContain('maxWidth: "min(320px, calc(100vw - 16px))"');
      expect(tooltipSource).toContain('overflowWrap: "anywhere"');
      expect(tooltipSource).toContain("left: clamp(centeredLeft, VIEWPORT_PADDING, maximumLeft)");
    },
    vertical: () => {
      expect(tooltipSource).toContain("const MAX_GENERATED_TEXT_LENGTH = 72");
      expect(tooltipSource).toContain("top: clamp(preferredTop, VIEWPORT_PADDING, maximumTop)");
    },
  },
  dialog: {
    horizontal: () => {
      expectTokens(declarationBlock(".dialog-backdrop"), ["position: fixed", "inset: 0", "padding: var(--sp-6)"]);
      expectTokens(declarationBlock(".dialog"), ["max-width: 460px"]);
      expect(460).toBeLessThanOrEqual(MINIMUM_APP_VIEWPORT.width - 32);
    },
    vertical: () => {
      expectTokens(declarationBlock(".dialog"), ["max-height: calc(100vh - 32px)", "overflow-y: auto"]);
    },
  },
  "wide-dialog": {
    horizontal: () => {
      expectTokens(declarationBlock(".dialog-wide"), ["max-width: 720px"]);
      expect(720).toBeLessThanOrEqual(MINIMUM_APP_VIEWPORT.width - 32);
    },
    vertical: () => {
      expectTokens(declarationBlock(".dialog-wide"), ["max-height: 88vh", "overflow-y: auto"]);
      expectTokens(
        declarationBlock('.dialog-wide[data-proposal-step="candidates"]'),
        ["max-height: calc(100vh - 36px)"],
      );
    },
  },
  "paper-position": {
    horizontal: () => {
      expectTokens(declarationBlock('.dialog-wide[data-proposal-step="paper-position"]'), ["padding: 16px", "overflow: hidden"]);
      expectTokens(declarationBlock(".paper-position-step"), ["grid-template-columns: 560px minmax(0, 1fr)", "min-width: 0", "overflow: hidden"]);
      expectTokens(declarationBlock(".paper-position-stage"), ["width: 560px", "overflow: hidden"]);
    },
    vertical: () => {
      expectTokens(declarationBlock('.dialog-wide[data-proposal-step="paper-position"]'), ["height: min(668px, calc(100vh - 36px))", "max-height: min(668px, calc(100vh - 36px))", "overflow: hidden"]);
      expectTokens(declarationBlock(".paper-position-step"), ["min-height: 0", "overflow: hidden"]);
      expectTokens(declarationBlock(".paper-position-stage"), ["height: 560px", "min-height: 560px", "max-height: 560px", "overflow: hidden"]);
      expectTokens(declarationBlock(".paper-position-sidebar > .proposal-position-notices"), ["max-height: 112px", "overflow-y: auto"]);
    },
  },
  help: {
    horizontal: () => {
      expectTokens(declarationBlock(".help-dialog"), ["width: min(1180px, 84vw)", "grid-template-columns: minmax(250px, 0.31fr) minmax(0, 1fr)", "overflow: hidden"]);
      expectTokens(declarationBlock(".help-content"), ["min-width: 0"]);
      expectTokens(declarationBlock(".help-table-wrap"), ["overflow-x: auto"]);
    },
    vertical: () => {
      expectTokens(declarationBlock(".help-dialog"), ["height: 84vh", "grid-template-rows: auto minmax(0, 1fr)", "overflow: hidden"]);
      expectTokens(declarationBlock(".help-sidebar"), ["min-height: 0", "grid-template-rows: auto auto minmax(0, 1fr) auto"]);
      expectTokens(declarationBlock(".help-toc"), ["min-height: 0", "overflow-y: auto"]);
      expectTokens(declarationBlock(".help-content"), ["overflow-y: auto"]);
    },
  },
  guide: {
    horizontal: () => {
      expectTokens(declarationBlock(".first-run-guide"), ["width: min(340px, calc(100vw - 32px))"]);
    },
    vertical: () => {
      expectTokens(declarationBlock(".first-run-guide"), ["max-height: calc(100vh - 84px)", "overflow-y: auto"]);
    },
  },
  "color-picker": {
    horizontal: () => {
      expectTokens(declarationBlock(".color-picker-popover"), ["width: min(320px, calc(100vw - 16px))", "overflow-x: hidden"]);
    },
    vertical: () => {
      expectTokens(declarationBlock(".color-picker-popover"), ["max-height: calc(100vh - 16px)", "overflow-y: auto"]);
    },
  },
};

describe("1000×700で点検する全100画面の正本", () => {
  it("P42・A11・L4・N8・O35が連番で、重複なく合計100件ある", () => {
    const ids = ALL_SCREEN_SCENARIOS.map((item) => item.id);
    expect(ids).toHaveLength(100);
    expect(new Set(ids).size).toBe(100);
    expect(ids.every((id) => /^[PALNO]\d{2}$/u.test(id))).toBe(true);

    const groups = { P: 42, A: 11, L: 4, N: 8, O: 35 } as const;
    for (const [prefix, count] of Object.entries(groups)) {
      const numbers = ids
        .filter((id) => id.startsWith(prefix))
        .map((id) => Number(id.slice(1)))
        .sort((a, b) => a - b);
      expect(numbers, prefix).toEqual(
        Array.from({ length: count }, (_, index) => index + 1),
      );
    }

    for (const item of ALL_SCREEN_SCENARIOS) {
      expect(item.label.trim(), `${item.id}: label`).not.toBe("");
      expect(item.notes.trim(), `${item.id}: notes`).not.toBe("");
      expect(item.coverage.branches.length, `${item.id}: branches`).toBeGreaterThan(0);
    }
  });

  it("型unionの正本tupleと実際の配列を、シナリオが漏れなく覆う", () => {
    expect([
      TOOL_IDS_ARE_EXHAUSTIVE,
      MEASURE_MODES_ARE_EXHAUSTIVE,
      CONSTRUCT_KINDS_ARE_EXHAUSTIVE,
      TECHNIQUE_KINDS_ARE_EXHAUSTIVE,
      ALIGN_MODES_ARE_EXHAUSTIVE,
      HELP_CHAPTER_IDS_ARE_EXHAUSTIVE,
      EXPORT_KINDS_ARE_EXHAUSTIVE,
      PROPOSAL_STEPS_ARE_EXHAUSTIVE,
      GUIDE_STEPS_ARE_EXHAUSTIVE,
    ]).toEqual(Array.from({ length: 9 }, () => true));

    expect(AUDITED_TOOL_IDS).toHaveLength(10);
    expect(AUDITED_MEASURE_MODES).toHaveLength(3);
    expect(AUDITED_CONSTRUCT_KINDS).toHaveLength(4);
    expect(AUDITED_TECHNIQUE_KINDS).toHaveLength(9);
    expect(AUDITED_ALIGN_MODES).toHaveLength(8);
    expect(AUDITED_HELP_CHAPTER_IDS).toHaveLength(13);
    expect(AUDITED_EXPORT_KINDS).toHaveLength(4);

    expectSameMembers(covered("toolIds"), AUDITED_TOOL_IDS);
    expectSameMembers(covered("measureModes"), AUDITED_MEASURE_MODES);
    expectSameMembers(covered("constructKinds"), AUDITED_CONSTRUCT_KINDS);
    expectSameMembers(covered("techniqueKinds"), AUDITED_TECHNIQUE_KINDS);
    expectSameMembers(covered("alignModes"), AUDITED_ALIGN_MODES);
    expectSameMembers(covered("helpChapterIds"), AUDITED_HELP_CHAPTER_IDS);
    expectSameMembers(covered("exportKinds"), AUDITED_EXPORT_KINDS);
    expectSameMembers(covered("proposalSteps"), AUDITED_PROPOSAL_STEPS);
    expectSameMembers(covered("guideSteps"), AUDITED_GUIDE_STEPS);

    expect(HELP_CHAPTERS.map((chapter) => chapter.id)).toEqual(
      AUDITED_HELP_CHAPTER_IDS,
    );
    expect(EXPORT_CHOICES.map((choice) => choice.kind)).toEqual(
      AUDITED_EXPORT_KINDS,
    );
    expect(SUPPORTED_TECHNIQUES.map((technique) => technique.kind)).toEqual(
      AUDITED_TECHNIQUE_KINDS,
    );
  });

  it("製品TSXにある17種類の浮動UIと、点検シナリオの割当が完全一致する", () => {
    const actual = productTsxSources.flatMap((source) =>
      [...source.matchAll(/data-floating-ui="([^"]+)"/gu)].map(
        (match) => match[1],
      ),
    );
    expect(uniqueSorted(actual)).toEqual(uniqueSorted(AUDITED_FLOATING_UI_IDS));
    expect(uniqueSorted(actual)).toHaveLength(17);
    expectSameMembers(covered("floatingUiIds"), AUDITED_FLOATING_UI_IDS);
  });

  it("全シナリオが1000×700の横・縦それぞれのCSS契約へ結び付く", () => {
    expect(MINIMUM_APP_VIEWPORT).toEqual({ width: 1000, height: 700 });
    const usedContracts = new Set(
      ALL_SCREEN_SCENARIOS.map((item) => item.layoutContract),
    );
    expectSameMembers([...usedContracts], Object.keys(layoutChecks));

    for (const contract of usedContracts) {
      layoutChecks[contract].horizontal();
      layoutChecks[contract].vertical();
    }
  });
});
