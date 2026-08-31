// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import type { BrowserFileToken } from "../../../desktop/src/platform/fileGateway";
import {
  chooseFileWithInput,
  createBrowserPlatformFileGateway,
} from "./browserFileGateway";
import {
  createBrowserFileTokenRegistry,
  isBrowserFileToken,
} from "./browserFileTokenRegistry";

const FILTERS = [
  { name: "ORIGAMI3作品", extensions: ["ori3"] },
  { name: "ほかの折り紙ソフトのファイル", extensions: ["fold"] },
];
const MISSING_TOKEN_MESSAGE =
  "選んだファイルをこのタブで確認できませんでした。作品は変更されていません。もう一度ファイルを選んでください。";

afterEach(() => vi.restoreAllMocks());

describe("browser file gateway", () => {
  it("API非対応時の一時inputは対象拡張子だけを受け付け、cancelで取り除く", async () => {
    const click = vi
      .spyOn(HTMLInputElement.prototype, "click")
      .mockImplementation(function (this: HTMLInputElement) {
        expect(this.type).toBe("file");
        expect(this.multiple).toBe(false);
        expect(this.accept).toBe(".ori3,.fold");
        expect(this.hidden).toBe(true);
        this.dispatchEvent(new Event("cancel"));
      });

    await expect(chooseFileWithInput(document, FILTERS)).resolves.toBeNull();
    expect(click).toHaveBeenCalledTimes(1);
    expect(document.querySelector('input[type="file"]')).toBeNull();
  });

  it("File System Access APIで選んだread/write handleをtokenから読める", async () => {
    const file = new File(["作品"], "鶴.ori3", {
      type: "application/json",
    });
    const handle = {
      kind: "file",
      name: file.name,
      getFile: vi.fn().mockResolvedValue(file),
      createWritable: vi.fn(),
    };
    const showOpenFilePicker = vi.fn().mockResolvedValue([handle]);
    const registry = createBrowserFileTokenRegistry();
    const gateway = createBrowserPlatformFileGateway({
      pickerWindow: { showOpenFilePicker } as never,
      browserDocument: document,
      registry,
    });

    const selected = await gateway.chooseOpenFile({ filters: FILTERS });

    expect(selected).not.toBeNull();
    expect(isBrowserFileToken(selected as string)).toBe(true);
    expect(registry.destinationOf(selected as BrowserFileToken)).toBe(
      "file-system",
    );
    expect(await registry.read(selected as BrowserFileToken)).toBe(file);
    expect(handle.getFile).toHaveBeenCalledTimes(1);
    expect(showOpenFilePicker).toHaveBeenCalledWith({
      multiple: false,
      types: [
        {
          description: "ORIGAMI3作品",
          accept: { "application/json": [".ori3"] },
        },
        {
          description: "ほかの折り紙ソフトのファイル",
          accept: { "application/json": [".fold"] },
        },
      ],
    });
    gateway.release(selected as string);
    await expect(
      registry.read(selected as BrowserFileToken),
    ).rejects.toThrow(MISSING_TOKEN_MESSAGE);
  });

  it("open picker非対応ならinput fallbackを使い、cancelなら作品を開かない", async () => {
    const file = new File(["{}"], "やっこさん.ori3");
    const chooseWithInput = vi
      .fn()
      .mockResolvedValueOnce(file)
      .mockResolvedValueOnce(null);
    const registry = createBrowserFileTokenRegistry();
    const gateway = createBrowserPlatformFileGateway({
      pickerWindow: {} as never,
      browserDocument: document,
      registry,
      chooseWithInput,
    });

    const selected = await gateway.chooseOpenFile({ filters: FILTERS });
    expect(await registry.read(selected as BrowserFileToken)).toBe(file);
    await expect(gateway.chooseOpenFile({ filters: FILTERS })).resolves.toBeNull();
    expect(chooseWithInput).toHaveBeenCalledTimes(2);
  });

  it("save picker非対応時はBlobを指定名でダウンロードする", async () => {
    const download = vi.fn();
    const registry = createBrowserFileTokenRegistry(download);
    const gateway = createBrowserPlatformFileGateway({
      pickerWindow: {} as never,
      browserDocument: document,
      registry,
    });

    expect(gateway.saveMode).toBe("download");
    const selected = await gateway.chooseSaveFile({
      filters: [FILTERS[0]],
      suggestedName: "作品.ori3",
    });
    expect(registry.destinationOf(selected as BrowserFileToken)).toBe(
      "download",
    );

    await expect(
      registry.write(
        selected as BrowserFileToken,
        "{\"schema_version\":1}",
        "application/json",
      ),
    ).resolves.toEqual({ destination: "download", name: "作品.ori3" });
    expect(download).toHaveBeenCalledTimes(1);
    const [blob, name] = download.mock.calls[0] as [Blob, string];
    expect(name).toBe("作品.ori3");
    expect(blob.type).toBe("application/json");
    await expect(blob.text()).resolves.toBe('{"schema_version":1}');
    gateway.release(selected as string);
    expect(() =>
      registry.destinationOf(selected as BrowserFileToken),
    ).toThrow(MISSING_TOKEN_MESSAGE);
  });

  it("save pickerの権限拒否は日本語理由を返し、downloadへ黙って切り替えない", async () => {
    const showSaveFilePicker = vi
      .fn()
      .mockRejectedValue(new DOMException("denied", "NotAllowedError"));
    const download = vi.fn();
    const gateway = createBrowserPlatformFileGateway({
      pickerWindow: { showSaveFilePicker } as never,
      browserDocument: document,
      registry: createBrowserFileTokenRegistry(download),
    });

    expect(gateway.saveMode).toBe("choose-destination");
    await expect(
      gateway.chooseSaveFile({
        filters: [FILTERS[0]],
        suggestedName: "作品.ori3",
      }),
    ).rejects.toThrow(
      "ファイルを保存する権限が許可されませんでした。作品は変更されていません。",
    );
    expect(download).not.toHaveBeenCalled();
  });

  it("複数SVGはdirectory APIがあれば保存先を1回だけ選ぶ", async () => {
    const directory = {
      kind: "directory",
      name: "折り図",
      getFileHandle: vi.fn(),
    } as unknown as FileSystemDirectoryHandle;
    const showDirectoryPicker = vi.fn().mockResolvedValue(directory);
    const showSaveFilePicker = vi.fn();
    const registry = createBrowserFileTokenRegistry();
    const gateway = createBrowserPlatformFileGateway({
      pickerWindow: {
        showDirectoryPicker,
        showSaveFilePicker,
      } as never,
      browserDocument: document,
      registry,
    });

    const token = await gateway.chooseSaveFile({
      filters: [{ name: "折り図(SVG)", extensions: ["svg"] }],
      suggestedName: "作品.svg",
      multipleFiles: true,
    });

    expect(showDirectoryPicker).toHaveBeenCalledWith({ mode: "readwrite" });
    expect(showSaveFilePicker).not.toHaveBeenCalled();
    expect(registry.destinationOf(token as BrowserFileToken)).toBe("directory");
    expect(registry.nameOf(token as BrowserFileToken)).toBe("作品.svg");
  });

  it("directory API非対応時だけ複数SVGを同名stemのZIPにする", async () => {
    const showSaveFilePicker = vi.fn();
    const registry = createBrowserFileTokenRegistry();
    const gateway = createBrowserPlatformFileGateway({
      pickerWindow: { showSaveFilePicker } as never,
      browserDocument: document,
      registry,
    });

    const token = await gateway.chooseSaveFile({
      filters: [{ name: "折り図(SVG)", extensions: ["svg"] }],
      suggestedName: "作品.svg",
      multipleFiles: true,
    });

    expect(showSaveFilePicker).not.toHaveBeenCalled();
    expect(registry.destinationOf(token as BrowserFileToken)).toBe("download");
    expect(registry.nameOf(token as BrowserFileToken)).toBe("作品-折り図.zip");
  });

  it("directory pickerの取消しや権限拒否をZIP fallbackへ変えない", async () => {
    const cancelled = createBrowserPlatformFileGateway({
      pickerWindow: {
        showDirectoryPicker: vi
          .fn()
          .mockRejectedValue(new DOMException("cancel", "AbortError")),
      } as never,
      browserDocument: document,
      registry: createBrowserFileTokenRegistry(),
    });
    await expect(
      cancelled.chooseSaveFile({
        filters: [{ name: "折り図(SVG)", extensions: ["svg"] }],
        suggestedName: "作品.svg",
        multipleFiles: true,
      }),
    ).resolves.toBeNull();

    const denied = createBrowserPlatformFileGateway({
      pickerWindow: {
        showDirectoryPicker: vi
          .fn()
          .mockRejectedValue(new DOMException("denied", "NotAllowedError")),
      } as never,
      browserDocument: document,
      registry: createBrowserFileTokenRegistry(),
    });
    await expect(
      denied.chooseSaveFile({
        filters: [{ name: "折り図(SVG)", extensions: ["svg"] }],
        suggestedName: "作品.svg",
        multipleFiles: true,
      }),
    ).rejects.toThrow(
      "ファイルを保存する権限が許可されませんでした。作品は変更されていません。",
    );
  });

  it("選んだFileSystemFileHandleだけへ書き込み、closeまで待つ", async () => {
    const writable = {
      write: vi.fn().mockResolvedValue(undefined),
      close: vi.fn().mockResolvedValue(undefined),
    };
    const handle = {
      kind: "file",
      name: "水風船.svg",
      createWritable: vi.fn().mockResolvedValue(writable),
      getFile: vi.fn(),
    } as unknown as FileSystemFileHandle;
    const registry = createBrowserFileTokenRegistry();
    const gateway = createBrowserPlatformFileGateway({
      pickerWindow: {
        showSaveFilePicker: vi.fn().mockResolvedValue(handle),
      } as never,
      browserDocument: document,
      registry,
    });
    const token = await gateway.chooseSaveFile({
      filters: [{ name: "展開図(SVG)", extensions: ["svg"] }],
      suggestedName: "水風船.svg",
    });

    await expect(
      registry.write(token as BrowserFileToken, "<svg/>", "image/svg+xml"),
    ).resolves.toEqual({
      destination: "file-system",
      name: "水風船.svg",
    });
    expect(writable.write).toHaveBeenCalledTimes(1);
    expect((writable.write.mock.calls[0][0] as Blob).type).toBe("image/svg+xml");
    expect(writable.close).toHaveBeenCalledTimes(1);
    gateway.release(token as string);
    expect(() =>
      registry.destinationOf(token as BrowserFileToken),
    ).toThrow(MISSING_TOKEN_MESSAGE);
  });

  it("処理失敗のfinallyでもFile・handle・downloadの3tokenをすべて無効にする", async () => {
    const registry = createBrowserFileTokenRegistry();
    const gateway = createBrowserPlatformFileGateway({
      pickerWindow: {} as never,
      browserDocument: document,
      registry,
    });
    const fileToken = registry.registerRead(
      new File(["{}"], "鳥の基本形.ori3"),
    );
    const handleToken = registry.registerFileSystemDestination({
      kind: "file",
      name: "鳥の基本形.pdf",
    } as FileSystemFileHandle);
    const downloadToken = registry.registerDownload("鳥の基本形.svg");

    const failThenRelease = async (token: BrowserFileToken): Promise<void> => {
      try {
        throw new Error("後続処理の失敗");
      } finally {
        gateway.release(token);
      }
    };

    for (const token of [fileToken, handleToken, downloadToken]) {
      await expect(failThenRelease(token)).rejects.toThrow("後続処理の失敗");
    }
    await expect(registry.read(fileToken)).rejects.toThrow(
      MISSING_TOKEN_MESSAGE,
    );
    expect(() => registry.destinationOf(handleToken)).toThrow(
      MISSING_TOKEN_MESSAGE,
    );
    expect(() => registry.destinationOf(downloadToken)).toThrow(
      MISSING_TOKEN_MESSAGE,
    );
  });

  it("retainした現在作品tokenはUIのrelease後も残り、pin解除後に消える", async () => {
    const registry = createBrowserFileTokenRegistry();
    const token = registry.registerRead(new File(["{}"], "カエル.ori3"));

    registry.retain(token);
    registry.release(token);
    await expect(registry.read(token)).resolves.toBeInstanceOf(File);

    registry.release(token);
    await expect(registry.read(token)).rejects.toThrow(MISSING_TOKEN_MESSAGE);
  });
});
