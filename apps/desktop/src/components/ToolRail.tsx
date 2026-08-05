// 左端ツールレール(64px)。ボタンは10個以内に保つ(要件§2)。

import { useAppStore, type ToolId } from "../store/appStore";

const TOOLS: { id: ToolId; label: string; title: string }[] = [
  { id: "select", label: "選択", title: "選択: クリックで線や点を選ぶ。ドラッグで範囲選択" },
  { id: "mountain", label: "山", title: "山折り線: 2回クリックで線を引く(Escで中止)" },
  { id: "valley", label: "谷", title: "谷折り線: 2回クリックで線を引く(Escで中止)" },
  { id: "aux", label: "補助", title: "補助線: 2回クリックで線を引く(Escで中止)" },
  { id: "delete", label: "削除", title: "削除: クリックした線を消す" },
];

interface Props {
  /** 「全体表示」ボタン: 展開図と立体表示を紙全体が収まる位置に戻す */
  onFitView: () => void;
}

export function ToolRail({ onFitView }: Props) {
  const activeTool = useAppStore((s) => s.activeTool);
  const setTool = useAppStore((s) => s.setTool);

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
