//! 折り図の1コマ(EXP-003)。
//!
//! 「その手順を折る直前の形」を上から見た絵にして、これから折る線と、
//! どう動かすかを示す矢印・技法の目印・手順番号・注記を重ねる。
//!
//! 形は [`ori3_layers::flat_state_at`] が返す平坦状態(各面の置き場所と
//! 下→上の重なり順)から作る。折り線([`ori3_model::DriverLine`])は展開図の
//! 座標で持っているので、その線に乗る折り目を今の展開図から引き当て、
//! 折り目に面している面の置き場所を通して畳んだ絵の上へ写す。

use std::collections::HashMap;

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_layers::{FlatState, flat_state_at, resolve_driver_edges};
use ori3_model::{Document, TechniqueKind, VertexId};

use crate::cp_svg::num;

/// 1コマの座標系(正方形100×100)。ページ側はこの箱を並べるだけでよい。
pub(crate) const CELL: f64 = 100.0;
/// 絵を描く範囲 [x0, y0, x1, y1]。上は手順番号、下は注記のために空ける。
const AREA: [f64; 4] = [7.0, 15.0, 93.0, 80.0];

/// 技法の呼び名(折り紙の言葉で書く。専門用語は使わない)。
pub(crate) fn technique_label(kind: TechniqueKind) -> &'static str {
    match kind {
        TechniqueKind::Simple => "単純折り",
        TechniqueKind::Pleat => "段折り",
        TechniqueKind::InsideReverse => "中割り折り",
        TechniqueKind::OutsideReverse => "かぶせ折り",
        TechniqueKind::Petal => "花弁折り",
        TechniqueKind::Squash => "開いてつぶす",
        TechniqueKind::OpenSink => "沈め折り",
        TechniqueKind::Swivel => "ひだ寄せ",
        TechniqueKind::Twist => "ねじり折り",
        TechniqueKind::Pose => "仕上げの角度",
    }
}

/// 技法ごとの目印(矢印の根元に添える小さな形)。原点まわり±4の座標で書く。
fn technique_mark(kind: TechniqueKind) -> &'static str {
    match kind {
        TechniqueKind::Simple => "M -4 0 L 4 0",
        TechniqueKind::Pleat => "M -4 -2 L 0 -2 L 0 2 L 4 2",
        TechniqueKind::InsideReverse => "M -4 3 L 0 -3 L 4 3",
        TechniqueKind::OutsideReverse => "M -4 -3 L 0 3 L 4 -3",
        TechniqueKind::Petal => "M 0 4 C -4 0 -4 -4 0 -4 C 4 -4 4 0 0 4",
        TechniqueKind::Squash => "M -4 -3 L 4 -3 M 0 -3 L 0 3 M -3 3 L 3 3",
        TechniqueKind::OpenSink => "M -4 -3 L 4 -3 L 0 4 Z",
        TechniqueKind::Swivel => "M -4 3 A 4 4 0 0 1 4 3",
        TechniqueKind::Twist => "M 3 -3 A 4 4 0 1 1 -1 -4",
        TechniqueKind::Pose => "M -4 0 L 4 0 M 0 -4 L 0 4",
    }
}

/// 文字をSVGに入れられる形に直す(&・<・>だけ気をつければよい)。
pub(crate) fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 展開図の座標での頂点の位置。
fn vertex_positions(doc: &Document) -> HashMap<VertexId, DVec2> {
    doc.cp
        .vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect()
}

/// 畳んだ絵の上での各面の輪郭(下→上の順)と、そのときの平坦状態。
fn folded_polygons(
    doc: &Document,
    faces: &[Face],
    index: usize,
) -> Result<(Vec<Vec<DVec2>>, FlatState), String> {
    let (state, _warnings) = flat_state_at(doc, faces, index)?;
    let pos = vertex_positions(doc);
    let by_id: HashMap<_, _> = faces.iter().map(|f| (f.id, f)).collect();
    let mut out = Vec::with_capacity(state.order.len());
    for id in &state.order {
        let (Some(face), Some(iso)) = (by_id.get(id), state.placements.get(id)) else {
            continue;
        };
        let poly: Vec<DVec2> = face
            .vertices
            .iter()
            .filter_map(|v| pos.get(v))
            .map(|p| iso.apply(*p))
            .collect();
        if poly.len() >= 3 {
            out.push(poly);
        }
    }
    if out.is_empty() {
        return Err("紙の形が求まりませんでした".to_string());
    }
    Ok((out, state))
}

