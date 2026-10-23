//! A bounded block-only source clipboard, not an NBT schematic converter.
//! No entity, Block Entity or opaque modded NBT is ever serialized.
use std::fs::{self,OpenOptions};
use std::io::Write;
use std::path::Path;
use serde::{Serialize,Deserialize};
use crate::editing::{parse_state,render_state};
use crate::sections::State;
use crate::{WorldError,WorldSummary};

const FORMAT:&str="MCEDIT2_RUST_BLOCK_ONLY_CLIPBOARD";
const VERSION:u32=1;
const MAX_CELLS:usize=4096;
const MAX_BYTES:usize=8*1024*1024;
fn refuse(s:impl Into<String>)->WorldError{WorldError::Invalid(s.into())}
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockClipboard {
    pub format:String,
    pub version:u32,
    pub source_root:String,
    pub dimension:String,
    pub data_version:Option<i32>,
    pub numeric:bool,
    pub source_min:[i32;3],
    pub size:[u16;3],
    // Y-major, then Z, then X; no Block Entity NBT or entities.
    pub blocks:Vec<State>,
}
impl BlockClipboard {
    pub fn validate(&self,world:&WorldSummary,dimension:&str)->Result<(),WorldError>{
        if self.format!=FORMAT||self.version!=VERSION{
            return Err(refuse("Unknown native block-only clipboard format/version"));
        }
        if self.source_root!=world.root.to_string_lossy()
            ||self.dimension!=dimension ||self.data_version!=world.data_version{
            return Err(refuse("Clipboard must match the same original world, dimension and DataVersion"));
        }
        let volume=self.size.iter().try_fold(1usize,|total,&size|{
            if size==0{return None;}
            total.checked_mul(size as usize)
        }).ok_or_else(||refuse("Clipboard dimensions invalid"))?;
        if volume>MAX_CELLS||volume!=self.blocks.len(){
            return Err(refuse("Clipboard size exceeds 4096 blocks or state count differs"));
        }
        for state in &self.blocks{
            if (self.numeric!=state.name.starts_with("numeric:"))
                ||parse_state(&render_state(state))?!=*state {
                return Err(refuse("Clipboard contains invalid, noncanonical or mismatched block states"));
            }
        }
        Ok(())
    }
}
pub(crate) fn new(world:&WorldSummary,dimension:&str,numeric:bool,
    source_min:[i32;3],size:[u16;3],blocks:Vec<State>,
)->Result<BlockClipboard,WorldError>{
    let clipboard=BlockClipboard{
        format:FORMAT.into(),version:VERSION,
        source_root:world.root.to_string_lossy().to_string(),
        dimension:dimension.into(),data_version:world.data_version,
        numeric,source_min,size,blocks,
    };
    clipboard.validate(world,dimension)?;
    Ok(clipboard)
}
pub fn save(world:&WorldSummary,path:&Path,clip:&BlockClipboard)->Result<(),WorldError>{
    clip.validate(world,&clip.dimension)?;
    if path.extension().and_then(|v|v.to_str())!=Some("mcedit2clip"){
        return Err(refuse("Clipboard must have .mcedit2clip extension"));
    }
    let parent=path.parent().filter(|p|!p.as_os_str().is_empty())
        .unwrap_or_else(||Path::new("."));
    if fs::canonicalize(parent)?.starts_with(fs::canonicalize(&world.root)?) {
        return Err(refuse("Clipboard cannot be saved inside the original world folder"));
    }
    let bytes=serde_json::to_vec_pretty(clip)
        .map_err(|e|refuse(format!("Clipboard encode failed: {e}")))?;
    if bytes.len()>MAX_BYTES{return Err(refuse("Clipboard JSON exceeds 8 MiB limit"));}
    let mut file=OpenOptions::new().create_new(true).write(true).open(path)?;
    let writing=file.write_all(&bytes).and_then(|_|file.sync_all());
    drop(file);
    if let Err(e)=writing{
        let _=fs::remove_file(path);
        return Err(e.into());
    }
    Ok(())
}
pub fn load(world:&WorldSummary,dimension:&str,path:&Path)->Result<BlockClipboard,WorldError>{
    if path.extension().and_then(|v|v.to_str())!=Some("mcedit2clip"){
        return Err(refuse("Only .mcedit2clip is accepted"));
    }
    let meta=fs::symlink_metadata(path)?;
    if !meta.is_file()||meta.file_type().is_symlink()||meta.len()>MAX_BYTES as u64{
        return Err(refuse("Clipboard must be a regular file under 8 MiB"));
    }
    let bytes=fs::read(path)?;
    if bytes.len()>MAX_BYTES{return Err(refuse("Clipboard JSON exceeds 8 MiB"));}
    let clipboard:BlockClipboard=serde_json::from_slice(&bytes)
        .map_err(|e|refuse(format!("Invalid clipboard JSON: {e}")))?;
    clipboard.validate(world,dimension)?;
    Ok(clipboard)
}
