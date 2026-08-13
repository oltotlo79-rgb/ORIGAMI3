// 下部コンテキストパネル。高さは上端の取っ手で変えられ、選択状態に応じて
// 内容を切り替える。
// 警告・エラーの詳細もここに表示する(常設パネルを増やさない)。

import { useEffect, useRef } from "react";
import {
  isStepSkipped,
  nextAlignKind,
  poseRecordReason,
  relaxationNotices,
  useAppStore,
  type AlignDraft,
  type FoldDraft,
  type PendingFoldThrough,
  type TechniqueDraft,
  type ToolId,
} from "../store/appStore";
import {
  CURVE_LABEL,
  MAX_CURVE_SEGMENTS,
  type CurveShape,
} from "../lib/curve";
import {
  ALIGN_HINTS,
  ALIGN_LABELS,
  ALIGN_STEPS,
  type AlignMode,
} from "../lib/alignFold";
import {
  TECHNIQUE_KINDS,
  TECHNIQUE_LABEL,
  uniqueWarnings,
} from "../lib/techniques";
import { flatFoldNotice } from "../lib/flatFoldNotice";
import { isTwistPolygonReady } from "../lib/twistPolygon";
import {
  clampTechniqueLayerCount,
  minimumTechniqueFlap,
  techniqueUsesOpenToBack,
} from "../lib/techniqueLayers";
import {
  buildLayerMotionPart,
  describeLayerMotionPart,
  hasLayerMotionInput,
  type LayerTurnMode,
} from "../lib/layerMotion";
import type { EdgeKind, FoldStep, TechniqueKind } from "../lib/types";
import { PaperAppearance } from "./PaperAppearance";
import { MirrorAxisControls } from "./MirrorAxisControls";
import { mirrorAxisLabel } from "../lib/mirror";
import { OperationSteps } from "./OperationSteps";
import { NumberStepper } from "./NumberStepper";

const KIND_LABEL: Record<EdgeKind, string> = {
  Border: "輪郭",
  Mountain: "山折り",
  Valley: "谷折り",
  Aux: "補助線",
};

/** 線を引くツール(曲線モードの切り替えを出す対象) */
const LINE_TOOLS: ToolId[] = ["mountain", "valley", "aux"];

/** 角度の指定できる範囲(度)。+=山折り、−=谷折り、±180=完全に折る */
const ANGLE_MIN = -180;
const ANGLE_MAX = 180;

function clampAngle(deg: number): number {
  return Math.max(ANGLE_MIN, Math.min(ANGLE_MAX, deg));
}

/**
 * 入力を終えた数値だけを返す。Number("12.")のような書きかけも数値へ
 * 変換できてしまうため、先に文字列の形を調べて入力途中と区別する。
 */
function completeNumber(raw: string): number | null {
  const text = raw.trim();
  if (!/^[+-]?(?:\d+(?:\.\d+)?|\.\d+)(?:[eE][+-]?\d+)?$/.test(text)) {
    return null;
  }
  const value = Number(text);
  return Number.isFinite(value) ? value : null;
}

/**
 * 角度の数値入力。入力途中の「−」だけ・空文字といった状態を打てるように、
 * 入力欄の表示は制御せず(値をストアで固定せず)、完全で範囲内の数値だけを
 * 入力中からストアへ反映する。最終確定はEnter・入力欄から離れたときに行う。
 * 表示専用の一時状態なのでrefで扱う。
 * 書き換えていない入力欄から離れただけでは角度を指定しない(選んだだけの
 * 折り線が勝手に指定済みになるのを防ぐ)。Escapeで書きかけを取り消す。
 */
function AngleNumberInput({
  value,
  ariaLabel,
  onValue,
  onFinish,
}: {
  value: number;
  ariaLabel: string;
  onValue: (value: number) => void;
  onFinish: () => void;
}) {
  const inputRef = useRef<HTMLInputElement | null>(null);
  /** 利用者がこの入力欄を書き換えたか(未編集なら確定しない) */
  const editedRef = useRef(false);

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
    const entered = completeNumber(el.value);
    if (entered === null) {
      revert(); // 数字になっていない入力は捨てて現在値へ戻す
      return;
    }
    const angle = clampAngle(Math.round(entered));
    el.value = String(angle);
    editedRef.current = false;
    onValue(angle);
  };

  return (
    <NumberStepper
      ref={inputRef}
      className="angle-number"
      aria-label={ariaLabel}
      data-tooltip={`${ariaLabel}を-180°から180°で指定します`}
      min={ANGLE_MIN}
      max={ANGLE_MAX}
      step={1}
      defaultValue={value}
      onChange={(e) => {
        editedRef.current = true;
        const entered = completeNumber(e.currentTarget.value);
        if (entered !== null && entered >= ANGLE_MIN && entered <= ANGLE_MAX) {
          // スライダーと同じストア操作へ送り、16ms間引きの3D追従に乗せる
          onValue(entered);
        }
      }}
      onStepComplete={() => {
        editedRef.current = false;
        onFinish();
      }}
      onBlur={() => {
        commit();
        onFinish();
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          commit();
          onFinish();
        } else if (e.key === "Escape") {
          revert();
          e.currentTarget.blur();
        }
      }}
    />
  );
}

