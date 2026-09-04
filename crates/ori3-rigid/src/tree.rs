//! 全域木の角度伝播: 展開図の各面を剛体とみなし、面隣接グラフ(折り辺を共有する
//! 面同士)のBFS全域木に沿って、ヒンジ角から各面の3D姿勢(回転+並進)を決める。
//!
//! # ヒンジ角の向きの規約(このアプリ全体で共通)
//!
//! 紙の表を+z側とする(根面はxy平面に固定され、表が+zを向く)。
//! ヒンジの回転軸は「固定面(親)の反時計回り境界に現れる向き a→b」に取る。
//! 面は反時計回りなので、このとき親面の内部は軸の左側、動く面(子)は右側にある。
//! 子面を右手系で軸まわりに+θ回転すると、θ∈(0°,180°)で子面は表(+z)から見て
//! 奥(−z)側へ畳まれる=山折り。−θなら手前(+z)側へ畳まれる=谷折り。
//!
//! ```text
//! 断面図(ヒンジ軸uは紙面の奥→手前へ向く。親面の内部は軸の左側)
//!         z(紙の表側)
//!         ↑
//!  親面───●───子面        ●=ヒンジ軸
//!         │    ↘ +θ(山折り): 子面が奥(−z)側へ倒れる
//!       −z側        −θ(谷折り): 子面が手前(+z)側へ起きる
//! ```
//!
//! この向き付けは子面の姿勢を親面のCP座標(=紙の材質座標)で決めてから親の変換を
//! 合成するため、途中の面がどう折られていても「紙の表に対する山谷」が保たれる。

use std::collections::{BTreeMap, HashMap, VecDeque};

use glam::{DMat3, DQuat, DVec3};
use ori3_cp::Face;
use ori3_model::{CreasePattern, EPS, EdgeId, Face3D, FaceId, Frame3D, VertexId};

/// 面の3D姿勢(回転+並進): `p3d = rot * vec3(p2d, 0) + trans`
#[derive(Clone, Debug)]
pub struct FoldedFrame {
    pub transforms: HashMap<FaceId, (DMat3, DVec3)>,
    /// 平坦な展開図から現在角まで、全ヒンジを同じ角度checkpointまで動かした
    /// 重なり順用の経路。部分折りは最終角へ達した後、その角度を維持する。
    pub(crate) surface_path_transforms: Vec<HashMap<FaceId, (DMat3, DVec3)>>,
    /// 完全折りのヒンジがあり、面積を持つ共平面重なりが生じ得るか。
    pub(crate) has_surface_stack: bool,
    /// 面隣接グラフに非木ヒンジがあり、同じexact制約を満たす閉路枝が複数あり得るか。
    pub(crate) has_loop_closures: bool,
    /// 根面から折り木をたどった鏡映回数の偶奇。
    /// ±90°を越えたヒンジを、最寄りの平坦状態での1回の鏡映として数える。
    pub mirrored: HashMap<FaceId, bool>,
    /// 完全に折り切った折り目がつなぐ隣接2面の、厳密な上下(下の面, 上の面)。
    /// 折り目の向きだけで決まり、深度の丸めでは壊れない。
    pub(crate) exact_stack_constraints: Vec<(FaceId, FaceId)>,
    /// 伝播時の警告(面が折り線で繋がっていない等)。`to_frame3d`でFrame3Dへ引き継ぐ。
    pub warnings: Vec<String>,
}

/// 全域木の1辺: 親面の姿勢から子面の姿勢を求めるのに必要な情報。
pub(crate) struct TreeStep {
    /// faces内の添字
    pub child: usize,
    pub parent: usize,
    /// `Forest::hinges` 内の添字
    pub hinge: usize,
    /// 回転軸上の1点(親面のCCW境界での辺の始点、z=0)
    pub axis_a: DVec3,
    /// 回転軸の単位方向(親面のCCW境界での向き。親面の内部が軸の左側になる)
    pub axis_u: DVec3,
}

/// 非木辺(全域木に入らなかったヒンジ)。ループ閉包の残差計算に使う:
/// from面の姿勢からこのヒンジで折って予測したto面の姿勢と、木経路で伝播した
/// to面の姿勢の差がループ一周の閉包残差に相当する。
pub(crate) struct LoopClosure {
    /// `Forest::hinges` 内の添字
    pub hinge: usize,
    /// faces内の添字。この面からヒンジ折りで相手面の姿勢を予測する
    pub from: usize,
    pub to: usize,
    /// 回転軸上の1点(from面のCCW境界での辺の始点、z=0)
    pub axis_a: DVec3,
    /// 回転軸の単位方向(from面のCCW境界での向き)
    pub axis_u: DVec3,
}

/// 面隣接グラフのBFS全域木(森)。ヒンジ=ちょうど2つの異なる面が共有する辺。
pub(crate) struct Forest {
    /// ヒンジ添字→辺ID
    pub hinges: Vec<EdgeId>,
    /// ヒンジ添字→2つの向き付き軸(面添字, 軸始点, 軸単位方向)。
    /// ソルバーが面隣接グラフを任意の向きに渡り歩くために使う。
    pub hinge_occ: Vec<[(usize, DVec3, DVec3); 2]>,
    /// BFS順の木辺(親の姿勢は必ず子より先に決まる)
    pub steps: Vec<TreeStep>,
    /// 非木辺(ループを作るヒンジ)
    pub loops: Vec<LoopClosure>,
    /// 各連結成分の根面(faces内の添字)。姿勢は恒等変換に固定される。
    pub roots: Vec<usize>,
}

/// 親姿勢(r, t)にヒンジ回転(軸上の点a・単位方向u・角theta_rad)を合成した
/// 子姿勢を返す。子のCP座標pは `r * (rot_local * (p - a) + a) + t` へ写る。
pub(crate) fn fold_child(r: DMat3, t: DVec3, a: DVec3, u: DVec3, theta_rad: f64) -> (DMat3, DVec3) {
    let rl = DMat3::from_quat(DQuat::from_axis_angle(u, theta_rad));
    (r * rl, t + r * (a - rl * a))
}

fn vertex_positions(cp: &CreasePattern) -> HashMap<VertexId, DVec3> {
    cp.vertices
        .iter()
        .map(|v| (v.id, DVec3::new(v.pos[0], v.pos[1], 0.0)))
        .collect()
}

