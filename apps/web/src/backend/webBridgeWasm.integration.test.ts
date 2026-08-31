// @vitest-environment jsdom

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { NewDocumentDialog } from "../../../desktop/src/components/dialogs/NewDocumentDialog";
import {
  documentNew,
  documentOpen,
  documentSave,
  documentExport,
  editApply,
  editApplyBatch,
  editRedo,
  editUndo,
  proposalApply,
  proposalControl,
  proposalGenerate,
  proposalProgress,
  recoveryCheck,
  recoveryRestore,
  sequenceApply,
  sequenceReplay,
} from "../../../desktop/src/ipc/client";
import type {
  Paper,
  ProposalJobResult,
  ProposalProgressSnapshot,
  SeqOp,
  Skeleton,
} from "../../../desktop/src/lib/types";
import { useAppStore } from "../../../desktop/src/store/appStore";
import {
  createOri3CoreWorkerClient,
  type Ori3CoreWorkerClient,
} from "./coreWorkerClient";
import type {
  CoreWorkerRequest,
  CoreWorkerResponse,
} from "./coreWorkerProtocol";
import {
  createBrowserCurrentDocumentCoordinator,
  createBrowserDocumentInvoker,
  createDocumentLifecycleCoreInvoker,
} from "./browserDocumentInvoker";
import {
  createWebBridgeDependencies,
  installOri3WebBridge,
  type WebBridgeDependencies,
} from "./installWebBridge";
import { createBrowserFileTokenRegistry } from "../platform/browserFileTokenRegistry";
import {
  initializeCoreWorker,
  startCoreWorker,
  type Ori3WasmBackendPort,
} from "./ori3-core.worker";
import {
  initSync,
  Ori3WasmBackend,
} from "./generated/ori3-web/ori3_web.js";
import {
  createProposalJobRegistry,
  type ProposalWorkerRequest,
  type ProposalWorkerResponse,
} from "./proposalJobRegistry";
import {
  initializeProposalWorker,
  startProposalWorker,
} from "./proposal.worker";
import {
  createWebRecoveryRuntime,
  type RecoveryCandidateSource,
  type RecoveryCandidateSummary,
  type WebRecoveryRepositoryPort,
} from "../recovery";
import type { AutosaveClock } from "../recovery/AutosaveScheduler";
import {
  serializeSavedDocument,
  type SavedDocumentSource,
} from "../recovery/savedDocument";

type MessageListener = (event: MessageEvent<unknown>) => void;

/**
 * Node/jsdom上で、製品と同じready・request ID・JSON境界を通すWorker代替。
 * 計算だけはmockせず、wasm-bindgenが生成した実backendへ渡す。
 */
class LoopbackCoreWorker {
  readonly requests: CoreWorkerRequest[] = [];
  readonly responses: CoreWorkerResponse[] = [];
  terminated = false;

  private requestListener?: MessageListener;
  private readonly responseListeners = new Set<MessageListener>();
  private readonly errorListeners = new Set<MessageListener>();
  private readonly responseMessageErrorListeners = new Set<MessageListener>();

  constructor(backend: Ori3WasmBackendPort) {
    const scope = {
      addEventListener: (
        type: "message" | "messageerror",
        listener: MessageListener,
      ): void => {
        if (type === "message") this.requestListener = listener;
      },
      postMessage: (response: CoreWorkerResponse): void => {
        this.responses.push(response);
        queueMicrotask(() => {
          if (this.terminated) return;
          const event = { data: response } as MessageEvent<unknown>;
          for (const listener of this.responseListeners) listener(event);
        });
      },
    };

    void startCoreWorker(scope, () => initializeCoreWorker(backend));
  }

  addEventListener(type: string, listener: MessageListener): void {
    if (type === "message") this.responseListeners.add(listener);
    if (type === "error") this.errorListeners.add(listener);
    if (type === "messageerror") {
      this.responseMessageErrorListeners.add(listener);
    }
  }

  postMessage(request: CoreWorkerRequest): void {
    this.requests.push(request);
    queueMicrotask(() => {
      if (this.terminated) return;
      this.requestListener?.({ data: request } as MessageEvent<unknown>);
    });
  }

  terminate(): void {
    this.terminated = true;
    this.responseListeners.clear();
    this.errorListeners.clear();
    this.responseMessageErrorListeners.clear();
    this.requestListener = undefined;
  }
}

class LoopbackProposalWorker {
  readonly createdAt = performance.now();
  readonly requests: ProposalWorkerRequest[] = [];
  readonly responses: Array<{
    at: number;
    response: ProposalWorkerResponse;
  }> = [];
  terminated = false;

  private requestListener?: MessageListener;
  private readonly responseListeners = new Set<MessageListener>();
  private readonly errorListeners = new Set<MessageListener>();
  private readonly responseMessageErrorListeners = new Set<MessageListener>();

