//! アプリ内ヘルプと同じJSONから作る、A4縦の取扱説明書。
//!
//! 本文はSVGでページ組版し、図解だけは印刷時にも読みやすい解像度へ`resvg`で
//! 描き起こしてから配置する。最後に折り図PDFと共通の変換器で複数ページPDFへまとめる。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path};
use std::sync::Arc;

use miniz_oxide::deflate::compress_to_vec_zlib;
use pdf_writer::types::{ActionType, AnnotationType, PageMode};
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref, TextStr};
use resvg::{tiny_skia, usvg};
use serde::Deserialize;

use crate::diagram::{FONT, esc};
use crate::pdf::{PdfPage, RasterPlacement};

const A4_W: f64 = 210.0;
const A4_H: f64 = 297.0;
const LEFT: f64 = 18.0;
const RIGHT: f64 = 192.0;
const CONTENT_W: f64 = RIGHT - LEFT;
const CONTENT_TOP: f64 = 25.0;
const CONTENT_BOTTOM: f64 = 279.0;
const TOC_ITEMS_PER_PAGE: usize = 17;
const TOC_FIRST_ITEM_Y: f64 = 61.0;
const TOC_ITEM_STEP_Y: f64 = 12.5;
const DIAGRAM_LONG_SIDE_PX: u32 = 1800;
const MISSING_CSS_VARIABLE_COLOR: &str = "#27213d";
const TROUBLESHOOTING_FIGURE_TITLE: &str = "警告時に確認する表示と操作";
const TROUBLESHOOTING_FIGURE_ALT: &str =
    "実画面の「警告 1」、詳しい警告文、「元に戻す」「やり直し」ボタン";
const COMPACT_OPERATION_HELP_CAPTION: &str = "各区画を狭くした画面。幅が足りない区画では、詳しい説明ボタンが▼だけになり、要点1行は残ります。ここでは展開図の案内が▼だけになり、3Dの「詳しい3D操作方法 ▼」には「モードの説明とマウス操作の割り当てを開きます」という吹き出しが出ています。";

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
    #[serde(rename = "screenshot")]
    Screenshot { image: String, caption: String },
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
    navigation: Vec<ManualNavigation>,
}

struct ManualNavigation {
    toc_page_index: usize,
    target_page_index: usize,
    link_rect: ManualRect,
    title: String,
}

#[derive(Clone, Copy)]
struct ManualRect {
    left_mm: f64,
    top_mm: f64,
    right_mm: f64,
    bottom_mm: f64,
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
            "  <text x=\"{LEFT}\" y=\"31\" font-family=\"{FONT}\" font-size=\"4.2\" font-weight=\"700\" fill=\"#27213d\">{}</text>\n\
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

