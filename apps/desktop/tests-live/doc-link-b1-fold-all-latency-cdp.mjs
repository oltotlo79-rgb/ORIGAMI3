// ADDITIONAL.FOLD-ALL.C02: NFR-002 on the "全部いっぺんに折ってみる" slider, measured live.
// `prepare` is offline. `verify` connects only to an already-running dedicated desktop.exe.
//
// 何を主張するか
// --------------
// docs/requirements-definition.md:387 NFR-002:
//   「性能目標: 展開図1,000辺・面400までの規模で、ヒンジ角操作時の3D更新30fps以上
//     (ソルバー1回33ms以内)、全手順再生の計算3秒以内」
// この script はそのうち一斉折りのつまみに関わる 2 点だけを測る。
//   (a) ソルバー1回 33 ms 以内 …… 1入力に対する `fold_all_preview` の要求から応答までの時間。
//       10 回の最大が 33 ms 以内であること。
//   (b) 3D更新 30 fps 以上 ……… つまみを連続で動かした 1 秒間に `data-applied-percent` が
//       何回変わったかを数え、30 回/秒以上であること。
//       設計上の上限は間引き `FOLD_ALL_THROTTLE_MS = 16`
//       (apps/desktop/src/store/slices/poseReplaySlice.ts) から 62.5 回/秒で、
//       実際の更新率は 1 / max(16 ms, ソルバー1回) になる。
//
// 「入力から画面反映まで 33 ms」は要件ではない（NFR-002 の 33 ms はソルバー1回の予算）ため、
// 端から端の値は assert せず参考値として出力するだけにする。
// (b) を「更新間隔の最大 33.3 ms 以内」ではなく「1 秒あたり 30 回以上」で見るのは、
// 30 fps が要件上まさに更新の「率」であり、1 回の GC で跳ねた間隔 1 件で
// 落とさないためである。間隔の最大・中央値は参考値として併せて出す。
//
// id について: `M2.T2-6b.C06` は `MANUAL.M2.T2-6b.C06.SCREEN-ACCEPTANCE`（技法サブメニュー9種、
// docs/implementation-roadmap.md:708）が使用済みなので再利用しない。
// 一斉折りの節の `ADDITIONAL.FOLD-ALL.C01` は「手順・保存・Undoに残さない」という主張だけを持ち、
// 性能については何も言っていない。そこで統括の判断（2026-09-05）により、同じ節へ
// `ADDITIONAL.FOLD-ALL.C02`（実機確認: 一斉折りの仮表示が NFR-002 を満たすこと、
// docs/implementation-roadmap.md:971）を新設し、この script をその証拠
// `MANUAL.ADDITIONAL.FOLD-ALL.C02.SCREEN-ACCEPTANCE` の実行本体とした。
//
// ソルバー1回の測り方: `window.__origami3Capture` に時刻の読取値は無い。製品へは何も足さない。
// 実機(WebView2 release)で CDP から実測した経路は次のとおり:
//   ipc/client.ts invoke("fold_all_preview") → @tauri-apps/api core invoke
//   → window.__TAURI_INTERNALS__.ipc → sendIpcMessage
//   → fetch("http://ipc.localhost/fold_all_preview", {method:"POST"})
// `__TAURI_INTERNALS__.invoke` と `.postMessage` は writable:false / configurable:false のため
// 包めない（非strictでの代入は例外も出さずに無視される。これが 2026-09-05 の
// `no fold_all_preview call was observed` の原因だった）。
// 一方 `window.fetch` は writable:true / configurable:true なので、測定のあいだだけ包み、
// URL の path が command 名と一致する要求の往復時間を採って、最後に必ず元へ戻す。
// 包めたことは代入直後に読み戻して確かめる（黙って 0 件になるのを二度と起こさないため）。

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

const id = "ADDITIONAL.FOLD-ALL.C02";
const phase = resolvePhase(id);

