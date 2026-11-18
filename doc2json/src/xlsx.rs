//! Lector de planillas Excel (.xlsx, formato SpreadsheetML).
//!
//! Cada hoja visible sale como un `heading` con su nombre seguido de una `table` con los valores
//! que Excel muestra: textos compartidos y en linea, numeros sin ruido de coma flotante, fechas y
//! horas en ISO 8601 (respetando el sistema de fechas 1904), porcentajes con su signo, verdadero
//! o falso y errores tal cual. De las formulas se toma el ultimo valor calculado que guardo Excel.
//!
//! Las filas y columnas vacias se quitan (tambien las intermedias), igual que las celdas vacias
//! al final de cada fila, y las celdas combinadas pasan a `colspan`/`rowspan` como en Word.
//! `page` es el numero de la hoja. Las hojas ocultas, los graficos y las imagenes no se leen.
//!
//! Una hoja muy dispersa (pocas celdas en un rango grande, como una diagonal) no se arma como
//! tabla, que tendria filas por columnas celdas casi todas vacias: sale como tabla de dos columnas,
//! referencia y valor ("B2", "123"), con un aviso. Los topes de celdas leidas y de celdas de
//! tabla valen para todo el libro, no por hoja, y una hoja referida varias veces se lee una vez.
//!
//! Los nombres de las etiquetas se comparan sin prefijo: algunos programas escriben `<x:row>`.

use crate::doc::*;
use crate::docx::{extract_images, image_key, part_rels, properties, relationships, resolve};
use crate::image::{format_from_name, ImageSet};
use crate::xml::{Ev, Xml};
use crate::zipread::ZipArchive;
use std::collections::HashMap;

/// Tope de celdas con valor que se leen en todo el libro; el resto se omite con un aviso.
const MAX_CELLS: usize = 2_000_000;
/// Tope de celdas (filas por columnas, ya sin las vacias) de las tablas de todo el libro: las
/// celdas vacias intermedias tambien ocupan memoria y salida. Pasado el tope se cortan filas. Las
/// hojas reales mas anchas vistas (formulas copiadas hasta la ultima columna) llegan a 3,7 millones.
const MAX_GRID: usize = 4_000_000;
/// Una hoja cuya tabla tendria mas de `SPARSE_RATIO` celdas por cada valor (y mas de
/// `SPARSE_MIN_GRID` en total) sale como lista de referencia y valor.
const SPARSE_RATIO: usize = 16;
const SPARSE_MIN_GRID: usize = 100_000;
/// Trabajo maximo (celdas recorridas) al ubicar las combinaciones, por cada celda de la tabla.
const MERGE_WORK: usize = 4;
/// Tope de hojas distintas que se leen.
const MAX_SHEETS: usize = 4096;
/// Ultima fila y cantidad de columnas de Excel (XFD1048576).
const MAX_ROW: usize = 1_048_576;
const MAX_COL: usize = 16_384;

/// Nombre de la etiqueta sin su prefijo de espacio de nombres.
fn local(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, l)| l)
}

/// Como se muestra un numero segun su formato.
#[derive(Clone, Copy, PartialEq, Debug)]
enum NumKind {
    Plain,
    Date,
    /// Tiempo transcurrido (`[h]:mm`): horas totales, aunque pasen de 24.
    Elapsed,
    Percent,
}

/// Formatos de numero integrados de Excel que son fechas, horas o porcentajes.
fn builtin_kind(id: u32) -> NumKind {
    match id {
        9 | 10 => NumKind::Percent,
        46 => NumKind::Elapsed,
        14..=22 | 27..=36 | 45 | 47 | 50..=58 => NumKind::Date,
        _ => NumKind::Plain,
    }
}

/// Clasifica un codigo de formato propio: fecha si usa y, m, d, h o s fuera de comillas,
/// corchetes y escapes; porcentaje si usa %.
fn code_kind(code: &str) -> NumKind {
    // Solo la primera seccion (la de los positivos) decide.
    let mut plain = String::new();
    let mut elapsed = false;
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                for d in chars.by_ref() {
                    if d == '"' {
                        break;
                    }
                }
            }
            '[' => {
                let mut inner = String::new();
                for d in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                    inner.push(d);
                }
                // [h], [mm], [ss]: tiempo transcurrido.
                if !inner.is_empty() && inner.chars().all(|d| matches!(d.to_ascii_lowercase(), 'h' | 'm' | 's')) {
                    plain.push('h');
                    elapsed = true;
                }
            }
            '\\' | '_' | '*' => {
                chars.next();
            }
            ';' => break,
            c => plain.push(c.to_ascii_lowercase()),
        }
    }
    if plain.contains('%') {
        NumKind::Percent
    } else if elapsed && !plain.contains('y') && !plain.contains('d') {
        NumKind::Elapsed
    } else if plain == "general" || plain.is_empty() {
        NumKind::Plain
    } else if plain.chars().any(|c| matches!(c, 'y' | 'm' | 'd' | 'h' | 's' | 'e' | 'g' | 'b')) && !plain.contains('#') && !plain.contains('0') {
        // e/g/b: anos de calendarios orientales; un formato con # o 0 es numerico ("0.00E+00").
        NumKind::Date
    } else if plain.chars().any(|c| matches!(c, 'y' | 'd' | 'h' | 's')) {
        // Horas con decimales de segundo ("hh:mm:ss.000").
        NumKind::Date
    } else {
        NumKind::Plain
    }
}

