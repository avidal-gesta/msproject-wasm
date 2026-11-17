//! Decodificador de fax CCITT (UIT-T T.4 y T.6): grupo 3 de una y dos dimensiones y grupo 4.
//!
//! Es el filtro `CCITTFaxDecode` de PDF y el modo MMR de JBIG2. Devuelve filas de bits
//! empaquetadas (la fila completa en bytes, el bit mas significativo primero) con 1 = negro;
//! quien lo usa invierte si necesita otra convencion. Un archivo danado no es un error: se
//! entrega lo decodificado hasta ahi y el resto de las filas queda en blanco. Si es un error un
//! tamano declarado fuera de rango: la memoria se reserva segun las columnas y filas declaradas.
//!
//! Escrito a partir de las recomendaciones UIT-T T.4 (codigos de largo de corrida, tablas 2 y 3,
//! y codificacion de dos dimensiones, 4.2) y T.6 (grupo 4).

/// Tope de columnas: ningun escaneo real se acerca (una hoja A0 a 1.200 dpi tiene unas 40.000).
pub const MAX_COLUMNS: usize = 1 << 20;
/// Tope de pixeles (columnas por filas) de una decodificacion: 25 MB de filas empaquetadas.
pub const MAX_PIXELS: u64 = 200_000_000;

/// Parametros de `/DecodeParms` (con sus valores por defecto de PDF).
#[derive(Clone, Debug)]
pub struct Params {
    /// < 0: grupo 4; 0: grupo 3 de una dimension; > 0: grupo 3 con filas de dos dimensiones.
    pub k: i64,
    pub columns: usize,
    /// 0 = hasta que se acaben los datos.
    pub rows: usize,
    /// Cada fila (grupo 4) o cada fin de linea (grupo 3) empieza en un byte nuevo.
    pub byte_align: bool,
    /// Se espera la marca de fin de bloque (si falta, igual se termina con los datos).
    pub end_of_block: bool,
}

impl Default for Params {
    fn default() -> Params {
        Params { k: 0, columns: 1728, rows: 0, byte_align: false, end_of_block: true }
    }
}

// Codigos de largo de corrida (T.4, tablas 2 y 3), como cadenas de bits.
const WHITE_TERM: [&str; 64] = [
    "00110101", "000111", "0111", "1000", "1011", "1100", "1110", "1111", "10011", "10100", "00111", "01000", "001000", "000011", "110100", "110101", "101010",
    "101011", "0100111", "0001100", "0001000", "0010111", "0000011", "0000100", "0101000", "0101011", "0010011", "0100100", "0011000", "00000010", "00000011",
    "00011010", "00011011", "00010010", "00010011", "00010100", "00010101", "00010110", "00010111", "00101000", "00101001", "00101010", "00101011", "00101100",
    "00101101", "00000100", "00000101", "00001010", "00001011", "01010010", "01010011", "01010100", "01010101", "00100100", "00100101", "01011000", "01011001",
    "01011010", "01011011", "01001010", "01001011", "00110010", "00110011", "00110100",
];
const WHITE_MAKEUP: [&str; 27] = [
    "11011", "10010", "010111", "0110111", "00110110", "00110111", "01100100", "01100101", "01101000", "01100111", "011001100", "011001101", "011010010",
    "011010011", "011010100", "011010101", "011010110", "011010111", "011011000", "011011001", "011011010", "011011011", "010011000", "010011001", "010011010",
    "011000", "010011011",
];
const BLACK_TERM: [&str; 64] = [
    "0000110111", "010", "11", "10", "011", "0011", "0010", "00011", "000101", "000100", "0000100", "0000101", "0000111", "00000100", "00000111", "000011000",
    "0000010111", "0000011000", "0000001000", "00001100111", "00001101000", "00001101100", "00000110111", "00000101000", "00000010111", "00000011000",
    "000011001010", "000011001011", "000011001100", "000011001101", "000001101000", "000001101001", "000001101010", "000001101011", "000011010010",
    "000011010011", "000011010100", "000011010101", "000011010110", "000011010111", "000001101100", "000001101101", "000011011010", "000011011011",
    "000001010100", "000001010101", "000001010110", "000001010111", "000001100100", "000001100101", "000001010010", "000001010011", "000000100100",
    "000000110111", "000000111000", "000000100111", "000000101000", "000001011000", "000001011001", "000000101011", "000000101100", "000001011010",
    "000001100110", "000001100111",
];
const BLACK_MAKEUP: [&str; 27] = [
    "0000001111", "000011001000", "000011001001", "000001011011", "000000110011", "000000110100", "000000110101", "0000001101100", "0000001101101",
    "0000001001010", "0000001001011", "0000001001100", "0000001001101", "0000001110010", "0000001110011", "0000001110100", "0000001110101", "0000001110110",
    "0000001110111", "0000001010010", "0000001010011", "0000001010100", "0000001010101", "0000001011010", "0000001011011", "0000001100100", "0000001100101",
];
/// Corridas largas (1792 a 2560), iguales para blanco y negro.
const EXTENDED_MAKEUP: [&str; 13] = [
    "00000001000", "00000001100", "00000001101", "000000010010", "000000010011", "000000010100", "000000010101", "000000010110", "000000010111", "000000011100",
    "000000011101", "000000011110", "000000011111",
];

