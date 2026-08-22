import type { ToolId } from "../store/appStore";

type ToolIconName = ToolId | "fit";
export type ToolbarIconName =
  | "new"
  | "open"
  | "save"
  | "undo"
  | "redo"
  | "proposal"
  | "export"
  | "help";

interface ToolIconProps {
  tool: ToolIconName;
}

interface ToolSubIconProps {
  name: string;
}

interface ToolbarIconProps {
  name: ToolbarIconName;
}

const svgProps = {
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 2,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
  "aria-hidden": true,
  focusable: false,
};

export function ToolIcon({ tool }: ToolIconProps) {
  switch (tool) {
    case "select":
      return (
        <svg className="tool-icon" {...svgProps}>
          <path
            d="m5 3.5 7.3 16 2.1-6 5.6-2.3L5 3.5Z"
            fill="currentColor"
            fillOpacity=".16"
          />
          <path d="m14.3 13.8 4.2 4.4" />
          <path d="M17.5 3.5v3M16 5h3" strokeWidth="1.6" />
        </svg>
      );
    case "measure":
      return (
        <svg className="tool-icon" {...svgProps}>
          <path
            d="M4 18.5 18.5 4 21 6.5 6.5 21 4 18.5Z"
            fill="var(--color-pop-yellow-soft)"
          />
          <path d="m8 16.5-2-2m5-1-2-2m5-1-2-2m5-1-2-2" strokeWidth="1.5" />
          <path d="M4 10V4h6M4 10a6 6 0 0 1 6-6" />
        </svg>
      );
    case "mountain":
      return (
        <svg className="tool-icon tool-icon-mountain" {...svgProps}>
          <path
            d="M3.5 18.5 12 4l8.5 14.5h-17Z"
            fill="var(--color-on-solid, #fff)"
            fillOpacity=".92"
          />
          <path
            d="m5 16.2 3.6-5.3 3 3.9 3.3-5.4 4.1 6.8"
            stroke="var(--color-crease-mountain)"
            strokeWidth="2.2"
            strokeDasharray="6 2 1.2 2"
          />
          <path d="m6.2 18.2 5.4-3.4 6.2 3.4" strokeOpacity=".28" strokeWidth="1.4" />
        </svg>
      );
    case "valley":
      return (
        <svg className="tool-icon tool-icon-valley" {...svgProps}>
          <path
            d="M3.5 5.5h17L12 20 3.5 5.5Z"
            fill="var(--color-on-solid, #fff)"
            fillOpacity=".92"
          />
          <path
            d="m5.2 7.6 3.4 5.2 3-3.7 3.2 5.3 4-6.8"
            stroke="var(--color-crease-valley)"
            strokeWidth="2.2"
            strokeDasharray="3.4 2.4"
          />
          <path d="m6.2 5.8 5.4 3.3 6.2-3.3" strokeOpacity=".28" strokeWidth="1.4" />
        </svg>
      );
    case "aux":
      return (
        <svg className="tool-icon" {...svgProps}>
          <rect x="3" y="6" width="18" height="12" rx="3" fill="currentColor" fillOpacity=".12" />
          <path d="m5.5 16 13-8" strokeDasharray="2 2.5" />
          <path d="M7 8.6v2M11 7.4v2M15 6.8v2" strokeWidth="1.5" />
          <circle cx="5.5" cy="16" r="1" fill="currentColor" stroke="none" />
          <circle cx="18.5" cy="8" r="1" fill="currentColor" stroke="none" />
        </svg>
      );
    case "delete":
      return (
        <svg className="tool-icon" {...svgProps}>
          <path
            d="m4.5 14.5 7.8-8.4 7.2 6.7-5.2 5.5H8l-3.5-3.8Z"
            fill="var(--color-danger-soft)"
          />
          <path d="m8.4 10.4 6.8 6.3M4 20h14" />
          <path d="M18.5 5v3M17 6.5h3" stroke="var(--color-pop-coral)" strokeWidth="1.6" />
        </svg>
      );
    case "fold":
      return (
        <svg className="tool-icon" {...svgProps}>
          <path d="m3.5 7.2 8-3.7 2.8 8.2-7.8 3-3-7.5Z" fill="var(--color-secondary-soft)" />
          <path d="m14.3 11.7 6.2 2-7.2 7-6.8-6" fill="var(--color-pop-yellow-soft)" />
          <path d="m11.5 3.5 2.8 8.2-7.8 3" strokeDasharray="2 2" />
          <path d="M15.7 5.5c2.8.5 4.1 2 4 4.4M17.5 8.2l2.2 1.7 1.4-2.4" />
        </svg>
      );
    case "pull":
      return (
        <svg className="tool-icon" {...svgProps}>
          <path d="M3.5 6.5h10v11h-10z" fill="var(--color-secondary-soft)" />
          <path d="m9.5 6.5 4 4h-4v-4Z" fill="var(--color-pop-yellow)" />
          <path d="M10.5 12H21m-4-4 4 4-4 4" />
          <path d="M3.5 9H1.8M3.5 12H1.2M3.5 15H2" strokeWidth="1.5" />
        </svg>
      );
    case "construct":
      return (
        <svg className="tool-icon" {...svgProps}>
          <circle cx="12" cy="5" r="2.4" fill="var(--color-pop-yellow)" />
          <path d="m10.5 6.8-5.2 12M13.5 6.8l5.2 12M7.1 14h9.8" />
          <path d="M4 20h4M16 20h4" strokeWidth="2.4" />
          <circle cx="12" cy="5" r=".65" fill="currentColor" stroke="none" />
        </svg>
      );
    case "technique":
      return (
        <svg className="tool-icon" {...svgProps}>
          <path
            d="m12 3 8 5-3 10H7L4 8l8-5Z"
            fill="var(--color-on-solid, #fff)"
            fillOpacity=".9"
          />
          <path d="m4 8 8 4 8-4-3 10-5-6-5 6-3-10Z" fill="var(--color-secondary-soft)" />
          <path d="m4 8 8 4 8-4M12 3v9M12 12l-5 6M12 12l5 6" />
          <circle cx="12" cy="12" r="1.4" fill="var(--color-pop-coral)" stroke="none" />
        </svg>
      );
    case "fit":
      return (
        <svg className="tool-icon" {...svgProps}>
          <path d="m12 7 5 5-5 5-5-5 5-5Z" fill="var(--color-pop-yellow-soft)" />
          <path d="M9 3H3v6M15 3h6v6M21 15v6h-6M3 15v6h6" />
          <path d="m8.5 8.5-3-3M15.5 8.5l3-3M8.5 15.5l-3 3M15.5 15.5l3 3" strokeWidth="1.5" />
        </svg>
      );
  }
}

