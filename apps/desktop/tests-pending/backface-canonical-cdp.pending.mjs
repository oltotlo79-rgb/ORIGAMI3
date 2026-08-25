// 通常suiteへ自動登録せず、同梱版を統括が起動した時だけ手動で走らせるCDP検査。
// ブラウザやdesktop.exeは起動せず、既存WebView2の9222番へ接続する。

import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { isDeepStrictEqual } from "node:util";
import { fileURLToPath } from "node:url";
import path from "node:path";

const cdpPort = Number.parseInt(process.env.ORI3_CDP_PORT ?? "9222", 10);
const fixturePath = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../crates/ori3-rigid/tests/fixtures/sa-warm-path.ori3",
);
const expectedDesired = [
  [17, -90],
  [19, 90],
  [21, 90],
];
const paths = {
  P: [
    { hinge: 17, deg: -90 },
    { hinge: 19, deg: 90 },
    { hinge: 21, deg: 90 },
  ],
  Q: [
    { hinge: 21, deg: 90 },
    { hinge: 19, deg: 90 },
    { hinge: 17, deg: -90 },
  ],
};

class CdpConnection {
  constructor(socket) {
    this.socket = socket;
    this.nextId = 1;
    this.pending = new Map();
    socket.addEventListener("message", (event) => void this.onMessage(event));
    socket.addEventListener("close", () => {
      for (const { reject } of this.pending.values()) {
        reject(new Error("CDP接続が応答前に閉じました"));
      }
      this.pending.clear();
    });
  }

  static async connect(url) {
    const socket = new WebSocket(url);
    await new Promise((resolve, reject) => {
      socket.addEventListener("open", resolve, { once: true });
      socket.addEventListener(
        "error",
        () => reject(new Error(`CDP WebSocketへ接続できません: ${url}`)),
        { once: true },
      );
    });
    return new CdpConnection(socket);
  }

