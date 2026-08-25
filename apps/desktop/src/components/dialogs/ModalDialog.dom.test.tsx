// @vitest-environment jsdom

import { StrictMode, useRef } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import {
  ModalDialog,
  focusableElements,
  type FocusTarget,
  type ModalEscapeAction,
} from "./ModalDialog";
import { useAppStore } from "../../store/appStore";

function BasicDialog({
  name,
  escapeAction,
  initialDisabled = false,
  fallbackFocusRef,
  returnFocusRef,
}: {
  name: string;
  escapeAction: ModalEscapeAction;
  initialDisabled?: boolean;
  fallbackFocusRef?: React.RefObject<FocusTarget | null>;
  returnFocusRef?: React.RefObject<FocusTarget | null>;
}) {
  const initialRef = useRef<HTMLButtonElement>(null);
  return (
    <ModalDialog
      labelledBy={`${name}-title`}
      data-testid={`${name}-dialog`}
      initialFocusRef={initialRef}
      fallbackFocusRef={fallbackFocusRef}
      returnFocusRef={returnFocusRef}
      escapeAction={escapeAction}
    >
      <h2 id={`${name}-title`}>{name}</h2>
      <button ref={initialRef} type="button" disabled={initialDisabled}>
        {name}の最初
      </button>
      <button type="button">{name}の最後</button>
    </ModalDialog>
  );
}

afterEach(async () => {
  cleanup();
  await Promise.resolve();
  for (const element of [...document.body.children]) {
    if (element.hasAttribute("data-test-outside")) element.remove();
  }
  vi.restoreAllMocks();
  useAppStore.setState({ uiTheme: "pop" });
});

