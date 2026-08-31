import type {
  BackendCommandName,
  BackendInvokeArgs,
} from "../../../desktop/src/ipc/runtime";
import type {
  ProposalPhase,
  ProposalJobResult,
  ProposalProgressSnapshot,
} from "../../../desktop/src/lib/types";
import type { WorkerClock } from "./coreWorkerClient";

export const PROPOSAL_WORKER_COMMANDS = [
  "proposal_generate",
  "proposal_progress",
  "proposal_control",
] as const satisfies readonly BackendCommandName[];

export type ProposalWorkerCommand =
  (typeof PROPOSAL_WORKER_COMMANDS)[number];

export interface ProposalWorkerRequest {
  type: "invoke";
  id: number;
  jobId: string;
  command: "proposal_generate";
  args?: BackendInvokeArgs;
}

export type ProposalWorkerResponse =
  | { type: "ready" }
  | { type: "initialization-error"; error: string }
  | { type: "fatal-error"; error: string }
  | {
      type: "progress";
      jobId: string;
      snapshot: ProposalProgressSnapshot;
    }
  | { type: "result"; id: number; ok: true; value: unknown }
  | { type: "result"; id: number; ok: false; error: string };

interface PendingProposalRequest {
  resolve(value: unknown): void;
  reject(reason: string): void;
}

interface ProposalWorkerSession {
  worker: Worker;
  ready: Promise<void>;
  resolveReady(): void;
  rejectReady(reason: string): void;
  readySettled: boolean;
  readyTimer: unknown;
  failed: boolean;
  started: boolean;
  pending: Map<number, PendingProposalRequest>;
}

export interface ProposalJobRegistryOptions {
  readyTimeoutMs?: number;
  clock?: WorkerClock;
}

export interface ProposalJobRegistry {
  invoke<T>(command: BackendCommandName, args?: BackendInvokeArgs): Promise<T>;
  /** job専用WorkerのRPC受付準備を待つ。 */
  ready(jobId: string): Promise<void>;
  disposeJob(jobId: string): void;
  dispose(): void;
}

export type ProposalWorkerFactory = (jobId: string) => Worker;

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
const PROPOSAL_PHASES = new Set<ProposalPhase>([
  "Queued",
  "Generating",
  "Verifying",
  "Finished",
  "Cancelled",
  "Failed",
]);

function defaultProposalWorkerFactory(jobId: string): Worker {
  return new Worker(new URL("./proposal.worker.ts", import.meta.url), {
    name: `ori3-proposal-${jobId}`,
    type: "module",
  });
}

function isProposalWorkerCommand(
  command: BackendCommandName,
): command is ProposalWorkerCommand {
  return (PROPOSAL_WORKER_COMMANDS as readonly BackendCommandName[]).includes(
    command,
  );
}

function objectValue(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null
    ? (value as Record<string, unknown>)
    : null;
}

function isProgressSnapshot(value: unknown): value is ProposalProgressSnapshot {
  const record = objectValue(value);
  return (
    typeof record?.job_id === "string" &&
    typeof record.done === "number" &&
    Number.isSafeInteger(record.done) &&
    record.done >= 0 &&
    typeof record.total === "number" &&
    Number.isSafeInteger(record.total) &&
    record.total >= 0 &&
    record.total <= 4 &&
    record.done <= record.total &&
    typeof record.phase === "string" &&
    PROPOSAL_PHASES.has(record.phase as ProposalPhase)
  );
}

function isProposalJobResult(
  value: unknown,
  jobId: string,
): value is ProposalJobResult {
  const record = objectValue(value);
  return (
    record?.job_id === jobId &&
    Array.isArray(record.candidates) &&
    record.candidates.length > 0 &&
    record.candidates.length <= 4
  );
}

