// 完成形で決めた場所と、紙の上で決めた場所を、葉ごとに1件の提案要求へまとめる。
// 画面表示と送信要求は必ずこの同じ判定を使い、「表示では紙、要求では完成形」のような
// 食い違いを作らない。

import { clampTipPos, leafNodes } from "./skeleton";
import {
  paperBounds,
  paperPositionsFromCandidate,
} from "./paperPosition";
import type {
  Paper,
  PaperPosition2d,
  PaperTipPosition,
  ProposalCandidate,
  Skeleton,
  TipPos2d,
} from "./types";

/**
 * 2026-08-21実測: 1000×700で使う560px幅の正方形編集面は、紙の外側6%ずつを
 * 含むviewBox幅1.12なので、横へ1px動かした入力差は
 * `2 * 1.12 / 560 = 0.004`（紙の中心・長辺半分=1の座標）だった。
 */
export const PAPER_POSITION_ONE_PIXEL_MEASURED = 0.004;

/**
 * 1px未満の丸めを「場所が違う」と見せず、1pxの意図した操作は必ず区別する境目。
 * 上の実測値そのものを境目にせず、リポジトリの実測作法に合わせて80%の0.0032とした。
 */
export const PAPER_POSITION_MATCH_TOLERANCE = 0.0032;

export type ProposalPositionSource = "completion" | "paper";

/** 葉ごとに、利用者が最後に動かした画面を覚える。 */
export interface ProposalPositionLastMoved {
  leaf_id: number;
  source: ProposalPositionSource;
}

export type ProposalLeafPositionKind =
  | "automatic"
  | "completion-only"
  | "paper-only"
  | "both"
  | "different";

/** 画面の目印と送信要求の双方が使う、葉1本の判定結果。 */
export interface ProposalLeafPositionState {
  leaf_id: number;
  kind: ProposalLeafPositionKind;
  used: ProposalPositionSource | null;
  completion: TipPos2d | null;
  paper: PaperPosition2d | null;
  automaticPaper: PaperPosition2d | null;
  difference: number | null;
}

function normalizedPaperSize(paper: Paper): { width: number; height: number } | null {
  const long = Math.max(paper.width_mm, paper.height_mm);
  if (!(long > 0) || !Number.isFinite(long)) return null;
  const width = paper.width_mm / long;
  const height = paper.height_mm / long;
  if (!(width > 0 && height > 0) || !Number.isFinite(width + height)) return null;
  return { width, height };
}

/**
 * 作業10と同じ写し方で、完成形の相対位置から自動で決まる紙上位置を求める。
 * 紙座標はPaperPosition2d（紙中心、長辺半分=1）へ直して返す。
 */
export function completionPositionsOnPaper(
  skeleton: Skeleton,
  paper: Paper,
): PaperTipPosition[] {
  const size = normalizedPaperSize(paper);
  if (!size) return [];
  const given = leafNodes(skeleton).flatMap((node) =>
    node.tip_pos_2d == null
      ? []
      : [{ leaf_id: node.id, position: clampTipPos(node.tip_pos_2d) }],
  );
  if (given.length === 0) return [];

  let x0 = 0;
  let x1 = 0;
  let y0 = 0;
  let y1 = 0;
  for (const entry of given) {
    x0 = Math.min(x0, entry.position.x);
    x1 = Math.max(x1, entry.position.x);
    y0 = Math.min(y0, entry.position.y);
    y1 = Math.max(y1, entry.position.y);
  }
  const extentX = x1 - x0;
  const extentY = y1 - y0;
  let scale: number;
  if (extentX > 0 && extentY > 0) {
    scale = Math.min(size.width / extentX, size.height / extentY);
  } else if (extentX > 0) {
    scale = size.width / extentX;
  } else if (extentY > 0) {
    scale = size.height / extentY;
  } else {
    return [];
  }
  if (!(scale > 0) || !Number.isFinite(scale)) return [];

  const bodyX = Math.min(
    size.width,
    Math.max(0, size.width * 0.5 - scale * (x0 + x1) * 0.5),
  );
  const bodyY = Math.min(
    size.height,
    Math.max(0, size.height * 0.5 - scale * (y0 + y1) * 0.5),
  );
  return given.map((entry) => {
    const targetX = Math.min(
      size.width,
      Math.max(0, bodyX + scale * entry.position.x),
    );
    const targetY = Math.min(
      size.height,
      Math.max(0, bodyY + scale * entry.position.y),
    );
    return {
      leaf_id: entry.leaf_id,
      position: {
        x: 2 * (targetX - size.width * 0.5),
        y: 2 * (targetY - size.height * 0.5),
      },
    };
  });
}

export function paperPositionDifference(
  first: PaperPosition2d,
  second: PaperPosition2d,
): number {
  return Math.hypot(first.x - second.x, first.y - second.y);
}

