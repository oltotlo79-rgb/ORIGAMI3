// 書き出しダイアログ(EXP-001 / EXP-002、Task 4-3)。
// 展開図を画像ファイルとして保存する。入口は上部ツールバーのボタン1つだけで、
// 常設4区画は増やさない。「ラスタライズ」「dpi」などの用語は出さず、
// どちらを選ぶと何ができるかを日本語で書く(設計原則3b)。

import { save } from "@tauri-apps/plugin-dialog";
import { useAppStore } from "../../store/appStore";
import { fileName } from "../RecoveryDialog";
import type { ExportKind } from "../../lib/types";

/** 種類ごとの表示名・拡張子・ひとこと説明 */
export const EXPORT_CHOICES: {
  kind: ExportKind;
  label: string;
  ext: string;
  hint: string;
}[] = [
  {
    kind: "CpSvg",
    label: "展開図の画像(SVG)",
    ext: "svg",
    hint: "紙の実物大(mm)で保存します。いくら拡大してもぼやけず、印刷向きです。",
  },
  {
    kind: "CpPng",
    label: "展開図の画像(PNG)",
    ext: "png",
    hint: "写真と同じ形式です。そのまま画面で見たり貼り付けたりできます。",
  },
];

export function ExportDialog() {
  const open = useAppStore((s) => s.exportOpen);
  const kind = useAppStore((s) => s.exportKind);
  const includeAux = useAppStore((s) => s.exportIncludeAux);
  const longSide = useAppStore((s) => s.exportLongSide);
  const busy = useAppStore((s) => s.exportBusy);
  const error = useAppStore((s) => s.exportError);
  const savedPath = useAppStore((s) => s.exportSavedPath);
  const setOption = useAppStore((s) => s.setExportOption);
  const runExport = useAppStore((s) => s.runExport);
  const close = useAppStore((s) => s.closeExport);
  if (!open) return null;

  const choice = EXPORT_CHOICES.find((c) => c.kind === kind) ?? EXPORT_CHOICES[0];

  const handleSave = async () => {
    const path = await save({
      filters: [{ name: choice.label, extensions: [choice.ext] }],
    });
    if (typeof path === "string") await runExport(path);
  };

  return (
    <div className="dialog-backdrop">
      <div
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="export-title"
      >
        <h2 id="export-title">画像として書き出す</h2>
        <fieldset>
          <legend>何を書き出しますか</legend>
          {EXPORT_CHOICES.map((c) => (
            <label key={c.kind}>
              <input
                type="radio"
                name="export-kind"
                checked={kind === c.kind}
                onChange={() => setOption({ exportKind: c.kind })}
              />
              {c.label}
            </label>
          ))}
        </fieldset>
        <p className="hint">{choice.hint}</p>
        <label>
          <input
            type="checkbox"
            checked={includeAux}
            onChange={(e) => setOption({ exportIncludeAux: e.target.checked })}
          />
          補助線(下書きの線)も含める
        </label>
        {kind === "CpPng" && (
          <label>
            画像の大きさ(長辺の点数)
            <input
              type="number"
              min={1}
              max={16384}
              step={256}
              value={longSide}
              onChange={(e) => setOption({ exportLongSide: Number(e.target.value) })}
            />
          </label>
        )}
        {savedPath && <p className="hint">保存しました:{fileName(savedPath)}</p>}
        {error && <p className="error-text">保存できませんでした:{error}</p>}
        <div className="button-row">
          <button type="button" disabled={busy} onClick={() => void handleSave()}>
            {busy ? "書き出しています…" : "保存先を選んで書き出す"}
          </button>
          <button type="button" onClick={close}>
            閉じる
          </button>
        </div>
      </div>
    </div>
  );
}
