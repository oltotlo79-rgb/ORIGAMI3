import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  type WheelEvent as ReactWheelEvent,
} from "react";
import * as THREE from "three";
import {
  VIEW_CUBE_CLICK_MOVE_PX,
  VIEW_CUBE_PLATES,
  VIEW_CUBE_TRANSITION_MS,
  VIEW_CUBE_ZONE_COLUMNS,
  VIEW_CUBE_ZONE_ROWS,
  cameraPositionForTarget,
  cameraQuaternionLookingAt,
  cameraUpForTarget,
  cubeCssTransform,
  interpolateCameraPose,
  isViewCubeTarget,
  orbitCameraOffset,
  smoothViewProgress,
  transportCameraScreenUp,
  viewCubeTarget,
  viewCubeZoneKind,
  viewCubeZoneTarget,
  type ViewCubeTarget,
} from "./viewCube";

export interface ViewCubeCameraControl {
  camera: THREE.PerspectiveCamera;
  target: THREE.Vector3;
  /** OrbitControlsが回転量の基準にしている3D canvasの高さ。 */
  canvasHeight: number;
}

interface Props {
  /** 立方体の姿勢を現在の視点へ追従させるための読み取り口。 */
  getCamera(): THREE.PerspectiveCamera | null;
  /** 操作開始時のカメラ・注視点・3D表示高さ。 */
  prepareCameraControl(): ViewCubeCameraControl | null;
  /** WebGL側は常時描画しないため、カメラを動かしたフレームだけ描画を頼む。 */
  requestRender(): void;
}

interface DragState {
  pointerId: number;
  startX: number;
  startY: number;
  target: ViewCubeTarget | null;
  moved: boolean;
  control: ViewCubeCameraControl;
  initialOffset: THREE.Vector3;
  currentOffset: THREE.Vector3;
  screenUp: THREE.Vector3;
}

interface ZoneDescriptor {
  key: string;
  plateId: string;
  target: ViewCubeTarget;
  kind: "face" | "edge" | "corner";
  label: string;
  actionLabel: string;
  /** 同じ行き先を指す区画は立方体上に最大3つ出るため、順路は先頭の1つだけにする。 */
  focusable: boolean;
}

/** 6枚の板を3×3へ割り、面6・辺12・角8の押し場所を作る。 */
function buildZones(): readonly { plateId: string; zones: readonly ZoneDescriptor[] }[] {
  const focusableSeen = new Set<ViewCubeTarget>();
  return VIEW_CUBE_PLATES.map((plate) => ({
    plateId: plate.id,
    zones: VIEW_CUBE_ZONE_ROWS.flatMap((row) =>
      VIEW_CUBE_ZONE_COLUMNS.map((column) => {
        const target = viewCubeZoneTarget(plate, column, row);
        const definition = viewCubeTarget(target);
        const focusable = !focusableSeen.has(target);
        focusableSeen.add(target);
        return {
          key: `${plate.id}:${row}:${column}`,
          plateId: plate.id,
          target,
          kind: viewCubeZoneKind(column, row),
          label: definition.label,
          actionLabel: definition.actionLabel,
          focusable,
        };
      }),
    ),
  }));
}

const ZONE_LAYOUT = buildZones();

function targetFromEventTarget(target: EventTarget | null): ViewCubeTarget | null {
  if (!(target instanceof Element)) return null;
  const value = target.closest<HTMLElement>("[data-view-cube-target]")?.dataset
    .viewCubeTarget;
  return isViewCubeTarget(value) ? value : null;
}

