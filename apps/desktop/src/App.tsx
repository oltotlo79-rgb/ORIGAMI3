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
import { ProposalWizard } from "./components/dialogs/ProposalWizard";
import { uniqueWarnings } from "./lib/techniques";
import "./App.css";

const DEFAULT_PAPER = { width_mm: 150, height_mm: 150 };
const ORI3_FILTERS = [{ name: "ORIGAMI3作品", extensions: ["ori3"] }];

function App() {
  const newDocument = useAppStore((s) => s.newDocument);
  const openDocument = useAppStore((s) => s.openDocument);
  const saveDocument = useAppStore((s) => s.saveDocument);
  const checkRecovery = useAppStore((s) => s.checkRecovery);
  const undo = useAppStore((s) => s.undo);
  const redo = useAppStore((s) => s.redo);
  const openProposal = useAppStore((s) => s.openProposal);
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
    <div className="app">
      <header className="toolbar">
        <button type="button" onClick={() => void newDocument(DEFAULT_PAPER)}>
          新規
        </button>
        <button type="button" onClick={() => void handleOpen()}>
          開く
        </button>
        <button type="button" onClick={() => void handleSave()}>
          保存
        </button>
        <span className="toolbar-separator" />
        <button type="button" onClick={() => void undo()}>
          元に戻す
        </button>
        <button type="button" onClick={() => void redo()}>
          やり直し
        </button>
        <span className="toolbar-separator" />
        {/* 提案ウィザードの入口。開くのは独立ダイアログで、常設区画は増やさない(PRO-004) */}
        <button type="button" onClick={openProposal}>
          提案
        </button>
      </header>
      <div className="main-row">
        <ToolRail
          onFitView={() => {
            fit2dRef.current?.();
            fit3dRef.current?.();
          }}
        />
        <section className="pane pane-2d">
          <CpEditor fitRef={fit2dRef} />
        </section>
        <section className="pane pane-3d">
          <div className="pane-3d-view">
            <Viewer3D fitRef={fit3dRef} />
            {(hasError || !poseConverged || warningCount > 0) && (
              <div
                className={hasError ? "status-badge error" : "status-badge"}
                title="詳細は下のパネルに表示されます"
              >
                {hasError
                  ? "エラー"
                  : poseConverged
                    ? `警告 ${warningCount}`
                    : "⚠ 追従計算が収束していません"}
              </div>
            )}
          </div>
          <Timeline />
        </section>
      </div>
      <ContextPanel />
      <RecoveryDialog />
      <ProposalWizard />
    </div>
  );
}

export default App;
