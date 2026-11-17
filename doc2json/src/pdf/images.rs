//! Imagenes de PDF: formato con que se entregan, ubicacion en la pagina y extraccion.
//!
//! - JPEG (DCTDecode) y JPEG 2000 (JPXDecode) se entregan tal cual, sin recomprimir.
//! - Fax (CCITTFaxDecode, grupos 3 y 4) y JBIG2 se decodifican y salen como PNG de un bit,
//!   listos para OCR. Si el fax no se puede decodificar, se entrega envuelto en un TIFF.
//! - Los pixeles (Flate, LZW, RunLength, sin compresion...) pasan a PNG: gris, RGB, CMYK
//!   (convertido a RGB), indexado (con su paleta), tinta plana (Separation) y mascaras de un bit;
//!   con transparencia si traen mascara suave (SMask) del mismo tamano. Si el flujo ya es un PNG
//!   por dentro (Flate con predictor PNG y parametros coincidentes) se copia sin recomprimir.
//! - Espacios de color como Lab, y JBIG2 con Huffman o semitonos, no se convierten: la imagen
//!   figura con una nota.

use super::document::Pdf;
use super::filters::{self, Parms};
use super::object::{Dict, Obj};
use crate::inflate;
use crate::png::{self, Color, Spec};
use crate::{ccitt, jbig2};

/// Tope de pixeles por imagen al extraer (5.000 x 5.000): mas alla, la memoria no compensa.
pub const MAX_PIXELS: u64 = 25_000_000;
/// Tope para imagenes de un bit (escaneos en blanco y negro), que ocupan mucho menos: alcanza
/// para una hoja A3 a 600 dpi (unos 70 millones de pixeles).
pub const MAX_PIXELS_1BIT: u64 = 100_000_000;

/// Filtro que ya es un formato de imagen: los datos se entregan desde ahi.
fn image_filter(name: &str) -> Option<&'static str> {
    match name {
        "DCTDecode" | "DCT" => Some("jpeg"),
        "JPXDecode" => Some("jp2"),
        _ => None,
    }
}

#[derive(Clone, Debug)]
enum Space {
    Gray,
    Rgb,
    Cmyk,
    /// Paleta ya convertida a RGB (3 bytes por entrada).
    Indexed(Vec<u8>),
    /// Una tinta (Separation o DeviceN de un componente): 1 = tinta plena (oscuro).
    Tint,
    Other(String),
}

impl Space {
    fn components(&self) -> usize {
        match self {
            Space::Rgb => 3,
            Space::Cmyk => 4,
            _ => 1,
        }
    }
}

fn int(pdf: &Pdf, d: &Dict, key: &str) -> Option<i64> {
    pdf.get(d, key).as_i64()
}

fn flag(pdf: &Pdf, d: &Dict, key: &str) -> bool {
    matches!(&*pdf.get(d, key), Obj::Bool(true))
}

/// El diccionario pide invertir los valores (`/Decode [1 0 ...]`).
fn inverted(pdf: &Pdf, d: &Dict) -> bool {
    match pdf.array(d, "Decode") {
        Some(a) if a.len() >= 2 => a[0].as_f64() == Some(1.0) && a[1].as_f64() == Some(0.0),
        _ => false,
    }
}

