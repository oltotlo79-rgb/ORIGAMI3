import type {
  GuideStep,
  MeasureMode,
  ProposalStep,
  ToolId,
} from "../store/appStore";
import type { HelpChapterId } from "../help/helpTypes";
import type { ConstructKind } from "./construct";
import type {
  AlignMode,
  DocumentExportKind,
  TechniqueKind,
} from "./types";

/** 実画素点検と恒久検査が共有する、製品の最小画面。 */
export const MINIMUM_APP_VIEWPORT = { width: 1000, height: 700 } as const;

export const AUDITED_TOOL_IDS = [
  "select",
  "measure",
  "mountain",
  "valley",
  "aux",
  "delete",
  "fold",
  "pull",
  "construct",
  "technique",
] as const satisfies readonly ToolId[];

export const AUDITED_MEASURE_MODES = [
  "angle",
  "length",
  "distance",
] as const satisfies readonly MeasureMode[];

export const AUDITED_CONSTRUCT_KINDS = [
  "bisector",
  "perpendicular",
  "divide",
  "angle",
] as const satisfies readonly ConstructKind[];

export type AuditedTechniqueKind = Exclude<TechniqueKind, "Pose">;
export const AUDITED_TECHNIQUE_KINDS = [
  "Simple",
  "Pleat",
  "InsideReverse",
  "OutsideReverse",
  "Squash",
  "Petal",
  "OpenSink",
  "Swivel",
  "Twist",
] as const satisfies readonly AuditedTechniqueKind[];

export const AUDITED_ALIGN_MODES = [
  "throughTwoPoints",
  "pointPoint",
  "lineLine",
  "pointPerpendicularLine",
  "pointLineThrough",
  "pointToLinePointToLine",
  "pointLinePerpendicular",
  "existingLine",
] as const satisfies readonly AlignMode[];

export const AUDITED_HELP_CHAPTER_IDS = [
  "overview",
  "workspace",
  "new-paper",
  "crease-pattern",
  "fold",
  "angles",
  "three-dimensional",
  "techniques",
  "timeline",
  "proposal",
  "save-export",
  "troubleshooting",
  "shortcuts",
] as const satisfies readonly HelpChapterId[];

export const AUDITED_EXPORT_KINDS = [
  "CpSvg",
  "CpPng",
  "DiagramPdf",
  "DiagramSvg",
  "FoldJson",
] as const satisfies readonly DocumentExportKind[];

export const AUDITED_PROPOSAL_STEPS = [
  "skeleton",
  "candidates",
  "paper-position",
  "confirm",
] as const satisfies readonly ProposalStep[];

export const AUDITED_GUIDE_STEPS = [0, 1, 2, 3, 4] as const satisfies readonly GuideStep[];

/** `data-floating-ui` の製品上の正本。追加時は画面シナリオにも割り当てる。 */
export const AUDITED_FLOATING_UI_IDS = [
  "status-badge",
  "suspect-hinge-guide",
  "color-picker",
  "first-run-guide",
  "cp-operation-hint",
  "cp-step-indicator",
  "export-dialog",
  "recovery-dialog",
  "new-document-dialog",
  "help-dialog",
  "proposal-dialog",
  "tooltip",
  "fold-direction-tip",
  "paper-action-tip",
  "view-cube",
  "viewer-operation-hint",
  "viewer-reset",
] as const;

export type FloatingUiId = (typeof AUDITED_FLOATING_UI_IDS)[number];

type ExactCoverage<Expected, Values extends readonly unknown[]> =
  Exclude<Expected, Values[number]> extends never
    ? Exclude<Values[number], Expected> extends never
      ? true
      : never
    : never;

// unionへ値が追加されたのに上の正本tupleを更新し忘れた場合、buildを失敗させる。
export const TOOL_IDS_ARE_EXHAUSTIVE: ExactCoverage<
  ToolId,
  typeof AUDITED_TOOL_IDS
> = true;
export const MEASURE_MODES_ARE_EXHAUSTIVE: ExactCoverage<
  MeasureMode,
  typeof AUDITED_MEASURE_MODES
> = true;
export const CONSTRUCT_KINDS_ARE_EXHAUSTIVE: ExactCoverage<
  ConstructKind,
  typeof AUDITED_CONSTRUCT_KINDS
