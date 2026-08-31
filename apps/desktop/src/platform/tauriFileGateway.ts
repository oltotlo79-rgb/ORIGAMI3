import type { FileDialogFilter } from "../lib/foldFileExchange";

interface TauriDialogOptions {
  filters: FileDialogFilter[];
}

/** Tauri pluginの読込みは、この実装の操作時だけに限定する。 */
export const TAURI_PLATFORM_FILE_GATEWAY = {
  saveMode: "choose-destination" as const,

  async chooseOpenFile({ filters }: TauriDialogOptions): Promise<string | null> {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({ filters, multiple: false });
    return typeof selected === "string" ? selected : null;
  },

  async chooseSaveFile({ filters }: TauriDialogOptions): Promise<string | null> {
    const { save } = await import("@tauri-apps/plugin-dialog");
    return save({ filters });
  },

  release(): void {
    // OS pathはgateway内の能力を保持しない。
  },
};