fn space(pdf: &Pdf, obj: &Obj, depth: usize) -> Space {
    let o = pdf.resolve(obj);
    match &*o {
        Obj::Name(n) => match n.as_str() {
            "DeviceGray" | "G" | "CalGray" => Space::Gray,
            "DeviceRGB" | "RGB" | "CalRGB" => Space::Rgb,
            "DeviceCMYK" | "CMYK" => Space::Cmyk,
            other => Space::Other(other.to_string()),
        },
        Obj::Array(a) if depth < 4 => {
            let head = a.first().map(|x| pdf.resolve(x)).and_then(|x| x.as_name().map(String::from)).unwrap_or_default();
            match head.as_str() {
                "CalGray" => Space::Gray,
                "CalRGB" => Space::Rgb,
                "ICCBased" => {
                    let stream = a.get(1).map(|x| pdf.resolve(x));
                    let n = stream.as_deref().and_then(|s| match s {
                        Obj::Stream { dict, .. } => pdf.get(dict, "N").as_i64(),
                        _ => None,
                    });
                    match n {
                        Some(1) => Space::Gray,
                        Some(3) => Space::Rgb,
                        Some(4) => Space::Cmyk,
                        _ => Space::Other("ICCBased".into()),
                    }
                }
                "Indexed" | "I" => {
                    let base = a.get(1).map(|b| space(pdf, b, depth + 1)).unwrap_or(Space::Other("Indexed".into()));
                    let hival = a.get(2).and_then(|v| pdf.resolve(v).as_i64()).unwrap_or(0).clamp(0, 255) as usize;
                    let lookup: Vec<u8> = match a.get(3).map(|l| pdf.resolve(l)).as_deref() {
                        Some(Obj::Str(s)) => s.clone(),
                        Some(s @ Obj::Stream { .. }) => pdf.stream_data(s).unwrap_or_default(),
                        _ => Vec::new(),
                    };
                    let n = base.components();
                    let mut palette = Vec::with_capacity((hival + 1) * 3);
                    for i in 0..=hival {
                        let at = |k: usize| lookup.get(i * n + k).copied().unwrap_or(0);
                        let rgb = match base {
                            Space::Rgb => [at(0), at(1), at(2)],
                            Space::Cmyk => cmyk([at(0), at(1), at(2), at(3)]),
                            Space::Tint => [255 - at(0); 3],
                            Space::Gray => [at(0); 3],
                            _ => return Space::Other("Indexed".into()),
                        };
                        palette.extend_from_slice(&rgb);
                    }
                    Space::Indexed(palette)
                }
                "Separation" => Space::Tint,
                "DeviceN" if a.get(1).and_then(|x| pdf.resolve(x).as_array().map(|v| v.len())) == Some(1) => Space::Tint,
                other => Space::Other(other.to_string()),
            }
        }
        _ => Space::Other("desconocido".into()),
    }
}

fn cmyk(c: [u8; 4]) -> [u8; 3] {
    let k = 255 - c[3] as u32;
    [((255 - c[0] as u32) * k / 255) as u8, ((255 - c[1] as u32) * k / 255) as u8, ((255 - c[2] as u32) * k / 255) as u8]
}

fn filter_names(pdf: &Pdf, d: &Dict) -> Vec<String> {
    match &*pdf.get(d, "Filter") {
        Obj::Name(n) => vec![n.clone()],
        Obj::Array(a) => a.iter().filter_map(|o| pdf.resolve(o).as_name().map(String::from)).collect(),
        _ => Vec::new(),
    }
}

fn image_space(pdf: &Pdf, d: &Dict) -> Space {
    if flag(pdf, d, "ImageMask") || flag(pdf, d, "IM") {
        return Space::Gray;
    }
    match d.get("ColorSpace").or_else(|| d.get("CS")) {
        Some(cs) => space(pdf, cs, 0),
        None => Space::Other("sin espacio de color".into()),
    }
}

/// Formato con que se entregaria la imagen y, si no se puede entregar, por que. No decodifica.
pub fn classify(pdf: &Pdf, d: &Dict) -> (&'static str, Option<String>) {
    if let Some(f) = filter_names(pdf, d).iter().find_map(|n| image_filter(n)) {
        return (f, None);
    }
    match image_space(pdf, d) {
        Space::Other(name) => ("desconocido", Some(format!("Espacio de color {} no soportado.", name))),
        _ => ("png", None),
    }
}

/// Ancho y alto en pixeles declarados por el diccionario.
pub fn size(pdf: &Pdf, d: &Dict) -> Option<(u32, u32)> {
    let w = int(pdf, d, "Width").or_else(|| int(pdf, d, "W"))?;
    let h = int(pdf, d, "Height").or_else(|| int(pdf, d, "H"))?;
    (w > 0 && h > 0 && w <= u32::MAX as i64 && h <= u32::MAX as i64).then_some((w as u32, h as u32))
}

