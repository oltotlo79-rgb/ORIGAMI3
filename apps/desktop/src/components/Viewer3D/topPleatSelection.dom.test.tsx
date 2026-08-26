// @vitest-environment jsdom
// 「合わせて折る」で得た折り線について、上からK枚のひだを選ぶ画面契約を固定する。

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ALIGN_STEPS, type AlignMode } from "../../lib/alignFold";
import type { FoldTargetInfo, Vec2 } from "../../lib/types";
import {
  useAppStore,
  type AlignDraft,
  type FoldDraft,
} from "../../store/appStore";
import {
  AlignDraftContent,
  FoldDraftContent,
} from "../contextAlignFold";
import { FoldDirectionTip } from "./FoldDirectionTip";

const initialStoreState = useAppStore.getState();

const EXPECTED_ALIGN_MODES = [
  "throughTwoPoints",
  "pointPoint",
  "lineLine",
  "pointPerpendicularLine",
  "pointLineThrough",
  "pointToLinePointToLine",
  "pointLinePerpendicular",
  "existingLine",
] as const satisfies readonly AlignMode[];

const ALL_TARGET_LABEL = "この線にかかる紙を全部（既定）";
const TOP_TARGET_LABEL = "いちばん上の紙だけ";
const K_INPUT_LABEL = "同時に折るひだの枚数";
const TOP_PLEATS_LABEL = /上から.*枚のひだを同時に折る/;
const TOP_IS_NOT_ONE_PLEAT =
  "「いちばん上の紙だけ」は、「ひだを1枚」とは別です。";

type Surface = "context" | "viewer3d";
type TestSelection =
  | { target: "all" }
  | { target: "top" }
  | { target: "topPleats"; topPleatCount: number };

function targetInfo(
  status: FoldTargetInfo["status"],
  availableCount: number | null,
  reason: string | null = null,
  topAction: FoldTargetInfo["topAction"] = null,
): FoldTargetInfo {
  return { status, availableCount, reason, topAction };
}

function makeDraft(
  selection: TestSelection,
  info: FoldTargetInfo | null,
  foldTargetBusy = false,
): FoldDraft {
  return {
    line: [
      [0, 0] as Vec2,
      [1, 0] as Vec2,
    ],
    direction: "Up",
    movingSide: "right",
    docEpoch: 0,
    stepCount: 0,
    upTo: 0,
    foldTargetInfo: info,
    foldTargetBusy,
    ...selection,
  } as FoldDraft;
}

function seed(
  selection: TestSelection,
  info: FoldTargetInfo | null,
  foldTargetBusy = false,
): FoldDraft {
  const draft = makeDraft(selection, info, foldTargetBusy);
  useAppStore.setState({
    activeTool: "fold",
    foldDraft: draft,
    alignDraft: {
      mode: "pointPoint",
      picks: [
        { kind: "point", p: [0, 0] },
        { kind: "point", p: [1, 0] },
      ],
      solutions: [draft.line],
      solutionIndex: 0,
      reason: null,
    },
    foldThroughBusy: false,
  });
  return draft;
}

function renderSurface(
  surface: Surface,
  selection: TestSelection,
  info: FoldTargetInfo | null,
  foldTargetBusy = false,
) {
  const draft = seed(selection, info, foldTargetBusy);
  return surface === "context"
    ? render(<FoldDraftContent draft={draft} showPleatTargets />)
    : render(<FoldDirectionTip />);
}

function queryTargetControl(name: string | RegExp): HTMLElement | null {
  return (
    screen.queryByRole("radio", { name }) ??
    screen.queryByRole("button", { name })
  );
}

function targetControl(name: string | RegExp): HTMLInputElement | HTMLButtonElement {
  const control = queryTargetControl(name);
  if (control === null) {
    throw new Error(`対象操作がありません: ${String(name)}`);
  }
  return control as HTMLInputElement | HTMLButtonElement;
}

function selected(control: HTMLInputElement | HTMLButtonElement): boolean {
  return control instanceof HTMLInputElement
    ? control.checked
    : control.getAttribute("aria-pressed") === "true";
}

