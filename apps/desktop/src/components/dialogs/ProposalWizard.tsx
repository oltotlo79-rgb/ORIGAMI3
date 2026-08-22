// 提案ウィザード(PRO-001/003/004/005、Task 3-4)。
// 「出っぱりの数と長さ・太さを決める → 候補を選ぶ → 使う」の3画面を独立ダイアログで出す。
// 常設4区画には何も足さない(入口は上部ツールバーのボタン1つだけ)。
// 「角」「木構造」「円充填」などの専門用語は画面に出さない(設計原則3b)。

import { useAppStore } from "../../store/appStore";
import {
  LENGTH_RANGE,
  MAX_LIMBS,
  MIN_LIMBS,
  WIDTH_RANGE,
  addLimb,
  canAddLimb,
  canRemoveLimb,
  leafNodes,
  removeLimb,
  setLimb,
  skeletonPathLabels,
  skeletonRows,
} from "../../lib/skeleton";
import {
  PROPOSAL_DIALOG_MAX_WIDTH_PX,
  PROPOSAL_PAPER_DIALOG_MAX_WIDTH_PX,
  PROPOSAL_DIALOG_VIEWPORT_GUTTER_PX,
  PROPOSAL_LIST_BASIS_PX,
  PROPOSAL_PREVIEW_MAX_WIDTH_PX,
  PROPOSAL_ROW_INDENT_MAX_PX,
  PROPOSAL_ROW_INDENT_STEP_PX,
} from "../../lib/proposalLayout";
import { SkeletonPreview } from "./SkeletonPreview";
import { CpThumbnail } from "./CpThumbnail";
import { PaperPositionEditor } from "./PaperPositionEditor";
import type { ProposalFoldPlan } from "../../lib/types";
import {
  paperPositionChanged,
  paperPositionsFromCandidate,
} from "../../lib/paperPosition";
import {
  proposalLeafPositionStates,
  type ProposalLeafPositionState,
} from "../../lib/proposalPosition";

const PROPOSAL_FALLBACK_PAPER = { width_mm: 150, height_mm: 150 };

const INTERNAL_PROPOSAL_WORDS =
  /骨格|充填|ソルバー|ヤコビアン|hard|soft|warm[\s-]+start|イテレーション|内部エラー|節点|木|根|深さ|円の中心|角|ID/iu;
const INTERNAL_MESSAGE_SHAPE = /[A-Za-z_{}[\]=]|\d/iu;

type ProposalMessageKind =
  | "initial-error"
  | "retry-error"
  | "paper-error"
  | "warning";

function hasInternalDetail(message: string): boolean {
  const withoutVisibleNumbers = message
    .replace(/出っぱり\d+/gu, "")
    .replace(/\d+か所/gu, "")
    .replace(/0より/gu, "");
  return (
    INTERNAL_PROPOSAL_WORDS.test(message) ||
    INTERNAL_MESSAGE_SHAPE.test(withoutVisibleNumbers)
  );
}

/** RustやIPCの内部表現を、画面で選べる操作の案内へ置き換える。 */
export function proposalUserMessage(
  message: string,
  kind: ProposalMessageKind,
): string {
  const cleaned = message.replace(/^Error:\s*/u, "").trim();
  if (cleaned && !hasInternalDetail(cleaned)) return cleaned;
  if (kind === "warning") {
    return "この置き方では希望した形を作りにくい部分があります。「選び直す」で戻って別の候補を選ぶか、「形を直す」から出っぱりの長さや太さを調整してください。";
  }
  if (kind === "retry-error") {
    return "別の置き方を作れませんでした。「形を直す」で出っぱりの本数・長さ・太さを見直すか、もう一度「別の置き方も見る」を押してください。";
  }
  if (kind === "paper-error") {
    return "この場所では別の置き方を作れませんでした。丸い印どうしを少し離してから、もう一度「この場所で作り直す」を押してください。";
  }
  return "展開図を作れませんでした。上の出っぱりの本数・長さ・太さを見直してから、もう一度「展開図を作ってもらう」を押してください。";
}

