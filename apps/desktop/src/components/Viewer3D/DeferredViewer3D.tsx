import {
  type ComponentType,
  type CSSProperties,
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
  type MutableRefObject,
  type ReactNode,
  type RefObject,
} from "react";
import { waitForViewer3DReady } from "../../captureReadbackBridge";

interface Viewer3DProps {
  fitRef: RefObject<(() => void) | null>;
  statusOverlays?: ReactNode;
}

export interface Viewer3DModule {
  Viewer3D: ComponentType<Viewer3DProps>;
}

export type Viewer3DModuleLoader = () => Promise<Viewer3DModule>;

const defaultViewer3DModuleLoader: Viewer3DModuleLoader = () => import("./Viewer3D");
const viewer3DModulePromises = new WeakMap<
  Viewer3DModuleLoader,
  Promise<Viewer3DModule>
>();

/** 同じsession・loaderでは、remountや複数の準備要求が重なってもmodule評価を1回にする。 */
function loadViewer3DModule(
  moduleLoader: Viewer3DModuleLoader,
): Promise<Viewer3DModule> {
  const existing = viewer3DModulePromises.get(moduleLoader);
  if (existing !== undefined) return existing;
  const loading = moduleLoader();
  viewer3DModulePromises.set(moduleLoader, loading);
  return loading;
}

type IdleWindow = Window & {
  requestIdleCallback?: (
    callback: () => void,
    options?: { timeout: number },
  ) => number;
  cancelIdleCallback?: (handle: number) => void;
};

export interface DeferredViewer3DHandle {
  ensureReady(): Promise<void>;
}

interface Props extends Viewer3DProps {
  /** 製品では省略する。境界の失敗・評価回数をThreeなしで検査するための注入口。 */
  moduleLoader?: Viewer3DModuleLoader;
}

interface FocusHandoffProps {
  focusHandoffRef: MutableRefObject<boolean>;
}

const FALLBACK_STYLE: CSSProperties = {
  display: "grid",
  placeItems: "center",
  background: "var(--color-canvas-3d)",
  color: "var(--color-text-muted)",
};

function useFallbackFocus(
  fallbackRef: RefObject<HTMLDivElement | null>,
  focusHandoffRef: MutableRefObject<boolean>,
): void {
  useLayoutEffect(() => {
    const fallback = fallbackRef.current;
    if (focusHandoffRef.current && fallback !== null) {
      fallback.focus();
      focusHandoffRef.current = false;
    }
    return () => {
      if (fallback?.contains(document.activeElement)) {
        focusHandoffRef.current = true;
      }
    };
  }, [fallbackRef, focusHandoffRef]);
}

function Viewer3DLoading({ focusHandoffRef }: FocusHandoffProps) {
  const loadingRef = useRef<HTMLDivElement | null>(null);
  useFallbackFocus(loadingRef, focusHandoffRef);

  return (
    <div
      ref={loadingRef}
      className="viewer3d-canvas"
      data-testid="viewer3d-loading"
      tabIndex={0}
      aria-label="3D表示を準備しています"
      aria-live="polite"
      aria-busy="true"
      style={FALLBACK_STYLE}
    >
      3D表示を準備しています…
    </div>
  );
}

function Viewer3DLoadError({
  focusHandoffRef,
  onRetry,
}: FocusHandoffProps & { onRetry: () => void }) {
  const errorRef = useRef<HTMLDivElement | null>(null);
  useFallbackFocus(errorRef, focusHandoffRef);

  return (
    <div
      ref={errorRef}
      className="viewer3d-canvas"
      data-testid="viewer3d-load-error"
      role="alert"
      tabIndex={0}
      aria-label="3D表示を読み込めませんでした"
      style={FALLBACK_STYLE}
    >
      <div style={{ display: "grid", justifyItems: "center", gap: "var(--sp-3)" }}>
        <span>3D表示を読み込めませんでした。2Dの編集は続けられます。</span>
        <button type="button" onClick={onRetry}>
          3D表示を再試行
        </button>
      </div>
    </div>
  );
}

