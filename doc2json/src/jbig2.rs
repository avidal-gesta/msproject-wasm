//! Decodificador JBIG2 (UIT-T T.88) para el filtro `JBIG2Decode` de PDF: escaneos en blanco y
//! negro comprimidos.
//!
//! Cubre lo que usan los PDF reales: regiones genericas (aritmeticas, plantillas 0 a 3 con
//! prediccion tipica, o MMR), diccionarios de simbolos y regiones de texto aritmeticos, con
//! refinamiento y agregacion. No cubre la codificacion Huffman, los semitonos, las regiones
//! intermedias, la plantilla extendida (EXTTEMPLATE) ni los diccionarios que reutilizan los
//! contextos de otro: si aparecen, se informa en vez de entregar una imagen equivocada.
//!
//! Devuelve el mapa de bits de la pagina con 1 = negro, un byte por pixel.
//!
//! Escrito a partir de la recomendacion UIT-T T.88 (2000): decodificador aritmetico MQ con la
//! convencion de software del anexo E (E.3: INITDEC, BYTEIN, DECODE, RENORMD) y su tabla Qe
//! (tabla E.1), enteros y simbolos (anexo A), region generica (6.2), refinamiento (6.3), region
//! de texto (6.4), diccionario de simbolos (6.5) y sintaxis de segmentos (7). El modo MMR usa el
//! decodificador de fax de `ccitt.rs` (UIT-T T.6).
//!
//! Los datos vienen del archivo y no son confiables. Cada decodificacion tiene un presupuesto
//! (`Budget`) de pixeles reservados y de trabajo, proporcional al tamano de la pagina: una
//! cabecera de pocos bytes no puede pedir gigabytes ni horas de calculo.

use crate::ccitt;
use std::collections::HashMap;
use std::rc::Rc;

/// Tope de simbolos de un diccionario o de una region de texto. Los reales tienen unos miles; con
/// este tope los identificadores tienen a lo sumo 20 bits.
const MAX_SYMBOLS: usize = 1 << 20;

/// Lecturas permitidas despues del final de los datos (o de la marca de fin). Un flujo bien
/// formado solo necesita unas pocas; pasado esto, los datos se terminaron antes de tiempo.
const MQ_TAIL: u32 = 256;

const ERR_BUDGET: &str = "JBIG2: la imagen pide mas memoria o mas calculo que el permitido para su tamano.";
const ERR_END: &str = "JBIG2: los datos terminan antes de tiempo.";

/// Presupuesto de una decodificacion: pixeles que se pueden reservar (memoria, sumando todos los
/// mapas de bits) y trabajo (pixeles combinados, simbolos colocados, contextos inicializados).
struct Budget {
    pixels: u64,
    work: u64,
}

impl Budget {
    /// Para una pagina de `w` x `h`: la pagina, sus regiones y sus simbolos suelen sumar menos de
    /// dos veces la pagina; se permite el doble de eso, mas un margen para paginas chicas.
    fn for_page(w: usize, h: usize) -> Budget {
        let page = (w as u64).saturating_mul(h as u64);
        Budget { pixels: page.saturating_mul(4).saturating_add(16_000_000), work: page.saturating_mul(8).saturating_add(64_000_000) }
    }

    /// Reserva un mapa de bits. Cobra sus pixeles y tambien sus filas y columnas como trabajo: uno
    /// de 0 x 4.000 millones no ocupa memoria, pero recorrer sus filas tomaria minutos.
    fn bitmap(&mut self, w: usize, h: usize, fill: u8) -> Result<Bitmap, String> {
        let n = (w as u64).saturating_mul(h as u64);
        self.pixels = self.pixels.checked_sub(n).ok_or(ERR_BUDGET)?;
        self.spend((w as u64).saturating_add(h as u64))?;
        Bitmap::new(w, h, fill)
    }

    fn spend(&mut self, n: u64) -> Result<(), String> {
        self.work = self.work.checked_sub(n).ok_or(ERR_BUDGET)?;
        Ok(())
    }
}

/// Dimension de un mapa de bits calculada con datos del archivo: los negativos valen 0 y lo que
/// no cabe en 32 bits es un error (en wasm32, `as usize` lo truncaria a un valor cualquiera).
fn dim(v: i64) -> Result<usize, String> {
    if v > u32::MAX as i64 {
        return Err("JBIG2: tamano de simbolo fuera de rango.".into());
    }
    Ok(v.max(0) as usize)
}

// ---------------------------------------------------------------------------------------------
// Decodificador aritmetico MQ (anexo E)
// ---------------------------------------------------------------------------------------------

/// (Qe, siguiente si MPS, siguiente si LPS, intercambia MPS).
const QE: [(u16, u8, u8, bool); 47] = [
    (0x5601, 1, 1, true),
    (0x3401, 2, 6, false),
    (0x1801, 3, 9, false),
    (0x0AC1, 4, 12, false),
    (0x0521, 5, 29, false),
    (0x0221, 38, 33, false),
    (0x5601, 7, 6, true),
    (0x5401, 8, 14, false),
    (0x4801, 9, 14, false),
    (0x3801, 10, 14, false),
    (0x3001, 11, 17, false),
    (0x2401, 12, 18, false),
    (0x1C01, 13, 20, false),
    (0x1601, 29, 21, false),
    (0x5601, 15, 14, true),
    (0x5401, 16, 14, false),
    (0x5101, 17, 15, false),
    (0x4801, 18, 16, false),
    (0x3801, 19, 17, false),
    (0x3401, 20, 18, false),
    (0x3001, 21, 19, false),
    (0x2801, 22, 19, false),
    (0x2401, 23, 20, false),
    (0x2201, 24, 21, false),
    (0x1C01, 25, 22, false),
    (0x1801, 26, 23, false),
    (0x1601, 27, 24, false),
    (0x1401, 28, 25, false),
    (0x1201, 29, 26, false),
    (0x1101, 30, 27, false),
    (0x0AC1, 31, 28, false),
    (0x09C1, 32, 29, false),
    (0x08A1, 33, 30, false),
    (0x0521, 34, 31, false),
    (0x0441, 35, 32, false),
    (0x02A1, 36, 33, false),
    (0x0221, 37, 34, false),
    (0x0141, 38, 35, false),
    (0x0111, 39, 36, false),
    (0x0085, 40, 37, false),
    (0x0049, 41, 38, false),
    (0x0025, 42, 39, false),
    (0x0015, 43, 40, false),
    (0x0009, 44, 41, false),
    (0x0005, 45, 42, false),
    (0x0001, 45, 43, false),
    (0x5601, 46, 46, false),
];

/// Contexto: indice en `QE` (7 bits) y simbolo mas probable (bit 0).
type Cx = u8;

/// Decodificador MQ con la convencion de software de T.88 (E.3).
struct Mq<'a> {
    data: &'a [u8],
    bp: usize,
    /// Registro C de 32 bits (E.3.1): los 16 altos (Chigh) se comparan con Qe.
    c: u32,
    a: u32,
    ct: u32,
    /// Lecturas sin datos nuevos: en la marca de fin (0xFF seguido de un byte mayor que 0x8F) o
    /// pasado el final, donde BYTEIN agrega unos.
    tail: u32,
}

impl<'a> Mq<'a> {
    /// INITDEC (figura E.20).
    fn new(data: &'a [u8]) -> Mq<'a> {
        let mut m = Mq { data, bp: 0, c: 0, a: 0, ct: 0, tail: 0 };
        m.c = (m.byte(0) as u32) << 16;
        m.byte_in();
        m.c <<= 7;
        m.ct -= 7;
        m.a = 0x8000;
        m
    }

    fn byte(&self, i: usize) -> u8 {
        self.data.get(i).copied().unwrap_or(0xFF)
    }

    /// BYTEIN (figura E.19).
    fn byte_in(&mut self) {
        if self.byte(self.bp) == 0xFF {
            if self.byte(self.bp + 1) > 0x8F {
                self.c = self.c.wrapping_add(0xFF00);
                self.ct = 8;
                self.tail = self.tail.saturating_add(1);
            } else {
                self.bp += 1;
                self.c = self.c.wrapping_add((self.byte(self.bp) as u32) << 9);
                self.ct = 7;
            }
        } else {
            self.bp += 1;
            self.c = self.c.wrapping_add((self.byte(self.bp) as u32) << 8);
            self.ct = 8;
        }
    }

    /// Si ya se leyo bastante mas alla del final: los datos estaban cortados.
    fn exhausted(&self) -> bool {
        self.tail > MQ_TAIL
    }

    /// DECODE (figura E.15) con LPS_EXCHANGE y MPS_EXCHANGE (E.16 y E.17) y RENORMD (E.18).
    fn bit(&mut self, cx: &mut Cx) -> u32 {
        let (qe, nmps, nlps, switch) = QE[(*cx >> 1) as usize];
        let qe = qe as u32;
        let mps = (*cx & 1) as u32;
        let lps = |cx: &mut Cx| *cx = (nlps << 1) | if switch { 1 - mps } else { mps } as u8;
        let d;
        self.a = self.a.wrapping_sub(qe);
        if (self.c >> 16) < qe {
            // LPS_EXCHANGE: intercambio condicional en el subintervalo menos probable.
            if self.a < qe {
                d = mps;
                *cx = (nmps << 1) | mps as u8;
            } else {
                d = 1 - mps;
                lps(cx);
            }
            self.a = qe;
        } else {
            self.c -= qe << 16;
            if self.a & 0x8000 != 0 {
                return mps;
            }
            // MPS_EXCHANGE.
            if self.a < qe {
                d = 1 - mps;
                lps(cx);
            } else {
                d = mps;
                *cx = (nmps << 1) | mps as u8;
            }
        }
        loop {
            if self.ct == 0 {
                self.byte_in();
            }
            self.a <<= 1;
            self.c <<= 1;
            self.ct -= 1;
            if self.a & 0x8000 != 0 {
                break;
            }
        }
        d
    }
}

// ---------------------------------------------------------------------------------------------
// Enteros codificados (anexo A)
// ---------------------------------------------------------------------------------------------

/// Contextos de un decodificador de enteros (IAx).
struct Ia(Vec<Cx>);

impl Ia {
    fn new() -> Ia {
        Ia(vec![0; 512])
    }

