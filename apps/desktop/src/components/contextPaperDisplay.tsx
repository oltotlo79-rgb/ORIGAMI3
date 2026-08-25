import { useEffect, useRef } from "react";
import {
  useAppStore,
  type ToolId,
} from "../store/appStore";
import {
  CURVE_LABEL,
  MAX_CURVE_SEGMENTS,
  type CurveShape,
} from "../lib/curve";
import type { EdgeKind } from "../lib/types";
import { PaperAppearance } from "./PaperAppearance";
import { MirrorAxisControls } from "./MirrorAxisControls";
import { mirrorAxisLabel } from "../lib/mirror";
import { NumberInput } from "./contextAngleSteps";

const KIND_LABEL: Record<EdgeKind, string> = {
  Border: "輪郭",
  Mountain: "山折り",
  Valley: "谷折り",
  Aux: "補助線",
};

/** 線を引くツール(曲線モードの切り替えを出す対象) */
export const LINE_TOOLS: ToolId[] = ["mountain", "valley", "aux"];

/**
 * 「引く」ツールを選んでいるときの内容(UI-007)。
 * 左右同時に動かすかの切替をここに置く(ツールレールも常設区画も増やさない)。
 * 折り紙の作品はほとんどが左右対称なので既定はオン。片方だけ形を変えたいとき
 * (くちばしの角度を少しだけ変える等)に切れるようにしてある。
 */
export function PullContent() {
  const pullMirror = useAppStore((s) => s.pullMirror);
  const setPullMirror = useAppStore((s) => s.setPullMirror);
  const mirrorAxis = useAppStore((s) => s.mirrorAxis);
  const drawingAxis = mirrorAxisLabel(mirrorAxis);

  return (
    <div>
      <div className="button-row">
        <label>
          <input
            type="checkbox"
            aria-label="左右対称に動かす"
            data-tooltip={
              pullMirror
                ? `動かすときは展開図から対になる折り目を自動で見つけ、反対側も同じ角度で動かします。線をそろえる現在の基準: ${drawingAxis}`
                : `つかんだ側の折り目だけを動かします。線をそろえる現在の基準: ${drawingAxis}`
            }
            checked={pullMirror}
            onChange={(e) => setPullMirror(e.target.checked)}
          />
          左右対称に動かす
        </label>
      </div>
      <PaperActionEntrances showPull={false} />
    </div>
  );
}

/**
 * 紙の形を直接変える3つの入口。ツール名だけでは結果を想像しにくいので、
 * 「何が起きるか」を動詞で並べる。膨らみは設定を開くだけで、強さは利用者が
 * 下のつまみを動かして決める(勝手に作品の形を変えない)。
 */
function PaperActionEntrances({ showPull = true }: { showPull?: boolean }) {
  const setTool = useAppStore((s) => s.setTool);
  const setSelection = useAppStore((s) => s.setSelection);
  const setSoft = useAppStore((s) => s.setSoft);
  const hingeCount = useAppStore((s) => s.hinges.size);
  const enterFoldAllPreview = useAppStore((s) => s.enterFoldAllPreview);

  const showInflate = () => {
    setTool("select");
    setSelection({ edgeIds: [], vertexIds: [] });
    setSoft({ soft_enabled: true });
  };

  return (
    <div className="paper-action-entrances" aria-label="紙の形を変える">
      <span className="paper-action-entrances-title row-label">紙の形を変える</span>
      {showPull && (
        <button
          type="button"
          data-tooltip="3Dの紙を引き、折り目を連動させます"
          onClick={() => setTool("pull")}
        >
          ↔ 紙を引いて動かす
        </button>
      )}
      <button
        type="button"
        data-tooltip="紙へ丸みと膨らみを付ける設定を開きます"
        onClick={showInflate}
      >
        ◯ 紙をふくらませる
      </button>
      <button
        type="button"
        disabled={hingeCount === 0}
        data-tooltip={
          hingeCount === 0
            ? "山折りか谷折りを引くと、全部の折り目をいっぺんに動かせます"
            : "山折りと谷折りを同じ割合で動かし、手順とは別に形を見ます"
        }
        onClick={() => void enterFoldAllPreview()}
      >
        ◇ 全部いっぺんに折ってみる
      </button>
    </div>
  );
}

