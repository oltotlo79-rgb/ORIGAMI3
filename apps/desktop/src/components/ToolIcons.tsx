import type { ToolId } from "../store/appStore";

type ToolIconName = ToolId | "fit";

interface ToolIconProps {
  tool: ToolIconName;
}

interface ToolSubIconProps {
  name: string;
}

const svgProps = {
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.8,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
  "aria-hidden": true,
  focusable: false,
};

export function ToolIcon({ tool }: ToolIconProps) {
  switch (tool) {
    case "select":
      return <svg className="tool-icon" {...svgProps}><path d="m5 3 7.4 16 2.2-6.1L20 10.6 5 3Z" /><path d="m14.2 14.2 4.1 4.1" /></svg>;
    case "mountain":
      return <svg className="tool-icon tool-icon-mountain" {...svgProps}><path d="m3 16 5.2-7 3.7 5 2.7-3.7L21 16" stroke="var(--color-crease-mountain)" strokeDasharray="7 2 1.5 2" /><path d="M3 19h18" stroke="currentColor" strokeOpacity=".35" /></svg>;
    case "valley":
      return <svg className="tool-icon tool-icon-valley" {...svgProps}><path d="m3 8 5.2 7 3.7-5 2.7 3.7L21 8" stroke="var(--color-crease-valley)" strokeDasharray="4 3" /><path d="M3 5h18" stroke="currentColor" strokeOpacity=".35" /></svg>;
    case "aux":
      return <svg className="tool-icon" {...svgProps}><path d="M4 7h16M4 17h16" strokeDasharray="2 3" /><path d="m7 19 10-14" strokeWidth="1.3" /><circle cx="7" cy="19" r="1" fill="currentColor" stroke="none" /><circle cx="17" cy="5" r="1" fill="currentColor" stroke="none" /></svg>;
    case "delete":
      return <svg className="tool-icon" {...svgProps}><path d="m5 14 7-8 7 7-5 5H8l-3-4Z" /><path d="m14.5 8.5 2.5-2.5M5 20 20 5" /></svg>;
    case "fold":
      return <svg className="tool-icon" {...svgProps}><path d="m4 6 7-3 3 8-7 3L4 6Z" /><path d="m14 11 6 2-7 8-6-7" /><path d="m11 3 3 8-7 3" strokeDasharray="2 2" /><path d="m16 5 3 3-4 .6" /></svg>;
    case "pull":
      return <svg className="tool-icon" {...svgProps}><path d="M4 12h12" /><path d="m13 7 5 5-5 5" /><path d="M7 8.5v7M9.5 9.5v5M4.5 10.5v3" /></svg>;
    case "construct":
      return <svg className="tool-icon" {...svgProps}><circle cx="12" cy="5" r="2" /><path d="m10.5 6.4-5 12.1M13.5 6.4l5 12.1M7.3 14h9.4M6 20h12" /></svg>;
    case "technique":
      return <svg className="tool-icon" {...svgProps}><path d="m12 3 8 5-3 10H7L4 8l8-5Z" /><path d="m4 8 8 4 8-4M12 12v6M12 3v9" /></svg>;
    case "fit":
      return <svg className="tool-icon" {...svgProps}><path d="M9 4H4v5M15 4h5v5M20 15v5h-5M4 15v5h5" /><path d="m10 10-3-3M14 10l3-3M10 14l-3 3M14 14l3 3" /></svg>;
  }
}

export function ToolSubIcon({ name }: ToolSubIconProps) {
  if (name === "bisector") return <svg className="tool-sub-icon" {...svgProps}><path d="m4 18 16-12M4 6h8M12 6l-2-2m2 2-2 2" /></svg>;
  if (name === "perpendicular") return <svg className="tool-sub-icon" {...svgProps}><path d="M4 18 18 4M7 15l3 3-3 3M12 10l3 3 3-3" /></svg>;
  if (name === "divide") return <svg className="tool-sub-icon" {...svgProps}><path d="M4 12h16M8 8v8M12 8v8M16 8v8" /></svg>;
  if (name === "angle") return <svg className="tool-sub-icon" {...svgProps}><path d="M5 18V6h12M8 15a7 7 0 0 1 7-7" /></svg>;
  return <svg className="tool-sub-icon" {...svgProps}><path d="m12 3 7 5-3 10H8L5 8l7-5ZM5 8l7 4 7-4" /></svg>;
}
