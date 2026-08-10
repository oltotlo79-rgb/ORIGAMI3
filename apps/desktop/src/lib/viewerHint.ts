// 立体表示の上に常に出す「今できること」の1行案内(UI-009)。
// 画面から切り離した純粋な関数にして、文言だけをテストできるようにする。
// 言い回しは折り紙で使う言葉に寄せる(層→重なった紙、レイヤ選択→何枚折るか)。

import { ALIGN_STEPS, type AlignMode } from "./alignFold";
import { MIN_TWIST_VERTICES } from "./twistPolygon";
import type { TechniqueKind } from "./types";
import type { ToolId } from "../store/appStore";
import {
  minimumTechniqueFlap,
  techniqueUsesOpenToBack,
} from "./techniqueLayers";

/** 折れる状態かどうかを決める材料(canFoldNowと同じ条件を文章にするため) */
export interface FoldReadiness {
  hasDoc: boolean;
  playing: boolean;
  playT: number;
  driverCount: number;
  currentStep: number | null;
  stepCount: number;
}

/** つかんで折る操作の説明(修飾キーの意味は常に出す) */
export const DRAG_FOLD_HINT =
  "紙をつかんでドラッグすると折れます(Shift=重なった紙を全部、Alt=いちばん上の1枚だけ、Ctrl+ドラッグ=折り線を引いて下のパネルで決める)";

/**
 * 今は折れない理由(折れるならnull)。
 * appStoreのcanFoldNowと同じ条件を、同じ順で日本語にする。
 */
export function foldBlockReason(s: FoldReadiness): string | null {
  if (!s.hasDoc) return "紙がありません。上の「新規」で紙を出してください";
  if (s.playing) return "再生中は折れません。下の再生ボタンで止めてください";
  if (s.playT !== 1)
    return "折り途中の形では折れません。手順を最後まで進めてください";
  if (s.driverCount > 0)
    return "角度を動かして形を変えている間は折れません。下の「全て平らに戻す」で戻せます";
  // 途中の手順を見ている間も折れる(その手順の前へ挟まる。SEQ-006)
  return null;
}

/** 今どこへ折りが入るかの案内(手順の途中を見ているときだけ添える) */
export function insertPositionHint(s: FoldReadiness): string {
  if (s.currentStep === null || s.currentStep >= s.stepCount) return "";
  return `(折ると手順${s.currentStep + 1}の前に挟まります)`;
}

/** 紙をつかんで引く操作の説明(UI-007) */
export const PULL_HINT =
  "紙をドラッグすると、折り線のつじつまを合わせて全体が連動して動きます(右ドラッグで視点を回す)";

/** ヒント1行を組み立てる材料 */
export interface HintState extends FoldReadiness {
  tool: ToolId;
  /** 引く操作ができない理由(できるならnull) */
  pullBlocked: string | null;
  /** 今つかんで引いている最中か */
  pulling: boolean;
  /** 引いている折り線に左右対称の相手がいて、一緒に動かしているか(UI-007) */
  pullMirrored: boolean;
  /** 折り線を引いて確定待ちか */
  hasFoldDraft: boolean;
  /** 技法を選んでいるか */
  hasTechnique: boolean;
  /** 技法で選んだ重なりの枚数 */
  techniqueFlapCount: number;
  /** クリック地点に重なっていた候補層の枚数 */
  techniqueCandidateCount?: number;
  /** 技法の折り線を引いたか */
  hasTechniqueLine: boolean;
  /** 選んでいる技法の種類(選んでいなければnull)。ねじり折りだけ操作が違う */
  techniqueKind?: TechniqueKind | null;
  /** ねじり折りの中央多角形として置いた頂点の数 */
  techniqueVertexCount?: number;
  /** ねじり折りの中心を自分で指したか(指していなければ多角形の重心) */
  techniqueHasCenter?: boolean;
  /** 技法の行き先・先端などの基準点を自分で指したか */
  techniqueHasReference?: boolean;
  /** 「合わせて折る」の合わせ方(合わせモードでなければnull) */
  alignMode?: AlignMode | null;
  /** 合わせるために選び終えた対象の数 */
  alignPickCount?: number;
  /** 求まった折り線の本数(0なら折れない) */
  alignSolutionCount?: number;
  /** 折り線が求まらなかった理由(求まったならnull) */
  alignReason?: string | null;
}

