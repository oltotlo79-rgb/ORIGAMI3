import {
  createBrowserRecoveryPorts,
  type PersistenceRequestResult,
  type RecoveryCandidateMetadata,
  type RecoveryPayloadStore,
  type WebRecoveryPorts,
} from "./recoveryPorts";
import {
  parseSavedDocument,
  serializeSavedDocument,
  type SavedDocumentSnapshot,
  type SavedDocumentSource,
} from "./savedDocument";

export interface RecoveryCandidateSummary {
  candidate_id: number;
  autosave_path: string;
  document_path: string | null;
  saved_at_ms: number;
  step_count: number | null;
}

export interface SaveRecoveryCheckpointOptions {
  candidateId?: number | null;
  documentPath?: string | null;
}

export interface RecoveryCandidateSource {
  candidate: RecoveryCandidateSummary;
  source: string;
}

export interface WebRecoveryRepositoryOptions {
  ports?: WebRecoveryPorts;
  now?: () => number;
}

export function assertBrowserCandidateId(candidateId: number): void {
  if (!Number.isSafeInteger(candidateId) || candidateId < 1) {
    throw new Error("復旧候補の番号が安全な整数の範囲を超えています。");
  }
}

function summary(metadata: RecoveryCandidateMetadata): RecoveryCandidateSummary {
  return {
    candidate_id: metadata.candidate_id,
    autosave_path: `browser-recovery://candidate/${metadata.candidate_id}`,
    document_path: metadata.document_path,
    saved_at_ms: metadata.saved_at_ms,
    step_count: metadata.step_count,
  };
}

/**
 * 復旧候補のmetadataをIndexedDB、payloadをOPFSへ保存する。
 * OPFSが無いか書き込みに失敗した場合だけ、payloadもIndexedDBへ置く。
 */
export class WebRecoveryRepository {
  private readonly ports: WebRecoveryPorts;
  private readonly now: () => number;
  private persistenceRequest: Promise<PersistenceRequestResult> | null = null;
  /** 同一instance内の同じcandidateだけを直列化する。cross-tab lockではない。 */
  private readonly candidateQueues = new Map<number, Promise<void>>();

  constructor(options: WebRecoveryRepositoryOptions = {}) {
    this.ports = options.ports ?? createBrowserRecoveryPorts();
    this.now = options.now ?? Date.now;
  }

  /** persist() の結果は状態として観測するだけで、保存可否の条件にはしない。 */
  requestPersistentStorage(): Promise<PersistenceRequestResult> {
    this.persistenceRequest ??= this.ports
      .requestPersistence()
      .catch((): PersistenceRequestResult => "unavailable");
    return this.persistenceRequest;
  }

  async listCandidates(): Promise<RecoveryCandidateSummary[]> {
    const candidates = await this.ports.metadata.listMetadata();
    for (const candidate of candidates) {
      assertBrowserCandidateId(candidate.candidate_id);
    }
    return candidates
      .sort(
        (left, right) =>
          right.saved_at_ms - left.saved_at_ms ||
          right.candidate_id - left.candidate_id,
      )
      .map(summary);
  }

  saveCheckpoint(
    source: SavedDocumentSource,
    options: SaveRecoveryCheckpointOptions = {},
  ): Promise<RecoveryCandidateSummary> {
    const candidateId = options.candidateId ?? null;
    if (candidateId === null) {
      return this.saveCheckpointLocked(source, options);
    }
    assertBrowserCandidateId(candidateId);
    return this.runForCandidate(candidateId, () =>
      this.saveCheckpointLocked(source, options),
    );
  }

  private async saveCheckpointLocked(
    source: SavedDocumentSource,
    options: SaveRecoveryCheckpointOptions,
  ): Promise<RecoveryCandidateSummary> {
    // pending/denied/unavailableでも続ける。persist() は保存のgateでも保証でもない。
    void this.requestPersistentStorage();

    let candidateId = options.candidateId ?? null;
    let previous: RecoveryCandidateMetadata | null = null;
    if (candidateId !== null) {
      assertBrowserCandidateId(candidateId);
      previous = await this.ports.metadata.getMetadata(candidateId);
    }
    if (candidateId === null || previous === null) {
      candidateId = await this.ports.metadata.allocateCandidateId();
      assertBrowserCandidateId(candidateId);
      previous = null;
    }

    const payload = serializeSavedDocument(source);
    const payloadStore = await this.writePayload(candidateId, payload);
    const savedAt = Math.trunc(this.now());
    if (!Number.isSafeInteger(savedAt) || savedAt < 0) {
      if (previous === null) {
        await this.ignoreDeleteFailure(payloadStore, candidateId);
      }
      throw new Error("復旧候補の保存時刻を安全な整数で記録できません。");
    }
    const metadata: RecoveryCandidateMetadata = {
      candidate_id: candidateId,
      document_path: options.documentPath ?? null,
      saved_at_ms: savedAt,
      step_count: source.doc.sequence.length,
      payload_location: payloadStore.location,
    };
    try {
      if (
        payloadStore.location === "indexeddb" &&
        this.ports.indexedDbCandidateTransactions !== undefined
      ) {
        await this.ports.indexedDbCandidateTransactions.putCandidate(
          metadata,
          payload,
        );
      } else {
        await this.ports.metadata.putMetadata(metadata);
      }
    } catch (error) {
      if (previous === null) {
        await this.ignoreDeleteFailure(payloadStore, candidateId);
      }
      throw error;
    }

    if (
      previous !== null &&
      previous.payload_location !== payloadStore.location
    ) {
      await this.ignoreDeleteFailure(
        this.payloadStore(previous.payload_location),
        candidateId,
      );
    }
    return summary(metadata);
  }

