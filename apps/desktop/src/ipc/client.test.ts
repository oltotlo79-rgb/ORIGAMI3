import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./runtime", () => ({ callBackend: vi.fn() }));

import { callBackend as invoke } from "./runtime";
import {
  documentNew,
  documentExport,
  documentOpen,
  documentSave,
  editApply,
  editApplyBatch,
  editRedo,
  editUndo,
  foldAllPreview,
  poseSolve,
  proposalApply,
  proposalControl,
  proposalGenerate,
  proposalProgress,
  recoveryCheck,
  recoveryRestore,
  sequenceApply,
  sequenceReplay,
} from "./client";
import type {
  CreasePattern,
  DocumentView,
  Driver,
  EditOp,
  FoldIssue,
  FoldAllPreviewOutcome,
  Paper,
  ProposalJobResult,
  ProposalProgressSnapshot,
  SeqOp,
  Skeleton,
  SolveResult,
} from "../lib/types";

const SKELETON: Skeleton = {
  nodes: [{ id: 0, parent: null, length: 0, width_factor: 1 }],
};
const PAPER: Paper = { width_mm: 150, height_mm: 150 };
const JOB_ID = "proposal-client-job";

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});

describe("IPCクライアントの公開契約", () => {
  it("18関数が従来と同じ名前と引数でbackendを呼ぶ", () => {
    const editOp = { type: "test-edit" } as unknown as EditOp;
    const sequenceOp = { type: "test-sequence" } as unknown as SeqOp;
    const creasePattern = { vertices_coords: [] } as unknown as CreasePattern;
    const hard: Driver[] = [{ hinge: 3, target_angle_deg: 90 }];

    documentNew(PAPER);
    documentOpen("C:\\作品\\sample.ori3");
    documentSave(null);
    editApply(editOp);
    editApplyBatch([editOp]);
    editUndo();
    editRedo();
    sequenceApply(sequenceOp);
    sequenceReplay(2, 0.5);
    poseSolve(hard);
    foldAllPreview(75);
    recoveryCheck();
    recoveryRestore(true, 42);
    proposalGenerate(SKELETON, PAPER, 7, JOB_ID);
    proposalProgress(JOB_ID);
    proposalControl({ type: "Cancel", job_id: JOB_ID });
    proposalApply(creasePattern, []);
    documentExport("FoldJson", "C:\\作品\\sample.fold", {
      include_aux: false,
      png_long_side: 2048,
    });

    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["document_new", { paper: PAPER }],
      ["document_open", { path: "C:\\作品\\sample.ori3" }],
      ["document_save", { path: null }],
      ["edit_apply", { op: editOp }],
      ["edit_apply_batch", { ops: [editOp] }],
      ["edit_undo"],
      ["edit_redo"],
      ["sequence_apply", { op: sequenceOp }],
      ["sequence_replay", { upTo: 2, t: 0.5, soft: null }],
      [
        "pose_solve",
        {
          request: {
            hard,
            preferred: null,
            warmSeed: null,
            soft: null,
            upTo: 0,
            t: 1,
            mode: "Follow",
          },
        },
      ],
      ["fold_all_preview", { percent: 75, warmSeed: null }],
      ["recovery_check"],
      ["recovery_restore", { accept: true, candidateId: 42 }],
      [
        "proposal_generate",
        {
          jobId: JOB_ID,
          skeleton: SKELETON,
          paper: PAPER,
          seed: 7,
          withFoldPlan: true,
        },
      ],
      ["proposal_progress", { jobId: JOB_ID }],
      [
        "proposal_control",
        { operation: { type: "Cancel", job_id: JOB_ID } },
      ],
      ["proposal_apply", { cp: creasePattern, steps: [] }],
      [
        "document_export",
        {
          kind: "FoldJson",
          path: "C:\\作品\\sample.fold",
          options: { include_aux: false, png_long_side: 2048 },
        },
      ],
    ]);
  });
});

describe("ほかの折り紙ソフトのファイルのIPC", () => {
  const issue: FoldIssue = {
    severity: "warning",
    code: "unsupported_field",
    path: "$.x_example",
    message: "raw message",
    original_value: true,
  };

  it("読込結果に含まれる注意を捨てずに返す", async () => {
    const view = { fold_issues: [issue] } as DocumentView;
    vi.mocked(invoke).mockResolvedValue(view);

    await expect(documentOpen("C:\\作品\\sample.fold")).resolves.toBe(view);
    expect(invoke).toHaveBeenCalledWith("document_open", {
      path: "C:\\作品\\sample.fold",
    });
  });

  it("書き出しの種類と注意配列を変えずに往復させる", async () => {
    vi.mocked(invoke).mockResolvedValue([issue]);

    await expect(
      documentExport("FoldJson", "C:\\作品\\sample.fold", {
        include_aux: false,
        png_long_side: 2048,
      }),
    ).resolves.toEqual([issue]);
    expect(invoke).toHaveBeenCalledWith("document_export", {
      kind: "FoldJson",
      path: "C:\\作品\\sample.fold",
      options: { include_aux: false, png_long_side: 2048 },
    });
  });
});

