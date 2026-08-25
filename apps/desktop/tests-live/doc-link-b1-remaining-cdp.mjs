// CDP acceptance checks for the three B1 items that have tracked fixtures.
// This script never launches or terminates desktop.exe.  It verifies the
// supplied process and restores the initial blank document before disconnecting.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const execute = process.env.ORI3_B1_CDP_RUN === "1";
const calibrate = process.env.ORI3_B1_CDP_CALIBRATE === "1";
const pid = Number(process.env.ORI3_DESKTOP_PID);
const executable = process.env.ORI3_DESKTOP_EXE ? path.resolve(process.env.ORI3_DESKTOP_EXE) : null;
const expectedHash = (process.env.ORI3_DESKTOP_SHA256 ?? "").toUpperCase();
const cdpPort = Number.parseInt(process.env.ORI3_CDP_PORT ?? "9222", 10);
const repositoryRoot = fileURLToPath(new URL("../../../", import.meta.url));

const fixtures = {
  birdBase: {
    path: path.resolve(repositoryRoot, "crates/ori3-rigid/tests/fixtures/check-bird-base.ori3"),
    sha256: "29A9B7807AFE2EB4D43719C11B168005BB7E855B8A61E9C63A7B87EA12E6E889",
  },
  yakko: {
    path: path.resolve(repositoryRoot, "crates/ori3-rigid/tests/fixtures/check-yakko.ori3"),
    sha256: "B9C3E2AF16A6382B47AA965100278C4FD50EF648DF5759E60C7C43E8BDEF2B26",
  },
  petalViolation: {
    path: path.resolve(repositoryRoot, "crates/ori3-layers/tests/fixtures/petal-not-flat-foldable.ori3"),
    sha256: null,
  },
};

// Calibrated on 2026-08-26 with the fixed 1280x860 CSS-pixel CDP viewport.
// The three substantial visible layers measured 17,718 / 19,216 / 60,317 pixels
// before the fixed gesture and 32,628 / 37,426 / 67,449 afterwards.  14,000 is
// below 80% of the smallest measurement (14,174), rather than using a measured
// value itself as the pass boundary.  The closest substantial-layer centroids
// were 105.1 pixels apart; 80 pixels retains approximately 76% margin.
const MIN_VISIBLE_LAYER_PIXELS = 14_000;
const MIN_LAYER_CENTROID_DISTANCE = 80;
const MIN_VIEW_CHANGE_DISTANCE = 50;
// The non-flat-foldable fixture rendered 412 orange (#ff8c00, RGB distance <=12)
// physical pixels.  320 is below its 80% value (329.6).
const MIN_VIOLATION_ORANGE_PIXELS = 320;
// The four fixed construction gestures increased Aux (#7a7a7a) pixels by
// 5,062 / 62 / 71 / 25.  The respective boundaries 4,000 / 45 / 55 / 20
// are at or below approximately 80% and do not copy measured values.
const MIN_CONSTRUCT_AUX_PIXEL_INCREASE = Object.freeze({ angle: 4_000, perpendicular: 45, divide: 55, bisector: 20 });

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex").toUpperCase();
}

function verifyFile(fixture, name) {
  assert.ok(statSync(fixture.path).isFile(), `${name} fixture is missing: ${fixture.path}`);
  const actual = sha256(fixture.path);
  if (fixture.sha256 !== null) assert.equal(actual, fixture.sha256, `${name} fixture SHA-256 does not match`);
  return actual;
}

