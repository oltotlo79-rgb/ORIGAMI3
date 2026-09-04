import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  acquireCaptureOwner,
  acquirePromotionLease,
  assertResumeFingerprint,
  captureFixturePaths,
  cleanupClient,
  executePlan,
  inputFingerprintSha256,
  parseArguments as parseRawArguments,
  PlanExecutionError,
  promotionRecoveryDecision,
  prepareOriginalBackups,
  promoteSelection,
  recoverPromotionBeforeResume,
  releaseCaptureOwner,
  releaseCaptureOwnerThenComplete,
  selectEntries,
  validateManifest,
  verifyFinalInputFingerprint,
} from "./capture-manual-screens.mjs";
import {
  awaitManualCaptureSocketOpen,
  discoverManualCaptureTargets,
  initializeManualCaptureConnection,
} from "./manual-capture/cdp-client.mjs";
import { bestEffortCleanup, createScenarioRegistry } from "./manual-capture/scenarios.mjs";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const manifestPath = path.join(scriptDirectory, "manual-screenshot-manifest.json");
const resumeRunPath = path.join(repositoryRoot, "verification", "manual-capture", "run-static-test");
const APP_ARGUMENTS = Object.freeze([
  "--app-exe",
  path.join(repositoryRoot, "target", "release", "desktop.exe"),
  "--app-sha256",
  "a".repeat(64),
  "--owner-token",
  "0".repeat(32),
]);

function parseArguments(argv) {
  return parseRawArguments([...APP_ARGUMENTS, ...argv]);
}

// This list is intentionally independent from the capture manifest.  It is the
// audited order in scratchpad/self-intersection-report.md, rows 1 through 42,
// plus the 4 rows added in scratchpad/manual-screenshot-plan-2026-09-04.md §1-3.
const EXPECTED_ENTRIES = Object.freeze([
  { name: "screen-overview-guide.png", scenario: "overviewGuide" },
  { name: "screen-workspace.png", scenario: "workspace" },
  { name: "screen-theme-japanese.png", scenario: "themeJapanese" },
  { name: "screen-theme-modern.png", scenario: "themeModern" },
  { name: "screen-pane-resize.png", scenario: "paneResize" },
  { name: "screen-pane-reset.png", scenario: "paneReset" },
  { name: "screen-tooltip-hover.png", scenario: "tooltipHover" },
  { name: "screen-compact-operation-help.png", scenario: "compactOperationHelp" },
  { name: "screen-new-dialog.png", scenario: "newDialog" },
  { name: "screen-paper-colors.png", scenario: "paperColors" },
  { name: "screen-color-picker.png", scenario: "colorPicker" },
  { name: "screen-draw-line.png", scenario: "drawLine" },
  { name: "screen-mirror-axis.png", scenario: "mirrorAxis" },
  { name: "screen-fold-drag.png", scenario: "foldDrag" },
  { name: "screen-angle-slider.png", scenario: "angleSlider" },
  { name: "screen-angle-pin.png", scenario: "anglePin" },
  { name: "screen-angle-pin-released.png", scenario: "anglePinReleased" },
  { name: "screen-natural-follow.png", scenario: "naturalFollow" },
  { name: "screen-natural-follow-overflow.png", scenario: "naturalFollowOverflow" },
  { name: "screen-natural-follow-best-effort.png", scenario: "naturalFollowBestEffort" },
  { name: "screen-flat-reset-after-playback-before.png", scenario: "flatResetBefore" },
  { name: "screen-flat-reset-after-playback-after.png", scenario: "flatResetAfter" },
  { name: "screen-3d-angle90.png", scenario: "angle90" },
  { name: "screen-prevention-settings.png", scenario: "preventionSettings" },
  { name: "screen-paper-inflate.png", scenario: "paperInflate" },
  { name: "screen-flat-complete-no-warning.png", scenario: "flatCompleteNoWarning" },
  { name: "screen-measure-angle-result.png", scenario: "measureAngleResult" },
  { name: "screen-measure-distance-result.png", scenario: "measureDistanceResult" },
  { name: "screen-techniques.png", scenario: "techniques" },
  { name: "screen-layer-motion-open-close.png", scenario: "layerMotionOpenClose" },
  { name: "screen-layer-motion-restack.png", scenario: "layerMotionRestack" },
  { name: "screen-timeline.png", scenario: "timeline" },
  { name: "screen-cp-history-step1.png", scenario: "cpHistoryStep1" },
  { name: "screen-cp-history-latest.png", scenario: "cpHistoryLatest" },
  { name: "screen-proposal-wizard.png", scenario: "proposalWizard" },
  { name: "screen-export-dialog.png", scenario: "exportDialog" },
  { name: "screen-warning.png", scenario: "warning" },
  { name: "screen-help-center.png", scenario: "helpCenter" },
  { name: "screen-foldall-slider.png", scenario: "foldAllSlider" },
  { name: "screen-proposal-progress.png", scenario: "proposalProgress" },
  { name: "screen-viewer3d-loading.png", scenario: "viewer3dLoading" },
  { name: "screen-viewer3d-load-error.png", scenario: "viewer3dLoadError" },
  { name: "screen-fold-pleat-target.png", scenario: "foldPleatTarget" },
  { name: "screen-self-intersection-pairs.png", scenario: "selfIntersectionPairs" },
  { name: "screen-recovery-choices.png", scenario: "recoveryChoices" },
  { name: "screen-export-fold-file.png", scenario: "exportFoldFile" },
]);
const EXPECTED_NAMES = Object.freeze(EXPECTED_ENTRIES.map((entry) => entry.name));

let assertions = 0;

function check(condition, message) {
  assertions += 1;
  assert.ok(condition, message);
}

function equal(actual, expected, message) {
  assertions += 1;
  assert.deepEqual(actual, expected, message);
}

function throws(action, message) {
  assertions += 1;
  let thrown;
  try {
    action();
  } catch (error) {
    thrown = error;
  }
  assert.ok(thrown instanceof Error, message);
}

async function rejects(action, message) {
  assertions += 1;
  let thrown;
  try {
    await action();
  } catch (error) {
    thrown = error;
  }
  assert.ok(thrown instanceof Error, message);
}