/// Numero sin ruido de coma flotante: hasta 15 cifras significativas (las que Excel muestra),
/// sin ceros de sobra; notacion cientifica solo para valores muy grandes o muy chicos.
pub(crate) fn clean_number(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    // Cifras y exponente de la representacion mas corta: "3.0000000000000004e-1".
    let s = format!("{:e}", v.abs());
    let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let mut exp: i32 = exp.parse().unwrap_or(0);
    let mut digits: Vec<u8> = mant.bytes().filter(u8::is_ascii_digit).collect();
    if digits.len() > 15 {
        let up = digits[15] >= b'5';
        digits.truncate(15);
        if up {
            let mut i = digits.len();
            loop {
                if i == 0 {
                    digits.insert(0, b'1');
                    exp += 1;
                    break;
                }
                i -= 1;
                if digits[i] == b'9' {
                    digits[i] = b'0';
                } else {
                    digits[i] += 1;
                    break;
                }
            }
        }
    }
    while digits.len() > 1 && digits.last() == Some(&b'0') {
        digits.pop();
    }
    let ds = String::from_utf8(digits).unwrap_or_default();
    let n = ds.len() as i32;
    let body = if exp >= n - 1 && exp < 21 {
        format!("{}{}", ds, "0".repeat((exp - n + 1) as usize))
    } else if (0..n - 1).contains(&exp) {
        format!("{}.{}", &ds[..exp as usize + 1], &ds[exp as usize + 1..])
    } else if (-7..0).contains(&exp) {
        format!("0.{}{}", "0".repeat((-exp - 1) as usize), ds)
    } else if n > 1 {
        format!("{}.{}E{}", &ds[..1], &ds[1..], exp)
    } else {
        format!("{}E{}", ds, exp)
    };
    if v < 0.0 && body.bytes().any(|b| (b'1'..=b'9').contains(&b)) {
        format!("-{}", body)
    } else {
        body
    }
}

/// Dias desde 1970-01-01 a fecha civil (algoritmo de Howard Hinnant).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Numero de serie de Excel a fecha u hora ISO 8601.
pub(crate) fn excel_date(serial: f64, date1904: bool) -> String {
    if !serial.is_finite() || !(0.0..=2_958_466.0).contains(&serial) {
        return clean_number(serial);
    }
    let mut seconds = (serial * 86_400.0).round() as i64;
    let mut days = seconds.div_euclid(86_400);
    seconds = seconds.rem_euclid(86_400);
    let time = format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60);
    // Una hora sin dia (0 < serie < 1) es una hora pura en los dos sistemas. La serie 0 justa
    // en el sistema 1904 es una fecha (1904-01-01).
    if days == 0 && (seconds != 0 || !date1904) {
        return time;
    }
    if date1904 {
        days += 1462;
    } else if days == 60 {
        // Excel cuenta un 29-02-1900 que no existio (heredado de Lotus 1-2-3) y lo muestra asi.
        return if seconds == 0 { "1900-02-29".into() } else { format!("1900-02-29T{}", time) };
    } else if days < 60 {
        // Antes de ese dia ficticio el desfase es un dia menos.
        days += 1;
    }
    // Serie 25569 = 1970-01-01 en el sistema 1900.
    let (y, m, d) = civil(days - 25_569);
    let date = format!("{:04}-{:02}-{:02}", y, m, d);
    if seconds == 0 {
        date
    } else {
        format!("{}T{}", date, time)
    }
}

/// Tiempo transcurrido como lo muestra `[h]:mm:ss`: "26:00:00" para 1 dia y 2 horas.
fn elapsed_time(v: f64) -> String {
    if !v.is_finite() || v.abs() > 1e9 {
        return clean_number(v);
    }
    let total = (v * 86_400.0).round() as i64;
    let sign = if total < 0 { "-" } else { "" };
    let t = total.abs();
    format!("{}{}:{:02}:{:02}", sign, t / 3600, t / 60 % 60, t % 60)
}

