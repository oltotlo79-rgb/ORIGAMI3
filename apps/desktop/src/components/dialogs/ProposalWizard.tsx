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
  skeletonRows,
} from "../../lib/skeleton";
import {
  PROPOSAL_DIALOG_MAX_WIDTH_PX,
  PROPOSAL_DIALOG_VIEWPORT_GUTTER_PX,
  PROPOSAL_LIST_BASIS_PX,
  PROPOSAL_PREVIEW_MAX_WIDTH_PX,
  PROPOSAL_ROW_INDENT_MAX_PX,
  PROPOSAL_ROW_INDENT_STEP_PX,
} from "../../lib/proposalLayout";
import { SkeletonPreview } from "./SkeletonPreview";
import { CpThumbnail } from "./CpThumbnail";

const INTERNAL_PROPOSAL_WORDS =
  /骨格|充填|ソルバー|ヤコビアン|hard|soft|warm[\s-]+start|イテレーション|内部エラー|節点|木|根|深さ|円の中心|角|ID/iu;
const INTERNAL_MESSAGE_SHAPE = /[A-Za-z_{}[\]=]|\d/iu;

type ProposalMessageKind = "initial-error" | "retry-error" | "warning";

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
  return "展開図を作れませんでした。上の出っぱりの本数・長さ・太さを見直してから、もう一度「展開図を作ってもらう」を押してください。";
}

/** 折りにくさの目安を日本語にする(数字だけだと良し悪しが伝わらない) */
export function violationLabel(count: number): string {
  if (count === 0) return "きれいに畳めそうです";
  return `折りたたみにくい点 ${count} か所`;
}

/** 1画面目: 出っぱりの数と長さ・太さを決める */
function SkeletonStep() {
  const skeleton = useAppStore((s) => s.proposalSkeleton);
  const setSkeleton = useAppStore((s) => s.setProposalSkeleton);
  const generate = useAppStore((s) => s.generateProposal);
  const busy = useAppStore((s) => s.proposalBusy);
  const error = useAppStore((s) => s.proposalError);
  const close = useAppStore((s) => s.closeProposal);
  const leaves = leafNodes(skeleton);
  const leafIds = new Set(leaves.map((node) => node.id));
  const rows = skeletonRows(skeleton);
  const rowById = new Map(rows.map((row) => [row.node.id, row]));
  const pathParts = new Map(
    rows.map((row) => {
      const labels = [row.label];
      let parent = row.node.parent;
      while (parent !== null) {
        const parentRow = rowById.get(parent);
        if (!parentRow) break;
        labels.unshift(parentRow.label);
        parent = parentRow.node.parent;
      }
      return [row.node.id, labels] as const;
    }),
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
        本まで決められます。それぞれの先へ足すこともでき、長さと先端の太さを変えると下の絵がそのまま変わります。
      </p>
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
          <SkeletonPreview skeleton={skeleton} />
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
  const select = useAppStore((s) => s.selectProposalCandidate);
  const setStep = useAppStore((s) => s.setProposalStep);
  const generate = useAppStore((s) => s.generateProposal);
  const busy = useAppStore((s) => s.proposalBusy);
  const error = useAppStore((s) => s.proposalError);

  return (
    <>
      <p>
        同じ形から{candidates.length}
        通りの置き方ができました。好きなものを選んでください。
      </p>
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
          className="button-primary"
          disabled={selected === null}
          onClick={() => setStep("confirm")}
        >
          これにする
        </button>
      </div>
      {error && (
        <p className="error-text">
          {proposalUserMessage(error, "retry-error")}
        </p>
      )}
    </>
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

  return (
    <>
      <p>この展開図を今の作品に入れます。入れた後は自由に描き足せます。</p>
      {existingStepCount > 0 && (
        <p className="warning-text">
          この展開図を使うと、今ある折り手順{existingStepCount}
          件はすべて消えます。
        </p>
      )}
      <div className="proposal-body">
        <CpThumbnail cp={candidate.cp} />
        <div>
          <p className="hint">{violationLabel(candidate.violations)}</p>
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
  return (
    <div className="dialog-backdrop">
      <div
        className="dialog dialog-wide"
        data-floating-ui="proposal-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="proposal-title"
        style={{
          width: `calc(100vw - ${PROPOSAL_DIALOG_VIEWPORT_GUTTER_PX}px)`,
          maxWidth: `${PROPOSAL_DIALOG_MAX_WIDTH_PX}px`,
          boxSizing: "border-box",
        }}
      >
        <h2 id="proposal-title">形を決めて展開図を作ってもらう</h2>
        {step === "skeleton" && <SkeletonStep />}
        {step === "candidates" && <CandidateStep />}
        {step === "confirm" && <ConfirmStep />}
      </div>
    </div>
  );
}
