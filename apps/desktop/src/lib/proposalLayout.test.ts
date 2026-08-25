import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  LENGTH_RANGE,
  ROOT_ID,
  WIDTH_RANGE,
  addLimb,
  defaultSkeleton,
  leafNodes,
  setTipPos,
  skeletonRows,
} from "./skeleton";
import {
  PAPER_POSITION_DIALOG_VIEWPORT_GUTTER_PX,
  PAPER_POSITION_EDITOR_MAX_WIDTH_PX,
  PAPER_POSITION_LAYOUT_GAP_PX,
  PAPER_POSITION_NOTICE_MAX_HEIGHT_PX,
  PREVIEW_FRAME_MARGIN,
  PROPOSAL_PAPER_DIALOG_MAX_WIDTH_PX,
  calculatePaperPositionEditorLayout,
  calculatePreviewFrame,
  calculateProposalLayout,
  clientPointToTipPos,
  paperPositionEditorWidthPx,
  previewPointToTipPos,
  svgPointFromClient,
  tipPosToPreviewPoint,
} from "./proposalLayout";
import type { Skeleton } from "./types";

const dialogsCss = readFileSync(
  new URL("../styles/dialogs.css", import.meta.url),
  "utf8",
);

function boundsIntersect(
  first: { left: number; top: number; right: number; bottom: number },
  second: { left: number; top: number; right: number; bottom: number },
): boolean {
  return !(
    first.right <= second.left ||
    second.right <= first.left ||
    first.top <= second.bottom ||
    second.top <= first.bottom
  );
}

function boundsIntersectCircle(
  bounds: { left: number; top: number; right: number; bottom: number },
  center: readonly [number, number],
  radius: number,
): boolean {
  const nearestX = Math.max(bounds.left, Math.min(center[0], bounds.right));
  const nearestY = Math.max(bounds.bottom, Math.min(center[1], bounds.top));
  return Math.hypot(nearestX - center[0], nearestY - center[1]) < radius;
}

function withLargestParts(skeleton: Skeleton): Skeleton {
  return {
    nodes: skeleton.nodes.map((node) =>
      node.parent === null
        ? node
        : {
            ...node,
            length: LENGTH_RANGE.max,
            width_factor: WIDTH_RANGE.max,
          },
    ),
  };
}

function twelveRadialTips(): Skeleton {
  let skeleton = defaultSkeleton();
  while (leafNodes(skeleton).length < 12) {
    skeleton = addLimb(skeleton, ROOT_ID);
  }
  return withLargestParts(skeleton);
}

function depthTwelveChain(): Skeleton {
  return withLargestParts({
    nodes: [
      { id: ROOT_ID, parent: null, length: 0, width_factor: 1 },
      ...Array.from({ length: 12 }, (_, index) => ({
        id: index + 1,
        parent: index,
        length: 1,
        width_factor: 1,
      })),
    ],
  });
}

function deepTwelveTipBranch(): Skeleton {
  let skeleton = defaultSkeleton();
  skeleton = addLimb(skeleton, 1);
  const secondLevel = skeleton.nodes.find((node) => node.parent === 1)!;
  skeleton = addLimb(skeleton, secondLevel.id);
  const thirdLevel = skeleton.nodes.find(
    (node) => node.parent === secondLevel.id,
  )!;
  for (let index = 0; index < 9; index += 1) {
    skeleton = addLimb(skeleton, thirdLevel.id);
  }
  return withLargestParts(skeleton);
}

function numericValues(value: unknown): number[] {
  if (typeof value === "number") return [value];
  if (Array.isArray(value)) return value.flatMap(numericValues);
  if (value !== null && typeof value === "object") {
    return Object.values(value).flatMap(numericValues);
  }
  return [];
}

