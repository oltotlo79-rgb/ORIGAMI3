import { describe, expect, it } from "vitest";
import type { Document } from "../../../desktop/src/lib/types";
import {
  WebRecoveryRepository,
  assertBrowserCandidateId,
} from "./WebRecoveryRepository";
import type {
  PersistenceRequestResult,
  RecoveryCandidateMetadata,
  RecoveryMetadataStore,
  RecoveryPayloadLocation,
  RecoveryPayloadStore,
  WebRecoveryPorts,
} from "./recoveryPorts";
import type { SavedDocumentSource } from "./savedDocument";

class MemoryMetadataStore implements RecoveryMetadataStore {
  readonly records = new Map<number, RecoveryCandidateMetadata>();
  putGate: Promise<void> | null = null;
  onPutStart: (() => void) | null = null;
  private nextCandidateId = 1;

  async allocateCandidateId(): Promise<number> {
    const candidateId = this.nextCandidateId;
    this.nextCandidateId += 1;
    return candidateId;
  }

  async listMetadata(): Promise<RecoveryCandidateMetadata[]> {
    return [...this.records.values()].map((record) => ({ ...record }));
  }

  async getMetadata(
    candidateId: number,
  ): Promise<RecoveryCandidateMetadata | null> {
    const record = this.records.get(candidateId);
    return record === undefined ? null : { ...record };
  }

  async putMetadata(metadata: RecoveryCandidateMetadata): Promise<void> {
    this.onPutStart?.();
    if (this.putGate !== null) await this.putGate;
    this.records.set(metadata.candidate_id, { ...metadata });
  }

  async deleteMetadata(candidateId: number): Promise<void> {
    this.records.delete(candidateId);
  }
}

class MemoryPayloadStore implements RecoveryPayloadStore {
  readonly payloads = new Map<number, string>();
  readonly deleted: number[] = [];
  readonly writeGates = new Map<number, Promise<void>>();
  readonly onWriteStart = new Map<number, () => void>();
  failWrites = false;
  failDeletes = false;

  constructor(readonly location: RecoveryPayloadLocation) {}

  async writePayload(candidateId: number, payload: string): Promise<void> {
    if (this.failWrites) throw new Error("OPFSを利用できません。");
    this.onWriteStart.get(candidateId)?.();
    const gate = this.writeGates.get(candidateId);
    if (gate !== undefined) await gate;
    this.payloads.set(candidateId, payload);
  }

  async readPayload(candidateId: number): Promise<string | null> {
    return this.payloads.get(candidateId) ?? null;
  }

  async deletePayload(candidateId: number): Promise<void> {
    if (this.failDeletes) throw new Error("payloadを削除できません。");
    this.deleted.push(candidateId);
    this.payloads.delete(candidateId);
  }
}