/** 選択中の折り線1本の角度操作(スライダー+数値入力+解除) */
function HingeAngle({ hinge, only }: { hinge: number; only: boolean }) {
  const drivers = useAppStore((s) => s.drivers);
  const poseAngles = useAppStore((s) => s.poseAngles);
  const sequenceTargets = useAppStore((s) => s.sequenceTargets);
  const relaxations = useAppStore((s) => s.relaxations);
  const setDriverAngle = useAppStore((s) => s.setDriverAngle);
  const clearDriver = useAppStore((s) => s.clearDriver);
  const setHoveredHinge = useAppStore((s) => s.setHoveredHinge);
  const finishAngleIntent = useAppStore((s) => s.finishAngleIntent);

  const relaxation = relaxationNotices(relaxations).find((item) => item.hinge === hinge);
  // 一時指定・保存済み希望は、計算結果が譲っても入力欄へそのまま残す。
  const specified = drivers.get(hinge);
  const desired =
    specified ??
    sequenceTargets.get(hinge) ??
    relaxation?.target_angle_deg ??
    poseAngles.get(hinge) ??
    0;
  const value = Math.round(desired);
  const label = `折り目 #${hinge}`;
  const sliderId = `hinge-angle-${hinge}`;

  return (
    <article
      className="hinge-angle-item"
      aria-label={`${label}の角度設定`}
      onMouseEnter={() => setHoveredHinge(hinge)}
      onMouseLeave={() => setHoveredHinge(null)}
      onFocus={() => setHoveredHinge(hinge)}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget)) setHoveredHinge(null);
      }}
    >
      <div className="hinge-angle-name">
        <strong>{only ? "折り角度" : label}</strong>
        <span>
          {value}°
          {relaxation && (
            <small className="actual-angle">現在{relaxation.actual_angle_deg.toFixed(1)}°</small>
          )}
        </span>
      </div>
      <div className="angle-row">
        <label className="sr-only" htmlFor={sliderId}>
          {label}の角度
        </label>
        <input
          id={sliderId}
          aria-label={`${label}の角度`}
          data-tooltip="折り目の角度を調整します。+は山折り、−は谷折りです"
          type="range"
          min={ANGLE_MIN}
          max={ANGLE_MAX}
          step={1}
          value={value}
          onChange={(e) => setDriverAngle(hinge, Number(e.target.value))}
          onPointerUp={() => void finishAngleIntent()}
          onPointerCancel={() => void finishAngleIntent()}
          onBlur={() => void finishAngleIntent()}
          onKeyUp={() => void finishAngleIntent()}
        />
        <AngleNumberInput
          key={hinge}
          value={value}
          ariaLabel={`${label}の角度（数値）`}
          onValue={(angle) => setDriverAngle(hinge, angle)}
          onFinish={() => void finishAngleIntent()}
        />
        <button
          type="button"
          data-tooltip="この折り線の角度指定を解除し、形を計算し直します"
          disabled={specified === undefined}
          onClick={() => clearDriver(hinge)}
        >
          {only ? "この折り線の角度を解除" : "角度を解除"}
        </button>
      </div>
    </article>
  );
}

/** 複数選択した折り目を同じ絶対角度へそろえる一括操作。 */
function HingeAngleGroup({ hinges }: { hinges: number[] }) {
  const drivers = useAppStore((s) => s.drivers);
  const poseAngles = useAppStore((s) => s.poseAngles);
  const sequenceTargets = useAppStore((s) => s.sequenceTargets);
  const relaxations = useAppStore((s) => s.relaxations);
  const setDriverAngles = useAppStore((s) => s.setDriverAngles);
  const finishAngleIntent = useAppStore((s) => s.finishAngleIntent);
  const noticed = relaxationNotices(relaxations);
  const values = hinges.map((hinge) =>
    Math.round(
      drivers.get(hinge) ??
        sequenceTargets.get(hinge) ??
        noticed.find((item) => item.hinge === hinge)?.target_angle_deg ??
        poseAngles.get(hinge) ??
        0,
    ),
  );
  const value = values[0] ?? 0;
  const mixed = values.some((angle) => angle !== value);
  const actualValues = hinges.map(
    (hinge, index) =>
      noticed.find((item) => item.hinge === hinge)?.actual_angle_deg ??
      poseAngles.get(hinge) ??
      values[index] ??
      0,
  );
  const hasRelaxation = hinges.some((hinge) => noticed.some((item) => item.hinge === hinge));
  const firstActual = actualValues[0] ?? 0;
  const mixedActual = actualValues.some(
    (angle) => Math.abs(angle - firstActual) >= 0.05,
  );

  return (
    <section className="bulk-angle-row" aria-label="選択した折り目の一括角度設定">
      <div className="hinge-angle-name">
        <strong>まとめて動かす</strong>
        <span>
          {mixed ? "角度はばらばら" : `${value}°`}
          {hasRelaxation && (
            <small className="actual-angle">
              {mixedActual ? "現在はばらばら" : `現在${firstActual.toFixed(1)}°`}
            </small>
          )}
        </span>
      </div>
      <div className="angle-row">
        <input
          type="range"
          aria-label="選択した折り目をまとめて動かす"
          data-tooltip="選択した折り目を同じ角度へまとめて動かします"
          min={ANGLE_MIN}
          max={ANGLE_MAX}
          step={1}
          value={value}
          onChange={(e) => setDriverAngles(hinges, Number(e.target.value))}
          onPointerUp={() => void finishAngleIntent()}
          onPointerCancel={() => void finishAngleIntent()}
          onBlur={() => void finishAngleIntent()}
          onKeyUp={() => void finishAngleIntent()}
        />
        <AngleNumberInput
          key={hinges.join(",")}
          value={value}
          ariaLabel="選択した折り目をまとめて動かす角度（数値）"
          onValue={(angle) => setDriverAngles(hinges, angle)}
          onFinish={() => void finishAngleIntent()}
        />
      </div>
    </section>
  );
}

/**
 * 今つけている立体的な形を手順として残すボタン(SIM-009)。
 * 折り鶴の中央の膨らみのように、平らに畳まない仕上げの形をそのまま残す。
 * 残せないときもボタンは消さず、短い理由を添えて押せなくする。
 */
