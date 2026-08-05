// 手順タイムライン: 3Dビュー区画の下側に置く(常設区画は増やさない)。
// 手順のチップ(番号+技法名+警告)と再生コントロールだけを持ち、
// 3D表示への反映と再生の進行はストアが行う(要件§2: 状態はストア1本)。
// サムネイル画像は作らない(1手順ごとの再生費用が大きいため、v1は文字表示)。

import { useAppStore } from "../store/appStore";
import {
  TECHNIQUE_LABEL,
  uniqueWarnings,
  warningsForStep,
} from "../lib/techniques";
import type { FoldStep } from "../lib/types";

/** 飛ばされた手順の説明が再生の警告に無いときに出す文言 */
const SKIPPED_FALLBACK = "折り線が見つからないため、この手順は飛ばされました";

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
  const skipped = useAppStore((s) => s.skipped);
  // 自動再生の警告は展開図の検査結果へ合流している。途中の手順を再生し直した
  // ときの警告と合わせて見る(同じ文言は1回だけ)
  const warnings = useAppStore((s) => s.warnings);
  const replayWarnings = useAppStore((s) => s.replayWarnings);
  const selectStep = useAppStore((s) => s.selectStep);

  const isSkipped = skipped.includes(step.id);
  const reasons = warningsForStep(
    uniqueWarnings(warnings, replayWarnings),
    number,
  );
  const warned = isSkipped || reasons.length > 0;
  const detail = reasons.length > 0 ? reasons.join(" / ") : "";
  const title = isSkipped
    ? detail || SKIPPED_FALLBACK
    : [TECHNIQUE_LABEL[step.kind], step.note, detail]
        .filter((text) => text !== "")
        .join(" / ");

  return (
    <button
      type="button"
      className={chipClass(currentStep === number, isSkipped)}
      title={title}
      onClick={() => selectStep(number)}
    >
      {number} {TECHNIQUE_LABEL[step.kind]}
      {warned ? " ⚠" : ""}
    </button>
  );
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
      <div className="timeline">
        <span className="hint">まだ手順がありません</span>
      </div>
    );
  }

  return (
    <div className="timeline">
      <div className="timeline-controls">
        <button
          type="button"
          title="折る前の状態に戻します"
          onClick={() => selectStep(0)}
        >
          ⏮ 最初へ
        </button>
        <button
          type="button"
          title="1つ前の手順を表示します"
          onClick={() => stepBy(-1)}
        >
          ◀ 前へ
        </button>
        <button
          type="button"
          title={
            playing
              ? "再生を止めます"
              : "今の手順から最後まで、折れていく様子を続けて表示します"
          }
          onClick={() => togglePlay()}
        >
          {playing ? "⏸ 一時停止" : "▶ 再生"}
        </button>
        <button
          type="button"
          title="1つ先の手順を表示します"
          onClick={() => stepBy(1)}
        >
          次へ ▶
        </button>
      </div>
      <div className="timeline-steps">
        <button
          type="button"
          className={chipClass(currentStep === 0)}
          title="まだ何も折っていない、平らな状態を表示します"
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
          title="全ての手順を折った、いちばん新しい状態を表示します"
          onClick={() => selectStep(null)}
        >
          最新
        </button>
      </div>
    </div>
  );
}
