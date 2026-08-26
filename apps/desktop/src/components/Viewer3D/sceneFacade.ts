// 3Dビューの外向けscene facade。scene資源の所有権と破棄を一か所に集める。
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import {
  registerViewer3DReadback,
  type Viewer3DReadback,
} from "../../captureReadbackBridge";
import type { Vec2 } from "../../lib/types";
import type { HingeSegment, PaperPickSurface } from "./hingePicker";
import {
  createSurfaceOwnerBinding,
  disposeSurfaceOwnerSurface,
  orderSurfaceOwner,
  type SurfaceOwnerBinding,
  type SurfaceOwnerSurface,
} from "./surfaceOwner";
import {
  createSurfaceOwnerPassResources,
  disposeSurfaceOwnerPassResources,
  resizeSurfaceOwnerPassResources,
} from "./surfaceOwnerShader";
import {
  triangulate,
  type SoftContent,
  type Viewer3DContent,
} from "./sceneContent";
import {
  CAMERA_DIR,
  CAMERA_FOV,
  MIN_ORBIT_RADIUS,
  applyCameraDragRotation,
  applyPaperFraming,
  boxFraming,
  cameraScreenUp,
  legacyBoxDistance,
  viewRotationStarts,
  type PaperFraming,
} from "./sceneCamera";
import {
  clearGroup,
  createHighlightLayer,
  createPreviewMaterial,
  createSupplementalEdgeLayer,
  disposeDrawable,
  type HighlightSegment,
} from "./sceneLayers";

/** CSSが読み込まれない単体テスト環境で使うPOPテーマの背景色。 */
const DEFAULT_BACKGROUND_COLOR = "#cfcbc2";

/** App.cssで選択中テーマの3D背景色を読む。 */
export function canvas3dBackgroundColor(canvas: HTMLCanvasElement): string {
  if (typeof getComputedStyle !== "function") return DEFAULT_BACKGROUND_COLOR;
  return (
    getComputedStyle(canvas).getPropertyValue("--color-canvas-3d").trim() ||
    DEFAULT_BACKGROUND_COLOR
  );
}

// ---------------------------------------------------------------------------
// シーン(レンダラ・カメラ・照明・入れ物)
// ---------------------------------------------------------------------------

export interface Viewer3DScene {
  readonly camera: THREE.PerspectiveCamera;
  /** 面と境界線を入れる入れ物(作り直しのたびに中身を破棄する) */
  readonly contentGroup: THREE.Group;
  /** 選択中の辺や折り線プレビューの強調を入れる入れ物 */
  readonly highlightGroup: THREE.Group;
  /** 全surface-aware材質が共有するowner textureのuniform。 */
  readonly ownerBinding: SurfaceOwnerBinding;
  /** 表示中の面・線。展開図が変わるまで作り替えない */
  content: Viewer3DContent | null;
  /** 現在実際に表示しているrigid/soft面。pickerも同じ面を使う。 */
  pickSurface?: PaperPickSurface | null;
  /** 次の描画機会に1回だけ描く(1フレーム1回にまとめる) */
  render(): void;
  /** 現在のテーマのCSS変数から背景色を読み直して描画する。 */
  syncTheme(): void;
  /**
   * 3D区画の大きさが変わったことを伝える。hintBottomPx を渡すと、案内の札の
   * 高さが変わったぶんも合わせ直しに反映する。
   */
  resize(widthPx: number, heightPx: number, hintBottomPx?: number): void;
  /**
   * 立体全体が見える斜め上の初期位置へカメラを戻す。
   * box には「いま実際に表示している形」が占める範囲(展開図の大きさではない。
   * 折る・技法で座標は動く)を渡す。hintBottomPx に左上の案内の札の下端
   * (区画の上からのCSS px)を渡すと、立体を小さくしすぎない範囲で札の下から逃がす。
   */
  resetCamera(box: THREE.Box3, hintBottomPx?: number): void;
  /** 面と線を差し替える(古い資源は破棄する) */
  setContent(content: Viewer3DContent): void;
  /**
   * たわみの網を表示する(SIM-012)。渡している間は面ごとの多角形の代わりに
   * 細かい三角形の網を描く。nullで従来の描き方へ戻る。
   *
   * 元の面(content.mesh)は入れ物から外すだけで捨てない。当たり判定も表示中の
   * 細分網へ切り替え、owner passと同じ可視面を選ぶ。面IDは細分前のIDを保つため、
   * たわみを入れても折る・つかむ操作の意味は変わらない。
   */
  setSoft(soft: SoftContent | null): void;
  /**
   * 面境界へ入らない既存線を、表示中の紙と同じ黒い線・表面判定で描く。
   * 呼び出し側が選択中の線を除き、現在のrigid/soft座標へ写した線分を渡す。
   */
  setSupplementalEdges(segments: readonly HingeSegment[]): void;
  /** 選択中の辺の強調を更新する(形と材質は使い回す) */
  setHighlight(segments: HighlightSegment[]): void;
  /**
   * 折った結果の下見を半透明の面で重ねる(UI-008)。
   * 多角形は畳み平面(z=0)の座標で、liftだけ持ち上げて描く。
   * 空配列を渡すと消える。
   */
  setPreview(polygons: Vec2[][], lift: number): void;
  /**
   * 折り線の描画中・紙を引いている間は左ドラッグの視点回転を止める
   * (拡大縮小・平行移動は残す)。
   * rotateWithRightを立てると、代わりに右ドラッグで視点を回せる
   * (立体を色々な向きから見ながら引くため。平行移動は中ボタンへ移る)
   */
  setDrawMode(enabled: boolean, rotateWithRight?: boolean): void;
  dispose(): void;
}

