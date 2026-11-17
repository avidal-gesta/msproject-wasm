//! Reconstruccion del texto de una pagina: de tramos sueltos con coordenadas a lineas en orden
//! de lectura, y de lineas a bloques (titulos, parrafos, items de lista y tablas).

use super::content::Item;
use crate::doc::{Block, Cell, Kind};

/// Una linea de texto ya armada. `y` se mide desde el borde superior de la pagina.
#[derive(Clone, Debug, Default)]
pub struct TLine {
    pub text: String,
    pub x0: f64,
    pub x1: f64,
    pub y: f64,
    pub size: f64,
    pub bold: bool,
    /// Celdas de la linea (tramos separados por un hueco ancho): (x inicial, x final, texto).
    pub cells: Vec<(f64, f64, String)>,
    /// La linea esta escrita en otra direccion que el resto (texto vertical, sellos...).
    pub rotated: bool,
}

struct It {
    text: String,
    /// Posicion a lo largo de la linea (inicio y fin) y altura de la linea base.
    a0: f64,
    a1: f64,
    b: f64,
    size: f64,
    bold: bool,
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 0.08 * a.max(b)
}

/// Arma las lineas de una pagina. Devuelve (lineas, ancho, alto) con la pagina ya derecha.
pub fn lines(items: Vec<Item>, bbox: [f64; 4], rotate: i64) -> (Vec<TLine>, f64, f64) {
    let (w, h) = (bbox[2] - bbox[0], bbox[3] - bbox[1]);
    let (pw, ph) = if rotate == 90 || rotate == 270 { (h, w) } else { (w, h) };
    // Coordenadas y direcciones con la pagina en su orientacion de lectura.
    let point = |x: f64, y: f64| {
        let (x, y) = (x - bbox[0], y - bbox[1]);
        match rotate {
            90 => (y, w - x),
            180 => (w - x, h - y),
            270 => (h - y, x),
            _ => (x, y),
        }
    };
    let dir = |dx: f64, dy: f64| match rotate {
        90 => (dy, -dx),
        180 => (-dx, -dy),
        270 => (-dy, dx),
        _ => (dx, dy),
    };

    // Un grupo por direccion de escritura: 0 = normal, 1 = hacia arriba, 2 = invertido, 3 = hacia abajo.
    let mut groups: [Vec<It>; 4] = Default::default();
    for it in items {
        let (dx, dy) = dir(it.dx, it.dy);
        let (x0, y0) = point(it.x0, it.y0);
        let (x1, y1) = point(it.x1, it.y1);
        let q = if dx >= 0.7 {
            0
        } else if dy > 0.7 {
            1
        } else if dx <= -0.7 {
            2
        } else {
            3
        };
        // Se gira cada grupo para que su texto quede horizontal de izquierda a derecha.
        let (a0, a1, b) = match q {
            0 => (x0, x1, y0),
            1 => (y0, y1, -x0),
            2 => (-x0, -x1, -y0),
            _ => (-y0, -y1, x0),
        };
        groups[q].push(It { text: it.text, a0: a0.min(a1), a1: a0.max(a1), b, size: it.size, bold: it.bold });
    }

    let mut out: Vec<TLine> = Vec::new();
    for (q, mut its) in groups.into_iter().enumerate() {
        // De arriba hacia abajo y, a igual altura, de izquierda a derecha.
        its.sort_by(|p, r| r.b.partial_cmp(&p.b).unwrap_or(std::cmp::Ordering::Equal).then(p.a0.partial_cmp(&r.a0).unwrap_or(std::cmp::Ordering::Equal)));
        let mut rows: Vec<Vec<It>> = Vec::new();
        let mut meta: Vec<(f64, f64)> = Vec::new(); // (linea base, tamano) de cada fila
        for it in its {
            let mut found = None;
            for k in (rows.len().saturating_sub(4)..rows.len()).rev() {
                let (lb, ls) = meta[k];
                let d = (it.b - lb).abs();
                // Misma linea base, o un superindice/subindice pegado a una linea mas grande.
                let small = it.size < 0.8 * ls || ls < 0.8 * it.size;
                if d <= 0.5 * it.size.min(ls) || (small && d <= 0.55 * it.size.max(ls)) {
                    found = Some(k);
                    break;
                }
            }
            match found {
                Some(k) => {
                    if it.size > meta[k].1 {
                        meta[k] = (it.b, it.size);
                    }
                    rows[k].push(it);
                }
                None => {
                    meta.push((it.b, it.size));
                    rows.push(vec![it]);
                }
            }
        }

        for (row, (base, _)) in rows.into_iter().zip(meta) {
            let mut row = row;
            row.sort_by(|p, r| p.a0.partial_cmp(&r.a0).unwrap_or(std::cmp::Ordering::Equal));
            let mut cells: Vec<(f64, f64, String)> = Vec::new();
            let mut cur = String::new();
            let (mut cell_start, mut end) = (0.0f64, 0.0f64);
            let mut prev: Option<(f64, f64, f64)> = None; // (a0, a1, tamano) del tramo anterior
            let mut prev_text = String::new();
            let (mut chars, mut bold_chars, mut size_weight) = (0usize, 0usize, 0.0f64);
            for it in &row {
                let n = it.text.chars().count();
                if let Some((pa0, pa1, psize)) = prev {
                    let sz = it.size.max(psize);
                    // Texto repetido casi en el mismo lugar: negrita simulada o sombra.
                    if it.text == prev_text && (it.a0 - pa0).abs() < 0.25 * sz + 0.5 {
                        continue;
                    }
                    let gap = it.a0 - pa1;
                    if gap > 1.5 * sz {
                        cells.push((cell_start, end, std::mem::take(&mut cur)));
                        cell_start = it.a0;
                    } else if (gap > 0.15 * sz || gap < -0.5 * sz) && !cur.ends_with(' ') && !it.text.starts_with(' ') {
                        cur.push(' ');
                    }
                } else {
                    cell_start = it.a0;
                }
                cur.push_str(&it.text);
                end = it.a1.max(end);
                if cells.is_empty() && cur.len() == it.text.len() {
                    end = it.a1;
                }
                prev = Some((it.a0, it.a1, it.size));
                prev_text = it.text.clone();
                chars += n;
                size_weight += it.size * n as f64;
                if it.bold {
                    bold_chars += n;
                }
            }
            cells.push((cell_start, end, cur));
            let cells: Vec<(f64, f64, String)> = cells.into_iter().map(|(a, b, t)| (a, b, t.trim().to_string())).filter(|c| !c.2.is_empty()).collect();
            if cells.is_empty() || chars == 0 {
                continue;
            }
            let text = cells.iter().map(|c| c.2.as_str()).collect::<Vec<_>>().join("\t");
            let (x0, x1) = (cells[0].0, cells.iter().map(|c| c.1).fold(f64::MIN, f64::max));
            // Las lineas giradas se informan con la posicion de su inicio en la pagina.
            let (px, py) = match q {
                0 => (x0, ph - base),
                1 => (-base, ph - x0),
                2 => (-x0, ph + base),
                _ => (base, ph + x0),
            };
            out.push(TLine { text, x0: px, x1: if q == 0 { x1 } else { px }, y: py, size: size_weight / chars as f64, bold: bold_chars * 2 > chars, cells, rotated: q != 0 });
        }
    }
    (out, pw, ph)
}

