import { describe, expect, it } from "vitest";
import { HELP_CHAPTERS, helpBlockText, helpChapterSearchText } from "./helpContent";
import { HELP_DIAGRAMS } from "./helpDiagrams";
import type { HelpBlock, HelpScreenshotBlock } from "./helpTypes";

const REAL_SCREEN_DIAGRAM_IDS = [
  "workspace-four-areas",
  "new-paper-settings",
  "crease-tools",
  "angle-controls",
  "three-dimensional-controls",
  "technique-cards",
  "timeline-flow",
] as const;

const INTENTIONAL_HAND_DRAWN_DIAGRAM_IDS = [
  "overview-flow",
  "fold-flow",
  "save-export-flow",
  "troubleshooting-flow",
  "shortcut-map",
] as const;

const DEFERRED_DIAGRAM_ID = "proposal-wizard" as const;

function manualImage(block: HelpBlock): string | null {
  if (block.type === "figure") return block.image?.trim() || null;
  if (block.type === "screenshot") return block.image.trim() || null;
  return null;
}

function userFacingBlockText(block: HelpBlock): string {
  if (block.type === "screenshot") return block.caption;
  if (block.type !== "figure") return helpBlockText(block);

  const diagram = HELP_DIAGRAMS[block.diagramId];
  const svgText = diagram.svg.replace(/<[^>]*>/g, " ");
  return [diagram.title, diagram.alt, svgText].join(" ");
}

function userFacingChapterText(chapter: (typeof HELP_CHAPTERS)[number]): string {
  return [chapter.title, chapter.summary, ...chapter.blocks.map(userFacingBlockText)].join(" ");
}

