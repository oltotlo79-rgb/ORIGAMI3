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
use ori3_model::{AlignmentMode, AlignmentTarget, Document, FoldStep, TechniqueKind, VertexId};

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

/// 畳んだ絵の上での折り線1本ぶん。
struct Crease {
    a: DVec2,
    b: DVec2,
    /// 折り上がりの角度(正なら山、負なら谷)。
    angle_deg: f64,
    /// 同じ指示(手順の`drivers`の何番目か)から出た線をまとめる番号。
    driver: usize,
}

/// これから折る線を畳んだ絵の上へ写す。
///
/// 角度0の指示は「折らない」という意味なので線も矢印も出さない(ここで落とす)。
fn folded_creases(
    doc: &Document,
    faces: &[Face],
    state: &FlatState,
    step_index: usize,
) -> Vec<Crease> {
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
    for (driver, line) in doc.sequence[step_index].drivers.iter().enumerate() {
        if line.target_angle_deg == 0.0 {
            continue; // 折らない指示。線を描かないので矢印も出さない
        }
        for id in resolve_driver_edges(&doc.cp, line) {
            let iso = face_of.get(&id).and_then(|f| state.placements.get(f));
            let (Some((v0, v1)), Some(iso)) = (ends.get(&id), iso) else {
                continue;
            };
            let (Some(a), Some(b)) = (pos.get(v0), pos.get(v1)) else {
                continue;
            };
            out.push(Crease {
                a: iso.apply(*a),
                b: iso.apply(*b),
                angle_deg: line.target_angle_deg,
                driver,
            });
        }
    }
    out
}

/// 1コマに出す矢印の上限。コマは100×100で矢印は長さ約9なので、
/// これ以上並べると絵が読めなくなる。
const MAX_ARROWS: usize = 6;

/// 矢印を付ける折り線を選ぶ。
///
/// 1つの指示は展開図の上で何本もの折り目に分かれることがあるため、指示ごとに
/// 一番長い1本だけを代表にする(同じ直線上に矢印が何本も並ぶのを防ぐ)。
/// それでも指示が多いときは、先頭から等間隔に間引いて最大 `MAX_ARROWS` 本にする。
fn arrow_targets(creases: &[Crease]) -> Vec<(DVec2, DVec2)> {
    let mut best: Vec<(usize, f64, DVec2, DVec2)> = Vec::new();
    for c in creases {
        let len = (c.b - c.a).length();
        match best.iter_mut().find(|(d, ..)| *d == c.driver) {
            Some(slot) => {
                if len > slot.1 {
                    *slot = (c.driver, len, c.a, c.b);
                }
            }
            None => best.push((c.driver, len, c.a, c.b)),
        }
    }
    let step = best.len().div_ceil(MAX_ARROWS).max(1);
    best.iter()
        .step_by(step)
        .map(|(_, _, a, b)| (*a, *b))
        .collect()
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

/// 点群の外接矩形 `[x0, y0, x1, y1]`。
///
/// 折り図では折る直前の紙の形を渡し、その見た目を九宮格へ分ける基準にする。
fn bounds_of_points(points: &[DVec2]) -> [f64; 4] {
    let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
    let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in points {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    if !x0.is_finite() {
        [0.0, 0.0, 1.0, 1.0]
    } else {
        [x0, y0, x1, y1]
    }
}

/// 九宮格の横・縦それぞれの位置。内部座標はyが上向き。
fn third(value: f64, lo: f64, hi: f64) -> i8 {
    let span = hi - lo;
    if span.abs() <= 1e-9 {
        return 0;
    }
    let t = (value - lo) / span;
    if t < 1.0 / 3.0 {
        -1
    } else if t > 2.0 / 3.0 {
        1
    } else {
        0
    }
}

/// 折る直前の紙を九宮格に分け、点が紙のどのあたりかを折り紙の言葉で返す。
fn position_label(point: [f64; 2], bounds: [f64; 4]) -> &'static str {
    match (
        third(point[0], bounds[0], bounds[2]),
        third(point[1], bounds[1], bounds[3]),
    ) {
        (-1, 1) => "左上",
        (0, 1) => "上",
        (1, 1) => "右上",
        (-1, 0) => "左",
        (0, 0) => "中央",
        (1, 0) => "右",
        (-1, -1) => "左下",
        (0, -1) => "下",
        (1, -1) => "右下",
        _ => "中央", // `third` の戻り値は -1/0/1 だけ
    }
}

/// 点の位置を説明に使う言い方へ直す。四隅は「角」、それ以外は「点」と呼ぶ。
fn point_label(point: [f64; 2], bounds: [f64; 4]) -> String {
    let place = position_label(point, bounds);
    let corner = matches!(place, "左上" | "右上" | "左下" | "右下");
    format!("{place}の{}", if corner { "角" } else { "点" })
}

fn midpoint(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
}

fn line_label(a: [f64; 2], b: [f64; 2], bounds: [f64; 4]) -> String {
    format!("{}の線", position_label(midpoint(a, b), bounds))
}

/// 山谷をdriver角の符号から読む。正負が混ざる技法も情報を落とさない。
fn fold_action(step: &FoldStep) -> &'static str {
    let mountain = step.drivers.iter().any(|d| d.target_angle_deg > 1e-9);
    let valley = step.drivers.iter().any(|d| d.target_angle_deg < -1e-9);
    match (mountain, valley) {
        (true, false) => "山折り",
        (false, true) => "谷折り",
        (true, true) => "山折りと谷折り",
        (false, false) => "折り目を開く",
    }
}

