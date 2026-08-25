// WebView2の本番WebGL描画で、太い強調線が接していない紙面へ漏れないことを測る。
// ブラウザやPlaywrightは使わず、起動済みdesktop.exeのCDPへ接続する。
//
// 必須:
//   ORI3_DESKTOP_PID=<いま起動しているdesktop.exeのPID>
//   ORI3_DESKTOP_EXE=<同じプロセスのdesktop.exe実パス>
// 任意:
//   ORI3_CDP_ENDPOINT=http://127.0.0.1:9222

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, statSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const endpoint = process.env.ORI3_CDP_ENDPOINT ?? "http://127.0.0.1:9222";
const repositoryRoot = fileURLToPath(new URL("../../../../../", import.meta.url));
const documentPath = resolve(
  repositoryRoot,
  "verification",
  "hidden-crease-20260825",
  "reproduction-document.ori3",
);
const desktopPid = Number(process.env.ORI3_DESKTOP_PID);
const claimedExecutablePath = process.env.ORI3_DESKTOP_EXE
  ? resolve(process.env.ORI3_DESKTOP_EXE)
  : undefined;
const edgeId = 19;
const expectedDocumentSha256 =
  "6A62DA884FEA65D54D79C8B7BF7D5D538374C4DB326448613116040127E0CF1F";
const emulatedViewport = { width: 1280, height: 860, deviceScaleFactor: 2, mobile: false };
const expectedCanvasPhysical = [1210, 872];
const angleOperations = [21, 22, 25, 26, 29, 30, 33, 34, 35, 36].map((hinge) => ({
  hinge,
  deg: 180,
}));
// 修正前にedge 19で adjacent=461 / foreign=36を実測した視点を固定する。
const reproductionCamera = {
  position: [0.8885659352892843, -0.44366012855969017, 1.0546789672137715],
  quaternion: [
    0.3368345828993243,
    0.1649832215370013,
    -0.060074771765839474,
    0.9250481188411731,
  ],
  up: [0.22228821844721058, 0.7658669711206413, 0.6033537357852857],
  near: 0.01,
  far: 100,
  fov: 45,
  aspect: 1.3661931175533442,
  zoom: 1,
  projectionMatrix: [
    1.7671100310449557,
    0,
    0,
    0,
    0,
    2.4520678183077944,
    0,
    0,
    0,
    0.015679746201694697,
    -1.0002000200020003,
    -1,
    0,
    0,
    -0.020002000200020003,
    0,
  ],
};

if (!Number.isSafeInteger(desktopPid) || desktopPid <= 0) {
  throw new Error("ORI3_DESKTOP_PIDに、起動中desktop.exeのPIDを指定してください");
}
if (!claimedExecutablePath) {
  throw new Error("ORI3_DESKTOP_EXEに、起動中desktop.exeの実パスを指定してください");
}
const activeExecutablePath = resolve(
  execFileSync(
    "powershell.exe",
    [
      "-NoLogo",
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      `(Get-Process -Id ${desktopPid} -ErrorAction Stop).Path`,
    ],
    { encoding: "utf8" },
  ).trim(),
);
if (activeExecutablePath.toLowerCase() !== claimedExecutablePath.toLowerCase()) {
  throw new Error(
    `PID ${desktopPid}の実行ファイルが指定値と一致しません: actual=${activeExecutablePath}, claimed=${claimedExecutablePath}`,
  );
}

const executable = {
  pid: desktopPid,
  path: activeExecutablePath,
  size: statSync(activeExecutablePath).size,
  sha256: createHash("sha256")
    .update(readFileSync(activeExecutablePath))
    .digest("hex")
    .toUpperCase(),
};
const documentFile = {
  path: documentPath,
  size: statSync(documentPath).size,
  sha256: createHash("sha256").update(readFileSync(documentPath)).digest("hex").toUpperCase(),
};
if (documentFile.sha256 !== expectedDocumentSha256) {
  throw new Error(
    `恒久fixtureのSHA-256が不一致です: actual=${documentFile.sha256}, expected=${expectedDocumentSha256}`,
  );
}

