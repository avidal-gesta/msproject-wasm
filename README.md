# Conversores para la web: mpp2xlsx y doc2json

Dos herramientas independientes, escritas en **Rust sin ninguna dependencia**, que corren igual como ejecutable de Windows y como WebAssembly dentro de una página web:

| Herramienta | Entrada | Salida | Ejecutable | WebAssembly |
|---|---|---|---|---|
| **mpp2xlsx** | Microsoft Project (`.mpp`) | Excel (`.xlsx`) con Gantt, y JSON | 569 KB | 339 KB (93 KB con brotli) |
| **doc2json** | Word (`.docx`) y PDF | JSON con el contenido estructurado | 596 KB | 378 KB (120 KB con brotli) |

No necesitan Microsoft Project, Word, Java ni conexión a internet. Todo el código que lee los formatos es propio: contenedor OLE2, formato MPP, ZIP, DEFLATE, XML, PDF y sus cifrados.

## Uso rápido

**Por consola:**

```bash
target\release\mpp2xlsx.exe plan.mpp
```

```bash
target\release\mpp2xlsx.exe plan.mpp salida.xlsx --json salida.json
```

```bash
target\release\doc2json.exe informe.pdf
```

```bash
target\release\doc2json.exe contrato.docx salida.json --full
```

**En una página web:** copia a tu proyecto los archivos de `web/` y llama a los cargadores como se muestra en [Integración en una web](#integración-en-una-web-vercel--supabase). La conversión ocurre en el navegador; el archivo no se sube a ningún lado.

En `ejemplo/` hay un procedimiento de muestra en `.docx` y `.pdf` con sus JSON.

---

## mpp2xlsx: Project a Excel

### Qué trae el Excel

| Hoja | Contenido |
|---|---|
| **Gantt** | ID, tarea, duración, inicio, fin, % avance, predecesoras y recursos, más las barras en escala de tiempo. La escala se ajusta sola (tramos de 15 min a semanas). |
| **Tareas** | Las mismas columnas, en el mismo orden y con los mismos títulos que la tabla del Gantt guardada en el `.mpp` (encabezado azul oscuro). A la derecha, datos de estructura (gris) y el resto de campos personalizados que tengan algún valor (azul claro). |
| **Recursos** | La hoja de recursos de Project: unidades, grupo, calendario base, trabajo y campos personalizados. |
| **Asignaciones** | Una fila por recurso asignado a cada tarea. |
| **Proyecto** | Datos generales, conteos y leyenda de colores. |

Detalles que se conservan de Project:

- Las filas se agrupan por nivel de esquema: se contraen y expanden con los botones 1-2-3 del margen.
- Tareas resumen en negrita; tareas inactivas en gris y tachadas.
- Los campos personalizados usan su alias (`Order Number`, `RWC`, `Respaldo Recursos`…), no `Number1` o `Text11`.
- Las barras del Gantt son **formato condicional** sobre `Start` y `Finish`: si alguien cambia una fecha en Excel, la barra se mueve.
- Fechas y duraciones son valores reales de Excel (se pueden filtrar, ordenar y sumar), no texto.

`--json-full` agrega a cada tarea todos los campos estándar de Project (fechas tempranas y tardías, holguras, etc.).

---

## doc2json: Word y PDF a JSON

Los dos formatos salen con **la misma forma**, así una sola tabla en la base de datos sirve para ambos:

```json
{
  "type": "pdf",
  "metadata": { "title": "…", "author": "…", "created": "2026-11-13T19:52:22-03:00", "producer": "…" },
  "page_count": 1,
  "stats": { "characters": 699, "words": 105, "headings": 6, "paragraphs": 4, "list_items": 6, "tables": 1, "images": 0 },
  "content": [
    { "type": "heading", "level": 1, "text": "Procedimiento de bloqueo de equipos", "page": 1 },
    { "type": "paragraph", "text": "Este procedimiento aplica a:", "page": 1 },
    { "type": "list_item", "level": 0, "ordered": false, "marker": "•", "text": "Personal de mantención mecánica", "page": 1 },
    { "type": "list_item", "level": 0, "ordered": true, "marker": "2.", "text": "Abrir el interruptor y colocar candado y tarjeta.", "page": 1 },
    { "type": "table", "rows": [[{ "text": "Cargo" }, { "text": "Tarea" }, { "text": "Registro" }]], "page": 1 }
  ],
  "pages": [{ "number": 1, "width": 612, "height": 792, "text": "…" }],
  "text": "texto plano completo, listo para búsqueda"
}
```

| Clave | Contenido |
|---|---|
| `metadata` | Título, autor, fechas (ISO 8601), programa de origen y demás propiedades del archivo. |
| `content` | Bloques en orden de lectura: `title`, `heading` (con `level`), `paragraph`, `list_item` (con `marker` y `ordered`), `table` (filas de celdas con `colspan`/`rowspan`), `image` y `page_break`. |
| `text` | Todo el texto plano. |
| `pages` | Solo PDF: tamaño y texto de cada página. |
| `outline` | Solo PDF: marcadores (tabla de contenidos) con su página. |
| `headers`, `footers`, `footnotes` | Encabezados, pies y notas al pie: de Word y de PDF etiquetados, cuando existen. |
| `endnotes`, `comments` | Solo Word, cuando existen. |
| `warnings` | Avisos: páginas escaneadas sin texto, índice reconstruido, fuentes ilegibles. |

Con `--full` (o `{ full: true }` en la web) se agregan estilo y alineación de cada bloque, los tramos con formato (negrita, cursiva, enlaces) y, en PDF, cada línea con su posición y tamaño de letra.

### Word (.docx)

- Títulos por su estilo real (`Título 1`, `Título 2`…), no por apariencia.
- **Numeración y viñetas calculadas.** En un `.docx` el "1.", "a)" o "3.2.1" de una lista o de un título numerado no está escrito en el texto: lo dibuja Word. Aquí se reconstruye y va en `marker`.
- Tablas con celdas combinadas, tablas anidadas y texto de cuadros de texto.
- Notas al pie citadas en el texto como `[1]` y listadas aparte.
- Se omite lo que no se ve: texto eliminado con control de cambios, texto oculto y códigos de campo.
- Viñetas de las fuentes Symbol y Wingdings traducidas a caracteres normales (`•`, `▪`, `✓`).

### PDF

**PDF etiquetado.** Word, LibreOffice y la mayoría de los programas de oficina exportan el PDF con un árbol de estructura (Tagged PDF, ISO 32000-1 §14.8) que dice qué es cada cosa. Cuando existe, los bloques salen de ahí y no de la posición del texto:

- **Tablas** con sus filas y celdas reales, aunque una celda ocupe varias líneas; las celdas combinadas (`RowSpan`/`ColSpan`) dejan su lugar marcado con `merged` en las filas de abajo, igual que en Word.
- **Títulos** con su nivel (`H1`…`H6`, `Title`), **listas** con su viñeta o numeración (`Lbl`) y las anidadas un nivel más adentro, y **figuras** con su texto alternativo.
- **Notas al pie**: la cita queda en el texto como `[1]` y la nota va en `footnotes`, como en Word.
- Los **encabezados, pies y números de página** vienen marcados como artefactos: quedan fuera del contenido y van en `headers` y `footers`, como en Word, una vez cada texto aunque se repita en todas las páginas (siguen también en el texto de cada página).
- El texto que no pertenece a ningún elemento (firmas, contenido sin etiquetar) se agrega por posición después de los bloques de su página, para no perder nada. Si el árbol cubre menos de la mitad del texto, se usa el método por posición para todo el documento.
- `metadata.tagged` vale `true` cuando la estructura salió de las etiquetas.

Con el `ejemplo/` de este repositorio, los bloques del PDF etiquetado son idénticos a los del `.docx` original (prueba `pdf_etiquetado_igual_al_docx`).

Sin etiquetas (escáneres, generadores de informes, PDF antiguos), la estructura se **deduce**: un PDF no guarda párrafos ni tablas, sino letras sueltas con coordenadas.

- Las letras se agrupan en palabras y líneas según su posición; el texto girado o en páginas rotadas se endereza.
- **Títulos** por tamaño de letra respecto del cuerpo del documento (o negrita con numeración de sección).
- **Tablas** por alineación: filas consecutivas cuyas celdas caen en las mismas columnas.
- **Listas** por su viñeta o numeración inicial.
- Páginas a dos columnas de texto corrido: se lee primero la columna izquierda.
- Incluye el texto de **firmas digitales, sellos y campos de formulario**, que no está en el contenido de la página sino en anotaciones.
- Abre PDF "protegidos" contra edición o copia (cifrado RC4 y AES-128 sin contraseña de apertura).
- Si el índice interno del archivo está dañado, lo reconstruye recorriendo el archivo.

---

## Integración en una web (Vercel + Supabase)

Copia a tu proyecto los archivos de `web/` de la herramienta que necesites:

```
public/mpp2xlsx.wasm   lib/mpp2xlsx.js   lib/mpp2xlsx.d.ts
public/doc2json.wasm   lib/doc2json.js   lib/doc2json.d.ts
```

Cada módulo es independiente: una página que solo convierte documentos no descarga el lector de Project, y viceversa.

`init()` recibe la URL del `.wasm` (o sus bytes, en Node) y se puede llamar varias veces: el módulo se descarga una sola vez. Las funciones de conversión aceptan un `Uint8Array`, un `ArrayBuffer` o directamente el `File` de un `<input type="file">`, y rechazan con un `Error` cuyo mensaje explica el problema (archivo dañado, formato no soportado, protegido con contraseña).

### En el navegador (recomendado)

Es lo más liviano: no gasta tiempo de función en Vercel, no tiene límite de tamaño de subida y el archivo no sale del equipo del usuario.

```tsx
'use client';
import { init, mppToXlsx, mppToJson, XLSX_MIME } from '@/lib/mpp2xlsx';

async function convertirPlan(file: File) {
  await init('/mpp2xlsx.wasm');                          // se descarga una sola vez
  const bytes = new Uint8Array(await file.arrayBuffer());
  const xlsx = await mppToXlsx(bytes);                   // Uint8Array para descargar
  const datos = await mppToJson(bytes);                  // { project, tasks, resources, assignments }
  return { blob: new Blob([xlsx], { type: XLSX_MIME }), datos };
}
```

```tsx
'use client';
import { init, docToJson } from '@/lib/doc2json';

async function convertirDocumento(file: File) {
  await init('/doc2json.wasm');
  return docToJson(new Uint8Array(await file.arrayBuffer()));   // { type, metadata, content, text, ... }
}
```

### En una función de Vercel (Node)

Útil si el archivo ya está en Supabase Storage o llega por otra vía. Los módulos no importan nada del sistema: solo necesitan `WebAssembly`, que Node trae incluido.

```ts
// app/api/documento/route.ts
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { init, docToJsonText } from '@/lib/doc2json';

export async function POST(req: Request) {
  await init(await readFile(path.join(process.cwd(), 'public/doc2json.wasm')));
  const json = await docToJsonText(new Uint8Array(await req.arrayBuffer()));
  return new Response(json, { headers: { 'content-type': 'application/json' } });
}
```

Dos advertencias sobre esta variante:

- Los cargadores están probados en Node y en el navegador, pero este ejemplo de ruta no se ha probado desplegado en Vercel. Si la función no encuentra el `.wasm`, hay que incluirlo en el empaquetado con `outputFileTracingIncludes` en `next.config.js`.
- Las funciones de Vercel aceptan cuerpos de hasta 4,5 MB. Para archivos más grandes, convierte en el navegador o lee el archivo desde Storage dentro de la función.

Si la web define una política de seguridad de contenido (CSP) estricta, `script-src` debe incluir `'wasm-unsafe-eval'`.

### Guardar documentos en Supabase

```sql
create table documentos (
  id          uuid primary key default gen_random_uuid(),
  archivo     text not null,
  tipo        text not null,                 -- 'docx' o 'pdf'
  titulo      text,
  autor       text,
  paginas     int,
  palabras    int,
  texto       text,                          -- texto plano completo
  contenido   jsonb not null,                -- bloques: títulos, párrafos, listas, tablas
  metadatos   jsonb not null default '{}',
  creado_en   timestamptz not null default now(),
  busqueda    tsvector generated always as
              (to_tsvector('spanish', coalesce(titulo, '') || ' ' || coalesce(texto, ''))) stored
);

create index on documentos using gin (busqueda);
```

```ts
const doc = await docToJson(bytes);

await supabase.from('documentos').insert({
  archivo: file.name, tipo: doc.type,
  titulo: doc.metadata.title ?? null, autor: doc.metadata.author ?? null,
  paginas: doc.page_count, palabras: doc.stats.words,
  texto: doc.text, contenido: doc.content, metadatos: doc.metadata,
});
```

Con la columna `busqueda` se puede buscar dentro de todos los documentos en español:

```sql
select archivo, titulo from documentos
where busqueda @@ websearch_to_tsquery('spanish', 'bloqueo candado');
```

PostgreSQL limita cada `tsvector` a 1 MB. Un documento de varios cientos de páginas puede superarlo; en ese caso conviene guardar una fila por página usando `doc.pages`.

### Guardar planes de Project en Supabase

Las fechas de Project son hora local sin zona, por eso `timestamp` y no `timestamptz`:

```sql
create table proyectos (
  id            uuid primary key default gen_random_uuid(),
  archivo       text not null,
  titulo        text,
  inicio        timestamp,
  fin           timestamp,
  fecha_estado  timestamp,
  creado_en     timestamptz not null default now()
);

create table tareas (
  proyecto_id     uuid not null references proyectos(id) on delete cascade,
  uid             int  not null,          -- identificador estable de la tarea en el .mpp
  id              int  not null,          -- número de fila en Project
  nombre          text,
  nivel           int,
  edt             text,
  padre_uid       int,
  inicio          timestamp,
  fin             timestamp,
  duracion_horas  numeric,
  avance          numeric,
  es_resumen      boolean,
  es_hito         boolean,
  es_critica      boolean,
  activa          boolean,
  predecesoras    text,
  recursos        text,
  trabajo_horas   numeric,
  campos          jsonb not null default '{}',  -- campos personalizados (Order Number, RWC, ...)
  primary key (proyecto_id, uid)
);

create index on tareas using gin (campos);
```

```ts
const { project, tasks } = await mppToJson(bytes);

const { data: proyecto } = await supabase
  .from('proyectos')
  .insert({ archivo: file.name, titulo: project.title, inicio: project.start, fin: project.finish, fecha_estado: project.status_date })
  .select('id')
  .single();

await supabase.from('tareas').insert(
  tasks.map((t) => ({
    proyecto_id: proyecto.id,
    uid: t.uid, id: t.id, nombre: t.name, nivel: t.outline_level, edt: t.wbs, padre_uid: t.parent_uid,
    inicio: t.start, fin: t.finish, duracion_horas: t.duration_hours, avance: t.percent_complete,
    es_resumen: t.summary, es_hito: t.milestone, es_critica: t.critical, activa: t.active,
    predecesoras: t.predecessors, recursos: t.resource_names, trabajo_horas: t.work_hours,
    campos: t.custom,
  })),
);
```

En ambos casos las tablas necesitan una política de seguridad (RLS) que permita insertar al usuario que sube el archivo.

---

## Rendimiento en el navegador

Medido con el motor de JavaScript de Chrome (V8) en un PC de escritorio. Cada conversión usa un solo núcleo y libera su memoria al terminar.

| Archivo | Tiempo | Memoria máxima |
|---|---|---|
| Plan de Project de 375 tareas (`.mpp` de 1,4 MB) → Excel | 110 ms | 14 MB |
| El mismo → JSON | 55 ms | 8 MB |
| PDF de 4 páginas | 12 ms | 2 MB |
| PDF de 25 páginas (1,9 MB) | 42 ms | 4 MB |
| Word de 49 páginas (3,9 MB) | 25 ms | 7 MB |
| PDF de 124 páginas (9 MB) | 260 ms | 14 MB |

La conversión corre en el hilo principal de la página, así que durante ese rato la página no responde. Con estos tamaños no se nota; para archivos mucho más grandes conviene moverla a un Web Worker.

## Compilar

Requisito único: [Rust](https://rustup.rs). La primera vez:

```bash
rustup target add wasm32-unknown-unknown
```

Después:

```bash
powershell -File construir.ps1
```

Ejecuta las pruebas, compila los dos `.exe` y los dos `.wasm`, deja estos últimos en `web/` y prueba los cargadores con `node web/prueba.mjs`. Para probarlos además con un plan propio:

```bash
node web/prueba.mjs plan.mpp
```

En Linux o macOS, los mismos pasos a mano:

```bash
cargo test --workspace
cargo build --release --workspace
cargo build --release --workspace --lib --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/*.wasm web/
node web/prueba.mjs
```

## Alcance y límites

**mpp2xlsx**

- Lee el formato MPP14: archivos guardados por Project 2010, 2013, 2016, 2019, 2021 y Microsoft 365. Un `.mpp` de Project 2007 o anterior da un mensaje claro pidiendo guardarlo de nuevo.
- No abre archivos con contraseña de apertura.
- No transcribe las columnas que Project calcula al vuelo y no guarda: `Indicators`, `Status` y `Status Indicator`.
- No lee los datos distribuidos en el tiempo (uso por día) ni las excepciones de calendario.

**doc2json**

- **No hace OCR.** Un PDF escaneado (solo imagen) no tiene texto que extraer; se informa en `warnings`.
- En PDF sin etiquetas la estructura es una deducción. Funciona bien en documentos de oficina, informes y certificados; puede equivocarse con diseños muy libres (afiches, planos, revistas a varias columnas con recuadros).
- En PDF sin etiquetas las tablas se detectan por alineación del texto, no por sus líneas dibujadas. Una tabla con celdas de varias líneas puede salir partida en más filas.
- En PDF etiquetado vale lo que diga el etiquetado. Word no exporta las combinaciones verticales de celdas (`RowSpan`), así que esas celdas salen vacías en vez de `merged`. No se leen las etiquetas de formularios incrustados (`/Stm`) ni las anotaciones referidas desde el árbol (`OBJR`): su texto igual se recupera por posición.
- No abre PDF con cifrado AES-256 ni los que piden contraseña para abrirse: da un mensaje claro.
- Si un PDF usa fuentes sin tabla de caracteres, ese texto es ilegible para cualquier extractor (también al copiar y pegar desde un visor).
- Las imágenes se cuentan y, en Word, se nombran; no se extraen.
- En Word no se lee el texto de gráficos ni de diagramas SmartArt.
- No lee `.doc` antiguos (Word 97-2003): da un mensaje pidiendo guardarlos como `.docx`.

**Ambos:** el texto se copia tal cual está en el archivo. Si el origen ya trae caracteres dañados (por ejemplo `Araï¿½a` en vez de `Araña` en una exportación desde SAP), salen igual.

## Cómo se validó

Cada lector se comparó contra la librería de referencia de su formato, sobre archivos reales:

**mpp2xlsx** contra [MPXJ](https://www.mpxj.org)

- Un plan real de 375 tareas, 84 recursos, 245 asignaciones y 185 relaciones, y 73 archivos de prueba de Project 2010 a 2019 (1.050 tareas): cero fallos y cero diferencias en nombres, fechas, duraciones, holguras, predecesoras, recursos y campos personalizados.
- El indicador de tarea crítica se contrastó además con XML exportado por el propio Project.
- El `.xlsx` se abrió en Excel de escritorio sin avisos de reparación.

**doc2json, Word** contra python-docx y contra el XML crudo

- 70 documentos reales (152.937 palabras, 873.805 caracteres visibles): no falta ni una palabra ni un carácter, y no hay contenido duplicado. Lo único que "sobra" son las viñetas y la numeración, que la referencia no calcula.

**doc2json, PDF** contra PyMuPDF

- 227 PDF elegidos por variedad de programa de origen (Word, Excel, Crystal Reports, LibreOffice, iText, wkhtmltopdf, Chrome, escáneres…), 794 páginas: coincide el 99,5 % de las palabras y falta el 0,3 % de los caracteres. Lo que falta está en fuentes sin tabla de caracteres, donde la referencia también devuelve texto ilegible.
- Descifrado probado con RC4 de 40 y 128 bits y AES-128, con y sin flujos de objetos: mismo texto que el original.
- Páginas apaisadas y rotadas (90°, 180°, 270°) y texto a dos columnas: mismo orden de lectura que la referencia.

**Resistencia**

- 1.500 PDF (6.127 páginas) y 400 Word reales: ningún fallo ni cuelgue; 51 ms por archivo en promedio.
- 460 archivos dañados a propósito (truncados, con bytes alterados, con tramos borrados): todos terminan en lectura parcial o en un error claro.
- En todas las herramientas, el archivo generado en el navegador es idéntico byte a byte al del ejecutable.

Los scripts de comparación y los archivos con que se hizo esta validación no forman parte de este repositorio.

## Estructura del código

```
src/                    mpp2xlsx
  cfb.rs                contenedor OLE2 (el "zip" interno de un .mpp)
  mpp/                  lector del formato MPP14
  schema.rs             qué columnas se muestran y qué valor va en cada celda
  xlsx.rs  zip.rs       escritor de Excel y compresor
  json.rs  wasm.rs  main.rs
doc2json/src/           doc2json
  inflate.rs            descompresor DEFLATE / zlib
  zipread.rs  xml.rs    lector ZIP y lector XML
  docx.rs               lector de Word: estilos, numeración, tablas, notas
  pdf/
    object.rs           objetos de PDF y su analizador
    document.rs         tabla de referencias, objetos, páginas, metadatos
    filters.rs          filtros de flujos (Flate, LZW, ASCII85, predictores…)
    crypt.rs            MD5, RC4, AES-128 y manejador de seguridad estándar
    encoding.rs         codificaciones y nombres de glifos
    font.rs             fuentes y tablas ToUnicode
    content.rs          intérprete del contenido de página (incluido el contenido marcado)
    layout.rs           de letras con coordenadas a líneas, párrafos y tablas
    structure.rs        PDF etiquetado: árbol de estructura a títulos, listas, tablas y notas
  doc.rs                modelo común de documento y salida JSON
  symbols.rs            fuentes Symbol y Wingdings a Unicode
doc2json/tests/
  ejemplo.rs            pruebas de punta a punta con los archivos de ejemplo/
web/
  mpp2xlsx.js  .d.ts  .wasm     módulo de Project
  doc2json.js  .d.ts  .wasm     módulo de documentos
  prueba.mjs                    prueba de los cargadores desde Node
```

## Origen del conocimiento de los formatos

- **MPP:** Microsoft no publica el formato. La estructura de los bloques y los identificadores de campo se tomaron del código fuente de [MPXJ](https://www.mpxj.org) (licencia LGPL 2.1), que es la documentación de hecho. Este proyecto es una implementación nueva en Rust, no una copia de ese código; por ese origen, mpp2xlsx se publica bajo la misma licencia.
- **PDF y DOCX:** son estándares públicos (ISO 32000 y ECMA-376). Se implementaron a partir de la especificación.

## Licencia

- **mpp2xlsx** (`src/`, `web/mpp2xlsx.*`): [LGPL-2.1-or-later](LICENSE).
- **doc2json** (`doc2json/`, `web/doc2json.*`): [MIT](doc2json/LICENSE-MIT) o [Apache-2.0](doc2json/LICENSE-APACHE), a elección.

Este proyecto no está afiliado a Microsoft ni cuenta con su respaldo. Microsoft Project, Word y Excel son marcas de Microsoft Corporation.
