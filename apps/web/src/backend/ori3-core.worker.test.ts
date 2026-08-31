import { describe, expect, it, vi } from "vitest";
import type {
  CoreWorkerRequest,
  CoreWorkerResponse,
} from "./coreWorkerProtocol";
import {
  handleCoreWorkerRequest,
  initializeCoreWorker,
  loadOri3WasmBackend,
  startCoreWorker,
  type Ori3WasmBackendPort,
} from "./ori3-core.worker";

const REQUEST: CoreWorkerRequest = {
  type: "invoke",
  id: 7,
  command: "pose_solve",
  args: { request: { hard: [] } },
};

interface TestScope {
  scope: {
    addEventListener(
      type: "message" | "messageerror",
      listener: (event: MessageEvent<unknown>) => void,
    ): void;
    postMessage(message: CoreWorkerResponse): void;
  };
  messages: CoreWorkerResponse[];
  send(value: unknown): void;
  messageError(): void;
}

function testScope(): TestScope {
  const messages: CoreWorkerResponse[] = [];
  let messageListener: ((event: MessageEvent<unknown>) => void) | undefined;
  let messageErrorListener: ((event: MessageEvent<unknown>) => void) | undefined;
  return {
    messages,
    scope: {
      addEventListener(type, listener): void {
        if (type === "message") messageListener = listener;
        else messageErrorListener = listener;
      },
      postMessage(message): void {
        messages.push(message);
      },
    },
    send(value): void {
      messageListener?.({ data: value } as MessageEvent<unknown>);
    },
    messageError(): void {
      messageErrorListener?.({ data: null } as MessageEvent<unknown>);
    },
  };
}

async function flushPromises(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
}

describe("core WorkerとWASMのJSON境界", () => {
  it("未接続ならコマンド名を含む日本語応答を返す", async () => {
    await expect(handleCoreWorkerRequest(REQUEST)).resolves.toEqual({
      type: "result",
      id: 7,
      ok: false,
      error:
        "Web版の「pose_solve」は計算WorkerとWASMの接続を準備中のため、まだ利用できません。",
    });
  });

  it("commandとargsをJSON文字列1本でWASMへ渡して結果を復元する", async () => {
    const invokeJson = vi.fn().mockReturnValue('{"revision":3}');
    const backend: Ori3WasmBackendPort = { invoke_json: invokeJson };

    await expect(handleCoreWorkerRequest(REQUEST, backend)).resolves.toEqual({
      type: "result",
      id: 7,
      ok: true,
      value: { revision: 3 },
    });
    expect(invokeJson).toHaveBeenCalledWith(
      JSON.stringify({
        command: "pose_solve",
        args: { request: { hard: [] } },
      }),
    );
  });

  it("listener登録と初期化の後にreadyを1件だけ通知する", async () => {
    const messages: CoreWorkerResponse[] = [];
    const registered: string[] = [];
    const scope = {
      addEventListener(type: string): void {
        registered.push(type);
      },
      postMessage(message: CoreWorkerResponse): void {
        expect(registered).toEqual(["message", "messageerror"]);
        messages.push(message);
      },
    };

    await startCoreWorker(scope, () => undefined);

    expect(messages).toEqual([{ type: "ready" }]);
  });

  it("WASM初期化の完了後にだけbackendを作る", async () => {
    const order: string[] = [];
    const backend: Ori3WasmBackendPort = {
      invoke_json: () => "{}",
    };

    await expect(
      loadOri3WasmBackend(
        async () => {
          order.push("initialize");
        },
        () => {
          order.push("create");
          return backend;
        },
      ),
    ).resolves.toBe(backend);
    expect(order).toEqual(["initialize", "create"]);
  });

  it("初期化に失敗してもWorkerを落とさず日本語理由を通知する", async () => {
    const messages: CoreWorkerResponse[] = [];
    const scope = {
      addEventListener: vi.fn(),
      postMessage: vi.fn((message: CoreWorkerResponse) => messages.push(message)),
    };

    await startCoreWorker(scope, () => {
      throw new Error("WASMを読めませんでした");
    });

    expect(messages).toEqual([
      {
        type: "initialization-error",
        error:
          "Web版の計算Workerを準備できませんでした。WASMを読めませんでした",
      },
    ]);
  });

  it("messageerrorと不正requestをfatal-errorで通知する", async () => {
    const target = testScope();
    await startCoreWorker(target.scope);
    target.messages.length = 0;

    target.send(null);
    target.messageError();

    expect(target.messages).toEqual([
      {
        type: "fatal-error",
        error:
          "Web版の計算Workerが正しい要求を受け取れませんでした。ページを読み直して、もう一度お試しください。",
      },
      {
        type: "fatal-error",
        error:
          "Web版の計算Workerが正しい要求を受け取れませんでした。ページを読み直して、もう一度お試しください。",
      },
    ]);
  });

  it("非同期backendでも受信順を保ち、queueをpoisonしない", async () => {
    let finishFirst: ((value: string) => void) | undefined;
    const invokeJson = vi.fn((requestJson: string): string | Promise<string> => {
      const request = JSON.parse(requestJson) as { command: string };
      if (request.command === "document_new") {
        return new Promise<string>((resolve) => {
          finishFirst = resolve;
        });
      }
      return '{"order":2}';
    });
    initializeCoreWorker({ invoke_json: invokeJson });
    const target = testScope();
    await startCoreWorker(target.scope);
    target.messages.length = 0;

    target.send({ type: "invoke", id: 1, command: "document_new" });
    target.send({ type: "invoke", id: 2, command: "edit_undo" });
    await flushPromises();
    expect(invokeJson).toHaveBeenCalledTimes(1);

    finishFirst?.('{"order":1}');
    await vi.waitFor(() => {
      expect(target.messages).toHaveLength(2);
    });

    expect(invokeJson).toHaveBeenCalledTimes(2);
    expect(target.messages).toEqual([
      { type: "result", id: 1, ok: true, value: { order: 1 } },
      { type: "result", id: 2, ok: true, value: { order: 2 } },
    ]);
  });
});
