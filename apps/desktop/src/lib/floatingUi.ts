/** 画面内へ配置する浮動UIが使える領域。 */
export interface FloatingViewport {
  width: number;
  height: number;
}

/** 配置前に実測した浮動UIの大きさ。 */
export interface FloatingSize {
  width: number;
  height: number;
}

export interface FloatingPosition {
  left: number;
  top: number;
}

export interface FloatingPlacementOptions {
  padding?: number;
  gap?: number;
}

export interface FloatingOverflow {
  element: HTMLElement;
  rect: DOMRect;
}

export const FLOATING_UI_SELECTOR = "[data-floating-ui]";
export const DEFAULT_FLOATING_PADDING = 8;
export const DEFAULT_FLOATING_GAP = 8;

type AnchorRect = Pick<DOMRect, "left" | "top" | "right" | "bottom">;

function nonNegative(value: number, fallback: number): number {
  return Number.isFinite(value) ? Math.max(0, value) : fallback;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), Math.max(minimum, maximum));
}

/**
 * 起点の下・右方向へまず置き、入らなければ上・左方向へ反転する。
 * どちら側にもまとまった余白が無い場合は、画面の余白内へ位置を詰める。
 *
 * 浮動UI自体が画面より大きい場合は位置だけでは収められないため、呼び出し側も
 * `max-width` / `max-height` と内部スクロールを指定する。
 */
export function placeFloatingUi(
  anchor: AnchorRect,
  floating: FloatingSize,
  viewport: FloatingViewport,
  options: FloatingPlacementOptions = {},
): FloatingPosition {
  const padding = nonNegative(options.padding ?? DEFAULT_FLOATING_PADDING, DEFAULT_FLOATING_PADDING);
  const gap = nonNegative(options.gap ?? DEFAULT_FLOATING_GAP, DEFAULT_FLOATING_GAP);
  const width = nonNegative(floating.width, 0);
  const height = nonNegative(floating.height, 0);
  const viewportWidth = nonNegative(viewport.width, 0);
  const viewportHeight = nonNegative(viewport.height, 0);

  const minimumLeft = padding;
  const maximumLeft = Math.max(minimumLeft, viewportWidth - padding - width);
  const rightwardLeft = anchor.left;
  const leftwardLeft = anchor.right - width;
  const fitsRightward =
    rightwardLeft >= minimumLeft && rightwardLeft + width <= viewportWidth - padding;
  const fitsLeftward =
    leftwardLeft >= minimumLeft && leftwardLeft + width <= viewportWidth - padding;

  const minimumTop = padding;
  const maximumTop = Math.max(minimumTop, viewportHeight - padding - height);
  const belowTop = anchor.bottom + gap;
  const aboveTop = anchor.top - height - gap;
  const fitsBelow =
    belowTop >= minimumTop && belowTop + height <= viewportHeight - padding;
  const fitsAbove =
    aboveTop >= minimumTop && aboveTop + height <= viewportHeight - padding;

  return {
    left: fitsRightward
      ? rightwardLeft
      : fitsLeftward
        ? leftwardLeft
        : clamp(rightwardLeft, minimumLeft, maximumLeft),
    top: fitsBelow
      ? belowTop
      : fitsAbove
        ? aboveTop
        : clamp(belowTop, minimumTop, maximumTop),
  };
}

function isOutsideViewport(rect: DOMRect, viewport: FloatingViewport): boolean {
  const edges = [rect.left, rect.top, rect.right, rect.bottom];
  return (
    edges.some((edge) => !Number.isFinite(edge)) ||
    rect.left < 0 ||
    rect.top < 0 ||
    rect.right > viewport.width ||
    rect.bottom > viewport.height
  );
}

/**
 * 開いている浮動UIを属性で一括収集し、画面から1辺でも出ているものを返す。
 * 新しい浮動UIはルートへ `data-floating-ui` を付ければ同じ検査対象になる。
 */
export function findOverflowingFloatingUi(
  root: ParentNode = document,
  viewport: FloatingViewport = {
    width: window.innerWidth,
    height: window.innerHeight,
  },
): FloatingOverflow[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FLOATING_UI_SELECTOR)).flatMap(
    (element) => {
      const rect = element.getBoundingClientRect();
      return isOutsideViewport(rect, viewport) ? [{ element, rect }] : [];
    },
  );
}
