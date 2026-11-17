//! Filtros de los flujos de PDF: FlateDecode (con predictores), LZW, ASCII85, ASCIIHex y
//! RunLength. Los filtros de imagen (DCT, JPX, CCITT, JBIG2) no se decodifican: no hay texto ahi.

use crate::inflate::{zlib, MAX_OUTPUT};

/// Parametros de un filtro (`/DecodeParms`).
#[derive(Clone, Copy)]
pub struct Parms {
    pub predictor: i64,
    pub colors: i64,
    pub bits: i64,
    pub columns: i64,
    pub early_change: i64,
}

impl Default for Parms {
    fn default() -> Parms {
        Parms { predictor: 1, colors: 1, bits: 8, columns: 1, early_change: 1 }
    }
}

/// Salida de un filtro que expande los datos, acotada por lo que quede del presupuesto de la
/// lectura (`budget`) y anotada en el.
fn budgeted(decode: impl FnOnce(usize) -> Vec<u8>) -> Vec<u8> {
    let cap = crate::budget::room(MAX_OUTPUT);
    let out = decode(cap);
    crate::budget::spend(out.len(), cap < MAX_OUTPUT && out.len() >= cap);
    out
}

/// Aplica un filtro. `None` si el filtro no esta soportado.
pub fn apply(name: &str, data: &[u8], p: &Parms) -> Option<Vec<u8>> {
    match name {
        "FlateDecode" | "Fl" => Some(predict(zlib(data).data, p)),
        "LZWDecode" | "LZW" => Some(predict(budgeted(|cap| lzw(data, p.early_change != 0, cap)), p)),
        "ASCII85Decode" | "A85" => Some(budgeted(|cap| ascii85(data, cap))),
        "ASCIIHexDecode" | "AHx" => Some(ascii_hex(data)),
        "RunLengthDecode" | "RL" => Some(budgeted(|cap| run_length(data, cap))),
        // Cifrado "identidad": no cambia los datos.
        "Crypt" => Some(data.to_vec()),
        _ => None,
    }
}

fn predict(data: Vec<u8>, p: &Parms) -> Vec<u8> {
    if p.predictor <= 1 {
        return data;
    }
    let colors = p.colors.clamp(1, 32) as usize;
    let bits = p.bits.clamp(1, 16) as usize;
    let columns = p.columns.clamp(1, 1 << 20) as usize;
    let bpp = ((colors * bits + 7) / 8).max(1);
    let row = (columns * colors * bits + 7) / 8;
    if p.predictor == 2 {
        // Predictor TIFF (solo componentes de 8 bits): cada byte es la diferencia con el pixel anterior.
        let mut out = data;
        if bits == 8 {
            for line in out.chunks_mut(row) {
                for i in bpp..line.len() {
                    line[i] = line[i].wrapping_add(line[i - bpp]);
                }
            }
        }
        return out;
    }
    // Predictores PNG: cada fila lleva delante un byte con el tipo de filtro.
    let mut out: Vec<u8> = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; row];
    for chunk in data.chunks(row + 1) {
        let kind = chunk[0];
        let mut cur = vec![0u8; row];
        let src = &chunk[1..];
        for i in 0..row.min(src.len()) {
            let a = if i >= bpp { cur[i - bpp] } else { 0 };
            let b = prev[i];
            let c = if i >= bpp { prev[i - bpp] } else { 0 };
            let x = src[i];
            cur[i] = match kind {
                1 => x.wrapping_add(a),
                2 => x.wrapping_add(b),
                3 => x.wrapping_add(((a as u16 + b as u16) / 2) as u8),
                4 => {
                    let (pa, pb, pc) = (b as i32 - c as i32, a as i32 - c as i32, a as i32 + b as i32 - 2 * c as i32);
                    let (pa, pb, pc) = (pa.abs(), pb.abs(), pc.abs());
                    x.wrapping_add(if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c })
                }
                _ => x,
            };
        }
        out.extend_from_slice(&cur[..row.min(src.len())]);
        prev = cur;
    }
    out
}

fn ascii_hex(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut high: Option<u8> = None;
    for &c in data {
        let v = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            b'>' => break,
            _ => continue,
        };
        match high.take() {
            Some(h) => out.push(h << 4 | v),
            None => high = Some(v),
        }
    }
    if let Some(h) = high {
        out.push(h << 4);
    }
    out
}

// Los filtros que expanden los datos dejan de decodificar al alcanzar `max` bytes de salida:
// un flujo malicioso de pocos KB no debe poder agotar la memoria.