    /// `None` = fuera de rango (OOB).
    fn decode(&mut self, mq: &mut Mq) -> Option<i64> {
        let mut prev = 1usize;
        let mut bit = |mq: &mut Mq| {
            let d = mq.bit(&mut self.0[prev]) as usize;
            prev = if prev < 256 { (prev << 1) | d } else { (((prev << 1) | d) & 511) | 256 };
            d
        };
        let s = bit(mq);
        let (n, offset) = if bit(mq) == 0 {
            (2, 0)
        } else if bit(mq) == 0 {
            (4, 4)
        } else if bit(mq) == 0 {
            (6, 20)
        } else if bit(mq) == 0 {
            (8, 84)
        } else if bit(mq) == 0 {
            (12, 340)
        } else {
            (32, 4436)
        };
        let mut v: i64 = 0;
        for _ in 0..n {
            v = (v << 1) | bit(mq) as i64;
        }
        v += offset;
        match (s, v) {
            (1, 0) => None,
            (1, v) => Some(-v),
            (_, v) => Some(v),
        }
    }
}

/// Identificadores de simbolo (IAID) de `len` bits.
struct Iaid {
    len: u32,
    cx: Vec<Cx>,
}

impl Iaid {
    /// `len` sale de `code_len` con a lo sumo `MAX_SYMBOLS` simbolos: 20 bits, 2 MB de contextos.
    /// Los contextos alcanzan para todo `prev` posible, sin recortar la tabla.
    fn new(len: u32) -> Iaid {
        debug_assert!(len <= 20);
        Iaid { len, cx: vec![0; 1 << (len + 1)] }
    }

    fn decode(&mut self, mq: &mut Mq) -> usize {
        let mut prev = 1usize;
        for _ in 0..self.len {
            let d = mq.bit(&mut self.cx[prev]) as usize;
            prev = (prev << 1) | d;
        }
        prev - (1 << self.len)
    }
}

// ---------------------------------------------------------------------------------------------
// Mapas de bits
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Bitmap {
    pub w: usize,
    pub h: usize,
    /// Un byte por pixel: 1 = negro.
    pub px: Vec<u8>,
}

impl Bitmap {
    /// Tope por mapa de bits, aparte del presupuesto de cada decodificacion (`Budget::bitmap`).
    fn new(w: usize, h: usize, fill: u8) -> Result<Bitmap, String> {
        if (w as u64).saturating_mul(h as u64) > 200_000_000 {
            return Err("JBIG2: mapa de bits demasiado grande.".into());
        }
        Ok(Bitmap { w, h, px: vec![fill; w * h] })
    }

    #[inline]
    fn get(&self, x: i64, y: i64) -> u32 {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            0
        } else {
            self.px[y as usize * self.w + x as usize] as u32
        }
    }

    /// Combina `src` en (x, y) con el operador de JBIG2: 0 O, 1 Y, 2 O exclusivo, 3 su negacion, 4
    /// reemplazo. Recorre solo la parte que cae dentro y devuelve cuantos pixeles combino.
    fn compose(&mut self, src: &Bitmap, x: i64, y: i64, op: u8) -> u64 {
        let (x0, y0) = (x.max(0), y.max(0));
        let x1 = x.saturating_add(src.w as i64).min(self.w as i64);
        let y1 = y.saturating_add(src.h as i64).min(self.h as i64);
        if x0 >= x1 || y0 >= y1 {
            return 0;
        }
        for ty in y0..y1 {
            let s_row = (ty - y) as usize * src.w;
            let t_row = ty as usize * self.w;
            for tx in x0..x1 {
                let s = src.px[s_row + (tx - x) as usize];
                let t = &mut self.px[t_row + tx as usize];
                *t = match op {
                    0 => *t | s,
                    1 => *t & s,
                    2 => *t ^ s,
                    3 => 1 ^ (*t ^ s),
                    _ => s,
                };
            }
        }
        ((x1 - x0) * (y1 - y0)) as u64
    }
}

// ---------------------------------------------------------------------------------------------
// Region generica (6.2)
// ---------------------------------------------------------------------------------------------

struct Generic {
    template: u8,
    tpgdon: bool,
    /// Pixeles adaptativos (dx, dy).
    at: Vec<(i64, i64)>,
}

fn generic_context(b: &Bitmap, x: i64, y: i64, g: &Generic) -> usize {
    let p = |dx: i64, dy: i64| b.get(x + dx, y + dy);
    let at = |i: usize| g.at.get(i).map_or(0, |&(dx, dy)| p(dx, dy));
    let c = match g.template {
        0 => {
            p(-1, 0)
                | p(-2, 0) << 1
                | p(-3, 0) << 2
                | p(-4, 0) << 3
                | at(0) << 4
                | p(2, -1) << 5
                | p(1, -1) << 6
                | p(0, -1) << 7
                | p(-1, -1) << 8
                | p(-2, -1) << 9
                | at(1) << 10
                | at(2) << 11
                | p(1, -2) << 12
                | p(0, -2) << 13
                | p(-1, -2) << 14
                | at(3) << 15
        }
        1 => {
            p(-1, 0)
                | p(-2, 0) << 1
                | p(-3, 0) << 2
                | at(0) << 3
                | p(2, -1) << 4
                | p(1, -1) << 5
                | p(0, -1) << 6
                | p(-1, -1) << 7
                | p(-2, -1) << 8
                | p(2, -2) << 9
                | p(1, -2) << 10
                | p(0, -2) << 11
                | p(-1, -2) << 12
        }
        2 => p(-1, 0) | p(-2, 0) << 1 | at(0) << 2 | p(1, -1) << 3 | p(0, -1) << 4 | p(-1, -1) << 5 | p(-2, -1) << 6 | p(1, -2) << 7 | p(0, -2) << 8 | p(-1, -2) << 9,
        _ => p(-1, 0) | p(-2, 0) << 1 | p(-3, 0) << 2 | p(-4, 0) << 3 | at(0) << 4 | p(1, -1) << 5 | p(0, -1) << 6 | p(-1, -1) << 7 | p(-2, -1) << 8 | p(-3, -1) << 9,
    };
    c as usize
}

/// Contexto de la prediccion tipica (SLTP) de cada plantilla.
const SLTP: [usize; 4] = [0x9B25, 0x0795, 0x00E5, 0x0195];

/// Cada pixel es una decision del decodificador: el presupuesto de pixeles acota tambien el tiempo.
fn decode_generic(mq: &mut Mq, cx: &mut [Cx], w: usize, h: usize, g: &Generic, budget: &mut Budget) -> Result<Bitmap, String> {
    let mut b = budget.bitmap(w, h, 0)?;
    let mut ltp = 0u32;
    for y in 0..h {
        if g.tpgdon {
            ltp ^= mq.bit(&mut cx[SLTP[g.template as usize & 3]]);
            if ltp == 1 {
                if y > 0 {
                    let (prev, cur) = b.px.split_at_mut(y * w);
                    cur[..w].copy_from_slice(&prev[(y - 1) * w..]);
                }
                continue;
            }
        }
        for x in 0..w {
            let c = generic_context(&b, x as i64, y as i64, g);
            b.px[y * w + x] = mq.bit(&mut cx[c]) as u8;
        }
    }
    Ok(b)
}

fn generic_cx(template: u8) -> Vec<Cx> {
    vec![0; 1 << if template == 0 { 16 } else if template == 1 { 13 } else { 10 }]
}

// ---------------------------------------------------------------------------------------------
// Refinamiento generico (6.3)
// ---------------------------------------------------------------------------------------------

struct Refine<'r> {
    template: u8,
    reference: &'r Bitmap,
    dx: i64,
    dy: i64,
    at: [(i64, i64); 2],
}

