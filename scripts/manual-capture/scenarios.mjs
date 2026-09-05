import fs from "node:fs/promises";
import path from "node:path";

const STANDARD_VIEW = { width: 1280, height: 860, deviceScaleFactor: 2 };

function fail(message) {
  throw new Error(`manual capture scenario: ${message}`);
}

async function rebind(client) {
  if (typeof client.rebindGeneration === "function") {
    await client.rebindGeneration();
  } else {
    await client.waitForCaptureApi();
  }
}

async function assertGeneration(client) {
  if (typeof client.assertGeneration === "function") await client.assertGeneration();
}

async function requirePage(client, expression, label, timeoutMs = 20_000) {
  return await client.waitFor(expression, label, timeoutMs);
}

async function clickExact(client, selector, text, index = 0) {
  const result = await client.evaluate(`(() => {
    const wanted = ${JSON.stringify(text)};
    const candidates = Array.from(document.querySelectorAll(${JSON.stringify(selector)}))
      .filter((element) => (element.textContent || "").trim() === wanted);
    const element = candidates[${index}];
    if (!element) return { status: "not-found", candidates: Array.from(document.querySelectorAll(${JSON.stringify(selector)})).map((node) => (node.textContent || "").trim()).filter(Boolean) };
    if (element.disabled || element.getAttribute("aria-disabled") === "true") return { status: "disabled" };
    element.click();
    return { status: "ok" };
  })()`);
  if (result?.status !== "ok") fail(`could not click exact text ${JSON.stringify(text)} in ${selector}: ${JSON.stringify(result)}`);
  await client.sleep(300);
}

async function clickContains(client, selector, text, index = 0) {
  await client.clickText(selector, text, index);
}

async function setCheckbox(client, labelText, checked) {
  const result = await client.evaluate(`(() => {
    const wanted = ${JSON.stringify(labelText.replace(/\s+/g, ""))};
    const candidates = Array.from(document.querySelectorAll("label"));
    const label = candidates.find((element) => (element.textContent || "").replace(/\\s+/g, "").includes(wanted));
    const input = label?.querySelector('input[type="checkbox"]');
    if (!(input instanceof HTMLInputElement)) return "not-found";
    if (input.checked !== ${checked ? "true" : "false"}) input.click();
    return input.checked === ${checked ? "true" : "false"} ? "ok" : "wrong:" + input.checked;
  })()`);
  if (result !== "ok") fail(`checkbox ${labelText} could not be set to ${checked}: ${result}`);
  await client.sleep(300);
}

async function setRangeElement(client, selector, value, finishWithPointerUp = false) {
  const result = await client.evaluate(`(() => {
    const input = document.querySelector(${JSON.stringify(selector)});
    if (!(input instanceof HTMLInputElement) || input.type !== "range") return "not-found";
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    if (!setter) return "no-setter";
    setter.call(input, ${JSON.stringify(String(value))});
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
    ${finishWithPointerUp ? 'input.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerId: 1, pointerType: "mouse" }));' : ""}
    return input.value;
  })()`);
  if (Number(result) !== Number(value)) fail(`range ${selector} did not accept ${value}; actual=${result}`);
  await client.sleep(800);
}

async function setTheme(client, label) {
  await client.setSelect("画面のデザイン", label);
  await requirePage(
    client,
    `(() => { const s = document.querySelector('select[aria-label="画面のデザイン"]'); return s && s.selectedOptions[0]?.textContent?.trim() === ${JSON.stringify(label)}; })()`,
    `theme ${label}`,
  );
}

async function paperRect(client) {
  const rect = await client.evaluate(`(() => {
    const canvas = document.querySelector(".cp-canvas, [data-testid='cp-canvas']");
    if (!canvas) return null;
    const r = canvas.getBoundingClientRect();
    const side = Math.min(r.width, r.height) * 0.8935;
    return { x: r.left + (r.width - side) / 2, y: r.top + (r.height - side) / 2, w: side, h: side };
  })()`);
  if (!rect || rect.w < 100 || rect.h < 100) fail(`invalid paper rectangle: ${JSON.stringify(rect)}`);
  return rect;
}

function point(rect, x, y) {
  return { x: rect.x + rect.w * x, y: rect.y + rect.h * y };
}

async function openFixture(context, fixtureName, { step = "latest", setView = true } = {}) {
  const { client, repositoryRoot } = context;
  const fixture = path.resolve(repositoryRoot, fixtureName);
  const info = await client.evaluate(`window.__origami3Capture.openDocument(${JSON.stringify(fixture)})`);
  if (!info || !Number.isInteger(info.stepCount)) fail(`fixture did not open: ${fixture}`);
  if (step === "latest" && info.stepCount > 0) {
    await client.evaluate(`window.__origami3Capture.goToStep(${info.stepCount})`);
  } else if (Number.isInteger(step)) {
    if (step < 0 || step > info.stepCount) fail(`step ${step} is outside 0..${info.stepCount} for ${fixture}`);
    await client.evaluate(`window.__origami3Capture.goToStep(${step})`);
  }
  if (setView) await client.evaluate(`window.__origami3Capture.setView("normal")`);
  await client.stable();
  return info;
}

// 「鶴」は正本 `crates/ori3-layers/tests/fixtures/traditional-crane/traditional-crane-cp.ori3`
// （利用者から受け取った traditional_crane_math_bundle）から作った3手の作品を使う。
// 以前使っていた `crates/ori3-rigid/tests/fixtures/check-crane.ori3` は提案探索が返した6手の
// 出力（頂点33・辺61）で、完成形は凧形であり鶴にならないため、説明書の撮影には使わない。
const FIXTURES = Object.freeze({
  crane: "apps/desktop/tests-live/fixtures/traditional-crane-full.ori3",
  yakko: "crates/ori3-rigid/tests/fixtures/check-yakko.ori3",
  bird: "crates/ori3-rigid/tests/fixtures/check-bird-base.ori3",
  penetration: "crates/ori3-layers/tests/fixtures/penetration-warning.ori3",
});

async function resetBaseline(context, fixtureName = FIXTURES.crane, options = {}) {
  const { client } = context;
  await client.lockMetrics();
  await client.reload();
  await rebind(client);
  const info = await openFixture(context, fixtureName, options);
  await client.key("Escape", "Escape", 27);
  await client.sleep(150);
  await clickExact(client, "[data-testid='tool-select']", "選択");
  await setTheme(client, "ポップ");
  await clickExact(client, ".context-panel button", "表示の広さを初期に戻す");
  await client.evaluate(`(() => {
    const toggle = document.querySelector('.paper-color-toggle');
    if (toggle?.getAttribute('aria-expanded') === 'true') toggle.click();
    return true;
  })()`);
  await setCheckbox(client, "左右対称に描く", false);
  await client.collapseHints();
  await client.resetScroll();
  await client.neutralMouse();
  await client.stable();
  return info;
}

async function freshPaper(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.crane, { step: 0 });
  await clickContains(client, ".toolbar button", "新規");
  await requirePage(client, `(() => { const d = document.querySelector('[role="dialog"], .dialog'); return d && /新しい紙|紙を作る/.test(d.textContent || "") && Array.from(d.querySelectorAll("button")).some((b) => (b.textContent || "").includes("この紙で作りはじめる")); })()`, "new-paper dialog");
  await clickContains(client, "[role='dialog'] button, .dialog button", "この紙で作りはじめる");
  await requirePage(client, `document.querySelector(".cp-canvas, [data-testid='cp-canvas']") !== null`, "fresh paper canvas");
  await setCheckbox(client, "左右対称に描く", false);
  await client.stable();
}

