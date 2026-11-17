//! Interfaz WebAssembly "a mano" (sin wasm-bindgen), con el mismo protocolo que mpp2xlsx:
//!   1. `ptr = doc_alloc(n)` y copiar los bytes del documento en la memoria del modulo.
//!   2. `estado = doc_convert(ptr, n, modo, mb, paginas, caracteres)`. El byte bajo de `modo` es la operacion y los bits
//!      8 a 31 su parametro (`operacion | (parametro << 8)`):
//!
//!      | operacion | salida                     | parametro                               |
//!      |-----------|----------------------------|-----------------------------------------|
//!      | 0         | JSON                       | -                                       |
//!      | 1         | JSON completo              | -                                       |
//!      | 2         | Markdown                   | -                                       |
//!      | 3         | fragmentos en JSON         | tamano maximo en caracteres (0 = 2000)  |
//!      | 4         | JSON con imagenes          | lado minimo de las imagenes             |
//!      | 5         | JSON completo con imagenes | lado minimo de las imagenes             |
//!
//!      Una operacion que el modulo no conoce da error ("Modo de conversion desconocido."), nunca
//!      otra salida.
//!      `mb`, `paginas` y `caracteres` son los limites de recursos de esta conversion (datos
//!      descomprimidos en MB, paginas de PDF, caracteres de texto); 0 = valor por defecto. Van
//!      en cada llamada y no quedan para la siguiente. Quien llama con solo tres argumentos (un
//!      cargador anterior) recibe ceros: los valores por defecto.
//!   3. Leer `doc_result_len()` bytes desde `doc_result_ptr()`:
//!      con estado 0 es el JSON en UTF-8 o, con imagenes, `[largo del JSON: u32 LE][JSON][bytes de
//!      las imagenes]` (cada imagen del JSON trae su `offset` y `length` en ese bloque final);
//!      con estado 1 es el mensaje de error en UTF-8.
//!   4. `doc_free(ptr, n)`.

use std::alloc::{alloc, dealloc, Layout};

static mut RESULT: Vec<u8> = Vec::new();

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
pub unsafe extern "C" fn doc_convert(ptr: *const u8, len: usize, mode: u32, mb: u32, pages: u32, chars: u32) -> u32 {
    let input: &[u8] = if ptr.is_null() || len == 0 { &[] } else { std::slice::from_raw_parts(ptr, len) };
    let limits = crate::budget::Limits::from_units(mb, pages, chars);
    let kind = mode & 0xff;
    let result = match kind {
        0 | 1 => crate::read_with_limits(input, limits).map(|d| crate::doc::to_json(&d, kind == 1).into_bytes()),
        2 => crate::read_with_limits(input, limits).map(|d| crate::markdown::to_markdown(&d).into_bytes()),
        3 => crate::read_with_limits(input, limits).map(|d| {
            let size = match (mode >> 8) as usize {
                0 => crate::markdown::DEFAULT_CHUNK,
                n => n,
            };
            crate::markdown::chunks_json(&d, &crate::markdown::chunks(&d, size)).into_bytes()
        }),
        4 | 5 => crate::read_with_options(input, limits, true).map(|doc| {
            let (json, blob) = crate::doc::to_json_with_images(&doc, kind == 5, mode >> 8);
            let mut out = Vec::with_capacity(4 + json.len() + blob.len());
            out.extend_from_slice(&(json.len() as u32).to_le_bytes());
            out.extend_from_slice(json.as_bytes());
            out.extend_from_slice(&blob);
            out
        }),
        _ => Err("Modo de conversion desconocido.".to_string()),
    };
    let (status, bytes) = match result {
        Ok(b) => (0, b),
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
