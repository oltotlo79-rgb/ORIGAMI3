import { open, save } from "@tauri-apps/plugin-dialog";
import { EXPORT_CHOICES } from "./dialogs/exportChoices";
import { HistoryButtons } from "./HistoryButtons";
import { ToolbarBrandMark } from "./ToolbarBrandMark";
import { ToolbarIcon } from "./ToolIcons";
import { useAppStore } from "../store/appStore";

const ORI3_FILTERS = [{ name: "ORIGAMI3作品", extensions: ["ori3"] }];
const EXPORT_GUIDANCE = `${EXPORT_CHOICES.map((choice) => choice.label).join("、")}を書き出します`;

/** 実際に選べる書き出し形式を、そのまま案内する上部ボタン。 */
export function ExportButton({ onClick }: { onClick: () => void }) {
  return (
    <button type="button" data-tooltip={EXPORT_GUIDANCE} onClick={onClick}>
      <ToolbarIcon name="export" />
      書き出し
    </button>
  );
}

export function AppToolbar({ onOpenHelp }: { onOpenHelp: () => void }) {
  const uiTheme = useAppStore((s) => s.uiTheme);
  const openDocument = useAppStore((s) => s.openDocument);
  const saveDocument = useAppStore((s) => s.saveDocument);
  const openProposal = useAppStore((s) => s.openProposal);
  const openExport = useAppStore((s) => s.openExport);
  const openNewDialog = useAppStore((s) => s.openNewDialog);

  const handleOpen = async () => {
    const path = await open({ filters: ORI3_FILTERS, multiple: false });
    if (typeof path === "string") {
      await openDocument(path);
    }
  };

  const handleSave = async () => {
    const path = await save({ filters: ORI3_FILTERS });
    if (path !== null) {
      await saveDocument(path);
    }
  };

  return (
    <header className="toolbar">
      <span className="toolbar-brand">
        <ToolbarBrandMark theme={uiTheme} />
        <span className="toolbar-brand-copy">
          <strong>
            ORIGAMI<span>3</span>
          </strong>
          <small>おりがみ工房</small>
        </span>
      </span>
      {/* 紙の形と大きさを決めてから作る(PAP-001)。開くのは独立ダイアログ */}
      <button
        type="button"
        data-tooltip="紙の形と大きさを決めて、新しい作品を始めます"
        onClick={openNewDialog}
      >
        <ToolbarIcon name="new" />
        新規
      </button>
      <button
        type="button"
        data-tooltip="保存した作品(.ori3)を開きます"
        onClick={() => void handleOpen()}
      >
        <ToolbarIcon name="open" />
        開く
      </button>
      <button
        type="button"
        data-tooltip="作品を.ori3ファイルへ保存します"
        onClick={() => void handleSave()}
      >
        <ToolbarIcon name="save" />
        保存
      </button>
      <span className="toolbar-separator" />
      {/* 折り角度の変更と作品データの変更は別々の履歴なので、次の1回が
          どちらに効くかを説明に出す(設計原則3b) */}
      <HistoryButtons />
      <span className="toolbar-separator" />
      {/* 提案ウィザードの入口。開くのは独立ダイアログで、常設区画は増やさない(PRO-004) */}
      <button
        type="button"
        data-tooltip="作りたい形から折り方の候補を探します"
        onClick={openProposal}
      >
        <ToolbarIcon name="proposal" />
        提案
      </button>
      {/* 書き出しの入口。開くのは独立ダイアログで、常設区画は増やさない(EXP-001/002) */}
      <ExportButton onClick={openExport} />
      <span className="toolbar-separator" />
      <button
        type="button"
        className="toolbar-help"
        data-tooltip="使い方を目次や検索から調べます(F1)"
        aria-label="ヘルプセンターを開く"
        onClick={onOpenHelp}
      >
        <ToolbarIcon name="help" />
        ヘルプ
      </button>
    </header>
  );
}
