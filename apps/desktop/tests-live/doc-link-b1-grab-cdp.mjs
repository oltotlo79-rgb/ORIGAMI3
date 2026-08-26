// CDP acceptance for M2.T2-6c.C02/C03.
// It connects only to an already-running desktop.exe and restores a blank
// document, select tool, dialogs, and CDP viewport before disconnecting.
// selectedLayerCount is intentionally treated only as the number of target
// faces for this grab; it is never used as a pleat count.
import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const execute = process.env.ORI3_B1_CDP_RUN === "1";
const pid = Number(process.env.ORI3_DESKTOP_PID);
const executable = process.env.ORI3_DESKTOP_EXE ? path.resolve(process.env.ORI3_DESKTOP_EXE) : null;
const expectedHash = (process.env.ORI3_DESKTOP_SHA256 ?? "").toUpperCase();
const cdpPort = Number.parseInt(process.env.ORI3_CDP_PORT ?? "9222", 10);
const repositoryRoot = fileURLToPath(new URL("../../../", import.meta.url));
const fixture = {
  path: path.resolve(repositoryRoot, "crates/ori3-rigid/tests/fixtures/check-yakko.ori3"),
  sha256: "B9C3E2AF16A6382B47AA965100278C4FD50EF648DF5759E60C7C43E8BDEF2B26",
};

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex").toUpperCase();
}

function verifyContract() {
  if (!execute) {
    process.stdout.write("M2.T2-6c.C02/C03 NOT EXECUTED: set ORI3_B1_CDP_RUN=1 for a dedicated desktop slot.\n");
    return false;
  }
  assert.ok(Number.isSafeInteger(pid) && pid > 0, "ORI3_DESKTOP_PID is required");
  assert.ok(executable, "ORI3_DESKTOP_EXE is required");
  assert.match(expectedHash, /^[A-F0-9]{64}$/u, "ORI3_DESKTOP_SHA256 must be SHA-256");
  const running = path.resolve(execFileSync(
    "powershell.exe",
    ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", `(Get-Process -Id ${pid} -ErrorAction Stop).Path`],
    { encoding: "utf8" },
  ).trim());
  assert.equal(running.toLowerCase(), executable.toLowerCase(), "PID executable differs from supplied desktop.exe");
  assert.equal(sha256(running), expectedHash, "desktop.exe SHA-256 differs from supplied value");
  assert.ok(statSync(fixture.path).isFile(), "yakko fixture is missing");
  assert.equal(sha256(fixture.path), fixture.sha256, "yakko fixture SHA-256 differs");
  return true;
}

class CdpConnection {
  constructor(socket) {
    this.socket = socket;
    this.nextId = 1;
    this.pending = new Map();
    socket.addEventListener("message", (event) => void this.onMessage(event));
    socket.addEventListener("close", () => {
      for (const { reject } of this.pending.values()) reject(new Error("CDP connection closed"));
      this.pending.clear();
    });
  }

  static async connect(url) {
    const socket = new WebSocket(url);
    await new Promise((resolve, reject) => {
      socket.addEventListener("open", resolve, { once: true });
      socket.addEventListener("error", () => reject(new Error(`cannot connect to CDP: ${url}`)), { once: true });
    });
    return new CdpConnection(socket);
  }

  async onMessage(event) {
    const text = typeof event.data === "string" ? event.data : event.data instanceof Blob
      ? await event.data.text() : Buffer.from(event.data).toString("utf8");
    const message = JSON.parse(text);
    if (message.id === undefined) return;
    const pending = this.pending.get(message.id);
    if (!pending) return;
    this.pending.delete(message.id);
    if (message.error) pending.reject(new Error(`CDP ${message.error.code}: ${message.error.message}`));
    else pending.resolve(message.result);
  }

  send(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  close() { this.socket.close(); }
}

async function evaluate(connection, expression) {
  const reply = await connection.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (reply.exceptionDetails) throw new Error(reply.exceptionDetails.exception?.description ?? reply.exceptionDetails.text);
  return reply.result.value;
}

async function restoreBlank(connection) {
  return evaluate(connection, `(${async function restore() {
    const frames = async (count) => { for (let i = 0; i < count; i += 1) await new Promise((r) => requestAnimationFrame(r)); };
    const newButton = document.querySelector("header.toolbar button");
    if (!(newButton instanceof HTMLButtonElement)) throw new Error("New document button is unavailable");
    newButton.click();
    await frames(3);
    const dialog = document.querySelector('[data-floating-ui="new-document-dialog"]');
    if (!dialog) throw new Error("New document dialog did not open");
    const square = dialog.querySelector(".button-row button");
    if (!(square instanceof HTMLButtonElement)) throw new Error("150 mm square preset is unavailable");
    square.click();
    const confirm = dialog.querySelector("button.button-primary");
    if (!(confirm instanceof HTMLButtonElement) || confirm.disabled) throw new Error("New document confirmation is unavailable");
    confirm.click();
    await frames(8);
    const select = document.querySelector('[data-testid="tool-select"]');
    if (!(select instanceof HTMLButtonElement)) throw new Error("Select tool is unavailable");
    select.click();
    const api = window.__origami3Capture;
    await api.setView("normal");
    await api.waitForStable();
    const interaction = api.getInteractionState();
    if (api.getDocumentInfo().stepCount !== 0 || interaction.activeTool !== "select") throw new Error("Blank baseline was not restored");
    if (document.querySelectorAll('[data-floating-ui$="dialog"]').length !== 0) throw new Error("A dialog remained open");
    return { stepCount: api.getDocumentInfo().stepCount, activeTool: interaction.activeTool };
  }})()`);
}

async function openBird(connection) {
  return evaluate(connection, `(${async function open(file) {
    const api = window.__origami3Capture;
    if (!api || api.version !== 1) throw new Error("Capture API version 1 is unavailable");
    await api.openDocument(file);
    await api.setView("normal");
    await api.waitForStable();
    const tool = document.querySelector('[data-testid="tool-fold"]');
    if (!(tool instanceof HTMLButtonElement)) throw new Error("Fold tool is unavailable");
    tool.click();
    await api.waitForStable();
    const canvas = document.querySelector('canvas[data-testid="viewer3d-canvas"]');
    if (!(canvas instanceof HTMLCanvasElement)) throw new Error("3D canvas is unavailable");
    const box = canvas.getBoundingClientRect();
    return { beforeSteps: api.getDocumentInfo().stepCount, box: { left: box.left, top: box.top, width: box.width, height: box.height } };
  }})(${JSON.stringify(fixture.path)})`);
}

async function grab(connection, box, modifiers) {
  const x = box.left + box.width * 0.5;
  const y = box.top + box.height * 0.5;
  const targetX = box.left + box.width * 0.65;
  await connection.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", buttons: 1, clickCount: 1, modifiers });
  await connection.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: targetX, y, button: "left", buttons: 1, modifiers });
  const during = await evaluate(connection, "window.__origami3Capture.getInteractionState()");
  await connection.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: targetX, y, button: "left", buttons: 0, clickCount: 1, modifiers });
  await evaluate(connection, "window.__origami3Capture.waitForStable()");
  const after = await evaluate(connection, "({ steps: window.__origami3Capture.getDocumentInfo().stepCount, interaction: window.__origami3Capture.getInteractionState() })");
  return { during, after };
}