/**
 * CDPの恒久検査が、画面に出たものと同じ3段の描画結果を照合するための読取結果。
 * WebGLのreadPixelsに合わせ、全bufferは左下を先頭にした物理画素順で返す。
 */
type Viewer3DReadbackSource = () => Viewer3DReadback;

export { captureViewer3DReadback } from "../../captureReadbackBridge";
export type { Viewer3DReadback } from "../../captureReadbackBridge";

function bytesAsBase64(bytes: Uint8Array): string {
  const chunkSize = 0x8000;
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize));
  }
  return btoa(binary);
}

export interface PackedDepthReadbackResources {
  readonly target: THREE.WebGLRenderTarget;
  readonly material: THREE.ShaderMaterial;
  readonly geometry: THREE.PlaneGeometry;
  readonly scene: THREE.Scene;
  readonly camera: THREE.Camera;
}

/** captureを初めて要求された時だけ作り、通常表示にはGPU資源を増やさない。 */
export function createPackedDepthReadbackResources(
  depthTexture: THREE.DepthTexture | null,
): PackedDepthReadbackResources {
  if (depthTexture === null) throw new Error("owner depth textureがありません");
  const target = new THREE.WebGLRenderTarget(1, 1, {
    format: THREE.RGBAFormat,
    type: THREE.UnsignedByteType,
    minFilter: THREE.NearestFilter,
    magFilter: THREE.NearestFilter,
    depthBuffer: false,
    stencilBuffer: false,
  });
  target.texture.generateMipmaps = false;
  target.texture.colorSpace = THREE.NoColorSpace;
  const material = new THREE.ShaderMaterial({
    uniforms: {
      surfaceOwnerDepthMap: { value: depthTexture },
    },
    vertexShader: /* glsl */ `
      varying vec2 vDepthReadbackUv;
      void main() {
        vDepthReadbackUv = uv;
        gl_Position = vec4( position.xy, 0.0, 1.0 );
      }
    `,
    fragmentShader: /* glsl */ `
      uniform sampler2D surfaceOwnerDepthMap;
      varying vec2 vDepthReadbackUv;
      #include <packing>
      void main() {
        gl_FragColor = packDepthToRGBA(
          texture2D( surfaceOwnerDepthMap, vDepthReadbackUv ).x
        );
      }
    `,
    depthTest: false,
    depthWrite: false,
    blending: THREE.NoBlending,
    toneMapped: false,
  });
  const geometry = new THREE.PlaneGeometry(2, 2);
  const mesh = new THREE.Mesh(geometry, material);
  mesh.frustumCulled = false;
  const scene = new THREE.Scene();
  scene.add(mesh);
  return { target, material, geometry, scene, camera: new THREE.Camera() };
}

