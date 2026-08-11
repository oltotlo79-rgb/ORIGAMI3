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
import { ColorPickerPopover } from "./ColorPickerPopover";
import { MirrorAxisControls } from "./MirrorAxisControls";
import {
  MAX_DIVISIONS,
  MIN_DIVISIONS,
  UI_THEMES,
  hexToRgb,
  overlapPreventionOf,
  penetrationPreventionOf,
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
 * 色名は共通の吹き出しにも使い、色だけに頼らず選べるようにする。
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
export const GRID_DIVISION_PRESETS = [4, 8, 12, 16, 24, 32, 64, 128, 256] as const;

function paletteMarkColor(hex: string): "#ffffff" | "#27213d" {
  const rgb = hexToRgb(hex);
  if (!rgb) return "#27213d";
  // sRGBの簡易輝度。小さなチェック印がどの見本でも読める側を選ぶ。
  const luminance = rgb[0] * 0.299 + rgb[1] * 0.587 + rgb[2] * 0.114;
  return luminance < 145 ? "#ffffff" : "#27213d";
}

function ColorPalette({
  label,
  value,
  onSelect,
}: {
  label: string;
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
              data-tooltip={`${swatch.name}を選びます`}
              aria-label={`${label}を${swatch.name}にする`}
              aria-pressed={selected}
              onClick={() => onSelect(swatch.hex)}
            >
              <span aria-hidden="true">{selected ? "✓" : ""}</span>
            </button>
          );
        })}
      </div>
      <ColorPickerPopover label={label} value={value} onSelect={onSelect} />
    </fieldset>
  );
}

