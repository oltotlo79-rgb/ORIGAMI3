import {
  useLayoutEffect,
  useRef,
  type HTMLAttributes,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import { useAppStore } from "../../store/appStore";

export type FocusTarget = Element & {
  focus: (options?: FocusOptions) => void;
  tabIndex: number;
};

export type ModalEscapeAction =
  | { kind: "dismiss"; run: () => void }
  | { kind: "stay" };

interface StackEntry {
  id: symbol;
  dialog: HTMLDivElement;
  layer: HTMLDivElement;
  initialFocusRef?: RefObject<FocusTarget | null>;
  fallbackFocusRef?: RefObject<FocusTarget | null>;
  returnFocusRef?: RefObject<FocusTarget | null>;
  escapeAction: ModalEscapeAction;
  returnTarget: FocusTarget | null;
  lastFocused: FocusTarget | null;
  needsInitialFocus: boolean;
}

interface StackEntryOptions {
  initialFocusRef?: RefObject<FocusTarget | null>;
  fallbackFocusRef?: RefObject<FocusTarget | null>;
  returnFocusRef?: RefObject<FocusTarget | null>;
  escapeAction: ModalEscapeAction;
}

const stack: StackEntry[] = [];
const inertBeforeModal = new Map<HTMLElement, boolean>();
let bodyObserver: MutationObserver | null = null;
let listenersInstalled = false;

// 共通のfocus輪は2px + 外向き2px。SVGの丸い操作は、別要素で
// より大きい輪を描くため、対象半径ぶんの余白を取る。
const FOCUS_REVEAL_MARGIN_PX = 4;

const FOCUSABLE_SELECTOR = [
  "a[href]",
  "area[href]",
  "button",
  "input",
  "select",
  "textarea",
  "iframe",
  "object",
  "embed",
  "summary",
  "audio[controls]",
  "video[controls]",
  "[contenteditable]",
  "[tabindex]",
].join(",");

function asFocusTarget(element: Element | null): FocusTarget | null {
  if (
    element &&
    "focus" in element &&
    typeof (element as { focus?: unknown }).focus === "function" &&
    "tabIndex" in element
  ) {
    return element as FocusTarget;
  }
  return null;
}

function hasDisabledState(element: Element): boolean {
  if (element.getAttribute("aria-disabled") === "true") return true;
  if ("disabled" in element && (element as { disabled?: boolean }).disabled === true) {
    return true;
  }
  let fieldset = element.closest("fieldset[disabled]");
  while (fieldset) {
    const firstLegend = [...fieldset.children].find(
      (child) => child instanceof HTMLLegendElement,
    );
    if (!firstLegend?.contains(element)) return true;
    fieldset = fieldset.parentElement?.closest("fieldset[disabled]") ?? null;
  }
  return false;
}

function isHidden(element: Element, ignoreInert = false): boolean {
  const hiddenSelector = ignoreInert
    ? "[hidden], [aria-hidden='true']"
    : "[hidden], [aria-hidden='true'], [inert]";
  if (element.closest(hiddenSelector)) return true;
  let current: Element | null = element;
  while (current) {
    const style = window.getComputedStyle(current);
    if (style.display === "none" || style.visibility === "hidden") return true;
    if (current instanceof HTMLDetailsElement && !current.open) {
      const firstSummary = [...current.children].find(
        (child) => child instanceof HTMLElement && child.tagName === "SUMMARY",
      );
      if (!firstSummary?.contains(element)) return true;
    }
    current = current.parentElement;
  }
  return false;
}

function isRadioTabStop(element: FocusTarget, root: ParentNode): boolean {
  if (!(element instanceof HTMLInputElement) || element.type !== "radio" || !element.name) {
    return true;
  }
  const group = [...root.querySelectorAll<HTMLInputElement>("input[type='radio']")].filter(
    (radio) =>
      radio.name === element.name &&
      radio.form === element.form &&
      !hasDisabledState(radio) &&
      !isHidden(radio),
  );
  const checked = group.find((radio) => radio.checked);
  return checked ? checked === element : group[0] === element;
}

/** 現在のTab順に入る、有効で見えている要素だけを返す。 */
export function focusableElements(root: ParentNode): FocusTarget[] {
  const candidates = [...root.querySelectorAll(FOCUSABLE_SELECTOR)]
    .map(asFocusTarget)
    .filter((element): element is FocusTarget => element !== null)
    .filter(
      (element) =>
        element.isConnected &&
        element.tabIndex >= 0 &&
        !hasDisabledState(element) &&
        !(element instanceof HTMLInputElement && element.type === "hidden") &&
        !isHidden(element) &&
        isRadioTabStop(element, root),
    );

  return candidates
    .map((element, order) => ({ element, order }))
    .sort((a, b) => {
      const aIndex = a.element.tabIndex;
      const bIndex = b.element.tabIndex;
      if (aIndex > 0 && bIndex === 0) return -1;
      if (aIndex === 0 && bIndex > 0) return 1;
      if (aIndex > 0 && bIndex > 0 && aIndex !== bIndex) return aIndex - bIndex;
      return a.order - b.order;
    })
    .map(({ element }) => element);
}

function canRestoreFocus(element: FocusTarget | null): element is FocusTarget {
  return (
    element !== null &&
    element !== document.body &&
    element !== document.documentElement &&
    element.isConnected &&
    !hasDisabledState(element) &&
    !isHidden(element)
  );
}

function canPreserveReturnTarget(element: FocusTarget | null): element is FocusTarget {
  return (
    element !== null &&
    element !== document.body &&
    element !== document.documentElement &&
    element.isConnected &&
    !hasDisabledState(element) &&
    !isHidden(element, true)
  );
}

function focusWithoutScroll(element: FocusTarget): void {
  try {
    element.focus({ preventScroll: true });
  } catch {
    element.focus();
  }
}

function focusRevealMargin(element: FocusTarget, rect: DOMRect): number {
  const hasLargerSvgRing =
    element.hasAttribute("data-paper-position-handle") ||
    element.hasAttribute("data-tip-handle");
  return hasLargerSvgRing
    ? Math.max(FOCUS_REVEAL_MARGIN_PX, rect.height / 2)
    : FOCUS_REVEAL_MARGIN_PX;
}

/**
 * Tabで選ばれた項目を、ダイアログ内の各縦送り領域へfocus輪ごと表示する。
 * 横位置・window・ダイアログを開く前の背景位置は動かさない。
 */
function revealFocusedTarget(dialog: HTMLElement, target: FocusTarget): void {
  if (target === dialog || !dialog.contains(target)) return;

  let scroller = target.parentElement;
  while (scroller && dialog.contains(scroller)) {
    const style = window.getComputedStyle(scroller);
    const canScrollVertically =
      (style.overflowY === "auto" || style.overflowY === "scroll") &&
      scroller.clientHeight > 0 &&
      scroller.scrollHeight > scroller.clientHeight;
    if (canScrollVertically) {
      const view = scroller.getBoundingClientRect();
      const item = target.getBoundingClientRect();
      const wantedMargin = focusRevealMargin(target, item);
      const margin = Math.min(
        wantedMargin,
        Math.max(0, (scroller.clientHeight - item.height) / 2),
      );
      const viewTop = view.top + scroller.clientTop;
      const viewBottom = viewTop + scroller.clientHeight;
      let next = scroller.scrollTop;
      if (item.top < viewTop + margin) {
        next += item.top - (viewTop + margin);
      } else if (item.bottom > viewBottom - margin) {
        next += item.bottom - (viewBottom - margin);
      }
      const max = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
      next = Math.min(max, Math.max(0, next));
      if (next !== scroller.scrollTop) scroller.scrollTop = next;
    }
    if (scroller === dialog) break;
    scroller = scroller.parentElement;
  }
}

function initialTarget(entry: StackEntry): FocusTarget {
  const requested = entry.initialFocusRef?.current ?? null;
  if (
    canRestoreFocus(requested) &&
    entry.dialog.contains(requested)
  ) {
    return requested;
  }
  return focusableElements(entry.dialog)[0] ?? (entry.dialog as FocusTarget);
}

function focusInitial(entry: StackEntry): void {
  const target = initialTarget(entry);
  entry.needsInitialFocus = false;
  entry.lastFocused = target;
  focusWithoutScroll(target);
}

function topEntry(): StackEntry | null {
  return stack[stack.length - 1] ?? null;
}

/** Tab・Shift+Tab・Escapeを、最前面の画面だけで処理する。 */
export function handleDialogKeyDown(event: KeyboardEvent, entry: StackEntry): void {
  if (topEntry() !== entry) return;
  if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    if (!event.repeat && entry.escapeAction.kind === "dismiss") {
      entry.escapeAction.run();
    }
    return;
  }
  if (event.key !== "Tab") return;

  const elements = focusableElements(entry.dialog);
  const active = asFocusTarget(document.activeElement);
  if (elements.length === 0) {
    event.preventDefault();
    focusWithoutScroll(entry.dialog as FocusTarget);
    return;
  }

  const first = elements[0];
  const last = elements[elements.length - 1];
  const outsideTabOrder = active === null || !elements.includes(active);
  if (
    outsideTabOrder ||
    (event.shiftKey && active === first) ||
    (!event.shiftKey && active === last)
  ) {
    event.preventDefault();
    focusWithoutScroll(event.shiftKey ? last : first);
  }
}