function makeResumeState(entries, passedThrough) {
  return {
    schemaVersion: 1,
    runId: "static-test",
    manifestHash: "static-manifest-hash",
    runnerHash: "static-runner-hash",
    startedAt: "2026-09-01T00:00:00.000Z",
    updatedAt: "2026-09-01T00:00:00.000Z",
    outputDir: "C:/static-test/assets",
    stagingDir: "C:/static-test/staging",
    selection: entries.map((entry) => entry.name),
    entries: Object.fromEntries(
      entries.map((entry) => [
        entry.name,
        {
          status: entry.ordinal <= passedThrough ? "passed" : "pending",
          ordinal: entry.ordinal,
          scenario: entry.scenario,
        },
      ]),
    ),
  };
}

function cloneJson(value) {
  return JSON.parse(JSON.stringify(value));
}

const manifest = JSON.parse(await fs.readFile(manifestPath, "utf8"));
const registry = createScenarioRegistry({ repositoryRoot });
const scenarioSource = await fs.readFile(
  path.join(scriptDirectory, "manual-capture", "scenarios.mjs"),
  "utf8",
);

/**
 * Keep a source contract around the live-CDP postconditions.  The offline test
 * cannot drive WebView2, but it must still fail if a scenario is weakened back
 * to checking only its active tool or generic page text.  Bounds are adjacent
 * function declarations so a matching assertion in another scenario cannot
 * accidentally satisfy this contract.
 */
function scenarioFunctionSource(name, nextName) {
  const startToken = `async function ${name}(`;
  const endToken = `async function ${nextName}(`;
  const start = scenarioSource.indexOf(startToken);
  const end = scenarioSource.indexOf(endToken, start + startToken.length);
  check(start >= 0, `scenario source must declare ${name}`);
  check(end > start, `scenario source contract for ${name} must end before ${nextName}`);
  return scenarioSource.slice(start, end);
}

const paneResizeSource = scenarioFunctionSource("paneResize", "paneReset");
for (const required of [
  "leftRight: Math.abs(after.leftRight.x - before.leftRight.x)",
  "upperLower: Math.abs(after.upperLower.y - before.upperLower.y)",
  "deltas.leftRight < 40 || deltas.upperLower < 40",
]) {
  check(
    paneResizeSource.includes(required),
    `paneResize must measure and reject missing movement for both splitters: ${required}`,
  );
}

const compactHelpSource = scenarioFunctionSource(
  "compactOperationHelp",
  "newDialog",
);
for (const required of [
  "origami3-active-tooltip",
  "aria-describedby",
  "モードの説明とマウス操作の割り当てを開きます",
  "tooltip.textContent?.trim() === expected",
  "style.visibility === 'visible'",
]) {
  check(
    compactHelpSource.includes(required),
    `compactOperationHelp must require its own visible, linked tooltip: ${required}`,
  );
}

const drawLineSource = scenarioFunctionSource("drawLinePreview", "mirrorAxis");
for (const required of [
  ".operation-steps li[aria-current=\"step\"]",
  "終点をクリック",
  "context.getImageData(",
  "mountainRed",
  "previewPixels.mountainRed < 8",
]) {
  check(
    drawLineSource.includes(required),
    `drawLinePreview must require endpoint-stage state and rendered mountain geometry: ${required}`,
  );
}

const foldDragSource = scenarioFunctionSource("foldDrag", "selectAnyCrease");
for (const required of [
  "state.viewer3d?.grab",
  "grab?.active === true",
  "grab.selectedLayerCount > 0",
  "preview?.visible === true",
  "preview.segmentCount > 0",
]) {
  check(
    foldDragSource.includes(required),
    `foldDrag must require a real Viewer3D grab and visible preview: ${required}`,
  );
}

check(Array.isArray(manifest), "manifest must be an array");
equal(manifest.length, 46, "manifest must have exactly 46 entries");
equal(
  manifest.map((entry) => entry.name),
  EXPECTED_NAMES,
  "manifest names and order must match all 46 audited rows one-for-one",
);
equal(
  manifest.map(({ name, scenario }) => ({ name, scenario })),
  EXPECTED_ENTRIES,
  "manifest names and scenario assignments must match all 46 independently audited rows one-for-one",
);
equal(
  manifest.map((entry) => entry.ordinal),
  Array.from({ length: 46 }, (_, index) => index + 1),
  "manifest ordinals must be exactly 1 through 46",
);

check(Array.isArray(registry), "scenario registry must be an array");
equal(registry.length, 46, "scenario registry must have exactly 46 implementations");
const registryIds = registry.map((scenario) => scenario.id);
equal(
  new Set(registryIds).size,
  46,
  "scenario registry ids must be unique",
);
for (const entry of manifest) {
  const scenario = registry.find((candidate) => candidate.id === entry.scenario);
  check(scenario !== undefined, `scenario ${entry.scenario} must implement ${entry.name}`);
  check(typeof scenario.run === "function", `scenario ${entry.scenario} must expose a run function`);
}
for (const scenario of registry) {
  check(
    manifest.some((entry) => entry.scenario === scenario.id),
    `scenario ${scenario.id} must be used by exactly one manifest entry`,
  );
}
equal(
  new Set(manifest.map((entry) => entry.scenario)).size,
  46,
  "each manifest entry must use a distinct scenario",
);
const entries = await validateManifest(manifest, registry);
assertions += 1;
equal(
  entries.map((entry) => entry.name),
  EXPECTED_NAMES,
  "validated capture entries must preserve all 46 manifest rows and their order",
);

