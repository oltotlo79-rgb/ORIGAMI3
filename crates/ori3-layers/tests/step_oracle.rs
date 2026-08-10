use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_layers::step_oracle::{
    FoldSense, LandmarkExpectation, LandmarkKind, LayerCountExpectation, StepDifference,
    StepExpectation, evaluate_step, extract_step_features, layer_count_at,
};
use ori3_layers::{FlatState, flat_state_at, representative_point};
use ori3_model::{
    CreasePattern, DisplaySettings, Document, DriverLine, Edge, EdgeKind, FoldStep, Paper,
    TechniqueKind, Vertex,
};

const ROSE_011: &str = include_str!("fixtures/rose-011.ori3");
const ROSE_029: &str = include_str!("fixtures/rose-029.ori3");
const DEVIL_024: &str = include_str!("fixtures/folded-sample.ori3");

#[test]
fn extracts_features_from_completed_rose_and_devil_fixtures() {
    for (name, json) in [("rose-029", ROSE_029), ("devil-024", DEVIL_024)] {
        let document = load_fixture(json);
        let (faces, state) = state_of(&document);
        let features = extract_step_features(&document.cp, &faces, &state);
        let probe = top_face_probe(&document, &faces, &state);

        assert!(features.outline.len() >= 3, "{name}: 輪郭を抽出できる");
        assert!(
            features.bounding_box.width() > 0.0,
            "{name}: 輪郭に幅がある"
        );
        assert!(
            features.bounding_box.height() > 0.0,
            "{name}: 輪郭に高さがある"
        );
        assert!(
            features.outline_area_ratio > 0.0,
            "{name}: 紙全体に対する面積比を抽出できる"
        );
        assert!(
            features
                .landmarks
                .iter()
                .any(|feature| feature.kind == LandmarkKind::PaperCorner),
            "{name}: 紙の角を抽出できる"
        );
        assert!(
            !features.visible_creases.is_empty(),
            "{name}: 上から見える山谷を抽出できる"
        );
        assert!(
            layer_count_at(&document.cp, &faces, &state, probe) >= 1,
            "{name}: 局所層数を抽出できる"
        );
    }
}

#[test]
fn state_matches_expectation_captured_from_itself() {
    let document = load_fixture(ROSE_029);
    let (faces, state) = state_of(&document);
    let probe = top_face_probe(&document, &faces, &state);
    let expectation = StepExpectation::from_state(&document.cp, &faces, &state, &[probe]);

    let report = evaluate_step(&document.cp, &faces, &state, &expectation);

    assert!(
        report.is_match(),
        "自己照合の差分: {:?}",
        report.explanations()
    );
    assert!(report.differences.is_empty());
    assert_eq!(report.layer_samples.len(), 1);
}

#[test]
fn earlier_rose_is_rejected_by_completed_rose_expectation() {
    let completed = load_fixture(ROSE_029);
    let (completed_faces, completed_state) = state_of(&completed);
    let probe = top_face_probe(&completed, &completed_faces, &completed_state);
    let expectation =
        StepExpectation::from_state(&completed.cp, &completed_faces, &completed_state, &[probe]);

    let earlier = load_fixture(ROSE_011);
    let (earlier_faces, earlier_state) = state_of(&earlier);
    let report = evaluate_step(&earlier.cp, &earlier_faces, &earlier_state, &expectation);

    assert!(!report.is_match(), "手順11を完成形として受理してはいけない");
    assert!(
        report.differences.iter().any(|difference| matches!(
            difference,
            StepDifference::OutlineVertexCount { .. }
                | StepDifference::OutlineVertexPosition { .. }
                | StepDifference::BoundingBox { .. }
                | StepDifference::AspectRatio { .. }
                | StepDifference::OutlineAreaRatio { .. }
                | StepDifference::LayerCount { .. }
        )),
        "形または層数の具体的な差分を返す: {:?}",
        report.explanations()
    );
    let explanations = report.explanations().join("\n");
    assert!(explanations.contains("期待"));
    assert!(explanations.contains("実際"));
}