describe("ヘルプと取扱説明書PDFの共通内容源", () => {
  it("13章が番号順に並び、章IDと題がそろっている", () => {
    expect(HELP_CHAPTERS).toHaveLength(13);
    expect(HELP_CHAPTERS.map((chapter) => chapter.number)).toEqual(
      Array.from({ length: 13 }, (_, index) => index + 1),
    );
    expect(new Set(HELP_CHAPTERS.map((chapter) => chapter.id)).size).toBe(13);

    for (const chapter of HELP_CHAPTERS) {
      expect(chapter.title.trim().length).toBeGreaterThan(0);
      expect(chapter.summary.trim().length).toBeGreaterThan(0);
      expect(chapter.blocks.some((block) => block.type === "paragraph")).toBe(true);
      expect(chapter.blocks.some((block) => block.type === "steps")).toBe(true);
      expect(chapter.blocks.some((block) => block.type === "callout")).toBe(true);
      expect(chapter.blocks.map(helpBlockText).join(" ").trim().length).toBeGreaterThan(0);
    }
  });

  it("全章に解決できる固有の図が1点以上ある", () => {
    const figureIds = HELP_CHAPTERS.flatMap((chapter) =>
      chapter.blocks
        .filter((block) => block.type === "figure")
        .map((block) => block.diagramId),
    );

    expect(figureIds).toHaveLength(13);
    expect(new Set(figureIds).size).toBe(13);
    expect(new Set(Object.keys(HELP_DIAGRAMS))).toEqual(new Set(figureIds));
    expect(new Set(Object.values(HELP_DIAGRAMS).map((diagram) => diagram.svg)).size).toBe(13);

    for (const id of figureIds) {
      const diagram = HELP_DIAGRAMS[id];
      expect(diagram.title.trim().length).toBeGreaterThan(0);
      expect(diagram.alt.trim().length).toBeGreaterThan(0);
      expect(diagram.svg).toContain("<svg");
      expect(diagram.svg).toContain('viewBox="0 0 720 280"');
      expect(diagram.svg).toContain(`<title>${diagram.title}</title>`);
      expect(diagram.svg).toContain(`<desc>${diagram.alt}</desc>`);
    }
  });

  it("対象7図だけが追跡対象の実画面PNGを表示し、PDF用画像名もそろっている", () => {
    expect(REAL_SCREEN_DIAGRAM_IDS).toHaveLength(7);
    const imageUrls = new Set<string>();

    for (const id of REAL_SCREEN_DIAGRAM_IDS) {
      const diagram = HELP_DIAGRAMS[id];
      const expectedImage = `figure-${id}.png`;
      expect(diagram.manualImage).toBe(expectedImage);
      expect(diagram.svg).toContain("<image ");
      expect(diagram.svg).toContain(expectedImage);
      expect(diagram.svg).toContain('width="720" height="280"');
      expect(diagram.svg).toContain('preserveAspectRatio="none"');
      const href = diagram.svg.match(/<image href="([^"]+)"/)?.[1];
      expect(href).toBeTruthy();
      imageUrls.add(href!);
    }
    expect(imageUrls.size).toBe(7);
  });

  it("意図して手描きで残す5図と延期する提案図を区別する", () => {
    expect(INTENTIONAL_HAND_DRAWN_DIAGRAM_IDS).toHaveLength(5);
    for (const id of INTENTIONAL_HAND_DRAWN_DIAGRAM_IDS) {
      const diagram = HELP_DIAGRAMS[id];
      expect(diagram.manualImage).toBeUndefined();
      expect(diagram.svg).not.toContain("<image ");
      expect(diagram.svg.length).toBeGreaterThan(500);
    }

    const deferred = HELP_DIAGRAMS[DEFERRED_DIAGRAM_ID];
    expect(deferred.manualImage).toBeUndefined();
    expect(deferred.svg).not.toContain("<image ");
    expect(deferred.svg.length).toBeGreaterThan(500);
  });

  it("全章に実画面が1点以上あり、独立した画面例も直列化できる", () => {
    const blocks: HelpBlock[] = HELP_CHAPTERS.flatMap((chapter) => [...chapter.blocks]);
    const screenshots = blocks.filter(
      (block): block is HelpScreenshotBlock => block.type === "screenshot",
    );
    const images = blocks.map(manualImage).filter((image): image is string => image !== null);

    for (const chapter of HELP_CHAPTERS) {
      expect(chapter.blocks.some(manualImage), `第${chapter.number}章`).toBe(true);
    }
    expect(screenshots).toHaveLength(21);
    for (const screenshot of screenshots) {
      expect(screenshot.image).toMatch(/^screen-[a-z0-9-]+\.png$/);
      expect(screenshot.caption.trim().length).toBeGreaterThan(0);
    }
    expect(images).toHaveLength(34);
    expect(new Set(images).size).toBe(34);
  });

  it("本文は表示部品を含まない直列化可能なデータで、題と本文を検索できる", () => {
    expect(JSON.parse(JSON.stringify(HELP_CHAPTERS))).toEqual(HELP_CHAPTERS);
    const proposal = HELP_CHAPTERS.find((chapter) => chapter.id === "proposal");
    const crease = HELP_CHAPTERS.find((chapter) => chapter.id === "crease-pattern");
    expect(proposal && helpChapterSearchText(proposal)).toContain("形から展開図を提案");
    expect(crease && helpChapterSearchText(crease)).toContain("ベジェ曲線");
  });

  it("v0.4.4で加わった利用者向け8機能を既存章で説明する", () => {
    const features = [
      {
        number: 4,
        checks: [
          {
            chapterId: "fold",
            phrases: [
              "8種類で必要な点や線",
              "2D展開図と3D立体表示のどちらからでも押して選べます",
            ],
          },
        ],
      },
      {
        number: 5,
        checks: [
          {
            chapterId: "three-dimensional",
            phrases: ["補助線と紙のふち（輪郭の辺）も選べます"],
          },
        ],
      },
      {
        number: 6,
        checks: [
          {
            chapterId: "crease-pattern",
            phrases: [
              "山・谷・補助のまっすぐな線",
              // v0.4.5で吸着が届く範囲まで受け付けるようになったため、
              // 「紙の外を押しても線は作られず」から現状の記述へ差し替えた。
              "吸着が届かないほど外側を押したときだけ、線は作られず画面に理由が出ます",
              "紙のふちにある点は動かせません",
            ],
          },
        ],
      },
      {
        number: 7,
        checks: [
          {
            chapterId: "proposal",
            phrases: [
              "形から展開図を提案",
              "＋ この先に足す",
              "新しく足した先でも同じ操作を繰り返せます",
              "先端は1〜12本",
            ],
          },
        ],
      },
      {
        number: 8,
        checks: [
          {
            chapterId: "crease-pattern",
            phrases: [
              "二等分・垂線・等分・角度線を途中で切り替えた場合",
              "「曲線で描く」のオン・オフや円弧・ベジェの切り替え",
              "すでに完成した線は残ります",
            ],
          },
        ],
      },
      {
        number: 9,
        checks: [
          {
            chapterId: "timeline",
            phrases: [
              "後の手順で足される未表示の線",
              "選ぶ・消す・吸着する・作図の基準にすることはできません",
              "「最新」を押すと",
            ],
          },
        ],
      },
      {
        number: 10,
        checks: [
          {
            chapterId: "angles",
            phrases: ["「引く」を押す", "立体表示の紙を左ボタンでつかみ"],
          },
          {
            chapterId: "techniques",
            phrases: [
              "ほかの技法は、中心線や折り返す線を立体表示でドラッグします",
              "立体表示をCtrl+クリック",
            ],
          },
        ],
      },
      {
        number: 11,
        checks: [
          {
            chapterId: "three-dimensional",
            phrases: [
              "「この形で仕上げる」は作業を終えるボタンではなく、1手を記録します",
              "その姿から続けて折れます",
              "新しい折り線は2D展開図へ、次の手順はタイムラインへ加わります",
            ],
          },
        ],
      },
    ] as const;

    expect(features).toHaveLength(8);
    expect(features.map((feature) => feature.number)).toEqual([4, 5, 6, 7, 8, 9, 10, 11]);
    for (const feature of features) {
      for (const check of feature.checks) {
        const chapter = HELP_CHAPTERS.find((entry) => entry.id === check.chapterId);
        expect(chapter, `機能#${feature.number}: ${check.chapterId}`).toBeDefined();
        const text = helpChapterSearchText(chapter!);
        for (const phrase of check.phrases) {
          expect(text, `機能#${feature.number}: ${phrase}`).toContain(phrase);
        }
      }
    }
  });

  it("v0.4.5で変わった利用者向けの表示・操作を既存章で説明する", () => {
    const changes = [
      {
        name: "3Dの視点が止まらずに一回りできる",
        chapterId: "three-dimensional",
        phrases: [
          "紙の真上や真下も通り越して一回りできます",
          "左右のドラッグは画面の上下を軸にして回します",
          "水平線がだんだん傾いて見えます",
          "「視点を戻す」を押すと、まっすぐな向きに戻ります",
        ],
      },
      {
        name: "見る向きを選ぶ立方体",
        chapterId: "three-dimensional",
        phrases: [
          "面6つ・辺12本・角8つの合わせて26箇所",
          "向かい合う面どうしは同じ色です",
          "立方体そのものを左ドラッグしても視点を回せます",
        ],
      },
      {
        name: "3Dの札の下も押せる",
        chapterId: "three-dimensional",
        phrases: [
          "札の下に紙・折り目・点が隠れていても、そのまま押して選べます",
          "札の中にある開閉のボタンは今までどおり押せます",
        ],
      },
      {
        name: "折り切った折り目は動かない",
        chapterId: "angles",
        phrases: [
          "0°または±180°まで折り切ってある折り目は、ほかの折り目を動かしてもそのまま保たれます",
          "指定した角度にならなかった折り目が2本あります",
          "紙が裂けないいちばん近い形を表示しています",
        ],
      },
      {
        name: "紙の重なりが正しく見える",
        chapterId: "three-dimensional",
        phrases: ["重なった紙は、手前にある紙が必ず手前に描かれます"],
      },
      {
        name: "追従の一覧に上限が無い",
        chapterId: "angles",
        phrases: [
              "追従した折り目は本数にかかわらずすべて並ぶので、「ほかN本」で隠れて見えないものはありません",
        ],
      },
      {
        name: "角・方眼の少し外も吸着で引ける",
        chapterId: "crease-pattern",
        phrases: [
          "紙のふち・角・方眼の交点のすぐ外側を押した場合も、いちばん近い紙の上の点へ吸い付いて線が引けます",
        ],
      },
      {
        name: "対称描画と1回で戻せる履歴",
        chapterId: "crease-pattern",
        phrases: [
          "二等分・垂線・等分・角度線の作図で作った線も、同じように反対側へ入ります",
          "その曲線で増えた線がすべて消え",
        ],
      },
      {
        name: "紙のふちを選んだときの4つのボタン",
        chapterId: "crease-pattern",
        phrases: ["紙のふちは紙そのものなので、山折りには変えられません"],
      },
      {
        name: "過去の手順の展開図と押せない操作",
        chapterId: "timeline",
        phrases: [
          "折る前から描いてあった線、補助線、輪郭は、どの手順を見ている間も消えずに残ります",
          "まだ引いていない先の線が原因の丸が過去の手順に出ることはありません",
          "いちばん最後の状態なので、これより先へは進めません",
          "前の手順の形を見ている間は引けません",
          "書いた直後に「前へ動かす」「後ろへ動かす」を押しても、入れた文はそのまま残ります",
        ],
      },
      {
        name: "保存した先が画面に出る",
        chapterId: "save-export",
        phrases: ["作品を「鶴.ori3」に保存しました"],
      },
      {
        name: "提案を採用する前の断り",
        chapterId: "proposal",
        phrases: [
          "今ある折り手順3件はすべて消えます",
          "手順がまだ0件のときは、この断りは出ません",
        ],
      },
      {
        name: "左右対称でも折るの下見は1本",
        chapterId: "fold",
        phrases: ["「折る」の下見は1本だけです"],
      },
      {
        name: "4区画の説明に立方体を書き添える",
        chapterId: "workspace",
        phrases: ["右上には見る向きを選ぶ立方体、右下には「視点を戻す」があります"],
      },
      {
        name: "警告の札と立方体の場所を区別する",
        chapterId: "troubleshooting",
        phrases: [
          "見る向きを選ぶ立方体の左どなりに出ます",
          "「指定した角度にならなかった折り目がn本あります」",
          "押せるのに何も起きないボタンは作らず",
        ],
      },
    ] as const;

    expect(changes).toHaveLength(15);
    for (const change of changes) {
      const chapter = HELP_CHAPTERS.find((entry) => entry.id === change.chapterId);
      expect(chapter, change.name).toBeDefined();
      const text = helpChapterSearchText(chapter!);
      for (const phrase of change.phrases) {
        expect(text, `${change.name}: ${phrase}`).toContain(phrase);
      }
    }
  });

  it("3D立体表示から点を選べるようになった13通りの操作を既存章で説明する", () => {
    // 3Dで点を指せるようになって使えるようになった操作を、章ごとに数え漏らさない。
    // 番号は「3Dから使えるようになった13通り」に対応する。
    const changes = [
      {
        name: "#1 選択-点を選ぶ / #2 点を動かす",
        chapterId: "three-dimensional",
        phrases: [
          "3Dの紙にある点（紙の角、折り目の端、線どうしの交点）もクリックして選べます",
          "2D展開図でも同じ点が選ばれた状態になります",
          "立体に折った姿のままでも選べます",
          "点をCtrl+クリックで選び足したり外したりできます",
          "そのままドラッグすると点を動かせます",
          "紙のふちにある点は動かせません",
        ],
      },
      {
        name: "#3〜#8 山・谷・補助の直線と曲線 / #10〜#13 作図",
        chapterId: "crease-pattern",
        phrases: [
          "山・谷・補助のまっすぐな線、曲線、二等分・垂線・等分・角度線の作図は、3D立体表示の紙の上を押しても同じように引けます",
          "立体に折った姿のままでも引けます",
          "引いた線は2D展開図の同じ場所へ出ます",
          "方眼への吸着と、既存線の延長・角の二等分へ向きをそろえる吸着は2D展開図だけの働きです",
          "点や線は2D展開図と3D立体表示のどちらから押しても同じです",
          "端点は3D立体表示の紙の上からもドラッグして動かせます",
        ],
      },
      {
        name: "#9 折る-2回のクリックで折り線",
        chapterId: "fold",
        phrases: [
          "Ctrlを押しながら紙の上を2回押しても同じ折り線を指定できます",
          "3D立体表示で押した点は展開図の同じ点なので",
        ],
      },
      {
        name: "2Dと3Dの対応表",
        chapterId: "three-dimensional",
        phrases: [
          "点を使う操作をどちらの区画からできるか",
          "折る前の折り線を2回のクリックで指定する",
          "線を消す、囲んでまとめて選ぶ",
          "2D展開図で行います",
          "そこで手前に見えている紙の点が選ばれます",
        ],
      },
      {
        name: "画面の見かたへの書き添え",
        chapterId: "workspace",
        phrases: [
          "紙の点や線を選んだり、線を引いたりもできます",
          "3Dの紙にある点も押して選べます",
        ],
      },
    ] as const;

    expect(changes).toHaveLength(5);
    for (const change of changes) {
      const chapter = HELP_CHAPTERS.find((entry) => entry.id === change.chapterId);
      expect(chapter, change.name).toBeDefined();
      const text = helpChapterSearchText(chapter!);
      for (const phrase of change.phrases) {
        expect(text, `${change.name}: ${phrase}`).toContain(phrase);
      }
    }
  });

  it("3D立体表示だけで山折り・谷折りを指定して折れることを既存章で説明する", () => {
    const changes = [
      {
        name: "3D左下の札の文言と押す順番",
        chapterId: "fold",
        phrases: [
          "3D立体表示の左下に「この折り線で折る」という札が出ます",
          "向き：手前へ折る(谷)",
          "向き：向こうへ折る(山)",
          "動かす側：こちら側 / 反対側",
          "この折り線を捨てます",
          "3D立体表示だけで折り終える",
          "今選んでいる方が濃い色になる",
          "動く側は3Dで黄色く光ります",
        ],
      },
      {
        name: "8種類すべてを3Dだけで折り終えられる",
        chapterId: "fold",
        phrases: [
          "合わせ方を選んだあとは下部パネルに触れずに3D立体表示だけで折り終えられます",
          "8種類のどれでも、点や線の指定は3D立体表示だけで済みます",
        ],
      },
      {
        name: "札と下部パネルが同じ内容で連動する",
        chapterId: "fold",
        phrases: [
          "下部パネルにあるものと同じ言葉・同じ並びです",
          "もう一方の表示も同じ状態に変わる",
          "札に無いのは「対象の層」だけで",
        ],
      },
      {
        name: "選べるものの上ではカーソルが変わり視点が回らない",
        chapterId: "fold",
        phrases: [
          "カーソルが指の形に変わります",
          "押したあとに手が少し動いても選び直しになりません",
          "選べるものが無い場所をドラッグしたときは、今までどおり視点が回ります",
        ],
      },
      {
        name: "第7章にも札とカーソルのことを書く",
        chapterId: "three-dimensional",
        phrases: [
          "3Dの左下に「この折り線で折る」の札が出ます",
          "カーソルが指の形に変わり、その場所では視点が回りません",
          "押した時点で選択が決まる",
        ],
      },
      {
        name: "第2章の案内にも札を書き添える",
        chapterId: "workspace",
        phrases: [
          "折る向きと動かす側をここで選んで「折る」まで進められ",
          "折り線が決まると左下に札が出て、折る向きもここで決められます",
        ],
      },
    ] as const;

    expect(changes).toHaveLength(6);
    for (const change of changes) {
      const chapter = HELP_CHAPTERS.find((entry) => entry.id === change.chapterId);
      expect(chapter, change.name).toBeDefined();
      const text = helpChapterSearchText(chapter!);
      for (const phrase of change.phrases) {
        expect(text, `${change.name}: ${phrase}`).toContain(phrase);
      }
    }
  });

  it("重なり防止と食い込み検出を現行画面と同じ言葉・既定値で説明する", () => {
    const threeDimensional = HELP_CHAPTERS.find(
      (chapter) => chapter.id === "three-dimensional",
    );
    const troubleshooting = HELP_CHAPTERS.find(
      (chapter) => chapter.id === "troubleshooting",
    );
    expect(threeDimensional).toBeDefined();
    expect(troubleshooting).toBeDefined();

    const threeDimensionalText = helpChapterSearchText(threeDimensional!);
    const troubleshootingText = helpChapterSearchText(troubleshooting!);
    const allText = `${threeDimensionalText}\n${troubleshootingText}`;

    for (const screenText of [
      "紙どうしの食い込みを減らすように形を補正します",
      "紙どうしの食い込みを赤い折り目と警告で知らせます。形は変えません",
      "紙が重なって食い込んでいます",
    ]) {
      expect(threeDimensionalText, screenText).toContain(screenText);
      expect(troubleshootingText, screenText).toContain(screenText);
    }

    for (const explanation of [
      "「重なり防止」は既定でオフ、「食い込み検出」は既定でオンです",
      "既定では、指定した角度のとおりに折れます",
      "食い込み検出が勝手に形を変えることはありません",
      "形は変わらず、操作も止まりません",
      "形を直してほしいときは「重なり防止」を自分でオンにします",
      "実際の形が指定した角度から変わることがあります",
    ]) {
      expect(threeDimensionalText, explanation).toContain(explanation);
    }

    expect(allText).not.toContain("どちらも既定はオン");
    expect(allText).not.toContain("どちらもオンとして扱われます");
  });

  it("紙の上の場所と完成形の場所を画面と同じ言葉で調整・復元できると説明する", () => {
    const proposal = HELP_CHAPTERS.find((chapter) => chapter.id === "proposal");
    expect(proposal).toBeDefined();
    const text = helpChapterSearchText(proposal!);

    for (const screenText of [
      "紙の上の場所も調整",
      "紙の上の場所を調整",
      "丸い印をつまんで、紙の上でその先端を作りたい場所へ動かしてください。",
      "この場所で作り直す",
      "この候補の場所に戻す",
      "候補へ戻る",
      "完成形と紙の上で場所が違う先が1か所あります。",
      "完成形で動かした場所を使います。",
      "紙の上で動かした場所を使います。",
      "紙の上の場所に戻す",
      "完成形の場所に戻す",
      "完成形の場所を取り消す",
      "元に戻す",
      "やり直す",
    ]) {
      expect(text, screenText).toContain(screenText);
    }

    expect(text).toContain("先端ごとに最後に動かしたほうが使われます");
    expect(text).toContain("使っていないほうへ戻すには");
    expect(text).not.toContain("優先規則");
    expect(text).not.toContain("葉ごと");
  });

  it("利用者向け文字列に指定された内部用語が0件である", () => {
    const internalTerms = [
      "骨格",
      "木",
      "節点",
      "根",
      "充填",
      "ソルバー",
      "ヤコビアン",
      "hard",
      "soft",
      "warm start",
      "prevent",
      "detect",
      "イテレーション",
    ] as const;
    const violations: string[] = [];

    for (const chapter of HELP_CHAPTERS) {
      const text = userFacingChapterText(chapter).toLowerCase().replace(/[\s-]+/g, " ");
      for (const term of internalTerms) {
        if (text.includes(term.toLowerCase())) {
          violations.push(`第${chapter.number}章「${chapter.title}」: ${term}`);
        }
      }
    }

    expect(violations).toEqual([]);
  });

  it("章本文の数え方と用語の表記を統一する", () => {
    const chapterText = HELP_CHAPTERS.map((chapter) =>
      [
        chapter.title,
        chapter.summary,
        ...chapter.blocks.map((block) =>
          block.type === "screenshot" ? block.caption : helpBlockText(block),
        ),
      ].join(" "),
    ).join(" ");
    const proseText = chapterText
      .replace(/「全て平らに戻す」/g, "")
      .replace(/「全ての層」/g, "");

    for (const oldForm of [
      "一こま",
      "3こま",
      "こまの内容",
      "一つの",
      "一つ目",
      "一つずつ",
      "一色",
      "一手",
      "一段階",
      "二つの",
      "全て",
      "下のパネル",
      "下部のパネル",
      "下の設定パネル",
      "折り筋",
    ]) {
      expect(proseText, oldForm).not.toContain(oldForm);
    }

    expect(chapterText).toContain("1コマ");
    expect(chapterText).toContain("下部パネル");
    expect(chapterText).toContain("すべて");
  });

  it("対称描画の目的・3つの基準・画面での選び方を利用者向けに説明する", () => {
    const crease = HELP_CHAPTERS.find((chapter) => chapter.id === "crease-pattern");
    expect(crease).toBeDefined();
    const text = helpChapterSearchText(crease!);

    expect(text).toContain("手間を半分");
    expect(text).toContain("紙の縦の中心線");
    expect(text).toContain("紙の横の中心線");
    expect(text).toContain("この線を基準にする");
    expect(text).toContain("紫の破線");
    expect(text).toContain("別の作品へ切り替えたときも紙の縦の中心線へ戻ります");
    expect(text).not.toContain("鏡映");
    expect(text).not.toContain("ヤコビアン");

    const angles = HELP_CHAPTERS.find((chapter) => chapter.id === "angles");
    expect(angles).toBeDefined();
    const angleText = helpChapterSearchText(angles!);
    expect(angleText).toContain("展開図から対になる折り目を自動で見つけ");
    expect(angleText).toContain("描画用の基準線とは別の動きです");
  });

  it("現行画面の折りたたみ・色・区画・テーマ・追従を利用者向けに網羅する", () => {
    const allText = HELP_CHAPTERS.map(helpChapterSearchText).join("\n");

    for (const label of [
      "この道具の詳しい操作方法 ▼",
      "展開図の詳しい操作方法 ▼",
      "詳しい3D操作方法 ▼",
      "丸みの詳しい操作方法 ▼",
      "紙の色 ▼",
    ]) {
      expect(allText).toContain(label);
    }
    expect(allText).toContain("初めて使うとき、長い説明はすべて閉じています");
    expect(allText).toContain("閉じたままでも表と裏の現在の色見本");
    expect(allText).toContain("Tabキーで枠を移したときも同じ吹き出し");
    expect(allText).toContain("文字の代わりに▼または▲のアイコン");

    for (const colorControl of [
      "色の面",
      "色相",
      "16進数",
      "取り消し",
      "この色にする",
      "Shiftを押しながら矢印キー",
      "Enterで確定",
      "Escで閉じます",
    ]) {
      expect(allText).toContain(colorControl);
    }

    expect(allText).toContain("展開図と立体表示は50%ずつ、下部パネルは32%");
    expect(allText).toContain("表示の広さを初期に戻す");
    expect(allText).toContain("和紙のような繊維と濃淡");
    expect(allText).toContain("ごく細かな粒");
    expect(allText).toContain("操作方法は共通です");

    expect(allText).toContain("動かしている折り目の角度を最優先");
    // 追従した折り目に専用の色は無い(2Dは選択の橙・ポインターの紫・操作中の水色・
    // 食い込みの赤、3Dは選択の黄色・水色・ポインターのコーラル・食い込みの赤の4役だけ)。
    // 「琥珀色」を期待していた行は、実際の画面に無い色を固定していたので現状の文へ置き換えた。
    expect(allText).toContain("前の希望から譲った折り目には色が付かない");
    expect(allText).not.toContain("琥珀色");
    expect(allText).toContain("現在72.0°");
    expect(allText).toContain("希望どおりの形が見つからない場合も操作は止まりません");

    expect(allText).toContain("初めて使うときの基準は「紙の縦の中心線」");
    expect(allText).toContain("紙の横の中心線");
    expect(allText).toContain("この線を基準にする");
    expect(allText).toContain("紫の破線");

    for (const internalWord of [
      "ヤコビアン",
      "ソルバ",
      "アルゴリズム",
      "データ構造",
      "細かな点の位置",
      "細かな直線の集まり",
    ]) {
      expect(allText).not.toContain(internalWord);
    }
  });

  it("一般的不収束は自動調整を案内し、折り線欠落だけは引き直し・削除を残す", () => {
    const troubleshooting = HELP_CHAPTERS.find(
      (chapter) => chapter.id === "troubleshooting",
    );
    expect(troubleshooting).toBeDefined();
    const text = helpChapterSearchText(troubleshooting!);

    expect(text).toContain("前の角度を自動調整しています。操作は続けられます");
    expect(text).not.toContain("合わなくなった手順を移動・削除");
    expect(text).toContain(
      "「折り線が見つからない」: 展開図で消えた折り線を引き直すか、その手順を削除します",
    );
  });
});
