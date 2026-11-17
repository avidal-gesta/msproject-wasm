//! Decodificador JBIG2 (UIT-T T.88) para el filtro `JBIG2Decode` de PDF: escaneos en blanco y
//! negro comprimidos.
//!
//! Cubre lo que usan los PDF reales: regiones genericas (aritmeticas, plantillas 0 a 3 con
//! prediccion tipica, o MMR), diccionarios de simbolos y regiones de texto aritmeticos, con
//! refinamiento y agregacion. No cubre la codificacion Huffman, los semitonos ni las regiones
//! intermedias: si aparecen, se informa en vez de entregar una imagen a medias.
//!
//! Devuelve el mapa de bits de la pagina con 1 = negro, un byte por pixel.

use crate::ccitt;
use std::collections::HashMap;

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

struct Mq<'a> {
    data: &'a [u8],
    bp: usize,
    /// Registro de codigo: parte alta (comparada con A) y baja.
    chigh: u32,
    clow: u32,
    a: u32,
    ct: u32,
}

impl<'a> Mq<'a> {
    fn new(data: &'a [u8]) -> Mq<'a> {
        let mut m = Mq { data, bp: 0, chigh: data.first().copied().unwrap_or(0xFF) as u32, clow: 0, a: 0, ct: 0 };
        m.byte_in();
        m.chigh = ((m.chigh << 7) & 0xFFFF) | ((m.clow >> 9) & 0x7F);
        m.clow = (m.clow << 7) & 0xFFFF;
        m.ct = m.ct.saturating_sub(7);
        m.a = 0x8000;
        m
    }

    fn byte_in(&mut self) {
        let cur = self.data.get(self.bp).copied().unwrap_or(0xFF);
        if cur == 0xFF {
            let next = self.data.get(self.bp + 1).copied().unwrap_or(0xFF);
            if next > 0x8F {
                self.clow += 0xFF00;
                self.ct = 8;
            } else {
                self.bp += 1;
                self.clow += (next as u32) << 9;
                self.ct = 7;
            }
        } else {
            self.bp += 1;
            let b = self.data.get(self.bp).copied().unwrap_or(0xFF);
            self.clow += (b as u32) << 8;
            self.ct = 8;
        }
        if self.clow > 0xFFFF {
            self.chigh += self.clow >> 16;
            self.clow &= 0xFFFF;
        }
    }

    fn bit(&mut self, cx: &mut Cx) -> u32 {
        let (qe, nmps, nlps, switch) = QE[(*cx >> 1) as usize];
        let qe = qe as u32;
        let mut mps = (*cx & 1) as u32;
        let index;
        let d;
        let mut a = self.a.wrapping_sub(qe);
        if self.chigh < qe {
            // Intercambio condicional en el subintervalo menos probable.
            if a < qe {
                a = qe;
                d = mps;
                index = nmps;
            } else {
                a = qe;
                d = 1 ^ mps;
                if switch {
                    mps = d;
                }
                index = nlps;
            }
        } else {
            self.chigh -= qe;
            if a & 0x8000 != 0 {
                self.a = a;
                return mps;
            }
            if a < qe {
                d = 1 ^ mps;
                if switch {
                    mps = d;
                }
                index = nlps;
            } else {
                d = mps;
                index = nmps;
            }
        }
        // Renormalizacion.
        loop {
            if self.ct == 0 {
                self.byte_in();
            }
            a <<= 1;
            self.chigh = ((self.chigh << 1) & 0xFFFF) | ((self.clow >> 15) & 1);
            self.clow = (self.clow << 1) & 0xFFFF;
            self.ct -= 1;
            if a & 0x8000 != 0 {
                break;
            }
        }
        self.a = a;
        *cx = (index << 1) | mps as u8;
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
    fn new(len: u32) -> Iaid {
        Iaid { len, cx: vec![0; 1 << (len + 1).min(24)] }
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
    fn new(w: usize, h: usize, fill: u8) -> Result<Bitmap, String> {
        if w.saturating_mul(h) > 200_000_000 {
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

    /// Combina `src` en (x, y) con el operador de JBIG2: 0 O, 1 Y, 2 O exclusivo, 3 su negacion, 4 reemplazo.
    fn compose(&mut self, src: &Bitmap, x: i64, y: i64, op: u8) {
        for sy in 0..src.h as i64 {
            let ty = y + sy;
            if ty < 0 || ty >= self.h as i64 {
                continue;
            }
            for sx in 0..src.w as i64 {
                let tx = x + sx;
                if tx < 0 || tx >= self.w as i64 {
                    continue;
                }
                let s = src.px[(sy as usize) * src.w + sx as usize];
                let t = &mut self.px[ty as usize * self.w + tx as usize];
                *t = match op {
                    0 => *t | s,
                    1 => *t & s,
                    2 => *t ^ s,
                    3 => 1 ^ (*t ^ s),
                    _ => s,
                };
            }
        }
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

fn decode_generic(mq: &mut Mq, cx: &mut [Cx], w: usize, h: usize, g: &Generic) -> Result<Bitmap, String> {
    let mut b = Bitmap::new(w, h, 0)?;
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

fn decode_refinement(mq: &mut Mq, cx: &mut [Cx], w: usize, h: usize, r: &Refine) -> Result<Bitmap, String> {
    let mut b = Bitmap::new(w, h, 0)?;
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

fn code_len(n: usize) -> u32 {
    let mut len = 0;
    while (1usize << len) < n {
        len += 1;
    }
    len
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

fn ok<T>(v: Option<T>) -> Result<T, String> {
    v.ok_or_else(|| "JBIG2: dato fuera de rango.".to_string())
}

fn decode_text(mq: &mut Mq, c: &mut Ctx, t: &Text, syms: &[&Bitmap]) -> Result<Bitmap, String> {
    let mut b = Bitmap::new(t.w, t.h, t.defpixel)?;
    let mut stript = -ok(c.ints.dt.decode(mq))? * t.strips;
    let mut firsts: i64 = 0;
    let mut n = 0usize;
    while n < t.instances {
        stript += ok(c.ints.dt.decode(mq))? * t.strips;
        let mut curs: i64 = 0;
        let mut first = true;
        // Cada franja termina con un valor fuera de rango de IADS.
        loop {
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
            let base = *syms.get(id).ok_or("JBIG2: simbolo inexistente.")?;
            let ri = if t.refine { ok(c.ints.ri.decode(mq))? } else { 0 };
            let refined;
            let ib: &Bitmap = if ri == 0 {
                base
            } else {
                let rdw = ok(c.ints.rdw.decode(mq))?;
                let rdh = ok(c.ints.rdh.decode(mq))?;
                let rdx = ok(c.ints.rdx.decode(mq))?;
                let rdy = ok(c.ints.rdy.decode(mq))?;
                let w = (base.w as i64 + rdw).max(0) as usize;
                let h = (base.h as i64 + rdh).max(0) as usize;
                let r = Refine { template: t.rtemplate, reference: base, dx: rdw.div_euclid(2) + rdx, dy: rdh.div_euclid(2) + rdy, at: t.rat };
                refined = decode_refinement(mq, &mut c.rcx, w, h, &r)?;
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
            b.compose(ib, x, y, t.combop);
            if !t.transposed && (rc == 1 || rc == 0) {
                curs += wi - 1;
            } else if t.transposed && (rc == 1 || rc == 3) {
                curs += hi - 1;
            }
            n += 1;
            if n > t.instances.saturating_mul(4) + 1024 {
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

fn decode_dict(mq: &mut Mq, d: &Dict, inputs: &[&Bitmap]) -> Result<Vec<Bitmap>, String> {
    if d.huff {
        return Err("JBIG2 con codificacion Huffman: no se decodifica.".into());
    }
    let mut ints = Ints::new();
    let mut gcx = generic_cx(d.template);
    let total = inputs.len() + d.new;
    let mut ctx = Ctx { ints: Ints::new(), iaid: Iaid::new(code_len(total)), rcx: refine_cx(d.rtemplate) };
    let mut new: Vec<Bitmap> = Vec::with_capacity(d.new.min(1 << 16));
    let mut hcheight: i64 = 0;
    let g = Generic { template: d.template, tpgdon: false, at: d.at.clone() };
    while new.len() < d.new {
        hcheight += ok(ints.dh.decode(mq))?;
        let mut symwidth: i64 = 0;
        while let Some(dw) = ints.dw.decode(mq) {
            symwidth += dw;
            if symwidth < 0 || hcheight < 0 || new.len() >= d.new {
                return Err("JBIG2: tamano de simbolo invalido.".into());
            }
            let (w, h) = (symwidth as usize, hcheight as usize);
            let bitmap = if !d.refagg {
                decode_generic(mq, &mut gcx, w, h, &g)?
            } else {
                // Los contextos de enteros son los mismos del diccionario.
                std::mem::swap(&mut ctx.ints, &mut ints);
                let result = refine_or_aggregate(mq, &mut ctx, d, inputs, &new, w, h);
                std::mem::swap(&mut ctx.ints, &mut ints);
                result?
            };
            new.push(bitmap);
        }
    }
    // Simbolos exportados: corridas alternadas de "no" y "si" sobre entradas y nuevos.
    let mut flags = Vec::with_capacity(total);
    let mut cur = false;
    while flags.len() < total {
        let run = ok(ints.ex.decode(mq))?;
        if run < 0 || flags.len() + run as usize > total {
            return Err("JBIG2: lista de simbolos exportados invalida.".into());
        }
        flags.extend(std::iter::repeat_n(cur, run as usize));
        cur = !cur;
    }
    let mut out = Vec::with_capacity(d.exported);
    for (i, f) in flags.iter().enumerate() {
        if *f {
            out.push(if i < inputs.len() { inputs[i].clone() } else { new[i - inputs.len()].clone() });
        }
    }
    Ok(out)
}

fn refine_or_aggregate(mq: &mut Mq, c: &mut Ctx, d: &Dict, inputs: &[&Bitmap], new: &[Bitmap], w: usize, h: usize) -> Result<Bitmap, String> {
    let instances = ok(c.ints.ai.decode(mq))?;
    let syms: Vec<&Bitmap> = inputs.iter().copied().chain(new.iter()).collect();
    if instances == 1 {
        let id = c.iaid.decode(mq);
        let rdx = ok(c.ints.rdx.decode(mq))?;
        let rdy = ok(c.ints.rdy.decode(mq))?;
        let reference = *syms.get(id).ok_or("JBIG2: simbolo inexistente.")?;
        let r = Refine { template: d.rtemplate, reference, dx: rdx, dy: rdy, at: d.rat };
        decode_refinement(mq, &mut c.rcx, w, h, &r)
    } else {
        let t = Text {
            w,
            h,
            instances: instances.max(0) as usize,
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
        decode_text(mq, c, &t, &syms)
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
        let end = if len == u32::MAX { d.len() } else { (i + len as usize).min(d.len()) };
        out.push(Segment { number, kind, refs, data: &d[i..end] });
        i = end;
        if kind == 51 {
            break;
        }
    }
    Ok(out)
}

/// Informacion de region: ancho, alto, x, y y operador de combinacion.
fn region_info(d: &[u8]) -> Result<(usize, usize, i64, i64, u8), String> {
    let get = |k| be32(d, k).ok_or_else(|| "JBIG2: region incompleta.".to_string());
    Ok((get(0)? as usize, get(4)? as usize, get(8)? as i64, get(12)? as i64, d.get(16).copied().unwrap_or(0) & 7))
}

fn at_pixels(d: &[u8], at: usize, n: usize) -> Vec<(i64, i64)> {
    (0..n).map(|k| (d.get(at + 2 * k).copied().unwrap_or(0) as i8 as i64, d.get(at + 2 * k + 1).copied().unwrap_or(0) as i8 as i64)).collect()
}

/// Decodifica la pagina de un flujo JBIG2 de PDF (con sus segmentos globales, si los hay) en un
/// mapa de `width` x `height` (los del diccionario de la imagen).
pub fn decode(data: &[u8], globals: Option<&[u8]>, width: usize, height: usize) -> Result<Bitmap, String> {
    let mut all = Vec::new();
    if let Some(g) = globals {
        all.extend(segments(g)?);
    }
    all.extend(segments(data)?);
    let mut dicts: HashMap<u32, Vec<Bitmap>> = HashMap::new();
    let mut page: Option<Bitmap> = None;
    let mut page_op = 0u8;
    for s in &all {
        match s.kind {
            48 => {
                // Informacion de pagina: color por defecto y operador por defecto.
                let flags = s.data.get(16).copied().unwrap_or(0);
                page_op = (flags >> 3) & 3;
                page = Some(Bitmap::new(width, height, (flags >> 2) & 1)?);
            }
            0 => {
                let f = be16(s.data, 0).ok_or("JBIG2: diccionario incompleto.")?;
                let (huff, refagg) = (f & 1 != 0, f & 2 != 0);
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
                let inputs: Vec<&Bitmap> = s.refs.iter().filter_map(|r| dicts.get(r)).flatten().collect();
                let d = Dict { huff, refagg, template, rtemplate, at, rat, exported, new };
                let mut mq = Mq::new(&s.data[(i + 8).min(s.data.len())..]);
                let syms = decode_dict(&mut mq, &d, &inputs)?;
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
                let syms: Vec<&Bitmap> = s.refs.iter().filter_map(|r| dicts.get(r)).flatten().collect();
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
                let mut c = Ctx { ints: Ints::new(), iaid: Iaid::new(code_len(syms.len())), rcx: refine_cx(rtemplate) };
                let mut mq = Mq::new(&s.data[(i + 4).min(s.data.len())..]);
                let region = decode_text(&mut mq, &mut c, &t, &syms)?;
                let p = page.as_mut().ok_or("JBIG2: region antes de la pagina.")?;
                p.compose(&region, x, y, if op == 0 { page_op.max(op) } else { op });
            }
            38 | 39 => {
                let (w, h, x, y, op) = region_info(s.data)?;
                let f = *s.data.get(17).ok_or("JBIG2: region generica incompleta.")?;
                let template = (f >> 1) & 3;
                let region = if f & 1 != 0 {
                    // MMR: grupo 4 de fax, con 1 = negro igual que JBIG2.
                    let p = ccitt::Params { k: -1, columns: w, rows: h, byte_align: false, end_of_block: true };
                    let (rows, _) = ccitt::decode(&s.data[18.min(s.data.len())..], &p, h);
                    let stride = w.div_ceil(8);
                    let mut b = Bitmap::new(w, h, 0)?;
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
                    let mut mq = Mq::new(&s.data[(18 + 2 * n).min(s.data.len())..]);
                    // Alto desconocido (0xFFFFFFFF): hasta el alto de la pagina.
                    let h = if h == u32::MAX as usize { height.saturating_sub(y.max(0) as usize) } else { h };
                    decode_generic(&mut mq, &mut cx, w, h, &g)?
                };
                let p = page.as_mut().ok_or("JBIG2: region antes de la pagina.")?;
                p.compose(&region, x, y, op);
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
        let back = decode_generic(&mut mq, &mut generic_cx(0), w, h, &g).unwrap();
        assert_eq!(back.px, src.px);
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
        // Codifica con el mismo procedimiento IAx (A.2) y decodifica.
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
}