#[test]
fn reports_landmark_layer_count_and_visible_fold_sense_differences() {
    let document = load_fixture(DEVIL_024);
    let (faces, state) = state_of(&document);
    let features = extract_step_features(&document.cp, &faces, &state);
    let probe = top_face_probe(&document, &faces, &state);
    let actual_layers = layer_count_at(&document.cp, &faces, &state, probe);
    let visible = features
        .visible_creases
        .first()
        .expect("devil-024には可視折り目がある");
    let visible_midpoint = [
        (visible.segment[0][0] + visible.segment[1][0]) * 0.5,
        (visible.segment[0][1] + visible.segment[1][1]) * 0.5,
    ];
    let wrong_sense = match visible.kind {
        FoldSense::Mountain => FoldSense::Valley,
        FoldSense::Valley => FoldSense::Mountain,
    };
    let expectation = StepExpectation {
        landmarks: vec![LandmarkExpectation {
            position: [100.0, 100.0],
            kind: LandmarkKind::PaperCorner,
        }],
        layer_counts: vec![LayerCountExpectation {
            position: probe,
            count: actual_layers + 1,
        }],
        visible_creases: vec![ori3_layers::step_oracle::VisibleCreaseExpectation {
            position: visible_midpoint,
            kind: wrong_sense,
        }],
        ..StepExpectation::default()
    };

    let report = evaluate_step(&document.cp, &faces, &state, &expectation);

    assert!(
        report
            .differences
            .iter()
            .any(|difference| matches!(difference, StepDifference::LandmarkMissing { .. }))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| matches!(difference, StepDifference::LayerCount { .. }))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| matches!(difference, StepDifference::VisibleCreaseSense { .. }))
    );
}

fn state_of(document: &Document) -> (Vec<Face>, FlatState) {
    let faces = extract_faces(&document.cp);
    let (state, _) = flat_state_at(document, &faces, document.sequence.len())
        .expect("fixtureを平坦状態として再生できる");
    assert_eq!(state.order.len(), faces.len(), "全ての面に層順がある");
    (faces, state)
}

fn top_face_probe(document: &Document, faces: &[Face], state: &FlatState) -> [f64; 2] {
    let face_id = *state.order.last().expect("面が一つ以上ある");
    let face = faces
        .iter()
        .find(|face| face.id == face_id)
        .expect("最上面が存在する");
    let local = representative_point(&document.cp, face);
    state.placements[&face_id]
        .apply(DVec2::from(local))
        .to_array()
}

/// The crate intentionally has no serde_json dependency.  This small fixture
/// reader covers the persisted fields needed to reconstruct a `Document` and
/// skips forward-compatible fields such as rose soft geometry.
fn load_fixture(json: &str) -> Document {
    FixtureParser::new(json).document()
}

struct FixtureParser<'a> {
    input: &'a [u8],
    cursor: usize,
}

