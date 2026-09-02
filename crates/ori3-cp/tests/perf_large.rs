//! 大規模CP(辺1,000〜20,000本)での主要処理の実測。
//!
//! NFR-002は「辺1,000・面400まで」を性能目標にしているが、その10倍規模でも
//! 動くのかという問いに実測で答えるためのテスト。結果は`--nocapture`で表に
//! 出力する。重いので`#[ignore]`付き(通常のcargo testでは走らない)。
//!
//! 実行方法:
//!   cargo test --release -p ori3-cp --test perf_large -- --ignored --nocapture
//!
//! 測定は「打ち切り方式」: ある規模で`GATE`を超えた処理は、それより大きい
//! 規模では測らずに「打ち切り」と表示する(遅いこと自体が答えなので、
//! 待ち続けない)。
//!
//! # 実測(2026-08-06, 開発機 Windows 11 / release、括弧内はdebug)
//!
//! | 処理 | 辺1,012 | 辺5,100 | 辺9,940 | 辺20,200 |
//! |---|---|---|---|---|
//! | extract_faces    | 1.5ms (12) | 7.0ms (74) | 12ms (150) | 29ms (309) |
//! | insert_segment×1 | 1.5ms (14) | 28ms (331) | 102ms (1,212) | 405ms (5,088) |
//! | validate         | 9.5ms (89) | 171ms (2,079) | 623ms (8,091) | 2,555ms (打切) |
//!
//! # 空間ハッシュ導入後の実測(2026-08-06, 同一機・release、同じ日に旧実装も再測)
//!
//! | 処理 | 辺1,012 | 辺5,100 | 辺9,940 | 辺20,200 |
//! |---|---|---|---|---|
//! | validate 旧          | 4.1ms | 94ms | 339ms | 1,329ms |
//! | validate 新          | 0.9ms | 4.1ms | 8.6ms | 20ms |
//! | insert_segment×1 旧  | 0.7ms | 15ms | 55ms | 228ms |
//! | insert_segment×1 新  | 0.7ms | 3.8ms | 7.8ms | 17ms |
//! | 線を引いて作る合計 旧 | 7.0ms | 318ms | 1,386ms | 8,189ms |
//! | 線を引いて作る合計 新 | 10ms | 112ms | 305ms | 1,082ms |
//!
//! どちらも辺数にほぼ比例(O(E))になり、辺2万での検査は約66倍、
//! 1本の挿入は約13倍速くなった。警告の内容・順序と吸着の挙動は変えていない。
//! | local_violations | 0.5ms (4) | 3.2ms (16) | 4.5ms (51) | 7.3ms (106) |
//! | cp_svg           | 2.2ms (8) | 15ms (30) | 22ms (62) | 43ms (141) |
//! | solve(cold 1本)  | 12ms (397) | 227ms (8,218) | 1,238ms (打切) | 4,683ms (打切) |
//! | self_intersects  | 0.4ms (17) | 7.3ms (273) | 25ms (—) | 93ms (—) |
//! | replay(3手順)    | 3.2ms (34) | 19ms (246) | 52ms (607) | 127ms (1,735) |
//! | flat_state_at    | 0.9ms (14) | 5.1ms (72) | 12ms (155) | 24ms (330) |
//!
//! 線を引いて規模を作る過程(insert_segmentの繰り返し、release):
//! 42回で6.7ms / 98回で269ms / 138回で1.34s / 198回で13.4s(debugは約20倍)。
//!
//! かつての律速は `validate`(全辺ペア総当たり = O(E²))と `insert_segment`
//! (呼び出しごとに全辺×全頂点を線形探索 = O(E·V))で、いずれも空間ハッシュ
//! (`ori3-cp/src/spatial.rs`)で解消した。残る律速は `solve`。
//! `solve` は疎ヤコビアン+エンベロープコレスキーなので、2D格子では
//! おおよそ O(k²)(k=ヒンジ数)で伸びる。
//! プロセスの最大ワーキングセットは辺20,200・面10,000の測定全体で約55MB。

use std::time::{Duration, Instant};

use ori3_cp::{extract_faces, insert_segment, local_violations, validate};
use ori3_export::{CpSvgOptions, cp_svg};
use ori3_layers::{flat_state_at, replay};
use ori3_model::{
    CreasePattern, Document, Driver, DriverLine, Edge, EdgeKind, FoldStep, Paper, TechniqueKind,
    Vertex,
};
use ori3_rigid::{self_intersects, solve};

/// 1つの処理がこの時間を超えたら、それより大きい規模では測らない。
const GATE: Duration = Duration::from_secs(8);