const endpointUrl = new URL(endpoint);
if (endpointUrl.protocol !== "http:" || endpointUrl.hostname !== "127.0.0.1") {
  throw new Error(`CDP endpointは127.0.0.1のHTTPに限定します: ${endpoint}`);
}
const endpointPort = Number(endpointUrl.port || 80);
const listenerPids = [
  ...new Set(
    execFileSync("netstat.exe", ["-ano", "-p", "tcp"], { encoding: "utf8" })
      .split(/\r?\n/u)
      .map((line) => line.trim().split(/\s+/u))
      .filter(
        (fields) =>
          fields.length >= 5 &&
          fields[0] === "TCP" &&
          fields[1] === `${endpointUrl.hostname}:${endpointPort}` &&
          fields[3] === "LISTENING",
      )
      .map((fields) => Number(fields[4]))
      .filter((pid) => Number.isSafeInteger(pid) && pid > 0),
  ),
];
if (listenerPids.length !== 1) {
  throw new Error(`CDP listener PIDを一意に特定できません: ${listenerPids.join(",") || "none"}`);
}
const cdpListenerPid = listenerPids[0];
const cdpListenerParentPid = Number(
  execFileSync(
    "powershell.exe",
    [
      "-NoLogo",
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      [
        `$listenerPid = ${cdpListenerPid}`,
        "$samples = (Get-Counter '\\Process(msedgewebview2*)\\ID Process','\\Process(msedgewebview2*)\\Creating Process ID' -ErrorAction Stop).CounterSamples",
        "$id = $samples | Where-Object { $_.Path.EndsWith('\\id process', [System.StringComparison]::OrdinalIgnoreCase) -and [int]$_.CookedValue -eq $listenerPid } | Select-Object -First 1",
        "if ($null -eq $id) { throw 'CDP listener process counter not found' }",
        "$instancePath = $id.Path.Substring(0, $id.Path.LastIndexOf('\\'))",
        "$parentPath = $instancePath + '\\creating process id'",
        "$parent = $samples | Where-Object { $_.Path.Equals($parentPath, [System.StringComparison]::OrdinalIgnoreCase) } | Select-Object -First 1",
        "if ($null -eq $parent) { throw 'CDP listener parent counter not found' }",
        "[int]$parent.CookedValue",
      ].join("\n"),
    ],
    { encoding: "utf8" },
  ).trim(),
);
if (cdpListenerParentPid !== desktopPid) {
  throw new Error(
    `CDP listener PID ${cdpListenerPid}の親が指定desktop PIDではありません: parent=${cdpListenerParentPid}, desktop=${desktopPid}`,
  );
}

const targets = await fetch(`${endpoint}/json/list`).then((response) => {
  if (!response.ok) throw new Error(`CDP /json/list: HTTP ${response.status}`);
  return response.json();
});
const page = targets.find(
  (target) =>
    target.type === "page" &&
    target.url === "http://tauri.localhost/" &&
    typeof target.webSocketDebuggerUrl === "string",
);
if (!page) throw new Error("CDP page targetがありません");

const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  socket.addEventListener("open", resolve, { once: true });
  socket.addEventListener("error", reject, { once: true });
});
let nextId = 1;
const pending = new Map();
socket.addEventListener("message", (event) => {
  const message = JSON.parse(String(event.data));
  if (!message.id) return;
  const waiter = pending.get(message.id);
  if (!waiter) return;
  pending.delete(message.id);
  if (message.error) waiter.reject(new Error(JSON.stringify(message.error)));
  else waiter.resolve(message.result);
});
const call = (method, params = {}) =>
  new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params }));
  });

