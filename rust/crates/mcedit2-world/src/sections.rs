//! Read-only 1.13+ Java NBT Section, Palette and block-state decoding.
//! Includes 1.18+ signed negative Section Y and 2026 26.x DataVersion layouts.
//! Never rewrites unknown Block Entity or other original NBT bytes.
use std::collections::BTreeSet;
use std::fmt;
use serde::{Deserialize, Serialize};
use sha2::{Digest,Sha256};

const MAX_DEPTH: u8 = 64;
const MAX_NODES: usize = 1_000_000;
const MAX_SECTIONS: usize = 128;
const MAX_PALETTE: usize = 4096;
const BLOCKS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State { pub name: String, pub properties: Vec<(String, String)> }
#[derive(Debug, Clone)]
pub struct Section { pub y: i8, pub palette: Vec<State>, pub indices: Vec<u16> }
impl Section {
    pub fn block(&self, x: u8, y: u8, z: u8) -> Option<&State> {
        if x >= 16 || y >= 16 || z >= 16 { return None; }
        self.palette.get(*self.indices.get((y as usize * 16 + z as usize) * 16 + x as usize)? as usize)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layout { McRegion128, NumericAnvil112, Named113, Modern118 }
#[derive(Debug, Clone)]
pub struct Chunk {
    pub data_version: Option<i32>,
    pub x: Option<i32>, pub z: Option<i32>,
    pub layout: Layout, pub sections: Vec<Section>,
    /// Original Block Entity positions. Opaque NBT is never rewritten.
    pub block_entities: BTreeSet<(i32, i32, i32)>,
}
impl Chunk {
    pub fn has_block_entity(&self, x: i32, y: i32, z: i32) -> bool {
        self.block_entities.contains(&(x, y, z))
    }
    pub fn block(&self, x: u8, y: i32, z: u8) -> Option<&State> {
        let sy = i8::try_from(y.div_euclid(16)).ok()?;
        self.sections.iter().find(|v| v.y == sy)?.block(x, y.rem_euclid(16) as u8, z)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(pub String);
impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}
impl std::error::Error for DecodeError {}
type Result<T> = std::result::Result<T, DecodeError>;
fn error(s: impl Into<String>) -> DecodeError { DecodeError(s.into()) }

#[derive(Debug)]
pub(super) struct Tag<'a> {
    pub(super) kind: u8, pub(super) raw: &'a [u8],
    pub(super) fields: Vec<(String, Tag<'a>)>, pub(super) items: Vec<Tag<'a>>,
}
impl Tag<'_> {
    pub(super) fn field(&self, key: &str) -> Option<&Self> {
        self.fields.iter().find(|(name, _)| name == key).map(|(_, tag)| tag)
    }
    pub(super) fn require(&self, key: &str, kind: u8) -> Result<&Self> {
        match self.field(key) {
            Some(v) if v.kind == kind => Ok(v),
            _ => Err(error(format!("Missing or mismatched original tag {key}"))),
        }
    }
    pub(super) fn i32(&self) -> Result<i32> {
        if self.kind != 3 || self.raw.len() != 4 { return Err(error("Expected TAG_Int")); }
        Ok(i32::from_be_bytes(self.raw.try_into().unwrap()))
    }
    pub(super) fn byte(&self) -> Result<i8> {
        if self.kind != 1 || self.raw.len() != 1 { return Err(error("Expected TAG_Byte")); }
        Ok(self.raw[0] as i8)
    }
    pub(super) fn string(&self) -> Result<String> {
        if self.kind != 8 || self.raw.len() < 2 { return Err(error("Expected TAG_String")); }
        let n = u16::from_be_bytes(self.raw[0..2].try_into().unwrap()) as usize;
        if n + 2 != self.raw.len() { return Err(error("Invalid original NBT string size")); }
        std::str::from_utf8(&self.raw[2..]).map(str::to_owned)
            .map_err(|_| error("Invalid original NBT UTF-8"))
    }
    fn longs(&self) -> Result<Vec<u64>> {
        if self.kind != 12 || self.raw.len() < 4 { return Err(error("Expected TAG_Long_Array")); }
        let count = i32::from_be_bytes(self.raw[0..4].try_into().unwrap());
        if count < 0 || count > 65_536 || self.raw.len() != 4 + count as usize * 8 {
            return Err(error("Invalid original BlockStates long-array length"));
        }
        Ok(self.raw[4..].chunks_exact(8).map(|s| u64::from_be_bytes(s.try_into().unwrap())).collect())
    }
}
struct Reader<'a> { buf: &'a [u8], at: usize, nodes: usize }
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or_else(|| error("NBT offset overflow"))?;
        let slice = self.buf.get(self.at..end).ok_or_else(|| error("NBT is truncated"))?;
        self.at = end; Ok(slice)
    }
    fn byte(&mut self) -> Result<u8> { Ok(self.take(1)?[0]) }
    fn name(&mut self) -> Result<String> {
        let n = u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as usize;
        std::str::from_utf8(self.take(n)?).map(str::to_owned)
            .map_err(|_| error("NBT name is not UTF-8"))
    }
    fn count(&mut self, max: usize) -> Result<usize> {
        let n = i32::from_be_bytes(self.take(4)?.try_into().unwrap());
        if n < 0 || n as usize > max { return Err(error("NBT collection count exceeds limit")); }
        Ok(n as usize)
    }
    fn node(&mut self, kind: u8, depth: u8) -> Result<Tag<'a>> {
        if depth > MAX_DEPTH || self.nodes >= MAX_NODES {
            return Err(error("NBT traversal budget exceeded"));
        }
        self.nodes += 1;
        let start = self.at;
        let mut fields = Vec::new();
        let mut items = Vec::new();
        match kind {
            1 => { self.take(1)?; }
            2 => { self.take(2)?; }
            3 | 5 => { self.take(4)?; }
            4 | 6 => { self.take(8)?; }
            7 => { let n = self.count(self.buf.len())?; self.take(n)?; }
            8 => { self.name()?; }
            9 => {
                let inner = self.byte()?;
                let n = self.count(MAX_NODES)?;
                if inner > 12 || (inner == 0 && n != 0) ||
                    self.nodes.checked_add(n).is_none_or(|total| total > MAX_NODES) {
                    return Err(error("Invalid original List schema/size"));
                }
                for _ in 0..n { items.push(self.node(inner, depth + 1)?); }
            }
            10 => loop {
                let next = self.byte()?;
                if next == 0 { break; }
                if next > 12 { return Err(error("Invalid original Compound kind")); }
                let name = self.name()?;
                if fields.iter().any(|(s, _)| s == &name) {
                    return Err(error("Ambiguous duplicate original NBT field"));
                }
                fields.push((name, self.node(next, depth + 1)?));
            },
            11 | 12 => {
                let n = self.count(self.buf.len())?;
                let bytes = n.checked_mul(if kind == 11 {4} else {8})
                    .ok_or_else(|| error("Packed NBT array overflow"))?;
                self.take(bytes)?;
            }
            _ => return Err(error("Unrecognized original NBT tag type")),
        }
        Ok(Tag { kind, raw: &self.buf[start..self.at], fields, items })
    }
}
pub(crate) fn parse_root(data: &[u8]) -> Result<Tag<'_>> {
    if data.len() > 64 * 1024 * 1024 { return Err(error("NBT exceeds 64 MiB")); }
    let mut r = Reader { buf: data, at: 0, nodes: 0 };
    if r.byte()? != 10 { return Err(error("Original root is not a Compound")); }
    r.name()?;
    let root = r.node(10, 0)?;
    if r.at != data.len() { return Err(error("Trailing bytes after NBT root")); }
    Ok(root)
}

