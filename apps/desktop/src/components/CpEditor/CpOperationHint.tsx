// 展開図の左上へ重ねる操作案内。常設の区画は増やさず、
// 現在できることを1行だけ残して、ホイールの詳しい割り当てを畳む。

import { useAppStore, type ToolId } from "../../store/appStore";
import { measureGuide } from "../../lib/measureGuide";
import { wheelHint } from "./interaction";

const CURRENT_ACTION: Record<ToolId, string> = {
  select: "線や点を選び、展開図を動かせます",
  measure: "2つの辺を指定してください",
  mountain: "山折り線: 2回クリック、または矢印キーで動かしてEnterを2回",
  valley: "谷折り線: 2回クリック、または矢印キーで動かしてEnterを2回",
  aux: "補助線: 2回クリック、または矢印キーで動かしてEnterを2回",
  delete: "消したい線をクリックします",
  fold: "折り線や基準にする点・線を選べます",
  pull: "「引く」は3Dの紙をつかんで操作します",
  technique: "「技法」の対象は3Dで選びます",
  construct: "作図に使う点や線を順に選べます",
};

export function CpOperationHint() {
  const activeTool = useAppStore((s) => s.activeTool);
  const measureDraft = useAppStore((s) => s.measureDraft);
  const wheelBehavior = useAppStore((s) => s.wheelBehavior);
  const expanded = useAppStore((s) => s.cpHelpExpanded);
  const toggle = useAppStore((s) => s.toggleCpHelp);
  const currentAction =
    activeTool === "measure"
      ? measureGuide(measureDraft.mode, measureDraft.picks.length)
      : CURRENT_ACTION[activeTool];

  return (
    <aside
      className={`cp-operation-hint ${expanded ? "expanded" : "collapsed"}`}
      aria-label="展開図の操作方法"
      data-floating-ui="cp-operation-hint"
    >
      <div className="cp-operation-current">
        <span>展開図</span>
        <strong
          className="operation-summary-line"
          data-tooltip={currentAction}
        >
          {currentAction}
        </strong>
      </div>
      {expanded && <p>{wheelHint(wheelBehavior)}</p>}
      <button
        type="button"
        className="cp-help-toggle operation-detail-toggle"
        aria-label={`展開図の詳しい操作方法 ${expanded ? "▲" : "▼"}`}
        aria-expanded={expanded}
        data-tooltip={expanded ? "展開図の詳しい操作を折りたたみます" : "展開図の詳しい操作を開きます"}
        onClick={toggle}
      >
        <span className="operation-detail-label">展開図の詳しい操作方法</span>
        <span className="operation-detail-icon">{expanded ? "▲" : "▼"}</span>
      </button>
    </aside>
  );
}
