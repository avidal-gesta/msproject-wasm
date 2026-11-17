//! Presupuesto de recursos: archivos "bomba" armados en la prueba (sin binarios en el repositorio).
//! Un DEFLATE con Huffman fijo y copias repetidas de 258 bytes comprime ~170 veces: unos pocos
//! cientos de KB se convierten en decenas de MB al descomprimir.

use doc2json::budget::Limits;

/// Escritor de bits de DEFLATE (el primer bit va en el bit menos significativo).
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn new() -> Bits {
        Bits { out: Vec::new(), acc: 0, n: 0 }
    }
    fn put(&mut self, value: u32, count: u32) {
        self.acc |= (value as u64) << self.n;
        self.n += count;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }
    /// Los codigos de Huffman van con su bit mas significativo primero.
    fn code(&mut self, code: u32, len: u32) {
        let mut rev = 0u32;
        for i in 0..len {
            rev |= ((code >> i) & 1) << (len - 1 - i);
        }
        self.put(rev, len);
    }
    fn literal(&mut self, b: u8) {
        if b < 144 {
            self.code(0x30 + b as u32, 8);
        } else {
            self.code(0x190 + (b as u32 - 144), 9);
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// DEFLATE de `prefijo` + `relleno` repetido `veces` + `sufijo`, en un solo bloque con Huffman fijo.
fn deflate(prefijo: &[u8], relleno: u8, veces: usize, sufijo: &[u8]) -> Vec<u8> {
    let mut b = Bits::new();
    b.put(1, 1); // ultimo bloque
    b.put(1, 2); // Huffman fijo
    for &c in prefijo {
        b.literal(c);
    }
    b.literal(relleno);
    let mut left = veces.saturating_sub(1);
    while left > 0 {
        // Copia de largo 258 (simbolo 285, sin bits extra) a distancia 1 (codigo 0, sin extra).
        let len = left.min(258);
        if len < 3 {
            for _ in 0..len {
                b.literal(relleno);
            }
            break;
        }
        if len == 258 {
            b.code(0xC0 + 5, 8);
        } else {
            // Largos 3..257 con su simbolo y bits extra (tabla de RFC 1951).
            const BASE: [u16; 28] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227];
            const EXTRA: [u8; 28] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5];
            let i = BASE.iter().rposition(|&x| x as usize <= len).unwrap();
            let sym = 257 + i as u32;
            if sym < 280 {
                b.code(sym - 256, 7);
            } else {
                b.code(0xC0 + (sym - 280), 8);
            }
            b.put((len - BASE[i] as usize) as u32, EXTRA[i] as u32);
        }
        b.code(0, 5);
        left -= len;
    }
    for &c in sufijo {
        b.literal(c);
    }
    b.code(0, 7); // fin de bloque (256)
    b.finish()
}

/// ZIP con una entrada comprimida (metodo 8). El lector no verifica el CRC.
fn zip(nombre: &str, comprimido: &[u8], tamano: usize) -> Vec<u8> {
    zip_varias(&[(nombre, comprimido.to_vec(), tamano)])
}

/// ZIP con varias entradas comprimidas (metodo 8): (nombre, DEFLATE, tamano descomprimido).
fn zip_varias(entradas: &[(&str, Vec<u8>, usize)]) -> Vec<u8> {
    let le16 = |v: usize| (v as u16).to_le_bytes();
    let le32 = |v: usize| (v as u32).to_le_bytes();
    let mut z = Vec::new();
    let mut offsets = Vec::new();
    for (nombre, comprimido, tamano) in entradas {
        offsets.push(z.len());
        z.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&le32(comprimido.len()));
        z.extend_from_slice(&le32(*tamano));
        z.extend_from_slice(&le16(nombre.len()));
        z.extend_from_slice(&[0, 0]);
        z.extend_from_slice(nombre.as_bytes());
        z.extend_from_slice(comprimido);
    }
    let central = z.len();
    for ((nombre, comprimido, tamano), off) in entradas.iter().zip(offsets) {
        z.extend_from_slice(&[0x50, 0x4B, 0x01, 0x02, 20, 0, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&le32(comprimido.len()));
        z.extend_from_slice(&le32(*tamano));
        z.extend_from_slice(&le16(nombre.len()));
        // Extra, comentario, disco, atributos internos (2 c/u) y externos (4); luego el desplazamiento.
        z.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&le32(off));
        z.extend_from_slice(nombre.as_bytes());
    }
    let tam_central = z.len() - central;
    z.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06, 0, 0, 0, 0]);
    z.extend_from_slice(&le16(entradas.len()));
    z.extend_from_slice(&le16(entradas.len()));
    z.extend_from_slice(&le32(tam_central));
    z.extend_from_slice(&le32(central));
    z.extend_from_slice(&[0, 0]);
    z
}

