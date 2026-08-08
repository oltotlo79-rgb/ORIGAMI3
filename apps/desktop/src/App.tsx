// 4区画レイアウト: 上部ツールバー / 左ツールレール / 中央(2D+3D) / 下部コンテキストパネル。
// 手順タイムラインは3D区画の内側を上下に分けて置く(常設区画は増やさない)。
// このファイルはレイアウト構成のみ(200行以内を維持)。
import { useEffect, useRef } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { useAppStore } from "./store/appStore";
import { ToolRail } from "./components/ToolRail";
import { ContextPanel } from "./components/ContextPanel";
import { CpEditor } from "./components/CpEditor/CpEditor";
import { Viewer3D } from "./components/Viewer3D/Viewer3D";
import { Timeline } from "./components/Timeline";
import { RecoveryDialog } from "./components/RecoveryDialog";
import { PaneSplitter } from "./components/PaneSplitter";
import { NewDocumentDialog } from "./components/dialogs/NewDocumentDialog";
import { ProposalWizard } from "./components/dialogs/ProposalWizard";
import { ExportDialog } from "./components/dialogs/ExportDialog";
import { HistoryButtons } from "./components/HistoryButtons";
import { HistoryShortcuts } from "./components/HistoryShortcuts";
import { ToolbarIcon } from "./components/ToolIcons";
import { ToolbarBrandMark } from "./components/ToolbarBrandMark";
import { FirstRunGuide } from "./components/FirstRunGuide";
import { HelpCenter } from "./components/dialogs/HelpCenter";
import { ThemeRoot } from "./components/ThemeRoot";
import { uniqueWarnings } from "./lib/techniques";
import "./App.css";

const DEFAULT_PAPER = { width_mm: 150, height_mm: 150 };
const ORI3_FILTERS = [{ name: "ORIGAMI3作品", extensions: ["ori3"] }];

function App() {
  const uiTheme = useAppStore((s) => s.uiTheme);
  const newDocument = useAppStore((s) => s.newDocument);
  const openDocument = useAppStore((s) => s.openDocument);
  const saveDocument = useAppStore((s) => s.saveDocument);
  const checkRecovery = useAppStore((s) => s.checkRecovery);
  const openProposal = useAppStore((s) => s.openProposal);
  const openExport = useAppStore((s) => s.openExport);
  const openNewDialog = useAppStore((s) => s.openNewDialog);
  const openHelp = useAppStore((s) => s.openHelp);
  // 中央の2区画の広さの割合(UI-004)。境目のドラッグで変わる
  const splitRatio = useAppStore((s) => s.splitRatio);
  const warningCount = useAppStore(
    (s) => uniqueWarnings(s.warnings, s.poseWarnings, s.replayWarnings).length,
  );
  const poseConverged = useAppStore((s) => s.poseConverged);
  const hasError = useAppStore((s) => s.errorMessage !== null);
  // 「全体表示」は2D・3D両方を紙全体が収まる表示に戻す(ボタンは増やさない)
  const fit2dRef = useRef<(() => void) | null>(null);
  const fit3dRef = useRef<(() => void) | null>(null);

  // 起動時に150×150mmの新規作品を開き、続けて前回の異常終了の有無を調べる
  // (残っていれば復旧ダイアログが出る。SYS-003)
  useEffect(() => {
    void newDocument(DEFAULT_PAPER).then(() => checkRecovery());
  }, [newDocument, checkRecovery]);

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
    <ThemeRoot>
      <HistoryShortcuts />
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
        <button type="button" onClick={openNewDialog}>
          <ToolbarIcon name="new" />
          新規
        </button>
        <button type="button" onClick={() => void handleOpen()}>
          <ToolbarIcon name="open" />
          開く
        </button>
        <button type="button" onClick={() => void handleSave()}>
          <ToolbarIcon name="save" />
          保存
        </button>
        <span className="toolbar-separator" />
        {/* 折り角度の変更と作品データの変更は別々の履歴なので、次の1回が
            どちらに効くかを説明に出す(設計原則3b) */}
        <HistoryButtons />
        <span className="toolbar-separator" />
        {/* 提案ウィザードの入口。開くのは独立ダイアログで、常設区画は増やさない(PRO-004) */}
        <button type="button" onClick={openProposal}>
          <ToolbarIcon name="proposal" />
          提案
        </button>
        {/* 書き出しの入口。開くのは独立ダイアログで、常設区画は増やさない(EXP-001/002) */}
        <button type="button" onClick={openExport}>
          <ToolbarIcon name="export" />
          書き出し
        </button>
        <span className="toolbar-separator" />
        <button
          type="button"
          className="toolbar-help"
          title="目次と検索のあるヘルプセンターを開きます(F1)"
          aria-label="ヘルプセンターを開く"
          onClick={openHelp}
        >
          <ToolbarIcon name="help" />
          ヘルプ
        </button>
      </header>
      <div
        className="main-row"
        style={{
          gridTemplateColumns: `64px ${splitRatio}fr 6px ${1 - splitRatio}fr`,
        }}
      >
        <ToolRail
          onFitView={() => {
            fit2dRef.current?.();
            fit3dRef.current?.();
          }}
        />
        <section className="pane pane-2d">
          <CpEditor fitRef={fit2dRef} />
        </section>
        <PaneSplitter />
        <section className="pane pane-3d">
          <div className="pane-3d-view">
            <Viewer3D fitRef={fit3dRef} />
            {(hasError || !poseConverged || warningCount > 0) && (
              <div
                className={hasError ? "status-badge error" : "status-badge"}
                title="詳細は下のパネルに表示されます"
              >
                <svg
                  className="status-icon"
                  viewBox="0 0 24 24"
                  aria-hidden="true"
                  focusable="false"
                >
                  <path d="M12 3 22 20H2L12 3Z" fill="none" stroke="currentColor" strokeWidth="2.4" />
                  <path d="M12 8v6" stroke="currentColor" strokeLinecap="round" strokeWidth="2.4" />
                  <circle cx="12" cy="17.5" r="1.2" fill="currentColor" />
                </svg>
                <span>
                  {hasError
                    ? "エラー"
                    : poseConverged
                      ? `警告 ${warningCount}`
                      : "⚠ 追従計算が収束していません"}
                </span>
              </div>
            )}
          </div>
          <Timeline />
        </section>
      </div>
      <ContextPanel />
      <RecoveryDialog />
      <NewDocumentDialog />
      <ProposalWizard />
      <ExportDialog />
      <HelpCenter />
      <FirstRunGuide />
    </ThemeRoot>
  );
}

export default App;
