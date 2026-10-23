//! Detached, source-verified, read-only cross-Region preflight manifest.
//! Preparing many verified Region candidates is NOT a multi-file atomic write.
//! This module only authenticates and inventories those candidates.
use std::collections::BTreeSet;
use std::fs::{self,OpenOptions};
use std::io::Write;
use std::path::Path;
use serde::{Serialize,Deserialize};
use sha2::{Digest,Sha256};
use crate::editing::Session;
use crate::{WorldError,WorldSummary};
fn refuse(s:impl Into<String>)->WorldError{WorldError::Invalid(s.into())}
fn digest(data:&[u8])->String{
    Sha256::digest(data).iter().map(|b|format!("{b:02x}")).collect()
}
#[derive(Debug,Clone,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct RegionDryRunEntry{
    pub path:String,
    pub region_x:i32,pub region_z:i32,
    pub edited_chunks:usize,
    pub be_scalar_steps:usize,
    pub original_bytes:usize,
    pub planned_bytes:usize,
    pub original_sha256:String,
    pub planned_sha256:String,
}
#[derive(Debug,Clone,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct RegionDryRun{
    pub version:u32,
    pub original_world_root:String,
    pub dimension:String,
    pub data_version:Option<i32>,
    pub timestamp:u32,
    pub regions:Vec<RegionDryRunEntry>,
}
pub fn prepare(session:&Session,timestamp:u32)->Result<RegionDryRun,WorldError>{
    let world=session.original_world();
    let proposals=if session.pending_be_chunks()>0{
        session.preview_be_region_candidates(timestamp)?
    }else{
        session.prepare_native_region_candidates(timestamp)?
    };
    if proposals.is_empty()||proposals.len()>256{
        return Err(refuse("Region manifest requires 1..256 existing region files"));
    }
    let root=fs::canonicalize(&world.root)?;
    let mut seen=BTreeSet::new();
    let mut regions=Vec::with_capacity(proposals.len());
    for proposed in proposals{
        let info=fs::symlink_metadata(&proposed.path)?;
        if !info.file_type().is_file()||info.file_type().is_symlink(){
            return Err(refuse("Region dry run source is not a regular file"));
        }
        let canonical=fs::canonicalize(&proposed.path)?;
        if !canonical.starts_with(&root)||!seen.insert(canonical.clone()){
            return Err(refuse("Region manifest contains duplicate/external source files"));
        }
        let bytes=fs::read(&proposed.path)?;
        if digest(&bytes)!=proposed.original_region_sha256
            ||bytes.len()!=proposed.original_bytes
            ||proposed.image.len()!=proposed.planned_bytes{
            return Err(refuse("Region original source changed during batch preflight"));
        }
        regions.push(RegionDryRunEntry{
            path:canonical.to_string_lossy().into_owned(),
            region_x:proposed.region_x,region_z:proposed.region_z,
            edited_chunks:proposed.modified_chunks,
            be_scalar_steps:proposed.be_replay.len(),
            original_bytes:proposed.original_bytes,
            planned_bytes:proposed.planned_bytes,
            original_sha256:proposed.original_region_sha256,
            planned_sha256:digest(&proposed.image),
        });
    }
    Ok(RegionDryRun{
        version:1,
        original_world_root:root.to_string_lossy().into_owned(),
        dimension:session.dimension().into(),
        data_version:world.data_version,
        timestamp,regions,
    })
}
fn check_output_path(world:&WorldSummary,path:&Path)->Result<(),WorldError>{
    if path.file_name().and_then(|s|s.to_str())
        .is_none_or(|s|!s.ends_with(".mcedit2regionplan.json")){
        return Err(refuse("Region plan needs .mcedit2regionplan.json extension"));
    }
    let parent=path.parent().filter(|p|!p.as_os_str().is_empty())
        .unwrap_or_else(||Path::new("."));
    if fs::canonicalize(parent)?.starts_with(fs::canonicalize(&world.root)?){
        return Err(refuse("Region dry-run manifest must be outside Minecraft world"));
    }
    Ok(())
}
pub fn export(session:&Session,timestamp:u32,path:&Path)->Result<usize,WorldError>{
    let report=prepare(session,timestamp)?;
    check_output_path(session.original_world(),path)?;
    let bytes=serde_json::to_vec_pretty(&report)
        .map_err(|e|refuse(format!("Region plan JSON serialization failed: {e}")))?;
    if bytes.len()>128*1024{return Err(refuse("Region plan manifest exceeds 128 KiB"));}
    let mut file=OpenOptions::new().create_new(true).write(true).open(path)?;
    let status=file.write_all(&bytes).and_then(|_|file.sync_all());
    drop(file);
    if let Err(e)=status{
        let _=fs::remove_file(path);
        return Err(e.into());
    }
    Ok(report.regions.len())
}
pub fn verify(session:&Session,path:&Path)->Result<usize,WorldError>{
    check_output_path(session.original_world(),path)?;
    let stat=fs::symlink_metadata(path)?;
    if !stat.file_type().is_file()||stat.file_type().is_symlink()
        ||stat.len()>128*1024{
        return Err(refuse("Region plan manifest is not a bounded regular file"));
    }
    let previous:RegionDryRun=serde_json::from_slice(&fs::read(path)?)
        .map_err(|e|refuse(format!("Region plan JSON malformed: {e}")))?;
    if previous.version!=1{
        return Err(refuse("Unsupported Region plan manifest version"));
    }
    let actual=prepare(session,previous.timestamp)?;
    if previous!=actual{
        return Err(refuse("Region plan no longer equals exact source and staged edits"));
    }
    Ok(actual.regions.len())
}
