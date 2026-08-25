// @vitest-environment jsdom
// 提案ウィザードの画面テスト(Task 3-4):
// 閉じているときは何も出さない、出っぱりの増減が骨格に反映される、
// 候補を選んで「この展開図を使う」と edit_apply ReplaceCreasePattern が飛ぶ。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render as testingLibraryRender,
  screen,
  within,
} from "@testing-library/react";

/**
 * 共通の画面土台は内容をdocument.body直下へ出す。
 * 既存の内容検査が確かめる対象を変えないよう、この画面が今回作ったdialogを
 * 従来どおりcontainerとして返す。閉じている場合は元のcontainerを返す。
 */
function render(
  ...args: Parameters<typeof testingLibraryRender>
): ReturnType<typeof testingLibraryRender> {
  const existing = new Set(
    document.body.querySelectorAll<HTMLElement>(
      '[data-floating-ui="proposal-dialog"]',
    ),
  );
  const result = testingLibraryRender(...args);
  const dialog = Array.from(
    document.body.querySelectorAll<HTMLElement>(
      '[data-floating-ui="proposal-dialog"]',
    ),
  ).find((element) => !existing.has(element));
  return dialog === undefined ? result : { ...result, container: dialog };
}

vi.mock("../../ipc/client", () => ({
  documentNew: vi.fn(),
  documentOpen: vi.fn(),
  documentSave: vi.fn(),
  editApply: vi.fn(),
  editApplyBatch: vi.fn(),
  editUndo: vi.fn(),
  editRedo: vi.fn(),
  sequenceApply: vi.fn(),
  sequenceReplay: vi.fn(),
  poseSolve: vi.fn(),
  recoveryCheck: vi.fn(),
  recoveryRestore: vi.fn(),
  proposalGenerate: vi.fn(),
  proposalProgress: vi.fn(),
  proposalControl: vi.fn(),
  proposalApply: vi.fn(),
}));

import * as ipc from "../../ipc/client";
import { ProposalWizard, foldPlanLabel, violationLabel } from "./ProposalWizard";
import { focusableElements, type FocusTarget } from "./ModalDialog";
import { useAppStore } from "../../store/appStore";
import {
  defaultSkeleton,
  leafNodes,
  limbs,
  skeletonRows,
} from "../../lib/skeleton";
import { completionPositionsOnPaper } from "../../lib/proposalPosition";
import type {
  CreasePattern,
  DocumentView,
  FoldStep,
  ProposalCandidate,
  ProposalFoldPlan,
  ProposalJobResult,
  ProposalProgressSnapshot,
  Skeleton,
} from "../../lib/types";

/** markで区別できる最小の展開図(正方形の輪郭だけ) */
function makeCp(mark: number): CreasePattern {
  return {
    vertices: [
      { id: 0, pos: [0, 0] },
      { id: 1, pos: [1, 0] },
      { id: 2, pos: [1, 1] },
      { id: 3, pos: [0, 1] },
    ],
    edges: [
      { id: 0, v0: 0, v1: 1, kind: "Border" },
      { id: 1, v0: 1, v1: 2, kind: "Mountain" },
      { id: 2, v0: 2, v1: 3, kind: "Valley" },
      { id: 3, v0: 3, v1: 0, kind: "Border" },
    ],
    next_vertex_id: 4,
    next_edge_id: mark,
  };
}

function makeCandidate(mark: number, violations: number): ProposalCandidate {
  return {
    cp: makeCp(mark),
    scale: 0.4,
    violations,
    warnings: [],
    fold_plan: null,
  };
}

function proposalResult(
  jobId: string,
  candidates: ProposalCandidate[],
): ProposalJobResult {
  return { job_id: jobId, candidates };
}

function progressSnapshot(
  jobId: string,
  done: number,
  total: number,
): ProposalProgressSnapshot {
  return { job_id: jobId, done, total, phase: "Generating" };
}

function mockProposalCandidates(candidates: ProposalCandidate[]): void {
  vi.mocked(ipc.proposalGenerate).mockImplementation(
    (_skeleton, _paper, _seed, jobId) =>
      Promise.resolve(proposalResult(jobId, candidates)),
  );
}

/** 紙の上の先端対応を持つ候補。12件でも重ならない4×3格子へ並べる。 */
function makeCandidateWithSites(
  mark: number,
  count: number,
): ProposalCandidate {
  return {
    ...makeCandidate(mark, 0),
    sites: Array.from({ length: count }, (_, index) => ({
      circle: {
        leaf_id: index + 1,
        circle_index: index,
        center: [((index % 4) + 0.5) / 4, (Math.floor(index / 4) + 0.5) / 3] as [
          number,
          number,
        ],
        radius: 0.04,
      },
      vertex: null,
      molecules: [],
    })),
  };
}

/** 縦スクロール検査用。最後の先端だけ紙の下辺へ置く。 */
function makeSizedCandidateWithSites(
  mark: number,
  count: number,
  width: number,
  height: number,
): ProposalCandidate {
  const candidate = makeCandidateWithSites(mark, count);
  return {
    ...candidate,
    cp: {
      ...candidate.cp,
      vertices: [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [width, 0] },
        { id: 2, pos: [width, height] },
        { id: 3, pos: [0, height] },
      ],
    },
    sites: candidate.sites?.map((site, index) => ({
      ...site,
      circle: {
        ...site.circle,
        center:
          index === count - 1
            ? ([width / 2, 0] as [number, number])
            : ([site.circle.center[0] * width, site.circle.center[1] * height] as [
                number,
                number,
              ]),
      },
    })),
  };
}

/** 折り方が付いた候補。`cp` は折り込んだ後の展開図なので候補の展開図と別にする */
function makeCandidateWithPlan(
  mark: number,
  checked: number,
  options: {
    planned?: number;
    status?: ProposalFoldPlan["status"];
  } = {},
): ProposalCandidate {
  return {
    ...makeCandidate(mark, 0),
    fold_plan: {
      steps: makeSteps(checked),
      cp: makeCp(mark + 100),
      planned: options.planned ?? checked,
      checked,
      status: options.status ?? "checked_to_finish",
    },
  };
}

