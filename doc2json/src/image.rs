//! Imagenes de los documentos: registro sin repeticiones y reconocimiento del formato y del
//! tamano en pixeles a partir de los primeros bytes del archivo.

use crate::doc::{Image, Place};
use std::collections::HashMap;

/// Imagenes del documento, una por archivo u objeto, en orden de aparicion.
#[derive(Default)]
pub struct ImageSet {
    pub list: Vec<Image>,
    /// Clave de cada imagen (ruta dentro del paquete u objeto de PDF), en el orden de `list`.
    pub keys: Vec<String>,
    index: HashMap<String, u32>,
}

impl ImageSet {
    /// Devuelve el `id` de la imagen identificada por `key`; si es nueva, la crea con `make`.
    /// Agrega el lugar y el texto alternativo si la imagen aun no tenia uno.
    pub fn add(&mut self, key: &str, place: Option<Place>, alt: &str, make: impl FnOnce() -> Image) -> u32 {
        let id = match self.index.get(key) {
            Some(&id) => id,
            None => {
                let id = self.list.len() as u32 + 1;
                let mut img = make();
                img.id = id;
                self.list.push(img);
                self.keys.push(key.to_string());
                self.index.insert(key.to_string(), id);
                id
            }
        };
        let img = &mut self.list[id as usize - 1];
        if img.alt.is_empty() && !alt.trim().is_empty() {
            img.alt = alt.trim().to_string();
        }
        if let Some(p) = place {
            img.places.push(p);
        }
        id
    }
}

/// Formato por la extension del nombre (cuando no se leyeron los bytes).
pub fn format_from_name(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "png",
        "jpg" | "jpeg" | "jpe" | "jfif" => "jpeg",
        "gif" => "gif",
        "bmp" | "dib" => "bmp",
        "tif" | "tiff" => "tiff",
        "emf" => "emf",
        "wmf" => "wmf",
        "svg" => "svg",
        "webp" => "webp",
        "jp2" | "j2k" | "jpx" => "jp2",
        "wdp" | "jxr" | "hdp" => "jxr",
        _ => "desconocido",
    }
}

fn be16(d: &[u8], i: usize) -> Option<u32> {
    Some(u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?]) as u32)
}

fn le16(d: &[u8], i: usize) -> Option<u32> {
    Some(u16::from_le_bytes([*d.get(i)?, *d.get(i + 1)?]) as u32)
}

fn be32(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*d.get(i)?, *d.get(i + 1)?, *d.get(i + 2)?, *d.get(i + 3)?]))
}

fn le32(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*d.get(i)?, *d.get(i + 1)?, *d.get(i + 2)?, *d.get(i + 3)?]))
}

/// Formato por los primeros bytes del archivo.
pub fn sniff(d: &[u8]) -> Option<&'static str> {
    Some(if d.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if d.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpeg"
    } else if d.starts_with(b"GIF87a") || d.starts_with(b"GIF89a") {
        "gif"
    } else if d.starts_with(b"BM") && d.len() > 26 {
        "bmp"
    } else if d.starts_with(b"II*\0") || d.starts_with(b"MM\0*") {
        "tiff"
    } else if d.starts_with(&[1, 0, 0, 0]) && d.get(40..44) == Some(b" EMF") {
        "emf"
    } else if d.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) || d.starts_with(&[1, 0, 9, 0]) || d.starts_with(&[2, 0, 9, 0]) {
        "wmf"
    } else if d.starts_with(b"RIFF") && d.get(8..12) == Some(b"WEBP") {
        "webp"
    } else if d.starts_with(&[0, 0, 0, 0x0C, b'j', b'P', b' ', b' ']) || d.starts_with(&[0xFF, 0x4F, 0xFF, 0x51]) {
        "jp2"
    } else {
        let head = String::from_utf8_lossy(&d[..d.len().min(512)]).to_ascii_lowercase();
        if head.contains("<svg") {
            "svg"
        } else {
            return None;
        }
    })
}

/// Ancho y alto en pixeles, leidos de la cabecera.
pub fn dimensions(d: &[u8]) -> Option<(u32, u32)> {
    match sniff(d)? {
        "png" => Some((be32(d, 16)?, be32(d, 20)?)),
        "gif" => Some((le16(d, 6)?, le16(d, 8)?)),
        "bmp" => {
            let w = le32(d, 18)? as i32;
            let h = le32(d, 22)? as i32;
            Some((w.unsigned_abs(), h.unsigned_abs()))
        }
        "jpeg" => {
            // Recorre los segmentos hasta el que declara el cuadro (SOF0..SOF15, salvo DHT, JPG y DAC).
            let mut i = 2;
            while i + 9 < d.len() {
                if d[i] != 0xFF {
                    return None;
                }
                let marker = d[i + 1];
                if marker == 0xFF {
                    i += 1;
                    continue;
                }
                let len = be16(d, i + 2)? as usize;
                if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
                    return Some((be16(d, i + 7)?, be16(d, i + 5)?));
                }
                i += 2 + len;
            }
            None
        }
        "webp" => match d.get(12..16)? {
            b"VP8X" => {
                let w = u32::from_le_bytes([*d.get(24)?, *d.get(25)?, *d.get(26)?, 0]) + 1;
                let h = u32::from_le_bytes([*d.get(27)?, *d.get(28)?, *d.get(29)?, 0]) + 1;
                Some((w, h))
            }
            b"VP8L" => {
                let b = le32(d, 21)?;
                Some(((b & 0x3FFF) + 1, ((b >> 14) & 0x3FFF) + 1))
            }
            b"VP8 " => Some((le16(d, 26)? & 0x3FFF, le16(d, 28)? & 0x3FFF)),
            _ => None,
        },
        _ => None,
    }
}

/// Completa formato y tamano de una imagen a partir de sus bytes.
pub fn describe(img: &mut Image, data: &[u8]) {
    if let Some(f) = sniff(data) {
        img.format = f;
    }
    if let Some((w, h)) = dimensions(data) {
        img.width = Some(w);
        img.height = Some(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cabeceras() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(sniff(&png), Some("png"));
        assert_eq!(dimensions(&png), Some((640, 480)));
        // JPEG: SOI, APP0 de 16 bytes y SOF0 con alto 300 y ancho 200.
        let mut jpg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
        jpg.extend_from_slice(&[0; 14]);
        jpg.extend_from_slice(&[0xFF, 0xC0, 0, 17, 8, 1, 44, 0, 200, 3, 0, 0, 0, 0, 0]);
        assert_eq!(dimensions(&jpg), Some((200, 300)));
        assert_eq!(dimensions(b"GIF89a\x10\0\x20\0"), Some((16, 32)));
        assert_eq!(format_from_name("word/media/image1.JPEG"), "jpeg");
        assert_eq!(format_from_name("x.emf"), "emf");
    }

    #[test]
    fn sin_repetir() {
        let mut set = ImageSet::default();
        let make = || Image { format: "png", ..Image::default() };
        let a = set.add("media/a.png", Some(Place { w: 10.0, h: 5.0, ..Place::default() }), "", make);
        let b = set.add("media/b.png", None, "logo", make);
        let a2 = set.add("media/a.png", Some(Place { w: 20.0, h: 10.0, ..Place::default() }), "grafico", make);
        assert_eq!((a, b, a2), (1, 2, 1));
        assert_eq!(set.list[0].places.len(), 2);
        assert_eq!(set.list[0].alt, "grafico");
    }
}
