import type {
  BackendInvokeArgs,
} from "../../../desktop/src/ipc/runtime";
import type {
  CoreCommandName,
  CoreWorkerRequest,
  CoreWorkerResponse,
} from "./coreWorkerProtocol";

interface PendingRequest {
  command: CoreCommandName;
  resolve(value: unknown): void;
  reject(reason: string): void;
}

interface ActiveWorker {
  worker: Worker;
  ready: Promise<void>;
  resolveReady(): void;
  rejectReady(reason: string): void;
  readySettled: boolean;
  readyTimer: unknown;
  failed: boolean;
}

export interface WorkerClock {
  setTimeout(callback: () => void, delayMs: number): unknown;
  clearTimeout(handle: unknown): void;
}

export interface CoreWorkerClientOptions {
  readyTimeoutMs?: number;
  clock?: WorkerClock;
}

export interface Ori3CoreWorkerClient {
  invoke<T>(command: CoreCommandName, args?: BackendInvokeArgs): Promise<T>;
  /** Workerで指定された初期化とRPC受付準備が完了するまで待つ。 */
  ready(): Promise<void>;
  dispose(): void;
}

export type CoreWorkerFactory = () => Worker;

const DEFAULT_READY_TIMEOUT_MS = 15_000;

const SYSTEM_CLOCK: WorkerClock = {
  setTimeout(callback, delayMs): unknown {
    return globalThis.setTimeout(callback, delayMs);
  },
  clearTimeout(handle): void {
    globalThis.clearTimeout(
      handle as ReturnType<typeof globalThis.setTimeout>,
    );
  },
};

function defaultCoreWorkerFactory(): Worker {
  return new Worker(new URL("./ori3-core.worker.ts", import.meta.url), {
    name: "ori3-core",
    type: "module",
  });
}

function publicCommandName(command: CoreCommandName): string {
  switch (command) {
    case "__web_document_open_source":
      return "document_open";
    case "__web_document_save_prepare":
    case "__web_document_save_abort":
      return "document_save";
    case "__web_document_export_prepare":
      return "document_export";
    case "__web_recovery_set_choices":
    case "__web_recovery_snapshot":
      return "recovery_check";
    case "__web_recovery_restore_source":
      return "recovery_restore";
    default:
      return command;
  }
}

function workerStartError(command: CoreCommandName): string {
  return `Web版の「${publicCommandName(command)}」は計算Workerを起動できないため、まだ利用できません。`;
}

const WORKER_DISPOSED =
  "Web版の計算Workerを終了したため、この操作はまだ利用できません。ページを読み直して、もう一度お試しください。";
const WORKER_FAILED =
  "Web版の計算Workerとの通信を続けられません。ページを読み直して、もう一度お試しください。";
const WORKER_PROTOCOL_ERROR =
  "Web版の計算Workerから正しい応答を受け取れませんでした。ページを読み直して、もう一度お試しください。";
const WORKER_READY_TIMEOUT =
  "Web版の計算Workerの準備に時間がかかりすぎました。ページを読み直して、もう一度お試しください。";

function objectValue(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null
    ? (value as Record<string, unknown>)
    : null;
}

function isCoreWorkerResponse(value: unknown): value is CoreWorkerResponse {
  const record = objectValue(value);
  if (!record || typeof record.type !== "string") return false;
  if (record.type === "ready") return true;
  if (
    record.type === "initialization-error" ||
    record.type === "fatal-error"
  ) {
    return typeof record.error === "string";
  }
  if (
    record.type !== "result" ||
    typeof record.id !== "number" ||
    !Number.isSafeInteger(record.id) ||
    typeof record.ok !== "boolean"
  ) {
    return false;
  }
  if (record.ok) return "value" in record;
  return typeof record.error === "string";
}

