//! PDF etiquetado (Tagged PDF, ISO 32000-1, seccion 14.8): lee el arbol de estructura del
//! documento y arma los bloques desde sus etiquetas (titulos, parrafos, listas, tablas y
//! figuras) en vez de deducirlos por la posicion del texto.
//!
//! Word, LibreOffice y la mayoria de los generadores de oficina etiquetan sus PDF. Con las
//! etiquetas una tabla sale con sus filas y celdas reales (incluidas las combinadas), una lista
//! con su vineta o numeracion, y los encabezados y pies de pagina quedan fuera porque vienen
//! marcados como artefactos.
//!
//! El texto de cada elemento sale del contenido marcado de las paginas (`BDC /MCID n`). El que no
//! pertenece a ningun elemento lo agrega `mod.rs` con el metodo por posicion, para no perder nada.

use super::content::Item;
use super::document::{text_string, Pdf};
use super::layout;
use super::object::{Dict, Obj};
use crate::doc::{Block, Cell, Kind};
use std::collections::{HashMap, HashSet};

/// Limites para arboles patologicos o maliciosos.
const MAX_NODES: usize = 500_000;
const MAX_DEPTH: usize = 64;

/// Hijo de un elemento de estructura.
#[derive(Debug)]
pub enum Kid {
    /// Contenido marcado de una pagina (indice de pagina desde 0, MCID).
    Content(usize, i64),
    Elem(Node),
}

/// Elemento de estructura, con su tipo ya traducido a uno estandar.
#[derive(Debug, Default)]
pub struct Node {
    pub role: String,
    pub kids: Vec<Kid>,
    /// Texto alternativo (figuras) y texto real que reemplaza al contenido (/ActualText).
    pub alt: Option<String>,
    pub actual: Option<String>,
    pub rowspan: u32,
    pub colspan: u32,
}

/// Texto de una pagina agrupado por contenido marcado.
pub struct PageStore {
    pub bbox: [f64; 4],
    pub rotate: i64,
    pub by_mcid: HashMap<i64, Vec<Item>>,
}

const STANDARD: &[&str] = &[
    "Document", "Part", "Art", "Sect", "Div", "BlockQuote", "Caption", "TOC", "TOCI", "Index", "NonStruct", "Private", "P", "H", "H1", "H2", "H3", "H4", "H5",
    "H6", "L", "LI", "Lbl", "LBody", "Table", "TR", "TH", "TD", "THead", "TBody", "TFoot", "Span", "Quote", "Note", "Reference", "BibEntry", "Code", "Link", "Annot",
    "Ruby", "Warichu", "Figure", "Formula", "Form", "Title", "FENote", "Sub", "Em", "Strong", "Aside", "DocumentFragment", "Artifact",
];

/// Tipos que forman un bloque propio (no van en linea con el texto que los rodea).
fn is_block(role: &str) -> bool {
    matches!(
        role,
        "Document" | "DocumentFragment" | "Part" | "Art" | "Sect" | "Div" | "Aside" | "P" | "H" | "H1" | "H2" | "H3" | "H4" | "H5" | "H6" | "Title" | "L" | "LI" | "Table" | "TR"
            | "Figure" | "TOC" | "TOCI" | "BlockQuote" | "Caption" | "Index" | "NonStruct"
    )
}

/// Nota al pie o final. No corta el parrafo que la cita: va aparte, como en el lector de Word.
fn is_note(role: &str) -> bool {
    matches!(role, "Note" | "FENote")
}

/// El elemento (en linea) contiene una nota: es la referencia que la cita ("1" en superindice).
fn has_note(node: &Node) -> bool {
    node.kids.iter().any(|k| matches!(k, Kid::Elem(e) if is_note(&e.role) || (!is_block(&e.role) && has_note(e))))
}

struct Reader<'p, 'a> {
    pdf: &'p Pdf<'a>,
    rolemap: Option<Dict>,
    page_index: &'p HashMap<u32, usize>,
    visited: HashSet<u32>,
    nodes: usize,
}

