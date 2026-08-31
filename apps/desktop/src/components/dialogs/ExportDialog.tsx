// 書き出しダイアログ(EXP-001 / EXP-002、Task 4-3)。
// 展開図と折り図をファイルとして保存する。入口は上部ツールバーのボタン1つだけで、
// 常設4区画は増やさない。「ラスタライズ」「dpi」などの用語は出さず、
// どちらを選ぶと何ができるかを日本語で書く(設計原則3b)。

import { useRef } from "react";
import {
  FOLD_UNSUPPORTED_CONTENT_ITEMS,
  FOLD_UNSUPPORTED_CONTENT_TITLE,
  foldIssueNotice,
  type FoldIssueNoticeInput,
} from "../../lib/foldNotices";
import {
  getPlatformFileGateway,
  platformFileErrorMessage,
} from "../../platform/fileGateway";
import { useAppStore } from "../../store/appStore";
import { fileName } from "../RecoveryDialog";
import { NumberStepper } from "../NumberStepper";
import { ModalDialog } from "./ModalDialog";
import { EXPORT_CHOICES, EXPORT_DIALOG_TITLE } from "./exportChoices";

// 既存の画面・検査のimport先を保つ。choices本体だけはAppでも使えるpure moduleへ置く。
export { EXPORT_CHOICES } from "./exportChoices";

/** 折り手順がまだ無いときに、その種類を選べない理由(日本語) */
export const NO_STEPS_REASON =
  "折り手順がまだありません。紙を折って手順を作ると折り図を書き出せます。";

export const EXPORT_FOLD_ISSUE_CONFIRMATION =
  "書き出した内容について、次の点をご確認ください。元の作品は変更されていません。";

const FOLD_EXPORT_FAILURE_NOTICE =
  "ほかの折り紙ソフトのファイルを書き出せませんでした。作品の内容と保存先を確認してください。";

/**
 * 書き出し結果の注意を、raw値を使わない安全な日本語だけで全件表示する。
 * storeには原情報を保ち、この表示境界では閉じた文言表だけを使う。
 */