/// Exact 4096 original indices, both cross-long pre-1.16 style and
/// non-crossing padded style; validates every index against actual palette.
pub fn unpack(palette_len: usize, words: &[u64]) -> Result<Vec<u16>> {
    if !(1..=MAX_PALETTE).contains(&palette_len) { return Err(error("Invalid palette size")); }
    if palette_len == 1 {
        if !words.is_empty() { return Err(error("Single-state palette with unexpected data")); }
        return Ok(vec![0; BLOCKS]);
    }
    let bits = (usize::BITS - (palette_len - 1).leading_zeros()) as usize;
    let bits = bits.max(4);
    let per_word = 64 / bits;
    let padded_len = BLOCKS.div_ceil(per_word);
    let crossing_len = (BLOCKS * bits).div_ceil(64);
    let padded = if words.len() == padded_len { true }
        else if words.len() == crossing_len { false }
        else { return Err(error("Incorrect original packed block-state count")); };
    let mask = (1u64 << bits) - 1;
    let mut out = Vec::with_capacity(BLOCKS);
    for n in 0..BLOCKS {
        let value = if padded {
            (words[n / per_word] >> ((n % per_word) * bits)) & mask
        } else {
            let bit = n * bits;
            let word = bit / 64;
            let offset = bit % 64;
            let mut v = words[word] >> offset;
            if offset + bits > 64 { v |= words[word + 1] << (64 - offset); }
            v & mask
        };
        if value as usize >= palette_len { return Err(error("Block index outside palette")); }
        out.push(value as u16);
    }
    Ok(out)
}
fn block_state(tag: &Tag<'_>) -> Result<State> {
    if tag.kind != 10 { return Err(error("Original palette member is not Compound")); }
    let name = tag.require("Name", 8)?.string()?;
    if name.is_empty() || name.len() > 512 { return Err(error("Invalid block state Name")); }
    let mut properties = Vec::new();
    if let Some(p) = tag.field("Properties") {
        if p.kind != 10 || p.fields.len() > 256 {
            return Err(error("Invalid original block properties"));
        }
        for (key, value) in &p.fields { properties.push((key.clone(), value.string()?)); }
    }
    properties.sort_unstable();
    Ok(State { name, properties })
}

/// Read only the original absolute Block Entity coordinates. Do not normalize
/// IDs, invented fields, item contents, mod-specific data or arbitrary NBT.
/// A malformed original list cannot be treated as an empty, editable list.
pub(super) fn original_block_entities(
    source: &Tag<'_>, modern: bool,
) -> Result<BTreeSet<(i32, i32, i32)>> {
    let key = if modern { "block_entities" } else { "TileEntities" };
    let Some(list) = source.field(key) else { return Ok(BTreeSet::new()); };
    if list.kind != 9 {
        return Err(error(format!("Original {key} is not an NBT List")));
    }
    let mut positions = BTreeSet::new();
    for item in &list.items {
        if item.kind != 10 {
            return Err(error(format!("Original {key} member is not a Compound")));
        }
        let position = (
            item.require("x", 3)?.i32()?,
            item.require("y", 3)?.i32()?,
            item.require("z", 3)?.i32()?,
        );
        if !positions.insert(position) {
            return Err(error("Duplicate original Block Entity XYZ; edit unsafe"));
        }
    }
    Ok(positions)
}

pub fn decode_sections(raw: &[u8]) -> Result<Chunk> {
    let root = parse_root(raw)?;
    let modern = root.field("sections").is_some();
    let source = if modern { &root } else { root.require("Level", 10)? };
    if !modern && source.field("Blocks").is_some() {
        return super::oldnumeric::decode_mcregion(&root, source);
    }
    let block_entities = original_block_entities(source, modern)?;
    let list = source.require(if modern {"sections"} else {"Sections"}, 9)?;
    if !modern && list.items.iter().any(|s| s.field("Blocks").is_some()) {
        if !list.items.iter().all(|s| s.kind == 10 && s.field("Blocks").is_some()) {
            return Err(error("Mixed named and numeric Section formats are unsafe"));
        }
        return super::oldnumeric::decode_anvil(&root, source, list);
    }
    if list.items.len() > MAX_SECTIONS { return Err(error("Too many original sections")); }
    let mut sections: Vec<Section> = Vec::new();
    for entry in &list.items {
        if entry.kind != 10 { return Err(error("Non-Compound original Section")); }
        let y = entry.require("Y", 1)?.byte()?;
        let states = if modern {
            match entry.field("block_states") {
                None => continue, // light-only entry
                Some(s) if s.kind == 10 => s,
                Some(_) => return Err(error("Invalid modern block_states Compound")),
            }
        } else { entry };
        let palette = states.require(if modern {"palette"} else {"Palette"}, 9)?;
        if palette.items.is_empty() || palette.items.len() > MAX_PALETTE {
            return Err(error("Invalid Section palette length"));
        }
        let palette: Vec<State> = palette.items.iter().map(block_state).collect::<Result<_>>()?;
        let packed = states.field(if modern {"data"} else {"BlockStates"});
        let longs = match packed {
            Some(p) => p.longs()?,
            None if palette.len() == 1 => vec![],
            None => return Err(error("Multi-state palette without original data")),
        };
        let indices = unpack(palette.len(), &longs)?;
        if sections.iter().any(|s| s.y == y) {
            return Err(error("Duplicate signed Section Y in chunk"));
        }
        sections.push(Section { y, palette, indices });
    }
    sections.sort_by_key(|s| s.y);
    Ok(Chunk {
        data_version: root.field("DataVersion").map(Tag::i32).transpose()?,
        x: source.field("xPos").map(Tag::i32).transpose()?,
        z: source.field("zPos").map(Tag::i32).transpose()?,
        layout: if modern { Layout::Modern118 } else { Layout::Named113 },
        sections, block_entities,
    })
}


/// One shallow, unmodified original NBT field summary. Compound/list
/// payloads are NOT normalized, decoded into guessed classes or rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginalNbtField {
    pub name:String,
    pub kind:u8,
    pub payload_bytes:usize,
    pub direct_children:usize,
}
/// Original opaque Java Block Entity contents available for safe inspection.
/// No value is automatically converted to the modern registry, and full
/// source NBT remains unchanged on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginalBlockEntity {
    pub x:i32,
    pub y:i32,
    pub z:i32,
    pub raw_id:Option<String>,
    pub original_compound_sha256:String,
    pub original_compound_bytes:usize,
    pub original_field_count:usize,
    pub fields:Vec<OriginalNbtField>,
}
impl OriginalBlockEntity {
    pub fn report(&self)->String {
        let id=self.raw_id.as_deref().unwrap_or("<original ID absent>");
        let mut s=format!(
            "Original Block Entity ({},{},{}), id={}, {} byte NBT Compound, SHA-256 {}, {} direct fields (read-only)\n",
            self.x,self.y,self.z,id,self.original_compound_bytes,
            self.original_compound_sha256,self.original_field_count
        );
        for field in &self.fields {
            s.push_str(&format!(
                "  {}: NBT type {}, {} payload bytes, {} immediate children\n",
                field.name,field.kind,field.payload_bytes,field.direct_children
            ));
        }
        if self.fields.len()!=self.original_field_count {
            s.push_str("  ... further fields omitted from display, original NBT retained ...\n");
        }
        s
    }
}
/// Inspect a single BE without cloning or rewriting its opaque raw NBT.
/// Absence is distinguishable from malformed source (which is refused).
pub fn inspect_original_block_entity(
    original_nbt:&[u8],position:(i32,i32,i32)
)->Result<Option<OriginalBlockEntity>>{
    let chunk=decode_sections(original_nbt)?;
    if !chunk.block_entities.contains(&position) {return Ok(None);}
    if chunk.x.is_some_and(|cx|position.0.div_euclid(16)!=cx)
        ||chunk.z.is_some_and(|cz|position.2.div_euclid(16)!=cz){
        return Err(error("Original Block Entity coordinates mismatch chunk owner"));
    }
    let root=parse_root(original_nbt)?;
    let modern=root.field("sections").is_some();
    let source=if modern {&root}else{root.require("Level",10)?};
    let list=source.require(if modern{"block_entities"}else{"TileEntities"},9)?;
    let original=list.items.iter().find(|tag|{
        tag.kind==10
        &&tag.field("x").and_then(|v|v.i32().ok())==Some(position.0)
        &&tag.field("y").and_then(|v|v.i32().ok())==Some(position.1)
        &&tag.field("z").and_then(|v|v.i32().ok())==Some(position.2)
    }).ok_or_else(||error("Block Entity disappeared from source NBT"))?;
    let raw_id=original.field("id").map(|tag|tag.string()).transpose()?;
    let original_compound_sha256=Sha256::digest(original.raw).iter()
        .map(|byte|format!("{byte:02x}")).collect::<String>();
    const MAX_FIELDS:usize=96;
    let fields=original.fields.iter().take(MAX_FIELDS).map(|(name,tag)|
        OriginalNbtField {
            name:name.clone(),kind:tag.kind,payload_bytes:tag.raw.len(),
            direct_children:if tag.kind==10{tag.fields.len()}
                else if tag.kind==9{tag.items.len()}else{0},
        }
    ).collect();
    Ok(Some(OriginalBlockEntity {
        x:position.0,y:position.1,z:position.2,
        raw_id,original_compound_sha256,
        original_compound_bytes:original.raw.len(),
        original_field_count:original.fields.len(),fields,
    }))
}


