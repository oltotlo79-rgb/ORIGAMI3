//! 展開図のSVG書き出し(EXP-001)。実寸(mm)で、線の種類ごとに見た目を変える。
//!
//! 内部座標は「紙の長辺=1.0」に正規化され、y軸は上向き。SVGはy軸が下向きなので、
//! 書き出すときに `y_mm = 紙の高さ - y * 長辺` で反転する。

use std::collections::HashMap;

use ori3_model::{Document, EdgeKind};

/// 書き出しの選択肢。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpSvgOptions {
    /// 補助線(下書きの線)も一緒に書き出すか。
    pub include_aux: bool,
}

impl Default for CpSvgOptions {
    fn default() -> Self {
        CpSvgOptions { include_aux: true }
    }
}

/// 線の種類ごとの見た目(色・太さmm・破線の刻みmm)。
/// 山=赤の一点鎖線、谷=青の破線、輪郭=黒の実線、補助=灰の点線。
fn style(kind: EdgeKind) -> (&'static str, f64, &'static str) {
    match kind {
        EdgeKind::Border => ("#222222", 0.5, "none"),
        EdgeKind::Mountain => ("#c8321e", 0.35, "4 1.2 0.8 1.2"),
        EdgeKind::Valley => ("#1e5ac8", 0.35, "3 1.6"),
        EdgeKind::Aux => ("#9a9a9a", 0.2, "0.4 1.2"),
    }
}

/// 小数を短く書く(末尾の0と小数点を落とす)。
pub(crate) fn num(v: f64) -> String {
    let s = format!("{:.4}", if v == 0.0 { 0.0 } else { v });
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() { "0".to_string() } else { s.to_string() }
}

/// 実寸mmのSVGを文字列で返す。viewBoxは紙の実寸(mm)。
///
/// 線は「補助 → 谷 → 山 → 輪郭」の順に重ねて書くので、重なっても輪郭が上に出る。
pub fn cp_svg(doc: &Document, opts: &CpSvgOptions) -> String {
    let (w_mm, h_mm) = (doc.paper.width_mm, doc.paper.height_mm);
    let long = w_mm.max(h_mm);
    let to_mm = |p: [f64; 2]| (p[0] * long, h_mm - p[1] * long);

    // 頂点は辺ごとに何度も引くので、先に一度だけ表にしておく
    let pos: HashMap<_, _> = doc.cp.vertices.iter().map(|v| (v.id, v.pos)).collect();

    let mut out = String::with_capacity(1024);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}mm\" height=\"{h}mm\" \
         viewBox=\"0 0 {w} {h}\">\n",
        w = num(w_mm),
        h = num(h_mm)
    ));
    out.push_str(&format!(
        "  <rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"#ffffff\"/>\n",
        num(w_mm),
        num(h_mm)
    ));

    const ORDER: [EdgeKind; 4] = [
        EdgeKind::Aux,
        EdgeKind::Valley,
        EdgeKind::Mountain,
        EdgeKind::Border,
    ];
    for kind in ORDER {
        if kind == EdgeKind::Aux && !opts.include_aux {
            continue;
        }
        let lines: Vec<_> = doc.cp.edges.iter().filter(|e| e.kind == kind).collect();
        if lines.is_empty() {
            continue;
        }
        let (color, width, dash) = style(kind);
        out.push_str(&format!(
            "  <g stroke=\"{color}\" stroke-width=\"{width}\" stroke-dasharray=\"{dash}\" \
             stroke-linecap=\"round\" fill=\"none\">\n"
        ));
        for e in lines {
            let (Some(a), Some(b)) = (pos.get(&e.v0), pos.get(&e.v1)) else {
                continue; // 参照切れの壊れた線は書かない
            };
            let (x1, y1) = to_mm(*a);
            let (x2, y2) = to_mm(*b);
            out.push_str(&format!(
                "    <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>\n",
                num(x1),
                num(y1),
                num(x2),
                num(y2)
            ));
        }
        out.push_str("  </g>\n");
    }
    out.push_str("</svg>\n");
    out
}

#[cfg(test)]
pub(crate) fn sample_doc() -> Document {
    use ori3_model::{Edge, Paper};
    // 150×100mm(長辺=150mm)。正規化座標は幅1.0×高さ2/3
    let mut doc = Document::new(Paper {
        width_mm: 150.0,
        height_mm: 100.0,
    });
    let mut add = |v0, v1, kind| {
        let id = doc.cp.next_edge_id;
        doc.cp.next_edge_id += 1;
        doc.cp.edges.push(Edge { id, v0, v1, kind });
    };
    add(0, 2, EdgeKind::Mountain);
    add(1, 3, EdgeKind::Valley);
    add(0, 1, EdgeKind::Aux);
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewbox_is_paper_size_in_mm() {
        let svg = cp_svg(&sample_doc(), &CpSvgOptions::default());
        assert!(svg.contains("width=\"150mm\""), "{svg}");
        assert!(svg.contains("height=\"100mm\""), "{svg}");
        assert!(svg.contains("viewBox=\"0 0 150 100\""), "{svg}");
    }

    #[test]
    fn each_edge_kind_has_its_own_style() {
        let svg = cp_svg(&sample_doc(), &CpSvgOptions::default());
        // 輪郭=黒の実線
        assert!(svg.contains("stroke=\"#222222\""), "{svg}");
        assert!(svg.contains("stroke=\"#222222\" stroke-width=\"0.5\" stroke-dasharray=\"none\""));
        // 山=赤の一点鎖線(線・点の繰り返し=刻みが4つ)
        assert!(svg.contains("stroke=\"#c8321e\""), "{svg}");
        assert!(svg.contains("stroke-dasharray=\"4 1.2 0.8 1.2\""), "{svg}");
        // 谷=青の破線(刻みが2つ)
        assert!(svg.contains("stroke=\"#1e5ac8\""), "{svg}");
        assert!(svg.contains("stroke-dasharray=\"3 1.6\""), "{svg}");
    }

    #[test]
    fn aux_lines_can_be_left_out() {
        let with = cp_svg(&sample_doc(), &CpSvgOptions { include_aux: true });
        let without = cp_svg(&sample_doc(), &CpSvgOptions { include_aux: false });
        assert!(with.contains("#9a9a9a"), "{with}");
        assert!(!without.contains("#9a9a9a"), "{without}");
        // 補助線1本ぶんだけ<line>が減る
        assert_eq!(with.matches("<line").count(), 7); // 輪郭4+山+谷+補助
        assert_eq!(without.matches("<line").count(), 6);
    }

    #[test]
    fn y_axis_is_flipped_and_coordinates_are_mm() {
        let svg = cp_svg(&sample_doc(), &CpSvgOptions::default());
        // 内部の(0,0)は紙の左下 → SVGでは左下 = (0mm, 100mm)、
        // 内部の(1, 2/3)は右上 → (150mm, 0mm)
        assert!(svg.contains("x1=\"0\" y1=\"100\" x2=\"150\" y2=\"100\""), "{svg}");
        assert!(svg.contains("x1=\"0\" y1=\"100\" x2=\"150\" y2=\"0\""), "{svg}");
    }

    #[test]
    fn numbers_are_written_short() {
        assert_eq!(num(150.0), "150");
        assert_eq!(num(0.0), "0");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(1.0 / 3.0), "0.3333");
    }
}
