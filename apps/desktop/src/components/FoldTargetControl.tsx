import { useEffect, type ChangeEvent } from "react";
import { useAppStore, type FoldDraft } from "../store/appStore";
import { NumberStepper } from "./NumberStepper";

const ALL_TARGET_LABEL = "この線にかかる紙を全部（既定）";
const TOP_TARGET_LABEL = "いちばん上の紙だけ";
const K_INPUT_LABEL = "同時に折るひだの枚数";

type FoldTargetControlVariant = "context" | "viewer3d";

export interface FoldTargetControlProps {
  draft: FoldDraft;
  disabled: boolean;
  variant: FoldTargetControlVariant;
}

function availableCountOf(draft: FoldDraft): number | null {
  const count = draft.foldTargetInfo?.availableCount;
  return count !== null && count !== undefined && Number.isSafeInteger(count) && count >= 0
    ? count
    : null;
}

function shownPleatCount(draft: FoldDraft): number {
  return draft.target === "topPleats" &&
    Number.isSafeInteger(draft.topPleatCount) &&
    draft.topPleatCount >= 1
    ? draft.topPleatCount
    : 1;
}

export function isCreaseOnlyFoldTarget(draft: FoldDraft): boolean {
  return (
    draft.foldTargetInfo?.status === "crease_only_top" ||
    draft.foldTargetInfo?.topAction === "crease_only_top"
  );
}

/**
 * all/topは照会に失敗しても従来どおり続けられる。Kだけは、現在の上限を
 * 確認できない間と上限外のときに確定させない。crease-onlyのCTAは別経路なので止めない。
 */
export function foldTargetCommitBlocked(draft: FoldDraft): boolean {
  if (isCreaseOnlyFoldTarget(draft) || draft.target !== "topPleats") return false;
  const count = availableCountOf(draft);
  return (
    draft.foldTargetBusy === true ||
    (draft.foldTargetInfo?.status !== "ready" &&
      draft.foldTargetInfo?.status !== "limited") ||
    count === null ||
    count < 1 ||
    draft.topPleatCount < 1 ||
    draft.topPleatCount > count
  );
}

function FoldTargetStatus({ draft }: { draft: FoldDraft }) {
  const info = draft.foldTargetInfo ?? null;
  const count = availableCountOf(draft);
  const selectedCount = shownPleatCount(draft);

  if (draft.foldTargetBusy === true || info === null) {
    return (
      <p className="hint" aria-live="polite">
        この折り線で同時に折れるひだを確認しています…
      </p>
    );
  }

  if (isCreaseOnlyFoldTarget(draft)) {
    return (
      <div aria-live="polite">
        <p className="warning-text">この折り線で同時に折れるひだ：0枚</p>
        <p className="warning-text">
          いちばん上の紙が最後まで折り重なっていないため、今回はひだをまとめて折りません。いちばん上の紙に折り目だけを付け、下の紙と3Dの形は動かしません。
        </p>
      </div>
    );
  }

  if (info.status === "varies") {
    return (
      <div aria-live="polite">
        <p className="warning-text">
          折り線の場所によって、同時に折れるひだの枚数が異なります。
        </p>
        <p className="hint">
          ひだの枚数は選べません。今までどおり「この線にかかる紙を全部」か「いちばん上の紙だけ」なら、このまま折れます。
        </p>
      </div>
    );
  }

  if (info.status === "unavailable" || count === null) {
    return (
      <p className="warning-text" aria-live="polite">
        この折り線で同時に折れるひだを確認できません。
      </p>
    );
  }

  const overLimit = draft.target === "topPleats" && selectedCount > count;
  return (
    <div aria-live="polite">
      {info.status === "limited" ? (
        <p className="hint">
          上から{count}枚まで選べます。{count}
          枚目の下は、まだ最後まで折り重なっていません。
        </p>
      ) : (
        <p className="hint">この折り線で同時に折れるひだ：{count}枚</p>
      )}
      {draft.target === "topPleats" && !overLimit && (
        <p className="hint">上から{selectedCount}枚のひだを同時に折ります。</p>
      )}
      {overLimit && (
        <p className="warning-text">
          選んだ{selectedCount}枚は、今同時に折れる{count}
          枚を超えています。1枚から{count}枚までで選び直してください。
        </p>
      )}
    </div>
  );
}

