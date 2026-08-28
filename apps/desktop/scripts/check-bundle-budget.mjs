import { readFile, readdir } from "node:fs/promises";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const DEFAULT_DIST_DIR = resolve(SCRIPT_DIR, "..", "dist");

// 仕様で固定された上限であり、実測をそのまま境界にはしていない。
// 変更前initial gzipは166359 bytes、余裕は83641 bytes (33.4564%)。
const INITIAL_GZIP_LIMIT = 250_000;
// 変更前最大rawは349725 bytes、余裕は150275 bytes (30.055%)。
const MAX_RAW_CHUNK_LIMIT = 500_000;

function manifestEntries(manifest) {
  if (manifest === null || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new TypeError("Vite manifest must be an object");
  }
  return Object.entries(manifest);
}

function manifestChunk(manifest, key) {
  const chunk = manifest[key];
  if (chunk === null || typeof chunk !== "object" || Array.isArray(chunk)) {
    throw new Error(`Manifest import does not resolve to a chunk: ${key}`);
  }
  if (typeof chunk.file !== "string" || chunk.file.length === 0) {
    throw new Error(`Manifest chunk has no output file: ${key}`);
  }
  if (chunk.imports !== undefined && !Array.isArray(chunk.imports)) {
    throw new Error(`Manifest chunk imports must be an array: ${key}`);
  }
  return chunk;
}

function staticEntryNodes(manifest) {
  const entries = manifestEntries(manifest)
    .filter(([, chunk]) => chunk?.isEntry === true)
    .map(([key]) => key);
  if (entries.length === 0) {
    throw new Error("Vite manifest has no isEntry chunk");
  }

  const visitedKeys = new Set();
  const visitedFiles = new Set();
  const nodes = [];

  function visit(key, parentPath) {
    if (visitedKeys.has(key)) return;
    const chunk = manifestChunk(manifest, key);
    visitedKeys.add(key);

    const path = [...parentPath, key];
    const normalizedFile = chunk.file.replaceAll("\\", "/");
    if (!visitedFiles.has(normalizedFile)) {
      visitedFiles.add(normalizedFile);
      nodes.push({ key, chunk, path });
    }

    // Only `imports` are eager. `dynamicImports` deliberately do not enter this DFS.
    for (const importedKey of chunk.imports ?? []) {
      if (typeof importedKey !== "string") {
        throw new Error(`Manifest chunk has a non-string import: ${key}`);
      }
      visit(importedKey, path);
    }
  }

  for (const entryKey of entries) visit(entryKey, []);
  return nodes;
}

/**
 * Return the chunks statically reachable from every Vite `isEntry` record.
 * Traversal follows `imports` only and returns each emitted file at most once.
 */
export function entryChunks(manifest) {
  return staticEntryNodes(manifest).map(({ chunk }) => chunk);
}

/** Return the exact byte size of gzip-compressing a string or byte buffer. */
export function gzipBytes(input) {
  if (typeof input === "string" || Buffer.isBuffer(input)) {
    return gzipSync(input).byteLength;
  }
  if (input instanceof ArrayBuffer) {
    return gzipSync(new Uint8Array(input)).byteLength;
  }
  if (ArrayBuffer.isView(input)) {
    return gzipSync(
      Buffer.from(input.buffer, input.byteOffset, input.byteLength),
    ).byteLength;
  }
  throw new TypeError("gzipBytes input must be a string, ArrayBuffer, or byte view");
}

function chunkLabels(key, chunk) {
  return [key, chunk.file, chunk.name, chunk.src]
    .filter((value) => typeof value === "string")
    .map((value) => value.replaceAll("\\", "/"));
}

function isThreeChunk(key, chunk) {
  return chunkLabels(key, chunk).some(
    (value) =>
      value.includes("/node_modules/three/") ||
      /(^|[/_.-])three(?:[/_.-]|$)/i.test(value),
  );
}

function isManualExportChunk(key, chunk) {
  return chunkLabels(key, chunk).some((value) => /manualExport(?:\.|[/_.-]|$)/i.test(value));
}

function isManualPreviewCandidate(value) {
  return /manual(?:[-_.]?(?:preview|export))/i.test(value);
}

