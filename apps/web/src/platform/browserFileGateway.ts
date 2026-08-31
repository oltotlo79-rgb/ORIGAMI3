import {
  PlatformFileGatewayError,
  platformFileErrorMessage,
  type PlatformFileGateway,
  type PlatformFileOperation,
  type PlatformOpenFileOptions,
} from "../../../desktop/src/platform/fileGateway";
import type { FileDialogFilter } from "../../../desktop/src/lib/foldFileExchange";
import {
  BROWSER_FILE_TOKEN_REGISTRY,
  isBrowserFileToken,
  type BrowserFileTokenRegistry,
} from "./browserFileTokenRegistry";

interface BrowserFilePickerType {
  description: string;
  accept: Record<string, string[]>;
}

interface BrowserOpenPickerOptions {
  multiple: false;
  types: BrowserFilePickerType[];
}

interface BrowserSavePickerOptions {
  suggestedName: string;
  types: BrowserFilePickerType[];
}

interface BrowserDirectoryPickerOptions {
  mode: "readwrite";
}

export interface BrowserFilePickerWindow extends Window {
  showOpenFilePicker?: (
    options: BrowserOpenPickerOptions,
  ) => Promise<FileSystemFileHandle[]>;
  showSaveFilePicker?: (
    options: BrowserSavePickerOptions,
  ) => Promise<FileSystemFileHandle>;
  showDirectoryPicker?: (
    options: BrowserDirectoryPickerOptions,
  ) => Promise<FileSystemDirectoryHandle>;
}

export type BrowserInputFilePicker = (
  browserDocument: Document,
  filters: FileDialogFilter[],
) => Promise<File | null>;

function mimeType(extension: string): string {
  switch (extension.toLowerCase()) {
    case "svg":
      return "image/svg+xml";
    case "png":
      return "image/png";
    case "pdf":
      return "application/pdf";
    case "fold":
    case "ori3":
      return "application/json";
    default:
      return "application/octet-stream";
  }
}

function pickerTypes(filters: FileDialogFilter[]): BrowserFilePickerType[] {
  return filters.map((filter) => {
    const accept: Record<string, string[]> = {};
    for (const extension of filter.extensions) {
      const mime = mimeType(extension);
      (accept[mime] ??= []).push(`.${extension}`);
    }
    return { description: filter.name, accept };
  });
}

function inputAccept(filters: FileDialogFilter[]): string {
  return filters
    .flatMap((filter) => filter.extensions)
    .map((extension) => `.${extension}`)
    .join(",");
}

function zipSuggestedName(suggestedName: string): string {
  const extensionAt = suggestedName.lastIndexOf(".");
  const stem = extensionAt > 0 ? suggestedName.slice(0, extensionAt) : suggestedName;
  return `${stem}-折り図.zip`;
}

export function chooseFileWithInput(
  browserDocument: Document,
  filters: FileDialogFilter[],
): Promise<File | null> {
  return new Promise((resolve, reject) => {
    if (!browserDocument.body) {
      reject(
        new PlatformFileGatewayError(
          "open",
          "ファイルを選ぶ画面を準備できませんでした。作品は変更されていません。",
        ),
      );
      return;
    }

    const input = browserDocument.createElement("input");
    input.type = "file";
    input.accept = inputAccept(filters);
    input.multiple = false;
    input.hidden = true;
    let settled = false;

    const finish = (file: File | null): void => {
      if (settled) return;
      settled = true;
      input.remove();
      resolve(file);
    };
    input.addEventListener("change", () => finish(input.files?.[0] ?? null), {
      once: true,
    });
    input.addEventListener("cancel", () => finish(null), { once: true });
    browserDocument.body.append(input);
    try {
      input.click();
    } catch (reason) {
      input.remove();
      reject(reason);
    }
  });
}

function isCancelled(reason: unknown): boolean {
  return (
    typeof DOMException !== "undefined" &&
    reason instanceof DOMException &&
    reason.name === "AbortError"
  );
}

function gatewayError(
  reason: unknown,
  operation: PlatformFileOperation,
): PlatformFileGatewayError {
  return reason instanceof PlatformFileGatewayError
    ? reason
    : new PlatformFileGatewayError(
        operation,
        platformFileErrorMessage(reason, operation),
      );
}

export interface BrowserPlatformFileGatewayOptions {
  pickerWindow?: BrowserFilePickerWindow;
  browserDocument?: Document;
  registry?: BrowserFileTokenRegistry;
  chooseWithInput?: BrowserInputFilePicker;
}

export function createBrowserPlatformFileGateway({
  pickerWindow =
    (typeof window === "undefined"
      ? undefined
      : (window as BrowserFilePickerWindow)),
  browserDocument = typeof document === "undefined" ? undefined : document,
  registry = BROWSER_FILE_TOKEN_REGISTRY,
  chooseWithInput = chooseFileWithInput,
}: BrowserPlatformFileGatewayOptions = {}): PlatformFileGateway {
  const saveMode =
    typeof pickerWindow?.showSaveFilePicker === "function"
      ? "choose-destination"
      : "download";

  return {
    saveMode,
    multipleFileSaveMode:
      typeof pickerWindow?.showDirectoryPicker === "function"
        ? "choose-directory"
        : "download",

    async chooseOpenFile({ filters }: PlatformOpenFileOptions) {
      if (typeof pickerWindow?.showOpenFilePicker === "function") {
        try {
          const handles = await pickerWindow.showOpenFilePicker({
            multiple: false,
            types: pickerTypes(filters),
          });
          const handle = handles[0];
          if (!handle) return null;
          return registry.registerFileSystemDestination(handle);
        } catch (reason) {
          if (isCancelled(reason)) return null;
          throw gatewayError(reason, "open");
        }
      }

      if (!browserDocument) {
        throw new PlatformFileGatewayError(
          "open",
          "この環境ではファイルを選べません。作品は変更されていません。",
        );
      }
      try {
        const file = await chooseWithInput(browserDocument, filters);
        return file ? registry.registerRead(file) : null;
      } catch (reason) {
        throw gatewayError(reason, "open");
      }
    },

    async chooseSaveFile({ filters, suggestedName, multipleFiles }) {
      if (multipleFiles === true) {
        if (typeof pickerWindow?.showDirectoryPicker !== "function") {
          return registry.registerDownload(zipSuggestedName(suggestedName));
        }
        try {
          const handle = await pickerWindow.showDirectoryPicker({
            mode: "readwrite",
          });
          return registry.registerDirectoryDestination(handle, suggestedName);
        } catch (reason) {
          if (isCancelled(reason)) return null;
          throw gatewayError(reason, "save");
        }
      }
      if (typeof pickerWindow?.showSaveFilePicker !== "function") {
        return registry.registerDownload(suggestedName);
      }
      try {
        const handle = await pickerWindow.showSaveFilePicker({
          suggestedName,
          types: pickerTypes(filters),
        });
        return registry.registerFileSystemDestination(handle);
      } catch (reason) {
        if (isCancelled(reason)) return null;
        throw gatewayError(reason, "save");
      }
    },

    release(path): void {
      if (isBrowserFileToken(path)) registry.release(path);
    },
  };
}

export const BROWSER_PLATFORM_FILE_GATEWAY =
  createBrowserPlatformFileGateway();
