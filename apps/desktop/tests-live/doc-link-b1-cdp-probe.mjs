// Read-only CDP preflight for the document-link B1 live checks.
// It verifies the supplied desktop process before reading the WebView state.
// If the product's recovery dialog is present, it chooses the required Restore action.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import path from "node:path";

const pid = Number(process.env.ORI3_DESKTOP_PID);
const executable = process.env.ORI3_DESKTOP_EXE ? path.resolve(process.env.ORI3_DESKTOP_EXE) : null;
const expectedHash = (process.env.ORI3_DESKTOP_SHA256 ?? "").toUpperCase();
const cdpPort = Number.parseInt(process.env.ORI3_CDP_PORT ?? "9222", 10);

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex").toUpperCase();
}

function verifyDesktop() {
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

  close() {
    this.socket.close();
  }
}

async function evaluate(connection, expression) {
  const reply = await connection.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (reply.exceptionDetails) throw new Error(reply.exceptionDetails.exception?.description ?? reply.exceptionDetails.text);
  return reply.result.value;
}

async function main() {
  verifyDesktop();
  const endpoint = `http://127.0.0.1:${cdpPort}`;
  const targets = await fetch(`${endpoint}/json/list`).then(async (response) => {
    if (!response.ok) throw new Error(`CDP endpoint returned HTTP ${response.status}`);
    return response.json();
  });
  const page = targets.find((target) => target.type === "page" && target.url === "http://tauri.localhost/" && target.webSocketDebuggerUrl);
  if (!page) throw new Error("ORIGAMI3 WebView target was not found");
  const connection = await CdpConnection.connect(page.webSocketDebuggerUrl);
  try {
    await connection.send("Runtime.enable");
    const screen = await evaluate(
      connection,
      `(() => {
        const compact = (value) => (value ?? "").replace(/\\s+/gu, " ").trim();
        const recovery = document.querySelector('[data-floating-ui="recovery-dialog"]');
        let recoveryRestored = false;
        if (recovery) {
          const restore = [...recovery.querySelectorAll("button")].find((button) => compact(button.textContent) === "\\u5fa9\\u5143\\u3059\\u308b");
          if (!restore) throw new Error("Recovery dialog is visible but its Restore button is unavailable");
          restore.click();
          recoveryRestored = true;
        }
        const capture = window.__origami3Capture;
        return {
          title: document.title,
          url: window.location.href,
          recoveryRestored,
          dialogs: [...document.querySelectorAll('[data-floating-ui$="dialog"]')].map((dialog) => dialog.getAttribute("data-floating-ui")),
          capture: capture ? {
            version: capture.version,
            status: capture.getStatus?.(),
            documentInfo: capture.getDocumentInfo?.(),
            interaction: capture.getInteractionState?.(),
          } : null,
          testids: {
            cpCanvas: document.querySelectorAll('[data-testid="cp-canvas"]').length,
            viewer3dCanvas: document.querySelectorAll('[data-testid="viewer3d-canvas"]').length,
            toolRail: document.querySelectorAll('[data-testid="tool-rail"]').length,
          },
          buttons: [...document.querySelectorAll("button")].map((button) => ({
            text: compact(button.textContent),
            ariaLabel: button.getAttribute("aria-label"),
            testid: button.getAttribute("data-testid"),
          })),
        };
      })()`,
    );
    process.stdout.write(`${JSON.stringify({ endpoint, pid, executable, screen }, null, 2)}\n`);
  } finally {
    connection.close();
  }
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.exitCode = 1;
});
