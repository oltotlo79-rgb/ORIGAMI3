import {
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import {
  VIEWER_OVERLAY_SCROLL_CONTROLS_HEIGHT_PX,
  viewerOverlayScrollState,
  viewerOverlayScrollTarget,
  type ViewerOverlayScrollMetrics,
  type ViewerOverlayScrollState,
} from "./viewerOverlayLayout";

interface ViewerOverlayStackProps {
  children: ReactNode;
}

const EMPTY_SCROLL_STATE = viewerOverlayScrollState({
  contentHeight: 0,
  availableHeight: 0,
  scrollTop: 0,
});

/** flexで積んだ直下の札を、折返し後の実寸とgap込みで測る。 */
function stackedContentHeight(stack: HTMLElement): number {
  const cards = Array.from(stack.children).filter(
    (child): child is HTMLElement => child instanceof HTMLElement,
  );
  if (cards.length === 0) return 0;
  const top = Math.min(...cards.map((card) => card.offsetTop));
  const bottom = Math.max(
    ...cards.map((card) => card.offsetTop + card.offsetHeight),
  );
  return Math.max(0, bottom - top);
}

function sameScrollState(
  previous: ViewerOverlayScrollState,
  next: ViewerOverlayScrollState,
): boolean {
  return (
    previous.overflowing === next.overflowing &&
    previous.canScrollUp === next.canScrollUp &&
    previous.canScrollDown === next.canScrollDown &&
    previous.scrollTop === next.scrollTop &&
    previous.maxScrollTop === next.maxScrollTop &&
    previous.viewportHeight === next.viewportHeight
  );
}

/**
 * 3D上の札を1列へまとめる。列自体は紙へのクリックを通し、高さ不足時だけ
 * キーボードでも押せる上下ボタンを列の外に出す。
 */
export function ViewerOverlayStack({ children }: ViewerOverlayStackProps) {
  const regionRef = useRef<HTMLDivElement>(null);
  const stackRef = useRef<HTMLDivElement>(null);
  const [scrollState, setScrollState] =
    useState<ViewerOverlayScrollState>(EMPTY_SCROLL_STATE);

  const readMetrics = useCallback((): ViewerOverlayScrollMetrics | null => {
    const region = regionRef.current;
    const stack = stackRef.current;
    if (!region || !stack) return null;
    return {
      contentHeight: stackedContentHeight(stack),
      availableHeight: region.clientHeight,
      scrollTop: stack.scrollTop,
    };
  }, []);

  const updateScrollState = useCallback(() => {
    const metrics = readMetrics();
    const stack = stackRef.current;
    if (!metrics || !stack) return;
    const next = viewerOverlayScrollState(metrics);
    if (stack.scrollTop !== next.scrollTop) stack.scrollTop = next.scrollTop;
    setScrollState((previous) =>
      sameScrollState(previous, next) ? previous : next,
    );
  }, [readMetrics]);

  useLayoutEffect(() => {
    const region = regionRef.current;
    const stack = stackRef.current;
    if (!region || !stack) return;

    const ResizeObserverClass = window.ResizeObserver;
    const resizeObserver =
      typeof ResizeObserverClass === "function"
        ? new ResizeObserverClass(updateScrollState)
        : null;
    const observeCurrentCards = () => {
      if (!resizeObserver) return;
      resizeObserver.disconnect();
      resizeObserver.observe(region);
      resizeObserver.observe(stack);
      for (const card of Array.from(stack.children)) {
        resizeObserver.observe(card);
      }
    };
    observeCurrentCards();

    const MutationObserverClass = window.MutationObserver;
    const mutationObserver =
      typeof MutationObserverClass === "function"
        ? new MutationObserverClass(() => {
            observeCurrentCards();
            updateScrollState();
          })
        : null;
    mutationObserver?.observe(stack, {
      childList: true,
      subtree: true,
      characterData: true,
    });
    window.addEventListener("resize", updateScrollState);
    const initialFrame = window.requestAnimationFrame(updateScrollState);
    return () => {
      window.cancelAnimationFrame(initialFrame);
      resizeObserver?.disconnect();
      mutationObserver?.disconnect();
      window.removeEventListener("resize", updateScrollState);
    };
  }, [updateScrollState]);

  const send = (direction: "up" | "down") => {
    const metrics = readMetrics();
    const stack = stackRef.current;
    if (!metrics || !stack) return;
    stack.scrollTop = viewerOverlayScrollTarget(metrics, direction);
    updateScrollState();
  };

  return (
    <div
      ref={regionRef}
      className="viewer-overlay-region"
      data-overflow={scrollState.overflowing ? "true" : "false"}
      style={
        {
          "--viewer-overlay-scroll-controls-height": `${VIEWER_OVERLAY_SCROLL_CONTROLS_HEIGHT_PX}px`,
        } as CSSProperties
      }
    >
      <div
        ref={stackRef}
        className="viewer-overlay-stack"
        onScroll={updateScrollState}
      >
        {children}
      </div>
      {scrollState.overflowing ? (
        <div className="viewer-overlay-scroll-controls" aria-label="3Dの案内を送る">
          <button
            type="button"
            aria-label="3Dの案内を上へ送る"
            data-tooltip="3Dの案内を上へ送る"
            disabled={!scrollState.canScrollUp}
            onClick={() => send("up")}
          >
            ▲ 上へ
          </button>
          <button
            type="button"
            aria-label="3Dの案内を下へ送る"
            data-tooltip="3Dの案内を下へ送る"
            disabled={!scrollState.canScrollDown}
            onClick={() => send("down")}
          >
            ▼ 下へ
          </button>
        </div>
      ) : null}
    </div>
  );
}