  constructor(backend: Ori3WasmBackendPort) {
    const scope = {
      addEventListener: (
        type: "message" | "messageerror",
        listener: MessageListener,
      ): void => {
        if (type === "message") this.requestListener = listener;
      },
      postMessage: (response: ProposalWorkerResponse): void => {
        this.responses.push({ at: performance.now(), response });
        queueMicrotask(() => {
          if (this.terminated) return;
          const event = { data: response } as MessageEvent<unknown>;
          for (const listener of this.responseListeners) listener(event);
        });
      },
    };

    void startProposalWorker(scope, () =>
      initializeProposalWorker(backend),
    );
  }

  addEventListener(type: string, listener: MessageListener): void {
    if (type === "message") this.responseListeners.add(listener);
    if (type === "error") this.errorListeners.add(listener);
    if (type === "messageerror") {
      this.responseMessageErrorListeners.add(listener);
    }
  }

  postMessage(request: ProposalWorkerRequest): void {
    this.requests.push(request);
    queueMicrotask(() => {
      if (this.terminated) return;
      this.requestListener?.({ data: request } as MessageEvent<unknown>);
    });
  }

  terminate(): void {
    this.terminated = true;
    this.responseListeners.clear();
    this.errorListeners.clear();
    this.responseMessageErrorListeners.clear();
    this.requestListener = undefined;
  }
}

function loadRealWasmBackend(): Ori3WasmBackend {
  const here = dirname(fileURLToPath(import.meta.url));
  const bytes = Uint8Array.from(
    readFileSync(
      join(here, "generated", "ori3-web", "ori3_web_bg.wasm"),
    ),
  );
  initSync({ module: bytes });
  return new Ori3WasmBackend();
}

function loadParityFixture(name: string): unknown {
  const here = dirname(fileURLToPath(import.meta.url));
  return JSON.parse(
    readFileSync(
      join(
        here,
        "..",
        "..",
        "..",
        "..",
        "crates",
        "ori3-app-core",
        "tests",
        "fixtures",
        name,
      ),
      "utf8",
    ),
  ) as unknown;
}

function loadDocumentNewParityFixture(): unknown {
  return loadParityFixture("document-new-150x100.json");
}

interface BirdBaseProposalCorpus {
  paper: Paper;
  skeleton: Skeleton;
  seed: number;
  with_fold_plan: boolean;
}

function loadBirdBaseProposalCorpus(): BirdBaseProposalCorpus {
  const here = dirname(fileURLToPath(import.meta.url));
  return JSON.parse(
    readFileSync(
      join(
        here,
        "..",
        "..",
        "..",
        "..",
        "crates",
        "ori3-propose",
        "tests",
        "fixtures",
        "corpus",
        "bird-base.json",
      ),
      "utf8",
    ),
  ) as BirdBaseProposalCorpus;
}

function sha256(bytes: Uint8Array): string {
  return createHash("sha256").update(bytes).digest("hex").toUpperCase();
}

function fnv1a64(bytes: Uint8Array): string {
  let hash = 0xcbf29ce484222325n;
  for (const byte of bytes) {
    hash ^= BigInt(byte);
    hash = BigInt.asUintN(64, hash * 0x100000001b3n);
  }
  return hash.toString(16).toUpperCase().padStart(16, "0");
}

function savedDocumentOf(view: unknown): string {
  const value = view as {
    doc: Record<string, unknown>;
    step_creases: unknown;
  };
  return JSON.stringify(
    { ...value.doc, step_creases: value.step_creases },
    null,
    2,
  );
}

function buttonNamed(name: string): HTMLButtonElement {
  const button = [...document.querySelectorAll<HTMLButtonElement>("button")].find(
    (candidate) => candidate.textContent?.trim() === name,
  );
  if (!button) throw new Error(`画面に「${name}」ボタンがありません。`);
  return button;
}

function waitForPaper(paper: {
  width_mm: number;
  height_mm: number;
}): Promise<void> {
  return new Promise((resolve) => {
    const unsubscribe = useAppStore.subscribe((state) => {
      if (
        state.doc?.paper.width_mm === paper.width_mm &&
        state.doc.paper.height_mm === paper.height_mm
      ) {
        unsubscribe();
        resolve();
      }
    });
  });
}

function waitForFoldAllPercent(percent: number): Promise<void> {
  const current = useAppStore.getState().foldAllPreview;
  if (current?.appliedPercent === percent && !current.busy) {
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    const unsubscribe = useAppStore.subscribe((state) => {
      const active = state.foldAllPreview;
      if (active?.appliedPercent === percent && !active.busy) {
        unsubscribe();
        resolve();
      }
    });
  });
}

