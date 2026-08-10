// 選択中ツールで「次に何をすればよいか」を、下部パネルへ折りたたんで出す。
// 操作そのものは既存のキャンバスへ任せ、ここではZustandの進行段階を読むだけにする。

import { useAppStore, type ToolId } from "../store/appStore";
import { ALIGN_LABELS } from "../lib/alignFold";

interface OperationGuide {
  title: string;
  steps: string[];
  current: number;
}

const LINE_LABEL: Partial<Record<ToolId, string>> = {
  mountain: "山折り線を引く",
  valley: "谷折り線を引く",
  aux: "補助線を引く",
};

/** ツールと途中状態から、短い手順と強調位置を組み立てる。 */
export function operationGuideFor(s: ReturnType<typeof useAppStore.getState>): OperationGuide {
  const lineTitle = LINE_LABEL[s.activeTool];
  if (lineTitle) {
    return {
      title: lineTitle,
      steps: ["展開図で始点をクリック", "終点をクリック", "線が完成"],
      current: s.operationStage,
    };
  }

  if (s.activeTool === "fold") {
    if (s.pendingFoldThrough) {
      return {
        title: "追加の折り目を確認する",
        steps: ["水色の線を確認", "折り方を選ぶ", "折りを確定"],
        current: 1,
      };
    }
    if (s.alignDraft) {
      const picked = s.alignDraft.picks.length;
      return {
        title: ALIGN_LABELS[s.alignDraft.mode],
        steps: [
          "案内どおりに点・線を順に選ぶ",
          "求まった折り目を確認する",
          "向きと動く側を決めて折る",
        ],
        current: s.foldDraft ? 2 : Math.min(picked, 1),
      };
    }
    if (s.foldDraft) {
      return {
        title: "線を決めて折る",
        steps: ["Ctrl+ドラッグで折り線を引く", "折る向き・動く側を選ぶ", "「折る」で確定"],
        current: Math.max(1, s.operationStage),
      };
    }
    return {
      title: "紙を折る",
      steps: ["3Dの紙をつかむ", "折りたい方へドラッグ", "離して折る"],
      current: s.operationStage,
    };
  }

  if (s.activeTool === "pull") {
    return {
      title: "紙を引いて動かす",
      steps: ["3Dの紙をつかむ", "動かしたい方へドラッグ", "離して形を残す"],
      current: s.operationStage,
    };
  }

  if (s.activeTool === "technique") {
    if (s.techniqueDraft?.kind === "Simple") {
      const draft = s.techniqueDraft;
      const hasPart = draft.motionParts.length > 0;
      const hasCurrent =
        draft.flap.length > 0 ||
        draft.line !== null ||
        draft.motionReverseLayers ||
        (draft.motionMode === "stay" && draft.motionTurn !== "Keep");
      return {
        title: "層を開く・重ね替える",
        steps: [
          "3Dで対象層を選び、開閉なら既存折り目をクリック",
          "重ね方・向き・山谷反転を決め、必要なら部分を追加",
          "「まとめて適用」で1手として確定",
        ],
        current: hasPart ? 2 : hasCurrent ? 1 : 0,
      };
    }
    if (s.techniqueDraft?.kind === "Twist") {
      const n = s.techniqueDraft.polygon.length;
      return {
        title: "ねじり折り",
        steps: ["中央の角を3つ以上クリック", "中心とねじる角を調整", "「適用」で折る"],
        current: n >= 3 ? 1 : 0,
      };
    }
    return {
      title: "技法で折る",
      steps: ["左で技法を選ぶ", "3Dで紙の層と折り線を選ぶ", "「適用」で折る"],
      current: !s.techniqueDraft
        ? 0
        : s.techniqueDraft.line
          ? 2
          : s.techniqueDraft.flap.length > 0
            ? 1
            : 0,
    };
  }

  if (s.activeTool === "construct") {
    return {
      title: "作図の補助線を引く",
      steps: ["左で作図の種類を選ぶ", "展開図の点・線を順にクリック", "できた補助線を確認"],
      current: s.operationStage,
    };
  }

  if (s.activeTool === "delete") {
    return {
      title: "線を削除する",
      steps: ["消したい線にカーソルを合わせてクリック"],
      current: 0,
    };
  }

  return {
    title: "紙と折り線を選ぶ",
    steps: ["クリック（Ctrlで複数選択）", "折り角度を個別・一括で変える"],
    current:
      s.selection.edgeIds.length > 0 || s.selection.vertexIds.length > 0 ? 1 : 0,
  };
}

export function OperationSteps() {
  // 関係する値が変わったときだけ手順を組み直す。
  const activeTool = useAppStore((s) => s.activeTool);
  const operationStage = useAppStore((s) => s.operationStage);
  const selection = useAppStore((s) => s.selection);
  const foldDraft = useAppStore((s) => s.foldDraft);
  const pendingFoldThrough = useAppStore((s) => s.pendingFoldThrough);
  const alignDraft = useAppStore((s) => s.alignDraft);
  const techniqueDraft = useAppStore((s) => s.techniqueDraft);
  const guide = operationGuideFor({
    ...useAppStore.getState(),
    activeTool,
    operationStage,
    selection,
    foldDraft,
    pendingFoldThrough,
    alignDraft,
    techniqueDraft,
  });
  const current = Math.min(Math.max(guide.current, 0), guide.steps.length - 1);

  return (
    <section className="operation-steps" aria-label={`${guide.title}の操作手順`}>
      <details>
        <summary className="operation-steps-heading">
          <span>今できる操作</span>
          <strong>{guide.title}</strong>
        </summary>
        <ol>
          {guide.steps.map((step, index) => (
            <li
              key={step}
              className={
                index === current
                  ? "current"
                  : index < current
                    ? "completed"
                    : "pending"
              }
              aria-current={index === current ? "step" : undefined}
            >
              <span className="operation-step-number" aria-hidden="true">
                {index < current ? "✓" : index + 1}
              </span>
              <span>{step}</span>
            </li>
          ))}
        </ol>
      </details>
    </section>
  );
}