function PoseRecordButton() {
  const reason = useAppStore((s) => poseRecordReason(s));
  const recordPoseStep = useAppStore((s) => s.recordPoseStep);

  return (
    <div className="button-row">
      <button
        type="button"
        disabled={reason !== null}
        data-tooltip={
          reason ??
          "今の立体的な形を、折り角度の手順として手順一覧の最後に残します"
        }
        onClick={() => void recordPoseStep()}
      >
        この形で仕上げる
      </button>
      {reason && <span className="hint">{reason}</span>}
    </div>
  );
}

/** 折り角度の操作(選択した全ヒンジの個別/一括入力)と、全解除ボタン */
function FoldControls({ primary = false }: { primary?: boolean }) {
  const hinges = useAppStore((s) => s.hinges);
  const selection = useAppStore((s) => s.selection);
  const drivers = useAppStore((s) => s.drivers);
  const clearDrivers = useAppStore((s) => s.clearDrivers);
  const setHoveredHinge = useAppStore((s) => s.setHoveredHinge);

  // 折り線(山折り・谷折りで、両側に面がある辺)だけが角度を指定できる
  // ヒンジ集合はストアが展開図の更新時に1度だけ導出したものを使う
  const selected = [...new Set(selection.edgeIds.filter((id) => hinges.has(id)))].sort(
    (a, b) => a - b,
  );

  useEffect(() => () => setHoveredHinge(null), [setHoveredHinge]);

  if (selected.length === 0 && drivers.size === 0) return null;

  return (
    <div className={`fold-controls${primary ? " fold-controls-primary" : ""}`}>
      {selected.length > 0 && (
        <>
          <div
            className="fold-controls-heading"
            data-tooltip="Ctrl+クリックで選択を追加・解除できます"
            tabIndex={0}
          >
            <strong>折り目を{selected.length}本選択中</strong>
          </div>
          {selected.length > 1 && <HingeAngleGroup hinges={selected} />}
          <div className="hinge-angle-list" aria-label="選択した折り目ごとの角度">
            {selected.map((hinge) => (
              <HingeAngle key={hinge} hinge={hinge} only={selected.length === 1} />
            ))}
          </div>
        </>
      )}
      <PoseRecordButton />
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
  const moveStep = useAppStore((s) => s.moveStep);
  const total = useAppStore((s) => s.doc?.sequence.length ?? 0);

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
        {/* 手順の並べ替え(SEQ-005)。押せないときもボタンは消さず理由を出す */}
        <button
          type="button"
          disabled={number <= 1}
          data-tooltip={
            number <= 1
              ? "いちばん最初の手順なので、これより前へは動かせません"
              : "この手順を1つ前へ動かします(元に戻すは2回押してください)"
          }
          onClick={() => void moveStep(number, -1)}
        >
          ◀ 前へ動かす
        </button>
        <button
          type="button"
          disabled={number >= total}
          data-tooltip={
            number >= total
              ? "いちばん最後の手順なので、これより後ろへは動かせません"
              : "この手順を1つ後ろへ動かします(元に戻すは2回押してください)"
          }
          onClick={() => void moveStep(number, 1)}
        >
          後ろへ動かす ▶
        </button>
        <button
          type="button"
          data-tooltip="この手順を一覧から削除します。展開図の折り線は残ります"
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

/** 「合わせて折る」の入口(折るツールのときだけ出す。ツールレールは増やさない) */
const ALIGN_MODES: AlignMode[] = [
  "throughTwoPoints",
  "pointPoint",
  "lineLine",
  "pointPerpendicularLine",
  "pointLineThrough",
  "pointToLinePointToLine",
  "pointLinePerpendicular",
  "existingLine",
];

function AlignStartRow() {
  const beginAlign = useAppStore((s) => s.beginAlign);
  return (
    <div className="button-row align-start-row">
      <strong className="align-start-label">合わせて折る</strong>
      <div
        className="align-mode-buttons"
        role="group"
        aria-label="折り目の決め方"
      >
        {ALIGN_MODES.map((mode) => (
          <button
            key={mode}
            type="button"
            data-tooltip={ALIGN_HINTS[mode]}
            onClick={() => beginAlign(mode)}
          >
            {ALIGN_LABELS[mode]}
          </button>
        ))}
      </div>
    </div>
  );
}

/**
 * 合わせて折るの途中経過と、求まった折り線の確定UI。
 * 折り線が求まったら、下に既存の折り確定UI(山谷・対象の層・折る)をそのまま出す。
 */
function AlignDraftContent({
  draft,
  foldDraft,
}: {
  draft: AlignDraft;
  foldDraft: FoldDraft | null;
}) {
  const nextAlignSolution = useAppStore((s) => s.nextAlignSolution);
  const undoAlignPick = useAppStore((s) => s.undoAlignPick);
  const cancelAlign = useAppStore((s) => s.cancelAlign);
  const need = ALIGN_STEPS[draft.mode].length;
  const kind = nextAlignKind(draft);

  return (
    <div>
      <div className="button-row">
        <span>{ALIGN_LABELS[draft.mode]}</span>
        <span>
          選択 {draft.picks.length} / {need}
          {kind !== null &&
            `(次は${kind === "point" ? "点" : "線"}を3D表示でクリック)`}
        </span>
        {draft.solutions.length >= 2 && (
          <button type="button" onClick={() => nextAlignSolution()}>
            別の解({draft.solutionIndex + 1}/{draft.solutions.length})
          </button>
        )}
        <button
          type="button"
          disabled={draft.picks.length === 0}
          onClick={() => undoAlignPick()}
        >
          1つ戻す
        </button>
        <button type="button" onClick={() => cancelAlign()}>
          合わせるのをやめる
        </button>
      </div>
      {draft.reason !== null && <p className="warning-text">{draft.reason}</p>}
      {foldDraft && <FoldDraftContent draft={foldDraft} />}
    </div>
  );
}

/** 引いた折り線の確定UI(向き・対象の層・動かす側を決めて折る) */
function FoldDraftContent({ draft }: { draft: FoldDraft }) {
  const paper = useAppStore((s) => s.doc?.paper ?? null);
  const updateFoldDraft = useAppStore((s) => s.updateFoldDraft);
  const cancelFoldDraft = useAppStore((s) => s.cancelFoldDraft);
  const commitFoldDraft = useAppStore((s) => s.commitFoldDraft);
  const busy = useAppStore((s) => s.foldThroughBusy);
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
            disabled={busy}
            checked={draft.direction === "Up"}
            onChange={() => updateFoldDraft({ direction: "Up" })}
          />
          手前へ折る(谷)
        </label>
        <label>
          <input
            type="radio"
            name="fold-direction"
            disabled={busy}
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
            disabled={busy}
            checked={draft.target === "all"}
            onChange={() => updateFoldDraft({ target: "all" })}
          />
          全ての層
        </label>
        <label>
          <input
            type="radio"
            name="fold-target"
            disabled={busy}
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
            data-tooltip="黄色く光る側の紙を動かします"
            disabled={busy}
            checked={draft.movingSide === "right"}
            onChange={() => updateFoldDraft({ movingSide: "right" })}
          />
          こちら側
        </label>
        <label>
          <input
            type="radio"
            name="fold-side"
            data-tooltip="黄色く光る側の紙を動かします"
            disabled={busy}
            checked={draft.movingSide === "left"}
            onChange={() => updateFoldDraft({ movingSide: "left" })}
          />
          反対側
        </label>
      </div>
      <div className="button-row">
        <button type="button" disabled={busy} onClick={() => void commitFoldDraft()}>
          {busy ? "折り方を確認中…" : "折る"}
        </button>
        <button type="button" disabled={busy} onClick={() => cancelFoldDraft()}>
          やめる
        </button>
      </div>
    </div>
  );
}

/**
 * 巻き込み用の折り目を入れるか、その場で選ぶ非モーダルの提案。
 * 「追加しない」も元の折りは実行し、貫通が残れば通常の警告として知らせる。
 */
function FoldThroughProposalContent({ pending }: { pending: PendingFoldThrough }) {
  const busy = useAppStore((s) => s.foldThroughBusy);
  const resolve = useAppStore((s) => s.resolveFoldThroughProposal);
  const count = pending.proposal.crease_segments.length;

  return (
    <section className="fold-through-proposal" aria-label="巻き込み折り目の提案">
      <p className="fold-through-proposal-title">
        指定した場所以外に、ここへ折り目がつきます
      </p>
      <p>{pending.proposal.message}</p>
      <p className="hint">
        追加位置を展開図の橙色の破線と、3D表示の水色の線で確認できます
        {count > 1 ? `（展開図では${count}線分）` : ""}。
      </p>
      <div className="button-row">
        <button type="button" disabled={busy} onClick={() => void resolve(true)}>
          {busy ? "適用中…" : "追加折り目を入れて折る"}
        </button>
        <button type="button" disabled={busy} onClick={() => void resolve(false)}>
          追加せず折る（警告のみ）
        </button>
      </div>
    </section>
  );
}

/**
 * 数値の入力欄(段の幅・ねじる角・曲線の分割数)。書きかけの文字を打てるよう
 * 表示は制御せず、完全で範囲内の数値はプレビューへ即時反映する。
 * Enter・入力欄から離れたときの最終確定も残す(要件§2: 状態はストア1本)。
 */
function NumberInput({
  id,
  ariaLabel,
  value,
  min,
  max,
  onPreview,
  onCommit,
  normalizeOnCommit,
}: {
  id: string;
  ariaLabel: string;
  value: number;
  min: number;
  max?: number;
  onPreview: (v: number) => void;
  onCommit: (v: number) => void;
  normalizeOnCommit?: (v: number) => number;
}) {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const editedRef = useRef(false);

  useEffect(() => {
    const el = inputRef.current;
    if (el && document.activeElement !== el) el.value = String(value);
  }, [value]);

  const commit = () => {
    const el = inputRef.current;
    if (!el || !editedRef.current) return;
    editedRef.current = false;
    const entered = completeNumber(el.value);
    if (entered === null || entered < min) {
      el.value = String(value); // 数字でない入力は捨てて現在値へ戻す
      return;
    }
    const committed = normalizeOnCommit?.(entered) ?? entered;
    el.value = String(committed);
    onCommit(committed);
  };

  return (
    <NumberStepper
      ref={inputRef}
      id={id}
      className="angle-number"
      aria-label={ariaLabel}
      min={min}
      max={max}
      step={1}
      defaultValue={value}
      onChange={(e) => {
        editedRef.current = true;
        const entered = completeNumber(e.currentTarget.value);
        if (entered !== null && entered >= min && (max === undefined || entered <= max)) {
          onPreview(entered);
        }
      }}
      onStepComplete={commit}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
      }}
    />
  );
}