export function FoldTargetControl({
  draft,
  disabled,
  variant,
}: FoldTargetControlProps) {
  const setFoldTarget = useAppStore((state) => state.setFoldTarget);
  const requestFoldTargetInfo = useAppStore(
    (state) => state.requestFoldTargetInfo,
  );
  const count = availableCountOf(draft);
  const selectedCount = shownPleatCount(draft);
  const [[lineAX, lineAY], [lineBX, lineBY]] = draft.line;
  const creaseOnly = isCreaseOnlyFoldTarget(draft);
  const targetDisabled = disabled || creaseOnly;
  const kDisabled =
    targetDisabled ||
    draft.foldTargetBusy === true ||
    draft.foldTargetInfo === null ||
    draft.foldTargetInfo === undefined ||
    draft.foldTargetInfo.status === "varies" ||
    draft.foldTargetInfo.status === "unavailable" ||
    count === null ||
    count < 1;
  const wrapperClassName =
    variant === "context"
      ? "button-row"
      : "paper-action-tip-buttons fold-direction-tip-buttons";
  const kSelectionLabel = `上から${selectedCount}枚のひだを同時に折る`;

  useEffect(() => {
    if (
      draft.foldTargetBusy !== true &&
      (draft.foldTargetInfo === null || draft.foldTargetInfo === undefined)
    ) {
      void requestFoldTargetInfo();
    }
  }, [
    draft.docEpoch,
    draft.foldTargetBusy,
    draft.foldTargetInfo,
    lineAX,
    lineAY,
    lineBX,
    lineBY,
    draft.movingSide,
    draft.stepCount,
    draft.upTo,
    requestFoldTargetInfo,
  ]);

  const selectPleatCount = (event: ChangeEvent<HTMLInputElement>) => {
    const next = event.currentTarget.valueAsNumber;
    if (Number.isSafeInteger(next) && next >= 1) {
      setFoldTarget({ target: "topPleats", topPleatCount: next });
    }
  };

  const targetOption = (
    target: "all" | "top",
    label: string,
  ) =>
    variant === "context" ? (
      <label>
        <input
          type="radio"
          name="fold-target"
          disabled={targetDisabled}
          checked={draft.target === target}
          onChange={() => setFoldTarget({ target })}
        />
        {label}
      </label>
    ) : (
      <button
        type="button"
        aria-pressed={draft.target === target}
        disabled={targetDisabled}
        onClick={() => setFoldTarget({ target })}
      >
        {label}
      </button>
    );

  const pleatSelector =
    variant === "context" ? (
      <label>
        <input
          type="radio"
          name="fold-target"
          aria-label={kSelectionLabel}
          disabled={kDisabled}
          checked={draft.target === "topPleats"}
          onChange={() =>
            setFoldTarget({
              target: "topPleats",
              topPleatCount: selectedCount,
            })
          }
        />
        上から
      </label>
    ) : (
      <button
        type="button"
        aria-label={kSelectionLabel}
        aria-pressed={draft.target === "topPleats"}
        disabled={kDisabled}
        onClick={() =>
          setFoldTarget({
            target: "topPleats",
            topPleatCount: selectedCount,
          })
        }
      >
        上から
      </button>
    );

  return (
    <>
      <div className={wrapperClassName} role="group" aria-label="折る紙">
        <span className="row-label">折る紙</span>
        {targetOption("all", ALL_TARGET_LABEL)}
        {pleatSelector}
        <NumberStepper
          aria-label={K_INPUT_LABEL}
          value={selectedCount}
          min={1}
          max={count !== null && count >= 1 ? count : undefined}
          step={1}
          disabled={kDisabled}
          onChange={selectPleatCount}
        />
        <span>枚のひだを同時に折る</span>
        {targetOption("top", TOP_TARGET_LABEL)}
        {count !== null && count >= 1 && !kDisabled && (
          <button
            type="button"
            disabled={targetDisabled}
            onClick={() =>
              setFoldTarget({ target: "topPleats", topPleatCount: count })
            }
          >
            同時に折れる{count}枚を全部選ぶ
          </button>
        )}
      </div>
      <p className="hint">{TOP_IS_NOT_ONE_PLEAT}</p>
      <FoldTargetStatus draft={draft} />
    </>
  );
}

const TOP_IS_NOT_ONE_PLEAT =
  "「いちばん上の紙だけ」は、「ひだを1枚」とは別です。";
