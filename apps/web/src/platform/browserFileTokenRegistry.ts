import {
  PlatformFileGatewayError,
  type BrowserFileToken,
} from "../../../desktop/src/platform/fileGateway";

export type BrowserFileDestination =
  | "read"
  | "file-system"
  | "directory"
  | "download";

interface ReadEntry {
  kind: "read";
  name: string;
  file: File;
}

interface FileSystemEntry {
  kind: "file-system";
  name: string;
  handle: FileSystemFileHandle;
}

interface DownloadEntry {
  kind: "download";
  name: string;
}

interface DirectoryEntry {
  kind: "directory";
  /** 利用者が選んだ保存先で使う、拡張子つきの基準名。 */
  name: string;
  handle: FileSystemDirectoryHandle;
}

type BrowserFileEntry =
  | ReadEntry
  | FileSystemEntry
  | DirectoryEntry
  | DownloadEntry;

export interface BrowserFileWriteResult {
  destination: "file-system" | "download";
  name: string;
}

export interface BrowserDirectoryWriteResult {
  destination: "directory";
  name: string;
}

export type BrowserBlobDownloader = (
  blob: Blob,
  name: string,
) => void | Promise<void>;

function safeFileName(name: string): string {
  const cleaned = name.replace(/[\\/]/g, "_").trim();
  return cleaned.length > 0 ? cleaned : "ORIGAMI3.ori3";
}

let fallbackTokenId = 1;

function tokenId(): string {
  if (typeof globalThis.crypto?.randomUUID === "function") {
    return globalThis.crypto.randomUUID();
  }
  const id = fallbackTokenId;
  fallbackTokenId += 1;
  return `session-${id}`;
}

function makeToken(
  kind: BrowserFileEntry["kind"],
  name: string,
): BrowserFileToken {
  return `browser-file://${kind}/${tokenId()}/${safeFileName(name)}` as BrowserFileToken;
}

export function isBrowserFileToken(path: string): path is BrowserFileToken {
  return path.startsWith("browser-file://");
}

export async function downloadBrowserBlob(blob: Blob, name: string): Promise<void> {
  if (
    typeof document === "undefined" ||
    typeof URL.createObjectURL !== "function"
  ) {
    throw new PlatformFileGatewayError(
      "download",
      "この環境ではダウンロードを開始できません。作品は変更されていません。",
    );
  }

  const objectUrl = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = objectUrl;
  anchor.download = name;
  anchor.hidden = true;
  document.body.append(anchor);
  try {
    anchor.click();
  } finally {
    anchor.remove();
    globalThis.setTimeout(() => URL.revokeObjectURL(objectUrl), 0);
  }
}

export interface BrowserFileTokenRegistry {
  registerRead(file: File): BrowserFileToken;
  registerFileSystemDestination(handle: FileSystemFileHandle): BrowserFileToken;
  registerDirectoryDestination(
    handle: FileSystemDirectoryHandle,
    suggestedName: string,
  ): BrowserFileToken;
  registerDownload(name: string): BrowserFileToken;
  destinationOf(token: BrowserFileToken): BrowserFileDestination;
  nameOf(token: BrowserFileToken): string;
  read(token: BrowserFileToken): Promise<File>;
  write(
    token: BrowserFileToken,
    content: Blob | BlobPart,
    contentType?: string,
  ): Promise<BrowserFileWriteResult>;
  writeDirectoryFile(
    token: BrowserFileToken,
    name: string,
    content: Blob | BlobPart,
    contentType?: string,
  ): Promise<BrowserDirectoryWriteResult>;
  retain(token: BrowserFileToken): void;
  release(token: BrowserFileToken): void;
}

