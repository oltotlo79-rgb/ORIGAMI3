// 紙の色(PAP-003)と方眼の分割数(CPE-003)の指定。
// 何も選んでいないときのコンテキストパネルに出すだけで、常設の区画は増やさない。
// 変えた結果は展開図・立体表示にその場で映る(設計原則3b)。

import { useAppStore } from "../store/appStore";
import {
  MAX_DIVISIONS,
  MIN_DIVISIONS,
  hexToRgb,
  rgbToHex,
} from "../lib/displayPrefs";

export function PaperAppearance() {
  const display = useAppStore((s) => s.display);
  const setDisplay = useAppStore((s) => s.setDisplay);

  return (
    <div className="paper-appearance">
      <label>
        紙の表の色
        <input
          type="color"
          aria-label="紙の表の色"
          value={rgbToHex(display.front_color)}
          onChange={(e) => {
            const rgb = hexToRgb(e.target.value);
            if (rgb) setDisplay({ front_color: rgb });
          }}
        />
      </label>
      <label>
        紙の裏の色
        <input
          type="color"
          aria-label="紙の裏の色"
          value={rgbToHex(display.back_color)}
          onChange={(e) => {
            const rgb = hexToRgb(e.target.value);
            if (rgb) setDisplay({ back_color: rgb });
          }}
        />
      </label>
      <label>
        方眼の数(1辺)
        <input
          type="number"
          min={MIN_DIVISIONS}
          max={MAX_DIVISIONS}
          step={1}
          value={display.grid_divisions}
          onChange={(e) => setDisplay({ grid_divisions: Number(e.target.value) })}
        />
      </label>
      <span className="hint">
        紙を{display.grid_divisions}等分した目盛りに線が吸い付きます({MIN_DIVISIONS}〜
        {MAX_DIVISIONS})
      </span>
    </div>
  );
}