/// Si la pagina tiene dos columnas de texto corrido, reordena las lineas para leer primero la
/// columna izquierda completa y luego la derecha. Las tablas se dejan fila por fila.
pub fn reorder_columns(lines: Vec<TLine>, page_width: f64) -> Vec<TLine> {
    let normal: Vec<&TLine> = lines.iter().filter(|l| !l.rotated).collect();
    if normal.len() < 12 {
        return lines;
    }
    // Candidatos a separacion: huecos entre celdas cerca del centro de la pagina.
    let (lo, hi) = (page_width * 0.3, page_width * 0.7);
    let mut votes: Vec<(f64, f64)> = Vec::new();
    for l in &normal {
        for pair in l.cells.windows(2) {
            let (a, b) = (pair[0].1, pair[1].0);
            if b - a > 8.0 && a < hi && b > lo {
                votes.push((a.max(lo), b.min(hi)));
            }
        }
    }
    if votes.len() * 2 < normal.len() {
        return lines;
    }
    // Posicion x cubierta por el mayor numero de huecos.
    let mut best = (0usize, 0.0f64);
    let mut x = lo;
    while x <= hi {
        let n = votes.iter().filter(|(a, b)| *a <= x && x <= *b).count();
        if n > best.0 {
            best = (n, x);
        }
        x += 2.0;
    }
    let gutter = best.1;
    let split: Vec<bool> = lines.iter().map(|l| !l.rotated && l.cells.len() == 2 && l.cells[0].1 <= gutter && l.cells[1].0 >= gutter).collect();
    let n_split = split.iter().filter(|s| **s).count();
    // Texto corrido: lineas largas a ambos lados. En una tabla las celdas son cortas o hay mas de dos.
    let words = |s: &str| s.split_whitespace().count();
    let prose = lines.iter().zip(&split).filter(|(l, s)| **s && words(&l.cells[0].2) >= 4 && words(&l.cells[1].2) >= 4).count();
    if n_split < 8 || prose * 10 < n_split * 6 {
        return lines;
    }
    // Tramos consecutivos de lineas partidas: primero todas las mitades izquierdas, luego las derechas.
    let mut out: Vec<TLine> = Vec::with_capacity(lines.len() + n_split);
    let mut right: Vec<TLine> = Vec::new();
    for (l, s) in lines.into_iter().zip(split) {
        let one_side = !l.rotated && l.cells.len() == 1;
        if s {
            let (a, b) = (l.cells[0].clone(), l.cells[1].clone());
            out.push(TLine { text: a.2.clone(), x0: a.0, x1: a.1, cells: vec![a], ..l.clone() });
            right.push(TLine { text: b.2.clone(), x0: b.0, x1: b.1, cells: vec![b], ..l });
        } else if one_side && !right.is_empty() && l.cells[0].0 >= gutter {
            // Linea que solo existe en la columna derecha.
            right.push(l);
        } else if one_side && !right.is_empty() && l.cells[0].1 <= gutter {
            out.push(l);
        } else {
            // Linea a todo el ancho: cierra el tramo de columnas.
            out.append(&mut right);
            out.push(l);
        }
    }
    out.append(&mut right);
    out
}

