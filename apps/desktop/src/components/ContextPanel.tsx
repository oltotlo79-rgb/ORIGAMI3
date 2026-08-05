// 下部コンテキストパネル(160px)。選択状態に応じて内容を切り替える。
// 警告・エラーの詳細もここに表示する(常設パネルを増やさない)。

import { useEffect, useRef } from "react";
import {
  isStepSkipped,
  useAppStore,
  type FoldDraft,
  type TechniqueDraft,
} from "../store/appStore";
import {
  TECHNIQUE_KINDS,
  TECHNIQUE_LABEL,
  uniqueWarnings,
} from "../lib/techniques";
import type { EdgeKind, FoldStep, TechniqueKind } from "../lib/types";

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
 * 書き換えていない入力欄から離れただけでは角度を指定しない(選んだだけの
 * 折り線が勝手に指定済みになるのを防ぐ)。Escapeで書きかけを取り消す。
 */
function AngleNumberInput({ hinge, value }: { hinge: number; value: number }) {
  const inputRef = useRef<HTMLInputElement | null>(null);
  /** 利用者がこの入力欄を書き換えたか(未編集なら確定しない) */
  const editedRef = useRef(false);
  const setDriverAngle = useAppStore((s) => s.setDriverAngle);

  // 入力中でなければ、スライダー操作や計算結果に表示を追従させる
  useEffect(() => {
    const el = inputRef.current;
    if (el && document.activeElement !== el) el.value = String(value);
  }, [value]);

  /** 書きかけを捨てて現在値の表示に戻す */
  const revert = () => {
    const el = inputRef.current;
    if (el) el.value = String(value);
    editedRef.current = false;
  };

  const commit = () => {
    const el = inputRef.current;
    if (!el) return;
    if (!editedRef.current) return; // 書き換えていないので何もしない
    const entered = Number(el.value);
    if (el.value.trim() === "" || !Number.isFinite(entered)) {
      revert(); // 数字になっていない入力は捨てて現在値へ戻す
      return;
    }
    const angle = clampAngle(Math.round(entered));
    el.value = String(angle);
    editedRef.current = false;
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
      onChange={() => {
        editedRef.current = true;
      }}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          commit();
        } else if (e.key === "Escape") {
          revert();
          e.currentTarget.blur();
        }
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
  const hinges = useAppStore((s) => s.hinges);
  const selection = useAppStore((s) => s.selection);
  const drivers = useAppStore((s) => s.drivers);
  const clearDrivers = useAppStore((s) => s.clearDrivers);

  const selected =
    selection.edgeIds.length === 1 && selection.vertexIds.length === 0
      ? selection.edgeIds[0]
      : null;
  // 折り線(山折り・谷折りで、両側に面がある辺)だけが角度を指定できる
  // ヒンジ集合はストアが展開図の更新時に1度だけ導出したものを使う
  const hinge = selected !== null && hinges.has(selected) ? selected : null;

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

/**
 * 手順の注記の入力欄。書きかけの文字を打てるよう表示は制御せず(値をストアで
 * 固定せず)、確定(Enter・入力欄から離れたとき)にストアへ送る。
 * 表示専用の一時状態なのでrefで扱う(要件§2: 状態はストア1本)。
 */
function NoteInput({ step }: { step: FoldStep }) {
  const inputRef = useRef<HTMLInputElement | null>(null);
  /** 利用者がこの入力欄を書き換えたか(未編集なら送らない) */
  const editedRef = useRef(false);
  const applySequenceOp = useAppStore((s) => s.applySequenceOp);

  // 入力中でなければ、外からの変更(元に戻す等)に表示を追従させる
  useEffect(() => {
    const el = inputRef.current;
    if (el && document.activeElement !== el) el.value = step.note;
  }, [step.note]);

  const commit = () => {
    const el = inputRef.current;
    if (!el || !editedRef.current) return;
    editedRef.current = false;
    if (el.value === step.note) return;
    void applySequenceOp({
      type: "UpdateStep",
      step: { ...step, note: el.value },
    });
  };

  return (
    <input
      ref={inputRef}
      type="text"
      className="note-input"
      placeholder="この手順の覚え書き(Enterで確定)"
      defaultValue={step.note}
      onChange={() => {
        editedRef.current = true;
      }}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          commit();
        } else if (e.key === "Escape") {
          if (inputRef.current) inputRef.current.value = step.note;
          editedRef.current = false;
          e.currentTarget.blur();
        }
      }}
    />
  );
}

