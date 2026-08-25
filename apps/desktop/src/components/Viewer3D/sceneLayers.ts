// 3Dビューのhighlightと補助描画layer。各layerが自分のGPU資源を破棄する。
import * as THREE from "three";
import { Line2 } from "three/examples/jsm/lines/Line2.js";
import { LineGeometry } from "three/examples/jsm/lines/LineGeometry.js";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import type { HingeSegment } from "./hingePicker";
import {
  createSurfaceOwnerBinding,
  ownerCodeBytes,
  ownerCodeVector,
  type SurfaceOwnerBinding,
} from "./surfaceOwner";
import {
  createSurfaceOwnerOutlineGeometry,
  filterLineMaterialBySurfaceOwner,
  setLineSurfaceOwner,
  type FilteredLineMaterial,
  type SurfaceOwnerOutlineGeometry,
} from "./surfaceOwnerShader";

/** 選択中ヒンジの強調色(黄色) */
const HIGHLIGHT_COLOR = 0xffd400;
/** 選択中だが折る操作の対象ではない縁・補助線などの強調色(水色) */
const REFERENCE_HIGHLIGHT_COLOR = 0x40cfff;
/** 複数スライダーのうち指している1本の強調色(POPのコーラル) */
const FOCUS_HIGHLIGHT_COLOR = 0xed5c70;
/** 補正後にも残る食い込みの原因候補。選択の黄色・水色と明確に分ける。 */
const SUSPECT_HIGHLIGHT_COLOR = 0xff2038;
/** いま利用者が角度を固定して動かしている折り目(水色)。 */
const ACTIVE_HIGHLIGHT_COLOR = 0x40cfff;
/**
 * 利用者が角度を固定した折り目(墨色)。
 *
 * 2D展開図の印(`CpEditor/renderer.ts` の `COLORS.pinned` = #1b2430)と同じ色にし、
 * 2Dと3Dで同じものだと分かるようにする。黄・水色・コーラル・赤のどれとも
 * 明度で見分けられるよう、色相を足さず濃い墨色にする。
 */
const PINNED_HIGHLIGHT_COLOR = 0x1b2430;
/** 固定の印(丸)の色。線と同じ墨色にして、色は増やさない。 */
const PIN_MARK_COLOR = PINNED_HIGHLIGHT_COLOR;
/** 折った結果の下見(実行前プレビュー)の色。動く紙と分かるよう青系にする */
const PREVIEW_COLOR = 0x2f8fff;
/** 下見の透け具合(下の紙が見える程度) */
const PREVIEW_OPACITY = 0.45;
/**
 * 選択中ヒンジの画面上の太さ(CSS px)。
 *
 * 層間隔の1/4以下の円柱にすると半径は0.00005以下となり、400pxの既定表示では
 * 直径が約0.03pxで見えない。そこで3Dの半径を持たない画面上4 CSS pxの線へ替える。
 * ANGLE/D3D11の原寸オフスクリーン描画でも紙面上で連続して見えることを目視済み。
 * 紙の裏側は深度と画素単位のsurface owner判定で隠す。
 */
export const HIGHLIGHT_WIDTH_PX = 4;
/** 指している1本は通常線より見分けやすくする。 */
export const FOCUS_HIGHLIGHT_WIDTH_PX = 6;
/** 食い込み原因は警告として最も目立たせる。 */
export const SUSPECT_HIGHLIGHT_WIDTH_PX = 8;
/**
 * 固定した折り目の中点に打つ印の大きさ(CSS px)。
 *
 * 折り目の線(4px)や輪郭の線と見分けるには、線より明らかに太い丸が要る。
 * 実機で普通の大きさ(拡大なし)にして目で見て決めた: 濃い線だけでは輪郭の線と
 * 見分けにくかったため、2D展開図の印(直径12px)とそろえた16pxにする。
 */
