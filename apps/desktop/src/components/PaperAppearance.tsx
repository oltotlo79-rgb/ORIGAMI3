// 紙の色(PAP-003)・方眼の分割数(CPE-003)・左右対称に描くか(CPE-010)の指定。
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
  const mirrorDraw = useAppStore((s) => s.mirrorDraw);
  const setMirrorDraw = useAppStore((s) => s.setMirrorDraw);

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
      {/* 左右対称に描く(CPE-010)。作品は左右対称のものが多いので、片側を
          描くと反対側にも同じ線が引かれ、作業が半分で済む */}
      <label>
        <input
          type="checkbox"
          aria-label="左右対称に描く"
          checked={mirrorDraw}
          onChange={(e) => setMirrorDraw(e.target.checked)}
        />
        左右対称に描く
      </label>
      <span className="hint">
        {mirrorDraw
          ? "左右対称に描いています。紙の縦の中心線をはさんで、反対側にも同じ線が引かれます(2Dの薄い縦線が中心線です)"
          : "紙の縦の中心線をはさんで、反対側にも同じ線を引きます"}
        。線を引くときだけ効きます(線を消す・種類を変えるときは片側ずつです)
      </span>
    </div>
  );
}
