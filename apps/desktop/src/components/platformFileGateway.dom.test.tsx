// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Document } from "../lib/types";
import {
  installPlatformFileGateway,
  isBrowserDownloadToken,
  type BrowserFileToken,
  type PlatformFileGateway,
} from "../platform/fileGateway";
import {
  useAppStore,
  type FoldAllPreviewState,
} from "../store/appStore";
import { AppToolbar } from "./AppToolbar";
import { ContextPanel } from "./ContextPanel";
import { FoldAllPreviewContent } from "./contextPaperDisplay";
import { ExportDialog } from "./dialogs/ExportDialog";
import { fileName } from "./RecoveryDialog";

vi.mock("./HistoryButtons", () => ({ HistoryButtons: () => null }));
vi.mock("./ToolIcons", () => ({ ToolbarIcon: () => null }));
vi.mock("./ToolbarBrandMark", () => ({ ToolbarBrandMark: () => null }));

const DOCUMENT: Document = {
  schema_version: 1,
  paper: { width_mm: 150, height_mm: 150 },
  cp: { vertices: [], edges: [], next_vertex_id: 0, next_edge_id: 0 },
  sequence: [],
  display: {
    front_color: [237, 28, 36],
    back_color: [255, 255, 255],
    grid_divisions: 8,
  },
};

const realOpenDocument = useAppStore.getState().openDocument;
const realSaveDocument = useAppStore.getState().saveDocument;
const realRunExport = useAppStore.getState().runExport;
let restoreGateway: (() => void) | undefined;

function install(gateway: PlatformFileGateway): void {
  restoreGateway?.();
  restoreGateway = installPlatformFileGateway(gateway);
}

afterEach(() => {
  cleanup();
  restoreGateway?.();
  restoreGateway = undefined;
  useAppStore.setState({
    doc: null,
    errorMessage: null,
    documentSavedPath: null,
    openDocument: realOpenDocument,
    saveDocument: realSaveDocument,
    exportOpen: false,
    exportBusy: false,
    exportError: null,
    exportSavedPath: null,
    exportFoldIssues: [],
    foldAllPreview: null,
    runExport: realRunExport,
  });
});

