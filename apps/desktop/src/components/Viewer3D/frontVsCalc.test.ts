import { describe, expect, it } from "vitest";
import * as THREE from "three";

import type { Document, Face, Frame3D } from "../../lib/types";
import { buildTopology, createContent, updateFrame } from "./sceneBuilder";
import { orderSurfaceOwner } from "./surfaceOwner";

const document: Document = {
  schema_version: 1,
  paper: { width_mm: 150, height_mm: 150 },
  cp: {
    vertices: [
      { id: 0, pos: [0, 0] },
      { id: 1, pos: [1, 0] },
      { id: 2, pos: [1, 1] },
      { id: 3, pos: [0, 1] },
      { id: 4, pos: [0, 0.5] },
      { id: 5, pos: [1, 0.5] },
      { id: 6, pos: [0.5, 1] },
      { id: 7, pos: [0.5, 0] },
      { id: 8, pos: [0.5, 0.5] },
      { id: 9, pos: [0.7928932188134525, 0.5] },
      { id: 10, pos: [0.5, 0.20710678118654752] },
      { id: 11, pos: [0.20710678118654752, 0.5] },
      { id: 12, pos: [0.5, 0.7928932188134525] },
    ],
    edges: [
      { id: 4, kind: "Border", v0: 3, v1: 4 },
      { id: 5, kind: "Border", v0: 4, v1: 0 },
      { id: 6, kind: "Border", v0: 1, v1: 5 },
      { id: 7, kind: "Border", v0: 5, v1: 2 },
      { id: 9, kind: "Border", v0: 2, v1: 6 },
      { id: 10, kind: "Border", v0: 6, v1: 3 },
      { id: 11, kind: "Border", v0: 0, v1: 7 },
      { id: 12, kind: "Border", v0: 7, v1: 1 },
      { id: 17, kind: "Valley", v0: 0, v1: 8 },
      { id: 18, kind: "Valley", v0: 8, v1: 2 },
      { id: 19, kind: "Mountain", v0: 8, v1: 9 },
      { id: 20, kind: "Mountain", v0: 9, v1: 5 },
      { id: 21, kind: "Mountain", v0: 2, v1: 9 },
      { id: 22, kind: "Mountain", v0: 9, v1: 1 },
      { id: 23, kind: "Mountain", v0: 8, v1: 10 },
      { id: 24, kind: "Mountain", v0: 10, v1: 7 },
      { id: 25, kind: "Mountain", v0: 0, v1: 10 },
      { id: 26, kind: "Mountain", v0: 10, v1: 1 },
      { id: 27, kind: "Mountain", v0: 4, v1: 11 },
      { id: 28, kind: "Mountain", v0: 11, v1: 8 },
      { id: 29, kind: "Mountain", v0: 0, v1: 11 },
      { id: 30, kind: "Mountain", v0: 11, v1: 3 },
      { id: 31, kind: "Mountain", v0: 6, v1: 12 },
      { id: 32, kind: "Mountain", v0: 12, v1: 8 },
      { id: 33, kind: "Mountain", v0: 2, v1: 12 },
      { id: 34, kind: "Mountain", v0: 12, v1: 3 },
      { id: 35, kind: "Valley", v0: 11, v1: 12 },
      { id: 36, kind: "Valley", v0: 9, v1: 10 },
    ],
    next_vertex_id: 13,
    next_edge_id: 37,
  },
  sequence: [],
  display: {
    front_color: [237, 28, 36],
    back_color: [255, 255, 255],
    grid_divisions: 8,
    soft_enabled: false,
    soft_stiffness: 0.5,
    soft_pressure: 0,
    overlap_prevention_enabled: true,
    penetration_prevention_enabled: true,
  },
};

const faces: Face[] = [
  { edges: [4, 27, 30], id: 0, vertices: [3, 4, 11] },
  { edges: [5, 29, 27], id: 1, vertices: [4, 0, 11] },
  { edges: [6, 20, 22], id: 2, vertices: [1, 5, 9] },
  { edges: [7, 21, 20], id: 3, vertices: [5, 2, 9] },
  { edges: [9, 31, 33], id: 4, vertices: [2, 6, 12] },
  { edges: [10, 34, 31], id: 5, vertices: [6, 3, 12] },
  { edges: [11, 24, 25], id: 6, vertices: [0, 7, 10] },
  { edges: [12, 26, 24], id: 7, vertices: [7, 1, 10] },
  { edges: [17, 28, 29], id: 8, vertices: [0, 8, 11] },
  { edges: [17, 25, 23], id: 9, vertices: [8, 0, 10] },
  { edges: [18, 33, 32], id: 10, vertices: [8, 2, 12] },
  { edges: [18, 19, 21], id: 11, vertices: [2, 8, 9] },
  { edges: [19, 23, 36], id: 12, vertices: [9, 8, 10] },
  { edges: [22, 36, 26], id: 13, vertices: [1, 9, 10] },
  { edges: [28, 32, 35], id: 14, vertices: [11, 8, 12] },
  { edges: [30, 35, 34], id: 15, vertices: [3, 11, 12] },
];

