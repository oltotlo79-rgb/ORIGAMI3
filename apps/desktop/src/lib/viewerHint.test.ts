import { describe, expect, it } from "vitest";
import {
  DRAG_FOLD_HINT,
  FOLD_DECIDE_PLACES,
  PULL_HINT,
  SELECTABLE_3D_EDGE_TARGETS,
  SNAP_POINT_TARGETS,
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

  it("角度の値を受け取り、0°だけなら折れるが0°以外があれば止める", () => {
    const common = {
      hasDoc: true,
      playing: false,
      playT: 1,
      currentStep: null,
      stepCount: 0,
    };
    expect(foldBlockReason({ ...common, driverAngles: [0, -0] })).toBeNull();
    expect(foldBlockReason({ ...common, driverAngles: [0, 45] })).toContain("角度");
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
    // ドラッグしにくい場所でも折り線を決められる2回のクリックも案内する
    expect(hint).toContain("Ctrl+クリックを2回");
    // 引いた折り線は3Dの札からも決められるので、下部パネルだけを案内しない
    expect(hint).toContain(FOLD_DECIDE_PLACES);
  });

  it("折れないときは理由を添える(操作自体は消さない)", () => {
    const hint = viewerHint({ ...READY, playing: true });
    expect(hint).toContain("今は折れません");
    expect(hint).toContain("再生中");
  });

  it("折り返す紙は3D札で確かめ、向きは3D札と下部パネルの両方で決めると案内する", () => {
    const hint = viewerHint({ ...READY, hasFoldDraft: true });
    expect(hint).toContain("折る");
    expect(hint).toContain(
      "折り返す紙は3Dの「この折り線で折る」の札で確かめ",
    );
    expect(hint).toContain("向きは同じ札か下部パネルで選んで");
    expect(hint).not.toContain("動かす側");
    expect(hint).toContain("やめる");
  });

  it("折り方を決める場所は、画面に見える札の見出しで案内する", () => {
    expect(FOLD_DECIDE_PLACES).toBe(
      "3Dの「この折り線で折る」の札か下部パネル",
    );
    for (const hint of [
      viewerHint(READY),
      viewerHint({
        ...READY,
        alignMode: "lineLine",
        alignPickCount: 2,
        alignSolutionCount: 1,
      }),
    ]) {
      expect(hint).toContain(FOLD_DECIDE_PLACES);
      // 「下部パネル」だけを指す統一表記を保つこと
      expect(hint.split("下部パネル")).toHaveLength(2);
    }

    // 折り線を引いた後は、直前に固有見出しを示した同じ札を短く指せる。
    const decided = viewerHint({ ...READY, hasFoldDraft: true });
    expect(decided).toContain(
      "折り返す紙は3Dの「この折り線で折る」の札で確かめ",
    );
    expect(decided).toContain("向きは同じ札か下部パネルで選んで");
    expect(decided.split("下部パネル")).toHaveLength(2);
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

  it("技法固有の必要層数と候補から選んだ部分集合を案内する", () => {
    const t: HintState = {
      ...READY,
      tool: "technique",
      hasTechnique: true,
      techniqueKind: "InsideReverse",
      techniqueCandidateCount: 5,
      techniqueFlapCount: 1,
    };
    const reverse = viewerHint(t);
    expect(reverse).toContain("候補5枚のうち1枚");
    expect(reverse).toContain("2枚以上");

    const squash = viewerHint({
      ...t,
      techniqueKind: "Squash",
      techniqueFlapCount: 1,
    });
    expect(squash).toContain("中心線");
    expect(squash).toContain("Ctrl+クリック");
    expect(squash).toContain("基準点");

    const sink = viewerHint({
      ...t,
      techniqueKind: "OpenSink",
      techniqueCandidateCount: 0,
      techniqueFlapCount: 0,
    });
    expect(sink).toContain("すべての層");
    expect(sink).toContain("中心線");

    const petal = viewerHint({
      ...t,
      techniqueKind: "Petal",
      techniqueFlapCount: 1,
      hasTechniqueLine: true,
    });
    expect(petal).toContain("開く側");
    expect(petal).toContain("Ctrl+クリック");
    expect(
      viewerHint({
        ...t,
        techniqueKind: "Swivel",
        techniqueFlapCount: 0,
        hasTechniqueLine: true,
        techniqueHasReference: true,
      }),
    ).toContain("基準点も指定しました");
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
    expect(few).toContain("3個以上");
    expect(few).toContain("現在2個");
    expect(few).toContain("Esc");
    expect(few).toContain("Shift+クリック");
    // 3つそろったら適用の案内へ変わる
    const ready = viewerHint({ ...t, techniqueVertexCount: 4 });
    expect(ready).toContain("4角形");
    expect(ready).toContain("適用");
    expect(ready).toContain("中心は形の重心");
    expect(ready).toContain("開く側");
    expect(ready).toContain("Shift+クリック");
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

  it("選択では3Dで選べる4種類だけを一意に列挙し、点も選べることを出す", () => {
    expect(SELECTABLE_3D_EDGE_TARGETS).toBe(
      "山折り線・谷折り線・補助線・紙の輪郭の辺",
    );
    const hint = viewerHint({ ...READY, tool: "select" });
    expect(hint).toBe(
      `3Dの紙を見回し、点と${SELECTABLE_3D_EDGE_TARGETS}を選べます(点はCtrl+クリックで足す・外す、ドラッグで動かせます)`,
    );
    // 4種類の列挙は1回だけ(言い換えや二重列挙をしない)
    expect(hint.split(SELECTABLE_3D_EDGE_TARGETS)).toHaveLength(2);
  });

  it("山・谷・補助では、3Dの紙にも同じ線を引けることを線の名前つきで出す", () => {
    for (const [tool, label] of [
      ["mountain", "山折り線"],
      ["valley", "谷折り線"],
      ["aux", "補助線"],
    ] as const) {
      const hint = viewerHint({ ...READY, tool });
      expect(hint.startsWith(`${label}:`), label).toBe(true);
      expect(hint).toContain("3Dの紙も2回クリック");
      expect(hint).toContain(SNAP_POINT_TARGETS);
      expect(hint).toContain("Esc");
    }
  });

  it("作図では、3Dの紙からも点や線を順に選べることを出す", () => {
    const hint = viewerHint({ ...READY, tool: "construct" });
    expect(hint).toContain("3Dの紙の上でも");
    expect(hint).toContain("点や線");
    expect(hint).toContain(SNAP_POINT_TARGETS);
    expect(hint).toContain("Esc");
  });

  it("削除は3Dから線を消せないので、選べるものだけを案内する", () => {
    const hint = viewerHint({ ...READY, tool: "delete" });
    expect(hint).toBe(`3Dの紙を見回し、${SELECTABLE_3D_EDGE_TARGETS}を選べます`);
    expect(hint).not.toContain("点");
  });

  it("点を指す道具の案内には、点のことが必ず書いてある", () => {
    // 3Dの逆写像で点を指せる道具(選択・山・谷・補助・作図・折る)を数え漏らさない
    for (const tool of [
      "select",
      "mountain",
      "valley",
      "aux",
      "construct",
    ] as const) {
      const hint = viewerHint({ ...READY, tool });
      expect(hint.includes("点") || hint.includes("クリック"), tool).toBe(true);
    }
  });
});

describe("合わせて折るの案内", () => {
  it.each([
    ["throughTwoPoints", ["折り目が通る1つ目の点", "折り目が通る2つ目の点"]],
    ["pointPoint", ["1つ目の点(動かす方)", "2つ目の点(合わせ先)"]],
    ["lineLine", ["1つ目の線(動かす方)", "2つ目の線(合わせ先)"]],
    ["pointPerpendicularLine", ["折り目が通る点", "折り目を垂直にする線"]],
    ["pointLineThrough", ["線に合わせたい点", "合わせ先の線", "折り目が通る点"]],
    [
      "pointToLinePointToLine",
      [
        "線に合わせたい1つ目の点",
        "1つ目の合わせ先の線",
        "線に合わせたい2つ目の点",
        "2つ目の合わせ先の線",
      ],
    ],
    [
      "pointLinePerpendicular",
      ["線に合わせたい点", "点の合わせ先の線", "折り目を垂直にする線"],
    ],
    ["existingLine", ["折り目にする既存の線"]],
  ] as const)("%sの選択を順に案内する", (alignMode, prompts) => {
    for (const [alignPickCount, prompt] of prompts.entries()) {
      expect(
        viewerHint({ ...READY, alignMode, alignPickCount }),
      ).toContain(prompt);
    }
    expect(
      viewerHint({
        ...READY,
        alignMode,
        alignPickCount: prompts.length,
        alignSolutionCount: 1,
      }),
    ).toContain("折り線が決まりました");
  });

  it.each([
    ["lineLine", 0],
    ["lineLine", 1],
    ["pointPerpendicularLine", 1],
    ["pointLineThrough", 1],
    ["pointToLinePointToLine", 1],
    ["pointToLinePointToLine", 3],
    ["pointLinePerpendicular", 1],
    ["pointLinePerpendicular", 2],
    ["existingLine", 0],
  ] as const)("%sの線選択手順%dでも同じ4種類を1回だけ列挙する", (alignMode, alignPickCount) => {
    const hint = viewerHint({ ...READY, alignMode, alignPickCount });
    expect(hint.split(SELECTABLE_3D_EDGE_TARGETS)).toHaveLength(2);
    expect(hint).toContain(`(${SELECTABLE_3D_EDGE_TARGETS}を選べます)`);
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

  it("公理6で解が3つあるときは実際の本数を伝える", () => {
    const hint = viewerHint({
      ...READY,
      alignMode: "pointToLinePointToLine",
      alignPickCount: 4,
      alignSolutionCount: 3,
    });
    expect(hint).toContain("解が3つ");
    expect(hint).toContain("別の解");
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
