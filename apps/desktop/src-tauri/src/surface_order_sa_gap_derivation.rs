/// 段階3: **折り目の向きの規則を使わずに**、裂けていない姿勢の隙間の符号だけから
/// 上下を決め、刻印された `surface_rank` と突き合わせる。
///
/// `crates/ori3-rigid/tests/surface_order.rs` の
/// `exact_folds_agree_with_the_surface_rank` は、`tree::exact_stack_constraints` が
/// 決めた `surface_rank` を**同じ規則**で照合しているため、規則そのものが誤って
/// いても違反0になる。ここでは規則をいっさい呼ばず、
///
/// 1. 折り切った折り目を**1本だけ**少し戻した姿勢を伝播で作り、
/// 2. その姿勢が**裂けていない**こと(`max_seam_gap < 1e-6`)を確かめ、
/// 3. 折り目に接する2面の重心が、表示姿勢の正準法線のどちら側にあるかを測り、
/// 4. その符号と `surface_rank` の上下が一致すること
///
/// だけを検査する。展開図と角度は検査コードへ埋め込んであり、`verification/` などの
/// 追跡対象外のファイルは読まない。期待値は数値ではなく「2つの実測量が一致する」と
/// いう関係なので、計算機が変わっても成り立つ(CLAUDE.md §10.7.7)。
#[test]
fn surface_rank_agrees_with_the_measured_gap_without_using_the_crease_rule() {
    let cases: [(&str, CreasePattern, HashMap<EdgeId, f64>); 4] = [
        ("live-frame", live_frame_cp(), live_frame_angles()),
        (
            "zero-back-user",
            zero_back_user_cp(),
            zero_back_user_angles(),
        ),
        (
            "diagonal-midline-square-edge12-mountain",
            flat_foldable_kome(),
            kome_edge12_angles(1.0),
        ),
        (
            "diagonal-midline-square-edge12-valley",
            flat_foldable_kome(),
            kome_edge12_angles(-1.0),
        ),
    ];
    let mut checked = 0_usize;
    for (name, cp, angles) in cases {
        let faces = extract_faces(&cp);
        let folded = to_frame3d(&cp, &faces, &propagate(&cp, &faces, &angles));
        checked +=
            assert_exact_creases_follow_the_measured_gap(name, &cp, &faces, &angles, &folded);
    }
    // この作業で直した `folded-sample.ori3` の辺425を含む3本。101本の角度はsolveで
    // 作るが、期待値は「実測の隙間」と「刻印された順位」の一致だけなので、solveの
    // 結果に数値を結び付けてはいない。
    let diagrams = boundary_diagrams();
    let diagram = diagrams
        .iter()
        .find(|diagram| diagram.name == "folded-sample.ori3")
        .expect("the folded acceptance fixture exists");
    for hinge in [425_u32, 12, 306] {
        for sign in [1.0_f64, -1.0] {
            let (_, folded) = endpoint_frames(diagram, hinge, sign);
            checked += assert_exact_creases_follow_the_measured_gap(
                &format!("folded-sample.ori3 edge{hinge} {sign:+}"),
                &diagram.cp,
                &diagram.faces,
                &folded.angles,
                &folded.frame,
            );
        }
    }
    println!("GAPRULEFREE checked_creases={checked}");
    // 主張の対象が空になっていないことの下限。裂ける探りや別の折り方へ飛んだ探りは
    // 根拠に使えないので、測れない折り目が残る。梯子の選別が壊れて対象が消えると、
    // この検査は何も主張しないまま緑になってしまう。
    //
    // **測れる本数そのものが計算機で変わる。** 探りは `ori3_rigid::solve` /
    // `solve_near` の結果であり、完全に折った状態のすぐ近くでは丸めの違いで解が
    // 近くの別の折り方へ移る(CLAUDE.md §10.7.7)。実測は
    // **この計算機で 50本、CI(GitHub Actions)の計算機で 49本**で、1本ずれた。
    // 以前は実測値 50 をそのまま下限にしていたため余裕が0で、CIが赤になった。
    //
    // そこで同じファイルの `robust_stacks`(実測4888に対し下限4,000 = 約82%)と
    // 同じ取り方にそろえ、実測の約8割にあたる **40本**を下限にする。実測49に対して
    // 18%、実測50に対して20%の余裕がある。空回りすれば0に近い値まで落ちるので、
    // この値でも検知できる。
    assert!(
        checked >= 40,
        "±180度の折り目をほとんど測れていない: checked={checked}"
    );
}

