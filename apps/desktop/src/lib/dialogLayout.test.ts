import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const css = readFileSync(new URL("../App.css", import.meta.url), "utf8");
const appStoreSource = readFileSync(
  new URL("../store/appStore.ts", import.meta.url),
  "utf8",
);
const proposalSource = readFileSync(
  new URL("../components/dialogs/ProposalWizard.tsx", import.meta.url),
  "utf8",
);
const firstRunGuideSource = readFileSync(
  new URL("../components/FirstRunGuide.tsx", import.meta.url),
  "utf8",
);
const colorPickerSource = readFileSync(
  new URL("../components/ColorPickerPopover.tsx", import.meta.url),
  "utf8",
);
const proposalStepType = /export type ProposalStep\s*=([\s\S]*?);/u.exec(
  appStoreSource,
);
if (proposalStepType === null) throw new Error("ProposalStepの定義がありません");
const proposalSteps = [
  ...proposalStepType[1].matchAll(/"([^"]+)"/gu),
].map((match) => match[1]);
const guideStepType = /export type GuideStep\s*=([\s\S]*?);/u.exec(
  appStoreSource,
);
if (guideStepType === null) throw new Error("GuideStep type is missing");
const guideSteps = [...guideStepType[1].matchAll(/\d+/gu)].map((match) =>
  Number(match[0]),
);

const modalScreens = [
  {
    name: "復旧",
    states: 1,
    source: readFileSync(
      new URL("../components/RecoveryDialog.tsx", import.meta.url),
      "utf8",
    ),
  },
  {
    name: "新規作成",
    states: 1,
    source: readFileSync(
      new URL("../components/dialogs/NewDocumentDialog.tsx", import.meta.url),
      "utf8",
    ),
  },
  {
    name: "提案4画面",
    states: proposalSteps.length,
    source: proposalSource,
  },
  {
    name: "書き出し",
    states: 1,
    source: readFileSync(
      new URL("../components/dialogs/ExportDialog.tsx", import.meta.url),
      "utf8",
    ),
  },
  {
    name: "ヘルプ",
    states: 1,
    source: readFileSync(
      new URL("../components/dialogs/HelpCenter.tsx", import.meta.url),
      "utf8",
    ),
  },
] as const;

function declarationBlock(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`${escaped}\\s*\\{([\\s\\S]*?)\\}`).exec(css);
  if (match === null) throw new Error(`CSSブロックがありません: ${selector}`);
  return match[1];
}

describe("1000×700の全画面と手前・後ろの区別", () => {
  it("全5モーダル・8画面状態が同じ強い背景と不透明な手前を通る", () => {
    expect(proposalSteps).toEqual([
      "skeleton",
      "candidates",
      "paper-position",
      "confirm",
    ]);
    for (const step of proposalSteps) {
      expect(proposalSource).toContain(`step === "${step}"`);
    }
    expect(modalScreens).toHaveLength(5);
    expect(modalScreens.reduce((sum, screen) => sum + screen.states, 0)).toBe(8);
    for (const screen of modalScreens) {
      expect(screen.source, `${screen.name}: 共通の後ろ幕`).toContain(
        'className="dialog-backdrop',
      );
      expect(screen.source, `${screen.name}: モーダルの明示`).toContain(
        'aria-modal="true"',
      );
      expect(screen.source, `${screen.name}: 共通の画面内監査`).toContain(
        "data-floating-ui=",
      );
      expect(screen.source, `${screen.name}: 不透明な共通枠`).toMatch(
        /className="dialog(?:\s|")/u,
      );
    }

    const backdrop = declarationBlock(".dialog-backdrop");
    const dialog = declarationBlock(".dialog");
    expect(backdrop).toContain("background: var(--color-overlay)");
    expect(backdrop).not.toContain("background: var(--color-scrim)");
    expect(dialog).toContain("background-color: var(--color-surface)");

    const overlayAlphas = [
      ...css.matchAll(
        /--color-overlay:\s*rgba\([^,]+,[^,]+,[^,]+,\s*([\d.]+)\s*\)/gu,
      ),
    ].map((match) => Number(match[1]));
    expect(overlayAlphas).toHaveLength(5);
    // 修正前の実測0.38〜0.46では後ろが54〜62%残った。0.8以上なら
    // 後ろは高々20%となり、手前の不透明な面と明確に分かれる。
    expect(overlayAlphas.every((alpha) => alpha >= 0.8)).toBe(true);
  });

  it("紙以外の7モーダル状態と非モーダル2種は外枠か内部を縦に送れる", () => {
    const dialog = declarationBlock(".dialog");
    const wide = declarationBlock(".dialog-wide");
    const candidates = declarationBlock(
      '.dialog-wide[data-proposal-step="candidates"]',
    );
    const help = declarationBlock(".help-dialog");
    const guide = declarationBlock(".first-run-guide");
    const colorPicker = declarationBlock(".color-picker-popover");

    expect(dialog).toContain("max-height: calc(100vh - 32px)");
    expect(dialog).toContain("overflow-y: auto");
    expect(wide).toContain("max-height: 88vh");
    expect(wide).toContain("overflow-y: auto");
    expect(candidates).toContain("max-height: calc(100vh - 36px)");
    expect(help).toContain("height: 84vh");
    expect(help).toContain("overflow: hidden");
    expect(guide).toContain("max-height: calc(100vh - 84px)");
    expect(guide).toContain("overflow-y: auto");
    expect(colorPicker).toContain("max-height: calc(100vh - 16px)");
    expect(colorPicker).toContain("overflow-y: auto");

    // 開ける重ね表示は、モーダル8状態、初回ガイド5状態、
    // 色選択1状態の計14状態。画面が増えたときに監査から漏れないよう数も固定する。
    expect(guideSteps).toEqual([0, 1, 2, 3, 4]);
    expect(firstRunGuideSource).toContain('className="first-run-guide');
    expect(firstRunGuideSource).toContain('data-floating-ui="first-run-guide"');
    expect(colorPickerSource).toContain('className="color-picker-popover"');
    expect(colorPickerSource).toContain('data-floating-ui="color-picker"');
    expect(
      modalScreens.reduce((sum, screen) => sum + screen.states, 0) +
        guideSteps.length +
        1,
    ).toBe(14);
  });
});