/// Rectangulo que ocupa en la pagina el cuadrado unidad transformado por `m` (asi se dibuja
/// una imagen), en la orientacion de lectura y con el origen arriba a la izquierda.
pub fn place(m: [f64; 6], bbox: [f64; 4], rotate: i64) -> (f64, f64, f64, f64) {
    let (w, h) = (bbox[2] - bbox[0], bbox[3] - bbox[1]);
    let ph = if rotate == 90 || rotate == 270 { w } else { h };
    let point = |x: f64, y: f64| {
        let (x, y) = (m[0] * x + m[2] * y + m[4] - bbox[0], m[1] * x + m[3] * y + m[5] - bbox[1]);
        match rotate {
            90 => (y, w - x),
            180 => (w - x, h - y),
            270 => (h - y, x),
            _ => (x, y),
        }
    };
    let corners = [point(0.0, 0.0), point(1.0, 0.0), point(0.0, 1.0), point(1.0, 1.0)];
    let (x0, x1) = corners.iter().fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.0), b.max(c.0)));
    let (y0, y1) = corners.iter().fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.1), b.max(c.1)));
    let r = |v: f64| (v * 100.0).round() / 100.0;
    (r(x0), r(ph - y1), r(x1 - x0), r(y1 - y0))
}

/// Extrae la imagen: formato, bytes del archivo y un aviso (por ejemplo, un fax danado que se
/// decodifico en parte), o el motivo por el que no se entrega.
pub fn extract(pdf: &Pdf, obj: &Obj) -> Result<(&'static str, Vec<u8>, Option<String>), String> {
    let d = match obj {
        Obj::Stream { dict, .. } => dict,
        _ => return Err("El objeto no es una imagen.".into()),
    };
    let (w, h) = size(pdf, d).ok_or("La imagen no declara su tamano.")?;
    let one_bit = bits(pdf, d) == 1 && image_space(pdf, d).components() == 1 && d.get("SMask").is_none();
    if w as u64 * h as u64 > if one_bit { MAX_PIXELS_1BIT } else { MAX_PIXELS } {
        return Err(format!("Imagen demasiado grande para extraerla ({} x {} pixeles).", w, h));
    }
    let (mut data, chain) = pdf.stream_parts(obj).ok_or("No se pudieron leer los datos de la imagen.")?;
    let last = chain.len().saturating_sub(1);
    let mut note = None;
    for (i, (name, parms, raw)) in chain.iter().enumerate() {
        if let Some(kind) = image_filter(name) {
            return Ok((kind, data, None));
        }
        match name.as_str() {
            "CCITTFaxDecode" | "CCF" => {
                let (rows, decoded) = ccitt_samples(pdf, &data, w, h, raw.as_ref())?;
                if decoded == 0 {
                    let tiff = ccitt_tiff(pdf, &data, w, h, raw.as_ref(), inverted(pdf, d));
                    return Ok(("tiff", tiff, Some("No se pudo decodificar el fax: se entrega tal cual, como TIFF.".into())));
                }
                if decoded < h as usize {
                    note = Some(format!("Fax danado: se decodificaron {} de {} filas.", decoded, h));
                }
                data = rows;
                continue;
            }
            "JBIG2Decode" => {
                let globals = raw.as_ref().map(|p| pdf.get(p, "JBIG2Globals")).filter(|g| matches!(**g, Obj::Stream { .. })).and_then(|g| pdf.stream_data(&g));
                let page = jbig2::decode(&data, globals.as_deref(), w as usize, h as usize)?;
                // En PDF, el filtro entrega 0 = negro (al reves que JBIG2).
                data = pack(&page.px, page.w, page.h, true);
                continue;
            }
            _ => {}
        }
        if i == last && matches!(name.as_str(), "FlateDecode" | "Fl") {
            if let Some(png) = passthrough(pdf, d, &data, w, h, parms) {
                return Ok(("png", png, None));
            }
        }
        data = filters::apply(name, &data, parms).ok_or_else(|| format!("Filtro {} no soportado.", name))?;
    }
    pixels_to_png(pdf, d, data, w, h).map(|p| ("png", p, note))
}