/// 折り切った折り目に接する2面が、少し戻した姿勢でどちら側へ離れるかを測って
/// `surface_rank` と突き合わせる。測れた折り目の本数を返す。
///
/// 戻した姿勢は `solved_unfold_ladder`(全ての折り目を同じ割合で縮めてsolveし、
/// **裂けた段は捨てる**梯子)を使う。製品の重なり順が使う「全ヒンジを共通の角度で
/// 頭打ちにする」経路とは刻み方が違う独立の道で、重なり順も刻印させない。
///
/// 「上」の向きは**表示する姿勢**での基準面の正準法線に取る。刻印された
/// `surface_rank` はその向きに下→上で並んでいるからである。戻した姿勢では基準面も
/// 少し動くので、平面の向きが 8°(内積0.99)以上ずれた段は使わない。また
/// **高さの差がその段の裂けの量以下**なら、離れている量より紙のちぎれのほうが
/// 大きいので答えの根拠にしない。
fn assert_exact_creases_follow_the_measured_gap(
    name: &str,
    cp: &CreasePattern,
    faces: &[Face],
    angles: &HashMap<EdgeId, f64>,
    folded: &Frame3D,
) -> usize {
    let ranks = folded
        .faces
        .iter()
        .map(|face| (face.face, face.surface_rank))
        .collect::<BTreeMap<_, _>>();
    let folded_polygons = frame_polygons(folded);
    let mut ladder = Vec::new();
    let mut warm = angles.clone();
    for probe_abs in [179.999_f64, 179.99, 179.9, 179.0, 175.0] {
        let mut drivers = angles
            .iter()
            .filter(|(_, angle)| angle.abs() > probe_abs)
            .map(|(&hinge, &angle)| Driver {
                hinge,
                target_angle_deg: angle.signum() * probe_abs,
            })
            .collect::<Vec<_>>();
        drivers.sort_unstable_by_key(|driver| driver.hinge);
        let solved = ori3_rigid::solve(cp, faces, &drivers, Some(&warm));
        let seam = ori3_rigid::max_seam_gap(cp, faces, &solved.frame);
        warm = solved.angles.clone();
        if seam < SEAM_TEAR_TOLERANCE
            && solved
                .frame
                .faces
                .iter()
                .flat_map(|face| &face.polygon)
                .flatten()
                .all(|coordinate| coordinate.is_finite())
        {
            ladder.push((seam, frame_polygons(&solved.frame)));
        }
    }
    let mut hinges = angles
        .iter()
        .filter(|(_, angle)| (angle.abs() - 180.0).abs() <= 1e-9)
        .map(|(&hinge, &angle)| (hinge, angle))
        .collect::<Vec<_>>();
    hinges.sort_by_key(|&(hinge, _)| hinge);
    let mut checked = 0_usize;
    for (hinge, angle) in hinges {
        let touching = faces
            .iter()
            .filter(|face| face.edges.contains(&hinge))
            .map(|face| face.id)
            .collect::<Vec<_>>();
        let [reference, other] = touching[..] else {
            continue;
        };
        let Some(axis) = folded_polygons
            .get(&reference)
            .and_then(|points| polygon_normal3(points))
            .map(canonical3)
        else {
            continue;
        };
        // 全ての折り切った折り目をまとめて戻した梯子で足りないときは、
        // **この折り目だけ**を戻した姿勢をsolveで作って探る。
        let mut probes = ladder.clone();
        if !probes.iter().any(|(seam, polygons)| {
            usable_probe_height(polygons, reference, other, axis, *seam).is_some()
        }) {
            let mut warm = angles.clone();
            for probe_abs in [179.999_f64, 179.99, 179.9, 179.0, 175.0] {
                let solved = ori3_rigid::solve_near(
                    cp,
                    faces,
                    &[Driver {
                        hinge,
                        target_angle_deg: angle.signum() * probe_abs,
                    }],
                    angles,
                    Some(&warm),
                );
                let seam = ori3_rigid::max_seam_gap(cp, faces, &solved.frame);
                let moved = max_vertex_distance(folded, &solved.frame);
                warm = solved.angles.clone();
                // 戻した角度が生む動きより桁違いに大きく動いた探りは、別の折り方へ
                // 飛んでいる。r度戻すと紙は高々 r ラジアン×紙の大きさ(≦1)だけ動く。
                if seam < SEAM_TEAR_TOLERANCE
                    && moved <= 20.0 * (angle.abs() - probe_abs).to_radians()
                    && solved
                        .frame
                        .faces
                        .iter()
                        .flat_map(|face| &face.polygon)
                        .flatten()
                        .all(|coordinate| coordinate.is_finite())
                {
                    probes.push((seam, frame_polygons(&solved.frame)));
                }
            }
        }
        for (seam, polygons) in &probes {
            let Some(height) = usable_probe_height(polygons, reference, other, axis, *seam) else {
                continue;
            };
            checked += 1;
            assert_eq!(
                ranks[&other] > ranks[&reference],
                height > 0.0,
                "{name}: 折り目{hinge}({angle:+.3}度)で、面{other}は面{reference}の\
                 実測で{height:.6e}(その段の裂け{seam:.3e})の側にあるのに、重なり順が逆である"
            );
            break;
        }
    }
    checked
}

