/** 長い説明を2段へ切り替える実幅。App.cssのcontainer queryと同じ。 */
export const VIEWER_OVERLAY_STACK_MAX_WIDTH_PX = 520;
/** 視点立方体の横ではなく下へ列を移す実幅。 */
export const VIEWER_OVERLAY_STACK_BELOW_CUBE_MAX_WIDTH_PX = 360;
export const VIEWER_OVERLAY_COLUMN_MAX_WIDTH_PX = 430;
export const VIEWER_OVERLAY_INSET_PX = 12;
export const VIEWER_OVERLAY_CUBE_CLEARANCE_PX = 152;
export const VIEWER_OVERLAY_BELOW_CUBE_TOP_PX = 148;
export const VIEWER_OVERLAY_RESET_CLEARANCE_PX = 56;
export const VIEWER_OVERLAY_GAP_PX = 8;
/** 案内列があふれたとき、末尾を送るボタンで隠さないために空ける高さ。 */
export const VIEWER_OVERLAY_SCROLL_CONTROLS_HEIGHT_PX = 40;
/** 1回の「上へ／下へ」で最低限送る距離。 */
export const VIEWER_OVERLAY_SCROLL_MIN_STEP_PX = 48;
/** 浮動小数点誤差だけを同寸とみなし、1px未満でも読める範囲が切れたら操作を出す。 */
const VIEWER_OVERLAY_SIZE_EPSILON_PX = 1e-6;
/** 端付近で上下ボタンがちらつかないためのscrollTop許容差。 */
const VIEWER_OVERLAY_SCROLL_EDGE_EPSILON_PX = 1e-6;

export interface ViewerOverlayRect {
  left: number;
  top: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
}

export interface ViewerOverlayStackLayout {
  viewport: ViewerOverlayRect;
  cards: ViewerOverlayRect[];
  contentHeight: number;
  scrollRange: number;
  horizontalOverflowPx: number;
}

export interface ViewerOverlayScrollMetrics {
  /** 札をすべて自然な高さで積んだ高さ。 */
  contentHeight: number;
  /** 上下の固定余白を除いた案内領域全体の高さ。 */
  availableHeight: number;
  scrollTop: number;
}

export interface ViewerOverlayScrollState {
  overflowing: boolean;
  canScrollUp: boolean;
  canScrollDown: boolean;
  scrollTop: number;
  maxScrollTop: number;
  viewportHeight: number;
}

function rect(left: number, top: number, right: number, bottom: number): ViewerOverlayRect {
  return {
    left,
    top,
    right,
    bottom,
    width: Math.max(0, right - left),
    height: Math.max(0, bottom - top),
  };
}

function finiteNonNegative(value: number, name: string): number {
  if (!Number.isFinite(value) || value < 0) {
    throw new Error(`${name} must be finite and non-negative`);
  }
  return value;
}

/**
 * 案内列の送れる範囲と、上下ボタンの状態を決める。
 * ボタンは内容が案内領域へ収まる間は出さず、出す間だけ40pxの操作行を予約する。
 */
export function viewerOverlayScrollState(
  metrics: ViewerOverlayScrollMetrics,
): ViewerOverlayScrollState {
  const contentHeight = finiteNonNegative(metrics.contentHeight, "viewer overlay content height");
  const availableHeight = finiteNonNegative(
    metrics.availableHeight,
    "viewer overlay available height",
  );
  const requestedScrollTop = finiteNonNegative(metrics.scrollTop, "viewer overlay scroll top");
  const overflowing =
    contentHeight > availableHeight + VIEWER_OVERLAY_SIZE_EPSILON_PX;
  const viewportHeight = Math.max(
    0,
    availableHeight - (overflowing ? VIEWER_OVERLAY_SCROLL_CONTROLS_HEIGHT_PX : 0),
  );
  const maxScrollTop = overflowing ? Math.max(0, contentHeight - viewportHeight) : 0;
  const scrollTop = Math.min(requestedScrollTop, maxScrollTop);
  return {
    overflowing,
    canScrollUp: overflowing && scrollTop > VIEWER_OVERLAY_SCROLL_EDGE_EPSILON_PX,
    canScrollDown:
      overflowing && scrollTop < maxScrollTop - VIEWER_OVERLAY_SCROLL_EDGE_EPSILON_PX,
    scrollTop,
    maxScrollTop,
    viewportHeight,
  };
}