function verifyExecutionContract() {
  if (!execute) {
    process.stdout.write(`${JSON.stringify({ executed: false, ids: ["M2.T2-6c.C01", "M2.T2-7.C01", "M2.T2-7.C02"] }, null, 2)}\n`);
    return null;
  }
  assert.ok(Number.isSafeInteger(pid) && pid > 0, "ORI3_DESKTOP_PID is required");
  assert.ok(executable, "ORI3_DESKTOP_EXE is required");
  assert.match(expectedHash, /^[A-F0-9]{64}$/u, "ORI3_DESKTOP_SHA256 must be a SHA-256 hash");
  const running = path.resolve(
    execFileSync(
      "powershell.exe",
      ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", `(Get-Process -Id ${pid} -ErrorAction Stop).Path`],
      { encoding: "utf8" },
    ).trim(),
  );
  assert.equal(running.toLowerCase(), executable.toLowerCase(), "PID executable does not match the supplied path");
  assert.equal(sha256(running), expectedHash, "desktop executable SHA-256 does not match");
  return Object.fromEntries(Object.entries(fixtures).map(([name, fixture]) => [name, { ...fixture, actualSha256: verifyFile(fixture, name) }]));
}

function euclideanDistance([leftX, leftY], [rightX, rightY]) {
  return Math.hypot(leftX - rightX, leftY - rightY);
}

function substantialLayers(readback) {
  return readback.layers.filter((layer) => layer.pixels >= MIN_VISIBLE_LAYER_PIXELS);
}

function verifyLayerOffsets(label, readback) {
  const layers = substantialLayers(readback);
  assert.ok(layers.length >= 3, `${label}: fewer than three substantially visible layers`);
  for (let left = 0; left < layers.length; left += 1) {
    for (let right = left + 1; right < layers.length; right += 1) {
      assert.ok(
        euclideanDistance(layers[left].centroid, layers[right].centroid) >= MIN_LAYER_CENTROID_DISTANCE,
        `${label}: layer centroids are not separated by ${MIN_LAYER_CENTROID_DISTANCE} pixels`,
      );
    }
  }
  return layers;
}

function verifyDistinctView(before, after) {
  const beforeLayers = substantialLayers(before);
  const afterByLayer = new Map(substantialLayers(after).map((layer) => [layer.layer, layer]));
  const distances = beforeLayers.flatMap((layer) => {
    const afterLayer = afterByLayer.get(layer.layer);
    return afterLayer === undefined ? [] : [euclideanDistance(layer.centroid, afterLayer.centroid)];
  });
  assert.ok(distances.some((distance) => distance >= MIN_VIEW_CHANGE_DISTANCE), "The fixed 3D gesture did not change the visible viewpoint");
  return distances;
}

function verifyConstructResult(result) {
  assert.equal(result.menuCount.buttonCount, 4, "Construct menu button count changed");
  assert.deepEqual(result.menuCount.kinds, [1, 1, 1, 1], "Construct kinds are missing or duplicated");
  assert.equal(result.angle.value, "22.5", "Angle construction step is not 22.5 degrees");
  assert.equal(result.divisions.value, "4", "Division construction count is not four");
  const deltas = {
    angle: result.afterAngle.auxPixels - result.before.auxPixels,
    perpendicular: result.afterPerpendicular.auxPixels - result.afterAngle.auxPixels,
    divide: result.afterDivide.auxPixels - result.afterPerpendicular.auxPixels,
    bisector: result.afterBisector.auxPixels - result.afterDivide.auxPixels,
  };
  for (const [kind, minimum] of Object.entries(MIN_CONSTRUCT_AUX_PIXEL_INCREASE)) {
    assert.ok(deltas[kind] >= minimum, `${kind} construction added too few visible Aux pixels: ${deltas[kind]} < ${minimum}`);
  }
  return deltas;
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
      socket.addEventListener("error", () => reject(new Error(`Cannot connect to CDP: ${url}`)), { once: true });
    });
    return new CdpConnection(socket);
  }

  async onMessage(event) {
    let raw;
    if (typeof event.data === "string") raw = event.data;
    else if (event.data instanceof Blob) raw = await event.data.text();
    else raw = Buffer.from(event.data).toString("utf8");
    const message = JSON.parse(raw);
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

  close() {
    this.socket.close();
  }
}

