// @vitest-environment jsdom

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it } from "vitest";
import { setupWebShell } from "./webShell";

const pageHtml = readFileSync(resolve("../web/index.html"), "utf8");

function requiredElement<T extends Element>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) throw new Error(`検査対象がありません: ${selector}`);
  return element;
}

beforeEach(() => {
  document.open();
  document.write(pageHtml);
  document.close();
  setupWebShell(document);
});

describe("Web版のダウンロード案内", () => {
  it("取扱説明書ボタンで正確な文言とはい・いいえを表示する", () => {
    const dialog = requiredElement<HTMLDialogElement>("#manual-confirmation");

    requiredElement<HTMLButtonElement>("#manual-download").click();

    expect(dialog.hasAttribute("open")).toBe(true);
    expect(
      requiredElement("#manual-confirmation-message").textContent?.trim(),
    ).toBe("取扱説明書（PDF、約28 MB）をダウンロードしますか？");
    expect(requiredElement("#manual-confirm-yes").textContent?.trim()).toBe(
      "はい",
    );
    expect(requiredElement("#manual-confirm-no").textContent?.trim()).toBe(
      "いいえ",
    );
  });

  it("いいえを選ぶと確認を閉じる", () => {
    const dialog = requiredElement<HTMLDialogElement>("#manual-confirmation");
    requiredElement<HTMLButtonElement>("#manual-download").click();

    requiredElement<HTMLButtonElement>("#manual-confirm-no").click();

    expect(dialog.hasAttribute("open")).toBe(false);
  });

  it("はいは最新版PDF、速い版はWindows版のGitHub Releasesへ進む", () => {
    const manual = requiredElement<HTMLAnchorElement>("#manual-confirm-yes");
    const windows = requiredElement<HTMLAnchorElement>(
      'a[href="https://github.com/oltotlo79-rgb/ORIGAMI3/releases/latest"]',
    );

    expect(manual.href).toBe(
      "https://github.com/oltotlo79-rgb/ORIGAMI3/releases/latest/download/ORIGAMI3.pdf",
    );
    expect(windows.textContent?.trim()).toBe("速い版（Windows）");
    expect(windows.href).toBe(
      "https://github.com/oltotlo79-rgb/ORIGAMI3/releases/latest",
    );
  });
});
