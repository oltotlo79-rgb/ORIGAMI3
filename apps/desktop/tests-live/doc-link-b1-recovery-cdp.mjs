// M2.T2-8.C02 recovery acceptance through the real Tauri command path.
//
// This file never starts or terminates desktop.exe.  It has two explicit phases:
//
// 1. Prepare an empty, isolated app-data directory before the dedicated app starts:
//
//    $env:ORI3_TEST_APP_DATA_DIR = "$env:TEMP\\ori3-doclink-recovery"
//    node apps/desktop/tests-live/doc-link-b1-recovery-cdp.mjs --prepare
//
// 2. Start a dedicated desktop.exe with the same ORI3_TEST_APP_DATA_DIR, then verify:
//
//    $env:ORI3_B1_CDP_RUN = "1"
//    $env:ORI3_DESKTOP_PID = "<PID>"
//    $env:ORI3_DESKTOP_EXE = "<absolute desktop.exe path>"
//    $env:ORI3_DESKTOP_SHA256 = "<SHA-256>"
//    node apps/desktop/tests-live/doc-link-b1-recovery-cdp.mjs --verify
//
// The preparation phase refuses a nonempty directory and never deletes anything.
// The verification phase only chooses "復元する"; it never sends "破棄する".

import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import { fileURLToPath } from "node:url";
import path from "node:path";

const phase = process.argv[2] ?? "--describe";
const execute = process.env.ORI3_B1_CDP_RUN === "1";
const cdpPort = Number.parseInt(process.env.ORI3_CDP_PORT ?? "9222", 10);
const pid = Number(process.env.ORI3_DESKTOP_PID);
const executable = process.env.ORI3_DESKTOP_EXE ? path.resolve(process.env.ORI3_DESKTOP_EXE) : null;
const expectedExecutableHash = (process.env.ORI3_DESKTOP_SHA256 ?? "").toUpperCase();
const repositoryRoot = fileURLToPath(new URL("../../../", import.meta.url));
const isolationValue = process.env.ORI3_TEST_APP_DATA_DIR;
const fixture = {
  path: path.resolve(repositoryRoot, "crates/ori3-rigid/tests/fixtures/check-crane.ori3"),
  sha256: "D44565B8CF3FF46AAD03905709CF891DA6627D235BD1CCE02F1F8EF8E67CF818",
  vertices: 33,
  edges: 61,
  steps: 6,
};
const autosaveName = "無題.ori3.autosave";
const markerName = "autosave-location.txt";

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex").toUpperCase();
}

function isolatedDirectory() {
  assert.ok(isolationValue, "ORI3_TEST_APP_DATA_DIR is required");
  const directory = path.resolve(isolationValue);
  const tempRoot = path.resolve(os.tmpdir());
  const relative = path.relative(tempRoot, directory);
  assert.notEqual(relative, "", "ORI3_TEST_APP_DATA_DIR must not be the temp root itself");
  assert.ok(!relative.startsWith("..") && !path.isAbsolute(relative), "ORI3_TEST_APP_DATA_DIR must be below the system temp directory");
  return directory;
}

function recoveryPaths(directory) {
  return {
    autosave: path.join(directory, autosaveName),
    marker: path.join(directory, markerName),
  };
}

function verifyFixture() {
  assert.ok(statSync(fixture.path).isFile(), `recovery fixture is missing: ${fixture.path}`);
  assert.equal(sha256(fixture.path), fixture.sha256, "recovery fixture SHA-256 does not match");
}

function prepare() {
  const directory = isolatedDirectory();
  verifyFixture();
  if (existsSync(directory)) {
    assert.equal(readdirSync(directory).length, 0, `isolation directory is not empty: ${directory}`);
  } else {
    mkdirSync(directory, { recursive: true });
  }
  const paths = recoveryPaths(directory);
  assert.ok(!existsSync(paths.autosave), `refusing to overwrite autosave: ${paths.autosave}`);
  assert.ok(!existsSync(paths.marker), `refusing to overwrite autosave marker: ${paths.marker}`);
  copyFileSync(fixture.path, paths.autosave);
  writeFileSync(paths.marker, paths.autosave, { encoding: "utf8", flag: "wx" });
  process.stdout.write(
    `${JSON.stringify({
      prepared: true,
      isolationDirectory: directory,
      autosave: paths.autosave,
      marker: paths.marker,
      fixture: { path: fixture.path, sha256: fixture.sha256 },
      next: "Start a dedicated desktop.exe with this same ORI3_TEST_APP_DATA_DIR, then run --verify.",
    }, null, 2)}\n`,
  );
}

