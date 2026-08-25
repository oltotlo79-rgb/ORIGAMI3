import { flatFoldViolationIds, statusBadgeText, warningCount } from "../lib/flatFoldNotice";
import type { AngleRelaxation } from "../lib/types";
import { relaxationNotices, useAppStore } from "../store/appStore";

/** 3D右上へ出す自然追従の短い知らせ。nullなら通常の警告表示へ譲る。 */
export function relaxationStatus(
  relaxations: readonly AngleRelaxation[],
  bestEffort: boolean,
): string | null {
  if (bestEffort) return "指定を優先し、いちばん近い形で追従中";
  const notices = relaxationNotices(relaxations);
  if (notices.length === 0) return null;
  const maxDelta = Math.max(...notices.map((item) => Math.abs(item.delta_deg)));
  return `前の折り目${notices.length}本が追従（最大${maxDelta.toFixed(1)}°）`;
}

export function ViewerStatusOverlays() {
  const warningCountValue = useAppStore((s) =>
    warningCount(
      s.warnings,
      s.poseWarnings,
      s.replayWarnings,
      s.flatFoldViolations,
    ),
  );
  const flatFoldViolationCount = useAppStore(
    (s) => flatFoldViolationIds(s.flatFoldViolations).length,
  );
  const poseConverged = useAppStore((s) => s.poseConverged);
  const relaxations = useAppStore((s) => s.relaxations);
  const poseBestEffort = useAppStore((s) => s.poseBestEffort);
  const hasError = useAppStore((s) => s.errorMessage !== null);
  const suspectHinges = useAppStore((s) => s.suspectHinges);
  const setSelection = useAppStore((s) => s.setSelection);
  const followStatus = relaxationStatus(relaxations, poseBestEffort);
  const badgeText = statusBadgeText({
    hasError,
    followStatus,
    poseConverged,
    warningCount: warningCountValue,
    flatFoldViolationCount,
  });
  const showFollowStatus =
    !hasError && flatFoldViolationCount === 0 && followStatus !== null;

  return (
    <>
      {badgeText !== null && (
        <div
          className={
            hasError
              ? "status-badge error"
              : showFollowStatus
                ? "status-badge follow"
                : "status-badge"
          }
          data-floating-ui="status-badge"
          data-tooltip={badgeText}
        >
          <svg
            className="status-icon"
            viewBox="0 0 24 24"
            aria-hidden="true"
            focusable="false"
          >
            <path
              d="M12 3 22 20H2L12 3Z"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.4"
            />
            <path
              d="M12 8v6"
              stroke="currentColor"
              strokeLinecap="round"
              strokeWidth="2.4"
            />
            <circle cx="12" cy="17.5" r="1.2" fill="currentColor" />
          </svg>
          <span>{badgeText}</span>
        </div>
      )}
      {suspectHinges.length > 0 && (
        <button
          type="button"
          className="suspect-hinge-guide"
          data-floating-ui="suspect-hinge-guide"
          data-tooltip="赤く光る折り目の角度を見直してください。押すと原因候補を選びます"
          onClick={() =>
            setSelection({
              edgeIds: [suspectHinges[0]],
              vertexIds: [],
            })
          }
        >
          赤く光る折り目の角度を見直してください
        </button>
      )}
    </>
  );
}