/** 3D表示へ重ねる、現在の視点と一緒に回る立方体。面・辺・角の26箇所を選べる。 */
export function ViewCube({ getCamera, prepareCameraControl, requestRender }: Props) {
  const rootRef = useRef<HTMLDivElement | null>(null);
  const bodyRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<DragState | null>(null);
  const transitionFrameRef = useRef<number | null>(null);
  // 押す前にどこを指しているかを示すだけの一時状態(表示専用)。
  const [pointed, setPointed] = useState<ViewCubeTarget | null>(null);

  const pointAt = useCallback((next: ViewCubeTarget | null) => {
    setPointed((previous) => (previous === next ? previous : next));
  }, []);

  const cancelTransition = useCallback(() => {
    if (transitionFrameRef.current !== null) {
      cancelAnimationFrame(transitionFrameRef.current);
      transitionFrameRef.current = null;
    }
  }, []);

  const moveToTarget = useCallback(
    (id: ViewCubeTarget, prepared?: ViewCubeCameraControl) => {
      cancelTransition();
      const control = prepared ?? prepareCameraControl();
      if (!control) return;
      const { camera, target } = control;
      const fromOffset = camera.position.clone().sub(target);
      const distance = fromOffset.length();
      if (distance <= 1e-9) return;

      const fromPosition = camera.position.clone();
      const fromQuaternion = camera.quaternion.clone();
      const fromScreenUp = new THREE.Vector3(0, 1, 0).applyQuaternion(fromQuaternion);
      const toPosition = cameraPositionForTarget(id, target, distance);
      const toOffset = toPosition.clone().sub(target);
      const toScreenUp = cameraUpForTarget(id);
      const toQuaternion = cameraQuaternionLookingAt(toPosition, target, toScreenUp);
      let startedAt: number | null = null;

      const animate = (now: number) => {
        if (startedAt === null) startedAt = now;
        const linear = Math.min((now - startedAt) / VIEW_CUBE_TRANSITION_MS, 1);
        const progress = smoothViewProgress(linear);
        const pose = interpolateCameraPose(
          fromOffset,
          toOffset,
          fromScreenUp,
          toScreenUp,
          progress,
        );
        camera.position.copy(target).add(pose.offset);
        camera.quaternion.copy(pose.attitude);
        camera.updateMatrixWorld(true);
        requestRender();

        if (linear < 1) {
          transitionFrameRef.current = requestAnimationFrame(animate);
        } else {
          // 補間の丸めを終端へ残さず、26箇所の期待方向へ厳密に合わせる。
          camera.position.copy(toPosition);
          camera.quaternion.copy(toQuaternion);
          camera.updateMatrixWorld(true);
          requestRender();
          transitionFrameRef.current = null;
        }
      };

      // 初回も次フレームに置き、クリック直後の瞬間移動を起こさない。
      camera.position.copy(fromPosition);
      camera.quaternion.copy(fromQuaternion);
      transitionFrameRef.current = requestAnimationFrame(animate);
    },
    [cancelTransition, prepareCameraControl, requestRender],
  );

  useEffect(() => {
    let orientationFrame = 0;
    const previous = new THREE.Quaternion(Number.NaN, Number.NaN, Number.NaN, Number.NaN);
    const followCamera = () => {
      const camera = getCamera();
      const body = bodyRef.current;
      if (camera && body && !previous.equals(camera.quaternion)) {
        previous.copy(camera.quaternion);
        body.style.transform = cubeCssTransform(camera.quaternion);
      }
      orientationFrame = requestAnimationFrame(followCamera);
    };
    orientationFrame = requestAnimationFrame(followCamera);
    return () => cancelAnimationFrame(orientationFrame);
  }, [getCamera]);

  useEffect(() => {
    /** canvas側の新しい操作・ホイール・ボタン操作を優先し、途中の補間を止める。 */
    const cancelFromOutside = (event: Event) => {
      const root = rootRef.current;
      if (root && event.target instanceof Node && root.contains(event.target)) return;
      cancelTransition();
    };
    window.addEventListener("pointerdown", cancelFromOutside, true);
    window.addEventListener("wheel", cancelFromOutside, true);
    return () => {
      window.removeEventListener("pointerdown", cancelFromOutside, true);
      window.removeEventListener("wheel", cancelFromOutside, true);
      cancelTransition();
    };
  }, [cancelTransition]);

  const stopPointerEvent = (event: ReactPointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.stopPropagation();
  };

  const handlePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    stopPointerEvent(event);
    if (event.button !== 0) return;
    const control = prepareCameraControl();
    if (!control) return;
    cancelTransition();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    const initialOffset = control.camera.position.clone().sub(control.target);
    dragRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      target: targetFromEventTarget(event.target),
      moved: false,
      control,
      initialOffset,
      currentOffset: initialOffset.clone(),
      screenUp: new THREE.Vector3(0, 1, 0).applyQuaternion(control.camera.quaternion),
    };
  };

  const handlePointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    stopPointerEvent(event);
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) {
      pointAt(targetFromEventTarget(event.target));
      return;
    }
    const dx = event.clientX - drag.startX;
    const dy = event.clientY - drag.startY;
    if (!drag.moved && Math.hypot(dx, dy) <= VIEW_CUBE_CLICK_MOVE_PX) return;
    drag.moved = true;
    pointAt(null);
    event.currentTarget.dataset.dragging = "true";
    const offset = orbitCameraOffset(
      drag.initialOffset,
      dx,
      dy,
      drag.control.canvasHeight,
    );
    const { camera, target } = drag.control;
    const screenUp = transportCameraScreenUp(
      drag.currentOffset,
      offset,
      drag.screenUp,
    );
    drag.currentOffset.copy(offset);
    drag.screenUp.copy(screenUp);
    camera.position.copy(target).add(offset);
    camera.quaternion.copy(cameraQuaternionLookingAt(camera.position, target, screenUp));
    camera.updateMatrixWorld(true);
    requestRender();
  };

  const finishPointer = (event: ReactPointerEvent<HTMLDivElement>, cancelled: boolean) => {
    stopPointerEvent(event);
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    dragRef.current = null;
    delete event.currentTarget.dataset.dragging;
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    if (!cancelled && !drag.moved && drag.target) moveToTarget(drag.target, drag.control);
  };

  const handleClick = (event: ReactMouseEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.stopPropagation();
    // マウス/タッチはpointerupで処理済み。detail=0のキーボード操作だけ補う。
    if (event.detail !== 0) return;
    const target = targetFromEventTarget(event.target);
    if (target) moveToTarget(target);
  };

  const stopWheel = (event: ReactWheelEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.stopPropagation();
  };

  return (
    <div
      ref={rootRef}
      className="view-cube"
      role="group"
      aria-label="視点を変える立方体。面・辺・角を選ぶか、ドラッグして回します"
      data-floating-ui="view-cube"
      data-tooltip="面・辺・角を選ぶとその向きへ移動します。ドラッグでも視点を回せます"
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={(event) => finishPointer(event, false)}
      onPointerCancel={(event) => finishPointer(event, true)}
      onPointerLeave={() => pointAt(null)}
      onFocusCapture={(event) => pointAt(targetFromEventTarget(event.target))}
      onBlurCapture={() => pointAt(null)}
      onClick={handleClick}
      onWheel={stopWheel}
      onContextMenu={(event) => {
        event.preventDefault();
        event.stopPropagation();
      }}
    >
      <div ref={bodyRef} className="view-cube-body">
        {ZONE_LAYOUT.map((plate) => (
          <div
            className={`view-cube-face view-cube-face-${plate.plateId}`}
            key={plate.plateId}
          >
            {plate.zones.map((zone) => (
              <button
                type="button"
                className="view-cube-zone"
                data-view-cube-target={zone.target}
                data-view-cube-kind={zone.kind}
                data-pointed={pointed === zone.target ? "true" : undefined}
                aria-label={zone.actionLabel}
                key={zone.key}
                tabIndex={zone.focusable ? 0 : -1}
              >
                {zone.kind === "face" ? zone.label : null}
              </button>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}
