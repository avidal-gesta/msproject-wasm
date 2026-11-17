//! Markdown y fragmentos con entradas hostiles o con casos limite: celdas que dicen ocupar millones
//! de columnas, HTML y enlaces peligrosos en el texto, tildes y emojis en los cortes.

use doc2json::doc::Kind;
use std::time::Instant;

/// ZIP sin compresion (metodo 0) con varias partes de texto.
fn zip(partes: &[(&str, &str)]) -> Vec<u8> {
    let le16 = |v: usize| (v as u16).to_le_bytes();
    let le32 = |v: usize| (v as u32).to_le_bytes();
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (nombre, contenido) in partes {
        let (n, c) = (nombre.as_bytes(), contenido.as_bytes());
        let offset = out.len();
        out.extend_from_slice(b"PK\x03\x04\x14\0\0\0\0\0\0\0\0\0\0\0\0\0");
        out.extend_from_slice(&le32(c.len()));
        out.extend_from_slice(&le32(c.len()));
        out.extend_from_slice(&le16(n.len()));
        out.extend_from_slice(&le16(0));
        out.extend_from_slice(n);
        out.extend_from_slice(c);
        central.extend_from_slice(b"PK\x01\x02\x14\0\x14\0\0\0\0\0\0\0\0\0\0\0\0\0");
        central.extend_from_slice(&le32(c.len()));
        central.extend_from_slice(&le32(c.len()));
        central.extend_from_slice(&le16(n.len()));
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&le32(offset));
        central.extend_from_slice(n);
    }
    let inicio = out.len();
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&le16(partes.len()));
    out.extend_from_slice(&le16(partes.len()));
    out.extend_from_slice(&le32(central.len()));
    out.extend_from_slice(&le32(inicio));
    out.extend_from_slice(&le16(0));
    out
}