/// Entrada de ZIP con un XML corto (comprimido, sin repeticiones).
fn parte(nombre: &'static str, xml: &str) -> (&'static str, Vec<u8>, usize) {
    let b = xml.as_bytes();
    (nombre, deflate(&b[..b.len() - 1], b[b.len() - 1], 1, b""), b.len())
}

const W: &str = "xmlns:w=\"w\"";

fn cuerpo(texto: &str) -> String {
    format!("<?xml version=\"1.0\"?><w:document {}><w:body><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:body></w:document>", W, texto)
}

/// .docx cuyo document.xml trae un parrafo de `veces` letras "a".
fn docx_bomba(veces: usize) -> Vec<u8> {
    let pre = b"<?xml version=\"1.0\"?><w:document xmlns:w=\"w\"><w:body><w:p><w:r><w:t>";
    let suf = b"</w:t></w:r></w:p><w:p><w:r><w:t>fin</w:t></w:r></w:p></w:body></w:document>";
    zip("word/document.xml", &deflate(pre, b'a', veces, suf), pre.len() + veces + suf.len())
}

/// PDF minimo: `paginas` paginas, cada una con el contenido dado (sin comprimir) y, si se pide,
/// la primera con un flujo FlateDecode extra que descomprime `bomba` bytes de espacios.
fn pdf(paginas: usize, contenido: &str, bomba: Option<usize>) -> Vec<u8> {
    let mut objs: Vec<Vec<u8>> = Vec::new();
    let kids: Vec<String> = (0..paginas).map(|i| format!("{} 0 R", 5 + i)).collect();
    objs.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), paginas).into_bytes());
    objs.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    let texto = format!("BT /F1 12 Tf 72 720 Td ({}) Tj ET", contenido);
    let mut flujo = format!("<< /Length {} >>\nstream\n", texto.len()).into_bytes();
    flujo.extend_from_slice(texto.as_bytes());
    flujo.extend_from_slice(b"\nendstream");
    objs.push(flujo);
    let bomba_obj = 5 + paginas;
    for i in 0..paginas {
        let contents = if i == 0 && bomba.is_some() { format!("[4 0 R {} 0 R]", bomba_obj) } else { "4 0 R".to_string() };
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {} >>", contents).into_bytes());
    }
    if let Some(n) = bomba {
        let mut z = vec![0x78, 0x9C]; // cabecera zlib
        z.extend(deflate(b"", b' ', n, b""));
        let mut o = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", z.len()).into_bytes();
        o.extend(z);
        o.extend_from_slice(b"\nendstream");
        objs.push(o);
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).into_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).into_bytes());
    for off in offsets {
        out.extend(format!("{:010} 00000 n \n", off).into_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objs.len() + 1, xref).into_bytes());
    out
}

fn limites(mb: usize) -> Limits {
    Limits { max_decompressed: mb << 20, ..Limits::default() }
}

#[test]
fn el_generador_deflate_es_valido() {
    let d = deflate(b"hola ", b'x', 1000, b" chao");
    let r = doc2json::inflate::inflate(&d);
    assert!(r.ok);
    assert_eq!(r.data.len(), 5 + 1000 + 5);
    assert!(r.data.starts_with(b"hola xxx") && r.data.ends_with(b"xxx chao"));
    // ~170 veces de compresion.
    let grande = deflate(b"", b'a', 1_000_000, b"");
    assert!(grande.len() < 10_000, "{}", grande.len());
}

