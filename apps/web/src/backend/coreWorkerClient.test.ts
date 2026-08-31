import { describe, expect, it } from "vitest";
import type { CoreWorkerRequest } from "./coreWorkerProtocol";
import {
  createOri3CoreWorkerClient,
  type WorkerClock,
} from "./coreWorkerClient";

class FakeWorker {
  readonly requests: CoreWorkerRequest[] = [];
  terminated = false;
  private messageListener?: (event: MessageEvent<unknown>) => void;
  private errorListener?: () => void;
  private messageErrorListener?: () => void;

  addEventListener(type: string, listener: unknown): void {
    if (type === "message") {
      this.messageListener = listener as (event: MessageEvent<unknown>) => void;
    }
    if (type === "error") this.errorListener = listener as () => void;
    if (type === "messageerror") {
      this.messageErrorListener = listener as () => void;
    }
  }

  postMessage(request: CoreWorkerRequest): void {
    this.requests.push(request);
  }

  terminate(): void {
    this.terminated = true;
  }

  respond(response: unknown): void {
    this.messageListener?.({ data: response } as MessageEvent<unknown>);
  }

  fail(): void {
    this.errorListener?.();
  }

  failMessage(): void {
    this.messageErrorListener?.();
  }
}

class ManualClock implements WorkerClock {
  private nextId = 1;
  private readonly callbacks = new Map<number, () => void>();

  setTimeout(callback: () => void): unknown {
    const id = this.nextId;
    this.nextId += 1;
    this.callbacks.set(id, callback);
    return id;
  }

  clearTimeout(handle: unknown): void {
    if (typeof handle === "number") this.callbacks.delete(handle);
  }

  fire(): void {
    const callbacks = [...this.callbacks.values()];
    this.callbacks.clear();
    for (const callback of callbacks) callback();
  }
}

