import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../../desktop/src/store/appStore";
import type { BrowserDocumentDeliveryListener } from "./browserDocumentInvoker";
import {
  browserDocumentDeliveryText,
  installBrowserDocumentDeliveryNotices,
} from "./browserDocumentDelivery";

describe("browserのファイル配送通知", () => {
  beforeEach(() => {
    useAppStore.setState({ exportDeliveryNotice: null });
  });

  it("SVG全ページ名とZIPのdownload開始を、利用者向け文言へする", () => {
    expect(
      browserDocumentDeliveryText({
        command: "document_export",
        destination: "directory",
        names: ["作品-01.svg", "作品-02.svg"],
      }),
    ).toBe(
      "折り図SVGを選んだ保存先へ保存しました: 作品-01.svg、作品-02.svg",
    );
    expect(
      browserDocumentDeliveryText({
        command: "document_export",
        destination: "download",
        names: ["作品-折り図.zip"],
      }),
    ).toBe("「作品-折り図.zip」のダウンロードを開始しました");
  });

  it("production購読を単一storeへ接続し、解除関数もそのまま返す", () => {
    let listener: BrowserDocumentDeliveryListener | undefined;
    const unsubscribe = vi.fn();
    const stop = installBrowserDocumentDeliveryNotices((next) => {
      listener = next;
      return unsubscribe;
    });

    listener?.({
      command: "document_export",
      destination: "file-system",
      names: ["水風船.pdf"],
    });
    expect(useAppStore.getState().exportDeliveryNotice).toBe(
      "「水風船.pdf」へ書き出しました",
    );
    listener?.({
      command: "document_save",
      destination: "download",
      names: ["水風船.ori3"],
    });
    expect(useAppStore.getState().exportDeliveryNotice).toBe(
      "「水風船.pdf」へ書き出しました",
    );

    stop();
    expect(unsubscribe).toHaveBeenCalledTimes(1);
  });
});
