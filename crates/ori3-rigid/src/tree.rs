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
    /// 根面から折り木をたどった鏡映回数の偶奇。
    /// ±90°を越えたヒンジを、最寄りの平坦状態での1回の鏡映として数える。
    pub mirrored: HashMap<FaceId, bool>,
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
    let mut mirrored = vec![false; faces.len()];
    for step in &forest.steps {
        mirrored[step.child] = mirrored[step.parent] ^ (angles_rad[step.hinge].cos() < 0.0);
    }
    let mut warnings = Vec::new();
    if forest.roots.len() > 1 {
        warnings.push(
            "展開図の面がひとつながりになっていません。離れた部分は元の場所を基準に表示します"
                .to_string(),
        );
    }
    FoldedFrame {
        transforms: faces
            .iter()
            .enumerate()
            .map(|(i, f)| (f.id, tf[i]))
            .collect(),
        surface_path_transforms,
        has_surface_stack,
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
    let mut previous_order = faces.iter().map(|face| face.id).collect::<Vec<_>>();
    previous_order.sort_unstable();
    let order = if derive_surface_order && frame.has_surface_stack {
        crate::surface_order::derive_surface_order(
            faces,
            &frame.surface_path_transforms,
            &frame.transforms,
            &output,
            &previous_order,
        )
        .unwrap_or(previous_order)
    } else {
        previous_order
    };
    crate::surface_order::stamp_surface_order(&mut output, &order)
        .expect("rigid surface order contains the same faces as its frame");
    output
}
