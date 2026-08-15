import { describe, expect, it, vi } from "vitest";
import * as THREE from "three";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import { Line2 } from "three/examples/jsm/lines/Line2.js";
import {
  createSurfaceOwnerBinding,
  disposeSurfaceOwnerSurface,
  ownerCodeVector,
} from "./surfaceOwner";
import {
  createContent,
  createHighlightLayer,
  createHighlightMaterials,
  createPreviewMaterial,
  createSoftContent,
  updateSoftContent,
} from "./sceneBuilder";
import {
  SURFACE_OWNER_DEPTH_BITS,
  SURFACE_OWNER_DEPTH_TOLERANCE,
  SURFACE_OWNER_DEPTH_TOLERANCE_CODES,
  SURFACE_OWNER_MAX_RADIUS_PX,
  bindSurfaceOwner,
  createSurfaceOwnerPassResources,
  createSurfaceOwnerOutlineGeometry,
  disposeSurfaceOwnerPassResources,
  filterLineMaterialBySurfaceOwner,
  filterMaterialBySurfaceOwnerAttribute,
  filterOutlineMaterialBySurfaceOwner,
  resizeSurfaceOwnerPassResources,
  setLineSurfaceOwner,
  type FilteredLineMaterial,
} from "./surfaceOwnerShader";

interface ShaderLike {
  uniforms: Record<string, THREE.IUniform>;
  vertexShader: string;
  fragmentShader: string;
}

function compileHook(
  material: THREE.MeshLambertMaterial | THREE.LineBasicMaterial,
): ShaderLike {
  const shader: ShaderLike = {
    uniforms: {},
    vertexShader: "#include <common>\nvoid main() { gl_Position = vec4( position, 1.0 ); }",
    fragmentShader: "#include <common>\nvoid main() { gl_FragColor = vec4( 1.0 ); }",
  };
  material.onBeforeCompile(
    shader as Parameters<typeof material.onBeforeCompile>[0],
    {} as THREE.WebGLRenderer,
  );
  return shader;
}

