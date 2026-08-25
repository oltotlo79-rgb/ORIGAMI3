// 書き出しダイアログ(EXP-001 / EXP-002、Task 4-3)。
// 展開図と折り図をファイルとして保存する。入口は上部ツールバーのボタン1つだけで、
// 常設4区画は増やさない。「ラスタライズ」「dpi」などの用語は出さず、
// どちらを選ぶと何ができるかを日本語で書く(設計原則3b)。

import { save } from "@tauri-apps/plugin-dialog";
import { useRef } from "react";
import { useAppStore } from "../../store/appStore";
import { fileName } from "../RecoveryDialog";
import { NumberStepper } from "../NumberStepper";
import { ModalDialog } from "./ModalDialog";
import { EXPORT_CHOICES } from "./exportChoices";

// 既存の画面・検査のimport先を保つ。choices本体だけはAppでも使えるpure moduleへ置く。
export { EXPORT_CHOICES } from "./exportChoices";

/** 折り手順がまだ無いときに、その種類を選べない理由(日本語) */
export const NO_STEPS_REASON =
  "折り手順がまだありません。紙を折って手順を作ると折り図を書き出せます。";

export function ExportDialog() {
  const initialChoiceRef = useRef<HTMLInputElement>(null);
  const saveButtonRef = useRef<HTMLButtonElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
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
  const stepCount = useAppStore((s) => s.doc?.sequence.length ?? 0);
  if (!open) return null;

  const choice = EXPORT_CHOICES.find((c) => c.kind === kind) ?? EXPORT_CHOICES[0];
  // 折り図は手順が要る。選べないときも選択肢は残し、理由を出す
  const blocked = (c: (typeof EXPORT_CHOICES)[number]) =>
    c.needsSteps === true && stepCount === 0;
  const initialChoice = blocked(choice)
    ? EXPORT_CHOICES.find((candidate) => !blocked(candidate))
    : choice;

  const handleSave = async () => {
    try {
      const path = await save({
        filters: [{ name: choice.label, extensions: [choice.ext] }],
      });
      if (typeof path === "string") {
        const exportTask = runExport(path);
        // 保存先を選んだ後、処理中は無効になる保存ボタンではなく、
        // いつでも使える既存の「閉じる」へ一時的に戻す。
        queueMicrotask(() => closeButtonRef.current?.focus({ preventScroll: true }));
        await exportTask;
      }
    } finally {
      // 中止時または書き出し完了後は、次の保存を始められる同じ操作へ戻す。
      queueMicrotask(() => saveButtonRef.current?.focus({ preventScroll: true }));
    }
  };

  return (
    <ModalDialog
      labelledBy="export-title"
      initialFocusRef={initialChoiceRef}
      escapeAction={busy ? { kind: "stay" } : { kind: "dismiss", run: close }}
      data-floating-ui="export-dialog"
    >
      <h2 id="export-title">展開図・折り図を書き出す</h2>
      <fieldset>
        <legend>何を書き出しますか</legend>
        {EXPORT_CHOICES.map((c) => (
          <label key={c.kind}>
            <input
              ref={c.kind === initialChoice?.kind ? initialChoiceRef : undefined}
              type="radio"
              name="export-kind"
              checked={kind === c.kind}
              disabled={blocked(c)}
              onChange={() => setOption({ exportKind: c.kind })}
            />
            {c.label}
          </label>
        ))}
      </fieldset>
      <p className="hint">{choice.hint}</p>
      {stepCount === 0 && (
        <p className="hint">折り図について:{NO_STEPS_REASON}</p>
      )}
      {(kind === "CpSvg" || kind === "CpPng") && (
        <label>
          <input
            type="checkbox"
            checked={includeAux}
            onChange={(e) => setOption({ exportIncludeAux: e.target.checked })}
          />
          補助線(下書きの線)も含める
        </label>
      )}
      {kind === "CpPng" && (
        <label>
          画像の大きさ(長辺の点数)
          <NumberStepper
            aria-label="画像の大きさ（長辺の点数）"
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
        <button
          ref={saveButtonRef}
          type="button"
          className="button-primary"
          disabled={busy || blocked(choice)}
          onClick={() => void handleSave()}
        >
          {busy ? "書き出しています…" : "保存先を選んで書き出す"}
        </button>
        <button ref={closeButtonRef} type="button" onClick={close}>
          閉じる
        </button>
      </div>
    </ModalDialog>
  );
}

export default ExportDialog;
