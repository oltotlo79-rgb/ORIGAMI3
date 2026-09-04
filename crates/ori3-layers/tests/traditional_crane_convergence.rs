//! 正本の折り鶴が、展開図から閉じた形として再生できることの受け入れ検査。
//!
//! 入力は追跡済み fixture の作品ファイルそのもの。依存を増やさないよう、
//! この検査の中に最小のJSON読取りを書いて読む。

use ori3_cp::extract_faces;
use ori3_model::{
    CreasePattern, DisplaySettings, Document, DriverLine, Edge, EdgeKind, FoldStep, Paper,
    TechniqueKind, Vertex,
};

// ---------------------------------------------------------------- 最小JSON読取り

#[derive(Clone, Debug)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> &Json {
        match self {
            Json::Obj(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value)
                .unwrap_or_else(|| panic!("JSONに{key}が無い")),
            _ => panic!("オブジェクトではない"),
        }
    }
    fn arr(&self) -> &[Json] {
        match self {
            Json::Arr(items) => items,
            _ => panic!("配列ではない"),
        }
    }
    fn num(&self) -> f64 {
        match self {
            Json::Num(value) => *value,
            _ => panic!("数値ではない"),
        }
    }
    fn text(&self) -> &str {
        match self {
            Json::Str(value) => value,
            _ => panic!("文字列ではない"),
        }
    }
    fn boolean(&self) -> bool {
        match self {
            Json::Bool(value) => *value,
            _ => panic!("真偽値ではない"),
        }
    }
    fn pair(&self) -> [f64; 2] {
        let items = self.arr();
        [items[0].num(), items[1].num()]
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Parser<'a> {
    fn skip(&mut self) {
        while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }
    fn value(&mut self) -> Json {
        self.skip();
        match self.bytes[self.at] {
            b'{' => {
                self.at += 1;
                let mut entries = Vec::new();
                loop {
                    self.skip();
                    if self.bytes[self.at] == b'}' {
                        self.at += 1;
                        break;
                    }
                    let key = match self.value() {
                        Json::Str(key) => key,
                        _ => panic!("キーが文字列でない"),
                    };
                    self.skip();
                    assert_eq!(self.bytes[self.at], b':');
                    self.at += 1;
                    let value = self.value();
                    entries.push((key, value));
                    self.skip();
                    if self.bytes[self.at] == b',' {
                        self.at += 1;
                    }
                }
                Json::Obj(entries)
            }
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.skip();
                    if self.bytes[self.at] == b']' {
                        self.at += 1;
                        break;
                    }
                    items.push(self.value());
                    self.skip();
                    if self.bytes[self.at] == b',' {
                        self.at += 1;
                    }
                }
                Json::Arr(items)
            }
            b'"' => {
                self.at += 1;
                let start = self.at;
                while self.bytes[self.at] != b'"' {
                    assert_ne!(self.bytes[self.at], b'\\', "この作品に脱出文字は無い");
                    self.at += 1;
                }
                let text = std::str::from_utf8(&self.bytes[start..self.at])
                    .expect("UTF-8")
                    .to_owned();
                self.at += 1;
                Json::Str(text)
            }
            b't' => {
                self.at += 4;
                Json::Bool(true)
            }
            b'f' => {
                self.at += 5;
                Json::Bool(false)
            }
            b'n' => {
                self.at += 4;
                Json::Null
            }
            _ => {
                let start = self.at;
                while self.at < self.bytes.len()
                    && matches!(self.bytes[self.at],
                        b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                {
                    self.at += 1;
                }
                Json::Num(
                    std::str::from_utf8(&self.bytes[start..self.at])
                        .expect("UTF-8")
                        .parse::<f64>()
                        .expect("数値"),
                )
            }
        }
    }
}

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/traditional-crane")
        .join(name)
}

