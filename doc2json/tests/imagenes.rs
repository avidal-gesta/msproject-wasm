//! Imagenes de Word y PDF: lista sin repetir, lugares, y extraccion de cada formato. Los PDF y
//! los .docx se arman dentro de la prueba, sin binarios en el repositorio.

use doc2json::doc::{Document, Image, Kind};
use doc2json::png::{self, Color, Spec};
use doc2json::{read, read_with_images};

// ---------------------------------------------------------------------------------------------
// Herramientas: PDF y ZIP minimos, zlib sin comprimir y lectura de PNG
// ---------------------------------------------------------------------------------------------

/// PDF con los objetos dados (el 1 debe ser el catalogo), con su tabla de referencias.
fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{:010} 00000 n \n", o).as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objects.len() + 1, xref).as_bytes());
    out
}

fn obj(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {} /Length {} >>\nstream\n", dict, data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// zlib con un solo bloque sin comprimir.
fn zlib_stored(d: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01, 1];
    let n = d.len() as u16;
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&(!n).to_le_bytes());
    out.extend_from_slice(d);
    let (mut a, mut b) = (1u32, 0u32);
    for &x in d {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

/// ZIP sin compresion.
fn zip(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let le16 = |v: usize| (v as u16).to_le_bytes();
    let le32 = |v: usize| (v as u32).to_le_bytes();
    let (mut z, mut central) = (Vec::new(), Vec::new());
    for (name, data) in parts {
        let n = name.as_bytes();
        let at = z.len();
        z.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&le32(data.len()));
        z.extend_from_slice(&le32(data.len()));
        z.extend_from_slice(&le16(n.len()));
        z.extend_from_slice(&[0, 0]);
        z.extend_from_slice(n);
        z.extend_from_slice(data);
        central.extend_from_slice(&[0x50, 0x4B, 0x01, 0x02, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&le32(data.len()));
        central.extend_from_slice(&le32(data.len()));
        central.extend_from_slice(&le16(n.len()));
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&le32(at));
        central.extend_from_slice(n);
    }
    let start = z.len();
    z.extend_from_slice(&central);
    z.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06, 0, 0, 0, 0]);
    z.extend_from_slice(&le16(parts.len()));
    z.extend_from_slice(&le16(parts.len()));
    z.extend_from_slice(&le32(central.len()));
    z.extend_from_slice(&le32(start));
    z.extend_from_slice(&[0, 0]);
    z
}

/// PNG leido: (ancho, alto, profundidad, tipo de color, paleta, pixeles sin filtros).
fn decode_png(p: &[u8]) -> (u32, u32, u8, u8, Vec<u8>, Vec<u8>) {
    assert!(p.starts_with(b"\x89PNG\r\n\x1a\n"), "no es PNG");
    let (mut i, mut ihdr, mut plte, mut idat) = (8, Vec::new(), Vec::new(), Vec::new());
    while i + 8 <= p.len() {
        let len = u32::from_be_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]]) as usize;
        let kind = &p[i + 4..i + 8];
        let data = &p[i + 8..i + 8 + len];
        match kind {
            b"IHDR" => ihdr = data.to_vec(),
            b"PLTE" => plte = data.to_vec(),
            b"IDAT" => idat.extend_from_slice(data),
            _ => {}
        }
        i += 12 + len;
    }
    let w = u32::from_be_bytes([ihdr[0], ihdr[1], ihdr[2], ihdr[3]]);
    let h = u32::from_be_bytes([ihdr[4], ihdr[5], ihdr[6], ihdr[7]]);
    let (depth, color) = (ihdr[8], ihdr[9]);
    let channels = match color {
        0 | 3 => 1,
        4 => 2,
        2 => 3,
        _ => 4,
    };
    let row = (w as usize * channels * depth as usize).div_ceil(8);
    let bpp = (channels * depth as usize).div_ceil(8);
    let raw = doc2json::inflate::zlib(&idat).data;
    let mut out: Vec<u8> = Vec::new();
    let mut prev = vec![0u8; row];
    for line in raw.chunks(row + 1).take(h as usize) {
        let mut cur = vec![0u8; row];
        for k in 0..row {
            let a = if k >= bpp { cur[k - bpp] } else { 0 } as i16;
            let b = prev[k] as i16;
            let c = if k >= bpp { prev[k - bpp] } else { 0 } as i16;
            let pred = match line[0] {
                0 => 0,
                1 => a,
                2 => b,
                3 => (a + b) / 2,
                _ => {
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
            };
            cur[k] = line[1 + k].wrapping_add(pred as u8);
        }
        out.extend_from_slice(&cur);
        prev = cur;
    }
    (w, h, depth, color, plte, out)
}

