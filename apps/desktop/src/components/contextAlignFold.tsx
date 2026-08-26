import {
  nextAlignKind,
  useAppStore,
  type AlignDraft,
  type FoldDraft,
  type PendingFoldThrough,
} from "../store/appStore";
import {
  ALIGN_HINTS,
  ALIGN_LABELS,
  ALIGN_STEPS,
  type AlignMode,
} from "../lib/alignFold";
import {
  FoldTargetControl,
  foldTargetCommitBlocked,
  isCreaseOnlyFoldTarget,
} from "./FoldTargetControl";

/** 「合わせて折る」の入口(折るツールのときだけ出す。ツールレールは増やさない) */
const ALIGN_MODES: AlignMode[] = [
  "throughTwoPoints",
  "pointPoint",
  "lineLine",
  "pointPerpendicularLine",
  "pointLineThrough",
  "pointToLinePointToLine",
  "pointLinePerpendicular",
  "existingLine",
];

export function AlignStartRow() {
  const beginAlign = useAppStore((s) => s.beginAlign);
  return (
    <div className="button-row align-start-row">
      <strong className="align-start-label row-label">合わせて折る</strong>
      <div
        className="align-mode-buttons"
        role="group"
        aria-label="折り目の決め方"
      >
        {ALIGN_MODES.map((mode) => (
          <button
            key={mode}
            type="button"
            data-tooltip={ALIGN_HINTS[mode]}
            onClick={() => beginAlign(mode)}
          >
            {ALIGN_LABELS[mode]}
          </button>
        ))}
      </div>
    </div>
  );
}

/**
 * 合わせて折るの途中経過と、求まった折り線の確定UI。
 * 折り線が求まったら、下に既存の折り確定UI(山谷・対象の層・折る)をそのまま出す。
 */
export function AlignDraftContent({
  draft,
  foldDraft,
}: {
  draft: AlignDraft;
  foldDraft: FoldDraft | null;
}) {
  const nextAlignSolution = useAppStore((s) => s.nextAlignSolution);
  const undoAlignPick = useAppStore((s) => s.undoAlignPick);
  const cancelAlign = useAppStore((s) => s.cancelAlign);
  const need = ALIGN_STEPS[draft.mode].length;
  const kind = nextAlignKind(draft);

  return (
    <div>
      <div className="button-row">
        <strong>{ALIGN_LABELS[draft.mode]}</strong>
        {/* 進み具合は技法の名前と地続きに読めていたので、控えめな色で区別する */}
        <span className="align-draft-progress">
          選択 {draft.picks.length} / {need}
          {kind !== null &&
            `(次は${kind === "point" ? "点" : "線"}を展開図または3D表示でクリック)`}
        </span>
        {draft.solutions.length >= 2 && (
          <button type="button" onClick={() => nextAlignSolution()}>
            別の解({draft.solutionIndex + 1}/{draft.solutions.length})
          </button>
        )}
        <button
          type="button"
          disabled={draft.picks.length === 0}
          onClick={() => undoAlignPick()}
        >
          1つ戻す
        </button>
        <button type="button" onClick={() => cancelAlign()}>
          合わせるのをやめる
        </button>
      </div>
      {draft.reason !== null && <p className="warning-text">{draft.reason}</p>}
      {foldDraft && <FoldDraftContent draft={foldDraft} showPleatTargets />}
    </div>
  );
}

