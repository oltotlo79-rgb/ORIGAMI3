// 紙の色(PAP-003)・方眼の分割数(CPE-003)・描画操作の好みの指定。
// 何も選んでいないときのコンテキストパネルに出すだけで、常設の区画は増やさない。
// 変えた結果は展開図・立体表示にその場で映る(設計原則3b)。
//
// 紙の色と方眼は作品ごとの設定として保存される(setDisplay が edit_apply の
// EditOp::SetDisplay を送る)。.ori3ファイルに入るので、作品を渡した相手にも
// 同じ色・同じ方眼で見え、元に戻す/やり直しも効く。
// 左右対称に描くか・ホイールの役割は作品の中身ではなく操作の好みなので端末側に覚える。
//
// 紙のたわみ(SIM-012/013/015)もここに置く。硬さ・膨らみの強さは
// 「パラメータだけを残して頂点の位置は保存しない」決まりなので、紙の色と同じく
// DisplaySettings に入れて .ori3ファイルへ保存する。

import { useAppStore } from "../store/appStore";
import {
  MAX_DIVISIONS,
  MIN_DIVISIONS,
  UI_THEMES,
  hexToRgb,
  overlapPreventionOf,
  rgbToHex,
  softOf,
  type UiTheme,
} from "../lib/displayPrefs";

const UI_THEME_LABELS: Record<UiTheme, string> = {
  pop: "ポップ",
  simple: "シンプル",
  japanese: "和風",
  modern: "モダン",
  classic: "クラシック",
};

/**
 * 子供向けの折り紙セットで親しまれている色を中心にした固定パレット。
 * 色名はボタンのtitleにも使い、色だけに頼らず選べるようにする。
 */
export const PAPER_COLOR_PALETTE = [
  { name: "赤", hex: "#ed1c24" },
  { name: "朱", hex: "#f4511e" },
  { name: "桃", hex: "#f06292" },
  { name: "桜", hex: "#f8bbd0" },
  { name: "橙", hex: "#ff8c00" },
  { name: "山吹", hex: "#f6b900" },
  { name: "黄", hex: "#ffd84d" },
  { name: "レモン", hex: "#fff176" },
  { name: "黄緑", hex: "#8bc34a" },
  { name: "緑", hex: "#20a162" },
  { name: "深緑", hex: "#006b4f" },
  { name: "水色", hex: "#4fc3f7" },
  { name: "空色", hex: "#29b6f6" },
  { name: "青", hex: "#3578e5" },
  { name: "紺", hex: "#243b78" },
  { name: "紫", hex: "#7040c9" },
  { name: "藤", hex: "#b39ddb" },
  { name: "茶", hex: "#8d5a3b" },
  { name: "肌色", hex: "#f4c7a1" },
  { name: "金茶", hex: "#c88a16" },
  { name: "銀鼠", hex: "#a7a9ac" },
  { name: "白", hex: "#ffffff" },
  { name: "灰", hex: "#777777" },
  { name: "黒", hex: "#1f1f1f" },
] as const;

/** よく使う方眼。任意入力への近道であり、これ以外の数も指定できる。 */
export const GRID_DIVISION_PRESETS = [4, 8, 12, 16, 24, 32] as const;

function paletteMarkColor(hex: string): "#ffffff" | "#27213d" {
  const rgb = hexToRgb(hex);
  if (!rgb) return "#27213d";
  // sRGBの簡易輝度。小さなチェック印がどの見本でも読める側を選ぶ。
  const luminance = rgb[0] * 0.299 + rgb[1] * 0.587 + rgb[2] * 0.114;
  return luminance < 145 ? "#ffffff" : "#27213d";
}

function ColorPalette({
  label,
  pickerLabel,
  value,
  onSelect,
}: {
  label: string;
  pickerLabel: string;
  value: string;
  onSelect: (hex: string) => void;
}) {
  const normalizedValue = value.toLowerCase();

  return (
    <fieldset className="paper-color-palette">
      <legend>{label}</legend>
      <div className="paper-color-swatches" role="group" aria-label={`${label}の24色パレット`}>
        {PAPER_COLOR_PALETTE.map((swatch) => {
          const selected = normalizedValue === swatch.hex;
          return (
            <button
              key={swatch.name}
              type="button"
              className="paper-color-swatch"
              style={{
                backgroundColor: swatch.hex,
                color: paletteMarkColor(swatch.hex),
              }}
              title={swatch.name}
              aria-label={`${label}を${swatch.name}にする`}
              aria-pressed={selected}
              onClick={() => onSelect(swatch.hex)}
            >
              <span aria-hidden="true">{selected ? "✓" : ""}</span>
            </button>
          );
        })}
      </div>
      <label className="paper-custom-color">
        その他の色
        <input
          type="color"
          aria-label={pickerLabel}
          value={value}
          onChange={(e) => onSelect(e.target.value)}
        />
      </label>
    </fieldset>
  );
}

