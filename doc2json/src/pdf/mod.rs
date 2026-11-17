//! Lector de PDF: extrae metadatos, marcadores y el texto de cada pagina, y lo organiza en
//! bloques de contenido (titulos, parrafos, items de lista y tablas).
//!
//! Limites: no hace OCR (una pagina escaneada no tiene texto que extraer) y no abre PDF
//! que piden contrasena para abrirse.

pub mod content;
pub mod crypt;
pub mod document;
pub mod encoding;
pub mod filters;
pub mod font;
pub mod images;
pub mod layout;
pub mod object;
pub mod structure;

use crate::doc::{Document, Image, Line, Meta, OutlineItem, Page, Place};
use crate::image::ImageSet;
use content::{FontCache, Interp};
use document::{date_string, text_string, Pdf};
use object::{Dict, Obj};
use std::collections::{HashMap, HashSet};

const IDENTITY: content::Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// Marcadores (tabla de contenidos del PDF), en orden y con su nivel.
fn outline(pdf: &Pdf, page_index: &HashMap<u32, u32>) -> Vec<OutlineItem> {
    let mut out = Vec::new();
    let root = match pdf.dict(&pdf.catalog(), "Outlines") {
        Some(r) => r,
        None => return out,
    };
    let mut visited: HashSet<u32> = HashSet::new();
    // Pila de (nodo, nivel); se visita el nodo y luego su hermano siguiente.
    let mut stack: Vec<(Obj, u8)> = Vec::new();
    if let Some(first) = root.get("First") {
        stack.push((first.clone(), 1));
    }
    while let Some((node, level)) = stack.pop() {
        if let Obj::Ref(n, _) = &node {
            if !visited.insert(*n) || out.len() >= 5000 {
                continue;
            }
        }
        let obj = pdf.resolve(&node);
        let d: &Dict = match obj.as_dict() {
            Some(d) => d,
            None => continue,
        };
        let title = pdf.get(d, "Title").as_bytes().map(text_string).unwrap_or_default();
        // Destino: directo (/Dest) o por accion "ir a" (/A << /S /GoTo /D ... >>).
        let dest = match d.get("Dest") {
            Some(x) => Some(pdf.resolve(x)),
            None => pdf.dict(d, "A").and_then(|a| a.get("D").map(|x| pdf.resolve(x))),
        };
        let page = dest.and_then(|x| match x.as_array().and_then(|a| a.first()) {
            Some(Obj::Ref(n, _)) => page_index.get(n).copied(),
            Some(Obj::Int(i)) => Some(*i as u32 + 1),
            _ => None,
        });
        if !title.is_empty() {
            out.push(OutlineItem { title, level, page });
        }
        if let Some(next) = d.get("Next") {
            stack.push((next.clone(), level));
        }
        if let Some(child) = d.get("First") {
            if level < 12 {
                stack.push((child.clone(), level + 1));
            }
        }
    }
    out
}

