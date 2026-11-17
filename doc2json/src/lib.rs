//! doc2json: convierte documentos Word (.docx), PDF, PowerPoint (.pptx) y Excel (.xlsx) a JSON.
//!
//! - Sin dependencias externas: descompresor, lector ZIP, lector XML y lector PDF propios.
//! - Funciona igual como binario nativo y como WebAssembly (navegador o funcion serverless).
//! - Todos los formatos salen con la misma forma: metadatos, bloques de contenido y texto plano.
//!
//! ```no_run
//! let bytes = std::fs::read("informe.pdf").unwrap();
//! let json = doc2json::to_json(&bytes, false).unwrap();
//! std::fs::write("informe.json", json).unwrap();
//! ```

pub mod budget;
pub mod doc;
pub mod docx;
pub mod ccitt;
pub mod image;
pub mod inflate;
pub mod jbig2;
pub mod json;
pub mod markdown;
pub mod pdf;
pub mod pptx;
pub mod png;
pub mod symbols;
#[cfg(target_arch = "wasm32")]
mod wasm;
pub mod xlsx;
pub mod xml;
pub mod zipread;

pub use doc::Document;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Docx,
    Pdf,
    Pptx,
    Xlsx,
}

/// Reconoce el formato por el contenido, no por la extension.
pub fn detect(bytes: &[u8]) -> Result<Format, String> {
    let head = &bytes[..bytes.len().min(1024)];
    if head.windows(5).any(|w| w == b"%PDF-") {
        return Ok(Format::Pdf);
    }
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        return Ok(office_kind(bytes));
    }
    if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        return Err("Es un documento de Office antiguo (.doc, .xls o .ppt) o protegido con contrasena. Abralo y guardelo sin contrasena como .docx, .xlsx o .pptx.".into());
    }
    if bytes.is_empty() {
        return Err("El archivo esta vacio.".into());
    }
    Err("Formato no reconocido: se esperaba un .docx, .pdf, .pptx o .xlsx.".into())
}

/// Tipo de paquete Office segun la parte principal que declara (o, si no la declara, segun sus
/// carpetas). Un ZIP que no es de Office se trata como Word, que dara el error adecuado.
fn office_kind(bytes: &[u8]) -> Format {
    let Ok(zip) = zipread::ZipArchive::open(bytes) else {
        // ZIP danado o incompleto: los nombres de las partes estan en claro en cada entrada, asi
        // el error puede hablar del formato correcto.
        let has = |name: &[u8]| bytes.windows(name.len()).any(|w| w == name);
        return if has(b"ppt/presentation.xml") {
            Format::Pptx
        } else if has(b"xl/workbook.xml") {
            Format::Xlsx
        } else {
            Format::Docx
        };
    };
    let main = docx::relationships(&zip, "_rels/.rels").into_iter().find(|r| r.1 == "officeDocument").map(|r| docx::resolve("", &r.2));
    let by_path = |p: &str| {
        if p.starts_with("ppt/") {
            Some(Format::Pptx)
        } else if p.starts_with("xl/") {
            Some(Format::Xlsx)
        } else if p.starts_with("word/") {
            Some(Format::Docx)
        } else {
            None
        }
    };
    if let Some(f) = main.as_deref().and_then(by_path) {
        return f;
    }
    if zip.find("ppt/presentation.xml").is_some() {
        Format::Pptx
    } else if zip.find("xl/workbook.xml").is_some() {
        Format::Xlsx
    } else {
        Format::Docx
    }
}

/// Lee un documento Word, PDF, PowerPoint o Excel con los limites de recursos por defecto
/// (`budget::Limits`).
pub fn read(bytes: &[u8]) -> Result<Document, String> {
    read_with_limits(bytes, budget::Limits::default())
}

/// Lee un documento Word, PDF, PowerPoint o Excel sin pasar los limites dados: datos descomprimidos en total,
/// paginas y caracteres de texto. Al alcanzar uno, entrega lo leido hasta ahi con un aviso en
/// `warnings`; no falla.
pub fn read_with_limits(bytes: &[u8], limits: budget::Limits) -> Result<Document, String> {
    read_with_options(bytes, limits, false)
}

