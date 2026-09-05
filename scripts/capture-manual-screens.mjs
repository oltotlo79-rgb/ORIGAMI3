import { createHash } from "node:crypto";
import { constants as fsConstants } from "node:fs";
import fs from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import { connectManualCapture } from "./manual-capture/cdp-client.mjs";
import { createScenarioRegistry } from "./manual-capture/scenarios.mjs";

const RUN_STATE_SCHEMA = 2;
const MANUAL_SCREEN_COUNT = 46;
const SCREEN_WIDTH = 2560;
const SCREEN_HEIGHT = 1720;
const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

const runnerPath = fileURLToPath(import.meta.url);
export const repositoryRoot = path.resolve(path.dirname(runnerPath), "..");
export const manifestPath = path.join(repositoryRoot, "scripts", "manual-screenshot-manifest.json");
const scenarioRegistryPath = path.join(repositoryRoot, "scripts", "manual-capture", "scenarios.mjs");
const cdpClientPath = path.join(repositoryRoot, "scripts", "manual-capture", "cdp-client.mjs");
const defaultOutputDirectory = path.join(repositoryRoot, "docs", "manual", "assets");
const defaultStagingRoot = path.join(repositoryRoot, "verification", "manual-capture");
const defaultCaptureOwnerLockPath = path.join(defaultStagingRoot, ".capture-owner-lock");
// `crane` は正本由来の3手の作品。`scripts/manual-capture/scenarios.mjs` の `FIXTURES` と同じ実体を指す。
export const captureFixturePaths = Object.freeze({
  crane: path.join(repositoryRoot, "apps", "desktop", "tests-live", "fixtures", "traditional-crane-full.ori3"),
  yakko: path.join(repositoryRoot, "crates", "ori3-rigid", "tests", "fixtures", "check-yakko.ori3"),
  bird: path.join(repositoryRoot, "crates", "ori3-rigid", "tests", "fixtures", "check-bird-base.ori3"),
  penetration: path.join(repositoryRoot, "crates", "ori3-layers", "tests", "fixtures", "penetration-warning.ori3"),
});

function argumentError(message) {
  return new Error(`${message}\n${usage()}`);
}

function usage() {
  return [
    "internal runner: use powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\capture-manual-screens.ps1",
    "wrapper-internal usage: node scripts/capture-manual-screens.mjs [options]",
    "  --only <screen-name-or-ordinal>",
    "  --resume <run-directory> [--from <screen-name-or-ordinal>]",
    "  --staging-root <directory>",
    "  --endpoint <http://127.0.0.1:port>",
    "  --app-exe <absolute desktop.exe path>",
    "  --app-sha256 <64 lowercase hexadecimal digits>",
    "  --owner-token <32 lowercase hexadecimal digits from the PowerShell wrapper>",
    "  --list",
  ].join("\n");
}

/** Parse without touching the filesystem or CDP so the CLI contract is testable offline. */
export function parseArguments(argv) {
  const result = {
    endpoint: "http://127.0.0.1:9222",
    stagingRoot: defaultStagingRoot,
    only: null,
    from: null,
    resume: null,
    appExe: null,
    appSha256: null,
    ownerToken: null,
    list: false,
  };
  const valueOptions = new Map([
    ["--endpoint", "endpoint"],
    ["--staging-root", "stagingRoot"],
    ["--only", "only"],
    ["--from", "from"],
    ["--resume", "resume"],
    ["--app-exe", "appExe"],
    ["--app-sha256", "appSha256"],
    ["--owner-token", "ownerToken"],
  ]);
  const seen = new Set();

  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index];
    if (option === "--list") {
      if (seen.has(option)) throw argumentError(`${option} was specified more than once`);
      seen.add(option);
      result.list = true;
      continue;
    }
    const property = valueOptions.get(option);
    if (!property) throw argumentError(`unknown option or positional argument: ${option}`);
    if (seen.has(option)) throw argumentError(`${option} was specified more than once`);
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--") || value.trim() === "") {
      throw argumentError(`${option} requires one non-empty value`);
    }
    seen.add(option);
    result[property] = value;
    index += 1;
  }

  if (result.only !== null && (result.resume !== null || result.from !== null)) {
    throw argumentError("--only cannot be combined with --resume or --from");
  }
  if (result.from !== null && result.resume === null) {
    throw argumentError("--from is valid only together with --resume");
  }
  if (result.list && (result.only !== null || result.resume !== null || result.from !== null)) {
    throw argumentError("--list cannot be combined with capture or resume selection options");
  }
  if (!result.list && (result.appExe === null || result.appSha256 === null || result.ownerToken === null)) {
    throw argumentError("capture and resume require --app-exe, --app-sha256, and a wrapper --owner-token");
  }
  if (result.appSha256 !== null && !/^[a-f0-9]{64}$/.test(result.appSha256)) {
    throw argumentError("--app-sha256 must be exactly 64 lowercase hexadecimal digits");
  }
  if (result.ownerToken !== null && !/^[a-f0-9]{32}$/.test(result.ownerToken)) {
    throw argumentError("--owner-token must be exactly 32 lowercase hexadecimal digits");
  }
  return result;
}

function resolveEntry(entries, selector, label) {
  const text = String(selector).trim();
  let entry;
  if (/^[1-9]\d*$/.test(text)) {
    const ordinal = Number.parseInt(text, 10);
    entry = entries.find((candidate) => candidate.ordinal === ordinal);
  } else {
    entry = entries.find((candidate) => candidate.name === text);
  }
  if (!entry) throw new Error(`${label} does not identify a manifest screen: ${JSON.stringify(text)}`);
  return entry;
}

/**
 * Validate the single tracked manifest against executable scenario IDs.
 * The returned entries retain their scenario runner but do not execute it.
 */
export function validateManifest(manifest, registry) {
  if (!Array.isArray(manifest)) throw new Error("manual screenshot manifest must be a top-level array");
  if (manifest.length !== MANUAL_SCREEN_COUNT) {
    throw new Error(`manual screenshot manifest must contain ${MANUAL_SCREEN_COUNT} entries, found ${manifest.length}`);
  }
  if (!Array.isArray(registry)) throw new Error("createScenarioRegistry() must return an array");

  const scenarioById = new Map();
  for (const scenario of registry) {
    if (!scenario || typeof scenario.id !== "string" || scenario.id.trim() === "") {
      throw new Error("each scenario registry item must have a non-empty string id");
    }
    if (typeof scenario.run !== "function") {
      throw new Error(`scenario ${JSON.stringify(scenario.id)} has no run(context) function`);
    }
    if (scenarioById.has(scenario.id)) throw new Error(`duplicate scenario id: ${scenario.id}`);
    scenarioById.set(scenario.id, scenario);
  }

  const names = new Set();
  const scenarioIds = new Set();
  const entries = manifest.map((raw, index) => {
    const ordinal = index + 1;
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
      throw new Error(`manifest entry ${ordinal} must be an object`);
    }
    if (raw.ordinal !== ordinal) {
      throw new Error(`manifest ordinal at position ${ordinal} must be ${ordinal}, got ${JSON.stringify(raw.ordinal)}`);
    }
    if (typeof raw.name !== "string" || !/^screen-[a-z0-9]+(?:-[a-z0-9]+)*\.png$/.test(raw.name)) {
      throw new Error(`manifest entry ${ordinal} has an invalid output name: ${JSON.stringify(raw.name)}`);
    }
    if (path.basename(raw.name) !== raw.name) {
      throw new Error(`manifest output name must be a basename: ${raw.name}`);
    }
    if (names.has(raw.name)) throw new Error(`duplicate manifest output name: ${raw.name}`);
    names.add(raw.name);
    if (typeof raw.scenario !== "string" || raw.scenario.trim() === "") {
      throw new Error(`manifest entry ${ordinal} has no scenario id`);
    }
    if (scenarioIds.has(raw.scenario)) throw new Error(`manifest scenario is assigned more than once: ${raw.scenario}`);
    scenarioIds.add(raw.scenario);
    const scenario = scenarioById.get(raw.scenario);
    if (!scenario) throw new Error(`manifest scenario is not implemented: ${raw.scenario}`);
    return Object.freeze({
      ordinal,
      name: raw.name,
      scenario: raw.scenario,
      raw: Object.freeze({ ...raw }),
      run: scenario.run,
    });
  });

  const unassigned = [...scenarioById.keys()].filter((id) => !scenarioIds.has(id));
  if (unassigned.length > 0) {
    throw new Error(`scenario registry contains unassigned ids: ${unassigned.join(", ")}`);
  }
  if (registry.length !== MANUAL_SCREEN_COUNT) {
    throw new Error(`scenario registry must contain ${MANUAL_SCREEN_COUNT} entries, found ${registry.length}`);
  }
  return entries;
}

