// M2.T2-8.C02 recovery acceptance through the real Tauri command path.
//
// This file never starts or terminates desktop.exe.  It has two explicit phases:
//
// 1. Prepare an empty, isolated app-data directory before the dedicated app starts:
//
//    $env:ORI3_TEST_APP_DATA_DIR = "$env:TEMP\\ori3-doclink-recovery"
//    node apps/desktop/tests-live/doc-link-b1-recovery-cdp.mjs prepare
//
// 2. Start a dedicated desktop.exe with the same ORI3_TEST_APP_DATA_DIR, then verify:
//
//    $env:ORI3_B1_CDP_RUN = "1"
//    $env:ORI3_DESKTOP_PID = "<PID>"
//    $env:ORI3_DESKTOP_EXE = "<absolute desktop.exe path>"
//    $env:ORI3_DESKTOP_SHA256 = "<SHA-256>"
//    node apps/desktop/tests-live/doc-link-b1-recovery-cdp.mjs verify
//
// The preparation phase refuses a nonempty directory and never deletes anything.
// The verification phase only chooses "復元する"; it never sends "破棄する" and
// never sends "あとで確認する".

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
// 復元後に利用者の画面へ出る作品なので、折り鶴の正本
// `crates/ori3-layers/tests/fixtures/traditional-crane/traditional-crane-cp.ori3`
// （利用者から受け取った traditional_crane_math_bundle）から作った3手の作品を使う。
// 以前は `crates/ori3-rigid/tests/fixtures/check-crane.ori3`（提案探索が返した6手の出力、
// 頂点33・辺61）を写しており、復元すると鶴ではなく凧形の展開図が表示されていた。
// 頂点56・辺114は正本の展開図と一致する。
const fixture = {
  path: path.resolve(repositoryRoot, "apps/desktop/tests-live/fixtures/traditional-crane-full.ori3"),
  sha256: "D2C6DC4A691824C42CC983118B22A9397B2641164DF1A0C7FECD40F2D41C214D",
  vertices: 56,
  edges: 114,
  steps: 3,
};
const autosaveName = "無題.ori3.autosave";
const markerName = "autosave-location.txt";
// 起動時の `prepare_session` は旧形式の1件を複数候補の索引へ移し、payload を
// `<app-data>/autosave-recovery/<番号>.ori3` へ置く。名前は
// `apps/desktop/src-tauri/src/autosave.rs:44` の `CANDIDATES_DIR` と
// 同 `:336` の `candidate_path` に一致する。
const candidatesDirName = "autosave-recovery";

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

/** 持ち越し候補の payload を絶対パスで並べる(名前順)。 */
function candidatePayloads(directory) {
  const candidates = path.join(directory, candidatesDirName);
  if (!existsSync(candidates)) return [];
  return readdirSync(candidates)
    .filter((name) => name.endsWith(".ori3"))
    .map((name) => path.join(candidates, name))
    .sort();
}

function verifyFixture() {
  assert.ok(statSync(fixture.path).isFile(), `recovery fixture is missing: ${fixture.path}`);
  assert.equal(sha256(fixture.path), fixture.sha256, "recovery fixture SHA-256 does not match");
}