/** 作業30の名前付き標本: 胴から頭1・尾1・足4が出る6先端の骨格。 */
function headTailFourLegsSkeleton(): Skeleton {
  return {
    nodes: [
      { id: 0, parent: null, length: 0, width_factor: 1 },
      { id: 1, parent: 0, length: 1, width_factor: 1 },
      { id: 2, parent: 0, length: 1, width_factor: 1 },
      { id: 3, parent: 0, length: 0.7, width_factor: 1 },
      { id: 4, parent: 0, length: 0.7, width_factor: 1 },
      { id: 5, parent: 0, length: 0.7, width_factor: 1 },
      { id: 6, parent: 0, length: 0.7, width_factor: 1 },
    ],
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

const VIEW: DocumentView = {
  doc: {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: makeCp(4),
    sequence: [],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  },
  faces: [],
  warnings: [],
  violations: [],
  frame: null,
  skipped: [],
  contact_detected: false,
};

function makeSteps(count: number): FoldStep[] {
  return Array.from({ length: count }, (_, index) => ({
    id: index + 1,
    kind: "Simple",
    drivers: [],
    layer_order: null,
    note: "",
  }));
}

function showConfirmationWithSteps(count: number) {
  useAppStore.setState({
    doc: { ...VIEW.doc, sequence: makeSteps(count) },
    proposalStep: "confirm",
    proposalCandidates: [makeCandidate(10, 0)],
    proposalSelected: 0,
    proposalBusy: false,
    proposalError: null,
  });
}

type KeyboardProposalStep =
  | "skeleton"
  | "candidates"
  | "paper-position"
  | "confirm";

const KEYBOARD_PROPOSAL_STEPS: readonly KeyboardProposalStep[] = [
  "skeleton",
  "candidates",
  "paper-position",
  "confirm",
];

function ProposalKeyboardHarness() {
  const open = useAppStore((state) => state.openProposal);
  return (
    <>
      <button type="button" onClick={open}>
        提案を開く
      </button>
      <ProposalWizard />
    </>
  );
}

/** 4段階を、それぞれ実際に到達し得る値で直接表示する。 */
function prepareKeyboardProposalStep(step: KeyboardProposalStep): void {
  if (step === "skeleton") {
    useAppStore.getState().openProposal();
    return;
  }

  const candidates = [
    makeCandidateWithSites(20, 4),
    makeCandidateWithSites(21, 4),
  ];
  useAppStore.setState({
    proposalStep: "candidates",
    proposalSkeleton: radialSkeleton(4),
    proposalCandidates: candidates,
    proposalSelected: step === "candidates" ? 1 : 0,
    proposalPaperSource: null,
    proposalPaperPositions: [],
    proposalBusy: false,
    proposalError: null,
  });
  if (step === "paper-position") {
    useAppStore.getState().openProposalPaperPositionEditor();
    return;
  }
  if (step === "confirm") {
    useAppStore.getState().setProposalStep("confirm");
  }
}

function expectedInitialProposalTarget(step: KeyboardProposalStep): FocusTarget {
  if (step === "skeleton") {
    return screen.getAllByRole("slider", { name: /の長さ$/u })[0] as FocusTarget;
  }
  if (step === "candidates") {
    const selected = useAppStore.getState().proposalSelected ?? 0;
    return screen.getByRole("button", {
      name: `候補${selected + 1}`,
    }) as FocusTarget;
  }
  if (step === "paper-position") {
    const handle = screen
      .getByRole("dialog")
      .querySelector<FocusTarget>(
        '[data-paper-position-handle][tabindex="0"]:not([aria-disabled="true"])',
      );
    expect(handle).not.toBeNull();
    return handle!;
  }
  return screen.getByRole("button", { name: "選び直す" }) as FocusTarget;
}

/**
 * jsdomはEnterからbuttonのclickを自動生成しないため、keydownが取り消されなかった
 * ときだけ、ブラウザーが続けて行うclickを補う。pointer/mousedownは発生させない。
 */
function activateButtonWithKeyboard(button: HTMLButtonElement): void {
  const down = new KeyboardEvent("keydown", {
    key: "Enter",
    bubbles: true,
    cancelable: true,
  });
  fireEvent(button, down);
  expect(down.defaultPrevented).toBe(false);
  if (!down.defaultPrevented) {
    act(() => button.click());
  }
  fireEvent.keyUp(button, { key: "Enter" });
}

/** jsdomに無い通常のTab移動だけを補い、端の循環は共通土台へ任せる。 */
function pressProposalTab(dialog: HTMLElement, shiftKey = false): void {
  const ordered = focusableElements(dialog);
  const active = document.activeElement as FocusTarget | null;
  const down = new KeyboardEvent("keydown", {
    key: "Tab",
    shiftKey,
    bubbles: true,
    cancelable: true,
  });
  fireEvent(active ?? dialog, down);
  if (!down.defaultPrevented) {
    const current = active === null ? -1 : ordered.indexOf(active);
    const next = shiftKey
      ? current <= 0
        ? ordered.length - 1
        : current - 1
      : current < 0 || current === ordered.length - 1
        ? 0
        : current + 1;
    ordered[next]?.focus();
  }
  fireEvent.keyUp(document.activeElement ?? dialog, { key: "Tab", shiftKey });
}

function tabToProposalTarget(dialog: HTMLElement, target: FocusTarget): void {
  const limit = focusableElements(dialog).length + 1;
  for (let index = 0; index < limit && document.activeElement !== target; index += 1) {
    pressProposalTab(dialog);
  }
  expect(document.activeElement).toBe(target);
}

function shapeRow(container: HTMLElement, id: number): HTMLElement {
  const row = container.querySelector<HTMLElement>(`[data-shape-row="${id}"]`);
  expect(row).not.toBeNull();
  return row!;
}

/** 画面上の指定行から1本足し、新しくできた部分のIDを返す。 */
function extendFrom(container: HTMLElement, parentId: number): number {
  const before = new Set(
    useAppStore.getState().proposalSkeleton.nodes.map((node) => node.id),
  );
  const row = shapeRow(container, parentId);
  const add = within(row).getByRole("button", { name: /のこの先に足す$/u });
  expect(add.textContent?.trim()).toBe("＋ この先に足す");
  fireEvent.click(add);
  const added = useAppStore
    .getState()
    .proposalSkeleton.nodes.find(
      (node) => node.parent === parentId && !before.has(node.id),
    );
  expect(added).toBeDefined();
  return added!.id;
}

beforeEach(() => {
  vi.mocked(ipc.proposalGenerate).mockReset();
  vi.mocked(ipc.proposalProgress).mockReset();
  vi.mocked(ipc.proposalControl).mockReset();
  vi.mocked(ipc.editApply).mockResolvedValue(VIEW);
  mockProposalCandidates([]);
  vi.mocked(ipc.proposalProgress).mockResolvedValue(null);
  vi.mocked(ipc.proposalControl).mockImplementation((operation) =>
    Promise.resolve({
      job_id: operation.job_id,
      done: 0,
      total: 0,
      phase: "Cancelled",
    }),
  );
  useAppStore.getState().closeProposal();
  useAppStore.setState({
    doc: { ...VIEW.doc, sequence: [] },
    proposalSkeleton: defaultSkeleton(),
    proposalCandidates: [],
    proposalSelected: null,
    proposalJobId: null,
    proposalProgress: null,
    proposalProgressWarning: null,
    proposalError: null,
  });
});

afterEach(() => {
  useAppStore.getState().closeProposal();
  cleanup();
  vi.clearAllMocks();
});

describe("提案ウィザード", () => {
  it.each([
    ["形", "skeleton"],
    ["候補", "candidates"],
    ["紙の上", "paper-position"],
    ["確認", "confirm"],
  ] as const)("%sの段階を開くと所定の最初の操作へ移る", (_name, step) => {
    prepareKeyboardProposalStep(step);
    render(<ProposalWizard />);

    expect(document.activeElement).toBe(expectedInitialProposalTarget(step));
  });

  it.each(KEYBOARD_PROPOSAL_STEPS)(
    "%sの段階でTabを100回循環する",
    (step) => {
      prepareKeyboardProposalStep(step);
      render(<ProposalWizard />);
      const dialog = screen.getByRole("dialog");
      const ordered = focusableElements(dialog);
      expect(ordered.length).toBeGreaterThan(0);
      const first = ordered[0];
      const last = ordered[ordered.length - 1];

      for (let index = 0; index < 100; index += 1) {
        last.focus();
        pressProposalTab(dialog);
        expect(document.activeElement, `Tab ${index + 1}回目`).toBe(first);
      }
    },
  );

  it.each(KEYBOARD_PROPOSAL_STEPS)(
    "%sの段階でShift+Tabを100回循環する",
    (step) => {
      prepareKeyboardProposalStep(step);
      render(<ProposalWizard />);
      const dialog = screen.getByRole("dialog");
      const ordered = focusableElements(dialog);
      expect(ordered.length).toBeGreaterThan(0);
      const first = ordered[0];
      const last = ordered[ordered.length - 1];

      for (let index = 0; index < 100; index += 1) {
        first.focus();
        pressProposalTab(dialog, true);
        expect(document.activeElement, `Shift+Tab ${index + 1}回目`).toBe(last);
      }
    },
  );

  it.each([
    [1, 50],
    [51, 100],
  ] as const)(
    "実際のopenProposalからEscapeで閉じて起点へ戻り、背景を元へ戻す（%i〜%i回目）",
    async (firstCycle, lastCycle) => {
      render(<ProposalKeyboardHarness />);
      const trigger = screen.getByRole("button", {
        name: "提案を開く",
      }) as HTMLButtonElement;

      for (let cycle = firstCycle; cycle <= lastCycle; cycle += 1) {
        trigger.focus();
        activateButtonWithKeyboard(trigger);
        const dialog = screen.getByRole("dialog");
        expect(document.activeElement, `開始 ${cycle}回目`).toBe(
          expectedInitialProposalTarget("skeleton"),
        );
        expect(trigger.closest("[inert]"), `背景停止 ${cycle}回目`).not.toBeNull();

        await act(async () => {
          fireEvent.keyDown(dialog, { key: "Escape" });
          await Promise.resolve();
        });

        expect(screen.queryByRole("dialog"), `終了 ${cycle}回目`).toBeNull();
        expect(document.activeElement, `起点復帰 ${cycle}回目`).toBe(trigger);
        expect(trigger.closest("[inert]"), `背景復帰 ${cycle}回目`).toBeNull();
      }
    },
  );

  it("pointerとmouseを使わず4段階を操作し、適用後に起点へ戻る", async () => {
    mockProposalCandidates([makeCandidateWithSites(20, 4)]);
    let pointerDownCount = 0;
    let mouseDownCount = 0;
    const countPointerDown = () => {
      pointerDownCount += 1;
    };
    const countMouseDown = () => {
      mouseDownCount += 1;
    };
    document.addEventListener("pointerdown", countPointerDown, true);
    document.addEventListener("mousedown", countMouseDown, true);

    try {
      render(<ProposalKeyboardHarness />);
      const trigger = screen.getByRole("button", {
        name: "提案を開く",
      }) as HTMLButtonElement;
      trigger.focus();
      activateButtonWithKeyboard(trigger);

      let dialog = screen.getByRole("dialog");
      const length = expectedInitialProposalTarget("skeleton") as HTMLInputElement;
      expect(document.activeElement).toBe(length);
      const nextLength = Math.min(
        Number(length.max),
        Number(length.value) + Number(length.step),
      );
      fireEvent.keyDown(length, { key: "ArrowRight" });
      fireEvent.change(length, { target: { value: String(nextLength) } });
      fireEvent.keyUp(length, { key: "ArrowRight" });
      expect(Number(length.value)).toBe(nextLength);

      const generate = screen.getByRole("button", {
        name: "展開図を作ってもらう",
      }) as HTMLButtonElement;
      tabToProposalTarget(dialog, generate);
      activateButtonWithKeyboard(generate);
      await vi.waitFor(() =>
        expect(screen.getByRole("button", { name: "候補1" })).not.toBeNull(),
      );

      dialog = screen.getByRole("dialog");
      const candidate = screen.getByRole("button", {
        name: "候補1",
      }) as HTMLButtonElement;
      expect(document.activeElement).toBe(candidate);
      activateButtonWithKeyboard(candidate);
      const openPaper = screen.getByRole("button", {
        name: "紙の上の場所も調整",
      }) as HTMLButtonElement;
      tabToProposalTarget(dialog, openPaper);
      activateButtonWithKeyboard(openPaper);

      dialog = screen.getByRole("dialog");
      const paperHandle = expectedInitialProposalTarget("paper-position");
      expect(document.activeElement).toBe(paperHandle);
      const positionsBefore = JSON.stringify(
        useAppStore.getState().proposalPaperPositions,
      );
      fireEvent.keyDown(paperHandle, { key: "ArrowRight" });
      fireEvent.keyUp(paperHandle, { key: "ArrowRight" });
      expect(JSON.stringify(useAppStore.getState().proposalPaperPositions)).not.toBe(
        positionsBefore,
      );

      const back = screen.getByRole("button", {
        name: "候補へ戻る",
      }) as HTMLButtonElement;
      tabToProposalTarget(dialog, back);
      activateButtonWithKeyboard(back);
      dialog = screen.getByRole("dialog");
      expect(document.activeElement).toBe(
        expectedInitialProposalTarget("candidates"),
      );

      const confirm = screen.getByRole("button", {
        name: "これにする",
      }) as HTMLButtonElement;
      tabToProposalTarget(dialog, confirm);
      activateButtonWithKeyboard(confirm);
      dialog = screen.getByRole("dialog");
      expect(document.activeElement).toBe(expectedInitialProposalTarget("confirm"));

      const apply = screen.getByRole("button", {
        name: "この展開図を使う",
      }) as HTMLButtonElement;
      tabToProposalTarget(dialog, apply);
      activateButtonWithKeyboard(apply);
      await vi.waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      await act(async () => {
        await Promise.resolve();
      });

      expect(document.activeElement).toBe(trigger);
      expect(pointerDownCount).toBe(0);
      expect(mouseDownCount).toBe(0);
    } finally {
      document.removeEventListener("pointerdown", countPointerDown, true);
      document.removeEventListener("mousedown", countMouseDown, true);
    }
  });

  it("処理中のEscapeは閉じず、無効になった現在位置だけ画面本体へ退避する", () => {
    useAppStore.getState().openProposal();
    render(<ProposalWizard />);
    const dialog = screen.getByRole("dialog");
    const length = expectedInitialProposalTarget("skeleton");

    act(() => useAppStore.setState({ proposalBusy: true }));
    expect(document.activeElement).toBe(length);
    act(() => useAppStore.setState({ proposalBusy: false }));

    const generate = screen.getByRole("button", {
      name: "展開図を作ってもらう",
    });
    generate.focus();
    act(() => useAppStore.setState({ proposalBusy: true }));
    expect(generate.hasAttribute("disabled")).toBe(true);
    expect(document.activeElement).toBe(dialog);

    for (let index = 0; index < 100; index += 1) {
      fireEvent.keyDown(dialog, { key: "Escape" });
      expect(useAppStore.getState().proposalStep, `Escape ${index + 1}回目`).toBe(
        "skeleton",
      );
      expect(screen.getByRole("dialog")).toBe(dialog);
    }
  });

  it("計算中だけ進み具合を出し、終わったら消す(常設UIを増やさない)", () => {
    useAppStore.getState().openProposal();
    // 計算していないときは出さない
    const idle = render(<ProposalWizard />);
    expect(
      idle.container.querySelector("[data-proposal-progress]"),
    ).toBeNull();
    cleanup();

    // 計算中は「折り方を考えています…」と「4件中2件め」を出す
    useAppStore.setState({
      proposalBusy: true,
      proposalJobId: "visible-progress",
      proposalProgress: progressSnapshot("visible-progress", 2, 4),
      proposalProgressWarning:
        "進み具合を読み取れませんでした。計算はそのまま続けています。",
    });
    const busy = render(<ProposalWizard />);
    const box = busy.container.querySelector<HTMLElement>(
      "[data-proposal-progress]",
    );
    expect(box).not.toBeNull();
    expect(box!.textContent).toContain("折り方を考えています…");
    expect(
      box!.querySelector("[data-proposal-progress-count]")?.textContent,
    ).toBe("4件中2件め");
    // 棒の伸びも件数と合う
    expect(
      box!
        .querySelector("[data-proposal-progress-fill]")
        ?.getAttribute("data-proposal-progress-fill"),
    ).toBe("50");
    // 画面に出す文字へ内部用語を出さない(CLAUDE.md §11.1)
    expect(box!.textContent ?? "").not.toMatch(
      /job|ID|検証|ソルバー|探索|骨格|充填|節点|hard|soft|warm[\s-]+start|イテレーション|姿勢|自己交差|裂け/iu,
    );
    expect(
      busy.container.querySelector("[data-proposal-progress-warning]")
        ?.textContent,
    ).toBe("進み具合を読み取れませんでした。計算はそのまま続けています。");
    cleanup();

    // まだ件数が届いていない間は「準備中」
    useAppStore.setState({
      proposalBusy: true,
      proposalProgress: null,
      proposalProgressWarning: null,
    });
    const waiting = render(<ProposalWizard />);
    expect(
      waiting.container.querySelector("[data-proposal-progress-count]")
        ?.textContent,
    ).toBe("準備中");
    cleanup();

    // 終わったら消える
    useAppStore.setState({
      proposalBusy: false,
      proposalJobId: null,
      proposalProgress: null,
    });
    const done = render(<ProposalWizard />);
    expect(done.container.querySelector("[data-proposal-progress]")).toBeNull();
  });

  it("計算が終わったら、棒を最後まで伸ばしてから消す", async () => {
    // 最後の1件が終わってから計算が返るまでは1ミリ秒ほどしかないので、
    // 150msごとの読み取りでは最後の「4/4 件」に間に合わない。
    // 実測(2026-08-24、先端12本、最適化ありのアプリを100msごとに読んだ)では
    // 0/4→1/4→2/4→3/4 と伸びたあと、棒が **3/4のまま消えた**。
    // 利用者には棒が途中で終わったように見えるので、
    // 計算が返った時点で満杯にしてから消す。
    vi.useFakeTimers();
    const pending = deferred<ProposalJobResult>();
    let jobId = "";
    let request: Promise<void> = Promise.resolve();
    vi.mocked(ipc.proposalProgress).mockImplementation((requestedJobId) =>
      Promise.resolve(progressSnapshot(requestedJobId, 3, 4)),
    );
    vi.mocked(ipc.proposalGenerate).mockImplementation(
      (_skeleton, _paper, _seed, requestedJobId) => {
        jobId = requestedJobId;
        return pending.promise;
      },
    );
    try {
      useAppStore.getState().openProposal();
      const view = render(<ProposalWizard />);
      request = useAppStore.getState().generateProposal();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(150);
      });
      expect(
        view.container.querySelector("[data-proposal-progress-count]")
          ?.textContent,
      ).toBe("4件中3件め");

      await act(async () => {
        pending.resolve(proposalResult(jobId, []));
        await Promise.resolve();
        await Promise.resolve();
      });
      expect(
        view.container.querySelector("[data-proposal-progress-count]")
          ?.textContent,
      ).toBe("4件中4件め");
      expect(
        view.container
          .querySelector("[data-proposal-progress-fill]")
          ?.getAttribute("data-proposal-progress-fill"),
      ).toBe("100");

      await act(async () => {
        await vi.advanceTimersByTimeAsync(150);
        await request;
      });
      expect(
        view.container.querySelector("[data-proposal-progress]"),
      ).toBeNull();
    } finally {
      useAppStore.getState().closeProposal();
      if (jobId) pending.resolve(proposalResult(jobId, []));
      await request;
      vi.useRealTimers();
    }
  });

  it("閉じているときは何も出さない(常設UIを増やさない)", () => {
    render(<ProposalWizard />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("出っぱりを増やす・減らすと本数が変わる", () => {
    useAppStore.getState().openProposal();
    const { container } = render(<ProposalWizard />);
    expect(limbs(useAppStore.getState().proposalSkeleton)).toHaveLength(4);

    fireEvent.click(screen.getByRole("button", { name: "出っぱりを増やす" }));
    expect(limbs(useAppStore.getState().proposalSkeleton)).toHaveLength(5);

    const head = skeletonRows(useAppStore.getState().proposalSkeleton)[0];
    fireEvent.click(
      within(shapeRow(container, head.node.id)).getByRole("button", {
        name: "頭とその先を消す",
      }),
    );
    expect(limbs(useAppStore.getState().proposalSkeleton)).toHaveLength(4);
  });

  it("対象行からその先と、そのさらに先を足せる", () => {
    useAppStore.getState().openProposal();
    const { container } = render(<ProposalWizard />);
    const head = skeletonRows(useAppStore.getState().proposalSkeleton)[0];

    fireEvent.change(
      within(shapeRow(container, head.node.id)).getByRole("slider", {
        name: "頭の太さ",
      }),
      { target: { value: "1.8" } },
    );
    const nextId = extendFrom(container, head.node.id);
    expect(
      within(shapeRow(container, head.node.id)).queryByRole("slider", {
        name: "頭の太さ",
      }),
    ).toBeNull();
    const inheritedWidth = within(shapeRow(container, nextId)).getByRole(
      "slider",
      {
        name: "頭のその先1の太さ",
      },
    ) as HTMLInputElement;
    expect(inheritedWidth.value).toBe("1.8");
    const fartherId = extendFrom(container, nextId);
    const siblingId = extendFrom(container, head.node.id);
    const skeleton = useAppStore.getState().proposalSkeleton;

    expect(skeleton.nodes.find((node) => node.id === nextId)?.parent).toBe(
      head.node.id,
    );
    expect(skeleton.nodes.find((node) => node.id === fartherId)?.parent).toBe(
      nextId,
    );
    expect(skeleton.nodes.find((node) => node.id === siblingId)?.parent).toBe(
      head.node.id,
    );
    expect(skeleton.nodes.filter((node) => node.parent === head.node.id)).toHaveLength(
      2,
    );
    expect(shapeRow(container, nextId).textContent).toContain("その先1");
    expect(shapeRow(container, siblingId).textContent).toContain("その先2");
    expect(shapeRow(container, nextId).dataset.indentLevel).toBe("2");
    expect(shapeRow(container, fartherId).dataset.indentLevel).toBe("3");
  });

  it("先端が12本でも既存の先は延ばせるが、新しい分かれ道は増やさない", () => {
    useAppStore.getState().openProposal();
    const { container } = render(<ProposalWizard />);
    const addFromBody = screen.getByRole("button", {
      name: "出っぱりを増やす",
    }) as HTMLButtonElement;
    for (let i = 4; i < 12; i++) fireEvent.click(addFromBody);
    expect(leafNodes(useAppStore.getState().proposalSkeleton)).toHaveLength(12);
    expect(addFromBody.disabled).toBe(true);
    expect(addFromBody.title).toBe("先端は12本までです");

    const head = skeletonRows(useAppStore.getState().proposalSkeleton)[0];
    const nextId = extendFrom(container, head.node.id);
    expect(leafNodes(useAppStore.getState().proposalSkeleton)).toHaveLength(12);

    const headAdd = within(shapeRow(container, head.node.id)).getByRole(
      "button",
      { name: "頭のこの先に足す" },
    ) as HTMLButtonElement;
    const nextAdd = within(shapeRow(container, nextId)).getByRole("button", {
      name: /のこの先に足す$/u,
    }) as HTMLButtonElement;
    expect(headAdd.disabled).toBe(true);
    expect(headAdd.title).toBe("先端は12本までです");
    expect(nextAdd.disabled).toBe(false);
  });

  it("親を消すと、その先を取り残さず一緒に消す", () => {
    useAppStore.getState().openProposal();
    const { container } = render(<ProposalWizard />);
    const head = skeletonRows(useAppStore.getState().proposalSkeleton)[0];
    const nextId = extendFrom(container, head.node.id);
    const fartherId = extendFrom(container, nextId);

    fireEvent.click(
      within(shapeRow(container, head.node.id)).getByRole("button", {
        name: "頭とその先を消す",
      }),
    );

    const remaining = useAppStore.getState().proposalSkeleton.nodes;
    expect(
      remaining.filter((node) =>
        [head.node.id, nextId, fartherId].includes(node.id),
      ),
    ).toHaveLength(0);
    expect(container.querySelector(`[data-shape-row="${nextId}"]`)).toBeNull();
    expect(
      container.querySelector(`[data-shape-row="${fartherId}"]`),
    ).toBeNull();
  });

  it("深さ3以上の形をそのまま候補生成へ渡す", async () => {
    mockProposalCandidates([makeCandidate(12, 0)]);
    useAppStore.getState().openProposal();
    const { container } = render(<ProposalWizard />);
    const head = skeletonRows(useAppStore.getState().proposalSkeleton)[0];
    const nextId = extendFrom(container, head.node.id);
    const fartherId = extendFrom(container, nextId);

    fireEvent.click(screen.getByRole("button", { name: "展開図を作ってもらう" }));
    await vi.waitFor(() => expect(ipc.proposalGenerate).toHaveBeenCalledTimes(1));

    const sent = vi.mocked(ipc.proposalGenerate).mock.calls[0][0];
    expect(sent.nodes.find((node) => node.id === nextId)?.parent).toBe(
      head.node.id,
    );
    expect(sent.nodes.find((node) => node.id === fartherId)?.parent).toBe(
      nextId,
    );
  });

  it("形見本は親の終点からその先を描く", () => {
    useAppStore.getState().openProposal();
    const { container } = render(<ProposalWizard />);
    const head = skeletonRows(useAppStore.getState().proposalSkeleton)[0];
    const nextId = extendFrom(container, head.node.id);
    const parentLine = container.querySelector<SVGLineElement>(
      `[data-preview-part="${head.node.id}"]`,
    );
    const childLine = container.querySelector<SVGLineElement>(
      `[data-preview-part="${nextId}"]`,
    );
    const parentLabel = container.querySelector<SVGTextElement>(
      `[data-preview-label="${head.node.id}"]`,
    );

    expect(screen.getByRole("img", { name: "形見本" })).not.toBeNull();
    expect(parentLine).not.toBeNull();
    expect(childLine).not.toBeNull();
    expect(parentLabel).not.toBeNull();
    expect(Number(childLine!.getAttribute("x1"))).toBeCloseTo(
      Number(parentLine!.getAttribute("x2")),
      12,
    );
    expect(Number(childLine!.getAttribute("y1"))).toBeCloseTo(
      Number(parentLine!.getAttribute("y2")),
      12,
    );
    const labelX = Number(parentLabel!.getAttribute("x"));
    const labelY = Number(parentLabel!.getAttribute("y"));
    const childX1 = Number(childLine!.getAttribute("x1"));
    const childY1 = Number(childLine!.getAttribute("y1"));
    const childX2 = Number(childLine!.getAttribute("x2"));
    const childY2 = Number(childLine!.getAttribute("y2"));
    const cross = Math.abs(
      (labelX - childX1) * (childY2 - childY1) -
        (labelY - childY1) * (childX2 - childX1),
    );
    expect(cross).toBeGreaterThan(1e-9);
  });

  it("深く足しても字下げを抑えて行を折り返し、横へ隠さない", () => {
    useAppStore.getState().openProposal();
    const { container } = render(<ProposalWizard />);
    let current = skeletonRows(
      useAppStore.getState().proposalSkeleton,
    )[0].node.id;
    for (let i = 0; i < 7; i++) current = extendFrom(container, current);

    const dialog = screen.getByRole("dialog");
    const list = container.querySelector<HTMLElement>("[data-shape-list]");
    const rows = Array.from(
      container.querySelectorAll<HTMLElement>("[data-shape-row]"),
    );
    const deepest = shapeRow(container, current);
    expect(dialog.style.width).toBe("calc(100vw - 48px)");
    expect(dialog.style.maxWidth).toBe("720px");
    expect(dialog.style.boxSizing).toBe("border-box");
    expect(dialog.style.overflowX).not.toBe("hidden");
    expect(list?.style.minWidth).toBe("0px");
    expect(list?.style.overflowX).not.toBe("hidden");
    expect(deepest.style.flexWrap).toBe("wrap");
    expect(deepest.style.boxSizing).toBe("border-box");
    expect(
      Math.max(
        ...rows.map((row) => parseFloat(row.style.marginInlineStart)),
      ),
    ).toBe(48);
    const deepestName = deepest.querySelector<HTMLElement>(".limb-name");
    expect(deepestName?.style.overflowWrap).toBe("anywhere");
    expect(deepestName?.style.minWidth).toBe("0px");
    expect(deepestName?.textContent?.split("›")).toHaveLength(8);
    const deepestAdd = within(deepest).getByRole("button", {
      name: /のこの先に足す$/u,
    });
    expect(deepestAdd.style.maxWidth).toBe("100%");
    expect(deepestAdd.style.whiteSpace).toBe("normal");
    const preview = screen.getByRole("img", { name: "形見本" });
    expect(preview.style.width).toBe("100%");
    expect(preview.style.maxWidth).toBe("200px");
    for (const slider of within(deepest).getAllByRole("slider")) {
      expect(slider.style.minWidth).toBe("0px");
      expect(slider.style.maxWidth).toBe("100%");
    }

    const visibleAndNamed = [
      dialog.textContent ?? "",
      ...Array.from(
        dialog.querySelectorAll<HTMLElement>("[aria-label]"),
        (node) => node.getAttribute("aria-label") ?? "",
      ),
    ].join("\n");
    expect(visibleAndNamed).not.toMatch(/木|節点|根|深さ/u);
    expect(leafNodes(useAppStore.getState().proposalSkeleton)).toHaveLength(4);
  });

  it("長さのスライダーが骨格に反映される", () => {
    useAppStore.getState().openProposal();
    render(<ProposalWizard />);
    const slider = screen.getByLabelText("尾の長さ");
    fireEvent.change(slider, { target: { value: "2.5" } });
    expect(limbs(useAppStore.getState().proposalSkeleton)[1].length).toBe(2.5);
  });

  it("作ってもらうと候補が並び、選んで使うと展開図が流し込まれる", async () => {
    mockProposalCandidates([
      makeCandidate(10, 0),
      makeCandidate(11, 3),
    ]);
    useAppStore.setState({ doc: { ...VIEW.doc, sequence: makeSteps(1) } });
    useAppStore.getState().openProposal();
    render(<ProposalWizard />);

    fireEvent.click(screen.getByRole("button", { name: "展開図を作ってもらう" }));
    await vi.waitFor(() =>
      expect(screen.getByRole("button", { name: "候補2" })).not.toBeNull(),
    );
    expect(ipc.proposalGenerate).toHaveBeenCalledTimes(1);
    // 折りにくさは数字だけでなく日本語で添える
    expect(screen.getByRole("button", { name: "候補1" }).textContent).toContain(
      "きれいに畳めそうです",
    );

    fireEvent.click(screen.getByRole("button", { name: "候補2" }));
    expect(useAppStore.getState().proposalSelected).toBe(1);
    fireEvent.click(screen.getByRole("button", { name: "これにする" }));
    fireEvent.click(screen.getByRole("button", { name: "この展開図を使う" }));

    await vi.waitFor(() => expect(ipc.editApply).toHaveBeenCalled());
    expect(ipc.editApply).toHaveBeenCalledWith({
      type: "ReplaceCreasePattern",
      cp: makeCp(11),
    });
    // 流し込んだらダイアログは閉じる
    expect(useAppStore.getState().proposalStep).toBeNull();
  });

  it.each([{ count: 0 }, { count: 1 }, { count: 100 }])(
    "既存の折り手順が$count件のとき、適用前の注意を正しく出す",
    ({ count }) => {
      showConfirmationWithSteps(count);
      render(<ProposalWizard />);

      const notice = screen.queryByText(/今ある折り手順.*すべて消えます/u);
      if (count === 0) {
        expect(notice).toBeNull();
      } else {
        expect(notice?.textContent?.replace(/\s+/gu, "")).toBe(
          `この展開図を使うと、今ある折り手順${count}件はすべて消えます。`,
        );
      }
      expect(notice?.textContent ?? "").not.toMatch(
        /骨格|充填|ソルバー|ヤコビアン|hard|soft|warm[\s-]+start|イテレーション|節点|円の中心|ID/iu,
      );
      expect(
        (screen.getByRole("button", {
          name: "この展開図を使う",
        }) as HTMLButtonElement).disabled,
      ).toBe(false);
    },
  );

  it("確認画面で選び直すと、今の作品を変えない", () => {
    showConfirmationWithSteps(100);
    const before = JSON.stringify(useAppStore.getState().doc);
    render(<ProposalWizard />);

    fireEvent.click(screen.getByRole("button", { name: "選び直す" }));

    expect(ipc.editApply).not.toHaveBeenCalled();
    expect(useAppStore.getState().proposalStep).toBe("candidates");
    expect(JSON.stringify(useAppStore.getState().doc)).toBe(before);
  });

  it("作れなかったときは日本語の理由を出す", async () => {
    vi.mocked(ipc.proposalGenerate).mockRejectedValue(
      "紙の大きさを読み取れませんでした。作品を開き直してください。",
    );
    useAppStore.getState().openProposal();
    render(<ProposalWizard />);
    fireEvent.click(screen.getByRole("button", { name: "展開図を作ってもらう" }));
    await vi.waitFor(() =>
      expect(
        screen.getByText(
          "紙の大きさを読み取れませんでした。作品を開き直してください。",
        ),
      ).not.toBeNull(),
    );
  });

  it("別の置き方を作り直せなかったときも理由を候補画面に出す", async () => {
    const reason =
      "別の置き方を作れませんでした。形を直してから、もう一度試してください。";
    vi.mocked(ipc.proposalGenerate).mockRejectedValue(reason);
    useAppStore.setState({
      proposalStep: "candidates",
      proposalCandidates: [makeCandidate(10, 0)],
      proposalSelected: 0,
      proposalBusy: false,
      proposalError: null,
    });
    render(<ProposalWizard />);

    fireEvent.click(screen.getByRole("button", { name: "別の置き方も見る" }));

    await vi.waitFor(() => expect(screen.getByText(reason)).not.toBeNull());
    expect(screen.getByRole("button", { name: "候補1" })).not.toBeNull();
  });

  it(
    "D22: 計算中にやめた後、完了しても提案画面を再表示しない",
    async () => {
      const pending = deferred<ProposalJobResult>();
      let jobId = "";
      vi.mocked(ipc.proposalGenerate).mockImplementation(
        (_skeleton, _paper, _seed, requestedJobId) => {
          jobId = requestedJobId;
          return pending.promise;
        },
      );
      useAppStore.getState().openProposal();
      render(<ProposalWizard />);

      fireEvent.click(
        screen.getByRole("button", { name: "展開図を作ってもらう" }),
      );
      fireEvent.click(screen.getByRole("button", { name: "やめる" }));
      expect(screen.queryByRole("dialog")).toBeNull();

      await act(async () => {
        pending.resolve(proposalResult(jobId, [makeCandidate(10, 0)]));
        await pending.promise;
        await Promise.resolve();
      });
      expect(useAppStore.getState().proposalCandidates).toHaveLength(0);
      expect(useAppStore.getState().proposalStep).toBeNull();
      expect(screen.queryByRole("dialog")).toBeNull();
    },
  );

  it("D22: 取り消した100要求が逆順に完了しても画面・候補・計算中表示を戻さない", async () => {
    const pending = Array.from({ length: 100 }, () =>
      deferred<ProposalJobResult>(),
    );
    const jobIds: string[] = [];
    let issued = 0;
    vi.mocked(ipc.proposalGenerate).mockImplementation(
      (_skeleton, _paper, _seed, jobId) => {
        jobIds.push(jobId);
        return pending[issued++].promise;
      },
    );
    render(<ProposalWizard />);

    const requests: Promise<void>[] = [];
    for (let index = 0; index < pending.length; index++) {
      useAppStore.getState().openProposal();
      requests.push(useAppStore.getState().generateProposal());
      useAppStore.getState().closeProposal();
    }
    expect(issued).toBe(100);
    expect(new Set(jobIds).size).toBe(100);

    for (let index = pending.length - 1; index >= 0; index--) {
      await act(async () => {
        pending[index].resolve(
          proposalResult(jobIds[index], [makeCandidate(index + 10, 0)]),
        );
        await requests[index];
      });
      const state = useAppStore.getState();
      expect(state.proposalStep).toBeNull();
      expect(state.proposalCandidates).toHaveLength(0);
      expect(state.proposalBusy).toBe(false);
      expect(screen.queryByRole("dialog")).toBeNull();
    }
  });

  it("古い仕事の遅い進捗を、新しく開いた画面へ戻さない", async () => {
    vi.useFakeTimers();
    const generated = [
      deferred<ProposalJobResult>(),
      deferred<ProposalJobResult>(),
    ];
    const oldProgress = deferred<ProposalProgressSnapshot | null>();
    const jobIds: string[] = [];
    let issued = 0;
    let requestA: Promise<void> = Promise.resolve();
    let requestB: Promise<void> = Promise.resolve();
    vi.mocked(ipc.proposalGenerate).mockImplementation(
      (_skeleton, _paper, _seed, jobId) => {
        jobIds.push(jobId);
        return generated[issued++].promise;
      },
    );
    vi.mocked(ipc.proposalProgress).mockImplementation((jobId) =>
      jobId === jobIds[0]
        ? oldProgress.promise
        : Promise.resolve(progressSnapshot(jobId, 2, 4)),
    );
    try {
      useAppStore.getState().openProposal();
      const view = render(<ProposalWizard />);
      requestA = useAppStore.getState().generateProposal();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(150);
      });
      expect(ipc.proposalProgress).toHaveBeenCalledWith(jobIds[0]);

      useAppStore.getState().closeProposal();
      expect(vi.getTimerCount()).toBe(0);
      useAppStore.getState().openProposal();
      requestB = useAppStore.getState().generateProposal();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(150);
      });

      const count = view.container.querySelector(
        "[data-proposal-progress-count]",
      );
      expect(count?.textContent).toBe("4件中2件め");
      expect(
        view.container
          .querySelector("[data-proposal-progress-fill]")
          ?.getAttribute("data-proposal-progress-fill"),
      ).toBe("50");

      await act(async () => {
        oldProgress.resolve(progressSnapshot(jobIds[0], 4, 4));
        await oldProgress.promise;
        await Promise.resolve();
      });
      expect(count?.textContent).toBe("4件中2件め");
      const visible = view.container.textContent ?? "";
      expect(visible).not.toContain(jobIds[0]);
      expect(visible).not.toContain(jobIds[1]);
      expect(visible).not.toMatch(/job|ID/iu);

      const bPollsBefore = vi
        .mocked(ipc.proposalProgress)
        .mock.calls.filter(([jobId]) => jobId === jobIds[1]).length;
      await act(async () => {
        generated[0].resolve(
          proposalResult(jobIds[0], [makeCandidate(777, 0)]),
        );
        await requestA;
        await vi.advanceTimersByTimeAsync(150);
      });
      const state = useAppStore.getState();
      expect(state.proposalJobId).toBe(jobIds[1]);
      expect(state.proposalBusy).toBe(true);
      expect(state.proposalCandidates).toHaveLength(0);
      expect(state.proposalProgress).toEqual(
        progressSnapshot(jobIds[1], 2, 4),
      );
      expect(
        vi
          .mocked(ipc.proposalProgress)
          .mock.calls.filter(([jobId]) => jobId === jobIds[1]).length,
      ).toBe(bPollsBefore + 1);

      useAppStore.getState().closeProposal();
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      useAppStore.getState().closeProposal();
      if (jobIds[0]) generated[0].resolve(proposalResult(jobIds[0], []));
      if (jobIds[1]) generated[1].resolve(proposalResult(jobIds[1], []));
      await Promise.all([requestA, requestB]);
      vi.useRealTimers();
    }
  });

  it("提案の警告と生成エラーには内部用語を表示しない", () => {
    const internalTerms = [
      "job",
      "探索",
      "骨格",
      "充填",
      "ソルバー",
      "ヤコビアン",
      "hard",
      "soft",
      "warm start",
      "イテレーション",
      "深さ",
    ];
    const internalMessage = `${internalTerms.join(" / ")} / 角17の円 / 節点ID 42`;
    const visibleMessages = (container: HTMLElement) =>
      Array.from(
        container.querySelectorAll<HTMLElement>(".error-text, .warning-text"),
      )
        .map((node) => node.textContent ?? "")
        .join("\n");

    useAppStore.setState({
      proposalStep: "skeleton",
      proposalError: internalMessage,
    });
    const skeletonView = render(<ProposalWizard />);
    const skeletonText = visibleMessages(skeletonView.container);
    for (const term of internalTerms) {
      expect(skeletonText.toLowerCase()).not.toContain(term.toLowerCase());
    }
    expect(skeletonText).not.toContain("17");
    expect(skeletonText).not.toContain("42");
    expect(skeletonText).toContain("展開図を作ってもらう");
    skeletonView.unmount();

    useAppStore.setState({
      proposalStep: "candidates",
      proposalCandidates: [makeCandidate(10, 0)],
      proposalSelected: 0,
      proposalError: internalMessage,
    });
    const candidateView = render(<ProposalWizard />);
    const candidateText = visibleMessages(candidateView.container);
    for (const term of internalTerms) {
      expect(candidateText.toLowerCase()).not.toContain(term.toLowerCase());
    }
    expect(candidateText).not.toContain("17");
    expect(candidateText).not.toContain("42");
    expect(candidateText).toContain("形を直す");
    expect(candidateText).toContain("別の置き方も見る");
    candidateView.unmount();

    useAppStore.setState({
      proposalStep: "confirm",
      proposalCandidates: [
        { ...makeCandidate(10, 0), warnings: [internalMessage] },
      ],
      proposalSelected: 0,
      proposalError: null,
    });
    const confirmView = render(<ProposalWizard />);
    const warningText = visibleMessages(confirmView.container);
    for (const term of internalTerms) {
      expect(warningText.toLowerCase()).not.toContain(term.toLowerCase());
    }
    expect(warningText).not.toContain("17");
    expect(warningText).not.toContain("42");
    expect(warningText).toContain("選び直す");
    expect(warningText).toContain("形を直す");

    for (const opaque of [
      "triangulation node 17 failed",
      "request 42 failed",
      "[object Object]",
      "内部エラーが発生しました: 爆発した",
    ]) {
      act(() => {
        useAppStore.setState({
          proposalStep: "candidates",
          proposalError: opaque,
        });
      });
      const sanitized = visibleMessages(confirmView.container);
      expect(sanitized).not.toContain(opaque);
      expect(sanitized).not.toMatch(
        /triangulation|node|request|17|42|object|内部エラー|爆発した/iu,
      );
    }
  });

  it("折りにくさの目安は0か所なら言い換える", () => {
    expect(violationLabel(0)).toBe("きれいに畳めそうです");
    expect(violationLabel(2)).toContain("2 か所");
  });
});

