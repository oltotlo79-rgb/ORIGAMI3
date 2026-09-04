// M2.T2-6c.C05: verify the confirmed 2026-09-04 technique-name display spec on screen.
//   1. A grab-move fold gesture (tool-fold, drag on the 3D canvas) records
//      "つかんで動かした折り" in the timeline chip, its tooltip, and captureApi's step name
//      (technique_classification = { kind: "GrabMove", origin: "Automatic" }).
//   2. Explicitly picking a technique from the technique submenu (Pleat) and completing it
//      records the technique's own name ("段折り", origin: "Explicit"), not the auto-detected one.
//   3. Selecting that step and choosing a different technique in the "折り方" select
//      (InsideReverse) replaces the recorded name with that technique's name ("中割り折り").
// The three expected strings below are copied from apps/desktop/src/lib/techniques.ts, not guessed:
//   techniques.ts:38  DISPLAY_TECHNIQUE_LABEL.GrabMove = "つかんで動かした折り"
//   techniques.ts:9   TECHNIQUE_LABEL.Pleat = "段折り" (reused at techniques.ts:30 for DISPLAY_TECHNIQUE_LABEL.Pleat)
//   techniques.ts:10  TECHNIQUE_LABEL.InsideReverse = "中割り折り"

import assert from "node:assert/strict";
import path from "node:path";
import {
  connectDesktop,
  evaluate,
  failed,
  passed,
  prepare,
  repositoryRoot,
  resolvePhase,
  restoreBlank,
  verifyRuntime,
} from "./doc-link-b1-cdp-support.mjs";

const id = "M2.T2-6c.C05";
const phase = resolvePhase(id);

const GRAB_MOVE_LABEL = "つかんで動かした折り"; // techniques.ts:38
const PLEAT_LABEL = "段折り"; // techniques.ts:9 (techniques.ts:30)
const INSIDE_REVERSE_LABEL = "中割り折り"; // techniques.ts:10

/** タイムラインの札の文言から先頭の手順番号と末尾の警告マークを取り除き、技法名だけにする。 */
function stripStepChipLabel(text) {
  return text.replace(/^\d+\s+/u, "").replace(/\s+⚠$/u, "");
}

/** captureApi の手順名(`${number} ${label}`)から技法名だけを取り出す。 */
function stripCaptureStepName(name) {
  return name.replace(/^\d+\s+/u, "");
}

async function clickTestId(connection, testId) {
  return evaluate(connection, `(${async function click(selector) {
    const el = document.querySelector(selector);
    if (!(el instanceof HTMLElement)) throw new Error("missing element: " + selector);
    if (el instanceof HTMLButtonElement && el.disabled) throw new Error("element is disabled: " + selector);
    el.click();
    await window.__origami3Capture.waitForStable();
    return true;
  }})(${JSON.stringify(`[data-testid="${testId}"]`)})`);
}

/**
 * `適用` ボタンには data-testid が無い(apps/desktop/src/components/contextTechniques.tsx:538 は
 * テキストだけの `<button>適用</button>`)。実装どおり文言で探す。
 */
async function clickButtonByText(connection, containerSelector, label) {
  return evaluate(connection, `(${async function click(container, text) {
    const scope = document.querySelector(container);
    if (!scope) throw new Error("missing container: " + container);
    const button = [...scope.querySelectorAll("button")].find(
      (b) => (b.textContent ?? "").trim() === text,
    );
    if (!(button instanceof HTMLButtonElement)) throw new Error("missing button with text: " + text);
    if (button.disabled) throw new Error("button is disabled: " + text);
    button.click();
    await window.__origami3Capture.waitForStable();
    return true;
  }})(${JSON.stringify(containerSelector)}, ${JSON.stringify(label)})`);
}

/**
 * `#step-kind` には data-testid が無く id だけを持つ
 * (apps/desktop/src/components/contextAngleSteps.tsx:528-530)。実装どおり id で選ぶ。
 * doc-link-b1-remaining-cdp.mjs の setSelect と同じ input+change の組で React へ伝える。
 */
async function setSelectValue(connection, selector, value) {
  return evaluate(connection, `(${async function setValue(inputSelector, nextValue) {
    const input = document.querySelector(inputSelector);
    if (!(input instanceof HTMLSelectElement)) throw new Error("missing select: " + inputSelector);
    input.value = String(nextValue);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
    await window.__origami3Capture.waitForStable();
    if (input.value !== String(nextValue)) throw new Error("select value did not persist: " + inputSelector);
    return input.value;
  }})(${JSON.stringify(selector)}, ${JSON.stringify(value)})`);
}

async function viewerBox(connection) {
  return evaluate(connection, `(${function box() {
    const canvas = document.querySelector('canvas[data-testid="viewer3d-canvas"]');
    if (!(canvas instanceof HTMLCanvasElement)) throw new Error("3D canvas is unavailable");
    const rect = canvas.getBoundingClientRect();
    if (!(rect.width > 0 && rect.height > 0)) throw new Error("3D canvas has no measurable area");
    return { left: rect.left, top: rect.top, width: rect.width, height: rect.height };
  }})()`);
}