describe("単一core WorkerのRPC client", () => {
  it("ready後にrequest ID・コマンド・引数を送り、対応する結果だけを返す", async () => {
    const worker = new FakeWorker();
    const client = createOri3CoreWorkerClient(
      () => worker as unknown as Worker,
    );

    const result = client.invoke<{ revision: number }>("edit_apply", {
      op: { type: "test" },
    });
    expect(worker.requests).toEqual([]);

    worker.respond({ type: "ready" });
    await Promise.resolve();
    expect(worker.requests).toEqual([
      {
        type: "invoke",
        id: 1,
        command: "edit_apply",
        args: { op: { type: "test" } },
      },
    ]);

    worker.respond({
      type: "result",
      id: 1,
      ok: true,
      value: { revision: 2 },
    });
    await expect(result).resolves.toEqual({ revision: 2 });
  });

  it("2要求の応答が逆順でもrequest IDで正しいPromiseへ返す", async () => {
    const worker = new FakeWorker();
    const client = createOri3CoreWorkerClient(
      () => worker as unknown as Worker,
    );
    const ready = client.ready();
    worker.respond({ type: "ready" });
    await ready;

    const first = client.invoke<number>("edit_undo");
    const second = client.invoke<number>("edit_redo");
    await Promise.resolve();
    worker.respond({ type: "result", id: 2, ok: true, value: 20 });
    worker.respond({ type: "result", id: 1, ok: true, value: 10 });

    await expect(first).resolves.toBe(10);
    await expect(second).resolves.toBe(20);
  });

  it("Worker側の日本語errorをstring rejectionのまま画面へ返す", async () => {
    const worker = new FakeWorker();
    const client = createOri3CoreWorkerClient(
      () => worker as unknown as Worker,
    );
    const result = client.invoke("document_new");

    worker.respond({ type: "ready" });
    await Promise.resolve();
    worker.respond({
      type: "result",
      id: 1,
      ok: false,
      error: "Web版の「document_new」はまだ利用できません。",
    });

    await expect(result).rejects.toBe(
      "Web版の「document_new」はまだ利用できません。",
    );
  });

  it("host限定commandも同じWorkerへ送り、起動失敗文言には公開名だけを出す", async () => {
    const worker = new FakeWorker();
    const client = createOri3CoreWorkerClient(
      () => worker as unknown as Worker,
    );
    const result = client.invoke<{ path: string; content: string }>(
      "__web_document_save_prepare",
      { path: "browser-file://read/current/作品.ori3" },
    );
    worker.respond({ type: "ready" });
    await Promise.resolve();
    expect(worker.requests).toEqual([
      {
        type: "invoke",
        id: 1,
        command: "__web_document_save_prepare",
        args: { path: "browser-file://read/current/作品.ori3" },
      },
    ]);
    worker.respond({
      type: "result",
      id: 1,
      ok: true,
      value: { path: "作品.ori3", content: "{}" },
    });
    await expect(result).resolves.toEqual({
      path: "作品.ori3",
      content: "{}",
    });

    const failed = createOri3CoreWorkerClient(() => {
      throw new Error("worker unavailable");
    });
    await expect(
      failed.invoke("__web_document_export_prepare", {
        kind: "CpSvg",
        options: { include_aux: false, png_long_side: 1200 },
      }),
    ).rejects.toBe(
      "Web版の「document_export」は計算Workerを起動できないため、まだ利用できません。",
    );
  });

  it("Workerを起動できない場合も対象コマンドを明示する", async () => {
    const client = createOri3CoreWorkerClient(() => {
      throw new Error("worker unavailable");
    });

    await expect(client.invoke("sequence_replay")).rejects.toBe(
      "Web版の「sequence_replay」は計算Workerを起動できないため、まだ利用できません。",
    );
  });

  it("readyまでは要求を送らず、同じPromiseで初期化を待ち合わせる", async () => {
    const worker = new FakeWorker();
    const client = createOri3CoreWorkerClient(
      () => worker as unknown as Worker,
    );

    const firstReady = client.ready();
    const secondReady = client.ready();
    expect(worker.requests).toEqual([]);

    worker.respond({ type: "ready" });
    await expect(firstReady).resolves.toBeUndefined();
    await expect(secondReady).resolves.toBeUndefined();
  });

  it("初期化失敗を待機中要求へ返し、Workerを終端する", async () => {
    const worker = new FakeWorker();
    const client = createOri3CoreWorkerClient(
      () => worker as unknown as Worker,
    );
    const result = client.invoke("pose_solve");
    const reason =
      "Web版の計算Workerを準備できませんでした。WASMを読めませんでした。";

    worker.respond({ type: "initialization-error", error: reason });

    await expect(result).rejects.toBe(reason);
    await expect(client.invoke("edit_undo")).rejects.toBe(reason);
    expect(worker.terminated).toBe(true);
    expect(worker.requests).toEqual([]);
  });

  it("ready後のerrorでpendingを拒否し、次回invokeも即時拒否する", async () => {
    const worker = new FakeWorker();
    const client = createOri3CoreWorkerClient(
      () => worker as unknown as Worker,
    );
    const ready = client.ready();
    worker.respond({ type: "ready" });
    await ready;
    const pending = client.invoke("edit_apply", { op: { type: "test" } });
    await Promise.resolve();

    worker.fail();

    await expect(pending).rejects.toBe(
      "Web版の「edit_apply」は計算Workerを起動できないため、まだ利用できません。",
    );
    await expect(client.invoke("edit_redo")).rejects.toBe(
      "Web版の計算Workerとの通信を続けられません。ページを読み直して、もう一度お試しください。",
    );
    expect(worker.terminated).toBe(true);
  });

  it("messageerrorと不正responseをterminal failureとして扱う", async () => {
    const firstWorker = new FakeWorker();
    const firstClient = createOri3CoreWorkerClient(
      () => firstWorker as unknown as Worker,
    );
    const first = firstClient.invoke("document_new");
    firstWorker.failMessage();
    await expect(first).rejects.toBe(
      "Web版の計算Workerとの通信を続けられません。ページを読み直して、もう一度お試しください。",
    );

    const secondWorker = new FakeWorker();
    const secondClient = createOri3CoreWorkerClient(
      () => secondWorker as unknown as Worker,
    );
    const second = secondClient.invoke("document_new");
    secondWorker.respond(null);
    await expect(second).rejects.toBe(
      "Web版の計算Workerから正しい応答を受け取れませんでした。ページを読み直して、もう一度お試しください。",
    );
    expect(firstWorker.terminated).toBe(true);
    expect(secondWorker.terminated).toBe(true);
  });

  it("ready timeoutで待機中要求を拒否してWorkerを終端する", async () => {
    const worker = new FakeWorker();
    const clock = new ManualClock();
    const client = createOri3CoreWorkerClient(
      () => worker as unknown as Worker,
      { readyTimeoutMs: 25, clock },
    );
    const result = client.invoke("document_new");

    clock.fire();

    await expect(result).rejects.toBe(
      "Web版の計算Workerの準備に時間がかかりすぎました。ページを読み直して、もう一度お試しください。",
    );
    expect(worker.terminated).toBe(true);
  });
});
