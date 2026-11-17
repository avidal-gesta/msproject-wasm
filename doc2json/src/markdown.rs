//! Markdown y fragmentos: el documento listo para un modelo de lenguaje o un buscador.
//!
//! - `to_markdown` arma Markdown (con tablas GFM) desde los bloques: titulos con `#`, listas con
//!   su numeracion o vineta, tablas con sus celdas combinadas, notas al pie como `[^1]`, y en Word
//!   negrita, cursiva y enlaces. En PDF marca el inicio de cada pagina con `<!-- pagina N -->`.
//! - `chunks` corta ese Markdown en fragmentos de un tamano maximo para indexar o enviar a un
//!   modelo: cada fragmento empieza en un titulo cuando se puede, nunca corta un bloque (salvo que
//!   el bloque solo ya sea mas grande que el fragmento) y trae la ruta de titulos de su seccion y
//!   las paginas que cubre. Las tablas largas se parten por filas repitiendo el encabezado.

use crate::doc::{Block, Cell, Document, Kind};
use crate::json::{self, Obj};
use std::collections::HashSet;

/// Tamano por defecto de un fragmento, en caracteres (unos 500 tokens en castellano).
pub const DEFAULT_CHUNK: usize = 2000;
/// Una seccion mas corta que esto no cierra su fragmento: se junta con la siguiente.
const MIN_CHUNK: usize = 200;

/// Escapa lo que Markdown leeria como marca al inicio de una linea (titulo, cita, lista).
fn escape_start(text: &str) -> String {
    let t = text.trim_start();
    let first = t.chars().next();
    let needs = match first {
        Some('#') | Some('>') => true,
        Some('-') | Some('+') | Some('*') => t.chars().nth(1).is_none_or(|c| c == ' '),
        Some(c) if c.is_ascii_digit() => {
            let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
            let rest = &t[digits.len()..];
            (rest.starts_with(". ") || rest.starts_with(") ")) && digits.len() <= 9
        }
        _ => false,
    };
    if needs {
        format!("\\{}", t)
    } else {
        t.to_string()
    }
}

/// Texto de un bloque con su formato de Word (negrita, cursiva, enlaces) si lo trae.
fn inline(b: &Block) -> String {
    let joined: String = b.runs.iter().map(|r| r.text.as_str()).collect();
    if b.runs.is_empty() || joined != b.text || !b.runs.iter().any(|r| r.bold || r.italic || r.link.is_some()) {
        return b.text.clone();
    }
    let mut out = String::new();
    for r in &b.runs {
        // Los espacios de los bordes van fuera de la marca: "** hola**" no es negrita en Markdown.
        let core = r.text.trim();
        if core.is_empty() {
            out.push_str(&r.text);
            continue;
        }
        let lead = &r.text[..r.text.len() - r.text.trim_start().len()];
        let tail = &r.text[r.text.trim_end().len()..];
        let mut s = core.to_string();
        if r.italic {
            s = format!("*{}*", s);
        }
        if r.bold {
            s = format!("**{}**", s);
        }
        if let Some(link) = &r.link {
            if link.starts_with("http") || link.starts_with("mailto:") {
                s = format!("[{}]({})", s, link.replace(' ', "%20").replace(')', "%29"));
            }
        }
        out.push_str(lead);
        out.push_str(&s);
        out.push_str(tail);
    }
    out
}