/// 「合わせて折る」で選ばれた対応から、人が読める「どことどこ」の部分を作る。
fn alignment_instruction(step: &FoldStep, bounds: [f64; 4]) -> Option<String> {
    let alignment = step.alignment.as_ref()?;
    match (&alignment.mode, alignment.picks.as_slice()) {
        (
            AlignmentMode::ThroughTwoPoints,
            [
                AlignmentTarget::Point { p: first },
                AlignmentTarget::Point { p: second },
                ..,
            ],
        ) => Some(format!(
            "{}と{}を通るように",
            point_label(*first, bounds),
            point_label(*second, bounds)
        )),
        (
            AlignmentMode::PointPoint,
            [
                AlignmentTarget::Point { p: from },
                AlignmentTarget::Point { p: to },
                ..,
            ],
        ) => Some(format!(
            "{}を{}に合わせて",
            point_label(*from, bounds),
            point_label(*to, bounds)
        )),
        (
            AlignmentMode::LineLine,
            [
                AlignmentTarget::Line { a: a0, b: a1 },
                AlignmentTarget::Line { a: b0, b: b1 },
                ..,
            ],
        ) => Some(format!(
            "{}を{}に合わせて",
            line_label(*a0, *a1, bounds),
            line_label(*b0, *b1, bounds)
        )),
        (
            AlignmentMode::PointPerpendicularLine,
            [
                AlignmentTarget::Point { p },
                AlignmentTarget::Line { a, b },
                ..,
            ],
        ) => Some(format!(
            "{}を通り、{}に垂直になるように",
            point_label(*p, bounds),
            line_label(*a, *b, bounds)
        )),
        (
            AlignmentMode::PointLineThrough,
            [
                AlignmentTarget::Point { p },
                AlignmentTarget::Line { a, b },
                AlignmentTarget::Point { p: through },
                ..,
            ],
        ) => Some(format!(
            "{}を{}に合わせ、{}を通るように",
            point_label(*p, bounds),
            line_label(*a, *b, bounds),
            point_label(*through, bounds)
        )),
        (
            AlignmentMode::PointToLinePointToLine,
            [
                AlignmentTarget::Point { p: first },
                AlignmentTarget::Line {
                    a: first_a,
                    b: first_b,
                },
                AlignmentTarget::Point { p: second },
                AlignmentTarget::Line {
                    a: second_a,
                    b: second_b,
                },
                ..,
            ],
        ) => Some(format!(
            "{}を{}に、同時に{}を{}に合わせて",
            point_label(*first, bounds),
            line_label(*first_a, *first_b, bounds),
            point_label(*second, bounds),
            line_label(*second_a, *second_b, bounds)
        )),
        (
            AlignmentMode::PointLinePerpendicular,
            [
                AlignmentTarget::Point { p },
                AlignmentTarget::Line {
                    a: target_a,
                    b: target_b,
                },
                AlignmentTarget::Line {
                    a: perpendicular_a,
                    b: perpendicular_b,
                },
                ..,
            ],
        ) => Some(format!(
            "{}を{}に合わせ、折り目が{}に垂直になるように",
            point_label(*p, bounds),
            line_label(*target_a, *target_b, bounds),
            line_label(*perpendicular_a, *perpendicular_b, bounds)
        )),
        (AlignmentMode::ExistingLine, [AlignmentTarget::Line { a, b }, ..]) => {
            Some(format!("{}に沿って", line_label(*a, *b, bounds)))
        }
        _ => None,
    }
}