/** Select one, a resume suffix, or all entries without mutating resume state. */
export function selectEntries(entries, options, resumeState = null) {
  if (options.only !== null && options.only !== undefined) {
    return [resolveEntry(entries, options.only, "--only")];
  }
  if (!resumeState) return [...entries];
  if (!Array.isArray(resumeState.selection) || resumeState.selection.length === 0) {
    throw new Error("resume state has no ordered selection");
  }
  const byName = new Map(entries.map((entry) => [entry.name, entry]));
  const selection = resumeState.selection.map((name) => {
    const entry = byName.get(name);
    if (!entry) throw new Error(`resume selection is absent from the current manifest: ${name}`);
    return entry;
  });
  if (new Set(resumeState.selection).size !== resumeState.selection.length) {
    throw new Error("resume selection contains duplicate names");
  }
  const sorted = [...selection].sort((left, right) => left.ordinal - right.ordinal);
  if (sorted.some((entry, index) => entry !== selection[index])) {
    throw new Error("resume selection is not in manifest order");
  }

  if (options.from !== null && options.from !== undefined) {
    const first = resolveEntry(selection, options.from, "--from");
    const start = selection.indexOf(first);
    for (const prior of selection.slice(0, start)) {
      if (resumeState.entries?.[prior.name]?.status !== "passed") {
        throw new Error(`--from would skip an entry that has not passed: ${prior.name}`);
      }
    }
    return selection.slice(start);
  }
  return selection.filter((entry) => resumeState.entries?.[entry.name]?.status !== "passed");
}

export class PlanExecutionError extends Error {
  constructor(entry, cause) {
    super(`capture failed at ${entry.ordinal}/${MANUAL_SCREEN_COUNT} ${entry.name}: ${errorText(cause)}`);
    this.name = "PlanExecutionError";
    this.entry = entry;
    this.cause = cause;
  }
}

/** Pure fail-fast sequencer; CDP and filesystem work are injected by the caller. */
export async function executePlan({ entries, executeEntry, onEvent = async () => {} }) {
  const results = [];
  for (const entry of entries) {
    await onEvent({ type: "start", entry });
    try {
      const result = await executeEntry(entry);
      results.push({ entry, result });
      await onEvent({ type: "pass", entry, result });
    } catch (error) {
      await onEvent({ type: "fail", entry, error });
      throw new PlanExecutionError(entry, error);
    }
  }
  return results;
}

function errorText(error) {
  if (error instanceof Error) return error.stack ?? error.message;
  return String(error);
}

function quotePowerShellLiteral(value) {
  return `'${String(value).replaceAll("'", "''")}'`;
}

function sha256Bytes(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

async function sha256File(filePath) {
  return sha256Bytes(await fs.readFile(filePath));
}

function stableFingerprintPayload(sourceHashes, appIdentity) {
  return JSON.stringify({
    schemaVersion: 1,
    application: {
      executablePath: appIdentity.executablePath,
      bytes: appIdentity.bytes,
      sha256: appIdentity.sha256,
    },
    sources: Object.fromEntries(Object.entries(sourceHashes).sort(([left], [right]) => left.localeCompare(right))),
  });
}

export function inputFingerprintSha256(sourceHashes, appIdentity) {
  return sha256Bytes(Buffer.from(stableFingerprintPayload(sourceHashes, appIdentity), "utf8"));
}

export function assertResumeFingerprint(state, sourceHashes, appIdentity) {
  for (const [name, hash] of Object.entries(sourceHashes)) {
    if (state.sourceHashes?.[name] !== hash) {
      throw new Error(`resume source hash changed for ${name}; start a fresh run`);
    }
  }
  if (
    state.appIdentity?.executablePath !== appIdentity.executablePath ||
    state.appIdentity?.bytes !== appIdentity.bytes ||
    state.appIdentity?.sha256 !== appIdentity.sha256
  ) {
    throw new Error("resume bundled application identity changed; start a fresh run");
  }
  const expected = inputFingerprintSha256(sourceHashes, appIdentity);
  if (state.inputFingerprint !== expected) {
    throw new Error("resume aggregate input fingerprint changed; start a fresh run");
  }
  return expected;
}

export async function verifyFinalInputFingerprint(expectedFingerprint, collectCurrentInputs) {
  if (!/^[a-f0-9]{64}$/.test(expectedFingerprint ?? "")) {
    throw new Error("recorded aggregate input fingerprint is invalid");
  }
  if (typeof collectCurrentInputs !== "function") {
    throw new Error("final input verification requires a current-input collector");
  }
  const current = await collectCurrentInputs();
  const actualFingerprint = inputFingerprintSha256(current.sourceHashes, current.appIdentity);
  if (actualFingerprint !== expectedFingerprint) {
    throw new Error(
      `inputs changed after capture and before promotion; start a fresh run: expected=${expectedFingerprint}, actual=${actualFingerprint}`,
    );
  }
  return { ...current, inputFingerprint: actualFingerprint };
}

async function inspectApplicationIdentity(appExe, claimedSha256) {
  if (!path.isAbsolute(appExe)) throw new Error("--app-exe must be an absolute path");
  const resolved = await fs.realpath(appExe);
  if (path.basename(resolved).toLowerCase() !== "desktop.exe") {
    throw new Error(`--app-exe does not identify desktop.exe: ${resolved}`);
  }
  const stat = await fs.stat(resolved);
  if (!stat.isFile()) throw new Error(`--app-exe is not a regular file: ${resolved}`);
  const actualSha256 = await sha256File(resolved);
  const afterHash = await fs.stat(resolved);
  if (
    afterHash.size !== stat.size ||
    afterHash.mtimeMs !== stat.mtimeMs ||
    afterHash.birthtimeMs !== stat.birthtimeMs
  ) {
    throw new Error(`desktop.exe changed while its build identity was being hashed: ${resolved}`);
  }
  if (actualSha256 !== claimedSha256) {
    throw new Error(
      `running desktop.exe SHA-256 differs from --app-sha256: expected=${claimedSha256}, actual=${actualSha256}`,
    );
  }
  return Object.freeze({ executablePath: resolved, bytes: afterHash.size, sha256: actualSha256 });
}

function inspectPng(bytes, label) {
  if (bytes.length < 24 || !bytes.subarray(0, PNG_SIGNATURE.length).equals(PNG_SIGNATURE)) {
    throw new Error(`${label} is not a PNG file`);
  }
  if (bytes.toString("ascii", 12, 16) !== "IHDR") throw new Error(`${label} has no leading IHDR chunk`);
  const width = bytes.readUInt32BE(16);
  const height = bytes.readUInt32BE(20);
  if (width !== SCREEN_WIDTH || height !== SCREEN_HEIGHT) {
    throw new Error(`${label} must be ${SCREEN_WIDTH}x${SCREEN_HEIGHT}, got ${width}x${height}`);
  }
  return { bytes: bytes.length, width, height, sha256: sha256Bytes(bytes) };
}

async function inspectPngFile(filePath, label = filePath) {
  return inspectPng(await fs.readFile(filePath), label);
}

function isPathWithin(parent, candidate) {
  const relative = path.relative(path.resolve(parent), path.resolve(candidate));
  return relative !== "" && !relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative);
}

function resolveCliPath(value) {
  return path.resolve(repositoryRoot, value);
}

function validateEndpoint(value) {
  const endpoint = new URL(value);
  if (endpoint.protocol !== "http:" || !["127.0.0.1", "localhost"].includes(endpoint.hostname)) {
    throw new Error(`CDP endpoint must be loopback HTTP: ${value}`);
  }
  if (endpoint.username || endpoint.password || (endpoint.pathname !== "/" && endpoint.pathname !== "")) {
    throw new Error(`CDP endpoint must not contain credentials or a path: ${value}`);
  }
  return endpoint.href.replace(/\/$/, "");
}

let durableFileSequence = 0;

async function fileExists(filePath) {
  try {
    await fs.lstat(filePath);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }
}

async function syncFile(filePath) {
  // On Windows fsync on a read-only handle fails with EPERM.  Every caller
  // owns the newly-created regular file, so use a writable handle and perform
  // the real flush instead of treating EPERM as success.
  const handle = await fs.open(filePath, "r+");
  try {
    await handle.sync();
  } finally {
    await handle.close();
  }
}

async function syncDirectory(directory) {
  let handle;
  try {
    handle = await fs.open(directory, "r");
    await handle.sync();
  } catch (error) {
    // Windows does not expose directory fsync through every filesystem.  Each
    // file is still fsynced before it is linked into its durable name.
    if (!['EACCES', 'EINVAL', 'EISDIR', 'ENOTSUP', 'EPERM'].includes(error?.code)) throw error;
  } finally {
    await handle?.close();
  }
}

async function writeDurableExclusive(filePath, bytes) {
  const handle = await fs.open(filePath, "wx");
  try {
    await handle.writeFile(bytes);
    await handle.sync();
  } finally {
    await handle.close();
  }
  await syncDirectory(path.dirname(filePath));
}

async function copyDurableExclusive(source, destination) {
  await fs.copyFile(source, destination, fsConstants.COPYFILE_EXCL);
  await syncFile(destination);
  await syncDirectory(path.dirname(destination));
}

async function linkThenUnlink(source, destination) {
  await fs.link(source, destination);
  await syncDirectory(path.dirname(destination));
  await fs.rm(source);
  await syncDirectory(path.dirname(source));
}

async function recoverRotatedFile(filePath) {
  const previous = `${filePath}.previous`;
  if (!(await fileExists(filePath)) && (await fileExists(previous))) {
    await fs.link(previous, filePath);
    await syncDirectory(path.dirname(filePath));
  }
}

