//! Linea de comandos:
//! `doc2json <archivo.docx|archivo.pdf> [salida] [--full | --markdown | --fragmentos[=N]]`

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
    for a in std::env::args().skip(1) {
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
    let doc = match doc2json::read(&bytes) {
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
    eprintln!("Uso: doc2json <archivo.docx|archivo.pdf> [salida] [--full | --markdown | --fragmentos[=N]]");
    eprintln!("  --full           JSON con estilos, formato y lineas de cada pagina");
    eprintln!("  --markdown       Markdown (tablas GFM, notas al pie, marcas de pagina en PDF)");
    eprintln!("  --fragmentos=N   fragmentos de hasta N caracteres por seccion, en JSON (por defecto 2000)");
    ExitCode::from(2)
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("Error: {}", msg);
    ExitCode::FAILURE
}
