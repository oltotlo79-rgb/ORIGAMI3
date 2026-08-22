// @vitest-environment jsdom
// 3D区画へ重ねる表示が、下の紙・線・点のクリックを吸わないことの検査。
//
// 利用者の指摘(2026-08-16): 「合わせて折る」で3D図の線を指定できない。
// 原因は左上の操作ヒントの札で、押す場所が無いのにクリックを受け取っていた。
// ここではApp.cssを実際にjsdomへ読み込ませ、計算後のpointer-eventsで当たり判定を作り、
// 札の内側で押した回数のうち紙に届いた回数を数える。

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { ViewerOperationHint } from "./Viewer3D/ViewerOperationHint";
import { useAppStore } from "../store/appStore";

// 画面で実際に効いているApp.cssを、そのままjsdomへ読み込ませて確かめる。
// vitestは.cssの取り込みを空にするため、既存のuiTokens.test.tsと同じく直に読む。
// jsdom環境のURLはページの位置を基準にしてしまうので、node側の場所へ直してから読む。
const appCss = readFileSync(
  join(dirname(fileURLToPath(import.meta.url)), "..", "App.css"),
  "utf8",
);
const initialStoreState = useAppStore.getState();

interface Rect {
  left: number;
  top: number;
  width: number;
  height: number;
}

const rects = new Map<Element, Rect>();
const originalGetBoundingClientRect = Element.prototype.getBoundingClientRect;

function setRect(element: Element, rect: Rect) {
  rects.set(element, rect);
}

function contains(rect: Rect, x: number, y: number): boolean {
  return (
    x >= rect.left &&
    x < rect.left + rect.width &&
    y >= rect.top &&
    y < rect.top + rect.height
  );
}

/**
 * ブラウザの当たり判定を、pointer-eventsの計算値だけで真似る。
 * 後に現れる要素ほど手前に描かれるので、条件に合う最後の要素を返す。
 */
function hitTest(root: Element, x: number, y: number): Element | null {
  let hit: Element | null = null;
  const walk = (element: Element) => {
    const rect = rects.get(element);
    if (
      rect &&
      contains(rect, x, y) &&
      window.getComputedStyle(element).pointerEvents !== "none"
    ) {
      hit = element;
    }
    for (const child of Array.from(element.children)) walk(child);
  };
  walk(root);
  return hit;
}

beforeAll(() => {
  const style = document.createElement("style");
  style.textContent = appCss;
  document.head.appendChild(style);
  Element.prototype.getBoundingClientRect = function (this: Element) {
    const rect = rects.get(this) ?? { left: 0, top: 0, width: 0, height: 0 };
    return {
      x: rect.left,
      y: rect.top,
      left: rect.left,
      top: rect.top,
      right: rect.left + rect.width,
      bottom: rect.top + rect.height,
      width: rect.width,
      height: rect.height,
      toJSON: () => rect,
    } as DOMRect;
  };
});

afterAll(() => {
  Element.prototype.getBoundingClientRect = originalGetBoundingClientRect;
});

afterEach(() => {
  cleanup();
  rects.clear();
  useAppStore.setState(initialStoreState, true);
});

/** 3D区画・紙のcanvas・操作ヒントの札を、実際の重なりと同じ形で組み立てる。 */
function renderPane(expanded = true) {
  useAppStore.setState({ activeTool: "fold", viewerHintExpanded: expanded });
  const pane = document.createElement("div");
  pane.className = "pane-3d-view";
  const canvas = document.createElement("canvas");
  canvas.className = "viewer3d-canvas";
  pane.appendChild(canvas);
  // Reactの描画先はcanvasを消してしまうので、製品と同じ縦列を別の入れ物へ描く。
  const overlayHost = document.createElement("div");
  overlayHost.className = "viewer-overlay-stack";
  pane.appendChild(overlayHost);
  document.body.appendChild(pane);

  render(
    <ViewerOperationHint hint="点または線をクリックして選びます" blocked={false} aligning />,
    { container: overlayHost },
  );

  const hint = pane.querySelector<HTMLElement>(".viewer-operation-hint");
  const toggle = pane.querySelector<HTMLButtonElement>(".viewer-hint-toggle");
  if (!hint || !toggle) throw new Error("操作ヒントの札が組み立てられていない");

  // jsdomには配置が無いので、実画面と同じ位置・大きさを与える。
  // 札は3D区画の左上、開閉ボタンはその右下の中に置く。
  setRect(pane, { left: 0, top: 0, width: 900, height: 600 });
  setRect(canvas, { left: 0, top: 0, width: 900, height: 600 });
  setRect(hint, { left: 20, top: 20, width: 430, height: 200 });
  setRect(toggle, { left: 300, top: 170, width: 130, height: 28 });

  return { pane, canvas, hint, toggle };
}

