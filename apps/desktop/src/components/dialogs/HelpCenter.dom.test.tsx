// @vitest-environment jsdom

import { useEffect, useRef } from "react";
import { afterEach, describe, expect, it } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { HelpCenter } from "./HelpCenter";
import { ModalDialog, focusableElements, type FocusTarget } from "./ModalDialog";
import { useAppStore } from "../../store/appStore";

const originalStoreState = useAppStore.getState();

function showHelp(): void {
  act(() => {
    useAppStore.setState({
      helpOpen: true,
      helpChapterId: "overview",
      helpQuery: "",
      guideOpen: false,
    });
  });
}

function HelpKeyboardHarness() {
  return (
    <>
      <HelpShortcutHarness />
      <button type="button">ヘルプの起点</button>
      <HelpCenter />
    </>
  );
}

/** Appが常駐させるF1入口を、Help本体のfocus回帰検査でも再現する。 */
function HelpShortcutHarness() {
  const openHelp = useAppStore((s) => s.openHelp);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "F1") return;
      event.preventDefault();
      openHelp();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [openHelp]);

  return null;
}

function LowerModalWithHelp() {
  const initialFocusRef = useRef<HTMLButtonElement>(null);
  return (
    <>
      <HelpShortcutHarness />
      <ModalDialog
        labelledBy="lower-help-test-title"
        initialFocusRef={initialFocusRef}
        escapeAction={{ kind: "stay" }}
        data-floating-ui="lower-help-test"
      >
        <h2 id="lower-help-test-title">下の画面</h2>
        <button ref={initialFocusRef} type="button">
          下の画面の操作
        </button>
      </ModalDialog>
      <HelpCenter />
    </>
  );
}

function pressF1(target: Element): KeyboardEvent {
  const event = new KeyboardEvent("keydown", {
    key: "F1",
    bubbles: true,
    cancelable: true,
  });
  fireEvent(target, event);
  return event;
}

function pressEscape(target: Element): KeyboardEvent {
  const event = new KeyboardEvent("keydown", {
    key: "Escape",
    bubbles: true,
    cancelable: true,
  });
  fireEvent(target, event);
  return event;
}

/** jsdomに無い通常のTab移動だけを補い、端の循環は共通土台へ任せる。 */
function pressHelpTab(dialog: HTMLElement, shiftKey = false): KeyboardEvent {
  const ordered = focusableElements(dialog);
  const active = document.activeElement as FocusTarget | null;
  const event = new KeyboardEvent("keydown", {
    key: "Tab",
    shiftKey,
    bubbles: true,
    cancelable: true,
  });
  fireEvent(active ?? dialog, event);
  if (!event.defaultPrevented) {
    const current = active === null ? -1 : ordered.indexOf(active);
    const next = shiftKey
      ? current <= 0
        ? ordered.length - 1
        : current - 1
      : current < 0 || current === ordered.length - 1
        ? 0
        : current + 1;
    ordered[next]?.focus();
  }
  fireEvent.keyUp(document.activeElement ?? dialog, { key: "Tab", shiftKey });
  return event;
}

function tabToHelpTarget(dialog: HTMLElement, target: FocusTarget): void {
  const limit = focusableElements(dialog).length + 1;
  for (let index = 0; index < limit && document.activeElement !== target; index += 1) {
    pressHelpTab(dialog);
  }
  expect(document.activeElement).toBe(target);
}

/**
 * jsdomはEnterからbuttonのclickを作らない。keydownが取り消されなかった場合だけ、
 * ブラウザーが続けて行うclickを補い、pointer/mousedownは発生させない。
 */
function activateHelpButton(button: HTMLButtonElement): void {
  const event = new KeyboardEvent("keydown", {
    key: "Enter",
    bubbles: true,
    cancelable: true,
  });
  fireEvent(button, event);
  expect(event.defaultPrevented).toBe(false);
  if (!event.defaultPrevented) {
    act(() => button.click());
  }
  fireEvent.keyUp(button, { key: "Enter" });
}

