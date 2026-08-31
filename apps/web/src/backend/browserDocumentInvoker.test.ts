// @vitest-environment jsdom

import { describe, expect, it, vi } from "vitest";
import { unzipSync } from "../../../desktop/node_modules/fflate/esm/browser.js";
import type {
  BackendInvokeArgs,
} from "../../../desktop/src/ipc/runtime";
import type { BrowserFileToken } from "../../../desktop/src/platform/fileGateway";
import {
  createBrowserCurrentDocumentCoordinator,
  createBrowserDocumentInvoker,
  createDocumentLifecycleCoreInvoker,
  type BrowserDocumentDeliveryNotice,
} from "./browserDocumentInvoker";
import type { Ori3CoreWorkerClient } from "./coreWorkerClient";
import type { CoreCommandName } from "./coreWorkerProtocol";
import { createBrowserPlatformFileGateway } from "../platform/browserFileGateway";
import { createBrowserFileTokenRegistry } from "../platform/browserFileTokenRegistry";

interface CoreCall {
  command: CoreCommandName;
  args?: BackendInvokeArgs;
}

class FakeCore implements Ori3CoreWorkerClient {
  readonly calls: CoreCall[] = [];

  constructor(
    private readonly handle: (
      command: CoreCommandName,
      args?: BackendInvokeArgs,
    ) => unknown | Promise<unknown>,
  ) {}

  async invoke<T>(command: CoreCommandName, args?: BackendInvokeArgs): Promise<T> {
    this.calls.push({ command, ...(args === undefined ? {} : { args }) });
    return (await this.handle(command, args)) as T;
  }

  ready(): Promise<void> {
    return Promise.resolve();
  }

  dispose(): void {}
}

function memoryFile(name: string, bytes: Uint8Array): File {
  return {
    name,
    arrayBuffer: () => Promise.resolve(bytes.slice().buffer),
  } as unknown as File;
}

function base64(text: string): string {
  return btoa(text);
}

function exportFile(
  suffix: string,
  text: string,
  pageNumber: number | null = null,
  firstCell: number | null = null,
  lastCell: number | null = null,
) {
  return {
    suffix,
    content_type: "image/svg+xml",
    content_base64: base64(text),
    page_number: pageNumber,
    first_cell: firstCell,
    last_cell: lastCell,
  };
}