function pleatInput(): HTMLInputElement {
  return screen.getByRole("spinbutton", {
    name: K_INPUT_LABEL,
  }) as HTMLInputElement;
}

function currentSelection(): TestSelection {
  const draft = useAppStore.getState().foldDraft;
  if (draft?.target === "topPleats") {
    return { target: "topPleats", topPleatCount: draft.topPleatCount };
  }
  return { target: draft?.target === "top" ? "top" : "all" };
}

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe.each<Surface>(["context", "viewer3d"])(
  "%sの上からK枚UI",
  (surface) => {
    it("readyでは上限とKの意味を示し、最大枚数を一操作で選べる", () => {
      renderSurface(surface, { target: "topPleats", topPleatCount: 2 }, targetInfo("ready", 4));

      expect(screen.getByText("折る紙")).toBeTruthy();
      expect(screen.getByText("この折り線で同時に折れるひだ：4枚")).toBeTruthy();
      expect(screen.getByText("上から2枚のひだを同時に折ります。")).toBeTruthy();
      expect(screen.getByText(TOP_IS_NOT_ONE_PLEAT)).toBeTruthy();

      const input = pleatInput();
      expect(input.value).toBe("2");
      expect(input.min).toBe("1");
      expect(input.max).toBe("4");
      expect(input.disabled).toBe(false);

      fireEvent.click(
        screen.getByRole("button", { name: "同時に折れる4枚を全部選ぶ" }),
      );
      expect(currentSelection()).toEqual({
        target: "topPleats",
        topPleatCount: 4,
      });
    });

    it("limitedでは途中の未完な境目で打ち切った上限を説明する", () => {
      renderSurface(
        surface,
        { target: "topPleats", topPleatCount: 2 },
        targetInfo("limited", 2),
      );

      expect(
        screen.getByText(
          "上から2枚まで選べます。2枚目の下は、まだ最後まで折り重なっていません。",
        ),
      ).toBeTruthy();
      expect(pleatInput().max).toBe("2");
      expect(targetControl(ALL_TARGET_LABEL).disabled).toBe(false);
      expect(targetControl(TOP_TARGET_LABEL).disabled).toBe(false);
      expect(targetControl(TOP_PLEATS_LABEL).disabled).toBe(false);
    });

    it("variesではKだけを無効にし、既存all/topで続けられると知らせる", () => {
      renderSurface(
        surface,
        { target: "all" },
        targetInfo(
          "varies",
          null,
          "折り線の場所によって、同時に折れるひだの枚数が異なります。",
        ),
      );

      expect(
        screen.getByText(
          "折り線の場所によって、同時に折れるひだの枚数が異なります。",
        ),
      ).toBeTruthy();
      expect(
        screen.getByText(
          "ひだの枚数は選べません。今までどおり「この線にかかる紙を全部」か「いちばん上の紙だけ」なら、このまま折れます。",
        ),
      ).toBeTruthy();
      expect(targetControl(ALL_TARGET_LABEL).disabled).toBe(false);
      expect(targetControl(TOP_TARGET_LABEL).disabled).toBe(false);
      expect(targetControl(TOP_PLEATS_LABEL).disabled).toBe(true);
      expect(pleatInput().disabled).toBe(true);
    });

    it("unavailableでもKだけを無効にして既存all/topを残す", () => {
      renderSurface(surface, { target: "top" }, targetInfo("unavailable", null));

      expect(
        screen.getByText("この折り線で同時に折れるひだを確認できません。"),
      ).toBeTruthy();
      expect(targetControl(ALL_TARGET_LABEL).disabled).toBe(false);
      expect(targetControl(TOP_TARGET_LABEL).disabled).toBe(false);
      expect(targetControl(TOP_PLEATS_LABEL).disabled).toBe(true);
      expect(pleatInput().disabled).toBe(true);
    });

    it("crease_only_topでは3対象を無効にし、折り目だけのCTAを有効にする", () => {
      renderSurface(
        surface,
        { target: "all" },
        targetInfo("crease_only_top", 0, null, "crease_only_top"),
      );

      expect(screen.getByText("この折り線で同時に折れるひだ：0枚")).toBeTruthy();
      expect(
        screen.getByText(
          "いちばん上の紙が最後まで折り重なっていないため、今回はひだをまとめて折りません。いちばん上の紙に折り目だけを付け、下の紙と3Dの形は動かしません。",
        ),
      ).toBeTruthy();
      expect(targetControl(ALL_TARGET_LABEL).disabled).toBe(true);
      expect(targetControl(TOP_TARGET_LABEL).disabled).toBe(true);
      expect(targetControl(TOP_PLEATS_LABEL).disabled).toBe(true);
      expect(pleatInput().disabled).toBe(true);
      expect(
        (screen.getByRole("button", { name: "折り目を付ける" }) as HTMLButtonElement)
          .disabled,
      ).toBe(false);
    });

    it("算出中は承認済みの進行文言を表示する", () => {
      renderSurface(surface, { target: "all" }, null, true);

      expect(
        screen.getByText("この折り線で同時に折れるひだを確認しています…"),
      ).toBeTruthy();
    });

    it("Kが新しい上限を超えたとき、現在値を捨てずに選び直しを促す", () => {
      renderSurface(
        surface,
        { target: "topPleats", topPleatCount: 5 },
        targetInfo("ready", 3),
      );

      expect(
        screen.getByText(
          "選んだ5枚は、今同時に折れる3枚を超えています。1枚から3枚までで選び直してください。",
        ),
      ).toBeTruthy();
      expect(pleatInput().value).toBe("5");
      expect(pleatInput().max).toBe("3");
    });

    it("既定allと既存topを同じ選択操作のまま往復でき、Kを残さない", () => {
      const view = renderSurface(
        surface,
        { target: "all" },
        targetInfo("ready", 3),
      );

      const all = targetControl(ALL_TARGET_LABEL);
      const top = targetControl(TOP_TARGET_LABEL);
      expect(selected(all)).toBe(true);
      expect(selected(top)).toBe(false);

      fireEvent.click(top);
      expect(currentSelection()).toEqual({ target: "top" });

      if (surface === "context") {
        view.rerender(
          <FoldDraftContent
            draft={useAppStore.getState().foldDraft!}
            showPleatTargets
          />,
        );
      }
      fireEvent.click(targetControl(ALL_TARGET_LABEL));
      expect(currentSelection()).toEqual({ target: "all" });
    });
  },
);