/// 面隣接グラフのBFS全域木(森)を作る。根は各連結成分の最小添字の面
/// (面IDは添字順に採番されるため最小FaceIdと一致)で、走査はヒンジの
/// 辺ID順に行うため結果は決定的。
pub(crate) fn build_forest(cp: &CreasePattern, faces: &[Face]) -> Forest {
    let vpos = vertex_positions(cp);

    // 辺ID→出現リスト(面添字, 軸始点a, 軸単位方向u)。BTreeMapで辺ID順に固定。
    type Occurrence = (usize, DVec3, DVec3);
    // 1辺がヒンジになる条件は「ちょうど2面」だけ。辺ごとの小Vec確保を避け、
    // 3面以上の非多様体辺はcountだけ保持して従来どおり除外する。
    let mut occ: BTreeMap<EdgeId, (usize, [Option<Occurrence>; 2])> = BTreeMap::new();
    for (fi, face) in faces.iter().enumerate() {
        let n = face.vertices.len();
        for (j, &eid) in face.edges.iter().enumerate() {
            let (Some(&a), Some(&b)) = (
                vpos.get(&face.vertices[j]),
                vpos.get(&face.vertices[(j + 1) % n]),
            ) else {
                continue;
            };
            let d = b - a;
            if d.length() < EPS {
                continue;
            }
            let entry = occ.entry(eid).or_insert((0, [None, None]));
            if entry.0 < entry.1.len() {
                entry.1[entry.0] = Some((fi, a, d.normalize()));
            }
            entry.0 += 1;
        }
    }

    // ヒンジ = ちょうど2つの異なる面に共有される辺(スリット両面走査などは除外)
    let mut hinges: Vec<EdgeId> = Vec::new();
    let mut hinge_faces: Vec<(usize, usize)> = Vec::new();
    let mut hinge_occ: Vec<[(usize, DVec3, DVec3); 2]> = Vec::new();
    for (&eid, (count, entries)) in &occ {
        if *count == 2 {
            let first = entries[0].expect("2面辺の第1出現");
            let second = entries[1].expect("2面辺の第2出現");
            if first.0 == second.0 {
                continue;
            }
            hinges.push(eid);
            hinge_faces.push((first.0, second.0));
            hinge_occ.push([first, second]);
        }
    }

    // 面ごとの接続ヒンジ(ヒンジ添字=辺ID昇順なので走査は決定的)
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); faces.len()];
    for (hi, &(f, g)) in hinge_faces.iter().enumerate() {
        adj[f].push(hi);
        adj[g].push(hi);
    }

    let mut visited = vec![false; faces.len()];
    let mut roots = Vec::new();
    let mut steps = Vec::new();
    let mut tree_edge = vec![false; hinges.len()];
    let mut queue = VecDeque::new();
    for start in 0..faces.len() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        roots.push(start);
        queue.push_back(start);
        while let Some(cur) = queue.pop_front() {
            for &hi in &adj[cur] {
                let (f, g) = hinge_faces[hi];
                let nb = if f == cur { g } else { f };
                if visited[nb] {
                    continue;
                }
                visited[nb] = true;
                tree_edge[hi] = true;
                // 回転軸は親(cur)側のCCW境界での向きを使う(冒頭の規約)
                let (a, u) = axis_for(&hinge_occ[hi], cur);
                steps.push(TreeStep {
                    child: nb,
                    parent: cur,
                    hinge: hi,
                    axis_a: a,
                    axis_u: u,
                });
                queue.push_back(nb);
            }
        }
    }

    // 木に入らなかったヒンジ = 非木辺(ループ)。from側の向き付き軸で残差を計算する
    let loops = (0..hinges.len())
        .filter(|&hi| !tree_edge[hi])
        .map(|hi| {
            let (f, g) = hinge_faces[hi];
            let (a, u) = axis_for(&hinge_occ[hi], f);
            LoopClosure {
                hinge: hi,
                from: f,
                to: g,
                axis_a: a,
                axis_u: u,
            }
        })
        .collect();

    Forest {
        hinges,
        hinge_occ,
        steps,
        loops,
        roots,
    }
}

/// 指定面のCCW境界での向き付き軸(始点, 単位方向)を返す。
fn axis_for(occ: &[(usize, DVec3, DVec3); 2], face: usize) -> (DVec3, DVec3) {
    let o = if occ[0].0 == face { occ[0] } else { occ[1] };
    (o.1, o.2)
}

/// ヒンジ角(ラジアン、`Forest::hinges` と同順)で全面の姿勢を伝播する。
pub(crate) fn propagate_with(
    forest: &Forest,
    n_faces: usize,
    angles_rad: &[f64],
) -> Vec<(DMat3, DVec3)> {
    let mut tf = vec![(DMat3::IDENTITY, DVec3::ZERO); n_faces];
    for s in &forest.steps {
        let (rp, tp) = tf[s.parent];
        tf[s.child] = fold_child(rp, tp, s.axis_a, s.axis_u, angles_rad[s.hinge]);
    }
    tf
}

/// 根面から折り木をたどった鏡映回数の偶奇。±90°を越えたヒンジを、最寄りの平坦
/// 状態での1回の鏡映として数える。world法線やcameraを使わないため、紙全体を
/// 剛体回転しても値は変わらない。
pub(crate) fn mirrored_flags(forest: &Forest, n_faces: usize, angles_rad: &[f64]) -> Vec<bool> {
    let mut mirrored = vec![false; n_faces];
    for step in &forest.steps {
        mirrored[step.child] = mirrored[step.parent] ^ (angles_rad[step.hinge].cos() < 0.0);
    }
    mirrored
}

