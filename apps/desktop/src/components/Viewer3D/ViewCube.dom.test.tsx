// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import * as THREE from "three";
import { ViewCube, type ViewCubeCameraControl } from "./ViewCube.jsx";
import {
  VIEW_CUBE_FACES,
  cameraQuaternionLookingAt,
  directionAngleDeg,
  viewDirectionForFace,
} from "./viewCube";

let nextFrameId = 1;
let frames = new Map<number, FrameRequestCallback>();

function runFrame(now: number) {
  const current = [...frames.values()];
  frames.clear();
  for (const callback of current) callback(now);
}

function cameraDirection(camera: THREE.PerspectiveCamera): THREE.Vector3 {
  return camera.getWorldDirection(new THREE.Vector3());
}

function makeControl() {
  const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
  const target = new THREE.Vector3(0, 0, 0);
  camera.position.set(0, 0, 5);
  camera.quaternion.copy(cameraQuaternionLookingAt(camera.position, target, camera.up));
  camera.updateMatrixWorld(true);
  const control: ViewCubeCameraControl = { camera, target, canvasHeight: 400 };
  return {
    camera,
    prepareCameraControl: vi.fn(() => control),
    requestRender: vi.fn(),
  };
}

describe("ViewCube(画面)", () => {
  beforeEach(() => {
    nextFrameId = 1;
    frames = new Map();
    vi.stubGlobal(
      "requestAnimationFrame",
      vi.fn((callback: FrameRequestCallback) => {
        const id = nextFrameId++;
        frames.set(id, callback);
        return id;
      }),
    );
    vi.stubGlobal(
      "cancelAnimationFrame",
      vi.fn((id: number) => {
        frames.delete(id);
      }),
    );
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it("前後左右上下の6面すべてを押せて、300ms後に期待方向へ着く", () => {
    const control = makeControl();
    render(
      <ViewCube
        getCamera={() => control.camera}
        prepareCameraControl={control.prepareCameraControl}
        requestRender={control.requestRender}
      />,
    );

    VIEW_CUBE_FACES.forEach((face, index) => {
      const start = index * 1000;
      const button = screen.getByRole("button", { name: `${face.label}を正面にする` });
      const group = screen.getByRole("group");
      fireEvent.pointerDown(button, {
        button: 0,
        pointerId: index + 1,
        clientX: 20,
        clientY: 20,
      });
      fireEvent.pointerUp(group, {
        button: 0,
        pointerId: index + 1,
        clientX: 20,
        clientY: 20,
      });
      runFrame(start);
      runFrame(start + 300);
      expect(
        directionAngleDeg(cameraDirection(control.camera), viewDirectionForFace(face.id)),
      ).toBeLessThan(0.5);
    });
    expect(control.prepareCameraControl).toHaveBeenCalledTimes(6);
  });

  it("面クリックの移動は瞬間移動せず、0.3秒の途中を通る", () => {
    const control = makeControl();
    render(
      <ViewCube
        getCamera={() => control.camera}
        prepareCameraControl={control.prepareCameraControl}
        requestRender={control.requestRender}
      />,
    );
    const initial = control.camera.position.clone();
    fireEvent.click(screen.getByRole("button", { name: "右を正面にする" }));
    expect(control.camera.position).toEqual(initial);

    runFrame(1000);
    expect(control.camera.position).toEqual(initial);
    runFrame(1150);
    expect(control.camera.position.distanceTo(initial)).toBeGreaterThan(0.1);
    expect(
      directionAngleDeg(cameraDirection(control.camera), viewDirectionForFace("right")),
    ).toBeGreaterThan(0.5);
    runFrame(1300);
    expect(
      directionAngleDeg(cameraDirection(control.camera), viewDirectionForFace("right")),
    ).toBeLessThan(0.5);
  });

  it("面移動の途中からドラッグへ切り替えても上下が反転しない", () => {
    const control = makeControl();
    render(
      <ViewCube
        getCamera={() => control.camera}
        prepareCameraControl={control.prepareCameraControl}
        requestRender={control.requestRender}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "上を正面にする" }));
    runFrame(0);
    runFrame(300);
    fireEvent.click(screen.getByRole("button", { name: "下を正面にする" }));
    runFrame(1000);
    runFrame(1150);

    const beforeUp = new THREE.Vector3(0, 1, 0).applyQuaternion(control.camera.quaternion);
    const face = screen.getByRole("button", { name: "前を正面にする" });
    const group = screen.getByRole("group");
    fireEvent.pointerDown(face, {
      button: 0,
      pointerId: 8,
      clientX: 20,
      clientY: 20,
    });
    fireEvent.pointerMove(group, { pointerId: 8, clientX: 25, clientY: 20 });
    const afterUp = new THREE.Vector3(0, 1, 0).applyQuaternion(control.camera.quaternion);
    expect(directionAngleDeg(beforeUp, afterUp)).toBeLessThan(5);
  });

  it.each([
    { name: "横", dx: 60, dy: 0 },
    { name: "縦", dx: 0, dy: 60 },
  ])("$name方向のドラッグで視点が回り、面クリックにはならない", ({ dx, dy }) => {
    const control = makeControl();
    render(
      <ViewCube
        getCamera={() => control.camera}
        prepareCameraControl={control.prepareCameraControl}
        requestRender={control.requestRender}
      />,
    );
    const face = screen.getByRole("button", { name: "前を正面にする" });
    const group = screen.getByRole("group");
    const before = cameraDirection(control.camera);
    fireEvent.pointerDown(face, {
      button: 0,
      pointerId: 4,
      clientX: 20,
      clientY: 20,
    });
    fireEvent.pointerMove(group, {
      pointerId: 4,
      clientX: 20 + dx,
      clientY: 20 + dy,
    });
    fireEvent.pointerUp(group, {
      button: 0,
      pointerId: 4,
      clientX: 20 + dx,
      clientY: 20 + dy,
    });

    expect(directionAngleDeg(before, cameraDirection(control.camera))).toBeGreaterThan(1);
    expect(control.prepareCameraControl).toHaveBeenCalledTimes(1);
    expect(control.requestRender).toHaveBeenCalled();
  });

  it("立方体のクリックとドラッグを、紙の選択・折りへ1回も渡さない", () => {
    const control = makeControl();
    const paperSelect = vi.fn();
    const paperFold = vi.fn();
    const parentMove = vi.fn();
    const parentClick = vi.fn();
    render(
      <div
        onPointerDown={paperSelect}
        onPointerMove={parentMove}
        onPointerUp={paperFold}
        onClick={parentClick}
      >
        <ViewCube
          getCamera={() => control.camera}
          prepareCameraControl={control.prepareCameraControl}
          requestRender={control.requestRender}
        />
      </div>,
    );
    const face = screen.getByRole("button", { name: "前を正面にする" });
    const group = screen.getByRole("group");

    fireEvent.pointerDown(face, { button: 0, pointerId: 1, clientX: 20, clientY: 20 });
    fireEvent.pointerUp(group, { button: 0, pointerId: 1, clientX: 20, clientY: 20 });
    fireEvent.click(face, { detail: 1 });
    fireEvent.pointerDown(face, { button: 0, pointerId: 2, clientX: 20, clientY: 20 });
    fireEvent.pointerMove(group, { pointerId: 2, clientX: 80, clientY: 20 });
    fireEvent.pointerUp(group, { button: 0, pointerId: 2, clientX: 80, clientY: 20 });

    expect(paperSelect).toHaveBeenCalledTimes(0);
    expect(parentMove).toHaveBeenCalledTimes(0);
    expect(paperFold).toHaveBeenCalledTimes(0);
    expect(parentClick).toHaveBeenCalledTimes(0);
  });

  it("画面に出る文言の内部用語は0件", () => {
    const control = makeControl();
    render(
      <ViewCube
        getCamera={() => control.camera}
        prepareCameraControl={control.prepareCameraControl}
        requestRender={control.requestRender}
      />,
    );
    const group = screen.getByRole("group");
    const userCopy = [
      group.textContent,
      group.getAttribute("aria-label"),
      group.getAttribute("data-tooltip"),
      ...screen
        .getAllByRole("button")
        .map((button) => button.getAttribute("aria-label")),
    ].join(" ");
    expect(userCopy.match(/(?:[+-][XYZ]|[XYZ]軸|座標|camera|OrbitControls|solver|facet|hinge)/gi) ?? []).toHaveLength(0);
  });
});
