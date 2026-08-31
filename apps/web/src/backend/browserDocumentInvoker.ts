import type {
  BackendCommandName,
  BackendInvokeArgs,
} from "../../../desktop/src/ipc/runtime";
import {
  PlatformFileGatewayError,
  type BrowserFileToken,
} from "../../../desktop/src/platform/fileGateway";
import { createBrowserZip } from "../../../desktop/src/platform/browserZip";
import type { Ori3CoreWorkerClient } from "./coreWorkerClient";
import type { InternalCoreCommandName } from "./coreWorkerProtocol";
import {
  BROWSER_FILE_TOKEN_REGISTRY,
  isBrowserFileToken,
  type BrowserFileTokenRegistry,
} from "../platform/browserFileTokenRegistry";

interface DocumentSavePreparation {
  path: string;
  content: string;
}

interface DocumentExportFileWire {
  suffix: string;
  content_type: string;
  content_base64: string;
  page_number: number | null;
  first_cell: number | null;
  last_cell: number | null;
}

interface DocumentExportPreparationWire {
  files: DocumentExportFileWire[];
  fold_issues: unknown[];
}

interface PreparedExportFile extends Omit<DocumentExportFileWire, "content_base64"> {
  bytes: Uint8Array;
}

export interface BrowserDocumentInvoker {
  invoke<T>(command: BackendCommandName, args?: BackendInvokeArgs): Promise<T>;
}

export interface BrowserCurrentDocumentCoordinator {
  adopt(token: BrowserFileToken): void;
  clear(): void;
  current(): BrowserFileToken | null;
}

export type BrowserDocumentDeliveryNotice = {
  command: "document_save" | "document_export";
  destination: "file-system" | "directory" | "download";
  names: readonly string[];
};

export type BrowserDocumentDeliveryListener = (
  notice: BrowserDocumentDeliveryNotice,
) => void;

const deliveryListeners = new Set<BrowserDocumentDeliveryListener>();

/** 外部IPC返答を変えず、UIへ保存方法を知らせるside channel。 */
export function subscribeBrowserDocumentDelivery(
  listener: BrowserDocumentDeliveryListener,
): () => void {
  deliveryListeners.add(listener);
  return () => deliveryListeners.delete(listener);
}

function publishDelivery(notice: BrowserDocumentDeliveryNotice): void {
  for (const listener of deliveryListeners) listener(notice);
}

function objectValue(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function commandArgs(args: BackendInvokeArgs | undefined): Record<string, unknown> {
  const value = objectValue(args);
  if (value) return value;
  throw "コマンド要求の args フィールドはobjectにしてください。";
}

function strictFields(
  command: BackendCommandName,
  args: Record<string, unknown>,
  expected: readonly string[],
): void {
  const unknown = Object.keys(args).find((name) => !expected.includes(name));
  if (unknown) {
    const expectation =
      expected.length === 1
        ? `\`${expected[0]}\``
        : `one of ${expected.map((name) => `\`${name}\``).join(", ")}`;
    throw `コマンド「${command}」の引数を読み取れません: unknown field \`${unknown}\`, expected ${expectation}`;
  }
  const missing = expected.find((name) => !(name in args));
  if (missing) {
    throw `コマンド「${command}」の引数を読み取れません: missing field \`${missing}\``;
  }
}

function requiredPath(
  command: "document_open" | "document_export",
  args: Record<string, unknown>,
): BrowserFileToken {
  if (typeof args.path !== "string") {
    throw `コマンド「${command}」の引数を読み取れません: pathには文字列を指定してください`;
  }
  if (!isBrowserFileToken(args.path)) {
    throw new PlatformFileGatewayError(
      command === "document_open" ? "open" : "save",
      "選んだファイルをこのタブで確認できませんでした。作品は変更されていません。もう一度ファイルを選んでください。",
    );
  }
  return args.path;
}

function normalizePrivateError(
  reason: unknown,
  internal: InternalCoreCommandName,
  external: BackendCommandName,
): unknown {
  const replace = (message: string): string =>
    message.split(`「${internal}」`).join(`「${external}」`);
  if (typeof reason === "string") return replace(reason);
  if (reason instanceof Error) return new Error(replace(reason.message));
  return reason;
}