describe("8つの合わせ方の共通UI経路", () => {
  it("全方式が同じFoldDraftContentからK選択へ到達する", () => {
    expect(Object.keys(ALIGN_STEPS)).toEqual(EXPECTED_ALIGN_MODES);

    for (const mode of EXPECTED_ALIGN_MODES) {
      const foldDraft = seed(
        { target: "all" },
        targetInfo("ready", 3),
      );
      const alignDraft: AlignDraft = {
        mode,
        picks: [],
        solutions: [],
        solutionIndex: 0,
        reason: null,
      };

      render(<AlignDraftContent draft={alignDraft} foldDraft={foldDraft} />);
      expect(screen.getByText("折る紙"), mode).toBeTruthy();
      expect(pleatInput(), mode).toBeTruthy();
      cleanup();
    }
  });
});

describe("生の3Dドラッグとの境界", () => {
  it("合わせ操作でないContextは従来のall/topだけを表示する", () => {
    const draft = seed({ target: "all" }, targetInfo("ready", 3));
    useAppStore.setState({ alignDraft: null });

    render(<FoldDraftContent draft={draft} />);

    expect(screen.getByText("対象の層")).toBeTruthy();
    expect(screen.getByRole("radio", { name: "全ての層" })).toBeTruthy();
    expect(screen.getByRole("radio", { name: "いちばん上の1枚" })).toBeTruthy();
    expect(screen.queryByLabelText(K_INPUT_LABEL)).toBeNull();
  });

  it("合わせ操作でない3D札へK選択を足さない", () => {
    seed({ target: "all" }, targetInfo("ready", 3));
    useAppStore.setState({ alignDraft: null });

    render(<FoldDirectionTip />);

    expect(screen.getByLabelText("折り方を決める")).toBeTruthy();
    expect(screen.queryByText("折る紙")).toBeNull();
    expect(screen.queryByLabelText(K_INPUT_LABEL)).toBeNull();
  });
});
