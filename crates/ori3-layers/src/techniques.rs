//! 基本技法のマクロ(段折り・中割り折り・かぶせ折り)。
//!
//! 全ての技法は [`fold_through`] の繰り返しと層順序の並べ替えの合成として実装し、
//! 専用のデータ構造を持たない(要件§12のリスク対策: 技法がうまく当てはまらない形でも、
//! 利用者は同じことを手動の折り操作で行える)。
//! [`fold_through`] 自身は汎用の折り操作([`crate::flat_motion`])の包み紙なので、
//! ここも間接的にその上に乗っている。
//!
//! # 手順の記録
//!
//! 手順再生は「そのステップが駆動する全ての折り線」を [`DriverLine`] として必要とする
//! (未指定の折り線は0°に固定される)。そのため技法が生成した折り線は、内部で
//! 何回 [`fold_through`] を呼んでも1つの [`FoldStep`] にまとめて記録する。
//! 技法の中で裏返った部分(段折りの段の中・中割り折りの先端)にもとからあった
//! 折り線は山谷が入れ替わるので、その分もDriverLineとして記録する。
//!
//! # 座標系
//!
//! 入力の座標は全て「畳んだ平面座標」(3D表示のxy)。[`fold_through`] は結果を
//! 「根面(最小面ID)が恒等」の座標系へそろえ直すため、1回折るごとに畳み平面の
//! 座標が全体の等長変換だけずれる。2本目以降の折り線はこのずれを打ち消してから
//! 渡す([`Session::fold`] が返す変換を使う)。
//!
//! # 既知の制限
//!
//! - 層順序は [`fold_through`] の近似(動いた層を山全体の上/下へまとめる)の上に、
//!   先端をもとの層の隣へ置き直して決める([`Session::reorder_tips`])
//! - 重なりの一部だけを選んだフラップなど、紙が裂ける指定・折り上がりの山谷と
//!   重なり順が食い違う指定は、断らずに警告して続ける(「止めずに警告」原則)
//! - 開いてつぶす(squash)・花弁折り(petal)はまだマクロにしていない。既存の
//!   折り目を**開く**動きが要るため [`fold_through`] だけでは組めないが、
//!   汎用の折り操作([`crate::flat_motion`])は開く動き・回転・層ごとに逆向きの
//!   回し方をすべて表せるので、その上のマクロとして書ける

use std::collections::{HashMap, HashSet};

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_geometry::Isometry2;
use ori3_model::{
    CreasePattern, DriverLine, EPS, EdgeId, EdgeKind, FaceId, FoldStep, TechniqueKind, VertexId,
};

use crate::flat_state::{FlatState, point_in_face, representative_point};
use crate::fold_through::{
    FoldDirection, FoldThroughInput, FoldThroughResult, TEAR_MARK, angle_of, faces_by_edge,
    fold_through, push_driver_line, vertex_positions,
};

/// 面の配置の一致を見る許容誤差(等長変換の積み重ねで出る誤差より十分大きく取る)。
const JOIN_EPS: f64 = 1e-6;

/// 技法の共通入力。座標は全て「畳んだ平面座標」(3D表示のxy)。
#[derive(Clone, Debug)]
pub struct TechniqueInput {
    /// 対象フラップ(畳み平面で選んだ層の面ID)。
    /// 段折りでは空を許し、その場合は折り線の可動側に掛かる全ての層を折る。
    pub flap: Vec<FaceId>,
    /// 折り線(2点。無限直線として扱う)
    pub line: [[f64; 2]; 2],
    /// 技法ごとに意味の変わる基準点(各関数のdocを参照)
    pub reference_point: [f64; 2],
}

