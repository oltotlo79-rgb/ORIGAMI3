// @vitest-environment jsdom

import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { useEffect, type ReactNode } from "react";

const captureApi = vi.hoisted(() => ({
  install: vi.fn(() => () => {}),
}));

vi.mock("../AppToolbar", () => ({
  AppToolbar: () => null,
  ExportButton: () => null,
}));
vi.mock("../ToolRail", () => ({
  ToolRail: () => (
    <button type="button" data-testid="tool-rail-operation">
      ツール操作
    </button>
  ),
}));
vi.mock("../ContextPanel", () => ({
  ContextPanel: () => (
    <button type="button" data-testid="context-panel-operation">
      設定操作
    </button>
  ),
}));
vi.mock("../CpEditor/CpEditor", () => ({
  CpEditor: () => (
    <button type="button" data-testid="cp-editor-operation">
      2D操作
    </button>
  ),
}));
vi.mock("../Timeline", () => ({
  Timeline: () => (
    <button type="button" data-testid="timeline-operation">
      手順操作
    </button>
  ),
}));
vi.mock("../ViewerStatusOverlays", () => ({
  ViewerStatusOverlays: () => null,
  relaxationStatus: () => null,
}));
vi.mock("../RecoveryDialog", () => ({ RecoveryDialog: () => null }));
vi.mock("../PaneSplitter", () => ({ PaneSplitter: () => null }));
vi.mock("../ContextPanelSplitter", () => ({
  ContextPanelSplitter: () => null,
}));
vi.mock("../dialogs/NewDocumentDialog", () => ({
  NewDocumentDialog: () => null,
}));
vi.mock("../HistoryShortcuts", () => ({ HistoryShortcuts: () => null }));
vi.mock("../FirstRunGuide", () => ({ FirstRunGuide: () => null }));
vi.mock("../ThemeRoot", () => ({
  ThemeRoot: ({ children }: { children: ReactNode }) => children,
}));
vi.mock("../Tooltip", () => ({ TooltipHost: () => null }));
vi.mock("../../captureApi", () => ({ installCaptureApi: captureApi.install }));
vi.mock("../../captureReadbackBridge", () => ({
  waitForViewer3DReady: vi.fn(async () => {}),
}));

import App from "../../App";
import { useAppStore } from "../../store/appStore";
import type { Viewer3DModuleLoader } from "./DeferredViewer3D";

type ViewerModule = Awaited<ReturnType<Viewer3DModuleLoader>>;

const VIEWER_WIDTH_PX = 480;
const VIEWER_HEIGHT_PX = 320;
// 合成矩形の実測差は縦横とも0px。仕様上限の1pxを境界に使い、
// 実測そのものを境界にしないことで描画系の1px丸めにも余裕を残す。
const MAX_ALLOCATION_DELTA_PX = 1;
const OPERATION_TEST_IDS = [
  "tool-rail-operation",
  "cp-editor-operation",
  "timeline-operation",
  "context-panel-operation",
] as const;

const realNewDocument = useAppStore.getState().newDocument;
const realCheckRecovery = useAppStore.getState().checkRecovery;
const initialDialogState = {
  proposalStep: useAppStore.getState().proposalStep,
  exportOpen: useAppStore.getState().exportOpen,
  helpOpen: useAppStore.getState().helpOpen,
};

let nextScheduleId = 1;
let animationFrames = new Map<number, FrameRequestCallback>();
let idleCallbacks = new Map<number, () => void>();

function viewerRect(): DOMRect {
  return {
    x: 0,
    y: 0,
    top: 0,
    right: VIEWER_WIDTH_PX,
    bottom: VIEWER_HEIGHT_PX,
    left: 0,
    width: VIEWER_WIDTH_PX,
    height: VIEWER_HEIGHT_PX,
    toJSON: () => ({}),
  } as DOMRect;
}

function runAnimationFrame(): void {
  const callbacks = [...animationFrames.values()];
  animationFrames.clear();
  for (const callback of callbacks) callback(0);
}

function runIdleCallbacks(): void {
  const callbacks = [...idleCallbacks.values()];
  idleCallbacks.clear();
  for (const callback of callbacks) callback();
}

async function requestDeferredViewer(): Promise<void> {
  await act(async () => {
    runAnimationFrame();
    runAnimationFrame();
    runIdleCallbacks();
    await Promise.resolve();
  });
}

