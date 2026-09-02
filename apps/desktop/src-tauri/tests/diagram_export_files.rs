//! 書き出しダイアログが選ぶ共有製品writerについて、実ファイルの構造を固定する。
//!
//! UI側の検査は`DiagramPdf`/`DiagramSvg`と保存先がdesktop commandへ届くことを
//! 固定する。この検査は同じcommandが使う`ori3-export`の成果物を実際に一時
//! directoryへ書いて読み直し、空のPDF/SVGを成功扱いにしない。

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use ori3_export::{diagram_pdf, diagram_svg_pages};
use ori3_model::{Document, DriverLine, Edge, EdgeKind, FoldStep, Paper, TechniqueKind, Vertex};

struct TempExportDir(PathBuf);

impl TempExportDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("現在時刻はUNIX epoch以後")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ori3-diagram-export-files-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("折り図の一時書き出し先を作れる");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempExportDir {
    fn drop(&mut self) {
        let temp = std::env::temp_dir();
        if self.0.starts_with(&temp)
            && self.0.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("ori3-diagram-export-files-")
            })
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn one_fold_document() -> Document {
    let mut document = Document::new(Paper {
        width_mm: 200.0,
        height_mm: 200.0,
    });
    document.cp.vertices = vec![
        Vertex {
            id: 0,
            pos: [0.0, 0.0],
        },
        Vertex {
            id: 1,
            pos: [0.0, 1.0],
        },
        Vertex {
            id: 2,
            pos: [0.5, 0.0],
        },
        Vertex {
            id: 3,
            pos: [0.5, 1.0],
        },
        Vertex {
            id: 4,
            pos: [1.0, 0.0],
        },
        Vertex {
            id: 5,
            pos: [1.0, 1.0],
        },
    ];
    document.cp.edges = vec![
        Edge {
            id: 0,
            v0: 0,
            v1: 2,
            kind: EdgeKind::Border,
        },
        Edge {
            id: 1,
            v0: 1,
            v1: 3,
            kind: EdgeKind::Border,
        },
        Edge {
            id: 2,
            v0: 2,
            v1: 4,
            kind: EdgeKind::Border,
        },
        Edge {
            id: 3,
            v0: 3,
            v1: 5,
            kind: EdgeKind::Border,
        },
        Edge {
            id: 4,
            v0: 0,
            v1: 1,
            kind: EdgeKind::Border,
        },
        Edge {
            id: 5,
            v0: 4,
            v1: 5,
            kind: EdgeKind::Border,
        },
        Edge {
            id: 6,
            v0: 2,
            v1: 3,
            kind: EdgeKind::Valley,
        },
    ];
    document.cp.next_vertex_id = 6;
    document.cp.next_edge_id = 7;
    document.sequence.push(FoldStep {
        id: 1,
        kind: TechniqueKind::Simple,
        drivers: vec![DriverLine {
            a: [0.5, 0.0],
            b: [0.5, 1.0],
            target_angle_deg: -180.0,
        }],
        layer_order: None,
        note: "中央の谷折り".to_owned(),
        alignment: None,
        curved_inside_reverse: None,
        finish_soft: None,
    });
    document
}

fn pdf_page_count(bytes: &[u8]) -> usize {
    String::from_utf8_lossy(bytes).matches("/MediaBox").count()
}

fn svg_shape_count(svg: &str) -> usize {
    [
        "<path ",
        "<polygon ",
        "<polyline ",
        "<line ",
        "<rect ",
        "<circle ",
        "<ellipse ",
    ]
    .into_iter()
    .map(|tag| svg.matches(tag).count())
    .sum()
}

#[test]
fn diagram_pdf_and_svg_create_nonempty_structured_files() {
    assert_eq!(
        pdf_page_count(b"%PDF-1.7\n%%EOF"),
        0,
        "印だけのPDFを頁ありと数えない"
    );
    assert_eq!(
        svg_shape_count("<svg></svg>"),
        0,
        "空のSVGを図形ありと数えない"
    );

    let document = one_fold_document();
    let output = TempExportDir::new();

    let pdf_path = output.path().join("折り図.pdf");
    let pdf = diagram_pdf(&document).expect("折り図PDFを生成できる");
    fs::write(&pdf_path, &pdf).expect("折り図PDFを実ファイルへ書ける");
    let saved_pdf = fs::read(&pdf_path).expect("書いた折り図PDFを読み直せる");
    assert!(!saved_pdf.is_empty(), "折り図PDFが0 byteではない");
    assert!(saved_pdf.starts_with(b"%PDF-"), "折り図PDFのsignature");
    let pdf_pages = pdf_page_count(&saved_pdf);
    assert!(pdf_pages >= 1, "折り図PDFに1頁以上ある");

    let pages = diagram_svg_pages(&document).expect("折り図SVGを生成できる");
    assert!(!pages.is_empty(), "折り図SVGに1頁以上ある");
    let mut svg_measurements = Vec::with_capacity(pages.len());
    for (index, page) in pages.iter().enumerate() {
        let path = output.path().join(format!("折り図-{:02}.svg", index + 1));
        fs::write(&path, page).expect("折り図SVGを実ファイルへ書ける");
        let saved_svg = fs::read_to_string(&path).expect("書いた折り図SVGを読み直せる");
        assert!(!saved_svg.is_empty(), "折り図SVGが0 byteではない: {path:?}");
        assert!(
            saved_svg.contains("<svg"),
            "折り図SVGのrootがある: {path:?}"
        );
        let shapes = svg_shape_count(&saved_svg);
        assert!(shapes >= 1, "折り図SVGに図形が1個以上ある: {path:?}");
        svg_measurements.push((saved_svg.len(), shapes));
    }

    println!(
        "diagram artifacts: pdf_bytes={} pdf_pages={} svg_pages={} svg(bytes,shapes)={svg_measurements:?}",
        saved_pdf.len(),
        pdf_pages,
        pages.len()
    );
}