/// Citas "[n]" de notas que existen: pasan a "[^n]" (nota al pie de Markdown).
fn footnote_refs(text: &str, labels: &HashSet<String>) -> String {
    if labels.is_empty() || !text.contains('[') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 8);
    let mut rest = text;
    while let Some(i) = rest.find('[') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find(']') {
            Some(j) if labels.contains(&after[..j]) => {
                out.push_str("[^");
                out.push_str(&after[..j]);
                out.push(']');
                rest = &after[j + 1..];
            }
            _ => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn cell_text(c: &Cell) -> String {
    if c.merged {
        return String::new();
    }
    c.text.trim().replace('|', "\\|").replace("\r\n", "\n").split('\n').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("<br>")
}

/// Celdas de cada fila de una tabla, ya escapadas y todas con el mismo numero de columnas.
fn table_cells(rows: &[Vec<Cell>]) -> Vec<Vec<String>> {
    let width = rows.iter().map(|r| r.iter().map(|c| c.colspan.max(1) as usize).sum::<usize>()).max().unwrap_or(0).max(1);
    rows.iter()
        .enumerate()
        // Las filas sin ningun texto (formularios por llenar) no aportan; la primera se deja como encabezado.
        .filter(|(i, r)| *i == 0 || r.iter().any(|c| !c.merged && !c.text.trim().is_empty()))
        .map(|(_, r)| {
            let mut cells: Vec<String> = Vec::with_capacity(width);
            for c in r {
                cells.push(cell_text(c));
                // Una celda que ocupa varias columnas deja las siguientes vacias.
                for _ in 1..c.colspan.max(1) {
                    cells.push(String::new());
                }
            }
            cells.resize(width, String::new());
            cells
        })
        .collect::<Vec<_>>()
        .pipe_drop_empty_columns()
}

trait DropEmptyColumns {
    fn pipe_drop_empty_columns(self) -> Self;
}

impl DropEmptyColumns for Vec<Vec<String>> {
    /// Quita las columnas sin texto en ninguna fila (rejillas de formularios, celdas de relleno).
    fn pipe_drop_empty_columns(self) -> Self {
        let width = self.first().map_or(0, Vec::len);
        let keep: Vec<bool> = (0..width).map(|j| self.iter().any(|r| r.get(j).is_some_and(|c| !c.is_empty()))).collect();
        if keep.iter().all(|k| *k) || !keep.iter().any(|k| *k) {
            return self;
        }
        self.into_iter().map(|r| r.into_iter().zip(&keep).filter(|(_, k)| **k).map(|(c, _)| c).collect()).collect()
    }
}

fn row_line(cells: &[String]) -> String {
    format!("| {} |", cells.join(" | "))
}

/// Filas de una tabla como lineas GFM.
fn table_rows(rows: &[Vec<Cell>]) -> Vec<String> {
    table_cells(rows).iter().map(|c| row_line(c)).collect()
}

/// Una fila demasiado larga para un fragmento se reparte en filas de continuacion con las mismas
/// columnas: cada celda larga se corta (en una oracion o un espacio) y sus partes van una por fila.
fn split_row(cells: &[String], room: usize) -> Vec<String> {
    let line = row_line(cells);
    if line.chars().count() <= room {
        return vec![line];
    }
    let filled = cells.iter().filter(|c| !c.is_empty()).count().max(1);
    // Lo que queda para texto despues de los separadores "| ", repartido entre las celdas con texto.
    let budget = (room.saturating_sub(4 * cells.len() + 2) / filled).max(40);
    let pieces: Vec<Vec<String>> = cells.iter().map(|c| if c.is_empty() { Vec::new() } else { split_text(c, budget) }).collect();
    let n = pieces.iter().map(Vec::len).max().unwrap_or(1).max(1);
    (0..n).map(|i| row_line(&pieces.iter().map(|p| p.get(i).cloned().unwrap_or_default()).collect::<Vec<_>>())).collect()
}

fn separator(width: usize) -> String {
    format!("|{}", " --- |".repeat(width.max(1)))
}

fn table_md(rows: &[Vec<Cell>]) -> String {
    let lines = table_rows(rows);
    if lines.is_empty() {
        return String::new();
    }
    let width = lines[0].matches(" | ").count() + 1;
    let mut out = vec![lines[0].clone(), separator(width)];
    out.extend(lines[1..].iter().cloned());
    out.join("\n")
}

/// Titulo de seccion de un bloque: (nivel, texto). Ademas de los titulos, muchas plantillas
/// escriben los de seccion ("1. OBJETIVO") como una tabla de una sola celda con texto corto;
/// esas tablas cuentan como titulo, con el nivel que de su numeracion ("3.2" = 2).
fn section(b: &Block) -> Option<(u8, String)> {
    match b.kind {
        Kind::Title => Some((0, b.text.trim().to_string())),
        Kind::Heading => Some((b.level.max(1), b.text.trim().to_string())),
        Kind::Table => {
            let cells: Vec<&Cell> = b.rows.iter().flatten().filter(|c| !c.merged && !c.text.trim().is_empty()).collect();
            if b.rows.len() != 1 || cells.len() != 1 {
                return None;
            }
            let t = cells[0].text.trim();
            if t.contains('\n') || t.chars().count() > 80 || t.chars().filter(|c| c.is_alphabetic()).count() < 3 {
                return None;
            }
            let token = t.split_whitespace().next().unwrap_or("").trim_end_matches('.');
            let depth = if !token.is_empty() && token.split('.').all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|c| c.is_ascii_digit())) { token.split('.').count() } else { 1 };
            Some((depth.min(6) as u8, t.to_string()))
        }
        _ => None,
    }
}

