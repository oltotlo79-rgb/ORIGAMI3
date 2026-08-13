import { uniqueWarnings } from "./techniques";

/** 同じ点を複数の折り目から見つけても、画面では点ごとに1件として扱う。 */
export function flatFoldViolationIds(ids: readonly number[]): number[] {
  return [...new Set(ids)];
}

/** 平らに畳めなかった点を、内部の判定名を使わず1行で知らせる。 */
export function flatFoldNotice(ids: readonly number[]): string | null {
  const count = flatFoldViolationIds(ids).length;
  if (count === 0) return null;
  return `この折り方では平らに畳めない点が${count}か所あります。場所は展開図の橙色の丸で確認してください。折り目を足すか、使う折り目を減らすと畳めるようになることがあります。`;
}

/** 通常の警告は文ごと、平らに畳めない場所は点ごとに3Dの件数へ数える。 */
export function warningCount(
  warnings: string[],
  poseWarnings: string[],
  replayWarnings: string[],
  flatFoldViolations: readonly number[],
): number {
  return (
    uniqueWarnings(warnings, poseWarnings, replayWarnings).length +
    flatFoldViolationIds(flatFoldViolations).length
  );
}

/** 3D右上の表示。平らに畳めない点は、追従中や不収束より先に知らせる。 */
export function statusBadgeText(input: {
  hasError: boolean;
  followStatus: string | null;
  poseConverged: boolean;
  warningCount: number;
  flatFoldViolationCount: number;
}): string | null {
  if (input.hasError) return "エラー";
  if (input.flatFoldViolationCount > 0) return `警告 ${input.warningCount}`;
  if (input.followStatus !== null) return input.followStatus;
  if (!input.poseConverged) return "⚠ 追従計算が収束していません";
  return input.warningCount > 0 ? `警告 ${input.warningCount}` : null;
}
