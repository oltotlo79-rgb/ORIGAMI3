// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import App from "./App";
import { AppToolbar, ExportButton } from "./components/AppToolbar";
import {
  relaxationStatus,
  ViewerStatusOverlays,
} from "./components/ViewerStatusOverlays";
import { EXPORT_CHOICES } from "./components/dialogs/exportChoices";
import { statusBadgeText, warningCount } from "./lib/flatFoldNotice";
import { useAppStore } from "./store/appStore";

const exportDialogPayload = vi.hoisted(() => ({
  load: vi.fn(() => new Promise(() => {})),
}));
const helpCenterPayload = vi.hoisted(() => ({
  load: vi.fn(() => new Promise(() => {})),
}));

vi.mock("./components/ToolRail", () => ({ ToolRail: () => null }));
vi.mock("./components/ContextPanel", () => ({ ContextPanel: () => null }));
vi.mock("./components/CpEditor/CpEditor", () => ({ CpEditor: () => null }));
vi.mock("./components/Viewer3D/Viewer3D", () => ({ Viewer3D: () => null }));
vi.mock("./components/Timeline", () => ({ Timeline: () => null }));
vi.mock("./components/RecoveryDialog", () => ({ RecoveryDialog: () => null }));
vi.mock("./components/PaneSplitter", () => ({ PaneSplitter: () => null }));
vi.mock("./components/ContextPanelSplitter", () => ({ ContextPanelSplitter: () => null }));
vi.mock("./components/dialogs/NewDocumentDialog", () => ({ NewDocumentDialog: () => null }));
vi.mock("./components/dialogs/ProposalWizard", () => ({ ProposalWizard: () => null }));
vi.mock("./components/HistoryShortcuts", () => ({ HistoryShortcuts: () => null }));
vi.mock("./components/ToolIcons", () => ({ ToolbarIcon: () => null }));
vi.mock("./components/ToolbarBrandMark", () => ({ ToolbarBrandMark: () => null }));
vi.mock("./components/FirstRunGuide", () => ({ FirstRunGuide: () => null }));
vi.mock("./components/dialogs/HelpCenter", () => helpCenterPayload.load());
vi.mock("./components/ThemeRoot", () => ({
  ThemeRoot: ({ children }: { children: unknown }) => children,
}));
vi.mock("./components/Tooltip", () => ({ TooltipHost: () => null }));
vi.mock("./captureApi", () => ({ installCaptureApi: vi.fn() }));
vi.mock("./components/dialogs/ExportDialog", () => exportDialogPayload.load());

const realNewDocument = useAppStore.getState().newDocument;
const realCheckRecovery = useAppStore.getState().checkRecovery;
const realOpenNewDialog = useAppStore.getState().openNewDialog;
const realOpenProposal = useAppStore.getState().openProposal;
const realOpenExport = useAppStore.getState().openExport;
const realSetSelection = useAppStore.getState().setSelection;
const initialStatusState = {
  warnings: useAppStore.getState().warnings,
  poseWarnings: useAppStore.getState().poseWarnings,
  replayWarnings: useAppStore.getState().replayWarnings,
  flatFoldViolations: useAppStore.getState().flatFoldViolations,
  poseConverged: useAppStore.getState().poseConverged,
  relaxations: useAppStore.getState().relaxations,
  poseBestEffort: useAppStore.getState().poseBestEffort,
  errorMessage: useAppStore.getState().errorMessage,
  suspectHinges: useAppStore.getState().suspectHinges,
};

