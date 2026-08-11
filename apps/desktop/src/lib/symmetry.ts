// 展開図そのものから左右の対称軸を見つける計算(UI-007)。画面もIPCも触らない。
//
// 「左右同時に引く」には、つかんだ折り線と対になる折り線が要る。対称軸を紙の形から
// 決め打ちすると作品ごとに外れるので、**展開図の折り線の並びから軸を探す**。
//
// 手順は次の3段:
//   1. 紙の中心を通る候補の角度を有限個だけ作る(紙の縦・横・対角に加え、
//      折り線の向きの組から α=(θi+θj)/2 を作る。鏡映は向き θ を 2α−θ へ移すので、
//      軸になり得る角度は必ずこの形になる)
//   2. 各候補について、折り線を鏡映した先に折り線があるかを数えて一致率を出す
//   3. 一致率がしきい値以上の軸だけを残し、効かせたい順に並べる
//
// 実物の折り鶴・カエルの展開図は**どの軸でも完全には一致しない**(中割り折りの
// 頭など片側にしかない折り目があるため)。そこで完全一致ではなく一致率で判定する。
// 計算量を抑えるため、一致率は最大 SCORE_SAMPLE 本の折り線を等間隔に抜き出して測る。

import {
  MIRROR_EPS,
  isSameSegment,
  isValidMirrorLine,
  mirrorPoint,
  mirrorSegment,
  normalizedPaperSize,
  type MirrorLine,
  type Segment,
} from "./mirror";
import type { Paper, Vec2 } from "./types";

export type { MirrorLine } from "./mirror";

/** 直線axで折り返した点。許容幅を持たなかった既存APIどおり、有限点は厳密に移す。 */
export function reflectPoint(q: Vec2, ax: MirrorLine): Vec2 {
  return mirrorPoint(q, ax, 0);
}

/** 直線axで折り返した線分 */
export function reflectSegment(seg: Segment, ax: MirrorLine): Segment {
  return mirrorSegment(seg, ax, 0);
}

/** 紙の中心(正規化座標) */
export function paperCenter(paper: Paper): Vec2 {
  const [width, height] = normalizedPaperSize(paper);
  return [width / 2, height / 2];
}

/** 角度(度)から、紙の中心を通る対称軸を作る */
export function axisAt(paper: Paper, deg: number): MirrorLine {
  const r = ((Number.isFinite(deg) ? deg : 0) * Math.PI) / 180;
  return { p: paperCenter(paper), d: [Math.cos(r), Math.sin(r)] };
}

// ---------------------------------------------------------------------------
// 線分の索引(鏡映した線分の相手を素早く探す)
// ---------------------------------------------------------------------------

/** 索引の格子の1マスの大きさ(正規化座標)。許容誤差よりずっと大きく取る */
const CELL = 1e-3;

/** 折り線の索引。中点を格子に振り分けて、近いマスだけ調べる */
export interface SegmentIndex {
  items: [number, Segment][];
  cells: Map<string, number[]>;
}

function cellKey(x: number, y: number): string {
  return `${Math.round(x / CELL)},${Math.round(y / CELL)}`;
}

/** 折り線(ID付き)から索引を作る */
export function buildSegmentIndex(items: [number, Segment][]): SegmentIndex {
  const cells = new Map<string, number[]>();
  items.forEach(([, s], i) => {
    const key = cellKey((s[0][0] + s[1][0]) / 2, (s[0][1] + s[1][1]) / 2);
    const list = cells.get(key);
    if (list) list.push(i);
    else cells.set(key, [i]);
  });
  return { items, cells };
}

/** その線分と同じ位置にある折り線のID(無ければnull) */
export function findSegment(
  ix: SegmentIndex,
  seg: Segment,
  eps = MIRROR_EPS,
): number | null {
  const mx = (seg[0][0] + seg[1][0]) / 2;
  const my = (seg[0][1] + seg[1][1]) / 2;
  for (let dx = -1; dx <= 1; dx++) {
    for (let dy = -1; dy <= 1; dy++) {
      for (const i of ix.cells.get(cellKey(mx + dx * CELL, my + dy * CELL)) ?? []) {
        if (isSameSegment(ix.items[i][1], seg, eps)) return ix.items[i][0];
      }
    }
  }
  return null;
}

// ---------------------------------------------------------------------------
// 対称軸を探す
// ---------------------------------------------------------------------------

/** 候補の角度を作るのに使う「折り線の向き」の種類の上限(多い順) */
const DIR_LIMIT = 5;
/** 候補の角度の上限(これ以上は増やさない) */
const AXIS_LIMIT = 32;
/** 一致率を測るのに使う折り線の本数の上限(大きな展開図でも一定時間で済ませる) */
const SCORE_SAMPLE = 512;
/** 対称軸とみなす一致率の下限 */
export const MIN_SYMMETRY = 0.5;
/** 同じ角度とみなす差(度) */
const ANGLE_EPS = 1e-6;