/// Texto de las anotaciones con apariencia propia: firmas digitales, campos de formulario
/// rellenos, sellos y cuadros de texto. No esta en el contenido de la pagina sino aparte.
fn annotations(pdf: &Pdf, page: &document::PageInfo, interp: &mut Interp) {
    let annots = match pdf.array(&page.dict, "Annots") {
        Some(a) => a,
        None => return,
    };
    for a in annots.iter().take(5000) {
        let ad = match a.as_dict() {
            Some(d) => d,
            None => continue,
        };
        let subtype = pdf.name(ad, "Subtype").unwrap_or_default();
        // Oculta (2) o no visible en pantalla (32).
        let flags = pdf.get(ad, "F").as_i64().unwrap_or(0);
        if subtype == "Link" || subtype == "Popup" || flags & (2 | 32) != 0 {
            continue;
        }
        let ap = match pdf.dict(ad, "AP") {
            Some(d) => d,
            None => continue,
        };
        // La apariencia normal puede depender del estado (casillas marcadas o no).
        let mut normal = pdf.get(&ap, "N");
        if let Obj::Dict(states) = &*normal {
            let state = pdf.name(ad, "AS").and_then(|s| states.get(&s).map(|o| pdf.resolve(o)));
            normal = match state {
                Some(o) => o,
                None => continue,
            };
        }
        let form = match normal.as_dict() {
            Some(d) if matches!(&*normal, Obj::Stream { .. }) => d.clone(),
            _ => continue,
        };
        let data = match pdf.stream_data(&normal) {
            Some(d) => d,
            None => continue,
        };
        let rect = match ad.get("Rect").and_then(|r| pdf.rect(r)) {
            Some(r) => r,
            None => continue,
        };
        let m: content::Matrix = match pdf.array(&form, "Matrix") {
            Some(v) if v.len() >= 6 => {
                let n = |i: usize| v[i].as_f64().unwrap_or(0.0);
                [n(0), n(1), n(2), n(3), n(4), n(5)]
            }
            _ => IDENTITY,
        };
        // La caja de la apariencia, transformada por su matriz, se ajusta al rectangulo de la anotacion.
        let b = form.get("BBox").and_then(|r| pdf.rect(r)).unwrap_or([0.0, 0.0, rect[2] - rect[0], rect[3] - rect[1]]);
        let corners = [(b[0], b[1]), (b[2], b[1]), (b[0], b[3]), (b[2], b[3])].map(|(x, y)| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]));
        let (x0, x1) = (corners.iter().map(|c| c.0).fold(f64::MAX, f64::min), corners.iter().map(|c| c.0).fold(f64::MIN, f64::max));
        let (y0, y1) = (corners.iter().map(|c| c.1).fold(f64::MAX, f64::min), corners.iter().map(|c| c.1).fold(f64::MIN, f64::max));
        let sx = if x1 - x0 > 1e-6 { (rect[2] - rect[0]) / (x1 - x0) } else { 1.0 };
        let sy = if y1 - y0 > 1e-6 { (rect[3] - rect[1]) / (y1 - y0) } else { 1.0 };
        let fit: content::Matrix = [sx, 0.0, 0.0, sy, rect[0] - x0 * sx, rect[1] - y0 * sy];
        let res = pdf.dict(&form, "Resources").unwrap_or_else(|| page.resources.clone());
        interp.run(&data, &res, content::mul(m, fit));
    }
}

fn visible_chars(items: &[content::Item]) -> usize {
    items.iter().map(|it| it.text.chars().filter(|c| !c.is_whitespace()).count()).sum()
}

/// Bloques desde el arbol de estructura, con el texto que no quedo en ningun elemento agregado
/// (por posicion) despues de los bloques de su pagina. `None` si la estructura no sirve.
fn tagged_blocks(tree: &structure::Node, stores: &[structure::PageStore], unmarked: &[Vec<content::Item>], body: f64, sizes: &[f64]) -> Option<(Vec<crate::doc::Block>, Vec<crate::doc::Block>)> {
    let mut b = structure::Builder { pages: stores, used: HashSet::new(), notes: Vec::new() };
    let mut blocks = Vec::new();
    b.walk(tree, &mut blocks);
    let total: usize = stores.iter().flat_map(|s| s.by_mcid.values()).map(|v| visible_chars(v)).sum::<usize>() + unmarked.iter().map(|v| visible_chars(v)).sum::<usize>();
    let covered: usize = b.used.iter().filter_map(|(p, m)| stores.get(*p).and_then(|s| s.by_mcid.get(m))).map(|v| visible_chars(v)).sum();
    if blocks.is_empty() || covered * 2 < total {
        return None;
    }
    for (i, store) in stores.iter().enumerate() {
        let mut items: Vec<content::Item> = unmarked[i].clone();
        let mut left: Vec<&i64> = store.by_mcid.keys().filter(|m| !b.used.contains(&(i, **m))).collect();
        left.sort_unstable();
        for m in left {
            items.extend(store.by_mcid[m].iter().cloned());
        }
        if visible_chars(&items) == 0 {
            continue;
        }
        let (lines, width, _) = layout::lines(items, store.bbox, store.rotate);
        let lines = layout::reorder_columns(lines, width);
        let mut extra = Vec::new();
        let page = i as u32 + 1;
        layout::blocks(&lines, page, body, sizes, &mut extra);
        // Despues del ultimo bloque de esa pagina (o de una anterior).
        let at = blocks.iter().rposition(|x| x.page.map_or(false, |p| p <= page)).map_or(0, |k| k + 1);
        blocks.splice(at..at, extra);
    }
    Some((blocks, b.notes))
}

