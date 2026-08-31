import type {
  BackendCommandName,
  BackendInvokeArgs,
  Ori3WebBridge,
} from "../../../desktop/src/ipc/runtime";
import {
  createOri3CoreWorkerClient,
  type Ori3CoreWorkerClient,
} from "./coreWorkerClient";
import {
  createBrowserCurrentDocumentCoordinator,
  createBrowserDocumentInvoker,
  createDocumentLifecycleCoreInvoker,
} from "./browserDocumentInvoker";
import {
  createProposalJobRegistry,
  type ProposalJobRegistry,
} from "./proposalJobRegistry";
import { WEB_COMMAND_ROUTES, type WebCommandRoute } from "./routes";
import { createWebRecoveryRuntime, type WebRecoveryRepositoryPort } from "../recovery";
import type { AutosaveClock } from "../recovery/AutosaveScheduler";
import type { BrowserFileTokenRegistry } from "../platform/browserFileTokenRegistry";

export interface WebCommandInvoker {
  invoke<T>(command: BackendCommandName, args?: BackendInvokeArgs): Promise<T>;
}

export type WebBridgeDependencies = Record<
  WebCommandRoute,
  WebCommandInvoker
>;

export interface ProductionWebBridgeOptions {
  onRecoveryError(message: string): void;
  repository?: WebRecoveryRepositoryPort;
  registry?: BrowserFileTokenRegistry;
  recoveryClock?: AutosaveClock;
  recoveryDebounceMs?: number;
  recoveryCheckpointMs?: number;
}

export interface ProductionWebBridgeRuntime {
  dependencies: WebBridgeDependencies;
  dispose(): void;
}

function unavailableInvoker(boundary: string): WebCommandInvoker {
  return {
    invoke<T>(command: BackendCommandName): Promise<T> {
      return Promise.reject(
        `Web版の「${command}」は${boundary}との接続を準備中のため、まだ利用できません。`,
      );
    },
  };
}

export function createWebBridgeDependencies(
  core: Ori3CoreWorkerClient = createOri3CoreWorkerClient(),
  proposal: ProposalJobRegistry = createProposalJobRegistry(),
): WebBridgeDependencies {
  const currentDocument = createBrowserCurrentDocumentCoordinator();
  return {
    core: createDocumentLifecycleCoreInvoker(core, currentDocument),
    proposal,
    browser: unavailableInvoker("ブラウザ保存領域"),
    mixed: createBrowserDocumentInvoker(core, { currentDocument }),
  };
}

/** browser製品用。復旧repository・autosaveを同じcore/current documentへ結線する。 */
export function createProductionWebBridgeRuntime(
  options: ProductionWebBridgeOptions,
): ProductionWebBridgeRuntime {
  const core = createOri3CoreWorkerClient();
  const proposal = createProposalJobRegistry();
  const currentDocument = createBrowserCurrentDocumentCoordinator(
    options.registry,
  );
  const recovery = createWebRecoveryRuntime(core, {
    currentDocument,
    onError: options.onRecoveryError,
    ...(options.repository === undefined
      ? {}
      : { repository: options.repository }),
    ...(options.registry === undefined ? {} : { registry: options.registry }),
    ...(options.recoveryClock === undefined
      ? {}
      : { clock: options.recoveryClock }),
    ...(options.recoveryDebounceMs === undefined
      ? {}
      : { debounceMs: options.recoveryDebounceMs }),
    ...(options.recoveryCheckpointMs === undefined
      ? {}
      : { checkpointMs: options.recoveryCheckpointMs }),
  });
  const lifecycleCore = createDocumentLifecycleCoreInvoker(
    core,
    currentDocument,
  );
  const mixed = createBrowserDocumentInvoker(core, {
    currentDocument,
    ...(options.registry === undefined ? {} : { registry: options.registry }),
  });
  return {
    dependencies: {
      core: recovery.decorateCore(lifecycleCore),
      proposal,
      browser: recovery.browser,
      mixed: recovery.decorateMixed(mixed),
    },
    dispose(): void {
      recovery.dispose();
      proposal.dispose();
      core.dispose();
    },
  };
}

export function invokeWebCommand<T>(
  dependencies: WebBridgeDependencies,
  command: BackendCommandName,
  args?: BackendInvokeArgs,
): Promise<T> {
  return dependencies[WEB_COMMAND_ROUTES[command]].invoke<T>(command, args);
}

export function createOri3WebBridge(
  dependencies: WebBridgeDependencies = createWebBridgeDependencies(),
): Ori3WebBridge {
  return {
    invoke<T>(command: BackendCommandName, args?: BackendInvokeArgs): Promise<T> {
      return invokeWebCommand<T>(dependencies, command, args);
    },
  };
}

export function installOri3WebBridge(
  target: Window,
  dependencies: WebBridgeDependencies = createWebBridgeDependencies(),
): void {
  target.__ori3Web = createOri3WebBridge(dependencies);
}
