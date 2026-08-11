// 対称描画と基準線の指定。何も選んでいない紙設定と、線を選んだ操作欄で共用する。
// 状態はすべてZustandに置き、この部品は現在値をそのまま見せるだけにする。

import { mirrorAxisLabel, selectedEdgeMirrorLine } from "../lib/mirror";
import { useAppStore } from "../store/appStore";

export function MirrorAxisControls() {
  const doc = useAppStore((state) => state.doc);
  const selection = useAppStore((state) => state.selection);
  const mirrorDraw = useAppStore((state) => state.mirrorDraw);
  const mirrorAxis = useAppStore((state) => state.mirrorAxis);
  const setMirrorDraw = useAppStore((state) => state.setMirrorDraw);
  const setMirrorAxisPreset = useAppStore(
    (state) => state.setMirrorAxisPreset,
  );
  const setSelectedLineAsMirrorAxis = useAppStore(
    (state) => state.setSelectedLineAsMirrorAxis,
  );

  const selectedIds = [...new Set(selection.edgeIds)];
  const selectedEdge =
    doc && selectedIds.length === 1
      ? doc.cp.edges.find((edge) => edge.id === selectedIds[0])
      : undefined;
  const canUseSelected =
    doc !== null &&
    selection.vertexIds.length === 0 &&
    selectedIds.length === 1 &&
    selectedEdgeMirrorLine(doc, selectedIds[0]) !== null;
  const selectedReason = canUseSelected
    ? "展開図で選んだ折り線・補助線を基準にします"
    : selectedIds.length > 1 || selection.vertexIds.length > 0
      ? "基準にする折り線または補助線を1本だけ選んでください"
      : selectedEdge?.kind === "Border"
        ? "紙の輪郭は基準にできません。折り線または補助線を選んでください"
        : "展開図で折り線または補助線を1本選ぶと使えます";
  const current = mirrorAxisLabel(mirrorAxis);

  return (
    <section className="mirror-draw-controls" aria-label="対称描画の設定">
      <label className="mirror-draw-toggle">
        <input
          type="checkbox"
          aria-label="左右対称に描く"
          data-tooltip={`片側に線を引くだけで反対側にも同じ線を引き、消す・線種変更もそろえます。現在の基準: ${current}`}
          checked={mirrorDraw}
          onChange={(event) => setMirrorDraw(event.target.checked)}
        />
        左右対称に描く
      </label>
      <div className="mirror-axis-heading">
        <strong>基準にする線</strong>
        <span aria-live="polite">現在: {current}</span>
      </div>
      <div className="mirror-axis-buttons" role="group" aria-label="基準にする線">
        <button
          type="button"
          aria-pressed={mirrorAxis.kind === "paperVertical"}
          data-tooltip={`紙の縦の中心線を基準に左右へそろえます。現在の基準: ${current}`}
          onClick={() => setMirrorAxisPreset("paperVertical")}
        >
          紙の縦の中心線
        </button>
        <button
          type="button"
          aria-pressed={mirrorAxis.kind === "paperHorizontal"}
          data-tooltip={`紙の横の中心線を基準に上下へそろえます。現在の基準: ${current}`}
          onClick={() => setMirrorAxisPreset("paperHorizontal")}
        >
          紙の横の中心線
        </button>
        <span
          className="mirror-axis-disabled-tooltip"
          data-tooltip={`${selectedReason}。現在の基準: ${current}`}
          tabIndex={canUseSelected ? undefined : 0}
        >
          <button
            type="button"
            disabled={!canUseSelected}
            aria-pressed={mirrorAxis.kind === "selectedLine"}
            data-tooltip={`${selectedReason}。現在の基準: ${current}`}
            onClick={setSelectedLineAsMirrorAxis}
          >
            この線を基準にする
          </button>
        </span>
      </div>
    </section>
  );
}