/// Bytes a partir de una cadena de bits ("0101..."), rellenando con ceros.
fn bits(s: &str) -> Vec<u8> {
    s.as_bytes().chunks(8).map(|c| c.iter().enumerate().fold(0u8, |b, (i, &v)| b | (((v == b'1') as u8) << (7 - i)))).collect()
}

fn image(d: &Document, id: u32) -> &Image {
    &d.images[id as usize - 1]
}

// ---------------------------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------------------------

/// JPEG minimo: SOI, SOF0 de 2 x 3 pixeles y EOI (basta para reconocer formato y tamano).
const JPEG: [u8; 23] = [0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8, 0, 3, 0, 2, 3, 1, 0x11, 0, 2, 0x11, 1, 3, 0x11, 1, 0xFF, 0xD9];

fn pdf_con_imagenes() -> Vec<u8> {
    // Pagina 1: siete imagenes; pagina 2 (girada 90 grados): la primera otra vez.
    let rgb = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
    let png_rows = [0, 10, 20, 30, 40, 50, 60, 0, 70, 80, 90, 100, 110, 120];
    let content1 = "q 100 0 0 50 72 600 cm /Im1 Do Q q 30 0 0 30 10 10 cm /Im2 Do Q q 20 0 0 20 300 300 cm /Im3 Do Q \
                    q 20 0 0 20 330 300 cm /Im4 Do Q q 20 0 0 20 360 300 cm /Im5 Do Q q 20 0 0 20 390 300 cm /Im6 Do Q \
                    q 20 0 0 20 420 300 cm /Im7 Do Q q 20 0 0 20 450 300 cm /Im8 Do Q";
    let content2 = "q 100 0 0 50 72 600 cm /Im1 Do Q q 10 0 0 10 0 0 cm /Im9 Do Q q 10 0 0 10 20 0 cm /Im10 Do Q";
    let resources = "/Resources << /XObject << /Im1 7 0 R /Im2 8 0 R /Im3 9 0 R /Im4 10 0 R /Im5 12 0 R /Im6 13 0 R /Im7 14 0 R /Im8 15 0 R /Im9 16 0 R /Im10 18 0 R >> >>";
    // Grupo 4, 8 x 4: fila 1 horizontal (2 blancos, 3 negros) y V0 hasta el final; filas 2 a 4
    // iguales a la anterior (tres V0); luego el fin de bloque.
    let g4 = bits("0010111101111111111000000000001000000000001");
    pdf(&[
        obj("<< /Type /Catalog /Pages 2 0 R >>"),
        obj("<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 612 792] >>"),
        obj(&format!("<< /Type /Page /Parent 2 0 R /Contents 5 0 R {} >>", resources)),
        obj(&format!("<< /Type /Page /Parent 2 0 R /Rotate 90 /Contents 6 0 R {} >>", resources)),
        stream("", content1.as_bytes()),
        stream("", content2.as_bytes()),
        // 7: RGB de 8 bits sin comprimir.
        stream("/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8", &rgb),
        // 8: JPEG, se entrega tal cual.
        stream("/Subtype /Image /Width 2 /Height 3 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode", &JPEG),
        // 9: indexada de 1 bit (negro y rojo), filas 10 y 01.
        stream("/Subtype /Image /Width 2 /Height 2 /ColorSpace [/Indexed /DeviceRGB 1 <000000FF0000>] /BitsPerComponent 1", &[0b1000_0000, 0b0100_0000]),
        // 10: gris con mascara suave (11).
        stream("/Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /SMask 11 0 R", &[10, 200]),
        stream("/Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8", &[255, 0]),
        // 12: Flate con predictor PNG: pasa sin descomprimir.
        stream(
            "/Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 3 /Columns 2 >>",
            &zlib_stored(&png_rows),
        ),
        // 13: fax grupo 4: se decodifica a PNG.
        stream("/Subtype /Image /Width 8 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 8 >>", &g4),
        // 14: JBIG2 sin segmentos validos: queda con una nota.
        stream("/Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /JBIG2Decode", &[0, 1, 2]),
        // 15: mascara de un bit (estarcido).
        stream("/Subtype /Image /Width 8 /Height 1 /ImageMask true", &[0b1111_0000]),
        // 16 y 17: gris premezclado con blanco (/Matte) y su mascara: 255 + (178 - 255) * 255 / 128 = 102.
        stream("/Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /SMask 17 0 R", &[178, 255]),
        stream("/Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Matte [1]", &[128, 0]),
        // 18: fax que no se puede decodificar: se entrega tal cual, en un TIFF.
        stream("/Subtype /Image /Width 8 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 8 >>", &[0, 0]),
    ])
}