/**
 * ねじり折りの中央多角形の指定状況(TEC-009)。
 * 立体表示で角を順にクリックすると、辺の数も長さも自由な多角形をそのまま折れる。
 * ここには「今いくつ置いたか」と取り消しだけを出す(操作そのものは立体表示側)。
 */
function TwistPolygonRow({ draft }: { draft: TechniqueDraft }) {
  const undoTechniqueVertex = useAppStore((s) => s.undoTechniqueVertex);
  const setTechniqueCenter = useAppStore((s) => s.setTechniqueCenter);
  const n = draft.polygon.length;
  const ready = isTwistPolygonReady(draft.polygon);

  return (
    <div className="button-row">
      <span>
        中央の形: 角を{n}個指定{ready ? `(${n}角形)` : "(あと3個以上必要)"}
        {draft.center ? " / 中心は指定した点" : " / 中心は形の重心"}
      </span>
      <button
        type="button"
        disabled={n === 0}
        data-tooltip="最後に選んだ中央の角を取り消します"
        onClick={() => undoTechniqueVertex()}
      >
        角を1つ戻す
      </button>
      <button
        type="button"
        disabled={draft.center === null}
        data-tooltip="指定した中心をやめ、中央の形の重心を使います"
        onClick={() => setTechniqueCenter(null)}
      >
        中心を重心へ戻す
      </button>
    </div>
  );
}