/** 全折り目を同じ割合で動かして形だけを見る、一時表示の操作欄。 */
export function FoldAllPreviewContent() {
  const preview = useAppStore((s) => s.foldAllPreview);
  const setFoldAllPercent = useAppStore((s) => s.setFoldAllPercent);
  const finishFoldAllPercent = useAppStore((s) => s.finishFoldAllPercent);
  const leaveFoldAllPreview = useAppStore((s) => s.leaveFoldAllPreview);
  const documentSavedPath = useAppStore((s) => s.documentSavedPath);
  const otherOperationFailed = useAppStore((s) => s.errorMessage !== null);
  const percentChangePending = useRef(false);
  useEffect(() => {
    percentChangePending.current = false;
  }, [preview?.session]);

  if (preview === null) return null;
  const percentText = Number.isInteger(preview.percent)
    ? preview.percent.toFixed(0)
    : preview.percent.toFixed(1);

  return (
    <section
      className="fold-all-preview"
      aria-label="全部いっぺんに折ってみる"
      data-fold-all-active="true"
      data-applied-percent={preview.appliedPercent ?? ""}
      data-returning={preview.returning ? "true" : "false"}
    >
      <div className="fold-all-preview-heading">
        <strong>全部いっぺんに折ってみる</strong>
        <strong className="fold-all-preview-promise">
          これは仮の形です
        </strong>
        <span className="hint">手順には記録されません。</span>
        {preview.layerOrder === "unavailable_without_sequence" && (
          <span className="hint fold-all-layer-order-note">
            紙を順番に折った形ではないため、どの紙が上になるかは決まっていません。
          </span>
        )}
      </div>

      <div className="fold-all-preview-control">
        <label htmlFor="fold-all-percent">折る割合</label>
        <output htmlFor="fold-all-percent">{percentText}%</output>
        <input
          id="fold-all-percent"
          type="range"
          min={0}
          max={100}
          step={1}
          value={preview.percent}
          disabled={preview.returning}
          aria-label="全部の折り目を動かす割合"
          aria-valuetext={`${percentText}%`}
          onChange={(event) => {
            percentChangePending.current = true;
            setFoldAllPercent(Number(event.target.value));
          }}
          onPointerUp={() => {
            percentChangePending.current = false;
            finishFoldAllPercent();
          }}
          onKeyUp={(event) => {
            if (
              event.key === "ArrowLeft" ||
              event.key === "ArrowRight" ||
              event.key === "ArrowUp" ||
              event.key === "ArrowDown" ||
              event.key === "PageUp" ||
              event.key === "PageDown" ||
              event.key === "Home" ||
              event.key === "End"
            ) {
              percentChangePending.current = false;
              finishFoldAllPercent();
            }
          }}
          onBlur={() => {
            // 開いた直後からTabで離れただけなら、変更は保留されていない。
            // 読み上げ操作などでchangeだけ届いた場合は、blurで確定する。
            if (!percentChangePending.current) return;
            percentChangePending.current = false;
            finishFoldAllPercent();
          }}
        />
        <span className="fold-all-preview-min">元に戻る 0%</span>
        <span className="fold-all-preview-max">できるところまで 100%</span>
      </div>

      <div className="button-row">
        <button
          type="button"
          disabled={preview.returning}
          onClick={() => void leaveFoldAllPreview()}
        >
          いつもの表示に戻る
        </button>
      </div>

      <div className="fold-all-preview-notices" aria-live="polite">
        {preview.returning ? (
          <p className="hint">いつもの表示に戻しています…</p>
        ) : (
          preview.busy && <p className="hint">形を動かしています…</p>
        )}
        {preview.converged === false && (
          <p className="warning-text">
            形を最後まで合わせきれませんでした。いちばん近い形を表示しています。
          </p>
        )}
        {preview.relaxationCount > 0 && (
          <p className="warning-text">
            全部の折り目を同じ割合にできないため、いちばん近い形を表示しています。
          </p>
        )}
        {preview.flatFoldViolationCount > 0 && (
          <p className="warning-text">
            平らにたためない場所があります。このまま形を見ることはできます。
          </p>
        )}
        {preview.suspectHingeCount > 0 && (
          <p className="warning-text">
            紙が突き抜けているところがあります。このまま形を見ることはできます。
          </p>
        )}
        {preview.contactDetected && preview.suspectHingeCount === 0 && (
          <p className="warning-text">
            紙どうしが触れているところがあります。このまま形を見ることはできます。
          </p>
        )}
        {preview.error !== null && (
          <p className="warning-text">{preview.error}</p>
        )}
        {documentSavedPath !== null && otherOperationFailed === false && (
          <p className="hint">
            作品を保存しました。いま見ている形は保存されません。
          </p>
        )}
        {otherOperationFailed && (
          <p className="warning-text">
            操作を終えられませんでした。いま見ている形はそのままです。
          </p>
        )}
      </div>
    </section>
  );
}

