//! アプリ内ヘルプと同じJSONから作る、A4縦の取扱説明書。
//!
//! 本文はSVGでページ組版し、図解だけは印刷時にも読みやすい解像度へ`resvg`で
//! 描き起こしてから配置する。最後に折り図PDFと共通の変換器で複数ページPDFへまとめる。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path};
use std::sync::Arc;

use resvg::{tiny_skia, usvg};
use serde::Deserialize;

use crate::diagram::{FONT, esc};
use crate::pdf::{PdfPage, RasterPlacement, svg_pdf_pages};

const A4_W: f64 = 210.0;
const A4_H: f64 = 297.0;
const LEFT: f64 = 18.0;
const RIGHT: f64 = 192.0;
const CONTENT_W: f64 = RIGHT - LEFT;
const CONTENT_TOP: f64 = 25.0;
const CONTENT_BOTTOM: f64 = 279.0;
const TOC_ITEMS_PER_PAGE: usize = 17;
const DIAGRAM_LONG_SIDE_PX: u32 = 1800;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManualContent {
    schema_version: u32,
    application: Application,
    chapters: Vec<Chapter>,
    diagrams: BTreeMap<String, Diagram>,
}

#[derive(Debug, Deserialize)]
struct Application {
    name: String,
    version: String,
}