/**
 * クリック地点に重なる層から、技法の対象だけを選ぶ。
 * 候補順は facesAtPoint と同じ奥→手前。大量の層でも4つの指定ボタンを先に使え、
 * 必要なときだけdetailsを開いて1枚ずつチェックできる。
 */
function TechniqueLayerPicker({ draft }: { draft: TechniqueDraft }) {
  const setPreset = useAppStore((s) => s.setTechniqueFlapPreset);
  const toggleFlap = useAppStore((s) => s.toggleTechniqueFlap);
  const updateTechniqueDraft = useAppStore((s) => s.updateTechniqueDraft);
  const candidates = draft.flapCandidates;
  const selected = new Set(draft.flap);
  const hasCandidates = candidates.length > 0;

  return (
    <fieldset className="technique-layer-picker">
      <legend>対象にする層</legend>
      <div className="button-row">
        <span>
          候補{candidates.length}枚(奥→手前) / 選択{draft.flap.length}枚
        </span>
        <label htmlFor="technique-layer-count">N(枚数・奥行き)</label>
        <NumberInput
          id="technique-layer-count"
          ariaLabel="対象層の枚数"
          value={draft.flapPickCount}
          min={1}
          max={Math.max(1, candidates.length)}
          onPreview={(v) => updateTechniqueDraft({ flapPickCount: v })}
          onCommit={(v) => updateTechniqueDraft({ flapPickCount: v })}
          normalizeOnCommit={(v) =>
            clampTechniqueLayerCount(v, candidates.length)
          }
        />
        <button type="button" disabled={!hasCandidates} onClick={() => setPreset("all")}>
          全部
        </button>
        <button type="button" disabled={!hasCandidates} onClick={() => setPreset("front")}>
          手前からN枚
        </button>
        <button type="button" disabled={!hasCandidates} onClick={() => setPreset("back")}>
          奥からN枚
        </button>
        <button
          type="button"
          disabled={!hasCandidates}
          onClick={() => setPreset("frontNth")}
        >
          手前からN枚目
        </button>
      </div>
      {hasCandidates ? (
        <details className="technique-layer-candidates">
          <summary>候補ごとのチェック切替</summary>
          <div className="technique-layer-candidate-list">
            {candidates.map((face, i) => {
              const fromBack = i + 1;
              const fromFront = candidates.length - i;
              return (
                <label key={face}>
                  <input
                    type="checkbox"
                    checked={selected.has(face)}
                    onChange={() => toggleFlap(face)}
                  />
                  奥から{fromBack}枚目 / 手前から{fromFront}枚目(面{face})
                </label>
              );
            })}
          </div>
        </details>
      ) : (
        <span className="hint">
          {draft.kind === "Twist"
            ? "Shift+クリックで対象層を選びます"
            : "3Dの紙をクリックして対象層を選びます"}
        </span>
      )}
    </fieldset>
  );
}

/** Ctrl+クリックで明示できる、名前付き技法ごとの基準点の呼び名。 */
function techniqueReferenceLabel(kind: TechniqueKind): string {
  switch (kind) {
    case "Pleat":
      return "段の行き先";
    case "InsideReverse":
    case "OutsideReverse":
      return "先端の行き先";
    case "Squash":
      return "つぶす先";
    case "Petal":
      return "持ち上げる先端";
    case "OpenSink":
      return "沈める先端";
    case "Swivel":
      return "寄せる先";
    default:
      return "基準点";
  }
}

/** 自動の左右指定では足りない技法で、任意の基準点を直接指定する入口。 */
function TechniqueReferenceRow({ draft }: { draft: TechniqueDraft }) {
  const setReference = useAppStore((s) => s.setTechniqueReferencePoint);
  const label = techniqueReferenceLabel(draft.kind);

  return (
    <div className="button-row">
      <span
        data-tooltip={`3DをCtrl+クリックすると任意の${label}を指定できます`}
        tabIndex={0}
      >
        {label}: {draft.referencePoint === null ? "自動" : "指定した点"}
      </span>
      <button
        type="button"
        disabled={draft.referencePoint === null}
        data-tooltip={`指定した${label}をやめ、自動で決めます`}
        onClick={() => setReference(null)}
      >
        基準点を自動へ戻す
      </button>
    </div>
  );
}

