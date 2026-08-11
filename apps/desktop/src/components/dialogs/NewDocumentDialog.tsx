// 新規作成ダイアログ(PAP-001)。紙の形(正方形/長方形)と大きさ(mm)を決める。
// 入口は上部ツールバーの「新規」だけで、常設4区画は増やさない。
// 決めた形はその場で見本の四角に映す(結果をプレビューで見せる。設計原則3b)。

import { draftToPaper, useAppStore } from "../../store/appStore";

/** よく使う紙の大きさ(mm)。押すとその大きさが入る */
export const PAPER_PRESETS: { label: string; width: number; height: number }[] = [
  { label: "折り紙 15cm角", width: 150, height: 150 },
  { label: "折り紙 24cm角", width: 240, height: 240 },
  { label: "A4の紙", width: 297, height: 210 },
];

/** 見本の四角の長辺(px) */
const PREVIEW_LONG_PX = 96;

export function NewDocumentDialog() {
  const open = useAppStore((s) => s.newDialogOpen);
  const draft = useAppStore((s) => s.newPaperDraft);
  const setDraft = useAppStore((s) => s.setNewPaperDraft);
  const confirm = useAppStore((s) => s.confirmNewDocument);
  const close = useAppStore((s) => s.closeNewDialog);
  if (!open) return null;

  const paper = draftToPaper(draft);
  const long = Math.max(paper.width_mm, paper.height_mm);
  const valid = paper.width_mm > 0 && paper.height_mm > 0 && long > 0;
  const size = (mm: number) => (valid ? (mm / long) * PREVIEW_LONG_PX : 0);

  return (
    <div className="dialog-backdrop">
      <div
        className="dialog"
        data-floating-ui="new-document-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="new-title"
      >
        <h2 id="new-title">新しい紙を用意する</h2>
        <fieldset>
          <legend>紙の形</legend>
          <label>
            <input
              type="radio"
              name="paper-shape"
              data-tooltip="正方形の紙を選びます"
              checked={draft.square}
              onChange={() => setDraft({ square: true })}
            />
            正方形(たて・よこが同じ)
          </label>
          <label>
            <input
              type="radio"
              name="paper-shape"
              data-tooltip="長方形の紙を選びます"
              checked={!draft.square}
              onChange={() => setDraft({ square: false })}
            />
            長方形(たて・よこを別に決める)
          </label>
        </fieldset>
        <div className="new-paper-row">
          <label>
            よこ(mm)
            <input
              type="number"
              min={1}
              max={2000}
              data-tooltip="紙の横の長さをmmで入力します"
              value={draft.widthMm}
              onChange={(e) => setDraft({ widthMm: Number(e.target.value) })}
            />
          </label>
          <label>
            たて(mm)
            <input
              type="number"
              min={1}
              max={2000}
              value={draft.square ? draft.widthMm : draft.heightMm}
              disabled={draft.square}
              data-tooltip={
                draft.square
                  ? "正方形なので、横と同じ長さになります"
                  : "紙の縦の長さをmmで入力します"
              }
              onChange={(e) => setDraft({ heightMm: Number(e.target.value) })}
            />
          </label>
          <span
            className="new-paper-preview"
            aria-hidden="true"
            style={{ width: size(paper.width_mm), height: size(paper.height_mm) }}
          />
        </div>
        <div className="button-row">
          {PAPER_PRESETS.map((p) => (
            <button
              key={p.label}
              type="button"
              data-tooltip={`${p.label}の大きさを使います`}
              onClick={() =>
                setDraft({
                  widthMm: p.width,
                  heightMm: p.height,
                  square: p.width === p.height,
                })
              }
            >
              {p.label}
            </button>
          ))}
        </div>
        {!valid && (
          <p className="error-text">大きさは0より大きいmmで入れてください</p>
        )}
        <div className="button-row">
          <button
            type="button"
            className="button-primary"
            disabled={!valid}
            data-tooltip="入力した大きさで新しい作品を始めます"
            onClick={() => void confirm()}
          >
            この紙で作りはじめる
          </button>
          <button type="button" data-tooltip="新規作成をやめます" onClick={close}>
            やめる
          </button>
        </div>
      </div>
    </div>
  );
}