export const PIN_MARK_WIDTH_PX = 16;
/**
 * 印にする「折り目の真ん中の太い部分」の長さ(折り目の長さに対する割合)。
 *
 * 印は**折り目そのものの一部を太く描いたもの**である。折り目に沿った向きにしか
 * 伸びないので、紙の面から外へは出ない(=紙の重なりを貫通できない)。
 *
 * | 量 | 値 | 向き |
 * |---|---:|---|
 * | 印の伸び(折り目に沿う) | 折り目の長さの **12%**(下限 1e-4・上限 0.02) | **紙の面の中**。面から外へ出ない |
 * | 印の太さ | **画面上で16px**(`PIN_MARK_WIDTH_PX`) | 世界座標の厚みを持たない |
 * | 紙の重なり全体の厚み `MAX_STACK_RATIO` | 0.001 | 紙の面に**垂直** |
 * | 層と層の間隔 `LAYER_STEP_RATIO` | 0.0002 | 紙の面に**垂直** |
 * | 事故を起こした円柱の強調線の半径(過去) | 0.006 | **全方向**(だから紙を貫通した) |
 *
 * 過去の事故(§10.7.8)は、強調線が**全方向に**半径0.006の円柱を持ち、
 * それが紙の重なり全体の厚み0.001の6倍あったために起きた。
 * この印は**面に垂直な厚みを1つも持たない**ので、同じことは起きない。
 * **紙の厚み(`LAYER_STEP_RATIO` / `MAX_STACK_RATIO`)は1つも変えていない。**
 */
export const PIN_MARK_RATIO = 0.12;
/**
 * 印の最小の長さ(世界座標)。
 *
 * 描画は32ビットの小数で行われる(相対精度 約1.2e-7)。長さが小さすぎると
 * 線の向きが数値の誤差に埋もれて印が消える。実機で 1e-6 にしたところ
 * **画面に何も出なかった**ため、桁を上げて 1e-4(誤差の約1000倍)にした。
 */
export const PIN_MARK_MIN_LENGTH = 1e-4;
/** 印の最大の長さ(世界座標)。長い折り目でも印が線全体を覆わないようにする。 */
export const PIN_MARK_MAX_LENGTH = 0.02;
/** 強調表示の伸ばす向き(回転計算に使う) */
const AXIS_Y = new THREE.Vector3(0, 1, 0);

/** 強調線分。role省略時は従来どおり操作対象の黄色で描く。 */
export interface HighlightSegment extends HingeSegment {
  role?: "hinge" | "reference" | "focus" | "suspect" | "active" | "pinned" | "pinMark";
}

/** @types/threeがまだ公開していないLineMaterialの実在するlinewidthアクセサ。 */
export type HighlightLineMaterial = FilteredLineMaterial & { linewidth: number };

/** 強調表示7種類の共有材質。 */
export interface HighlightMaterials {
  highlightMaterial: HighlightLineMaterial;
  referenceHighlightMaterial: HighlightLineMaterial;
  focusHighlightMaterial: HighlightLineMaterial;
  suspectHighlightMaterial: HighlightLineMaterial;
  activeHighlightMaterial: HighlightLineMaterial;
  pinnedHighlightMaterial: HighlightLineMaterial;
  /** 固定した折り目の中点に打つ丸。線と同じ材質の仕組みで、太さだけ変える。 */
  pinMarkMaterial: HighlightLineMaterial;
}

/** 世界単位の断面を作らず、画面上で一定の太さを保つ線材質を作る。 */
function createHighlightMaterial(
  color: number,
  linewidth: number,
  depthTest: boolean,
  ownerBinding: SurfaceOwnerBinding,
): HighlightLineMaterial {
  const material = new LineMaterial({
    color,
    worldUnits: false,
    alphaToCoverage: true,
    depthTest,
    depthFunc: THREE.LessEqualDepth,
    depthWrite: false,
  }) as HighlightLineMaterial;
  material.linewidth = linewidth;
  if (depthTest) filterLineMaterialBySurfaceOwner(material, ownerBinding);
  return material;
}

