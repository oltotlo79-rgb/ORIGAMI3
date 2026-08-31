import type { StoreApi } from "zustand";
import * as ipc from "../../ipc/client";
import {
  DEFAULT_CONTEXT_PANEL_RATIO,
  DEFAULT_DISPLAY,
  DEFAULT_SPLIT_RATIO,
  clampContextPanelRatio,
  clampDivisions,
  clampSplitRatio,
  clampUnit,
  loadPrefs,
  savePrefs,
  softOf,
} from "../../lib/displayPrefs";
import { loadOnboarding, saveOnboarding } from "../../lib/firstRunGuide";
import type { DisplaySettings, DocumentView } from "../../lib/types";
import type { SerialQueue } from "../ipcQueue";
import {
  DEFAULT_NEW_PAPER,
  DEFAULT_PNG_LONG_SIDE,
  draftToPaper,
  type DialogSettingsHostState,
  type DialogSettingsSlice,
  type GuideAction,
} from "../slices/dialogSettingsSlice";

const GUIDE_ACTIONS: GuideAction[] = ["fold", "angle", "pull", "inflate"];

interface DialogSettingsDependencies {
  queue: SerialQueue;
  fail: (error: unknown) => void;
  runViewCommand: (
    task: () => Promise<DocumentView>,
    isNewDocument: boolean,
  ) => Promise<void>;
  waitForFoldAllRestore: () => Promise<void>;
  scheduleSoftShape: () => void;
  queueSoftSave: (display: DisplaySettings) => void;
}

interface DialogSettingsInternals {
  persistPrefs: () => void;
}

interface CreatedDialogSettingsSlice {
  slice: DialogSettingsSlice;
  internals: DialogSettingsInternals;
}