/// Quita los escapes `_xHHHH_` con que Excel guarda caracteres de control en los textos.
fn unescape_excel(s: String) -> String {
    if !s.contains("_x") {
        return s;
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'_' && i + 7 <= b.len() && b[i + 1] == b'x' && b[i + 6] == b'_' {
            if let Some(c) = std::str::from_utf8(&b[i + 2..i + 6]).ok().and_then(|h| u32::from_str_radix(h, 16).ok()).and_then(char::from_u32) {
                // "_x005F_" escapa el propio guion bajo.
                out.push(c);
                i += 7;
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8().max(1);
    }
    out
}

/// Indice de columna (desde 0) y fila (desde 1) de una referencia "AB12" (o "$AB$12").
fn cell_ref(r: &str) -> Option<(usize, usize)> {
    let r = r.trim().replace('$', "");
    let letters = r.bytes().take_while(|b| b.is_ascii_alphabetic()).count();
    if letters == 0 || letters > 3 {
        return None;
    }
    let col = r[..letters].bytes().fold(0usize, |acc, b| acc * 26 + (b.to_ascii_uppercase() - b'A' + 1) as usize) - 1;
    let row = r[letters..].parse::<usize>().ok()?;
    // Fuera del rango de Excel (A1:XFD1048576): no es una referencia valida.
    if row == 0 || row > MAX_ROW || col >= MAX_COL {
        return None;
    }
    Some((col, row))
}

/// Letras de una columna (desde 0): 0 = A, 27 = AB.
fn col_name(mut col: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// Texto de un `si` o `is` (ya abierto, de nombre `end`): los `t`, sin la guia fonetica (`rPh`).
fn rich_text(x: &mut Xml, end: &str) -> String {
    let mut out = String::new();
    let mut in_t = false;
    while let Some(ev) = x.next() {
        match ev {
            Ev::Start(t) if local(t.name) == "rPh" && !t.empty => x.skip(t.name),
            Ev::Start(t) if local(t.name) == "t" && !t.empty => in_t = true,
            Ev::Text(s) if in_t => out.push_str(&s),
            Ev::End(n) if n == end => break,
            Ev::End(n) if local(n) == "t" => in_t = false,
            _ => {}
        }
    }
    unescape_excel(out)
}

fn shared_strings(zip: &ZipArchive, path: Option<&String>) -> Vec<String> {
    let mut out = Vec::new();
    let Some(src) = path.and_then(|p| zip.read_text(p)) else { return out };
    let mut x = Xml::new(&src);
    while let Some(ev) = x.next() {
        if let Ev::Start(t) = ev {
            if local(t.name) == "si" {
                out.push(if t.empty { String::new() } else { rich_text(&mut x, t.name) });
            }
        }
    }
    out
}

/// Tipo de numero de cada estilo de celda (`cellXfs`).
fn styles(zip: &ZipArchive, path: Option<&String>) -> Vec<NumKind> {
    let mut out = Vec::new();
    let Some(src) = path.and_then(|p| zip.read_text(p)) else { return out };
    // Formatos propios: pocos, se buscan en orden.
    let mut custom: Vec<(u32, NumKind)> = Vec::new();
    let mut x = Xml::new(&src);
    let mut in_xfs = false;
    while let Some(ev) = x.next() {
        match ev {
            Ev::Start(t) => match local(t.name) {
                "numFmt" => {
                    if let (Some(id), Some(code)) = (t.attr("numFmtId").and_then(|v| v.parse().ok()), t.attr("formatCode")) {
                        custom.push((id, code_kind(&code)));
                    }
                }
                "cellXfs" if !t.empty => in_xfs = true,
                "xf" if in_xfs => {
                    let id: u32 = t.attr("numFmtId").and_then(|v| v.parse().ok()).unwrap_or(0);
                    out.push(custom.iter().rev().find(|c| c.0 == id).map_or_else(|| builtin_kind(id), |c| c.1));
                    if !t.empty {
                        x.skip(t.name);
                    }
                }
                _ => {}
            },
            Ev::End(n) if local(n) == "cellXfs" => in_xfs = false,
            _ => {}
        }
    }
    out
}

struct Ctx<'a> {
    strings: &'a [String],
    kinds: &'a [NumKind],
    date1904: bool,
}

/// Valor que muestra una celda segun su tipo (`t`) y su estilo (`s`).
fn display(ctx: &Ctx, kind: &str, style: usize, raw: &str) -> String {
    match kind {
        "s" => raw.trim().parse::<usize>().ok().and_then(|i| ctx.strings.get(i)).cloned().unwrap_or_default(),
        "b" => (if raw.trim() == "1" { "TRUE" } else { "FALSE" }).to_string(),
        "str" | "inlineStr" | "e" | "d" => raw.to_string(),
        _ => match raw.trim().parse::<f64>() {
            Ok(v) => match ctx.kinds.get(style).copied().unwrap_or(NumKind::Plain) {
                NumKind::Date => excel_date(v, ctx.date1904),
                NumKind::Elapsed => elapsed_time(v),
                NumKind::Percent => format!("{}%", clean_number(v * 100.0)),
                NumKind::Plain => clean_number(v),
            },
            Err(_) => raw.trim().to_string(),
        },
    }
}

struct Sheet {
    /// (fila, columna, valor), en el orden del archivo.
    cells: Vec<(usize, usize, String)>,
    /// (fila inicial, columna inicial, fila final, columna final).
    merges: Vec<(usize, usize, usize, usize)>,
    truncated: bool,
}

/// Lee las celdas con valor de una hoja, hasta `max_cells`.
fn read_sheet(src: &str, ctx: &Ctx, max_cells: usize) -> Sheet {
    let mut sheet = Sheet { cells: Vec::new(), merges: Vec::new(), truncated: false };
    let mut x = Xml::new(src);
    let mut row = 0usize;
    let mut col = 0usize;
    while let Some(ev) = x.next() {
        let Ev::Start(t) = ev else { continue };
        match local(t.name) {
            "row" => {
                row = t.attr("r").and_then(|v| v.trim().parse().ok()).filter(|r| (1..=MAX_ROW).contains(r)).unwrap_or_else(|| (row + 1).min(MAX_ROW));
                col = 0;
            }
            "c" => {
                let (c, r) = t.attr("r").and_then(|v| cell_ref(&v)).unwrap_or((col.min(MAX_COL - 1), row.max(1)));
                row = r;
                col = c + 1;
                if t.empty {
                    continue;
                }
                let cell = t.name;
                let kind = t.attr("t").unwrap_or_default();
                let style = t.attr("s").and_then(|v| v.parse().ok()).unwrap_or(0usize);
                let mut raw = String::new();
                while let Some(e2) = x.next() {
                    match e2 {
                        Ev::Start(v) if !v.empty => match local(v.name) {
                            "v" => raw = x.text_until(v.name),
                            "is" => raw = rich_text(&mut x, v.name),
                            _ => x.skip(v.name),
                        },
                        Ev::End(n) if n == cell => break,
                        _ => {}
                    }
                }
                let value = display(ctx, &kind, style, &raw);
                if value.trim().is_empty() {
                    continue;
                }
                if sheet.cells.len() >= max_cells {
                    sheet.truncated = true;
                    break;
                }
                sheet.cells.push((r, c, value));
            }
            "mergeCell" => {
                if let Some((a, b)) = t.attr("ref").as_deref().and_then(|v| v.split_once(':')) {
                    if let (Some((c1, r1)), Some((c2, r2))) = (cell_ref(a), cell_ref(b)) {
                        sheet.merges.push((r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2)));
                    }
                }
            }
            _ => {}
        }
    }
    sheet
}

/// Hoja muy dispersa: una tabla de dos columnas, referencia y valor, en el orden de la hoja.
fn sparse_table(cells: Vec<(usize, usize, String)>, page: u32) -> Block {
    let cell = |text: String| Cell { text, colspan: 1, rowspan: 1, merged: false, images: Vec::new(), blocks: Vec::new() };
    let rows = cells.into_iter().map(|(r, c, v)| vec![cell(format!("{}{}", col_name(c), r)), cell(v)]).collect();
    Block { kind: Kind::Table, rows, page: Some(page), ..Block::default() }
}

/// Combinaciones en coordenadas de la tabla (`nr` filas por `nc` columnas). Devuelve `span`
/// (alto y ancho de la celda inicial de cada combinacion) y `covered` (celda tapada por una
/// combinacion: su valor es el ancho del marcador que la representa, al inicio de cada fila de
/// la combinacion, o 0 si no lleva marcador). Excel no admite combinaciones que se solapan: vale
/// la primera y las demas se descartan. El trabajo total esta acotado por el tamano de la tabla.
#[allow(clippy::type_complexity)]
fn place_merges(
    merges: &[(usize, usize, usize, usize)],
    rows: &[usize],
    cols: &[usize],
    warnings: &mut Vec<String>,
    name: &str,
) -> (HashMap<(usize, usize), (u32, u32)>, HashMap<(usize, usize), u32>) {
    let mut span: HashMap<(usize, usize), (u32, u32)> = HashMap::new();
    let mut covered: HashMap<(usize, usize), u32> = HashMap::new();
    let (nr, nc) = (rows.len(), cols.len());
    if merges.is_empty() || nr == 0 || nc == 0 {
        return (span, covered);
    }
    // Celdas ya tomadas por una combinacion, una por byte (la tabla ya esta acotada a MAX_GRID).
    let mut taken = vec![false; nr * nc];
    let mut budget = (nr * nc).saturating_mul(MERGE_WORK).saturating_add(merges.len());
    for &(r1, c1, r2, c2) in merges {
        let rs = rows.partition_point(|&r| r < r1)..rows.partition_point(|&r| r <= r2);
        let cs = cols.partition_point(|&c| c < c1)..cols.partition_point(|&c| c <= c2);
        let (h, w) = (rs.len(), cs.len());
        if h * w <= 1 {
            continue;
        }
        // Una combinacion que se solapa con otra ya ubicada se descarta.
        let mut free = true;
        'scan: for i in rs.clone() {
            for j in cs.clone() {
                if budget == 0 {
                    warnings.push(format!("Hoja «{}»: hay demasiadas combinaciones de celdas superpuestas; se omitieron las restantes.", name));
                    return (span, covered);
                }
                budget -= 1;
                if taken[i * nc + j] {
                    free = false;
                    break 'scan;
                }
            }
        }
        if !free {
            continue;
        }
        let top_left = (rs.start, cs.start);
        span.insert(top_left, (h as u32, w as u32));
        for i in rs {
            for j in cs.clone() {
                taken[i * nc + j] = true;
                if (i, j) != top_left {
                    covered.insert((i, j), if j == cs.start { w as u32 } else { 0 });
                }
            }
        }
    }
    (span, covered)
}

