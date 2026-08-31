export type RecoveryPayloadLocation = "opfs" | "indexeddb";

export interface RecoveryCandidateMetadata {
  candidate_id: number;
  document_path: string | null;
  saved_at_ms: number;
  step_count: number;
  payload_location: RecoveryPayloadLocation;
}

export interface RecoveryMetadataStore {
  allocateCandidateId(): Promise<number>;
  listMetadata(): Promise<RecoveryCandidateMetadata[]>;
  getMetadata(candidateId: number): Promise<RecoveryCandidateMetadata | null>;
  putMetadata(metadata: RecoveryCandidateMetadata): Promise<void>;
  deleteMetadata(candidateId: number): Promise<void>;
}

export interface RecoveryPayloadStore {
  readonly location: RecoveryPayloadLocation;
  writePayload(candidateId: number, payload: string): Promise<void>;
  readPayload(candidateId: number): Promise<string | null>;
  deletePayload(candidateId: number): Promise<void>;
}

/** IndexedDB fallbackのmetadataとpayloadを同じtransactionで確定する。 */
export interface IndexedDbCandidateTransactionStore {
  putCandidate(
    metadata: RecoveryCandidateMetadata,
    payload: string,
  ): Promise<void>;
  deleteCandidate(candidateId: number): Promise<void>;
}

export type PersistenceRequestResult =
  | "granted"
  | "denied"
  | "unavailable";

export interface WebRecoveryPorts {
  metadata: RecoveryMetadataStore;
  indexedDbPayload: RecoveryPayloadStore;
  indexedDbCandidateTransactions?: IndexedDbCandidateTransactionStore;
  opfsPayload: RecoveryPayloadStore | null;
  requestPersistence(): Promise<PersistenceRequestResult>;
}

const DATABASE_NAME = "ori3-web-recovery";
const DATABASE_VERSION = 1;
const METADATA_STORE = "candidates";
const PAYLOAD_STORE = "payloads";
const STATE_STORE = "state";
const NEXT_CANDIDATE_ID = "next_candidate_id";

interface PayloadRecord {
  candidate_id: number;
  payload: string;
}

interface StateRecord {
  key: string;
  value: number;
}

function requestResult<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () =>
      reject(request.error ?? new Error("IndexedDBの読み書きに失敗しました。"));
  });
}

function transactionDone(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onabort = () =>
      reject(
        transaction.error ?? new Error("IndexedDBの処理が中断されました。"),
      );
    transaction.onerror = () =>
      reject(transaction.error ?? new Error("IndexedDBの処理に失敗しました。"));
  });
}

class IndexedDbRecoveryStore implements RecoveryMetadataStore {
  private readonly databasePromise: Promise<IDBDatabase>;

  constructor(factory: IDBFactory) {
    this.databasePromise = new Promise((resolve, reject) => {
      const request = factory.open(DATABASE_NAME, DATABASE_VERSION);
      request.onupgradeneeded = () => {
        const database = request.result;
        if (!database.objectStoreNames.contains(METADATA_STORE)) {
          database.createObjectStore(METADATA_STORE, {
            keyPath: "candidate_id",
          });
        }
        if (!database.objectStoreNames.contains(PAYLOAD_STORE)) {
          database.createObjectStore(PAYLOAD_STORE, {
            keyPath: "candidate_id",
          });
        }
        if (!database.objectStoreNames.contains(STATE_STORE)) {
          database.createObjectStore(STATE_STORE, { keyPath: "key" });
        }
      };
      request.onsuccess = () => resolve(request.result);
      request.onerror = () =>
        reject(
          request.error ?? new Error("ブラウザの復旧保存領域を開けません。"),
        );
      request.onblocked = () =>
        reject(new Error("別のタブが使用中のため、復旧保存領域を更新できません。"));
    });
  }

  async allocateCandidateId(): Promise<number> {
    const database = await this.databasePromise;
    const transaction = database.transaction(STATE_STORE, "readwrite");
    const done = transactionDone(transaction);
    const store = transaction.objectStore(STATE_STORE);
    const current = (await requestResult(
      store.get(NEXT_CANDIDATE_ID),
    )) as StateRecord | undefined;
    const candidateId = current?.value ?? 1;
    if (!Number.isSafeInteger(candidateId) || candidateId < 1) {
      transaction.abort();
      await done.catch(() => undefined);
      throw new Error("復旧候補の番号が安全な整数の範囲を超えました。");
    }
    const nextCandidateId = candidateId + 1;
    if (!Number.isSafeInteger(nextCandidateId)) {
      transaction.abort();
      await done.catch(() => undefined);
      throw new Error("復旧候補の番号が安全な整数の範囲を超えました。");
    }
    await requestResult(
      store.put({
        key: NEXT_CANDIDATE_ID,
        value: nextCandidateId,
      } satisfies StateRecord),
    );
    await done;
    return candidateId;
  }

