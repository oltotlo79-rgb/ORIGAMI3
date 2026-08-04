// 下部コンテキストパネル(160px)。選択状態に応じて内容を切り替える。
// 警告・エラーの詳細もここに表示する(常設パネルを増やさない)。

import { useAppStore } from "../store/appStore";
import { hingeEdgeIds } from "../lib/hinges";
import type { EdgeKind } from "../lib/types";

const KIND_LABEL: Record<EdgeKind, string> = {
  Border: "輪郭",
  Mountain: "山折り",
  Valley: "谷折り",
  Aux: "補助線",
};

/** 角度の指定できる範囲(度)。+=山折り、−=谷折り、±180=完全に折る */
const ANGLE_MIN = -180;
const ANGLE_MAX = 180;

function clampAngle(deg: number): number {
  return Math.max(ANGLE_MIN, Math.min(ANGLE_MAX, deg));
}

/** 選択中の折り線1本の角度操作(スライダー+数値入力+解除) */
function HingeAngle({ hinge }: { hinge: number }) {
  const drivers = useAppStore((s) => s.drivers);
  const poseAngles = useAppStore((s) => s.poseAngles);
  const setDriverAngle = useAppStore((s) => s.setDriverAngle);
  const clearDriver = useAppStore((s) => s.clearDriver);

  // 指定値 → 計算結果 → 0度(平ら)の順に現在値を決める
  const specified = drivers.get(hinge);
  const value = Math.round(specified ?? poseAngles.get(hinge) ?? 0);

  return (
    <div className="angle-row">
      <label htmlFor="hinge-angle">折り角度</label>
      <input
        id="hinge-angle"
        type="range"
        min={ANGLE_MIN}
        max={ANGLE_MAX}
        step={1}
        value={value}
        onChange={(e) => setDriverAngle(hinge, Number(e.target.value))}
      />
      <input
        type="number"
        className="angle-number"
        min={ANGLE_MIN}
        max={ANGLE_MAX}
        step={1}
        value={value}
        onChange={(e) => {
          const v = Number(e.target.value);
          if (e.target.value !== "" && Number.isFinite(v)) {
            setDriverAngle(hinge, clampAngle(v));
          }
        }}
      />
      <span className="hint">度(+は山折り、−は谷折り、±180で完全に折る)</span>
      <button
        type="button"
        title="この折り線の角度指定をやめます(形は他の折り線に合わせて計算されます)"
        disabled={specified === undefined}
        onClick={() => clearDriver(hinge)}
      >
        この折り線の角度を解除
      </button>
    </div>
  );
}

/** 折り角度の操作(折り線を1本だけ選んでいるとき)と、全解除ボタン */
function FoldControls() {
  const doc = useAppStore((s) => s.doc);
  const faces = useAppStore((s) => s.faces);
  const selection = useAppStore((s) => s.selection);
  const drivers = useAppStore((s) => s.drivers);
  const clearDrivers = useAppStore((s) => s.clearDrivers);

  const selected =
    selection.edgeIds.length === 1 && selection.vertexIds.length === 0
      ? selection.edgeIds[0]
      : null;
  // 折り線(山折り・谷折りで、両側に面がある辺)だけが角度を指定できる
  const hinge =
    doc !== null && selected !== null && hingeEdgeIds(doc, faces).has(selected)
      ? selected
      : null;

  if (hinge === null && drivers.size === 0) return null;

  return (
    <div className="fold-controls">
      {hinge !== null && <HingeAngle hinge={hinge} />}
      {drivers.size > 0 && (
        <div className="button-row">
          <button type="button" onClick={() => clearDrivers()}>
            全て平らに戻す
          </button>
        </div>
      )}
    </div>
  );
}

function SelectionContent() {
  const doc = useAppStore((s) => s.doc);
  const selection = useAppStore((s) => s.selection);
  const applyEdit = useAppStore((s) => s.applyEdit);

  if (!doc) return <p>読み込み中…</p>;

  if (selection.edgeIds.length > 0) {
    const edges = doc.cp.edges.filter((e) => selection.edgeIds.includes(e.id));
    const kinds = [...new Set(edges.map((e) => KIND_LABEL[e.kind]))].join("・");
    const setKind = (kind: EdgeKind) =>
      applyEdit({ type: "SetEdgeKind", ids: selection.edgeIds, kind });
    return (
      <div>
        <p>
          線を{edges.length}本選択中(種類: {kinds})
        </p>
        <div className="button-row">
          <button type="button" onClick={() => setKind("Mountain")}>
            山折りにする
          </button>
          <button type="button" onClick={() => setKind("Valley")}>
            谷折りにする
          </button>
          <button type="button" onClick={() => setKind("Aux")}>
            補助線にする
          </button>
          <button
            type="button"
            onClick={() =>
              applyEdit({ type: "RemoveEdges", ids: selection.edgeIds })
            }
          >
            削除
          </button>
        </div>
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
      </div>
    );
  }

  return (
    <p className="hint">
      左のツールを選んで操作します。山折り・谷折り・補助線: 2回クリックで線を引く(Escで中止)/
      選択: クリックまたはドラッグで選ぶ / Deleteキー: 選択した線を削除
    </p>
  );
}

export function ContextPanel() {
  const warnings = useAppStore((s) => s.warnings);
  const poseWarnings = useAppStore((s) => s.poseWarnings);
  const errorMessage = useAppStore((s) => s.errorMessage);
  const allWarnings = [...warnings, ...poseWarnings];

  return (
    <footer className="context-panel">
      <div className="context-selection">
        <SelectionContent />
        <FoldControls />
      </div>
      {(errorMessage !== null || allWarnings.length > 0) && (
        <div className="context-messages">
          {errorMessage !== null && <p className="error-text">{errorMessage}</p>}
          {allWarnings.map((w, i) => (
            <p key={i} className="warning-text">
              {w}
            </p>
          ))}
        </div>
      )}
    </footer>
  );
}