fn is_bullet(c: char) -> bool {
    matches!(c, '•' | '▪' | '●' | '◦' | '○' | '■' | '□' | '➢' | '✓' | '◆' | '❖' | '·' | '–' | '—' | '-' | '*' | '‣' | '⁃')
}

/// Detecta el marcador inicial de un item de lista: devuelve (marcador, es_ordenada, resto).
pub fn list_marker(text: &str) -> Option<(String, bool, String)> {
    let mut chars = text.chars();
    let first = chars.next()?;
    let rest: String = chars.collect();
    if is_bullet(first) && rest.starts_with(' ') && rest.trim().len() > 1 {
        return Some((first.to_string(), false, rest.trim().to_string()));
    }
    // "1.", "1)", "a)", "(a)", "iv."
    let (token, tail) = text.split_once(' ')?;
    let core = token.trim_start_matches('(');
    // El cierre es "." o ")", ambos de un byte: asi el corte nunca cae dentro de una letra.
    let body = core.strip_suffix('.').or_else(|| core.strip_suffix(')'))?;
    if body.is_empty() || body.len() > 4 || tail.trim().is_empty() {
        return None;
    }
    let numeric = body.bytes().all(|b| b.is_ascii_digit());
    let letter = body.len() == 1 && body.bytes().all(|b| b.is_ascii_alphabetic());
    let roman = body.len() <= 4 && body.bytes().all(|b| matches!(b, b'i' | b'v' | b'x' | b'I' | b'V' | b'X'));
    if numeric || letter || roman {
        Some((token.to_string(), true, tail.trim().to_string()))
    } else {
        None
    }
}

/// Numeracion de seccion al inicio de un titulo: "3.2.1 Alcance" -> profundidad 3.
fn section_depth(text: &str) -> Option<usize> {
    let token = text.split_whitespace().next()?;
    let t = token.trim_end_matches('.');
    if t.is_empty() || !t.split('.').all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    if text.split_whitespace().nth(1).is_none() {
        return None;
    }
    Some(t.split('.').count())
}