/** doc-link-b1-grab-cdp.mjs の grab() と同じ press→move→release の並びで、3D canvas 上をドラッグする。 */
async function dragOnViewer(connection, box, startNormalized, endNormalized) {
  const point = (normalized) => ({
    x: box.left + box.width * normalized[0],
    y: box.top + box.height * normalized[1],
  });
  const start = point(startNormalized);
  const end = point(endNormalized);
  await connection.send("Input.dispatchMouseEvent", { type: "mousePressed", x: start.x, y: start.y, button: "left", buttons: 1, clickCount: 1 });
  await connection.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: end.x, y: end.y, button: "left", buttons: 1 });
  await connection.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: end.x, y: end.y, button: "left", buttons: 0, clickCount: 1 });
  await evaluate(connection, "window.__origami3Capture.waitForStable()");
}

/** 事実1: つかんで動かす(tool-fold)操作で増えた手順が、自動判定名だけを持つこと。 */
async function verifyGrabMoveRecordsAutoLabel(connection, runtime) {
  const setup = await evaluate(connection, `(${async function open(input) {
    const api = window.__origami3Capture;
    if (!api || api.version !== 1) throw new Error("Capture API version 1 is unavailable");
    await api.openDocument(input.fixturePath);
    await api.setView("normal");
    await api.waitForStable();
    const tool = document.querySelector('[data-testid="tool-fold"]');
    if (!(tool instanceof HTMLButtonElement)) throw new Error("fold tool is unavailable");
    tool.click();
    await api.waitForStable();
    const canvas = document.querySelector('canvas[data-testid="viewer3d-canvas"]');
    if (!(canvas instanceof HTMLCanvasElement)) throw new Error("3D canvas is unavailable");
    const box = canvas.getBoundingClientRect();
    if (!(box.width > 0 && box.height > 0)) throw new Error("3D canvas has no measurable area");
    return { before: api.getDocumentInfo(), box: { left: box.left, top: box.top, width: box.width, height: box.height } };
  }})(${JSON.stringify(runtime)})`);

  await dragOnViewer(connection, setup.box, runtime.start, runtime.end);

  const result = await evaluate(connection, `(${async function read(before) {
    const api = window.__origami3Capture;
    await api.waitForStable();
    const after = api.getDocumentInfo();
    const addedStep = document.querySelector(`[data-testid="timeline-step-${before.stepCount + 1}"]`);
    const chipText = addedStep instanceof HTMLButtonElement
      ? (addedStep.textContent ?? "").replace(/\s+/gu, " ").trim()
      : null;
    const tooltip = addedStep instanceof HTMLButtonElement ? addedStep.getAttribute("data-tooltip") : null;
    return { before, after, chipText, tooltip, technique: api.getInteractionState().technique };
  }})(${JSON.stringify(setup.before)})`);

  assert.equal(result.after.stepCount, result.before.stepCount + 1, "grab-move fold gesture must add exactly one step");
  assert.ok(result.chipText !== null, "exactly one timeline entry must be added");
  assert.equal(stripStepChipLabel(result.chipText), GRAB_MOVE_LABEL, `timeline chip must show ${GRAB_MOVE_LABEL}: ${result.chipText}`);
  assert.equal(result.tooltip, GRAB_MOVE_LABEL, `timeline tooltip must equal ${GRAB_MOVE_LABEL}: ${result.tooltip}`);
  const addedCaptureStep = result.after.steps[result.after.steps.length - 1];
  assert.equal(
    stripCaptureStepName(addedCaptureStep.name),
    GRAB_MOVE_LABEL,
    `captureApi step name must equal ${GRAB_MOVE_LABEL}: ${addedCaptureStep.name}`,
  );
  assert.equal(result.technique.active, false, "auto-detection must not leave the manual technique draft active");
  return result;
}