const PEEK: u32 = 13;

/// Tabla de busqueda por los proximos 13 bits: (largo de corrida, bits del codigo); 0 bits = invalido.
type Table = Vec<(u16, u8)>;

fn table(term: &[&str], makeup: &[&str]) -> Table {
    let mut t = vec![(0u16, 0u8); 1 << PEEK];
    let mut put = |code: &str, run: u16| {
        let len = code.len() as u32;
        let value = u32::from_str_radix(code, 2).unwrap_or(0);
        let start = (value << (PEEK - len)) as usize;
        for e in &mut t[start..start + (1 << (PEEK - len))] {
            debug_assert_eq!(e.1, 0, "codigo repetido: {}", code);
            *e = (run, len as u8);
        }
    };
    for (run, code) in term.iter().enumerate() {
        put(code, run as u16);
    }
    for (i, code) in makeup.iter().enumerate() {
        put(code, 64 * (i as u16 + 1));
    }
    for (i, code) in EXTENDED_MAKEUP.iter().enumerate() {
        put(code, 1792 + 64 * i as u16);
    }
    t
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn peek(&self, n: u32) -> u32 {
        let mut v = 0u32;
        for k in 0..n as usize {
            let p = self.pos + k;
            let bit = self.data.get(p / 8).map_or(0, |b| (b >> (7 - p % 8)) & 1);
            v = (v << 1) | bit as u32;
        }
        v
    }

    fn skip(&mut self, n: usize) {
        self.pos += n;
    }

    fn done(&self) -> bool {
        self.pos >= self.data.len() * 8
    }

    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }

    /// Salta un fin de linea (11 ceros y un uno, con ceros de relleno delante) si viene uno.
    fn eol(&mut self) -> bool {
        let start = self.pos;
        let mut zeros = 0;
        while !self.done() && self.peek(1) == 0 {
            self.skip(1);
            zeros += 1;
        }
        if zeros >= 11 && self.peek(1) == 1 {
            self.skip(1);
            return true;
        }
        self.pos = start;
        false
    }
}

enum Mode {
    Pass,
    Horizontal,
    Vertical(i64),
    Eol,
}

struct Decoder<'a> {
    bits: Bits<'a>,
    white: Table,
    black: Table,
    columns: usize,
}