    fn screenshot(&mut self, filename: &str, caption: Option<&str>, screenshot: &RasterScreenshot) {
        let image_height = 96.0;
        let caption_lines = caption
            .map(|text| wrap_text(text, CONTENT_W - 8.0, 2.95))
            .unwrap_or_default();
        let height = image_height + 14.0 + caption_lines.len() as f64 * 4.15;
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
        let mut caption_y = y0 + 7.0 + image_height + 4.3;
        for line in caption_lines {
            self.text(LEFT + 4.0, caption_y, 2.95, "400", "#655c73", &line);
            caption_y += 4.15;
        }
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
    let pdf = manual_pages_pdf(&pages)?;
    Ok((pdf, pages.stats))
}

/// 取扱説明書のページをPDFへ束ね、目次リンクと同じ章単位のしおりを付ける。
fn manual_pages_pdf(manual: &ManualPages) -> Result<Vec<u8>, String> {
    let pages: Vec<PdfPage<'_>> = manual
        .pages
        .iter()
        .map(|page| PdfPage {
            svg: &page.svg,
            images: &page.images,
        })
        .collect();
    let mut options = svg2pdf::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    if manual_japanese_font_missing(&options.fontdb) {
        eprintln!(
            "注意: 日本語を出せる書体が見つかりませんでした。\
             取扱説明書の文字が出ないことがあります"
        );
    }
    let conversion = svg2pdf::ConversionOptions {
        // 文字を輪郭線へ変換すると、日本語本文の同じ字形がページごとに大量の
        // パスとして重複する。使用字形だけを字体へ埋め込み、検索・コピーも保つ。
        embed_text: true,
        ..Default::default()
    };

    let mut alloc = Ref::new(1);
    let catalog_id = alloc.bump();
    let page_tree_id = alloc.bump();
    let outline_id = alloc.bump();
    let outline_item_ids: Vec<_> = manual.navigation.iter().map(|_| alloc.bump()).collect();
    let link_ids: Vec<_> = manual.navigation.iter().map(|_| alloc.bump()).collect();

    // 先に全ページを図形へ直し、リンクとしおりが参照できるページIDを確定する。
    let mut parts = Vec::with_capacity(pages.len());
    for page in &pages {
        validate_manual_raster_images(page.images)?;
        let tree = svg2pdf::usvg::Tree::from_str(page.svg, &options)
            .map_err(|error| format!("取扱説明書を組み立てられませんでした: {error}"))?;
        let (chunk, svg_ref) = svg2pdf::to_chunk(&tree, conversion)
            .map_err(|error| format!("取扱説明書をPDFに直せませんでした: {error}"))?;
        let mut reference_map = HashMap::new();
        let chunk = chunk.renumber(|old| *reference_map.entry(old).or_insert_with(|| alloc.bump()));
        let svg_id = *reference_map
            .get(&svg_ref)
            .ok_or_else(|| "取扱説明書の中身が見つかりませんでした".to_string())?;
        let image_ids: Vec<_> = page.images.iter().map(|_| alloc.bump()).collect();
        parts.push((
            chunk,
            alloc.bump(),
            alloc.bump(),
            svg_id,
            image_ids,
            page.images,
        ));
    }
    let page_ids: Vec<_> = parts.iter().map(|part| part.1).collect();

    let mut pdf = Pdf::new();
    {
        let mut catalog = pdf.catalog(catalog_id);
        catalog.pages(page_tree_id);
        if !outline_item_ids.is_empty() {
            catalog.outlines(outline_id);
            catalog.page_mode(PageMode::UseOutlines);
        }
    }
    pdf.pages(page_tree_id)
        .kids(page_ids.iter().copied())
        .count(page_ids.len() as i32);

    if let (Some(first), Some(last)) = (outline_item_ids.first(), outline_item_ids.last()) {
        pdf.outline(outline_id)
            .first(*first)
            .last(*last)
            .count(outline_item_ids.len() as i32);
    }
    for (index, (entry, item_id)) in manual.navigation.iter().zip(&outline_item_ids).enumerate() {
        let target_page_id = page_ids[entry.target_page_index];
        let mut item = pdf.outline_item(*item_id);
        item.title(TextStr(&entry.title));
        item.parent(outline_id);
        if index > 0 {
            item.prev(outline_item_ids[index - 1]);
        }
        if index + 1 < outline_item_ids.len() {
            item.next(outline_item_ids[index + 1]);
        }
        item.dest().page(target_page_id).fit();
        item.finish();
    }

    for (index, (entry, link_id)) in manual.navigation.iter().zip(&link_ids).enumerate() {
        let toc_page_id = page_ids[entry.toc_page_index];
        let target_page_id = page_ids[entry.target_page_index];
        let rect = entry.link_rect.to_pdf_rect();
        let link_name = format!("manual-toc-link-{index}");
        let mut annotation = pdf.annotation(*link_id);
        annotation.subtype(AnnotationType::Link);
        annotation.rect(rect);
        annotation.page(toc_page_id);
        annotation.name(TextStr(&link_name));
        annotation.contents(TextStr(&entry.title));
        annotation.border(0.0, 0.0, 0.0, None);
        {
            let mut action = annotation.action();
            action.action_type(ActionType::GoTo);
            action.destination().page(target_page_id).fit();
        }
        annotation.finish();
    }

    let svg_name = Name(b"S1");
    for (page_index, (chunk, page_id, content_id, svg_id, image_ids, images)) in
        parts.into_iter().enumerate()
    {
        let mut page = pdf.page(page_id);
        page.media_box(Rect::new(0.0, 0.0, manual_pt(A4_W), manual_pt(A4_H)));
        page.parent(page_tree_id);
        page.contents(content_id);
        let annotations: Vec<_> = manual
            .navigation
            .iter()
            .zip(&link_ids)
            .filter_map(|(entry, id)| (entry.toc_page_index == page_index).then_some(*id))
            .collect();
        if !annotations.is_empty() {
            page.annotations(annotations);
        }
        let mut resources = page.resources();
        let mut x_objects = resources.x_objects();
        x_objects.pair(svg_name, svg_id);
        for (index, image_id) in image_ids.iter().enumerate() {
            let image_name = format!("Im{}", index + 1);
            x_objects.pair(Name(image_name.as_bytes()), *image_id);
        }
        x_objects.finish();
        resources.finish();
        page.finish();

        let mut content = Content::new();
        content
            .save_state()
            .transform([manual_pt(A4_W), 0.0, 0.0, manual_pt(A4_H), 0.0, 0.0])
            .x_object(svg_name)
            .restore_state();
        for (index, image) in images.iter().enumerate() {
            let image_name = format!("Im{}", index + 1);
            let bottom_mm = A4_H - image.y_mm - image.height_mm;
            content
                .save_state()
                .transform([
                    manual_pt(image.width_mm),
                    0.0,
                    0.0,
                    manual_pt(image.height_mm),
                    manual_pt(image.x_mm),
                    manual_pt(bottom_mm),
                ])
                .x_object(Name(image_name.as_bytes()))
                .restore_state();
        }
        pdf.stream(content_id, &content.finish());

        for (image_id, image) in image_ids.into_iter().zip(images) {
            let rgb = composite_manual_rgba_over_white(&image.pixels);
            // 画素は変えず、PDFへ格納するときだけ最大圧縮する。
            let compressed = compress_to_vec_zlib(&rgb, 9);
            let mut x_object = pdf.image_xobject(image_id, &compressed);
            x_object.filter(Filter::FlateDecode);
            x_object.width(image.pixel_width as i32);
            x_object.height(image.pixel_height as i32);
            x_object.color_space().device_rgb();
            x_object.bits_per_component(8);
            x_object.finish();
        }
        pdf.extend(&chunk);
    }
    Ok(pdf.finish())
}

impl ManualRect {
    fn to_pdf_rect(self) -> Rect {
        Rect::new(
            manual_pt(self.left_mm),
            manual_pt(A4_H - self.bottom_mm),
            manual_pt(self.right_mm),
            manual_pt(A4_H - self.top_mm),
        )
    }
}

fn manual_pt(mm: f64) -> f32 {
    (mm * 72.0 / 25.4) as f32
}

fn manual_japanese_font_missing(db: &svg2pdf::usvg::fontdb::Database) -> bool {
    let wanted: Vec<String> = FONT
        .split(',')
        .map(|name| name.trim().to_lowercase())
        .filter(|name| name != "sans-serif")
        .collect();
    !db.faces().any(|face| {
        face.families
            .iter()
            .any(|(name, _)| wanted.contains(&name.to_lowercase()))
    })
}

fn validate_manual_raster_images(images: &[RasterPlacement]) -> Result<(), String> {
    for (index, image) in images.iter().enumerate() {
        let number = index + 1;
        if image.pixel_width == 0 || image.pixel_height == 0 {
            return Err(format!("取扱説明書の画像{number}の大きさが0です"));
        }
        if image.pixel_width > i32::MAX as u32 || image.pixel_height > i32::MAX as u32 {
            return Err(format!("取扱説明書の画像{number}が大きすぎます"));
        }
        let expected = (image.pixel_width as usize)
            .checked_mul(image.pixel_height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| format!("取扱説明書の画像{number}が大きすぎます"))?;
        if image.pixels.len() != expected {
            return Err(format!(
                "取扱説明書の画像{number}の画素数が合いません（必要: {expected}バイト、実際: {}バイト）",
                image.pixels.len()
            ));
        }
        if !image.x_mm.is_finite()
            || !image.y_mm.is_finite()
            || !image.width_mm.is_finite()
            || !image.height_mm.is_finite()
            || image.width_mm <= 0.0
            || image.height_mm <= 0.0
        {
            return Err(format!("取扱説明書の画像{number}の配置が正しくありません"));
        }
    }
    Ok(())
}

fn composite_manual_rgba_over_white(rgba: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for pixel in rgba.as_chunks::<4>().0 {
        let white = 255 - pixel[3];
        rgb.push(pixel[0].saturating_add(white));
        rgb.push(pixel[1].saturating_add(white));
        rgb.push(pixel[2].saturating_add(white));
    }
    rgb
}

fn manual_svg_pages(json: &str, assets_dir: &Path) -> Result<ManualPages, String> {
    let content: ManualContent =
        serde_json::from_str(json).map_err(|e| format!("取扱説明書JSONを読めませんでした: {e}"))?;
    validate_content(&content)?;

    let mut raster_diagrams = HashMap::with_capacity(content.diagrams.len());
    for (id, diagram) in &content.diagrams {
        let raster = if id == "troubleshooting-flow" {
            troubleshooting_screen_diagram(diagram, assets_dir)?
        } else {
            rasterize_diagram(diagram)?
        };
        raster_diagrams.insert(id.clone(), raster);
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
                        layout.screenshot(filename, None, &screenshot);
                    }
                }
                Block::Screenshot { image, caption } => {
                    if let Some(screenshot) = screenshot_data(assets_dir, image)? {
                        let caption = if image == "screen-compact-operation-help.png" {
                            COMPACT_OPERATION_HELP_CAPTION
                        } else {
                            caption
                        };
                        layout.screenshot(image, Some(caption), &screenshot);
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
    let navigation = content
        .chapters
        .iter()
        .zip(&chapter_pages)
        .enumerate()
        .map(|(index, (chapter, page_number))| {
            let item_index = index % TOC_ITEMS_PER_PAGE;
            let center_y = TOC_FIRST_ITEM_Y + item_index as f64 * TOC_ITEM_STEP_Y;
            ManualNavigation {
                toc_page_index: 1 + index / TOC_ITEMS_PER_PAGE,
                target_page_index: page_number - 1,
                link_rect: ManualRect {
                    left_mm: LEFT,
                    top_mm: center_y - TOC_ITEM_STEP_Y / 2.0,
                    right_mm: RIGHT,
                    bottom_mm: center_y + TOC_ITEM_STEP_Y / 2.0,
                },
                title: format!("第{}章 {}", chapter.number, chapter.title),
            }
        })
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
        navigation,
    })
}