impl<'p, 'a> Reader<'p, 'a> {
    /// Traduce un tipo propio del documento a uno estandar con /RoleMap (puede encadenarse).
    fn role(&self, name: &str) -> String {
        let mut cur = name.to_string();
        for _ in 0..8 {
            if STANDARD.contains(&cur.as_str()) {
                break;
            }
            match self.rolemap.as_ref().and_then(|m| m.get(&cur)).and_then(|o| o.as_name()) {
                Some(next) if next != cur => cur = next.to_string(),
                _ => break,
            }
        }
        cur
    }

    fn page_of(&self, d: &Dict) -> Option<usize> {
        match d.get("Pg") {
            Some(Obj::Ref(n, _)) => self.page_index.get(n).copied(),
            _ => None,
        }
    }

    /// Lee los hijos (/K) de un elemento: un MCID, una referencia a otro elemento, un
    /// diccionario de contenido marcado (MCR) o un arreglo con cualquiera de ellos.
    fn kids(&mut self, k: &Obj, page: Option<usize>, depth: usize, out: &mut Vec<Kid>) {
        if self.nodes >= MAX_NODES || depth > MAX_DEPTH {
            return;
        }
        match k {
            Obj::Int(mcid) => {
                if let Some(p) = page {
                    out.push(Kid::Content(p, *mcid));
                }
            }
            Obj::Array(items) => {
                for it in items {
                    self.kids(it, page, depth, out);
                }
            }
            Obj::Ref(n, _) => {
                if !self.visited.insert(*n) {
                    return; // ciclo o elemento ya leido
                }
                let obj = self.pdf.resolve(k);
                match &*obj {
                    Obj::Dict(d) => self.dict(d, page, depth, out),
                    Obj::Array(_) | Obj::Int(_) => self.kids(&obj, page, depth, out),
                    _ => {}
                }
            }
            Obj::Dict(d) => self.dict(d, page, depth, out),
            _ => {}
        }
    }

    fn dict(&mut self, d: &Dict, page: Option<usize>, depth: usize, out: &mut Vec<Kid>) {
        match d.get("Type").and_then(|t| t.as_name()) {
            Some("MCR") => {
                // Contenido marcado de una pagina (o de un formulario: /Stm, no soportado).
                if d.get("Stm").is_some() {
                    return;
                }
                if let (Some(p), Some(m)) = (self.page_of(d).or(page), self.pdf.get(d, "MCID").as_i64()) {
                    out.push(Kid::Content(p, m));
                }
            }
            Some("OBJR") => {} // anotaciones y formularios: su texto lo aporta el contenido de la pagina
            _ => {
                if let Some(node) = self.elem(d, page, depth + 1) {
                    out.push(Kid::Elem(node));
                }
            }
        }
    }

    fn elem(&mut self, d: &Dict, page: Option<usize>, depth: usize) -> Option<Node> {
        self.nodes += 1;
        let role = self.role(&self.pdf.name(d, "S")?);
        if role == "Artifact" {
            return None;
        }
        let page = self.page_of(d).or(page);
        let text = |key: &str| self.pdf.get(d, key).as_bytes().map(text_string).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let (alt, actual) = (text("Alt"), text("ActualText"));
        // Atributos de celda: /A puede ser un diccionario o un arreglo (con numeros de revision).
        let (mut rowspan, mut colspan) = (1u32, 1u32);
        let attrs = self.pdf.get(d, "A");
        let mut look = |o: &Obj| {
            if let Some(a) = self.pdf.resolve(o).as_dict() {
                if let Some(r) = self.pdf.get(a, "RowSpan").as_i64() {
                    rowspan = r.clamp(1, 1000) as u32;
                }
                if let Some(c) = self.pdf.get(a, "ColSpan").as_i64() {
                    colspan = c.clamp(1, 1000) as u32;
                }
            }
        };
        match &*attrs {
            Obj::Array(list) => list.iter().for_each(&mut look),
            Obj::Dict(_) => look(&attrs),
            _ => {}
        }
        let mut node = Node { role, alt, actual, rowspan, colspan, kids: Vec::new() };
        if let Some(k) = d.get("K") {
            let k = k.clone();
            self.kids(&k, page, depth, &mut node.kids);
        }
        Some(node)
    }
}

