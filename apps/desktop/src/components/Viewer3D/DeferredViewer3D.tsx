import {
  forwardRef,
  lazy,
  Suspense,
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

const loadViewer3DModule = () => import("./Viewer3D");
const LazyViewer3D = lazy(async () => ({
  default: (await loadViewer3DModule()).Viewer3D,
}));

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

interface Props {
  fitRef: RefObject<(() => void) | null>;
  statusOverlays?: ReactNode;
}

interface FocusHandoffProps {
  focusHandoffRef: MutableRefObject<boolean>;
}

function Viewer3DLoading({ focusHandoffRef }: FocusHandoffProps) {
  const loadingRef = useRef<HTMLDivElement | null>(null);

  useLayoutEffect(() => {
    const loading = loadingRef.current;
    if (focusHandoffRef.current && loading !== null) {
      loading.focus();
      focusHandoffRef.current = false;
    }
    return () => {
      if (document.activeElement === loading) focusHandoffRef.current = true;
    };
  }, [focusHandoffRef]);

  return (
    <div
      ref={loadingRef}
      className="viewer3d-canvas"
      data-testid="viewer3d-loading"
      tabIndex={0}
      aria-label="3D表示を準備しています"
      aria-live="polite"
      aria-busy="true"
      style={{
        display: "grid",
        placeItems: "center",
        background: "var(--color-canvas-3d)",
        color: "var(--color-text-muted)",
      }}
    >
      3D表示を準備しています…
    </div>
  );
}

function Viewer3DReady({
  fitRef,
  statusOverlays,
  focusHandoffRef,
}: Props & FocusHandoffProps) {
  useLayoutEffect(() => {
    if (!focusHandoffRef.current) return;
    const canvas = document.querySelector<HTMLCanvasElement>(
      ".pane-3d-view canvas.viewer3d-canvas",
    );
    canvas?.focus();
    focusHandoffRef.current = false;
  }, [focusHandoffRef]);

  return <LazyViewer3D fitRef={fitRef} statusOverlays={statusOverlays} />;
}

/** 2Dの最初の描画を先に通し、ブラウザーが空いてから3D本体を読む軽量な外枠。 */
export const DeferredViewer3D = forwardRef<DeferredViewer3DHandle, Props>(
  function DeferredViewer3D({ fitRef, statusOverlays }, ref) {
    const [requested, setRequested] = useState(false);
    const focusHandoffRef = useRef(false);
    const ensureReady = useCallback(async () => {
      setRequested(true);
      await loadViewer3DModule();
      await waitForViewer3DReady();
    }, []);

    useImperativeHandle(ref, () => ({ ensureReady }), [ensureReady]);

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

    if (!requested) return <Viewer3DLoading focusHandoffRef={focusHandoffRef} />;
    return (
      <Suspense fallback={<Viewer3DLoading focusHandoffRef={focusHandoffRef} />}>
        <Viewer3DReady
          fitRef={fitRef}
          statusOverlays={statusOverlays}
          focusHandoffRef={focusHandoffRef}
        />
      </Suspense>
    );
  },
);