async function evaluate(connection, expression) {
  const reply = await connection.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (reply.exceptionDetails) throw new Error(reply.exceptionDetails.exception?.description ?? reply.exceptionDetails.text ?? "Runtime.evaluate failed");
  return reply.result.value;
}

async function openFixture(connection, fixturePath) {
  return evaluate(
    connection,
    `(${async function open(path) {
      const api = window.__origami3Capture;
      if (!api || api.version !== 1) throw new Error("Capture API version 1 is unavailable");
      await api.openDocument(path);
      await api.setView("normal");
      await api.waitForStable();
      return { info: api.getDocumentInfo(), interaction: api.getInteractionState() };
    }})(${JSON.stringify(fixturePath)})`,
  );
}

async function resetBlankDocument(connection) {
  return evaluate(
    connection,
    `(${async function resetBlank() {
      const frames = async (count = 4) => {
        for (let index = 0; index < count; index++) await new Promise((resolve) => requestAnimationFrame(resolve));
      };
      const toolbar = document.querySelector("header.toolbar");
      const newButton = toolbar?.querySelector("button");
      if (!(newButton instanceof HTMLButtonElement)) throw new Error("New document button is unavailable for restoration");
      newButton.click();
      await frames();
      const dialog = document.querySelector('[data-floating-ui="new-document-dialog"]');
      if (!dialog) throw new Error("New document dialog did not open during restoration");
      const square150 = dialog.querySelector(".button-row button");
      if (!(square150 instanceof HTMLButtonElement)) throw new Error("150 mm square preset is unavailable for restoration");
      square150.click();
      const confirm = dialog.querySelector("button.button-primary");
      if (!(confirm instanceof HTMLButtonElement) || confirm.disabled) throw new Error("New document confirmation is unavailable for restoration");
      confirm.click();
      await frames(8);
      if (document.querySelector('[data-floating-ui="new-document-dialog"]')) throw new Error("New document dialog did not close during restoration");
      const select = document.querySelector('[data-testid="tool-select"]');
      if (!(select instanceof HTMLButtonElement)) throw new Error("Select tool is unavailable for restoration");
      select.click();
      const api = window.__origami3Capture;
      await api.setView("normal");
      await api.waitForStable();
      const interaction = api.getInteractionState();
      const info = api.getDocumentInfo();
      if (info.stepCount !== 0 || interaction.activeTool !== "select") throw new Error("Blank-document restoration did not reach the original baseline");
      if (document.querySelectorAll('[data-floating-ui$="dialog"]').length !== 0) throw new Error("A dialog remained after restoration");
      return { info, interaction };
    }})()`,
  );
}

