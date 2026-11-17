//! Descompresor DEFLATE (RFC 1951) con envoltura zlib opcional (RFC 1950).
//!
//! Lo usan los .docx (son un ZIP) y los PDF (filtro FlateDecode). Es tolerante: si los datos
//! estan truncados o danados devuelve lo que alcanzo a descomprimir y lo indica en `ok`.

pub struct Inflated {
    pub data: Vec<u8>,
    pub ok: bool,
}

/// Tope de salida, para que un archivo malicioso no agote la memoria.
pub const MAX_OUTPUT: usize = 256 << 20;

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    acc: u32,
    n: u32,
}

impl Bits<'_> {
    #[inline]
    fn bits(&mut self, count: u32) -> Option<u32> {
        while self.n < count {
            let b = *self.d.get(self.pos)?;
            self.pos += 1;
            self.acc |= (b as u32) << self.n;
            self.n += 8;
        }
        let v = self.acc & ((1u32 << count) - 1);
        self.acc >>= count;
        self.n -= count;
        Some(v)
    }

    fn align(&mut self) {
        self.acc = 0;
        self.n = 0;
    }
}

/// Tabla de codigos Huffman canonicos: cantidad de codigos por longitud y simbolos en orden.
struct Huffman {
    count: [u16; 16],
    symbol: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Huffman {
        let mut count = [0u16; 16];
        for &l in lengths {
            count[(l & 15) as usize] += 1;
        }
        count[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..16 {
            offs[i] = offs[i - 1] + count[i - 1];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                let o = &mut offs[(l & 15) as usize];
                if (*o as usize) < symbol.len() {
                    symbol[*o as usize] = sym as u16;
                }
                *o += 1;
            }
        }
        Huffman { count, symbol }
    }

    #[inline]
    fn decode(&self, b: &mut Bits) -> Option<u16> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for len in 1..16 {
            code |= b.bits(1)? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return self.symbol.get((index + (code - first)) as usize).copied();
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        None
    }
}

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
const CLEN_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

fn block(b: &mut Bits, out: &mut Vec<u8>, lit: &Huffman, dist: &Huffman, cap: usize) -> Option<()> {
    loop {
        let sym = lit.decode(b)? as usize;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Some(());
        } else {
            let i = sym - 257;
            if i >= 29 {
                return None;
            }
            let len = LEN_BASE[i] as usize + b.bits(LEN_EXTRA[i] as u32)? as usize;
            let d = dist.decode(b)? as usize;
            if d >= 30 {
                return None;
            }
            let distance = DIST_BASE[d] as usize + b.bits(DIST_EXTRA[d] as u32)? as usize;
            if distance > out.len() || out.len() + len > cap {
                return None;
            }
            let start = out.len() - distance;
            for k in 0..len {
                let v = out[start + k];
                out.push(v);
            }
        }
        if out.len() > cap {
            return None;
        }
    }
}

fn run(b: &mut Bits, out: &mut Vec<u8>, cap: usize) -> Option<()> {
    loop {
        let last = b.bits(1)?;
        match b.bits(2)? {
            0 => {
                b.align();
                let len = *b.d.get(b.pos)? as usize | (*b.d.get(b.pos + 1)? as usize) << 8;
                b.pos += 4;
                let chunk = b.d.get(b.pos..b.pos + len)?;
                if out.len() + len > cap {
                    return None;
                }
                out.extend_from_slice(chunk);
                b.pos += len;
            }
            1 => {
                let mut l = [8u8; 288];
                l[144..256].fill(9);
                l[256..280].fill(7);
                block(b, out, &Huffman::new(&l), &Huffman::new(&[5u8; 30]), cap)?;
            }
            2 => {
                let nlit = b.bits(5)? as usize + 257;
                let ndist = b.bits(5)? as usize + 1;
                let nclen = b.bits(4)? as usize + 4;
                if nlit > 286 || ndist > 30 {
                    return None;
                }
                let mut cl = [0u8; 19];
                for &o in CLEN_ORDER.iter().take(nclen) {
                    cl[o] = b.bits(3)? as u8;
                }
                let clh = Huffman::new(&cl);
                let mut lengths = vec![0u8; nlit + ndist];
                let mut i = 0usize;
                while i < nlit + ndist {
                    let sym = clh.decode(b)?;
                    let (value, repeat) = match sym {
                        0..=15 => (sym as u8, 1),
                        16 => {
                            if i == 0 {
                                return None;
                            }
                            (lengths[i - 1], 3 + b.bits(2)? as usize)
                        }
                        17 => (0, 3 + b.bits(3)? as usize),
                        _ => (0, 11 + b.bits(7)? as usize),
                    };
                    if i + repeat > nlit + ndist {
                        return None;
                    }
                    lengths[i..i + repeat].fill(value);
                    i += repeat;
                }
                block(b, out, &Huffman::new(&lengths[..nlit]), &Huffman::new(&lengths[nlit..]), cap)?;
            }
            _ => return None,
        }
        if last == 1 {
            return Some(());
        }
    }
}

