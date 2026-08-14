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

const rasterSvg = (
  id: HelpDiagramId,
  title: string,
  alt: string,
  imageUrl: string,
  manualImage: string,
): HelpDiagram => ({
  id,
  title,
  alt,
  manualImage,
  svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 720 280" aria-hidden="true" focusable="false">
  <title>${title}</title>
  <desc>${alt}</desc>
  <image href="${imageUrl}" x="0" y="0" width="720" height="280" preserveAspectRatio="none"/>
</svg>`,
});

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

const workspaceFourAreas = rasterSvg(
  "workspace-four-areas",
  "画面の4区画",
  "ツールレール、2D展開図、3D表示と手順、高さを変えられる下の設定パネルの位置",
  new URL("./diagram-assets/figure-workspace-four-areas.png", import.meta.url).href,
  "figure-workspace-four-areas.png",
);

const newPaperSettings = rasterSvg(
  "new-paper-settings",
  "新しい紙の設定",
  "紙の幅と高さを決め、24色から表と裏の色を選ぶ画面",
  new URL("./diagram-assets/figure-new-paper-settings.png", import.meta.url).href,
  "figure-new-paper-settings.png",
);

const creaseTools = rasterSvg(
  "crease-tools",
  "展開図の道具",
  "山・谷・補助・曲線を方眼と吸着、二等分方向を使って引く様子",
  new URL("./diagram-assets/figure-crease-tools.png", import.meta.url).href,
  "figure-crease-tools.png",
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

const angleControls = rasterSvg(
  "angle-controls",
  "角度の操作",
  "複数の折り目を選び、個別またはまとめて角度を変え、紙を引く操作",
  new URL("./diagram-assets/figure-angle-controls.png", import.meta.url).href,
  "figure-angle-controls.png",
);

const threeDimensionalControls = rasterSvg(
  "three-dimensional-controls",
  "立体の調整",
  "仕上げの角度、たわみ、ふくらみ、重なり防止で立体を整える様子",
  new URL("./diagram-assets/figure-three-dimensional-controls.png", import.meta.url).href,
  "figure-three-dimensional-controls.png",
);

const techniqueCards = rasterSvg(
  "technique-cards",
  "層操作と8つの名前付き技法",
  "層操作と、段折りからねじり折りまで8種類の名前付き技法を合わせた9つの入口",
  new URL("./diagram-assets/figure-technique-cards.png", import.meta.url).href,
  "figure-technique-cards.png",
);

const timelineFlow = rasterSvg(
  "timeline-flow",
  "手順タイムライン",
  "記録した折り手順を選択し、途中へ挿入して自動再生するタイムライン",
  new URL("./diagram-assets/figure-timeline-flow.png", import.meta.url).href,
  "figure-timeline-flow.png",
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