describe("提案画面の初回準備表示（6-C）", () => {
  it("閉じている間は読まず、100回開いてもpayloadは最初の1回だけ読む", async () => {
    vi.resetModules();
    const proposalPayload = {
      load: vi.fn(() => new Promise<never>(() => {})),
    };

    vi.doMock("../../components/ToolRail", () => ({ ToolRail: () => null }));
    vi.doMock("../../components/ContextPanel", () => ({ ContextPanel: () => null }));
    vi.doMock("../../components/CpEditor/CpEditor", () => ({
      CpEditor: () => null,
    }));
    vi.doMock("../../components/Viewer3D/Viewer3D", () => ({
      Viewer3D: () => null,
    }));
    vi.doMock("../../components/Timeline", () => ({ Timeline: () => null }));
    vi.doMock("../../components/RecoveryDialog", () => ({
      RecoveryDialog: () => null,
    }));
    vi.doMock("../../components/PaneSplitter", () => ({
      PaneSplitter: () => null,
    }));
    vi.doMock("../../components/ContextPanelSplitter", () => ({
      ContextPanelSplitter: () => null,
    }));
    vi.doMock("../../components/dialogs/NewDocumentDialog", () => ({
      NewDocumentDialog: () => null,
    }));
    vi.doMock("../../components/HistoryButtons", () => ({
      HistoryButtons: () => null,
    }));
    vi.doMock("../../components/HistoryShortcuts", () => ({
      HistoryShortcuts: () => null,
    }));
    vi.doMock("../../components/ToolIcons", () => ({
      ToolbarIcon: () => null,
    }));
    vi.doMock("../../components/ToolbarBrandMark", () => ({
      ToolbarBrandMark: () => null,
    }));
    vi.doMock("../../components/FirstRunGuide", () => ({
      FirstRunGuide: () => null,
    }));
    vi.doMock("../../components/dialogs/HelpCenter", () => ({
      HelpCenter: () => null,
    }));
    vi.doMock("../../components/ThemeRoot", () => ({
      ThemeRoot: ({ children }: { children: unknown }) => children,
    }));
    vi.doMock("../../components/Tooltip", () => ({ TooltipHost: () => null }));
    vi.doMock("../../captureApi", () => ({ installCaptureApi: vi.fn() }));
    vi.doMock("./ProposalWizard", () => proposalPayload.load());

    const [{ default: App }, { useAppStore: appStore }] = await Promise.all([
      import("../../App"),
      import("../../store/appStore"),
    ]);
    appStore.setState({
      proposalStep: null,
      newDocument: vi.fn().mockResolvedValue(undefined),
      checkRecovery: vi.fn().mockResolvedValue(undefined),
    });

    render(<App />);
    const open = screen.getByRole("button", { name: "提案" });
    expect(proposalPayload.load).not.toHaveBeenCalled();

    for (let index = 0; index < 100; index += 1) {
      fireEvent.click(open);
      if (index === 0) {
        await vi.waitFor(() =>
          expect(proposalPayload.load).toHaveBeenCalledTimes(1),
        );
        expect(screen.getByRole("status").textContent).toBe(
          "提案の準備をしています…",
        );
      }
      appStore.getState().closeProposal();
    }

    expect(proposalPayload.load).toHaveBeenCalledTimes(1);
  });
});