function savedSource(note = "控え"): SavedDocumentSource {
  const doc: Document = {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: {
      vertices: [{ id: 0, pos: [0, 0] }],
      edges: [],
      next_vertex_id: 1,
      next_edge_id: 0,
    },
    sequence: [
      {
        id: 1,
        kind: "Simple",
        drivers: [],
        layer_order: [],
        note,
      },
    ],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
  return { doc };
}

function fakePorts(options?: {
  opfs?: MemoryPayloadStore | null;
  persistence?: PersistenceRequestResult;
  persistenceError?: Error;
  persistencePromise?: Promise<PersistenceRequestResult>;
}): {
  ports: WebRecoveryPorts;
  metadata: MemoryMetadataStore;
  indexedDb: MemoryPayloadStore;
  opfs: MemoryPayloadStore | null;
  persistenceCalls: { value: number };
  atomicCalls: { puts: number; deletes: number };
} {
  const metadata = new MemoryMetadataStore();
  const indexedDb = new MemoryPayloadStore("indexeddb");
  const opfs =
    options?.opfs === undefined
      ? new MemoryPayloadStore("opfs")
      : options.opfs;
  const persistenceCalls = { value: 0 };
  const atomicCalls = { puts: 0, deletes: 0 };
  return {
    metadata,
    indexedDb,
    opfs,
    persistenceCalls,
    atomicCalls,
    ports: {
      metadata,
      indexedDbPayload: indexedDb,
      indexedDbCandidateTransactions: {
        async putCandidate(candidate, payload) {
          atomicCalls.puts += 1;
          metadata.records.set(candidate.candidate_id, { ...candidate });
          indexedDb.payloads.set(candidate.candidate_id, payload);
        },
        async deleteCandidate(candidateId) {
          atomicCalls.deletes += 1;
          metadata.records.delete(candidateId);
          indexedDb.payloads.delete(candidateId);
        },
      },
      opfsPayload: opfs,
      async requestPersistence() {
        persistenceCalls.value += 1;
        if (options?.persistenceError !== undefined) {
          throw options.persistenceError;
        }
        if (options?.persistencePromise !== undefined) {
          return options.persistencePromise;
        }
        return options?.persistence ?? "denied";
      },
    },
  };
}

describe("WebRecoveryRepository", () => {
  it("persist拒否でもOPFSへ複数候補を保存し、安全なcandidate_idで識別する", async () => {
    const storage = fakePorts({ persistence: "denied" });
    let now = 1_000;
    const repository = new WebRecoveryRepository({
      ports: storage.ports,
      now: () => now++,
    });

    const first = await repository.saveCheckpoint(savedSource("鶴"), {
      documentPath: "browser-file:crane",
    });
    const second = await repository.saveCheckpoint(savedSource("やっこさん"));

    expect(first.candidate_id).toBe(1);
    expect(second.candidate_id).toBe(2);
    expect(storage.persistenceCalls.value).toBe(1);
    expect(storage.opfs?.payloads.size).toBe(2);
    expect(storage.indexedDb.payloads.size).toBe(0);
    expect((await repository.listCandidates()).map((item) => item.candidate_id))
      .toEqual([2, 1]);
  });

  it("同じ作業中は候補を更新し、復元では消さず、明示保存成功でも当該候補だけ消す", async () => {
    const storage = fakePorts();
    let now = 2_000;
    const repository = new WebRecoveryRepository({
      ports: storage.ports,
      now: () => now++,
    });
    const first = await repository.saveCheckpoint(savedSource("初回"));
    const second = await repository.saveCheckpoint(savedSource("別作品"));
    const updated = await repository.saveCheckpoint(savedSource("更新後"), {
      candidateId: first.candidate_id,
    });

    expect(updated.candidate_id).toBe(first.candidate_id);
    expect(storage.metadata.records.size).toBe(2);
    expect((await repository.restoreCandidate(first.candidate_id)).sequence[0]?.note)
      .toBe("更新後");
    const raw = await repository.readCandidateSource(first.candidate_id);
    expect(raw.candidate.candidate_id).toBe(first.candidate_id);
    expect(JSON.parse(raw.source)).toMatchObject({
      sequence: [{ note: "更新後" }],
    });
    expect(storage.opfs?.deleted).toEqual([]);

    await repository.clearCandidateAfterExplicitSaveSucceeded(
      first.candidate_id,
    );
    expect((await repository.listCandidates()).map((item) => item.candidate_id))
      .toEqual([second.candidate_id]);
    expect(storage.opfs?.deleted).toEqual([first.candidate_id]);

    await repository.discardCandidate(second.candidate_id);
    expect(await repository.listCandidates()).toEqual([]);
    expect(storage.opfs?.deleted).toEqual([
      first.candidate_id,
      second.candidate_id,
    ]);
  });

  it("見つからないcandidate IDを別候補へ読み替えず明示的に失敗する", async () => {
    const repository = new WebRecoveryRepository({ ports: fakePorts().ports });

    await expect(repository.restoreCandidate(404)).rejects.toThrow(
      "選んだ復旧候補は保存領域に見つかりません。",
    );
    await expect(repository.discardCandidate(404)).rejects.toThrow(
      "選んだ復旧候補は保存領域に見つかりません。",
    );
  });

  it("OPFSが無い場合と書き込み失敗時はpayloadをIndexedDBへ退避する", async () => {
    const unavailable = fakePorts({ opfs: null });
    const unavailableRepository = new WebRecoveryRepository({
      ports: unavailable.ports,
      now: () => 3_000,
    });
    const first = await unavailableRepository.saveCheckpoint(savedSource());
    expect(unavailable.indexedDb.payloads.has(first.candidate_id)).toBe(true);
    expect(unavailable.atomicCalls.puts).toBe(1);
    expect(unavailable.metadata.records.get(first.candidate_id)?.payload_location)
      .toBe("indexeddb");

    const failingOpfs = new MemoryPayloadStore("opfs");
    failingOpfs.failWrites = true;
    const failed = fakePorts({ opfs: failingOpfs });
    const failedRepository = new WebRecoveryRepository({
      ports: failed.ports,
      now: () => 4_000,
    });
    const second = await failedRepository.saveCheckpoint(savedSource());
    expect(failed.indexedDb.payloads.has(second.candidate_id)).toBe(true);
    expect(failed.atomicCalls.puts).toBe(1);
    expect(failed.metadata.records.get(second.candidate_id)?.payload_location)
      .toBe("indexeddb");

    const persistRejected = fakePorts({
      persistenceError: new Error("永続化要求を拒否"),
    });
    const persistRejectedRepository = new WebRecoveryRepository({
      ports: persistRejected.ports,
      now: () => 5_000,
    });
    await expect(persistRejectedRepository.saveCheckpoint(savedSource()))
      .resolves.toMatchObject({ candidate_id: 1 });
  });

  it("安全な整数でないcandidate_idを保存領域へ渡さない", () => {
    expect(() => assertBrowserCandidateId(0)).toThrow(
      "復旧候補の番号が安全な整数の範囲を超えています。",
    );
    expect(() => assertBrowserCandidateId(1.5)).toThrow();
    expect(() => assertBrowserCandidateId(Number.MAX_SAFE_INTEGER + 1)).toThrow();
  });

  it("persistが未解決でもmetadataとpayloadのcheckpointを完了する", async () => {
    const pendingPersistence = new Promise<PersistenceRequestResult>(
      () => undefined,
    );
    const storage = fakePorts({ persistencePromise: pendingPersistence });
    const repository = new WebRecoveryRepository({
      ports: storage.ports,
      now: () => 6_000,
    });

    await expect(repository.saveCheckpoint(savedSource())).resolves.toMatchObject({
      candidate_id: 1,
    });
    expect(storage.metadata.records.size).toBe(1);
    expect(storage.opfs?.payloads.size).toBe(1);
  });

  it("同じcandidateのsave途中にdiscardしても直列化し、壊れたmetadataを残さない", async () => {
    const storage = fakePorts();
    const repository = new WebRecoveryRepository({
      ports: storage.ports,
      now: () => 7_000,
    });
    const candidate = await repository.saveCheckpoint(savedSource("初回"));
    let releasePut = (): void => undefined;
    storage.metadata.putGate = new Promise<void>((resolve) => {
      releasePut = resolve;
    });
    let signalPutStarted = (): void => undefined;
    const putStarted = new Promise<void>((resolve) => {
      signalPutStarted = resolve;
    });
    storage.metadata.onPutStart = signalPutStarted;

    const updating = repository.saveCheckpoint(savedSource("更新"), {
      candidateId: candidate.candidate_id,
    });
    await putStarted;
    const discarding = repository.discardCandidate(candidate.candidate_id);
    expect(storage.metadata.records.has(candidate.candidate_id)).toBe(true);

    releasePut();
    await updating;
    await discarding;

    expect(await repository.listCandidates()).toEqual([]);
    expect(storage.metadata.records.has(candidate.candidate_id)).toBe(false);
    expect(storage.opfs?.payloads.has(candidate.candidate_id)).toBe(false);
  });

  it("payload削除失敗でもmetadataを先に消し、一覧と復元へ壊れた候補を出さない", async () => {
    const storage = fakePorts();
    const repository = new WebRecoveryRepository({
      ports: storage.ports,
      now: () => 8_000,
    });
    const candidate = await repository.saveCheckpoint(savedSource());
    if (storage.opfs === null) throw new Error("検査用OPFSがありません。");
    storage.opfs.failDeletes = true;

    await expect(repository.discardCandidate(candidate.candidate_id)).rejects
      .toThrow("payloadを削除できません。");

    expect(await repository.listCandidates()).toEqual([]);
    await expect(repository.restoreCandidate(candidate.candidate_id)).rejects
      .toThrow("選んだ復旧候補は保存領域に見つかりません。");
  });

  it("異なるcandidateのsaveは一方の待機に巻き込まず並行できる", async () => {
    const storage = fakePorts();
    const repository = new WebRecoveryRepository({
      ports: storage.ports,
      now: () => 9_000,
    });
    const first = await repository.saveCheckpoint(savedSource("一"));
    const second = await repository.saveCheckpoint(savedSource("二"));
    if (storage.opfs === null) throw new Error("検査用OPFSがありません。");
    let releaseFirst = (): void => undefined;
    storage.opfs.writeGates.set(
      first.candidate_id,
      new Promise<void>((resolve) => {
        releaseFirst = resolve;
      }),
    );
    let signalFirstStarted = (): void => undefined;
    const firstStarted = new Promise<void>((resolve) => {
      signalFirstStarted = resolve;
    });
    storage.opfs.onWriteStart.set(first.candidate_id, signalFirstStarted);

    const firstUpdate = repository.saveCheckpoint(savedSource("一更新"), {
      candidateId: first.candidate_id,
    });
    await firstStarted;
    await expect(
      repository.saveCheckpoint(savedSource("二更新"), {
        candidateId: second.candidate_id,
      }),
    ).resolves.toMatchObject({ candidate_id: second.candidate_id });

    releaseFirst();
    await firstUpdate;
    expect((await repository.restoreCandidate(second.candidate_id)).sequence[0]?.note)
      .toBe("二更新");
  });
});