/** 場所が違う葉だけ、いま使う側と反対側へ戻す直接操作を同じ場所に出す。 */
function ProposalPositionNotices() {
  const skeleton = useAppStore((s) => s.proposalSkeleton);
  const paperSpecified = useAppStore((s) => s.proposalPaperSpecified);
  const lastMoved = useAppStore((s) => s.proposalPositionLastMoved);
  const paper = useAppStore((s) => s.doc?.paper) ?? PROPOSAL_FALLBACK_PAPER;
  const busy = useAppStore((s) => s.proposalBusy);
  const restore = useAppStore((s) => s.restoreOtherProposalPosition);
  const labels = skeletonPathLabels(skeleton);
  const different = proposalLeafPositionStates(
    skeleton,
    paperSpecified,
    lastMoved,
    paper,
  ).filter((state) => state.kind === "different");
  if (different.length === 0) return null;

  return (
    <section
      className="proposal-position-notices"
      data-proposal-position-notices={different.length}
      aria-live="polite"
    >
      <p>
        完成形と紙の上で場所が違う先が{different.length}か所あります。
      </p>
      <ul>
        {different.map((state) => {
          const name = (labels.get(state.leaf_id) ?? [
            `出っぱり${state.leaf_id}`,
          ]).join("の");
          const completionUsed = state.used === "completion";
          const destination = completionUsed ? "紙の上" : "完成形";
          return (
            <li
              key={state.leaf_id}
              data-position-different={state.leaf_id}
              data-position-used={state.used ?? "automatic"}
            >
              <span>
                <strong>{name}</strong>：
                {completionUsed
                  ? "完成形で動かした場所を使います。"
                  : "紙の上で動かした場所を使います。"}
              </span>
              <button
                type="button"
                disabled={busy}
                aria-label={`${name}を${destination}の場所に戻す`}
                onClick={() => restore(state.leaf_id)}
              >
                {destination}の場所に戻す
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

/** 提案位置の履歴はダイアログ内からも直接戻せる。 */
function ProposalPositionHistoryButtons() {
  const undoCount = useAppStore((s) => s.proposalPositionUndoStack.length);
  const redoCount = useAppStore((s) => s.proposalPositionRedoStack.length);
  const busy = useAppStore((s) => s.proposalBusy);
  const undo = useAppStore((s) => s.undoProposalPosition);
  const redo = useAppStore((s) => s.redoProposalPosition);
  if (undoCount === 0 && redoCount === 0) return null;
  return (
    <div className="button-row proposal-position-history">
      <button
        type="button"
        disabled={busy || undoCount === 0}
        aria-label="場所の操作を元に戻す"
        onClick={undo}
      >
        元に戻す
      </button>
      <button
        type="button"
        disabled={busy || redoCount === 0}
        aria-label="場所の操作をやり直す"
        onClick={redo}
      >
        やり直す
      </button>
    </div>
  );
}

/** 折りにくさの目安を日本語にする(数字だけだと良し悪しが伝わらない) */
export function violationLabel(count: number): string {
  if (count === 0) return "きれいに畳めそうです";
  return `折りたたみにくい点 ${count} か所`;
}

/** 折り方の確認状態を、作業29で決めた利用者向けの言葉にする。 */
export function foldPlanLabel(
  plan: ProposalFoldPlan | null | undefined,
): string {
  if (plan == null) return "折り方はまだありません";
  switch (plan.status) {
    case "checked_to_finish":
      return "最後まで確認できました";
    case "partial":
      return "途中に注意があります";
  }
}

/** 適用後にタイムラインへ入る手数を、確認状態とは分けて補助表示する。 */
function foldPlanStepsLabel(
  plan: ProposalFoldPlan | null | undefined,
): string | null {
  return plan == null ? null : `${plan.steps.length}手`;
}

/** 1画面目: 出っぱりの数と長さ・太さを決める */
function SkeletonStep() {
  const skeleton = useAppStore((s) => s.proposalSkeleton);
  const setSkeleton = useAppStore((s) => s.setProposalSkeleton);
  const setTipPosition = useAppStore((s) => s.setProposalTipPosition);
  const paperSpecified = useAppStore((s) => s.proposalPaperSpecified);
  const lastMoved = useAppStore((s) => s.proposalPositionLastMoved);
  const paper = useAppStore((s) => s.doc?.paper) ?? PROPOSAL_FALLBACK_PAPER;
  const generate = useAppStore((s) => s.generateProposal);
  const busy = useAppStore((s) => s.proposalBusy);
  const error = useAppStore((s) => s.proposalError);
  const close = useAppStore((s) => s.closeProposal);
  const leaves = leafNodes(skeleton);
  const leafIds = new Set(leaves.map((node) => node.id));
  const rows = skeletonRows(skeleton);
  const pathParts = skeletonPathLabels(skeleton);
  const paperSpecifiedIds = new Set(
    paperSpecified.map((entry) => entry.leaf_id),
  );
  const positionStates: ProposalLeafPositionState[] = proposalLeafPositionStates(
    skeleton,
    paperSpecified,
    lastMoved,
    paper,
  );
  const lastChildByParent = new Map<number, number>();
  for (const row of rows) {
    if (row.node.parent !== null) {
      lastChildByParent.set(row.node.parent, row.node.id);
    }
  }

  return (
    <>
      <p>
        頭・尾・足のような「出っぱり」を{MIN_LIMBS}〜{MAX_LIMBS}
        本まで決められます。それぞれの先へ足すこともでき、長さと先端の太さを変えると下の絵がそのまま変わります。絵の中の丸い先はつまんで動かせます。動かすと、その先を出したい場所として覚えます。
      </p>
      <ProposalPositionNotices />
      <div
        className="proposal-body"
        style={{ flexWrap: "wrap", minWidth: 0, maxWidth: "100%" }}
      >
        <div
          style={{
            flex: `0 1 ${PROPOSAL_PREVIEW_MAX_WIDTH_PX}px`,
            minWidth: 0,
            maxWidth: "100%",
          }}
        >
          <SkeletonPreview
            skeleton={skeleton}
            disabled={busy}
            positionStates={positionStates}
            onTipPosChange={setTipPosition}
          />
        </div>
        <div
          className="limb-list"
          data-shape-list="nested"
          role="list"
          aria-label={`出っぱり${leaves.length}本の並び`}
          style={{
            flex: `1 1 ${PROPOSAL_LIST_BASIS_PX}px`,
            minWidth: 0,
            maxWidth: "100%",
          }}
        >
          <div
            className="limb-row"
            data-shape-body="true"
            role="listitem"
            style={{ minWidth: 0, maxWidth: "100%", boxSizing: "border-box" }}
          >
            <strong>胴</strong>
          </div>
          {rows.map(({ node, depth, label }) => {
            const parts = pathParts.get(node.id) ?? [label];
            const pathLabel = parts.join("の");
            // 48px以降は横幅を狭めず、祖先の並びを折り返して親子関係を示す。
            const visibleLabel = depth > 4 ? parts.join(" › ") : label;
            const indent = Math.min(
              Math.max(depth - 1, 0) * PROPOSAL_ROW_INDENT_STEP_PX,
              PROPOSAL_ROW_INDENT_MAX_PX,
            );
            const addAllowed = canAddLimb(skeleton, node.id);
            const connector =
              lastChildByParent.get(node.parent ?? -1) === node.id ? "└─" : "├─";
            return (
              <div
                className="limb-row"
                data-shape-row={node.id}
                data-parent-part={node.parent ?? undefined}
                data-indent-level={depth}
                role="listitem"
                key={node.id}
                style={{
                  marginInlineStart: `${indent}px`,
                  width: `calc(100% - ${indent}px)`,
                  minWidth: 0,
                  maxWidth: "100%",
                  boxSizing: "border-box",
                  flexWrap: "wrap",
                  overflowWrap: "anywhere",
                  borderInlineStart:
                    depth > 1 ? "2px solid rgba(59, 111, 201, 0.35)" : undefined,
                  paddingInlineStart: depth > 1 ? "6px" : undefined,
                }}
              >
                <span aria-hidden="true" style={{ flex: "none" }}>
                  {connector}
                </span>
                <span
                  className="limb-name"
                  style={{
                    width: "auto",
                    minWidth: 0,
                    flex: "1 1 6em",
                    overflowWrap: "anywhere",
                  }}
                >
                  {visibleLabel}
                </span>
                <div
                  style={{
                    display: "flex",
                    flex: "1 1 250px",
                    flexWrap: "wrap",
                    alignItems: "center",
                    gap: "8px",
                    minWidth: 0,
                    maxWidth: "100%",
                  }}
                >
                  <label
                    style={{
                      display: "inline-flex",
                      alignItems: "center",
                      flexWrap: "wrap",
                      flex: "1 1 112px",
                      minWidth: 0,
                      maxWidth: "100%",
                    }}
                  >
                    長さ
                    <input
                      type="range"
                      aria-label={`${pathLabel}の長さ`}
                      min={LENGTH_RANGE.min}
                      max={LENGTH_RANGE.max}
                      step={LENGTH_RANGE.step}
                      value={node.length}
                      style={{ flex: "1 1 80px", minWidth: 0, maxWidth: "100%" }}
                      onChange={(e) =>
                        setSkeleton(
                          setLimb(skeleton, node.id, {
                            length: Number(e.target.value),
                          }),
                        )
                      }
                    />
                  </label>
                  {leafIds.has(node.id) && (
                    <label
                      style={{
                        display: "inline-flex",
                        alignItems: "center",
                        flexWrap: "wrap",
                        flex: "1 1 112px",
                        minWidth: 0,
                        maxWidth: "100%",
                      }}
                    >
                      太さ
                      <input
                        type="range"
                        aria-label={`${pathLabel}の太さ`}
                        min={WIDTH_RANGE.min}
                        max={WIDTH_RANGE.max}
                        step={WIDTH_RANGE.step}
                        value={node.width_factor}
                        style={{ flex: "1 1 80px", minWidth: 0, maxWidth: "100%" }}
                        onChange={(e) =>
                          setSkeleton(
                            setLimb(skeleton, node.id, {
                              width_factor: Number(e.target.value),
                            }),
                          )
                        }
                      />
                    </label>
                  )}
                  {/* 場所を決めた先端にだけ出す。決めていないときは何も増やさない。 */}
                  {leafIds.has(node.id) && node.tip_pos_2d != null && (
                    <button
                      type="button"
                      aria-label={
                        paperSpecifiedIds.has(node.id)
                          ? `${pathLabel}の完成形の場所を取り消す`
                          : `${pathLabel}の場所を自動に戻す`
                      }
                      style={{
                        maxWidth: "100%",
                        whiteSpace: "normal",
                        overflowWrap: "anywhere",
                      }}
                      onClick={() => setTipPosition(node.id, null)}
                    >
                      {paperSpecifiedIds.has(node.id)
                        ? "完成形の場所を取り消す"
                        : "場所を自動に戻す"}
                    </button>
                  )}
                  <button
                    type="button"
                    aria-label={`${pathLabel}のこの先に足す`}
                    title={addAllowed ? undefined : "先端は12本までです"}
                    disabled={!addAllowed}
                    style={{
                      maxWidth: "100%",
                      whiteSpace: "normal",
                      overflowWrap: "anywhere",
                    }}
                    onClick={() => setSkeleton(addLimb(skeleton, node.id))}
                  >
                    ＋ この先に足す
                  </button>
                  <button
                    type="button"
                    aria-label={`${pathLabel}とその先を消す`}
                    disabled={!canRemoveLimb(skeleton, node.id)}
                    style={{ maxWidth: "100%" }}
                    onClick={() => setSkeleton(removeLimb(skeleton, node.id))}
                  >
                    ✕
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      </div>
      <div className="button-row">
        <button
          type="button"
          disabled={!canAddLimb(skeleton)}
          title={canAddLimb(skeleton) ? undefined : "先端は12本までです"}
          onClick={() => setSkeleton(addLimb(skeleton))}
        >
          出っぱりを増やす
        </button>
        <button
          type="button"
          className="button-primary"
          disabled={busy}
          onClick={() => void generate()}
        >
          {busy ? "計算中…" : "展開図を作ってもらう"}
        </button>
        <button type="button" onClick={close}>
          やめる
        </button>
      </div>
      {error && (
        <p className="error-text">
          {proposalUserMessage(error, "initial-error")}
        </p>
      )}
    </>
  );
}

/** 2画面目: 置き方の違う候補(最大4件)から1つ選ぶ(PRO-005) */
function CandidateStep() {
  const candidates = useAppStore((s) => s.proposalCandidates);
  const selected = useAppStore((s) => s.proposalSelected);
  const skeleton = useAppStore((s) => s.proposalSkeleton);
  const select = useAppStore((s) => s.selectProposalCandidate);
  const setStep = useAppStore((s) => s.setProposalStep);
  const generate = useAppStore((s) => s.generateProposal);
  const busy = useAppStore((s) => s.proposalBusy);
  const error = useAppStore((s) => s.proposalError);
  const openPaperEditor = useAppStore(
    (s) => s.openProposalPaperPositionEditor,
  );
  const selectedCandidate =
    selected === null ? null : (candidates[selected] ?? null);
  const selectableLeafIds = new Set(leafNodes(skeleton).map((node) => node.id));
  const canAdjustPaper =
    selectedCandidate !== null &&
    paperPositionsFromCandidate(selectedCandidate).some((entry) =>
      selectableLeafIds.has(entry.leaf_id),
    );

  return (
    <>
      <p>
        同じ形から{candidates.length}
        通りの置き方ができました。好きなものを選んでください。
      </p>
      <ProposalPositionNotices />
      <div className="candidate-grid">
        {candidates.map((c, i) => (
          <button
            type="button"
            key={i}
            className={i === selected ? "candidate selected" : "candidate"}
            aria-pressed={i === selected}
            aria-label={`候補${i + 1}`}
            onClick={() => select(i)}
          >
            <CpThumbnail cp={c.cp} />
            <span className="candidate-caption">
              候補{i + 1}:{violationLabel(c.violations)}
            </span>
            <span className="candidate-caption" data-fold-plan={i}>
              <span data-fold-plan-state={i}>{foldPlanLabel(c.fold_plan)}</span>
              {foldPlanStepsLabel(c.fold_plan) !== null && (
                <span data-fold-plan-steps={i}>
                  （{foldPlanStepsLabel(c.fold_plan)}）
                </span>
              )}
            </span>
          </button>
        ))}
      </div>
      <div className="button-row">
        <button type="button" onClick={() => setStep("skeleton")}>
          形を直す
        </button>
        <button type="button" disabled={busy} onClick={() => void generate()}>
          {busy ? "計算中…" : "別の置き方も見る"}
        </button>
        <button
          type="button"
          disabled={!canAdjustPaper}
          onClick={openPaperEditor}
        >
          紙の上の場所も調整
        </button>
        <button
          type="button"
          className="button-primary"
          disabled={selected === null}
          onClick={() => setStep("confirm")}
        >
          これにする
        </button>
      </div>
      {selected !== null && !canAdjustPaper && (
        <p className="hint">
          この候補では紙の上の場所を動かせません。別の候補を選んでください。
        </p>
      )}
      {error && (
        <p className="error-text">
          {proposalUserMessage(error, "retry-error")}
        </p>
      )}
    </>
  );
}

/** 選んだ候補だけを大きく出し、紙の上の場所を直接動かす任意画面(作業12) */
function PaperPositionStep() {
  const source = useAppStore((s) => s.proposalPaperSource);
  const candidate = useAppStore((s) =>
    s.proposalPaperSource === null
      ? null
      : (s.proposalCandidates[s.proposalPaperSource] ?? null),
  );
  const skeleton = useAppStore((s) => s.proposalSkeleton);
  const positions = useAppStore((s) => s.proposalPaperPositions);
  const paperSpecified = useAppStore((s) => s.proposalPaperSpecified);
  const lastMoved = useAppStore((s) => s.proposalPositionLastMoved);
  const paper = useAppStore((s) => s.doc?.paper) ?? PROPOSAL_FALLBACK_PAPER;
  const setPosition = useAppStore((s) => s.setProposalPaperPosition);
  const reset = useAppStore((s) => s.resetProposalPaperPositions);
  const generate = useAppStore((s) => s.generateProposalFromPaperPositions);
  const setStep = useAppStore((s) => s.setProposalStep);
  const busy = useAppStore((s) => s.proposalBusy);
  const error = useAppStore((s) => s.proposalError);

  if (!candidate || source === null) {
    return (
      <div className="button-row">
        <button type="button" onClick={() => setStep("candidates")}>
          候補へ戻る
        </button>
      </div>
    );
  }

  const original = new Map(
    paperPositionsFromCandidate(candidate).map((entry) => [
      entry.leaf_id,
      entry.position,
    ]),
  );
  const changedCount = positions.filter((entry) =>
    paperPositionChanged(entry.position, original.get(entry.leaf_id)),
  ).length;
  const shownIds = new Set(positions.map((entry) => entry.leaf_id));
  const specifiedCount = paperSpecified.filter((entry) =>
    shownIds.has(entry.leaf_id),
  ).length;
  const positionStates = proposalLeafPositionStates(
    skeleton,
    paperSpecified,
    lastMoved,
    paper,
  );

  return (
    <div className="paper-position-step">
      <div className="paper-position-sidebar">
        <h2 id="proposal-title">紙の上の場所を調整</h2>
        <ProposalPositionHistoryButtons />
        <p>
          丸い印をつまんで、紙の上でその先端を作りたい場所へ動かしてください。
        </p>
        <p
          className="paper-position-status"
          data-paper-position-status={changedCount > 0 ? "changed" : "original"}
          aria-live="polite"
        >
          {changedCount > 0
            ? `紙の上の場所を${changedCount}か所動かしました。`
            : "いまは選んだ候補と同じ場所です。"}
        </p>
        <ProposalPositionNotices />
        {error && (
          <p className="error-text">
            {proposalUserMessage(error, "paper-error")}
          </p>
        )}
      </div>
      <div className="paper-position-stage">
        <PaperPositionEditor
          candidate={candidate}
          skeleton={skeleton}
          positions={positions}
          disabled={busy}
          positionStates={positionStates}
          onPositionChange={setPosition}
        />
      </div>
      <div className="button-row paper-position-actions">
        <button
          type="button"
          disabled={busy}
          onClick={() => setStep("candidates")}
        >
          候補へ戻る
        </button>
        <button
          type="button"
          disabled={busy || specifiedCount === 0}
          onClick={reset}
        >
          この候補の場所に戻す
        </button>
        <button
          type="button"
          className="button-primary"
          disabled={busy}
          onClick={() => void generate()}
        >
          {busy ? "計算中…" : "この場所で作り直す"}
        </button>
      </div>
    </div>
  );
}

/** 3画面目: 選んだ候補を確かめて、今の作品の展開図として使う(PRO-003) */
function ConfirmStep() {
  const candidate = useAppStore((s) =>
    s.proposalSelected === null
      ? null
      : (s.proposalCandidates[s.proposalSelected] ?? null),
  );
  const existingStepCount = useAppStore((s) => s.doc?.sequence.length ?? 0);
  const setStep = useAppStore((s) => s.setProposalStep);
  const apply = useAppStore((s) => s.applyProposalCandidate);
  if (!candidate) return null;
  const planSteps = candidate.fold_plan?.steps.length ?? 0;

  return (
    <>
      <p>この展開図を今の作品に入れます。入れた後は自由に描き足せます。</p>
      <ProposalPositionNotices />
      {existingStepCount > 0 && (
        <p className="warning-text">
          この展開図を使うと、今ある折り手順{existingStepCount}
          件はすべて消えます。
        </p>
      )}
      <div className="proposal-body">
        {/* 折り方が付いているときは、折る線の山谷が決まった展開図がそのまま入る。
            入るものと同じ絵を見せる */}
        <CpThumbnail cp={candidate.fold_plan?.cp ?? candidate.cp} />
        <div>
          <p className="hint">{violationLabel(candidate.violations)}</p>
          <p className="hint" data-fold-plan="confirm">
            <span data-fold-plan-state="confirm">
              {foldPlanLabel(candidate.fold_plan)}
            </span>
            {foldPlanStepsLabel(candidate.fold_plan) !== null && (
              <span data-fold-plan-steps="confirm">
                （{foldPlanStepsLabel(candidate.fold_plan)}）
              </span>
            )}
          </p>
          {planSteps > 0 && (
            <p className="hint">
              「この展開図を使う」を押すと、展開図と折り方が一緒に入ります。
            </p>
          )}
          {candidate.warnings.map((w, i) => (
            <p className="warning-text" key={i}>
              {proposalUserMessage(w, "warning")}
            </p>
          ))}
          <p className="hint">
            気に入らなければ、入れた後で「元に戻す」で消せます。
          </p>
        </div>
      </div>
      <div className="button-row">
        <button type="button" onClick={() => setStep("candidates")}>
          選び直す
        </button>
        <button
          type="button"
          className="button-primary"
          onClick={() => void apply()}
        >
          この展開図を使う
        </button>
      </div>
    </>
  );
}

export function ProposalWizard() {
  const step = useAppStore((s) => s.proposalStep);
  if (step === null) return null;
  const title =
    step === "paper-position"
      ? "紙の上の場所を調整"
      : "形を決めて展開図を作ってもらう";
  return (
    <div className="dialog-backdrop">
      <div
        className="dialog dialog-wide"
        data-floating-ui="proposal-dialog"
        data-proposal-step={step}
        role="dialog"
        aria-modal="true"
        aria-labelledby="proposal-title"
        style={{
          width: `calc(100vw - ${PROPOSAL_DIALOG_VIEWPORT_GUTTER_PX}px)`,
          maxWidth: `${
            step === "paper-position"
              ? PROPOSAL_PAPER_DIALOG_MAX_WIDTH_PX
              : PROPOSAL_DIALOG_MAX_WIDTH_PX
          }px`,
          boxSizing: "border-box",
        }}
      >
        {step !== "paper-position" && <h2 id="proposal-title">{title}</h2>}
        {step !== "paper-position" && <ProposalPositionHistoryButtons />}
        {step === "skeleton" && <SkeletonStep />}
        {step === "candidates" && <CandidateStep />}
        {step === "paper-position" && <PaperPositionStep />}
        {step === "confirm" && <ConfirmStep />}
      </div>
    </div>
  );
}
