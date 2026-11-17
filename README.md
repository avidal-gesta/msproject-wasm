# Conversores para la web: mpp2xlsx y doc2json

Dos herramientas independientes, escritas en **Rust sin ninguna dependencia**, que corren igual como ejecutable de Windows y como WebAssembly dentro de una página web:

| Herramienta | Entrada | Salida | Ejecutable | WebAssembly |
|---|---|---|---|---|
| **mpp2xlsx** | Microsoft Project (`.mpp`) | Excel (`.xlsx`) con Gantt, y JSON | 569 KB | 339 KB (93 KB con brotli) |
| **doc2json** | Word (`.docx`), PDF, PowerPoint (`.pptx`) y Excel (`.xlsx`) | JSON con el contenido estructurado, Markdown y fragmentos, e imágenes | 738 KB | 651 KB (203 KB con brotli) |

No necesitan Microsoft Project, Office, Java ni conexión a internet. Todo el código que lee los formatos es propio: contenedor OLE2, formato MPP, ZIP, DEFLATE, XML, PDF y sus cifrados.

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

```bash
target\release\doc2json.exe informe.pdf --markdown
```

```bash
target\release\doc2json.exe manual.docx --fragmentos=1500
```

```bash
target\release\doc2json.exe informe.pdf --imagenes
```

