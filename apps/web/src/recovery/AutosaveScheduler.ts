import type { RecoveryCandidateSummary } from "./WebRecoveryRepository";
import type { SavedDocumentSource } from "./savedDocument";

export interface AutosaveRepositoryPort {
  saveCheckpoint(
    source: SavedDocumentSource,
    options: {
      candidateId: number | null;
      documentPath: string | null;
    },
  ): Promise<RecoveryCandidateSummary>;
  clearCandidateAfterExplicitSaveSucceeded(candidateId: number): Promise<void>;
}

export interface AutosaveClock {
  setTimeout(callback: () => void, delayMs: number): unknown;
  clearTimeout(handle: unknown): void;
  setInterval(callback: () => void, delayMs: number): unknown;
  clearInterval(handle: unknown): void;
}

export interface AutosaveSchedulerOptions {
  repository: AutosaveRepositoryPort;
  getSnapshot(): SavedDocumentSource | null | Promise<SavedDocumentSource | null>;
  getDocumentPath?: () => string | null;
  onError?: (message: string) => void;
  clock?: AutosaveClock;
  debounceMs?: number;
  checkpointMs?: number;
}

const DEFAULT_DEBOUNCE_MS = 1_000;
export const AUTOSAVE_CHECKPOINT_MS = 30_000;

const browserClock: AutosaveClock = {
  setTimeout: (callback, delayMs) => globalThis.setTimeout(callback, delayMs),
  clearTimeout: (handle) =>
    globalThis.clearTimeout(handle as ReturnType<typeof globalThis.setTimeout>),
  setInterval: (callback, delayMs) => globalThis.setInterval(callback, delayMs),
  clearInterval: (handle) =>
    globalThis.clearInterval(handle as ReturnType<typeof globalThis.setInterval>),
};

function positiveDelay(value: number, label: string): number {
  if (!Number.isFinite(value) || value <= 0) {
    throw new Error(`${label}は正の有限値で指定してください。`);
  }
  return value;
}

function errorMessage(error: unknown): string {
  if (error instanceof Error && error.message.length > 0) return error.message;
  return "ブラウザの自動保存に失敗しました。作品は変更していません。";
}

/**
 * 変更が止まった後のdebounceと、変更が続く間の30秒checkpointを両立する。
 * `dispose()` はtimerを止めるだけで、復旧候補を削除しない。
 */
export class AutosaveScheduler {
  private readonly repository: AutosaveRepositoryPort;
  private readonly getSnapshot: () =>
    | SavedDocumentSource
    | null
    | Promise<SavedDocumentSource | null>;
  private readonly getDocumentPath: () => string | null;
  private readonly onError: (message: string) => void;
  private readonly clock: AutosaveClock;
  private readonly debounceMs: number;
  private readonly checkpointMs: number;
  private readonly checkpointHandle: unknown;
  private debounceHandle: unknown | null = null;
  private currentCandidateId: number | null = null;
  private restoredSourceCandidateId: number | null = null;
  private documentGeneration = 0;
  private revision = 0;
  private savedRevision = 0;
  private disposed = false;
  private queue: Promise<void> = Promise.resolve();

  constructor(options: AutosaveSchedulerOptions) {
    this.repository = options.repository;
    this.getSnapshot = options.getSnapshot;
    this.getDocumentPath = options.getDocumentPath ?? (() => null);
    this.onError = options.onError ?? (() => undefined);
    this.clock = options.clock ?? browserClock;
    this.debounceMs = positiveDelay(
      options.debounceMs ?? DEFAULT_DEBOUNCE_MS,
      "自動保存の待ち時間",
    );
    this.checkpointMs = positiveDelay(
      options.checkpointMs ?? AUTOSAVE_CHECKPOINT_MS,
      "自動保存の定期確認時間",
    );
    this.checkpointHandle = this.clock.setInterval(() => {
      void this.flushNow().catch(() => undefined);
    }, this.checkpointMs);
  }

  markChanged(): void {
    if (this.disposed) return;
    this.revision += 1;
    if (this.debounceHandle !== null) {
      this.clock.clearTimeout(this.debounceHandle);
    }
    this.debounceHandle = this.clock.setTimeout(() => {
      this.debounceHandle = null;
      void this.flushNow().catch(() => undefined);
    }, this.debounceMs);
  }

