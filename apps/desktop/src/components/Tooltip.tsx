import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { createPortal } from "react-dom";

const TOOLTIP_TARGET_SELECTOR = "button, input, select, [data-tooltip]";
const VIEWPORT_PADDING = 8;
const TARGET_GAP = 8;
const MAX_GENERATED_TEXT_LENGTH = 72;
const TOOLTIP_ID = "origami3-active-tooltip";

interface ActiveTooltip {
  element: HTMLElement;
  text: string;
}

interface TooltipPosition {
  left: number;
  top: number;
}

const baseStyle: CSSProperties = {
  position: "fixed",
  zIndex: 10000,
  maxWidth: "min(320px, calc(100vw - 16px))",
  padding: "6px 9px",
  border: "1px solid rgba(255, 255, 255, 0.18)",
  borderRadius: "6px",
  background: "rgba(24, 29, 38, 0.96)",
  boxShadow: "0 4px 14px rgba(0, 0, 0, 0.28)",
  color: "#ffffff",
  fontSize: "12px",
  fontWeight: 500,
  lineHeight: 1.4,
  overflowWrap: "anywhere",
  pointerEvents: "none",
  whiteSpace: "normal",
};

function normalizeText(value: string | null | undefined): string {
  return value?.replace(/\s+/g, " ").trim() ?? "";
}

function shortenGeneratedText(value: string): string {
  if (value.length <= MAX_GENERATED_TEXT_LENGTH) return value;
  return `${value.slice(0, MAX_GENERATED_TEXT_LENGTH - 1).trimEnd()}…`;
}

function associatedLabelText(element: HTMLElement): string {
  if (
    element instanceof HTMLButtonElement ||
    element instanceof HTMLInputElement ||
    element instanceof HTMLSelectElement
  ) {
    for (const label of Array.from(element.labels ?? [])) {
      const text = normalizeText(label.textContent);
      if (text) return text;
    }
  }

  return normalizeText(element.closest("label")?.textContent);
}

/** data-tooltipが無い部品にも、既存のアクセシブル名から短い説明を付ける。 */
function tooltipTextFor(element: HTMLElement): string {
  if (element.hasAttribute("data-tooltip")) {
    return normalizeText(element.getAttribute("data-tooltip"));
  }

  const ariaLabel = normalizeText(element.getAttribute("aria-label"));
  if (ariaLabel) return shortenGeneratedText(ariaLabel);

  const label = associatedLabelText(element);
  if (label) return shortenGeneratedText(label);

  if (element instanceof HTMLInputElement) {
    const displayedValue = ["button", "submit", "reset"].includes(element.type)
      ? normalizeText(element.value)
      : normalizeText(element.placeholder);
    if (displayedValue) return shortenGeneratedText(displayedValue);
  }

  if (element instanceof HTMLSelectElement) {
    const selectedText = normalizeText(element.selectedOptions.item(0)?.textContent);
    if (selectedText) return shortenGeneratedText(selectedText);
  }

  return shortenGeneratedText(normalizeText(element.textContent));
}

function tooltipTarget(value: EventTarget | null): HTMLElement | null {
  if (!(value instanceof Element)) return null;
  const target = value.closest(TOOLTIP_TARGET_SELECTOR);
  return target instanceof HTMLElement ? target : null;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), Math.max(minimum, maximum));
}

function placeTooltip(
  targetRect: DOMRect,
  tooltipRect: DOMRect,
  viewportWidth: number,
  viewportHeight: number,
): TooltipPosition {
  const maximumLeft = viewportWidth - tooltipRect.width - VIEWPORT_PADDING;
  const maximumTop = viewportHeight - tooltipRect.height - VIEWPORT_PADDING;
  const centeredLeft =
    targetRect.left + targetRect.width / 2 - tooltipRect.width / 2;

  const above = targetRect.top - tooltipRect.height - TARGET_GAP;
  const below = targetRect.bottom + TARGET_GAP;
  const preferredTop = above >= VIEWPORT_PADDING ? above : below;

  return {
    left: clamp(centeredLeft, VIEWPORT_PADDING, maximumLeft),
    top: clamp(preferredTop, VIEWPORT_PADDING, maximumTop),
  };
}