/// Arbol de estructura del documento, o `None` si el PDF no esta etiquetado.
pub fn tree(pdf: &Pdf, page_index: &HashMap<u32, usize>) -> Option<Node> {
    let root = pdf.dict(&pdf.catalog(), "StructTreeRoot")?;
    let mut r = Reader { pdf, rolemap: pdf.dict(&root, "RoleMap"), page_index, visited: HashSet::new(), nodes: 0 };
    let mut node = Node { role: "Document".into(), ..Node::default() };
    let k = root.get("K")?.clone();
    r.kids(&k, None, 0, &mut node.kids);
    if node.kids.is_empty() {
        None
    } else {
        Some(node)
    }
}

/// Arma los bloques recorriendo el arbol. Anota en `used` el contenido que tomo.
pub struct Builder<'s> {
    pub pages: &'s [PageStore],
    pub used: HashSet<(usize, i64)>,
    /// Notas al pie, con su numero en `label`; el texto que las cita lleva "[n]".
    pub notes: Vec<Block>,
}

fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl<'s> Builder<'s> {
    /// Texto de un grupo de contenido marcado, ordenado como lineas de la pagina.
    fn flush(&mut self, pending: &mut Vec<(usize, i64)>, page: &mut Option<u32>) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut i = 0;
        while i < pending.len() {
            let p = pending[i].0;
            let mut items: Vec<Item> = Vec::new();
            while i < pending.len() && pending[i].0 == p {
                let key = pending[i];
                if self.used.insert(key) {
                    if let Some(its) = self.pages.get(p).and_then(|s| s.by_mcid.get(&key.1)) {
                        items.extend(its.iter().cloned());
                    }
                }
                i += 1;
            }
            if items.is_empty() {
                continue;
            }
            page.get_or_insert(p as u32 + 1);
            let store = &self.pages[p];
            let (lines, _, _) = layout::lines(items, store.bbox, store.rotate);
            let mut text = String::new();
            for l in lines {
                let t = l.text.replace('\t', " ");
                // Palabra cortada con guion al final de una linea: se vuelve a unir.
                let cut = text.ends_with('-') && text.chars().rev().nth(1).map_or(false, |c| c.is_alphabetic()) && t.chars().next().map_or(false, |c| c.is_lowercase());
                if cut {
                    text.pop();
                } else if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(&t);
            }
            parts.push(clean(&text));
        }
        pending.clear();
        parts.join(" ")
    }

    /// Marca como usado todo el contenido bajo un elemento (cuando su texto viene en /ActualText).
    fn consume(&mut self, node: &Node) {
        for k in &node.kids {
            match k {
                Kid::Content(p, m) => {
                    self.used.insert((*p, *m));
                }
                Kid::Elem(e) => self.consume(e),
            }
        }
    }

    /// Pagina del primer contenido con texto bajo un elemento.
    fn first_page(&self, node: &Node) -> Option<u32> {
        for k in &node.kids {
            match k {
                Kid::Content(p, m) if self.pages.get(*p).map_or(false, |s| s.by_mcid.contains_key(m)) => return Some(*p as u32 + 1),
                Kid::Elem(e) => {
                    if let Some(p) = self.first_page(e) {
                        return Some(p);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Texto de un elemento: lo en linea se une con espacios; cada sub-bloque va en su propia linea.
    pub fn text(&mut self, node: &Node, page: &mut Option<u32>) -> String {
        if let Some(a) = &node.actual {
            if page.is_none() {
                *page = self.first_page(node);
            }
            self.consume(node);
            return clean(a);
        }
        let mut parts: Vec<String> = Vec::new();
        let mut pending: Vec<(usize, i64)> = Vec::new();
        self.text_into(node, page, &mut parts, &mut pending);
        let t = self.flush(&mut pending, page);
        if !t.is_empty() {
            parts.push(t);
        }
        parts.retain(|p| !p.is_empty());
        parts.join("\n")
    }

    /// Guarda una nota al pie. Su texto empieza con el mismo numero que la cita: se separa como rotulo.
    fn note(&mut self, node: &Node, label: Option<&str>) {
        let mut page = None;
        let text = self.text(node, &mut page).replace('\n', " ");
        let (mut label, mut body) = (label.map(String::from), text.clone());
        if let Some((first, rest)) = text.split_once(' ') {
            let looks = first.chars().count() <= 4 && first.chars().all(|c| c.is_alphanumeric() || c == '*');
            if label.as_deref() == Some(first) || (label.is_none() && looks) {
                label = Some(first.to_string());
                body = rest.trim().to_string();
            }
        }
        if !body.is_empty() {
            self.notes.push(Block { kind: Kind::Paragraph, label, text: body, page, ..Block::default() });
        }
    }

    /// Referencia a una nota (p. ej. Link > [Span "1", Note]): deja "[1]" en el texto y guarda la nota.
    fn note_ref(&mut self, node: &Node, page: &mut Option<u32>, parts: &mut Vec<String>, pending: &mut Vec<(usize, i64)>) {
        let before = self.flush(pending, page);
        if !before.is_empty() {
            parts.push(before);
        }
        let (mut lparts, mut lpend): (Vec<String>, Vec<(usize, i64)>) = (Vec::new(), Vec::new());
        let mut notes: Vec<&Node> = Vec::new();
        for k in &node.kids {
            match k {
                Kid::Elem(e) if is_note(&e.role) => notes.push(e),
                Kid::Content(p, m) => lpend.push((*p, *m)),
                Kid::Elem(e) => self.text_into(e, page, &mut lparts, &mut lpend),
            }
        }
        let t = self.flush(&mut lpend, page);
        lparts.push(t);
        let label = clean(&lparts.join(" "));
        if !label.is_empty() {
            match parts.last_mut() {
                Some(last) => last.push_str(&format!("[{}]", label)),
                None => parts.push(format!("[{}]", label)),
            }
        }
        for n in notes {
            self.note(n, if label.is_empty() { None } else { Some(&label) });
        }
    }

    fn text_into(&mut self, node: &Node, page: &mut Option<u32>, parts: &mut Vec<String>, pending: &mut Vec<(usize, i64)>) {
        for k in &node.kids {
            match k {
                Kid::Content(p, m) => pending.push((*p, *m)),
                Kid::Elem(e) if is_note(&e.role) => self.note(e, None),
                Kid::Elem(e) if !is_block(&e.role) && e.actual.is_none() && has_note(e) => self.note_ref(e, page, parts, pending),
                Kid::Elem(e) if e.actual.is_some() || is_block(&e.role) => {
                    let before = self.flush(pending, page);
                    if !before.is_empty() {
                        parts.push(before);
                    }
                    let t = self.text(e, page);
                    if !t.is_empty() {
                        if is_block(&e.role) {
                            parts.push(t);
                        } else {
                            // Tramo en linea con texto propio: se pega a lo anterior.
                            match parts.last_mut() {
                                Some(last) => {
                                    last.push(' ');
                                    last.push_str(&t);
                                }
                                None => parts.push(t),
                            }
                        }
                    }
                }
                Kid::Elem(e) => self.text_into(e, page, parts, pending),
            }
        }
    }

    fn has_block_kid(node: &Node) -> bool {
        // Las notas no cuentan: van aparte sin cortar el parrafo que las cita.
        node.kids.iter().any(|k| matches!(k, Kid::Elem(e) if !is_note(&e.role) && (is_block(&e.role) || Self::has_block_kid(e))))
    }

    fn paragraph(&mut self, node: &Node, kind: Kind, level: u8, out: &mut Vec<Block>) {
        let mut page = None;
        let text = self.text(node, &mut page).replace('\n', " ");
        if !text.is_empty() {
            out.push(Block { kind, level, text, page, ..Block::default() });
        }
    }

    /// Recorre un elemento y agrega sus bloques.
    pub fn walk(&mut self, node: &Node, out: &mut Vec<Block>) {
        match node.role.as_str() {
            "H" | "H1" | "H2" | "H3" | "H4" | "H5" | "H6" | "Title" if !Self::has_block_kid(node) => {
                let (kind, level) = match node.role.as_str() {
                    "Title" => (Kind::Title, 0),
                    "H" => (Kind::Heading, 1),
                    r => (Kind::Heading, r[1..].parse().unwrap_or(1)),
                };
                self.paragraph(node, kind, level, out);
            }
            "L" => self.list(node, 0, out),
            "Note" | "FENote" => self.note(node, None),
            "Table" => self.table(node, out),
            "Figure" => {
                // Una figura con texto adentro (cuadros de texto de InDesign o Canva, rotulos de un
                // diagrama) aporta ese texto como contenido; el bloque de imagen solo va si la figura
                // trae texto alternativo o si es una imagen pura, sin texto.
                let page = self.first_page(node);
                let alt = node.alt.clone().unwrap_or_default();
                if !alt.is_empty() || page.is_none() {
                    out.push(Block { kind: Kind::Image, text: alt, page, ..Block::default() });
                }
                if page.is_some() {
                    if Self::has_block_kid(node) {
                        self.group(node, out);
                    } else {
                        self.paragraph(node, Kind::Paragraph, 0, out);
                    }
                }
            }
            "Document" | "DocumentFragment" | "Part" | "Art" | "Sect" | "Div" | "Aside" | "NonStruct" | "Private" | "TOC" | "Index" | "BlockQuote" | "THead" | "TBody"
            | "TFoot" | "TR" | "LI" | "LBody" | "Form" => self.group(node, out),
            _ => {
                if Self::has_block_kid(node) {
                    self.group(node, out);
                } else {
                    self.paragraph(node, Kind::Paragraph, 0, out);
                }
            }
        }
    }

    /// Elemento contenedor: sus hijos son bloques; el texto suelto entre ellos va como parrafo.
    fn group(&mut self, node: &Node, out: &mut Vec<Block>) {
        let mut pending: Vec<(usize, i64)> = Vec::new();
        for k in &node.kids {
            match k {
                Kid::Content(p, m) => pending.push((*p, *m)),
                Kid::Elem(e) if is_block(&e.role) || Self::has_block_kid(e) => {
                    self.loose(&mut pending, out);
                    self.walk(e, out);
                }
                Kid::Elem(e) if is_note(&e.role) => self.note(e, None),
                Kid::Elem(e) => {
                    // Tramo en linea suelto dentro de un contenedor: se trata como parrafo propio.
                    self.loose(&mut pending, out);
                    self.paragraph(e, Kind::Paragraph, 0, out);
                }
            }
        }
        self.loose(&mut pending, out);
    }

    fn loose(&mut self, pending: &mut Vec<(usize, i64)>, out: &mut Vec<Block>) {
        if pending.is_empty() {
            return;
        }
        let mut page = None;
        let text = self.flush(pending, &mut page);
        if !text.is_empty() {
            out.push(Block { kind: Kind::Paragraph, text, page, ..Block::default() });
        }
    }

    /// Lista: cada LI con su rotulo (Lbl) como vineta o numeracion; las listas anidadas suben de nivel.
    fn list(&mut self, node: &Node, level: u8, out: &mut Vec<Block>) {
        for k in &node.kids {
            let li = match k {
                Kid::Elem(e) => e,
                Kid::Content(..) => continue,
            };
            match li.role.as_str() {
                "LI" => {}
                "L" => {
                    self.list(li, level.saturating_add(1).min(8), out);
                    continue;
                }
                "Caption" => {
                    self.paragraph(li, Kind::Paragraph, 0, out);
                    continue;
                }
                _ => {
                    self.walk(li, out);
                    continue;
                }
            }
            let mut page = None;
            let mut marker = String::new();
            let mut body: Vec<String> = Vec::new();
            let mut nested: Vec<&Node> = Vec::new();
            for kk in &li.kids {
                match kk {
                    Kid::Elem(e) if e.role == "Lbl" => marker = self.text(e, &mut page).replace('\n', " "),
                    Kid::Elem(e) if e.role == "L" => nested.push(e),
                    Kid::Elem(e) if e.role == "LBody" => {
                        // El cuerpo puede traer una lista anidada: va despues, un nivel mas adentro.
                        let mut parts: Vec<String> = Vec::new();
                        let mut pending: Vec<(usize, i64)> = Vec::new();
                        for c in &e.kids {
                            match c {
                                Kid::Elem(x) if x.role == "L" => nested.push(x),
                                Kid::Content(p, m) => pending.push((*p, *m)),
                                Kid::Elem(x) => {
                                    let before = self.flush(&mut pending, &mut page);
                                    if !before.is_empty() {
                                        parts.push(before);
                                    }
                                    let t = self.text(x, &mut page);
                                    if !t.is_empty() {
                                        parts.push(t);
                                    }
                                }
                            }
                        }
                        let t = self.flush(&mut pending, &mut page);
                        if !t.is_empty() {
                            parts.push(t);
                        }
                        body.push(parts.join(" "));
                    }
                    Kid::Elem(e) => {
                        let t = self.text(e, &mut page);
                        if !t.is_empty() {
                            body.push(t);
                        }
                    }
                    Kid::Content(p, m) => {
                        let mut pend = vec![(*p, *m)];
                        let t = self.flush(&mut pend, &mut page);
                        if !t.is_empty() {
                            body.push(t);
                        }
                    }
                }
            }
            let text = clean(&body.join(" "));
            let marker = marker.trim().to_string();
            if !text.is_empty() || !marker.is_empty() {
                // Ordenada si el rotulo es un numero, una letra o un romano ("1.", "a)", "iv.").
                let ordered = !marker.is_empty() && layout::list_marker(&format!("{} x", marker)).map_or(false, |(_, o, _)| o);
                out.push(Block { kind: Kind::ListItem, level, ordered, marker: if marker.is_empty() { None } else { Some(marker) }, text, page, ..Block::default() });
            }
            for n in nested {
                self.list(n, level.saturating_add(1).min(8), out);
            }
        }
    }

    /// Filas de una tabla, atravesando THead/TBody/TFoot.
    fn rows<'n>(node: &'n Node, out: &mut Vec<&'n Node>) {
        for k in &node.kids {
            if let Kid::Elem(e) = k {
                match e.role.as_str() {
                    "TR" => out.push(e),
                    "THead" | "TBody" | "TFoot" | "Div" | "Sect" | "NonStruct" => Self::rows(e, out),
                    _ => {}
                }
            }
        }
    }

    /// Tabla con sus celdas reales. Las combinadas verticalmente dejan, en las filas de abajo, una
    /// celda `merged` en su columna (igual que en Word), para que todas las filas cuadren.
    fn table(&mut self, node: &Node, out: &mut Vec<Block>) {
        let mut trs: Vec<&Node> = Vec::new();
        Self::rows(node, &mut trs);
        let mut caption: Option<&Node> = None;
        for k in &node.kids {
            if let Kid::Elem(e) = k {
                if e.role == "Caption" {
                    caption = Some(e);
                }
            }
        }
        if let Some(c) = caption {
            self.paragraph(c, Kind::Paragraph, 0, out);
        }
        // Columnas ocupadas por combinaciones verticales que vienen de filas anteriores: (filas restantes, ancho).
        let mut carry: HashMap<u32, (u32, u32)> = HashMap::new();
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        let mut page: Option<u32> = None;
        for tr in trs {
            let mut row: Vec<Cell> = Vec::new();
            let mut col = 0u32;
            let take_carry = |col: &mut u32, row: &mut Vec<Cell>, carry: &mut HashMap<u32, (u32, u32)>| {
                while let Some((left, width)) = carry.get(col).copied() {
                    row.push(Cell { merged: true, colspan: width, rowspan: 1, ..Cell::default() });
                    if left <= 1 {
                        carry.remove(col);
                    } else {
                        carry.insert(*col, (left - 1, width));
                    }
                    *col += width;
                }
            };
            for k in &tr.kids {
                let cell = match k {
                    Kid::Elem(e) => e,
                    Kid::Content(..) => continue,
                };
                take_carry(&mut col, &mut row, &mut carry);
                let text = self.text(cell, &mut page);
                let (rs, cs) = (cell.rowspan.max(1), cell.colspan.max(1));
                if rs > 1 {
                    carry.insert(col, (rs - 1, cs));
                }
                row.push(Cell { text, colspan: cs, rowspan: rs, merged: false });
                col += cs;
            }
            // Combinaciones que siguen a la derecha de la ultima celda de la fila.
            let mut rest: Vec<u32> = carry.keys().copied().filter(|c| *c >= col).collect();
            rest.sort_unstable();
            for c in rest {
                while col < c {
                    row.push(Cell { colspan: 1, rowspan: 1, ..Cell::default() });
                    col += 1;
                }
                take_carry(&mut col, &mut row, &mut carry);
            }
            if !row.is_empty() {
                rows.push(row);
            }
        }
        if rows.iter().any(|r| r.iter().any(|c| !c.text.is_empty())) {
            out.push(Block { kind: Kind::Table, rows, page: page.or_else(|| self.first_page(node)), ..Block::default() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pagina de prueba: cada MCID es una linea de texto, de arriba hacia abajo.
    fn pagina(textos: &[(i64, &str)]) -> Vec<PageStore> {
        let mut by_mcid: HashMap<i64, Vec<Item>> = HashMap::new();
        for (i, (m, t)) in textos.iter().enumerate() {
            let y = 700.0 - 14.0 * i as f64;
            let w = 5.0 * t.chars().count() as f64;
            by_mcid.entry(*m).or_default().push(Item { text: t.to_string(), x0: 72.0, y0: y, x1: 72.0 + w, y1: y, size: 10.0, dx: 1.0, dy: 0.0, bold: false, mcid: Some(*m), artifact: false, art: 0 });
        }
        vec![PageStore { bbox: [0.0, 0.0, 612.0, 792.0], rotate: 0, by_mcid }]
    }

    fn el(role: &str, kids: Vec<Kid>) -> Kid {
        Kid::Elem(Node { role: role.into(), kids, rowspan: 1, colspan: 1, ..Node::default() })
    }

    fn c(m: i64) -> Kid {
        Kid::Content(0, m)
    }

    fn armar(root: Kid, pages: &[PageStore]) -> (Vec<Block>, Vec<Block>, HashSet<(usize, i64)>) {
        let mut b = Builder { pages, used: HashSet::new(), notes: Vec::new() };
        let mut out = Vec::new();
        if let Kid::Elem(n) = root {
            b.walk(&n, &mut out);
        }
        (out, b.notes, b.used)
    }

    #[test]
    fn tabla_con_celda_combinada_hacia_abajo() {
        let pages = pagina(&[(0, "Riesgo"), (1, "Control"), (2, "RM3"), (3, "CC1"), (4, "CC2")]);
        let rm3 = Node { role: "TD".into(), kids: vec![c(2)], rowspan: 2, colspan: 1, ..Node::default() };
        let tabla = el("Table", vec![el("THead", vec![el("TR", vec![el("TH", vec![c(0)]), el("TH", vec![c(1)])])]), el("TBody", vec![el("TR", vec![Kid::Elem(rm3), el("TD", vec![c(3)])]), el("TR", vec![el("TD", vec![c(4)])])])]);
        let (out, _, used) = armar(el("Document", vec![tabla]), &pages);
        assert_eq!(out.len(), 1);
        let t = &out[0];
        assert_eq!(t.kind, Kind::Table);
        let filas: Vec<Vec<(String, u32, bool)>> = t.rows.iter().map(|r| r.iter().map(|c| (c.text.clone(), c.rowspan, c.merged)).collect()).collect();
        assert_eq!(filas[0], [("Riesgo".into(), 1, false), ("Control".into(), 1, false)]);
        assert_eq!(filas[1], [("RM3".into(), 2, false), ("CC1".into(), 1, false)]);
        // La celda combinada deja su lugar marcado en la fila de abajo: todas las filas cuadran.
        assert_eq!(filas[2], [(String::new(), 1, true), ("CC2".into(), 1, false)]);
        assert_eq!(used.len(), 5);
    }

    #[test]
    fn listas_con_rotulo_y_anidadas() {
        let pages = pagina(&[(0, "1."), (1, "Detener el equipo"), (2, "•"), (3, "Avisar a sala"), (4, "2."), (5, "Bloquear")]);
        let anidada = el("L", vec![el("LI", vec![el("Lbl", vec![c(2)]), el("LBody", vec![c(3)])])]);
        let lista = el("L", vec![el("LI", vec![el("Lbl", vec![c(0)]), el("LBody", vec![c(1), anidada])]), el("LI", vec![el("Lbl", vec![c(4)]), el("LBody", vec![c(5)])])]);
        let (out, _, _) = armar(el("Document", vec![lista]), &pages);
        let items: Vec<(u8, Option<&str>, bool, &str)> = out.iter().map(|b| (b.level, b.marker.as_deref(), b.ordered, b.text.as_str())).collect();
        assert_eq!(items, [(0, Some("1."), true, "Detener el equipo"), (1, Some("•"), false, "Avisar a sala"), (0, Some("2."), true, "Bloquear")]);
        assert!(out.iter().all(|b| b.kind == Kind::ListItem));
    }

    #[test]
    fn nota_al_pie_va_aparte_y_se_cita() {
        let pages = pagina(&[(0, "Aislar las fuentes de energia."), (1, "1"), (2, "1 Electrica e hidraulica.")]);
        let p = el("P", vec![c(0), el("Link", vec![el("Span", vec![c(1)]), el("Note", vec![el("P", vec![c(2)])])])]);
        let (out, notas, _) = armar(el("Document", vec![p]), &pages);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "Aislar las fuentes de energia.[1]");
        assert_eq!(notas.len(), 1);
        assert_eq!((notas[0].label.as_deref(), notas[0].text.as_str()), (Some("1"), "Electrica e hidraulica."));
    }

    #[test]
    fn texto_real_titulos_y_figuras() {
        let pages = pagina(&[(0, "Alcance"), (1, "ﬁn"), (2, "texto de la figura")]);
        let span = Kid::Elem(Node { role: "Span".into(), kids: vec![c(1)], actual: Some("fin".into()), rowspan: 1, colspan: 1, ..Node::default() });
        let fig = Kid::Elem(Node { role: "Figure".into(), kids: vec![c(2)], alt: Some("Pictograma de casco".into()), rowspan: 1, colspan: 1, ..Node::default() });
        let doc = el("Document", vec![el("H2", vec![c(0)]), el("P", vec![span]), fig]);
        let (out, _, used) = armar(doc, &pages);
        assert_eq!((out[0].kind, out[0].level, out[0].text.as_str()), (Kind::Heading, 2, "Alcance"));
        // /ActualText reemplaza al contenido (ligaduras, simbolos).
        assert_eq!(out[1].text, "fin");
        assert_eq!((out[2].kind, out[2].text.as_str()), (Kind::Image, "Pictograma de casco"));
        // El texto dentro de la figura es contenido: va como parrafo despues de la imagen.
        assert_eq!((out[3].kind, out[3].text.as_str()), (Kind::Paragraph, "texto de la figura"));
        assert_eq!(used.len(), 3);
    }

    #[test]
    fn contenido_inexistente_no_rompe() {
        // MCID que no esta en la pagina y pagina fuera de rango: se ignoran sin panic.
        let pages = pagina(&[(0, "Hola")]);
        let doc = el("Document", vec![el("P", vec![c(0), c(99), Kid::Content(7, 1)]), el("Table", vec![el("TR", vec![])])]);
        let (out, _, _) = armar(doc, &pages);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "Hola");
    }
}