#[derive(Debug, Deserialize)]
struct Chapter {
    id: String,
    number: usize,
    title: String,
    summary: String,
    blocks: Vec<Block>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Block {
    #[serde(rename = "paragraph")]
    Paragraph { text: String },
    #[serde(rename = "heading")]
    Heading { text: String },
    #[serde(rename = "bulletList")]
    BulletList {
        #[serde(default)]
        title: Option<String>,
        items: Vec<String>,
    },
    #[serde(rename = "steps")]
    Steps { title: String, items: Vec<Step> },
    #[serde(rename = "callout")]
    Callout {
        tone: CalloutTone,
        title: String,
        text: String,
    },
    #[serde(rename = "figure")]
    Figure {
        #[serde(rename = "diagramId")]
        diagram_id: String,
        #[serde(default)]
        image: Option<String>,
    },
    #[serde(rename = "table")]
    Table {
        #[serde(default)]
        title: Option<String>,
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

#[derive(Debug, Deserialize)]
struct Step {
    title: String,
    description: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CalloutTone {
    Tip,
    Note,
    Warning,
}

#[derive(Debug, Deserialize)]
struct Diagram {
    id: String,
    title: String,
    alt: String,
    svg: String,
}

struct RasterDiagram {
    title: String,
    alt: String,
    pixels: Arc<[u8]>,
    pixel_width: u32,
    pixel_height: u32,
    aspect_ratio: f64,
}

struct RasterScreenshot {
    pixels: Arc<[u8]>,
    pixel_width: u32,
    pixel_height: u32,
}

/// 生成された取扱説明書の検査・表示に使う集計値。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManualPdfStats {
    pub page_count: usize,
    pub toc_item_count: usize,
}

struct ManualPages {
    pages: Vec<Page>,
    stats: ManualPdfStats,
}

struct Page {
    svg: String,
    images: Vec<RasterPlacement>,
}

impl Page {
    fn content(chapter_label: &str) -> Self {
        let mut svg = page_open();
        svg.push_str(&format!(
            "  <text x=\"{LEFT}\" y=\"14.5\" font-family=\"{FONT}\" font-size=\"3.1\" font-weight=\"700\" fill=\"#7040c9\">ORIGAMI3 取扱説明書</text>\n\
               <text x=\"{RIGHT}\" y=\"14.5\" text-anchor=\"end\" font-family=\"{FONT}\" font-size=\"2.9\" fill=\"#655c73\">{}</text>\n\
               <path d=\"M{LEFT} 18H{RIGHT}\" stroke=\"#d8d0e8\" stroke-width=\"0.5\"/>\n",
            esc(chapter_label)
        ));
        Self {
            svg,
            images: Vec::new(),
        }
    }

    fn finish(mut self, page_number: usize, total_pages: usize) -> Self {
        self.svg.push_str(&format!(
            "  <path d=\"M{LEFT} 284H{RIGHT}\" stroke=\"#d8d0e8\" stroke-width=\"0.35\"/>\n\
               <text x=\"{LEFT}\" y=\"290\" font-family=\"{FONT}\" font-size=\"2.7\" fill=\"#756d80\">ORIGAMI3</text>\n\
               <text x=\"{RIGHT}\" y=\"290\" text-anchor=\"end\" font-family=\"{FONT}\" font-size=\"2.7\" fill=\"#756d80\">{page_number} / {total_pages}</text>\n\
             </svg>\n"
        ));
        self
    }
}

struct BookLayout {
    pages: Vec<Page>,
    y: f64,
    chapter_label: String,
}

impl BookLayout {
    fn new() -> Self {
        Self {
            pages: Vec::new(),
            y: CONTENT_TOP,
            chapter_label: String::new(),
        }
    }

    fn start_chapter(&mut self, chapter: &Chapter) {
        self.chapter_label = format!("第{}章 {}", chapter.number, chapter.title);
        self.pages.push(Page::content(&self.chapter_label));
        self.y = CONTENT_TOP;
        self.draw_chapter_lead(chapter);
    }

    fn new_page(&mut self) {
        self.pages.push(Page::content(&self.chapter_label));
        self.y = CONTENT_TOP;
        self.raw(&format!(
            "  <text x=\"{LEFT}\" y=\"31\" font-family=\"{FONT}\" font-size=\"4.2\" font-weight=\"700\" fill=\"#27213d\">{}（続き）</text>\n\
               <path d=\"M{LEFT} 35H{RIGHT}\" stroke=\"#7040c9\" stroke-width=\"0.8\"/>\n",
            esc(&self.chapter_label)
        ));
        self.y = 41.0;
    }

    fn raw(&mut self, fragment: &str) {
        self.pages
            .last_mut()
            .expect("章ページがある")
            .svg
            .push_str(fragment);
    }

    fn ensure_space(&mut self, height: f64) -> bool {
        if self.y + height <= CONTENT_BOTTOM {
            return false;
        }
        self.new_page();
        true
    }

    fn draw_chapter_lead(&mut self, chapter: &Chapter) {
        let title_lines = wrap_text(&chapter.title, CONTENT_W - 28.0, 7.2);
        let summary_lines = wrap_text(&chapter.summary, CONTENT_W - 12.0, 3.55);
        let height = 17.0 + title_lines.len() as f64 * 8.2 + summary_lines.len() as f64 * 5.2;
        let y0 = self.y;
        self.raw(&format!(
            "  <rect x=\"{LEFT}\" y=\"{y0}\" width=\"{CONTENT_W}\" height=\"{height}\" rx=\"5\" fill=\"#f5f2ff\"/>\n\
               <rect x=\"{LEFT}\" y=\"{y0}\" width=\"7\" height=\"{height}\" rx=\"3.5\" fill=\"#7040c9\"/>\n\
               <text x=\"{}\" y=\"{}\" font-family=\"{FONT}\" font-size=\"3.3\" font-weight=\"700\" fill=\"#7040c9\">第{}章</text>\n",
            LEFT + 13.0,
            y0 + 9.0,
            chapter.number
        ));
        let mut y = y0 + 19.0;
        for line in title_lines {
            self.text(LEFT + 13.0, y, 7.2, "700", "#27213d", &line);
            y += 8.2;
        }
        y += 1.0;
        for line in summary_lines {
            self.text(LEFT + 13.0, y, 3.55, "400", "#4a4258", &line);
            y += 5.2;
        }
        self.y = y0 + height + 7.0;
    }

    fn text(&mut self, x: f64, y: f64, size: f64, weight: &str, fill: &str, text: &str) {
        self.raw(&format!(
            "  <text x=\"{x}\" y=\"{y}\" font-family=\"{FONT}\" font-size=\"{size}\" font-weight=\"{weight}\" fill=\"{fill}\">{}</text>\n",
            esc(text)
        ));
    }

    fn paragraph(&mut self, text: &str) {
        let lines = wrap_text(text, CONTENT_W, 3.55);
        self.y += 0.8;
        for line in lines {
            self.ensure_space(5.35);
            self.text(LEFT, self.y + 3.6, 3.55, "400", "#302a38", &line);
            self.y += 5.35;
        }
        self.y += 2.0;
    }

    fn heading(&mut self, text: &str) {
        let lines = wrap_text(text, CONTENT_W - 7.0, 5.0);
        let height = lines.len() as f64 * 6.4 + 6.0;
        self.ensure_space(height);
        self.y += 2.0;
        let y0 = self.y;
        self.raw(&format!(
            "  <rect x=\"{LEFT}\" y=\"{y0}\" width=\"3\" height=\"{}\" rx=\"1.5\" fill=\"#007a70\"/>\n",
            (height - 3.0).max(7.0)
        ));
        for line in lines {
            self.text(LEFT + 7.0, self.y + 5.0, 5.0, "700", "#27213d", &line);
            self.y += 6.4;
        }
        self.y += 4.0;
    }

    fn bullet_list(&mut self, title: Option<&str>, items: &[String]) {
        if let Some(title) = title {
            self.ensure_space(7.0);
            self.text(LEFT, self.y + 4.0, 3.75, "700", "#27213d", title);
            self.y += 6.3;
        }
        for item in items {
            let lines = wrap_text(item, CONTENT_W - 8.0, 3.4);
            for (index, line) in lines.iter().enumerate() {
                self.ensure_space(5.15);
                if index == 0 {
                    self.raw(&format!(
                        "  <circle cx=\"{}\" cy=\"{}\" r=\"1.25\" fill=\"#007a70\"/>\n",
                        LEFT + 2.0,
                        self.y + 2.7
                    ));
                }
                self.text(LEFT + 7.0, self.y + 3.5, 3.4, "400", "#302a38", line);
                self.y += 5.15;
            }
            self.y += 1.0;
        }
        self.y += 1.5;
    }

    fn steps(&mut self, title: &str, items: &[Step]) {
        self.ensure_space(8.0);
        self.text(LEFT, self.y + 4.0, 3.8, "700", "#27213d", title);
        self.y += 7.0;
        for (index, item) in items.iter().enumerate() {
            let title_lines = wrap_text(&item.title, CONTENT_W - 18.0, 3.55);
            let description_lines = wrap_text(&item.description, CONTENT_W - 18.0, 3.25);
            let height =
                7.0 + title_lines.len() as f64 * 4.8 + description_lines.len() as f64 * 4.65;
            self.ensure_space(height + 2.0);
            let y0 = self.y;
            self.raw(&format!(
                "  <rect x=\"{LEFT}\" y=\"{y0}\" width=\"{CONTENT_W}\" height=\"{height}\" rx=\"4\" fill=\"#faf9fd\" stroke=\"#d8d0e8\" stroke-width=\"0.45\"/>\n\
                   <circle cx=\"{}\" cy=\"{}\" r=\"5\" fill=\"#7040c9\"/>\n\
                   <text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-family=\"{FONT}\" font-size=\"3.6\" font-weight=\"700\" fill=\"#ffffff\">{}</text>\n",
                LEFT + 7.5,
                y0 + 7.5,
                LEFT + 7.5,
                y0 + 8.7,
                index + 1
            ));
            let mut y = y0 + 6.0;
            for line in title_lines {
                self.text(LEFT + 16.0, y, 3.55, "700", "#27213d", &line);
                y += 4.8;
            }
            y += 1.0;
            for line in description_lines {
                self.text(LEFT + 16.0, y, 3.25, "400", "#4a4258", &line);
                y += 4.65;
            }
            self.y = y0 + height + 2.3;
        }
        self.y += 1.5;
    }

    fn callout(&mut self, tone: CalloutTone, title: &str, text: &str) {
        let (fill, stroke, label) = match tone {
            CalloutTone::Tip => ("#ddf8f1", "#007a70", "ヒント"),
            CalloutTone::Note => ("#eee7ff", "#7040c9", "メモ"),
            CalloutTone::Warning => ("#fff5c2", "#a26300", "注意"),
        };
        let lines = wrap_text(text, CONTENT_W - 12.0, 3.3);
        let height = 13.0 + lines.len() as f64 * 4.7;
        self.ensure_space(height + 3.0);
        let y0 = self.y;
        self.raw(&format!(
            "  <rect x=\"{LEFT}\" y=\"{y0}\" width=\"{CONTENT_W}\" height=\"{height}\" rx=\"4\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"0.65\"/>\n\
               <rect x=\"{}\" y=\"{}\" width=\"18\" height=\"6.2\" rx=\"3.1\" fill=\"{stroke}\"/>\n\
               <text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-family=\"{FONT}\" font-size=\"2.8\" font-weight=\"700\" fill=\"#ffffff\">{label}</text>\n",
            LEFT + 5.0,
            y0 + 4.0,
            LEFT + 14.0,
            y0 + 8.3
        ));
        self.text(LEFT + 26.0, y0 + 8.3, 3.65, "700", "#27213d", title);
        let mut y = y0 + 15.0;
        for line in lines {
            self.text(LEFT + 6.0, y, 3.3, "400", "#302a38", &line);
            y += 4.7;
        }
        self.y = y0 + height + 4.0;
    }

    fn table(&mut self, title: Option<&str>, columns: &[String], rows: &[Vec<String>]) {
        if columns.is_empty() {
            return;
        }
        if let Some(title) = title {
            self.ensure_space(8.0);
            self.text(LEFT, self.y + 4.0, 3.75, "700", "#27213d", title);
            self.y += 7.0;
        }
        let column_width = CONTENT_W / columns.len() as f64;
        self.draw_table_row(columns, column_width, true);
        for row in rows {
            let cells = normalized_row(row, columns.len());
            let height = table_row_height(&cells, column_width, 3.0);
            if self.ensure_space(height) {
                self.draw_table_row(columns, column_width, true);
            }
            self.draw_table_row(&cells, column_width, false);
        }
        self.y += 4.0;
    }

    fn draw_table_row(&mut self, cells: &[String], column_width: f64, header: bool) {
        let height = table_row_height(cells, column_width, 3.0);
        self.ensure_space(height);
        let y0 = self.y;
        let fill = if header { "#eee7ff" } else { "#ffffff" };
        self.raw(&format!(
            "  <rect x=\"{LEFT}\" y=\"{y0}\" width=\"{CONTENT_W}\" height=\"{height}\" fill=\"{fill}\" stroke=\"#9f93b8\" stroke-width=\"0.45\"/>\n"
        ));
        for index in 1..cells.len() {
            let x = LEFT + index as f64 * column_width;
            self.raw(&format!(
                "  <path d=\"M{x} {y0}V{}\" stroke=\"#9f93b8\" stroke-width=\"0.35\"/>\n",
                y0 + height
            ));
        }
        for (index, cell) in cells.iter().enumerate() {
            let x = LEFT + index as f64 * column_width + 2.0;
            let lines = wrap_text(cell, column_width - 4.0, 3.0);
            let mut y = y0 + 4.5;
            for line in lines {
                self.text(
                    x,
                    y,
                    3.0,
                    if header { "700" } else { "400" },
                    "#302a38",
                    &line,
                );
                y += 4.25;
            }
        }
        self.y += height;
    }

    fn figure(&mut self, diagram: &RasterDiagram) {
        let image_height = (CONTENT_W / diagram.aspect_ratio).clamp(48.0, 74.0);
        let caption_lines = wrap_text(&diagram.alt, CONTENT_W - 8.0, 2.95);
        let height = 10.0 + image_height + caption_lines.len() as f64 * 4.15 + 5.0;
        self.ensure_space(height + 4.0);
        let y0 = self.y;
        self.text(LEFT, y0 + 4.0, 3.65, "700", "#27213d", &diagram.title);
        let image_y = y0 + 7.0;
        self.raw(&format!(
            "  <rect x=\"{LEFT}\" y=\"{image_y}\" width=\"{CONTENT_W}\" height=\"{image_height}\" rx=\"4\" fill=\"#ffffff\" stroke=\"#d8d0e8\" stroke-width=\"0.45\"/>\n\
               <g data-role=\"diagram\"/>\n"
        ));
        let (x_mm, y_mm, width_mm, height_mm) = fit_rect(
            diagram.pixel_width,
            diagram.pixel_height,
            LEFT + 1.5,
            image_y + 1.5,
            CONTENT_W - 3.0,
            image_height - 3.0,
        );
        self.pages
            .last_mut()
            .expect("章ページがある")
            .images
            .push(RasterPlacement {
                pixels: diagram.pixels.clone(),
                pixel_width: diagram.pixel_width,
                pixel_height: diagram.pixel_height,
                x_mm,
                y_mm,
                width_mm,
                height_mm,
            });
        let mut y = image_y + image_height + 4.3;
        for line in caption_lines {
            self.text(LEFT + 4.0, y, 2.95, "400", "#655c73", &line);
            y += 4.15;
        }
        self.y = y0 + height + 3.0;
    }

    fn screenshot(&mut self, filename: &str, screenshot: &RasterScreenshot) {
        let image_height = 96.0;
        let height = image_height + 14.0;
        self.ensure_space(height + 4.0);
        let y0 = self.y;
        self.text(LEFT, y0 + 4.0, 3.65, "700", "#27213d", "画面例");
        self.raw(&format!(
            "  <text x=\"{RIGHT}\" y=\"{}\" text-anchor=\"end\" font-family=\"{FONT}\" font-size=\"2.7\" fill=\"#756d80\">{}</text>\n",
            y0 + 4.0,
            esc(filename)
        ));
        self.raw(&format!(
            "  <rect x=\"{LEFT}\" y=\"{}\" width=\"{CONTENT_W}\" height=\"{image_height}\" rx=\"3\" fill=\"#f7f6f9\" stroke=\"#9f93b8\" stroke-width=\"0.5\"/>\n\
               <g data-role=\"screenshot\"/>\n",
            y0 + 7.0
        ));
        let (x_mm, y_mm, width_mm, height_mm) = fit_rect(
            screenshot.pixel_width,
            screenshot.pixel_height,
            LEFT + 1.5,
            y0 + 8.5,
            CONTENT_W - 3.0,
            image_height - 3.0,
        );
        self.pages
            .last_mut()
            .expect("章ページがある")
            .images
            .push(RasterPlacement {
                pixels: screenshot.pixels.clone(),
                pixel_width: screenshot.pixel_width,
                pixel_height: screenshot.pixel_height,
                x_mm,
                y_mm,
                width_mm,
                height_mm,
            });
        self.y = y0 + height + 4.0;
    }
}

/// JSONから取扱説明書PDFを生成する。`assets_dir`には任意の画面写真PNGを置ける。
pub fn manual_pdf(json: &str, assets_dir: &Path) -> Result<Vec<u8>, String> {
    manual_pdf_with_stats(json, assets_dir).map(|(pdf, _)| pdf)
}

/// PDF本体とページ数・目次件数を同時に返す。
pub fn manual_pdf_with_stats(
    json: &str,
    assets_dir: &Path,
) -> Result<(Vec<u8>, ManualPdfStats), String> {
    let pages = manual_svg_pages(json, assets_dir)?;
    let pdf_pages: Vec<PdfPage<'_>> = pages
        .pages
        .iter()
        .map(|page| PdfPage {
            svg: &page.svg,
            images: &page.images,
        })
        .collect();
    let pdf = svg_pdf_pages(&pdf_pages, "取扱説明書")?;
    Ok((pdf, pages.stats))
}

fn manual_svg_pages(json: &str, assets_dir: &Path) -> Result<ManualPages, String> {
    let content: ManualContent =
        serde_json::from_str(json).map_err(|e| format!("取扱説明書JSONを読めませんでした: {e}"))?;
    validate_content(&content)?;

    let mut raster_diagrams = HashMap::with_capacity(content.diagrams.len());
    for (id, diagram) in &content.diagrams {
        raster_diagrams.insert(id.clone(), rasterize_diagram(diagram)?);
    }

    let mut layout = BookLayout::new();
    let mut relative_starts = Vec::with_capacity(content.chapters.len());
    for chapter in &content.chapters {
        relative_starts.push(layout.pages.len());
        layout.start_chapter(chapter);
        for block in &chapter.blocks {
            match block {
                Block::Paragraph { text } => layout.paragraph(text),
                Block::Heading { text } => layout.heading(text),
                Block::BulletList { title, items } => layout.bullet_list(title.as_deref(), items),
                Block::Steps { title, items } => layout.steps(title, items),
                Block::Callout { tone, title, text } => layout.callout(*tone, title, text),
                Block::Table {
                    title,
                    columns,
                    rows,
                } => layout.table(title.as_deref(), columns, rows),
                Block::Figure { diagram_id, image } => {
                    let diagram = raster_diagrams
                        .get(diagram_id)
                        .ok_or_else(|| format!("図解ID「{diagram_id}」の描画結果がありません"))?;
                    layout.figure(diagram);
                    if let Some(filename) = image
                        && let Some(screenshot) = screenshot_data(assets_dir, filename)?
                    {
                        layout.screenshot(filename, &screenshot);
                    }
                }
            }
        }
    }

    let toc_page_count = content.chapters.len().div_ceil(TOC_ITEMS_PER_PAGE).max(1);
    let total_pages = 1 + toc_page_count + layout.pages.len();
    let chapter_pages: Vec<usize> = relative_starts
        .iter()
        .map(|relative| 2 + toc_page_count + relative)
        .collect();

    let mut pages = Vec::with_capacity(total_pages);
    pages.push(cover_page(&content, total_pages));
    for toc_index in 0..toc_page_count {
        let from = toc_index * TOC_ITEMS_PER_PAGE;
        let to = (from + TOC_ITEMS_PER_PAGE).min(content.chapters.len());
        pages.push(toc_page(
            &content.chapters[from..to],
            &chapter_pages[from..to],
            toc_index,
            toc_page_count,
            total_pages,
        ));
    }
    for (index, page) in layout.pages.into_iter().enumerate() {
        let absolute = 1 + toc_page_count + index + 1;
        pages.push(page.finish(absolute, total_pages));
    }

    Ok(ManualPages {
        pages,
        stats: ManualPdfStats {
            page_count: total_pages,
            toc_item_count: content.chapters.len(),
        },
    })
}

fn validate_content(content: &ManualContent) -> Result<(), String> {
    if content.schema_version != 1 {
        return Err(format!(
            "取扱説明書JSONのschemaVersionは1にしてください(指定: {})",
            content.schema_version
        ));
    }
    if content.application.name.trim().is_empty() || content.application.version.trim().is_empty() {
        return Err("アプリ名とバージョンは空にできません".to_string());
    }
    if content.chapters.is_empty() {
        return Err("取扱説明書には1章以上必要です".to_string());
    }
    let mut chapter_ids = HashSet::new();
    for (index, chapter) in content.chapters.iter().enumerate() {
        if chapter.number != index + 1 {
            return Err(format!(
                "章番号は1から順に並べてください(位置{}の番号: {})",
                index + 1,
                chapter.number
            ));
        }
        if !chapter_ids.insert(&chapter.id) {
            return Err(format!("章ID「{}」が重複しています", chapter.id));
        }
        if chapter.title.trim().is_empty() {
            return Err(format!("第{}章の題名が空です", chapter.number));
        }
        for block in &chapter.blocks {
            match block {
                Block::Figure { diagram_id, .. } => {
                    if !content.diagrams.contains_key(diagram_id) {
                        return Err(format!(
                            "第{}章が存在しない図解ID「{diagram_id}」を参照しています",
                            chapter.number
                        ));
                    }
                }
                Block::Table { columns, rows, .. } => {
                    if columns.is_empty() {
                        return Err(format!("第{}章の表に列がありません", chapter.number));
                    }
                    if rows.iter().any(|row| row.len() != columns.len()) {
                        return Err(format!(
                            "第{}章の表で見出しと行の列数が一致しません",
                            chapter.number
                        ));
                    }
                }
                _ => {}
            }
        }
    }
    for (key, diagram) in &content.diagrams {
        if key != &diagram.id {
            return Err(format!(
                "図解辞書のキー「{key}」と図解ID「{}」が一致しません",
                diagram.id
            ));
        }
        if !diagram.svg.contains("<svg") || !diagram.svg.contains("</svg>") {
            return Err(format!("図解「{key}」が完全なSVGではありません"));
        }
    }
    Ok(())
}

fn rasterize_diagram(diagram: &Diagram) -> Result<RasterDiagram, String> {
    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(&diagram.svg, &options)
        .map_err(|e| format!("図解「{}」のSVGを読めませんでした: {e}", diagram.id))?;
    let size = tree.size();
    let width = size.width();
    let height = size.height();
    if !(width > 0.0 && height > 0.0) {
        return Err(format!("図解「{}」の大きさが不正です", diagram.id));
    }
    let scale = DIAGRAM_LONG_SIDE_PX as f32 / width.max(height);
    let pixel_width = ((width * scale).round() as u32).max(1);
    let pixel_height = ((height * scale).round() as u32).max(1);
    let mut pixmap = tiny_skia::Pixmap::new(pixel_width, pixel_height)
        .ok_or_else(|| format!("図解「{}」の画像領域を確保できませんでした", diagram.id))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(RasterDiagram {
        title: diagram.title.clone(),
        alt: diagram.alt.clone(),
        pixels: Arc::from(pixmap.data().to_vec()),
        pixel_width,
        pixel_height,
        aspect_ratio: f64::from(width / height),
    })
}

fn screenshot_data(assets_dir: &Path, filename: &str) -> Result<Option<RasterScreenshot>, String> {
    let path = Path::new(filename);
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || path.file_name() != Some(OsStr::new(filename))
        || path
            .extension()
            .and_then(OsStr::to_str)
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("png"))
    {
        return Err(format!(
            "画面写真はassets直下のPNGファイル名で指定してください: {filename}"
        ));
    }
    let full_path = assets_dir.join(filename);
    let bytes = match fs::read(&full_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "注意: 画面写真が見つからないため図解だけを載せます: {}",
                full_path.display()
            );
            return Ok(None);
        }
        Err(error) => {
            return Err(format!(
                "画面写真「{}」を読めませんでした: {error}",
                full_path.display()
            ));
        }
    };
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(format!(
            "画面写真「{}」はPNGではありません",
            full_path.display()
        ));
    }
    let pixmap = tiny_skia::Pixmap::decode_png(&bytes).map_err(|error| {
        format!(
            "画面写真「{}」を展開できませんでした: {error}",
            full_path.display()
        )
    })?;
    Ok(Some(RasterScreenshot {
        pixels: Arc::from(pixmap.data().to_vec()),
        pixel_width: pixmap.width(),
        pixel_height: pixmap.height(),
    }))
}

