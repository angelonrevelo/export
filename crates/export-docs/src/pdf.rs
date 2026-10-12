//! PDF 1.4 reader and writer. Classic xref tables, no rasterizer.
//! Object streams and cross-reference streams are refused with an error.

use std::collections::HashMap;

use crate::deflate::{deflate_fixed, deflate_lz77, inflate_vec};
use crate::error::{Error, Result};

const PAGE_W: i64 = 612;
const PAGE_H: i64 = 792;
const MARGIN: f32 = 54.0;

#[derive(Clone, Debug)]
enum Obj {
    Null,
    Bool(bool),
    Int(i64),
    Real(String),
    Name(String),
    Str(Vec<u8>),
    Arr(Vec<Obj>),
    Dict(Vec<(String, Obj)>),
    Stream { dict: Vec<(String, Obj)>, data: Vec<u8> },
    Ref(u32),
}

impl Obj {
    fn dict_get(&self, key: &str) -> Option<&Obj> {
        items(self)?.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn name_val(&self, key: &str) -> Option<&str> {
        match self.dict_get(key)? {
            Obj::Name(n) => Some(n.as_str()),
            _ => None,
        }
    }

    fn int_val(&self, key: &str) -> Option<i64> {
        match self.dict_get(key)? {
            Obj::Int(n) => Some(*n),
            _ => None,
        }
    }

    fn insert(&mut self, key: &str, val: Obj) {
        let Some(items) = items_mut(self) else {
            return;
        };
        if let Some((_, slot)) = items.iter_mut().find(|(k, _)| k == key) {
            *slot = val;
        } else {
            items.push((key.to_string(), val));
        }
    }

    fn remove(&mut self, key: &str) {
        if let Some(items) = items_mut(self) {
            items.retain(|(k, _)| k != key);
        }
    }
}

fn items(obj: &Obj) -> Option<&[(String, Obj)]> {
    match obj {
        Obj::Dict(d) | Obj::Stream { dict: d, .. } => Some(d),
        _ => None,
    }
}

fn items_mut(obj: &mut Obj) -> Option<&mut Vec<(String, Obj)>> {
    match obj {
        Obj::Dict(d) | Obj::Stream { dict: d, .. } => Some(d),
        _ => None,
    }
}

struct Src<'a> {
    data: &'a [u8],
    xref: Vec<Option<u64>>,
    cache: HashMap<u32, Obj>,
}

impl<'a> Src<'a> {
    fn open(data: &'a [u8]) -> Result<Self> {
        if !data.starts_with(b"%PDF-") {
            return Err(Error::new("pdf: missing %PDF- header"));
        }
        let mut xref = Vec::new();
        let start = find_startxref(data)?;
        let mut at = start;
        let mut newest = true;
        let mut guard = 0;
        while at > 0 {
            guard += 1;
            if guard > 16 {
                return Err(Error::new("pdf: xref chain too long"));
            }
            let prev = read_xref(data, at, &mut xref, newest)?;
            newest = false;
            at = prev;
        }
        if xref.is_empty() {
            return Err(Error::new("pdf: empty xref"));
        }
        Ok(Self {
            data,
            xref,
            cache: HashMap::new(),
        })
    }

    fn root_id(&self) -> Result<u32> {
        // Trailer was not stored. Re-read the newest trailer for /Root.
        let start = find_startxref(self.data)?;
        let trailer = read_trailer_only(self.data, start)?;
        match trailer.dict_get("Root") {
            Some(Obj::Ref(id)) => Ok(*id),
            _ => Err(Error::new("pdf: trailer has no root")),
        }
    }

    fn load(&mut self, id: u32) -> Result<Obj> {
        if let Some(obj) = self.cache.get(&id) {
            return Ok(obj.clone());
        }
        let off = self
            .xref
            .get(id as usize)
            .copied()
            .flatten()
            .ok_or_else(|| Error::new(format!("pdf: missing object {id}")))?;
        let obj = self.load_at(off, 0)?;
        self.cache.insert(id, obj.clone());
        Ok(obj)
    }

    fn load_at(&mut self, offset: u64, depth: u32) -> Result<Obj> {
        if depth > 32 {
            return Err(Error::new("pdf: object nesting too deep"));
        }
        if offset as usize >= self.data.len() {
            return Err(Error::new("pdf: object offset past end"));
        }
        let mut p = Cur::new(self.data, offset as usize);
        p.skip();
        let _id = p.parse_int()?;
        let _gen = p.parse_int()?;
        if !p.keyword("obj") {
            return Err(Error::new("pdf: expected obj"));
        }
        let val = p.parse_value()?;
        p.skip();
        if p.keyword("stream") {
            let Obj::Dict(dict) = val else {
                return Err(Error::new("pdf: stream without a dictionary"));
            };
            let len = stream_len(self, &dict, depth)?;
            if p.bump(b'\r') {
                // consumed
            }
            p.bump(b'\n');
            if p.i + len > p.data.len() {
                return Err(Error::new("pdf: stream length past end"));
            }
            let data = p.data[p.i..p.i + len].to_vec();
            Ok(Obj::Stream { dict, data })
        } else {
            Ok(val)
        }
    }

    fn page_ids(&mut self) -> Result<Vec<u32>> {
        let root = self.root_id()?;
        let catalog = self.load(root)?;
        let pages = match catalog.dict_get("Pages") {
            Some(Obj::Ref(id)) => *id,
            _ => return Err(Error::new("pdf: catalog has no pages")),
        };
        let mut out = Vec::new();
        self.walk(pages, 0, &mut out)?;
        Ok(out)
    }

    fn walk(&mut self, id: u32, depth: u32, out: &mut Vec<u32>) -> Result<()> {
        if depth > 64 {
            return Err(Error::new("pdf: page tree too deep"));
        }
        let node = self.load(id)?;
        let kind = node.name_val("Type").unwrap_or("");
        let has_kids = node.dict_get("Kids").is_some();
        if kind == "Page" || (!has_kids && node.dict_get("Contents").is_some()) {
            out.push(id);
            return Ok(());
        }
        let kids = node
            .dict_get("Kids")
            .cloned()
            .ok_or_else(|| Error::new("pdf: pages node has no kids"))?;
        let Obj::Arr(arr) = kids else {
            return Err(Error::new("pdf: kids is not an array"));
        };
        for kid in arr {
            let Obj::Ref(id) = kid else {
                return Err(Error::new("pdf: kid is not a reference"));
            };
            self.walk(id, depth + 1, out)?;
        }
        Ok(())
    }

    fn take_page(&mut self, page_id: u32) -> Result<Graph> {
        let mut page = self.load(page_id)?;
        for key in ["Resources", "MediaBox", "CropBox", "Rotate"] {
            if page.dict_get(key).is_none() {
                if let Some(v) = self.inherit(page_id, key)? {
                    page.insert(key, v);
                }
            }
        }
        page.remove("Parent");
        if page.dict_get("MediaBox").is_none() {
            page.insert(
                "MediaBox",
                Obj::Arr(vec![
                    Obj::Int(0),
                    Obj::Int(0),
                    Obj::Int(PAGE_W),
                    Obj::Int(PAGE_H),
                ]),
            );
        }
        page.insert("Type", Obj::Name("Page".into()));
        let mut map: HashMap<u32, u32> = HashMap::new();
        let mut objs = Vec::new();
        let rewritten = rewrite(self, page, &mut map, &mut objs)?;
        let page_index = objs.len() as u32;
        objs.push(rewritten);
        Ok(Graph {
            objs,
            page: page_index,
        })
    }

    fn inherit(&mut self, page_id: u32, key: &str) -> Result<Option<Obj>> {
        let page = self.load(page_id)?;
        let Some(Obj::Ref(mut id)) = page.dict_get("Parent").cloned() else {
            return Ok(None);
        };
        for _ in 0..32 {
            let node = self.load(id)?;
            if let Some(v) = node.dict_get(key).cloned() {
                return Ok(Some(v));
            }
            match node.dict_get("Parent") {
                Some(Obj::Ref(next)) => id = *next,
                _ => return Ok(None),
            }
        }
        Ok(None)
    }

    fn stream_bytes(&mut self, obj: &Obj) -> Result<Vec<u8>> {
        match obj {
            Obj::Ref(id) => {
                let loaded = self.load(*id)?;
                self.stream_bytes(&loaded)
            }
            Obj::Arr(items) => {
                let mut all = Vec::new();
                for item in items {
                    all.extend(self.stream_bytes(item)?);
                    all.push(b'\n');
                }
                Ok(all)
            }
            Obj::Stream { dict, data } => decode_stream(dict, data),
            _ => Ok(Vec::new()),
        }
    }
}

fn stream_len(src: &mut Src, dict: &[(String, Obj)], depth: u32) -> Result<usize> {
    let len_obj = dict
        .iter()
        .find(|(k, _)| k == "Length")
        .map(|(_, v)| v)
        .ok_or_else(|| Error::new("pdf: stream missing length"))?;
    let n = match len_obj {
        Obj::Int(n) if *n >= 0 => *n as usize,
        Obj::Ref(id) => {
            let loaded = src.load(*id)?;
            let _ = depth;
            match loaded {
                Obj::Int(n) if n >= 0 => n as usize,
                _ => return Err(Error::new("pdf: length is not an integer")),
            }
        }
        _ => return Err(Error::new("pdf: length is not an integer")),
    };
    Ok(n)
}

fn decode_stream(dict: &[(String, Obj)], data: &[u8]) -> Result<Vec<u8>> {
    let filter = dict.iter().find(|(k, _)| k == "Filter").map(|(_, v)| v);
    match filter {
        None => Ok(data.to_vec()),
        Some(Obj::Name(n)) if n == "FlateDecode" => inflate_vec(data),
        Some(Obj::Arr(a)) if a.len() == 1 => decode_stream(&[("Filter".into(), a[0].clone())], data),
        Some(_) => Err(Error::new("pdf: unsupported stream filter")),
    }
}