/// 測る規模(格子の分割数)。辺数は 2·n·(n+1) 本。
/// 22→1,012 / 50→5,100 / 70→9,940 / 100→20,200
const SIZES: [usize; 4] = [22, 50, 70, 100];

/// nc×nr面のミウラ折り風CP(perf_miura.rsのmiura_cpと同じ作り)。
/// 内部頂点は次数4・縦2辺が一直線で、川崎/前川を満たす平坦折り可能な平面グラフ。
fn miura_cp(nc: usize, nr: usize) -> CreasePattern {
    let s = 0.35;
    let dx = 1.0 / nc as f64;
    let dy = 1.0 / (nr as f64 + s);
    let vid = |i: usize, j: usize| (j * (nc + 1) + i) as u32;
    let mut vertices = Vec::new();
    for j in 0..=nr {
        for i in 0..=nc {
            vertices.push(Vertex {
                id: vid(i, j),
                pos: [
                    i as f64 * dx,
                    (j as f64 + if i % 2 == 1 { s } else { 0.0 }) * dy,
                ],
            });
        }
    }
    let mut edges: Vec<Edge> = Vec::new();
    for j in 0..nr {
        for i in 0..=nc {
            let kind = if i == 0 || i == nc {
                EdgeKind::Border
            } else if (i + j) % 2 == 0 {
                EdgeKind::Mountain
            } else {
                EdgeKind::Valley
            };
            edges.push(Edge {
                id: edges.len() as u32,
                v0: vid(i, j),
                v1: vid(i, j + 1),
                kind,
            });
        }
    }
    for j in 0..=nr {
        for i in 0..nc {
            let kind = if j == 0 || j == nr {
                EdgeKind::Border
            } else if j % 2 == 1 {
                EdgeKind::Mountain
            } else {
                EdgeKind::Valley
            };
            edges.push(Edge {
                id: edges.len() as u32,
                v0: vid(i, j),
                v1: vid(i + 1, j),
                kind,
            });
        }
    }
    CreasePattern {
        next_vertex_id: vertices.len() as u32,
        next_edge_id: edges.len() as u32,
        vertices,
        edges,
    }
}

/// 処理を1回走らせて所要時間を返す。
fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t0 = Instant::now();
    let v = f();
    (v, t0.elapsed())
}

/// 実測1行を表形式で出す。
fn row(op: &str, n: usize, edges: usize, faces: usize, dt: Duration, note: &str) {
    println!(
        "| {op:22} | n={n:3} | 辺{edges:6} | 面{faces:6} | {:>10.1} ms | {note}",
        dt.as_secs_f64() * 1e3
    );
}

/// 手順を数個持つDocumentを作る(再生の測定用)。
/// driverは内部の縦の折り線(x=一定の直線)を線分で指定する。
fn doc_with_steps(cp: CreasePattern, nc: usize) -> Document {
    let mut doc = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    doc.cp = cp;
    doc.sequence = (1..=3usize)
        .map(|s| {
            let i = nc / 4 + s * nc / 8;
            let x = i as f64 / nc as f64;
            FoldStep {
                id: s as u32,
                kind: TechniqueKind::Simple,
                drivers: vec![DriverLine {
                    a: [x, 0.0],
                    b: [x, 1.0],
                    target_angle_deg: 30.0,
                }],
                layer_order: None,
                alignment: None,
                curved_inside_reverse: None,
                finish_soft: None,
                note: String::new(),
            }
        })
        .collect();
    doc
}

