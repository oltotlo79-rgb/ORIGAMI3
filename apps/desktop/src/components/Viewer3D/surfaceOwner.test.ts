import { describe, expect, it, vi } from "vitest";
import * as THREE from "three";
import {
  SURFACE_OWNER_BACKGROUND_CODE,
  SURFACE_OWNER_DEPTH_PLANE_ATTRIBUTE,
  SURFACE_OWNER_DEPTH_TOLERANCE,
  SURFACE_OWNER_GROUP_ATTRIBUTE,
  createSurfaceOwnerBinding,
  createSurfaceOwnerCodes,
  createSurfaceOwnerSurface,
  disposeSurfaceOwnerSurface,
  orderSurfaceOwner,
  ownerCodeBytes,
  ownerCodeVector,
  updateSurfaceOwnerFaceRanks,
  updateSurfaceOwnerFaceLayers,
  updateSurfaceOwnerTriangleLayers,
} from "./surfaceOwner";
import {
  selectPaperHit,
  type PaperHitCandidate,
} from "./hingePicker";

function positionOfTwoOverlappingFaces(): THREE.BufferAttribute {
  return new THREE.BufferAttribute(
    new Float32Array([
      0, 0, 0,
      1, 0, 0,
      0, 1, 0,
      0, 0, 0,
      1, 0, 0,
      0, 1, 0,
    ]),
    3,
  );
}

/** 同じ三角形を、紙の表法線が反対になるwindingで完全に重ねる。 */
function positionOfOppositeWindingFaces(): THREE.BufferAttribute {
  return new THREE.BufferAttribute(
    new Float32Array([
      // 表法線 +z
      0, 0, 0,
      1, 0, 0,
      0, 1, 0,
      // 表法線 -z
      0, 0, 0,
      0, 1, 0,
      1, 0, 0,
    ]),
    3,
  );
}

function cameraAt(z: number): THREE.PerspectiveCamera {
  const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
  camera.position.set(0.25, 0.25, z);
  camera.lookAt(0.25, 0.25, 0);
  camera.updateMatrixWorld(true);
  return camera;
}

function indicesOf(surface: ReturnType<typeof createSurfaceOwnerSurface>): number[] {
  const index = surface.geometry.getIndex();
  if (!index) return [];
  return Array.from({ length: index.count }, (_, at) => index.getX(at));
}

function overlappingSurface(
  ranks: [number, number] = [0, 0],
) {
  const surface = createSurfaceOwnerSurface({
    position: positionOfTwoOverlappingFaces(),
    vertexFaces: [10, 10, 10, 20, 20, 20],
    indices: [0, 1, 2, 3, 4, 5],
    triangleFaces: [10, 20],
    triangleLayers: ranks,
  });
  updateSurfaceOwnerFaceRanks(surface, new Map([[10, ranks[0]], [20, ranks[1]]]));
  return surface;
}

function oppositeWindingSurface(
  frontFaceId: number,
  backFaceId: number,
  ranks: [number, number] = [0, 0],
) {
  const surface = createSurfaceOwnerSurface({
    position: positionOfOppositeWindingFaces(),
    vertexFaces: [
      frontFaceId,
      frontFaceId,
      frontFaceId,
      backFaceId,
      backFaceId,
      backFaceId,
    ],
    indices: [0, 1, 2, 3, 4, 5],
    triangleFaces: [frontFaceId, backFaceId],
    triangleLayers: ranks,
  });
  updateSurfaceOwnerFaceRanks(
    surface,
    new Map([[frontFaceId, ranks[0]], [backFaceId, ranks[1]]]),
  );
  return surface;
}