  /** 復元元は保持したまま、以後の編集を必ず別のactive候補へ保存する。 */
  associateRestoredCandidate(candidateId: number): void {
    if (!Number.isSafeInteger(candidateId) || candidateId < 1) {
      throw new Error("復旧候補の番号が安全な整数の範囲を超えています。");
    }
    this.beginDocumentSession(candidateId);
  }

  /** 新規作成・ファイルを開く成功時は候補を消さず、現在作品との関連だけ外す。 */
  releaseCandidateAssociations(): void {
    this.beginDocumentSession(null);
  }

  /** 現在のtabで扱っている候補は、同じtabの復旧一覧へ重ねて出さない。 */
  associatedCandidateIds(): readonly number[] {
    return [...new Set(
      [this.restoredSourceCandidateId, this.currentCandidateId].filter(
        (candidateId): candidateId is number => candidateId !== null,
      ),
    )];
  }

  /** 検査と明示的な同期点のため、待てる形で現在の変更を保存する。 */
  flushNow(): Promise<void> {
    if (this.disposed || this.revision <= this.savedRevision) {
      return this.queue;
    }
    const targetRevision = this.revision;
    const targetGeneration = this.documentGeneration;
    const operation = this.queue.then(async () => {
      if (targetGeneration !== this.documentGeneration) return;
      if (targetRevision <= this.savedRevision) return;
      const snapshot = await this.getSnapshot();
      if (targetGeneration !== this.documentGeneration) return;
      if (snapshot === null) {
        this.savedRevision = Math.max(this.savedRevision, targetRevision);
        return;
      }
      const candidate = await this.repository.saveCheckpoint(snapshot, {
        candidateId: this.currentCandidateId,
        documentPath: this.getDocumentPath(),
      });
      if (targetGeneration !== this.documentGeneration) return;
      this.currentCandidateId = candidate.candidate_id;
      this.savedRevision = Math.max(this.savedRevision, targetRevision);
    });
    this.queue = operation.catch((error: unknown) => {
      this.onError(errorMessage(error));
    });
    return operation;
  }

  /**
   * 作品の明示保存成功をautosave queueへ直列化する。
   * 呼出時点までをcleanとし、当該候補だけを消す。後発変更は新候補へ保存する。
   */
  markExplicitSaveSucceeded(): Promise<void> {
    if (this.disposed) return this.queue;
    const cleanRevision = this.revision;
    this.savedRevision = Math.max(this.savedRevision, cleanRevision);
    if (this.debounceHandle !== null) {
      this.clock.clearTimeout(this.debounceHandle);
      this.debounceHandle = null;
    }
    const targetGeneration = this.documentGeneration;
    const restoredAtSave = this.restoredSourceCandidateId;
    const operation = this.queue.then(async () => {
      if (targetGeneration !== this.documentGeneration) return;
      const candidateIds = [...new Set(
        [restoredAtSave, this.currentCandidateId].filter(
          (candidateId): candidateId is number => candidateId !== null,
        ),
      )];
      const errors: string[] = [];
      for (const candidateId of candidateIds) {
        try {
          await this.repository.clearCandidateAfterExplicitSaveSucceeded(
            candidateId,
          );
          if (this.restoredSourceCandidateId === candidateId) {
            this.restoredSourceCandidateId = null;
          }
          if (this.currentCandidateId === candidateId) {
            this.currentCandidateId = null;
          }
        } catch (error) {
          errors.push(errorMessage(error));
        }
      }
      if (errors.length > 0) {
        throw new Error(
          `作品は保存しましたが、復旧候補を片付けられませんでした: ${errors.join(" / ")}`,
        );
      }
    });
    this.queue = operation.catch((error: unknown) => {
      this.onError(errorMessage(error));
    });
    return operation;
  }

  private beginDocumentSession(
    restoredSourceCandidateId: number | null,
  ): void {
    this.documentGeneration += 1;
    if (this.debounceHandle !== null) {
      this.clock.clearTimeout(this.debounceHandle);
      this.debounceHandle = null;
    }
    this.revision = 0;
    this.savedRevision = 0;
    this.restoredSourceCandidateId = restoredSourceCandidateId;
    this.currentCandidateId = null;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    if (this.debounceHandle !== null) {
      this.clock.clearTimeout(this.debounceHandle);
      this.debounceHandle = null;
    }
    this.clock.clearInterval(this.checkpointHandle);
    // タブ終了・component破棄では候補を消さない。明示保存成功/個別破棄だけが削除する。
  }
}