function isProposalWorkerResponse(
  value: unknown,
): value is ProposalWorkerResponse {
  const record = objectValue(value);
  if (!record || typeof record.type !== "string") return false;
  if (record.type === "ready") return true;
  if (
    record.type === "initialization-error" ||
    record.type === "fatal-error"
  ) {
    return typeof record.error === "string";
  }
  if (record.type === "progress") {
    return (
      typeof record.jobId === "string" &&
      isProgressSnapshot(record.snapshot)
    );
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

export function proposalJobId(
  command: ProposalWorkerCommand,
  args: BackendInvokeArgs | undefined,
): string | null {
  const record = objectValue(args);
  if (!record) return null;
  if (command === "proposal_control") {
    const operation = objectValue(record.operation);
    return typeof operation?.job_id === "string" && operation.job_id.length > 0
      ? operation.job_id
      : null;
  }
  return typeof record.jobId === "string" && record.jobId.length > 0
    ? record.jobId
    : null;
}

function missingJobId(command: ProposalWorkerCommand): string {
  if (command === "proposal_generate") {
    return "Web版の「proposal_generate」は提案計算用Workerとの接続を準備中のため、まだ利用できません。";
  }
  return `Web版の「${command}」を始められませんでした。提案計算を識別する番号がありません。`;
}

function workerFailure(jobId: string): string {
  return `Web版の提案計算（${jobId}）のWorkerを利用できません。もう一度お試しください。`;
}

function workerTimeout(jobId: string): string {
  return `Web版の提案計算（${jobId}）のWorker準備に時間がかかりすぎました。もう一度お試しください。`;
}

function protocolFailure(jobId: string): string {
  return `Web版の提案計算（${jobId}）から正しい応答を受け取れませんでした。もう一度お試しください。`;
}

function duplicateJob(jobId: string): string {
  return `同じ提案job IDは同時に使えません: ${jobId}`;
}

function unknownJob(command: ProposalWorkerCommand, jobId: string): string {
  void command;
  return `提案jobが見つかりません: ${jobId}`;
}

function cancellation(): string {
  return "提案の計算を取り消しました(途中の候補は返していません)";
}

function queuedSnapshot(jobId: string): ProposalProgressSnapshot {
  return { job_id: jobId, done: 0, total: 0, phase: "Queued" };
}

function isTerminalPhase(phase: ProposalPhase): boolean {
  return phase === "Finished" || phase === "Cancelled" || phase === "Failed";
}

function phaseRank(phase: ProposalPhase): number {
  switch (phase) {
    case "Queued":
      return 0;
    case "Generating":
      return 1;
    case "Verifying":
      return 2;
    case "Finished":
    case "Cancelled":
    case "Failed":
      return 3;
  }
}

function followsProgress(
  current: ProposalProgressSnapshot,
  next: ProposalProgressSnapshot,
): boolean {
  if (isTerminalPhase(next.phase)) return false;
  if (next.done < current.done || next.done > current.done + 1) return false;
  if (phaseRank(next.phase) < phaseRank(current.phase)) return false;
  if (current.phase === "Queued") {
    return (
      current.done === 0 &&
      current.total === 0 &&
      next.phase === "Generating" &&
      next.done === 0
    );
  }
  return next.total === current.total;
}

export function createProposalJobRegistry(
  workerFactory: ProposalWorkerFactory = defaultProposalWorkerFactory,
  options: ProposalJobRegistryOptions = {},
): ProposalJobRegistry {
  const readyTimeoutMs = options.readyTimeoutMs ?? DEFAULT_READY_TIMEOUT_MS;
  const clock = options.clock ?? SYSTEM_CLOCK;
  const sessions = new Map<string, ProposalWorkerSession>();
  const snapshots = new Map<string, ProposalProgressSnapshot>();
  let nextRequestId = 1;
  let disposed = false;

  const clearReadyTimer = (session: ProposalWorkerSession): void => {
    clock.clearTimeout(session.readyTimer);
  };

  const failSession = (
    jobId: string,
    session: ProposalWorkerSession,
    reason: string,
  ): void => {
    if (sessions.get(jobId) !== session || session.failed) return;
    session.failed = true;
    clearReadyTimer(session);
    if (!session.readySettled) {
      session.readySettled = true;
      session.rejectReady(reason);
    }
    for (const request of session.pending.values()) request.reject(reason);
    session.pending.clear();
    session.worker.terminate();
    sessions.delete(jobId);
    snapshots.delete(jobId);
  };

  const completeSession = (
    jobId: string,
    session: ProposalWorkerSession,
  ): void => {
    if (sessions.get(jobId) !== session) return;
    clearReadyTimer(session);
    session.worker.terminate();
    sessions.delete(jobId);
    snapshots.delete(jobId);
  };

  const receiveMessage = (
    jobId: string,
    session: ProposalWorkerSession,
    event: MessageEvent<unknown>,
  ): void => {
    if (sessions.get(jobId) !== session || session.failed) return;
    if (!isProposalWorkerResponse(event.data)) {
      failSession(jobId, session, protocolFailure(jobId));
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
      failSession(jobId, session, response.error);
      return;
    }
    if (response.type === "progress") {
      const current = snapshots.get(jobId) ?? queuedSnapshot(jobId);
      if (
        response.jobId !== jobId ||
        response.snapshot.job_id !== jobId ||
        !followsProgress(current, response.snapshot)
      ) {
        failSession(jobId, session, protocolFailure(jobId));
        return;
      }
      snapshots.set(jobId, response.snapshot);
      return;
    }
    const request = session.pending.get(response.id);
    if (!request) return;
    if (response.ok && !isProposalJobResult(response.value, jobId)) {
      failSession(jobId, session, protocolFailure(jobId));
      return;
    }
    session.pending.delete(response.id);
    completeSession(jobId, session);
    if (response.ok) request.resolve(response.value);
    else request.reject(response.error);
  };

  const createSession = (jobId: string): ProposalWorkerSession => {
    if (disposed) throw workerFailure(jobId);
    const existing = sessions.get(jobId);
    if (existing) return existing;

    const worker = workerFactory(jobId);
    let resolveReady = (): void => undefined;
    let rejectReady = (): void => undefined;
    const ready = new Promise<void>((resolve, reject) => {
      resolveReady = resolve;
      rejectReady = reject;
    });
    const session: ProposalWorkerSession = {
      worker,
      ready,
      resolveReady,
      rejectReady,
      readySettled: false,
      readyTimer: undefined,
      failed: false,
      started: false,
      pending: new Map(),
    };
    sessions.set(jobId, session);

    worker.addEventListener("message", (event: MessageEvent<unknown>) => {
      receiveMessage(jobId, session, event);
    });
    const fail = (): void =>
      failSession(jobId, session, workerFailure(jobId));
    worker.addEventListener("error", fail);
    worker.addEventListener("messageerror", fail);
    session.readyTimer = clock.setTimeout(() => {
      failSession(jobId, session, workerTimeout(jobId));
    }, readyTimeoutMs);
    return session;
  };

  const disposeJob = (jobId: string): void => {
    const session = sessions.get(jobId);
    if (session) failSession(jobId, session, workerFailure(jobId));
    snapshots.delete(jobId);
  };

  return {
    async invoke<T>(
      command: BackendCommandName,
      args?: BackendInvokeArgs,
    ): Promise<T> {
      if (!isProposalWorkerCommand(command)) {
        throw `Web版の「${command}」は提案Workerへ送れません。`;
      }
      const jobId = proposalJobId(command, args);
      if (jobId === null) throw missingJobId(command);

      if (command === "proposal_progress") {
        const snapshot = snapshots.get(jobId);
        if (!snapshot) return null as unknown as T;
        if (isTerminalPhase(snapshot.phase)) snapshots.delete(jobId);
        return snapshot as unknown as T;
      }

      if (command === "proposal_control") {
        const session = sessions.get(jobId);
        if (!session || !session.started) throw unknownJob(command, jobId);
        const current = snapshots.get(jobId) ?? queuedSnapshot(jobId);
        const cancelled: ProposalProgressSnapshot = {
          ...current,
          phase: "Cancelled",
        };
        failSession(jobId, session, cancellation());
        return cancelled as unknown as T;
      }

      const existing = sessions.get(jobId);
      if (existing?.started || snapshots.has(jobId)) throw duplicateJob(jobId);
      let session: ProposalWorkerSession;
      try {
        session = existing ?? createSession(jobId);
        session.started = true;
        snapshots.set(jobId, queuedSnapshot(jobId));
        await session.ready;
      } catch (reason) {
        throw typeof reason === "string" && reason.length > 0
          ? reason
          : workerFailure(jobId);
      }

      return new Promise<T>((resolve, reject) => {
        const id = nextRequestId;
        nextRequestId += 1;
        session.pending.set(id, {
          resolve: (value) => resolve(value as T),
          reject,
        });
        const request: ProposalWorkerRequest = {
          type: "invoke",
          id,
          jobId,
          command: "proposal_generate",
          ...(args === undefined ? {} : { args }),
        };
        try {
          session.worker.postMessage(request);
        } catch {
          session.pending.delete(id);
          failSession(jobId, session, workerFailure(jobId));
          reject(workerFailure(jobId));
        }
      });
    },

    ready(jobId: string): Promise<void> {
      if (jobId.length === 0) {
        return Promise.reject(missingJobId("proposal_generate"));
      }
      try {
        return createSession(jobId).ready;
      } catch (reason) {
        return Promise.reject(
          typeof reason === "string" && reason.length > 0
            ? reason
            : workerFailure(jobId),
        );
      }
    },

    disposeJob,

    dispose(): void {
      disposed = true;
      for (const jobId of [...sessions.keys()]) disposeJob(jobId);
      snapshots.clear();
    },
  };
}