await call("Runtime.enable");
const expression = `(${async function measureOwnerPixels(input) {
  const captureApi = window.__origami3Capture;
  if (!captureApi) throw new Error("__origami3Captureがありません");
  const openReproductionPose = async () => {
    await captureApi.openDocument(input.documentPath);
    await captureApi.runAnglePath(input.angleOperations);
    await captureApi.setView("normal");
    await captureApi.waitForStable();
  };
  let scene = null;
  try {
    await openReproductionPose();

  if (
    innerWidth !== input.viewport.width ||
    innerHeight !== input.viewport.height ||
    devicePixelRatio !== input.viewport.deviceScaleFactor
  ) {
    throw new Error(
      `CDP viewportが不一致です: ${innerWidth}x${innerHeight}@${devicePixelRatio}`,
    );
  }

  const canvas = document.querySelector("canvas.viewer3d-canvas");
  const fiberKey = canvas && Object.keys(canvas).find((key) => key.startsWith("__reactFiber$"));
  if (!canvas || !fiberKey) throw new Error("3D canvasのReact fiberがありません");
  let fiber = canvas[fiberKey];
  while (fiber && !scene) {
    let hook = fiber.memoizedState;
    while (hook) {
      const current = hook.memoizedState?.current;
      if (
        current?.camera?.isPerspectiveCamera &&
        current?.content?.hingeSegments &&
        typeof current.setHighlight === "function" &&
        typeof current.render === "function"
      ) {
        scene = current;
        break;
      }
      hook = hook.next;
    }
    fiber = fiber.return;
  }
  if (!scene) throw new Error("Viewer3D sceneがありません");
  scene.camera.position.fromArray(input.camera.position);
  scene.camera.quaternion.fromArray(input.camera.quaternion);
  scene.camera.up.fromArray(input.camera.up);
  scene.camera.near = input.camera.near;
  scene.camera.far = input.camera.far;
  scene.camera.fov = input.camera.fov;
  scene.camera.aspect = input.camera.aspect;
  scene.camera.zoom = input.camera.zoom;
  scene.camera.updateProjectionMatrix();
  scene.camera.projectionMatrix.fromArray(input.camera.projectionMatrix);
  scene.camera.projectionMatrixInverse.copy(scene.camera.projectionMatrix).invert();
  scene.camera.updateMatrixWorld(true);
  scene.render();

  if (
    canvas.width !== input.expectedCanvasPhysical[0] ||
    canvas.height !== input.expectedCanvasPhysical[1]
  ) {
    throw new Error(
      `3D canvas physical size mismatch: ${canvas.width}x${canvas.height}, expected ${input.expectedCanvasPhysical.join("x")}`,
    );
  }

  const source = scene.content.hingeSegments.filter((segment) => segment.edgeId === input.edgeId);
  if (source.length === 0) throw new Error(`edge ${input.edgeId}の表示線分がありません`);
  const expectedFaces = [...new Set(source.map((segment) => segment.ownerFace))].filter(
    (face) => Number.isSafeInteger(face),
  );
  if (expectedFaces.length === 0) throw new Error(`edge ${input.edgeId}にowner faceがありません`);

    const decode = (base64) => {
      const binary = atob(base64);
      const bytes = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
      return bytes;
    };
    const assertReadback = (readback, label) => {
      if (
        readback?.version !== 1 ||
        readback.rowOrder !== "bottom-to-top" ||
        readback.owner?.encoding !== "rgba8-base64" ||
        readback.depth?.encoding !== "rgba8-packed-depth-base64" ||
        readback.final?.encoding !== "rgba8-base64"
      ) {
        throw new Error(`${label}: readback契約が不一致です`);
      }
      const expectedLength = readback.width * readback.height * 4;
      const owner = decode(readback.owner.data);
      const depth = decode(readback.depth.data);
      const final = decode(readback.final.data);
      if (
        owner.length !== expectedLength ||
        depth.length !== expectedLength ||
        final.length !== expectedLength
      ) {
        throw new Error(
          `${label}: readback byte長が不一致です: owner=${owner.length}, depth=${depth.length}, final=${final.length}, expected=${expectedLength}`,
        );
      }
      return { owner, final };
    };
    const capture = () => captureApi.captureCanonical3D();
    const nextFrame = () => new Promise((resolve) => requestAnimationFrame(() => resolve()));

    scene.setHighlight([]);
    scene.render();
    await nextFrame();
    const baseline = capture();
    const baselineDecoded = assertReadback(baseline.readback, "baseline");
    const baselineFinal = baselineDecoded.final;
    const baselineOwner = baseline.readback.owner.data;
    const baselineCodeToFace = JSON.stringify(baseline.readback.owner.codeToFace);
    const width = baseline.readback.width;
    const height = baseline.readback.height;
    const ownerRoles = ["hinge", "reference", "focus", "active", "pinned", "pinMark"];
    const roles = [...ownerRoles, "suspect"];
    const measurements = [];

    for (const role of roles) {
      scene.setHighlight(source.map((segment) => ({ ...segment, role })));
      scene.render();
      await nextFrame();
      const selected = capture();
      const selectedDecoded = assertReadback(selected.readback, role);
      if (selected.readback.width !== width || selected.readback.height !== height) {
        throw new Error(`${role}: readback寸法が途中で変わりました`);
      }
      if (selected.readback.owner.data !== baselineOwner) {
        throw new Error(`${role}: highlightだけの変更でowner bufferが変わりました`);
      }
      if (JSON.stringify(selected.readback.owner.codeToFace) !== baselineCodeToFace) {
        throw new Error(`${role}: highlightだけの変更でowner code表が変わりました`);
      }
      const final = selectedDecoded.final;
      const owner = selectedDecoded.owner;
      const codeToFace = new Map(selected.readback.owner.codeToFace);
      const counts = { changed: 0, adjacentPaper: 0, foreignPaper: 0, background: 0 };
      for (let pixel = 0; pixel < width * height; pixel++) {
        const at = pixel * 4;
        if (
          final[at] === baselineFinal[at] &&
          final[at + 1] === baselineFinal[at + 1] &&
          final[at + 2] === baselineFinal[at + 2] &&
          final[at + 3] === baselineFinal[at + 3]
        ) {
          continue;
        }
        counts.changed += 1;
        const code =
          owner[at] +
          owner[at + 1] * 256 +
          owner[at + 2] * 65536 +
          owner[at + 3] * 16777216;
        const face = codeToFace.get(code);
        if (code === 0) counts.background += 1;
        else if (face === undefined) throw new Error(`${role}: 未登録owner code ${code}`);
        else if (expectedFaces.includes(face)) counts.adjacentPaper += 1;
        else counts.foreignPaper += 1;
      }
      measurements.push({ role, ...counts });
    }

    const ownerFailures = measurements
      .filter((measurement) => ownerRoles.includes(measurement.role))
      .flatMap((measurement) => {
        const failures = [];
        if (measurement.foreignPaper !== 0) {
          failures.push(`${measurement.role}: foreignPaper=${measurement.foreignPaper}`);
        }
        if (measurement.adjacentPaper <= 0) {
          failures.push(`${measurement.role}: adjacentPaper=${measurement.adjacentPaper}`);
        }
        return failures;
      });
    const suspect = measurements.find((measurement) => measurement.role === "suspect");
    if (!suspect || suspect.foreignPaper <= 0) {
      ownerFailures.push(`suspect: foreignPaper=${suspect?.foreignPaper ?? "missing"}`);
    }

    const rect = canvas.getBoundingClientRect();
    return {
      passed: ownerFailures.length === 0,
      failures: ownerFailures,
      edgeId: input.edgeId,
      sourceSegmentCount: source.length,
      expectedFaces,
      measurements,
      viewport: {
        canvasPhysical: [canvas.width, canvas.height],
        canvasCss: [rect.width, rect.height],
        devicePixelRatio,
      },
      camera: {
        position: scene.camera.position.toArray(),
        direction: scene.camera.getWorldDirection(scene.camera.position.clone()).toArray(),
        up: scene.camera.up.toArray(),
        quaternion: scene.camera.quaternion.toArray(),
        matrixWorld: scene.camera.matrixWorld.toArray(),
        projectionMatrix: scene.camera.projectionMatrix.toArray(),
        near: scene.camera.near,
        far: scene.camera.far,
        fov: scene.camera.fov,
        aspect: scene.camera.aspect,
        zoom: scene.camera.zoom,
      },
    };
  } finally {
    if (scene) {
      scene.setHighlight([]);
      scene.render();
    }
    await openReproductionPose();
  }
}})(${JSON.stringify({
  documentPath,
  edgeId,
  viewport: emulatedViewport,
  expectedCanvasPhysical,
  angleOperations,
  camera: reproductionCamera,
})})`;