/// Un .docx minimo con ese cuerpo y, si hace falta, relaciones del documento.
fn docx(cuerpo: &str, rels: &str) -> Vec<u8> {
    let w = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
    let documento = format!(r#"<?xml version="1.0"?><w:document {}><w:body>{}</w:body></w:document>"#, w, cuerpo);
    let rels_doc = format!(r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{}</Relationships>"#, rels);
    zip(&[
        ("_rels/.rels", r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#),
        ("word/document.xml", &documento),
        ("word/_rels/document.xml.rels", &rels_doc),
    ])
}

fn parrafo(texto: &str) -> String {
    format!(r#"<w:p><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#, texto)
}

fn celda(texto: &str, span: &str) -> String {
    let pr = if span.is_empty() { String::new() } else { format!(r#"<w:tcPr><w:gridSpan w:val="{}"/></w:tcPr>"#, span) };
    format!("<w:tc>{}{}</w:tc>", pr, parrafo(texto))
}

#[test]
fn grid_span_enorme_se_acota_al_leer_y_al_armar_la_tabla() {
    let filas = [
        format!("<w:tr>{}</w:tr>", celda("Encabezado", "100000000")),
        format!("<w:tr>{}{}</w:tr>", celda("b", ""), celda("c", "4294967295")),
        format!("<w:tr>{}{}</w:tr>", celda("x", "4294967295"), celda("y", "-3")),
    ];
    let bytes = docx(&format!("<w:tbl>{}</w:tbl>", filas.concat()), "");
    assert!(bytes.len() < 2048);
    let d = doc2json::read(&bytes).unwrap();
    let t = d.content.iter().find(|b| b.kind == Kind::Table).unwrap();
    assert!(t.rows.iter().flatten().all(|c| (1..=doc2json::doc::MAX_COLSPAN).contains(&c.colspan)), "{:?}", t.rows);
    let inicio = Instant::now();
    let md = doc2json::markdown::to_markdown(&d);
    assert!(md.len() < 20_000, "{} bytes", md.len());
    for t in ["Encabezado", "| b |", "c", "x", "y"] {
        assert!(md.contains(t), "{}: {}", t, md);
    }
    let c = doc2json::markdown::chunks(&d, 300);
    assert!(c.iter().map(|x| x.text.len()).sum::<usize>() < 20_000);
    assert!(inicio.elapsed().as_secs() < 5);
}

#[test]
fn una_fila_con_miles_de_celdas_no_multiplica_las_demas() {
    // 3.000 celdas reales en una fila y 2.000 filas de una celda: sin tope serian 6 millones.
    let ancha: String = (0..3000).map(|i| celda(&format!("c{}", i), "")).collect();
    let mut filas = format!("<w:tr>{}</w:tr>", ancha);
    for i in 0..2000 {
        filas.push_str(&format!("<w:tr>{}</w:tr>", celda(&format!("f{}", i), "")));
    }
    let d = doc2json::read(&docx(&format!("<w:tbl>{}</w:tbl>", filas), "")).unwrap();
    let md = doc2json::markdown::to_markdown(&d);
    assert!(md.len() < 2_000_000, "{} bytes", md.len());
    // No se pierde texto: las celdas que no caben en el ancho se juntan en la ultima columna.
    assert!(md.contains("c0 |") && md.contains("c2999") && md.contains("f1999"));
}

#[test]
fn el_texto_no_trae_html_ni_enlaces_peligrosos() {
    let rels = r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="javascript:alert(1)" TargetMode="External"/><Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="HTTPS://ejemplo.cl/a b(c)" TargetMode="External"/><Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="httpx://malo" TargetMode="External"/>"#;
    let enlace = |id: &str, t: &str| format!(r#"<w:hyperlink r:id="{}"><w:r><w:rPr><w:b/></w:rPr><w:t>{}</w:t></w:r></w:hyperlink>"#, id, t);
    let cuerpo = [
        parrafo("&lt;img src=x onerror=alert(1)&gt; y &lt;script&gt;"),
        parrafo("[clic](javascript:alert(1)) y ![x](javascript:alert(2))"),
        parrafo("[1]: javascript:alert(3)"),
        parrafo("Entidad &amp;lt;b&amp;gt; y barra \\&lt;i&gt;"),
        format!("<w:p>{}{}{}</w:p>", enlace("rId9", "uno"), enlace("rId8", "dos"), enlace("rId7", "tres")),
        r#"<w:p><w:r><w:t>linea</w:t></w:r><w:r><w:br/><w:t># no titulo</w:t></w:r><w:r><w:br/><w:t>---</w:t></w:r></w:p>"#.to_string(),
        format!("<w:tbl><w:tr>{}{}</w:tr><w:tr>{}{}</w:tr></w:tbl>", celda("A", ""), celda("B", ""), celda("&lt;b onclick=x&gt;", ""), celda("[a](javascript:x)", "")),
    ]
    .concat();
    let md = doc2json::to_markdown(&docx(&cuerpo, rels)).unwrap();
    // Todo "<" y todo "[" del texto van escapados (con un numero impar de barras antes); los unicos
    // sin escapar son los que arma el conversor: "<br>" en las celdas y los enlaces permitidos.
    let sin_escapar = |c: char| {
        md.match_indices(c)
            .filter(|(i, _)| md[..*i].chars().rev().take_while(|x| *x == '\\').count() % 2 == 0)
            .map(|(i, _)| md[i..].chars().take(12).collect::<String>())
            .collect::<Vec<_>>()
    };
    assert!(sin_escapar('<').iter().all(|s| s.starts_with("<br>")), "{:?}\n{}", sin_escapar('<'), md);
    assert_eq!(sin_escapar('[').iter().map(|s| s.as_str()).collect::<Vec<_>>(), ["[**dos**](HT"], "{}", md);
    assert!(md.contains("\\<img src=x onerror=alert(1)> y \\<script>"), "{}", md);
    assert!(md.contains("\\[clic\\](javascript:alert(1)) y !\\[x\\](javascript:alert(2))"), "{}", md);
    assert!(md.contains("\\\\\\<i>"), "{}", md);
    assert!(md.contains("\\[1\\]: javascript:alert(3)"), "{}", md);
    // Una entidad escrita en el texto no se convierte en el caracter.
    assert!(md.contains("\\&lt;b\\&gt;"), "{}", md);
    // Los enlaces: solo http(s) y mailto, codificados; el resto queda como texto.
    assert!(md.contains("**uno**") && !md.contains("[**uno**]"), "{}", md);
    assert!(md.contains("[**dos**](HTTPS://ejemplo.cl/a%20b%28c%29)"), "{}", md);
    assert!(md.contains("**tres**") && !md.contains("httpx"), "{}", md);
    // Un salto de linea dentro del parrafo no abre un titulo ni una linea horizontal.
    assert!(md.contains("\\# no titulo") && md.contains("\\---"), "{}", md);
}

#[test]
fn el_formato_se_conserva_con_espacios_al_final() {
    let cuerpo = r#"<w:p><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">Importante: </w:t></w:r><w:r><w:t xml:space="preserve">use casco.  </w:t></w:r></w:p><w:p><w:r><w:t xml:space="preserve">  </w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t xml:space="preserve">cursiva </w:t></w:r></w:p>"#;
    let md = doc2json::to_markdown(&docx(cuerpo, "")).unwrap();
    assert_eq!(md, "**Importante:** use casco.\n\n*cursiva*\n");
}

#[test]
fn tildes_enes_y_emojis_en_los_cortes() {
    let frase = "Señalización añadida: ¿Está el área bloqueada? Sí 👷‍♀️🦺. ";
    let largo = frase.repeat(80);
    let cuerpo = [parrafo("Título"), parrafo(&largo), parrafo(&"ñ".repeat(900)), parrafo(&"🦺".repeat(700))].concat();
    let d = doc2json::read(&docx(&cuerpo, "")).unwrap();
    for max in [200, 333, 1000] {
        let c = doc2json::markdown::chunks(&d, max);
        assert!(c.iter().all(|x| x.text.chars().count() <= max), "{}: {:?}", max, c.iter().map(|x| x.text.chars().count()).collect::<Vec<_>>());
        let todo: String = c.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(todo.matches("Señalización").count(), 80);
        assert_eq!(todo.matches('ñ').count(), 80 * 2 + 900);
        assert_eq!(todo.matches('🦺').count(), 80 + 700);
    }
}

#[test]
fn archivos_cortados_no_provocan_panic() {
    let cuerpo = [parrafo("Objetivo"), format!("<w:tbl><w:tr>{}{}</w:tr></w:tbl>", celda("a", "2"), celda("b", ""))].concat();
    let bytes = docx(&cuerpo, "");
    for corte in 0..bytes.len() {
        if let Ok(d) = doc2json::read(&bytes[..corte]) {
            let _ = doc2json::markdown::to_markdown(&d);
            let _ = doc2json::markdown::chunks(&d, 200);
        }
        let _ = doc2json::to_chunks_json(&bytes[..corte], 0);
    }
}

/// Posiciones de `c` que no van escapadas (sin un numero impar de barras antes).
fn sin_escapar(md: &str, c: char) -> Vec<String> {
    md.match_indices(c)
        .filter(|(i, _)| md[..*i].chars().rev().take_while(|x| *x == '\\').count() % 2 == 0)
        .map(|(i, _)| md[i..].chars().take(8).collect())
        .collect()
}

#[test]
fn los_cortes_no_dejan_marcas_sin_escapar() {
    // Sin espacios el corte cae en cualquier caracter, tambien entre una barra y lo que escapa.
    let pegado = "&lt;img&gt;[x]".repeat(200);
    let frases = "texto # titulo 1. lista - item &gt; cita ~~~ ".repeat(40);
    // Una celda con saltos de linea: en la tabla van como "<br>", que un corte no debe partir.
    let multilinea = "<w:r><w:t>linea con &lt;b&gt;</w:t><w:br/></w:r>".repeat(60);
    let tabla = format!("<w:tbl><w:tr>{}{}</w:tr><w:tr>{}<w:tc><w:p>{}</w:p></w:tc></w:tr></w:tbl>", celda("Paso", ""), celda("Detalle", ""), celda("1", ""), multilinea);
    // Todo corte en un espacio deja al inicio "#", "-" o "1.".
    let marcas = "# a - b 1) c ".repeat(80);
    let cuerpo = [parrafo("Inicio"), parrafo(&pegado), parrafo(&frases), parrafo(&marcas), tabla].concat();
    let d = doc2json::read(&docx(&cuerpo, "")).unwrap();
    for max in 200..260 {
        for c in doc2json::markdown::chunks(&d, max) {
            assert!(c.text.chars().count() <= max, "{}: {}", max, c.text.chars().count());
            assert!(sin_escapar(&c.text, '<').iter().all(|s| s.starts_with("<br>")), "{}: {:?}\n{}", max, sin_escapar(&c.text, '<'), c.text);
            assert!(sin_escapar(&c.text, '[').is_empty(), "{}: {}", max, c.text);
            for linea in c.text.lines() {
                assert!(!linea.starts_with('#') || linea.starts_with("# Inicio"), "{}: {:?}", max, linea);
                assert!(!linea.starts_with("1)") && !linea.starts_with("- ") && !linea.starts_with("> ") && !linea.starts_with("~~~"), "{}: {:?}", max, linea);
            }
        }
    }
}

/// Palabras de un texto sin las marcas de Markdown (barras, barras invertidas, guiones de tabla).
fn palabras(s: &str) -> Vec<String> {
    let mut v: Vec<String> = s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect();
    v.sort();
    v
}

#[test]
fn una_tabla_muy_ancha_respeta_el_tamano_de_los_fragmentos() {
    // 64 columnas con texto en todas (como una planilla ancha): ni una fila sola cabe con el
    // encabezado en 2.000 caracteres. Antes salian fragmentos de mas; ahora ninguno pasa el tope
    // y no se pierde ninguna palabra, tampoco con fragmentos chicos.
    let fila = |p: &str| format!("<w:tr>{}</w:tr>", (0..64).map(|i| celda(&format!("{}{} con algo de texto en la celda", p, i), "")).collect::<String>());
    let filas: String = std::iter::once(fila("titulo")).chain((0..3).map(|r| fila(&format!("dato{}x", r)))).collect();
    let d = doc2json::read(&docx(&format!("<w:tbl>{}</w:tbl>", filas), "")).unwrap();
    let todo = palabras(&doc2json::markdown::to_markdown(&d));
    // 200 es el minimo de un fragmento (MIN_CHUNK).
    for max in [2000, 500, 200] {
        let c = doc2json::markdown::chunks(&d, max);
        assert!(c.iter().all(|x| x.text.chars().count() <= max), "max {}: {:?}", max, c.iter().map(|x| x.text.chars().count()).max());
        let juntas = palabras(&c.iter().map(|x| x.text.as_str()).collect::<Vec<_>>().join("\n"));
        for w in &todo {
            assert!(juntas.binary_search(w).is_ok(), "max {}: falta {}", max, w);
        }
    }
}
