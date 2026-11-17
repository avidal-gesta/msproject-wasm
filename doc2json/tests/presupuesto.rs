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
    let mut z = Vec::new();
    let n = nombre.as_bytes();
    let le16 = |v: usize| (v as u16).to_le_bytes();
    let le32 = |v: usize| (v as u32).to_le_bytes();
    z.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    z.extend_from_slice(&le32(comprimido.len()));
    z.extend_from_slice(&le32(tamano));
    z.extend_from_slice(&le16(n.len()));
    z.extend_from_slice(&[0, 0]);
    z.extend_from_slice(n);
    z.extend_from_slice(comprimido);
    let central = z.len();
    z.extend_from_slice(&[0x50, 0x4B, 0x01, 0x02, 20, 0, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    z.extend_from_slice(&le32(comprimido.len()));
    z.extend_from_slice(&le32(tamano));
    z.extend_from_slice(&le16(n.len()));
    // Extra, comentario, disco, atributos internos (2 c/u) y externos (4); luego el desplazamiento.
    z.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    z.extend_from_slice(&le32(0));
    z.extend_from_slice(n);
    let tam_central = z.len() - central;
    z.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06, 0, 0, 0, 0, 1, 0, 1, 0]);
    z.extend_from_slice(&le32(tam_central));
    z.extend_from_slice(&le32(central));
    z.extend_from_slice(&[0, 0]);
    z
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