function staticPaths(manifest, predicate) {
  const entries = manifestEntries(manifest)
    .filter(([, chunk]) => chunk?.isEntry === true)
    .map(([key]) => key);
  const paths = [];
  const renderedPaths = new Set();

  function visit(key, parentPath, ancestors) {
    if (ancestors.has(key)) return;
    const chunk = manifestChunk(manifest, key);
    const path = [...parentPath, key];
    if (predicate(key, chunk)) {
      const rendered = path.join("\u0000");
      if (!renderedPaths.has(rendered)) {
        renderedPaths.add(rendered);
        paths.push(path);
      }
    }

    const nextAncestors = new Set(ancestors);
    nextAncestors.add(key);
    for (const importedKey of chunk.imports ?? []) {
      if (typeof importedKey !== "string") {
        throw new Error(`Manifest chunk has a non-string import: ${key}`);
      }
      visit(importedKey, path, nextAncestors);
    }
  }

  for (const entryKey of entries) visit(entryKey, [], new Set());
  return paths;
}

function artifactPath(distDir, manifestFile) {
  const absolute = resolve(distDir, manifestFile);
  const fromDist = relative(distDir, absolute);
  if (
    fromDist === ".." ||
    fromDist.startsWith(`..${sep}`) ||
    isAbsolute(fromDist)
  ) {
    throw new Error(`Manifest output escapes dist directory: ${manifestFile}`);
  }
  return absolute;
}

async function javascriptFiles(directory) {
  const files = [];

  async function visit(current) {
    const entries = await readdir(current, { withFileTypes: true });
    entries.sort((left, right) => left.name.localeCompare(right.name, "en"));
    for (const entry of entries) {
      const absolute = resolve(current, entry.name);
      if (entry.isDirectory()) {
        await visit(absolute);
      } else if (entry.isFile() && entry.name.toLowerCase().endsWith(".js")) {
        files.push(absolute);
      }
    }
  }

  await visit(directory);
  return files;
}

function formatManifestPath(manifest, path) {
  return path
    .map((key) => {
      const chunk = manifestChunk(manifest, key);
      return `${key} (${chunk.file})`;
    })
    .join(" -> ");
}

function outputFunction(options) {
  if (options.log === false) return () => {};
  if (typeof options.log === "function") return options.log;
  if (options.logger && typeof options.logger.log === "function") {
    return options.logger.log.bind(options.logger);
  }
  return console.log;
}

function positiveByteLimit(value, name) {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new TypeError(`${name} must be a positive safe integer`);
  }
  return value;
}

/**
 * Measure and assert the production bundle budget.
 *
 * Options may provide `distDir`, `manifestPath`, an already parsed `manifest`,
 * byte limits for boundary tests, and either `log` or `logger` for output.
 */