const defaultArguments = parseArguments([]);
check(
  !Object.hasOwn(defaultArguments, "outDir"),
  "the capture destination must not be overridable from the command line",
);
equal(
  Object.keys(captureFixturePaths).sort(),
  ["bird", "crane", "penetration", "yakko"],
  "the resume fingerprint must pin exactly the four fixtures used by scenarios",
);
throws(
  () => parseRawArguments([]),
  "direct capture execution without a bundled app path and SHA-256 must fail",
);
throws(
  () => parseRawArguments(["--app-exe", APP_ARGUMENTS[1]]),
  "an application path without its SHA-256 must fail",
);
throws(
  () => parseRawArguments(["--app-sha256", "a".repeat(64)]),
  "an application SHA-256 without its path must fail",
);
throws(
  () =>
    parseRawArguments([
      "--app-exe",
      APP_ARGUMENTS[1],
      "--app-sha256",
      "A".repeat(64),
      "--owner-token",
      "0".repeat(32),
    ]),
  "an uppercase or malformed application SHA-256 must fail closed",
);
throws(
  () => parseRawArguments(["--app-exe", APP_ARGUMENTS[1], "--app-sha256", "a".repeat(64)]),
  "direct Node capture with an app identity but no wrapper owner token must fail before CDP",
);
throws(
  () =>
    parseRawArguments([
      "--app-exe",
      APP_ARGUMENTS[1],
      "--app-sha256",
      "a".repeat(64),
      "--owner-token",
      "A".repeat(32),
    ]),
  "an uppercase or malformed wrapper owner token must fail closed",
);
equal(parseRawArguments(["--list"]).list, true, "--list alone must not require a running application identity");

let rejectedDiscoveryAborts = 0;
await rejects(
  () =>
    discoverManualCaptureTargets({
      endpoint: "http://127.0.0.1:9222",
      fetchImpl: async () => {
        throw new Error("injected discovery rejection");
      },
      createAbortController: () => ({
        signal: {},
        abort: () => {
          rejectedDiscoveryAborts += 1;
        },
      }),
      timeoutMilliseconds: 50,
    }),
  "a rejected discovery fetch must propagate",
);
equal(rejectedDiscoveryAborts, 1, "a rejected discovery fetch must abort its request exactly once");

let bodyDiscoveryStarted = false;
let bodyDiscoveryAborts = 0;
await rejects(
  () =>
    discoverManualCaptureTargets({
      endpoint: "http://127.0.0.1:9222",
      fetchImpl: async () => ({
        ok: true,
        status: 200,
        json: () => {
          bodyDiscoveryStarted = true;
          return new Promise(() => {});
        },
      }),
      createAbortController: () => ({
        signal: {},
        abort: () => {
          bodyDiscoveryAborts += 1;
        },
      }),
      timeoutMilliseconds: 1,
    }),
  "a discovery response whose JSON body hangs must time out",
);
check(bodyDiscoveryStarted, "the body-hang injection must reach response.json()");
equal(bodyDiscoveryAborts, 1, "a timed-out discovery JSON body must abort its request exactly once");

for (const failurePoint of ["Runtime.enable", "Page.enable", "lockMetrics"]) {
  const initializationCalls = [];
  let closeCalls = 0;
  await rejects(
    () =>
      initializeManualCaptureConnection({
        call: async (method) => {
          initializationCalls.push(method);
          if (method === failurePoint) throw new Error(`injected ${failurePoint} failure`);
        },
        lockMetrics: async () => {
          initializationCalls.push("lockMetrics");
          if (failurePoint === "lockMetrics") throw new Error("injected lockMetrics failure");
        },
        close: async () => {
          closeCalls += 1;
        },
      }),
    `${failurePoint} initialization failure must propagate`,
  );
  equal(closeCalls, 1, `${failurePoint} initialization failure must close the already-open CDP socket exactly once`);
  check(
    initializationCalls.includes(failurePoint),
    `${failurePoint} failure injection must reach the intended initialization stage`,
  );
}

let failedOpenCloseCalls = 0;
await rejects(
  () =>
    awaitManualCaptureSocketOpen({
      openPromise: Promise.reject(new Error("injected connection wait failure")),
      close: async () => {
        failedOpenCloseCalls += 1;
      },
      timeoutMilliseconds: 50,
    }),
  "a rejected or timed-out WebSocket open wait must propagate",
);
equal(failedOpenCloseCalls, 1, "a failed WebSocket open wait must close its CONNECTING socket exactly once");
let timedOutOpenCloseCalls = 0;
await rejects(
  () =>
    awaitManualCaptureSocketOpen({
      openPromise: new Promise(() => {}),
      close: async () => {
        timedOutOpenCloseCalls += 1;
      },
      timeoutMilliseconds: 1,
    }),
  "a timed-out WebSocket open wait must propagate",
);
equal(timedOutOpenCloseCalls, 1, "a timed-out WebSocket open wait must close its CONNECTING socket exactly once");

const failedCompletionEvents = [];
await rejects(
  () =>
    releaseCaptureOwnerThenComplete({
      owner: { test: true },
      completion: {
        selectedCount: 42,
        totalCount: 42,
        outDir: "C:/offline/assets",
        runDirectory: "C:/offline/run",
      },
      releaseOwner: async () => {
        failedCompletionEvents.push("release");
        throw new Error("injected owner release failure");
      },
      write: (line) => failedCompletionEvents.push(`write:${line}`),
    }),
  "owner release failure must make final completion fail",
);
equal(
  failedCompletionEvents,
  ["release"],
  "owner release failure must not emit a COMPLETE event or write any success stdout",
);

const successfulCompletionEvents = [];
await releaseCaptureOwnerThenComplete({
  owner: { test: true },
  completion: {
    selectedCount: 42,
    totalCount: 42,
    outDir: "C:/offline/assets",
    runDirectory: "C:/offline/run",
  },
  releaseOwner: async () => successfulCompletionEvents.push("release"),
  write: (line) => successfulCompletionEvents.push(line.includes("COMPLETE 42/42") ? "complete" : "bad"),
});
equal(
  successfulCompletionEvents,
  ["release", "complete"],
  "COMPLETE must be emitted only after owner release succeeds",
);

const ownerTempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "ori3-manual-owner-test-"));
const ownerLockPath = path.join(ownerTempRoot, ".capture-owner-lock");
const ownerTokenA = "1".repeat(32);
const ownerTokenB = "2".repeat(32);
let heldOwner = null;
try {
  heldOwner = await acquireCaptureOwner(ownerTokenA, { lockPath: ownerLockPath });
  await rejects(
    () => acquireCaptureOwner(ownerTokenA, { lockPath: ownerLockPath }),
    "a second live process for the same run/token must be rejected",
  );
  await rejects(
    () => acquireCaptureOwner(ownerTokenB, { lockPath: ownerLockPath }),
    "a different run/token must not enter while a capture owner lock exists",
  );
  await releaseCaptureOwner(heldOwner);
  heldOwner = null;

  // Model a process that died immediately after owner acquisition, before any
  // run.json existed. The wrapper reuses this exact fixed-lock token for its
  // next one-command fresh run, and Node must reclaim it without opening CDP.
  await acquireCaptureOwner(ownerTokenA, {
    lockPath: ownerLockPath,
    pid: 41001,
    processIsAlive: () => false,
  });
  await rejects(
    () =>
      acquireCaptureOwner(ownerTokenA, {
        lockPath: ownerLockPath,
        pid: 41002,
        processIsAlive: () => false,
      }),
    "Node must reject even a same-token dead owner instead of recovering it",
  );
  // Model the wrapper's separately audited, Global-mutex protected removal.
  await fs.rm(ownerLockPath);
  heldOwner = await acquireCaptureOwner(ownerTokenA, {
    lockPath: ownerLockPath,
    pid: 41002,
    processIsAlive: () => false,
  });
  equal(heldOwner.descriptor.pid, 41002, "Node may acquire only after the wrapper removed the stale fixed lock");
  await releaseCaptureOwner(heldOwner);
  heldOwner = null;

  const competingOwners = await Promise.allSettled([
    acquireCaptureOwner(ownerTokenA, { lockPath: ownerLockPath, pid: 41004, processIsAlive: () => false }),
    acquireCaptureOwner(ownerTokenB, { lockPath: ownerLockPath, pid: 41005, processIsAlive: () => false }),
  ]);
  equal(
    competingOwners.filter((result) => result.status === "fulfilled").length,
    1,
    "an atomic fixed lock race must admit at most one Node owner",
  );
  equal(
    competingOwners.filter((result) => result.status === "rejected").length,
    1,
    "the losing Node owner must fail instead of deleting or recovering the winner's lock",
  );
  heldOwner = competingOwners.find((result) => result.status === "fulfilled").value;
  await releaseCaptureOwner(heldOwner);
  heldOwner = null;

  await fs.writeFile(
    ownerLockPath,
    `${JSON.stringify({
      schemaVersion: 1,
      ownerToken: ownerTokenA,
      pid: 41003,
      acquiredAt: "2026-09-01T00:00:00.000Z",
      unexpected: true,
    })}\n`,
    "utf8",
  );
  await rejects(
    () =>
      acquireCaptureOwner(ownerTokenA, {
        lockPath: ownerLockPath,
        processIsAlive: () => false,
      }),
    "a stale fixed owner with extra or corrupt descriptor fields must fail closed",
  );
  await fs.rm(ownerLockPath);
} finally {
  if (heldOwner) {
    try {
      await releaseCaptureOwner(heldOwner);
    } catch {
      // The test's assertions report ownership failures; cleanup remains best effort.
    }
  }
  await fs.rm(ownerTempRoot, { recursive: true, force: true });
}
const fingerprintSources = { fixtureCrane: "1".repeat(64), manifest: "2".repeat(64) };
const fingerprintApp = { executablePath: APP_ARGUMENTS[1], bytes: 1234, sha256: "a".repeat(64) };
const firstFingerprint = inputFingerprintSha256(fingerprintSources, fingerprintApp);
equal(
  firstFingerprint,
  inputFingerprintSha256({ manifest: "2".repeat(64), fixtureCrane: "1".repeat(64) }, fingerprintApp),
  "input fingerprint must not depend on source object insertion order",
);
check(
  firstFingerprint !== inputFingerprintSha256(fingerprintSources, { ...fingerprintApp, sha256: "b".repeat(64) }),
  "changing the bundled application bytes identity must change the aggregate fingerprint",
);
check(
  firstFingerprint !==
    inputFingerprintSha256({ ...fingerprintSources, fixtureCrane: "3".repeat(64) }, fingerprintApp),
  "changing a fixture content hash must change the aggregate fingerprint",
);
const savedFingerprintState = {
  sourceHashes: { ...fingerprintSources },
  appIdentity: { ...fingerprintApp },
  inputFingerprint: firstFingerprint,
};
equal(
  assertResumeFingerprint(savedFingerprintState, fingerprintSources, fingerprintApp),
  firstFingerprint,
  "resume must accept only the exact recorded source/application fingerprint",
);
throws(
  () =>
    assertResumeFingerprint(
      savedFingerprintState,
      { ...fingerprintSources, fixtureCrane: "3".repeat(64) },
      fingerprintApp,
    ),
  "resume must reject a changed fixture before mixing passed and new screenshots",
);
throws(
  () =>
    assertResumeFingerprint(savedFingerprintState, fingerprintSources, {
      ...fingerprintApp,
      sha256: "b".repeat(64),
    }),
  "resume must reject a different bundled desktop executable before promotion",
);
equal(
  promotionRecoveryDecision({ transactionExists: false, backupsReady: false, attemptCount: 0 }),
  "none",
  "a run that never entered promotion needs no rollback",
);
equal(
  promotionRecoveryDecision({ transactionExists: true, backupsReady: false, attemptCount: 0 }),
  "none",
  "an interrupted original-backup preparation cannot have mutated final outputs",
);
equal(
  promotionRecoveryDecision({ transactionExists: true, backupsReady: true, attemptCount: 0 }),
  "none",
  "durable originals without a started attempt need no rollback",
);
equal(
  promotionRecoveryDecision({ transactionExists: true, backupsReady: true, attemptCount: 1 }),
  "rollback",
  "an interrupted first promotion attempt must roll back before re-promotion",
);
equal(
  promotionRecoveryDecision({ transactionExists: true, backupsReady: true, attemptCount: 3 }),
  "rollback",
  "even a later or committed attempt must deterministically restore originals before re-promotion",
);
throws(
  () => promotionRecoveryDecision({ transactionExists: true, backupsReady: false, attemptCount: 1 }),
  "promotion must fail closed if output mutation could precede durable original backups",
);
throws(
  () => promotionRecoveryDecision({ transactionExists: false, backupsReady: true, attemptCount: 0 }),
  "orphaned promotion journal children without a transaction must fail closed",
);
throws(
  () => promotionRecoveryDecision({ transactionExists: true, backupsReady: true, attemptCount: -1 }),
  "invalid promotion attempt counts must fail closed",
);
const freshSelection = selectEntries(entries, defaultArguments);
equal(
  freshSelection.map((entry) => entry.name),
  EXPECTED_NAMES,
  "a fresh run must select all 46 entries in audited order",
);