async function drawLine(context, toolLabel, start, end, { leavePreview = false } = {}) {
  const { client, deferCleanup } = context;
  await clickExact(client, ".tool-rail button", toolLabel);
  await client.overlayHitTest(false);
  deferCleanup(async () => {
    await client.overlayHitTest(true);
    await client.key("Escape", "Escape", 27);
  });
  const rect = await paperRect(client);
  const a = point(rect, start[0], start[1]);
  const b = point(rect, end[0], end[1]);
  await client.click(a.x, a.y);
  if (leavePreview) {
    await client.mouse("mouseMoved", b.x, b.y, { buttons: 0 });
    await client.sleep(500);
  } else {
    await client.click(b.x, b.y);
    await client.sleep(800);
    await client.overlayHitTest(true);
  }
  return rect;
}

async function selectCrease(context, xFraction, yFraction) {
  const { client } = context;
  await clickExact(client, "[data-testid='tool-select']", "選択");
  const rect = await paperRect(client);
  const base = point(rect, xFraction, yFraction);
  for (const dx of [0, 1, -1, 2, -2, 3, -3, 4, -4]) {
    for (const dy of [0, 1, -1, 2, -2]) {
      await client.click(base.x + dx, base.y + dy);
      const selected = await client.evaluate(`(() => Array.from(document.querySelectorAll('input[type="range"]')).some((input) => (input.getAttribute("aria-label") || "").includes("の角度")))()`);
      if (selected) return;
    }
  }
  fail(`could not select a crease near ${xFraction},${yFraction}`);
}

async function setSelectedAngle(client, degrees) {
  const result = await client.evaluate(`(() => {
    const input = Array.from(document.querySelectorAll('input[type="range"]')).find((node) => (node.getAttribute("aria-label") || "").includes("の角度"));
    if (!(input instanceof HTMLInputElement)) return "not-found";
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    setter.call(input, ${JSON.stringify(String(degrees))});
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
    return input.value;
  })()`);
  if (Number(result) !== Number(degrees)) fail(`selected angle is ${result}, expected ${degrees}`);
  await client.stable();
  await requirePage(client, `document.querySelector('.context-panel')?.textContent?.includes(${JSON.stringify(`${degrees}°`)}) === true`, `angle label ${degrees}°`);
}

async function buildCross(context) {
  await freshPaper(context);
  await drawLine(context, "山", [0.01, 0.99], [0.99, 0.01]);
  await drawLine(context, "谷", [0.5, 0.5], [0.99, 0.5]);
  await drawLine(context, "山", [0.5, 0.5], [0.01, 0.5]);
  await drawLine(context, "谷", [0.5, 0.5], [0.5, 0.01]);
}

function captured(setup, { neutralMouse = true } = {}) {
  return async (context) => {
    await setup(context);
    await assertGeneration(context.client);
    if (neutralMouse) await context.client.neutralMouse();
    await context.capture();
  };
}

export async function bestEffortCleanup(label, attempts) {
  const errors = [];
  for (const attempt of attempts) {
    try {
      await attempt();
    } catch (error) {
      errors.push(error);
    }
  }
  if (errors.length > 0) throw new AggregateError(errors, label);
}

async function overviewGuide(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.crane, { step: 0 });
  await clickContains(client, ".toolbar button", "ヘルプ");
  await requirePage(client, `document.querySelector('.help-guide-entry button') !== null`, "help guide entry");
  await clickContains(client, ".help-guide-entry button", "基本操作ガイド");
  await requirePage(client, `(() => { const t = document.body.innerText; return t.includes("1 / 4") || t.includes("1/4"); })()`, "guide page 1/4");
}

async function workspace(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.crane, { step: 2 });
  await requirePage(client, `(() => [".tool-rail", ".cp-editor", ".pane-3d-view", ".context-panel"].every((s) => document.querySelector(s)))()`, "four workspace areas");
}

async function themeJapanese(context) {
  await resetBaseline(context, FIXTURES.yakko);
  await setTheme(context.client, "和風");
}

async function themeModern(context) {
  await resetBaseline(context, FIXTURES.bird);
  await setTheme(context.client, "モダン");
}

async function paneResize(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.yakko);
  // A prior interrupted capture may have left the persisted pane preference at
  // either drag destination.  Start this scenario from the product's reset
  // action so both before/after measurements prove this run moved the panes.
  await clickExact(client, ".context-panel button", "表示の広さを初期に戻す");
  const before = {
    leftRight: await client.center(".pane-splitter"),
    upperLower: await client.center(".context-panel-splitter"),
  };
  if (!before.leftRight || !before.upperLower) fail("pane splitters are missing before resize");
  await client.drag(before.leftRight.x, before.leftRight.y, 425, before.leftRight.y, 12);
  await client.drag(before.upperLower.x, before.upperLower.y, before.upperLower.x, 425, 12);
  const after = {
    leftRight: await client.center(".pane-splitter"),
    upperLower: await client.center(".context-panel-splitter"),
  };
  if (!after.leftRight || !after.upperLower) fail("pane splitters are missing after resize");
  const deltas = {
    leftRight: Math.abs(after.leftRight.x - before.leftRight.x),
    upperLower: Math.abs(after.upperLower.y - before.upperLower.y),
  };
  if (deltas.leftRight < 40 || deltas.upperLower < 40) {
    fail(`both pane splitters must move by at least 40 CSS px: ${JSON.stringify({ before, after, deltas })}`);
  }
}

async function paneReset(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.yakko);
  const splitter = await client.center(".pane-splitter");
  const lower = await client.center(".context-panel-splitter");
  if (!splitter || !lower) fail("pane splitters are missing");
  await client.drag(splitter.x, splitter.y, 420, splitter.y, 10);
  await client.drag(lower.x, lower.y, lower.x, 430, 10);
  await clickExact(client, ".context-panel button", "表示の広さを初期に戻す");
  const ratios = await client.evaluate(`(() => {
    const main = document.querySelector('.main-row')?.getBoundingClientRect();
    const cp = document.querySelector('.cp-editor')?.getBoundingClientRect();
    const context = document.querySelector('.context-panel')?.getBoundingClientRect();
    return main && cp && context ? { cp: cp.width / main.width, lower: context.height / innerHeight } : null;
  })()`);
  if (!ratios || Math.abs(ratios.cp - 0.5) > 0.04 || Math.abs(ratios.lower - 0.32) > 0.05) fail(`pane reset ratios are wrong: ${JSON.stringify(ratios)}`);
}

async function tooltipHover(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.crane);
  const button = await client.center(".toolbar button", "開く");
  if (!button) fail("Open toolbar button is missing");
  await client.hover(button.x, button.y);
  await requirePage(client, `(() => Array.from(document.querySelectorAll('[role="tooltip"], .tooltip')).some((n) => n.offsetParent !== null && (n.textContent || "").trim().length > 0))()`, "Open tooltip");
}

async function compactOperationHelp(context) {
  const { client, deferCleanup } = context;
  await resetBaseline(context, FIXTURES.bird);
  deferCleanup(async () => await client.lockMetrics());
  await client.setCompactMetrics();
  await client.sleep(700);
  await client.collapseHints();
  const toggle = await client.center(".viewer-hint-toggle");
  if (!toggle) fail("viewer operation help toggle is missing");
  await client.hover(toggle.x, toggle.y);
  await requirePage(client, `innerWidth === 768 && Math.abs(devicePixelRatio - (10 / 3)) < 0.01`, "compact 2560x1720 metrics");
  await requirePage(
    client,
    `(() => {
      const toggle = document.querySelector('.viewer-hint-toggle');
      const tooltip = document.getElementById('origami3-active-tooltip');
      if (!(toggle instanceof HTMLButtonElement) || !(tooltip instanceof HTMLElement)) return false;
      const expected = toggle.getAttribute('aria-expanded') === 'true'
        ? '案内をたたんで、3Dの紙を広く見ます'
        : 'モードの説明とマウス操作の割り当てを開きます';
      const describedBy = (toggle.getAttribute('aria-describedby') || '').split(/\\s+/);
      const style = getComputedStyle(tooltip);
      const rect = tooltip.getBoundingClientRect();
      return describedBy.includes('origami3-active-tooltip') &&
        tooltip.getAttribute('role') === 'tooltip' &&
        tooltip.textContent?.trim() === expected &&
        style.visibility === 'visible' && style.display !== 'none' &&
        rect.width > 0 && rect.height > 0 &&
        rect.left >= 0 && rect.top >= 0 && rect.right <= innerWidth && rect.bottom <= innerHeight;
    })()`,
    "compact Viewer3D operation-help tooltip with its own visible copy",
  );
}