describe("componentのplatform file入口", () => {
  it("開く権限を拒否されてもボタンと作品を残し、日本語の理由をstoreへ置く", async () => {
    const openDocument = vi.fn().mockResolvedValue(undefined);
    install({
      saveMode: "choose-destination",
      chooseOpenFile: vi
        .fn()
        .mockRejectedValue(new DOMException("denied", "NotAllowedError")),
      chooseSaveFile: vi.fn().mockResolvedValue(null),
      release: vi.fn(),
    });
    useAppStore.setState({
      doc: DOCUMENT,
      errorMessage: null,
      documentSavedPath: "前回.ori3",
      openDocument,
    });

    render(<AppToolbar onOpenHelp={vi.fn()} />);
    const button = screen.getByRole("button", { name: "開く" });
    fireEvent.click(button);

    await waitFor(() =>
      expect(useAppStore.getState().errorMessage).toBe(
        "ファイルを開く権限が許可されませんでした。作品は変更されていません。",
      ),
    );
    expect(useAppStore.getState().doc).toBe(DOCUMENT);
    expect(useAppStore.getState().documentSavedPath).toBeNull();
    expect(openDocument).not.toHaveBeenCalled();
    expect(button.isConnected).toBe(true);
  });

  it("保存先を選べないWeb環境では保存ボタンを消さずダウンロードと明記する", async () => {
    const token =
      "browser-file://download/test/作品.ori3" as BrowserFileToken;
    const saveDocument = vi.fn().mockResolvedValue(undefined);
    const chooseSaveFile = vi.fn().mockResolvedValue(token);
    install({
      saveMode: "download",
      chooseOpenFile: vi.fn().mockResolvedValue(null),
      chooseSaveFile,
      release: vi.fn(),
    });
    useAppStore.setState({ doc: DOCUMENT, saveDocument });

    render(<AppToolbar onOpenHelp={vi.fn()} />);
    const button = screen.getByRole("button", { name: "ダウンロード" });
    expect(button.getAttribute("data-tooltip")).toContain(
      "このブラウザでは保存先を選べないため",
    );
    fireEvent.click(button);

    await waitFor(() => expect(saveDocument).toHaveBeenCalledWith(token));
    expect(chooseSaveFile).toHaveBeenCalledWith({
      filters: [{ name: "ORIGAMI3作品", extensions: ["ori3"] }],
      suggestedName: "作品.ori3",
    });
    expect(useAppStore.getState().doc).toBe(DOCUMENT);
  });

  it("開く・保存のtokenはactionの成功と失敗のどちらでもsettle後に解放する", async () => {
    const openToken =
      "browser-file://read/open/折り鶴.ori3" as BrowserFileToken;
    const saveToken =
      "browser-file://file-system/save/折り鶴.ori3" as BrowserFileToken;
    const release = vi.fn();
    const openDocument = vi
      .fn()
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce("読込失敗");
    const saveDocument = vi
      .fn()
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce("保存失敗");
    install({
      saveMode: "choose-destination",
      chooseOpenFile: vi.fn().mockResolvedValue(openToken),
      chooseSaveFile: vi.fn().mockResolvedValue(saveToken),
      release,
    });
    useAppStore.setState({ doc: DOCUMENT, openDocument, saveDocument });
    render(<AppToolbar onOpenHelp={vi.fn()} />);

    const openButton = screen.getByRole("button", { name: "開く" });
    fireEvent.click(openButton);
    await waitFor(() => expect(release).toHaveBeenCalledTimes(1));
    fireEvent.click(openButton);
    await waitFor(() => expect(release).toHaveBeenCalledTimes(2));

    const saveButton = screen.getByRole("button", { name: "保存" });
    fireEvent.click(saveButton);
    await waitFor(() => expect(release).toHaveBeenCalledTimes(3));
    fireEvent.click(saveButton);
    await waitFor(() => expect(release).toHaveBeenCalledTimes(4));

    expect(release.mock.calls).toEqual([
      [openToken],
      [openToken],
      [saveToken],
      [saveToken],
    ]);
  });

  it("書き出し先の権限拒否でもrunExportせず、作品を残して理由を表示する", async () => {
    const runExport = vi.fn().mockResolvedValue(undefined);
    install({
      saveMode: "choose-destination",
      chooseOpenFile: vi.fn().mockResolvedValue(null),
      chooseSaveFile: vi
        .fn()
        .mockRejectedValue(new DOMException("denied", "NotAllowedError")),
      release: vi.fn(),
    });
    useAppStore.setState({
      doc: DOCUMENT,
      exportOpen: true,
      exportKind: "CpSvg",
      exportBusy: false,
      exportError: null,
      runExport,
    });

    render(<ExportDialog />);
    const button = screen.getByRole("button", {
      name: "保存先を選んで書き出す",
    });
    fireEvent.click(button);

    expect(
      await screen.findByText(
        "保存できませんでした:ファイルを保存する権限が許可されませんでした。作品は変更されていません。",
      ),
    ).not.toBeNull();
    expect(useAppStore.getState().doc).toBe(DOCUMENT);
    expect(runExport).not.toHaveBeenCalled();
    expect(button.isConnected).toBe(true);
  });

  it("書き出しtokenもrunExportの成功と失敗のどちらでもsettle後に解放する", async () => {
    const token =
      "browser-file://download/export/水風船.svg" as BrowserFileToken;
    const release = vi.fn();
    const runExport = vi
      .fn()
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce("書出失敗");
    install({
      saveMode: "download",
      chooseOpenFile: vi.fn().mockResolvedValue(null),
      chooseSaveFile: vi.fn().mockResolvedValue(token),
      release,
    });
    useAppStore.setState({
      doc: DOCUMENT,
      exportOpen: true,
      exportKind: "CpSvg",
      exportBusy: false,
      exportError: null,
      runExport,
    });
    render(<ExportDialog />);
    const button = screen.getByRole("button", { name: "ダウンロード" });

    fireEvent.click(button);
    await waitFor(() => expect(release).toHaveBeenCalledTimes(1));
    fireEvent.click(button);
    await waitFor(() => expect(release).toHaveBeenCalledTimes(2));

    expect(release).toHaveBeenNthCalledWith(1, token);
    expect(release).toHaveBeenNthCalledWith(2, token);
  });

  it("書き出しもsave picker非対応なら理由とダウンロードボタンを出す", () => {
    install({
      saveMode: "download",
      chooseOpenFile: vi.fn().mockResolvedValue(null),
      chooseSaveFile: vi.fn().mockResolvedValue(null),
      release: vi.fn(),
    });
    useAppStore.setState({
      doc: DOCUMENT,
      exportOpen: true,
      exportKind: "CpSvg",
      exportBusy: false,
      exportError: null,
    });

    render(<ExportDialog />);

    expect(
      screen.getByText(
        "このブラウザでは保存先を選べないため、ファイルをダウンロードします。",
      ),
    ).not.toBeNull();
    expect(screen.getByRole("button", { name: "ダウンロード" })).not.toBeNull();
    expect(screen.queryByText("保存先へ上書き")).toBeNull();
  });

  it("折り図SVGはdirectory非対応なら複数ファイル指定のままZIP受取を明示する", async () => {
    const token =
      "browser-file://download/export/作品-折り図.zip" as BrowserFileToken;
    const chooseSaveFile = vi.fn().mockResolvedValue(token);
    const runExport = vi.fn().mockResolvedValue(undefined);
    install({
      saveMode: "choose-destination",
      multipleFileSaveMode: "download",
      chooseOpenFile: vi.fn().mockResolvedValue(null),
      chooseSaveFile,
      release: vi.fn(),
    });
    useAppStore.setState({
      doc: {
        ...DOCUMENT,
        sequence: [
          {
            id: 0,
            kind: "Simple",
            drivers: [],
            layer_order: null,
            note: "",
          },
        ],
      },
      exportOpen: true,
      exportKind: "DiagramSvg",
      exportBusy: false,
      exportError: null,
      runExport,
    });

    render(<ExportDialog />);

    expect(
      screen.getByText(
        "このブラウザでは複数ファイルの保存先を選べないため、折り図SVGをZIPでダウンロードします。",
      ),
    ).not.toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "ダウンロード" }));
    await waitFor(() =>
      expect(chooseSaveFile).toHaveBeenCalledWith({
        filters: [
          {
            name: "折り図(ページごとのSVG)",
            extensions: ["svg"],
          },
        ],
        suggestedName: "作品.svg",
        multipleFiles: true,
      }),
    );
    expect(runExport).toHaveBeenCalledWith(token);
  });

  it("download tokenの末尾名だけを、通常表示と一斉折り表示でダウンロード開始と知らせる", () => {
    const token =
      "browser-file://download/test/折り鶴.ori3" as BrowserFileToken;
    expect(isBrowserDownloadToken(token)).toBe(true);
    expect(isBrowserDownloadToken("C:\\作品\\折り鶴.ori3")).toBe(false);
    expect(fileName(token)).toBe("折り鶴.ori3");

    useAppStore.setState({
      documentSavedPath: token,
      errorMessage: null,
      foldAllPreview: null,
      warnings: [],
      foldIssues: [],
      poseWarnings: [],
      replayWarnings: [],
      flatFoldViolations: [],
      relaxations: [],
    });
    render(<ContextPanel />);
    // ファイル名が.user-text(利用者の文字は同梱フォント対象外)へ入っていること、
    // かつ文全体(案内文+ファイル名+案内文)が今までどおりであることを両方検査する。
    const savedName = screen.getByText("折り鶴.ori3", {
      selector: ".user-text",
    });
    expect(savedName.closest(".mirror-axis-notice")?.textContent).toBe(
      "作品を「折り鶴.ori3」としてダウンロードを開始しました",
    );

    cleanup();
    const preview: FoldAllPreviewState = {
      session: 1,
      percent: 50,
      appliedPercent: 50,
      busy: false,
      returning: false,
      error: null,
      converged: true,
      bestEffort: false,
      relaxationCount: 0,
      flatFoldViolationCount: 0,
      suspectHingeCount: 0,
      contactDetected: false,
      layerOrder: "unavailable_without_sequence",
      nextWarmSeed: [],
      returnState: {
        docEpoch: 0,
        currentStep: null,
        playT: 1,
        activeTool: "select",
        selection: { edgeIds: [], vertexIds: [] },
      },
    };
    useAppStore.setState({ foldAllPreview: preview });
    render(<FoldAllPreviewContent />);
    expect(
      screen.getByText(
        "作品のダウンロードを開始しました。いま見ている形は保存されません。",
      ),
    ).not.toBeNull();
  });
});
