import type { HelpDiagram, HelpDiagramId } from "./helpTypes";

const HELP_UI_COLOR_TOKENS: Readonly<Record<string, string>> = {
  "#f5f2ff": "--color-bg",
  "#fff": "--color-surface",
  "#ddd7ec": "--color-border",
  "#9f93b8": "--color-border-strong",
  "#27213d": "--color-text",
  "#007a70": "--color-accent",
  "#005e56": "--color-accent-strong",
  "#ddf8f1": "--color-accent-soft",
  "#7040c9": "--color-secondary",
  "#eee7ff": "--color-secondary-soft",
  "#ffd84d": "--color-pop-yellow",
  "#fff5c2": "--color-pop-yellow-soft",
  "#ed5c70": "--color-pop-coral",
  "#b4233d": "--color-danger",
  "#ffe8ed": "--color-danger-soft",
};

const themeHelpUiColors = (markup: string): string => {
  const preservedPaperColors: string[] = [];
  const protectedMarkup = markup.replace(
    /<!-- preserve-paper-colors:start -->([\s\S]*?)<!-- preserve-paper-colors:end -->/g,
    (_match, paperMarkup: string) => {
      const placeholder = `__HELP_PAPER_COLORS_${preservedPaperColors.length}__`;
      preservedPaperColors.push(paperMarkup);
      return placeholder;
    },
  );

  const themedMarkup = protectedMarkup.replace(
    /(fill|stroke)="(#[0-9a-fA-F]{3,8})"/g,
    (attribute, _property: string, color: string, offset: number, source: string) => {
      const elementStart = source.lastIndexOf("<", offset);
      if (source.slice(elementStart, offset).includes("data-help-paper")) {
        return attribute;
      }
      const token = HELP_UI_COLOR_TOKENS[color.toLowerCase()];
      return token ? attribute.replace(color, `var(${token}, ${color})`) : attribute;
    },
  );

  const restoredMarkup = preservedPaperColors.reduce(
    (result, paperMarkup, index) =>
      result.replace(`__HELP_PAPER_COLORS_${index}__`, paperMarkup),
    themedMarkup,
  );
  return restoredMarkup.replace(/ data-help-paper/g, "");
};

