import { describe, expect, it, vi } from "vitest";
import type {
  ProposalWorkerRequest,
  ProposalWorkerResponse,
} from "./proposalJobRegistry";
import {
  handleProposalWorkerRequest,
  loadProposalWasmBackend,
  startProposalWorker,
  type ProposalWasmBackendPort,
} from "./proposal.worker";

const REQUEST: ProposalWorkerRequest = {
  type: "invoke",
  id: 23,
  jobId: "bird-base-job-23",
  command: "proposal_generate",
  args: {
    jobId: "bird-base-job-23",
    skeleton: { nodes: [{ id: 0, parent: null, length: 0 }] },
    paper: { width_mm: 150, height_mm: 150 },
    seed: 1,
    withFoldPlan: true,
  },
};

function command(requestJson: string): string {
  return (JSON.parse(requestJson) as { command: string }).command;
}

describe("proposal WorkerのWASM RPC", () => {
  it("未接続を成功に見せず、要求ID・command・job IDを保つ", async () => {
    await expect(handleProposalWorkerRequest(REQUEST)).resolves.toEqual({
      type: "result",
      id: 23,
      ok: false,
      error:
        "Web版の「proposal_generate」（提案計算 bird-base-job-23）は計算処理を利用できません。",
    });
  });

  it("prepare→候補生成→折り方検証をRustへ渡し、実境界で単調な進捗を出す", async () => {
    const calls: string[] = [];
    let generated = 0;
    const backend: ProposalWasmBackendPort = {
      invoke_json(requestJson): string {
        const name = command(requestJson);
        calls.push(name);
        if (name === "__web_proposal_prepare") {
          return JSON.stringify({
            paper_w: 1,
            paper_h: 1,
            packings: [{ scale: 1 }, { scale: 0.8 }],
          });
        }
        if (name === "__web_proposal_generate_candidate") {
          generated += 1;
          return generated === 1
            ? JSON.stringify({
                candidate: { cp: { marker: "bird-base" }, fold_plan: null },
                error: null,
              })
            : JSON.stringify({
                candidate: null,
                error: "2件目だけ配置できませんでした",
              });
        }
        if (name === "__web_proposal_verify_candidate") {
          return JSON.stringify({
            cp: { marker: "bird-base" },
            fold_plan: { status: "checked_to_finish" },
          });
        }
        throw new Error(`unexpected command: ${name}`);
      },
    };
    const progress: ProposalWorkerResponse[] = [];

    await expect(
      handleProposalWorkerRequest(REQUEST, backend, (message) =>
        progress.push(message),
      ),
    ).resolves.toEqual({
      type: "result",
      id: 23,
      ok: true,
      value: {
        job_id: "bird-base-job-23",
        candidates: [
          {
            cp: { marker: "bird-base" },
            fold_plan: { status: "checked_to_finish" },
          },
        ],
      },
    });
    expect(calls).toEqual([
      "__web_proposal_prepare",
      "__web_proposal_generate_candidate",
      "__web_proposal_verify_candidate",
      "__web_proposal_generate_candidate",
    ]);
    expect(progress).toEqual([
      {
        type: "progress",
        jobId: "bird-base-job-23",
        snapshot: {
          job_id: "bird-base-job-23",
          done: 0,
          total: 2,
          phase: "Generating",
        },
      },
      {
        type: "progress",
        jobId: "bird-base-job-23",
        snapshot: {
          job_id: "bird-base-job-23",
          done: 0,
          total: 2,
          phase: "Verifying",
        },
      },
      {
        type: "progress",
        jobId: "bird-base-job-23",
        snapshot: {
          job_id: "bird-base-job-23",
          done: 1,
          total: 2,
          phase: "Verifying",
        },
      },
      {
        type: "progress",
        jobId: "bird-base-job-23",
        snapshot: {
          job_id: "bird-base-job-23",
          done: 2,
          total: 2,
          phase: "Verifying",
        },
      },
    ]);
  });

  it("watchdog失敗時は既に作った候補も返さない", async () => {
    const backend: ProposalWasmBackendPort = {
      invoke_json(requestJson): string {
        switch (command(requestJson)) {
          case "__web_proposal_prepare":
            return JSON.stringify({
              paper_w: 1,
              paper_h: 1,
              packings: [{ scale: 1 }],
            });
          case "__web_proposal_generate_candidate":
            return JSON.stringify({ candidate: { cp: {} }, error: null });
          case "__web_proposal_verify_candidate":
            throw "提案の探索が見張り時間を超えたため中断しました(途中の候補は返していません)";
          default:
            throw "unexpected";
        }
      },
    };

    await expect(handleProposalWorkerRequest(REQUEST, backend)).resolves.toEqual({
      type: "result",
      id: 23,
      ok: false,
      error:
        "提案の探索が見張り時間を超えたため中断しました(途中の候補は返していません)",
    });
  });

  it("折り方なし指定はverifyを呼ばずGeneratingのまま候補を返す", async () => {
    const calls: string[] = [];
    const backend: ProposalWasmBackendPort = {
      invoke_json(requestJson): string {
        const name = command(requestJson);
        calls.push(name);
        if (name === "__web_proposal_prepare") {
          return JSON.stringify({
            paper_w: 1,
            paper_h: 1,
            packings: [{ scale: 1 }],
          });
        }
        if (name === "__web_proposal_generate_candidate") {
          return JSON.stringify({
            candidate: { cp: { marker: "bird-base" }, fold_plan: null },
            error: null,
          });
        }
        throw new Error(`verifyを呼んだ: ${name}`);
      },
    };
    const progress: ProposalWorkerResponse[] = [];
    const request: ProposalWorkerRequest = {
      ...REQUEST,
      args: { ...REQUEST.args, withFoldPlan: false },
    };

    await expect(
      handleProposalWorkerRequest(request, backend, (message) =>
        progress.push(message),
      ),
    ).resolves.toEqual({
      type: "result",
      id: 23,
      ok: true,
      value: {
        job_id: "bird-base-job-23",
        candidates: [{ cp: { marker: "bird-base" }, fold_plan: null }],
      },
    });
    expect(calls).toEqual([
      "__web_proposal_prepare",
      "__web_proposal_generate_candidate",
    ]);
    expect(
      progress.map((message) =>
        message.type === "progress" ? message.snapshot : null,
      ),
    ).toEqual([
      {
        job_id: "bird-base-job-23",
        done: 0,
        total: 1,
        phase: "Generating",
      },
      {
        job_id: "bird-base-job-23",
        done: 1,
        total: 1,
        phase: "Generating",
      },
    ]);
  });

  it("全候補の生成失敗も全件を数え、最後のdesktop生成errorを返す", async () => {
    let generated = 0;
    const backend: ProposalWasmBackendPort = {
      invoke_json(requestJson): string {
        const name = command(requestJson);
        if (name === "__web_proposal_prepare") {
          return JSON.stringify({
            paper_w: 1,
            paper_h: 1,
            packings: [{ scale: 1 }, { scale: 0.8 }],
          });
        }
        if (name === "__web_proposal_generate_candidate") {
          generated += 1;
          return JSON.stringify({
            candidate: null,
            error: `${generated}件目を生成できませんでした`,
          });
        }
        throw new Error(`unexpected command: ${name}`);
      },
    };
    const progress: ProposalWorkerResponse[] = [];

    await expect(
      handleProposalWorkerRequest(REQUEST, backend, (message) =>
        progress.push(message),
      ),
    ).resolves.toEqual({
      type: "result",
      id: 23,
      ok: false,
      error: "2件目を生成できませんでした",
    });
    expect(
      progress.map((message) =>
        message.type === "progress" ? message.snapshot.done : null,
      ),
    ).toEqual([0, 1, 2]);
    expect(
      progress.map((message) =>
        message.type === "progress" ? message.snapshot.total : null,
      ),
    ).toEqual([2, 2, 2]);
  });

  it("listener登録とWASM初期化の後にreadyを通知する", async () => {
    const messages: ProposalWorkerResponse[] = [];
    const registered: string[] = [];
    const scope = {
      addEventListener(type: string): void {
        registered.push(type);
      },
      postMessage(message: ProposalWorkerResponse): void {
        expect(registered).toEqual(["message", "messageerror"]);
        messages.push(message);
      },
    };

    await startProposalWorker(scope);

    expect(messages).toEqual([{ type: "ready" }]);
  });

  it("WASM初期化の完了後にだけbackendを作る", async () => {
    const order: string[] = [];
    const backend: ProposalWasmBackendPort = { invoke_json: () => "{}" };
    await expect(
      loadProposalWasmBackend(
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

  it("不正request・messageerror・初期化失敗を明示する", async () => {
    const messages: ProposalWorkerResponse[] = [];
    let messageListener: ((event: MessageEvent<unknown>) => void) | undefined;
    let messageErrorListener: ((event: MessageEvent<unknown>) => void) | undefined;
    const scope = {
      addEventListener(
        type: "message" | "messageerror",
        listener: (event: MessageEvent<unknown>) => void,
      ): void {
        if (type === "message") messageListener = listener;
        else messageErrorListener = listener;
      },
      postMessage(message: ProposalWorkerResponse): void {
        messages.push(message);
      },
    };
    await startProposalWorker(scope);
    messages.length = 0;

    messageListener?.({ data: null } as MessageEvent<unknown>);
    messageErrorListener?.({ data: null } as MessageEvent<unknown>);
    expect(messages).toEqual([
      {
        type: "fatal-error",
        error:
          "Web版の提案Workerが正しい要求を受け取れませんでした。もう一度お試しください。",
      },
      {
        type: "fatal-error",
        error:
          "Web版の提案Workerが正しい要求を受け取れませんでした。もう一度お試しください。",
      },
    ]);

    const failed = { addEventListener: vi.fn(), postMessage: vi.fn() };
    await startProposalWorker(failed, () => {
      throw new Error("WASMを読めませんでした");
    });
    expect(failed.postMessage).toHaveBeenCalledWith({
      type: "initialization-error",
      error:
        "Web版の提案Workerを準備できませんでした。WASMを読めませんでした",
    });
  });
});
