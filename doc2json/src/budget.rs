//! Presupuesto de recursos de una lectura: cuanto se puede descomprimir en total, cuantas paginas
//! leer y cuanto texto juntar. Protege contra archivos hechos para agotar la memoria (un .docx o
//! un PDF con muchos flujos "bomba", millones de paginas vacias) y permite acotar el costo de cada
//! documento, por ejemplo en una funcion serverless con poca memoria.
//!
//! Cada flujo ya tiene su propio tope (`inflate::MAX_OUTPUT`); este presupuesto suma todos los de
//! un mismo documento. Al agotarse, la lectura no falla: entrega lo leido hasta ahi y lo informa en
//! `warnings`. Solo si se agota antes de llegar al contenido (p. ej. una bomba en los estilos de un
//! .docx que ademas deja sin presupuesto al cuerpo) la lectura falla, con un error que lo explica.
//!
//! El texto tambien se acota mientras se lee (`take_text`), no solo al final: asi un .docx con
//! millones de caracteres no los junta en memoria si el limite es menor.
//!
//! El estado es por hilo y por lectura: `read_with_limits` lo abre y lo cierra.

use std::cell::RefCell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Bytes que se pueden descomprimir en total (todas las partes y flujos del archivo).
    pub max_decompressed: usize,
    /// Paginas de PDF que se leen; las siguientes se cuentan pero no se procesan.
    pub max_pages: usize,
    /// Caracteres de texto que se juntan; lo que sigue se descarta.
    pub max_chars: usize,
}

impl Default for Limits {
    fn default() -> Self {
        // Holgados: en 900 documentos de oficina reales el maximo fue 40 MB descomprimidos, 572
        // paginas y 2,5 millones de caracteres (ver README).
        Limits { max_decompressed: 128 << 20, max_pages: 20_000, max_chars: 50_000_000 }
    }
}

impl Limits {
    /// Limites en las unidades de la interfaz WebAssembly y de la consola: MB descomprimidos,
    /// paginas y caracteres. Un 0 deja el valor por defecto de ese limite.
    pub fn from_units(mb: u32, pages: u32, chars: u32) -> Limits {
        let d = Limits::default();
        let pick = |v: u32, def: usize, unit: usize| if v == 0 { def } else { (v as usize).saturating_mul(unit) };
        Limits { max_decompressed: pick(mb, d.max_decompressed, 1 << 20), max_pages: pick(pages, d.max_pages, 1), max_chars: pick(chars, d.max_chars, 1) }
    }
}

/// Lo gastado en una lectura, al cerrarla.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spent {
    /// Bytes descomprimidos en total.
    pub decompressed: usize,
    /// Se corto algun flujo por falta de presupuesto de descompresion.
    pub decompressed_hit: bool,
    /// Se descarto texto por el limite de caracteres mientras se leia.
    pub text_hit: bool,
}

#[derive(Default)]
struct State {
    active: bool,
    limits: Option<Limits>,
    spent: Spent,
    /// Caracteres que aun se pueden juntar.
    chars_left: usize,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Abre el presupuesto de una lectura.
pub fn begin(limits: Limits) {
    STATE.with(|s| *s.borrow_mut() = State { active: true, limits: Some(limits), spent: Spent::default(), chars_left: limits.max_chars });
}

/// Cierra el presupuesto y devuelve lo gastado y que topes se alcanzaron.
pub fn end() -> Spent {
    STATE.with(|s| std::mem::take(&mut *s.borrow_mut()).spent)
}

/// Limites vigentes (los por defecto si no hay una lectura abierta).
pub fn limits() -> Limits {
    STATE.with(|s| s.borrow().limits.unwrap_or_default())
}

/// Cuanto se puede descomprimir ahora, sin pasar `cap` (el tope por flujo).
pub fn room(cap: usize) -> usize {
    STATE.with(|s| {
        let st = s.borrow();
        match (st.active, st.limits) {
            (true, Some(l)) => l.max_decompressed.saturating_sub(st.spent.decompressed).min(cap),
            _ => cap,
        }
    })
}

/// Anota bytes descomprimidos. `cut` indica que la salida se corto por falta de presupuesto.
pub fn spend(bytes: usize, cut: bool) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.active {
            st.spent.decompressed = st.spent.decompressed.saturating_add(bytes);
            if cut {
                st.spent.decompressed_hit = true;
            }
        }
    })
}