struct Ctx {
    /// Suma a los niveles de titulo: 1 si el documento tiene un titulo principal (va con `#`).
    offset: u8,
    labels: HashSet<String>,
}

/// Markdown de un bloque (sin la marca de pagina). Vacio si el bloque no aporta texto.
fn block_md(b: &Block, cx: &Ctx) -> String {
    let text = || footnote_refs(&inline(b), &cx.labels);
    match b.kind {
        Kind::Title => format!("# {}", text().trim()),
        Kind::Heading => format!("{} {}", "#".repeat((b.level.max(1) + cx.offset).min(6) as usize), text().trim()),
        Kind::Paragraph => escape_start(&text()),
        Kind::ListItem => {
            let indent = "  ".repeat(b.level as usize);
            let marker = b.marker.as_deref().unwrap_or("").trim();
            let numbered = !marker.is_empty() && marker.trim_end_matches(['.', ')']).chars().all(|c| c.is_ascii_digit()) && marker.len() <= 10 && (marker.ends_with('.') || marker.ends_with(')'));
            let body = text();
            if numbered {
                format!("{}{} {}", indent, marker, body.trim())
            } else if b.ordered && !marker.is_empty() {
                // "a)", "iv.", "3.2.1": Markdown no los numera; se dejan escritos.
                format!("{}- {} {}", indent, marker, body.trim())
            } else {
                format!("{}- {}", indent, body.trim())
            }
        }
        Kind::Table => match section(b) {
            Some((level, t)) => format!("{} {}", "#".repeat((level.max(1) + cx.offset).min(6) as usize), t),
            None => table_md(&b.rows),
        },
        Kind::Image => {
            let alt = b.text.trim().replace(['[', ']'], "");
            match &b.name {
                Some(n) => format!("![{}]({})", alt, n.replace(' ', "%20")),
                None if !alt.is_empty() => format!("*[Imagen: {}]*", alt),
                None => String::new(),
            }
        }
        Kind::PageBreak => String::new(),
    }
}

fn context(d: &Document) -> Ctx {
    let labels: HashSet<String> = d
        .extras
        .iter()
        .filter(|(k, _)| *k == "footnotes" || *k == "endnotes")
        .flat_map(|(_, v)| v.iter().filter_map(|b| b.label.clone()))
        .collect();
    Ctx { offset: if d.content.iter().any(|b| b.kind == Kind::Title) { 1 } else { 0 }, labels }
}

/// Notas al pie y finales como definiciones de Markdown (`[^1]: texto`).
fn notes_md(d: &Document) -> String {
    let mut out: Vec<String> = Vec::new();
    for (k, blocks) in &d.extras {
        if *k != "footnotes" && *k != "endnotes" {
            continue;
        }
        for b in blocks {
            if let Some(l) = &b.label {
                out.push(format!("[^{}]: {}", l, b.text.trim()));
            }
        }
    }
    out.join("\n")
}

/// Separador entre dos bloques: los items de una misma lista van seguidos.
fn joiner(prev: Option<Kind>, next: Kind) -> &'static str {
    match (prev, next) {
        (None, _) => "",
        (Some(Kind::ListItem), Kind::ListItem) => "\n",
        _ => "\n\n",
    }
}

