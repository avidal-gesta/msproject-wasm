//! Interfaz WebAssembly "a mano" (sin wasm-bindgen), con el mismo protocolo que mpp2xlsx:
//!   1. `ptr = doc_alloc(n)` y copiar los bytes del documento en la memoria del modulo.
//!   2. `estado = doc_convert(ptr, n, modo)`: modo 0 = JSON, 1 = JSON completo, 2 = Markdown,
//!      3 = fragmentos en JSON (el tamano maximo en caracteres va en los bits altos: `3 | (n << 8)`).
//!   3. Leer `doc_result_len()` bytes desde `doc_result_ptr()`:
//!      con estado 0 es el JSON; con estado 1 es el mensaje de error, ambos en UTF-8.
//!   4. `doc_free(ptr, n)`.
//!
//! Opcional, antes de convertir: `doc_set_limits(mb, paginas, caracteres)` fija el presupuesto de
//! las conversiones siguientes (datos descomprimidos en MB, paginas, caracteres; 0 = por defecto).

use std::alloc::{alloc, dealloc, Layout};

static mut RESULT: Vec<u8> = Vec::new();
static mut LIMITS: Option<crate::budget::Limits> = None;

/// Fija los limites de las conversiones siguientes. Un 0 deja el valor por defecto de ese limite.
#[no_mangle]
pub extern "C" fn doc_set_limits(mb: u32, pages: u32, chars: u32) {
    let d = crate::budget::Limits::default();
    let pick = |v: u32, def: usize, unit: usize| if v == 0 { def } else { (v as usize).saturating_mul(unit) };
    let l = crate::budget::Limits { max_decompressed: pick(mb, d.max_decompressed, 1 << 20), max_pages: pick(pages, d.max_pages, 1), max_chars: pick(chars, d.max_chars, 1) };
    // WebAssembly aqui es de un solo hilo: no hay acceso concurrente a LIMITS.
    unsafe { *std::ptr::addr_of_mut!(LIMITS) = Some(l) };
}

#[no_mangle]
pub extern "C" fn doc_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::null_mut();
    }
    match Layout::from_size_align(len, 1) {
        Ok(layout) => unsafe { alloc(layout) },
        Err(_) => std::ptr::null_mut(),
    }
}

/// # Safety
/// `ptr` debe venir de `doc_alloc(len)` y no haberse liberado antes.
#[no_mangle]
pub unsafe extern "C" fn doc_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len != 0 {
        if let Ok(layout) = Layout::from_size_align(len, 1) {
            dealloc(ptr, layout);
        }
    }
}

/// # Safety
/// `ptr` debe apuntar a `len` bytes validos dentro de la memoria del modulo.
#[no_mangle]
pub unsafe extern "C" fn doc_convert(ptr: *const u8, len: usize, mode: u32) -> u32 {
    let input: &[u8] = if ptr.is_null() || len == 0 { &[] } else { std::slice::from_raw_parts(ptr, len) };
    let limits = unsafe { (*std::ptr::addr_of!(LIMITS)).unwrap_or_default() };
    let result = match mode & 0xff {
        0..=3 => crate::read_with_limits(input, limits).map(|d| match mode & 0xff {
            0 | 1 => crate::doc::to_json(&d, mode & 0xff == 1),
            2 => crate::markdown::to_markdown(&d),
            _ => {
                let size = match (mode >> 8) as usize {
                    0 => crate::markdown::DEFAULT_CHUNK,
                    n => n,
                };
                crate::markdown::chunks_json(&d, &crate::markdown::chunks(&d, size))
            }
        }),
        _ => Err("Modo de conversion desconocido.".to_string()),
    };
    let (status, bytes) = match result {
        Ok(s) => (0, s.into_bytes()),
        Err(e) => (1, e.into_bytes()),
    };
    // WebAssembly aqui es de un solo hilo: no hay acceso concurrente a RESULT.
    *std::ptr::addr_of_mut!(RESULT) = bytes;
    status
}

#[no_mangle]
pub extern "C" fn doc_result_ptr() -> *const u8 {
    unsafe { (*std::ptr::addr_of!(RESULT)).as_ptr() }
}

#[no_mangle]
pub extern "C" fn doc_result_len() -> usize {
    unsafe { (*std::ptr::addr_of!(RESULT)).len() }
}
