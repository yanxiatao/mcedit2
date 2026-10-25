//! Versioned, declarative safe-plugin foundation for the Rust rewrite.
//! No arbitrary executable, Python, DLL, Wasm, file/network or shell access.
//! The first safe operation delegates to the existing original NBT checked
//! and Undo-able Region edit preview: one bounded conditional replacement.
use std::fs;
use std::path::Path;
use serde::Deserialize;
use crate::editing::{Bounds,BrushShape,Preview,Session,parse_state};
use crate::WorldError;

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
#[derive(Debug,Clone,Deserialize)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum PluginOperation{
    ReplaceBlocks{
        minimum:[i32;3],
        maximum:[i32;3],
        with_state:String,
        only_if_state:Option<String>,
    },
    Brush{
        minimum:[i32;3],
        maximum:[i32;3],
        shape:PluginBrush,
        with_state:String,
        only_if_state:Option<String>,
    },
}
#[derive(Debug,Clone,Copy,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum PluginBrush{
    HollowBox,Ellipsoid,EllipsoidShell,CylinderY,CylinderShellY,
}
impl PluginBrush{
    pub fn shape(self)->BrushShape{
        match self{
            Self::HollowBox=>BrushShape::HollowBox,
            Self::Ellipsoid=>BrushShape::Ellipsoid,
            Self::EllipsoidShell=>BrushShape::EllipsoidShell,
            Self::CylinderY=>BrushShape::CylinderY,
            Self::CylinderShellY=>BrushShape::CylinderShellY,
        }
    }
}
#[derive(Debug,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifestV2 {
    pub schema_version:u32,
    pub name:String,
    pub description:Option<String>,
    pub only_data_version:Option<i32>,
    pub only_dimension:Option<String>,
    pub operations:Vec<PluginOperation>,
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
                prepare_operation(session,*minimum,*maximum,with_state,only_if_state,None)
            },
            PluginOperation::Brush{minimum,maximum,shape,with_state,only_if_state}=>{
                prepare_operation(session,*minimum,*maximum,with_state,only_if_state,Some(shape.shape()))
            }
        }
    }
}

fn prepare_operation(
    session:&Session,minimum:[i32;3],maximum:[i32;3],
    state:&str,filter:&Option<String>,brush:Option<BrushShape>,
)->Result<Preview,WorldError>{
    let bounds=Bounds::new((minimum[0],minimum[1],minimum[2]),
        (maximum[0],maximum[1],maximum[2]))?;
    let target=parse_state(state)?;
    let required=filter.as_ref().map(|s|parse_state(s)).transpose()?;
    match brush{
        Some(shape)=>session.prepare_brush(bounds,shape,&target,required.as_ref()),
        None=>session.prepare(bounds,&target,required.as_ref()),
    }
}
pub(crate) fn preview_op(session:&Session,operation:&PluginOperation)
    ->Result<Preview,WorldError>{
    match operation{
        PluginOperation::ReplaceBlocks{minimum,maximum,with_state,only_if_state}=>
            prepare_operation(session,*minimum,*maximum,with_state,only_if_state,None),
        PluginOperation::Brush{minimum,maximum,shape,with_state,only_if_state}=>
            prepare_operation(session,*minimum,*maximum,with_state,only_if_state,Some(shape.shape())),
    }
}
fn manifest_bytes(path:&Path)->Result<Vec<u8>,WorldError>{
    if !path.file_name().and_then(|s|s.to_str())
        .is_some_and(|s|s.ends_with(".mcedit2plugin.json")){
        return Err(refuse("Only .mcedit2plugin.json manifests accepted"));
    }
    let meta=fs::symlink_metadata(path)?;
    if !meta.file_type().is_file()||meta.file_type().is_symlink()
        ||meta.len()>32*1024{
        return Err(refuse("Plugin must be a regular file of <=32 KiB"));
    }
    Ok(fs::read(path)?)
}
fn valid_meta(name:&str,description:Option<&str>)->bool{
    !name.is_empty()&&name.len()<=96&&!name.contains('\0')
        &&description.is_none_or(|s|s.len()<=1024)
}
impl PluginManifestV2{
    pub fn prepare(&self,session:&Session)->Result<Preview,WorldError>{
        if self.schema_version!=2||!valid_meta(&self.name,self.description.as_deref())
            ||self.operations.is_empty()||self.operations.len()>24{
            return Err(refuse("Invalid version-2 plugin metadata or operation count"));
        }
        if self.only_data_version.is_some_and(|dv|
            session.original_world().data_version!=Some(dv)){
            return Err(refuse("Plugin requires exact source DataVersion"));
        }
        if self.only_dimension.as_deref().is_some_and(|d|d!=session.dimension()){
            return Err(refuse("Plugin requires matching original dimension"));
        }
        session.prepare_plugin_sequence(&self.operations)
    }
}
pub fn prepare_file(session:&Session,path:&Path)->Result<Preview,WorldError>{
    let bytes=manifest_bytes(path)?;
    let base:serde_json::Value=serde_json::from_slice(&bytes)
        .map_err(|e|refuse(format!("Invalid plugin JSON: {e}")))?;
    match base.get("schema_version").and_then(|s|s.as_u64()){
        Some(1)=>load(path)?.prepare(session),
        Some(2)=>{
            let manifest:PluginManifestV2=serde_json::from_slice(&bytes)
                .map_err(|e|refuse(format!("Invalid v2 plugin schema: {e}")))?;
            manifest.prepare(session)
        },
        _=>Err(refuse("Unsupported declarative plugin schema version")),
    }
}