/// Arma la tabla de una hoja: solo filas y columnas con algun valor, con las combinaciones.
/// La memoria es proporcional a la tabla que sale, no al rango de la hoja. `grid` es lo que
/// queda del tope de celdas de tabla del libro y se descuenta lo usado.
fn sheet_table(sheet: Sheet, page: u32, warnings: &mut Vec<String>, name: &str, grid: &mut usize) -> Option<Block> {
    let Sheet { mut cells, merges, .. } = sheet;
    if cells.is_empty() {
        return None;
    }
    cells.sort_unstable_by_key(|c| (c.0, c.1));
    // Una celda repetida en el archivo: vale la primera.
    cells.dedup_by(|b, a| a.0 == b.0 && a.1 == b.1);
    let mut rows: Vec<usize> = cells.iter().map(|c| c.0).collect();
    rows.dedup();
    let mut cols: Vec<usize> = cells.iter().map(|c| c.1).collect();
    cols.sort_unstable();
    cols.dedup();
    let nc = cols.len();
    let dense = rows.len().saturating_mul(nc);
    if dense > SPARSE_MIN_GRID && dense / SPARSE_RATIO > cells.len() {
        // Pocas celdas en un rango grande: como tabla serian casi todas vacias.
        let n = cells.len().min(*grid / 2);
        if n == 0 {
            warnings.push(format!("Hoja «{}»: se omitio, porque el libro ya alcanzo el tope de {} celdas de tabla.", name, MAX_GRID));
            return None;
        }
        if n < cells.len() {
            cells.truncate(n);
            warnings.push(format!("Hoja «{}»: se leyeron solo las primeras {} celdas, por el tope de celdas de tabla del libro.", name, n));
        }
        *grid -= 2 * n;
        warnings.push(format!("Hoja «{}»: los datos estan muy dispersos; se listan como celda y valor, sin combinaciones.", name));
        return Some(sparse_table(cells, page));
    }
    let max_rows = (*grid).min(MAX_GRID) / nc;
    if max_rows == 0 {
        warnings.push(format!("Hoja «{}»: se omitio, porque el libro ya alcanzo el tope de {} celdas de tabla.", name, MAX_GRID));
        return None;
    }
    if rows.len() > max_rows {
        rows.truncate(max_rows);
        let last = rows[max_rows - 1];
        cells.retain(|c| c.0 <= last);
        warnings.push(format!("Hoja «{}»: se leyeron solo las primeras {} filas con datos ({} columnas).", name, max_rows, nc));
    }
    *grid -= rows.len() * nc;
    let (span, covered) = place_merges(&merges, &rows, &cols, warnings, name);
    let mut out = Vec::with_capacity(rows.len());
    let mut values = cells.into_iter().peekable();
    for (i, &r) in rows.iter().enumerate() {
        let mut line = Vec::with_capacity(nc);
        // Valores de esta fila, ya ordenados por columna.
        let mut next = |j: usize| match values.peek() {
            Some(v) if v.0 == r && cols.get(j) == Some(&v.1) => values.next().map(|v| v.2),
            _ => None,
        };
        for j in 0..nc {
            let text = next(j).unwrap_or_default();
            if let Some(&w) = covered.get(&(i, j)) {
                // Excel solo muestra el valor de la primera celda de una combinacion.
                if w > 0 {
                    line.push(Cell { text: String::new(), colspan: w, rowspan: 1, merged: true, images: Vec::new(), blocks: Vec::new() });
                }
                continue;
            }
            let (rowspan, colspan) = span.get(&(i, j)).copied().unwrap_or((1, 1));
            line.push(Cell { text, colspan, rowspan, merged: false, images: Vec::new(), blocks: Vec::new() });
        }
        // Las celdas vacias del final de la fila no aportan: en hojas anchas y ralas son la mayoria.
        while line.last().is_some_and(|c| c.text.is_empty() && !c.merged && c.colspan == 1 && c.rowspan == 1) {
            line.pop();
        }
        out.push(line);
    }
    Some(Block { kind: Kind::Table, rows: out, page: Some(page), ..Block::default() })
}

