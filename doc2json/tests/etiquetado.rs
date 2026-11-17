//! PDF etiquetado de punta a punta: archivos armados en el codigo que pasan por el lector del
//! arbol de estructura y por el contenido marcado de las paginas, incluidos arboles danados o
//! malintencionados.

use doc2json::doc::{Block, Kind};

/// Arma un PDF con los objetos dados (el primero es el 1) y su tabla xref.
fn pdf(objs: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::with_capacity(objs.len());
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
    }
    let x = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend_from_slice(format!("{:010} 00000 n \n", o).as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objs.len() + 1, x).as_bytes());
    out
}

fn stream(dict: &str, data: &str) -> String {
    format!("<< {} /Length {} >>\nstream\n{}\nendstream", dict, data.len(), data)
}

/// Texto en (x, y) dentro de contenido marcado con el MCID dado.
fn marcado(mcid: i64, x: i32, y: i32, texto: &str) -> String {
    format!("/P << /MCID {} >> BDC BT /F1 10 Tf {} {} Td ({}) Tj ET EMC\n", mcid, x, y, texto)
}

/// PDF de una pagina etiquetado: 1 catalogo, 2 paginas, 3 pagina, 4 contenido, 5 raiz de la
/// estructura (con `raiz` como /K), 6 fuente y los objetos `mas` desde el 7.
fn etiquetado(contenido: &str, raiz: &str, mas: &[String]) -> Vec<u8> {
    etiquetado_con(contenido, &format!("<< /Type /StructTreeRoot /K {} >>", raiz), "", mas)
}

fn etiquetado_con(contenido: &str, raiz: &str, recursos: &str, mas: &[String]) -> Vec<u8> {
    let mut objs = vec![
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 5 0 R /MarkInfo << /Marked true >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 6 0 R >> {} >> >>", recursos),
        stream("", contenido),
        raiz.to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
    ];
    objs.extend(mas.iter().cloned());
    pdf(&objs)
}

fn etiquetado_si(d: &doc2json::Document) -> bool {
    d.meta.iter().any(|(k, v)| *k == "tagged" && matches!(v, doc2json::doc::Meta::Bool(true)))
}

fn textos(d: &doc2json::Document) -> Vec<&str> {
    d.content.iter().map(|b| b.text.as_str()).collect()
}

fn celdas(bloques: &[Block]) -> usize {
    bloques.iter().filter(|b| b.kind == Kind::Table).map(|b| b.rows.iter().map(|r| r.len()).sum::<usize>()).sum()
}

#[test]
fn parrafo_que_cruza_de_columna_sigue_el_orden_del_arbol() {
    // El parrafo empieza abajo en la columna izquierda y sigue arriba en la derecha: el orden
    // de lectura es el de /K, no el de la posicion en la pagina.
    let c = marcado(0, 72, 100, "Inicio del parrafo en la columna izquierda") + &marcado(1, 330, 700, "y su final arriba en la derecha.") + &marcado(2, 72, 60, "Otro parrafo.");
    let d = doc2json::read(&etiquetado(&c, "7 0 R", &["<< /S /Document /K [8 0 R 9 0 R] >>".into(), "<< /S /P /Pg 3 0 R /K [0 1] >>".into(), "<< /S /P /Pg 3 0 R /K 2 >>".into()])).unwrap();
    assert!(etiquetado_si(&d));
    assert_eq!(textos(&d), ["Inicio del parrafo en la columna izquierda y su final arriba en la derecha.", "Otro parrafo."]);
}

