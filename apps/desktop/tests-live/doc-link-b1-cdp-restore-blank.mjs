// Emergency-safe restoration for the supplied B1 CDP slot.
// It uses the visible New Document dialog to return to the default 150 mm square blank document.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import path from "node:path";

const pid = Number(process.env.ORI3_DESKTOP_PID);
const executable = process.env.ORI3_DESKTOP_EXE ? path.resolve(process.env.ORI3_DESKTOP_EXE) : null;
const expectedHash = (process.env.ORI3_DESKTOP_SHA256 ?? "").toUpperCase();

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex").toUpperCase();
}

function verifyDesktop() {
  assert.ok(Number.isSafeInteger(pid) && pid > 0, "ORI3_DESKTOP_PID is required");
  assert.ok(executable, "ORI3_DESKTOP_EXE is required");
  const running = path.resolve(execFileSync("powershell.exe", ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", `(Get-Process -Id ${pid} -ErrorAction Stop).Path`], { encoding: "utf8" }).trim());
  assert.equal(running.toLowerCase(), executable.toLowerCase(), "PID executable does not match the supplied path");
  assert.equal(sha256(running), expectedHash, "desktop executable SHA-256 does not match");
}

class CdpConnection {
  constructor(socket) {
    this.socket = socket;
    this.nextId = 1;
    this.pending = new Map();
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(typeof event.data === "string" ? event.data : Buffer.from(event.data).toString("utf8"));
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if (message.error) pending.reject(new Error(`CDP ${message.error.code}: ${message.error.message}`));
      else pending.resolve(message.result);
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
  send(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }
  close() { this.socket.close(); }
}

async function main() {
  verifyDesktop();
  const targets = await fetch("http://127.0.0.1:9222/json/list").then((response) => response.json());
  const page = targets.find((target) => target.type === "page" && target.url === "http://tauri.localhost/" && target.webSocketDebuggerUrl);
  if (!page) throw new Error("ORIGAMI3 WebView target was not found");
  const connection = await CdpConnection.connect(page.webSocketDebuggerUrl);
  try {
    const reply = await connection.send("Runtime.evaluate", {
      awaitPromise: true,
      returnByValue: true,
      expression: `(${async function restore() {
        const frames = async (count) => { for (let i = 0; i < count; i++) await new Promise((resolve) => requestAnimationFrame(resolve)); };
        const toolbar = document.querySelector("header.toolbar");
        const newButton = toolbar?.querySelector("button");
        if (!(newButton instanceof HTMLButtonElement)) throw new Error("New document button is unavailable");
        newButton.click();
        await frames(3);
        const dialog = document.querySelector('[data-floating-ui="new-document-dialog"]');
        if (!dialog) throw new Error("New document dialog did not open");
        const preset = dialog.querySelector(".button-row button");
        if (!(preset instanceof HTMLButtonElement)) throw new Error("150 mm square preset is unavailable");
        preset.click();
        const confirm = dialog.querySelector("button.button-primary");
        if (!(confirm instanceof HTMLButtonElement) || confirm.disabled) throw new Error("New document confirmation is unavailable");
        confirm.click();
        await frames(8);
        const api = window.__origami3Capture;
        if (!api) throw new Error("Capture API is unavailable");
        document.querySelector('[data-testid="tool-select"]')?.click();
        await api.setView("normal");
        await api.waitForStable();
        return { info: api.getDocumentInfo(), interaction: api.getInteractionState(), dialogs: document.querySelectorAll('[data-floating-ui$="dialog"]').length };
      }})()`,
    });
    if (reply.exceptionDetails) throw new Error(reply.exceptionDetails.exception?.description ?? reply.exceptionDetails.text ?? "Restoration evaluation failed");
    await connection.send("Emulation.clearDeviceMetricsOverride");
    const value = reply.result.value;
    assert.equal(value.info.stepCount, 0, "Restoration did not create a blank document");
    assert.equal(value.interaction.activeTool, "select", "Restoration did not select the select tool");
    assert.equal(value.dialogs, 0, "A dialog remained after restoration");
    process.stdout.write(`${JSON.stringify(value, null, 2)}\n`);
  } finally {
    connection.close();
  }
}

main().catch((error) => { process.stderr.write(`${error.stack ?? error}\n`); process.exitCode = 1; });
