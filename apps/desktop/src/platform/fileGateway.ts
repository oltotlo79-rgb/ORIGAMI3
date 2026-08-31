import type { FileDialogFilter } from "../lib/foldFileExchange";
import { TAURI_PLATFORM_FILE_GATEWAY } from "./tauriFileGateway";

declare const browserFileTokenBrand: unique symbol;

/**
 * ブラウザ内だけで有効なファイル能力を、既存IPCの `path: string` に載せる識別子。
 * 実ファイルやOSのパスではなく、対応するregistryが生きている間だけ使える。
 */
export type BrowserFileToken = string & {
  readonly [browserFileTokenBrand]: "BrowserFileToken";
};

/** UI表示だけでdownloadと通常保存を区別する。tokenの有効性確認はWeb registryが行う。 */
export function isBrowserDownloadToken(
  path: string,
): path is BrowserFileToken {
  return path.startsWith("browser-file://download/");
}

export type PlatformSaveMode = "choose-destination" | "download";
export type PlatformFileOperation = "open" | "save" | "download";

export interface PlatformOpenFileOptions {
  filters: FileDialogFilter[];
}

export interface PlatformSaveFileOptions {
  filters: FileDialogFilter[];
  /** Webのダウンロード名。TauriではOSの保存画面に名前を委ねる。 */
  suggestedName: string;
  /** 折り図SVGのように、1回の操作で複数ファイルを保存する場合だけ指定する。 */
  multipleFiles?: boolean;
}

export interface PlatformFileGateway {
  readonly saveMode: PlatformSaveMode;
  readonly multipleFileSaveMode?: "choose-directory" | "download";
  chooseOpenFile(options: PlatformOpenFileOptions): Promise<string | null>;
  chooseSaveFile(options: PlatformSaveFileOptions): Promise<string | null>;
  /**
   * Web tokenの能力を破棄する。mixed adapterはcomponentがawaitしている間に
   * read/writeを完了し、対応するactionのPromiseがsettleした後はtokenを使わない。
   */
  release(path: string): void;
}

export class PlatformFileGatewayError extends Error {
  readonly operation: PlatformFileOperation;

  constructor(operation: PlatformFileOperation, message: string) {
    super(message);
    this.name = "PlatformFileGatewayError";
    this.operation = operation;
  }
}

function isPermissionDenied(reason: unknown): boolean {
  return (
    typeof DOMException !== "undefined" &&
    reason instanceof DOMException &&
    (reason.name === "NotAllowedError" || reason.name === "SecurityError")
  );
}

/** ファイル画面やブラウザAPIの失敗を、作品が変わらなかったことまで含む日本語にする。 */
export function platformFileErrorMessage(
  reason: unknown,
  operation: PlatformFileOperation,
): string {
  if (reason instanceof PlatformFileGatewayError) return reason.message;

  const action =
    operation === "open"
      ? "ファイルを開く"
      : operation === "download"
        ? "ファイルをダウンロードする"
        : "ファイルを保存する";
  if (isPermissionDenied(reason)) {
    return `${action}権限が許可されませんでした。作品は変更されていません。`;
  }
  return `${action}操作を完了できませんでした。作品は変更されていません。`;
}

let activeGateway: PlatformFileGateway = TAURI_PLATFORM_FILE_GATEWAY;

/** React componentが実行環境を分岐せずに使う、ファイル操作の唯一の入口。 */
export function getPlatformFileGateway(): PlatformFileGateway {
  return activeGateway;
}

/** Web起動時にbrowser実装を設定する。戻り値は検査で安全に元へ戻すためにも使う。 */
export function installPlatformFileGateway(
  gateway: PlatformFileGateway,
): () => void {
  const previous = activeGateway;
  activeGateway = gateway;
  return () => {
    if (activeGateway === gateway) activeGateway = previous;
  };
}