  async onMessage(event) {
    let text;
    if (typeof event.data === "string") text = event.data;
    else if (event.data instanceof Blob) text = await event.data.text();
    else text = Buffer.from(event.data).toString("utf8");
    const message = JSON.parse(text);
    if (message.id === undefined) return;
    const pending = this.pending.get(message.id);
    if (!pending) return;
    this.pending.delete(message.id);
    if (message.error) {
      pending.reject(new Error(`CDP ${message.error.code}: ${message.error.message}`));
    } else {
      pending.resolve(message.result);
    }
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
  const reply = await connection.send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
  if (reply.exceptionDetails) {
    const description =
      reply.exceptionDetails.exception?.description ??
      reply.exceptionDetails.text ??
      "Runtime.evaluateに失敗しました";
    throw new Error(description);
  }
  return reply.result.value;
}

async function capturePath(connection, operations) {
  return evaluate(
    connection,
    `(async () => {
      const api = window.__origami3Capture;
      if (!api) throw new Error("__origami3Captureがありません");
      if (typeof api.runAnglePath !== "function" || typeof api.captureCanonical3D !== "function") {
        throw new Error("この同梱版にはcanonical CDP検査口がありません");
      }
      const documentInfo = await api.openDocument(${JSON.stringify(fixturePath)});
      await api.runAnglePath(${JSON.stringify(operations)});
      await api.setView("3d");
      await api.waitForStable();
      return { documentInfo, capture: api.captureCanonical3D() };
    })()`,
  );
}

function rgbaDifference(leftBase64, rightBase64) {
  const left = Buffer.from(leftBase64, "base64");
  const right = Buffer.from(rightBase64, "base64");
  if (left.length !== right.length) {
    return { pixels: Number.POSITIVE_INFINITY, maxChannel: 255, lengths: [left.length, right.length] };
  }
  let pixels = 0;
  let maxChannel = 0;
  for (let offset = 0; offset < left.length; offset += 4) {
    let differs = false;
    for (let channel = 0; channel < 4; channel++) {
      const delta = Math.abs(left[offset + channel] - right[offset + channel]);
      if (delta !== 0) differs = true;
      maxChannel = Math.max(maxChannel, delta);
    }
    if (differs) pixels++;
  }
  return { pixels, maxChannel, lengths: [left.length, right.length] };
}

function maximumActualDifference(left, right) {
  const a = new Map(left);
  const b = new Map(right);
  const hinges = new Set([...a.keys(), ...b.keys()]);
  let maximum = 0;
  for (const hinge of hinges) {
    if (!a.has(hinge) || !b.has(hinge)) return Number.POSITIVE_INFINITY;
    maximum = Math.max(maximum, Math.abs(a.get(hinge) - b.get(hinge)));
  }
  return maximum;
}

function maximumVertexDifference(left, right) {
  if (left.length !== right.length) return Number.POSITIVE_INFINITY;
  let maximum = 0;
  for (let faceIndex = 0; faceIndex < left.length; faceIndex++) {
    const a = left[faceIndex];
    const b = right[faceIndex];
    if (a.face !== b.face || a.polygon.length !== b.polygon.length) {
      return Number.POSITIVE_INFINITY;
    }
    for (let vertex = 0; vertex < a.polygon.length; vertex++) {
      const dx = a.polygon[vertex][0] - b.polygon[vertex][0];
      const dy = a.polygon[vertex][1] - b.polygon[vertex][1];
      const dz = a.polygon[vertex][2] - b.polygon[vertex][2];
      maximum = Math.max(maximum, Math.hypot(dx, dy, dz));
    }
  }
  return maximum;
}

function validateReadback(capture) {
  const { width, height, owner, depth, final } = capture.readback;
  assert.equal(capture.faces.length, 14, "sa fixtureの面数が14ではありません");
  assert.ok(capture.actual.length > 0, "actual角度が空です");
  assert.ok(owner.codeToFace.length > 0, "owner code→face表が空です");
  assert.ok(width > 0 && height > 0, `描画寸法が空です: ${width}x${height}`);
  const byteLength = width * height * 4;
  const ownerBytes = Buffer.from(owner.data, "base64");
  const depthBytes = Buffer.from(depth.data, "base64");
  const finalBytes = Buffer.from(final.data, "base64");
  assert.equal(ownerBytes.length, byteLength, "owner readbackの長さが違います");
  assert.equal(depthBytes.length, byteLength, "depth readbackの長さが違います");
  assert.equal(finalBytes.length, byteLength, "final readbackの長さが違います");
  assert.equal(
    depth.encoding,
    "rgba8-packed-depth-base64",
    "depth readbackの符号化方式が違います",
  );
  const ownerCodes = owner.codeToFace.map(([code]) => code);
  assert.ok(
    ownerCodes.every((code) => Number.isSafeInteger(code) && code > 0),
    "owner codeに0以下または整数でない値があります",
  );
  assert.equal(
    new Set(ownerCodes).size,
    ownerCodes.length,
    "owner code→face表のcodeが重複しています",
  );
  let paperPixels = 0;
  let backgroundPixels = 0;
  let backgroundDepthMismatches = 0;
  let paperDepthMismatches = 0;
  const knownOwnerCodes = new Set(ownerCodes);
  const unknownOwnerCodes = new Set();
  const finalTokens = new Set();
  const backgroundFinalCounts = new Map();
  for (let offset = 0; offset < byteLength; offset += 4) {
    const ownerCode = ownerBytes.readUInt32LE(offset);
    const packedDepthIsFar =
      depthBytes[offset] === 0xff &&
      depthBytes[offset + 1] === 0xff &&
      depthBytes[offset + 2] === 0xff &&
      depthBytes[offset + 3] === 0xff;
    const finalToken = finalBytes.readUInt32LE(offset);
    finalTokens.add(finalToken);
    if (ownerCode !== 0) {
      paperPixels++;
      if (!knownOwnerCodes.has(ownerCode)) unknownOwnerCodes.add(ownerCode);
      if (packedDepthIsFar) paperDepthMismatches++;
    } else {
      backgroundPixels++;
      if (!packedDepthIsFar) backgroundDepthMismatches++;
      backgroundFinalCounts.set(
        finalToken,
        (backgroundFinalCounts.get(finalToken) ?? 0) + 1,
      );
    }
  }
  assert.ok(paperPixels > 0, "owner readbackに紙の画素がありません");
  assert.ok(backgroundPixels > 0, "owner readbackに背景の画素がありません");
  assert.deepEqual(
    [...unknownOwnerCodes],
    [],
    "owner readbackにcode→face表へ無いtokenがあります",
  );
  assert.equal(
    backgroundDepthMismatches,
    0,
    "owner背景画素のpacked depthがFF FF FF FFではありません",
  );
  assert.equal(
    paperDepthMismatches,
    0,
    "owner紙画素のpacked depthが背景値FF FF FF FFです",
  );
  assert.ok(finalTokens.size > 1, "final RGBAが単一色です");
  const dominantBackgroundToken = [...backgroundFinalCounts.entries()].sort(
    (left, right) => right[1] - left[1],
  )[0][0];
  let paperPixelsDifferentFromBackground = 0;
  for (let offset = 0; offset < byteLength; offset += 4) {
    if (
      ownerBytes.readUInt32LE(offset) !== 0 &&
      finalBytes.readUInt32LE(offset) !== dominantBackgroundToken
    ) {
      paperPixelsDifferentFromBackground++;
    }
  }
  assert.ok(
    paperPixelsDifferentFromBackground > 0,
    "owner紙画素のfinal RGBAがすべて背景色です",
  );
}

function splitClaims(capture) {
  return {
    actual: capture.actual,
    frame: capture.faces.map(({ face, polygon, layer }) => ({ face, polygon, layer })),
    rank: capture.faces.map(({ face, surfaceRank }) => [face, surfaceRank]),
    mirrored: capture.faces.map(({ face, mirrored }) => [face, mirrored]),
    owner: {
      width: capture.readback.width,
      height: capture.readback.height,
      rowOrder: capture.readback.rowOrder,
      codeToFace: capture.readback.owner.codeToFace,
      data: capture.readback.owner.data,
    },
    depth: {
      width: capture.readback.width,
      height: capture.readback.height,
      rowOrder: capture.readback.rowOrder,
      data: capture.readback.depth.data,
    },
    finalRgba: {
      width: capture.readback.width,
      height: capture.readback.height,
      rowOrder: capture.readback.rowOrder,
      data: capture.readback.final.data,
    },
  };
}

async function main() {
  const targets = await fetch(`http://127.0.0.1:${cdpPort}/json`).then((response) => {
    if (!response.ok) throw new Error(`CDP target一覧を読めません: HTTP ${response.status}`);
    return response.json();
  });
  const target = targets.find(
    (candidate) => candidate.type === "page" && candidate.url === "http://tauri.localhost/",
  );
  if (!target?.webSocketDebuggerUrl) {
    throw new Error(`ORIGAMI3のWebView2 targetが9222番にありません`);
  }

  const connection = await CdpConnection.connect(target.webSocketDebuggerUrl);
  try {
    const firstP = await capturePath(connection, paths.P);
    const secondP = await capturePath(connection, paths.P);
    const resultQ = await capturePath(connection, paths.Q);
    for (const result of [firstP, secondP, resultQ]) {
      assert.equal(result.documentInfo.stepCount, 0, "sa fixtureに保存手順があります");
    }
    const captureP = firstP.capture;
    const controlP = secondP.capture;
    const captureQ = resultQ.capture;
    assert.deepEqual(captureP.desired, expectedDesired, "経路Pの最終希望角が違います");
    assert.deepEqual(controlP.desired, expectedDesired, "経路P controlの最終希望角が違います");
    assert.deepEqual(captureQ.desired, expectedDesired, "経路Qの最終希望角が違います");
    validateReadback(captureP);
    validateReadback(controlP);
    validateReadback(captureQ);

    const claimsP = splitClaims(captureP);
    const controlClaimsP = splitClaims(controlP);
    const claimsQ = splitClaims(captureQ);
    assert.deepEqual(
      controlClaimsP,
      claimsP,
      "同じ経路Pを開き直したcontrolがcanonical一致しません",
    );
    const equality = Object.fromEntries(
      Object.keys(claimsP).map((name) => [name, isDeepStrictEqual(claimsP[name], claimsQ[name])]),
    );
    const rankDifferences = claimsP.rank.filter(
      ([face, rank]) => new Map(claimsQ.rank).get(face) !== rank,
    ).length;
    const mirroredDifferences = claimsP.mirrored.filter(
      ([face, mirrored]) => new Map(claimsQ.mirrored).get(face) !== mirrored,
    ).length;
    const diagnostics = {
      fixturePath,
      dimensions: [captureP.readback.width, captureP.readback.height],
      samePathControl: "P/P exact match",
      equality,
      maximumActualDifference: maximumActualDifference(captureP.actual, captureQ.actual),
      maximumVertexDifference: maximumVertexDifference(captureP.faces, captureQ.faces),
      rankDifferences,
      mirroredDifferences,
      ownerDifference: rgbaDifference(
        captureP.readback.owner.data,
        captureQ.readback.owner.data,
      ),
      depthDifference: rgbaDifference(
        captureP.readback.depth.data,
        captureQ.readback.depth.data,
      ),
      finalRgbaDifference: rgbaDifference(
        captureP.readback.final.data,
        captureQ.readback.final.data,
      ),
    };
    process.stdout.write(`${JSON.stringify(diagnostics, null, 2)}\n`);

    const failures = Object.entries(equality)
      .filter(([, equal]) => !equal)
      .map(([name]) => name);
    assert.deepEqual(
      failures,
      [],
      `同じ希望角のcanonical結果が一致しません: ${failures.join(", ")}`,
    );
  } finally {
    connection.close();
  }
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.exitCode = 1;
});