async function installPreparedFile(temporary, destination) {
  const previous = `${destination}.previous`;
  await recoverRotatedFile(destination);
  if (await fileExists(destination)) {
    await fs.rm(previous, { force: true });
    await fs.link(destination, previous);
    await syncDirectory(path.dirname(destination));
    await fs.rm(destination);
    await syncDirectory(path.dirname(destination));
  }
  try {
    await linkThenUnlink(temporary, destination);
  } catch (error) {
    await recoverRotatedFile(destination);
    throw error;
  }
  await fs.rm(previous, { force: true });
  await syncDirectory(path.dirname(destination));
}

async function writeJsonAtomic(filePath, value) {
  durableFileSequence += 1;
  const temporary = `${filePath}.tmp-${process.pid}-${durableFileSequence}`;
  await writeDurableExclusive(temporary, `${JSON.stringify(value, null, 2)}\n`);
  await installPreparedFile(temporary, filePath);
}

async function replaceFile(source, destination) {
  durableFileSequence += 1;
  const temporary = path.join(
    path.dirname(destination),
    `.${path.basename(destination)}.manual-capture-${process.pid}-${durableFileSequence}.tmp`,
  );
  await copyDurableExclusive(source, temporary);
  await installPreparedFile(temporary, destination);
}

async function readJson(filePath, label) {
  let text;
  try {
    await recoverRotatedFile(filePath);
    text = await fs.readFile(filePath, "utf8");
  } catch (error) {
    throw new Error(`${label} could not be read at ${filePath}: ${errorText(error)}`);
  }
  try {
    return { value: JSON.parse(text), text };
  } catch (error) {
    throw new Error(`${label} is not valid JSON at ${filePath}: ${errorText(error)}`);
  }
}

function assertCaptureOwnerToken(ownerToken) {
  if (typeof ownerToken !== "string" || !/^[a-f0-9]{32}$/.test(ownerToken)) {
    throw new Error("capture owner token must be exactly 32 lowercase hexadecimal digits");
  }
}

function assertCaptureOwnerLockPath(lockPath) {
  const resolved = path.resolve(lockPath);
  if (path.basename(resolved) !== ".capture-owner-lock" || path.dirname(resolved) === resolved) {
    throw new Error(`capture owner lock must use the exact .capture-owner-lock path: ${resolved}`);
  }
  return resolved;
}

function captureOwnerProcessIsAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (error?.code === "ESRCH") return false;
    if (error?.code === "EPERM") return true;
    throw error;
  }
}

async function readCaptureOwner(lockPath) {
  let lockStat;
  try {
    lockStat = await fs.lstat(lockPath);
  } catch (error) {
    throw new Error(`manual capture owner lock could not be inspected: ${lockPath}: ${errorText(error)}`);
  }
  if (!lockStat.isFile() || lockStat.isSymbolicLink()) {
    throw new Error(`manual capture owner lock is not an ordinary file: ${lockPath}`);
  }
  const ownerPath = lockPath;
  let text;
  try {
    text = await fs.readFile(ownerPath, "utf8");
  } catch (error) {
    throw new Error(`manual capture owner lock is incomplete and requires inspection: ${ownerPath}: ${errorText(error)}`);
  }
  let value;
  try {
    value = JSON.parse(text);
  } catch (error) {
    throw new Error(`manual capture owner lock is corrupt and requires inspection: ${ownerPath}: ${errorText(error)}`);
  }
  const descriptorKeys = value && typeof value === "object" && !Array.isArray(value) ? Object.keys(value).sort() : [];
  if (
    descriptorKeys.join(",") !== "acquiredAt,ownerToken,pid,schemaVersion" ||
    value?.schemaVersion !== 1 ||
    typeof value.ownerToken !== "string" ||
    !/^[a-f0-9]{32}$/.test(value.ownerToken) ||
    !Number.isSafeInteger(value.pid) ||
    value.pid <= 0 ||
    typeof value.acquiredAt !== "string" ||
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/.test(value.acquiredAt) ||
    !Number.isFinite(Date.parse(value.acquiredAt))
  ) {
    throw new Error(`manual capture owner lock has an invalid descriptor and requires inspection: ${ownerPath}`);
  }
  return { value, text, ownerPath };
}

/**
 * Hold a repository-fixed filesystem owner for the entire capture/resume.
 * The PowerShell wrapper also owns a Global mutex; this durable file is
 * the cross-session/process boundary that a direct Node invocation cannot
 * bypass merely by racing the wrapper. A dead owner is reusable only by the
 * exact token persisted in run.json, or read from this complete descriptor by
 * the wrapper after a pre-run crash.
 */
export async function acquireCaptureOwner(
  ownerToken,
  {
    lockPath = defaultCaptureOwnerLockPath,
    pid = process.pid,
    processIsAlive = captureOwnerProcessIsAlive,
  } = {},
) {
  assertCaptureOwnerToken(ownerToken);
  if (!Number.isSafeInteger(pid) || pid <= 0) throw new Error("capture owner pid must be a positive safe integer");
  if (typeof processIsAlive !== "function") throw new Error("capture owner process liveness probe is invalid");
  const resolvedLockPath = assertCaptureOwnerLockPath(lockPath);
  const lockParent = path.dirname(resolvedLockPath);
  await fs.mkdir(lockParent, { recursive: true });

  const descriptor = {
    schemaVersion: 1,
    ownerToken,
    pid,
    acquiredAt: new Date().toISOString(),
  };
  durableFileSequence += 1;
  const candidatePath = `${resolvedLockPath}.candidate-${ownerToken}-${pid}-${Date.now()}-${durableFileSequence}`;
  await writeDurableExclusive(candidatePath, `${JSON.stringify(descriptor, null, 2)}\n`);
  try {
    await fs.link(candidatePath, resolvedLockPath);
    await syncDirectory(lockParent);
  } catch (error) {
    await fs.rm(candidatePath, { force: true });
    await syncDirectory(lockParent);
    if (error?.code !== "EEXIST") throw error;
    const existing = await readCaptureOwner(resolvedLockPath);
    const tokenRelation = existing.value.ownerToken === ownerToken ? "same-token" : "different-token";
    const liveness = processIsAlive(existing.value.pid) ? "live" : "dead";
    throw new Error(
      `manual capture owner already exists (${tokenRelation}, ${liveness}, pid ${existing.value.pid}); only the Global-mutex PowerShell wrapper may recover a stale lock`,
    );
  }

  try {
    await fs.rm(candidatePath);
    await syncDirectory(lockParent);
  } catch (candidateCleanupError) {
    throw new AggregateError(
      [candidateCleanupError],
      "manual capture owner was installed but its candidate could not be cleaned; leaving the fixed lock for wrapper recovery",
    );
  }
  return Object.freeze({ lockPath: resolvedLockPath, descriptor });
}

export async function releaseCaptureOwner(owner) {
  const lockPath = assertCaptureOwnerLockPath(owner?.lockPath ?? "");
  const current = await readCaptureOwner(lockPath);
  if (
    current.value.ownerToken !== owner?.descriptor?.ownerToken ||
    current.value.pid !== owner?.descriptor?.pid ||
    current.text !== `${JSON.stringify(owner.descriptor, null, 2)}\n`
  ) {
    throw new Error("manual capture owner changed before release; leaving the lock in place");
  }
  await fs.rm(lockPath);
  await syncDirectory(path.dirname(lockPath));
  if (await fileExists(lockPath)) throw new Error("manual capture owner lock still exists after release");
}

/** Emit success only after the durable cross-process owner is gone. */
export async function releaseCaptureOwnerThenComplete({
  owner,
  completion,
  releaseOwner = releaseCaptureOwner,
  write = (text) => process.stdout.write(text),
}) {
  if (typeof releaseOwner !== "function" || typeof write !== "function") {
    throw new Error("completion boundary requires releaseOwner and write functions");
  }
  await releaseOwner(owner);
  const line = `[manual-capture] COMPLETE ${completion.selectedCount}/${completion.totalCount} output=${completion.outDir} run=${completion.runDirectory}\n`;
  write(line);
  return line;
}

async function sourceHashes(manifestText) {
  return {
    manifest: sha256Bytes(Buffer.from(manifestText, "utf8")),
    runner: await sha256File(runnerPath),
    scenarios: await sha256File(scenarioRegistryPath),
    cdpClient: await sha256File(cdpClientPath),
    fixtureCrane: await sha256File(captureFixturePaths.crane),
    fixtureYakko: await sha256File(captureFixturePaths.yakko),
    fixtureBird: await sha256File(captureFixturePaths.bird),
    fixturePenetration: await sha256File(captureFixturePaths.penetration),
  };
}

async function collectCurrentInputs(appExe, appSha256) {
  const currentManifestText = await fs.readFile(manifestPath, "utf8");
  const appIdentity = await inspectApplicationIdentity(appExe, appSha256);
  return {
    sourceHashes: await sourceHashes(currentManifestText),
    appIdentity,
  };
}

function makeRunId() {
  const stamp = new Date().toISOString().replace(/[-:.TZ]/g, "");
  return `run-${stamp}-${process.pid}`;
}

