import { describe, expect, it } from "vitest";
import type { Document } from "../../../desktop/src/lib/types";
import {
  AUTOSAVE_CHECKPOINT_MS,
  AutosaveScheduler,
  type AutosaveClock,
  type AutosaveRepositoryPort,
} from "./AutosaveScheduler";
import type { RecoveryCandidateSummary } from "./WebRecoveryRepository";
import type { SavedDocumentSource } from "./savedDocument";

class FakeClock implements AutosaveClock {
  readonly timeoutDelays: number[] = [];
  readonly intervalDelays: number[] = [];
  readonly clearedTimeouts: unknown[] = [];
  readonly clearedIntervals: unknown[] = [];
  private nextHandle = 1;
  private readonly timeouts = new Map<number, () => void>();
  private readonly intervals = new Map<number, () => void>();

  setTimeout(callback: () => void, delayMs: number): unknown {
    const handle = this.nextHandle++;
    this.timeouts.set(handle, callback);
    this.timeoutDelays.push(delayMs);
    return handle;
  }

  clearTimeout(handle: unknown): void {
    this.clearedTimeouts.push(handle);
    this.timeouts.delete(handle as number);
  }

  setInterval(callback: () => void, delayMs: number): unknown {
    const handle = this.nextHandle++;
    this.intervals.set(handle, callback);
    this.intervalDelays.push(delayMs);
    return handle;
  }

  clearInterval(handle: unknown): void {
    this.clearedIntervals.push(handle);
    this.intervals.delete(handle as number);
  }

  fireTimeouts(): void {
    const callbacks = [...this.timeouts.values()];
    this.timeouts.clear();
    callbacks.forEach((callback) => callback());
  }

  fireIntervals(): void {
    [...this.intervals.values()].forEach((callback) => callback());
  }
}

class FakeRepository implements AutosaveRepositoryPort {
  readonly calls: Array<{
    source: SavedDocumentSource;
    candidateId: number | null;
    documentPath: string | null;
  }> = [];
  failWith: Error | null = null;
  readonly failClearIds = new Set<number>();
  readonly deletedCandidateIds: number[] = [];
  readonly events: string[] = [];
  saveGate: Promise<void> | null = null;
  saveStarted = 0;
  private nextCandidateId = 17;

  async clearCandidateAfterExplicitSaveSucceeded(
    candidateId: number,
  ): Promise<void> {
    if (this.failClearIds.has(candidateId)) {
      throw new Error(`候補${candidateId}を削除できません。`);
    }
    this.deletedCandidateIds.push(candidateId);
    this.events.push(`delete:${candidateId}`);
  }

  async saveCheckpoint(
    source: SavedDocumentSource,
    options: {
      candidateId: number | null;
      documentPath: string | null;
    },
  ): Promise<RecoveryCandidateSummary> {
    if (this.failWith !== null) throw this.failWith;
    this.saveStarted += 1;
    if (this.saveGate !== null) await this.saveGate;
    this.calls.push({ source, ...options });
    const candidateId = options.candidateId ?? this.nextCandidateId++;
    this.events.push(`save:${candidateId}`);
    return {
      candidate_id: candidateId,
      autosave_path: `browser-recovery://candidate/${candidateId}`,
      document_path: options.documentPath,
      saved_at_ms: 1_000,
      step_count: source.doc.sequence.length,
    };
  }
}