struct Graph {
    objs: Vec<Obj>,
    page: u32,
}

fn rewrite(src: &mut Src, obj: Obj, map: &mut HashMap<u32, u32>, objs: &mut Vec<Obj>) -> Result<Obj> {
    match obj {
        Obj::Arr(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(rewrite(src, item, map, objs)?);
            }
            Ok(Obj::Arr(out))
        }
        Obj::Dict(items) => {
            let mut out = Vec::with_capacity(items.len());
            for (k, v) in items {
                if k == "Parent" {
                    continue;
                }
                out.push((k, rewrite(src, v, map, objs)?));
            }
            Ok(Obj::Dict(out))
        }
        Obj::Stream { dict, data } => {
            let mut out = Vec::with_capacity(dict.len());
            for (k, v) in dict {
                out.push((k, rewrite(src, v, map, objs)?));
            }
            Ok(Obj::Stream { dict: out, data })
        }
        Obj::Ref(id) => {
            let slot = intern(src, id, map, objs)?;
            Ok(Obj::Ref(slot))
        }
        other => Ok(other),
    }
}

fn intern(src: &mut Src, id: u32, map: &mut HashMap<u32, u32>, objs: &mut Vec<Obj>) -> Result<u32> {
    if let Some(slot) = map.get(&id) {
        return Ok(*slot);
    }
    let slot = objs.len() as u32;
    map.insert(id, slot);
    objs.push(Obj::Null);
    let loaded = src.load(id)?;
    let rewritten = rewrite(src, loaded, map, objs)?;
    objs[slot as usize] = rewritten;
    Ok(slot)
}

fn shift(obj: &Obj, add: u32) -> Obj {
    match obj {
        Obj::Arr(items) => Obj::Arr(items.iter().map(|i| shift(i, add)).collect()),
        Obj::Dict(items) => Obj::Dict(items.iter().map(|(k, v)| (k.clone(), shift(v, add))).collect()),
        Obj::Stream { dict, data } => Obj::Stream {
            dict: dict.iter().map(|(k, v)| (k.clone(), shift(v, add))).collect(),
            data: data.clone(),
        },
        Obj::Ref(n) => Obj::Ref(*n + add),
        other => other.clone(),
    }
}

fn assemble(graphs: Vec<Graph>) -> Result<Vec<u8>> {
    if graphs.is_empty() {
        return Err(Error::new("pdf has no pages"));
    }
    // objs[i] is PDF object i+1. 1 = catalog, 2 = pages.
    let mut objs: Vec<Obj> = vec![Obj::Null, Obj::Null];
    let mut kids = Vec::new();
    for g in graphs {
        let base = objs.len() as u32; // graph ref 0 becomes object number base+1
        for obj in &g.objs {
            objs.push(shift(obj, base + 1));
        }
        let page_obj = base + 1 + g.page;
        let page_index = (page_obj - 1) as usize;
        objs[page_index].insert("Parent", Obj::Ref(2));
        objs[page_index].insert("Type", Obj::Name("Page".into()));
        kids.push(Obj::Ref(page_obj));
    }
    objs[1] = Obj::Dict(vec![
        ("Type".into(), Obj::Name("Pages".into())),
        ("Count".into(), Obj::Int(kids.len() as i64)),
        ("Kids".into(), Obj::Arr(kids)),
    ]);
    objs[0] = Obj::Dict(vec![
        ("Type".into(), Obj::Name("Catalog".into())),
        ("Pages".into(), Obj::Ref(2)),
    ]);
    Ok(serialize(&objs))
}

pub fn page_count(data: &[u8]) -> Result<usize> {
    Ok(Src::open(data)?.page_ids()?.len())
}

pub fn plain_text(data: &[u8]) -> Result<String> {
    let mut src = Src::open(data)?;
    let ids = src.page_ids()?;
    let mut out = String::new();
    for id in ids {
        let page = src.load(id)?;
        let Some(contents) = page.dict_get("Contents").cloned() else {
            continue;
        };
        let bytes = src.stream_bytes(&contents)?;
        let text = content_strings(&bytes);
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&text);
    }
    Ok(out)
}

pub fn split(data: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut src = Src::open(data)?;
    let ids = src.page_ids()?;
    if ids.is_empty() {
        return Err(Error::new("pdf has no pages"));
    }
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let g = src.take_page(id)?;
        out.push(assemble(vec![g])?);
    }
    Ok(out)
}

pub fn merge(inputs: &[&[u8]]) -> Result<Vec<u8>> {
    if inputs.len() < 2 {
        return Err(Error::new("pdf: need at least 2 PDFs to merge"));
    }
    let mut graphs = Vec::new();
    for input in inputs {
        let mut src = Src::open(input)?;
        let ids = src.page_ids()?;
        if ids.is_empty() {
            return Err(Error::new("pdf has no pages"));
        }
        for id in ids {
            graphs.push(src.take_page(id)?);
        }
    }
    assemble(graphs)
}

pub fn rotate(data: &[u8], degrees: i32) -> Result<Vec<u8>> {
    if !matches!(degrees, 90 | 180 | 270) {
        return Err(Error::new("degrees must be 90, 180, or 270"));
    }
    let mut src = Src::open(data)?;
    let ids = src.page_ids()?;
    if ids.is_empty() {
        return Err(Error::new("pdf has no pages"));
    }
    let mut graphs = Vec::new();
    for id in ids {
        let mut g = src.take_page(id)?;
        let cur = g.objs[g.page as usize].int_val("Rotate").unwrap_or(0);
        let sum = (cur + i64::from(degrees)).rem_euclid(360);
        g.objs[g.page as usize].insert("Rotate", Obj::Int(sum));
        graphs.push(g);
    }
    assemble(graphs)
}

pub fn watermark(data: &[u8], text: &str) -> Result<Vec<u8>> {
    let mut src = Src::open(data)?;
    let ids = src.page_ids()?;
    if ids.is_empty() {
        return Err(Error::new("pdf has no pages"));
    }
    let mut graphs = Vec::new();
    for id in ids {
        let mut g = src.take_page(id)?;
        stamp(&mut g, text)?;
        graphs.push(g);
    }
    assemble(graphs)
}

/// Recompress `/FlateDecode` streams. Returns the file only when a recompressed
/// stream inflates to the same bytes and the whole file is smaller.
pub fn shrink(data: &[u8]) -> Result<Vec<u8>> {
    let result = (|| {
        let before = flate_plains(data)?;
        let (out, replaced) = rebuild(data, recompress_one)?;
        if replaced == 0 || out.len() >= data.len() {
            return Err(Error::new("no smaller FlateDecode stream"));
        }
        let after = flate_plains(&out)?;
        if before != after {
            return Err(Error::new("recompressed FlateDecode inflated to different bytes"));
        }
        Ok(out)
    })();
    match result {
        Ok(out) => Ok(out),
        Err(e) => Err(Error::new(format!(
            "MISS: pdf shrink ({}) input_b={}",
            e.message,
            data.len()
        ))),
    }
}

/// Wrap uncompressed streams in fixed-Huffman FlateDecode so shrink has something
/// to recompress. Not a product conversion.
pub fn flate_fixture(data: &[u8]) -> Result<Vec<u8>> {
    let (out, _) = rebuild(data, wrap_fixed)?;
    Ok(out)
}

fn recompress_one(obj: Obj) -> Result<Option<Obj>> {
    let Obj::Stream { dict, data } = obj else {
        return Ok(None);
    };
    if !is_flate(&dict) {
        return Ok(None);
    }
    let plain = inflate_vec(&data)?;
    let packed = deflate_lz77(&plain);
    if inflate_vec(&packed)? != plain {
        return Err(Error::new(
            "recompressed FlateDecode inflated to different bytes",
        ));
    }
    if packed.len() >= data.len() {
        return Ok(None);
    }
    Ok(Some(Obj::Stream { dict, data: packed }))
}

fn wrap_fixed(obj: Obj) -> Result<Option<Obj>> {
    let Obj::Stream { mut dict, data } = obj else {
        return Ok(None);
    };
    if is_flate(&dict) || dict.iter().any(|(k, _)| k == "Filter") {
        return Ok(None);
    }
    let packed = deflate_fixed(&data);
    if let Some((_, slot)) = dict.iter_mut().find(|(k, _)| k == "Length") {
        *slot = Obj::Int(packed.len() as i64);
    } else {
        dict.push(("Length".into(), Obj::Int(packed.len() as i64)));
    }
    dict.push(("Filter".into(), Obj::Name("FlateDecode".into())));
    Ok(Some(Obj::Stream { dict, data: packed }))
}

fn is_flate(dict: &[(String, Obj)]) -> bool {
    let Some((_, filter)) = dict.iter().find(|(k, _)| k == "Filter") else {
        return false;
    };
    match filter {
        Obj::Name(n) => n == "FlateDecode",
        Obj::Arr(items) => {
            items.len() == 1 && matches!(&items[0], Obj::Name(n) if n == "FlateDecode")
        }
        _ => false,
    }
}

/// True when every `/FlateDecode` stream in `left` inflates to the same bytes as `right`.
pub fn flate_plain_matches(left: &[u8], right: &[u8]) -> Result<bool> {
    Ok(flate_plains(left)? == flate_plains(right)?)
}

fn flate_plains(data: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut src = Src::open(data)?;
    let mut out = Vec::new();
    for id in 1..src.xref.len() {
        if src.xref[id].is_none() {
            continue;
        }
        if let Obj::Stream { dict, data } = src.load(id as u32)? {
            if is_flate(&dict) {
                out.push(inflate_vec(&data)?);
            }
        }
    }
    Ok(out)
}

