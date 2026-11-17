//! Escritor PNG minimo, para entregar como PNG las imagenes de PDF que vienen en pixeles.
//!
//! El compresor DEFLATE es el de `src/zip.rs` de mpp2xlsx (LZ77 + codigos Huffman fijos), con la
//! tabla de cadenas acotada a la ventana de 32 KB para que la memoria no crezca con la imagen.
//! Cuando el PDF ya trae los pixeles comprimidos como un PNG (Flate con predictor PNG), se copian
//! tal cual sin descomprimir ni volver a comprimir.

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

fn crc32(parts: &[&[u8]]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for part in parts {
        for &b in *part {
            c = CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
        }
    }
    c ^ 0xFFFF_FFFF
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    fn bits(&mut self, value: u32, count: u32) {
        self.acc |= (value as u64) << self.n;
        self.n += count;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    fn code(&mut self, code: u32, len: u32) {
        self.bits(code.reverse_bits() >> (32 - len), len);
    }

    fn symbol(&mut self, sym: u32) {
        match sym {
            0..=143 => self.code(0x30 + sym, 8),
            144..=255 => self.code(0x190 + sym - 144, 9),
            256..=279 => self.code(sym - 256, 7),
            _ => self.code(0xC0 + sym - 280, 8),
        }
    }
}

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

const WINDOW: usize = 32768;
const HASH_BITS: u32 = 15;
const MAX_CHAIN: usize = 16;
const MAX_MATCH: usize = 258;
const NICE_MATCH: usize = 64;

fn hash3(d: &[u8], i: usize) -> usize {
    let v = (d[i] as u32) | (d[i + 1] as u32) << 8 | (d[i + 2] as u32) << 16;
    (v.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
}

/// DEFLATE con codigos Huffman fijos. Memoria fija: tabla de hash y cadenas de 32 KB.
fn deflate(d: &[u8]) -> Vec<u8> {
    let mut w = BitWriter { out: Vec::with_capacity(d.len() / 4 + 64), acc: 0, n: 0 };
    w.bits(1, 1);
    w.bits(1, 2);
    let n = d.len();
    // Posicion + 1 de la ultima aparicion de cada hash, y de la anterior con el mismo hash.
    let mut head = vec![0u32; 1 << HASH_BITS];
    let mut prev = vec![0u32; WINDOW];
    let insert = |head: &mut Vec<u32>, prev: &mut Vec<u32>, j: usize| {
        let h = hash3(d, j);
        prev[j % WINDOW] = head[h];
        head[h] = j as u32 + 1;
    };
    let mut i = 0usize;
    while i < n {
        let mut best_len = 0usize;
        let mut best_dist = 0usize;
        if i + 3 <= n {
            let mut cand = head[hash3(d, i)] as usize;
            let limit = (n - i).min(MAX_MATCH);
            let mut chain = 0;
            while cand != 0 && chain < MAX_CHAIN {
                let c = cand - 1;
                if c >= i || i - c > WINDOW {
                    break;
                }
                // La entrada de c sigue vigente: solo se reemplaza al insertar c + 32 KB, que aun no llega.
                cand = prev[c % WINDOW] as usize;
                chain += 1;
                if d[c + best_len] != d[i + best_len] {
                    continue;
                }
                let mut l = 0usize;
                while l < limit && d[c + l] == d[i + l] {
                    l += 1;
                }
                if l > best_len {
                    best_len = l;
                    best_dist = i - c;
                    if l == limit || l >= NICE_MATCH {
                        break;
                    }
                }
            }
            insert(&mut head, &mut prev, i);
        }
        if best_len >= 3 {
            let li = LEN_BASE.iter().rposition(|&b| b as usize <= best_len).unwrap_or(0);
            w.symbol(257 + li as u32);
            w.bits((best_len - LEN_BASE[li] as usize) as u32, LEN_EXTRA[li] as u32);
            let di = DIST_BASE.iter().rposition(|&b| b as usize <= best_dist).unwrap_or(0);
            w.code(di as u32, 5);
            w.bits((best_dist - DIST_BASE[di] as usize) as u32, DIST_EXTRA[di] as u32);
            for j in i + 1..i + best_len {
                if j + 3 <= n {
                    insert(&mut head, &mut prev, j);
                }
            }
            i += best_len;
        } else {
            w.symbol(d[i] as u32);
            i += 1;
        }
    }
    w.symbol(256);
    if w.n > 0 {
        w.bits(0, 8 - w.n);
    }
    w.out
}

/// Datos zlib (RFC 1950) de `d`.
fn zlib(d: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    out.extend_from_slice(&deflate(d));
    out.extend_from_slice(&adler32(d).to_be_bytes());
    out
}

/// Tipo de color de PNG.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Color {
    Gray = 0,
    Rgb = 2,
    Palette = 3,
    GrayAlpha = 4,
    Rgba = 6,
}

impl Color {
    pub fn channels(self) -> usize {
        match self {
            Color::Gray | Color::Palette => 1,
            Color::GrayAlpha => 2,
            Color::Rgb => 3,
            Color::Rgba => 4,
        }
    }
}

pub struct Spec {
    pub width: u32,
    pub height: u32,
    pub depth: u8,
    pub color: Color,
    /// Paleta RGB (3 bytes por entrada), solo con `Color::Palette`.
    pub palette: Vec<u8>,
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
}

/// Arma el PNG con los datos de IDAT ya comprimidos (zlib de filas con su byte de filtro).
pub fn from_idat(spec: &Spec, idat: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(idat.len() + spec.palette.len() + 64);
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&spec.width.to_be_bytes());
    ihdr.extend_from_slice(&spec.height.to_be_bytes());
    ihdr.extend_from_slice(&[spec.depth, spec.color as u8, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    if spec.color == Color::Palette {
        chunk(&mut out, b"PLTE", &spec.palette);
    }
    // IDAT en trozos de hasta 1 MB, como hacen los codificadores habituales.
    for part in idat.chunks(1 << 20) {
        chunk(&mut out, b"IDAT", part);
    }
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Bytes por fila sin el byte de filtro.
pub fn row_bytes(spec: &Spec) -> usize {
    (spec.width as usize * spec.color.channels() * spec.depth as usize).div_ceil(8)
}

/// Arma el PNG a partir de los pixeles sin comprimir (filas consecutivas, sin byte de filtro).
/// Con 8 o mas bits por muestra elige por fila el filtro que deja los valores mas chicos.
pub fn encode(spec: &Spec, pixels: &[u8]) -> Vec<u8> {
    let row = row_bytes(spec);
    let height = spec.height as usize;
    let bpp = (spec.color.channels() * spec.depth as usize).div_ceil(8);
    let adaptive = spec.depth >= 8 && spec.color != Color::Palette;
    let mut filtered = Vec::with_capacity((row + 1) * height);
    let zero = vec![0u8; row];
    let mut candidate = vec![0u8; row];
    let mut best = vec![0u8; row];
    for y in 0..height {
        let cur = pixels.get(y * row..(y + 1) * row).unwrap_or(&zero);
        let up = if y == 0 { &zero[..] } else { pixels.get((y - 1) * row..y * row).unwrap_or(&zero) };
        if !adaptive {
            filtered.push(0);
            filtered.extend_from_slice(cur);
            continue;
        }
        let mut best_kind = 0u8;
        let mut best_score = u64::MAX;
        for kind in 0..5u8 {
            for i in 0..row {
                let a = if i >= bpp { cur[i - bpp] } else { 0 };
                let b = up[i];
                let c = if i >= bpp { up[i - bpp] } else { 0 };
                candidate[i] = cur[i].wrapping_sub(match kind {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => ((a as u16 + b as u16) / 2) as u8,
                    _ => paeth(a, b, c),
                });
            }
            let score: u64 = candidate.iter().map(|&v| (v as i8).unsigned_abs() as u64).sum();
            if score < best_score {
                best_score = score;
                best_kind = kind;
                best.copy_from_slice(&candidate);
            }
        }
        filtered.push(best_kind);
        filtered.extend_from_slice(&best);
    }
    from_idat(spec, &zlib(&filtered))
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inflate::zlib as inflate;

    #[test]
    fn deflate_ida_y_vuelta() {
        let mut d = Vec::new();
        for i in 0..200_000u32 {
            d.push((i % 251) as u8 ^ (i / 7000) as u8);
        }
        d.extend(std::iter::repeat_n(7u8, 100_000));
        let z = zlib(&d);
        assert!(z.len() < d.len() / 3);
        assert_eq!(inflate(&z).data, d);
    }

    #[test]
    fn png_valido() {
        let spec = Spec { width: 3, height: 2, depth: 8, color: Color::Rgb, palette: Vec::new() };
        let pixels = [255, 0, 0, 0, 255, 0, 0, 0, 255, 10, 20, 30, 40, 50, 60, 70, 80, 90];
        let png = encode(&spec, &pixels);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(&png[12..16], b"IHDR");
        // Descomprime IDAT y deshace los filtros: deben volver los mismos pixeles.
        let idat_len = u32::from_be_bytes([png[33], png[34], png[35], png[36]]) as usize;
        assert_eq!(&png[37..41], b"IDAT");
        let raw = inflate(&png[41..41 + idat_len]).data;
        let mut prev = [0u8; 9];
        let mut out = Vec::new();
        for line in raw.chunks(10) {
            let mut cur = [0u8; 9];
            for i in 0..9 {
                let a = if i >= 3 { cur[i - 3] } else { 0 };
                let b = prev[i];
                let c = if i >= 3 { prev[i - 3] } else { 0 };
                cur[i] = line[1 + i].wrapping_add(match line[0] {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => ((a as u16 + b as u16) / 2) as u8,
                    _ => paeth(a, b, c),
                });
            }
            out.extend_from_slice(&cur);
            prev = cur;
        }
        assert_eq!(out, pixels);
        assert!(png.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]));
    }
}