#[test]
fn docx_dentro_del_limite_se_lee_completo() {
    let d = doc2json::read_with_limits(&docx_bomba(2_000_000), limites(16)).unwrap();
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    assert_eq!(d.content.len(), 2);
    assert_eq!(d.content[0].text.len(), 2_000_000);
    assert_eq!(d.content[1].text, "fin");
}

#[test]
fn docx_bomba_se_corta_en_el_presupuesto() {
    // 40 MB de texto en ~250 KB de archivo; con 8 MB de presupuesto se lee solo una parte.
    let archivo = docx_bomba(40_000_000);
    assert!(archivo.len() < 400_000, "{}", archivo.len());
    let d = doc2json::read_with_limits(&archivo, limites(8)).unwrap();
    assert!(d.warnings.iter().any(|w| w.contains("limite de datos descomprimidos (8 MB)")), "{:?}", d.warnings);
    let leido: usize = d.content.iter().map(|b| b.text.len()).sum();
    assert!(leido <= 8 << 20, "{}", leido);
}

#[test]
fn pdf_con_flujo_bomba_se_corta_y_sigue() {
    let archivo = pdf(3, "Hola mundo", Some(30_000_000));
    let d = doc2json::read_with_limits(&archivo, limites(4)).unwrap();
    assert!(d.warnings.iter().any(|w| w.contains("limite de datos descomprimidos (4 MB)")), "{:?}", d.warnings);
    // Las paginas se leen igual: el texto de cada una sigue ahi.
    assert_eq!(d.page_count, Some(3));
    assert!(d.pages.iter().all(|p| p.text.contains("Hola mundo")), "{:?}", d.pages.iter().map(|p| &p.text).collect::<Vec<_>>());
    // Sin limite chico, el mismo archivo se lee sin aviso.
    let d = doc2json::read(&archivo).unwrap();
    assert!(!d.warnings.iter().any(|w| w.contains("descomprimidos")), "{:?}", d.warnings);
}

#[test]
fn pdf_con_muchas_paginas_se_corta_en_el_limite() {
    let archivo = pdf(50, "Pagina", None);
    let d = doc2json::read_with_limits(&archivo, Limits { max_pages: 10, ..Limits::default() }).unwrap();
    assert_eq!(d.page_count, Some(50));
    assert_eq!(d.pages.len(), 10);
    assert!(d.warnings.iter().any(|w| w.contains("primeras 10 de 50 paginas")), "{:?}", d.warnings);
    assert!(!d.warnings.iter().any(|w| w.contains("Ninguna pagina tiene texto")));
}

#[test]
fn limite_de_texto() {
    let d = doc2json::read_with_limits(&docx_bomba(5_000), Limits { max_chars: 1_000, ..Limits::default() }).unwrap();
    let total: usize = d.content.iter().map(|b| b.text.chars().count()).sum();
    // El bloque que cruza el limite se corta por dentro: llega exactamente al limite.
    assert_eq!(total, 1_000);
    assert_eq!(d.content.len(), 1);
    assert!(d.warnings.iter().any(|w| w.contains("limite de 1000 caracteres")), "{:?}", d.warnings);
    let p = doc2json::read_with_limits(&pdf(30, "Texto de la pagina", None), Limits { max_chars: 40, ..Limits::default() }).unwrap();
    assert!(p.pages.len() < 30);
    assert!(p.warnings.iter().any(|w| w.contains("limite de 40 caracteres")), "{:?}", p.warnings);
}

#[test]
fn el_presupuesto_es_por_lectura() {
    // Una lectura que agota su presupuesto no afecta a la siguiente.
    let _ = doc2json::read_with_limits(&docx_bomba(20_000_000), limites(2)).unwrap();
    let d = doc2json::read_with_limits(&docx_bomba(1_000_000), limites(4)).unwrap();
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    assert_eq!(d.content[0].text.len(), 1_000_000);
}