export function ToolSubIcon({ name }: ToolSubIconProps) {
  if (name === "bisector") {
    return (
      <svg className="tool-sub-icon" {...svgProps}>
        <path d="M4 19 20 5M4 6h8M8 3l4 3-4 3" />
      </svg>
    );
  }
  if (name === "perpendicular") {
    return (
      <svg className="tool-sub-icon" {...svgProps}>
        <path d="M4 19 19 4M7 16l3 3-3 3M12 11l3 3 3-3" />
      </svg>
    );
  }
  if (name === "divide") {
    return (
      <svg className="tool-sub-icon" {...svgProps}>
        <path d="M3 12h18M7 8v8M12 8v8M17 8v8" />
        <circle cx="7" cy="12" r="1" fill="currentColor" stroke="none" />
        <circle cx="12" cy="12" r="1" fill="currentColor" stroke="none" />
        <circle cx="17" cy="12" r="1" fill="currentColor" stroke="none" />
      </svg>
    );
  }
  if (name === "angle") {
    return (
      <svg className="tool-sub-icon" {...svgProps}>
        <path d="M5 19V5h14M8 16A8 8 0 0 1 16 8" />
        <path d="m14 6 2 2-2 2" />
      </svg>
    );
  }
  if (name === "Pleat") {
    return (
      <svg className="tool-sub-icon" {...svgProps}>
        <path d="M4 7h16" stroke="var(--color-crease-mountain)" strokeDasharray="5 2 1 2" />
        <path d="M4 16h16" stroke="var(--color-crease-valley)" strokeDasharray="3 2" />
        <path d="m7 10 2 2-2 2M17 14l-2-2 2-2" strokeWidth="1.5" />
      </svg>
    );
  }
  if (name === "InsideReverse") {
    return (
      <svg className="tool-sub-icon" {...svgProps}>
        <path d="m4 18 8-13 8 13-8-4-8 4Z" fill="currentColor" fillOpacity=".1" />
        <path d="M12 7v7m-3-3 3 3 3-3" />
      </svg>
    );
  }
  if (name === "OutsideReverse") {
    return (
      <svg className="tool-sub-icon" {...svgProps}>
        <path d="m4 18 8-13 8 13-8-4-8 4Z" fill="currentColor" fillOpacity=".1" />
        <path d="M12 15V8m-3 3 3-3 3 3" />
      </svg>
    );
  }
  if (name === "Squash") {
    return (
      <svg className="tool-sub-icon" {...svgProps}>
        <path d="m12 4 6 8-6 8-6-8 6-8Z" fill="currentColor" fillOpacity=".1" />
        <path d="M12 12H3m3-3-3 3 3 3M12 12h9m-3-3 3 3-3 3" />
      </svg>
    );
  }
  if (name === "Petal") {
    return (
      <svg className="tool-sub-icon" {...svgProps}>
        <path d="M12 20C5 16 5 9 12 4c7 5 7 12 0 16Z" fill="currentColor" fillOpacity=".1" />
        <path d="M12 18V7m-3 3 3-3 3 3" />
      </svg>
    );
  }
  if (name === "OpenSink") {
    return (
      <svg className="tool-sub-icon" {...svgProps}>
        <path d="m12 4 8 6-3 9H7l-3-9 8-6Z" fill="currentColor" fillOpacity=".1" />
        <path d="M12 6v9m-3-3 3 3 3-3" />
      </svg>
    );
  }
  if (name === "Swivel") {
    return (
      <svg className="tool-sub-icon" {...svgProps}>
        <circle cx="7" cy="17" r="2" fill="var(--color-pop-yellow)" />
        <path d="M7 15c1-7 7-9 12-5M16 6l3 4-5 1" />
        <path d="m8.5 15.5 7-7" strokeDasharray="2 2" />
      </svg>
    );
  }
  if (name === "Twist") {
    return (
      <svg className="tool-sub-icon" {...svgProps}>
        <path d="m12 6 5 3v6l-5 3-5-3V9l5-3Z" fill="currentColor" fillOpacity=".1" />
        <path d="M5 7a9 9 0 0 1 14 1M17 4l2 4-4 1M19 17A9 9 0 0 1 5 16M7 20l-2-4 4-1" />
      </svg>
    );
  }
  return (
    <svg className="tool-sub-icon" {...svgProps}>
      <path d="m12 3 7 5-3 10H8L5 8l7-5ZM5 8l7 4 7-4" />
    </svg>
  );
}

