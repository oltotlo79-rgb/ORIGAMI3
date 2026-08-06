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
  limbLabel,
  limbs,
  removeLimb,
  setLimb,
} from "../../lib/skeleton";
import { SkeletonPreview } from "./SkeletonPreview";
import { CpThumbnail } from "./CpThumbnail";

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
  const list = limbs(skeleton);

  return (
    <>
      <p>
        頭・尾・足のような「出っぱり」を{MIN_LIMBS}〜{MAX_LIMBS}
        本まで決められます。長さと太さを変えると、下の絵がそのまま変わります。
      </p>
      <div className="proposal-body">
        <SkeletonPreview skeleton={skeleton} />
        <div className="limb-list">
          {list.map((n, i) => (
            <div className="limb-row" key={n.id}>
              <span className="limb-name">{limbLabel(i)}</span>
              <label>
                長さ
                <input
                  type="range"
                  aria-label={`${limbLabel(i)}の長さ`}
                  min={LENGTH_RANGE.min}
                  max={LENGTH_RANGE.max}
                  step={LENGTH_RANGE.step}
                  value={n.length}
                  onChange={(e) =>
                    setSkeleton(
                      setLimb(skeleton, n.id, { length: Number(e.target.value) }),
                    )
                  }
                />
              </label>
              <label>
                太さ
                <input
                  type="range"
                  aria-label={`${limbLabel(i)}の太さ`}
                  min={WIDTH_RANGE.min}
                  max={WIDTH_RANGE.max}
                  step={WIDTH_RANGE.step}
                  value={n.width_factor}
                  onChange={(e) =>
                    setSkeleton(
                      setLimb(skeleton, n.id, {
                        width_factor: Number(e.target.value),
                      }),
                    )
                  }
                />
              </label>
              <button
                type="button"
                aria-label={`${limbLabel(i)}を減らす`}
                disabled={list.length <= MIN_LIMBS}
                onClick={() => setSkeleton(removeLimb(skeleton, n.id))}
              >
                ✕
              </button>
            </div>
          ))}
        </div>
      </div>
      <div className="button-row">
        <button
          type="button"
          disabled={list.length >= MAX_LIMBS}
          onClick={() => setSkeleton(addLimb(skeleton))}
        >
          出っぱりを増やす
        </button>
        <button type="button" disabled={busy} onClick={() => void generate()}>
          {busy ? "計算中…" : "展開図を作ってもらう"}
        </button>
        <button type="button" onClick={close}>
          やめる
        </button>
      </div>
      {error && <p className="error-text">{error}</p>}
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
          disabled={selected === null}
          onClick={() => setStep("confirm")}
        >
          これにする
        </button>
      </div>
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
  const setStep = useAppStore((s) => s.setProposalStep);
  const apply = useAppStore((s) => s.applyProposalCandidate);
  if (!candidate) return null;

  return (
    <>
      <p>この展開図を今の作品に入れます。入れた後は自由に描き足せます。</p>
      <div className="proposal-body">
        <CpThumbnail cp={candidate.cp} />
        <div>
          <p className="hint">{violationLabel(candidate.violations)}</p>
          {candidate.warnings.map((w, i) => (
            <p className="warning-text" key={i}>
              {w}
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
        <button type="button" onClick={() => void apply()}>
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
        role="dialog"
        aria-modal="true"
        aria-labelledby="proposal-title"
      >
        <h2 id="proposal-title">形を決めて展開図を作ってもらう</h2>
        {step === "skeleton" && <SkeletonStep />}
        {step === "candidates" && <CandidateStep />}
        {step === "confirm" && <ConfirmStep />}
      </div>
    </div>
  );
}
