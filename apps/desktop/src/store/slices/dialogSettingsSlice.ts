import type { UiTheme, WheelBehavior } from "../../lib/displayPrefs";
import type { HelpChapterId } from "../../help/helpTypes";
import type { MirrorAxisChoice } from "../../lib/mirror";
import type {
  DisplaySettings,
  DocumentExportKind,
  FoldIssue,
  Paper,
  RecoveryInfo,
  SoftMesh,
  Vec2,
} from "../../lib/types";
import type { DocumentSlice } from "./documentSlice";

/** UI-012の4操作と、全てできた後の完了画面。 */
export type GuideStep = 0 | 1 | 2 | 3 | 4;
export type GuideAction = "fold" | "angle" | "pull" | "inflate";

/** 新規作成ダイアログで決める紙(PAP-001)。squareなら縦を横に合わせる */
export interface NewPaperDraft {
  widthMm: number;
  heightMm: number;
  square: boolean;
}

/** 新規作成ダイアログの初期値(起動時と同じ150×150mmの正方形) */
export const DEFAULT_NEW_PAPER: NewPaperDraft = {
  widthMm: 150,
  heightMm: 150,
  square: true,
};

/** 下書きから実際の紙を作る(正方形なら縦=横) */
export function draftToPaper(draft: NewPaperDraft): Paper {
  return {
    width_mm: draft.widthMm,
    height_mm: draft.square ? draft.widthMm : draft.heightMm,
  };
}

/** 書き出しダイアログで変えられる指定 */
export interface ExportSettings {
  exportKind: DocumentExportKind;
  exportIncludeAux: boolean;
  exportLongSide: number;
}

/** PNGの既定の長辺(点)。Rust側のDEFAULT_LONG_SIDE_PXと揃える(EXP-002) */
export const DEFAULT_PNG_LONG_SIDE = 2048;

/** ダイアログ・設定が所有する状態。同じ1本のZustand storeへ合成する。 */
export interface DialogSettingsSliceState {
  /** 前回の異常終了で残った作業中の内容。あれば復旧ダイアログを出す(SYS-003) */
  recovery: RecoveryInfo | null;
  /** 利用者が選べる前回までの作業。閉じても「前回の作業を確認」から再表示できる。 */
  recoveryChoices: RecoveryInfo[];
  /** 「あとで確認する」を選んだ後は、候補を消さずに復旧ダイアログだけを閉じる。 */
  recoveryDismissed: boolean;
  /** 持ち越しが4件以上あることを既存の下部メッセージ領域へ出すための状態。 */
  recoveryOverflowNotice: string | null;
  /** 復元・破棄の二度押しを防ぐ。 */
  recoveryBusy: boolean;
  /** 書き出しダイアログを開いているか(常設UIは増やさない。EXP-001/EXP-002) */
  exportOpen: boolean;
  /** 書き出す種類 */
  exportKind: DocumentExportKind;
  /** 補助線も含めるか */
  exportIncludeAux: boolean;
  /** PNGのときの長いほうの辺の点数 */
  exportLongSide: number;
  /** 書き出し中か(ボタンの二度押し防止) */
  exportBusy: boolean;
  /** 書き出しに失敗した理由(日本語)。成功したらnull */
  exportError: string | null;
  /** 保存できたファイルの場所。まだならnull(「保存しました」の表示用) */
  exportSavedPath: string | null;
  /** browserが実際に選んだ配送方法を、dialogを閉じた後も知らせる文言。 */
  exportDeliveryNotice: string | null;
  /** 書き出しは続行できたが、利用者へ知らせる必要がある点。 */
  exportFoldIssues: FoldIssue[];
  /** 新規作成ダイアログを開いているか(常設UIは増やさない。PAP-001) */
  newDialogOpen: boolean;
  /** 新規作成ダイアログで決めている紙の形と大きさ */
  newPaperDraft: NewPaperDraft;
  /** 紙の色・方眼の分割数(PAP-003 / CPE-003)。作品ごとの設定。 */
  display: DisplaySettings;
  /** 中央の2D区画の幅の割合(残りが3D区画。UI-004) */
  splitRatio: number;
  /** 下部の「今できる操作」の高さの割合。端末にだけ保存する。 */
  contextPanelRatio: number;
  /** 対称に線を引くか(CPE-010)。消す・種類を変える操作にも同じ基準線で効く。 */
  mirrorDraw: boolean;
  /** 描画・削除・線種変更で共通して使う基準線。作品ファイルには保存しない。 */
  mirrorAxis: MirrorAxisChoice;
  /** 選んだ基準線が無くなったとき、既存の下部通知欄へ出す非エラーのお知らせ。 */
  mirrorAxisNotice: string | null;
  /** 3Dで紙を引くとき左右対称の相手も同時に動かすか(UI-007)。 */
  pullMirror: boolean;
  /** 2D展開図で修飾キーなしのホイールをどう使うか。 */
  wheelBehavior: WheelBehavior;
  /** 画面全体のデザイン。端末にだけ保存する。 */
  uiTheme: UiTheme;
  contextHelpExpanded: boolean;
  viewerHintExpanded: boolean;
  cpHelpExpanded: boolean;
  paperHelpExpanded: boolean;
  paperColorExpanded: boolean;
  guideOpen: boolean;
  guideStep: GuideStep;
  helpOpen: boolean;
  helpChapterId: HelpChapterId;
  helpQuery: string;
  operationStage: number;
  lineInputStart: Vec2 | null;
  paperActionTipVisible: boolean;
  paperActionTipExpanded: boolean;
}

