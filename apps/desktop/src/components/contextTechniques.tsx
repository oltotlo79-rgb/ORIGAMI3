import { useAppStore, type TechniqueDraft } from "../store/appStore";
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
import { TECHNIQUE_LABEL } from "../lib/techniques";
import type { TechniqueKind } from "../lib/types";
import { NumberInput } from "./contextAngleSteps";

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
          候補{candidates.length}枚（奥から手前の順） / 選択
          {draft.flap.length}枚
        </span>
        <label htmlFor="technique-layer-count">選ぶ枚数</label>
        <NumberInput
          id="technique-layer-count"
          ariaLabel="選ぶ層の枚数"
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
          手前から{draft.flapPickCount}枚
        </button>
        <button type="button" disabled={!hasCandidates} onClick={() => setPreset("back")}>
          奥から{draft.flapPickCount}枚
        </button>
        <button
          type="button"
          disabled={!hasCandidates}
          onClick={() => setPreset("frontNth")}
        >
          手前から{draft.flapPickCount}枚目
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
                  奥から{fromBack}枚目 / 手前から{fromFront}枚目
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
  const motionAnchorIsVisible = draft.flapCandidates.includes(
    draft.motionAnchor,
  );
  const current = {
    layers: draft.flap,
    line: draft.line,
    mode: draft.motionMode,
    turn: draft.motionTurn,
    direction: draft.motionDirection,
    // 「隣へ置く面」は候補から選んだときだけ有効にする。内部の面番号を
    // 利用者へ入力させず、奥・手前の順から選べるようにする。
    anchor:
      draft.motionTurn === "Beside" && !motionAnchorIsVisible
        ? -1
        : draft.motionAnchor,
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
        <span className="row-label">操作:</span>
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
            軸: {draft.motionAxisEdgeId === null ? "未選択" : "選んだ折り目"}
          </span>
        </div>
      ) : (
        <div className="button-row">
          <label htmlFor="layer-motion-turn">重ね方</label>
          <select
            id="layer-motion-turn"
            value={draft.motionTurn}
            onChange={(e) => {
              const motionTurn = e.target.value as LayerTurnMode;
              updateTechniqueDraft({
                motionTurn,
                // 内部の既定値0を暗黙に面0の選択として扱わない。
                // 「指定面」を選んだら、候補一覧から必ず明示してもらう。
                ...(motionTurn === "Beside" ? { motionAnchor: -1 } : {}),
              });
            }}
          >
            <option value="Keep">位置を保つ</option>
            <option value="Outside">重なり全体の外側</option>
            <option value="Inside">元の紙のすぐ隣</option>
            <option value="Beside">指定面のすぐ隣</option>
          </select>
          {draft.motionTurn === "Beside" && (
            <>
              <label htmlFor="layer-motion-anchor">隣に置く面</label>
              <select
                id="layer-motion-anchor"
                aria-label="隣に置く面"
                value={motionAnchorIsVisible ? draft.motionAnchor : ""}
                onChange={(event) =>
                  updateTechniqueDraft({
                    motionAnchor: Number(event.target.value),
                  })
                }
              >
                <option value="" disabled>
                  面を選んでください
                </option>
                {draft.flapCandidates.map((face, index) => (
                  <option key={face} value={face}>
                    奥から{index + 1}枚目 / 手前から
                    {draft.flapCandidates.length - index}枚目
                  </option>
                ))}
              </select>
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
export function TechniqueDraftContent({ draft }: { draft: TechniqueDraft }) {
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
        <span className="row-label">
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
          <span className="row-label">開く側:</span>
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