// ---------------------------------------------------------------------------
// 完成形での先端の場所を、見本の絵の上で直接動かす(PRO-006〜PRO-008)
// ---------------------------------------------------------------------------

/** 見本を置く画面上の位置。左右上下に余白のない正方形にする。 */
const PREVIEW_RECT = { left: 100, top: 50, width: 200, height: 200 };
/**
 * 胴から長さ1・太さ1で出した形の手計算値。
 * 届く距離 = 太さの半分(1×1×0.5) + 長さ1 = 1.5 → 場所1.0が1.5にあたる。
 * 枠はその1.25倍で1.875。200px四方へ収めるので1目盛りは 200/3.75 px。
 */
const POSITION_RADIUS = 1.5;
const FRAME_RADIUS = 1.875;
const PIXELS_PER_UNIT = 200 / (2 * FRAME_RADIUS);

/** 胴から等間隔にcount本だけ出した形 */
function radialSkeleton(count: number): Skeleton {
  return {
    nodes: [
      { id: 0, parent: null, length: 0, width_factor: 1 },
      ...Array.from({ length: count }, (_, index) => ({
        id: index + 1,
        parent: 0,
        length: 1,
        width_factor: 1,
      })),
    ],
  };
}

/** 見本のSVGを取り出し、画面上の大きさを固定する。 */
function previewSvg(container: HTMLElement): SVGSVGElement {
  const svg = container.querySelector<SVGSVGElement>("svg.skeleton-preview");
  expect(svg).not.toBeNull();
  svg!.getBoundingClientRect = () =>
    ({
      ...PREVIEW_RECT,
      right: PREVIEW_RECT.left + PREVIEW_RECT.width,
      bottom: PREVIEW_RECT.top + PREVIEW_RECT.height,
      x: PREVIEW_RECT.left,
      y: PREVIEW_RECT.top,
      toJSON: () => ({}),
    }) as DOMRect;
  return svg!;
}

/**
 * 描かれた絵を、書き並べる順番によらない形にしてから比べる。
 * 同じ絵でも、属性を消して付け直すと文字列の並びだけが変わるため。
 * 「いまどれを選んでいるか」の輪は形ではないので、比べる対象から外す。
 */
function drawingSignature(svg: SVGSVGElement): string {
  return [svg, ...Array.from(svg.querySelectorAll("*"))]
    .filter((element) => !element.hasAttribute("data-tip-focus-ring"))
    .map((element) => {
      const attributes = Array.from(element.attributes)
        .map((attribute) => `${attribute.name}=${attribute.value}`)
        .sort()
        .join(",");
      const text = element.tagName === "text" ? (element.textContent ?? "") : "";
      return `${element.tagName}{${attributes}}${text}`;
    })
    .join("|");
}

const PAPER_RECT = { left: 20, top: 30, width: 560, height: 560 };
const PAPER_VIEWBOX_MIN = -0.06;
const PAPER_VIEWBOX_SIZE = 1.12;

/** 紙位置の大きなSVGを取り出し、画面上の大きさを固定する。 */
function paperEditorSvg(container: HTMLElement): SVGSVGElement {
  const svg = container.querySelector<SVGSVGElement>(
    "svg.paper-position-editor",
  );
  expect(svg).not.toBeNull();
  svg!.getBoundingClientRect = () =>
    ({
      ...PAPER_RECT,
      right: PAPER_RECT.left + PAPER_RECT.width,
      bottom: PAPER_RECT.top + PAPER_RECT.height,
      x: PAPER_RECT.left,
      y: PAPER_RECT.top,
      toJSON: () => ({}),
    }) as DOMRect;
  return svg!;
}

/** 正方形の紙で、中心・長辺基準の場所に対応するクライアント座標。 */
function paperClientFor(position: { x: number; y: number }) {
  const paperX = 0.5 + position.x / 2;
  const svgY = -(0.5 + position.y / 2);
  return {
    clientX:
      PAPER_RECT.left +
      ((paperX - PAPER_VIEWBOX_MIN) / PAPER_VIEWBOX_SIZE) * PAPER_RECT.width,
    clientY:
      PAPER_RECT.top +
      ((svgY - (-1.06)) / PAPER_VIEWBOX_SIZE) * PAPER_RECT.height,
  };
}

function paperHandle(container: HTMLElement, id: number): SVGCircleElement {
  const handle = container.querySelector<SVGCircleElement>(
    `[data-paper-position-handle="${id}"]`,
  );
  expect(handle).not.toBeNull();
  return handle!;
}

function storedPaperPosition(id: number) {
  return (
    useAppStore
      .getState()
      .proposalPaperPositions.find((entry) => entry.leaf_id === id)?.position ??
    null
  );
}

function dragPaperTip(
  container: HTMLElement,
  id: number,
  position: { x: number; y: number },
  pointerId = 1,
) {
  const svg = paperEditorSvg(container);
  const at = paperClientFor(position);
  fireEvent.pointerDown(paperHandle(container, id), { pointerId, ...at });
  fireEvent.pointerMove(svg, { pointerId, ...at });
  fireEvent.pointerUp(svg, { pointerId });
}

function expectSentPaperPosition(
  sent: Skeleton,
  id: number,
  expected: { x: number; y: number },
  label: string,
) {
  const position = sent.nodes.find((node) => node.id === id)?.tip_pos_2d;
  expect(position, label).not.toBeNull();
  expect(
    Math.abs((position?.x ?? 0) - expected.x),
    `${label}の横`,
  ).toBeLessThanOrEqual(1e-6);
  expect(
    Math.abs((position?.y ?? 0) - expected.y),
    `${label}の縦`,
  ).toBeLessThanOrEqual(1e-6);
}