/// Imagenes de los dibujos de una hoja (`xl/drawings/...`), como bloques `image` citados en la
/// lista comun. Excel las ancla a celdas, no a puntos: su lugar solo dice la hoja.
fn sheet_images(zip: &ZipArchive, sheet_path: &str, number: u32, set: &mut ImageSet) -> Vec<Block> {
    let (dir, file) = sheet_path.rsplit_once('/').unwrap_or(("", sheet_path));
    let rels_path = format!("{}/_rels/{}.rels", dir, file);
    let mut out = Vec::new();
    for (_, kind, target, external) in relationships(zip, &rels_path) {
        if kind != "drawing" || external {
            continue;
        }
        let drawing = resolve(dir, &target);
        let Some(src) = zip.read_text(&drawing) else { continue };
        let rels = part_rels(zip, &drawing);
        let mut x = Xml::new(&src);
        let (mut alt, mut pic): (String, Option<(String, bool)>) = (String::new(), None);
        while let Some(ev) = x.next() {
            match ev {
                Ev::Start(t) => match local(t.name) {
                    "pic" => {
                        alt.clear();
                        pic = None;
                    }
                    "cNvPr" => alt = t.attr("descr").or_else(|| t.attr("title")).unwrap_or_default(),
                    "blip" => pic = t.attr("r:embed").or_else(|| t.attr("r:link")).and_then(|id| rels.get(&id).cloned()),
                    _ => {}
                },
                Ev::End(n) if local(n) == "pic" => {
                    if let Some((path, external)) = pic.take() {
                        let name = path.rsplit('/').next().map(str::to_string);
                        let text = alt.trim().to_string();
                        let place = Place { page: Some(number), ..Place::default() };
                        let id = set.add(&image_key(&path, external), Some(place), &text, || Image { format: format_from_name(&path), name: name.clone(), ..Image::default() });
                        out.push(Block { kind: Kind::Image, name, text, page: Some(number), image: id, ..Block::default() });
                    }
                }
                _ => {}
            }
        }
    }
    out
}