const svg = (
  id: HelpDiagramId,
  title: string,
  alt: string,
  drawing: (arrowId: string) => string,
): HelpDiagram => {
  const arrowId = `${id}-arrow`;
  return {
    id,
    title,
    alt,
    svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 720 280" aria-hidden="true" focusable="false">
  <title>${title}</title>
  <desc>${alt}</desc>
  <defs>
    <marker id="${arrowId}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
      <path d="M0 0 10 5 0 10Z" fill="var(--color-secondary, #7040c9)"/>
    </marker>
  </defs>
  <rect x="8" y="8" width="704" height="264" rx="24" fill="var(--color-bg, #f5f2ff)" stroke="var(--color-border-strong, #9f93b8)" stroke-width="3"/>
  <g font-family="Yu Gothic UI, Meiryo, sans-serif" fill="var(--color-text, #27213d)">${themeHelpUiColors(drawing(arrowId))}</g>
</svg>`,
  };
};

const overviewFlow = svg(
  "overview-flow",
  "作品づくりの流れ",
  "紙を用意し、展開図を描き、折って立体を整え、保存する流れ",
  (arrow) => `
    <g font-size="14" font-weight="700" text-anchor="middle">
      <g transform="translate(22 55)"><rect width="112" height="116" rx="16" fill="#fff" stroke="#007a70" stroke-width="3"/><rect x="34" y="18" width="44" height="44" rx="4" data-help-paper fill="#ddf8f1" stroke="#007a70" stroke-width="2"/><text x="56" y="88">紙を用意</text></g>
      <g transform="translate(163 55)"><rect width="112" height="116" rx="16" fill="#fff" stroke="#007a70" stroke-width="3"/><rect x="30" y="18" width="52" height="44" rx="3" data-help-paper fill="#fff" stroke="#9f93b8" stroke-width="2"/><path d="M32 56 75 20" stroke="#d43c3c" stroke-width="3"/><path d="M34 26 79 55" stroke="#3b6fc9" stroke-width="3"/><text x="56" y="88">線を引く</text></g>
      <g transform="translate(304 55)"><rect width="112" height="116" rx="16" fill="#fff" stroke="#007a70" stroke-width="3"/><path d="M25 58 56 18l31 40-31-13Z" data-help-paper fill="#ffd84d" stroke="#7040c9" stroke-width="2" stroke-linejoin="round"/><path d="M56 18v27" stroke="#d43c3c" stroke-width="3"/><text x="56" y="88">折る</text></g>
      <g transform="translate(445 55)"><rect width="112" height="116" rx="16" fill="#fff" stroke="#007a70" stroke-width="3"/><path d="M22 54Q56 8 90 54L56 67Z" data-help-paper fill="#ddf8f1" stroke="#007a70" stroke-width="2"/><path d="M56 21v34M43 34l13-13 13 13" fill="none" stroke="#7040c9" stroke-width="2"/><text x="56" y="88">立体を整える</text></g>
      <g transform="translate(586 55)"><rect width="112" height="116" rx="16" fill="#fff" stroke="#007a70" stroke-width="3"/><path d="M30 18h40l14 14v36H30Z" fill="#eee7ff" stroke="#7040c9" stroke-width="2"/><path d="M70 18v16h14" fill="none" stroke="#7040c9" stroke-width="2"/><text x="56" y="48" font-size="11">.ori3</text><text x="56" y="88">保存・書き出し</text></g>
    </g>
    <g fill="none" stroke="#7040c9" stroke-width="4" marker-end="url(#${arrow})"><path d="M136 113h23"/><path d="M277 113h23"/><path d="M418 113h23"/><path d="M559 113h23"/></g>
    <path d="M500 194Q360 260 218 194" fill="none" stroke="#7040c9" stroke-width="3" stroke-dasharray="7 6" marker-end="url(#${arrow})"/>
    <text x="360" y="241" font-size="14" text-anchor="middle" font-weight="700">立体を見て展開図を直す</text>`,
);

const workspaceFourAreas = svg(
  "workspace-four-areas",
  "画面の4区画",
  "ツールレール、2D展開図、3D表示と手順、高さを変えられる下の設定パネルの位置",
  () => `
    <rect x="34" y="25" width="652" height="230" rx="13" fill="#fff" stroke="#27213d" stroke-width="3"/>
    <rect x="42" y="34" width="636" height="30" rx="7" fill="#eee7ff"/><text x="60" y="54" font-size="13" font-weight="700">上部ツールバー（作品全体の操作）</text>
    <rect x="42" y="70" width="58" height="118" rx="8" fill="#fff5c2" stroke="#ffd84d" stroke-width="2"/><text x="71" y="104" font-size="12" font-weight="700" text-anchor="middle"><tspan x="71">①</tspan><tspan x="71" dy="22">道具</tspan><tspan x="71" dy="18">レール</tspan></text>
    <rect x="106" y="70" width="238" height="118" rx="8" fill="#ddf8f1" stroke="#007a70" stroke-width="2"/><text x="124" y="91" font-size="14" font-weight="700">② 2D 展開図</text><rect x="166" y="99" width="116" height="70" data-help-paper fill="#fff" stroke="#9f93b8"/><path d="M166 169 282 99M166 99l116 70" stroke="#d43c3c" stroke-width="3"/><path d="M224 99v70" stroke="#3b6fc9" stroke-width="3"/>
    <rect x="350" y="70" width="328" height="82" rx="8" fill="#eee7ff" stroke="#7040c9" stroke-width="2"/><text x="368" y="91" font-size="14" font-weight="700">③ 立体表示</text><path d="M478 139 521 94l55 44-46-14Z" data-help-paper fill="#fff5c2" stroke="#7040c9" stroke-width="3"/><path d="M521 94l9 30" stroke="#d43c3c" stroke-width="3"/>
    <rect x="350" y="160" width="328" height="28" rx="8" fill="#fff" stroke="#7040c9" stroke-width="2"/><text x="367" y="179" font-size="12" font-weight="700">手順タイムライン</text><g fill="#ddf8f1" stroke="#007a70"><rect x="520" y="166" width="28" height="16" rx="7"/><rect x="554" y="166" width="28" height="16" rx="7"/><rect x="588" y="166" width="28" height="16" rx="7"/></g>
    <path d="M330 129h32M338 122l-8 7 8 7M354 122l8 7-8 7" fill="none" stroke="#7040c9" stroke-width="2"/><text x="346" y="97" font-size="10" text-anchor="middle">左右</text>
    <rect x="42" y="196" width="636" height="51" rx="8" fill="#ffe8ed" stroke="#ed5c70" stroke-width="2"/><path d="M360 181v24M353 189l7-8 7 8M353 197l7 8 7-8" fill="none" stroke="#7040c9" stroke-width="2"/><text x="58" y="220" font-size="13" font-weight="700">④ 下部 — 今できる操作と選んだものの設定</text><text x="58" y="238" font-size="11">上端を上下へ動かして高さを変える</text>`,
);

const newPaperSettings = svg(
  "new-paper-settings",
  "新しい紙の設定",
  "紙の幅と高さを決め、24色から表と裏の色を選ぶ画面",
  (arrow) => `
    <!-- preserve-paper-colors:start -->
    <rect x="77" y="51" width="202" height="172" rx="5" fill="#ffd84d" stroke="#27213d" stroke-width="3"/>
    <path d="M77 51h202L238 92H77Z" fill="#fff" fill-opacity=".75"/>
    <!-- preserve-paper-colors:end -->
    <text x="93" y="79" font-size="13" font-weight="700">表</text><text x="243" y="79" font-size="13" font-weight="700">裏</text>
    <path d="M77 235h202M77 229v12M279 229v12" stroke="#7040c9" stroke-width="2" marker-start="url(#${arrow})" marker-end="url(#${arrow})"/><text x="178" y="259" font-size="13" text-anchor="middle">よこ 150 mm</text>
    <path d="M58 51v172M52 51h12M52 223h12" stroke="#7040c9" stroke-width="2" marker-start="url(#${arrow})" marker-end="url(#${arrow})"/><text x="25" y="142" font-size="13" text-anchor="middle" transform="rotate(-90 25 142)">たて 150 mm</text>
    <g transform="translate(330 45)"><text x="0" y="0" font-size="14" font-weight="700">紙の表・紙の裏：24色</text>
      <!-- preserve-paper-colors:start -->
      <g stroke="#fff" stroke-width="2">
        <circle cx="18" cy="34" r="13" fill="#ed1c24"/><circle cx="56" cy="34" r="13" fill="#f4511e"/><circle cx="94" cy="34" r="13" fill="#f06292"/><circle cx="132" cy="34" r="13" fill="#f8bbd0"/><circle cx="170" cy="34" r="13" fill="#ff8c00"/><circle cx="208" cy="34" r="13" fill="#f6b900"/>
        <circle cx="18" cy="70" r="13" fill="#ffd84d"/><circle cx="56" cy="70" r="13" fill="#fff176"/><circle cx="94" cy="70" r="13" fill="#8bc34a"/><circle cx="132" cy="70" r="13" fill="#20a162"/><circle cx="170" cy="70" r="13" fill="#006b4f"/><circle cx="208" cy="70" r="13" fill="#4fc3f7"/>
        <circle cx="18" cy="106" r="13" fill="#29b6f6"/><circle cx="56" cy="106" r="13" fill="#3578e5"/><circle cx="94" cy="106" r="13" fill="#243b78"/><circle cx="132" cy="106" r="13" fill="#7040c9"/><circle cx="170" cy="106" r="13" fill="#b39ddb"/><circle cx="208" cy="106" r="13" fill="#8d5a3b"/>
        <circle cx="18" cy="142" r="13" fill="#f4c7a1"/><circle cx="56" cy="142" r="13" fill="#c88a16"/><circle cx="94" cy="142" r="13" fill="#a7a9ac"/><circle cx="132" cy="142" r="13" fill="#fff" stroke="#9f93b8"/><circle cx="170" cy="142" r="13" fill="#777"/><circle cx="208" cy="142" r="13" fill="#1f1f1f"/>
      </g>
      <!-- preserve-paper-colors:end -->
      <circle cx="18" cy="70" r="17" fill="none" stroke="#7040c9" stroke-width="4"/><path d="m10 70 6 6 11-14" fill="none" stroke="#27213d" stroke-width="3" stroke-linecap="round"/>
      <rect x="0" y="174" width="226" height="35" rx="16" fill="#ddf8f1"/><text x="113" y="197" font-size="13" font-weight="700" text-anchor="middle">24色＋「その他の色」</text>
    </g>`,
);

const creaseTools = svg(
  "crease-tools",
  "展開図の道具",
  "山・谷・補助・曲線を方眼と吸着、二等分方向を使って引く様子",
  () => `
    <rect x="42" y="31" width="408" height="218" rx="7" data-help-paper fill="#fff" stroke="#27213d" stroke-width="3"/>
    <g stroke="#ddd7ec" stroke-width="1"><path d="M93 31v218M144 31v218M195 31v218M246 31v218M297 31v218M348 31v218M399 31v218M42 85h408M42 139h408M42 193h408"/></g>
    <path d="M62 219 247 50" stroke="#d43c3c" stroke-width="5"/><text x="78" y="202" font-size="13" font-weight="700" fill="#d43c3c">山</text>
    <path d="M68 54 419 213" stroke="#3b6fc9" stroke-width="5"/><text x="376" y="195" font-size="13" font-weight="700" fill="#3b6fc9">谷</text>
    <path d="M78 139h345" stroke="#777" stroke-width="3" stroke-dasharray="3 6"/><text x="370" y="132" font-size="12" fill="#666">補助</text>
    <path d="M112 215Q220 118 389 65" fill="none" stroke="#7040c9" stroke-width="4"/><text x="286" y="89" font-size="12" font-weight="700" fill="#7040c9">曲線</text>
    <circle cx="246" cy="139" r="10" fill="none" stroke="#2aa02a" stroke-width="3"/><text x="263" y="158" font-size="12" font-weight="700" fill="#2aa02a">吸着候補</text>
    <g transform="translate(479 35)"><rect width="205" height="87" rx="14" fill="#fff" stroke="#9f93b8" stroke-width="2"/><text x="102" y="20" font-size="13" font-weight="700" text-anchor="middle">方眼の細かさ</text><rect x="18" y="31" width="56" height="43" fill="#ddf8f1" stroke="#007a70"/><path d="M46 31v43M18 52h56" stroke="#9f93b8"/><rect x="129" y="31" width="56" height="43" fill="#ddf8f1" stroke="#007a70"/><g stroke="#9f93b8"><path d="M143 31v43M157 31v43M171 31v43M129 42h56M129 53h56M129 64h56"/></g><path d="M82 53h37" stroke="#7040c9" stroke-width="3"/><path d="m113 47 8 6-8 6" fill="#7040c9"/></g>
    <g transform="translate(479 135)"><rect width="205" height="83" rx="14" fill="#fff" stroke="#9f93b8" stroke-width="2"/><text x="102" y="20" font-size="13" font-weight="700" text-anchor="middle">二等分する向き</text><path d="M28 68 103 35l74 33" fill="none" stroke="#27213d" stroke-width="3"/><path d="M103 35v41" stroke="#7040c9" stroke-width="4" stroke-dasharray="6 4"/><path d="M76 53q27 21 54 0" fill="none" stroke="#ffd84d" stroke-width="5"/></g>
    <rect x="492" y="230" width="180" height="27" rx="13" fill="#fff5c2" stroke="#ffd84d"/><text x="582" y="248" font-size="12" font-weight="700" text-anchor="middle">Shift：方向吸着だけ外す</text>`,
);

const foldFlow = svg(
  "fold-flow",
  "折る操作",
  "折り線、動かす向きと紙の重なりを選び、追加折り目を確認する流れ",
  (arrow) => `
    <g font-size="13" font-weight="700" text-anchor="middle">
      <g transform="translate(25 43)"><rect width="196" height="190" rx="18" fill="#fff" stroke="#007a70" stroke-width="3"/><circle cx="26" cy="27" r="17" fill="#007a70"/><text x="26" y="32" fill="var(--color-on-solid, #fff)">1</text><text x="98" y="31">折り線を引く</text><rect x="42" y="55" width="112" height="96" data-help-paper fill="#ddf8f1" stroke="#27213d" stroke-width="2"/><path d="M49 139 148 65" stroke="#ffd400" stroke-width="4"/><path d="M66 128 128 81" stroke="#7040c9" stroke-width="3" marker-end="url(#${arrow})"/><text x="98" y="175" font-size="11">3Dは Ctrl + ドラッグ</text></g>
      <g transform="translate(262 43)"><rect width="196" height="190" rx="18" fill="#fff" stroke="#007a70" stroke-width="3"/><circle cx="26" cy="27" r="17" fill="#007a70"/><text x="26" y="32" fill="var(--color-on-solid, #fff)">2</text><text x="98" y="31">向きと紙を選ぶ</text><path d="M38 145 98 59l60 86-60-25Z" data-help-paper fill="#ffd84d" stroke="#27213d" stroke-width="2"/><path d="M98 59v61" stroke="#d43c3c" stroke-width="4"/><path d="M133 72q30 30 2 62" fill="none" stroke="#7040c9" stroke-width="3" marker-end="url(#${arrow})"/><g transform="translate(51 157)"><rect x="0" y="8" width="50" height="8" rx="4" data-help-paper fill="#eee7ff"/><rect x="4" y="4" width="50" height="8" rx="4" data-help-paper fill="#ddf8f1"/><rect x="8" width="50" height="8" rx="4" data-help-paper fill="#ffd84d"/></g><text x="136" y="174" font-size="11">対象の層</text></g>
      <g transform="translate(499 43)"><rect width="196" height="190" rx="18" fill="#fff" stroke="#007a70" stroke-width="3"/><circle cx="26" cy="27" r="17" fill="#007a70"/><text x="26" y="32" fill="var(--color-on-solid, #fff)">3</text><text x="104" y="31">折って確認</text><path d="M32 143 94 57l72 83-68-24Z" data-help-paper fill="#ddf8f1" stroke="#27213d" stroke-width="2"/><path d="M94 57 98 116" stroke="#3b6fc9" stroke-width="4"/><path d="M56 119q45-38 91 5" fill="none" stroke="#40cfff" stroke-width="5"/><rect x="28" y="157" width="142" height="25" rx="12" fill="#fff5c2"/><text x="99" y="174" font-size="10">巻き込みの追加折り目</text></g>
    </g>
    <path d="M225 139h29M462 139h29" stroke="#7040c9" stroke-width="4" marker-end="url(#${arrow})"/>`,
);

const angleControls = svg(
  "angle-controls",
  "角度の操作",
  "複数の折り目を選び、個別またはまとめて角度を変え、紙を引く操作",
  (arrow) => `
    <g transform="translate(35 38)"><rect width="210" height="190" rx="18" fill="#fff" stroke="#9f93b8" stroke-width="2"/><text x="105" y="25" font-size="13" font-weight="700" text-anchor="middle">Ctrl + クリックで複数選択</text><rect x="34" y="45" width="142" height="112" data-help-paper fill="#ddf8f1" stroke="#27213d" stroke-width="2"/><path d="M42 147 166 55M45 58l121 88M105 48v104" stroke="#7040c9" stroke-width="5"/><g fill="#ffd84d" stroke="#27213d" stroke-width="2" font-size="11" font-weight="700" text-anchor="middle"><circle cx="75" cy="122" r="13"/><circle cx="134" cy="116" r="13"/><circle cx="105" cy="91" r="13"/></g><g font-size="11" font-weight="700" text-anchor="middle"><text x="75" y="126">1</text><text x="134" y="120">2</text><text x="105" y="95">3</text></g></g>
    <g transform="translate(273 33)"><rect width="205" height="202" rx="18" fill="#fff" stroke="#007a70" stroke-width="3"/><text x="102" y="24" font-size="13" font-weight="700" text-anchor="middle">折り角度</text><g font-size="11"><text x="18" y="59">まとめて動かす</text><text x="18" y="99">折り目 1</text><text x="18" y="139">折り目 2</text><text x="18" y="179">折り目 3</text></g><g stroke="#9f93b8" stroke-width="6" stroke-linecap="round"><path d="M110 54h72M110 94h72M110 134h72M110 174h72"/></g><g fill="#007a70"><circle cx="150" cy="54" r="9"/><circle cx="132" cy="94" r="9"/><circle cx="166" cy="134" r="9"/><circle cx="144" cy="174" r="9"/></g><path d="M150 66v20M150 66l-31 16M150 66l9 16M150 66l-13 16" fill="none" stroke="#7040c9" stroke-width="2" marker-end="url(#${arrow})"/></g>
    <g transform="translate(506 38)"><rect width="178" height="190" rx="18" fill="#fff" stroke="#9f93b8" stroke-width="2"/><text x="89" y="25" font-size="13" font-weight="700" text-anchor="middle">紙を引いて動かす</text><path d="M28 139 80 68l70 67-60-17Z" data-help-paper fill="#ddf8f1" stroke="#27213d" stroke-width="2"/><path d="M80 68l10 50" stroke="#d43c3c" stroke-width="4"/><circle cx="123" cy="92" r="10" fill="#ffd84d" stroke="#7040c9" stroke-width="2"/><path d="M124 92q32-28 28-55" fill="none" stroke="#7040c9" stroke-width="4" marker-end="url(#${arrow})"/><text x="89" y="172" font-size="11" text-anchor="middle">折り目が一緒に動く</text></g>`,
);

const threeDimensionalControls = svg(
  "three-dimensional-controls",
  "立体の調整",
  "仕上げの角度、たわみ、ふくらみ、重なり防止で立体を整える様子",
  (arrow) => `
    <g font-size="12" font-weight="700" text-anchor="middle">
      <g transform="translate(32 34)"><rect width="165" height="72" rx="16" fill="#fff" stroke="#7040c9" stroke-width="2"/><text x="82" y="22">この形で仕上げる</text><path d="M42 58 68 32l25 23 28-22 18 25" fill="none" stroke="#7040c9" stroke-width="3"/><g fill="#ffd84d" stroke="#27213d"><circle cx="68" cy="32" r="4"/><circle cx="93" cy="55" r="4"/><circle cx="121" cy="33" r="4"/></g></g>
      <g transform="translate(523 34)"><rect width="165" height="72" rx="16" fill="#fff" stroke="#007a70" stroke-width="2"/><text x="82" y="22">紙のたわみ</text><path d="M27 40q55 38 111 0" fill="none" stroke="#007a70" stroke-width="5"/><path d="M82 34v27" stroke="#7040c9" stroke-width="3" marker-end="url(#${arrow})"/></g>
      <g transform="translate(32 176)"><rect width="165" height="72" rx="16" fill="#fff" stroke="#007a70" stroke-width="2"/><text x="82" y="22">ふくらます</text><ellipse cx="82" cy="52" rx="37" ry="14" data-help-paper fill="#ddf8f1" stroke="#007a70" stroke-width="3"/><path d="M82 49V30M50 49 34 38M114 49l16-11" stroke="#7040c9" stroke-width="2" marker-end="url(#${arrow})"/></g>
      <g transform="translate(523 176)"><rect width="165" height="72" rx="16" fill="#fff" stroke="#ed5c70" stroke-width="2"/><text x="82" y="22">重なり防止</text><path d="M39 60 93 35l38 18-54 17Z" data-help-paper fill="#eee7ff" stroke="#7040c9"/><path d="M34 50 88 25l38 18-54 17Z" data-help-paper fill="#ffd84d" stroke="#27213d"/><path d="M137 30v29M128 45h18" stroke="#ed5c70" stroke-width="4"/></g>
    </g>
    <g transform="translate(244 62)"><path d="M18 124 111 17l121 113-111-39Z" data-help-paper fill="#ddf8f1" stroke="#27213d" stroke-width="3"/><path d="M111 17 121 91M18 124l103-33 111 39" fill="none" stroke="#7040c9" stroke-width="3"/><g stroke="#9f93b8" stroke-width="1"><path d="M54 83l100 20M77 56l107 58M94 37l119 82"/><path d="M61 109 131 36M91 114l66-55M124 123l61-36"/></g><path d="M121 91q-5-40-10-74" fill="none" stroke="#007a70" stroke-width="4"/></g>
    <g fill="none" stroke="#7040c9" stroke-width="3" marker-end="url(#${arrow})"><path d="M197 89 266 113"/><path d="M523 89 454 113"/><path d="M197 211 266 177"/><path d="M523 211 454 177"/></g>`,
);

const techniqueCards = svg(
  "technique-cards",
  "層操作と8つの名前付き技法",
  "層操作と、段折りからねじり折りまで8種類の名前付き技法を合わせた9つの入口",
  () => `
    <g font-size="11" font-weight="700" text-anchor="middle">
      <g transform="translate(29 22)"><rect width="202" height="65" rx="13" fill="#fff5c2" stroke="#ffd84d" stroke-width="4"/><text x="101" y="17">層操作</text><path d="m57 49 39-17 49 15-39 17Z" data-help-paper fill="#eee7ff" stroke="#7040c9" stroke-width="2"/><path d="m57 40 39-17 49 15-39 17Z" data-help-paper fill="#ddf8f1" stroke="#27213d" stroke-width="2"/><path d="M42 29v24m0-24-7 8m7-8 7 8M160 53V29m0 24-7-8m7 8 7-8" fill="none" stroke="#7040c9" stroke-width="2"/></g>
      <g transform="translate(259 22)"><rect width="202" height="65" rx="13" fill="#fff" stroke="#9f93b8" stroke-width="2"/><text x="101" y="17">段折り</text><path d="M50 37h102" stroke="#d43c3c" stroke-width="4"/><path d="M50 51h102" stroke="#3b6fc9" stroke-width="4"/></g>
      <g transform="translate(489 22)"><rect width="202" height="65" rx="13" fill="#fff" stroke="#9f93b8" stroke-width="2"/><text x="101" y="17">中割り折り</text><path d="M65 55 101 25l36 30-36-12Z" data-help-paper fill="#ddf8f1" stroke="#27213d"/><path d="M101 25v29" stroke="#7040c9" stroke-width="3"/></g>
      <g transform="translate(29 107)"><rect width="202" height="65" rx="13" fill="#fff" stroke="#9f93b8" stroke-width="2"/><text x="101" y="17">かぶせ折り</text><path d="M65 55 101 26l36 29-36-11Z" data-help-paper fill="#eee7ff" stroke="#27213d"/><path d="M76 43q25-25 51 2" fill="none" stroke="#7040c9" stroke-width="3"/></g>
      <g transform="translate(259 107)"><rect width="202" height="65" rx="13" fill="#fff" stroke="#9f93b8" stroke-width="2"/><text x="101" y="17">開いてつぶす</text><path d="M101 24 66 55h70Z" data-help-paper fill="#ddf8f1" stroke="#27213d"/><path d="M101 28 78 53m23-25 23 25" stroke="#7040c9" stroke-width="3"/></g>
      <g transform="translate(489 107)"><rect width="202" height="65" rx="13" fill="#fff" stroke="#9f93b8" stroke-width="2"/><text x="101" y="17">花弁折り</text><path d="M66 56 101 25l35 31-35-13Z" data-help-paper fill="#ddf8f1" stroke="#27213d"/><path d="M101 25v31" stroke="#d43c3c" stroke-width="3"/></g>
      <g transform="translate(29 192)"><rect width="202" height="65" rx="13" fill="#fff" stroke="#9f93b8" stroke-width="2"/><text x="101" y="17">沈め折り</text><path d="M67 55 101 25l34 30Z" data-help-paper fill="#eee7ff" stroke="#27213d"/><path d="M101 28v21" stroke="#ed5c70" stroke-width="4"/><path d="m94 43 7 10 7-10" fill="#ed5c70"/></g>
      <g transform="translate(259 192)"><rect width="202" height="65" rx="13" fill="#fff" stroke="#9f93b8" stroke-width="2"/><text x="101" y="17">ひだ寄せ</text><path d="M65 55q36-34 72 0M76 55q25-22 51 0" fill="none" stroke="#7040c9" stroke-width="4"/></g>
      <g transform="translate(489 192)"><rect width="202" height="65" rx="13" fill="#fff" stroke="#9f93b8" stroke-width="2"/><text x="101" y="17">ねじり折り</text><path d="m101 24 27 18-10 20H84L74 42Z" data-help-paper fill="#ffd84d" stroke="#27213d" stroke-width="2"/><path d="M101 24q28 9 20 31" fill="none" stroke="#7040c9" stroke-width="3"/><path d="m116 49 6 8 6-8" fill="#7040c9"/></g>
    </g>`,
);

const timelineFlow = svg(
  "timeline-flow",
  "手順タイムライン",
  "記録した折り手順を選択し、途中へ挿入して自動再生するタイムライン",
  (arrow) => `
    <g transform="translate(58 39)"><path d="M20 54 55 20l35 34-35-13Z" data-help-paper fill="#fff" stroke="#27213d"/><path d="M158 54 193 20l35 34-35-13Z" data-help-paper fill="#ddf8f1" stroke="#27213d"/><path d="M296 54 331 20l35 34-35-13Z" data-help-paper fill="#fff5c2" stroke="#27213d"/><path d="M434 54 469 20l35 34-35-13Z" data-help-paper fill="#eee7ff" stroke="#27213d"/></g>
    <path d="M61 168h598" stroke="#9f93b8" stroke-width="6" stroke-linecap="round"/>
    <g font-size="11" font-weight="700" text-anchor="middle"><g transform="translate(48 140)"><rect width="104" height="57" rx="14" fill="#fff" stroke="#9f93b8" stroke-width="2"/><text x="52" y="34">1 単純折り</text></g><g transform="translate(181 140)"><rect width="104" height="57" rx="14" fill="#eee7ff" stroke="#7040c9" stroke-width="4"/><text x="52" y="34">2 中割り折り</text></g><g transform="translate(430 140)"><rect width="104" height="57" rx="14" fill="#fff" stroke="#9f93b8" stroke-width="2"/><text x="52" y="34">3 段折り</text></g><g transform="translate(563 140)"><rect width="104" height="57" rx="14" fill="#fff" stroke="#9f93b8" stroke-width="2"/><text x="52" y="34">4 仕上げの角度</text></g></g>
    <g transform="translate(309 92)"><rect width="96" height="48" rx="14" fill="#fff5c2" stroke="#ffd84d" stroke-width="3"/><text x="48" y="21" font-size="12" font-weight="700" text-anchor="middle">新しい手順</text><text x="48" y="38" font-size="11" text-anchor="middle">途中へ挿入</text><path d="M48 49v42" stroke="#7040c9" stroke-width="4" marker-end="url(#${arrow})"/></g>
    <g transform="translate(58 224)"><circle cx="22" cy="15" r="15" fill="#007a70"/><path d="m18 8 12 7-12 7Z" fill="var(--color-on-solid, #fff)"/><text x="48" y="20" font-size="13" font-weight="700">自動再生</text><path d="M128 15h100" stroke="#7040c9" stroke-width="3" marker-end="url(#${arrow})"/><text x="245" y="20" font-size="12">手順を順番に折る</text></g>`,
);

const proposalWizard = svg(
  "proposal-wizard",
  "提案ウィザード",
  "出っぱりを指定し、候補を選び、展開図として使う3段階",
  (arrow) => `
    <g font-size="13" font-weight="700" text-anchor="middle">
      <g transform="translate(27 36)"><rect width="205" height="207" rx="18" fill="#fff" stroke="#007a70" stroke-width="3"/><circle cx="27" cy="27" r="17" fill="#007a70"/><text x="27" y="32" fill="var(--color-on-solid, #fff)">1</text><text x="105" y="28">出っぱりを作る</text><circle cx="102" cy="111" r="19" fill="#ffd84d" stroke="#27213d"/><g stroke="#7040c9" stroke-width="6" stroke-linecap="round"><path d="M102 92 102 57"/><path d="M102 130 102 168"/><path d="M88 99 55 76"/><path d="M116 99 151 73"/><path d="M88 123 56 150"/><path d="M116 123 152 151"/></g><text x="102" y="191" font-size="11">長さ・太さを調整</text></g>
      <g transform="translate(258 36)"><rect width="205" height="207" rx="18" fill="#fff" stroke="#007a70" stroke-width="3"/><circle cx="27" cy="27" r="17" fill="#007a70"/><text x="27" y="32" fill="var(--color-on-solid, #fff)">2</text><text x="105" y="28">候補を選ぶ</text><g data-help-paper fill="#ddf8f1" stroke="#9f93b8" stroke-width="2"><rect x="23" y="50" width="72" height="72"/><rect x="109" y="50" width="72" height="72"/><rect x="23" y="134" width="72" height="50"/><rect x="109" y="134" width="72" height="50"/></g><g stroke="#7040c9" stroke-width="2"><path d="M23 122 95 50M23 50l72 72M109 122l72-72M109 50l72 72M23 184l72-50M109 184l72-50"/></g><circle cx="166" cy="63" r="13" fill="#ffd84d" stroke="#27213d"/><path d="m159 63 5 5 9-11" fill="none" stroke="#27213d" stroke-width="2"/></g>
      <g transform="translate(489 36)"><rect width="205" height="207" rx="18" fill="#fff" stroke="#007a70" stroke-width="3"/><circle cx="27" cy="27" r="17" fill="#007a70"/><text x="27" y="32" fill="var(--color-on-solid, #fff)">3</text><text x="105" y="28">展開図を使う</text><rect x="42" y="55" width="122" height="122" data-help-paper fill="#ddf8f1" stroke="#27213d" stroke-width="2"/><path d="M42 177 164 55M42 55l122 122M103 55v122M42 116h122" stroke="#7040c9" stroke-width="3"/><rect x="46" y="184" width="114" height="17" rx="8" fill="#fff5c2"/><text x="103" y="197" font-size="10">あとから自由に直せる</text></g>
    </g><path d="M235 140h16M466 140h16" stroke="#7040c9" stroke-width="4" marker-end="url(#${arrow})"/>`,
);

const saveExportFlow = svg(
  "save-export-flow",
  "保存と書き出し",
  "編集を続ける.ori3保存とSVG、PNG、折り図への書き出しの違い",
  (arrow) => `
    <g transform="translate(65 60)"><path d="M0 0h164l35 35v147H0Z" fill="#ddf8f1" stroke="#007a70" stroke-width="4"/><path d="M164 0v35h35" fill="none" stroke="#007a70" stroke-width="3"/><text x="99" y="55" font-size="20" font-weight="700" text-anchor="middle">作品.ori3</text><g transform="translate(28 78)"><rect width="44" height="44" data-help-paper fill="#fff" stroke="#27213d"/><path d="M0 44 44 0" stroke="#d43c3c" stroke-width="3"/><rect x="55" width="44" height="44" rx="9" fill="#eee7ff" stroke="#7040c9"/><text x="77" y="28" font-size="12" text-anchor="middle">1 2 3</text><circle cx="129" cy="22" r="21" fill="#ffd84d" stroke="#27213d"/></g><text x="99" y="158" font-size="12" text-anchor="middle">紙・線・手順・表示設定</text></g>
    <path d="M66 173H31V89h31" fill="none" stroke="#7040c9" stroke-width="3" marker-end="url(#${arrow})"/><text x="21" y="76" font-size="12" font-weight="700">開いて</text><text x="21" y="91" font-size="12" font-weight="700">続ける</text>
    <g font-size="13" font-weight="700" text-anchor="middle"><g transform="translate(434 31)"><rect width="218" height="58" rx="16" fill="#fff" stroke="#7040c9" stroke-width="3"/><text x="109" y="25">展開図 SVG</text><text x="109" y="44" font-size="11" font-weight="400">実寸・拡大しても鮮明</text></g><g transform="translate(434 111)"><rect width="218" height="58" rx="16" fill="#fff" stroke="#7040c9" stroke-width="3"/><text x="109" y="25">展開図 PNG</text><text x="109" y="44" font-size="11" font-weight="400">手軽に開ける画像</text></g><g transform="translate(434 191)"><rect width="218" height="58" rx="16" fill="#fff" stroke="#7040c9" stroke-width="3"/><text x="109" y="25">折り図 PDF / SVG</text><text x="109" y="44" font-size="11" font-weight="400">折り方の説明つき</text></g></g>
    <g fill="none" stroke="#7040c9" stroke-width="4" marker-end="url(#${arrow})"><path d="M269 151Q340 60 426 60"/><path d="M269 151h157"/><path d="M269 151q71 69 157 69"/></g><rect x="290" y="126" width="106" height="30" rx="15" fill="#fff5c2"/><text x="343" y="146" font-size="12" font-weight="700" text-anchor="middle">書き出し</text>`,
);

const troubleshootingFlow = svg(
  "troubleshooting-flow",
  "困ったときの確認順",
  "警告を読み、折り目の確認または元に戻す操作で直して続ける流れ",
  (arrow) => `
    <g transform="translate(32 82)"><path d="M50 0 98 84H2Z" fill="#ffe8ed" stroke="#ed5c70" stroke-width="4"/><text x="50" y="57" font-size="30" font-weight="700" text-anchor="middle" fill="#b4233d">!</text><text x="50" y="105" font-size="13" font-weight="700" text-anchor="middle">警告を読む</text></g>
    <path d="M137 129h59" stroke="#7040c9" stroke-width="4" marker-end="url(#${arrow})"/>
    <g transform="translate(205 62)"><rect width="179" height="135" rx="18" fill="#fff" stroke="#ed5c70" stroke-width="3"/><path d="M24 102 85 35l70 67-66-22Z" data-help-paper fill="#fff5c2" stroke="#27213d" stroke-width="2"/><path d="M88 31 88 109" stroke="#ed5c70" stroke-width="6"/><text x="90" y="124" font-size="12" font-weight="700" text-anchor="middle">紙が突き抜けそう</text></g>
    <g fill="none" stroke="#7040c9" stroke-width="4" marker-end="url(#${arrow})"><path d="M386 103Q421 54 459 54"/><path d="M386 160q35 55 73 55"/></g>
    <g transform="translate(468 25)"><rect width="213" height="78" rx="17" fill="#fff5c2" stroke="#ffd84d" stroke-width="3"/><text x="106" y="23" font-size="13" font-weight="700" text-anchor="middle">追加折り目を確認</text><rect x="48" y="34" width="116" height="31" data-help-paper fill="#fff" stroke="#27213d"/><path d="M51 62q56-42 110 0" fill="none" stroke="#ed8a2b" stroke-width="4" stroke-dasharray="7 5"/></g>
    <g transform="translate(468 174)"><rect width="213" height="78" rx="17" fill="#eee7ff" stroke="#7040c9" stroke-width="3"/><text x="106" y="28" font-size="13" font-weight="700" text-anchor="middle">元に戻す</text><g transform="translate(44 39)"><rect width="83" height="25" rx="8" fill="#fff" stroke="#9f93b8"/><text x="41" y="17" font-size="12" font-weight="700" text-anchor="middle">Ctrl + Z</text></g><path d="M158 62q25-30 2-42" fill="none" stroke="#7040c9" stroke-width="3" marker-end="url(#${arrow})"/></g>
    <circle cx="692" cy="139" r="17" fill="#007a70"/><path d="m684 139 6 6 11-14" fill="none" stroke="var(--color-on-solid, #fff)" stroke-width="4"/><text x="656" y="145" font-size="12" font-weight="700" text-anchor="end">直して続ける</text>`,
);

const shortcutMap = svg(
  "shortcut-map",
  "キーボード操作",
  "F1やCtrlとZなど、よく使うキーと対応する操作の一覧",
  () => `
    <g font-size="12" font-weight="700">
      <g transform="translate(30 32)"><rect width="205" height="92" rx="16" fill="#fff" stroke="#9f93b8" stroke-width="2"/><g transform="translate(18 18)"><rect width="67" height="34" rx="8" fill="#fff5c2" stroke="#7040c9" stroke-width="2"/><text x="33" y="22" text-anchor="middle">Ctrl</text><text x="92" y="22">+</text><rect x="108" width="42" height="34" rx="8" fill="#fff5c2" stroke="#7040c9" stroke-width="2"/><text x="129" y="22" text-anchor="middle">Z</text></g><rect x="34" y="62" width="137" height="23" rx="11" fill="#ddf8f1"/><text x="102" y="78" text-anchor="middle">元に戻す</text></g>
      <g transform="translate(258 32)"><rect width="205" height="92" rx="16" fill="#fff" stroke="#9f93b8" stroke-width="2"/><g transform="translate(18 18)"><rect width="67" height="34" rx="8" fill="#fff5c2" stroke="#7040c9" stroke-width="2"/><text x="33" y="22" text-anchor="middle">Ctrl</text><text x="92" y="22">+</text><rect x="108" width="42" height="34" rx="8" fill="#fff5c2" stroke="#7040c9" stroke-width="2"/><text x="129" y="22" text-anchor="middle">Y</text></g><rect x="34" y="62" width="137" height="23" rx="11" fill="#ddf8f1"/><text x="102" y="78" text-anchor="middle">やり直す</text></g>
      <g transform="translate(486 32)"><rect width="205" height="92" rx="16" fill="#fff" stroke="#9f93b8" stroke-width="2"/><rect x="72" y="18" width="62" height="34" rx="8" fill="#fff5c2" stroke="#7040c9" stroke-width="2"/><text x="103" y="40" text-anchor="middle">F1</text><rect x="34" y="62" width="137" height="23" rx="11" fill="#ddf8f1"/><text x="102" y="78" text-anchor="middle">ヘルプを開く</text></g>
      <g transform="translate(30 151)"><rect width="205" height="92" rx="16" fill="#fff" stroke="#9f93b8" stroke-width="2"/><rect x="61" y="18" width="84" height="34" rx="8" fill="#fff5c2" stroke="#7040c9" stroke-width="2"/><text x="103" y="40" text-anchor="middle">Shift</text><rect x="25" y="62" width="155" height="23" rx="11" fill="#ddf8f1"/><text x="102" y="78" text-anchor="middle">方向吸着だけ外す</text></g>
      <g transform="translate(258 151)"><rect width="205" height="92" rx="16" fill="#fff" stroke="#9f93b8" stroke-width="2"/><rect x="22" y="18" width="62" height="34" rx="8" fill="#fff5c2" stroke="#7040c9" stroke-width="2"/><text x="53" y="40" text-anchor="middle">Ctrl</text><text x="92" y="40">+</text><circle cx="133" cy="35" r="17" fill="#fff5c2" stroke="#7040c9" stroke-width="2"/><path d="m130 27 8 9-6 2 4 8-5 2-4-8-5 4Z" fill="#27213d"/><rect x="34" y="62" width="137" height="23" rx="11" fill="#ddf8f1"/><text x="102" y="78" text-anchor="middle">複数選択</text></g>
      <g transform="translate(486 151)"><rect width="205" height="92" rx="16" fill="#fff" stroke="#9f93b8" stroke-width="2"/><rect x="62" y="18" width="82" height="34" rx="8" fill="#fff5c2" stroke="#7040c9" stroke-width="2"/><text x="103" y="40" text-anchor="middle">Esc</text><rect x="25" y="62" width="155" height="23" rx="11" fill="#ddf8f1"/><text x="102" y="78" text-anchor="middle">閉じる・やめる</text></g>
    </g>`,
);

export const HELP_DIAGRAMS: Readonly<Record<HelpDiagramId, HelpDiagram>> = {
  "overview-flow": overviewFlow,
  "workspace-four-areas": workspaceFourAreas,
  "new-paper-settings": newPaperSettings,
  "crease-tools": creaseTools,
  "fold-flow": foldFlow,
  "angle-controls": angleControls,
  "three-dimensional-controls": threeDimensionalControls,
  "technique-cards": techniqueCards,
  "timeline-flow": timelineFlow,
  "proposal-wizard": proposalWizard,
  "save-export-flow": saveExportFlow,
  "troubleshooting-flow": troubleshootingFlow,
  "shortcut-map": shortcutMap,
};
