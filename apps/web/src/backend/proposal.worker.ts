import type {
  ProposalWorkerRequest,
  ProposalWorkerResponse,
} from "./proposalJobRegistry";
import initOri3Web, {
  Ori3WasmBackend,
} from "./generated/ori3-web/ori3_web.js";

export interface ProposalWasmBackendPort {
  invoke_json(requestJson: string): string | Promise<string>;
}

interface ProposalWorkerScope {
  addEventListener(
    type: "message" | "messageerror",
    listener: (event: MessageEvent<unknown>) => void,
  ): void;
  postMessage(message: ProposalWorkerResponse): void;
}

export type ProposalWorkerInitializer = () => void | Promise<void>;

interface PreparedProposal {
  paper_w: number;
  paper_h: number;
  packings: unknown[];
}

interface GeneratedProposalCandidate {
  candidate: unknown | null;
  error: string | null;
}

type ProgressPublisher = (
  response: Extract<ProposalWorkerResponse, { type: "progress" }>,
) => void;

let wasmBackend: ProposalWasmBackendPort | undefined;

export function initializeProposalWorker(
  backend: ProposalWasmBackendPort,
): void {
  wasmBackend = backend;
}

export async function loadProposalWasmBackend(
  initialize: () => Promise<unknown> = initOri3Web,
  create: () => ProposalWasmBackendPort = () => new Ori3WasmBackend(),
): Promise<ProposalWasmBackendPort> {
  await initialize();
  return create();
}

export async function connectProposalWasmBackend(): Promise<void> {
  initializeProposalWorker(await loadProposalWasmBackend());
}

function unavailable(request: ProposalWorkerRequest): string {
  return `Web版の「${request.command}」（提案計算 ${request.jobId}）は計算処理を利用できません。`;
}

function objectValue(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null
    ? (value as Record<string, unknown>)
    : null;
}

function isProposalWorkerRequest(value: unknown): value is ProposalWorkerRequest {
  const record = objectValue(value);
  return (
    record?.type === "invoke" &&
    typeof record.id === "number" &&
    Number.isSafeInteger(record.id) &&
    typeof record.jobId === "string" &&
    record.jobId.length > 0 &&
    record.command === "proposal_generate"
  );
}

function errorText(reason: unknown, request: ProposalWorkerRequest): string {
  if (typeof reason === "string" && reason.length > 0) return reason;
  if (reason instanceof Error && reason.message.length > 0) {
    return `提案の計算に失敗しました。${reason.message}`;
  }
  return unavailable(request);
}

async function invokeRust(
  backend: ProposalWasmBackendPort,
  command: string,
  args: Record<string, unknown>,
): Promise<unknown> {
  const resultJson = await backend.invoke_json(
    JSON.stringify({ command, args }),
  );
  return JSON.parse(resultJson) as unknown;
}

function preparedProposal(value: unknown): PreparedProposal {
  const record = objectValue(value);
  if (
    !record ||
    typeof record.paper_w !== "number" ||
    !Number.isFinite(record.paper_w) ||
    typeof record.paper_h !== "number" ||
    !Number.isFinite(record.paper_h) ||
    !Array.isArray(record.packings)
  ) {
    throw "提案の充填結果が正しい形ではありません。";
  }
  return {
    paper_w: record.paper_w,
    paper_h: record.paper_h,
    packings: record.packings,
  };
}

function generatedCandidate(value: unknown): GeneratedProposalCandidate {
  const record = objectValue(value);
  if (!record) throw "提案候補の生成結果が正しい形ではありません。";
  const candidate = record.candidate ?? null;
  const error = record.error ?? null;
  if (
    (candidate === null) === (error === null) ||
    (error !== null && typeof error !== "string")
  ) {
    throw "提案候補の生成結果が正しい形ではありません。";
  }
  return { candidate, error };
}

function generationArgs(request: ProposalWorkerRequest): {
  skeleton: unknown;
  paper: unknown;
  seed: unknown;
  withFoldPlan: boolean;
} {
  const args = objectValue(request.args);
  if (
    !args ||
    !("skeleton" in args) ||
    !("paper" in args) ||
    !("seed" in args) ||
    typeof args.withFoldPlan !== "boolean"
  ) {
    throw "proposal_generate 引数を読み取れません。";
  }
  return {
    skeleton: args.skeleton,
    paper: args.paper,
    seed: args.seed,
    withFoldPlan: args.withFoldPlan,
  };
}

