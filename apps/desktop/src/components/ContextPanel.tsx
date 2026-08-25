// 下部コンテキストパネル。高さは上端の取っ手で変えられ、選択状態に応じて
// 内容を切り替える。
// 警告・エラーの詳細もここに表示する(常設パネルを増やさない)。

import {
  relaxationNotices,
  stepPanelSelected,
  useAppStore,
} from "../store/appStore";
import { flatFoldNotice } from "../lib/flatFoldNotice";
import { uniqueWarnings } from "../lib/techniques";
import { fileName } from "./RecoveryDialog";
import { MeasureControls } from "./MeasureControls";
import { OperationSteps } from "./OperationSteps";
import {
  FoldControls,
  RelaxationMessages,
  StepContent,
} from "./contextAngleSteps";
import {
  AlignDraftContent,
  AlignStartRow,
  FoldDraftContent,
  FoldThroughProposalContent,
} from "./contextAlignFold";
import {
  CurveRow,
  FoldAllPreviewContent,
  LINE_TOOLS,
  PullContent,
  SelectionContent,
} from "./contextPaperDisplay";
import { TechniqueDraftContent } from "./contextTechniques";

export function ContextPanel() {
  const warnings = useAppStore((s) => s.warnings);
  const poseWarnings = useAppStore((s) => s.poseWarnings);
  const replayWarnings = useAppStore((s) => s.replayWarnings);
  const flatFoldViolations = useAppStore((s) => s.flatFoldViolations);
  const errorMessage = useAppStore((s) => s.errorMessage);
  const documentSavedPath = useAppStore((s) => s.documentSavedPath);
  const mirrorAxisNotice = useAppStore((s) => s.mirrorAxisNotice);
  const foldAllPreview = useAppStore((s) => s.foldAllPreview);
  const currentStep = useAppStore((s) => s.currentStep);
  const activeTool = useAppStore((s) => s.activeTool);
  const foldDraft = useAppStore((s) => s.foldDraft);
  const pendingFoldThrough = useAppStore((s) => s.pendingFoldThrough);
  const alignDraft = useAppStore((s) => s.alignDraft);
  const techniqueDraft = useAppStore((s) => s.techniqueDraft);
  const selection = useAppStore((s) => s.selection);
  const hinges = useAppStore((s) => s.hinges);
  const relaxations = useAppStore((s) => s.relaxations);
  // 同じ文言は1回だけ出す(展開図の検査結果には自動再生の警告も合流している)
  const flatFoldWarning = flatFoldNotice(flatFoldViolations);
  const allWarnings = uniqueWarnings(
    warnings,
    poseWarnings,
    replayWarnings,
    flatFoldWarning === null ? [] : [flatFoldWarning],
  );
  const hasRelaxations = relaxationNotices(relaxations).length > 0;
  // 手順を選んでいる間はその手順の設定を出す(「折る前」「最新」は選択なし扱い)。
  // 同じ判断を3Dの紙の案内(ふくらます入口)も使うので、条件はストア側に1つだけ置く。
  const selectedStep = stepPanelSelected({ currentStep }) ? currentStep : null;
  const hasSelection =
    selection.edgeIds.length > 0 || selection.vertexIds.length > 0;
  const hasSelectedHinge = selection.edgeIds.some((id) => hinges.has(id));

  return (
    <footer className="context-panel" id="context-panel">
      <div className="context-selection">
        {/* 未処理の折り方確認を最優先し、測定中は過去の手順を見ていても測定欄を出す。
            それ以外は、選んでいる手順の設定を優先する。 */}
        {foldAllPreview !== null ? (
          <FoldAllPreviewContent />
        ) : pendingFoldThrough ? (
          <>
            <FoldThroughProposalContent pending={pendingFoldThrough} />
            <OperationSteps />
          </>
        ) : activeTool === "measure" ? (
          <>
            <MeasureControls />
            <OperationSteps />
          </>
        ) : selectedStep !== null ? (
          <>
            <StepContent number={selectedStep} />
            <OperationSteps />
          </>
        ) : techniqueDraft ? (
          <>
            <TechniqueDraftContent draft={techniqueDraft} />
            <OperationSteps />
          </>
        ) : alignDraft ? (
          <>
            <AlignDraftContent draft={alignDraft} foldDraft={foldDraft} />
            <OperationSteps />
          </>
        ) : foldDraft ? (
          <>
            <FoldDraftContent draft={foldDraft} />
            <OperationSteps />
          </>
        ) : hasSelectedHinge ? (
          <>
            <FoldControls primary />
            <OperationSteps />
            {activeTool === "pull" ? <PullContent /> : <SelectionContent />}
            {LINE_TOOLS.includes(activeTool) && <CurveRow />}
            {activeTool === "fold" && <AlignStartRow />}
          </>
        ) : (
          <>
            {!hasSelection && <OperationSteps />}
            {activeTool === "pull" ? (
              <PullContent />
            ) : (
              <>
                {!hasSelection && LINE_TOOLS.includes(activeTool) && <CurveRow />}
                {!hasSelection && activeTool === "fold" && <AlignStartRow />}
                <SelectionContent />
                {hasSelection && LINE_TOOLS.includes(activeTool) && <CurveRow />}
                {hasSelection && activeTool === "fold" && <AlignStartRow />}
              </>
            )}
            <FoldControls />
            {hasSelection && <OperationSteps />}
          </>
        )}
      </div>
      {foldAllPreview === null &&
        (errorMessage !== null ||
          documentSavedPath !== null ||
          mirrorAxisNotice !== null ||
          allWarnings.length > 0 ||
          hasRelaxations) && (
          <div className="context-messages">
            {errorMessage !== null && <p className="error-text">{errorMessage}</p>}
            {documentSavedPath !== null && errorMessage === null && (
              <p className="mirror-axis-notice" aria-live="polite">
                作品を「{fileName(documentSavedPath)}」に保存しました
              </p>
            )}
            {mirrorAxisNotice !== null && (
              <p className="mirror-axis-notice">{mirrorAxisNotice}</p>
            )}
            <RelaxationMessages />
            {allWarnings.map((w, i) => (
              <p key={i} className="warning-text">
                {w}
              </p>
            ))}
          </div>
        )}
    </footer>
  );
}
