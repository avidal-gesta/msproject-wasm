// Cargador de doc2json.wasm para el navegador y Node. Sin dependencias.
//
//   await init('/doc2json.wasm');          // una sola vez (en Node: init(bytes del .wasm))
//   const doc = await docToJson(archivo);  // { type, metadata, content, text, ... }
//
// Acepta .docx y .pdf; el formato se reconoce por el contenido. La conversion ocurre donde
// corre este codigo: el archivo no se envia a ningun servidor.

let modulo; // Promise<WebAssembly.Module>
let instancia; // Promise con los exports de la instancia en uso

async function compilar(origen) {
  if (origen instanceof WebAssembly.Module) return origen;
  if (origen instanceof ArrayBuffer || ArrayBuffer.isView(origen)) return WebAssembly.compile(origen);
  const respuesta = await (typeof origen === 'string' || origen instanceof URL ? fetch(origen) : origen);
  if (!respuesta.ok) throw new Error(`No se pudo descargar doc2json.wasm (HTTP ${respuesta.status}).`);
  // compileStreaming exige el tipo application/wasm; si el servidor no lo envia, se compila desde los bytes.
  const tipo = respuesta.headers.get('content-type') || '';
  if (WebAssembly.compileStreaming && tipo.startsWith('application/wasm')) return WebAssembly.compileStreaming(respuesta);
  return WebAssembly.compile(await respuesta.arrayBuffer());
}

/** Descarga y compila el modulo. Llamarla varias veces no lo descarga de nuevo. */
export function init(origen) {
  if (!modulo) {
    if (origen == null) return Promise.reject(new TypeError('init() necesita la URL o los bytes de doc2json.wasm.'));
    const intento = compilar(origen);
    modulo = intento;
    // Si la descarga falla, un init() posterior puede reintentar.
    intento.catch(() => {
      if (modulo === intento) modulo = undefined;
    });
  }
  return modulo.then(() => undefined);
}

async function convertir(archivo, modo) {
  if (!modulo) throw new Error('Llame primero a init() con la ubicacion de doc2json.wasm.');
  if (typeof Blob !== 'undefined' && archivo instanceof Blob) archivo = await archivo.arrayBuffer();
  const entrada = archivo instanceof Uint8Array ? archivo : new Uint8Array(archivo);
  if (!instancia) instancia = modulo.then((m) => WebAssembly.instantiate(m, {})).then((i) => i.exports);
  const usada = instancia;
  const x = await usada;
  // Desde aqui no hay ningun await: dos conversiones simultaneas no se mezclan dentro del modulo.
  try {
    const ptr = x.doc_alloc(entrada.length);
    if (entrada.length && !ptr) throw new Error('No hay memoria suficiente para un archivo de este tamano.');
    new Uint8Array(x.memory.buffer, ptr, entrada.length).set(entrada);
    const estado = x.doc_convert(ptr, entrada.length, modo);
    const salida = new TextDecoder().decode(new Uint8Array(x.memory.buffer, x.doc_result_ptr(), x.doc_result_len()));
    x.doc_free(ptr, entrada.length);
    if (estado !== 0) throw new Error(salida);
    return salida;
  } catch (e) {
    if (e instanceof WebAssembly.RuntimeError) {
      // El modulo aborto (tipicamente por falta de memoria) y esa instancia ya no es confiable:
      // la siguiente conversion crea una nueva.
      if (instancia === usada) instancia = undefined;
      throw new Error('No se pudo convertir el archivo: el conversor se quedo sin memoria o el archivo esta danado.');
    }
    throw e;
  }
}

/** Convierte un .docx o un .pdf en JSON, como texto. Con `full` agrega estilos, tramos con formato y lineas de PDF. */
export function docToJsonText(archivo, opciones) {
  return convertir(archivo, opciones && opciones.full ? 1 : 0);
}

/** Convierte un .docx o un .pdf en un objeto con metadatos, bloques de contenido y texto plano. */
export async function docToJson(archivo, opciones) {
  return JSON.parse(await docToJsonText(archivo, opciones));
}

/** Convierte un .docx o un .pdf en Markdown: titulos, listas, tablas GFM y notas al pie. */
export function docToMarkdown(archivo) {
  return convertir(archivo, 2);
}

/**
 * Corta un .docx o un .pdf en fragmentos de hasta `maxChars` caracteres (2000 por defecto), con
 * la ruta de titulos de su seccion y las paginas que cubren. Listo para indexar o enviar a un modelo.
 */
export async function docToChunks(archivo, opciones) {
  const n = Math.floor((opciones && opciones.maxChars) || 0);
  if (n < 0 || n > 0xffffff) throw new RangeError('maxChars debe estar entre 1 y 16777215.');
  return JSON.parse(await convertir(archivo, 3 + n * 256));
}