async function newDialog(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.crane, { step: 0 });
  await clickContains(client, ".toolbar button", "新規");
  await requirePage(client, `(() => { const d = document.querySelector('[role="dialog"], .dialog'); return d && Array.from(d.querySelectorAll("button")).some((b) => (b.textContent || "").includes("この紙で作りはじめる")); })()`, "new-paper dialog");
}

async function paperColors(context) {
  const { client } = context;
  await freshPaper(context);
  await clickContains(client, ".context-panel button", "紙の色");
  await requirePage(client, `(() => { const s = document.querySelector('#paper-color-settings'); return s && s.offsetParent !== null && s.querySelectorAll('button').length >= 2; })()`, "paper color swatches");
}

async function colorPicker(context) {
  const { client } = context;
  await freshPaper(context);
  await clickContains(client, ".context-panel button", "紙の色");
  await clickContains(client, ".context-panel button", "その他の色", 0);
  await requirePage(client, `(() => { const d = document.querySelector('[role="dialog"], .dialog'); return d && d.querySelector('input[type="color"]') && /色/.test(d.textContent || ""); })()`, "custom color picker");
}

async function drawLinePreview(context) {
  const { client } = context;
  await freshPaper(context);
  await drawLine(context, "山", [0.5, 0.95], [0.5, 0.2], { leavePreview: true });

  // The operation-stage row reads the same lineInputStart state that drives
  // CpEditor's RenderOverlay.  Opening it momentarily distinguishes an
  // accepted first point from a click that merely left the mountain tool on.
  const detailToggle = await client.evaluate(`(() => {
    const button = document.querySelector('.operation-help-toggle');
    if (!(button instanceof HTMLButtonElement)) return "missing";
    if (button.getAttribute('aria-expanded') !== 'true') button.click();
    return "ok";
  })()`);
  if (detailToggle !== "ok") fail("line-preview operation-stage control is missing");
  await requirePage(
    client,
    `(() => {
      const current = document.querySelector('.operation-steps li[aria-current="step"]');
      return current && (current.textContent || '').includes('終点をクリック');
    })()`,
    "accepted line start (waiting for endpoint)",
  );
  await client.sleep(250);

  // Check the actual 2D canvas, not generic help prose.  A fresh paper has no
  // mountain crease; the pending vertical segment must therefore contribute
  // red mountain-preview pixels in its interior corridor.  Endpoints are
  // excluded so the hover/snap marker alone cannot satisfy this assertion.
  const previewPixels = await client.evaluate(`(() => {
    const canvas = document.querySelector('.cp-canvas, [data-testid="cp-canvas"]');
    if (!(canvas instanceof HTMLCanvasElement)) return null;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    if (!context) return null;
    const bounds = canvas.getBoundingClientRect();
    const paperSide = Math.min(bounds.width, bounds.height) * 0.8935;
    const paperLeft = (bounds.width - paperSide) / 2;
    const paperTop = (bounds.height - paperSide) / 2;
    const scaleX = canvas.width / bounds.width;
    const scaleY = canvas.height / bounds.height;
    const centerX = Math.round((paperLeft + paperSide * 0.5) * scaleX);
    const top = Math.round((paperTop + paperSide * 0.30) * scaleY);
    const bottom = Math.round((paperTop + paperSide * 0.85) * scaleY);
    const halfWidth = Math.max(3, Math.ceil(3 * scaleX));
    const image = context.getImageData(centerX - halfWidth, top, halfWidth * 2 + 1, Math.max(1, bottom - top));
    let mountainRed = 0;
    for (let offset = 0; offset < image.data.length; offset += 4) {
      const red = image.data[offset];
      const green = image.data[offset + 1];
      const blue = image.data[offset + 2];
      const alpha = image.data[offset + 3];
      if (alpha > 128 && red > 145 && red > green * 1.55 && red > blue * 1.35) mountainRed += 1;
    }
    return { mountainRed, sampledPixels: image.data.length / 4 };
  })()`);
  if (!previewPixels || previewPixels.mountainRed < 8) {
    fail(`pending mountain-line geometry is not visible in the CP canvas: ${JSON.stringify(previewPixels)}`);
  }
  // Restore the compact operation card without clearing the pending line.
  await client.evaluate(`(() => {
    const button = document.querySelector('.operation-help-toggle');
    if (button instanceof HTMLButtonElement && button.getAttribute('aria-expanded') === 'true') button.click();
    return true;
  })()`);
  await client.stable();
}

async function mirrorAxis(context) {
  const { client } = context;
  await freshPaper(context);
  await drawLine(context, "補助", [0.15, 0.9], [0.85, 0.1]);
  await clickExact(client, "[data-testid='tool-select']", "選択");
  await client.overlayHitTest(false);
  const rect = await paperRect(client);
  const onLine = point(rect, 0.36, 0.66);
  let selected = false;
  for (const dx of [0, 1, -1, 2, -2, 3, -3, 4, -4]) {
    for (const dy of [0, 2, -2]) {
      await client.click(onLine.x + dx, onLine.y + dy);
      selected = await client.evaluate(`(() => { const t = document.querySelector('.context-panel')?.textContent || ""; return t.includes("線を1本選択中") && t.includes("補助"); })()`);
      if (selected) break;
    }
    if (selected) break;
  }
  await client.overlayHitTest(true);
  if (!selected) fail("mirror-axis auxiliary line was not selected");
  await setCheckbox(client, "左右対称に描く", true);
  await clickExact(client, ".context-panel button", "この線を基準にする");
  await drawLine(context, "山", [0.25, 0.8], [0.4, 0.5], { leavePreview: true });
  await requirePage(client, `(() => { const t = document.querySelector('.context-panel')?.textContent || ""; return t.includes("左右対称") && t.includes("基準"); })()`, "mirror preview controls");
}

async function viewerPane(client) {
  const pane = await client.evaluate(`(() => {
    const node = document.querySelector('.pane-3d-view, [data-testid="viewer3d-canvas"]')?.closest('.pane-3d-view') || document.querySelector('.pane-3d-view');
    if (!node) return null;
    const r = node.getBoundingClientRect();
    return { x: r.left, y: r.top, w: r.width, h: r.height };
  })()`);
  if (!pane || pane.w < 100 || pane.h < 100) fail(`invalid 3D pane: ${JSON.stringify(pane)}`);
  return pane;
}

async function foldDrag(context) {
  const { client, deferCleanup } = context;
  await freshPaper(context);
  await clickExact(client, "[data-testid='tool-fold']", "折る");
  const pane = await viewerPane(client);
  const x = pane.x + pane.w * 0.62;
  const y = pane.y + pane.h * 0.66;
  const endX = x - 72;
  const endY = y - 60;
  deferCleanup(async () => {
    await client.releaseMouse(endX, endY);
    await client.key("Escape", "Escape", 27);
  });
  await client.mouse("mouseMoved", x, y, { buttons: 0 });
  await client.mouse("mousePressed", x, y, { buttons: 1 });
  for (let index = 1; index <= 12; index += 1) {
    await client.mouse("mouseMoved", x - index * 6, y - index * 5, { buttons: 1 });
    await client.sleep(45);
  }
  await requirePage(
    client,
    `(() => {
      const state = window.__origami3Capture.getInteractionState();
      const grab = state.viewer3d?.grab;
      const preview = state.viewer3d?.preview;
      return state.activeTool === 'fold' &&
        grab?.active === true && Number.isInteger(grab.face) &&
        ['single', 'flap', 'all'].includes(grab.mode) &&
        grab.selectedLayerCount > 0 && preview?.visible === true &&
        preview.segmentCount > 0 &&
        (grab.spatial === true || preview.polygonCount > 0);
    })()`,
    "3D fold drag hit with visible landing/fold-line preview",
  );
}