/// 段折り(平行な2本の折り線で山・谷を交互に折る)。
///
/// `line` が1本目の折り線、`reference_point` は2本目の折り線の位置を示す点
/// (1本目に平行で `reference_point` を通る直線が2本目)。1本目の可動側は
/// `reference_point` のある側(段になる部分と、その先の紙が動く)。
///
/// 2回の [`fold_through`] の合成で、`reference_point` の先の紙は段の幅の2倍だけずれる。
/// どちらの折りも「動く側を上へ回す」(Up)。段の中は1本目で裏返っているので、
/// 2本目の折り線は展開図では1本目と反対の線種になり、山谷が交互に並ぶ。
/// 重なりは下から「もとの紙・段の中・その先」の順(紙を横から見た Z 字)。
/// 段の中(2本の折り線に挟まれた帯)にもとからあった折り目は、裏返ると同時に
/// 重なり順も入れ替わるため、山谷は変わらない
/// (詳しくは [`Session::fix_reversed_creases`])。
///
/// `flap` が空なら折り線の可動側に掛かる全ての層を折る。
pub fn pleat(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &TechniqueInput,
) -> Result<FoldThroughResult, String> {
    let (l0, l1) = line_points(input.line)?;
    let u = (l1 - l0).normalize();
    let r = DVec2::from(input.reference_point);
    let gap = u.perp_dot(r - l0);
    if gap.abs() <= EPS {
        return Err(
            "段の幅が0です。2本目の折り線の位置を1本目から離して指定してください".to_string(),
        );
    }

    let mut s = Session::new(cp, faces, state)?;
    let flap = s.resolve_flap(&input.flap, "段折り")?;

    // 1本目: reference_point のある側(段とその先)が動く。動かさない側は反対側。
    let normal = DVec2::new(-u.y, u.x);
    let keep1 = r - normal * (gap * 2.0);
    let targets1 = s.movable_targets(input.line, [keep1.x, keep1.y], flap.as_deref());
    let m1 = s.fold(
        input.line,
        [keep1.x, keep1.y],
        targets1.as_deref(),
        FoldDirection::Up,
    )?;

    // 2本目: 1本目で動いた側にあるので、1本目の鏡映を掛けてから新しい平面座標へ写す。
    let moved_map = m1.compose(&Isometry2::reflection(l0, l1));
    let line2 = [moved_map.apply(r), moved_map.apply(r + u)];
    let line2 = [[line2[0].x, line2[0].y], [line2[1].x, line2[1].y]];
    // 2本目の動かさない側=2本の折り線に挟まれた帯(こちらも1本目で動いている)
    let keep2 = moved_map.apply(r - normal * (gap * 0.5));
    // 2本目で折るのは1本目で動いた紙だけ。動かなかった紙(もとの位置に残っている
    // 層)は、2本目の折り線の可動側に掛かっていても折ってはいけない
    let flap2 = s.moved_faces();
    let targets2 = s.movable_targets(line2, [keep2.x, keep2.y], Some(&flap2));
    s.fold(
        line2,
        [keep2.x, keep2.y],
        targets2.as_deref(),
        turn_direction(FoldDirection::Up, m1.mirrored),
    )?;

    // 段の中の折り目を、動いた後の重なり順に合わせ直す
    s.fix_reversed_creases();
    s.finish(TechniqueKind::Pleat, "段折り", cp)
}

/// 中割り折り(フラップの先端を内側へ折り込む)。
///
/// `reference_point` は折り込む先(先端が向かう側)を示す点で、その側は動かない。
/// フラップは2層以上で、折り線が全ての層を横切っている必要がある。
///
/// 折り線が横切る折り目(背)でつながった2つの層は、先端どうしがつながったまま
/// 裏返るので必ず反対向きに回る。この関係で層を2群に塗り分け
/// ([`Session::split_flap_by_connection`])、「下へ回す層」「上へ回す層」の
/// 2回に分けて [`fold_through`] を呼ぶ。層の数が奇数でも、重なりの一部だけを
/// 選んだ場合でも、紙のつながりどおりに折れる。
/// 展開図には同じ線種の折り線が2本(背を挟んでV字に)入る。
/// そのあと先端をもとの層の隣(中割りでは層の**内側**)へ置き直し、
/// 先端の中の背の山谷を新しい重なり順に合わせ直す
/// (中割り折りで背の向きが入れ替わるのはこのため)。
pub fn inside_reverse(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &TechniqueInput,
) -> Result<FoldThroughResult, String> {
    reverse_fold(cp, faces, state, input, true)
}

/// かぶせ折り(フラップの先端を外側へかぶせる)。中割り折りの逆。
///
/// `reference_point` はかぶせる先(先端が向かう側)を示す点で、その側は動かない。
/// いちばん手前(上)の層の先端が上へ回り、つながった層はその反対向きに回るので、
/// 先端はフラップの層の**外側**を包む。層の追い方は [`inside_reverse`] と同じ。
pub fn outside_reverse(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &TechniqueInput,
) -> Result<FoldThroughResult, String> {
    reverse_fold(cp, faces, state, input, false)
}