async function createFreshRun({ entries, outDir, stagingRoot, hashes, appIdentity, inputFingerprint, ownerToken }) {
  await fs.mkdir(stagingRoot, { recursive: true });
  const runId = makeRunId();
  const runDirectory = path.join(stagingRoot, runId);
  await fs.mkdir(runDirectory, { recursive: false });
  await fs.mkdir(path.join(runDirectory, "screens"), { recursive: false });
  const now = new Date().toISOString();
  const state = {
    schemaVersion: RUN_STATE_SCHEMA,
    runId,
    status: "running",
    startedAt: now,
    updatedAt: now,
    outputDir: outDir,
    stagingDir: runDirectory,
    sourceHashes: hashes,
    appIdentity,
    inputFingerprint,
    ownerToken,
    selection: entries.map((entry) => entry.name),
    current: null,
    entries: Object.fromEntries(
      entries.map((entry) => [
        entry.name,
        {
          ordinal: entry.ordinal,
          scenario: entry.scenario,
          status: "pending",
          stagedPath: path.join("screens", entry.name),
        },
      ]),
    ),
    promotion: { status: "pending" },
  };
  await writeJsonAtomic(path.join(runDirectory, "run.json"), state);
  return { runDirectory, state };
}

function promotionEntriesFromResumeState(state) {
  if (!Array.isArray(state.selection) || state.selection.length === 0) {
    throw new Error("resume state has no ordered selection for promotion recovery");
  }
  if (new Set(state.selection).size !== state.selection.length) {
    throw new Error("resume promotion selection contains duplicate names");
  }
  const recoveryEntries = state.selection.map((name) => {
    if (typeof name !== "string" || !/^screen-[a-z0-9]+(?:-[a-z0-9]+)*\.png$/.test(name)) {
      throw new Error(`resume promotion selection has an invalid name: ${JSON.stringify(name)}`);
    }
    const record = state.entries?.[name];
    if (
      !record ||
      !Number.isInteger(record.ordinal) ||
      record.ordinal < 1 ||
      typeof record.scenario !== "string" ||
      record.scenario.length === 0
    ) {
      throw new Error(`resume promotion record is invalid: ${name}`);
    }
    return { ordinal: record.ordinal, name, scenario: record.scenario };
  });
  if (recoveryEntries.some((entry, index) => index > 0 && entry.ordinal <= recoveryEntries[index - 1].ordinal)) {
    throw new Error("resume promotion selection is not in strictly increasing ordinal order");
  }
  return recoveryEntries;
}

async function loadResumeRun({
  resume,
  stagingRoot,
  outDir,
  hashes,
  appIdentity,
  inputFingerprint,
  ownerToken,
  entries,
  from,
}) {
  const resolved = resolveCliPath(resume);
  let stat;
  try {
    stat = await fs.stat(resolved);
  } catch (error) {
    if (error?.code === "ENOENT" && path.basename(resolved) === "run.json" && (await fileExists(`${resolved}.previous`))) {
      stat = await fs.stat(`${resolved}.previous`);
    } else {
      throw new Error(`resume path does not exist: ${resolved}: ${errorText(error)}`);
    }
  }
  const explicitStatePath = path.basename(resolved) === "run.json";
  const runDirectory = !explicitStatePath && stat.isDirectory() ? resolved : path.dirname(resolved);
  const statePath = !explicitStatePath && stat.isDirectory() ? path.join(resolved, "run.json") : resolved;
  if (path.basename(statePath) !== "run.json") throw new Error("--resume must name a run directory or its run.json");
  if (!isPathWithin(stagingRoot, runDirectory) || !path.basename(runDirectory).startsWith("run-")) {
    throw new Error(`resume run must be a run-* child of the staging root: ${runDirectory}`);
  }
  const { value: state } = await readJson(statePath, "resume state");
  if (state?.schemaVersion !== RUN_STATE_SCHEMA) {
    throw new Error(`unsupported resume state schema: ${JSON.stringify(state?.schemaVersion)}`);
  }
  if (state.runId !== path.basename(runDirectory) || !/^run-\d+-\d+$/.test(state.runId)) {
    throw new Error("resume state runId is invalid or does not match its directory");
  }
  if (path.resolve(state.stagingDir ?? "") !== runDirectory) throw new Error("resume state stagingDir does not match its location");
  if (path.resolve(state.outputDir ?? "") !== outDir) {
    throw new Error("resume outputDir differs from the fixed docs/manual/assets destination");
  }
  if (state.ownerToken !== ownerToken) {
    throw new Error("resume owner token does not match the wrapper token recorded for this run");
  }
  // The durable promotion journal belongs to the old, recorded inputs.  If a
  // process died after changing even one final PNG, restore every old original
  // before deciding whether changed sources/app identity force a fresh run.
  const promotionEntries = promotionEntriesFromResumeState(state);
  const recoveryId = await recoverPromotionBeforeResume(promotionEntries, state, runDirectory, outDir);
  if (recoveryId) {
    await fs.appendFile(
      path.join(runDirectory, "progress.jsonl"),
      `${JSON.stringify({
        timestamp: new Date().toISOString(),
        type: "recovery",
        phase: "promotion-rollback",
        recovery: recoveryId,
        count: promotionEntries.length,
      })}\n`,
      "utf8",
    );
  }
  if (assertResumeFingerprint(state, hashes, appIdentity) !== inputFingerprint) {
    throw new Error("resume caller fingerprint is inconsistent with the verified inputs");
  }

  const selected = selectEntries(entries, { only: null, from }, state);
  const selectedNames = new Set(state.selection);
  for (const name of selectedNames) {
    const current = entries.find((entry) => entry.name === name);
    const record = state.entries?.[name];
    if (!current || !record || record.ordinal !== current.ordinal || record.scenario !== current.scenario) {
      throw new Error(`resume manifest record is inconsistent: ${name}`);
    }
    if (record.status === "passed") {
      const stagedPath = path.join(runDirectory, "screens", name);
      const actual = await inspectPngFile(stagedPath, `resume image ${name}`);
      if (record.bytes !== actual.bytes || record.sha256 !== actual.sha256) {
        throw new Error(`resume image receipt does not match bytes on disk: ${name}`);
      }
    }
  }

  const rerunNames = new Set(selected.map((entry) => entry.name));
  for (const entry of entries) {
    if (!rerunNames.has(entry.name)) continue;
    state.entries[entry.name] = {
      ordinal: entry.ordinal,
      scenario: entry.scenario,
      status: "pending",
      stagedPath: path.join("screens", entry.name),
    };
  }
  state.status = "running";
  state.current = null;
  delete state.error;
  state.updatedAt = new Date().toISOString();
  state.promotion = { status: "pending", ...(recoveryId ? { recoveredBy: recoveryId } : {}) };
  await writeJsonAtomic(statePath, state);
  return { runDirectory, state, selected };
}

function progressLine(event) {
  const prefix = `${event.entry.ordinal}/${MANUAL_SCREEN_COUNT} ${event.entry.name}`;
  if (event.type === "start") return `[manual-capture] START ${prefix} phase=scenario`;
  if (event.type === "pass") {
    return `[manual-capture] PASS ${prefix} bytes=${event.result.bytes} sha256=${event.result.sha256}`;
  }
  return `[manual-capture] FAIL ${prefix} phase=${event.phase ?? "scenario"} ${errorText(event.error)}`;
}

async function recordProgress(runDirectory, state, event) {
  const now = new Date().toISOString();
  const phase =
    event.phase ??
    (event.type !== "start" && state.current?.name === event.entry.name ? state.current.phase : null) ??
    "scenario";
  const normalizedEvent = { ...event, phase };
  const record = {
    timestamp: now,
    type: event.type,
    ordinal: event.entry.ordinal,
    total: MANUAL_SCREEN_COUNT,
    name: event.entry.name,
    scenario: event.entry.scenario,
    phase,
  };
  if (event.result) Object.assign(record, event.result);
  if (event.error) record.error = errorText(event.error);
  await fs.appendFile(path.join(runDirectory, "progress.jsonl"), `${JSON.stringify(record)}\n`, "utf8");

  const entryState = state.entries[event.entry.name];
  if (event.type === "start") {
    entryState.status = "running";
    entryState.startedAt = now;
    delete entryState.error;
    state.current = { ordinal: event.entry.ordinal, name: event.entry.name, phase: record.phase };
  } else if (event.type === "pass") {
    Object.assign(entryState, event.result, { status: "passed", completedAt: now });
    state.current = null;
  } else {
    Object.assign(entryState, { status: "failed", failedAt: now, phase: record.phase, error: record.error });
    state.status = "failed";
    state.current = { ordinal: event.entry.ordinal, name: event.entry.name, phase: record.phase };
  }
  state.updatedAt = now;
  await writeJsonAtomic(path.join(runDirectory, "run.json"), state);
  process.stdout.write(`${progressLine(normalizedEvent)}\n`);
}

