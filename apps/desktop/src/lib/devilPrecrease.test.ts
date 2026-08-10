// 悪魔の手順1〜16で作る予備折りを、藤田・羽鳥の基本作図から再現できることを固定する。

import { describe, expect, it } from "vitest";
import { distanceToLine, solveAlign, type FoldLine } from "./alignFold";
import type { EdgeKind, Vec2 } from "./types";

const TOLERANCE = 1e-12;

// 全て Q(√2) の点。紙の正規化座標は左上(0,0)、右下(1,1)。
const sqrt2 = Math.SQRT2;
const t = sqrt2 - 1;
const q = 2 - sqrt2;
const s = 2 * t;
const e = 2 * q - 1;
const a = sqrt2 / 4;
const b = (2 + sqrt2) / 4;
const k = 4 * t - 1;

function point(x: number, y: number): Vec2 {
  return [x, y];
}

const landmarks = {
  p00: point(0, 0),
  p10: point(1, 0),
  p11: point(1, 1),
  p01: point(0, 1),
  leftQ: point(0, q),
  bottomQ: point(q, 0),
  rightQ: point(1, q),
  topQ: point(q, 1),
  leftT: point(0, t),
  bottomT: point(t, 0),
  rightT: point(1, t),
  topT: point(t, 1),
  leftS: point(0, s),
  bottomS: point(s, 0),
  topE: point(e, 1),
  rightE: point(1, e),
  leftA: point(0, a),
  bottomA: point(a, 0),
  leftB: point(0, b),
  bottomB: point(b, 0),
  leftK: point(0, k),
  bottomK: point(k, 0),
  leftHalf: point(0, 0.5),
  bottomHalf: point(0.5, 0),
} satisfies Record<string, Vec2>;

interface DevilPrecrease {
  id: `C${string}`;
  line: FoldLine;
  kind: EdgeKind;
}

const precreases: DevilPrecrease[] = [
  { id: "C01", line: [landmarks.p00, landmarks.p11], kind: "Valley" },
  { id: "C02", line: [landmarks.p10, landmarks.p01], kind: "Aux" },
  { id: "C03", line: [landmarks.p11, landmarks.leftQ], kind: "Aux" },
  { id: "C04", line: [landmarks.p11, landmarks.bottomQ], kind: "Aux" },
  { id: "C05", line: [landmarks.p01, landmarks.rightQ], kind: "Aux" },
  { id: "C06", line: [landmarks.p10, landmarks.topQ], kind: "Aux" },
  { id: "C07", line: [landmarks.leftQ, landmarks.rightQ], kind: "Aux" },
  { id: "C08", line: [landmarks.bottomQ, landmarks.topQ], kind: "Aux" },
  { id: "C09", line: [landmarks.leftT, landmarks.topQ], kind: "Aux" },
  { id: "C10", line: [landmarks.bottomT, landmarks.rightQ], kind: "Aux" },
  { id: "C11", line: [landmarks.leftT, landmarks.bottomT], kind: "Aux" },
  { id: "C12", line: [landmarks.topQ, landmarks.rightQ], kind: "Aux" },
  { id: "C13", line: [landmarks.bottomS, landmarks.topT], kind: "Aux" },
  { id: "C14", line: [landmarks.leftS, landmarks.rightT], kind: "Aux" },
  { id: "C15", line: [landmarks.bottomQ, landmarks.topE], kind: "Aux" },
  { id: "C16", line: [landmarks.leftQ, landmarks.rightE], kind: "Aux" },
  { id: "C17", line: [landmarks.leftS, landmarks.bottomS], kind: "Aux" },
  { id: "C18", line: [landmarks.topE, landmarks.rightE], kind: "Aux" },
  { id: "C19", line: [landmarks.leftA, landmarks.bottomB], kind: "Aux" },
  { id: "C20", line: [landmarks.bottomA, landmarks.leftB], kind: "Aux" },
  { id: "C21", line: [landmarks.leftK, landmarks.bottomK], kind: "Aux" },
  { id: "C22", line: [landmarks.leftHalf, landmarks.bottomHalf], kind: "Aux" },
];

function lineDistance(actual: FoldLine, expected: FoldLine): number {
  return Math.max(
    distanceToLine(actual, expected[0]),
    distanceToLine(actual, expected[1]),
    distanceToLine(expected, actual[0]),
    distanceToLine(expected, actual[1]),
  );
}

function linePick(line: FoldLine) {
  return { kind: "line" as const, a: line[0], b: line[1] };
}

describe("悪魔の手順1〜16の予備折り", () => {
  it("22本を基本作図1（2点を通る）で誤差1e-12以内に再構成できる", () => {
    for (const crease of precreases) {
      const solution = solveAlign("throughTwoPoints", [
        { kind: "point", p: crease.line[0] },
        { kind: "point", p: crease.line[1] },
      ]);
      expect(solution.reason, crease.id).toBeNull();
      expect(solution.lines, crease.id).toHaveLength(1);
      expect(lineDistance(solution.lines[0], crease.line), crease.id).toBeLessThanOrEqual(
        TOLERANCE,
      );
    }
  });

  it("C01だけが谷折りで、C02〜C22は作図補助線である", () => {
    expect(precreases.map((crease) => crease.id)).toEqual(
      Array.from({ length: 22 }, (_, index) => `C${String(index + 1).padStart(2, "0")}`),
    );
    expect(precreases[0].kind).toBe("Valley");
    expect(precreases.slice(1).every((crease) => crease.kind === "Aux")).toBe(true);
  });

  it("C03〜C06は対角線と紙端の角の二等分線としても再構成できる", () => {
    const byId = new Map(precreases.map((crease) => [crease.id, crease.line]));
    const diagonal1 = byId.get("C01")!;
    const diagonal2 = byId.get("C02")!;
    const top: FoldLine = [landmarks.p01, landmarks.p11];
    const right: FoldLine = [landmarks.p10, landmarks.p11];
    const origins: [DevilPrecrease["id"], FoldLine, FoldLine][] = [
      ["C03", top, diagonal1],
      ["C04", right, diagonal1],
      ["C05", top, diagonal2],
      ["C06", right, diagonal2],
    ];

    for (const [id, edge, diagonal] of origins) {
      const solution = solveAlign("lineLine", [linePick(edge), linePick(diagonal)]);
      expect(solution.reason, id).toBeNull();
      expect(solution.lines, id).toHaveLength(2);
      const error = Math.min(...solution.lines.map((line) => lineDistance(line, byId.get(id)!)));
      expect(error, id).toBeLessThanOrEqual(TOLERANCE);
    }
  });
});