/// Encabezados y pies de un PDF etiquetado (artefactos de paginacion), como `headers` y `footers`
/// del lector de Word. Cada texto se informa una vez: el que se repite en varias paginas (aunque
/// cambie el numero de pagina) se toma de la primera. Sin subtipo declarado, decide la posicion.
fn running_texts(arts: &[Vec<content::Item>], stores: &[structure::PageStore]) -> (Vec<crate::doc::Block>, Vec<crate::doc::Block>) {
    use crate::doc::{Block, Kind};
    let (mut headers, mut footers): (Vec<Block>, Vec<Block>) = (Vec::new(), Vec::new());
    let mut seen: HashSet<(bool, String)> = HashSet::new();
    for (i, items) in arts.iter().enumerate() {
        let store = &stores[i];
        // (es encabezado, lineas) de cada tipo de artefacto de la pagina.
        let mut lines_of: Vec<(bool, String)> = Vec::new();
        for kind in [content::ART_ENCABEZADO, content::ART_PIE, content::ART_PAGINACION] {
            let group: Vec<content::Item> = items.iter().filter(|it| it.art == kind).cloned().collect();
            if group.is_empty() {
                continue;
            }
            let (lines, _, height) = layout::lines(group, store.bbox, store.rotate);
            for l in lines {
                let header = match kind {
                    content::ART_ENCABEZADO => true,
                    content::ART_PIE => false,
                    _ => l.y < height / 2.0,
                };
                // Espacios de relleno entre partes de un membrete: se compactan (los tabuladores quedan).
                let t = l.text.split('\t').map(|c| c.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\t");
                lines_of.push((header, t));
            }
        }
        for header in [true, false] {
            let text = lines_of.iter().filter(|(h, _)| *h == header).map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("\n");
            if text.trim().is_empty() {
                continue;
            }
            let key: String = text.chars().filter(|c| !c.is_ascii_digit() && !c.is_whitespace()).collect();
            if !seen.insert((header, key)) {
                continue;
            }
            let b = Block { kind: Kind::Paragraph, text, page: Some(i as u32 + 1), ..Block::default() };
            if header {
                headers.push(b);
            } else {
                footers.push(b);
            }
        }
    }
    (headers, footers)
}

pub fn read(bytes: &[u8]) -> Result<Document, String> {
    read_opts(bytes, true, false)
}

/// Como `read`, pero con `tags = false` ignora el etiquetado y deduce la estructura por la
/// posicion del texto (lo que se hace siempre con los PDF sin etiquetas).
pub fn read_with(bytes: &[u8], tags: bool) -> Result<Document, String> {
    read_opts(bytes, tags, false)
}

/// Lee el PDF; con `tags` usa el etiquetado si lo hay y con `extract` agrega los bytes de cada
/// imagen (JPEG, JPEG 2000, TIFF o PNG).
pub fn read_opts(bytes: &[u8], tags: bool, extract: bool) -> Result<Document, String> {
    let pdf = Pdf::open(bytes)?;
    let pages = pdf.pages();
    if pages.is_empty() {
        return Err("El PDF no tiene paginas legibles.".into());
    }

    let mut doc = Document { kind: "pdf", page_count: Some(pages.len()), ..Document::default() };
    if let Some(info) = pdf.dict(&pdf.trailer, "Info") {
        let text = |key: &str| pdf.get(&info, key).as_bytes().map(text_string);
        doc.meta_text("title", text("Title"));
        doc.meta_text("author", text("Author"));
        doc.meta_text("subject", text("Subject"));
        doc.meta_text("keywords", text("Keywords"));
        doc.meta_text("application", text("Creator"));
        doc.meta_text("producer", text("Producer"));
        doc.meta_text("created", text("CreationDate").and_then(|d| date_string(&d)));
        doc.meta_text("modified", text("ModDate").and_then(|d| date_string(&d)));
    }
    if !pdf.version.is_empty() {
        doc.meta.push(("pdf_version", Meta::Text(pdf.version.clone())));
    }
    if pdf.is_encrypted() {
        doc.meta.push(("encrypted", Meta::Bool(true)));
    }

    let fonts = FontCache::default();
    let mut page_lines: Vec<Vec<layout::TLine>> = Vec::with_capacity(pages.len());
    let (mut empty_pages, mut unmapped, mut total_chars) = (Vec::new(), 0usize, 0usize);
    // Para el PDF etiquetado: el texto de cada pagina agrupado por contenido marcado, el que no
    // esta marcado (anotaciones, contenido sin etiquetar) y los encabezados y pies (artefactos de
    // paginacion). Los demas artefactos (lineas, fondos, marcas de agua) no se guardan.
    let mut stores: Vec<structure::PageStore> = Vec::with_capacity(pages.len());
    let mut unmarked: Vec<Vec<content::Item>> = Vec::with_capacity(pages.len());
    let mut arts: Vec<Vec<content::Item>> = Vec::with_capacity(pages.len());
    // Presupuesto de la lectura: paginas y texto. Las paginas que no se leen se informan.
    let limits = crate::budget::limits();
    let mut stopped: Option<String> = None;
    let mut set = ImageSet::default();
    let mut inline_images = 0usize;
    for (i, page) in pages.iter().enumerate() {
        if i >= limits.max_pages {
            stopped = Some(format!("Se leyeron las primeras {} de {} paginas (limite de paginas).", i, pages.len()));
            break;
        }
        if total_chars > limits.max_chars {
            stopped = Some(format!("Se leyeron las primeras {} de {} paginas: se alcanzo el limite de {} caracteres de texto.", i, pages.len(), limits.max_chars));
            break;
        }
        let data = pdf.page_content(page);
        let mut interp = Interp::new(&pdf, &fonts);
        interp.run(&data, &page.resources, IDENTITY);
        interp.end_marked();
        annotations(&pdf, page, &mut interp);
        let mut text = interp.out;
        unmapped += text.unmapped;
        let mut by_mcid: HashMap<i64, Vec<content::Item>> = HashMap::new();
        let mut loose: Vec<content::Item> = Vec::new();
        for it in text.items.iter().filter(|it| !it.artifact) {
            match it.mcid {
                Some(m) => by_mcid.entry(m).or_default().push(it.clone()),
                None => loose.push(it.clone()),
            }
        }
        stores.push(structure::PageStore { bbox: page.bbox, rotate: page.rotate, by_mcid });
        unmarked.push(loose);
        arts.push(text.items.iter().filter(|it| it.artifact && it.art != content::ART_OTRO).cloned().collect());
        inline_images += text.inline_images;
        for (k, u) in std::mem::take(&mut text.uses).into_iter().enumerate() {
            let (x, y, w, h) = images::place(u.ctm, page.bbox, page.rotate);
            let place = Place { page: Some(i as u32 + 1), x: Some(x), y: Some(y), w, h };
            // Un objeto sin numero (no deberia pasar con flujos) cuenta como imagen propia.
            let key = match u.key {
                Some((n, g)) => format!("{} {}", n, g),
                None => format!("p{} {}", i + 1, k),
            };
            set.add(&key, Some(place), "", || {
                let (format, note) = images::classify(&pdf, &u.dict);
                let (width, height) = images::size(&pdf, &u.dict).map_or((None, None), |(w, h)| (Some(w), Some(h)));
                Image { format, width, height, note, ..Image::default() }
            });
        }
        let (lines, width, height) = layout::lines(text.items, page.bbox, page.rotate);
        let lines = layout::reorder_columns(lines, width);
        let page_text = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        total_chars += page_text.chars().filter(|c| !c.is_whitespace()).count();
        if page_text.trim().is_empty() {
            empty_pages.push(i + 1);
        }
        doc.pages.push(Page {
            number: i as u32 + 1,
            width: (width * 100.0).round() / 100.0,
            height: (height * 100.0).round() / 100.0,
            text: page_text,
            lines: lines.iter().map(|l| Line { text: l.text.clone(), x: l.x0, y: l.y, size: l.size, bold: l.bold }).collect(),
            images: text.images,
        });
        page_lines.push(lines);
    }

    // Tamanos de letra que cuentan como titulo: los mayores que el cuerpo, de mayor a menor.
    let body = layout::body_size(&page_lines);
    let mut sizes: Vec<f64> = Vec::new();
    for l in page_lines.iter().flatten() {
        let s = (l.size * 2.0).round() / 2.0;
        if s >= body * 1.15 && !sizes.iter().any(|x| (x - s).abs() < 0.26) {
            sizes.push(s);
        }
    }
    sizes.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    sizes.truncate(6);

    // PDF etiquetado: los bloques salen del arbol de estructura. Si no hay arbol, o cubre menos
    // de la mitad del texto (etiquetado roto o parcial), se usa la deduccion por posicion.
    let page_index0: HashMap<u32, usize> = pages.iter().enumerate().filter(|(_, p)| p.obj != 0).map(|(i, p)| (p.obj, i)).collect();
    let tagged = if tags { structure::tree(&pdf, &page_index0).and_then(|tree| tagged_blocks(&tree, &stores, &unmarked, body, &sizes)) } else { None };
    match tagged {
        Some((blocks, notes)) => {
            doc.content = blocks;
            let (headers, footers) = running_texts(&arts, &stores);
            for (key, list) in [("headers", headers), ("footers", footers), ("footnotes", notes)] {
                if !list.is_empty() {
                    doc.extras.push((key, list));
                }
            }
            doc.meta.push(("tagged", Meta::Bool(true)));
        }
        None => {
            for (i, lines) in page_lines.iter().enumerate() {
                layout::blocks(lines, i as u32 + 1, body, &sizes, &mut doc.content);
            }
        }
    }

    doc.images = set.list;
    if extract {
        for (img, key) in doc.images.iter_mut().zip(&set.keys) {
            let obj = key.split_once(' ').and_then(|(n, _)| n.parse::<u32>().ok()).map(|n| pdf.object(n));
            match obj.map(|o| images::extract(&pdf, &o)) {
                Some(Ok((format, data, note))) => {
                    img.format = format;
                    img.note = note;
                    img.data = Some(data);
                }
                Some(Err(note)) => img.note = Some(note),
                None => img.note = Some("No se pudo ubicar el objeto de la imagen.".into()),
            }
        }
        if inline_images > 0 {
            doc.warnings.push(format!("{} imagenes incrustadas en el contenido de las paginas (BI ... EI) no se extraen.", inline_images));
        }
    }

    let page_index: HashMap<u32, u32> = pages.iter().enumerate().filter(|(_, p)| p.obj != 0).map(|(i, p)| (p.obj, i as u32 + 1)).collect();
    doc.outline = outline(&pdf, &page_index);

    if pdf.rebuilt {
        doc.warnings.push("El indice interno del PDF estaba danado; se reconstruyo recorriendo el archivo.".into());
    }
    if let Some(w) = stopped {
        doc.warnings.push(w);
    }
    if !doc.pages.is_empty() && empty_pages.len() == doc.pages.len() {
        doc.warnings.push("Ninguna pagina tiene texto: el PDF parece ser una imagen escaneada. Se necesita OCR para leerlo.".into());
    } else if !empty_pages.is_empty() {
        let list: Vec<String> = empty_pages.iter().take(20).map(|p| p.to_string()).collect();
        doc.warnings.push(format!("Paginas sin texto (posible imagen escaneada): {}{}", list.join(", "), if empty_pages.len() > 20 { "..." } else { "" }));
    }
    if unmapped > 0 && unmapped * 5 > total_chars.max(1) {
        doc.warnings.push("Parte del texto usa fuentes sin tabla de caracteres; esos tramos aparecen como \u{FFFD}.".into());
    }
    Ok(doc)
}