/// 完全に折り切った折り目がつなぐ2面の、厳密な上下(下の面, 上の面)を作る。
///
/// 180°に折られた折り目は、経路を解かなくても上下を決める。谷折り(角度が負)なら
/// 相手は基準面の紙の表側へ、山折り(正)なら紙の裏側へ来る。
///
/// この規則は `ori3-layers::flat_motion::want_kind` と同じで、実測とも一致する
/// (軸からの距離 0.5・角度 1.735° の折り目で `0.5*sin(1.735°)=0.0151385` に対し
/// 実測のずれ 0.01513728)。深度の差と違い、丸めでは壊れない。
///
/// 「上」の向きは `surface_order` が深度を測るのと同じ軸、つまり2面が乗る平面の
/// canonical法線(絶対値が最大の成分を正にした向き)で決める。**紙の表が世界の
/// どちらを向いているかを `mirrored` の偶奇で代用してはならない。** `mirrored` は
/// 平らな姿勢でしか世界の上下と対応せず、束が傾いた平面に乗る立体姿勢では逆になる。
/// 実測: `diagonal_midline_square` を edge43 で180°まで送った姿勢(裂け 4.0e-16)で、
/// 面12の紙の表は `(+0.9976, 0, -0.0692)`(canonical法線と同じ向き)なのに
/// `mirrored=true` であり、`mirrored` を使うと面12と面13の上下が、裂けていない
/// 179.999° の実深度(差 1.205e-6)と逆になっていた。
///
/// # 1本の折り線の上で向きが割れていたら、多い側にそろえる(2026-08-22)
///
/// 角度が0°の折り目でつながる面は、折れていない1枚の硬い紙(以下「塊」)である。
/// **同じ塊の組を、同じ直線の上でつなぐ `±180°` の折り目は、
/// どれも同じ上下を言わなければならない。** 2枚の硬い板を1本の線で折り重ねるのだから、
/// 線の一部だけが反対向きに折れていることは実際の紙では起こらない。
///
/// **「同じ直線の上で」という条件は外せない。** 同じ塊の組でも、**別々の直線**で
/// つながっているなら、上下が食い違ってよい。平らに畳んだ紙では、2つの塊が
/// 場所によって互い違いに重なる(片方の一部が上、別の一部が下)ことがあるからである。
/// 実測(カエル): 塊の組 `(8,9)`・`(9,10)`・`(36,39)`・`(133,139)`・`(138,139)` の
/// 5組は、**向きが直交する**折り線(向きの外積 1.0、直線からの外れ 0.354〜0.407)で
/// つながっており、上下が半々に割れていた。これは矛盾ではない。
///
/// 同じ直線の上で割れていたら、**本数の多い側の上下にそろえる**。少ない側の折り目には
/// 多い側と同じ上下を言わせる(拘束の上下を入れ替える)。**同数のときは、多い側が
/// 無いのでその折り線の拘束を1件も出さず、上下は実測の深度に決めさせる。**
///
/// 割れが1件も無ければ、多い側は元のままなので**出力は1件も変わらない**。
///
/// 実測(`crates/ori3-layers/tests/fixtures/folded-sample.ori3` の8手目):
/// 主対角 `x = y` 上の連続した11本だけが折れており、7本が `−180°`、4本が `+180°` で
/// 記録されていた。主対角以外の90本はすべて **ちょうど 0.0°**(再生・運動の解のどちらでも
/// 最大値が 0.0)なので、両半分はそれぞれ1つの塊になる。割れた4本がつなぐ面の組は
/// `(6,7)` と `(8,9)` の2組で、`derive_surface_order_with` が捨てていた深度拘束2件と
/// 1対1で一致していた。多い側(7本の `−180°`)へそろえると、上下は
/// すき間の実測どおり「面7が面6より上」「面9が面8より上」になり、
/// 捨てる深度拘束は0件、ぴったり重なる23組すべてで実測と一致する。
///
/// **少ない側が誤りである根拠**(この標本での実測): 主対角は手順1で全区間が `−180°`
/// (谷)に折られ、その後の折り返し技法が区間の一部だけを `+180°` へ書き換えた。
/// 手順7が「主対角以外を平らへ戻す」ときに、その書き換えが取り残された。
/// したがって少ない側は**技法の途中でだけ正しかった古い値**である。
/// **ただし「取り残される側が必ず少数になる」ことは、この仕組みからは出てこない。**
/// 多い側を採るのは、この標本で実測が一致する選び方であって、原因からの帰結ではない。
/// 同数のときに何も言わせないのは、そのためである。
pub(crate) fn exact_stack_constraints(
    forest: &Forest,
    faces: &[Face],
    angles_rad: &[f64],
    transforms: &[(DMat3, DVec3)],
) -> Vec<(FaceId, FaceId)> {
    let is_exact_stack = |angle: f64| {
        (angle.abs() - std::f64::consts::PI).abs() <= crate::surface_order::EXACT_FLAT_EPS_RAD
    };
    // `±180°` が1本も無ければ拘束は生まれない。塊を数える費用も払わない。
    if !angles_rad.iter().copied().any(is_exact_stack) {
        return Vec::new();
    }
    // 角度の符号は「基準面のCCW境界に取った軸」に対して定義されている。
    // 木辺では親面、非木辺では `LoopClosure::from` 側がその基準面である。
    // 別の面を基準にすると軸の向きが逆になり、上下が反転する。
    let mut reference_of = vec![None; forest.hinges.len()];
    for step in &forest.steps {
        reference_of[step.hinge] = Some(step.parent);
    }
    for closure in &forest.loops {
        reference_of[closure.hinge] = Some(closure.from);
    }
    let blocks = rigid_blocks(forest, faces.len(), angles_rad);
    // 折り線ごとの票: (番号の大きい塊が上と言った本数, 小さい塊が上と言った本数)。
    let mut lines: Vec<FoldLine> = Vec::new();
    // 折り目ごとの候補。同じ折り線に属する候補どうしで向きが割れていたら、
    // 本数の多い側へそろえる(同数ならその折り線の候補をまとめて捨てる)。
    let mut candidates: Vec<ExactStackCandidate> = Vec::new();
    for (hinge, occurrences) in forest.hinge_occ.iter().enumerate() {
        let Some(&angle) = angles_rad.get(hinge) else {
            continue;
        };
        if !is_exact_stack(angle) {
            continue;
        }
        let Some(Some(reference)) = reference_of.get(hinge).copied() else {
            continue;
        };
        let other = if occurrences[0].0 == reference {
            occurrences[1].0
        } else {
            occurrences[0].0
        };
        let (Some(reference_face), Some(other_face), Some(&(rotation, translation))) = (
            faces.get(reference),
            faces.get(other),
            transforms.get(reference),
        ) else {
            continue;
        };
        // 展開図の面は反時計回りなので、平らな紙の表は +z。剛体回転で写した
        // `rotation * z` が、いまの姿勢での基準面の紙の表の向きである。
        let paper_front = rotation * DVec3::Z;
        let up = crate::surface_order::canonical(paper_front);
        if !paper_front.is_finite() || !up.is_finite() {
            continue;
        }
        let front_points_up = paper_front.dot(up) > 0.0;
        let other_is_above = (angle < 0.0) == front_points_up;
        let (below, above) = if other_is_above {
            (reference, other)
        } else {
            (other, reference)
        };
        let constraint = if other_is_above {
            (reference_face.id, other_face.id)
        } else {
            (other_face.id, reference_face.id)
        };
        let (low, high) = (
            blocks[below].min(blocks[above]),
            blocks[below].max(blocks[above]),
        );
        if low == high {
            // 同じ塊の中で `±180°` に折れている記録は、塊の分け方そのものと食い違う。
            // 上下を「塊どうしの前後」として数えられないので、そろえる相手を持たせず、
            // これまでどおりそのまま拘束にする。
            candidates.push((None, true, constraint));
            continue;
        }
        // いまの姿勢での折り線(3D)。同じ塊の組を、同じ直線上でつなぐ折り目だけが
        // 「同じ向きでなければならない」仲間である。
        let (_, axis_a, axis_u) = if occurrences[0].0 == reference {
            occurrences[0]
        } else {
            occurrences[1]
        };
        let point = rotation * axis_a + translation;
        let direction = rotation * axis_u;
        if !point.is_finite() || !direction.is_finite() {
            candidates.push((None, true, constraint));
            continue;
        }
        let high_is_above = blocks[above] == high;
        let line = lines.iter().position(|line| {
            line.blocks == (low, high)
                && direction.cross(line.direction).length() <= SAME_FOLD_LINE_EPS
                && (point - line.point).cross(line.direction).length() <= SAME_FOLD_LINE_EPS
        });
        let line = line.unwrap_or_else(|| {
            lines.push(FoldLine {
                blocks: (low, high),
                point,
                direction,
                high_above: 0,
                low_above: 0,
            });
            lines.len() - 1
        });
        if high_is_above {
            lines[line].high_above += 1;
        } else {
            lines[line].low_above += 1;
        }
        candidates.push((Some(line), high_is_above, constraint));
    }
    candidates
        .into_iter()
        .filter_map(|(line, high_is_above, constraint)| {
            let Some(line) = line.map(|line| &lines[line]) else {
                return Some(constraint);
            };
            if line.high_above == line.low_above {
                // 同数。多い側が無いので、この折り線については上下を語らない。
                // 3Dでの折り目の長さのような「計算した小数」で決着させると、
                // 丸めで答えが変わり得る(CLAUDE.md §10.7.9)。本数だけで決める。
                return None;
            }
            // 多い側の上下へそろえる。少ない側は拘束の上下を入れ替える。
            let winner_high_above = line.high_above > line.low_above;
            Some(if winner_high_above == high_is_above {
                constraint
            } else {
                (constraint.1, constraint.0)
            })
        })
        .collect()
}

