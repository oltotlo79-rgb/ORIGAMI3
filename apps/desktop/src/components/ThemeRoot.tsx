import { useLayoutEffect, useRef, type CSSProperties, type ReactNode } from "react";
import { useAppStore } from "../store/appStore";

const PORTAL_THEME_TOKENS = [
  ["--color-tooltip-background", "--color-overlay"],
  ["--color-tooltip-border", "--floating-border-color"],
  ["--color-tooltip-text", "--color-on-solid"],
  ["--shadow-tooltip", "--shadow-2"],
  ["--radius-tooltip", "--radius-sm"],
  ["--tooltip-font-weight", "--fw-medium"],
  ["--tooltip-font-family", "--font-body"],
  ["--tooltip-border-width", "--control-border-width"],
  ["--tooltip-padding-inline", "--sp-5"],
] as const;

/** POPは属性なしを既定とし、追加テーマだけをCSS変数の切替属性へ写す。 */
export function ThemeRoot({ children }: { children: ReactNode }) {
  const appRef = useRef<HTMLDivElement>(null);
  const uiTheme = useAppStore((s) => s.uiTheme);
  const contextPanelRatio = useAppStore((s) => s.contextPanelRatio);
  /* TooltipHostはbodyへportalする。現在テーマの実効値だけをdocument rootへ橋渡しし、
     app外へ出る吹き出しも5テーマの角丸・色・影を継承できるようにする。 */
  useLayoutEffect(() => {
    const app = appRef.current;
    if (!app) return;
    const rootStyle = document.documentElement.style;
    const computed = window.getComputedStyle(app);
    const previous = PORTAL_THEME_TOKENS.map(([target]) => [
      target,
      rootStyle.getPropertyValue(target),
      rootStyle.getPropertyPriority(target),
    ] as const);

    for (const [target, source] of PORTAL_THEME_TOKENS) {
      const value = computed.getPropertyValue(source).trim();
      if (value) rootStyle.setProperty(target, value);
    }
    return () => {
      for (const [target, value, priority] of previous) {
        if (value) rootStyle.setProperty(target, value, priority);
        else rootStyle.removeProperty(target);
      }
    };
  }, [uiTheme]);
  // grid-template-rows自体をインライン指定すると解説画像の撮影モードを
  // 上書きしてしまうため、通常レイアウトが読むfr値だけを渡す。
  const layoutStyle = {
    "--main-row-share": `${Number((1 - contextPanelRatio).toFixed(4))}fr`,
    "--context-panel-share": `${Number(contextPanelRatio.toFixed(4))}fr`,
  } as CSSProperties;
  return (
    <div
      ref={appRef}
      className="app"
      data-theme={uiTheme === "pop" ? undefined : uiTheme}
      style={layoutStyle}
    >
      {children}
    </div>
  );
}