> = true;
export const TECHNIQUE_KINDS_ARE_EXHAUSTIVE: ExactCoverage<
  AuditedTechniqueKind,
  typeof AUDITED_TECHNIQUE_KINDS
> = true;
export const ALIGN_MODES_ARE_EXHAUSTIVE: ExactCoverage<
  AlignMode,
  typeof AUDITED_ALIGN_MODES
> = true;
export const HELP_CHAPTER_IDS_ARE_EXHAUSTIVE: ExactCoverage<
  HelpChapterId,
  typeof AUDITED_HELP_CHAPTER_IDS
> = true;
export const EXPORT_KINDS_ARE_EXHAUSTIVE: ExactCoverage<
  DocumentExportKind,
  typeof AUDITED_EXPORT_KINDS
> = true;
export const PROPOSAL_STEPS_ARE_EXHAUSTIVE: ExactCoverage<
  ProposalStep,
  typeof AUDITED_PROPOSAL_STEPS
> = true;
export const GUIDE_STEPS_ARE_EXHAUSTIVE: ExactCoverage<
  GuideStep,
  typeof AUDITED_GUIDE_STEPS
> = true;

/** 同じDOM骨格を共有する値違いは統合し、利用者が見分ける安定状態だけを数える。 */
export type ScreenLayoutContract =
  | "workspace"
  | "viewer-overlay"
  | "tooltip"
  | "dialog"
  | "wide-dialog"
  | "paper-position"
  | "help"
  | "guide"
  | "color-picker";

export interface ScreenScenarioCoverage {
  toolIds?: readonly ToolId[];
  measureModes?: readonly MeasureMode[];
  constructKinds?: readonly ConstructKind[];
  techniqueKinds?: readonly AuditedTechniqueKind[];
  alignModes?: readonly AlignMode[];
  helpChapterIds?: readonly HelpChapterId[];
  exportKinds?: readonly DocumentExportKind[];
  proposalSteps?: readonly ProposalStep[];
  guideSteps?: readonly GuideStep[];
  floatingUiIds?: readonly FloatingUiId[];
  /** optional JSX分岐をどの状態で実画素点検するかを固定する。 */
  branches: readonly string[];
}

export interface ScreenScenario<Id extends string = string> {
  id: Id;
  label: string;
  layoutContract: ScreenLayoutContract;
  notes: string;
  coverage: ScreenScenarioCoverage;
}

function scenario<const Id extends string>(
  id: Id,
  label: string,
  layoutContract: ScreenLayoutContract,
  notes: string,
  coverage: ScreenScenarioCoverage,
): ScreenScenario<Id> {
  return { id, label, layoutContract, notes, coverage };
}

/**
 * 1000×700で実画素点検する101状態。
 * P=主要操作、A=案内・展開、L=手順、N=知らせ、O=開く重ね表示。
 */
