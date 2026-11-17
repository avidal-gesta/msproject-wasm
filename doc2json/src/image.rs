//! Imagenes de los documentos: registro sin repeticiones y reconocimiento del formato y del
//! tamano en pixeles a partir de los primeros bytes del archivo.

use crate::doc::{Image, Place};
use std::collections::HashMap;

/// Tope de imagenes distintas por documento: las siguientes no figuran en la lista.
pub const MAX_IMAGES: usize = 10_000;
/// Tope de lugares (veces que se dibujan) entre todas las imagenes del documento.
pub const MAX_PLACES: usize = 100_000;
/// Tope de bytes extraidos entre todas las imagenes del documento.
pub const MAX_EXTRACTED: usize = 128 << 20;

/// Imagenes del documento, una por archivo u objeto, en orden de aparicion.
pub struct ImageSet {
    pub list: Vec<Image>,
    /// Clave de cada imagen (ruta dentro del paquete u objeto de PDF), en el orden de `list`.
    pub keys: Vec<String>,
    index: HashMap<String, u32>,
    places: usize,
    max_images: usize,
    max_places: usize,
    /// Imagenes y lugares que no se registraron por los topes.
    dropped_images: usize,
    dropped_places: usize,
}

impl Default for ImageSet {
    fn default() -> Self {
        ImageSet::with_caps(MAX_IMAGES, MAX_PLACES)
    }
}

impl ImageSet {
    pub fn with_caps(max_images: usize, max_places: usize) -> Self {
        ImageSet { list: Vec::new(), keys: Vec::new(), index: HashMap::new(), places: 0, max_images, max_places, dropped_images: 0, dropped_places: 0 }
    }

    /// Devuelve el `id` de la imagen identificada por `key`; si es nueva, la crea con `make`.
    /// Agrega el lugar y el texto alternativo si la imagen aun no tenia uno. `None` si es nueva
    /// y ya se alcanzo el tope de imagenes.
    pub fn add(&mut self, key: &str, place: Option<Place>, alt: &str, make: impl FnOnce() -> Image) -> Option<u32> {
        let id = match self.index.get(key) {
            Some(&id) => id,
            None if self.list.len() >= self.max_images => {
                self.dropped_images += 1;
                return None;
            }
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
            if self.places < self.max_places {
                self.places += 1;
                img.places.push(p);
            } else {
                self.dropped_places += 1;
            }
        }
        Some(id)
    }

    /// Avisos por lo que no se registro al llegar a los topes.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.dropped_images > 0 {
            out.push(format!("El documento tiene mas de {} imagenes distintas: las siguientes no figuran en la lista.", self.max_images));
        }
        if self.dropped_places > 0 {
            out.push(format!("Las imagenes se dibujan mas de {} veces: {} lugares no figuran en la lista.", self.max_places, self.dropped_places));
        }
        out
    }
}

/// Clave de una parte de un paquete ZIP para reconocer la misma imagen: sin la barra inicial y
/// sin distinguir mayusculas, igual que `ZipArchive::find`.
pub fn part_key(path: &str) -> String {
    path.trim_start_matches('/').to_ascii_lowercase()
}

/// Bytes extraidos entre todas las imagenes de un documento, con su tope.
pub struct Budget {
    used: usize,
    limit: usize,
    skipped: usize,
}

impl Budget {
    pub fn new(limit: usize) -> Self {
        Budget { used: 0, limit, skipped: 0 }
    }

    /// Ya no cabe nada mas: las imagenes que faltan no se leen.
    pub fn exhausted(&self) -> bool {
        self.used >= self.limit
    }

    /// Entrega los bytes en la imagen si caben en el tope; si no, deja una nota.
    pub fn store(&mut self, img: &mut Image, data: Vec<u8>) {
        match self.used.checked_add(data.len()) {
            Some(total) if total <= self.limit => {
                self.used = total;
                img.data = Some(data);
            }
            _ => self.skip(img),
        }
    }

    /// La imagen no se extrae por el tope.
    pub fn skip(&mut self, img: &mut Image) {
        self.skipped += 1;
        img.data = None;
        img.note = Some(format!("No se extrae: se alcanzo el tope de {} de imagenes por documento.", mb(self.limit)));
    }

    pub fn warning(&self) -> Option<String> {
        (self.skipped > 0).then(|| format!("{} imagenes no se extrajeron: superan el tope de {} por documento.", self.skipped, mb(self.limit)))
    }
}

fn mb(bytes: usize) -> String {
    if bytes >= 1 << 20 {
        format!("{} MB", bytes >> 20)
    } else {
        format!("{} bytes", bytes)
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
        assert_eq!((a, b, a2), (Some(1), Some(2), Some(1)));
        assert_eq!(set.list[0].places.len(), 2);
        assert_eq!(set.list[0].alt, "grafico");
        assert!(set.warnings().is_empty());
    }

    #[test]
    fn topes_de_imagenes_lugares_y_bytes() {
        let mut set = ImageSet::with_caps(2, 3);
        let make = Image::default;
        let p = || Some(Place::default());
        assert_eq!(set.add("a", p(), "", make), Some(1));
        assert_eq!(set.add("b", p(), "", make), Some(2));
        assert_eq!(set.add("c", p(), "", make), None);
        assert_eq!(set.add("a", p(), "", make), Some(1));
        assert_eq!(set.add("a", p(), "", make), Some(1));
        assert_eq!((set.list.len(), set.list[0].places.len()), (2, 2));
        assert_eq!(set.warnings().len(), 2);

        let mut budget = Budget::new(10);
        let (mut x, mut y, mut z) = (Image::default(), Image::default(), Image::default());
        budget.store(&mut x, vec![0; 6]);
        budget.store(&mut y, vec![0; 6]);
        budget.store(&mut z, vec![0; 4]);
        assert_eq!((x.data.is_some(), y.data.is_some(), z.data.is_some()), (true, false, true));
        assert!(y.note.as_deref().unwrap().contains("tope"));
        assert!(budget.exhausted());
        assert!(budget.warning().unwrap().starts_with("1 imagenes"));
        assert_eq!(part_key("/word/MEDIA/Image1.PNG"), part_key("word/media/image1.png"));
    }
}