describe("Web画面storeから実WASMまでの文書・編集・手順・姿勢往復", () => {
  it("新規作成、編集4命令、手順2命令、姿勢2命令をinstall済みbridgeの単一core Workerへ連続送信する", async () => {
    const backend = loadRealWasmBackend();
    let loopback: LoopbackCoreWorker | undefined;
    const core: Ori3CoreWorkerClient = createOri3CoreWorkerClient(() => {
      loopback = new LoopbackCoreWorker(backend);
      return loopback as unknown as Worker;
    });
    const beforeEpoch = useAppStore.getState().docEpoch;
    const paper = { width_mm: 150, height_mm: 100 };
    const actEnvironment = globalThis as typeof globalThis & {
      IS_REACT_ACT_ENVIRONMENT?: boolean;
    };
    const previousActEnvironment = actEnvironment.IS_REACT_ACT_ENVIRONMENT;
    const host = document.createElement("div");

    actEnvironment.IS_REACT_ACT_ENVIRONMENT = true;
    document.body.append(host);
    useAppStore.setState({
      doc: null,
      faces: [],
      errorMessage: null,
      newDialogOpen: true,
      newPaperDraft: {
        widthMm: paper.width_mm,
        heightMm: paper.height_mm,
        square: false,
      },
    });
    delete window.__ori3Web;
    installOri3WebBridge(window, createWebBridgeDependencies(core));
    const root = createRoot(host);
    await act(async () => root.render(createElement(NewDocumentDialog)));

    try {
      expect(window.__ori3Web).toEqual(
        expect.objectContaining({ invoke: expect.any(Function) }),
      );

      const created = waitForPaper(paper);
      await act(async () => {
        buttonNamed("この紙で作りはじめる").click();
        await created;
      });

      const state = useAppStore.getState();
      expect(state.doc?.paper).toEqual(paper);
      expect(state.faces).toHaveLength(1);
      expect(state.docEpoch).toBe(beforeEpoch + 1);
      expect(state.errorMessage).toBeNull();
      expect(state.frame3d).toBeNull();
      expect(state.newDialogOpen).toBe(false);

      const diagonal = loadParityFixture("edit-apply-diagonal-150x100.json");
      const removed = loadParityFixture(
        "edit-apply-batch-remove-diagonal-150x100.json",
      );
      const applied = await editApply({
        type: "AddSegment",
        a: [0, 0],
        b: [1, 2 / 3],
        kind: "Mountain",
      });
      const batched = await editApplyBatch([
        { type: "SetEdgeKind", ids: [4], kind: "Valley" },
        { type: "RemoveEdges", ids: [4] },
      ]);
      const undone = await editUndo();
      const redone = await editRedo();

      expect(applied).toEqual(diagonal);
      expect(batched).toEqual(removed);
      expect(undone).toEqual(diagonal);
      expect(redone).toEqual(removed);

      const resetForSequence = await documentNew(paper);
      expect(resetForSequence).toEqual(loadDocumentNewParityFixture());
      const previewOperation: SeqOp = {
        type: "PreviewFoldThrough",
        up_to: 0,
        line: [
          [0, 0],
          [1, 2 / 3],
        ],
        keep_side_point: [0, 2 / 3],
        target_layers: null,
        direction: "Up",
      };
      const applyOperation: SeqOp = {
        type: "FoldThrough",
        up_to: 0,
        line: [
          [0, 0],
          [1, 2 / 3],
        ],
        keep_side_point: [0, 2 / 3],
        target_layers: null,
        direction: "Up",
        accept_additional_crease: false,
      };
      const previewFixture = loadParityFixture(
        "sequence-preview-fold-through-150x100.json",
      );
      const applyFixture = loadParityFixture(
        "sequence-apply-fold-through-150x100.json",
      );
      const replayFixture = loadParityFixture(
        "sequence-replay-fold-through-half-150x100.json",
      );
      const previewed = await sequenceApply(previewOperation);
      const sequenceApplied = await sequenceApply(applyOperation);
      const replayed = await sequenceReplay(1, 0.5, null);

      expect(previewed).toEqual(previewFixture);
      expect(sequenceApplied).toEqual(applyFixture);
      expect(replayed).toEqual(replayFixture);

      await useAppStore.getState().newDocument(paper);
      expect(useAppStore.getState().doc).toEqual(
        (loadDocumentNewParityFixture() as { doc: unknown }).doc,
      );
      await useAppStore.getState().applyEdit({
        type: "AddSegment",
        a: [0, 0],
        b: [1, 2 / 3],
        kind: "Mountain",
      });
      expect(useAppStore.getState().doc).toEqual(
        (diagonal as { doc: unknown }).doc,
      );
      const poseFixture = loadParityFixture(
        "pose-solve-diagonal-150x100.json",
      );
      const foldAllZeroFixture = loadParityFixture(
        "fold-all-preview-diagonal-0-150x100.json",
      );
      const foldAllFixture = loadParityFixture(
        "fold-all-preview-diagonal-50-150x100.json",
      );

      useAppStore.getState().setDriverAngle(4, 90);
      await useAppStore.getState().finishAngleIntent();
      expect(useAppStore.getState().errorMessage).toBeNull();
      expect(useAppStore.getState().poseAngles.get(4)).toBe(90);

      await useAppStore.getState().enterFoldAllPreview();
      expect(useAppStore.getState().foldAllPreview?.appliedPercent).toBe(0);
      const fiftyPercent = waitForFoldAllPercent(50);
      useAppStore.getState().setFoldAllPercent(50);
      useAppStore.getState().finishFoldAllPercent();
      await fiftyPercent;
      expect(useAppStore.getState().frame3d).toEqual(
        (foldAllFixture as { frame: unknown }).frame,
      );
      expect(loopback?.requests).toEqual([
        {
          type: "invoke",
          id: 1,
          command: "document_new",
          args: { paper },
        },
        {
          type: "invoke",
          id: 2,
          command: "edit_apply",
          args: {
            op: {
              type: "AddSegment",
              a: [0, 0],
              b: [1, 2 / 3],
              kind: "Mountain",
            },
          },
        },
        {
          type: "invoke",
          id: 3,
          command: "edit_apply_batch",
          args: {
            ops: [
              { type: "SetEdgeKind", ids: [4], kind: "Valley" },
              { type: "RemoveEdges", ids: [4] },
            ],
          },
        },
        { type: "invoke", id: 4, command: "edit_undo" },
        { type: "invoke", id: 5, command: "edit_redo" },
        {
          type: "invoke",
          id: 6,
          command: "document_new",
          args: { paper },
        },
        {
          type: "invoke",
          id: 7,
          command: "sequence_apply",
          args: { op: previewOperation },
        },
        {
          type: "invoke",
          id: 8,
          command: "sequence_apply",
          args: { op: applyOperation },
        },
        {
          type: "invoke",
          id: 9,
          command: "sequence_replay",
          args: { upTo: 1, t: 0.5, soft: null },
        },
        {
          type: "invoke",
          id: 10,
          command: "document_new",
          args: { paper },
        },
        {
          type: "invoke",
          id: 11,
          command: "edit_apply",
          args: {
            op: {
              type: "AddSegment",
              a: [0, 0],
              b: [1, 2 / 3],
              kind: "Mountain",
            },
          },
        },
        {
          type: "invoke",
          id: 12,
          command: "pose_solve",
          args: {
            request: {
              hard: [{ hinge: 4, target_angle_deg: 90 }],
              preferred: null,
              warmSeed: null,
              soft: null,
              upTo: 0,
              t: 1,
              mode: "Follow",
            },
          },
        },
        {
          type: "invoke",
          id: 13,
          command: "pose_solve",
          args: {
            request: {
              hard: [],
              preferred: [{ hinge: 4, target_angle_deg: 90 }],
              warmSeed: [{ hinge: 4, target_angle_deg: 90 }],
              soft: null,
              upTo: 0,
              t: 1,
              mode: "Canonical",
            },
          },
        },
        {
          type: "invoke",
          id: 14,
          command: "fold_all_preview",
          args: { percent: 0, warmSeed: null },
        },
        {
          type: "invoke",
          id: 15,
          command: "fold_all_preview",
          args: {
            percent: 50,
            warmSeed: [{ hinge: 4, target_angle_deg: 0 }],
          },
        },
      ]);
      expect(loopback?.responses).toEqual([
        { type: "ready" },
        {
          type: "result",
          id: 1,
          ok: true,
          value: loadDocumentNewParityFixture(),
        },
        { type: "result", id: 2, ok: true, value: diagonal },
        { type: "result", id: 3, ok: true, value: removed },
        { type: "result", id: 4, ok: true, value: diagonal },
        { type: "result", id: 5, ok: true, value: removed },
        {
          type: "result",
          id: 6,
          ok: true,
          value: loadDocumentNewParityFixture(),
        },
        { type: "result", id: 7, ok: true, value: previewFixture },
        { type: "result", id: 8, ok: true, value: applyFixture },
        { type: "result", id: 9, ok: true, value: replayFixture },
        {
          type: "result",
          id: 10,
          ok: true,
          value: loadDocumentNewParityFixture(),
        },
        { type: "result", id: 11, ok: true, value: diagonal },
        { type: "result", id: 12, ok: true, value: poseFixture },
        { type: "result", id: 13, ok: true, value: poseFixture },
        {
          type: "result",
          id: 14,
          ok: true,
          value: foldAllZeroFixture,
        },
        { type: "result", id: 15, ok: true, value: foldAllFixture },
      ]);
    } finally {
      await act(async () => root.unmount());
      host.remove();
      core.dispose();
      backend.free();
      delete window.__ori3Web;
      if (previousActEnvironment === undefined) {
        delete actEnvironment.IS_REACT_ACT_ENVIRONMENT;
      } else {
        actEnvironment.IS_REACT_ACT_ENVIRONMENT = previousActEnvironment;
      }
    }
  });

  it("既存作品を開き、同じWorker stateを保存し、折り図SVGとPDFをRust成果物のまま配送する", async () => {
    const backend = loadRealWasmBackend();
    let loopback: LoopbackCoreWorker | undefined;
    const core: Ori3CoreWorkerClient = createOri3CoreWorkerClient(() => {
      loopback = new LoopbackCoreWorker(backend);
      return loopback as unknown as Worker;
    });
    const downloads: Array<{ name: string; bytes: Uint8Array }> = [];
    const registry = createBrowserFileTokenRegistry(async (blob, name) => {
      downloads.push({
        name,
        bytes: new Uint8Array(await blob.arrayBuffer()),
      });
    });
    const currentDocument =
      createBrowserCurrentDocumentCoordinator(registry);
    const dependencies = createWebBridgeDependencies(core);
    dependencies.core = createDocumentLifecycleCoreInvoker(
      core,
      currentDocument,
    );
    dependencies.mixed = createBrowserDocumentInvoker(core, {
      registry,
      currentDocument,
    });
    const fixture = loadParityFixture(
      "sequence-apply-fold-through-150x100.json",
    );
    const savedSource = savedDocumentOf(fixture);
    const openToken = registry.registerRead(
      new File([savedSource], "既存作品.ori3", {
        type: "application/json",
      }),
    );
    const savedBytes: Uint8Array[] = [];
    const saveHandle = {
      name: "保存作品.ori3",
      async createWritable() {
        return {
          async write(value: Blob) {
            savedBytes.push(new Uint8Array(await value.arrayBuffer()));
          },
          async close() {},
        };
      },
    } as unknown as FileSystemFileHandle;
    const saveToken = registry.registerFileSystemDestination(saveHandle);
    const directoryFiles = new Map<string, Uint8Array>();
    const directoryHandle = {
      name: "折り図",
      async getFileHandle(name: string) {
        return {
          name,
          async createWritable() {
            return {
              async write(value: Blob) {
                directoryFiles.set(
                  name,
                  new Uint8Array(await value.arrayBuffer()),
                );
              },
              async close() {},
            };
          },
        };
      },
    } as unknown as FileSystemDirectoryHandle;
    const directoryToken = registry.registerDirectoryDestination(
      directoryHandle,
      "作品.svg",
    );
    const pdfToken = registry.registerDownload("作品.pdf");

    delete window.__ori3Web;
    installOri3WebBridge(window, dependencies);
    try {
      expect(await documentOpen(openToken)).toEqual(fixture);

      await documentSave(saveToken);
      expect(savedBytes).toHaveLength(1);
      const firstSavedSource = new TextDecoder().decode(savedBytes[0]);
      expect(JSON.parse(firstSavedSource)).toEqual(JSON.parse(savedSource));
      expect(firstSavedSource).toContain('"width_mm": 150.0');
      await documentSave(null);
      expect(savedBytes).toHaveLength(2);
      expect(savedBytes[1]).toEqual(savedBytes[0]);

      expect(
        await documentExport("DiagramSvg", directoryToken, {
          include_aux: true,
          png_long_side: 2048,
        }),
      ).toEqual([]);
      expect([...directoryFiles.keys()]).toEqual([
        "作品-01.svg",
        "作品-02.svg",
      ]);
      for (const bytes of directoryFiles.values()) {
        expect(new TextDecoder().decode(bytes)).toMatch(
          /^<\?xml version="1\.0" encoding="UTF-8"\?>/,
        );
      }

      expect(
        await documentExport("DiagramPdf", pdfToken, {
          include_aux: true,
          png_long_side: 2048,
        }),
      ).toEqual([]);
      expect(downloads).toHaveLength(1);
      expect(downloads[0].name).toBe("作品.pdf");
      expect(new TextDecoder().decode(downloads[0].bytes.slice(0, 5))).toBe(
        "%PDF-",
      );
      await documentExport("DiagramPdf", pdfToken, {
        include_aux: true,
        png_long_side: 2048,
      });
      expect(downloads).toHaveLength(2);
      expect(downloads[1]).toEqual(downloads[0]);
      const firstPdfHash = sha256(downloads[0].bytes);
      expect(downloads[0].bytes).toHaveLength(39_546);
      expect(firstPdfHash).toBe(
        "9F61A6943FF743FEC9B64A1466399AA6EA1F5D879E2D26753A6A8ECBEB6B0935",
      );
      expect(fnv1a64(downloads[0].bytes)).toBe("5AA38F199E360526");

      expect(loopback?.requests.map((request) => request.command)).toEqual([
        "__web_document_open_source",
        "document_open",
        "__web_document_save_prepare",
        "document_save",
        "__web_document_save_prepare",
        "document_save",
        "__web_document_export_prepare",
        "__web_document_export_prepare",
        "__web_document_export_prepare",
      ]);
    } finally {
      registry.release(openToken);
      registry.release(saveToken);
      registry.release(directoryToken);
      registry.release(pdfToken);
      core.dispose();
      backend.free();
      delete window.__ori3Web;
    }
  });

  it("runs the bird-base proposal through the product Worker and authoritative core", async () => {
    const corpus = loadBirdBaseProposalCorpus();
    expect(corpus.with_fold_plan).toBe(true);

    const coreBackend = loadRealWasmBackend();
    const directBackend = new Ori3WasmBackend();
    const proposalBackends: Ori3WasmBackend[] = [];
    const proposalLoopbacks: LoopbackProposalWorker[] = [];
    const core = createOri3CoreWorkerClient(
      () =>
        new LoopbackCoreWorker(coreBackend) as unknown as Worker,
    );
    const proposal = createProposalJobRegistry(() => {
      const backend = new Ori3WasmBackend();
      const worker = new LoopbackProposalWorker(backend);
      proposalBackends.push(backend);
      proposalLoopbacks.push(worker);
      return worker as unknown as Worker;
    });
    const jobId = "bird-base-product";
    const directRequest = JSON.stringify({
      command: "proposal_generate",
      args: {
        jobId,
        skeleton: corpus.skeleton,
        paper: corpus.paper,
        seed: corpus.seed,
        withFoldPlan: corpus.with_fold_plan,
      },
    });

    delete window.__ori3Web;
    installOri3WebBridge(
      window,
      createWebBridgeDependencies(core, proposal),
    );
    try {
      const directStartedAt = performance.now();
      const directJson = directBackend.invoke_json(directRequest);
      const directElapsedMs = performance.now() - directStartedAt;
      const directBytes = new TextEncoder().encode(directJson);
      const direct = JSON.parse(directJson) as ProposalJobResult;

      expect(directBytes).toHaveLength(32_037);
      expect(sha256(directBytes)).toBe(
        "2AA44347E5982C5EDAC60806B36DA48034E66EC4056505F2879C09B7FA73D47B",
      );
      expect(fnv1a64(directBytes)).toBe("28CCC5CEF500E5BC");
      expect(direct.job_id).toBe(jobId);
      expect(direct.candidates).toHaveLength(4);
      expect(directElapsedMs).toBeLessThan(30_000);

      const beforeApply = await documentNew(corpus.paper);
      const workerStartedAt = performance.now();
      const generated = await proposalGenerate(
        corpus.skeleton,
        corpus.paper,
        corpus.seed,
        jobId,
      );
      const workerElapsedMs = performance.now() - workerStartedAt;

      expect(generated).toEqual(direct);
      expect(generated.candidates).toHaveLength(4);
      expect(workerElapsedMs).toBeLessThan(30_000);
      expect(await proposalProgress(jobId)).toBeNull();

      const proposalWorker = proposalLoopbacks[0];
      if (!proposalWorker) throw new Error("proposal Worker was not created");
      const ready = proposalWorker.responses.find(
        ({ response }) => response.type === "ready",
      );
      if (!ready) throw new Error("proposal Worker did not become ready");
      const readyElapsedMs = ready.at - proposalWorker.createdAt;
      expect(readyElapsedMs).toBeLessThan(30_000);

      const progress: Array<{
        at: number;
        snapshot: ProposalProgressSnapshot;
      }> = [];
      for (const event of proposalWorker.responses) {
        if (event.response.type === "progress") {
          progress.push({
            at: event.at,
            snapshot: event.response.snapshot,
          });
        }
      }
      expect(progress.map(({ snapshot }) => snapshot)).toEqual([
        { job_id: jobId, done: 0, total: 4, phase: "Generating" },
        { job_id: jobId, done: 0, total: 4, phase: "Verifying" },
        { job_id: jobId, done: 1, total: 4, phase: "Verifying" },
        { job_id: jobId, done: 2, total: 4, phase: "Verifying" },
        { job_id: jobId, done: 3, total: 4, phase: "Verifying" },
        { job_id: jobId, done: 4, total: 4, phase: "Verifying" },
      ]);
      const candidateElapsedMs = progress
        .slice(2)
        .map((event, index) => event.at - progress[index + 1].at);
      expect(candidateElapsedMs).toHaveLength(4);
      expect(Math.max(...candidateElapsedMs)).toBeLessThan(30_000);

      const plan = generated.candidates[0]?.fold_plan;
      if (!plan) throw new Error("bird-base candidate has no fold plan");
      const applied = await proposalApply(plan.cp, plan.steps);
      expect(applied.doc.cp).toEqual(plan.cp);
      expect(applied.doc.sequence).toEqual(plan.steps);
      expect((await editUndo()).doc).toEqual(beforeApply.doc);

      const cancelJobId = "bird-base-cancel";
      const cancelledGeneration = proposalGenerate(
        corpus.skeleton,
        corpus.paper,
        corpus.seed,
        cancelJobId,
      );
      const cancellation = expect(cancelledGeneration).rejects.toBeTruthy();
      expect(
        await proposalControl({
          type: "Cancel",
          job_id: cancelJobId,
        }),
      ).toEqual({
        job_id: cancelJobId,
        done: 0,
        total: 0,
        phase: "Cancelled",
      });
      await cancellation;
      expect(await proposalProgress(cancelJobId)).toBeNull();

      console.info(
        [
          "PROPOSAL_ACTUAL",
          "candidates=4",
          "progress=0G,0V,1V,2V,3V,4V",
          `ready_ms=${readyElapsedMs.toFixed(2)}`,
          `direct_ms=${directElapsedMs.toFixed(2)}`,
          `worker_ms=${workerElapsedMs.toFixed(2)}`,
          `candidate_max_ms=${Math.max(...candidateElapsedMs).toFixed(2)}`,
        ].join(" "),
      );
    } finally {
      proposal.dispose();
      core.dispose();
      for (const backend of proposalBackends) backend.free();
      directBackend.free();
      coreBackend.free();
      delete window.__ori3Web;
    }
  }, 30_000);
});