  async listMetadata(): Promise<RecoveryCandidateMetadata[]> {
    const database = await this.databasePromise;
    const transaction = database.transaction(METADATA_STORE, "readonly");
    const done = transactionDone(transaction);
    const records = (await requestResult(
      transaction.objectStore(METADATA_STORE).getAll(),
    )) as RecoveryCandidateMetadata[];
    await done;
    return records;
  }

  async getMetadata(
    candidateId: number,
  ): Promise<RecoveryCandidateMetadata | null> {
    const database = await this.databasePromise;
    const transaction = database.transaction(METADATA_STORE, "readonly");
    const done = transactionDone(transaction);
    const record = (await requestResult(
      transaction.objectStore(METADATA_STORE).get(candidateId),
    )) as RecoveryCandidateMetadata | undefined;
    await done;
    return record ?? null;
  }

  async putMetadata(metadata: RecoveryCandidateMetadata): Promise<void> {
    const database = await this.databasePromise;
    const transaction = database.transaction(METADATA_STORE, "readwrite");
    const done = transactionDone(transaction);
    await requestResult(transaction.objectStore(METADATA_STORE).put(metadata));
    await done;
  }

  async deleteMetadata(candidateId: number): Promise<void> {
    const database = await this.databasePromise;
    const transaction = database.transaction(METADATA_STORE, "readwrite");
    const done = transactionDone(transaction);
    await requestResult(
      transaction.objectStore(METADATA_STORE).delete(candidateId),
    );
    await done;
  }

  async writeIndexedDbPayload(
    candidateId: number,
    payload: string,
  ): Promise<void> {
    const database = await this.databasePromise;
    const transaction = database.transaction(PAYLOAD_STORE, "readwrite");
    const done = transactionDone(transaction);
    await requestResult(
      transaction.objectStore(PAYLOAD_STORE).put({
        candidate_id: candidateId,
        payload,
      } satisfies PayloadRecord),
    );
    await done;
  }

  async readIndexedDbPayload(candidateId: number): Promise<string | null> {
    const database = await this.databasePromise;
    const transaction = database.transaction(PAYLOAD_STORE, "readonly");
    const done = transactionDone(transaction);
    const record = (await requestResult(
      transaction.objectStore(PAYLOAD_STORE).get(candidateId),
    )) as PayloadRecord | undefined;
    await done;
    return record?.payload ?? null;
  }

  async deleteIndexedDbPayload(candidateId: number): Promise<void> {
    const database = await this.databasePromise;
    const transaction = database.transaction(PAYLOAD_STORE, "readwrite");
    const done = transactionDone(transaction);
    await requestResult(
      transaction.objectStore(PAYLOAD_STORE).delete(candidateId),
    );
    await done;
  }

  async putIndexedDbCandidate(
    metadata: RecoveryCandidateMetadata,
    payload: string,
  ): Promise<void> {
    const database = await this.databasePromise;
    const transaction = database.transaction(
      [METADATA_STORE, PAYLOAD_STORE],
      "readwrite",
    );
    const done = transactionDone(transaction);
    await Promise.all([
      requestResult(
        transaction.objectStore(METADATA_STORE).put(metadata),
      ),
      requestResult(
        transaction.objectStore(PAYLOAD_STORE).put({
          candidate_id: metadata.candidate_id,
          payload,
        } satisfies PayloadRecord),
      ),
    ]);
    await done;
  }

  async deleteIndexedDbCandidate(candidateId: number): Promise<void> {
    const database = await this.databasePromise;
    const transaction = database.transaction(
      [METADATA_STORE, PAYLOAD_STORE],
      "readwrite",
    );
    const done = transactionDone(transaction);
    await Promise.all([
      requestResult(
        transaction.objectStore(METADATA_STORE).delete(candidateId),
      ),
      requestResult(
        transaction.objectStore(PAYLOAD_STORE).delete(candidateId),
      ),
    ]);
    await done;
  }
}

class IndexedDbPayloadStore implements RecoveryPayloadStore {
  readonly location = "indexeddb" as const;

  constructor(private readonly store: IndexedDbRecoveryStore) {}

