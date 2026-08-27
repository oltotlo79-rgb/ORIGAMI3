/** 保存しない3D合わせ入力・解だけが使うworld座標。 */
export type SpatialVec3 = [number, number, number];

/** 点または線を載せられる3D支持面。 */
export interface SpatialSupportPlane {
  point: SpatialVec3;
  normal: SpatialVec3;
}

/** 3D表示で選んだ、保存しない合わせ対象。 */
export type SpatialAlignTarget =
  | {
      kind: "point";
      world: SpatialVec3;
      supportPlanes: SpatialSupportPlane[];
      /** raw Frame3D上でもz=0面へ一意に写せた点。表示層offsetからの推測はしない。 */
      foldedPoint: [number, number] | null;
    }
  | {
      kind: "line";
      aWorld: SpatialVec3;
      bWorld: SpatialVec3;
      supportPlanes: SpatialSupportPlane[];
      /** raw Frame3D上でもz=0面へ一意に写せた線。非平坦ならnull。 */
      foldedLine: [[number, number], [number, number]] | null;
    };

/** 共通3D平面で解き、同じ面内へclipした保存しない折り線。 */
export interface SpatialFoldTarget {
  lineWorld: [SpatialVec3, SpatialVec3];
  keepWorldForMovingSide: {
    left: SpatialVec3 | null;
    right: SpatialVec3 | null;
  };
  /**
   * 表示用共通面がglobal z軸に平行で、かつ全入力がraw Frame3Dのz=0面へ
   * 一意に戻せた場合だけ持つ。表示層offsetやworld端点のzからは推測せず、
   * 非平坦面をglobal XYへ補う用途には使わない。
   */
  foldedPlane: {
    line: [[number, number], [number, number]];
    keepPointForMovingSide: {
      left: [number, number] | null;
      right: [number, number] | null;
    };
  } | null;
  /** 同じ局所chart上の解線と1つ目の入力から決めた、保存しない初期side。 */
  sideForFirstPick: {
    automatic: "left" | "right" | null;
    initial: "left" | "right";
  };
}