async function selectAnyCrease(context) {
  const { client } = context;
  await clickExact(client, "[data-testid='tool-select']", "選択");
  const rect = await paperRect(client);
  for (const y of [0.5, 0.35, 0.65, 0.25, 0.75]) {
    for (const x of [0.5, 0.35, 0.65, 0.2, 0.8]) {
      const at = point(rect, x, y);
      for (const offset of [0, 1, -1, 2, -2, 4, -4]) {
        await client.click(at.x + offset, at.y);
        const found = await client.evaluate(`(() => Array.from(document.querySelectorAll('input[type="range"]')).some((node) => (node.getAttribute("aria-label") || "").includes("の角度")))()`);
        if (found) return;
      }
    }
  }
  fail("no selectable crease was found in the paper grid");
}

async function freshVerticalCrease(context) {
  await freshPaper(context);
  await drawLine(context, "山", [0.5, 0.01], [0.5, 0.99]);
  await selectCrease(context, 0.5, 0.65);
}

async function angleSlider(context) {
  await freshVerticalCrease(context);
  await setSelectedAngle(context.client, -45);
}

async function pinCurrentAngle(client) {
  const result = await client.evaluate(`(() => {
    const button = document.querySelector('.pin-toggle') || Array.from(document.querySelectorAll('.context-panel button')).find((b) => (b.getAttribute('aria-label') || '').includes('角度を固定'));
    if (!(button instanceof HTMLButtonElement)) return "not-found";
    if (!button.classList.contains("pinned") && !(button.getAttribute("aria-label") || "").includes("固定を外す")) button.click();
    return "ok";
  })()`);
  if (result !== "ok") fail(`pin button failed: ${result}`);
  await requirePage(client, `(() => { const b = document.querySelector('.pin-toggle'); return b && (b.classList.contains("pinned") || (b.getAttribute("aria-label") || "").includes("固定を外す")); })()`, "pinned angle");
}

async function anglePin(context) {
  await freshVerticalCrease(context);
  await setSelectedAngle(context.client, 90);
  await pinCurrentAngle(context.client);
}

async function anglePinReleased(context) {
  const { client } = context;
  await buildCross(context);
  for (const [x, y, angle] of [
    [0.75, 0.25, 90],
    [0.75, 0.5, 120],
    [0.5, 0.25, -120],
  ]) {
    await selectCrease(context, x, y);
    await setSelectedAngle(client, angle);
    await pinCurrentAngle(client);
  }
  await selectCrease(context, 0.25, 0.5);
  await setSelectedAngle(client, 170);
  await requirePage(client, `(() => { const text = document.body.innerText; return /固定.*外|自動.*解除|固定を.*解除/.test(text); })()`, "automatic angle-pin release", 30_000);
}

async function naturalState(context, stage) {
  const { client } = context;
  await buildCross(context);
  await selectCrease(context, 0.75, 0.25);
  await setSelectedAngle(client, 90);
  await selectCrease(context, 0.75, 0.5);
  await setSelectedAngle(client, 150);
  if (stage >= 2) {
    await selectCrease(context, 0.5, 0.25);
    await setSelectedAngle(client, -160);
  }
  if (stage >= 3) {
    await selectCrease(context, 0.25, 0.5);
    await setSelectedAngle(client, 170);
  }
  const expected = stage === 1 ? /追従|連動/ : stage === 2 ? /ほか|件|追従/ : /届かな|近い形|同時には/;
  await requirePage(client, `(() => ${expected.toString()}.test(document.body.innerText))()`, `natural-follow stage ${stage}`, 30_000);
}

async function flatResetBefore(context) {
  await resetBaseline(context, FIXTURES.crane, { step: "latest" });
  await selectAnyCrease(context);
  await setSelectedAngle(context.client, 65);
  await requirePage(context.client, `(() => window.__origami3Capture.getDocumentInfo().stepCount >= 3)()`, "recorded timeline before flat reset");
}

async function flatResetAfter(context) {
  const { client } = context;
  await flatResetBefore(context);
  await clickContains(client, ".context-panel button", "全て平らに戻す");
  await client.stable();
  await requirePage(client, `(() => { const s = window.__origami3Capture.getInteractionState(); return s.currentStep >= 3 && !document.body.innerText.includes("65°"); })()`, "flat reset with timeline retained");
}

async function angle90(context) {
  await resetBaseline(context, FIXTURES.crane, { step: 0 });
  await selectAnyCrease(context);
  await setSelectedAngle(context.client, 90);
  await clickExact(context.client, "[data-testid='tool-fit']", "全体");
}

async function preventionSettings(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.yakko);
  await requirePage(client, `(() => { const panel = document.querySelector('.context-panel'); return panel && Array.from(panel.querySelectorAll('input[type="checkbox"]')).some((i) => i.getAttribute("aria-label") === "重なり防止") && Array.from(panel.querySelectorAll('input[type="checkbox"]')).some((i) => i.getAttribute("aria-label") === "食い込み検出"); })()`, "penetration prevention settings");
}

async function paperInflate(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.bird);
  await setCheckbox(client, "丸みをつける", true);
  await requirePage(client, `(() => { const section = document.querySelector('.soft-controls'); return section && section.querySelectorAll('input[type="range"]').length >= 2 && section.textContent.includes("紙をふくらませる"); })()`, "paper inflate controls");
}

async function flatCompleteNoWarning(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.bird, { step: "latest" });
  await requirePage(client, `(() => document.querySelector('.self-intersection-guide') === null && !Array.from(document.querySelectorAll('.status-badge')).some((n) => /突き抜け|食い込み/.test(n.textContent || "")))()`, "flat fold without penetration warning");
}

async function exactPaperRect(client) {
  const rect = await client.evaluate(`(() => { const e = document.querySelector('.cp-canvas, [data-testid="cp-canvas"]'); if (!e) return null; const r = e.getBoundingClientRect(); return { x:r.left, y:r.top, w:r.width, h:r.height }; })()`);
  if (!rect) fail("CP canvas is missing");
  const scale = Math.min(rect.w, rect.h) * 0.9;
  return { x: rect.x + (rect.w - scale) / 2, y: rect.y + (rect.h - scale) / 2, w: scale, h: scale };
}

async function measurementPickCount(client) {
  const count = await client.evaluate(`(() => { const t = document.querySelector('.measure-progress')?.textContent || ""; const m = t.match(/選択\\s*(\\d+)\\s*\\//); return m ? Number(m[1]) : -1; })()`);
  return Number(count);
}

async function clickUntilMeasurement(client, x, y, before, label) {
  for (const dx of [0, 1, -1, 2, -2, 3, -3, 5, -5, 8, -8]) {
    for (const dy of [0, 1, -1, 2, -2, 3, -3, 5, -5, 8, -8]) {
      await client.click(x + dx, y + dy);
      if ((await measurementPickCount(client)) > before) return;
    }
  }
  fail(`${label} was not accepted as a measurement pick`);
}

async function measureAngleResult(context) {
  const { client } = context;
  await freshPaper(context);
  const rect = await exactPaperRect(client);
  const p = (x, y) => point(rect, x, y);
  await clickExact(client, "[data-testid='tool-construct']", "作図");
  await clickExact(client, "[data-testid='construct-bisector']", "二等分");
  await client.overlayHitTest(false);
  for (const at of [p(1, 1), p(0, 1), p(1, 0)]) await client.click(at.x, at.y);
  await client.overlayHitTest(true);
  await clickExact(client, "[data-testid='tool-measure']", "測る");
  let before = await measurementPickCount(client);
  await clickUntilMeasurement(client, p(0.5, 1).x, p(0.5, 1).y, before, "bottom edge");
  before = await measurementPickCount(client);
  const onBisector = p(0.3, 1 - 0.3 * Math.tan((22.5 * Math.PI) / 180));
  await clickUntilMeasurement(client, onBisector.x, onBisector.y, before, "22.5 degree bisector");
  await requirePage(client, `(() => { const card = document.querySelector('.measure-result-card'); return card && card.textContent.includes("22.5°"); })()`, "exact 22.5-degree result");
}