export function createBrowserFileTokenRegistry(
  download: BrowserBlobDownloader = downloadBrowserBlob,
): BrowserFileTokenRegistry {
  const entries = new Map<BrowserFileToken, BrowserFileEntry>();
  const references = new Map<BrowserFileToken, number>();

  const register = (
    token: BrowserFileToken,
    entry: BrowserFileEntry,
  ): BrowserFileToken => {
    entries.set(token, entry);
    references.set(token, 1);
    return token;
  };

  const requireEntry = (token: BrowserFileToken): BrowserFileEntry => {
    const entry = entries.get(token);
    if (entry) return entry;
    throw new PlatformFileGatewayError(
      "open",
      "選んだファイルをこのタブで確認できませんでした。作品は変更されていません。もう一度ファイルを選んでください。",
    );
  };

  return {
    registerRead(file): BrowserFileToken {
      const token = makeToken("read", file.name);
      return register(token, { kind: "read", name: file.name, file });
    },

    registerFileSystemDestination(handle): BrowserFileToken {
      const token = makeToken("file-system", handle.name);
      return register(token, {
        kind: "file-system",
        name: handle.name,
        handle,
      });
    },

    registerDirectoryDestination(handle, suggestedName): BrowserFileToken {
      const safeName = safeFileName(suggestedName);
      const token = makeToken("directory", safeName);
      return register(token, {
        kind: "directory",
        name: safeName,
        handle,
      });
    },

    registerDownload(name): BrowserFileToken {
      const safeName = safeFileName(name);
      const token = makeToken("download", safeName);
      return register(token, { kind: "download", name: safeName });
    },

    destinationOf(token): BrowserFileDestination {
      return requireEntry(token).kind;
    },

    nameOf(token): string {
      return requireEntry(token).name;
    },

    async read(token): Promise<File> {
      const entry = requireEntry(token);
      if (entry.kind === "read") return entry.file;
      if (entry.kind === "file-system") return entry.handle.getFile();
      throw new PlatformFileGatewayError(
        "open",
        "ダウンロード用に選んだ名前から作品を開くことはできません。作品は変更されていません。",
      );
    },

    async write(token, content, contentType = "application/octet-stream") {
      const entry = requireEntry(token);
      const blob =
        content instanceof Blob ? content : new Blob([content], { type: contentType });
      if (entry.kind === "file-system") {
        try {
          const writable = await entry.handle.createWritable();
          await writable.write(blob);
          await writable.close();
        } catch (reason) {
          const denied =
            typeof DOMException !== "undefined" &&
            reason instanceof DOMException &&
            (reason.name === "NotAllowedError" || reason.name === "SecurityError");
          throw new PlatformFileGatewayError(
            "save",
            denied
              ? "ファイルを保存する権限が許可されませんでした。作品は変更されていません。"
              : "選んだファイルへ書き込めませんでした。作品は変更されていません。",
          );
        }
        return { destination: "file-system", name: entry.name };
      }
      if (entry.kind === "download") {
        await download(blob, entry.name);
        return { destination: "download", name: entry.name };
      }
      if (entry.kind === "directory") {
        throw new PlatformFileGatewayError(
          "save",
          "選んだ保存先には、複数のファイルとして保存してください。作品は変更されていません。",
        );
      }
      throw new PlatformFileGatewayError(
        "save",
        "開くために選んだファイルへは保存できません。作品は変更されていません。",
      );
    },

    async writeDirectoryFile(
      token,
      name,
      content,
      contentType = "application/octet-stream",
    ) {
      const entry = requireEntry(token);
      if (entry.kind !== "directory") {
        throw new PlatformFileGatewayError(
          "save",
          "複数ファイルの保存先を確認できませんでした。作品は変更されていません。",
        );
      }
      const safeName = safeFileName(name);
      if (safeName !== name || name === "." || name === "..") {
        throw new PlatformFileGatewayError(
          "save",
          "保存するファイル名を確認できませんでした。作品は変更されていません。",
        );
      }
      const blob =
        content instanceof Blob ? content : new Blob([content], { type: contentType });
      try {
        const handle = await entry.handle.getFileHandle(name, { create: true });
        const writable = await handle.createWritable();
        await writable.write(blob);
        await writable.close();
      } catch (reason) {
        const denied =
          typeof DOMException !== "undefined" &&
          reason instanceof DOMException &&
          (reason.name === "NotAllowedError" || reason.name === "SecurityError");
        throw new PlatformFileGatewayError(
          "save",
          denied
            ? "選んだ保存先へ書き込む権限が許可されませんでした。作品は変更されていません。"
            : "選んだ保存先へファイルを書き込めませんでした。作品は変更されていません。",
        );
      }
      return { destination: "directory", name };
    },

    retain(token): void {
      requireEntry(token);
      references.set(token, (references.get(token) ?? 0) + 1);
    },

    release(token): void {
      const count = references.get(token);
      if (count === undefined) return;
      if (count > 1) {
        references.set(token, count - 1);
        return;
      }
      references.delete(token);
      entries.delete(token);
    },
  };
}

/** mixed routeとfile gatewayが共有する、タブ内だけの能力registry。 */
export const BROWSER_FILE_TOKEN_REGISTRY = createBrowserFileTokenRegistry();
