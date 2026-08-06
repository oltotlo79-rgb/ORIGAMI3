// 立体表示の上に常に出す「今できること」の1行案内(UI-009)。
// 画面から切り離した純粋な関数にして、文言だけをテストできるようにする。
// 言い回しは折り紙で使う言葉に寄せる(層→重なった紙、レイヤ選択→何枚折るか)。

import type { ToolId } from "../store/appStore";

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
  /** 技法の折り線を引いたか */
  hasTechniqueLine: boolean;
}

/** 立体表示に出す1行の案内。どのツールでも必ず何か返す(空にしない) */
export function viewerHint(s: HintState): string {
  const blocked = foldBlockReason(s);
  if (s.tool === "fold") {
    if (blocked) return `今は折れません: ${blocked}`;
    const where = insertPositionHint(s);
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
    if (s.techniqueFlapCount === 0)
      return "紙をクリックすると、その場所の重なりをまとめて選べます";
    if (!s.hasTechniqueLine)
      return `重なり${s.techniqueFlapCount}枚を選びました。続けて紙の上をドラッグして中心線を引いてください`;
    return "中心線を引きました。下のパネルで向きを決めて「適用」を押してください";
  }
  return "ドラッグで回して見る、ホイールで拡大縮小、折り線をクリックで選択(折るときは左の「折る」)";
}