describe("surface owner shader", () => {
  it("異なる三角形分割でも最前深度とowner色を別passにし、共平面tieを後勝ちにする", () => {
    const binding = createSurfaceOwnerBinding();
    const resources = createSurfaceOwnerPassResources(binding);
    try {
      expect(resources.depthTarget).not.toBe(resources.colorTarget);
      expect(resources.depthTarget.depthBuffer).toBe(true);
      expect(resources.depthTarget.depthTexture).toBeInstanceOf(THREE.DepthTexture);
      expect(resources.depthTarget.depthTexture?.format).toBe(THREE.DepthFormat);
      expect(resources.depthTarget.depthTexture?.type).toBe(THREE.UnsignedIntType);
      expect(resources.depthTarget.depthTexture?.minFilter).toBe(THREE.NearestFilter);
      expect(resources.depthTarget.depthTexture?.magFilter).toBe(THREE.NearestFilter);
      expect(resources.depthTarget.depthTexture?.generateMipmaps).toBe(false);
      expect(resources.colorTarget.depthBuffer).toBe(false);
      expect(resources.colorTarget.depthTexture).toBeNull();
      expect(resources.depthMaterial.side).toBe(THREE.DoubleSide);
      expect(resources.depthMaterial.depthTest).toBe(true);
      expect(resources.depthMaterial.depthWrite).toBe(true);
      expect(resources.depthMaterial.depthFunc).toBe(THREE.LessEqualDepth);
      expect(resources.depthMaterial.colorWrite).toBe(false);
      expect(resources.colorMaterial.depthTest).toBe(false);
      expect(resources.colorMaterial.depthWrite).toBe(false);
      expect(resources.colorMaterial.blending).toBe(THREE.NoBlending);
      expect(resources.colorMaterial.uniforms.surfaceOwnerDepthMap.value).toBe(
        resources.depthTarget.depthTexture,
      );
      expect(resources.colorMaterial.uniforms.surfaceOwnerResolution).toBe(
        binding.resolution,
      );
      expect(resources.depthMaterial.uniforms.surfaceOwnerResolution).toBe(
        binding.resolution,
      );
      expect(resources.colorMaterial.uniforms.surfaceOwnerDepthTolerance.value).toBe(
        SURFACE_OWNER_DEPTH_TOLERANCE,
      );
      for (const material of [resources.depthMaterial, resources.colorMaterial]) {
        expect(material.vertexShader).toContain(
          "attribute vec4 surfaceOwnerDepthPlane;",
        );
        expect(material.fragmentShader).toContain(
          "float surfaceOwnerCanonicalDepth()",
        );
        expect(material.fragmentShader).toContain(
          "if ( vSurfaceOwnerDepthPlane.w < 0.5 ) return gl_FragCoord.z;",
        );
      }
      expect(resources.depthMaterial.fragmentShader).toContain(
        "gl_FragDepthEXT = surfaceOwnerCanonicalDepth();",
      );
      expect(resources.colorMaterial.fragmentShader).toContain(
        "candidateDepth - nearestDepth > surfaceOwnerDepthTolerance",
      );
      expect(binding.map.value).toBe(resources.colorTarget.texture);

      // 旧LEQUAL方式では、同一平面でも三角形分割ごとの1 code丸めが交互に
      // 前後して下面が残った。最前深度から2 code内を両方通し、上面を後描きすれば一様になる。
      const lowerDepthCodes = [1_000_000, 1_000_001, 1_000_000, 1_000_001];
      const upperDepthCodes = [1_000_001, 1_000_000, 1_000_001, 1_000_000];
      const owners = lowerDepthCodes.map((lower, pixel) => {
        const upper = upperDepthCodes[pixel];
        const nearest = Math.min(lower, upper);
        let owner = 0;
        if (lower - nearest <= SURFACE_OWNER_DEPTH_TOLERANCE_CODES) owner = 1;
        if (upper - nearest <= SURFACE_OWNER_DEPTH_TOLERANCE_CODES) owner = 2;
        return owner;
      });
      expect(owners).toEqual([2, 2, 2, 2]);
    } finally {
      disposeSurfaceOwnerPassResources(resources);
    }
  });

  it("24-bit depthの2 code許容は既定層間隔の視線方向差より十分小さい", () => {
    const maxDepthCode = 2 ** SURFACE_OWNER_DEPTH_BITS - 1;
    expect(SURFACE_OWNER_DEPTH_TOLERANCE * maxDepthCode).toBe(
      SURFACE_OWNER_DEPTH_TOLERANCE_CODES,
    );

    // 測定済みの斜め視点: 紙中心まで1.6295941546、層間隔0.0002の
    // 視線方向成分0.0001437292001。near/farは製品cameraと同じ0.01/100。
    const near = 0.01;
    const far = 100;
    const depth = (distance: number) =>
      far / (far - near) - (far * near) / ((far - near) * distance);
    const centerDistance = 1.6295941546;
    const layerViewDistance = 0.0001437292001;
    const layerDepthCodes =
      Math.abs(depth(centerDistance + layerViewDistance) - depth(centerDistance)) *
      maxDepthCode;

    expect(layerDepthCodes).toBeGreaterThan(9);
    expect(layerDepthCodes).toBeLessThan(9.2);
    expect(layerDepthCodes).toBeGreaterThan(4 * SURFACE_OWNER_DEPTH_TOLERANCE_CODES);
  });

  it("owner passの両targetを同じ物理画素へresizeし、GPU資源を全て破棄する", () => {
    const binding = createSurfaceOwnerBinding();
    const resources = createSurfaceOwnerPassResources(binding);
    const disposeDepthMaterial = vi.spyOn(resources.depthMaterial, "dispose");
    const disposeColorMaterial = vi.spyOn(resources.colorMaterial, "dispose");
    const disposeDepthTarget = vi.spyOn(resources.depthTarget, "dispose");
    const disposeColorTarget = vi.spyOn(resources.colorTarget, "dispose");

    resizeSurfaceOwnerPassResources(resources, binding, 640.9, 480.8);
    expect([resources.depthTarget.width, resources.depthTarget.height]).toEqual([640, 480]);
    expect([resources.colorTarget.width, resources.colorTarget.height]).toEqual([640, 480]);
    expect(binding.resolution.value.toArray()).toEqual([640, 480]);
    expect(binding.map.value).toBe(resources.colorTarget.texture);

    // RenderTarget.setSize自身も再確保のためdispose eventを発火する。明示破棄だけを数える。
    disposeDepthTarget.mockClear();
    disposeColorTarget.mockClear();
    disposeSurfaceOwnerPassResources(resources);
    expect(disposeDepthMaterial).toHaveBeenCalledOnce();
    expect(disposeColorMaterial).toHaveBeenCalledOnce();
    expect(disposeDepthTarget).toHaveBeenCalledOnce();
    expect(disposeColorTarget).toHaveBeenCalledOnce();
  });

  it("通常材質へowner attribute・共有uniform・fragment discardを1組だけ注入する", () => {
    const binding = createSurfaceOwnerBinding();
    const material = new THREE.MeshLambertMaterial();
    filterMaterialBySurfaceOwnerAttribute(material, binding, 0);

    const shader = compileHook(material);

    expect(shader.vertexShader).toContain("attribute vec4 surfaceOwnerToken;");
    expect(shader.vertexShader).toContain("vSurfaceOwnerToken = surfaceOwnerToken;");
    expect(shader.fragmentShader).toContain(
      "if ( ! surfaceOwnerVisible( vSurfaceOwnerToken ) ) discard;",
    );
    expect(shader.uniforms.surfaceOwnerEnabled).toBe(binding.enabled);
    expect(shader.uniforms.surfaceOwnerMap).toBe(binding.map);
    expect(shader.uniforms.surfaceOwnerResolution).toBe(binding.resolution);
    expect(shader.uniforms.surfaceOwnerMode.value).toBe(1);
    expect(shader.uniforms.surfaceOwnerRadius.value).toBe(0);
  });

  it("別owner画素では近傍探索をせず、背景側だけ外周線の近傍を調べる", () => {
    const material = new THREE.LineBasicMaterial();
    filterMaterialBySurfaceOwnerAttribute(material, createSurfaceOwnerBinding(), 1);
    const shader = compileHook(material);
    const fragment = shader.fragmentShader;

    const foreignGuard = fragment.indexOf(
      "if ( ! surfaceOwnerIsBackground( actual ) ) return false;",
    );
    const neighborLoop = fragment.indexOf("for ( int step = 1;");
    expect(foreignGuard).toBeGreaterThan(-1);
    expect(neighborLoop).toBeGreaterThan(foreignGuard);
  });

  it("黒outlineはedge固有probeの内向き1pxだけをexact照合し、一般近傍を使わない", () => {
    const binding = createSurfaceOwnerBinding();
    const material = new THREE.LineBasicMaterial();
    filterOutlineMaterialBySurfaceOwner(material, binding);
    const shader = compileHook(material);

    expect(shader.vertexShader).toContain("attribute vec3 surfaceOwnerOther;");
    expect(shader.vertexShader).toContain("attribute vec3 surfaceOwnerProbe;");
    expect(shader.vertexShader).toContain("abs( ownerProbeSide ) > 1e-4");
    expect(shader.fragmentShader).toContain(
      "surfaceOwnerOutlineMatches( gl_FragCoord.xy + normalize( vSurfaceOwnerInward ) )",
    );
    expect(shader.fragmentShader).toContain(
      "if ( vSurfaceOwnerInwardValid < 0.5 ) return false;",
    );
    expect(shader.fragmentShader).not.toContain("for ( int step");
    expect(shader.fragmentShader).not.toContain("surfaceOwnerIsBackground");
    expect(shader.uniforms.surfaceOwnerMap).toBe(binding.map);
    expect(material.userData.surfaceOwnerFilter).toBe("outline-inward");
  });

  it("outline geometryはedgeごとに頂点を複製し、反対端・第3頂点・ownerを対応させる", () => {
    const sourcePosition = new THREE.BufferAttribute(
      new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]),
      3,
    );
    const sourceToken = new THREE.BufferAttribute(
      new Uint8Array([1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]),
      4,
      true,
    );
    const outline = createSurfaceOwnerOutlineGeometry({
      sourcePosition,
      sourceToken,
      lineIndices: [0, 1],
      lineProbeIndices: [2],
    });
    try {
      expect(outline.geometry.getIndex()).toBeNull();
      expect(outline.geometry.getAttribute("position").count).toBe(2);
      expect(outline.geometry.getAttribute("surfaceOwnerOther").array).toEqual(
        new Float32Array([1, 0, 0, 0, 0, 0]),
      );
      expect(outline.geometry.getAttribute("surfaceOwnerProbe").array).toEqual(
        new Float32Array([0, 1, 0, 0, 1, 0]),
      );
      expect(outline.geometry.getAttribute("surfaceOwnerToken").array).toEqual(
        new Uint8Array([1, 0, 0, 0, 1, 0, 0, 0]),
      );
      expect(outline.endpointSources).toEqual(new Int32Array([0, 1]));
      expect(outline.otherSources).toEqual(new Int32Array([1, 0]));
      expect(outline.probeSources).toEqual(new Int32Array([2, 2]));
    } finally {
      outline.geometry.dispose();
    }
  });

  it("outlineのdirect copy契約外のposition属性は作成時に拒否する", () => {
    const sourceToken = new THREE.BufferAttribute(new Uint8Array(8), 4, true);
    expect(() =>
      createSurfaceOwnerOutlineGeometry({
        sourcePosition: new THREE.BufferAttribute(new Float32Array(4), 2),
        sourceToken,
        lineIndices: [0, 1],
        lineProbeIndices: [0],
      }),
    ).toThrow(/unnormalized xyz/);
  });

  it("LineMaterialの既存shaderを保ちつつowner判定をmainの先頭へ加える", () => {
    const material = new LineMaterial() as FilteredLineMaterial;
    const original = material.fragmentShader;
    filterLineMaterialBySurfaceOwner(material, createSurfaceOwnerBinding());

    expect(material.fragmentShader).toContain("uniform sampler2D surfaceOwnerMap;");
    expect(material.fragmentShader).toContain(
      "if ( ! surfaceOwnerLineVisible( surfaceOwnerExpected ) ) discard;",
    );
    expect(material.fragmentShader).toContain("vec2 center = surfaceOwnerLineStart + edge * along;");
    expect(material.fragmentShader).toContain(
      "surfaceOwnerLineProbeSupplied > 0.5",
    );
    expect(material.fragmentShader).toContain(
      "if ( surfaceOwnerLineCenterValid < 0.5 ) return false;",
    );
    expect(material.fragmentShader).toContain(
      "if ( surfaceOwnerLineInwardValid < 0.5 ) return false;",
    );
    const centerForeignGuard = material.fragmentShader.indexOf(
      "if ( ! surfaceOwnerIsBackground( centerOwner ) ) {",
    );
    const foreignOnePixelOnly = material.fragmentShader.indexOf(
      "return surfaceOwnerSampleMatches( center + surfaceOwnerLineInward, expected );",
    );
    const directedLoop = material.fragmentShader.indexOf(
      "center + surfaceOwnerLineInward * float( step )",
    );
    const inwardForeignGuard = material.fragmentShader.indexOf(
      "if ( ! surfaceOwnerIsBackground( inwardOwner ) ) return false;",
    );
    expect(centerForeignGuard).toBeGreaterThan(-1);
    expect(foreignOnePixelOnly).toBeGreaterThan(centerForeignGuard);
    expect(directedLoop).toBeGreaterThan(foreignOnePixelOnly);
    expect(inwardForeignGuard).toBeGreaterThan(directedLoop);
    expect(material.fragmentShader).toContain("vec4 diffuseColor = vec4( diffuse, alpha );");
    expect(material.fragmentShader.length).toBeGreaterThan(original.length);
    expect(material.uniforms.surfaceOwnerMode.value).toBe(2);
  });

  it("LineMaterialのCSS px半径をDPR相当の物理画素へ直し、上限で止める", () => {
    const binding = createSurfaceOwnerBinding();
    const texture = new THREE.Texture();
    bindSurfaceOwner(binding, texture, 1200, 800);
    const material = new LineMaterial() as FilteredLineMaterial;
    filterLineMaterialBySurfaceOwner(material, binding);
    material.resolution.set(600, 400);

    setLineSurfaceOwner(material, "exact", ownerCodeVector(7), 4);
    expect(material.uniforms.surfaceOwnerMode.value).toBe(1);
    expect(material.uniforms.surfaceOwnerRadius.value).toBe(8);
    expect(material.uniforms.surfaceOwnerLineProbeSupplied.value).toBe(0);
    expect(material.uniforms.surfaceOwnerLineCenterValid.value).toBe(0);
    expect(material.uniforms.surfaceOwnerLineInwardValid.value).toBe(0);
    expect(
      (material.uniforms.surfaceOwnerExpected.value as THREE.Vector4).toArray(),
    ).toEqual(ownerCodeVector(7).toArray());

    setLineSurfaceOwner(material, "any", ownerCodeVector(0), 99);
    expect(material.uniforms.surfaceOwnerMode.value).toBe(2);
    expect(material.uniforms.surfaceOwnerRadius.value).toBe(SURFACE_OWNER_MAX_RADIUS_PX);

    setLineSurfaceOwner(material, "bypass", ownerCodeVector(0), 0);
    expect(material.uniforms.surfaceOwnerMode.value).toBe(0);
  });

  it("太い強調線は画面上の中心線と所有面内向きを物理画素uniformへ写す", () => {
    const binding = createSurfaceOwnerBinding();
    bindSurfaceOwner(binding, new THREE.Texture(), 200, 100);
    const material = new LineMaterial() as FilteredLineMaterial;
    filterLineMaterialBySurfaceOwner(material, binding);
    const camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0.1, 10);
    camera.position.set(0, 0, 2);
    camera.lookAt(0, 0, 0);
    camera.updateProjectionMatrix();
    camera.updateMatrixWorld(true);
    const start = new THREE.Vector3(-0.5, 0, 0);
    const end = new THREE.Vector3(0.5, 0, 0);

    setLineSurfaceOwner(material, "exact", ownerCodeVector(1), 3, {
      camera,
      start,
      end,
      inside: new THREE.Vector3(0, 0.5, 0),
    });
    expect(material.uniforms.surfaceOwnerLineProbeSupplied.value).toBe(1);
    expect(material.uniforms.surfaceOwnerLineCenterValid.value).toBe(1);
    expect(material.uniforms.surfaceOwnerLineInwardValid.value).toBe(1);
    expect(
      (material.uniforms.surfaceOwnerLineStart.value as THREE.Vector2).toArray(),
    ).toEqual([50, 50]);
    expect(
      (material.uniforms.surfaceOwnerLineEnd.value as THREE.Vector2).toArray(),
    ).toEqual([150, 50]);
    const inward = material.uniforms.surfaceOwnerLineInward.value as THREE.Vector2;
    expect(inward.x).toBeCloseTo(0, 12);
    expect(inward.y).toBeCloseTo(1, 12);

    setLineSurfaceOwner(material, "exact", ownerCodeVector(1), 3, {
      camera,
      start,
      end,
      inside: new THREE.Vector3(0, 0, 0),
    });
    expect(material.uniforms.surfaceOwnerLineProbeSupplied.value).toBe(1);
    expect(material.uniforms.surfaceOwnerLineCenterValid.value).toBe(1);
    expect(material.uniforms.surfaceOwnerLineInwardValid.value).toBe(0);

    setLineSurfaceOwner(material, "exact", ownerCodeVector(1), 3, {
      camera,
      start,
      end: start,
      inside: new THREE.Vector3(0, 0.5, 0),
    });
    expect(material.uniforms.surfaceOwnerLineProbeSupplied.value).toBe(1);
    expect(material.uniforms.surfaceOwnerLineCenterValid.value).toBe(0);
    expect(material.uniforms.surfaceOwnerLineInwardValid.value).toBe(0);
  });

  it("Line2本来の描画前処理を保ち、更新済みviewportでowner半径を換算する", () => {
    const binding = createSurfaceOwnerBinding();
    bindSurfaceOwner(binding, new THREE.Texture(), 800, 600);
    const layer = createHighlightLayer(binding);
    try {
      layer.setOwnerCodes(new Map([[12, 1]]));
      layer.setSegments([
        {
          edgeId: 3,
          ownerFace: 12,
          a: new THREE.Vector3(0, 0, 0),
          b: new THREE.Vector3(1, 0, 0),
        },
      ]);
      const line = layer.group.children[0] as Line2;
      const material = line.material as FilteredLineMaterial & {
        resolution: THREE.Vector2;
      };
      const renderer = {
        getViewport(target: THREE.Vector4) {
          return target.set(0, 0, 400, 300);
        },
      } as THREE.WebGLRenderer;

      line.onBeforeRender(renderer);

      expect(material.resolution.toArray()).toEqual([400, 300]);
      // 4 CSS px線の半幅2px + 外周余白1pxを、DPR 2の物理画素へ直す。
      expect(material.uniforms.surfaceOwnerRadius.value).toBe(6);
      expect(material.uniforms.surfaceOwnerMode.value).toBe(1);
    } finally {
      layer.dispose();
    }
  });

  it("共有LineMaterialを各Line2の描画直前にowner・中心線・内向きまで切り替える", () => {
    const binding = createSurfaceOwnerBinding();
    bindSurfaceOwner(binding, new THREE.Texture(), 800, 600);
    const layer = createHighlightLayer(binding);
    try {
      layer.setOwnerCodes(
        new Map([
          [12, 1],
          [13, 2],
        ]),
      );
      layer.setSegments([
        {
          edgeId: 3,
          ownerFace: 12,
          a: new THREE.Vector3(-0.5, 0, 0),
          b: new THREE.Vector3(0.5, 0, 0),
          surfaceProbe: new THREE.Vector3(0, 0.5, 0),
        },
        {
          edgeId: 4,
          ownerFace: 13,
          a: new THREE.Vector3(0, -0.5, 0),
          b: new THREE.Vector3(0, 0.5, 0),
          surfaceProbe: new THREE.Vector3(-0.5, 0, 0),
        },
      ]);
      const first = layer.group.children[0] as Line2;
      const second = layer.group.children[1] as Line2;
      expect(first.material).toBe(second.material);
      const material = first.material as FilteredLineMaterial;
      const renderer = {
        getViewport(target: THREE.Vector4) {
          return target.set(0, 0, 400, 300);
        },
      } as THREE.WebGLRenderer;
      const camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0.1, 10);
      camera.position.set(0, 0, 2);
      camera.lookAt(0, 0, 0);
      camera.updateProjectionMatrix();
      camera.updateMatrixWorld(true);
      const beforeRender = (line: Line2) =>
        (
          line.onBeforeRender as unknown as (
            renderer: THREE.WebGLRenderer,
            scene: THREE.Scene,
            camera: THREE.Camera,
          ) => void
        )(renderer, new THREE.Scene(), camera);

      beforeRender(second);
      expect(
        (material.uniforms.surfaceOwnerExpected.value as THREE.Vector4).toArray(),
      ).toEqual(ownerCodeVector(2).toArray());
      expect(
        (material.uniforms.surfaceOwnerLineStart.value as THREE.Vector2).toArray(),
      ).toEqual([400, 150]);
      expect(
        (material.uniforms.surfaceOwnerLineEnd.value as THREE.Vector2).toArray(),
      ).toEqual([400, 450]);
      expect(
        (material.uniforms.surfaceOwnerLineInward.value as THREE.Vector2).toArray(),
      ).toEqual([-1, 0]);

      beforeRender(first);
      expect(
        (material.uniforms.surfaceOwnerExpected.value as THREE.Vector4).toArray(),
      ).toEqual(ownerCodeVector(1).toArray());
      expect(
        (material.uniforms.surfaceOwnerLineStart.value as THREE.Vector2).toArray(),
      ).toEqual([200, 300]);
      expect(
        (material.uniforms.surfaceOwnerLineEnd.value as THREE.Vector2).toArray(),
      ).toEqual([600, 300]);
      const inward = material.uniforms.surfaceOwnerLineInward.value as THREE.Vector2;
      expect(inward.x).toBeCloseTo(0, 12);
      expect(inward.y).toBeCloseTo(1, 12);
      expect(material.uniforms.surfaceOwnerLineProbeSupplied.value).toBe(1);
      expect(material.uniforms.surfaceOwnerLineCenterValid.value).toBe(1);
      expect(material.uniforms.surfaceOwnerLineInwardValid.value).toBe(1);
    } finally {
      layer.dispose();
    }
  });

  it("resize bindingはtexture参照を保ちつつ解像度と有効状態を更新する", () => {
    const binding = createSurfaceOwnerBinding();
    const texture = new THREE.Texture();
    bindSurfaceOwner(binding, texture, 0, -1);

    expect(binding.map.value).toBe(texture);
    expect(binding.resolution.value.toArray()).toEqual([1, 1]);
    expect(binding.enabled.value).toBe(1);

    bindSurfaceOwner(binding, texture, 640, 480);
    expect(binding.resolution.value.toArray()).toEqual([640, 480]);
  });

  it("実行時12材質を役割別に全数えし、owner対象10・赤迂回1・preview維持1にする", () => {
    const display = {
      front_color: [240, 80, 70] as [number, number, number],
      back_color: [245, 245, 245] as [number, number, number],
      grid_divisions: 8,
    };
    const topology = {
      slots: new Map([[3, { offset: 0, count: 3 }]]),
      vertexCount: 3,
      indices: [0, 1, 2],
      triangleFaceIds: [3],
      vertexFaceIds: [3, 3, 3],
      lineIndices: [0, 1, 1, 2, 2, 0],
      lineProbeIndices: [2, 0, 1],
      hingeSlots: [],
      flatPositions: new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]),
    };
    const binding = createSurfaceOwnerBinding();
    const rigid = createContent(topology, display, binding);
    const softMesh = {
      positions: [[0, 0, 0], [1, 0, 0], [0, 1, 0]] as [number, number, number][],
      triangles: [[0, 1, 2]] as [number, number, number][],
      triangle_faces: [3],
      triangle_layers: [0],
      warnings: [],
    };
    const soft = createSoftContent(
      softMesh,
      display,
      binding,
      rigid.owner.ownerCodes,
    );
    const highlights = createHighlightMaterials(binding);
    const rigidPaper = rigid.mesh.material as THREE.MeshLambertMaterial[];
    const softPaper = soft.mesh.material as THREE.MeshLambertMaterial[];
    const blackLines = [rigid.line.material, soft.line.material] as THREE.Material[];
    const highlightMaterials = Object.values(highlights);
    const preview = createPreviewMaterial();
    try {
      const ownerFiltered = (material: THREE.Material) =>
        material.userData.surfaceOwnerFilter !== undefined;
      expect(rigidPaper).toHaveLength(2);
      expect(softPaper).toHaveLength(2);
      expect([...rigidPaper, ...softPaper].filter(ownerFiltered)).toHaveLength(4);
      const rigidToken = rigid.owner.geometry.getAttribute("surfaceOwnerToken");
      expect(rigid.mesh.geometry.getAttribute("surfaceOwnerToken")).toBe(rigidToken);
      expect(rigid.line.geometry.getAttribute("surfaceOwnerToken").count).toBe(6);
      expect(rigid.line.geometry.getAttribute("surfaceOwnerProbe").count).toBe(6);
      const softToken = soft.owner.geometry.getAttribute("surfaceOwnerToken");
      expect(soft.mesh.geometry.getAttribute("surfaceOwnerToken")).toBe(softToken);
      expect(soft.line.geometry.getAttribute("surfaceOwnerToken").count).toBe(6);
      expect(soft.line.geometry.getAttribute("surfaceOwnerProbe").count).toBe(6);
      expect(rigidPaper.every((material) => material.userData.surfaceOwnerRadiusPx === 10)).toBe(
        true,
      );
      expect(softPaper.every((material) => material.userData.surfaceOwnerRadiusPx === 10)).toBe(
        true,
      );
      softMesh.triangle_layers[0] = 7;
      updateSoftContent(soft, softMesh, null);
      expect(soft.owner.triangleLayers).toEqual([7]);
      expect(blackLines).toHaveLength(2);
      expect(blackLines.filter(ownerFiltered)).toHaveLength(2);
      expect(highlightMaterials).toHaveLength(5);
      expect(highlightMaterials.filter(ownerFiltered)).toHaveLength(4);
      expect(highlights.suspectHighlightMaterial.depthTest).toBe(false);
      expect(ownerFiltered(highlights.suspectHighlightMaterial)).toBe(false);
      expect(preview.depthTest).toBe(false);
      expect(ownerFiltered(preview)).toBe(false);
      expect([
        ...rigidPaper,
        ...softPaper,
        ...blackLines,
        ...highlightMaterials,
        preview,
      ]).toHaveLength(12);
    } finally {
      rigid.mesh.geometry.dispose();
      rigid.line.geometry.dispose();
      for (const material of rigidPaper) material.dispose();
      (rigid.line.material as THREE.Material).dispose();
      disposeSurfaceOwnerSurface(rigid.owner);
      soft.mesh.geometry.dispose();
      soft.line.geometry.dispose();
      for (const material of softPaper) material.dispose();
      (soft.line.material as THREE.Material).dispose();
      disposeSurfaceOwnerSurface(soft.owner);
      for (const material of highlightMaterials) material.dispose();
      preview.dispose();
    }
  });
});