const uiStateExpression = `(${async function readUiState() {
  await new Promise((resolve) =>
    requestAnimationFrame(() => requestAnimationFrame(() => requestAnimationFrame(resolve))),
  );
  const canvas = document.querySelector("canvas.viewer3d-canvas");
  const rect = canvas?.getBoundingClientRect();
  return {
    innerWidth,
    innerHeight,
    devicePixelRatio,
    captureView: document.documentElement.getAttribute("data-origami3-capture-view"),
    canvasPhysical: canvas ? [canvas.width, canvas.height] : null,
    canvasCss: rect ? [rect.width, rect.height] : null,
  };
}})()`;
const originalUiEvaluation = await call("Runtime.evaluate", {
  expression: uiStateExpression,
  awaitPromise: true,
  returnByValue: true,
});
if (originalUiEvaluation.exceptionDetails) {
  throw new Error(
    originalUiEvaluation.exceptionDetails.exception?.description ??
      originalUiEvaluation.exceptionDetails.text,
  );
}
const originalUi = originalUiEvaluation.result?.value;

await call("Emulation.setDeviceMetricsOverride", emulatedViewport);
let evaluation;
let restoration;
try {
  evaluation = await call("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
} finally {
  try {
    await call("Emulation.clearDeviceMetricsOverride");
    const restorationEvaluation = await call("Runtime.evaluate", {
      expression: uiStateExpression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (restorationEvaluation.exceptionDetails) {
      throw new Error(
        restorationEvaluation.exceptionDetails.exception?.description ??
          restorationEvaluation.exceptionDetails.text,
      );
    }
    restoration = restorationEvaluation.result?.value;
    if (
      restoration.captureView !== null ||
      restoration.innerWidth !== originalUi.innerWidth ||
      restoration.innerHeight !== originalUi.innerHeight ||
      restoration.devicePixelRatio !== originalUi.devicePixelRatio ||
      JSON.stringify(restoration.canvasPhysical) !== JSON.stringify(originalUi.canvasPhysical) ||
      JSON.stringify(restoration.canvasCss) !== JSON.stringify(originalUi.canvasCss)
    ) {
      throw new Error(
        `CDP測定後のUI復元が不一致です: original=${JSON.stringify(originalUi)}, restored=${JSON.stringify(restoration)}`,
      );
    }
  } finally {
    socket.close();
  }
}
if (evaluation.exceptionDetails) {
  throw new Error(evaluation.exceptionDetails.exception?.description ?? evaluation.exceptionDetails.text);
}

const measurement = evaluation.result?.value;
const report = {
  version: 1,
  capturedAt: new Date().toISOString(),
  endpoint,
  cdpBinding: {
    listenerPid: cdpListenerPid,
    listenerParentPid: cdpListenerParentPid,
    desktopPid,
    verified: true,
  },
  restoration: { original: originalUi, restored: restoration, verified: true },
  page: { id: page.id, title: page.title, url: page.url },
  executable,
  document: documentFile,
  ...measurement,
};
console.log(JSON.stringify(report, null, 2));
if (!report.passed) process.exitCode = 1;