export function ToolbarIcon({ name }: ToolbarIconProps) {
  switch (name) {
    case "new":
      return (
        <svg className="toolbar-icon" {...svgProps}>
          <path d="M5 3h10l4 4v14H5V3Z" fill="currentColor" fillOpacity=".1" />
          <path d="M15 3v5h4M12 11v6M9 14h6" />
        </svg>
      );
    case "open":
      return (
        <svg className="toolbar-icon" {...svgProps}>
          <path d="M3 7h7l2 2h9l-2 10H5L3 7Z" fill="currentColor" fillOpacity=".12" />
          <path d="M5 7V4h6l2 3M7 13h9" />
        </svg>
      );
    case "save":
      return (
        <svg className="toolbar-icon" {...svgProps}>
          <path d="M4 4h14l2 2v14H4V4Z" fill="currentColor" fillOpacity=".1" />
          <path d="M8 4v6h8V4M8 20v-6h8v6" />
          <path d="m10 17 1.5 1.5L15 15" strokeWidth="1.6" />
        </svg>
      );
    case "undo":
      return (
        <svg className="toolbar-icon" {...svgProps}>
          <path d="m9 6-5 5 5 5M5 11h8a6 6 0 0 1 6 6" />
          <path d="M4 6v5h5" fill="currentColor" fillOpacity=".12" />
        </svg>
      );
    case "redo":
      return (
        <svg className="toolbar-icon" {...svgProps}>
          <path d="m15 6 5 5-5 5M19 11h-8a6 6 0 0 0-6 6" />
          <path d="M20 6v5h-5" fill="currentColor" fillOpacity=".12" />
        </svg>
      );
    case "proposal":
      return (
        <svg className="toolbar-icon" {...svgProps}>
          <path d="m5 19 10-10 3 3L8 22l-3-3Z" fill="currentColor" fillOpacity=".12" />
          <path d="m14 4 1-2 1 2 2 1-2 1-1 2-1-2-2-1 2-1ZM5 8l.8-1.7L7 8l1.7.8L7 10l-1.2 1.7L5 10l-1.7-1.2L5 8Z" strokeWidth="1.5" />
        </svg>
      );
    case "export":
      return (
        <svg className="toolbar-icon" {...svgProps}>
          <rect x="3" y="5" width="13" height="14" rx="2" fill="currentColor" fillOpacity=".1" />
          <path d="m5.5 16 3.5-4 2.5 3 2-2 2.5 3M17 8h4m-2-2 2 2-2 2" />
          <circle cx="7.5" cy="9" r="1.2" fill="currentColor" stroke="none" />
        </svg>
      );
    case "help":
      return (
        <svg className="toolbar-icon" {...svgProps}>
          <circle cx="12" cy="12" r="9" fill="var(--color-pop-yellow-soft)" />
          <path d="M9.7 9a2.5 2.5 0 0 1 4.8 1c0 2-2.5 2-2.5 4" />
          <circle cx="12" cy="17.5" r="1" fill="currentColor" stroke="none" />
        </svg>
      );
  }
}
