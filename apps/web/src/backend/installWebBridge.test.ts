// @vitest-environment jsdom

import type { InvokeArgs } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it } from "vitest";
import { BACKEND_COMMAND_NAMES } from "../../../desktop/src/ipc/runtime";
import {
  createOri3WebBridge,
  installOri3WebBridge,
  type WebBridgeDependencies,
  type WebCommandInvoker,
} from "./installWebBridge";
import { WEB_COMMAND_ROUTES, type WebCommandRoute } from "./routes";

interface RecordedCall {
  command: (typeof BACKEND_COMMAND_NAMES)[number];
  args?: InvokeArgs;
}

function recordingInvoker(route: WebCommandRoute): {
  invoker: WebCommandInvoker;
  calls: RecordedCall[];
} {
  const calls: RecordedCall[] = [];
  return {
    calls,
    invoker: {
      invoke<T>(command, args): Promise<T> {
        calls.push({ command, ...(args === undefined ? {} : { args }) });
        return Promise.resolve(route as unknown as T);
      },
    },
  };
}

function recordingDependencies(): {
  dependencies: WebBridgeDependencies;
  calls: Record<WebCommandRoute, RecordedCall[]>;
} {
  const core = recordingInvoker("core");
  const proposal = recordingInvoker("proposal");
  const browser = recordingInvoker("browser");
  const mixed = recordingInvoker("mixed");
  return {
    dependencies: {
      core: core.invoker,
      proposal: proposal.invoker,
      browser: browser.invoker,
      mixed: mixed.invoker,
    },
    calls: {
      core: core.calls,
      proposal: proposal.calls,
      browser: browser.calls,
      mixed: mixed.calls,
    },
  };
}

beforeEach(() => {
  delete window.__ori3Web;
});

describe("Web bridge", () => {
  it("18件を宣言したrouteへ排他的に振り分ける", async () => {
    const { dependencies, calls } = recordingDependencies();
    const bridge = createOri3WebBridge(dependencies);

    for (const command of BACKEND_COMMAND_NAMES) {
      const args = { marker: command };
      await expect(bridge.invoke(command, args)).resolves.toBe(
        WEB_COMMAND_ROUTES[command],
      );
    }

    expect(Object.fromEntries(
      Object.entries(calls).map(([route, routeCalls]) => [
        route,
        routeCalls.length,
      ]),
    )).toEqual({ core: 10, proposal: 3, browser: 1, mixed: 4 });
    expect(Object.values(calls).flat()).toHaveLength(18);
  });

  it("React起動前に同期的にwindowへbridgeを設定できる", () => {
    const { dependencies } = recordingDependencies();

    installOri3WebBridge(window, dependencies);

    expect(typeof window.__ori3Web?.invoke).toBe("function");
  });

  it("未接続routeはコマンド名を含む日本語で理由を返す", async () => {
    const bridge = createOri3WebBridge();

    await expect(bridge.invoke("proposal_generate")).rejects.toBe(
      "Web版の「proposal_generate」は提案計算用Workerとの接続を準備中のため、まだ利用できません。",
    );
    await expect(bridge.invoke("recovery_check")).rejects.toBe(
      "Web版の「recovery_check」はブラウザ保存領域との接続を準備中のため、まだ利用できません。",
    );
  });
});