struct Piece {
    gen: u16,
    bytes: Vec<u8>,
}

fn rebuild<F>(data: &[u8], mut edit: F) -> Result<(Vec<u8>, usize)>
where
    F: FnMut(Obj) -> Result<Option<Obj>>,
{
    let start = find_startxref(data)?;
    let trailer = read_trailer_only(data, start)?;
    if trailer.dict_get("Encrypt").is_some() {
        return Err(Error::new("encrypted"));
    }
    let root = match trailer.dict_get("Root") {
        Some(Obj::Ref(id)) => *id,
        _ => return Err(Error::new("trailer has no root")),
    };
    let mut src = Src::open(data)?;
    let n = src.xref.len();
    let mut pieces = Vec::with_capacity(n);
    pieces.resize_with(n, || None);
    let mut replaced = 0usize;
    let mut earliest = data.len();
    for (id, slot) in pieces.iter_mut().enumerate().skip(1) {
        let Some(off) = src.xref[id] else {
            continue;
        };
        earliest = earliest.min(off as usize);
        let gen = read_gen(data, off as usize)?;
        let loaded = src.load(id as u32)?;
        if let Some(new_obj) = edit(loaded.clone())? {
            replaced += 1;
            *slot = Some(Piece {
                gen: 0,
                bytes: object_bytes(id as u32, &new_obj),
            });
        } else {
            *slot = Some(Piece {
                gen,
                bytes: raw_object(data, off, &loaded)?.to_vec(),
            });
        }
    }
    let preamble = if earliest < data.len() {
        &data[..earliest]
    } else {
        &[]
    };
    Ok((stitch(preamble, &pieces, root), replaced))
}

fn object_bytes(id: u32, obj: &Obj) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
    write_obj(&mut out, obj);
    out.extend_from_slice(b"\nendobj");
    out
}

fn read_gen(data: &[u8], offset: usize) -> Result<u16> {
    let mut p = Cur::new(data, offset);
    p.skip();
    let _id = p.parse_int()?;
    let gen = p.parse_int()?;
    u16::try_from(gen).map_err(|_| Error::new("pdf: bad generation"))
}

fn raw_object<'a>(data: &'a [u8], offset: u64, loaded: &Obj) -> Result<&'a [u8]> {
    let start = offset as usize;
    let mut p = Cur::new(data, start);
    p.skip();
    let _id = p.parse_int()?;
    let _gen = p.parse_int()?;
    if !p.keyword("obj") {
        return Err(Error::new("pdf: expected obj"));
    }
    let _val = p.parse_value()?;
    p.skip();
    if p.keyword("stream") {
        let Obj::Stream { data: stream, .. } = loaded else {
            return Err(Error::new("pdf: stream object mismatch"));
        };
        p.bump(b'\r');
        p.bump(b'\n');
        if p.i + stream.len() > data.len() {
            return Err(Error::new("pdf: stream length past end"));
        }
        p.i += stream.len();
        if !p.keyword("endstream") {
            return Err(Error::new("pdf: missing endstream"));
        }
    }
    if !p.keyword("endobj") {
        return Err(Error::new("pdf: missing endobj"));
    }
    Ok(&data[start..p.i])
}

fn stitch(preamble: &[u8], pieces: &[Option<Piece>], root: u32) -> Vec<u8> {
    let size = pieces.len();
    let mut out = preamble.to_vec();
    if !out.is_empty() && !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    let mut offsets = vec![None; size];
    for (id, piece) in pieces.iter().enumerate().skip(1) {
        let Some(piece) = piece else {
            continue;
        };
        offsets[id] = Some((out.len() as u64, piece.gen));
        out.extend_from_slice(&piece.bytes);
        if !out.ends_with(b"\n") {
            out.push(b'\n');
        }
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for slot in offsets.iter().skip(1) {
        match slot {
            Some((off, gen)) => {
                out.extend_from_slice(format!("{off:010} {gen:05} n \n").as_bytes());
            }
            None => out.extend_from_slice(b"0000000000 00000 f \n"),
        }
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root {root} 0 R >>\nstartxref\n{xref_at}\n%%EOF\n")
            .as_bytes(),
    );
    out
}

fn stamp(g: &mut Graph, text: &str) -> Result<()> {
    let contents = g.objs[g.page as usize]
        .dict_get("Contents")
        .cloned()
        .ok_or_else(|| Error::new("pdf: page has no contents"))?;
    let Obj::Ref(slot) = contents else {
        return Err(Error::new("pdf: watermark needs one content stream"));
    };
    let stream = g
        .objs
        .get_mut(slot as usize)
        .ok_or_else(|| Error::new("pdf: bad contents ref"))?;
    let Obj::Stream { dict, data } = stream else {
        return Err(Error::new("pdf: watermark needs one content stream"));
    };
    if dict.iter().any(|(k, _)| k == "Filter") {
        return Err(Error::new(
            "pdf: watermark does not decode filtered content streams",
        ));
    }
    let line = format!("\nBT /F1 36 Tf 1 0 0 1 72 400 Tm {} Tj ET\n", pdf_string(text));
    data.extend_from_slice(line.as_bytes());
    let n = data.len() as i64;
    if let Some((_, slot)) = dict.iter_mut().find(|(k, _)| k == "Length") {
        *slot = Obj::Int(n);
    } else {
        dict.push(("Length".into(), Obj::Int(n)));
    }
    ensure_font(&mut g.objs[g.page as usize]);
    Ok(())
}

fn ensure_font(page: &mut Obj) {
    let resources = page.dict_get("Resources").cloned().unwrap_or_else(|| Obj::Dict(vec![]));
    let mut resources = resources;
    let mut font = resources
        .dict_get("Font")
        .cloned()
        .unwrap_or_else(|| Obj::Dict(vec![]));
    if font.dict_get("F1").is_none() {
        font.insert("F1", helvetica("Helvetica"));
    }
    resources.insert("Font", font);
    page.insert("Resources", resources);
}

fn helvetica(base: &str) -> Obj {
    Obj::Dict(vec![
        ("Type".into(), Obj::Name("Font".into())),
        ("Subtype".into(), Obj::Name("Type1".into())),
        ("BaseFont".into(), Obj::Name(base.into())),
        ("Encoding".into(), Obj::Name("WinAnsiEncoding".into())),
    ])
}

pub struct Span {
    pub text: String,
    pub font: u8,
    pub size: f32,
}

pub struct Line {
    pub spans: Vec<Span>,
    pub leading: f32,
}

/// One-page PDF that paints `rgb` (`width` × `height` × 3, top-down) as an image XObject.
/// `flate` stores the pixels in a FlateDecode stream. Otherwise the stream is the raw pixels.
pub fn image_xobject(width: u32, height: u32, rgb: &[u8], flate: bool) -> Result<Vec<u8>> {
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(3))
        .ok_or_else(|| Error::new("image size"))?;
    if width == 0 || height == 0 || width > 4096 || height > 4096 || rgb.len() != pixels {
        return Err(Error::new("image size"));
    }
    let data = if flate {
        let packed = deflate_lz77(rgb);
        if inflate_vec(&packed)? != rgb {
            return Err(Error::new("image flate did not round-trip"));
        }
        packed
    } else {
        rgb.to_vec()
    };
    let mut image_dict = vec![
        ("Type".into(), Obj::Name("XObject".into())),
        ("Subtype".into(), Obj::Name("Image".into())),
        ("Width".into(), Obj::Int(i64::from(width))),
        ("Height".into(), Obj::Int(i64::from(height))),
        ("ColorSpace".into(), Obj::Name("DeviceRGB".into())),
        ("BitsPerComponent".into(), Obj::Int(8)),
        ("Length".into(), Obj::Int(data.len() as i64)),
    ];
    if flate {
        image_dict.push(("Filter".into(), Obj::Name("FlateDecode".into())));
    }
    let content = format!("q {width} 0 0 {height} 0 0 cm /Im1 Do Q\n");
    let content_bytes = content.into_bytes();
    let objs = vec![
        Obj::Dict(vec![
            ("Type".into(), Obj::Name("Catalog".into())),
            ("Pages".into(), Obj::Ref(2)),
        ]),
        Obj::Dict(vec![
            ("Type".into(), Obj::Name("Pages".into())),
            ("Count".into(), Obj::Int(1)),
            ("Kids".into(), Obj::Arr(vec![Obj::Ref(3)])),
        ]),
        Obj::Dict(vec![
            ("Type".into(), Obj::Name("Page".into())),
            (
                "MediaBox".into(),
                Obj::Arr(vec![
                    Obj::Int(0),
                    Obj::Int(0),
                    Obj::Int(i64::from(width)),
                    Obj::Int(i64::from(height)),
                ]),
            ),
            (
                "Resources".into(),
                Obj::Dict(vec![(
                    "XObject".into(),
                    Obj::Dict(vec![("Im1".into(), Obj::Ref(4))]),
                )]),
            ),
            ("Contents".into(), Obj::Ref(5)),
        ]),
        Obj::Stream {
            dict: image_dict,
            data,
        },
        Obj::Stream {
            dict: vec![("Length".into(), Obj::Int(content_bytes.len() as i64))],
            data: content_bytes,
        },
    ];
    Ok(serialize(&objs))
}

/// One-page text PDF that also paints `rgb` as a FlateDecode image XObject.
pub fn from_lines_with_image(
    lines: &[Line],
    width: u32,
    height: u32,
    rgb: &[u8],
) -> Result<Vec<u8>> {
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(3))
        .ok_or_else(|| Error::new("image size"))?;
    if width == 0 || height == 0 || width > 4096 || height > 4096 || rgb.len() != pixels {
        return Err(Error::new("image size"));
    }
    let packed = deflate_lz77(rgb);
    if inflate_vec(&packed)? != rgb {
        return Err(Error::new("image flate did not round-trip"));
    }
    assemble(vec![page_with_image(lines, width, height, &packed)?])
}

