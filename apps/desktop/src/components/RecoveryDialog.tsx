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
    second: "2-digit",
  });
}

/** ファイルのある場所から名前だけを取り出す */
export function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

export function RecoveryDialog() {
  const recovery = useAppStore((s) => s.recovery);
  const recoveryChoices = useAppStore((s) => s.recoveryChoices);
  const recoveryDismissed = useAppStore((s) => s.recoveryDismissed);
  const recoveryBusy = useAppStore((s) => s.recoveryBusy);
  const resolveRecovery = useAppStore((s) => s.resolveRecovery);
  const dismissRecovery = useAppStore((s) => s.dismissRecovery);
  const restoreButtonRef = useRef<HTMLButtonElement>(null);
  if (!recovery || recoveryDismissed) return null;
  const choices = recoveryChoices.length > 0 ? recoveryChoices : [recovery];

  return (
    <ModalDialog
      labelledBy="recovery-title"
      initialFocusRef={restoreButtonRef}
      escapeAction={{ kind: "stay" }}
      data-floating-ui="recovery-dialog"
    >
      <h2 id="recovery-title">前回の終了が正常に行われませんでした</h2>
      <p>
        {choices.length === 1
          ? "作業中だった内容が残っています。どうしますか?"
          : `作業中だった内容が${choices.length}件残っています。内容ごとに選べます。`}
      </p>
      <ul aria-label="前回の作業">
        {choices.map((choice, index) => {
          const at = formatSavedAt(choice.saved_at_ms);
          const target = choice.document_path;
          return (
            <li key={choice.candidate_id}>
              <p>
                {at ? `保存した日時: ${at}` : "保存した日時: 分かりません"}
              </p>
              <p className="hint">
                {target
                  ? `元の作品: ${fileName(target)}`
                  : "元の作品: まだ保存していない作品"}
              </p>
              <p className="hint">
                手順数: {choice.step_count === null ? "分かりません" : `${choice.step_count}件`}
              </p>
              <div className="button-row">
                <button
                  ref={index === 0 ? restoreButtonRef : undefined}
                  type="button"
                  className="button-primary"
                  disabled={recoveryBusy}
                  onClick={() =>
                    void resolveRecovery(true, choice.candidate_id)
                  }
                >
                  復元する
                </button>
                <button
                  type="button"
                  className="button-danger"
                  disabled={recoveryBusy}
                  onClick={() =>
                    void resolveRecovery(false, choice.candidate_id)
                  }
                >
                  破棄する
                </button>
              </div>
            </li>
          );
        })}
      </ul>
      <p className="hint">
        復元しても、保存するまで元のファイルは変わりません。「破棄する」を選ぶと、控えていた内容は消えて元に戻せません。
      </p>
      <div className="button-row">
        <button
          type="button"
          disabled={recoveryBusy}
          onClick={dismissRecovery}
        >
          あとで確認する
        </button>
      </div>
    </ModalDialog>
  );
}