/** 手順を選んでいるときの内容: 技法の変更・注記・削除 */
function StepContent({ number }: { number: number }) {
  const doc = useAppStore((s) => s.doc);
  // 飛ばされたかどうかは作品全体の再生結果で決める(タイムラインの札と同じ判断)
  const skipped = useAppStore((s) => s.skipped);
  const replaySkipped = useAppStore((s) => s.replaySkipped);
  const applySequenceOp = useAppStore((s) => s.applySequenceOp);

  const step = doc?.sequence[number - 1];
  if (!step) return <p className="hint">この手順はもうありません</p>;

  const setKind = (kind: TechniqueKind) =>
    void applySequenceOp({ type: "UpdateStep", step: { ...step, kind } });

  return (
    <div>
      <p>
        手順{number}: {TECHNIQUE_LABEL[step.kind]}(折り線
        {step.drivers.length}本)
        {isStepSkipped({ skipped, replaySkipped }, step.id) && (
          <span className="error-text">
            {" "}
            ※折り線が見つからないため飛ばされています
          </span>
        )}
      </p>
      <div className="button-row">
        <label htmlFor="step-kind">折り方</label>
        <select
          id="step-kind"
          value={step.kind}
          onChange={(e) => setKind(e.target.value as TechniqueKind)}
        >
          {TECHNIQUE_KINDS.map((k) => (
            <option key={k} value={k}>
              {TECHNIQUE_LABEL[k]}
            </option>
          ))}
        </select>
        <NoteInput key={step.id} step={step} />
        <button
          type="button"
          title="この手順を手順一覧から取り除きます(展開図の折り線は残ります)"
          onClick={() =>
            void applySequenceOp({ type: "RemoveStep", id: step.id })
          }
        >
          この手順を削除
        </button>
      </div>
    </div>
  );
}

/** 引いた折り線の確定UI(向き・対象の層・動かす側を決めて折る) */
function FoldDraftContent({ draft }: { draft: FoldDraft }) {
  const paper = useAppStore((s) => s.doc?.paper ?? null);
  const updateFoldDraft = useAppStore((s) => s.updateFoldDraft);
  const cancelFoldDraft = useAppStore((s) => s.cancelFoldDraft);
  const commitFoldDraft = useAppStore((s) => s.commitFoldDraft);
  const [a, b] = draft.line;
  // 座標は「紙の長辺=1」に正規化された値なので、紙の寸法を掛けてmmで見せる
  const scale = paper ? Math.max(paper.width_mm, paper.height_mm) : 1;
  const mm = (v: number) => (v * scale).toFixed(1);

  return (
    <div>
      <p>
        折り線: ({mm(a[0])}, {mm(a[1])}) →({mm(b[0])}, {mm(b[1])}) mm
      </p>
      <div className="button-row">
        <span>向き</span>
        <label>
          <input
            type="radio"
            name="fold-direction"
            checked={draft.direction === "Up"}
            onChange={() => updateFoldDraft({ direction: "Up" })}
          />
          手前へ折る(谷)
        </label>
        <label>
          <input
            type="radio"
            name="fold-direction"
            checked={draft.direction === "Down"}
            onChange={() => updateFoldDraft({ direction: "Down" })}
          />
          向こうへ折る(山)
        </label>
        <span>対象の層</span>
        <label>
          <input
            type="radio"
            name="fold-target"
            checked={draft.target === "all"}
            onChange={() => updateFoldDraft({ target: "all" })}
          />
          全ての層
        </label>
        <label>
          <input
            type="radio"
            name="fold-target"
            checked={draft.target === "top"}
            onChange={() => updateFoldDraft({ target: "top" })}
          />
          いちばん上の1枚
        </label>
      </div>
      {/* 「左/右」は畳み平面の向きで決まるため、カメラを回すと画面の左右と
          食い違う。画面に合わせるにはカメラの向きが要るので、言葉では側を
          言い当てず、動く側は立体表示のハイライトで見てもらう */}
      <div className="button-row">
        <span>動かす側</span>
        <label>
          <input
            type="radio"
            name="fold-side"
            checked={draft.movingSide === "right"}
            onChange={() => updateFoldDraft({ movingSide: "right" })}
          />
          こちら側
        </label>
        <label>
          <input
            type="radio"
            name="fold-side"
            checked={draft.movingSide === "left"}
            onChange={() => updateFoldDraft({ movingSide: "left" })}
          />
          反対側
        </label>
        <span className="hint">
          (立体表示で黄色く光っている方が動きます。違う方を動かしたいときは
          もう一方を選んでください)
        </span>
      </div>
      <div className="button-row">
        <button type="button" onClick={() => void commitFoldDraft()}>
          折る
        </button>
        <button type="button" onClick={() => cancelFoldDraft()}>
          やめる
        </button>
      </div>
    </div>
  );
}

/**
 * 段の幅(mm)の入力欄。書きかけの文字を打てるよう表示は制御せず、確定
 * (Enter・入力欄から離れたとき)にストアへ送る(要件§2: 状態はストア1本)。
 */