/** ダイアログ・設定が所有する公開action。 */
export interface DialogSettingsSliceActions {
  setPullMirror: (on: boolean) => void;
  setWheelBehavior: (behavior: WheelBehavior) => void;
  setUiTheme: (theme: UiTheme) => void;
  toggleContextHelp: () => void;
  toggleViewerHint: () => void;
  toggleCpHelp: () => void;
  togglePaperHelp: () => void;
  togglePaperColor: () => void;
  openGuide: () => void;
  openHelp: () => void;
  closeHelp: () => void;
  selectHelpChapter: (chapterId: HelpChapterId) => void;
  setHelpQuery: (query: string) => void;
  dismissGuide: () => void;
  completeGuideAction: (action: GuideAction) => void;
  setOperationStage: (stage: number) => void;
  setLineInputStart: (start: Vec2 | null) => void;
  showPaperActionTip: () => void;
  collapsePaperActionTip: () => void;
  expandPaperActionTip: () => void;
  hidePaperActionTip: () => void;
  checkRecovery: () => Promise<void>;
  resolveRecovery: (accept: boolean, candidateId: number) => Promise<void>;
  dismissRecovery: () => void;
  openRecovery: () => void;
  openExport: () => void;
  closeExport: () => void;
  setExportOption: (patch: Partial<ExportSettings>) => void;
  runExport: (path: string) => Promise<void>;
  openNewDialog: () => void;
  closeNewDialog: () => void;
  setNewPaperDraft: (patch: Partial<NewPaperDraft>) => void;
  confirmNewDocument: () => Promise<void>;
  setDisplay: (patch: Partial<DisplaySettings>) => Promise<void>;
  setSoft: (patch: Partial<DisplaySettings>) => void;
  setSplitRatio: (ratio: number) => void;
  setContextPanelRatio: (ratio: number) => void;
  resetPaneSizes: () => void;
}

export type DialogSettingsSlice = DialogSettingsSliceState &
  DialogSettingsSliceActions;

/** B2が所有し、B4 actionが同じ1本のstore上で更新する構造契約。 */
interface DialogSettingsPoseState {
  pullMirrorHinge: number | null;
  softMesh: SoftMesh | null;
  softWarnings: string[];
}

/** B1・B2・B4を同じstoreへ再合成するときの構造契約。 */
export type DialogSettingsHostState = DocumentSlice &
  DialogSettingsSlice &
  DialogSettingsPoseState;