/// 中割り折り/かぶせ折りの本体(違いは折る向きと、先端を層の内側へ入れるか外側へ出すか)。
fn reverse_fold(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &TechniqueInput,
    inside: bool,
) -> Result<FoldThroughResult, String> {
    let name = if inside { "中割り折り" } else { "かぶせ折り" };
    let (l0, l1) = line_points(input.line)?;
    let u = (l1 - l0).normalize();
    let keep = DVec2::from(input.reference_point);
    if u.perp_dot(keep - l0).abs() <= EPS {
        return Err(format!(
            "{name}の向きを示す点が折り線の上にあります。先端を{}側の点を指してください",
            if inside { "折り込む先の" } else { "かぶせる先の" }
        ));
    }

    let mut s = Session::new(cp, faces, state)?;
    // フラップの面を層順(下→上)に並べる
    let flap = s.flap_in_layer_order(&input.flap, name)?;
    if flap.len() < 2 {
        return Err(format!(
            "{name}にはフラップが2層以上必要です。重なった層をまとめて選んでください"
        ));
    }
    for &id in &flap {
        if !s.crosses(id, input.line) {
            return Err(format!(
                "このフラップには{name}ができません。折り線がフラップを横切っていないか確認してください"
            ));
        }
    }

    // 紙のつながりから、先端を上へ回す層と下へ回す層に塗り分ける。
    // 折り線が横切る折り目(背)でつながった2層は、先端どうしがつながったまま
    // 裏返るので、必ず反対向きに回る。層の数を機械的に半分に割ると、奇数層や
    // 一部だけを選んだフラップで紙のつながりと食い違ってしまう。
    let up = s.split_flap_by_connection(&flap, input.line, inside, name)?;
    let (up_faces, down_faces): (Vec<FaceId>, Vec<FaceId>) =
        flap.iter().partition(|id| up[id]);

    // 下へ回す層と上へ回す層をそれぞれ1回の折りにまとめる(どちらかが空になる
    // 指定=つながっていない層だけを選んだ場合は、その回を飛ばす)。
    let keep_point = [keep.x, keep.y];
    let mut line_now = input.line;
    let mut keep_now = keep_point;
    let mut turned = false;
    for (targets, direction) in [
        (&down_faces, FoldDirection::Down),
        (&up_faces, FoldDirection::Up),
    ] {
        let now = s.descendants(targets);
        if now.is_empty() {
            continue;
        }
        let m = s.fold(line_now, keep_now, Some(&now), turn_direction(direction, turned))?;
        // 折り線と動かさない側の点は動かない側の幾何なので、平面座標のずれだけを打ち消す
        let a = m.apply(DVec2::from(line_now[0]));
        let b = m.apply(DVec2::from(line_now[1]));
        line_now = [[a.x, a.y], [b.x, b.y]];
        let k = m.apply(DVec2::from(keep_now));
        keep_now = [k.x, k.y];
        turned = turned != m.mirrored;
    }

    // 先端を、それぞれが回った向きの側(上へ回った層は元の層のすぐ上、
    // 下へ回った層はすぐ下)へ置き直し、先端の中の折り目(層と層をつなぐ背)の
    // 山谷を新しい重なり順に合わせ直す。
    // 折るたびに畳み平面ごと裏返ることがあり、そのときは重なり順の上下も
    // 入れ替わっているので、置く側も入れ替える(turn_directionと同じ事情)。
    s.reorder_tips(&up, turned);
    s.fix_reversed_creases();

    let kind = if inside {
        TechniqueKind::InsideReverse
    } else {
        TechniqueKind::OutsideReverse
    };
    s.finish(kind, name, cp)
}

// ---------------------------------------------------------------------------
// 技法の作業場: fold_throughを繰り返しながら、面の由来・裏返りの偶奇を追う
// ---------------------------------------------------------------------------

/// 技法の途中経過。CPは複製の上で書き換え、成功したときだけ [`Session::finish`] で
/// 呼び出し側のCPへ反映する(原子性: 途中で失敗しても元のCPは変わらない)。
struct Session {
    cp: CreasePattern,
    faces: Vec<Face>,
    state: FlatState,
    /// 現在の面ID → 技法開始時のどの面の子孫か
    origin: HashMap<FaceId, FaceId>,
    /// 現在の面ID → 技法の中で裏返った回数の偶奇(trueなら裏返っている)
    flipped: HashMap<FaceId, bool>,
    drivers: Vec<DriverLine>,
    added: Vec<EdgeId>,
    warnings: Vec<String>,
}

impl Session {
    fn new(cp: &CreasePattern, faces: &[Face], state: &FlatState) -> Result<Session, String> {
        for f in faces {
            if !state.placements.contains_key(&f.id) {
                return Err(format!("面 {} の配置が平坦状態に見つかりません", f.id));
            }
        }
        Ok(Session {
            cp: cp.clone(),
            faces: faces.to_vec(),
            state: state.clone(),
            origin: faces.iter().map(|f| (f.id, f.id)).collect(),
            flipped: faces.iter().map(|f| (f.id, false)).collect(),
            drivers: Vec::new(),
            added: Vec::new(),
            warnings: Vec::new(),
        })
    }