function verifyGrab(label, result, beforeSteps, expectedMode) {
  const state = result.during.viewer3d;
  assert.equal(state.grab.active, true, `${label}: grab was not active while dragging`);
  assert.equal(state.grab.mode, expectedMode, `${label}: unexpected grab mode`);
  assert.ok(Number.isInteger(state.grab.selectedLayerCount) && state.grab.selectedLayerCount > 0, `${label}: no target faces were selected`);
  // This count is deliberately not a pleat count.  It counts target faces only.
  assert.equal(state.preview.visible, true, `${label}: preview was not visible while dragging`);
  assert.ok(state.preview.polygonCount + state.preview.segmentCount > 0, `${label}: preview contained neither polygons nor segments`);
  assert.equal(result.after.steps, beforeSteps + 1, `${label}: drag did not add exactly one step`);
  assert.equal(result.after.interaction.viewer3d.grab.active, false, `${label}: grab remained active after release`);
  return { selectedLayerCount: state.grab.selectedLayerCount, polygonCount: state.preview.polygonCount, segmentCount: state.preview.segmentCount };
}

async function main() {
  if (!verifyContract()) return;
  const targets = await fetch(`http://127.0.0.1:${cdpPort}/json/list`).then((response) => {
    if (!response.ok) throw new Error(`CDP endpoint HTTP ${response.status}`);
    return response.json();
  });
  const page = targets.find((target) => target.type === "page" && target.url === "http://tauri.localhost/" && target.webSocketDebuggerUrl);
  if (!page) throw new Error("ORIGAMI3 WebView target was not found");
  const connection = await CdpConnection.connect(page.webSocketDebuggerUrl);
  let metrics = null;
  try {
    await connection.send("Runtime.enable");
    const initial = await evaluate(connection, "({ title: document.title, dialogs: [...document.querySelectorAll('[data-floating-ui$=\"dialog\"]')].length, info: window.__origami3Capture?.getDocumentInfo?.(), interaction: window.__origami3Capture?.getInteractionState?.() })");
    assert.equal(initial.title, "ORIGAMI3", "Unexpected page title");
    assert.equal(initial.dialogs, 0, "A dialog was open before the check began");
    assert.equal(initial.info?.stepCount, 0, "The dedicated slot was not blank");
    assert.equal(initial.interaction?.activeTool, "select", "The dedicated slot did not start with select");
    metrics = await evaluate(connection, "({ innerWidth, innerHeight, devicePixelRatio })");
    await connection.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 860, deviceScaleFactor: 2, mobile: false });

    const normalSetup = await openBird(connection);
    const normal = grab(connection, normalSetup.box, 0);
    const normalProof = verifyGrab("normal grab", await normal, normalSetup.beforeSteps, "flap");
    await restoreBlank(connection);

    const shiftSetup = await openBird(connection);
    const shift = grab(connection, shiftSetup.box, 8);
    const shiftProof = verifyGrab("Shift grab", await shift, shiftSetup.beforeSteps, "all");
    assert.notEqual(normalProof.selectedLayerCount, shiftProof.selectedLayerCount, "normal and Shift grabbed the same number of target faces");
    const restored = await restoreBlank(connection);
    process.stdout.write("M2.T2-6c.C02/C03 PASSED\n");
    process.stdout.write(`${JSON.stringify({
      ids: ["M2.T2-6c.C02", "M2.T2-6c.C03"], fixture, normal: normalProof, shift: shiftProof, restored,
    }, null, 2)}\n`);
  } finally {
    try { await restoreBlank(connection); } catch { /* preserve the primary assertion failure */ }
    if (metrics) await connection.send("Emulation.clearDeviceMetricsOverride");
    connection.close();
  }
}

main().catch((error) => {
  process.stderr.write(`M2.T2-6c.C02/C03 FAILED: ${error.stack ?? error}\n`);
  process.exitCode = 1;
});