/**
 * 画面全体の操作部品へイベント委譲で共通の吹き出しを付ける。
 * App内のどこかに一度だけ置けばよく、吹き出し自体はbodyへ描画する。
 */
export function TooltipHost() {
  const [active, setActive] = useState<ActiveTooltip | null>(null);
  const tooltipRef = useRef<HTMLDivElement>(null);
  const activeElementRef = useRef<HTMLElement | null>(null);
  const activeTextRef = useRef("");

  useEffect(() => {
    let hoveredElement: HTMLElement | null = null;
    let focusedElement: HTMLElement | null = null;

    const reveal = (element: HTMLElement | null) => {
      const text = element ? tooltipTextFor(element) : "";
      if (!element || !text) {
        activeElementRef.current = null;
        activeTextRef.current = "";
        setActive(null);
        return;
      }

      if (
        activeElementRef.current === element &&
        activeTextRef.current === text
      ) {
        return;
      }

      activeElementRef.current = element;
      activeTextRef.current = text;
      setActive({ element, text });
    };

    const onMouseOver = (event: MouseEvent) => {
      const next = tooltipTarget(event.target);
      if (!next || next === tooltipTarget(event.relatedTarget)) return;
      hoveredElement = next;
      reveal(focusedElement ?? hoveredElement);
    };

    const onMouseOut = (event: MouseEvent) => {
      const leaving = tooltipTarget(event.target);
      const entering = tooltipTarget(event.relatedTarget);
      if (!leaving || leaving === entering || hoveredElement !== leaving) return;
      hoveredElement = entering;
      reveal(focusedElement ?? hoveredElement);
    };

    const onFocusIn = (event: FocusEvent) => {
      const next = tooltipTarget(event.target);
      if (!next || next === tooltipTarget(event.relatedTarget)) return;
      focusedElement = next;
      reveal(focusedElement);
    };

    const onFocusOut = (event: FocusEvent) => {
      const leaving = tooltipTarget(event.target);
      const entering = tooltipTarget(event.relatedTarget);
      if (!leaving || leaving === entering || focusedElement !== leaving) return;
      focusedElement = entering;
      reveal(focusedElement ?? hoveredElement);
    };

    document.addEventListener("mouseover", onMouseOver);
    document.addEventListener("mouseout", onMouseOut);
    document.addEventListener("focusin", onFocusIn);
    document.addEventListener("focusout", onFocusOut);
    return () => {
      document.removeEventListener("mouseover", onMouseOver);
      document.removeEventListener("mouseout", onMouseOut);
      document.removeEventListener("focusin", onFocusIn);
      document.removeEventListener("focusout", onFocusOut);
    };
  }, []);

  useLayoutEffect(() => {
    if (!active) return;

    const updatePosition = () => {
      const tooltip = tooltipRef.current;
      if (!tooltip || !active.element.isConnected) {
        if (tooltip) tooltip.style.visibility = "hidden";
        return;
      }

      const next = placeTooltip(
        active.element.getBoundingClientRect(),
        tooltip.getBoundingClientRect(),
        window.innerWidth,
        window.innerHeight,
      );
      tooltip.style.left = `${next.left}px`;
      tooltip.style.top = `${next.top}px`;
      tooltip.style.visibility = "visible";
    };

    updatePosition();
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    return () => {
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
    };
  }, [active]);

  useEffect(() => {
    if (!active) return;
    const element = active.element;
    const previous = element.getAttribute("aria-describedby");
    const ids = new Set(normalizeText(previous).split(" ").filter(Boolean));
    ids.add(TOOLTIP_ID);
    element.setAttribute("aria-describedby", [...ids].join(" "));
    return () => {
      if (previous === null) element.removeAttribute("aria-describedby");
      else element.setAttribute("aria-describedby", previous);
    };
  }, [active]);

  if (!active) return null;

  return createPortal(
    <div
      id={TOOLTIP_ID}
      ref={tooltipRef}
      role="tooltip"
      data-floating-ui="tooltip"
      style={{
        ...baseStyle,
        left: VIEWPORT_PADDING,
        top: VIEWPORT_PADDING,
        visibility: "hidden",
      }}
    >
      {active.text}
    </div>,
    document.body,
  );
}
