// 下部コンテキストパネル(160px)。選択状態に応じて内容を切り替える。
// 警告・エラーの詳細もここに表示する(常設パネルを増やさない)。

import { useEffect, useRef } from "react";
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

/**
 * 角度の数値入力。入力途中の「−」だけ・空文字といった状態を打てるように、
 * 入力欄の表示は制御せず(値をストアで固定せず)、確定(Enter・入力欄から
 * 離れたとき)にストアへ反映する。表示専用の一時状態なのでrefで扱う。
 */
function AngleNumberInput({ hinge, value }: { hinge: number; value: number }) {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const setDriverAngle = useAppStore((s) => s.setDriverAngle);

  // 入力中でなければ、スライダー操作や計算結果に表示を追従させる
  useEffect(() => {
    const el = inputRef.current;
    if (el && document.activeElement !== el) el.value = String(value);
  }, [value]);

  const commit = () => {
    const el = inputRef.current;
    if (!el) return;
    const entered = Number(el.value);
    if (el.value.trim() === "" || !Number.isFinite(entered)) {
      el.value = String(value); // 数字になっていない入力は捨てて現在値へ戻す
      return;
    }
    const angle = clampAngle(Math.round(entered));
    el.value = String(angle);
    setDriverAngle(hinge, angle);
  };

  return (
    <input
      ref={inputRef}
      type="number"
      className="angle-number"
      min={ANGLE_MIN}
      max={ANGLE_MAX}
      step={1}
      defaultValue={value}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
      }}
    />
  );
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
      <AngleNumberInput key={hinge} hinge={hinge} value={value} />
      <span className="hint">
        度(+は山折り、−は谷折り、±180で完全に折る。数値はEnterで確定)
      </span>
      <button
        type="button"
        title="この折り線の角度指定をやめます(この線は平らに戻り、形は残りの指定から計算し直します)"
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
