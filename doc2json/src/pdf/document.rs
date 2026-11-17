//! Estructura del archivo PDF: tabla de referencias cruzadas, carga de objetos, arbol de
//! paginas, metadatos y marcadores.

use super::crypt::{Crypt, Method, Params};
use super::filters::{self, Parms};
use super::object::{is_ws, Dict, Lexer, Obj};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[derive(Clone, Copy)]
enum XRef {
    /// Desplazamiento del objeto dentro del archivo.
    InFile(usize),
    /// Objeto guardado dentro de un flujo de objetos: (numero del flujo, indice).
    InStream(u32, u32),
}

struct ObjStm {
    data: Vec<u8>,
    index: Vec<(u32, usize)>,
}

pub struct PageInfo {
    pub obj: u32,
    pub dict: Dict,
    pub resources: Dict,
    /// Caja visible de la pagina: [x0, y0, x1, y1].
    pub bbox: [f64; 4],
    pub rotate: i64,
}

pub struct Pdf<'a> {
    pub data: &'a [u8],
    xref: HashMap<u32, XRef>,
    pub trailer: Dict,
    pub version: String,
    /// La tabla de referencias estaba danada y se reconstruyo recorriendo el archivo.
    pub rebuilt: bool,
    cache: RefCell<HashMap<u32, Rc<Obj>>>,
    objstms: RefCell<HashMap<u32, Option<Rc<ObjStm>>>>,
    loading: RefCell<Vec<u32>>,
    crypt: Option<Crypt>,
    /// Objeto que contiene el diccionario de cifrado (sus cadenas no van cifradas).
    encrypt_obj: Option<u32>,
    /// Numero de objeto de cada flujo, por posicion de sus datos (la clave depende de el).
    stream_ids: RefCell<HashMap<usize, (u32, u16)>>,
}

/// Descifra todas las cadenas de un objeto.
fn decrypt_strings(o: &mut Obj, c: &Crypt, num: u32, gen: u16) {
    match o {
        Obj::Str(s) => *s = c.decrypt(c.strings, num, gen, s),
        Obj::Array(a) => a.iter_mut().for_each(|x| decrypt_strings(x, c, num, gen)),
        Obj::Dict(d) | Obj::Stream { dict: d, .. } => d.0.iter_mut().for_each(|(_, v)| decrypt_strings(v, c, num, gen)),
        _ => {}
    }
}

fn find_last(d: &[u8], pat: &[u8]) -> Option<usize> {
    if d.len() < pat.len() {
        return None;
    }
    (0..=d.len() - pat.len()).rev().find(|&i| &d[i..i + pat.len()] == pat)
}

fn find_from(d: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    if from >= d.len() || d.len() - from < pat.len() {
        return None;
    }
    d[from..].windows(pat.len()).position(|w| w == pat).map(|p| p + from)
}

impl<'a> Pdf<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Pdf<'a>, String> {
        // Hay archivos con basura antes de la cabecera: los desplazamientos cuentan desde "%PDF-".
        let head = &bytes[..bytes.len().min(2048)];
        let base = head.windows(5).position(|w| w == b"%PDF-").ok_or("El archivo no es un PDF (falta la cabecera %PDF-).")?;
        let data = &bytes[base..];
        let version: String = data[5..data.len().min(8)].iter().map(|&b| b as char).filter(|c| c.is_ascii_digit() || *c == '.').collect();
        let mut pdf = Pdf {
            data,
            xref: HashMap::new(),
            trailer: Dict::default(),
            version,
            rebuilt: false,
            cache: RefCell::new(HashMap::new()),
            objstms: RefCell::new(HashMap::new()),
            loading: RefCell::new(Vec::new()),
            crypt: None,
            encrypt_obj: None,
            stream_ids: RefCell::new(HashMap::new()),
        };