impl<'a> FixtureParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            cursor: 0,
        }
    }

    fn document(mut self) -> Document {
        self.expect(b'{');
        let mut first = true;
        let mut schema_version = 1;
        let mut paper = None;
        let mut cp = None;
        let mut sequence = Vec::new();
        while let Some(key) = self.next_object_key(&mut first) {
            match key.as_str() {
                "schema_version" => schema_version = self.u32(),
                "paper" => paper = Some(self.paper()),
                "cp" => cp = Some(self.crease_pattern()),
                "sequence" => sequence = self.fold_steps(),
                _ => self.skip_value(),
            }
        }
        self.skip_whitespace();
        assert_eq!(self.cursor, self.input.len(), "fixture末尾まで読み取る");
        Document {
            schema_version,
            paper: paper.expect("paper"),
            cp: cp.expect("cp"),
            sequence,
            display: DisplaySettings::default(),
        }
    }

    fn paper(&mut self) -> Paper {
        self.expect(b'{');
        let mut first = true;
        let mut width_mm = None;
        let mut height_mm = None;
        while let Some(key) = self.next_object_key(&mut first) {
            match key.as_str() {
                "width_mm" => width_mm = Some(self.number()),
                "height_mm" => height_mm = Some(self.number()),
                _ => self.skip_value(),
            }
        }
        Paper {
            width_mm: width_mm.expect("paper.width_mm"),
            height_mm: height_mm.expect("paper.height_mm"),
        }
    }

    fn crease_pattern(&mut self) -> CreasePattern {
        self.expect(b'{');
        let mut first = true;
        let mut vertices = Vec::new();
        let mut edges = Vec::new();
        let mut next_vertex_id = None;
        let mut next_edge_id = None;
        while let Some(key) = self.next_object_key(&mut first) {
            match key.as_str() {
                "vertices" => vertices = self.vertices(),
                "edges" => edges = self.edges(),
                "next_vertex_id" => next_vertex_id = Some(self.u32()),
                "next_edge_id" => next_edge_id = Some(self.u32()),
                _ => self.skip_value(),
            }
        }
        let inferred_vertex_id = vertices
            .iter()
            .map(|vertex| vertex.id)
            .max()
            .map_or(0, |id| id + 1);
        let inferred_edge_id = edges
            .iter()
            .map(|edge| edge.id)
            .max()
            .map_or(0, |id| id + 1);
        CreasePattern {
            vertices,
            edges,
            next_vertex_id: next_vertex_id.unwrap_or(inferred_vertex_id),
            next_edge_id: next_edge_id.unwrap_or(inferred_edge_id),
        }
    }

    fn vertices(&mut self) -> Vec<Vertex> {
        self.expect(b'[');
        let mut first = true;
        let mut vertices = Vec::new();
        while self.next_array_value(&mut first) {
            self.expect(b'{');
            let mut field_first = true;
            let mut id = None;
            let mut pos = None;
            while let Some(key) = self.next_object_key(&mut field_first) {
                match key.as_str() {
                    "id" => id = Some(self.u32()),
                    "pos" => pos = Some(self.point()),
                    _ => self.skip_value(),
                }
            }
            vertices.push(Vertex {
                id: id.expect("vertex.id"),
                pos: pos.expect("vertex.pos"),
            });
        }
        vertices
    }

    fn edges(&mut self) -> Vec<Edge> {
        self.expect(b'[');
        let mut first = true;
        let mut edges = Vec::new();
        while self.next_array_value(&mut first) {
            self.expect(b'{');
            let mut field_first = true;
            let mut id = None;
            let mut v0 = None;
            let mut v1 = None;
            let mut kind = None;
            while let Some(key) = self.next_object_key(&mut field_first) {
                match key.as_str() {
                    "id" => id = Some(self.u32()),
                    "v0" => v0 = Some(self.u32()),
                    "v1" => v1 = Some(self.u32()),
                    "kind" => kind = Some(self.edge_kind()),
                    _ => self.skip_value(),
                }
            }
            edges.push(Edge {
                id: id.expect("edge.id"),
                v0: v0.expect("edge.v0"),
                v1: v1.expect("edge.v1"),
                kind: kind.expect("edge.kind"),
            });
        }
        edges
    }

    fn fold_steps(&mut self) -> Vec<FoldStep> {
        self.expect(b'[');
        let mut first = true;
        let mut steps = Vec::new();
        while self.next_array_value(&mut first) {
            self.expect(b'{');
            let mut field_first = true;
            let mut id = None;
            let mut kind = None;
            let mut drivers = Vec::new();
            let mut layer_order = None;
            let mut note = String::new();
            while let Some(key) = self.next_object_key(&mut field_first) {
                match key.as_str() {
                    "id" => id = Some(self.u32()),
                    "kind" => kind = Some(self.technique_kind()),
                    "drivers" => drivers = self.drivers(),
                    "layer_order" => layer_order = self.optional_points(),
                    "note" => note = self.string(),
                    _ => self.skip_value(),
                }
            }
            steps.push(FoldStep {
                id: id.expect("step.id"),
                kind: kind.expect("step.kind"),
                drivers,
                layer_order,
                alignment: None,
                note,
            });
        }
        steps
    }

    fn drivers(&mut self) -> Vec<DriverLine> {
        self.expect(b'[');
        let mut first = true;
        let mut drivers = Vec::new();
        while self.next_array_value(&mut first) {
            self.expect(b'{');
            let mut field_first = true;
            let mut a = None;
            let mut b = None;
            let mut target_angle_deg = None;
            while let Some(key) = self.next_object_key(&mut field_first) {
                match key.as_str() {
                    "a" => a = Some(self.point()),
                    "b" => b = Some(self.point()),
                    "target_angle_deg" => target_angle_deg = Some(self.number()),
                    _ => self.skip_value(),
                }
            }
            drivers.push(DriverLine {
                a: a.expect("driver.a"),
                b: b.expect("driver.b"),
                target_angle_deg: target_angle_deg.expect("driver.target_angle_deg"),
            });
        }
        drivers
    }

    fn optional_points(&mut self) -> Option<Vec<[f64; 2]>> {
        self.skip_whitespace();
        if self.peek() == Some(b'n') {
            self.literal(b"null");
            return None;
        }
        self.expect(b'[');
        let mut first = true;
        let mut points = Vec::new();
        while self.next_array_value(&mut first) {
            points.push(self.point());
        }
        Some(points)
    }

    fn point(&mut self) -> [f64; 2] {
        self.expect(b'[');
        let x = self.number();
        self.expect(b',');
        let y = self.number();
        self.expect(b']');
        [x, y]
    }

    fn edge_kind(&mut self) -> EdgeKind {
        match self.string().as_str() {
            "Border" => EdgeKind::Border,
            "Mountain" => EdgeKind::Mountain,
            "Valley" => EdgeKind::Valley,
            "Aux" => EdgeKind::Aux,
            kind => panic!("未知のEdgeKind: {kind}"),
        }
    }

    fn technique_kind(&mut self) -> TechniqueKind {
        match self.string().as_str() {
            "Simple" => TechniqueKind::Simple,
            "Pleat" => TechniqueKind::Pleat,
            "InsideReverse" => TechniqueKind::InsideReverse,
            "OutsideReverse" => TechniqueKind::OutsideReverse,
            "Petal" => TechniqueKind::Petal,
            "Squash" => TechniqueKind::Squash,
            "OpenSink" => TechniqueKind::OpenSink,
            "Swivel" => TechniqueKind::Swivel,
            "Twist" => TechniqueKind::Twist,
            "Pose" => TechniqueKind::Pose,
            kind => panic!("未知のTechniqueKind: {kind}"),
        }
    }

    fn next_object_key(&mut self, first: &mut bool) -> Option<String> {
        self.skip_whitespace();
        if self.consume(b'}') {
            return None;
        }
        if *first {
            *first = false;
        } else {
            self.expect(b',');
        }
        let key = self.string();
        self.expect(b':');
        Some(key)
    }

    fn next_array_value(&mut self, first: &mut bool) -> bool {
        self.skip_whitespace();
        if self.consume(b']') {
            return false;
        }
        if *first {
            *first = false;
        } else {
            self.expect(b',');
        }
        true
    }

    fn skip_value(&mut self) {
        self.skip_whitespace();
        match self.peek().expect("JSON value") {
            b'{' => {
                self.expect(b'{');
                let mut first = true;
                while self.next_object_key(&mut first).is_some() {
                    self.skip_value();
                }
            }
            b'[' => {
                self.expect(b'[');
                let mut first = true;
                while self.next_array_value(&mut first) {
                    self.skip_value();
                }
            }
            b'"' => {
                self.string();
            }
            b't' => self.literal(b"true"),
            b'f' => self.literal(b"false"),
            b'n' => self.literal(b"null"),
            b'-' | b'0'..=b'9' => {
                self.number();
            }
            byte => panic!("未知のJSON値開始: {byte}"),
        }
    }

    fn string(&mut self) -> String {
        self.expect(b'"');
        let mut output = Vec::new();
        loop {
            let byte = self.take().expect("文字列終端");
            match byte {
                b'"' => break,
                b'\\' => {
                    let escaped = self.take().expect("escape文字");
                    match escaped {
                        b'"' | b'\\' | b'/' => output.push(escaped),
                        b'b' => output.push(8),
                        b'f' => output.push(12),
                        b'n' => output.push(b'\n'),
                        b'r' => output.push(b'\r'),
                        b't' => output.push(b'\t'),
                        b'u' => {
                            let code = (0..4).fold(0_u32, |value, _| {
                                value * 16 + hex_value(self.take().expect("unicode escape"))
                            });
                            let character =
                                char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER);
                            let mut encoded = [0; 4];
                            output
                                .extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
                        }
                        _ => panic!("未知のescape"),
                    }
                }
                _ => output.push(byte),
            }
        }
        String::from_utf8(output).expect("fixture文字列はUTF-8")
    }

    fn u32(&mut self) -> u32 {
        let value = self.number();
        assert!(value >= 0.0 && value.fract() == 0.0, "u32値: {value}");
        value as u32
    }

    fn number(&mut self) -> f64 {
        self.skip_whitespace();
        let start = self.cursor;
        while self
            .peek()
            .is_some_and(|byte| matches!(byte, b'+' | b'-' | b'.' | b'e' | b'E' | b'0'..=b'9'))
        {
            self.cursor += 1;
        }
        assert!(self.cursor > start, "数値が必要");
        std::str::from_utf8(&self.input[start..self.cursor])
            .expect("数値はASCII")
            .parse()
            .expect("正しいJSON数値")
    }

    fn literal(&mut self, literal: &[u8]) {
        self.skip_whitespace();
        let end = self.cursor + literal.len();
        assert_eq!(self.input.get(self.cursor..end), Some(literal));
        self.cursor = end;
    }

    fn expect(&mut self, expected: u8) {
        self.skip_whitespace();
        assert_eq!(self.take(), Some(expected));
    }

    fn consume(&mut self, expected: u8) -> bool {
        self.skip_whitespace();
        if self.peek() == Some(expected) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.cursor += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.cursor).copied()
    }

    fn take(&mut self) -> Option<u8> {
        let value = self.peek()?;
        self.cursor += 1;
        Some(value)
    }
}

fn hex_value(byte: u8) -> u32 {
    match byte {
        b'0'..=b'9' => u32::from(byte - b'0'),
        b'a'..=b'f' => u32::from(byte - b'a' + 10),
        b'A'..=b'F' => u32::from(byte - b'A' + 10),
        _ => panic!("16進数が必要"),
    }
}