/** 葉ごとに4状態と、実際に送る側を決める。 */
export function proposalLeafPositionStates(
  skeleton: Skeleton,
  paperPositions: readonly PaperTipPosition[],
  lastMoved: readonly ProposalPositionLastMoved[],
  paper: Paper,
): ProposalLeafPositionState[] {
  const paperByLeaf = new Map(
    paperPositions.map((entry) => [entry.leaf_id, entry.position]),
  );
  const automaticByLeaf = new Map(
    completionPositionsOnPaper(skeleton, paper).map((entry) => [
      entry.leaf_id,
      entry.position,
    ]),
  );
  const lastByLeaf = new Map(
    lastMoved.map((entry) => [entry.leaf_id, entry.source]),
  );

  return leafNodes(skeleton).map((node) => {
    const completion = node.tip_pos_2d == null ? null : clampTipPos(node.tip_pos_2d);
    const paperPosition = paperByLeaf.get(node.id) ?? null;
    const automaticPaper = automaticByLeaf.get(node.id) ?? null;
    if (completion === null && paperPosition === null) {
      return {
        leaf_id: node.id,
        kind: "automatic",
        used: null,
        completion,
        paper: paperPosition,
        automaticPaper,
        difference: null,
      };
    }
    if (completion !== null && paperPosition === null) {
      return {
        leaf_id: node.id,
        kind: "completion-only",
        used: "completion",
        completion,
        paper: paperPosition,
        automaticPaper,
        difference: null,
      };
    }
    if (completion === null && paperPosition !== null) {
      return {
        leaf_id: node.id,
        kind: "paper-only",
        used: "paper",
        completion,
        paper: paperPosition,
        automaticPaper,
        difference: null,
      };
    }

    const difference =
      automaticPaper === null || paperPosition === null
        ? Number.POSITIVE_INFINITY
        : paperPositionDifference(automaticPaper, paperPosition);
    const same = difference <= PAPER_POSITION_MATCH_TOLERANCE;
    return {
      leaf_id: node.id,
      kind: same ? "both" : "different",
      // 同じなら計算へ直接渡せる紙位置。違うなら、その葉で最後に動かした側。
      // 旧い一時状態に記録が無い場合だけ、紙位置を後から足した従来経路に合わせる。
      used: same ? "paper" : (lastByLeaf.get(node.id) ?? "paper"),
      completion,
      paper: paperPosition,
      automaticPaper,
      difference,
    };
  });
}

/** 4状態を葉ごとにまとめ、IPCへ渡すSkeletonを1件だけ作る。 */
export function proposalRequestSkeleton(
  skeleton: Skeleton,
  paperPositions: readonly PaperTipPosition[],
  lastMoved: readonly ProposalPositionLastMoved[],
  paper: Paper,
): Skeleton {
  const stateByLeaf = new Map(
    proposalLeafPositionStates(skeleton, paperPositions, lastMoved, paper).map(
      (state) => [state.leaf_id, state],
    ),
  );
  return {
    nodes: skeleton.nodes.map((node) => {
      const state = stateByLeaf.get(node.id);
      if (!state) return node;
      const copy = { ...node };
      if (state.used === "paper" && state.paper !== null) {
        copy.tip_pos_2d = { ...state.paper };
      } else if (state.used === "completion" && state.completion !== null) {
        copy.tip_pos_2d = { ...state.completion };
      } else {
        delete copy.tip_pos_2d;
      }
      return copy;
    }),
  };
}

/** 候補由来の表示位置へ、利用者が決めた紙位置だけを重ねる。 */
export function paperEditorPositions(
  candidate: ProposalCandidate,
  skeleton: Skeleton,
  specified: readonly PaperTipPosition[],
): PaperTipPosition[] {
  const leafIds = new Set(leafNodes(skeleton).map((node) => node.id));
  const specifiedByLeaf = new Map(
    specified.map((entry) => [entry.leaf_id, entry.position]),
  );
  return paperPositionsFromCandidate(candidate)
    .filter((entry) => leafIds.has(entry.leaf_id))
    .map((entry) => ({
      leaf_id: entry.leaf_id,
      position: specifiedByLeaf.get(entry.leaf_id) ?? entry.position,
    }))
    .slice(0, 12);
}

/** 紙位置1件を葉ID順で差し替える。 */
export function setSpecifiedPaperPosition(
  positions: readonly PaperTipPosition[],
  leafId: number,
  position: PaperPosition2d,
): PaperTipPosition[] {
  const next = positions.filter((entry) => entry.leaf_id !== leafId);
  next.push({ leaf_id: leafId, position: { ...position } });
  return next.sort((a, b) => a.leaf_id - b.leaf_id);
}

/** 最後に動かした側を葉ID順で差し替える。 */
export function setLastMovedSource(
  entries: readonly ProposalPositionLastMoved[],
  leafId: number,
  source: ProposalPositionSource,
): ProposalPositionLastMoved[] {
  const next = entries.filter((entry) => entry.leaf_id !== leafId);
  next.push({ leaf_id: leafId, source });
  return next.sort((a, b) => a.leaf_id - b.leaf_id);
}

/** 画面用の紙範囲。許容差の1px実測テストでも本番と同じ紙寸法を使う。 */
export function candidatePaperBounds(candidate: ProposalCandidate) {
  return paperBounds(candidate.cp);
}