/** 線分の向き(度。0以上180未満) */
function segmentAngle(s: Segment): number {
  const a = (Math.atan2(s[1][1] - s[0][1], s[1][0] - s[0][0]) * 180) / Math.PI;
  return ((a % 180) + 180) % 180;
}

/**
 * 対称軸になり得る角度の候補(度)。
 * 鏡映は向き θ を 2α−θ へ移すので、折り線の集合を保つ軸の角度 α は
 * かならず2つの向きの平均 (θi+θj)/2(または+90°)になる。多い向きだけを使って
 * 候補を有限個に抑える。紙の縦・横・対角も必ず入れる(折り目が少ない作品向け)。
 */
export function candidateAngles(paper: Paper, segs: Segment[]): number[] {
  const [width, height] = normalizedPaperSize(paper);
  const diag = (Math.atan2(height, width) * 180) / Math.PI;
  const out: number[] = [];
  const add = (deg: number) => {
    if (!Number.isFinite(deg)) return;
    const a = ((deg % 180) + 180) % 180;
    if (out.length < AXIS_LIMIT && !out.some((b) => Math.abs(a - b) < ANGLE_EPS)) out.push(a);
  };
  for (const a of [90, 0, diag, 180 - diag]) add(a);
  const count = new Map<number, number>();
  for (const s of segs) {
    const angle = segmentAngle(s);
    if (!Number.isFinite(angle)) continue;
    const k = Math.round(angle * 1e6) / 1e6;
    count.set(k, (count.get(k) ?? 0) + 1);
  }
  const top = [...count]
    .sort((x, y) => y[1] - x[1] || x[0] - y[0])
    .slice(0, DIR_LIMIT)
    .map(([a]) => a);
  for (let i = 0; i < top.length; i++) {
    for (let j = i; j < top.length; j++) {
      add((top[i] + top[j]) / 2);
      add((top[i] + top[j]) / 2 + 90);
    }
  }
  return out;
}

/** その軸で折り返したとき、折り線が元の折り線に重なる割合(0〜1) */
export function symmetryScore(ix: SegmentIndex, ax: MirrorLine, eps = MIRROR_EPS): number {
  const n = ix.items.length;
  if (n === 0 || !isValidMirrorLine(ax)) return 0;
  const step = Math.max(1, Math.ceil(n / SCORE_SAMPLE));
  let tried = 0;
  let hit = 0;
  for (let i = 0; i < n; i += step) {
    tried++;
    if (findSegment(ix, reflectSegment(ix.items[i][1], ax), eps) !== null) hit++;
  }
  return hit / tried;
}

/** その軸で折り返しても動かない多角形か(頂点の集合が元と同じ) */
export function keepsPolygon(poly: Vec2[], ax: MirrorLine, eps = MIRROR_EPS): boolean {
  if (!isValidMirrorLine(ax)) return false;
  const tolerance = Number.isFinite(eps) && eps >= 0 ? eps : MIRROR_EPS;
  return poly.every((q) => {
    const o = reflectPoint(q, ax);
    return poly.some(
      (p) =>
        Math.abs(p[0] - o[0]) <= tolerance &&
        Math.abs(p[1] - o[1]) <= tolerance,
    );
  });
}

/**
 * 展開図の対称軸を、効かせたい順に返す。
 *
 * 並べ方は「根の面(ソルバーが動かさない基準の面)を保つ軸が先、次に一致率の高い順」。
 * 根の面が動く軸で対にすると、左右が同じようには動かない。折り鶴では紙の対角線
 * (羽どうしを入れ替える軸)だけが根の面を保ち、一致率がより高い別の対角線
 * (同じ羽の層どうしを入れ替えてしまう軸)より先に来る。
 */
export function findMirrorAxes(
  paper: Paper,
  ix: SegmentIndex,
  rootPolygon: Vec2[] = [],
  eps = MIRROR_EPS,
): MirrorLine[] {
  const segs = ix.items.map(([, s]) => s);
  const scored = candidateAngles(paper, segs)
    .map((deg) => axisAt(paper, deg))
    .map((ax) => ({ ax, score: symmetryScore(ix, ax, eps), keeps: keepsPolygon(rootPolygon, ax, eps) }))
    .filter((x) => x.score >= MIN_SYMMETRY);
  scored.sort((a, b) => Number(b.keeps) - Number(a.keeps) || b.score - a.score);
  return scored.map((x) => x.ax);
}
