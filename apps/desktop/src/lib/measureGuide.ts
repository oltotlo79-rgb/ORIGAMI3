import type { MeasureMode } from "../store/appStore";

const RESULT_GUIDE: Record<MeasureMode, string> = {
  angle: "角度を表示しています",
  length: "線の長さを表示しています",
  distance: "2つの距離を表示しています",
};

/** 展開図・3D・下部パネルで共用する、次の操作を示す1行。 */
export function measureGuide(mode: MeasureMode, pickCount: number): string {
  if (mode === "length") {
    return pickCount === 0 ? "線を1本選んでください" : RESULT_GUIDE.length;
  }
  if (mode === "angle") {
    if (pickCount === 0) return "2つの辺を指定してください";
    if (pickCount === 1) return "あと1つの辺を指定してください";
    return RESULT_GUIDE.angle;
  }
  if (pickCount === 0) return "2つの点を指定してください";
  if (pickCount === 1) return "あと1つの点を指定してください";
  return RESULT_GUIDE.distance;
}

export const MEASURE_STEPS: Record<MeasureMode, string[]> = {
  angle: ["1つ目の辺を選ぶ", "あと1つの辺を選ぶ", "角度を確認"],
  length: ["線を1本選ぶ", "長さを確認"],
  distance: ["1つ目の点を選ぶ", "あと1つの点を選ぶ", "2つの距離を確認"],
};