export function PaperAppearance() {
  const display = useAppStore((s) => s.display);
  const setDisplay = useAppStore((s) => s.setDisplay);
  const setSoft = useAppStore((s) => s.setSoft);
  const softWarnings = useAppStore((s) => s.softWarnings);
  const mirrorDraw = useAppStore((s) => s.mirrorDraw);
  const setMirrorDraw = useAppStore((s) => s.setMirrorDraw);
  const wheelBehavior = useAppStore((s) => s.wheelBehavior);
  const setWheelBehavior = useAppStore((s) => s.setWheelBehavior);
  const uiTheme = useAppStore((s) => s.uiTheme);
  const setUiTheme = useAppStore((s) => s.setUiTheme);
  const soft = softOf(display);
  const overlapPrevention = overlapPreventionOf(display);

  return (
    <div className="paper-appearance">
      {/* 「膨らます」への入口は、色や方眼より先に見える位置へ置く。
          入れると折り目以外も丸く曲がり、つまみの結果はその場で3Dへ映る。 */}
      <section className="soft-controls" aria-label="紙をふくらませる設定">
        <div className="soft-controls-heading">
          <strong>紙をふくらませる</strong>
          <span>袋になったところへ空気を入れるように丸みをつけます</span>
        </div>
        <label>
          <input
            type="checkbox"
            aria-label="紙のたわみを表現する"
            checked={soft.enabled}
            onChange={(e) => setSoft({ soft_enabled: e.target.checked })}
          />
          丸みをつける
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
      </section>
      <label>
        ホイールの動作
        <select
          aria-label="ホイールの動作"
          value={wheelBehavior}
          onChange={(e) => setWheelBehavior(e.target.value === "zoom" ? "zoom" : "scroll")}
        >
          <option value="scroll">スクロール</option>
          <option value="zoom">拡大縮小</option>
        </select>
      </label>
      <span className="hint">
        {wheelBehavior === "scroll"
          ? "ホイール: 上下 / Shift+ホイール: 左右 / Ctrl+ホイール: カーソル位置を中心に拡大縮小"
          : "ホイール: カーソル位置を中心に拡大縮小 / Ctrl+ホイール: 上下 / Ctrl+Shift+ホイール: 左右"}
      </span>
      <label>
        画面のデザイン
        <select
          aria-label="画面のデザイン"
          value={uiTheme}
          onChange={(e) => setUiTheme(e.target.value as UiTheme)}
        >
          {UI_THEMES.map((theme) => (
            <option key={theme} value={theme}>
              {UI_THEME_LABELS[theme]}
            </option>
          ))}
        </select>
      </label>
      <div className="paper-color-settings">
        <ColorPalette
          label="紙の表"
          pickerLabel="紙の表の色"
          value={rgbToHex(display.front_color)}
          onSelect={(hex) => {
            const rgb = hexToRgb(hex);
            if (rgb) setDisplay({ front_color: rgb });
          }}
        />
        <ColorPalette
          label="紙の裏"
          pickerLabel="紙の裏の色"
          value={rgbToHex(display.back_color)}
          onSelect={(hex) => {
            const rgb = hexToRgb(hex);
            if (rgb) setDisplay({ back_color: rgb });
          }}
        />
      </div>
      <section className="grid-divisions-control" aria-labelledby="grid-divisions-heading">
        <div className="grid-divisions-heading">
          <span id="grid-divisions-heading">方眼の細かさ</span>
          <output aria-live="polite">
            {display.grid_divisions}
            <small>等分</small>
          </output>
        </div>
        <div className="grid-division-presets" role="group" aria-label="よく使う方眼の細かさ">
          {GRID_DIVISION_PRESETS.map((divisions) => (
            <button
              key={divisions}
              type="button"
              aria-pressed={display.grid_divisions === divisions}
              onClick={() => setDisplay({ grid_divisions: divisions })}
            >
              {divisions}
            </button>
          ))}
        </div>
        <label className="grid-division-custom">
          自由に指定
          <input
            type="number"
            aria-label="方眼の細かさ（1辺の等分数）"
            min={MIN_DIVISIONS}
            max={MAX_DIVISIONS}
            step={1}
            value={display.grid_divisions}
            onChange={(e) => setDisplay({ grid_divisions: Number(e.target.value) })}
          />
        </label>
        <span className="hint">
          紙を{display.grid_divisions}等分した目盛りに線が吸い付きます（{MIN_DIVISIONS}〜
          {MAX_DIVISIONS}）
        </span>
      </section>
      <label>
        <input
          type="checkbox"
          aria-label="重なり防止"
          checked={overlapPrevention}
          onChange={(e) => setDisplay({ overlap_prevention_enabled: e.target.checked })}
        />
        重なり防止
      </label>
      <span className="hint">
        折っている途中で紙どうしが突き抜けにくいよう補正します(完全には防げません)
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
        。線を消すとき・種類を変えるときにも効き、そちらは展開図から見つけた対称軸で相手の線を探します(対になる線が無いところは、その線だけが変わります)
      </span>
    </div>
  );
}
