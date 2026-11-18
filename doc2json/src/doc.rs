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

/// Columnas que puede ocupar una celda (`colspan`). Word no pasa de 63 columnas; el valor del
/// archivo se acota para que una celda que dice ocupar millones no se materialice.
pub const MAX_COLSPAN: u32 = 64;

#[derive(Clone, Default, Debug)]
pub struct Cell {
    pub text: String,
    pub colspan: u32,
    pub rowspan: u32,
    /// Celda cubierta por una combinacion vertical iniciada mas arriba.
    pub merged: bool,
    /// Imagenes de la celda (su `id` en la lista de imagenes), en orden.
    pub images: Vec<u32>,
    /// Solo Word: los bloques de la celda (parrafos con su estilo, tablas anidadas) cuando alguno es
    /// un titulo o un encabezado. Salen en el JSON completo.
    pub blocks: Vec<Block>,
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
    /// Imagen: su `id` en la lista de imagenes del documento.
    pub image: Option<u32>,
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

/// Un lugar donde aparece una imagen, en puntos (1/72 de pulgada).
#[derive(Clone, Default, Debug)]
pub struct Place {
    /// PDF: pagina.
    pub page: Option<u32>,
    /// PDF: esquina superior izquierda, medida desde la esquina superior izquierda de la pagina
    /// en su orientacion de lectura (igual que `y` de las lineas).
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// Ancho y alto con que se muestra.
    pub w: f64,
    pub h: f64,
}

/// Una imagen del documento: una por archivo o por objeto, aunque se muestre varias veces.
#[derive(Clone, Default, Debug)]
pub struct Image {
    /// Desde 1, en orden de aparicion. Los bloques `image` la citan con este numero.
    pub id: u32,
    /// "png", "jpeg", "gif", "bmp", "tiff", "emf", "wmf", "svg", "jp2", "jbig2", "ccitt"...
    pub format: &'static str,
    /// Pixeles. En PDF siempre; en Word, solo al extraer (hay que leer el archivo).
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Nombre del archivo dentro del documento (Word) o del recurso (PDF).
    pub name: Option<String>,
    /// Texto alternativo (el primero que no este vacio).
    pub alt: String,
    pub places: Vec<Place>,
    /// El archivo de la imagen, solo cuando se pide extraerlas.
    pub data: Option<Vec<u8>>,
    /// Por que no se entrega el archivo (formato que no se convierte, imagen demasiado grande...).
    pub note: Option<String>,
}

impl Image {
    pub fn mime(&self) -> &'static str {
        match self.format {
            "png" => "image/png",
            "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "bmp" => "image/bmp",
            "tiff" => "image/tiff",
            "svg" => "image/svg+xml",
            "webp" => "image/webp",
            "emf" => "image/emf",
            "wmf" => "image/wmf",
            "jp2" => "image/jp2",
            _ => "application/octet-stream",
        }
    }
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
    /// Imagenes, sin repetir: Word las cita desde bloques `image`; en PDF se ubican por pagina.
    pub images: Vec<Image>,
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
                            if !c.images.is_empty() {
                                co.raw("images", &json::array(&c.images.iter().map(|i| i.to_string()).collect::<Vec<_>>()));
                            }
                            if c.merged {
                                co.bool("merged", true);
                            }
                            if full && !c.blocks.is_empty() {
                                co.raw("blocks", &json::array(&c.blocks.iter().map(|b| block_json(b, full)).collect::<Vec<_>>()));
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
            if let Some(id) = b.image {
                o.int("image", id as i64);
            }
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
/// Datos de una imagen (sin sus bytes). `offset`/`length`: ubicacion de los bytes cuando se
/// entregan aparte (WebAssembly).
pub fn image_json_with(img: &Image, at: Option<(usize, usize)>) -> String {
    let mut o = Obj::new();
    o.int("id", img.id as i64).str("format", img.format);
    if let (Some(w), Some(h)) = (img.width, img.height) {
        o.int("width", w as i64).int("height", h as i64);
    }
    o.opt("name", img.name.as_deref());
    if !img.alt.is_empty() {
        o.str("alt", &img.alt);
    }
    if !img.places.is_empty() {
        let places: Vec<String> = img
            .places
            .iter()
            .map(|p| {
                let mut po = Obj::new();
                if let Some(n) = p.page {
                    po.int("page", n as i64);
                }
                if let (Some(x), Some(y)) = (p.x, p.y) {
                    po.num("x", x).num("y", y);
                }
                po.num("w", p.w).num("h", p.h);
                po.end()
            })
            .collect();
        o.raw("places", &json::array(&places));
    }
    if let Some(d) = &img.data {
        o.str("mime", img.mime()).int("size", d.len() as i64);
    }
    if let Some((offset, length)) = at {
        o.int("offset", offset as i64).int("length", length as i64);
    }
    o.opt("note", img.note.as_deref());
    o.end()
}

pub fn to_json(d: &Document, full: bool) -> String {
    to_json_at(d, full, &|_| None)
}

/// JSON y, a continuacion, los bytes de las imagenes extraidas: cada imagen del JSON lleva
/// `offset` y `length` dentro de ese bloque. Se omiten los bytes de las imagenes cuyo lado menor
/// tiene menos de `min_side` pixeles (siguen en la lista, sin `offset`).
pub fn to_json_with_images(d: &Document, full: bool, min_side: u32) -> (String, Vec<u8>) {
    let mut blob = Vec::new();
    let mut at: Vec<Option<(usize, usize)>> = Vec::with_capacity(d.images.len());
    for img in &d.images {
        let small = matches!((img.width, img.height), (Some(w), Some(h)) if w.min(h) < min_side);
        match &img.data {
            Some(bytes) if !small => {
                at.push(Some((blob.len(), bytes.len())));
                blob.extend_from_slice(bytes);
            }
            _ => at.push(None),
        }
    }
    let json = to_json_at(d, full, &|img: &Image| at.get(img.id as usize - 1).copied().flatten());
    (json, blob)
}

fn to_json_at(d: &Document, full: bool, at: &dyn Fn(&Image) -> Option<(usize, usize)>) -> String {
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
        // En PDF las imagenes son las que dibujan las paginas: un bloque de imagen (figura de un PDF
        // etiquetado) es una de ellas y no se suma otra vez.
        .int("images", if d.pages.is_empty() { count(Kind::Image) } else { d.pages.iter().map(|p| p.images as i64).sum::<i64>() });

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
    if !d.images.is_empty() {
        let items: Vec<String> = d.images.iter().map(|i| image_json_with(i, at(i))).collect();
        root.raw("images", &json::array(&items));
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