/** 強調表示7種類の材質を作る。太さ・深度・surface owner判定を検査できる形にまとめる。 */
export function createHighlightMaterials(
  ownerBinding: SurfaceOwnerBinding = createSurfaceOwnerBinding(),
): HighlightMaterials {
  const highlightMaterial = createHighlightMaterial(
    HIGHLIGHT_COLOR,
    HIGHLIGHT_WIDTH_PX,
    true,
    ownerBinding,
  );
  const referenceHighlightMaterial = createHighlightMaterial(
    REFERENCE_HIGHLIGHT_COLOR,
    HIGHLIGHT_WIDTH_PX,
    true,
    ownerBinding,
  );
  const focusHighlightMaterial = createHighlightMaterial(
    FOCUS_HIGHLIGHT_COLOR,
    FOCUS_HIGHLIGHT_WIDTH_PX,
    true,
    ownerBinding,
  );
  const suspectHighlightMaterial = createHighlightMaterial(
    SUSPECT_HIGHLIGHT_COLOR,
    SUSPECT_HIGHLIGHT_WIDTH_PX,
    // 食い込みは紙の内側で起きるため、隠れると原因の折り目を見つけられない。
    // これだけは意図的に手前へ描く。
    false,
    ownerBinding,
  );
  const activeHighlightMaterial = createHighlightMaterial(
    ACTIVE_HIGHLIGHT_COLOR,
    HIGHLIGHT_WIDTH_PX,
    true,
    ownerBinding,
  );
  // 固定の印は「いま操作している折り目」より控えめにし、紙に隠れる側も
  // 同じ規則(深度と紙面の持ち主で隠す)にする。手前へ無理に出さない。
  const pinnedHighlightMaterial = createHighlightMaterial(
    PINNED_HIGHLIGHT_COLOR,
    HIGHLIGHT_WIDTH_PX,
    true,
    ownerBinding,
  );
  const pinMarkMaterial = createHighlightMaterial(
    PIN_MARK_COLOR,
    PIN_MARK_WIDTH_PX,
    true,
    ownerBinding,
  );
  return {
    highlightMaterial,
    referenceHighlightMaterial,
    focusHighlightMaterial,
    suspectHighlightMaterial,
    activeHighlightMaterial,
    pinnedHighlightMaterial,
    pinMarkMaterial,
  };
}

/**
 * 強調表示の中心線。画面方向へだけ太くするLine2用なので、世界座標ではx/z方向の
 * 幅が0、すなわち紙の重なりを幾何的に貫通する半径を持たない。
 */
export function createHighlightGeometry(): LineGeometry {
  const geometry = new LineGeometry();
  geometry.setPositions([0, 0, 0, 0, 1, 0]);
  return geometry;
}

const HIGHLIGHT_RENDER_ORDER = 5;
const ACTIVE_HIGHLIGHT_RENDER_ORDER = 6;
const SUSPECT_HIGHLIGHT_RENDER_ORDER = 7;

/** 7役割を実際に使う材質・描画順へ対応付ける。role省略時はhingeと同じ。 */
export function highlightAppearance(
  materials: HighlightMaterials,
  role: HighlightSegment["role"],
): { material: HighlightLineMaterial; renderOrder: number } {
  switch (role) {
    case "suspect":
      return {
        material: materials.suspectHighlightMaterial,
        renderOrder: SUSPECT_HIGHLIGHT_RENDER_ORDER,
      };
    case "active":
      return {
        material: materials.activeHighlightMaterial,
        renderOrder: ACTIVE_HIGHLIGHT_RENDER_ORDER,
      };
    case "pinned":
      return {
        material: materials.pinnedHighlightMaterial,
        renderOrder: HIGHLIGHT_RENDER_ORDER,
      };
    case "pinMark":
      return {
        material: materials.pinMarkMaterial,
        renderOrder: HIGHLIGHT_RENDER_ORDER,
      };
    case "reference":
      return {
        material: materials.referenceHighlightMaterial,
        renderOrder: HIGHLIGHT_RENDER_ORDER,
      };
    case "focus":
      return {
        material: materials.focusHighlightMaterial,
        renderOrder: HIGHLIGHT_RENDER_ORDER,
      };
    case "hinge":
    case undefined:
      return {
        material: materials.highlightMaterial,
        renderOrder: HIGHLIGHT_RENDER_ORDER,
      };
  }
}