function PleatWidthInput({ value }: { value: number }) {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const editedRef = useRef(false);
  const updateTechniqueDraft = useAppStore((s) => s.updateTechniqueDraft);

  useEffect(() => {
    const el = inputRef.current;
    if (el && document.activeElement !== el) el.value = String(value);
  }, [value]);

  const commit = () => {
    const el = inputRef.current;
    if (!el || !editedRef.current) return;
    editedRef.current = false;
    const entered = Number(el.value);
    if (!Number.isFinite(entered) || entered <= 0) {
      el.value = String(value); // 数字でない入力は捨てて現在値へ戻す
      return;
    }
    el.value = String(entered);
    updateTechniqueDraft({ widthMm: entered });
  };

  return (
    <input
      ref={inputRef}
      id="pleat-width"
      type="number"
      className="angle-number"
      min={0.1}
      step={1}
      defaultValue={value}
      onChange={() => {
        editedRef.current = true;
      }}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
      }}
    />
  );
}

/** 技法の確定UI(フラップ・折り線を選んでから適用する) */
function TechniqueDraftContent({ draft }: { draft: TechniqueDraft }) {
  const paper = useAppStore((s) => s.doc?.paper ?? null);
  const updateTechniqueDraft = useAppStore((s) => s.updateTechniqueDraft);
  const cancelTechnique = useAppStore((s) => s.cancelTechnique);
  const commitTechnique = useAppStore((s) => s.commitTechnique);
  const scale = paper ? Math.max(paper.width_mm, paper.height_mm) : 1;
  const mm = (v: number) => (v * scale).toFixed(1);
  const needsFlap = draft.kind !== "Pleat";
  const ready = draft.line !== null && (!needsFlap || draft.flap.length >= 2);

  return (
    <div>
      <p>
        {TECHNIQUE_LABEL[draft.kind]}: 層を{draft.flap.length}枚選択中
        {draft.line ? (
          <>
            {" "}
            / 折り線 ({mm(draft.line[0][0])}, {mm(draft.line[0][1])}) →(
            {mm(draft.line[1][0])}, {mm(draft.line[1][1])}) mm
          </>
        ) : (
          " / 折り線はまだ引かれていません"
        )}
      </p>
      <div className="button-row">
        <span>{needsFlap ? "先端が向かう側" : "段になる側"}</span>
        <label>
          <input
            type="radio"
            name="technique-side"
            checked={draft.movingSide === "right"}
            onChange={() => updateTechniqueDraft({ movingSide: "right" })}
          />
          こちら側
        </label>
        <label>
          <input
            type="radio"
            name="technique-side"
            checked={draft.movingSide === "left"}
            onChange={() => updateTechniqueDraft({ movingSide: "left" })}
          />
          反対側
        </label>
        {draft.kind === "Pleat" && (
          <>
            <label htmlFor="pleat-width">段の幅(mm)</label>
            <PleatWidthInput value={draft.widthMm} />
          </>
        )}
      </div>
      <div className="button-row">
        <button
          type="button"
          disabled={!ready}
          title={
            ready
              ? "選んだ技法で折ります"
              : needsFlap
                ? "立体表示で紙をクリックして層を選び、折り線をドラッグしてください"
                : "立体表示で折り線をドラッグしてください"
          }
          onClick={() => void commitTechnique()}
        >
          適用
        </button>
        <button type="button" onClick={() => cancelTechnique()}>
          やめる
        </button>
        <span className="hint">
          立体表示で紙をクリックすると、その場所に重なっている層をまとめて選びます。
          そのままドラッグすると折り線を引けます(黄色く光っている層が対象です)
        </span>
      </div>
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
  const replayWarnings = useAppStore((s) => s.replayWarnings);
  const errorMessage = useAppStore((s) => s.errorMessage);
  const currentStep = useAppStore((s) => s.currentStep);
  const foldDraft = useAppStore((s) => s.foldDraft);
  const techniqueDraft = useAppStore((s) => s.techniqueDraft);
  // 同じ文言は1回だけ出す(展開図の検査結果には自動再生の警告も合流している)
  const allWarnings = uniqueWarnings(warnings, poseWarnings, replayWarnings);
  // 手順を選んでいる間はその手順の設定を出す(「折る前」「最新」は選択なし扱い)
  const stepSelected = currentStep !== null && currentStep >= 1;

  return (
    <footer className="context-panel">
      <div className="context-selection">
        {/* 手順を選んでいるときはその設定を優先する。折り線は「今見えている形」の
            上に引くものなので、手順を選んだ時点でストアが捨てている(ここは念のため) */}
        {stepSelected ? (
          <StepContent number={currentStep} />
        ) : techniqueDraft ? (
          <TechniqueDraftContent draft={techniqueDraft} />
        ) : foldDraft ? (
          <FoldDraftContent draft={foldDraft} />
        ) : (
          <>
            <SelectionContent />
            <FoldControls />
          </>
        )}
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
