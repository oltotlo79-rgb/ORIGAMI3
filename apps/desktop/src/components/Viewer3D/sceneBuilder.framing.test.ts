// 紙が3D区画へ収まるかを、四隅の投影の位置で測る検査。
//
// 直す前は縦の画角だけで距離を決めていたため、区画の縦横比を無視していた。
// 実測(区画200×600 CSS px): 紙が左へ136px・右へ189pxはみ出していた。
// ここでは「はみ出し0画素」を、窓の大きさ・区画の大きさを変えながら数で確かめる。

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import {
  CAMERA_DIR,
  legacyPaperDistance,
  paperFraming,
  type PaperFraming,
  type ScreenBounds,
} from "./sceneBuilder";

/** 「視点を戻す」直後の向きから見た、画面の右向きと上向き。 */
const UP = new THREE.Vector3(0, 1, 0);
const RIGHT = new THREE.Vector3().crossVectors(UP, CAMERA_DIR).normalize();
const SCREEN_UP = new THREE.Vector3().crossVectors(CAMERA_DIR, RIGHT).normalize();

/** 実測した3D区画の大きさ(CSS px)。窓の大きさ→区画の大きさは実機で測った値。 */
const PANES = [
  { window: "1280x860", width: 605, height: 261 },
  { window: "1000x700", width: 465, height: 191 },
  { window: "1920x1080", width: 925, height: 360 },
  { window: "1936x1048", width: 933, height: 346 },
  // 説明書の画像を撮ったときの区画(コンテキストパネルが今より低かった)
  { window: "1280x860(説明書撮影時)", width: 605, height: 439 },
] as const;

/** 実測した案内の札の下端(canvasの上から107 CSS px)。 */
const HINT_BOTTOM = 107;

function framingOf(width: number, height: number, hintBottom = HINT_BOTTOM) {
  return paperFraming(1, 1, width, height, hintBottom, CAMERA_DIR, RIGHT, SCREEN_UP);
}

/** 四角形が区画からはみ出した画素数(4辺の合計ではなく、辺ごとに返す)。 */
function overflow(bounds: ScreenBounds, width: number, height: number) {
  return {
    left: Math.max(0, -bounds.left),
    right: Math.max(0, bounds.right - width),
    top: Math.max(0, -bounds.top),
    bottom: Math.max(0, bounds.bottom - height),
  };
}

/** 紙の外接する四角形が区画に占める割合。 */
function share(bounds: ScreenBounds, width: number, height: number): number {
  return (
    ((bounds.right - bounds.left) * (bounds.bottom - bounds.top)) / (width * height)
  );
}

/** 直す前の「視点を戻す」が作っていた四角形(縦の画角だけで距離を決めていた)。 */
function legacyBounds(width: number, height: number): ScreenBounds {
  const distance = legacyPaperDistance(1, 1);
  const scale = height / (2 * Math.tan((45 * Math.PI) / 360));
  let left = Infinity;
  let right = -Infinity;
  let top = Infinity;
  let bottom = -Infinity;
  for (const [x, y] of [
    [0, 0],
    [1, 0],
    [1, 1],
    [0, 1],
  ]) {
    const v = new THREE.Vector3(x - 0.5, y - 0.5, 0);
    const depth = distance - v.dot(CAMERA_DIR);
    const px = width / 2 + (v.dot(RIGHT) * scale) / depth;
    const py = height / 2 - (v.dot(SCREEN_UP) * scale) / depth;
    left = Math.min(left, px);
    right = Math.max(right, px);
    top = Math.min(top, py);
    bottom = Math.max(bottom, py);
  }
  return { left, right, top, bottom };
}