const frame: Frame3D = {
  faces: [
    { face: 0, layer: 0, mirrored: false, polygon: [[0, 1, 0], [0, 0.5, 0], [0.20710678118654752, 0.5, 0]], surface_rank: 14 },
    { face: 1, layer: 0, mirrored: true, polygon: [[0, 0.5, 0], [0, 0.9998700236504696, 0.011399976126235322], [0.20710678118654752, 0.5, 0]], surface_rank: 0 },
    { face: 2, layer: 0, mirrored: true, polygon: [[-2.220446049250313e-16, 0.5147768228297747, 0.4997815978075393], [0, 0.5000000000000001, -1.1102230246251565e-16], [-0.20710678118654746, 0.5000000000000001, -1.6653345369377348e-16]], surface_rank: 3 },
    { face: 3, layer: 0, mirrored: false, polygon: [[0, 0.49999999999999994, 7.632783294297951e-17], [0, 0.9998700236504697, 0.011399976126235507], [-0.20710678118654746, 0.5, 6.418476861114186e-17]], surface_rank: 11 },
    { face: 4, layer: 0, mirrored: true, polygon: [[0, 0.9998700236504698, 0.011399976126235323], [-2.220446049250313e-16, 0.5, 0], [-0.20710678118654757, 0.5000000000000001, -4.683753385137379e-17]], surface_rank: 1 },
    { face: 5, layer: 0, mirrored: false, polygon: [[-1.1102230246251565e-16, 0.5, 0], [0, 1, 0], [-0.20710678118654757, 0.5000000000000001, -4.686520405326299e-17]], surface_rank: 15 },
    { face: 6, layer: 0, mirrored: false, polygon: [[0, 0.9998700236504696, 0.011399976126235322], [2.7755575615628904e-17, 0.5, 4.683753385137379e-17], [0.20710678118654746, 0.5, 1.1622647289044608e-16]], surface_rank: 8 },
    { face: 7, layer: 0, mirrored: true, polygon: [[2.7755575615628914e-17, 0.5, 0], [-1.435982094375912e-16, 0.5147768228297743, 0.49978159780753917], [0.20710678118654746, 0.5, 5.551115123125783e-17]], surface_rank: 4 },
    { face: 8, layer: 0, mirrored: false, polygon: [[0, 0.9998700236504696, 0.011399976126235322], [0, 0.2929470567802157, -0.004722024722216138], [0.20710678118654752, 0.5, 0]], surface_rank: 6 },
    { face: 9, layer: 0, mirrored: true, polygon: [[0, 0.2929470567802157, -0.004722024722216138], [0, 0.9998700236504696, 0.011399976126235322], [0.20710678118654743, 0.5, 1.1622647289044608e-16]], surface_rank: 7 },
    { face: 10, layer: 0, mirrored: false, polygon: [[0, 0.29294705678021554, -0.004722024722216167], [0, 0.9998700236504696, 0.011399976126235514], [-0.20710678118654757, 0.5, -4.85722573273506e-17]], surface_rank: 9 },
    { face: 11, layer: 0, mirrored: true, polygon: [[0, 0.9998700236504696, 0.011399976126235514], [0, 0.29294705678021554, -0.004722024722216167], [-0.20710678118654746, 0.5, 6.591949208711867e-17]], surface_rank: 10 },
    { face: 12, layer: 0, mirrored: false, polygon: [[-0.20710678118654746, 0.5, -3.469446951953614e-17], [0, 0.29294705678021576, -0.004722024722216138], [0.20710678118654746, 0.5, 1.1449174941446927e-16]], surface_rank: 12 },
    { face: 13, layer: 0, mirrored: false, polygon: [[-2.220446049250313e-16, 0.5147768228297748, 0.4997815978075394], [-0.20710678118654757, 0.5000000000000001, -8.326672684688674e-17], [0.20710678118654746, 0.5, 8.326672684688674e-17]], surface_rank: 2 },
    { face: 14, layer: 0, mirrored: true, polygon: [[0.20710678118654752, 0.5, -1.734723475976807e-18], [0, 0.2929470567802156, -0.004722024722216167], [-0.20710678118654757, 0.5, -4.85722573273506e-17]], surface_rank: 5 },
    { face: 15, layer: 0, mirrored: true, polygon: [[0, 1, 0], [0.20710678118654752, 0.5, 0], [-0.20710678118654757, 0.5000000000000001, -4.6865204053262986e-17]], surface_rank: 13 },
  ],
  warnings: [],
};