/// Filas de un bit empaquetadas a partir de un pixel por byte (1 = negro); `invert` deja 0 = negro.
fn pack(px: &[u8], w: usize, h: usize, invert: bool) -> Vec<u8> {
    let stride = w.div_ceil(8);
    let mut out = vec![0u8; stride * h];
    for y in 0..h {
        for x in 0..w {
            if px.get(y * w + x).copied().unwrap_or(0) != 0 {
                out[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    if invert {
        out.iter_mut().for_each(|b| *b = !*b);
    }
    out
}

/// Fax decodificado como lo entrega el filtro de PDF (sin `BlackIs1`, 0 = negro), con filas del
/// ancho de la imagen. Devuelve tambien cuantas filas se decodificaron. Columnas o filas fuera de
/// rango son un error.
fn ccitt_samples(pdf: &Pdf, data: &[u8], w: u32, h: u32, parms: Option<&Dict>) -> Result<(Vec<u8>, usize), String> {
    let get = |k: &str| parms.and_then(|p| pdf.get(p, k).as_i64());
    let on = |k: &str| parms.is_some_and(|p| flag(pdf, p, k));
    // Sin `as usize`: en wasm32 truncaria un valor enorme del archivo a uno chico cualquiera.
    let size = |k: &str, default: u32| match get(k) {
        Some(v) if v > 0 => usize::try_from(v).map_err(|_| format!("Fax con /{} fuera de rango ({}).", k, v)),
        _ => Ok(default as usize),
    };
    let columns = size("Columns", w)?;
    let p = ccitt::Params {
        k: get("K").unwrap_or(0),
        columns,
        // Mas filas que las de la imagen no se usan: no se decodifican.
        rows: size("Rows", h)?.min(h as usize),
        byte_align: on("EncodedByteAlign"),
        end_of_block: !parms.is_some_and(|p| matches!(&*pdf.get(p, "EndOfBlock"), Obj::Bool(false))),
    };
    let (rows, decoded) = ccitt::decode(data, &p, h as usize)?;
    let black_is_1 = on("BlackIs1");
    let (w, h) = (w as usize, h as usize);
    let src_stride = columns.div_ceil(8);
    let stride = w.div_ceil(8);
    let mut out = vec![0u8; stride * h];
    for y in 0..h.min(rows.len() / src_stride.max(1)) {
        if columns == w {
            out[y * stride..(y + 1) * stride].copy_from_slice(&rows[y * src_stride..(y + 1) * src_stride]);
        } else {
            for x in 0..w.min(columns) {
                if rows[y * src_stride + x / 8] & (0x80 >> (x % 8)) != 0 {
                    out[y * stride + x / 8] |= 0x80 >> (x % 8);
                }
            }
        }
    }
    if !black_is_1 {
        out.iter_mut().for_each(|b| *b = !*b);
    }
    Ok((out, decoded))
}

fn bits(pdf: &Pdf, d: &Dict) -> u8 {
    if flag(pdf, d, "ImageMask") || flag(pdf, d, "IM") {
        return 1;
    }
    match int(pdf, d, "BitsPerComponent").or_else(|| int(pdf, d, "BPC")) {
        Some(b @ (1 | 2 | 4 | 8 | 16)) => b as u8,
        _ => 8,
    }
}

/// Los datos Flate con predictor PNG ya son el contenido de IDAT de un PNG. Se copian solo si un
/// lector de PNG estricto los aceptaria: zlib sin diccionario, filas completas, tipos de filtro
/// 0 a 4 y suma Adler-32 correcta. Si no, `None` y la imagen se decodifica y se vuelve a comprimir.
fn passthrough(pdf: &Pdf, d: &Dict, data: &[u8], w: u32, h: u32, p: &Parms) -> Option<Vec<u8>> {
    if d.get("SMask").is_some() || p.predictor < 10 || inverted(pdf, d) {
        return None;
    }
    // Encabezado zlib valido y sin diccionario previo: algunos PDF traen DEFLATE sin el, y un
    // PNG no admite ninguna de las dos cosas.
    if data.len() < 6 || data[0] & 0x0F != 8 || ((data[0] as u32) << 8 | data[1] as u32) % 31 != 0 || data[1] & 0x20 != 0 {
        return None;
    }
    let bpc = bits(pdf, d);
    let (color, palette) = match image_space(pdf, d) {
        Space::Gray => (Color::Gray, Vec::new()),
        Space::Rgb if bpc >= 8 => (Color::Rgb, Vec::new()),
        Space::Indexed(pal) if bpc <= 8 => (Color::Palette, pal),
        _ => return None,
    };
    if p.colors != color.channels() as i64 || p.bits != bpc as i64 || p.columns != w as i64 {
        return None;
    }
    let spec = Spec { width: w, height: h, depth: bpc, color, palette: fit_palette(palette, bpc) };
    let row = png::row_bytes(&spec)?.checked_add(1)?;
    let rows = inflate::inflate(&data[2..]);
    let adler = u32::from_be_bytes([data[data.len() - 4], data[data.len() - 3], data[data.len() - 2], data[data.len() - 1]]);
    let valid = rows.ok
        && Some(rows.data.len()) == row.checked_mul(h as usize)
        && rows.data.chunks(row).all(|r| r[0] <= 4)
        && png::adler32(&rows.data) == adler;
    valid.then(|| png::from_idat(&spec, data))
}

/// Paleta para el PNG: exactamente una entrada por cada indice posible con `bpc` bits. Un PNG no
/// admite mas, y un indice sin entrada lo deja invalido: las que el PDF no trae quedan en negro.
fn fit_palette(mut pal: Vec<u8>, bpc: u8) -> Vec<u8> {
    pal.resize((1usize << bpc.min(8)) * 3, 0);
    pal
}

/// Bytes de una imagen de `w` x `h` con `comps` componentes de `bpc` bits, filas completas al
/// byte. `None` si no cabe en la memoria direccionable (en wasm32, 4 GB).
fn image_bytes(w: u32, h: u32, comps: usize, bpc: u8) -> Option<usize> {
    let row = (w as u64).checked_mul(comps as u64)?.checked_mul(bpc as u64)?.checked_add(7)? / 8;
    usize::try_from(row.checked_mul(h as u64)?).ok()
}

/// Los datos decodificados alcanzan para la imagen declarada. Faltando algo (un escaneo
/// cortado), se completa con ceros; faltando mas de la mitad no se extrae: el tamano declarado
/// no debe decidir cuanta memoria se pide si los datos no lo respaldan.
fn enough(len: usize, need: Option<usize>) -> Result<usize, String> {
    match need {
        Some(n) if len >= n - n / 2 => Ok(n),
        Some(n) => Err(format!("Los datos de la imagen estan incompletos ({} de {} bytes).", len, n)),
        None => Err("Imagen demasiado grande para extraerla.".into()),
    }
}

/// Muestra `i` de una fila (sin escalar).
fn sample(row: &[u8], i: usize, bpc: u8) -> u16 {
    match bpc {
        8 => row.get(i).copied().unwrap_or(0) as u16,
        16 => u16::from_be_bytes([row.get(2 * i).copied().unwrap_or(0), row.get(2 * i + 1).copied().unwrap_or(0)]),
        b => {
            let bit = i * b as usize;
            let byte = row.get(bit / 8).copied().unwrap_or(0);
            ((byte >> (8 - b as usize - bit % 8)) & ((1u8 << b) - 1)) as u16
        }
    }
}

/// Lleva una muestra a 0..255.
fn scale(v: u16, bpc: u8) -> u8 {
    match bpc {
        8 => v as u8,
        16 => (v >> 8) as u8,
        b => (v as u32 * 255 / ((1u32 << b) - 1)) as u8,
    }
}

/// Pixeles en 8 bits por canal, gris o RGB.
fn to_8bit(space: &Space, data: &[u8], w: u32, h: u32, bpc: u8, invert: bool) -> Result<(Vec<u8>, Color), String> {
    let (w, h) = (w as usize, h as usize);
    let comps = space.components();
    let row = (w * comps * bpc as usize + 7) / 8;
    let gray = matches!(space, Space::Gray | Space::Tint);
    let mut out = Vec::with_capacity(w * h * if gray { 1 } else { 3 });
    let empty: Vec<u8> = Vec::new();
    for y in 0..h {
        let line = data.get(y * row..((y + 1) * row).min(data.len())).unwrap_or(&empty);
        for x in 0..w {
            let s = |k: usize| {
                let v = scale(sample(line, x * comps + k, bpc), bpc);
                if invert {
                    255 - v
                } else {
                    v
                }
            };
            match space {
                Space::Gray => out.push(s(0)),
                Space::Tint => out.push(255 - s(0)),
                Space::Rgb => out.extend_from_slice(&[s(0), s(1), s(2)]),
                Space::Cmyk => out.extend_from_slice(&cmyk([s(0), s(1), s(2), s(3)])),
                Space::Indexed(pal) => {
                    let i = sample(line, x, bpc) as usize * 3;
                    out.extend_from_slice(pal.get(i..i + 3).unwrap_or(&[0, 0, 0]));
                }
                Space::Other(name) => return Err(format!("Espacio de color {} no soportado.", name)),
            }
        }
    }
    Ok((out, if gray { Color::Gray } else { Color::Rgb }))
}

/// Mascara suave (transparencia) en 8 bits, si tiene el mismo tamano que la imagen, y su color
/// `/Matte` (0-255) si los colores de la imagen vienen ya mezclados con el.
fn soft_mask(pdf: &Pdf, d: &Dict, w: u32, h: u32) -> Option<(Vec<u8>, Option<Vec<f64>>)> {
    let obj = pdf.resolve(d.get("SMask")?);
    let md = match &*obj {
        Obj::Stream { dict, .. } => dict,
        _ => return None,
    };
    if size(pdf, md)? != (w, h) {
        return None;
    }
    let data = pdf.stream_data(&obj)?;
    // Una mascara sin datos que la respalden se ignora: la imagen sale sin transparencia.
    enough(data.len(), image_bytes(w, h, 1, bits(pdf, md))).ok()?;
    let matte = pdf.array(md, "Matte").map(|m| m.iter().map(|v| v.as_f64().unwrap_or(0.0).clamp(0.0, 1.0) * 255.0).collect());
    to_8bit(&Space::Gray, &data, w, h, bits(pdf, md), inverted(pdf, md)).ok().map(|(a, _)| (a, matte))
}

fn pixels_to_png(pdf: &Pdf, d: &Dict, mut data: Vec<u8>, w: u32, h: u32) -> Result<Vec<u8>, String> {
    let bpc = bits(pdf, d);
    let space = image_space(pdf, d);
    let invert = inverted(pdf, d);
    if let Space::Other(name) = &space {
        return Err(format!("Espacio de color {} no soportado.", name));
    }
    enough(data.len(), image_bytes(w, h, space.components(), bpc))?;
    if let Some((alpha, matte)) = soft_mask(pdf, d, w, h) {
        let (pixels, color) = to_8bit(&space, &data, w, h, bpc, invert)?;
        let ch = color.channels();
        // Con /Matte el color guardado es m + a * (c - m): se recupera c = m + (guardado - m) / a.
        let matte = matte.filter(|m| m.len() == ch && matches!(space, Space::Gray | Space::Rgb));
        let mut out = Vec::with_capacity(pixels.len() + alpha.len());
        for (px, a) in pixels.chunks(ch).zip(alpha) {
            match &matte {
                Some(m) if a > 0 => {
                    let f = 255.0 / a as f64;
                    out.extend(px.iter().zip(m).map(|(&c, &mv)| (mv + (c as f64 - mv) * f).round().clamp(0.0, 255.0) as u8));
                }
                _ => out.extend_from_slice(px),
            }
            out.push(a);
        }
        let color = if color == Color::Gray { Color::GrayAlpha } else { Color::Rgba };
        return encode(&Spec { width: w, height: h, depth: 8, color, palette: Vec::new() }, &out);
    }
    // Sin transparencia se conserva la profundidad original cuando PNG la admite.
    let native = match &space {
        Space::Gray | Space::Tint => Some((Color::Gray, Vec::new())),
        Space::Rgb if bpc >= 8 => Some((Color::Rgb, Vec::new())),
        Space::Indexed(pal) if bpc <= 8 => Some((Color::Palette, fit_palette(pal.clone(), bpc))),
        _ => None,
    };
    match native {
        Some((color, palette)) => {
            let spec = Spec { width: w, height: h, depth: bpc, color, palette };
            // Ya se comprobo que los datos cubren al menos la mitad: el relleno queda acotado.
            let need = png::row_bytes(&spec).and_then(|r| r.checked_mul(h as usize)).ok_or("Imagen demasiado grande para extraerla.")?;
            data.resize(need, 0);
            // Invertir todos los bits invierte cada muestra: v -> maximo - v.
            let flip = (space == Space::Tint) != invert && color == Color::Gray;
            if flip {
                data.iter_mut().for_each(|b| *b = !*b);
            }
            encode(&spec, &data)
        }
        None => {
            let (pixels, color) = to_8bit(&space, &data, w, h, bpc, invert)?;
            encode(&Spec { width: w, height: h, depth: 8, color, palette: Vec::new() }, &pixels)
        }
    }
}

fn encode(spec: &Spec, pixels: &[u8]) -> Result<Vec<u8>, String> {
    png::encode(spec, pixels).ok_or_else(|| "Los datos de la imagen estan incompletos.".to_string())
}

impl PartialEq for Space {
    fn eq(&self, other: &Space) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// TIFF de una tira con los datos de fax tal cual (Grupo 3 o 4 segun `K`).
fn ccitt_tiff(pdf: &Pdf, data: &[u8], w: u32, h: u32, parms: Option<&Dict>, invert: bool) -> Vec<u8> {
    let get = |k: &str| parms.and_then(|p| pdf.get(p, k).as_i64());
    let on = |k: &str| parms.map_or(false, |p| flag(pdf, p, k));
    let k = get("K").unwrap_or(0);
    let compression: u32 = if k < 0 { 4 } else { 3 };
    // En PDF, sin BlackIs1 los bits 0 son negros: en TIFF eso es "BlackIsZero" (1).
    let black_is_zero = !on("BlackIs1") != invert;
    let mut entries: Vec<(u16, u16, u32)> = vec![
        (256, 4, w),
        (257, 4, h),
        (258, 3, 1),
        (259, 3, compression),
        (262, 3, black_is_zero as u32),
        (273, 4, 0), // desplazamiento de la tira: se completa abajo
        (277, 3, 1),
        (278, 4, h),
        (279, 4, data.len() as u32),
    ];
    if compression == 3 {
        entries.push((292, 4, (k > 0) as u32 | if on("EncodedByteAlign") { 4 } else { 0 }));
    } else {
        entries.push((293, 4, 0));
    }
    let offset = 8 + 2 + 12 * entries.len() as u32 + 4;
    let mut out = Vec::with_capacity(offset as usize + data.len());
    out.extend_from_slice(b"II*\0");
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, kind, value) in entries {
        let value = if tag == 273 { offset } else { value };
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn muestras() {
        assert_eq!(sample(&[0b1011_0010], 0, 1), 1);
        assert_eq!(sample(&[0b1011_0010], 1, 1), 0);
        assert_eq!(sample(&[0b1011_0010], 1, 2), 0b11);
        assert_eq!(sample(&[0xAB, 0xCD], 1, 4), 0xB);
        assert_eq!(sample(&[0x12, 0x34], 0, 16), 0x1234);
        assert_eq!(scale(3, 2), 255);
        assert_eq!(scale(1, 1), 255);
        assert_eq!(cmyk([0, 0, 0, 0]), [255, 255, 255]);
        assert_eq!(cmyk([255, 0, 0, 0]), [0, 255, 255]);
    }

    #[test]
    fn ubicacion() {
        // Imagen de 100 x 50 puntos dibujada en (72, 600) en una pagina carta vertical.
        let m = [100.0, 0.0, 0.0, 50.0, 72.0, 600.0];
        assert_eq!(place(m, [0.0, 0.0, 612.0, 792.0], 0), (72.0, 142.0, 100.0, 50.0));
        // La misma pagina girada 90 grados: se lee apaisada.
        let (x, y, w, h) = place(m, [0.0, 0.0, 612.0, 792.0], 90);
        assert_eq!((w, h), (50.0, 100.0));
        assert_eq!((x, y), (600.0, 72.0));
    }
}
