//! doc2json: convierte documentos Word (.docx) y PDF a JSON.
//!
//! - Sin dependencias externas: descompresor, lector ZIP, lector XML y lector PDF propios.
//! - Funciona igual como binario nativo y como WebAssembly (navegador o funcion serverless).
//! - Los dos formatos salen con la misma forma: metadatos, bloques de contenido y texto plano.
//!
//! ```no_run
//! let bytes = std::fs::read("informe.pdf").unwrap();
//! let json = doc2json::to_json(&bytes, false).unwrap();
//! std::fs::write("informe.json", json).unwrap();
//! ```

pub mod budget;
pub mod doc;
pub mod docx;
pub mod inflate;
pub mod json;
pub mod markdown;
pub mod pdf;
pub mod symbols;
#[cfg(target_arch = "wasm32")]
mod wasm;
pub mod xml;
pub mod zipread;

pub use doc::Document;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Docx,
    Pdf,
}

/// Reconoce el formato por el contenido, no por la extension.
pub fn detect(bytes: &[u8]) -> Result<Format, String> {
    let head = &bytes[..bytes.len().min(1024)];
    if head.windows(5).any(|w| w == b"%PDF-") {
        return Ok(Format::Pdf);
    }
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        return Ok(Format::Docx);
    }
    if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        return Err("Es un documento de Word antiguo (.doc). Abralo en Word y guardelo como .docx.".into());
    }
    if bytes.is_empty() {
        return Err("El archivo esta vacio.".into());
    }
    Err("Formato no reconocido: se esperaba un .docx o un .pdf.".into())
}

/// Lee un documento Word o PDF con los limites de recursos por defecto (`budget::Limits`).
pub fn read(bytes: &[u8]) -> Result<Document, String> {
    read_with_limits(bytes, budget::Limits::default())
}

/// Lee un documento Word o PDF sin pasar los limites dados: datos descomprimidos en total,
/// paginas y caracteres de texto. Al alcanzar uno, entrega lo leido hasta ahi con un aviso en
/// `warnings`; no falla.
pub fn read_with_limits(bytes: &[u8], limits: budget::Limits) -> Result<Document, String> {
    let format = detect(bytes)?;
    budget::begin(limits);
    let result = match format {
        Format::Docx => docx::read(bytes),
        Format::Pdf => pdf::read(bytes),
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