/// 追跡済みfixtureの正本作品をそのまま読む。
fn traditional_crane_document() -> Document {
    let text = std::fs::read_to_string(fixture_path("traditional-crane-cp.ori3"))
        .expect("正本CP作品fixtureを読む");
    let mut parser = Parser {
        bytes: text.as_bytes(),
        at: 0,
    };
    let root = parser.value();

    let cp_json = root.get("cp");
    let vertices = cp_json
        .get("vertices")
        .arr()
        .iter()
        .map(|vertex| Vertex {
            id: vertex.get("id").num() as u32,
            pos: vertex.get("pos").pair(),
        })
        .collect::<Vec<_>>();
    let edges = cp_json
        .get("edges")
        .arr()
        .iter()
        .map(|edge| Edge {
            id: edge.get("id").num() as u32,
            v0: edge.get("v0").num() as u32,
            v1: edge.get("v1").num() as u32,
            kind: match edge.get("kind").text() {
                "Border" => EdgeKind::Border,
                "Mountain" => EdgeKind::Mountain,
                "Valley" => EdgeKind::Valley,
                "Aux" => EdgeKind::Aux,
                other => panic!("未知の折り線種別{other}"),
            },
        })
        .collect::<Vec<_>>();
    let sequence = root
        .get("sequence")
        .arr()
        .iter()
        .map(|step| FoldStep {
            id: step.get("id").num() as u32,
            kind: match step.get("kind").text() {
                "Twist" => TechniqueKind::Twist,
                "Simple" => TechniqueKind::Simple,
                "Pose" => TechniqueKind::Pose,
                other => panic!("この作品に無い技法{other}"),
            },
            drivers: step
                .get("drivers")
                .arr()
                .iter()
                .map(|driver| DriverLine {
                    a: driver.get("a").pair(),
                    b: driver.get("b").pair(),
                    target_angle_deg: driver.get("target_angle_deg").num(),
                })
                .collect(),
            layer_order: Some(
                step.get("layer_order")
                    .arr()
                    .iter()
                    .map(Json::pair)
                    .collect(),
            ),
            alignment: None,
            finish_soft: None,
            note: step.get("note").text().to_owned(),
        })
        .collect::<Vec<_>>();
    let display_json = root.get("display");
    let color = |value: &Json| {
        let items = value.arr();
        [
            items[0].num() as u8,
            items[1].num() as u8,
            items[2].num() as u8,
        ]
    };
    Document {
        schema_version: root.get("schema_version").num() as u32,
        paper: Paper {
            width_mm: root.get("paper").get("width_mm").num(),
            height_mm: root.get("paper").get("height_mm").num(),
        },
        cp: CreasePattern {
            vertices,
            edges,
            next_vertex_id: cp_json.get("next_vertex_id").num() as u32,
            next_edge_id: cp_json.get("next_edge_id").num() as u32,
        },
        sequence,
        display: DisplaySettings {
            front_color: color(display_json.get("front_color")),
            back_color: color(display_json.get("back_color")),
            grid_divisions: display_json.get("grid_divisions").num() as u32,
            soft_enabled: display_json.get("soft_enabled").boolean(),
            soft_stiffness: display_json.get("soft_stiffness").num(),
            soft_pressure: display_json.get("soft_pressure").num(),
            overlap_prevention_enabled: display_json
                .get("overlap_prevention_enabled")
                .boolean(),
            penetration_prevention_enabled: display_json
                .get("penetration_prevention_enabled")
                .boolean(),
        },
    }
}

// ---------------------------------------------------------------- 検査

/// 正本の折り鶴を通常の再生経路で折り上げたとき、展開図から形が求まりきることを表明する。
///
/// 座標そのものは書かない。計算機ごとに最下位の桁が変わり得るためで(§10.7.7)、
/// 「閉じた」「警告が無い」「何度やっても同じ」という性質だけを条件にする。
///
/// 2026-09-03より前は、正本CSVの小数12桁の座標をそのまま使っていたため
/// 展開図が平坦に折れる条件を 2.6e-11 rad 破っており、閉包残差が
/// 7.218742174998615e-12 で止まって警告が1件出ていた。利用者の承認を受けて
/// 座標を12桁の丸め幅の内側で置き直し、残差は 1e-13 を下回った。
#[test]
fn traditional_crane_replays_closed_without_warnings() {
    /// 剛体ソルバーの収束判定と同じ上限。
    const CLOSURE_LIMIT: f64 = 1e-13;
    /// 平らに畳んだ形の厚み。
    const FLAT_LIMIT: f64 = 1e-9;
    /// 紙のちぎれ。
    const SEAM_LIMIT: f64 = 1e-9;

    let document = traditional_crane_document();
    assert_eq!(document.cp.vertices.len(), 56);
    assert_eq!(document.cp.edges.len(), 114);
    assert_eq!(document.sequence.len(), 1);
    assert_eq!(document.sequence[0].drivers.len(), 102);
    let faces = extract_faces(&document.cp);
    assert_eq!(faces.len(), 59);

    let replayed = ori3_layers::replay(&document, document.sequence.len(), 1.0);
    assert!(
        replayed.converged,
        "展開図から形が求まる: closure_rms={} warnings={:?}",
        replayed.closure_rms, replayed.warnings
    );
    assert!(!replayed.best_effort, "最良近似ではなく本解を返す");
    assert!(
        replayed.closure_rms < CLOSURE_LIMIT,
        "閉包残差{}が上限{CLOSURE_LIMIT:e}以上",
        replayed.closure_rms
    );
    assert!(
        replayed.warnings.is_empty(),
        "警告0: {:?}",
        replayed.warnings
    );
    assert!(replayed.skipped.is_empty(), "飛ばした手順0");

    let max_z = replayed
        .frame
        .faces
        .iter()
        .flat_map(|face| &face.polygon)
        .map(|point| point[2].abs())
        .fold(0.0_f64, f64::max);
    assert!(max_z <= FLAT_LIMIT, "平らに畳めている: max|z|={max_z:e}");
    let seam = ori3_rigid::max_seam_gap(&document.cp, &faces, &replayed.frame);
    assert!(seam <= SEAM_LIMIT, "紙がちぎれていない: 継ぎ目={seam:e}");

    // 同じ入力を10回再生して、形が1ビットも変わらないこと。
    let bits = |result: &ori3_layers::ReplayResult| -> Vec<u64> {
        result
            .frame
            .faces
            .iter()
            .flat_map(|face| face.polygon.iter().flat_map(|point| point.iter()))
            .map(|value| value.to_bits())
            .collect()
    };
    let first = bits(&replayed);
    assert!(!first.is_empty(), "形に頂点がある");
    for round in 1..10 {
        let again = ori3_layers::replay(&document, document.sequence.len(), 1.0);
        assert!(again.converged && again.warnings.is_empty());
        assert_eq!(bits(&again), first, "{round}回目の再生で形が変わった");
    }
}
