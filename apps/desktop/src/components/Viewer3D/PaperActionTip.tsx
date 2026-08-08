// 3Dで紙そのものを選んだときに出す、引く・膨らますへの直接の入口。
// 初回は説明つき、畳んだ後は小さなヒントとして同じ場所に残す。

import { useAppStore } from "../../store/appStore";

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

  if (!visible || activeTool !== "select") return null;

  if (!expanded) {
    return (
      <button type="button" className="paper-action-tip compact" onClick={expand}>
        ↔ この紙を動かす・ふくらます
      </button>
    );
  }

  return (
    <aside className="paper-action-tip expanded" aria-label="選んだ紙でできること">
      <button
        type="button"
        className="paper-action-tip-close"
        aria-label="紙の操作案内を小さくする"
        onClick={collapse}
      >
        ×
      </button>
      <strong>この紙、もっと動かせます！</strong>
      <p>
        「引く」で紙を連動して動かしたり、「ふくらます」で袋のような丸みをつけたりできます。
      </p>
      <div className="paper-action-tip-buttons">
        <button
          type="button"
          onClick={() => {
            hide();
            setTool("pull");
          }}
        >
          ↔ この紙を引いて動かす
        </button>
        <button
          type="button"
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