function operationTestIds(container: HTMLElement): Set<string> {
  return new Set(
    Array.from(container.querySelectorAll<HTMLElement>("[data-testid]"))
      .map((element) => element.dataset.testid ?? "")
      .filter((testId) => OPERATION_TEST_IDS.includes(
        testId as (typeof OPERATION_TEST_IDS)[number],
      )),
  );
}

function expectSameViewerAllocation(
  fallback: DOMRect,
  viewer: DOMRect,
): void {
  expect(fallback.width).toBe(VIEWER_WIDTH_PX);
  expect(fallback.height).toBe(VIEWER_HEIGHT_PX);
  expect(viewer.width).toBe(VIEWER_WIDTH_PX);
  expect(viewer.height).toBe(VIEWER_HEIGHT_PX);
  expect(Math.abs(fallback.width - viewer.width)).toBeLessThanOrEqual(
    MAX_ALLOCATION_DELTA_PX,
  );
  expect(Math.abs(fallback.height - viewer.height)).toBeLessThanOrEqual(
    MAX_ALLOCATION_DELTA_PX,
  );
}

function ViewerCanvas() {
  return (
    <canvas
      className="viewer3d-canvas"
      data-testid="viewer3d-ready"
      tabIndex={0}
      aria-label="3D表示"
    />
  );
}

