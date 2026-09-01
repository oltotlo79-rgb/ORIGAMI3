import crypto from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const manifestPath = path.join(scriptDirectory, "manual-screenshot-manifest.json");
const assetRoot = path.join(repositoryRoot, "docs", "manual", "assets");

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
]);
const EXPECTED_NAMES = Object.freeze(EXPECTED_ENTRIES.map((entry) => entry.name));

const EXPECTED_SHA256 = "214d8fa36633d0e79d553a8a5c50993e01d67e9dfd01c2bce549d1c7179b29e8";
const EXISTING_HELP_NAMES = Object.freeze(EXPECTED_NAMES.slice(0, 38));
const assertions = [];

function assert(condition, message) {
  assertions.push(message);
  if (!condition) throw new Error(message);
}

async function existingHelpScreenNames() {
  const chapterRoot = path.join(repositoryRoot, "apps", "desktop", "src", "help", "chapters");
  const names = new Set();
  for (const entry of await fs.readdir(chapterRoot, { withFileTypes: true })) {
    if (!entry.isFile() || !entry.name.endsWith(".ts")) continue;
    const source = await fs.readFile(path.join(chapterRoot, entry.name), "utf8");
    for (const match of source.matchAll(/image:\s*["'](screen-[a-z0-9-]+\.png)["']/g)) {
      names.add(match[1]);
    }
  }
  return names;
}

const manifest = JSON.parse(await fs.readFile(manifestPath, "utf8"));
assert(Array.isArray(manifest), "manifest root must be an array");
assert(manifest.length === 42, `manifest must contain exactly 42 rows: ${manifest.length}`);

const names = manifest.map((entry, index) => {
  assert(entry !== null && typeof entry === "object", `row ${index + 1} must be an object`);
  assert(entry.ordinal === index + 1, `row ${index + 1} ordinal must be ${index + 1}`);
  assert(typeof entry.name === "string", `row ${index + 1} name must be a string`);
  assert(typeof entry.scenario === "string" && entry.scenario.length > 0, `row ${index + 1} scenario is required`);
  assert(/^screen-[a-z0-9]+(?:-[a-z0-9]+)*\.png$/.test(entry.name), `row ${index + 1} has an unsafe name: ${entry.name}`);
  assert(path.basename(entry.name) === entry.name, `row ${index + 1} name must not contain a path`);
  const target = path.resolve(assetRoot, entry.name);
  assert(path.dirname(target) === assetRoot, `row ${index + 1} escapes the asset root`);
  assert(entry.name === EXPECTED_NAMES[index], `row ${index + 1} differs from the audited order`);
  assert(
    entry.scenario === EXPECTED_ENTRIES[index].scenario,
    `row ${index + 1} scenario differs from the audited assignment`,
  );
  return entry.name;
});

assert(new Set(names.map((name) => name.toLowerCase())).size === 42, "manifest names must be unique ignoring case");
assert(new Set(manifest.map((entry) => entry.scenario)).size === 42, "scenario ids must be unique");
const digest = crypto.createHash("sha256").update(`${names.join("\n")}\n`, "utf8").digest("hex");
assert(digest === EXPECTED_SHA256, `ordered name digest changed: ${digest}`);

const helpNames = await existingHelpScreenNames();
const allowedHelpNameSets = [EXISTING_HELP_NAMES, EXPECTED_NAMES];
const matchingHelpNameSet = allowedHelpNameSets.find(
  (expected) =>
    helpNames.size === expected.length &&
    expected.every((name) => helpNames.has(name)) &&
    [...helpNames].every((name) => expected.includes(name)),
);
assert(
  matchingHelpNameSet !== undefined,
  `help must reference either the exact current 38-name set or the exact future 42-name set: ${helpNames.size}`,
);

process.stdout.write(
  `MANUAL_SCREEN_MANIFEST assertions=${assertions.length} rows=${manifest.length} unique=${new Set(names).size} sha256=${digest}\n`,
);
