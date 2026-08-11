// UI-011: 目次・検索・章表示を備えた詳しいヘルプセンター。
// 内容はhelp/の構造化データだけを描画し、このコンポーネントには説明文を持たせない。

import { useEffect, useRef } from "react";
import { HELP_CHAPTERS, helpChapterSearchText } from "../../help/helpContent";
import { HELP_DIAGRAMS } from "../../help/helpDiagrams";
import type { HelpBlock } from "../../help/helpTypes";
import { useAppStore } from "../../store/appStore";

function normalizeSearch(value: string): string {
  return value.normalize("NFKC").toLocaleLowerCase("ja").trim();
}

function BlockView({ block }: { block: HelpBlock }) {
  switch (block.type) {
    case "paragraph":
      return <p>{block.text}</p>;
    case "heading":
      return <h4>{block.text}</h4>;
    case "bulletList":
      return (
        <section className="help-block-list">
          {block.title && <h4>{block.title}</h4>}
          <ul>
            {block.items.map((item) => <li key={item}>{item}</li>)}
          </ul>
        </section>
      );
    case "steps":
      return (
        <section className="help-block-steps">
          <h4>{block.title}</h4>
          <ol>
            {block.items.map((item) => (
              <li key={`${item.title}-${item.description}`}>
                <strong>{item.title}</strong>
                <span>{item.description}</span>
              </li>
            ))}
          </ol>
        </section>
      );
    case "callout":
      return (
        <aside className={`help-callout ${block.tone}`}>
          <strong>{block.title}</strong>
          <p>{block.text}</p>
        </aside>
      );
    case "figure": {
      const diagram = HELP_DIAGRAMS[block.diagramId];
      return (
        <figure className="help-figure">
          <div
            className="help-figure-image"
            role="img"
            aria-label={diagram.alt}
            dangerouslySetInnerHTML={{ __html: diagram.svg }}
          />
          <figcaption>{diagram.title}</figcaption>
        </figure>
      );
    }
    case "screenshot":
      // docs/manual/assets/ の画面写真はPDF専用。アプリの配布物へは含めない。
      return null;
    case "table":
      return (
        <section className="help-table-wrap">
          {block.title && <h4>{block.title}</h4>}
          <table>
            <thead>
              <tr>{block.columns.map((column) => <th key={column}>{column}</th>)}</tr>
            </thead>
            <tbody>
              {block.rows.map((row, rowIndex) => (
                <tr key={`${rowIndex}-${row.join("-")}`}>
                  {row.map((cell, cellIndex) => (
                    <td key={`${cellIndex}-${cell}`}>{cell}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      );
  }
}

export function HelpCenter() {
  const open = useAppStore((s) => s.helpOpen);
  const chapterId = useAppStore((s) => s.helpChapterId);
  const query = useAppStore((s) => s.helpQuery);
  const openHelp = useAppStore((s) => s.openHelp);
  const closeHelp = useAppStore((s) => s.closeHelp);
  const selectChapter = useAppStore((s) => s.selectHelpChapter);
  const setQuery = useAppStore((s) => s.setHelpQuery);
  const openGuide = useAppStore((s) => s.openGuide);
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "F1") {
        event.preventDefault();
        openHelp();
      } else if (event.key === "Escape" && useAppStore.getState().helpOpen) {
        event.preventDefault();
        closeHelp();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [closeHelp, openHelp]);

  useEffect(() => {
    if (open) searchRef.current?.focus();
  }, [open]);

  if (!open) return null;

  const normalizedQuery = normalizeSearch(query);
  const matches = normalizedQuery
    ? HELP_CHAPTERS.filter((chapter) =>
        normalizeSearch(helpChapterSearchText(chapter)).includes(normalizedQuery),
      )
    : HELP_CHAPTERS;
  const chapter = matches.find((item) => item.id === chapterId) ?? matches[0] ?? null;

  return (
    <div
      className="dialog-backdrop help-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) closeHelp();
      }}
    >
      <div
        className="dialog help-dialog"
        data-floating-ui="help-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="help-center-title"
      >
        <header className="help-header">
          <div>
            <span className="help-kicker">ORIGAMI3 取扱ガイド</span>
            <h2 id="help-center-title">ヘルプセンター</h2>
          </div>
          <button
            type="button"
            className="help-close"
            aria-label="ヘルプセンターを閉じる"
            onClick={closeHelp}
          >
            ×
          </button>
        </header>

        <aside className="help-sidebar">
          <label className="help-search">
            <span>章題・本文を検索</span>
            <span className="help-search-control">
              <span aria-hidden="true">⌕</span>
              <input
                ref={searchRef}
                type="search"
                value={query}
                placeholder="例: 曲線、保存、F1"
                onChange={(event) => setQuery(event.target.value)}
              />
            </span>
          </label>
          <div className="help-result-count" aria-live="polite">
            {normalizedQuery ? `${matches.length}章が見つかりました` : `全${HELP_CHAPTERS.length}章`}
          </div>
          <nav className="help-toc" aria-label="ヘルプの目次">
            {matches.map((item) => (
              <button
                key={item.id}
                type="button"
                className={item.id === chapter?.id ? "current" : ""}
                aria-current={item.id === chapter?.id ? "page" : undefined}
                onClick={() => selectChapter(item.id)}
              >
                <span>{item.number}</span>
                <span>{item.title}</span>
              </button>
            ))}
          </nav>
          <section className="help-guide-entry">
            <strong>手を動かして覚える</strong>
            <p>折る・角度・引く・ふくらますを画面上で練習できます。</p>
            <button
              type="button"
              onClick={() => {
                closeHelp();
                openGuide();
              }}
            >
              基本操作ガイドをもう一度
            </button>
          </section>
        </aside>

        <main className="help-content" tabIndex={-1}>
          {chapter ? (
            <article aria-labelledby={`help-chapter-${chapter.id}`}>
              <div className="help-chapter-heading">
                <span>第{chapter.number}章</span>
                <h3 id={`help-chapter-${chapter.id}`}>{chapter.title}</h3>
                <p>{chapter.summary}</p>
              </div>
              {chapter.blocks.map((block, index) => (
                <BlockView key={`${chapter.id}-${index}`} block={block} />
              ))}
            </article>
          ) : (
            <div className="help-empty" role="status">
              <span aria-hidden="true">◇</span>
              <h3>見つかりませんでした</h3>
              <p>言葉を短くするか、別の折り紙の言葉で探してみてください。</p>
              <button type="button" onClick={() => setQuery("")}>検索を消す</button>
            </div>
          )}
        </main>
      </div>
    </div>
  );
}