/** 引いた折り線の確定UI(向き・対象の層を決めて折る) */
export function FoldDraftContent({
  draft,
  showPleatTargets = false,
}: {
  draft: FoldDraft;
  showPleatTargets?: boolean;
}) {
  const paper = useAppStore((s) => s.doc?.paper ?? null);
  const updateFoldDraft = useAppStore((s) => s.updateFoldDraft);
  const cancelFoldDraft = useAppStore((s) => s.cancelFoldDraft);
  const commitFoldDraft = useAppStore((s) => s.commitFoldDraft);
  const busy = useAppStore((s) => s.foldThroughBusy);
  const creaseOnly = isCreaseOnlyFoldTarget(draft);
  const commitDisabled = busy || foldTargetCommitBlocked(draft);
  const [a, b] = draft.line;
  // 座標は「紙の長辺=1」に正規化された値なので、紙の寸法を掛けてmmで見せる
  const scale = paper ? Math.max(paper.width_mm, paper.height_mm) : 1;
  const mm = (v: number) => (v * scale).toFixed(1);

  return (
    <div>
      <p>
        折り線: ({mm(a[0])}, {mm(a[1])}) →({mm(b[0])}, {mm(b[1])}) mm
      </p>
      {/* 「向き」「対象の層」は別々の問いなので、1行に1問ずつ置く。
          同じ行へ詰めると問いの切れ目が隙間の違いで分からなくなる。
          先頭のラベルは .row-label で同じ列幅にし、答えの左端をそろえる。 */}
      <div className="button-row">
        <span className="row-label">向き</span>
        <label>
          <input
            type="radio"
            name="fold-direction"
            disabled={busy}
            checked={draft.direction === "Up"}
            onChange={() => updateFoldDraft({ direction: "Up" })}
          />
          手前へ折る(谷)
        </label>
        <label>
          <input
            type="radio"
            name="fold-direction"
            disabled={busy}
            checked={draft.direction === "Down"}
            onChange={() => updateFoldDraft({ direction: "Down" })}
          />
          向こうへ折る(山)
        </label>
      </div>
      {showPleatTargets ? (
        <FoldTargetControl draft={draft} disabled={busy} variant="context" />
      ) : (
        <div className="button-row">
          <span className="row-label">対象の層</span>
          <label>
            <input
              type="radio"
              name="fold-target"
              disabled={busy}
              checked={draft.target === "all"}
              onChange={() => updateFoldDraft({ target: "all" })}
            />
            全ての層
          </label>
          <label>
            <input
              type="radio"
              name="fold-target"
              disabled={busy}
              checked={draft.target === "top"}
              onChange={() => updateFoldDraft({ target: "top" })}
            />
            いちばん上の1枚
          </label>
        </div>
      )}
      {/* 折り返す紙は1つ目の選択から自動で決め、黄色が見える3D上の札で示す。
          このパネルで同じ内容を二択として聞き直さない。 */}
      <div className="button-row">
        <button
          type="button"
          disabled={commitDisabled}
          onClick={() => void commitFoldDraft()}
        >
          {busy ? "折り方を確認中…" : creaseOnly ? "折り目を付ける" : "折る"}
        </button>
        <button type="button" disabled={busy} onClick={() => cancelFoldDraft()}>
          やめる
        </button>
      </div>
    </div>
  );
}

/**
 * 巻き込み用の折り目を入れるか、その場で選ぶ非モーダルの提案。
 * 「追加しない」も元の折りは実行し、貫通が残れば通常の警告として知らせる。
 */
export function FoldThroughProposalContent({ pending }: { pending: PendingFoldThrough }) {
  const busy = useAppStore((s) => s.foldThroughBusy);
  const resolve = useAppStore((s) => s.resolveFoldThroughProposal);
  const count = pending.proposal.crease_segments.length;

  return (
    <section className="fold-through-proposal" aria-label="巻き込み折り目の提案">
      <p className="fold-through-proposal-title">
        指定した場所以外に、ここへ折り目がつきます
      </p>
      <p>{pending.proposal.message}</p>
      <p className="hint">
        追加位置を展開図の橙色の破線と、3D表示の水色の線で確認できます
        {count > 1 ? `（展開図では${count}線分）` : ""}。
      </p>
      <div className="button-row">
        <button type="button" disabled={busy} onClick={() => void resolve(true)}>
          {busy ? "適用中…" : "追加折り目を入れて折る"}
        </button>
        <button type="button" disabled={busy} onClick={() => void resolve(false)}>
          追加せず折る（警告のみ）
        </button>
      </div>
    </section>
  );
}