/// 折り操作の内容から、折り図へ添える短い日本語説明を作る純関数。
///
/// `projected_lines` は折る直前の畳み平面へ写した折り線。分割された線が複数ある
/// 場合は最長の1本を位置の代表にする。技法と合わせ折りは `FoldStep` の永続化情報を
/// 使うため、PDF/SVGのどちらでも同じ文になる。
pub fn automatic_instruction(
    step: &FoldStep,
    projected_lines: &[[[f64; 2]; 2]],
    bounds: [f64; 4],
) -> String {
    match step.kind {
        TechniqueKind::Pleat => "段折りにする".to_string(),
        TechniqueKind::InsideReverse => "中割り折りにする".to_string(),
        TechniqueKind::OutsideReverse => "かぶせ折りにする".to_string(),
        TechniqueKind::Petal => "花弁折りにする".to_string(),
        TechniqueKind::Squash => "開いてつぶす".to_string(),
        TechniqueKind::OpenSink => "沈め折りにする".to_string(),
        TechniqueKind::Swivel => "ひだ寄せにする".to_string(),
        TechniqueKind::Twist => "ねじり折りにする".to_string(),
        TechniqueKind::Pose => "折り目の角度を整える".to_string(),
        TechniqueKind::Simple => {
            let action = fold_action(step);
            if let Some(prefix) = alignment_instruction(step, bounds) {
                return format!("{prefix}{action}");
            }
            let representative = projected_lines.iter().max_by(|a, b| {
                let length2 = |line: &&[[f64; 2]; 2]| {
                    let dx = line[1][0] - line[0][0];
                    let dy = line[1][1] - line[0][1];
                    dx * dx + dy * dy
                };
                length2(a).total_cmp(&length2(b))
            });
            match representative {
                Some([a, b]) => format!("{}に沿って{action}", line_label(*a, *b, bounds)),
                None => action.to_string(),
            }
        }
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
fn creases_svg(creases: &[Crease], fit: &Fit) -> String {
    let mut out = String::new();
    for (mountain, color, dash) in [
        (true, "#c8321e", "2.4 0.8 0.5 0.8"),
        (false, "#1e5ac8", "1.8 1.0"),
    ] {
        let lines: Vec<_> = creases
            .iter()
            .filter(|c| (c.angle_deg > 0.0) == mountain && c.angle_deg != 0.0)
            .collect();
        if lines.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "  <g stroke=\"{color}\" stroke-width=\"0.5\" stroke-dasharray=\"{dash}\" \
             fill=\"none\" stroke-linecap=\"round\">\n"
        ));
        for c in lines {
            let (x1, y1) = fit.map(c.a);
            let (x2, y2) = fit.map(c.b);
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

/// 重心に寄っているかどうかを見分ける幅(コマの座標。コマは100×100)。
/// これより近ければ「折り線が真ん中を通っている」とみなす。
const TIE: f64 = 1e-3;

/// 矢印を伸ばす向き(折り線と直角の2方向のうちどちらか)を決める。
///
/// ふつうは紙の重心のある側(残る側)へ向ける。ただし折り線が重心のちょうど上を
/// 通る左右対称な半分折りでは重心を使った判定が0の近くになり、丸め誤差だけで
/// 向きが決まってしまう。そこで差が `TIE` に満たないときは重心を見ずに、
/// 折り線の向きだけから決める(画面の上側。それも決まらなければ右側)。
/// こうすると同じ形からは必ず同じ絵になる。
fn arrow_normal(dx: f64, dy: f64, mid: (f64, f64), toward: (f64, f64)) -> (f64, f64) {
    let len = (dx * dx + dy * dy).sqrt().max(1e-9);
    let (ux, uy) = (-dy / len, dx / len);
    let d = ux * (toward.0 - mid.0) + uy * (toward.1 - mid.1);
    let sign = if d > TIE {
        1.0
    } else if d < -TIE {
        -1.0
    } else if uy.abs() > TIE {
        // 画面の上(yが小さいほう)へ
        if uy < 0.0 { 1.0 } else { -1.0 }
    } else if ux > 0.0 {
        1.0
    } else {
        -1.0
    };
    (sign * ux, sign * uy)
}

/// 折り線をまたぐ矢印と、技法を表す小さな目印。
///
/// 矢印は折り線の真ん中から線と直角に伸ばし、[`arrow_normal`] が決めた側へ向ける。
/// 「はみ出している側をこちらへ倒す」という読み方になる。
/// 山折りか谷折りかは線の描き方(一点鎖線か破線か)のほうで示す。
fn arrow_svg(
    crease: &(DVec2, DVec2),
    fit: &Fit,
    kind: TechniqueKind,
    toward: (f64, f64),
) -> String {
    let (a, b) = *crease;
    let (x1, y1) = fit.map(a);
    let (x2, y2) = fit.map(b);
    let (mx, my) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
    let (dx, dy) = (x2 - x1, y2 - y1);
    let (nx, ny) = arrow_normal(dx, dy, (mx, my), toward);
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

/// 手順番号・技法の呼び名・注記・自動説明の文字。
///
/// 手動注記があれば従来の大きさで先に最大2行を確保し、自動説明はその後ろへ
/// 小さく1行載せる。注記がなければ自動説明を読みやすい大きさで最大2行使う。
fn labels_svg(number: usize, kind: TechniqueKind, note: &str, instruction: &str) -> String {
    let mut out = format!(
        "  <text x=\"7\" y=\"9.5\" font-family=\"{FONT}\" font-size=\"5.5\" \
         font-weight=\"bold\" fill=\"#1a1a1a\">{number}. {}</text>\n",
        esc(technique_label(kind))
    );

    let note_chars: Vec<char> = note.chars().collect();
    let note_lines: Vec<String> = note_chars
        .chunks(26)
        .take(2)
        .map(|line| line.iter().collect())
        .collect();
    for (row, line) in note_lines.iter().enumerate() {
        out.push_str(&format!(
            "  <text x=\"7\" y=\"{}\" font-family=\"{FONT}\" font-size=\"3.4\" \
             fill=\"#333333\" data-role=\"manual-note\">{}</text>\n",
            num(86.0 + row as f64 * 4.6),
            esc(line)
        ));
    }

    let instruction_chars: Vec<char> = instruction.chars().collect();
    let (width, rows, size, y, line_height, color) = if note_lines.is_empty() {
        (26, 2, 3.4, 86.0, 4.6, "#333333")
    } else {
        // 手動注記を2行使っても、最下部の1行は自動説明用に残る。
        (
            34,
            1,
            2.7,
            86.0 + note_lines.len() as f64 * 4.6,
            3.8,
            "#666666",
        )
    };
    for (row, line) in instruction_chars.chunks(width).take(rows).enumerate() {
        out.push_str(&format!(
            "  <text x=\"7\" y=\"{}\" font-family=\"{FONT}\" font-size=\"{}\" \
             fill=\"{color}\" data-role=\"automatic-instruction\">{}</text>\n",
            num(y + row as f64 * line_height),
            num(size),
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

    let paper_points: Vec<DVec2> = polys.iter().flatten().copied().collect();
    let paper_bounds = bounds_of_points(&paper_points);
    let mut points = paper_points;
    for c in &creases {
        points.push(c.a);
        points.push(c.b);
    }
    let fit = fit_to_area(&points);

    let mut out = shape_svg(&polys, &fit);
    out.push_str(&creases_svg(&creases, &fit));
    if show_step {
        let step = &doc.sequence[index];
        // 紙の重心(コマの座標)を矢印の向きの手がかりにする
        let n = points.len().max(1) as f64;
        let toward = points
            .iter()
            .map(|p| fit.map(*p))
            .fold((0.0, 0.0), |s, p| (s.0 + p.0 / n, s.1 + p.1 / n));
        // 折り線ごとに矢印を出す(段折りのように1手順で何本も折る技法に対応)
        for crease in arrow_targets(&creases) {
            out.push_str(&arrow_svg(&crease, &fit, step.kind, toward));
        }
        let projected_lines: Vec<[[f64; 2]; 2]> = creases
            .iter()
            .map(|c| [[c.a.x, c.a.y], [c.b.x, c.b.y]])
            .collect();
        let instruction = automatic_instruction(step, &projected_lines, paper_bounds);
        out.push_str(&labels_svg(index + 1, step.kind, &step.note, &instruction));
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
        vertices.push(Vertex {
            id: 2 * i as u32,
            pos: [x, 0.0],
        });
        vertices.push(Vertex {
            id: 2 * i as u32 + 1,
            pos: [x, 1.0],
        });
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
        let kind = if valley {
            EdgeKind::Valley
        } else {
            EdgeKind::Mountain
        };
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
            alignment: None,
            finish_soft: None,
            technique_classification: None,
        });
    }
    doc.cp.vertices = vertices;
    doc.cp.edges = edges;
    doc.cp.next_vertex_id = 2 * n as u32 + 2;
    doc.cp.next_edge_id = id;
    doc
}

/// 段折りやひだ寄せのように、1つの手順で何本もの折り目を同時に折る作品。
/// [`strip_doc`] の折り目を全部まとめて1手順にしたもの。
#[cfg(test)]
pub(crate) fn multi_driver_doc(creases: usize) -> Document {
    let mut doc = strip_doc(creases);
    let drivers: Vec<_> = doc
        .sequence
        .iter()
        .flat_map(|s| s.drivers.clone())
        .collect();
    doc.sequence.truncate(1);
    doc.sequence[0].kind = TechniqueKind::Pleat;
    doc.sequence[0].drivers = drivers;
    doc.sequence[0].note = format!("{creases}本の折り目を一度に折ります");
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1コマに出ている矢印の本数(矢印1本につき専用の`<g>`が1つ出る)。
    fn arrow_count(svg: &str) -> usize {
        svg.matches("stroke=\"#1a1a1a\" stroke-width=\"0.55\"")
            .count()
    }

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
            assert!(
                svg.contains(&format!(">{}. ", i + 1)),
                "手順番号がない: {svg}"
            );
            assert!(svg.contains("本目の折り目を折ります"), "注記がない: {svg}");
            assert!(
                svg.contains("data-role=\"automatic-instruction\""),
                "自動説明がない: {svg}"
            );
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
        assert!(
            mountain.contains("stroke-dasharray=\"2.4 0.8 0.5 0.8\""),
            "{mountain}"
        );
        assert!(mountain.contains("#c8321e"), "{mountain}");
    }

    /// 技法ごとに矢印へ添える目印が変わる。
    #[test]
    fn each_technique_has_its_own_mark() {
        let doc = strip_doc(3);
        let simple = render_step(&doc, 0).unwrap(); // 単純折り
        let pleat = render_step(&doc, 1).unwrap(); // 段折り
        assert!(
            simple.contains(technique_mark(TechniqueKind::Simple)),
            "{simple}"
        );
        assert!(
            pleat.contains(technique_mark(TechniqueKind::Pleat)),
            "{pleat}"
        );
        // 目印は10種すべて別の形
        let mut marks: Vec<&str> = [
            TechniqueKind::Simple,
            TechniqueKind::Pleat,
            TechniqueKind::InsideReverse,
            TechniqueKind::OutsideReverse,
            TechniqueKind::Petal,
            TechniqueKind::Squash,
            TechniqueKind::OpenSink,
            TechniqueKind::Swivel,
            TechniqueKind::Twist,
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

    /// 合わせ折りの点対応は、折る直前の形を九宮格に分けて自然な日本語にする。
    #[test]
    fn point_alignment_names_both_corners_and_fold_direction() {
        let mut doc = strip_doc(1);
        let step = &mut doc.sequence[0];
        step.kind = TechniqueKind::Simple;
        step.drivers[0].target_angle_deg = -180.0;
        step.alignment = Some(ori3_model::FoldAlignment {
            mode: AlignmentMode::PointPoint,
            picks: vec![
                AlignmentTarget::Point { p: [0.9, 0.1] },
                AlignmentTarget::Point { p: [0.1, 0.9] },
            ],
        });
        assert_eq!(
            automatic_instruction(step, &[[[0.5, 0.0], [0.5, 1.0]]], [0.0, 0.0, 1.0, 1.0]),
            "右下の角を左上の角に合わせて谷折り"
        );
    }

    /// 藤田・羽鳥の7作図と既存折り筋の指定を、選択内容を落とさず日本語にする。
    #[test]
    fn every_alignment_mode_has_a_japanese_instruction() {
        let point = |p| AlignmentTarget::Point { p };
        let line = |a, b| AlignmentTarget::Line { a, b };
        let left = || line([0.1, 0.1], [0.1, 0.9]);
        let top = || line([0.1, 0.9], [0.9, 0.9]);
        let center = || line([0.1, 0.1], [0.9, 0.9]);
        let cases = vec![
            (
                AlignmentMode::ThroughTwoPoints,
                vec![point([0.1, 0.9]), point([0.9, 0.1])],
                "左上の角と右下の角を通るように谷折り",
            ),
            (
                AlignmentMode::PointPoint,
                vec![point([0.1, 0.9]), point([0.9, 0.1])],
                "左上の角を右下の角に合わせて谷折り",
            ),
            (
                AlignmentMode::LineLine,
                vec![left(), top()],
                "左の線を上の線に合わせて谷折り",
            ),
            (
                AlignmentMode::PointPerpendicularLine,
                vec![point([0.5, 0.5]), left()],
                "中央の点を通り、左の線に垂直になるように谷折り",
            ),
            (
                AlignmentMode::PointLineThrough,
                vec![point([0.1, 0.9]), left(), point([0.5, 0.5])],
                "左上の角を左の線に合わせ、中央の点を通るように谷折り",
            ),
            (
                AlignmentMode::PointToLinePointToLine,
                vec![point([0.1, 0.9]), left(), point([0.9, 0.1]), top()],
                "左上の角を左の線に、同時に右下の角を上の線に合わせて谷折り",
            ),
            (
                AlignmentMode::PointLinePerpendicular,
                vec![point([0.1, 0.9]), left(), top()],
                "左上の角を左の線に合わせ、折り目が上の線に垂直になるように谷折り",
            ),
            (
                AlignmentMode::ExistingLine,
                vec![center()],
                "中央の線に沿って谷折り",
            ),
        ];

        for (mode, picks, expected) in cases {
            let mut doc = strip_doc(1);
            let step = &mut doc.sequence[0];
            step.kind = TechniqueKind::Simple;
            step.drivers[0].target_angle_deg = -180.0;
            step.alignment = Some(ori3_model::FoldAlignment { mode, picks });
            assert_eq!(
                automatic_instruction(step, &[[[0.5, 0.0], [0.5, 1.0]]], [0.0, 0.0, 1.0, 1.0]),
                expected,
                "mode={mode:?}"
            );
        }
    }

    /// 合わせ指定がない通常折りは、折り線の九宮格位置と山谷を説明する。
    #[test]
    fn centered_mountain_uses_the_crease_position() {
        let mut doc = strip_doc(1);
        let step = &mut doc.sequence[0];
        step.kind = TechniqueKind::Simple;
        step.drivers[0].target_angle_deg = 180.0;
        step.alignment = None;
        assert_eq!(
            automatic_instruction(step, &[[[0.5, 0.0], [0.5, 1.0]]], [0.0, 0.0, 1.0, 1.0]),
            "中央の線に沿って山折り"
        );
    }

    /// 名前のある技法は山谷の内訳より、折り紙で通じる技法名を優先する。
    #[test]
    fn pleat_has_a_short_technique_instruction() {
        let mut doc = multi_driver_doc(2);
        let step = &mut doc.sequence[0];
        step.kind = TechniqueKind::Pleat;
        assert_eq!(
            automatic_instruction(step, &[], [0.0, 0.0, 1.0, 1.0]),
            "段折りにする"
        );
    }

    /// 手動注記を先に従来サイズで置き、自動文はその後ろへ小さく添える。
    #[test]
    fn manual_note_precedes_the_smaller_automatic_instruction() {
        let svg = labels_svg(
            1,
            TechniqueKind::Simple,
            "角をしっかり押さえる",
            "右下の角を左上の角に合わせて谷折り",
        );
        let manual = svg.find("data-role=\"manual-note\"").unwrap();
        let automatic = svg.find("data-role=\"automatic-instruction\"").unwrap();
        assert!(manual < automatic, "手動注記が先ではない: {svg}");
        let automatic_tag = svg[..automatic].rfind("<text").unwrap();
        assert!(
            svg[automatic_tag..automatic].contains("font-size=\"2.7\""),
            "自動文が小さくない: {svg}"
        );
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

    /// 段折りのように1手順で何本も折るときは、折り線の数だけ矢印が出る。
    #[test]
    fn every_crease_of_a_step_gets_its_own_arrow() {
        for creases in 1..=4 {
            let doc = multi_driver_doc(creases);
            assert_eq!(doc.sequence[0].drivers.len(), creases);
            let svg = render_step(&doc, 0).unwrap_or_else(|e| panic!("{creases}本: {e}"));
            assert_eq!(arrow_count(&svg), creases, "矢印の本数が合わない: {svg}");
        }
    }

    /// 折り目が多すぎるときは、絵が潰れないよう間引いて最大6本にする。
    #[test]
    fn too_many_creases_are_thinned_out() {
        let doc = multi_driver_doc(9);
        let svg = render_step(&doc, 0).unwrap();
        // 9本 → 2本ごとに間引いて5本(上限6以下)
        assert_eq!(arrow_count(&svg), 5, "{svg}");
        assert!(arrow_count(&svg) <= MAX_ARROWS);
    }

    /// 角度0(折らない)の指示は折り線も矢印も出さない。
    #[test]
    fn a_zero_angle_driver_draws_neither_line_nor_arrow() {
        let mut doc = multi_driver_doc(2);
        doc.sequence[0].drivers[0].target_angle_deg = 0.0;
        let svg = render_step(&doc, 0).unwrap();
        assert_eq!(arrow_count(&svg), 1, "折る指示1本ぶんだけ残るはず: {svg}");
        // 谷折り(角度が負)の線だけが残り、山折りの線は消える
        assert_eq!(svg.matches("<line x1=").count(), 2, "{svg}");
    }

    /// 折り線が紙の重心をちょうど通る左右対称な半分折りでも、矢印の向きが揺れない。
    #[test]
    fn a_symmetric_half_fold_picks_a_stable_arrow_direction() {
        // 重心との差がちょうど0/ごくわずかな正/ごくわずかな負、どれでも同じ向き
        let mid = (50.0, 50.0);
        let vertical = [
            arrow_normal(0.0, -20.0, mid, mid),
            arrow_normal(0.0, -20.0, mid, (50.0 + 1e-15, 50.0)),
            arrow_normal(0.0, -20.0, mid, (50.0 - 1e-15, 50.0)),
        ];
        assert_eq!(vertical[0], (1.0, 0.0), "縦線なら右へ向ける");
        assert!(vertical.iter().all(|n| *n == vertical[0]), "{vertical:?}");
        // 横線なら画面の上へ
        assert_eq!(arrow_normal(20.0, 0.0, mid, mid), (0.0, -1.0));
        // 重心がはっきり片側にあるときは、これまでどおりそちらへ向く
        assert_eq!(arrow_normal(0.0, -20.0, mid, (10.0, 50.0)), (-1.0, 0.0));

        // 実際の作品(1本の折り目で半分に折る)でも毎回同じ絵になる
        let doc = strip_doc(1);
        let svg = render_step(&doc, 0).unwrap();
        assert_eq!(svg, render_step(&doc, 0).unwrap());
        assert_eq!(arrow_count(&svg), 1, "{svg}");
    }
}