/// El documento completo en Markdown.
pub fn to_markdown(d: &Document) -> String {
    let cx = context(d);
    let mut out = String::new();
    let mut prev: Option<Kind> = None;
    let mut page: Option<u32> = None;
    let pdf = d.kind == "pdf";
    for b in &d.content {
        let md = block_md(b, &cx);
        if md.trim().is_empty() {
            continue;
        }
        if pdf && b.page.is_some() && b.page != page {
            page = b.page;
            out.push_str(if out.is_empty() { "" } else { "\n\n" });
            out.push_str(&format!("<!-- pagina {} -->", page.unwrap_or(0)));
            prev = Some(Kind::PageBreak);
        }
        out.push_str(joiner(prev, b.kind));
        out.push_str(&md);
        prev = Some(b.kind);
    }
    let notes = notes_md(d);
    if !notes.is_empty() {
        out.push_str("\n\n");
        out.push_str(&notes);
    }
    out.push('\n');
    out
}

/// Fragmento del documento.
#[derive(Debug, Clone, Default)]
pub struct Chunk {
    /// Titulos de las secciones que lo contienen, del mas general al mas especifico.
    pub path: Vec<String>,
    pub text: String,
    /// Paginas que cubre (solo PDF).
    pub pages: Option<(u32, u32)>,
}

struct Builder {
    max: usize,
    out: Vec<Chunk>,
    cur: Chunk,
    prev: Option<Kind>,
    /// El fragmento en curso solo tiene titulos: acompanan al bloque que sigue aunque sea grande.
    only_headings: bool,
}

impl Builder {
    fn add_page(&mut self, p: Option<u32>) {
        if let Some(p) = p {
            self.cur.pages = Some(match self.cur.pages {
                Some((a, b)) => (a.min(p), b.max(p)),
                None => (p, p),
            });
        }
    }

    fn emit(&mut self) {
        if !self.cur.text.trim().is_empty() {
            let mut c = std::mem::take(&mut self.cur);
            c.text = c.text.trim().to_string();
            self.out.push(c);
        }
        self.cur = Chunk::default();
        self.prev = None;
        self.only_headings = false;
    }

    fn push(&mut self, md: &str, kind: Kind, page: Option<u32>, path: &[String]) {
        if self.cur.text.is_empty() {
            self.cur.path = path.to_vec();
            self.only_headings = kind == Kind::Heading;
        } else if kind != Kind::Heading {
            self.only_headings = false;
        }
        self.cur.text.push_str(joiner(self.prev, kind));
        self.cur.text.push_str(md);
        self.prev = Some(kind);
        self.add_page(page);
    }
}

/// Parte un texto largo en trozos de hasta `max` caracteres, cortando de preferencia al final de
/// una oracion y si no en un espacio.
fn split_text(text: &str, max: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut rest = text.trim();
    while rest.chars().count() > max {
        let limit = rest.char_indices().nth(max).map_or(rest.len(), |(i, _)| i);
        let window = &rest[..limit];
        let cut = window.rfind(". ").map(|i| i + 1).filter(|i| *i > limit / 3).or_else(|| window.rfind(' ').filter(|i| *i > limit / 3)).unwrap_or(limit);
        parts.push(rest[..cut].trim().to_string());
        rest = rest[cut..].trim_start();
    }
    if !rest.is_empty() {
        parts.push(rest.to_string());
    }
    parts
}

