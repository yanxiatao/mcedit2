//! Fail-closed batch actions for existing opaque Block Entity NBT scalars.
//! Apply each action to a cloned Session and adopt it only after all actions
//! validate. A failed action never leaves a partly modified editing overlay.
use std::fs;
use std::path::Path;
use serde::Deserialize;
use crate::editing::Session;
use crate::{WorldError,WorldSummary};
fn refuse(s:impl Into<String>)->WorldError{WorldError::Invalid(s.into())}
#[derive(Debug,Clone,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeOperation {
    pub xyz:[i32;3],
    pub field:String,
    pub value:String,
}
#[derive(Debug,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeOperationBatch{
    pub version:u32,
    pub original_world_root:String,
    pub dimension:String,
    pub data_version:Option<i32>,
    pub operations:Vec<BeOperation>,
}
pub fn read_batch(world:&WorldSummary,dimension:&str,path:&Path)
    ->Result<BeOperationBatch,WorldError>{
    if path.file_name().and_then(|x|x.to_str())
        .is_none_or(|x|!x.ends_with(".mcedit2beops.json")){
        return Err(refuse("BE operations need .mcedit2beops.json extension"));
    }
    let stat=fs::symlink_metadata(path)?;
    if !stat.file_type().is_file()||stat.file_type().is_symlink()
        ||stat.len()>65536{
        return Err(refuse("BE operations must be regular <=64KiB file"));
    }
    let batch:BeOperationBatch=serde_json::from_slice(&fs::read(path)?)
        .map_err(|e|refuse(format!("BE operation manifest JSON invalid: {e}")))?;
    if batch.version!=1
        ||batch.original_world_root!=world.root.to_string_lossy()
        ||batch.dimension!=dimension
        ||batch.data_version!=world.data_version
        ||batch.operations.is_empty()||batch.operations.len()>24{
        return Err(refuse("BE operation file identity or operation count invalid"));
    }
    if batch.operations.iter().any(|x|x.field.len()>128||x.value.len()>4096){
        return Err(refuse("BE operation field/value exceeds scalar budget"));
    }
    Ok(batch)
}
fn apply_on_clone(source:&Session,operations:&[BeOperation])
    ->Result<Session,WorldError>{
    if operations.is_empty()||operations.len()>24{
        return Err(refuse("BE scalar batch must contain 1..24 operations"));
    }
    let mut staged=source.clone();
    for step in operations{
        let point=(step.xyz[0],step.xyz[1],step.xyz[2]);
        let prepared=staged.prepare_be_edit(point,&step.field,&step.value)?;
        staged.commit_be_edit(prepared)?;
    }
    Ok(staged)
}
/// Preview an entire batch without mutating even one BE in the source.
pub fn preview_file(source:&Session,path:&Path)->Result<(usize,usize),WorldError>{
    let batch=read_batch(source.original_world(),source.dimension(),path)?;
    let staged=apply_on_clone(source,&batch.operations)?;
    Ok((batch.operations.len(),staged.pending_be_chunks()))
}
/// Full all-or-nothing batch staging, retaining each original scalar in the
/// existing separate BE Undo/Redo stack.
pub fn stage_file(source:&mut Session,path:&Path)->Result<usize,WorldError>{
    let batch=read_batch(source.original_world(),source.dimension(),path)?;
    let staged=apply_on_clone(source,&batch.operations)?;
    *source=staged;
    Ok(batch.operations.len())
}