describe("3D区画への紙の収まり", () => {
  it.each(PANES)(
    "窓$window(区画$width×$height)で紙がはみ出さない",
    ({ width, height }) => {
      const { bounds } = framingOf(width, height);
      expect(overflow(bounds, width, height)).toEqual({
        left: 0,
        right: 0,
        top: 0,
        bottom: 0,
      });
    },
  );

  it.each(PANES)(
    "窓$window(区画$width×$height)で紙が直す前より小さくならない",
    ({ width, height }) => {
      const before = share(legacyBounds(width, height), width, height);
      const after = share(framingOf(width, height).bounds, width, height);
      // 札を避けるために縮めてよいのは面積で1割まで(実測: 605×439で47.1%→42.6%)
      expect(after).toBeGreaterThanOrEqual(before * 0.9);
    },
  );

  it("区画が縦長でも左右へはみ出さない(直す前は左136px・右189pxはみ出した)", () => {
    const width = 200;
    const height = 600;
    const before = overflow(legacyBounds(width, height), width, height);
    expect(Math.round(before.left)).toBe(136);
    expect(Math.round(before.right)).toBe(189);
    const after = overflow(framingOf(width, height).bounds, width, height);
    expect(after).toEqual({ left: 0, right: 0, top: 0, bottom: 0 });
  });

  it.each([
    { how: "横に広げる", width: 900, height: 261 },
    { how: "横に狭める", width: 400, height: 261 },
    { how: "縦に伸ばす", width: 605, height: 500 },
    { how: "縦に縮める", width: 605, height: 120 },
  ])("区画を$howと収まり直す", ({ width, height }) => {
    const { bounds } = framingOf(width, height);
    expect(overflow(bounds, width, height)).toEqual({
      left: 0,
      right: 0,
      top: 0,
      bottom: 0,
    });
  });

  it("区画が高いときは紙を案内の札の下から逃がす", () => {
    // 説明書を撮ったときの区画。直す前は紙の上端が76pxで札(下端107px)に隠れていた
    expect(Math.round(legacyBounds(605, 439).top)).toBe(76);
    expect(framingOf(605, 439).bounds.top).toBeGreaterThanOrEqual(HINT_BOTTOM);
  });

  it("札が区画の4割を占める低い区画では、避けきれない代わりに紙を縮めない", () => {
    // 区画605×261では札(107px)が高さの41%。避けようとすると紙が小さくなりすぎる
    const width = 605;
    const height = 261;
    const before = legacyBounds(width, height);
    const after = framingOf(width, height).bounds;
    expect(after.top).toBeLessThan(HINT_BOTTOM); // 避けきれていない
    expect(after.top).toBeGreaterThan(before.top); // それでも直す前より下がっている
    expect(share(after, width, height)).toBeGreaterThanOrEqual(
      share(before, width, height) * 0.95,
    );
  });

  it("札の下端を渡さなければ紙は区画の中央に置かれる", () => {
    const framing = framingOf(605, 439, 0);
    const center = (framing.bounds.top + framing.bounds.bottom) / 2;
    // 中央の軸(区画の高さの半分)を基準に組み立てるので、枠のずれは0になる
    expect(framing.offsetY).toBe(0);
    expect(framing.fullHeight).toBe(439);
    expect(center).toBeGreaterThan(0);
  });

  it("求めた枠は投影に矛盾なく入れられる", () => {
    const framing: PaperFraming = framingOf(605, 439);
    const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
    camera.setViewOffset(
      framing.fullWidth,
      framing.fullHeight,
      framing.offsetX,
      framing.offsetY,
      605,
      439,
    );
    camera.up.copy(UP);
    camera.position
      .set(0.5, 0.5, 0)
      .addScaledVector(CAMERA_DIR, framing.distance);
    camera.lookAt(0.5, 0.5, 0);
    camera.updateMatrixWorld(true);
    // three.jsの投影で四隅を写しても、求めた四角形と1px以内で一致する
    let left = Infinity;
    let right = -Infinity;
    let top = Infinity;
    let bottom = -Infinity;
    for (const [x, y] of [
      [0, 0],
      [1, 0],
      [1, 1],
      [0, 1],
    ]) {
      const ndc = new THREE.Vector3(x, y, 0).project(camera);
      const px = ((ndc.x + 1) / 2) * 605;
      const py = ((1 - ndc.y) / 2) * 439;
      left = Math.min(left, px);
      right = Math.max(right, px);
      top = Math.min(top, py);
      bottom = Math.max(bottom, py);
    }
    expect(left).toBeCloseTo(framing.bounds.left, 1);
    expect(right).toBeCloseTo(framing.bounds.right, 1);
    expect(top).toBeCloseTo(framing.bounds.top, 1);
    expect(bottom).toBeCloseTo(framing.bounds.bottom, 1);
  });
});