    /// 1回の [`fold_through`]。戻り値は「この折りで動かなかった側の点を、
    /// 折った後の畳み平面座標へ移す等長変換」(次の折り線を渡すときに使う)。
    ///
    /// `targets` が `None` なら可動側に掛かる全ての層が対象。
    ///
    /// 途中の折りで出る「紙が裂けます」の警告は捨てる。技法は複数回の折りで
    /// 1つの形を作るため、1回目だけを見ると必ず層のつながりが切れて見える
    /// (最終形での裂けは [`Session::tear_warnings`] で改めて調べる)。
    fn fold(
        &mut self,
        line: [[f64; 2]; 2],
        keep_side_point: [f64; 2],
        targets: Option<&[FaceId]>,
        direction: FoldDirection,
    ) -> Result<Isometry2, String> {
        let old_cp = self.cp.clone();
        let old_faces = self.faces.clone();
        let old_state = self.state.clone();
        let res = fold_through(
            &mut self.cp,
            &old_faces,
            &old_state,
            &FoldThroughInput {
                line,
                keep_side_point,
                target_layers: targets.map(<[FaceId]>::to_vec),
                direction,
            },
        )?;

        let (l0, l1) = line_points(line)?;
        let u = (l1 - l0).normalize();
        let keep_sign = u.perp_dot(DVec2::from(keep_side_point) - l0).signum();
        let signed = |q: DVec2| keep_sign * u.perp_dot(q - l0);
        let refl = Isometry2::reflection(l0, l1);

        let new_faces = extract_faces(&self.cp);
        let mut origin: HashMap<FaceId, FaceId> = HashMap::with_capacity(new_faces.len());
        let mut flipped: HashMap<FaceId, bool> = HashMap::with_capacity(new_faces.len());
        // 動かなかった側の平面座標のずれ(根面をそろえ直す変換)。
        // 動いた面からも鏡映を打ち消せば同じものが求まる。
        let mut plane_map: Option<Isometry2> = None;
        for nf in &new_faces {
            let r = representative_point(&self.cp, nf);
            let parent = old_faces.iter().find(|f| point_in_face(&old_cp, f, r));
            let Some(pf) = parent else {
                // 親を特定できない面(fold_throughが警告済み)。由来を引き継げないので
                // 「技法の外の紙」として扱う
                origin.insert(nf.id, nf.id);
                flipped.insert(nf.id, false);
                continue;
            };
            let ppl = old_state.placements[&pf.id];
            let is_target = targets.is_none_or(|t| t.contains(&pf.id));
            let moved = is_target && signed(ppl.apply(DVec2::from(r))) < -EPS;
            origin.insert(nf.id, self.origin[&pf.id]);
            flipped.insert(nf.id, self.flipped[&pf.id] != moved);
            if plane_map.is_none() {
                let m = res.state.placements[&nf.id].compose(&ppl.inverse());
                plane_map = Some(if moved { m.compose(&refl) } else { m });
            }
        }

        self.faces = new_faces;
        self.state = res.state;
        self.origin = origin;
        self.flipped = flipped;
        self.drivers.extend(res.step.drivers);
        self.added.extend(res.added_edges);
        self.warnings
            .extend(res.warnings.into_iter().filter(|w| !w.contains(TEAR_MARK)));
        Ok(plane_map.unwrap_or_else(Isometry2::identity))
    }

    /// 入力のフラップ指定を検証する。空なら `None`(可動側の全ての層が対象)。
    fn resolve_flap(&self, flap: &[FaceId], name: &str) -> Result<Option<Vec<FaceId>>, String> {
        if flap.is_empty() {
            return Ok(None);
        }
        let mut out: Vec<FaceId> = Vec::with_capacity(flap.len());
        for &id in flap {
            if !self.faces.iter().any(|f| f.id == id) {
                return Err(format!(
                    "{name}の対象に指定された層 {id} が見つかりません。層を選び直してください"
                ));
            }
            if !out.contains(&id) {
                out.push(id);
            }
        }
        Ok(Some(out))
    }

    /// フラップの面を層順(下→上)に並べる。
    fn flap_in_layer_order(&self, flap: &[FaceId], name: &str) -> Result<Vec<FaceId>, String> {
        let set = self
            .resolve_flap(flap, name)?
            .ok_or_else(|| format!("{name}にはフラップ(重なった層)の指定が必要です"))?;
        Ok(self
            .state
            .order
            .iter()
            .copied()
            .filter(|id| set.contains(id))
            .collect())
    }

