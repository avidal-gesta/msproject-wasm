//! Linea de comandos:
//! `doc2json <archivo.docx|.pdf|.pptx|.xlsx> [salida] [--full | --markdown | --fragmentos[=N]] [--max-mb=N] [--max-paginas=N] [--max-caracteres=N]`

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
    let doc = match doc2json::read_with_limits(&bytes, limits) {
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
    let pages = doc.page_count.map_or(String::new(), |n| format!("{} paginas, ", n));
    say(&format!("Leido ({}): {}{} bloques de contenido.", doc.kind, pages, doc.content.len()));
    for w in &doc.warnings {
        say(&format!("Aviso: {}", w));
    }
    say(&format!("Salida: {}", out.display()));
    ExitCode::SUCCESS
}

/// Imprime sin abortar si la salida esta cerrada.
fn say(msg: &str) {
    let _ = writeln!(std::io::stdout(), "{}", msg);
}

fn usage() -> ExitCode {
    eprintln!("Uso: doc2json <archivo.docx|.pdf|.pptx|.xlsx> [salida] [--full | --markdown | --fragmentos[=N]] [--max-mb=N] [--max-paginas=N] [--max-caracteres=N]");
    eprintln!("  --full              JSON con estilos, formato y lineas de cada pagina");
    eprintln!("  --markdown          Markdown (tablas GFM, notas al pie, marcas de pagina en PDF)");
    eprintln!("  --fragmentos=N      fragmentos de hasta N caracteres por seccion, en JSON (por defecto 2000)");
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