class RecoveryIntegrationClock implements AutosaveClock {
  private next = 1;
  setTimeout(): unknown { return this.next++; }
  clearTimeout(): void {}
  setInterval(): unknown { return this.next++; }
  clearInterval(): void {}
}

class RecoveryIntegrationRepository implements WebRecoveryRepositoryPort {
  readonly records = new Map<number, RecoveryCandidateSource>();
  readonly discarded: number[] = [];
  readonly cleared: number[] = [];
  private nextId = 1;
  private savedAt = 10_000;

  add(candidateId: number, source: string, savedAt: number): void {
    const parsed = JSON.parse(source) as { sequence?: unknown[] };
    this.addRaw(candidateId, source, savedAt, parsed.sequence?.length ?? 0);
  }

  addRaw(
    candidateId: number,
    source: string,
    savedAt: number,
    stepCount: number | null,
  ): void {
    this.records.set(candidateId, {
      candidate: {
        candidate_id: candidateId,
        autosave_path: `browser-recovery://candidate/${candidateId}`,
        document_path: null,
        saved_at_ms: savedAt,
        step_count: stepCount,
      },
      source,
    });
  }

  async listCandidates(): Promise<RecoveryCandidateSummary[]> {
    return [...this.records.values()].map(({ candidate }) => ({ ...candidate }));
  }

