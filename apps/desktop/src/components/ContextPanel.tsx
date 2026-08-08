// 下部コンテキストパネル(160px)。選択状態に応じて内容を切り替える。
// 警告・エラーの詳細もここに表示する(常設パネルを増やさない)。

import { useEffect, useRef } from "react";
import {
  isStepSkipped,
  nextAlignKind,
  poseRecordReason,
  useAppStore,
  type AlignDraft,
  type FoldDraft,
  type PendingFoldThrough,
  type TechniqueDraft,
  type ToolId,
} from "../store/appStore";
import {
  CURVE_LABEL,
  DEFAULT_CURVE_TOL,
  MAX_CURVE_SEGMENTS,
  type CurveShape,
} from "../lib/curve";
import { ALIGN_LABELS, ALIGN_STEPS, type AlignMode } from "../lib/alignFold";
import {
  TECHNIQUE_KINDS,
  TECHNIQUE_LABEL,
  uniqueWarnings,
} from "../lib/techniques";
import { isTwistPolygonReady } from "../lib/twistPolygon";
import type { EdgeKind, FoldStep, TechniqueKind } from "../lib/types";
import { PaperAppearance } from "./PaperAppearance";
import { OperationSteps } from "./OperationSteps";

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
        title={
          reason ??
          "今の立体的な形を、折り角度の手順として手順一覧の最後に残します"
        }
        onClick={() => void recordPoseStep()}
      >
        この形で仕上げる
      </button>
      <span className="hint">
        {reason ?? "今の形が手順一覧に残り、展開図を編集しても戻せます"}
      </span>
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
          title={
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
          title={
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

/** 「合わせて折る」の入口(折るツールのときだけ出す。ツールレールは増やさない) */
function AlignStartRow() {
  const beginAlign = useAppStore((s) => s.beginAlign);
  const modes: AlignMode[] = ["pointPoint", "lineLine", "pointLineThrough"];
  return (
    <div className="button-row">
      <span>合わせて折る</span>
      {modes.map((m) => (
        <button key={m} type="button" onClick={() => beginAlign(m)}>
          {ALIGN_LABELS[m]}
        </button>
      ))}
      <span className="hint">
        (目分量ではなく、選んだ点や線から折り線を正確に決めます)
      </span>
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
            disabled={busy}
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
 * 数値の入力欄(段の幅・ねじる角)。書きかけの文字を打てるよう表示は制御せず、
 * 確定(Enter・入力欄から離れたとき)にストアへ送る(要件§2: 状態はストア1本)。
 */
function NumberInput({
  id,
  value,
  min,
  onCommit,
}: {
  id: string;
  value: number;
  min: number;
  onCommit: (v: number) => void;
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
    const entered = Number(el.value);
    if (!Number.isFinite(entered) || entered < min) {
      el.value = String(value); // 数字でない入力は捨てて現在値へ戻す
      return;
    }
    el.value = String(entered);
    onCommit(entered);
  };

  return (
    <input
      ref={inputRef}
      id={id}
      type="number"
      className="angle-number"
      min={min}
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
      <button type="button" disabled={n === 0} onClick={() => undoTechniqueVertex()}>
        角を1つ戻す
      </button>
      <button
        type="button"
        disabled={draft.center === null}
        onClick={() => setTechniqueCenter(null)}
      >
        中心を重心へ戻す
      </button>
      <span className="hint">
        立体表示で中央の形の角を順にクリックしてください(3つ以上)。
        Ctrl+クリックで中心、Backspaceで1つ戻す、Escでやめる
      </span>
    </div>
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
  // ねじり折りは中央多角形を角のクリックで指せる(層は選ばなくてよい)
  const byPolygon = draft.kind === "Twist" && isTwistPolygonReady(draft.polygon);
  const needsFlap = draft.kind !== "Pleat" && !byPolygon;
  // 中割り折り・かぶせ折りには重なった層が要る(枚数は奇数でもよい)
  const flapOk = draft.flap.length >= 2;
  const ready = (draft.line !== null || byPolygon) && (!needsFlap || flapOk);

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
      {/* どちらの技法でも「動く側」を選ぶ。中割り・かぶせでは折り返される先端の側、
          段折りでは段になって送られる側にあたる(反対側の紙はその場に残る) */}
      <div className="button-row">
        <span>
          {draft.kind === "Twist"
            ? "ねじる向き"
            : needsFlap
              ? "先端(動く側)"
              : "段になる側"}
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
              value={draft.widthMm}
              min={0.1}
              onCommit={(v) => updateTechniqueDraft({ widthMm: v })}
            />
          </>
        )}
        {draft.kind === "Twist" && (
          <>
            <label htmlFor="twist-deg">ねじる角(度)</label>
            <NumberInput
              id="twist-deg"
              value={draft.twistDeg}
              min={0.1}
              onCommit={(v) => updateTechniqueDraft({ twistDeg: v })}
            />
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
              : draft.kind === "Twist"
                ? "立体表示で中央の形の角を3つ以上クリックしてください"
                : needsFlap && !flapOk
                  ? "立体表示で紙をクリックして、重なった層を選んでください"
                  : "立体表示で紙の上をドラッグして折り線を引いてください"
          }
          onClick={() => void commitTechnique()}
        >
          適用
        </button>
        <button type="button" onClick={() => cancelTechnique()}>
          やめる
        </button>
        <span className="hint">
          {draft.kind === "Twist"
            ? "立体表示で指した中央の形と、そこから出るひだの折り線を黄色で見せています。層を選ばなければ全ての層をねじります"
            : "立体表示で紙をクリックすると、その場所に重なっている層をまとめて選びます。そのままドラッグすると折り線を引けます(黄色く光っている層が対象です)"}
        </span>
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

  return (
    <div>
      <p className="hint">
        立体表示で紙をドラッグすると、折り線のつじつまを合わせて全体が連動して動きます
        (右ドラッグで視点を回す)
      </p>
      <div className="button-row">
        <label>
          <input
            type="checkbox"
            aria-label="左右対称に動かす"
            checked={pullMirror}
            onChange={(e) => setPullMirror(e.target.checked)}
          />
          左右対称に動かす
        </label>
        <span className="hint">
          {pullMirror
            ? "作品の対称軸をはさんで対になる折り線も同じ角度で動きます(鶴の両羽が一緒に開きます)。対になる折り線が無いところでは、そこだけが動きます"
            : "つかんだ側の折り線だけが動きます(片方の羽だけ形を変えたいときはこちら)"}
        </span>
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
        <button type="button" onClick={() => setTool("pull")}>
          ↔ 紙を引いて動かす
        </button>
      )}
      <button type="button" onClick={showInflate}>
        ◯ 紙をふくらませる
      </button>
      <span className="hint">3Dを見ながら、その場で形を調整できます</span>
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
          checked={curve.enabled}
          onChange={(e) => setCurve({ enabled: e.target.checked })}
        />
        曲線で描く
      </label>
      {!curve.enabled ? (
        <span className="hint">
          曲線の折り目(曲線折り)を引きます。細かい折れ線として展開図に入ります
        </span>
      ) : (
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
              checked={curve.segments !== null}
              onChange={(e) => setCurve({ segments: e.target.checked ? 16 : null })}
            />
            分割数を指定
          </label>
          {curve.segments === null ? (
            <span className="hint">
              自動(曲線と折れ線のずれが紙の長辺の
              {(DEFAULT_CURVE_TOL * 100).toFixed(1)}%以内になるまで細かくします)
            </span>
          ) : (
            <NumberInput
              id="curve-segments"
              value={curve.segments}
              min={1}
              onCommit={(v) =>
                setCurve({ segments: Math.min(MAX_CURVE_SEGMENTS, Math.round(v)) })
              }
            />
          )}
          <label>
            <input
              type="checkbox"
              aria-label="紙が曲がるための線も引く"
              checked={curve.rulings}
              onChange={(e) => setCurve({ rulings: e.target.checked })}
            />
            曲がるための線も引く
          </label>
          <span className="hint">
            {curve.rulings
              ? "曲線の両側に、紙が曲がるための線を入れます(実際の紙と同じで、これが無いと曲線折りは折れません)"
              : "折り線だけを引きます(展開図の見た目は素直ですが、このままでは折れません)"}
          </span>
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
    <>
      <PaperActionEntrances />
      <p className="hint">
        左のツールを選んで操作します。山折り・谷折り・補助線: 2回クリックで線を引く(Escで中止)/
        選択: クリックまたはドラッグで選ぶ、点はドラッグで動かせる / Deleteキー:
        選択した線を削除 / 展開図をつかんで動かす:
        スペースキーを押しながらドラッグ、右ドラッグ、中ボタンドラッグのどれでも /{" "}
        {wheelBehavior === "scroll"
          ? "ホイールで上下、Shift+ホイールで左右、Ctrl+ホイールで拡大縮小"
          : "ホイールで拡大縮小、Ctrl+ホイールで上下、Ctrl+Shift+ホイールで左右"}
      </p>
      {/* 紙の色と方眼の数は、何も選んでいないときだけここに出す(PAP-003 / CPE-003) */}
      <PaperAppearance />
    </>
  );
}

export function ContextPanel() {
  const warnings = useAppStore((s) => s.warnings);
  const poseWarnings = useAppStore((s) => s.poseWarnings);
  const replayWarnings = useAppStore((s) => s.replayWarnings);
  const errorMessage = useAppStore((s) => s.errorMessage);
  const currentStep = useAppStore((s) => s.currentStep);
  const activeTool = useAppStore((s) => s.activeTool);
  const foldDraft = useAppStore((s) => s.foldDraft);
  const pendingFoldThrough = useAppStore((s) => s.pendingFoldThrough);
  const alignDraft = useAppStore((s) => s.alignDraft);
  const techniqueDraft = useAppStore((s) => s.techniqueDraft);
  // 同じ文言は1回だけ出す(展開図の検査結果には自動再生の警告も合流している)
  const allWarnings = uniqueWarnings(warnings, poseWarnings, replayWarnings);
  // 手順を選んでいる間はその手順の設定を出す(「折る前」「最新」は選択なし扱い)
  const stepSelected = currentStep !== null && currentStep >= 1;

  return (
    <footer className="context-panel">
      <div className="context-selection">
        <OperationSteps />
        {/* 手順を選んでいるときはその設定を優先する。折り線は「今見えている形」の
            上に引くものなので、手順を選んだ時点でストアが捨てている(ここは念のため) */}
        {pendingFoldThrough ? (
          <FoldThroughProposalContent pending={pendingFoldThrough} />
        ) : stepSelected ? (
          <StepContent number={currentStep} />
        ) : techniqueDraft ? (
          <TechniqueDraftContent draft={techniqueDraft} />
        ) : alignDraft ? (
          <AlignDraftContent draft={alignDraft} foldDraft={foldDraft} />
        ) : foldDraft ? (
          <FoldDraftContent draft={foldDraft} />
        ) : (
          <>
            {activeTool === "pull" ? <PullContent /> : <SelectionContent />}
            {LINE_TOOLS.includes(activeTool) && <CurveRow />}
            {activeTool === "fold" && <AlignStartRow />}
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