export function createOri3CoreWorkerClient(
  workerFactory: CoreWorkerFactory = defaultCoreWorkerFactory,
  options: CoreWorkerClientOptions = {},
): Ori3CoreWorkerClient {
  const readyTimeoutMs = options.readyTimeoutMs ?? DEFAULT_READY_TIMEOUT_MS;
  const clock = options.clock ?? SYSTEM_CLOCK;
  let active: ActiveWorker | undefined;
  let terminalFailure: string | undefined;
  let disposed = false;
  let nextRequestId = 1;
  const pending = new Map<number, PendingRequest>();

  const clearReadyTimer = (session: ActiveWorker): void => {
    clock.clearTimeout(session.readyTimer);
  };

  const failSession = (
    session: ActiveWorker,
    reason: string,
    commandReason?: (command: CoreCommandName) => string,
  ): void => {
    if (active !== session || session.failed) return;
    session.failed = true;
    terminalFailure = reason;
    clearReadyTimer(session);
    if (!session.readySettled) {
      session.readySettled = true;
      session.rejectReady(reason);
    }
    for (const request of pending.values()) {
      request.reject(commandReason?.(request.command) ?? reason);
    }
    pending.clear();
    session.worker.terminate();
    active = undefined;
  };

  const receiveMessage = (
    session: ActiveWorker,
    event: MessageEvent<unknown>,
  ): void => {
    if (active !== session || session.failed) return;
    if (!isCoreWorkerResponse(event.data)) {
      failSession(session, WORKER_PROTOCOL_ERROR);
      return;
    }
    const response = event.data;
    if (response.type === "ready") {
      if (!session.readySettled) {
        session.readySettled = true;
        clearReadyTimer(session);
        session.resolveReady();
      }
      return;
    }
    if (
      response.type === "initialization-error" ||
      response.type === "fatal-error"
    ) {
      failSession(session, response.error);
      return;
    }
    const request = pending.get(response.id);
    if (!request) return;
    pending.delete(response.id);
    if (response.ok) {
      request.resolve(response.value);
      return;
    }
    request.reject(response.error);
  };

  const requireWorker = (): ActiveWorker => {
    if (disposed) throw WORKER_DISPOSED;
    if (terminalFailure) throw terminalFailure;
    if (active) return active;

    const worker = workerFactory();
    let resolveReady = (): void => undefined;
    let rejectReady = (): void => undefined;
    const ready = new Promise<void>((resolve, reject) => {
      resolveReady = resolve;
      rejectReady = reject;
    });
    const session: ActiveWorker = {
      worker,
      ready,
      resolveReady,
      rejectReady,
      readySettled: false,
      readyTimer: undefined,
      failed: false,
    };
    active = session;
    worker.addEventListener("message", (event: MessageEvent<unknown>) => {
      receiveMessage(session, event);
    });
    const fail = (): void =>
      failSession(session, WORKER_FAILED, workerStartError);
    worker.addEventListener("error", fail);
    worker.addEventListener("messageerror", fail);
    session.readyTimer = clock.setTimeout(() => {
      failSession(session, WORKER_READY_TIMEOUT);
    }, readyTimeoutMs);
    return session;
  };

  return {
    async invoke<T>(
      command: CoreCommandName,
      args?: BackendInvokeArgs,
    ): Promise<T> {
      let session: ActiveWorker;
      try {
        session = requireWorker();
        await session.ready;
      } catch (reason) {
        if (typeof reason === "string" && reason.length > 0) throw reason;
        throw workerStartError(command);
      }

      return new Promise<T>((resolve, reject) => {
        const id = nextRequestId;
        nextRequestId += 1;
        pending.set(id, {
          command,
          resolve: (value) => resolve(value as T),
          reject,
        });
        const request: CoreWorkerRequest = {
          type: "invoke",
          id,
          command,
          ...(args === undefined ? {} : { args }),
        };
        try {
          session.worker.postMessage(request);
        } catch {
          pending.delete(id);
          reject(workerStartError(command));
        }
      });
    },

    ready(): Promise<void> {
      try {
        return requireWorker().ready;
      } catch (reason) {
        return Promise.reject(
          typeof reason === "string" && reason.length > 0
            ? reason
            : WORKER_FAILED,
        );
      }
    },

    dispose(): void {
      disposed = true;
      const session = active;
      if (session) failSession(session, WORKER_DISPOSED);
    },
  };
}
