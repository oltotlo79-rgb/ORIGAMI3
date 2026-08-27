import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import desktopConfig from "../desktop/vite.config.ts";
import type {
  ConfigEnv,
  UserConfig,
} from "../desktop/node_modules/vite";

const webRoot = fileURLToPath(new URL(".", import.meta.url));
const desktopNodeModules = resolve(webRoot, "../desktop/node_modules");
const sharedPackages = [
  "@tauri-apps/api",
  "@tauri-apps/plugin-dialog",
  "@tauri-apps/plugin-opener",
  "react",
  "react-dom",
  "three",
  "zustand",
] as const;

async function resolveDesktopConfig(env: ConfigEnv): Promise<UserConfig> {
  if (typeof desktopConfig === "function") {
    return desktopConfig(env);
  }
  return desktopConfig;
}

export default async function webConfig(env: ConfigEnv): Promise<UserConfig> {
  const desktop = await resolveDesktopConfig(env);
  return {
    ...desktop,
    root: webRoot,
    publicDir: resolve(webRoot, "public"),
    resolve: {
      ...desktop.resolve,
      alias: sharedPackages.map((name) => ({
        find: name,
        replacement: resolve(desktopNodeModules, name),
      })),
    },
    build: {
      ...desktop.build,
      outDir: resolve(webRoot, "dist"),
      emptyOutDir: true,
    },
  };
}
