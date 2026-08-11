// UI-012: 実際のキャンバスを触りながら進む、画面隅の非モーダル初回ガイド。
// 「次へ」ボタンでは進まず、ストアが実操作の成功を検知したときだけ段階が変わる。

import { useAppStore, type GuideStep } from "../store/appStore";

interface GuidePage {
  title: string;
  instruction: string;
  note: string;
  actionLabel: string;
  prepare: () => void;
}

export function FirstRunGuide() {
  const open = useAppStore((s) => s.guideOpen);
  const step = useAppStore((s) => s.guideStep);
  const dismiss = useAppStore((s) => s.dismissGuide);
  const setTool = useAppStore((s) => s.setTool);
  const setSelection = useAppStore((s) => s.setSelection);
  const setSoft = useAppStore((s) => s.setSoft);

  if (!open) return null;

  if (step === 4) {
    return (
      <aside
        className="first-run-guide complete"
        aria-label="基本操作ガイド完了"
        data-floating-ui="first-run-guide"
      >
        <button
          type="button"
          className="first-run-guide-close"
          aria-label="基本操作ガイドを閉じる"
          onClick={dismiss}
        >
          ×
        </button>
        <div className="first-run-guide-celebration" aria-hidden="true">
          ✓
        </div>
        <small>基本操作ガイド</small>
        <h2>できました！</h2>
        <p>折る・角度・引く・ふくらます。4つの基本操作をすべて試せました。</p>
        <button type="button" className="first-run-guide-done" onClick={dismiss}>
          作品づくりを続ける
        </button>
      </aside>
    );
  }

  const pages: Record<Exclude<GuideStep, 4>, GuidePage> = {
    0: {
      title: "線を引いて折ってみよう",
      instruction:
        "3Dの紙で Ctrl を押しながらドラッグして折り線を引き、下のパネルで向きと動く側を選んで「折る」を押します。",
      note: "紙をそのままドラッグする、かんたんな折り方でも達成できます。",
      actionLabel: "「折る」ツールにする",
      prepare: () => setTool("fold"),
    },
    1: {
      title: "角度を変えてみよう",
      instruction:
        "「選択」にして、3Dまたは展開図の折り線をクリック。下に出る「折り角度」のつまみを動かします。",
      note: "＋は山折り、−は谷折り。形を見ながら何度でも調整できます。",
      actionLabel: "折り線を選べる状態にする",
      prepare: () => setTool("select"),
    },
    2: {
      title: "紙を引いて動かそう",
      instruction:
        "「引く」にして3Dの紙をつかみ、動かしたい方へドラッグしてから離します。折り線のつじつまを保って全体が動きます。",
      note: "右ドラッグなら、引くモードのまま視点を回せます。",
      actionLabel: "「引く」ツールにする",
      prepare: () => setTool("pull"),
    },
    3: {
      title: "紙をふくらませよう",
      instruction:
        "下の「紙をふくらませる」で丸みをオンにし、「膨らみの強さ」のつまみを0より大きく動かします。",
      note: "袋になったところへ空気を入れるような丸みが、その場で3Dに映ります。",
      actionLabel: "ふくらみ設定を表示",
      prepare: () => {
        setTool("select");
        setSelection({ edgeIds: [], vertexIds: [] });
        setSoft({ soft_enabled: true });
      },
    },
  };
  const page = pages[step];

  return (
    <aside
      className="first-run-guide"
      aria-label="基本操作ガイド"
      data-floating-ui="first-run-guide"
    >
      <button
        type="button"
        className="first-run-guide-close"
        aria-label="基本操作ガイドを閉じる"
        onClick={dismiss}
      >
        ×
      </button>
      <div className="first-run-guide-header">
        <span>はじめてガイド</span>
        <strong>{step + 1} / 4</strong>
      </div>
      <ol className="first-run-guide-progress" aria-label="ガイドの進み具合">
        {[0, 1, 2, 3].map((number) => (
          <li
            key={number}
            className={number === step ? "current" : number < step ? "completed" : "pending"}
            aria-current={number === step ? "step" : undefined}
            aria-label={`ステップ${number + 1}`}
          >
            {number < step ? "✓" : number + 1}
          </li>
        ))}
      </ol>
      <span className="first-run-guide-try">やってみて</span>
      <h2>{page.title}</h2>
      <p>{page.instruction}</p>
      <p className="first-run-guide-note">ヒント: {page.note}</p>
      <button type="button" className="first-run-guide-prepare" onClick={page.prepare}>
        {page.actionLabel}
      </button>
      <button type="button" className="first-run-guide-skip" onClick={dismiss}>
        ガイドをスキップ
      </button>
      <small className="first-run-guide-auto">操作できると自動で次へ進みます</small>
    </aside>
  );
}
