//! Presupuesto de recursos de una lectura: cuanto se puede descomprimir en total, cuantas paginas
//! leer y cuanto texto juntar. Protege contra archivos hechos para agotar la memoria (un .docx o
//! un PDF con muchos flujos "bomba", millones de paginas vacias) y permite acotar el costo de cada
//! documento, por ejemplo en una funcion serverless con poca memoria.
//!
//! Cada flujo ya tiene su propio tope (`inflate::MAX_OUTPUT`); este presupuesto suma todos los de
//! un mismo documento. Al agotarse, la lectura no falla: entrega lo leido hasta ahi y lo informa en
//! `warnings`.
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

#[derive(Default)]
struct State {
    active: bool,
    limits: Option<Limits>,
    used: usize,
    /// Que tope se alcanzo, si alguno (para el aviso).
    decompressed_hit: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Abre el presupuesto de una lectura.
pub fn begin(limits: Limits) {
    STATE.with(|s| *s.borrow_mut() = State { active: true, limits: Some(limits), used: 0, decompressed_hit: false });
}

/// Cierra el presupuesto y devuelve lo descomprimido en total y si se alcanzo el tope.
pub fn end() -> (usize, bool) {
    STATE.with(|s| {
        let st = std::mem::take(&mut *s.borrow_mut());
        (st.used, st.decompressed_hit)
    })
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
            (true, Some(l)) => l.max_decompressed.saturating_sub(st.used).min(cap),
            _ => cap,
        }
    })
}

/// Anota bytes descomprimidos. `cut` indica que la salida se corto por falta de presupuesto.
pub fn spend(bytes: usize, cut: bool) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.active {
            st.used = st.used.saturating_add(bytes);
            if cut {
                st.decompressed_hit = true;
            }
        }
    })
}

/// Aviso legible del tope de descompresion alcanzado.
pub fn decompressed_warning(limit: usize) -> String {
    format!(
        "Se alcanzo el limite de datos descomprimidos ({} MB): el documento se leyo solo en parte. Un archivo legitimo rara vez llega a ese tamano; puede estar danado o hecho para agotar la memoria.",
        limit >> 20
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sin_lectura_abierta_no_limita() {
        assert_eq!(room(1000), 1000);
        spend(10, true);
        assert_eq!(end(), (0, false));
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
        assert_eq!(end(), (100, true));
        // Al cerrar, todo vuelve a no limitar.
        assert_eq!(room(1000), 1000);
    }
}
