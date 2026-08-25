// @vitest-environment jsdom
// 書き出しダイアログの画面テスト(EXP-001〜EXP-004、Task 4-3/4-4/4-5):
// 閉じているときは何も出さない、種類を選んで保存すると document_export が
// 正しい引数で飛ぶ、成功・失敗の知らせが日本語で出る。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));
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
const realRunExport = useAppStore.getState().runExport;

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
    runExport: realRunExport,
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
    render(<ExportDialog />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("選べる種類は展開図2つと折り図2つ", () => {
    render(<ExportDialog />);
    expect(EXPORT_CHOICES).toHaveLength(4);
    expect(EXPORT_CHOICES.map(({ kind, label, ext }) => ({ kind, label, ext }))).toEqual([
      { kind: "CpSvg", label: "展開図(SVG)", ext: "svg" },
      { kind: "CpPng", label: "展開図(PNG)", ext: "png" },
      { kind: "DiagramPdf", label: "折り図(PDF)", ext: "pdf" },
      { kind: "DiagramSvg", label: "折り図(ページごとのSVG)", ext: "svg" },
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
    fireEvent.click(screen.getByRole("radio", { name: "展開図(PNG)" }));
    const size = screen.getByLabelText("画像の大きさ（長辺の点数）") as HTMLInputElement;
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

  it("PNGの大きさは上下ボタンで256点ずつ変わる", () => {
    render(<ExportDialog />);
    fireEvent.click(screen.getByRole("radio", { name: "展開図(PNG)" }));
    const size = screen.getByLabelText("画像の大きさ（長辺の点数）") as HTMLInputElement;

    fireEvent.click(
      screen.getByRole("button", { name: "画像の大きさ（長辺の点数）を増やす" }),
    );
    expect(size.value).toBe("2304");
    expect(useAppStore.getState().exportLongSide).toBe(2304);

    fireEvent.click(
      screen.getByRole("button", { name: "画像の大きさ（長辺の点数）を減らす" }),
    );
    expect(size.value).toBe("2048");
    expect(useAppStore.getState().exportLongSide).toBe(2048);
  });

  it("保存先を選ばずに閉じたら何も書き出さない", async () => {
    render(<ExportDialog />);
    const saveButton = screen.getByRole("button", { name: "保存先を選んで書き出す" });
    saveMock.mockImplementation(async () => {
      (saveButton as HTMLButtonElement).blur();
      return null;
    });
    saveButton.focus();
    fireEvent.click(saveButton);
    await waitFor(() => expect(saveMock).toHaveBeenCalled());
    expect(exportMock).not.toHaveBeenCalled();
    await waitFor(() => expect(document.activeElement).toBe(saveButton));
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
    fireEvent.click(screen.getByRole("radio", { name: "折り図(ページごとのSVG)" }));
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
    for (const kind of ["折り図(PDF)", "折り図(ページごとのSVG)"]) {
      const radio = screen.getByRole("radio", { name: kind }) as HTMLInputElement;
      expect(radio.disabled).toBe(true);
    }
    // 展開図の書き出しはそのまま選べる
    expect((screen.getByRole("radio", { name: "展開図(SVG)" }) as HTMLInputElement).disabled)
      .toBe(false);
    const button = screen.getByRole("button", { name: "保存先を選んで書き出す" });
    expect(button).not.toBeNull();
    expect((button as HTMLButtonElement).disabled).toBe(true);
  });

  it("PDFを画像と呼ばず、展開図と折り図の書き出し画面だと示す", () => {
    render(<ExportDialog />);
    expect(
      screen.getByRole("dialog", { name: "展開図・折り図を書き出す" }),
    ).not.toBeNull();
    expect(screen.queryByRole("heading", { name: "画像として書き出す" })).toBeNull();
  });

  it("選択中で有効な書き出し種類を最初に選び、既存の見た目用classを保つ", async () => {
    useAppStore.setState({ exportKind: "CpPng" });
    render(<ExportDialog />);

    const dialog = screen.getByRole("dialog", { name: "展開図・折り図を書き出す" });
    const selected = screen.getByRole("radio", { name: "展開図(PNG)" });
    await waitFor(() => expect(document.activeElement).toBe(selected));
    expect(dialog.className).toBe("dialog");
    expect(dialog.parentElement?.className).toBe("app dialog-backdrop");
  });

  it("選択中の種類が無効なら最初の有効な種類を最初に選ぶ", async () => {
    useAppStore.setState({ doc: docWithSteps(0), exportKind: "DiagramPdf" });
    render(<ExportDialog />);

    const firstEnabled = screen.getByRole("radio", { name: "展開図(SVG)" });
    await waitFor(() => expect(document.activeElement).toBe(firstEnabled));
    expect(
      (screen.getByRole("radio", { name: "折り図(PDF)" }) as HTMLInputElement).checked,
    ).toBe(true);
  });

  it("TabとShift+Tabを100回ずつ循環しても画面外へ出ない", async () => {
    render(<ExportDialog />);
    const first = screen.getByRole("radio", { name: "展開図(SVG)" });
    const last = screen.getByRole("button", { name: "閉じる" });
    await waitFor(() => expect(document.activeElement).toBe(first));

    for (let i = 0; i < 100; i += 1) {
      last.focus();
      fireEvent.keyDown(last, { key: "Tab" });
      expect(document.activeElement).toBe(first);
      fireEvent.keyDown(first, { key: "Tab", shiftKey: true });
      expect(document.activeElement).toBe(last);
    }
  });

  it("キーボードだけで開いて指定を変え、Escape後の背景と起点復帰を100回確かめる", async () => {
    useAppStore.setState({ exportOpen: false });
    const { container } = render(
      <>
        <button
          type="button"
          onClick={() => useAppStore.getState().openExport()}
        >
          書き出しを開く
        </button>
        <ExportDialog />
      </>,
    );
    const pointerDown = vi.fn();
    const mouseDown = vi.fn();
    document.addEventListener("pointerdown", pointerDown);
    document.addEventListener("mousedown", mouseDown);
    const trigger = screen.getByRole("button", { name: "書き出しを開く" });
    expect(trigger).toBeInstanceOf(HTMLButtonElement);
    expect((trigger as HTMLButtonElement).disabled).toBe(false);

    try {
      for (let cycle = 0; cycle < 100; cycle += 1) {
        trigger.focus();
        const enter = new KeyboardEvent("keydown", {
          key: "Enter",
          bubbles: true,
          cancelable: true,
        });
        fireEvent(trigger, enter);
        expect(enter.defaultPrevented).toBe(false);
        // jsdomはbuttonのEnter既定動作を作らないため、ブラウザが発生させるclickだけを代行する。
        if (!enter.defaultPrevented) act(() => (trigger as HTMLButtonElement).click());
        fireEvent.keyUp(trigger, { key: "Enter" });

        const first = screen.getByRole("radio", { name: "展開図(SVG)" });
        expect(document.activeElement).toBe(first);
        expect(container.hasAttribute("inert")).toBe(true);
        expect(
          screen
            .getByRole("dialog", { name: "展開図・折り図を書き出す" })
            .parentElement?.hasAttribute("inert"),
        ).toBe(false);

        if (cycle === 0) {
          const includeAux = screen.getByLabelText("補助線(下書きの線)も含める");
          expect(includeAux).toBeInstanceOf(HTMLInputElement);
          (includeAux as HTMLInputElement).focus();
          const space = new KeyboardEvent("keydown", {
            key: " ",
            bubbles: true,
            cancelable: true,
          });
          fireEvent(includeAux, space);
          expect(space.defaultPrevented).toBe(false);
          // jsdomはcheckboxの空白キー既定動作を作らないため、clickだけを代行する。
          if (!space.defaultPrevented) act(() => (includeAux as HTMLInputElement).click());
          fireEvent.keyUp(includeAux, { key: " " });
          expect(useAppStore.getState().exportIncludeAux).toBe(false);
        }

        fireEvent.keyDown(first, { key: "Escape" });
        expect(
          screen.queryByRole("dialog", { name: "展開図・折り図を書き出す" }),
        ).toBeNull();
        await Promise.resolve();
        expect(document.activeElement).toBe(trigger);
        expect(container.hasAttribute("inert")).toBe(false);
      }

      expect(pointerDown).toHaveBeenCalledTimes(0);
      expect(mouseDown).toHaveBeenCalledTimes(0);
    } finally {
      document.removeEventListener("pointerdown", pointerDown);
      document.removeEventListener("mousedown", mouseDown);
    }
  }, 15_000);

  it("書き出し中はEscapeで閉じず、既存の閉じるボタンはそのまま使える", async () => {
    useAppStore.setState({ exportBusy: true });
    render(<ExportDialog />);
    const first = screen.getByRole("radio", { name: "展開図(SVG)" });
    await waitFor(() => expect(document.activeElement).toBe(first));

    fireEvent.keyDown(first, { key: "Escape" });
    expect(useAppStore.getState().exportOpen).toBe(true);
    expect(screen.getByRole("dialog", { name: "展開図・折り図を書き出す" })).not.toBeNull();

    const closeButton = screen.getByRole("button", { name: "閉じる" }) as HTMLButtonElement;
    expect(closeButton.disabled).toBe(false);
    fireEvent.click(closeButton);
    expect(useAppStore.getState().exportOpen).toBe(false);
  });

  it("OSの保存先選択後は処理中の閉じるへ移り、完了後に保存へ戻る", async () => {
    let finishExport: (() => void) | null = null;
    const runExport = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          useAppStore.setState({ exportBusy: true });
          finishExport = () => {
            useAppStore.setState({ exportBusy: false });
            resolve();
          };
        }),
    );
    useAppStore.setState({ runExport });
    render(<ExportDialog />);
    const saveButton = screen.getByRole("button", {
      name: "保存先を選んで書き出す",
    });
    const closeButton = screen.getByRole("button", { name: "閉じる" });
    saveMock.mockImplementation(async () => {
      expect(document.activeElement).toBe(saveButton);
      (saveButton as HTMLButtonElement).blur();
      expect(document.activeElement).toBe(document.body);
      return "C:\\出力\\鶴.svg";
    });
    saveButton.focus();

    fireEvent.click(saveButton);
    await waitFor(() => expect(saveMock).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(runExport).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(document.activeElement).toBe(closeButton));
    expect((saveButton as HTMLButtonElement).disabled).toBe(true);

    act(() => finishExport?.());
    await waitFor(() => expect(document.activeElement).toBe(saveButton));
    expect((saveButton as HTMLButtonElement).disabled).toBe(false);
  });
});