/// これから折る線を畳んだ絵の上へ写す。戻り値は(始点, 終点, 折り上がりの角度)。
fn folded_creases(
    doc: &Document,
    faces: &[Face],
    state: &FlatState,
    step_index: usize,
) -> Vec<(DVec2, DVec2, f64)> {
    let pos = vertex_positions(doc);
    let ends: HashMap<_, _> = doc.cp.edges.iter().map(|e| (e.id, (e.v0, e.v1))).collect();
    // 折り目に面している面(最初に見つかったもの)の置き場所を使う。
    // 折り目を挟む2つの面は畳んだ絵の上でその線に沿って重なるので、どちらでもよい。
    let mut face_of = HashMap::new();
    for f in faces {
        for e in &f.edges {
            face_of.entry(*e).or_insert(f.id);
        }
    }
    let mut out = Vec::new();
    for line in &doc.sequence[step_index].drivers {
        for id in resolve_driver_edges(&doc.cp, line) {
            let iso = face_of.get(&id).and_then(|f| state.placements.get(f));
            let (Some((v0, v1)), Some(iso)) = (ends.get(&id), iso) else {
                continue;
            };
            let (Some(a), Some(b)) = (pos.get(v0), pos.get(v1)) else {
                continue;
            };
            out.push((iso.apply(*a), iso.apply(*b), line.target_angle_deg));
        }
    }
    out
}

/// 折り紙の座標からコマの座標への当てはめ(縦横比はそのまま、中央にそろえる)。
struct Fit {
    scale: f64,
    ox: f64,
    oy: f64,
}

impl Fit {
    fn map(&self, p: DVec2) -> (f64, f64) {
        (self.ox + p.x * self.scale, self.oy - p.y * self.scale)
    }
}

