import type { HelpChapter } from "../helpTypes";

export const overviewChapter = {
  id: "overview",
  number: 1,
  title: "ORIGAMI3でできること",
  summary: "展開図を描き、紙を折り、完成までの手順を一つの作品として残せます。",
  blocks: [
    {
      type: "paragraph",
      text: "ORIGAMI3は、折り線を描く平らな展開図と、折った姿を確かめる立体表示を並べて使う折り紙設計アプリです。紙を直接つかんで折ることも、折り目と角度を細かく決めることもできます。",
    },
    { type: "figure", diagramId: "overview-flow", image: "screen-overview-guide.png" },
    { type: "heading", text: "作品づくりの流れ" },
    {
      type: "steps",
      title: "最初の作品を作る",
      items: [
        { title: "紙を用意する", description: "上部の「新規」を押し、紙の形と大きさを決めます。" },
        { title: "折り線を描く", description: "左端の「山」「谷」「補助」を選び、展開図で線の始めと終わりを押します。" },
        { title: "紙を折る", description: "左端の「折る」を選び、立体表示の紙をつかんで折りたい場所へ動かします。" },
        { title: "形を整える", description: "折り目の角度、引く操作、たわみやふくらみを使い、立体の姿を整えます。" },
        { title: "残して伝える", description: "「保存」で作品を残し、「書き出し」で展開図の画像や折り図を作ります。" },
      ],
    },
    {
      type: "bulletList",
      title: "設計を助ける機能",
      items: [
        "折った操作は順番に記録され、途中の形を見たり自動で再生したりできます。",
        "段折り・中割り折りなど、よく使う8つの技法をまとめて適用できます。",
        "頭・尾・足のような出っぱりを決めると、展開図の候補を最大4つ提案します。",
        "畳みにくい場所や紙の突き抜けは警告で知らせます。警告を読みながら設計を続けられます。",
      ],
    },
    {
      type: "callout",
      tone: "tip",
      title: "まずは紙を触ってみましょう",
      text: "このヘルプの目次下にある「基本操作ガイドをもう一度」では、折る・角度・引く・ふくらますを実際の画面で練習できます。",
    },
  ],
} satisfies HelpChapter;
