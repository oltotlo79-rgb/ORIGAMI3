// UI-012の初回ガイドと、紙を選んだときの一度だけの詳しい案内を端末に覚える。
// 作品データではないため.ori3へは入れず、既存の画面設定と同じlocalStorageを使う。

import { defaultStorage, type StorageLike } from "./displayPrefs";

export const ONBOARDING_STORAGE_KEY = "origami3.onboarding.v1";

export interface OnboardingState {
  guideComplete: boolean;
  paperActionTipSeen: boolean;
}

export const DEFAULT_ONBOARDING: OnboardingState = {
  guideComplete: false,
  paperActionTipSeen: false,
};

/** 保存が無い・壊れている・読めない場合は初回として扱う。 */
export function loadOnboarding(
  storage?: StorageLike | null,
): OnboardingState {
  try {
    const target = storage === undefined ? defaultStorage() : storage;
    const raw = target?.getItem(ONBOARDING_STORAGE_KEY);
    if (!raw) return DEFAULT_ONBOARDING;
    const saved = JSON.parse(raw) as Partial<OnboardingState>;
    return {
      guideComplete: saved.guideComplete === true,
      paperActionTipSeen: saved.paperActionTipSeen === true,
    };
  } catch {
    return DEFAULT_ONBOARDING;
  }
}

/** 保存できない環境でもガイドや編集操作は止めない。 */
export function saveOnboarding(
  state: OnboardingState,
  storage?: StorageLike | null,
): void {
  try {
    const target = storage === undefined ? defaultStorage() : storage;
    target?.setItem(ONBOARDING_STORAGE_KEY, JSON.stringify(state));
  } catch {
    // private modeや容量制限中でも、この起動中のUI状態はZustandで続けられる。
  }
}