**En una página web:** copia a tu proyecto los archivos de `web/` y llama a los cargadores como se muestra en [Integración en una web](#integración-en-una-web-vercel--supabase). La conversión ocurre en el navegador; el archivo no se sube a ningún lado.

En `ejemplo/` hay un procedimiento de muestra en `.docx` y `.pdf` con sus JSON y su Markdown.

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

## doc2json: Word, PDF, PowerPoint y Excel a JSON

Los cuatro formatos salen con **la misma forma**, así una sola tabla en la base de datos sirve para todos:

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
| `page` (en cada bloque) | PDF: página. PowerPoint: diapositiva. Excel: número de hoja. |
| `headers`, `footers`, `footnotes` | Encabezados, pies y notas al pie: de Word y de PDF etiquetados, cuando existen. |
| `endnotes`, `comments` | Solo Word, cuando existen. |
| `notes` | Solo PowerPoint: notas del orador, con el número de la diapositiva en `label`. |
| `images` | Imágenes sin repetir: formato, tamaño en píxeles, texto alternativo y dónde se muestran (ver [Imágenes](#imágenes)). |
| `warnings` | Avisos: páginas escaneadas sin texto, índice reconstruido, fuentes ilegibles. |

Con `--full` (o `{ full: true }` en la web) se agregan estilo y alineación de cada bloque, los tramos con formato (negrita, cursiva, enlaces) y, en PDF, cada línea con su posición y tamaño de letra.

### Markdown y fragmentos (para modelos de lenguaje y búsqueda)

Los mismos bloques salen también como **Markdown** (`--markdown`, `docToMarkdown`): títulos con `#` según su nivel, listas con su numeración o viñeta e indentación, tablas GFM (las celdas combinadas dejan su lugar vacío, sin filas ni columnas en blanco), notas al pie como `[^1]` y, en Word, **negrita**, *cursiva* y enlaces. En PDF cada página empieza con `<!-- pagina N -->`, para citar de dónde sale cada parte.

Y como **fragmentos** listos para indexar o enviar a un modelo (`--fragmentos[=N]`, `docToChunks`), de hasta N caracteres (2000 por defecto, unos 500 tokens en castellano):

```json
{ "type": "docx", "metadata": { … }, "page_count": 30,
  "chunks": [ { "index": 4, "path": ["3. DEFINICIONES Y TÉRMINOS"], "chars": 1821, "text": "**AST**\n\n- Análisis de Seguridad en el Trabajo. …" } ] }
```

- Cada fragmento empieza en un título cuando se puede y trae en `path` los títulos de las secciones que lo contienen; en PDF, `pages` con la primera y la última página.
- Nunca corta un bloque, salvo que el bloque solo sea más grande que el fragmento: un párrafo se corta al final de una oración, una tabla por filas repitiendo el encabezado, y una fila con una celda enorme en filas de continuación con las mismas columnas.
- Una sección más corta que 200 caracteres se junta con la siguiente; un título queda con el bloque que lo sigue.
- Muchas plantillas escriben los títulos de sección como una tabla de una sola celda ("1. OBJETIVO"): se tratan como título, con el nivel que dé su numeración.
- Las notas al pie van en un fragmento final con ruta `["Notas"]`.

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
- **Orden de lectura**: el del árbol (`/K`), aunque un párrafo cruce de columna o de página. La posición solo arma las líneas: las de cada contenido marcado y las que comparten contenidos seguidos (un subíndice, un enlace o una letra en negrita en medio de una palabra).
- **Notas al pie**: la cita queda en el texto como `[1]` y la nota va en `footnotes`, como en Word. El rótulo de la nota es su `Lbl` o, si no lo trae, el número o símbolo con que empieza su texto («Ver anexo 2» no tiene rótulo).
- Los **encabezados, pies y números de página** vienen marcados como artefactos: quedan fuera del contenido y van en `headers` y `footers`, como en Word, una vez cada texto aunque se repita en todas las páginas (siguen también en el texto de cada página). Un artefacto que no se cierra (`BMC`/`BDC` sin su `EMC`) no se lleva el resto de la página: lo que quedó dentro vuelve a ser contenido.
- El texto que no pertenece a ningún elemento (firmas, contenido sin etiquetar) se agrega por posición después de los bloques de su página, para no perder nada. Si el árbol cubre menos de la mitad del texto, se usa el método por posición para todo el documento.
- `metadata.tagged` vale `true` cuando la estructura salió de las etiquetas.

Límites ante árboles dañados o malintencionados: 500.000 elementos y contenidos marcados, 64 niveles de elementos y 8 arreglos o referencias encadenados entre un elemento y sus hijos (los ciclos se cortan). Una tabla de más de 1.000 columnas o 200.000 celdas, contando el relleno de las combinadas, o que pase de 1.000.000 de celdas sumando todas las del documento, sale como párrafos y con un aviso en `warnings`.

Con el `ejemplo/` de este repositorio, los bloques del PDF etiquetado son idénticos a los del `.docx` original (prueba `pdf_etiquetado_igual_al_docx`).

Sin etiquetas (escáneres, generadores de informes, PDF antiguos), la estructura se **deduce**: un PDF no guarda párrafos ni tablas, sino letras sueltas con coordenadas.

- Las letras se agrupan en palabras y líneas según su posición; el texto girado o en páginas rotadas se endereza.
- **Títulos** por tamaño de letra respecto del cuerpo del documento (o negrita con numeración de sección).
- **Tablas** por alineación: filas consecutivas cuyas celdas caen en las mismas columnas.
- **Listas** por su viñeta o numeración inicial.
- Páginas a dos columnas de texto corrido: se lee primero la columna izquierda.
- Incluye el texto de **firmas digitales, sellos y campos de formulario**, que no está en el contenido de la página sino en anotaciones.
- Abre PDF "protegidos" contra edición o copia (cifrado RC4, AES-128 y AES-256 sin contraseña de apertura).
  Con AES-256 no se comprueba `/Perms` (la copia cifrada de los permisos) ni se prueba la contraseña de propietario: no hacen falta para leer, porque la clave del archivo sale de la contraseña de usuario vacía.
- Si el índice interno del archivo está dañado, lo reconstruye recorriendo el archivo.

### PowerPoint (.pptx)

- Diapositivas en el orden de la presentación (no en el de los archivos internos); `page_count` es su cantidad.
- Dentro de cada diapositiva, **orden de lectura**: el título primero y luego de arriba hacia abajo y de izquierda a derecha, también dentro de los grupos de formas.
- El título sale como `heading`; el texto de los marcadores de contenido, como `list_item` con su nivel y su viñeta o numeración automática (`1.`, `a)`, `IV.`); los cuadros de texto, como párrafos.
- Tablas con celdas combinadas, imágenes con su texto alternativo y notas del orador aparte, en `notes`.
- Se omite lo que no es contenido (número de diapositiva, fecha y pie) y las diapositivas ocultas, con un aviso en `warnings`.

### Excel (.xlsx)

- Cada hoja visible sale como un `heading` con su nombre y una `table` con los valores **como los muestra Excel**: fechas y horas en ISO 8601 (también en libros con sistema de fechas 1904), porcentajes con `%`, tiempos transcurridos (`[h]:mm`) en horas totales, `TRUE`/`FALSE` y errores (`#N/A`) tal cual.
- Números sin ruido de coma flotante: `0.30000000000000004` sale `0.3`.
- De las fórmulas se toma el último valor que calculó Excel; no se recalculan.
- Se quitan las filas y columnas vacías, también las intermedias; las celdas combinadas pasan a `colspan`/`rowspan`.
- Se omiten las hojas ocultas, con un aviso en `warnings`.

### Imágenes

`images` lista cada imagen **una vez**, aunque se muestre varias (un logo en cada página), con un `id` desde 1:

```json
{ "id": 3, "format": "png", "width": 1240, "height": 620, "alt": "Diagrama del circuito",
  "places": [{ "page": 2, "x": 72, "y": 142, "w": 300, "h": 150 }] }
```

- **Word:** cada imagen sigue en `content` como bloque `image` en su lugar de lectura, ahora con `"image": <id>`; las que están dentro de una tabla (pictogramas, firmas) quedan citadas desde su celda, en `images`. `places` trae el tamaño con que se muestra (`w`, `h`, en puntos). Las de encabezados y pies figuran en la lista aunque esos bloques salgan como texto.
- **PDF:** `places` dice en qué página y dónde se dibuja cada vez: `x`, `y` (esquina superior izquierda, medida desde la esquina superior izquierda de la página en su orientación de lectura, igual que `y` de las líneas), `w` y `h`, en puntos. `width` y `height` son los píxeles de la imagen.
- **PowerPoint:** bloque `image` con su `id` en la diapositiva, y `places` con la diapositiva (`page`) y su rectángulo en puntos. Dentro de un grupo, el rectángulo va en las coordenadas del grupo (casi siempre las de la diapositiva). Las de diapositivas ocultas no se listan.
- **Excel:** las imágenes de los dibujos de cada hoja, como bloques `image` después de su tabla; `places` solo dice la hoja (`page`), porque Excel las ancla a celdas y no a puntos.

**Extraer los archivos:** `--imagenes[=carpeta]` en la consola (quedan como `imagen-<id>.<formato>`), `docToJson(archivo, { images: true })` en la web (cada imagen trae sus bytes en `data`, un `Uint8Array`, con `mime` y `size`; `minSide: 32` omite los bytes de iconos y líneas) y `doc2json::read_with_images` en Rust.

| Origen | Se entrega como |
|---|---|
| Word, PowerPoint y Excel | El archivo incrustado tal cual (PNG, JPEG, GIF, EMF, WMF, SVG…). |
| PDF con JPEG (DCTDecode) o JPEG 2000 (JPXDecode) | El mismo JPEG o JPEG 2000, sin recomprimir. |
| PDF con fax (CCITTFaxDecode, grupos 3 y 4) o JBIG2 | PNG de un bit, decodificado: escaneos en blanco y negro listos para OCR o para un modelo de visión. Un fax que no se puede decodificar se entrega tal cual, en un TIFF. |
| PDF con píxeles (Flate, LZW, RunLength o sin comprimir) | PNG: gris, RGB, CMYK (convertido a RGB), indexado con su paleta, tinta plana y máscaras de un bit, con transparencia si traen máscara suave del mismo tamaño (también la premezclada con `/Matte`). Si los datos ya son un PNG por dentro (Flate con predictor PNG), se copian sin recomprimir. |

No se convierten, y figuran con `note`: JBIG2 con codificación Huffman, semitonos o regiones intermedias (no aparecieron en más de mil imágenes JBIG2 reales), espacios de color Lab o DeviceN de varias tintas, e imágenes de más de 25 millones de píxeles (100 millones si son de un bit, como los escaneos). Las imágenes incrustadas en el contenido de la página (`BI … EI`, poco comunes) se cuentan pero no se extraen.

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
import { init, docToJson, docToMarkdown, docToChunks } from '@/lib/doc2json';

async function convertirDocumento(file: File) {
  await init('/doc2json.wasm');
  const bytes = new Uint8Array(await file.arrayBuffer());
  const doc = await docToJson(bytes);                            // { type, metadata, content, text, ... }
  const md = await docToMarkdown(bytes);                         // Markdown con tablas GFM
  const { chunks } = await docToChunks(bytes, { maxChars: 1500 }); // [{ path, pages, text, ... }]
  return { doc, md, chunks };
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

### Límites de recursos

Cada lectura tiene un presupuesto: cuánto se puede descomprimir en total (todas las partes del `.docx` o todos los flujos del PDF), cuántas páginas de PDF se leen y cuánto texto se junta. Protege contra archivos hechos para agotar la memoria (un `.docx` de 1,5 MB puede descomprimir 250 MB) y permite acotar el costo de cada documento. Al alcanzar un límite **no se rechaza el archivo**: se entrega lo leído hasta ahí, con un aviso en `warnings`.

| Límite | Por defecto | Web (`limits`) | Consola |
|---|---|---|---|
| Datos descomprimidos en total | 128 MB | `maxDecompressedMB` | `--max-mb=N` |
| Páginas de PDF | 20 000 | `maxPages` | `--max-paginas=N` |
| Caracteres de texto | 50 000 000 | `maxChars` | `--max-caracteres=N` |

```ts
// En una función con poca memoria (p. ej. 256 MB), conviene bajarlos:
const doc = await docToJson(bytes, { limits: { maxDecompressedMB: 32, maxPages: 500 } });
```

Los valores por defecto son holgados: en 900 documentos de oficina reales el máximo fue 40 MB descomprimidos (mediana 0,7 MB), 572 páginas y 2,5 millones de caracteres, y con ellos la salida es idéntica byte a byte a la de una lectura sin límites. Con un `.docx` de 1,5 MB que descomprime 250 MB, la memoria máxima del proceso pasa de 2,9 GB (sin presupuesto) a 875 MB por defecto y a 464 MB con `maxDecompressedMB: 32`. En Rust: `doc2json::read_with_limits(bytes, Limits { … })`.

### Guardar documentos en Supabase

```sql
create table documentos (
  id          uuid primary key default gen_random_uuid(),
  archivo     text not null,
  tipo        text not null,                 -- 'docx', 'pdf', 'pptx' o 'xlsx'
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
- Fragmentos: el tamaño máximo se respeta siempre, salvo una fila de una tabla muy ancha (decenas de columnas con texto) con un tamaño chico, donde ni repartiéndola cabe junto al encabezado. Probado con 896 documentos reales: 0 fragmentos sobre el máximo con 2000 caracteres y 1 de 57.558 con 800.
- En PDF sin etiquetas la estructura es una deducción. Funciona bien en documentos de oficina, informes y certificados; puede equivocarse con diseños muy libres (afiches, planos, revistas a varias columnas con recuadros).
- En PDF sin etiquetas las tablas se detectan por alineación del texto, no por sus líneas dibujadas. Una tabla con celdas de varias líneas puede salir partida en más filas.
- En PDF etiquetado vale lo que diga el etiquetado. Word no exporta las combinaciones verticales de celdas (`RowSpan`), así que esas celdas salen vacías en vez de `merged`. No se leen las etiquetas de formularios incrustados (`/Stm`) ni las anotaciones referidas desde el árbol (`OBJR`): su texto igual se recupera por posición.
- No abre los PDF que piden contraseña para abrirse ni los protegidos con certificados o DRM: da un mensaje claro.
- Si un PDF usa fuentes sin tabla de caracteres, ese texto es ilegible para cualquier extractor (también al copiar y pegar desde un visor).
- En PDF las imágenes no se insertan en `content` (que se deduce del texto): se ubican con `places`.
- Los PNG se comprimen con códigos Huffman fijos: una imagen sin compresión en el PDF puede quedar más pesada que con un compresor completo. Las fotografías suelen venir como JPEG, que se entrega tal cual.
- En Word y PowerPoint no se lee el texto de gráficos ni de diagramas SmartArt; en Excel no se leen gráficos ni comentarios.
- En Excel no se recalculan fórmulas ni se aplican los formatos de número completos (separador de miles, decimales fijos, moneda): los números salen con su valor exacto.
- No lee `.doc`, `.xls` ni `.ppt` antiguos (Office 97-2003), ni libros binarios `.xlsb`, ni archivos de Office con contraseña de apertura: da un mensaje pidiendo guardarlos en el formato actual.

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
- Descifrado probado con RC4 de 40 y 128 bits, AES-128 y AES-256 (revisiones 5 y 6), con y sin flujos de objetos: mismo texto que el original. 150 PDF reales guardados por MuPDF sin cifrar, con AES-128 y con AES-256 dan exactamente el mismo JSON. La revisión 5 se prueba además con un PDF escrito por pypdf (`doc2json/tests/datos/`), y la 6 con datos de MuPDF.
- Páginas apaisadas y rotadas (90°, 180°, 270°) y texto a dos columnas: mismo orden de lectura que la referencia.

**doc2json, PowerPoint y Excel** contra python-pptx y openpyxl

- 162 presentaciones reales: coincide el 99,8 % de los caracteres y no hay texto duplicado. El resto también está, cortado distinto: los saltos de línea dentro de una celda de tabla se conservan como tales.
- 1.003 planillas reales (2,2 millones de celdas con valor): coincide el 99,8 % de las celdas. Lo que no coincide son textos con escapes `_xHHHH_` que la referencia deja sin decodificar (Excel los muestra decodificados, como aquí), valores tapados por una combinación de celdas que Excel no muestra, y 112 números.
- Imágenes, contra las que entregan python-pptx y openpyxl: en las 162 presentaciones y los 1.003 libros no falta ninguna y los bytes son idénticos. Aquí aparecen además 92 imágenes de PowerPoint que python-pptx no expone (formas rellenas con una imagen y figuras dentro de `mc:AlternateContent`) y 387 de Excel que openpyxl no carga (por ejemplo, las ancladas a una sola celda, como el logo de muchas exportaciones).

**doc2json, imágenes** contra PyMuPDF y Pillow

- 300 PDF reales con imágenes (8.812 imágenes): se extraen 8.807 (las otras 5 pasan de 25 millones de píxeles) y todas abren en Pillow con el tamaño declarado (3.520 JPEG, 5.162 PNG, 125 TIFF). De los PNG comparables con el mapa de píxeles de PyMuPDF, 3.336 de 3.338 difieren en a lo sumo 1 nivel por canal; los otros 2 usan perfiles ICC, que PyMuPDF aplica y aquí no.
- Lugares: 54.044 contra 54.230 de PyMuPDF; en páginas sin girar, el 98,8 % coincide con su rectángulo con menos de 1,5 puntos de diferencia.
- 200 Word reales (10.426 imágenes): los bytes extraídos son idénticos a los del archivo incrustado en todos los casos, y el 98,3 % queda citado desde un bloque o una celda (el resto está en encabezados y pies).
- Escaneos en fax y JBIG2 (84 y 7 PDF reales): las 3.780 imágenes de fax y las 1.092 JBIG2 comparadas son idénticas píxel a píxel a las de PyMuPDF; solo quedan fuera 15 planos de más de 100 millones de píxeles. Además, 354 imágenes armadas para cubrir todos los largos de corrida y texto, codificadas en grupos 3 y 4 por libtiff, se decodifican idénticas. Mediana de 15 ms por imagen de fax y 154 ms por página JBIG2.

**Resistencia**

- 1.500 PDF (6.127 páginas) y 400 Word reales: ningún fallo ni cuelgue; 51 ms por archivo en promedio.
- 162 presentaciones y 1.003 planillas reales: ningún fallo ni cuelgue; mediana de 39 ms por presentación y 16 ms por planilla. Los únicos errores fueron archivos de bloqueo de Office y uno truncado, con un mensaje claro.
- 460 archivos dañados a propósito (truncados, con bytes alterados, con tramos borrados): todos terminan en lectura parcial o en un error claro.
- Archivos "bomba" armados en `doc2json/tests/presupuesto.rs` (un `.docx` de ~250 KB que descomprime 40 MB, un PDF con un flujo que descomprime 30 MB, un PDF de 50 páginas): se cortan en el límite pedido con su aviso, sin afectar a la lectura siguiente.
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
  budget.rs             presupuesto de recursos de cada lectura (descompresión, páginas, texto)
  inflate.rs            descompresor DEFLATE / zlib
  zipread.rs  xml.rs    lector ZIP y lector XML
  docx.rs               lector de Word: estilos, numeración, tablas, notas
  pptx.rs               lector de PowerPoint: orden de lectura, viñetas, tablas, notas
  xlsx.rs               lector de Excel: textos compartidos, formatos de fecha, combinaciones
  image.rs  png.rs      lista de imágenes, formato por cabecera y escritor PNG
  ccitt.rs              fax CCITT grupos 3 y 4 (T.4 y T.6), también el modo MMR de JBIG2
  jbig2.rs              JBIG2 (T.88): decodificador aritmético, regiones genéricas, de texto y refinamiento
  pdf/
    object.rs           objetos de PDF y su analizador
    document.rs         tabla de referencias, objetos, páginas, metadatos
    filters.rs          filtros de flujos (Flate, LZW, ASCII85, predictores…)
    crypt.rs            MD5, SHA-2, RC4, AES-128/256 y manejador de seguridad estándar
    encoding.rs         codificaciones y nombres de glifos
    font.rs             fuentes y tablas ToUnicode
    content.rs          intérprete del contenido de página (incluido el contenido marcado)
    layout.rs           de letras con coordenadas a líneas, párrafos y tablas
    structure.rs        PDF etiquetado: árbol de estructura a títulos, listas, tablas y notas
    images.rs           imágenes de PDF: ubicación y conversión a PNG, JPEG o TIFF
  doc.rs                modelo común de documento y salida JSON
  markdown.rs           Markdown (tablas GFM, notas) y fragmentos por sección
  symbols.rs            fuentes Symbol y Wingdings a Unicode
doc2json/tests/
  ejemplo.rs            pruebas de punta a punta con los archivos de ejemplo/
  presupuesto.rs        archivos "bomba" armados en la prueba y los límites de recursos
  office.rs             PowerPoint y Excel con paquetes armados en la prueba
  imagenes.rs           imágenes de Word y PDF armados en la prueba
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
