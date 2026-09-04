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

/**
 * 1回の列挙のあいだだけ、祖先の「見えないか」を覚えておく手控え。
 *
 * 同じ画面の要素は祖先をほとんど共有するため、覚えておかないと同じ祖先の
 * 表示状態を要素の数だけ調べ直すことになる。判定の内容は変えず、同じ答えを
 * 二度計算しないだけである。DOMは列挙のあいだに変わらないので、手控えは
 * 列挙ごとに作って捨てる。
 */
type VisibilityMemo = Map<Element, boolean>;

/** 閉じた `<details>` の中（最初の `<summary>` の外）にあるか。 */
function isInsideClosedDetails(element: Element): boolean {
  let closed = element.closest("details:not([open])");
  while (closed instanceof HTMLDetailsElement) {
    const firstSummary = [...closed.children].find(
      (child) => child instanceof HTMLElement && child.tagName === "SUMMARY",
    );
    if (!firstSummary?.contains(element)) return true;
    closed = closed.parentElement?.closest("details:not([open])") ?? null;
  }
  return false;
}

/** 自分か祖先が `display:none` / `visibility:hidden` か。 */
function hasHiddenStyleChain(element: Element, memo?: VisibilityMemo): boolean {
  const walked: Element[] = [];
  let current: Element | null = element;
  let hidden = false;
  while (current) {
    const remembered = memo?.get(current);
    if (remembered !== undefined) {
      hidden = remembered;
      break;
    }
    const style = window.getComputedStyle(current);
    walked.push(current);
    if (style.display === "none" || style.visibility === "hidden") {
      hidden = true;
      break;
    }
    current = current.parentElement;
  }
  if (memo) for (const node of walked) memo.set(node, hidden);
  return hidden;
}

function isHidden(
  element: Element,
  ignoreInert = false,
  memo?: VisibilityMemo,
): boolean {
  const hiddenSelector = ignoreInert
    ? "[hidden], [aria-hidden='true']"
    : "[hidden], [aria-hidden='true'], [inert]";
  if (element.closest(hiddenSelector)) return true;
  if (isInsideClosedDetails(element)) return true;
  return hasHiddenStyleChain(element, memo);
}

function isRadioTabStop(
  element: FocusTarget,
  root: ParentNode,
  memo?: VisibilityMemo,
  radios?: HTMLInputElement[],
): boolean {
  if (!(element instanceof HTMLInputElement) || element.type !== "radio" || !element.name) {
    return true;
  }
  const all =
    radios ?? [...root.querySelectorAll<HTMLInputElement>("input[type='radio']")];
  const group = all.filter(
    (radio) =>
      radio.name === element.name &&
      radio.form === element.form &&
      !hasDisabledState(radio) &&
      !isHidden(radio, false, memo),
  );
  const checked = group.find((radio) => radio.checked);
  return checked ? checked === element : group[0] === element;
}

/**
 * 1回の走査のあいだ使い回す道具。表示状態の手控えと、radioの一覧を持つ。
 *
 * radioの一覧は、走査の中で最初にradioを見たときだけ作る。
 */
interface TabStopScan {
  root: ParentNode;
  memo: VisibilityMemo;
  radios?: HTMLInputElement[];
}

function newScan(root: ParentNode): TabStopScan {
  return { root, memo: new Map() };
}

/**
 * 表示状態を見ない、軽い側の条件だけを見る。
 *
 * `getComputedStyle` は1要素あたりの費用が大きいので、まずここで落とせるものを落とす。
 */
function isCheapTabStopCandidate(element: FocusTarget): boolean {
  return (
    element.isConnected &&
    element.tabIndex >= 0 &&
    !hasDisabledState(element) &&
    !(element instanceof HTMLInputElement && element.type === "hidden")
  );
}

/** 表示状態とradio groupまで見て、本当にTab順へ入るか決める。 */
function isVisibleTabStop(element: FocusTarget, scan: TabStopScan): boolean {
  if (isHidden(element, false, scan.memo)) return false;
  if (element instanceof HTMLInputElement && element.type === "radio") {
    scan.radios ??= [
      ...scan.root.querySelectorAll<HTMLInputElement>("input[type='radio']"),
    ];
  }
  return isRadioTabStop(element, scan.root, scan.memo, scan.radios);
}

/** Tab順に入り得る要素を、表示状態を見ずに書かれた順で集める。 */
function cheapTabStopCandidates(root: ParentNode): FocusTarget[] {
  return [...root.querySelectorAll(FOCUSABLE_SELECTOR)]
    .map(asFocusTarget)
    .filter((element): element is FocusTarget => element !== null)
    .filter(isCheapTabStopCandidate);
}

/**
 * Tab順の両端と、いま焦点のある要素がTab順に入っているかだけを求める。
 *
 * 折り返しの判定に要るのはこの3つだけなので、Tabのたびに全要素の表示状態を
 * 調べ直さず、前後の端が見つかった時点で止める。`tabindex` に正の値を持つ要素が
 * ある画面だけは並びが書かれた順と変わるため、そのときは今までどおり
 * 全体を並べてから両端を取る。判定の内容はどちらの経路でも同じである。
 */
function tabOrderEdges(
  root: ParentNode,
  active: FocusTarget | null,
): { first: FocusTarget | null; last: FocusTarget | null; activeInOrder: boolean } {
  const candidates = cheapTabStopCandidates(root);
  if (candidates.some((element) => element.tabIndex > 0)) {
    const ordered = focusableElements(root);
    return {
      first: ordered[0] ?? null,
      last: ordered[ordered.length - 1] ?? null,
      activeInOrder: active !== null && ordered.includes(active),
    };
  }

  const scan = newScan(root);
  let first: FocusTarget | null = null;
  for (const element of candidates) {
    if (isVisibleTabStop(element, scan)) {
      first = element;
      break;
    }
  }
  let last: FocusTarget | null = null;
  for (let index = candidates.length - 1; index >= 0; index -= 1) {
    if (isVisibleTabStop(candidates[index], scan)) {
      last = candidates[index];
      break;
    }
  }
  const activeInOrder =
    active !== null &&
    candidates.includes(active) &&
    isVisibleTabStop(active, scan);
  return { first, last, activeInOrder };
}

/** 現在のTab順に入る、有効で見えている要素だけを返す。 */
export function focusableElements(root: ParentNode): FocusTarget[] {
  const scan = newScan(root);
  const candidates = cheapTabStopCandidates(root).filter((element) =>
    isVisibleTabStop(element, scan),
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

  const active = asFocusTarget(document.activeElement);
  const { first, last, activeInOrder } = tabOrderEdges(entry.dialog, active);
  if (first === null || last === null) {
    event.preventDefault();
    focusWithoutScroll(entry.dialog as FocusTarget);
    return;
  }

  const outsideTabOrder = !activeInOrder;
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
