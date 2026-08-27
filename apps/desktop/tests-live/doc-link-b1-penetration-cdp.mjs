// M2.T2-7.C03: self-intersection warning acceptance.
// `prepare` validates the offline contract; `verify` requires an explicit, hash-pinned intersection fixture.

import assert from "node:assert/strict";
import path from "node:path";
import {
  connectDesktop,
  evaluate,
  failed,
  passed,
  prepare,
  repositoryRoot,
  resolvePhase,
  restoreBlank,
  verifyRuntime,
} from "./doc-link-b1-cdp-support.mjs";

const id = "M2.T2-7.C03";
const phase = resolvePhase(id);

async function verify() {
  const runtime = verifyRuntime(id, "ORI3_B1_PENETRATION");
  const connection = await connectDesktop();
  try {
    const result = await evaluate(connection, `(${async function inspect(input) {
      const api = window.__origami3Capture;
      if (!api || api.version !== 1) throw new Error("Capture API version 1 is unavailable");
      await api.openDocument(input.fixturePath);
      await api.setView("normal");
      await api.waitForStable();
      const badge = [...document.querySelectorAll('[data-floating-ui="status-badge"]')];
      const guide = [...document.querySelectorAll('[data-floating-ui="suspect-hinge-guide"]')];
      const interaction = api.getInteractionState();
      return {
        badgeCount: badge.length,
        guideCount: guide.length,
        badgeClasses: badge.map((node) => node.className),
        badgeText: badge.map((node) => (node.textContent ?? "").replace(/\s+/gu, " ").trim()),
        warningCount: interaction.diagnosis.warningCount,
      };
    }})(${JSON.stringify(runtime)})`);
    assert.equal(result.badgeCount, 1, "intersection fixture must show exactly one warning badge");
    assert.equal(result.guideCount, 1, "intersection fixture must show exactly one suspect-hinge guide");
    assert.ok(result.warningCount >= 1, "capture API must report at least one warning");
    assert.ok(result.badgeClasses[0].split(/\s+/u).includes("status-badge"), "intersection warning badge must use the status-badge state");
    assert.ok(!result.badgeClasses[0].split(/\s+/u).includes("error"), "intersection warning badge must not use the operation-error state");
    assert.ok(result.badgeText[0].length > 0, "warning badge text must not be empty");
    const restored = await restoreBlank(connection);
    passed(id, { runtime, result, restored });
  } finally {
    connection.close();
  }
}

try {
  if (phase === "prepare") {
    prepare(id, [
      path.resolve(repositoryRoot, "apps/desktop/src/components/ViewerStatusOverlays.tsx"),
      path.resolve(repositoryRoot, "apps/desktop/src/captureApi.ts"),
    ], ["ORI3_B1_PENETRATION_FIXTURE", "ORI3_B1_PENETRATION_FIXTURE_SHA256"]);
  } else if (phase === "verify") {
    await verify();
  } else {
    process.stdout.write(`${id} PREPARE/VERIFY NOT EXECUTED\n`);
    process.stdout.write(`${JSON.stringify({ id, phases: ["prepare", "verify"], cdpConnected: false }, null, 2)}\n`);
  }
} catch (error) {
  failed(id, phase, error);
}