/// 全ての点が描画範囲に収まるような当てはめを決める(y軸は上下反転する)。
fn fit_to_area(points: &[DVec2]) -> Fit {
    let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
    let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in points {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    let (w, h) = ((x1 - x0).max(1e-9), (y1 - y0).max(1e-9));
    let (aw, ah) = (AREA[2] - AREA[0], AREA[3] - AREA[1]);
    let scale = (aw / w).min(ah / h);
    Fit {
        scale,
        ox: AREA[0] + (aw - w * scale) / 2.0 - x0 * scale,
        oy: AREA[1] + (ah - h * scale) / 2.0 + y1 * scale,
    }
}

/// 折る前の形(面を下の層から順に描く。最後に描いた上の層が手前に見える)。
fn shape_svg(polys: &[Vec<DVec2>], fit: &Fit) -> String {
    let mut out = String::from(
        "  <g fill=\"#fdfdfb\" stroke=\"#555555\" stroke-width=\"0.35\" \
         stroke-linejoin=\"round\">\n",
    );
    for poly in polys {
        let pts: Vec<String> = poly
            .iter()
            .map(|p| {
                let (x, y) = fit.map(*p);
                format!("{},{}", num(x), num(y))
            })
            .collect();
        out.push_str(&format!("    <polygon points=\"{}\"/>\n", pts.join(" ")));
    }
    out.push_str("  </g>\n");
    out
}

/// これから折る線。山は一点鎖線(赤)、谷は破線(青)で描き分ける。
fn creases_svg(creases: &[(DVec2, DVec2, f64)], fit: &Fit) -> String {
    let mut out = String::new();
    for (mountain, color, dash) in [
        (true, "#c8321e", "2.4 0.8 0.5 0.8"),
        (false, "#1e5ac8", "1.8 1.0"),
    ] {
        let lines: Vec<_> = creases
            .iter()
            .filter(|(_, _, deg)| (*deg > 0.0) == mountain && *deg != 0.0)
            .collect();
        if lines.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "  <g stroke=\"{color}\" stroke-width=\"0.5\" stroke-dasharray=\"{dash}\" \
             fill=\"none\" stroke-linecap=\"round\">\n"
        ));
        for (a, b, _) in lines {
            let (x1, y1) = fit.map(*a);
            let (x2, y2) = fit.map(*b);
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
    out
}

/// 折り線をまたぐ矢印と、技法を表す小さな目印。
///
/// 矢印は折り線の真ん中から線と直角に伸ばし、紙の重心のある側(残る側)へ
/// 向ける。「はみ出している側をこちらへ倒す」という読み方になる。
/// 山折りか谷折りかは線の描き方(一点鎖線か破線か)のほうで示す。
fn arrow_svg(crease: &(DVec2, DVec2), fit: &Fit, kind: TechniqueKind, toward: (f64, f64)) -> String {
    let (a, b) = *crease;
    let (x1, y1) = fit.map(a);
    let (x2, y2) = fit.map(b);
    let (mx, my) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
    let (dx, dy) = (x2 - x1, y2 - y1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-9);
    // 折り線と直角の2方向のうち、紙の重心に近づくほうを選ぶ
    let (ux, uy) = (-dy / len, dx / len);
    let sign = if ux * (toward.0 - mx) + uy * (toward.1 - my) >= 0.0 {
        1.0
    } else {
        -1.0
    };
    let (nx, ny) = (sign * ux, sign * uy);
    let reach = 9.0_f64;
    let (tx, ty) = (mx + nx * reach, my + ny * reach);
    let (px, py) = (-ny, nx); // 矢じりの横向き
    let head = format!(
        "M {} {} L {} {} L {} {} Z",
        num(tx),
        num(ty),
        num(tx - nx * 3.4 + px * 1.7),
        num(ty - ny * 3.4 + py * 1.7),
        num(tx - nx * 3.4 - px * 1.7),
        num(ty - ny * 3.4 - py * 1.7)
    );
    format!(
        "  <g stroke=\"#1a1a1a\" stroke-width=\"0.55\" fill=\"none\" stroke-linecap=\"round\">\n\
         \x20   <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>\n\
         \x20   <path d=\"{head}\" fill=\"#1a1a1a\"/>\n\
         \x20   <path d=\"{mark}\" transform=\"translate({} {}) scale(0.62)\"/>\n\
         \x20 </g>\n",
        num(mx - nx * reach * 0.55),
        num(my - ny * reach * 0.55),
        num(tx - nx * 3.0),
        num(ty - ny * 3.0),
        num(mx - nx * reach * 1.15),
        num(my - ny * reach * 1.15),
        head = head,
        mark = technique_mark(kind),
    )
}

/// 日本語が出る一般的な書体を順に指定する(見つかった最初のものが使われる)。
pub(crate) const FONT: &str = "Yu Gothic, Meiryo, Hiragino Sans, Noto Sans JP, sans-serif";

/// 手順番号・技法の呼び名・注記の文字。注記は長すぎるとコマから溢れるので折り返す。
fn labels_svg(number: usize, kind: TechniqueKind, note: &str) -> String {
    let mut out = format!(
        "  <text x=\"7\" y=\"9.5\" font-family=\"{FONT}\" font-size=\"5.5\" \
         font-weight=\"bold\" fill=\"#1a1a1a\">{number}. {}</text>\n",
        esc(technique_label(kind))
    );
    let chars: Vec<char> = note.chars().collect();
    for (row, line) in chars.chunks(26).take(2).enumerate() {
        out.push_str(&format!(
            "  <text x=\"7\" y=\"{}\" font-family=\"{FONT}\" font-size=\"3.4\" \
             fill=\"#333333\">{}</text>\n",
            num(86.0 + row as f64 * 4.6),
            esc(&line.iter().collect::<String>())
        ));
    }
    out
}

/// 1コマの中身(SVGの要素の並び)を組み立てる。
///
/// `index` は「そこまで折り終えた状態」を表す手順数。`show_step` が真なら
/// `doc.sequence[index]` をこれから折る手順として、折り線・矢印・番号を重ねる。
pub(crate) fn cell_body(
    doc: &Document,
    faces: &[Face],
    index: usize,
    show_step: bool,
) -> Result<String, String> {
    let (polys, state) = folded_polygons(doc, faces, index)?;
    let creases = if show_step {
        folded_creases(doc, faces, &state, index)
    } else {
        Vec::new()
    };

    let mut points: Vec<DVec2> = polys.iter().flatten().copied().collect();
    for (a, b, _) in &creases {
        points.push(*a);
        points.push(*b);
    }
    let fit = fit_to_area(&points);

    let mut out = shape_svg(&polys, &fit);
    out.push_str(&creases_svg(&creases, &fit));
    if show_step {
        let step = &doc.sequence[index];
        if let Some((a, b, _)) = creases.first() {
            // 紙の重心(コマの座標)を矢印の向きの手がかりにする
            let n = points.len().max(1) as f64;
            let toward = points.iter().map(|p| fit.map(*p)).fold((0.0, 0.0), |s, p| {
                (s.0 + p.0 / n, s.1 + p.1 / n)
            });
            out.push_str(&arrow_svg(&(*a, *b), &fit, step.kind, toward));
        }
        out.push_str(&labels_svg(index + 1, step.kind, &step.note));
    }
    Ok(out)
}

/// 手順 `step_index`(0始まり)の折り図を1枚のSVGとして返す(EXP-003)。
///
/// 絵は「その手順を折る直前の形」で、これから折る線・動かし方の矢印・
/// 技法の目印・手順番号・注記が載る。
pub fn render_step(doc: &Document, step_index: usize) -> Result<String, String> {
    if doc.sequence.is_empty() {
        return Err("折り手順がまだありません".to_string());
    }
    if step_index >= doc.sequence.len() {
        return Err(format!(
            "手順{}はありません(手順は{}個です)",
            step_index + 1,
            doc.sequence.len()
        ));
    }
    let faces = extract_faces(&doc.cp);
    let body = cell_body(doc, &faces, step_index, true)?;
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{c}mm\" height=\"{c}mm\" \
         viewBox=\"0 0 {c} {c}\">\n\
         \x20 <rect x=\"0\" y=\"0\" width=\"{c}\" height=\"{c}\" fill=\"#ffffff\"/>\n\
         {body}</svg>\n",
        c = num(CELL),
    ))
}