export function PaperAppearance() {
  const display = useAppStore((s) => s.display);
  const setDisplay = useAppStore((s) => s.setDisplay);
  const setSoft = useAppStore((s) => s.setSoft);
  const softWarnings = useAppStore((s) => s.softWarnings);
  const wheelBehavior = useAppStore((s) => s.wheelBehavior);
  const setWheelBehavior = useAppStore((s) => s.setWheelBehavior);
  const uiTheme = useAppStore((s) => s.uiTheme);
  const setUiTheme = useAppStore((s) => s.setUiTheme);
  const resetPaneSizes = useAppStore((s) => s.resetPaneSizes);
  const paperHelpExpanded = useAppStore((s) => s.paperHelpExpanded);
  const togglePaperHelp = useAppStore((s) => s.togglePaperHelp);
  const paperColorExpanded = useAppStore((s) => s.paperColorExpanded);
  const togglePaperColor = useAppStore((s) => s.togglePaperColor);
  const soft = softOf(display);
  const overlapPrevention = overlapPreventionOf(display);
  const penetrationPrevention = penetrationPreventionOf(display);

  return (
    <div className="paper-appearance">
      {/* 「膨らます」への入口は、色や方眼より先に見える位置へ置く。
          入れると折り目以外も丸く曲がり、つまみの結果はその場で3Dへ映る。 */}
      <section className="soft-controls" aria-label="紙をふくらませる設定">
        <div className="soft-controls-heading">
          <strong>紙をふくらませる</strong>
          <span
            className="operation-summary-line"
            data-tooltip="丸みと膨らみを3Dで調整できます"
          >
            丸みと膨らみを3Dで調整できます
          </span>
          <button
            type="button"
            className="paper-help-toggle operation-detail-toggle"
            aria-label={`丸みの詳しい操作方法 ${paperHelpExpanded ? "▲" : "▼"}`}
            aria-expanded={paperHelpExpanded}
            data-tooltip={paperHelpExpanded ? "丸みの説明を折りたたみます" : "丸みの説明を開きます"}
            onClick={togglePaperHelp}
          >
            <span className="operation-detail-label">丸みの詳しい操作方法</span>
            <span className="operation-detail-icon">
              {paperHelpExpanded ? "▲" : "▼"}
            </span>
          </button>
        </div>
        <label>
          <input
            type="checkbox"
            aria-label="紙のたわみを表現する"
            data-tooltip="折り目以外も滑らかに曲がる表示を切り替えます"
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
                data-tooltip="紙の面を平らに保つ強さを調整します"
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
                data-tooltip="袋へ空気を入れたような膨らみを調整します"
                min={0}
                max={1}
                step={0.05}
                value={soft.pressure}
                onChange={(e) => setSoft({ soft_pressure: Number(e.target.value) })}
              />
            </label>
          </>
        )}
        {paperHelpExpanded && (
          <span className="hint soft-help-detail">
            {soft.enabled
              ? "硬さで紙の曲がりやすさを、膨らみの強さで袋へ入れる空気の量を調整します。変更はその場で3Dに映ります。"
              : "折り目以外にも丸みを見せる表示です。見た目だけが変わり、折り手順や折り図は変わりません。"}
          </span>
        )}
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
          data-tooltip={
            wheelBehavior === "scroll"
              ? "ホイールで上下、Shiftで左右、Ctrlで拡大縮小します"
              : "ホイールで拡大縮小、Ctrlで上下、Ctrl+Shiftで左右へ動かします"
          }
          value={wheelBehavior}
          onChange={(e) => setWheelBehavior(e.target.value === "zoom" ? "zoom" : "scroll")}
        >
          <option value="scroll">スクロール</option>
          <option value="zoom">拡大縮小</option>
        </select>
      </label>
      <label>
        画面のデザイン
        <select
          aria-label="画面のデザイン"
          data-tooltip="画面全体の配色と形を切り替えます"
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
      <button
        type="button"
        data-tooltip="展開図・立体・今できる操作の広さを初期値へ戻します"
        onClick={resetPaneSizes}
      >
        表示の広さを初期に戻す
      </button>
      <section className="paper-color-section" aria-label="紙の色の設定">
        <button
          type="button"
          className="paper-color-toggle"
          aria-label={`紙の色 ${paperColorExpanded ? "▲" : "▼"}`}
          aria-expanded={paperColorExpanded}
          aria-controls="paper-color-settings"
          data-tooltip={paperColorExpanded ? "紙の色見本を折りたたみます" : "紙の色見本を開きます"}
          onClick={togglePaperColor}
        >
          <span>紙の色 {paperColorExpanded ? "▲" : "▼"}</span>
          <span className="paper-color-current" aria-hidden="true">
            <span>
              表
              <i style={{ backgroundColor: rgbToHex(display.front_color) }} />
            </span>
            <span>
              裏
              <i style={{ backgroundColor: rgbToHex(display.back_color) }} />
            </span>
          </span>
        </button>
        <span className="sr-only" aria-live="polite">
          紙の表の現在色 {rgbToHex(display.front_color)}、紙の裏の現在色 {rgbToHex(display.back_color)}
        </span>
        {paperColorExpanded && (
          <div id="paper-color-settings" className="paper-color-settings">
            <ColorPalette
              label="紙の表"
              value={rgbToHex(display.front_color)}
              onSelect={(hex) => {
                const rgb = hexToRgb(hex);
                if (rgb) setDisplay({ front_color: rgb });
              }}
            />
            <ColorPalette
              label="紙の裏"
              value={rgbToHex(display.back_color)}
              onSelect={(hex) => {
                const rgb = hexToRgb(hex);
                if (rgb) setDisplay({ back_color: rgb });
              }}
            />
          </div>
        )}
      </section>
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
              data-tooltip={`方眼を${divisions}等分にします`}
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
            data-tooltip="1辺を何等分する方眼にするか指定します"
            min={MIN_DIVISIONS}
            max={MAX_DIVISIONS}
            step={1}
            value={display.grid_divisions}
            onChange={(e) => setDisplay({ grid_divisions: Number(e.target.value) })}
          />
        </label>
      </section>
      <label>
        <input
          type="checkbox"
          aria-label="重なり防止"
          data-tooltip="折る途中で紙どうしが突き抜けにくい補正を切り替えます"
          checked={overlapPrevention}
          onChange={(e) => setDisplay({ overlap_prevention_enabled: e.target.checked })}
        />
        重なり防止
      </label>
      <label>
        <input
          type="checkbox"
          aria-label="食い込み検出"
          data-tooltip="紙の接触を赤い折り目と警告で知らせる検出を切り替えます"
          checked={penetrationPrevention}
          onChange={(e) =>
            setDisplay({ penetration_prevention_enabled: e.target.checked })
          }
        />
        食い込み検出
      </label>
      <MirrorAxisControls />
    </div>
  );
}