/// Tamano de letra mas usado en el documento (ponderado por cantidad de texto).
pub fn body_size(pages: &[Vec<TLine>]) -> f64 {
    let mut buckets: Vec<(i64, usize)> = Vec::new();
    for l in pages.iter().flatten() {
        let key = (l.size * 2.0).round() as i64;
        match buckets.iter_mut().find(|(k, _)| *k == key) {
            Some(b) => b.1 += l.text.len(),
            None => buckets.push((key, l.text.len())),
        }
    }
    buckets.iter().max_by_key(|(_, n)| *n).map_or(10.0, |(k, _)| *k as f64 / 2.0)
}

/// Agrupa en una tabla las lineas consecutivas que tienen varias celdas, alineando las celdas
/// por columnas segun su posicion horizontal.
fn table_from(lines: &[&TLine], page: u32) -> Option<Block> {
    // Columnas: intervalos horizontales ocupados por alguna celda, fusionando los que se tocan.
    let mut spans: Vec<(f64, f64)> = lines.iter().flat_map(|l| l.cells.iter().map(|c| (c.0, c.1))).collect();
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut cols: Vec<(f64, f64)> = Vec::new();
    for (a, b) in spans {
        match cols.last_mut() {
            Some(last) if a <= last.1 + 1.0 => last.1 = last.1.max(b),
            _ => cols.push((a, b)),
        }
    }
    if cols.len() < 2 {
        return None;
    }
    let rows: Vec<Vec<Cell>> = lines
        .iter()
        .map(|l| {
            let mut row: Vec<Cell> = (0..cols.len()).map(|_| Cell { colspan: 1, rowspan: 1, ..Cell::default() }).collect();
            for (a, b, text) in &l.cells {
                let mid = (a + b) / 2.0;
                let k = cols.iter().position(|(c0, c1)| mid >= *c0 - 1.0 && mid <= *c1 + 1.0).unwrap_or(0);
                if !row[k].text.is_empty() {
                    row[k].text.push(' ');
                }
                row[k].text.push_str(text);
            }
            row
        })
        .collect();
    Some(Block { kind: Kind::Table, rows, page: Some(page), ..Block::default() })
}