/** createSceneが実際に使う強調線の生成・プール更新・破棄をまとめた層。 */
export interface HighlightLayer {
  readonly group: THREE.Group;
  readonly geometry: LineGeometry;
  readonly materials: HighlightMaterials;
  setSegments(segments: HighlightSegment[]): void;
  setOwnerCodes(codes: ReadonlyMap<number, number>): void;
  dispose(): void;
}

/**
 * 固定した折り目に、真ん中の太い印を足した線分の並びを返す。
 *
 * 濃い線だけでは輪郭の線と見分けにくかった(実機で普通の大きさにして確認)ので、
 * 折り目1本につき印を1つ打つ。同じ折り目が複数の面に分かれているときは、
 * **いちばん長い線分の真ん中**に1つだけ打つ(面の数だけ印が並ばないように)。
 *
 * 印は折り目に沿った短い線分を画面上で太く描いたもので、
 * **紙の面から外へは出ない**(`PIN_MARK_RATIO` の表を参照)。
 */
export function withPinMarks(
  segments: readonly HighlightSegment[],
): HighlightSegment[] {
  const longest = new Map<number, { segment: HighlightSegment; length: number }>();
  for (const segment of segments) {
    if (segment.role !== "pinned") continue;
    const length = segment.a.distanceTo(segment.b);
    if (!Number.isFinite(length)) continue;
    const found = longest.get(segment.edgeId);
    if (found === undefined || length > found.length) {
      longest.set(segment.edgeId, { segment, length });
    }
  }
  if (longest.size === 0) return [...segments];
  const marks: HighlightSegment[] = [];
  for (const { segment, length } of longest.values()) {
    if (!(length > PIN_MARK_MIN_LENGTH)) continue; // 短すぎる折り目には打たない
    const markLength = Math.min(
      Math.max(length * PIN_MARK_RATIO, PIN_MARK_MIN_LENGTH),
      Math.min(PIN_MARK_MAX_LENGTH, length),
    );
    const middle = segment.a.clone().add(segment.b).multiplyScalar(0.5);
    const along = segment.b
      .clone()
      .sub(segment.a)
      .normalize()
      .multiplyScalar(markLength / 2);
    marks.push({
      ...segment,
      role: "pinMark",
      a: middle.clone().sub(along),
      b: middle.clone().add(along),
    });
  }
  return [...segments, ...marks];
}

/**
 * 強調線の実描画物を作る。補助関数だけを検査して本番が円柱へ戻る退行を防ぐため、
 * createSceneと単体検査はこの同じ経路を使う。
 */