#[test]
#[ignore = "重い(数十秒〜数分)。cargo test --release -- --ignored --nocapture で実行"]
fn large_cp_operation_costs() {
    println!("\n=== 大規模CPでの1操作あたりの実測(GATE={GATE:?}) ===");
    // 打ち切りフラグ(処理ごと)
    let mut stop = [false; 8];
    for &n in &SIZES {
        let cp = miura_cp(n, n);
        let (faces, dt) = timed(|| extract_faces(&cp));
        let (e, f) = (cp.edges.len(), faces.len());
        row("extract_faces", n, e, f, dt, "");
        stop[1] |= dt > GATE;

        // 1) insert_segment: その規模のCPへ1本追加(1マスの対角線=交差なし)
        if !stop[0] {
            let d = 0.5 / n as f64;
            let (_, dt) = timed(|| {
                let mut c = cp.clone();
                insert_segment(&mut c, [d, d], [d * 2.0, d * 2.0], EdgeKind::Aux)
            });
            row("insert_segment x1", n, e, f, dt, "1本追加");
            stop[0] |= dt > GATE;
        }

        // 3) validate
        if !stop[2] {
            let (w, dt) = timed(|| validate(&cp));
            row("validate", n, e, f, dt, &format!("警告{}件", w.len()));
            stop[2] |= dt > GATE;
        }

        // 4) local_violations
        if !stop[3] {
            let (v, dt) = timed(|| local_violations(&cp));
            row(
                "local_violations",
                n,
                e,
                f,
                dt,
                &format!("違反{}点", v.len()),
            );
            stop[3] |= dt > GATE;
        }

        // 8) cp_svg
        let doc = doc_with_steps(cp.clone(), n);
        if !stop[7] {
            let (s, dt) = timed(|| cp_svg(&doc, &CpSvgOptions::default()));
            row("cp_svg", n, e, f, dt, &format!("{} KB", s.len() / 1024));
            stop[7] |= dt > GATE;
        }

        // 5) solve(中央付近の縦ヒンジ1本をdriverに)
        let mut frame = None;
        if !stop[4] {
            let hinge = (n / 2 * (n + 1) + n / 2) as u32;
            let drv = vec![Driver {
                hinge,
                target_angle_deg: 20.0,
            }];
            let (res, dt) = timed(|| solve(&cp, &faces, &drv, None));
            row(
                "solve (cold)",
                n,
                e,
                f,
                dt,
                &format!("反復{} 収束{}", res.iterations, res.converged),
            );
            stop[4] |= dt > GATE;
            frame = Some(res.frame);
        }

        // 7) self_intersects
        if !stop[6]
            && let Some(fr) = &frame
        {
            let (hit, dt) = timed(|| self_intersects(fr));
            row("self_intersects", n, e, f, dt, &format!("めり込み{hit}"));
            stop[6] |= dt > GATE;
        }

        // 6) replay / flat_state_at(手順3個)
        if !stop[5] {
            let (r, dt) = timed(|| replay(&doc, 3, 1.0));
            row(
                "replay(3手順)",
                n,
                e,
                f,
                dt,
                &format!("警告{}", r.warnings.len()),
            );
            stop[5] |= dt > GATE;
            let (st, dt) = timed(|| flat_state_at(&doc, &faces, 3));
            row(
                "flat_state_at",
                n,
                e,
                f,
                dt,
                if st.is_ok() { "ok" } else { "平坦でない" },
            );
        }

        // 概算メモリ(CP+面+半辺の実体サイズ)
        let mem = cp.vertices.len() * 24
            + e * 16
            + faces.iter().map(|x| x.vertices.len() * 8).sum::<usize>();
        println!("|   memory(CP+faces)   | n={n:3} | 約 {} KB", mem / 1024);
    }
    for (i, s) in stop.iter().enumerate() {
        if *s {
            println!("打ち切り: 処理#{i} は上記の最大規模でGATE超過");
        }
    }
}

/// 実際に線を引いて規模を作る過程(insert_segmentの繰り返し)の総時間。
/// 正方形の紙へ縦(n-1)本・横(n-1)本の直線を引くと辺は 2·n·(n+1) 本になる。
#[test]
#[ignore = "重い。cargo test --release -- --ignored --nocapture で実行"]
fn building_by_insert_segment() {
    println!("\n=== insert_segmentの繰り返しで規模を作る(GATE={GATE:?}) ===");
    for &n in &SIZES {
        let mut cp = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        })
        .cp;
        let mut last = Duration::ZERO;
        let t0 = Instant::now();
        for i in 1..n {
            let x = i as f64 / n as f64;
            let (_, dt) = timed(|| insert_segment(&mut cp, [x, 0.0], [x, 1.0], EdgeKind::Mountain));
            last = dt;
        }
        for j in 1..n {
            let y = j as f64 / n as f64;
            let (_, dt) = timed(|| insert_segment(&mut cp, [0.0, y], [1.0, y], EdgeKind::Valley));
            last = dt;
        }
        let total = t0.elapsed();
        println!(
            "| build n={n:3} | 辺{:6} 点{:6} | 呼び出し{:4}回 | 合計 {:>9.1} ms | 最後の1本 {:>8.1} ms",
            cp.edges.len(),
            cp.vertices.len(),
            2 * (n - 1),
            total.as_secs_f64() * 1e3,
            last.as_secs_f64() * 1e3
        );
        if total > GATE {
            println!("打ち切り: n={n}(辺{})でGATE超過", cp.edges.len());
            break;
        }
    }
}