/// An original bounded nested NBT path. Slash-delimited path fields are
/// informational; only a scalar path through real Compound ancestors can
/// be passed to the surgical editor. No NBT bytes are normalized.
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct OriginalBePath{
    pub path:String,
    pub kind:u8,
    pub payload_bytes:usize,
    pub immediate_children:usize,
    pub safely_editable_scalar:bool,
}
fn flatten_original_be(
    node:&Tag<'_>,prefix:&str,depth:usize,
    safe_parent:bool,out:&mut Vec<OriginalBePath>,
){
    for (key,child) in &node.fields{
        if out.len()>=256{return;}
        let clean=!key.is_empty()&&!key.contains('/')
            &&key.len()<=64
            &&!matches!(key.as_str(),"x"|"y"|"z"|"id"|"keepPacked");
        let name=if prefix.is_empty(){key.clone()}
            else{format!("{prefix}/{key}")};
        let editable=safe_parent&&clean&&depth<8
            &&matches!(child.kind,1|2|3|4|5|6|8);
        out.push(OriginalBePath{
            path:name.clone(),kind:child.kind,
            payload_bytes:child.raw.len(),
            immediate_children:if child.kind==10{child.fields.len()}
                else if child.kind==9{child.items.len()}else{0},
            safely_editable_scalar:editable,
        });
        if child.kind==10 && depth<7{
            flatten_original_be(child,&name,depth+1,safe_parent&&clean,out);
        }
    }
}
pub fn inspect_original_be_tree(
    original_nbt:&[u8],position:(i32,i32,i32)
)->Result<Vec<OriginalBePath>>{
    if inspect_original_block_entity(original_nbt,position)?.is_none(){
        return Ok(Vec::new());
    }
    let root=parse_root(original_nbt)?;
    let modern=root.field("sections").is_some();
    let source=if modern{&root}else{root.require("Level",10)?};
    let list=source.require(if modern{"block_entities"}else{"TileEntities"},9)?;
    let entity=list.items.iter().find(|t|
        t.kind==10
        &&t.field("x").and_then(|x|x.i32().ok())==Some(position.0)
        &&t.field("y").and_then(|y|y.i32().ok())==Some(position.1)
        &&t.field("z").and_then(|z|z.i32().ok())==Some(position.2)
    ).ok_or_else(||error("BE field tree position is inconsistent"))?;
    let mut out=Vec::new();
    flatten_original_be(entity,"",0,true,&mut out);
    Ok(out)
}

/// Read-only prototype for precisely editing one existing scalar TAG payload
/// inside a single original Block Entity. This NEVER saves a world file.
/// Unknown siblings, other entities, blocks, registry IDs and nested NBT
/// remain exact original bytes. No deletions, additions or type changes.
/// Every ancestor must be a real Compound in the original source. Only
/// the final leaf is replaceable; all sibling names, kinds and raw payloads
/// must survive byte-for-byte. Lists and arrays are deliberately opaque.
fn verify_be_scalar_path<'a>(
    original:&Tag<'a>,replaced:&Tag<'_>,path:&[&str],bytes:&[u8],
)->Result<()>{
    if original.kind!=10||replaced.kind!=10
        ||original.fields.len()!=replaced.fields.len()||path.is_empty(){
        return Err(error("Nested BE edit changed Compound schema"));
    }
    for ((name,old),(new_name,new)) in
        original.fields.iter().zip(&replaced.fields){
        if name!=new_name||old.kind!=new.kind{
            return Err(error("Nested BE edit changed original field identity"));
        }
        if name==path[0]{
            if path.len()==1{
                if new.raw!=bytes{
                    return Err(error("Targeted BE scalar bytes not found after replay"));
                }
            }else{
                verify_be_scalar_path(old,new,&path[1..],bytes)?;
            }
        }else if old.raw!=new.raw{
            return Err(error("Nested BE edit touched an unselected field"));
        }
    }
    if original.field(path[0]).is_none(){
        return Err(error("Nested BE path disappeared during replay"));
    }
    Ok(())
}

pub fn patch_original_block_entity_scalar(
    original_nbt:&[u8],position:(i32,i32,i32),field:&str,text:&str,
)->Result<Vec<u8>>{
    if field.is_empty()||field.len()>128||text.len()>4096{
        return Err(error("Block Entity field/value exceeds preview safety budget"));
    }
    let path=field.split('/').collect::<Vec<_>>();
    if path.is_empty()||path.len()>8||path.iter().any(|segment|
        segment.is_empty()||segment.len()>64
        ||matches!(*segment,"x"|"y"|"z"|"id"|"keepPacked")
    ){
        return Err(error("Unsafe or excessively deep original BE scalar field path"));
    }
    let before=decode_sections(original_nbt)?;
    if !before.block_entities.contains(&position){
        return Err(error("No original Block Entity at selected XYZ"));
    }
    let root=parse_root(original_nbt)?;
    let modern=root.field("sections").is_some();
    let source=if modern{&root}else{root.require("Level",10)?};
    let list=source.require(if modern{"block_entities"}else{"TileEntities"},9)?;
    let entity=list.items.iter().find(|item|{
        item.field("x").and_then(|t|t.i32().ok())==Some(position.0)
        &&item.field("y").and_then(|t|t.i32().ok())==Some(position.1)
        &&item.field("z").and_then(|t|t.i32().ok())==Some(position.2)
    }).ok_or_else(||error("Original Block Entity missing from compound List"))?;
    let mut parent=entity;
    for segment in &path[..path.len()-1]{
        parent=parent.require(segment,10)?;
    }
    let old=parent.field(path[path.len()-1])
        .ok_or_else(||error("Field not present in original Block Entity"))?;
    let replacement=match old.kind{
        1=>vec![text.parse::<i8>().map_err(|_|error("Expected signed TAG_Byte"))? as u8],
        2=>text.parse::<i16>().map_err(|_|error("Expected signed TAG_Short"))?
            .to_be_bytes().to_vec(),
        3=>text.parse::<i32>().map_err(|_|error("Expected signed TAG_Int"))?
            .to_be_bytes().to_vec(),
        4=>text.parse::<i64>().map_err(|_|error("Expected signed TAG_Long"))?
            .to_be_bytes().to_vec(),
        5=>{
            let number=text.parse::<f32>()
                .map_err(|_|error("Expected TAG_Float numeric value"))?;
            if !number.is_finite(){return Err(error("Non-finite NBT Float rejected"));}
            number.to_bits().to_be_bytes().to_vec()
        },
        6=>{
            let number=text.parse::<f64>()
                .map_err(|_|error("Expected TAG_Double numeric value"))?;
            if !number.is_finite(){return Err(error("Non-finite NBT Double rejected"));}
            number.to_bits().to_be_bytes().to_vec()
        },
        8=>{
            let size=u16::try_from(text.len())
                .map_err(|_|error("NBT UTF-8 string too long"))?;
            let mut bytes=size.to_be_bytes().to_vec();
            bytes.extend(text.as_bytes());bytes
        },
        _=>return Err(error("Only original numeric or String scalar tags can be preview-edited")),
    };
    if old.raw==replacement {
        return Err(error("Scalar preview does not change original data"));
    }
    let (start,end)=payload_span(original_nbt,old.raw)?;
    let mut patched=Vec::with_capacity(original_nbt.len() + replacement.len());
    patched.extend_from_slice(&original_nbt[..start]);
    patched.extend_from_slice(&replacement);
    patched.extend_from_slice(&original_nbt[end..]);
    if patched.len()>64*1024*1024{
        return Err(error("Scalar NBT preview exceeds 64 MiB"));
    }
    let after=decode_sections(&patched)?;
    if before.x!=after.x||before.z!=after.z
        ||before.data_version!=after.data_version
        ||before.layout!=after.layout||before.block_entities!=after.block_entities
        ||before.sections.len()!=after.sections.len(){
        return Err(error("Original chunk structure altered by BE scalar surgery"));
    }
    for section in &before.sections {
        let updated=after.sections.iter().find(|s|s.y==section.y)
            .ok_or_else(||error("Section disappeared after scalar patch"))?;
        if updated.indices!=section.indices||updated.palette!=section.palette{
            return Err(error("BlockStates unexpectedly changed during scalar edit"));
        }
    }
    let new_root=parse_root(&patched)?;
    let new_source=if modern{&new_root}else{new_root.require("Level",10)?};
    let new_list=new_source.require(if modern{"block_entities"}else{"TileEntities"},9)?;
    if new_list.items.len()!=list.items.len(){
        return Err(error("Block Entity list changed length after scalar preview"));
    }
    for (original,updated) in list.items.iter().zip(&new_list.items){
        if original.raw.as_ptr()==entity.raw.as_ptr(){
            if original.fields.len()!=updated.fields.len(){
                return Err(error("Edited BE changed number of existing tags"));
            }
            verify_be_scalar_path(original,updated,&path,&replacement)?;
        }else if original.raw!=updated.raw{
            return Err(error("BE scalar preview touched a different original Block Entity"));
        }
    }
    Ok(patched)
}

