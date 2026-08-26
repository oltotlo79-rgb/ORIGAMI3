// 3Dビューへ重ねる「折り方を決める」札。折り線が決まった間だけ出す。
//
// 折り返す紙は1つ目の選択から自動で決め、黄色で見せる。二択として聞き直さず、
// 意図と違うときだけ単一の「反対側の紙を折り返す」で直せるようにする。
// 黄色を見る場所と切り替える場所を離さないため、この操作は3Dの札だけに置く。
//
// 固定の区画も設定の項目も増やさない。既にある浮かぶ札(PaperActionTip)と
// 同じ見た目・同じ置き場所を使い、3Dだけで「選ぶ→折り方を決める→折る」まで進めるようにする。

import {
  automaticMovingSide,
  initialMovingSide,
  useAppStore,
} from "../../store/appStore";
import {
  FoldTargetControl,
  foldTargetCommitBlocked,
  isCreaseOnlyFoldTarget,
} from "../FoldTargetControl";

export function FoldDirectionTip() {
  const activeTool = useAppStore((s) => s.activeTool);
  const draft = useAppStore((s) => s.foldDraft);
  const alignDraft = useAppStore((s) => s.alignDraft);
  const busy = useAppStore((s) => s.foldThroughBusy);
  const updateFoldDraft = useAppStore((s) => s.updateFoldDraft);
  const cancelFoldDraft = useAppStore((s) => s.cancelFoldDraft);
  const commitFoldDraft = useAppStore((s) => s.commitFoldDraft);

  if (activeTool !== "fold" || !draft) return null;

  const creaseOnly = isCreaseOnlyFoldTarget(draft);
  const commitDisabled = busy || foldTargetCommitBlocked(draft);
  const automaticSide = automaticMovingSide(draft.line, alignDraft?.picks[0]);
  const changedFromAutomatic =
    alignDraft !== null &&
    draft.movingSide !== initialMovingSide(draft.line, alignDraft.picks[0]);
  const sideMessage = !alignDraft
    ? "黄色で示した紙を折り返します"
    : changedFromAutomatic
      ? "反対側へ切り替えた紙を黄色で示しています"
      : automaticSide === null
        ? "自動で決められません。今は黄色で示した紙を折り返します"
        : "1つ目に選んだものがある紙を黄色で示しています";

  return (
    <aside
      className="paper-action-tip expanded fold-direction-tip"
      aria-label="折り方を決める"
      data-floating-ui="fold-direction-tip"
    >
      <strong className="paper-action-tip-title">この折り線で折る</strong>
      <div className="paper-action-tip-buttons fold-direction-tip-buttons">
        <span className="row-label">向き</span>
        <button
          type="button"
          aria-pressed={draft.direction === "Up"}
          disabled={busy}
          data-tooltip="こちら側へ紙を倒します。折り目は谷折りになります"
          onClick={() => updateFoldDraft({ direction: "Up" })}
        >
          手前へ折る(谷)
        </button>
        <button
          type="button"
          aria-pressed={draft.direction === "Down"}
          disabled={busy}
          data-tooltip="向こう側へ紙を倒します。折り目は山折りになります"
          onClick={() => updateFoldDraft({ direction: "Down" })}
        >
          向こうへ折る(山)
        </button>
      </div>
      <div className="paper-action-tip-buttons fold-direction-tip-buttons">
        <span className="row-label">折り返す紙</span>
        <span aria-live="polite">
          {sideMessage}
        </span>
        <button
          type="button"
          disabled={busy}
          data-tooltip="黄色で示す紙を反対側へ切り替えます"
          onClick={() =>
            updateFoldDraft({
              movingSide: draft.movingSide === "right" ? "left" : "right",
            })
          }
        >
          反対側の紙を折り返す
        </button>
      </div>
      {alignDraft && (
        <FoldTargetControl draft={draft} disabled={busy} variant="viewer3d" />
      )}
      <div className="paper-action-tip-buttons">
        <button
          type="button"
          disabled={commitDisabled}
          data-tooltip={
            creaseOnly
              ? "いちばん上の紙に折り目だけを付けます"
              : "選んだ向きでこの折り線を折ります"
          }
          onClick={() => void commitFoldDraft()}
        >
          {busy ? "折り方を確認中…" : creaseOnly ? "折り目を付ける" : "折る"}
        </button>
        <button
          type="button"
          disabled={busy}
          data-tooltip="この折り線を捨てます"
          onClick={() => cancelFoldDraft()}
        >
          やめる
        </button>
      </div>
    </aside>
  );
}