/** 場所の指定(-1.0〜1.0)に対応する画面上の位置。テスト側で独立に計算する。 */
function clientFor(pos: { x: number; y: number }) {
  return {
    clientX:
      PREVIEW_RECT.left + (pos.x * POSITION_RADIUS + FRAME_RADIUS) * PIXELS_PER_UNIT,
    clientY:
      PREVIEW_RECT.top + (-pos.y * POSITION_RADIUS + FRAME_RADIUS) * PIXELS_PER_UNIT,
  };
}

function tipHandle(container: HTMLElement, id: number): SVGCircleElement {
  const handle = container.querySelector<SVGCircleElement>(
    `[data-tip-handle="${id}"]`,
  );
  expect(handle).not.toBeNull();
  return handle!;
}

function storedTipPos(id: number) {
  return (
    useAppStore
      .getState()
      .proposalSkeleton.nodes.find((node) => node.id === id)?.tip_pos_2d ?? null
  );
}

/** つまんで指定した場所まで動かし、放す。 */
function dragTip(
  container: HTMLElement,
  id: number,
  pos: { x: number; y: number },
  pointerId = 1,
) {
  const svg = previewSvg(container);
  const at = clientFor(pos);
  fireEvent.pointerDown(tipHandle(container, id), { pointerId, ...at });
  fireEvent.pointerMove(svg, { pointerId, ...at });
  fireEvent.pointerUp(svg, { pointerId });
}

/** 骨格に入った場所と、画面に描かれた場所が同じ点を指しているか。 */
function expectTipAt(
  container: HTMLElement,
  id: number,
  pos: { x: number; y: number },
  label: string,
) {
  const stored = storedTipPos(id);
  expect(stored, label).not.toBeNull();
  expect(Math.abs(stored!.x - pos.x), `${label} の横`).toBeLessThan(1e-9);
  expect(Math.abs(stored!.y - pos.y), `${label} の縦`).toBeLessThan(1e-9);

  const handle = tipHandle(container, id);
  expect(
    Math.abs(Number(handle.getAttribute("cx")) - stored!.x * POSITION_RADIUS),
    `${label} の画面上の横`,
  ).toBeLessThan(1e-9);
  // SVGは下が正なので、上向きの場所とは符号が逆になる
  expect(
    Math.abs(Number(handle.getAttribute("cy")) + stored!.y * POSITION_RADIUS),
    `${label} の画面上の縦`,
  ).toBeLessThan(1e-9);
  expect(handle.dataset.tipDecided, label).toBe("true");
}