export async function assertBundleBudget(options = {}) {
  if (options === null || typeof options !== "object" || Array.isArray(options)) {
    throw new TypeError("assertBundleBudget options must be an object");
  }

  const explicitManifestPath = options.manifestPath
    ? resolve(options.manifestPath)
    : undefined;
  const distDir = resolve(
    options.distDir ??
      (explicitManifestPath
        ? resolve(dirname(explicitManifestPath), "..")
        : DEFAULT_DIST_DIR),
  );
  const manifestPath =
    explicitManifestPath ?? resolve(distDir, ".vite", "manifest.json");
  const initialGzipLimit = positiveByteLimit(
    options.initialGzipLimit ?? INITIAL_GZIP_LIMIT,
    "initialGzipLimit",
  );
  const maxRawChunkLimit = positiveByteLimit(
    options.maxRawChunkLimit ?? MAX_RAW_CHUNK_LIMIT,
    "maxRawChunkLimit",
  );
  const log = outputFunction(options);
  const manifest =
    options.manifest ?? JSON.parse(await readFile(manifestPath, "utf8"));

  const initialNodes = staticEntryNodes(manifest).filter(({ chunk }) =>
    chunk.file.toLowerCase().endsWith(".js"),
  );
  const initialMeasurements = [];
  for (const node of initialNodes) {
    const bytes = await readFile(artifactPath(distDir, node.chunk.file));
    initialMeasurements.push({
      file: node.chunk.file.replaceAll("\\", "/"),
      rawBytes: bytes.byteLength,
      gzipBytes: gzipBytes(bytes),
      path: node.path,
    });
  }

  const initialGzipBytes = initialMeasurements.reduce(
    (total, chunk) => total + chunk.gzipBytes,
    0,
  );
  const allJavascript = await javascriptFiles(distDir);
  const allChunks = [];
  for (const absolute of allJavascript) {
    const bytes = await readFile(absolute);
    allChunks.push({
      file: relative(distDir, absolute).split(sep).join("/"),
      rawBytes: bytes.byteLength,
    });
  }
  if (allChunks.length === 0) {
    throw new Error(`No JavaScript chunks found under ${distDir}`);
  }
  const largestRawChunk = allChunks.reduce((largest, chunk) =>
    chunk.rawBytes > largest.rawBytes ? chunk : largest,
  );

  const threeStaticImportPaths = staticPaths(manifest, isThreeChunk);
  const manualExportStaticImportPaths = staticPaths(
    manifest,
    isManualExportChunk,
  );
  const manualPreviewCandidates = new Set(
    allChunks
      .filter(({ file }) => isManualPreviewCandidate(file))
      .map(({ file }) => file),
  );
  for (const [key, chunk] of manifestEntries(manifest)) {
    if (
      chunk !== null &&
      typeof chunk === "object" &&
      !Array.isArray(chunk) &&
      chunkLabels(key, chunk).some(isManualPreviewCandidate)
    ) {
      manualPreviewCandidates.add(chunk.file ?? key);
    }
  }
  const sortedManualPreviewCandidates = [...manualPreviewCandidates].sort();

  log(`manifest: ${manifestPath}`);
  log(`dist: ${distDir}`);
  log(`initial JS chunk count: ${initialMeasurements.length}`);
  for (const chunk of initialMeasurements) {
    log(
      `initial path: ${formatManifestPath(manifest, chunk.path)} | ` +
        `raw ${chunk.rawBytes} bytes | gzip ${chunk.gzipBytes} bytes`,
    );
  }
  log(`initial gzip bytes: ${initialGzipBytes}`);
  log(`all JS chunk count: ${allChunks.length}`);
  log(
    `largest raw JS chunk: ${largestRawChunk.rawBytes} bytes (${largestRawChunk.file})`,
  );
  log(`entry -> Three static import paths: ${threeStaticImportPaths.length}`);
  for (const path of threeStaticImportPaths) {
    log(`Three path: ${formatManifestPath(manifest, path)}`);
  }
  log(
    `entry -> manualExport static import paths: ${manualExportStaticImportPaths.length}`,
  );
  for (const path of manualExportStaticImportPaths) {
    log(`manualExport path: ${formatManifestPath(manifest, path)}`);
  }
  log(`manual-preview chunk candidates: ${sortedManualPreviewCandidates.length}`);
  for (const candidate of sortedManualPreviewCandidates) {
    log(`manual-preview candidate: ${candidate}`);
  }

  const result = {
    manifestPath,
    distDir,
    initialGzipBytes,
    initialGzipLimit,
    initialChunkCount: initialMeasurements.length,
    initialChunks: initialMeasurements,
    chunkCount: allChunks.length,
    allChunks,
    largestRawBytes: largestRawChunk.rawBytes,
    maxRawChunkBytes: largestRawChunk.rawBytes,
    maxRawChunkLimit,
    largestRawChunk,
    threeStaticImportPaths,
    manualExportStaticImportPaths,
    manualPreviewCandidates: sortedManualPreviewCandidates,
  };
  const violations = [];
  if (initialGzipBytes > initialGzipLimit) {
    violations.push(
      `initial gzip ${initialGzipBytes} bytes exceeds ${initialGzipLimit} bytes`,
    );
  }
  if (largestRawChunk.rawBytes > maxRawChunkLimit) {
    violations.push(
      `largest raw chunk ${largestRawChunk.file} is ${largestRawChunk.rawBytes} bytes, ` +
        `exceeding ${maxRawChunkLimit} bytes`,
    );
  }
  if (threeStaticImportPaths.length > 0) {
    violations.push(
      `entry reaches Three through ${threeStaticImportPaths.length} static import path(s)`,
    );
  }
  if (manualExportStaticImportPaths.length > 0) {
    violations.push(
      `entry reaches manualExport through ${manualExportStaticImportPaths.length} static import path(s)`,
    );
  }
  if (sortedManualPreviewCandidates.length > 0) {
    violations.push(
      `found ${sortedManualPreviewCandidates.length} manual-preview chunk candidate(s)`,
    );
  }

  if (violations.length > 0) {
    const error = new Error(`Bundle budget failed:\n- ${violations.join("\n- ")}`);
    error.result = result;
    throw error;
  }

  log("bundle budget: PASS");
  return result;
}

function isDirectRun() {
  if (!process.argv[1]) return false;
  const modulePath = resolve(fileURLToPath(import.meta.url));
  const invokedPath = resolve(process.argv[1]);
  return process.platform === "win32"
    ? modulePath.toLowerCase() === invokedPath.toLowerCase()
    : modulePath === invokedPath;
}

if (isDirectRun()) {
  assertBundleBudget().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