function augmentClient(rawClient) {
  let expectedGeneration = null;
  const preloadScriptIdentifiers = new Set();

  async function getCaptureStatus() {
    const status = await rawClient.evaluate(`(() => {
      const api = window.__origami3Capture;
      if (!api || typeof api.getStatus !== "function") return null;
      return api.getStatus();
    })()`);
    if (!status || status.version !== 1 || status.ready !== true || typeof status.generation !== "string") {
      throw new Error("ORIGAMI3 capture API version 1 is not ready");
    }
    return status;
  }

  async function rebindGeneration() {
    await rawClient.waitForCaptureApi();
    const status = await getCaptureStatus();
    expectedGeneration = status.generation;
    return status;
  }

  async function assertGeneration() {
    const status = await getCaptureStatus();
    if (expectedGeneration === null) expectedGeneration = status.generation;
    if (status.generation !== expectedGeneration) {
      throw new Error(
        `capture generation changed unexpectedly: expected ${JSON.stringify(expectedGeneration)}, got ${JSON.stringify(status.generation)}`,
      );
    }
    return status;
  }

  function trackPreloadScript(identifier) {
    if (typeof identifier !== "string" || identifier.length === 0) {
      throw new Error("preload script identifier must be a non-empty string");
    }
    preloadScriptIdentifiers.add(identifier);
  }

  async function removeTrackedPreloadScript(identifier) {
    if (!preloadScriptIdentifiers.has(identifier)) return;
    await rawClient.call("Page.removeScriptToEvaluateOnNewDocument", { identifier });
    preloadScriptIdentifiers.delete(identifier);
  }

  async function clearTrackedPreloadScripts() {
    const errors = [];
    for (const identifier of [...preloadScriptIdentifiers]) {
      try {
        await removeTrackedPreloadScript(identifier);
      } catch (error) {
        errors.push(error);
      }
    }
    if (errors.length > 0) throw new AggregateError(errors, "tracked preload scripts could not be removed");
  }

  return Object.assign({}, rawClient, {
    send: rawClient.call,
    waitForStable: rawClient.stable,
    getCaptureStatus,
    rebindGeneration,
    assertGeneration,
    getExpectedGeneration: () => expectedGeneration,
    trackPreloadScript,
    removeTrackedPreloadScript,
    clearTrackedPreloadScripts,
  });
}

async function runCleanupStack(cleanups) {
  const errors = [];
  for (const cleanup of cleanups.reverse()) {
    try {
      await cleanup();
    } catch (error) {
      errors.push(error);
    }
  }
  if (errors.length > 0) throw new AggregateError(errors, "scenario cleanup failed");
}

async function executeScenario({ entry, client, runDirectory, state }) {
  const stagedPath = path.join(runDirectory, "screens", entry.name);
  let captureCount = 0;
  let captured = null;
  const cleanups = [];
  let phase = "scenario";

  const updatePhase = async (nextPhase) => {
    if (typeof nextPhase !== "string" || nextPhase.trim() === "") throw new Error("phase must be a non-empty string");
    phase = nextPhase;
    state.current = { ordinal: entry.ordinal, name: entry.name, phase };
    state.entries[entry.name].phase = phase;
    state.updatedAt = new Date().toISOString();
    await writeJsonAtomic(path.join(runDirectory, "run.json"), state);
  };

  const context = Object.freeze({
    entry: entry.raw,
    repositoryRoot,
    client,
    setPhase: updatePhase,
    deferCleanup(cleanup) {
      if (typeof cleanup !== "function") throw new Error("deferCleanup expects a function");
      cleanups.push(cleanup);
    },
    async capture() {
      captureCount += 1;
      if (captureCount !== 1) throw new Error(`scenario ${entry.scenario} called context.capture() more than once`);
      await updatePhase("capture");
      const temporary = `${stagedPath}.tmp-${process.pid}`;
      try {
        await client.screenshot(temporary);
        const receipt = await inspectPngFile(temporary, `${entry.name} screenshot`);
        await replaceFile(temporary, stagedPath);
        const stored = await inspectPngFile(stagedPath, `${entry.name} staged screenshot`);
        if (stored.sha256 !== receipt.sha256 || stored.bytes !== receipt.bytes) {
          throw new Error(`staged screenshot changed while storing ${entry.name}`);
        }
        captured = stored;
        return stored;
      } finally {
        await fs.rm(temporary, { force: true });
      }
    },
  });

  let scenarioError = null;
  try {
    await client.lockMetrics();
    await client.assertGeneration();
    await entry.run(context);
    if (captureCount !== 1 || captured === null) {
      throw new Error(`scenario ${entry.scenario} must call context.capture() exactly once; actual=${captureCount}`);
    }
  } catch (error) {
    scenarioError = error;
  }

  const cleanupErrors = [];
  try {
    await runCleanupStack(cleanups);
  } catch (error) {
    cleanupErrors.push(error);
  }
  for (const cleanup of [
    () => client.releaseMouse(1275, 858),
    () => client.lockMetrics(),
    () => client.assertGeneration(),
  ]) {
    try {
      await cleanup();
    } catch (error) {
      cleanupErrors.push(error);
    }
  }

  if (scenarioError && cleanupErrors.length > 0) {
    throw new AggregateError([scenarioError, ...cleanupErrors], `scenario failed during ${phase} and cleanup also failed`);
  }
  if (scenarioError) throw scenarioError;
  if (cleanupErrors.length > 0) throw new AggregateError(cleanupErrors, `scenario cleanup failed after ${phase}`);
  return captured;
}

async function retryCleanupAction(label, attempt, count = 3) {
  const errors = [];
  for (let number = 1; number <= count; number += 1) {
    try {
      return await attempt();
    } catch (error) {
      errors.push(error);
    }
  }
  throw new AggregateError(errors, `${label} failed after ${count} attempts`);
}

export async function cleanupClient(client) {
  const errors = [];
  const attempts = [
    ["release pressed pointer", () => client.releaseMouse(1275, 858)],
    ["remove delayed Viewer3D preload", () => client.clearTrackedPreloadScripts()],
    ["enable Network cleanup domain", () => client.call("Network.enable")],
    ["unblock Viewer3D chunks", () => client.call("Network.setBlockedURLs", { urls: [] })],
    ["clear potentially blocked cache", () => client.call("Network.clearBrowserCache")],
    ["restore cache", () => client.call("Network.setCacheDisabled", { cacheDisabled: false })],
    // Repeat the two stateful reversals immediately before reload.  If their
    // first pass failed transiently, the final reload must not run under the
    // delayed/blocked conditions.
    ["remove delayed Viewer3D preload before reload", () => client.clearTrackedPreloadScripts()],
    ["unblock Viewer3D chunks before reload", () => client.call("Network.setBlockedURLs", { urls: [] })],
    ["restore cache before reload", () => client.call("Network.setCacheDisabled", { cacheDisabled: false })],
    ["reload restored application", () => client.reload()],
    ["rebind restored capture generation", () => client.rebindGeneration()],
    ["restore normal application view", () => client.evaluate(`(async () => {
      const api = window.__origami3Capture;
      if (api && typeof api.setView === "function") await api.setView("normal");
      return true;
    })()`)],
    [
      "verify normal Viewer3D after cleanup",
      () =>
        client.waitFor(
          `document.querySelector('canvas.viewer3d-canvas[aria-label="3D表示"]') !== null &&
            document.querySelector('[data-testid="viewer3d-loading"], [data-testid="viewer3d-load-error"]') === null`,
          "normal Viewer3D after final cleanup",
          30_000,
        ),
    ],
    ["disable Network cleanup domain", () => client.call("Network.disable")],
    ["clear device metrics override", () => client.call("Emulation.clearDeviceMetricsOverride")],
  ];
  for (const [label, attempt] of attempts) {
    try {
      await retryCleanupAction(label, attempt);
    } catch (error) {
      errors.push(error);
    }
  }
  client.close();
  if (errors.length > 0) throw new AggregateError(errors, "final CDP cleanup failed");
}

async function validateStagedSelection(entries, state, runDirectory) {
  const receipts = new Map();
  for (const entry of entries) {
    const record = state.entries[entry.name];
    if (record?.status !== "passed") throw new Error(`cannot promote an entry that has not passed: ${entry.name}`);
    const stagedPath = path.join(runDirectory, "screens", entry.name);
    const receipt = await inspectPngFile(stagedPath, `staged image ${entry.name}`);
    if (receipt.bytes !== record.bytes || receipt.sha256 !== record.sha256) {
      throw new Error(`staged image differs from its run receipt: ${entry.name}`);
    }
    receipts.set(entry.name, { ...receipt, stagedPath });
  }
  return receipts;
}

async function fileReceipt(filePath) {
  const stat = await fs.stat(filePath);
  if (!stat.isFile()) throw new Error(`expected a regular file: ${filePath}`);
  return { bytes: stat.size, sha256: await sha256File(filePath) };
}

async function writeImmutableFile(filePath, bytes) {
  durableFileSequence += 1;
  const temporary = `${filePath}.prepare-${process.pid}-${durableFileSequence}`;
  await writeDurableExclusive(temporary, bytes);
  try {
    await linkThenUnlink(temporary, filePath);
  } catch (error) {
    if (error?.code !== "EEXIST") throw error;
    await fs.rm(temporary, { force: true });
  }
}

async function ensureImmutableJson(filePath, expected) {
  const serialized = `${JSON.stringify(expected, null, 2)}\n`;
  if (!(await fileExists(filePath))) await writeImmutableFile(filePath, serialized);
  const { value, text } = await readJson(filePath, `immutable promotion journal ${path.basename(filePath)}`);
  if (text !== serialized) {
    throw new Error(`immutable promotion journal changed: ${filePath}`);
  }
  return value;
}

