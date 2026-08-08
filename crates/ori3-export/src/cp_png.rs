//! 展開図のPNG書き出し(EXP-002)。SVGを作ってから指定の大きさに描き起こす。

use ori3_model::Document;
use resvg::{tiny_skia, usvg};

use crate::cp_svg::{CpSvgOptions, cp_svg};

/// 既定の長辺の点数(px)。
pub const DEFAULT_LONG_SIDE_PX: u32 = 2048;
/// 長辺の点数の上限。大きすぎる指定でメモリを使い切らないよう頭打ちにする。
pub const MAX_LONG_SIDE_PX: u32 = 16384;

/// 展開図をPNG画像(バイト列)にする。`long_side_px` は長いほうの辺の点数。
///
/// 紙が長方形なら短いほうの辺は縦横比に合わせて決まる(必ず1点以上になる)。
pub fn cp_png(doc: &Document, opts: &CpSvgOptions, long_side_px: u32) -> Result<Vec<u8>, String> {
    if long_side_px == 0 {
        return Err("画像の大きさは1以上にしてください".to_string());
    }
    if long_side_px > MAX_LONG_SIDE_PX {
        return Err(format!(
            "画像の大きさは{MAX_LONG_SIDE_PX}までにしてください(指定: {long_side_px})"
        ));
    }
    let svg = cp_svg(doc, opts);
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default())
        .map_err(|e| format!("画像のもとになる図を作れませんでした: {e}"))?;

    let size = tree.size();
    let (sw, sh) = (size.width(), size.height());
    if !(sw > 0.0 && sh > 0.0) {
        return Err("紙の大きさが読み取れませんでした".to_string());
    }
    // 長辺を指定の点数に合わせ、短辺は縦横比のまま(最低1点)
    let scale = long_side_px as f32 / sw.max(sh);
    let w = ((sw * scale).round() as u32).max(1);
    let h = ((sh * scale).round() as u32).max(1);

    let mut pixmap = tiny_skia::Pixmap::new(w, h)
        .ok_or_else(|| "画像を置く場所を確保できませんでした".to_string())?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|e| format!("画像を書き出せませんでした: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cp_svg::sample_doc;

    /// PNGの先頭(署名+IHDR)から画像の幅・高さ(点)を読む。
    fn png_size(bytes: &[u8]) -> (u32, u32) {
        assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n", "PNGの署名がない");
        assert_eq!(&bytes[12..16], b"IHDR", "IHDRがない");
        let n = |i: usize| u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap());
        (n(16), n(20))
    }

    #[test]
    fn png_matches_requested_long_side() {
        let doc = sample_doc(); // 150×100mm
        let png = cp_png(&doc, &CpSvgOptions::default(), 256).expect("書き出せるはず");
        assert!(png.len() > 100, "中身が空に近い: {}バイト", png.len());
        // 長辺(幅)が指定どおり、短辺は縦横比のまま
        assert_eq!(png_size(&png), (256, 171));
    }

    #[test]
    fn default_long_side_is_2048() {
        assert_eq!(DEFAULT_LONG_SIDE_PX, 2048);
        let png = cp_png(
            &sample_doc(),
            &CpSvgOptions::default(),
            DEFAULT_LONG_SIDE_PX,
        )
        .unwrap();
        assert_eq!(png_size(&png).0, 2048);
    }

    #[test]
    fn aux_lines_change_the_picture() {
        let doc = sample_doc();
        let with = cp_png(&doc, &CpSvgOptions { include_aux: true }, 128).unwrap();
        let without = cp_png(&doc, &CpSvgOptions { include_aux: false }, 128).unwrap();
        assert_ne!(with, without, "補助線の有無で絵が変わるはず");
    }

    #[test]
    fn bad_size_is_a_japanese_error() {
        let doc = sample_doc();
        let zero = cp_png(&doc, &CpSvgOptions::default(), 0).unwrap_err();
        assert!(zero.contains("1以上"), "err={zero}");
        let huge = cp_png(&doc, &CpSvgOptions::default(), MAX_LONG_SIDE_PX + 1).unwrap_err();
        assert!(huge.contains("大きさ"), "err={huge}");
    }
}
