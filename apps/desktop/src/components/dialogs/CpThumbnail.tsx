// 展開図の小さな見本(提案の候補選び用)。線種の色はCpEditorと同じものを使い、
// 2D区画で見えるものと同じ配色にする。

import { EDGE_COLORS } from "../CpEditor/renderer";
import type { CreasePattern } from "../../lib/types";

/** 紙の輪郭から表示範囲を求める(頂点が無ければ1×1) */
function extent(cp: CreasePattern): [number, number] {
  const xs = cp.vertices.map((v) => v.pos[0]);
  const ys = cp.vertices.map((v) => v.pos[1]);
  return [Math.max(...xs, 1e-6), Math.max(...ys, 1e-6)];
}

export function CpThumbnail({ cp }: { cp: CreasePattern }) {
  const [w, h] = extent(cp);
  const pos = new Map(cp.vertices.map((v) => [v.id, v.pos]));
  const stroke = Math.max(w, h) * 0.006;
  // SVGはy軸が下向きなので、紙のyを反転して展開図の上下を2D区画と揃える
  return (
    <svg
      className="cp-thumb"
      viewBox={`0 0 ${w} ${h}`}
      role="img"
      aria-label="展開図の見本"
    >
      <rect x={0} y={0} width={w} height={h} fill="#ffffff" />
      {cp.edges.map((e) => {
        const a = pos.get(e.v0);
        const b = pos.get(e.v1);
        if (!a || !b) return null;
        return (
          <line
            key={e.id}
            x1={a[0]}
            y1={h - a[1]}
            x2={b[0]}
            y2={h - b[1]}
            stroke={EDGE_COLORS[e.kind]}
            strokeWidth={stroke}
          />
        );
      })}
    </svg>
  );
}