/** 名前付き技法では表せない、既存折り目の開閉・重ね替え・層限定反転。 */
function LayerMotionDraftContent({ draft }: { draft: TechniqueDraft }) {
  const updateTechniqueDraft = useAppStore((s) => s.updateTechniqueDraft);
  const addLayerMotionPart = useAppStore((s) => s.addLayerMotionPart);
  const undoLayerMotionPart = useAppStore((s) => s.undoLayerMotionPart);
  const cancelTechnique = useAppStore((s) => s.cancelTechnique);
  const commitTechnique = useAppStore((s) => s.commitTechnique);
  const current = {
    layers: draft.flap,
    line: draft.line,
    mode: draft.motionMode,
    turn: draft.motionTurn,
    direction: draft.motionDirection,
    anchor: draft.motionAnchor,
    reverseLayers: draft.motionReverseLayers,
  } as const;
  const hasCurrent = hasLayerMotionInput(current);
  const built = buildLayerMotionPart(current);
  const exactAxisReady =
    draft.motionMode !== "reflect" || draft.motionAxisEdgeId !== null;
  const currentValid = !hasCurrent || (built.ok && exactAxisReady);
  const ready =
    currentValid &&
    (draft.motionParts.length > 0 || (hasCurrent && built.ok));

  return (
    <div>
      <p>
        層操作: 追加済み{draft.motionParts.length}部分 / 現在{draft.flap.length}層を選択
      </p>
      <TechniqueLayerPicker draft={draft} />
      <div className="button-row">
        <span>操作:</span>
        <label>
          <input
            type="radio"
            name="layer-motion-mode"
            checked={draft.motionMode === "reflect"}
            onChange={() =>
              updateTechniqueDraft({ motionMode: "reflect", motionTurn: "Keep" })
            }
          />
          既存折り目で開閉
        </label>
        <label>
          <input
            type="radio"
            name="layer-motion-mode"
            checked={draft.motionMode === "stay"}
            onChange={() =>
              updateTechniqueDraft({
                motionMode: "stay",
                line: null,
                motionAxisEdgeId: null,
              })
            }
          />
          動かさず重ね替え
        </label>
      </div>
      {draft.motionMode === "reflect" ? (
        <div className="button-row">
          <span
            data-tooltip="3Dの既存折り目をクリックして、正確な開閉軸を選びます"
            tabIndex={0}
          >
            軸: {draft.motionAxisEdgeId === null ? "未選択" : `折り目${draft.motionAxisEdgeId}`}
          </span>
        </div>
      ) : (
        <div className="button-row">
          <label htmlFor="layer-motion-turn">重ね方</label>
          <select
            id="layer-motion-turn"
            value={draft.motionTurn}
            onChange={(e) =>
              updateTechniqueDraft({ motionTurn: e.target.value as LayerTurnMode })
            }
          >
            <option value="Keep">位置を保つ</option>
            <option value="Outside">重なり全体の外側</option>
            <option value="Inside">元の紙のすぐ隣</option>
            <option value="Beside">指定面のすぐ隣</option>
          </select>
          {draft.motionTurn === "Beside" && (
            <>
              <label htmlFor="layer-motion-anchor">基準面ID</label>
              <NumberInput
                id="layer-motion-anchor"
                ariaLabel="基準面ID"
                value={draft.motionAnchor}
                min={0}
                onPreview={(v) => updateTechniqueDraft({ motionAnchor: v })}
                onCommit={(v) => updateTechniqueDraft({ motionAnchor: v })}
                normalizeOnCommit={(v) => Math.max(0, Math.round(v))}
              />
            </>
          )}
          {draft.motionTurn !== "Keep" && (
            <>
              <label>
                <input
                  type="radio"
                  name="layer-motion-direction"
                  checked={draft.motionDirection === "Up"}
                  onChange={() => updateTechniqueDraft({ motionDirection: "Up" })}
                />
                手前側
              </label>
              <label>
                <input
                  type="radio"
                  name="layer-motion-direction"
                  checked={draft.motionDirection === "Down"}
                  onChange={() => updateTechniqueDraft({ motionDirection: "Down" })}
                />
                奥側
              </label>
            </>
          )}
        </div>
      )}
      <div className="button-row">
        <label>
          <input
            type="checkbox"
            data-tooltip="選択した層だけ山折りと谷折り、層順を反転します。未選択なら全層が対象です"
            checked={draft.motionReverseLayers}
            onChange={(e) =>
              updateTechniqueDraft({ motionReverseLayers: e.target.checked })
            }
          />
          選択層だけ山谷反転(層順も反転)
        </label>
      </div>
      {draft.motionParts.length > 0 && (
        <div className="button-row" aria-label="追加済みの同時層操作">
          {draft.motionParts.map((part, index) => (
            <span key={`${index}-${describeLayerMotionPart(part)}`}>
              {index + 1}. {describeLayerMotionPart(part)}
            </span>
          ))}
        </div>
      )}
      <div className="button-row">
        <button
          type="button"
          disabled={!hasCurrent || !built.ok || !exactAxisReady}
          data-tooltip={
            !exactAxisReady
              ? "立体表示で既存の折り目をクリックして、正確な開閉軸を選んでください"
              : built.ok
                ? "現在の部分を同じ1手へ追加します"
                : built.error
          }
          onClick={() => addLayerMotionPart()}
        >
          この部分を追加
        </button>
        <button
          type="button"
          disabled={draft.motionParts.length === 0}
          onClick={() => undoLayerMotionPart()}
        >
          直前の追加を外す
        </button>
        <button
          type="button"
          disabled={!ready}
          data-tooltip={
            ready
              ? "追加済みと現在の部分を1手として同時に適用します"
              : hasCurrent && !exactAxisReady
                ? "立体表示で既存の折り目をクリックして、正確な開閉軸を選んでください"
                : hasCurrent && !built.ok
                  ? built.error
                : "層操作を1つ以上指定してください"
          }
          onClick={() => void commitTechnique()}
        >
          まとめて適用
        </button>
        <button type="button" onClick={() => cancelTechnique()}>
          やめる
        </button>
      </div>
    </div>
  );
}

/** 技法の確定UI(フラップ・折り線を選んでから適用する) */
function TechniqueDraftContent({ draft }: { draft: TechniqueDraft }) {
  if (draft.kind === "Simple") return <LayerMotionDraftContent draft={draft} />;
  return <NamedTechniqueDraftContent draft={draft} />;
}