function handleDocumentKeyDown(event: KeyboardEvent): void {
  // Escapeは子要素が先に使えるよう、dialogのbubble段階で処理する。
  if (event.key !== "Tab") return;
  const entry = topEntry();
  if (entry) handleDialogKeyDown(event, entry);
}

function handleDocumentFocusIn(event: FocusEvent): void {
  const entry = topEntry();
  const target = asFocusTarget(event.target instanceof Element ? event.target : null);
  if (!entry || !target) return;
  if (entry.dialog.contains(target)) {
    entry.lastFocused = target;
    revealFocusedTarget(entry.dialog, target);
    return;
  }
  const fallback =
    canRestoreFocus(entry.lastFocused) && entry.dialog.contains(entry.lastFocused)
      ? entry.lastFocused
      : initialTarget(entry);
  focusWithoutScroll(fallback);
}

function rememberInert(element: HTMLElement): void {
  if (!inertBeforeModal.has(element)) {
    inertBeforeModal.set(element, element.hasAttribute("inert"));
  }
  if (!element.hasAttribute("inert")) element.setAttribute("inert", "");
}

function restoreRememberedInert(element: HTMLElement): void {
  const hadInert = inertBeforeModal.get(element);
  if (hadInert === undefined) return;
  if (hadInert && !element.hasAttribute("inert")) element.setAttribute("inert", "");
  if (!hadInert && element.hasAttribute("inert")) element.removeAttribute("inert");
  inertBeforeModal.delete(element);
}