/// La parte de `s` que cabe en el limite de caracteres de la lectura en curso (todo `s` si no hay
/// una abierta), y la descuenta. Los lectores la usan al juntar texto para no acumular de mas.
pub fn take_text(s: &str) -> &str {
    STATE.with(|st| {
        let mut st = st.borrow_mut();
        if !st.active {
            return s;
        }
        let left = st.chars_left;
        // Byte donde termina el caracter numero `left` (o el final, si `s` es mas corto).
        let (end, taken) = match s.char_indices().nth(left) {
            Some((i, _)) => (i, left),
            None => (s.len(), s.chars().count()),
        };
        st.chars_left -= taken;
        if end < s.len() {
            st.spent.text_hit = true;
        }
        &s[..end]
    })
}

/// Aviso legible del tope de descompresion alcanzado.
pub fn decompressed_warning(limit: usize) -> String {
    format!(
        "Se alcanzo el limite de datos descomprimidos ({} MB): el documento se leyo solo en parte. Un archivo legitimo rara vez llega a ese tamano; puede estar danado o hecho para agotar la memoria.",
        limit >> 20
    )
}

/// Error de una lectura que agoto el presupuesto de descompresion antes de llegar al contenido.
/// `cause` es el error que dio el lector con los datos que alcanzo a descomprimir.
pub fn decompressed_error(limit: usize, cause: &str) -> String {
    format!(
        "Se alcanzo el limite de datos descomprimidos ({} MB) antes de llegar al contenido del documento ({}). Un archivo legitimo rara vez llega a ese tamano; puede estar danado o hecho para agotar la memoria.",
        limit >> 20,
        cause.trim_end_matches('.')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sin_lectura_abierta_no_limita() {
        assert_eq!(room(1000), 1000);
        spend(10, true);
        assert_eq!(take_text("hola"), "hola");
        assert_eq!(end(), Spent::default());
    }

    #[test]
    fn suma_y_corta_al_agotarse() {
        begin(Limits { max_decompressed: 100, ..Limits::default() });
        assert_eq!(room(1000), 100);
        spend(70, false);
        assert_eq!(room(1000), 30);
        assert_eq!(room(10), 10);
        spend(30, true);
        assert_eq!(room(1000), 0);
        assert_eq!(end(), Spent { decompressed: 100, decompressed_hit: true, text_hit: false });
        // Al cerrar, todo vuelve a no limitar.
        assert_eq!(room(1000), 1000);
    }

    #[test]
    fn el_texto_se_descuenta_por_caracteres() {
        begin(Limits { max_chars: 5, ..Limits::default() });
        assert_eq!(take_text("ñan"), "ñan");
        assert_eq!(take_text("dúes"), "dú");
        assert_eq!(take_text("x"), "");
        assert_eq!(end(), Spent { text_hit: true, ..Spent::default() });
        // Justo en el limite no se descarta nada.
        begin(Limits { max_chars: 3, ..Limits::default() });
        assert_eq!(take_text("abc"), "abc");
        assert!(!end().text_hit);
    }

    #[test]
    fn unidades_de_la_interfaz() {
        assert_eq!(Limits::from_units(0, 0, 0), Limits::default());
        let l = Limits::from_units(32, 500, 1000);
        assert_eq!((l.max_decompressed, l.max_pages, l.max_chars), (32 << 20, 500, 1000));
        // En wasm32 (usize de 32 bits) los MB enormes se saturan en vez de desbordar.
        assert_eq!(Limits::from_units(u32::MAX, 1, 1).max_decompressed, (u32::MAX as usize).saturating_mul(1 << 20));
    }
}
