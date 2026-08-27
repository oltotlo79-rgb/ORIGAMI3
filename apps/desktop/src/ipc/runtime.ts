import {
  invoke as tauriInvoke,
  isTauri,
  type InvokeArgs,
} from "@tauri-apps/api/core";

export type Runtime = "tauri" | "web";

export interface Ori3WebBridge {
  invoke<T>(name: string, args?: InvokeArgs): Promise<T>;
}

declare global {
  interface Window {
    __ori3Web?: Ori3WebBridge;
  }
}

const ACTIVE_RUNTIME: Runtime = isTauri() ? "tauri" : "web";

/** モジュール初期化時に1回だけ決めた実行環境を返す。 */
export function detectRuntime(): Runtime {
  return ACTIVE_RUNTIME;
}

const UNAVAILABLE_WEB_BRIDGE =
  "Web版の計算機能を準備できていないため、この操作はまだ利用できません。ページを読み直して、もう一度お試しください。";

/** 画面から計算機能を呼ぶ唯一の入口。 */
export function callBackend<T>(
  name: string,
  args?: InvokeArgs,
): Promise<T> {
  if (ACTIVE_RUNTIME === "tauri") {
    return args === undefined
      ? tauriInvoke<T>(name)
      : tauriInvoke<T>(name, args);
  }

  const bridge =
    typeof window === "undefined" ? undefined : window.__ori3Web;
  if (typeof bridge?.invoke !== "function") {
    return Promise.reject(UNAVAILABLE_WEB_BRIDGE);
  }
  return bridge.invoke<T>(name, args);
}
