import { describe, expect, it } from "vitest";
import { createGenerationGate } from "./services/generationGate";

describe("generation gate", () => {
  it("invalidates every older token without reusing it", () => {
    const gate = createGenerationGate();
    const first = gate.issue();
    expect(gate.isCurrent(first)).toBe(true);

    const second = gate.issue();
    expect(second).toBeGreaterThan(first);
    expect(gate.isCurrent(first)).toBe(false);
    expect(gate.isCurrent(second)).toBe(true);

    const issued = Array.from({ length: 1_000 }, () => gate.issue());
    expect(new Set([first, second, ...issued]).size).toBe(1_002);
    expect(gate.isCurrent(issued[issued.length - 1]!)).toBe(true);
  });

  it("keeps unrelated asynchronous domains independent", () => {
    const documentGate = createGenerationGate();
    const proposalGate = createGenerationGate();
    const documentToken = documentGate.issue();
    const proposalToken = proposalGate.issue();

    documentGate.issue();

    expect(documentGate.isCurrent(documentToken)).toBe(false);
    expect(proposalGate.isCurrent(proposalToken)).toBe(true);
  });
});