function prepare() {
  // The offline preflight must be usable before the coordinator allocates an
  // app-data directory.  It verifies the pinned source fixture but does not
  // create an autosave until ORI3_TEST_APP_DATA_DIR is explicitly supplied.
  if (!isolationValue) {
    verifyFixture();
    process.stdout.write("M2.T2-8.C02 PREPARE READY\n");
    process.stdout.write(
      `${JSON.stringify({
        prepared: false,
        cdpConnected: false,
        desktopStarted: false,
        fixture: { path: fixture.path, sha256: fixture.sha256 },
        requiredForMaterialize: "ORI3_TEST_APP_DATA_DIR below the system temp directory",
        next: "Set ORI3_TEST_APP_DATA_DIR, run prepare again, then start a dedicated desktop.exe with that same value and run verify.",
      }, null, 2)}\n`,
    );
    return;
  }
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
  process.stdout.write("M2.T2-8.C02 PREPARE READY\n");
  process.stdout.write(
    `${JSON.stringify({
      prepared: true,
      isolationDirectory: directory,
      autosave: paths.autosave,
      marker: paths.marker,
      fixture: { path: fixture.path, sha256: fixture.sha256 },
      next: "Start a dedicated desktop.exe with this same ORI3_TEST_APP_DATA_DIR, then run verify.",
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

function assertViewer3dInteractionCapture(value, label) {
  assert.equal(typeof value?.grab?.active, "boolean", `${label}.grab.active must be boolean`);
  assert.ok(
    value.grab.spatial === true || value.grab.spatial === false || value.grab.spatial === null,
    `${label}.grab.spatial must be boolean or null`,
  );
  assert.ok(value.grab.face === null || Number.isInteger(value.grab.face), `${label}.grab.face must be an integer or null`);
  assert.ok(
    value.grab.mode === "flap" || value.grab.mode === "all" || value.grab.mode === "single" || value.grab.mode === null,
    `${label}.grab.mode must be flap, all, single, or null`,
  );
  assert.ok(
    Number.isInteger(value.grab.selectedLayerCount) && value.grab.selectedLayerCount >= 0,
    `${label}.grab.selectedLayerCount must be a nonnegative integer`,
  );
  assert.equal(typeof value?.preview?.visible, "boolean", `${label}.preview.visible must be boolean`);
  assert.ok(
    Number.isInteger(value.preview.polygonCount) && value.preview.polygonCount >= 0,
    `${label}.preview.polygonCount must be a nonnegative integer`,
  );
  assert.ok(
    Number.isInteger(value.preview.segmentCount) && value.preview.segmentCount >= 0,
    `${label}.preview.segmentCount must be a nonnegative integer`,
  );
}

async function verify() {
  const directory = isolatedDirectory();
  const paths = recoveryPaths(directory);
  verifyFixture();
  assert.ok(existsSync(paths.autosave) && existsSync(paths.marker), "run --prepare before starting desktop.exe");
  verifyExecutionContract();
  // 復元を押す前の持ち越し候補。押した後に同じ payload が残ることを確かめるため、
  // 画面を触る前にここで読む。
  const candidatesBeforeRestore = candidatePayloads(directory);
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
        // 候補ごとの選択(復元する/破棄する)は一覧の各項目の中にあり、
        // 「あとで確認する」は候補の外にある画面全体の先送りである。
        const candidateLabels = [...dialog.querySelectorAll('ul[aria-label="前回の作業"] > li')].map(
          (item) => [...item.querySelectorAll("button")].map((button) => compact(button.textContent)),
        );
        const restore = buttons.find((button) => compact(button.textContent) === "復元する");
        if (!restore) throw new Error("recovery dialog has no Restore button");
        const before = api.getInteractionState();
        if (!before.diagnosis.recoveryVisible) throw new Error("capture state does not report recovery visibility");
        // setView waits for the deferred 3D reader.  The following snapshot reads
        // all eight Viewer3D interaction fields.  selectedLayerCount is checked
        // only as a target-face count; this test never treats it as a pleat count.
        await api.setView("normal");
        const viewer3dBeforeRecovery = api.getInteractionState().viewer3d;
        restore.click();
        for (let attempt = 0; attempt < 100; attempt += 1) {
          if (!document.querySelector('[data-floating-ui="recovery-dialog"]')) break;
          await new Promise((resolve) => window.setTimeout(resolve, 25));
        }
        await api.waitForStable();
        if (document.querySelector('[data-floating-ui="recovery-dialog"]')) throw new Error("recovery dialog did not close after Restore");
        return {
          labels,
          candidateLabels,
          before,
          viewer3dBeforeRecovery,
          after: api.getInteractionState(),
          info: api.getDocumentInfo(),
        };
      }})()`,
    );
    // 「あとで確認する」は製品が意図して置いた3つ目のbuttonで、画面の検査
    // `apps/desktop/src/components/RecoveryDialog.dom.test.tsx:88` とヘルプ
    // `apps/desktop/src/help/chapters/saveExport.ts:44,50,55` が実名で固定している
    // (緩和ではなく、意図した変更に対する照合値の更新)。
    assert.deepEqual(
      result.labels,
      ["復元する", "破棄する", "あとで確認する"],
      "recovery choices changed",
    );
    // `docs/traceability/b1-cdp-automation-plan.md:28` の「選択肢はちょうど2」は
    // 候補ごとの選択を指す。`apps/desktop/src/components/RecoveryDialog.tsx:69-91` の
    // 候補内 `.button-row` がその2つで、画面全体の先送りは別に数える。
    assert.deepEqual(
      result.candidateLabels,
      [["復元する", "破棄する"]],
      "per-candidate recovery choices changed",
    );
    assertViewer3dInteractionCapture(result.viewer3dBeforeRecovery, "before recovery");
    assertViewer3dInteractionCapture(result.after.viewer3d, "after recovery");
    assert.equal(result.after.diagnosis.recoveryVisible, false, "recovery remains visible after Restore");
    assert.equal(result.after.document.vertexCount, fixture.vertices, "restored vertex count differs from fixture");
    assert.equal(result.after.document.edgeCount, fixture.edges, "restored edge count differs from fixture");
    assert.equal(result.info.stepCount, fixture.steps, "restored step count differs from fixture");
    // 復元しただけでは控えを消さない、が製品の契約である。
    // `apps/desktop/src-tauri/src/autosave.rs:1762`
    // `restored_candidate_is_deleted_only_after_a_successful_save` が、復元直後は
    // `assert!(candidate.is_file(), "復元だけで候補を消してはいけない")`、明示保存の成功後に
    // `discard_after_save` で初めて消えることを固定する。画面が呼ぶ `recovery_restore`
    // (`apps/desktop/src-tauri/src/commands.rs:506`) は同 `:1214` の `restore_candidate` へ入り、
    // そこで消すのは現行作業枠 `autosave-current.ori3` (同 `:1276`) だけである。
    // 旧形式の payload と目印は同 `:2055`
    // `legacy_single_candidate_is_migrated_without_deleting_its_source` が移行後も残すことを
    // 固定している。以前ここは復元直後に両方が消えていることを求めており、契約と逆だった。
    // 保存を経て消える側は上記Rust検査が受け持つ。capture API に保存の入口が無く、
    // この script は実機の保存ダイアログを開かないため、ここでは検査しない。
    assert.equal(candidatesBeforeRestore.length, 1, "prepare should leave exactly one carried candidate");
    for (const payload of candidatesBeforeRestore) {
      assert.ok(existsSync(payload), `carried candidate disappeared after Restore alone: ${payload}`);
    }
    assert.ok(existsSync(paths.autosave), "legacy autosave payload disappeared after Restore alone");
    assert.ok(existsSync(paths.marker), "legacy autosave marker disappeared after Restore alone");
    process.stdout.write("M2.T2-8.C02 VERIFY PASSED\n");
    process.stdout.write(
      `${JSON.stringify(
        {
          passed: true,
          id: "M2.T2-8.C02",
          keptAfterRestore: {
            candidates: candidatesBeforeRestore,
            legacyAutosave: paths.autosave,
            legacyMarker: paths.marker,
          },
          result,
        },
        null,
        2,
      )}\n`,
    );
  } finally {
    connection.close();
  }
}

async function main() {
  if (phase === "prepare" || phase === "--prepare") prepare();
  else if (phase === "verify" || phase === "--verify") await verify();
  else if (phase === "describe" || phase === "--describe") {
    process.stdout.write("M2.T2-8.C02 PREPARE/VERIFY NOT EXECUTED\n");
    process.stdout.write(`${JSON.stringify({ id: "M2.T2-8.C02", phases: ["prepare", "verify"], executed: false }, null, 2)}\n`);
  } else {
    throw new Error(`unknown phase: ${phase}`);
  }
}

try {
  await main();
} catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  process.stderr.write(`M2.T2-8.C02 ${phase} FAILED: ${message}\n`);
  process.exitCode = 1;
}
