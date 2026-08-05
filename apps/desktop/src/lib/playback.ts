// 手順再生の進み具合の計算(純関数)。画面にもIPCにも触れないのでそのまま試験できる。
// 1手順を STEP_DURATION_MS かけて 0→1 で補間し、折り終わったら次の手順へ進む。
// 実際の描画(sequence_replayの呼び出しとコマの予約)はストアが行う。

/** 1手順あたりのアニメーション時間(ms) */
export const STEP_DURATION_MS = 320;

export interface PlaybackState {
  /** 表示中の手順番号(1始まり。0は「折る前」) */
  step: number;
  /** 表示中の手順の進み具合(0..1。1で折り終わり) */
  t: number;
  playing: boolean;
}

/**
 * 1コマ分だけ進める。totalStepsは手順の総数で、最終手順を折り終えたら停止する。
 * 画面が長く止まっていた場合でも、1コマで進めるのは最大1手順ぶんまでにする
 * (裏に回っていた間の時間で何手順も飛ばして見えるのを防ぐ)。
 */
export function advancePlayback(
  state: PlaybackState,
  dtMs: number,
  totalSteps: number,
): PlaybackState {
  if (!state.playing) return state;
  if (totalSteps <= 0) return { step: 0, t: 1, playing: false };
  if (state.step >= totalSteps && state.t >= 1) {
    return { step: totalSteps, t: 1, playing: false };
  }
  const dt = Math.min(Math.max(dtMs, 0), STEP_DURATION_MS);
  const step = Math.max(0, state.step);
  const t = state.t + dt / STEP_DURATION_MS;
  if (t < 1) return { step, t, playing: true };
  if (step >= totalSteps) return { step: totalSteps, t: 1, playing: false };
  // 手順の境目を越えた。余った時間は切り捨て、1コマで進むのは1手順ぶんまでにする
  const rest = Math.min(t - 1, 1);
  return { step: step + 1, t: rest, playing: rest < 1 || step + 1 < totalSteps };
}

/**
 * 再生開始時の位置を決める。
 * 最新表示(null)や最後まで折り終えた状態からは「折る前」に戻して最初から見せ、
 * 途中で一時停止していた場合はその続きから再開する。
 */
export function startPlayback(
  currentStep: number | null,
  t: number,
  totalSteps: number,
): PlaybackState {
  if (totalSteps <= 0) return { step: 0, t: 1, playing: false };
  if (currentStep === null || (currentStep >= totalSteps && t >= 1)) {
    return { step: 0, t: 1, playing: true };
  }
  return { step: Math.max(0, currentStep), t, playing: true };
}