fn validate_content(content: &ManualContent) -> Result<(), String> {
    if content.schema_version != 2 {
        return Err(format!(
            "取扱説明書JSONのschemaVersionは2にしてください(指定: {})",
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
                Block::Screenshot { image, caption } => {
                    if image.trim().is_empty() || caption.trim().is_empty() {
                        return Err(format!(
                            "第{}章の画面例には画像名と説明が必要です",
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
    // usvgはCSSカスタムプロパティを解決しないため、そのまま渡すとfillは黒、
    // strokeは線なしになる。共有SVGは変えず、PDF用の一時文字列だけ予備値へ直す。
    let printable_svg = resolve_svg_css_variable_fallbacks(&diagram.svg);
    // marker-end付きの1本のpathへ複数の絶対サブパスを入れると、PDF描画では
    // 最後のサブパスにしか矢じりが出ない。矢印ごとのpathへ正規化してから描く。
    let printable_svg = split_marker_end_subpaths(&printable_svg);
    let tree = usvg::Tree::from_str(&printable_svg, &options)
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

/// `marker-end`付きのpathに絶対座標のサブパスが複数ある場合、各サブパスを
/// 独立したpathへ分ける。ブラウザーとPDF変換器で矢じりの数が変わらない形にする。
fn split_marker_end_subpaths(svg: &str) -> String {
    let mut output = String::with_capacity(svg.len());
    let mut remaining = svg;

    while let Some(relative_start) = remaining.find("<path") {
        output.push_str(&remaining[..relative_start]);
        let path_and_rest = &remaining[relative_start..];
        let Some(relative_end) = path_and_rest.find('>') else {
            output.push_str(path_and_rest);
            return output;
        };
        let element = &path_and_rest[..=relative_end];
        if let Some(split) = split_marker_end_path_element(element) {
            output.push_str(&split);
        } else {
            output.push_str(element);
        }
        remaining = &path_and_rest[relative_end + 1..];
    }

    output.push_str(remaining);
    output
}

fn split_marker_end_path_element(element: &str) -> Option<String> {
    if !element.contains("marker-end=") {
        return None;
    }
    let (value_start, value_end) = quoted_attribute_value_range(element, "d")?;
    let data = &element[value_start..value_end];
    let starts: Vec<_> = data
        .char_indices()
        .filter_map(|(index, character)| (character == 'M').then_some(index))
        .collect();
    if starts.len() < 2 {
        return None;
    }

    let mut split = String::with_capacity(element.len() * starts.len());
    for (index, start) in starts.iter().copied().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(data.len());
        let subpath = data[start..end].trim();
        if subpath.is_empty() {
            return None;
        }
        if index > 0 {
            split.push('\n');
        }
        split.push_str(&element[..value_start]);
        split.push_str(subpath);
        split.push_str(&element[value_end..]);
    }
    Some(split)
}

fn quoted_attribute_value_range(element: &str, name: &str) -> Option<(usize, usize)> {
    let needle = format!("{name}=");
    let mut search_start = 0;
    while let Some(relative) = element[search_start..].find(&needle) {
        let key_start = search_start + relative;
        let valid_boundary =
            key_start > 0 && element.as_bytes()[key_start - 1].is_ascii_whitespace();
        let quote_index = key_start + needle.len();
        let quote = element.as_bytes().get(quote_index).copied()?;
        if valid_boundary && matches!(quote, b'\'' | b'"') {
            let value_start = quote_index + 1;
            let relative_end = element[value_start..].find(char::from(quote))?;
            return Some((value_start, value_start + relative_end));
        }
        search_start = quote_index.saturating_add(1);
    }
    None
}

fn troubleshooting_screen_diagram(
    _diagram: &Diagram,
    assets_dir: &Path,
) -> Result<RasterDiagram, String> {
    let source = required_screen_pixmap(assets_dir, "screen-warning.png")?;
    let detail = troubleshooting_screen_detail(&source)?;
    Ok(RasterDiagram {
        title: TROUBLESHOOTING_FIGURE_TITLE.to_string(),
        alt: TROUBLESHOOTING_FIGURE_ALT.to_string(),
        pixels: Arc::from(detail.data().to_vec()),
        pixel_width: detail.width(),
        pixel_height: detail.height(),
        aspect_ratio: f64::from(detail.width()) / f64::from(detail.height()),
    })
}

fn troubleshooting_screen_detail(source: &tiny_skia::Pixmap) -> Result<tiny_skia::Pixmap, String> {
    let mut detail = tiny_skia::Pixmap::new(1800, 600)
        .ok_or_else(|| "警告画面の図解領域を確保できませんでした".to_string())?;
    detail.fill(tiny_skia::Color::from_rgba8(245, 242, 255, 255));
    // 実画面から、切れ目のない操作ボタン・警告札・警告欄だけを取り出す。
    draw_relative_screen_crop(
        &mut detail,
        source,
        [850.0 / 2560.0, 0.0, 410.0 / 2560.0, 105.0 / 1720.0],
        [50, 5, 900, 230],
        "元に戻す・やり直しボタン",
    )?;
    // 警告札の座標は、実画面写真の画素位置を固定値で埋め込んでいる(内容を解析して
    // 自動追跡してはいない)。`ViewerOverlayStack`が札の並べ方を変えると、この座標だけ
    // 空白を切り出すようになり、「警告 n」の文字が消える(2026-08-23に実際に発生)。
    // 直したときの実測(1280論理px/2倍密度=2560物理px撮影、`.status-badge`の
    // getBoundingClientRect): x=687,y=68,w=415,h=31(論理px)。ここではその周囲へ
    // 余白を足した physical px [1330,100,920,130] を切り出す(右は視点キューブの
    // 開始位置 x=2280 の手前で必ず止め、キューブを巻き込まない)。
    // 内容追跡(色や文字を画像から検出して札を自動で見つける)への置き換えは、
    // この関数の他の切り出し(操作ボタン・警告文・警告欄下枠)も含め本ファイル全体が
    // 同じ「固定座標を実測して埋め込む」方式で統一されており、ここだけを自動追跡へ
    // 変えると一貫性が崩れ、かつ撮影パイプライン側(scratchpad配下、非コミット)が
    // 要素の実測値を書き出す仕組みを持たない限り実現できないため見送った。
    // レイアウトが再び変わったら、この座標もまた壊れる。直すときは
    // `scratchpad/manual-shots-v046/shot-warning-final.mjs`等でアプリを実際に動かし、
    // `.status-badge`の`getBoundingClientRect()`を測り直すこと。
    draw_relative_screen_crop(
        &mut detail,
        source,
        [
            1330.0 / 2560.0,
            100.0 / 1720.0,
            920.0 / 2560.0,
            130.0 / 1720.0,
        ],
        [1000, 65, 708, 100],
        "警告札",
    )?;
    // 警告欄は、本文の下にある無地の空白帯だけを除き、実画面の四辺をつなぎ直す。
    // 上下を同じ倍率で描くため、文字を縦につぶさず、枠の継ぎ目も無地部分に置く。
    draw_relative_screen_crop(
        &mut detail,
        source,
        [
            1280.0 / 2560.0,
            1218.0 / 1720.0,
            1270.0 / 2560.0,
            132.0 / 1720.0,
        ],
        [50, 315, 1700, 177],
        "警告画面の警告文と上枠",
    )?;
    draw_relative_screen_crop(
        &mut detail,
        source,
        [
            1280.0 / 2560.0,
            1690.0 / 1720.0,
            1270.0 / 2560.0,
            28.0 / 1720.0,
        ],
        [50, 492, 1700, 37],
        "警告画面の警告欄下枠",
    )?;
    Ok(detail)
}

fn timeline_screen_detail(source: &tiny_skia::Pixmap) -> Result<tiny_skia::Pixmap, String> {
    let mut detail = tiny_skia::Pixmap::new(1800, 730)
        .ok_or_else(|| "タイムラインの図解領域を確保できませんでした".to_string())?;
    detail.fill(tiny_skia::Color::from_rgba8(245, 242, 255, 255));
    // 実画面の部品境界に合わせ、タイムラインと前後移動欄を拡大して並べる。
    draw_relative_screen_crop(
        &mut detail,
        source,
        [
            1360.0 / 2560.0,
            1010.0 / 1720.0,
            860.0 / 2560.0,
            145.0 / 1720.0,
        ],
        [50, 10, 1700, 287],
        "タイムライン",
    )?;
    draw_relative_screen_crop(
        &mut detail,
        source,
        [
            45.0 / 2560.0,
            1248.0 / 1720.0,
            320.0 / 2560.0,
            210.0 / 1720.0,
        ],
        [50, 396, 500, 328],
        "選択中の手順欄",
    )?;
    draw_relative_screen_crop(
        &mut detail,
        source,
        [
            735.0 / 2560.0,
            1290.0 / 1720.0,
            470.0 / 2560.0,
            100.0 / 1720.0,
        ],
        [700, 440, 1000, 213],
        "手順の前後移動ボタン",
    )?;
    Ok(detail)
}

fn compact_operation_help_detail(source: &tiny_skia::Pixmap) -> Result<tiny_skia::Pixmap, String> {
    let mut detail = tiny_skia::Pixmap::new(1800, 700)
        .ok_or_else(|| "狭い画面の操作案内領域を確保できませんでした".to_string())?;
    detail.fill(tiny_skia::Color::from_rgba8(245, 242, 255, 255));
    // 狭い画面で変化する2つの操作案内と吹き出しだけを、完全な外枠ごと示す。
    draw_relative_screen_crop(
        &mut detail,
        source,
        [
            235.0 / 2560.0,
            350.0 / 1720.0,
            870.0 / 2560.0,
            130.0 / 1720.0,
        ],
        [50, 10, 1700, 254],
        "狭い展開図の操作案内",
    )?;
    draw_relative_screen_crop(
        &mut detail,
        source,
        [
            1520.0 / 2560.0,
            210.0 / 1720.0,
            710.0 / 2560.0,
            160.0 / 1720.0,
        ],
        [450, 492, 900, 203],
        "狭い3Dの操作案内",
    )?;
    draw_relative_screen_crop(
        &mut detail,
        source,
        [
            1600.0 / 2560.0,
            120.0 / 1720.0,
            850.0 / 2560.0,
            100.0 / 1720.0,
        ],
        [200, 315, 1400, 165],
        "3D操作案内の吹き出し",
    )?;
    Ok(detail)
}

fn screen_without_partial_bottom_row(
    source: &tiny_skia::Pixmap,
    reference_height: u32,
    description: &str,
) -> Result<tiny_skia::Pixmap, String> {
    let height = (reference_height as f32 / 1720.0 * source.height() as f32).round() as u32;
    let rect = tiny_skia::IntRect::from_xywh(0, 0, source.width(), height)
        .ok_or_else(|| format!("{description}の完全な行までを切り出せませんでした"))?;
    source
        .clone_rect(rect)
        .ok_or_else(|| format!("{description}の完全な行までを複製できませんでした"))
}

fn draw_relative_screen_crop(
    destination: &mut tiny_skia::Pixmap,
    source: &tiny_skia::Pixmap,
    relative_crop: [f32; 4],
    target: [u32; 4],
    description: &str,
) -> Result<(), String> {
    let source_width = source.width() as f32;
    let source_height = source.height() as f32;
    let x = (relative_crop[0] * source_width).round() as i32;
    let y = (relative_crop[1] * source_height).round() as i32;
    let width = (relative_crop[2] * source_width).round().max(1.0) as u32;
    let height = (relative_crop[3] * source_height).round().max(1.0) as u32;
    let source_bounds = tiny_skia::IntRect::from_xywh(0, 0, source.width(), source.height())
        .expect("0より大きい画面写真の範囲");
    let crop_rect = tiny_skia::IntRect::from_xywh(x, y, width, height)
        .filter(|rect| source_bounds.contains(rect))
        .ok_or_else(|| format!("{description}の切り出し範囲が画面外です"))?;
    let crop = source
        .clone_rect(crop_rect)
        .ok_or_else(|| format!("{description}を画面から切り出せませんでした"))?;
    let [target_x, target_y, target_width, target_height] = target;
    if target_x + target_width > destination.width()
        || target_y + target_height > destination.height()
    {
        return Err(format!("{description}の配置先が図解領域外です"));
    }
    let paint = tiny_skia::PixmapPaint {
        quality: tiny_skia::FilterQuality::Bicubic,
        ..Default::default()
    };
    let transform = tiny_skia::Transform::from_row(
        target_width as f32 / crop.width() as f32,
        0.0,
        0.0,
        target_height as f32 / crop.height() as f32,
        target_x as f32,
        target_y as f32,
    );
    destination.draw_pixmap(0, 0, crop.as_ref(), &paint, transform, None);
    Ok(())
}

fn required_screen_pixmap(assets_dir: &Path, filename: &str) -> Result<tiny_skia::Pixmap, String> {
    let full_path = assets_dir.join(filename);
    let bytes = fs::read(&full_path).map_err(|error| {
        format!(
            "実画面に基づく図解に必要な画面写真「{}」を読めませんでした: {error}",
            full_path.display()
        )
    })?;
    decode_screen_png(&full_path, &bytes)
}

fn decode_screen_png(full_path: &Path, bytes: &[u8]) -> Result<tiny_skia::Pixmap, String> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(format!(
            "画面写真「{}」はPNGではありません",
            full_path.display()
        ));
    }
    tiny_skia::Pixmap::decode_png(bytes).map_err(|error| {
        format!(
            "画面写真「{}」を展開できませんでした: {error}",
            full_path.display()
        )
    })
}

fn resolve_svg_css_variable_fallbacks(svg: &str) -> String {
    let mut resolved = String::with_capacity(svg.len());
    let mut remaining = svg;

    while let Some(var_start) = remaining.find("var(") {
        resolved.push_str(&remaining[..var_start]);
        let arguments_start = var_start + "var(".len();
        let arguments_and_rest = &remaining[arguments_start..];
        let Some(arguments_end) = find_css_function_end(arguments_and_rest) else {
            // 壊れた式は勝手に切り詰めず、そのままSVG解析側へ渡す。
            resolved.push_str(&remaining[var_start..]);
            return resolved;
        };
        let arguments = &arguments_and_rest[..arguments_end];
        let fallback = find_top_level_css_comma(arguments)
            .map(|comma| arguments[comma + 1..].trim())
            .filter(|fallback| !fallback.is_empty())
            .unwrap_or(MISSING_CSS_VARIABLE_COLOR);
        resolved.push_str(&resolve_svg_css_variable_fallbacks(fallback));
        remaining = &arguments_and_rest[arguments_end + 1..];
    }

    resolved.push_str(remaining);
    resolved
}

fn find_css_function_end(arguments_and_rest: &str) -> Option<usize> {
    let mut nested_parentheses = 0_usize;
    for (index, character) in arguments_and_rest.char_indices() {
        match character {
            '(' => nested_parentheses += 1,
            ')' if nested_parentheses == 0 => return Some(index),
            ')' => nested_parentheses -= 1,
            _ => {}
        }
    }
    None
}

fn find_top_level_css_comma(arguments: &str) -> Option<usize> {
    let mut nested_parentheses = 0_usize;
    for (index, character) in arguments.char_indices() {
        match character {
            '(' => nested_parentheses += 1,
            ')' => nested_parentheses = nested_parentheses.saturating_sub(1),
            ',' if nested_parentheses == 0 => return Some(index),
            _ => {}
        }
    }
    None
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
    // 過去のタイムライン注釈PNGには説明に使わない枠と交差する枠が焼き込まれて
    // いるため、同時に保存した実画面から必要な2領域を毎回作り直す。
    let source_filename = if filename == "figure-timeline-flow.png" {
        "screen-timeline.png"
    } else {
        filename
    };
    let full_path = assets_dir.join(source_filename);
    let bytes = match fs::read(&full_path) {
        Ok(bytes) => bytes,
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound && source_filename != filename =>
        {
            return Err(format!(
                "タイムライン図解に必要な実画面「{}」が見つかりません",
                full_path.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "注意: 画面写真が見つからないため、この画面例を省略します: {}",
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
    let mut pixmap = decode_screen_png(&full_path, &bytes)?;
    if filename == "figure-timeline-flow.png" {
        pixmap = timeline_screen_detail(&pixmap)?;
    } else if filename == "screen-compact-operation-help.png" {
        pixmap = compact_operation_help_detail(&pixmap)?;
    } else if filename == "screen-fold-drag.png" {
        pixmap = screen_without_partial_bottom_row(&pixmap, 1644, filename)?;
    } else if filename == "screen-prevention-settings.png" {
        pixmap = screen_without_partial_bottom_row(&pixmap, 1580, filename)?;
    } else if filename == "figure-angle-controls.png" {
        draw_angle_control_arrow(&mut pixmap)?;
    }
    Ok(Some(RasterScreenshot {
        pixels: Arc::from(pixmap.data().to_vec()),
        pixel_width: pixmap.width(),
        pixel_height: pixmap.height(),
    }))
}

fn draw_angle_control_arrow(pixmap: &mut tiny_skia::Pixmap) -> Result<(), String> {
    let width_scale = pixmap.width() as f32 / 1800.0;
    let height_scale = pixmap.height() as f32 / 700.0;
    let x = 1120.0 * width_scale;
    let tip_y = 463.0 * height_scale;
    let base_y = 482.0 * height_scale;
    let shaft_y = 520.0 * height_scale;

    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(112, 64, 201, 255);
    paint.anti_alias = true;
    let stroke = tiny_skia::Stroke {
        width: (5.0 * width_scale.min(height_scale)).max(1.0),
        line_cap: tiny_skia::LineCap::Round,
        ..Default::default()
    };
    let mut shaft = tiny_skia::PathBuilder::new();
    shaft.move_to(x, shaft_y);
    shaft.line_to(x, base_y - 2.0 * height_scale);
    let shaft = shaft
        .finish()
        .ok_or_else(|| "角度図解の矢印軸を作れませんでした".to_string())?;
    pixmap.stroke_path(
        &shaft,
        &paint,
        &stroke,
        tiny_skia::Transform::identity(),
        None,
    );

    let half_width = 11.0 * width_scale;
    let mut head = tiny_skia::PathBuilder::new();
    head.move_to(x, tip_y);
    head.line_to(x - half_width, base_y);
    head.line_to(x + half_width, base_y);
    head.close();
    let head = head
        .finish()
        .ok_or_else(|| "角度図解の矢じりを作れませんでした".to_string())?;
    pixmap.fill_path(
        &head,
        &paint,
        tiny_skia::FillRule::Winding,
        tiny_skia::Transform::identity(),
        None,
    );
    Ok(())
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
    let mut y = TOC_FIRST_ITEM_Y;
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
        y += TOC_ITEM_STEP_Y;
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
    use std::collections::BTreeMap;

    fn pixel_at(pixmap: &tiny_skia::Pixmap, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * pixmap.width() + x) * 4) as usize;
        <[u8; 4]>::try_from(&pixmap.data()[offset..offset + 4]).unwrap()
    }

    fn fill_test_rect(
        pixmap: &mut tiny_skia::Pixmap,
        [x, y, width, height]: [u32; 4],
        color: [u8; 4],
    ) {
        let stride = pixmap.width() as usize * 4;
        let data = pixmap.data_mut();
        for row in y..y + height {
            let start = row as usize * stride + x as usize * 4;
            let end = start + width as usize * 4;
            for pixel in data[start..end].as_chunks_mut::<4>().0 {
                pixel.copy_from_slice(&color);
            }
        }
    }

    fn representative_json(image: Option<&str>) -> String {
        representative_json_with_screenshot(image, None)
    }

    fn representative_json_with_screenshot(
        image: Option<&str>,
        screenshot: Option<(&str, &str)>,
    ) -> String {
        let image = image
            .map(|name| format!(r#", "image": "{name}""#))
            .unwrap_or_default();
        let screenshot = screenshot
            .map(|(name, caption)| {
                format!(
                    r#", {{ "type": "screenshot", "image": "{name}", "caption": "{caption}" }}"#
                )
            })
            .unwrap_or_default();
        format!(
            r##"{{
              "schemaVersion": 2,
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
                    {{ "type": "figure", "diagramId": "flow"{image} }}{screenshot}
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

    fn json_with_chapter_count(count: usize) -> String {
        let mut value: serde_json::Value =
            serde_json::from_str(&representative_json(None)).unwrap();
        value["chapters"] = serde_json::Value::Array(
            (0..count)
                .map(|index| {
                    serde_json::json!({
                        "id": format!("chapter-{}", index + 1),
                        "number": index + 1,
                        "title": format!("章{}", index + 1),
                        "summary": "短い説明です。",
                        "blocks": [{ "type": "paragraph", "text": "短い本文です。" }]
                    })
                })
                .collect(),
        );
        serde_json::to_string(&value).unwrap()
    }

    struct ExpectedPdfNavigation {
        toc_page_index: usize,
        target_page_index: usize,
        link_rect: [f32; 4],
        title: String,
    }

    fn expected_pdf_navigation(chapter_count: usize) -> (usize, Vec<ExpectedPdfNavigation>) {
        let toc_page_count = chapter_count.div_ceil(TOC_ITEMS_PER_PAGE);
        let page_count = 1 + toc_page_count + chapter_count;
        let navigation = (0..chapter_count)
            .map(|index| {
                let item_index = index % TOC_ITEMS_PER_PAGE;
                let center_y = TOC_FIRST_ITEM_Y + item_index as f64 * TOC_ITEM_STEP_Y;
                ExpectedPdfNavigation {
                    toc_page_index: 1 + index / TOC_ITEMS_PER_PAGE,
                    target_page_index: 1 + toc_page_count + index,
                    link_rect: [
                        expected_pt(LEFT),
                        expected_pt(A4_H - (center_y + TOC_ITEM_STEP_Y / 2.0)),
                        expected_pt(RIGHT),
                        expected_pt(A4_H - (center_y - TOC_ITEM_STEP_Y / 2.0)),
                    ],
                    title: format!("第{}章 章{}", index + 1, index + 1),
                }
            })
            .collect();
        (page_count, navigation)
    }

    fn expected_pt(mm: f64) -> f32 {
        (mm * 72.0 / 25.4) as f32
    }

    fn last_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .rposition(|window| window == needle)
    }

    fn pdf_objects(pdf: &[u8]) -> BTreeMap<u32, String> {
        let marker = b"startxref\n";
        let marker_start = last_subslice(pdf, marker).expect("startxrefがある");
        let offset_start = marker_start + marker.len();
        let offset_end = pdf[offset_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|relative| offset_start + relative)
            .expect("startxrefの数値が終わる");
        let xref_offset: usize = std::str::from_utf8(&pdf[offset_start..offset_end])
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut lines = pdf[xref_offset..].split(|byte| *byte == b'\n');
        assert_eq!(lines.next().unwrap().trim_ascii_end(), b"xref");
        let header = std::str::from_utf8(lines.next().unwrap()).unwrap().trim();
        let mut header_parts = header.split_whitespace();
        let first_object: u32 = header_parts.next().unwrap().parse().unwrap();
        let object_count: u32 = header_parts.next().unwrap().parse().unwrap();
        assert_eq!(first_object, 0, "単一のxref表を想定する");

        let mut offsets = Vec::new();
        for object_id in first_object..first_object + object_count {
            let line = std::str::from_utf8(lines.next().unwrap()).unwrap();
            let mut fields = line.split_whitespace();
            let offset: usize = fields.next().unwrap().parse().unwrap();
            let _generation = fields.next().unwrap();
            if fields.next() == Some("n") {
                offsets.push((object_id, offset));
            }
        }
        offsets.sort_unstable_by_key(|(_, offset)| *offset);

        let mut objects = BTreeMap::new();
        for (index, (object_id, offset)) in offsets.iter().copied().enumerate() {
            let end = offsets
                .get(index + 1)
                .map(|(_, next)| *next)
                .unwrap_or(xref_offset);
            objects.insert(
                object_id,
                String::from_utf8_lossy(&pdf[offset..end]).into_owned(),
            );
        }
        objects
    }

    fn references_in_array(object: &str, key: &str) -> Vec<u32> {
        let Some(key_start) = object.find(key) else {
            return Vec::new();
        };
        let after_key = &object[key_start + key.len()..];
        let open = after_key.find('[').expect("参照配列が始まる");
        let close = after_key[open + 1..]
            .find(']')
            .map(|relative| open + 1 + relative)
            .expect("参照配列が終わる");
        let tokens: Vec<_> = after_key[open + 1..close].split_whitespace().collect();
        tokens
            .windows(3)
            .filter(|window| window[1] == "0" && window[2] == "R")
            .map(|window| window[0].parse().unwrap())
            .collect()
    }

    fn numbers_in_array(object: &str, key: &str) -> Vec<f32> {
        let key_start = object.find(key).expect("数値配列のキーがある");
        let after_key = &object[key_start + key.len()..];
        let open = after_key.find('[').expect("数値配列が始まる");
        let close = after_key[open + 1..]
            .find(']')
            .map(|relative| open + 1 + relative)
            .expect("数値配列が終わる");
        after_key[open + 1..close]
            .split_whitespace()
            .map(|value| value.parse().unwrap())
            .collect()
    }

    fn fit_destination_after(object: &str, key: &str) -> Option<u32> {
        let key_start = object.find(key)?;
        let after_key = &object[key_start + key.len()..];
        let open = after_key.find('[')?;
        let close = after_key[open + 1..]
            .find(']')
            .map(|relative| open + 1 + relative)?;
        let tokens: Vec<_> = after_key[open + 1..close].split_whitespace().collect();
        if tokens.len() == 4 && tokens[1] == "0" && tokens[2] == "R" && tokens[3] == "/Fit" {
            tokens[0].parse().ok()
        } else {
            None
        }
    }

    fn reference_after(object: &str, key: &str) -> Option<u32> {
        let key_start = object.find(key)?;
        let mut tokens = object[key_start + key.len()..].split_whitespace();
        let reference = tokens.next()?.parse().ok()?;
        (tokens.next() == Some("0") && tokens.next() == Some("R")).then_some(reference)
    }

    fn integer_after(object: &str, key: &str) -> Option<usize> {
        let key_start = object.find(key)?;
        object[key_start + key.len()..]
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    }

    fn annotation_index(object: &str) -> Option<usize> {
        let marker = "manual-toc-link-";
        let start = object.find(marker)? + marker.len();
        let end = object[start..]
            .find(|character: char| !character.is_ascii_digit())
            .map(|relative| start + relative)
            .unwrap_or(object.len());
        object[start..end].parse().ok()
    }

    fn encoded_pdf_title(title: &str) -> String {
        let mut encoded = String::from("<FEFF");
        for value in title.encode_utf16() {
            encoded.push_str(&format!("{value:04X}"));
        }
        encoded.push('>');
        encoded
    }

    fn assert_pdf_navigation(
        pdf: &[u8],
        expected_page_count: usize,
        expected_navigation: &[ExpectedPdfNavigation],
    ) {
        let objects = pdf_objects(pdf);
        let (_, page_tree) = objects
            .iter()
            .find(|(_, object)| object.contains("/Type /Pages"))
            .expect("ページ木がある");
        let page_ids = references_in_array(page_tree, "/Kids");
        assert_eq!(page_ids.len(), expected_page_count);

        let link_objects: Vec<_> = objects
            .iter()
            .filter(|(_, object)| object.contains("/Subtype /Link"))
            .collect();
        assert_eq!(
            link_objects.len(),
            expected_navigation.len(),
            "目次項目ごとにリンクが1件ある"
        );
        let mut link_ids_by_index = vec![None; expected_navigation.len()];
        for (object_id, object) in link_objects {
            let index = annotation_index(object).expect("目次リンクの固有名がある");
            assert!(index < expected_navigation.len());
            assert!(link_ids_by_index[index].replace(*object_id).is_none());
            let expected = &expected_navigation[index];
            assert_eq!(
                reference_after(object, "/P"),
                Some(page_ids[expected.toc_page_index]),
                "リンク{index}の掲載ページ"
            );
            assert_eq!(
                fit_destination_after(object, "/D"),
                Some(page_ids[expected.target_page_index]),
                "リンク{index}の飛び先"
            );
            assert!(object.contains("/S /GoTo"), "リンク{index}はPDF内移動");
            assert!(object.contains("/Border [0 0 0]"), "リンク枠は表示しない");
            let actual_rect = numbers_in_array(object, "/Rect");
            assert_eq!(actual_rect.len(), expected.link_rect.len());
            for (coordinate, expected_coordinate) in actual_rect.iter().zip(expected.link_rect) {
                assert!(
                    (coordinate - expected_coordinate).abs() < 0.001,
                    "リンク{index}の範囲: {actual_rect:?}"
                );
            }
        }
        let link_ids_by_index: Vec<u32> = link_ids_by_index
            .into_iter()
            .map(|id| id.expect("全リンクに連番がある"))
            .collect();
        for (page_index, page_id) in page_ids.iter().enumerate() {
            let actual = references_in_array(&objects[page_id], "/Annots");
            let expected: Vec<_> = expected_navigation
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| {
                    (entry.toc_page_index == page_index).then_some(link_ids_by_index[index])
                })
                .collect();
            assert_eq!(actual, expected, "PDFページ{}のリンク", page_index + 1);
        }

        let (outline_id, outline) = objects
            .iter()
            .find(|(_, object)| object.contains("/Type /Outlines"))
            .expect("しおりの親がある");
        let (_, catalog) = objects
            .iter()
            .find(|(_, object)| object.contains("/Type /Catalog"))
            .expect("PDFカタログがある");
        assert_eq!(reference_after(catalog, "/Outlines"), Some(*outline_id));
        assert!(catalog.contains("/PageMode /UseOutlines"));
        assert_eq!(
            integer_after(outline, "/Count"),
            Some(expected_navigation.len())
        );
        let mut current = reference_after(outline, "/First");
        let mut previous = None;
        for (index, expected) in expected_navigation.iter().enumerate() {
            let item_id = current.expect("全しおり項目をたどれる");
            let item = &objects[&item_id];
            assert_eq!(reference_after(item, "/Parent"), Some(*outline_id));
            assert_eq!(reference_after(item, "/Prev"), previous);
            assert_eq!(
                fit_destination_after(item, "/Dest"),
                Some(page_ids[expected.target_page_index]),
                "しおり{index}の飛び先"
            );
            assert!(
                item.contains(&encoded_pdf_title(&expected.title)),
                "しおり{index}の日本語題名"
            );
            previous = Some(item_id);
            current = reference_after(item, "/Next");
        }
        assert!(current.is_none(), "余分なしおり項目がない");
        assert_eq!(reference_after(outline, "/Last"), previous);
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
        assert!(text.contains("/FontDescriptor"), "本文は字体として埋め込む");
    }

    #[test]
    fn pdf_links_and_outlines_follow_every_chapter_and_target_page() {
        for chapter_count in [1, TOC_ITEMS_PER_PAGE + 1] {
            let manual =
                manual_svg_pages(&json_with_chapter_count(chapter_count), Path::new("unused"))
                    .unwrap();
            let (expected_page_count, expected_navigation) = expected_pdf_navigation(chapter_count);
            assert_eq!(manual.pages.len(), expected_page_count);
            assert_eq!(manual.navigation.len(), chapter_count);
            assert_eq!(manual.stats.toc_item_count, chapter_count);
            let pdf = manual_pages_pdf(&manual).unwrap();
            assert_pdf_navigation(&pdf, expected_page_count, &expected_navigation);
        }
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
    fn continued_page_repeats_the_chapter_title_without_a_continued_suffix() {
        let chapter = Chapter {
            id: "continued".to_string(),
            number: 1,
            title: "ページをまたぐ章題".to_string(),
            summary: "要約".to_string(),
            blocks: Vec::new(),
        };
        let mut layout = BookLayout::new();
        layout.start_chapter(&chapter);
        layout.y = CONTENT_BOTTOM;

        assert!(layout.ensure_space(1.0));

        let chapter_label = "第1章 ページをまたぐ章題";
        let continued_page = &layout.pages[1].svg;
        let continued_header = continued_page
            .lines()
            .find(|line| line.contains("y=\"31\""))
            .expect("続きページの章題がある");
        assert!(continued_header.contains(chapter_label));
        assert!(!continued_header.contains("続き"));
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
    fn standalone_screenshot_block_is_placed_with_its_caption() {
        let assets = std::env::temp_dir().join(format!(
            "ori3-manual-standalone-screenshot-assets-{}",
            std::process::id()
        ));
        fs::create_dir_all(&assets).unwrap();
        let mut pixmap = tiny_skia::Pixmap::new(16, 9).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(112, 64, 201, 255));
        fs::write(assets.join("screen.png"), pixmap.encode_png().unwrap()).unwrap();

        let json = representative_json_with_screenshot(
            None,
            Some(("screen.png", "設定パネルの画面です。")),
        );
        let pages = manual_svg_pages(&json, &assets).unwrap();
        assert!(pages.pages.iter().any(|page| {
            page.svg.contains("data-role=\"screenshot\"")
                && page.svg.contains("設定パネルの画面です。")
        }));
        assert_eq!(
            pages
                .pages
                .iter()
                .map(|page| page.images.len())
                .sum::<usize>(),
            2,
            "図解1枚と独立した画面写真1枚"
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
    fn every_absolute_subpath_with_marker_end_gets_its_own_arrow() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <path d="M10 10H20M30 10H40" stroke="#7040c9" marker-end="url(#arrow)"/>
        </svg>"##;
        let printable = split_marker_end_subpaths(svg);
        assert_eq!(printable.matches("marker-end=").count(), 2);
        assert!(printable.contains("d=\"M10 10H20\""));
        assert!(printable.contains("d=\"M30 10H40\""));
    }

    #[test]
    fn timeline_detail_is_rebuilt_from_the_actual_screen_without_annotation_frames() {
        let mut screen = tiny_skia::Pixmap::new(2560, 1720).unwrap();
        let excluded = [240, 10, 20, 255];
        let timeline = [12, 34, 56, 255];
        let selected_step = [78, 90, 123, 255];
        let move_buttons = [140, 150, 160, 255];
        screen.fill(tiny_skia::Color::from_rgba8(
            excluded[0],
            excluded[1],
            excluded[2],
            excluded[3],
        ));
        fill_test_rect(&mut screen, [1360, 1010, 860, 145], timeline);
        fill_test_rect(&mut screen, [45, 1248, 320, 210], selected_step);
        fill_test_rect(&mut screen, [735, 1290, 470, 100], move_buttons);
        let detail = timeline_screen_detail(&screen).unwrap();

        assert_eq!((detail.width(), detail.height()), (1800, 730));
        assert_eq!(pixel_at(&detail, 0, 0), [245, 242, 255, 255]);
        assert_eq!(pixel_at(&detail, 100, 100), timeline);
        assert_eq!(pixel_at(&detail, 100, 500), selected_step);
        assert_eq!(pixel_at(&detail, 900, 500), move_buttons);
        assert!(!detail.data().as_chunks::<4>().0.iter().any(|pixel| pixel == &excluded));
    }

    #[test]
    fn compact_help_detail_keeps_only_complete_controls_from_the_actual_screen() {
        let mut screen = tiny_skia::Pixmap::new(2560, 1720).unwrap();
        let excluded = [240, 10, 20, 255];
        let crease_help = [12, 34, 56, 255];
        let three_dimensional_help = [78, 90, 123, 255];
        let tooltip = [140, 150, 160, 255];
        screen.fill(tiny_skia::Color::from_rgba8(
            excluded[0],
            excluded[1],
            excluded[2],
            excluded[3],
        ));
        fill_test_rect(&mut screen, [235, 350, 870, 130], crease_help);
        fill_test_rect(&mut screen, [1520, 210, 710, 160], three_dimensional_help);
        fill_test_rect(&mut screen, [1600, 120, 850, 100], tooltip);
        let detail = compact_operation_help_detail(&screen).unwrap();

        assert_eq!((detail.width(), detail.height()), (1800, 700));
        assert_eq!(pixel_at(&detail, 100, 100), crease_help);
        assert_eq!(pixel_at(&detail, 900, 600), three_dimensional_help);
        assert_eq!(pixel_at(&detail, 900, 400), tooltip);
        assert!(!detail.data().as_chunks::<4>().0.iter().any(|pixel| pixel == &excluded));
    }

    #[test]
    fn full_screen_detail_stops_before_a_partial_bottom_row() {
        let mut screen = tiny_skia::Pixmap::new(2560, 1720).unwrap();
        let retained = [12, 34, 56, 255];
        let excluded = [240, 10, 20, 255];
        screen.fill(tiny_skia::Color::from_rgba8(
            retained[0],
            retained[1],
            retained[2],
            retained[3],
        ));
        fill_test_rect(&mut screen, [0, 1644, 2560, 76], excluded);

        let detail = screen_without_partial_bottom_row(&screen, 1644, "test").unwrap();

        assert_eq!((detail.width(), detail.height()), (2560, 1644));
        assert_eq!(pixel_at(&detail, 1280, 1643), retained);
        assert!(!detail.data().as_chunks::<4>().0.iter().any(|pixel| pixel == &excluded));
    }

    #[test]
    fn troubleshooting_detail_uses_only_complete_controls_from_the_actual_screen() {
        let mut screen = tiny_skia::Pixmap::new(2560, 1720).unwrap();
        let excluded = [240, 10, 20, 255];
        let history_buttons = [90, 80, 70, 255];
        let warning_badge = [40, 50, 60, 255];
        let warning_message_top = [120, 130, 140, 255];
        let warning_message_bottom = [150, 160, 170, 255];
        screen.fill(tiny_skia::Color::from_rgba8(
            excluded[0],
            excluded[1],
            excluded[2],
            excluded[3],
        ));
        fill_test_rect(&mut screen, [850, 0, 410, 105], history_buttons);
        fill_test_rect(&mut screen, [1330, 100, 920, 130], warning_badge);
        fill_test_rect(&mut screen, [1280, 1218, 1270, 132], warning_message_top);
        fill_test_rect(&mut screen, [1280, 1690, 1270, 28], warning_message_bottom);
        let detail = troubleshooting_screen_detail(&screen).unwrap();

        assert_eq!((detail.width(), detail.height()), (1800, 600));
        assert_eq!(pixel_at(&detail, 100, 100), history_buttons);
        assert_eq!(pixel_at(&detail, 1350, 110), warning_badge);
        assert_eq!(pixel_at(&detail, 900, 400), warning_message_top);
        assert_eq!(pixel_at(&detail, 900, 510), warning_message_bottom);
        assert!(!detail.data().as_chunks::<4>().0.iter().any(|pixel| pixel == &excluded));
    }

    #[test]
    fn static_manual_figures_do_not_claim_an_unshown_sequence() {
        for text in [
            TROUBLESHOOTING_FIGURE_TITLE,
            TROUBLESHOOTING_FIGURE_ALT,
            COMPACT_OPERATION_HELP_CAPTION,
        ] {
            assert!(!text.contains("流れ"));
            assert!(!text.contains("順に"));
        }
    }

    #[test]
    fn angle_control_annotation_has_a_visible_arrowhead() {
        let mut annotated = tiny_skia::Pixmap::new(1800, 700).unwrap();
        annotated.fill(tiny_skia::Color::WHITE);
        draw_angle_control_arrow(&mut annotated).unwrap();

        assert_eq!(pixel_at(&annotated, 1120, 470), [112, 64, 201, 255]);
        assert_eq!(pixel_at(&annotated, 1120, 510), [112, 64, 201, 255]);
    }

    #[test]
    fn diagram_css_variables_use_fallback_colors_in_rasterized_pixels() {
        let diagram = Diagram {
            id: "css-colors".to_string(),
            title: "色の検査".to_string(),
            alt: "予備色を使う図".to_string(),
            svg: r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 720 280">
                <rect width="720" height="280" fill="var(--background, #f5f2ff)"/>
                <rect x="20" y="20" width="180" height="100" fill="var(--panel, #123456)"/>
                <path d="M250 70H450" fill="none" stroke="var(--arrow, #654321)" stroke-width="20"/>
                <rect x="520" y="20" width="180" height="100" fill="var(--missing)"/>
            </svg>"##
                .to_string(),
        };

        let raster = rasterize_diagram(&diagram).expect("CSS変数を含む図を描ける");
        assert_eq!((raster.pixel_width, raster.pixel_height), (1800, 700));
        let pixel_at = |x: u32, y: u32| {
            let offset = ((y * raster.pixel_width + x) * 4) as usize;
            <[u8; 4]>::try_from(&raster.pixels[offset..offset + 4]).unwrap()
        };
        assert_eq!(pixel_at(250, 175), [0x12, 0x34, 0x56, 0xff]);
        assert_eq!(pixel_at(875, 175), [0x65, 0x43, 0x21, 0xff]);
        assert_eq!(pixel_at(1525, 175), [0x27, 0x21, 0x3d, 0xff]);
    }

    #[test]
    fn japanese_text_wraps_by_character_width() {
        assert_eq!(wrap_text("あいうえお", 9.0, 3.0), ["あいう", "えお"]);
        assert_eq!(wrap_text("ABCDEF", 9.0, 3.0), ["ABCD", "EF"]);
    }
}