/** 合わせて折るときに、次に何を選べばよいかの案内(選ぶ順にそのまま並べる) */
const ALIGN_PROMPTS: Record<AlignMode, string[]> = {
  throughTwoPoints: [
    "折り目が通る1つ目の点をクリックしてください(角・折り目の端・交点に吸着します)",
    "折り目が通る2つ目の点をクリックしてください",
  ],
  pointPoint: [
    "1つ目の点(動かす方)をクリックしてください(角・折り目の端・交点に吸着します)",
    "2つ目の点(合わせ先)をクリックしてください",
  ],
  lineLine: [
    "1つ目の線(動かす方)をクリックしてください(紙の辺・折り線を選べます)",
    "2つ目の線(合わせ先)をクリックしてください",
  ],
  pointPerpendicularLine: [
    "折り目が通る点をクリックしてください(角・折り目の端・交点に吸着します)",
    "折り目を垂直にする線をクリックしてください(紙の辺・折り線を選べます)",
  ],
  pointLineThrough: [
    "線に合わせたい点をクリックしてください",
    "合わせ先の線をクリックしてください",
    "折り目が通る点をクリックしてください",
  ],
  pointToLinePointToLine: [
    "線に合わせたい1つ目の点をクリックしてください",
    "1つ目の合わせ先の線をクリックしてください",
    "線に合わせたい2つ目の点をクリックしてください",
    "2つ目の合わせ先の線をクリックしてください",
  ],
  pointLinePerpendicular: [
    "線に合わせたい点をクリックしてください",
    "点の合わせ先の線をクリックしてください",
    "折り目を垂直にする線をクリックしてください",
  ],
  existingLine: [
    "折り目にする既存の線をクリックしてください(紙の辺・折り線を選べます)",
  ],
};

/** 選び直しの操作(選び始めたら常に添える) */
const ALIGN_KEYS = "(Backspaceで1つ戻す、Escでやめる)";

/** 「合わせて折る」の途中経過の案内(UI-009: 今すべきことを常に出す) */
export function alignHint(s: HintState): string {
  const mode = s.alignMode;
  if (!mode) return "";
  const picked = s.alignPickCount ?? 0;
  const prompts = ALIGN_PROMPTS[mode];
  const keys = picked > 0 ? ALIGN_KEYS : "";
  if (picked < ALIGN_STEPS[mode].length) return `${prompts[picked]}${keys}`;
  if (s.alignReason) return `${s.alignReason}${ALIGN_KEYS}`;
  const solutionCount = s.alignSolutionCount ?? 0;
  const other =
    solutionCount >= 2
      ? `解が${solutionCount}つあります。下のパネルの「別の解」で切り替えられます。`
      : "";
  return `折り線が決まりました。${other}下のパネルで山折り/谷折りを選んで「折る」を押してください${ALIGN_KEYS}`;
}

/** ねじり折りの中央多角形を指すときの案内(UI-009: 今すべきことを常に出す) */
export function twistHint(s: HintState): string {
  const n = s.techniqueVertexCount ?? 0;
  const center = s.techniqueHasCenter ? "中心は指定した点" : "中心は形の重心";
  if (n < MIN_TWIST_VERTICES)
    return `中央の形の角を順にクリックしてください(3つ以上。いま${n}個)。Ctrl+クリックで中心、Shift+クリックで対象層を指定、Backspaceで1つ戻す、Escでやめる`;
  return `中央の形を${n}角形で指しました(${center})。角を足すクリック、Ctrl+クリックで中心、Shift+クリックで対象層も指定できます。下のパネルでねじる角・向き・開く側を決めて「適用」を押してください`;
}

