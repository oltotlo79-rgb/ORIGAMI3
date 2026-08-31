import type {
  CoreWorkerRequest,
  CoreWorkerResponse,
} from "./coreWorkerProtocol";
import initOri3Web, {
  Ori3WasmBackend,
} from "./generated/ori3-web/ori3_web.js";

export interface Ori3WasmBackendPort {
  invoke_json(requestJson: string): string | Promise<string>;
}

interface CoreWorkerScope {
  addEventListener(
    type: "message" | "messageerror",
    listener: (event: MessageEvent<unknown>) => void,
  ): void;
  postMessage(message: CoreWorkerResponse): void;
}

export type CoreWorkerInitializer = () => void | Promise<void>;

let wasmBackend: Ori3WasmBackendPort | undefined;

export function initializeCoreWorker(backend: Ori3WasmBackendPort): void {
  wasmBackend = backend;
}

export async function loadOri3WasmBackend(
  initialize: () => Promise<unknown> = initOri3Web,
  create: () => Ori3WasmBackendPort = () => new Ori3WasmBackend(),
): Promise<Ori3WasmBackendPort> {
  await initialize();
  return create();
}

export async function connectOri3WasmBackend(): Promise<void> {
  initializeCoreWorker(await loadOri3WasmBackend());
}

function rejectionText(command: CoreWorkerRequest["command"]): string {
  return `Web版の「${command}」は計算WorkerとWASMの接続を準備中のため、まだ利用できません。`;
}

function errorText(reason: unknown, command: CoreWorkerRequest["command"]): string {
  if (typeof reason === "string" && reason.length > 0) return reason;
  if (reason instanceof Error && reason.message.length > 0) {
    return `Web版の「${command}」の計算に失敗しました。${reason.message}`;
  }
  return rejectionText(command);
}

function objectValue(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null
    ? (value as Record<string, unknown>)
    : null;
}

function isCoreWorkerRequest(value: unknown): value is CoreWorkerRequest {
  const record = objectValue(value);
  return (
    record?.type === "invoke" &&
    typeof record.id === "number" &&
    Number.isSafeInteger(record.id) &&
    typeof record.command === "string"
  );
}

export async function handleCoreWorkerRequest(
  request: CoreWorkerRequest,
  backend: Ori3WasmBackendPort | undefined = wasmBackend,
): Promise<CoreWorkerResponse> {
  if (!backend) {
    return {
      type: "result",
      id: request.id,
      ok: false,
      error: rejectionText(request.command),
    };
  }

  try {
    const resultJson = await backend.invoke_json(
      JSON.stringify({
        command: request.command,
        args: request.args ?? null,
      }),
    );
    return {
      type: "result",
      id: request.id,
      ok: true,
      value: JSON.parse(resultJson) as unknown,
    };
  } catch (reason) {
    return {
      type: "result",
      id: request.id,
      ok: false,
      error: errorText(reason, request.command),
    };
  }
}

function initializationErrorText(reason: unknown): string {
  const detail =
    typeof reason === "string"
      ? reason
      : reason instanceof Error
        ? reason.message
        : "原因を読み取れませんでした";
  return `Web版の計算Workerを準備できませんでした。${detail}`;
}

const WORKER_PROTOCOL_ERROR =
  "Web版の計算Workerが正しい要求を受け取れませんでした。ページを読み直して、もう一度お試しください。";

function postFatal(scope: CoreWorkerScope, error: string): void {
  try {
    scope.postMessage({ type: "fatal-error", error });
  } catch {
    return;
  }
}

/** listenerを先に登録し、初期化完了後にだけreadyを通知する。 */
export async function startCoreWorker(
  scope: CoreWorkerScope,
  initialize: CoreWorkerInitializer = () => undefined,
): Promise<void> {
  let queue = Promise.resolve();
  scope.addEventListener("message", (event) => {
    if (!isCoreWorkerRequest(event.data)) {
      postFatal(scope, WORKER_PROTOCOL_ERROR);
      return;
    }
    const request = event.data;
    queue = queue
      .then(async () => {
        const response = await handleCoreWorkerRequest(request);
        scope.postMessage(response);
      })
      .catch((reason: unknown) => {
        postFatal(scope, initializationErrorText(reason));
      });
  });
  scope.addEventListener("messageerror", () => {
    postFatal(scope, WORKER_PROTOCOL_ERROR);
  });
  try {
    await initialize();
    scope.postMessage({ type: "ready" });
  } catch (reason) {
    scope.postMessage({
      type: "initialization-error",
      error: initializationErrorText(reason),
    });
  }
}

const workerScope = globalThis as unknown as CoreWorkerScope;
if (
  typeof document === "undefined" &&
  typeof globalThis.addEventListener === "function" &&
  typeof globalThis.postMessage === "function"
) {
  void startCoreWorker(workerScope, connectOri3WasmBackend);
}