function verifyExecutionContract() {
  assert.ok(execute, "set ORI3_B1_CDP_RUN=1 before --verify");
  assert.ok(Number.isSafeInteger(pid) && pid > 0, "ORI3_DESKTOP_PID is required");
  assert.ok(executable, "ORI3_DESKTOP_EXE is required");
  assert.match(expectedExecutableHash, /^[A-F0-9]{64}$/u, "ORI3_DESKTOP_SHA256 must be a SHA-256 hash");
  const running = path.resolve(
    execFileSync(
      "powershell.exe",
      ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", `(Get-Process -Id ${pid} -ErrorAction Stop).Path`],
      { encoding: "utf8" },
    ).trim(),
  );
  assert.equal(running.toLowerCase(), executable.toLowerCase(), "PID executable does not match ORI3_DESKTOP_EXE");
  assert.equal(sha256(running), expectedExecutableHash, "desktop executable SHA-256 does not match");
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
  if (reply.exceptionDetails) throw new Error(reply.exceptionDetails.exception?.description ?? reply.exceptionDetails.text);
  return reply.result.value;
}

async function verify() {
  const directory = isolatedDirectory();
  const paths = recoveryPaths(directory);
  verifyFixture();
  assert.ok(existsSync(paths.autosave) && existsSync(paths.marker), "run --prepare before starting desktop.exe");
  verifyExecutionContract();
  const endpoint = `http://127.0.0.1:${cdpPort}`;
  const targets = await fetch(`${endpoint}/json/list`).then((response) => {
    if (!response.ok) throw new Error(`CDP /json/list: HTTP ${response.status}`);
    return response.json();
  });
  const page = targets.find((target) => target.type === "page" && target.url === "http://tauri.localhost/" && target.webSocketDebuggerUrl);
  if (!page) throw new Error("ORIGAMI3 WebView target was not found");
  const connection = await CdpConnection.connect(page.webSocketDebuggerUrl);
  try {
    await connection.send("Runtime.enable");
    const result = await evaluate(
      connection,
      `(${async function recover() {
        const compact = (value) => (value ?? "").replace(/\\s+/gu, " ").trim();
        const api = window.__origami3Capture;
        if (!api || api.version !== 1) throw new Error("Capture API version 1 is unavailable");
        const dialog = document.querySelector('[data-floating-ui="recovery-dialog"]');
        if (!dialog) throw new Error("recovery dialog did not appear at startup");
        const buttons = [...dialog.querySelectorAll("button")];
        const labels = buttons.map((button) => compact(button.textContent));
        const restore = buttons.find((button) => compact(button.textContent) === "復元する");
        if (!restore) throw new Error("recovery dialog has no Restore button");
        const before = api.getInteractionState();
        if (!before.diagnosis.recoveryVisible) throw new Error("capture state does not report recovery visibility");
        restore.click();
        for (let attempt = 0; attempt < 100; attempt += 1) {
          if (!document.querySelector('[data-floating-ui="recovery-dialog"]')) break;
          await new Promise((resolve) => window.setTimeout(resolve, 25));
        }
        await api.waitForStable();
        if (document.querySelector('[data-floating-ui="recovery-dialog"]')) throw new Error("recovery dialog did not close after Restore");
        return { labels, before, after: api.getInteractionState(), info: api.getDocumentInfo() };
      }})()`,
    );
    assert.deepEqual(result.labels, ["復元する", "破棄する"], "recovery choices changed");
    assert.equal(result.after.diagnosis.recoveryVisible, false, "recovery remains visible after Restore");
    assert.equal(result.after.document.vertexCount, fixture.vertices, "restored vertex count differs from fixture");
    assert.equal(result.after.document.edgeCount, fixture.edges, "restored edge count differs from fixture");
    assert.equal(result.info.stepCount, fixture.steps, "restored step count differs from fixture");
    assert.ok(!existsSync(paths.autosave), "autosave remains after real recovery_restore");
    assert.ok(!existsSync(paths.marker), "autosave marker remains after real recovery_restore");
    process.stdout.write(`${JSON.stringify({ passed: true, id: "M2.T2-8.C02", result }, null, 2)}\n`);
  } finally {
    connection.close();
  }
}

if (phase === "--prepare") prepare();
else if (phase === "--verify") await verify();
else if (phase === "--describe") {
  process.stdout.write(`${JSON.stringify({ id: "M2.T2-8.C02", phases: ["--prepare", "--verify"], executed: false }, null, 2)}\n`);
} else {
  throw new Error(`unknown phase: ${phase}`);
}