// ---------------------------------------------------------------------------------------------
// Word: el presupuesto alcanzado antes del cuerpo, y el texto de notas y comentarios
// ---------------------------------------------------------------------------------------------

#[test]
fn una_bomba_en_los_estilos_no_impide_leer_el_cuerpo() {
    // styles.xml descomprime 40 MB; el cuerpo es corto. Con 8 MB se lee el cuerpo y se avisa.
    let pre = format!("<?xml version=\"1.0\"?><w:styles {}><w:style w:styleId=\"x\"><w:name w:val=\"", W);
    let suf = b"\"/></w:style></w:styles>";
    let archivo = zip_varias(&[
        ("word/styles.xml", deflate(pre.as_bytes(), b'a', 40_000_000, suf), pre.len() + 40_000_000 + suf.len()),
        // Mas de 258 bytes (el largo de una copia de DEFLATE): no cabe en lo que la bomba deja libre.
        parte("word/document.xml", &cuerpo("Hola mundo").replace("<w:document ", &format!("<w:document relleno=\"{}\" ", "x".repeat(5_000)))),
    ]);
    let d = doc2json::read_with_limits(&archivo, limites(8)).unwrap();
    assert_eq!(d.content.iter().map(|b| b.text.as_str()).collect::<Vec<_>>(), ["Hola mundo"]);
    assert!(d.warnings.iter().any(|w| w.contains("limite de datos descomprimidos (8 MB)")), "{:?}", d.warnings);
}

#[test]
fn el_presupuesto_agotado_antes_del_cuerpo_se_explica() {
    // El cuerpo esta despues de 40 MB de un atributo: con 8 MB no se llega a <w:body>.
    let pre = format!("<?xml version=\"1.0\"?><w:document {} x=\"", W);
    let suf = b"\"><w:body><w:p><w:r><w:t>Hola</w:t></w:r></w:p></w:body></w:document>";
    let archivo = zip("word/document.xml", &deflate(pre.as_bytes(), b'a', 40_000_000, suf), pre.len() + 40_000_000 + suf.len());
    let e = doc2json::read_with_limits(&archivo, limites(8)).unwrap_err();
    assert!(e.contains("limite de datos descomprimidos (8 MB)"), "{}", e);
    // Con presupuesto suficiente se lee.
    assert_eq!(doc2json::read_with_limits(&archivo, limites(64)).unwrap().content[0].text, "Hola");
}

/// .docx con un cuerpo corto y una nota al pie y un comentario largos.
fn docx_con_notas(nota: usize, comentario: usize) -> Vec<u8> {
    let notas = format!(
        "<?xml version=\"1.0\"?><w:footnotes {}><w:footnote w:id=\"1\"><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:footnote></w:footnotes>",
        W,
        "n".repeat(nota)
    );
    let comentarios = format!(
        "<?xml version=\"1.0\"?><w:comments {}><w:comment w:id=\"0\" w:author=\"Ana\"><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:comment></w:comments>",
        W,
        "c".repeat(comentario)
    );
    zip_varias(&[parte("word/document.xml", &cuerpo("Cuerpo corto")), parte("word/footnotes.xml", &notas), parte("word/comments.xml", &comentarios)])
}

fn caracteres(d: &doc2json::Document) -> usize {
    let bloques = d.content.iter().chain(d.extras.iter().flat_map(|(_, b)| b.iter()));
    bloques.map(|b| doc2json::doc::block_text(b).chars().count()).sum()
}

#[test]
fn el_limite_de_texto_cuenta_notas_y_comentarios() {
    let archivo = docx_con_notas(5_000, 5_000);
    let completo = doc2json::read(&archivo).unwrap();
    assert_eq!(caracteres(&completo), 12 + 5_000 + 5_000);
    let d = doc2json::read_with_limits(&archivo, Limits { max_chars: 1_000, ..Limits::default() }).unwrap();
    assert_eq!(caracteres(&d), 1_000);
    assert_eq!(d.content[0].text, "Cuerpo corto");
    assert!(d.warnings.iter().any(|w| w.contains("limite de 1000 caracteres")), "{:?}", d.warnings);
    // El cuerpo va primero: con 12 caracteres no queda nada para las notas.
    let d = doc2json::read_with_limits(&archivo, Limits { max_chars: 12, ..Limits::default() }).unwrap();
    assert_eq!(caracteres(&d), 12);
    assert!(d.warnings.iter().any(|w| w.contains("limite de 12 caracteres")), "{:?}", d.warnings);
}

