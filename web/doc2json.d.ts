/** De donde sale doc2json.wasm: una URL, una respuesta de `fetch`, sus bytes o un modulo ya compilado. */
export type WasmSource = string | URL | Response | Promise<Response> | BufferSource | WebAssembly.Module;

/** Archivo de entrada: sus bytes o directamente el `File` de un `<input type="file">`. */
export type FileInput = Uint8Array | ArrayBuffer | Blob;

/**
 * Presupuesto de recursos de una conversion. Al alcanzar un limite, el documento se entrega leido
 * hasta ahi con un aviso en `warnings` (no se rechaza). Los que se omiten quedan por defecto.
 */
export interface Limits {
  /** Datos descomprimidos en total, en MB (todas las partes y flujos del archivo). Por defecto 128. */
  maxDecompressedMB?: number;
  /** Paginas de PDF que se leen. Por defecto 20000. */
  maxPages?: number;
  /** Caracteres de texto. Por defecto 50 000 000. */
  maxChars?: number;
}

export interface JsonOptions {
  /** Agrega estilo y alineacion de cada bloque, los tramos con formato y, en PDF, las lineas de cada pagina. */
  full?: boolean;
  /** Limites de recursos de esta conversion (por ejemplo, mas bajos en una funcion con poca memoria). */
  limits?: Limits;
}

/** Descarga y compila el modulo. Llamarla varias veces no lo descarga de nuevo. */
export declare function init(source: WasmSource): Promise<void>;

/** Convierte un .docx o un .pdf en JSON, como texto. Rechaza con `Error` si el archivo no se puede leer. */
export declare function docToJsonText(file: FileInput, options?: JsonOptions): Promise<string>;

/** Convierte un .docx o un .pdf en un objeto con metadatos, bloques de contenido y texto plano. */
export declare function docToJson(file: FileInput, options?: JsonOptions): Promise<DocDocument>;

/**
 * Convierte un .docx o un .pdf en Markdown: titulos con `#`, listas con su numeracion, tablas GFM,
 * notas al pie como `[^1]` y, en Word, negrita, cursiva y enlaces. En PDF cada pagina empieza con
 * `<!-- pagina N -->`.
 */
export declare function docToMarkdown(file: FileInput, options?: { limits?: Limits }): Promise<string>;

export interface ChunkOptions {
  /** Tamano maximo de cada fragmento, en caracteres. Por defecto 2000 (unos 500 tokens en castellano); minimo 200. */
  maxChars?: number;
  /** Limites de recursos de la lectura, como en `docToJson`. */
  limits?: Limits;
}

export interface DocChunk {
  index: number;
  /** Titulos de las secciones que contienen el fragmento, del mas general al mas especifico. */
  path: string[];
  /** Solo PDF: primera y ultima pagina que cubre. */
  pages?: [number, number];
  chars: number;
  /** El fragmento en Markdown. */
  text: string;
}

export interface DocChunks {
  type: 'docx' | 'pdf';
  metadata: DocMetadata;
  page_count: number | null;
  chunks: DocChunk[];
}

/**
 * Corta un .docx o un .pdf en fragmentos para indexar o enviar a un modelo: cada uno empieza en un
 * titulo cuando se puede, nunca corta un bloque (salvo uno mas grande que el fragmento) y las tablas
 * largas se parten por filas repitiendo el encabezado.
 */
export declare function docToChunks(file: FileInput, options?: ChunkOptions): Promise<DocChunks>;

/** Propiedades del archivo. Solo aparecen las que el documento trae; las fechas van en ISO 8601. */
export interface DocMetadata {
  title?: string;
  author?: string;
  subject?: string;
  keywords?: string;
  created?: string;
  modified?: string;
  application?: string;
  /** Solo PDF. */
  producer?: string;
  pdf_version?: string;
  encrypted?: boolean;
  /** Solo PDF: la estructura salio del etiquetado del documento (Tagged PDF), no de la posicion del texto. */
  tagged?: boolean;
  [key: string]: string | number | boolean | undefined;
}

export interface DocStats {
  characters: number;
  words: number;
  headings: number;
  paragraphs: number;
  list_items: number;
  tables: number;
  images: number;
}

export interface DocCell {
  text: string;
  colspan?: number;
  rowspan?: number;
  /** Celda cubierta por una combinacion vertical iniciada mas arriba. */
  merged?: true;
}

export interface DocRun {
  text: string;
  bold?: true;
  italic?: true;
  underline?: true;
  strike?: true;
  link?: string;
}

export interface DocBlock {
  type: 'title' | 'heading' | 'paragraph' | 'list_item' | 'table' | 'image' | 'page_break';
  /** Titulos: 1 = principal. Items de lista: 0 = primer nivel. */
  level?: number;
  /** Items de lista: numerado o con vineta. */
  ordered?: boolean;
  /** Numeracion o vineta visible: "1.1", "a)", "•". */
  marker?: string;
  /** Notas y comentarios: su numero o su autor. */
  label?: string;
  /** Todos los bloques menos tablas, imagenes y saltos de pagina. */
  text?: string;
  /** Tablas: filas de celdas. */
  rows?: DocCell[][];
  /** Imagenes: nombre del archivo incrustado y texto alternativo. */
  name?: string;
  alt?: string;
  /** Solo PDF. */
  page?: number;
  /** Solo con `full`. */
  style?: string;
  align?: string;
  size?: number;
  bold?: true;
  runs?: DocRun[];
}

export interface DocLine {
  text: string;
  x: number;
  y: number;
  size: number;
  bold?: true;
}

export interface DocPage {
  number: number;
  width: number;
  height: number;
  text: string;
  images?: number;
  /** Solo con `full`. */
  lines?: DocLine[];
}

export interface DocOutlineItem {
  title: string;
  level: number;
  page?: number;
}

export interface DocDocument {
  type: 'docx' | 'pdf';
  metadata: DocMetadata;
  page_count: number | null;
  stats: DocStats;
  /** Bloques en orden de lectura. */
  content: DocBlock[];
  /** Solo PDF. */
  pages?: DocPage[];
  /** Solo PDF: marcadores (tabla de contenidos). */
  outline?: DocOutlineItem[];
  /** Notas al pie (Word y PDF etiquetado), con su numero en `label`; el texto las cita como "[n]". */
  footnotes?: DocBlock[];
  /** Encabezados y pies (Word y PDF etiquetado). En PDF, una vez cada texto aunque se repita en varias paginas. */
  headers?: DocBlock[];
  footers?: DocBlock[];
  /** Solo Word, cuando existen. */
  endnotes?: DocBlock[];
  comments?: DocBlock[];
  /** Avisos: paginas escaneadas sin texto, indice reconstruido, fuentes ilegibles. */
  warnings?: string[];
  /** Todo el texto plano. */
  text: string;
}