/// 探る姿勢での「基準面から相手面の重心までの符号付き高さ」。使えないとき `None`。
///
/// 使えない条件は2つ。基準面の平面が表示姿勢から8度(内積0.99)以上傾いた探りと、
/// **高さがその探りの裂けの量以下**のもの。後者は、面が離れている量より紙が
/// ちぎれている量のほうが大きく、符号に信号が無いためである。
fn usable_probe_height(
    polygons: &BTreeMap<FaceId, Vec<V3>>,
    reference: FaceId,
    other: FaceId,
    axis: V3,
    seam: f64,
) -> Option<f64> {
    let reference_points = polygons.get(&reference)?;
    let other_points = polygons.get(&other)?;
    if polygon_normal3(reference_points)?.dot(axis).abs() < 0.99 {
        return None;
    }
    let centroid = |points: &[V3]| {
        points.iter().fold(V3::ZERO, |sum, &point| sum + point) / points.len() as f64
    };
    let height = (centroid(other_points) - centroid(reference_points)).dot(axis);
    (height.abs() > seam.max(1e-9)).then_some(height)
}


/// 保存した層順序は編集用の `layer` にだけ残し、表示用 `surface_rank` は同じ
/// 8手を幾何だけで再生した順位と一致させる。面ID順を表示順位へ戻さない回帰検査。
#[test]
fn saved_layer_order_does_not_override_geometric_surface_rank() {
    const EXACT_OVERLAPS: [(FaceId, FaceId); 9] = [
        (4, 5),
        (8, 9),
        (14, 15),
        (24, 29),
        (25, 28),
        (26, 32),
        (27, 33),
        (30, 35),
        (31, 34),
    ];

    let saved_doc: Document =
        serde_json::from_str(FOLDED_SAMPLE).expect("folded-sample fixture is a Document");
    assert_eq!(saved_doc.sequence.len(), 8, "保存標本は8手である");
    let faces = extract_faces(&saved_doc.cp);
    assert_eq!(faces.len(), 46, "保存標本は46面である");

    let mut geometric_doc = saved_doc.clone();
    for step in &mut geometric_doc.sequence {
        step.layer_order = None;
    }
    assert_eq!(geometric_doc.sequence.len(), 8, "幾何版も同じ8手を再生する");

    let saved = ori3_layers::replay(&saved_doc, saved_doc.sequence.len(), 1.0);
    let geometric = ori3_layers::replay(&geometric_doc, geometric_doc.sequence.len(), 1.0);
    assert!(
        saved.surface_order_provenance.is_some(),
        "保存順版のsurface順位はcompleteな幾何導出である"
    );
    assert!(
        geometric.surface_order_provenance.is_some(),
        "幾何版のsurface順位はcompleteな幾何導出である"
    );
    assert!(
        saved.skipped.is_empty() && geometric.skipped.is_empty(),
        "8手を1つも飛ばさず再生する"
    );
    assert_eq!(saved.frame.faces.len(), 46, "保存順版の表示は46面である");
    assert_eq!(geometric.frame.faces.len(), 46, "幾何版の表示は46面である");
    assert_eq!(
        saved.hinge_angles, geometric.hinge_angles,
        "保存層順序の有無で再生角を変えない"
    );

    let ranks_by_face = |frame: &Frame3D| {
        frame
            .faces
            .iter()
            .map(|face| (face.face, face.surface_rank))
            .collect::<BTreeMap<_, _>>()
    };
    let saved_ranks = ranks_by_face(&saved.frame);
    let geometric_ranks = ranks_by_face(&geometric.frame);
    assert_eq!(
        saved_ranks, geometric_ranks,
        "保存層順序が表示用surface_rankを上書きした"
    );
    let numbered = saved_ranks
        .iter()
        .filter(|&(face, rank)| face == rank)
        .count();
    assert_eq!(
        numbered, 0,
        "surface_rankが面IDと同じ面が残っている: {saved_ranks:?}"
    );

    let saved_order =
        ori3_layers::saved_layer_order_at(&saved_doc, &faces, saved_doc.sequence.len(), 1.0)
            .expect("8手目の保存layer_orderを解決できる");
    let expected_layers = saved_order
        .iter()
        .enumerate()
        .map(|(layer, &face)| (u32::try_from(layer).expect("46層はu32に収まる"), face))
        .collect::<Vec<_>>();
    let mut actual_layers = saved
        .frame
        .faces
        .iter()
        .map(|face| (face.layer, face.face))
        .collect::<Vec<_>>();
    actual_layers.sort_unstable();
    assert_eq!(
        actual_layers, expected_layers,
        "編集用layerは保存layer_orderを保つ"
    );

    let long = saved_doc.paper.width_mm.max(saved_doc.paper.height_mm);
    let folded_sample = diagram(
        "folded-sample-saved-rank",
        saved_doc.cp.clone(),
        saved_doc.paper.width_mm / long,
        saved_doc.paper.height_mm / long,
    );
    let exact_pairs = coincident_overlaps(&folded_sample, &geometric.frame)
        .into_iter()
        .map(|pair| (pair.left.min(pair.right), pair.left.max(pair.right)))
        .collect::<BTreeSet<_>>();
    let required_exact_pairs = EXACT_OVERLAPS.into_iter().collect::<BTreeSet<_>>();
    assert!(
        required_exact_pairs.is_subset(&exact_pairs),
        "指定した完全重なり9組が欠けた: {:?}",
        required_exact_pairs
            .difference(&exact_pairs)
            .collect::<Vec<_>>()
    );
    for (left, right) in EXACT_OVERLAPS {
        assert_ne!(saved_ranks[&left], saved_ranks[&right]);
        assert_eq!(
            saved_ranks[&left] < saved_ranks[&right],
            geometric_ranks[&left] < geometric_ranks[&right],
            "完全重なり面({left}, {right})の上下が保存層順序で変わった"
        );
    }

    // `replay` と独立な直接伝播でも、指定9組の幾何的な上下が一致する。
    let canonical = to_frame3d(
        &saved_doc.cp,
        &faces,
        &propagate(&saved_doc.cp, &faces, &saved.hinge_angles),
    );
    let canonical_ranks = ranks_by_face(&canonical);
    for (left, right) in EXACT_OVERLAPS {
        assert_eq!(
            canonical_ranks[&left] < canonical_ranks[&right],
            geometric_ranks[&left] < geometric_ranks[&right],
            "直接伝播と幾何再生で完全重なり面({left}, {right})の上下が違う"
        );
    }

    let mut hard = saved
        .hinge_angles
        .iter()
        .map(|(&hinge, &target_angle_deg)| Driver {
            hinge,
            target_angle_deg,
        })
        .collect::<Vec<_>>();
    hard.sort_unstable_by_key(|driver| driver.hinge);
    let motion = solve_motion(
        &saved_doc.cp,
        &faces,
        &hard,
        None,
        Some(&saved.hinge_angles),
        true,
    );
    assert!(
        motion.surface_order_authoritative,
        "FOLDED_SAMPLEのmotion順はcompleteな幾何導出を刻印できる"
    );
    let diagnostics = motion
        .surface_order
        .expect("surface順を刻印するmotionは診断を返す");
    println!("FOLDED_SAMPLE_MOTION_DIAGNOSTICS {diagnostics:?}");
    // 現在の継続法はlive3の実motion pathだけで23重なり対をcompleteにできる。
    // flatからのcanonical fallbackへ進む前に、表示した運動そのものを採ることを固定する。
    assert_eq!(diagnostics.source, SurfaceOrderSource::SolvedMotionPath);
    // 以前は2件だった。捨てていた2件は「面7が面6より上」「面9が面8より上」で、
    // すき間の実測(`solved_unfold_ladder`)が示す上下そのものだった。捨てていた理由は、
    // 主対角 `x = y` 上の11本の折り目のうち7本が `−180°`、4本が `+180°` と
    // **割れて記録されていた**ことにある。主対角以外の90本はすべて 0.0° なので
    // 紙は両半分とも1枚の硬い板であり、同じ板どうしを折り重ねる折り目が
    // 山と谷に割れることは実際の紙では起こらない。
    // `tree::exact_stack_constraints` が割れを見つけてその板の組の厳密な拘束を
    // 出さなくなったので、上下は実測の深度が決め、**捨てる理由そのものが消えた**。
    // 期待値を緩めたのではなく、捨てる対象が無くなった結果である。
    assert_eq!(diagnostics.dropped_depth_constraints, 0);
    assert_eq!(diagnostics.unresolved_overlaps, 0);
    assert_eq!(diagnostics.broken_constraints, 0);
    let motion_ranks = ranks_by_face(&motion.result.frame);
    for (left, right) in EXACT_OVERLAPS {
        assert_eq!(
            motion_ranks[&left] < motion_ranks[&right],
            geometric_ranks[&left] < geometric_ranks[&right],
            "直接solveと幾何再生で完全重なり面({left}, {right})の上下が違う"
        );
    }

    println!(
        "FOLDED_SAMPLE_SAVED_RANK faces={} numbered={} overlaps={} saved_ranks={saved_ranks:?}",
        saved.frame.faces.len(),
        numbered,
        exact_pairs.len(),
    );
}