  async readCandidateSource(candidateId: number): Promise<RecoveryCandidateSource> {
    const value = this.records.get(candidateId);
    if (value === undefined) throw "選んだ復旧候補は保存領域に見つかりません。";
    return { candidate: { ...value.candidate }, source: value.source };
  }

  async saveCheckpoint(
    source: SavedDocumentSource,
    options: { candidateId: number | null; documentPath: string | null },
  ): Promise<RecoveryCandidateSummary> {
    const candidateId = options.candidateId ?? this.nextId++;
    const candidate: RecoveryCandidateSummary = {
      candidate_id: candidateId,
      autosave_path: `browser-recovery://candidate/${candidateId}`,
      document_path: options.documentPath,
      saved_at_ms: this.savedAt++,
      step_count: source.doc.sequence.length,
    };
    this.records.set(candidateId, {
      candidate,
      source: serializeSavedDocument(source),
    });
    return candidate;
  }

  async discardCandidate(candidateId: number): Promise<void> {
    if (!this.records.delete(candidateId)) throw "選んだ復旧候補は保存領域に見つかりません。";
    this.discarded.push(candidateId);
  }

  async clearCandidateAfterExplicitSaveSucceeded(candidateId: number): Promise<void> {
    if (!this.records.delete(candidateId)) throw "選んだ復旧候補は保存領域に見つかりません。";
    this.cleared.push(candidateId);
  }
}