/// Borrow the original Block Entity NBT Compound *payload* and clone
/// it byte-for-byte for an independent opaque snapshot.
pub fn original_block_entity_bytes(
    original_nbt:&[u8],position:(i32,i32,i32),
)->Result<Option<Vec<u8>>>{
    let decoded=decode_sections(original_nbt)?;
    if !decoded.block_entities.contains(&position){return Ok(None);}
    let root=parse_root(original_nbt)?;
    let source=if root.field("sections").is_some(){
        &root
    }else{root.require("Level",10)?};
    let key=if root.field("sections").is_some(){"block_entities"}else{"TileEntities"};
    let list=source.require(key,9)?;
    let item=list.items.iter().find(|node|
        node.kind==10
        &&node.field("x").and_then(|tag|tag.i32().ok())==Some(position.0)
        &&node.field("y").and_then(|tag|tag.i32().ok())==Some(position.1)
        &&node.field("z").and_then(|tag|tag.i32().ok())==Some(position.2)
    ).ok_or_else(||error("Original BE list inconsistent with decoded coordinates"))?;
    Ok(Some(item.raw.to_vec()))
}

/// A strict original-byte, SAME-PALETTE named-state patch candidate.
/// This is an IN-MEMORY NBT transform, not a disk writer.
#[derive(Debug, Clone)]
pub struct InPaletteEdit {
    pub local_x: u8,
    pub y: i32,
    pub local_z: u8,
    pub original: State,
    pub replacement: State,
}
fn overwrite_word(bytes: &mut [u8], offset: usize, mask: u64, value: u64)
    -> Result<()> {
    let cell=bytes.get_mut(offset..offset+8)
        .ok_or_else(||error("NBT packed-long offset outside original buffer"))?;
    let word=u64::from_be_bytes(cell.as_ref().try_into().unwrap());
    cell.copy_from_slice(&((word & !mask)|(value & mask)).to_be_bytes());
    Ok(())
}
fn change_index(
    bytes:&mut [u8],longs_offset:usize,position:usize,
    palette_len:usize,long_count:usize,index:u16,
)->Result<()>{
    let bits=((usize::BITS-(palette_len-1).leading_zeros()) as usize).max(4);
    let per_word=64/bits;
    let padded_len=BLOCKS.div_ceil(per_word);
    let crossing_len=(BLOCKS*bits).div_ceil(64);
    let padded=if long_count==padded_len{true}
        else if long_count==crossing_len{false}
        else{return Err(error("Original block-state packing length is unsupported"))};
    let bit=if padded{((position%per_word)*bits) as u32}
        else{((position*bits)%64) as u32};
    let word=if padded{position/per_word}else{position*bits/64};
    let first=longs_offset.checked_add(word*8)
        .ok_or_else(||error("NBT original-long address overflow"))?;
    let mask=(1u64<<bits)-1;
    if bit as usize+bits<=64{
        overwrite_word(bytes,first,mask<<bit,(index as u64)<<bit)?;
    }else{
        let low=64-bit as usize;
        let low_mask=(1u64<<low)-1;
        overwrite_word(bytes,first,low_mask<<bit,((index as u64)&low_mask)<<bit)?;
        let upper=bits-low;
        let upper_mask=(1u64<<upper)-1;
        overwrite_word(bytes,first+8,upper_mask,(index as u64)>>low)?;
    }
    Ok(())
}
/// Patches only 8-byte packed TAG_Long_Array entries in their EXISTING
/// original locations. Palette growth, legacy numeric arrays and world
/// file replacement are intentionally not supported here. Every other NBT
/// byte (including arbitrary modded/Block Entity/entity data) remains
/// byte-for-byte original. Any unsafe candidate refuses the entire output.
pub fn patch_named_existing_palette(
    original_nbt:&[u8],edits:&[InPaletteEdit],
)->Result<Vec<u8>>{
    if edits.is_empty() || edits.len()>4096 {
        return Err(error("Palette patch requires 1..4096 original-state edits"));
    }
    let decoded=decode_sections(original_nbt)?;
    if !matches!(decoded.layout,Layout::Named113|Layout::Modern118) {
        return Err(error("Existing-palette byte patch requires named Anvil Sections"));
    }
    if decoded.block_entities.iter().any(|&(x,_,z)|
        decoded.x.is_none_or(|cx|x.div_euclid(16)!=cx)
        ||decoded.z.is_none_or(|cz|z.div_euclid(16)!=cz)
    ){return Err(error("Block Entity ownership is ambiguous or invalid"));}
    let root=parse_root(original_nbt)?;
    let modern=decoded.layout==Layout::Modern118;
    let source=if modern{&root}else{root.require("Level",10)?};
    let list=source.require(if modern{"sections"}else{"Sections"},9)?;
    let mut output=original_nbt.to_vec();
    let mut seen=BTreeSet::new();
    for change in edits{
        if change.local_x>=16||change.local_z>=16{
            return Err(error("Palette patch coordinate is outside its original chunk"));
        }
        let pos=(change.local_x,change.y,change.local_z);
        if !seen.insert(pos){return Err(error("Duplicate original palette patch coordinate"));}
        let sy=i8::try_from(change.y.div_euclid(16))
            .map_err(|_|error("Palette patch Section Y out of range"))?;
        let section=decoded.sections.iter().find(|s|s.y==sy)
            .ok_or_else(||error("Palette patch original Section does not exist"))?;
        let local_y=change.y.rem_euclid(16) as usize;
        let index=(local_y*16+change.local_z as usize)*16+change.local_x as usize;
        if section.block(change.local_x,local_y as u8,change.local_z)
            !=Some(&change.original) {
            return Err(error("Original block state differs from palette patch precondition"));
        }
        if change.original==change.replacement {
            return Err(error("Palette patch refuses no-op changes"));
        }
        if decoded.block_entities.iter().any(|&(x,y,z)|
            decoded.x.is_some_and(|cx|
                x.div_euclid(16)==cx && x.rem_euclid(16)==change.local_x as i32)
            && y==change.y
            && decoded.z.is_some_and(|cz|
                z.div_euclid(16)==cz && z.rem_euclid(16)==change.local_z as i32)
        ){return Err(error("Palette patch would replace an original Block Entity"));}
        let desired=section.palette.iter().position(|state|state==&change.replacement)
            .ok_or_else(||error("Replacement is not in original palette; growth unsupported"))?;
        let original_entry=list.items.iter().find(|entry|
            entry.field("Y").is_some_and(|y|y.byte().ok()==Some(sy))
        ).ok_or_else(||error("Original Section tag is missing"))?;
        let states=if modern{
            original_entry.require("block_states",10)?
        }else{original_entry};
        let packed=states.require(if modern{"data"}else{"BlockStates"},12)?;
        let longs=packed.longs()?;
        if longs.is_empty(){return Err(error("Original palette has no writable packed state data"));}
        let offset=(packed.raw.as_ptr() as usize)
            .checked_sub(original_nbt.as_ptr() as usize)
            .and_then(|n|n.checked_add(4))
            .ok_or_else(||error("Original data array offset is invalid"))?;
        let end=offset.checked_add(longs.len()*8)
            .ok_or_else(||error("Original packed data size overflow"))?;
        if original_nbt.get(offset..end)!=Some(&packed.raw[4..]) {
            return Err(error("NBT span is not part of original source buffer"));
        }
        change_index(&mut output,offset,index,section.palette.len(),longs.len(),
            u16::try_from(desired).map_err(|_|error("Palette index overflow"))?)?;
    }
    let verified=decode_sections(&output)?;
    if verified.block_entities!=decoded.block_entities || verified.layout!=decoded.layout
        || verified.x!=decoded.x || verified.z!=decoded.z {
        return Err(error("Patched original NBT metadata/Block Entities changed"));
    }
    for change in edits {
        if verified.block(change.local_x,change.y,change.local_z)!=Some(&change.replacement) {
            return Err(error("Patched palette result failed byte-level roundtrip check"));
        }
    }
    Ok(output)
}