function promotionPaths(runDirectory) {
  const root = path.join(runDirectory, "promotion-journal");
  return {
    root,
    transaction: path.join(root, "transaction.json"),
    originals: path.join(root, "originals"),
    attempts: path.join(root, "attempts"),
    recoveries: path.join(root, "recoveries"),
    backupsReady: path.join(root, "backups-ready.json"),
  };
}

function promotionDescriptor(entries, state, outDir) {
  return {
    schemaVersion: 1,
    runId: state.runId,
    inputFingerprint: state.inputFingerprint,
    selectionSha256: promotionSelectionSha256(entries),
    outputDir: outDir,
    entries: entries.map((entry) => ({ ordinal: entry.ordinal, name: entry.name })),
  };
}

export function promotionSelectionSha256(entries) {
  const selection = entries.map((entry) => ({ ordinal: entry.ordinal, name: entry.name }));
  return sha256Bytes(Buffer.from(JSON.stringify(selection), "utf8"));
}

function promotionLeasePath(outDir) {
  return path.join(outDir, ".origami3-manual-promotion-lease.json");
}

function promotionLeaseDescriptor(entries, state) {
  if (!/^run-\d+-\d+$/.test(state.runId ?? "")) throw new Error("promotion lease runId is invalid");
  if (!/^[a-f0-9]{64}$/.test(state.inputFingerprint ?? "")) {
    throw new Error("promotion lease input fingerprint is invalid");
  }
  return {
    schemaVersion: 1,
    runId: state.runId,
    inputFingerprint: state.inputFingerprint,
    selectionSha256: promotionSelectionSha256(entries),
  };
}

export async function acquirePromotionLease(entries, state, outDir) {
  await fs.mkdir(outDir, { recursive: true });
  const leasePath = promotionLeasePath(outDir);
  const descriptor = promotionLeaseDescriptor(entries, state);
  const serialized = `${JSON.stringify(descriptor, null, 2)}\n`;
  if (!(await fileExists(leasePath))) await writeImmutableFile(leasePath, serialized);
  const { text } = await readJson(leasePath, "manual screenshot promotion lease");
  if (text !== serialized) {
    throw new Error(
      `manual screenshot output is leased by a different run; resume that run before promotion: ${leasePath}`,
    );
  }
  return Object.freeze({ path: leasePath, descriptor: Object.freeze(descriptor), serialized });
}

export async function releasePromotionLease(lease, outDir) {
  const expectedPath = promotionLeasePath(outDir);
  if (!lease || lease.path !== expectedPath) throw new Error("promotion lease release target is inconsistent");
  const before = await fs.stat(expectedPath);
  const { text } = await readJson(expectedPath, "manual screenshot promotion lease before release");
  const after = await fs.stat(expectedPath);
  if (
    text !== lease.serialized ||
    before.dev !== after.dev ||
    before.ino !== after.ino ||
    before.size !== after.size ||
    before.mtimeMs !== after.mtimeMs
  ) {
    throw new Error("promotion lease changed while release was being verified");
  }
  await fs.rm(expectedPath);
  await syncDirectory(outDir);
  if (await fileExists(expectedPath)) throw new Error("promotion lease still exists after release");
}

function originalPaths(paths, entry) {
  const stem = `${String(entry.ordinal).padStart(2, "0")}-${entry.name}`;
  return {
    backup: path.join(paths.originals, `${stem}.original`),
    receipt: path.join(paths.originals, `${stem}.json`),
  };
}

async function ensurePromotionDirectories(paths) {
  await fs.mkdir(paths.root, { recursive: true });
  await fs.mkdir(paths.originals, { recursive: true });
  await fs.mkdir(paths.attempts, { recursive: true });
  await fs.mkdir(paths.recoveries, { recursive: true });
}

export async function prepareOriginalBackups(entries, state, runDirectory, outDir) {
  const paths = promotionPaths(runDirectory);
  await ensurePromotionDirectories(paths);
  const descriptor = promotionDescriptor(entries, state, outDir);
  await ensureImmutableJson(paths.transaction, descriptor);
  const backupsWereReady = await fileExists(paths.backupsReady);

  const receipts = new Map();
  for (const entry of entries) {
    const destination = path.join(outDir, entry.name);
    const original = originalPaths(paths, entry);
    let receipt;
    if (await fileExists(original.receipt)) {
      ({ value: receipt } = await readJson(original.receipt, `original receipt ${entry.name}`));
    } else if (await fileExists(original.backup)) {
      if (!(await fileExists(destination))) {
        throw new Error(`unreceipted original backup has no matching output: ${entry.name}`);
      }
      const backupReceipt = await fileReceipt(original.backup);
      const destinationReceipt = await fileReceipt(destination);
      if (
        backupReceipt.bytes !== destinationReceipt.bytes ||
        backupReceipt.sha256 !== destinationReceipt.sha256
      ) {
        throw new Error(`unreceipted original backup differs from untouched output: ${entry.name}`);
      }
      receipt = { exists: true, ...backupReceipt };
      await ensureImmutableJson(original.receipt, receipt);
    } else if (await fileExists(destination)) {
      durableFileSequence += 1;
      const temporary = `${original.backup}.prepare-${process.pid}-${durableFileSequence}`;
      await copyDurableExclusive(destination, temporary);
      await linkThenUnlink(temporary, original.backup);
      receipt = { exists: true, ...(await fileReceipt(original.backup)) };
      const stillCurrent = await fileReceipt(destination);
      if (stillCurrent.bytes !== receipt.bytes || stillCurrent.sha256 !== receipt.sha256) {
        throw new Error(`output changed while its immutable original was being backed up: ${entry.name}`);
      }
      await ensureImmutableJson(original.receipt, receipt);
    } else {
      receipt = { exists: false };
      await ensureImmutableJson(original.receipt, receipt);
    }

    if (receipt?.exists === true) {
      if (!(await fileExists(original.backup))) throw new Error(`original backup is missing: ${entry.name}`);
      const actual = await fileReceipt(original.backup);
      if (actual.bytes !== receipt.bytes || actual.sha256 !== receipt.sha256) {
        throw new Error(`immutable original backup changed: ${entry.name}`);
      }
      if (!backupsWereReady) {
        if (!(await fileExists(destination))) {
          throw new Error(`output disappeared before all original backups became durable: ${entry.name}`);
        }
        const stillCurrent = await fileReceipt(destination);
        if (stillCurrent.bytes !== receipt.bytes || stillCurrent.sha256 !== receipt.sha256) {
          throw new Error(`output changed before all original backups became durable: ${entry.name}`);
        }
      }
    } else if (receipt?.exists !== false || (await fileExists(original.backup))) {
      throw new Error(`original absence receipt is inconsistent: ${entry.name}`);
    } else if (!backupsWereReady && (await fileExists(destination))) {
      throw new Error(`previously absent output appeared before all original backups became durable: ${entry.name}`);
    }
    receipts.set(entry.name, { ...receipt, backupPath: original.backup });
  }

  const descriptorSha256 = sha256Bytes(Buffer.from(JSON.stringify(descriptor), "utf8"));
  await ensureImmutableJson(paths.backupsReady, {
    schemaVersion: 1,
    transactionSha256: descriptorSha256,
    count: entries.length,
  });
  return { paths, descriptor, receipts };
}

async function numberedDirectories(parent, prefix) {
  if (!(await fileExists(parent))) return [];
  const names = await fs.readdir(parent, { withFileTypes: true });
  return names
    .filter((item) => item.isDirectory() && new RegExp(`^${prefix}-\\d{6}$`).test(item.name))
    .map((item) => item.name)
    .sort();
}

async function createNumberedDirectory(parent, prefix) {
  const existing = await numberedDirectories(parent, prefix);
  const last = existing.length === 0 ? 0 : Number.parseInt(existing.at(-1).slice(prefix.length + 1), 10);
  for (let number = last + 1; number <= last + 100; number += 1) {
    const id = `${prefix}-${String(number).padStart(6, "0")}`;
    const directory = path.join(parent, id);
    try {
      await fs.mkdir(directory, { recursive: false });
      await syncDirectory(parent);
      return { id, directory };
    } catch (error) {
      if (error?.code !== "EEXIST") throw error;
    }
  }
  throw new Error(`could not allocate a ${prefix} journal directory`);
}

function outputArtifact(outDir, state, operationId, entry, kind) {
  return path.join(outDir, `.${entry.name}.${state.runId}.${operationId}.${kind}`);
}

async function moveDestinationAside(destination, aside) {
  if (!(await fileExists(destination))) return false;
  if (await fileExists(aside)) throw new Error(`promotion artifact already exists: ${aside}`);
  await fs.link(destination, aside);
  await syncDirectory(path.dirname(destination));
  await fs.rm(destination);
  await syncDirectory(path.dirname(destination));
  return true;
}