/** 従来の名前付き技法。層操作とは下書きの入力形が異なるため別コンポーネントにする。 */
function NamedTechniqueDraftContent({ draft }: { draft: TechniqueDraft }) {
  const paper = useAppStore((s) => s.doc?.paper ?? null);
  const updateTechniqueDraft = useAppStore((s) => s.updateTechniqueDraft);
  const cancelTechnique = useAppStore((s) => s.cancelTechnique);
  const commitTechnique = useAppStore((s) => s.commitTechnique);
  const scale = paper ? Math.max(paper.width_mm, paper.height_mm) : 1;
  const mm = (v: number) => (v * scale).toFixed(1);
  // ねじり折りは中央多角形を角のクリックで指せる(層は選ばなくてよい)
  const byPolygon = draft.kind === "Twist" && isTwistPolygonReady(draft.polygon);
  const minimumFlap = minimumTechniqueFlap(draft.kind);
  const needsFlap = minimumFlap > 0;
  const flapOk = draft.flap.length >= minimumFlap;
  const ready = (draft.line !== null || byPolygon) && flapOk;
  const openSide = techniqueUsesOpenToBack(draft.kind);

  return (
    <div>
      {draft.kind === "Twist" && <TwistPolygonRow draft={draft} />}
      <p>
        {TECHNIQUE_LABEL[draft.kind]}: 層を{draft.flap.length}枚選択中
        {byPolygon ? (
          " / 中央の形で折ります(層を選ばなければ全ての層)"
        ) : draft.line ? (
          <>
            {" "}
            / 折り線 ({mm(draft.line[0][0])}, {mm(draft.line[0][1])}) →(
            {mm(draft.line[1][0])}, {mm(draft.line[1][1])}) mm
          </>
        ) : (
          " / 折り線はまだ引かれていません"
        )}
      </p>
      <TechniqueLayerPicker draft={draft} />
      {draft.kind !== "Twist" && <TechniqueReferenceRow draft={draft} />}
      {/* どちらの技法でも「動く側」を選ぶ。中割り・かぶせでは折り返される先端の側、
          段折りでは段になって送られる側にあたる(反対側の紙はその場に残る) */}
      <div className="button-row">
        <span>
          {draft.kind === "Twist"
            ? "ねじる向き"
            : draft.kind === "Pleat"
              ? "段になる側"
              : "先端(動く側)"}
        </span>
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
            <NumberInput
              id="pleat-width"
              ariaLabel="段の幅（mm）"
              value={draft.widthMm}
              min={0.1}
              onPreview={(v) => updateTechniqueDraft({ widthMm: v })}
              onCommit={(v) => updateTechniqueDraft({ widthMm: v })}
            />
          </>
        )}
        {draft.kind === "Twist" && (
          <>
            <label htmlFor="twist-deg">ねじる角(度)</label>
            <NumberInput
              id="twist-deg"
              ariaLabel="ねじる角（度）"
              value={draft.twistDeg}
              min={0.1}
              onPreview={(v) => updateTechniqueDraft({ twistDeg: v })}
              onCommit={(v) => updateTechniqueDraft({ twistDeg: v })}
            />
          </>
        )}
      </div>
      {openSide && (
        <div className="button-row">
          <span>開く側:</span>
          <label>
            <input
              type="radio"
              name="technique-open-side"
              aria-label="開く側: 手前"
              data-tooltip="動かした紙を重なりの手前へ置きます"
              checked={!draft.openToBack}
              onChange={() => updateTechniqueDraft({ openToBack: false })}
            />
            手前
          </label>
          <label>
            <input
              type="radio"
              name="technique-open-side"
              aria-label="開く側: 向こう"
              data-tooltip="動かした紙を重なりの奥へ入れます"
              checked={draft.openToBack}
              onChange={() => updateTechniqueDraft({ openToBack: true })}
            />
            向こう
          </label>
        </div>
      )}
      <div className="button-row">
        <button
          type="button"
          disabled={!ready}
          data-tooltip={
            ready
              ? "選んだ技法で折ります"
              : draft.line === null && draft.kind === "Twist"
                ? "立体表示で中央の形の角を3つ以上クリックしてください"
                : draft.line === null
                  ? "立体表示で紙の上をドラッグして折り線を引いてください"
                  : needsFlap && !flapOk
                    ? `立体表示で紙をクリックして、対象の層を${minimumFlap}枚以上選んでください`
                    : "選んだ技法の指定を確認してください"
          }
          onClick={() => void commitTechnique()}
        >
          適用
        </button>
        <button type="button" onClick={() => cancelTechnique()}>
          やめる
        </button>
      </div>
    </div>
  );
}

/**
 * 「引く」ツールを選んでいるときの内容(UI-007)。
 * 左右同時に動かすかの切替をここに置く(ツールレールも常設区画も増やさない)。
 * 折り紙の作品はほとんどが左右対称なので既定はオン。片方だけ形を変えたいとき
 * (くちばしの角度を少しだけ変える等)に切れるようにしてある。
 */
