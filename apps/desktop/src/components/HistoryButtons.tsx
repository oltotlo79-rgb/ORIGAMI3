// 上部ツールバーの「元に戻す」「やり直し」。
// ORIGAMI3の戻せる操作は2種類ある(折り角度の変更 / 展開図・手順の変更)ので、
// 次の1回がどちらに効くかを説明として添える(設計原則3b)。

import { useAppStore } from "../store/appStore";
import { redoHintText, undoHintText } from "../lib/undoHint";
import { ToolbarIcon } from "./ToolIcons";

export function HistoryButtons() {
  const undo = useAppStore((s) => s.undo);
  const redo = useAppStore((s) => s.redo);
  const undoHint = useAppStore((s) => undoHintText(s.angleUndoStack.length));
  const redoHint = useAppStore((s) =>
    redoHintText(s.docUndoDepth, s.angleRedoStack.length),
  );

  return (
    <>
      <button
        type="button"
        data-tooltip={`${undoHint} (Ctrl+Z)`}
        onClick={() => void undo()}
      >
        <ToolbarIcon name="undo" />
        元に戻す
      </button>
      <button
        type="button"
        data-tooltip={`${redoHint} (Ctrl+Y)`}
        onClick={() => void redo()}
      >
        <ToolbarIcon name="redo" />
        やり直し
      </button>
    </>
  );
}