describe("surface owner code", () => {
  it("背景を0に予約し、重複した面IDを数値順の1始まりcodeへ詰める", () => {
    expect(SURFACE_OWNER_BACKGROUND_CODE).toBe(0);
    expect([...createSurfaceOwnerCodes([20, 3, 20, 9])]).toEqual([
      [3, 1],
      [9, 2],
      [20, 3],
    ]);
  });

  it("32-bit codeをlittle-endian RGBA8と正規化Vector4へ直す", () => {
    expect(ownerCodeBytes(0)).toEqual([0, 0, 0, 0]);
    expect(ownerCodeBytes(0x7856_3412)).toEqual([0x12, 0x34, 0x56, 0x78]);
    expect(ownerCodeBytes(0xffff_ffff)).toEqual([255, 255, 255, 255]);
    expect(ownerCodeVector(0x0403_0201).toArray()).toEqual([
      1 / 255,
      2 / 255,
      3 / 255,
      4 / 255,
    ]);
  });

  it("texture未接続のbindingは無効で、安全な解像度から始まる", () => {
    const binding = createSurfaceOwnerBinding();
    expect(binding.enabled.value).toBe(0);
    expect(binding.map.value).toBeNull();
    expect(binding.resolution.value).toBeInstanceOf(THREE.Vector2);
    expect(binding.resolution.value.toArray()).toEqual([1, 1]);
  });
});

describe("createSurfaceOwnerSurface", () => {
  it("positionを共有し、正規化owner属性・動的index・面batchを別geometryへ作る", () => {
    const position = positionOfTwoOverlappingFaces();
    const indices = [0, 1, 2, 3, 4, 5];
    const triangleFaces = [20, 10];
    const triangleLayers = [4, 2];
    const surface = createSurfaceOwnerSurface({
      position,
      vertexFaces: [20, 20, 20, 10, 10, 10],
      indices,
      triangleFaces,
      triangleLayers,
    });

    expect(surface.geometry.getAttribute("position")).toBe(position);
    expect(surface.position).toBe(position);
    const token = surface.geometry.getAttribute("surfaceOwnerToken");
    expect(token).toBeInstanceOf(THREE.Uint8BufferAttribute);
    expect(token.itemSize).toBe(4);
    expect(token.normalized).toBe(true);
    const depthPlanes = surface.geometry.getAttribute(
      SURFACE_OWNER_DEPTH_PLANE_ATTRIBUTE,
    );
    expect(depthPlanes).toBe(surface.depthPlanes);
    expect(depthPlanes.itemSize).toBe(4);
    expect(surface.depthPlanes.usage).toBe(THREE.DynamicDrawUsage);
    // 面ID 10→code 1、面ID 20→code 2。最初の3頂点は面20。
    expect(Array.from(token.array)).toEqual([
      2, 0, 0, 0,
      2, 0, 0, 0,
      2, 0, 0, 0,
      1, 0, 0, 0,
      1, 0, 0, 0,
      1, 0, 0, 0,
    ]);
    expect(surface.geometry.getIndex()?.usage).toBe(THREE.DynamicDrawUsage);
    expect(surface.batches.map((batch) => batch.faceId)).toEqual([10, 20]);
    expect(surface.batches.map((batch) => batch.layer)).toEqual([2, 4]);
    expect(surface.triangleFaces).toEqual([20, 10]);
    expect(surface.triangleLayers).toEqual([4, 2]);
    expect(surface.batches.map((batch) => batch.surfaceRank)).toEqual([0, 0]);
    expect([...surface.faceSurfaceRanks]).toEqual([[20, 0], [10, 0]]);

    // 入力配列を書き換えてもowner側の記録・indexは変わらない。
    indices[0] = 99;
    triangleFaces[0] = 99;
    triangleLayers[0] = 99;
    expect(indicesOf(surface)).toEqual([0, 1, 2, 3, 4, 5]);
    expect(surface.triangleFaces).toEqual([20, 10]);
    expect(surface.triangleLayers).toEqual([4, 2]);
  });

  it("渡されたowner codeをrigid/soft間で共有できる", () => {
    const codes = createSurfaceOwnerCodes([10, 20, 30]);
    const surface = createSurfaceOwnerSurface({
      position: positionOfTwoOverlappingFaces(),
      vertexFaces: [10, 10, 10, 20, 20, 20],
      indices: [0, 1, 2, 3, 4, 5],
      triangleFaces: [10, 20],
      ownerCodes: codes,
    });
    expect([...surface.ownerCodes]).toEqual([...codes]);
  });
});

