import type { UiTheme } from "../lib/displayPrefs";

/** テーマごとのアートディレクションを保ったツールバーロゴ。 */
export function ToolbarBrandMark({ theme }: { theme: UiTheme }) {
  return (
    <svg
      className="toolbar-brand-mark"
      viewBox="0 0 44 44"
      aria-hidden="true"
      focusable="false"
    >
      {theme === "simple" ? (
        <g
          fill="none"
          stroke="var(--color-accent)"
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth="2"
        >
          <path d="M4 24 18 8 17 23 39 12 27 29 37 37 23 31 12 39 17 25Z" />
          <path d="m27 29 4-19 7-5-4 12" />
          <path d="m17 23 10 6-9-21" />
        </g>
      ) : theme === "japanese" ? (
        <>
          <polygon
            points="22,3 26,8 24,15 40,10 34,24 28,27 36,38 22,31 8,38 16,27 10,24 4,10 20,15 18,9 19,5"
            fill="var(--color-accent)"
          />
          <g
            fill="none"
            stroke="var(--color-bg)"
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth="1.5"
          >
            <path d="M4 10 22 25 40 10" />
            <path d="m10 24 12 1 12-1" />
            <path d="M22 15v16" />
          </g>
          <rect
            x="31"
            y="2"
            width="12"
            height="12"
            rx="1"
            fill="var(--color-secondary)"
          />
          <text
            x="37"
            y="11"
            fill="var(--color-on-solid)"
            fontFamily="Yu Mincho, Hiragino Mincho ProN, serif"
            fontSize="8"
            fontWeight="700"
            textAnchor="middle"
          >
            折
          </text>
        </>
      ) : theme === "modern" ? (
        <g transform="rotate(45 22 22)">
          <polygon points="11,11 33,11 11,33" fill="var(--color-accent)" />
          <polygon points="33,11 33,33 11,33" fill="var(--color-text)" />
          <path
            d="M33 11 11 33"
            fill="none"
            stroke="var(--color-on-solid)"
            strokeWidth="2"
          />
        </g>
      ) : theme === "classic" ? (
        <>
          <circle
            cx="22"
            cy="22"
            r="20"
            fill="none"
            stroke="var(--color-pop-yellow)"
            strokeWidth="1"
          />
          <circle
            cx="22"
            cy="22"
            r="18"
            fill="none"
            stroke="var(--color-pop-yellow)"
            strokeWidth="1"
          />
          <g
            fill="none"
            stroke="var(--color-accent)"
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth="1.5"
          >
            <path d="M5 24 18 8 17 23 39 12 27 29 37 37 23 31 12 39 17 25Z" />
            <path d="m27 29 4-19 7-5-4 12" />
            <path d="m19 22 13-7M20 25l10-6M22 27l7-5" />
          </g>
        </>
      ) : (
        <>
          <circle cx="22" cy="22" r="20" fill="var(--color-accent-soft)" />
          <path d="M4 22 20 7l-3 17Z" fill="var(--color-secondary)" />
          <path d="m17 24 22-12-14 18Z" fill="var(--color-pop-yellow)" />
          <path d="m17 24 8 6-12 8Z" fill="var(--color-pop-coral)" />
          <path d="m25 30 7 6-9-3Z" fill="var(--color-accent)" />
          <path d="m25 30 6-20 5-4-4 22Z" fill="var(--color-accent)" />
          <path
            d="m17 24 8 6-5-23Z"
            fill="var(--color-on-solid)"
            fillOpacity=".76"
          />
          <circle cx="34.5" cy="8" r="1.1" fill="var(--color-text)" />
          <path
            d="M7 9v4M5 11h4M37 28v4M35 30h4"
            fill="none"
            stroke="var(--color-secondary)"
            strokeLinecap="round"
            strokeWidth="1.8"
          />
        </>
      )}
    </svg>
  );
}
