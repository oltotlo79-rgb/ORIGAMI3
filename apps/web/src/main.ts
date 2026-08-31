import "./webShell.css";
import {
  createProductionWebBridgeRuntime,
  installOri3WebBridge,
} from "./backend/installWebBridge";
import { installBrowserDocumentDeliveryNotices } from "./backend/browserDocumentDelivery";
import { BROWSER_PLATFORM_FILE_GATEWAY } from "./platform/browserFileGateway";
import { installPlatformFileGateway } from "../../desktop/src/platform/fileGateway";
import { setupWebShell } from "./webShell";
import { useAppStore } from "../../desktop/src/store/appStore";

setupWebShell(document);
installPlatformFileGateway(BROWSER_PLATFORM_FILE_GATEWAY);
const bridgeRuntime = createProductionWebBridgeRuntime({
  onRecoveryError(message): void {
    useAppStore.setState({
      errorMessage: message,
      documentSavedPath: null,
    });
  },
});
installOri3WebBridge(window, bridgeRuntime.dependencies);
const unsubscribeDelivery = installBrowserDocumentDeliveryNotices();
const hot = (
  import.meta as ImportMeta & {
    hot?: { dispose(callback: () => void): void };
  }
).hot;
hot?.dispose(() => {
  unsubscribeDelivery();
  bridgeRuntime.dispose();
});
void import("../../desktop/src/main.tsx");