function snapshot(): SavedDocumentSource {
  const doc: Document = {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: {
      vertices: [],
      edges: [],
      next_vertex_id: 0,
      next_edge_id: 0,
    },
    sequence: [],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
  return { doc };
}

describe("AutosaveScheduler", () => {
  it("変更後debounceし、変更が続く間も30秒checkpointで同じ候補を更新する", async () => {
    const clock = new FakeClock();
    const repository = new FakeRepository();
    const scheduler = new AutosaveScheduler({
      repository,
      getSnapshot: snapshot,
      getDocumentPath: () => "browser-file:crane",
      clock,
      debounceMs: 500,
    });

    expect(clock.intervalDelays).toEqual([AUTOSAVE_CHECKPOINT_MS]);
    scheduler.markChanged();
    scheduler.markChanged();
    expect(clock.timeoutDelays).toEqual([500, 500]);
    expect(clock.clearedTimeouts).toHaveLength(1);

    clock.fireTimeouts();
    await scheduler.flushNow();
    expect(repository.calls).toHaveLength(1);
    expect(repository.calls[0]).toMatchObject({
      candidateId: null,
      documentPath: "browser-file:crane",
    });

    scheduler.markChanged();
    scheduler.markChanged();
    clock.fireIntervals();
    await scheduler.flushNow();
    expect(repository.calls).toHaveLength(2);
    expect(repository.calls[1]?.candidateId).toBe(17);

    clock.fireTimeouts();
    await scheduler.flushNow();
    expect(repository.calls).toHaveLength(2);
    scheduler.dispose();
  });

  it("dispose（tab close相当）はtimerだけを止め、候補の削除も新しい保存も行わない", async () => {
    const clock = new FakeClock();
    const repository = new FakeRepository();
    const scheduler = new AutosaveScheduler({
      repository,
      getSnapshot: snapshot,
      clock,
    });
    scheduler.markChanged();

    scheduler.dispose();
    clock.fireTimeouts();
    clock.fireIntervals();
    await scheduler.flushNow();

    expect(repository.calls).toEqual([]);
    expect(repository.deletedCandidateIds).toEqual([]);
    expect(clock.clearedTimeouts).toHaveLength(1);
    expect(clock.clearedIntervals).toHaveLength(1);
  });

  it("保存失敗を日本語の通知へ運び、変更済みrevisionを成功扱いにしない", async () => {
    const clock = new FakeClock();
    const repository = new FakeRepository();
    repository.failWith = new Error("ブラウザ保存領域への書き込みを拒否されました。");
    const messages: string[] = [];
    const scheduler = new AutosaveScheduler({
      repository,
      getSnapshot: snapshot,
      onError: (message) => messages.push(message),
      clock,
    });
    scheduler.markChanged();

    await expect(scheduler.flushNow()).rejects.toThrow(
      "ブラウザ保存領域への書き込みを拒否されました。",
    );
    await Promise.resolve();
    expect(messages).toEqual([
      "ブラウザ保存領域への書き込みを拒否されました。",
    ]);

    repository.failWith = null;
    await scheduler.flushNow();
    expect(repository.calls).toHaveLength(1);
    scheduler.dispose();
  });

  it("明示保存成功を進行中checkpointの後へ直列化し、後発変更を新候補にする", async () => {
    const clock = new FakeClock();
    const repository = new FakeRepository();
    let releaseSave = (): void => undefined;
    repository.saveGate = new Promise<void>((resolve) => {
      releaseSave = resolve;
    });
    const scheduler = new AutosaveScheduler({
      repository,
      getSnapshot: snapshot,
      clock,
    });
    scheduler.markChanged();
    const pendingSave = scheduler.flushNow();
    await Promise.resolve();
    await Promise.resolve();
    expect(repository.saveStarted).toBe(1);

    const explicitSave = scheduler.markExplicitSaveSucceeded();
    expect(repository.deletedCandidateIds).toEqual([]);
    releaseSave();
    await pendingSave;
    await explicitSave;
    expect(repository.events).toEqual(["save:17", "delete:17"]);

    repository.saveGate = null;
    scheduler.markChanged();
    await scheduler.flushNow();
    expect(repository.calls[1]?.candidateId).toBeNull();
    expect(repository.events).toEqual([
      "save:17",
      "delete:17",
      "save:18",
    ]);
    scheduler.dispose();
  });

  it("候補が無い明示保存成功では削除を呼ばない", async () => {
    const repository = new FakeRepository();
    const scheduler = new AutosaveScheduler({
      repository,
      getSnapshot: snapshot,
      clock: new FakeClock(),
    });

    await scheduler.markExplicitSaveSucceeded();

    expect(repository.deletedCandidateIds).toEqual([]);
    scheduler.dispose();
  });

  it("復元元と編集後active候補を分離し、明示保存成功時だけ両方を消す", async () => {
    const repository = new FakeRepository();
    const scheduler = new AutosaveScheduler({
      repository,
      getSnapshot: async () => snapshot(),
      clock: new FakeClock(),
    });

    scheduler.associateRestoredCandidate(5);
    expect(scheduler.associatedCandidateIds()).toEqual([5]);
    scheduler.markChanged();
    await scheduler.flushNow();

    expect(repository.calls[0]?.candidateId).toBeNull();
    expect(scheduler.associatedCandidateIds()).toEqual([5, 17]);
    expect(repository.deletedCandidateIds).toEqual([]);

    await scheduler.markExplicitSaveSucceeded();
    expect(repository.deletedCandidateIds).toEqual([5, 17]);
    expect(scheduler.associatedCandidateIds()).toEqual([]);
    scheduler.dispose();
  });

  it("new/open相当では候補を消さず関連だけ外し、次の編集を新候補にする", async () => {
    const repository = new FakeRepository();
    const scheduler = new AutosaveScheduler({
      repository,
      getSnapshot: snapshot,
      clock: new FakeClock(),
    });
    scheduler.associateRestoredCandidate(6);
    scheduler.markChanged();
    await scheduler.flushNow();
    expect(scheduler.associatedCandidateIds()).toEqual([6, 17]);

    scheduler.releaseCandidateAssociations();
    expect(scheduler.associatedCandidateIds()).toEqual([]);
    expect(repository.deletedCandidateIds).toEqual([]);
    scheduler.markChanged();
    await scheduler.flushNow();

    expect(repository.calls[1]?.candidateId).toBeNull();
    expect(scheduler.associatedCandidateIds()).toEqual([18]);
    scheduler.dispose();
  });

  it("cleanup失敗を常設通知へ渡し、失敗した復元元だけ関連を保持する", async () => {
    const repository = new FakeRepository();
    repository.failClearIds.add(7);
    const messages: string[] = [];
    const scheduler = new AutosaveScheduler({
      repository,
      getSnapshot: snapshot,
      onError: (message) => messages.push(message),
      clock: new FakeClock(),
    });
    scheduler.associateRestoredCandidate(7);
    scheduler.markChanged();
    await scheduler.flushNow();

    await expect(scheduler.markExplicitSaveSucceeded()).rejects.toThrow(
      "作品は保存しましたが、復旧候補を片付けられませんでした",
    );
    await Promise.resolve();
    expect(repository.deletedCandidateIds).toEqual([17]);
    expect(scheduler.associatedCandidateIds()).toEqual([7]);
    expect(messages).toEqual([
      "作品は保存しましたが、復旧候補を片付けられませんでした: 候補7を削除できません。",
    ]);
    scheduler.dispose();
  });
});
