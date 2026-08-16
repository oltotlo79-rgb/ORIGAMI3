// 検査(vitest)からファイルを読むためだけの型の宣言。
// @types/node を入れると本体のビルドにもNode向けの型が混ざるので、
// 使う関数だけをここで宣言する。アプリ本体はNodeのAPIを使わない。

declare module "node:fs" {
  export function readFileSync(path: string | URL): Uint8Array;
  export function readFileSync(path: string | URL, encoding: "utf8"): string;
}

// jsdomを使う検査では、画面側のURLがページの位置を基準にしてしまうため、
// 検査ファイル自身の場所からファイルの位置を組み立てるのに使う。
declare module "node:path" {
  export function dirname(path: string): string;
  export function join(...paths: string[]): string;
}

declare module "node:url" {
  export function fileURLToPath(url: string): string;
}
