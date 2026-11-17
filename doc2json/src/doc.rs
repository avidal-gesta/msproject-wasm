//! Modelo comun de documento (Word, PDF, PowerPoint o Excel) y su salida JSON.
//!
//! Los dos formatos terminan en la misma forma: metadatos, una lista de bloques de contenido
//! (titulos, parrafos, items de lista, tablas) y el texto plano completo. Asi una sola tabla
//! en la base de datos sirve para ambos.

use crate::json::{self, Obj};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Kind {
    Title,
    Heading,
    #[default]
    Paragraph,
    ListItem,
    Table,
    Image,
    PageBreak,
}

#[derive(Clone, Default, Debug)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub link: Option<String>,
}

#[derive(Clone, Default, Debug)]
pub struct Cell {
    pub text: String,
    pub colspan: u32,
    pub rowspan: u32,
    /// Celda cubierta por una combinacion vertical iniciada mas arriba.
    pub merged: bool,
}

#[derive(Clone, Default, Debug)]
pub struct Block {
    pub kind: Kind,
    pub text: String,
    /// Nivel del titulo (1 = principal) o nivel de anidamiento del item de lista (0 = primero).
    pub level: u8,
    pub ordered: bool,
    /// Numeracion o vineta visible: "1.1", "a)", "•".
    pub marker: Option<String>,
    pub style: Option<String>,
    pub align: Option<String>,
    pub page: Option<u32>,
    pub runs: Vec<Run>,
    pub rows: Vec<Vec<Cell>>,
    /// Imagen: nombre del archivo incrustado.
    pub name: Option<String>,
    /// Nota al pie o comentario: numero o autor.
    pub label: Option<String>,
    /// PDF: tamano de letra en puntos y si esta en negrita.
    pub size: Option<f64>,
    pub bold: bool,
}

#[derive(Clone, Debug)]
pub enum Meta {
    Text(String),
    Int(i64),
    Bool(bool),
}

#[derive(Clone, Default, Debug)]
pub struct Line {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub bold: bool,
}

#[derive(Clone, Default, Debug)]
pub struct Page {
    pub number: u32,
    pub width: f64,
    pub height: f64,
    pub text: String,
    pub lines: Vec<Line>,
    pub images: usize,
}

#[derive(Clone, Default, Debug)]
pub struct OutlineItem {
    pub title: String,
    pub level: u8,
    pub page: Option<u32>,
}

#[derive(Default, Debug)]
pub struct Document {
    pub kind: &'static str,
    pub meta: Vec<(&'static str, Meta)>,
    pub page_count: Option<usize>,
    pub content: Vec<Block>,
    pub pages: Vec<Page>,
    /// Partes secundarias: encabezados, pies, notas al pie, notas finales, comentarios.
    pub extras: Vec<(&'static str, Vec<Block>)>,
    pub outline: Vec<OutlineItem>,
    pub warnings: Vec<String>,
}

impl Document {
    pub fn meta_text(&mut self, key: &'static str, value: Option<String>) {
        if let Some(v) = value.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            self.meta.push((key, Meta::Text(v)));
        }
    }

    pub fn meta_int(&mut self, key: &'static str, value: Option<i64>) {
        if let Some(v) = value {
            self.meta.push((key, Meta::Int(v)));
        }
    }
}

/// Texto plano de un bloque, con su numeracion o vineta delante.
pub fn block_text(b: &Block) -> String {
    match b.kind {
        Kind::Table => b.rows.iter().map(|r| r.iter().map(|c| c.text.replace('\n', " ")).collect::<Vec<_>>().join("\t")).collect::<Vec<_>>().join("\n"),
        Kind::PageBreak | Kind::Image => String::new(),
        _ => match &b.marker {
            Some(m) if !m.is_empty() => format!("{} {}", m, b.text),
            _ => b.text.clone(),
        },
    }
}

pub fn plain_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        let t = block_text(b);
        if t.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&t);
    }
    out
}

