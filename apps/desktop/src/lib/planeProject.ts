// 画面座標(px)から畳み平面(z=0)への投影。3Dビューで折り線を引くために使う。
// カメラの種類に依存しないよう、視錐台の手前面と奥面の2点を逆投影して光線を作る。

import * as THREE from "three";
import type { Vec2 } from "./types";

/** 光線が平面と平行とみなす向きの閾値 */
const PARALLEL_EPS = 1e-9;

/**
 * 畳み平面の原点から交点までの距離の上限(座標の単位は「紙の長辺=1」)。
 * カメラが平面とほぼ平行になると交点は際限なく遠ざかる。そのまま採用すると
 * 紙から大きく外れた位置に折り線が引かれるため、この距離を超えたら
 * 「平面を捉えていない」として弾く。紙は畳んでも数単位の範囲に収まるので、
 * 50は実用上あり得ない遠さ。
 */
const MAX_PLANE_DISTANCE = 50;

/** 使い回す作業用ベクトル(毎フレーム呼ばれるので割り当てを増やさない) */
const NEAR = new THREE.Vector3();
const FAR = new THREE.Vector3();

/**
 * 画面座標(canvasの左上基準・px)から見た光線と畳み平面(z=0)の交点を返す。
 * 平面と平行な向き、交点が描画範囲(手前面〜奥面)の外にある場合、
 * および交点が紙からかけ離れて遠い場合はnull。
 * 平面を裏側(z<0)から見ていても交点は同じように求まる。
 */
export function screenToPlane(
  camera: THREE.Camera,
  widthPx: number,
  heightPx: number,
  x: number,
  y: number,
): Vec2 | null {
  if (widthPx <= 0 || heightPx <= 0) return null;
  const nx = (x / widthPx) * 2 - 1;
  const ny = 1 - (y / heightPx) * 2;
  NEAR.set(nx, ny, -1).unproject(camera);
  FAR.set(nx, ny, 1).unproject(camera);
  const dz = FAR.z - NEAR.z;
  if (Math.abs(dz) < PARALLEL_EPS) return null;
  const t = -NEAR.z / dz;
  if (!(t >= 0 && t <= 1)) return null;
  const hitX = NEAR.x + (FAR.x - NEAR.x) * t;
  const hitY = NEAR.y + (FAR.y - NEAR.y) * t;
  if (!Number.isFinite(hitX) || !Number.isFinite(hitY)) return null;
  if (Math.hypot(hitX, hitY) > MAX_PLANE_DISTANCE) return null;
  return [hitX, hitY];
}

/**
 * 画面上の距離(px)を、その位置での畳み平面上の距離へ直す(吸着半径の換算用)。
 * 平面へ投影できない位置では `fallback` を返す。
 */
export function planeRadius(
  camera: THREE.Camera,
  widthPx: number,
  heightPx: number,
  x: number,
  y: number,
  px: number,
  fallback: number,
): number {
  const a = screenToPlane(camera, widthPx, heightPx, x, y);
  const b = screenToPlane(camera, widthPx, heightPx, x + px, y);
  if (!a || !b) return fallback;
  const r = Math.hypot(b[0] - a[0], b[1] - a[1]);
  return Number.isFinite(r) && r > 0 ? r : fallback;
}
