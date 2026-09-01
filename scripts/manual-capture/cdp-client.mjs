import fs from "node:fs/promises";
import path from "node:path";

function timeout(label, milliseconds) {
  let handle;
  const promise = new Promise((_, reject) => {
    handle = setTimeout(() => reject(new Error(`${label} timed out after ${milliseconds}ms`)), milliseconds);
  });
  return { promise, cancel: () => clearTimeout(handle) };
}

async function withTimeout(promise, label, milliseconds) {
  const timer = timeout(label, milliseconds);
  try {
    return await Promise.race([promise, timer.promise]);
  } finally {
    timer.cancel();
  }
}

/**
 * Initialize an already-open CDP socket. Kept injectable so the failure path
 * can be verified offline without opening a browser or WebSocket.
 */
export async function initializeManualCaptureConnection({ call, lockMetrics, close }) {
  if (typeof call !== "function" || typeof lockMetrics !== "function" || typeof close !== "function") {
    throw new Error("manual capture connection initializer requires call, lockMetrics, and close functions");
  }
  try {
    await call("Runtime.enable");
    await call("Page.enable");
    await lockMetrics();
  } catch (initializationError) {
    try {
      await close();
    } catch (closeError) {
      throw new AggregateError(
        [initializationError, closeError],
        "CDP initialization failed and its WebSocket could not be closed",
      );
    }
    throw initializationError;
  }
}

/** Close a CONNECTING/OPEN socket when target connection itself fails. */
export async function awaitManualCaptureSocketOpen({ openPromise, close, timeoutMilliseconds = 5_000 }) {
  if (!openPromise || typeof openPromise.then !== "function" || typeof close !== "function") {
    throw new Error("manual capture socket opener requires a promise and close function");
  }
  try {
    await withTimeout(openPromise, "CDP WebSocket connection", timeoutMilliseconds);
  } catch (connectionError) {
    try {
      await close();
    } catch (closeError) {
      throw new AggregateError(
        [connectionError, closeError],
        "CDP WebSocket connection failed and its socket could not be closed",
      );
    }
    throw connectionError;
  }
}

/**
 * Fetch and parse target discovery under one abortable deadline. The injected
 * seams let timeout/rejection behavior be verified without any network access.
 */
export async function discoverManualCaptureTargets({
  endpoint,
  fetchImpl = globalThis.fetch,
  createAbortController = () => new AbortController(),
  timeoutMilliseconds = 5_000,
}) {
  if (typeof fetchImpl !== "function" || typeof createAbortController !== "function") {
    throw new Error("CDP target discovery requires fetch and AbortController factories");
  }
  const controller = createAbortController();
  if (!controller || typeof controller.abort !== "function" || !("signal" in controller)) {
    throw new Error("CDP target discovery AbortController is invalid");
  }
  const operation = (async () => {
    const response = await fetchImpl(`${endpoint}/json/list`, { signal: controller.signal });
    if (!response?.ok) throw new Error(`CDP target discovery failed: ${response?.status ?? "unknown"}`);
    const targets = await response.json();
    if (!Array.isArray(targets)) throw new Error("CDP target discovery response must be an array");
    return targets;
  })();
  try {
    return await withTimeout(operation, "CDP target discovery headers/body", timeoutMilliseconds);
  } catch (discoveryError) {
    // Promise.race cannot cancel fetch or response.json by itself. Abort first,
    // and keep a rejection observer on injected implementations that settle
    // after this function has returned.
    void operation.catch(() => {});
    try {
      controller.abort();
    } catch (abortError) {
      throw new AggregateError(
        [discoveryError, abortError],
        "CDP target discovery failed and its fetch could not be aborted",
      );
    }
    throw discoveryError;
  }
}