function PullContent() {
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
 * 紙の形を直接変える2つの入口。ツール名だけでは結果を想像しにくいので、
 * 「何が起きるか」を動詞で並べる。膨らみは設定を開くだけで、強さは利用者が
 * 下のつまみを動かして決める(勝手に作品の形を変えない)。
 */
function PaperActionEntrances({ showPull = true }: { showPull?: boolean }) {
  const setTool = useAppStore((s) => s.setTool);
  const setSelection = useAppStore((s) => s.setSelection);
  const setSoft = useAppStore((s) => s.setSoft);

  const showInflate = () => {
    setTool("select");
    setSelection({ edgeIds: [], vertexIds: [] });
    setSoft({ soft_enabled: true });
  };

  return (
    <div className="paper-action-entrances" aria-label="紙の形を変える">
      <span className="paper-action-entrances-title">紙の形を変える</span>
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
    </div>
  );
}

/**
 * 曲線の折り目(CPE-011)の設定。山折り・谷折り・補助線ツールのときだけ出す。
 * ツールレールは10個で上限なので曲線用のツールは増やさず、既存の線ツールの
 * 「直線/曲線」の切り替えとしてここに置く(線を引く操作の設定は1か所にまとまる)。
 */
function CurveRow() {
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

function SelectionContent() {
  const doc = useAppStore((s) => s.doc);
  const selection = useAppStore((s) => s.selection);
  const applyEdit = useAppStore((s) => s.applyEdit);
  const wheelBehavior = useAppStore((s) => s.wheelBehavior);
  const contextHelpExpanded = useAppStore((s) => s.contextHelpExpanded);

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

/** 希望角を譲った折り目。作品を色付けせず、控えめな一覧から選択できるようにする。 */
function RelaxationMessages() {
  const relaxations = useAppStore((s) => s.relaxations);
  const setSelection = useAppStore((s) => s.setSelection);
  const setHoveredHinge = useAppStore((s) => s.setHoveredHinge);
  const notices = relaxationNotices(relaxations);
  const shown = notices.slice(0, 5);
  const remaining = notices.length - shown.length;

  if (shown.length === 0) return null;
  return (
    <div className="relaxation-messages" aria-label="前の折り目の追従">
      {shown.map((item) => (
        <button
          type="button"
          className="relaxation-message"
          key={item.hinge}
          onMouseEnter={() => setHoveredHinge(item.hinge)}
          onMouseLeave={() => setHoveredHinge(null)}
          onFocus={() => setHoveredHinge(item.hinge)}
          onBlur={() => setHoveredHinge(null)}
          onClick={() => {
            setSelection({ edgeIds: [item.hinge], vertexIds: [] });
            setHoveredHinge(item.hinge);
          }}
        >
          折り目 #{item.hinge}: 指定{item.target_angle_deg.toFixed(1)}° → 現在
          {item.actual_angle_deg.toFixed(1)}°
        </button>
      ))}
      {remaining > 0 && <p className="relaxation-more">ほか{remaining}本</p>}
    </div>
  );
}

export function ContextPanel() {
  const warnings = useAppStore((s) => s.warnings);
  const poseWarnings = useAppStore((s) => s.poseWarnings);
  const replayWarnings = useAppStore((s) => s.replayWarnings);
  const flatFoldViolations = useAppStore((s) => s.flatFoldViolations);
  const errorMessage = useAppStore((s) => s.errorMessage);
  const mirrorAxisNotice = useAppStore((s) => s.mirrorAxisNotice);
  const currentStep = useAppStore((s) => s.currentStep);
  const activeTool = useAppStore((s) => s.activeTool);
  const foldDraft = useAppStore((s) => s.foldDraft);
  const pendingFoldThrough = useAppStore((s) => s.pendingFoldThrough);
  const alignDraft = useAppStore((s) => s.alignDraft);
  const techniqueDraft = useAppStore((s) => s.techniqueDraft);
  const selection = useAppStore((s) => s.selection);
  const hinges = useAppStore((s) => s.hinges);
  const relaxations = useAppStore((s) => s.relaxations);
  // 同じ文言は1回だけ出す(展開図の検査結果には自動再生の警告も合流している)
  const flatFoldWarning = flatFoldNotice(flatFoldViolations);
  const allWarnings = uniqueWarnings(
    warnings,
    poseWarnings,
    replayWarnings,
    flatFoldWarning === null ? [] : [flatFoldWarning],
  );
  const hasRelaxations = relaxationNotices(relaxations).length > 0;
  // 手順を選んでいる間はその手順の設定を出す(「折る前」「最新」は選択なし扱い)
  const stepSelected = currentStep !== null && currentStep >= 1;
  const hasSelection =
    selection.edgeIds.length > 0 || selection.vertexIds.length > 0;
  const hasSelectedHinge = selection.edgeIds.some((id) => hinges.has(id));

  return (
    <footer className="context-panel" id="context-panel">
      <div className="context-selection">
        {/* 手順を選んでいるときはその設定を優先する。折り線は「今見えている形」の
            上に引くものなので、手順を選んだ時点でストアが捨てている(ここは念のため) */}
        {pendingFoldThrough ? (
          <>
            <FoldThroughProposalContent pending={pendingFoldThrough} />
            <OperationSteps />
          </>
        ) : stepSelected ? (
          <>
            <StepContent number={currentStep} />
            <OperationSteps />
          </>
        ) : techniqueDraft ? (
          <>
            <TechniqueDraftContent draft={techniqueDraft} />
            <OperationSteps />
          </>
        ) : alignDraft ? (
          <>
            <AlignDraftContent draft={alignDraft} foldDraft={foldDraft} />
            <OperationSteps />
          </>
        ) : foldDraft ? (
          <>
            <FoldDraftContent draft={foldDraft} />
            <OperationSteps />
          </>
        ) : hasSelectedHinge ? (
          <>
            <FoldControls primary />
            <OperationSteps />
            {activeTool === "pull" ? <PullContent /> : <SelectionContent />}
            {LINE_TOOLS.includes(activeTool) && <CurveRow />}
            {activeTool === "fold" && <AlignStartRow />}
          </>
        ) : (
          <>
            {!hasSelection && <OperationSteps />}
            {activeTool === "pull" ? (
              <PullContent />
            ) : (
              <>
                {!hasSelection && LINE_TOOLS.includes(activeTool) && <CurveRow />}
                {!hasSelection && activeTool === "fold" && <AlignStartRow />}
                <SelectionContent />
                {hasSelection && LINE_TOOLS.includes(activeTool) && <CurveRow />}
                {hasSelection && activeTool === "fold" && <AlignStartRow />}
              </>
            )}
            <FoldControls />
            {hasSelection && <OperationSteps />}
          </>
        )}
      </div>
      {(errorMessage !== null ||
        mirrorAxisNotice !== null ||
        allWarnings.length > 0 ||
        hasRelaxations) && (
        <div className="context-messages">
          {errorMessage !== null && <p className="error-text">{errorMessage}</p>}
          {mirrorAxisNotice !== null && (
            <p className="mirror-axis-notice">{mirrorAxisNotice}</p>
          )}
          <RelaxationMessages />
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
