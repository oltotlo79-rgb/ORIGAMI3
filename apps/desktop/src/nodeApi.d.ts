// 検査(vitest)から正本や固有の一時ファイルを扱うためだけの型の宣言。
// @types/node を入れると本体のビルドにもNode向けの型が混ざるので、
// 使う関数だけをここで宣言する。アプリ本体はNodeのAPIを使わない。

declare module "node:fs" {
  export function mkdtempSync(prefix: string): string;
  export function readFileSync(path: string | URL): Uint8Array;
  export function readFileSync(path: string | URL, encoding: "utf8"): string;
  export function rmSync(
    path: string | URL,
    options?: { recursive?: boolean; force?: boolean },
  ): void;
  export function writeFileSync(
    path: string | URL,
    data: string | Uint8Array,
  ): void;
}

declare module "node:os" {
  export function tmpdir(): string;
}

// jsdomを使う検査では、画面側のURLがページの位置を基準にしてしまうため、
// 検査ファイル自身の場所からファイルの位置を組み立てるのに使う。
declare module "node:path" {
  export function dirname(path: string): string;
  export function join(...paths: string[]): string;
  export function resolve(...paths: string[]): string;
}

declare module "node:url" {
  export function fileURLToPath(url: string): string;
}

declare const process: {
  cwd(): string;
};
