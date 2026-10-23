//! Fixed-budget original-state 3D surface projection of one Java footprint.
//! A top visible block from each column is represented as a voxel, with an
//! original BE marker. Air, missing sections and missing chunks are never
//! silently replaced with generated Minecraft data.
use std::collections::BTreeMap;
use crate::block_lookup::read_world_chunk;
use crate::editing::Session;
use crate::sections::{Chunk,State};
use crate::{WorldError,WorldSummary};

fn refuse(s:impl Into<String>)->WorldError{WorldError::Invalid(s.into())}
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct SurfaceVoxel {
    pub x:i32,pub y:i32,pub z:i32,
    pub state:State,
    pub original_block_entity:bool,
}
#[derive(Debug,Clone)]
pub struct SurfaceView {
    pub x:i32,pub z:i32,
    pub y_min:i32,pub y_max:i32,
    pub width:u8,pub depth:u8,
    pub voxels:Vec<SurfaceVoxel>,
    pub missing_columns:usize,
    pub source_chunks:usize,
}
fn is_air(state:&State)->bool{
    matches!(state.name.as_str(),
        "minecraft:air"|"minecraft:cave_air"|"minecraft:void_air"|
        "numeric:0")
}
/// Render a true-height voxel column surface from the current in-memory
/// overlay. Bounded to 16x16 footprint by 32 Y levels (8192 reads, but
/// each original chunk is decoded only once). No generation or disk write.
pub fn inspect_surface(
    session:&Session,x:i32,z:i32,y_min:i32,y_max:i32,
    width:u8,depth:u8,
)->Result<SurfaceView,WorldError>{
    if !(1..=16).contains(&width)||!(1..=16).contains(&depth)
        ||y_min>y_max||y_max as i64-y_min as i64>=32{
        return Err(refuse("3D surface must fit 16x16 X/Z and <=32 Y layers"));
    }
    let far_x=x.checked_add(width as i32-1)
        .ok_or_else(||refuse("Surface X overflow"))?;
    let far_z=z.checked_add(depth as i32-1)
        .ok_or_else(||refuse("Surface Z overflow"))?;
    // Bounds::new enforces the EDIT batch's 4096-cell limit, while this
    // read-only 3D inspector legitimately scans up to 8192 cells. Validate
    // the two extrema as single-cell coordinates instead.
    let _=crate::editing::Bounds::new((x,y_min,z),(x,y_min,z))?;
    let _=crate::editing::Bounds::new((far_x,y_max,far_z),(far_x,y_max,far_z))?;
    let mut cache:BTreeMap<(i32,i32),Option<Chunk>>=BTreeMap::new();
    let mut voxels=Vec::new();
    let mut missing_columns=0;
    for dz in 0..depth as i32{
        for dx in 0..width as i32{
            let (wx,wz)=(x+dx,z+dz);
            let key=(wx.div_euclid(16),wz.div_euclid(16));
            if !cache.contains_key(&key){
                cache.insert(key,read_world_chunk(
                    session.original_world(),session.dimension(),key.0,key.1
                )?);
            }
            let Some(chunk)=cache.get(&key).and_then(Option::as_ref) else{
                missing_columns+=1;continue;
            };
            let mut chosen=None;
            let mut fallback=None;
            for y in (y_min..=y_max).rev(){
                let Some(original)=chunk.block(wx.rem_euclid(16) as u8,y,
                    wz.rem_euclid(16) as u8) else{continue};
                let state=session.visible_block((wx,y,wz))?;
                let state=match state{
                    crate::block_lookup::Lookup::Present(details)=>details.state,
                    _=>return Err(refuse("Original Section changed during surface inspection")),
                };
                let voxel=SurfaceVoxel{
                    x:wx,y,z:wz,state,
                    original_block_entity:chunk.has_block_entity(wx,y,wz),
                };
                if fallback.is_none(){fallback=Some(voxel.clone());}
                if !is_air(&voxel.state){
                    chosen=Some(voxel);break;
                }
                let _=original;
            }
            if let Some(voxel)=chosen.or(fallback){
                voxels.push(voxel);
            }else{missing_columns+=1;}
        }
    }
    Ok(SurfaceView{
        x,z,y_min,y_max,width,depth,voxels,missing_columns,
        source_chunks:cache.len(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn air_detection_is_explicit_not_derived_from_arbitrary_modded_name(){
        for name in ["minecraft:air","minecraft:cave_air","minecraft:void_air","numeric:0"]{
            assert!(is_air(&State{name:name.into(),properties:vec![]}));
        }
        assert!(!is_air(&State{name:"mod:custom_air".into(),properties:vec![]}));
    }
}
