//! Pruebas de punta a punta con el documento de `ejemplo/` (el mismo contenido en .docx y .pdf).

use doc2json::doc::{Block, Kind};
use std::path::PathBuf;

fn ejemplo(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("ejemplo").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("no se pudo leer {}: {}", path.display(), e))
}

fn count(blocks: &[Block], kind: Kind) -> usize {
    blocks.iter().filter(|b| b.kind == kind).count()
}

fn table_texts(blocks: &[Block]) -> Vec<Vec<String>> {
    let t = blocks.iter().find(|b| b.kind == Kind::Table).expect("deberia haber una tabla");
    t.rows.iter().map(|r| r.iter().map(|c| c.text.clone()).collect()).collect()
}

const TABLA: [[&str; 3]; 3] =
    [["Cargo", "Tarea", "Registro"], ["Supervisor", "Autoriza el bloqueo", "Permiso de trabajo"], ["Mantenedor", "Instala su candado", "Tarjeta personal"]];

#[test]
fn docx_de_ejemplo() {
    let doc = doc2json::read(&ejemplo("procedimiento.docx")).unwrap();
    assert_eq!(doc.kind, "docx");
    let c = &doc.content;
    assert_eq!(c[0].kind, Kind::Title);
    assert_eq!(c[0].text, "Procedimiento de bloqueo de equipos");
    assert_eq!(count(c, Kind::Heading), 5);
    assert_eq!(count(c, Kind::ListItem), 6);
    assert_eq!(count(c, Kind::Table), 1);

    // Titulos con su nivel.
    let headings: Vec<(u8, &str)> = c.iter().filter(|b| b.kind == Kind::Heading).map(|b| (b.level, b.text.as_str())).collect();
    assert_eq!(headings, [(1, "Objetivo"), (1, "Alcance"), (1, "Pasos"), (2, "Antes de intervenir"), (1, "Responsables")]);

    // Vinetas y numeracion: el numero visible no esta en el texto del .docx, se calcula.
    let items: Vec<(Option<&str>, bool)> = c.iter().filter(|b| b.kind == Kind::ListItem).map(|b| (b.marker.as_deref(), b.ordered)).collect();
    assert_eq!(items[..3], [(Some("\u{2022}"), false); 3]);
    assert_eq!(items[3..], [(Some("1."), true), (Some("2."), true), (Some("3."), true)]);

    assert_eq!(table_texts(c), TABLA);

    // La nota al pie queda citada en el texto y listada aparte.
    assert!(c.iter().any(|b| b.text.ends_with("intervenir un equipo.[1]")));
    let notes = &doc.extras.iter().find(|(k, _)| *k == "footnotes").unwrap().1;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].label.as_deref(), Some("1"));
    assert!(notes[0].text.starts_with("Aplica a energ"));

    // Tramos con formato: "Importante:" va en negrita.
    let last = c.last().unwrap();
    assert_eq!(last.text, "Importante: nadie retira un candado que no sea el propio.");
    assert!(last.runs.first().map_or(false, |r| r.bold && r.text.starts_with("Importante")));
}

/// Resumen comparable de un bloque: tipo, nivel, marcador, texto y celdas.
fn firma(b: &Block) -> String {
    let rows: Vec<String> = b.rows.iter().map(|r| r.iter().map(|c| format!("{}/{}/{}/{}", c.text, c.colspan, c.rowspan, c.merged)).collect::<Vec<_>>().join("|")).collect();
    format!("{:?} {} {:?} {:?} {} {}", b.kind, b.level, b.marker, b.label, b.text, rows.join("//"))
}

#[test]
fn pdf_etiquetado_igual_al_docx() {
    // El PDF de ejemplo lo exporto Word con etiquetas: leyendo el arbol de estructura, los bloques
    // salen iguales a los del .docx original (titulo, niveles, listas, tabla y nota al pie).
    let pdf = doc2json::read(&ejemplo("procedimiento.pdf")).unwrap();
    let docx = doc2json::read(&ejemplo("procedimiento.docx")).unwrap();
    assert!(pdf.meta.iter().any(|(k, v)| *k == "tagged" && matches!(v, doc2json::doc::Meta::Bool(true))));
    let a: Vec<String> = pdf.content.iter().map(firma).collect();
    let b: Vec<String> = docx.content.iter().map(firma).collect();
    assert_eq!(a, b);
    let notes = |d: &doc2json::Document| d.extras.iter().find(|(k, _)| *k == "footnotes").map(|(_, v)| v.iter().map(firma).collect::<Vec<_>>()).unwrap_or_default();
    assert_eq!(notes(&pdf), notes(&docx));
    assert!(pdf.content.iter().all(|b| b.page == Some(1)));
    // El texto de la pagina no cambia: sigue siendo el de la pagina completa.
    assert!(pdf.pages[0].text.contains("Supervisor\tAutoriza el bloqueo\tPermiso de trabajo"));
}