/// A surgery over one original NBT payload span; all other bytes are retained.
struct NbtSurgery {
    start: usize,
    end: usize,
    replacement: Vec<u8>,
}
fn payload_span(nbt: &[u8], payload: &[u8]) -> Result<(usize, usize)> {
    let start = (payload.as_ptr() as usize)
        .checked_sub(nbt.as_ptr() as usize)
        .ok_or_else(|| error("NBT payload pointer precedes original bytes"))?;
    let end = start.checked_add(payload.len())
        .ok_or_else(|| error("NBT payload span overflow"))?;
    if nbt.get(start..end) != Some(payload) {
        return Err(error("NBT payload is not part of the original source"));
    }
    Ok((start, end))
}
fn write_named_tag(name: &str, value: &str, out: &mut Vec<u8>) -> Result<()> {
    let name_len = u16::try_from(name.len()).map_err(|_|error("NBT tag name too long"))?;
    let value_len = u16::try_from(value.len()).map_err(|_|error("NBT tag string too long"))?;
    out.push(8);
    out.extend_from_slice(&name_len.to_be_bytes());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(&value_len.to_be_bytes());
    out.extend_from_slice(value.as_bytes());
    Ok(())
}
fn appended_palette_entry(state: &State) -> Result<Vec<u8>> {
    // The original palette members are retained as opaque bytes. Only new
    // members use this narrowly scoped, canonical NBT member encoder.
    let mut member = Vec::new();
    write_named_tag("Name", &state.name, &mut member)?;
    if !state.properties.is_empty() {
        member.push(10); // Properties compound
        member.extend_from_slice(&("Properties".len() as u16).to_be_bytes());
        member.extend_from_slice(b"Properties");
        for (key, value) in &state.properties {
            write_named_tag(key, value, &mut member)?;
        }
        member.push(0); // end Properties
    }
    member.push(0); // end list-member compound
    Ok(member)
}
fn encode_packed_indices(indices: &[u16], palette_len: usize, padded: bool) -> Result<Vec<u8>> {
    if indices.len() != BLOCKS || !(2..=MAX_PALETTE).contains(&palette_len) {
        return Err(error("Invalid palette re-packing parameters"));
    }
    let bits = ((usize::BITS - (palette_len - 1).leading_zeros()) as usize).max(4);
    let per_word = 64 / bits;
    let word_count = if padded { BLOCKS.div_ceil(per_word) }
                     else { (BLOCKS * bits).div_ceil(64) };
    let mut words = vec![0u64; word_count];
    for (index, &state) in indices.iter().enumerate() {
        if state as usize >= palette_len {
            return Err(error("Palette index exceeds grown palette"));
        }
        if padded {
            let word = index / per_word;
            let shift = (index % per_word) * bits;
            words[word] |= (state as u64) << shift;
        } else {
            let bit = index * bits;
            let word = bit / 64;
            let offset = bit % 64;
            words[word] |= (state as u64) << offset;
            if offset + bits > 64 {
                words[word + 1] |= (state as u64) >> (64 - offset);
            }
        }
    }
    let mut output = Vec::with_capacity(4 + words.len() * 8);
    output.extend_from_slice(&(words.len() as i32).to_be_bytes());
    for word in words { output.extend_from_slice(&word.to_be_bytes()); }
    Ok(output)
}
fn source_packing(chunk: &Chunk, palette_len: usize, target_len: usize, source_words: usize) -> Result<bool> {
    if palette_len == 1 && source_words == 0 {
        // First growth uses four bits; padded and contiguous are identical.
        return Ok(true);
    }
    let bits = ((usize::BITS - (palette_len - 1).leading_zeros()) as usize).max(4);
    let padded = BLOCKS.div_ceil(64 / bits);
    let contiguous = (BLOCKS * bits).div_ceil(64);
    match (source_words == padded, source_words == contiguous) {
        (true, false) => Ok(true),
        (false, true) => Ok(false),
        (true, true) => {
            let target_bits = ((usize::BITS - (target_len - 1).leading_zeros()) as usize).max(4);
            if target_bits == bits {
                // Identical word packing remains identical at this bit-width.
                return Ok(true);
            }
            // At the 16 -> 17 palette threshold the original four-bit
            // layout is indistinguishable. The *version* must choose
            // crossing (1.13-1.15) vs padded (1.16+) for the five-bit result.
            match chunk.layout {
                Layout::Modern118 => Ok(true),
                Layout::Named113 => match chunk.data_version {
                    Some(version) if (1519..=2230).contains(&version) => Ok(false),
                    Some(version) if version >= 2566 => Ok(true),
                    _ => Err(error("Original DataVersion cannot determine five-bit packing")),
                },
                _ => Err(error("Palette source is not a named Java Section")),
            }
        }
        _ => Err(error(format!(
            "Cannot infer original packed layout for {} source words", source_words
        ))),
    }
}
/// Grow an existing named Section palette without round-tripping the NBT tree.
///
/// This function only splices specific Palette and BlockStates/data payloads,
/// retaining byte-for-byte all other original tags. Missing packed data in a
/// one-state palette is inserted immediately before the enclosing compound's
/// original TAG_End. It never writes Minecraft files. The entire candidate
/// is decoded and every unedited voxel compared before it is returned.
pub fn patch_named_growing_palette(
    original_nbt: &[u8], edits: &[InPaletteEdit],
) -> Result<Vec<u8>> {
    if edits.is_empty() || edits.len() > BLOCKS * MAX_SECTIONS {
        return Err(error("Palette growth batch is empty or exceeds section budget"));
    }
    let decoded = decode_sections(original_nbt)?;
    if !matches!(decoded.layout, Layout::Named113 | Layout::Modern118) {
        return Err(error("Palette growth supports named Java Anvil chunks only"));
    }
    if decoded.block_entities.iter().any(|&(x, _, z)| {
        decoded.x.is_none_or(|cx| x.div_euclid(16) != cx)
            || decoded.z.is_none_or(|cz| z.div_euclid(16) != cz)
    }) {
        return Err(error("Original Block Entity ownership cannot be proven"));
    }
    let modern = decoded.layout == Layout::Modern118;
    let root = parse_root(original_nbt)?;
    let container = if modern { &root } else { root.require("Level", 10)? };
    let sections = container.require(if modern { "sections" } else { "Sections" }, 9)?;
    let mut per_section: std::collections::BTreeMap<i8, Vec<&InPaletteEdit>>
        = std::collections::BTreeMap::new();
    let mut visited = BTreeSet::new();
    for edit in edits {
        if edit.local_x >= 16 || edit.local_z >= 16 {
            return Err(error("Palette growth coordinate outside chunk"));
        }
        let sy = i8::try_from(edit.y.div_euclid(16))
            .map_err(|_| error("Invalid signed Section Y"))?;
        if !visited.insert((edit.local_x, edit.y, edit.local_z)) {
            return Err(error("Duplicate block in palette growth batch"));
        }
        let chunk_section = decoded.sections.iter().find(|s| s.y == sy)
            .ok_or_else(|| error("Original named Section does not exist"))?;
        if chunk_section.block(edit.local_x, edit.y.rem_euclid(16) as u8, edit.local_z)
            != Some(&edit.original) || edit.original == edit.replacement {
            return Err(error("Original block precondition or replacement is invalid"));
        }
        // Fail closed if original chunk coordinates are absent with BEs.
        if decoded.block_entities.iter().any(|&(x,y,z)| {
            decoded.x.is_some_and(|cx|
                x.div_euclid(16)==cx && x.rem_euclid(16)==edit.local_x as i32)
                && y==edit.y
                && decoded.z.is_some_and(|cz|
                    z.div_euclid(16)==cz && z.rem_euclid(16)==edit.local_z as i32)
        }) {
            return Err(error("Palette growth would replace an original Block Entity"));
        }
        per_section.entry(sy).or_default().push(edit);
    }
    let mut operations = Vec::<NbtSurgery>::new();
    for (y, section_edits) in per_section {
        let decoded_section = decoded.sections.iter().find(|s| s.y == y)
            .ok_or_else(|| error("Missing decoded Section"))?;
        let original_section = sections.items.iter().find(|item|
            item.field("Y").is_some_and(|tag| tag.byte().ok() == Some(y))
        ).ok_or_else(|| error("Missing original Section tag"))?;
        let block_states = if modern {
            original_section.require("block_states", 10)?
        } else { original_section };
        let palette_tag = block_states.require(if modern { "palette" } else { "Palette" }, 9)?;
        if palette_tag.items.len() != decoded_section.palette.len() {
            return Err(error("Original palette decode mismatch"));
        }
        let mut palette = decoded_section.palette.clone();
        let mut indices = decoded_section.indices.clone();
        let mut added = Vec::new();
        for edit in section_edits {
            let target = if let Some(existing) = palette.iter().position(|s|s==&edit.replacement) {
                existing
            } else {
                if palette.len() >= MAX_PALETTE {
                    return Err(error("Section palette cannot exceed 4096 states"));
                }
                added.extend_from_slice(&appended_palette_entry(&edit.replacement)?);
                palette.push(edit.replacement.clone());
                palette.len()-1
            };
            let index = ((edit.y.rem_euclid(16) as usize * 16
                + edit.local_z as usize) * 16) + edit.local_x as usize;
            indices[index] = u16::try_from(target)
                .map_err(|_| error("Section palette index overflow"))?;
        }
        if !added.is_empty() {
            // Preserve each original palette member verbatim. Only the list
            // count and appended entry bytes differ in this single payload.
            let mut payload = palette_tag.raw.to_vec();
            if payload.len()<5 || payload[0]!=10 {
                return Err(error("Original palette list has incompatible element type"));
            }
            payload[1..5].copy_from_slice(&(palette.len() as i32).to_be_bytes());
            payload.extend_from_slice(&added);
            let (start,end)=payload_span(original_nbt,palette_tag.raw)?;
            operations.push(NbtSurgery{start,end,replacement:payload});
        }
        let packed = block_states.field(if modern { "data" } else { "BlockStates" });
        let (original_len, padded) = match packed {
            Some(tag) => {
                let words=tag.longs()?;
                (words.len(),source_packing(&decoded, palette_tag.items.len(),palette.len(),words.len())?)
            }
            None if decoded_section.palette.len()==1 => (0,true),
            None => return Err(error("Cannot expand palette with missing original packed data")),
        };
        let _ = original_len;
        let replacement = encode_packed_indices(&indices,palette.len(),padded)?;
        if let Some(tag)=packed {
            let (start,end)=payload_span(original_nbt,tag.raw)?;
            operations.push(NbtSurgery{start,end,replacement});
        }else{
            // Insert a new LongArray TAG immediately before the original
            // compound's TAG_End without disturbing any other original tags.
            let key=if modern {"data"}else{"BlockStates"};
            let mut entry = vec![12u8];
            entry.extend_from_slice(&(key.len() as u16).to_be_bytes());
            entry.extend_from_slice(key.as_bytes());
            entry.extend_from_slice(&replacement);
            let (start,end)=payload_span(original_nbt,block_states.raw)?;
            if start==end || original_nbt[end-1]!=0 {
                return Err(error("Original Section Compound has no TAG_End"));
            }
            operations.push(NbtSurgery{start:end-1,end:end-1,replacement:entry});
        }
    }
    operations.sort_by_key(|s|s.start);
    for pair in operations.windows(2) {
        if pair[0].end > pair[1].start {
            return Err(error("Overlapping original NBT payload surgeries"));
        }
    }
    let mut output=original_nbt.to_vec();
    for op in operations.into_iter().rev() {
        output.splice(op.start..op.end,op.replacement);
        if output.len()>64*1024*1024 {
            return Err(error("Patched NBT exceeded maximum decoded size"));
        }
    }
    let after=decode_sections(&output)?;
    if after.layout!=decoded.layout || after.data_version!=decoded.data_version
        || after.x!=decoded.x || after.z!=decoded.z
        || after.block_entities!=decoded.block_entities
        || after.sections.len()!=decoded.sections.len() {
        return Err(error("NBT palette surgery altered original metadata"));
    }
    let expected: std::collections::BTreeMap<(i8,usize),&State> = edits.iter().map(|edit|{
        let sy=edit.y.div_euclid(16) as i8;
        let idx=((edit.y.rem_euclid(16) as usize*16
            +edit.local_z as usize)*16)+edit.local_x as usize;
        ((sy,idx),&edit.replacement)
    }).collect();
    for section in &decoded.sections {
        let new=after.sections.iter().find(|s|s.y==section.y)
            .ok_or_else(||error("Patched Section disappeared"))?;
        for index in 0..BLOCKS {
            let before=section.palette.get(section.indices[index] as usize)
                .ok_or_else(||error("Invalid original palette index"))?;
            let want=expected.get(&(section.y,index)).copied().unwrap_or(before);
            let actual=new.palette.get(new.indices[index] as usize)
                .ok_or_else(||error("Invalid new palette index"))?;
            if actual!=want {
                return Err(error("Original untouched voxel changed during palette surgery"));
            }
        }
    }
    Ok(output)
}