async function installPromotionFile(source, destination, candidate, displaced) {
  await copyDurableExclusive(source, candidate);
  const sourceReceipt = await fileReceipt(source);
  const candidateReceipt = await fileReceipt(candidate);
  if (sourceReceipt.bytes !== candidateReceipt.bytes || sourceReceipt.sha256 !== candidateReceipt.sha256) {
    throw new Error(`promotion candidate differs from source: ${destination}`);
  }
  await moveDestinationAside(destination, displaced);
  await linkThenUnlink(candidate, destination);
  const installed = await fileReceipt(destination);
  if (installed.bytes !== sourceReceipt.bytes || installed.sha256 !== sourceReceipt.sha256) {
    throw new Error(`installed promotion file differs from source: ${destination}`);
  }
  return installed;
}

async function writeJournalEvent(directory, name, value) {
  await ensureImmutableJson(path.join(directory, name), value);
}

async function cleanupPromotionArtifacts(entries, state, outDir, paths) {
  const attemptIds = await numberedDirectories(paths.attempts, "attempt");
  const recoveryIds = await numberedDirectories(paths.recoveries, "recovery");
  for (const entry of entries) {
    for (const operationId of attemptIds) {
      await fs.rm(outputArtifact(outDir, state, operationId, entry, "candidate"), { force: true });
      await fs.rm(outputArtifact(outDir, state, operationId, entry, "displaced"), { force: true });
    }
    for (const operationId of recoveryIds) {
      await fs.rm(outputArtifact(outDir, state, operationId, entry, "restore"), { force: true });
      await fs.rm(outputArtifact(outDir, state, operationId, entry, "rollback-displaced"), { force: true });
    }
  }
  await syncDirectory(outDir);
}

async function rollbackOriginals(entries, state, runDirectory, outDir, journal) {
  const recovery = await createNumberedDirectory(journal.paths.recoveries, "recovery");
  await writeJournalEvent(recovery.directory, "started.json", {
    schemaVersion: 1,
    id: recovery.id,
    startedAt: new Date().toISOString(),
  });
  for (const entry of entries) {
    const receipt = journal.receipts.get(entry.name);
    if (!receipt) throw new Error(`original receipt is unavailable during rollback: ${entry.name}`);
    const destination = path.join(outDir, entry.name);
    const restore = outputArtifact(outDir, state, recovery.id, entry, "restore");
    const displaced = outputArtifact(outDir, state, recovery.id, entry, "rollback-displaced");
    await writeJournalEvent(recovery.directory, `${String(entry.ordinal).padStart(2, "0")}-started.json`, {
      ordinal: entry.ordinal,
      name: entry.name,
      originalExists: receipt.exists,
    });
    if (receipt.exists) {
      await copyDurableExclusive(receipt.backupPath, restore);
      await moveDestinationAside(destination, displaced);
      await linkThenUnlink(restore, destination);
      const actual = await fileReceipt(destination);
      if (actual.bytes !== receipt.bytes || actual.sha256 !== receipt.sha256) {
        throw new Error(`rollback did not restore the immutable original: ${entry.name}`);
      }
    } else {
      await moveDestinationAside(destination, displaced);
    }
    await writeJournalEvent(recovery.directory, `${String(entry.ordinal).padStart(2, "0")}-passed.json`, {
      ordinal: entry.ordinal,
      name: entry.name,
      originalExists: receipt.exists,
    });
  }
  await cleanupPromotionArtifacts(entries, state, outDir, journal.paths);
  await writeJournalEvent(recovery.directory, "completed.json", {
    schemaVersion: 1,
    id: recovery.id,
    completedAt: new Date().toISOString(),
    count: entries.length,
  });
  return recovery.id;
}

export function promotionRecoveryDecision({ transactionExists, backupsReady, attemptCount }) {
  if (!Number.isInteger(attemptCount) || attemptCount < 0) {
    throw new Error("promotion attempt count must be a non-negative integer");
  }
  if (!transactionExists) {
    if (backupsReady || attemptCount > 0) throw new Error("promotion journal children exist without a transaction");
    return "none";
  }
  if (!backupsReady) {
    if (attemptCount > 0) {
      throw new Error("promotion attempt exists before immutable originals were made durable");
    }
    return "none";
  }
  return attemptCount > 0 ? "rollback" : "none";
}

async function journalStepWasStarted(paths, entry) {
  const stepName = `${String(entry.ordinal).padStart(2, "0")}-started.json`;
  for (const [parent, prefix] of [
    [paths.attempts, "attempt"],
    [paths.recoveries, "recovery"],
  ]) {
    for (const operationId of await numberedDirectories(parent, prefix)) {
      if (await fileExists(path.join(parent, operationId, stepName))) return true;
    }
  }
  return false;
}

export async function preflightPromotionRollback(entries, state, runDirectory, outDir, journal) {
  // Validate all staged receipts and all current destinations before changing
  // even the first image.  This is a content CAS: after our lease was released,
  // a later run is allowed to own the output and must never be rolled back by
  // an older run whose final run.json write was interrupted.
  const stagedReceipts = await validateStagedSelection(entries, state, runDirectory);
  const mismatches = [];
  for (const entry of entries) {
    const original = journal.receipts.get(entry.name);
    const staged = stagedReceipts.get(entry.name);
    if (!original || !staged) throw new Error(`rollback preflight receipt is unavailable: ${entry.name}`);
    const destination = path.join(outDir, entry.name);
    if (!(await fileExists(destination))) {
      const absenceAllowed = original.exists === false || (await journalStepWasStarted(journal.paths, entry));
      if (!absenceAllowed) mismatches.push(`${entry.name}:absent`);
      continue;
    }
    const current = await fileReceipt(destination);
    const matchesOriginal =
      original.exists === true && current.bytes === original.bytes && current.sha256 === original.sha256;
    const matchesStaged = current.bytes === staged.bytes && current.sha256 === staged.sha256;
    if (!matchesOriginal && !matchesStaged) {
      mismatches.push(`${entry.name}:${current.sha256}`);
    }
  }
  if (mismatches.length > 0) {
    throw new Error(
      `promotion rollback CAS rejected output owned by a later/different run: ${mismatches.join(", ")}`,
    );
  }
}

export async function recoverPromotionBeforeResume(entries, state, runDirectory, outDir) {
  const paths = promotionPaths(runDirectory);
  const leaseExists = await fileExists(promotionLeasePath(outDir));
  const transactionExists = await fileExists(paths.transaction);
  const backupsReady = await fileExists(paths.backupsReady);
  const attempts = await numberedDirectories(paths.attempts, "attempt");
  const decision = promotionRecoveryDecision({
    transactionExists,
    backupsReady,
    attemptCount: attempts.length,
  });
  if (decision === "none") {
    if (leaseExists) {
      const unusedLease = await acquirePromotionLease(entries, state, outDir);
      await releasePromotionLease(unusedLease, outDir);
    }
    return null;
  }
  const lease = await acquirePromotionLease(entries, state, outDir);
  let rollbackStarted = false;
  try {
    const journal = await prepareOriginalBackups(entries, state, runDirectory, outDir);
    await preflightPromotionRollback(entries, state, runDirectory, outDir, journal);
    rollbackStarted = true;
    const recoveryId = await rollbackOriginals(entries, state, runDirectory, outDir, journal);
    await releasePromotionLease(lease, outDir);
    return recoveryId;
  } catch (recoveryError) {
    if (!rollbackStarted) {
      try {
        await releasePromotionLease(lease, outDir);
      } catch (releaseError) {
        throw new AggregateError(
          [recoveryError, releaseError],
          "promotion rollback preflight failed and its lease could not be released",
        );
      }
    }
    throw recoveryError;
  }
}