/**
 * 曲線の折り目(CPE-011)の設定。山折り・谷折り・補助線ツールのときだけ出す。
 * ツールレールは10個で上限なので曲線用のツールは増やさず、既存の線ツールの
 * 「直線/曲線」の切り替えとしてここに置く(線を引く操作の設定は1か所にまとまる)。
 */
export function CurveRow() {
  const curve = useAppStore((s) => s.curve);
  const setCurve = useAppStore((s) => s.setCurve);
  const shapes: CurveShape[] = ["arc", "bezier"];

  return (
    <div className="button-row">
      <label>
        <input
          type="checkbox"
          aria-label="曲線で描く"
          data-tooltip="曲線の折り目を細かな折れ線として引きます"
          checked={curve.enabled}
          onChange={(e) => setCurve({ enabled: e.target.checked })}
        />
        曲線で描く
      </label>
      {curve.enabled && (
        <>
          <label htmlFor="curve-shape">描き方</label>
          <select
            id="curve-shape"
            value={curve.shape}
            onChange={(e) => setCurve({ shape: e.target.value as CurveShape })}
          >
            {shapes.map((s) => (
              <option key={s} value={s}>
                {CURVE_LABEL[s]}
                {s === "arc" ? "(3点)" : "(4点・S字も可)"}
              </option>
            ))}
          </select>
          <label>
            <input
              type="checkbox"
              aria-label="分割の細かさを自分で決める"
              data-tooltip="曲線を何本の短い線へ分けるか自分で指定します"
              checked={curve.segments !== null}
              onChange={(e) => setCurve({ segments: e.target.checked ? 16 : null })}
            />
            分割数を指定
          </label>
          {curve.segments !== null && (
            <NumberInput
              id="curve-segments"
              ariaLabel="曲線の分割数"
              value={curve.segments}
              min={1}
              max={MAX_CURVE_SEGMENTS}
              onPreview={(v) => setCurve({ segments: v })}
              normalizeOnCommit={(v) =>
                Math.min(MAX_CURVE_SEGMENTS, Math.round(v))
              }
              onCommit={(v) => setCurve({ segments: v })}
            />
          )}
          <label>
            <input
              type="checkbox"
              aria-label="紙が曲がるための線も引く"
              data-tooltip={
                curve.rulings
                  ? "曲線の両側へ、紙が滑らかに曲がるための線も引きます"
                  : "折り線だけを引きます。このままでは3Dで曲線折りできません"
              }
              checked={curve.rulings}
              onChange={(e) => setCurve({ rulings: e.target.checked })}
            />
            曲がるための線も引く
          </label>
        </>
      )}
    </div>
  );
}

