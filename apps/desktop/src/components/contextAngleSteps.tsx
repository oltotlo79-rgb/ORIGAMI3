// ContextPanel の角度操作・手順表示。状態の窓口は appStore facade に固定する。

import { useEffect, useRef } from "react";
import {
  isStepSkipped,
  poseRecordReason,
  relaxationNotices,
  useAppStore,
} from "../store/appStore";
import {
  stepDisplayLabel,
  TECHNIQUE_KINDS,
  TECHNIQUE_LABEL,
} from "../lib/techniques";
import type { FoldStep, TechniqueKind } from "../lib/types";
import { NumberStepper } from "./NumberStepper";

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

/**
 * 角度を固定した折り目に付ける印。2D展開図・3Dに出る印と同じ形にして、
 * ボタンと折り目の印が同じものだと言葉なしで分かるようにする。
 */
function PinMark({ released }: { released: boolean }) {
  return (
    <svg
      className="pin-mark"
      viewBox="0 0 16 16"
      aria-hidden="true"
      focusable="false"
    >
      <circle
        cx="8"
        cy="8"
        r="5.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeDasharray={released ? "3 3" : undefined}
      />
      {!released && <circle cx="8" cy="8" r="1.8" fill="currentColor" />}
    </svg>
  );
}

/** 選択中の折り線1本の角度操作(スライダー+数値入力+固定+解除) */
function HingeAngle({ hinge, only }: { hinge: number; only: boolean }) {
  const drivers = useAppStore((s) => s.drivers);
  const poseAngles = useAppStore((s) => s.poseAngles);
  const sequenceTargets = useAppStore((s) => s.sequenceTargets);
  const relaxations = useAppStore((s) => s.relaxations);
  const pinnedFolds = useAppStore((s) => s.pinnedFolds);
  const releasedPins = useAppStore((s) => s.releasedPins);
  const setDriverAngle = useAppStore((s) => s.setDriverAngle);
  const clearDriver = useAppStore((s) => s.clearDriver);
  const togglePinnedFold = useAppStore((s) => s.togglePinnedFold);
  const setHoveredHinge = useAppStore((s) => s.setHoveredHinge);
  const finishAngleIntent = useAppStore((s) => s.finishAngleIntent);
  const pinned = pinnedFolds.has(hinge);
  const released = releasedPins.find((pin) => pin.hinge === hinge);

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
          {released ? (
            <small className="actual-angle">
              固定を外して現在{released.actual.toFixed(1)}°
            </small>
          ) : (
            relaxation && (
              <small className="actual-angle">
                現在{relaxation.actual_angle_deg.toFixed(1)}°
              </small>
            )
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
          className={`pin-toggle${pinned ? " pinned" : ""}`}
          aria-pressed={pinned}
          data-tooltip={
            pinned
              ? "この折り目の角度の固定をやめます。ほかの折り目に合わせて動くようになります"
              : "この折り目の角度を固定します。ほかの折り目を動かしても、この角度のままになります"
          }
          onClick={() => togglePinnedFold(hinge)}
        >
          <PinMark released={released !== undefined} />
          {pinned ? "角度の固定を外す" : "角度を固定"}
        </button>
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
  const pinnedFolds = useAppStore((s) => s.pinnedFolds);
  const setPinnedFolds = useAppStore((s) => s.setPinnedFolds);
  const allPinned = hinges.every((hinge) => pinnedFolds.has(hinge));
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
        <button
          type="button"
          className={`pin-toggle${allPinned ? " pinned" : ""}`}
          aria-pressed={allPinned}
          data-tooltip={
            allPinned
              ? "選んだ折り目の固定をまとめてやめます"
              : "選んだ折り目の角度をまとめて固定します。ほかの折り目を動かしても、この角度のままになります"
          }
          onClick={() => setPinnedFolds(hinges, !allPinned)}
        >
          <PinMark released={false} />
          {allPinned ? "角度の固定をまとめて外す" : "角度をまとめて固定"}
        </button>
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
export function FoldControls({ primary = false }: { primary?: boolean }) {
  const hinges = useAppStore((s) => s.hinges);
  const selection = useAppStore((s) => s.selection);
  const drivers = useAppStore((s) => s.drivers);
  const pinnedFolds = useAppStore((s) => s.pinnedFolds);
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
            {pinnedFolds.size > 0 && (
              // 選んでいない折り目も含めた「いま固定している本数」。
              // 新しい区画は作らず、この見出しの中に出す。
              <span className="pinned-count">
                <PinMark released={false} />
                角度を固定中{pinnedFolds.size}本
              </span>
            )}
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
export function StepContent({ number }: { number: number }) {
  const doc = useAppStore((s) => s.doc);
  // 飛ばされたかどうかは作品全体の再生結果で決める(タイムラインの札と同じ判断)
  const skipped = useAppStore((s) => s.skipped);
  const replaySkipped = useAppStore((s) => s.replaySkipped);
  const applySequenceOp = useAppStore((s) => s.applySequenceOp);
  const moveStep = useAppStore((s) => s.moveStep);
  const total = useAppStore((s) => s.doc?.sequence.length ?? 0);

  const step = doc?.sequence[number - 1];
  if (!step) return <p className="hint">この手順はもうありません</p>;

  // 利用者が「折り方」を明示的に選び直したときは、記録済みの技法名を引き継がない
  // (設計§6)。手順に技法名の項目が無ければそのまま(項目を新設しない)。
  const setKind = (kind: TechniqueKind) => {
    const next: FoldStep = { ...step, kind };
    if (next.technique_classification !== undefined) {
      delete next.technique_classification;
    }
    void applySequenceOp({ type: "UpdateStep", step: next });
  };

  return (
    <div>
      <p>
        手順{number}: {stepDisplayLabel(step)}(折り線
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
              : "この手順を1つ前へ動かします(元に戻す1回で戻せます)"
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
              : "この手順を1つ後ろへ動かします(元に戻す1回で戻せます)"
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

/**
 * 数値の入力欄(段の幅・ねじる角・曲線の分割数)。書きかけの文字を打てるよう
 * 表示は制御せず、完全で範囲内の数値はプレビューへ即時反映する。
 * Enter・入力欄から離れたときの最終確定も残す(要件§2: 状態はストア1本)。
 */
export function NumberInput({
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

/** 希望角を譲った折り目。作品を色付けせず、控えめな一覧から選択できるようにする。 */
export function RelaxationMessages() {
  const relaxations = useAppStore((s) => s.relaxations);
  const releasedPins = useAppStore((s) => s.releasedPins);
  const setSelection = useAppStore((s) => s.setSelection);
  const setHoveredHinge = useAppStore((s) => s.setHoveredHinge);
  const notices = relaxationNotices(relaxations);
  const released = new Set(releasedPins.map((pin) => pin.hinge));

  if (notices.length === 0) return null;
  return (
    <div className="relaxation-messages" aria-label="前の折り目の追従">
      {notices.map((item) => (
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
          折り目 #{item.hinge}: {released.has(item.hinge) ? "固定" : "指定"}
          {item.target_angle_deg.toFixed(1)}° → 現在
          {item.actual_angle_deg.toFixed(1)}°
        </button>
      ))}
    </div>
  );
}