describe("Web復旧のproduction bridgeから実WASMまでの往復", () => {
  it("A復元後の編集を別候補へ控え、保存でAとactiveだけを消す", async () => {
    const backend = loadRealWasmBackend();
    let loopback: LoopbackCoreWorker | undefined;
    const core = createOri3CoreWorkerClient(() => {
      loopback = new LoopbackCoreWorker(backend);
      return loopback as unknown as Worker;
    });
    const downloads: string[] = [];
    const registry = createBrowserFileTokenRegistry((_blob, name) => {
      downloads.push(name);
    });
    const currentDocument = createBrowserCurrentDocumentCoordinator(registry);
    const repository = new RecoveryIntegrationRepository();
    const errors: string[] = [];
    const recovery = createWebRecoveryRuntime(core, {
      currentDocument,
      registry,
      repository,
      onError: (message) => errors.push(message),
      clock: new RecoveryIntegrationClock(),
    });
    const proposal = createProposalJobRegistry();
    const dependencies: WebBridgeDependencies = {
      core: recovery.decorateCore(
        createDocumentLifecycleCoreInvoker(core, currentDocument),
      ),
      proposal,
      browser: recovery.browser,
      mixed: recovery.decorateMixed(
        createBrowserDocumentInvoker(core, { currentDocument, registry }),
      ),
    };
    delete window.__ori3Web;
    installOri3WebBridge(window, dependencies);

    try {
      await documentNew({ width_mm: 150, height_mm: 100 });
      const saveName = registry.registerDownload("折り鶴.ori3");
      currentDocument.adopt(saveName);
      registry.release(saveName);
      const original = await editApply({
        type: "AddSegment",
        a: [0, 0],
        b: [1, 2 / 3],
        kind: "Mountain",
      });
      await recovery.flushAutosave();
      const sourceA = repository.records.get(1);
      if (sourceA === undefined) throw new Error("候補Aがありません");
      expect(sourceA.candidate.document_path).toBe("折り鶴.ori3");
      repository.add(99, sourceA.source, 1);

      await documentNew({ width_mm: 100, height_mm: 100 });
      const choices = await recoveryCheck();
      expect(choices).toEqual({
        choices: [sourceA.candidate, repository.records.get(99)!.candidate],
        overflow_count: 0,
      });

      const restored = await recoveryRestore(true, 1);
      expect(restored?.doc).toEqual(original.doc);
      expect(repository.records.has(1)).toBe(true);
      expect(recovery.associatedCandidateIds()).toEqual([1]);
      expect(registry.nameOf(currentDocument.current()!)).toBe("折り鶴.ori3");

      await editApply({
        type: "AddSegment",
        a: [0, 1 / 3],
        b: [1, 1 / 3],
        kind: "Valley",
      });
      await recovery.flushAutosave();
      expect(recovery.associatedCandidateIds()).toEqual([1, 2]);

      await documentSave(null);
      expect(repository.cleared).toEqual([1, 2]);
      expect(repository.records.has(99)).toBe(true);
      expect(repository.records.has(2)).toBe(false);
      expect(downloads).toEqual(["折り鶴.ori3"]);

      await recoveryRestore(false, 99);
      expect(repository.discarded).toEqual([99]);
      expect(repository.records.size).toBe(0);

      await core.invoke("edit_apply", {
        op: {
          type: "AddSegment",
          a: [0, 1],
          b: [1, 0],
          kind: "Aux",
        },
      });
      const dirty = await core.invoke<SavedDocumentSource | null>(
        "__web_recovery_snapshot",
        {},
      );
      expect(dirty).not.toBeNull();
      repository.addRaw(77, "{", 20_000, null);
      const beforeToken = currentDocument.current();
      await expect(recoveryRestore(true, 77)).rejects.toBeTruthy();
      expect(
        await core.invoke<SavedDocumentSource | null>(
          "__web_recovery_snapshot",
          {},
        ),
      ).toEqual(dirty);
      expect(repository.records.has(77)).toBe(true);
      expect(currentDocument.current()).toBe(beforeToken);
      expect(errors).toEqual([]);

      if (loopback === undefined) throw new Error("core Workerがありません");
      const commands = loopback.requests.map((request) => request.command);
      const restoreSourceAt = commands.indexOf("__web_recovery_restore_source");
      expect(commands.slice(restoreSourceAt - 1, restoreSourceAt + 2)).toEqual([
        "__web_recovery_set_choices",
        "__web_recovery_restore_source",
        "recovery_restore",
      ]);
      const discardAt = commands.findIndex(
        (command, index) =>
          command === "recovery_restore" && index > restoreSourceAt + 1,
      );
      expect(commands.slice(discardAt - 1, discardAt + 1)).toEqual([
        "__web_recovery_set_choices",
        "recovery_restore",
      ]);
    } finally {
      recovery.dispose();
      proposal.dispose();
      core.dispose();
      backend.free();
      delete window.__ori3Web;
    }
  }, 30_000);
});
