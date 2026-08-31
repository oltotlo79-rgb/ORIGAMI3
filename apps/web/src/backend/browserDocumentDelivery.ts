import { useAppStore } from "../../../desktop/src/store/appStore";
import {
  subscribeBrowserDocumentDelivery,
  type BrowserDocumentDeliveryListener,
  type BrowserDocumentDeliveryNotice,
} from "./browserDocumentInvoker";

type DeliverySubscription = (
  listener: BrowserDocumentDeliveryListener,
) => () => void;

export function browserDocumentDeliveryText(
  notice: BrowserDocumentDeliveryNotice,
): string | null {
  if (notice.command !== "document_export") return null;
  if (notice.destination === "directory") {
    return `折り図SVGを選んだ保存先へ保存しました: ${notice.names.join("、")}`;
  }
  if (notice.destination === "download") {
    return `「${notice.names.join("、")}」のダウンロードを開始しました`;
  }
  return `「${notice.names.join("、")}」へ書き出しました`;
}

/** browser adapterの配送結果を、dialogを閉じても残る単一storeへ接続する。 */
export function installBrowserDocumentDeliveryNotices(
  subscribe: DeliverySubscription = subscribeBrowserDocumentDelivery,
): () => void {
  return subscribe((notice) => {
    const message = browserDocumentDeliveryText(notice);
    if (message !== null) {
      useAppStore.setState({ exportDeliveryNotice: message });
    }
  });
}