/** 検査環境が作らない文字入力結果だけを、keydownの直後に補う。 */
function typeSearchWithKeyboard(search: HTMLInputElement, value: string): void {
  let entered = "";
  for (const character of value) {
    fireEvent.keyDown(search, { key: character });
    entered += character;
    fireEvent.input(search, { target: { value: entered } });
    fireEvent.keyUp(search, { key: character });
  }
}

afterEach(() => {
  cleanup();
  useAppStore.setState(originalStoreState, true);
});

describe("ヘルプセンター", () => {
  it.each([
    [1, 50],
    [51, 100],
  ] as const)(
    "F1から検索欄へ移り、Escape後に起点と背景を戻す(%i〜%i回)",
    async (start, end) => {
      render(<HelpKeyboardHarness />);
      const trigger = screen.getByRole("button", { name: "ヘルプの起点" });

      for (let index = start; index <= end; index += 1) {
        trigger.focus();
        const f1 = pressF1(trigger);
        expect(f1.defaultPrevented, `F1 ${index}回目`).toBe(true);
        const dialog = screen.getByRole("dialog", { name: "ヘルプセンター" });
        const search = screen.getByRole("searchbox", { name: "章題・本文を検索" });
        expect(document.activeElement, `最初 ${index}回目`).toBe(search);
        expect(trigger.closest("[inert]"), `背景停止 ${index}回目`).not.toBeNull();

        const escape = pressEscape(search);
        expect(escape.defaultPrevented, `Escape ${index}回目`).toBe(true);
        await act(async () => {
          await Promise.resolve();
        });

        expect(screen.queryByRole("dialog"), `終了 ${index}回目`).toBeNull();
        expect(document.activeElement, `起点復帰 ${index}回目`).toBe(trigger);
        expect(trigger.closest("[inert]"), `背景復帰 ${index}回目`).toBeNull();
        expect(dialog.isConnected, `外した画面 ${index}回目`).toBe(false);
      }
    },
  );

  it.each([
    ["Tab", 1, 50, false],
    ["Tab", 51, 100, false],
    ["Shift+Tab", 1, 50, true],
    ["Shift+Tab", 51, 100, true],
  ] as const)(
    "%sを端から循環する(%i〜%i回)",
    (_name, start, end, shiftKey) => {
      showHelp();
      render(<HelpCenter />);
      const dialog = screen.getByRole("dialog", { name: "ヘルプセンター" });
      const ordered = focusableElements(dialog);
      expect(ordered.length).toBeGreaterThan(1);
      const first = ordered[0];
      const last = ordered[ordered.length - 1];

      for (let index = start; index <= end; index += 1) {
        const from = shiftKey ? first : last;
        const expected = shiftKey ? last : first;
        from.focus();
        const tab = pressHelpTab(dialog, shiftKey);
        expect(tab.defaultPrevented, `${_name} ${index}回目`).toBe(true);
        expect(document.activeElement, `${_name}後 ${index}回目`).toBe(expected);
      }
    },
  );

  it("pointerとmouseを使わずF1・検索・目次・章移動・Escapeを完了する", async () => {
    let pointerDownCount = 0;
    let mouseDownCount = 0;
    const countPointerDown = () => {
      pointerDownCount += 1;
    };
    const countMouseDown = () => {
      mouseDownCount += 1;
    };
    document.addEventListener("pointerdown", countPointerDown, true);
    document.addEventListener("mousedown", countMouseDown, true);

    try {
      render(<HelpKeyboardHarness />);
      const trigger = screen.getByRole("button", { name: "ヘルプの起点" });
      trigger.focus();
      const f1 = pressF1(trigger);
      expect(f1.defaultPrevented).toBe(true);

      const dialog = screen.getByRole("dialog", { name: "ヘルプセンター" });
      const search = screen.getByRole("searchbox", {
        name: "章題・本文を検索",
      }) as HTMLInputElement;
      expect(document.activeElement).toBe(search);
      typeSearchWithKeyboard(search, "手順の記録と再生");
      expect(search.value).toBe("手順の記録と再生");

      const timeline = screen.getByRole("button", {
        name: /手順の記録と再生/u,
      }) as HTMLButtonElement;
      tabToHelpTarget(dialog, timeline);
      activateHelpButton(timeline);
      expect(useAppStore.getState().helpChapterId).toBe("timeline");
      expect(screen.getByRole("heading", { name: "手順の記録と再生" })).toBeTruthy();

      const escape = pressEscape(timeline);
      expect(escape.defaultPrevented).toBe(true);
      await act(async () => {
        await Promise.resolve();
      });

      expect(screen.queryByRole("dialog")).toBeNull();
      expect(document.activeElement).toBe(trigger);
      expect(pointerDownCount).toBe(0);
      expect(mouseDownCount).toBe(0);
    } finally {
      document.removeEventListener("pointerdown", countPointerDown, true);
      document.removeEventListener("mousedown", countMouseDown, true);
    }
  });

  it("画面内のmousedownでは維持し、外幕自身のmousedownでは閉じる", () => {
    showHelp();
    render(<HelpCenter />);
    const dialog = screen.getByRole("dialog", { name: "ヘルプセンター" });
    const layer = dialog.closest<HTMLElement>('[data-modal-layer="true"]');
    expect(layer).not.toBeNull();

    fireEvent.mouseDown(dialog);
    expect(screen.getByRole("dialog", { name: "ヘルプセンター" })).toBe(dialog);
    fireEvent.mouseDown(layer!);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(useAppStore.getState().helpOpen).toBe(false);
  });

  it("下の画面の要素からF1で重ね、Escape後にその要素へ戻る", async () => {
    act(() => useAppStore.setState({ helpOpen: false }));
    render(<LowerModalWithHelp />);
    const lowerButton = screen.getByRole("button", { name: "下の画面の操作" });
    const lowerDialog = screen.getByRole("dialog", { name: "下の画面" });
    expect(document.activeElement).toBe(lowerButton);

    const f1 = pressF1(lowerButton);
    expect(f1.defaultPrevented).toBe(true);
    expect(screen.getAllByRole("dialog")).toHaveLength(2);
    const search = screen.getByRole("searchbox", { name: "章題・本文を検索" });
    expect(document.activeElement).toBe(search);
    expect(lowerButton.closest("[inert]")).not.toBeNull();

    const escape = pressEscape(search);
    expect(escape.defaultPrevented).toBe(true);
    await act(async () => {
      await Promise.resolve();
    });

    expect(screen.getAllByRole("dialog")).toEqual([lowerDialog]);
    expect(document.activeElement).toBe(lowerButton);
    expect(lowerButton.closest("[inert]")).toBeNull();
  });

  it("閉じた状態からF1で開き、Escで閉じる", () => {
    act(() => useAppStore.setState({ helpOpen: false }));
    render(<HelpKeyboardHarness />);
    expect(screen.queryByRole("dialog")).toBeNull();

    const f1 = new KeyboardEvent("keydown", { key: "F1", bubbles: true, cancelable: true });
    fireEvent(window, f1);
    expect(f1.defaultPrevented).toBe(true);
    expect(screen.getByRole("dialog", { name: "ヘルプセンター" })).toBeTruthy();
    expect(useAppStore.getState().helpOpen).toBe(true);
    const activeSearch = screen.getByRole("searchbox", { name: "章題・本文を検索" });

    const escape = new KeyboardEvent("keydown", {
      key: "Escape",
      bubbles: true,
      cancelable: true,
    });
    fireEvent(activeSearch, escape);
    expect(escape.defaultPrevented).toBe(true);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(useAppStore.getState().helpOpen).toBe(false);
  });

  it("閉じるボタンでダイアログを閉じる", () => {
    showHelp();
    render(<HelpCenter />);

    fireEvent.click(screen.getByRole("button", { name: "ヘルプセンターを閉じる" }));

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(useAppStore.getState().helpOpen).toBe(false);
  });

  it("目次から章を選ぶと本文と選択状態が変わる", () => {
    showHelp();
    render(<HelpCenter />);

    const timeline = screen.getByRole("button", { name: /手順の記録と再生/ });
    fireEvent.click(timeline);

    expect(useAppStore.getState().helpChapterId).toBe("timeline");
    expect(timeline.getAttribute("aria-current")).toBe("page");
    expect(screen.getByRole("heading", { name: "手順の記録と再生" })).toBeTruthy();
    expect(screen.getByText("途中へ新しい折りを挿入する")).toBeTruthy();
  });

  it("保存と書き出し章はほかのソフト用の説明と対応外8項目を安全に表示する", () => {
    showHelp();
    render(<HelpCenter />);

    fireEvent.click(screen.getByRole("button", { name: /保存と書き出し/ }));
    const article = screen.getByRole("article", { name: "保存と書き出し" });
    expect(
      within(article).getByRole("heading", { name: "5つの書き出し形式" }),
    ).toBeTruthy();
    expect(
      within(article).getByText("ほかの折り紙ソフトのファイル", {
        exact: true,
      }),
    ).toBeTruthy();
    const scopeHeading = within(article).getByRole("heading", {
      name: "ほかの折り紙ソフトのファイルでそのまま扱えない内容（8項目）",
    });
    const scope = scopeHeading.closest("section");
    expect(scope).not.toBeNull();
    expect(
      within(scope as HTMLElement)
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual([
      "立体になったときの点の位置",
      "途中から複数の流れに分かれる折る手順",
      "動画として記録された動き",
      "名前の付いた折り方が何を意味するか",
      "作品につけたメモや説明",
      "仕上げにつけた丸み",
      "元のファイルで「平らな折り目」と「種類が指定されていない折り目」を区別すること",
      // 2026-09-05追加。平らな形で終わる手順は書き出せるようになったので、残る範囲だけを示す。
      "まだ平らになっていない途中の形で終わる手順のうち、紙を曲げないと作れないもの",
    ]);

    const displayed = article.innerHTML;
    for (const forbidden of [
      "FOLD 1.1",
      "FOLD 1.2",
      "parser",
      "schema",
      "validator",
      "パーサ",
      "スキーマ",
      "バリデータ",
      "faceOrders",
      "frame",
      "Aux",
      "JSON path",
      "$.",
    ]) {
      expect(displayed).not.toContain(forbidden);
    }
  });

  it("章題と本文の単純な文字列一致で目次と章表示を絞り込む", () => {
    showHelp();
    render(<HelpCenter />);
    const search = screen.getByRole("searchbox", { name: "章題・本文を検索" });

    fireEvent.change(search, { target: { value: "形から展開図" } });
    expect(screen.getByText("1章が見つかりました")).toBeTruthy();
    expect(screen.getByRole("button", { name: /形から展開図を提案/ })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "形から展開図を提案" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /画面の見かた/ })).toBeNull();

    fireEvent.change(search, { target: { value: "ベジェ曲線" } });
    expect(screen.getByText("1章が見つかりました")).toBeTruthy();
    expect(screen.getByRole("button", { name: /展開図に線を引く/ })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "展開図に線を引く" })).toBeTruthy();

    const clear = screen.getByRole("button", { name: "検索語を消す" });
    expect(clear.getAttribute("data-tooltip")).toBe("入力した検索語を消します");
    expect(clear.hasAttribute("title")).toBe(false);
    clear.focus();
    expect(document.activeElement).toBe(clear);
    fireEvent.click(clear);
    expect((search as HTMLInputElement).value).toBe("");
    expect(document.activeElement).toBe(search);
    expect(screen.queryByRole("button", { name: "検索語を消す" })).toBeNull();
    expect(screen.getByText("全13章")).toBeTruthy();
    expect(screen.getByRole("button", { name: /画面の見かた/ })).toBeTruthy();
  });

  it("ヘルプ内から基本操作ガイドを最初から開ける", () => {
    showHelp();
    act(() => useAppStore.setState({ guideStep: 3 }));
    render(<HelpCenter />);

    fireEvent.click(screen.getByRole("button", { name: "基本操作ガイドをもう一度" }));

    const state = useAppStore.getState();
    expect(state.helpOpen).toBe(false);
    expect(state.guideOpen).toBe(true);
    expect(state.guideStep).toBe(0);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
