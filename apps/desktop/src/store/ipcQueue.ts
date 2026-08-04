// IPC要求の直列化キュー。
// バックエンドはスレッドプール+Mutexで実行され、ロック獲得順にFIFO保証がない。
// 素早い連続操作(undo連打・将来のドラッグ編集など)で適用順が入れ替わると
// undo履歴や画面表示が壊れるため、フロント側で「前の要求が完了してから次を送る」
// ことで適用順を発行順に固定する。
// あわせて単調増加の要求番号を持ち、完了時点でより新しい要求が積まれていた応答には
// isLatest=false を付けて返す。呼び出し側はそれを見て古い応答の状態反映を破棄する
// (古いdocが新しいdocを上書きするのを防ぐ。最新の応答は必ず後から届く)。

export type QueueResult<T> =
  | { ok: true; value: T; isLatest: boolean }
  | { ok: false; error: unknown; isLatest: boolean };

export interface SerialQueue {
  /** 要求を積む。返るPromiseはrejectしない(失敗はok:falseで表す) */
  run<T>(task: () => Promise<T>): Promise<QueueResult<T>>;
}

export function createSerialQueue(): SerialQueue {
  let tail: Promise<unknown> = Promise.resolve();
  let latestSeq = 0;

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
      // 前の要求の完了(成功・失敗どちらでも)を待ってから実行する
      const result = tail.then(() => execute(seq, task));
      tail = result;
      return result;
    },
  };
}
