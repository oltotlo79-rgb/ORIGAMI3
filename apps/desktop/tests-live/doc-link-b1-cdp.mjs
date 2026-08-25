// B1のCDP受入検査。
//
// 起動済みのdesktop.exeだけへ接続する。browser / Playwright / desktop.exeの起動は行わない。
// 実行には、専用枠で次を明示すること:
//
//   $env:ORI3_B1_CDP_RUN = "1"
//   $env:ORI3_DESKTOP_PID = "<desktop.exe PID>"
//   $env:ORI3_DESKTOP_EXE = "<desktop.exe full path>"
//   $env:ORI3_B1_RESTORE_DOCUMENT = "<終了後に開き直す .ori3 full path>"
//   $env:ORI3_B1_RESTORE_STEP = "0" # 任意。省略時は0
//   node apps/desktop/tests-live/doc-link-b1-cdp.mjs
//
// この検査は既存の利用者の未保存状態を複製できない。専用枠の保存済み作品だけを
// RESTORE_DOCUMENTへ指定する。開始時に回復画面があれば「復元する」だけを押す。
// 「破棄する」は、このファイルにイベント送信を含めない。

import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const EXECUTE = process.env.ORI3_B1_CDP_RUN === "1";
const cdpPort = Number.parseInt(process.env.ORI3_CDP_PORT ?? "9222", 10);
const desktopPid = Number(process.env.ORI3_DESKTOP_PID);
const claimedExecutablePath = process.env.ORI3_DESKTOP_EXE
  ? path.resolve(process.env.ORI3_DESKTOP_EXE)
  : null;
const expectedExecutableSha256 = (
  process.env.ORI3_DESKTOP_SHA256 ??
  "127CCB0C03FBC9A1B43F371FE015E2088D637E2EFD66F0C22AABA0C25DDB864B"
).toUpperCase();
const restoreDocument = process.env.ORI3_B1_RESTORE_DOCUMENT
  ? path.resolve(process.env.ORI3_B1_RESTORE_DOCUMENT)
  : null;
const restoreStep = Number.parseInt(process.env.ORI3_B1_RESTORE_STEP ?? "0", 10);
const repositoryRoot = fileURLToPath(new URL("../../../", import.meta.url));

const fixtures = {
  crane: {
    path: path.resolve(repositoryRoot, "crates/ori3-rigid/tests/fixtures/check-crane.ori3"),
    sha256: "D44565B8CF3FF46AAD03905709CF891DA6627D235BD1CCE02F1F8EF8E67CF818",
  },
  yakko: {
    path: path.resolve(repositoryRoot, "crates/ori3-rigid/tests/fixtures/check-yakko.ori3"),
    sha256: "B9C3E2AF16A6382B47AA965100278C4FD50EF648DF5759E60C7C43E8BDEF2B26",
  },
  birdBase: {
    path: path.resolve(repositoryRoot, "crates/ori3-rigid/tests/fixtures/check-bird-base.ori3"),
    sha256: "29A9B7807AFE2EB4D43719C11B168005BB7E855B8A61E9C63A7B87EA12E6E889",
  },
};