#[test]
fn el_limite_de_texto_acota_lo_que_el_lector_de_word_junta() {
    // Sin el recorte final de `read_with_limits`: el lector mismo deja de juntar texto (y tramos)
    // al agotar el limite, asi la memoria queda acotada aunque el parrafo traiga millones.
    doc2json::budget::begin(Limits { max_chars: 1_000, ..Limits::default() });
    let d = doc2json::docx::read(&docx_bomba(3_000_000));
    doc2json::budget::end();
    let d = d.unwrap();
    let texto: usize = d.content.iter().map(|b| b.text.len() + b.runs.iter().map(|r| r.text.len()).sum::<usize>()).sum();
    assert!(texto <= 2_000, "{}", texto);
}

// ---------------------------------------------------------------------------------------------
// DEFLATE: bloque almacenado que no cabe
// ---------------------------------------------------------------------------------------------

/// DEFLATE de bloques almacenados (sin compresion) con `n` bytes `b`.
fn almacenado(b: u8, n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut left = n;
    loop {
        let len = left.min(65_535);
        left -= len;
        out.push(if left == 0 { 1 } else { 0 });
        out.extend_from_slice(&(len as u16).to_le_bytes());
        out.extend_from_slice(&(!(len as u16)).to_le_bytes());
        out.resize(out.len() + len, b);
        if left == 0 {
            return out;
        }
    }
}

#[test]
fn un_bloque_almacenado_que_no_cabe_avisa() {
    doc2json::budget::begin(Limits { max_decompressed: 10_000, ..Limits::default() });
    let r = doc2json::inflate::inflate(&almacenado(b'x', 60_000));
    let gastado = doc2json::budget::end();
    assert!(!r.ok);
    assert!(gastado.decompressed_hit, "el corte por presupuesto debe quedar anotado");
    // Se entrega lo que cabe, como en los bloques comprimidos.
    assert_eq!((r.data.len(), gastado.decompressed), (10_000, 10_000));
}

// ---------------------------------------------------------------------------------------------
// PDF: varios flujos, LZW, ASCII85, RunLength, cadenas de filtros y flujos de objetos
// ---------------------------------------------------------------------------------------------

const TEXTO: &[u8] = b"BT /F1 12 Tf 72 720 Td (Hola mundo) Tj ET ";

/// Flujo con su diccionario (`dict`: entradas extra, p. ej. "/Filter /LZWDecode").
fn flujo(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut o = format!("<< /Length {} {} >>\nstream\n", data.len(), dict).into_bytes();
    o.extend_from_slice(data);
    o.extend_from_slice(b"\nendstream");
    o
}

/// PDF con una pagina por flujo de contenido: (entradas del diccionario, datos).
fn pdf_flujos(flujos: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut objs: Vec<(usize, Vec<u8>)> = Vec::new();
    let kids: Vec<String> = (0..flujos.len()).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    objs.push((1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()));
    objs.push((2, format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), flujos.len()).into_bytes()));
    objs.push((3, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec()));
    for (i, (dict, data)) in flujos.iter().enumerate() {
        let page = format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>", 5 + 2 * i);
        objs.push((4 + 2 * i, page.into_bytes()));
        objs.push((5 + 2 * i, flujo(dict, data)));
    }
    ensamblar(&objs, true)
}

/// Une los objetos (numero, contenido). Con `xref` agrega la tabla de referencias (los numeros
/// deben ser 1..n en orden); sin ella el lector la reconstruye recorriendo el archivo.
fn ensamblar(objs: &[(usize, Vec<u8>)], xref: bool) -> Vec<u8> {
    let mut out = b"%PDF-1.5\n".to_vec();
    let mut offsets = Vec::new();
    for (n, o) in objs {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", n).into_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    if xref {
        let pos = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).into_bytes());
        for off in offsets {
            out.extend(format!("{:010} 00000 n \n", off).into_bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n", objs.len() + 1, pos).into_bytes());
    }
    out.extend_from_slice(b"%%EOF\n");
    out
}