async function measureDistanceResult(context) {
  const { client } = context;
  await freshPaper(context);
  await clickExact(client, "[data-testid='tool-measure']", "測る");
  await clickExact(client, ".measure-mode-buttons button", "2点の距離");
  const rect = await exactPaperRect(client);
  const first = point(rect, 0, 1);
  const second = point(rect, 1, 0);
  await client.overlayHitTest(false);
  await client.click(first.x, first.y);
  await client.click(second.x, second.y);
  await client.overlayHitTest(true);
  await requirePage(client, `(() => { const card = document.querySelector('.measure-result-card'); return card && card.textContent.includes("展開図での距離") && /212(\\.1)?|150√2/.test(card.textContent || ""); })()`, "paper diagonal distance result");
}

async function techniques(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.bird);
  await clickExact(client, "[data-testid='tool-technique']", "技法");
  await client.evaluate(`(() => { const rail = document.querySelector('.tool-rail'); if (rail) rail.scrollTop = rail.scrollHeight; return true; })()`);
  await requirePage(client, `(() => { const menu = document.querySelector('[data-testid="technique-menu"]'); return menu && menu.querySelectorAll('button').length === 9 && Array.from(menu.querySelectorAll('button')).every((b) => b.offsetParent !== null); })()`, "all nine technique entries");
}

async function clickLabelInput(client, text, type) {
  const result = await client.evaluate(`(() => {
    const wanted = ${JSON.stringify(text.replace(/\s+/g, ""))};
    const label = Array.from(document.querySelectorAll("label"))
      .find((node) => (node.textContent || "").replace(/\\s+/g, "").includes(wanted));
    const input = label?.querySelector(${JSON.stringify(`input[type="${type}"]`)});
    if (!(input instanceof HTMLInputElement)) return "not-found";
    if (!input.checked) input.click();
    return input.checked ? "ok" : "not-checked";
  })()`);
  if (result !== "ok") fail(`${type} label ${text} could not be selected: ${result}`);
  await client.sleep(300);
}

async function enterLayerMotion(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.bird, { step: "latest" });
  await clickExact(client, "[data-testid='tool-technique']", "技法");
  await clickExact(client, "[data-testid='technique-simple']", "層");
  await requirePage(
    client,
    `(() => (document.querySelector('.context-panel')?.textContent || '').includes('層操作:'))()`,
    "layer-motion controls",
  );
}

async function pickFourLayerCandidates(client) {
  const pane = await viewerPane(client);
  const xMin = Math.round(pane.x + 24);
  const xMax = Math.round(pane.x + pane.w - 24);
  const yMin = Math.round(pane.y + 80);
  const yMax = Math.round(pane.y + pane.h - 24);
  let best = null;
  for (let y = yMin; y <= yMax; y += 20) {
    for (let x = xMin; x <= xMax; x += 26) {
      await client.click(x, y);
      const state = await client.evaluate(`(() => {
        const text = (document.querySelector('.context-panel')?.textContent || '').replace(/\\s+/g, ' ');
        const match = text.match(/候補\\s*(\\d+)\\s*枚/);
        return match ? { count: Number(match[1]), text: text.slice(0, 360) } : null;
      })()`);
      if (state?.count === 4) return { x, y, state };
      if (state?.count > 0 && (best === null || state.count > best.state.count)) {
        best = { x, y, state };
      }
    }
  }
  fail(`the bird-base scan found no point with exactly four layer candidates; best=${JSON.stringify(best)}`);
}

async function raiseContextPanel(client, targetY = 400) {
  const splitter = await client.center(".context-panel-splitter");
  if (!splitter) fail("context-panel splitter is missing");
  await client.drag(splitter.x, splitter.y, splitter.x, targetY, 12);
}

async function layerMotionOpenClose(context) {
  const { client } = context;
  await enterLayerMotion(context);
  await pickFourLayerCandidates(client);
  await clickExact(client, ".context-panel button", "手前から1枚");
  await clickLabelInput(client, "既存折り目で開閉", "radio");
  await requirePage(
    client,
    `(() => {
      const text = (document.querySelector('.context-panel')?.textContent || '').replace(/\\s+/g, '');
      return text.includes('候補4枚') && text.includes('選択1枚') && text.includes('軸:未選択');
    })()`,
    "four candidates, one selected layer, and no reflect axis",
  );
  await raiseContextPanel(client);
}

async function layerMotionRestack(context) {
  const { client } = context;
  await enterLayerMotion(context);
  await pickFourLayerCandidates(client);
  await clickExact(client, ".context-panel button", "手前から1枚");
  await clickLabelInput(client, "動かさず重ね替え", "radio");
  const selected = await client.evaluate(`(() => {
    const select = document.querySelector('#layer-motion-turn');
    return select instanceof HTMLSelectElement ? select.selectedOptions[0]?.textContent?.trim() : null;
  })()`);
  if (selected !== "位置を保つ") fail(`restack position must be Keep, got ${JSON.stringify(selected)}`);
  await requirePage(
    client,
    `(() => {
      const text = (document.querySelector('.context-panel')?.textContent || '').replace(/\\s+/g, '');
      return text.includes('候補4枚') && text.includes('選択1枚') && text.includes('動かさず重ね替え');
    })()`,
    "four candidates and one selected restack layer",
  );
  await raiseContextPanel(client);
}

async function selectTimelineEntry(context, selector, expectedStep) {
  const { client } = context;
  const info = await resetBaseline(context, FIXTURES.crane, { step: "latest" });
  if (info?.stepCount < 3) fail(`the crane timeline has only ${info?.stepCount ?? "unknown"} steps`);
  const result = await client.evaluate(`(() => {
    const button = document.querySelector(${JSON.stringify(selector)});
    if (!(button instanceof HTMLButtonElement) || button.disabled) return "not-found-or-disabled";
    button.click();
    return "ok";
  })()`);
  if (result !== "ok") fail(`timeline selector ${selector} failed: ${result}`);
  await client.stable();
  await requirePage(
    client,
    `(() => {
      const button = document.querySelector(${JSON.stringify(selector)});
      const state = window.__origami3Capture.getInteractionState();
      return button?.classList.contains('selected') === true && ${
        expectedStep === null
          ? "state.currentStep === null"
          : `state.currentStep === ${Number(expectedStep)}`
      };
    })()`,
    `selected timeline entry ${selector}`,
  );
}

async function timeline(context) {
  await selectTimelineEntry(context, "[data-testid='timeline-step-2']", 2);
}

async function cpHistoryStep1(context) {
  await selectTimelineEntry(context, "[data-testid='timeline-step-1']", 1);
}

async function cpHistoryLatest(context) {
  await selectTimelineEntry(context, "[data-testid='timeline-step-latest']", null);
}

async function proposalWizard(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.bird, { step: "latest" });
  await clickContains(client, ".toolbar button", "提案");
  await requirePage(client, `document.querySelectorAll('[data-tip-handle]').length >= 3`, "proposal tip handles");
  const handle = await client.evaluate(`(() => {
    const nodes = Array.from(document.querySelectorAll('[data-tip-handle]'));
    const node = nodes[nodes.length - 1];
    if (!node) return null;
    const rect = node.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2, count: nodes.length };
  })()`);
  if (!handle) fail("proposal tip handle is missing");
  await client.drag(handle.x, handle.y, handle.x + 28, handle.y - 34, 14);
  await requirePage(
    client,
    `(() => {
      const dialog = document.querySelector('[data-floating-ui="proposal-dialog"], [role="dialog"]');
      return dialog && (dialog.textContent || '').includes('展開図を作ってもらう');
    })()`,
    "proposal skeleton editor",
  );
}