describe("orderSurfaceOwner", () => {
  it("共平面の別triangleへ同じwindow-depth平面を割り当てる", () => {
    const surface = oppositeWindingSurface(10, 20);
    const camera = cameraAt(2);
    try {
      orderSurfaceOwner(surface, camera);
      const attribute = surface.depthPlanes;
      const expected = [
        attribute.getX(0),
        attribute.getY(0),
        attribute.getZ(0),
        1,
      ];
      for (let vertex = 0; vertex < attribute.count; vertex++) {
        expect([
          attribute.getX(vertex),
          attribute.getY(vertex),
          attribute.getZ(vertex),
          attribute.getW(vertex),
        ]).toEqual(expected);
        const projected = new THREE.Vector3()
          .fromBufferAttribute(surface.position, vertex)
          .project(camera);
        const actualDepth = (projected.z + 1) * 0.5;
        const sharedDepth =
          expected[0] * projected.x + expected[1] * projected.y + expected[2];
        expect(Math.abs(sharedDepth - actualDepth)).toBeLessThanOrEqual(
          SURFACE_OWNER_DEPTH_TOLERANCE,
        );
      }
    } finally {
      disposeSurfaceOwnerSurface(surface);
    }
  });

  it("同じ支持平面の面へ、深度に依存しない同じ組符号を配る", () => {
    const surface = oppositeWindingSurface(10, 20);
    try {
      orderSurfaceOwner(surface, cameraAt(2));
      const tokens = surface.geometry.getAttribute(
        SURFACE_OWNER_GROUP_ATTRIBUTE,
      ) as THREE.BufferAttribute;
      expect(tokens).toBe(surface.groupTokens);
      expect(tokens.itemSize).toBe(4);
      expect(tokens.normalized).toBe(true);
      expect(tokens.usage).toBe(THREE.DynamicDrawUsage);
      const expected = ownerCodeBytes(1);
      for (let vertex = 0; vertex < tokens.count; vertex++) {
        expect([
          (tokens.array as Uint8Array)[vertex * 4],
          (tokens.array as Uint8Array)[vertex * 4 + 1],
          (tokens.array as Uint8Array)[vertex * 4 + 2],
          (tokens.array as Uint8Array)[vertex * 4 + 3],
        ]).toEqual(expected);
      }
    } finally {
      disposeSurfaceOwnerSurface(surface);
    }
  });

  it("別の平面に乗る面へは別の組符号を配り、非平面faceには配らない", () => {
    // 0.0002だけ離れた実在の層差。ここを同じ組にすると層が入れ替わる。
    const position = positionOfTwoOverlappingFaces();
    for (let vertex = 3; vertex < 6; vertex++) position.setZ(vertex, 0.0002);
    const separated = createSurfaceOwnerSurface({
      position,
      vertexFaces: [10, 10, 10, 20, 20, 20],
      indices: [0, 1, 2, 3, 4, 5],
      triangleFaces: [10, 20],
    });
    const bent = createSurfaceOwnerSurface({
      position: new THREE.BufferAttribute(
        new Float32Array([0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0.001]),
        3,
      ),
      vertexFaces: [30, 30, 30, 30],
      indices: [0, 1, 2, 0, 2, 3],
      triangleFaces: [30, 30],
    });
    try {
      orderSurfaceOwner(separated, cameraAt(2));
      orderSurfaceOwner(bent, cameraAt(2));
      const tokens = separated.groupTokens.array as Uint8Array;
      const lower = [...tokens.slice(0, 4)];
      const upper = [...tokens.slice(12, 16)];
      expect(lower).not.toEqual([0, 0, 0, 0]);
      expect(upper).not.toEqual([0, 0, 0, 0]);
      expect(lower).not.toEqual(upper);
      for (let vertex = 0; vertex < 3; vertex++) {
        expect([...tokens.slice(vertex * 4, vertex * 4 + 4)]).toEqual(lower);
        expect([...tokens.slice(12 + vertex * 4, 16 + vertex * 4)]).toEqual(upper);
      }
      // 平面に乗らない面は組を作れないので、従来どおり深度だけで判定させる。
      expect([...(bent.groupTokens.array as Uint8Array)]).toEqual(
        new Array(4 * 4).fill(0),
      );
    } finally {
      disposeSurfaceOwnerSurface(separated);
      disposeSurfaceOwnerSurface(bent);
    }
  });

  it("斜めの支持平面もoff-axis視点のwindow depthへ変換する", () => {
    const position = new THREE.BufferAttribute(
      new Float32Array([
        -0.4, -0.2, 0.15,
        0.7, -0.1, 0.4,
        0.1, 0.8, -0.25,
        -0.4, -0.2, 0.15,
        0.1, 0.8, -0.25,
        0.7, -0.1, 0.4,
      ]),
      3,
    );
    const surface = createSurfaceOwnerSurface({
      position,
      vertexFaces: [10, 10, 10, 20, 20, 20],
      indices: [0, 1, 2, 3, 4, 5],
      triangleFaces: [10, 20],
    });
    const camera = new THREE.PerspectiveCamera(51, 1.3, 0.01, 100);
    camera.position.set(1.4, -0.8, 2.6);
    camera.lookAt(0.1, 0.1, 0);
    camera.updateMatrixWorld(true);
    try {
      orderSurfaceOwner(surface, camera);
      const plane = surface.depthPlanes;
      expect(plane.getW(0)).toBe(1);
      for (let vertex = 0; vertex < plane.count; vertex++) {
        expect([
          plane.getX(vertex),
          plane.getY(vertex),
          plane.getZ(vertex),
          plane.getW(vertex),
        ]).toEqual([
          plane.getX(0),
          plane.getY(0),
          plane.getZ(0),
          1,
        ]);
        const projected = new THREE.Vector3()
          .fromBufferAttribute(position, vertex)
          .project(camera);
        const originalDepth = (projected.z + 1) * 0.5;
        const sharedDepth =
          plane.getX(vertex) * projected.x +
          plane.getY(vertex) * projected.y +
          plane.getZ(vertex);
        expect(Math.abs(sharedDepth - originalDepth)).toBeLessThanOrEqual(
          SURFACE_OWNER_DEPTH_TOLERANCE,
        );
      }
    } finally {
      disposeSurfaceOwnerSurface(surface);
    }
  });

  it("実在する0.0002の平面差と非平面faceは共通depthへ丸めない", () => {
    const separatedPosition = positionOfTwoOverlappingFaces();
    for (let vertex = 3; vertex < 6; vertex++) {
      separatedPosition.setZ(vertex, 0.0002);
    }
    const separated = createSurfaceOwnerSurface({
      position: separatedPosition,
      vertexFaces: [10, 10, 10, 20, 20, 20],
      indices: [0, 1, 2, 3, 4, 5],
      triangleFaces: [10, 20],
    });
    const bent = createSurfaceOwnerSurface({
      position: new THREE.BufferAttribute(
        new Float32Array([
          0, 0, 0,
          1, 0, 0,
          1, 1, 0,
          0, 1, 0.001,
        ]),
        3,
      ),
      vertexFaces: [30, 30, 30, 30],
      indices: [0, 1, 2, 0, 2, 3],
      triangleFaces: [30, 30],
    });
    try {
      orderSurfaceOwner(separated, cameraAt(2));
      orderSurfaceOwner(bent, cameraAt(2));
      expect(
        Array.from(
          { length: separated.depthPlanes.count },
          (_, vertex) => separated.depthPlanes.getW(vertex),
        ),
      ).toEqual(new Array(6).fill(0));
      expect(
        Array.from(
          { length: bent.depthPlanes.count },
          (_, vertex) => bent.depthPlanes.getW(vertex),
        ),
      ).toEqual(new Array(4).fill(0));
    } finally {
      disposeSurfaceOwnerSurface(separated);
      disposeSurfaceOwnerSurface(bent);
    }
  });

  it("同じlayerなら+側は大きいfaceIdを後勝ち、-側は逆にする", () => {
    const surface = overlappingSurface();
    orderSurfaceOwner(surface, cameraAt(2));
    expect(indicesOf(surface)).toEqual([0, 1, 2, 3, 4, 5]);

    orderSurfaceOwner(surface, cameraAt(-2));
    expect(indicesOf(surface)).toEqual([3, 4, 5, 0, 1, 2]);
  });

  it("異なるlayerは+側で大きい層、-側で小さい層を後勝ちにする", () => {
    const surface = overlappingSurface([3, 1]);
    orderSurfaceOwner(surface, cameraAt(2));
    expect(indicesOf(surface)).toEqual([3, 4, 5, 0, 1, 2]);

    orderSurfaceOwner(surface, cameraAt(-2));
    expect(indicesOf(surface)).toEqual([0, 1, 2, 3, 4, 5]);
  });

  it("rank未指定の旧入力は面IDによらず材質windingを選び、裏視点でも同じ物理面を保つ", () => {
    // 表面のface IDが大きい場合と小さい場合の両方を通し、ID順の偶然を排除する。
    for (const [frontFaceId, backFaceId] of [[30, 10], [10, 30]] as const) {
      const surface = oppositeWindingSurface(frontFaceId, backFaceId);
      try {
        orderSurfaceOwner(surface, cameraAt(2));
        // 防御fallbackで裏windingを先、表windingの物理表面を後に描く。
        expect(indicesOf(surface)).toEqual([3, 4, 5, 0, 1, 2]);

        orderSurfaceOwner(surface, cameraAt(-2));
        // カメラに向く面へ乗り換えない。同じ物理面を裏から見て裏色を出す。
        expect(indicesOf(surface)).toEqual([3, 4, 5, 0, 1, 2]);
      } finally {
        disposeSurfaceOwnerSurface(surface);
      }
    }
  });

  it("A/B/Cの旧rank未指定fixtureでも材質表面のownerを表裏視点とも保つ", () => {
    const states = [
      { label: "A: 8本を山折り+180°", tiltDeg: 0 },
      { label: "B: 8本を谷折り-180°", tiltDeg: 0 },
      { label: "C: 山折り+180°と#43=-85°", tiltDeg: 88.9 },
    ] as const;
    for (const state of states) {
      // 実測中央と同じく、face 2/7がface 13を二分して完全に覆い、
      // face 2/7とface 13のwindingは反対。Cだけはほぼ垂直へ回す。
      const source = [
        [0.25, 0.75], [0.25, -0.25], [-0.25, -0.25],
        [0.25, -0.25], [0.25, 0.75], [0.75, -0.25],
        [0.25, 0.75], [-0.25, -0.25], [0.75, -0.25],
      ] as const;
      const angle = THREE.MathUtils.degToRad(state.tiltDeg);
      const positions = source.flatMap(([x, y]) => {
        const dy = y - 0.25;
        return [x, 0.25 + dy * Math.cos(angle), dy * Math.sin(angle)];
      });
      const position = new THREE.BufferAttribute(
        new Float32Array(positions),
        3,
      );
      const surface = createSurfaceOwnerSurface({
        position,
        vertexFaces: [2, 2, 2, 7, 7, 7, 13, 13, 13],
        indices: [0, 1, 2, 3, 4, 5, 6, 7, 8],
        triangleFaces: [2, 7, 13],
      });
      try {
        for (const camera of [cameraAt(2), cameraAt(-2)]) {
          orderSurfaceOwner(surface, camera);
          const indices = indicesOf(surface);
          const ownerVertex = indices[indices.length - 1];
          const owner = ownerVertex === undefined ? -1 : [2, 7, 13][Math.floor(ownerVertex / 3)];
          expect(owner, state.label).toBe(13);
        }
      } finally {
        disposeSurfaceOwnerSurface(surface);
      }
    }
  });

  it("紙全体を剛体回転してworld法線の符号が反転してもownerを変えない", () => {
    const surface = oppositeWindingSurface(10, 20);
    updateSurfaceOwnerFaceRanks(surface, new Map([[10, 1], [20, 0]]));
    try {
      orderSurfaceOwner(surface, cameraAt(2));
      const before = indicesOf(surface);
      const position = surface.position.array as Float32Array;
      for (let vertex = 0; vertex < surface.position.count; vertex++) {
        position[vertex * 3 + 1] *= -1;
        position[vertex * 3 + 2] *= -1;
      }
      surface.position.needsUpdate = true;
      orderSurfaceOwner(surface, cameraAt(2));
      expect(indicesOf(surface)).toEqual(before);
    } finally {
      disposeSurfaceOwnerSurface(surface);
    }
  });

  it("異なるsurface rankは視点側に応じて比較方向を反転する", () => {
    const surface = oppositeWindingSurface(20, 10, [1, 2]);
    try {
      orderSurfaceOwner(surface, cameraAt(2));
      // +側ではrank 2の裏winding面が手前なので、こちらが最後になる。
      expect(indicesOf(surface)).toEqual([0, 1, 2, 3, 4, 5]);

      orderSurfaceOwner(surface, cameraAt(-2));
      // -側ではrank 1の面が手前になる。
      expect(indicesOf(surface)).toEqual([3, 4, 5, 0, 1, 2]);
    } finally {
      disposeSurfaceOwnerSurface(surface);
    }
  });

  it("同じ入力のsurface rank選択を10回繰り返してもindex順が変わらない", () => {
    const surface = oppositeWindingSurface(7, 99);
    const orders = new Set<string>();
    try {
      for (let run = 0; run < 10; run++) {
        orderSurfaceOwner(surface, cameraAt(2));
        orders.add(indicesOf(surface).join(","));
      }
      expect(orders).toEqual(new Set(["3,4,5,0,1,2"]));
    } finally {
      disposeSurfaceOwnerSurface(surface);
    }
  });

  it("layer更新を保持しつつsurface rank更新だけを次の並べ替えへ反映する", () => {
    const surface = overlappingSurface();
    updateSurfaceOwnerFaceLayers(surface, new Map([[10, 5], [20, 1]]));
    expect(surface.triangleLayers).toEqual([5, 1]);
    expect(surface.batches.map((batch) => batch.layer)).toEqual([5, 1]);
    updateSurfaceOwnerFaceRanks(surface, new Map([[10, 5], [20, 1]]));
    orderSurfaceOwner(surface, cameraAt(2));
    expect(indicesOf(surface)).toEqual([3, 4, 5, 0, 1, 2]);

    updateSurfaceOwnerTriangleLayers(surface, [0, 7]);
    expect(surface.triangleLayers).toEqual([0, 7]);
    expect(surface.batches.map((batch) => batch.layer)).toEqual([0, 7]);
    updateSurfaceOwnerFaceRanks(surface, new Map([[10, 0], [20, 7]]));
    orderSurfaceOwner(surface, cameraAt(2));
    expect(indicesOf(surface)).toEqual([0, 1, 2, 3, 4, 5]);

    updateSurfaceOwnerFaceRanks(surface, new Map([[10, 3], [20, 2]]));
    expect([...surface.faceSurfaceRanks]).toEqual([[10, 3], [20, 2]]);
    expect(surface.batches.map((batch) => batch.surfaceRank)).toEqual([3, 2]);
  });

  it("positionの現座標から法線を取り直す", () => {
    const surface = overlappingSurface();
    // 両面を+y法線の壁へ動かす。canonical normalも+yとなり、+y視点で昇順になる。
    const moved = surface.position.array as Float32Array;
    const wall = [
      0, 0, 0,
      0, 0, 1,
      1, 0, 0,
      0, 0, 0,
      0, 0, 1,
      1, 0, 0,
    ];
    moved.set(wall);
    surface.position.needsUpdate = true;
    const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
    camera.position.set(0.25, 2, 0.25);
    camera.updateMatrixWorld(true);
    orderSurfaceOwner(surface, camera);
    expect(indicesOf(surface)).toEqual([0, 1, 2, 3, 4, 5]);
  });

  it("同じ候補表を描画順とpickerへ通して同じ所有面を選ぶ", () => {
    type Candidate = {
      face: number;
      rank: number;
      normal: readonly [number, number, number];
    };
    const cases: {
      label: string;
      camera: readonly [number, number, number];
      candidates: readonly Candidate[];
      expected: number;
    }[] = [
      {
        label: "+側は大きいrank",
        camera: [0, 0, 2],
        candidates: [
          { face: 10, rank: 1, normal: [0, 0, 1] },
          { face: 20, rank: 2, normal: [0, 0, 1] },
        ],
        expected: 20,
      },
      {
        label: "-側は小さいrank",
        camera: [0, 0, -2],
        candidates: [
          { face: 10, rank: 1, normal: [0, 0, 1] },
          { face: 20, rank: 2, normal: [0, 0, 1] },
        ],
        expected: 10,
      },
      {
        label: "候補ごとのside",
        camera: [0, 0, 2],
        candidates: [
          { face: 10, rank: 1, normal: [1, 0, 0.25] },
          { face: 20, rank: 2, normal: [1, 0, -0.25] },
        ],
        expected: 10,
      },
      {
        label: "rank同値の材質winding",
        camera: [2, 0, 0],
        candidates: [
          { face: 10, rank: 0, normal: [1, 0, -0.25] },
          { face: 20, rank: 0, normal: [1, 0, 0.25] },
        ],
        expected: 20,
      },
      {
        label: "rankと材質同値のface ID",
        camera: [0, 0, 2],
        candidates: [
          { face: 10, rank: 0, normal: [0, 0, 1] },
          { face: 20, rank: 0, normal: [0, 0, 1] },
        ],
        expected: 20,
      },
      {
        label: "front優先へ変えず材質fallbackを維持",
        camera: [0, 0, -2],
        candidates: [
          { face: 30, rank: 0, normal: [0, 0, 1] },
          { face: 10, rank: 0, normal: [0, 0, -1] },
        ],
        expected: 30,
      },
    ];

    for (const row of cases) {
      const positions: number[] = [];
      const vertexFaces: number[] = [];
      const indices: number[] = [];
      const triangleFaces: number[] = [];
      const ranks = new Map<number, number>();
      const hits: PaperHitCandidate[] = [];
      for (const candidate of row.candidates) {
        const normal = new THREE.Vector3(...candidate.normal).normalize();
        const helper = Math.abs(normal.z) < 0.9
          ? new THREE.Vector3(0, 0, 1)
          : new THREE.Vector3(1, 0, 0);
        const u = helper.cross(normal).normalize();
        const v = normal.clone().cross(u).normalize();
        const points = [u, v, u.clone().add(v).multiplyScalar(-1)];
        const start = vertexFaces.length;
        for (const point of points) {
          positions.push(point.x, point.y, point.z);
          vertexFaces.push(candidate.face);
        }
        indices.push(start, start + 1, start + 2);
        triangleFaces.push(candidate.face);
        ranks.set(candidate.face, candidate.rank);
        hits.push({
          face: candidate.face,
          surfaceRank: candidate.rank,
          distance: new THREE.Vector3(...row.camera).length(),
          point: new THREE.Vector3(),
          normal,
        });
      }
      const surface = createSurfaceOwnerSurface({
        position: new THREE.BufferAttribute(new Float32Array(positions), 3),
        vertexFaces,
        indices,
        triangleFaces,
      });
      updateSurfaceOwnerFaceRanks(surface, ranks);
      const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
      camera.position.set(...row.camera);
      camera.lookAt(0, 0, 0);
      camera.updateMatrixWorld(true);
      try {
        orderSurfaceOwner(surface, camera);
        const orderedIndices = indicesOf(surface);
        const lastVertex = orderedIndices[orderedIndices.length - 1];
        const drawnFace = vertexFaces[lastVertex];
        expect(drawnFace, row.label).toBe(row.expected);
        expect(selectPaperHit(hits, camera.position)?.face, row.label).toBe(
          drawnFace,
        );
        expect(
          selectPaperHit([...hits].reverse(), camera.position)?.face,
          `${row.label}: reverse`,
        ).toBe(drawnFace);
      } finally {
        disposeSurfaceOwnerSurface(surface);
      }
    }
  });
});

describe("disposeSurfaceOwnerSurface", () => {
  it("owner geometryだけを破棄し、共有positionを置き換えない", () => {
    const position = positionOfTwoOverlappingFaces();
    const surface = createSurfaceOwnerSurface({
      position,
      vertexFaces: [10, 10, 10, 20, 20, 20],
      indices: [0, 1, 2, 3, 4, 5],
      triangleFaces: [10, 20],
    });
    const dispose = vi.spyOn(surface.geometry, "dispose");

    disposeSurfaceOwnerSurface(surface);

    expect(dispose).toHaveBeenCalledTimes(1);
    expect(surface.position).toBe(position);
    expect(surface.geometry.getAttribute("position")).toBe(position);
  });
});