export const ALL_SCREEN_SCENARIOS = [
  scenario("P01", "選択・何も選んでいない", "workspace", "4区画の基準状態。常設の5浮動要素も同時に数える。", {
    toolIds: ["select"],
    floatingUiIds: ["cp-operation-hint", "cp-step-indicator", "view-cube", "viewer-operation-hint", "viewer-reset"],
    branches: ["selection-empty", "timeline-empty"],
  }),
  scenario("P02", "測る・角度", "workspace", "無効な組合せの理由と正確表示不可の注記を含む最長状態で見る。", {
    toolIds: ["measure"], measureModes: ["angle"], branches: ["measure-invalid", "measure-exact-unavailable"],
  }),
  scenario("P03", "山折り線", "workspace", "始点選択後の現在手順も含める。", {
    toolIds: ["mountain"], branches: ["line-tool-mountain", "operation-stage"],
  }),
  scenario("P04", "谷折り線", "workspace", "谷折り線を引く案内を表示する。", {
    toolIds: ["valley"], branches: ["line-tool-valley"],
  }),
  scenario("P05", "補助線", "workspace", "補助線を引く案内を表示する。", {
    toolIds: ["aux"], branches: ["line-tool-aux"],
  }),
  scenario("P06", "削除", "workspace", "削除対象を選ぶ案内を表示する。", {
    toolIds: ["delete"], branches: ["delete-tool"],
  }),
  scenario("P07", "折る・通常", "workspace", "3Dを直接つかむ通常の折る状態。", {
    toolIds: ["fold"], branches: ["fold-idle"],
  }),
  scenario("P08", "引く", "workspace", "左右対称の切替と紙操作入口を表示する。", {
    toolIds: ["pull"], branches: ["pull-tool"],
  }),
  scenario("P09", "作図・二等分", "workspace", "作図サブメニューの二等分を表示する。", {
    toolIds: ["construct"], constructKinds: ["bisector"], branches: ["construct-bisector"],
  }),
  scenario("P10", "技法・未選択", "workspace", "9技法のサブメニューを開き、下書きはまだ作らない。", {
    toolIds: ["technique"], branches: ["technique-menu"],
  }),
  scenario("P11", "普通の線を選択", "workspace", "線種変更・削除・対称基準の操作を表示する。", {
    branches: ["selection-edge-non-hinge"],
  }),
  scenario("P12", "点を選択", "workspace", "点の座標一覧と対称基準の押せない理由を表示する。", {
    branches: ["selection-vertex"],
  }),
  scenario("P13", "折り目を1本選択", "workspace", "個別角度、固定、記録の操作を表示する。", {
    branches: ["hinge-single", "fold-controls-primary"],
  }),
  scenario("P14", "折り目を複数選択", "workspace", "一括つまみと個別角度の最長行を表示する。", {
    branches: ["hinge-multiple", "hinge-angle-group"],
  }),
  scenario("P15", "測る・線の長さ", "workspace", "長さの結果カードと表示切替を表示する。", {
    toolIds: ["measure"], measureModes: ["length"], branches: ["measure-length-result"],
  }),
  scenario("P16", "測る・2点の距離", "workspace", "展開図と3Dの2結果、3Dで測れない注記も含める。", {
    toolIds: ["measure"], measureModes: ["distance"], branches: ["measure-distance-result", "measure-spatial-unavailable"],
  }),
  scenario("P17", "曲線の全設定", "workspace", "曲線、描き方、分割数、曲がるための線をすべて開く。", {
    branches: ["curve-enabled", "curve-segments-manual", "curve-rulings"],
  }),
  scenario("P18", "合わせて折る・2点を通る", "workspace", "理由文と複数解ボタンをこの方式で同時に出す。", {
    toolIds: ["fold"], alignModes: ["throughTwoPoints"], branches: ["align-reason", "align-multiple-solutions"],
  }),
  scenario("P19", "合わせて折る・点と点", "workspace", "点と点を合わせる選択途中を表示する。", {
    toolIds: ["fold"], alignModes: ["pointPoint"], branches: ["align-progress"],
  }),
  scenario("P20", "合わせて折る・線と線", "workspace", "線と線を合わせる選択途中を表示する。", {
    toolIds: ["fold"], alignModes: ["lineLine"], branches: ["align-progress"],
  }),
  scenario("P21", "合わせて折る・点から線へ垂直", "workspace", "点と線の選択途中を表示する。", {
    toolIds: ["fold"], alignModes: ["pointPerpendicularLine"], branches: ["align-progress"],
  }),
  scenario("P22", "合わせて折る・点を線へ、点を通る", "workspace", "3選択方式の進み具合を表示する。", {
    toolIds: ["fold"], alignModes: ["pointLineThrough"], branches: ["align-three-picks"],
  }),
  scenario("P23", "合わせて折る・2組の点と線", "workspace", "4選択方式の最長進み具合を表示する。", {
    toolIds: ["fold"], alignModes: ["pointToLinePointToLine"], branches: ["align-four-picks"],
  }),
  scenario("P24", "合わせて折る・点と線へ垂直", "workspace", "3選択方式の進み具合を表示する。", {
    toolIds: ["fold"], alignModes: ["pointLinePerpendicular"], branches: ["align-three-picks"],
  }),
  scenario("P25", "合わせて折る・既存の折り目", "workspace", "求まった折り線の確定欄も同時に表示する。", {
    toolIds: ["fold"], alignModes: ["existingLine"], branches: ["align-with-fold-draft"],
  }),
  scenario("P26", "折り方を確定", "workspace", "下部と3Dの両方に向き・動かす側・折る操作を表示する。", {
    toolIds: ["fold"], floatingUiIds: ["fold-direction-tip"], branches: ["fold-draft"],
  }),
  scenario("P27", "巻き込み折り目の提案", "workspace", "追加するか警告だけにするかを表示する。", {
    toolIds: ["fold"], branches: ["fold-through-proposal"],
  }),
  scenario("P28", "作図・垂線", "workspace", "作図サブメニューの垂線を表示する。", {
    toolIds: ["construct"], constructKinds: ["perpendicular"], branches: ["construct-perpendicular"],
  }),
  scenario("P29", "作図・等分", "workspace", "2〜8等分の選択欄を表示する。", {
    toolIds: ["construct"], constructKinds: ["divide"], branches: ["construct-divide-select"],
  }),
  scenario("P30", "作図・角度線", "workspace", "角度刻みの選択欄を表示する。", {
    toolIds: ["construct"], constructKinds: ["angle"], branches: ["construct-angle-select"],
  }),
  scenario("P31", "層操作・既存折り目で開閉", "workspace", "追加済み部分も1件出し、一覧の折返しを確認する。", {
    toolIds: ["technique"], techniqueKinds: ["Simple"], branches: ["layer-motion-reflect", "motion-parts-nonempty"],
  }),
  scenario("P32", "層操作・位置を保つ", "workspace", "動かさず重ね替えの位置保持を表示する。", {
    toolIds: ["technique"], techniqueKinds: ["Simple"], branches: ["layer-motion-stay-keep"],
  }),
  scenario("P33", "層操作・全体の外／元の紙の隣", "workspace", "同じDOM骨格の外側と内側は1状態へ統合する。", {
    toolIds: ["technique"], techniqueKinds: ["Simple"], branches: ["layer-motion-stay-outside-inside"],
  }),
  scenario("P34", "層操作・指定面の隣", "workspace", "隣に置く面を奥・手前の順から選ぶ。", {
    toolIds: ["technique"], techniqueKinds: ["Simple"], branches: ["layer-motion-stay-beside"],
  }),
  scenario("P35", "段折り", "workspace", "候補層を作り、個別チェック一覧を開いた最大分岐で見る。", {
    toolIds: ["technique"], techniqueKinds: ["Pleat"], branches: ["technique-layer-candidates", "technique-layer-details-open", "pleat-width"],
  }),
  scenario("P36", "中割り折り", "workspace", "先端の行き先と開く側を表示する。", {
    toolIds: ["technique"], techniqueKinds: ["InsideReverse"], branches: ["inside-reverse", "open-side"],
  }),
  scenario("P37", "かぶせ折り", "workspace", "先端の行き先と開く側を表示する。", {
    toolIds: ["technique"], techniqueKinds: ["OutsideReverse"], branches: ["outside-reverse", "open-side"],
  }),
  scenario("P38", "開いてつぶす", "workspace", "つぶす先と対象層を表示する。", {
    toolIds: ["technique"], techniqueKinds: ["Squash"], branches: ["squash"],
  }),
  scenario("P39", "花弁折り", "workspace", "持ち上げる先端と対象層を表示する。", {
    toolIds: ["technique"], techniqueKinds: ["Petal"], branches: ["petal"],
  }),
  scenario("P40", "沈め折り", "workspace", "沈める先端と対象層を表示する。", {
    toolIds: ["technique"], techniqueKinds: ["OpenSink"], branches: ["open-sink"],
  }),
  scenario("P41", "ひだ寄せ", "workspace", "寄せる先と対象層を表示する。", {
    toolIds: ["technique"], techniqueKinds: ["Swivel"], branches: ["swivel"],
  }),
  scenario("P42", "ねじり折り", "workspace", "多角形、中心、ねじる角を表示する。", {
    toolIds: ["technique"], techniqueKinds: ["Twist"], branches: ["twist-polygon-ready", "twist-angle"],
  }),

  scenario("A01", "下部の詳しい操作", "workspace", "下部の3段階説明を開く。", {
    branches: ["context-help-expanded"],
  }),
  scenario("A02", "展開図の詳しい操作", "workspace", "展開図のホイール説明を開く。", {
    branches: ["cp-help-expanded"],
  }),
  scenario("A03", "詳しい3D操作", "workspace", "マウス操作3件を開く。", {
    branches: ["viewer-help-expanded"],
  }),
  scenario("A04", "丸みの詳しい操作", "workspace", "丸みの説明を開く。", {
    branches: ["paper-help-expanded"],
  }),
  scenario("A05", "紙の色", "workspace", "表裏48色と2つのその他の色入口を開く。", {
    branches: ["paper-color-expanded"],
  }),
  scenario("A06", "丸みをつける", "workspace", "硬さ・膨らみのつまみと長い注意文を同時に表示する。", {
    branches: ["soft-enabled", "soft-warnings"],
  }),
  scenario("A07", "紙クリックの大きい案内", "viewer-overlay", "引く・ふくらますが使える展開状態。", {
    floatingUiIds: ["paper-action-tip"], branches: ["paper-action-expanded-available"],
  }),
  scenario("A08", "紙クリックの小さい入口", "viewer-overlay", "案内をたたんだ1行状態。", {
    floatingUiIds: ["paper-action-tip"], branches: ["paper-action-compact"],
  }),
  scenario("A09", "紙操作を使えない理由", "viewer-overlay", "2操作とも使えない理由を本文へ出す。", {
    floatingUiIds: ["paper-action-tip"], branches: ["paper-action-all-blocked"],
  }),
  scenario("A10", "3D案内列の送り", "viewer-overlay", "札を重ねず縦送りボタンが出る最大列。", {
    branches: ["viewer-overlay-overflow", "viewer-overlay-scroll-controls"],
  }),
  scenario("A11", "共通の吹き出し", "tooltip", "最長72文字を右下の入口から出し、四辺を確認する。", {
    floatingUiIds: ["tooltip"], branches: ["tooltip-long-bottom-right"],
  }),

  scenario("L01", "手順・最新", "workspace", "10種類の技法名を含む長い手順列を横送りできる状態。", {
    branches: ["timeline-latest", "timeline-all-technique-labels", "timeline-horizontal-overflow"],
  }),
  scenario("L02", "手順・途中を選択", "workspace", "途中の手順設定と並べ替え操作を表示する。", {
    branches: ["timeline-middle-selected", "step-content"],
  }),
  scenario("L03", "手順・再生", "workspace", "折る前から再生し、currentStep=0の押せない操作も含める。", {
    branches: ["timeline-playing", "timeline-at-start"],
  }),
  scenario("L04", "手順・飛ばし警告", "workspace", "飛ばされた札と手順本文の警告を同時に表示する。", {
    branches: ["timeline-skipped", "step-skipped-warning"],
  }),

  scenario("N01", "エラー", "workspace", "3Dのエラー札と下部の本文を表示する。", {
    floatingUiIds: ["status-badge"], branches: ["error-message"],
  }),
  scenario("N02", "平らに畳めない警告", "workspace", "点の件数と直し方を表示する。", {
    floatingUiIds: ["status-badge"], branches: ["flat-fold-warning"],
  }),
  scenario("N03", "前の角度へ自然追従", "workspace", "追従札と折り目ごとの変更値を表示する。", {
    floatingUiIds: ["status-badge"], branches: ["relaxation-status", "relaxation-messages"],
  }),
  scenario("N04", "指定した角度に近い形", "workspace", "指定と違う形でも操作を止めない札を表示する。", {
    floatingUiIds: ["status-badge"], branches: ["pose-not-converged"],
  }),
  scenario("N05", "一般の警告", "workspace", "出どころの違う複数の長文警告を重複なしで積む。", {
    floatingUiIds: ["status-badge"], branches: ["multiple-long-warnings"],
  }),
  scenario("N06", "原因候補の折り目", "workspace", "3Dの原因候補ボタンを表示する。", {
    floatingUiIds: ["suspect-hinge-guide"], branches: ["suspect-hinge"],
  }),
  scenario("N07", "保存できた知らせ", "workspace", "保存先のファイル名を下部に表示する。", {
    branches: ["document-saved"],
  }),
  scenario("N08", "左右対称の知らせ", "workspace", "対称操作の結果を下部に表示する。", {
    branches: ["mirror-axis-notice"],
  }),

  scenario("O01", "前回の作業を復旧", "dialog", "保存時刻と作品名を含む長い復旧文で見る。", {
    floatingUiIds: ["recovery-dialog"], branches: ["recovery-with-path-and-time"],
  }),
  scenario("O02", "新規作成・正方形", "dialog", "縦入力が横へ追従する正方形。", {
    floatingUiIds: ["new-document-dialog"], branches: ["new-square"],
  }),
  scenario("O03", "新規作成・長方形", "dialog", "縦横を別に指定する長方形。", {
    floatingUiIds: ["new-document-dialog"], branches: ["new-rectangle"],
  }),
  scenario("O04", "新規作成・入力エラー", "dialog", "大きさのエラー行と押せない決定を表示する。", {
    floatingUiIds: ["new-document-dialog"], branches: ["new-invalid"],
  }),
  scenario("O05", "提案・形を決める", "wide-dialog", "出っぱり12本の最大行数で見る。", {
    floatingUiIds: ["proposal-dialog"], proposalSteps: ["skeleton"], branches: ["proposal-skeleton-max"],
  }),
  scenario("O06", "提案・候補未選択", "wide-dialog", "4候補を並べ、まだ選ばない。", {
    floatingUiIds: ["proposal-dialog"], proposalSteps: ["candidates"], branches: ["proposal-candidates-unselected"],
  }),
  scenario("O07", "提案・候補選択済み", "wide-dialog", "4候補の状態札と選択後の操作を表示する。", {
    floatingUiIds: ["proposal-dialog"], proposalSteps: ["candidates"], branches: ["proposal-candidates-selected", "candidate-plan-status"],
  }),
  scenario("O08", "提案・紙の上の場所", "paper-position", "紙全体と最大12つまみを560pxのまま表示する。", {
    floatingUiIds: ["proposal-dialog"], proposalSteps: ["paper-position"], branches: ["paper-position-normal", "paper-position-12-handles"],
  }),
  scenario("O09", "提案・場所の食い違い", "paper-position", "最大12件の知らせ、使用元、戻す操作を表示する。", {
    floatingUiIds: ["proposal-dialog"], proposalSteps: ["paper-position"], branches: ["paper-position-difference", "position-notices-max"],
  }),
  scenario("O10", "提案・確認", "wide-dialog", "選んだ展開図と手順数を確認する。", {
    floatingUiIds: ["proposal-dialog"], proposalSteps: ["confirm"], branches: ["proposal-confirm"],
  }),
  scenario("O11", "書き出し・手順なし", "dialog", "折り図を選べない理由を表示する。", {
    floatingUiIds: ["export-dialog"], branches: ["export-no-steps"],
  }),
  scenario("O12", "書き出し・展開図SVG", "dialog", "補助線を含める選択を表示する。", {
    floatingUiIds: ["export-dialog"], exportKinds: ["CpSvg"], branches: ["export-cp-svg"],
  }),
  scenario("O13", "書き出し・展開図PNG", "dialog", "補助線と画像の大きさを表示する。", {
    floatingUiIds: ["export-dialog"], exportKinds: ["CpPng"], branches: ["export-cp-png", "export-long-side"],
  }),
  scenario("O14", "書き出し・折り図PDF", "dialog", "折り図PDFの説明を表示する。", {
    floatingUiIds: ["export-dialog"], exportKinds: ["DiagramPdf"], branches: ["export-diagram-pdf"],
  }),
  scenario("O15", "書き出し・ページごとのSVG", "dialog", "最長の折り図説明を表示する。", {
    floatingUiIds: ["export-dialog"], exportKinds: ["DiagramSvg"], branches: ["export-diagram-svg"],
  }),
  scenario("O16", "ヘルプ・はじめに", "help", "第1章を表示する。", {
    floatingUiIds: ["help-dialog"], helpChapterIds: ["overview"], branches: ["help-chapter"],
  }),
  scenario("O17", "ヘルプ・画面の見かた", "help", "4区画の章を表示する。", {
    floatingUiIds: ["help-dialog"], helpChapterIds: ["workspace"], branches: ["help-chapter"],
  }),
  scenario("O18", "ヘルプ・新しい紙", "help", "紙の形と大きさの章を表示する。", {
    floatingUiIds: ["help-dialog"], helpChapterIds: ["new-paper"], branches: ["help-chapter"],
  }),
  scenario("O19", "ヘルプ・展開図", "help", "展開図を描く章を表示する。", {
    floatingUiIds: ["help-dialog"], helpChapterIds: ["crease-pattern"], branches: ["help-chapter"],
  }),
  scenario("O20", "ヘルプ・折る", "help", "折る操作の章を表示する。", {
    floatingUiIds: ["help-dialog"], helpChapterIds: ["fold"], branches: ["help-chapter"],
  }),
  scenario("O21", "ヘルプ・角度", "help", "角度操作の章を表示する。", {
    floatingUiIds: ["help-dialog"], helpChapterIds: ["angles"], branches: ["help-chapter"],
  }),
  scenario("O22", "ヘルプ・3D", "help", "立体表示の章を表示する。", {
    floatingUiIds: ["help-dialog"], helpChapterIds: ["three-dimensional"], branches: ["help-chapter"],
  }),
  scenario("O23", "ヘルプ・技法", "help", "技法の章を表示する。", {
    floatingUiIds: ["help-dialog"], helpChapterIds: ["techniques"], branches: ["help-chapter"],
  }),
  scenario("O24", "ヘルプ・手順", "help", "手順の章を表示する。", {
    floatingUiIds: ["help-dialog"], helpChapterIds: ["timeline"], branches: ["help-chapter"],
  }),
  scenario("O25", "ヘルプ・提案", "help", "提案の章を表示する。", {
    floatingUiIds: ["help-dialog"], helpChapterIds: ["proposal"], branches: ["help-chapter"],
  }),
  scenario("O26", "ヘルプ・保存と書き出し", "help", "保存と書き出しの章を表示する。", {
    floatingUiIds: ["help-dialog"], helpChapterIds: ["save-export"], branches: ["help-chapter"],
  }),
  scenario("O27", "ヘルプ・困ったとき", "help", "警告と直し方の章を表示する。", {
    floatingUiIds: ["help-dialog"], helpChapterIds: ["troubleshooting"], branches: ["help-chapter"],
  }),
  scenario("O28", "ヘルプ・近道", "help", "キー操作の表を横送りできる状態で表示する。", {
    floatingUiIds: ["help-dialog"], helpChapterIds: ["shortcuts"], branches: ["help-chapter", "help-table-horizontal-scroll"],
  }),
  scenario("O29", "ヘルプ・検索結果なし", "help", "検索0件の案内と検索を消す操作を表示する。", {
    floatingUiIds: ["help-dialog"], branches: ["help-search-empty"],
  }),
  scenario("O30", "基本操作ガイド1", "guide", "折る操作の案内。", {
    floatingUiIds: ["first-run-guide"], guideSteps: [0], branches: ["guide-step-0"],
  }),
  scenario("O31", "基本操作ガイド2", "guide", "角度操作の案内。", {
    floatingUiIds: ["first-run-guide"], guideSteps: [1], branches: ["guide-step-1"],
  }),
  scenario("O32", "基本操作ガイド3", "guide", "引く操作の案内。", {
    floatingUiIds: ["first-run-guide"], guideSteps: [2], branches: ["guide-step-2"],
  }),
  scenario("O33", "基本操作ガイド4", "guide", "ふくらます操作の最長案内。", {
    floatingUiIds: ["first-run-guide"], guideSteps: [3], branches: ["guide-step-3"],
  }),
  scenario("O34", "基本操作ガイド完了", "guide", "完了の祝いと続ける操作を表示する。", {
    floatingUiIds: ["first-run-guide"], guideSteps: [4], branches: ["guide-complete"],
  }),
  scenario("O35", "その他の色", "color-picker", "不正な16進数の追加行も出す最大高さで見る。", {
    floatingUiIds: ["color-picker"], branches: ["color-picker-invalid-hex"],
  }),
  scenario(
    "O36",
    "書き出し・ほかの折り紙ソフトのファイル",
    "dialog",
    "ほかの折り紙ソフトのファイルで使える内容と、そのまま扱えない7項目を安全な文で表示する。",
    {
      floatingUiIds: ["export-dialog"],
      exportKinds: ["FoldJson"],
      branches: ["export-fold-json"],
    },
  ),
] as const;

export type ScreenScenarioId = (typeof ALL_SCREEN_SCENARIOS)[number]["id"];
