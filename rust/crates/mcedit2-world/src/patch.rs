//! Original NBT SHA-256-verified external block-edit journal; no world writes.
use std::collections::{BTreeMap,BTreeSet};
use std::fs::{self,OpenOptions};
use std::io::Write;
use std::path::Path;
use serde::{Serialize,Deserialize};
use sha2::{Digest,Sha256};
use crate::block_lookup::read_world_chunk_bytes;
use crate::editing::{parse_state,render_state};
use crate::sections::{decode_sections,Chunk,Layout,State};
use crate::{WorldError,WorldSummary};

type Pos=(i32,i32,i32);
type ChunkPos=(i32,i32);
const MAGIC:&str="MCEDIT2_RUST_ORIGINAL_BLOCK_PATCH";
const VERSION:u32=1;
const MAX_BYTES:usize=32*1024*1024;
const MAX_CELLS:usize=65_536;
const MAX_CHUNKS:usize=8_192;
fn err(s:impl Into<String>)->WorldError{WorldError::Invalid(s.into())}
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal{
    magic:String,version:u32,source_root:String,dimension:String,
    data_version:Option<i32>,chunks:Vec<Stamp>,edits:Vec<Entry>,
}
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Stamp{x:i32,z:i32,nbt_sha256:String}
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry{x:i32,y:i32,z:i32,original:State,replacement:State}
struct Snapshot{chunk:Chunk,digest:String}
fn digest(bytes:&[u8])->String{
    Sha256::digest(bytes).iter().map(|b|format!("{b:02x}")).collect()
}
fn source_chunk(world:&WorldSummary,dimension:&str,cx:i32,cz:i32)->Result<Snapshot,WorldError>{
    let raw=read_world_chunk_bytes(world,dimension,cx,cz)?
        .ok_or_else(||err(format!("Original chunk {cx},{cz} is absent")))?;
    let chunk=decode_sections(&raw)
        .map_err(|e|err(format!("Original chunk NBT invalid: {e}")))?;
    if chunk.x.is_some_and(|x|x!=cx) || chunk.z.is_some_and(|z|z!=cz) {
        return Err(err("Chunk NBT coordinates do not match source slot"));
    }
    if chunk.block_entities.iter().any(|&(x,_,z)|
        x.div_euclid(16)!=cx || z.div_euclid(16)!=cz
    ){return Err(err("Block Entity is outside its original chunk"));}
    Ok(Snapshot{chunk,digest:digest(&raw)})
}
fn original(chunk:&Chunk,pos:Pos)->Result<State,WorldError>{
    chunk.block(pos.0.rem_euclid(16) as u8,pos.1,pos.2.rem_euclid(16) as u8)
        .cloned().ok_or_else(||err("Journal target original Section is absent"))
}
fn valid_replacement(state:&State,layout:&Layout)->Result<(),WorldError>{
    if parse_state(&render_state(state))? != *state {
        return Err(err("Journal replacement has invalid or noncanonical properties"));
    }
    let numeric=matches!(layout,Layout::McRegion128|Layout::NumericAnvil112);
    if numeric != state.name.starts_with("numeric:") {
        return Err(err("Journal replacement does not match original ID family"));
    }
    Ok(())
}
fn root(world:&WorldSummary)->Result<&str,WorldError>{
    world.root.to_str().ok_or_else(||err("Non-Unicode world path not journal-compatible"))
}
fn snapshots(world:&WorldSummary,dimension:&str,expected:&BTreeMap<ChunkPos,String>)
    ->Result<BTreeMap<ChunkPos,Snapshot>,WorldError>{
    let mut found=BTreeMap::new();
    for (&(cx,cz),sha) in expected {
        let loaded=source_chunk(world,dimension,cx,cz)?;
        if loaded.digest!=*sha {
            return Err(err(format!("Original NBT changed in chunk {cx},{cz}; import refused")));
        }
        found.insert((cx,cz),loaded);
    }
    Ok(found)
}
pub(crate) fn encode(world:&WorldSummary,dimension:&str,overlay:&BTreeMap<Pos,State>)
    ->Result<Vec<u8>,WorldError>{
    if overlay.is_empty()||overlay.len()>MAX_CELLS{
        return Err(err("Journal requires 1..65536 staged cells"));
    }
    let mut source:BTreeMap<ChunkPos,Snapshot>=BTreeMap::new();
    let mut edits=Vec::with_capacity(overlay.len());
    for (&pos,replacement) in overlay{
        let key=(pos.0.div_euclid(16),pos.2.div_euclid(16));
        if !source.contains_key(&key){
            if source.len()>=MAX_CHUNKS{return Err(err("Too many touched chunks"));}
            source.insert(key,source_chunk(world,dimension,key.0,key.1)?);
        }
        let chunk=&source.get(&key).expect("loaded chunk").chunk;
        if chunk.has_block_entity(pos.0,pos.1,pos.2) {
            return Err(err("Cannot journal edits over an original Block Entity"));
        }
        let previous=original(chunk,pos)?;
        valid_replacement(replacement,&chunk.layout)?;
        if previous==*replacement{return Err(err("Redundant journal overlay state"));}
        edits.push(Entry{
            x:pos.0,y:pos.1,z:pos.2,original:previous,replacement:replacement.clone()
        });
    }
    let expected:BTreeMap<_,_>=source.iter().map(|(&key,value)|
        (key,value.digest.clone())).collect();
    snapshots(world,dimension,&expected)?;
    let chunks=expected.into_iter().map(|((x,z),nbt_sha256)|
        Stamp{x,z,nbt_sha256}).collect();
    let file=Journal{
        magic:MAGIC.into(),version:VERSION,source_root:root(world)?.into(),
        dimension:dimension.into(),data_version:world.data_version,chunks,edits
    };
    let bytes=serde_json::to_vec_pretty(&file)
        .map_err(|e|err(format!("Cannot serialize journal: {e}")))?;
    if bytes.len()>MAX_BYTES{return Err(err("Journal exceeds 32 MiB"));}
    Ok(bytes)
}
pub(crate) fn decode(world:&WorldSummary,dimension:&str,bytes:&[u8])
    ->Result<Vec<(Pos,State,State)>,WorldError>{
    if bytes.len()>MAX_BYTES{return Err(err("Journal exceeds 32 MiB"));}
    let journal:Journal=serde_json::from_slice(bytes)
        .map_err(|e|err(format!("Invalid edit journal: {e}")))?;
    if journal.magic!=MAGIC||journal.version!=VERSION{return Err(err("Unknown journal format/version"));}
    if journal.source_root!=root(world)?||journal.dimension!=dimension
        ||journal.data_version!=world.data_version{
        return Err(err("Journal belongs to a different world/dimension/DataVersion"));
    }
    if journal.edits.is_empty()||journal.edits.len()>MAX_CELLS
        ||journal.chunks.is_empty()||journal.chunks.len()>MAX_CHUNKS{
        return Err(err("Invalid journal edit/chunk count"));
    }
    let mut expected=BTreeMap::new();
    for stamp in journal.chunks {
        if stamp.nbt_sha256.len()!=64||!stamp.nbt_sha256.bytes().all(|b|
            b.is_ascii_digit()||(b'a'..=b'f').contains(&b)
        )||expected.insert((stamp.x,stamp.z),stamp.nbt_sha256).is_some(){
            return Err(err("Invalid or duplicate original NBT SHA-256 stamp"));
        }
    }
    let source=snapshots(world,dimension,&expected)?;
    let mut used=BTreeSet::new();
    let mut coordinates=BTreeSet::new();
    let mut result=Vec::with_capacity(journal.edits.len());
    for edit in journal.edits{
        let pos=(edit.x,edit.y,edit.z);
        if !coordinates.insert(pos){return Err(err("Duplicate edited coordinate"));}
        let key=(pos.0.div_euclid(16),pos.2.div_euclid(16));
        let chunk=&source.get(&key)
            .ok_or_else(||err("Edited chunk missing from journal source stamps"))?.chunk;
        used.insert(key);
        if chunk.has_block_entity(pos.0,pos.1,pos.2){
            return Err(err("Journal targets an original Block Entity"));
        }
        let actual=original(chunk,pos)?;
        if actual!=edit.original{return Err(err("Original journal block identity differs"));}
        valid_replacement(&edit.replacement,&chunk.layout)?;
        if actual==edit.replacement{return Err(err("Journal contains a no-op edit"));}
        result.push((pos,actual,edit.replacement));
    }
    if used.len()!=expected.len(){return Err(err("Unreferenced original chunk stamp"));}
    snapshots(world,dimension,&expected)?;
    Ok(result)
}
fn good_extension(path:&Path)->bool{
    path.extension().and_then(|x|x.to_str())==Some("mcedit2patch")
}
pub(crate) fn save(path:&Path,world:&WorldSummary,bytes:&[u8])->Result<(),WorldError>{
    if !good_extension(path){return Err(err("Use .mcedit2patch as journal extension"));}
    let parent=path.parent().filter(|p|!p.as_os_str().is_empty())
        .unwrap_or_else(||Path::new("."));
    if fs::canonicalize(parent)?.starts_with(fs::canonicalize(&world.root)?) {
        return Err(err("Journal files must be outside the original Minecraft world"));
    }
    // create_new disallows overwriting previous journal checkpoints.
    let mut handle=OpenOptions::new().write(true).create_new(true).open(path)?;
    let result=handle.write_all(bytes).and_then(|_|handle.sync_all());
    drop(handle);
    if let Err(e)=result{
        let _=fs::remove_file(path);
        return Err(e.into());
    }
    Ok(())
}
pub(crate) fn load(path:&Path)->Result<Vec<u8>,WorldError>{
    if !good_extension(path){return Err(err("Expected a .mcedit2patch journal"));}
    if fs::metadata(path)?.len()>MAX_BYTES as u64{return Err(err("Journal exceeds 32 MiB"));}
    let bytes=fs::read(path)?;
    if bytes.len()>MAX_BYTES{return Err(err("Journal exceeds 32 MiB"));}
    Ok(bytes)
}
