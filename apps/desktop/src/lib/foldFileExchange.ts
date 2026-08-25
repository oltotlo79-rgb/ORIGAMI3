export type FileDialogFilter = {
  name: string;
  extensions: string[];
};

const ORI3_FILE_FILTER: FileDialogFilter = {
  name: "ORIGAMI3作品",
  extensions: ["ori3"],
};

const FOLD_FILE_FILTER: FileDialogFilter = {
  name: "ほかの折り紙ソフトのファイル",
  extensions: ["fold"],
};

/**
 * 8-Dの接続点。appStoreが読込/書出しのFoldIssueを保持し、既存の
 * ContextPanel/ExportDialogがfoldIssueNoticeだけを表示するまでfalseを維持する。
 * trueへ変えると、既存の「開く」「書き出し」に準備済みの選択肢が加わる。
 */
export const FOLD_FILE_EXCHANGE_READY = false as const;

export function openFileFiltersForReadiness(
  ready: boolean,
): FileDialogFilter[] {
  return ready ? [ORI3_FILE_FILTER, FOLD_FILE_FILTER] : [ORI3_FILE_FILTER];
}

export function openFileTooltipForReadiness(ready: boolean): string {
  return ready
    ? "保存した作品または、ほかの折り紙ソフトのファイルを開きます"
    : "保存した作品(.ori3)を開きます";
}

export const OPEN_FILE_FILTERS = openFileFiltersForReadiness(
  FOLD_FILE_EXCHANGE_READY,
);
export const OPEN_FILE_TOOLTIP = openFileTooltipForReadiness(
  FOLD_FILE_EXCHANGE_READY,
);
export const SAVE_FILE_FILTERS: FileDialogFilter[] = [ORI3_FILE_FILTER];