describe("提案画面の純粋な配置計算", () => {
  it("1000×700の紙位置の大きな別画面も横へ1pxもはみ出さない", () => {
    const layout = calculatePaperPositionEditorLayout({
      width: 1_000,
      height: 700,
    });
    expect(layout.horizontalExcessPx).toBe(-24);
    expect(layout.horizontalOverflowPx).toBe(0);
    expect(layout.dialog).toEqual({ left: 24, width: 952, right: 976 });
    expect(layout.editor).toEqual({
      left: 42,
      width: 560,
      right: 602,
    });
    expect(layout.controls).toEqual({ left: 618, width: 340, right: 958 });
    expect(layout.dialogMaxHeight).toBe(664);
    expect(layout.dialogVertical).toEqual({ top: 18, height: 664, bottom: 682 });
    expect(layout.editorVertical).toEqual({ top: 70, height: 560, bottom: 630 });
    expect(layout.verticalExcessPx).toBe(-18);
    expect(layout.verticalOverflowPx).toBe(0);
    expect(layout.contentVerticalOverflowPx).toBe(0);
    expect(layout.noticeMaximumHeight).toBe(
      PAPER_POSITION_NOTICE_MAX_HEIGHT_PX,
    );
    expect(layout.editorViewportHeight).toBe(560);
    expect(layout.squareEditorContentHeight).toBe(560);
    expect(layout.squareEditorScrollRange).toBe(0);
    expect(PROPOSAL_PAPER_DIALOG_MAX_WIDTH_PX).toBe(960);
    expect(PAPER_POSITION_DIALOG_VIEWPORT_GUTTER_PX).toBe(36);
    expect(PAPER_POSITION_EDITOR_MAX_WIDTH_PX).toBe(560);
    expect(PAPER_POSITION_LAYOUT_GAP_PX).toBe(16);
    for (const span of [layout.dialog, layout.editor, layout.controls]) {
      expect(span.left).toBeGreaterThanOrEqual(0);
      expect(span.right).toBeLessThanOrEqual(1_000);
      expect(span.width).toBeGreaterThanOrEqual(0);
    }
    expect(numericValues(layout).every(Number.isFinite)).toBe(true);
  });

  it("正方形・横長・縦長のどれも長辺を560pxに保ち、つまみの倍率を変えない", () => {
    expect(paperPositionEditorWidthPx({ width: 1.12, height: 1.12 })).toBe(560);
    expect(paperPositionEditorWidthPx({ width: 1.12, height: 0.17 })).toBe(560);
    expect(paperPositionEditorWidthPx({ width: 0.17, height: 1.12 })).toBe(85);
    expect(paperPositionEditorWidthPx({ width: 0, height: 1 })).toBe(560);
    for (const viewBox of [
      { width: 1.12, height: 1.12 },
      { width: 1.12, height: 0.17 },
      { width: 0.17, height: 1.12 },
    ]) {
      const width = paperPositionEditorWidthPx(viewBox);
      const height = width * (viewBox.height / viewBox.width);
      expect(Math.max(width, height)).toBeCloseTo(560, 12);
    }
  });

  it("1000×700では込み入った形も横へ1pxもはみ出さない", () => {
    const complex = deepTwelveTipBranch();
    expect(leafNodes(complex)).toHaveLength(12);
    expect(Math.max(...skeletonRows(complex).map((row) => row.depth))).toBe(4);
    const samples = [
      ["初期4本", defaultSkeleton()],
      ["胴から12本", twelveRadialTips()],
      ["深さ12の一本道", depthTwelveChain()],
      ["深さ4・先端12本", complex],
    ] as const;

    for (const [name, skeleton] of samples) {
      const layout = calculateProposalLayout(skeleton, {
        width: 1_000,
        height: 700,
      });
      expect(layout.horizontalExcessPx, name).toBeLessThanOrEqual(0);
      expect(layout.horizontalOverflowPx, name).toBe(0);
      expect(layout.dialog, name).toEqual({ left: 140, width: 720, right: 860 });
      expect(layout.preview, name).toEqual({ left: 166, width: 200, right: 366 });
      expect(layout.list, name).toEqual({ left: 380, width: 437, right: 817 });
      expect(layout.horizontalExcessPx, name).toBe(-140);
      expect(layout.dialogMaxHeight, name).toBe(616);
      expect(layout.rows, name).toHaveLength(skeleton.nodes.length - 1);
      for (const span of [
        layout.dialog,
        layout.preview,
        layout.list,
        ...layout.rows,
      ]) {
        expect(span.left, name).toBeGreaterThanOrEqual(0);
        expect(span.right, name).toBeLessThanOrEqual(1_000);
        expect(span.width, name).toBeGreaterThanOrEqual(0);
      }
      expect(numericValues(layout).every(Number.isFinite), name).toBe(true);
    }
  });

  it("場所を決めても見本の倍率が動かない", () => {
    const base = defaultSkeleton();
    const ids = leafNodes(base).map((node) => node.id);
    let moved = base;
    for (const [index, id] of ids.entries()) {
      moved = setTipPos(moved, id, {
        x: index % 2 === 0 ? 1 : -1,
        y: index < 2 ? -1 : 1,
      });
    }
    const before = calculatePreviewFrame(base);
    const after = calculatePreviewFrame(moved);

    // 決めた場所は倍率の計算に入れない。入れると動かすたびに手元がずれる。
    expect(after.frameRadius).toBe(before.frameRadius);
    expect(after.positionRadius).toBe(before.positionRadius);
    expect(after.viewBox).toEqual(before.viewBox);
    // 長さ1・太さ1の4本立てでは、届く距離は 0.5*1 + 1 = 1.5(手計算)
    expect(before.positionRadius).toBe(1.5);
    expect(before.frameRadius).toBe(1.875);
    expect(before.frameRadius / before.positionRadius).toBe(
      PREVIEW_FRAME_MARGIN,
    );
  });

  it("決めた場所と見本の座標が同じ点を指す", () => {
    const skeleton = setTipPos(defaultSkeleton(), 1, { x: 0.25, y: -0.5 });
    const frame = calculatePreviewFrame(skeleton);
    const part = frame.parts.find((p) => p.id === 1)!;

    // 手計算: positionRadius = 1.5 なので (0.375, -0.75)
    expect(part.end[0]).toBeCloseTo(0.375, 12);
    expect(part.end[1]).toBeCloseTo(-0.75, 12);
    expect(part.tipPos).toEqual({ x: 0.25, y: -0.5 });
    expect(part.isTip).toBe(true);
    // 場所を決めていない先端は自動のまま
    expect(frame.parts.find((p) => p.id === 2)!.tipPos).toBeNull();
  });

  it("場所と見本の座標を往復しても値が変わらない", () => {
    const radius = 1.5;
    let worst = 0;
    for (let i = -10; i <= 10; i += 1) {
      for (let j = -10; j <= 10; j += 1) {
        const pos = { x: i / 10, y: j / 10 };
        const back = previewPointToTipPos(
          tipPosToPreviewPoint(pos, radius),
          radius,
        );
        worst = Math.max(
          worst,
          Math.abs(back.x - pos.x),
          Math.abs(back.y - pos.y),
        );
      }
    }
    // 実測: 1.1102230246251565e-16(2026-08-17、441点)。合格条件は1e-9。
    expect(worst).toBeLessThan(1e-9);
    expect(worst).toBeLessThanOrEqual(1.2e-16);
  });

  it("枠の外へ引っぱっても -1.0〜1.0 の中へ収める", () => {
    const radius = 1.5;
    expect(previewPointToTipPos([99, -99], radius)).toEqual({ x: 1, y: -1 });
    expect(previewPointToTipPos([-99, 99], radius)).toEqual({ x: -1, y: 1 });
    expect(previewPointToTipPos([Infinity, -Infinity], radius)).toEqual({
      x: 1,
      y: -1,
    });
    expect(previewPointToTipPos([NaN, NaN], radius)).toEqual({ x: 0, y: 0 });
    // 大きさのない見本でも数にならない値を返さない
    expect(previewPointToTipPos([1, 1], 0)).toEqual({ x: 0, y: 0 });
  });

  it("画面上の位置を見本の座標へ直すとき、余白の分を戻す", () => {
    const viewBox = { x: -2, y: -2, width: 4, height: 4 };
    // 横に長い場所へ置くと、左右へ (200-100)/2 = 50px の余白ができる
    const rect = { left: 10, top: 20, width: 200, height: 100 };
    expect(
      svgPointFromClient([10 + 50 + 50, 20 + 50], rect, viewBox),
    ).toEqual([0, 0]);
    expect(svgPointFromClient([10 + 50, 20], rect, viewBox)).toEqual([-2, -2]);
    // 大きさが取れない場合でも数にならない値を返さない
    expect(
      svgPointFromClient([0, 0], { left: 0, top: 0, width: 0, height: 0 }, viewBox),
    ).toEqual([-2, -2]);
  });

  it("画面上の位置からの場所は、上が正・下が負になる", () => {
    const frame = calculatePreviewFrame(defaultSkeleton());
    // frameRadius=1.875 の正方形を 200px 四方へ置く
    const rect = { left: 0, top: 0, width: 200, height: 200 };
    const scale = 200 / 3.75;
    const at = (x: number, y: number) =>
      clientPointToTipPos(
        [(x * 1.5 + 1.875) * scale, (-y * 1.5 + 1.875) * scale],
        rect,
        frame,
      );
    for (const [x, y] of [
      [0, 0],
      [0.5, 0.25],
      [-0.75, -0.5],
      [1, 1],
      [-1, -1],
    ]) {
      const got = at(x, y);
      expect(Math.abs(got.x - x)).toBeLessThan(1e-9);
      expect(Math.abs(got.y - y)).toBeLessThan(1e-9);
    }
  });

  it("枠の角へ場所を決めても、線とつまみが枠から出ない", () => {
    let skeleton = withLargestParts(defaultSkeleton());
    const corners = [
      { x: 1, y: 1 },
      { x: -1, y: 1 },
      { x: 1, y: -1 },
      { x: -1, y: -1 },
    ];
    leafNodes(skeleton).forEach((node, index) => {
      skeleton = setTipPos(skeleton, node.id, corners[index]);
    });
    const frame = calculatePreviewFrame(skeleton);
    const limit = frame.frameRadius;

    for (const part of frame.parts) {
      const pad = Math.max(part.strokeWidth / 2, part.handleRadius);
      for (const axis of [0, 1]) {
        expect(Math.abs(part.end[axis]) + pad).toBeLessThanOrEqual(limit);
        expect(Math.abs(part.start[axis])).toBeLessThanOrEqual(limit);
        expect(Math.abs(part.labelPosition[axis])).toBeLessThanOrEqual(limit);
      }
    }
  });

  it("最大12先端・長い呼び名・四隅でも、呼び名をつまみや他の呼び名と重ねない", () => {
    let twelveAtCorners = twelveRadialTips();
    const corners = [
      { x: 1, y: 1 },
      { x: -1, y: 1 },
      { x: 1, y: -1 },
      { x: -1, y: -1 },
    ];
    leafNodes(twelveAtCorners).forEach((node, index) => {
      twelveAtCorners = setTipPos(
        twelveAtCorners,
        node.id,
        corners[index % corners.length],
      );
    });
    let deepAtCorners = deepTwelveTipBranch();
    leafNodes(deepAtCorners).forEach((node, index) => {
      deepAtCorners = setTipPos(
        deepAtCorners,
        node.id,
        corners[index % corners.length],
      );
    });
    const samples = [
      ["初期4本（右前足を含む）", defaultSkeleton()],
      ["胴から12本", twelveRadialTips()],
      ["胴から12本・四隅", twelveAtCorners],
      ["長い呼び名・先端12本", deepTwelveTipBranch()],
      ["長い呼び名・先端12本・四隅", deepAtCorners],
    ] as const;

    for (const [name, skeleton] of samples) {
      const frame = calculatePreviewFrame(skeleton);
      const handles = frame.parts.filter((part) => part.isTip);
      const labels = frame.parts.map((part) => part.labelBounds);
      expect(handles.length, name).toBeLessThanOrEqual(12);
      expect(
        frame.parts.some(
          (part) => part.label === "出っぱり12" || part.label === "その先1",
        ),
        name,
      ).toBe(name.startsWith("長い") || name.includes("12本"));

      for (const [index, part] of frame.parts.entries()) {
        expect(part.labelBounds.left, `${name}: ${part.label} 左`).toBeGreaterThanOrEqual(
          -frame.frameRadius,
        );
        expect(part.labelBounds.right, `${name}: ${part.label} 右`).toBeLessThanOrEqual(
          frame.frameRadius,
        );
        expect(part.labelBounds.bottom, `${name}: ${part.label} 下`).toBeGreaterThanOrEqual(
          -frame.frameRadius,
        );
        expect(part.labelBounds.top, `${name}: ${part.label} 上`).toBeLessThanOrEqual(
          frame.frameRadius,
        );
        for (const handle of handles) {
          expect(
            boundsIntersectCircle(
              part.labelBounds,
              handle.end,
              handle.handleRadius + handle.handleStrokeWidth / 2,
            ),
            `${name}: ${part.label} と ${handle.label} のつまみ`,
          ).toBe(false);
        }
        for (let other = index + 1; other < labels.length; other += 1) {
          expect(
            boundsIntersect(part.labelBounds, labels[other]),
            `${name}: ${part.label} と ${frame.parts[other].label}`,
          ).toBe(false);
        }
      }
    }

    const defaultFrame = calculatePreviewFrame(defaultSkeleton());
    const rightFront = defaultFrame.parts.find(
      (part) => part.label === "右前足",
    )!;
    // 実画面では修正前に約10.5px重なった組。200px表示へ換算しても離れている。
    const scale = 200 / defaultFrame.viewBox.width;
    const horizontalGapPx =
      (rightFront.labelBounds.left -
        (rightFront.end[0] +
          rightFront.handleRadius +
          rightFront.handleStrokeWidth / 2)) *
      scale;
    expect(horizontalGapPx).toBeGreaterThan(0);
  });

  it("見本のつまみに共通の枠線を出さない(絵の中では桁が違うため)", () => {
    // 実機で起きたこと: 共通の `:focus-visible { outline: 2px; box-shadow: 0 0 0 4px }`
    // が絵の中では「2」「4」を絵の座標として扱われ、200pxの見本のほぼ全面が
    // 塗りつぶされた(実測: 見本の枠の半径1.875に対し、半径約2.5の塗り)。
    // 選んでいる先端は、絵の中の長さで描く輪(tip-focus-ring)で示す。
    const css = dialogsCss;
    const rule = css.match(
      /\.skeleton-preview \.tip-handle:focus,\s*\n\s*\.skeleton-preview \.tip-handle:focus-visible \{([^}]*)\}/u,
    );
    expect(rule, "つまみの枠線を止める指定").not.toBeNull();
    expect(rule![1]).toContain("outline: none");
    expect(rule![1]).toContain("box-shadow: none");
  });

  it("紙位置の大きな絵でもつまみに共通の枠線を出さない", () => {
    const css = dialogsCss;
    const rule = css.match(
      /\.paper-position-handle:focus,\s*\n\s*\.paper-position-handle:focus-visible \{([^}]*)\}/u,
    );
    expect(rule, "紙位置つまみの枠線を止める指定").not.toBeNull();
    expect(rule![1]).toContain("outline: none");
    expect(rule![1]).toContain("box-shadow: none");
  });

  it("紙位置の呼び名と引き出し線はつまみの操作を遮らない", () => {
    const css = dialogsCss;
    const rule = css.match(
      /\.paper-position-label,\s*\n\s*\.paper-position-label-leader \{([^}]*)\}/u,
    );
    expect(rule, "呼び名と引き出し線の表示専用指定").not.toBeNull();
    expect(rule![1]).toContain("pointer-events: none");
  });

  it("1000×700では紙全体と説明・操作を2列に収める", () => {
    const css = dialogsCss;
    const dialog = css.match(
      /\.dialog-wide\[data-proposal-step="paper-position"\] \{([^}]*)\}/u,
    );
    const step = css.match(/\.paper-position-step \{([^}]*)\}/u);
    const sidebar = css.match(/\.paper-position-sidebar \{([^}]*)\}/u);
    const stage = [
      ...css.matchAll(/\.paper-position-stage \{([^}]*)\}/gu),
    ].find((match) => match[1].includes("width: 560px"));
    const notices = css.match(
      /\.paper-position-sidebar > \.proposal-position-notices \{([^}]*)\}/u,
    );
    const actions = css.match(
      /\.dialog \.paper-position-actions:last-child \{([^}]*)\}/u,
    );
    const editor = [
      ...css.matchAll(/\.paper-position-editor \{([^}]*)\}/gu),
    ].find((match) => match[1].includes("flex: none"));
    expect(dialog, "紙位置ダイアログ").not.toBeNull();
    expect(dialog![1]).toContain("height: min(668px, calc(100vh - 36px))");
    expect(dialog![1]).toContain("max-height: min(668px, calc(100vh - 36px))");
    expect(dialog![1]).toContain("padding: 16px");
    expect(dialog![1]).toContain("overflow: hidden");
    expect(step, "紙と操作の2列").not.toBeNull();
    expect(step![1]).toContain("grid-template-columns: 560px minmax(0, 1fr)");
    expect(step![1]).toContain("grid-template-rows: minmax(0, 1fr) auto");
    expect(step![1]).toContain("gap: var(--sp-4) 16px");
    expect(step![1]).toContain("min-height: 0");
    expect(sidebar, "説明・通知・操作の列").not.toBeNull();
    expect(sidebar![1]).toContain("grid-column: 2");
    expect(sidebar![1]).toContain("overflow: hidden");
    expect(stage, "紙全体の固定領域").toBeDefined();
    expect(stage![1]).toContain("width: 560px");
    expect(stage![1]).toContain("height: 560px");
    expect(stage![1]).toContain("min-height: 560px");
    expect(stage![1]).toContain("overflow: hidden");
    expect(stage![1]).toContain("scrollbar-gutter: auto");
    expect(notices, "最大12本の知らせ").not.toBeNull();
    expect(notices![1]).toContain(
      `max-height: ${PAPER_POSITION_NOTICE_MAX_HEIGHT_PX}px`,
    );
    expect(notices![1]).toContain("overflow-y: auto");
    expect(actions, "常に残す3操作").not.toBeNull();
    expect(actions![1]).toContain("grid-column: 2");
    expect(actions![1]).toContain("grid-template-columns: minmax(0, 1fr)");
    expect(editor, "紙の長辺560pxを縮ませない指定").toBeDefined();
    expect(editor![1]).toContain("flex: none");
  });

  it("場所が違う葉を12件出しても、1000×700の既存ダイアログ幅で横へ折り返す", () => {
    const css = dialogsCss;
    const notice = css.match(/\.proposal-position-notices \{([^}]*)\}/u);
    const row = css.match(/\.proposal-position-notices li \{([^}]*)\}/u);
    const text = css.match(
      /\.proposal-position-notices li > span \{([^}]*)\}/u,
    );
    expect(notice, "食い違う場所の知らせ").not.toBeNull();
    expect(notice![1]).toContain("width: 100%");
    expect(notice![1]).toContain("min-width: 0");
    expect(notice![1]).toContain("max-width: 100%");
    expect(notice![1]).toContain("box-sizing: border-box");
    expect(row, "葉ごとの行").not.toBeNull();
    expect(row![1]).toContain("flex-wrap: wrap");
    expect(row![1]).toContain("min-width: 0");
    expect(row![1]).toContain("max-width: 100%");
    expect(text, "長い葉名").not.toBeNull();
    expect(text![1]).toContain("overflow-wrap: anywhere");
  });

  it("SVGの全ての線・接続位置・文字位置を見本枠の内側に置く", () => {
    const frame = calculatePreviewFrame(deepTwelveTipBranch());
    const minX = frame.viewBox.x;
    const maxX = frame.viewBox.x + frame.viewBox.width;

    for (const part of frame.parts) {
      const linePadding = part.strokeWidth / 2;
      expect(Math.min(part.start[0], part.end[0]) - linePadding).toBeGreaterThanOrEqual(
        minX,
      );
      expect(Math.max(part.start[0], part.end[0]) + linePadding).toBeLessThanOrEqual(
        maxX,
      );
      expect(part.start[0] - part.connectionRadius).toBeGreaterThanOrEqual(minX);
      expect(part.start[0] + part.connectionRadius).toBeLessThanOrEqual(maxX);
      expect(part.labelPosition[0]).toBeGreaterThanOrEqual(minX);
      expect(part.labelPosition[0]).toBeLessThanOrEqual(maxX);
    }
  });
});