fn cover_page(content: &ManualContent, total_pages: usize) -> Page {
    let mut svg = page_open();
    svg.push_str(
        "  <rect x=\"0\" y=\"0\" width=\"210\" height=\"297\" fill=\"#fbfaff\"/>\n\
           <circle cx=\"176\" cy=\"35\" r=\"64\" fill=\"#eee7ff\"/>\n\
           <circle cx=\"28\" cy=\"273\" r=\"48\" fill=\"#ddf8f1\"/>\n\
           <g transform=\"translate(105 95)\">\n\
             <polygon points=\"-58,8 -8,-42 4,8\" fill=\"#ffd84d\" stroke=\"#7040c9\" stroke-width=\"1.5\"/>\n\
             <polygon points=\"-8,-42 58,0 4,8\" fill=\"#ed5c70\" stroke=\"#7040c9\" stroke-width=\"1.5\"/>\n\
             <polygon points=\"4,8 58,0 20,28\" fill=\"#7040c9\" stroke=\"#7040c9\" stroke-width=\"1.5\"/>\n\
             <polygon points=\"4,8 20,28 -20,38\" fill=\"#007a70\" stroke=\"#7040c9\" stroke-width=\"1.5\"/>\n\
             <path d=\"M-58 8-8-42 4 8 58 0M4 8-20 38\" fill=\"none\" stroke=\"#ffffff\" stroke-width=\"1\" opacity=\".8\"/>\n\
           </g>\n",
    );
    svg.push_str(&format!(
        "  <text x=\"105\" y=\"167\" text-anchor=\"middle\" font-family=\"{FONT}\" font-size=\"10\" font-weight=\"700\" fill=\"#27213d\">{}</text>\n\
           <text x=\"105\" y=\"184\" text-anchor=\"middle\" font-family=\"{FONT}\" font-size=\"14\" font-weight=\"700\" fill=\"#7040c9\">取扱説明書</text>\n\
           <path d=\"M58 195H152\" stroke=\"#007a70\" stroke-width=\"1.2\"/>\n\
           <text x=\"105\" y=\"210\" text-anchor=\"middle\" font-family=\"{FONT}\" font-size=\"4\" fill=\"#4a4258\">おりがみ工房を、最初の1枚から作品の書き出しまで</text>\n\
           <rect x=\"75\" y=\"226\" width=\"60\" height=\"13\" rx=\"6.5\" fill=\"#ffffff\" stroke=\"#9f93b8\" stroke-width=\"0.5\"/>\n\
           <text x=\"105\" y=\"234.5\" text-anchor=\"middle\" font-family=\"{FONT}\" font-size=\"3.4\" font-weight=\"700\" fill=\"#4a4258\">Version {}</text>\n\
           <text x=\"105\" y=\"275\" text-anchor=\"middle\" font-family=\"{FONT}\" font-size=\"2.9\" fill=\"#756d80\">全{}章・{}ページ / アプリ内ヘルプと共通の内容から生成</text>\n\
         </svg>\n",
        esc(&content.application.name),
        esc(&content.application.version),
        content.chapters.len(),
        total_pages
    ));
    Page {
        svg,
        images: Vec::new(),
    }
}