/** 立体表示に出す1行の案内。どのツールでも必ず何か返す(空にしない) */
export function viewerHint(s: HintState): string {
  const blocked = foldBlockReason(s);
  if (s.tool === "fold") {
    if (blocked) return `今は折れません: ${blocked}`;
    const where = insertPositionHint(s);
    // 合わせモードの間は、選ぶ途中経過を常に出す(折り線は選択から決まる)
    if (s.alignMode) return `${alignHint(s)}${where}`;
    if (s.hasFoldDraft)
      return `折り線を引きました。下のパネルで向きと動かす側を決めて「折る」を押してください(やり直すときは「やめる」)${where}`;
    return `${DRAG_FOLD_HINT}${where}`;
  }
  if (s.tool === "pull") {
    if (s.pullBlocked) return `今は引けません: ${s.pullBlocked}`;
    if (s.pulling)
      return s.pullMirrored
        ? "左右対称に動かしています。黄色い2本の折り線(左右の対)が同じ角度で一緒に動きます。離すとその形のまま残ります"
        : "引いている折り線を黄色で示しています。離すとその形のまま残ります";
    return PULL_HINT;
  }
  if (s.tool === "technique") {
    if (blocked) return `今は折れません: ${blocked}`;
    if (!s.hasTechnique) return "左の一覧から技法を選んでください";
    if (s.techniqueKind === "Simple") {
      return "層操作: 紙面をクリックして対象層、既存折り目をクリックして正確な開閉軸を選びます。重ね替え・山谷反転・同時操作は下のパネルで指定できます";
    }
    // ねじり折りは中央の多角形を頂点で指す(層は選ばなくてよい)
    if (s.techniqueKind === "Twist") return twistHint(s);
    // 古い呼び出しや技法未選択のテスト材料は、従来の一般的な案内を保つ。
    if (!s.techniqueKind) {
      if (s.techniqueFlapCount === 0)
        return "紙をクリックすると、その場所の重なりをまとめて選べます";
      if (!s.hasTechniqueLine)
        return `重なり${s.techniqueFlapCount}枚を選びました。続けて紙の上をドラッグして中心線を引いてください`;
      return "中心線を引きました。下のパネルで向きを決めて「適用」を押してください";
    }
    const minimum = minimumTechniqueFlap(s.techniqueKind);
    const candidates = s.techniqueCandidateCount ?? s.techniqueFlapCount;
    if (s.techniqueFlapCount < minimum) {
      if (candidates === 0)
        return `紙をクリックして候補層を出し、対象を${minimum}枚以上選んでください`;
      return `候補${candidates}枚のうち${s.techniqueFlapCount}枚を選択中です。下のパネルで対象を${minimum}枚以上にしてください`;
    }
    if (!s.hasTechniqueLine) {
      const layers =
        s.techniqueFlapCount === 0
          ? "層を選ばなければ、その領域の全ての層が対象です。"
          : `候補${candidates}枚のうち${s.techniqueFlapCount}枚を選びました。`;
      return `${layers}紙の上をドラッグして中心線を引いてください。Ctrl+クリックで基準点も直接指定できます`;
    }
    const reference = s.techniqueHasReference
      ? "基準点も指定しました。"
      : "必要ならCtrl+クリックで基準点を直接指定できます。";
    return techniqueUsesOpenToBack(s.techniqueKind)
      ? `中心線を引きました。${reference}下のパネルで向きと開く側を決めて「適用」を押してください`
      : `中心線を引きました。${reference}下のパネルで向きを決めて「適用」を押してください`;
  }
  return "ドラッグで回して見る、ホイールで拡大縮小、折り線をクリックで選択（Ctrl+クリックで追加・解除）。展開図で選んだ辺も黄色(折り目)・水色(縁や補助線)で表示します(折るときは左の「折る」)";
}
