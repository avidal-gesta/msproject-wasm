// Prueba los cargadores y los .wasm de esta carpeta desde Node:
//
//   node web/prueba.mjs                 documentos de ejemplo/ y manejo de errores
//   node web/prueba.mjs plan.mpp        ademas convierte ese .mpp; si existen plan.xlsx y
//                                       plan.json a su lado, exige que la salida sea identica
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import * as doc from './doc2json.js';
import * as mpp from './mpp2xlsx.js';

const aqui = path.dirname(fileURLToPath(import.meta.url));
const ejemplo = path.join(aqui, '..', 'ejemplo');
let fallos = 0;

function comprobar(nombre, ok, detalle = '') {
  if (!ok) fallos++;
  console.log(`${ok ? 'ok   ' : 'FALLA'} ${nombre}${detalle ? ` (${detalle})` : ''}`);
}

async function mensajeDeError(promesa) {
  try {
    await promesa;
    return null;
  } catch (e) {
    return e.message;
  }
}

const leerSiExiste = (ruta) => readFile(ruta).catch(() => null);

comprobar('convertir antes de init() se rechaza', (await mensajeDeError(doc.docToJson(new Uint8Array(4)))) !== null);
await doc.init(await readFile(path.join(aqui, 'doc2json.wasm')));
await mpp.init(await readFile(path.join(aqui, 'mpp2xlsx.wasm')));

for (const nombre of ['procedimiento.docx', 'procedimiento.pdf']) {
  const bytes = await readFile(path.join(ejemplo, nombre));
  const esperado = await readFile(path.join(ejemplo, `${nombre}.json`), 'utf8');
  const texto = await doc.docToJsonText(bytes);
  comprobar(`${nombre}: JSON identico al de ejemplo/`, texto === esperado);
  const objeto = await doc.docToJson(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength));
  comprobar(`${nombre}: objeto desde un ArrayBuffer`, objeto.type === nombre.split('.').pop() && objeto.content.length > 0);
  comprobar(`${nombre}: la salida completa trae mas datos`, (await doc.docToJsonText(bytes, { full: true })).length > texto.length);
  const md = await doc.docToMarkdown(bytes);
  comprobar(`${nombre}: Markdown identico al de ejemplo/`, md === (await readFile(path.join(ejemplo, `${nombre}.md`), 'utf8')));
  const frag = await doc.docToChunks(bytes, { maxChars: 300 });
  comprobar(`${nombre}: fragmentos con ruta de titulos y sin pasar el maximo`, frag.chunks.length >= 3 && frag.chunks.every((c) => c.chars <= 300 && Array.isArray(c.path)) && frag.chunks.some((c) => c.path.includes('Responsables')));
}

comprobar('docToChunks rechaza un tamano fuera de rango', /maxChars/.test(await mensajeDeError(doc.docToChunks(new Uint8Array(4), { maxChars: -1 }))));

comprobar('doc2json rechaza un archivo que no es documento', /no reconocido/.test(await mensajeDeError(doc.docToJson(new TextEncoder().encode('hola mundo')))));
comprobar('doc2json rechaza un archivo vacio', /vac/.test(await mensajeDeError(doc.docToJson(new Uint8Array(0)))));
comprobar('mpp2xlsx rechaza un archivo que no es .mpp', /no es un \.mpp/.test(await mensajeDeError(mpp.mppToXlsx(await readFile(path.join(ejemplo, 'procedimiento.pdf'))))));

// Contenedor OLE2 cuya cadena DIFAT apunta a si misma: debe dar un error claro, no agotar la memoria.
for (const [desplazamiento, largo] of [[9, 1024], [12, 8192]]) {
  const malo = new Uint8Array(largo);
  malo.set([0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
  malo[30] = desplazamiento;
  malo[32] = 6;
  comprobar(`mpp2xlsx rechaza un contenedor malicioso de ${largo} bytes`, /no soportado/.test(await mensajeDeError(mpp.mppToJson(malo))));
}

const plan = process.argv[2];
if (plan) {
  const bytes = await readFile(plan);
  const base = plan.replace(/\.mpp$/i, '');
  const xlsx = await mpp.mppToXlsx(bytes);
  comprobar('mpp -> xlsx es un ZIP', xlsx[0] === 0x50 && xlsx[1] === 0x4b, `${xlsx.length} bytes`);
  const texto = await mpp.mppToJsonText(bytes);
  const datos = JSON.parse(texto);
  comprobar('mpp -> json trae las tareas', datos.tasks.length > 0 && datos.tasks.length === datos.project.task_count, `${datos.tasks.length} tareas`);
  comprobar('mpp -> json completo trae los campos estandar', 'fields' in (await mpp.mppToJson(bytes, { full: true })).tasks[0]);
  const xlsxEsperado = await leerSiExiste(`${base}.xlsx`);
  if (xlsxEsperado) comprobar('mpp -> xlsx identico al guardado', Buffer.compare(Buffer.from(xlsx), xlsxEsperado) === 0);
  const jsonEsperado = await leerSiExiste(`${base}.json`);
  if (jsonEsperado) comprobar('mpp -> json identico al guardado', texto === jsonEsperado.toString('utf8'));
}

console.log(fallos === 0 ? 'Todo bien.' : `Fallos: ${fallos}`);
process.exit(fallos === 0 ? 0 : 1);
