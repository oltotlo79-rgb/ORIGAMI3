import { beforeEach, describe, expect, it } from "vitest";
import {
  DEFAULT_ONBOARDING,
  ONBOARDING_STORAGE_KEY,
  loadOnboarding,
  saveOnboarding,
} from "./firstRunGuide";
import type { StorageLike } from "./displayPrefs";

let saved: Record<string, string>;

const storage: StorageLike = {
  getItem: (key) => saved[key] ?? null,
  setItem: (key, value) => {
    saved[key] = value;
  },
};

beforeEach(() => {
  saved = {};
});

describe("初回ガイドの保存", () => {
  it("保存が無ければ初回として扱う", () => {
    expect(loadOnboarding(storage)).toEqual(DEFAULT_ONBOARDING);
    expect(loadOnboarding(null)).toEqual(DEFAULT_ONBOARDING);
  });

  it("ガイドと紙操作ヒントの確認済み状態を読み戻す", () => {
    saved[ONBOARDING_STORAGE_KEY] = JSON.stringify({
      guideComplete: true,
      paperActionTipSeen: true,
    });

    expect(loadOnboarding(storage)).toEqual({
      guideComplete: true,
      paperActionTipSeen: true,
    });
  });

  it("壊れた保存や真偽値でない項目は未確認として扱う", () => {
    saved[ONBOARDING_STORAGE_KEY] = "{壊れたJSON";
    expect(loadOnboarding(storage)).toEqual(DEFAULT_ONBOARDING);

    saved[ONBOARDING_STORAGE_KEY] = JSON.stringify({
      guideComplete: "true",
      paperActionTipSeen: 1,
    });
    expect(loadOnboarding(storage)).toEqual(DEFAULT_ONBOARDING);
  });

  it("読み書きで例外が起きても操作を止めない", () => {
    const unavailable: StorageLike = {
      getItem: () => {
        throw new Error("read unavailable");
      },
      setItem: () => {
        throw new Error("write unavailable");
      },
    };

    expect(loadOnboarding(unavailable)).toEqual(DEFAULT_ONBOARDING);
    expect(() => saveOnboarding(DEFAULT_ONBOARDING, unavailable)).not.toThrow();
  });

  it("指定のキーへ状態を保存する", () => {
    const state = { guideComplete: true, paperActionTipSeen: false };

    saveOnboarding(state, storage);

    expect(saved).toEqual({
      [ONBOARDING_STORAGE_KEY]: JSON.stringify(state),
    });
  });
});