const seventhName = EXPECTED_NAMES[6];
equal(
  selectEntries(entries, parseArguments(["--only", "7"])).map((entry) => entry.name),
  [seventhName],
  "--only must accept a one-based ordinal",
);
equal(
  selectEntries(entries, parseArguments(["--only", seventhName])).map((entry) => entry.name),
  [seventhName],
  "--only must accept an exact filename",
);

const resumeAfterSeven = makeResumeState(entries, 7);
equal(
  selectEntries(
    entries,
    parseArguments(["--resume", resumeRunPath]),
    resumeAfterSeven,
  ).map((entry) => entry.name),
  EXPECTED_NAMES.slice(7),
  "--resume must preserve selection order and skip passed entries",
);
const resumeAfterTen = makeResumeState(entries, 10);
equal(
  selectEntries(
    entries,
    parseArguments(["--resume", resumeRunPath, "--from", "8"]),
    resumeAfterTen,
  ).map((entry) => entry.name),
  EXPECTED_NAMES.slice(7),
  "--from must accept a one-based ordinal and restart passed entries in that resume suffix",
);
equal(
  selectEntries(
    entries,
    parseArguments(["--resume", resumeRunPath, "--from", EXPECTED_NAMES[7]]),
    resumeAfterTen,
  ).map((entry) => entry.name),
  EXPECTED_NAMES.slice(7),
  "--from must accept an exact filename and restart passed entries in that resume suffix",
);

throws(() => parseArguments(["--only"]), "--only without a selector must fail");
throws(() => parseArguments(["--from"]), "--from without a selector must fail");
throws(() => parseArguments(["--resume"]), "--resume without a run directory must fail");
throws(
  () => parseArguments(["--only", "7", "--only", "8"]),
  "duplicate --only must fail",
);
throws(
  () =>
    parseArguments([
      "--resume",
      resumeRunPath,
      "--from",
      "7",
      "--from",
      "8",
    ]),
  "duplicate --from must fail",
);
throws(
  () => parseArguments(["--resume", resumeRunPath, "--resume", resumeRunPath]),
  "duplicate --resume must fail",
);
throws(
  () => parseArguments(["--only", "7", "--resume", resumeRunPath]),
  "--only and --resume must be mutually exclusive",
);
throws(
  () =>
    parseArguments(["--only", "7", "--from", "8", "--resume", resumeRunPath]),
  "--only and --from must be mutually exclusive",
);
throws(
  () => parseArguments(["--from", "8"]),
  "--from without --resume must fail",
);
throws(() => parseArguments(["--unknown"]), "unknown flags must fail");
throws(() => parseArguments(["7"]), "positional arguments must fail");
throws(
  () => parseArguments(["--out-dir", path.join(repositoryRoot, "unsafe-output")]),
  "--out-dir must be rejected so output stays under docs/manual/assets",
);
throws(
  () => selectEntries(entries, parseArguments(["--only", "0"])),
  "--only ordinal zero must fail",
);
throws(
  () => selectEntries(entries, parseArguments(["--only", "47"])),
  "--only ordinal above the manifest must fail",
);
throws(
  () => selectEntries(entries, parseArguments(["--only", "screen-not-in-manifest.png"])),
  "--only unknown filename must fail",
);
throws(
  () =>
    selectEntries(
      entries,
      parseArguments(["--resume", resumeRunPath, "--from", "43"]),
      resumeAfterSeven,
    ),
  "--from ordinal above the manifest must fail",
);
throws(
  () =>
    selectEntries(
      entries,
      parseArguments([
        "--resume",
        resumeRunPath,
        "--from",
        "screen-not-in-manifest.png",
      ]),
      resumeAfterSeven,
    ),
  "--from unknown filename must fail",
);

const resumeWithUnknownName = cloneJson(resumeAfterSeven);
resumeWithUnknownName.selection[7] = "screen-not-in-manifest.png";
throws(
  () =>
    selectEntries(
      entries,
      parseArguments(["--resume", resumeRunPath]),
      resumeWithUnknownName,
    ),
  "--resume must reject a saved selection containing an unknown filename",
);
const resumeWithDuplicateName = cloneJson(resumeAfterSeven);
resumeWithDuplicateName.selection[7] = resumeWithDuplicateName.selection[6];
throws(
  () =>
    selectEntries(
      entries,
      parseArguments(["--resume", resumeRunPath]),
      resumeWithDuplicateName,
    ),
  "--resume must reject duplicate filenames in its saved selection",
);
const resumeOutOfOrder = cloneJson(resumeAfterSeven);
[resumeOutOfOrder.selection[7], resumeOutOfOrder.selection[8]] = [
  resumeOutOfOrder.selection[8],
  resumeOutOfOrder.selection[7],
];
throws(
  () =>
    selectEntries(
      entries,
      parseArguments(["--resume", resumeRunPath]),
      resumeOutOfOrder,
    ),
  "--resume must reject a saved selection that is not in manifest order",
);
throws(
  () =>
    selectEntries(
      entries,
      parseArguments(["--resume", resumeRunPath, "--from", "10"]),
      resumeAfterSeven,
    ),
  "--from must not skip an earlier resume entry that has not passed",
);

