// 名前付き技法に閉じない汎用層操作を、画面の下書きからRustのserde形へ変換する。
// 既存折り目の開閉と重ね替えは新しい折り線を作らないため、regionは常に空。

import type {
  FoldDirection,
  LayerTurn,
  MotionPart,
  Vec2,
} from "./types";

export type LayerMotionMode = "reflect" | "stay";
export type LayerTurnMode = "Keep" | "Outside" | "Inside" | "Beside";

export interface LayerMotionPartDraft {
  layers: number[];
  line: [Vec2, Vec2] | null;
  mode: LayerMotionMode;
  turn: LayerTurnMode;
  direction: FoldDirection;
  anchor: number;
  reverseLayers: boolean;
}

export type LayerMotionBuildResult =
  | { ok: true; part: MotionPart }
  | { ok: false; error: string };

function validLine(line: [Vec2, Vec2]): boolean {
  return Math.hypot(line[1][0] - line[0][0], line[1][1] - line[0][1]) > 1e-9;
}

function uniqueLayers(layers: readonly number[]): number[] | null {
  if (layers.some((id) => !Number.isInteger(id) || id < 0)) return null;
  return [...new Set(layers)];
}

function layerTurn(draft: LayerMotionPartDraft): LayerTurn | null {
  if (draft.turn === "Keep") return "Keep";
  if (draft.turn === "Outside") return { Outside: draft.direction };
  if (draft.turn === "Inside") return { Inside: draft.direction };
  if (!Number.isInteger(draft.anchor) || draft.anchor < 0) return null;
  return { Beside: { anchor: draft.anchor, direction: draft.direction } };
}

/** 下書きに、キューへ足すべき現在partの入力があるか。 */
export function hasLayerMotionInput(draft: LayerMotionPartDraft): boolean {
  return (
    draft.layers.length > 0 ||
    draft.line !== null ||
    draft.reverseLayers ||
    (draft.mode === "stay" && draft.turn !== "Keep")
  );
}

/** UI下書きをRustのMotionPartへ変換する。定義できない指定は日本語で返す。 */
export function buildLayerMotionPart(
  draft: LayerMotionPartDraft,
): LayerMotionBuildResult {
  const layers = uniqueLayers(draft.layers);
  if (layers === null) return { ok: false, error: "対象層の指定が正しくありません" };

  if (draft.mode === "reflect") {
    if (draft.line === null || !validLine(draft.line)) {
      return {
        ok: false,
        error: "立体表示で既存の折り目をクリックして、開閉の軸を選んでください",
      };
    }
    return {
      ok: true,
      part: {
        layers,
        region: [],
        transform: { Reflect: [draft.line] },
        turn: "Keep",
        ...(draft.reverseLayers ? { reverse_layers: true } : {}),
      },
    };
  }

  if (draft.turn === "Keep" && !draft.reverseLayers) {
    return {
      ok: false,
      error: "重ね方を選ぶか、選択層だけ山谷を反転してください",
    };
  }
  const turn = layerTurn(draft);
  if (turn === null) {
    return { ok: false, error: "隣へ置く基準面IDを0以上の整数で指定してください" };
  }
  return {
    ok: true,
    part: {
      layers,
      region: [],
      transform: "Stay",
      turn,
      ...(draft.reverseLayers ? { reverse_layers: true } : {}),
    },
  };
}

/** 追加済みpartを短い日本語で表示する。 */
export function describeLayerMotionPart(part: MotionPart): string {
  const layers = part.layers.length === 0 ? "全層" : `${part.layers.length}層`;
  const reverse = part.reverse_layers ? "・山谷反転" : "";
  if (part.transform !== "Stay") return `${layers}を折り目で開閉${reverse}`;
  if (part.turn === "Keep") {
    return part.reverse_layers ? `${layers}だけ山谷反転` : `${layers}の位置を保持`;
  }
  if ("Outside" in part.turn) {
    return `${layers}を全体の${part.turn.Outside === "Up" ? "手前" : "奥"}へ${reverse}`;
  }
  if ("Inside" in part.turn) {
    return `${layers}を元の紙の${part.turn.Inside === "Up" ? "手前隣" : "奥隣"}へ${reverse}`;
  }
  return `${layers}を面${part.turn.Beside.anchor}の${part.turn.Beside.direction === "Up" ? "手前隣" : "奥隣"}へ${reverse}`;
}