fn zlib_bomba(n: usize) -> Vec<u8> {
    let mut z = vec![0x78, 0x9C];
    z.extend(deflate(TEXTO, b' ', n, b""));
    z
}

/// LZW de PDF (EarlyChange 1): `TEXTO` y despues una racha de espacios. La racha usa cada vez el
/// codigo que aun no existe (el anterior mas su primer byte), asi cada codigo agrega un byte mas:
/// 3.000 codigos de 12 bits dan unos 4,5 MB.
fn lzw_bomba(codigos: usize) -> Vec<u8> {
    // Ancho del codigo segun las entradas que el lector ya agrego (`next`), como en filters.rs.
    let ancho = |next: usize| if next + 1 >= 2048 { 12 } else if next + 1 >= 1024 { 11 } else if next + 1 >= 512 { 10 } else { 9 };
    let mut lista: Vec<usize> = TEXTO.iter().map(|&b| b as usize).collect();
    lista.push(b' ' as usize);
    let base = lista.len();
    for k in 0..codigos {
        lista.push(258 + base - 1 + k);
    }
    let (mut acc, mut n, mut out) = (0u64, 0u32, Vec::new());
    let mut put = |code: usize, bits: u32| {
        acc = acc << bits | code as u64;
        n += bits;
        while n >= 8 {
            out.push((acc >> (n - 8)) as u8);
            n -= 8;
        }
    };
    for (i, &c) in lista.iter().enumerate() {
        // Antes del codigo i el lector agrego i - 1 entradas a la tabla.
        let bits = if i == 0 { 9 } else { ancho(258 + i - 1) };
        assert!(258 + i < 4096, "la tabla no debe llenarse");
        put(c, bits);
    }
    put(257, ancho(258 + lista.len() - 1));
    put(0, 7);
    out
}

/// ASCII85 de `datos` (completados con `relleno` a multiplo de 4), sin terminar.
fn ascii85(datos: &[u8], relleno: u8) -> Vec<u8> {
    let mut t = datos.to_vec();
    let falta = (4 - t.len() % 4) % 4;
    t.resize(t.len() + falta, relleno);
    let mut out = Vec::new();
    for g in t.chunks(4) {
        let mut v = u32::from_be_bytes([g[0], g[1], g[2], g[3]]);
        let mut d = [0u8; 5];
        for x in d.iter_mut().rev() {
            *x = (v % 85) as u8 + b'!';
            v /= 85;
        }
        out.extend_from_slice(&d);
    }
    out
}

/// ASCII85: `TEXTO` y `z` (cuatro bytes en cero) `n` veces.
fn ascii85_bomba(n: usize) -> Vec<u8> {
    let mut out = ascii85(TEXTO, b' ');
    out.resize(out.len() + n, b'z');
    out.extend_from_slice(b"~>");
    out
}

/// RunLength: `TEXTO` literal y `n` rachas de 128 espacios (2 bytes cada una).
fn run_length_bomba(n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for c in TEXTO.chunks(128) {
        out.push((c.len() - 1) as u8);
        out.extend_from_slice(c);
    }
    for _ in 0..n {
        out.extend_from_slice(&[129, b' ']);
    }
    out.push(128);
    out
}

fn hex(d: &[u8]) -> Vec<u8> {
    let mut s: Vec<u8> = d.iter().flat_map(|b| format!("{:02X}", b).into_bytes()).collect();
    s.push(b'>');
    s
}

fn avisa(d: &doc2json::Document, mb: usize) -> bool {
    d.warnings.iter().any(|w| w.contains(&format!("limite de datos descomprimidos ({} MB)", mb)))
}

fn textos(d: &doc2json::Document) -> Vec<&str> {
    d.pages.iter().map(|p| p.text.trim()).collect()
}