/// 縦に `creases` 本の折り目が入った短冊。手順は右の折り目から順に、
/// 山と谷を交互に180度折る(段折りの形)。折り図の見た目を確かめるための土台。
#[cfg(test)]
pub(crate) fn strip_doc(creases: usize) -> Document {
    use ori3_model::{DriverLine, Edge, EdgeKind, FoldStep, Paper, Vertex};
    let n = creases + 1; // 縦に分かれる面の数
    let mut doc = Document::new(Paper {
        width_mm: 200.0,
        height_mm: 200.0,
    });
    let mut vertices = Vec::new();
    for i in 0..=n {
        let x = i as f64 / n as f64;
        vertices.push(Vertex { id: 2 * i as u32, pos: [x, 0.0] });
        vertices.push(Vertex { id: 2 * i as u32 + 1, pos: [x, 1.0] });
    }
    let mut edges = Vec::new();
    let mut id = 0u32;
    let mut add = |v0: u32, v1: u32, kind| {
        edges.push(Edge { id, v0, v1, kind });
        id += 1;
    };
    for i in 0..n as u32 {
        add(2 * i, 2 * i + 2, EdgeKind::Border); // 下辺
        add(2 * i + 1, 2 * i + 3, EdgeKind::Border); // 上辺
    }
    add(0, 1, EdgeKind::Border); // 左端
    add(2 * n as u32, 2 * n as u32 + 1, EdgeKind::Border); // 右端
    // 折り目(手順で折る順に、右から左へ。奇数本目は谷、偶数本目は山)
    const KINDS: [TechniqueKind; 4] = [
        TechniqueKind::Simple,
        TechniqueKind::Pleat,
        TechniqueKind::InsideReverse,
        TechniqueKind::Petal,
    ];
    for (k, i) in (1..=creases).rev().enumerate() {
        let valley = k % 2 == 0;
        let kind = if valley { EdgeKind::Valley } else { EdgeKind::Mountain };
        add(2 * i as u32, 2 * i as u32 + 1, kind);
        let x = i as f64 / n as f64;
        doc.sequence.push(FoldStep {
            id: k as u32 + 1,
            kind: KINDS[k % KINDS.len()],
            drivers: vec![DriverLine {
                a: [x, 0.0],
                b: [x, 1.0],
                target_angle_deg: if valley { -180.0 } else { 180.0 },
            }],
            layer_order: None,
            note: format!("{}本目の折り目を折ります", k + 1),
        });
    }
    doc.cp.vertices = vertices;
    doc.cp.edges = edges;
    doc.cp.next_vertex_id = 2 * n as u32 + 2;
    doc.cp.next_edge_id = id;
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3手順なら3コマぶんの図ができ、どのコマにも
    /// (a)折る前の形 (b)折り線 (c)技法の矢印 (d)手順番号 が入る。
    #[test]
    fn every_step_becomes_one_picture() {
        let doc = strip_doc(3);
        assert_eq!(doc.sequence.len(), 3);
        for i in 0..3 {
            let svg = render_step(&doc, i).unwrap_or_else(|e| panic!("手順{i}: {e}"));
            assert!(svg.contains("<polygon"), "折る前の形がない: {svg}");
            assert!(svg.contains("stroke-dasharray"), "折り線がない: {svg}");
            assert!(svg.contains("<path"), "矢印がない: {svg}");
            assert!(svg.contains(&format!(">{}. ", i + 1)), "手順番号がない: {svg}");
            assert!(svg.contains("本目の折り目を折ります"), "注記がない: {svg}");
        }
    }

    /// 山は一点鎖線(刻み4つ)、谷は破線(刻み2つ)で描き分ける。
    #[test]
    fn mountain_and_valley_look_different() {
        let doc = strip_doc(3);
        let valley = render_step(&doc, 0).unwrap(); // 1本目は谷
        let mountain = render_step(&doc, 1).unwrap(); // 2本目は山
        assert!(valley.contains("stroke-dasharray=\"1.8 1.0\""), "{valley}");
        assert!(valley.contains("#1e5ac8"), "{valley}");
        assert!(mountain.contains("stroke-dasharray=\"2.4 0.8 0.5 0.8\""), "{mountain}");
        assert!(mountain.contains("#c8321e"), "{mountain}");
    }

    /// 技法ごとに矢印へ添える目印が変わる。
    #[test]
    fn each_technique_has_its_own_mark() {
        let doc = strip_doc(3);
        let simple = render_step(&doc, 0).unwrap(); // 単純折り
        let pleat = render_step(&doc, 1).unwrap(); // 段折り
        assert!(simple.contains(technique_mark(TechniqueKind::Simple)), "{simple}");
        assert!(pleat.contains(technique_mark(TechniqueKind::Pleat)), "{pleat}");
        // 目印は10種すべて別の形
        let mut marks: Vec<&str> = [
            TechniqueKind::Simple, TechniqueKind::Pleat, TechniqueKind::InsideReverse,
            TechniqueKind::OutsideReverse, TechniqueKind::Petal, TechniqueKind::Squash,
            TechniqueKind::OpenSink, TechniqueKind::Swivel, TechniqueKind::Twist,
            TechniqueKind::Pose,
        ]
        .into_iter()
        .map(technique_mark)
        .collect();
        marks.sort_unstable();
        marks.dedup();
        assert_eq!(marks.len(), 10);
    }

    /// 技法の呼び名は折り紙の言葉(専門用語を出さない。設計原則3b)。
    #[test]
    fn technique_names_are_origami_words() {
        assert_eq!(technique_label(TechniqueKind::InsideReverse), "中割り折り");
        assert_eq!(technique_label(TechniqueKind::Squash), "開いてつぶす");
    }

    #[test]
    fn no_steps_or_missing_step_is_a_japanese_error() {
        let empty = strip_doc(0);
        let err = render_step(&empty, 0).unwrap_err();
        assert!(err.contains("折り手順がまだありません"), "err={err}");
        let doc = strip_doc(3);
        let err = render_step(&doc, 3).unwrap_err();
        assert!(err.contains("手順4はありません"), "err={err}");
    }

    #[test]
    fn special_characters_in_notes_do_not_break_the_picture() {
        assert_eq!(esc("<a & b>"), "&lt;a &amp; b&gt;");
    }
}
