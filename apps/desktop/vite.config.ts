import { gzipSync } from "node:zlib";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
const INITIAL_GZIP_LIMIT = 250_000;
const MAX_RAW_CHUNK_LIMIT = 500_000;
const INITIAL_VIEWER_ALLOWLIST = new Set([
  "DeferredViewer3D.tsx",
  "edgeHighlight.ts",
  "foldDraw.ts",
  "grabFold.ts",
]);

function manualChunk(id: string): string | undefined {
  const normalized = id.replaceAll("\\", "/");
  if (normalized.includes("/node_modules/three/build/three.core.js")) {
    return "three-core";
  }
  if (normalized.includes("/node_modules/three/build/three.module.js")) {
    return "three-module";
  }
  if (normalized.includes("/node_modules/three/examples/jsm/")) {
    return "three-examples";
  }
  if (
    /\/node_modules\/(?:react|react-dom|scheduler)\//.test(normalized)
  ) {
    return "react-vendor";
  }
  return undefined;
}

function bundleBudget(): Plugin {
  return {
    name: "origami3-bundle-budget",
    apply: "build",
    generateBundle(_options, bundle) {
      const chunks = Object.values(bundle).filter(
        (output): output is Extract<typeof output, { type: "chunk" }> =>
          output.type === "chunk",
      );
      const chunksByFile = new Map(chunks.map((chunk) => [chunk.fileName, chunk]));
      const initialFiles = new Set<string>();
      const pending = chunks.filter((chunk) => chunk.isEntry).map((chunk) => chunk.fileName);
      while (pending.length > 0) {
        const fileName = pending.pop();
        if (fileName === undefined || initialFiles.has(fileName)) continue;
        initialFiles.add(fileName);
        for (const imported of chunksByFile.get(fileName)?.imports ?? []) pending.push(imported);
      }

      const initialChunks = chunks.filter((chunk) => initialFiles.has(chunk.fileName));
      const initialRawBytes = initialChunks.reduce(
        (total, chunk) => total + new TextEncoder().encode(chunk.code).byteLength,
        0,
      );
      const initialGzipBytes = initialChunks.reduce(
        (total, chunk) =>
          total + gzipSync(new TextEncoder().encode(chunk.code)).byteLength,
        0,
      );
      const chunkBytes = chunks.map((chunk) => ({
        fileName: chunk.fileName,
        bytes: new TextEncoder().encode(chunk.code).byteLength,
      }));
      const largest = chunkBytes.reduce((left, right) =>
        right.bytes > left.bytes ? right : left,
      );
      const forbiddenInitialModules = initialChunks.flatMap((chunk) =>
        Object.keys(chunk.modules).flatMap((id) => {
          const normalized = id.replaceAll("\\", "/");
          if (normalized.includes("/node_modules/three/")) return [normalized];
          const marker = "/src/components/Viewer3D/";
          const markerAt = normalized.indexOf(marker);
          if (markerAt < 0) return [];
          const leaf = normalized.slice(markerAt + marker.length);
          return INITIAL_VIEWER_ALLOWLIST.has(leaf) ? [] : [normalized];
        }),
      );

      this.info(
        `initial raw ${initialRawBytes} bytes / initial gzip ${initialGzipBytes} bytes / ` +
          `largest raw ${largest.bytes} bytes (${largest.fileName})`,
      );
      if (initialGzipBytes > INITIAL_GZIP_LIMIT) {
        this.error(
          `初期JS gzipが上限を超えました: ${initialGzipBytes} > ${INITIAL_GZIP_LIMIT}`,
        );
      }
      if (largest.bytes > MAX_RAW_CHUNK_LIMIT) {
        this.error(
          `最大raw chunkが上限を超えました: ${largest.fileName} ${largest.bytes} > ${MAX_RAW_CHUNK_LIMIT}`,
        );
      }
      if (forbiddenInitialModules.length > 0) {
        this.error(
          `初期集合へViewer/Threeが戻りました:\n${forbiddenInitialModules.join("\n")}`,
        );
      }
    },
  };
}

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), bundleBudget()],
  build: {
    rollupOptions: {
      output: {
        manualChunks: manualChunk,
      },
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