function reconcileInert(): void {
  const top = topEntry();
  const modalLayers = new Set<HTMLElement>(stack.map((entry) => entry.layer));
  const shouldBeInert = new Set<HTMLElement>();
  if (top) {
    for (const child of document.body.children) {
      if (child instanceof HTMLElement && !modalLayers.has(child)) shouldBeInert.add(child);
    }
    for (const entry of stack) {
      if (entry !== top) shouldBeInert.add(entry.layer);
    }
  }

  for (const element of [...inertBeforeModal.keys()]) {
    if (!shouldBeInert.has(element)) restoreRememberedInert(element);
  }
  for (const element of shouldBeInert) rememberInert(element);
}

function installGlobalLifecycle(): void {
  if (listenersInstalled) return;
  document.addEventListener("keydown", handleDocumentKeyDown, true);
  document.addEventListener("focusin", handleDocumentFocusIn, true);
  bodyObserver = new MutationObserver(reconcileInert);
  bodyObserver.observe(document.body, {
    childList: true,
    attributes: true,
    attributeFilter: ["inert"],
    subtree: true,
  });
  listenersInstalled = true;
}

function removeGlobalLifecycle(): void {
  if (!listenersInstalled) return;
  document.removeEventListener("keydown", handleDocumentKeyDown, true);
  document.removeEventListener("focusin", handleDocumentFocusIn, true);
  bodyObserver?.disconnect();
  bodyObserver = null;
  listenersInstalled = false;
}