/// Descomprime un flujo DEFLATE "crudo" (el de las entradas de un ZIP). La salida se corta en
/// `MAX_OUTPUT` o en lo que quede del presupuesto de la lectura en curso (`budget`), lo que sea menor.
pub fn inflate(d: &[u8]) -> Inflated {
    let cap = crate::budget::room(MAX_OUTPUT);
    let mut out = Vec::with_capacity(d.len().saturating_mul(3).min(cap));
    let mut b = Bits { d, pos: 0, acc: 0, n: 0 };
    let ok = run(&mut b, &mut out, cap).is_some();
    // Se corto por el presupuesto (no por un tope fijo ni por datos danados): se avisa.
    crate::budget::spend(out.len(), !ok && cap < MAX_OUTPUT && out.len() + 258 >= cap);
    Inflated { data: out, ok }
}

/// Descomprime un flujo zlib (el del filtro FlateDecode de PDF). Si no trae cabecera zlib
/// valida, lo intenta como DEFLATE crudo.
pub fn zlib(d: &[u8]) -> Inflated {
    if d.len() >= 2 && d[0] & 0x0F == 8 && ((d[0] as u32) << 8 | d[1] as u32) % 31 == 0 {
        let skip = if d[1] & 0x20 != 0 { 6 } else { 2 };
        let r = inflate(d.get(skip..).unwrap_or(&[]));
        if r.ok || !r.data.is_empty() {
            return r;
        }
    }
    inflate(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bloque_almacenado() {
        // 1 bloque final sin compresion con "hola".
        let d = [0x01, 0x04, 0x00, 0xFB, 0xFF, b'h', b'o', b'l', b'a'];
        let r = inflate(&d);
        assert!(r.ok);
        assert_eq!(r.data, b"hola");
    }

    #[test]
    fn huffman_fijo_y_zlib() {
        // zlib.compress(b"hola hola hola hola hola", 9)
        let d = [0x78, 0xDA, 0xCB, 0xC8, 0xCF, 0x49, 0x54, 0xC8, 0xC0, 0x24, 0x00, 0x6D, 0x1E, 0x08, 0xB5];
        let r = zlib(&d);
        assert!(r.ok);
        assert_eq!(r.data, b"hola hola hola hola hola");
    }

    #[test]
    fn huffman_dinamico() {
        // zlib.compress(("Mantenimiento de chancador primario, cambio de poste y limpieza de taza. " * 6 + "Fin.").encode(), 9)
        let d = [
            0x78, 0xDA, 0xED, 0xCC, 0xB1, 0x0D, 0x80, 0x30, 0x0C, 0x44, 0xD1, 0x55, 0x3C, 0x00, 0xCA, 0x18, 0x74, 0x19, 0xE2, 0x48, 0x2C, 0x71,
            0x12, 0xB1, 0xA3, 0xE0, 0x86, 0x4C, 0x8F, 0x60, 0x08, 0x2A, 0xDA, 0xFF, 0xA5, 0x97, 0x61, 0xA1, 0xC6, 0x46, 0xB5, 0x70, 0xA9, 0x2A,
            0x65, 0x87, 0x15, 0x54, 0x1F, 0xD2, 0x07, 0x1B, 0x06, 0x7D, 0x91, 0x82, 0xB6, 0xF1, 0xBD, 0xDD, 0xCF, 0x50, 0xB9, 0xE4, 0x60, 0xEB,
            0xD4, 0x89, 0xA7, 0x05, 0x26, 0x92, 0xE4, 0x1F, 0xFA, 0x0E, 0x5A, 0x69, 0xE9, 0x06, 0xC7, 0x65, 0xA0, 0x9A,
        ];
        let r = zlib(&d);
        assert!(r.ok);
        let esperado = "Mantenimiento de chancador primario, cambio de poste y limpieza de taza. ".repeat(6) + "Fin.";
        assert_eq!(r.data, esperado.as_bytes());
    }

    #[test]
    fn datos_truncados() {
        let d = [0x78, 0xDA, 0xCB, 0xC8];
        let r = zlib(&d);
        assert!(!r.ok);
    }
}