/// 「同じ塊の組を、同じ直線上でつなぐ折り目」とみなす許容差(紙の長辺 = 1)。
///
/// 実測(2026-08-22)。同じ折り線に乗っている折り目の組では、
/// 直線からの外れが最大 **3.854e-11**、向きの外積が最大 **1.994e-10** だった
/// (`folded-sample.ori3` の主対角11本は両方とも **0.0**、
/// カエルの3組は 5.1e-17〜3.9e-11)。
/// 別々の直線でつながっている組では、外れが最小 **0.3536**、外積が **1.0** だった
/// (カエルの5組。直交している)。
/// 両者は9桁以上離れているので、境目はその間のどこでもよい。
/// このリポジトリで「同じ平面」に使っている `COPLANAR_DISTANCE_EPS` と同じ
/// **1e-6** にそろえる。同じ直線の実測 3.854e-11 に対して約26,000倍、
/// 別の直線の実測 0.3536 に対して約1/354,000の余裕がある。
const SAME_FOLD_LINE_EPS: f64 = 1e-6;

/// 同じ塊の組を、同じ直線上でつなぐ折り目のまとまり。
struct FoldLine {
    /// 塊の組(小さい代表, 大きい代表)
    blocks: (usize, usize),
    /// 折り線上の1点(いまの姿勢での3D)
    point: DVec3,
    /// 折り線の向き(いまの姿勢での3D)
    direction: DVec3,
    /// 「番号の大きい塊が上」と言った折り目の本数
    high_above: usize,
    /// 「番号の小さい塊が上」と言った折り目の本数
    low_above: usize,
}

/// `exact_stack_constraints` が集める折り目1本ぶんの候補。
/// (折り線の添字(そろえる相手がいなければ `None`), 番号の大きい塊が上か,
/// 拘束(下の面, 上の面))。
type ExactStackCandidate = (Option<usize>, bool, (FaceId, FaceId));

/// 角度が0°の折り目でつながる面をまとめ、面添字→塊の代表添字を返す(union-find)。
///
/// 0°の折り目は紙を折っていないので、その両側は1枚の硬い平らな紙である。
///
/// 「0°」の許容差は、`±180°`を「折り切った」とみなすのに使っている
/// `EXACT_FLAT_EPS_RAD`(1e-8 rad = 5.73e-7 度)と同じ値にそろえる。実測では
/// `folded-sample.ori3` の8手目で、折れていない90本の角度はすべて **ちょうど 0.0 度**
/// (手順の再生でも `solve_motion` の解でも最大 0.0)なので、この境目には十分な余裕がある。
fn rigid_blocks(forest: &Forest, n_faces: usize, angles_rad: &[f64]) -> Vec<usize> {
    fn find(parent: &mut [usize], mut index: usize) -> usize {
        while parent[index] != index {
            parent[index] = parent[parent[index]];
            index = parent[index];
        }
        index
    }
    let mut parent = (0..n_faces).collect::<Vec<_>>();
    for (hinge, occurrences) in forest.hinge_occ.iter().enumerate() {
        let angle = angles_rad.get(hinge).copied().unwrap_or(0.0);
        if angle.abs() > crate::surface_order::EXACT_FLAT_EPS_RAD {
            continue;
        }
        let (left, right) = (occurrences[0].0, occurrences[1].0);
        if left >= n_faces || right >= n_faces {
            continue;
        }
        let (left_root, right_root) = (find(&mut parent, left), find(&mut parent, right));
        if left_root != right_root {
            parent[left_root] = right_root;
        }
    }
    (0..n_faces).map(|index| find(&mut parent, index)).collect()
}