/** 札の内側を等間隔に取り、開閉ボタンに重なる点だけ除く。 */
function samplePointsInHint(hint: Rect, toggle: Rect): { x: number; y: number }[] {
  const points: { x: number; y: number }[] = [];
  for (let ix = 1; ix <= 8; ix += 1) {
    for (let iy = 1; iy <= 6; iy += 1) {
      const x = hint.left + (hint.width * ix) / 9;
      const y = hint.top + (hint.height * iy) / 7;
      if (contains(toggle, x, y)) continue;
      points.push({ x, y });
    }
  }
  return points;
}

describe("3D区画へ重ねる表示のクリックの通し方", () => {
  it("札の四角形の内側でも、下の紙のクリックが届く", () => {
    const { pane, canvas, hint, toggle } = renderPane();
    let reachedPaper = 0;
    canvas.addEventListener("pointerdown", () => {
      reachedPaper += 1;
    });

    const hintRect = rects.get(hint)!;
    const toggleRect = rects.get(toggle)!;
    // 8×6の48点のうち、開閉ボタンに重なる3点を除いた45点を押す。
    const points = samplePointsInHint(hintRect, toggleRect);
    expect(points.length).toBe(45);

    let blocked = 0;
    for (const point of points) {
      const target = hitTest(pane, point.x, point.y);
      if (target !== canvas) blocked += 1;
      if (target) {
        fireEvent.pointerDown(target, { clientX: point.x, clientY: point.y });
      }
    }

    // 札に吸われた回数が0で、44点すべてが紙へ届く。
    expect(blocked).toBe(0);
    expect(reachedPaper).toBe(points.length);
  });

  it("札の中の開閉ボタンは今までどおり押せる", () => {
    const { pane, canvas, toggle } = renderPane();
    let reachedPaper = 0;
    canvas.addEventListener("pointerdown", () => {
      reachedPaper += 1;
    });

    const toggleRect = rects.get(toggle)!;
    const center = {
      x: toggleRect.left + toggleRect.width / 2,
      y: toggleRect.top + toggleRect.height / 2,
    };
    const target = hitTest(pane, center.x, center.y);
    expect(target).toBe(toggle);

    expect(useAppStore.getState().viewerHintExpanded).toBe(true);
    fireEvent.pointerDown(target!, { clientX: center.x, clientY: center.y });
    fireEvent.click(target!);
    expect(useAppStore.getState().viewerHintExpanded).toBe(false);
    // 開閉ボタンの上のクリックは紙へ渡さない。
    expect(reachedPaper).toBe(0);

    fireEvent.click(target!);
    expect(useAppStore.getState().viewerHintExpanded).toBe(true);
  });

  it("畳んだ札でも、内側のクリックは紙へ届く", () => {
    const { pane, canvas, hint, toggle } = renderPane(false);
    let reachedPaper = 0;
    canvas.addEventListener("pointerdown", () => {
      reachedPaper += 1;
    });
    setRect(hint, { left: 20, top: 20, width: 430, height: 110 });
    setRect(toggle, { left: 300, top: 90, width: 130, height: 28 });

    // 8×6の48点のうち、開閉ボタンに重なる6点を除いた42点を押す。
    const points = samplePointsInHint(rects.get(hint)!, rects.get(toggle)!);
    expect(points.length).toBe(42);
    let blocked = 0;
    for (const point of points) {
      const target = hitTest(pane, point.x, point.y);
      if (target !== canvas) blocked += 1;
      if (target) fireEvent.pointerDown(target, { clientX: point.x, clientY: point.y });
    }

    expect(blocked).toBe(0);
    expect(reachedPaper).toBe(points.length);
  });

  // 3D区画へ重なる表示をすべて並べ、通す側と受ける側を固定する。
  // 押す場所が無い札は通す、押せるものは受ける。新しく重ねる表示もこの表へ足す。
  it.each<[string, string, string]>([
    ["紙のcanvas", "viewer3d-canvas", "auto"],
    ["案内列の領域", "viewer-overlay-region", "none"],
    ["案内列", "viewer-overlay-stack", "none"],
    ["案内送り操作行", "viewer-overlay-scroll-controls", "none"],
    ["3Dの操作ヒントの札", "viewer-operation-hint", "none"],
    ["3Dの操作ヒントの開閉ボタン", "viewer-hint-toggle", "auto"],
    ["通知の札", "status-badge", "none"],
    ["食い込み候補の案内", "suspect-hinge-guide", "auto"],
    ["視点立方体", "view-cube", "auto"],
    ["視点を戻すボタン", "viewer-reset", "auto"],
    ["紙の案内(小さい形)", "paper-action-tip compact", "auto"],
    ["紙の案内(開いた形)", "paper-action-tip expanded", "auto"],
    // 展開図側の同じ札。3Dはこのやり方にそろえている。
    ["展開図の操作ヒントの札", "cp-operation-hint", "none"],
    ["展開図の操作ヒントの開閉ボタン", "cp-help-toggle", "auto"],
  ])("%s(.%s)のクリックは%s", (_name, className, expected) => {
    const element = document.createElement("div");
    element.className = className;
    const stackClasses = [
      "viewer-operation-hint",
      "status-badge",
      "suspect-hinge-guide",
      "paper-action-tip",
    ];
    let stack: HTMLDivElement | null = null;
    if (stackClasses.some((name) => element.classList.contains(name))) {
      stack = document.createElement("div");
      stack.className = "viewer-overlay-stack";
      stack.appendChild(element);
      document.body.appendChild(stack);
    } else {
      document.body.appendChild(element);
    }
    expect(window.getComputedStyle(element).pointerEvents).toBe(expected);
    if (stack) stack.remove();
    else element.remove();
  });

  it("案内列と操作行の余白は紙へ通し、上下ボタンだけがクリックを受ける", () => {
    const region = document.createElement("div");
    region.className = "viewer-overlay-region";
    const stack = document.createElement("div");
    stack.className = "viewer-overlay-stack";
    const controls = document.createElement("div");
    controls.className = "viewer-overlay-scroll-controls";
    const button = document.createElement("button");
    controls.appendChild(button);
    region.append(stack, controls);
    document.body.appendChild(region);

    expect(window.getComputedStyle(region).pointerEvents).toBe("none");
    expect(window.getComputedStyle(stack).pointerEvents).toBe("none");
    expect(window.getComputedStyle(controls).pointerEvents).toBe("none");
    expect(window.getComputedStyle(button).pointerEvents).toBe("auto");
    region.remove();
  });

  it.each(["viewer-operation-hint collapsed", "paper-action-tip compact", "paper-action-tip expanded"])(
    "縦列内の.%sは後勝ちの個別配置で横へずれない",
    (className) => {
      const stack = document.createElement("div");
      stack.className = "viewer-overlay-stack";
      const card = document.createElement("div");
      card.className = className;
      stack.appendChild(card);
      document.body.appendChild(stack);
      const style = window.getComputedStyle(card);
      expect(style.position).toBe("relative");
      expect(style.right).toBe("auto");
      expect(style.width).toBe("100%");
      expect(style.maxWidth).toBe("100%");
      stack.remove();
    },
  );
});