pub fn read(data: &[u8]) -> Result<Document, String> {
    read_opts(data, None)
}

/// Lee el libro; con `images` agrega los bytes de cada imagen de `Document::images`.
pub fn read_opts(data: &[u8], images: Option<usize>) -> Result<Document, String> {
    let zip = ZipArchive::open(data).map_err(|_| "El archivo .xlsx esta danado o incompleto: no se pudo abrir como ZIP.".to_string())?;
    let main = relationships(&zip, "_rels/.rels")
        .into_iter()
        .find(|(_, kind, _, _)| kind == "officeDocument")
        .map(|(_, _, target, _)| resolve("", &target))
        .filter(|p| zip.find(p).is_some())
        .or_else(|| zip.find("xl/workbook.xml").map(|e| e.name.clone()))
        .ok_or("El archivo no es una planilla de Excel (.xlsx): no contiene xl/workbook.xml.")?;
    if main.ends_with(".bin") {
        return Err("Es un libro binario de Excel (.xlsb). Abralo en Excel y guardelo como .xlsx.".into());
    }
    let dir = main.rfind('/').map_or("", |i| &main[..i]).to_string();
    let file = &main[main.rfind('/').map_or(0, |i| i + 1)..];
    let rels_path = if dir.is_empty() { format!("_rels/{}.rels", file) } else { format!("{}/_rels/{}.rels", dir, file) };
    let by_kind: HashMap<String, String> =
        relationships(&zip, &rels_path).into_iter().filter(|r| !r.3).map(|(_, kind, target, _)| (kind, resolve(&dir, &target))).collect();
    let rels = part_rels(&zip, &main);
    let src = zip.read_text(&main).ok_or("No se pudo leer el libro.")?;

    let mut date1904 = false;
    // (nombre, ruta de la hoja, oculta)
    let mut sheets: Vec<(String, Option<String>, bool)> = Vec::new();
    let mut x = Xml::new(&src);
    while let Some(ev) = x.next() {
        if let Ev::Start(t) = ev {
            match local(t.name) {
                "workbookPr" => date1904 = matches!(t.attr("date1904").as_deref(), Some("1") | Some("true")),
                "sheet" => {
                    let hidden = matches!(t.attr("state").as_deref(), Some("hidden") | Some("veryHidden"));
                    let path = t.attr("r:id").and_then(|id| rels.get(&id).map(|(target, _)| target.clone()));
                    sheets.push((t.attr("name").unwrap_or_default(), path, hidden));
                }
                _ => {}
            }
        }
    }
    let strings = shared_strings(&zip, by_kind.get("sharedStrings"));
    let kinds = styles(&zip, by_kind.get("styles"));
    let ctx = Ctx { strings: &strings, kinds: &kinds, date1904 };

    let mut doc = Document { kind: "xlsx", ..Document::default() };
    properties(&zip, &mut doc);
    let mut hidden = 0usize;
    let mut number = 0u32;
    let mut set = ImageSet::default();
    // Una misma parte referida por varias hojas se lee una vez (vale la primera referencia).
    let mut seen = std::collections::HashSet::new();
    let mut repeated = 0usize;
    let mut cells_left = MAX_CELLS;
    let mut grid_left = MAX_GRID;
    for (name, path, is_hidden) in &sheets {
        if let Some(p) = path {
            if !seen.insert(p.as_str()) {
                repeated += 1;
                continue;
            }
            if seen.len() > MAX_SHEETS {
                doc.warnings.push(format!("El libro tiene mas de {} hojas; se leyeron solo las primeras.", MAX_SHEETS));
                break;
            }
        }
        if *is_hidden {
            hidden += 1;
            continue;
        }
        number += 1;
        let Some(path) = path else { continue };
        let Some(src) = zip.read_text(path) else { continue };
        // Las hojas de graficos o de macros no tienen celdas, pero pueden tener imagenes.
        let table = if !src.contains("sheetData") {
            None
        } else if cells_left == 0 {
            doc.warnings.push(format!("Hoja «{}»: se omitio, porque el libro ya alcanzo el tope de {} celdas con datos.", name, MAX_CELLS));
            None
        } else {
            let sheet = read_sheet(&src, &ctx, cells_left);
            cells_left -= sheet.cells.len();
            if sheet.truncated {
                doc.warnings.push(format!("Hoja «{}»: se leyo solo una parte, porque el libro alcanzo el tope de {} celdas con datos.", name, MAX_CELLS));
            }
            sheet_table(sheet, number, &mut doc.warnings, name, &mut grid_left)
        };
        let pics = sheet_images(&zip, path, number, &mut set);
        if table.is_some() || !pics.is_empty() {
            doc.content.push(Block { kind: Kind::Heading, level: 1, text: name.clone(), page: Some(number), ..Block::default() });
            doc.content.extend(table);
            doc.content.extend(pics);
        }
    }
    doc.page_count = Some(number as usize);
    if hidden > 0 {
        doc.warnings.push(format!("Se omitieron {} hojas ocultas.", hidden));
    }
    doc.warnings.extend(set.warnings());
    doc.images = set.list;
    if let Some(limit) = images {
        doc.warnings.extend(extract_images(&zip, &mut doc.images, &set.keys, limit));
    }
    if repeated > 0 {
        doc.warnings.push(format!("Se omitieron {} referencias repetidas a hojas ya leidas.", repeated));
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeros_limpios() {
        assert_eq!(clean_number(3.0), "3");
        assert_eq!(clean_number(0.1 + 0.2), "0.3");
        assert_eq!(clean_number(-0.000_000_1), "-0.0000001");
        assert_eq!(clean_number(1.5e-10), "1.5E-10");
        assert_eq!(clean_number(1234.5678), "1234.5678");
        assert_eq!(clean_number(-2.675), "-2.675");
        assert_eq!(clean_number(0.999_999_999_999_999_9), "1");
        assert_eq!(clean_number(1e20), "100000000000000000000");
        assert_eq!(clean_number(1.25e22), "1.25E22");
    }

    #[test]
    fn fechas_de_excel() {
        assert_eq!(excel_date(45_000.0, false), "2023-03-15");
        assert_eq!(excel_date(45_000.5, false), "2023-03-15T12:00:00");
        assert_eq!(excel_date(0.75, false), "18:00:00");
        assert_eq!(excel_date(1.0, false), "1900-01-01");
        assert_eq!(excel_date(61.0, false), "1900-03-01");
        assert_eq!(excel_date(0.0, true), "1904-01-01");
        // El 29-02-1900 ficticio de Excel: 59, 60 y 61 son tres dias distintos.
        assert_eq!(excel_date(59.0, false), "1900-02-28");
        assert_eq!(excel_date(60.0, false), "1900-02-29");
        assert_eq!(excel_date(60.5, false), "1900-02-29T12:00:00");
        // Una hora pura sale sin fecha tambien en el sistema 1904.
        assert_eq!(excel_date(0.75, true), "18:00:00");
        assert_eq!(excel_date(1.75, true), "1904-01-02T18:00:00");
    }

    fn hoja(cells: &[(usize, usize)]) -> Sheet {
        Sheet { cells: cells.iter().map(|&(r, c)| (r, c, format!("{}-{}", r, c))).collect(), merges: Vec::new(), truncated: false }
    }

    #[test]
    fn tope_de_tabla_por_libro() {
        // Dos hojas densas de 10 x 10 con un tope de 150 celdas: la segunda se corta.
        let cuadro: Vec<(usize, usize)> = (1..=10).flat_map(|r| (0..10).map(move |c| (r, c))).collect();
        let mut avisos = Vec::new();
        let mut tope = 150;
        let a = sheet_table(hoja(&cuadro), 1, &mut avisos, "A", &mut tope).unwrap();
        assert_eq!((a.rows.len(), tope), (10, 50));
        let b = sheet_table(hoja(&cuadro), 2, &mut avisos, "B", &mut tope).unwrap();
        assert_eq!((b.rows.len(), tope), (5, 0));
        assert!(sheet_table(hoja(&cuadro), 3, &mut avisos, "C", &mut tope).is_none());
        assert_eq!(avisos.len(), 2, "{:?}", avisos);
    }

    #[test]
    fn nombres_de_columnas() {
        assert_eq!(col_name(0), "A");
        assert_eq!(col_name(25), "Z");
        assert_eq!(col_name(26), "AA");
        assert_eq!(col_name(27), "AB");
        assert_eq!(col_name(16_383), "XFD");
        assert_eq!(cell_ref("XFE1"), None);
        assert_eq!(cell_ref("A1048577"), None);
    }

    #[test]
    fn formatos() {
        assert_eq!(code_kind("dd/mm/yyyy"), NumKind::Date);
        assert_eq!(code_kind("[$-409]d-mmm-yy;@"), NumKind::Date);
        assert_eq!(code_kind("[h]:mm:ss"), NumKind::Elapsed);
        assert_eq!(code_kind("[$-F400]h:mm:ss AM/PM"), NumKind::Date);
        assert_eq!(code_kind("hh:mm:ss.000"), NumKind::Date);
        assert_eq!(code_kind("0.00%"), NumKind::Percent);
        assert_eq!(code_kind("#,##0.00 \"m3\""), NumKind::Plain);
        assert_eq!(code_kind("0.00E+00"), NumKind::Plain);
        assert_eq!(code_kind("General"), NumKind::Plain);
        assert_eq!(builtin_kind(14), NumKind::Date);
        assert_eq!(builtin_kind(46), NumKind::Elapsed);
        assert_eq!(builtin_kind(9), NumKind::Percent);
        assert_eq!(elapsed_time(1.0 + 2.0 / 24.0), "26:00:00");
        assert_eq!(unescape_excel("a_x000D_b_x005F_x0041_".into()), "a\rb_x0041_");
    }

    #[test]
    fn referencias() {
        assert_eq!(cell_ref("A1"), Some((0, 1)));
        assert_eq!(cell_ref("AB12"), Some((27, 12)));
        assert_eq!(cell_ref("$C$3"), Some((2, 3)));
        assert_eq!(cell_ref("XFD1048576"), Some((16_383, 1_048_576)));
        assert_eq!(cell_ref("12"), None);
        assert_eq!(local("x:row"), "row");
        assert_eq!(local("row"), "row");
    }
}