export async function connectManualCapture(endpoint) {
  const targets = await discoverManualCaptureTargets({ endpoint });
  const pages = targets.filter((target) => target.type === "page" && target.webSocketDebuggerUrl);
  const candidates = pages.filter((target) => {
    try {
      const url = new URL(target.url);
      return url.protocol === "http:" && url.hostname === "tauri.localhost";
    } catch {
      return false;
    }
  });
  if (candidates.length !== 1) {
    throw new Error(
      `Expected exactly one ORIGAMI3 CDP page, found ${candidates.length}: ${JSON.stringify(
        pages.map(({ id, title, url }) => ({ id, title, url })),
      )}`,
    );
  }

  const endpointUrl = new URL(endpoint);
  const socketUrl = new URL(candidates[0].webSocketDebuggerUrl);
  socketUrl.hostname = endpointUrl.hostname;
  socketUrl.port = endpointUrl.port;
  const socket = new WebSocket(socketUrl);
  const close = () => {
    if (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING) socket.close();
  };
  await awaitManualCaptureSocketOpen({
    openPromise: new Promise((resolve, reject) => {
      socket.addEventListener("open", resolve, { once: true });
      socket.addEventListener("error", reject, { once: true });
    }),
    close,
  });

  let nextId = 1;
  const pending = new Map();
  const rejectPending = (error) => {
    for (const waiter of pending.values()) waiter.reject(error);
    pending.clear();
  };
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(String(event.data));
    if (!message.id) return;
    const waiter = pending.get(message.id);
    if (!waiter) return;
    pending.delete(message.id);
    if (message.error) waiter.reject(new Error(JSON.stringify(message.error)));
    else waiter.resolve(message.result ?? {});
  });
  socket.addEventListener("close", () => rejectPending(new Error("CDP WebSocket closed")));
  socket.addEventListener("error", () => rejectPending(new Error("CDP WebSocket failed")));

  async function call(method, params = {}, milliseconds = 60_000) {
    if (socket.readyState !== WebSocket.OPEN) throw new Error(`Cannot call ${method}: CDP socket is not open`);
    const id = nextId++;
    const response = new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
    socket.send(JSON.stringify({ id, method, params }));
    try {
      return await withTimeout(response, `CDP ${method}`, milliseconds);
    } finally {
      pending.delete(id);
    }
  }

  async function evaluate(expression) {
    const result = await call("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
      userGesture: true,
      allowUnsafeEvalBlockedByCSP: true,
    });
    if (result.exceptionDetails) {
      throw new Error(
        result.exceptionDetails.exception?.description ?? result.exceptionDetails.text ?? "page evaluation failed",
      );
    }
    return result.result?.value ?? result.result?.description;
  }

  const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

  async function waitFor(expression, label, milliseconds = 20_000, interval = 100) {
    const deadline = Date.now() + milliseconds;
    let last;
    let lastError = null;
    while (Date.now() < deadline) {
      try {
        last = await evaluate(expression);
        lastError = null;
        if (last) return last;
      } catch (error) {
        // A full Page.reload briefly destroys the JavaScript execution context.
        // Treat only that polling window as not-ready; a persistent expression
        // error is still reported when the deadline expires.
        lastError = error;
      }
      await sleep(interval);
    }
    throw new Error(
      `${label} did not become ready in ${milliseconds}ms; last=${JSON.stringify(last)}` +
        (lastError ? `; lastError=${lastError instanceof Error ? lastError.message : String(lastError)}` : ""),
    );
  }

  async function lockMetrics() {
    await call("Emulation.setDeviceMetricsOverride", {
      width: 1280,
      height: 860,
      deviceScaleFactor: 2,
      mobile: false,
      screenWidth: 1280,
      screenHeight: 860,
    });
  }

  async function setCompactMetrics() {
    await call("Emulation.setDeviceMetricsOverride", {
      width: 768,
      height: 516,
      deviceScaleFactor: 10 / 3,
      mobile: false,
      screenWidth: 768,
      screenHeight: 516,
    });
  }

  async function screenshot(targetPath) {
    const result = await call("Page.captureScreenshot", {
      format: "png",
      fromSurface: true,
      captureBeyondViewport: false,
    });
    const bytes = Buffer.from(result.data, "base64");
    await fs.mkdir(path.dirname(targetPath), { recursive: true });
    await fs.writeFile(targetPath, bytes);
    return bytes.byteLength;
  }

  async function mouse(type, x, y, extra = {}) {
    return await call("Input.dispatchMouseEvent", {
      type,
      x,
      y,
      button: extra.button ?? "left",
      buttons: extra.buttons ?? (type === "mouseMoved" ? 0 : 1),
      clickCount: type === "mouseMoved" ? 0 : 1,
      modifiers: extra.modifiers ?? 0,
      ...extra,
    });
  }

  async function click(x, y, modifiers = 0) {
    await mouse("mouseMoved", x, y, { buttons: 0, modifiers });
    await sleep(40);
    await mouse("mousePressed", x, y, { buttons: 1, modifiers });
    await sleep(60);
    await mouse("mouseReleased", x, y, { buttons: 0, modifiers });
    await sleep(180);
  }

  async function drag(x0, y0, x1, y1, steps = 14, modifiers = 0, release = true) {
    await mouse("mouseMoved", x0, y0, { buttons: 0, modifiers });
    await mouse("mousePressed", x0, y0, { buttons: 1, modifiers });
    for (let step = 1; step <= steps; step += 1) {
      const fraction = step / steps;
      await mouse("mouseMoved", x0 + (x1 - x0) * fraction, y0 + (y1 - y0) * fraction, {
        buttons: 1,
        modifiers,
      });
      await sleep(20);
    }
    if (release) {
      await mouse("mouseReleased", x1, y1, { buttons: 0, modifiers });
      await sleep(220);
    }
  }

  async function releaseMouse(x, y, modifiers = 0) {
    await mouse("mouseReleased", x, y, { buttons: 0, modifiers });
    await sleep(180);
  }

  async function hover(x, y) {
    await mouse("mouseMoved", x, y, { buttons: 0 });
    await sleep(80);
    await mouse("mouseMoved", x + 1, y, { buttons: 0 });
  }

  async function key(keyValue, code, keyCode, modifiers = 0) {
    for (const type of ["keyDown", "keyUp"]) {
      await call("Input.dispatchKeyEvent", {
        type,
        key: keyValue,
        code,
        windowsVirtualKeyCode: keyCode,
        nativeVirtualKeyCode: keyCode,
        modifiers,
      });
      await sleep(type === "keyDown" ? 30 : 120);
    }
  }

  async function stable() {
    await evaluate(`(async () => {
      const api = window.__origami3Capture;
      if (api && typeof api.waitForStable === "function") await api.waitForStable();
      else await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      return true;
    })()`);
    await sleep(250);
  }

  async function waitForCaptureApi(milliseconds = 20_000) {
    return await waitFor(
      `(() => {
        const api = window.__origami3Capture;
        if (!api || typeof api.getStatus !== "function") return false;
        const status = api.getStatus();
        return status?.version === 1 && status.ready === true ? status : false;
      })()`,
      "ORIGAMI3 capture API",
      milliseconds,
    );
  }

  async function clickText(scope, text, index = 0) {
    const result = await evaluate(`(() => {
      const wanted = ${JSON.stringify(text.replace(/\s+/g, ""))};
      const items = Array.from(document.querySelectorAll(${JSON.stringify(scope)}))
        .filter((element) => (element.textContent || "").replace(/\s+/g, "").includes(wanted));
      const element = items[${index}];
      if (!element) return "notfound:" + items.length;
      if (element.disabled) return "disabled";
      element.click();
      return "ok";
    })()`);
    if (result !== "ok") throw new Error(`clickText(${scope}, ${text}, ${index}) -> ${result}`);
    await sleep(300);
  }

  async function center(scope, text = null, index = 0) {
    return await evaluate(`(() => {
      let items = Array.from(document.querySelectorAll(${JSON.stringify(scope)}));
      ${
        text === null
          ? ""
          : `items = items.filter((element) => (element.textContent || "").replace(/\\s+/g, "").includes(${JSON.stringify(
              text.replace(/\s+/g, ""),
            )}));`
      }
      const element = items[${index}];
      if (!element) return null;
      const rect = element.getBoundingClientRect();
      return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2, w: rect.width, h: rect.height,
        left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom };
    })()`);
  }

  async function setSelect(labelText, optionText) {
    const result = await evaluate(`(() => {
      const wanted = ${JSON.stringify(labelText.replace(/\s+/g, ""))};
      const nameOf = (select) => {
        const label = select.closest("label") || (select.id && document.querySelector('label[for="' + select.id + '"]'));
        return [select.title, select.getAttribute("aria-label"), label?.textContent || ""].join(" ");
      };
      const select = Array.from(document.querySelectorAll("select"))
        .find((candidate) => nameOf(candidate).replace(/\s+/g, "").includes(wanted));
      if (!select) return "notfound";
      const option = Array.from(select.options).find((candidate) => (candidate.textContent || "").trim() === ${JSON.stringify(
        optionText,
      )});
      if (!option) return "nooption";
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set;
      setter.call(select, option.value);
      select.dispatchEvent(new Event("change", { bubbles: true }));
      return "ok";
    })()`);
    if (result !== "ok") throw new Error(`setSelect(${labelText}, ${optionText}) -> ${result}`);
    await sleep(500);
  }

  async function setRange(labelText, value) {
    const result = await evaluate(`(() => {
      const wanted = ${JSON.stringify(labelText.replace(/\s+/g, ""))};
      const inputs = Array.from(document.querySelectorAll('input[type="range"]'));
      const input = inputs.find((candidate) => {
        const label = candidate.closest("label") || (candidate.id && document.querySelector('label[for="' + candidate.id + '"]'));
        const name = label?.textContent || candidate.getAttribute("aria-label") || candidate.title || "";
        return name.replace(/\s+/g, "").includes(wanted);
      });
      if (!input) return "notfound:" + inputs.length;
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
      setter.call(input, ${JSON.stringify(String(value))});
      input.dispatchEvent(new Event("input", { bubbles: true }));
      input.dispatchEvent(new Event("change", { bubbles: true }));
      return "ok:" + input.value;
    })()`);
    if (!String(result).startsWith("ok:")) throw new Error(`setRange(${labelText}, ${value}) -> ${result}`);
    await sleep(650);
  }

  async function closeDialogs() {
    for (let attempt = 0; attempt < 8; attempt += 1) {
      const result = await evaluate(`(() => {
        const root = document.querySelector('.dialog-backdrop, .dialog, .help-center, [role="dialog"]');
        if (!root) return "none";
        const words = ["やめる", "取り消し", "キャンセル", "閉じる", "×"];
        const buttons = Array.from(root.querySelectorAll("button"));
        for (const word of words) {
          const button = buttons.find((candidate) =>
            (candidate.textContent || "").trim() === word || candidate.getAttribute("aria-label") === word || candidate.title === word);
          if (button && !button.disabled) { button.click(); return "clicked:" + word; }
        }
        return "stuck";
      })()`);
      if (result === "none") return;
      if (result === "stuck") await key("Escape", "Escape", 27);
      await sleep(350);
    }
    throw new Error("could not close all dialogs");
  }

  async function neutralMouse() {
    await evaluate(`(() => { document.activeElement?.blur?.(); return true; })()`);
    await mouse("mouseMoved", 1270, 855, { buttons: 0 });
    await sleep(120);
    await mouse("mouseMoved", 1275, 858, { buttons: 0 });
    await sleep(750);
  }

  async function collapseHints() {
    for (const selector of [
      ".cp-help-toggle",
      ".viewer-hint-toggle",
      ".operation-help-toggle",
      ".paper-help-toggle",
    ]) {
      await evaluate(`(() => {
        const button = document.querySelector(${JSON.stringify(selector)});
        if (button && (button.textContent || "").includes("▲")) button.click();
        return true;
      })()`);
      await sleep(150);
    }
  }

  async function resetScroll() {
    await evaluate(`(() => {
      for (const element of document.querySelectorAll("*")) {
        if (element.scrollTop > 0 && element.scrollHeight > element.clientHeight + 2) element.scrollTop = 0;
        if (element.scrollLeft > 0 && element.scrollWidth > element.clientWidth + 2) element.scrollLeft = 0;
      }
      return true;
    })()`);
    await sleep(180);
  }

  async function overlayHitTest(enabled) {
    await evaluate(`(() => {
      for (const selector of [".cp-operation-hint", ".cp-step-indicator"]) {
        const root = document.querySelector(selector);
        if (!root) continue;
        for (const element of [root, ...root.querySelectorAll("*")]) {
          element.style.pointerEvents = ${enabled ? '""' : '"none"'};
        }
      }
      return true;
    })()`);
    await sleep(120);
  }

  async function reload() {
    await call("Page.reload", { ignoreCache: true });
    await waitFor(`document.readyState === "complete" || document.readyState === "interactive"`, "page reload", 20_000);
  }

  await initializeManualCaptureConnection({ call, lockMetrics, close });

  return {
    target: candidates[0],
    call,
    evaluate,
    waitFor,
    sleep,
    lockMetrics,
    setCompactMetrics,
    screenshot,
    mouse,
    click,
    drag,
    releaseMouse,
    hover,
    key,
    stable,
    waitForCaptureApi,
    clickText,
    center,
    setSelect,
    setRange,
    closeDialogs,
    neutralMouse,
    collapseHints,
    resetScroll,
    overlayHitTest,
    reload,
    close,
  };
}