fn decode_refinement(mq: &mut Mq, cx: &mut [Cx], w: usize, h: usize, r: &Refine, budget: &mut Budget) -> Result<Bitmap, String> {
    let mut b = budget.bitmap(w, h, 0)?;
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let p = |dx: i64, dy: i64| b.get(x + dx, y + dy);
            let q = |dx: i64, dy: i64| r.reference.get(x - r.dx + dx, y - r.dy + dy);
            let c = if r.template == 0 {
                p(-1, 0)
                    | p(1, -1) << 1
                    | p(0, -1) << 2
                    | p(r.at[0].0, r.at[0].1) << 3
                    | q(1, 1) << 4
                    | q(0, 1) << 5
                    | q(-1, 1) << 6
                    | q(1, 0) << 7
                    | q(0, 0) << 8
                    | q(-1, 0) << 9
                    | q(1, -1) << 10
                    | q(0, -1) << 11
                    | q(r.at[1].0, r.at[1].1) << 12
            } else {
                p(-1, 0) | p(1, -1) << 1 | p(0, -1) << 2 | p(-1, -1) << 3 | q(1, 1) << 4 | q(0, 1) << 5 | q(1, 0) << 6 | q(0, 0) << 7 | q(-1, 0) << 8 | q(0, -1) << 9
            };
            let v = mq.bit(&mut cx[c as usize]) as u8;
            b.px[y as usize * w + x as usize] = v;
        }
    }
    Ok(b)
}

fn refine_cx(template: u8) -> Vec<Cx> {
    vec![0; 1 << if template == 0 { 13 } else { 10 }]
}

// ---------------------------------------------------------------------------------------------
// Region de texto (6.4) y diccionario de simbolos (6.5)
// ---------------------------------------------------------------------------------------------

/// Contextos de enteros compartidos por diccionarios y regiones de texto.
struct Ints {
    dh: Ia,
    dw: Ia,
    ex: Ia,
    ai: Ia,
    dt: Ia,
    fs: Ia,
    ds: Ia,
    it: Ia,
    ri: Ia,
    rdw: Ia,
    rdh: Ia,
    rdx: Ia,
    rdy: Ia,
}

impl Ints {
    fn new() -> Ints {
        Ints { dh: Ia::new(), dw: Ia::new(), ex: Ia::new(), ai: Ia::new(), dt: Ia::new(), fs: Ia::new(), ds: Ia::new(), it: Ia::new(), ri: Ia::new(), rdw: Ia::new(), rdh: Ia::new(), rdx: Ia::new(), rdy: Ia::new() }
    }
}

/// Bits de un identificador entre `n` simbolos: el menor `len` con 2^len >= n. En u64, sin
/// bucle: en wasm32 un `1usize << len` con `n` mayor que 2^31 no terminaba nunca.
fn code_len(n: usize) -> u32 {
    if n <= 1 {
        0
    } else {
        64 - ((n as u64) - 1).leading_zeros()
    }
}

/// Simbolos disponibles: los de los diccionarios de entrada y, dentro de un diccionario, los
/// nuevos ya decodificados. Se consultan sin copiarlos.
struct Syms<'s> {
    inputs: &'s [Rc<Bitmap>],
    new: &'s [Bitmap],
}

impl Syms<'_> {
    fn get(&self, i: usize) -> Option<&Bitmap> {
        match self.inputs.get(i) {
            Some(b) => Some(b),
            None => self.new.get(i - self.inputs.len()),
        }
    }
}

struct Text {
    w: usize,
    h: usize,
    instances: usize,
    strips: i64,
    refcorner: u8,
    transposed: bool,
    combop: u8,
    defpixel: u8,
    dsoffset: i64,
    refine: bool,
    rtemplate: u8,
    rat: [(i64, i64); 2],
}

struct Ctx {
    ints: Ints,
    iaid: Iaid,
    rcx: Vec<Cx>,
}

impl Ctx {
    /// Contextos para elegir entre `symbols` simbolos (a lo sumo `MAX_SYMBOLS`).
    fn new(symbols: usize, rtemplate: u8, budget: &mut Budget) -> Result<Ctx, String> {
        if symbols > MAX_SYMBOLS {
            return Err("JBIG2: demasiados simbolos.".into());
        }
        let len = code_len(symbols);
        budget.spend((1u64 << (len + 1)) + (1 << 13) + 13 * 512)?;
        Ok(Ctx { ints: Ints::new(), iaid: Iaid::new(len), rcx: refine_cx(rtemplate) })
    }
}

fn ok<T>(v: Option<T>) -> Result<T, String> {
    v.ok_or_else(|| "JBIG2: dato fuera de rango.".to_string())
}

fn decode_text(mq: &mut Mq, c: &mut Ctx, t: &Text, syms: &Syms, budget: &mut Budget) -> Result<Bitmap, String> {
    let mut b = budget.bitmap(t.w, t.h, t.defpixel)?;
    let mut stript = -ok(c.ints.dt.decode(mq))? * t.strips;
    let mut firsts: i64 = 0;
    let mut n = 0usize;
    let limit = t.instances.saturating_mul(4).saturating_add(1024);
    while n < t.instances {
        stript += ok(c.ints.dt.decode(mq))? * t.strips;
        let mut curs: i64 = 0;
        let mut first = true;
        // Cada franja termina con un valor fuera de rango de IADS.
        loop {
            if mq.exhausted() {
                return Err(ERR_END.into());
            }
            if first {
                firsts += ok(c.ints.fs.decode(mq))?;
                curs = firsts;
                first = false;
            } else {
                match c.ints.ds.decode(mq) {
                    Some(ids) => curs += ids + t.dsoffset,
                    None => break,
                }
            }
            let curt = if t.strips == 1 { 0 } else { ok(c.ints.it.decode(mq))? };
            let tt = stript + curt;
            let id = c.iaid.decode(mq);
            let base = syms.get(id).ok_or("JBIG2: simbolo inexistente.")?;
            let ri = if t.refine { ok(c.ints.ri.decode(mq))? } else { 0 };
            let refined;
            let ib: &Bitmap = if ri == 0 {
                base
            } else {
                let rdw = ok(c.ints.rdw.decode(mq))?;
                let rdh = ok(c.ints.rdh.decode(mq))?;
                let rdx = ok(c.ints.rdx.decode(mq))?;
                let rdy = ok(c.ints.rdy.decode(mq))?;
                let w = dim(base.w as i64 + rdw)?;
                let h = dim(base.h as i64 + rdh)?;
                let r = Refine { template: t.rtemplate, reference: base, dx: rdw.div_euclid(2) + rdx, dy: rdh.div_euclid(2) + rdy, at: t.rat };
                refined = decode_refinement(mq, &mut c.rcx, w, h, &r, budget)?;
                &refined
            };
            let (wi, hi) = (ib.w as i64, ib.h as i64);
            // Esquina de referencia: 0 abajo-izquierda, 1 arriba-izquierda, 2 abajo-derecha, 3 arriba-derecha.
            let rc = t.refcorner;
            if !t.transposed && (rc == 2 || rc == 3) {
                curs += wi - 1;
            } else if t.transposed && (rc == 0 || rc == 2) {
                curs += hi - 1;
            }
            let si = curs;
            let (x, y) = if !t.transposed {
                match rc {
                    1 => (si, tt),
                    3 => (si - wi + 1, tt),
                    0 => (si, tt - hi + 1),
                    _ => (si - wi + 1, tt - hi + 1),
                }
            } else {
                match rc {
                    1 => (tt, si),
                    3 => (tt - wi + 1, si),
                    0 => (tt, si - hi + 1),
                    _ => (tt - wi + 1, si - hi + 1),
                }
            };
            // Cada simbolo colocado cuesta sus decisiones de enteros y los pixeles que combina:
            // aunque sea de 0 x 0, millones de ellos no salen gratis.
            let area = b.compose(ib, x, y, t.combop);
            budget.spend(64 + area)?;
            if !t.transposed && (rc == 1 || rc == 0) {
                curs += wi - 1;
            } else if t.transposed && (rc == 1 || rc == 3) {
                curs += hi - 1;
            }
            n += 1;
            if n > limit {
                return Err("JBIG2: region de texto con mas simbolos que los declarados.".into());
            }
        }
    }
    Ok(b)
}

struct Dict {
    huff: bool,
    refagg: bool,
    template: u8,
    rtemplate: u8,
    at: Vec<(i64, i64)>,
    rat: [(i64, i64); 2],
    exported: usize,
    new: usize,
}