/// Corta el documento en fragmentos de hasta `max_chars` caracteres (minimo 200).
pub fn chunks(d: &Document, max_chars: usize) -> Vec<Chunk> {
    let max = max_chars.max(MIN_CHUNK);
    let cx = context(d);
    let mut b = Builder { max, out: Vec::new(), cur: Chunk::default(), prev: None, only_headings: false };
    let mut path: Vec<(u8, String)> = Vec::new();
    let names = |p: &[(u8, String)]| p.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>();
    for blk in &d.content {
        let md = block_md(blk, &cx);
        if md.trim().is_empty() {
            continue;
        }
        let len = md.chars().count();
        if let Some((level, title)) = section(blk) {
            // Un titulo abre fragmento nuevo, salvo que el actual sea muy corto (se juntan).
            if b.cur.text.chars().count() >= MIN_CHUNK {
                b.emit();
            }
            while path.last().is_some_and(|(l, _)| *l >= level) {
                path.pop();
            }
            path.push((level, title));
            b.push(&md, Kind::Heading, blk.page, &names(&path));
            continue;
        }
        let here = names(&path);
        let used = b.cur.text.chars().count();
        if used > 0 && used + len + 2 > b.max && !(b.only_headings && len > b.max) {
            b.emit();
        }
        if len <= b.max {
            b.push(&md, blk.kind, blk.page, &here);
            continue;
        }
        // Bloque mas grande que un fragmento: va en partes. Los titulos que lo preceden (si el
        // fragmento en curso solo tiene titulos) quedan con la primera parte.
        if !b.only_headings {
            b.emit();
        }
        // Espacio que dejan los titulos que ya estan en el fragmento (solo para la primera parte).
        let room = |b: &Builder| b.max.saturating_sub(b.cur.text.chars().count() + 2).max(MIN_CHUNK / 2);
        if blk.kind == Kind::Table && blk.rows.len() > 1 {
            let cells = table_cells(&blk.rows);
            let width = cells[0].len();
            // Una primera fila muy larga no es un encabezado sino contenido: se usa un encabezado
            // vacio (valido en GFM) y esa fila se reparte como las demas.
            let real_head = row_line(&cells[0]).chars().count() <= b.max / 3;
            let head_cells = if real_head { cells[0].clone() } else { vec![String::new(); width] };
            let head = format!("{}\n{}", row_line(&head_cells), separator(width));
            let head_len = head.chars().count() + 1;
            let body = if real_head { &cells[1..] } else { &cells[..] };
            // Filas que caben en un fragmento junto al encabezado (las mas largas, repartidas).
            let lines: Vec<String> = body.iter().flat_map(|r| split_row(r, b.max.saturating_sub(head_len))).collect();
            let mut part = head.clone();
            let mut limit = room(&b);
            for row in &lines {
                let fits = part.chars().count() + row.chars().count() < limit;
                if !fits && part == head && limit < b.max {
                    // Ni la primera fila cabe junto a los titulos: los titulos van solos.
                    b.emit();
                    limit = b.max;
                }
                if part.chars().count() + row.chars().count() + 1 > limit && part != head {
                    b.push(&part, Kind::Table, blk.page, &here);
                    b.emit();
                    part = head.clone();
                    limit = b.max;
                }
                part.push('\n');
                part.push_str(row);
            }
            b.push(&part, Kind::Table, blk.page, &here);
            b.emit();
        } else {
            let first = room(&b);
            let pieces = split_text(&md, first);
            let (head, rest) = pieces.split_first().map_or((None, &[][..]), |(h, r)| (Some(h), r));
            if let Some(h) = head {
                b.push(h, Kind::Paragraph, blk.page, &here);
                b.emit();
            }
            // El resto se vuelve a partir con el tamano completo.
            let rest = rest.join(" ");
            for piece in split_text(&rest, b.max) {
                b.push(&piece, Kind::Paragraph, blk.page, &here);
                b.emit();
            }
        }
    }
    // Las notas al pie van en un fragmento propio al final.
    let notes = notes_md(d);
    b.emit();
    if !notes.is_empty() {
        for piece in split_text(&notes, b.max) {
            b.push(&piece, Kind::Paragraph, None, &["Notas".to_string()]);
            b.emit();
        }
    }
    b.out
}