const calculated = new Map([
  [7, { order: 0, back: true }],
  [2, { order: 1, back: true }],
  [13, { order: 2, back: false }],
  [1, { order: 3, back: true }],
  [4, { order: 4, back: true }],
  [14, { order: 5, back: true }],
  [8, { order: 6, back: false }],
  [9, { order: 7, back: true }],
  [6, { order: 8, back: false }],
  [10, { order: 9, back: false }],
  [11, { order: 10, back: true }],
  [3, { order: 11, back: false }],
  [12, { order: 12, back: false }],
  [15, { order: 13, back: true }],
  [0, { order: 14, back: false }],
  [5, { order: 15, back: false }],
]);

function defaultCamera(): THREE.PerspectiveCamera {
  const camera = new THREE.PerspectiveCamera(45, 925 / 536, 0.01, 100);
  const center = new THREE.Vector3(0.5, 0.5, 0);
  const direction = new THREE.Vector3(0.35, -0.85, 0.95).normalize();
  const distance = (1 / (2 * Math.tan((45 * Math.PI) / 360))) * 1.35;
  camera.position.copy(center).addScaledVector(direction, distance);
  camera.lookAt(center);
  camera.updateMatrixWorld();
  camera.updateProjectionMatrix();
  return camera;
}

function orderedFaces(content: ReturnType<typeof createContent>): number[] {
  const index = content.owner.geometry.getIndex();
  if (!index) throw new Error("surface owner geometry has no index");
  const result: number[] = [];
  for (let i = 0; i < index.count; i += 3) {
    const face = content.topology.vertexFaceIds[index.getX(i)];
    if (result[result.length - 1] !== face) result.push(face);
  }
  return result;
}

function isBackFacing(
  content: ReturnType<typeof createContent>,
  faceId: number,
  camera: THREE.PerspectiveCamera,
): boolean {
  const triangle = content.topology.triangleFaceIds.indexOf(faceId);
  if (triangle < 0) throw new Error(`face ${faceId} has no triangle`);
  const indices = content.topology.indices.slice(triangle * 3, triangle * 3 + 3);
  const point = (index: number) => new THREE.Vector3().fromArray(content.positions, index * 3);
  const a = point(indices[0]);
  const b = point(indices[1]);
  const c = point(indices[2]);
  const normal = b.clone().sub(a).cross(c.clone().sub(a));
  const center = a.clone().add(b).add(c).multiplyScalar(1 / 3);
  return normal.dot(camera.position.clone().sub(center)) < 0;
}

describe("利用者の手順0件姿勢の計算側と画面側の面別受け渡し", () => {
  it("Frame3DをIPC相当のJSON往復後に実際のViewer3D経路へ渡して全面を採取する", () => {
    const ipcFrame = JSON.parse(JSON.stringify(frame)) as Frame3D;
    const hinges = new Set(
      document.cp.edges
        .filter((edge) => edge.kind === "Mountain" || edge.kind === "Valley")
        .map((edge) => edge.id),
    );
    const content = createContent(buildTopology(document, faces, hinges), document.display);
    updateFrame(content, ipcFrame);
    const camera = defaultCamera();
    orderSurfaceOwner(content.owner, camera);
    const screenOrder = orderedFaces(content);
    const rows = frame.faces
      .map((rustFace) => {
        const ipcFace = ipcFrame.faces.find((face) => face.face === rustFace.face)!;
        const slot = content.topology.slots.get(rustFace.face)!;
        const scenePolygon = Array.from({ length: slot.count }, (_, index) =>
          Array.from(content.positions.slice((slot.offset + index) * 3, (slot.offset + index + 1) * 3)),
        );
        const calculation = calculated.get(rustFace.face)!;
        const screenBack = isBackFacing(content, rustFace.face, camera);
        return {
          face: rustFace.face,
          rust_rank: rustFace.surface_rank,
          ipc_rank: ipcFace.surface_rank,
          scene_rank: content.owner.faceSurfaceRanks.get(rustFace.face),
          calc_order: calculation.order,
          screen_order: screenOrder.indexOf(rustFace.face),
          calc_side: calculation.back ? "back" : "front",
          screen_side: screenBack ? "back" : "front",
          color: screenBack ? document.display.back_color : document.display.front_color,
          rust_polygon: rustFace.polygon,
          ipc_polygon: ipcFace.polygon,
          scene_polygon: scenePolygon,
        };
      })
      .sort((a, b) => a.face - b.face);

    console.log(`FRONT_VS_CALC_ROWS ${JSON.stringify(rows)}`);
    expect(rows).toHaveLength(16);
    expect(screenOrder).toEqual([7, 2, 13, 1, 4, 14, 8, 9, 6, 10, 11, 3, 12, 15, 0, 5]);
    expect(rows.every((row) => row.rust_rank === row.ipc_rank)).toBe(true);
    expect(rows.every((row) => row.ipc_rank === row.scene_rank)).toBe(true);
    expect(rows.every((row) => row.calc_order === row.screen_order)).toBe(true);
    expect(rows.every((row) => row.calc_side === row.screen_side)).toBe(true);
    expect(ipcFrame.faces.map((face) => face.polygon)).toEqual(
      frame.faces.map((face) => face.polygon),
    );
  });
});
