import type { BackendCommandName, BackendInvokeArgs } from "../../../desktop/src/ipc/runtime";
import type { DocumentView, RecoveryChoices, RecoveryInfo } from "../../../desktop/src/lib/types";
import type { BrowserFileToken } from "../../../desktop/src/platform/fileGateway";
import type { BrowserCurrentDocumentCoordinator } from "../backend/browserDocumentInvoker";
import type { Ori3CoreWorkerClient } from "../backend/coreWorkerClient";
import {
  BROWSER_FILE_TOKEN_REGISTRY,
  type BrowserFileTokenRegistry,
} from "../platform/browserFileTokenRegistry";
import {
  AutosaveScheduler,
  type AutosaveClock,
  type AutosaveRepositoryPort,
} from "./AutosaveScheduler";
import {
  WebRecoveryRepository,
  type RecoveryCandidateSource,
  type RecoveryCandidateSummary,
} from "./WebRecoveryRepository";
import type { SavedDocumentSource } from "./savedDocument";

export interface RecoveryCommandInvoker {
  invoke<T>(command: BackendCommandName, args?: BackendInvokeArgs): Promise<T>;
}

export interface WebRecoveryRepositoryPort extends AutosaveRepositoryPort {
  listCandidates(): Promise<RecoveryCandidateSummary[]>;
  readCandidateSource(candidateId: number): Promise<RecoveryCandidateSource>;
  discardCandidate(candidateId: number): Promise<void>;
}

interface RecoveryCorePort {
  invoke<T>(
    command: Parameters<Ori3CoreWorkerClient["invoke"]>[0],
    args?: BackendInvokeArgs,
  ): Promise<T>;
}

export interface WebRecoveryRuntimeOptions {
  currentDocument: BrowserCurrentDocumentCoordinator;
  registry?: BrowserFileTokenRegistry;
  repository?: WebRecoveryRepositoryPort;
  createRepository?: () => WebRecoveryRepositoryPort;
  onError(message: string): void;
  clock?: AutosaveClock;
  debounceMs?: number;
  checkpointMs?: number;
}

export interface WebRecoveryRuntime {
  readonly browser: RecoveryCommandInvoker;
  decorateCore(delegate: RecoveryCommandInvoker): RecoveryCommandInvoker;
  decorateMixed(delegate: RecoveryCommandInvoker): RecoveryCommandInvoker;
  flushAutosave(): Promise<void>;
  associatedCandidateIds(): readonly number[];
  dispose(): void;
}

const MUTATING_CORE_COMMANDS = new Set<BackendCommandName>([
  "edit_apply",
  "edit_apply_batch",
  "edit_undo",
  "edit_redo",
  "sequence_apply",
  "proposal_apply",
]);

function commandArgs(args: BackendInvokeArgs | undefined): Record<string, unknown> {
  if (typeof args === "object" && args !== null && !Array.isArray(args)) {
    return args as Record<string, unknown>;
  }
  throw "コマンド要求の args フィールドはobjectにしてください。";
}

function recoveryRestoreArgs(argsValue: BackendInvokeArgs | undefined): {
  accept: boolean;
  candidateId: number;
} {
  const args = commandArgs(argsValue);
  const unknown = Object.keys(args).find(
    (name) => name !== "accept" && name !== "candidateId",
  );
  if (unknown !== undefined) {
    throw `コマンド「recovery_restore」の引数を読み取れません: unknown field \`${unknown}\``;
  }
  if (typeof args.accept !== "boolean") {
    throw "コマンド「recovery_restore」の引数を読み取れません: acceptにはtrueまたはfalseを指定してください";
  }
  if (
    typeof args.candidateId !== "number" ||
    !Number.isSafeInteger(args.candidateId) ||
    args.candidateId < 1
  ) {
    throw "復旧候補の番号が安全な整数の範囲を超えています。";
  }
  return { accept: args.accept, candidateId: args.candidateId };
}

function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || "ORIGAMI3.ori3";
}

function recoveryInfo(candidate: RecoveryCandidateSummary): RecoveryInfo {
  return {
    candidate_id: candidate.candidate_id,
    autosave_path: candidate.autosave_path,
    document_path: candidate.document_path,
    saved_at_ms: candidate.saved_at_ms,
    step_count: candidate.step_count,
  };
}

