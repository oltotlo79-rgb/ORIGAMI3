import { describe, expect, it } from "vitest";
import type { ProposalProgressSnapshot } from "../../../desktop/src/lib/types";
import type { WorkerClock } from "./coreWorkerClient";
import {
  createProposalJobRegistry,
  proposalJobId,
  type ProposalWorkerRequest,
} from "./proposalJobRegistry";

class FakeProposalWorker {
  readonly requests: ProposalWorkerRequest[] = [];
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

  postMessage(request: ProposalWorkerRequest): void {
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

describe("proposal job Worker registry", () => {
  it("ready後にgenerateだけをWorkerへ送り、terminal resultで破棄する", async () => {
    const worker = new FakeProposalWorker();
    const registry = createProposalJobRegistry(
      () => worker as unknown as Worker,
    );
    const result = registry.invoke("proposal_generate", {
      jobId: "job-a",
      seed: 4,
    });
    expect(worker.requests).toEqual([]);

    worker.respond({ type: "ready" });
    await Promise.resolve();
    expect(worker.requests).toEqual([
      {
        type: "invoke",
        id: 1,
        jobId: "job-a",
        command: "proposal_generate",
        args: { jobId: "job-a", seed: 4 },
      },
    ]);

    worker.respond({
      type: "result",
      id: 1,
      ok: true,
      value: { job_id: "job-a", candidates: [{}] },
    });
    await expect(result).resolves.toEqual({ job_id: "job-a", candidates: [{}] });
    await expect(
      registry.invoke("proposal_progress", { jobId: "job-a" }),
    ).resolves.toBeNull();
    expect(worker.terminated).toBe(true);
  });

  it("Workerのprogress eventを保持し、progress commandはWorkerへ送らない", async () => {
    const worker = new FakeProposalWorker();
    const registry = createProposalJobRegistry(
      () => worker as unknown as Worker,
    );
    const result = registry.invoke("proposal_generate", { jobId: "job-p" });
    worker.respond({ type: "ready" });
    await Promise.resolve();
    worker.respond({
      type: "progress",
      jobId: "job-p",
      snapshot: {
        job_id: "job-p",
        done: 0,
        total: 4,
        phase: "Generating",
      },
    });
    worker.respond({
      type: "progress",
      jobId: "job-p",
      snapshot: {
        job_id: "job-p",
        done: 1,
        total: 4,
        phase: "Generating",
      },
    });

    await expect(
      registry.invoke<ProposalProgressSnapshot | null>("proposal_progress", {
        jobId: "job-p",
      }),
    ).resolves.toEqual({
      job_id: "job-p",
      done: 1,
      total: 4,
      phase: "Generating",
    });
    expect(worker.requests).toHaveLength(1);

    worker.respond({
      type: "result",
      id: 1,
      ok: true,
      value: { job_id: "job-p", candidates: [{}] },
    });
    await expect(result).resolves.toEqual({
      job_id: "job-p",
      candidates: [{}],
    });
  });

  it("Cancelをmain側でsnapshot化し、生成Workerを直ちに終了する", async () => {
    const worker = new FakeProposalWorker();
    const registry = createProposalJobRegistry(
      () => worker as unknown as Worker,
    );
    const generate = registry.invoke("proposal_generate", { jobId: "job-c" });
    const cancelledGenerate = expect(generate).rejects.toBe(
      "提案の計算を取り消しました(途中の候補は返していません)",
    );
    worker.respond({ type: "ready" });
    await Promise.resolve();

    await expect(
      registry.invoke<ProposalProgressSnapshot>("proposal_control", {
        operation: { type: "Cancel", job_id: "job-c" },
      }),
    ).resolves.toEqual({
      job_id: "job-c",
      done: 0,
      total: 0,
      phase: "Cancelled",
    });
    await cancelledGenerate;
    expect(worker.requests).toHaveLength(1);
    expect(worker.terminated).toBe(true);
    await expect(
      registry.invoke("proposal_progress", { jobId: "job-c" }),
    ).resolves.toBeNull();
  });

  it("unknown progressはnull、unknown controlは日本語errorでWorkerを作らない", async () => {
    let factoryCalls = 0;
    const registry = createProposalJobRegistry(() => {
      factoryCalls += 1;
      return new FakeProposalWorker() as unknown as Worker;
    });

    await expect(
      registry.invoke("proposal_progress", { jobId: "missing" }),
    ).resolves.toBeNull();
    await expect(
      registry.invoke("proposal_control", {
        operation: { type: "Cancel", job_id: "missing" },
      }),
    ).rejects.toBe("提案jobが見つかりません: missing");
    expect(factoryCalls).toBe(0);
  });

  it("同じjob IDのgenerateを重ねず、jobごとにWorkerを分ける", async () => {
    const workers = new Map<string, FakeProposalWorker>();
    const registry = createProposalJobRegistry((jobId) => {
      const worker = new FakeProposalWorker();
      workers.set(jobId, worker);
      return worker as unknown as Worker;
    });
    const first = registry.invoke("proposal_generate", { jobId: "job-a" });
    const second = registry.invoke("proposal_generate", { jobId: "job-b" });
    const firstRejected = expect(first).rejects.toContain("Workerを利用できません");
    const secondRejected = expect(second).rejects.toContain("Workerを利用できません");

    await expect(
      registry.invoke("proposal_generate", { jobId: "job-a" }),
    ).rejects.toBe("同じ提案job IDは同時に使えません: job-a");
    expect(workers).toHaveLength(2);

    registry.dispose();
    await firstRejected;
    await secondRejected;
    expect(workers.get("job-a")?.terminated).toBe(true);
    expect(workers.get("job-b")?.terminated).toBe(true);
  });

  it("job Aのmessageerrorを局所化し、job Bを継続する", async () => {
    const workers = new Map<string, FakeProposalWorker>();
    const registry = createProposalJobRegistry((jobId) => {
      const worker = new FakeProposalWorker();
      workers.set(jobId, worker);
      return worker as unknown as Worker;
    });
    const first = registry.invoke("proposal_generate", { jobId: "job-a" });
    const second = registry.invoke("proposal_generate", { jobId: "job-b" });
    workers.get("job-a")?.respond({ type: "ready" });
    workers.get("job-b")?.respond({ type: "ready" });
    await Promise.resolve();
    const firstRejected = expect(first).rejects.toContain("job-a");

    workers.get("job-a")?.failMessage();
    workers.get("job-b")?.respond({
      type: "result",
      id: 2,
      ok: true,
      value: { job_id: "job-b", candidates: [{}] },
    });

    await firstRejected;
    await expect(second).resolves.toEqual({
      job_id: "job-b",
      candidates: [{}],
    });
    expect(workers.get("job-a")?.terminated).toBe(true);
    expect(workers.get("job-b")?.terminated).toBe(true);
  });

  it("done・total・phaseの後戻りと別jobのterminal結果を拒否する", async () => {
    const worker = new FakeProposalWorker();
    const registry = createProposalJobRegistry(
      () => worker as unknown as Worker,
    );
    const result = registry.invoke("proposal_generate", { jobId: "job-order" });
    worker.respond({ type: "ready" });
    await Promise.resolve();
    worker.respond({
      type: "progress",
      jobId: "job-order",
      snapshot: {
        job_id: "job-order",
        done: 0,
        total: 2,
        phase: "Generating",
      },
    });
    worker.respond({
      type: "progress",
      jobId: "job-order",
      snapshot: {
        job_id: "job-order",
        done: 0,
        total: 2,
        phase: "Verifying",
      },
    });
    worker.respond({
      type: "progress",
      jobId: "job-order",
      snapshot: {
        job_id: "job-order",
        done: 1,
        total: 2,
        phase: "Generating",
      },
    });
    await expect(result).rejects.toBe(
      "Web版の提案計算（job-order）から正しい応答を受け取れませんでした。もう一度お試しください。",
    );
    expect(worker.terminated).toBe(true);

    const resultWorker = new FakeProposalWorker();
    const resultRegistry = createProposalJobRegistry(
      () => resultWorker as unknown as Worker,
    );
    const wrongJob = resultRegistry.invoke("proposal_generate", {
      jobId: "job-right",
    });
    resultWorker.respond({ type: "ready" });
    await Promise.resolve();
    resultWorker.respond({
      type: "result",
      id: 1,
      ok: true,
      value: { job_id: "job-wrong", candidates: [{}] },
    });
    await expect(wrongJob).rejects.toContain("正しい応答");
    expect(resultWorker.terminated).toBe(true);
  });

  it("不正responseとready timeoutでpendingを回収する", async () => {
    const malformedWorker = new FakeProposalWorker();
    const malformedRegistry = createProposalJobRegistry(
      () => malformedWorker as unknown as Worker,
    );
    const malformed = malformedRegistry.invoke("proposal_generate", {
      jobId: "job-m",
    });
    malformedWorker.respond(null);
    await expect(malformed).rejects.toBe(
      "Web版の提案計算（job-m）から正しい応答を受け取れませんでした。もう一度お試しください。",
    );
    expect(malformedWorker.terminated).toBe(true);

    const timeoutWorker = new FakeProposalWorker();
    const clock = new ManualClock();
    const timeoutRegistry = createProposalJobRegistry(
      () => timeoutWorker as unknown as Worker,
      { readyTimeoutMs: 25, clock },
    );
    const timeout = timeoutRegistry.invoke("proposal_generate", {
      jobId: "job-t",
    });
    clock.fire();
    await expect(timeout).rejects.toBe(
      "Web版の提案計算（job-t）のWorker準備に時間がかかりすぎました。もう一度お試しください。",
    );
    expect(timeoutWorker.terminated).toBe(true);
  });

  it("3 commandの既存引数から同じjob IDを取り出す", () => {
    expect(proposalJobId("proposal_generate", { jobId: "a" })).toBe("a");
    expect(proposalJobId("proposal_progress", { jobId: "b" })).toBe("b");
    expect(
      proposalJobId("proposal_control", {
        operation: { type: "Cancel", job_id: "c" },
      }),
    ).toBe("c");
    expect(proposalJobId("proposal_progress", {})).toBeNull();
  });
});
