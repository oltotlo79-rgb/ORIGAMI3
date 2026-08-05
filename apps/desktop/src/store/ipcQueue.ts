// IPC要求の直列化キュー。
// バックエンドはスレッドプール+Mutexで実行され、ロック獲得順にFIFO保証がない。
// 素早い連続操作(undo連打・将来のドラッグ編集など)で適用順が入れ替わると
// undo履歴や画面表示が壊れるため、フロント側で「前の要求が完了してから次を送る」
// ことで適用順を発行順に固定する。
// あわせて単調増加の要求番号を持ち、完了時点でより新しい要求が積まれていた応答には
// isLatest=false を付けて返す。呼び出し側はそれを見て古い応答の状態反映を破棄する
// (古いdocが新しいdocを上書きするのを防ぐ。最新の応答は必ず後から届く)。
//
// 積み方は2種類ある:
//   run       … 従来どおり全件をFIFOで実行する(編集系。1件も落とせない)
//   runLatest … 待ち行列に置けるのは1件だけで、後から来た要求が上書きする
//               (追従計算系。手順再生で毎フレーム要求が出ても行列が伸びず、
//                表示が計算待ちで遅れない)

/** 後から来た要求に追い越されて実行されなかったことを表す印 */
export const SUPERSEDED = Symbol("superseded");

export type QueueResult<T> =
  | { ok: true; value: T; isLatest: boolean }
  | { ok: false; error: unknown; isLatest: boolean };

export interface SerialQueue {
  /** 要求を積む。返るPromiseはrejectしない(失敗はok:falseで表す) */
  run<T>(task: () => Promise<T>): Promise<QueueResult<T>>;
  /**
   * 最新の1件だけを積む。すでに待っている(まだ始まっていない)同種の要求が
   * あればそれを取り消して置き換える。取り消された側は
   * { ok:false, error:SUPERSEDED, isLatest:false } で返るので、呼び出し側は
   * 何もしない(isLatest=falseなので通常の破棄規約にそのまま乗る)。
   */
  runLatest<T>(task: () => Promise<T>): Promise<QueueResult<T>>;
}

/** 待ち行列に置かれた1件 */
interface Slot {
  seq: number;
  task: () => Promise<unknown>;
  settle: (result: QueueResult<unknown>) => void;
}

/** 待ち行列上の1つの枠。中身(slot)は実行開始まで差し替えられる */
interface Replaceable {
  slot: Slot;
}

export function createSerialQueue(): SerialQueue {
  let tail: Promise<unknown> = Promise.resolve();
  let latestSeq = 0;
  /** 差し替えできる枠。実行が始まるか、後ろにrunが積まれたらnullに戻す
   * (nullにすると次のrunLatestは行列の最後尾に新しい枠を作る) */
  let replaceable: Replaceable | null = null;

  async function execute<T>(seq: number, task: () => Promise<T>): Promise<QueueResult<T>> {
    try {
      const value = await task();
      return { ok: true, value, isLatest: seq === latestSeq };
    } catch (error) {
      return { ok: false, error, isLatest: seq === latestSeq };
    }
  }

  return {
    run<T>(task: () => Promise<T>): Promise<QueueResult<T>> {
      const seq = ++latestSeq;
      // これ以降のrunLatestは、この要求より後ろに積まれる(順番の逆転を防ぐ)
      replaceable = null;
      // 前の要求の完了(成功・失敗どちらでも)を待ってから実行する
      const result = tail.then(() => execute(seq, task));
      tail = result;
      return result;
    },

    runLatest<T>(task: () => Promise<T>): Promise<QueueResult<T>> {
      const seq = ++latestSeq;
      let settle!: (result: QueueResult<T>) => void;
      const promise = new Promise<QueueResult<T>>((resolve) => {
        settle = resolve;
      });
      const slot: Slot = {
        seq,
        task,
        // 実際に渡す値は必ずQueueResult<T>なので、ここでの緩めは安全
        settle: settle as unknown as (result: QueueResult<unknown>) => void,
      };

      if (replaceable !== null) {
        // まだ始まっていない枠を最新の内容で置き換える(古い方は実行しない)
        const superseded = replaceable.slot;
        replaceable.slot = slot;
        superseded.settle({ ok: false, error: SUPERSEDED, isLatest: false });
        return promise;
      }

      const cell: Replaceable = { slot };
      replaceable = cell;
      tail = tail.then(async () => {
        // 順番が来た時点で枠に入っている最新の内容を実行する
        if (replaceable === cell) replaceable = null;
        cell.slot.settle(await execute(cell.slot.seq, cell.slot.task));
      });
      return promise;
    },
  };
}