fn decode_dict(mq: &mut Mq, d: &Dict, inputs: &[Rc<Bitmap>], budget: &mut Budget) -> Result<Vec<Rc<Bitmap>>, String> {
    if d.huff {
        return Err("JBIG2 con codificacion Huffman: no se decodifica.".into());
    }
    let total = inputs.len().checked_add(d.new).filter(|&t| t <= MAX_SYMBOLS).ok_or("JBIG2: demasiados simbolos.")?;
    let mut ints = Ints::new();
    let mut gcx = generic_cx(d.template);
    budget.spend(gcx.len() as u64)?;
    let mut ctx = Ctx::new(total, d.rtemplate, budget)?;
    let mut new: Vec<Bitmap> = Vec::with_capacity(d.new.min(1 << 16));
    let mut hcheight: i64 = 0;
    let mut classes = 0usize;
    let g = Generic { template: d.template, tpgdon: false, at: d.at.clone() };
    while new.len() < d.new {
        // Cada clase de altura trae al menos un simbolo en un archivo real: con datos fabricados,
        // clases vacias repetidas no avanzarian nunca.
        classes += 1;
        if classes > d.new.saturating_add(64) {
            return Err("JBIG2: diccionario con clases de altura vacias.".into());
        }
        if mq.exhausted() {
            return Err(ERR_END.into());
        }
        budget.spend(32)?;
        hcheight += ok(ints.dh.decode(mq))?;
        let mut symwidth: i64 = 0;
        while let Some(dw) = ints.dw.decode(mq) {
            symwidth += dw;
            if symwidth < 0 || hcheight < 0 || new.len() >= d.new {
                return Err("JBIG2: tamano de simbolo invalido.".into());
            }
            if mq.exhausted() {
                return Err(ERR_END.into());
            }
            budget.spend(32)?;
            let (w, h) = (dim(symwidth)?, dim(hcheight)?);
            let bitmap = if !d.refagg {
                decode_generic(mq, &mut gcx, w, h, &g, budget)?
            } else {
                // Los contextos de enteros son los mismos del diccionario.
                std::mem::swap(&mut ctx.ints, &mut ints);
                let syms = Syms { inputs, new: &new };
                let result = refine_or_aggregate(mq, &mut ctx, d, &syms, w, h, budget);
                std::mem::swap(&mut ctx.ints, &mut ints);
                result?
            };
            new.push(bitmap);
        }
    }
    // Simbolos exportados: corridas alternadas de "no" y "si" sobre entradas y nuevos. Una corrida
    // de 0 no avanza: a lo sumo puede haber una por simbolo, mas la primera.
    let mut flags: Vec<bool> = Vec::with_capacity(total);
    let mut cur = false;
    let mut runs = 0usize;
    while flags.len() < total {
        runs += 1;
        if runs > 2 * total + 2 || mq.exhausted() {
            return Err("JBIG2: lista de simbolos exportados invalida.".into());
        }
        let run = ok(ints.ex.decode(mq))?;
        // En u64: en wasm32, `run as usize` truncaria un valor de mas de 32 bits.
        if run < 0 || (flags.len() as u64).saturating_add(run as u64) > total as u64 {
            return Err("JBIG2: lista de simbolos exportados invalida.".into());
        }
        flags.resize(flags.len() + run as usize, cur);
        cur = !cur;
    }
    // Los exportados se comparten con `Rc`: un diccionario que reexporta los de otro no los copia.
    let new: Vec<Rc<Bitmap>> = new.into_iter().map(Rc::new).collect();
    let mut out = Vec::with_capacity(d.exported.min(total));
    for (i, f) in flags.iter().enumerate() {
        if *f {
            out.push(Rc::clone(if i < inputs.len() { &inputs[i] } else { &new[i - inputs.len()] }));
        }
    }
    Ok(out)
}

fn refine_or_aggregate(mq: &mut Mq, c: &mut Ctx, d: &Dict, syms: &Syms, w: usize, h: usize, budget: &mut Budget) -> Result<Bitmap, String> {
    let instances = ok(c.ints.ai.decode(mq))?;
    if instances == 1 {
        let id = c.iaid.decode(mq);
        let rdx = ok(c.ints.rdx.decode(mq))?;
        let rdy = ok(c.ints.rdy.decode(mq))?;
        let reference = syms.get(id).ok_or("JBIG2: simbolo inexistente.")?;
        let r = Refine { template: d.rtemplate, reference, dx: rdx, dy: rdy, at: d.rat };
        decode_refinement(mq, &mut c.rcx, w, h, &r, budget)
    } else {
        let t = Text {
            w,
            h,
            // Acotado a 32 bits: en wasm32, `as usize` truncaria.
            instances: instances.clamp(0, u32::MAX as i64) as usize,
            strips: 1,
            refcorner: 1,
            transposed: false,
            combop: 0,
            defpixel: 0,
            dsoffset: 0,
            refine: true,
            rtemplate: d.rtemplate,
            rat: d.rat,
        };
        decode_text(mq, c, &t, syms, budget)
    }
}

// ---------------------------------------------------------------------------------------------
// Segmentos (7)
// ---------------------------------------------------------------------------------------------

fn be32(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

fn be16(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(i..i + 2)?.try_into().ok()?))
}

struct Segment<'a> {
    number: u32,
    kind: u8,
    refs: Vec<u32>,
    data: &'a [u8],
    /// Cantidad de filas del final de una region generica de largo desconocido (7.2.7).
    rows: Option<u32>,
}

/// Segmentos en organizacion secuencial (como van dentro de un PDF, sin encabezado de archivo).
fn segments(d: &[u8]) -> Result<Vec<Segment<'_>>, String> {
    let bad = || "JBIG2: encabezado de segmento danado.".to_string();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 11 <= d.len() {
        let number = be32(d, i).ok_or_else(bad)?;
        let flags = d[i + 4];
        i += 5;
        let kind = flags & 63;
        let r = *d.get(i).ok_or_else(bad)?;
        let mut count = (r >> 5) as usize;
        if count == 7 {
            count = (be32(d, i).ok_or_else(bad)? & 0x1FFF_FFFF) as usize;
            i += 4 + (count + 8) / 8;
        } else {
            i += 1;
        }
        let size = if number <= 256 { 1 } else if number <= 65536 { 2 } else { 4 };
        let mut refs = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            let v = match size {
                1 => *d.get(i).ok_or_else(bad)? as u32,
                2 => be16(d, i).ok_or_else(bad)? as u32,
                _ => be32(d, i).ok_or_else(bad)?,
            };
            refs.push(v);
            i += size;
        }
        i += if flags & 64 != 0 { 4 } else { 1 };
        let len = be32(d, i).ok_or_else(bad)?;
        i += 4;
        let (end, rows) = if len == u32::MAX {
            // Largo desconocido: solo en una region generica inmediata, que termina con su marca
            // de fin y la cantidad de filas.
            if kind != 38 && kind != 39 {
                return Err("JBIG2: segmento de largo desconocido que no es una region generica.".into());
            }
            let (end, rows) = unknown_end(d, i).ok_or("JBIG2: region generica de largo desconocido sin su marca de fin.")?;
            (end, Some(rows))
        } else {
            // En u64: en wasm32, `i + len` en usize desborda con largos cercanos a 4 GB y el fin
            // quedaria antes del principio.
            ((i as u64 + len as u64).min(d.len() as u64) as usize, None)
        };
        let data_end = if rows.is_some() { end - 4 } else { end };
        out.push(Segment { number, kind, refs, data: &d[i..data_end], rows });
        i = end;
        if kind == 51 {
            break;
        }
    }
    Ok(out)
}

/// Fin de una region generica de largo desconocido cuyos datos empiezan en `start` (7.2.7):
/// despues de la informacion de region (17 bytes), las banderas y los pixeles adaptativos, la
/// marca de fin (0xFF 0xAC si es aritmetica, 0x00 0x00 si es MMR) y 4 bytes con la cantidad de
/// filas. Devuelve el fin (despues de esos 4 bytes) y la cantidad de filas.
fn unknown_end(d: &[u8], start: usize) -> Option<(usize, u32)> {
    let flags = *d.get(start + 17)?;
    let (marker, from) = if flags & 1 != 0 { ([0u8, 0], start + 18) } else { ([0xFF, 0xAC], start + 18 + if (flags >> 1) & 3 == 0 { 8 } else { 2 }) };
    let p = from + d.get(from..)?.windows(2).position(|w| w == marker)?;
    Some((p + 6, be32(d, p + 2)?))
}

/// Informacion de region: ancho, alto, x, y y operador de combinacion.
fn region_info(d: &[u8]) -> Result<(usize, usize, i64, i64, u8), String> {
    let get = |k| be32(d, k).ok_or_else(|| "JBIG2: region incompleta.".to_string());
    Ok((get(0)? as usize, get(4)? as usize, get(8)? as i64, get(12)? as i64, d.get(16).copied().unwrap_or(0) & 7))
}

fn at_pixels(d: &[u8], at: usize, n: usize) -> Vec<(i64, i64)> {
    (0..n).map(|k| (d.get(at + 2 * k).copied().unwrap_or(0) as i8 as i64, d.get(at + 2 * k + 1).copied().unwrap_or(0) as i8 as i64)).collect()
}

/// Simbolos exportados por los diccionarios a los que se refiere un segmento, en orden. Se cuentan
/// antes de juntarlos: mil referencias al mismo diccionario grande no deben armar una lista enorme.
fn gather(dicts: &HashMap<u32, Vec<Rc<Bitmap>>>, refs: &[u32], budget: &mut Budget) -> Result<Vec<Rc<Bitmap>>, String> {
    let total: u64 = refs.iter().filter_map(|r| dicts.get(r)).map(|v| v.len() as u64).sum();
    if total > MAX_SYMBOLS as u64 {
        return Err("JBIG2: demasiados simbolos.".into());
    }
    budget.spend(total + refs.len() as u64)?;
    Ok(refs.iter().filter_map(|r| dicts.get(r)).flatten().cloned().collect())
}