    /// 技法開始時の面の集合に対応する、現在の面(子孫)の一覧。
    fn descendants(&self, of: &[FaceId]) -> Vec<FaceId> {
        self.faces
            .iter()
            .filter(|f| {
                self.origin
                    .get(&f.id)
                    .is_some_and(|o| of.contains(o))
            })
            .map(|f| f.id)
            .collect()
    }

    /// ここまでの折りで裏返った(=動いた)面の一覧。
    fn moved_faces(&self) -> Vec<FaceId> {
        self.faces
            .iter()
            .filter(|f| self.flipped.get(&f.id).copied() == Some(true))
            .map(|f| f.id)
            .collect()
    }

    /// 対象候補のうち、折り線の可動側に掛かる面だけを残す。
    /// [`fold_through`] は掛からない層を警告付きで除外するので、技法の内部呼び出しでは
    /// あらかじめ絞って余計な警告を出さない。
    fn movable_targets(
        &self,
        line: [[f64; 2]; 2],
        keep_side_point: [f64; 2],
        candidates: Option<&[FaceId]>,
    ) -> Option<Vec<FaceId>> {
        let candidates = candidates?;
        let Ok((l0, l1)) = line_points(line) else {
            return Some(candidates.to_vec());
        };
        let u = (l1 - l0).normalize();
        let keep_sign = u.perp_dot(DVec2::from(keep_side_point) - l0).signum();
        let pos = vertex_positions(&self.cp);
        Some(
            candidates
                .iter()
                .copied()
                .filter(|id| {
                    let Some(f) = self.faces.iter().find(|f| f.id == *id) else {
                        return false;
                    };
                    let pl = self.state.placements[id];
                    f.vertices.iter().filter_map(|v| pos.get(v)).any(|&p| {
                        keep_sign * u.perp_dot(pl.apply(p) - l0) < -EPS
                    })
                })
                .collect(),
        )
    }

    /// フラップの層を「先端を上へ回す層」と「下へ回す層」に塗り分ける。
    ///
    /// 折り線が横切る折り目(背)でつながった2つの層は、先端どうしがつながったまま
    /// 裏返る(これが「裏返して差し込む」形になる理由)。つながった相手とは必ず
    /// 反対向きに回るので、つながりの図を2色で塗り分ければ向きが決まる。
    /// 層の数を機械的に半分に割る方法と違い、奇数層のフラップや、重なりの一部だけを
    /// 選んだフラップでも紙のつながりどおりに折れる。
    ///
    /// 塗り分けの向きは、つながりのかたまりごとに「いちばん手前(上)の層」で決める:
    /// 中割り折りではその層の先端が下(層の内側)へ、かぶせ折りでは上(外側)へ回る。
    /// つながりの図が2色で塗り分けられない(奇数の輪になる)場合は、紙として
    /// 成り立たない指定なのでErr。
    fn split_flap_by_connection(
        &self,
        flap: &[FaceId],
        line: [[f64; 2]; 2],
        inside: bool,
        name: &str,
    ) -> Result<HashMap<FaceId, bool>, String> {
        let members: HashSet<FaceId> = flap.iter().copied().collect();
        // つながりの図: 折り線が横切る折り目を共有する層の組
        let mut adj: HashMap<FaceId, Vec<FaceId>> = flap.iter().map(|&id| (id, Vec::new())).collect();
        for (eid, fs) in faces_by_edge(&self.faces) {
            if fs.len() != 2 || !fs.iter().all(|id| members.contains(id)) {
                continue;
            }
            let Some(e) = self.cp.edges.iter().find(|e| e.id == eid) else {
                continue;
            };
            if !matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley) {
                continue;
            }
            if !self.edge_crosses_line(e.v0, e.v1, fs[0], line) {
                continue;
            }
            adj.get_mut(&fs[0]).expect("フラップの面").push(fs[1]);
            adj.get_mut(&fs[1]).expect("フラップの面").push(fs[0]);
        }

