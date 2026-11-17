//! Memoria que piden los PDF fabricados para agotarla con imagenes. Un asignador que cuenta el
//! maximo de bytes vivos mide cada caso; los casos corren uno tras otro (un solo `#[test]`) para
//! que otras pruebas no se mezclen en la cuenta.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

struct Contador;

static VIVOS: AtomicUsize = AtomicUsize::new(0);
static MAXIMO: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Contador {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            let v = VIVOS.fetch_add(l.size(), Relaxed) + l.size();
            MAXIMO.fetch_max(v, Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l);
        VIVOS.fetch_sub(l.size(), Relaxed);
    }
}

#[global_allocator]
static ASIGNADOR: Contador = Contador;

/// Maximo de bytes vivos por encima de los de antes, mientras corre `f`.
fn pico<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let base = VIVOS.load(Relaxed);
    MAXIMO.store(base, Relaxed);
    let r = f();
    (r, MAXIMO.load(Relaxed) - base)
}

fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{:010} 00000 n \n", o).as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objects.len() + 1, xref).as_bytes());
    out
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {} /Length {} >>\nstream\n", dict, data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

fn pagina(content: &str, image: Vec<u8>) -> Vec<u8> {
    pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 612 792] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /XObject << /Im1 5 0 R >> >> >>".to_vec(),
        stream("", content.as_bytes()),
        image,
    ])
}

const MB: usize = 1 << 20;

#[test]
fn imagenes_fabricadas_no_agotan_la_memoria() {
    // 1. Diccionario de imagen con una cadena de 1 MB, dibujado 400 veces: cada `Do` no debe
    //    copiar el diccionario (antes, 400 MB).
    let grande = format!("/Relleno <{}>", "41".repeat(MB));
    let dict = format!("/Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 {}", grande);
    let bytes = pagina(&"/Im1 Do ".repeat(400), stream(&dict, &[0]));
    let (d, usado) = pico(|| doc2json::read(&bytes).unwrap());
    assert_eq!(d.images[0].places.len(), 400);
    assert!(usado < 40 * MB, "dibujar 400 veces la misma imagen uso {} MB", usado / MB);

    // 2. 5.000 x 5.000 en RGB (75 MB) declarados con 150 bytes: no se reserva por el tamano declarado.
    let bytes = pagina("/Im1 Do", stream("/Subtype /Image /Width 5000 /Height 5000 /ColorSpace /DeviceRGB /BitsPerComponent 8", &[0; 150]));
    let (d, usado) = pico(|| doc2json::read_with_images(&bytes).unwrap());
    assert!(d.images[0].data.is_none());
    assert!(usado < 4 * MB, "una imagen de 150 bytes uso {} MB", usado / MB);

    // 3. Lo mismo con CMYK (se convierte a RGB) y con una mascara suave de pocos bytes.
    let bytes = pagina("/Im1 Do", stream("/Subtype /Image /Width 5000 /Height 5000 /ColorSpace /DeviceCMYK /BitsPerComponent 8", &[0; 150]));
    let (_, usado) = pico(|| doc2json::read_with_images(&bytes).unwrap());
    assert!(usado < 4 * MB, "CMYK de 150 bytes uso {} MB", usado / MB);
}