/// Decodifica la pagina de un flujo JBIG2 de PDF (con sus segmentos globales, si los hay) en un
/// mapa de `width` x `height` (los del diccionario de la imagen).
pub fn decode(data: &[u8], globals: Option<&[u8]>, width: usize, height: usize) -> Result<Bitmap, String> {
    decode_with(data, globals, width, height, &mut Budget::for_page(width, height))
}

fn decode_with(data: &[u8], globals: Option<&[u8]>, width: usize, height: usize, budget: &mut Budget) -> Result<Bitmap, String> {
    let mut all = Vec::new();
    if let Some(g) = globals {
        all.extend(segments(g)?);
    }
    all.extend(segments(data)?);
    let mut dicts: HashMap<u32, Vec<Rc<Bitmap>>> = HashMap::new();
    let mut page: Option<Bitmap> = None;
    let mut page_op = 0u8;
    for s in &all {
        match s.kind {
            48 => {
                // Informacion de pagina: color por defecto y operador por defecto.
                let flags = s.data.get(16).copied().unwrap_or(0);
                page_op = (flags >> 3) & 3;
                page = Some(budget.bitmap(width, height, (flags >> 2) & 1)?);
            }
            0 => {
                let f = be16(s.data, 0).ok_or("JBIG2: diccionario incompleto.")?;
                let (huff, refagg) = (f & 1 != 0, f & 2 != 0);
                // Bit 8: el diccionario parte de los contextos que retuvo otro (bit 9 de ese). Con
                // contextos nuevos los simbolos saldrian mal: se informa.
                if f & 0x100 != 0 {
                    return Err("JBIG2 con un diccionario que reutiliza los contextos de otro: no se decodifica.".into());
                }
                let template = ((f >> 10) & 3) as u8;
                let rtemplate = ((f >> 12) & 1) as u8;
                let mut at = Vec::new();
                let mut i = 2;
                if !huff {
                    let n = if template == 0 { 4 } else { 1 };
                    at = at_pixels(s.data, i, n);
                    i += 2 * n;
                }
                let mut rat = [(-1, -1), (-1, -1)];
                if refagg && rtemplate == 0 {
                    let r = at_pixels(s.data, i, 2);
                    rat = [r[0], r[1]];
                    i += 4;
                }
                let exported = be32(s.data, i).ok_or("JBIG2: diccionario incompleto.")? as usize;
                let new = be32(s.data, i + 4).ok_or("JBIG2: diccionario incompleto.")? as usize;
                let inputs = gather(&dicts, &s.refs, budget)?;
                let d = Dict { huff, refagg, template, rtemplate, at, rat, exported, new };
                let mut mq = Mq::new(&s.data[(i + 8).min(s.data.len())..]);
                let syms = decode_dict(&mut mq, &d, &inputs, budget)?;
                dicts.insert(s.number, syms);
            }
            6 | 7 => {
                let (w, h, x, y, op) = region_info(s.data)?;
                let f = be16(s.data, 17).ok_or("JBIG2: region de texto incompleta.")?;
                if f & 1 != 0 {
                    return Err("JBIG2 con codificacion Huffman: no se decodifica.".into());
                }
                let refine = f & 2 != 0;
                let rtemplate = ((f >> 15) & 1) as u8;
                let mut i = 19;
                let mut rat = [(-1, -1), (-1, -1)];
                if refine && rtemplate == 0 {
                    let r = at_pixels(s.data, i, 2);
                    rat = [r[0], r[1]];
                    i += 4;
                }
                let instances = be32(s.data, i).ok_or("JBIG2: region de texto incompleta.")? as usize;
                let syms = gather(&dicts, &s.refs, budget)?;
                // DSOFFSET es un entero de 5 bits con signo.
                let raw = ((f >> 10) & 31) as i64;
                let t = Text {
                    w,
                    h,
                    instances,
                    strips: 1 << ((f >> 2) & 3),
                    refcorner: ((f >> 4) & 3) as u8,
                    transposed: f & 64 != 0,
                    combop: ((f >> 7) & 3) as u8,
                    defpixel: ((f >> 9) & 1) as u8,
                    dsoffset: if raw >= 16 { raw - 32 } else { raw },
                    refine,
                    rtemplate,
                    rat,
                };
                let mut c = Ctx::new(syms.len(), rtemplate, budget)?;
                let mut mq = Mq::new(&s.data[(i + 4).min(s.data.len())..]);
                let region = decode_text(&mut mq, &mut c, &t, &Syms { inputs: &syms, new: &[] }, budget)?;
                let p = page.as_mut().ok_or("JBIG2: region antes de la pagina.")?;
                let area = p.compose(&region, x, y, if op == 0 { page_op.max(op) } else { op });
                budget.spend(area)?;
            }
            38 | 39 => {
                let (w, h, x, y, op) = region_info(s.data)?;
                let f = *s.data.get(17).ok_or("JBIG2: region generica incompleta.")?;
                // EXTTEMPLATE (bit 4): plantilla de 16 pixeles adaptativos, no implementada. Leerla
                // como la plantilla 0 daria una imagen equivocada.
                if f & 16 != 0 {
                    return Err("JBIG2 con plantilla extendida (EXTTEMPLATE): no se decodifica.".into());
                }
                let template = (f >> 1) & 3;
                // Con largo desconocido, el alto es la cantidad de filas del final del segmento;
                // con alto desconocido sin eso, hasta el borde de la pagina.
                let h = match s.rows {
                    Some(rows) => (rows as usize).min(h),
                    None if h == u32::MAX as usize => height.saturating_sub(y.max(0) as usize),
                    None => h,
                };
                let region = if f & 1 != 0 {
                    // MMR: grupo 4 de fax, con 1 = negro igual que JBIG2. El mapa de bits se reserva
                    // antes, con su tope y el presupuesto.
                    let mut b = budget.bitmap(w, h, 0)?;
                    let p = ccitt::Params { k: -1, columns: w, rows: h, byte_align: false, end_of_block: true };
                    let (rows, _) = ccitt::decode(&s.data[18.min(s.data.len())..], &p, h)?;
                    let stride = w.max(1).div_ceil(8);
                    for yy in 0..h {
                        for xx in 0..w {
                            b.px[yy * w + xx] = (rows[yy * stride + xx / 8] >> (7 - xx % 8)) & 1;
                        }
                    }
                    b
                } else {
                    let n = if template == 0 { 4 } else { 1 };
                    let g = Generic { template, tpgdon: f & 8 != 0, at: at_pixels(s.data, 18, n) };
                    let mut cx = generic_cx(template);
                    budget.spend(cx.len() as u64)?;
                    let mut mq = Mq::new(&s.data[(18 + 2 * n).min(s.data.len())..]);
                    decode_generic(&mut mq, &mut cx, w, h, &g, budget)?
                };
                let p = page.as_mut().ok_or("JBIG2: region antes de la pagina.")?;
                let area = p.compose(&region, x, y, op);
                budget.spend(area)?;
            }
            4 | 20 | 22 | 23 | 16 | 36 | 40 | 42 | 43 => {
                return Err("JBIG2 con semitonos o regiones intermedias: no se decodifica.".into());
            }
            _ => {}
        }
    }
    page.ok_or_else(|| "JBIG2: no tiene informacion de pagina.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Codificador MQ (anexo E.2, con la convencion de JBIG2) para armar datos de prueba.
    struct Enc {
        a: u32,
        c: u32,
        ct: i32,
        b: i32,
        out: Vec<u8>,
    }

    impl Enc {
        fn new() -> Enc {
            Enc { a: 0x8000, c: 0, ct: 12, b: -1, out: Vec::new() }
        }
        fn byte_out(&mut self) {
            if self.b == 0xFF {
                self.emit_with_stuff();
            } else if self.c < 0x8000000 {
                self.push();
                self.b = (self.c >> 19) as i32;
                self.c &= 0x7FFFF;
                self.ct = 8;
            } else {
                self.b += 1;
                if self.b == 0xFF {
                    self.c &= 0x7FFFFFF;
                    self.emit_with_stuff();
                } else {
                    self.push();
                    self.b = (self.c >> 19) as i32;
                    self.c &= 0x7FFFF;
                    self.ct = 8;
                }
            }
        }
        fn emit_with_stuff(&mut self) {
            self.push();
            self.b = (self.c >> 20) as i32;
            self.c &= 0xFFFFF;
            self.ct = 7;
        }
        fn push(&mut self) {
            if self.b >= 0 {
                self.out.push(self.b as u8);
            }
        }
        fn renorm(&mut self) {
            loop {
                self.a <<= 1;
                self.c <<= 1;
                self.ct -= 1;
                if self.ct == 0 {
                    self.byte_out();
                }
                if self.a & 0x8000 != 0 {
                    break;
                }
            }
        }
        fn encode(&mut self, cx: &mut Cx, d: u32) {
            let (qe, nmps, nlps, switch) = QE[(*cx >> 1) as usize];
            let qe = qe as u32;
            let mps = (*cx & 1) as u32;
            if d == mps {
                self.a -= qe;
                if self.a & 0x8000 == 0 {
                    if self.a < qe {
                        self.a = qe;
                    } else {
                        self.c += qe;
                    }
                    *cx = (nmps << 1) | mps as u8;
                    self.renorm();
                } else {
                    self.c += qe;
                }
            } else {
                self.a -= qe;
                if self.a < qe {
                    self.c += qe;
                } else {
                    self.a = qe;
                }
                let m = if switch { 1 - mps } else { mps };
                *cx = (nlps << 1) | m as u8;
                self.renorm();
            }
        }
        fn flush(mut self) -> Vec<u8> {
            let temp = self.a.wrapping_sub(1).wrapping_add(self.c);
            self.c |= 0xFFFF;
            if self.c >= temp {
                self.c -= 0x8000;
            }
            self.c <<= self.ct;
            self.byte_out();
            self.c <<= self.ct;
            self.byte_out();
            self.push();
            self.out.push(0xFF);
            self.out.push(0xAC);
            self.out
        }
    }

    #[test]
    fn mq_ida_y_vuelta() {
        let bits: Vec<u32> = (0..5000u32).map(|i| ((i * 7919) % 13 < 3) as u32 ^ ((i / 100) % 2)).collect();
        let mut enc = Enc::new();
        let mut cx = [0u8; 4];
        for (i, &b) in bits.iter().enumerate() {
            enc.encode(&mut cx[i % 4], b);
        }
        let data = enc.flush();
        let mut mq = Mq::new(&data);
        let mut cx = [0u8; 4];
        let back: Vec<u32> = (0..bits.len()).map(|i| mq.bit(&mut cx[i % 4])).collect();
        assert_eq!(back, bits);
    }

    #[test]
    fn region_generica_ida_y_vuelta() {
        // Codifica un dibujo con la plantilla 0 y comprueba que se decodifica igual.
        let (w, h) = (37usize, 11usize);
        let mut src = Bitmap::new(w, h, 0).unwrap();
        for y in 0..h {
            for x in 0..w {
                src.px[y * w + x] = ((x / 3 + y / 2) % 3 == 0 || x == y * 3) as u8;
            }
        }
        let g = Generic { template: 0, tpgdon: false, at: vec![(3, -1), (-3, -1), (2, -2), (-2, -2)] };
        let mut enc = Enc::new();
        let mut cx = generic_cx(0);
        for y in 0..h {
            for x in 0..w {
                let c = generic_context(&src, x as i64, y as i64, &g);
                enc.encode(&mut cx[c], src.px[y * w + x] as u32);
            }
        }
        let data = enc.flush();
        let mut mq = Mq::new(&data);
        let back = decode_generic(&mut mq, &mut generic_cx(0), w, h, &g, &mut Budget::for_page(w, h)).unwrap();
        assert_eq!(back.px, src.px);
    }

    /// Codifica un entero con el procedimiento IAx (A.2), para armar datos de prueba.
    fn encode_int(enc: &mut Enc, cx: &mut [Cx], v: Option<i64>) {
        let mut prev = 1usize;
        let mut bit = |enc: &mut Enc, d: u32| {
            enc.encode(&mut cx[prev], d);
            prev = if prev < 256 { (prev << 1) | d as usize } else { (((prev << 1) | d as usize) & 511) | 256 };
        };
        let (s, mag) = match v {
            None => (1, 0),
            Some(v) => ((v < 0) as u32, v.unsigned_abs() as i64),
        };
        bit(enc, s);
        let ranges = [(2, 0, 3), (4, 4, 19), (6, 20, 83), (8, 84, 339), (12, 340, 4435), (32, 4436, i64::MAX)];
        for (k, &(n, lo, hi)) in ranges.iter().enumerate() {
            if mag <= hi {
                for _ in 0..k {
                    bit(enc, 1);
                }
                if k < 5 {
                    bit(enc, 0);
                }
                for i in (0..n).rev() {
                    bit(enc, ((mag - lo) >> i) as u32 & 1);
                }
                return;
            }
        }
    }

    /// Encabezado de segmento con numero de 1 byte, sin referencias y pagina 1.
    fn segment(number: u32, kind: u8, data: &[u8]) -> Vec<u8> {
        let mut s = number.to_be_bytes().to_vec();
        s.extend_from_slice(&[kind, 0, 1]);
        s.extend_from_slice(&(data.len() as u32).to_be_bytes());
        s.extend_from_slice(data);
        s
    }

    #[test]
    fn pagina_con_region_generica() {
        // Pagina de 20 x 6 con una region de 12 x 4 ubicada en (5, 1).
        let (w, h) = (12usize, 4usize);
        let mut src = Bitmap::new(w, h, 0).unwrap();
        for y in 0..h {
            for x in 0..w {
                src.px[y * w + x] = ((x + y) % 4 == 0) as u8;
            }
        }
        let at = [(3i8, -1i8), (-3, -1), (2, -2), (-2, -2)];
        let g = Generic { template: 0, tpgdon: false, at: at.iter().map(|&(x, y)| (x as i64, y as i64)).collect() };
        let mut enc = Enc::new();
        let mut cx = generic_cx(0);
        for y in 0..h {
            for x in 0..w {
                enc.encode(&mut cx[generic_context(&src, x as i64, y as i64, &g)], src.px[y * w + x] as u32);
            }
        }
        let mut info = Vec::new();
        for v in [20u32, 6, 300, 300] {
            info.extend_from_slice(&v.to_be_bytes());
        }
        info.extend_from_slice(&[0, 0, 0]);
        let mut region = Vec::new();
        for v in [w as u32, h as u32, 5, 1] {
            region.extend_from_slice(&v.to_be_bytes());
        }
        region.push(0); // operador O
        region.push(0); // plantilla 0, aritmetica, sin prediccion tipica
        for (x, y) in at {
            region.extend_from_slice(&[x as u8, y as u8]);
        }
        region.extend(enc.flush());
        let mut stream = segment(0, 48, &info);
        stream.extend(segment(1, 39, &region));
        stream.extend(segment(2, 49, &[]));
        let page = decode(&stream, None, 20, 6).unwrap();
        for y in 0..6 {
            for x in 0..20 {
                let inside = (5..17).contains(&x) && (1..5).contains(&y);
                let expected = if inside { src.px[(y - 1) * w + x - 5] } else { 0 };
                assert_eq!(page.px[y * 20 + x], expected, "pixel ({}, {})", x, y);
            }
        }
        // Huffman no se decodifica: se informa.
        let mut huff = segment(0, 48, &info);
        huff.extend(segment(1, 0, &[0, 1, 0, 0, 0, 1, 0, 0, 0, 1]));
        assert!(decode(&huff, None, 20, 6).unwrap_err().contains("Huffman"));
    }

    #[test]
    fn enteros_ida_y_vuelta() {
        let values = [Some(0), Some(3), Some(-4), Some(19), Some(-83), Some(339), Some(4000), Some(-70000), None, Some(1)];
        let mut enc = Enc::new();
        let mut cx = vec![0u8; 512];
        for v in values {
            encode_int(&mut enc, &mut cx, v);
        }
        let data = enc.flush();
        let mut mq = Mq::new(&data);
        let mut ia = Ia::new();
        let back: Vec<Option<i64>> = values.iter().map(|_| ia.decode(&mut mq)).collect();
        assert_eq!(back, values);
    }

    /// Encabezado de segmento con numero de 1 byte, referencias de 1 byte y pagina 1.
    fn segment_refs(number: u32, kind: u8, refs: &[u8], data: &[u8]) -> Vec<u8> {
        let mut s = number.to_be_bytes().to_vec();
        s.push(kind);
        s.push((refs.len() as u8) << 5);
        s.extend_from_slice(refs);
        s.push(1);
        s.extend_from_slice(&(data.len() as u32).to_be_bytes());
        s.extend_from_slice(data);
        s
    }

    const AT0: [(i8, i8); 4] = [(3, -1), (-3, -1), (2, -2), (-2, -2)];

    fn g0() -> Generic {
        Generic { template: 0, tpgdon: false, at: AT0.iter().map(|&(x, y)| (x as i64, y as i64)).collect() }
    }

    /// Informacion de pagina de `w` x `h`.
    fn page_info(w: u32, h: u32) -> Vec<u8> {
        let mut info = Vec::new();
        for v in [w, h, 300, 300] {
            info.extend_from_slice(&v.to_be_bytes());
        }
        info.extend_from_slice(&[0, 0, 0]);
        info
    }

    /// Simbolo de prueba: un marco de `w` x `h` con una diagonal.
    fn glyph(w: usize, h: usize) -> Bitmap {
        let mut b = Bitmap::new(w, h, 0).unwrap();
        for y in 0..h {
            for x in 0..w {
                b.px[y * w + x] = (x == 0 || y == 0 || x == w - 1 || y == h - 1 || x == y) as u8;
            }
        }
        b
    }

    /// Codificador de un diccionario aritmetico (plantilla 0, sin refinamiento) con los mismos
    /// contextos que usa `decode_dict`.
    struct DictEnc {
        enc: Enc,
        dh: Vec<Cx>,
        dw: Vec<Cx>,
        ex: Vec<Cx>,
        gcx: Vec<Cx>,
    }

    impl DictEnc {
        fn new() -> DictEnc {
            DictEnc { enc: Enc::new(), dh: vec![0; 512], dw: vec![0; 512], ex: vec![0; 512], gcx: generic_cx(0) }
        }
        /// Una clase de altura: diferencia de alto y sus simbolos (todos de ese alto).
        fn class(&mut self, dh: i64, syms: &[Bitmap]) {
            encode_int(&mut self.enc, &mut self.dh, Some(dh));
            let (mut prev, g) = (0i64, g0());
            for s in syms {
                encode_int(&mut self.enc, &mut self.dw, Some(s.w as i64 - prev));
                prev = s.w as i64;
                for y in 0..s.h {
                    for x in 0..s.w {
                        self.enc.encode(&mut self.gcx[generic_context(s, x as i64, y as i64, &g)], s.px[y * s.w + x] as u32);
                    }
                }
            }
            encode_int(&mut self.enc, &mut self.dw, None);
        }
        fn runs(&mut self, runs: &[i64]) {
            for &r in runs {
                encode_int(&mut self.enc, &mut self.ex, Some(r));
            }
        }
        /// Datos del segmento: banderas, pixeles adaptativos, exportados, nuevos y el flujo MQ.
        fn data(self, flags: u16, exported: u32, new: u32) -> Vec<u8> {
            let mut d = flags.to_be_bytes().to_vec();
            for (x, y) in AT0 {
                d.extend_from_slice(&[x as u8, y as u8]);
            }
            d.extend_from_slice(&exported.to_be_bytes());
            d.extend_from_slice(&new.to_be_bytes());
            d.extend(self.enc.flush());
            d
        }
    }

    /// Region de texto (sin refinamiento, una franja, esquina arriba-izquierda) que coloca el
    /// simbolo 0, de ancho `sym_w`, en cada `x` de `xs`, todas en la fila `y`.
    fn text_data(w: u32, h: u32, y: i64, xs: &[i64], sym_w: i64, instances: u32) -> Vec<u8> {
        let mut enc = Enc::new();
        let (mut dt, mut fs, mut ds) = (vec![0u8; 512], vec![0u8; 512], vec![0u8; 512]);
        encode_int(&mut enc, &mut dt, Some(0));
        encode_int(&mut enc, &mut dt, Some(y));
        encode_int(&mut enc, &mut fs, Some(xs[0]));
        let mut curs = xs[0] + sym_w - 1;
        for &x in &xs[1..] {
            encode_int(&mut enc, &mut ds, Some(x - curs));
            curs = x + sym_w - 1;
        }
        encode_int(&mut enc, &mut ds, None);
        let mut d = Vec::new();
        for v in [w, h, 0, 0] {
            d.extend_from_slice(&v.to_be_bytes());
        }
        d.push(0);
        d.extend_from_slice(&0x0010u16.to_be_bytes());
        d.extend_from_slice(&instances.to_be_bytes());
        d.extend(enc.flush());
        d
    }

    /// Pagina de 20 x 6 con un diccionario de un simbolo de 5 x 4 colocado en (2, 1) y (10, 1).
    fn pagina_con_texto(exported_field: u32) -> Vec<u8> {
        let mut de = DictEnc::new();
        de.class(4, &[glyph(5, 4)]);
        de.runs(&[0, 1]);
        let mut s = segment(0, 48, &page_info(20, 6));
        s.extend(segment(1, 0, &de.data(0, exported_field, 1)));
        s.extend(segment_refs(2, 6, &[1], &text_data(20, 6, 1, &[2, 10], 5, 2)));
        s.extend(segment(3, 49, &[]));
        s
    }

    fn con_simbolos(page: &Bitmap, at: &[(usize, usize)], g: &Bitmap) -> bool {
        (0..page.h).all(|y| {
            (0..page.w).all(|x| {
                let v = at.iter().find(|&&(ax, ay)| (ax..ax + g.w).contains(&x) && (ay..ay + g.h).contains(&y)).map_or(0, |&(ax, ay)| g.px[(y - ay) * g.w + x - ax]);
                page.px[y * page.w + x] == v
            })
        })
    }

    #[test]
    fn diccionario_y_region_de_texto() {
        let page = decode(&pagina_con_texto(1), None, 20, 6).unwrap();
        assert!(con_simbolos(&page, &[(2, 1), (10, 1)], &glyph(5, 4)));
        // Un campo de exportados enorme ya no reserva memoria por adelantado (antes pedia 4.294
        // millones de lugares): lo que manda es la lista de exportados decodificada.
        let page = decode(&pagina_con_texto(u32::MAX), None, 20, 6).unwrap();
        assert!(con_simbolos(&page, &[(2, 1), (10, 1)], &glyph(5, 4)));
    }

    #[test]
    fn simbolos_reexportados_se_comparten() {
        let inputs = vec![Rc::new(glyph(5, 4))];
        let mut de = DictEnc::new();
        de.runs(&[0, 1]);
        let data = de.enc.flush();
        let d = Dict { huff: false, refagg: false, template: 0, rtemplate: 0, at: g0().at, rat: [(-1, -1); 2], exported: 1, new: 0 };
        let out = decode_dict(&mut Mq::new(&data), &d, &inputs, &mut Budget::for_page(20, 6)).unwrap();
        assert!(Rc::ptr_eq(&out[0], &inputs[0]));
    }

    #[test]
    fn mq_vector_del_anexo_h() {
        // Secuencia de prueba del decodificador MQ de T.88 (H.2): 256 decisiones con un solo
        // contexto, y su codificacion.
        let coded = [
            0x84, 0xC7, 0x3B, 0xFC, 0xE1, 0xA1, 0x43, 0x04, 0x02, 0x20, 0x00, 0x00, 0x41, 0x0D, 0xBB, 0x86, 0xF4, 0x31, 0x7F, 0xFF, 0x88, 0xFF, 0x37, 0x47, 0x1A, 0xDB, 0x6A,
            0xDF, 0xFF, 0xAC,
        ];
        let plain = [
            0x00, 0x02, 0x00, 0x51, 0x00, 0x00, 0x00, 0xC0, 0x03, 0x52, 0x87, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA, 0x82, 0xC0, 0x20, 0x00, 0xFC, 0xD7, 0x9E, 0xF6, 0xBF, 0x7F,
            0xED, 0x90, 0x4F, 0x46, 0xA3, 0xBF,
        ];
        let mut mq = Mq::new(&coded);
        let mut cx = 0u8;
        let back: Vec<u8> = (0..32).map(|_| (0..8).fold(0u8, |b, _| (b << 1) | mq.bit(&mut cx) as u8)).collect();
        assert_eq!(back, plain);
        let mut enc = Enc::new();
        let mut cx = 0u8;
        for byte in plain {
            for i in (0..8).rev() {
                enc.encode(&mut cx, (byte >> i) as u32 & 1);
            }
        }
        assert_eq!(enc.flush(), coded);
    }

    #[test]
    fn segmento_de_11_bytes_con_largo_enorme() {
        // En wasm32, 11 + 0xFFFFFFFE desbordaba y el fin quedaba antes del principio.
        let d = [0, 0, 0, 0, 48, 0, 1, 0xFF, 0xFF, 0xFF, 0xFE];
        let s = segments(&d).unwrap();
        assert_eq!((s.len(), s[0].data.len()), (1, 0));
        assert!(decode(&d, None, 8, 8).is_ok());
    }

    #[test]
    fn mas_simbolos_que_el_tope() {
        assert_eq!([0, 1, 2, 3, 4, 5, 1 << 20].map(code_len), [0, 0, 1, 2, 2, 3, 20]);
        assert_eq!((code_len((1 << 31) + 1), code_len(usize::MAX)), (32, 64));
        // 2^31 + 1 simbolos nuevos: en wasm32 `code_len` no terminaba; en 64 bits la tabla de
        // contextos recortada quedaba corta y se decodificaban simbolos sin fin.
        let mut s = segment(0, 48, &page_info(8, 8));
        s.extend(segment(1, 0, &DictEnc::new().data(0, 0, 0x8000_0001)));
        assert!(decode(&s, None, 8, 8).unwrap_err().contains("demasiados simbolos"));
        // Una region de texto que junta mas simbolos que el tope repitiendo la referencia al mismo
        // diccionario (forma larga de las referencias: 1.100.000 de ellas).
        let mut de = DictEnc::new();
        de.class(1, &[glyph(1, 1)]);
        de.runs(&[0, 1]);
        let mut s = segment(1, 0, &de.data(0, 1, 1));
        let n = 1_100_000u32;
        let body = text_data(8, 8, 0, &[0], 1, 1);
        s.extend_from_slice(&2u32.to_be_bytes());
        s.push(6);
        s.extend_from_slice(&(n | 0xE000_0000).to_be_bytes());
        s.extend(vec![0u8; (n as usize + 8) / 8]);
        s.extend(vec![1u8; n as usize]);
        s.push(1);
        s.extend_from_slice(&(body.len() as u32).to_be_bytes());
        s.extend(body);
        assert!(decode(&s, None, 8, 8).unwrap_err().contains("demasiados simbolos"));
    }

    #[test]
    fn clases_de_altura_vacias_y_corridas_en_cero() {
        // Clases de altura sin simbolos, una tras otra: antes no avanzaba nunca.
        let mut de = DictEnc::new();
        for _ in 0..100 {
            de.class(0, &[]);
        }
        let s = segment(1, 0, &de.data(0, 1, 5));
        assert!(decode(&s, None, 8, 8).unwrap_err().contains("clases de altura vacias"));
        // Corridas de exportados de largo 0, una tras otra.
        let mut de = DictEnc::new();
        de.class(4, &[glyph(5, 4)]);
        de.runs(&[0; 20]);
        let s = segment(1, 0, &de.data(0, 1, 1));
        assert!(decode(&s, None, 8, 8).unwrap_err().contains("exportados"));
    }

    #[test]
    fn datos_cortados_y_presupuesto() {
        // Una region de texto que declara un millon de simbolos y trae dos: se informa.
        let mut de = DictEnc::new();
        de.class(4, &[glyph(5, 4)]);
        de.runs(&[0, 1]);
        let mut s = segment(0, 48, &page_info(20, 6));
        s.extend(segment(1, 0, &de.data(0, 1, 1)));
        s.extend(segment_refs(2, 6, &[1], &text_data(20, 6, 1, &[2, 10], 5, 1_000_000)));
        let start = std::time::Instant::now();
        assert!(decode(&s, None, 20, 6).is_err());
        assert!(start.elapsed().as_secs() < 2);
        // Una region de 10.000 x 10.000 en una pagina de 64 x 64: antes reservaba 100 MB y
        // decodificaba 100 millones de pixeles; ahora no entra en el presupuesto de la pagina.
        for mmr in [0u8, 1] {
            let mut region = Vec::new();
            for v in [10_000u32, 10_000, 0, 0] {
                region.extend_from_slice(&v.to_be_bytes());
            }
            region.extend_from_slice(&[0, mmr, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xAC]);
            let mut s = segment(0, 48, &page_info(64, 64));
            s.extend(segment(1, 38, &region));
            assert!(decode(&s, None, 64, 64).unwrap_err().contains("memoria"), "mmr {}", mmr);
        }
        // MMR con 2^31 columnas: antes la memoria se reservaba en el decodificador de fax, sin tope.
        let mut region = Vec::new();
        for v in [0x8000_0000u32, 16, 0, 0] {
            region.extend_from_slice(&v.to_be_bytes());
        }
        region.extend_from_slice(&[0, 1, 0xFF, 0xFF]);
        let mut s = segment(0, 48, &page_info(64, 64));
        s.extend(segment(1, 38, &region));
        assert!(decode(&s, None, 64, 64).is_err());
    }

    #[test]
    fn variantes_no_soportadas_se_informan() {
        // Diccionario que reutiliza los contextos de otro (bit 8 de sus banderas).
        let mut de = DictEnc::new();
        de.class(4, &[glyph(5, 4)]);
        de.runs(&[0, 1]);
        let s = segment(1, 0, &de.data(0x0100, 1, 1));
        assert!(decode(&s, None, 8, 8).unwrap_err().contains("contextos de otro"));
        // Region generica con plantilla extendida (bit 4).
        let mut region = Vec::new();
        for v in [4u32, 4, 0, 0] {
            region.extend_from_slice(&v.to_be_bytes());
        }
        region.extend_from_slice(&[0, 0x10, 3, 0xFF, 0, 0, 0, 0, 0, 0, 0xFF, 0xAC]);
        let mut s = segment(0, 48, &page_info(8, 8));
        s.extend(segment(1, 38, &region));
        assert!(decode(&s, None, 8, 8).unwrap_err().contains("EXTTEMPLATE"));
    }

    /// Region generica de 12 x 4 (plantilla 0) y sus pixeles.
    fn region_generica() -> (Bitmap, Vec<u8>) {
        let (w, h) = (12usize, 4usize);
        let mut src = Bitmap::new(w, h, 0).unwrap();
        for y in 0..h {
            for x in 0..w {
                src.px[y * w + x] = ((x + y) % 4 == 0) as u8;
            }
        }
        let g = g0();
        let mut enc = Enc::new();
        let mut cx = generic_cx(0);
        for y in 0..h {
            for x in 0..w {
                enc.encode(&mut cx[generic_context(&src, x as i64, y as i64, &g)], src.px[y * w + x] as u32);
            }
        }
        (src, enc.flush())
    }

    #[test]
    fn largo_desconocido_con_cantidad_de_filas() {
        // Region de alto y largo desconocidos (0xFFFFFFFF): termina con la marca 0xFF 0xAC y la
        // cantidad de filas; despues sigue el fin de pagina, que ya no se toma como parte de ella.
        let (src, coded) = region_generica();
        let mut region = Vec::new();
        for v in [12u32, u32::MAX, 5, 1] {
            region.extend_from_slice(&v.to_be_bytes());
        }
        region.push(0);
        region.push(0);
        for (x, y) in AT0 {
            region.extend_from_slice(&[x as u8, y as u8]);
        }
        region.extend(coded);
        region.extend_from_slice(&4u32.to_be_bytes());
        let mut s = segment(0, 48, &page_info(20, 10));
        s.extend_from_slice(&[0, 0, 0, 1, 38, 0, 1, 0xFF, 0xFF, 0xFF, 0xFF]);
        s.extend(region);
        s.extend(segment(2, 49, &[]));
        let segs = segments(&s).unwrap();
        assert_eq!(segs.iter().map(|s| (s.kind, s.rows)).collect::<Vec<_>>(), [(48, None), (38, Some(4)), (49, None)]);
        let page = decode(&s, None, 20, 10).unwrap();
        assert!(con_simbolos(&page, &[(5, 1)], &src));
        // Sin la marca de fin, se informa.
        let corto = &s[..s.len() - 20];
        assert!(segments(corto).err().unwrap_or_default().contains("marca de fin"));
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
        // Flujos validos (region generica, diccionario con region de texto, largo desconocido) con
        // bytes cambiados, cortes e inserciones; y segmentos con cabeceras al azar.
        let (_, coded) = region_generica();
        let mut generic = Vec::new();
        for v in [12u32, 4, 5, 1] {
            generic.extend_from_slice(&v.to_be_bytes());
        }
        generic.extend_from_slice(&[0, 0]);
        for (x, y) in AT0 {
            generic.extend_from_slice(&[x as u8, y as u8]);
        }
        generic.extend(coded);
        let mut pagina = segment(0, 48, &page_info(20, 6));
        pagina.extend(segment(1, 38, &generic));
        let seeds = [pagina, pagina_con_texto(1)];
        let kinds = [0u8, 4, 6, 7, 16, 38, 39, 40, 42, 48, 49, 50, 51, 62];
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        for case in 0..3000 {
            let mut data = if case % 5 == 0 {
                let mut d = Vec::new();
                for _ in 0..1 + rng.below(4) {
                    let len = rng.below(40) as usize;
                    let body: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
                    let mut seg = segment(rng.below(4) as u32, kinds[rng.below(kinds.len() as u64) as usize], &body);
                    seg[5] = rng.next() as u8 & 0xE0;
                    if rng.below(4) == 0 {
                        seg[7..11].copy_from_slice(&(rng.next() as u32).to_be_bytes());
                    }
                    d.extend(seg);
                }
                d
            } else {
                seeds[rng.below(2) as usize].clone()
            };
            for _ in 0..1 + rng.below(4) {
                let i = rng.below(data.len() as u64) as usize;
                match rng.below(4) {
                    0 => data[i] ^= 1 << rng.below(8),
                    1 => data[i] = rng.next() as u8,
                    2 => data.insert(i, rng.next() as u8),
                    _ => data[i] = [0, 0xFF, 0x7F, 0x80][rng.below(4) as usize],
                }
            }
            if rng.below(5) == 0 {
                data.truncate(rng.below(data.len() as u64 + 1) as usize);
            }
            let (w, h) = if rng.below(3) == 0 { (1 + rng.below(300) as usize, 1 + rng.below(300) as usize) } else { (20, 6) };
            let start = std::time::Instant::now();
            let _ = decode_with(&data, None, w, h, &mut Budget { pixels: 250_000, work: 1_000_000 });
            assert!(start.elapsed().as_secs() < 2, "caso {}: demasiado lento", case);
        }
    }

    #[test]
    fn simbolo_de_ancho_cero_y_alto_enorme() {
        // 0 x 2.000 millones: no ocupa memoria, pero antes se recorrian sus filas una por una
        // (lo encontro la prueba con datos alterados).
        let mut de = DictEnc::new();
        encode_int(&mut de.enc, &mut de.dh, Some(2_000_000_000));
        encode_int(&mut de.enc, &mut de.dw, Some(0));
        let s = segment(1, 0, &de.data(0, 1, 1));
        let start = std::time::Instant::now();
        assert!(decode(&s, None, 8, 8).unwrap_err().contains("memoria"));
        assert!(start.elapsed().as_secs() < 1);
    }
}