        // かたまりごとに2色で塗り分ける(層順で後ろ=手前の層から始める)
        let mut up: HashMap<FaceId, bool> = HashMap::with_capacity(flap.len());
        // 手前の層の先端は、中割りなら下(内側)・かぶせなら上(外側)へ回る
        let front_up = !inside;
        for &start in flap.iter().rev() {
            if up.contains_key(&start) {
                continue;
            }
            up.insert(start, front_up);
            let mut queue = vec![start];
            while let Some(cur) = queue.pop() {
                let cur_up = up[&cur];
                for &next in &adj[&cur] {
                    match up.get(&next) {
                        Some(&v) if v == cur_up => {
                            return Err(format!(
                                "この重なり方には{name}ができません(層のつながりが輪になっていて、先端の向きが決められません)。フラップの選び方か折り線を変えるか、手動の折り操作で代替してください"
                            ));
                        }
                        Some(_) => {}
                        None => {
                            up.insert(next, !cur_up);
                            queue.push(next);
                        }
                    }
                }
            }
        }
        Ok(up)
    }

    /// 辺(CP座標)が畳み平面の折り線に横切られているか(両端が線の反対側にあるか)。
    /// `on` はこの辺を境界に持つ面(その配置で畳み平面へ写す)。
    fn edge_crosses_line(
        &self,
        v0: VertexId,
        v1: VertexId,
        on: FaceId,
        line: [[f64; 2]; 2],
    ) -> bool {
        let Ok((l0, l1)) = line_points(line) else {
            return false;
        };
        let u = (l1 - l0).normalize();
        let pos = vertex_positions(&self.cp);
        let (Some(&p0), Some(&p1)) = (pos.get(&v0), pos.get(&v1)) else {
            return false;
        };
        let pl = self.state.placements[&on];
        let d0 = u.perp_dot(pl.apply(p0) - l0);
        let d1 = u.perp_dot(pl.apply(p1) - l0);
        (d0 > EPS && d1 < -EPS) || (d0 < -EPS && d1 > EPS)
    }

    /// 面が折り線の両側に掛かっている(折り線が面を横切っている)か。
    fn crosses(&self, id: FaceId, line: [[f64; 2]; 2]) -> bool {
        let Ok((l0, l1)) = line_points(line) else {
            return false;
        };
        let u = (l1 - l0).normalize();
        let Some(f) = self.faces.iter().find(|f| f.id == id) else {
            return false;
        };
        let pl = self.state.placements[&id];
        let pos = vertex_positions(&self.cp);
        let (mut plus, mut minus) = (false, false);
        for p in f.vertices.iter().filter_map(|v| pos.get(v)) {
            let d = u.perp_dot(pl.apply(*p) - l0);
            if d > EPS {
                plus = true;
            } else if d < -EPS {
                minus = true;
            }
        }
        plus && minus
    }

    /// 動いた部分の中にある折り目の山谷を、動いた後の重なり順に合わせ直す。
    ///
    /// 折り目でつながった2面の上下は、その折り目を表から見たときの山谷で決まる
    /// (表向きの面から見て谷なら相手は上、山なら下)。技法で紙の一部が裏返り、
    /// 重なり順が入れ替わると、この関係が崩れることがある:
    ///
    /// - 段折り: 段の中の紙は「まとめて折り返された」ので、裏返ると同時に重なり順も
    ///   逆になる。差し引きで折り目の山谷は変わらない
    /// - 中割り折り・かぶせ折り: 先端は裏返るが、先端どうしの重なり順は保たれる
    ///   (だから「裏返して差し込む」形になる)。そのぶん折り目の山谷が入れ替わる
    ///
    /// どちらの場合も結果は同じ規則で決まるので、重なり順から必要な山谷を求めて
    /// 書き換え、変えた折り目はDriverLineとして記録する
    /// (記録しないと再生時に前の手順の角度のまま折られてしまう)。
    ///
    /// 層順序の並べ替え([`Session::reorder_reversed`])より後に呼ぶこと。
    fn fix_reversed_creases(&mut self) {
        let edge_faces = faces_by_edge(&self.faces);
        let pos = vertex_positions(&self.cp);
        let rank: HashMap<FaceId, usize> = self
            .state
            .order
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i))
            .collect();
        // (辺ID, あるべき線種)
        let mut fixes: Vec<(EdgeId, EdgeKind)> = Vec::new();
        for (eid, fs) in &edge_faces {
            if fs.len() != 2 {
                continue;
            }
            // 技法で動いた紙の中の折り目だけを見る
            if !fs.iter().all(|id| self.flipped.get(id).copied() == Some(true)) {
                continue;
            }
            let (a, b) = (fs[0], fs[1]);
            let (pa, pb) = (self.state.placements[&a], self.state.placements[&b]);
            // 折られていない(平らにつながったまま)折り目は上下を決めない
            if pa.mirrored == pb.mirrored {
                continue;
            }
            let (Some(&ra), Some(&rb)) = (rank.get(&a), rank.get(&b)) else {
                continue;
            };
            // 相手が上にあるのは「表向きから見て谷」のとき(裏返っていれば逆)
            let want = if (rb > ra) == pa.mirrored {
                EdgeKind::Mountain
            } else {
                EdgeKind::Valley
            };
            fixes.push((*eid, want));
        }
        for (eid, want) in fixes {
            let Some(e) = self.cp.edges.iter_mut().find(|e| e.id == eid) else {
                continue;
            };
            if !matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley) || e.kind == want {
                continue;
            }
            e.kind = want;
            let (v0, v1) = (e.v0, e.v1);
            let (Some(&a), Some(&b)) = (pos.get(&v0), pos.get(&v1)) else {
                continue;
            };
            push_driver_line(&mut self.drivers, a, b, angle_of(want));
        }
    }

    /// 裏返った先端を、それぞれが回った向きの側へ置き直す。
    ///
    /// [`fold_through`] は動いた層を山全体のいちばん上(Up)/いちばん下(Down)へ
    /// まとめる近似なので、そのままでは先端が紙の外側に出てしまう。実際には、
    /// 先端はもとの層(同じ面から分かれた残りの部分)のすぐ隣へ入る:
    /// 上へ回った先端はすぐ上、下へ回った先端はすぐ下。折り目の向き(山谷)と
    /// 重なり順の関係もこれで満たされる。
    ///
    /// `up` は面ID(技法開始時)→ 先端を上へ回すか(技法を始めた時点の畳み平面での向き)。
    /// `turned` は折りの途中で畳み平面ごと裏返った(重なり順の上下が入れ替わった)場合にtrue。
    fn reorder_tips(&mut self, up: &HashMap<FaceId, bool>, turned: bool) {
        let is_tip = |id: &FaceId| self.flipped.get(id).copied() == Some(true);
        // 先端を除いた並び(下→上)
        let base: Vec<FaceId> = self
            .state
            .order
            .iter()
            .copied()
            .filter(|id| !is_tip(id))
            .collect();
        let tips: Vec<FaceId> = self.state.order.iter().copied().filter(is_tip).collect();
        if tips.is_empty() {
            return;
        }
        let mut placed: HashSet<FaceId> = HashSet::new();
        let mut out: Vec<FaceId> = Vec::with_capacity(self.state.order.len());
        for &id in &base {
            let origin = self.origin[&id];
            let Some(&up_at_start) = up.get(&origin) else {
                out.push(id);
                continue;
            };
            let goes_up = up_at_start != turned;
            // 同じ面から分かれた先端を、回った向きの側の隣へ挟む
            let group: Vec<FaceId> = tips
                .iter()
                .copied()
                .filter(|t| !placed.contains(t) && self.origin[t] == origin)
                .collect();
            placed.extend(group.iter().copied());
            if goes_up {
                out.push(id);
                out.extend(group);
            } else {
                out.extend(group);
                out.push(id);
            }
        }
        // 相手が見つからなかった先端(面が丸ごと動いた場合)は、もとの並びの位置へ戻す
        for &t in &tips {
            if !placed.contains(&t) {
                let at = self
                    .state
                    .order
                    .iter()
                    .position(|&id| id == t)
                    .unwrap_or(0)
                    .min(out.len());
                out.insert(at, t);
            }
        }
        debug_assert_eq!(out.len(), self.state.order.len());
        self.state.order = out;
    }

    /// 最終形で紙が裂けている(動いた面とその隣の面のつながりが切れている)辺の警告。
    ///
    /// 技法の中で動いた面に接する辺だけを見る(技法より前の手順で入った裂けを
    /// 二重に報告しないため)。
    fn tear_warnings(&self) -> Vec<String> {
        let edge_faces = faces_by_edge(&self.faces);
        let pos = vertex_positions(&self.cp);
        let mut out: Vec<String> = Vec::new();
        for (eid, fs) in &edge_faces {
            if fs.len() != 2 {
                continue;
            }
            if !fs.iter().any(|id| self.flipped.get(id).copied() == Some(true)) {
                continue;
            }
            let Some(e) = self.cp.edges.iter().find(|e| e.id == *eid) else {
                continue;
            };
            let (Some(&p0), Some(&p1)) = (pos.get(&e.v0), pos.get(&e.v1)) else {
                continue;
            };
            let a = self.state.placements[&fs[0]];
            let b = self.state.placements[&fs[1]];
            if (a.apply(p0) - b.apply(p0)).length() > JOIN_EPS
                || (a.apply(p1) - b.apply(p1)).length() > JOIN_EPS
            {
                out.push(format!(
                    "折り目(辺ID {eid})の両側の紙が離れています。このままでは紙が裂けます(指定のまま続行します)"
                ));
            }
        }
        out
    }

    /// 折り上がりが紙の重なりとして成り立っているかを調べ、食い違いを警告にする。
    ///
    /// 折り目でつながった2面の上下は、その折り目を表から見たときの山谷で決まる
    /// (表向きの面から見て谷なら相手は上、山なら下)。技法で動いた紙に接する
    /// 折り目がこの関係を満たさないなら、展開図の山谷と記録した重なり順が食い違う
    /// = 展開図から折り直すと別の形になる。
    ///
    /// 断らずに警告にするのは「実際に折れるものを断らない」ため(「止めずに警告」原則。
    /// 紙が裂ける指定を [`fold_through`] が警告で通すのと同じ扱い)。多くは、重なりの
    /// 一部だけを選んだために紙が裂ける指定と一緒に出る。
    ///
    /// 技法で動いた面に接する折り目だけを見る(技法より前の手順で入った食い違いまで
    /// 数えない)。
    fn layer_consistency_warnings(&self, name: &str) -> Vec<String> {
        let rank: HashMap<FaceId, usize> = self
            .state
            .order
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i))
            .collect();
        let mut bad = 0usize;
        for (eid, fs) in faces_by_edge(&self.faces) {
            if fs.len() != 2 {
                continue;
            }
            let (a, b) = (fs[0], fs[1]);
            if !self.flipped.get(&a).copied().unwrap_or(false)
                && !self.flipped.get(&b).copied().unwrap_or(false)
            {
                continue;
            }
            let Some(e) = self.cp.edges.iter().find(|e| e.id == eid) else {
                continue;
            };
            if !matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley) {
                continue;
            }
            let (pa, pb) = (self.state.placements[&a], self.state.placements[&b]);
            // 折られていない(平らにつながったまま)折り目は上下を決めない
            if pa.mirrored == pb.mirrored {
                continue;
            }
            let (Some(&ra), Some(&rb)) = (rank.get(&a), rank.get(&b)) else {
                continue;
            };
            let want = if (rb > ra) == pa.mirrored {
                EdgeKind::Mountain
            } else {
                EdgeKind::Valley
            };
            if e.kind != want {
                bad += 1;
            }
        }
        if bad > 0 {
            return vec![format!(
                "この{name}では、折り目{bad}本の山谷と紙の重なり順が食い違います。このままでは展開図から折り直したときに形が変わります(指定のまま続行します)"
            )];
        }
        Vec::new()
    }

    /// 技法の結果をまとめ、CPを呼び出し側へ書き戻す。
    /// 紙が裂ける・山谷と重なり順が食い違うといった危うい折り上がりは警告にする
    /// (「止めずに警告」原則)。
    fn finish(
        mut self,
        kind: TechniqueKind,
        name: &str,
        cp: &mut CreasePattern,
    ) -> Result<FoldThroughResult, String> {
        let tears = self.tear_warnings();
        self.warnings.extend(tears);
        let inconsistent = self.layer_consistency_warnings(name);
        self.warnings.extend(inconsistent);
        let mut added = self.added;
        added.sort_unstable();
        added.dedup();
        added.retain(|id| self.cp.edges.iter().any(|e| e.id == *id));
        let layer_points = self.state.to_layer_points(&self.cp, &self.faces);
        let step = FoldStep {
            id: 0,
            kind,
            drivers: self.drivers,
            layer_order: Some(layer_points),
            note: String::new(),
        };
        *cp = self.cp;
        Ok(FoldThroughResult {
            state: self.state,
            added_edges: added,
            step,
            warnings: self.warnings,
        })
    }
}

