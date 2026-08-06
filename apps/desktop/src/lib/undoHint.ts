// 「元に戻す」「やり直し」が“どちらの履歴”に効くのかを日本語で説明する。
// ORIGAMI3の戻せる操作は2種類ある:
//   1. 折り角度の変更(3Dの形。作品データではないのでファイルに残らない)
//   2. 展開図・手順の変更(作品データ。edit_undo/edit_redoの履歴に載る)
// どちらが使われるか分からないと「線が消えた」と驚くので、ボタンの説明で先に
// 知らせる(設計原則3b: 何が起きるか押す前に分かること)。

/** 元に戻すボタンの説明。角度の履歴が残っていれば、次の1回は角度が戻る */
export function undoHintText(angleUndoCount: number): string {
  return angleUndoCount > 0
    ? "折り角度の変更を戻します(折り線はそのまま残ります)"
    : "折り線の追加など、展開図・手順の変更を戻します";
}

/**
 * やり直しボタンの説明。作品データを戻したぶんが残っていればそちらが先で、
 * その後に角度の変更をやり直す(操作した順に復元される)。
 */
export function redoHintText(
  docUndoDepth: number,
  angleRedoCount: number,
): string {
  if (docUndoDepth > 0) {
    return "折り線の追加など、展開図・手順の変更をやり直します";
  }
  return angleRedoCount > 0
    ? "折り角度の変更をやり直します"
    : "やり直せる操作はありません";
}