/// Como `read_with_limits`; con `images` extrae ademas los bytes de cada imagen de
/// `Document::images` (en `data`, o el motivo por el que no, en `note`).
pub fn read_with_options(bytes: &[u8], limits: budget::Limits, images: bool) -> Result<Document, String> {
    let format = detect(bytes)?;
    budget::begin(limits);
    let result = match format {
        Format::Docx => docx::read_opts(bytes, images),
        Format::Pdf => pdf::read_opts(bytes, true, images),
        Format::Pptx => pptx::read_opts(bytes, images),
        Format::Xlsx => xlsx::read_opts(bytes, images),
    };
    let (_, decompressed_hit) = budget::end();
    let mut d = result?;
    if decompressed_hit {
        d.warnings.push(budget::decompressed_warning(limits.max_decompressed));
    }
    limit_text(&mut d, limits.max_chars);
    Ok(d)
}

/// Deja como maximo `max` caracteres de texto en los bloques (y en las paginas de un PDF).
fn limit_text(d: &mut Document, max: usize) {
    let mut total = 0usize;
    let mut cut = None;
    for (i, b) in d.content.iter().enumerate() {
        let n = doc::block_text(b).chars().count();
        if total + n > max {
            cut = Some((i, max - total));
            break;
        }
        total += n;
    }
    let pages_total: usize = d.pages.iter().map(|p| p.text.chars().count()).sum();
    if cut.is_none() && pages_total <= max {
        return;
    }
    if let Some((i, left)) = cut {
        // El bloque que cruza el limite se corta por dentro (el texto, o las filas de una tabla)
        // y los siguientes se descartan.
        d.content.truncate(i + 1);
        let b = &mut d.content[i];
        if b.kind == doc::Kind::Table {
            let mut used = 0usize;
            let keep = b.rows.iter().take_while(|r| {
                used += r.iter().map(|c| c.text.chars().count() + 1).sum::<usize>();
                used <= left
            });
            let n = keep.count();
            b.rows.truncate(n);
        } else {
            b.text = b.text.chars().take(left).collect();
            b.runs.clear();
        }
        if (b.kind == doc::Kind::Table && b.rows.is_empty()) || (b.kind != doc::Kind::Table && b.text.is_empty()) {
            d.content.pop();
        }
    }
    let mut left = max;
    for p in d.pages.iter_mut() {
        let n = p.text.chars().count();
        if n > left {
            p.text = p.text.chars().take(left).collect();
            p.lines.clear();
        }
        left = left.saturating_sub(n);
    }
    d.warnings.push(format!("El documento tiene mas texto que el limite de {} caracteres: se entrega solo la primera parte.", max));
}

/// Lee un documento y extrae ademas sus imagenes: cada una de `Document::images` trae sus
/// bytes en `data` (o el motivo por el que no, en `note`).
pub fn read_with_images(bytes: &[u8]) -> Result<Document, String> {
    read_with_options(bytes, budget::Limits::default(), true)
}

/// Convierte un documento a JSON. Con `full` se incluyen estilos, formato de los tramos de
/// texto y, en PDF, las lineas de cada pagina con su posicion.
pub fn to_json(bytes: &[u8], full: bool) -> Result<String, String> {
    Ok(doc::to_json(&read(bytes)?, full))
}

/// Convierte un documento a Markdown (tablas GFM, notas al pie, marcas de pagina en PDF).
pub fn to_markdown(bytes: &[u8]) -> Result<String, String> {
    Ok(markdown::to_markdown(&read(bytes)?))
}

/// Convierte un documento en fragmentos de hasta `max_chars` caracteres, en JSON, con la ruta
/// de titulos y las paginas de cada uno. `0` usa el tamano por defecto.
pub fn to_chunks_json(bytes: &[u8], max_chars: usize) -> Result<String, String> {
    let d = read(bytes)?;
    let size = if max_chars == 0 { markdown::DEFAULT_CHUNK } else { max_chars };
    Ok(markdown::chunks_json(&d, &markdown::chunks(&d, size)))
}