// 画素・浮動小数の境目は未測定のまま書かない。下の10件が必要とする3回測定値と
// `floor(0.8 * min(n1, n2, n3))` は、専用fixtureと読取り口がそろってから別途固定する。
// solve時間はCIと手元で3.6倍異なるため、ここでも合否に使わない。
const blockedCases = [
  {
    id: "M2.T2-6b.C05",
    reason:
      "引く操作の安定したpick座標、cursor/readbackの3回基準値、手順数を読む公開口が無い。",
    required: [
      "apps/desktop/tests-live/fixtures/（引く操作用の追跡済みfixture）",
      "apps/desktop/src/captureApi.ts（手順数・cursor又は読取り専用値）",
      "apps/desktop/src/components/ToolRail.tsx（恒久selector）",
    ],
  },
  {
    id: "M2.T2-6c.C01",
    reason:
      "多層表示の追跡済みfixtureと、層ごとの可視画素3回基準値が無い。",
    required: [
      "apps/desktop/tests-live/fixtures/（多層fixture）",
      "apps/desktop/src/captureApi.ts（層ごとの読取り値、又は固定readback仕様）",
    ],
  },
  {
    id: "M2.T2-6c.C02",
    reason:
      "通常/Shiftで選ばれた層数をCDPから読めず、安定した最前面pick座標も無い。",
    required: [
      "apps/desktop/tests-live/fixtures/（最前面フラップfixture）",
      "apps/desktop/src/captureApi.ts（選択層数の読取り専用値）",
    ],
  },
  {
    id: "M2.T2-6c.C03",
    reason:
      "プレビュー・動く層・折線の別々のreadbackと、3回測定した画素基準値が無い。",
    required: [
      "apps/desktop/src/captureApi.ts（プレビュー読取り専用値）",
      "apps/desktop/tests-live/fixtures/（プレビュー固定fixture）",
    ],
  },
  {
    id: "M2.T2-6c.C05",
    reason:
      "段折りを完了させる線・層の安定座標と、手順数を読む公開口が無い。",
    required: [
      "apps/desktop/tests-live/fixtures/（段折り操作fixture）",
      "apps/desktop/src/captureApi.ts（手順数の読取り専用値）",
      "apps/desktop/src/components/Timeline.tsx（恒久selector）",
    ],
  },
  {
    id: "M2.T2-7.C01",
    reason:
      "4種類の作図menuとselectは検査できるが、各々がCPへ線を追加する安定した点・線座標が無い。",
    required: ["apps/desktop/tests-live/fixtures/（作図用CP fixture）"],
  },
  {
    id: "M2.T2-7.C02",
    reason:
      "前川・川崎違反を含む追跡済みfixtureと、橙画素の3回測定基準値が無い。",
    required: ["apps/desktop/tests-live/fixtures/（違反頂点fixture）"],
  },
  {
    id: "M2.T2-7.C03",
    reason:
      "面交差fixtureと、badge/guideを出す状態を作る公開手段、赤画素の3回測定基準値が無い。",
    required: ["apps/desktop/tests-live/fixtures/（面交差fixture）"],
  },
  {
    id: "M2.T2-8.C02",
    reason:
      "回復画面はアプリ起動前のautosaveでのみ作られる。tests-liveから既存アプリを終了・再起動できず、autosave作成口も無い。",
    required: [
      "apps/desktop/src-tauri/src/autosave.rs（専用fixture作成又はテスト起動契約）",
      "利用者が確保する起動前の専用実機枠",
    ],
  },
  {
    id: "M4.T4-5.C03",
    reason:
      "PDF/SVGの選択までは検査できるが、native保存先dialogをCDPだけで安全に指定できず、出力ファイル数・sizeを確認できない。",
    required: [
      "apps/desktop/src-tauri/src/commands.rs（CDP専用の安全な出力先、又はexport結果読取り口）",
      "apps/desktop/src/captureApi.ts（出力結果の読取り専用値）",
    ],
  },
];

class CdpConnection {
  constructor(socket) {
    this.socket = socket;
    this.nextId = 1;
    this.pending = new Map();
    socket.addEventListener("message", (event) => void this.onMessage(event));
    socket.addEventListener("close", (event) => {
      for (const { reject } of this.pending.values()) {
        reject(new Error(`CDP接続が応答前に閉じました: code=${event.code}`));
      }
      this.pending.clear();
    });
  }