#[test]
fn tramos_de_una_misma_linea_se_unen_sin_espacio_de_mas() {
    // Una palabra partida en dos MCID (negrita en medio de la palabra) sigue siendo una palabra.
    let c = "/P << /MCID 0 >> BDC BT /F1 10 Tf 72 700 Td (Aislar las fuen) Tj ET EMC\n/Span << /MCID 1 >> BDC BT /F1 10 Tf 134.8 700 Td (tes) Tj ET EMC\n/P << /MCID 2 >> BDC BT /F1 10 Tf 148.14 700 Td ( de energia.) Tj ET EMC\n";
    let d = doc2json::read(&etiquetado(c, "7 0 R", &["<< /S /P /Pg 3 0 R /K [0 << /Type /MCR /MCID 1 >> 2] >>".into()])).unwrap();
    assert_eq!(textos(&d), ["Aislar las fuentes de energia."]);
}

#[test]
fn espacio_al_final_de_un_tramo_separa_del_siguiente() {
    // El primer MCID termina en espacio ("Hola "): su fin incluye el avance de ese espacio, asi
    // que el hueco hasta "mundo" es cero, pero son dos palabras.
    let c = "/P << /MCID 0 >> BDC BT /F1 10 Tf 72 700 Td (Hola ) Tj ET EMC\n/Span << /MCID 1 >> BDC BT /F1 10 Tf 95.34 700 Td (mundo) Tj ET EMC\n";
    let d = doc2json::read(&etiquetado(c, "7 0 R", &["<< /S /P /Pg 3 0 R /K [0 << /Type /MCR /MCID 1 >>] >>".into()])).unwrap();
    assert_eq!(textos(&d), ["Hola mundo"]);
}

#[test]
fn subindice_en_su_propio_contenido_queda_pegado() {
    // "p0-calculadora" con el 0 como subindice (mas chico y mas abajo) en otro MCID: sigue siendo
    // una palabra, como cuando la pagina se arma por posicion.
    let c = "/P << /MCID 0 >> BDC BT /F1 10 Tf 72 700 Td (Use la p) Tj ET EMC\n/Span << /MCID 1 >> BDC BT /F1 6.5 Tf 108.68 696.5 Td (0) Tj ET EMC\n/P << /MCID 2 >> BDC BT /F1 10 Tf 112.29 700 Td (-calculadora.) Tj ET EMC\n";
    let d = doc2json::read(&etiquetado(c, "7 0 R", &["<< /S /P /Pg 3 0 R /K [0 << /Type /MCR /MCID 1 >> 2] >>".into()])).unwrap();
    assert_eq!(textos(&d), ["Use la p0-calculadora."]);
}

#[test]
fn dos_columnas_a_la_misma_altura_no_se_mezclan() {
    // El parrafo termina abajo a la izquierda y sigue arriba a la derecha, con otra linea suya a la
    // misma altura que el final: no se arma como una sola linea con las dos columnas.
    let c = marcado(0, 72, 700, "Primera linea izquierda") + &marcado(0, 72, 686, "segunda linea izquierda") + &marcado(1, 330, 686, "y sigue a la derecha.");
    let d = doc2json::read(&etiquetado(&c, "7 0 R", &["<< /S /P /Pg 3 0 R /K [0 1] >>".into()])).unwrap();
    assert_eq!(textos(&d), ["Primera linea izquierda segunda linea izquierda y sigue a la derecha."]);
}

#[test]
fn artefacto_sin_emc_no_se_traga_el_resto_de_la_pagina() {
    // Falta el EMC del encabezado: sin la correccion, todo lo que sigue quedaba como artefacto.
    let c = marcado(0, 72, 700, "Primer parrafo del procedimiento.") + "/Artifact BMC BT /F1 10 Tf 72 760 Td (Encabezado) Tj ET\n" + &marcado(1, 72, 650, "Segundo parrafo, despues del artefacto abierto.");
    let d = doc2json::read(&etiquetado(&c, "[7 0 R 8 0 R]", &["<< /S /P /Pg 3 0 R /K 0 >>".into(), "<< /S /P /Pg 3 0 R /K 1 >>".into()])).unwrap();
    assert!(d.content.iter().any(|b| b.text == "Segundo parrafo, despues del artefacto abierto."), "{:?}", textos(&d));
    assert!(d.content.iter().any(|b| b.text.contains("Encabezado")), "{:?}", textos(&d));
}

