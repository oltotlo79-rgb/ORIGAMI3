// 4区画レイアウト: 上部ツールバー / 左ツールレール / 中央(2D+3D) / 下部コンテキストパネル。
// 手順タイムラインは3D区画の内側を上下に分けて置く(常設区画は増やさない)。
// このファイルはレイアウト構成のみ(200行以内を維持)。
import { lazy, Suspense, useEffect, useRef } from "react";
import { useAppStore } from "./store/appStore";
import { AppToolbar } from "./components/AppToolbar";
import { ToolRail } from "./components/ToolRail";
import { ContextPanel } from "./components/ContextPanel";
import { CpEditor } from "./components/CpEditor/CpEditor";
import { Viewer3D } from "./components/Viewer3D/Viewer3D";
import { ViewerStatusOverlays } from "./components/ViewerStatusOverlays";
import { Timeline } from "./components/Timeline";
import { RecoveryDialog } from "./components/RecoveryDialog";
import { PaneSplitter } from "./components/PaneSplitter";
import { ContextPanelSplitter } from "./components/ContextPanelSplitter";
import { NewDocumentDialog } from "./components/dialogs/NewDocumentDialog";
import { HistoryShortcuts } from "./components/HistoryShortcuts";
import { FirstRunGuide } from "./components/FirstRunGuide";
import { ThemeRoot } from "./components/ThemeRoot";
import { TooltipHost } from "./components/Tooltip";
import { installCaptureApi } from "./captureApi";
import "./App.css";

export { ExportButton } from "./components/AppToolbar";
export { relaxationStatus } from "./components/ViewerStatusOverlays";

const DEFAULT_PAPER = { width_mm: 150, height_mm: 150 };
const LazyExportDialog = lazy(() => import("./components/dialogs/ExportDialog"));
const LazyProposalWizard = lazy(async () => ({
  default: (await import("./components/dialogs/ProposalWizard")).ProposalWizard,
}));
const LazyHelpCenter = lazy(async () => ({
  default: (await import("./components/dialogs/HelpCenter")).HelpCenter,
}));

function App() {
  const newDocument = useAppStore((s) => s.newDocument);
  const checkRecovery = useAppStore((s) => s.checkRecovery);
  const proposalOpen = useAppStore((s) => s.proposalStep !== null);
  const exportOpen = useAppStore((s) => s.exportOpen);
  const openHelp = useAppStore((s) => s.openHelp);
  const helpOpen = useAppStore((s) => s.helpOpen);
  // 中央の2区画の広さの割合(UI-004)。境目のドラッグで変わる
  const splitRatio = useAppStore((s) => s.splitRatio);
  // 「全体表示」は2D・3D両方を紙全体が収まる表示に戻す(ボタンは増やさない)
  const fit2dRef = useRef<(() => void) | null>(null);
  const fit3dRef = useRef<(() => void) | null>(null);

  // 起動時に150×150mmの新規作品を開き、続けて前回の異常終了の有無を調べる
  // (残っていれば復旧ダイアログが出る。SYS-003)
  useEffect(() => {
    void newDocument(DEFAULT_PAPER).then(() => checkRecovery());
  }, [newDocument, checkRecovery]);

  // F1は起動直後から使える入口として残す。本文・図を含むヘルプ本体は開く時だけ読む。
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "F1") return;
      event.preventDefault();
      openHelp();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [openHelp]);

  // 解説画像の自動撮影口。DOM上のボタンは増やさず、WebView2/CDPからだけ使う。
  useEffect(
    () => installCaptureApi({ fit2d: fit2dRef, fit3d: fit3dRef }),
    [fit2dRef, fit3dRef],
  );

  return (
    <ThemeRoot>
      <HistoryShortcuts />
      <TooltipHost />
      <AppToolbar onOpenHelp={openHelp} />
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
            <Viewer3D
              fitRef={fit3dRef}
              statusOverlays={<ViewerStatusOverlays />}
            />
          </div>
          <Timeline />
        </section>
      </div>
      <ContextPanelSplitter />
      <ContextPanel />
      <RecoveryDialog />
      <NewDocumentDialog />
      {proposalOpen && (
        <Suspense
          fallback={
            <div
              role="status"
              style={{
                position: "fixed",
                inset: "auto 1rem 1rem auto",
              }}
            >
              提案の準備をしています…
            </div>
          }
        >
          <LazyProposalWizard />
        </Suspense>
      )}
      {exportOpen && (
        <Suspense
          fallback={
            <div
              role="status"
              style={{
                position: "fixed",
                inset: "auto 1rem 1rem auto",
              }}
            >
              書き出しの準備をしています…
            </div>
          }
        >
          <LazyExportDialog />
        </Suspense>
      )}
      {helpOpen && (
        <Suspense
          fallback={
            <div
              role="status"
              style={{
                position: "fixed",
                inset: "auto 1rem 1rem auto",
              }}
            >
              ヘルプを開く準備をしています…
            </div>
          }
        >
          <LazyHelpCenter />
        </Suspense>
      )}
      <FirstRunGuide />
    </ThemeRoot>
  );
}

export default App;
