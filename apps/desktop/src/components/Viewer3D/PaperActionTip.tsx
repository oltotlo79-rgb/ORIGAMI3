// 3Dで紙そのものをクリックしたときに出す、引く・膨らますへの直接の入口。
// 初回は説明つき、畳んだ後は小さなヒントとして同じ場所に残す。
//
// 使えない入口は押せない状態にし、理由を必ず見せる(押せるのに何も起きない状態を作らない)。
// 理由は吹き出しだけに頼らず、案内の本文にもそのまま出す。押せないボタンは
// マウスの出入りを受け取らないため、吹き出しだけでは読めないことがあるため。

import {
  inflateBlockReason,
  pullBlockedOf,
  useAppStore,
} from "../../store/appStore";

/** 案内の本文。使えない入口があるときは、その理由をそのまま並べる。 */
export function paperActionTipMessage(
  pullBlocked: string | null,
  inflateBlocked: string | null,
): string {
  const reasons = [
    ...new Set(
      [pullBlocked, inflateBlocked].filter(
        (reason): reason is string => reason !== null,
      ),
    ),
  ];
  if (reasons.length === 0) {
    return "「引く」で紙を連動して動かしたり、「ふくらます」で袋のような丸みをつけたりできます。";
  }
  return reasons.join(" ");
}

export function PaperActionTip() {
  const activeTool = useAppStore((s) => s.activeTool);
  const visible = useAppStore((s) => s.paperActionTipVisible);
  const expanded = useAppStore((s) => s.paperActionTipExpanded);
  const collapse = useAppStore((s) => s.collapsePaperActionTip);
  const expand = useAppStore((s) => s.expandPaperActionTip);
  const hide = useAppStore((s) => s.hidePaperActionTip);
  const setTool = useAppStore((s) => s.setTool);
  const setSelection = useAppStore((s) => s.setSelection);
  const setSoft = useAppStore((s) => s.setSoft);
  const pullBlocked = useAppStore(pullBlockedOf);
  const inflateBlocked = useAppStore(inflateBlockReason);

  if (!visible || activeTool !== "select") return null;

  if (!expanded) {
    return (
      <button
        type="button"
        className="paper-action-tip compact"
        data-floating-ui="paper-action-tip"
        data-tooltip="この紙を動かす・ふくらます案内を開きます"
        onClick={expand}
      >
        ↔ この紙を動かす・ふくらます
      </button>
    );
  }

  // 両方とも使えないときは、見出しでも「今はできない」と分かるようにする
  const allBlocked = pullBlocked !== null && inflateBlocked !== null;
  const title = allBlocked
    ? "今はこの紙を動かせません"
    : "この紙、もっと動かせます！";

  return (
    <aside
      className="paper-action-tip expanded"
      aria-label="クリックした紙でできること"
      data-floating-ui="paper-action-tip"
    >
      <button
        type="button"
        className="paper-action-tip-close"
        aria-label="紙の操作案内を小さくする"
        onClick={collapse}
      >
        ×
      </button>
      <strong className="paper-action-tip-title" data-tooltip={title}>
        {title}
      </strong>
      <p>{paperActionTipMessage(pullBlocked, inflateBlocked)}</p>
      <div className="paper-action-tip-buttons">
        <button
          type="button"
          disabled={pullBlocked !== null}
          data-tooltip={pullBlocked ?? "この紙を引いて動かします"}
          onClick={() => {
            hide();
            setTool("pull");
          }}
        >
          ↔ この紙を引いて動かす
        </button>
        <button
          type="button"
          disabled={inflateBlocked !== null}
          data-tooltip={inflateBlocked ?? "この紙をふくらませます"}
          onClick={() => {
            setSelection({ edgeIds: [], vertexIds: [] });
            setSoft({ soft_enabled: true });
            collapse();
          }}
        >
          ◯ この紙をふくらます
        </button>
      </div>
    </aside>
  );
}