/// 構築済みの森でヒンジ角(ラジアン、`Forest::hinges` と同順)を伝播し、
/// FoldedFrameを組み立てる(非連結の警告付与を含む)。solveが最終フレームの
/// 生成で `build_forest` を再実行しないための入口。
pub(crate) fn fold_frame(forest: &Forest, faces: &[Face], angles_rad: &[f64]) -> FoldedFrame {
    let tf = propagate_with(forest, faces.len(), angles_rad);
    let is_exact_stack = |angle: f64| {
        (angle.abs() - std::f64::consts::PI).abs() <= crate::surface_order::EXACT_FLAT_EPS_RAD
    };
    let has_surface_stack = angles_rad.iter().copied().any(is_exact_stack);
    let surface_path_transforms = if has_surface_stack {
        crate::surface_order::SURFACE_PATH_CHECKPOINT_DEG
            .iter()
            .map(|checkpoint| {
                let checkpoint = checkpoint.to_radians();
                let path_angles = angles_rad
                    .iter()
                    .map(|angle| angle.signum() * angle.abs().min(checkpoint))
                    .collect::<Vec<_>>();
                propagate_with(forest, faces.len(), &path_angles)
                    .into_iter()
                    .enumerate()
                    .map(|(index, transform)| (faces[index].id, transform))
                    .collect()
            })
            .collect()
    } else {
        Vec::new()
    };
    // 平坦状態の Isometry2::mirrored と同じ偶奇を、非平坦姿勢にも連続する
    // 「最寄りの平坦枝」として持たせる。world法線やcameraを使わないため、紙全体を
    // 剛体回転しても値は変わらない。山谷の符号によらず±180°は1回、±85°は0回。
    let mirrored = mirrored_flags(forest, faces.len(), angles_rad);
    let mut warnings = Vec::new();
    if forest.roots.len() > 1 {
        warnings.push(
            "展開図の面がひとつながりになっていません。離れた部分は元の場所を基準に表示します"
                .to_string(),
        );
    }
    let exact_stack_constraints = exact_stack_constraints(forest, faces, angles_rad, &tf);
    FoldedFrame {
        exact_stack_constraints,
        transforms: faces
            .iter()
            .enumerate()
            .map(|(i, f)| (f.id, tf[i]))
            .collect(),
        surface_path_transforms,
        has_surface_stack,
        has_loop_closures: !forest.loops.is_empty(),
        mirrored: faces
            .iter()
            .enumerate()
            .map(|(i, f)| (f.id, mirrored[i]))
            .collect(),
        warnings,
    }
}

/// 面隣接グラフのBFS全域木を作り、根面をxy平面に固定して、木辺のヒンジ角
/// (`angles`は度。載っていないヒンジは0度=平ら)で子面の姿勢を伝播する。
///
/// - 根面は決定的に選ぶ(連結成分ごとに最小のFaceId)
/// - 折り線で繋がっていない部分は警告を載せ、各部分の根を恒等変換のまま
///   その場で伝播する(処理は止めない)
pub fn propagate(cp: &CreasePattern, faces: &[Face], angles: &HashMap<EdgeId, f64>) -> FoldedFrame {
    let forest = build_forest(cp, faces);
    let rad: Vec<f64> = forest
        .hinges
        .iter()
        .map(|e| angles.get(e).copied().unwrap_or(0.0).to_radians())
        .collect();
    fold_frame(&forest, faces, &rad)
}

/// FoldedFrameを表示用のFrame3Dへ変換する。物理的な持ち上げに使うlayerは
/// M2(ori3-layers)で計算するため全面0とし、同一深度専用のsurface_rankは
/// 0.001°手前の実深度から決定的に求める。
pub fn to_frame3d(cp: &CreasePattern, faces: &[Face], frame: &FoldedFrame) -> Frame3D {
    to_frame3d_with_surface_order(cp, faces, frame, true)
}

/// 多角形・裏表・警告だけを使う呼び出し向けの [`to_frame3d`]。
///
/// 面の座標・`mirrored`・`warnings` は [`to_frame3d`] と同じ値になる。違うのは
/// `surface_rank` だけで、こちらは展開図の材質多角形から決まるseed順のままにする。
/// 重なり順を読まない用途(2つの姿勢の幾何が一致するかを確かめるだけ、など)で
/// 高価な重なり順の導出を走らせないための入口であり、
/// 重なり順が要る呼び出しは [`to_frame3d`] を使うこと。
pub fn to_frame3d_geometry_only(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &FoldedFrame,
) -> Frame3D {
    to_frame3d_with_surface_order(cp, faces, frame, false)
}

/// 全ヒンジ角からcompleteなcanonical surface順と、次手順へ渡す幾何provenanceを返す。
///
/// 前姿勢のprovenanceは、前姿勢でも現在姿勢でも正面積を共有する面対にだけ使われる。
/// 現姿勢のexact/depth制約が比較できる対ではそちらが優先される。幾何導出が不完全な
/// ときは順序だけを推測せず、provenanceも発行しない。
pub fn surface_order_from_angles(
    cp: &CreasePattern,
    faces: &[Face],
    angles: &HashMap<EdgeId, f64>,
    current_path: &[Frame3D],
    previous: Option<&crate::surface_order::SurfaceOrderProvenance>,
) -> Result<(Vec<FaceId>, crate::surface_order::SurfaceOrderProvenance), String> {
    let folded = propagate(cp, faces, angles);
    let frame = to_frame3d_with_surface_order(cp, faces, &folded, false);
    let derived = if current_path.is_empty() {
        // 履歴をまだ必要としない姿勢は、まず現在の実深度とexact制約だけで完結させる。
        // ここで不完全な場合だけ、replayが前endpointと実current-step pathを用意して
        // もう一度呼ぶ。generic flat→all-final経路をcurrent geometryとは扱わない。
        crate::surface_order::derive_surface_order_from_current_depths(
            cp,
            faces,
            &frame,
            &folded.exact_stack_constraints,
        )?
    } else {
        crate::surface_order::derive_surface_order_from_frame_path_with_previous(
            cp,
            faces,
            current_path,
            &frame,
            &folded.exact_stack_constraints,
            previous,
        )?
    };
    if !derived.complete {
        return Err(format!(
            "surface order is incomplete (unresolved={}, skipped={}, broken={})",
            derived.unresolved_overlaps, derived.skipped_pairs, derived.broken_constraints
        ));
    }
    if folded.has_loop_closures
        && derived.resolved_overlaps > 0
        && derived.sampled_depth_constraints == 0
    {
        return Err(
            "loop surface order needs an accepted current-path depth constraint".to_string(),
        );
    }
    // 現在の厳密折り目制約だけで全overlapを比較でき、深度制約との食い違いも無い
    // endpointは、それ自体がcompleteな幾何証明である。current depthやseedで補った
    // complete状態は履歴を要求し、replayの実current-step経路で決め直す。
    let exact_endpoint_is_authoritative = !folded.has_loop_closures
        && derived.exact_resolved_overlaps == derived.resolved_overlaps
        && derived.dropped_depth_constraints == 0;
    if current_path.is_empty() && derived.resolved_overlaps > 0 && !exact_endpoint_is_authoritative
    {
        return Err("surface order needs previous-step geometry for an overlap stack".to_string());
    }
    Ok((derived.order, derived.provenance))
}

