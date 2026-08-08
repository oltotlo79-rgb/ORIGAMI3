// アプリ全体の「元に戻す」「やり直し」キーボードショートカット。
// DOMは増やさず、ストアに既にある二段構えのundo/redoをそのまま呼ぶ。

import { useEffect } from "react";
import { isEditableTarget } from "../lib/keyboard";
import { useAppStore } from "../store/appStore";

export function HistoryShortcuts() {
  const undo = useAppStore((s) => s.undo);
  const redo = useAppStore((s) => s.redo);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (
        !event.ctrlKey ||
        event.altKey ||
        event.metaKey ||
        isEditableTarget(event.target) ||
        isEditableTarget(document.activeElement)
      ) {
        return;
      }

      const key = event.key.toLowerCase();
      if (key === "z" && !event.shiftKey) {
        event.preventDefault();
        void undo();
      } else if (key === "y" || (key === "z" && event.shiftKey)) {
        event.preventDefault();
        void redo();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [undo, redo]);

  return null;
}
