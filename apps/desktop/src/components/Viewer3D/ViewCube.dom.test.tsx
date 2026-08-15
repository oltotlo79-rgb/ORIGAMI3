// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import * as THREE from "three";
import { ViewCube, type ViewCubeCameraControl } from "./ViewCube.jsx";
import {
  VIEW_CUBE_TARGETS,
  cameraQuaternionLookingAt,
  directionAngleDeg,
  viewDirectionForTarget,
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

/** 同じ行き先は最大3枚の板に出るので、順路に載っている先頭の1つを押す。 */
function zoneFor(actionLabel: string): HTMLElement {
  return screen.getAllByRole("button", { name: actionLabel })[0];
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

  it("面6・辺12・角8の26箇所すべてを押せて、300ms後に期待方向へ着く", () => {
    const control = makeControl();
    render(
      <ViewCube
        getCamera={() => control.camera}
        prepareCameraControl={control.prepareCameraControl}
        requestRender={control.requestRender}
      />,
    );

    let worstAngleDeg = 0;
    VIEW_CUBE_TARGETS.forEach((target, index) => {
      const start = index * 1000;
      const zone = zoneFor(target.actionLabel);
      const group = screen.getByRole("group");
      fireEvent.pointerDown(zone, {
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
      worstAngleDeg = Math.max(
        worstAngleDeg,
        directionAngleDeg(cameraDirection(control.camera), viewDirectionForTarget(target.id)),
      );
    });
    expect(VIEW_CUBE_TARGETS).toHaveLength(26);
    // 受け入れ条件は0.5度未満。終端で期待姿勢へ直接合わせるため実測は0.000度で、
    // 余裕を確かめるために500倍厳しい値でも見ておく。
    expect(worstAngleDeg).toBeLessThan(0.5);
    expect(worstAngleDeg).toBeLessThan(1e-3);
    expect(control.prepareCameraControl).toHaveBeenCalledTimes(26);
  });

  it("移動は瞬間移動せず、0.3秒の途中を通る", () => {
    const control = makeControl();
    render(
      <ViewCube
        getCamera={() => control.camera}
        prepareCameraControl={control.prepareCameraControl}
        requestRender={control.requestRender}
      />,
    );
    const initial = control.camera.position.clone();
    fireEvent.click(zoneFor("右を正面にする"));
    expect(control.camera.position).toEqual(initial);

    runFrame(1000);
    expect(control.camera.position).toEqual(initial);
    runFrame(1150);
    expect(control.camera.position.distanceTo(initial)).toBeGreaterThan(0.1);
    expect(
      directionAngleDeg(cameraDirection(control.camera), viewDirectionForTarget("right")),
    ).toBeGreaterThan(0.5);
    runFrame(1300);
    expect(
      directionAngleDeg(cameraDirection(control.camera), viewDirectionForTarget("right")),
    ).toBeLessThan(0.5);
  });

  it("上から下へ移る間、画面の上下が一度も反転しない", () => {
    const control = makeControl();
    render(
      <ViewCube
        getCamera={() => control.camera}
        prepareCameraControl={control.prepareCameraControl}
        requestRender={control.requestRender}
      />,
    );
    fireEvent.click(zoneFor("上を正面にする"));
    runFrame(0);
    runFrame(300);
    fireEvent.click(zoneFor("下を正面にする"));

    const screenUp = () => new THREE.Vector3(0, 1, 0).applyQuaternion(control.camera.quaternion);
    let previous = screenUp();
    let flips = 0;
    for (let step = 0; step <= 60; step += 1) {
      runFrame(1000 + step * 5);
      const now = screenUp();
      if (previous.dot(now) < 0) flips += 1;
      previous = now;
    }
    expect(flips).toBe(0);
  });

  it("移動の途中からドラッグへ切り替えても上下が反転しない", () => {
    const control = makeControl();
    render(
      <ViewCube
        getCamera={() => control.camera}
        prepareCameraControl={control.prepareCameraControl}
        requestRender={control.requestRender}
      />,
    );
    fireEvent.click(zoneFor("上を正面にする"));
    runFrame(0);
    runFrame(300);
    fireEvent.click(zoneFor("下を正面にする"));
    runFrame(1000);
    runFrame(1150);

    const beforeUp = new THREE.Vector3(0, 1, 0).applyQuaternion(control.camera.quaternion);
    const zone = zoneFor("前を正面にする");
    const group = screen.getByRole("group");
    fireEvent.pointerDown(zone, {
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
  ])("$name方向のドラッグで視点が回り、行き先の選択にはならない", ({ dx, dy }) => {
    const control = makeControl();
    render(
      <ViewCube
        getCamera={() => control.camera}
        prepareCameraControl={control.prepareCameraControl}
        requestRender={control.requestRender}
      />,
    );
    const zone = zoneFor("前を正面にする");
    const group = screen.getByRole("group");
    const before = cameraDirection(control.camera);
    fireEvent.pointerDown(zone, {
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
    const group = screen.getByRole("group");
    for (const label of [
      "前を正面にする",
      "上と前の間の辺を正面にする",
      "上と前と右が集まる角を正面にする",
    ]) {
      const zone = zoneFor(label);
      fireEvent.pointerDown(zone, { button: 0, pointerId: 1, clientX: 20, clientY: 20 });
      fireEvent.pointerUp(group, { button: 0, pointerId: 1, clientX: 20, clientY: 20 });
      fireEvent.click(zone, { detail: 1 });
      fireEvent.pointerDown(zone, { button: 0, pointerId: 2, clientX: 20, clientY: 20 });
      fireEvent.pointerMove(group, { pointerId: 2, clientX: 80, clientY: 20 });
      fireEvent.pointerUp(group, { button: 0, pointerId: 2, clientX: 80, clientY: 20 });
    }

    expect(paperSelect).toHaveBeenCalledTimes(0);
    expect(parentMove).toHaveBeenCalledTimes(0);
    expect(paperFold).toHaveBeenCalledTimes(0);
    expect(parentClick).toHaveBeenCalledTimes(0);
  });

  it("押す前に、指している場所が面・辺・角のどれかまで分かる印が付く", () => {
    const control = makeControl();
    render(
      <ViewCube
        getCamera={() => control.camera}
        prepareCameraControl={control.prepareCameraControl}
        requestRender={control.requestRender}
      />,
    );
    const group = screen.getByRole("group");
    expect(document.querySelectorAll("[data-pointed='true']")).toHaveLength(0);

    const corner = zoneFor("上と前と右が集まる角を正面にする");
    fireEvent.pointerMove(corner, { pointerId: 11, clientX: 20, clientY: 20 });
    // 角は3枚の板に出るため、同じ角を指す区画がまとめて光る。
    expect(document.querySelectorAll("[data-pointed='true']")).toHaveLength(3);
    expect(corner.dataset.viewCubeKind).toBe("corner");

    const edge = zoneFor("上と前の間の辺を正面にする");
    fireEvent.pointerMove(edge, { pointerId: 11, clientX: 21, clientY: 20 });
    expect(document.querySelectorAll("[data-pointed='true']")).toHaveLength(2);
    expect(edge.dataset.viewCubeKind).toBe("edge");

    const face = zoneFor("前を正面にする");
    fireEvent.pointerMove(face, { pointerId: 11, clientX: 22, clientY: 20 });
    expect(document.querySelectorAll("[data-pointed='true']")).toHaveLength(1);
    expect(face.dataset.viewCubeKind).toBe("face");

    // 押し場所から外れると印は消える。
    fireEvent.pointerMove(group, { pointerId: 11, clientX: 60, clientY: 60 });
    expect(document.querySelectorAll("[data-pointed='true']")).toHaveLength(0);
  });

  it("面・辺・角の押し場所が6面×9区画あり、26箇所すべてを順路でたどれる", () => {
    const control = makeControl();
    render(
      <ViewCube
        getCamera={() => control.camera}
        prepareCameraControl={control.prepareCameraControl}
        requestRender={control.requestRender}
      />,
    );
    const zones = screen.getAllByRole("button");
    expect(zones).toHaveLength(54);
    const reachable = zones.filter((zone) => zone.tabIndex === 0);
    expect(reachable).toHaveLength(26);
    expect(new Set(reachable.map((zone) => zone.dataset.viewCubeTarget)).size).toBe(26);
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