const visited = [];
const events = [];
let injectedFailure;
try {
  await executePlan({
    entries: entries.slice(0, 10),
    executeEntry: async (entry) => {
      visited.push(entry.ordinal);
      if (entry.ordinal === 7) throw new Error("injected failure at entry 7");
    },
    onEvent: ({ type, entry }) => events.push(`${type}:${entry.ordinal}`),
  });
} catch (error) {
  injectedFailure = error;
}
check(
  injectedFailure instanceof PlanExecutionError,
  "executePlan must propagate an entry failure as PlanExecutionError",
);
equal(
  injectedFailure?.entry?.ordinal,
  7,
  "PlanExecutionError must identify the exact manifest entry that failed",
);
equal(
  injectedFailure?.cause?.message,
  "injected failure at entry 7",
  "executePlan must preserve the cause that stopped the run",
);
equal(
  visited,
  [1, 2, 3, 4, 5, 6, 7],
  "executePlan must fail fast: entry 7 may run, entries 8 and later must not run",
);
equal(
  events,
  [
    "start:1",
    "pass:1",
    "start:2",
    "pass:2",
    "start:3",
    "pass:3",
    "start:4",
    "pass:4",
    "start:5",
    "pass:5",
    "start:6",
    "pass:6",
    "start:7",
    "fail:7",
  ],
  "fail-fast events must stop at the failure and must not report entry 8",
);

const cleanupAttempts = [];
let cleanupFailure = null;
try {
  await bestEffortCleanup("injected cleanup failure", [
    async () => {
      cleanupAttempts.push(1);
      throw new Error("first cleanup step failed");
    },
    async () => cleanupAttempts.push(2),
    async () => cleanupAttempts.push(3),
  ]);
} catch (error) {
  cleanupFailure = error;
}
check(cleanupFailure instanceof AggregateError, "best-effort cleanup must report an AggregateError");
equal(cleanupAttempts, [1, 2, 3], "one failed cleanup step must not skip the remaining restoration steps");
equal(cleanupFailure?.errors?.length, 1, "best-effort cleanup must retain the exact failed restoration step");

const finalCleanupCalls = [];
let trackedRemovalAttempts = 0;
let unblockAttempts = 0;
await cleanupClient({
  async releaseMouse() {
    finalCleanupCalls.push("release");
  },
  async clearTrackedPreloadScripts() {
    trackedRemovalAttempts += 1;
    finalCleanupCalls.push(`preload:${trackedRemovalAttempts}`);
    if (trackedRemovalAttempts === 1) throw new Error("transient preload removal failure");
  },
  async call(method) {
    finalCleanupCalls.push(method);
    if (method === "Network.setBlockedURLs") {
      unblockAttempts += 1;
      if (unblockAttempts === 1) throw new Error("transient unblock failure");
    }
  },
  async reload() {
    finalCleanupCalls.push("reload");
  },
  async rebindGeneration() {
    finalCleanupCalls.push("rebind");
  },
  async evaluate() {
    finalCleanupCalls.push("normal-view");
  },
  async waitFor() {
    finalCleanupCalls.push("normal-verified");
    return true;
  },
  close() {
    finalCleanupCalls.push("close");
  },
});
check(trackedRemovalAttempts >= 3, "final cleanup must retry and then repeat preload removal before reload");
check(unblockAttempts >= 3, "final cleanup must retry and then repeat URL unblock before reload");
check(
  finalCleanupCalls.indexOf("reload") < finalCleanupCalls.indexOf("normal-verified"),
  "final cleanup must verify normal Viewer3D only after a restored full reload",
);
equal(finalCleanupCalls.at(-1), "close", "final cleanup must close CDP after normal-view verification");

const terminalCleanupCalls = [];
let terminalCleanupError = null;
try {
  await cleanupClient({
    async releaseMouse() {},
    async clearTrackedPreloadScripts() {},
    async call(method) {
      terminalCleanupCalls.push(method);
      if (method === "Network.setBlockedURLs") throw new Error("persistent unblock failure");
    },
    async reload() {
      terminalCleanupCalls.push("reload");
    },
    async rebindGeneration() {
      terminalCleanupCalls.push("rebind");
    },
    async evaluate() {
      terminalCleanupCalls.push("normal-view");
    },
    async waitFor() {
      terminalCleanupCalls.push("normal-verified");
      return true;
    },
    close() {
      terminalCleanupCalls.push("close");
    },
  });
} catch (error) {
  terminalCleanupError = error;
}
check(terminalCleanupError instanceof AggregateError, "persistent restoration failure must keep the run failed");
check(terminalCleanupCalls.includes("reload"), "persistent unblock failure must not skip the final reload attempt");
check(
  terminalCleanupCalls.includes("normal-verified"),
  "persistent unblock failure must not skip final normal-view verification",
);
equal(terminalCleanupCalls.at(-1), "close", "persistent cleanup failure must still close the CDP client");

