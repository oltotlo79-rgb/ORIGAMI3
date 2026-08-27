// Shared, read-only CDP support for the remaining B1 acceptances.
// This module never starts or terminates desktop.exe.

import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

export const repositoryRoot = fileURLToPath(new URL("../../../", import.meta.url));

export function resolvePhase(id) {
  const raw = process.argv[2] ?? "describe";
  if (raw === "prepare" || raw === "--prepare") return "prepare";
  if (raw === "verify" || raw === "--verify") return "verify";
  if (raw === "describe" || raw === "--describe") return "describe";
  throw new Error(`${id}: unknown phase ${raw}; use prepare or verify`);
}

export function sha256(filePath) {
  return createHash("sha256").update(readFileSync(filePath)).digest("hex").toUpperCase();
}

function parsePoint(value, name) {
  let parsed;
  try {
    parsed = JSON.parse(value);
  } catch {
    throw new Error(`${name} must be a JSON pair such as [0.50,0.50]`);
  }
  assert.ok(Array.isArray(parsed) && parsed.length === 2, `${name} must have exactly two numbers`);
  for (const coordinate of parsed) {
    assert.ok(Number.isFinite(coordinate) && coordinate >= 0 && coordinate <= 1, `${name} must be within 0..1`);
  }
  return parsed;
}

export function prepare(id, requiredFiles, runtime) {
  for (const filePath of requiredFiles) {
    assert.ok(statSync(filePath).isFile(), `${id}: required source is missing: ${filePath}`);
  }
  process.stdout.write(`${id} PREPARE READY\n`);
  process.stdout.write(
    `${JSON.stringify(
      {
        id,
        phase: "prepare",
        cdpConnected: false,
        desktopStarted: false,
        requiredRuntimeInputs: runtime,
      },
      null,
      2,
    )}\n`,
  );
}

export function verifyRuntime(id, prefix, { points = false } = {}) {
  assert.equal(process.env.ORI3_B1_CDP_RUN, "1", `${id}: set ORI3_B1_CDP_RUN=1 for a dedicated slot`);
  const pid = Number(process.env.ORI3_DESKTOP_PID);
  const executable = process.env.ORI3_DESKTOP_EXE ? path.resolve(process.env.ORI3_DESKTOP_EXE) : null;
  const executableSha256 = (process.env.ORI3_DESKTOP_SHA256 ?? "").toUpperCase();
  const fixturePath = process.env[`${prefix}_FIXTURE`] ? path.resolve(process.env[`${prefix}_FIXTURE`]) : null;
  const fixtureSha256 = (process.env[`${prefix}_FIXTURE_SHA256`] ?? "").toUpperCase();
  assert.ok(Number.isSafeInteger(pid) && pid > 0, `${id}: ORI3_DESKTOP_PID is required`);
  assert.ok(executable, `${id}: ORI3_DESKTOP_EXE is required`);
  assert.match(executableSha256, /^[A-F0-9]{64}$/u, `${id}: ORI3_DESKTOP_SHA256 must be SHA-256`);
  assert.ok(fixturePath, `${id}: ${prefix}_FIXTURE is required`);
  assert.match(fixtureSha256, /^[A-F0-9]{64}$/u, `${id}: ${prefix}_FIXTURE_SHA256 must be SHA-256`);
  assert.ok(statSync(fixturePath).isFile(), `${id}: fixture is missing: ${fixturePath}`);
  assert.equal(sha256(fixturePath), fixtureSha256, `${id}: fixture SHA-256 differs`);
  const running = path.resolve(
    execFileSync(
      "powershell.exe",
      ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", `(Get-Process -Id ${pid} -ErrorAction Stop).Path`],
      { encoding: "utf8" },
    ).trim(),
  );
  assert.equal(running.toLowerCase(), executable.toLowerCase(), `${id}: PID executable differs`);
  assert.equal(sha256(running), executableSha256, `${id}: desktop executable SHA-256 differs`);
  const result = { pid, executable, executableSha256, fixturePath, fixtureSha256 };
  if (points) {
    result.start = parsePoint(process.env[`${prefix}_START`] ?? "", `${prefix}_START`);
    result.end = parsePoint(process.env[`${prefix}_END`] ?? "", `${prefix}_END`);
  }
  return result;
}

export class CdpConnection {
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
    const raw = typeof event.data === "string"
      ? event.data
      : event.data instanceof Blob
        ? await event.data.text()
        : Buffer.from(event.data).toString("utf8");
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

export async function connectDesktop(port = Number.parseInt(process.env.ORI3_CDP_PORT ?? "9222", 10)) {
  const endpoint = `http://127.0.0.1:${port}`;
  const targets = await fetch(`${endpoint}/json/list`).then((response) => {
    if (!response.ok) throw new Error(`CDP /json/list: HTTP ${response.status}`);
    return response.json();
  });
  const page = targets.find((target) => target.type === "page" && target.url === "http://tauri.localhost/" && target.webSocketDebuggerUrl);
  if (!page) throw new Error("ORIGAMI3 WebView target was not found");
  const connection = await CdpConnection.connect(page.webSocketDebuggerUrl);
  await connection.send("Runtime.enable");
  return connection;
}

export async function evaluate(connection, expression) {
  const reply = await connection.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (reply.exceptionDetails) {
    throw new Error(reply.exceptionDetails.exception?.description ?? reply.exceptionDetails.text ?? "Runtime.evaluate failed");
  }
  return reply.result.value;
}

export async function restoreBlank(connection) {
  return evaluate(connection, `(${async function restore() {
    const frames = async (count) => { for (let i = 0; i < count; i += 1) await new Promise((resolve) => requestAnimationFrame(resolve)); };
    const newButton = document.querySelector("header.toolbar button");
    if (!(newButton instanceof HTMLButtonElement)) throw new Error("new document button is unavailable");
    newButton.click();
    await frames(3);
    const dialog = document.querySelector('[data-floating-ui="new-document-dialog"]');
    if (!dialog) throw new Error("new document dialog did not open");
    const square = dialog.querySelector(".button-row button");
    if (!(square instanceof HTMLButtonElement)) throw new Error("150 mm square preset is unavailable");
    square.click();
    const confirm = dialog.querySelector("button.button-primary");
    if (!(confirm instanceof HTMLButtonElement) || confirm.disabled) throw new Error("new document confirmation is unavailable");
    confirm.click();
    await frames(8);
    const select = document.querySelector('[data-testid="tool-select"]');
    if (!(select instanceof HTMLButtonElement)) throw new Error("select tool is unavailable");
    select.click();
    const api = window.__origami3Capture;
    if (!api || api.version !== 1) throw new Error("Capture API version 1 is unavailable");
    await api.setView("normal");
    await api.waitForStable();
    const interaction = api.getInteractionState();
    if (api.getDocumentInfo().stepCount !== 0 || interaction.activeTool !== "select") throw new Error("blank baseline was not restored");
    if (document.querySelectorAll('[data-floating-ui$="dialog"]').length !== 0) throw new Error("a dialog remained open");
    return { stepCount: 0, activeTool: interaction.activeTool };
  }})()`);
}

export function passed(id, result) {
  process.stdout.write(`${id} VERIFY PASSED\n`);
  process.stdout.write(`${JSON.stringify({ id, passed: true, result }, null, 2)}\n`);
}

export function failed(id, phase, error) {
  const message = error instanceof Error ? error.stack ?? error.message : String(error);
  process.stderr.write(`${id} ${phase.toUpperCase()} FAILED: ${message}\n`);
  process.exitCode = 1;
}
