// ipcQueue.tsのユニットテスト: 直列実行の順序保証と、要求番号による
// 「最新でない応答の破棄判定(isLatest)」、および追従計算用の
// 「待ちは最新1件だけ(runLatest)」を検証する。

import { describe, expect, it } from "vitest";
import { createSerialQueue, SUPERSEDED } from "./ipcQueue";

/** 手動でresolve/rejectできるPromise */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** マイクロタスクを数回進める(直列化の「まだ始まらない」確認用) */
async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await Promise.resolve();
  }
}

describe("createSerialQueue", () => {
  it("前の要求が完了するまで次の要求を開始しない(発行順に直列実行)", async () => {
    const q = createSerialQueue();
    const events: string[] = [];
    const first = deferred<string>();

    const p1 = q.run(() => {
      events.push("start1");
      return first.promise;
    });
    const p2 = q.run(() => {
      events.push("start2");
      return Promise.resolve("b");
    });

    await flushMicrotasks();
    expect(events).toEqual(["start1"]); // 1件目が終わるまで2件目は始まらない

    first.resolve("a");
    const [r1, r2] = await Promise.all([p1, p2]);
    expect(events).toEqual(["start1", "start2"]);
    expect(r1).toEqual({ ok: true, value: "a", isLatest: false });
    expect(r2).toEqual({ ok: true, value: "b", isLatest: true });
  });

  it("完了時点で最新の要求ならisLatest=true", async () => {
    const q = createSerialQueue();
    const r = await q.run(() => Promise.resolve(42));
    expect(r).toEqual({ ok: true, value: 42, isLatest: true });
  });

  it("実行中に新しい要求が積まれた応答はisLatest=false(呼び出し側が破棄する)", async () => {
    const q = createSerialQueue();
    const slow = deferred<string>();
    const p1 = q.run(() => slow.promise);
    const p2 = q.run(() => Promise.resolve("新"));

    slow.resolve("古");
    expect((await p1).isLatest).toBe(false); // 古い応答: 反映してはいけない
    expect((await p2).isLatest).toBe(true); // 最新の応答は必ず後から届く
  });

  it("失敗はrejectではなくok:falseで返し、キューは止まらない", async () => {
    const q = createSerialQueue();
    const r1 = await q.run(() => Promise.reject("だめでした"));
    expect(r1).toEqual({ ok: false, error: "だめでした", isLatest: true });

    const r2 = await q.run(() => Promise.resolve(2));
    expect(r2).toEqual({ ok: true, value: 2, isLatest: true });
  });

  it("タスクが同期的に例外を投げてもok:falseで捕捉する", async () => {
    const q = createSerialQueue();
    const boom = new Error("同期の失敗");
    const r1 = await q.run(() => {
      throw boom;
    });
    expect(r1).toEqual({ ok: false, error: boom, isLatest: true });

    const r2 = await q.run(() => Promise.resolve("続行"));
    expect(r2).toEqual({ ok: true, value: "続行", isLatest: true });
  });

  it("3件連続でも実行順・isLatestが正しい", async () => {
    const q = createSerialQueue();
    const order: number[] = [];
    const results = await Promise.all([
      q.run(async () => {
        order.push(1);
        return 1;
      }),
      q.run(async () => {
        order.push(2);
        return 2;
      }),
      q.run(async () => {
        order.push(3);
        return 3;
      }),
    ]);
    expect(order).toEqual([1, 2, 3]);
    expect(results.map((r) => r.isLatest)).toEqual([false, false, true]);
  });
});

describe("createSerialQueue.runLatest(待ちは最新1件だけ)", () => {
  it("実行中に3件積んだら、最後の1件だけが実行される", async () => {
    const q = createSerialQueue();
    const running = deferred<string>();
    const started: string[] = [];

    const p0 = q.runLatest(() => {
      started.push("実行中");
      return running.promise;
    });
    await flushMicrotasks();
    expect(started).toEqual(["実行中"]);

    const p1 = q.runLatest(() => {
      started.push("1");
      return Promise.resolve("1");
    });
    const p2 = q.runLatest(() => {
      started.push("2");
      return Promise.resolve("2");
    });
    const p3 = q.runLatest(() => {
      started.push("3");
      return Promise.resolve("3");
    });

    // 追い越された2件は実行されずに返る(呼び出し側は何もしない)
    expect(await p1).toEqual({ ok: false, error: SUPERSEDED, isLatest: false });
    expect(await p2).toEqual({ ok: false, error: SUPERSEDED, isLatest: false });

    running.resolve("実行中の結果");
    expect(await p0).toEqual({ ok: true, value: "実行中の結果", isLatest: false });
    expect(await p3).toEqual({ ok: true, value: "3", isLatest: true });
    expect(started).toEqual(["実行中", "3"]); // 1と2は動いていない
  });

  it("待ちが無ければ通常どおり順番に実行する", async () => {
    const q = createSerialQueue();
    const order: string[] = [];
    const r1 = await q.runLatest(async () => {
      order.push("a");
      return "a";
    });
    const r2 = await q.runLatest(async () => {
      order.push("b");
      return "b";
    });
    expect(order).toEqual(["a", "b"]);
    expect(r1.ok && r1.value).toBe("a");
    expect(r2.ok && r2.value).toBe("b");
  });

  it("編集系(run)の後ろに積まれ、順番は入れ替わらない", async () => {
    const q = createSerialQueue();
    const edit = deferred<string>();
    const order: string[] = [];

    const pEdit = q.run(() => {
      order.push("編集");
      return edit.promise;
    });
    const pPose = q.runLatest(() => {
      order.push("追従");
      return Promise.resolve("追従の結果");
    });

    await flushMicrotasks();
    expect(order).toEqual(["編集"]); // 編集が終わるまで追従は始まらない

    edit.resolve("編集の結果");
    await Promise.all([pEdit, pPose]);
    expect(order).toEqual(["編集", "追従"]);
  });

  it("待っている間に編集が積まれたら、その後の最新1件は編集の後ろに回る", async () => {
    const q = createSerialQueue();
    const running = deferred<string>();
    const order: string[] = [];

    const p0 = q.run(() => {
      order.push("実行中");
      return running.promise;
    });
    await flushMicrotasks();

    const p1 = q.runLatest(async () => {
      order.push("追従1");
      return "追従1";
    });
    const p2 = q.run(async () => {
      order.push("編集");
      return "編集";
    });
    const p3 = q.runLatest(async () => {
      order.push("追従2");
      return "追従2";
    });

    running.resolve("済");
    await Promise.all([p0, p1, p2, p3]);
    // 追従1は編集より前に積まれているので追い越されず、そのまま実行される
    expect(order).toEqual(["実行中", "追従1", "編集", "追従2"]);
  });

  it("失敗してもキューは止まらず、次の要求が実行される", async () => {
    const q = createSerialQueue();
    const r1 = await q.runLatest(() => Promise.reject("だめでした"));
    expect(r1).toEqual({ ok: false, error: "だめでした", isLatest: true });
    const r2 = await q.runLatest(() => Promise.resolve("続行"));
    expect(r2).toEqual({ ok: true, value: "続行", isLatest: true });
  });
});