async function exportDialog(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.crane, { step: "latest" });
  await clickContains(client, ".toolbar button", "書き出し");
  await requirePage(
    client,
    `(() => {
      const dialog = document.querySelector('[data-floating-ui="export-dialog"], [role="dialog"], .dialog');
      return dialog && /PDF|SVG/.test(dialog.textContent || '') && /書き出し|保存|ダウンロード/.test(dialog.textContent || '');
    })()`,
    "export dialog",
  );
}

async function warning(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.penetration, { step: "latest" });
  await requirePage(
    client,
    `(() => {
      const badge = document.querySelector('.self-intersection-guide');
      return badge && /めり込み|突き抜け|食い込み/.test(badge.textContent || '') && /面/.test(badge.textContent || '');
    })()`,
    "self-intersection face-pair warning",
    30_000,
  );
}

async function helpCenter(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.crane, { step: 0 });
  await clickContains(client, ".toolbar button", "ヘルプ");
  await requirePage(
    client,
    `(() => {
      const root = document.querySelector('.help-center, [data-floating-ui="help-dialog"], [role="dialog"]');
      if (!root) return false;
      const chapterButtons = Array.from(root.querySelectorAll('button')).filter((button) => /^\\s*\\d+/.test(button.textContent || ''));
      return chapterButtons.length === 13;
    })()`,
    "13-chapter help center",
  );
}

async function foldAllSlider(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.crane, { step: 0 });
  await clickExact(client, ".context-panel button", "◇ 全部いっぺんに折ってみる");
  await requirePage(client, `document.querySelector('[data-fold-all-active="true"]') !== null`, "fold-all preview");
  await setRangeElement(client, "#fold-all-percent", 50, true);
  await requirePage(
    client,
    `(() => {
      const root = document.querySelector('[data-fold-all-active="true"]');
      const slider = document.querySelector('#fold-all-percent');
      const output = root?.querySelector('output');
      return root && slider?.value === '50' && output?.textContent?.trim() === '50%' &&
        (root.textContent || '').includes('これは仮の形です') &&
        (root.textContent || '').includes('元に戻る 0%') &&
        (root.textContent || '').includes('できるところまで 100%') &&
        root.getAttribute('data-returning') !== 'true';
    })()`,
    "fold-all 50 percent preview",
    30_000,
  );
}

async function proposalProgress(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.bird, { step: "latest" });
  await clickContains(client, ".toolbar button", "提案");
  await requirePage(client, `document.querySelector('[aria-label^="出っぱり"][aria-label$="本の並び"]') !== null`, "proposal skeleton list");

  for (;;) {
    const count = await client.evaluate(`(() => {
      const list = document.querySelector('[aria-label^="出っぱり"][aria-label$="本の並び"]');
      const match = list?.getAttribute('aria-label')?.match(/出っぱり(\\d+)本/);
      return match ? Number(match[1]) : -1;
    })()`);
    if (count === 12) break;
    if (!Number.isInteger(count) || count < 1 || count > 12) fail(`invalid proposal leaf count: ${count}`);
    const added = await client.evaluate(`(() => {
      const button = Array.from(document.querySelectorAll('[role="dialog"] button, .dialog button'))
        .find((node) => (node.textContent || '').trim() === '出っぱりを増やす');
      if (!(button instanceof HTMLButtonElement) || button.disabled) return false;
      button.click();
      return true;
    })()`);
    if (!added) fail(`proposal could not add leaf ${count + 1}`);
    await client.sleep(80);
  }

  await client.neutralMouse();
  const started = await client.evaluate(`(() => {
    const button = Array.from(document.querySelectorAll('[role="dialog"] button, .dialog button'))
      .find((node) => (node.textContent || '').trim() === '展開図を作ってもらう');
    if (!(button instanceof HTMLButtonElement) || button.disabled) return false;
    button.click();
    return true;
  })()`);
  if (!started) fail("proposal generation could not be started");
  await requirePage(
    client,
    `(() => {
      const progress = document.querySelector('[data-proposal-progress]');
      const count = document.querySelector('[data-proposal-progress-count]')?.textContent || '';
      return progress && /\\d+件中\\d+件め/.test(count) && !count.includes('準備中');
    })()`,
    "proposal progress with completed/total count",
    15_000,
  );
}

async function viewer3dLoading(context) {
  const { client, deferCleanup } = context;
  await resetBaseline(context, FIXTURES.crane, { step: 2 });
  const source = `(() => {
    window.requestIdleCallback = (callback) => window.setTimeout(
      () => callback({ didTimeout: true, timeRemaining: () => 0 }),
      15000,
    );
    window.cancelIdleCallback = (id) => window.clearTimeout(id);
  })();`;
  const { identifier } = await client.call("Page.addScriptToEvaluateOnNewDocument", { source });
  if (!identifier) fail("Page.addScriptToEvaluateOnNewDocument returned no identifier");
  client.trackPreloadScript(identifier);
  deferCleanup(async () => {
    await bestEffortCleanup("delayed Viewer3D cleanup failed", [
      () => client.removeTrackedPreloadScript(identifier),
      () => client.reload(),
      () => rebind(client),
      () => requirePage(client, `document.querySelector('canvas.viewer3d-canvas[aria-label="3D表示"]') !== null`, "normal Viewer3D after loading cleanup", 30_000),
    ]);
  });
  await client.reload();
  await rebind(client);
  await requirePage(
    client,
    `(() => {
      const loading = document.querySelector('[data-testid="viewer3d-loading"][aria-busy="true"]');
      return loading && (loading.textContent || '').includes('3D表示を準備しています');
    })()`,
    "delayed Viewer3D loading fallback",
    8_000,
  );
}

async function viewer3dLoadError(context) {
  const { client, deferCleanup } = context;
  await resetBaseline(context, FIXTURES.crane, { step: 2 });
  await client.call("Network.enable");
  await client.call("Network.setCacheDisabled", { cacheDisabled: true });
  await client.call("Network.clearBrowserCache");
  await client.call("Network.setBlockedURLs", {
    urls: [
      "*://tauri.localhost/assets/Viewer3D-*.js*",
      "*/src/components/Viewer3D/Viewer3D.tsx*",
    ],
  });
  deferCleanup(async () => {
    await bestEffortCleanup("blocked Viewer3D cleanup failed", [
      () => client.call("Network.setBlockedURLs", { urls: [] }),
      () => client.call("Network.clearBrowserCache"),
      () => client.call("Network.setCacheDisabled", { cacheDisabled: false }),
      () => client.reload(),
      () => rebind(client),
      () => requirePage(client, `document.querySelector('canvas.viewer3d-canvas[aria-label="3D表示"]') !== null`, "normal Viewer3D after error cleanup", 30_000),
      () => client.call("Network.disable"),
    ]);
  });
  await client.reload();
  await rebind(client);
  await requirePage(
    client,
    `(() => {
      const error = document.querySelector('[data-testid="viewer3d-load-error"][role="alert"]');
      const retry = Array.from(error?.querySelectorAll('button') || []).find((button) => (button.textContent || '').trim() === '3D表示を再試行');
      return error && (error.textContent || '').includes('3D表示を読み込めませんでした。2Dの編集は続けられます。') && retry;
    })()`,
    "Viewer3D load-error fallback",
    20_000,
  );
}

async function alignDraftProgress(client) {
  return await client.evaluate(`(() => {
    const el = document.querySelector('.align-draft-progress');
    if (!el) return null;
    const match = (el.textContent || '').match(/選択\\s*(\\d+)\\s*\\/\\s*(\\d+)/);
    return match ? { picked: Number(match[1]), need: Number(match[2]) } : null;
  })()`);
}

async function alignFailureReason(client) {
  return await client.evaluate(`(() => document.querySelector('.warning-text')?.textContent ?? null)()`);
}

async function undoLastAlignPick(client) {
  await clickExact(client, ".button-row button", "1つ戻す");
}

