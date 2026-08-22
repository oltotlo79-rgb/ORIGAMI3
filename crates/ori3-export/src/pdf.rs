//! 折り図のページ組版(EXP-003 / EXP-004)。
//!
//! A4(210×297mm)の紙に、1ページあたり2列×3段の6コマを並べる。1ページ目は
//! 表紙で、題と完成の形を大きく載せる。同じ組版からPDF(1つのファイル)と
//! ページごとのSVG(EXP-004)の両方を作る。

use std::sync::Arc;

use miniz_oxide::deflate::compress_to_vec_zlib;
use ori3_cp::extract_faces;
use ori3_model::Document;
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref};

use crate::diagram::{CELL, FONT, cell_body};

/// A4の大きさ(mm)。
const A4_W: f64 = 210.0;
const A4_H: f64 = 297.0;
/// コマの一辺(mm)と、コマとコマの間・上下左右の余白(mm)。
const CELL_MM: f64 = 83.0;
const GAP: f64 = 6.0;
const LEFT: f64 = (A4_W - 2.0 * CELL_MM - GAP) / 2.0;
const TOP: f64 = 22.0;
/// 1ページに載せるコマ数(2列×3段)。
pub const CELLS_PER_PAGE: usize = 6;

/// mmをPDFの単位(1/72インチ)に直す。
fn pt(mm: f64) -> f32 {
    (mm * 72.0 / 25.4) as f32
}

fn page_open() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{A4_W}mm\" height=\"{A4_H}mm\" \
         viewBox=\"0 0 {A4_W} {A4_H}\">\n\
         \x20 <rect x=\"0\" y=\"0\" width=\"{A4_W}\" height=\"{A4_H}\" fill=\"#ffffff\"/>\n"
    )
}

/// コマの中身(100×100の座標で書かれている)を、ページの指定の場所へ縮めて置く。
fn place(body: &str, x: f64, y: f64, size: f64) -> String {
    format!(
        "  <g transform=\"translate({x} {y}) scale({})\">\n{body}  </g>\n",
        size / CELL
    )
}

/// 表紙(題と完成の形)。
fn cover_page(doc: &Document, faces: &[ori3_cp::Face]) -> Result<String, String> {
    let body = cell_body(doc, faces, doc.sequence.len(), false)?;
    let size = 150.0;
    let mut page = page_open();
    page.push_str(&format!(
        "  <text x=\"{}\" y=\"48\" text-anchor=\"middle\" font-family=\"{FONT}\" \
         font-size=\"16\" font-weight=\"bold\" fill=\"#1a1a1a\">折り図</text>\n",
        A4_W / 2.0
    ));
    page.push_str(&place(&body, (A4_W - size) / 2.0, 70.0, size));
    page.push_str(&format!(
        "  <text x=\"{}\" y=\"245\" text-anchor=\"middle\" font-family=\"{FONT}\" \
         font-size=\"5\" fill=\"#333333\">できあがりの形(全{}手順)</text>\n",
        A4_W / 2.0,
        doc.sequence.len()
    ));
    page.push_str(&format!(
        "  <text x=\"{}\" y=\"258\" text-anchor=\"middle\" font-family=\"{FONT}\" \
         font-size=\"4\" fill=\"#555555\">紙の大きさ {}×{}mm</text>\n",
        A4_W / 2.0,
        doc.paper.width_mm,
        doc.paper.height_mm
    ));
    page.push_str("</svg>\n");
    Ok(page)
}

