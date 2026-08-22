// 左端ツールレール(64px)。道具は既存の同じ縦並びへ追加する。
// 「技法」を選んだときだけ、どの技法かを選ぶサブメニューを下に出す
// (常設のボタンは増やさない)。

import { useAppStore, type ToolId } from "../store/appStore";
import { SUPPORTED_TECHNIQUES, TECHNIQUE_LABEL } from "../lib/techniques";
import { CONSTRUCT_LABEL, type ConstructKind } from "../lib/construct";
import { ToolIcon, ToolSubIcon } from "./ToolIcons";

/** 作図の種類とその説明(サブメニューの並び順) */
const CONSTRUCT_KINDS: ConstructKind[] = ["bisector", "perpendicular", "divide", "angle"];
const CONSTRUCT_TOOLTIP: Record<ConstructKind, string> = {
  bisector: "角を二等分する3点を順に選びます",
  perpendicular: "線と点を選び、垂線を引きます",
  divide: "区間の両端を選び、等分する目印を付けます",
  angle: "点を選び、決まった角度の方向線を引きます",
};

const TOOLS: { id: ToolId; label: string; tooltip: string }[] = [
  {
    id: "select",
    label: "選択",
    tooltip: "線や点を選びます。Ctrlで複数選択できます",
  },
  {
    id: "measure",
    label: "測る",
    tooltip: "角度や長さ、2つの点の距離を測ります",
  },
  { id: "mountain", label: "山", tooltip: "2回クリックして山折り線を引きます" },
  { id: "valley", label: "谷", tooltip: "2回クリックして谷折り線を引きます" },
  { id: "aux", label: "補助", tooltip: "2回クリックして補助線を引きます" },
  { id: "delete", label: "削除", tooltip: "クリックした線を削除します" },
  {
    id: "fold",
    label: "折る",
    tooltip:
      "3Dの紙をドラッグして折ります。点や線を合わせて折り目を決める8通りの方法も、選ぶと下のパネルに出ます",
  },
  {
    id: "pull",
    label: "引く",
    tooltip: "3Dの紙をドラッグして、全体を連動させます",
  },
  {
    id: "construct",
    label: "作図",
    tooltip: "二等分線や垂線などの補助線を引きます",
  },
  {
    id: "technique",
    label: "技法",
    tooltip: "段折りなどの技法を選んで折ります",
  },
];

interface Props {
  /** 「全体表示」ボタン: 展開図と立体表示を紙全体が収まる位置に戻す */
  onFitView: () => void;
}

export function ToolRail({ onFitView }: Props) {
  const activeTool = useAppStore((s) => s.activeTool);
  const setTool = useAppStore((s) => s.setTool);
  const techniqueDraft = useAppStore((s) => s.techniqueDraft);
  const beginTechnique = useAppStore((s) => s.beginTechnique);
  const construct = useAppStore((s) => s.construct);
  const setConstruct = useAppStore((s) => s.setConstruct);

  return (
    <nav className="tool-rail">
      {TOOLS.map((t) => (
        <button
          key={t.id}
          type="button"
          data-tooltip={t.tooltip}
          className={activeTool === t.id ? "tool-button active" : "tool-button"}
          onClick={() => setTool(t.id)}
        >
          <ToolIcon tool={t.id} />
          <span className="tool-label">{t.label}</span>
        </button>
      ))}
      {activeTool === "technique" && (
        <div className="tool-submenu" role="group" aria-label="技法を選ぶ">
          {SUPPORTED_TECHNIQUES.map((t) => (
            <button
              key={t.kind}
              type="button"
              data-tooltip={`${t.kind === "Simple" ? "層操作" : TECHNIQUE_LABEL[t.kind]}を使います`}
              aria-label={t.kind === "Simple" ? "層操作" : TECHNIQUE_LABEL[t.kind]}
              className={
                techniqueDraft?.kind === t.kind
                  ? "tool-button small active"
                  : "tool-button small"
              }
              onClick={() => beginTechnique(t.kind)}
            >
              <ToolSubIcon name={t.kind} />
              {t.short}
            </button>
          ))}
        </div>
      )}
      {activeTool === "construct" && (
        <div className="tool-submenu" role="group" aria-label="作図の種類を選ぶ">
          {CONSTRUCT_KINDS.map((k) => (
            <button
              key={k}
              type="button"
              data-tooltip={CONSTRUCT_TOOLTIP[k]}
              className={
                construct.kind === k ? "tool-button small active" : "tool-button small"
              }
              onClick={() => setConstruct({ kind: k })}
            >
              <ToolSubIcon name={k} />
              {CONSTRUCT_LABEL[k]}
            </button>
          ))}
          {construct.kind === "divide" && (
            <select
              className="tool-select"
              aria-label="いくつに等分するか"
              data-tooltip="区間を何等分するか選びます"
              value={construct.divisions}
              onChange={(e) => setConstruct({ divisions: Number(e.target.value) })}
            >
              {[2, 3, 4, 5, 6, 7, 8].map((n) => (
                <option key={n} value={n}>{`${n}等分`}</option>
              ))}
            </select>
          )}
          {construct.kind === "angle" && (
            <select
              className="tool-select"
              aria-label="角度の刻み"
              data-tooltip="方向線の角度刻みを選びます"
              value={construct.stepDeg}
              onChange={(e) => setConstruct({ stepDeg: Number(e.target.value) })}
            >
              {[22.5, 45, 30, 15].map((d) => (
                <option key={d} value={d}>{`${d}°`}</option>
              ))}
            </select>
          )}
        </div>
      )}
      <button
        type="button"
        data-tooltip="展開図と3Dを紙全体が見える位置へ戻します"
        className="tool-button"
        onClick={onFitView}
      >
        <ToolIcon tool="fit" />
        全体
      </button>
    </nav>
  );
}
