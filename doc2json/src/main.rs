//! Linea de comandos:
//! `doc2json <archivo.docx|.pdf|.pptx|.xlsx> [salida] [--full | --markdown | --fragmentos[=N]] [--imagenes[=carpeta]] [--max-mb=N] [--max-paginas=N] [--max-caracteres=N]`
//!
//! Con `--imagenes` cada imagen queda como `imagen-<id>.<formato>` en la carpeta indicada (por
//! defecto, `<documento>_imagenes` junto al documento); su `id` es el de la lista `images` del JSON.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

enum Salida {
    Json,
    Markdown,
    Fragmentos(usize),
}

fn main() -> ExitCode {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut full = false;
    let mut salida = Salida::Json;
    let mut limits = doc2json::budget::Limits::default();
    let mut images: Option<Option<PathBuf>> = None;
    for a in std::env::args().skip(1) {
        // Limites de recursos: --max-mb, --max-paginas, --max-caracteres (enteros positivos).
        if let Some((flag, value)) = a.split_once('=').filter(|(f, _)| f.starts_with("--max-")) {
            let n = match value.parse::<usize>() {
                Ok(n) if n > 0 => n,
                _ => return usage(),
            };
            match flag {
                "--max-mb" => limits.max_decompressed = n.saturating_mul(1 << 20),
                "--max-paginas" => limits.max_pages = n,
                "--max-caracteres" => limits.max_chars = n,
                _ => return usage(),
            }
            continue;
        }
        match a.as_str() {
            "--full" => full = true,
            "--markdown" | "--md" => salida = Salida::Markdown,
            "--fragmentos" | "--chunks" => salida = Salida::Fragmentos(doc2json::markdown::DEFAULT_CHUNK),
            s if s.starts_with("--fragmentos=") || s.starts_with("--chunks=") => match s.split_once('=').and_then(|(_, n)| n.parse::<usize>().ok()) {
                Some(n) if n > 0 => salida = Salida::Fragmentos(n),
                _ => return usage(),
            },
            "--imagenes" => images = Some(None),
            _ if a.starts_with("--imagenes=") => images = Some(Some(PathBuf::from(&a["--imagenes=".len()..]))),
            "-h" | "--help" => return usage(),
            _ if input.is_none() => input = Some(PathBuf::from(a)),
            _ if output.is_none() => output = Some(PathBuf::from(a)),
            _ => return usage(),
        }
    }
    let input = match input {
        Some(p) => p,
        None => return usage(),
    };
    let bytes = match std::fs::read(&input) {
        Ok(b) => b,
        Err(e) => return fail(&format!("No se pudo leer {}: {}", input.display(), e)),
    };
    let doc = match doc2json::read_with_options(&bytes, limits, images.is_some()) {
        Ok(d) => d,
        Err(e) => return fail(&e),
    };
    // Sin destino explicito, la salida queda junto al documento con el mismo nombre.
    let (texto, ext) = match salida {
        Salida::Json => (doc2json::doc::to_json(&doc, full), "json"),
        Salida::Markdown => (doc2json::markdown::to_markdown(&doc), "md"),
        Salida::Fragmentos(n) => (doc2json::markdown::chunks_json(&doc, &doc2json::markdown::chunks(&doc, n)), "fragmentos.json"),
    };
    let out = output.unwrap_or_else(|| input.with_extension(ext));
    if let Err(e) = std::fs::write(&out, texto) {
        return fail(&format!("No se pudo escribir {}: {}", out.display(), e));
    }
    if let Some(dir) = images {
        let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = dir.unwrap_or_else(|| input.with_file_name(format!("{}_imagenes", stem)));
        let with_data: Vec<_> = doc.images.iter().filter(|i| i.data.is_some()).collect();
        if !with_data.is_empty() {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                return fail(&format!("No se pudo crear {}: {}", dir.display(), e));
            }
        }
        for img in &with_data {
            let path = dir.join(format!("imagen-{:03}.{}", img.id, extension(img.format)));
            if let Err(e) = std::fs::write(&path, img.data.as_deref().unwrap_or_default()) {
                return fail(&format!("No se pudo escribir {}: {}", path.display(), e));
            }
        }
        let skipped = doc.images.len() - with_data.len();
        say(&format!("Imagenes: {} extraidas en {}{}.", with_data.len(), dir.display(), if skipped > 0 { format!(", {} sin extraer (ver `note` en el JSON)", skipped) } else { String::new() }));
    }
    let pages = doc.page_count.map_or(String::new(), |n| format!("{} paginas, ", n));
    say(&format!("Leido ({}): {}{} bloques de contenido.", doc.kind, pages, doc.content.len()));
    for w in &doc.warnings {
        say(&format!("Aviso: {}", w));
    }
    say(&format!("Salida: {}", out.display()));
    ExitCode::SUCCESS
}

fn extension(format: &str) -> &str {
    match format {
        "jpeg" => "jpg",
        "tiff" => "tif",
        "desconocido" => "bin",
        f => f,
    }
}

/// Imprime sin abortar si la salida esta cerrada.
fn say(msg: &str) {
    let _ = writeln!(std::io::stdout(), "{}", msg);
}

fn usage() -> ExitCode {
    eprintln!("Uso: doc2json <archivo.docx|.pdf|.pptx|.xlsx> [salida] [--full | --markdown | --fragmentos[=N]] [--imagenes[=carpeta]] [--max-mb=N] [--max-paginas=N] [--max-caracteres=N]");
    eprintln!("  --full              JSON con estilos, formato y lineas de cada pagina");
    eprintln!("  --markdown          Markdown (tablas GFM, notas al pie, marcas de pagina en PDF)");
    eprintln!("  --fragmentos=N      fragmentos de hasta N caracteres por seccion, en JSON (por defecto 2000)");
    eprintln!("  --imagenes[=dir]    extrae las imagenes a una carpeta (por defecto <documento>_imagenes)");
    eprintln!("  --max-mb=N          datos descomprimidos en total, en MB (por defecto 128)");
    eprintln!("  --max-paginas=N     paginas de PDF que se leen (por defecto 20000)");
    eprintln!("  --max-caracteres=N  caracteres de texto (por defecto 50000000)");
    eprintln!("  Al alcanzar un limite se entrega lo leido hasta ahi, con un aviso.");
    ExitCode::from(2)
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("Error: {}", msg);
    ExitCode::FAILURE
}