fn toc_page(
    chapters: &[Chapter],
    page_numbers: &[usize],
    toc_index: usize,
    toc_page_count: usize,
    total_pages: usize,
) -> Page {
    let label = if toc_page_count == 1 {
        "目次".to_string()
    } else {
        format!("目次 {}/{}", toc_index + 1, toc_page_count)
    };
    let mut page = Page::content(&label);
    page.svg.push_str(&format!(
        "  <text x=\"{LEFT}\" y=\"38\" font-family=\"{FONT}\" font-size=\"9\" font-weight=\"700\" fill=\"#27213d\">目次</text>\n\
           <text x=\"{LEFT}\" y=\"48\" font-family=\"{FONT}\" font-size=\"3.2\" fill=\"#655c73\">知りたい機能の章から読み始められます。</text>\n"
    ));
    let mut y = 61.0;
    for (chapter, page_number) in chapters.iter().zip(page_numbers) {
        let number = chapter.number;
        page.svg.push_str(&format!(
            "  <g data-role=\"toc-item\">\n\
                 <circle cx=\"{}\" cy=\"{}\" r=\"4.8\" fill=\"#7040c9\"/>\n\
                 <text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-family=\"{FONT}\" font-size=\"3.2\" font-weight=\"700\" fill=\"#ffffff\">{number}</text>\n\
                 <text x=\"{}\" y=\"{}\" font-family=\"{FONT}\" font-size=\"3.75\" font-weight=\"700\" fill=\"#302a38\">{}</text>\n\
                 <path d=\"M{} {}H{}\" stroke=\"#bcb4c8\" stroke-width=\"0.4\" stroke-dasharray=\"1.2 1.8\"/>\n\
                 <text x=\"{RIGHT}\" y=\"{}\" text-anchor=\"end\" font-family=\"{FONT}\" font-size=\"3.6\" font-weight=\"700\" fill=\"#7040c9\">{page_number}</text>\n\
               </g>\n",
            LEFT + 5.0,
            y - 1.2,
            LEFT + 5.0,
            y,
            LEFT + 13.0,
            y,
            esc(&chapter.title),
            LEFT + 13.0,
            y + 3.5,
            RIGHT,
            y
        ));
        y += 12.5;
    }
    page.finish(2 + toc_index, total_pages)
}