export function createHighlightLayer(
  ownerBinding: SurfaceOwnerBinding = createSurfaceOwnerBinding(),
): HighlightLayer {
  const group = new THREE.Group();
  const geometry = createHighlightGeometry();
  const materials = createHighlightMaterials(ownerBinding);
  const dir = new THREE.Vector3();
  let ownerCodes: ReadonlyMap<number, number> = new Map();

  return {
    group,
    geometry,
    materials,
    setSegments(rawSegments) {
      // 固定した折り目には、線に加えて中点の丸い印も描く。
      const segments = withPinMarks(rawSegments);
      const pool = group.children;
      let used = 0;
      for (const seg of segments) {
        dir.subVectors(seg.b, seg.a);
        const length = dir.length();
        if (length < 1e-9) continue;
        let line = pool[used] as Line2 | undefined;
        if (!line) {
          line = new Line2(geometry, materials.highlightMaterial);
          line.frustumCulled = false;
          // Line2本来のcallbackは、worldUnits=falseの線幅計算に必要なviewport解像度を
          // materialへ入れる。owner uniformを足しても必ず先に呼び、上書きで失わない。
          const updateLineResolution = line.onBeforeRender.bind(line);
          line.onBeforeRender = (
            renderer: THREE.WebGLRenderer,
            _scene?: THREE.Scene,
            camera?: THREE.Camera,
          ) => {
            updateLineResolution(renderer);
            const mode = line!.userData.surfaceOwnerMode as
              | "bypass"
              | "exact"
              | "any";
            if (mode === "bypass") return;
            setLineSurfaceOwner(
              line!.material as HighlightLineMaterial,
              mode,
              line!.userData.surfaceOwnerExpected as THREE.Vector4,
              line!.userData.surfaceOwnerRadius as number,
              camera && line!.userData.surfaceOwnerProbe instanceof THREE.Vector3
                ? {
                    camera,
                    start: line!.userData.surfaceOwnerStart as THREE.Vector3,
                    end: line!.userData.surfaceOwnerEnd as THREE.Vector3,
                    inside: line!.userData.surfaceOwnerProbe as THREE.Vector3,
                  }
                : undefined,
            );
          };
          group.add(line);
        }
        const appearance = highlightAppearance(materials, seg.role);
        line.material = appearance.material;
        const code = seg.ownerFace === undefined ? undefined : ownerCodes.get(seg.ownerFace);
        line.userData.surfaceOwnerMode =
          seg.role === "suspect" ? "bypass" : code === undefined ? "any" : "exact";
        line.userData.surfaceOwnerExpected = ownerCodeVector(code ?? 0);
        line.userData.surfaceOwnerRadius = Math.ceil(appearance.material.linewidth / 2) + 1;
        const ownerStart =
          line.userData.surfaceOwnerStart instanceof THREE.Vector3
            ? (line.userData.surfaceOwnerStart as THREE.Vector3)
            : new THREE.Vector3();
        const ownerEnd =
          line.userData.surfaceOwnerEnd instanceof THREE.Vector3
            ? (line.userData.surfaceOwnerEnd as THREE.Vector3)
            : new THREE.Vector3();
        line.userData.surfaceOwnerStart = ownerStart.copy(seg.a);
        line.userData.surfaceOwnerEnd = ownerEnd.copy(seg.b);
        if (seg.surfaceProbe) {
          const ownerProbe =
            line.userData.surfaceOwnerProbe instanceof THREE.Vector3
              ? (line.userData.surfaceOwnerProbe as THREE.Vector3)
              : new THREE.Vector3();
          line.userData.surfaceOwnerProbe = ownerProbe.copy(seg.surfaceProbe);
        } else {
          line.userData.surfaceOwnerProbe = null;
        }
        // 紙と同じ深度の表面では強調線を見せるため、紙より後に描く。
        // 食い込み以外の6種類は深度とsurface ownerの両方で紙の裏側なら隠れ、
        // 食い込みの赤だけは両判定を通さず最後に描くため、内側でも原因を見つけられる。
        line.renderOrder = appearance.renderOrder;
        line.position.copy(seg.a);
        line.quaternion.setFromUnitVectors(AXIS_Y, dir.normalize());
        line.scale.set(1, length, 1);
        line.visible = true;
        used++;
      }
      for (let i = used; i < pool.length; i++) pool[i].visible = false;
    },
    setOwnerCodes(codes) {
      ownerCodes = codes;
    },
    dispose() {
      group.clear(); // 全Line2が共有する資源は以下でそれぞれ1回だけ破棄する
      geometry.dispose();
      for (const material of Object.values(materials)) material.dispose();
    },
  };
}

