import { zipSync, type Zippable } from "fflate";

export interface BrowserZipFile {
  name: string;
  bytes: Uint8Array;
}

const FIXED_ZIP_TIME = "1980-01-01T00:00:00";

/**
 * directory pickerを持たないブラウザだけで使うZIP生成。
 * 時刻と入力順を固定し、同じRust bytesから常に同じ配布物を作る。
 */
export function createBrowserZip(files: readonly BrowserZipFile[]): Uint8Array {
  const entries: Zippable = Object.create(null) as Zippable;
  for (const file of files) {
    if (
      file.name.length === 0 ||
      file.name.includes("/") ||
      file.name.includes(String.fromCharCode(92)) ||
      file.name === "." ||
      file.name === ".." ||
      Object.prototype.hasOwnProperty.call(entries, file.name)
    ) {
      throw new Error("ZIPへ入れるファイル名を確認できませんでした。");
    }
    entries[file.name] = file.bytes;
  }
  return zipSync(entries, { level: 6, mtime: FIXED_ZIP_TIME });
}
