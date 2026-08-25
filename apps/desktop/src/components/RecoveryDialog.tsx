// 復旧ダイアログ(SYS-003 / UI-006): 前回の終了が正常でなかったとき、
// 30秒ごとに控えていた作業中の内容を復元するか尋ねる。
// 専門用語を使わず、何が起きたか・どちらを選ぶとどうなるかを日本語で示す(設計原則3b)。

import { useRef } from "react";
import { useAppStore } from "../store/appStore";
import { ModalDialog } from "./dialogs/ModalDialog";

/** 自動保存した時刻の表示(分からなければ空文字) */
export function formatSavedAt(savedAtMs: number | null): string {
  if (savedAtMs === null) return "";
  return new Date(savedAtMs).toLocaleString("ja-JP", {
    year: "numeric",
    month: "long",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** ファイルのある場所から名前だけを取り出す */
export function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

export function RecoveryDialog() {
  const recovery = useAppStore((s) => s.recovery);
  const resolveRecovery = useAppStore((s) => s.resolveRecovery);
  const restoreButtonRef = useRef<HTMLButtonElement>(null);
  if (!recovery) return null;

  const at = formatSavedAt(recovery.saved_at_ms);
  const target = recovery.document_path;

  return (
    <ModalDialog
      labelledBy="recovery-title"
      initialFocusRef={restoreButtonRef}
      escapeAction={{ kind: "stay" }}
      data-floating-ui="recovery-dialog"
    >
        <h2 id="recovery-title">前回の終了が正常に行われませんでした</h2>
        <p>
          {at
            ? `作業中だった内容(${at}時点)が残っています。復元しますか?`
            : "作業中だった内容が残っています。復元しますか?"}
        </p>
        <p className="hint">
          {target
            ? `元の作品:${fileName(target)}(復元しても、保存するまで元のファイルは変わりません)`
            : "まだ保存していない作品です(復元しても、保存するまでファイルは作られません)"}
        </p>
        <p className="hint">
          「破棄する」を選ぶと、控えていた内容は消えて元に戻せません。
        </p>
        <div className="button-row">
          <button
            ref={restoreButtonRef}
            type="button"
            className="button-primary"
            onClick={() => void resolveRecovery(true)}
          >
            復元する
          </button>
          <button
            type="button"
            className="button-danger"
            onClick={() => void resolveRecovery(false)}
          >
            破棄する
          </button>
        </div>
    </ModalDialog>
  );
}
