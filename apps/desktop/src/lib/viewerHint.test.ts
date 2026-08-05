import { describe, expect, it } from "vitest";
import {
  DRAG_FOLD_HINT,
  foldBlockReason,
  viewerHint,
  type HintState,
} from "./viewerHint";

const READY: HintState = {
  hasDoc: true,
  playing: false,
  playT: 1,
  driverCount: 0,
  currentStep: null,
  stepCount: 0,
  tool: "fold",
  hasFoldDraft: false,
  hasTechnique: false,
  techniqueFlapCount: 0,
  hasTechniqueLine: false,
};

describe("foldBlockReason", () => {
  it("折れる状態ならnull", () => {
    expect(foldBlockReason(READY)).toBeNull();
  });

  it("再生中・角度操作中・途中の手順は理由を日本語で返す", () => {
    expect(foldBlockReason({ ...READY, playing: true })).toContain("再生中");
    expect(foldBlockReason({ ...READY, driverCount: 1 })).toContain("角度");
    expect(foldBlockReason({ ...READY, playT: 0.5 })).toContain("折り途中");
    expect(
      foldBlockReason({ ...READY, currentStep: 1, stepCount: 3 }),
    ).toContain("前の手順");
  });

  it("最後の手順を表示しているときは折れる", () => {
    expect(foldBlockReason({ ...READY, currentStep: 3, stepCount: 3 })).toBeNull();
  });
});

describe("viewerHint", () => {
  it("折るツールではドラッグ操作と修飾キーの意味を常に出す", () => {
    const hint = viewerHint(READY);
    expect(hint).toBe(DRAG_FOLD_HINT);
    expect(hint).toContain("Shift");
    expect(hint).toContain("Alt");
    expect(hint).toContain("Ctrl");
  });

  it("折れないときは理由を添える(操作自体は消さない)", () => {
    const hint = viewerHint({ ...READY, playing: true });
    expect(hint).toContain("今は折れません");
    expect(hint).toContain("再生中");
  });

  it("折り線を引いた後はパネルでの決め方を案内する", () => {
    expect(viewerHint({ ...READY, hasFoldDraft: true })).toContain("折る");
  });

  it("技法では選ぶ→層→中心線の順に案内が変わる", () => {
    const t: HintState = { ...READY, tool: "technique" };
    expect(viewerHint(t)).toContain("技法を選んで");
    expect(viewerHint({ ...t, hasTechnique: true })).toContain("重なり");
    expect(
      viewerHint({ ...t, hasTechnique: true, techniqueFlapCount: 2 }),
    ).toContain("中心線");
    expect(
      viewerHint({
        ...t,
        hasTechnique: true,
        techniqueFlapCount: 2,
        hasTechniqueLine: true,
      }),
    ).toContain("適用");
  });

  it("折る以外のツールでも空にならない", () => {
    expect(viewerHint({ ...READY, tool: "select" }).length).toBeGreaterThan(0);
  });
});