#[test]
fn artefacto_sin_emc_dentro_de_un_formulario_no_sale_de_el() {
    // Un formulario (XObject) que abre un artefacto y no lo cierra no afecta a la pagina.
    let form = stream("/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> >>", "/Artifact BMC BT /F1 10 Tf 72 760 Td (Logo) Tj ET");
    let c = marcado(0, 72, 700, "Antes del formulario.") + "/Fm1 Do\n" + &marcado(1, 72, 650, "Despues del formulario.");
    let raiz = "<< /Type /StructTreeRoot /K [7 0 R 8 0 R] >>";
    let d = doc2json::read(&etiquetado_con(&c, raiz, "/XObject << /Fm1 9 0 R >>", &["<< /S /P /Pg 3 0 R /K 0 >>".into(), "<< /S /P /Pg 3 0 R /K 1 >>".into(), form])).unwrap();
    assert!(d.content.iter().any(|b| b.text == "Despues del formulario."), "{:?}", textos(&d));
}

#[test]
fn tabla_con_relleno_enorme_no_agota_la_memoria() {
    // Una celda de 999 columnas y otra combinada hacia abajo 1000 filas, seguidas de 1000 filas
    // vacias: cada fila se rellenaba con 1000 celdas (un millon en total, con unos pocos KB).
    let filas = "<< /S /TR >> ".repeat(1000);
    let tabla = format!("<< /S /Table /Pg 3 0 R /K [<< /S /TR /K [<< /S /TD /A << /ColSpan 999 >> /K 0 >> << /S /TD /A << /RowSpan 1000 >> /K 1 >>] >> {}] >>", filas);
    let c = marcado(0, 72, 700, "Riesgo") + &marcado(1, 300, 700, "Control");
    let d = doc2json::read(&etiquetado(&c, "7 0 R", &[tabla])).unwrap();
    assert!(celdas(&d.content) <= 200_000, "{} celdas", celdas(&d.content));
    // El texto no se pierde: va como parrafos, con un aviso.
    assert!(d.content.iter().any(|b| b.text.contains("Riesgo")) && d.content.iter().any(|b| b.text.contains("Control")), "{:?}", textos(&d));
    assert!(d.warnings.iter().any(|w| w.contains("tabla")), "{:?}", d.warnings);
}

#[test]
fn tabla_mas_ancha_que_el_limite_va_como_parrafos() {
    // 50 celdas de 1000 columnas: 50.000 columnas, que se rellenaban en cada fila.
    let celdas_ = "<< /S /TD /A << /ColSpan 1000 >> >> ".repeat(49);
    let tabla = format!("<< /S /Table /Pg 3 0 R /K [<< /S /TR /K [{} << /S /TD /A << /RowSpan 3 >> /K 0 >>] >> << /S /TR >> << /S /TR >>] >>", celdas_);
    let d = doc2json::read(&etiquetado(&marcado(0, 72, 700, "Celda final"), "7 0 R", &[tabla])).unwrap();
    assert_eq!(celdas(&d.content), 0);
    assert!(d.content.iter().any(|b| b.text == "Celda final"), "{:?}", textos(&d));
}

#[test]
fn muchas_tablas_grandes_tienen_un_tope_por_documento() {
    // Seis tablas de casi 200.000 celdas cada una: cada una cabe, todas juntas no.
    let filas = "<< /S /TR >> ".repeat(999);
    let mut mas = vec![format!("<< /S /Document /K [{}] >>", (8..14).map(|n| format!("{} 0 R", n)).collect::<Vec<_>>().join(" "))];
    let mut c = String::new();
    for i in 0..6 {
        c += &marcado(i, 72, 700 - 20 * i as i32, &format!("Tabla {}", i + 1));
        mas.push(format!("<< /S /Table /Pg 3 0 R /K [<< /S /TR /K [<< /S /TD /A << /ColSpan 198 >> /K {} >> << /S /TD /A << /RowSpan 1000 >> >>] >> {}] >>", i, filas));
    }
    let d = doc2json::read(&etiquetado(&c, "7 0 R", &mas)).unwrap();
    assert!(celdas(&d.content) <= 1_000_000, "{} celdas", celdas(&d.content));
    for i in 1..=6 {
        assert!(d.content.iter().any(|b| b.text.contains(&format!("Tabla {}", i)) || b.rows.iter().flatten().any(|c| c.text == format!("Tabla {}", i))), "falta la tabla {}", i);
    }
    assert!(d.warnings.iter().any(|w| w.contains("tabla")), "{:?}", d.warnings);
}