async function invokePrivate<T>(
  core: Ori3CoreWorkerClient,
  internal: InternalCoreCommandName,
  external: BackendCommandName,
  args: BackendInvokeArgs,
): Promise<T> {
  try {
    return await core.invoke<T>(internal, args);
  } catch (reason) {
    throw normalizePrivateError(reason, internal, external);
  }
}

function decodeUtf8(file: File): Promise<string> {
  return file
    .arrayBuffer()
    .then((bytes) => {
      try {
        return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      } catch {
        throw new PlatformFileGatewayError(
          "open",
          "ファイルを開けませんでした: UTF-8でない文字が含まれています。作品は変更されていません。",
        );
      }
    })
    .catch((reason: unknown) => {
      if (reason instanceof PlatformFileGatewayError) throw reason;
      throw new PlatformFileGatewayError(
        "open",
        "ファイルを開けませんでした: 選んだファイルの内容を読み込めませんでした。作品は変更されていません。",
      );
    });
}

function validOptionalIndex(value: unknown): value is number | null {
  return (
    value === null ||
    (typeof value === "number" && Number.isSafeInteger(value) && value >= 1)
  );
}

function invalidExportPreparation(): PlatformFileGatewayError {
  return new PlatformFileGatewayError(
    "save",
    "書き出すファイルの内容を確認できませんでした。作品は変更されていません。",
  );
}