describe("共通ダイアログ土台", () => {
  it("body直下へ出ても既存のapp部品と選択中テーマを引き継ぐ", () => {
    useAppStore.setState({ uiTheme: "japanese" });
    render(<BasicDialog name="テーマ" escapeAction={{ kind: "stay" }} />);

    const layer = screen.getByRole("dialog", { name: "テーマ" }).parentElement;
    expect(layer?.className).toBe("app dialog-backdrop");
    expect(layer?.getAttribute("data-theme")).toBe("japanese");
  });

  it("操作要素が0件なら画面本体を最初に選び、Tabを外へ出さない", () => {
    render(
      <ModalDialog labelledBy="empty-title" escapeAction={{ kind: "stay" }}>
        <h2 id="empty-title">知らせ</h2>
        <p>選べる項目はありません</p>
      </ModalDialog>,
    );

    const dialog = screen.getByRole("dialog", { name: "知らせ" });
    expect(document.activeElement).toBe(dialog);
    expect(focusableElements(dialog)).toHaveLength(0);

    for (const shiftKey of [false, true]) {
      const event = new KeyboardEvent("keydown", {
        key: "Tab",
        shiftKey,
        bubbles: true,
        cancelable: true,
      });
      fireEvent(dialog, event);
      expect(event.defaultPrevented).toBe(true);
      expect(document.activeElement).toBe(dialog);
    }
  });

  it("無効・非表示・閉じた詳細・tabIndex -1を候補から除く", () => {
    function DisabledCases() {
      const disabledRef = useRef<HTMLButtonElement>(null);
      return (
        <ModalDialog
          labelledBy="disabled-title"
          initialFocusRef={disabledRef}
          escapeAction={{ kind: "stay" }}
        >
          <h2 id="disabled-title">有効な項目</h2>
          <button ref={disabledRef} type="button" disabled>無効1</button>
          <button type="button" aria-disabled="true">無効2</button>
          <fieldset disabled><button type="button">無効3</button></fieldset>
          <button type="button" hidden>無効4</button>
          <button type="button" tabIndex={-1}>無効5</button>
          <div style={{ display: "none" }}><button type="button">無効6</button></div>
          <button type="button">使える項目</button>
          <fieldset disabled>
            <legend><button type="button">使える凡例</button></legend>
            <legend><button type="button">無効な凡例</button></legend>
          </fieldset>
          <details>
            <summary>閉じた項目</summary>
            <summary>無効な2番目の項目</summary>
            <button type="button">無効7</button>
          </details>
          <label><input type="radio" name="choice" /> 選択肢1</label>
          <label><input type="radio" name="choice" defaultChecked /> 選択肢2</label>
          <fieldset disabled>
            <label><input type="radio" name="shared" defaultChecked /> 無効8</label>
          </fieldset>
          <label><input type="radio" name="shared" /> 使える選択肢</label>
        </ModalDialog>
      );
    }
    render(<DisabledCases />);

    const dialog = screen.getByRole("dialog", { name: "有効な項目" });
    const enabledButton = screen.getByRole("button", { name: "使える項目" });
    const enabledLegendButton = screen.getByRole("button", { name: "使える凡例" });
    const enabledSummary = screen.getByText("閉じた項目");
    const checkedRadio = screen.getByRole("radio", { name: "選択肢2" });
    const enabledSharedRadio = screen.getByRole("radio", { name: "使える選択肢" });
    expect(focusableElements(dialog)).toEqual([
      enabledButton,
      enabledLegendButton,
      enabledSummary,
      checkedRadio,
      enabledSharedRadio,
    ]);
    expect(document.activeElement).toBe(enabledButton);
  });

  it("明示した見出しはtabIndex -1でも最初に選ぶ", () => {
    function HeadingFirst() {
      const headingRef = useRef<HTMLHeadingElement>(null);
      return (
        <ModalDialog
          labelledBy="heading-title"
          initialFocusRef={headingRef}
          escapeAction={{ kind: "stay" }}
        >
          <h2 id="heading-title" ref={headingRef} tabIndex={-1}>長いお知らせ</h2>
          <button type="button">確認する</button>
        </ModalDialog>
      );
    }
    render(<HeadingFirst />);

    expect(document.activeElement).toBe(
      screen.getByRole("heading", { name: "長いお知らせ" }),
    );
  });

  it("Tab末尾→先頭とShift+Tab先頭→末尾を各100回循環する", () => {
    render(<BasicDialog name="循環" escapeAction={{ kind: "stay" }} />);
    const dialog = screen.getByRole("dialog", { name: "循環" });
    const first = screen.getByRole("button", { name: "循環の最初" });
    const last = screen.getByRole("button", { name: "循環の最後" });

    for (let index = 0; index < 100; index += 1) {
      last.focus();
      const forward = new KeyboardEvent("keydown", {
        key: "Tab",
        bubbles: true,
        cancelable: true,
      });
      fireEvent(last, forward);
      expect(forward.defaultPrevented).toBe(true);
      expect(document.activeElement).toBe(first);

      const backward = new KeyboardEvent("keydown", {
        key: "Tab",
        shiftKey: true,
        bubbles: true,
        cancelable: true,
      });
      fireEvent(first, backward);
      expect(backward.defaultPrevented).toBe(true);
      expect(document.activeElement).toBe(last);
    }

    dialog.focus();
    fireEvent.keyDown(dialog, { key: "Tab" });
    expect(document.activeElement).toBe(first);
    dialog.focus();
    fireEvent.keyDown(dialog, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(last);
  });

  it("Escapeは最前面だけを1回呼び、stayでは破壊操作を呼ばない", async () => {
    const background = document.createElement("button");
    background.setAttribute("data-test-outside", "true");
    document.body.append(background);
    const closeLower = vi.fn();
    const closeUpper = vi.fn();
    const view = render(
      <>
        <BasicDialog
          name="下"
          escapeAction={{ kind: "dismiss", run: closeLower }}
        />
        <BasicDialog
          name="上"
          escapeAction={{ kind: "dismiss", run: closeUpper }}
        />
      </>,
    );

    const layers = document.querySelectorAll<HTMLElement>("[data-modal-layer='true']");
    expect(layers).toHaveLength(2);
    expect(layers[0].hasAttribute("inert")).toBe(true);
    expect(layers[1].hasAttribute("inert")).toBe(false);
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "上の最初" }));

    const escape = new KeyboardEvent("keydown", {
      key: "Escape",
      bubbles: true,
      cancelable: true,
    });
    fireEvent(document.activeElement as Element, escape);
    expect(escape.defaultPrevented).toBe(true);
    expect(closeUpper).toHaveBeenCalledTimes(1);
    expect(closeLower).toHaveBeenCalledTimes(0);

    view.rerender(<BasicDialog name="下" escapeAction={{ kind: "stay" }} />);
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "下の最初" })),
    );
    const remainingLayers = document.querySelectorAll<HTMLElement>(
      "[data-modal-layer='true']",
    );
    expect(remainingLayers).toHaveLength(1);
    expect(remainingLayers[0].hasAttribute("inert")).toBe(false);
    expect(background.hasAttribute("inert")).toBe(true);
    fireEvent.keyDown(document.activeElement as Element, { key: "Escape" });
    expect(closeLower).toHaveBeenCalledTimes(0);
    view.unmount();
    await waitFor(() => expect(background.hasAttribute("inert")).toBe(false));
  });

  it("子の操作がEscapeを使ったときは画面を閉じない", () => {
    const close = vi.fn();
    render(
      <ModalDialog
        labelledBy="child-escape-title"
        escapeAction={{ kind: "dismiss", run: close }}
      >
        <h2 id="child-escape-title">候補のある入力</h2>
        <button
          type="button"
          onKeyDown={(event) => {
            if (event.key === "Escape") event.preventDefault();
          }}
        >
          候補だけ閉じる
        </button>
      </ModalDialog>,
    );

    const button = screen.getByRole("button", { name: "候補だけ閉じる" });
    const escape = new KeyboardEvent("keydown", {
      key: "Escape",
      bubbles: true,
      cancelable: true,
    });
    fireEvent(button, escape);
    expect(escape.defaultPrevented).toBe(true);
    expect(close).toHaveBeenCalledTimes(0);
  });

  it("段階が変わった時だけ最初の項目を選び直し、通常の再描画では奪わない", () => {
    function Stepped({ step }: { step: number }) {
      const target = useRef<HTMLButtonElement>(null);
      return (
        <ModalDialog
          labelledBy="step-title"
          initialFocusRef={target}
          initialFocusKey={step}
          escapeAction={{ kind: "stay" }}
        >
          <h2 id="step-title">段階{step}</h2>
          <button ref={target} type="button">段階{step}の最初</button>
          <button type="button">利用者が選んだ項目</button>
        </ModalDialog>
      );
    }
    const { rerender } = render(<Stepped step={1} />);
    const chosen = screen.getByRole("button", { name: "利用者が選んだ項目" });
    chosen.focus();
    rerender(<Stepped step={1} />);
    expect(document.activeElement).toBe(chosen);

    rerender(<Stepped step={2} />);
    expect(document.activeElement).toBe(
      screen.getByRole("button", { name: "段階2の最初" }),
    );
  });

  it("閉じた後は起点へ戻り、起点が消えていれば明示した予備へ戻る", async () => {
    const trigger = document.createElement("button");
    trigger.textContent = "起点";
    trigger.setAttribute("data-test-outside", "true");
    document.body.append(trigger);
    trigger.focus();

    const first = render(<BasicDialog name="復帰1" escapeAction={{ kind: "stay" }} />);
    first.unmount();
    await waitFor(() => expect(document.activeElement).toBe(trigger));

    const detachedReturnRef: React.RefObject<FocusTarget | null> = {
      current: document.createElement("button"),
    };
    trigger.focus();
    const invalidExplicit = render(
      <BasicDialog
        name="復帰1b"
        escapeAction={{ kind: "stay" }}
        returnFocusRef={detachedReturnRef}
      />,
    );
    invalidExplicit.unmount();
    await waitFor(() => expect(document.activeElement).toBe(trigger));

    const fallback = document.createElement("button");
    fallback.textContent = "予備";
    fallback.setAttribute("data-test-outside", "true");
    document.body.append(fallback);
    trigger.focus();
    const fallbackRef: React.RefObject<FocusTarget | null> = { current: fallback };
    const second = render(
      <BasicDialog
        name="復帰2"
        escapeAction={{ kind: "stay" }}
        fallbackFocusRef={fallbackRef}
      />,
    );
    trigger.remove();
    second.unmount();
    await waitFor(() => expect(document.activeElement).toBe(fallback));
  });

  it("重なった上の画面を閉じると、下の画面内で明示した場所へ戻る", async () => {
    function NestedDialogs({ showUpper }: { showUpper: boolean }) {
      const lowerReturnRef = useRef<HTMLButtonElement>(null);
      const upperInitialRef = useRef<HTMLButtonElement>(null);
      return (
        <>
          <ModalDialog labelledBy="nested-lower-title" escapeAction={{ kind: "stay" }}>
            <h2 id="nested-lower-title">下の画面</h2>
            <button type="button">下の直前の場所</button>
            <button ref={lowerReturnRef} type="button">下の明示した場所</button>
          </ModalDialog>
          {showUpper ? (
            <ModalDialog
              labelledBy="nested-upper-title"
              initialFocusRef={upperInitialRef}
              returnFocusRef={lowerReturnRef}
              escapeAction={{ kind: "stay" }}
            >
              <h2 id="nested-upper-title">上の画面</h2>
              <button ref={upperInitialRef} type="button">上の最初</button>
            </ModalDialog>
          ) : null}
        </>
      );
    }

    const view = render(<NestedDialogs showUpper />);
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "上の最初" }));
    view.rerender(<NestedDialogs showUpper={false} />);
    await waitFor(() =>
      expect(document.activeElement).toBe(
        screen.getByRole("button", { name: "下の明示した場所" }),
      ),
    );
  });

  it("別々に表示した画面を下から同時に閉じても、元の起点へ戻る", async () => {
    const decoy = document.createElement("button");
    decoy.setAttribute("data-test-outside", "true");
    const trigger = document.createElement("button");
    trigger.setAttribute("data-test-outside", "true");
    document.body.append(decoy, trigger);
    trigger.focus();

    const lower = render(<BasicDialog name="別root下" escapeAction={{ kind: "stay" }} />);
    const upper = render(<BasicDialog name="別root上" escapeAction={{ kind: "stay" }} />);
    expect(document.activeElement).toBe(
      screen.getByRole("button", { name: "別root上の最初" }),
    );

    lower.unmount();
    expect(document.activeElement).toBe(
      screen.getByRole("button", { name: "別root上の最初" }),
    );
    upper.unmount();
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });

  it("下の画面の段階変更を、上の画面を閉じた直後に反映する", async () => {
    function CoveredStep({ step, showUpper }: { step: number; showUpper: boolean }) {
      const lowerInitialRef = useRef<HTMLButtonElement>(null);
      const upperInitialRef = useRef<HTMLButtonElement>(null);
      return (
        <>
          <ModalDialog
            labelledBy="covered-lower-title"
            initialFocusRef={lowerInitialRef}
            initialFocusKey={step}
            escapeAction={{ kind: "stay" }}
          >
            <h2 id="covered-lower-title">下の段階{step}</h2>
            <button ref={lowerInitialRef} type="button">下の段階{step}の最初</button>
          </ModalDialog>
          {showUpper ? (
            <ModalDialog
              labelledBy="covered-upper-title"
              initialFocusRef={upperInitialRef}
              escapeAction={{ kind: "stay" }}
            >
              <h2 id="covered-upper-title">上の画面</h2>
              <button ref={upperInitialRef} type="button">上の最初</button>
            </ModalDialog>
          ) : null}
        </>
      );
    }

    const view = render(<CoveredStep step={1} showUpper />);
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "上の最初" }));
    view.rerender(<CoveredStep step={2} showUpper />);
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "上の最初" }));
    view.rerender(<CoveredStep step={2} showUpper={false} />);
    await waitFor(() =>
      expect(document.activeElement).toBe(
        screen.getByRole("button", { name: "下の段階2の最初" }),
      ),
    );
  });

  it("開く前がbodyでもbodyへ黙って戻さず、背景の有効な項目を選ぶ", async () => {
    const fallback = document.createElement("button");
    fallback.textContent = "背景の先頭";
    fallback.setAttribute("data-test-outside", "true");
    document.body.append(fallback);
    (document.activeElement as HTMLElement | null)?.blur();
    expect(document.activeElement).toBe(document.body);

    const detachedFallbackRef: React.RefObject<FocusTarget | null> = {
      current: document.createElement("button"),
    };
    const view = render(
      <BasicDialog
        name="body以外"
        escapeAction={{ kind: "stay" }}
        fallbackFocusRef={detachedFallbackRef}
      />,
    );
    view.unmount();
    await waitFor(() => expect(document.activeElement).toBe(fallback));
  });

  it("背景と下位画面をinertにし、最後に元の属性だけを戻す", async () => {
    const normal = document.createElement("button");
    normal.setAttribute("data-test-outside", "true");
    const alreadyInert = document.createElement("section");
    alreadyInert.setAttribute("data-test-outside", "true");
    alreadyInert.setAttribute("inert", "");
    document.body.append(normal, alreadyInert);

    const view = render(<BasicDialog name="背景" escapeAction={{ kind: "stay" }} />);
    expect(normal.hasAttribute("inert")).toBe(true);
    expect(alreadyInert.hasAttribute("inert")).toBe(true);

    const late = document.createElement("button");
    late.setAttribute("data-test-outside", "true");
    document.body.append(late);
    await waitFor(() => expect(late.hasAttribute("inert")).toBe(true));

    normal.removeAttribute("inert");
    await waitFor(() => expect(normal.hasAttribute("inert")).toBe(true));

    normal.focus();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "背景の最初" }));

    view.unmount();
    await waitFor(() => expect(normal.hasAttribute("inert")).toBe(false));
    expect(late.hasAttribute("inert")).toBe(false);
    expect(alreadyInert.hasAttribute("inert")).toBe(true);
  });

  it("画面内のDeleteを背景の全体キー処理へ流さず、F1入口だけを保つ", () => {
    const backgroundKey = vi.fn();
    window.addEventListener("keydown", backgroundKey);
    render(<BasicDialog name="キー" escapeAction={{ kind: "stay" }} />);
    const first = screen.getByRole("button", { name: "キーの最初" });

    fireEvent.keyDown(first, { key: "Delete" });
    expect(backgroundKey).toHaveBeenCalledTimes(0);
    fireEvent.keyDown(first, { key: "F1" });
    expect(backgroundKey).toHaveBeenCalledTimes(1);
    window.removeEventListener("keydown", backgroundKey);
  });

  it("StrictModeと同時mountでも登録を重ねず、上を閉じると下へ戻る", async () => {
    const decoy = document.createElement("button");
    decoy.setAttribute("data-test-outside", "true");
    const trigger = document.createElement("button");
    trigger.setAttribute("data-test-outside", "true");
    document.body.append(decoy, trigger);
    trigger.focus();
    const close = vi.fn();
    const view = render(
      <StrictMode>
        <BasicDialog name="一枚目" escapeAction={{ kind: "stay" }} />
        <BasicDialog
          name="二枚目"
          escapeAction={{ kind: "dismiss", run: close }}
        />
      </StrictMode>,
    );
    expect(document.querySelectorAll("[data-modal-layer='true']")).toHaveLength(2);
    fireEvent.keyDown(document.activeElement as Element, { key: "Escape" });
    expect(close).toHaveBeenCalledTimes(1);

    view.rerender(
      <StrictMode>
        <BasicDialog name="一枚目" escapeAction={{ kind: "stay" }} />
      </StrictMode>,
    );
    await waitFor(() =>
      expect(document.activeElement).toBe(
        screen.getByRole("button", { name: "一枚目の最初" }),
      ),
    );
    view.unmount();
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });

  it("100回mount/unmount後にlistener・inert・画面の残りが0件", async () => {
    const background = document.createElement("button");
    background.setAttribute("data-test-outside", "true");
    document.body.append(background);
    const add = vi.spyOn(document, "addEventListener");
    const remove = vi.spyOn(document, "removeEventListener");
    const close = vi.fn();

    for (let index = 0; index < 100; index += 1) {
      const view = render(
        <BasicDialog
          name={`反復${index}`}
          escapeAction={{ kind: "dismiss", run: close }}
        />,
      );
      expect(background.hasAttribute("inert")).toBe(true);
      view.unmount();
      await Promise.resolve();
      expect(background.hasAttribute("inert")).toBe(false);
    }

    const addedKeydown = add.mock.calls.filter(
      ([type, , options]) => type === "keydown" && options === true,
    );
    const removedKeydown = remove.mock.calls.filter(
      ([type, , options]) => type === "keydown" && options === true,
    );
    const addedFocusin = add.mock.calls.filter(
      ([type, , options]) => type === "focusin" && options === true,
    );
    const removedFocusin = remove.mock.calls.filter(
      ([type, , options]) => type === "focusin" && options === true,
    );
    expect(addedKeydown).toHaveLength(100);
    expect(removedKeydown).toHaveLength(100);
    expect(addedFocusin).toHaveLength(100);
    expect(removedFocusin).toHaveLength(100);
    expect(document.querySelectorAll("[data-modal-layer='true']")).toHaveLength(0);
    expect(document.querySelectorAll("[data-modal-layer='true'] [tabindex='0']")).toHaveLength(0);

    fireEvent.keyDown(document, { key: "Escape" });
    expect(close).toHaveBeenCalledTimes(0);
  });
});
