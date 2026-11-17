//! Espacios entre palabras en PDF armados en el codigo: interletra declarada y no usada, anchos de
//! fuente exagerados, lineas justificadas apretadas, letras mal ubicadas y ligaduras.

/// Arma un PDF con los objetos dados (el primero es el 1) y su tabla xref.
fn pdf(objs: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::with_capacity(objs.len());
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
    }
    let x = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend_from_slice(format!("{:010} 00000 n \n", o).as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objs.len() + 1, x).as_bytes());
    out
}

/// El texto de una pagina con el contenido dado y la fuente `/F1` descrita por `font`.
fn texto(font: &str, contenido: &str) -> String {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 612 792] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{}\nendstream", contenido.len(), contenido),
        font.to_string(),
    ];
    doc2json::read(&pdf(&objs)).unwrap().pages[0].text.clone()
}

const HELVETICA: &str = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";

#[test]
fn interletra_declarada_que_el_pdf_no_usa() {
    // `Tc` de un em, pero cada letra ubicada con `Td` justo donde termina la anterior (anchos de
    // Helvetica: S 667, E 667, L 556, R 722, espacio 278, O 778, K 667).
    let c = "BT /F1 10 Tf 10 Tc 72 700 Td (S) Tj 6.67 0 Td (E) Tj 6.67 0 Td (L) Tj 5.56 0 Td (L) Tj 5.56 0 Td (E) Tj 6.67 0 Td (R) Tj \
             7.22 0 Td ( ) Tj 2.78 0 Td (O) Tj 7.78 0 Td (K) Tj ET";
    assert_eq!(texto(HELVETICA, c), "SELLER OK");
}

#[test]
fn interletra_que_si_se_usa_no_separa_letras() {
    // Una palabra con `Tc` dentro de una sola cadena: las letras avanzan con el espaciado.
    assert_eq!(texto(HELVETICA, "BT /F1 10 Tf 3 Tc 72 700 Td (TITULO) Tj ET"), "TITULO");
}

#[test]
fn anchos_de_fuente_exagerados() {
    // La fuente declara 1500 milesimas para cada letra, pero el PDF ubica cada una 6 puntos despues
    // de la anterior: van pegadas, no montadas en palabras distintas.
    let font = "<< /Type /Font /Subtype /Type1 /BaseFont /Arial /FirstChar 32 /LastChar 90 /Widths [278 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 \
                1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500 1500] >>";
    let c = "BT /F1 10 Tf 72 700 Td (S) Tj 6 0 Td (E) Tj 6 0 Td (L) Tj 6 0 Td (L) Tj 6 0 Td (E) Tj 6 0 Td (R) Tj ET";
    assert_eq!(texto(font, c), "SELLER");
}

#[test]
fn linea_justificada_apretada() {
    // Palabras separadas solo por su posicion, a 0,146 em (menos que el espacio comun de 0,15).
    let c = "BT /F1 10 Tf 72 700 Td [(The)-146(next)-146(step)-146(after)-146(correction)]TJ ET";
    assert_eq!(texto(HELVETICA, c), "The next step after correction");
}

#[test]
fn hueco_dudoso_suelto_no_parte_una_palabra() {
    // Una letra corrida en una exportacion (un hueco, o dos, de 0,11-0,12 em) sigue siendo la misma palabra.
    assert_eq!(texto(HELVETICA, "BT /F1 10 Tf 72 700 Td [(VELOC)-120(IDAD)]TJ ET"), "VELOCIDAD");
    assert_eq!(texto(HELVETICA, "BT /F1 10 Tf 72 700 Td [(expro)-110(piacio)-112(nes)]TJ ET"), "expropiaciones");
    // Letras todas separadas por igual (interletra amplia): no hay apretadas, no se inventan espacios.
    assert_eq!(texto(HELVETICA, "BT /F1 10 Tf 72 700 Td [(c)-110(o)-110(t)-110(a)-110(s)]TJ ET"), "cotas");
}

#[test]
fn ligadura_y_su_espacio_fantasma() {
    // 0xAE es la ligadura "fi" en la codificacion estandar; tras ella el PDF dibuja un espacio
    // montado sobre el final de la ligadura, que no separa nada.
    let c = "BT /F1 10 Tf 72 700 Td [(super\\256)250( cial)]TJ ET";
    assert_eq!(texto(HELVETICA, c), "superficial");
    // Un espacio de verdad tras una ligadura si separa.
    assert_eq!(texto(HELVETICA, "BT /F1 10 Tf 72 700 Td (o\\256 ce) Tj ET"), "ofi ce");
}