#[test]
fn cadena_larga_de_arreglos_indirectos_no_desborda_la_pila() {
    // /K -> [8 0 R] -> [9 0 R] -> ... : antes la profundidad solo se contaba en los elementos.
    let n = 100_000;
    let mut mas: Vec<String> = (0..n).map(|i| format!("[{} 0 R]", 8 + i)).collect();
    mas.push("<< /S /P /Pg 3 0 R /K 0 >>".into());
    let d = doc2json::read(&etiquetado(&marcado(0, 72, 700, "Texto al final de la cadena."), "7 0 R", &mas)).unwrap();
    assert!(d.pages[0].text.contains("Texto al final de la cadena."));
}

#[test]
fn arbol_con_ciclos_no_se_cuelga_ni_repite() {
    let c = marcado(0, 72, 700, "Unico parrafo.");
    let d = doc2json::read(&etiquetado(&c, "[7 0 R 7 0 R]", &["<< /S /Div /K [8 0 R 7 0 R] >>".into(), "<< /S /P /Pg 3 0 R /K [0 8 0 R 7 0 R] >>".into()])).unwrap();
    assert_eq!(textos(&d), ["Unico parrafo."]);
}

#[test]
fn raiz_de_estructura_danada_usa_la_posicion() {
    let c = marcado(0, 72, 700, "Texto de la pagina.");
    for raiz in ["99 0 R", "(texto)", "[<< /S /P /K 0 >>]", "<< /S /P /Pg 77 0 R /K 0 >>", "null"] {
        let d = doc2json::read(&etiquetado(&c, raiz, &[])).unwrap();
        assert_eq!(textos(&d), ["Texto de la pagina."], "raiz {}", raiz);
        assert!(!etiquetado_si(&d), "raiz {}", raiz);
    }
    // La raiz misma no es un diccionario.
    let mut b = etiquetado_con(&c, "42", "", &[]);
    assert_eq!(textos(&doc2json::read(&b).unwrap()), ["Texto de la pagina."]);
    // Archivo cortado a la mitad: se reconstruye el indice y no hay panic.
    b.truncate(b.len() / 2);
    let _ = doc2json::read(&b);
}

#[test]
fn figura_con_imagen_cuenta_una_sola_vez() {
    let imagen = "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 1 >>\nstream\nX\nendstream".to_string();
    let c = marcado(0, 72, 700, "Pictograma de casco.") + "/Figure << /MCID 1 >> BDC q 50 0 0 50 72 600 cm /Im1 Do Q EMC\n";
    let raiz = "<< /Type /StructTreeRoot /K [7 0 R 8 0 R] >>";
    let d = doc2json::read(&etiquetado_con(&c, raiz, "/XObject << /Im1 9 0 R >>", &["<< /S /P /Pg 3 0 R /K 0 >>".into(), "<< /S /Figure /Alt (Casco) /Pg 3 0 R /K 1 >>".into(), imagen])).unwrap();
    assert!(d.content.iter().any(|b| b.kind == Kind::Image && b.text == "Casco"));
    let json = doc2json::doc::to_json(&d, false);
    let desde = &json[json.find("\"stats\"").unwrap()..];
    let stats = &desde[..desde.find('}').unwrap() + 1];
    assert!(stats.contains("\"images\":1"), "{}", stats);
}
