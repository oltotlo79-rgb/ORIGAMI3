// @vitest-environment jsdom
// 提案ウィザードの画面テスト(Task 3-4):
// 閉じているときは何も出さない、出っぱりの増減が骨格に反映される、
// 候補を選んで「この展開図を使う」と edit_apply ReplaceCreasePattern が飛ぶ。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";

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
import { defaultSkeleton, limbs } from "../../lib/skeleton";
import type { CreasePattern, DocumentView, ProposalCandidate } from "../../lib/types";

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

beforeEach(() => {
  vi.mocked(ipc.editApply).mockResolvedValue(VIEW);
  useAppStore.getState().closeProposal();
  useAppStore.setState({
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
    render(<ProposalWizard />);
    expect(limbs(useAppStore.getState().proposalSkeleton)).toHaveLength(4);

    fireEvent.click(screen.getByRole("button", { name: "出っぱりを増やす" }));
    expect(limbs(useAppStore.getState().proposalSkeleton)).toHaveLength(5);

    fireEvent.click(screen.getByRole("button", { name: "頭を減らす" }));
    expect(limbs(useAppStore.getState().proposalSkeleton)).toHaveLength(4);
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

  it("作れなかったときは日本語の理由を出す", async () => {
    vi.mocked(ipc.proposalGenerate).mockRejectedValue("角は12本までです");
    useAppStore.getState().openProposal();
    render(<ProposalWizard />);
    fireEvent.click(screen.getByRole("button", { name: "展開図を作ってもらう" }));
    await vi.waitFor(() =>
      expect(screen.getByText("角は12本までです")).not.toBeNull(),
    );
  });

  it("折りにくさの目安は0か所なら言い換える", () => {
    expect(violationLabel(0)).toBe("きれいに畳めそうです");
    expect(violationLabel(2)).toContain("2 か所");
  });
});
