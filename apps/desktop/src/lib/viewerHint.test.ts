import { describe, expect, it } from "vitest";
import {
  DRAG_FOLD_HINT,
  PULL_HINT,
  foldBlockReason,
  insertPositionHint,
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
  pullBlocked: null,
  pulling: false,
  pullMirrored: false,
};

describe("foldBlockReason", () => {
  it("折れる状態ならnull", () => {
    expect(foldBlockReason(READY)).toBeNull();
  });

  it("再生中・角度操作中・折り途中は理由を日本語で返す", () => {
    expect(foldBlockReason({ ...READY, playing: true })).toContain("再生中");
    expect(foldBlockReason({ ...READY, driverCount: 1 })).toContain("角度");
    expect(foldBlockReason({ ...READY, playT: 0.5 })).toContain("折り途中");
  });

  it("最後の手順でも途中の手順でも折れる(途中なら手順が挟まる。SEQ-006)", () => {
    expect(foldBlockReason({ ...READY, currentStep: 3, stepCount: 3 })).toBeNull();
    expect(foldBlockReason({ ...READY, currentStep: 1, stepCount: 3 })).toBeNull();
  });

  it("途中の手順を見ているときは、どこへ挟まるかを添える", () => {
    expect(insertPositionHint({ ...READY, currentStep: 1, stepCount: 3 })).toContain(
      "手順2の前",
    );
    // 最新(null)や最後の手順を見ているときは末尾へ足すので何も添えない
    expect(insertPositionHint({ ...READY, currentStep: null, stepCount: 3 })).toBe("");
    expect(insertPositionHint({ ...READY, currentStep: 3, stepCount: 3 })).toBe("");
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

  it("ねじり折りでは、中央の形の角を順にクリックするよう常に案内する", () => {
    const t: HintState = {
      ...READY,
      tool: "technique",
      hasTechnique: true,
      techniqueKind: "Twist",
    };
    // まだ足りないうちは「あと何をするか」と今の個数を出す
    const few = viewerHint({ ...t, techniqueVertexCount: 2 });
    expect(few).toContain("角を順にクリック");
    expect(few).toContain("3つ以上");
    expect(few).toContain("いま2個");
    expect(few).toContain("Esc");
    // 3つそろったら適用の案内へ変わる
    const ready = viewerHint({ ...t, techniqueVertexCount: 4 });
    expect(ready).toContain("4角形");
    expect(ready).toContain("適用");
    expect(ready).toContain("中心は形の重心");
    expect(
      viewerHint({ ...t, techniqueVertexCount: 4, techniqueHasCenter: true }),
    ).toContain("中心は指定した点");
  });

  it("引くツールでは、つじつまを合わせて全体が動くことを案内する", () => {
    const p: HintState = { ...READY, tool: "pull" };
    expect(viewerHint(p)).toBe(PULL_HINT);
    expect(viewerHint(p)).toContain("つじつま");
    expect(viewerHint({ ...p, pulling: true })).toContain("折り線");
    // 左右同時に動かしている間は、そのことが分かるように示す(UI-007)
    expect(
      viewerHint({ ...p, pulling: true, pullMirrored: true }),
    ).toContain("左右対称に動かしています");
    expect(viewerHint({ ...p, pullBlocked: "再生中は引けません" })).toContain(
      "今は引けません",
    );
  });

  it("折る以外のツールでも空にならない", () => {
    expect(viewerHint({ ...READY, tool: "select" }).length).toBeGreaterThan(0);
  });
});

describe("合わせて折るの案内", () => {
  it("点を線に合わせるときは、3つの選択を順に案内する", () => {
    const base = { ...READY, alignMode: "pointLineThrough" as const };
    expect(viewerHint({ ...base, alignPickCount: 0 })).toContain(
      "線に合わせたい点",
    );
    expect(viewerHint({ ...base, alignPickCount: 1 })).toContain("合わせ先の線");
    expect(viewerHint({ ...base, alignPickCount: 2 })).toContain(
      "折り目が通る点",
    );
  });

  it("選び始めたら、取り消しのキーを常に添える", () => {
    const hint = viewerHint({
      ...READY,
      alignMode: "pointPoint",
      alignPickCount: 1,
    });
    expect(hint).toContain("Backspace");
    expect(hint).toContain("Esc");
  });

  it("解が2つあるときは切り替えられることを伝える", () => {
    const hint = viewerHint({
      ...READY,
      alignMode: "lineLine",
      alignPickCount: 2,
      alignSolutionCount: 2,
    });
    expect(hint).toContain("解が2つ");
    expect(hint).toContain("折る");
  });

  it("折れないときは理由をそのまま出す", () => {
    const hint = viewerHint({
      ...READY,
      alignMode: "pointLineThrough",
      alignPickCount: 3,
      alignSolutionCount: 0,
      alignReason: "この点を通る折り方では届きません",
    });
    expect(hint).toContain("届きません");
  });

  it("合わせモードでないときは、これまでどおりの案内に戻る", () => {
    expect(viewerHint({ ...READY, alignMode: null })).toContain(DRAG_FOLD_HINT);
  });
});