// Offline filesystem recovery: create durable originals, mimic a process kill
// after only the first destination changed, recover every original, then run a
// complete re-promotion.  No application, CDP endpoint, or screenshot capture
// is involved.
const promotionTempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "ori3-manual-promotion-test-"));
try {
  const promotionRunDirectory = path.join(promotionTempRoot, "run-offline");
  const promotionOutputDirectory = path.join(promotionTempRoot, "assets");
  const promotionScreensDirectory = path.join(promotionRunDirectory, "screens");
  await fs.mkdir(promotionScreensDirectory, { recursive: true });
  await fs.mkdir(promotionOutputDirectory, { recursive: true });

  const promotionEntries = [
    { ordinal: 1, name: "screen-offline-a.png", scenario: "offlineA" },
    { ordinal: 2, name: "screen-offline-b.png", scenario: "offlineB" },
  ];
  const originalBytes = new Map([
    [promotionEntries[0].name, Buffer.from("immutable original A\n", "utf8")],
    [promotionEntries[1].name, Buffer.from("immutable original B\n", "utf8")],
  ]);
  const promotedBytes = new Map([
    [
      promotionEntries[0].name,
      await fs.readFile(path.join(repositoryRoot, "docs", "manual", "assets", "screen-workspace.png")),
    ],
    [
      promotionEntries[1].name,
      await fs.readFile(path.join(repositoryRoot, "docs", "manual", "assets", "screen-warning.png")),
    ],
  ]);
  const promotionState = {
    runId: "run-20260901000000000-9001",
    inputFingerprint: "a".repeat(64),
    status: "running",
    current: null,
    entries: {},
  };
  for (const entry of promotionEntries) {
    const original = originalBytes.get(entry.name);
    const promoted = promotedBytes.get(entry.name);
    await fs.writeFile(path.join(promotionOutputDirectory, entry.name), original);
    await fs.writeFile(path.join(promotionScreensDirectory, entry.name), promoted);
    promotionState.entries[entry.name] = {
      ordinal: entry.ordinal,
      scenario: entry.scenario,
      status: "passed",
      bytes: promoted.length,
      width: 2560,
      height: 1720,
      sha256: createHash("sha256").update(promoted).digest("hex"),
    };
  }

  await acquirePromotionLease(promotionEntries, promotionState, promotionOutputDirectory);
  const originalJournal = await prepareOriginalBackups(
    promotionEntries,
    promotionState,
    promotionRunDirectory,
    promotionOutputDirectory,
  );
  const immutableBackupHashes = new Map();
  for (const entry of promotionEntries) {
    const backup = originalJournal.receipts.get(entry.name).backupPath;
    immutableBackupHashes.set(entry.name, createHash("sha256").update(await fs.readFile(backup)).digest("hex"));
  }

  const interruptedAttempt = path.join(originalJournal.paths.attempts, "attempt-000001");
  await fs.mkdir(interruptedAttempt);
  await fs.writeFile(
    path.join(interruptedAttempt, "started.json"),
    `${JSON.stringify({ schemaVersion: 1, id: "attempt-000001" })}\n`,
    "utf8",
  );
  await fs.writeFile(
    path.join(promotionOutputDirectory, promotionEntries[0].name),
    promotedBytes.get(promotionEntries[0].name),
  );

  const competingRunDirectory = path.join(promotionTempRoot, "run-competing");
  const competingScreensDirectory = path.join(competingRunDirectory, "screens");
  await fs.mkdir(competingScreensDirectory, { recursive: true });
  const competingState = {
    runId: "run-20260901000000000-9002",
    inputFingerprint: "b".repeat(64),
    status: "running",
    current: null,
    entries: JSON.parse(JSON.stringify(promotionState.entries)),
  };
  for (const entry of promotionEntries) {
    await fs.writeFile(
      path.join(competingScreensDirectory, entry.name),
      promotedBytes.get(entry.name),
    );
  }
  await rejects(
    () =>
      promoteSelection({
        entries: promotionEntries,
        state: competingState,
        runDirectory: competingRunDirectory,
        outDir: promotionOutputDirectory,
        requireExactSet: false,
      }),
    "a fresh run must not promote or roll back while an interrupted older run owns the output lease",
  );

  const recoveryId = await recoverPromotionBeforeResume(
    promotionEntries,
    promotionState,
    promotionRunDirectory,
    promotionOutputDirectory,
  );
  check(/^recovery-\d{6}$/.test(recoveryId), "an interrupted promotion must create a durable recovery journal");
  for (const entry of promotionEntries) {
    equal(
      await fs.readFile(path.join(promotionOutputDirectory, entry.name)),
      originalBytes.get(entry.name),
      `promotion recovery must restore every original byte: ${entry.name}`,
    );
  }

  await promoteSelection({
    entries: promotionEntries,
    state: promotionState,
    runDirectory: promotionRunDirectory,
    outDir: promotionOutputDirectory,
    requireExactSet: false,
  });
  for (const entry of promotionEntries) {
    equal(
      await fs.readFile(path.join(promotionOutputDirectory, entry.name)),
      promotedBytes.get(entry.name),
      `re-promotion after recovery must install every new image: ${entry.name}`,
    );
    const backup = originalJournal.receipts.get(entry.name).backupPath;
    equal(
      createHash("sha256").update(await fs.readFile(backup)).digest("hex"),
      immutableBackupHashes.get(entry.name),
      `re-promotion must never overwrite the immutable original backup: ${entry.name}`,
    );
  }

  const laterRunBytes = new Map([
    [promotionEntries[0].name, Buffer.from("later run B owns A\n", "utf8")],
    [promotionEntries[1].name, Buffer.from("later run B owns B\n", "utf8")],
  ]);
  for (const entry of promotionEntries) {
    await fs.writeFile(path.join(promotionOutputDirectory, entry.name), laterRunBytes.get(entry.name));
  }
  await rejects(
    () =>
      recoverPromotionBeforeResume(
        promotionEntries,
        promotionState,
        promotionRunDirectory,
        promotionOutputDirectory,
      ),
    "an old run whose lease was released must reject outputs containing bytes from a later run",
  );
  for (const entry of promotionEntries) {
    equal(
      await fs.readFile(path.join(promotionOutputDirectory, entry.name)),
      laterRunBytes.get(entry.name),
      `rollback CAS rejection must leave every later-run byte untouched: ${entry.name}`,
    );
  }

  await promoteSelection({
    entries: promotionEntries,
    state: competingState,
    runDirectory: competingRunDirectory,
    outDir: promotionOutputDirectory,
    requireExactSet: false,
  });
  check(
    competingState.status === "complete",
    "after the old run resumes, recovers, and releases its lease, a new run must be able to promote",
  );

  const changingInput = path.join(promotionTempRoot, "changing-fixture.bin");
  await fs.writeFile(changingInput, "before", "utf8");
  const finalVerificationApp = {
    executablePath: "C:/offline/desktop.exe",
    bytes: 123,
    sha256: "c".repeat(64),
  };
  const collectChangingInput = async () => ({
    sourceHashes: {
      fixtureCrane: createHash("sha256").update(await fs.readFile(changingInput)).digest("hex"),
    },
    appIdentity: finalVerificationApp,
  });
  const initialChangingInputs = await collectChangingInput();
  const expectedChangingFingerprint = inputFingerprintSha256(
    initialChangingInputs.sourceHashes,
    initialChangingInputs.appIdentity,
  );
  await verifyFinalInputFingerprint(expectedChangingFingerprint, collectChangingInput);
  await fs.writeFile(changingInput, "after", "utf8");
  await rejects(
    () => verifyFinalInputFingerprint(expectedChangingFingerprint, collectChangingInput),
    "a fixture changed after capture must fail final verification before promotion",
  );
} finally {
  await fs.rm(promotionTempRoot, { recursive: true, force: true });
}

