// アプリ全体のキーボード操作で、文字編集を横取りしてはいけない対象の判定。

/** input / textarea / contenteditable 内で発生したキー操作か */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  if (target.closest("input, textarea")) return true;

  // contenteditable="false" は外側の編集可能領域を打ち消すので、近い祖先から見る。
  for (let element: Element | null = target; element; element = element.parentElement) {
    const value = element.getAttribute("contenteditable");
    if (value !== null) return value.toLowerCase() !== "false";
  }
  return false;
}
