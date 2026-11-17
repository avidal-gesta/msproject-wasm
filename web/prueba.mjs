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
{
  // Limites de recursos: con pocos caracteres se entrega la primera parte con un aviso, y los
  // limites de una llamada no quedan para la siguiente.
  const bytes = await readFile(path.join(ejemplo, 'procedimiento.docx'));
  const corto = await doc.docToJson(bytes, { limits: { maxChars: 100 } });
  comprobar('limits.maxChars corta el texto y avisa', (corto.warnings || []).some((w) => /limite de 100 caracteres/.test(w)));
  const completo = await doc.docToJson(bytes);
  comprobar('los limites no quedan para la conversion siguiente', !completo.warnings && completo.content.length > corto.content.length);
  const md = await doc.docToMarkdown(bytes, { limits: { maxChars: 100 } });
  comprobar('limits tambien acota el Markdown', md.length < (await doc.docToMarkdown(bytes)).length);
  const largo = (r) => r.chunks.reduce((t, c) => t + c.text.length, 0);
  const frag = await doc.docToChunks(bytes, { maxChars: 300, limits: { maxChars: 100 } });
  comprobar('limits tambien acota los fragmentos', largo(frag) < largo(await doc.docToChunks(bytes, { maxChars: 300 })));
  comprobar('limits rechaza valores invalidos', /maxPages/.test(await mensajeDeError(doc.docToJson(bytes, { limits: { maxPages: 0 } }))));
  // Un .wasm anterior (doc_convert con tres parametros) no puede ignorar los limites en silencio.
  // Modulo minimo armado a mano: solo exporta doc_convert(i32, i32, i32) -> i32.
  const anterior = await import('./doc2json.js?wasm-anterior');
  await anterior.init(new Uint8Array([
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x08, 0x01, 0x60, 0x03, 0x7f, 0x7f, 0x7f, 0x01, 0x7f,
    0x03, 0x02, 0x01, 0x00,
    0x07, 0x0f, 0x01, 0x0b, ...new TextEncoder().encode('doc_convert'), 0x00, 0x00,
    0x0a, 0x06, 0x01, 0x04, 0x00, 0x41, 0x00, 0x0b,
  ]));
  comprobar('limits con un .wasm anterior da error', /no admite limites/.test(await mensajeDeError(anterior.docToJson(bytes, { limits: { maxChars: 100 } }))));
}

// PDF minimo con una imagen RGB de 2 x 2 (sin comprimir) dibujada en la pagina.
function pdfConImagen() {
  const partes = [];
  const objetos = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 612 792] >>',
    '<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /XObject << /Im1 5 0 R >> >> >>',
    '<< /Length 33 >>\nstream\nq 100 0 0 50 72 600 cm /Im1 Do Q\nendstream',
    '<< /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 12 >>\nstream\n\xff\x00\x00\x00\xff\x00\x00\x00\xff\xff\xff\xff\nendstream',
  ];
  let texto = '%PDF-1.7\n';
  objetos.forEach((o, i) => {
    partes.push(texto.length);
    texto += `${i + 1} 0 obj\n${o}\nendobj\n`;
  });
  const xref = texto.length;
  texto += `xref\n0 ${objetos.length + 1}\n0000000000 65535 f \n${partes.map((p) => `${String(p).padStart(10, '0')} 00000 n \n`).join('')}`;
  texto += `trailer\n<< /Size ${objetos.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Uint8Array.from(texto, (c) => c.charCodeAt(0));
}

const conImagen = await doc.docToJson(pdfConImagen(), { images: true });
const imagen = conImagen.images && conImagen.images[0];
comprobar('doc2json extrae la imagen de un PDF como PNG', imagen && imagen.mime === 'image/png' && imagen.data instanceof Uint8Array && imagen.data[1] === 0x50 && imagen.data.length === imagen.size);
comprobar('doc2json ubica la imagen en la pagina', imagen && JSON.stringify(imagen.places) === '[{"page":1,"x":72,"y":142,"w":100,"h":50}]');
comprobar('doc2json no deja offset ni length en el objeto', imagen && !('offset' in imagen) && !('length' in imagen));
const chicas = await doc.docToJson(pdfConImagen(), { images: true, minSide: 3 });
comprobar('minSide omite los bytes de imagenes chicas', chicas.images[0].data === undefined && chicas.images[0].width === 2);
const sinExtraer = await doc.docToJson(pdfConImagen());
comprobar('sin images: lista sin bytes', sinExtraer.images.length === 1 && sinExtraer.images[0].data === undefined);
comprobar('minSide invalido se rechaza', (await mensajeDeError(doc.docToJson(pdfConImagen(), { images: true, minSide: -1 }))) !== null);
for (const malo of ['abc', NaN, 1.5, 0, '2000', 0x1000000]) {
  comprobar(`docToChunks rechaza maxChars ${String(malo)}`, /maxChars/.test(await mensajeDeError(doc.docToChunks(new Uint8Array(4), { maxChars: malo }))));
}
{
  const html = await doc.docToMarkdown(await readFile(path.join(ejemplo, 'procedimiento.docx')));
  comprobar('docToMarkdown no deja etiquetas HTML del texto', !/<(?!br>|!-- pagina)/.test(html));
}

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