/// 完全に重なっている面対の上下が、**導出の経路によって変わらない**ことを固定する。
///
/// 重なり順を出す道は3つあり、どれも同じ答えでなければならない。
///
/// 1. `replay`(手順を再生し、その手の追従経路の深度で決める)
/// 2. `propagate` + `to_frame3d`(手順を通さず、最終角の伝播経路の深度で決める)
/// 3. `solve_motion`(平らな紙から最終角まで解いた運動の深度で決める)
///
/// さらに、この3つと独立した**すき間の実測**(`solved_unfold_ladder` は全ての折り目を
/// 同じ割合で縮めながら実際に解き、裂けた段を捨てる。重なり順は刻印させない)と
/// 突き合わせる。
///
/// **この検査が捕まえた不具合(2026-08-22)**: `folded-sample.ori3` の手順7で、
/// `replay` が渡す探り経路の3姿勢が**すべて同じ姿勢**で、終点から 0.5756(紙の長辺=1.0)
/// 離れたまま1度も動いていなかった。その止まった姿勢の高さの差が、完全重なり4組
/// (24,29)・(25,28)・(27,33)・(39,43) の上下を、**実際に表示する動きと逆**に決めていた。
/// 手順8は角度を1本も変えないので手順7の順をそのまま引き継ぎ、誤りが最終形へ残っていた。
#[test]
fn coincident_overlap_order_is_the_same_for_every_derivation() {
    let saved_doc: Document =
        serde_json::from_str(FOLDED_SAMPLE).expect("folded-sample fixture is a Document");
    let faces = extract_faces(&saved_doc.cp);
    let mut geometric_doc = saved_doc.clone();
    for step in &mut geometric_doc.sequence {
        step.layer_order = None;
    }
    let replayed = ori3_layers::replay(&geometric_doc, geometric_doc.sequence.len(), 1.0);
    assert!(
        replayed.surface_order_provenance.is_some(),
        "再生した重なり順はcompleteな幾何導出である"
    );
    let angles = replayed.hinge_angles.clone();

    let propagated = to_frame3d(
        &saved_doc.cp,
        &faces,
        &propagate(&saved_doc.cp, &faces, &angles),
    );

    let mut hard = angles
        .iter()
        .map(|(&hinge, &target_angle_deg)| Driver {
            hinge,
            target_angle_deg,
        })
        .collect::<Vec<_>>();
    hard.sort_unstable_by_key(|driver| driver.hinge);
    let motion = solve_motion(&saved_doc.cp, &faces, &hard, None, Some(&angles), true);
    assert!(
        motion.surface_order_authoritative,
        "同じ最終角のmotionはcompleteな幾何導出を刻印できる"
    );

    let ranks_of = |frame: &Frame3D| {
        frame
            .faces
            .iter()
            .map(|face| (face.face, face.surface_rank))
            .collect::<BTreeMap<_, _>>()
    };
    let replayed_ranks = ranks_of(&replayed.frame);
    let propagated_ranks = ranks_of(&propagated);
    let motion_ranks = ranks_of(&motion.result.frame);

    let long = saved_doc.paper.width_mm.max(saved_doc.paper.height_mm);
    let folded_sample = diagram(
        "folded-sample-every-derivation",
        saved_doc.cp.clone(),
        saved_doc.paper.width_mm / long,
        saved_doc.paper.height_mm / long,
    );
    let overlaps = coincident_overlaps(&folded_sample, &replayed.frame);
    // 主張の対象が空にならないことの下限。実測は23組で、その約8割を下限にする
    // (CLAUDE.md §10.7.9)。重なりの拾い方が壊れて対象が消えると、この検査は
    // 何も主張しないまま緑になってしまう。
    assert!(
        overlaps.len() >= 18,
        "完全に重なる面対をほとんど拾えていない: {}",
        overlaps.len()
    );

    let exact_polygons = frame_polygons(&replayed.frame);
    let ladder = solved_unfold_ladder(&saved_doc.cp, &faces, &angles);
    let mut measured = 0_usize;
    let mut measured_agrees = 0_usize;
    for overlap in &overlaps {
        let (left, right) = (overlap.left, overlap.right);
        let replayed_above = replayed_ranks[&left] > replayed_ranks[&right];
        let propagated_above = propagated_ranks[&left] > propagated_ranks[&right];
        let motion_above = motion_ranks[&left] > motion_ranks[&right];
        assert_eq!(
            replayed_above, propagated_above,
            "手順の再生と直接伝播で、完全重なり面({left}, {right})の上下が違う"
        );
        assert_eq!(
            replayed_above, motion_above,
            "手順の再生と運動の解で、完全重なり面({left}, {right})の上下が違う"
        );
        if let Some((above, _, _)) = ladder_truth(&exact_polygons, &ladder, overlap) {
            measured += 1;
            if above == replayed_above {
                measured_agrees += 1;
            }
        }
    }
    println!(
        "EVERY_DERIVATION overlaps={} measured={measured} measured_agrees={measured_agrees}",
        overlaps.len()
    );
    // すき間の実測で上下を読めた面対のうち、いくつが刻印された順位と一致したか。
    // 実測は 23組中23組を測れて 21組が一致する(残る2組は、折り切った折り目の向きが
    // 決める上下と実測が逆になる面対 (6,7)・(8,9) で、製品は丸めで壊れない
    // 折り目の向きを優先し、深度側2件を捨てている。`dropped_depth_constraints = 2`)。
    // 下限は実測21の約8割にあたる17とする(CLAUDE.md §10.7.9)。
    assert!(
        measured >= 18,
        "すき間から上下を読めた面対が少なすぎる: {measured}"
    );
    assert!(
        measured_agrees >= 17,
        "刻印された順位が、すき間の実測とほとんど一致していない: {measured_agrees}/{measured}"
    );
}
