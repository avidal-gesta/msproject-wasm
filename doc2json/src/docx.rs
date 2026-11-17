//! Lector de documentos Word (.docx, formato WordprocessingML).
//!
//! Un .docx es un ZIP con archivos XML: el cuerpo (`word/document.xml`), los estilos, la
//! numeracion de listas, las notas, encabezados y pies, y las propiedades del documento.

use crate::doc::*;
use crate::symbols::{classify, translate_str, SymbolFont};
use crate::xml::{Ev, Tag, Xml};
use crate::zipread::ZipArchive;
use std::collections::HashMap;

#[derive(Default, Clone)]
struct Style {
    name: String,
    based_on: Option<String>,
    outline: Option<u8>,
    num_id: Option<u32>,
    ilvl: Option<u8>,
}

#[derive(Default, Clone)]
struct Level {
    start: i64,
    fmt: String,
    text: String,
    pstyle: Option<String>,
    font: String,
}

#[derive(Default)]
struct Num {
    abstract_id: String,
    start_override: HashMap<u8, i64>,
}

#[derive(Default)]
struct ParaProps {
    style: Option<String>,
    num_id: Option<u32>,
    ilvl: Option<u8>,
    outline: Option<u8>,
    align: Option<String>,
}

#[derive(Default, Clone, PartialEq)]
struct RunFmt {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    hidden: bool,
}

#[derive(Default)]
struct CellTmp {
    text: String,
    colspan: u32,
    /// Some(true) = inicia una combinacion vertical; Some(false) = continua la de arriba.
    vmerge: Option<bool>,
}

struct Reader<'a> {
    zip: &'a ZipArchive<'a>,
    dir: String,
    styles: HashMap<String, Style>,
    abstracts: HashMap<String, Vec<Level>>,
    nums: HashMap<u32, Num>,
    rels: HashMap<String, String>,
    /// Contadores de numeracion por lista; -1 = nivel aun sin usar.
    counters: HashMap<u32, [i64; 9]>,
    /// Numero visible de cada nota (segun el orden en que se citan): (es_final, id) -> n.
    note_numbers: HashMap<(bool, String), u32>,
    note_seq: [u32; 2],
    /// Profundidad de tablas y cuadros de texto anidados (tope contra archivos maliciosos).
    depth: u32,
}