function decodeBase64(value: unknown): Uint8Array {
  if (
    typeof value !== "string" ||
    value.length % 4 !== 0 ||
    !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(
      value,
    )
  ) {
    throw invalidExportPreparation();
  }
  let binary: string;
  try {
    binary = globalThis.atob(value);
    if (globalThis.btoa(binary) !== value) throw invalidExportPreparation();
  } catch (reason) {
    if (reason instanceof PlatformFileGatewayError) throw reason;
    throw invalidExportPreparation();
  }
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function decodeExportPreparation(value: unknown): {
  files: PreparedExportFile[];
  foldIssues: unknown[];
} {
  const preparation = objectValue(value) as
    | (Record<string, unknown> & Partial<DocumentExportPreparationWire>)
    | null;
  if (
    !preparation ||
    !Array.isArray(preparation.files) ||
    preparation.files.length === 0 ||
    !Array.isArray(preparation.fold_issues)
  ) {
    throw invalidExportPreparation();
  }
  const suffixes = new Set<string>();
  const files = preparation.files.map((wireValue) => {
    const wire = objectValue(wireValue) as
      | (Record<string, unknown> & Partial<DocumentExportFileWire>)
      | null;
    if (
      !wire ||
      typeof wire.suffix !== "string" ||
      (wire.suffix !== "" && !/^-[0-9]+$/.test(wire.suffix)) ||
      suffixes.has(wire.suffix) ||
      typeof wire.content_type !== "string" ||
      wire.content_type.length === 0 ||
      !validOptionalIndex(wire.page_number) ||
      !validOptionalIndex(wire.first_cell) ||
      !validOptionalIndex(wire.last_cell) ||
      ((wire.first_cell === null) !== (wire.last_cell === null)) ||
      (wire.first_cell !== null &&
        wire.last_cell !== null &&
        wire.first_cell > wire.last_cell)
    ) {
      throw invalidExportPreparation();
    }
    suffixes.add(wire.suffix);
    return {
      suffix: wire.suffix,
      content_type: wire.content_type,
      page_number: wire.page_number,
      first_cell: wire.first_cell,
      last_cell: wire.last_cell,
      bytes: decodeBase64(wire.content_base64),
    };
  });
  return { files, foldIssues: preparation.fold_issues };
}

function fileStem(name: string): string {
  const extensionAt = name.lastIndexOf(".");
  return extensionAt > 0 ? name.slice(0, extensionAt) : name;
}

function diagramFileName(baseName: string, file: PreparedExportFile): string {
  return `${fileStem(baseName)}${file.suffix}.svg`;
}

function pageDescription(file: PreparedExportFile): string {
  if (file.first_cell === null || file.last_cell === null) {
    return file.page_number === 1 ? "表紙" : `${file.page_number ?? "不明"}ページ`;
  }
  return file.first_cell === file.last_cell
    ? `${file.first_cell}番`
    : `${file.first_cell}〜${file.last_cell}番`;
}

function describedFile(name: string, file: PreparedExportFile): string {
  return `${name}（${pageDescription(file)}）`;
}

function reasonText(reason: unknown): string {
  if (typeof reason === "string" && reason.length > 0) return reason;
  if (reason instanceof Error && reason.message.length > 0) return reason.message;
  return "原因を確認できませんでした。";
}

function partialDirectoryError(
  files: readonly PreparedExportFile[],
  names: readonly string[],
  failedAt: number,
  reason: unknown,
): PlatformFileGatewayError {
  const saved = files
    .slice(0, failedAt)
    .map((file, index) => describedFile(names[index], file));
  const remaining = files
    .slice(failedAt + 1)
    .map((file, index) => describedFile(names[failedAt + index + 1], file));
  return new PlatformFileGatewayError(
    "save",
    `折り図SVGの保存を途中で中止しました。保存済み（ここまで）: ${saved.length > 0 ? saved.join("、") : "なし"}。失敗: ${describedFile(names[failedAt], files[failedAt])}。未保存: ${remaining.length > 0 ? remaining.join("、") : "なし"}。原因: ${reasonText(reason)}`,
  );
}

async function abortPreparedSave(
  core: Ori3CoreWorkerClient,
  originalReason: unknown,
): Promise<never> {
  try {
    await invokePrivate<void>(
      core,
      "__web_document_save_abort",
      "document_save",
      {},
    );
  } catch (abortReason) {
    throw `保存操作に失敗し、保存準備も破棄できませんでした。保存の失敗: ${reasonText(originalReason)} 破棄の失敗: ${reasonText(abortReason)}`;
  }
  throw originalReason;
}

export function createBrowserCurrentDocumentCoordinator(
  registry: BrowserFileTokenRegistry = BROWSER_FILE_TOKEN_REGISTRY,
): BrowserCurrentDocumentCoordinator {
  let currentToken: BrowserFileToken | null = null;
  return {
    adopt(token): void {
      if (currentToken === token) return;
      registry.retain(token);
      const previous = currentToken;
      currentToken = token;
      if (previous) registry.release(previous);
    },
    clear(): void {
      const previous = currentToken;
      currentToken = null;
      if (previous) registry.release(previous);
    },
    current(): BrowserFileToken | null {
      return currentToken;
    },
  };
}

export function createDocumentLifecycleCoreInvoker(
  core: Ori3CoreWorkerClient,
  currentDocument: BrowserCurrentDocumentCoordinator,
): BrowserDocumentInvoker {
  return {
    async invoke<T>(
      command: BackendCommandName,
      args?: BackendInvokeArgs,
    ): Promise<T> {
      const result = await core.invoke<T>(command, args);
      if (command === "document_new") currentDocument.clear();
      return result;
    },
  };
}

export interface BrowserDocumentInvokerOptions {
  registry?: BrowserFileTokenRegistry;
  currentDocument?: BrowserCurrentDocumentCoordinator;
  notifyDelivery?: BrowserDocumentDeliveryListener;
}

export function createBrowserDocumentInvoker(
  core: Ori3CoreWorkerClient,
  options: BrowserDocumentInvokerOptions = {},
): BrowserDocumentInvoker {
  const registry = options.registry ?? BROWSER_FILE_TOKEN_REGISTRY;
  const currentDocument =
    options.currentDocument ?? createBrowserCurrentDocumentCoordinator(registry);
  const notifyDelivery = options.notifyDelivery ?? publishDelivery;
  const deliver = (notice: BrowserDocumentDeliveryNotice): void => {
    // 通知側の例外で、完了済みの保存を失敗へ変えない。例外自体はglobalへ隠さず出す。
    globalThis.queueMicrotask(() => notifyDelivery(notice));
  };

  const open = async <T>(argsValue: BackendInvokeArgs | undefined): Promise<T> => {
    const args = commandArgs(argsValue);
    strictFields("document_open", args, ["path"]);
    const token = requiredPath("document_open", args);
    const source = await decodeUtf8(await registry.read(token));
    await invokePrivate<void>(
      core,
      "__web_document_open_source",
      "document_open",
      { path: token, source },
    );
    const result = await core.invoke<T>("document_open", argsValue);
    currentDocument.adopt(token);
    return result;
  };

  const save = async <T>(argsValue: BackendInvokeArgs | undefined): Promise<T> => {
    const args = commandArgs(argsValue);
    const prepared = await invokePrivate<DocumentSavePreparation>(
      core,
      "__web_document_save_prepare",
      "document_save",
      args,
    );
    if (
      !objectValue(prepared) ||
      typeof prepared.path !== "string" ||
      typeof prepared.content !== "string" ||
      !isBrowserFileToken(prepared.path)
    ) {
      return abortPreparedSave(core, new PlatformFileGatewayError(
        "save",
        "保存するファイルの内容を確認できませんでした。作品は変更されていません。",
      ));
    }

    const target = prepared.path as BrowserFileToken;
    let temporaryDownload: BrowserFileToken | null = null;
    try {
      let writeResult;
      if (registry.destinationOf(target) === "read") {
        temporaryDownload = registry.registerDownload(registry.nameOf(target));
        writeResult = await registry.write(
          temporaryDownload,
          prepared.content,
          "application/json",
        );
      } else {
        writeResult = await registry.write(
          target,
          prepared.content,
          "application/json",
        );
      }
      const result = await core.invoke<T>("document_save", argsValue);
      currentDocument.adopt(target);
      deliver({
        command: "document_save",
        destination: writeResult.destination,
        names: [writeResult.name],
      });
      return result;
    } catch (reason) {
      return abortPreparedSave(core, reason);
    } finally {
      if (temporaryDownload) registry.release(temporaryDownload);
    }
  };

  const exportDocument = async <T>(
    argsValue: BackendInvokeArgs | undefined,
  ): Promise<T> => {
    const args = commandArgs(argsValue);
    strictFields("document_export", args, ["kind", "path", "options"]);
    const token = requiredPath("document_export", args);
    const preparationWire = await invokePrivate<unknown>(
      core,
      "__web_document_export_prepare",
      "document_export",
      { kind: args.kind, options: args.options },
    );
    const { files, foldIssues } = decodeExportPreparation(preparationWire);
    const destination = registry.destinationOf(token);

    if (args.kind === "DiagramSvg") {
      const names = files.map((file) =>
        diagramFileName(registry.nameOf(token), file),
      );
      if (destination === "directory") {
        for (let index = 0; index < files.length; index += 1) {
          try {
            await registry.writeDirectoryFile(
              token,
              names[index],
              files[index].bytes,
              files[index].content_type,
            );
          } catch (reason) {
            throw partialDirectoryError(files, names, index, reason);
          }
        }
        deliver({
          command: "document_export",
          destination: "directory",
          names,
        });
        return foldIssues as T;
      }
      if (destination === "download") {
        const zip = createBrowserZip(
          files.map((file, index) => ({ name: names[index], bytes: file.bytes })),
        );
        const writeResult = await registry.write(
          token,
          zip,
          "application/zip",
        );
        deliver({
          command: "document_export",
          destination: "download",
          names: [writeResult.name],
        });
        return foldIssues as T;
      }
      throw new PlatformFileGatewayError(
        "save",
        "折り図SVGは複数ファイルになるため、保存先を選ぶかZIPをダウンロードしてください。作品は変更されていません。",
      );
    }

    if (files.length !== 1 || destination === "directory") {
      throw invalidExportPreparation();
    }
    const writeResult = await registry.write(
      token,
      files[0].bytes,
      files[0].content_type,
    );
    deliver({
      command: "document_export",
      destination: writeResult.destination,
      names: [writeResult.name],
    });
    return foldIssues as T;
  };

  return {
    invoke<T>(
      command: BackendCommandName,
      args?: BackendInvokeArgs,
    ): Promise<T> {
      switch (command) {
        case "document_open":
          return open<T>(args);
        case "document_save":
          return save<T>(args);
        case "document_export":
          return exportDocument<T>(args);
        default:
          return Promise.reject(
            `Web版のmixed file adapterでは「${command}」を処理できません。`,
          );
      }
    },
  };
}
