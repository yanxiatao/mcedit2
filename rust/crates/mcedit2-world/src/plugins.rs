//! Versioned, declarative safe-plugin foundation for the Rust rewrite.
//! No arbitrary executable, Python, DLL, Wasm, file/network or shell access.
//! The first safe operation delegates to the existing original NBT checked
//! and Undo-able Region edit preview: one bounded conditional replacement.
use std::fs;
use std::path::Path;
use serde::Deserialize;
use crate::editing::{Bounds,Preview,Session,parse_state};
use crate::{WorldError,WorldSummary};

fn refuse(s:impl Into<String>)->WorldError{WorldError::Invalid(s.into())}
#[derive(Debug,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest{
    pub schema_version:u32,
    pub name:String,
    pub description:Option<String>,
    pub only_data_version:Option<i32>,
    pub only_dimension:Option<String>,
    pub operation:PluginOperation,
}
#[derive(Debug,Deserialize)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum PluginOperation{
    ReplaceBlocks{
        minimum:[i32;3],
        maximum:[i32;3],
        with_state:String,
        only_if_state:Option<String>,
    }
}
pub fn load(path:&Path)->Result<PluginManifest,WorldError>{
    if !path.file_name().and_then(|s|s.to_str())
        .is_some_and(|s|s.ends_with(".mcedit2plugin.json")){
        return Err(refuse("Only .mcedit2plugin.json manifests are accepted"));
    }
    let info=fs::symlink_metadata(path)?;
    if !info.file_type().is_file()||info.file_type().is_symlink()
        ||info.len()>32*1024{
        return Err(refuse("Plugin manifest must be a small regular file <=32 KiB"));
    }
    let manifest:PluginManifest=serde_json::from_slice(&fs::read(path)?)
        .map_err(|e|refuse(format!("Invalid declarative plugin JSON: {e}")))?;
    if manifest.schema_version!=1||manifest.name.is_empty()
        ||manifest.name.len()>96||manifest.name.contains('\0')
        ||manifest.description.as_ref().is_some_and(|v|v.len()>1024){
        return Err(refuse("Unsupported plugin version or invalid metadata"));
    }
    Ok(manifest)
}
impl PluginManifest{
    pub fn prepare(&self,session:&Session)->Result<Preview,WorldError>{
        if let Some(dv)=self.only_data_version{
            if session.original_world().data_version!=Some(dv){
                return Err(refuse("Plugin requires an exact original DataVersion"));
            }
        }
        if self.only_dimension.as_ref()
            .is_some_and(|d|d!=session.dimension()){
            return Err(refuse("Plugin dimension does not match the original world"));
        }
        match &self.operation{
            PluginOperation::ReplaceBlocks{minimum,maximum,with_state,only_if_state}=>{
                let start=(minimum[0],minimum[1],minimum[2]);
                let end=(maximum[0],maximum[1],maximum[2]);
                let bounds=Bounds::new(start,end)?;
                let target=parse_state(with_state)?;
                let filter=only_if_state.as_ref().map(|x|parse_state(x))
                    .transpose()?;
                session.prepare(bounds,&target,filter.as_ref())
            },
        }
    }
}
pub fn prepare_file(session:&Session,path:&Path)->Result<Preview,WorldError>{
    load(path)?.prepare(session)
}
