//! Lectores de PowerPoint y Excel con paquetes armados dentro de la prueba: cada caso prueba
//! una regla de lectura (orden, vinetas, combinaciones, fechas, ocultas) sin depender de
//! archivos de terceros.

use doc2json::doc::{Block, Document, Kind};
use doc2json::{detect, read, Format};

/// ZIP sin compresion (metodo 0) con varias partes.
fn zip(partes: &[(&str, &str)]) -> Vec<u8> {
    let le16 = |v: usize| (v as u16).to_le_bytes();
    let le32 = |v: usize| (v as u32).to_le_bytes();
    let mut z = Vec::new();
    let mut central = Vec::new();
    for (nombre, contenido) in partes {
        let (n, c) = (nombre.as_bytes(), contenido.as_bytes());
        let desde = z.len();
        z.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&le32(c.len()));
        z.extend_from_slice(&le32(c.len()));
        z.extend_from_slice(&le16(n.len()));
        z.extend_from_slice(&[0, 0]);
        z.extend_from_slice(n);
        z.extend_from_slice(c);
        central.extend_from_slice(&[0x50, 0x4B, 0x01, 0x02, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&le32(c.len()));
        central.extend_from_slice(&le32(c.len()));
        central.extend_from_slice(&le16(n.len()));
        central.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&le32(desde));
        central.extend_from_slice(n);
    }
    let inicio = z.len();
    z.extend_from_slice(&central);
    z.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06, 0, 0, 0, 0]);
    z.extend_from_slice(&le16(partes.len()));
    z.extend_from_slice(&le16(partes.len()));
    z.extend_from_slice(&le32(central.len()));
    z.extend_from_slice(&le32(inicio));
    z.extend_from_slice(&[0, 0]);
    z
}

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn rels(lista: &[(&str, &str, &str)]) -> String {
    let mut s = String::from(r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (id, tipo, destino) in lista {
        let externo = if destino.starts_with("http") { r#" TargetMode="External""# } else { "" };
        s.push_str(&format!(r#"<Relationship Id="{}" Type="{}/{}" Target="{}"{}/>"#, id, REL, tipo, destino, externo));
    }
    s + "</Relationships>"
}

/// Forma de texto en la posicion (x, y), con marcador de posicion opcional.
fn forma(ph: &str, x: i64, y: i64, parrafos: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="f"/><p:cNvSpPr/><p:nvPr>{}</p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="{}" y="{}"/><a:ext cx="100" cy="100"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/>{}</p:txBody></p:sp>"#,
        ph, x, y, parrafos
    )
}

fn diapositiva(formas: &str, oculta: bool) -> String {
    format!(r#"<?xml version="1.0"?><p:sld {}{}><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/>{}</p:spTree></p:cSld></p:sld>"#, NS, if oculta { r#" show="0""# } else { "" }, formas)
}

fn presentacion() -> Vec<u8> {
    let tabla = r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="5" name="t"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="0" y="3000000"/></p:xfrm><a:graphic><a:graphicData><a:tbl><a:tblGrid><a:gridCol/><a:gridCol/><a:gridCol/></a:tblGrid>
        <a:tr><a:tc gridSpan="2"><a:txBody><a:p><a:r><a:t>Equipo</a:t></a:r></a:p></a:txBody></a:tc><a:tc hMerge="1"><a:txBody><a:p/></a:txBody></a:tc><a:tc rowSpan="2"><a:txBody><a:p><a:r><a:t>Riesgo</a:t></a:r></a:p></a:txBody></a:tc></a:tr>
        <a:tr><a:tc><a:txBody><a:p><a:r><a:t>Grua</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>50 t</a:t></a:r></a:p></a:txBody></a:tc><a:tc vMerge="1"><a:txBody><a:p/></a:txBody></a:tc></a:tr>
        </a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#;
    let imagen = r#"<p:pic><p:nvPicPr><p:cNvPr id="6" name="Imagen" descr="Diagrama del circuito"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId2"/></p:blipFill><p:spPr><a:xfrm><a:off x="0" y="5000000"/></a:xfrm></p:spPr></p:pic>"#;
    let grupo = format!(
        r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="7" name="g"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="4000000"/></a:xfrm></p:grpSpPr>{}{}</p:grpSp>"#,
        forma("", 5000000, 4000000, "<a:p><a:r><a:t>Grupo derecha</a:t></a:r></a:p>"),
        forma("", 0, 4000000, "<a:p><a:r><a:t>Grupo izquierda</a:t></a:r></a:p>"),
    );
    let s1 = diapositiva(
        &[
            // Un cuadro de texto mas arriba que el titulo: el titulo igual sale primero.
            forma("", 0, 100000, r#"<a:p><a:r><a:rPr b="1"/><a:t>Nota previa</a:t></a:r><a:r><a:rPr><a:hlinkClick r:id="rId4"/></a:rPr><a:t> enlace</a:t></a:r></a:p>"#),
            forma(r#"<p:ph type="title"/>"#, 0, 500000, "<a:p><a:r><a:t>Izaje de cargas</a:t></a:r></a:p>"),
            forma(
                r#"<p:ph idx="1"/>"#,
                0,
                1500000,
                r#"<a:p><a:r><a:t>Antes de izar</a:t></a:r></a:p><a:p><a:pPr lvl="1"/><a:r><a:t>Revisar eslingas</a:t></a:r></a:p><a:p><a:pPr><a:buNone/></a:pPr><a:r><a:t>Sin vineta</a:t></a:r></a:p>"#,
            ),
            forma(
                "",
                0,
                2500000,
                r#"<a:p><a:pPr><a:buAutoNum type="alphaLcParenR"/></a:pPr><a:r><a:t>Primero</a:t></a:r></a:p><a:p><a:pPr><a:buAutoNum type="alphaLcParenR"/></a:pPr><a:r><a:t>Segundo</a:t></a:r></a:p><a:p><a:pPr><a:buFont typeface="Wingdings"/><a:buChar char="&#xF0FC;"/></a:pPr><a:r><a:t>Listo</a:t></a:r></a:p>"#,
            ),
            tabla.to_string(),
            grupo,
            imagen.to_string(),
            forma(r#"<p:ph type="sldNum" idx="12"/>"#, 0, 6000000, "<a:p><a:r><a:t>1</a:t></a:r></a:p>"),
        ]
        .concat(),
        false,
    );
    let s2 = diapositiva(&forma(r#"<p:ph type="ctrTitle"/>"#, 0, 0, "<a:p><a:r><a:t>Portada</a:t></a:r></a:p>"), false);
    let s3 = diapositiva(&forma(r#"<p:ph type="title"/>"#, 0, 0, "<a:p><a:r><a:t>Respaldo oculto</a:t></a:r></a:p>"), true);
    let notas = format!(
        r#"<?xml version="1.0"?><p:notes {}><p:cSld><p:spTree>{}{}</p:spTree></p:cSld></p:notes>"#,
        NS,
        forma(r#"<p:ph type="sldImg"/>"#, 0, 0, ""),
        forma(r#"<p:ph type="body" idx="1"/>"#, 0, 1000, "<a:p><a:r><a:t>Recordar la charla de 5 minutos.</a:t></a:r></a:p>"),
    );
    let presentacion = format!(
        r#"<?xml version="1.0"?><p:presentation {}><p:sldIdLst><p:sldId id="256" r:id="rId3"/><p:sldId id="257" r:id="rId1"/><p:sldId id="258" r:id="rId2"/></p:sldIdLst></p:presentation>"#,
        NS
    );
    zip(&[
        ("_rels/.rels", &rels(&[("rId1", "officeDocument", "ppt/presentation.xml"), ("rId2", "metadata/core-properties", "docProps/core.xml")])),
        ("docProps/core.xml", r#"<?xml version="1.0"?><cp:coreProperties xmlns:cp="x" xmlns:dc="y"><dc:title>Charla de izaje</dc:title></cp:coreProperties>"#),
        ("ppt/presentation.xml", &presentacion),
        ("ppt/_rels/presentation.xml.rels", &rels(&[("rId1", "slide", "slides/slide1.xml"), ("rId2", "slide", "slides/slide3.xml"), ("rId3", "slide", "slides/slide2.xml")])),
        ("ppt/slides/slide1.xml", &s1),
        ("ppt/slides/_rels/slide1.xml.rels", &rels(&[("rId2", "image", "../media/image1.png"), ("rId3", "notesSlide", "../notesSlides/notesSlide1.xml"), ("rId4", "hyperlink", "https://example.com/izaje")])),
        ("ppt/slides/slide2.xml", &s2),
        ("ppt/slides/slide3.xml", &s3),
        ("ppt/notesSlides/notesSlide1.xml", &notas),
    ])
}

fn textos(d: &Document) -> Vec<String> {
    d.content.iter().map(|b| b.text.clone()).collect()
}

fn bloque<'a>(d: &'a Document, texto: &str) -> &'a Block {
    d.content.iter().find(|b| b.text == texto).unwrap_or_else(|| panic!("falta el bloque «{}»", texto))
}

#[test]
fn powerpoint_orden_y_tipos() {
    let bytes = presentacion();
    assert_eq!(detect(&bytes).unwrap(), Format::Pptx);
    let d = read(&bytes).unwrap();
    assert_eq!(d.kind, "pptx");
    assert_eq!(d.page_count, Some(3));
    let t = textos(&d);
    // Orden de la presentacion (no el de los archivos): la portada es la primera.
    assert_eq!(t[0], "Portada");
    assert_eq!(bloque(&d, "Portada").page, Some(1));
    // Dentro de la diapositiva: titulo primero, luego de arriba hacia abajo.
    assert_eq!(&t[1..6], &["Izaje de cargas", "Nota previa enlace", "Antes de izar", "Revisar eslingas", "Sin vineta"]);
    let titulo = bloque(&d, "Izaje de cargas");
    assert_eq!((titulo.kind, titulo.level, titulo.page), (Kind::Heading, 1, Some(2)));
    let nota = bloque(&d, "Nota previa enlace");
    assert_eq!(nota.kind, Kind::Paragraph);
    assert!(nota.runs[0].bold);
    assert_eq!(nota.runs[1].link.as_deref(), Some("https://example.com/izaje"));
    let item = bloque(&d, "Antes de izar");
    assert_eq!((item.kind, item.level, item.marker.as_deref()), (Kind::ListItem, 0, Some("\u{2022}")));
    assert_eq!(bloque(&d, "Revisar eslingas").level, 1);
    assert_eq!(bloque(&d, "Sin vineta").kind, Kind::Paragraph);
    // Cuadro de texto con numeracion y con vineta propia (Wingdings 0xFC = visto bueno).
    assert_eq!(bloque(&d, "Primero").marker.as_deref(), Some("a)"));
    assert_eq!(bloque(&d, "Segundo").marker.as_deref(), Some("b)"));
    assert!(bloque(&d, "Segundo").ordered);
    assert_eq!(bloque(&d, "Listo").marker.as_deref(), Some("\u{2713}"));
    // Numero de diapositiva y diapositiva oculta: fuera.
    assert!(!t.iter().any(|s| s == "1" || s == "Respaldo oculto"));
    assert!(d.warnings.iter().any(|w| w.contains("1 diapositivas ocultas")));
}

#[test]
fn powerpoint_tablas_grupos_imagenes_y_notas() {
    let d = read(&presentacion()).unwrap();
    let tabla = d.content.iter().find(|b| b.kind == Kind::Table).expect("tabla");
    let filas: Vec<Vec<(String, u32, u32, bool)>> = tabla.rows.iter().map(|r| r.iter().map(|c| (c.text.clone(), c.colspan, c.rowspan, c.merged)).collect()).collect();
    assert_eq!(
        filas,
        vec![
            vec![("Equipo".into(), 2, 1, false), ("Riesgo".into(), 1, 2, false)],
            vec![("Grua".into(), 1, 1, false), ("50 t".into(), 1, 1, false), (String::new(), 1, 1, true)],
        ]
    );
    let t = textos(&d);
    let i = t.iter().position(|s| s == "Grupo izquierda").unwrap();
    assert_eq!(t[i + 1], "Grupo derecha");
    let img = d.content.iter().find(|b| b.kind == Kind::Image).expect("imagen");
    assert_eq!((img.name.as_deref(), img.text.as_str(), img.page), (Some("image1.png"), "Diagrama del circuito", Some(2)));
    // El orden completo: tabla (y=3M) antes del grupo (y=4M) antes de la imagen (y=5M).
    let pos = |k: Kind| d.content.iter().position(|b| b.kind == k).unwrap();
    assert!(pos(Kind::Table) < i && i < pos(Kind::Image));
    let notas = &d.extras.iter().find(|(k, _)| *k == "notes").expect("notas").1;
    assert_eq!(notas.len(), 1);
    assert_eq!((notas[0].text.as_str(), notas[0].label.as_deref()), ("Recordar la charla de 5 minutos.", Some("2")));
    assert!(d.meta.iter().any(|(k, _)| *k == "title"));
}

#[test]
fn powerpoint_en_markdown_con_diapositivas_y_notas() {
    let md = doc2json::markdown::to_markdown(&read(&presentacion()).unwrap());
    let d1 = md.find("<!-- diapositiva 1 -->").expect("marca de la diapositiva 1");
    let d2 = md.find("<!-- diapositiva 2 -->").expect("marca de la diapositiva 2");
    let nota = md.find("> **Notas del orador:** Recordar la charla de 5 minutos.").expect("notas");
    // Las notas de la diapositiva 2 van despues de su marca (y de su contenido).
    assert!(d1 < d2 && d2 < nota, "{md}");
}

fn planilla(fecha1904: bool) -> Vec<u8> {
    let libro = format!(
        r#"<?xml version="1.0"?><workbook xmlns:r="{}"><workbookPr{}/><sheets><sheet name="Datos" sheetId="1" r:id="rId1"/><sheet name="Oculta" sheetId="2" state="hidden" r:id="rId2"/><sheet name="Vacia" sheetId="3" r:id="rId3"/></sheets></workbook>"#,
        REL,
        if fecha1904 { r#" date1904="1""# } else { "" }
    );
    let compartidos = r#"<?xml version="1.0"?><sst><si><t>Nombre</t></si><si><t>Fecha</t></si><si><t>Avance</t></si><si><r><rPr><b/></rPr><t>Juan </t></r><r><t>Perez</t></r><rPh><t>fonetica</t></rPh></si><si><t>Combinada</t></si><si><t>Nota</t></si></sst>"#;
    let estilos = r#"<?xml version="1.0"?><styleSheet><numFmts count="1"><numFmt numFmtId="164" formatCode="dd/mm/yyyy;@"/></numFmts><cellStyleXfs><xf numFmtId="10"/></cellStyleXfs><cellXfs><xf numFmtId="0"/><xf numFmtId="164" applyNumberFormat="1"/><xf numFmtId="10"/><xf numFmtId="22"/></cellXfs></styleSheet>"#;
    let hoja = r#"<?xml version="1.0"?><worksheet><sheetData>
        <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1" t="s"><v>2</v></c><c r="E1" t="s"><v>5</v></c></row>
        <row r="2"><c r="A2" t="s"><v>3</v></c><c r="B2" s="1"><v>45000</v></c><c r="C2" s="2"><v>0.25</v></c><c r="D2" s="1"/><c r="E2" t="inlineStr"><is><t>en linea</t></is></c></row>
        <row r="3"><c r="A3"><v></v></c></row>
        <row r="4"><c r="A4"><v>0.30000000000000004</v></c><c r="B4" t="b"><v>1</v></c><c r="C4" t="str"><f>A1&amp;"x"</f><v>Nombrex</v></c><c r="E4" t="e"><v>#N/A</v></c></row>
        <row r="5"><c r="A5" t="s"><v>4</v></c><c r="B5"/><c r="C5"/></row>
        <row r="6"><c r="E6" s="3"><v>45000.75</v></c></row>
        </sheetData><mergeCells count="1"><mergeCell ref="A5:C6"/></mergeCells></worksheet>"#;
    zip(&[
        ("_rels/.rels", &rels(&[("rId1", "officeDocument", "xl/workbook.xml")])),
        ("xl/workbook.xml", &libro),
        ("xl/_rels/workbook.xml.rels", &rels(&[("rId1", "worksheet", "worksheets/sheet1.xml"), ("rId2", "worksheet", "worksheets/sheet2.xml"), ("rId3", "worksheet", "worksheets/sheet3.xml"), ("rId4", "sharedStrings", "sharedStrings.xml"), ("rId5", "styles", "styles.xml")])),
        ("xl/sharedStrings.xml", compartidos),
        ("xl/styles.xml", estilos),
        ("xl/worksheets/sheet1.xml", hoja),
        ("xl/worksheets/sheet2.xml", r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>secreto</t></is></c></row></sheetData></worksheet>"#),
        ("xl/worksheets/sheet3.xml", r#"<worksheet><sheetData/></worksheet>"#),
    ])
}

#[test]
fn excel_valores_como_se_muestran() {
    let bytes = planilla(false);
    assert_eq!(detect(&bytes).unwrap(), Format::Xlsx);
    let d = read(&bytes).unwrap();
    assert_eq!(d.kind, "xlsx");
    let encabezado = &d.content[0];
    assert_eq!((encabezado.kind, encabezado.text.as_str(), encabezado.page), (Kind::Heading, "Datos", Some(1)));
    let tabla = &d.content[1];
    assert_eq!(tabla.kind, Kind::Table);
    let celdas: Vec<Vec<(String, u32, u32, bool)>> = tabla.rows.iter().map(|r| r.iter().map(|c| (c.text.clone(), c.colspan, c.rowspan, c.merged)).collect()).collect();
    let c = |t: &str| (t.to_string(), 1, 1, false);
    // Sin la columna D ni la fila 3 (vacias) ni la celda vacia al final de la fila 5. La
    // combinacion A5:C6 conserva su alto y su ancho.
    assert_eq!(
        celdas,
        vec![
            vec![c("Nombre"), c("Fecha"), c("Avance"), c("Nota")],
            vec![c("Juan Perez"), c("2023-03-15"), c("25%"), c("en linea")],
            vec![c("0.3"), c("TRUE"), c("Nombrex"), c("#N/A")],
            vec![("Combinada".into(), 3, 2, false)],
            vec![(String::new(), 3, 1, true), c("2023-03-15T18:00:00")],
        ]
    );
    // Hoja oculta: no se lee. Hoja vacia: no deja bloques.
    assert!(!d.content.iter().any(|b| b.text.contains("secreto") || b.text == "Vacia"));
    assert!(d.warnings.iter().any(|w| w.contains("1 hojas ocultas")));
    assert_eq!(d.page_count, Some(2));
}

#[test]
fn excel_sistema_de_fechas_1904() {
    let d = read(&planilla(true)).unwrap();
    assert_eq!(d.content[1].rows[1][1].text, "2027-03-16");
}

#[test]
fn excel_con_prefijo_de_espacio_de_nombres() {
    // Asi escriben algunos generadores (por ejemplo, el Open XML SDK de .NET).
    let libro = format!(r#"<x:workbook xmlns:x="m" xmlns:r="{}"><x:sheets><x:sheet name="Hoja1" sheetId="1" r:id="rId1"/></x:sheets></x:workbook>"#, REL);
    let hoja = r#"<x:worksheet xmlns:x="m"><x:sheetData><x:row r="1"><x:c r="A1" t="s"><x:v>0</x:v></x:c><x:c r="B1" s="1"><x:v>45000</x:v></x:c></x:row></x:sheetData><x:mergeCells><x:mergeCell ref="A1:A2"/></x:mergeCells></x:worksheet>"#;
    let bytes = zip(&[
        ("_rels/.rels", &rels(&[("rId1", "officeDocument", "xl/workbook.xml")])),
        ("xl/workbook.xml", &libro),
        ("xl/_rels/workbook.xml.rels", &rels(&[("rId1", "worksheet", "worksheets/sheet1.xml"), ("rId2", "sharedStrings", "sharedStrings.xml"), ("rId3", "styles", "styles.xml")])),
        ("xl/sharedStrings.xml", r#"<x:sst xmlns:x="m"><x:si><x:r><x:t>Turno</x:t></x:r><x:r><x:t xml:space="preserve"> A</x:t></x:r></x:si></x:sst>"#),
        ("xl/styles.xml", r#"<x:styleSheet xmlns:x="m"><x:cellXfs><x:xf numFmtId="0"/><x:xf numFmtId="14"/></x:cellXfs></x:styleSheet>"#),
        ("xl/worksheets/sheet1.xml", hoja),
    ]);
    let d = read(&bytes).unwrap();
    assert_eq!(d.content[0].text, "Hoja1");
    let fila: Vec<&str> = d.content[1].rows[0].iter().map(|c| c.text.as_str()).collect();
    assert_eq!(fila, ["Turno A", "2023-03-15"]);
}

#[test]
fn zip_que_no_es_office_sigue_siendo_error_de_word() {
    let bytes = zip(&[("hola.txt", "hola")]);
    assert_eq!(detect(&bytes).unwrap(), Format::Docx);
    assert!(read(&bytes).is_err());
}