#[test]
fn pdf_lista_lugares_y_formatos_sin_extraer() {
    let d = read(&pdf_con_imagenes()).unwrap();
    assert_eq!(d.images.len(), 10);
    let im1 = image(&d, 1);
    assert_eq!((im1.format, im1.width, im1.height), ("png", Some(2), Some(2)));
    assert!(im1.data.is_none());
    // Dibujada en las dos paginas: una sola imagen con dos lugares.
    assert_eq!(im1.places.len(), 2);
    let p = &im1.places[0];
    assert_eq!((p.page, p.x, p.y, p.w, p.h), (Some(1), Some(72.0), Some(142.0), 100.0, 50.0));
    // En la pagina girada se lee apaisada: el ancho y el alto se intercambian.
    let q = &im1.places[1];
    assert_eq!((q.page, q.w, q.h), (Some(2), 50.0, 100.0));
    let formats: Vec<&str> = d.images.iter().map(|i| i.format).collect();
    // Fax y JBIG2 se anuncian como PNG: se decodifican al extraer.
    assert_eq!(formats, ["png", "jpeg", "png", "png", "png", "png", "png", "png", "png", "png"]);
    // La mascara suave (objeto 11) no se dibuja sola: no figura como imagen propia.
}

#[test]
fn pdf_extrae_cada_formato() {
    let d = read_with_images(&pdf_con_imagenes()).unwrap();
    let data = |id: u32| image(&d, id).data.clone().unwrap_or_else(|| panic!("imagen {} sin datos: {:?}", id, image(&d, id).note));

    let (w, h, depth, color, _, px) = decode_png(&data(1));
    assert_eq!((w, h, depth, color), (2, 2, 8, 2));
    assert_eq!(px, [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);

    assert_eq!(data(2), JPEG);
    assert_eq!(image(&d, 2).mime(), "image/jpeg");

    let (_, _, depth, color, plte, px) = decode_png(&data(3));
    assert_eq!((depth, color), (1, 3));
    assert_eq!(plte, [0, 0, 0, 255, 0, 0]);
    assert_eq!(px, [0b1000_0000, 0b0100_0000]);

    // Gris con transparencia: gris + alfa.
    let (_, _, depth, color, _, px) = decode_png(&data(4));
    assert_eq!((depth, color), (8, 4));
    assert_eq!(px, [10, 255, 200, 0]);

    // Predictor PNG: los datos comprimidos son los mismos del PDF.
    let p5 = data(5);
    let (_, _, _, color, _, px) = decode_png(&p5);
    assert_eq!(color, 2);
    assert_eq!(px, [10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120]);
    assert!(p5.windows(5).any(|w| w == [0x78, 0x01, 1, 14, 0]));

    // Fax de grupo 4 decodificado: gris de 1 bit, 0 = negro (sin BlackIs1).
    let (w, h, depth, color, _, px) = decode_png(&data(6));
    assert_eq!((w, h, depth, color), (8, 4, 1, 0));
    assert_eq!(px, [0b1100_0111; 4]);
    assert!(image(&d, 6).note.is_none());

    // Fax que no se decodifica: TIFF de grupo 4 con los datos tal cual al final, y una nota.
    let tif = data(10);
    assert!(tif.starts_with(b"II*\0"));
    assert!(tif.ends_with(&[0, 0]));
    assert!(image(&d, 10).note.as_deref().unwrap_or("").contains("TIFF"));
    let entry = |tag: u16| {
        let n = u16::from_le_bytes([tif[8], tif[9]]) as usize;
        (0..n).map(|k| 10 + 12 * k).find(|&o| u16::from_le_bytes([tif[o], tif[o + 1]]) == tag).map(|o| u32::from_le_bytes([tif[o + 8], tif[o + 9], tif[o + 10], tif[o + 11]]))
    };
    assert_eq!((entry(256), entry(257), entry(259)), (Some(8), Some(4), Some(4)));

    assert!(image(&d, 7).data.is_none());
    assert!(image(&d, 7).note.as_deref().unwrap_or("").contains("JBIG2"));

    // Estarcido: gris de 1 bit, los ceros son la tinta (negro).
    let (w, _, depth, color, _, px) = decode_png(&data(8));
    assert_eq!((w, depth, color), (8, 1, 0));
    assert_eq!(px, [0b1111_0000]);

    // Premezclado con /Matte: se recupera el color original; la parte transparente queda igual.
    let (_, _, _, color, _, px) = decode_png(&data(9));
    assert_eq!(color, 4);
    assert_eq!(px, [102, 128, 255, 0]);
}

#[test]
fn pdf_json_y_bloque_de_bytes() {
    let d = read_with_images(&pdf_con_imagenes()).unwrap();
    let (json, blob) = doc2json::doc::to_json_with_images(&d, false, 3);
    // Con lado minimo 3 solo quedan los bytes de los dos fax (8 x 4); el estarcido (8 x 1) no.
    let mut both = image(&d, 6).data.clone().unwrap();
    both.extend(image(&d, 10).data.clone().unwrap());
    assert_eq!(blob, both);
    assert!(json.contains("\"offset\":0"));
    assert!(json.contains("\"places\":[{\"page\":1,\"x\":72,\"y\":142,\"w\":100,\"h\":50}"));
    // Sin extraer, el JSON no cambia por las imagenes salvo la lista.
    let plain = doc2json::to_json(&pdf_con_imagenes(), false).unwrap();
    assert!(plain.contains("\"images\":[{\"id\":1,\"format\":\"png\",\"width\":2,\"height\":2"));
    assert!(!plain.contains("offset"));
}

// ---------------------------------------------------------------------------------------------
// Word
// ---------------------------------------------------------------------------------------------

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn dibujo(rid: &str, cx: u64, cy: u64, alt: &str) -> String {
    format!(
        r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="{}" cy="{}"/><wp:docPr id="1" name="Imagen" descr="{}"/><a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip r:embed="{}"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#,
        cx, cy, alt, rid
    )
}

fn docx_con_imagenes(png_bytes: &[u8], logo: &[u8]) -> Vec<u8> {
    let ns = r#"xmlns:w="w" xmlns:wp="wp" xmlns:a="a" xmlns:pic="pic" xmlns:r="r""#;
    let body = format!(
        r#"<w:document {}><w:body><w:p><w:r><w:t>Antes del diagrama</w:t></w:r></w:p>{}{}<w:sectPr><w:headerReference r:id="rId6"/></w:sectPr></w:body></w:document>"#,
        ns,
        dibujo("rId5", 1_270_000, 635_000, "Diagrama del circuito"),
        dibujo("rId5", 635_000, 317_500, ""),
    );
    let header = format!(r#"<w:hdr {}>{}</w:hdr>"#, ns, dibujo("rId1", 254_000, 254_000, "Logo"));
    let rels = |list: &[(&str, &str, &str)]| {
        let mut s = String::from(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
        for (id, kind, target) in list {
            s.push_str(&format!(r#"<Relationship Id="{}" Type="{}/{}" Target="{}"/>"#, id, REL, kind, target));
        }
        s + "</Relationships>"
    };
    zip(&[
        ("_rels/.rels", rels(&[("rId1", "officeDocument", "word/document.xml")]).as_bytes()),
        ("word/document.xml", body.as_bytes()),
        ("word/_rels/document.xml.rels", rels(&[("rId5", "image", "media/image1.png"), ("rId6", "header", "header1.xml")]).as_bytes()),
        ("word/header1.xml", header.as_bytes()),
        ("word/_rels/header1.xml.rels", rels(&[("rId1", "image", "media/logo.png")]).as_bytes()),
        ("word/media/image1.png", png_bytes),
        ("word/media/logo.png", logo),
    ])
}

#[test]
fn word_lista_bloques_y_extraccion() {
    let img = png::encode(&Spec { width: 4, height: 3, depth: 8, color: Color::Gray, palette: Vec::new() }, &[7; 12]);
    let logo = png::encode(&Spec { width: 1, height: 1, depth: 8, color: Color::Rgb, palette: Vec::new() }, &[1, 2, 3]);
    let bytes = docx_con_imagenes(&img, &logo);

    let d = read(&bytes).unwrap();
    let blocks: Vec<_> = d.content.iter().filter(|b| b.kind == Kind::Image).collect();
    assert_eq!(blocks.len(), 2);
    assert!(blocks.iter().all(|b| b.image == Some(1)));
    assert_eq!(d.images.len(), 2);
    let im = image(&d, 1);
    assert_eq!((im.format, im.name.as_deref(), im.alt.as_str()), ("png", Some("image1.png"), "Diagrama del circuito"));
    // Mismo archivo, dos tamanos de dibujo; en Word los pixeles se conocen solo al extraer.
    assert_eq!(im.places.iter().map(|p| (p.w, p.h)).collect::<Vec<_>>(), [(100.0, 50.0), (50.0, 25.0)]);
    assert_eq!((im.width, im.data.is_none()), (None, true));
    // El logo del encabezado figura en la lista aunque el encabezado salga como texto.
    assert_eq!((image(&d, 2).name.as_deref(), image(&d, 2).alt.as_str()), (Some("logo.png"), "Logo"));

    let d = read_with_images(&bytes).unwrap();
    assert_eq!(image(&d, 1).data.as_deref(), Some(&img[..]));
    assert_eq!((image(&d, 1).width, image(&d, 1).height), (Some(4), Some(3)));
    assert_eq!(image(&d, 2).data.as_deref(), Some(&logo[..]));
    let json = doc2json::doc::to_json(&d, false);
    assert!(json.contains("\"type\":\"image\",\"image\":1,\"name\":\"image1.png\""));
    assert!(json.contains("\"mime\":\"image/png\""));
}

#[test]
fn word_imagen_en_celda_de_tabla() {
    // Como los pictogramas de EPP: una tabla con el nombre y su imagen en la misma celda.
    let ns = r#"xmlns:w="w" xmlns:wp="wp" xmlns:a="a" xmlns:pic="pic" xmlns:r="r""#;
    let celda = |texto: &str, rid: &str| format!(r#"<w:tc><w:p><w:r><w:t>{}</w:t></w:r></w:p>{}</w:tc>"#, texto, dibujo(rid, 254_000, 254_000, texto));
    let body = format!(
        r#"<w:document {}><w:body><w:tbl><w:tr>{}{}</w:tr><w:tr><w:tc><w:tbl><w:tr>{}</w:tr></w:tbl></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl></w:body></w:document>"#,
        ns,
        celda("Casco", "rId1"),
        celda("Guantes", "rId2"),
        celda("Casco", "rId1"),
    );
    let rels = format!(
        r#"<Relationships xmlns="r"><Relationship Id="rId1" Type="{0}/image" Target="media/casco.png"/><Relationship Id="rId2" Type="{0}/image" Target="media/guantes.png"/></Relationships>"#,
        REL
    );
    let pkg = format!(r#"<Relationships xmlns="r"><Relationship Id="rId1" Type="{}/officeDocument" Target="word/document.xml"/></Relationships>"#, REL);
    let bytes = zip(&[
        ("_rels/.rels", pkg.as_bytes()),
        ("word/document.xml", body.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/media/casco.png", b"x"),
        ("word/media/guantes.png", b"y"),
    ]);
    let d = read(&bytes).unwrap();
    let table = d.content.iter().find(|b| b.kind == Kind::Table).unwrap();
    let ids: Vec<Vec<Vec<u32>>> = table.rows.iter().map(|r| r.iter().map(|c| c.images.clone()).collect()).collect();
    // La tabla anidada aporta su imagen a la celda que la contiene.
    assert_eq!(ids, [vec![vec![1], vec![2]], vec![vec![1], vec![]]]);
    assert_eq!(table.rows[0][0].text, "Casco");
    assert_eq!(image(&d, 1).places.len(), 2);
    assert!(doc2json::doc::to_json(&d, false).contains("{\"text\":\"Casco\",\"images\":[1]}"));
}

#[test]
fn ejemplos_sin_imagenes_no_cambian() {
    for name in ["procedimiento.docx", "procedimiento.pdf"] {
        let path = format!("{}/../ejemplo/{}", env!("CARGO_MANIFEST_DIR"), name);
        let bytes = std::fs::read(&path).unwrap();
        let d = read_with_images(&bytes).unwrap();
        assert!(d.images.is_empty());
        let expected = std::fs::read_to_string(format!("{}.json", path)).unwrap();
        assert_eq!(doc2json::doc::to_json(&d, false), expected);
    }
}
