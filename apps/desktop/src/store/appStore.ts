// アプリ全体の状態を1本で管理するZustandストア(要件§2: フロント状態はストア1本)。
// IPC呼び出しはactionの中で行い、成功したらDocumentViewの内容を一括反映、
// 失敗(reject)はerrorMessageへ入れる(「止めずに警告」原則)。
// 全IPC要求は直列化キュー(ipcQueue.ts)を通し、連続操作でも適用順を発行順に
// 固定する。最新でない応答は破棄する(古いdocによる上書きを防ぐ)。

import { create } from "zustand";
export type { ToolId } from "./toolTypes";
import { createCommandService } from "./services/commandService";
export { MIRROR_AXIS_REMOVED_NOTICE } from "./services/commandService";
import { createDialogSettingsSlice } from "./services/dialogSettingsActions";
import { createDocumentSlice } from "./services/documentActions";
import {
  createPoseReplaySlice,
  type CreatedPoseReplaySlice,
} from "./services/poseReplayActions";
import { createProposalSlice } from "./services/proposalActions";
export { resetPoseThrottle } from "./services/poseRuntime";
export { resetFoldAllPreviewRuntime } from "./services/foldAllRuntime";
import type { DocumentSlice } from "./slices/documentSlice";
import type { DialogSettingsSlice } from "./slices/dialogSettingsSlice";
import {
  relaxationNotices,
  type PoseReplaySlice,
} from "./slices/poseReplaySlice";
import type { ProposalSlice } from "./slices/proposalSlice";
export type {
  ExportSettings,
  GuideAction,
  GuideStep,
  NewPaperDraft,
} from "./slices/dialogSettingsSlice";
export {
  DEFAULT_NEW_PAPER,
  DEFAULT_PNG_LONG_SIDE,
  draftToPaper,
} from "./slices/dialogSettingsSlice";
export type {
  AlignCpPick,
  AlignDraft,
  FoldDraft,
  FoldTarget,
  FoldTargetSelection,
  MeasureDisplay,
  MeasureDraft,
  MeasureEdgePick,
  MeasureMode,
  MeasurePick,
  MeasurePointPick,
  PendingFoldThrough,
  Selection,
  SpatialFoldDrag,
  TechniqueDraft,
} from "./slices/documentSlice";
export {
  alignFoldDraft,
  automaticMovingSide,
  canFoldNow,
  foldInsertAt,
  initialMovingSide,
  isAlignComplete,
  isSpatialFoldFrame,
  nextAlignKind,
} from "./slices/documentSlice";
export type {
  ActiveAngleIntent,
  AngleSnapshot,
  FoldAllPreviewState,
  FoldAllReturnState,
} from "./slices/poseReplaySlice";
export type {
  ProposalPositionSnapshot,
  ProposalStep,
} from "./slices/proposalSlice";
export {
  FINISH_JUMP_NOTICE,
  FINISH_JUMP_NOTICE_THRESHOLD,
  RELAX_NOTICE_EPS_DEG,
  inflateBlockReason,
  isStepSkipped,
  maximumFrameVertexMovement,
  poseRecordReason,
  pullBlockReason,
  pullBlockedOf,
  relaxationNotices,
  stepPanelSelected,
} from "./slices/poseReplaySlice";

interface AppState extends DocumentSlice, PoseReplaySlice, ProposalSlice, DialogSettingsSlice {}

export const useAppStore = create<AppState>((set, get) => {
  // command/documentの遅延callbackとB2 factoryを一度だけ相互結線する。
  // eslint-disable-next-line prefer-const
  let poseReplay!: CreatedPoseReplaySlice;
  const commandService = createCommandService<AppState>(set, get, {
    discardFoldAllPreview: () =>
      poseReplay.internals.discardFoldAllPreview(),
    stopPlayback: () => poseReplay.internals.stopPlayback(),
    resetPoseSchedule: () => poseReplay.internals.resetPoseSchedule(),
    clearAngleHistory: () => poseReplay.internals.clearAngleHistory(),
    syncSequence: (view) => poseReplay.internals.syncSequence(view),
    syncPose: () => poseReplay.internals.syncPose(),
  });
  const {
    queue,
    fail,
    runViewCommand,
    applyDocChangeResult,
    applyDocChange,
  } = commandService;
  const dialogSettingsSlice = createDialogSettingsSlice<AppState>(set, get, {
    queue,
    fail,
    runViewCommand,
    waitForFoldAllRestore: () =>
      poseReplay.internals.waitForFoldAllRestore(),
    scheduleSoftShape: () => poseReplay.internals.scheduleSoftShape(),
    queueSoftSave: (display) =>
      poseReplay.internals.queueSoftSave(display),
  });

  const proposalSlice = createProposalSlice<AppState>(set, get, {
    applyDocChange,
  });

  const documentSlice = createDocumentSlice<AppState>(set, get, {
    queue,
    runViewCommand,
    applyDocChange,
    fail,
    invalidateFoldAllEntry: () =>
      poseReplay.internals.invalidateFoldAllEntry(),
    flushSoftSave: () => poseReplay.internals.flushSoftSave(),
    waitForFoldAllRestore: () =>
      poseReplay.internals.waitForFoldAllRestore(),
    restoreAfterFoldAllPreview: (restoreInput) =>
      poseReplay.internals.restoreAfterFoldAllPreview(restoreInput),
    stopPlayback: () => poseReplay.internals.stopPlayback(),
    isStepReplayPending: () =>
      poseReplay.internals.isStepReplayPending(),
    persistPrefs: () => dialogSettingsSlice.internals.persistPrefs(),
    relaxationNotices,
    clearZeroOnlyDrivers: () =>
      poseReplay.internals.clearZeroOnlyDrivers(),
  });

  poseReplay = createPoseReplaySlice<AppState>(set, get, {
    queue,
    fail,
    runViewCommand,
    applyDocChangeResult,
    latestDocChange: commandService.latestDocChange,
    invalidateFoldThrough: () =>
      documentSlice.internals.invalidateFoldThrough(),
    undoProposalPositionState: () =>
      proposalSlice.internals.undoProposalPositionState(),
    redoProposalPositionState: () =>
      proposalSlice.internals.redoProposalPositionState(),
  });

  return {
    ...documentSlice.slice,
    ...poseReplay.slice,
    ...{ recovery: dialogSettingsSlice.slice.recovery },
    ...proposalSlice.slice,
    ...dialogSettingsSlice.slice,
  };
});