/** 明示した復帰先、開く直前の要素、明示した予備、背景の先頭の順で戻す。 */
export function restoreFocus(entry: StackEntry): void {
  queueMicrotask(() => {
    const remaining = topEntry();
    if (remaining) {
      if (remaining.needsInitialFocus) {
        focusInitial(remaining);
        return;
      }
      const returnInsideRemaining = [
        entry.returnFocusRef?.current ?? null,
        entry.returnTarget,
        entry.fallbackFocusRef?.current ?? null,
      ].find(
        (candidate): candidate is FocusTarget =>
          canRestoreFocus(candidate) && remaining.dialog.contains(candidate),
      );
      const target =
        returnInsideRemaining ??
        (canRestoreFocus(remaining.lastFocused) && remaining.dialog.contains(remaining.lastFocused)
          ? remaining.lastFocused
          : initialTarget(remaining));
      focusWithoutScroll(target);
      return;
    }

    const requested = [entry.returnFocusRef?.current ?? null, entry.returnTarget].find(
      canRestoreFocus,
    );
    if (requested) {
      focusWithoutScroll(requested);
      return;
    }
    const fallback = [
      entry.fallbackFocusRef?.current ?? null,
      ...focusableElements(document.body),
    ].find(canRestoreFocus);
    if (fallback) focusWithoutScroll(fallback);
  });
}

/** DOM参照だけの一時的な台帳。画面のopen/busy等は既存Zustandストアを正とする。 */
export function useModalStack(
  dialogRef: RefObject<HTMLDivElement | null>,
  layerRef: RefObject<HTMLDivElement | null>,
  options: StackEntryOptions,
  initialFocusKey: string | number,
): RefObject<StackEntry | null> {
  const optionsRef = useRef(options);
  const entryRef = useRef<StackEntry | null>(null);
  const previousFocusKey = useRef(initialFocusKey);
  // StrictModeはDOMを残したままlayout effectだけをcleanup→setupする。
  // 2回目のsetup時は選択位置が既にdialog内なので、最初の外部起点をrefへ保全する。
  const preservedReturnTarget = useRef<FocusTarget | null>(null);
  const returnTargetCaptured = useRef(false);

  useLayoutEffect(() => {
    optionsRef.current = options;
    const entry = entryRef.current;
    if (entry) {
      Object.assign(entry, options);
      if (previousFocusKey.current !== initialFocusKey) {
        previousFocusKey.current = initialFocusKey;
        if (topEntry() === entry) focusInitial(entry);
        else entry.needsInitialFocus = true;
      }
    }
  });

  useLayoutEffect(() => {
    const dialog = dialogRef.current;
    const layer = layerRef.current;
    if (!dialog || !layer) return;
    const active = asFocusTarget(document.activeElement);
    const outsideTarget = active && !layer.contains(active) ? active : null;
    if (!returnTargetCaptured.current) {
      preservedReturnTarget.current = outsideTarget;
      returnTargetCaptured.current = true;
    }
    const entry: StackEntry = {
      id: Symbol("modal-dialog"),
      dialog,
      layer,
      ...optionsRef.current,
      returnTarget: preservedReturnTarget.current,
      lastFocused: null,
      needsInitialFocus: false,
    };
    entryRef.current = entry;
    stack.push(entry);
    installGlobalLifecycle();
    reconcileInert();
    focusInitial(entry);

    return () => {
      const index = stack.findIndex((candidate) => candidate.id === entry.id);
      const wasTopmost = index >= 0 && index === stack.length - 1;
      if (index >= 0 && !wasTopmost) {
        const nextEntry = stack[index + 1];
        const nextReturnsInsideClosingLayer =
          nextEntry.returnTarget !== null && entry.layer.contains(nextEntry.returnTarget);
        if (nextReturnsInsideClosingLayer || !canPreserveReturnTarget(nextEntry.returnTarget)) {
          nextEntry.returnTarget = [
            entry.returnFocusRef?.current ?? null,
            entry.returnTarget,
            entry.fallbackFocusRef?.current ?? null,
          ].find(
            (candidate): candidate is FocusTarget =>
              canPreserveReturnTarget(candidate) && !entry.layer.contains(candidate),
          ) ?? null;
        }
      }
      if (index >= 0) stack.splice(index, 1);
      entryRef.current = null;
      reconcileInert();
      if (stack.length === 0) removeGlobalLifecycle();
      // 下位だけを外す時は最前面の操作位置を奪わず、最終復帰先だけを引き継ぐ。
      if (wasTopmost) restoreFocus(entry);
    };
  }, [dialogRef, layerRef]);

  return entryRef;
}