/// One-page text PDF that paints two FlateDecode image XObjects, then the text.
pub(crate) fn from_lines_with_two_images(
    lines: &[Line],
    first_width: u32,
    first_height: u32,
    first_rgb: &[u8],
    second_width: u32,
    second_height: u32,
    second_rgb: &[u8],
) -> Result<Vec<u8>> {
    let first = flate_image(first_width, first_height, first_rgb)?;
    let second = flate_image(second_width, second_height, second_rgb)?;
    assemble(vec![page_with_two_images(
        lines,
        first_width,
        first_height,
        &first,
        second_width,
        second_height,
        &second,
    )?])
}

/// One-page text PDF that paints three FlateDecode image XObjects, then the text.
pub(crate) fn from_lines_with_three_images(
    lines: &[Line],
    pictures: [(u32, u32, &[u8]); 3],
) -> Result<Vec<u8>> {
    let mut packed = Vec::with_capacity(3);
    for (width, height, rgb) in pictures {
        packed.push((width, height, flate_image(width, height, rgb)?));
    }
    assemble(vec![page_with_three_images(lines, &packed)?])
}

/// One-page text PDF that paints four FlateDecode image XObjects, then the text.
pub(crate) fn from_lines_with_four_images(
    lines: &[Line],
    pictures: [(u32, u32, &[u8]); 4],
) -> Result<Vec<u8>> {
    let mut packed = Vec::with_capacity(4);
    for (width, height, rgb) in pictures {
        packed.push((width, height, flate_image(width, height, rgb)?));
    }
    assemble(vec![page_with_four_images(lines, &packed)?])
}

/// One-page text PDF that paints five FlateDecode image XObjects, then the text.
pub(crate) fn from_lines_with_five_images(
    lines: &[Line],
    pictures: [(u32, u32, &[u8]); 5],
) -> Result<Vec<u8>> {
    let mut packed = Vec::with_capacity(5);
    for (width, height, rgb) in pictures {
        packed.push((width, height, flate_image(width, height, rgb)?));
    }
    assemble(vec![page_with_five_images(lines, &packed)?])
}

/// One-page text PDF that paints six FlateDecode image XObjects, then the text.
pub(crate) fn from_lines_with_six_images(
    lines: &[Line],
    pictures: [(u32, u32, &[u8]); 6],
) -> Result<Vec<u8>> {
    let mut packed = Vec::with_capacity(6);
    for (width, height, rgb) in pictures {
        packed.push((width, height, flate_image(width, height, rgb)?));
    }
    assemble(vec![page_with_six_images(lines, &packed)?])
}

fn flate_image(width: u32, height: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(3))
        .ok_or_else(|| Error::new("image size"))?;
    if width == 0 || height == 0 || width > 4096 || height > 4096 || rgb.len() != pixels {
        return Err(Error::new("image size"));
    }
    let packed = deflate_lz77(rgb);
    if inflate_vec(&packed)? != rgb {
        return Err(Error::new("image flate did not round-trip"));
    }
    Ok(packed)
}