describe("完成形の先端の場所を直接動かす(PRO-008)", () => {
  it("つまんで動かした場所が骨格に入り、画面上の位置と一致する(20操作)", () => {
    useAppStore.getState().openProposal();
    const { container } = render(<ProposalWizard />);
    const ids = leafNodes(useAppStore.getState().proposalSkeleton).map(
      (node) => node.id,
    );
    expect(ids).toHaveLength(4);

    const targets = [
      { x: 0, y: 0 },
      { x: 0.5, y: 0.25 },
      { x: -0.5, y: 0.75 },
      { x: 0.125, y: -0.875 },
      { x: -0.25, y: -0.25 },
    ];
    let done = 0;
    for (const pos of targets) {
      for (const id of ids) {
        // 先端ごとに少しずつ違う場所へ運び、取り違えていれば落ちるようにする
        const moved = { x: pos.x, y: pos.y - id / 40 };
        dragTip(container, id, moved, id);
        expectTipAt(container, id, moved, `操作${done + 1}(先端${id})`);
        done += 1;
      }
    }
    expect(done).toBe(20);
  });

  it("先端が1本から12本までのすべてで動かせる", () => {
    let moved = 0;
    for (let count = 1; count <= 12; count += 1) {
      useAppStore.getState().openProposal();
      useAppStore.setState({ proposalSkeleton: radialSkeleton(count) });
      const view = render(<ProposalWizard />);
      const target = { x: 0.5 - count / 24, y: -0.25 + count / 24 };
      // いちばん後ろの先端(数が増えるほど混み合う場所)を動かす
      dragTip(view.container, count, target, count);
      expectTipAt(view.container, count, target, `${count}本立て`);
      expect(
        view.container.querySelectorAll("[data-tip-handle]"),
        `${count}本立てのつまみ`,
      ).toHaveLength(count);
      moved += 1;
      view.unmount();
    }
    expect(moved).toBe(12);
  });

  it("枠の外へ引っぱっても -1.0〜1.0 の外へは出ない", () => {
    useAppStore.getState().openProposal();
    const { container } = render(<ProposalWizard />);
    const svg = previewSvg(container);
    const corners = [
      { pulled: { x: 12, y: 9 }, expected: { x: 1, y: 1 } },
      { pulled: { x: -12, y: 9 }, expected: { x: -1, y: 1 } },
      { pulled: { x: 12, y: -9 }, expected: { x: 1, y: -1 } },
      { pulled: { x: -12, y: -9 }, expected: { x: -1, y: -1 } },
    ];
    for (const [index, { pulled, expected }] of corners.entries()) {
      const id = index + 1;
      fireEvent.pointerDown(tipHandle(container, id), {
        pointerId: id,
        ...clientFor({ x: 0, y: 0 }),
      });
      // 画面の外まで引っぱっても、はみ出した分は縁で止める
      fireEvent.pointerMove(svg, { pointerId: id, ...clientFor(pulled) });
      fireEvent.pointerUp(svg, { pointerId: id });
      expectTipAt(container, id, expected, `外へ引っぱった${index + 1}`);
    }
    // 縁を越えた値は1つも入らない
    for (const node of useAppStore.getState().proposalSkeleton.nodes) {
      if (!node.tip_pos_2d) continue;
      expect(Math.abs(node.tip_pos_2d.x)).toBeLessThanOrEqual(1);
      expect(Math.abs(node.tip_pos_2d.y)).toBeLessThanOrEqual(1);
    }
  });

  it("矢印キーでも動かせ、DeleteとBackSpaceで自動へ戻る(20操作)", () => {
    useAppStore.getState().openProposal();
    const { container } = render(<ProposalWizard />);
    const handle = tipHandle(container, 1);
    // 胴の真上から出た先端の自動の場所(手計算: 角度π/2、長さ1)
    let x = Math.cos(Math.PI / 2) / POSITION_RADIUS;
    let y = Math.sin(Math.PI / 2) / POSITION_RADIUS;
    const press = (key: string, shiftKey = false) =>
      fireEvent.keyDown(handle, { key, shiftKey });

    let done = 0;
    for (let i = 0; i < 4; i += 1) {
      press("ArrowRight");
      x = Math.min(1, x + 0.05);
      expectTipAt(container, 1, { x, y }, `右${i + 1}`);
      press("ArrowDown");
      y = Math.max(-1, y - 0.05);
      expectTipAt(container, 1, { x, y }, `下${i + 1}`);
      press("ArrowLeft", true);
      x = Math.max(-1, x - 0.2);
      expectTipAt(container, 1, { x, y }, `左(大)${i + 1}`);
      press("ArrowUp", true);
      y = Math.min(1, y + 0.2);
      expectTipAt(container, 1, { x, y }, `上(大)${i + 1}`);
      done += 4;
    }
    expect(done).toBe(16);

    press("Delete");
    expect(storedTipPos(1)).toBeNull();
    press("ArrowRight");
    expect(storedTipPos(1)).not.toBeNull();
    press("Backspace");
    expect(storedTipPos(1)).toBeNull();
    // 決めていない先端でも矢印キーで決め始められる
    press("ArrowUp");
    expect(storedTipPos(1)).not.toBeNull();
    expect(done + 4).toBe(20);
  });

  it("場所を決めた先端と決めていない先端を見分けられる", () => {
    useAppStore.getState().openProposal();
    const { container } = render(<ProposalWizard />);
    const before = tipHandle(container, 1);
    expect(before.dataset.tipDecided).toBe("false");
    expect(before.getAttribute("fill")).toBe("var(--color-surface)");
    expect(before.getAttribute("stroke")).toBe("#3b6fc9");
    const undecidedRadius = Number(before.getAttribute("r"));
    expect(before.getAttribute("aria-label")).toBe("頭を出したい場所（自動）");
    expect(
      screen.queryByRole("button", { name: "頭の場所を自動に戻す" }),
    ).toBeNull();

    dragTip(container, 1, { x: 0.4, y: -0.2 });

    const after = tipHandle(container, 1);
    expect(after.dataset.tipDecided).toBe("true");
    expect(after.getAttribute("fill")).toBe("var(--color-accent)");
    // 色が分かりにくい場合でも大きさで見分けられる
    expect(Number(after.getAttribute("r"))).toBeGreaterThan(undecidedRadius);
    expect(after.getAttribute("aria-label")).toBe("頭を出したい場所（決めました）");
    expect(
      screen.getByRole("button", { name: "頭の場所を自動に戻す" }),
    ).not.toBeNull();
    // 決めていない先端は見分けがつくまま
    expect(tipHandle(container, 2).dataset.tipDecided).toBe("false");
    expect(
      screen.queryByRole("button", { name: "尾の場所を自動に戻す" }),
    ).toBeNull();
  });

  it("場所を自動に戻すと、決める前と完全に同じ形へ戻る", () => {
    useAppStore.getState().openProposal();
    const { container } = render(<ProposalWizard />);
    const svg = previewSvg(container);
    const skeletonBefore = JSON.stringify(
      useAppStore.getState().proposalSkeleton,
    );
    const drawingBefore = drawingSignature(svg);
    const inputsBefore = container.querySelectorAll("input").length;

    dragTip(container, 1, { x: 0.4, y: -0.2 });
    dragTip(container, 3, { x: -0.6, y: 0.8 });
    expect(JSON.stringify(useAppStore.getState().proposalSkeleton)).not.toBe(
      skeletonBefore,
    );
    // 場所を決めても、決める項目(スライダー等)は1つも増えない
    expect(container.querySelectorAll("input")).toHaveLength(inputsBefore);

    fireEvent.click(screen.getByRole("button", { name: "頭の場所を自動に戻す" }));
    fireEvent.click(
      screen.getByRole("button", { name: "右前足の場所を自動に戻す" }),
    );

    const skeletonAfter = JSON.stringify(
      useAppStore.getState().proposalSkeleton,
    );
    expect(skeletonAfter).toBe(skeletonBefore);
    expect(skeletonAfter).not.toContain("tip_pos_2d");
    expect(drawingSignature(previewSvg(container))).toBe(drawingBefore);
  });

  it("決めた場所は、そのまま展開図の注文へ載る", async () => {
    mockProposalCandidates([makeCandidate(20, 0)]);
    useAppStore.getState().openProposal();
    const { container } = render(<ProposalWizard />);
    dragTip(container, 1, { x: 0.25, y: 0.5 });
    dragTip(container, 2, { x: -0.75, y: -0.5 });

    fireEvent.click(screen.getByRole("button", { name: "展開図を作ってもらう" }));
    await vi.waitFor(() => expect(ipc.proposalGenerate).toHaveBeenCalledTimes(1));

    const sent = vi.mocked(ipc.proposalGenerate).mock.calls[0][0];
    const head = sent.nodes.find((node) => node.id === 1)!;
    const tail = sent.nodes.find((node) => node.id === 2)!;
    expect(Math.abs((head.tip_pos_2d?.x ?? 0) - 0.25)).toBeLessThan(1e-9);
    expect(Math.abs((head.tip_pos_2d?.y ?? 0) - 0.5)).toBeLessThan(1e-9);
    expect(Math.abs((tail.tip_pos_2d?.x ?? 0) + 0.75)).toBeLessThan(1e-9);
    expect(Math.abs((tail.tip_pos_2d?.y ?? 0) + 0.5)).toBeLessThan(1e-9);
    // 決めていない先端には場所を作らない
    expect(sent.nodes.find((node) => node.id === 3)?.tip_pos_2d ?? null).toBeNull();
  });

  it("選んでいる先端の目印を、絵の中の長さで描く", () => {
    useAppStore.getState().openProposal();
    const { container } = render(<ProposalWizard />);
    expect(container.querySelectorAll("[data-tip-focus-ring]")).toHaveLength(0);

    const handle = tipHandle(container, 1);
    fireEvent.focus(handle);
    const ring = container.querySelector<SVGCircleElement>(
      '[data-tip-focus-ring="1"]',
    );
    expect(ring).not.toBeNull();
    // 目印は見本の枠(半径1.875)の中に収まる大きさで、つまみより一回り大きい
    const ringRadius = Number(ring!.getAttribute("r"));
    expect(ringRadius).toBeGreaterThan(Number(handle.getAttribute("r")));
    expect(ringRadius).toBeLessThan(1.875 * 0.25);
    expect(Number(ring!.getAttribute("stroke-width"))).toBeLessThan(0.1);
    expect(ring!.getAttribute("cx")).toBe(handle.getAttribute("cx"));
    expect(ring!.getAttribute("cy")).toBe(handle.getAttribute("cy"));
    // 目印は1つだけ。別の先端へ移ると前の目印は消える
    fireEvent.blur(handle);
    fireEvent.focus(tipHandle(container, 2));
    expect(container.querySelectorAll("[data-tip-focus-ring]")).toHaveLength(1);
    expect(
      container.querySelector('[data-tip-focus-ring="2"]'),
    ).not.toBeNull();
  });

  it("場所を動かす案内に、画面で使わない言葉を出さない", () => {
    useAppStore.getState().openProposal();
    const { container } = render(<ProposalWizard />);
    dragTip(container, 1, { x: 0.4, y: -0.2 });
    const dialog = screen.getByRole("dialog");
    const visibleAndNamed = [
      dialog.textContent ?? "",
      ...Array.from(
        dialog.querySelectorAll<HTMLElement>("[aria-label]"),
        (node) => node.getAttribute("aria-label") ?? "",
      ),
    ].join("\n");

    expect(visibleAndNamed).toContain("つまんで動かせます");
    expect(visibleAndNamed).not.toMatch(/木|節点|根|深さ|座標|投影|相対/u);
  });

  describe("紙の上の場所を大きな別画面で動かす(作業12)", () => {
    it("小さい4候補の絵はつまみ0個で、操作要素の入れ子も0件", () => {
      useAppStore.setState({
        proposalStep: "candidates",
        proposalSkeleton: radialSkeleton(4),
        proposalCandidates: Array.from({ length: 4 }, (_, index) =>
          makeCandidateWithSites(20 + index, 4),
        ),
        proposalSelected: 0,
        proposalPaperSource: null,
        proposalPaperPositions: [],
        proposalBusy: false,
        proposalError: null,
      });
      const { container } = render(<ProposalWizard />);
      const candidates = container.querySelectorAll<HTMLButtonElement>(
        "button.candidate",
      );
      expect(candidates).toHaveLength(4);

      const interactive =
        'button, a[href], input, select, textarea, summary, [role="button"], [role="link"], [role="slider"], [tabindex]:not([tabindex="-1"])';
      for (const [index, candidate] of [...candidates].entries()) {
        expect(
          candidate.querySelectorAll("[data-paper-position-handle]"),
          `小候補${index + 1}の紙位置つまみ`,
        ).toHaveLength(0);
        expect(
          candidate.querySelectorAll("[data-tip-handle]"),
          `小候補${index + 1}の完成位置つまみ`,
        ).toHaveLength(0);
        expect(
          candidate.querySelectorAll(interactive),
          `候補ボタン${index + 1}の中の操作要素`,
        ).toHaveLength(0);
      }
      for (const element of container.querySelectorAll<HTMLElement>(interactive)) {
        expect(element.parentElement?.closest(interactive) ?? null).toBeNull();
      }

      fireEvent.click(
        screen.getByRole("button", { name: "紙の上の場所も調整" }),
      );
      expect(useAppStore.getState().proposalStep).toBe("paper-position");
      expect(container.querySelectorAll("button.candidate")).toHaveLength(0);
      expect(container.querySelectorAll("[data-paper-position-handle]")).toHaveLength(
        4,
      );
      for (const element of container.querySelectorAll<HTMLElement>(interactive)) {
        expect(element.parentElement?.closest(interactive) ?? null).toBeNull();
      }
    });

    it("壊れた候補に余分な対応があっても大きな画面のつまみは最大12個", () => {
      useAppStore.setState({
        proposalStep: "candidates",
        proposalSkeleton: radialSkeleton(12),
        proposalCandidates: [makeCandidateWithSites(30, 13)],
        proposalSelected: 0,
        proposalPaperSource: null,
        proposalPaperPositions: [],
        proposalBusy: false,
        proposalError: null,
      });
      const { container } = render(<ProposalWizard />);
      fireEvent.click(
        screen.getByRole("button", { name: "紙の上の場所も調整" }),
      );
      expect(container.querySelectorAll("[data-paper-position-handle]")).toHaveLength(
        12,
      );
      for (let id = 1; id <= 12; id += 1) {
        expect(paperHandle(container, id)).not.toBeNull();
      }
      expect(
        container.querySelector('[data-paper-position-handle="13"]'),
      ).toBeNull();
    });

    it("引き出し線を全つまみより後ろへまとめ、各つまみに標準の名前を持たせる", () => {
      useAppStore.setState({
        proposalStep: "candidates",
        proposalSkeleton: radialSkeleton(12),
        proposalCandidates: [makeCandidateWithSites(34, 12)],
        proposalSelected: 0,
        proposalPaperSource: null,
        proposalPaperPositions: [],
        proposalBusy: false,
        proposalError: null,
      });
      const { container } = render(<ProposalWizard />);
      fireEvent.click(
        screen.getByRole("button", { name: "紙の上の場所も調整" }),
      );
      // 同じ場所へ集め、名前をずらす引き出し線が必ず出る標本にする。
      act(() => {
        useAppStore.setState({
          proposalPaperPositions: Array.from({ length: 12 }, (_, index) => ({
            leaf_id: index + 1,
            position: { x: 0, y: 0 },
          })),
        });
      });

      const editor = container.querySelector<SVGSVGElement>(
        '[data-paper-position-editor="large"]',
      );
      const leaders = Array.from(
        editor?.querySelectorAll<SVGLineElement>(
          ".paper-position-label-leader",
        ) ?? [],
      );
      const handles = Array.from(
        editor?.querySelectorAll<SVGCircleElement>(
          "[data-paper-position-handle]",
        ) ?? [],
      );
      const labels = Array.from(
        editor?.querySelectorAll<SVGTextElement>(".paper-position-label") ?? [],
      );
      expect(leaders.length).toBeGreaterThan(0);
      expect(handles).toHaveLength(12);
      for (const leader of leaders) {
        for (const foreground of [...handles, ...labels]) {
          expect(
            leader.compareDocumentPosition(foreground) &
              Node.DOCUMENT_POSITION_FOLLOWING,
          ).not.toBe(0);
        }
      }
      for (const handle of handles) {
        expect(handle.hasAttribute("data-tooltip")).toBe(false);
        const title = handle.querySelector("title");
        expect(title?.textContent).not.toBe("");
        expect(handle.getAttribute("aria-label")).toContain(title?.textContent);
      }
    });

    it("1000×700向けの左列へ紙全体を置き、右列へ説明と操作を固定する", () => {
      useAppStore.setState({
        proposalStep: "candidates",
        proposalSkeleton: radialSkeleton(12),
        proposalCandidates: [makeCandidateWithSites(35, 12)],
        proposalSelected: 0,
        proposalPaperSource: null,
        proposalPaperPositions: [],
        proposalBusy: false,
        proposalError: null,
      });
      const { container } = render(<ProposalWizard />);
      fireEvent.click(
        screen.getByRole("button", { name: "紙の上の場所も調整" }),
      );

      const dialog = screen.getByRole("dialog");
      const step = container.querySelector(".paper-position-step");
      const sidebar = container.querySelector(".paper-position-sidebar");
      const stage = container.querySelector(".paper-position-stage");
      expect(dialog.getAttribute("data-proposal-step")).toBe("paper-position");
      expect(dialog.getAttribute("aria-labelledby")).toBe("proposal-title");
      expect(dialog.style.maxWidth).toBe("960px");
      expect(step).not.toBeNull();
      expect(sidebar).not.toBeNull();
      expect(stage).not.toBeNull();
      expect(stage?.querySelector('[data-paper-position-editor="large"]')).not.toBeNull();
      expect(stage?.querySelectorAll("[data-paper-position-handle]")).toHaveLength(12);
      // stage自身を操作要素にはせず、紙全体の12個のつまみだけを直接触る。
      expect(stage?.hasAttribute("tabindex")).toBe(false);
      expect(sidebar?.querySelector("#proposal-title")?.textContent).toBe(
        "紙の上の場所を調整",
      );
      for (const name of [
        "候補へ戻る",
        "この候補の場所に戻す",
        "この場所で作り直す",
      ]) {
        const action = screen.getByRole("button", { name });
        expect(stage?.contains(action), name).toBe(false);
        expect(step?.contains(action), name).toBe(true);
        expect(sidebar?.contains(action), name).toBe(false);
      }
    });

    it("正方形・横長・縦長の紙を560px領域へ全体表示する", () => {
      for (const [index, [width, height]] of [
        [1, 1],
        [1, 0.05],
        [0.05, 1],
      ].entries()) {
        cleanup();
        useAppStore.setState({
          proposalStep: "candidates",
          proposalSkeleton: radialSkeleton(12),
          proposalCandidates: [
            makeSizedCandidateWithSites(90 + index, 12, width, height),
          ],
          proposalSelected: 0,
          proposalPaperSource: null,
          proposalPaperPositions: [],
          proposalBusy: false,
          proposalError: null,
        });
        const { container } = render(<ProposalWizard />);
        fireEvent.click(
          screen.getByRole("button", { name: "紙の上の場所も調整" }),
        );
        const stage = container.querySelector<HTMLElement>(".paper-position-stage");
        const editor = container.querySelector<SVGSVGElement>(
          '[data-paper-position-editor="large"]',
        );
        const bottomHandle = container.querySelector<SVGCircleElement>(
          '[data-paper-position-handle="12"]',
        );
        expect(stage).not.toBeNull();
        expect(editor).not.toBeNull();
        expect(bottomHandle).not.toBeNull();

        const viewBox = (editor?.getAttribute("viewBox") ?? "")
          .split(/\s+/u)
          .map(Number);
        expect(viewBox).toHaveLength(4);
        const [viewX, viewY, viewWidth, viewHeight] = viewBox;
        expect([viewX, viewY, viewWidth, viewHeight].every(Number.isFinite)).toBe(
          true,
        );
        const editorWidth = Number.parseFloat(editor?.style.maxWidth ?? "");
        const contentHeight = editorWidth * (viewHeight / viewWidth);
        expect(Math.max(editorWidth, contentHeight)).toBeCloseTo(560, 9);
        expect(editorWidth).toBeLessThanOrEqual(560);
        expect(contentHeight).toBeLessThanOrEqual(560);
        const scale = contentHeight / viewHeight;
        const cy = Number(bottomHandle?.getAttribute("cy"));
        const radius = Number(bottomHandle?.getAttribute("r"));
        const visibleTop = (cy - radius - viewY) * scale;
        const visibleBottom = (cy + radius - viewY) * scale;
        expect(visibleTop).toBeGreaterThanOrEqual(-1e-9);
        expect(visibleBottom).toBeLessThanOrEqual(contentHeight + 1e-9);
      }
    });

    it("マウス20操作のすべてが再計算へ渡る入力と1e-6以内で一致する", async () => {
      // 候補0件なら同じ大画面に留まる。各操作の直後に本番ストア経路で
      // 再計算を1件ずつ送り、テスト用の純関数で代用しない。
      mockProposalCandidates([]);
      useAppStore.setState({
        proposalStep: "candidates",
        proposalSkeleton: radialSkeleton(4),
        proposalCandidates: [makeCandidateWithSites(40, 4)],
        proposalSelected: 0,
        proposalPaperSource: null,
        proposalPaperPositions: [],
        proposalBusy: false,
        proposalError: null,
      });
      const { container } = render(<ProposalWizard />);
      fireEvent.click(
        screen.getByRole("button", { name: "紙の上の場所も調整" }),
      );
      paperEditorSvg(container);

      const targets = [
        { x: -0.6, y: -0.5 },
        { x: -0.3, y: 0.4 },
        { x: 0, y: 0 },
        { x: 0.45, y: -0.3 },
        { x: 0.7, y: 0.6 },
      ];
      let done = 0;
      for (const target of targets) {
        for (let id = 1; id <= 4; id += 1) {
          const expected = { x: target.x, y: target.y - id / 100 };
          dragPaperTip(container, id, expected, done + 1);
          const stored = storedPaperPosition(id);
          expect(stored).not.toBeNull();
          expect(Math.abs((stored?.x ?? 0) - expected.x)).toBeLessThanOrEqual(
            1e-6,
          );
          expect(Math.abs((stored?.y ?? 0) - expected.y)).toBeLessThanOrEqual(
            1e-6,
          );
          await act(async () => {
            await useAppStore.getState().generateProposalFromPaperPositions();
          });
          expect(ipc.proposalGenerate).toHaveBeenCalledTimes(done + 1);
          expect(useAppStore.getState().proposalBusy).toBe(false);
          expectSentPaperPosition(
            vi.mocked(ipc.proposalGenerate).mock.calls[done][0],
            id,
            expected,
            `マウス操作${done + 1}`,
          );
          done += 1;
        }
      }
      expect(done).toBe(20);
      expect(ipc.proposalGenerate).toHaveBeenCalledTimes(20);
    });

    it("キーボード20操作のすべてが再計算へ渡る入力と1e-6以内で一致する", async () => {
      mockProposalCandidates([]);
      useAppStore.setState({
        proposalStep: "candidates",
        proposalSkeleton: radialSkeleton(4),
        proposalCandidates: [makeCandidateWithSites(50, 4)],
        proposalSelected: 0,
        proposalPaperSource: null,
        proposalPaperPositions: [],
        proposalBusy: false,
        proposalError: null,
      });
      const { container } = render(<ProposalWizard />);
      fireEvent.click(
        screen.getByRole("button", { name: "紙の上の場所も調整" }),
      );

      let expected = { x: -0.75, y: -2 / 3 };
      const operations = [
        { key: "ArrowRight", dx: 0.02, dy: 0, shiftKey: false },
        { key: "ArrowUp", dx: 0, dy: 0.02, shiftKey: false },
        { key: "ArrowLeft", dx: -0.1, dy: 0, shiftKey: true },
        { key: "ArrowDown", dx: 0, dy: -0.1, shiftKey: true },
      ];
      let done = 0;
      for (let round = 0; round < 5; round += 1) {
        for (const operation of operations) {
          fireEvent.keyDown(paperHandle(container, 1), {
            key: operation.key,
            shiftKey: operation.shiftKey,
          });
          expected = {
            x: Math.max(-1, Math.min(1, expected.x + operation.dx)),
            y: Math.max(-1, Math.min(1, expected.y + operation.dy)),
          };
          await act(async () => {
            await useAppStore.getState().generateProposalFromPaperPositions();
          });
          expect(ipc.proposalGenerate).toHaveBeenCalledTimes(done + 1);
          expect(useAppStore.getState().proposalBusy).toBe(false);
          expectSentPaperPosition(
            vi.mocked(ipc.proposalGenerate).mock.calls[done][0],
            1,
            expected,
            `キーボード操作${done + 1}`,
          );
          done += 1;
        }
      }
      expect(done).toBe(20);
      expect(ipc.proposalGenerate).toHaveBeenCalledTimes(20);
    });

    it("再計算中は表示中の場所を動かさず、押した時点の入力と食い違わない", async () => {
      const pending = deferred<ProposalJobResult>();
      let jobId = "";
      vi.mocked(ipc.proposalGenerate).mockImplementation(
        (_skeleton, _paper, _seed, requestedJobId) => {
          jobId = requestedJobId;
          return pending.promise;
        },
      );
      useAppStore.setState({
        proposalStep: "candidates",
        proposalSkeleton: radialSkeleton(4),
        proposalCandidates: [makeCandidateWithSites(55, 4)],
        proposalSelected: 0,
        proposalPaperSource: null,
        proposalPaperPositions: [],
        proposalBusy: false,
        proposalError: null,
      });
      const { container } = render(<ProposalWizard />);
      fireEvent.click(
        screen.getByRole("button", { name: "紙の上の場所も調整" }),
      );
      dragPaperTip(container, 1, { x: 0.2, y: -0.2 });
      const sentAt = storedPaperPosition(1);
      expect(sentAt).not.toBeNull();

      fireEvent.click(
        screen.getByRole("button", { name: "この場所で作り直す" }),
      );
      expect(useAppStore.getState().proposalBusy).toBe(true);
      expect(
        screen.getByRole("button", { name: "候補へ戻る" }).hasAttribute("disabled"),
      ).toBe(true);
      expect(
        screen
          .getByRole("button", { name: "この候補の場所に戻す" })
          .hasAttribute("disabled"),
      ).toBe(true);
      expect(paperHandle(container, 1).getAttribute("aria-disabled")).toBe("true");
      expect(paperHandle(container, 1).tabIndex).toBe(-1);

      dragPaperTip(container, 1, { x: -0.7, y: 0.6 });
      fireEvent.keyDown(paperHandle(container, 1), { key: "ArrowRight" });
      useAppStore.getState().setProposalPaperPosition(1, { x: -0.9, y: 0.9 });
      useAppStore.getState().resetProposalPaperPositions();
      expect(storedPaperPosition(1)).toEqual(sentAt);

      await act(async () => {
        pending.resolve(proposalResult(jobId, []));
        await pending.promise;
        await Promise.resolve();
      });
      expectSentPaperPosition(
        vi.mocked(ipc.proposalGenerate).mock.calls[0][0],
        1,
        sentAt!,
        "再計算を押した時点",
      );
    });

    it("動かした状態・選択中の先端・元へ戻した状態が画面から分かる", () => {
      useAppStore.setState({
        proposalStep: "candidates",
        proposalSkeleton: radialSkeleton(4),
        proposalCandidates: [makeCandidateWithSites(60, 4)],
        proposalSelected: 0,
        proposalPaperSource: null,
        proposalPaperPositions: [],
        proposalBusy: false,
        proposalError: null,
      });
      const { container } = render(<ProposalWizard />);
      fireEvent.click(
        screen.getByRole("button", { name: "紙の上の場所も調整" }),
      );
      expect(
        container.querySelector("[data-paper-position-status]")?.textContent,
      ).toContain("選んだ候補と同じ場所");
      expect(paperHandle(container, 1).dataset.paperPositionChanged).toBe(
        "false",
      );

      fireEvent.focus(paperHandle(container, 1));
      expect(container.querySelectorAll("[data-paper-focus-ring]")).toHaveLength(1);
      dragPaperTip(container, 1, { x: 0.4, y: -0.2 });
      expect(paperHandle(container, 1).dataset.paperPositionChanged).toBe("true");
      expect(paperHandle(container, 1).getAttribute("aria-label")).toContain(
        "動かしました",
      );
      expect(
        container.querySelector("[data-paper-position-status]")?.textContent,
      ).toContain("1か所動かしました");

      fireEvent.click(
        screen.getByRole("button", { name: "この候補の場所に戻す" }),
      );
      expect(paperHandle(container, 1).dataset.paperPositionChanged).toBe(
        "false",
      );
      expect(
        container.querySelector("[data-paper-position-status]")?.textContent,
      ).toContain("選んだ候補と同じ場所");
    });

    it("紙位置画面は直接操作だけで、内部用語・追加の入力欄が0件", () => {
      useAppStore.setState({
        proposalStep: "candidates",
        proposalSkeleton: radialSkeleton(4),
        proposalCandidates: [makeCandidateWithSites(70, 4)],
        proposalSelected: 0,
        proposalPaperSource: null,
        proposalPaperPositions: [],
        proposalBusy: false,
        proposalError: null,
      });
      const { container } = render(<ProposalWizard />);
      fireEvent.click(
        screen.getByRole("button", { name: "紙の上の場所も調整" }),
      );
      const dialog = screen.getByRole("dialog");
      const visibleAndNamed = [
        dialog.textContent ?? "",
        ...Array.from(
          dialog.querySelectorAll<HTMLElement>("[aria-label]"),
          (node) => node.getAttribute("aria-label") ?? "",
        ),
      ].join("\n");
      expect(visibleAndNamed).toContain("丸い印をつまんで");
      expect(visibleAndNamed).not.toMatch(
        /ソルバー|hard|soft|ヤコビアン|warm[\s-]+start|骨格|充填|節点|円の中心|座標|ID/iu,
      );
      expect(container.querySelectorAll("input, select, textarea")).toHaveLength(0);
      expect(container.querySelectorAll('[data-paper-position-editor="large"]')).toHaveLength(
        1,
      );
    });
  });

  describe("完成形と紙の上の場所を葉ごとにまとめる(作業13)", () => {
    const positionState = () => {
      const state = useAppStore.getState();
      return JSON.stringify({
        skeleton: state.proposalSkeleton,
        paperPositions: state.proposalPaperPositions,
        paperSpecified: state.proposalPaperSpecified,
        lastMoved: state.proposalPositionLastMoved,
      });
    };

    const resetPositions = (skeleton: Skeleton = radialSkeleton(2)) => {
      useAppStore.getState().closeProposal();
      useAppStore.getState().openProposal();
      useAppStore.setState({
        doc: { ...VIEW.doc, sequence: [] },
        proposalStep: "skeleton",
        proposalSkeleton: skeleton,
        proposalCandidates: [],
        proposalSelected: null,
        proposalPaperSource: null,
        proposalPaperPositions: [],
        proposalPaperSpecified: [],
        proposalPositionLastMoved: [],
        proposalPositionUndoStack: [],
        proposalPositionRedoStack: [],
        proposalBusy: false,
        proposalError: null,
      });
    };

    const withCompletion = (
      count: number,
      positions: Readonly<Record<number, { x: number; y: number }>>,
    ): Skeleton => ({
      nodes: radialSkeleton(count).nodes.map((node) =>
        positions[node.id] === undefined
          ? node
          : { ...node, tip_pos_2d: { ...positions[node.id] } },
      ),
    });

    const openPaperEditor = (count: number, mark: number) => {
      useAppStore.setState({
        proposalStep: "candidates",
        proposalCandidates: [makeCandidateWithSites(mark, count)],
        proposalSelected: 0,
      });
      useAppStore.getState().openProposalPaperPositionEditor();
      expect(useAppStore.getState().proposalStep).toBe("paper-position");
    };

    const sentPosition = (sent: Skeleton, leafId: number) =>
      sent.nodes.find((node) => node.id === leafId)?.tip_pos_2d ?? null;

    it("12本すべてが違うときも、通知欄だけを送り紙と12個の戻す手段を残す", () => {
      const completion = Object.fromEntries(
        Array.from({ length: 12 }, (_, index) => [
          index + 1,
          {
            x: Math.cos((index / 12) * Math.PI * 2) * 0.55,
            y: Math.sin((index / 12) * Math.PI * 2) * 0.55,
          },
        ]),
      );
      resetPositions(withCompletion(12, completion));
      openPaperEditor(12, 89);
      const specified = Array.from({ length: 12 }, (_, index) => ({
        leaf_id: index + 1,
        position: { x: 0.9 - index * 0.01, y: -0.9 + index * 0.01 },
      }));
      useAppStore.setState({
        proposalPaperPositions: specified,
        proposalPaperSpecified: specified,
        proposalPositionLastMoved: specified.map(({ leaf_id }) => ({
          leaf_id,
          source: "paper" as const,
        })),
      });
      const { container } = render(<ProposalWizard />);
      const notices = container.querySelector(
        '[data-proposal-position-notices="12"]',
      );
      const stage = container.querySelector(".paper-position-stage");
      expect(notices).not.toBeNull();
      expect(notices?.querySelectorAll("li")).toHaveLength(12);
      expect(notices?.querySelectorAll("button")).toHaveLength(12);
      expect(stage?.querySelectorAll("[data-paper-position-handle]")).toHaveLength(12);
      expect(stage?.contains(notices)).toBe(false);
      expect(
        container.querySelectorAll(
          ".paper-position-sidebar > .proposal-position-notices",
        ),
      ).toHaveLength(1);
    });

    it("完成位置だけ・紙の上だけ・両方・食い違うの4状態で要求を各1件だけ送る", async () => {
      mockProposalCandidates([]);
      const completion = { x: 0.2, y: -0.3 };
      const completionSkeleton = withCompletion(1, { 1: completion });
      const automaticPaper = completionPositionsOnPaper(
        completionSkeleton,
        VIEW.doc.paper,
      )[0].position;
      const paperOnly = { x: -0.45, y: 0.55 };
      const differentPaper = { x: 0.85, y: 0.75 };

      const cases = [
        {
          label: "完成位置だけ",
          skeleton: completionSkeleton,
          specified: [],
          lastMoved: [{ leaf_id: 1, source: "completion" as const }],
          expected: completion,
        },
        {
          label: "紙の上だけ",
          skeleton: radialSkeleton(1),
          specified: [{ leaf_id: 1, position: paperOnly }],
          lastMoved: [{ leaf_id: 1, source: "paper" as const }],
          expected: paperOnly,
        },
        {
          label: "両方",
          skeleton: completionSkeleton,
          specified: [{ leaf_id: 1, position: automaticPaper }],
          lastMoved: [{ leaf_id: 1, source: "completion" as const }],
          expected: automaticPaper,
        },
        {
          label: "食い違う",
          skeleton: completionSkeleton,
          specified: [{ leaf_id: 1, position: differentPaper }],
          lastMoved: [{ leaf_id: 1, source: "paper" as const }],
          expected: differentPaper,
        },
      ];

      for (const testCase of cases) {
        resetPositions(testCase.skeleton);
        useAppStore.setState({
          proposalPaperSpecified: testCase.specified,
          proposalPositionLastMoved: testCase.lastMoved,
        });
        vi.mocked(ipc.proposalGenerate).mockClear();
        await act(async () => {
          await useAppStore.getState().generateProposal();
        });
        expect(
          ipc.proposalGenerate,
          `${testCase.label}の要求数`,
        ).toHaveBeenCalledTimes(1);
        expectSentPaperPosition(
          vi.mocked(ipc.proposalGenerate).mock.calls[0][0],
          1,
          testCase.expected,
          testCase.label,
        );
      }
    });

    it("食い違う葉は完成形から紙・紙から完成形の両方向で後から動かした場所を送る", async () => {
      mockProposalCandidates([]);
      resetPositions(radialSkeleton(1));
      const firstCompletion = { x: -0.25, y: 0.35 };
      const paper = { x: 0.7, y: -0.65 };
      const lastCompletion = { x: 0.4, y: 0.15 };

      useAppStore.getState().setProposalTipPosition(1, firstCompletion);
      openPaperEditor(1, 80);
      useAppStore.getState().setProposalPaperPosition(1, paper);
      vi.mocked(ipc.proposalGenerate).mockClear();
      await act(async () => {
        await useAppStore.getState().generateProposalFromPaperPositions();
      });
      expect(ipc.proposalGenerate).toHaveBeenCalledTimes(1);
      expect(sentPosition(vi.mocked(ipc.proposalGenerate).mock.calls[0][0], 1)).toEqual(
        paper,
      );

      useAppStore.getState().setProposalTipPosition(1, lastCompletion);
      vi.mocked(ipc.proposalGenerate).mockClear();
      await act(async () => {
        await useAppStore.getState().generateProposal();
      });
      expect(ipc.proposalGenerate).toHaveBeenCalledTimes(1);
      expect(sentPosition(vi.mocked(ipc.proposalGenerate).mock.calls[0][0], 1)).toEqual(
        lastCompletion,
      );
    });

    it("1つの葉は完成形、別の葉は紙の上を使い、まとめた要求を1件だけ送る", async () => {
      mockProposalCandidates([]);
      resetPositions(radialSkeleton(2));
      const completion = { x: -0.35, y: -0.25 };
      const paper = { x: 0.6, y: 0.5 };
      useAppStore.getState().setProposalTipPosition(1, completion);
      openPaperEditor(2, 81);
      useAppStore.getState().setProposalPaperPosition(2, paper);

      vi.mocked(ipc.proposalGenerate).mockClear();
      await act(async () => {
        await useAppStore.getState().generateProposalFromPaperPositions();
      });
      expect(ipc.proposalGenerate).toHaveBeenCalledTimes(1);
      const sent = vi.mocked(ipc.proposalGenerate).mock.calls[0][0];
      expect(sentPosition(sent, 1)).toEqual(completion);
      expect(sentPosition(sent, 2)).toEqual(paper);
    });

    it("食い違う印・いま使う場所・反対側へ戻す操作を両画面で示し、入れ子にしない", () => {
      const completion = { x: -0.2, y: 0.25 };
      const paper = { x: 0.8, y: -0.75 };
      const skeleton = withCompletion(1, { 1: completion });
      const interactive =
        'button, a[href], input, select, textarea, summary, [role="button"], [role="link"], [role="slider"], [tabindex]:not([tabindex="-1"])';

      resetPositions(skeleton);
      useAppStore.setState({
        proposalPaperSpecified: [{ leaf_id: 1, position: paper }],
        proposalPositionLastMoved: [{ leaf_id: 1, source: "completion" }],
      });
      const completionView = render(<ProposalWizard />);
      expect(
        completionView.container.querySelector(
          '[data-tip-position-different="1"][data-position-used="completion"]',
        ),
      ).not.toBeNull();
      expect(screen.getByText("完成形と紙の上で場所が違う先が1か所あります。"))
        .not.toBeNull();
      expect(screen.getByText(/完成形で動かした場所を使います。/u)).not.toBeNull();
      const toPaper = screen.getByRole("button", { name: /を紙の上の場所に戻す$/u });
      for (const element of completionView.container.querySelectorAll<HTMLElement>(
        interactive,
      )) {
        expect(element.parentElement?.closest(interactive) ?? null).toBeNull();
      }
      fireEvent.click(toPaper);
      expect(useAppStore.getState().proposalSkeleton.nodes[1].tip_pos_2d).toBeUndefined();
      expect(useAppStore.getState().proposalPaperSpecified[0].position).toEqual(paper);
      completionView.unmount();

      resetPositions(skeleton);
      useAppStore.setState({
        proposalPaperSpecified: [{ leaf_id: 1, position: paper }],
        proposalPositionLastMoved: [{ leaf_id: 1, source: "paper" }],
      });
      openPaperEditor(1, 82);
      const paperView = render(<ProposalWizard />);
      expect(
        paperView.container.querySelector(
          '[data-paper-position-different="1"][data-position-used="paper"]',
        ),
      ).not.toBeNull();
      expect(screen.getByText(/紙の上で動かした場所を使います。/u)).not.toBeNull();
      const toCompletion = screen.getByRole("button", {
        name: /を完成形の場所に戻す$/u,
      });
      for (const element of paperView.container.querySelectorAll<HTMLElement>(
        interactive,
      )) {
        expect(element.parentElement?.closest(interactive) ?? null).toBeNull();
      }
      fireEvent.click(toCompletion);
      expect(useAppStore.getState().proposalPaperSpecified).toHaveLength(0);
      expect(useAppStore.getState().proposalSkeleton.nodes[1].tip_pos_2d).toEqual(
        completion,
      );
    });

    it("候補画面で反対側へ戻しても空画面にせず、取り消し・やり直しで画面ごと戻る", () => {
      const skeleton = withCompletion(1, { 1: { x: -0.2, y: 0.25 } });
      const candidate = makeCandidateWithSites(83, 1);
      resetPositions(skeleton);
      useAppStore.setState({
        proposalStep: "candidates",
        proposalCandidates: [candidate],
        proposalSelected: 0,
        proposalPaperSpecified: [
          { leaf_id: 1, position: { x: 0.8, y: -0.75 } },
        ],
        proposalPositionLastMoved: [{ leaf_id: 1, source: "paper" }],
      });
      render(<ProposalWizard />);

      fireEvent.click(
        screen.getByRole("button", { name: /を完成形の場所に戻す$/u }),
      );
      expect(useAppStore.getState().proposalStep).toBe("skeleton");
      expect(useAppStore.getState().proposalCandidates).toHaveLength(0);
      expect(screen.getByRole("dialog").textContent).toContain(
        "展開図を作ってもらう",
      );

      act(() => useAppStore.getState().undoProposalPosition());
      expect(useAppStore.getState().proposalStep).toBe("candidates");
      expect(useAppStore.getState().proposalCandidates).toEqual([candidate]);
      expect(screen.getByRole("button", { name: "候補1" })).not.toBeNull();

      act(() => useAppStore.getState().redoProposalPosition());
      expect(useAppStore.getState().proposalStep).toBe("skeleton");
      expect(useAppStore.getState().proposalCandidates).toHaveLength(0);
    });

    it("計算中は場所の履歴を動かさず、開始時の入力と返る候補を食い違わせない", async () => {
      resetPositions(radialSkeleton(1));
      useAppStore
        .getState()
        .setProposalTipPosition(1, { x: 0.3, y: -0.2 });
      const before = positionState();
      useAppStore.setState({ proposalBusy: true });
      vi.mocked(ipc.editUndo).mockClear();
      render(<ProposalWizard />);

      expect(
        (
          screen.getByRole("button", {
            name: "場所の操作を元に戻す",
          }) as HTMLButtonElement
        ).disabled,
      ).toBe(true);
      await act(async () => {
        await useAppStore.getState().undo();
      });
      expect(positionState()).toBe(before);
      expect(ipc.editUndo).not.toHaveBeenCalled();
    });

    it("4種類の場所操作を取り消し・やり直しでき、作品側の履歴へ送らない", async () => {
      let checked = 0;
      const verifyUndoRedo = async (change: () => void, label: string) => {
        const before = positionState();
        change();
        const after = positionState();
        expect(after, `${label}の変更`).not.toBe(before);
        await useAppStore.getState().undo();
        expect(positionState(), `${label}の取り消し`).toBe(before);
        await useAppStore.getState().redo();
        expect(positionState(), `${label}のやり直し`).toBe(after);
        checked += 1;
      };

      vi.mocked(ipc.editUndo).mockClear();
      vi.mocked(ipc.editRedo).mockClear();

      resetPositions(radialSkeleton(2));
      await verifyUndoRedo(
        () =>
          useAppStore
            .getState()
            .setProposalTipPosition(1, { x: 0.25, y: -0.15 }),
        "完成位置だけ",
      );

      resetPositions(radialSkeleton(2));
      openPaperEditor(2, 91);
      await verifyUndoRedo(
        () =>
          useAppStore
            .getState()
            .setProposalPaperPosition(1, { x: 0.55, y: -0.45 }),
        "紙の上だけ",
      );

      resetPositions(radialSkeleton(2));
      useAppStore
        .getState()
        .setProposalTipPosition(1, { x: -0.3, y: 0.2 });
      openPaperEditor(2, 92);
      await verifyUndoRedo(
        () =>
          useAppStore
            .getState()
            .setProposalPaperPosition(1, { x: 0.7, y: -0.6 }),
        "同じ葉の完成形から紙の上",
      );

      resetPositions(radialSkeleton(2));
      useAppStore
        .getState()
        .setProposalTipPosition(1, { x: -0.45, y: 0.35 });
      openPaperEditor(2, 93);
      await verifyUndoRedo(
        () =>
          useAppStore
            .getState()
            .setProposalPaperPosition(2, { x: 0.65, y: 0.6 }),
        "葉ごとに完成形と紙の上",
      );

      expect(checked).toBe(4);
      expect(ipc.editUndo).not.toHaveBeenCalled();
      expect(ipc.editRedo).not.toHaveBeenCalled();
    });

    it("場所をまとめる表示に内部の言葉を出さない", () => {
      resetPositions(withCompletion(1, { 1: { x: 0.1, y: 0.2 } }));
      useAppStore.setState({
        proposalPaperSpecified: [
          { leaf_id: 1, position: { x: 0.8, y: -0.7 } },
        ],
        proposalPositionLastMoved: [{ leaf_id: 1, source: "paper" }],
      });
      render(<ProposalWizard />);
      const dialog = screen.getByRole("dialog");
      const visibleAndNamed = [
        dialog.textContent ?? "",
        ...Array.from(
          dialog.querySelectorAll<HTMLElement>("[aria-label]"),
          (node) => node.getAttribute("aria-label") ?? "",
        ),
      ].join("\n");
      expect(visibleAndNamed).toContain("紙の上で動かした場所を使います");
      expect(visibleAndNamed).not.toMatch(
        /ソルバー|hard|soft|ヤコビアン|warm[\s-]+start|優先規則|衝突/iu,
      );
    });
  });

  it.each([
    {
      name: "完成まで確認済み",
      candidates: Array.from({ length: 4 }, (_, index) =>
        makeCandidateWithPlan(10 + index, index + 1),
      ),
      state: "最後まで確認できました",
      steps: ["（1手）", "（2手）", "（3手）", "（4手）"],
    },
    {
      name: "途中まで確認済み",
      candidates: Array.from({ length: 4 }, (_, index) =>
        makeCandidateWithPlan(20 + index, index + 1, {
          planned: index + 3,
          status: "partial",
        }),
      ),
      state: "途中に注意があります",
      steps: ["（1手）", "（2手）", "（3手）", "（4手）"],
    },
    {
      name: "折り方なし",
      candidates: Array.from({ length: 4 }, (_, index) =>
        makeCandidate(30 + index, 0),
      ),
      state: "折り方はまだありません",
      steps: [],
    },
  ])("4候補すべてに$nameの状態を出す(作業29・12/12)", ({ candidates, state, steps }) => {
    useAppStore.setState({
      proposalStep: "candidates",
      proposalCandidates: candidates,
      proposalSelected: 0,
      proposalBusy: false,
      proposalError: null,
    });
    const { container } = render(<ProposalWizard />);

    const shownStates = Array.from(
      container.querySelectorAll<HTMLElement>("[data-fold-plan-state]"),
      (node) => node.textContent ?? "",
    );
    expect(shownStates).toEqual(Array.from({ length: 4 }, () => state));
    const shownSteps = Array.from(
      container.querySelectorAll<HTMLElement>("[data-fold-plan-steps]"),
      (node) => node.textContent ?? "",
    );
    expect(shownSteps).toEqual(steps);
  });

  it("確認の画面にも、折り方の手数と確かめた範囲を出す(作業29)", () => {
    useAppStore.setState({
      doc: { ...VIEW.doc, sequence: [] },
      proposalStep: "confirm",
      proposalCandidates: [makeCandidateWithPlan(10, 3)],
      proposalSelected: 0,
      proposalBusy: false,
      proposalError: null,
    });
    const { container } = render(<ProposalWizard />);

    expect(
      container.querySelector('[data-fold-plan-state="confirm"]')?.textContent,
    ).toBe("最後まで確認できました");
    expect(
      container.querySelector('[data-fold-plan-steps="confirm"]')?.textContent,
    ).toBe("（3手）");
    expect(
      screen.getByText(/展開図と折り方が一緒に入ります/u),
    ).not.toBeNull();
  });

  it("確認の画面で、途中までの参考手順へ注意と手数を出す(作業29)", () => {
    useAppStore.setState({
      doc: { ...VIEW.doc, sequence: [] },
      proposalStep: "confirm",
      proposalCandidates: [
        makeCandidateWithPlan(11, 2, { planned: 4, status: "partial" }),
      ],
      proposalSelected: 0,
      proposalBusy: false,
      proposalError: null,
    });
    const { container } = render(<ProposalWizard />);

    expect(
      container.querySelector('[data-fold-plan-state="confirm"]')?.textContent,
    ).toBe("途中に注意があります");
    expect(
      container.querySelector('[data-fold-plan-steps="confirm"]')?.textContent,
    ).toBe("（2手）");
  });

  it("折り方が無い候補では、確認の画面でもそう伝える(作業29)", () => {
    showConfirmationWithSteps(0);
    const { container } = render(<ProposalWizard />);
    expect(
      container.querySelector('[data-fold-plan-state="confirm"]')?.textContent,
    ).toBe("折り方はまだありません");
    expect(
      container.querySelector('[data-fold-plan-steps="confirm"]'),
    ).toBeNull();
    expect(screen.queryByText(/展開図と折り方が一緒に入ります/u)).toBeNull();
  });

  it.each([
    ["完成まで確認済み", "checked_to_finish"],
    ["途中までの参考手順", "partial"],
  ] as const)(
    "%sの候補を使うと、展開図と折り手順が1回でまとめて入る(作業28)",
    async (_name, status) => {
      vi.mocked(ipc.proposalApply).mockResolvedValue(VIEW);
      const candidate = makeCandidateWithPlan(10, 3, { status });
      useAppStore.setState({
        doc: { ...VIEW.doc, sequence: makeSteps(2) },
        proposalStep: "confirm",
        proposalCandidates: [candidate],
        proposalSelected: 0,
        proposalBusy: false,
        proposalError: null,
      });
      render(<ProposalWizard />);

      fireEvent.click(
        screen.getByRole("button", { name: "この展開図を使う" }),
      );

      await vi.waitFor(() => expect(ipc.proposalApply).toHaveBeenCalledTimes(1));
      expect(ipc.proposalApply).toHaveBeenCalledWith(
        candidate.fold_plan!.cp,
        candidate.fold_plan!.steps,
      );
      // 展開図だけを入れる古い経路は使わない(途中の状態を作らない)
      expect(ipc.editApply).not.toHaveBeenCalled();
      expect(useAppStore.getState().proposalStep).toBeNull();
    },
  );

  it("作業30: 頭1・尾1・足4の4候補受領から全手順の一括適用まで運ぶ", async () => {
    const namedSkeleton = headTailFourLegsSkeleton();
    const completed = makeCandidateWithPlan(30, 5);
    const returned: DocumentView = {
      ...VIEW,
      doc: {
        ...VIEW.doc,
        cp: completed.fold_plan!.cp,
        sequence: completed.fold_plan!.steps,
      },
      skipped: [],
    };
    mockProposalCandidates([
      completed,
      makeCandidateWithPlan(31, 3, { planned: 5, status: "partial" }),
      makeCandidate(32, 0),
      makeCandidate(33, 0),
    ]);
    vi.mocked(ipc.proposalApply).mockResolvedValue(returned);
    useAppStore.getState().openProposal();
    useAppStore.setState({ proposalSkeleton: namedSkeleton });
    render(<ProposalWizard />);

    fireEvent.click(screen.getByRole("button", { name: "展開図を作ってもらう" }));
    await vi.waitFor(() =>
      expect(screen.getByRole("button", { name: "候補4" })).not.toBeNull(),
    );
    expect(vi.mocked(ipc.proposalGenerate).mock.calls[0][0]).toEqual(
      namedSkeleton,
    );
    expect(screen.getByRole("button", { name: "候補1" }).textContent).toContain(
      "最後まで確認できました",
    );
    expect(screen.getByRole("button", { name: "候補1" }).textContent).toContain(
      "（5手）",
    );

    fireEvent.click(screen.getByRole("button", { name: "候補1" }));
    fireEvent.click(screen.getByRole("button", { name: "これにする" }));
    expect(screen.getByText("最後まで確認できました")).not.toBeNull();
    expect(screen.getByText("（5手）")).not.toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "この展開図を使う" }));

    await vi.waitFor(() => expect(ipc.proposalApply).toHaveBeenCalledTimes(1));
    expect(ipc.proposalApply).toHaveBeenCalledWith(
      completed.fold_plan!.cp,
      completed.fold_plan!.steps,
    );
    expect(ipc.editApply).not.toHaveBeenCalled();
    expect(useAppStore.getState().doc?.sequence).toEqual(
      completed.fold_plan!.steps,
    );
    expect(useAppStore.getState().skipped).toHaveLength(0);
  });

  it("折り方が無い候補は、今までどおり展開図だけを入れる", async () => {
    useAppStore.setState({
      doc: { ...VIEW.doc, sequence: [] },
      proposalStep: "confirm",
      proposalCandidates: [makeCandidate(13, 0)],
      proposalSelected: 0,
      proposalBusy: false,
      proposalError: null,
    });
    render(<ProposalWizard />);

    fireEvent.click(screen.getByRole("button", { name: "この展開図を使う" }));

    await vi.waitFor(() => expect(ipc.editApply).toHaveBeenCalledTimes(1));
    expect(ipc.editApply).toHaveBeenCalledWith({
      type: "ReplaceCreasePattern",
      cp: makeCp(13),
    });
    expect(ipc.proposalApply).not.toHaveBeenCalled();
  });

  it("折り方の言い方に、画面で使わない言葉を出さない(§11.1)", () => {
    const sentences = [
      foldPlanLabel(null),
      foldPlanLabel(undefined),
      foldPlanLabel(makeCandidateWithPlan(10, 3).fold_plan),
      foldPlanLabel(
        makeCandidateWithPlan(11, 2, { planned: 5, status: "partial" })
          .fold_plan,
      ),
      "「この展開図を使う」を押すと、展開図と折り方が一緒に入ります。",
    ];
    for (const sentence of sentences) {
      expect(sentence).not.toMatch(
        /検証|ソルバー|ヤコビアン|探索|骨格|充填|節点|木構造|hard|soft|warm[\s-]+start|イテレーション|姿勢|自己交差|裂け/iu,
      );
    }
    expect(foldPlanLabel(null)).toBe("折り方はまだありません");
    expect(foldPlanLabel(makeCandidateWithPlan(10, 3).fold_plan)).toBe(
      "最後まで確認できました",
    );
    expect(
      foldPlanLabel(
        makeCandidateWithPlan(11, 2, { status: "partial" }).fold_plan,
      ),
    ).toBe("途中に注意があります");
  });
});