fn page_open() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{A4_W}mm\" height=\"{A4_H}mm\" viewBox=\"0 0 {A4_W} {A4_H}\">\n\
           <rect x=\"0\" y=\"0\" width=\"{A4_W}\" height=\"{A4_H}\" fill=\"#ffffff\"/>\n"
    )
}

fn normalized_row(row: &[String], columns: usize) -> Vec<String> {
    (0..columns)
        .map(|index| row.get(index).cloned().unwrap_or_default())
        .collect()
}

fn table_row_height(cells: &[String], column_width: f64, font_size: f64) -> f64 {
    let lines = cells
        .iter()
        .map(|cell| wrap_text(cell, column_width - 4.0, font_size).len())
        .max()
        .unwrap_or(1);
    (lines as f64 * 4.25 + 3.0).max(8.0)
}

fn fit_rect(
    pixel_width: u32,
    pixel_height: u32,
    box_x: f64,
    box_y: f64,
    box_width: f64,
    box_height: f64,
) -> (f64, f64, f64, f64) {
    let image_ratio = pixel_width as f64 / pixel_height.max(1) as f64;
    let box_ratio = box_width / box_height;
    if image_ratio >= box_ratio {
        let height = box_width / image_ratio;
        (
            box_x,
            box_y + (box_height - height) / 2.0,
            box_width,
            height,
        )
    } else {
        let width = box_height * image_ratio;
        (box_x + (box_width - width) / 2.0, box_y, width, box_height)
    }
}