export function ExportFoldIssueNotices({
  issues,
}: {
  issues: readonly FoldIssueNoticeInput[];
}) {
  if (issues.length === 0) return null;

  return (
    <div className="hint" role="status" aria-live="polite">
      <p>
        ほかの折り紙ソフトのファイルを書き出しました（注意
        {issues.length}件）
      </p>
      <p>{EXPORT_FOLD_ISSUE_CONFIRMATION}</p>
      <ul>
        {issues.map((issue, index) => (
          <li key={index}>{foldIssueNotice(issue, "export")}</li>
        ))}
      </ul>
    </div>
  );
}

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
  const deliveryNotice = useAppStore((s) => s.exportDeliveryNotice);
  const foldIssues = useAppStore((s) => s.exportFoldIssues);
  const setOption = useAppStore((s) => s.setExportOption);
  const runExport = useAppStore((s) => s.runExport);
  const close = useAppStore((s) => s.closeExport);
  const stepCount = useAppStore((s) => s.doc?.sequence.length ?? 0);
  const fileGateway = getPlatformFileGateway();
  const downloadsInsteadOfChoosing = fileGateway.saveMode === "download";
  const downloadsThisChoice =
    downloadsInsteadOfChoosing ||
    (kind === "DiagramSvg" &&
      fileGateway.multipleFileSaveMode === "download");
  if (!open) return null;

  const choice = EXPORT_CHOICES.find((c) => c.kind === kind) ?? EXPORT_CHOICES[0];
  // 失敗の原情報はstoreへ残し、ほかのソフト用だけは画面境界で内部語を隠す。
  const safePlatformError = error?.endsWith("作品は変更されていません。") === true;
  const visibleError =
    kind === "FoldJson" && !safePlatformError
      ? FOLD_EXPORT_FAILURE_NOTICE
      : error;
  // 折り図は手順が要る。選べないときも選択肢は残し、理由を出す
  const blocked = (c: (typeof EXPORT_CHOICES)[number]) =>
    c.needsSteps === true && stepCount === 0;
  const initialChoice = blocked(choice)
    ? EXPORT_CHOICES.find((candidate) => !blocked(candidate))
    : choice;

  const handleSave = async () => {
    try {
      const path = await fileGateway.chooseSaveFile({
        filters: [{ name: choice.label, extensions: [choice.ext] }],
        suggestedName: `作品.${choice.ext}`,
        multipleFiles: kind === "DiagramSvg",
      });
      if (path !== null) {
        try {
          const exportTask = runExport(path);
          // 保存先を選んだ後、処理中は無効になる保存ボタンではなく、
          // いつでも使える既存の「閉じる」へ一時的に戻す。
          queueMicrotask(() =>
            closeButtonRef.current?.focus({ preventScroll: true }),
          );
          await exportTask;
        } finally {
          fileGateway.release(path);
        }
      }
    } catch (reason) {
      useAppStore.setState({
        exportError: platformFileErrorMessage(
          reason,
          downloadsThisChoice ? "download" : "save",
        ),
        exportSavedPath: null,
        exportFoldIssues: [],
      });
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
      <h2 id="export-title">{EXPORT_DIALOG_TITLE}</h2>
      <fieldset>
        <legend>何を書き出しますか</legend>
        {EXPORT_CHOICES.map((c) => (
          <label key={c.kind}>
            <input
              ref={c.kind === initialChoice?.kind ? initialChoiceRef : undefined}
              type="radio"
              name="export-kind"
              checked={kind === c.kind}
              disabled={(busy && kind !== c.kind) || blocked(c)}
              onChange={() => setOption({ exportKind: c.kind })}
            />
            {c.label}
          </label>
        ))}
      </fieldset>
      <p className="hint">{choice.hint}</p>
      {downloadsThisChoice && (
        <p className="hint">
          {kind === "DiagramSvg"
            ? "このブラウザでは複数ファイルの保存先を選べないため、折り図SVGをZIPでダウンロードします。"
            : "このブラウザでは保存先を選べないため、ファイルをダウンロードします。"}
        </p>
      )}
      {kind === "FoldJson" && (
        <section
          className="hint"
          aria-labelledby="fold-unsupported-content-title"
        >
          <p id="fold-unsupported-content-title">
            {FOLD_UNSUPPORTED_CONTENT_TITLE}
          </p>
          <ul>
            {FOLD_UNSUPPORTED_CONTENT_ITEMS.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
        </section>
      )}
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
      {deliveryNotice !== null && (
        <p className="hint" aria-live="polite">
          {deliveryNotice}
        </p>
      )}
      {savedPath && deliveryNotice === null && (
        <p className="hint">
          {downloadsThisChoice ? "ダウンロードを開始しました" : "保存しました"}:
          <span className="user-text">{fileName(savedPath)}</span>
        </p>
      )}
      <ExportFoldIssueNotices issues={foldIssues} />
      {error && (
        <p className="error-text" role="alert">
          {downloadsThisChoice
            ? "ダウンロードできませんでした"
            : "保存できませんでした"}
          :{visibleError}
        </p>
      )}
      <div className="button-row">
        <button
          ref={saveButtonRef}
          type="button"
          className="button-primary"
          disabled={busy || blocked(choice)}
          onClick={() => void handleSave()}
        >
          {busy
            ? downloadsThisChoice
              ? "ダウンロードの準備中…"
              : "書き出しています…"
            : downloadsThisChoice
              ? "ダウンロード"
              : "保存先を選んで書き出す"}
        </button>
        <button ref={closeButtonRef} type="button" onClick={close}>
          閉じる
        </button>
      </div>
    </ModalDialog>
  );
}

export default ExportDialog;