beforeEach(() => {
  nextScheduleId = 1;
  animationFrames = new Map();
  idleCallbacks = new Map();
  captureApi.install.mockClear();

  vi.stubGlobal(
    "requestAnimationFrame",
    vi.fn((callback: FrameRequestCallback) => {
      const id = nextScheduleId++;
      animationFrames.set(id, callback);
      return id;
    }),
  );
  vi.stubGlobal(
    "cancelAnimationFrame",
    vi.fn((id: number) => animationFrames.delete(id)),
  );
  vi.stubGlobal(
    "requestIdleCallback",
    vi.fn((callback: () => void) => {
      const id = nextScheduleId++;
      idleCallbacks.set(id, callback);
      return id;
    }),
  );
  vi.stubGlobal(
    "cancelIdleCallback",
    vi.fn((id: number) => idleCallbacks.delete(id)),
  );

  const getBoundingClientRect = HTMLElement.prototype.getBoundingClientRect;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
    function getAllocatedRect(this: HTMLElement) {
      if (this.classList.contains("viewer3d-canvas")) return viewerRect();
      return getBoundingClientRect.call(this);
    },
  );

  useAppStore.setState({
    ...initialDialogState,
    proposalStep: null,
    exportOpen: false,
    helpOpen: false,
    newDocument: vi.fn().mockResolvedValue(undefined),
    checkRecovery: vi.fn().mockResolvedValue(undefined),
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState({
    ...initialDialogState,
    newDocument: realNewDocument,
    checkRecovery: realCheckRecovery,
  });
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("Viewer/Three遅延境界", () => {
  it("fallbackと実Viewerの割当矩形差を縦横1px以下に保ち、4区画の操作要素を失わない", async () => {
    let resolveViewerModule: (module: ViewerModule) => void = () => {};
    const moduleLoader = vi.fn(
      () =>
        new Promise<ViewerModule>((resolve) => {
          resolveViewerModule = resolve;
        }),
    );
    const { container } = render(<App viewerModuleLoader={moduleLoader} />);

    const fallback = screen.getByTestId("viewer3d-loading");
    const fallbackRect = fallback.getBoundingClientRect();
    const allocationOwner = fallback.parentElement;
    expect(allocationOwner?.classList.contains("pane-3d-view")).toBe(true);
    const operationsBefore = operationTestIds(container);
    expect([...operationsBefore].sort()).toEqual([...OPERATION_TEST_IDS].sort());

    await requestDeferredViewer();
    expect(moduleLoader).toHaveBeenCalledTimes(1);
    await act(async () => {
      resolveViewerModule({ Viewer3D: ViewerCanvas });
      await Promise.resolve();
    });

    const viewer = await screen.findByTestId("viewer3d-ready");
    expect(viewer.parentElement).toBe(allocationOwner);
    expectSameViewerAllocation(fallbackRect, viewer.getBoundingClientRect());

    const operationsAfter = operationTestIds(container);
    const missingOperations = OPERATION_TEST_IDS.filter(
      (testId) => !operationsAfter.has(testId),
    );
    expect(operationsAfter.size).toBe(4);
    expect(missingOperations).toHaveLength(0);
    for (const testId of OPERATION_TEST_IDS) {
      const operation = screen.getByTestId(testId);
      expect(operation.tagName).toBe("BUTTON");
      expect((operation as HTMLButtonElement).disabled).toBe(false);
    }
  });

  it("1 sessionでViewer moduleを1回だけ評価し、unmount→remountごとに描画資源をdisposeする", async () => {
    const createDrawingResource = vi.fn();
    const disposeDrawingResource = vi.fn();
    function ResourceViewer() {
      useEffect(() => {
        createDrawingResource();
        return () => disposeDrawingResource();
      }, []);
      return (
        <canvas
          className="viewer3d-canvas"
          data-testid="resource-viewer"
          tabIndex={0}
          aria-label="3D表示"
        />
      );
    }
    const moduleLoader = vi.fn(async (): Promise<ViewerModule> => ({
      Viewer3D: ResourceViewer,
    }));

    const firstMount = render(<App viewerModuleLoader={moduleLoader} />);
    await requestDeferredViewer();
    await screen.findByTestId("resource-viewer");
    await waitFor(() => expect(createDrawingResource).toHaveBeenCalledTimes(1));
    firstMount.unmount();
    expect(disposeDrawingResource).toHaveBeenCalledTimes(1);

    const secondMount = render(<App viewerModuleLoader={moduleLoader} />);
    await requestDeferredViewer();
    await screen.findByTestId("resource-viewer");
    await waitFor(() => expect(createDrawingResource).toHaveBeenCalledTimes(2));
    secondMount.unmount();

    expect(moduleLoader).toHaveBeenCalledTimes(1);
    expect(disposeDrawingResource).toHaveBeenCalledTimes(2);
    expect(createDrawingResource.mock.calls.length).toBe(
      disposeDrawingResource.mock.calls.length,
    );
  });

  it("読込失敗時も同じ矩形で2D継続を知らせ、再試行後だけViewerをmountする", async () => {
    const createDrawingResource = vi.fn();
    let resolveRetryModule: (module: ViewerModule) => void = () => {};
    function RetryViewer() {
      useEffect(() => {
        createDrawingResource();
      }, []);
      return (
        <canvas
          className="viewer3d-canvas"
          data-testid="retry-viewer"
          tabIndex={0}
          aria-label="3D表示"
        />
      );
    }
    const moduleLoader = vi
      .fn<Viewer3DModuleLoader>()
      .mockRejectedValueOnce(new Error("viewer chunk failed"))
      .mockImplementationOnce(
        () =>
          new Promise<ViewerModule>((resolve) => {
            resolveRetryModule = resolve;
          }),
      );
    render(<App viewerModuleLoader={moduleLoader} />);

    const fallback = screen.getByTestId("viewer3d-loading");
    const fallbackRect = fallback.getBoundingClientRect();
    const allocationOwner = fallback.parentElement;
    expect(allocationOwner?.classList.contains("pane-3d-view")).toBe(true);
    await requestDeferredViewer();

    const errorFallback = await screen.findByRole("alert");
    expect(errorFallback.parentElement).toBe(allocationOwner);
    expectSameViewerAllocation(
      fallbackRect,
      errorFallback.getBoundingClientRect(),
    );
    expect(errorFallback.textContent).toContain(
      "3D表示を読み込めませんでした。2Dの編集は続けられます。",
    );
    expect(screen.queryByTestId("retry-viewer")).toBeNull();
    expect(
      (screen.getByTestId("cp-editor-operation") as HTMLButtonElement).disabled,
    ).toBe(false);
    expect(createDrawingResource).toHaveBeenCalledTimes(0);
    expect(moduleLoader).toHaveBeenCalledTimes(1);

    const retryButton = screen.getByRole("button", { name: "3D表示を再試行" });
    retryButton.focus();
    expect(document.activeElement).toBe(retryButton);
    fireEvent.click(retryButton);

    const retryLoading = screen.getByTestId("viewer3d-loading");
    expect(document.activeElement).toBe(retryLoading);
    await act(async () => {
      resolveRetryModule({ Viewer3D: RetryViewer });
      await Promise.resolve();
    });

    const viewer = await screen.findByTestId("retry-viewer");
    expect(viewer.parentElement).toBe(allocationOwner);
    expectSameViewerAllocation(fallbackRect, viewer.getBoundingClientRect());
    expect(document.activeElement).toBe(viewer);
    await waitFor(() => expect(createDrawingResource).toHaveBeenCalledTimes(1));
    expect(moduleLoader).toHaveBeenCalledTimes(2);
  });
});
