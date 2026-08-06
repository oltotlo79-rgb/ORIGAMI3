// @vitest-environment jsdom
// 書き出しダイアログの画面テスト(EXP-001〜EXP-004、Task 4-3/4-4/4-5):
// 閉じているときは何も出さない、種類を選んで保存すると document_export が
// 正しい引数で飛ぶ、成功・失敗の知らせが日本語で出る。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));
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
  documentExport: vi.fn(),
}));

import { save } from "@tauri-apps/plugin-dialog";
import * as ipc from "../../ipc/client";
import { ExportDialog, EXPORT_CHOICES, NO_STEPS_REASON } from "./ExportDialog";
import { useAppStore } from "../../store/appStore";
import type { Document } from "../../lib/types";

/** 手順が `steps` 個ある作品(折り図が書き出せる状態にするための土台) */
function docWithSteps(steps: number): Document {
  return {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: { vertices: [], edges: [], next_vertex_id: 0, next_edge_id: 0 },
    sequence: Array.from({ length: steps }, (_, i) => ({
      id: i + 1,
      kind: "Simple" as const,
      drivers: [],
      layer_order: null,
      note: "",
    })),
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
}

const saveMock = vi.mocked(save);
const exportMock = vi.mocked(ipc.documentExport);

beforeEach(() => {
  vi.clearAllMocks();
  exportMock.mockResolvedValue(undefined);
  useAppStore.setState({
    exportOpen: true,
    exportKind: "CpSvg",
    exportIncludeAux: true,
    exportLongSide: 2048,
    exportBusy: false,
    exportError: null,
    exportSavedPath: null,
    doc: docWithSteps(3),
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState({ exportOpen: false });
});

describe("書き出しダイアログ", () => {
  it("閉じているときは何も出さない", () => {
    useAppStore.setState({ exportOpen: false });
    const { container } = render(<ExportDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("選べる種類は展開図2つと折り図2つ", () => {
    render(<ExportDialog />);
    expect(EXPORT_CHOICES.map((c) => c.kind)).toEqual([
      "CpSvg",
      "CpPng",
      "DiagramPdf",
      "DiagramSvg",
    ]);
    for (const c of EXPORT_CHOICES) {
      expect(screen.getByRole("radio", { name: c.label })).not.toBeNull();
    }
    // PNGを選んでいないうちは大きさの入力は出さない
    expect(screen.queryByLabelText(/画像の大きさ/)).toBeNull();
  });

  it("SVGは補助線の有無だけを引数にして書き出す", async () => {
    saveMock.mockResolvedValue("C:\\出力\\鶴.svg");
    render(<ExportDialog />);
    fireEvent.click(screen.getByLabelText("補助線(下書きの線)も含める"));
    fireEvent.click(screen.getByRole("button", { name: "保存先を選んで書き出す" }));

    await waitFor(() =>
      expect(exportMock).toHaveBeenCalledWith("CpSvg", "C:\\出力\\鶴.svg", {
        include_aux: false,
        png_long_side: 2048,
      }),
    );
    expect(await screen.findByText(/保存しました/)).not.toBeNull();
  });

  it("PNGを選ぶと大きさを指定でき、その点数が渡る", async () => {
    saveMock.mockResolvedValue("C:\\出力\\鶴.png");
    render(<ExportDialog />);
    fireEvent.click(screen.getByRole("radio", { name: "展開図の画像(PNG)" }));
    const size = screen.getByLabelText(/画像の大きさ/) as HTMLInputElement;
    expect(size.value).toBe("2048"); // 既定は長辺2048点
    fireEvent.change(size, { target: { value: "1024" } });
    fireEvent.click(screen.getByRole("button", { name: "保存先を選んで書き出す" }));

    await waitFor(() =>
      expect(exportMock).toHaveBeenCalledWith("CpPng", "C:\\出力\\鶴.png", {
        include_aux: true,
        png_long_side: 1024,
      }),
    );
  });

  it("保存先を選ばずに閉じたら何も書き出さない", async () => {
    saveMock.mockResolvedValue(null);
    render(<ExportDialog />);
    fireEvent.click(screen.getByRole("button", { name: "保存先を選んで書き出す" }));
    await waitFor(() => expect(saveMock).toHaveBeenCalled());
    expect(exportMock).not.toHaveBeenCalled();
  });

  it("失敗したら日本語の理由を出す", async () => {
    saveMock.mockResolvedValue("C:\\出力\\鶴.png");
    exportMock.mockRejectedValue("ファイルに書き出せませんでした: 書き込めません");
    render(<ExportDialog />);
    fireEvent.click(screen.getByRole("button", { name: "保存先を選んで書き出す" }));
    expect(await screen.findByText(/保存できませんでした/)).not.toBeNull();
    expect(screen.getByText(/書き込めません/)).not.toBeNull();
  });
  it("折り図(PDF)を選ぶと種類だけを変えて書き出す", async () => {
    saveMock.mockResolvedValue("C:/出力/鶴の折り図.pdf");
    render(<ExportDialog />);
    fireEvent.click(screen.getByRole("radio", { name: "折り図(PDF)" }));
    fireEvent.click(screen.getByRole("button", { name: "保存先を選んで書き出す" }));

    await waitFor(() =>
      expect(exportMock).toHaveBeenCalledWith("DiagramPdf", "C:/出力/鶴の折り図.pdf", {
        include_aux: true,
        png_long_side: 2048,
      }),
    );
    // 折り図には補助線の指定は関係ないので出さない
    expect(screen.queryByLabelText(/補助線/)).toBeNull();
  });

  it("折り図(画像)はページごとに分かれると説明したうえで書き出す", async () => {
    saveMock.mockResolvedValue("C:/出力/鶴.svg");
    render(<ExportDialog />);
    fireEvent.click(screen.getByRole("radio", { name: "折り図(画像・ページごと)" }));
    expect(screen.getByText(/「-01」「-02」/)).not.toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "保存先を選んで書き出す" }));

    await waitFor(() =>
      expect(exportMock).toHaveBeenCalledWith("DiagramSvg", "C:/出力/鶴.svg", {
        include_aux: true,
        png_long_side: 2048,
      }),
    );
  });

  it("手順が無いときは折り図を選べない理由を出す(選択肢もボタンも消さない)", () => {
    useAppStore.setState({ doc: docWithSteps(0), exportKind: "DiagramPdf" });
    render(<ExportDialog />);
    expect(screen.getByText(new RegExp(NO_STEPS_REASON))).not.toBeNull();
    for (const kind of ["折り図(PDF)", "折り図(画像・ページごと)"]) {
      const radio = screen.getByRole("radio", { name: kind }) as HTMLInputElement;
      expect(radio.disabled).toBe(true);
    }
    // 展開図の書き出しはそのまま選べる
    expect((screen.getByRole("radio", { name: "展開図の画像(SVG)" }) as HTMLInputElement).disabled)
      .toBe(false);
    const button = screen.getByRole("button", { name: "保存先を選んで書き出す" });
    expect(button).not.toBeNull();
    expect((button as HTMLButtonElement).disabled).toBe(true);
  });
});