/// 手順のページ(2列×3段)。`from` はこのページの先頭の手順番号(0始まり)。
fn step_page(doc: &Document, faces: &[ori3_cp::Face], from: usize) -> Result<String, String> {
    let mut page = page_open();
    for slot in 0..CELLS_PER_PAGE {
        let index = from + slot;
        if index >= doc.sequence.len() {
            break;
        }
        let body = cell_body(doc, faces, index, true)?;
        let x = LEFT + (slot % 2) as f64 * (CELL_MM + GAP);
        let y = TOP + (slot / 2) as f64 * (CELL_MM + GAP);
        page.push_str(&place(&body, x, y, CELL_MM));
    }
    page.push_str(&format!(
        "  <text x=\"{}\" y=\"288\" text-anchor=\"middle\" font-family=\"{FONT}\" \
         font-size=\"3.6\" fill=\"#777777\">{}ページ</text>\n",
        A4_W / 2.0,
        from / CELLS_PER_PAGE + 2
    ));
    page.push_str("</svg>\n");
    Ok(page)
}

/// 折り図をページごとのSVGにして返す(EXP-004)。先頭は表紙。
pub fn diagram_svg_pages(doc: &Document) -> Result<Vec<String>, String> {
    if doc.sequence.is_empty() {
        return Err("折り手順がまだありません。手順を作ってから折り図を書き出してください".into());
    }
    let faces = extract_faces(&doc.cp);
    let mut pages = vec![cover_page(doc, &faces)?];
    let mut from = 0;
    while from < doc.sequence.len() {
        pages.push(step_page(doc, &faces, from)?);
        from += CELLS_PER_PAGE;
    }
    Ok(pages)
}

/// 折り図の文字に使える書体が機械に入っているか調べる。入っていなければ真。
///
/// [`FONT`] に並べた書体名のどれかが見つかればよい(`sans-serif` は書体名ではなく
/// 「見つからなければ適当なものに任せる」という指定なので数に入れない)。
fn japanese_font_missing(db: &svg2pdf::usvg::fontdb::Database) -> bool {
    let wanted: Vec<String> = FONT
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| s != "sans-serif")
        .collect();
    !db.faces().any(|face| {
        face.families
            .iter()
            .any(|(name, _)| wanted.contains(&name.to_lowercase()))
    })
}

/// 折り図を1つのPDF(A4・複数ページ)にして返す(EXP-003)。
///
/// ページごとのSVGをそれぞれPDFの図形に直し、A4のページへ貼り合わせる。
/// 文字は形(線)に直してから埋めるので、読む人の機械に同じ書体が無くても崩れない。
///
/// 注意: 文字を形に直すには、書き出す側の機械に日本語の出せる書体が必要になる。
/// 1つも見つからないと手順番号や注記が黙って消えてしまうため、そのときは
/// 標準エラー出力に日本語で注意を出す(絵そのものは問題なく書き出せる)。
pub fn diagram_pdf(doc: &Document) -> Result<Vec<u8>, String> {
    let pages = diagram_svg_pages(doc)?;
    svg_pages_pdf(&pages, "折り図")
}

/// SVGページの上へ重ねる、premultiplied RGBA形式のラスター画像。
///
/// 座標はSVGと同じくページ左上が原点で、配置の単位はmm。`pixels` はtiny-skiaの
/// Pixmapが返す並びと同じ、1画素4バイトのpremultiplied RGBAでなければならない。
#[derive(Clone, Debug)]
pub(crate) struct RasterPlacement {
    pub(crate) pixels: Arc<[u8]>,
    pub(crate) pixel_width: u32,
    pub(crate) pixel_height: u32,
    pub(crate) x_mm: f64,
    pub(crate) y_mm: f64,
    pub(crate) width_mm: f64,
    pub(crate) height_mm: f64,
}

/// PDFへ変換する1ページ。SVGの後から`images`を描くため、画像は必ず前面に出る。
#[derive(Clone, Copy, Debug)]
pub(crate) struct PdfPage<'a> {
    pub(crate) svg: &'a str,
    pub(crate) images: &'a [RasterPlacement],
}

/// A4のページごとのSVGを、1つの複数ページPDFに束ねる。
///
/// `context` は、SVGの解析やPDF変換に失敗したときの日本語エラーへ入れる対象名。
/// 文字はパスへ変換するため、生成側の機械に日本語書体が必要になる。
pub(crate) fn svg_pages_pdf(pages: &[String], context: &str) -> Result<Vec<u8>, String> {
    let pages: Vec<_> = pages
        .iter()
        .map(|svg| PdfPage { svg, images: &[] })
        .collect();
    svg_pdf_pages(&pages, context)
}