impl Decoder<'_> {
    fn run(&mut self, black: bool) -> Option<usize> {
        let mut total = 0usize;
        loop {
            let (run, len) = if black { self.black[self.bits.peek(PEEK) as usize] } else { self.white[self.bits.peek(PEEK) as usize] };
            if len == 0 || self.bits.done() {
                return None;
            }
            self.bits.skip(len as usize);
            // Saturado: con codigos de relleno de 2.560 seguidos, en wasm32 la suma desbordaria.
            total = total.saturating_add(run as usize);
            if run < 64 {
                return Some(total);
            }
        }
    }

    fn mode(&mut self) -> Option<Mode> {
        let b = &mut self.bits;
        if b.done() {
            return None;
        }
        let v = b.peek(7);
        let (mode, len) = if v >> 6 == 1 {
            (Mode::Vertical(0), 1)
        } else if v >> 4 == 0b011 {
            (Mode::Vertical(1), 3)
        } else if v >> 4 == 0b010 {
            (Mode::Vertical(-1), 3)
        } else if v >> 4 == 0b001 {
            (Mode::Horizontal, 3)
        } else if v >> 3 == 0b0001 {
            (Mode::Pass, 4)
        } else if v >> 1 == 0b000011 {
            (Mode::Vertical(2), 6)
        } else if v >> 1 == 0b000010 {
            (Mode::Vertical(-2), 6)
        } else if v == 0b0000011 {
            (Mode::Vertical(3), 7)
        } else if v == 0b0000010 {
            (Mode::Vertical(-3), 7)
        } else if b.peek(12) == 1 {
            (Mode::Eol, 12)
        } else {
            // Modo sin compresion (0000001xxx) u otro codigo invalido.
            return None;
        };
        b.skip(len);
        Some(mode)
    }

    /// Fila de una dimension: corridas alternadas desde blanco. Devuelve los cambios de color.
    fn row_1d(&mut self) -> Option<Vec<usize>> {
        let mut changes = Vec::new();
        let (mut pos, mut black) = (0usize, false);
        while pos < self.columns {
            pos = pos.saturating_add(self.run(black)?).min(self.columns);
            changes.push(pos);
            black = !black;
        }
        Some(changes)
    }

    /// Fila de dos dimensiones respecto de la anterior (`reference`, con dos centinelas al final).
    fn row_2d(&mut self, reference: &[usize]) -> Option<Vec<usize>> {
        let cols = self.columns as i64;
        let mut changes: Vec<usize> = Vec::new();
        let mut a0: i64 = -1;
        let mut black = false;
        let mut bi = 0usize;
        while a0 < cols {
            // b1: primer cambio de la fila de referencia a la derecha de a0 y del color contrario
            // (los cambios en posicion par van de blanco a negro).
            while bi > 0 && reference[bi - 1] as i64 > a0 {
                bi -= 1;
            }
            while bi < reference.len() - 2 && (reference[bi] as i64 <= a0 || (bi % 2 == 1) != black) {
                bi += 1;
            }
            let b1 = reference[bi] as i64;
            let b2 = reference[(bi + 1).min(reference.len() - 1)] as i64;
            let push = |changes: &mut Vec<usize>, at: i64| {
                let at = at.clamp(0, cols) as usize;
                let last = changes.last().copied().unwrap_or(0);
                changes.push(at.max(last));
            };
            match self.mode()? {
                Mode::Pass => a0 = b2,
                Mode::Horizontal => {
                    let start = a0.max(0);
                    let a1 = start + self.run(black)? as i64;
                    let a2 = a1 + self.run(!black)? as i64;
                    push(&mut changes, a1);
                    push(&mut changes, a2);
                    a0 = a2;
                }
                Mode::Vertical(d) => {
                    let a1 = b1 + d;
                    push(&mut changes, a1);
                    a0 = a1.max(a0 + 1).min(cols.max(a1));
                    black = !black;
                }
                Mode::Eol => return None,
            }
        }
        Some(changes)
    }
}

/// Escribe una fila: pixeles negros en 1 entre cada par de cambios.
fn paint(row: &mut [u8], changes: &[usize], columns: usize) {
    let mut start = 0usize;
    for (i, &c) in changes.iter().enumerate() {
        let end = c.min(columns);
        if i % 2 == 1 {
            for x in start..end {
                row[x / 8] |= 0x80 >> (x % 8);
            }
        }
        start = end;
    }
    if changes.len() % 2 == 1 {
        for x in start..columns {
            row[x / 8] |= 0x80 >> (x % 8);
        }
    }
}