export interface ModalDialogProps
  extends Omit<
    HTMLAttributes<HTMLDivElement>,
    | "aria-labelledby"
    | "aria-modal"
    | "children"
    | "onKeyDown"
    | "onKeyUp"
    | "role"
    | "tabIndex"
  > {
  children: ReactNode;
  labelledBy: string;
  initialFocusRef?: RefObject<FocusTarget | null>;
  /** 同じ画面内で段階が変わったときだけ、最初の要素を選び直すための値。 */
  initialFocusKey?: string | number;
  escapeAction: ModalEscapeAction;
  returnFocusRef?: RefObject<FocusTarget | null>;
  fallbackFocusRef?: RefObject<FocusTarget | null>;
  backdropClassName?: string;
  onBackdropDismiss?: () => void;
}

export function ModalDialog({
  children,
  labelledBy,
  initialFocusRef,
  initialFocusKey = "open",
  escapeAction,
  returnFocusRef,
  fallbackFocusRef,
  backdropClassName,
  onBackdropDismiss,
  className,
  ...dialogAttributes
}: ModalDialogProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const layerRef = useRef<HTMLDivElement>(null);
  const uiTheme = useAppStore((state) => state.uiTheme);
  const entryRef = useModalStack(
    dialogRef,
    layerRef,
    {
      initialFocusRef,
      fallbackFocusRef,
      returnFocusRef,
      escapeAction,
    },
    initialFocusKey,
  );

  const handleBackdropMouseDown = (event: ReactMouseEvent<HTMLDivElement>) => {
    if (
      event.target === event.currentTarget &&
      entryRef.current === topEntry() &&
      onBackdropDismiss
    ) {
      onBackdropDismiss();
    }
  };

  return createPortal(
    <div
      ref={layerRef}
      className={
        backdropClassName
          ? `app dialog-backdrop ${backdropClassName}`
          : "app dialog-backdrop"
      }
      data-theme={uiTheme === "pop" ? undefined : uiTheme}
      data-modal-layer="true"
      onMouseDown={handleBackdropMouseDown}
    >
      <div
        {...dialogAttributes}
        ref={dialogRef}
        className={className ? `dialog ${className}` : "dialog"}
        role="dialog"
        aria-modal="true"
        aria-labelledby={labelledBy}
        tabIndex={-1}
        onKeyDown={(event) => {
          if (event.key === "Escape" && !event.defaultPrevented && entryRef.current) {
            handleDialogKeyDown(event.nativeEvent, entryRef.current);
          }
          // 子要素の操作後、背景にある展開図などの全体キー処理へ流さない。
          // F1だけは既存のヘルプ入口へ渡す。
          if (event.key !== "F1") event.stopPropagation();
        }}
        onKeyUp={(event) => {
          if (event.key !== "F1") event.stopPropagation();
        }}
      >
        {children}
      </div>
    </div>,
    document.body,
  );
}