/// SVGとラスター画像からなるA4ページを、1つの複数ページPDFに束ねる。
pub(crate) fn svg_pdf_pages(pages: &[PdfPage<'_>], context: &str) -> Result<Vec<u8>, String> {
    let mut options = svg2pdf::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    if japanese_font_missing(&options.fontdb) {
        let text_context = if context == "折り図" {
            "折り図の手順番号や説明"
        } else {
            context
        };
        eprintln!(
            "注意: 日本語を出せる書体が見つかりませんでした。\
             {text_context}の文字が出ないことがあります"
        );
    }
    let conv = svg2pdf::ConversionOptions {
        embed_text: false,
        ..Default::default()
    };

    let mut alloc = Ref::new(1);
    let catalog_id = alloc.bump();
    let page_tree_id = alloc.bump();

    // 先に全ページを図形へ直し、番号がぶつからないように振り直しておく
    let mut parts = Vec::with_capacity(pages.len());
    for page in pages {
        validate_raster_images(page.images, context)?;
        let tree = svg2pdf::usvg::Tree::from_str(page.svg, &options)
            .map_err(|e| format!("{context}を組み立てられませんでした: {e}"))?;
        let (chunk, svg_ref) = svg2pdf::to_chunk(&tree, conv)
            .map_err(|e| format!("{context}をPDFに直せませんでした: {e}"))?;
        let mut map = std::collections::HashMap::new();
        let chunk = chunk.renumber(|old| *map.entry(old).or_insert_with(|| alloc.bump()));
        let svg_id = *map
            .get(&svg_ref)
            .ok_or_else(|| format!("{context}の中身が見つかりませんでした"))?;
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

    let mut pdf = Pdf::new();
    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.pages(page_tree_id)
        .kids(parts.iter().map(|p| p.1))
        .count(parts.len() as i32);

    let svg_name = Name(b"S1");
    for (chunk, page_id, content_id, svg_id, image_ids, images) in parts {
        let mut page = pdf.page(page_id);
        page.media_box(Rect::new(0.0, 0.0, pt(A4_W), pt(A4_H)));
        page.parent(page_tree_id);
        page.contents(content_id);
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
            .transform([pt(A4_W), 0.0, 0.0, pt(A4_H), 0.0, 0.0])
            .x_object(svg_name)
            .restore_state();
        for (index, image) in images.iter().enumerate() {
            let image_name = format!("Im{}", index + 1);
            let bottom_mm = A4_H - image.y_mm - image.height_mm;
            content
                .save_state()
                .transform([
                    pt(image.width_mm),
                    0.0,
                    0.0,
                    pt(image.height_mm),
                    pt(image.x_mm),
                    pt(bottom_mm),
                ])
                .x_object(Name(image_name.as_bytes()))
                .restore_state();
        }
        pdf.stream(content_id, &content.finish());

        for (image_id, image) in image_ids.into_iter().zip(images) {
            let rgb = composite_rgba_over_white(&image.pixels);
            let compressed = compress_to_vec_zlib(&rgb, 6);
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

fn validate_raster_images(images: &[RasterPlacement], context: &str) -> Result<(), String> {
    for (index, image) in images.iter().enumerate() {
        let number = index + 1;
        if image.pixel_width == 0 || image.pixel_height == 0 {
            return Err(format!("{context}の画像{number}の大きさが0です"));
        }
        if image.pixel_width > i32::MAX as u32 || image.pixel_height > i32::MAX as u32 {
            return Err(format!("{context}の画像{number}が大きすぎます"));
        }
        let expected = (image.pixel_width as usize)
            .checked_mul(image.pixel_height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| format!("{context}の画像{number}が大きすぎます"))?;
        if image.pixels.len() != expected {
            return Err(format!(
                "{context}の画像{number}の画素数が合いません（必要: {expected}バイト、実際: {}バイト）",
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
            return Err(format!("{context}の画像{number}の配置が正しくありません"));
        }
    }
    Ok(())
}

/// premultiplied RGBAを白背景へ合成し、PDFのDeviceRGBへ渡すRGB列にする。
fn composite_rgba_over_white(rgba: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for pixel in rgba.as_chunks::<4>().0 {
        let white = 255 - pixel[3];
        rgb.push(pixel[0].saturating_add(white));
        rgb.push(pixel[1].saturating_add(white));
        rgb.push(pixel[2].saturating_add(white));
    }
    rgb
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::strip_doc;

    /// 7手順ならA4の2列×3コマで2ページ、表紙を足して3ページになる。
    #[test]
    fn seven_steps_make_a_cover_and_two_pages() {
        let doc = strip_doc(7);
        assert_eq!(doc.sequence.len(), 7);
        let pages = diagram_svg_pages(&doc).expect("ページができるはず");
        assert_eq!(pages.len(), 3, "表紙1+手順2ページ");
        assert!(pages[0].contains("折り図"), "表紙に題がない");
        assert!(pages[0].contains("できあがりの形(全7手順)"), "{}", pages[0]);
        for p in &pages {
            assert!(p.contains("viewBox=\"0 0 210 297\""), "A4ではない: {p}");
        }
        // 1ページ目に6コマ、2ページ目に残り1コマ
        assert_eq!(pages[1].matches("<g transform=\"translate(").count(), 6);
        assert_eq!(pages[2].matches("<g transform=\"translate(").count(), 1);
        // 6コマ目と7コマ目はページをまたぐ
        assert!(pages[1].contains(">6. "), "{}", pages[1]);
        assert!(pages[2].contains(">7. "), "{}", pages[2]);
    }

    /// 同じ組版からA4・3ページのPDFが1つできる。
    #[test]
    fn pdf_has_one_a4_page_per_svg_page() {
        let pdf = diagram_pdf(&strip_doc(7)).expect("PDFができるはず");
        assert!(pdf.len() > 1000, "中身が空に近い: {}バイト", pdf.len());
        assert_eq!(&pdf[0..5], b"%PDF-", "PDFの印がない");
        let text = String::from_utf8_lossy(&pdf);
        assert_eq!(text.matches("/MediaBox").count(), 3, "ページ数が合わない");
        assert!(text.contains("/Count 3"), "ページ数の記載がない");
        // A4(210×297mm)= 595×842ポイント
        assert!(text.contains("595.2756"), "A4の幅ではない");
    }

    /// 1ページに収まる手順数なら表紙+1ページ。
    #[test]
    fn six_steps_fit_on_one_page() {
        assert_eq!(diagram_svg_pages(&strip_doc(6)).unwrap().len(), 2);
        assert_eq!(diagram_svg_pages(&strip_doc(1)).unwrap().len(), 2);
    }

    /// 書体が1つも無ければ「見つからない」と分かる(文字が黙って消えるのを防ぐ)。
    #[test]
    fn a_machine_without_fonts_is_detected() {
        let empty = svg2pdf::usvg::fontdb::Database::new();
        assert!(japanese_font_missing(&empty), "空なら見つからないはず");
        let mut db = svg2pdf::usvg::fontdb::Database::new();
        db.load_system_fonts();
        // 見つかっても見つからなくても書き出し自体は通る(注意を出すだけ)
        let _ = japanese_font_missing(&db);
        assert!(diagram_pdf(&strip_doc(2)).is_ok());
    }

    #[test]
    fn no_steps_is_a_japanese_error() {
        let doc = strip_doc(0);
        for err in [
            diagram_svg_pages(&doc).unwrap_err(),
            diagram_pdf(&doc).unwrap_err(),
        ] {
            assert!(err.contains("折り手順がまだありません"), "err={err}");
        }
    }
}