/// Fragmentos en JSON: `{ type, metadata, page_count, chunks: [{ index, path, pages, chars, text }] }`.
pub fn chunks_json(d: &Document, list: &[Chunk]) -> String {
    let mut meta = Obj::new();
    for (k, v) in &d.meta {
        match v {
            crate::doc::Meta::Text(s) => meta.str(k, s),
            crate::doc::Meta::Int(i) => meta.int(k, *i),
            crate::doc::Meta::Bool(x) => meta.bool(k, *x),
        };
    }
    let items: Vec<String> = list
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut o = Obj::new();
            o.int("index", i as i64);
            let path: Vec<String> = c.path.iter().map(|p| json::string(p)).collect();
            o.raw("path", &json::array(&path));
            if let Some((a, z)) = c.pages {
                o.raw("pages", &json::array(&[a.to_string(), z.to_string()]));
            }
            o.int("chars", c.text.chars().count() as i64);
            o.str("text", &c.text);
            o.end()
        })
        .collect();
    let mut root = Obj::new();
    root.str("type", d.kind).raw("metadata", &meta.end());
    match d.page_count {
        Some(n) => root.int("page_count", n as i64),
        None => root.raw("page_count", "null"),
    };
    root.raw("chunks", &json::array(&items));
    root.end()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> Block {
        Block { kind: Kind::Paragraph, text: text.into(), ..Block::default() }
    }

    fn h(level: u8, text: &str) -> Block {
        Block { kind: Kind::Heading, level, text: text.into(), ..Block::default() }
    }

    fn celda(t: &str) -> Cell {
        Cell { text: t.into(), colspan: 1, rowspan: 1, merged: false }
    }

    fn tabla(rows: Vec<Vec<Cell>>) -> Block {
        Block { kind: Kind::Table, rows, ..Block::default() }
    }

    fn doc(content: Vec<Block>) -> Document {
        Document { kind: "docx", content, ..Document::default() }
    }

    #[test]
    fn escapa_lo_que_markdown_leeria_como_marca() {
        assert_eq!(escape_start("# no es titulo"), "\\# no es titulo");
        assert_eq!(escape_start("1. no es lista"), "\\1. no es lista");
        assert_eq!(escape_start("- tampoco"), "\\- tampoco");
        assert_eq!(escape_start("2024 fue un buen anio"), "2024 fue un buen anio");
        assert_eq!(escape_start("-5 grados"), "-5 grados");
    }

    #[test]
    fn tablas_gfm_con_combinadas_y_sin_filas_vacias() {
        let mut ancha = celda("Riesgos");
        ancha.colspan = 3;
        let t = tabla(vec![vec![ancha], vec![celda("A|B"), celda("x\ny"), celda("z")], vec![celda(""), celda(" "), celda("")], vec![Cell { merged: true, colspan: 1, rowspan: 1, ..Cell::default() }, celda("c"), celda("d")]]);
        let md = to_markdown(&doc(vec![t]));
        assert_eq!(md, "| Riesgos |  |  |\n| --- | --- | --- |\n| A\\|B | x<br>y | z |\n|  | c | d |\n");
    }

    #[test]
    fn titulos_listas_y_notas() {
        let mut d = doc(vec![
            Block { kind: Kind::Title, text: "Manual".into(), ..Block::default() },
            h(1, "Pasos"),
            p("Antes de partir.[1]"),
            Block { kind: Kind::ListItem, ordered: true, marker: Some("1.".into()), text: "Detener".into(), ..Block::default() },
            Block { kind: Kind::ListItem, level: 1, marker: Some("•".into()), text: "Avisar".into(), ..Block::default() },
            Block { kind: Kind::ListItem, ordered: true, marker: Some("a)".into()), text: "Bloquear".into(), ..Block::default() },
            p("[2] no es una nota"),
        ]);
        d.extras.push(("footnotes", vec![Block { label: Some("1".into()), text: "Solo electricos.".into(), ..Block::default() }]));
        assert_eq!(to_markdown(&d), "# Manual\n\n## Pasos\n\nAntes de partir.[^1]\n\n1. Detener\n  - Avisar\n- a) Bloquear\n\n[2] no es una nota\n\n[^1]: Solo electricos.\n");
    }

    #[test]
    fn tabla_de_una_celda_es_titulo_de_seccion() {
        let d = doc(vec![tabla(vec![vec![celda("3.2 RESPONSABLES")]]), p("Texto")]);
        assert_eq!(to_markdown(&d), "## 3.2 RESPONSABLES\n\nTexto\n");
        let c = chunks(&d, 500);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].path, ["3.2 RESPONSABLES"]);
    }

    #[test]
    fn fragmentos_por_seccion_sin_pasar_el_maximo() {
        let largo = "Oracion de prueba con varias palabras. ".repeat(30); // ~1170 caracteres
        let mut content = vec![h(1, "Objetivo"), p(&largo), h(1, "Desarrollo"), h(2, "Antes"), p(&largo), p(&largo)];
        // Tabla de 40 filas: mas grande que un fragmento.
        let mut rows = vec![vec![celda("Paso"), celda("Detalle")]];
        for i in 0..40 {
            rows.push(vec![celda(&i.to_string()), celda("Revisar el equipo y registrar la medicion")]);
        }
        content.push(h(2, "Tabla"));
        content.push(tabla(rows));
        let c = chunks(&doc(content), 1000);
        assert!(c.iter().all(|x| x.text.chars().count() <= 1000), "{:?}", c.iter().map(|x| x.text.chars().count()).collect::<Vec<_>>());
        assert_eq!(c[0].path, ["Objetivo"]);
        assert!(c[0].text.starts_with("## Objetivo") || c[0].text.starts_with("# Objetivo"));
        assert!(c.iter().any(|x| x.path == ["Desarrollo", "Antes"]));
        // La tabla se parte por filas y cada parte repite el encabezado; el titulo va con la primera.
        let partes: Vec<&Chunk> = c.iter().filter(|x| x.path == ["Desarrollo", "Tabla"]).collect();
        assert!(partes.len() >= 2);
        assert!(partes[0].text.starts_with("## Tabla\n\n| Paso | Detalle |"));
        assert!(partes.iter().all(|x| x.text.contains("| Paso | Detalle |\n| --- | --- |")));
        // No se pierde ninguna fila.
        let filas: usize = partes.iter().map(|x| x.text.matches("Revisar el equipo").count()).sum();
        assert_eq!(filas, 40);
    }

    #[test]
    fn fragmentos_en_json() {
        let d = doc(vec![h(1, "Alcance"), p("Aplica a \"todos\".")]);
        let j = chunks_json(&d, &chunks(&d, 2000));
        assert!(j.contains("\"chunks\":[{\"index\":0,\"path\":[\"Alcance\"],\"chars\":"));
        assert!(j.contains("Aplica a \\\"todos\\\"."));
    }
}