beforeEach(() => {
  exportDialogPayload.load.mockClear();
  helpCenterPayload.load.mockClear();
  useAppStore.setState({
    exportOpen: false,
    helpOpen: false,
    newDocument: vi.fn().mockResolvedValue(undefined),
    checkRecovery: vi.fn().mockResolvedValue(undefined),
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState({
    exportOpen: false,
    helpOpen: false,
    newDocument: realNewDocument,
    checkRecovery: realCheckRecovery,
    openNewDialog: realOpenNewDialog,
    openProposal: realOpenProposal,
    openExport: realOpenExport,
    setSelection: realSetSelection,
    ...initialStatusState,
  });
});

describe("上部ツールバーの操作順(D8)", () => {
  it("headerを増やさず、押しどころ・自然なTab順・読み上げ順を保つ", () => {
    const openNewDialog = vi.fn();
    const openProposal = vi.fn();
    const openExport = vi.fn();
    const onOpenHelp = vi.fn();
    useAppStore.setState({ openNewDialog, openProposal, openExport });

    const { container } = render(<AppToolbar onOpenHelp={onOpenHelp} />);
    const toolbar = container.firstElementChild as HTMLElement;
    expect(toolbar.tagName).toBe("HEADER");
    expect(toolbar.className).toBe("toolbar");
    expect(container.children).toHaveLength(1);

    const buttons = Array.from(toolbar.querySelectorAll("button"));
    expect(buttons.map((button) => button.textContent?.trim())).toEqual([
      "新規",
      "開く",
      "保存",
      "元に戻す",
      "やり直し",
      "提案",
      "書き出し",
      "ヘルプ",
    ]);
    for (const button of buttons) {
      expect(button.type).toBe("button");
      expect(button.tabIndex).toBe(0);
      button.focus();
      expect(document.activeElement).toBe(button);
    }

    fireEvent.click(buttons[0]);
    fireEvent.click(buttons[5]);
    fireEvent.click(buttons[6]);
    fireEvent.click(buttons[7]);
    expect(openNewDialog).toHaveBeenCalledTimes(1);
    expect(openProposal).toHaveBeenCalledTimes(1);
    expect(openExport).toHaveBeenCalledTimes(1);
    expect(onOpenHelp).toHaveBeenCalledTimes(1);
    expect(buttons[7].getAttribute("aria-label")).toBe(
      "ヘルプセンターを開く",
    );
  });
});

describe("上部の書き出し案内(D27)", () => {
  it("実際に選べる4形式だけを、選択肢と同じ順で案内する", () => {
    render(<ExportButton onClick={vi.fn()} />);

    const formats = EXPORT_CHOICES.map((choice) => choice.label);
    const button = screen.getByRole("button", { name: "書き出し" });
    expect(formats).toHaveLength(4);
    expect(button.getAttribute("data-tooltip")).toBe(
      `${formats.join("、")}を書き出します`,
    );
    expect(button.getAttribute("data-tooltip")).not.toContain("3D");
  });
});

describe("書き出し画面の初回準備表示", () => {
  it("closed時は読まず、open時だけpayloadを1回読み日本語の待機表示を出す", async () => {
    render(<App />);

    expect(exportDialogPayload.load).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "書き出し" }));

    await waitFor(() => expect(exportDialogPayload.load).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("status").textContent).toBe("書き出しの準備をしています…");
  });
});

describe("ヘルプ画面の遅延読み込み", () => {
  it("closed時は読み込まず、F1は100回ともヘルプの準備を始める", async () => {
    render(<App />);

    expect(helpCenterPayload.load).not.toHaveBeenCalled();

    for (let index = 1; index <= 100; index += 1) {
      const f1 = new KeyboardEvent("keydown", {
        key: "F1",
        bubbles: true,
        cancelable: true,
      });
      fireEvent(window, f1);

      expect(f1.defaultPrevented, `F1 ${index}回目`).toBe(true);
      expect(useAppStore.getState().helpOpen).toBe(true);
      if (index === 1) {
        await waitFor(() => expect(helpCenterPayload.load).toHaveBeenCalledTimes(1));
      } else {
        expect(helpCenterPayload.load).toHaveBeenCalledTimes(1);
      }
      expect(screen.getByRole("status").textContent).toBe(
        "ヘルプを開く準備をしています…",
      );

      act(() => useAppStore.setState({ helpOpen: false }));
      expect(screen.queryByRole("status")).toBeNull();
    }
  });
});