async function inspectViewerAndCanvas(connection, includeCanonical, includeOrange) {
  return evaluate(
    connection,
    `(() => {
      const rect = (selector) => {
        const element = document.querySelector(selector);
        if (!(element instanceof HTMLElement)) throw new Error("Missing " + selector);
        const box = element.getBoundingClientRect();
        return { left: box.left, top: box.top, width: box.width, height: box.height };
      };
      const orangePixels = () => {
        const canvas = document.querySelector('canvas[data-testid="cp-canvas"]');
        if (!(canvas instanceof HTMLCanvasElement)) throw new Error("CP canvas is unavailable");
        const context = canvas.getContext("2d", { willReadFrequently: true });
        if (!context) throw new Error("CP canvas 2D context is unavailable");
        const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
        let count = 0;
        for (let index = 0; index < data.length; index += 4) {
          if (data[index + 3] === 0) continue;
          if (Math.hypot(data[index] - 255, data[index + 1] - 140, data[index + 2]) <= 12) count += 1;
        }
        return { width: canvas.width, height: canvas.height, count };
      };
      const layerReadback = () => {
        const capture = window.__origami3Capture.captureCanonical3D();
        const faceToLayer = new Map(capture.faces.map((face) => [face.face, face.layer]));
        const codeToFace = new Map(capture.readback.owner.codeToFace);
        const bytes = Uint8Array.from(atob(capture.readback.owner.data), (character) => character.charCodeAt(0));
        const layers = new Map();
        for (let pixel = 0, offset = 0; offset < bytes.length; pixel += 1, offset += 4) {
          const code = (bytes[offset] | (bytes[offset + 1] << 8) | (bytes[offset + 2] << 16) | (bytes[offset + 3] << 24)) >>> 0;
          const face = codeToFace.get(code);
          const layer = face === undefined ? undefined : faceToLayer.get(face);
          if (layer === undefined) continue;
          const prior = layers.get(layer) ?? { count: 0, x: 0, y: 0 };
          prior.count += 1;
          prior.x += pixel % capture.readback.width;
          prior.y += Math.floor(pixel / capture.readback.width);
          layers.set(layer, prior);
        }
        return {
          width: capture.readback.width,
          height: capture.readback.height,
          faceCount: capture.faces.length,
          layers: [...layers.entries()].sort((left, right) => left[0] - right[0]).map(([layer, stat]) => ({
            layer,
            pixels: stat.count,
            centroid: [stat.x / stat.count, stat.y / stat.count],
          })),
        };
      };
      return {
        viewer: rect('[data-testid="viewer3d-canvas"]'),
        cp: rect('canvas[data-testid="cp-canvas"]'),
        orange: ${includeOrange ? "orangePixels()" : "null"},
        canonical: ${includeCanonical ? "layerReadback()" : "null"},
      };
    })()`,
  );
}

async function rotateAndZoomViewer(connection) {
  return evaluate(
    connection,
    `(${async function rotateAndZoom() {
      const canvas = document.querySelector('[data-testid="viewer3d-canvas"]');
      if (!(canvas instanceof HTMLCanvasElement)) throw new Error("3D canvas is unavailable");
      const box = canvas.getBoundingClientRect();
      if (Math.abs(box.left - 675) > 1 || Math.abs(box.top - 56.015625) > 1 || Math.abs(box.width - 605) > 1 || Math.abs(box.height - 435.890625) > 1) {
        throw new Error("Unexpected fixed 3D canvas rectangle: " + JSON.stringify({ left: box.left, top: box.top, width: box.width, height: box.height }));
      }
      const event = (type, clientX, clientY, buttons) => canvas.dispatchEvent(new PointerEvent(type, {
        bubbles: true, cancelable: true, pointerId: 901, pointerType: "mouse", isPrimary: true,
        button: 0, buttons, clientX, clientY,
      }));
      event("pointerdown", 978, 274, 1);
      event("pointermove", 1038, 314, 1);
      event("pointerup", 1038, 314, 0);
      canvas.dispatchEvent(new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY: -160, clientX: 978, clientY: 274 }));
      await window.__origami3Capture.waitForStable();
      return { left: box.left, top: box.top, width: box.width, height: box.height };
    }})()`,
  );
}

async function clickControl(connection, selector) {
  return evaluate(
    connection,
    `(${async function click(selector) {
      const control = document.querySelector(selector);
      if (!(control instanceof HTMLElement)) throw new Error("Missing control: " + selector);
      control.click();
      await window.__origami3Capture.waitForStable();
      return { selector, activeTool: window.__origami3Capture.getInteractionState().activeTool };
    }})(${JSON.stringify(selector)})`,
  );
}

async function setSelect(connection, selector, value) {
  return evaluate(
    connection,
    `(${async function setValue(inputSelector, nextValue) {
      const input = document.querySelector(inputSelector);
      if (!(input instanceof HTMLSelectElement)) throw new Error("Missing select: " + inputSelector);
      input.value = String(nextValue);
      input.dispatchEvent(new Event("input", { bubbles: true }));
      input.dispatchEvent(new Event("change", { bubbles: true }));
      await window.__origami3Capture.waitForStable();
      if (input.value !== String(nextValue)) throw new Error("Select value did not persist: " + inputSelector);
      return { selector: inputSelector, value: input.value };
    }})(${JSON.stringify(selector)}, ${JSON.stringify(value)})`,
  );
}

