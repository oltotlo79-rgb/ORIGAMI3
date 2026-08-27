// M2.T2-6c.C05: verify that a normal fold gesture records its auto-detected technique.
// The script deliberately does not choose the technique submenu: that would test manual selection, not auto-detection.

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
const expectedLabel = process.env.ORI3_B1_AUTO_TECHNIQUE_LABEL ?? "単純折り";

async function verify() {
  const runtime = verifyRuntime(id, "ORI3_B1_AUTO_TECHNIQUE", { points: true });
  const connection = await connectDesktop();
  try {
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
    const point = (normalized) => ({ x: setup.box.left + setup.box.width * normalized[0], y: setup.box.top + setup.box.height * normalized[1] });
    const start = point(runtime.start);
    const end = point(runtime.end);
    await connection.send("Input.dispatchMouseEvent", { type: "mousePressed", x: start.x, y: start.y, button: "left", buttons: 1, clickCount: 1 });
    await connection.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: end.x, y: end.y, button: "left", buttons: 1 });
    await connection.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: end.x, y: end.y, button: "left", buttons: 0, clickCount: 1 });
    const result = await evaluate(connection, `(${async function read(before, label) {
      const api = window.__origami3Capture;
      await api.waitForStable();
      const after = api.getDocumentInfo();
      const addedStep = document.querySelector(`[data-testid="timeline-step-${before.stepCount + 1}"]`);
      const addedLabels = addedStep instanceof HTMLButtonElement
        ? [(addedStep.textContent ?? "").replace(/\s+/gu, " ").trim()]
        : [];
      return { before, after, addedLabels, technique: api.getInteractionState().technique, expectedLabel: label };
    }})(${JSON.stringify(setup.before)}, ${JSON.stringify(expectedLabel)})`);
    assert.equal(result.after.stepCount, result.before.stepCount + 1, "normal fold gesture must add exactly one step");
    assert.equal(result.addedLabels.length, 1, "exactly one timeline entry must be added");
    assert.ok(result.addedLabels[0].includes(expectedLabel), `auto-detected technique must be ${expectedLabel}: ${result.addedLabels[0]}`);
    assert.equal(result.technique.active, false, "auto-detection must not leave the manual technique draft active");
    const restored = await restoreBlank(connection);
    passed(id, { runtime, result, restored });
  } finally {
    connection.close();
  }
}

try {
  if (phase === "prepare") {
    prepare(id, [
      path.resolve(repositoryRoot, "apps/desktop/src/components/ToolRail.tsx"),
      path.resolve(repositoryRoot, "apps/desktop/src/components/Timeline.tsx"),
      path.resolve(repositoryRoot, "apps/desktop/src/captureApi.ts"),
    ], ["ORI3_B1_AUTO_TECHNIQUE_FIXTURE", "ORI3_B1_AUTO_TECHNIQUE_FIXTURE_SHA256", "ORI3_B1_AUTO_TECHNIQUE_START", "ORI3_B1_AUTO_TECHNIQUE_END", "ORI3_B1_AUTO_TECHNIQUE_LABEL (default: 単純折り)"]);
  } else if (phase === "verify") {
    await verify();
  } else {
    process.stdout.write(`${id} PREPARE/VERIFY NOT EXECUTED\n`);
    process.stdout.write(`${JSON.stringify({ id, phases: ["prepare", "verify"], cdpConnected: false }, null, 2)}\n`);
  }
} catch (error) {
  failed(id, phase, error);
}