pub(crate) fn contains_image(data: &[u8], width: u32, height: u32, rgb: &[u8]) -> Result<bool> {
    let mut src = Src::open(data)?;
    let count = src.xref.len();
    for id in 1..count as u32 {
        let present = src.xref.get(id as usize).copied().flatten().is_some();
        if !present {
            continue;
        }
        let obj = match src.load(id) {
            Ok(obj) => obj,
            Err(_) => continue,
        };
        let is_image = obj.name_val("Subtype") == Some("Image");
        let w = obj.int_val("Width");
        let h = obj.int_val("Height");
        if !is_image || w != Some(i64::from(width)) || h != Some(i64::from(height)) {
            continue;
        }
        let Obj::Stream { dict, data: raw } = obj else {
            continue;
        };
        if decode_stream(&dict, &raw)? == rgb {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn from_lines(lines: &[Line]) -> Result<Vec<u8>> {
    let mut pages: Vec<Vec<&Line>> = Vec::new();
    let mut current: Vec<&Line> = Vec::new();
    let mut y = PAGE_H as f32 - MARGIN;
    for line in lines {
        let lead = if line.leading > 0.0 { line.leading } else { 14.0 };
        if y - lead < MARGIN && !current.is_empty() {
            pages.push(std::mem::take(&mut current));
            y = PAGE_H as f32 - MARGIN;
        }
        y -= lead;
        current.push(line);
    }
    if !current.is_empty() {
        pages.push(current);
    }
    if pages.is_empty() {
        pages.push(Vec::new());
    }
    let mut graphs = Vec::new();
    for page_lines in pages {
        graphs.push(page_graph(&page_lines)?);
    }
    assemble(graphs)
}

fn page_with_image(lines: &[Line], width: u32, height: u32, flate: &[u8]) -> Result<Graph> {
    let mut content = format!("q {width} 0 0 {height} {MARGIN:.2} {MARGIN:.2} cm /Im1 Do Q\nBT\n");
    let mut y = PAGE_H as f32 - MARGIN;
    for line in lines {
        let lead = if line.leading > 0.0 { line.leading } else { 14.0 };
        let size = line.spans.first().map(|s| s.size).unwrap_or(11.0);
        y -= lead;
        let baseline = y + (lead - size).max(0.0) * 0.2;
        content.push_str(&format!("1 0 0 1 {MARGIN:.2} {baseline:.2} Tm\n"));
        for span in &line.spans {
            let font = match span.font {
                1 => "F2",
                2 => "F3",
                3 => "F4",
                _ => "F1",
            };
            content.push_str(&format!("/{font} {size:.2} Tf\n", size = span.size));
            content.push_str(&pdf_string(&span.text));
            content.push_str(" Tj\n");
        }
    }
    content.push_str("ET\n");
    let data = content.into_bytes();
    let stream = Obj::Stream {
        dict: vec![("Length".into(), Obj::Int(data.len() as i64))],
        data,
    };
    let image = Obj::Stream {
        dict: vec![
            ("Type".into(), Obj::Name("XObject".into())),
            ("Subtype".into(), Obj::Name("Image".into())),
            ("Width".into(), Obj::Int(i64::from(width))),
            ("Height".into(), Obj::Int(i64::from(height))),
            ("ColorSpace".into(), Obj::Name("DeviceRGB".into())),
            ("BitsPerComponent".into(), Obj::Int(8)),
            ("Length".into(), Obj::Int(flate.len() as i64)),
            ("Filter".into(), Obj::Name("FlateDecode".into())),
        ],
        data: flate.to_vec(),
    };
    let fonts = Obj::Dict(vec![
        ("F1".into(), helvetica("Helvetica")),
        ("F2".into(), helvetica("Helvetica-Bold")),
        ("F3".into(), helvetica("Helvetica-Oblique")),
        ("F4".into(), helvetica("Helvetica-BoldOblique")),
    ]);
    let page = Obj::Dict(vec![
        ("Type".into(), Obj::Name("Page".into())),
        (
            "MediaBox".into(),
            Obj::Arr(vec![
                Obj::Int(0),
                Obj::Int(0),
                Obj::Int(PAGE_W),
                Obj::Int(PAGE_H),
            ]),
        ),
        ("Contents".into(), Obj::Ref(0)),
        (
            "Resources".into(),
            Obj::Dict(vec![
                ("Font".into(), fonts),
                (
                    "XObject".into(),
                    Obj::Dict(vec![("Im1".into(), Obj::Ref(2))]),
                ),
            ]),
        ),
    ]);
    Ok(Graph {
        objs: vec![stream, page, image],
        page: 1,
    })
}

fn page_with_two_images(
    lines: &[Line],
    first_width: u32,
    first_height: u32,
    first_flate: &[u8],
    second_width: u32,
    second_height: u32,
    second_flate: &[u8],
) -> Result<Graph> {
    let second_y = MARGIN + first_height as f32;
    let mut content = format!(
        "q {first_width} 0 0 {first_height} {MARGIN:.2} {MARGIN:.2} cm /Im1 Do Q\nq {second_width} 0 0 {second_height} {MARGIN:.2} {second_y:.2} cm /Im2 Do Q\nBT\n"
    );
    let mut y = PAGE_H as f32 - MARGIN;
    for line in lines {
        let lead = if line.leading > 0.0 { line.leading } else { 14.0 };
        let size = line.spans.first().map(|s| s.size).unwrap_or(11.0);
        y -= lead;
        let baseline = y + (lead - size).max(0.0) * 0.2;
        content.push_str(&format!("1 0 0 1 {MARGIN:.2} {baseline:.2} Tm\n"));
        for span in &line.spans {
            let font = match span.font {
                1 => "F2",
                2 => "F3",
                3 => "F4",
                _ => "F1",
            };
            content.push_str(&format!("/{font} {size:.2} Tf\n", size = span.size));
            content.push_str(&pdf_string(&span.text));
            content.push_str(" Tj\n");
        }
    }
    content.push_str("ET\n");
    let data = content.into_bytes();
    let stream = Obj::Stream {
        dict: vec![("Length".into(), Obj::Int(data.len() as i64))],
        data,
    };
    let first = flate_image_obj(first_width, first_height, first_flate);
    let second = flate_image_obj(second_width, second_height, second_flate);
    let fonts = Obj::Dict(vec![
        ("F1".into(), helvetica("Helvetica")),
        ("F2".into(), helvetica("Helvetica-Bold")),
        ("F3".into(), helvetica("Helvetica-Oblique")),
        ("F4".into(), helvetica("Helvetica-BoldOblique")),
    ]);
    let page = Obj::Dict(vec![
        ("Type".into(), Obj::Name("Page".into())),
        (
            "MediaBox".into(),
            Obj::Arr(vec![
                Obj::Int(0),
                Obj::Int(0),
                Obj::Int(PAGE_W),
                Obj::Int(PAGE_H),
            ]),
        ),
        ("Contents".into(), Obj::Ref(0)),
        (
            "Resources".into(),
            Obj::Dict(vec![
                ("Font".into(), fonts),
                (
                    "XObject".into(),
                    Obj::Dict(vec![
                        ("Im1".into(), Obj::Ref(2)),
                        ("Im2".into(), Obj::Ref(3)),
                    ]),
                ),
            ]),
        ),
    ]);
    Ok(Graph {
        objs: vec![stream, page, first, second],
        page: 1,
    })
}

fn page_with_three_images(lines: &[Line], pictures: &[(u32, u32, Vec<u8>)]) -> Result<Graph> {
    let (w0, h0, _) = &pictures[0];
    let (w1, h1, _) = &pictures[1];
    let (w2, h2, _) = &pictures[2];
    let y1 = MARGIN + *h0 as f32;
    let y2 = y1 + *h1 as f32;
    let mut content = format!(
        "q {w0} 0 0 {h0} {MARGIN:.2} {MARGIN:.2} cm /Im1 Do Q\nq {w1} 0 0 {h1} {MARGIN:.2} {y1:.2} cm /Im2 Do Q\nq {w2} 0 0 {h2} {MARGIN:.2} {y2:.2} cm /Im3 Do Q\nBT\n"
    );
    let mut y = PAGE_H as f32 - MARGIN;
    for line in lines {
        let lead = if line.leading > 0.0 { line.leading } else { 14.0 };
        let size = line.spans.first().map(|s| s.size).unwrap_or(11.0);
        y -= lead;
        let baseline = y + (lead - size).max(0.0) * 0.2;
        content.push_str(&format!("1 0 0 1 {MARGIN:.2} {baseline:.2} Tm\n"));
        for span in &line.spans {
            let font = match span.font {
                1 => "F2",
                2 => "F3",
                3 => "F4",
                _ => "F1",
            };
            content.push_str(&format!("/{font} {size:.2} Tf\n", size = span.size));
            content.push_str(&pdf_string(&span.text));
            content.push_str(" Tj\n");
        }
    }
    content.push_str("ET\n");
    let data = content.into_bytes();
    let stream = Obj::Stream {
        dict: vec![("Length".into(), Obj::Int(data.len() as i64))],
        data,
    };
    let fonts = Obj::Dict(vec![
        ("F1".into(), helvetica("Helvetica")),
        ("F2".into(), helvetica("Helvetica-Bold")),
        ("F3".into(), helvetica("Helvetica-Oblique")),
        ("F4".into(), helvetica("Helvetica-BoldOblique")),
    ]);
    let page = Obj::Dict(vec![
        ("Type".into(), Obj::Name("Page".into())),
        (
            "MediaBox".into(),
            Obj::Arr(vec![
                Obj::Int(0),
                Obj::Int(0),
                Obj::Int(PAGE_W),
                Obj::Int(PAGE_H),
            ]),
        ),
        ("Contents".into(), Obj::Ref(0)),
        (
            "Resources".into(),
            Obj::Dict(vec![
                ("Font".into(), fonts),
                (
                    "XObject".into(),
                    Obj::Dict(vec![
                        ("Im1".into(), Obj::Ref(2)),
                        ("Im2".into(), Obj::Ref(3)),
                        ("Im3".into(), Obj::Ref(4)),
                    ]),
                ),
            ]),
        ),
    ]);
    let mut objs = vec![stream, page];
    for (width, height, flate) in pictures {
        objs.push(flate_image_obj(*width, *height, flate));
    }
    Ok(Graph { objs, page: 1 })
}

fn page_with_four_images(lines: &[Line], pictures: &[(u32, u32, Vec<u8>)]) -> Result<Graph> {
    let names = ["Im1", "Im2", "Im3", "Im4"];
    let mut y_pos = MARGIN;
    let mut content = String::new();
    for (i, (width, height, _)) in pictures.iter().enumerate() {
        content.push_str(&format!(
            "q {width} 0 0 {height} {MARGIN:.2} {y_pos:.2} cm /{name} Do Q\n",
            name = names[i],
        ));
        y_pos += *height as f32;
    }
    content.push_str("BT\n");
    let mut y = PAGE_H as f32 - MARGIN;
    for line in lines {
        let lead = if line.leading > 0.0 { line.leading } else { 14.0 };
        let size = line.spans.first().map(|s| s.size).unwrap_or(11.0);
        y -= lead;
        let baseline = y + (lead - size).max(0.0) * 0.2;
        content.push_str(&format!("1 0 0 1 {MARGIN:.2} {baseline:.2} Tm\n"));
        for span in &line.spans {
            let font = match span.font {
                1 => "F2",
                2 => "F3",
                3 => "F4",
                _ => "F1",
            };
            content.push_str(&format!("/{font} {size:.2} Tf\n", size = span.size));
            content.push_str(&pdf_string(&span.text));
            content.push_str(" Tj\n");
        }
    }
    content.push_str("ET\n");
    let data = content.into_bytes();
    let stream = Obj::Stream {
        dict: vec![("Length".into(), Obj::Int(data.len() as i64))],
        data,
    };
    let fonts = Obj::Dict(vec![
        ("F1".into(), helvetica("Helvetica")),
        ("F2".into(), helvetica("Helvetica-Bold")),
        ("F3".into(), helvetica("Helvetica-Oblique")),
        ("F4".into(), helvetica("Helvetica-BoldOblique")),
    ]);
    let page = Obj::Dict(vec![
        ("Type".into(), Obj::Name("Page".into())),
        (
            "MediaBox".into(),
            Obj::Arr(vec![
                Obj::Int(0),
                Obj::Int(0),
                Obj::Int(PAGE_W),
                Obj::Int(PAGE_H),
            ]),
        ),
        ("Contents".into(), Obj::Ref(0)),
        (
            "Resources".into(),
            Obj::Dict(vec![
                ("Font".into(), fonts),
                (
                    "XObject".into(),
                    Obj::Dict(vec![
                        ("Im1".into(), Obj::Ref(2)),
                        ("Im2".into(), Obj::Ref(3)),
                        ("Im3".into(), Obj::Ref(4)),
                        ("Im4".into(), Obj::Ref(5)),
                    ]),
                ),
            ]),
        ),
    ]);
    let mut objs = vec![stream, page];
    for (width, height, flate) in pictures {
        objs.push(flate_image_obj(*width, *height, flate));
    }
    Ok(Graph { objs, page: 1 })
}

fn page_with_five_images(lines: &[Line], pictures: &[(u32, u32, Vec<u8>)]) -> Result<Graph> {
    let names = ["Im1", "Im2", "Im3", "Im4", "Im5"];
    let mut y_pos = MARGIN;
    let mut content = String::new();
    for (i, (width, height, _)) in pictures.iter().enumerate() {
        content.push_str(&format!(
            "q {width} 0 0 {height} {MARGIN:.2} {y_pos:.2} cm /{name} Do Q\n",
            name = names[i],
        ));
        y_pos += *height as f32;
    }
    content.push_str("BT\n");
    let mut y = PAGE_H as f32 - MARGIN;
    for line in lines {
        let lead = if line.leading > 0.0 { line.leading } else { 14.0 };
        let size = line.spans.first().map(|s| s.size).unwrap_or(11.0);
        y -= lead;
        let baseline = y + (lead - size).max(0.0) * 0.2;
        content.push_str(&format!("1 0 0 1 {MARGIN:.2} {baseline:.2} Tm\n"));
        for span in &line.spans {
            let font = match span.font {
                1 => "F2",
                2 => "F3",
                3 => "F4",
                _ => "F1",
            };
            content.push_str(&format!("/{font} {size:.2} Tf\n", size = span.size));
            content.push_str(&pdf_string(&span.text));
            content.push_str(" Tj\n");
        }
    }
    content.push_str("ET\n");
    let data = content.into_bytes();
    let stream = Obj::Stream {
        dict: vec![("Length".into(), Obj::Int(data.len() as i64))],
        data,
    };
    let fonts = Obj::Dict(vec![
        ("F1".into(), helvetica("Helvetica")),
        ("F2".into(), helvetica("Helvetica-Bold")),
        ("F3".into(), helvetica("Helvetica-Oblique")),
        ("F4".into(), helvetica("Helvetica-BoldOblique")),
    ]);
    let page = Obj::Dict(vec![
        ("Type".into(), Obj::Name("Page".into())),
        (
            "MediaBox".into(),
            Obj::Arr(vec![
                Obj::Int(0),
                Obj::Int(0),
                Obj::Int(PAGE_W),
                Obj::Int(PAGE_H),
            ]),
        ),
        ("Contents".into(), Obj::Ref(0)),
        (
            "Resources".into(),
            Obj::Dict(vec![
                ("Font".into(), fonts),
                (
                    "XObject".into(),
                    Obj::Dict(vec![
                        ("Im1".into(), Obj::Ref(2)),
                        ("Im2".into(), Obj::Ref(3)),
                        ("Im3".into(), Obj::Ref(4)),
                        ("Im4".into(), Obj::Ref(5)),
                        ("Im5".into(), Obj::Ref(6)),
                    ]),
                ),
            ]),
        ),
    ]);
    let mut objs = vec![stream, page];
    for (width, height, flate) in pictures {
        objs.push(flate_image_obj(*width, *height, flate));
    }
    Ok(Graph { objs, page: 1 })
}

fn page_with_six_images(lines: &[Line], pictures: &[(u32, u32, Vec<u8>)]) -> Result<Graph> {
    let names = ["Im1", "Im2", "Im3", "Im4", "Im5", "Im6"];
    let mut y_pos = MARGIN;
    let mut content = String::new();
    for (i, (width, height, _)) in pictures.iter().enumerate() {
        content.push_str(&format!(
            "q {width} 0 0 {height} {MARGIN:.2} {y_pos:.2} cm /{name} Do Q\n",
            name = names[i],
        ));
        y_pos += *height as f32;
    }
    content.push_str("BT\n");
    let mut y = PAGE_H as f32 - MARGIN;
    for line in lines {
        let lead = if line.leading > 0.0 { line.leading } else { 14.0 };
        let size = line.spans.first().map(|s| s.size).unwrap_or(11.0);
        y -= lead;
        let baseline = y + (lead - size).max(0.0) * 0.2;
        content.push_str(&format!("1 0 0 1 {MARGIN:.2} {baseline:.2} Tm\n"));
        for span in &line.spans {
            let font = match span.font {
                1 => "F2",
                2 => "F3",
                3 => "F4",
                _ => "F1",
            };
            content.push_str(&format!("/{font} {size:.2} Tf\n", size = span.size));
            content.push_str(&pdf_string(&span.text));
            content.push_str(" Tj\n");
        }
    }
    content.push_str("ET\n");
    let data = content.into_bytes();
    let stream = Obj::Stream {
        dict: vec![("Length".into(), Obj::Int(data.len() as i64))],
        data,
    };
    let fonts = Obj::Dict(vec![
        ("F1".into(), helvetica("Helvetica")),
        ("F2".into(), helvetica("Helvetica-Bold")),
        ("F3".into(), helvetica("Helvetica-Oblique")),
        ("F4".into(), helvetica("Helvetica-BoldOblique")),
    ]);
    let page = Obj::Dict(vec![
        ("Type".into(), Obj::Name("Page".into())),
        (
            "MediaBox".into(),
            Obj::Arr(vec![
                Obj::Int(0),
                Obj::Int(0),
                Obj::Int(PAGE_W),
                Obj::Int(PAGE_H),
            ]),
        ),
        ("Contents".into(), Obj::Ref(0)),
        (
            "Resources".into(),
            Obj::Dict(vec![
                ("Font".into(), fonts),
                (
                    "XObject".into(),
                    Obj::Dict(vec![
                        ("Im1".into(), Obj::Ref(2)),
                        ("Im2".into(), Obj::Ref(3)),
                        ("Im3".into(), Obj::Ref(4)),
                        ("Im4".into(), Obj::Ref(5)),
                        ("Im5".into(), Obj::Ref(6)),
                        ("Im6".into(), Obj::Ref(7)),
                    ]),
                ),
            ]),
        ),
    ]);
    let mut objs = vec![stream, page];
    for (width, height, flate) in pictures {
        objs.push(flate_image_obj(*width, *height, flate));
    }
    Ok(Graph { objs, page: 1 })
}

fn flate_image_obj(width: u32, height: u32, flate: &[u8]) -> Obj {
    Obj::Stream {
        dict: vec![
            ("Type".into(), Obj::Name("XObject".into())),
            ("Subtype".into(), Obj::Name("Image".into())),
            ("Width".into(), Obj::Int(i64::from(width))),
            ("Height".into(), Obj::Int(i64::from(height))),
            ("ColorSpace".into(), Obj::Name("DeviceRGB".into())),
            ("BitsPerComponent".into(), Obj::Int(8)),
            ("Length".into(), Obj::Int(flate.len() as i64)),
            ("Filter".into(), Obj::Name("FlateDecode".into())),
        ],
        data: flate.to_vec(),
    }
}

fn page_graph(lines: &[&Line]) -> Result<Graph> {
    let mut content = String::from("BT\n");
    let mut y = PAGE_H as f32 - MARGIN;
    for line in lines {
        let lead = if line.leading > 0.0 { line.leading } else { 14.0 };
        let size = line.spans.first().map(|s| s.size).unwrap_or(11.0);
        y -= lead;
        let baseline = y + (lead - size).max(0.0) * 0.2;
        content.push_str(&format!("1 0 0 1 {MARGIN:.2} {baseline:.2} Tm\n"));
        for span in &line.spans {
            let font = match span.font {
                1 => "F2",
                2 => "F3",
                3 => "F4",
                _ => "F1",
            };
            content.push_str(&format!("/{font} {size:.2} Tf\n", size = span.size));
            content.push_str(&pdf_string(&span.text));
            content.push_str(" Tj\n");
        }
    }
    content.push_str("ET\n");
    let data = content.into_bytes();
    let stream = Obj::Stream {
        dict: vec![("Length".into(), Obj::Int(data.len() as i64))],
        data,
    };
    let fonts = Obj::Dict(vec![
        ("F1".into(), helvetica("Helvetica")),
        ("F2".into(), helvetica("Helvetica-Bold")),
        ("F3".into(), helvetica("Helvetica-Oblique")),
        ("F4".into(), helvetica("Helvetica-BoldOblique")),
    ]);
    let page = Obj::Dict(vec![
        ("Type".into(), Obj::Name("Page".into())),
        (
            "MediaBox".into(),
            Obj::Arr(vec![
                Obj::Int(0),
                Obj::Int(0),
                Obj::Int(PAGE_W),
                Obj::Int(PAGE_H),
            ]),
        ),
        ("Contents".into(), Obj::Ref(0)),
        (
            "Resources".into(),
            Obj::Dict(vec![("Font".into(), fonts)]),
        ),
    ]);
    Ok(Graph {
        objs: vec![stream, page],
        page: 1,
    })
}

pub(crate) fn text_width(text: &str, size: f32) -> f32 {
    let mut units = 0u32;
    for ch in text.chars() {
        let cp = ch as u32;
        let w = if (32..127).contains(&cp) {
            u32::from(HELV[(cp - 32) as usize])
        } else {
            500
        };
        units += w;
    }
    units as f32 / 1000.0 * size
}

pub(crate) fn pdf_string(text: &str) -> String {
    let mut s = String::from("(");
    for ch in text.chars() {
        let b = if ch == '\n' || ch == '\r' || ch == '\t' || (32..=126).contains(&(ch as u32)) {
            ch as u8
        } else {
            b'?'
        };
        match b {
            b'(' | b')' | b'\\' => {
                s.push('\\');
                s.push(b as char);
            }
            b'\n' => s.push_str("\\n"),
            b'\r' => s.push_str("\\r"),
            b'\t' => s.push_str("\\t"),
            32..=126 => s.push(b as char),
            _ => s.push_str(&format!("\\{b:03o}")),
        }
    }
    s.push(')');
    s
}

fn content_strings(data: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < data.len() {
        if data[i] == b'(' {
            if let Some((bytes, next)) = parse_lit(data, i) {
                let after = skip_ws_idx(data, next);
                let show = data[after..].starts_with(b"Tj")
                    || data[after..].starts_with(b"'")
                    || data[after..].starts_with(b"TJ");
                if show {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str(&String::from_utf8_lossy(&bytes));
                }
                i = next;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn parse_lit(data: &[u8], start: usize) -> Option<(Vec<u8>, usize)> {
    if data.get(start) != Some(&b'(') {
        return None;
    }
    let mut i = start + 1;
    let mut out = Vec::new();
    let mut depth = 1i32;
    while i < data.len() && depth > 0 {
        let b = data[i];
        i += 1;
        if b == b'\\' {
            let e = *data.get(i)?;
            i += 1;
            match e {
                b'n' => out.push(b'\n'),
                b'r' => out.push(b'\r'),
                b't' => out.push(b'\t'),
                b'(' | b')' | b'\\' => out.push(e),
                b'0'..=b'7' => {
                    let mut v = u32::from(e - b'0');
                    for _ in 0..2 {
                        if let Some(d) = data.get(i).copied() {
                            if (b'0'..=b'7').contains(&d) {
                                v = v * 8 + u32::from(d - b'0');
                                i += 1;
                            }
                        }
                    }
                    out.push(v as u8);
                }
                other => out.push(other),
            }
        } else if b == b'(' {
            depth += 1;
            out.push(b);
        } else if b == b')' {
            depth -= 1;
            if depth > 0 {
                out.push(b);
            }
        } else {
            out.push(b);
        }
    }
    if depth != 0 {
        return None;
    }
    Some((out, i))
}

fn skip_ws_idx(data: &[u8], mut i: usize) -> usize {
    while i < data.len() && matches!(data[i], b' ' | b'\t' | b'\n' | b'\r' | 0 | 12) {
        i += 1;
    }
    i
}

fn serialize(objs: &[Obj]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let mut offsets = vec![0u64; objs.len() + 1];
    for (i, obj) in objs.iter().enumerate() {
        let num = i as u32 + 1;
        offsets[num as usize] = out.len() as u64;
        let head = format!("{num} 0 obj\n");
        out.extend_from_slice(head.as_bytes());
        write_obj(&mut out, obj);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = out.len();
    let size = objs.len() + 1;
    out.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in offsets.iter().skip(1) {
        out.extend_from_slice(format!("{off:010} {gen:05} n \n", gen = 0).as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    out
}

fn write_obj(out: &mut Vec<u8>, obj: &Obj) {
    match obj {
        Obj::Null => out.extend_from_slice(b"null"),
        Obj::Bool(true) => out.extend_from_slice(b"true"),
        Obj::Bool(false) => out.extend_from_slice(b"false"),
        Obj::Int(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Obj::Real(s) => out.extend_from_slice(s.as_bytes()),
        Obj::Name(n) => {
            out.push(b'/');
            for b in n.bytes() {
                if b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'_') {
                    out.push(b);
                } else {
                    out.extend_from_slice(format!("#{b:02X}").as_bytes());
                }
            }
        }
        Obj::Str(bytes) => {
            out.push(b'(');
            for &b in bytes {
                match b {
                    b'(' | b')' | b'\\' => {
                        out.push(b'\\');
                        out.push(b);
                    }
                    32..=126 => out.push(b),
                    _ => out.extend_from_slice(format!("\\{b:03o}").as_bytes()),
                }
            }
            out.push(b')');
        }
        Obj::Arr(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b' ');
                }
                write_obj(out, item);
            }
            out.push(b']');
        }
        Obj::Dict(items) => write_dict(out, items),
        Obj::Ref(id) => {
            out.extend_from_slice(format!("{id} 0 R").as_bytes());
        }
        Obj::Stream { dict, data } => {
            let mut dict = dict.clone();
            if let Some((_, slot)) = dict.iter_mut().find(|(k, _)| k == "Length") {
                *slot = Obj::Int(data.len() as i64);
            } else {
                dict.push(("Length".into(), Obj::Int(data.len() as i64)));
            }
            write_dict(out, &dict);
            out.extend_from_slice(b"\nstream\n");
            out.extend_from_slice(data);
            out.extend_from_slice(b"\nendstream");
        }
    }
}

fn write_dict(out: &mut Vec<u8>, items: &[(String, Obj)]) {
    out.extend_from_slice(b"<<");
    for (k, v) in items {
        out.push(b' ');
        write_obj(out, &Obj::Name(k.clone()));
        out.push(b' ');
        write_obj(out, v);
    }
    out.extend_from_slice(b" >>");
}

fn find_startxref(data: &[u8]) -> Result<u64> {
    let start = data.len().saturating_sub(8192);
    let tail = &data[start..];
    let rel = tail
        .windows(9)
        .rposition(|w| w == b"startxref")
        .ok_or_else(|| Error::new("pdf: missing startxref"))?;
    let mut i = rel + 9;
    while i < tail.len() && tail[i].is_ascii_whitespace() {
        i += 1;
    }
    let b = i;
    while i < tail.len() && tail[i].is_ascii_digit() {
        i += 1;
    }
    if b == i {
        return Err(Error::new("pdf: bad startxref"));
    }
    let n = std::str::from_utf8(&tail[b..i])
        .map_err(|_| Error::new("pdf: bad startxref"))?
        .parse::<u64>()
        .map_err(|_| Error::new("pdf: bad startxref"))?;
    Ok(n)
}

fn read_trailer_only(data: &[u8], at: u64) -> Result<Obj> {
    let mut xref = Vec::new();
    let _prev = read_xref(data, at, &mut xref, true)?;
    // read_xref returns prev and we need the trailer. Parse again for the dict.
    let mut p = Cur::new(data, at as usize);
    p.skip();
    if !p.keyword("xref") {
        return Err(Error::new("pdf: xref stream is not supported"));
    }
    loop {
        p.skip();
        if p.keyword("trailer") {
            return p.parse_value();
        }
        let _start = p.parse_int()?;
        let count = p.parse_int()?;
        if !(0..=1_000_000).contains(&count) {
            return Err(Error::new("pdf: xref count exceeds bound"));
        }
        for _ in 0..count {
            let _off = p.parse_int()?;
            let _gen = p.parse_int()?;
            p.skip();
            if p.keyword("n") || p.keyword("f") {
                continue;
            }
            return Err(Error::new("pdf: bad xref entry"));
        }
    }
}

fn read_xref(data: &[u8], at: u64, xref: &mut Vec<Option<u64>>, fill: bool) -> Result<u64> {
    let mut p = Cur::new(data, at as usize);
    p.skip();
    if !p.keyword("xref") {
        return Err(Error::new("pdf: xref stream is not supported"));
    }
    loop {
        p.skip();
        if p.starts_with("trailer") {
            break;
        }
        if p.i >= p.data.len() {
            return Err(Error::new("pdf: truncated xref"));
        }
        let start = p.parse_int()?;
        let count = p.parse_int()?;
        if start < 0 || !(0..=1_000_000).contains(&count) {
            return Err(Error::new("pdf: xref count exceeds bound"));
        }
        for n in 0..count {
            p.skip();
            let off = p.parse_int()?;
            let _gen = p.parse_int()?;
            p.skip();
            let used = if p.keyword("n") {
                true
            } else if p.keyword("f") {
                false
            } else {
                return Err(Error::new("pdf: bad xref entry"));
            };
            let id = start as usize + n as usize;
            if xref.len() <= id {
                xref.resize(id + 1, None);
            }
            if fill && xref[id].is_none() && used && off >= 0 {
                xref[id] = Some(off as u64);
            }
        }
    }
    if !p.keyword("trailer") {
        return Err(Error::new("pdf: missing trailer"));
    }
    let trailer = p.parse_value()?;
    Ok(match trailer.dict_get("Prev") {
        Some(Obj::Int(n)) if *n > 0 => *n as u64,
        _ => 0,
    })
}

struct Cur<'a> {
    data: &'a [u8],
    i: usize,
}

impl<'a> Cur<'a> {
    fn new(data: &'a [u8], i: usize) -> Self {
        Self { data, i }
    }

    fn skip(&mut self) {
        loop {
            if self.i >= self.data.len() {
                return;
            }
            let b = self.data[self.i];
            if b == b'%' {
                while self.i < self.data.len() && self.data[self.i] != b'\n' && self.data[self.i] != b'\r'
                {
                    self.i += 1;
                }
                continue;
            }
            if matches!(b, 0 | b'\t' | b'\n' | 12 | b'\r' | b' ') {
                self.i += 1;
                continue;
            }
            return;
        }
    }

    fn bump(&mut self, b: u8) -> bool {
        if self.data.get(self.i) == Some(&b) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn keyword(&mut self, word: &str) -> bool {
        self.skip();
        let b = word.as_bytes();
        if self.data[self.i..].starts_with(b) {
            let end = self.i + b.len();
            if end == self.data.len() || is_delim(self.data[end]) {
                self.i = end;
                return true;
            }
        }
        false
    }

    fn starts_with(&self, word: &str) -> bool {
        let mut i = self.i;
        while i < self.data.len() && matches!(self.data[i], 0 | b'\t' | b'\n' | 12 | b'\r' | b' ') {
            i += 1;
        }
        self.data[i..].starts_with(word.as_bytes())
    }

    fn parse_int(&mut self) -> Result<i64> {
        self.skip();
        let start = self.i;
        if self.i < self.data.len() && (self.data[self.i] == b'+' || self.data[self.i] == b'-') {
            self.i += 1;
        }
        let digits = self.i;
        while self.i < self.data.len() && self.data[self.i].is_ascii_digit() {
            self.i += 1;
        }
        if self.i == digits {
            return Err(Error::new("pdf: expected integer"));
        }
        std::str::from_utf8(&self.data[start..self.i])
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| Error::new("pdf: bad integer"))
    }

    fn parse_value(&mut self) -> Result<Obj> {
        self.skip();
        if self.i >= self.data.len() {
            return Err(Error::new("pdf: truncated object"));
        }
        let b = self.data[self.i];
        match b {
            b'n' if self.keyword("null") => Ok(Obj::Null),
            b't' if self.keyword("true") => Ok(Obj::Bool(true)),
            b'f' if self.keyword("false") => Ok(Obj::Bool(false)),
            b'/' => Ok(Obj::Name(self.parse_name()?)),
            b'(' => Ok(Obj::Str(self.parse_string()?)),
            b'<' => {
                if self.data.get(self.i + 1) == Some(&b'<') {
                    self.parse_dict()
                } else {
                    self.parse_hex()
                }
            }
            b'[' => self.parse_array(),
            b'+' | b'-' | b'.' | b'0'..=b'9' => self.parse_number_or_ref(),
            _ => Err(Error::new("pdf: unexpected byte in object")),
        }
    }

    fn parse_name(&mut self) -> Result<String> {
        if self.data.get(self.i) != Some(&b'/') {
            return Err(Error::new("pdf: expected name"));
        }
        self.i += 1;
        let mut s = String::new();
        while self.i < self.data.len() && !is_delim(self.data[self.i]) {
            let b = self.data[self.i];
            self.i += 1;
            if b == b'#' {
                let h1 = self.data.get(self.i).copied().ok_or_else(|| Error::new("pdf: bad name"))?;
                let h2 = self
                    .data
                    .get(self.i + 1)
                    .copied()
                    .ok_or_else(|| Error::new("pdf: bad name"))?;
                self.i += 2;
                let v = hex_byte(h1, h2).ok_or_else(|| Error::new("pdf: bad name hex"))?;
                s.push(v as char);
            } else {
                s.push(b as char);
            }
        }
        Ok(s)
    }

    fn parse_string(&mut self) -> Result<Vec<u8>> {
        let (bytes, next) = parse_lit(self.data, self.i).ok_or_else(|| Error::new("pdf: bad string"))?;
        self.i = next;
        Ok(bytes)
    }

    fn parse_hex(&mut self) -> Result<Obj> {
        if self.data.get(self.i) != Some(&b'<') {
            return Err(Error::new("pdf: expected hex string"));
        }
        self.i += 1;
        let mut hexits = Vec::new();
        while self.i < self.data.len() && self.data[self.i] != b'>' {
            let b = self.data[self.i];
            self.i += 1;
            if b.is_ascii_whitespace() {
                continue;
            }
            hexits.push(b);
        }
        if self.data.get(self.i) != Some(&b'>') {
            return Err(Error::new("pdf: truncated hex string"));
        }
        self.i += 1;
        if hexits.len() % 2 == 1 {
            hexits.push(b'0');
        }
        let mut out = Vec::with_capacity(hexits.len() / 2);
        for pair in hexits.chunks(2) {
            let v = hex_byte(pair[0], pair[1]).ok_or_else(|| Error::new("pdf: bad hex string"))?;
            out.push(v);
        }
        Ok(Obj::Str(out))
    }

    fn parse_array(&mut self) -> Result<Obj> {
        if self.data.get(self.i) != Some(&b'[') {
            return Err(Error::new("pdf: expected array"));
        }
        self.i += 1;
        let mut items = Vec::new();
        loop {
            self.skip();
            if self.i >= self.data.len() {
                return Err(Error::new("pdf: truncated array"));
            }
            if self.data[self.i] == b']' {
                self.i += 1;
                break;
            }
            items.push(self.parse_value()?);
        }
        Ok(Obj::Arr(items))
    }

    fn parse_dict(&mut self) -> Result<Obj> {
        if self.data.get(self.i..self.i + 2) != Some(b"<<") {
            return Err(Error::new("pdf: expected dictionary"));
        }
        self.i += 2;
        let mut items = Vec::new();
        loop {
            self.skip();
            if self.data.get(self.i..self.i + 2) == Some(b">>") {
                self.i += 2;
                break;
            }
            if self.i >= self.data.len() {
                return Err(Error::new("pdf: truncated dictionary"));
            }
            let key = self.parse_name()?;
            let val = self.parse_value()?;
            items.push((key, val));
        }
        Ok(Obj::Dict(items))
    }

    fn parse_number_or_ref(&mut self) -> Result<Obj> {
        let start = self.i;
        let first = self.scan_number_token()?;
        let after_first = self.i;
        self.skip();
        if self.i < self.data.len() && is_num_start(self.data[self.i]) {
            let saved = self.i;
            if self.scan_number_token().is_ok() {
                let after_second = self.i;
                self.skip();
                if self.keyword("R") {
                    let id = int_token(&self.data[start..after_first])?;
                    if id < 0 {
                        return Err(Error::new("pdf: negative object id"));
                    }
                    return Ok(Obj::Ref(id as u32));
                }
                self.i = saved;
                let _ = after_second;
            }
        }
        self.i = after_first;
        Ok(first)
    }

    fn scan_number_token(&mut self) -> Result<Obj> {
        self.skip();
        let start = self.i;
        if self.i < self.data.len() && (self.data[self.i] == b'+' || self.data[self.i] == b'-') {
            self.i += 1;
        }
        let mut dot = false;
        let mut digits = 0;
        while self.i < self.data.len() {
            let b = self.data[self.i];
            if b.is_ascii_digit() {
                digits += 1;
                self.i += 1;
            } else if b == b'.' && !dot {
                dot = true;
                self.i += 1;
            } else {
                break;
            }
        }
        if digits == 0 {
            return Err(Error::new("pdf: expected number"));
        }
        let tok = std::str::from_utf8(&self.data[start..self.i]).unwrap_or("");
        if dot {
            Ok(Obj::Real(tok.to_string()))
        } else {
            let n = tok.parse::<i64>().map_err(|_| Error::new("pdf: bad integer"))?;
            Ok(Obj::Int(n))
        }
    }
}

fn int_token(tok: &[u8]) -> Result<i64> {
    std::str::from_utf8(tok)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error::new("pdf: bad object id"))
}

fn is_num_start(b: u8) -> bool {
    b.is_ascii_digit() || b == b'+' || b == b'-' || b == b'.'
}

fn is_delim(b: u8) -> bool {
    matches!(
        b,
        0 | b'\t' | b'\n' | 12 | b'\r' | b' ' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn hex_byte(a: u8, b: u8) -> Option<u8> {
    Some(hex_val(a)? << 4 | hex_val(b)?)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

const HELV: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let lines = vec![
            Line {
                spans: vec![Span {
                    text: "ALPHA page".into(),
                    font: 0,
                    size: 11.0,
                }],
                leading: 14.0,
            },
            Line {
                spans: vec![Span {
                    text: "second line".into(),
                    font: 1,
                    size: 11.0,
                }],
                leading: 14.0,
            },
        ];
        from_lines(&lines).unwrap()
    }

    #[test]
    fn roundtrip_text_and_xref() {
        let pdf = sample();
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf.windows(9).any(|w| w == b"startxref"));
        assert!(pdf.windows(5).any(|w| w == b"%%EOF"));
        assert_eq!(page_count(&pdf).unwrap(), 1);
        let text = plain_text(&pdf).unwrap();
        assert!(text.contains("ALPHA page"), "{text}");
        assert!(text.contains("second line"), "{text}");
        let pages = split(&pdf).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(page_count(&pages[0]).unwrap(), 1);
    }

    #[test]
    fn split_merge_two_pages() {
        let mut lines = Vec::new();
        lines.push(Line {
            spans: vec![Span {
                text: "PAGEONEWORD".into(),
                font: 0,
                size: 12.0,
            }],
            leading: 16.0,
        });
        for i in 0..60 {
            lines.push(Line {
                spans: vec![Span {
                    text: format!("filler {i}"),
                    font: 0,
                    size: 11.0,
                }],
                leading: 14.0,
            });
        }
        lines.push(Line {
            spans: vec![Span {
                text: "PAGETWOWORD".into(),
                font: 0,
                size: 12.0,
            }],
            leading: 16.0,
        });
        let pdf = from_lines(&lines).unwrap();
        assert!(page_count(&pdf).unwrap() >= 2);
        let parts = split(&pdf).unwrap();
        assert!(parts.len() >= 2);
        assert_eq!(page_count(&parts[0]).unwrap(), 1);
        let t0 = plain_text(&parts[0]).unwrap();
        let t1 = plain_text(&parts[1]).unwrap();
        assert!(t0.contains("PAGEONEWORD"), "{t0}");
        assert!(!t0.contains("PAGETWOWORD"));
        assert!(t1.contains("PAGETWOWORD") || plain_text(&parts[parts.len() - 1]).unwrap().contains("PAGETWOWORD"));
        let merged = merge(&[&parts[0], &parts[1]]).unwrap();
        assert_eq!(page_count(&merged).unwrap(), 2);
        let both = plain_text(&merged).unwrap();
        assert!(both.contains("PAGEONEWORD") && both.contains("second line") || both.contains("filler") || both.contains("PAGETWOWORD"));
    }

    #[test]
    fn rotate_and_watermark() {
        let pdf = sample();
        let turned = rotate(&pdf, 90).unwrap();
        assert!(turned.windows(10).any(|w| w == b"/Rotate 90") || turned.windows(11).any(|w| w == b"/Rotate 90"));
        assert_eq!(page_count(&turned).unwrap(), 1);
        let marked = watermark(&pdf, "SECRET").unwrap();
        let text = plain_text(&marked).unwrap();
        assert!(text.contains("SECRET"), "{text}");
        assert!(text.contains("ALPHA"));
    }

    #[test]
    fn corrupt_is_an_error() {
        assert!(page_count(b"not a pdf").is_err());
        assert!(page_count(b"%PDF-1.4\n").is_err());
    }

    #[test]
    fn shrink_flate_roundtrip_is_smaller() {
        let mut lines = Vec::new();
        lines.push(Line {
            spans: vec![Span {
                text: "PAGEONEWORD".into(),
                font: 0,
                size: 12.0,
            }],
            leading: 14.0,
        });
        for _ in 0..80 {
            lines.push(Line {
                spans: vec![Span {
                    text: "alpha beta gamma delta".into(),
                    font: 0,
                    size: 11.0,
                }],
                leading: 14.0,
            });
        }
        let raw = from_lines(&lines).unwrap();
        let missed = shrink(&raw).unwrap_err();
        assert!(missed.message.starts_with("MISS:"), "{missed}");
        assert!(
            missed.message.contains(&format!("input_b={}", raw.len())),
            "{missed}"
        );
        let flate = flate_fixture(&raw).unwrap();
        assert!(flate.windows(11).any(|w| w == b"FlateDecode"));
        let shrunk = shrink(&flate).unwrap();
        assert!(shrunk.len() < flate.len(), "{} -> {}", flate.len(), shrunk.len());
        assert_eq!(flate_plains(&flate).unwrap(), flate_plains(&shrunk).unwrap());
        assert_eq!(plain_text(&shrunk).unwrap(), plain_text(&raw).unwrap());
        assert!(plain_text(&shrunk).unwrap().contains("PAGEONEWORD"));
    }

    #[test]
    fn shrink_markdown_flate_fixture_stays_37246_to_2659() {
        let mut md = String::from("# Notes\n\nA **word** and a list.\n\n- one\n- two\n\n");
        for i in 0..80 {
            md.push_str(&format!(
                "Paragraph {i} of the markdown fixture stays in the stream.\n\n"
            ));
        }
        let raw = crate::convert::convert("md", "pdf", md.as_bytes()).unwrap();
        assert_eq!(raw.len(), 37_200);
        let missed = shrink(&raw).unwrap_err();
        assert!(missed.message.starts_with("MISS:"), "{missed}");
        assert!(missed.message.contains("input_b=37200"), "{missed}");
        let flate = flate_fixture(&raw).unwrap();
        assert_eq!(flate.len(), 37_246);
        let shrunk = shrink(&flate).unwrap();
        assert_eq!(shrunk.len(), 2_659);
        assert!(shrunk.len() < flate.len());
        assert!(flate_plain_matches(&flate, &shrunk).unwrap());
        assert_eq!(flate_plains(&flate).unwrap(), flate_plains(&shrunk).unwrap());
        let text = plain_text(&shrunk).unwrap();
        assert_eq!(plain_text(&raw).unwrap(), text);
        assert!(text.contains("Paragraph"), "{text}");
    }
}