describe("提案job IPC", () => {
  it("生成の外側引数をcamelCaseで送り、job_id付き結果をそのまま返す", async () => {
    const result: ProposalJobResult = { job_id: JOB_ID, candidates: [] };
    vi.mocked(invoke).mockResolvedValue(result);

    await expect(proposalGenerate(SKELETON, PAPER, 7, JOB_ID)).resolves.toBe(
      result,
    );
    expect(invoke).toHaveBeenCalledWith("proposal_generate", {
      jobId: JOB_ID,
      skeleton: SKELETON,
      paper: PAPER,
      seed: 7,
      withFoldPlan: true,
    });
  });

  it("150ms pollの読取り先をjobIdで指定し、回収済みnullも保つ", async () => {
    const snapshot: ProposalProgressSnapshot = {
      job_id: JOB_ID,
      done: 2,
      total: 4,
      phase: "Generating",
    };
    vi.mocked(invoke).mockResolvedValueOnce(snapshot).mockResolvedValueOnce(null);

    await expect(proposalProgress(JOB_ID)).resolves.toBe(snapshot);
    await expect(proposalProgress(JOB_ID)).resolves.toBeNull();
    expect(invoke).toHaveBeenNthCalledWith(1, "proposal_progress", {
      jobId: JOB_ID,
    });
  });

  it("取消しの入れ子だけはserdeどおりsnake_caseで送る", async () => {
    const snapshot: ProposalProgressSnapshot = {
      job_id: JOB_ID,
      done: 4,
      total: 4,
      phase: "Cancelled",
    };
    vi.mocked(invoke).mockResolvedValue(snapshot);

    await proposalControl({ type: "Cancel", job_id: JOB_ID });
    expect(invoke).toHaveBeenCalledWith("proposal_control", {
      operation: { type: "Cancel", job_id: JOB_ID },
    });
  });

  it("生成の中断を候補0件の成功へ変換しない", async () => {
    vi.mocked(invoke).mockRejectedValue("計算を取り消しました");

    await expect(
      proposalGenerate(SKELETON, PAPER, 9, JOB_ID),
    ).rejects.toBe("計算を取り消しました");
  });
});

describe("全折り目をいっぺんに動かすIPC", () => {
  it("percentと直前の実角を実引数名で送り、構造化結果をそのまま返す", async () => {
    const warmSeed = [{ hinge: 5, target_angle_deg: 45 }];
    const result: FoldAllPreviewOutcome = {
      frame: { faces: [], warnings: [] },
      converged: true,
      angles: { "5": 90 },
      iterations: 2,
      requested_percent: 50,
      requested_angles: [{ hinge: 5, target_angle_deg: 90 }],
      next_warm_seed: [{ hinge: 5, target_angle_deg: 90 }],
      suspect_hinges: [],
      contact_detected: false,
      flat_fold_violations: [],
      layer_order: "unavailable_without_sequence",
    };
    vi.mocked(invoke).mockResolvedValue(result);

    await expect(foldAllPreview(50, warmSeed)).resolves.toBe(result);
    expect(invoke).toHaveBeenCalledWith("fold_all_preview", {
      percent: 50,
      warmSeed,
    });

    vi.mocked(invoke).mockClear();
    await foldAllPreview(0, []);
    expect(invoke).toHaveBeenCalledWith("fold_all_preview", {
      percent: 0,
      warmSeed: null,
    });
  });
});

describe("角度姿勢の計算mode IPC", () => {
  it("省略時はFollowを送り、Canonicalは末尾引数で明示できる", async () => {
    const hard: Driver[] = [{ hinge: 19, target_angle_deg: 90 }];
    const preferred: Driver[] = [{ hinge: 17, target_angle_deg: -90 }];
    const warmSeed: Driver[] = [{ hinge: 21, target_angle_deg: 45 }];
    const result: SolveResult = {
      frame: { faces: [], warnings: [] },
      converged: true,
      angles: {},
      iterations: 3,
    };
    vi.mocked(invoke).mockResolvedValue(result);

    await expect(
      poseSolve(hard, preferred, null, warmSeed, 2, 0.4),
    ).resolves.toBe(result);
    expect(invoke).toHaveBeenCalledWith("pose_solve", {
      request: {
        hard,
        preferred,
        warmSeed,
        soft: null,
        upTo: 2,
        t: 0.4,
        mode: "Follow",
      },
    });

    vi.mocked(invoke).mockClear();
    await poseSolve(hard, preferred, null, warmSeed, 2, 0.4, "Canonical");
    expect(invoke).toHaveBeenCalledWith("pose_solve", {
      request: {
        hard,
        preferred,
        warmSeed,
        soft: null,
        upTo: 2,
        t: 0.4,
        mode: "Canonical",
      },
    });
  });
});