/// Convierte las lineas de una pagina en bloques de contenido.
pub fn blocks(lines: &[TLine], page: u32, body: f64, heading_sizes: &[f64], out: &mut Vec<Block>) {
    let flow: Vec<&TLine> = lines.iter().collect();
    // Interlineado tipico de la pagina: la distancia mas repetida entre lineas del mismo tamano.
    let mut gaps: Vec<(i64, usize)> = Vec::new();
    for w in flow.windows(2) {
        let d = w[1].y - w[0].y;
        if near(w[0].size, w[1].size) && d > 0.5 * w[0].size && d < 2.5 * w[0].size {
            let key = (d * 2.0).round() as i64;
            match gaps.iter_mut().find(|(k, _)| *k == key) {
                Some(g) => g.1 += 1,
                None => gaps.push((key, 1)),
            }
        }
    }
    let lead = gaps.iter().max_by_key(|(_, n)| *n).map_or(body * 1.2, |(k, _)| *k as f64 / 2.0);

    enum Entry<'a> {
        Para { text: String, lines: Vec<&'a TLine> },
        Table(Block),
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut i = 0usize;
    while i < flow.len() {
        let l = flow[i];
        // Tabla: dos o mas lineas seguidas con varias celdas.
        if l.cells.len() >= 2 && !l.rotated {
            let mut j = i + 1;
            while j < flow.len() && flow[j].cells.len() >= 2 && !flow[j].rotated && flow[j].y >= flow[j - 1].y && (flow[j].y - flow[j - 1].y) < 3.0 * flow[j].size.max(lead) {
                j += 1;
            }
            if j - i >= 2 {
                if let Some(t) = table_from(&flow[i..j], page) {
                    entries.push(Entry::Table(t));
                    i = j;
                    continue;
                }
            }
        }
        let start_new = match entries.last() {
            Some(Entry::Para { lines, .. }) => {
                let prev = lines[lines.len() - 1];
                let gap = l.y - prev.y;
                gap < 0.0
                    || gap > lead * 1.3 + 0.5
                    || !near(l.size, prev.size)
                    || l.bold != prev.bold
                    || l.rotated != prev.rotated
                    || l.cells.len() >= 2
                    || prev.cells.len() >= 2
                    || list_marker(&l.text).is_some()
            }
            _ => true,
        };
        if start_new {
            entries.push(Entry::Para { text: l.text.clone(), lines: vec![l] });
        } else if let Some(Entry::Para { text, lines }) = entries.last_mut() {
            // Palabra cortada con guion al final de la linea: se vuelve a unir.
            let cut = text.ends_with('-') && text.chars().rev().nth(1).map_or(false, |c| c.is_alphabetic()) && l.text.chars().next().map_or(false, |c| c.is_lowercase());
            if cut {
                text.pop();
            } else {
                text.push(' ');
            }
            text.push_str(&l.text);
            lines.push(l);
        }
        i += 1;
    }

    for e in entries {
        let (text, lines) = match e {
            Entry::Table(t) => {
                out.push(t);
                continue;
            }
            Entry::Para { text, lines } => (text, lines),
        };
        let first = lines[0];
        let size = first.size;
        let all_bold = lines.iter().all(|l| l.bold);
        let text = text.replace('\t', "  ");
        let mut b = Block { text: text.clone(), page: Some(page), size: Some((size * 10.0).round() / 10.0), bold: all_bold, ..Block::default() };

        let sized = heading_sizes.iter().position(|h| near(*h, size) || size > *h);
        let short = text.chars().count() <= 200 && lines.len() <= 3;
        let last = text.chars().last().unwrap_or(' ');
        let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
        let upper = letters.len() >= 4 && letters.iter().all(|c| !c.is_lowercase());
        let depth = section_depth(&text);
        if size >= body * 1.15 && short && sized.is_some() && !first.rotated {
            b.kind = Kind::Heading;
            b.level = (sized.unwrap_or(0) + 1).min(6) as u8;
        } else if all_bold && lines.len() == 1 && text.chars().count() <= 120 && !matches!(last, ':' | '.' | ',' | ';') && (depth.is_some() || upper) && !first.rotated {
            b.kind = Kind::Heading;
            b.level = match depth {
                Some(d) => d.min(6) as u8,
                None => (heading_sizes.len() + 1).min(6) as u8,
            };
        } else if let Some((marker, ordered, rest)) = list_marker(&text) {
            b.kind = Kind::ListItem;
            b.ordered = ordered;
            b.marker = Some(marker);
            b.text = rest;
        }
        out.push(b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str, x: f64, y: f64, w: f64) -> Item {
        Item { text: text.into(), x0: x, y0: y, x1: x + w, y1: y, size: 10.0, tail: 0.0, dx: 1.0, dy: 0.0, bold: false, mcid: None, artifact: false, art: 0, scope: 0 }
    }

    #[test]
    fn lineas_y_espacios() {
        let items = vec![
            item("mundo", 131.0, 700.0, 28.0),
            item("Hola", 100.0, 700.0, 28.0), // hueco de 3 pt: espacio
            item("Segunda", 100.0, 688.0, 40.0),
            item("linea", 140.3, 688.0, 22.0), // pegado: misma palabra
            item("Col A", 100.0, 650.0, 30.0),
            item("Col B", 300.0, 650.0, 30.0), // hueco grande: otra celda
        ];
        let (ls, w, h) = lines(items, [0.0, 0.0, 600.0, 800.0], 0);
        assert_eq!((w, h), (600.0, 800.0));
        let texts: Vec<&str> = ls.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["Hola mundo", "Segundalinea", "Col A\tCol B"]);
        assert_eq!(ls[0].y, 100.0);
        assert_eq!(ls[2].cells.len(), 2);
    }

    #[test]
    fn marcadores_de_lista() {
        assert_eq!(list_marker("• Usar casco"), Some(("•".into(), false, "Usar casco".into())));
        assert_eq!(list_marker("1. Bloquear equipo"), Some(("1.".into(), true, "Bloquear equipo".into())));
        assert_eq!(list_marker("a) Primero"), Some(("a)".into(), true, "Primero".into())));
        assert_eq!(list_marker("2024 fue un buen anio"), None);
        assert_eq!(list_marker("Texto normal"), None);
        assert_eq!(section_depth("3.2.1 Alcance"), Some(3));
        assert_eq!(section_depth("Alcance"), None);
    }
}
