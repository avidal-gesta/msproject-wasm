//! Celdas de tablas de Word con titulos dentro: el JSON completo conserva sus bloques (con el estilo),
//! y una tabla comun no los guarda.

use doc2json::doc::Kind;

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

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

/// Un .docx con ese cuerpo y los estilos «Ttulo1» (heading 1, como lo guarda Word en espanol) y «Normal».
fn docx(cuerpo: &str) -> Vec<u8> {
    let documento = format!(r#"<?xml version="1.0"?><w:document {}><w:body>{}</w:body></w:document>"#, W, cuerpo);
    let estilos = format!(
        r#"<?xml version="1.0"?><w:styles {}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Ttulo1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="0"/></w:pPr></w:style></w:styles>"#,
        W
    );
    zip(&[
        ("_rels/.rels", r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#),
        ("word/document.xml", &documento),
        ("word/styles.xml", &estilos),
        ("word/_rels/document.xml.rels", r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#),
    ])
}

fn parrafo(texto: &str, estilo: Option<&str>) -> String {
    let pr = estilo.map(|e| format!(r#"<w:pPr><w:pStyle w:val="{}"/></w:pPr>"#, e)).unwrap_or_default();
    format!(r#"<w:p>{}<w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#, pr, texto)
}

fn tabla(celdas: &[String]) -> String {
    format!("<w:tbl><w:tr>{}</w:tr></w:tbl>", celdas.iter().map(|c| format!("<w:tc>{}</w:tc>", c)).collect::<String>())
}

#[test]
fn una_celda_con_un_titulo_conserva_sus_bloques_y_su_estilo() {
    let cuerpo = tabla(&[parrafo("Codigo:", None), format!("{}{}", parrafo("Procedimiento de apoyo", Some("Ttulo1")), parrafo("cambio de cinta", None))]);
    let d = doc2json::read(&docx(&cuerpo)).unwrap();
    let t = d.content.iter().find(|b| b.kind == Kind::Table).unwrap();
    let (a, b) = (&t.rows[0][0], &t.rows[0][1]);
    // El texto de la celda no cambia; solo la que trae el titulo guarda sus bloques.
    assert_eq!(b.text, "Procedimiento de apoyo\ncambio de cinta");
    assert!(a.blocks.is_empty());
    assert_eq!(b.blocks.len(), 2);
    assert_eq!((b.blocks[0].kind, b.blocks[0].level, b.blocks[0].style.as_deref()), (Kind::Heading, 1, Some("heading 1")));
    assert_eq!(b.blocks[1].kind, Kind::Paragraph);

    // En el JSON completo salen los bloques de la celda; en el comun no.
    let completo = doc2json::to_json(&docx(&cuerpo), true).unwrap();
    assert!(completo.contains(r#""blocks":[{"type":"heading","level":1,"text":"Procedimiento de apoyo","style":"heading 1""#), "{}", completo);
    let comun = doc2json::to_json(&docx(&cuerpo), false).unwrap();
    assert!(!comun.contains("\"blocks\""), "{}", comun);
}

#[test]
fn una_tabla_comun_no_guarda_bloques() {
    let cuerpo = tabla(&[parrafo("Paso", None), parrafo("Riesgo", Some("Normal"))]);
    let d = doc2json::read(&docx(&cuerpo)).unwrap();
    let t = d.content.iter().find(|b| b.kind == Kind::Table).unwrap();
    assert!(t.rows.iter().flatten().all(|c| c.blocks.is_empty()));
    assert!(!doc2json::to_json(&docx(&cuerpo), true).unwrap().contains("\"blocks\""));
}

#[test]
fn un_titulo_en_una_tabla_anidada_tambien_llega() {
    let anidada = tabla(&[parrafo("Inspector mecanico", Some("Ttulo1"))]);
    let cuerpo = tabla(&[parrafo("Cargo:", None), anidada]);
    let d = doc2json::read(&docx(&cuerpo)).unwrap();
    let t = d.content.iter().find(|b| b.kind == Kind::Table).unwrap();
    let externa = &t.rows[0][1];
    assert_eq!(externa.blocks.len(), 1);
    let interna = &externa.blocks[0].rows[0][0];
    assert_eq!((interna.blocks[0].kind, interna.blocks[0].text.as_str()), (Kind::Heading, "Inspector mecanico"));
}
