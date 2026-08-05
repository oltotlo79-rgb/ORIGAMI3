// 左端ツールレール(64px)。ボタンは10個以内に保つ(要件§2)。
// 「技法」を選んだときだけ、どの技法かを選ぶサブメニューを下に出す
// (常設のボタンは増やさない)。

import { useAppStore, type ToolId } from "../store/appStore";
import { SUPPORTED_TECHNIQUES, TECHNIQUE_LABEL } from "../lib/techniques";

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
      "折る: 立体表示の紙の上をドラッグして折り線を引き、下のパネルで向きと対象の層を選んで折る(平らに畳んだ状態で使える)",
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
          {t.label}
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
              {t.short}
            </button>
          ))}
        </div>
      )}
      <button
        type="button"
        title="全体表示: 展開図と立体表示の両方を、紙全体が見える位置まで戻す"
        className="tool-button"
        onClick={onFitView}
      >
        全体
      </button>
    </nav>
  );
}