        let tail_start = data.len().saturating_sub(2048);
        let start = find_last(&data[tail_start..], b"startxref").and_then(|p| {
            let mut lx = Lexer::new(data, tail_start + p + 9);
            lx.object().and_then(|o| o.as_i64()).filter(|v| *v > 0).map(|v| v as usize)
        });
        let mut ok = false;
        if let Some(off) = start {
            let mut visited = HashSet::new();
            ok = pdf.read_xref(off, &mut visited, true);
        }
        if ok {
            pdf.setup_crypt()?;
        }
        if !ok || !pdf.root_is_valid() {
            pdf.crypt = None;
            pdf.rebuild();
            pdf.setup_crypt()?;
        }
        if !pdf.root_is_valid() {
            return Err("No se pudo leer la estructura del PDF: el archivo esta danado o incompleto.".into());
        }
        Ok(pdf)
    }

    /// Si el documento esta cifrado, prepara la clave (contrasena de usuario vacia).
    fn setup_crypt(&mut self) -> Result<(), String> {
        let entry = match self.trailer.get("Encrypt") {
            Some(e) => e.clone(),
            None => return Ok(()),
        };
        self.encrypt_obj = match &entry {
            Obj::Ref(n, _) => Some(*n),
            _ => None,
        };
        let enc = self.resolve(&entry);
        let enc = enc.as_dict().ok_or("El PDF esta cifrado pero su informacion de cifrado no se pudo leer.")?;
        let id: Vec<u8> = self.get(&self.trailer, "ID").as_array().and_then(|a| a.first()).map(|o| self.resolve(o)).and_then(|o| o.as_bytes().map(|b| b.to_vec())).unwrap_or_default();
        let int = |k: &str, def: i64| self.get(enc, k).as_i64().unwrap_or(def);
        let bytes = |k: &str| self.get(enc, k).as_bytes().map(|b| b.to_vec()).unwrap_or_default();
        // Version 4: el metodo de cada filtro de cifrado esta en /CF.
        let cf = self.dict(enc, "CF").unwrap_or_default();
        let method_of = |key: &str| -> Option<String> {
            let name = self.name(enc, key)?;
            if name == "Identity" {
                return Some("None".into());
            }
            self.dict(&cf, &name).and_then(|f| self.name(&f, "CFM"))
        };
        let (strf, stmf) = (method_of("StrF"), method_of("StmF"));
        let filter = self.name(enc, "Filter").unwrap_or_default();
        let (o, u) = (bytes("O"), bytes("U"));
        let params = Params {
            filter: &filter,
            v: int("V", 0),
            r: int("R", 2),
            length: int("Length", 40),
            o: &o,
            u: &u,
            p: int("P", 0),
            id: &id,
            encrypt_metadata: !matches!(&*self.get(enc, "EncryptMetadata"), Obj::Bool(false)),
            strings: strf.as_deref(),
            streams: stmf.as_deref(),
        };
        self.crypt = Some(Crypt::new(&params)?);
        // Lo cargado antes de tener la clave puede traer cadenas sin descifrar.
        self.cache.borrow_mut().clear();
        self.objstms.borrow_mut().clear();
        Ok(())
    }

    fn root_is_valid(&self) -> bool {
        match self.trailer.get("Root") {
            Some(r) => {
                let root = self.resolve(r);
                root.as_dict().map_or(false, |d| d.has("Pages"))
            }
            None => false,
        }
    }

    /// Lee una seccion de referencias (tabla clasica o flujo) y sigue la cadena `/Prev`.
    fn read_xref(&mut self, offset: usize, visited: &mut HashSet<usize>, newest: bool) -> bool {
        if offset >= self.data.len() || !visited.insert(offset) || visited.len() > 512 {
            return false;
        }
        let mut lx = Lexer::new(self.data, offset);
        lx.skip_ws();
        let trailer: Dict;
        if lx.starts_with(b"xref") {
            lx.pos += 4;
            loop {
                lx.skip_ws();
                if lx.starts_with(b"trailer") {
                    lx.pos += 7;
                    break;
                }
                let (first, count) = match (lx.object(), lx.object()) {
                    (Some(Obj::Int(a)), Some(Obj::Int(b))) if a >= 0 && b >= 0 => (a as u32, b as u32),
                    _ => return false,
                };
                for i in 0..count {
                    let (off, _gen, kind) = match (lx.object(), lx.object(), lx.object()) {
                        (Some(Obj::Int(o)), Some(Obj::Int(g)), Some(Obj::Op(k))) => (o, g, k),
                        _ => return false,
                    };
                    if kind == "n" && off > 0 {
                        self.xref.entry(first + i).or_insert(XRef::InFile(off as usize));
                    }
                }
            }
            trailer = match lx.object() {
                Some(Obj::Dict(d)) => d,
                _ => return false,
            };
            // PDF hibrido: parte de las referencias esta en un flujo aparte.
            if let Some(x) = trailer.get("XRefStm").and_then(|o| o.as_i64()) {
                self.read_xref(x as usize, visited, false);
            }
        } else {
            // Flujo de referencias cruzadas (PDF 1.5+).
            let (dict, data) = match self.parse_at(offset, None).map(|(o, _)| o) {
                Some(o @ Obj::Stream { .. }) => match (o.as_dict().cloned(), self.stream_data(&o)) {
                    (Some(d), Some(bytes)) => (d, bytes),
                    _ => return false,
                },
                _ => return false,
            };
            let w: Vec<usize> = dict.get("W").and_then(|o| o.as_array()).map(|a| a.iter().map(|v| v.as_i64().unwrap_or(0).clamp(0, 8) as usize).collect()).unwrap_or_default();
            if w.len() < 3 {
                return false;
            }
            let size = dict.get("Size").and_then(|o| o.as_i64()).unwrap_or(0).max(0);
            let index: Vec<i64> = match dict.get("Index").and_then(|o| o.as_array()) {
                Some(a) => a.iter().map(|v| v.as_i64().unwrap_or(0)).collect(),
                None => vec![0, size],
            };
            let entry = w[0] + w[1] + w[2];
            if entry == 0 {
                return false;
            }
            let mut pos = 0usize;
            let field = |d: &[u8], width: usize| d.iter().take(width).fold(0u64, |acc, b| acc << 8 | *b as u64);
            for pair in index.chunks(2) {
                if pair.len() < 2 {
                    break;
                }
                for i in 0..pair[1].max(0) {
                    let rec = match data.get(pos..pos + entry) {
                        Some(r) => r,
                        None => break,
                    };
                    pos += entry;
                    let kind = if w[0] == 0 { 1 } else { field(rec, w[0]) };
                    let f2 = field(&rec[w[0]..], w[1]);
                    let f3 = field(&rec[w[0] + w[1]..], w[2]);
                    let num = (pair[0] + i) as u32;
                    match kind {
                        1 => {
                            self.xref.entry(num).or_insert(XRef::InFile(f2 as usize));
                        }
                        2 => {
                            self.xref.entry(num).or_insert(XRef::InStream(f2 as u32, f3 as u32));
                        }
                        _ => {}
                    }
                }
            }
            trailer = dict;
        }
        if newest || self.trailer.0.is_empty() {
            self.trailer = trailer.clone();
        } else {
            // Las secciones antiguas solo aportan claves que falten (p. ej. /Info).
            for (k, v) in &trailer.0 {
                if !self.trailer.has(k) {
                    self.trailer.0.push((k.clone(), v.clone()));
                }
            }
        }
        if let Some(prev) = trailer.get("Prev").and_then(|o| o.as_i64()).filter(|v| *v > 0) {
            self.read_xref(prev as usize, visited, false);
        }
        true
    }

    /// Reconstruye la tabla buscando todos los "N G obj" del archivo (para PDF danados).
    fn rebuild(&mut self) {
        self.rebuilt = true;
        self.xref.clear();
        self.cache.borrow_mut().clear();
        self.objstms.borrow_mut().clear();
        let d = self.data;
        let mut trailer = Dict::default();
        let mut i = 0usize;
        while let Some(p) = find_from(d, i, b"obj") {
            i = p + 3;
            // Retrocede sobre "N G " justo antes de "obj".
            let mut j = p;
            let back_ws = |j: &mut usize| {
                let s = *j;
                while *j > 0 && is_ws(d[*j - 1]) {
                    *j -= 1;
                }
                s != *j
            };
            let back_digits = |j: &mut usize| {
                let s = *j;
                while *j > 0 && d[*j - 1].is_ascii_digit() {
                    *j -= 1;
                }
                s != *j
            };
            if !back_ws(&mut j) || !back_digits(&mut j) || !back_ws(&mut j) {
                continue;
            }
            let end_num = j;
            if !back_digits(&mut j) {
                continue;
            }
            if j > 0 && !is_ws(d[j - 1]) {
                continue;
            }
            if let Some(num) = std::str::from_utf8(&d[j..end_num]).ok().and_then(|s| s.parse::<u32>().ok()) {
                // La ultima definicion de un objeto es la vigente.
                self.xref.insert(num, XRef::InFile(j));
            }
        }
        // Objetos dentro de flujos de objetos, y el diccionario raiz.
        let nums: Vec<u32> = self.xref.keys().copied().collect();
        let mut inside: Vec<(u32, u32, u32)> = Vec::new();
        for num in nums {
            let obj = self.object(num);
            if let Some(dict) = obj.as_dict() {
                match dict.get("Type").and_then(|t| t.as_name()) {
                    Some("ObjStm") => {
                        if let Some(s) = self.objstm(num) {
                            for (k, (n, _)) in s.index.iter().enumerate() {
                                inside.push((*n, num, k as u32));
                            }
                        }
                    }
                    Some("Catalog") => {
                        if !trailer.has("Root") {
                            trailer.0.push(("Root".into(), Obj::Ref(num, 0)));
                        }
                    }
                    Some("XRef") => {
                        for key in ["Root", "Info", "Encrypt", "ID"] {
                            if let (Some(v), false) = (dict.get(key), trailer.has(key)) {
                                trailer.0.push((key.into(), v.clone()));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        for (n, stm, k) in inside {
            self.xref.entry(n).or_insert(XRef::InStream(stm, k));
        }
        // Un "trailer" clasico, si existe, manda.
        if let Some(p) = find_last(d, b"trailer") {
            if let Some(Obj::Dict(t)) = Lexer::new(d, p + 7).object() {
                for (k, v) in t.0 {
                    if !trailer.has(&k) {
                        trailer.0.push((k, v));
                    }
                }
            }
        }
        if !trailer.has("Root") {
            // Ultimo recurso: cualquier objeto que parezca el catalogo.
            let nums: Vec<u32> = self.xref.keys().copied().collect();
            for num in nums {
                if self.object(num).as_dict().map_or(false, |d| d.has("Pages") && !d.has("Parent")) {
                    trailer.0.push(("Root".into(), Obj::Ref(num, 0)));
                    break;
                }
            }
        }
        self.trailer = trailer;
        self.cache.borrow_mut().clear();
    }

    /// Analiza el objeto indirecto que comienza en `off`.
    fn parse_at(&self, off: usize, expect: Option<u32>) -> Option<(Obj, u16)> {
        let mut lx = Lexer::new(self.data, off);
        let num = lx.object()?.as_i64()?;
        let gen = lx.object()?.as_i64()?.clamp(0, 65535) as u16;
        if lx.object()? != Obj::Op("obj".into()) {
            return None;
        }
        if expect.map_or(false, |e| e as i64 != num) {
            return None;
        }
        let obj = lx.object()?;
        if let Obj::Dict(dict) = &obj {
            lx.skip_ws();
            if lx.starts_with(b"stream") {
                let mut start = lx.pos + 6;
                match self.data.get(start) {
                    Some(b'\r') => start += if self.data.get(start + 1) == Some(&b'\n') { 2 } else { 1 },
                    Some(b'\n') => start += 1,
                    _ => {}
                }
                let declared = dict.get("Length").map(|l| self.resolve(l)).and_then(|l| l.as_i64()).filter(|l| *l >= 0).map(|l| l as usize);
                // La longitud declarada vale solo si justo despues viene "endstream".
                let valid = declared.filter(|len| {
                    let mut e = start.saturating_add(*len);
                    while e < self.data.len() && is_ws(self.data[e]) {
                        e += 1;
                    }
                    self.data.get(e..e + 9) == Some(b"endstream")
                });
                let len = match valid {
                    Some(l) => l,
                    None => {
                        let mut end = find_from(self.data, start, b"endstream").unwrap_or(self.data.len());
                        if end > start && self.data[end - 1] == b'\n' {
                            end -= 1;
                        }
                        if end > start && self.data[end - 1] == b'\r' {
                            end -= 1;
                        }
                        end - start
                    }
                };
                self.stream_ids.borrow_mut().insert(start, (num.max(0) as u32, gen));
                return Some((Obj::Stream { dict: dict.clone(), start, len }, gen));
            }
        }
        Some((obj, gen))
    }

    fn objstm(&self, num: u32) -> Option<Rc<ObjStm>> {
        if let Some(cached) = self.objstms.borrow().get(&num) {
            return cached.clone();
        }
        let built = (|| {
            let obj = self.object(num);
            let dict = obj.as_dict()?;
            let n = dict.get("N").map(|v| self.resolve(v)).and_then(|v| v.as_i64())?.clamp(0, 1 << 20) as usize;
            let first = dict.get("First").map(|v| self.resolve(v)).and_then(|v| v.as_i64())?.max(0) as usize;
            let data = self.stream_data(&obj)?;
            let mut lx = Lexer::new(&data, 0);
            let mut index = Vec::with_capacity(n);
            for _ in 0..n {
                match (lx.object(), lx.object()) {
                    (Some(Obj::Int(a)), Some(Obj::Int(b))) if a >= 0 && b >= 0 => index.push((a as u32, first + b as usize)),
                    _ => break,
                }
            }
            Some(Rc::new(ObjStm { data, index }))
        })();
        self.objstms.borrow_mut().insert(num, built.clone());
        built
    }

    /// Devuelve el objeto indirecto `num` (o `Null` si no existe).
    pub fn object(&self, num: u32) -> Rc<Obj> {
        if let Some(o) = self.cache.borrow().get(&num) {
            return o.clone();
        }
        // Protege contra referencias circulares (p. ej. /Length que apunta a si mismo).
        if self.loading.borrow().contains(&num) || self.loading.borrow().len() > 64 {
            return Rc::new(Obj::Null);
        }
        self.loading.borrow_mut().push(num);
        let obj = match self.xref.get(&num).copied() {
            Some(XRef::InFile(off)) => self.parse_at(off, Some(num)).map(|(mut o, gen)| {
                // Las cadenas de los objetos sueltos van cifradas una a una.
                if let Some(c) = self.crypt.as_ref().filter(|_| self.encrypt_obj != Some(num)) {
                    decrypt_strings(&mut o, c, num, gen);
                }
                o
            }),
            Some(XRef::InStream(stm, idx)) => self.objstm(stm).and_then(|s| {
                let pos = s.index.get(idx as usize).filter(|(n, _)| *n == num).or_else(|| s.index.iter().find(|(n, _)| *n == num)).map(|(_, p)| *p)?;
                Lexer::new(&s.data, pos).object()
            }),
            None => None,
        }
        .unwrap_or(Obj::Null);
        self.loading.borrow_mut().pop();
        let rc = Rc::new(obj);
        self.cache.borrow_mut().insert(num, rc.clone());
        rc
    }

    /// Sigue una referencia indirecta hasta el objeto final.
    pub fn resolve(&self, o: &Obj) -> Rc<Obj> {
        let mut cur = match o {
            Obj::Ref(n, _) => self.object(*n),
            other => return Rc::new(other.clone()),
        };
        for _ in 0..16 {
            match &*cur {
                Obj::Ref(n, _) => cur = self.object(*n),
                _ => break,
            }
        }
        cur
    }

    pub fn get(&self, d: &Dict, key: &str) -> Rc<Obj> {
        match d.get(key) {
            Some(o) => self.resolve(o),
            None => Rc::new(Obj::Null),
        }
    }

    pub fn dict(&self, d: &Dict, key: &str) -> Option<Dict> {
        self.get(d, key).as_dict().cloned()
    }

    pub fn name(&self, d: &Dict, key: &str) -> Option<String> {
        self.get(d, key).as_name().map(String::from)
    }

    pub fn num(&self, d: &Dict, key: &str) -> Option<f64> {
        self.get(d, key).as_f64()
    }

    /// Arreglo con cada elemento ya resuelto.
    pub fn array(&self, d: &Dict, key: &str) -> Option<Vec<Rc<Obj>>> {
        self.get(d, key).as_array().map(|a| a.iter().map(|o| self.resolve(o)).collect())
    }

    /// Datos de un flujo con sus filtros aplicados. `None` si usa un filtro no soportado.
    pub fn stream_data(&self, obj: &Obj) -> Option<Vec<u8>> {
        let (mut data, chain) = self.stream_parts(obj)?;
        for (name, parms, _) in &chain {
            data = filters::apply(name, &data, parms)?;
        }
        Some(data)
    }

    /// Datos de un flujo ya descifrados pero sin filtrar, y sus filtros en orden de aplicacion con
    /// sus parametros (tambien el diccionario original, que piden filtros de imagen como CCITT).
    pub fn stream_parts(&self, obj: &Obj) -> Option<(Vec<u8>, Vec<(String, Parms, Option<Dict>)>)> {
        let (dict, start, len) = match obj {
            Obj::Stream { dict, start, len } => (dict, *start, *len),
            _ => return None,
        };
        let raw = self.data.get(start..start.checked_add(len)?.min(self.data.len()))?;
        // Descifrado previo a los filtros. Los flujos de referencias cruzadas nunca van cifrados.
        let decrypted: Option<Vec<u8>> = match &self.crypt {
            Some(c) if c.streams != Method::None && dict.get("Type").and_then(|t| t.as_name()) != Some("XRef") => {
                self.stream_ids.borrow().get(&start).map(|(num, gen)| c.decrypt(c.streams, *num, *gen, raw))
            }
            _ => None,
        };
        let raw: &[u8] = decrypted.as_deref().unwrap_or(raw);
        let filter = self.get(dict, "Filter");
        let names: Vec<String> = match &*filter {
            Obj::Name(n) => vec![n.clone()],
            Obj::Array(a) => a.iter().filter_map(|o| self.resolve(o).as_name().map(String::from)).collect(),
            _ => Vec::new(),
        };
        if names.is_empty() {
            return Some((raw.to_vec(), Vec::new()));
        }
        let parms_obj = match dict.get("DecodeParms").or_else(|| dict.get("DP")) {
            Some(o) => self.resolve(o),
            None => Rc::new(Obj::Null),
        };
        let single = names.len() == 1;
        let parms_at = |i: usize| -> (Parms, Option<Dict>) {
            let d = match &*parms_obj {
                Obj::Dict(d) if i == 0 || single => Some(d.clone()),
                Obj::Array(a) => a.get(i).map(|o| self.resolve(o)).and_then(|o| o.as_dict().cloned()),
                _ => None,
            };
            let mut p = Parms::default();
            if let Some(d) = &d {
                let int = |k: &str, def: i64| self.get(d, k).as_i64().unwrap_or(def);
                p = Parms { predictor: int("Predictor", 1), colors: int("Colors", 1), bits: int("BitsPerComponent", 8), columns: int("Columns", 1), early_change: int("EarlyChange", 1) };
            }
            (p, d)
        };
        let chain = names
            .into_iter()
            .enumerate()
            .map(|(i, name)| {
                let (p, d) = parms_at(i);
                (name, p, d)
            })
            .collect();
        Some((raw.to_vec(), chain))
    }

    pub fn is_encrypted(&self) -> bool {
        self.crypt.is_some()
    }

    pub fn catalog(&self) -> Dict {
        self.get(&self.trailer, "Root").as_dict().cloned().unwrap_or_default()
    }

    /// Rectangulo normalizado [x0, y0, x1, y1]; `None` si no es valido o es degenerado.
    pub fn rect(&self, o: &Obj) -> Option<[f64; 4]> {
        let a = self.resolve(o);
        let a = a.as_array()?;
        if a.len() < 4 {
            return None;
        }
        let v: Vec<f64> = a.iter().take(4).map(|x| self.resolve(x).as_f64().unwrap_or(0.0)).collect();
        let r = [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])];
        if r[2] - r[0] > 1.0 && r[3] - r[1] > 1.0 {
            Some(r)
        } else {
            None
        }
    }

    /// Paginas del documento en orden, con sus atributos heredados ya resueltos.
    pub fn pages(&self) -> Vec<PageInfo> {
        struct Inherited {
            resources: Dict,
            media: Option<[f64; 4]>,
            crop: Option<[f64; 4]>,
            rotate: i64,
        }
        let mut out = Vec::new();
        let mut visited: HashSet<u32> = HashSet::new();
        // Recorrido en profundidad con pila explicita: (nodo, atributos heredados).
        let root = match self.catalog().get("Pages") {
            Some(r) => r.clone(),
            None => return out,
        };
        let mut stack: Vec<(Obj, Rc<Inherited>)> = vec![(root, Rc::new(Inherited { resources: Dict::default(), media: None, crop: None, rotate: 0 }))];
        while let Some((node, inh)) = stack.pop() {
            if out.len() >= 50_000 {
                break;
            }
            let num = match &node {
                Obj::Ref(n, _) => {
                    if !visited.insert(*n) {
                        continue;
                    }
                    *n
                }
                _ => 0,
            };
            let obj = self.resolve(&node);
            let dict = match obj.as_dict() {
                Some(d) => d.clone(),
                None => continue,
            };
            let mine = Inherited {
                resources: self.dict(&dict, "Resources").unwrap_or_else(|| inh.resources.clone()),
                media: dict.get("MediaBox").and_then(|o| self.rect(o)).or(inh.media),
                crop: dict.get("CropBox").and_then(|o| self.rect(o)).or(inh.crop),
                rotate: self.get(&dict, "Rotate").as_i64().unwrap_or(inh.rotate),
            };
            match self.get(&dict, "Kids").as_array() {
                Some(kids) if dict.get("Type").and_then(|t| t.as_name()) != Some("Page") => {
                    let mine = Rc::new(mine);
                    // Se apilan al reves para visitarlos en orden.
                    for kid in kids.iter().rev() {
                        stack.push((kid.clone(), mine.clone()));
                    }
                }
                _ => {
                    let media = mine.media.unwrap_or([0.0, 0.0, 612.0, 792.0]);
                    // La caja de recorte, si existe, es lo que se ve.
                    let bbox = match mine.crop {
                        Some(c) => {
                            let r = [c[0].max(media[0]), c[1].max(media[1]), c[2].min(media[2]), c[3].min(media[3])];
                            if r[2] - r[0] > 1.0 && r[3] - r[1] > 1.0 {
                                r
                            } else {
                                media
                            }
                        }
                        None => media,
                    };
                    out.push(PageInfo { obj: num, dict, resources: mine.resources, bbox, rotate: mine.rotate.rem_euclid(360) });
                }
            }
        }
        out
    }

    /// Contenido de una pagina: sus flujos concatenados.
    pub fn page_content(&self, page: &PageInfo) -> Vec<u8> {
        let mut out = Vec::new();
        let contents = self.get(&page.dict, "Contents");
        let parts: Vec<Rc<Obj>> = match &*contents {
            Obj::Array(a) => a.iter().map(|o| self.resolve(o)).collect(),
            Obj::Stream { .. } => vec![contents.clone()],
            _ => Vec::new(),
        };
        for p in parts {
            if let Some(d) = self.stream_data(&p) {
                out.extend_from_slice(&d);
                out.push(b'\n');
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Cadenas de texto y fechas de los metadatos
// ---------------------------------------------------------------------------------------------

const PDFDOC_HIGH: [char; 33] = [
    '\u{2022}', '\u{2020}', '\u{2021}', '\u{2026}', '\u{2014}', '\u{2013}', '\u{192}', '\u{2044}', '\u{2039}', '\u{203A}', '\u{2212}', '\u{2030}',
    '\u{201E}', '\u{201C}', '\u{201D}', '\u{2018}', '\u{2019}', '\u{201A}', '\u{2122}', '\u{FB01}', '\u{FB02}', '\u{141}', '\u{152}', '\u{160}',
    '\u{178}', '\u{17D}', '\u{131}', '\u{142}', '\u{153}', '\u{161}', '\u{17E}', '\u{FFFD}', '\u{20AC}',
];

/// Decodifica una "cadena de texto" de PDF: UTF-16 con marca de orden, UTF-8 o PDFDocEncoding.
pub fn text_string(b: &[u8]) -> String {
    let s = if b.starts_with(&[0xFE, 0xFF]) || b.starts_with(&[0xFF, 0xFE]) {
        let be = b[0] == 0xFE;
        let units: Vec<u16> = b[2..].chunks_exact(2).map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }).collect();
        String::from_utf16_lossy(&units)
    } else if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF][..]) {
        String::from_utf8_lossy(rest).into_owned()
    } else {
        b.iter()
            .map(|&c| match c {
                0x80..=0xA0 => PDFDOC_HIGH[(c - 0x80) as usize],
                0x18..=0x1F => ['\u{2D8}', '\u{2C7}', '\u{2C6}', '\u{2D9}', '\u{2DD}', '\u{2DB}', '\u{2DA}', '\u{2DC}'][(c - 0x18) as usize],
                _ => c as char,
            })
            .collect()
    };
    s.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect::<String>().trim().to_string()
}

/// Convierte una fecha de PDF ("D:20240105131700-03'00'") a ISO 8601.
pub fn date_string(raw: &str) -> Option<String> {
    let s = raw.trim().trim_start_matches("D:");
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() < 4 {
        return None;
    }
    let part = |from: usize, len: usize, def: &str| digits.get(from..from + len).map(String::from).unwrap_or_else(|| def.to_string());
    let mut out = format!("{}-{}-{}T{}:{}:{}", part(0, 4, "0000"), part(4, 2, "01"), part(6, 2, "01"), part(8, 2, "00"), part(10, 2, "00"), part(12, 2, "00"));
    let rest = &s[digits.len()..];
    let mut chars = rest.chars();
    match chars.next() {
        Some('Z') | Some('z') => out.push('Z'),
        Some(sign @ ('+' | '-')) => {
            let tz: String = chars.filter(|c| c.is_ascii_digit()).collect();
            if tz.len() >= 2 {
                out.push(sign);
                out.push_str(&tz[..2]);
                out.push(':');
                out.push_str(tz.get(2..4).unwrap_or("00"));
            }
        }
        _ => {}
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fechas_y_cadenas() {
        assert_eq!(date_string("D:20240105131700-03'00'").as_deref(), Some("2024-01-05T13:17:00-03:00"));
        assert_eq!(date_string("D:20240105").as_deref(), Some("2024-01-05T00:00:00"));
        assert_eq!(date_string("20240105131700Z").as_deref(), Some("2024-01-05T13:17:00Z"));
        assert_eq!(date_string("sin fecha"), None);
        assert_eq!(text_string(&[0xFE, 0xFF, 0x00, b'A', 0x00, 0xF1]), "A\u{f1}");
        assert_eq!(text_string(b"Informe\x85final"), "Informe\u{2013}final");
    }

    #[test]
    fn pdf_minimo() {
        // PDF de una pagina escrito a mano, con la tabla de referencias correcta.
        let mut pdf = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for body in [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 200] >>",
            "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
            "<< /Length 5 >>\nstream\nHola!\nendstream",
        ] {
            offsets.push(pdf.len());
            pdf.push_str(&format!("{} 0 obj\n{}\nendobj\n", offsets.len(), body));
        }
        let xref = pdf.len();
        pdf.push_str("xref\n0 5\n0000000000 65535 f \n");
        for o in &offsets {
            pdf.push_str(&format!("{:010} 00000 n \n", o));
        }
        pdf.push_str(&format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", xref));

        let doc = Pdf::open(pdf.as_bytes()).unwrap();
        assert!(!doc.rebuilt);
        let pages = doc.pages();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].bbox, [0.0, 0.0, 300.0, 200.0]);
        assert_eq!(doc.page_content(&pages[0]), b"Hola!\n");

        // El mismo archivo con la tabla rota debe recuperarse recorriendo los objetos.
        let broken = pdf.replace(&format!("startxref\n{}", xref), "startxref\n999999");
        let doc = Pdf::open(broken.as_bytes()).unwrap();
        assert!(doc.rebuilt);
        assert_eq!(doc.pages().len(), 1);
    }
}