function publishProgress(
  publish: ProgressPublisher,
  jobId: string,
  done: number,
  total: number,
  phase: "Generating" | "Verifying",
): void {
  publish({
    type: "progress",
    jobId,
    snapshot: { job_id: jobId, done, total, phase },
  });
}

export async function handleProposalWorkerRequest(
  request: ProposalWorkerRequest,
  backend: ProposalWasmBackendPort | undefined = wasmBackend,
  publish: ProgressPublisher = () => undefined,
): Promise<ProposalWorkerResponse> {
  if (!backend) {
    return {
      type: "result",
      id: request.id,
      ok: false,
      error: unavailable(request),
    };
  }

  try {
    const args = generationArgs(request);
    const prepared = preparedProposal(
      await invokeRust(backend, "__web_proposal_prepare", {
        skeleton: args.skeleton,
        paper: args.paper,
        seed: args.seed,
      }),
    );
    const total = prepared.packings.length;
    let done = 0;
    let phase: "Generating" | "Verifying" = "Generating";
    let lastGenerationError: string | null = null;
    const candidates: unknown[] = [];
    publishProgress(publish, request.jobId, done, total, phase);

    for (const packing of prepared.packings) {
      const generated = generatedCandidate(
        await invokeRust(backend, "__web_proposal_generate_candidate", {
          skeleton: args.skeleton,
          packing,
          paperW: prepared.paper_w,
          paperH: prepared.paper_h,
        }),
      );
      if (generated.candidate === null) {
        lastGenerationError = generated.error;
      } else if (args.withFoldPlan) {
        if (phase === "Generating") {
          phase = "Verifying";
          publishProgress(publish, request.jobId, done, total, phase);
        }
        candidates.push(
          await invokeRust(backend, "__web_proposal_verify_candidate", {
            skeleton: args.skeleton,
            paper: args.paper,
            packing,
            candidate: generated.candidate,
          }),
        );
      } else {
        candidates.push(generated.candidate);
      }
      done += 1;
      publishProgress(publish, request.jobId, done, total, phase);
    }

    if (candidates.length === 0) {
      throw (
        lastGenerationError ??
        "この骨格を紙の上に配置できませんでした(角を減らすか短くしてみてください)"
      );
    }
    return {
      type: "result",
      id: request.id,
      ok: true,
      value: { job_id: request.jobId, candidates },
    };
  } catch (reason) {
    return {
      type: "result",
      id: request.id,
      ok: false,
      error: errorText(reason, request),
    };
  }
}

function initializationError(reason: unknown): string {
  const detail =
    typeof reason === "string"
      ? reason
      : reason instanceof Error
        ? reason.message
        : "原因を読み取れませんでした";
  return `Web版の提案Workerを準備できませんでした。${detail}`;
}

const PROTOCOL_ERROR =
  "Web版の提案Workerが正しい要求を受け取れませんでした。もう一度お試しください。";

function postFatal(scope: ProposalWorkerScope): void {
  try {
    scope.postMessage({ type: "fatal-error", error: PROTOCOL_ERROR });
  } catch {
    return;
  }
}

export async function startProposalWorker(
  scope: ProposalWorkerScope,
  initialize: ProposalWorkerInitializer = () => undefined,
): Promise<void> {
  let queue = Promise.resolve();
  scope.addEventListener("message", (event) => {
    if (!isProposalWorkerRequest(event.data)) {
      postFatal(scope);
      return;
    }
    const request = event.data;
    queue = queue
      .then(async () => {
        const response = await handleProposalWorkerRequest(
          request,
          undefined,
          (progress) => scope.postMessage(progress),
        );
        scope.postMessage(response);
      })
      .catch((reason: unknown) => {
        scope.postMessage({
          type: "fatal-error",
          error: initializationError(reason),
        });
      });
  });
  scope.addEventListener("messageerror", () => {
    postFatal(scope);
  });
  try {
    await initialize();
    scope.postMessage({ type: "ready" });
  } catch (reason) {
    scope.postMessage({
      type: "initialization-error",
      error: initializationError(reason),
    });
  }
}

const workerScope = globalThis as unknown as ProposalWorkerScope;
if (
  typeof document === "undefined" &&
  typeof globalThis.addEventListener === "function" &&
  typeof globalThis.postMessage === "function"
) {
  void startProposalWorker(workerScope, connectProposalWasmBackend);
}
