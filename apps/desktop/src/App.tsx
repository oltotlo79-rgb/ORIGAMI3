// 4区画レイアウト: 上部ツールバー / 左ツールレール / 中央(2D+3D) / 下部コンテキストパネル。
// 手順タイムラインは3D区画の内側を上下に分けて置く(常設区画は増やさない)。
// このファイルはレイアウト構成のみ(200行以内を維持)。
import { useEffect, useRef } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { relaxationNotices, useAppStore } from "./store/appStore";
import { ToolRail } from "./components/ToolRail";
import { ContextPanel } from "./components/ContextPanel";
import { CpEditor } from "./components/CpEditor/CpEditor";
import { Viewer3D } from "./components/Viewer3D/Viewer3D";
import { Timeline } from "./components/Timeline";
import { RecoveryDialog } from "./components/RecoveryDialog";
import { PaneSplitter } from "./components/PaneSplitter";
import { ContextPanelSplitter } from "./components/ContextPanelSplitter";
import { NewDocumentDialog } from "./components/dialogs/NewDocumentDialog";
import { ProposalWizard } from "./components/dialogs/ProposalWizard";
import { ExportDialog, EXPORT_CHOICES } from "./components/dialogs/ExportDialog";
import { HistoryButtons } from "./components/HistoryButtons";
import { HistoryShortcuts } from "./components/HistoryShortcuts";
import { ToolbarIcon } from "./components/ToolIcons";
import { ToolbarBrandMark } from "./components/ToolbarBrandMark";
import { FirstRunGuide } from "./components/FirstRunGuide";
import { HelpCenter } from "./components/dialogs/HelpCenter";
import { ThemeRoot } from "./components/ThemeRoot";
import { TooltipHost } from "./components/Tooltip";
import {
  flatFoldViolationIds,
  statusBadgeText,
  warningCount as countWarnings,
} from "./lib/flatFoldNotice";
import { installCaptureApi } from "./captureApi";
import type { AngleRelaxation } from "./lib/types";
import "./App.css";

const DEFAULT_PAPER = { width_mm: 150, height_mm: 150 };
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

/** 3D右上へ出す自然追従の短い知らせ。nullなら通常の警告表示へ譲る。 */
export function relaxationStatus(
  relaxations: readonly AngleRelaxation[],
  bestEffort: boolean,
): string | null {
  if (bestEffort) return "指定を優先し、いちばん近い形で追従中";
  const notices = relaxationNotices(relaxations);
  if (notices.length === 0) return null;
  const maxDelta = Math.max(...notices.map((item) => Math.abs(item.delta_deg)));
  return `前の折り目${notices.length}本が追従（最大${maxDelta.toFixed(1)}°）`;
}

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
    (s) =>
      countWarnings(
        s.warnings,
        s.poseWarnings,
        s.replayWarnings,
        s.flatFoldViolations,
      ),
  );
  const flatFoldViolationCount = useAppStore(
    (s) => flatFoldViolationIds(s.flatFoldViolations).length,
  );
  const poseConverged = useAppStore((s) => s.poseConverged);
  const relaxations = useAppStore((s) => s.relaxations);
  const poseBestEffort = useAppStore((s) => s.poseBestEffort);
  const hasError = useAppStore((s) => s.errorMessage !== null);
  const suspectHinges = useAppStore((s) => s.suspectHinges);
  const setSelection = useAppStore((s) => s.setSelection);
  // 「全体表示」は2D・3D両方を紙全体が収まる表示に戻す(ボタンは増やさない)
  const fit2dRef = useRef<(() => void) | null>(null);
  const fit3dRef = useRef<(() => void) | null>(null);
  const followStatus = relaxationStatus(relaxations, poseBestEffort);
  const badgeText = statusBadgeText({
    hasError,
    followStatus,
    poseConverged,
    warningCount,
    flatFoldViolationCount,
  });
  const showFollowStatus =
    !hasError && flatFoldViolationCount === 0 && followStatus !== null;

  // 起動時に150×150mmの新規作品を開き、続けて前回の異常終了の有無を調べる
  // (残っていれば復旧ダイアログが出る。SYS-003)
  useEffect(() => {
    void newDocument(DEFAULT_PAPER).then(() => checkRecovery());
  }, [newDocument, checkRecovery]);

  // 解説画像の自動撮影口。DOM上のボタンは増やさず、WebView2/CDPからだけ使う。
  useEffect(
    () => installCaptureApi({ fit2d: fit2dRef, fit3d: fit3dRef }),
    [fit2dRef, fit3dRef],
  );

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
      <TooltipHost />
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
            {badgeText !== null && (
              <div
                className={
                  hasError
                    ? "status-badge error"
                    : showFollowStatus
                      ? "status-badge follow"
                      : "status-badge"
                }
                data-floating-ui="status-badge"
                data-tooltip="詳しい通知を下のパネルで確認できます"
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
                <span>{badgeText}</span>
              </div>
            )}
            {suspectHinges.length > 0 && (
              <button
                type="button"
                className="suspect-hinge-guide"
                data-floating-ui="suspect-hinge-guide"
                data-tooltip="原因候補の折り目を選び、角度を確認します"
                onClick={() =>
                  setSelection({ edgeIds: [suspectHinges[0]], vertexIds: [] })
                }
              >
                赤く光る折り目の角度を見直してください
              </button>
            )}
          </div>
          <Timeline />
        </section>
      </div>
      <ContextPanelSplitter />
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