async function resetAlignDraftToZero(client) {
  for (let guard = 0; guard < 4; guard += 1) {
    const progress = await alignDraftProgress(client);
    if (!progress || progress.picked === 0) return;
    await undoLastAlignPick(client);
  }
  fail("could not reset the align draft back to zero picks between attempts");
}

/**
 * 「線と線を合わせる」で拾える線を3Dペイン全体から1回だけ走査し、当たった画面座標を集める。
 * 当たるたびに直ちに「1つ戻す」で選択0へ戻し、本番の組み合わせ試行を汚さない。
 */
async function collectAlignLineCandidates(client, pane) {
  const xMin = Math.round(pane.x + 24);
  const xMax = Math.round(pane.x + pane.w - 24);
  const yMin = Math.round(pane.y + 70);
  const yMax = Math.round(pane.y + pane.h - 24);
  const candidates = [];
  for (let y = yMin; y <= yMax; y += 16) {
    for (let x = xMin; x <= xMax; x += 16) {
      await client.click(x, y);
      const progress = await alignDraftProgress(client);
      if (progress?.picked === 1) {
        candidates.push({ x, y });
        await undoLastAlignPick(client);
        const back = await alignDraftProgress(client);
        if (back?.picked !== 0) {
          fail(`could not undo the trial align pick at ${x},${y}: ${JSON.stringify(back)}`);
        }
      } else if (progress && progress.picked !== 0) {
        fail(`unexpected align pick count while scanning for lines: ${JSON.stringify(progress)}`);
      }
    }
  }
  if (candidates.length < 2) {
    fail(`found only ${candidates.length} pickable line(s) on the folded crane; "線と線を合わせる" needs at least 2`);
  }
  return candidates;
}

/** 「重なりのある折り目と辺」を想定し、画面上で近い組から試す。 */
function alignPairsByProximity(candidates) {
  const pairs = [];
  for (let firstIndex = 0; firstIndex < candidates.length; firstIndex += 1) {
    for (let secondIndex = 0; secondIndex < candidates.length; secondIndex += 1) {
      if (firstIndex === secondIndex) continue;
      const first = candidates[firstIndex];
      const second = candidates[secondIndex];
      const distance = Math.hypot(first.x - second.x, first.y - second.y);
      pairs.push({ first, second, distance });
    }
  }
  pairs.sort((left, right) => left.distance - right.distance);
  return pairs;
}

async function foldPleatTarget(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.crane, { step: "latest" });
  await clickExact(client, "[data-testid='tool-fold']", "折る");
  await requirePage(client, `document.querySelector('.align-mode-buttons') !== null`, "align-fold mode buttons");
  await clickExact(client, ".align-mode-buttons button", "線と線を合わせる");
  await requirePage(client, `document.querySelector('.align-draft-progress') !== null`, "line-to-line align draft started");
  const start = await alignDraftProgress(client);
  if (!start || start.picked !== 0 || start.need !== 2) {
    fail(`unexpected initial align progress: ${JSON.stringify(start)}`);
  }

  const pane = await viewerPane(client);
  const candidates = await collectAlignLineCandidates(client, pane);
  const pairs = alignPairsByProximity(candidates).slice(0, 80);

  let solved = false;
  for (const pair of pairs) {
    await client.click(pair.first.x, pair.first.y);
    const afterFirst = await alignDraftProgress(client);
    if (afterFirst?.picked !== 1) {
      await resetAlignDraftToZero(client);
      continue;
    }
    await client.click(pair.second.x, pair.second.y);
    const afterSecond = await alignDraftProgress(client);
    if (afterSecond?.picked === 2) {
      const reason = await alignFailureReason(client);
      const hasFoldTarget = await client.evaluate(
        `document.querySelector('[role="group"][aria-label="折る紙"]') !== null`,
      );
      if (reason === null && hasFoldTarget) {
        solved = true;
        break;
      }
    }
    await resetAlignDraftToZero(client);
  }
  if (!solved) {
    fail(`no pair among ${pairs.length} candidate line combinations produced a valid fold line with 折る紙 controls`);
  }

  await requirePage(
    client,
    `(() => {
      const input = Array.from(document.querySelectorAll('input[type="radio"][name="fold-target"]')).find((node) => (node.getAttribute('aria-label') || '').startsWith('上から'));
      return input instanceof HTMLInputElement && !input.disabled;
    })()`,
    "pleat-target radio ready (fold-target info resolved)",
    15_000,
  );
  const selectedTop = await client.evaluate(`(() => {
    const input = Array.from(document.querySelectorAll('input[type="radio"][name="fold-target"]')).find((node) => (node.getAttribute('aria-label') || '').startsWith('上から'));
    if (!(input instanceof HTMLInputElement)) return "not-found";
    input.click();
    return input.checked ? "ok" : "not-checked";
  })()`);
  if (selectedTop !== "ok") fail(`could not select the "上から" pleat-target radio: ${selectedTop}`);
  await client.stable();
  const incremented = await client.evaluate(`(() => {
    const button = document.querySelector('button[aria-label="同時に折るひだの枚数を増やす"]');
    if (!(button instanceof HTMLButtonElement) || button.disabled) return "not-found-or-disabled";
    button.click();
    return "ok";
  })()`);
  if (incremented !== "ok") fail(`could not increase the pleat count to 2: ${incremented}`);
  await client.stable();
  await requirePage(
    client,
    `(() => {
      const input = document.querySelector('input[type="number"][aria-label="同時に折るひだの枚数"]');
      const text = document.querySelector('.context-panel')?.textContent || '';
      return input instanceof HTMLInputElement && input.value === "2" && text.includes("上から2枚のひだを同時に折ります。");
    })()`,
    "pleat count set to 2 with the matching hint",
    15_000,
  );
}

async function selfIntersectionPairs(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.penetration, { step: "latest" });
  await setCheckbox(client, "食い込み検出", true);
  await requirePage(
    client,
    `(() => /紙のめり込み\\s*\\d+組（1\\/\\d+、Face ID/.test(document.querySelector('.self-intersection-guide')?.textContent || ''))()`,
    "self-intersection guide badge with the first pair",
    30_000,
  );
  const total = await client.evaluate(`(() => {
    const match = (document.querySelector('.self-intersection-guide')?.textContent || '').match(/紙のめり込み\\s*(\\d+)組/);
    return match ? Number(match[1]) : 0;
  })()`);
  if (!Number.isInteger(total) || total < 1) {
    fail(`self-intersection guide badge has no readable pair count: ${total}`);
  }
  // 「札を1回押して1組目を表示」の指示どおり札を押しつつ、押した回数ぶんだけ
  // 循環させて最終的に1組目(index 0)へ戻す。組数は実測するまで分からないため
  // 固定値を仮定しない。
  for (let step = 1; step <= total; step += 1) {
    const badge = await client.center(".self-intersection-guide");
    if (!badge) fail("self-intersection guide badge disappeared mid-cycle");
    await client.click(badge.x, badge.y);
    await client.stable();
  }
  const backToFirst = `（1/${total}、Face ID`;
  await requirePage(
    client,
    `(() => (document.querySelector('.self-intersection-guide')?.textContent || '').includes(${JSON.stringify(backToFirst)}))()`,
    `self-intersection guide back on the first of ${total} pairs`,
    10_000,
  );
}

async function exportFoldFile(context) {
  const { client } = context;
  await resetBaseline(context, FIXTURES.crane, { step: "latest" });
  await clickContains(client, ".toolbar button", "書き出し");
  await requirePage(client, `document.querySelector('[data-floating-ui="export-dialog"]') !== null`, "export dialog open");
  await clickExact(client, '[data-floating-ui="export-dialog"] label', "ほかの折り紙ソフトのファイル");
  await requirePage(
    client,
    `(() => {
      const dialog = document.querySelector('[data-floating-ui="export-dialog"]');
      const text = dialog?.textContent || '';
      return dialog !== null && text.includes('でそのまま扱えない内容（7項目）');
    })()`,
    "fold-file export choice with the seven-item unsupported-content list",
  );
}

