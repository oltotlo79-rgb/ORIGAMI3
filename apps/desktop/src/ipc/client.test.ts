import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";
import {
  foldAllPreview,
  poseSolve,
  proposalControl,
  proposalGenerate,
  proposalProgress,
} from "./client";
import type {
  Driver,
  FoldAllPreviewOutcome,
  Paper,
  ProposalJobResult,
  ProposalProgressSnapshot,
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
      hard,
      preferred,
      warmSeed,
      soft: null,
      upTo: 2,
      t: 0.4,
      mode: "Follow",
    });

    vi.mocked(invoke).mockClear();
    await poseSolve(hard, preferred, null, warmSeed, 2, 0.4, "Canonical");
    expect(invoke).toHaveBeenCalledWith("pose_solve", {
      hard,
      preferred,
      warmSeed,
      soft: null,
      upTo: 2,
      t: 0.4,
      mode: "Canonical",
    });
  });
});
