//! PDF cifrados hechos por otra implementacion: comprueban la conformidad, no solo el cableado.

use std::path::PathBuf;

fn datos(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("datos").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("no se pudo leer {}: {}", path.display(), e))
}

/// AES-256 revision 5 (la de Acrobat 9, que ISO 32000-2 reemplazo por la 6) escrito por pypdf
/// con contrasena de usuario vacia y de propietario "propietario"; ver `datos/aes256-r5-pypdf.py`.
#[test]
fn aes256_revision_5_de_pypdf() {
    let pdf = datos("aes256-r5-pypdf.pdf");
    let doc = doc2json::pdf::read(&pdf).unwrap();
    let json = doc2json::doc::to_json(&doc, false);
    assert!(json.contains("\"title\":\"Titulo pypdf\""), "{json}");
    assert!(json.contains("\"text\":\"Hola pypdf R5\""), "{json}");
    assert!(json.contains("\"encrypted\":true"), "{json}");
}
