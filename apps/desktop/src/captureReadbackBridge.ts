/**
 * CDPの恒久検査が、画面に出たものと同じ3段の描画結果を照合するための読取結果。
 * WebGLのreadPixelsに合わせ、全bufferは左下を先頭にした物理画素順で返す。
 *
 * この末端はThree.jsを読まない。通常起動時のcapture APIから3D実装を切り離すため、
 * JSONへ直せる契約と、現在表示中のsceneを結ぶ登録口だけを所有する。
 */
export interface Viewer3DReadback {
  readonly version: 1;
  readonly width: number;
  readonly height: number;
  readonly rowOrder: "bottom-to-top";
  readonly owner: {
    readonly encoding: "rgba8-base64";
    readonly data: string;
    /** owner code 0は背景。紙のcodeだけを [code, face ID] で返す。 */
    readonly codeToFace: readonly (readonly [number, number])[];
  };
  readonly depth: {
    readonly encoding: "rgba8-packed-depth-base64";
    readonly data: string;
  };
  readonly final: {
    readonly encoding: "rgba8-base64";
    readonly data: string;
  };
}

type Viewer3DReadbackSource = () => Viewer3DReadback;

let activeSource: Viewer3DReadbackSource | null = null;
const readyWaiters = new Set<() => void>();

/**
 * 現在表示中のsceneだけを登録する。古いsceneのcleanupで新しい登録を消さない。
 */
export function registerViewer3DReadback(
  source: Viewer3DReadbackSource,
): () => void {
  activeSource = source;
  for (const resolve of readyWaiters) resolve();
  readyWaiters.clear();

  return () => {
    if (activeSource === source) activeSource = null;
  };
}

/** Viewer3Dへ検査用refを足さず、現在表示中の実sceneだけを同期して読み取る。 */
export function captureViewer3DReadback(): Viewer3DReadback {
  if (activeSource === null) {
    throw new Error("3D表示の描画資源がまだ用意されていません");
  }
  return activeSource();
}

/**
 * scene登録まで待つ。StrictModeで登録直後に古いsceneが外れた場合は、次の登録を待ち直す。
 */
export async function waitForViewer3DReady(): Promise<void> {
  while (activeSource === null) {
    await new Promise<void>((resolve) => readyWaiters.add(resolve));
  }
}