  static async connect(url) {
    const socket = new WebSocket(url);
    await new Promise((resolve, reject) => {
      socket.addEventListener("open", resolve, { once: true });
      socket.addEventListener("error", () => reject(new Error(`CDP WebSocketへ接続できません: ${url}`)), {
        once: true,
      });
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
  const reply = await connection.send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
  if (reply.exceptionDetails) {
    throw new Error(
      reply.exceptionDetails.exception?.description ?? reply.exceptionDetails.text ?? "Runtime.evaluateに失敗しました",
    );
  }
  return reply.result.value;
}

function sha256(filePath) {
  return createHash("sha256").update(readFileSync(filePath)).digest("hex").toUpperCase();
}

function verifyFile(file, label) {
  assert.ok(statSync(file.path).isFile(), `${label} fixtureがありません: ${file.path}`);
  assert.equal(sha256(file.path), file.sha256, `${label} fixtureのSHA-256が違います`);
}

function verifyExecutionContract() {
  if (!EXECUTE) {
    process.stdout.write(
      `${JSON.stringify(
        {
          version: 1,
          executed: false,
          instruction: "ORI3_B1_CDP_RUN=1とPID・EXE・RESTORE_DOCUMENTを指定した専用枠でだけ実行します",
          runnableIds: ["M2.T2-6b.C06", "M2.T2-6c.C04", "M3.T3-4.C01", "M3.T3-4.C02", "M4.T4-3.C02"],
          blocked: blockedCases,
        },
        null,
        2,
      )}\n`,
    );
    return false;
  }
  assert.ok(Number.isSafeInteger(desktopPid) && desktopPid > 0, "ORI3_DESKTOP_PIDを指定してください");
  assert.ok(claimedExecutablePath, "ORI3_DESKTOP_EXEを指定してください");
  assert.ok(restoreDocument, "ORI3_B1_RESTORE_DOCUMENTを指定してください");
  assert.ok(Number.isSafeInteger(restoreStep) && restoreStep >= 0, "RESTORE_STEPは0以上の整数です");
  const activeExecutablePath = path.resolve(
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
  assert.equal(
    activeExecutablePath.toLowerCase(),
    claimedExecutablePath.toLowerCase(),
    "PIDの実行ファイルが指定したdesktop.exeと一致しません",
  );
  assert.equal(sha256(activeExecutablePath), expectedExecutableSha256, "desktop.exeのSHA-256が一致しません");
  assert.ok(statSync(restoreDocument).isFile(), `復元先がありません: ${restoreDocument}`);
  for (const [name, fixture] of Object.entries(fixtures)) verifyFile(fixture, name);
  return true;
}

async function restoreUi(connection, originalUi) {
  return evaluate(
    connection,
    `(${async function restore(input) {
      const api = window.__origami3Capture;
      if (!api) throw new Error("__origami3Captureがありません");
      const frames = async (count = 3) => {
        for (let index = 0; index < count; index++) {
          await new Promise((resolve) => requestAnimationFrame(resolve));
        }
      };
      const text = (element) => (element?.textContent ?? "").replace(/\\s+/gu, " ").trim();
      const exactButton = (root, label) =>
        [...root.querySelectorAll("button")].find((button) => text(button) === label) ?? null;
      const close = exactButton(document, "閉じる");
      if (close) close.click();
      await frames();
      await api.openDocument(input.restoreDocument);
      const info = api.getDocumentInfo();
      if (input.restoreStep > info.stepCount) {
        throw new Error(`復元step ${input.restoreStep} が作品の手順数 ${info.stepCount} を超えます`);
      }
      await api.goToStep(input.restoreStep);
      if (input.activeTool) {
        const rail = document.querySelector("nav.tool-rail");
        const tool = rail && exactButton(rail, input.activeTool);
        if (!tool) throw new Error(`元のtoolを戻せません: ${input.activeTool}`);
        tool.click();
      }
      await api.setView("normal");
      await api.waitForStable();
      return {
        activeTool: text(document.querySelector("nav.tool-rail .tool-button.active")),
        dialogs: document.querySelectorAll('[data-floating-ui$="dialog"]').length,
        captureView: document.documentElement.getAttribute("data-origami3-capture-view"),
      };
    }})(${JSON.stringify({
      restoreDocument,
      restoreStep,
      activeTool: originalUi.activeTool,
    })})`,
  );
}

async function runInWebView(connection, input) {
  return evaluate(
    connection,
    `(${async function runB1(input) {
      const api = window.__origami3Capture;
      if (!api || api.version !== 1) throw new Error("version 1の__origami3Captureがありません");
      const pause = async (frames = 3) => {
        for (let index = 0; index < frames; index++) {
          await new Promise((resolve) => requestAnimationFrame(resolve));
        }
      };
      // timeoutは壊れたUIを待ち続けないための実行器保護で、solve時間を合否には使わない。
      const waitFor = async (read, label) => {
        for (let frame = 0; frame < 600; frame++) {
          const value = read();
          if (value) return value;
          await new Promise((resolve) => requestAnimationFrame(resolve));
        }
        throw new Error(`${label}が描画されません`);
      };
      const text = (element) => (element?.textContent ?? "").replace(/\\s+/gu, " ").trim();
      const exactButton = (root, label) =>
        [...root.querySelectorAll("button")].find((button) => text(button) === label) ?? null;
      const mustButton = (root, label) => {
        const button = exactButton(root, label);
        if (!button) throw new Error(`buttonがありません: ${label}`);
        return button;
      };
      const clickTool = async (label) => {
        const rail = document.querySelector("nav.tool-rail");
        if (!rail) throw new Error("tool-railがありません");
        mustButton(rail, label).click();
        await pause();
      };
      const openFixture = async (fixturePath) => {
        await api.openDocument(fixturePath);
        await api.setView("normal");
        await api.waitForStable();
      };
      const results = [];

      // M2.T2-6b.C06: 9種の定数順はUIから直接読む。値は離散的な製品契約である。
      await openFixture(input.fixtures.yakko);
      await clickTool("技法");
      const techniqueMenu = await waitFor(
        () => document.querySelector('[role="group"][aria-label="技法を選ぶ"]'),
        "技法menu",
      );
      const techniqueNames = [...techniqueMenu.querySelectorAll("button[aria-label]")].map((button) =>
        button.getAttribute("aria-label"),
      );
      const expectedTechniques = [
        "層操作",
        "段折り",
        "中割り折り",
        "かぶせ折り",
        "開いてつぶす",
        "花弁折り",
        "沈め折り",
        "ひだ寄せ",
        "ねじり折り",
      ];
      if (JSON.stringify(techniqueNames) !== JSON.stringify(expectedTechniques)) {
        throw new Error(`技法9種が一致しません: ${JSON.stringify(techniqueNames)}`);
      }
      results.push({ id: "M2.T2-6b.C06", passed: true, techniqueNames });

      // M2.T2-6c.C04: normalと途中stepでヒントが1件ずつあり、状態文が変わる。
      await openFixture(input.fixtures.crane);
      const craneInfo = api.getDocumentInfo();
      if (craneInfo.stepCount < 2) throw new Error("crane fixtureに途中stepがありません");
      await clickTool("折る");
      const readHint = () => {
        const hints = [...document.querySelectorAll('aside[data-floating-ui="viewer-operation-hint"]')];
        if (hints.length !== 1) throw new Error(`操作ヒント数=${hints.length}`);
        const status = hints[0].querySelector('p[role="status"]');
        if (!status) throw new Error("操作ヒントのstatusがありません");
        const message = text(status);
        if (!message) throw new Error("操作ヒントの文が空です");
        // 修飾キー名 Shift / Alt / Ctrl は既存の利用者向けDOM検査が要求する表示である。
        // それ以外のASCII英字語は、利用者向け状態文へ出さない。日本語の直後・直前に
        // 連結した語も検出するため、\b は使用しない。
        const latinWords = message.match(/[A-Za-z]{2,}/gu) ?? [];
        const unexpectedLatinWords = latinWords.filter(
          (word) => !["Shift", "Alt", "Ctrl"].includes(word),
        );
        if (unexpectedLatinWords.length !== 0) {
          throw new Error(`利用者向けヒントに許可されない英字語があります: ${unexpectedLatinWords.join(",")}`);
        }
        return message;
      };
      const normalHint = readHint();
      await api.goToStep(craneInfo.stepCount - 1);
      await api.waitForStable();
      const blockedHint = readHint();
      if (normalHint === blockedHint) throw new Error("通常時と途中step時の操作ヒントが同じです");
      results.push({ id: "M2.T2-6c.C04", passed: true, normalHint, blockedHint });

      // M3.T3-4.C01/C02: 提案の3画面、4候補、適用後closeと常設4区画を通す。
      await openFixture(input.fixtures.yakko);
      const fourPanes = () => ({
        toolRail: document.querySelectorAll("nav.tool-rail").length,
        cp: document.querySelectorAll("section.pane-2d").length,
        viewer: document.querySelectorAll("section.pane-3d").length,
        context: document.querySelectorAll("#context-panel").length,
      });
      const beforePanes = fourPanes();
      if (Object.values(beforePanes).some((count) => count !== 1)) {
        throw new Error(`常設4区画が開始時に各1ではありません: ${JSON.stringify(beforePanes)}`);
      }
      mustButton(document.querySelector("header.toolbar") ?? document, "提案").click();
      const proposal = await waitFor(
        () => document.querySelector('[data-floating-ui="proposal-dialog"]'),
        "提案dialog",
      );
      if (proposal.getAttribute("data-proposal-step") !== "skeleton") {
        throw new Error(`提案の初期stepがskeletonではありません: ${proposal.getAttribute("data-proposal-step")}`);
      }
      mustButton(proposal, "展開図を作ってもらう").click();
      const candidatesDialog = await waitFor(
        () => document.querySelector('[data-floating-ui="proposal-dialog"][data-proposal-step="candidates"]'),
        "候補step",
      );
      const candidates = [...candidatesDialog.querySelectorAll('button[aria-label^="候補"]')];
      if (candidates.length !== 4) throw new Error(`候補数が4ではありません: ${candidates.length}`);
      const violationCaptions = [...candidatesDialog.querySelectorAll(".candidate-caption")].filter((caption) =>
        /^候補[1-4]:/u.test(text(caption)),
      );
      if (violationCaptions.length !== 4) {
        throw new Error(`候補の違反数文が4件ではありません: ${violationCaptions.length}`);
      }
      candidates[0].click();
      await pause();
      mustButton(candidatesDialog, "これにする").click();
      const confirmDialog = await waitFor(
        () => document.querySelector('[data-floating-ui="proposal-dialog"][data-proposal-step="confirm"]'),
        "確認step",
      );
      mustButton(confirmDialog, "この展開図を使う").click();
      await waitFor(
        () => (document.querySelector('[data-floating-ui="proposal-dialog"]') === null ? true : null),
        "提案dialogのclose",
      );
      const afterPanes = fourPanes();
      if (Object.values(afterPanes).some((count) => count !== 1)) {
        throw new Error(`提案後の常設4区画が各1ではありません: ${JSON.stringify(afterPanes)}`);
      }
      results.push({
        id: "M3.T3-4.C01",
        passed: true,
        steps: ["skeleton", "candidates", "confirm"],
        candidateCount: candidates.length,
        violationCaptionCount: violationCaptions.length,
      });
      results.push({
        id: "M3.T3-4.C02",
        passed: true,
        beforePanes,
        afterPanes,
      });

      // M4.T4-3.C02: native保存dialogを開かず、書出しの種類・補助線・PNG値だけを検証する。
      await openFixture(input.fixtures.birdBase);
      mustButton(document.querySelector("header.toolbar") ?? document, "書き出し").click();
      const exportDialog = await waitFor(
        () => document.querySelector('[data-floating-ui="export-dialog"]'),
        "書き出しdialog",
      );
      const radios = [...exportDialog.querySelectorAll('input[type="radio"][name="export-kind"]')];
      if (radios.length !== 4) throw new Error(`書出しradio数が4ではありません: ${radios.length}`);
      const pngLabel = [...exportDialog.querySelectorAll("label")].find((label) => text(label) === "展開図(PNG)");
      const pngRadio = pngLabel?.querySelector('input[type="radio"]');
      if (!pngRadio) throw new Error("展開図(PNG) radioがありません");
      pngRadio.click();
      await pause();
      const sizeInput = exportDialog.querySelector('input[aria-label="画像の大きさ（長辺の点数）"]');
      if (!(sizeInput instanceof HTMLInputElement)) throw new Error("PNG長辺inputがありません");
      const nativeValueSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      if (!nativeValueSetter) throw new Error("input value setterがありません");
      nativeValueSetter.call(sizeInput, "1024");
      sizeInput.dispatchEvent(new Event("input", { bubbles: true }));
      sizeInput.dispatchEvent(new Event("change", { bubbles: true }));
      await pause();
      if (sizeInput.value !== "1024") throw new Error(`PNG長辺が1024になりません: ${sizeInput.value}`);
      const aux = exportDialog.querySelector('input[type="checkbox"]');
      if (!(aux instanceof HTMLInputElement)) throw new Error("補助線checkboxがありません");
      const beforeAux = aux.checked;
      aux.click();
      await pause();
      const afterAux = aux.checked;
      if (beforeAux === afterAux) throw new Error("補助線checkboxが切り替わりません");
      mustButton(exportDialog, "閉じる").click();
      await waitFor(
        () => (document.querySelector('[data-floating-ui="export-dialog"]') === null ? true : null),
        "書き出しdialogのclose",
      );
      results.push({ id: "M4.T4-3.C02", passed: true, radioCount: radios.length, pngLongSide: 1024 });

      // 完全な作図・PDF/SVG保存はblockedCasesに残す。ここで選択UIだけを通してもB1完了には数えない。
      return { results, blocked: input.blocked };
    }})(${JSON.stringify({
      fixtures: Object.fromEntries(Object.entries(fixtures).map(([name, fixture]) => [name, fixture.path])),
      blocked: blockedCases,
    })})`,
  );
}

async function main() {
  if (!verifyExecutionContract()) return;
  const endpoint = `http://127.0.0.1:${cdpPort}`;
  const targets = await fetch(`${endpoint}/json/list`).then((response) => {
    if (!response.ok) throw new Error(`CDP /json/list: HTTP ${response.status}`);
    return response.json();
  });
  const page = targets.find(
    (target) => target.type === "page" && target.url === "http://tauri.localhost/" && target.webSocketDebuggerUrl,
  );
  if (!page) throw new Error("ORIGAMI3のWebView2 page targetがありません");
  const connection = await CdpConnection.connect(page.webSocketDebuggerUrl);
  let originalUi = null;
  let originalMetrics = null;
  try {
    await connection.send("Runtime.enable");
    originalUi = await evaluate(
      connection,
      `(${async function snapshotUi() {
        const recovery = document.querySelector('[data-floating-ui="recovery-dialog"]');
        if (recovery) {
          const restore = [...recovery.querySelectorAll("button")].find(
            (button) => (button.textContent ?? "").replace(/\\s+/gu, " ").trim() === "復元する",
          );
          if (!restore) throw new Error("回復画面に復元するbuttonがありません");
          restore.click();
          for (let index = 0; index < 3; index++) await new Promise((resolve) => requestAnimationFrame(resolve));
        }
        const dialogs = document.querySelectorAll('[data-floating-ui$="dialog"]');
        if (dialogs.length !== 0) throw new Error(`開始時に回復以外のdialogが開いています: ${dialogs.length}`);
        return {
          activeTool: (document.querySelector("nav.tool-rail .tool-button.active")?.textContent ?? "")
            .replace(/\\s+/gu, " ")
            .trim(),
          dialogs: dialogs.length,
          captureView: document.documentElement.getAttribute("data-origami3-capture-view"),
        };
      }})()`,
    );
    originalMetrics = await evaluate(
      connection,
      "({ innerWidth, innerHeight, devicePixelRatio, captureView: document.documentElement.getAttribute('data-origami3-capture-view') })",
    );
    await connection.send("Emulation.setDeviceMetricsOverride", {
      width: 1280,
      height: 860,
      deviceScaleFactor: 2,
      mobile: false,
    });
    const outcome = await runInWebView(connection, { originalUi });
    assert.equal(outcome.results.length, 5, "実行可能なB1検査5件が完走していません");
    assert.ok(outcome.results.every((result) => result.passed === true), "B1検査に不合格があります");
    process.stdout.write(
      `${JSON.stringify(
        {
          version: 1,
          endpoint,
          executable: {
            pid: desktopPid,
            path: claimedExecutablePath,
            sha256: expectedExecutableSha256,
          },
          runnable: outcome.results,
          blocked: outcome.blocked,
        },
        null,
        2,
      )}\n`,
    );
    // blockedの10件を合格へ見せかけない。専用fixture/APIがそろうまでexit 2にする。
    process.exitCode = 2;
  } finally {
    try {
      if (originalUi) {
        const restored = await restoreUi(connection, originalUi);
        assert.equal(restored.activeTool, originalUi.activeTool, "active toolを元へ戻せませんでした");
        assert.equal(restored.dialogs, 0, "dialogを閉じ切れませんでした");
        assert.equal(restored.captureView, null, "capture view属性を戻せませんでした");
      }
    } finally {
      if (originalMetrics) {
        await connection.send("Emulation.clearDeviceMetricsOverride");
        const restoredMetrics = await evaluate(
          connection,
          "({ innerWidth, innerHeight, devicePixelRatio, captureView: document.documentElement.getAttribute('data-origami3-capture-view') })",
        );
        assert.deepEqual(restoredMetrics, originalMetrics, "CDP viewportを元へ戻せませんでした");
      }
      connection.close();
    }
  }
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.exitCode = 1;
});
