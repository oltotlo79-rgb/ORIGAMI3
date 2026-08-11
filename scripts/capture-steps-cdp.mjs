import fs from "node:fs/promises";
import path from "node:path";

const [
  ,
  ,
  actionsPath,
  endpoint = "http://127.0.0.1:9222",
  expectedTargetIdArgument,
  expectedGenerationArgument,
] = process.argv;
if (!actionsPath) {
  throw new Error(
    "usage: node capture-steps-cdp.mjs <actions.json> [endpoint] [expectedTargetId] [expectedGeneration]",
  );
}
function optionalExpectation(argument) {
  const value = argument?.trim();
  return value && value !== "-" ? value : undefined;
}

const expectedTargetId = optionalExpectation(expectedTargetIdArgument);
const expectedGeneration = optionalExpectation(expectedGenerationArgument);

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

function isOrigamiTarget(candidate) {
  if (candidate.title === "ORIGAMI3") return true;
  try {
    const candidateUrl = new URL(candidate.url);
    return (
      (candidateUrl.hostname === "localhost" || candidateUrl.hostname === "127.0.0.1") &&
      candidateUrl.port === "1420"
    );
  } catch {
    return false;
  }
}

const candidates = expectedTargetId
  ? pages.filter((candidate) => candidate.id === expectedTargetId)
  : pages.filter(isOrigamiTarget);
if (candidates.length !== 1) {
  const selection = expectedTargetId
    ? `with id ${JSON.stringify(expectedTargetId)}`
    : "matching ORIGAMI3";
  throw new Error(
    `Expected one CDP page target ${selection}, found ${candidates.length} (${pages.length} total page target(s))`,
  );
}
const target = candidates[0];
const targetInfo = {
  id: target.id ?? "",
  url: target.url ?? "",
  title: target.title ?? "",
};

// WebView2 sometimes advertises `localhost` even when the debugger is listening
// only on IPv4. Match the already-working discovery endpoint to avoid an IPv6
// connection stall on Windows.
const endpointUrl = new URL(endpoint);
const socketUrl = new URL(target.webSocketDebuggerUrl);
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

async function evaluate(expression) {
  const result = await call("Runtime.evaluate", {
    expression,
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

async function assertExpectedGeneration() {
  if (!expectedGeneration) return;
  const status = await evaluate(`(() => {
    const api = window.__origami3Capture;
    if (!api || typeof api.getStatus !== "function") return null;
    return api.getStatus();
  })()`);
  if (
    !status ||
    status.version !== 1 ||
    status.ready !== true ||
    status.generation !== expectedGeneration
  ) {
    const actualGeneration = status?.generation ?? "unavailable";
    throw new Error(
      `ORIGAMI3 capture generation changed: expected ${JSON.stringify(expectedGeneration)}, got ${JSON.stringify(actualGeneration)}`,
    );
  }
}

async function run(action, index) {
  if (action.act === "eval") {
    return await evaluate(action.js);
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
  const actions = JSON.parse(await fs.readFile(actionsPath, "utf8"));
  for (let index = 0; index < actions.length; index += 1) {
    const action = actions[index];
    await assertExpectedGeneration();
    const result = await run(action, index);
    process.stdout.write(
      `${JSON.stringify({ index, act: action.act, target: targetInfo, result })}\n`,
    );
  }
} finally {
  if (socket.readyState === WebSocket.OPEN) socket.close();
}
