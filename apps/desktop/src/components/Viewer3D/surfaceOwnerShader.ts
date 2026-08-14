// surface owner textureを既存の紙面・境界線・画面幅つき強調線へ接続する。
// 色・照明・線幅は既存materialのままにし、fragmentを捨てる条件だけを足す。

import * as THREE from "three";
import type { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import type { SurfaceOwnerBinding } from "./surfaceOwner";

export const SURFACE_OWNER_ATTRIBUTE = "surfaceOwnerToken";
export const SURFACE_OWNER_PROBE_ATTRIBUTE = "surfaceOwnerProbe";
export const SURFACE_OWNER_OTHER_ATTRIBUTE = "surfaceOwnerOther";
/** 6pxのfocus線を高DPIでも紙の外周まで残せる、近傍探索の固定上限。 */
export const SURFACE_OWNER_MAX_RADIUS_PX = 12;
/** owner最前深度targetの固定精度。UnsignedIntType + DepthFormatはDEPTH_COMPONENT24になる。 */
export const SURFACE_OWNER_DEPTH_BITS = 24;
/** 別三角形分割の共平面で生じる補間丸めだけをtieとして扱う深度code数。 */
export const SURFACE_OWNER_DEPTH_TOLERANCE_CODES = 2;
export const SURFACE_OWNER_DEPTH_TOLERANCE =
  SURFACE_OWNER_DEPTH_TOLERANCE_CODES / (2 ** SURFACE_OWNER_DEPTH_BITS - 1);

const OWNER_FRAGMENT_SUPPORT = /* glsl */ `
uniform float surfaceOwnerEnabled;
uniform sampler2D surfaceOwnerMap;
uniform vec2 surfaceOwnerResolution;
uniform float surfaceOwnerMode;
uniform vec4 surfaceOwnerExpected;
uniform float surfaceOwnerRadius;

bool surfaceOwnerIsBackground( const in vec4 token ) {
  return dot( abs( token ), vec4( 1.0 ) ) < ( 0.5 / 255.0 );
}

bool surfaceOwnerTokenMatches( const in vec4 actual, const in vec4 expected ) {
  if ( surfaceOwnerMode > 1.5 ) return ! surfaceOwnerIsBackground( actual );
  return all( lessThan( abs( actual - expected ), vec4( 0.5 / 255.0 ) ) );
}

vec4 surfaceOwnerSample( const in vec2 pixel ) {
  vec2 size = max( surfaceOwnerResolution, vec2( 1.0 ) );
  vec2 uv = clamp( pixel / size, vec2( 0.0 ), vec2( 1.0 ) );
  return texture2D( surfaceOwnerMap, uv );
}

bool surfaceOwnerSampleMatches( const in vec2 pixel, const in vec4 expected ) {
  return surfaceOwnerTokenMatches( surfaceOwnerSample( pixel ), expected );
}

bool surfaceOwnerVisible( const in vec4 expected ) {
  if ( surfaceOwnerEnabled < 0.5 || surfaceOwnerMode < 0.5 ) return true;
  vec4 actual = surfaceOwnerSample( gl_FragCoord.xy );
  if ( surfaceOwnerTokenMatches( actual, expected ) ) return true;

  // 別の紙が所有する画素では近傍を探さない。ここで探すと、下の面の線が重なりの
  // 境界から上の紙へradius分にじみ、surface-onlyの保証を破ってしまう。
  if ( ! surfaceOwnerIsBackground( actual ) ) return false;
  if ( surfaceOwnerRadius < 0.5 ) return false;

  // 線は中心が紙の輪郭上にあり、幅の半分が紙の外へ出る。中心画素だけを見ると
  // 正しい外周まで欠けるため、背景画素に限り半径内を8方向へ調べる。紙面にも
  // 真横のMSAA coverageをsingle-sample ownerへ合わせる実測済み半径を渡す。
  for ( int step = 1; step <= ${SURFACE_OWNER_MAX_RADIUS_PX}; step ++ ) {
    if ( float( step ) > surfaceOwnerRadius ) continue;
    float d = float( step );
    if ( surfaceOwnerSampleMatches( gl_FragCoord.xy + vec2( d, 0.0 ), expected ) ) return true;
    if ( surfaceOwnerSampleMatches( gl_FragCoord.xy + vec2( - d, 0.0 ), expected ) ) return true;
    if ( surfaceOwnerSampleMatches( gl_FragCoord.xy + vec2( 0.0, d ), expected ) ) return true;
    if ( surfaceOwnerSampleMatches( gl_FragCoord.xy + vec2( 0.0, - d ), expected ) ) return true;
    if ( surfaceOwnerSampleMatches( gl_FragCoord.xy + vec2( d, d ), expected ) ) return true;
    if ( surfaceOwnerSampleMatches( gl_FragCoord.xy + vec2( d, - d ), expected ) ) return true;
    if ( surfaceOwnerSampleMatches( gl_FragCoord.xy + vec2( - d, d ), expected ) ) return true;
    if ( surfaceOwnerSampleMatches( gl_FragCoord.xy + vec2( - d, - d ), expected ) ) return true;
  }
  return false;
}
`;

/**
 * 画面幅を持つLine2は、fragment中心ではなく中心線が属する面で可視性を決める。
 * これにより上面の外周は全幅を保ち、下面の線は外周近傍へにじまない。
 */
const OWNER_LINE_FRAGMENT_SUPPORT = /* glsl */ `
uniform vec2 surfaceOwnerLineStart;
uniform vec2 surfaceOwnerLineEnd;
uniform vec2 surfaceOwnerLineInward;
uniform float surfaceOwnerLineProbeSupplied;
uniform float surfaceOwnerLineCenterValid;
uniform float surfaceOwnerLineInwardValid;

bool surfaceOwnerLineVisible( const in vec4 expected ) {
  if ( surfaceOwnerEnabled < 0.5 || surfaceOwnerMode < 0.5 ) return true;
  if ( surfaceOwnerMode < 1.5 && surfaceOwnerLineProbeSupplied > 0.5 ) {
    // A caller-provided physical-edge probe is an exact contract.  If its
    // projected center line degenerates, fail closed instead of falling back
    // to the general background-neighbour search, which could reveal a lower
    // face.  Edge-on probes still test the center line, but have no inward
    // sample unless its direction can be established unambiguously.
    if ( surfaceOwnerLineCenterValid < 0.5 ) return false;
    vec2 edge = surfaceOwnerLineEnd - surfaceOwnerLineStart;
    float edgeLength2 = dot( edge, edge );
    if ( edgeLength2 <= 1e-8 ) return false;
    float along = clamp(
      dot( gl_FragCoord.xy - surfaceOwnerLineStart, edge ) / edgeLength2,
      0.0,
      1.0
    );
    vec2 center = surfaceOwnerLineStart + edge * along;
    // 中心線から当該faceの内側だけを見る。別ownerなら直後の境界coverage用1pxだけ、
    // 背景なら同じ方向のholeだけを調べ、覆われた下面線が重なり境界へにじまない。
    vec4 centerOwner = surfaceOwnerSample( center );
    if ( surfaceOwnerTokenMatches( centerOwner, expected ) ) return true;
    if ( surfaceOwnerLineInwardValid < 0.5 ) return false;
    if ( ! surfaceOwnerIsBackground( centerOwner ) ) {
      // 三角形とLine2の境界coverageが反対側のownerを選ぶ場合は、面内1pxだけを
      // 確認する。それより先へ進まないので、下面線が近くの露出部を拾わない。
      return surfaceOwnerSampleMatches( center + surfaceOwnerLineInward, expected );
    }
    for ( int step = 1; step <= ${SURFACE_OWNER_MAX_RADIUS_PX}; step ++ ) {
      if ( float( step ) > surfaceOwnerRadius ) continue;
      vec4 inwardOwner = surfaceOwnerSample(
        center + surfaceOwnerLineInward * float( step )
      );
      if ( surfaceOwnerTokenMatches( inwardOwner, expected ) ) return true;
      if ( ! surfaceOwnerIsBackground( inwardOwner ) ) return false;
    }
    return false;
  }
  return surfaceOwnerVisible( expected );
}
`;

function ownerUniforms(binding: SurfaceOwnerBinding) {
  return {
    surfaceOwnerEnabled: binding.enabled,
    surfaceOwnerMap: binding.map,
    surfaceOwnerResolution: binding.resolution,
  };
}

export interface SurfaceOwnerPassResources {
  /** 最前深度だけを作るtarget。color textureは第2passから読まない。 */
  readonly depthTarget: THREE.WebGLRenderTarget;
  /** 最終的なowner codeを書き、通常材質が読むtarget。depthは持たない。 */
  readonly colorTarget: THREE.WebGLRenderTarget;
  readonly depthMaterial: THREE.MeshDepthMaterial;
  readonly colorMaterial: THREE.ShaderMaterial;
}

export interface SurfaceOwnerOutlineGeometry {
  readonly geometry: THREE.BufferGeometry;
  readonly sourcePosition: THREE.BufferAttribute;
  readonly endpointSources: Int32Array;
  readonly otherSources: Int32Array;
  readonly probeSources: Int32Array;
}

/**
 * native LineBasicのcoverageは面三角形と一致しないため、外周線をedgeごとの2頂点へ
 * 複製する。各頂点に「反対端」と「隣接三角形の第3頂点」を持たせ、shaderが視点ごとの
 * 内向きを決められるようにする。面の重心では凹面の外へ出るので使用しない。
 */
export function createSurfaceOwnerOutlineGeometry(input: {
  sourcePosition: THREE.BufferAttribute;
  sourceToken: THREE.BufferAttribute;
  lineIndices: ArrayLike<number>;
  lineProbeIndices: ArrayLike<number>;
}): SurfaceOwnerOutlineGeometry {
  if (input.sourcePosition.itemSize !== 3 || input.sourcePosition.normalized) {
    throw new RangeError("surface owner outline positions must be unnormalized xyz");
  }
  if (input.lineIndices.length % 2 !== 0) {
    throw new RangeError("surface owner outline indices must contain complete edges");
  }
  if (input.lineProbeIndices.length * 2 !== input.lineIndices.length) {
    throw new RangeError("surface owner outline probes must match edge count");
  }
  const count = input.lineIndices.length;
  const endpointSources = new Int32Array(count);
  const otherSources = new Int32Array(count);
  const probeSources = new Int32Array(count);
  const position = new THREE.BufferAttribute(new Float32Array(count * 3), 3);
  const other = new THREE.BufferAttribute(new Float32Array(count * 3), 3);
  const probe = new THREE.BufferAttribute(new Float32Array(count * 3), 3);
  const token = new THREE.BufferAttribute(new Uint8Array(count * 4), 4, true);
  position.setUsage(THREE.DynamicDrawUsage);
  other.setUsage(THREE.DynamicDrawUsage);
  probe.setUsage(THREE.DynamicDrawUsage);
  for (let edge = 0; edge < count / 2; edge++) {
    const a = input.lineIndices[edge * 2];
    const b = input.lineIndices[edge * 2 + 1];
    const p = input.lineProbeIndices[edge];
    for (const source of [a, b, p]) {
      if (!Number.isSafeInteger(source) || source < 0 || source >= input.sourcePosition.count) {
        throw new RangeError(`invalid surface owner outline vertex: ${source}`);
      }
    }
    endpointSources[edge * 2] = a;
    endpointSources[edge * 2 + 1] = b;
    otherSources[edge * 2] = b;
    otherSources[edge * 2 + 1] = a;
    probeSources[edge * 2] = p;
    probeSources[edge * 2 + 1] = p;
    for (let endpoint = 0; endpoint < 2; endpoint++) {
      const at = edge * 2 + endpoint;
      const source = endpointSources[at];
      token.setXYZW(
        at,
        input.sourceToken.getX(source),
        input.sourceToken.getY(source),
        input.sourceToken.getZ(source),
        input.sourceToken.getW(source),
      );
    }
  }
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", position);
  geometry.setAttribute(SURFACE_OWNER_OTHER_ATTRIBUTE, other);
  geometry.setAttribute(SURFACE_OWNER_PROBE_ATTRIBUTE, probe);
  geometry.setAttribute(SURFACE_OWNER_ATTRIBUTE, token);
  const outline = {
    geometry,
    sourcePosition: input.sourcePosition,
    endpointSources,
    otherSources,
    probeSources,
  };
  updateSurfaceOwnerOutlineGeometry(outline);
  return outline;
}

/** shared face/soft positionの現在値を、edge固有の非indexed outlineへ同期する。 */
export function updateSurfaceOwnerOutlineGeometry(
  outline: SurfaceOwnerOutlineGeometry,
): void {
  const position = outline.geometry.getAttribute("position") as THREE.BufferAttribute;
  const other = outline.geometry.getAttribute(
    SURFACE_OWNER_OTHER_ATTRIBUTE,
  ) as THREE.BufferAttribute;
  const probe = outline.geometry.getAttribute(
    SURFACE_OWNER_PROBE_ATTRIBUTE,
  ) as THREE.BufferAttribute;
  const sourceArray = outline.sourcePosition.array;
  const positionArray = position.array;
  const otherArray = other.array;
  const probeArray = probe.array;
  for (let at = 0; at < outline.endpointSources.length; at++) {
    const targetAt = at * 3;
    const endpointAt = outline.endpointSources[at] * 3;
    const otherAt = outline.otherSources[at] * 3;
    const probeAt = outline.probeSources[at] * 3;
    positionArray[targetAt] = sourceArray[endpointAt];
    positionArray[targetAt + 1] = sourceArray[endpointAt + 1];
    positionArray[targetAt + 2] = sourceArray[endpointAt + 2];
    otherArray[targetAt] = sourceArray[otherAt];
    otherArray[targetAt + 1] = sourceArray[otherAt + 1];
    otherArray[targetAt + 2] = sourceArray[otherAt + 2];
    probeArray[targetAt] = sourceArray[probeAt];
    probeArray[targetAt + 1] = sourceArray[probeAt + 1];
    probeArray[targetAt + 2] = sourceArray[probeAt + 2];
  }
  position.needsUpdate = true;
  other.needsUpdate = true;
  probe.needsUpdate = true;
  // 黒線は形が毎frame変わりfrustumCulled=falseなので、使われない境界球を
  // 数千頂点から毎回計算しない。座標・probeのGPU更新だけで十分。
}

/** owner用RGBA8 targetを作る。background code 0を厳密に保つため補間・mipmap・色変換を切る。 */
function createOwnerColorTarget(depthBuffer: boolean): THREE.WebGLRenderTarget {
  const target = new THREE.WebGLRenderTarget(1, 1, {
    format: THREE.RGBAFormat,
    type: THREE.UnsignedByteType,
    minFilter: THREE.NearestFilter,
    magFilter: THREE.NearestFilter,
    depthBuffer,
    stencilBuffer: false,
  });
  target.texture.generateMipmaps = false;
  target.texture.colorSpace = THREE.NoColorSpace;
  return target;
}

/**
 * owner passのGPU資源を作る。
 *
 * 共平面でも三角形分割が異なると、補間丸めにより同じ平面のdepthが1〜2 codeずれ、
 * LEQUALの後描きだけでは下面ownerが周期的に残る。そこで第1passは実際の最前深度、
 * 第2passはその値から2 code以内だけを通し、layer/面の鏡映偶奇/決定的fallback順で
 * owner色を後勝ちさせる。
 * 既定中心の層間隔0.0002は12.64 code、視線方向成分でも約9.08 codeあるため、2 codeは
 * 実在する層差をtieへ丸めない。depthTargetを読みながら別colorTargetへ書くので、同じ
 * textureをsampling attachmentにもするWebGLのfeedback loopも作らない。
 */
export function createSurfaceOwnerPassResources(
  binding: SurfaceOwnerBinding,
): SurfaceOwnerPassResources {
  const depthTexture = new THREE.DepthTexture(1, 1, THREE.UnsignedIntType);
  depthTexture.format = THREE.DepthFormat;
  depthTexture.minFilter = THREE.NearestFilter;
  depthTexture.magFilter = THREE.NearestFilter;
  depthTexture.generateMipmaps = false;

  const depthTarget = createOwnerColorTarget(true);
  depthTarget.depthTexture = depthTexture;
  const colorTarget = createOwnerColorTarget(false);
  const depthMaterial = new THREE.MeshDepthMaterial({
    side: THREE.DoubleSide,
    depthPacking: THREE.BasicDepthPacking,
  });
  depthMaterial.depthTest = true;
  depthMaterial.depthWrite = true;
  depthMaterial.depthFunc = THREE.LessEqualDepth;

  const colorMaterial = new THREE.ShaderMaterial({
    uniforms: {
      surfaceOwnerDepthMap: { value: depthTexture },
      surfaceOwnerResolution: binding.resolution,
      surfaceOwnerDepthTolerance: { value: SURFACE_OWNER_DEPTH_TOLERANCE },
    },
    vertexShader: /* glsl */ `
      attribute vec4 ${SURFACE_OWNER_ATTRIBUTE};
      varying vec4 vSurfaceOwnerToken;
      void main() {
        vSurfaceOwnerToken = ${SURFACE_OWNER_ATTRIBUTE};
        gl_Position = projectionMatrix * modelViewMatrix * vec4( position, 1.0 );
      }
    `,
    fragmentShader: /* glsl */ `
      uniform sampler2D surfaceOwnerDepthMap;
      uniform vec2 surfaceOwnerResolution;
      uniform float surfaceOwnerDepthTolerance;
      varying vec4 vSurfaceOwnerToken;
      void main() {
        vec2 size = max( surfaceOwnerResolution, vec2( 1.0 ) );
        vec2 uv = clamp( gl_FragCoord.xy / size, vec2( 0.0 ), vec2( 1.0 ) );
        float nearestDepth = texture2D( surfaceOwnerDepthMap, uv ).x;
        if ( gl_FragCoord.z - nearestDepth > surfaceOwnerDepthTolerance ) discard;
        gl_FragColor = vSurfaceOwnerToken;
      }
    `,
    side: THREE.DoubleSide,
    depthTest: false,
    depthWrite: false,
    blending: THREE.NoBlending,
    toneMapped: false,
  });

  bindSurfaceOwner(binding, colorTarget.texture, 1, 1);
  return { depthTarget, colorTarget, depthMaterial, colorMaterial };
}

/** depth/color両targetと共有uniformを同じdrawing-buffer物理画素数へそろえる。 */
export function resizeSurfaceOwnerPassResources(
  resources: SurfaceOwnerPassResources,
  binding: SurfaceOwnerBinding,
  width: number,
  height: number,
): void {
  const safeWidth = Math.max(1, Math.floor(width));
  const safeHeight = Math.max(1, Math.floor(height));
  resources.depthTarget.setSize(safeWidth, safeHeight);
  resources.colorTarget.setSize(safeWidth, safeHeight);
  bindSurfaceOwner(binding, resources.colorTarget.texture, safeWidth, safeHeight);
}

export function disposeSurfaceOwnerPassResources(
  resources: SurfaceOwnerPassResources,
): void {
  resources.depthMaterial.dispose();
  resources.colorMaterial.dispose();
  resources.depthTarget.dispose();
  resources.colorTarget.dispose();
}

/** MeshLambertMaterial / LineBasicMaterialへ頂点ごとの所有面照合を加える。 */
export function filterMaterialBySurfaceOwnerAttribute(
  material: THREE.MeshLambertMaterial | THREE.LineBasicMaterial,
  binding: SurfaceOwnerBinding,
  radiusPx: number,
): void {
  material.onBeforeCompile = (shader) => {
    Object.assign(shader.uniforms, ownerUniforms(binding), {
      surfaceOwnerMode: { value: 1 },
      surfaceOwnerExpected: { value: new THREE.Vector4() },
      surfaceOwnerRadius: { value: radiusPx },
    });
    shader.vertexShader = shader.vertexShader
      .replace(
        "#include <common>",
        `#include <common>\nattribute vec4 ${SURFACE_OWNER_ATTRIBUTE};\nvarying vec4 vSurfaceOwnerToken;`,
      )
      .replace(
        "void main() {",
        `void main() {\n  vSurfaceOwnerToken = ${SURFACE_OWNER_ATTRIBUTE};`,
      );
    shader.fragmentShader = shader.fragmentShader
      .replace(
        "#include <common>",
        `#include <common>\nvarying vec4 vSurfaceOwnerToken;\n${OWNER_FRAGMENT_SUPPORT}`,
      )
      .replace(
        "void main() {",
        "void main() {\n  if ( ! surfaceOwnerVisible( vSurfaceOwnerToken ) ) discard;",
      );
  };
  material.customProgramCacheKey = () => `surface-owner-attribute-${radiusPx}`;
  material.userData.surfaceOwnerFilter = "attribute";
  material.userData.surfaceOwnerRadiusPx = radiusPx;
}

/**
 * 黒い1px外周線専用のowner判定。
 *
 * LineBasicと三角形では境界画素のcoverageが異なるため、中心だけの照合では正しい
 * 斜め外周が点線になる。一方、foreign ownerの周囲を無方向に探すと、紙の下の線が
 * 1px漏れる。そこでedgeに接する三角形の第3頂点から内向きを決め、中心または内側
 * ちょうど1 device pixelだけをexact照合する。投影面積が潰れたedge-onでは中心だけに
 * fail closedし、任意方向の近傍探索へは切り替えない。
 */
export function filterOutlineMaterialBySurfaceOwner(
  material: THREE.LineBasicMaterial,
  binding: SurfaceOwnerBinding,
): void {
  material.onBeforeCompile = (shader) => {
    Object.assign(shader.uniforms, ownerUniforms(binding));
    shader.vertexShader = shader.vertexShader
      .replace(
        "#include <common>",
        `#include <common>
attribute vec4 ${SURFACE_OWNER_ATTRIBUTE};
attribute vec3 ${SURFACE_OWNER_OTHER_ATTRIBUTE};
attribute vec3 ${SURFACE_OWNER_PROBE_ATTRIBUTE};
uniform vec2 surfaceOwnerResolution;
varying vec4 vSurfaceOwnerToken;
varying vec2 vSurfaceOwnerInward;
varying float vSurfaceOwnerInwardValid;`,
      )
      .replace(
        "void main() {",
        `void main() {
  vSurfaceOwnerToken = ${SURFACE_OWNER_ATTRIBUTE};
  vSurfaceOwnerInward = vec2( 0.0 );
  vSurfaceOwnerInwardValid = 0.0;
  vec4 ownerPointClip = projectionMatrix * modelViewMatrix * vec4( position, 1.0 );
  vec4 ownerOtherClip = projectionMatrix * modelViewMatrix * vec4( ${SURFACE_OWNER_OTHER_ATTRIBUTE}, 1.0 );
  vec4 ownerProbeClip = projectionMatrix * modelViewMatrix * vec4( ${SURFACE_OWNER_PROBE_ATTRIBUTE}, 1.0 );
  if ( abs( ownerPointClip.w ) > 1e-6 && abs( ownerOtherClip.w ) > 1e-6 && abs( ownerProbeClip.w ) > 1e-6 ) {
    vec2 ownerSize = max( surfaceOwnerResolution, vec2( 1.0 ) );
    vec2 ownerPointPx = ( ownerPointClip.xy / ownerPointClip.w * 0.5 + 0.5 ) * ownerSize;
    vec2 ownerOtherPx = ( ownerOtherClip.xy / ownerOtherClip.w * 0.5 + 0.5 ) * ownerSize;
    vec2 ownerProbePx = ( ownerProbeClip.xy / ownerProbeClip.w * 0.5 + 0.5 ) * ownerSize;
    vec2 ownerEdge = ownerOtherPx - ownerPointPx;
    float ownerEdgeLength = length( ownerEdge );
    if ( ownerEdgeLength > 1e-4 ) {
      vec2 ownerNormal = vec2( - ownerEdge.y, ownerEdge.x ) / ownerEdgeLength;
      float ownerProbeSide = dot( ownerProbePx - ( ownerPointPx + ownerOtherPx ) * 0.5, ownerNormal );
      if ( ownerProbeSide < 0.0 ) ownerNormal = - ownerNormal;
      if ( abs( ownerProbeSide ) > 1e-4 ) {
        vSurfaceOwnerInward = ownerNormal;
        vSurfaceOwnerInwardValid = 1.0;
      }
    }
  }`,
      );
    shader.fragmentShader = shader.fragmentShader
      .replace(
        "#include <common>",
        `#include <common>
uniform float surfaceOwnerEnabled;
uniform sampler2D surfaceOwnerMap;
uniform vec2 surfaceOwnerResolution;
varying vec4 vSurfaceOwnerToken;
varying vec2 vSurfaceOwnerInward;
varying float vSurfaceOwnerInwardValid;

vec4 surfaceOwnerOutlineSample( const in vec2 pixel ) {
  vec2 size = max( surfaceOwnerResolution, vec2( 1.0 ) );
  return texture2D( surfaceOwnerMap, clamp( pixel / size, vec2( 0.0 ), vec2( 1.0 ) ) );
}

bool surfaceOwnerOutlineMatches( const in vec2 pixel ) {
  vec4 actual = surfaceOwnerOutlineSample( pixel );
  return all( lessThan( abs( actual - vSurfaceOwnerToken ), vec4( 0.5 / 255.0 ) ) );
}

bool surfaceOwnerOutlineVisible() {
  if ( surfaceOwnerEnabled < 0.5 ) return true;
  if ( surfaceOwnerOutlineMatches( gl_FragCoord.xy ) ) return true;
  if ( vSurfaceOwnerInwardValid < 0.5 ) return false;
  return surfaceOwnerOutlineMatches( gl_FragCoord.xy + normalize( vSurfaceOwnerInward ) );
}`,
      )
      .replace(
        "void main() {",
        "void main() {\n  if ( ! surfaceOwnerOutlineVisible() ) discard;",
      );
  };
  material.customProgramCacheKey = () => "surface-owner-outline-inward-v1";
  material.userData.surfaceOwnerFilter = "outline-inward";
  material.userData.surfaceOwnerRadiusPx = 1;
}

export type FilteredLineMaterial = LineMaterial & {
  uniformsNeedUpdate: boolean;
};

const lineStartNdc = new THREE.Vector3();
const lineEndNdc = new THREE.Vector3();
const lineProbeNdc = new THREE.Vector3();
const lineStartPx = new THREE.Vector2();
const lineEndPx = new THREE.Vector2();
const lineProbePx = new THREE.Vector2();
const lineEdgePx = new THREE.Vector2();
const lineMidPx = new THREE.Vector2();
const lineInwardPx = new THREE.Vector2();

function setLineProbeUniforms(
  material: FilteredLineMaterial,
  probe:
    | {
        camera: THREE.Camera;
        start: THREE.Vector3;
        end: THREE.Vector3;
        inside: THREE.Vector3;
      }
    | undefined,
): void {
  material.uniforms.surfaceOwnerLineProbeSupplied.value = probe ? 1 : 0;
  material.uniforms.surfaceOwnerLineCenterValid.value = 0;
  material.uniforms.surfaceOwnerLineInwardValid.value = 0;
  if (!probe) return;
  const resolution = material.uniforms.surfaceOwnerResolution.value as THREE.Vector2;
  if (!(resolution.x > 0 && resolution.y > 0)) return;
  lineStartNdc.copy(probe.start).project(probe.camera);
  lineEndNdc.copy(probe.end).project(probe.camera);
  if (
    ![lineStartNdc.x, lineStartNdc.y, lineEndNdc.x, lineEndNdc.y].every(
      Number.isFinite,
    )
  ) {
    return;
  }
  const toPixel = (out: THREE.Vector2, point: THREE.Vector3) =>
    out.set(
      (point.x * 0.5 + 0.5) * resolution.x,
      (point.y * 0.5 + 0.5) * resolution.y,
    );
  toPixel(lineStartPx, lineStartNdc);
  toPixel(lineEndPx, lineEndNdc);
  lineEdgePx.subVectors(lineEndPx, lineStartPx);
  const length = lineEdgePx.length();
  if (!(length > 1e-4)) return;
  (material.uniforms.surfaceOwnerLineStart.value as THREE.Vector2).copy(lineStartPx);
  (material.uniforms.surfaceOwnerLineEnd.value as THREE.Vector2).copy(lineEndPx);
  material.uniforms.surfaceOwnerLineCenterValid.value = 1;

  lineProbeNdc.copy(probe.inside).project(probe.camera);
  if (![lineProbeNdc.x, lineProbeNdc.y].every(Number.isFinite)) return;
  toPixel(lineProbePx, lineProbeNdc);
  lineInwardPx.set(-lineEdgePx.y / length, lineEdgePx.x / length);
  lineMidPx.addVectors(lineStartPx, lineEndPx).multiplyScalar(0.5);
  const side = lineInwardPx.dot(lineProbePx.sub(lineMidPx));
  if (!Number.isFinite(side) || Math.abs(side) <= 1e-4) return;
  if (side < 0) lineInwardPx.multiplyScalar(-1);
  (material.uniforms.surfaceOwnerLineInward.value as THREE.Vector2).copy(lineInwardPx);
  material.uniforms.surfaceOwnerLineInwardValid.value = 1;
}

/** LineMaterialへ、各Line2が描画直前に与える所有面uniformの照合を加える。 */
export function filterLineMaterialBySurfaceOwner(
  material: FilteredLineMaterial,
  binding: SurfaceOwnerBinding,
): void {
  Object.assign(material.uniforms, ownerUniforms(binding), {
    surfaceOwnerMode: { value: 2 },
    surfaceOwnerExpected: { value: new THREE.Vector4() },
    surfaceOwnerRadius: { value: 0 },
    surfaceOwnerLineStart: { value: new THREE.Vector2() },
    surfaceOwnerLineEnd: { value: new THREE.Vector2() },
    surfaceOwnerLineInward: { value: new THREE.Vector2() },
    surfaceOwnerLineProbeSupplied: { value: 0 },
    surfaceOwnerLineCenterValid: { value: 0 },
    surfaceOwnerLineInwardValid: { value: 0 },
  });
  material.fragmentShader = material.fragmentShader.replace(
    "void main() {",
    `${OWNER_FRAGMENT_SUPPORT}\n${OWNER_LINE_FRAGMENT_SUPPORT}\nvoid main() {\n  if ( ! surfaceOwnerLineVisible( surfaceOwnerExpected ) ) discard;`,
  );
  material.userData.surfaceOwnerFilter = "uniform";
  material.needsUpdate = true;
}

/** 共有LineMaterialの値を、今から描く1本の所有面へ切り替える。 */
export function setLineSurfaceOwner(
  material: FilteredLineMaterial,
  mode: "bypass" | "exact" | "any",
  expected: THREE.Vector4,
  radiusPx: number,
  probe?: {
    camera: THREE.Camera;
    start: THREE.Vector3;
    end: THREE.Vector3;
    inside: THREE.Vector3;
  },
): void {
  material.uniforms.surfaceOwnerMode.value = mode === "bypass" ? 0 : mode === "exact" ? 1 : 2;
  (material.uniforms.surfaceOwnerExpected.value as THREE.Vector4).copy(expected);
  const ownerResolution = material.uniforms.surfaceOwnerResolution.value as THREE.Vector2;
  const lineResolution = material.uniforms.resolution.value as THREE.Vector2;
  const scale =
    lineResolution.x > 0 && lineResolution.y > 0
      ? Math.max(
          ownerResolution.x / lineResolution.x,
          ownerResolution.y / lineResolution.y,
        )
      : 1;
  // LineMaterialのlinewidth/radiusはCSS px、owner targetはdrawing-buffer pxなので、
  // DPRを両解像度の比から求めて物理画素へ換算する。
  material.uniforms.surfaceOwnerRadius.value = Math.min(
    SURFACE_OWNER_MAX_RADIUS_PX,
    Math.max(0, radiusPx * (Number.isFinite(scale) ? scale : 1)),
  );
  setLineProbeUniforms(material, probe);
  material.uniformsNeedUpdate = true;
}

/** render targetとdrawing bufferの物理画素数を全materialへ共有する。 */
export function bindSurfaceOwner(
  binding: SurfaceOwnerBinding,
  texture: THREE.Texture,
  width: number,
  height: number,
): void {
  binding.map.value = texture;
  binding.resolution.value.set(Math.max(1, width), Math.max(1, height));
  binding.enabled.value = 1;
}
