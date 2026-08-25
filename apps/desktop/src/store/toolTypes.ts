/** 道具の公開ID。importを持たない末端に置き、表示案内とstoreの循環を防ぐ。 */
export type ToolId =
  | "select"
  | "measure"
  | "mountain"
  | "valley"
  | "aux"
  | "delete"
  | "fold"
  | "pull"
  | "technique"
  | "construct";