// ---------------------------------------------------------------------------
// 小さな道具
// ---------------------------------------------------------------------------

/// 折り線の2点。退化(2点が一致)しているときはErr。
fn line_points(line: [[f64; 2]; 2]) -> Result<(DVec2, DVec2), String> {
    let l0 = DVec2::from(line[0]);
    let l1 = DVec2::from(line[1]);
    if (l1 - l0).length() < EPS {
        return Err("折り線の2点が一致しています".to_string());
    }
    Ok((l0, l1))
}

/// ここまでの折りで畳み平面が裏返っているなら、折る向きを反対にする。
///
/// [`fold_through`] は結果を「根面(最小面ID)が恒等」の座標系へそろえ直すため、
/// 動いた側に根面があると畳み平面ごと裏返る(層順序も上下が入れ替わる)。
/// 技法の2回目以降の折りは1回目と同じ紙の側へ重ねたいので、裏返っている場合は
/// Up/Downを入れ替えて指定する(これをしないと、どの面が最小IDだったかという
/// 内部の事情で技法の結果が変わってしまう)。
fn turn_direction(direction: FoldDirection, turned: bool) -> FoldDirection {
    if !turned {
        return direction;
    }
    match direction {
        FoldDirection::Up => FoldDirection::Down,
        FoldDirection::Down => FoldDirection::Up,
    }
}