  writePayload(candidateId: number, payload: string): Promise<void> {
    return this.store.writeIndexedDbPayload(candidateId, payload);
  }

  readPayload(candidateId: number): Promise<string | null> {
    return this.store.readIndexedDbPayload(candidateId);
  }

  deletePayload(candidateId: number): Promise<void> {
    return this.store.deleteIndexedDbPayload(candidateId);
  }
}

class IndexedDbCandidateTransactions
  implements IndexedDbCandidateTransactionStore
{
  constructor(private readonly store: IndexedDbRecoveryStore) {}

  putCandidate(
    metadata: RecoveryCandidateMetadata,
    payload: string,
  ): Promise<void> {
    return this.store.putIndexedDbCandidate(metadata, payload);
  }

  deleteCandidate(candidateId: number): Promise<void> {
    return this.store.deleteIndexedDbCandidate(candidateId);
  }
}

interface OpfsWritable {
  write(data: string): Promise<void>;
  close(): Promise<void>;
}

interface OpfsFileHandle {
  createWritable(): Promise<OpfsWritable>;
  getFile(): Promise<File>;
}

interface OpfsDirectoryHandle {
  getDirectoryHandle(
    name: string,
    options?: { create?: boolean },
  ): Promise<OpfsDirectoryHandle>;
  getFileHandle(
    name: string,
    options?: { create?: boolean },
  ): Promise<OpfsFileHandle>;
  removeEntry(name: string): Promise<void>;
}

interface StorageManagerWithOpfs {
  getDirectory?(): Promise<OpfsDirectoryHandle>;
  persist?(): Promise<boolean>;
}

class OpfsPayloadStore implements RecoveryPayloadStore {
  readonly location = "opfs" as const;

  constructor(
    private readonly getRoot: () => Promise<OpfsDirectoryHandle>,
  ) {}

  private fileName(candidateId: number): string {
    return `candidate-${candidateId}.json`;
  }

  private async directory(): Promise<OpfsDirectoryHandle> {
    const root = await this.getRoot();
    return root.getDirectoryHandle("ori3-recovery", { create: true });
  }

  async writePayload(candidateId: number, payload: string): Promise<void> {
    const directory = await this.directory();
    const handle = await directory.getFileHandle(this.fileName(candidateId), {
      create: true,
    });
    const writable = await handle.createWritable();
    await writable.write(payload);
    await writable.close();
  }

  async readPayload(candidateId: number): Promise<string | null> {
    try {
      const directory = await this.directory();
      const handle = await directory.getFileHandle(this.fileName(candidateId));
      return (await handle.getFile()).text();
    } catch (error) {
      if (error instanceof DOMException && error.name === "NotFoundError") {
        return null;
      }
      throw error;
    }
  }

  async deletePayload(candidateId: number): Promise<void> {
    try {
      const directory = await this.directory();
      await directory.removeEntry(this.fileName(candidateId));
    } catch (error) {
      if (!(error instanceof DOMException) || error.name !== "NotFoundError") {
        throw error;
      }
    }
  }
}

export interface BrowserRecoveryEnvironment {
  indexedDB?: IDBFactory;
  navigator?: Navigator;
}

export function createBrowserRecoveryPorts(
  environment: BrowserRecoveryEnvironment = {},
): WebRecoveryPorts {
  const indexedDbFactory = environment.indexedDB ?? globalThis.indexedDB;
  if (indexedDbFactory === undefined) {
    throw new Error("このブラウザでは復旧用のIndexedDBを利用できません。");
  }
  const indexedDbStore = new IndexedDbRecoveryStore(indexedDbFactory);
  const browserNavigator =
    environment.navigator ??
    (typeof navigator === "undefined" ? undefined : navigator);
  const storage = browserNavigator?.storage as
    | (StorageManager & StorageManagerWithOpfs)
    | undefined;
  const getDirectory = storage?.getDirectory;
  const opfsPayload =
    getDirectory === undefined
      ? null
      : new OpfsPayloadStore(() => getDirectory.call(storage));

  return {
    metadata: indexedDbStore,
    indexedDbPayload: new IndexedDbPayloadStore(indexedDbStore),
    indexedDbCandidateTransactions: new IndexedDbCandidateTransactions(
      indexedDbStore,
    ),
    opfsPayload,
    async requestPersistence(): Promise<PersistenceRequestResult> {
      if (storage?.persist === undefined) return "unavailable";
      try {
        return (await storage.persist()) ? "granted" : "denied";
      } catch {
        return "unavailable";
      }
    },
  };
}