describe("browser document mixed invoker", () => {
  it("openはUTF-8本文をstageして公開命令を呼び、成功時だけ現在tokenをpinする", async () => {
    const registry = createBrowserFileTokenRegistry();
    const source = '{"作品":"折り鶴"}';
    const token = registry.registerRead(
      memoryFile("折り鶴.ori3", new TextEncoder().encode(source)),
    );
    const current = createBrowserCurrentDocumentCoordinator(registry);
    const core = new FakeCore((command) => {
      if (command === "__web_document_open_source") return null;
      if (command === "document_open") return { opened: true };
      if (command === "document_new") return { created: true };
      throw new Error(`unexpected ${command}`);
    });
    const invoker = createBrowserDocumentInvoker(core, {
      registry,
      currentDocument: current,
    });

    await expect(
      invoker.invoke("document_open", { path: token }),
    ).resolves.toEqual({ opened: true });
    expect(core.calls.slice(0, 2)).toEqual([
      {
        command: "__web_document_open_source",
        args: { path: token, source },
      },
      { command: "document_open", args: { path: token } },
    ]);

    registry.release(token);
    await expect(registry.read(token)).resolves.toMatchObject({
      name: "折り鶴.ori3",
    });

    const lifecycleCore = createDocumentLifecycleCoreInvoker(core, current);
    await lifecycleCore.invoke("document_new", { paper: { kind: "Square" } });
    await expect(registry.read(token)).rejects.toThrow(
      "選んだファイルをこのタブで確認できませんでした。",
    );
  });

  it("openは無効UTF-8をdesktopと同じprefixで拒否しcoreを呼ばない", async () => {
    const registry = createBrowserFileTokenRegistry();
    const token = registry.registerRead(
      memoryFile("やっこさん.ori3", new Uint8Array([0xff, 0xfe])),
    );
    const core = new FakeCore(() => null);
    const invoker = createBrowserDocumentInvoker(core, { registry });

    await expect(
      invoker.invoke("document_open", { path: token }),
    ).rejects.toThrow(
      "ファイルを開けませんでした: UTF-8でない文字が含まれています。作品は変更されていません。",
    );
    expect(core.calls).toEqual([]);
  });

  it("saveはwriteとcloseの完了後だけ公開命令でcommitし、保存先をpinする", async () => {
    const events: string[] = [];
    const writable = {
      write: vi.fn(async () => events.push("write")),
      close: vi.fn(async () => events.push("close")),
    };
    const handle = {
      kind: "file",
      name: "水風船.ori3",
      getFile: vi.fn(),
      createWritable: vi.fn(async () => writable),
    } as unknown as FileSystemFileHandle;
    const registry = createBrowserFileTokenRegistry();
    const token = registry.registerFileSystemDestination(handle);
    const notices: BrowserDocumentDeliveryNotice[] = [];
    const core = new FakeCore((command) => {
      if (command === "__web_document_save_prepare") {
        return { path: token, content: '{"schema_version":1}' };
      }
      if (command === "document_save") {
        expect(events).toEqual(["write", "close"]);
        events.push("commit");
        return null;
      }
      if (command === "__web_document_save_abort") return null;
      throw new Error(`unexpected ${command}`);
    });
    const current = createBrowserCurrentDocumentCoordinator(registry);
    const invoker = createBrowserDocumentInvoker(core, {
      registry,
      currentDocument: current,
      notifyDelivery: (notice) => notices.push(notice),
    });

    await expect(
      invoker.invoke("document_save", { path: token }),
    ).resolves.toBeNull();
    expect(events).toEqual(["write", "close", "commit"]);
    expect(notices).toEqual([
      {
        command: "document_save",
        destination: "file-system",
        names: ["水風船.ori3"],
      },
    ]);

    registry.release(token);
    expect(registry.destinationOf(token)).toBe("file-system");
    current.clear();
    expect(() => registry.destinationOf(token)).toThrow(
      "選んだファイルをこのタブで確認できませんでした。",
    );
  });

  it("read-onlyで開いた作品のsave(null)は同名downloadを開始してcommitする", async () => {
    const downloads: Array<{ blob: Blob; name: string }> = [];
    const registry = createBrowserFileTokenRegistry((blob, name) => {
      downloads.push({ blob, name });
    });
    const token = registry.registerRead(memoryFile("鳥の基本形.ori3", new Uint8Array()));
    const current = createBrowserCurrentDocumentCoordinator(registry);
    current.adopt(token);
    registry.release(token);
    const notices: BrowserDocumentDeliveryNotice[] = [];
    const core = new FakeCore((command, args) => {
      if (command === "__web_document_save_prepare") {
        expect(args).toEqual({ path: null });
        return { path: token, content: '{"schema_version":1}' };
      }
      if (command === "document_save") {
        expect(args).toEqual({ path: null });
        return null;
      }
      if (command === "__web_document_save_abort") return null;
      throw new Error(`unexpected ${command}`);
    });
    const invoker = createBrowserDocumentInvoker(core, {
      registry,
      currentDocument: current,
      notifyDelivery: (notice) => notices.push(notice),
    });

    await expect(
      invoker.invoke("document_save", { path: null }),
    ).resolves.toBeNull();
    expect(downloads).toHaveLength(1);
    expect(downloads[0].name).toBe("鳥の基本形.ori3");
    await expect(downloads[0].blob.text()).resolves.toBe(
      '{"schema_version":1}',
    );
    expect(notices[0]).toEqual({
      command: "document_save",
      destination: "download",
      names: ["鳥の基本形.ori3"],
    });
    expect(current.current()).toBe(token);
  });

  it("pickerで開いたread/write handleはUI release後のsave(null)で同じhandleへ上書きする", async () => {
    const file = memoryFile(
      "やっこさん.ori3",
      new TextEncoder().encode('{"schema_version":1}'),
    );
    const writable = {
      write: vi.fn().mockResolvedValue(undefined),
      close: vi.fn().mockResolvedValue(undefined),
    };
    const handle = {
      kind: "file",
      name: file.name,
      getFile: vi.fn().mockResolvedValue(file),
      createWritable: vi.fn().mockResolvedValue(writable),
    } as unknown as FileSystemFileHandle;
    const registry = createBrowserFileTokenRegistry();
    const gateway = createBrowserPlatformFileGateway({
      pickerWindow: {
        showOpenFilePicker: vi.fn().mockResolvedValue([handle]),
      } as never,
      registry,
    });
    const selected = await gateway.chooseOpenFile({
      filters: [{ name: "ORIGAMI3作品", extensions: ["ori3"] }],
    });
    expect(selected).not.toBeNull();
    const token = selected as BrowserFileToken;
    const current = createBrowserCurrentDocumentCoordinator(registry);
    const core = new FakeCore((command, args) => {
      if (command === "__web_document_open_source") return null;
      if (command === "document_open") return { opened: true };
      if (command === "__web_document_save_prepare") {
        expect(args).toEqual({ path: null });
        return { path: token, content: '{"schema_version":1}' };
      }
      if (command === "document_save") return null;
      if (command === "__web_document_save_abort") return null;
      throw new Error(`unexpected ${command}`);
    });
    const invoker = createBrowserDocumentInvoker(core, {
      registry,
      currentDocument: current,
    });

    await invoker.invoke("document_open", { path: token });
    gateway.release(token);
    await invoker.invoke("document_save", { path: null });

    expect(handle.getFile).toHaveBeenCalledTimes(1);
    expect(handle.createWritable).toHaveBeenCalledTimes(1);
    expect(writable.write).toHaveBeenCalledTimes(1);
    expect(writable.close).toHaveBeenCalledTimes(1);
  });

  it("saveのwrite失敗は公開commitを呼ばず、private準備をabortする", async () => {
    const writable = {
      write: vi.fn().mockRejectedValue(new Error("disk")),
      close: vi.fn(),
    };
    const handle = {
      kind: "file",
      name: "カエル.ori3",
      createWritable: vi.fn().mockResolvedValue(writable),
    } as unknown as FileSystemFileHandle;
    const registry = createBrowserFileTokenRegistry();
    const token = registry.registerFileSystemDestination(handle);
    const core = new FakeCore((command) => {
      if (command === "__web_document_save_prepare") {
        return { path: token, content: "{}" };
      }
      if (command === "__web_document_save_abort") return null;
      throw new Error(`unexpected ${command}`);
    });
    const invoker = createBrowserDocumentInvoker(core, { registry });

    await expect(
      invoker.invoke("document_save", { path: token }),
    ).rejects.toThrow(
      "選んだファイルへ書き込めませんでした。作品は変更されていません。",
    );
    expect(core.calls.map(({ command }) => command)).toEqual([
      "__web_document_save_prepare",
      "__web_document_save_abort",
    ]);
    expect(writable.close).not.toHaveBeenCalled();
  });

  it("単一exportは厳格base64をRust bytesへ戻しfold_issuesだけを返す", async () => {
    const downloads: Array<{ blob: Blob; name: string }> = [];
    const registry = createBrowserFileTokenRegistry((blob, name) => {
      downloads.push({ blob, name });
    });
    const token = registry.registerDownload("カエル.svg");
    const issue = { kind: "UnsupportedAssignment", edge: 4 };
    const notices: BrowserDocumentDeliveryNotice[] = [];
    const core = new FakeCore((command) => {
      if (command === "__web_document_export_prepare") {
        return {
          files: [exportFile("", "<svg>frog</svg>")],
          fold_issues: [issue],
        };
      }
      throw new Error(`unexpected ${command}`);
    });
    const invoker = createBrowserDocumentInvoker(core, {
      registry,
      notifyDelivery: (notice) => notices.push(notice),
    });

    await expect(
      invoker.invoke("document_export", {
        kind: "CpSvg",
        path: token,
        options: { include_aux: true, png_long_side: 1200 },
      }),
    ).resolves.toEqual([issue]);
    expect(downloads).toHaveLength(1);
    expect(downloads[0].name).toBe("カエル.svg");
    await expect(downloads[0].blob.text()).resolves.toBe("<svg>frog</svg>");
    expect(notices[0]).toEqual({
      command: "document_export",
      destination: "download",
      names: ["カエル.svg"],
    });
  });

  it("非正規base64は1byteも書かずに明示失敗する", async () => {
    const download = vi.fn();
    const registry = createBrowserFileTokenRegistry(download);
    const token = registry.registerDownload("折り鶴.svg");
    const core = new FakeCore(() => ({
      files: [
        {
          ...exportFile("", "ok"),
          content_base64: "a===",
        },
      ],
      fold_issues: [],
    }));
    const invoker = createBrowserDocumentInvoker(core, { registry });

    await expect(
      invoker.invoke("document_export", {
        kind: "CpSvg",
        path: token,
        options: { include_aux: false, png_long_side: 1200 },
      }),
    ).rejects.toThrow(
      "書き出すファイルの内容を確認できませんでした。作品は変更されていません。",
    );
    expect(download).not.toHaveBeenCalled();
  });

  it("directory保存はclose成功分だけを保存済みにし、失敗後へ進まない", async () => {
    const requestedNames: string[] = [];
    const makeWritable = (name: string) => ({
      write: vi.fn().mockResolvedValue(undefined),
      close:
        name === "作品-02.svg"
          ? vi.fn().mockRejectedValue(new Error("close failed"))
          : vi.fn().mockResolvedValue(undefined),
    });
    const directory = {
      kind: "directory",
      name: "折り図",
      getFileHandle: vi.fn(async (name: string) => {
        requestedNames.push(name);
        return {
          kind: "file",
          name,
          createWritable: vi.fn().mockResolvedValue(makeWritable(name)),
        } as unknown as FileSystemFileHandle;
      }),
    } as unknown as FileSystemDirectoryHandle;
    const registry = createBrowserFileTokenRegistry();
    const token = registry.registerDirectoryDestination(directory, "作品.svg");
    const core = new FakeCore(() => ({
      files: [
        exportFile("-01", "cover", 1),
        exportFile("-02", "cells 1-6", 2, 1, 6),
        exportFile("-03", "cells 7-9", 3, 7, 9),
      ],
      fold_issues: [],
    }));
    const invoker = createBrowserDocumentInvoker(core, { registry });

    await expect(
      invoker.invoke("document_export", {
        kind: "DiagramSvg",
        path: token,
        options: { include_aux: false, png_long_side: 1200 },
      }),
    ).rejects.toThrow(
      "保存済み（ここまで）: 作品-01.svg（表紙）。失敗: 作品-02.svg（1〜6番）。未保存: 作品-03.svg（7〜9番）。",
    );
    expect(requestedNames).toEqual(["作品-01.svg", "作品-02.svg"]);
  });

  it("directory API非対応用ZIPはflat名・Rust bytesを保ち、同入力でbit一致する", async () => {
    const downloads: Blob[] = [];
    const registry = createBrowserFileTokenRegistry((blob) => {
      downloads.push(blob);
    });
    const core = new FakeCore(() => ({
      files: [
        exportFile("-01", "cover", 1),
        exportFile("-02", "cell", 2, 1, 1),
      ],
      fold_issues: [],
    }));
    const invoker = createBrowserDocumentInvoker(core, { registry });

    for (let run = 0; run < 2; run += 1) {
      const token = registry.registerDownload("作品.zip");
      await invoker.invoke("document_export", {
        kind: "DiagramSvg",
        path: token,
        options: { include_aux: false, png_long_side: 1200 },
      });
      registry.release(token);
    }

    const first = new Uint8Array(await downloads[0].arrayBuffer());
    const second = new Uint8Array(await downloads[1].arrayBuffer());
    expect(second).toEqual(first);
    const files = unzipSync(first);
    expect(Object.keys(files)).toEqual(["作品-01.svg", "作品-02.svg"]);
    expect(new TextDecoder().decode(files["作品-01.svg"])).toBe("cover");
    expect(new TextDecoder().decode(files["作品-02.svg"])).toBe("cell");
  });

  it("外部exportの余分なfieldはprepareもwriteも始める前に拒否する", async () => {
    const download = vi.fn();
    const registry = createBrowserFileTokenRegistry(download);
    const token = registry.registerDownload("作品.svg");
    const core = new FakeCore(() => {
      throw new Error("core must not run");
    });
    const invoker = createBrowserDocumentInvoker(core, { registry });

    await expect(
      invoker.invoke("document_export", {
        kind: "CpSvg",
        path: token,
        options: { include_aux: false, png_long_side: 1200 },
        extra: true,
      }),
    ).rejects.toContain("unknown field `extra`");
    expect(core.calls).toEqual([]);
    expect(download).not.toHaveBeenCalled();
  });
});