#[test]
fn pdf_de_ejemplo() {
    // Sin usar las etiquetas: la estructura se deduce por posicion (camino de los PDF sin etiquetar).
    let doc = doc2json::pdf::read_with(&ejemplo("procedimiento.pdf"), false).unwrap();
    assert_eq!(doc.kind, "pdf");
    assert_eq!(doc.page_count, Some(1));
    assert_eq!(doc.pages.len(), 1);
    assert!(doc.warnings.is_empty());
    let c = &doc.content;

    // La estructura se deduce del tamano de letra y la posicion: debe coincidir con la del .docx.
    let headings: Vec<(u8, &str)> = c.iter().filter(|b| b.kind == Kind::Heading).map(|b| (b.level, b.text.as_str())).collect();
    assert_eq!(headings, [(1, "Procedimiento de bloqueo de equipos"), (2, "Objetivo"), (2, "Alcance"), (2, "Pasos"), (3, "Antes de intervenir"), (2, "Responsables")]);
    let items: Vec<(Option<&str>, &str)> = c.iter().filter(|b| b.kind == Kind::ListItem).map(|b| (b.marker.as_deref(), b.text.as_str())).collect();
    assert_eq!(items.len(), 6);
    assert_eq!(items[0], (Some("\u{2022}"), "Personal de mantenci\u{f3}n mec\u{e1}nica"));
    assert_eq!(items[4], (Some("2."), "Abrir el interruptor y colocar candado y tarjeta."));
    assert_eq!(table_texts(c), TABLA);
    assert!(c.iter().all(|b| b.page == Some(1)));

    // Texto de la pagina: tildes correctas y celdas separadas por tabulador.
    let text = &doc.pages[0].text;
    assert!(text.contains("energ\u{ed}a el\u{e9}ctrica, hidr\u{e1}ulica y neum\u{e1}tica"));
    assert!(text.contains("Supervisor\tAutoriza el bloqueo\tPermiso de trabajo"));
}

#[test]
fn salida_json() {
    for name in ["procedimiento.docx", "procedimiento.pdf"] {
        let bytes = ejemplo(name);
        let compact = doc2json::to_json(&bytes, false).unwrap();
        let full = doc2json::to_json(&bytes, true).unwrap();
        assert!(compact.starts_with("{\"type\":\"") && compact.ends_with('}'));
        assert!(compact.contains("\"metadata\":{") && compact.contains("\"content\":[") && compact.contains("\"text\":\""));
        assert!(full.len() > compact.len());
        // Llaves y corchetes balanceados fuera de las cadenas.
        let (mut depth, mut in_str, mut esc) = (0i32, false, false);
        for ch in compact.chars() {
            if in_str {
                match (esc, ch) {
                    (true, _) => esc = false,
                    (false, '\\') => esc = true,
                    (false, '"') => in_str = false,
                    _ => {}
                }
            } else {
                match ch {
                    '"' => in_str = true,
                    '{' | '[' => depth += 1,
                    '}' | ']' => depth -= 1,
                    _ => {}
                }
            }
            assert!(depth >= 0);
        }
        assert_eq!(depth, 0);
        assert!(!in_str);
    }
}

#[test]
fn entradas_invalidas() {
    assert!(doc2json::read(b"").unwrap_err().contains("vac"));
    assert!(doc2json::read(b"hola mundo").unwrap_err().contains("no reconocido"));
    assert!(doc2json::read(&[0xD0, 0xCF, 0x11, 0xE0, 0, 0, 0, 0]).unwrap_err().contains(".doc"));
    // Archivos cortados: nunca deben provocar un panic, solo un error o una lectura parcial.
    for name in ["procedimiento.docx", "procedimiento.pdf"] {
        let bytes = ejemplo(name);
        for cut in [10usize, 100, 1000, bytes.len() / 2, bytes.len() - 20] {
            let _ = doc2json::read(&bytes[..cut.min(bytes.len())]);
        }
    }
}