async function canvasClick(connection, x, y) {
  await connection.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", buttons: 1, clickCount: 1 });
  await connection.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", buttons: 0, clickCount: 1 });
  await evaluate(connection, "window.__origami3Capture.waitForStable()");
}

async function cpAuxPixels(connection) {
  return evaluate(
    connection,
    `(() => {
      const canvas = document.querySelector('canvas[data-testid="cp-canvas"]');
      if (!(canvas instanceof HTMLCanvasElement)) throw new Error("CP canvas is unavailable");
      const context = canvas.getContext("2d", { willReadFrequently: true });
      if (!context) throw new Error("CP canvas 2D context is unavailable");
      const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
      let auxPixels = 0;
      for (let index = 0; index < data.length; index += 4) {
        if (data[index + 3] !== 0 && Math.hypot(data[index] - 122, data[index + 1] - 122, data[index + 2] - 122) <= 12) auxPixels += 1;
      }
      return { width: canvas.width, height: canvas.height, auxPixels };
    })()`,
  );
}

async function runConstructTools(connection, fixturePath) {
  await openFixture(connection, fixturePath);
  const menuTrigger = await clickControl(connection, '[data-testid="tool-construct"]');
  const menuCount = await evaluate(
    connection,
    `(() => ({
      buttonCount: document.querySelectorAll('[data-testid="construct-menu"] button').length,
      kinds: ["bisector", "perpendicular", "divide", "angle"].map((kind) => document.querySelectorAll('[data-testid="construct-' + kind + '"]').length),
      canvas: (() => { const box = document.querySelector('canvas[data-testid="cp-canvas"]')?.getBoundingClientRect(); return box ? { left: box.left, top: box.top, width: box.width, height: box.height } : null; })(),
    }))()`,
  );
  if (menuCount.buttonCount !== 4 || menuCount.kinds.some((count) => count !== 1)) throw new Error("Construct menu does not expose exactly four kinds");
  const canvas = menuCount.canvas;
  if (!canvas || Math.abs(canvas.left - 64) > 1 || Math.abs(canvas.top - 56.015625) > 1 || Math.abs(canvas.width - 605) > 1 || Math.abs(canvas.height - 531.890625) > 1) {
    throw new Error("Unexpected fixed CP canvas rectangle: " + JSON.stringify(canvas));
  }
  const before = await cpAuxPixels(connection);

  await clickControl(connection, '[data-testid="construct-angle"]');
  const angle = await setSelect(connection, '[data-testid="construct-angle-step"]', 22.5);
  await canvasClick(connection, 367, 322);
  const afterAngle = await cpAuxPixels(connection);

  await clickControl(connection, '[data-testid="construct-perpendicular"]');
  await canvasClick(connection, 430, 322);
  await canvasClick(connection, 427, 382);
  const afterPerpendicular = await cpAuxPixels(connection);

  await clickControl(connection, '[data-testid="construct-divide"]');
  const divisions = await setSelect(connection, '[data-testid="construct-divisions"]', 4);
  await canvasClick(connection, 250, 230);
  await canvasClick(connection, 475, 250);
  const afterDivide = await cpAuxPixels(connection);

  await clickControl(connection, '[data-testid="construct-bisector"]');
  await canvasClick(connection, 254, 406);
  await canvasClick(connection, 366, 286);
  await canvasClick(connection, 490, 406);
  const afterBisector = await cpAuxPixels(connection);
  return { menuTrigger, menuCount, before, angle, divisions, afterAngle, afterPerpendicular, afterDivide, afterBisector };
}

