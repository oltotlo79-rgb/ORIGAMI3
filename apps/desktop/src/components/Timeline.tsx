// 手順タイムライン: 3Dビュー区画の下側に置く(常設区画は増やさない)。
// 手順のチップ(番号+技法名+警告)と再生コントロールだけを持ち、
// 3D表示への反映と再生の進行はストアが行う(要件§2: 状態はストア1本)。
// サムネイル画像は作らない(1手順ごとの再生費用が大きいため、v1は文字表示)。

import { isStepSkipped, useAppStore } from "../store/appStore";
import {
  TECHNIQUE_LABEL,
  uniqueWarnings,
  warningsForStep,
} from "../lib/techniques";
import type { FoldStep } from "../lib/types";
import { UiIcon } from "./UiIcon";

/** 飛ばされた手順の説明が再生の警告に無いときに出す文言(直し方まで書く) */
const SKIPPED_FALLBACK =
  "折り線が見つからないため、この手順は飛ばされました。展開図にその折り線を引き直すか、この手順を削除してください";

function chipClass(selected: boolean, skipped = false): string {
  return [
    "timeline-chip",
    selected ? "selected" : "",
    skipped ? "skipped" : "",
  ]
    .filter((c) => c !== "")
    .join(" ");
}

/** 手順1つぶんのチップ。number は利用者向けの手順番号(1始まり) */
function StepChip({ step, number }: { step: FoldStep; number: number }) {
  const currentStep = useAppStore((s) => s.currentStep);
  // 飛ばされたかどうかは作品全体の再生結果で決める(途中の手順を選んでいる間も、
  // その先の手順の赤表示を消さない)
  const isSkipped = useAppStore((s) => isStepSkipped(s, step.id));
  // 自動再生の警告は展開図の検査結果へ合流している。途中の手順を再生し直した
  // ときの警告と合わせて見る(同じ文言は1回だけ)
  const warnings = useAppStore((s) => s.warnings);
  const replayWarnings = useAppStore((s) => s.replayWarnings);
  const selectStep = useAppStore((s) => s.selectStep);

  const reasons = warningsForStep(
    uniqueWarnings(warnings, replayWarnings),
    number,
  );
  const warned = isSkipped || reasons.length > 0;
  const detail = reasons.length > 0 ? reasons.join(" / ") : "";
  const tooltip = isSkipped
    ? detail || SKIPPED_FALLBACK
    : [TECHNIQUE_LABEL[step.kind], step.note, detail]
        .filter((text) => text !== "")
        .join(" / ");

  return (
    <span className="timeline-slot">
      {/* この手順の前に折りを挟む導線(SEQ-006)。押すと1つ前の形を表示し、
          その状態で折ると新しい手順がここへ入る(後ろの手順は残る) */}
      <button
        type="button"
        className={insertClass(currentStep === number - 1)}
        data-testid={`timeline-insert-before-${number}`}
        data-tooltip={`手順${number}の前に新しい折りを挟みます`}
        onClick={() => selectStep(number - 1)}
      >
        ＋
      </button>
      <button
        type="button"
        className={chipClass(currentStep === number, isSkipped)}
        data-testid={`timeline-step-${number}`}
        data-tooltip={tooltip}
        onClick={() => selectStep(number)}
      >
        {number} {TECHNIQUE_LABEL[step.kind]}
        {warned ? " ⚠" : ""}
      </button>
    </span>
  );
}

/** 「ここに挿入」ボタンの見た目(挿入位置を表示中なら目立たせる) */
function insertClass(active: boolean): string {
  return active ? "timeline-insert active" : "timeline-insert";
}

export function Timeline() {
  const steps = useAppStore((s) => s.doc?.sequence ?? null);
  const currentStep = useAppStore((s) => s.currentStep);
  const playing = useAppStore((s) => s.playing);
  const selectStep = useAppStore((s) => s.selectStep);
  const stepBy = useAppStore((s) => s.stepBy);
  const togglePlay = useAppStore((s) => s.togglePlay);

  if (steps === null || steps.length === 0) {
    return (
      <div className="timeline" data-testid="timeline">
        <span className="hint">まだ手順がありません</span>
      </div>
    );
  }

  const atStart = currentStep === 0;
  const atEnd = currentStep === null || currentStep >= steps.length;

  return (
    <div className="timeline" data-testid="timeline">
      <div className="timeline-controls">
        <button
          type="button"
          disabled={atStart}
          data-testid="timeline-to-start"
          data-tooltip={
            atStart ? "すでに折る前の状態です" : "折る前の状態に戻します"
          }
          onClick={() => selectStep(0)}
        >
          <UiIcon name="skip-to-start" /> 最初へ
        </button>
        <button
          type="button"
          disabled={atStart}
          data-testid="timeline-previous"
          data-tooltip={
            atStart
              ? "まだ折る前なので、これより前へは戻れません"
              : "1つ前の手順を表示します"
          }
          onClick={() => stepBy(-1)}
        >
          ◀ 前へ
        </button>
        <button
          type="button"
          data-testid="timeline-play"
          data-tooltip={
            playing
              ? "再生を止めます"
                : "今の手順から最後まで続けて表示します"
          }
          onClick={() => togglePlay()}
        >
          {playing ? (
            <>
              <UiIcon name="pause" /> 一時停止
            </>
          ) : (
            "▶ 再生"
          )}
        </button>
        <button
          type="button"
          disabled={atEnd}
          data-testid="timeline-next"
          data-tooltip={
            atEnd
              ? "いちばん最後の状態なので、これより先へは進めません"
              : "1つ先の手順を表示します"
          }
          onClick={() => stepBy(1)}
        >
          次へ ▶
        </button>
      </div>
      <div className="timeline-steps">
        <button
          type="button"
          className={chipClass(currentStep === 0)}
          data-testid="timeline-step-0"
          data-tooltip="まだ折っていない平らな状態を表示します"
          onClick={() => selectStep(0)}
        >
          折る前
        </button>
        {steps.map((step, i) => (
          <StepChip key={step.id} step={step} number={i + 1} />
        ))}
        <button
          type="button"
          className={chipClass(currentStep === null)}
          data-testid="timeline-step-latest"
          data-tooltip="全手順を折った最新の状態を表示します"
          onClick={() => selectStep(null)}
        >
          最新
        </button>
      </div>
    </div>
  );
}