describe("3D右上の自然追従表示(SIM-018)", () => {
  it("0.1度以上では本数と最大偏差を出し、0.099度は表示しない", () => {
    expect(
      relaxationStatus(
        [{ hinge: 5, target_angle_deg: 90, actual_angle_deg: 89.901, delta_deg: -0.099 }],
        false,
      ),
    ).toBeNull();

    expect(
      relaxationStatus(
        [
          { hinge: 5, target_angle_deg: 90, actual_angle_deg: 89.9, delta_deg: -0.1 },
          { hinge: 9, target_angle_deg: 90, actual_angle_deg: 72, delta_deg: -18 },
        ],
        false,
      ),
    ).toBe("前の折り目2本が追従（最大18.0°）");

    // 10進の90.0°と89.9°を実際に引くと、二進浮動小数ではわずかに
    // 0.1°を下回る。この丸め誤差だけで通知を落とさない。
    expect(
      relaxationStatus(
        [
          {
            hinge: 11,
            target_angle_deg: 90,
            actual_angle_deg: 89.9,
            delta_deg: 89.9 - 90,
          },
        ],
        false,
      ),
    ).toBe("前の折り目1本が追従（最大0.1°）");
  });

  it("最良候補では指定を優先して追従中と知らせる", () => {
    expect(relaxationStatus([], true)).toBe("指定を優先し、いちばん近い形で追従中");
  });
});

describe("3D右上の平らに畳めない点の件数", () => {
  it("通常警告2件と4点を警告6件として数える", () => {
    expect(
      warningCount(
        ["通常警告A", "通常警告B"],
        ["通常警告A"],
        [],
        [9, 10, 11, 12],
      ),
    ).toBe(6);
  });

  it("平らに畳めない点は自然追従の表示より優先する", () => {
    expect(
      statusBadgeText({
        hasError: false,
        followStatus: "前の折り目6本が追従（最大89.4°）",
        poseConverged: true,
        warningCount: 4,
        flatFoldViolationCount: 4,
      }),
    ).toBe("警告 4");
  });
});

describe("3D右上の状態表示順(D8)", () => {
  it("wrapperを増やさず、通知から原因候補の順に読み上げて同じ折り目を選ぶ", () => {
    const setSelection = vi.fn();
    useAppStore.setState({
      warnings: [],
      poseWarnings: [],
      replayWarnings: [],
      flatFoldViolations: [],
      poseConverged: true,
      relaxations: [],
      poseBestEffort: false,
      errorMessage: "検査用エラー",
      suspectHinges: [23, 29],
      setSelection,
    });

    const { container } = render(<ViewerStatusOverlays />);
    const badge = container.querySelector<HTMLElement>(
      '[data-floating-ui="status-badge"]',
    );
    const guide = container.querySelector<HTMLButtonElement>(
      '[data-floating-ui="suspect-hinge-guide"]',
    );
    expect(badge).not.toBeNull();
    expect(guide).not.toBeNull();
    expect(container.children).toHaveLength(2);
    expect(badge?.nextElementSibling).toBe(guide);
    expect(badge?.querySelector("svg")?.getAttribute("aria-hidden")).toBe(
      "true",
    );
    expect(badge?.querySelector("svg")?.getAttribute("focusable")).toBe(
      "false",
    );
    expect(guide?.type).toBe("button");
    expect(guide?.tabIndex).toBe(0);
    guide?.focus();
    expect(document.activeElement).toBe(guide);
    expect(guide?.getAttribute("data-tooltip")).toBe(
      "赤く光る折り目の角度を見直してください。押すと原因候補を選びます",
    );

    fireEvent.click(guide as HTMLButtonElement);
    expect(setSelection).toHaveBeenCalledWith({
      edgeIds: [23],
      vertexIds: [],
    });
  });
});