  restoreCandidate(candidateId: number): Promise<SavedDocumentSnapshot> {
    assertBrowserCandidateId(candidateId);
    return this.runForCandidate(candidateId, async () =>
      parseSavedDocument((await this.readCandidateSourceLocked(candidateId)).source),
    );
  }

  readCandidateSource(candidateId: number): Promise<RecoveryCandidateSource> {
    assertBrowserCandidateId(candidateId);
    return this.runForCandidate(candidateId, () =>
      this.readCandidateSourceLocked(candidateId),
    );
  }

  private async readCandidateSourceLocked(
    candidateId: number,
  ): Promise<RecoveryCandidateSource> {
    const metadata = await this.ports.metadata.getMetadata(candidateId);
    if (metadata === null) {
      throw new Error("選んだ復旧候補は保存領域に見つかりません。");
    }
    const payload = await this.payloadStore(
      metadata.payload_location,
    ).readPayload(candidateId);
    if (payload === null) {
      throw new Error("選んだ復旧候補の作品データが見つかりません。");
    }
    // 復元しただけでは候補を消さない。明示保存の成功を待つ。
    return { candidate: summary(metadata), source: payload };
  }

  /** 利用者が選んだ1候補だけを破棄する。 */
  discardCandidate(candidateId: number): Promise<void> {
    assertBrowserCandidateId(candidateId);
    return this.runForCandidate(candidateId, () =>
      this.discardCandidateLocked(candidateId),
    );
  }

  private async discardCandidateLocked(candidateId: number): Promise<void> {
    const metadata = await this.ports.metadata.getMetadata(candidateId);
    if (metadata === null) {
      throw new Error("選んだ復旧候補は保存領域に見つかりません。");
    }
    if (
      metadata.payload_location === "indexeddb" &&
      this.ports.indexedDbCandidateTransactions !== undefined
    ) {
      await this.ports.indexedDbCandidateTransactions.deleteCandidate(
        candidateId,
      );
      return;
    }
    const payloadStore = this.payloadStore(metadata.payload_location);
    // metadataを先に消す。payload削除失敗/中断時も壊れた候補を一覧へ残さない。
    await this.ports.metadata.deleteMetadata(candidateId);
    await payloadStore.deletePayload(candidateId);
  }

  /** 作品の明示保存が成功した後にだけ、その復元元候補を消す。 */
  clearCandidateAfterExplicitSaveSucceeded(candidateId: number): Promise<void> {
    return this.discardCandidate(candidateId);
  }

  private async writePayload(
    candidateId: number,
    payload: string,
  ): Promise<RecoveryPayloadStore> {
    if (this.ports.opfsPayload !== null) {
      try {
        await this.ports.opfsPayload.writePayload(candidateId, payload);
        return this.ports.opfsPayload;
      } catch {
        // OPFSが使えない環境・権限状態ではIndexedDBへ退避する。
      }
    }
    if (this.ports.indexedDbCandidateTransactions === undefined) {
      await this.ports.indexedDbPayload.writePayload(candidateId, payload);
    }
    return this.ports.indexedDbPayload;
  }

  private payloadStore(location: unknown): RecoveryPayloadStore {
    if (location === "opfs") {
      if (this.ports.opfsPayload === null) {
        throw new Error("この環境ではOPFSの復旧データを読み取れません。");
      }
      return this.ports.opfsPayload;
    }
    if (location === "indexeddb") return this.ports.indexedDbPayload;
    throw new Error("復旧候補の保存場所を判別できません。");
  }

  private async ignoreDeleteFailure(
    store: RecoveryPayloadStore,
    candidateId: number,
  ): Promise<void> {
    try {
      await store.deletePayload(candidateId);
    } catch {
      // metadataの無い孤立payloadは候補一覧へ現れず、次回同IDも割り当てない。
    }
  }

  private runForCandidate<T>(
    candidateId: number,
    operation: () => Promise<T>,
  ): Promise<T> {
    const previous = this.candidateQueues.get(candidateId) ?? Promise.resolve();
    const result = previous.then(operation, operation);
    // rejectionを次の操作へ伝播させず、失敗後も同candidateを操作できる。
    const settled = result.then(
      () => undefined,
      () => undefined,
    );
    this.candidateQueues.set(candidateId, settled);
    void settled.then(() => {
      if (this.candidateQueues.get(candidateId) === settled) {
        this.candidateQueues.delete(candidateId);
      }
    });
    return result;
  }
}