/// 実current-step経路でもcompleteにならない場合の、平坦展開図からのcanonical経路。
///
/// replayは実経路を必ず先に試し、この経路を最後の幾何fallbackとしてだけ使う。
/// material seedや保存layerを物理順として発行せず、completeな導出だけを返す。
/// 非木辺を持つ紙ではcheckpointの単純伝播が閉包を保証しないため、このfallbackを
/// authorityにせず、solve済みの実current-step経路だけを使う。
pub fn surface_order_from_angles_flat_path(
    cp: &CreasePattern,
    faces: &[Face],
    angles: &HashMap<EdgeId, f64>,
) -> Result<(Vec<FaceId>, crate::surface_order::SurfaceOrderProvenance), String> {
    let folded = propagate(cp, faces, angles);
    let frame = to_frame3d_with_surface_order(cp, faces, &folded, false);
    let derived = crate::surface_order::derive_surface_order(
        cp,
        faces,
        &folded.surface_path_transforms,
        &folded.transforms,
        &frame,
        &folded.exact_stack_constraints,
        None,
    )?;
    if !derived.complete {
        return Err(format!(
            "flat-path surface order is incomplete (unresolved={}, skipped={}, broken={})",
            derived.unresolved_overlaps, derived.skipped_pairs, derived.broken_constraints
        ));
    }
    if folded.has_loop_closures && derived.resolved_overlaps > 0 {
        return Err("loop surface order cannot use a propagated flat-path fallback".to_string());
    }
    Ok((derived.order, derived.provenance))
}

/// replayのように直後に保存済み順序を刻印する呼出し向け。幾何と警告は同一で、
/// 高価な完全重なり順の導出だけを省く。
pub(crate) fn to_frame3d_with_surface_order(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &FoldedFrame,
    derive_surface_order: bool,
) -> Frame3D {
    let vpos = vertex_positions(cp);
    let faces3d = faces
        .iter()
        .map(|f| {
            let (r, t) = frame
                .transforms
                .get(&f.id)
                .copied()
                .unwrap_or((DMat3::IDENTITY, DVec3::ZERO));
            Face3D {
                face: f.id,
                polygon: f
                    .vertices
                    .iter()
                    .filter_map(|v| vpos.get(v))
                    .map(|&p| {
                        let q = r * p + t;
                        [q.x, q.y, q.z]
                    })
                    .collect(),
                layer: 0,
                surface_rank: 0,
                mirrored: frame.mirrored.get(&f.id).copied().unwrap_or(false),
            }
        })
        .collect();
    let mut output = Frame3D {
        faces: faces3d,
        warnings: frame.warnings.clone(),
    };
    // 実面積で重ならない面どうしのtieにも、紙と無関係なFaceIdを使わない。
    // 重なる全対を制約閉包で比較できたときだけ導出順を採り、不完全な経路では
    // 展開図の材質多角形だけから決まるseedを残す。
    if let Ok(seed_order) = crate::surface_order::geometric_seed_order(cp, faces) {
        let order = if derive_surface_order && frame.has_surface_stack {
            crate::surface_order::derive_surface_order(
                cp,
                faces,
                &frame.surface_path_transforms,
                &frame.transforms,
                &output,
                &frame.exact_stack_constraints,
                None,
            )
            .ok()
            .filter(|derived| derived.complete)
            .map_or(seed_order, |derived| derived.order)
        } else {
            seed_order
        };
        crate::surface_order::stamp_surface_order(&mut output, &order)
            .expect("rigid surface order contains the same faces as its frame");
    }
    output
}

#[cfg(test)]
mod exact_stack_sign_tests {
    use super::*;
    use ori3_cp::extract_faces;
    use ori3_model::{Edge, EdgeKind, Vertex};

    fn vertex(id: u32, x: f64, y: f64) -> Vertex {
        Vertex { id, pos: [x, y] }
    }

    fn edge(id: u32, v0: u32, v1: u32, kind: EdgeKind) -> Edge {
        Edge { id, v0, v1, kind }
    }

    /// 主対角を2本に割った正方形。両半分は、それぞれ折り目1本を平ら(0°)に保つことで
    /// 「1枚の硬い紙(塊)」になる。折り目10・11は**同じ塊の組**をつなぐ。
    ///
    /// ```text
    ///   3(0,1)------------2(1,1)
    ///     |             /  |
    ///   6(0,.5)---4(.5,.5)-5(1,.5)   4-5 と 6-4 は 0°(平ら)
    ///     |    /            |
    ///   0(0,0)------------1(1,0)     0-4 と 4-2 が主対角の折り目
    /// ```
    fn two_segment_diagonal_square() -> CreasePattern {
        CreasePattern {
            vertices: vec![
                vertex(0, 0.0, 0.0),
                vertex(1, 1.0, 0.0),
                vertex(2, 1.0, 1.0),
                vertex(3, 0.0, 1.0),
                vertex(4, 0.5, 0.5),
                vertex(5, 1.0, 0.5),
                vertex(6, 0.0, 0.5),
            ],
            edges: vec![
                edge(0, 0, 1, EdgeKind::Border),
                edge(1, 1, 5, EdgeKind::Border),
                edge(2, 5, 2, EdgeKind::Border),
                edge(3, 2, 3, EdgeKind::Border),
                edge(4, 3, 6, EdgeKind::Border),
                edge(5, 6, 0, EdgeKind::Border),
                edge(10, 0, 4, EdgeKind::Valley),
                edge(11, 4, 2, EdgeKind::Valley),
                edge(12, 4, 5, EdgeKind::Valley),
                edge(13, 6, 4, EdgeKind::Valley),
            ],
            next_vertex_id: 7,
            next_edge_id: 14,
        }
    }