/// Resuelve un destino de relacion respecto de la carpeta de la parte que lo referencia.
pub(crate) fn resolve(dir: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for p in target.split('/') {
        match p {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Relaciones de una parte: lista de (id, tipo, destino, es_externo).
pub(crate) fn relationships(zip: &ZipArchive, rels_path: &str) -> Vec<(String, String, String, bool)> {
    let mut out = Vec::new();
    if let Some(src) = zip.read_text(rels_path) {
        let mut x = Xml::new(&src);
        while let Some(ev) = x.next() {
            if let Ev::Start(t) = ev {
                if t.name.ends_with("Relationship") {
                    let kind = t.attr("Type").unwrap_or_default();
                    out.push((
                        t.attr("Id").unwrap_or_default(),
                        kind.rsplit('/').next().unwrap_or("").to_string(),
                        t.attr("Target").unwrap_or_default(),
                        t.attr("TargetMode").as_deref() == Some("External"),
                    ));
                }
            }
        }
    }
    out
}

pub(crate) fn roman(mut n: i64, upper: bool) -> String {
    const T: [(i64, &str); 13] =
        [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
    if n <= 0 || n > 3999 {
        return n.to_string();
    }
    let mut s = String::new();
    for (v, r) in T {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    if upper {
        s.to_uppercase()
    } else {
        s
    }
}

/// a, b, ..., z, aa, bb, ... (asi numera Word).
pub(crate) fn letters(n: i64, upper: bool) -> String {
    if n <= 0 {
        return n.to_string();
    }
    let c = (b'a' + ((n - 1) % 26) as u8) as char;
    let s: String = std::iter::repeat(c).take(((n - 1) / 26 + 1) as usize).collect();
    if upper {
        s.to_uppercase()
    } else {
        s
    }
}

fn format_number(n: i64, fmt: &str) -> String {
    match fmt {
        "lowerLetter" => letters(n, false),
        "upperLetter" => letters(n, true),
        "lowerRoman" => roman(n, false),
        "upperRoman" => roman(n, true),
        "decimalZero" => format!("{:02}", n),
        "ordinal" => format!("{}\u{ba}", n),
        "bullet" | "none" => String::new(),
        _ => n.to_string(),
    }
}

impl<'a> Reader<'a> {
    fn load_styles(&mut self, path: &str) {
        let src = match self.zip.read_text(path) {
            Some(s) => s,
            None => return,
        };
        let mut x = Xml::new(&src);
        let mut cur: Option<(String, Style)> = None;
        while let Some(ev) = x.next() {
            match ev {
                Ev::Start(t) => match t.name {
                    "w:style" => cur = t.attr("w:styleId").map(|id| (id, Style::default())),
                    "w:name" => {
                        if let Some((_, s)) = cur.as_mut() {
                            s.name = t.attr("w:val").unwrap_or_default();
                        }
                    }
                    "w:basedOn" => {
                        if let Some((_, s)) = cur.as_mut() {
                            s.based_on = t.attr("w:val");
                        }
                    }
                    "w:outlineLvl" => {
                        if let Some((_, s)) = cur.as_mut() {
                            s.outline = t.attr("w:val").and_then(|v| v.parse().ok());
                        }
                    }
                    "w:numId" => {
                        if let Some((_, s)) = cur.as_mut() {
                            s.num_id = t.attr("w:val").and_then(|v| v.parse().ok());
                        }
                    }
                    "w:ilvl" => {
                        if let Some((_, s)) = cur.as_mut() {
                            s.ilvl = t.attr("w:val").and_then(|v| v.parse().ok());
                        }
                    }
                    _ => {}
                },
                Ev::End("w:style") => {
                    if let Some((id, s)) = cur.take() {
                        self.styles.insert(id, s);
                    }
                }
                _ => {}
            }
        }
    }

    fn load_numbering(&mut self, path: &str) {
        let src = match self.zip.read_text(path) {
            Some(s) => s,
            None => return,
        };
        let mut x = Xml::new(&src);
        let mut abs_id: Option<String> = None;
        let mut lvl: Option<usize> = None;
        let mut num_id: Option<u32> = None;
        let mut override_lvl: Option<u8> = None;
        while let Some(ev) = x.next() {
            match ev {
                Ev::Start(t) => match t.name {
                    "w:abstractNum" => {
                        abs_id = t.attr("w:abstractNumId");
                        if let Some(id) = &abs_id {
                            self.abstracts.insert(id.clone(), vec![Level { start: 1, fmt: "decimal".into(), ..Level::default() }; 9]);
                        }
                    }
                    "w:lvl" => {
                        if num_id.is_some() {
                            // Definicion de nivel dentro de una redefinicion: no se usa.
                            if !t.empty {
                                x.skip("w:lvl");
                            }
                        } else {
                            lvl = t.attr("w:ilvl").and_then(|v| v.parse::<usize>().ok()).filter(|v| *v < 9);
                        }
                    }
                    "w:num" => num_id = t.attr("w:numId").and_then(|v| v.parse().ok()),
                    "w:abstractNumId" => {
                        if let Some(n) = num_id {
                            self.nums.entry(n).or_default().abstract_id = t.attr("w:val").unwrap_or_default();
                        }
                    }
                    "w:lvlOverride" => override_lvl = t.attr("w:ilvl").and_then(|v| v.parse().ok()),
                    "w:startOverride" => {
                        if let (Some(n), Some(l), Some(v)) = (num_id, override_lvl, t.attr("w:val").and_then(|v| v.parse().ok())) {
                            self.nums.entry(n).or_default().start_override.insert(l, v);
                        }
                    }
                    name => {
                        if let (Some(id), Some(l)) = (&abs_id, lvl) {
                            if let Some(level) = self.abstracts.get_mut(id).and_then(|v| v.get_mut(l)) {
                                let val = t.attr("w:val");
                                match name {
                                    "w:start" => level.start = val.and_then(|v| v.parse().ok()).unwrap_or(1),
                                    "w:numFmt" => level.fmt = val.unwrap_or_default(),
                                    "w:lvlText" => level.text = val.unwrap_or_default(),
                                    "w:pStyle" => level.pstyle = val,
                                    "w:rFonts" => level.font = t.attr("w:ascii").or_else(|| t.attr("w:hAnsi")).unwrap_or_default(),
                                    _ => {}
                                }
                            }
                        }
                    }
                },
                Ev::End("w:lvl") => lvl = None,
                Ev::End("w:abstractNum") => abs_id = None,
                Ev::End("w:num") => num_id = None,
                Ev::End("w:lvlOverride") => override_lvl = None,
                _ => {}
            }
        }
    }

    /// Recorre la cadena de estilos (estilo -> basado en -> ...).
    fn style_chain(&self, id: &Option<String>) -> Vec<&Style> {
        let mut out = Vec::new();
        let mut cur = id.clone();
        while let Some(c) = cur {
            match self.styles.get(&c) {
                Some(s) if out.len() < 12 => {
                    out.push(s);
                    cur = s.based_on.clone();
                }
                _ => break,
            }
        }
        out
    }

    /// Nivel de titulo segun el estilo: `Some(0)` es el titulo del documento.
    fn heading_level(&self, p: &ParaProps) -> Option<u8> {
        if let Some(o) = p.outline {
            return if o < 9 { Some(o + 1) } else { None };
        }
        for s in self.style_chain(&p.style) {
            let name = s.name.to_ascii_lowercase();
            if let Some(n) = name.strip_prefix("heading ").or_else(|| name.strip_prefix("t\u{ed}tulo ")).or_else(|| name.strip_prefix("titulo ")) {
                if let Ok(v) = n.trim().parse::<u8>() {
                    if (1..=9).contains(&v) {
                        return Some(v);
                    }
                }
            }
            if name == "title" {
                return Some(0);
            }
            if let Some(o) = s.outline {
                return if o < 9 { Some(o + 1) } else { None };
            }
        }
        None
    }

    /// Numeracion efectiva del parrafo: (lista, nivel), propia o heredada del estilo.
    fn numbering_of(&self, p: &ParaProps) -> Option<(u32, u8)> {
        let chain = self.style_chain(&p.style);
        let num_id = p.num_id.or_else(|| chain.iter().find_map(|s| s.num_id))?;
        if num_id == 0 {
            return None;
        }
        let mut ilvl = p.ilvl.or_else(|| chain.iter().find_map(|s| s.ilvl));
        if ilvl.is_none() {
            // El nivel puede estar ligado al estilo desde la definicion de la lista.
            if let (Some(style), Some(levels)) = (&p.style, self.nums.get(&num_id).and_then(|n| self.abstracts.get(&n.abstract_id))) {
                ilvl = levels.iter().position(|l| l.pstyle.as_ref() == Some(style)).map(|i| i as u8);
            }
        }
        Some((num_id, ilvl.unwrap_or(0).min(8)))
    }

    /// Avanza los contadores y devuelve (es_ordenada, marcador visible).
    fn list_marker(&mut self, num_id: u32, ilvl: u8) -> (bool, String) {
        let num = match self.nums.get(&num_id) {
            Some(n) => n,
            None => return (false, String::new()),
        };
        let levels = match self.abstracts.get(&num.abstract_id) {
            Some(l) => l,
            None => return (false, String::new()),
        };
        let start_of = |l: usize| num.start_override.get(&(l as u8)).copied().unwrap_or(levels[l].start);
        let c = self.counters.entry(num_id).or_insert([-1; 9]);
        let i = ilvl as usize;
        c[i] = if c[i] < 0 { start_of(i) } else { c[i] + 1 };
        for l in 0..i {
            if c[l] < 0 {
                c[l] = start_of(l);
            }
        }
        for v in c.iter_mut().skip(i + 1) {
            *v = -1;
        }
        let level = &levels[i];
        if level.fmt == "bullet" {
            let font = classify(&level.font);
            let m = translate_str(if font == SymbolFont::None { SymbolFont::Other } else { font }, &level.text);
            let m = if m.trim().is_empty() || m == "o" { "\u{2022}".to_string() } else { m };
            return (false, m);
        }
        if level.fmt == "none" {
            return (false, String::new());
        }
        // "%1.%2." -> "3.1."
        let mut out = String::new();
        let mut chars = level.text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '%' {
                if let Some(d) = chars.peek().and_then(|d| d.to_digit(10)) {
                    chars.next();
                    let l = (d as usize).saturating_sub(1).min(8);
                    let value = if c[l] < 0 { start_of(l) } else { c[l] };
                    out.push_str(&format_number(value, &levels[l].fmt));
                    continue;
                }
            }
            out.push(ch);
        }
        (true, out.trim().to_string())
    }

    fn note_number(&mut self, endnote: bool, id: Option<String>) -> u32 {
        let id = id.unwrap_or_default();
        if let Some(n) = self.note_numbers.get(&(endnote, id.clone())) {
            return *n;
        }
        self.note_seq[endnote as usize] += 1;
        let n = self.note_seq[endnote as usize];
        self.note_numbers.insert((endnote, id), n);
        n
    }

    fn para_props(&self, x: &mut Xml, p: &mut ParaProps) {
        while let Some(ev) = x.next() {
            match ev {
                Ev::End("w:pPr") => break,
                Ev::Start(t) => match t.name {
                    "w:pStyle" => p.style = t.attr("w:val"),
                    "w:numId" => p.num_id = t.attr("w:val").and_then(|v| v.parse().ok()),
                    "w:ilvl" => p.ilvl = t.attr("w:val").and_then(|v| v.parse().ok()),
                    "w:outlineLvl" => p.outline = t.attr("w:val").and_then(|v| v.parse().ok()),
                    "w:jc" => p.align = t.attr("w:val").filter(|v| v != "left" && v != "start"),
                    // Formato de la marca de parrafo, cambios registrados y seccion: no aportan.
                    "w:rPr" | "w:pPrChange" | "w:sectPr" if !t.empty => x.skip(t.name),
                    _ => {}
                },
                _ => {}
            }
        }
    }

    fn run_props(&self, x: &mut Xml) -> RunFmt {
        let mut f = RunFmt::default();
        while let Some(ev) = x.next() {
            match ev {
                Ev::End("w:rPr") => break,
                Ev::Start(t) => match t.name {
                    "w:b" => f.bold = t.on("w:val"),
                    "w:i" => f.italic = t.on("w:val"),
                    "w:u" => f.underline = t.on("w:val"),
                    "w:strike" | "w:dstrike" => f.strike = t.on("w:val"),
                    "w:vanish" => f.hidden = t.on("w:val"),
                    "w:rPrChange" if !t.empty => x.skip("w:rPrChange"),
                    _ => {}
                },
                _ => {}
            }
        }
        f
    }

    /// Lee un parrafo (ya abierto) y agrega a `out` los bloques resultantes.
    fn paragraph(&mut self, x: &mut Xml, out: &mut Vec<Block>) {
        let mut props = ParaProps::default();
        let mut text = String::new();
        let mut runs: Vec<Run> = Vec::new();
        let mut fmt = RunFmt::default();
        let mut in_run = false;
        let mut link: Option<String> = None;
        let mut images: Vec<(String, String)> = Vec::new();
        let mut alt = String::new();
        let mut nested: Vec<Block> = Vec::new();
        let mut page_break = false;

        fn push(text: &mut String, runs: &mut Vec<Run>, s: &str, fmt: &RunFmt, link: &Option<String>) {
            if s.is_empty() || fmt.hidden {
                return;
            }
            text.push_str(s);
            match runs.last_mut() {
                Some(r) if r.bold == fmt.bold && r.italic == fmt.italic && r.underline == fmt.underline && r.strike == fmt.strike && r.link == *link => r.text.push_str(s),
                _ => runs.push(Run { text: s.to_string(), bold: fmt.bold, italic: fmt.italic, underline: fmt.underline, strike: fmt.strike, link: link.clone() }),
            }
        }

        while let Some(ev) = x.next() {
            match ev {
                Ev::End("w:p") => break,
                Ev::End("w:r") => in_run = false,
                Ev::End("w:hyperlink") => link = None,
                Ev::End(_) | Ev::Text(_) => {}
                Ev::Start(t) => match t.name {
                    "w:pPr" if !t.empty => self.para_props(x, &mut props),
                    "w:r" => {
                        in_run = !t.empty;
                        fmt = RunFmt::default();
                    }
                    "w:rPr" if !t.empty => {
                        if in_run {
                            fmt = self.run_props(x);
                        } else {
                            x.skip("w:rPr");
                        }
                    }
                    "w:t" if !t.empty => {
                        let s = x.text_until("w:t");
                        push(&mut text, &mut runs, &translate_str(SymbolFont::None, &s), &fmt, &link);
                    }
                    "w:tab" | "w:ptab" => push(&mut text, &mut runs, "\t", &fmt, &link),
                    "w:br" | "w:cr" => match t.attr("w:type").as_deref() {
                        Some("page") => page_break = true,
                        Some("column") => {}
                        _ => push(&mut text, &mut runs, "\n", &fmt, &link),
                    },
                    "w:noBreakHyphen" => push(&mut text, &mut runs, "-", &fmt, &link),
                    "w:sym" => {
                        let code = t.attr("w:char").and_then(|c| u32::from_str_radix(&c, 16).ok()).and_then(char::from_u32);
                        if let Some(c) = code {
                            let font = classify(&t.attr("w:font").unwrap_or_default());
                            let s = translate_str(if font == SymbolFont::None { SymbolFont::Other } else { font }, &c.to_string());
                            push(&mut text, &mut runs, &s, &fmt, &link);
                        }
                    }
                    "w:hyperlink" => {
                        link = t.attr("r:id").and_then(|id| self.rels.get(&id).cloned()).or_else(|| t.attr("w:anchor").map(|a| format!("#{}", a)));
                    }
                    "w:footnoteReference" => {
                        let n = self.note_number(false, t.attr("w:id"));
                        push(&mut text, &mut runs, &format!("[{}]", n), &fmt, &link);
                    }
                    "w:endnoteReference" => {
                        let n = self.note_number(true, t.attr("w:id"));
                        push(&mut text, &mut runs, &format!("[{}]", roman(n as i64, false)), &fmt, &link);
                    }
                    // Texto eliminado (control de cambios), codigos de campo y la copia de
                    // compatibilidad de los cuadros de texto: no son contenido visible.
                    "w:del" | "w:moveFrom" | "w:instrText" | "w:delText" | "w:delInstrText" | "mc:Fallback" | "w:sdtPr" | "w:sdtEndPr" if !t.empty => x.skip(t.name),
                    "w:txbxContent" if !t.empty => {
                        let mut b = self.blocks(x, "w:txbxContent");
                        nested.append(&mut b);
                    }
                    "wp:docPr" => alt = t.attr("descr").or_else(|| t.attr("title")).unwrap_or_default(),
                    "a:blip" | "v:imagedata" => {
                        if let Some(target) = t.attr("r:embed").or_else(|| t.attr("r:id")).and_then(|id| self.rels.get(&id).cloned()) {
                            let name = target.rsplit('/').next().unwrap_or(&target).to_string();
                            images.push((name, std::mem::take(&mut alt)));
                        }
                    }
                    "m:t" if !t.empty => {
                        let s = x.text_until("m:t");
                        push(&mut text, &mut runs, &s, &fmt, &link);
                    }
                    _ => {}
                },
            }
        }

        let trimmed = text.trim_matches(|c: char| c.is_whitespace() && c != '\t').trim_end();
        if !trimmed.trim().is_empty() {
            let mut b = Block { text: trimmed.to_string(), runs, align: props.align.clone(), ..Block::default() };
            b.style = self.style_chain(&props.style).first().map(|s| s.name.clone()).filter(|n| !n.is_empty());
            let heading = self.heading_level(&props);
            let numbering = self.numbering_of(&props);
            if let Some((num_id, ilvl)) = numbering {
                let (ordered, marker) = self.list_marker(num_id, ilvl);
                if !marker.is_empty() {
                    b.marker = Some(marker);
                }
                b.ordered = ordered;
                b.level = ilvl;
                b.kind = Kind::ListItem;
            }
            match heading {
                Some(0) => b.kind = Kind::Title,
                Some(l) => {
                    b.kind = Kind::Heading;
                    b.level = l;
                }
                None => {}
            }
            out.push(b);
        }
        for (name, alt) in images {
            out.push(Block { kind: Kind::Image, name: Some(name), text: alt, ..Block::default() });
        }
        out.append(&mut nested);
        if page_break {
            out.push(Block { kind: Kind::PageBreak, ..Block::default() });
        }
    }

    fn table(&mut self, x: &mut Xml) -> Block {
        let mut rows: Vec<Vec<CellTmp>> = Vec::new();
        if self.depth >= 40 {
            x.skip("w:tbl");
            return Block { kind: Kind::Table, ..Block::default() };
        }
        self.depth += 1;
        while let Some(ev) = x.next() {
            match ev {
                Ev::End("w:tbl") => break,
                Ev::Start(t) => match t.name {
                    "w:tr" if !t.empty => rows.push(Vec::new()),
                    "w:tc" if !t.empty => {
                        let cell = self.cell(x);
                        if rows.is_empty() {
                            rows.push(Vec::new());
                        }
                        if let Some(r) = rows.last_mut() {
                            r.push(cell);
                        }
                    }
                    "w:tblPr" | "w:tblGrid" | "w:trPr" | "w:tblPrEx" | "w:del" | "w:sdtPr" if !t.empty => x.skip(t.name),
                    _ => {}
                },
                _ => {}
            }
        }
        self.depth -= 1;
        // Combinaciones verticales: la celda que inicia recibe el alto total.
        let col_of = |row: &Vec<CellTmp>, idx: usize| row.iter().take(idx).map(|c| c.colspan.max(1)).sum::<u32>();
        let mut out: Vec<Vec<Cell>> = Vec::with_capacity(rows.len());
        for (r, row) in rows.iter().enumerate() {
            let mut cells = Vec::with_capacity(row.len());
            for (i, c) in row.iter().enumerate() {
                let mut rowspan = 1;
                if c.vmerge == Some(true) {
                    let col = col_of(row, i);
                    for below in rows.iter().skip(r + 1) {
                        match below.iter().enumerate().find(|(j, _)| col_of(below, *j) == col) {
                            Some((_, b)) if b.vmerge == Some(false) => rowspan += 1,
                            _ => break,
                        }
                    }
                }
                cells.push(Cell { text: c.text.clone(), colspan: c.colspan.max(1), rowspan, merged: c.vmerge == Some(false) });
            }
            out.push(cells);
        }
        Block { kind: Kind::Table, rows: out, ..Block::default() }
    }

    fn cell(&mut self, x: &mut Xml) -> CellTmp {
        let mut c = CellTmp { colspan: 1, ..CellTmp::default() };
        let mut blocks = Vec::new();
        while let Some(ev) = x.next() {
            match ev {
                Ev::End("w:tc") => break,
                Ev::Start(t) if t.name == "w:tcPr" && !t.empty => {
                    while let Some(ev) = x.next() {
                        match ev {
                            Ev::End("w:tcPr") => break,
                            Ev::Start(p) => match p.name {
                                "w:gridSpan" => c.colspan = p.attr("w:val").and_then(|v| v.parse().ok()).unwrap_or(1),
                                "w:vMerge" => c.vmerge = Some(p.attr("w:val").as_deref() == Some("restart")),
                                "w:tcPrChange" if !p.empty => x.skip("w:tcPrChange"),
                                _ => {}
                            },
                            _ => {}
                        }
                    }
                }
                Ev::Start(t) => self.dispatch(x, &t, &mut blocks),
                _ => {}
            }
        }
        c.text = plain_text(&blocks);
        c
    }

    /// Trata una etiqueta de nivel de bloque.
    fn dispatch(&mut self, x: &mut Xml, t: &Tag, out: &mut Vec<Block>) {
        match t.name {
            "w:p" if !t.empty => self.paragraph(x, out),
            "w:tbl" if !t.empty => {
                let b = self.table(x);
                if !b.rows.is_empty() {
                    out.push(b);
                }
            }
            "w:sdtPr" | "w:sdtEndPr" | "w:sectPr" | "w:del" | "w:moveFrom" | "mc:Fallback" if !t.empty => x.skip(t.name),
            _ => {}
        }
    }

    /// Lee bloques hasta el cierre del contenedor `end`.
    fn blocks(&mut self, x: &mut Xml, end: &str) -> Vec<Block> {
        let mut out = Vec::new();
        if self.depth >= 40 {
            x.skip(end);
            return out;
        }
        self.depth += 1;
        while let Some(ev) = x.next() {
            match ev {
                Ev::End(n) if n == end => break,
                Ev::Start(t) => self.dispatch(x, &t, &mut out),
                _ => {}
            }
        }
        self.depth -= 1;
        out
    }

    /// Lee una parte secundaria completa (encabezado, pie): devuelve sus bloques.
    fn part_blocks(&mut self, path: &str, root: &str) -> Vec<Block> {
        let src = match self.zip.read_text(path) {
            Some(s) => s,
            None => return Vec::new(),
        };
        let saved = std::mem::take(&mut self.rels);
        self.rels = part_rels(self.zip, path);
        let mut x = Xml::new(&src);
        let mut out = Vec::new();
        while let Some(ev) = x.next() {
            if let Ev::Start(t) = ev {
                if t.name == root && !t.empty {
                    out = self.blocks(&mut x, root);
                    break;
                }
            }
        }
        self.rels = saved;
        out
    }

    /// Notas al pie, notas finales o comentarios: un bloque por nota.
    fn notes(&mut self, path: &str, item: &str, endnote: Option<bool>) -> Vec<Block> {
        let src = match self.zip.read_text(path) {
            Some(s) => s,
            None => return Vec::new(),
        };
        let mut x = Xml::new(&src);
        let mut out: Vec<(u32, Block)> = Vec::new();
        while let Some(ev) = x.next() {
            if let Ev::Start(t) = ev {
                if t.name == item && !t.empty {
                    let id = t.attr("w:id");
                    let special = t.attr("w:type").is_some();
                    let author = t.attr("w:author");
                    let text = plain_text(&self.blocks(&mut x, item));
                    if special || text.trim().is_empty() {
                        continue; // separadores de notas
                    }
                    let (order, label) = match endnote {
                        Some(e) => match self.note_numbers.get(&(e, id.clone().unwrap_or_default())) {
                            Some(n) => (*n, if e { roman(*n as i64, false) } else { n.to_string() }),
                            None => (u32::MAX, id.unwrap_or_default()),
                        },
                        None => (out.len() as u32, author.unwrap_or_default()),
                    };
                    out.push((order, Block { text, label: Some(label).filter(|l| !l.is_empty()), ..Block::default() }));
                }
            }
        }
        out.sort_by_key(|(o, _)| *o);
        out.into_iter().map(|(_, b)| b).collect()
    }
}

pub(crate) fn part_rels(zip: &ZipArchive, part: &str) -> HashMap<String, String> {
    let (dir, file) = match part.rfind('/') {
        Some(i) => (&part[..i], &part[i + 1..]),
        None => ("", part),
    };
    let rels_path = if dir.is_empty() { format!("_rels/{}.rels", file) } else { format!("{}/_rels/{}.rels", dir, file) };
    relationships(zip, &rels_path).into_iter().map(|(id, _, target, external)| (id, if external { target } else { resolve(dir, &target) })).collect()
}

/// Propiedades del documento (docProps/core.xml y app.xml).
pub(crate) fn properties(zip: &ZipArchive, doc: &mut Document) {
    let mut text = HashMap::new();
    for path in ["docProps/core.xml", "docProps/app.xml"] {
        if let Some(src) = zip.read_text(path) {
            let mut x = Xml::new(&src);
            while let Some(ev) = x.next() {
                if let Ev::Start(t) = ev {
                    if !t.empty && !t.name.ends_with("Properties") && !t.name.contains("vt:") && !matches!(t.name, "HeadingPairs" | "TitlesOfParts") {
                        let name = t.name.to_string();
                        let v = x.text_until(&name);
                        text.insert(name, v);
                    } else if !t.empty && matches!(t.name, "HeadingPairs" | "TitlesOfParts") {
                        x.skip(t.name);
                    }
                }
            }
        }
    }
    let mut get = |k: &str| text.remove(k);
    doc.meta_text("title", get("dc:title"));
    doc.meta_text("subject", get("dc:subject"));
    doc.meta_text("author", get("dc:creator"));
    doc.meta_text("keywords", get("cp:keywords"));
    doc.meta_text("description", get("dc:description"));
    doc.meta_text("category", get("cp:category"));
    doc.meta_text("last_modified_by", get("cp:lastModifiedBy"));
    doc.meta_text("created", get("dcterms:created"));
    doc.meta_text("modified", get("dcterms:modified"));
    doc.meta_int("revision", get("cp:revision").and_then(|v| v.trim().parse().ok()));
    doc.meta_text("application", get("Application"));
    doc.meta_text("company", get("Company"));
    doc.meta_text("manager", get("Manager"));
    doc.meta_text("template", get("Template"));
    let pages = get("Pages").and_then(|v| v.trim().parse::<i64>().ok()).filter(|v| *v > 0);
    doc.page_count = pages.map(|p| p as usize);
    doc.meta_int("words_reported", get("Words").and_then(|v| v.trim().parse().ok()));
}

pub fn read(data: &[u8]) -> Result<Document, String> {
    let zip = ZipArchive::open(data).map_err(|_| "El archivo no es un .docx valido (no es un ZIP).".to_string())?;

    // La parte principal se declara en las relaciones del paquete.
    let main = relationships(&zip, "_rels/.rels")
        .into_iter()
        .find(|(_, kind, _, _)| kind == "officeDocument")
        .map(|(_, _, target, _)| resolve("", &target))
        .filter(|p| zip.find(p).is_some())
        .or_else(|| zip.find("word/document.xml").map(|e| e.name.clone()));
    let main = match main {
        Some(m) if !m.starts_with("xl/") && !m.starts_with("ppt/") => m,
        Some(m) if m.starts_with("xl/") => return Err("El archivo es un libro de Excel (.xlsx), no un documento de Word.".into()),
        Some(_) => return Err("El archivo es una presentacion de PowerPoint (.pptx), no un documento de Word.".into()),
        None => return Err("El archivo no es un documento de Word (.docx): no contiene word/document.xml.".into()),
    };
    let dir = main.rfind('/').map_or(String::new(), |i| main[..i].to_string());

    let mut r = Reader {
        zip: &zip,
        dir: dir.clone(),
        styles: HashMap::new(),
        abstracts: HashMap::new(),
        nums: HashMap::new(),
        rels: HashMap::new(),
        counters: HashMap::new(),
        note_numbers: HashMap::new(),
        note_seq: [0, 0],
        depth: 0,
    };

    // Partes relacionadas con el documento principal.
    let file = main.rsplit('/').next().unwrap_or(&main);
    let rels_path = if dir.is_empty() { format!("_rels/{}.rels", file) } else { format!("{}/_rels/{}.rels", dir, file) };
    let mut parts: HashMap<String, Vec<String>> = HashMap::new();
    for (id, kind, target, external) in relationships(&zip, &rels_path) {
        let path = if external { target } else { resolve(&r.dir, &target) };
        parts.entry(kind).or_default().push(path.clone());
        r.rels.insert(id, path);
    }
    let first = |k: &str, default: &str| parts.get(k).and_then(|v| v.first().cloned()).unwrap_or_else(|| resolve(&dir, default));
    r.load_styles(&first("styles", "styles.xml"));
    r.load_numbering(&first("numbering", "numbering.xml"));

    let src = zip.read_text(&main).ok_or("No se pudo leer el contenido del documento.")?;
    let mut doc = Document { kind: "docx", ..Document::default() };
    properties(&zip, &mut doc);

    let mut x = Xml::new(&src);
    let mut found = false;
    while let Some(ev) = x.next() {
        if let Ev::Start(t) = ev {
            if t.name == "w:body" && !t.empty {
                doc.content = r.blocks(&mut x, "w:body");
                found = true;
                break;
            }
        }
    }
    if !found {
        return Err("El documento no tiene cuerpo (w:body); puede estar danado.".into());
    }
    // Un salto de pagina al final no separa nada.
    while matches!(doc.content.last(), Some(b) if b.kind == Kind::PageBreak) {
        doc.content.pop();
    }

    let body_rels = std::mem::take(&mut r.rels);
    for (key, kind, root) in [("headers", "header", "w:hdr"), ("footers", "footer", "w:ftr")] {
        let mut blocks: Vec<Block> = Vec::new();
        let mut paths = parts.get(kind).cloned().unwrap_or_default();
        paths.sort();
        for path in paths {
            let text = plain_text(&r.part_blocks(&path, root));
            if !text.trim().is_empty() && !blocks.iter().any(|b| b.text == text) {
                blocks.push(Block { text, ..Block::default() });
            }
        }
        doc.extras.push((key, blocks));
    }
    r.rels = body_rels;
    let footnotes = r.notes(&first("footnotes", "footnotes.xml"), "w:footnote", Some(false));
    let endnotes = r.notes(&first("endnotes", "endnotes.xml"), "w:endnote", Some(true));
    let comments = r.notes(&first("comments", "comments.xml"), "w:comment", None);
    doc.extras.push(("footnotes", footnotes));
    doc.extras.push(("endnotes", endnotes));
    doc.extras.push(("comments", comments));
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeros() {
        assert_eq!(roman(4, false), "iv");
        assert_eq!(roman(1994, true), "MCMXCIV");
        assert_eq!(letters(1, false), "a");
        assert_eq!(letters(27, true), "AA");
        assert_eq!(format_number(3, "decimalZero"), "03");
        assert_eq!(resolve("word", "media/image1.png"), "word/media/image1.png");
        assert_eq!(resolve("word", "../docProps/app.xml"), "docProps/app.xml");
        assert_eq!(resolve("word", "/word/styles.xml"), "word/styles.xml");
    }
}