/// Decode an original numeric NBT ByteArray without modifying its payload.
fn byte_array_exact<'a>(tag: &'a Tag<'_>, length: usize) -> Result<&'a [u8]> {
    if tag.kind != 7 || tag.raw.len() != 4 + length {
        return Err(error("Original numeric byte array has unexpected length"));
    }
    let declared = i32::from_be_bytes(tag.raw[..4].try_into().unwrap());
    if declared != length as i32 {
        return Err(error("Original numeric byte array count mismatch"));
    }
    Ok(&tag.raw[4..])
}
fn numeric_pair(state: &State) -> Result<(u16, u8)> {
    let id = state.name.strip_prefix("numeric:")
        .ok_or_else(||error("Numeric chunk requires original numeric:ID state"))?
        .parse::<u16>().map_err(|_|error("Original numeric ID is invalid"))?;
    if id > 4095 || state.properties.len()!=1 || state.properties[0].0!="data" {
        return Err(error("Numeric state must contain exactly data=0..15"));
    }
    let meta=state.properties[0].1.parse::<u8>()
        .map_err(|_|error("Numeric data is not an integer"))?;
    if meta > 15 {return Err(error("Numeric block metadata exceeds four bits"));}
    Ok((id,meta))
}
fn write_nibble(array: &mut [u8], position: usize, value: u8) -> Result<()> {
    let byte = array.get_mut(position/2)
        .ok_or_else(||error("Numeric nibble position out of bounds"))?;
    if position%2 == 0 { *byte = (*byte & 0xF0) | value; }
    else { *byte = (*byte & 0x0F) | (value << 4); }
    Ok(())
}
fn span_replacement(nbt:&[u8], tag:&Tag<'_>, bytes:Vec<u8>) -> Result<NbtSurgery> {
    let (start,end)=payload_span(nbt,tag.raw)?;
    if bytes.len()!=tag.raw.len(){return Err(error("Original byte array length changed"));}
    Ok(NbtSurgery{start,end,replacement:bytes})
}
/// Apply original 1.12.2 numeric Anvil or MCRegion/Alpha numeric ID edits.
///
/// Writes only the original Blocks/Data/Add byte-array PAYLOADS. Existing
/// optional Add arrays are preserved, and Add is created only if a target
/// ID requires high bits in an original numeric Anvil Section. MCRegion's
/// 8-bit IDs cannot be extended. The modified candidate remains IN MEMORY.
pub fn patch_original_numeric_arrays(
    original_nbt:&[u8], edits:&[InPaletteEdit],
)->Result<Vec<u8>>{
    if edits.is_empty() || edits.len()>BLOCKS*MAX_SECTIONS {
        return Err(error("Numeric patch batch is empty or over budget"));
    }
    let before=decode_sections(original_nbt)?;
    if !matches!(before.layout,Layout::McRegion128|Layout::NumericAnvil112){
        return Err(error("Original numeric patch requires numeric Java world storage"));
    }
    if before.block_entities.iter().any(|&(x,_,z)|
        before.x.is_none_or(|cx|x.div_euclid(16)!=cx)
            ||before.z.is_none_or(|cz|z.div_euclid(16)!=cz)
    ){return Err(error("Numeric Block Entity original coordinates are ambiguous"));}
    let root=parse_root(original_nbt)?;
    let level=root.require("Level",10)?;
    let mcregion=before.layout==Layout::McRegion128;
    let sections=if mcregion{None}else{Some(level.require("Sections",9)?)};
    let mut groups:std::collections::BTreeMap<i8,Vec<&InPaletteEdit>>
        =std::collections::BTreeMap::new();
    let mut visited=BTreeSet::new();
    for edit in edits{
        if edit.local_x>=16 || edit.local_z>=16{
            return Err(error("Numeric patch coordinate outside original chunk"));
        }
        let sy=i8::try_from(edit.y.div_euclid(16))
            .map_err(|_|error("Numeric edit Section Y outside signed byte range"))?;
        if mcregion && !(0..128).contains(&edit.y){
            return Err(error("MCRegion numeric edits must stay in Y 0..127"));
        }
        if !visited.insert((edit.local_x,edit.y,edit.local_z)){
            return Err(error("Duplicate numeric patch coordinate"));
        }
        let original=before.block(edit.local_x,edit.y,edit.local_z);
        if original!=Some(&edit.original) || edit.original==edit.replacement {
            return Err(error("Numeric edit source state is stale or unchanged"));
        }
        numeric_pair(&edit.original)?;
        let (id,_) = numeric_pair(&edit.replacement)?;
        if mcregion && id>255 {
            return Err(error("MCRegion Blocks array supports only 8-bit IDs"));
        }
        if before.block_entities.iter().any(|&(x,y,z)|
            before.x.is_some_and(|cx|
                x.div_euclid(16)==cx && x.rem_euclid(16)==edit.local_x as i32)
                && y==edit.y
                && before.z.is_some_and(|cz|
                    z.div_euclid(16)==cz && z.rem_euclid(16)==edit.local_z as i32)
        ){return Err(error("Numeric patch would replace original Block Entity"));}
        groups.entry(if mcregion{0}else{sy}).or_default().push(edit);
    }
    let mut operations=Vec::<NbtSurgery>::new();
    for (section_y,group) in groups{
        let source=if mcregion{level}else{
            sections.expect("required numeric Sections").items.iter().find(|s|
                s.field("Y").is_some_and(|tag|tag.byte().ok()==Some(section_y))
            ).ok_or_else(||error("Original numeric Section is missing"))?
        };
        let array_len=if mcregion{32768}else{4096};
        let nibble_len=if mcregion{16384}else{2048};
        let blocks_tag=source.require("Blocks",7)?;
        let data_tag=source.require("Data",7)?;
        let mut blocks=blocks_tag.raw.to_vec();
        let mut data=data_tag.raw.to_vec();
        byte_array_exact(blocks_tag,array_len)?;
        byte_array_exact(data_tag,nibble_len)?;
        let add_tag=if mcregion{None}else{source.field("Add")};
        let mut add=if let Some(tag)=add_tag {
            byte_array_exact(tag,nibble_len)?;
            tag.raw.to_vec()
        }else{
            let mut empty=(nibble_len as i32).to_be_bytes().to_vec();
            empty.extend(vec![0u8;nibble_len]);
            empty
        };
        let mut need_add=false;
        for change in group{
            let (id,meta)=numeric_pair(&change.replacement)?;
            let index=if mcregion{
                (change.local_x as usize*16+change.local_z as usize)*128
                    +change.y as usize
            }else{
                ((change.y.rem_euclid(16) as usize*16
                    +change.local_z as usize)*16)+change.local_x as usize
            };
            *blocks.get_mut(4+index)
                .ok_or_else(||error("Original numeric Blocks index outside array"))?=id as u8;
            write_nibble(&mut data[4..],index,meta)?;
            if !mcregion {
                let high=(id >> 8) as u8;
                write_nibble(&mut add[4..],index,high)?;
                need_add |= high != 0;
            }
        }
        operations.push(span_replacement(original_nbt,blocks_tag,blocks)?);
        operations.push(span_replacement(original_nbt,data_tag,data)?);
        if let Some(tag)=add_tag{
            operations.push(span_replacement(original_nbt,tag,add)?);
        }else if need_add{
            let mut entry=vec![7u8];
            entry.extend_from_slice(&3u16.to_be_bytes());
            entry.extend_from_slice(b"Add");
            entry.extend_from_slice(&add);
            let (_start,end)=payload_span(original_nbt,source.raw)?;
            if end==0 || original_nbt[end-1]!=0 {
                return Err(error("Numeric Section Compound has no TAG_End"));
            }
            operations.push(NbtSurgery{
                start:end-1,end:end-1,replacement:entry
            });
        }
    }
    operations.sort_by_key(|operation|operation.start);
    for two in operations.windows(2){
        if two[0].end>two[1].start{
            return Err(error("Overlapping original numeric NBT spans"));
        }
    }
    let mut output=original_nbt.to_vec();
    for operation in operations.into_iter().rev(){
        output.splice(operation.start..operation.end,operation.replacement);
        if output.len()>64*1024*1024{
            return Err(error("Patched numeric chunk exceeds 64 MiB NBT limit"));
        }
    }
    let after=decode_sections(&output)?;
    if before.layout!=after.layout || before.x!=after.x || before.z!=after.z
        ||before.data_version!=after.data_version
        ||before.block_entities!=after.block_entities
        ||before.sections.len()!=after.sections.len(){
        return Err(error("Numeric NBT patch changed original chunk metadata"));
    }
    let expected:std::collections::BTreeMap<(i8,usize),&State>=edits.iter().map(|edit|{
        let section_y=edit.y.div_euclid(16) as i8;
        let index=((edit.y.rem_euclid(16) as usize*16
            +edit.local_z as usize)*16)+edit.local_x as usize;
        ((section_y,index),&edit.replacement)
    }).collect();
    for old in &before.sections {
        let current=after.sections.iter().find(|section|section.y==old.y)
            .ok_or_else(||error("Numeric patched Section missing"))?;
        for index in 0..BLOCKS {
            let original=old.palette.get(old.indices[index] as usize)
                .ok_or_else(||error("Original numeric palette index missing"))?;
            let desired=expected.get(&(old.y,index)).copied().unwrap_or(original);
            let actual=current.palette.get(current.indices[index] as usize)
                .ok_or_else(||error("Result numeric palette index missing"))?;
            if desired!=actual {
                return Err(error("Unedited original numeric block changed during byte patch"));
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn named(kind: u8, name: &str, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![kind];out.extend((name.len() as u16).to_be_bytes());
        out.extend(name.as_bytes());out.extend(payload);out
    }
    fn compound(children: Vec<Vec<u8>>) -> Vec<u8> {
        let mut p = children.concat();p.push(0);p
    }
    fn string(s: &str) -> Vec<u8> {
        let mut p = (s.len() as u16).to_be_bytes().to_vec();p.extend(s.as_bytes());p
    }
    fn modern_chunk(y: i8, data: &[u64]) -> Vec<u8> {
        let palette = {
            let mut p = vec![10];p.extend(2i32.to_be_bytes());
            for name in ["minecraft:stone","minecraft:oak_log"] {
                p.extend(compound(vec![named(8, "Name", &string(name))]));
            }
            p
        };
        let mut longs = (data.len() as i32).to_be_bytes().to_vec();
        for word in data { longs.extend(word.to_be_bytes()); }
        let section = compound(vec![
            named(1,"Y",&[y as u8]),
            named(10,"block_states",&compound(vec![
                named(9,"palette",&palette),
                named(12,"data",&longs)
            ]))
        ]);
        let mut section_list = vec![10];section_list.extend(1i32.to_be_bytes());
        section_list.extend(section);
        named(10,"",&compound(vec![
            named(3,"DataVersion",&5023i32.to_be_bytes()),
            named(3,"xPos",&(-7i32).to_be_bytes()),
            named(3,"zPos",&3i32.to_be_bytes()),
            named(9,"sections",&section_list),
            named(9,"block_entities",&[10,0,0,0,0])
        ]))
    }

    #[test]
    fn in_memory_palette_patch_changes_only_existing_packed_words() {
        let source=modern_chunk(-4,&vec![0;256]);
        let original_len=source.len();
        let transformed=patch_named_existing_palette(&source,&[
            InPaletteEdit{
                local_x:2,y:-63,local_z:3,
                original:State{name:"minecraft:stone".into(),properties:vec![]},
                replacement:State{name:"minecraft:oak_log".into(),properties:vec![]},
            },
            InPaletteEdit{
                local_x:3,y:-63,local_z:3,
                original:State{name:"minecraft:stone".into(),properties:vec![]},
                replacement:State{name:"minecraft:oak_log".into(),properties:vec![]},
            },
        ]).unwrap();
        assert_eq!(source.len(),original_len);
        assert_eq!(transformed.len(),original_len);
        assert_eq!(decode_sections(&transformed).unwrap().block(2,-63,3).unwrap().name,
            "minecraft:oak_log");
        assert_eq!(decode_sections(&transformed).unwrap().block(3,-63,3).unwrap().name,
            "minecraft:oak_log");
        assert_eq!(decode_sections(&transformed).unwrap().block(4,-63,3).unwrap().name,
            "minecraft:stone");
        let differences=source.iter().zip(&transformed)
            .filter(|(before,after)|before!=after).count();
        assert!(differences>0 && differences<=8);
    }
    #[test]
    fn palette_patch_refuses_new_state_and_duplicate_or_wrong_source() {
        let source=modern_chunk(-4,&vec![0;256]);
        let stone=State{name:"minecraft:stone".into(),properties:vec![]};
        let oak=State{name:"minecraft:oak_log".into(),properties:vec![]};
        assert!(patch_named_existing_palette(&source,&[
            InPaletteEdit{local_x:0,y:-64,local_z:0,original:stone.clone(),
                replacement:State{name:"minecraft:gold_block".into(),properties:vec![]}},
        ]).is_err());
        assert!(patch_named_existing_palette(&source,&[
            InPaletteEdit{local_x:0,y:-64,local_z:0,original:oak.clone(),replacement:stone.clone()},
        ]).is_err());
        assert!(patch_named_existing_palette(&source,&[
            InPaletteEdit{local_x:0,y:-64,local_z:0,original:stone.clone(),replacement:oak.clone()},
            InPaletteEdit{local_x:0,y:-64,local_z:0,original:stone,replacement:oak},
        ]).is_err());
    }
    #[test]
    fn grow_modern_palette_and_preserve_all_untouched_originals() {
        let old=modern_chunk(-4,&vec![0;256]);
        let original=decode_sections(&old).unwrap();
        let before=old.clone();
        let updated=patch_named_growing_palette(&old,&[
            InPaletteEdit{
                local_x:1,y:-64,local_z:1,
                original:State{name:"minecraft:stone".into(),properties:vec![]},
                replacement:State{name:"mod:custom_machine".into(),
                    properties:vec![("facing".into(),"north".into())]},
            },
        ]).unwrap();
        let now=decode_sections(&updated).unwrap();
        assert_eq!(now.block(1,-64,1).unwrap().name,"mod:custom_machine");
        assert_eq!(now.block(0,-64,0),original.block(0,-64,0));
        assert_eq!(old,before);
        assert!(updated.len()>old.len());
        assert!(updated.windows("block_entities".len()).any(|x|x==b"block_entities"));
    }
    fn single_state_modern_no_data() -> Vec<u8> {
        let mut palette=vec![10];
        palette.extend(1i32.to_be_bytes());
        palette.extend(compound(vec![named(8,"Name",&string("minecraft:stone"))]));
        let section=compound(vec![
            named(1,"Y",&[(-4i8) as u8]),
            named(10,"block_states",&compound(vec![named(9,"palette",&palette)])),
            named(8,"opaque_state",&string("exact bytes retained")),
        ]);
        let mut sections=vec![10];sections.extend(1i32.to_be_bytes());sections.extend(section);
        named(10,"",&compound(vec![
            named(3,"DataVersion",&5023i32.to_be_bytes()),
            named(3,"xPos",&(-7i32).to_be_bytes()),
            named(3,"zPos",&3i32.to_be_bytes()),
            named(9,"sections",&sections),
            named(9,"block_entities",&[10,0,0,0,0]),
        ]))
    }
    #[test]
    fn grow_one_state_without_data_and_keep_modded_nbt_bytes() {
        let old=single_state_modern_no_data();
        let next=patch_named_growing_palette(&old,&[
            InPaletteEdit{
                local_x:0,y:-64,local_z:0,
                original:State{name:"minecraft:stone".into(),properties:vec![]},
                replacement:State{name:"minecraft:gold_block".into(),properties:vec![]}
            }
        ]).unwrap();
        let parsed=decode_sections(&next).unwrap();
        assert_eq!(parsed.block(0,-64,0).unwrap().name,"minecraft:gold_block");
        assert_eq!(parsed.block(15,-64,15).unwrap().name,"minecraft:stone");
        assert!(next.windows("exact bytes retained".len())
            .any(|x|x==b"exact bytes retained"));
        assert!(old.windows("exact bytes retained".len())
            .any(|x|x==b"exact bytes retained"));
    }
    #[test]
    fn palette_growth_rejects_duplicate_target_and_original_mismatch() {
        let old=single_state_modern_no_data();
        let stone=State{name:"minecraft:stone".into(),properties:vec![]};
        let gold=State{name:"minecraft:gold_block".into(),properties:vec![]};
        let entry=InPaletteEdit{
            local_x:0,y:-64,local_z:0,original:stone.clone(),replacement:gold.clone()
        };
        assert!(patch_named_growing_palette(&old,&[entry.clone(),entry]).is_err());
        assert!(patch_named_growing_palette(&old,&[
            InPaletteEdit{local_x:0,y:-64,local_z:0,original:gold,replacement:stone}
        ]).is_err());
    }

    fn sixteen_palette_old_named(version: i32) -> Vec<u8> {
        let mut palette=vec![10];
        palette.extend(16i32.to_be_bytes());
        for index in 0..16 {
            palette.extend(compound(vec![
                named(8,"Name",&string(&format!("test:block_{index}"))),
            ]));
        }
        let mut data=(256i32).to_be_bytes().to_vec();
        data.extend(vec![0u8;256*8]);
        let section=compound(vec![
            named(1,"Y",&[0]),
            named(9,"Palette",&palette),
            named(12,"BlockStates",&data),
            named(8,"Opaque",&string("original opaque NBT")),
        ]);
        let mut sections=vec![10];sections.extend(1i32.to_be_bytes());sections.extend(section);
        named(10,"",&compound(vec![
            named(3,"DataVersion",&version.to_be_bytes()),
            named(10,"Level",&compound(vec![
                named(3,"xPos",&0i32.to_be_bytes()),
                named(3,"zPos",&0i32.to_be_bytes()),
                named(9,"Sections",&sections),
            ]))
        ]))
    }
    #[test]
    fn legacy_named_palette_grows_from_four_to_five_bits_with_crossing_words() {
        let old=sixteen_palette_old_named(1631); // Java 1.13.x
        let changed=patch_named_growing_palette(&old,&[
            InPaletteEdit {
                local_x:15,y:1,local_z:15,
                original:State{name:"test:block_0".into(),properties:vec![]},
                replacement:State{name:"modded:new_state".into(),properties:vec![]},
            },
        ]).unwrap();
        let decoded=decode_sections(&changed).unwrap();
        assert_eq!(decoded.block(15,1,15).unwrap().name,"modded:new_state");
        assert_eq!(decoded.block(14,1,15).unwrap().name,"test:block_0");
        let root=parse_root(&changed).unwrap();
        let data=root.require("Level",10).unwrap().require("Sections",9).unwrap()
            .items[0].require("BlockStates",12).unwrap().longs().unwrap();
        assert_eq!(data.len(),320); // 4096 * 5 / 64, pre-1.16 crossing
        assert!(changed.windows(b"original opaque NBT".len())
            .any(|x|x==b"original opaque NBT"));
    }
    #[test]
    fn ambiguous_legacy_version_rejects_palette_bit_growth() {
        let old=sixteen_palette_old_named(2400); // unsupported transition version
        assert!(patch_named_growing_palette(&old,&[
            InPaletteEdit {
                local_x:0,y:0,local_z:0,
                original:State{name:"test:block_0".into(),properties:vec![]},
                replacement:State{name:"mod:new_state".into(),properties:vec![]},
            },
        ]).is_err());
    }
    #[test]
    fn latest_snapshot_26_4_snapshot_5_5125_reuses_structural_layout_without_guessing() {
        let words = vec![0u64; 256];
        let mut original = modern_chunk(-4, &words);
        let key = 5023_i32.to_be_bytes();
        let idx = original.windows(4).position(|x| x == key)
            .expect("synthetic fixture DataVersion");
        original[idx..idx + 4].copy_from_slice(&5125_i32.to_be_bytes());
        let result = decode_sections(&original).unwrap();
        assert_eq!(result.data_version, Some(5125));
        assert_eq!(result.layout, Layout::Modern118);
        assert_eq!(result.block(0, -64, 0).unwrap().name, "minecraft:stone");
    }
    #[test]
    fn minecraft_26_3_padded_and_negative_y() {
        let mut data = vec![0u64;256];
        let n = 306usize;
        data[n/16] |= 1u64 << ((n%16)*4);
        let raw = modern_chunk(-4,&data);
        let original = raw.clone();
        let chunk = decode_sections(&raw).unwrap();
        assert_eq!(chunk.data_version,Some(5023));
        assert_eq!(chunk.layout,Layout::Modern118);
        assert_eq!(chunk.block(2,-63,3).unwrap().name,"minecraft:oak_log");
        assert_eq!(chunk.block(2,-62,3).unwrap().name,"minecraft:stone");
        assert_eq!(chunk.x,Some(-7));
        assert_eq!(raw,original);
    }
    #[test]
    fn contiguous_and_padded_five_bit_encoding() {
        let mut contiguous = vec![0u64;320];
        let mut padded = vec![0u64;342];
        contiguous[0] |= 1 << 60;
        padded[1] |= 1;
        assert_eq!(unpack(17,&contiguous).unwrap()[12],1);
        assert_eq!(unpack(17,&padded).unwrap()[12],1);
    }
    #[test]
    fn reject_unknown_palette_and_invalid_index() {
        assert!(unpack(0,&[]).is_err());
        assert!(unpack(2,&[]).is_err());
        let mut words = vec![0;256];words[0]=2;
        assert!(unpack(2,&words).is_err());
        assert!(decode_sections(&modern_chunk(-4,&vec![0;15])).is_err());
    }
}