/** 上下ボタンを1回押した後のscrollTop。端を越えず、短い領域でも48pxは送る。 */
export function viewerOverlayScrollTarget(
  metrics: ViewerOverlayScrollMetrics,
  direction: "up" | "down",
): number {
  const state = viewerOverlayScrollState(metrics);
  if (!state.overflowing) return 0;
  const step = Math.max(
    VIEWER_OVERLAY_SCROLL_MIN_STEP_PX,
    state.viewportHeight * 0.75,
  );
  const target = state.scrollTop + (direction === "up" ? -step : step);
  return Math.max(0, Math.min(state.maxScrollTop, target));
}

/**
 * 3D幅にかかわらず札は1列にする。1000×700の既定465pxと最大744pxでは
 * 立方体の左、最狭186pxでは立方体の下を使う。
 * 高さ不足は外へ出さず、App.cssの `.viewer-overlay-region` 内だけを縦に送る。
 */
export function viewerOverlayStackViewport(
  paneWidth: number,
  paneHeight: number,
): ViewerOverlayRect {
  if (
    !Number.isFinite(paneWidth) ||
    !Number.isFinite(paneHeight) ||
    paneWidth < 0 ||
    paneHeight < 0
  ) {
    throw new Error("viewer dimensions must be finite and non-negative");
  }
  const belowCube = paneWidth <= VIEWER_OVERLAY_STACK_BELOW_CUBE_MAX_WIDTH_PX;
  const left = VIEWER_OVERLAY_INSET_PX;
  const top = belowCube
    ? VIEWER_OVERLAY_BELOW_CUBE_TOP_PX
    : VIEWER_OVERLAY_INSET_PX;
  const availableWidth = Math.max(
    0,
    paneWidth -
      left -
      (belowCube ? VIEWER_OVERLAY_INSET_PX : VIEWER_OVERLAY_CUBE_CLEARANCE_PX),
  );
  const right =
    left + Math.min(VIEWER_OVERLAY_COLUMN_MAX_WIDTH_PX, availableWidth);
  const bottom = Math.max(
    top,
    paneHeight -
      (belowCube ? VIEWER_OVERLAY_RESET_CLEARANCE_PX : VIEWER_OVERLAY_INSET_PX),
  );
  return rect(left, top, right, bottom);
}

/** flexの縦列を数値化し、全札が交差せず、最後まで送って読めることを検査する。 */
export function layoutViewerOverlayCards(
  paneWidth: number,
  paneHeight: number,
  cardHeights: readonly number[],
  gap = VIEWER_OVERLAY_GAP_PX,
): ViewerOverlayStackLayout {
  if (!Number.isFinite(gap) || gap < 0) {
    throw new Error("viewer overlay gap must be finite and non-negative");
  }
  if (cardHeights.some((height) => !Number.isFinite(height) || height < 0)) {
    throw new Error("viewer overlay heights must be finite and non-negative");
  }
  const viewport = viewerOverlayStackViewport(paneWidth, paneHeight);
  let top = viewport.top;
  const cards = cardHeights.map((height) => {
    const card = rect(viewport.left, top, viewport.right, top + height);
    top = card.bottom + gap;
    return card;
  });
  const contentHeight =
    cards.length === 0 ? 0 : cards[cards.length - 1].bottom - viewport.top;
  return {
    viewport,
    cards,
    contentHeight,
    scrollRange: Math.max(0, contentHeight - viewport.height),
    horizontalOverflowPx: Math.max(0, -viewport.left, viewport.right - paneWidth),
  };
}
