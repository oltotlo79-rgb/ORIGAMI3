import type { HelpChapter } from "../helpTypes";

export const newPaperChapter = {
  id: "new-paper",
  number: 3,
  title: "新しい紙を用意する",
  summary: "正方形・長方形の寸法をmmで決め、表と裏を24色から選べます。",
  blocks: [
    {
      type: "paragraph",
      text: "起動直後は一般的な折り紙と同じ150×150mmの紙が用意されます。別の形や大きさで始めたいときは、上部の「新規」から取り替えます。",
    },
    { type: "figure", diagramId: "new-paper-settings" },
    {
      type: "steps",
      title: "紙の形と大きさを決める",
      items: [
        { title: "「新規」を押す", description: "「新しい紙を用意する」画面が開きます。" },
        { title: "形を選ぶ", description: "「正方形（たて・よこが同じ）」または「長方形（たて・よこを別に決める）」を選びます。" },
        { title: "寸法を入れる", description: "「よこ(mm)」と「たて(mm)」へ0より大きい数を入れます。正方形では、たてはよこと同じになります。" },
        { title: "近道を使う", description: "必要なら「折り紙 15cm角」「折り紙 24cm角」「A4の紙」を押して寸法をすぐに入れます。" },
        { title: "作りはじめる", description: "見本の形を確かめ、「この紙で作りはじめる」を押します。" },
      ],
    },
    {
      type: "heading",
      text: "紙の表と裏の色を変える",
    },
    {
      type: "paragraph",
      text: "線や手順を何も選んでいないとき、下部に「紙の表」と「紙の裏」の色見本が出ます。表裏を違う色にすると、折り返した場所が立体表示で見分けやすくなります。",
    },
    {
      type: "bulletList",
      title: "用意されている24色",
      items: [
        "赤・朱・桃・桜・橙・山吹・黄・レモン",
        "黄緑・緑・深緑・水色・空色・青・紺・紫",
        "藤・茶・肌色・金茶・銀鼠・白・灰・黒",
        "「その他の色」では、色見本にない好みの色も選べます。",
      ],
    },
    {
      type: "steps",
      title: "紙の色を選ぶ",
      items: [
        { title: "空いている場所を押す", description: "展開図または立体表示で、線や折り目を選んでいない状態にします。" },
        { title: "表の色を押す", description: "下部の「紙の表」で24色の見本から一色を選びます。" },
        { title: "裏の色を押す", description: "同じように「紙の裏」を選び、立体表示で折り返しを確認します。" },
      ],
    },
    {
      type: "callout",
      tone: "warning",
      title: "新規作成は今の作品と入れ替わります",
      text: "残したい作品は先に上部の「保存」を押してください。紙の色、方眼、たわみ、重なり防止の設定も作品ファイルに保存されます。",
    },
  ],
} satisfies HelpChapter;