#[test]
fn el_presupuesto_suma_varios_flujos() {
    // Seis paginas con un flujo de ~1 MB cada una: ninguno pasa los 3 MB, la suma si.
    let flujos: Vec<(&str, Vec<u8>)> = (0..6).map(|_| ("/Filter /FlateDecode", zlib_bomba(1_000_000))).collect();
    let archivo = pdf_flujos(&flujos);
    let d = doc2json::read_with_limits(&archivo, limites(3)).unwrap();
    assert!(avisa(&d, 3), "{:?}", d.warnings);
    // Las primeras paginas, dentro del presupuesto, se leen completas.
    assert_eq!(textos(&d)[..2], ["Hola mundo", "Hola mundo"]);
    let d = doc2json::read_with_limits(&archivo, limites(8)).unwrap();
    assert!(!avisa(&d, 8), "{:?}", d.warnings);
    assert_eq!(textos(&d), vec!["Hola mundo"; 6]);
}

#[test]
fn lzw_ascii85_runlength_y_cadenas_respetan_el_presupuesto() {
    let mut a85_rl = ascii85(&run_length_bomba(40_000), 128);
    a85_rl.extend_from_slice(b"~>");
    let casos: Vec<(&str, &str, Vec<u8>)> = vec![
        ("LZW", "/Filter /LZWDecode", lzw_bomba(3_000)),
        ("ASCII85", "/Filter /ASCII85Decode", ascii85_bomba(1_000_000)),
        ("RunLength", "/Filter /RunLengthDecode", run_length_bomba(40_000)),
        ("ASCIIHex + Flate", "/Filter [/ASCIIHexDecode /FlateDecode]", hex(&zlib_bomba(5_000_000))),
        ("ASCII85 + RunLength", "/Filter [/ASCII85Decode /RunLengthDecode]", a85_rl),
    ];
    for (nombre, dict, data) in casos {
        let archivo = pdf_flujos(&[(dict, data)]);
        // Sin limite chico, el flujo se lee entero: el codificador de la prueba es valido.
        let d = doc2json::read(&archivo).unwrap();
        assert_eq!(textos(&d), ["Hola mundo"], "{}", nombre);
        assert!(!d.warnings.iter().any(|w| w.contains("descomprimidos")), "{}: {:?}", nombre, d.warnings);
        let d = doc2json::read_with_limits(&archivo, limites(2)).unwrap();
        assert!(avisa(&d, 2), "{}: {:?}", nombre, d.warnings);
        assert_eq!(textos(&d), ["Hola mundo"], "{}", nombre);
    }
}

#[test]
fn los_flujos_de_objetos_cuentan_en_el_presupuesto() {
    // Paginas, pagina y fuente van dentro de un flujo de objetos (ObjStm) comprimido, seguidos de
    // 6 MB de relleno. Sin tabla de referencias: el lector la reconstruye y abre el ObjStm.
    let objetos = [
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ];
    let (mut cuerpo, mut indice) = (String::new(), String::new());
    for (i, o) in objetos.iter().enumerate() {
        indice.push_str(&format!("{} {} ", 2 + i, cuerpo.len()));
        cuerpo.push_str(o);
        cuerpo.push(' ');
    }
    let mut z = vec![0x78, 0x9C];
    z.extend(deflate(format!("{}{}", indice, cuerpo).as_bytes(), b' ', 6_000_000, b""));
    let stm = flujo(&format!("/Type /ObjStm /N 3 /First {} /Filter /FlateDecode", indice.len()), &z);
    let objs = vec![(1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()), (5, flujo("", TEXTO)), (6, stm)];
    let archivo = ensamblar(&objs, false);
    let d = doc2json::read(&archivo).unwrap();
    assert_eq!(textos(&d), ["Hola mundo"]);
    assert!(!avisa(&d, 4), "{:?}", d.warnings);
    let d = doc2json::read_with_limits(&archivo, limites(4)).unwrap();
    assert!(avisa(&d, 4), "{:?}", d.warnings);
    // El relleno va al final del flujo: los objetos se leen igual.
    assert_eq!(textos(&d), ["Hola mundo"]);
}