/** ダイアログと表示設定を、同じ1本のZustand storeへ合成する。 */
export function createDialogSettingsSlice<State extends DialogSettingsHostState>(
  setState: StoreApi<State>["setState"],
  getState: StoreApi<State>["getState"],
  dependencies: DialogSettingsDependencies,
): CreatedDialogSettingsSlice {
  const set =
    setState as StoreApi<DialogSettingsHostState>["setState"];
  const get =
    getState as StoreApi<DialogSettingsHostState>["getState"];
  const {
    queue,
    fail,
    runViewCommand,
    waitForFoldAllRestore,
    scheduleSoftShape,
    queueSoftSave,
  } = dependencies;
  const prefs = loadPrefs();
  let onboarding = loadOnboarding();

  /** 画面の使い方の好み(作品の中身ではないもの)を端末に覚えておく */
  const persistPrefs = () => {
    const {
      splitRatio,
      contextPanelRatio,
      mirrorDraw,
      mirrorAxis,
      pullMirror,
      wheelBehavior,
      uiTheme,
      contextHelpExpanded,
      viewerHintExpanded,
      cpHelpExpanded,
      paperHelpExpanded,
      paperColorExpanded,
    } = get();
    savePrefs({
      splitRatio,
      contextPanelRatio,
      mirrorDraw,
      // 作品内の線は保存しない。選択中なら再起動時の戻り先も初期値の縦にする。
      mirrorAxis:
        mirrorAxis.kind === "paperHorizontal"
          ? "paperHorizontal"
          : "paperVertical",
      pullMirror,
      wheelBehavior,
      uiTheme,
      contextHelpExpanded,
      viewerHintExpanded,
      cpHelpExpanded,
      paperHelpExpanded,
      paperColorExpanded,
    });
  };

  /** 初回案内の既読状態だけを端末へ覚える(作品の内容には含めない)。 */
  const updateOnboarding = (patch: Partial<typeof onboarding>) => {
    onboarding = { ...onboarding, ...patch };
    saveOnboarding(onboarding);
  };

  const slice: DialogSettingsSlice = {
    recovery: null,
    recoveryChoices: [],
    recoveryDismissed: false,
    recoveryOverflowNotice: null,
    recoveryBusy: false,
    exportOpen: false,
    exportKind: "CpSvg",
    exportIncludeAux: true,
    exportLongSide: DEFAULT_PNG_LONG_SIDE,
    exportBusy: false,
    exportError: null,
    exportSavedPath: null,
    exportDeliveryNotice: null,
    exportFoldIssues: [],
    newDialogOpen: false,
    newPaperDraft: DEFAULT_NEW_PAPER,
    display: DEFAULT_DISPLAY,
    splitRatio: prefs.splitRatio,
    contextPanelRatio: prefs.contextPanelRatio,
    mirrorDraw: prefs.mirrorDraw,
    mirrorAxis: { kind: prefs.mirrorAxis },
    mirrorAxisNotice: null,
    pullMirror: prefs.pullMirror,
    wheelBehavior: prefs.wheelBehavior,
    uiTheme: prefs.uiTheme,
    contextHelpExpanded: prefs.contextHelpExpanded,
    viewerHintExpanded: prefs.viewerHintExpanded,
    cpHelpExpanded: prefs.cpHelpExpanded,
    paperHelpExpanded: prefs.paperHelpExpanded,
    paperColorExpanded: prefs.paperColorExpanded,
    guideOpen: !onboarding.guideComplete,
    guideStep: 0,
    helpOpen: false,
    helpChapterId: "overview",
    helpQuery: "",
    operationStage: 0,
    lineInputStart: null,
    paperActionTipVisible: false,
    paperActionTipExpanded: false,

    setPullMirror: (on) => {
      set({ pullMirror: on });
      // 切ったら、いま一緒に動かしている相手もその場で外す(次のドラッグを待たない)
      if (!on) set({ pullMirrorHinge: null });
      persistPrefs();
    },

    setWheelBehavior: (behavior) => {
      set({ wheelBehavior: behavior });
      persistPrefs();
    },

    setUiTheme: (theme) => {
      set({ uiTheme: theme });
      persistPrefs();
    },

    toggleContextHelp: () => {
      set((state) => ({ contextHelpExpanded: !state.contextHelpExpanded }));
      persistPrefs();
    },

    toggleViewerHint: () => {
      set((state) => ({ viewerHintExpanded: !state.viewerHintExpanded }));
      persistPrefs();
    },

    toggleCpHelp: () => {
      set((state) => ({ cpHelpExpanded: !state.cpHelpExpanded }));
      persistPrefs();
    },

    togglePaperHelp: () => {
      set((state) => ({ paperHelpExpanded: !state.paperHelpExpanded }));
      persistPrefs();
    },

    togglePaperColor: () => {
      set((state) => ({ paperColorExpanded: !state.paperColorExpanded }));
      persistPrefs();
    },

    openGuide: () => set({ guideOpen: true, guideStep: 0 }),
    openHelp: () => set({ helpOpen: true }),
    closeHelp: () => set({ helpOpen: false }),
    selectHelpChapter: (chapterId) => set({ helpChapterId: chapterId }),
    setHelpQuery: (query) => set({ helpQuery: query }),

    dismissGuide: () => {
      set({ guideOpen: false });
      updateOnboarding({ guideComplete: true });
    },

    completeGuideAction: (action) => {
      const state = get();
      if (
        !state.guideOpen ||
        state.guideStep >= 4 ||
        GUIDE_ACTIONS[state.guideStep] !== action
      ) {
        return;
      }
      const next = (state.guideStep + 1) as DialogSettingsSlice["guideStep"];
      set({ guideStep: next });
      // 最後の操作を実際にできた時点で既読にする。完了カードは閉じるまで残す。
      if (next === 4) updateOnboarding({ guideComplete: true });
    },

    setOperationStage: (stage) => {
      const next = Math.max(0, Math.floor(stage));
      if (get().operationStage !== next) set({ operationStage: next });
    },

    setLineInputStart: (start) => {
      if (get().lineInputStart !== start) set({ lineInputStart: start });
    },

    showPaperActionTip: () => {
      const firstTime = !onboarding.paperActionTipSeen;
      set((state) => ({
        paperActionTipVisible: true,
        // 初回だけ詳しく開く。その後の紙選択では小さなヒントから始める。
        paperActionTipExpanded: state.paperActionTipVisible
          ? state.paperActionTipExpanded
          : firstTime,
      }));
      if (firstTime) updateOnboarding({ paperActionTipSeen: true });
    },

    collapsePaperActionTip: () => set({ paperActionTipExpanded: false }),
    expandPaperActionTip: () =>
      set({ paperActionTipVisible: true, paperActionTipExpanded: true }),
    hidePaperActionTip: () =>
      set({ paperActionTipVisible: false, paperActionTipExpanded: false }),

    checkRecovery: async () => {
      const result = await queue.run(() => ipc.recoveryCheck());
      if (!result.ok) {
        fail(result.error);
        return;
      }
      if (result.value === null) {
        set({
          recovery: null,
          recoveryChoices: [],
          recoveryDismissed: false,
          recoveryOverflowNotice: null,
        });
        return;
      }
      const { choices, overflow_count: overflowCount } = result.value;
      set({
        recovery: choices[0] ?? null,
        recoveryChoices: choices,
        recoveryDismissed: false,
        recoveryOverflowNotice:
          overflowCount > 0
            ? "前回までの作業を4件以上控えています。今の作業は引き続き控えています。不要な内容は「前回の作業を確認」から破棄できます。"
            : null,
      });
    },

    resolveRecovery: async (accept, candidateId) => {
      const state = get();
      if (state.recovery === null || state.recoveryBusy) return;
      const choice =
        state.recoveryChoices.find(
          (candidate) => candidate.candidate_id === candidateId,
        ) ?? null;
      if (choice === null) {
        fail("選んだ復旧候補は一覧に見つかりません。");
        return;
      }
      set({ recoveryBusy: true });
      if (!accept) {
        const result = await queue.run(() =>
          ipc.recoveryRestore(false, choice.candidate_id),
        );
        if (!result.ok) {
          set({ recoveryBusy: false });
          fail(result.error);
          return;
        }
        await get().checkRecovery();
        set({ recoveryBusy: false });
        return;
      }
      // 別の作品に入れ替わるので、新規・開くと同じ扱いで反映する
      try {
        await runViewCommand(async () => {
          const view =
            await ipc.recoveryRestore(true, choice.candidate_id);
          if (!view) throw "作業中だった内容が見つかりませんでした";
          return view;
        }, true);
        await get().checkRecovery();
      } finally {
        set({ recoveryBusy: false });
      }
    },

    dismissRecovery: () => set({ recoveryDismissed: true }),

    openRecovery: () => {
      if (get().recoveryChoices.length > 0) set({ recoveryDismissed: false });
    },

    openExport: () =>
      set({
        exportOpen: true,
        exportError: null,
        exportSavedPath: null,
        exportDeliveryNotice: null,
        exportFoldIssues: [],
      }),

    // 閉じても処理は続く。busyの解除は開始したrunExportだけが行う。
    closeExport: () => set({ exportOpen: false }),

    // 指定を変えたら前回の「保存しました」は別の話になるので消す
    setExportOption: (patch) =>
      set({
        ...patch,
        exportError: null,
        exportSavedPath: null,
        exportDeliveryNotice: null,
        exportFoldIssues: [],
      }),

    runExport: async (path) => {
      const state = get();
      if (state.exportBusy) return;
      if (
        state.exportKind === "CpPng" &&
        !Number.isFinite(state.exportLongSide)
      ) {
        set({
          exportError: "画像の大きさを数で入れてください",
          exportSavedPath: null,
          exportDeliveryNotice: null,
          exportFoldIssues: [],
        });
        return;
      }
      // 保存filterと同じ時点の種類・指定を固定し、復帰待ち中も二重開始を防ぐ。
      const request = {
        kind: state.exportKind,
        includeAux: state.exportIncludeAux,
        pngLongSide: Math.round(state.exportLongSide),
      };
      set({
        exportBusy: true,
        exportError: null,
        exportSavedPath: null,
        exportDeliveryNotice: null,
        exportFoldIssues: [],
      });
      await waitForFoldAllRestore();
      // 書き出しは作品を書き換えないが、直前の編集が反映された内容を出したいので
      // 直列化キューに載せる(編集 → 書き出しの順が守られる)
      const result = await queue.run(() =>
        ipc.documentExport(request.kind, path, {
          include_aux: request.includeAux,
          png_long_side: request.pngLongSide,
        }),
      );
      if (result.ok) {
        set({
          exportBusy: false,
          exportSavedPath: path,
          exportFoldIssues: result.value,
        });
      } else {
        const error = result.error;
        set({
          exportBusy: false,
          exportError:
            typeof error === "string"
              ? error
              : error instanceof Error
                ? error.message
                : String(error),
          exportFoldIssues: [],
        });
      }
    },

    openNewDialog: () => set({ newDialogOpen: true, errorMessage: null }),

    closeNewDialog: () => set({ newDialogOpen: false }),

    setNewPaperDraft: (patch) =>
      set((state) => ({
        newPaperDraft: { ...state.newPaperDraft, ...patch },
      })),

    confirmNewDocument: async () => {
      const paper = draftToPaper(get().newPaperDraft);
      if (!(paper.width_mm > 0) || !(paper.height_mm > 0)) {
        set({ errorMessage: "紙の大きさは0より大きいmmで入れてください" });
        return;
      }
      set({ newDialogOpen: false });
      await get().newDocument(paper);
    },

    setDisplay: async (patch) => {
      const display = { ...get().display, ...patch };
      if (patch.grid_divisions !== undefined) {
        display.grid_divisions = clampDivisions(patch.grid_divisions);
      }
      const doc = get().doc;
      // 色見本や数の入力はその場で見えたほうがよいので、先に画面へ映してから
      // 作品へ書き込む(設計原則3b)。書き込んだ結果が返ればそれで上書きされる
      set(doc ? { display, doc: { ...doc, display } } : { display });
      // 作品ごとの設定として保存する(.ori3に入り、元に戻す/やり直しも効く)。
      // 作品をまだ開いていないときは画面の表示だけ変える
      if (doc) await get().applyEdit({ type: "SetDisplay", display });
    },

    setSoft: (patch) => {
      const beforeSoft = softOf(get().display);
      const display = { ...get().display, ...patch };
      if (patch.soft_stiffness !== undefined) {
        display.soft_stiffness = clampUnit(patch.soft_stiffness, 0.5);
      }
      if (patch.soft_pressure !== undefined) {
        display.soft_pressure = clampUnit(patch.soft_pressure, 0);
      }
      const doc = get().doc;
      // つまみの位置はその場で映す(設計原則3b: 結果を見ながら調整できること)
      set(doc ? { display, doc: { ...doc, display } } : { display });
      const afterSoft = softOf(display);
      if (
        afterSoft.enabled &&
        afterSoft.pressure > 0 &&
        (!beforeSoft.enabled ||
          Math.abs(afterSoft.pressure - beforeSoft.pressure) >= 0.01)
      ) {
        get().completeGuideAction("inflate");
      }
      // 切ったらすぐ従来の描き方へ戻す(次の計算を待たない)
      if (display.soft_enabled !== true) {
        set({ softMesh: null, softWarnings: [] });
      }
      scheduleSoftShape();
      if (doc) queueSoftSave(display);
    },

    setSplitRatio: (ratio) => {
      set({ splitRatio: clampSplitRatio(ratio) });
      persistPrefs();
    },

    setContextPanelRatio: (ratio) => {
      set({ contextPanelRatio: clampContextPanelRatio(ratio) });
      persistPrefs();
    },

    resetPaneSizes: () => {
      set({
        splitRatio: DEFAULT_SPLIT_RATIO,
        contextPanelRatio: DEFAULT_CONTEXT_PANEL_RATIO,
      });
      persistPrefs();
    },
  };

  return {
    slice,
    internals: { persistPrefs },
  };
}