fn block_json(b: &Block, full: bool) -> String {
    let mut o = Obj::new();
    o.str(
        "type",
        match b.kind {
            Kind::Title => "title",
            Kind::Heading => "heading",
            Kind::Paragraph => "paragraph",
            Kind::ListItem => "list_item",
            Kind::Table => "table",
            Kind::Image => "image",
            Kind::PageBreak => "page_break",
        },
    );
    match b.kind {
        Kind::Heading => {
            o.int("level", b.level as i64);
        }
        Kind::ListItem => {
            o.int("level", b.level as i64).bool("ordered", b.ordered);
        }
        _ => {}
    }
    o.opt("marker", b.marker.as_deref());
    o.opt("label", b.label.as_deref());
    match b.kind {
        Kind::Table => {
            let rows: Vec<String> = b
                .rows
                .iter()
                .map(|r| {
                    let cells: Vec<String> = r
                        .iter()
                        .map(|c| {
                            let mut co = Obj::new();
                            co.str("text", &c.text);
                            if c.colspan > 1 {
                                co.int("colspan", c.colspan as i64);
                            }
                            if c.rowspan > 1 {
                                co.int("rowspan", c.rowspan as i64);
                            }
                            if c.merged {
                                co.bool("merged", true);
                            }
                            co.end()
                        })
                        .collect();
                    json::array(&cells)
                })
                .collect();
            o.raw("rows", &json::array(&rows));
        }
        Kind::PageBreak => {}
        Kind::Image => {
            o.opt("name", b.name.as_deref());
            o.opt("alt", Some(b.text.as_str()));
        }
        _ => {
            o.str("text", &b.text);
        }
    }
    if let Some(p) = b.page {
        o.int("page", p as i64);
    }
    if full {
        o.opt("style", b.style.as_deref());
        o.opt("align", b.align.as_deref());
        if let Some(s) = b.size {
            o.num("size", s);
        }
        if b.bold {
            o.bool("bold", true);
        }
        // Los tramos solo aportan si alguno trae formato o enlace.
        if b.runs.iter().any(|r| r.bold || r.italic || r.underline || r.strike || r.link.is_some()) {
            let runs: Vec<String> = b
                .runs
                .iter()
                .map(|r| {
                    let mut ro = Obj::new();
                    ro.str("text", &r.text);
                    for (k, v) in [("bold", r.bold), ("italic", r.italic), ("underline", r.underline), ("strike", r.strike)] {
                        if v {
                            ro.bool(k, true);
                        }
                    }
                    ro.opt("link", r.link.as_deref());
                    ro.end()
                })
                .collect();
            o.raw("runs", &json::array(&runs));
        }
    }
    o.end()
}

/// Serializa el documento. Con `full` se agregan estilo, alineacion, tramos con formato y,
/// en PDF, las lineas de cada pagina con su posicion.
pub fn to_json(d: &Document, full: bool) -> String {
    let mut meta = Obj::new();
    for (k, v) in &d.meta {
        match v {
            Meta::Text(s) => meta.str(k, s),
            Meta::Int(i) => meta.int(k, *i),
            Meta::Bool(b) => meta.bool(k, *b),
        };
    }

    let text = if d.pages.is_empty() { plain_text(&d.content) } else { d.pages.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n\n") };
    let count = |k: Kind| d.content.iter().filter(|b| b.kind == k).count() as i64;
    let mut stats = Obj::new();
    stats
        .int("characters", text.chars().count() as i64)
        .int("words", json::word_count(&text) as i64)
        .int("headings", count(Kind::Heading) + count(Kind::Title))
        .int("paragraphs", count(Kind::Paragraph))
        .int("list_items", count(Kind::ListItem))
        .int("tables", count(Kind::Table))
        .int("images", count(Kind::Image) + d.pages.iter().map(|p| p.images as i64).sum::<i64>());

    let mut root = Obj::new();
    root.str("type", d.kind).raw("metadata", &meta.end());
    match d.page_count {
        Some(n) => root.int("page_count", n as i64),
        None => root.raw("page_count", "null"),
    };
    root.raw("stats", &stats.end());
    let blocks: Vec<String> = d.content.iter().map(|b| block_json(b, full)).collect();
    root.raw("content", &json::array(&blocks));

    if !d.pages.is_empty() {
        let pages: Vec<String> = d
            .pages
            .iter()
            .map(|p| {
                let mut o = Obj::new();
                o.int("number", p.number as i64).num("width", p.width).num("height", p.height).str("text", &p.text);
                if p.images > 0 {
                    o.int("images", p.images as i64);
                }
                if full {
                    let lines: Vec<String> = p
                        .lines
                        .iter()
                        .map(|l| {
                            let mut lo = Obj::new();
                            lo.str("text", &l.text).num("x", l.x).num("y", l.y).num("size", l.size);
                            if l.bold {
                                lo.bool("bold", true);
                            }
                            lo.end()
                        })
                        .collect();
                    o.raw("lines", &json::array(&lines));
                }
                o.end()
            })
            .collect();
        root.raw("pages", &json::array(&pages));
    }
    if !d.outline.is_empty() {
        let items: Vec<String> = d
            .outline
            .iter()
            .map(|i| {
                let mut o = Obj::new();
                o.str("title", &i.title).int("level", i.level as i64);
                if let Some(p) = i.page {
                    o.int("page", p as i64);
                }
                o.end()
            })
            .collect();
        root.raw("outline", &json::array(&items));
    }
    for (key, blocks) in &d.extras {
        if !blocks.is_empty() {
            let items: Vec<String> = blocks.iter().map(|b| block_json(b, full)).collect();
            root.raw(key, &json::array(&items));
        }
    }
    if !d.warnings.is_empty() {
        let w: Vec<String> = d.warnings.iter().map(|s| json::string(s)).collect();
        root.raw("warnings", &json::array(&w));
    }
    root.str("text", &text);
    root.end()
}
