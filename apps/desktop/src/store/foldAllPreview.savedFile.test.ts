import {
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, expect, it } from "vitest";

import {
  initSync,
  Ori3WasmBackend,
} from "../../../web/src/backend/generated/ori3-web/ori3_web.js";
import type { Document, DocumentView } from "../lib/types";

interface SavePreparation {
  path: string;
  content: string;
}

interface FoldAllResult {
  requested_percent: number;
}

const TEMP_PREFIX = join(tmpdir(), "ori3-fold-all-saved-file-");
let outputDirectory = "";

function invoke<T>(
  backend: Ori3WasmBackend,
  command: string,
  args?: Record<string, unknown>,
): T {
  return JSON.parse(
    backend.invoke_json(
      JSON.stringify({
        command,
        args: args ?? null,
      }),
    ),
  ) as T;
}

function containsExactNumber(value: unknown, expected: number): boolean {
  if (typeof value === "number") return Object.is(value, expected);
  if (Array.isArray(value)) {
    return value.some((item) => containsExactNumber(item, expected));
  }
  if (typeof value === "object" && value !== null) {
    return Object.values(value).some((item) =>
      containsExactNumber(item, expected),
    );
  }
  return false;
}

function edgeCount(document: Document): number {
  return document.cp.edges.length;
}

beforeAll(() => {
  const wasmBytes = Uint8Array.from(
    readFileSync(
      new URL(
        "../../../web/src/backend/generated/ori3-web/ori3_web_bg.wasm",
        import.meta.url,
      ),
    ),
  );
  initSync({ module: wasmBytes });
  outputDirectory = mkdtempSync(TEMP_PREFIX);
});

afterAll(() => {
  if (outputDirectory.startsWith(TEMP_PREFIX)) {
    rmSync(outputDirectory, { recursive: true, force: true });
  }
});

it("一斉表示中の73%を実ファイルへ保存せず、新しいbackendで開き直しても手順・履歴に現れない", () => {
  const backend = new Ori3WasmBackend();
  const reopenedBackend = new Ori3WasmBackend();
  const savedPath = join(outputDirectory, "一斉表示を保存した作品.ori3");

  try {
    const fresh = invoke<DocumentView>(backend, "document_new", {
      paper: { width_mm: 150, height_mm: 150 },
    });
    const edited = invoke<DocumentView>(backend, "edit_apply", {
      op: {
        type: "AddSegment",
        a: [0, 0.5],
        b: [1, 0.5],
        kind: "Valley",
      },
    });
    expect(edited.doc.sequence).toHaveLength(0);
    expect(edgeCount(edited.doc)).toBeGreaterThan(edgeCount(fresh.doc));

    const preview = invoke<FoldAllResult>(backend, "fold_all_preview", {
      percent: 73,
      warmSeed: null,
    });
    expect(preview.requested_percent).toBe(73);

    const undone = invoke<DocumentView>(backend, "edit_undo");
    expect(undone.doc).toEqual(fresh.doc);
    expect(undone.doc.sequence).toHaveLength(0);
    const redone = invoke<DocumentView>(backend, "edit_redo");
    expect(redone.doc).toEqual(edited.doc);
    expect(redone.doc.sequence).toHaveLength(0);

    const previewAtSave = invoke<FoldAllResult>(backend, "fold_all_preview", {
      percent: 73,
      warmSeed: null,
    });
    expect(previewAtSave.requested_percent).toBe(73);
    const prepared = invoke<SavePreparation>(
      backend,
      "__web_document_save_prepare",
      { path: savedPath },
    );
    expect(prepared.path).toBe(savedPath);
    writeFileSync(savedPath, prepared.content);

    const savedSource = readFileSync(savedPath, "utf8");
    const savedBytes = new TextEncoder().encode(savedSource).byteLength;
    expect(savedBytes).toBeGreaterThan(0);
    const savedDocument = JSON.parse(savedSource) as Document;
    expect(savedDocument.sequence).toHaveLength(0);
    expect(savedDocument.cp.edges).toEqual(edited.doc.cp.edges);
    expect(containsExactNumber(savedDocument, 73)).toBe(false);
    for (const temporaryField of [
      "foldAllPreview",
      "fold_all_preview",
      "requested_percent",
      "requested_angles",
      "next_warm_seed",
      "appliedPercent",
      "entryFrame3d",
    ]) {
      expect(savedSource).not.toContain(`"${temporaryField}"`);
    }

    invoke<null>(reopenedBackend, "__web_document_open_source", {
      path: savedPath,
      source: savedSource,
    });
    const reopened = invoke<DocumentView>(reopenedBackend, "document_open", {
      path: savedPath,
    });
    expect(reopened.doc.sequence).toHaveLength(0);
    expect(reopened.doc.cp.edges).toEqual(edited.doc.cp.edges);
    expect(containsExactNumber(reopened.doc, 73)).toBe(false);
    console.info(
      `fold-all saved roundtrip: bytes=${savedBytes} sequence=${reopened.doc.sequence.length} edges=${reopened.doc.cp.edges.length} percent73=false`,
    );
  } finally {
    backend.free();
    reopenedBackend.free();
  }
});