fn ascii85(data: &[u8], max: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 4 / 5);
    let mut group = [0u32; 5];
    let mut n = 0usize;
    let mut i = 0usize;
    // Prefijo opcional "<~".
    if data.starts_with(b"<~") {
        i = 2;
    }
    while i < data.len() && out.len() < max {
        let c = data[i];
        i += 1;
        match c {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0, 0, 0, 0]),
            b'!'..=b'u' => {
                group[n] = (c - b'!') as u32;
                n += 1;
                if n == 5 {
                    let v = group.iter().fold(0u32, |acc, d| acc.wrapping_mul(85).wrapping_add(*d));
                    out.extend_from_slice(&v.to_be_bytes());
                    n = 0;
                }
            }
            _ => {}
        }
    }
    if n > 1 {
        // Grupo final incompleto: se completa con 'u' y se descartan los bytes de relleno.
        for g in group.iter_mut().skip(n) {
            *g = 84;
        }
        let v = group.iter().fold(0u32, |acc, d| acc.wrapping_mul(85).wrapping_add(*d));
        out.extend_from_slice(&v.to_be_bytes()[..n - 1]);
    }
    out
}

fn run_length(data: &[u8], max: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < data.len() && out.len() < max {
        let n = data[i] as usize;
        i += 1;
        if n == 128 {
            break;
        } else if n < 128 {
            let end = (i + n + 1).min(data.len());
            out.extend_from_slice(&data[i..end]);
            i = end;
        } else if i < data.len() {
            out.extend(std::iter::repeat(data[i]).take(257 - n));
            i += 1;
        }
    }
    out
}

fn lzw(data: &[u8], early: bool, max: usize) -> Vec<u8> {
    let mut out = Vec::new();
    // Tabla: cada entrada es (codigo previo, byte); las 256 primeras son los bytes mismos.
    let mut prefix: Vec<u16> = vec![0; 4096];
    let mut suffix: Vec<u8> = vec![0; 4096];
    let mut next = 258usize;
    let mut bits = 9u32;
    let mut acc = 0u32;
    let mut have = 0u32;
    let mut prev: Option<usize> = None;
    let mut stack: Vec<u8> = Vec::new();
    for &b in data {
        acc = acc << 8 | b as u32;
        have += 8;
        while have >= bits {
            let code = ((acc >> (have - bits)) & ((1 << bits) - 1)) as usize;
            have -= bits;
            if code == 256 {
                next = 258;
                bits = 9;
                prev = None;
                continue;
            }
            if code == 257 {
                return out;
            }
            // Reconstruye la cadena del codigo (al reves) en `stack`.
            let mut cur = code;
            let mut first;
            if code >= next {
                // Caso especial: el codigo aun no existe; es prev + primer byte de prev.
                match prev {
                    Some(p) => {
                        cur = p;
                        stack.clear();
                        let mut c = cur;
                        while c >= 258 {
                            stack.push(suffix[c]);
                            c = prefix[c] as usize;
                        }
                        first = c as u8;
                        stack.push(first);
                        stack.reverse();
                        stack.push(first);
                    }
                    None => return out,
                }
            } else {
                stack.clear();
                while cur >= 258 {
                    stack.push(suffix[cur]);
                    cur = prefix[cur] as usize;
                }
                first = cur as u8;
                stack.push(first);
                stack.reverse();
            }
            out.extend_from_slice(&stack);
            if out.len() >= max {
                return out;
            }
            first = stack[0];
            if let Some(p) = prev {
                if next < 4096 {
                    prefix[next] = p as u16;
                    suffix[next] = first;
                    next += 1;
                }
            }
            prev = Some(code);
            let limit = if early { next + 1 } else { next };
            bits = if limit >= 2048 { 12 } else if limit >= 1024 { 11 } else if limit >= 512 { 10 } else { 9 };
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtros_ascii() {
        assert_eq!(ascii_hex(b"48 6F 6c 61>"), b"Hola");
        // "Hola mundo" en ASCII85.
        assert_eq!(ascii85(b"87cURD]i,\"Ebo7~>", MAX_OUTPUT), b"Hello World");
        assert_eq!(ascii85(b"z~>", MAX_OUTPUT), [0, 0, 0, 0]);
        assert_eq!(run_length(&[2, b'a', b'b', b'c', 254, b'x', 128], MAX_OUTPUT), b"abcxxx");
    }

    #[test]
    fn tope_de_salida() {
        // Cada par (129, x) se expande a 128 bytes y cada 'z' a 4.
        let rl: Vec<u8> = std::iter::repeat([129, b'x']).take(100).flatten().collect();
        assert_eq!(run_length(&rl, MAX_OUTPUT).len(), 12_800);
        assert_eq!(run_length(&rl, 1000).len(), 1024);
        assert_eq!(ascii85(&[b'z'; 100], 10).len(), 12);
        // El ejemplo de `lzw_basico`, cortado al alcanzar el tope.
        let data = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(lzw(&data, true, 4), [45; 5]);
    }

    #[test]
    fn lzw_basico() {
        // Ejemplo de la especificacion PDF: 45 45 45 45 45 65 45 45 45 66.
        let data = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(lzw(&data, true, MAX_OUTPUT), [45, 45, 45, 45, 45, 65, 45, 45, 45, 66]);
    }

    #[test]
    fn predictor_png() {
        // Dos filas de 3 bytes con filtro "Up": la segunda suma a la primera.
        let p = Parms { predictor: 12, columns: 3, ..Parms::default() };
        assert_eq!(predict(vec![0, 1, 2, 3, 2, 1, 1, 1], &p), [1, 2, 3, 2, 3, 4]);
    }
}