/**
 * Face.edgesへ入らない補助線・行き止まりの折り線を、基本outlineへ足す描画層。
 * materialは現在表示中のrigid/soft outlineそのものを借り、ここでは生成・破棄しない。
 */
export interface SupplementalEdgeLayer {
  readonly group: THREE.Group;
  setSegments(
    segments: readonly HingeSegment[],
    material: THREE.LineBasicMaterial,
    ownerCodes: ReadonlyMap<number, number>,
  ): void;
  clear(): void;
  dispose(): void;
}

function finiteVector3(point: THREE.Vector3): boolean {
  return Number.isFinite(point.x) && Number.isFinite(point.y) && Number.isFinite(point.z);
}

/**
 * 補足線を、既存outlineと同じ非indexed属性へ変換する。
 * ownerFaceが無い／現在のsurfaceに無い線は、裏線を漏らすany判定へ落とさず描かない。
 */
export function createSupplementalEdgeLayer(): SupplementalEdgeLayer {
  const group = new THREE.Group();
  let outline: SurfaceOwnerOutlineGeometry | null = null;

  const clear = () => {
    group.clear();
    outline?.geometry.dispose();
    outline = null;
  };

  return {
    group,
    setSegments(segments, material, ownerCodes) {
      clear();
      const positions: number[] = [];
      const tokens: number[] = [];
      const lineIndices: number[] = [];
      const lineProbeIndices: number[] = [];
      for (const segment of segments) {
        if (!finiteVector3(segment.a) || !finiteVector3(segment.b)) continue;
        if (segment.a.distanceToSquared(segment.b) < 1e-18) continue;
        const code =
          segment.ownerFace === undefined ? undefined : ownerCodes.get(segment.ownerFace);
        if (code === undefined) continue;
        const bytes = ownerCodeBytes(code);
        const base = positions.length / 3;
        const probe =
          segment.surfaceProbe && finiteVector3(segment.surfaceProbe)
            ? segment.surfaceProbe
            : segment.a;
        for (const point of [segment.a, segment.b, probe]) {
          positions.push(point.x, point.y, point.z);
          tokens.push(...bytes);
        }
        lineIndices.push(base, base + 1);
        lineProbeIndices.push(base + 2);
      }
      if (lineIndices.length === 0) return;

      const sourcePosition = new THREE.BufferAttribute(new Float32Array(positions), 3);
      const sourceToken = new THREE.Uint8BufferAttribute(new Uint8Array(tokens), 4, true);
      outline = createSurfaceOwnerOutlineGeometry({
        sourcePosition,
        sourceToken,
        lineIndices,
        lineProbeIndices,
      });
      const line = new THREE.LineSegments(outline.geometry, material);
      line.renderOrder = 1;
      line.frustumCulled = false;
      group.add(line);
    },
    clear,
    dispose: clear,
  };
}

/** 面・線1つ分の資源を破棄する */
export function disposeDrawable(child: THREE.Object3D): void {
  if (!(child instanceof THREE.Mesh || child instanceof THREE.LineSegments)) return;
  child.geometry.dispose();
  const material = child.material;
  if (Array.isArray(material)) {
    for (const m of material) m.dispose();
  } else {
    material.dispose();
  }
}

/** グループの中身を破棄して空にする(ジオメトリ・マテリアルのリーク防止) */
export function clearGroup(group: THREE.Group): void {
  for (const child of [...group.children]) {
    group.remove(child);
    disposeDrawable(child);
  }
}

/** 折った結果の半透明下見。紙の内側まで見る用途なのでowner/depth判定を通さない。 */
export function createPreviewMaterial(): THREE.MeshBasicMaterial {
  return new THREE.MeshBasicMaterial({
    color: PREVIEW_COLOR,
    transparent: true,
    opacity: PREVIEW_OPACITY,
    side: THREE.DoubleSide,
    depthTest: false,
  });
}