export function createWebRecoveryRuntime(
  core: RecoveryCorePort,
  options: WebRecoveryRuntimeOptions,
): WebRecoveryRuntime {
  const registry = options.registry ?? BROWSER_FILE_TOKEN_REGISTRY;
  let repository = options.repository ?? null;
  const getRepository = (): WebRecoveryRepositoryPort => {
    repository ??= options.createRepository?.() ?? new WebRecoveryRepository();
    return repository;
  };
  const repositoryForAutosave: AutosaveRepositoryPort = {
    saveCheckpoint: (source, saveOptions) =>
      getRepository().saveCheckpoint(source, saveOptions),
    clearCandidateAfterExplicitSaveSucceeded: (candidateId) =>
      getRepository().clearCandidateAfterExplicitSaveSucceeded(candidateId),
  };
  const scheduler = new AutosaveScheduler({
    repository: repositoryForAutosave,
    getSnapshot: () =>
      core.invoke<SavedDocumentSource | null>("__web_recovery_snapshot", {}),
    getDocumentPath: () => {
      const token = options.currentDocument.current();
      return token === null ? null : registry.nameOf(token);
    },
    onError: options.onError,
    ...(options.clock === undefined ? {} : { clock: options.clock }),
    ...(options.debounceMs === undefined ? {} : { debounceMs: options.debounceMs }),
    ...(options.checkpointMs === undefined
      ? {}
      : { checkpointMs: options.checkpointMs }),
  });

  const stageChoices = async (): Promise<RecoveryChoices | null> => {
    const hidden = new Set(scheduler.associatedCandidateIds());
    const candidates = (await getRepository().listCandidates())
      .filter((candidate) => !hidden.has(candidate.candidate_id))
      .sort(
        (left, right) =>
          right.saved_at_ms - left.saved_at_ms ||
          right.candidate_id - left.candidate_id,
      );
    const choices = candidates.map(recoveryInfo);
    const value: RecoveryChoices | null =
      choices.length === 0
        ? null
        : { choices, overflow_count: Math.max(choices.length - 3, 0) };
    await core.invoke("__web_recovery_set_choices", { choices: value });
    return value;
  };

  const check = async <T>(): Promise<T> => {
    await stageChoices();
    return core.invoke<T>("recovery_check");
  };

  const restore = async <T>(
    argsValue: BackendInvokeArgs | undefined,
  ): Promise<T> => {
    const args = recoveryRestoreArgs(argsValue);
    if (args.accept) await scheduler.flushNow();
    const choices = await stageChoices();
    const choice = choices?.choices.find(
      (candidate) => candidate.candidate_id === args.candidateId,
    );
    if (choice === undefined) {
      throw "選んだ復旧候補は保存領域に見つかりません。";
    }

    if (!args.accept) {
      const result = await core.invoke<T>("recovery_restore", argsValue);
      await getRepository().discardCandidate(args.candidateId);
      return result;
    }

    const selected = await getRepository().readCandidateSource(args.candidateId);
    let restoredToken: BrowserFileToken | null = null;
    if (selected.candidate.document_path !== null) {
      restoredToken = registry.registerDownload(
        fileName(selected.candidate.document_path),
      );
    }
    try {
      await core.invoke("__web_recovery_restore_source", {
        candidateId: args.candidateId,
        documentPath: restoredToken,
        source: selected.source,
      });
      const result = await core.invoke<DocumentView>(
        "recovery_restore",
        argsValue,
      );
      if (restoredToken === null) options.currentDocument.clear();
      else options.currentDocument.adopt(restoredToken);
      scheduler.associateRestoredCandidate(args.candidateId);
      return result as T;
    } finally {
      if (restoredToken !== null) registry.release(restoredToken);
    }
  };

  const browser: RecoveryCommandInvoker = {
    invoke<T>(command: BackendCommandName): Promise<T> {
      if (command === "recovery_check") return check<T>();
      return Promise.reject(
        `Web版の復旧保存領域では「${command}」を処理できません。`,
      );
    },
  };

  return {
    browser,
    decorateCore(delegate): RecoveryCommandInvoker {
      return {
        async invoke<T>(
          command: BackendCommandName,
          args?: BackendInvokeArgs,
        ): Promise<T> {
          if (command === "document_new") await scheduler.flushNow();
          const result = await delegate.invoke<T>(command, args);
          if (command === "document_new") {
            scheduler.releaseCandidateAssociations();
            scheduler.markChanged();
          } else if (MUTATING_CORE_COMMANDS.has(command)) {
            scheduler.markChanged();
          }
          return result;
        },
      };
    },
    decorateMixed(delegate): RecoveryCommandInvoker {
      return {
        async invoke<T>(
          command: BackendCommandName,
          args?: BackendInvokeArgs,
        ): Promise<T> {
          if (command === "recovery_restore") return restore<T>(args);
          if (command === "document_open") await scheduler.flushNow();
          const result = await delegate.invoke<T>(command, args);
          if (command === "document_open") {
            const openedToken = options.currentDocument.current();
            if (
              openedToken !== null &&
              registry.nameOf(openedToken).toLocaleLowerCase("en-US").endsWith(".fold")
            ) {
              // FOLDはimportであり保存先を引き継がない。core側はdirty=trueかつpathなし。
              options.currentDocument.clear();
            }
            scheduler.releaseCandidateAssociations();
            scheduler.markChanged();
          } else if (command === "document_save") {
            await scheduler.markExplicitSaveSucceeded();
          }
          return result;
        },
      };
    },
    flushAutosave: () => scheduler.flushNow(),
    associatedCandidateIds: () => scheduler.associatedCandidateIds(),
    dispose: () => scheduler.dispose(),
  };
}