const runnerSource = await fs.readFile(path.join(scriptDirectory, "capture-manual-screens.mjs"), "utf8");
const wrapperSource = await fs.readFile(path.join(scriptDirectory, "capture-manual-screens.ps1"), "utf8");
const cdpClientSource = await fs.readFile(path.join(scriptDirectory, "manual-capture", "cdp-client.mjs"), "utf8");
equal(
  [...runnerSource.matchAll(/fs\.copyFile\(/g)].length,
  1,
  "all durable copies must go through the one exclusive-copy primitive",
);
check(
  runnerSource.includes("fs.copyFile(source, destination, fsConstants.COPYFILE_EXCL)"),
  "the copy primitive must refuse to overwrite an existing original or output artifact",
);
check(
  !runnerSource.includes("fs.rename("),
  "promotion must not fall back to platform-dependent rename-overwrite behavior",
);
check(
  !runnerSource.includes("fs.writeFile("),
  "promotion and run state must not directly overwrite an existing file",
);
check(
  runnerSource.includes("await recoverPromotionBeforeResume(promotionEntries, state, runDirectory, outDir)"),
  "resume must invoke durable promotion recovery before resetting and re-promoting entries",
);
const finalVerificationIndex = runnerSource.indexOf("await verifyFinalInputFingerprint(state.inputFingerprint");
const finalPromotionIndex = runnerSource.lastIndexOf("await promoteSelection({");
check(
  finalVerificationIndex >= 0 && finalPromotionIndex > finalVerificationIndex,
  "all current input hashes must be verified after capture and before final promotion",
);
check(
  runnerSource.includes(".origami3-manual-promotion-lease.json"),
  "cross-run promotion ownership must be recorded beside the final output",
);
check(
  runnerSource.includes('path.join(defaultStagingRoot, ".capture-owner-lock")'),
  "capture ownership must use a repository-fixed filesystem lock independent of a custom staging root",
);
check(
  runnerSource.includes("const owner = await acquireCaptureOwner(options.ownerToken)"),
  "the Node runner must acquire its filesystem owner before entering the capture workflow",
);
const nodeOwnerExistsStart = runnerSource.indexOf('if (error?.code !== "EEXIST") throw error;');
const nodeOwnerExistsEnd = runnerSource.indexOf("\n  }\n\n  try {", nodeOwnerExistsStart);
check(
  nodeOwnerExistsStart >= 0 && nodeOwnerExistsEnd > nodeOwnerExistsStart,
  "the Node EEXIST owner branch must remain statically identifiable",
);
const nodeOwnerExistsBranch = runnerSource.slice(nodeOwnerExistsStart, nodeOwnerExistsEnd);
check(
  !nodeOwnerExistsBranch.includes("fs.rm(resolvedLockPath)"),
  "Node must never delete or recover an existing owner lock after EEXIST",
);
check(
  nodeOwnerExistsBranch.includes("only the Global-mutex PowerShell wrapper may recover a stale lock"),
  "Node EEXIST diagnostics must delegate all stale recovery to the wrapper",
);
const nodeAcquireOwnerSource = runnerSource.slice(
  runnerSource.indexOf("export async function acquireCaptureOwner("),
  runnerSource.indexOf("export async function releaseCaptureOwner("),
);
check(
  !nodeAcquireOwnerSource.includes("fs.rm(resolvedLockPath)"),
  "Node owner acquisition must never remove the fixed lock; only release or the wrapper may do so",
);
check(
  cdpClientSource.includes("await initializeManualCaptureConnection({ call, lockMetrics, close })"),
  "the production CDP connector must use the initialization helper whose failure path closes the socket",
);
check(
  cdpClientSource.includes("await awaitManualCaptureSocketOpen({"),
  "the production CDP connector must use the open-wait helper whose failure path closes CONNECTING sockets",
);
check(
  cdpClientSource.includes("const targets = await discoverManualCaptureTargets({ endpoint })"),
  "production target discovery must place headers and JSON body parsing under the abortable helper",
);
check(
  !cdpClientSource.includes("withTimeout(fetch("),
  "production target discovery must not leave an uncancelled fetch behind a Promise.race timeout",
);
check(
  !wrapperSource.includes('if ([string]::IsNullOrWhiteSpace($Resume))'),
  "resume must not bypass the exact-one desktop process and listener-owner preflight",
);
for (const requiredWrapperContract of [
  'Get-Process -Name "desktop"',
  "Get-NetTCPConnection -LocalPort $Port",
  "Test-ProcessDescendsFrom",
  "Get-FileHash -LiteralPath $appExecutable -Algorithm SHA256",
  ".Hash.ToLowerInvariant()",
  '"--app-exe"',
  '"--app-sha256"',
  '"--owner-token"',
  '[Guid]::NewGuid().ToString("N").ToLowerInvariant()',
  'ConvertFrom-Json',
  'verification\\manual-capture\\.capture-owner-lock',
  'Get-Process -Id $first.OwnerPid -ErrorAction SilentlyContinue',
  '$first.OwnerToken -cne $ExpectedOwnerToken',
  '$second.Raw -cne $first.Raw',
  '[System.IO.FileAttributes]::ReparsePoint',
  '$second.CreationTimeUtcTicks -ne $first.CreationTimeUtcTicks',
  '$second.LastWriteTimeUtcTicks -ne $first.LastWriteTimeUtcTicks',
  'Remove-Item -LiteralPath $captureOwnerLockPath -Force -ErrorAction Stop',
  '$ownerToken = $recoveredOwnerToken',
  '"Global\\ORIGAMI3-ManualCapture-$rootHash"',
  "System.Threading.Mutex",
  "System.Threading.AbandonedMutexException",
  'Get-CimInstance -ClassName Win32_Process -Filter "Name = \'node.exe\'"',
  "A manual screenshot Node runner is already active or orphaned",
]) {
  check(
    wrapperSource.includes(requiredWrapperContract),
    `PowerShell wrapper must preserve capture identity/preflight contract: ${requiredWrapperContract}`,
  );
}

process.stdout.write(
  `MANUAL_SCREEN_CAPTURE_TEST assertions=${assertions} rows=${manifest.length} scenarios=${registry.length}\n`,
);
