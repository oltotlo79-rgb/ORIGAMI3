// 左端ツールレール(64px)。ボタンは10個以内に保つ(要件§2)。
// 「技法」を選んだときだけ、どの技法かを選ぶサブメニューを下に出す
// (常設のボタンは増やさない)。

import { useAppStore, type ToolId } from "../store/appStore";
import { SUPPORTED_TECHNIQUES, TECHNIQUE_LABEL } from "../lib/techniques";
import { CONSTRUCT_LABEL, type ConstructKind } from "../lib/construct";
import { ToolIcon, ToolSubIcon } from "./ToolIcons";

/** 作図の種類とその説明(サブメニューの並び順) */
const CONSTRUCT_KINDS: ConstructKind[] = ["bisector", "perpendicular", "divide", "angle"];
const CONSTRUCT_TITLE: Record<ConstructKind, string> = {
  bisector: "角を半分に分ける線: 角の1本目の先 → 角の点 → もう1本の先の順にクリック",
  perpendicular: "線に直角な線: 直角にしたい線 → 通したい点の順にクリック",
  divide: "区間を等分する目印: 始点 → 終点の順にクリック(等分の数は右で選ぶ)",
  angle: "決まった角度の方向線: 出したい点をクリック(刻みは右で選ぶ)",
};

const TOOLS: { id: ToolId; label: string; title: string }[] = [
  { id: "select", label: "選択", title: "選択: クリックで線や点を選ぶ。ドラッグで範囲選択" },
  { id: "mountain", label: "山", title: "山折り線: 2回クリックで線を引く(Escで中止)" },
  { id: "valley", label: "谷", title: "谷折り線: 2回クリックで線を引く(Escで中止)" },
  { id: "aux", label: "補助", title: "補助線: 2回クリックで線を引く(Escで中止)" },
  { id: "delete", label: "削除", title: "削除: クリックした線を消す" },
  {
    id: "fold",
    label: "折る",
    title:
      "折る: 立体表示で紙をつかんでドラッグすると、その紙が離した位置へ倒れて折れる(Shift=重なった紙を全部、Alt=1枚だけ)。位置をきっちり決めたいときはCtrl+ドラッグで折り線を引き、下のパネルで指定する(平らに畳んだ状態で使える)",
  },
  {
    id: "pull",
    label: "引く",
    title:
      "引く: 立体表示で紙をつかんでドラッグすると、折り線のつじつまを保ったまま全体が連動して動く(鶴の羽を広げて胴を膨らませる、など)。折り上がった作品でも使える。右ドラッグで視点を回せる",
  },
  {
    id: "construct",
    label: "作図",
    title:
      "作図の補助: 角を二等分する線・線に直角な線・区間を等分する目印・22.5°刻みの方向線を、あたりの線(補助線)として引く。下のサブメニューで選び、展開図で必要な点や線をクリックする",
  },
  {
    id: "technique",
    label: "技法",
    title:
      "技法: 段折り・中割り折り・かぶせ折り・開いてつぶす・花弁折りを選んで折る。下のサブメニューから技法を選び、立体表示で層をクリック→折り線をドラッグして下のパネルで適用する",
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
          title={t.title}
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
              title={t.title}
              aria-label={TECHNIQUE_LABEL[t.kind]}
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
              title={CONSTRUCT_TITLE[k]}
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
        title="全体表示: 展開図と立体表示の両方を、紙全体が見える位置まで戻す"
        className="tool-button"
        onClick={onFitView}
      >
        <ToolIcon tool="fit" />
        全体
      </button>
    </nav>
  );
}