    /// 主対角を**3本**に割った正方形。両半分はやはり折り目1本を平ら(0°)に保って
    /// 塊になる。折り目20・21・22が同じ塊の組をつなぐので、2対1の多い側を試せる。
    ///
    /// ```text
    ///   3(0,1)-------------2(1,1)
    ///     |            /5(2/3,2/3)
    ///   7(0,.5)-------/     |        5-7 と 4-6 は 0°(平ら)
    ///     |    4(1/3,1/3)---6(1,.5)
    ///   0(0,0)-------------1(1,0)    0-4・4-5・5-2 が主対角の折り目
    /// ```
    fn three_segment_diagonal_square() -> CreasePattern {
        let third = 1.0 / 3.0;
        CreasePattern {
            vertices: vec![
                vertex(0, 0.0, 0.0),
                vertex(1, 1.0, 0.0),
                vertex(2, 1.0, 1.0),
                vertex(3, 0.0, 1.0),
                vertex(4, third, third),
                vertex(5, 2.0 * third, 2.0 * third),
                vertex(6, 1.0, 0.5),
                vertex(7, 0.0, 0.5),
            ],
            edges: vec![
                edge(0, 0, 1, EdgeKind::Border),
                edge(1, 1, 6, EdgeKind::Border),
                edge(2, 6, 2, EdgeKind::Border),
                edge(3, 2, 3, EdgeKind::Border),
                edge(4, 3, 7, EdgeKind::Border),
                edge(5, 7, 0, EdgeKind::Border),
                edge(20, 0, 4, EdgeKind::Valley),
                edge(21, 4, 5, EdgeKind::Valley),
                edge(22, 5, 2, EdgeKind::Valley),
                edge(30, 4, 6, EdgeKind::Valley),
                edge(31, 5, 7, EdgeKind::Valley),
            ],
            next_vertex_id: 8,
            next_edge_id: 32,
        }
    }

    /// 2×2に割った正方形。折り目40(下の縦)と43(右の横)を平ら(0°)にすると、
    /// 左下・右下・右上の3面が1つの塊になり、左上の1面だけが別の塊になる。
    /// 残る折り目41(上の縦)と42(左の横)は、**同じ塊の組を、直交する2本の直線で**つなぐ。
    ///
    /// ```text
    ///   6(0,1)----5(.5,1)----4(1,1)
    ///     |   41(8-5)|          |
    ///   7(0,.5)---8(.5,.5)---3(1,.5)   42=7-8(左の横)  43=8-3(右の横、0°)
    ///     |          |40(1-8)   |
    ///   0(0,0)----1(.5,0)----2(1,0)
    /// ```
    fn perpendicular_lines_square() -> CreasePattern {
        CreasePattern {
            vertices: vec![
                vertex(0, 0.0, 0.0),
                vertex(1, 0.5, 0.0),
                vertex(2, 1.0, 0.0),
                vertex(3, 1.0, 0.5),
                vertex(4, 1.0, 1.0),
                vertex(5, 0.5, 1.0),
                vertex(6, 0.0, 1.0),
                vertex(7, 0.0, 0.5),
                vertex(8, 0.5, 0.5),
            ],
            edges: vec![
                edge(0, 0, 1, EdgeKind::Border),
                edge(1, 1, 2, EdgeKind::Border),
                edge(2, 2, 3, EdgeKind::Border),
                edge(3, 3, 4, EdgeKind::Border),
                edge(4, 4, 5, EdgeKind::Border),
                edge(5, 5, 6, EdgeKind::Border),
                edge(6, 6, 7, EdgeKind::Border),
                edge(7, 7, 0, EdgeKind::Border),
                edge(40, 1, 8, EdgeKind::Valley),
                edge(41, 8, 5, EdgeKind::Valley),
                edge(42, 7, 8, EdgeKind::Valley),
                edge(43, 8, 3, EdgeKind::Valley),
            ],
            next_vertex_id: 9,
            next_edge_id: 44,
        }
    }

    fn constraints_of(
        cp: &CreasePattern,
        expected_faces: usize,
        angles_deg: &[(EdgeId, f64)],
    ) -> Vec<(FaceId, FaceId)> {
        let faces = extract_faces(cp);
        assert_eq!(faces.len(), expected_faces, "標本の面の数が変わっている");
        let angles = angles_deg.iter().copied().collect::<HashMap<EdgeId, f64>>();
        let forest = build_forest(cp, &faces);
        let angles_rad = forest
            .hinges
            .iter()
            .map(|hinge| angles.get(hinge).copied().unwrap_or(0.0).to_radians())
            .collect::<Vec<_>>();
        let transforms = propagate_with(&forest, faces.len(), &angles_rad);
        exact_stack_constraints(&forest, &faces, &angles_rad, &transforms)
    }

    fn two_segment_constraints(angles_deg: &[(EdgeId, f64)]) -> Vec<(FaceId, FaceId)> {
        constraints_of(&two_segment_diagonal_square(), 4, angles_deg)
    }

    fn three_segment_constraints(angles_deg: &[(EdgeId, f64)]) -> Vec<(FaceId, FaceId)> {
        constraints_of(&three_segment_diagonal_square(), 4, angles_deg)
    }

    fn blocks_of(cp: &CreasePattern, angles_deg: &[(EdgeId, f64)]) -> (Vec<Face>, Vec<usize>) {
        let faces = extract_faces(cp);
        let angles = angles_deg.iter().copied().collect::<HashMap<EdgeId, f64>>();
        let forest = build_forest(cp, &faces);
        let angles_rad = forest
            .hinges
            .iter()
            .map(|hinge| angles.get(hinge).copied().unwrap_or(0.0).to_radians())
            .collect::<Vec<_>>();
        let blocks = rigid_blocks(&forest, faces.len(), &angles_rad);
        (faces, blocks)
    }