export async function promoteSelection({ entries, state, runDirectory, outDir, requireExactSet }) {
  const receipts = await validateStagedSelection(entries, state, runDirectory);
  await fs.mkdir(outDir, { recursive: true });
  const lease = await acquirePromotionLease(entries, state, outDir);
  let leaseHeld = true;
  let journal = null;
  let attempt = null;
  try {
    journal = await prepareOriginalBackups(entries, state, runDirectory, outDir);
    attempt = await createNumberedDirectory(journal.paths.attempts, "attempt");
    await writeJournalEvent(attempt.directory, "started.json", {
      schemaVersion: 1,
      id: attempt.id,
      startedAt: new Date().toISOString(),
      count: entries.length,
    });
    state.promotion = { status: "running", attempt: attempt.id, startedAt: new Date().toISOString() };
    state.updatedAt = new Date().toISOString();
    await writeJsonAtomic(path.join(runDirectory, "run.json"), state);

    for (const entry of entries) {
      const destination = path.join(outDir, entry.name);
      const candidate = outputArtifact(outDir, state, attempt.id, entry, "candidate");
      const displaced = outputArtifact(outDir, state, attempt.id, entry, "displaced");
      await writeJournalEvent(attempt.directory, `${String(entry.ordinal).padStart(2, "0")}-started.json`, {
        ordinal: entry.ordinal,
        name: entry.name,
      });
      const installed = await installPromotionFile(
        receipts.get(entry.name).stagedPath,
        destination,
        candidate,
        displaced,
      );
      await writeJournalEvent(attempt.directory, `${String(entry.ordinal).padStart(2, "0")}-passed.json`, {
        ordinal: entry.ordinal,
        name: entry.name,
        bytes: installed.bytes,
        sha256: installed.sha256,
      });
    }
    for (const entry of entries) {
      const finalReceipt = await inspectPngFile(path.join(outDir, entry.name), `promoted image ${entry.name}`);
      const stagedReceipt = receipts.get(entry.name);
      if (finalReceipt.bytes !== stagedReceipt.bytes || finalReceipt.sha256 !== stagedReceipt.sha256) {
        throw new Error(`promoted image differs from staging: ${entry.name}`);
      }
    }
    if (requireExactSet) {
      const actual = (await fs.readdir(outDir))
        .filter((name) => /^screen-[a-z0-9]+(?:-[a-z0-9]+)*\.png$/.test(name))
        .sort();
      const expected = entries.map((entry) => entry.name).sort();
      if (JSON.stringify(actual) !== JSON.stringify(expected)) {
        throw new Error(`final screen set differs from manifest: expected=${expected.length}, actual=${actual.length}`);
      }
    }
    await writeJournalEvent(attempt.directory, "committed.json", {
      schemaVersion: 1,
      id: attempt.id,
      committedAt: new Date().toISOString(),
      count: entries.length,
    });
    await cleanupPromotionArtifacts(entries, state, outDir, journal.paths);
    state.promotion = {
      status: "release-pending",
      attempt: attempt.id,
      committedAt: new Date().toISOString(),
      count: entries.length,
    };
    state.status = "running";
    state.current = { phase: "promotion-lease-release" };
    state.updatedAt = new Date().toISOString();
    await writeJsonAtomic(path.join(runDirectory, "run.json"), state);
    await releasePromotionLease(lease, outDir);
    leaseHeld = false;

    state.promotion = { status: "passed", completedAt: new Date().toISOString(), count: entries.length };
    state.status = "complete";
    state.current = null;
    state.updatedAt = new Date().toISOString();
    await writeJsonAtomic(path.join(runDirectory, "run.json"), state);
  } catch (promotionError) {
    const recoveryErrors = [];
    if (attempt) {
      try {
        await writeJournalEvent(attempt.directory, "failed.json", {
          schemaVersion: 1,
          id: attempt.id,
          failedAt: new Date().toISOString(),
          error: errorText(promotionError),
        });
      } catch {
        // The immutable started/step journals are sufficient for resume recovery.
      }
    }
    let rollbackSafe = attempt === null;
    if (leaseHeld && attempt && journal) {
      try {
        await rollbackOriginals(entries, state, runDirectory, outDir, journal);
        rollbackSafe = true;
      } catch (rollbackError) {
        recoveryErrors.push(rollbackError);
      }
    }
    if (leaseHeld && rollbackSafe) {
      try {
        await releasePromotionLease(lease, outDir);
        leaseHeld = false;
      } catch (releaseError) {
        recoveryErrors.push(releaseError);
      }
    }
    if (recoveryErrors.length > 0) {
      throw new AggregateError(
        [promotionError, ...recoveryErrors],
        "promotion failed and its durable rollback/lease release did not fully complete",
      );
    }
    throw promotionError;
  }
}

async function recordPromotionFailure({ state, runDirectory, error, phase = "promotion" }) {
  const now = new Date().toISOString();
  state.status = "failed";
  state.current = { phase };
  state.updatedAt = now;
  state.error = errorText(error);
  state.promotion = { status: "failed", phase, failedAt: now, error: state.error };
  await fs.appendFile(
    path.join(runDirectory, "progress.jsonl"),
    `${JSON.stringify({ timestamp: now, type: "fail", phase, error: state.error })}\n`,
    "utf8",
  );
  await writeJsonAtomic(path.join(runDirectory, "run.json"), state);
}

async function runMain(options) {
  const outDir = defaultOutputDirectory;
  const stagingRoot = resolveCliPath(options.stagingRoot);
  if (
    path.resolve(outDir) === path.resolve(stagingRoot) ||
    isPathWithin(outDir, stagingRoot) ||
    isPathWithin(stagingRoot, outDir)
  ) {
    throw new Error("staging root and final output directory must not contain one another");
  }

  const { value: rawManifest, text: manifestText } = await readJson(manifestPath, "manual screenshot manifest");
  const registry = await createScenarioRegistry({ repositoryRoot });
  const entries = validateManifest(rawManifest, registry);
  if (options.list) {
    for (const entry of entries) process.stdout.write(`${entry.ordinal}\t${entry.name}\t${entry.scenario}\n`);
    return;
  }

  const endpoint = validateEndpoint(options.endpoint);
  const appIdentity = await inspectApplicationIdentity(options.appExe, options.appSha256);
  const hashes = await sourceHashes(manifestText);
  const inputFingerprint = inputFingerprintSha256(hashes, appIdentity);
  let runDirectory;
  let state;
  let plan;
  if (options.resume) {
    ({ runDirectory, state, selected: plan } = await loadResumeRun({
      resume: options.resume,
      stagingRoot,
      outDir,
      hashes,
      appIdentity,
      inputFingerprint,
      ownerToken: options.ownerToken,
      entries,
      from: options.from,
    }));
  } else {
    plan = selectEntries(entries, options);
    ({ runDirectory, state } = await createFreshRun({
      entries: plan,
      outDir,
      stagingRoot,
      hashes,
      appIdentity,
      inputFingerprint,
      ownerToken: options.ownerToken,
    }));
  }

  process.stdout.write(`[manual-capture] run=${runDirectory}\n`);
  let captureError = null;
  let cleanupError = null;
  let client = null;
  if (plan.length > 0) {
    try {
      client = augmentClient(await connectManualCapture(endpoint));
      await client.rebindGeneration();
      await executePlan({
        entries: plan,
        executeEntry: (entry) => executeScenario({ entry, client, runDirectory, state }),
        onEvent: (event) => recordProgress(runDirectory, state, event),
      });
    } catch (error) {
      captureError = error;
    }
    if (client) {
      try {
        await cleanupClient(client);
      } catch (error) {
        cleanupError = error;
      }
    }
  } else {
    process.stdout.write("[manual-capture] no pending screenshots; validating and promoting the recorded staging set without CDP\n");
  }
  if (captureError || cleanupError) {
    const errors = [captureError, cleanupError].filter(Boolean);
    const failedAt = new Date().toISOString();
    state.status = "failed";
    state.updatedAt = failedAt;
    state.error = errors.map(errorText).join("\n");
    await fs.appendFile(
      path.join(runDirectory, "progress.jsonl"),
      `${JSON.stringify({
        timestamp: failedAt,
        type: "fatal",
        phase: state.current?.phase ?? (client ? "cleanup" : "connect"),
        current: state.current,
        error: state.error,
      })}\n`,
      "utf8",
    );
    await writeJsonAtomic(path.join(runDirectory, "run.json"), state);
    process.stderr.write(`[manual-capture] stopped; diagnostics=${runDirectory}\n`);
    process.stderr.write(
      `[manual-capture] resume: powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\capture-manual-screens.ps1 -Resume ${quotePowerShellLiteral(runDirectory)}\n`,
    );
    throw errors.length === 1 ? errors[0] : new AggregateError(errors, "capture and cleanup failed");
  }

  const selectedEntries = state.selection.map((name) => {
    const entry = entries.find((candidate) => candidate.name === name);
    if (!entry) throw new Error(`run selection disappeared from manifest: ${name}`);
    return entry;
  });
  let failurePhase = "final-input-verification";
  try {
    await verifyFinalInputFingerprint(state.inputFingerprint, () =>
      collectCurrentInputs(options.appExe, options.appSha256),
    );
    failurePhase = "promotion";
    await promoteSelection({
      entries: selectedEntries,
      state,
      runDirectory,
      outDir,
      requireExactSet: selectedEntries.length === MANUAL_SCREEN_COUNT,
    });
  } catch (error) {
    await recordPromotionFailure({ state, runDirectory, error, phase: failurePhase });
    process.stderr.write(`[manual-capture] promotion failed; diagnostics=${runDirectory}\n`);
    process.stderr.write(
      `[manual-capture] resume: powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\capture-manual-screens.ps1 -Resume ${quotePowerShellLiteral(runDirectory)}\n`,
    );
    throw error;
  }
  return {
    selectedCount: selectedEntries.length,
    totalCount: MANUAL_SCREEN_COUNT,
    outDir,
    runDirectory,
  };
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  if (options.list) {
    await runMain(options);
    return;
  }

  // Acquire before reading capture inputs or inspecting the application, and
  // hold until all capture, promotion, state writes, and cleanup have ended.
  // Thus a missing wrapper token or a competing owner is rejected before CDP.
  const owner = await acquireCaptureOwner(options.ownerToken);
  let completion = null;
  let runError = null;
  try {
    completion = await runMain(options);
  } catch (error) {
    runError = error;
  }
  if (runError) {
    try {
      await releaseCaptureOwner(owner);
    } catch (releaseError) {
      throw new AggregateError(
        [runError, releaseError],
        "manual capture failed and its owner lock could not be released",
      );
    }
    throw runError;
  }
  await releaseCaptureOwnerThenComplete({ owner, completion });
}

const isMain = Boolean(process.argv[1]) && path.resolve(process.argv[1]) === runnerPath;
if (isMain) {
  main().catch((error) => {
    process.stderr.write(`[manual-capture] ERROR ${errorText(error)}\n`);
    process.exitCode = 1;
  });
}