function Viewer3DReady({
  fitRef,
  statusOverlays,
  focusHandoffRef,
  Viewer,
}: Viewer3DProps & FocusHandoffProps & { Viewer: ComponentType<Viewer3DProps> }) {
  useLayoutEffect(() => {
    if (!focusHandoffRef.current) return;
    const canvas = document.querySelector<HTMLCanvasElement>(
      ".pane-3d-view canvas.viewer3d-canvas",
    );
    canvas?.focus();
    focusHandoffRef.current = false;
  }, [focusHandoffRef]);

  return <Viewer fitRef={fitRef} statusOverlays={statusOverlays} />;
}

type Viewer3DLoadState =
  | { status: "loading" }
  | { status: "ready"; module: Viewer3DModule }
  | { status: "failed" };

/** 2Dの最初の描画を先に通し、ブラウザーが空いてから3D本体を読む軽量な外枠。 */
export const DeferredViewer3D = forwardRef<DeferredViewer3DHandle, Props>(
  function DeferredViewer3D(
    {
      fitRef,
      statusOverlays,
      moduleLoader = defaultViewer3DModuleLoader,
    },
    ref,
  ) {
    const [requested, setRequested] = useState(false);
    const [loadAttempt, setLoadAttempt] = useState(0);
    const [loadState, setLoadState] = useState<Viewer3DLoadState>({
      status: "loading",
    });
    const focusHandoffRef = useRef(false);
    const ensureReady = useCallback(async () => {
      setRequested(true);
      await loadViewer3DModule(moduleLoader);
      await waitForViewer3DReady();
    }, [moduleLoader]);

    const retry = useCallback(() => {
      // rejected Promiseだけを捨てる。正常remountは同じPromiseを使いmodule評価を増やさない。
      viewer3DModulePromises.delete(moduleLoader);
      setLoadState({ status: "loading" });
      setLoadAttempt((attempt) => attempt + 1);
    }, [moduleLoader]);

    useImperativeHandle(ref, () => ({ ensureReady }), [ensureReady]);

    useEffect(() => {
      if (!requested) return;
      let mounted = true;
      void loadViewer3DModule(moduleLoader).then(
        (module) => {
          if (mounted) setLoadState({ status: "ready", module });
        },
        () => {
          if (mounted) setLoadState({ status: "failed" });
        },
      );
      return () => {
        mounted = false;
      };
    }, [loadAttempt, moduleLoader, requested]);

    // 計測や永続状態へは混ぜず、この表示だけの一時状態として扱う。
    useEffect(() => {
      let cancelled = false;
      let secondFrame = 0;
      let idleHandle: number | null = null;
      let timeoutHandle: number | null = null;
      const idleWindow = window as IdleWindow;
      const requestViewer = () => {
        if (!cancelled) setRequested(true);
      };
      const firstFrame = window.requestAnimationFrame(() => {
        secondFrame = window.requestAnimationFrame(() => {
          if (idleWindow.requestIdleCallback) {
            idleHandle = idleWindow.requestIdleCallback(requestViewer, { timeout: 1_000 });
          } else {
            timeoutHandle = window.setTimeout(requestViewer, 0);
          }
        });
      });
      return () => {
        cancelled = true;
        window.cancelAnimationFrame(firstFrame);
        if (secondFrame !== 0) window.cancelAnimationFrame(secondFrame);
        if (idleHandle !== null) idleWindow.cancelIdleCallback?.(idleHandle);
        if (timeoutHandle !== null) window.clearTimeout(timeoutHandle);
      };
    }, []);

    if (!requested || loadState.status === "loading") {
      return <Viewer3DLoading focusHandoffRef={focusHandoffRef} />;
    }
    if (loadState.status === "failed") {
      return (
        <Viewer3DLoadError
          focusHandoffRef={focusHandoffRef}
          onRetry={retry}
        />
      );
    }
    return (
      <Viewer3DReady
        fitRef={fitRef}
        statusOverlays={statusOverlays}
        focusHandoffRef={focusHandoffRef}
        Viewer={loadState.module.Viewer3D}
      />
    );
  },
);