/** NFR-002「ソルバー1回33ms以内」。要件の数値そのままで、緩めていない。 */
const SOLVE_BUDGET_MS = 33;
/** NFR-002「3D更新30fps以上」。要件の数値そのまま。 */
const MIN_UPDATES_PER_SECOND = 30;
/** (a) で測る入力の回数と順番。 */
const PERCENTS = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
/** (b) で連続して動かす時間。30 fps を 1 秒の窓で数えるため。 */
const SWEEP_MS = 1000;
/** 1 入力がこの時間内に反映されなければ、測定値ではなく不具合として落とす。 */
const STUCK_MS = 5000;
/**
 * 一斉折りのbackend command名。`apps/desktop/src/ipc/client.ts:113` の
 * `invoke("fold_all_preview", …)` と `apps/desktop/src/ipc/runtime.ts` の
 * `BACKEND_COMMAND_NAMES` に一致する。実機では
 * `POST http://ipc.localhost/fold_all_preview` として飛ぶことを CDP で実測済み。
 */
const SOLVE_COMMAND = "fold_all_preview";

async function verify() {
  const runtime = verifyRuntime(id, "ORI3_B1_FOLD_ALL_LATENCY");
  const connection = await connectDesktop();
  try {
    const result = await evaluate(connection, `(${async function measure(input) {
      const api = window.__origami3Capture;
      if (!api || api.version !== 1) throw new Error("Capture API version 1 is unavailable");
      const frames = async (count) => {
        for (let i = 0; i < count; i += 1) await new Promise((resolve) => requestAnimationFrame(resolve));
      };
      const waitFor = async (read, description) => {
        const deadline = performance.now() + input.stuckMs;
        for (;;) {
          const value = read();
          if (value) return value;
          if (performance.now() > deadline) throw new Error(`timed out waiting for ${description}`);
          await new Promise((resolve) => requestAnimationFrame(resolve));
        }
      };

      await api.openDocument(input.fixturePath);
      await api.setView("normal");
      await api.waitForStable();

      const entry = Array.from(document.querySelectorAll(".paper-action-entrances button")).find(
        (button) => (button.textContent ?? "").includes("全部いっぺんに折ってみる"),
      );
      if (!(entry instanceof HTMLButtonElement)) throw new Error("the fold-all entry button is unavailable");
      if (entry.disabled) throw new Error("the fixture has no mountain or valley crease, so fold-all cannot start");
      entry.click();

      const active = await waitFor(
        () => document.querySelector("[data-fold-all-active]"),
        "the fold-all panel to open",
      );
      await waitFor(
        () => active.getAttribute("data-applied-percent") === "0",
        "the fold-all panel to settle at 0%",
      );

      const slider = document.getElementById("fold-all-percent");
      if (!(slider instanceof HTMLInputElement)) throw new Error("the fold-all slider is unavailable");
      if (slider.getAttribute("aria-label") !== "全部の折り目を動かす割合") {
        throw new Error("the fold-all slider lost its accessible name");
      }
      // Reactが値の変化を受け取れるように、値はネイティブのsetterで入れてからinputを配る。
      const setValue = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
      const moveTo = (percent) => {
        setValue.call(slider, String(percent));
        slider.dispatchEvent(new Event("input", { bubbles: true }));
      };

      // ソルバー1回を測るため、製品を変えずに window.fetch を測定中だけ包む。
      // `__TAURI_INTERNALS__.invoke` と `.postMessage` は writable:false / configurable:false
      // なので包めない（代入は非strictで黙って失敗する）。実測した経路は
      // invoke → __TAURI_INTERNALS__.ipc → sendIpcMessage → fetch("http://ipc.localhost/<cmd>", POST)。
      const originalFetch = window.fetch;
      if (typeof originalFetch !== "function") throw new Error("window.fetch is unavailable");
      const solveCalls = [];
      const commandOf = (resource) => {
        const raw = typeof resource === "string"
          ? resource
          : (resource && typeof resource.url === "string" ? resource.url : String(resource));
        try {
          return new URL(raw).pathname.replace(/^\//u, "");
        } catch {
          return "";
        }
      };
      window.fetch = function timedFetch(resource, init) {
        const returned = originalFetch.call(this, resource, init);
        if (commandOf(resource) !== input.solveCommand) return returned;
        const record = { started: performance.now(), ended: null };
        solveCalls.push(record);
        return Promise.resolve(returned).then(
          (value) => {
            record.ended = performance.now();
            return value;
          },
          (error) => {
            record.ended = performance.now();
            throw error;
          },
        );
      };
      // 包めたことを必ず確かめる。黙って失敗すると 0 件を性能値と取り違える。
      if (window.fetch === originalFetch) {
        throw new Error("window.fetch could not be wrapped, so the solver round trip cannot be read");
      }

      const endToEnd = [];
      let sweepGaps = [];
      let sweepUpdates = 0;
      let sweepElapsed = 0;
      let sweepInputs = 0;
      let solveCallsInPartA = 0;
      try {
        // (a) 1入力ごとに反映まで待ち、そのぶんのソルバー1回を測る。
        for (const percent of input.percents) {
          const changed = new Promise((resolve, reject) => {
            let stuck = 0;
            const observer = new MutationObserver(() => {
              if (active.getAttribute("data-applied-percent") === String(percent)) {
                clearTimeout(stuck);
                observer.disconnect();
                resolve();
              }
            });
            stuck = setTimeout(() => {
              observer.disconnect();
              reject(new Error(`the shape never reached ${percent}%`));
            }, input.stuckMs);
            observer.observe(active, { attributes: true });
          });
          const started = performance.now();
          moveTo(percent);
          await changed;
          endToEnd.push(performance.now() - started);
        }

        // ここまでが (a)。以降の solve は (b) の連続操作ぶんなので境目を覚えておく。
        solveCallsInPartA = solveCalls.length;

        // (b) 1秒間つまみを動かし続け、形の更新が何回届くかを数える。
        const stamps = [];
        const sweepObserver = new MutationObserver((records) => {
          // 1回の呼び出しに複数件まとまることがあるので、記録の件数で数える。
          const at = performance.now();
          for (let i = 0; i < records.length; i += 1) stamps.push(at);
        });
        sweepObserver.observe(active, { attributes: true, attributeFilter: ["data-applied-percent"] });
        const sweepStart = performance.now();
        let step = 0;
        while (performance.now() - sweepStart < input.sweepMs) {
          step += 1;
          moveTo((step % 100) + 1);
          await new Promise((resolve) => requestAnimationFrame(resolve));
        }
        sweepElapsed = performance.now() - sweepStart;
        // 送った入力の数も残す。これが少ない回は、製品ではなく
        // 機械の混雑や rAF の間引き(窓が隠れている等)で駆動できていない。
        sweepInputs = step;
        // 最後の要求が返るぶんだけ待ってから数え終える。
        await frames(4);
        sweepObserver.disconnect();
        sweepUpdates = stamps.length;
        sweepGaps = stamps.slice(1).map((stamp, index) => stamp - stamps[index]);
      } finally {
        window.fetch = originalFetch;
      }

      // (a) の主張は「1入力ごとに反映まで待った 10 回」に限る。
      // (b) の連続操作中の solve は、待たずに次を送る別の条件なので参考として分けて出す。
      const durationOf = (call) => call.ended - call.started;
      const solveDurations = solveCalls
        .slice(0, solveCallsInPartA)
        .filter((call) => call.ended !== null)
        .map(durationOf);
      const sweepSolveDurations = solveCalls
        .slice(solveCallsInPartA)
        .filter((call) => call.ended !== null)
        .map(durationOf);
      if (solveDurations.length === 0) {
        throw new Error(
          `no ${input.solveCommand} fetch was observed although the shape did apply; ` +
            "the IPC may have fallen back to window.chrome.webview.postMessage instead of the ipc:// custom protocol",
        );
      }

      const round = (value) => Number(value.toFixed(3));
      const median = (values) => {
        if (values.length === 0) return 0;
        const sorted = [...values].sort((left, right) => left - right);
        return sorted[Math.floor(sorted.length / 2)];
      };
      const interaction = api.getInteractionState();

      // 測り終えたら専用表示を閉じ、次の受入のために元の表示へ戻す。
      const back = Array.from(document.querySelectorAll("[data-fold-all-active] button")).find(
        (button) => (button.textContent ?? "").trim() === "いつもの表示に戻る",
      );
      if (!(back instanceof HTMLButtonElement)) throw new Error("the return button is unavailable");
      back.click();
      await waitFor(() => document.querySelector("[data-fold-all-active]") === null, "the fold-all panel to close");
      await frames(3);

      return {
        scale: {
          edgeCount: interaction.document.edgeCount,
          stepCount: api.getDocumentInfo().stepCount,
        },
        solve: {
          count: solveDurations.length,
          maximum: round(Math.max(...solveDurations)),
          average: round(solveDurations.reduce((sum, value) => sum + value, 0) / solveDurations.length),
          median: round(median(solveDurations)),
          // 1件ずつ出す。跳ねたのが初回(暖機)か、途中のどれかを見分けるため。
          durations: solveDurations.map(round),
          slowestIndex: solveDurations.indexOf(Math.max(...solveDurations)),
        },
        // 参考: (b) の連続操作中のsolve。待たずに次を送るので (a) とは条件が違い、assert しない。
        solveDuringSweepReference: sweepSolveDurations.length === 0
          ? null
          : {
              count: sweepSolveDurations.length,
              maximum: round(Math.max(...sweepSolveDurations)),
              average: round(
                sweepSolveDurations.reduce((sum, value) => sum + value, 0) / sweepSolveDurations.length,
              ),
              median: round(median(sweepSolveDurations)),
            },
        updates: {
          count: sweepUpdates,
          inputsSent: sweepInputs,
          elapsedMs: round(sweepElapsed),
          perSecond: round((sweepUpdates * 1000) / sweepElapsed),
          maxGapMs: sweepGaps.length > 0 ? round(Math.max(...sweepGaps)) : null,
          medianGapMs: sweepGaps.length > 0 ? round(median(sweepGaps)) : null,
        },
        // 参考値のみ。要件ではないので assert しない。
        inputToAppliedReference: {
          average: round(endToEnd.reduce((sum, value) => sum + value, 0) / endToEnd.length),
          maximum: round(Math.max(...endToEnd)),
        },
      };
    }})(${JSON.stringify({
      fixturePath: runtime.fixturePath,
      percents: PERCENTS,
      sweepMs: SWEEP_MS,
      stuckMs: STUCK_MS,
      solveCommand: SOLVE_COMMAND,
    })})`);

    // assert より先に全数値を出す。赤でも測定値が読めるようにするため。
    process.stdout.write(`${id} MEASURED\n`);
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);

    // NFR-002 (a): ソルバー1回 33 ms 以内。
    assert.equal(
      result.solve.count,
      PERCENTS.length,
      `expected one solver call per input: ${PERCENTS.length} inputs but ${result.solve.count} calls`,
    );
    assert.ok(
      result.solve.maximum <= SOLVE_BUDGET_MS,
      `NFR-002: solver call ${result.solve.maximum}ms exceeds ${SOLVE_BUDGET_MS}ms (average ${result.solve.average}ms)`,
    );
    // NFR-002 (b): 3D更新 30 fps 以上。
    assert.ok(
      result.updates.perSecond >= MIN_UPDATES_PER_SECOND,
      `NFR-002: 3D updated ${result.updates.perSecond}/s, below ${MIN_UPDATES_PER_SECOND}/s`,
    );
    const restored = await restoreBlank(connection);
    passed(id, {
      runtime,
      requirement: "docs/requirements-definition.md:387 NFR-002",
      budgets: { solveMs: SOLVE_BUDGET_MS, updatesPerSecond: MIN_UPDATES_PER_SECOND },
      result,
      restored,
    });
  } finally {
    connection.close();
  }
}

try {
  if (phase === "prepare") {
    prepare(
      id,
      [
        path.resolve(repositoryRoot, "apps/desktop/src/components/contextPaperDisplay.tsx"),
        path.resolve(repositoryRoot, "apps/desktop/src/captureApi.ts"),
        path.resolve(repositoryRoot, "apps/desktop/src/ipc/runtime.ts"),
      ],
      ["ORI3_B1_FOLD_ALL_LATENCY_FIXTURE", "ORI3_B1_FOLD_ALL_LATENCY_FIXTURE_SHA256"],
    );
  } else if (phase === "verify") {
    await verify();
  } else {
    process.stdout.write(`${id} PREPARE/VERIFY NOT EXECUTED\n`);
    process.stdout.write(`${JSON.stringify({ id, phases: ["prepare", "verify"], cdpConnected: false }, null, 2)}\n`);
  }
} catch (error) {
  failed(id, phase, error);
}