    /// 同じ塊の組をつなぐ2本の折り目が同じ向きなら、これまでどおり2件とも発行し、
    /// 2件は同じ上下(同じ塊が上)を言う。矛盾の無い作品で拘束が変わらないことの下支え。
    #[test]
    fn two_creases_on_one_fold_line_that_agree_keep_both_exact_constraints() {
        for sign in [1.0_f64, -1.0] {
            let angles = [
                (10_u32, sign * 180.0),
                (11, sign * 180.0),
                (12, 0.0),
                (13, 0.0),
            ];
            let constraints = two_segment_constraints(&angles);
            assert_eq!(
                constraints.len(),
                2,
                "sign={sign}: 折り目2本ぶんの厳密な上下がそのまま出る"
            );
            let (faces, blocks) = blocks_of(&two_segment_diagonal_square(), &angles);
            let index_of = |id: FaceId| {
                faces
                    .iter()
                    .position(|face| face.id == id)
                    .expect("拘束の面はこの標本の面である")
            };
            let above_blocks = constraints
                .iter()
                .map(|&(_, above)| blocks[index_of(above)])
                .collect::<Vec<_>>();
            let below_blocks = constraints
                .iter()
                .map(|&(below, _)| blocks[index_of(below)])
                .collect::<Vec<_>>();
            assert_eq!(
                above_blocks[0], above_blocks[1],
                "sign={sign}: 2本とも同じ塊が上だと言う"
            );
            assert_eq!(below_blocks[0], below_blocks[1]);
            assert_ne!(
                below_blocks[0], above_blocks[0],
                "sign={sign}: 上と下は別の塊である"
            );
        }
    }

    /// 同じ塊の組をつなぐ3本のうち1本だけ向きが逆なら、**多い側(2本)へそろえる**。
    /// 結果は「3本とも多い側の向きで記録されていた場合」と**1件も違わない**。
    ///
    /// 実測(`folded-sample.ori3` の8手目)では、主対角11本のうち7本が `−180°`、
    /// 4本が `+180°` で、多い側(7本)が、すき間の実測が示す上下と一致していた。
    #[test]
    fn one_crease_against_the_majority_is_turned_to_the_majority_side() {
        for sign in [1.0_f64, -1.0] {
            let unanimous = [
                (20_u32, sign * 180.0),
                (21, sign * 180.0),
                (22, sign * 180.0),
                (30, 0.0),
                (31, 0.0),
            ];
            let expected = three_segment_constraints(&unanimous);
            assert_eq!(
                expected.len(),
                3,
                "sign={sign}: 折り目3本ぶんの厳密な上下が出る"
            );
            for odd_one_out in [20_u32, 21, 22] {
                let mixed = unanimous
                    .iter()
                    .map(|&(hinge, angle)| {
                        if hinge == odd_one_out {
                            (hinge, -angle)
                        } else {
                            (hinge, angle)
                        }
                    })
                    .collect::<Vec<_>>();
                let actual = three_segment_constraints(&mixed);
                assert_eq!(
                    actual, expected,
                    "sign={sign} 折り目{odd_one_out}だけ逆: 多い側2本へそろえた結果が、3本とも多い側だった場合と一致しない"
                );
            }
        }
    }

    /// 多い側と少ない側が**同数**なら、多い側が無いので上下を1件も語らない。
    /// 上下は実測の深度に決めさせる。3Dでの折り目の長さのような計算した小数で
    /// 決着させると、丸めで答えが変わり得るため使わない(CLAUDE.md §10.7.9)。
    #[test]
    fn an_even_split_says_nothing_about_the_order_of_that_block_pair() {
        for (first, second) in [(180.0_f64, -180.0_f64), (-180.0, 180.0)] {
            let constraints =
                two_segment_constraints(&[(10, first), (11, second), (12, 0.0), (13, 0.0)]);
            assert!(
                constraints.is_empty(),
                "({first}, {second}): 同数の割れから厳密な上下を出してはいけない: {constraints:?}"
            );
        }
    }

    /// 向きが割れていても、**塊が違えば**そろえない。折り目12・13を平らでない角度に
    /// すると両半分は1枚の硬い紙ではなくなり、折り目10・11は別々の塊の組をつなぐ。
    /// 割れの判定を、無関係な折り目まで巻き込む形にしないための歯止め。
    #[test]
    fn creases_between_different_rigid_blocks_keep_their_constraints_even_with_opposite_signs() {
        let constraints =
            two_segment_constraints(&[(10, 180.0), (11, -180.0), (12, 90.0), (13, -90.0)]);
        assert_eq!(
            constraints.len(),
            2,
            "別々の塊の組をつなぐ折り目は、向きが違っても両方とも残る: {constraints:?}"
        );
    }

    /// 同じ塊の組でも、**別々の直線**でつながっている折り目は、向きが食い違っても
    /// そろえない。平らに畳んだ紙では、2つの塊が場所によって互い違いに重なることが
    /// あるからである。
    ///
    /// 実測(カエルの受け入れ検査、2026-08-22): 塊の組 `(8,9)`・`(9,10)`・`(36,39)`・
    /// `(133,139)`・`(138,139)` の5組が、**向きが直交する**折り線(向きの外積 1.0、
    /// 直線からの外れ 0.354〜0.407)でつながり、上下がちょうど半々に割れていた。
    /// 直線の条件を入れないと、この5組の厳密な上下をすべて失う。
    #[test]
    fn creases_on_different_fold_lines_keep_their_constraints_even_with_opposite_signs() {
        let cp = perpendicular_lines_square();
        let mut disagreeing = 0_usize;
        for (upper, left) in [
            (180.0_f64, 180.0_f64),
            (180.0, -180.0),
            (-180.0, 180.0),
            (-180.0, -180.0),
        ] {
            let angles = [(41_u32, upper), (42, left), (40, 0.0), (43, 0.0)];
            let constraints = constraints_of(&cp, 4, &angles);
            assert_eq!(
                constraints.len(),
                2,
                "({upper}, {left}): 別々の直線でつながる折り目は両方とも残る: {constraints:?}"
            );
            let (faces, blocks) = blocks_of(&cp, &angles);
            let index_of = |id: FaceId| {
                faces
                    .iter()
                    .position(|face| face.id == id)
                    .expect("拘束の面はこの標本の面である")
            };
            let above = constraints
                .iter()
                .map(|&(_, above)| blocks[index_of(above)])
                .collect::<Vec<_>>();
            if above[0] != above[1] {
                disagreeing += 1;
            }
        }
        assert!(
            disagreeing >= 1,
            "この標本は、2本が別の塊を上だと言う符号の組み合わせを必ず含む"
        );
    }

    /// `±180°` が1本も無ければ拘束は空。塊を数える処理を足しても変わらない。
    #[test]
    fn a_pattern_without_any_fully_folded_crease_has_no_exact_constraints() {
        let constraints = two_segment_constraints(&[(10, 90.0), (11, 90.0), (12, 0.0), (13, 0.0)]);
        assert!(constraints.is_empty());
    }
}
