// 3Dビューへ重ねる操作の吹き出し。現在モードとマウス割り当てを常時見せ、
// 詳細だけを折りたためるようにする(UI-009 / UI-012)。

import { useAppStore, type ToolId } from "../../store/appStore";
import { SELECTABLE_3D_EDGE_TARGETS } from "../../lib/viewerHint";
import { ToolIcon } from "../ToolIcons";

interface Props {
  hint: string;
  blocked: boolean;
  aligning?: boolean;
}

interface Assignment {
  control: "left" | "right" | "wheel";
  label: string;
  action: string;
}

const MODE_LABEL: Record<ToolId, string> = {
  select: "見る・選ぶ",
  mountain: "山折り線",
  valley: "谷折り線",
  aux: "補助線",
  delete: "削除",
  fold: "折る",
  pull: "引く",
  technique: "技法",
  construct: "作図",
};

/** Three.js側のOrbitControlsと紙操作の割り当てを、そのまま日本語へ直す。 */
export function viewerAssignments(
  tool: ToolId,
  blocked = false,
  aligning = false,
): Assignment[] {
  if (blocked) {
    return [
      { control: "left", label: "左ドラッグ", action: "視点を回す" },
      { control: "right", label: "右ドラッグ", action: "視点を動かす" },
      { control: "wheel", label: "ホイール", action: "拡大・縮小" },
    ];
  }
  if (tool === "pull") {
    return [
      { control: "left", label: "左ドラッグ", action: "紙を引く" },
      { control: "right", label: "右ドラッグ", action: "視点を回す" },
      { control: "wheel", label: "ホイール", action: "拡大・縮小" },
    ];
  }
  if (tool === "fold") {
    if (aligning) {
      return [
        {
          control: "left",
          label: "左クリック／ドラッグ",
          action: `点・${SELECTABLE_3D_EDGE_TARGETS}を選ぶ／視点を回す`,
        },
        { control: "right", label: "右ドラッグ", action: "視点を動かす" },
        { control: "wheel", label: "ホイール", action: "拡大・縮小" },
      ];
    }
    return [
      { control: "left", label: "左ドラッグ", action: "紙をつかんで折る" },
      { control: "right", label: "右ドラッグ", action: "視点を動かす" },
      { control: "wheel", label: "ホイール", action: "拡大・縮小" },
    ];
  }
  if (tool === "technique") {
    return [
      {
        control: "left",
        label: "左クリック／ドラッグ",
        action: "層・角・基準点／折り線を選ぶ",
      },
      { control: "right", label: "右ドラッグ", action: "視点を動かす" },
      { control: "wheel", label: "ホイール", action: "拡大・縮小" },
    ];
  }
  if (tool === "select") {
    return [
      {
        control: "left",
        label: "左クリック／ドラッグ",
        action: `${SELECTABLE_3D_EDGE_TARGETS}を選ぶ／視点を回す`,
      },
      { control: "right", label: "右ドラッグ", action: "視点を動かす" },
      { control: "wheel", label: "ホイール", action: "拡大・縮小" },
    ];
  }
  return [
    { control: "left", label: "左ドラッグ", action: "視点を回す" },
    { control: "right", label: "右ドラッグ", action: "視点を動かす" },
    { control: "wheel", label: "ホイール", action: "拡大・縮小" },
  ];
}

function MouseGlyph({ control }: { control: Assignment["control"] }) {
  return (
    <svg className={`mouse-glyph ${control}`} viewBox="0 0 28 28" aria-hidden="true">
      <rect x="6" y="3" width="16" height="22" rx="8" />
      <path d="M14 3v8M6 11h16" />
      {control === "left" && <path className="mouse-glyph-active" d="M7 10V9a6 6 0 0 1 6-5v6Z" />}
      {control === "right" && <path className="mouse-glyph-active" d="M15 4a6 6 0 0 1 6 5v1h-6Z" />}
      {control === "wheel" && <rect className="mouse-glyph-active" x="12" y="5" width="4" height="7" rx="2" />}
    </svg>
  );
}

export function ViewerOperationHint({ hint, blocked, aligning = false }: Props) {
  const activeTool = useAppStore((s) => s.activeTool);
  const expanded = useAppStore((s) => s.viewerHintExpanded);
  const paperActionTipOpen = useAppStore(
    (s) => s.paperActionTipVisible && s.paperActionTipExpanded,
  );
  const toggle = useAppStore((s) => s.toggleViewerHint);
  const assignments = viewerAssignments(activeTool, blocked, aligning);

  return (
    <aside
      className={`viewer-operation-hint${expanded ? " expanded" : " collapsed"}${blocked ? " blocked" : ""}${paperActionTipOpen ? " paper-action-tip-open" : ""}`}
      aria-label="3Dビューの操作ヒント"
      data-floating-ui="viewer-operation-hint"
    >
      <div className="viewer-operation-heading">
        <span className="viewer-mode-icon">
          <ToolIcon tool={activeTool} />
        </span>
        <span className="viewer-mode-copy">
          <small>いまのモード</small>
          <strong>{MODE_LABEL[activeTool]}</strong>
        </span>
      </div>
      <div className="viewer-current-row">
        <p
          className="viewer-current-action operation-summary-line"
          role="status"
          data-tooltip={hint}
        >
          {hint}
        </p>
        <button
          type="button"
          className="viewer-hint-toggle operation-detail-toggle"
          aria-label={expanded ? "詳しい3D操作方法を折りたたむ" : "詳しい3D操作方法を開く"}
          aria-expanded={expanded}
          data-tooltip={expanded ? "マウス操作の説明を折りたたみます" : "マウス操作の説明を開きます"}
          onClick={toggle}
        >
          <span className="operation-detail-label">詳しい3D操作方法</span>
          <span className="operation-detail-icon">{expanded ? "▲" : "▼"}</span>
        </button>
      </div>
      {expanded && (
        <div className="viewer-mouse-assignments" aria-label="マウス操作の割り当て">
          {assignments.map((item) => (
            <div className="viewer-mouse-assignment" key={item.control}>
              <MouseGlyph control={item.control} />
              <span>
                <small>{item.label}</small>
                <strong>{item.action}</strong>
              </span>
            </div>
          ))}
        </div>
      )}
    </aside>
  );
}