fn wrap_text(text: &str, width_mm: f64, font_size_mm: f64) -> Vec<String> {
    let limit = (width_mm / font_size_mm).max(1.0);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut units = 0.0;
    for character in text.chars() {
        if character == '\n' {
            lines.push(std::mem::take(&mut current));
            units = 0.0;
            continue;
        }
        let next = character_width(character);
        if !current.is_empty() && units + next > limit {
            lines.push(std::mem::take(&mut current));
            units = 0.0;
        }
        current.push(character);
        units += next;
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

fn character_width(character: char) -> f64 {
    if character.is_ascii_whitespace() {
        0.35
    } else if character.is_ascii_punctuation() {
        0.55
    } else if character.is_ascii() {
        0.62
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn representative_json(image: Option<&str>) -> String {
        let image = image
            .map(|name| format!(r#", "image": "{name}""#))
            .unwrap_or_default();
        format!(
            r##"{{
              "schemaVersion": 1,
              "application": {{ "name": "ORIGAMI3", "version": "0.1.0" }},
              "chapters": [
                {{
                  "id": "start", "number": 1, "title": "はじめに", "summary": "最初の章です。",
                  "blocks": [
                    {{ "type": "paragraph", "text": "本文を読みます。" }},
                    {{ "type": "heading", "text": "紙を用意する" }},
                    {{ "type": "bulletList", "title": "確認", "items": ["幅を決める", "色を選ぶ"] }},
                    {{ "type": "steps", "title": "手順", "items": [{{ "title": "新規", "description": "新しい紙を開きます。" }}] }},
                    {{ "type": "callout", "tone": "tip", "title": "覚えておこう", "text": "あとから変更できます。" }},
                    {{ "type": "table", "title": "線", "columns": ["種類", "色"], "rows": [["山折り", "赤"], ["谷折り", "青"]] }}
                  ]
                }},
                {{
                  "id": "draw", "number": 2, "title": "図を使う", "summary": "図解を見ながら進めます。",
                  "blocks": [
                    {{ "type": "figure", "diagramId": "flow"{image} }}
                  ]
                }}
              ],
              "diagrams": {{
                "flow": {{
                  "id": "flow", "title": "操作の流れ", "alt": "左から右へ進む図です。",
                  "svg": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 720 280\"><rect width=\"720\" height=\"280\" fill=\"#f5f2ff\"/><path d=\"M80 140H640\" stroke=\"#7040c9\" stroke-width=\"20\"/></svg>"
                }}
              }}
            }}"##
        )
    }

    #[test]
    fn representative_json_makes_four_page_pdf_and_two_toc_items() {
        let (pdf, stats) = manual_pdf_with_stats(&representative_json(None), Path::new("unused"))
            .expect("取扱説明書PDFを生成できる");
        assert_eq!(
            stats,
            ManualPdfStats {
                page_count: 4,
                toc_item_count: 2
            }
        );
        assert_eq!(&pdf[..5], b"%PDF-");
        let text = String::from_utf8_lossy(&pdf);
        assert_eq!(text.matches("/MediaBox").count(), 4);
        assert!(text.contains("/Count 4"));
    }

    #[test]
    fn toc_has_one_item_per_chapter_and_correct_start_pages() {
        let pages = manual_svg_pages(&representative_json(None), Path::new("unused")).unwrap();
        assert_eq!(
            pages.pages[1].svg.matches("data-role=\"toc-item\"").count(),
            2
        );
        assert!(pages.pages[1].svg.contains(">3</text>"));
        assert!(pages.pages[1].svg.contains(">4</text>"));
    }

    #[test]
    fn missing_optional_screenshot_falls_back_to_diagram() {
        let pages = manual_svg_pages(
            &representative_json(Some("not-yet-added.png")),
            Path::new("missing-assets"),
        )
        .unwrap();
        assert_eq!(pages.stats.page_count, 4);
        assert!(
            pages
                .pages
                .iter()
                .any(|page| page.svg.contains("data-role=\"diagram\""))
        );
        assert!(
            !pages
                .pages
                .iter()
                .any(|page| page.svg.contains("data-role=\"screenshot\""))
        );
    }

    #[test]
    fn existing_png_is_placed_after_the_diagram() {
        let assets =
            std::env::temp_dir().join(format!("ori3-manual-assets-{}", std::process::id()));
        fs::create_dir_all(&assets).unwrap();
        let mut pixmap = tiny_skia::Pixmap::new(16, 9).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(112, 64, 201, 255));
        fs::write(assets.join("screen.png"), pixmap.encode_png().unwrap()).unwrap();

        let pages = manual_svg_pages(&representative_json(Some("screen.png")), &assets).unwrap();
        assert!(
            pages
                .pages
                .iter()
                .any(|page| page.svg.contains("data-role=\"screenshot\""))
        );
        assert_eq!(
            pages
                .pages
                .iter()
                .map(|page| page.images.len())
                .sum::<usize>(),
            2,
            "図解1枚と画面写真1枚"
        );

        fs::remove_file(assets.join("screen.png")).unwrap();
        fs::remove_dir(assets).unwrap();
    }

    #[test]
    fn screenshot_path_must_stay_inside_assets_directory() {
        let error = manual_svg_pages(
            &representative_json(Some("../outside.png")),
            Path::new("assets"),
        )
        .err()
        .expect("assets外は拒否する");
        assert!(error.contains("assets直下"), "{error}");
    }

    #[test]
    fn japanese_text_wraps_by_character_width() {
        assert_eq!(wrap_text("あいうえお", 9.0, 3.0), ["あいう", "えお"]);
        assert_eq!(wrap_text("ABCDEF", 9.0, 3.0), ["ABCD", "EF"]);
    }
}
