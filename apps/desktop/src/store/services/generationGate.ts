interface GenerationGate {
  /** 新しい要求世代を発行し、それ以前のtokenを無効にする。 */
  issue: () => number;
  /** tokenが最後に発行した要求だけに属するかを調べる。 */
  isCurrent: (token: number) => boolean;
  /** 現在値を公開せず、比較が必要な所有moduleだけで使う。 */
  current: () => number;
}

/**
 * 非同期domainごとの単調増加世代。
 * instanceを共有しないことで、fold-through・一斉表示・提案などを互いに
 * 無効化しない。queueのisLatestやdocumentのdocEpochとは別の関門である。
 */
export function createGenerationGate(initialGeneration = 0): GenerationGate {
  let generation = initialGeneration;
  return {
    issue: () => ++generation,
    isCurrent: (token) => token === generation,
    current: () => generation,
  };
}