/// Decodifica hasta `height` filas (o `p.rows`). Devuelve las filas (1 = negro) y cuantas
/// se decodificaron de verdad (las demas quedan en blanco). Es un error que las columnas pasen de
/// `MAX_COLUMNS` o que columnas por filas pase de `MAX_PIXELS`.
pub fn decode(data: &[u8], p: &Params, height: usize) -> Result<(Vec<u8>, usize), String> {
    let columns = p.columns.max(1);
    let rows = if p.rows > 0 { p.rows } else { height };
    // En u64: en wasm32 el producto en usize puede desbordar y dar 0 (o poco), y la primera fila
    // quedaria fuera del arreglo.
    if columns > MAX_COLUMNS || (columns as u64).saturating_mul(rows as u64) > MAX_PIXELS {
        return Err(format!("Fax demasiado grande para decodificarlo ({} columnas, {} filas).", p.columns, rows));
    }
    let stride = columns.div_ceil(8);
    let mut out = vec![0u8; stride * rows];
    let mut d = Decoder { bits: Bits { data, pos: 0 }, white: table(&WHITE_TERM, &WHITE_MAKEUP), black: table(&BLACK_TERM, &BLACK_MAKEUP), columns };
    // Fila de referencia inicial: toda blanca.
    let mut reference = vec![columns, columns, columns];
    let mut decoded = 0;
    for y in 0..rows {
        if d.bits.done() {
            break;
        }
        let changes = if p.k < 0 {
            if p.byte_align {
                d.bits.align();
            }
            // Fin de bloque (dos fines de linea): no hay mas filas.
            if d.bits.peek(12) == 1 {
                break;
            }
            d.row_2d(&reference)
        } else {
            if p.byte_align {
                // El relleno va antes del fin de linea, para que este termine en un borde de byte.
                let start = d.bits.pos;
                let mut probe = Bits { data, pos: start };
                while !probe.done() && probe.peek(1) == 0 {
                    probe.skip(1);
                }
                if probe.pos - start >= 11 && probe.peek(1) == 1 {
                    d.bits.pos = probe.pos + 1;
                } else {
                    d.bits.align();
                }
            } else {
                d.bits.eol();
            }
            // Un segundo fin de linea seguido marca el fin del bloque.
            if p.end_of_block && y > 0 && d.bits.peek(12) == 1 {
                break;
            }
            let two_d = p.k > 0 && {
                let tag = d.bits.peek(1);
                d.bits.skip(1);
                tag == 0
            };
            if two_d {
                d.row_2d(&reference)
            } else {
                d.row_1d()
            }
        };
        let Some(changes) = changes else { break };
        paint(&mut out[y * stride..(y + 1) * stride], &changes, columns);
        reference.clear();
        reference.extend_from_slice(&changes);
        reference.extend_from_slice(&[columns, columns, columns]);
        decoded = y + 1;
    }
    Ok((out, decoded))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Escritor de bits para armar datos de prueba con los mismos codigos.
    struct W(Vec<bool>);
    impl W {
        fn s(&mut self, bits: &str) -> &mut W {
            self.0.extend(bits.chars().map(|c| c == '1'));
            self
        }
        fn run(&mut self, n: usize, black: bool) -> &mut W {
            let (term, makeup) = if black { (&BLACK_TERM, &BLACK_MAKEUP) } else { (&WHITE_TERM, &WHITE_MAKEUP) };
            let mut n = n;
            while n >= 64 {
                let m = (n / 64).min(27);
                self.s(makeup[m - 1]);
                n -= 64 * m;
            }
            self.s(term[n])
        }
        fn bytes(&self) -> Vec<u8> {
            self.0.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |b, (i, &v)| b | ((v as u8) << (7 - i)))).collect()
        }
    }

    #[test]
    fn codigos_sin_prefijos_repetidos() {
        // Cada entrada de la tabla de 13 bits queda tomada por un solo codigo.
        for (t, m) in [(&WHITE_TERM, &WHITE_MAKEUP), (&BLACK_TERM, &BLACK_MAKEUP)] {
            let all: Vec<&str> = t.iter().chain(m.iter()).chain(EXTENDED_MAKEUP.iter()).copied().collect();
            for (i, a) in all.iter().enumerate() {
                for b in &all[i + 1..] {
                    assert!(!a.starts_with(b) && !b.starts_with(a), "{} y {}", a, b);
                }
            }
        }
    }

    #[test]
    fn grupo3_una_dimension() {
        // 2 filas de 20: 5 blancos, 10 negros, 5 blancos; luego todo negro.
        let mut w = W(Vec::new());
        w.s("000000000001").run(5, false).run(10, true).run(5, false);
        w.s("000000000001").run(0, false).run(20, true);
        let (out, n) = decode(&w.bytes(), &Params { columns: 20, ..Params::default() }, 2).unwrap();
        assert_eq!(n, 2);
        assert_eq!(out, [0b0000_0111, 0b1111_1110, 0b0000_0000, 0xFF, 0xFF, 0xF0]);
    }

    #[test]
    fn grupo4_dos_dimensiones() {
        // Fila 1 (horizontal): 3 blancos, 4 negros, resto blanco. Fila 2: igual que la anterior
        // (V0, V0). Fila 3: el bloque negro se corre uno a la derecha (VR1, VR1). Fila 4: paso
        // sobre el bloque y todo blanco (P y luego V0 al final).
        let mut w = W(Vec::new());
        w.s("001").run(3, false).run(4, true).s("001").run(9, false).run(0, true);
        w.s("1").s("1").s("1");
        w.s("011").s("011").s("1");
        w.s("0001").s("1");
        w.s("000000000001000000000001");
        let (out, n) = decode(&w.bytes(), &Params { k: -1, columns: 16, ..Params::default() }, 4).unwrap();
        assert_eq!(n, 4);
        assert_eq!(out, [0b0001_1110, 0, 0b0001_1110, 0, 0b0000_1111, 0, 0, 0]);
    }

    #[test]
    fn corridas_largas_y_datos_cortados() {
        let mut w = W(Vec::new());
        // 2000 blancos con el codigo extendido de 1984 mas 16; luego 48 negros.
        w.s(EXTENDED_MAKEUP[3]).run(16, false).run(48, true);
        let (out, n) = decode(&w.bytes(), &Params { columns: 2048, ..Params::default() }, 3).unwrap();
        assert_eq!(n, 1);
        assert_eq!(out.len(), 3 * 256);
        assert_eq!(&out[249..256], &[0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert!(out[256..].iter().all(|&b| b == 0));
    }

    #[test]
    fn tamanos_declarados_fuera_de_rango() {
        let data = [0xFF, 0xFF];
        let g4 = |columns: usize, rows: usize| Params { k: -1, columns, rows, ..Params::default() };
        // En 64 bits el producto desbordaba; en wasm32, 2^31 columnas por 16 filas daba 0 bytes y
        // la primera fila quedaba fuera del arreglo.
        for (columns, rows) in [(1usize << 62, 16usize), (1 << 31, 16), (MAX_COLUMNS + 1, 1), (MAX_COLUMNS, 1000)] {
            let e = decode(&data, &g4(columns, rows), rows).unwrap_err();
            assert!(e.contains("demasiado grande"), "{} x {}: {}", columns, rows, e);
        }
        // Sin /Rows, el alto pedido por quien llama tambien cuenta.
        assert!(decode(&data, &g4(1000, 0), usize::MAX).is_err());
        // Dentro del tope, una hoja A3 a 600 dpi se puede decodificar.
        let (out, n) = decode(&data, &g4(7016, 9921), 9921).unwrap();
        assert_eq!((out.len(), n > 0), (877 * 9921, true));
    }

    /// Generador pseudoaleatorio fijo (xorshift), para que las pruebas se repitan igual.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }
    }

    #[test]
    fn datos_alterados_no_rompen_ni_demoran() {
        // Filas al azar codificadas en grupo 3 (una dimension, con fin de linea) y en grupo 4 (solo
        // modo horizontal); luego bytes cambiados, cortes y parametros al azar.
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut seeds = Vec::new();
        for _ in 0..12 {
            let columns = 1 + rng.below(300) as usize;
            let rows = 1 + rng.below(40) as usize;
            let (mut g3, mut g4) = (W(Vec::new()), W(Vec::new()));
            for _ in 0..rows {
                g3.s("000000000001");
                let mut pos = 0;
                let mut black = false;
                while pos < columns {
                    let run = (1 + rng.below(40) as usize).min(columns - pos);
                    g3.run(run, black);
                    pos += run;
                    black = !black;
                }
                if black {
                    g3.run(0, true);
                }
                let mut pos = 0;
                while pos < columns {
                    let white = (rng.below(30) as usize).min(columns - pos);
                    let black = (rng.below(30) as usize).min(columns - pos - white);
                    g4.s("001").run(white, false).run(black, true);
                    pos += white + black;
                    if white + black == 0 {
                        break;
                    }
                }
            }
            g4.s("000000000001000000000001");
            seeds.push((g3.bytes(), Params { columns, ..Params::default() }, rows));
            seeds.push((g4.bytes(), Params { k: -1, columns, ..Params::default() }, rows));
        }
        for case in 0..4000 {
            let (base, p, rows) = &seeds[rng.below(seeds.len() as u64) as usize];
            let mut data = if case % 10 == 0 { (0..rng.below(64)).map(|_| rng.next() as u8).collect() } else { base.clone() };
            for _ in 0..rng.below(5) {
                if !data.is_empty() {
                    let i = rng.below(data.len() as u64) as usize;
                    data[i] = if rng.below(2) == 0 { data[i] ^ (1 << rng.below(8)) } else { rng.next() as u8 };
                }
            }
            if rng.below(4) == 0 {
                data.truncate(rng.below(data.len() as u64 + 1) as usize);
            }
            let mut p = p.clone();
            if rng.below(4) == 0 {
                p.k = [-1, 0, 1, 4][rng.below(4) as usize];
                p.byte_align = rng.below(2) == 0;
                p.end_of_block = rng.below(2) == 0;
                p.columns = rng.below(5000) as usize;
                p.rows = rng.below(3000) as usize;
            }
            let start = std::time::Instant::now();
            let _ = decode(&data, &p, *rows + rng.below(3) as usize);
            assert!(start.elapsed().as_secs() < 2, "caso {}: demasiado lento", case);
        }
    }
}