#[cfg(test)]
mod filas_largas {
    use super::*;

    #[test]
    fn una_fila_mas_grande_que_el_fragmento_se_reparte() {
        let larga = "Verificar el bloqueo antes de intervenir el equipo. ".repeat(60); // ~3100 caracteres
        let fila = vec![Cell { text: "1".into(), colspan: 1, rowspan: 1, merged: false }, Cell { text: larga.clone(), colspan: 1, rowspan: 1, merged: false }];
        let cab = vec![Cell { text: "Paso".into(), colspan: 1, rowspan: 1, merged: false }, Cell { text: "Riesgos y controles".into(), colspan: 1, rowspan: 1, merged: false }];
        let d = Document { kind: "docx", content: vec![Block { kind: Kind::Heading, level: 1, text: "Ejecucion".into(), ..Block::default() }, Block { kind: Kind::Table, rows: vec![cab, fila], ..Block::default() }], ..Document::default() };
        let c = chunks(&d, 1000);
        assert!(c.iter().all(|x| x.text.chars().count() <= 1000), "{:?}", c.iter().map(|x| x.text.chars().count()).collect::<Vec<_>>());
        assert!(c.len() >= 4);
        assert!(c.iter().all(|x| x.text.contains("| Paso | Riesgos y controles |\n| --- | --- |")));
        // No se pierde texto: todas las repeticiones de la oracion siguen estando.
        let total: usize = c.iter().map(|x| x.text.matches("Verificar el bloqueo").count()).sum();
        assert_eq!(total, 60);
    }
}