/** 事実2: 技法submenu(段折り)で完了した手順が、選んだ技法自身の名前を持つこと。 */
async function verifyPleatSubmenuRecordsOwnLabel(connection) {
  await restoreBlank(connection);
  const before = await evaluate(connection, "window.__origami3Capture.getDocumentInfo()");

  await clickTestId(connection, "tool-technique");
  await clickTestId(connection, "technique-pleat");
  const box = await viewerBox(connection);
  await dragOnViewer(connection, box, [0.5, 0.5], [0.65, 0.5]);

  const drawn = await evaluate(connection, "window.__origami3Capture.getInteractionState().technique");
  assert.equal(drawn.active, true, "Pleat technique draft must be active after choosing it from the submenu");
  assert.equal(drawn.kind, "Pleat", `technique draft kind must be Pleat: ${drawn.kind}`);
  assert.equal(drawn.guideCreaseCount, 1, "dragging across the paper must set the Pleat guide line");

  await clickButtonByText(connection, "#context-panel", "適用");

  const after = await evaluate(connection, "window.__origami3Capture.getDocumentInfo()");
  assert.equal(after.stepCount, before.stepCount + 1, "committing the Pleat technique must add exactly one step");
  const addedStep = after.steps[after.steps.length - 1];
  assert.equal(
    stripCaptureStepName(addedStep.name),
    PLEAT_LABEL,
    `committed Pleat step name must equal ${PLEAT_LABEL}: ${addedStep.name}`,
  );

  const chip = await evaluate(connection, `(${function read(number) {
    const el = document.querySelector(`[data-testid="timeline-step-${number}"]`);
    return el instanceof HTMLButtonElement ? (el.textContent ?? "").replace(/\s+/gu, " ").trim() : null;
  }})(${addedStep.number})`);
  assert.ok(chip !== null, `timeline chip for the Pleat step must exist: step ${addedStep.number}`);
  assert.equal(stripStepChipLabel(chip), PLEAT_LABEL, `timeline chip for the Pleat step must equal ${PLEAT_LABEL}: ${chip}`);

  return { before, after, addedStep };
}

/** 事実3: 手順を選び「折り方」select で技法を選び直すと、記録名がその技法の名前へ替わること。 */
async function verifyReassigningTechniqueReplacesLabel(connection, stepNumber) {
  await clickTestId(connection, `timeline-step-${stepNumber}`);

  const stepKindBefore = await evaluate(connection, `(${function read() {
    const select = document.querySelector("#step-kind");
    if (!(select instanceof HTMLSelectElement)) throw new Error("step-kind select is unavailable");
    return select.value;
  }})()`);
  assert.equal(stepKindBefore, "Pleat", `step-kind select must start on Pleat: ${stepKindBefore}`);

  await setSelectValue(connection, "#step-kind", "InsideReverse");

  const after = await evaluate(connection, "window.__origami3Capture.getDocumentInfo()");
  const changedStep = after.steps.find((step) => step.number === stepNumber);
  if (!changedStep) throw new Error(`step ${stepNumber} is missing after re-picking the technique`);
  assert.equal(
    stripCaptureStepName(changedStep.name),
    INSIDE_REVERSE_LABEL,
    `captureApi step name must equal ${INSIDE_REVERSE_LABEL}: ${changedStep.name}`,
  );

  const chip = await evaluate(connection, `(${function read(number) {
    const el = document.querySelector(`[data-testid="timeline-step-${number}"]`);
    return el instanceof HTMLButtonElement ? (el.textContent ?? "").replace(/\s+/gu, " ").trim() : null;
  }})(${stepNumber})`);
  assert.ok(chip !== null, `timeline chip for the reassigned step must exist: step ${stepNumber}`);
  assert.equal(
    stripStepChipLabel(chip),
    INSIDE_REVERSE_LABEL,
    `timeline chip must equal ${INSIDE_REVERSE_LABEL} after re-picking the technique: ${chip}`,
  );

  return { after, changedStep, chip };
}

async function verify() {
  const runtime = verifyRuntime(id, "ORI3_B1_AUTO_TECHNIQUE", { points: true });
  const connection = await connectDesktop();
  try {
    const grabResult = await verifyGrabMoveRecordsAutoLabel(connection, runtime);
    const pleatResult = await verifyPleatSubmenuRecordsOwnLabel(connection);
    const reassignResult = await verifyReassigningTechniqueReplacesLabel(connection, pleatResult.addedStep.number);
    const restored = await restoreBlank(connection);
    passed(id, { runtime, grabResult, pleatResult, reassignResult, restored });
  } finally {
    connection.close();
  }
}

try {
  if (phase === "prepare") {
    prepare(id, [
      path.resolve(repositoryRoot, "apps/desktop/src/components/ToolRail.tsx"),
      path.resolve(repositoryRoot, "apps/desktop/src/components/Timeline.tsx"),
      path.resolve(repositoryRoot, "apps/desktop/src/components/contextTechniques.tsx"),
      path.resolve(repositoryRoot, "apps/desktop/src/components/contextAngleSteps.tsx"),
      path.resolve(repositoryRoot, "apps/desktop/src/captureApi.ts"),
      path.resolve(repositoryRoot, "apps/desktop/src/lib/techniques.ts"),
    ], [
      "ORI3_B1_AUTO_TECHNIQUE_FIXTURE",
      "ORI3_B1_AUTO_TECHNIQUE_FIXTURE_SHA256",
      "ORI3_B1_AUTO_TECHNIQUE_START",
      "ORI3_B1_AUTO_TECHNIQUE_END",
    ]);
  } else if (phase === "verify") {
    await verify();
  } else {
    process.stdout.write(`${id} PREPARE/VERIFY NOT EXECUTED\n`);
    process.stdout.write(`${JSON.stringify({ id, phases: ["prepare", "verify"], cdpConnected: false }, null, 2)}\n`);
  }
} catch (error) {
  failed(id, phase, error);
}