export function disposePackedDepthReadbackResources(
  resources: PackedDepthReadbackResources,
): void {
  resources.geometry.dispose();
  resources.material.dispose();
  resources.target.dispose();
}


/** canvasにレンダラ・カメラ・軌道操作・照明を用意する */
export function createScene(canvas: HTMLCanvasElement): Viewer3DScene {
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
  renderer.setPixelRatio(window.devicePixelRatio || 1);

  // 1画素につき最前面の面IDをRGBA8へ書く前処理。最前深度とowner色を分けるため
  // draw callは面数によらず2回だけ増える。全三角形はどちらも動的index 1本で描く。
  const ownerBinding = createSurfaceOwnerBinding();
  const ownerPass = createSurfaceOwnerPassResources(ownerBinding);
  // WebView2/ANGLEはdepth attachmentへのDEPTH_COMPONENT readPixelsを
  // GL_INVALID_ENUMにするため、検査時だけ既存depth textureをRGBA8へGPU copyする。
  // productionのdepth/owner/final passには入れず、同じtextureを読むだけにする。
  let depthReadback: PackedDepthReadbackResources | null = null;
  const emptyOwnerGeometry = new THREE.BufferGeometry();
  const ownerMesh = new THREE.Mesh(emptyOwnerGeometry, ownerPass.colorMaterial);
  ownerMesh.frustumCulled = false;
  ownerMesh.visible = false;
  const ownerScene = new THREE.Scene();
  ownerScene.add(ownerMesh);
  let ownerSurface: SurfaceOwnerSurface | null = null;
  const drawingBufferSize = new THREE.Vector2();
  const savedClearColor = new THREE.Color();

  const scene = new THREE.Scene();
  scene.background = new THREE.Color(canvas3dBackgroundColor(canvas));

  const camera = new THREE.PerspectiveCamera(CAMERA_FOV, 1, 0.01, 100);
  camera.position.set(0.5, -1.2, 1.4);

  // 折った面がどちら向きでも暗くなりすぎないよう、環境光+表裏2方向の平行光
  scene.add(new THREE.AmbientLight(0xffffff, 1.4));
  const key = new THREE.DirectionalLight(0xffffff, 1.6);
  key.position.set(0.4, 0.8, 1.0);
  scene.add(key);
  const fill = new THREE.DirectionalLight(0xffffff, 1.0);
  fill.position.set(-0.5, -0.8, -1.0);
  scene.add(fill);

  const contentGroup = new THREE.Group();
  const supplementalEdgeLayer = createSupplementalEdgeLayer();
  const highlightLayer = createHighlightLayer(ownerBinding);
  const highlightGroup = highlightLayer.group;
  // 折った結果の下見。紙に隠れず常に見えるよう深度判定を切って最後に描く
  const previewMaterial = createPreviewMaterial();
  const previewMesh = new THREE.Mesh(new THREE.BufferGeometry(), previewMaterial);
  previewMesh.renderOrder = 2;
  previewMesh.frustumCulled = false;
  previewMesh.visible = false;
  scene.add(contentGroup, supplementalEdgeLayer.group, highlightGroup, previewMesh);

  const controls = new OrbitControls(camera, canvas);
  controls.enableDamping = false; // 常時描画ループを持たない(変化時だけ描く)
  // 視点を回すのは下のドラッグ処理だけにする。OrbitControlsの回転は
  // 世界の上下を軸にするため真上・真下で行き止まりになる(利用者の指摘)。
  // 寄る・平行移動・ボタンの割り当て(setDrawMode)はOrbitControlsのまま使う。
  controls.enableRotate = false;

  // --- 立体を区画へ収める視点合わせ ------------------------------------------
  // 区画の大きさが変わったときにも合わせ直せるよう、材料を覚えておく。
  // 3Dの状態は保存しないので、この記憶はこの画面が生きている間だけのもの。
  // box は「いま実際に表示している形」の範囲(展開図の大きさではない)。
  let fitRequest: {
    box: THREE.Box3;
    hintBottomPx: number;
  } | null = null;
  let viewWidth = 0;
  let viewHeight = 0;
  const framingRight = new THREE.Vector3();
  const framingUp = new THREE.Vector3();
  const framingDir = new THREE.Vector3();
  const framingCenter = new THREE.Vector3();
  const framingOffset = new THREE.Vector3();

  /**
   * 覚えている立体の範囲と、渡された視線・画面の上向きから枠を求める。
   * 区画の大きさがまだ届いていない間は求めない(nullを返す)。
   */
  const framingFor = (dir: THREE.Vector3, up: THREE.Vector3): PaperFraming | null => {
    if (fitRequest === null || viewWidth <= 0 || viewHeight <= 0) return null;
    framingRight.crossVectors(up, dir);
    if (framingRight.lengthSq() <= MIN_ORBIT_RADIUS ** 2) return null;
    framingRight.normalize();
    framingUp.crossVectors(dir, framingRight).normalize();
    return boxFraming(
      fitRequest.box,
      viewWidth,
      viewHeight,
      fitRequest.hintBottomPx,
      dir,
      framingRight,
      framingUp,
    );
  };

  /** 枠を求められないとき(紙の大きさがまだ届いていない等)の素直な投影。 */
  const plainProjection = () => {
    if (viewWidth <= 0 || viewHeight <= 0) return;
    camera.clearViewOffset();
    camera.aspect = viewWidth / viewHeight;
    camera.updateProjectionMatrix();
  };

  /**
   * 区画の大きさが変わったあとに合わせ直す。向きと注視点はそのままにして、
   * 枠を作り直し、狭くなって紙が収まらなくなったぶんだけカメラを引く。
   * 利用者が寄せた分を勝手に戻さないよう、寄せる向きへは動かさない。
   */
  const refitToViewport = () => {
    framingDir.copy(camera.position).sub(controls.target);
    const framing =
      fitRequest === null || framingDir.lengthSq() <= MIN_ORBIT_RADIUS ** 2
        ? null
        : framingFor(framingDir.normalize(), cameraScreenUp(camera));
    if (framing === null || fitRequest === null) {
      plainProjection();
      return;
    }
    applyPaperFraming(camera, framing, viewWidth, viewHeight);
    camera.updateProjectionMatrix();
    fitRequest.box.getCenter(framingCenter);
    const current = framingOffset
      .copy(camera.position)
      .sub(framingCenter)
      .dot(framingDir);
    if (current < framing.distance) {
      camera.position.addScaledVector(framingDir, framing.distance - current);
      controls.update();
    }
  };

  // 描画は1フレームに1回だけ。連続した変化(座標更新・選択・視点操作)が
  // 同じフレームに重なっても描画は1回にまとまる
  let frameHandle: number | null = null;
  let disposed = false;
  const drawProductionFrame = () => {
    if (ownerSurface !== null) {
      orderSurfaceOwner(ownerSurface, camera);
      const previousTarget = renderer.getRenderTarget();
      renderer.getClearColor(savedClearColor);
      const previousClearAlpha = renderer.getClearAlpha();
      const previousOverrideMaterial = ownerScene.overrideMaterial;
      try {
        renderer.setClearColor(0x000000, 0);

        // Pass 1: 三角形分割や描画順に依存しない最前深度だけを確定する。
        ownerScene.overrideMaterial = ownerPass.depthMaterial;
        renderer.setRenderTarget(ownerPass.depthTarget);
        renderer.render(ownerScene, camera);

        // Pass 2: depth textureを読む別targetへ、最前深度とtieの面だけを
        // layer/面の鏡映偶奇/決定的fallback順で上書きする。描画中target自身は
        // 読まないためfeedbackにならない。
        ownerScene.overrideMaterial = null;
        renderer.setRenderTarget(ownerPass.colorTarget);
        renderer.render(ownerScene, camera);
      } finally {
        ownerScene.overrideMaterial = previousOverrideMaterial;
        renderer.setRenderTarget(previousTarget);
        renderer.setClearColor(savedClearColor, previousClearAlpha);
      }
    }
    renderer.render(scene, camera);
  };
  const draw = () => {
    frameHandle = null;
    if (disposed) return;
    drawProductionFrame();
  };
  const render = () => {
    if (!disposed && frameHandle === null) frameHandle = requestAnimationFrame(draw);
  };
  controls.addEventListener("change", render);

  // --- 視点のドラッグ回転 -------------------------------------------------
  // 押した位置からの差分だけを毎回渡す。OrbitControlsの回転と同じ量になる。
  let rotateDrag: { pointerId: number; x: number; y: number } | null = null;

  /**
   * カメラのupを、いま画面が上と見なしている向きへそろえる。
   * 視点立方体で移った直後はupが取り残されるため、操作を始める前に必ず合わせる。
   * これをしないと、寄る・平行移動のときのOrbitControlsの向き直しで傾きが跳ぶ。
   */
  const syncCameraUp = () => {
    camera.up.copy(cameraScreenUp(camera));
  };

  const onCanvasPointerDown = (event: PointerEvent) => {
    syncCameraUp();
    if (rotateDrag !== null) {
      // 2本目の指が触れたらOrbitControlsの2本指操作へ譲る。
      rotateDrag = null;
      return;
    }
    if (!controls.enabled) return;
    const rotates =
      event.pointerType === "touch"
        ? controls.touches.ONE === THREE.TOUCH.ROTATE
        : viewRotationStarts(
            controls.mouseButtons,
            event.button,
            event.ctrlKey || event.metaKey || event.shiftKey,
          );
    if (!rotates) return;
    rotateDrag = { pointerId: event.pointerId, x: event.clientX, y: event.clientY };
  };

  const onDocumentPointerMove = (event: PointerEvent) => {
    if (rotateDrag === null || rotateDrag.pointerId !== event.pointerId) return;
    const dragX = event.clientX - rotateDrag.x;
    const dragY = event.clientY - rotateDrag.y;
    rotateDrag.x = event.clientX;
    rotateDrag.y = event.clientY;
    if (dragX === 0 && dragY === 0) return;
    applyCameraDragRotation(camera, controls.target, dragX, dragY, canvas.clientHeight);
    render();
  };

  const onDocumentPointerUp = (event: PointerEvent) => {
    if (rotateDrag !== null && rotateDrag.pointerId === event.pointerId) {
      rotateDrag = null;
    }
  };

  const documentOf = canvas.ownerDocument;
  canvas.addEventListener("pointerdown", onCanvasPointerDown);
  canvas.addEventListener("wheel", syncCameraUp, { capture: true });
  documentOf.addEventListener("pointermove", onDocumentPointerMove);
  documentOf.addEventListener("pointerup", onDocumentPointerUp);
  documentOf.addEventListener("pointercancel", onDocumentPointerUp);

  // 描画資源が失われて復帰したときは描き直す(復帰直後は画面が空になるため)
  const onContextRestored = () => render();
  canvas.addEventListener("webglcontextrestored", onContextRestored);

  /** 表示中のたわみの網(null なら従来の面の描き方) */
  let soft: SoftContent | null = null;

  const showOwner = (surface: SurfaceOwnerSurface | null) => {
    ownerSurface = surface;
    ownerMesh.visible = surface !== null;
    if (surface !== null) ownerMesh.geometry = surface.geometry;
    ownerBinding.enabled.value = surface === null ? 0 : 1;
  };

  const pickSurfaceOf = (
    mesh: THREE.Mesh,
    surface: SurfaceOwnerSurface,
  ): PaperPickSurface => ({
    mesh,
    triangleFaceIds: surface.triangleFaces,
    triangleLayers: surface.triangleLayers,
    faceSurfaceRanks: surface.faceSurfaceRanks,
  });

  const captureReadback: Viewer3DReadbackSource = () => {
    if (disposed) throw new Error("3D表示の描画資源は破棄済みです");
    if (ownerSurface === null) throw new Error("読み取れる紙面がありません");
    renderer.getDrawingBufferSize(drawingBufferSize);
    const width = Math.floor(drawingBufferSize.x);
    const height = Math.floor(drawingBufferSize.y);
    if (width <= 0 || height <= 0) {
      throw new Error(`3D表示の物理画素数が不正です: ${width}x${height}`);
    }

    if (frameHandle !== null) {
      cancelAnimationFrame(frameHandle);
      frameHandle = null;
    }
    const previousTarget = renderer.getRenderTarget();
    const previousCubeFace = renderer.getActiveCubeFace();
    const previousMipmapLevel = renderer.getActiveMipmapLevel();
    const gl = renderer.getContext();
    if (gl.isContextLost()) throw new Error("3D表示のWebGL contextが失われています");
    const pixelCount = width * height;
    const finalPixels = new Uint8Array(pixelCount * 4);
    const ownerPixels = new Uint8Array(pixelCount * 4);
    const depthPixels = new Uint8Array(pixelCount * 4);
    depthReadback ??= createPackedDepthReadbackResources(
      ownerPass.depthTarget.depthTexture,
    );
    depthReadback.target.setSize(width, height);
    const productionDepthTexture = ownerPass.depthTarget.depthTexture;
    if (
      productionDepthTexture === null ||
      ownerPass.colorMaterial.uniforms.surfaceOwnerDepthMap.value !== productionDepthTexture ||
      depthReadback.material.uniforms.surfaceOwnerDepthMap.value !== productionDepthTexture
    ) {
      throw new Error("depth/owner/captureが同じproduction depth textureを参照していません");
    }
    for (const [name, target] of [
      ["depth", ownerPass.depthTarget],
      ["owner", ownerPass.colorTarget],
      ["packed depth", depthReadback.target],
    ] as const) {
      if (target.width !== width || target.height !== height) {
        throw new Error(
          `${name} targetの物理画素数が違います: ${target.width}x${target.height} != ${width}x${height}`,
        );
      }
    }
    const ensureReadSucceeded = (kind: string) => {
      const error = gl.getError();
      if (error !== gl.NO_ERROR) {
        throw new Error(`${kind}のWebGL readPixelsに失敗しました: 0x${error.toString(16)}`);
      }
    };

    try {
      // 通常画面と同じ depth -> owner -> final の順を、同じscene/materialで描く。
      renderer.setRenderTarget(null);
      drawProductionFrame();
      gl.finish();

      // preserveDrawingBufferに依存しないよう、default framebufferは描画直後に読む。
      gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, finalPixels);
      ensureReadSucceeded("最終RGBA");

      renderer.readRenderTargetPixels(
        ownerPass.colorTarget,
        0,
        0,
        width,
        height,
        ownerPixels,
      );
      ensureReadSucceeded("owner token");

      // depth attachmentをCPUから直接読まず、production pass 1のdepth textureを
      // Three.js標準のpackDepthToRGBAでcapture専用RGBA8 targetへ写して読む。
      renderer.setRenderTarget(depthReadback.target);
      renderer.render(depthReadback.scene, depthReadback.camera);
      gl.finish();
      renderer.readRenderTargetPixels(
        depthReadback.target,
        0,
        0,
        width,
        height,
        depthPixels,
      );
      ensureReadSucceeded("RGBA8へpackしたowner depth");
      if (gl.isContextLost()) throw new Error("readback中にWebGL contextが失われました");
    } finally {
      renderer.setRenderTarget(previousTarget, previousCubeFace, previousMipmapLevel);
    }

    return {
      version: 1,
      width,
      height,
      rowOrder: "bottom-to-top",
      owner: {
        encoding: "rgba8-base64",
        data: bytesAsBase64(ownerPixels),
        codeToFace: [...ownerSurface.ownerCodes.entries()]
          .map(([face, code]) => [code, face] as const)
          .sort((left, right) => left[0] - right[0]),
      },
      depth: {
        encoding: "rgba8-packed-depth-base64",
        data: bytesAsBase64(depthPixels),
      },
      final: {
        encoding: "rgba8-base64",
        data: bytesAsBase64(finalPixels),
      },
    };
  };

  const api: Viewer3DScene = {
    camera,
    contentGroup,
    highlightGroup,
    ownerBinding,
    content: null,
    pickSurface: null,
    render,
    syncTheme() {
      scene.background = new THREE.Color(canvas3dBackgroundColor(canvas));
      render();
    },
    resize(widthPx, heightPx, hintBottomPx) {
      if (widthPx === 0 || heightPx === 0) return;
      viewWidth = widthPx;
      viewHeight = heightPx;
      if (fitRequest !== null && hintBottomPx !== undefined) {
        fitRequest.hintBottomPx = hintBottomPx;
      }
      // 画面の細かさは移動先の画面で変わることがあるので毎回合わせ直す
      renderer.setPixelRatio(window.devicePixelRatio || 1);
      renderer.setSize(widthPx, heightPx, false);
      renderer.getDrawingBufferSize(drawingBufferSize);
      resizeSurfaceOwnerPassResources(
        ownerPass,
        ownerBinding,
        drawingBufferSize.x,
        drawingBufferSize.y,
      );
      // 区画の大きさが変わると収まり方も変わる。向きと注視点はそのままに、
      // 枠を作り直し、狭くなって収まらなくなったぶんだけカメラを引く。
      refitToViewport();
      render();
    },
    resetCamera(box, hintBottomPx = 0) {
      // 空の範囲(頂点が1つも無い等)は原点まわりの小さな箱に置き換え、
      // NaN/Infinityで視点計算が壊れないようにする。
      const safeBox = box.isEmpty()
        ? new THREE.Box3(
            new THREE.Vector3(-0.5, -0.5, 0),
            new THREE.Vector3(0.5, 0.5, 0),
          )
        : box.clone();
      fitRequest = { box: safeBox, hintBottomPx };
      const center = safeBox.getCenter(new THREE.Vector3());
      // 回して傾いたままの上向きを持ち込まないよう、世界の上へ戻してから向き直す。
      camera.up.set(0, 1, 0);
      controls.target.copy(center);
      const framing = framingFor(CAMERA_DIR, camera.up);
      const distance =
        framing?.distance ?? legacyBoxDistance(safeBox.getSize(new THREE.Vector3()));
      camera.position.copy(center).addScaledVector(CAMERA_DIR, distance);
      if (framing === null) plainProjection();
      else applyPaperFraming(camera, framing, viewWidth, viewHeight);
      camera.updateProjectionMatrix();
      controls.update();
      render();
    },
    setContent(content) {
      api.setSoft(null); // たわみの表示物を片付け、外していた面・線を入れ物へ戻す
      if (api.content !== null && api.content !== content) {
        disposeSurfaceOwnerSurface(api.content.owner);
      }
      clearGroup(contentGroup);
      api.content = content;
      contentGroup.add(content.mesh, content.line);
      showOwner(content.owner);
      highlightLayer.setOwnerCodes(content.owner.ownerCodes);
      api.pickSurface = pickSurfaceOf(content.mesh, content.owner);
      render();
    },
    setSoft(next) {
      // 座標・owner token・共有materialの参照元が切り替わる。呼び出し側が現在表示中の
      // 面へ写した線分を直後に渡し直すまで、古い面の補足線は表示しない。
      supplementalEdgeLayer.clear();
      if (soft !== null && soft !== next) {
        contentGroup.remove(soft.mesh, soft.line);
        disposeDrawable(soft.mesh);
        disposeDrawable(soft.line);
        disposeSurfaceOwnerSurface(soft.owner);
      }
      soft = next;
      const base = api.content;
      if (next !== null) {
        if (base) contentGroup.remove(base.mesh, base.line);
        if (next.mesh.parent !== contentGroup) contentGroup.add(next.mesh, next.line);
        showOwner(next.owner);
        highlightLayer.setOwnerCodes(next.owner.ownerCodes);
        api.pickSurface = pickSurfaceOf(next.mesh, next.owner);
      } else if (base && base.mesh.parent !== contentGroup) {
        contentGroup.add(base.mesh, base.line);
        showOwner(base.owner);
        highlightLayer.setOwnerCodes(base.owner.ownerCodes);
        api.pickSurface = pickSurfaceOf(base.mesh, base.owner);
      } else if (!base) {
        showOwner(null);
        api.pickSurface = null;
      }
      render();
    },
    setSupplementalEdges(segments) {
      const displayed = soft ?? api.content;
      const material = displayed?.line.material;
      if (
        !displayed ||
        Array.isArray(material) ||
        !(material instanceof THREE.LineBasicMaterial)
      ) {
        supplementalEdgeLayer.clear();
      } else {
        supplementalEdgeLayer.setSegments(segments, material, displayed.owner.ownerCodes);
      }
      render();
    },
    setHighlight(segments) {
      highlightLayer.setSegments(segments);
      render();
    },
    setPreview(polygons, lift) {
      const points: number[] = [];
      const indices: number[] = [];
      for (const poly of polygons) {
        if (poly.length < 3) continue;
        const base = points.length / 3;
        for (const p of poly) points.push(p[0], p[1], lift);
        for (const t of triangulate(poly)) {
          indices.push(base + t[0], base + t[1], base + t[2]);
        }
      }
      // 形は毎回変わるので作り直す(前の形は必ず捨てる)
      previewMesh.geometry.dispose();
      const geometry = new THREE.BufferGeometry();
      geometry.setAttribute(
        "position",
        new THREE.BufferAttribute(new Float32Array(points), 3),
      );
      geometry.setIndex(indices);
      previewMesh.geometry = geometry;
      previewMesh.visible = indices.length > 0;
      render();
    },
    setDrawMode(enabled, rotateWithRight = false) {
      controls.mouseButtons.LEFT = enabled ? null : THREE.MOUSE.ROTATE;
      const swap = enabled && rotateWithRight;
      controls.mouseButtons.RIGHT = swap ? THREE.MOUSE.ROTATE : THREE.MOUSE.PAN;
      controls.mouseButtons.MIDDLE = swap ? THREE.MOUSE.PAN : THREE.MOUSE.DOLLY;
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      unregisterReadback();
      previewMesh.geometry.dispose();
      previewMaterial.dispose();
      if (frameHandle !== null) {
        cancelAnimationFrame(frameHandle);
        frameHandle = null;
      }
      canvas.removeEventListener("webglcontextrestored", onContextRestored);
      canvas.removeEventListener("pointerdown", onCanvasPointerDown);
      canvas.removeEventListener("wheel", syncCameraUp, { capture: true });
      documentOf.removeEventListener("pointermove", onDocumentPointerMove);
      documentOf.removeEventListener("pointerup", onDocumentPointerUp);
      documentOf.removeEventListener("pointercancel", onDocumentPointerUp);
      rotateDrag = null;
      controls.removeEventListener("change", render);
      controls.dispose();
      api.setSoft(null); // たわみの表示物も片付ける(外していた面・線が入れ物へ戻る)
      if (api.content !== null) disposeSurfaceOwnerSurface(api.content.owner);
      clearGroup(contentGroup);
      api.content = null;
      api.pickSurface = null;
      showOwner(null);
      supplementalEdgeLayer.dispose();
      highlightLayer.dispose();
      emptyOwnerGeometry.dispose();
      if (depthReadback !== null) {
        disposePackedDepthReadbackResources(depthReadback);
        depthReadback = null;
      }
      disposeSurfaceOwnerPassResources(ownerPass);
      renderer.dispose();
    },
  };
  const unregisterReadback = registerViewer3DReadback(captureReadback);
  return api;
}
