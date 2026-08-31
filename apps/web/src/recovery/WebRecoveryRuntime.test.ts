import { describe, expect, it } from "vitest";
import type { BackendCommandName, BackendInvokeArgs } from "../../../desktop/src/ipc/runtime";
import type { Document, DocumentView, RecoveryChoices } from "../../../desktop/src/lib/types";
import { createBrowserCurrentDocumentCoordinator } from "../backend/browserDocumentInvoker";
import type { CoreCommandName } from "../backend/coreWorkerProtocol";
import { createBrowserFileTokenRegistry } from "../platform/browserFileTokenRegistry";
import type { AutosaveClock } from "./AutosaveScheduler";
import { createWebRecoveryRuntime, type WebRecoveryRepositoryPort } from "./WebRecoveryRuntime";
import type { RecoveryCandidateSource, RecoveryCandidateSummary } from "./WebRecoveryRepository";
import type { SavedDocumentSource } from "./savedDocument";

class FakeClock implements AutosaveClock {
  private next = 1;
  setTimeout(): unknown { return this.next++; }
  clearTimeout(): void {}
  setInterval(): unknown { return this.next++; }
  clearInterval(): void {}
}

function document(note: string): Document {
  return {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: { vertices: [], edges: [], next_vertex_id: 0, next_edge_id: 0 },
    sequence: [{ id: 1, kind: "Simple", drivers: [], layer_order: [], note }],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
}

function view(doc: Document): DocumentView {
  return {
    doc,
    step_creases: [],
    faces: [],
    warnings: [],
    violations: [],
    frame: null,
    skipped: [],
    contact_detected: false,
  };
}

class MemoryRepository implements WebRecoveryRepositoryPort {
  readonly records = new Map<number, RecoveryCandidateSource>();
  readonly discarded: number[] = [];
  readonly cleared: number[] = [];
  readonly saveOptions: Array<{ candidateId: number | null; documentPath: string | null }> = [];
  failSave = false;
  private nextId = 20;

  add(candidateId: number, savedAt: number, name: string | null, note: string): void {
    const candidate: RecoveryCandidateSummary = {
      candidate_id: candidateId,
      autosave_path: `browser-recovery://candidate/${candidateId}`,
      document_path: name,
      saved_at_ms: savedAt,
      step_count: 1,
    };
    this.records.set(candidateId, {
      candidate,
      source: JSON.stringify(document(note)),
    });
  }

  async listCandidates(): Promise<RecoveryCandidateSummary[]> {
    return [...this.records.values()].map(({ candidate }) => ({ ...candidate }));
  }

  async readCandidateSource(candidateId: number): Promise<RecoveryCandidateSource> {
    const value = this.records.get(candidateId);
    if (value === undefined) throw new Error("選んだ復旧候補は保存領域に見つかりません。");
    return { candidate: { ...value.candidate }, source: value.source };
  }

  async saveCheckpoint(
    source: SavedDocumentSource,
    options: { candidateId: number | null; documentPath: string | null },
  ): Promise<RecoveryCandidateSummary> {
    if (this.failSave) throw new Error("復旧用の控えを書き込めません。");
    this.saveOptions.push({ ...options });
    const candidateId = options.candidateId ?? this.nextId++;
    const candidate: RecoveryCandidateSummary = {
      candidate_id: candidateId,
      autosave_path: `browser-recovery://candidate/${candidateId}`,
      document_path: options.documentPath,
      saved_at_ms: 9_000,
      step_count: source.doc.sequence.length,
    };
    this.records.set(candidateId, {
      candidate,
      source: JSON.stringify(source.doc),
    });
    return candidate;
  }

  async discardCandidate(candidateId: number): Promise<void> {
    if (!this.records.delete(candidateId)) throw new Error("候補がありません。");
    this.discarded.push(candidateId);
  }

  async clearCandidateAfterExplicitSaveSucceeded(candidateId: number): Promise<void> {
    if (!this.records.delete(candidateId)) throw new Error("候補がありません。");
    this.cleared.push(candidateId);
  }
}

class FakeCore {
  readonly calls: Array<{ command: CoreCommandName; args?: BackendInvokeArgs }> = [];
  choices: RecoveryChoices | null = null;
  snapshot: SavedDocumentSource | null = null;
  stagedDocument: Document | null = null;

  async invoke<T>(command: CoreCommandName, args?: BackendInvokeArgs): Promise<T> {
    this.calls.push({ command, ...(args === undefined ? {} : { args }) });
    const record = args as Record<string, unknown> | undefined;
    if (command === "__web_recovery_set_choices") {
      this.choices = (record?.choices ?? null) as RecoveryChoices | null;
      return null as T;
    }
    if (command === "recovery_check") return this.choices as T;
    if (command === "__web_recovery_snapshot") return this.snapshot as T;
    if (command === "__web_recovery_restore_source") {
      try {
        this.stagedDocument = JSON.parse(String(record?.source)) as Document;
      } catch {
        throw "復旧データのJSONが壊れているため、作品を復元できません。";
      }
      return null as T;
    }
    if (command === "recovery_restore") {
      if (record?.accept === false) return null as T;
      if (this.stagedDocument === null) throw new Error("復元元がありません。");
      return view(this.stagedDocument) as T;
    }
    throw new Error(`unexpected core command: ${command}`);
  }
}

function delegate(calls: BackendCommandName[], result: unknown = null) {
  return {
    async invoke<T>(command: BackendCommandName): Promise<T> {
      calls.push(command);
      return result as T;
    },
  };
}

describe("WebRecoveryRuntime", () => {
  it("全候補を降順stageし、復元元Aと編集後Cだけを明示保存成功で消す", async () => {
    const repository = new MemoryRepository();
    repository.add(1, 100, "C:\\作品\\折り鶴.ori3", "復元元A");
    repository.add(2, 300, null, "無関係B");
    repository.add(3, 200, null, "無関係その2");
    repository.add(4, 50, null, "無関係その3");
    const core = new FakeCore();
    const registry = createBrowserFileTokenRegistry(() => undefined);
    const currentDocument = createBrowserCurrentDocumentCoordinator(registry);
    const messages: string[] = [];
    const runtime = createWebRecoveryRuntime(core, {
      currentDocument,
      registry,
      repository,
      onError: (message) => messages.push(message),
      clock: new FakeClock(),
    });

    const before = await runtime.browser.invoke<RecoveryChoices>("recovery_check");
    expect(before.choices.map((choice) => choice.candidate_id)).toEqual([2, 3, 1, 4]);
    expect(before.overflow_count).toBe(1);

    const mixedCalls: BackendCommandName[] = [];
    const mixed = runtime.decorateMixed(delegate(mixedCalls));
    const restored = await mixed.invoke<DocumentView>("recovery_restore", {
      accept: true,
      candidateId: 1,
    });
    expect(restored.doc.sequence[0]?.note).toBe("復元元A");
    expect(repository.records.has(1)).toBe(true);
    expect(runtime.associatedCandidateIds()).toEqual([1]);
    const stagedPath = core.calls.find(
      (call) => call.command === "__web_recovery_restore_source",
    )?.args as Record<string, unknown>;
    expect(stagedPath.documentPath).toMatch(/^browser-file:\/\/download\//);
    expect(String(stagedPath.documentPath)).not.toContain("C:\\作品");
    expect(registry.nameOf(currentDocument.current()!)).toBe("折り鶴.ori3");

    const during = await runtime.browser.invoke<RecoveryChoices>("recovery_check");
    expect(during.choices.map((choice) => choice.candidate_id)).toEqual([2, 3, 4]);

    core.snapshot = { doc: document("編集後C"), step_creases: [] };
    const coreCalls: BackendCommandName[] = [];
    const editing = runtime.decorateCore(delegate(coreCalls, view(document("編集後C"))));
    await editing.invoke("edit_apply", { op: { type: "noop" } });
    await runtime.flushAutosave();
    expect(repository.saveOptions).toEqual([
      { candidateId: null, documentPath: "折り鶴.ori3" },
    ]);
    expect(runtime.associatedCandidateIds()).toEqual([1, 20]);

    await mixed.invoke("document_save", { path: null });
    expect(repository.cleared).toEqual([1, 20]);
    expect(repository.records.has(2)).toBe(true);
    expect(repository.records.has(3)).toBe(true);
    expect(repository.records.has(4)).toBe(true);
    expect(messages).toEqual([]);
    expect(mixedCalls).toEqual(["document_save"]);
    expect(core.calls.map((call) => call.command)).toEqual([
      "__web_recovery_set_choices",
      "recovery_check",
      "__web_recovery_set_choices",
      "__web_recovery_restore_source",
      "recovery_restore",
      "__web_recovery_set_choices",
      "recovery_check",
      "__web_recovery_snapshot",
    ]);
    runtime.dispose();
  });

  it("破棄は選択1件だけを消し、raw破損restoreはcore/current/repositoryを変えない", async () => {
    const repository = new MemoryRepository();
    repository.add(5, 500, null, "破棄対象");
    repository.add(6, 400, null, "保持対象");
    const core = new FakeCore();
    const registry = createBrowserFileTokenRegistry(() => undefined);
    const currentDocument = createBrowserCurrentDocumentCoordinator(registry);
    const runtime = createWebRecoveryRuntime(core, {
      currentDocument,
      registry,
      repository,
      onError: () => undefined,
      clock: new FakeClock(),
    });
    const mixed = runtime.decorateMixed(delegate([]));

    await expect(mixed.invoke("recovery_restore", { accept: false, candidateId: 5 }))
      .resolves.toBeNull();
    expect(repository.discarded).toEqual([5]);
    expect([...repository.records.keys()]).toEqual([6]);

    const corrupt = repository.records.get(6)!;
    repository.records.set(6, { ...corrupt, source: "{" });
    await expect(
      mixed.invoke("recovery_restore", { accept: true, candidateId: 6 }),
    ).rejects.toBe("復旧データのJSONが壊れているため、作品を復元できません。");
    expect(repository.records.has(6)).toBe(true);
    expect(currentDocument.current()).toBeNull();
    expect(runtime.associatedCandidateIds()).toEqual([]);
    runtime.dispose();
  });

  it("作品切替前のcheckpoint失敗は常設errorへ出し、delegateを呼ばず旧作品を保つ", async () => {
    const repository = new MemoryRepository();
    repository.failSave = true;
    const core = new FakeCore();
    core.snapshot = { doc: document("切替前"), step_creases: [] };
    const registry = createBrowserFileTokenRegistry(() => undefined);
    const messages: string[] = [];
    const runtime = createWebRecoveryRuntime(core, {
      currentDocument: createBrowserCurrentDocumentCoordinator(registry),
      registry,
      repository,
      onError: (message) => messages.push(message),
      clock: new FakeClock(),
    });
    const calls: BackendCommandName[] = [];
    const decoratedCore = runtime.decorateCore(delegate(calls, view(document("新規"))));
    await decoratedCore.invoke("edit_apply", { op: { type: "noop" } });

    await expect(decoratedCore.invoke("document_new", { paper: {} }))
      .rejects.toThrow("復旧用の控えを書き込めません。");
    await Promise.resolve();
    expect(calls).toEqual(["edit_apply"]);
    expect(messages).toEqual(["復旧用の控えを書き込めません。"]);
    runtime.dispose();
  });

  it("FOLD importは保存先を継がずautosaveし、clean ORI3 openは候補を作らない", async () => {
    const repository = new MemoryRepository();
    const core = new FakeCore();
    const registry = createBrowserFileTokenRegistry(() => undefined);
    const currentDocument = createBrowserCurrentDocumentCoordinator(registry);
    const runtime = createWebRecoveryRuntime(core, {
      currentDocument,
      registry,
      repository,
      onError: () => undefined,
      clock: new FakeClock(),
    });
    const foldToken = registry.registerDownload("鳥の基本形.fold");
    const ori3Token = registry.registerDownload("折り鶴.ori3");
    const mixed = runtime.decorateMixed({
      async invoke<T>(command: BackendCommandName): Promise<T> {
        currentDocument.adopt(
          registry.nameOf(foldToken).endsWith(".fold") ? foldToken : ori3Token,
        );
        return view(document(String(command))) as T;
      },
    });

    core.snapshot = { doc: document("FOLD import"), step_creases: [] };
    await mixed.invoke("document_open", { path: foldToken });
    expect(currentDocument.current()).toBeNull();
    await runtime.flushAutosave();
    expect(repository.saveOptions).toEqual([
      { candidateId: null, documentPath: null },
    ]);

    core.snapshot = null;
    const cleanMixed = runtime.decorateMixed({
      async invoke<T>(): Promise<T> {
        currentDocument.adopt(ori3Token);
        return view(document("clean")) as T;
      },
    });
    await cleanMixed.invoke("document_open", { path: ori3Token });
    expect(registry.nameOf(currentDocument.current()!)).toBe("折り鶴.ori3");
    await runtime.flushAutosave();
    expect(repository.saveOptions).toHaveLength(1);
    runtime.dispose();
  });

  it("不正な紙でdocument_newが失敗したときは新しい候補も関連も作らない", async () => {
    const repository = new MemoryRepository();
    const core = new FakeCore();
    core.snapshot = null;
    const registry = createBrowserFileTokenRegistry(() => undefined);
    const runtime = createWebRecoveryRuntime(core, {
      currentDocument: createBrowserCurrentDocumentCoordinator(registry),
      registry,
      repository,
      onError: () => undefined,
      clock: new FakeClock(),
    });
    const decorated = runtime.decorateCore({
      async invoke(): Promise<never> {
        throw "紙の幅と高さは0より大きくしてください";
      },
    });

    await expect(
      decorated.invoke("document_new", {
        paper: { width_mm: 0, height_mm: 100 },
      }),
    ).rejects.toBe("紙の幅と高さは0より大きくしてください");
    expect(repository.records.size).toBe(0);
    expect(runtime.associatedCandidateIds()).toEqual([]);
    runtime.dispose();
  });
});
