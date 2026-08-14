// @vitest-environment jsdom
// 提案ウィザードの画面テスト(Task 3-4):
// 閉じているときは何も出さない、出っぱりの増減が骨格に反映される、
// 候補を選んで「この展開図を使う」と edit_apply ReplaceCreasePattern が飛ぶ。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";

vi.mock("../../ipc/client", () => ({
  documentNew: vi.fn(),
  documentOpen: vi.fn(),
  documentSave: vi.fn(),
  editApply: vi.fn(),
  editUndo: vi.fn(),
  editRedo: vi.fn(),
  sequenceApply: vi.fn(),
  sequenceReplay: vi.fn(),
  poseSolve: vi.fn(),
  recoveryCheck: vi.fn(),
  recoveryRestore: vi.fn(),
  proposalGenerate: vi.fn(),
}));

import * as ipc from "../../ipc/client";
import { ProposalWizard, violationLabel } from "./ProposalWizard";
import { useAppStore } from "../../store/appStore";
import {
  defaultSkeleton,
  leafNodes,
  limbs,
  skeletonRows,
} from "../../lib/skeleton";
import type {
  CreasePattern,
  DocumentView,
  FoldStep,
  ProposalCandidate,
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
  return { cp: makeCp(mark), scale: 0.4, violations, warnings: [] };
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
  vi.mocked(ipc.editApply).mockResolvedValue(VIEW);
  useAppStore.getState().closeProposal();
  useAppStore.setState({
    doc: { ...VIEW.doc, sequence: [] },
    proposalSkeleton: defaultSkeleton(),
    proposalCandidates: [],
    proposalSelected: null,
    proposalError: null,
  });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("提案ウィザード", () => {
  it("閉じているときは何も出さない(常設UIを増やさない)", () => {
    const { container } = render(<ProposalWizard />);
    expect(container.firstChild).toBeNull();
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
    vi.mocked(ipc.proposalGenerate).mockResolvedValue([makeCandidate(12, 0)]);
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
    vi.mocked(ipc.proposalGenerate).mockResolvedValue([
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
      const pending = deferred<ProposalCandidate[]>();
      vi.mocked(ipc.proposalGenerate).mockReturnValue(pending.promise);
      useAppStore.getState().openProposal();
      render(<ProposalWizard />);

      fireEvent.click(
        screen.getByRole("button", { name: "展開図を作ってもらう" }),
      );
      fireEvent.click(screen.getByRole("button", { name: "やめる" }));
      expect(screen.queryByRole("dialog")).toBeNull();

      await act(async () => {
        pending.resolve([makeCandidate(10, 0)]);
        await pending.promise;
        await Promise.resolve();
      });
      expect(useAppStore.getState().proposalCandidates).toHaveLength(0);
      expect(useAppStore.getState().proposalStep).toBeNull();
      expect(screen.queryByRole("dialog")).toBeNull();
    },
  );

  it("D22: 取り消した100要求が逆順に完了しても画面・候補・計算中表示を戻さない", async () => {
    const pending = Array.from({ length: 100 }, () => deferred<ProposalCandidate[]>());
    let issued = 0;
    vi.mocked(ipc.proposalGenerate).mockImplementation(() => pending[issued++].promise);
    render(<ProposalWizard />);

    const requests: Promise<void>[] = [];
    for (let index = 0; index < pending.length; index++) {
      useAppStore.getState().openProposal();
      requests.push(useAppStore.getState().generateProposal());
      useAppStore.getState().closeProposal();
    }
    expect(issued).toBe(100);

    for (let index = pending.length - 1; index >= 0; index--) {
      await act(async () => {
        pending[index].resolve([makeCandidate(index + 10, 0)]);
        await requests[index];
      });
      const state = useAppStore.getState();
      expect(state.proposalStep).toBeNull();
      expect(state.proposalCandidates).toHaveLength(0);
      expect(state.proposalBusy).toBe(false);
      expect(screen.queryByRole("dialog")).toBeNull();
    }
  });

  it("提案の警告と生成エラーには内部用語を表示しない", () => {
    const internalTerms = [
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
