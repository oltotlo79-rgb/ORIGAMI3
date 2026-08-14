// 骨格の編集(PRO-001)の純関数テスト:
// 任意の親への追加、先端数の上下限、子孫削除、表示順、プレビュー配置。

import { describe, expect, it } from "vitest";
import {
  MAX_LIMBS,
  MIN_LIMBS,
  ROOT_ID,
  addLimb,
  canAddLimb,
  canRemoveLimb,
  defaultSkeleton,
  leafNodes,
  limbLabel,
  limbs,
  previewLayout,
  removeLimb,
  setLimb,
  skeletonRows,
} from "./skeleton";
import type { Skeleton } from "./types";

function seededRandom(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (Math.imul(state, 1_664_525) + 1_013_904_223) >>> 0;
    return state / 0x1_0000_0000;
  };
}

describe("骨格の編集", () => {
  it("初期状態は根1つ+出っぱり4本", () => {
    const s = defaultSkeleton();
    expect(limbs(s)).toHaveLength(4);
    expect(s.nodes.filter((n) => n.parent === null)).toHaveLength(1);
  });

  it("増やすと本数が1つ増え、IDは重ならない", () => {
    const s = addLimb(defaultSkeleton());
    expect(limbs(s)).toHaveLength(5);
    expect(s.nodes[s.nodes.length - 1].parent).toBe(ROOT_ID);
    expect(new Set(s.nodes.map((n) => n.id)).size).toBe(s.nodes.length);
  });

  it("指定した出っぱりの先と、そのさらに先へ足せる", () => {
    let s = defaultSkeleton();
    const headId = skeletonRows(s)[0].node.id;

    s = addLimb(s, headId);
    const next = s.nodes.find((n) => n.parent === headId);
    expect(next?.parent).toBe(headId);

    s = addLimb(s, next!.id);
    const further = s.nodes.find((n) => n.parent === next!.id);
    expect(further?.parent).toBe(next!.id);
    expect(skeletonRows(s).find((row) => row.node.id === further?.id)?.depth).toBe(
      3,
    );
    // 先端をまっすぐ延ばしただけなので、本当の先端数は4本のまま。
    expect(leafNodes(s)).toHaveLength(4);
  });

  it("先端を延ばすと太さを受け継ぎ、途中からの新しい分岐は既定の太さになる", () => {
    let s = setLimb(defaultSkeleton(), 1, { width_factor: 2 });
    s = addLimb(s, 1);
    const inherited = s.nodes.find((n) => n.parent === 1)!;
    expect(inherited.width_factor).toBe(2);

    s = addLimb(s, 1);
    const branches = s.nodes.filter((n) => n.parent === 1);
    expect(branches).toHaveLength(2);
    expect(branches[1].width_factor).toBe(1);
  });

  it("親のまとまりの直後へ安定して挿入し、表示名は親子ごとに付く", () => {
    let s = defaultSkeleton();
    const headId = 1;
    s = addLimb(s, headId); // 5: 頭のその先1
    s = addLimb(s, 5); // 6: そのさらに先
    s = addLimb(s, headId); // 7: 頭のその先2

    expect(s.nodes.map((n) => n.id)).toEqual([0, 1, 5, 6, 7, 2, 3, 4]);
    expect(
      skeletonRows(s).map(({ node, depth, label }) => [node.id, depth, label]),
    ).toEqual([
      [1, 1, "頭"],
      [5, 2, "その先1"],
      [6, 3, "その先1"],
      [7, 2, "その先2"],
      [2, 1, "尾"],
      [3, 1, "右前足"],
      [4, 1, "左前足"],
    ]);
  });

  it("上限12本を超えて増えない", () => {
    let s = defaultSkeleton();
    for (let i = 0; i < 20; i++) s = addLimb(s);
    expect(limbs(s)).toHaveLength(MAX_LIMBS);
  });

  it("先端12本でも延長はでき、13本目になる分岐だけを止める", () => {
    let s = defaultSkeleton();
    while (leafNodes(s).length < MAX_LIMBS) s = addLimb(s);
    const oldTip = leafNodes(s)[0];

    expect(canAddLimb(s, oldTip.id)).toBe(true);
    s = addLimb(s, oldTip.id);
    const newTip = leafNodes(s).find((n) => n.parent === oldTip.id);
    expect(newTip).toBeDefined();
    expect(leafNodes(s)).toHaveLength(MAX_LIMBS);

    // oldTipは途中になったため、ここでさらに分けると13本になる。
    expect(canAddLimb(s, oldTip.id)).toBe(false);
    expect(addLimb(s, oldTip.id)).toBe(s);
    expect(canAddLimb(s, ROOT_ID)).toBe(false);

    // 新しい先端をさらに延ばすだけなら、まだ12本のままなので許可する。
    expect(canAddLimb(s, newTip!.id)).toBe(true);
    const extended = addLimb(s, newTip!.id);
    expect(leafNodes(extended)).toHaveLength(MAX_LIMBS);
  });

  it("減らすと本数が1つ減り、下限1本で止まる", () => {
    let s = defaultSkeleton();
    s = removeLimb(s, limbs(s)[0].id);
    expect(limbs(s)).toHaveLength(3);
    for (let i = 0; i < 10; i++) s = removeLimb(s, limbs(s)[0].id);
    expect(limbs(s)).toHaveLength(MIN_LIMBS);
    // 根は消さない(消すとRust側の検査で「根がちょうど1つ」に反する)
    expect(s.nodes.filter((n) => n.parent === null)).toHaveLength(1);
  });

  it("根は削除できない", () => {
    const s = defaultSkeleton();
    expect(removeLimb(s, 0)).toBe(s);
  });

  it("途中を消すとその先も全て消え、存在しない親への参照を0件にする", () => {
    let s = defaultSkeleton();
    s = addLimb(s, 1); // 5
    s = addLimb(s, 5); // 6
    s = addLimb(s, 1); // 7

    const removed = removeLimb(s, 1);
    const remainingIds = new Set(removed.nodes.map((n) => n.id));
    expect(removed.nodes.filter((n) => [1, 5, 6, 7].includes(n.id))).toHaveLength(
      0,
    );
    expect(
      removed.nodes.filter(
        (n) => n.parent !== null && !remainingIds.has(n.parent),
      ),
    ).toHaveLength(0);
    expect(leafNodes(removed)).toHaveLength(3);
  });

  it("無作為な追加・削除1,000回の全段階で壊れた形を作らない", () => {
    const seed = 0xdecafbad;
    const random = seededRandom(seed);
    const operations: string[] = [];
    const passed = { singleBody: 0, noMissingParent: 0, validTipCount: 0 };
    let added = 0;
    let removed = 0;
    let minTips = Number.POSITIVE_INFINITY;
    let maxTips = Number.NEGATIVE_INFINITY;
    let maxDepth = 0;
    let skeleton = defaultSkeleton();

    for (let step = 1; step <= 1_000; step += 1) {
      let operation: "add" | "remove" = random() < 0.5 ? "add" : "remove";
      let candidates = skeleton.nodes.filter((node) =>
        operation === "add"
          ? canAddLimb(skeleton, node.id)
          : canRemoveLimb(skeleton, node.id),
      );
      if (candidates.length === 0) {
        operation = operation === "add" ? "remove" : "add";
        candidates = skeleton.nodes.filter((node) =>
          operation === "add"
            ? canAddLimb(skeleton, node.id)
            : canRemoveLimb(skeleton, node.id),
        );
      }

      const target = candidates[Math.floor(random() * candidates.length)];
      const before = skeleton;
      skeleton =
        operation === "add"
          ? addLimb(skeleton, target.id)
          : removeLimb(skeleton, target.id);
      operations.push(`${step}:${operation}(${target.id})`);
      if (operation === "add") added += 1;
      else removed += 1;

      const ids = new Set(skeleton.nodes.map((node) => node.id));
      const bodyCount = skeleton.nodes.filter(
        (node) => node.parent === null,
      ).length;
      const missingParentCount = skeleton.nodes.filter(
        (node) => node.parent !== null && !ids.has(node.parent),
      ).length;
      const tipCount = leafNodes(skeleton).length;
      const failures = [
        bodyCount === 1 ? null : `胴の数=${bodyCount}`,
        missingParentCount === 0
          ? null
          : `親が見つからない出っぱり=${missingParentCount}`,
        tipCount >= MIN_LIMBS && tipCount <= MAX_LIMBS
          ? null
          : `本当の先端=${tipCount}`,
        skeleton === before ? "操作後の状態が変化していない" : null,
        ids.size === skeleton.nodes.length ? null : "IDが重複している",
      ].filter((failure): failure is string => failure !== null);
      if (failures.length > 0) {
        throw new Error(
          [
            `seed=${seed}, ${step}/1000: ${failures.join(", ")}`,
            `操作列=${JSON.stringify(operations)}`,
            `状態=${JSON.stringify(skeleton.nodes)}`,
          ].join("\n"),
        );
      }

      passed.singleBody += 1;
      passed.noMissingParent += 1;
      passed.validTipCount += 1;
      minTips = Math.min(minTips, tipCount);
      maxTips = Math.max(maxTips, tipCount);
      maxDepth = Math.max(
        maxDepth,
        ...skeletonRows(skeleton).map((row) => row.depth),
      );
    }

    expect(passed).toEqual({
      singleBody: 1_000,
      noMissingParent: 1_000,
      validTipCount: 1_000,
    });
    expect(added + removed).toBe(1_000);
    expect(added).toBeGreaterThan(0);
    expect(removed).toBeGreaterThan(0);
    expect(minTips).toBe(MIN_LIMBS);
    expect(maxTips).toBe(MAX_LIMBS);
    expect(maxDepth).toBeGreaterThanOrEqual(3);
  });

  it("鎖1本の末端は消せて親が新しい先端になり、最後の1本は消せない", () => {
    const chain: Skeleton = {
      nodes: [
        { id: 0, parent: null, length: 0, width_factor: 1 },
        { id: 1, parent: 0, length: 1, width_factor: 1 },
        { id: 2, parent: 1, length: 1, width_factor: 1 },
      ],
    };

    expect(canRemoveLimb(chain, 2)).toBe(true);
    const shortened = removeLimb(chain, 2);
    expect(leafNodes(shortened).map((n) => n.id)).toEqual([1]);
    expect(canRemoveLimb(shortened, 1)).toBe(false);
    expect(removeLimb(shortened, 1)).toBe(shortened);
  });

  it("長さと太さを書き換えられる(他の出っぱりは変わらない)", () => {
    const s0 = defaultSkeleton();
    const id = limbs(s0)[1].id;
    const s = setLimb(s0, id, { length: 2.5, width_factor: 0.4 });
    const changed = s.nodes.find((n) => n.id === id);
    expect(changed?.length).toBe(2.5);
    expect(changed?.width_factor).toBe(0.4);
    expect(limbs(s)[0]).toEqual(limbs(s0)[0]);
  });

  it("従来の放射状プレビューを保ち、太いほど先端の丸が大きい", () => {
    const s = setLimb(defaultSkeleton(), limbs(defaultSkeleton())[0].id, {
      width_factor: 2,
    });
    const layout = previewLayout(s);
    expect(layout).toHaveLength(4);
    // 先頭は真上(y>0, x≒0)
    expect(layout[0].start).toEqual([0, 0]);
    expect(layout[0].end[1]).toBeGreaterThan(0);
    expect(Math.abs(layout[0].end[0])).toBeLessThan(1e-9);
    expect(layout[0].radius).toBeGreaterThan(layout[1].radius);
  });

  it("見本の子は親の終点から始まり、深い線は原点から始まらない", () => {
    let s = defaultSkeleton();
    s = addLimb(s, 1); // 5
    s = addLimb(s, 5); // 6
    const byId = new Map(previewLayout(s).map((layout) => [layout.id, layout]));
    const head = byId.get(1)!;
    const next = byId.get(5)!;
    const further = byId.get(6)!;

    expect(next.start).toEqual(head.end);
    expect(next.start).not.toEqual([0, 0]);
    expect(further.start).toEqual(next.end);
    expect(further.start).not.toEqual([0, 0]);
    expect(Math.hypot(next.end[0] - next.start[0], next.end[1] - next.start[1])).toBeCloseTo(
      1,
      12,
    );
    expect(next.label).toBe("その先1");
  });

  it("呼び名は専門用語を使わない", () => {
    expect(limbLabel(0)).toBe("頭");
    expect(limbLabel(11)).toBe("出っぱり12");
  });
});
