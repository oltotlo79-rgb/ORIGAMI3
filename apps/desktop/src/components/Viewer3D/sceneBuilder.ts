// Viewer3D sceneの互換facade。
// C1〜C4の実装所有先を分けても、既存consumerはこの入口と公開名を変えずに使う。
// uiTokens.test.tsが意味色の変更を検知するために読む互換mirror。実装の所有先はsceneLayers。
// HIGHLIGHT_COLOR = 0xffd400
// REFERENCE_HIGHLIGHT_COLOR = 0x40cfff
// SUSPECT_HIGHLIGHT_COLOR = 0xff2038
// ACTIVE_HIGHLIGHT_COLOR = 0x40cfff
// PREVIEW_COLOR = 0x2f8fff

export {
  buildTopology,
  contentBoundingBox,
  createContent,
  createSoftContent,
  updateFrame,
  updateSoftContent,
} from "./sceneContent";
export type {
  FaceSlot,
  HingeSlot,
  SoftContent,
  Topology,
  Viewer3DContent,
} from "./sceneContent";

export {
  CAMERA_DIR,
  applyCameraDragRotation,
  applyPaperFraming,
  boxCorners,
  boxFraming,
  cameraScreenUp,
  legacyPaperDistance,
  paperCorners,
  paperFraming,
  rotateCameraByDrag,
  viewRotationStarts,
} from "./sceneCamera";
export type {
  CameraOrbitPose,
  PaperFraming,
  ScreenBounds,
  ViewRotationMouseButtons,
} from "./sceneCamera";

export {
  FOCUS_HIGHLIGHT_WIDTH_PX,
  HIGHLIGHT_WIDTH_PX,
  PIN_MARK_MAX_LENGTH,
  PIN_MARK_MIN_LENGTH,
  PIN_MARK_RATIO,
  PIN_MARK_WIDTH_PX,
  SUSPECT_HIGHLIGHT_WIDTH_PX,
  clearGroup,
  createHighlightGeometry,
  createHighlightLayer,
  createHighlightMaterials,
  createPreviewMaterial,
  createSupplementalEdgeLayer,
  highlightAppearance,
  withPinMarks,
} from "./sceneLayers";
export type {
  HighlightLayer,
  HighlightLineMaterial,
  HighlightMaterials,
  HighlightSegment,
  SupplementalEdgeLayer,
} from "./sceneLayers";

export {
  canvas3dBackgroundColor,
  captureViewer3DReadback,
  createScene,
} from "./sceneFacade";
export type { Viewer3DReadback, Viewer3DScene } from "./sceneFacade";