async function main() {
  const checkedFixtures = verifyExecutionContract();
  if (!checkedFixtures) return;
  const endpoint = `http://127.0.0.1:${cdpPort}`;
  const targets = await fetch(`${endpoint}/json/list`).then(async (response) => {
    if (!response.ok) throw new Error(`CDP endpoint returned HTTP ${response.status}`);
    return response.json();
  });
  const page = targets.find((target) => target.type === "page" && target.url === "http://tauri.localhost/" && target.webSocketDebuggerUrl);
  if (!page) throw new Error("ORIGAMI3 WebView target was not found");
  const connection = await CdpConnection.connect(page.webSocketDebuggerUrl);
  let originalMetrics = null;
  let restored = false;
  try {
    await connection.send("Runtime.enable");
    const initial = await evaluate(
      connection,
      `(() => ({
        title: document.title,
        dialogs: [...document.querySelectorAll('[data-floating-ui$="dialog"]')].map((element) => element.getAttribute("data-floating-ui")),
        info: window.__origami3Capture?.getDocumentInfo?.(),
        interaction: window.__origami3Capture?.getInteractionState?.(),
      }))()`,
    );
    assert.equal(initial.title, "ORIGAMI3", "Unexpected page title");
    assert.deepEqual(initial.dialogs, [], "A dialog was open before the check began");
    assert.equal(initial.info?.stepCount, 0, "The supplied slot did not start from a blank document");
    assert.equal(initial.interaction?.activeTool, "select", "The supplied slot did not start with the select tool");
    originalMetrics = await evaluate(connection, "({ innerWidth, innerHeight, devicePixelRatio })");
    await connection.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 860, deviceScaleFactor: 2, mobile: false });

    const bird = await openFixture(connection, checkedFixtures.birdBase.path);
    const birdMetrics = await inspectViewerAndCanvas(connection, true, false);
    const viewerGesture = await rotateAndZoomViewer(connection);
    const birdMetricsAfterGesture = await inspectViewerAndCanvas(connection, true, false);
    const petal = await openFixture(connection, checkedFixtures.petalViolation.path);
    const petalMetrics = await inspectViewerAndCanvas(connection, false, true);
    const construct = await runConstructTools(connection, checkedFixtures.yakko.path);
    const runnable = [];
    if (!calibrate) {
      const layersBefore = verifyLayerOffsets("M2.T2-6c.C01 before fixed gesture", birdMetrics.canonical);
      const layersAfter = verifyLayerOffsets("M2.T2-6c.C01 after fixed gesture", birdMetricsAfterGesture.canonical);
      const viewDistances = verifyDistinctView(birdMetrics.canonical, birdMetricsAfterGesture.canonical);
      runnable.push({
        id: "M2.T2-6c.C01",
        passed: true,
        visibleLayerIdsBefore: layersBefore.map((layer) => layer.layer),
        visibleLayerIdsAfter: layersAfter.map((layer) => layer.layer),
        viewDistances,
      });
      assert.ok(petalMetrics.orange.count >= MIN_VIOLATION_ORANGE_PIXELS, `M2.T2-7.C02 orange pixel count is too small: ${petalMetrics.orange.count}`);
      runnable.push({ id: "M2.T2-7.C02", passed: true, orangePixels: petalMetrics.orange.count });
      runnable.push({ id: "M2.T2-7.C01", passed: true, auxPixelIncreases: verifyConstructResult(construct) });
    }
    const restoredSnapshot = await resetBlankDocument(connection);
    restored = true;
    const result = {
      version: 1,
      mode: calibrate ? "calibration" : "inspection",
      executable: { pid, path: executable, sha256: expectedHash },
      fixtures: checkedFixtures,
      initial,
      bird,
      birdMetrics,
      viewerGesture,
      birdMetricsAfterGesture,
      petal,
      petalMetrics,
      construct,
      runnable,
      restored: restoredSnapshot,
    };
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  } finally {
    try {
      if (!restored) await resetBlankDocument(connection);
    } finally {
      if (originalMetrics) await connection.send("Emulation.clearDeviceMetricsOverride");
      connection.close();
    }
  }
}

const keepAlive = setInterval(() => {}, 1_000);

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.exitCode = 1;
}).finally(() => clearInterval(keepAlive));
