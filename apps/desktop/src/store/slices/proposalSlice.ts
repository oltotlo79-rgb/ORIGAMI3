import type {
  PaperPosition2d,
  PaperTipPosition,
  ProposalCandidate,
  ProposalJobId,
  ProposalProgressSnapshot,
  Skeleton,
  TipPos2d,
} from "../../lib/types";
import type { ProposalPositionLastMoved } from "../../lib/proposalPosition";
import type { DocumentSlice } from "./documentSlice";

/** 提案ウィザードの画面。 */
export type ProposalStep =
  | "skeleton"
  | "candidates"
  | "paper-position"
  | "confirm";

/** 提案画面の場所操作を1回戻すための、作品へ保存しない一時状態。 */
export interface ProposalPositionSnapshot {
  step: ProposalStep;
  skeleton: Skeleton;
  candidates: ProposalCandidate[];
  selected: number | null;
  paperSource: number | null;
  paperPositions: PaperTipPosition[];
  paperSpecified: PaperTipPosition[];
  lastMoved: ProposalPositionLastMoved[];
}

/** 提案が所有する状態。同じ1本のZustand storeへ合成する。 */
export interface ProposalSliceState {
  proposalStep: ProposalStep | null;
  proposalSkeleton: Skeleton;
  proposalCandidates: ProposalCandidate[];
  proposalSelected: number | null;
  proposalPaperSource: number | null;
  proposalPaperPositions: PaperTipPosition[];
  proposalPaperSpecified: PaperTipPosition[];
  proposalPositionLastMoved: ProposalPositionLastMoved[];
  proposalPositionUndoStack: ProposalPositionSnapshot[];
  proposalPositionRedoStack: ProposalPositionSnapshot[];
  proposalBusy: boolean;
  proposalJobId: ProposalJobId | null;
  proposalProgress: ProposalProgressSnapshot | null;
  proposalProgressWarning: string | null;
  proposalError: string | null;
  proposalSeed: number;
}

/** 提案が所有する公開action。 */
export interface ProposalSliceActions {
  openProposal: () => void;
  closeProposal: () => void;
  setProposalStep: (step: ProposalStep) => void;
  setProposalSkeleton: (skeleton: Skeleton) => void;
  setProposalTipPosition: (
    leafId: number,
    position: TipPos2d | null,
  ) => void;
  generateProposal: () => Promise<void>;
  selectProposalCandidate: (index: number) => void;
  openProposalPaperPositionEditor: () => void;
  setProposalPaperPosition: (
    leafId: number,
    position: PaperPosition2d,
  ) => void;
  resetProposalPaperPositions: () => void;
  restoreOtherProposalPosition: (leafId: number) => void;
  undoProposalPosition: () => void;
  redoProposalPosition: () => void;
  generateProposalFromPaperPositions: () => Promise<void>;
  applyProposalCandidate: () => Promise<void>;
}

export type ProposalSlice = ProposalSliceState & ProposalSliceActions;

/** B1とB3を同じstoreへ再合成するときの構造契約。 */
export type ProposalSliceHostState = DocumentSlice & ProposalSlice;
