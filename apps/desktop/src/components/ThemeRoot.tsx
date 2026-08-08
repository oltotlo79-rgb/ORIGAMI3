import type { ReactNode } from "react";
import { useAppStore } from "../store/appStore";

/** POPは属性なしを既定とし、追加テーマだけをCSS変数の切替属性へ写す。 */
export function ThemeRoot({ children }: { children: ReactNode }) {
  const uiTheme = useAppStore((s) => s.uiTheme);
  return (
    <div className="app" data-theme={uiTheme === "pop" ? undefined : uiTheme}>
      {children}
    </div>
  );
}
