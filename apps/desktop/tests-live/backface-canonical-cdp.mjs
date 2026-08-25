// 実WebView2・本番WebGLを使う恒久CDP受入検査。
// package.jsonの専用scriptから明示実行し、ブラウザやdesktop.exeは起動せず、
// 統括が起動した既存WebView2の9222番（ORI3_CDP_PORTで変更可）へ接続する。

import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
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
    socket.addEventListener("close", (event) => {
      for (const { reject } of this.pending.values()) {
        reject(
          new Error(
            `CDP接続が応答前に閉じました: code=${event.code}, reason=${event.reason || "(none)"}`,
          ),
        );
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
    if (message.method === "Runtime.consoleAPICalled") {
      const values = message.params.args.map(
        (argument) => argument.value ?? argument.description ?? argument.type,
      );
      process.stderr.write(`[WebView ${message.params.type}] ${values.join(" ")}\n`);
      return;
    }
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

async function capturePath(connection, slot, operations) {
  return evaluate(
    connection,
    `(async () => {
      const api = window.__origami3Capture;
      if (!api) throw new Error("__origami3Captureがありません");
      if (typeof api.runAnglePath !== "function" || typeof api.captureCanonical3D !== "function") {
        throw new Error("この同梱版にはcanonical CDP検査口がありません");
      }
      console.log("[canonical-cdp] ${slot}: open");
      const documentInfo = await api.openDocument(${JSON.stringify(fixturePath)});
      console.log("[canonical-cdp] ${slot}: angle path");
      await api.runAnglePath(${JSON.stringify(operations)});
      await api.setView("3d");
      await api.waitForStable();
      console.log("[canonical-cdp] ${slot}: GPU readback");
      const capture = api.captureCanonical3D();
      console.log(
        "[canonical-cdp] ${slot}: captured",
        capture.readback.width,
        capture.readback.height,
      );
      const captures = globalThis.__ori3CanonicalPendingCaptures ??= {};
      captures[${JSON.stringify(slot)}] = { documentInfo, capture };
      return {
        slot: ${JSON.stringify(slot)},
        stepCount: documentInfo.stepCount,
        desired: capture.desired,
        actualCount: capture.actual.length,
        faceCount: capture.faces.length,
        dimensions: [capture.readback.width, capture.readback.height],
      };
    })()`,
  );
}

/**
 * 全画素3枚×P/P/QをCDP応答へ載せず、同じWebView内で完全比較する。
 * CDPへ返すのはpreflight件数と差分件数だけなので、検査器の直列化でrendererを失わない。
 */
function compareStoredCapturesInWebView() {
  const stored = globalThis.__ori3CanonicalPendingCaptures;
  if (!stored?.P1?.capture || !stored?.P2?.capture || !stored?.Q?.capture) {
    throw new Error("P/P/QのreadbackがWebView内に揃っていません");
  }

  const readU32Le = (bytes, offset) =>
    (bytes.charCodeAt(offset) |
      (bytes.charCodeAt(offset + 1) << 8) |
      (bytes.charCodeAt(offset + 2) << 16) |
      (bytes.charCodeAt(offset + 3) << 24)) >>>
    0;
  const isFarDepth = (bytes, offset) =>
    bytes.charCodeAt(offset) === 0xff &&
    bytes.charCodeAt(offset + 1) === 0xff &&
    bytes.charCodeAt(offset + 2) === 0xff &&
    bytes.charCodeAt(offset + 3) === 0xff;
  const same = (left, right) => JSON.stringify(left) === JSON.stringify(right);
  const rgbaDifference = (leftBase64, rightBase64) => {
    const left = atob(leftBase64);
    const right = atob(rightBase64);
    if (left.length !== right.length) {
      return { pixels: null, maxChannel: 255, lengths: [left.length, right.length] };
    }
    let pixels = 0;
    let maxChannel = 0;
    for (let offset = 0; offset < left.length; offset += 4) {
      let differs = false;
      for (let channel = 0; channel < 4; channel++) {
        const delta = Math.abs(
          left.charCodeAt(offset + channel) - right.charCodeAt(offset + channel),
        );
        if (delta !== 0) differs = true;
        maxChannel = Math.max(maxChannel, delta);
      }
      if (differs) pixels++;
    }
    return { pixels, maxChannel, lengths: [left.length, right.length] };
  };
  const maximumActualDifference = (left, right) => {
    const a = new Map(left);
    const b = new Map(right);
    const hinges = new Set([...a.keys(), ...b.keys()]);
    let maximum = 0;
    for (const hinge of hinges) {
      if (!a.has(hinge) || !b.has(hinge)) return null;
      maximum = Math.max(maximum, Math.abs(a.get(hinge) - b.get(hinge)));
    }
    return maximum;
  };
  const maximumVertexDifference = (left, right) => {
    if (left.length !== right.length) return null;
    let maximum = 0;
    for (let faceIndex = 0; faceIndex < left.length; faceIndex++) {
      const a = left[faceIndex];
      const b = right[faceIndex];
      if (a.face !== b.face || a.polygon.length !== b.polygon.length) return null;
      for (let vertex = 0; vertex < a.polygon.length; vertex++) {
        maximum = Math.max(
          maximum,
          Math.hypot(
            a.polygon[vertex][0] - b.polygon[vertex][0],
            a.polygon[vertex][1] - b.polygon[vertex][1],
            a.polygon[vertex][2] - b.polygon[vertex][2],
          ),
        );
      }
    }
    return maximum;
  };
  const claims = (capture) => ({
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
  });
  const summarize = ({ documentInfo, capture }) => {
    const { width, height, rowOrder, owner, depth, final } = capture.readback;
    const ownerBytes = atob(owner.data);
    const depthBytes = atob(depth.data);
    const finalBytes = atob(final.data);
    const expectedByteLength = width * height * 4;
    const ownerCodes = owner.codeToFace.map(([code]) => code);
    const knownOwnerCodes = new Set(ownerCodes);
    const unknownOwnerCodes = new Set();
    const finalTokens = new Set();
    const backgroundFinalCounts = new Map();
    let paperPixels = 0;
    let backgroundPixels = 0;
    let backgroundDepthMismatches = 0;
    let paperDepthMismatches = 0;
    for (let offset = 0; offset < ownerBytes.length; offset += 4) {
      const ownerCode = readU32Le(ownerBytes, offset);
      const far = isFarDepth(depthBytes, offset);
      const finalToken = readU32Le(finalBytes, offset);
      finalTokens.add(finalToken);
      if (ownerCode === 0) {
        backgroundPixels++;
        if (!far) backgroundDepthMismatches++;
        backgroundFinalCounts.set(
          finalToken,
          (backgroundFinalCounts.get(finalToken) ?? 0) + 1,
        );
      } else {
        paperPixels++;
        if (!knownOwnerCodes.has(ownerCode)) unknownOwnerCodes.add(ownerCode);
        if (far) paperDepthMismatches++;
      }
    }
    const dominantBackgroundToken = [...backgroundFinalCounts.entries()].sort(
      (left, right) => right[1] - left[1],
    )[0]?.[0];
    let paperPixelsDifferentFromBackground = 0;
    for (let offset = 0; offset < ownerBytes.length; offset += 4) {
      if (
        readU32Le(ownerBytes, offset) !== 0 &&
        readU32Le(finalBytes, offset) !== dominantBackgroundToken
      ) {
        paperPixelsDifferentFromBackground++;
      }
    }
    return {
      stepCount: documentInfo.stepCount,
      desired: capture.desired,
      actualCount: capture.actual.length,
      faceCount: capture.faces.length,
      version: capture.readback.version,
      dimensions: [width, height],
      rowOrder,
      encodings: [owner.encoding, depth.encoding, final.encoding],
      expectedByteLength,
      byteLengths: [ownerBytes.length, depthBytes.length, finalBytes.length],
      ownerCodes,
      ownerCodeCount: ownerCodes.length,
      uniqueOwnerCodeCount: new Set(ownerCodes).size,
      invalidOwnerCodes: ownerCodes.filter(
        (code) => !Number.isSafeInteger(code) || code <= 0,
      ),
      unknownOwnerCodes: [...unknownOwnerCodes],
      paperPixels,
      backgroundPixels,
      backgroundDepthMismatches,
      paperDepthMismatches,
      finalTokenCount: finalTokens.size,
      paperPixelsDifferentFromBackground,
    };
  };

  const captureP = stored.P1.capture;
  const controlP = stored.P2.capture;
  const captureQ = stored.Q.capture;
  const claimsP = claims(captureP);
  const controlClaimsP = claims(controlP);
  const claimsQ = claims(captureQ);
  const equalityFor = (left, right) =>
    Object.fromEntries(Object.keys(left).map((name) => [name, same(left[name], right[name])]));
  const equality = equalityFor(claimsP, claimsQ);
  const controlEquality = equalityFor(claimsP, controlClaimsP);
  const qRanks = new Map(claimsQ.rank);
  const qMirrored = new Map(claimsQ.mirrored);
  return {
    summaries: [summarize(stored.P1), summarize(stored.P2), summarize(stored.Q)],
    controlEquality,
    equality,
    maximumActualDifference: maximumActualDifference(captureP.actual, captureQ.actual),
    maximumVertexDifference: maximumVertexDifference(captureP.faces, captureQ.faces),
    rankDifferences: claimsP.rank.filter(([face, rank]) => qRanks.get(face) !== rank).length,
    mirroredDifferences: claimsP.mirrored.filter(
      ([face, mirrored]) => qMirrored.get(face) !== mirrored,
    ).length,
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
    await connection.send("Runtime.enable");
    const milestones = [
      await capturePath(connection, "P1", paths.P),
      await capturePath(connection, "P2", paths.P),
      await capturePath(connection, "Q", paths.Q),
    ];
    for (const milestone of milestones) {
      assert.equal(milestone.stepCount, 0, "sa fixtureに保存手順があります");
      assert.deepEqual(
        milestone.desired,
        expectedDesired,
        `${milestone.slot}の最終希望角が違います`,
      );
      assert.equal(milestone.faceCount, 14, `${milestone.slot}の面数が14ではありません`);
      assert.ok(milestone.actualCount > 0, `${milestone.slot}のactual角度が空です`);
      assert.ok(
        milestone.dimensions[0] > 0 && milestone.dimensions[1] > 0,
        `${milestone.slot}の描画寸法が空です`,
      );
    }

    const compact = await evaluate(
      connection,
      `(${compareStoredCapturesInWebView.toString()})()`,
    );
    for (const summary of compact.summaries) {
      assert.equal(summary.stepCount, 0, "sa fixtureに保存手順があります");
      assert.deepEqual(summary.desired, expectedDesired, "最終希望角が違います");
      assert.equal(summary.faceCount, 14, "sa fixtureの面数が14ではありません");
      assert.ok(summary.actualCount > 0, "actual角度が空です");
      assert.equal(summary.version, 1, "readback versionが違います");
      assert.ok(
        summary.dimensions[0] > 0 && summary.dimensions[1] > 0,
        `描画寸法が空です: ${summary.dimensions.join("x")}`,
      );
      assert.equal(summary.rowOrder, "bottom-to-top", "readbackの行順が違います");
      assert.deepEqual(
        summary.encodings,
        ["rgba8-base64", "rgba8-packed-depth-base64", "rgba8-base64"],
        "readbackの符号化方式が違います",
      );
      assert.deepEqual(
        summary.byteLengths,
        [summary.expectedByteLength, summary.expectedByteLength, summary.expectedByteLength],
        "owner/depth/final readbackの長さが違います",
      );
      assert.ok(summary.ownerCodeCount > 0, "owner code→face表が空です");
      assert.equal(
        summary.uniqueOwnerCodeCount,
        summary.ownerCodeCount,
        "owner code→face表のcodeが重複しています",
      );
      assert.deepEqual(summary.invalidOwnerCodes, [], "owner codeに不正値があります");
      assert.deepEqual(
        summary.unknownOwnerCodes,
        [],
        "owner readbackにcode→face表へ無いtokenがあります",
      );
      assert.ok(summary.paperPixels > 0, "owner readbackに紙の画素がありません");
      assert.ok(summary.backgroundPixels > 0, "owner readbackに背景の画素がありません");
      assert.equal(
        summary.backgroundDepthMismatches,
        0,
        "owner背景画素のpacked depthがFF FF FF FFではありません",
      );
      assert.equal(
        summary.paperDepthMismatches,
        0,
        "owner紙画素のpacked depthが背景値FF FF FF FFです",
      );
      assert.ok(summary.finalTokenCount > 1, "final RGBAが単一色です");
      assert.ok(
        summary.paperPixelsDifferentFromBackground > 0,
        "owner紙画素のfinal RGBAがすべて背景色です",
      );
    }

    const controlFailures = Object.entries(compact.controlEquality)
      .filter(([, equal]) => !equal)
      .map(([name]) => name);
    assert.deepEqual(
      controlFailures,
      [],
      `同じ経路Pを開き直したcontrolがcanonical一致しません: ${controlFailures.join(", ")}`,
    );
    const diagnostics = {
      fixturePath,
      dimensions: compact.summaries[0].dimensions,
      samePathControl: "P/P exact match",
      equality: compact.equality,
      maximumActualDifference: compact.maximumActualDifference,
      maximumVertexDifference: compact.maximumVertexDifference,
      rankDifferences: compact.rankDifferences,
      mirroredDifferences: compact.mirroredDifferences,
      ownerDifference: compact.ownerDifference,
      depthDifference: compact.depthDifference,
      finalRgbaDifference: compact.finalRgbaDifference,
      readbackPreflight: compact.summaries.map((summary) => ({
        dimensions: summary.dimensions,
        paperPixels: summary.paperPixels,
        backgroundPixels: summary.backgroundPixels,
        finalTokenCount: summary.finalTokenCount,
      })),
    };
    process.stdout.write(`${JSON.stringify(diagnostics, null, 2)}\n`);

    const failures = Object.entries(compact.equality)
      .filter(([, equal]) => !equal)
      .map(([name]) => name);
    assert.deepEqual(
      failures,
      [],
      `同じ希望角のcanonical結果が一致しません: ${failures.join(", ")}`,
    );
  } finally {
    try {
      await evaluate(
        connection,
        "delete globalThis.__ori3CanonicalPendingCaptures; true",
      );
    } catch {
      // WebView自体を失ったinfra failureではcleanupできない。元の失敗を優先する。
    }
    connection.close();
  }
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.exitCode = 1;
});