export function SelectionContent() {
  const doc = useAppStore((s) => s.doc);
  const selection = useAppStore((s) => s.selection);
  const applyEdit = useAppStore((s) => s.applyEdit);
  const wheelBehavior = useAppStore((s) => s.wheelBehavior);
  const contextHelpExpanded = useAppStore((s) => s.contextHelpExpanded);

  if (!doc) return <p>読み込み中…</p>;

  if (selection.edgeIds.length > 0) {
    const edges = doc.cp.edges.filter((e) => selection.edgeIds.includes(e.id));
    const kinds = [...new Set(edges.map((e) => KIND_LABEL[e.kind]))].join("・");
    const includesPaperEdge = edges.some((e) => e.kind === "Border");
    const setKind = (kind: EdgeKind) =>
      applyEdit({ type: "SetEdgeKind", ids: selection.edgeIds, kind });
    return (
      <div>
        <p>
          線を{edges.length}本選択中(種類: {kinds})
        </p>
        <div className="button-row">
          <button
            type="button"
            disabled={includesPaperEdge}
            data-tooltip={
              includesPaperEdge
                ? "紙のふちは紙そのものなので、山折りには変えられません"
                : "選んだ線を山折りに変えます"
            }
            onClick={() => setKind("Mountain")}
          >
            山折りにする
          </button>
          <button
            type="button"
            disabled={includesPaperEdge}
            data-tooltip={
              includesPaperEdge
                ? "紙のふちは紙そのものなので、谷折りには変えられません"
                : "選んだ線を谷折りに変えます"
            }
            onClick={() => setKind("Valley")}
          >
            谷折りにする
          </button>
          <button
            type="button"
            disabled={includesPaperEdge}
            data-tooltip={
              includesPaperEdge
                ? "紙のふちは紙そのものなので、補助線には変えられません"
                : "選んだ線を補助線に変えます"
            }
            onClick={() => setKind("Aux")}
          >
            補助線にする
          </button>
          <button
            type="button"
            disabled={includesPaperEdge}
            data-tooltip={
              includesPaperEdge
                ? "紙のふちは紙そのものなので、削除できません"
                : "選んだ線を削除します"
            }
            onClick={() =>
              applyEdit({ type: "RemoveEdges", ids: selection.edgeIds })
            }
          >
            削除
          </button>
        </div>
        <MirrorAxisControls />
      </div>
    );
  }

  if (selection.vertexIds.length > 0) {
    const vertices = doc.cp.vertices.filter((v) =>
      selection.vertexIds.includes(v.id),
    );
    return (
      <div>
        <p>点を{vertices.length}個選択中</p>
        <ul className="vertex-list">
          {vertices.map((v) => (
            <li key={v.id}>
              点{v.id}: ({v.pos[0].toFixed(3)}, {v.pos[1].toFixed(3)})
            </li>
          ))}
        </ul>
        {/* 点だけを選んでいる間も、線を選べない理由を吹き出しで確認できる。 */}
        <MirrorAxisControls />
      </div>
    );
  }

  return (
    <>
      <PaperActionEntrances />
      {contextHelpExpanded && (
        <p className="hint context-help-detail">
          山折り・谷折り・補助線は2回クリックで引き、Escで中止します。選択はクリック、Ctrl+クリックで追加・解除、ドラッグで矩形選択します。点はドラッグで動かせます。Deleteキーで選択した線を削除します。展開図はスペースキーを押しながらドラッグ、右ドラッグ、中ボタンドラッグのどれでも動かせます。{" "}
          {wheelBehavior === "scroll"
            ? "ホイールで上下、Shift+ホイールで左右、Ctrl+ホイールで拡大縮小します。"
            : "ホイールで拡大縮小、Ctrl+ホイールで上下、Ctrl+Shift+ホイールで左右へ動かします。"}
        </p>
      )}
      {/* 紙の色と方眼の数は、何も選んでいないときだけここに出す(PAP-003 / CPE-003) */}
      <PaperAppearance />
    </>
  );
}
