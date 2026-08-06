// 紙の色(PAP-003)・方眼の分割数(CPE-003)・左右対称に描くか(CPE-010)の指定。
// 何も選んでいないときのコンテキストパネルに出すだけで、常設の区画は増やさない。
// 変えた結果は展開図・立体表示にその場で映る(設計原則3b)。
//
// 紙の色と方眼は作品ごとの設定として保存される(setDisplay が edit_apply の
// EditOp::SetDisplay を送る)。.ori3ファイルに入るので、作品を渡した相手にも
// 同じ色・同じ方眼で見え、元に戻す/やり直しも効く。
// 左右対称に描くかは作品の中身ではなく描き方の好みなので端末側に覚える。
//
// 紙のたわみ(SIM-012/013/015)もここに置く。硬さ・膨らみの強さは
// 「パラメータだけを残して頂点の位置は保存しない」決まりなので、紙の色と同じく
// DisplaySettings に入れて .ori3ファイルへ保存する。

import { useAppStore } from "../store/appStore";
import {
  MAX_DIVISIONS,
  MIN_DIVISIONS,
  hexToRgb,
  rgbToHex,
  softOf,
} from "../lib/displayPrefs";

export function PaperAppearance() {
  const display = useAppStore((s) => s.display);
  const setDisplay = useAppStore((s) => s.setDisplay);
  const setSoft = useAppStore((s) => s.setSoft);
  const softWarnings = useAppStore((s) => s.softWarnings);
  const mirrorDraw = useAppStore((s) => s.mirrorDraw);
  const setMirrorDraw = useAppStore((s) => s.setMirrorDraw);
  const soft = softOf(display);

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
      {/* 紙のたわみ(SIM-012 / SIM-013)。既定はオフで、入れると折り目以外の
          ところでも紙が丸く曲がった形になる。膨らみは動かすとすぐ3Dに映る */}
      <label>
        <input
          type="checkbox"
          aria-label="紙のたわみを表現する"
          checked={soft.enabled}
          onChange={(e) => setSoft({ soft_enabled: e.target.checked })}
        />
        紙のたわみを表現する
      </label>
      {soft.enabled && (
        <>
          <label>
            紙の硬さ
            <input
              type="range"
              aria-label="紙の硬さ"
              min={0}
              max={1}
              step={0.05}
              value={soft.stiffness}
              onChange={(e) => setSoft({ soft_stiffness: Number(e.target.value) })}
            />
          </label>
          <label>
            膨らみの強さ
            <input
              type="range"
              aria-label="膨らみの強さ"
              min={0}
              max={1}
              step={0.05}
              value={soft.pressure}
              onChange={(e) => setSoft({ soft_pressure: Number(e.target.value) })}
            />
          </label>
        </>
      )}
      <span className="hint">
        {soft.enabled
          ? "面を細かく分けて曲げ、紙の丸みを見せています。硬くすると面が平らに近づき、膨らませると袋になっているところに空気が入ります(動かすとその場で3Dに映ります)"
          : "折り目以外のところでも紙が丸く曲がった形を見せます(見た目だけの表現で、折り手順や折り図は変わりません)"}
      </span>
      {softWarnings.map((w) => (
        <span className="hint" key={w}>
          {w}
        </span>
      ))}
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
        。線を消すとき・種類を変えるときにも効き、そちらは展開図から見つけた対称軸で相手の線を探します(対になる線が無いところは、その線だけが変わります)
      </span>
    </div>
  );
}