/** SYS-003自動保存索引が使うTauriアプリデータの識別子(tauri.conf.json:5と同じ)。 */
const APP_DATA_IDENTIFIER = "com.oltot.origami3";

function resolveAppDataDir() {
  if (process.platform !== "win32") {
    fail(`recovery-choices fixture setup only supports win32 app-data resolution; platform=${process.platform}`);
  }
  const roaming = process.env.APPDATA;
  if (!roaming) fail("the APPDATA environment variable is not set; cannot locate the Tauri app-data directory");
  return path.join(roaming, APP_DATA_IDENTIFIER);
}

async function readFileIfExists(filePath) {
  try {
    return await fs.readFile(filePath);
  } catch (error) {
    if (error?.code === "ENOENT") return null;
    throw error;
  }
}

async function readdirIfExists(directoryPath) {
  try {
    return await fs.readdir(directoryPath);
  } catch (error) {
    if (error?.code === "ENOENT") return [];
    throw error;
  }
}

/**
 * 復旧ダイアログの複数候補を、実際のTauriアプリデータへ持ち越し候補として置き、
 * ページ再読み込みでApp起動時のcheckRecovery()に自然に拾わせる。
 * `apps/desktop/tests-live/doc-link-b1-recovery-cdp.mjs` の控え作成手順を参考にし、
 * 復旧候補の索引(`autosave-index.json`、version 2)と候補payload
 * (`autosave-recovery/<id>.ori3`)を直接書く。desktop.exeの再起動は行わない
 * (`prepare_session`はTauri起動ごとに1回だけ走るため、レガシー単一候補の目印
 * 形式では複数候補を再現できない)。既存の索引・候補は書く前に控え、
 * deferCleanupで元へ戻す。
 */
async function recoveryChoices(context) {
  const { client, repositoryRoot, deferCleanup } = context;
  const appData = resolveAppDataDir();
  const indexPath = path.join(appData, "autosave-index.json");
  const candidatesDir = path.join(appData, "autosave-recovery");

  await fs.mkdir(candidatesDir, { recursive: true });
  const originalIndexBytes = await readFileIfExists(indexPath);
  const existingCandidateIds = (await readdirIfExists(candidatesDir))
    .filter((name) => /^[1-9]\d*\.ori3$/.test(name))
    .map((name) => Number(name.slice(0, -".ori3".length)));
  const baseId = existingCandidateIds.length > 0 ? Math.max(...existingCandidateIds) + 1 : 1;
  const candidateIds = [baseId, baseId + 1];

  const sources = [
    { fixture: FIXTURES.crane, id: candidateIds[0], agoMs: 30 * 60 * 1000 },
    { fixture: FIXTURES.yakko, id: candidateIds[1], agoMs: 3 * 60 * 60 * 1000 },
  ];
  const addedFiles = [];
  const now = Date.now();
  const carried = [];
  for (const source of sources) {
    const fixturePath = path.resolve(repositoryRoot, source.fixture);
    const bytes = await fs.readFile(fixturePath);
    let stepCount = 0;
    try {
      const parsed = JSON.parse(bytes.toString("utf8"));
      stepCount = Array.isArray(parsed?.sequence) ? parsed.sequence.length : 0;
    } catch (error) {
      fail(`recovery payload fixture is not valid JSON: ${fixturePath}: ${error?.message ?? error}`);
    }
    const destination = path.join(candidatesDir, `${source.id}.ori3`);
    await fs.writeFile(destination, bytes);
    addedFiles.push(destination);
    carried.push({
      id: source.id,
      session_id: null,
      document_path: null,
      saved_at_ms: now - source.agoMs,
      step_count: stepCount,
    });
  }
  const index = {
    version: 2,
    next_candidate_id: candidateIds[1] + 1,
    active: null,
    carried,
  };
  await fs.writeFile(indexPath, `${JSON.stringify(index, null, 2)}\n`, "utf8");

  deferCleanup(async () => {
    await bestEffortCleanup("recovery-choices fixture cleanup failed", [
      async () => {
        for (const file of addedFiles) await fs.rm(file, { force: true });
      },
      async () => {
        if (originalIndexBytes === null) await fs.rm(indexPath, { force: true });
        else await fs.writeFile(indexPath, originalIndexBytes);
      },
      () => client.reload(),
      () => rebind(client),
    ]);
  });

  await client.reload();
  await rebind(client);
  await requirePage(
    client,
    `(() => {
      const dialog = document.querySelector('[data-floating-ui="recovery-dialog"]');
      if (!dialog) return false;
      const text = dialog.textContent || '';
      return text.includes('前回の終了が正常に行われませんでした') &&
        text.includes('作業中だった内容が2件残っています。内容ごとに選べます。') &&
        Array.from(dialog.querySelectorAll('button')).some((b) => (b.textContent || '').trim() === 'あとで確認する');
    })()`,
    "recovery dialog with two carried candidates",
    20_000,
  );
}

export function createScenarioRegistry() {
  return [
    ["overviewGuide", captured(overviewGuide)],
    ["workspace", captured(workspace)],
    ["themeJapanese", captured(themeJapanese)],
    ["themeModern", captured(themeModern)],
    ["paneResize", captured(paneResize)],
    ["paneReset", captured(paneReset)],
    ["tooltipHover", captured(tooltipHover, { neutralMouse: false })],
    ["compactOperationHelp", captured(compactOperationHelp, { neutralMouse: false })],
    ["newDialog", captured(newDialog)],
    ["paperColors", captured(paperColors)],
    ["colorPicker", captured(colorPicker)],
    ["drawLine", captured(drawLinePreview, { neutralMouse: false })],
    ["mirrorAxis", captured(mirrorAxis, { neutralMouse: false })],
    ["foldDrag", captured(foldDrag, { neutralMouse: false })],
    ["angleSlider", captured(angleSlider)],
    ["anglePin", captured(anglePin)],
    ["anglePinReleased", captured(anglePinReleased)],
    ["naturalFollow", captured((context) => naturalState(context, 1))],
    ["naturalFollowOverflow", captured((context) => naturalState(context, 2))],
    ["naturalFollowBestEffort", captured((context) => naturalState(context, 3))],
    ["flatResetBefore", captured(flatResetBefore)],
    ["flatResetAfter", captured(flatResetAfter)],
    ["angle90", captured(angle90)],
    ["preventionSettings", captured(preventionSettings)],
    ["paperInflate", captured(paperInflate)],
    ["flatCompleteNoWarning", captured(flatCompleteNoWarning)],
    ["measureAngleResult", captured(measureAngleResult)],
    ["measureDistanceResult", captured(measureDistanceResult)],
    ["techniques", captured(techniques, { neutralMouse: false })],
    ["layerMotionOpenClose", captured(layerMotionOpenClose)],
    ["layerMotionRestack", captured(layerMotionRestack)],
    ["timeline", captured(timeline)],
    ["cpHistoryStep1", captured(cpHistoryStep1)],
    ["cpHistoryLatest", captured(cpHistoryLatest)],
    ["proposalWizard", captured(proposalWizard)],
    ["exportDialog", captured(exportDialog)],
    ["warning", captured(warning)],
    ["helpCenter", captured(helpCenter)],
    ["foldAllSlider", captured(foldAllSlider)],
    ["proposalProgress", captured(proposalProgress, { neutralMouse: false })],
    ["viewer3dLoading", captured(viewer3dLoading)],
    ["viewer3dLoadError", captured(viewer3dLoadError)],
    ["foldPleatTarget", captured(foldPleatTarget)],
    ["selfIntersectionPairs", captured(selfIntersectionPairs)],
    ["recoveryChoices", captured(recoveryChoices)],
    ["exportFoldFile", captured(exportFoldFile)],
  ].map(([id, run]) => Object.freeze({ id, run }));
}
