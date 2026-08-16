// 3Dビューへ重ねる「折り方を決める」札。折り線が決まった間だけ出す。
//
// 下のパネルと同じ言葉・同じ順番にそろえる。パネルの並びは
// 「向き(手前へ折る(谷)/向こうへ折る(山))」→「動かす側(こちら側/反対側)」→「折る」で、
// ここでも同じ言葉・同じ順番にし、今どちらを選んでいるかは色で分かるようにする。
//
// 動かす側を3Dにも置くのは、動く側が黄色く光って見えるのが3Dの中だからで、
// 見ている場所と選ぶ場所を離さないため。
//
// 固定の区画も設定の項目も増やさない。既にある浮かぶ札(PaperActionTip)と
// 同じ見た目・同じ置き場所を使い、3Dだけで「選ぶ→折り方を決める→折る」まで進めるようにする。

import { useAppStore } from "../../store/appStore";

export function FoldDirectionTip() {
  const activeTool = useAppStore((s) => s.activeTool);
  const draft = useAppStore((s) => s.foldDraft);
  const busy = useAppStore((s) => s.foldThroughBusy);
  const updateFoldDraft = useAppStore((s) => s.updateFoldDraft);
  const cancelFoldDraft = useAppStore((s) => s.cancelFoldDraft);
  const commitFoldDraft = useAppStore((s) => s.commitFoldDraft);

  if (activeTool !== "fold" || !draft) return null;

  return (
    <aside
      className="paper-action-tip expanded fold-direction-tip"
      aria-label="折り方を決める"
      data-floating-ui="fold-direction-tip"
    >
      <strong className="paper-action-tip-title">この折り線で折る</strong>
      <div className="paper-action-tip-buttons fold-direction-tip-buttons">
        <span>向き</span>
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
        <span>動かす側</span>
        <button
          type="button"
          aria-pressed={draft.movingSide === "right"}
          disabled={busy}
          data-tooltip="黄色く光る側の紙を動かします"
          onClick={() => updateFoldDraft({ movingSide: "right" })}
        >
          こちら側
        </button>
        <button
          type="button"
          aria-pressed={draft.movingSide === "left"}
          disabled={busy}
          data-tooltip="黄色く光る側の紙を動かします"
          onClick={() => updateFoldDraft({ movingSide: "left" })}
        >
          反対側
        </button>
      </div>
      <div className="paper-action-tip-buttons">
        <button
          type="button"
          disabled={busy}
          data-tooltip="選んだ向きでこの折り線を折ります"
          onClick={() => void commitFoldDraft()}
        >
          {busy ? "折り方を確認中…" : "折る"}
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
