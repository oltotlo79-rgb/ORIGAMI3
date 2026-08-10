import fs from "node:fs/promises";
import path from "node:path";

const [, , actionsPath, endpoint = "http://127.0.0.1:9222"] = process.argv;
if (!actionsPath) {
  throw new Error("usage: node capture-steps-cdp.mjs <actions.json> [endpoint]");
}

const timeout = (label, milliseconds) => {
  let handle;
  const promise = new Promise((_, reject) => {
    handle = setTimeout(() => reject(new Error(`${label} timed out`)), milliseconds);
  });
  return { promise, cancel: () => clearTimeout(handle) };
};

async function withTimeout(promise, label, milliseconds) {
  const timer = timeout(label, milliseconds);
  try {
    return await Promise.race([promise, timer.promise]);
  } finally {
    timer.cancel();
  }
}

const discovery = await withTimeout(
  fetch(`${endpoint}/json/list`),
  "CDP target discovery",
  5_000,
);
if (!discovery.ok) {
  throw new Error(`CDP target discovery failed: ${discovery.status}`);
}
const targets = await discovery.json();
const pages = targets.filter(
  (candidate) => candidate.type === "page" && candidate.webSocketDebuggerUrl,
);
if (pages.length !== 1) {
  throw new Error(`Expected one CDP page target, found ${pages.length}`);
}

// WebView2 sometimes advertises `localhost` even when the debugger is listening
// only on IPv4. Match the already-working discovery endpoint to avoid an IPv6
// connection stall on Windows.
const endpointUrl = new URL(endpoint);
const socketUrl = new URL(pages[0].webSocketDebuggerUrl);
socketUrl.hostname = endpointUrl.hostname;
socketUrl.port = endpointUrl.port;

const socket = new WebSocket(socketUrl);
await withTimeout(
  new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  }),
  "CDP WebSocket connection",
  5_000,
);

let nextId = 1;
const pending = new Map();

function rejectPending(error) {
  for (const waiter of pending.values()) waiter.reject(error);
  pending.clear();
}

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

async function call(method, params = {}) {
  if (socket.readyState !== WebSocket.OPEN) {
    throw new Error(`Cannot call ${method}: CDP WebSocket is not open`);
  }
  const id = nextId++;
  const response = new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
  });
  socket.send(JSON.stringify({ id, method, params }));
  try {
    return await withTimeout(response, `CDP ${method}`, 30_000);
  } finally {
    pending.delete(id);
  }
}

async function run(action, index) {
  if (action.act === "eval") {
    const result = await call("Runtime.evaluate", {
      expression: action.js,
      awaitPromise: true,
      returnByValue: true,
      userGesture: true,
    });
    if (result.exceptionDetails) {
      throw new Error(
        result.exceptionDetails.exception?.description ?? result.exceptionDetails.text,
      );
    }
    return result.result?.value ?? result.result?.description;
  }
  if (action.act === "shot") {
    const result = await call("Page.captureScreenshot", {
      format: "png",
      fromSurface: true,
      captureBeyondViewport: false,
    });
    await fs.mkdir(path.dirname(action.path), { recursive: true });
    const image = Buffer.from(result.data, "base64");
    await fs.writeFile(action.path, image);
    return { path: action.path, bytes: image.byteLength };
  }
  throw new Error(`Unsupported action at ${index}: ${action.act}`);
}

try {
  await call("Runtime.enable");
  await call("Page.enable");
  await call("Page.bringToFront");
  const actions = JSON.parse(await fs.readFile(actionsPath, "utf8"));
  for (let index = 0; index < actions.length; index += 1) {
    const action = actions[index];
    const result = await run(action, index);
    process.stdout.write(`${JSON.stringify({ index, act: action.act, result })}\n`);
  }
} finally {
  if (socket.readyState === WebSocket.OPEN) socket.close();
}
