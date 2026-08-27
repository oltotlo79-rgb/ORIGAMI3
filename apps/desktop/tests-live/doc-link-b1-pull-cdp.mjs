// M2.T2-6b.C05: automatic pull gesture acceptance.
// `prepare` is offline. `verify` connects only to an already-running dedicated desktop.exe.

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

const id = "M2.T2-6b.C05";
const phase = resolvePhase(id);

async function verify() {
  const runtime = verifyRuntime(id, "ORI3_B1_PULL", { points: true });
  const connection = await connectDesktop();
  try {
    const setup = await evaluate(connection, `(${async function open(input) {
      const api = window.__origami3Capture;
      if (!api || api.version !== 1) throw new Error("Capture API version 1 is unavailable");
      await api.openDocument(input.fixturePath);
      await api.setView("normal");
      await api.waitForStable();
      const tool = document.querySelector('[data-testid="tool-pull"]');
      if (!(tool instanceof HTMLButtonElement)) throw new Error("pull tool is unavailable");
      tool.click();
      await api.waitForStable();
      const canvas = document.querySelector('canvas[data-testid="viewer3d-canvas"]');
      if (!(canvas instanceof HTMLCanvasElement)) throw new Error("3D canvas is unavailable");
      const box = canvas.getBoundingClientRect();
      if (!(box.width > 0 && box.height > 0)) throw new Error("3D canvas has no measurable area");
      const before = api.getDocumentInfo();
      return { before, box: { left: box.left, top: box.top, width: box.width, height: box.height } };
    }})(${JSON.stringify(runtime)})`);
    const point = (normalized) => ({ x: setup.box.left + setup.box.width * normalized[0], y: setup.box.top + setup.box.height * normalized[1] });
    const start = point(runtime.start);
    const end = point(runtime.end);
    await connection.send("Input.dispatchMouseEvent", { type: "mousePressed", x: start.x, y: start.y, button: "left", buttons: 1, clickCount: 1 });
    await connection.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: end.x, y: end.y, button: "left", buttons: 1 });
    await connection.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: end.x, y: end.y, button: "left", buttons: 0, clickCount: 1 });
    const result = await evaluate(connection, `(${async function read(before) {
      const api = window.__origami3Capture;
      await api.waitForStable();
      const after = api.getDocumentInfo();
      const interaction = api.getInteractionState();
      return { before, after, pull: interaction.pull };
    }})(${JSON.stringify(setup.before)})`);
    assert.equal(result.after.stepCount, result.before.stepCount + 1, "pull gesture must add exactly one step");
    assert.ok(result.after.edgeCount > result.before.edgeCount, "pull gesture must add at least one crease edge");
    assert.ok(result.pull.hinge === null || Number.isInteger(result.pull.hinge), "pull hinge capture is invalid");
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
      path.resolve(repositoryRoot, "apps/desktop/src/captureApi.ts"),
    ], ["ORI3_B1_PULL_FIXTURE", "ORI3_B1_PULL_FIXTURE_SHA256", "ORI3_B1_PULL_START", "ORI3_B1_PULL_END"]);
  } else if (phase === "verify") {
    await verify();
  } else {
    process.stdout.write(`${id} PREPARE/VERIFY NOT EXECUTED\n`);
    process.stdout.write(`${JSON.stringify({ id, phases: ["prepare", "verify"], cdpConnected: false }, null, 2)}\n`);
  }
} catch (error) {
  failed(id, phase, error);
}
