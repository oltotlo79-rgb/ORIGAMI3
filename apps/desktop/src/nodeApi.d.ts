// 検査(vitest)からファイルを読むためだけの型の宣言。
// @types/node を入れると本体のビルドにもNode向けの型が混ざるので、
// 使う関数だけをここで宣言する。アプリ本体はNodeのAPIを使わない。

declare module "node:fs" {
  export function readFileSync(path: string | URL, encoding: "utf8"): string;
}
