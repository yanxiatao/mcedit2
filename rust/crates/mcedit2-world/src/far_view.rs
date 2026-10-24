//! Progressive, read-only distant terrain preview: original Java chunks only.
//! The surface is NOT an editor selection; distant samples are approximate,
//! never used as edit targets or NBT provenance. No entity renderer is called.
use std::collections::BTreeSet;
use crate::block_lookup::read_world_chunk;
use crate::editing::{Bounds,Session};
use crate::surface::{
    SurfaceView,SurfaceVoxel,SourceChunkTile,ALL_FACES,material,
};
use crate::{WorldError};

pub const MAX_DISTANCE_CHUNKS:u16=1024;
pub const FAR_BATCH_CHUNKS:usize=12;
pub const MAX_PLANNED_CHUNKS:usize=36_000;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct FarChunkRequest{
    pub cx:i32,pub cz:i32,
    /// 1=full sampled column density; 4/8/16 are visual-only terrain LODs.
    pub span:u8,
}
#[derive(Clone,Debug)]
pub struct FarChunkPatch{
    pub tile:SourceChunkTile,
    pub voxels:Vec<SurfaceVoxel>,
}
fn reject(reason:&str)->WorldError{WorldError::Invalid(reason.to_string())}
fn lod_for(radius:i32)->u8{
    if radius<=8{1}else if radius<=16{4}else if radius<=48{8}else{16}
}
fn validate_height(y_min:i32,y_max:i32)->Result<(),WorldError>{
    if y_min>y_max||y_max as i64-y_min as i64+1>512{
        return Err(reject("Far 3D terrain height must fit 1..512 source levels"));
    }
    Ok(())
}
fn extrema(center_x:i32,center_z:i32,radius:u16)->Result<(i32,i32,u16),WorldError>{
    if radius==0||radius>MAX_DISTANCE_CHUNKS{
        return Err(reject("Far preview radius must be 1..1024 source chunks"));
    }
    let span=((radius as i64*2)+1)*16;
    let xmin=(center_x.div_euclid(16) as i64-radius as i64)*16;
    let zmin=(center_z.div_euclid(16) as i64-radius as i64)*16;
    let xmax=xmin+span-1;let zmax=zmin+span-1;
    let convert=|n:i64|i32::try_from(n).map_err(|_|reject("Far view coordinate overflow"));
    let x0=convert(xmin)?;let z0=convert(zmin)?;
    let _=Bounds::new((x0,0,z0),(x0,0,z0))?;
    let _=Bounds::new((convert(xmax)?,0,convert(zmax)?),
        (convert(xmax)?,0,convert(zmax)?))?;
    Ok((x0,z0,span as u16))
}
/// Camera-centric concentric source-chunk rings, including outermost
/// horizons. Above 8 chunks, sparsely sample chunks rather than scanning
/// millions of full-resolution blocks. All returned chunk positions are
/// original storage coordinates, deduplicated and ordered nearest first.
pub fn plan_far_chunks(center_x:i32,center_z:i32,radius:u16)
    ->Result<Vec<FarChunkRequest>,WorldError>{
    let _=extrema(center_x,center_z,radius)?;
    let cx=center_x.div_euclid(16);let cz=center_z.div_euclid(16);
    let mut seen=BTreeSet::new();
    let mut out=Vec::new();
    for ring in 0..=radius as i32{
        let step=if ring<=8{1}else if ring<=16{2}
            else if ring<=48{4}else if ring<=128{8}
            else if ring<=256{32}else if ring<=512{128}else{512};
        let mut along=Vec::new();
        let mut n=-ring;
        while n<=ring{
            along.push(n);
            n+=step;
        }
        if along.last()!=Some(&ring){along.push(ring);}
        for n in along{
            let positions=if ring==0{vec![(0,0)]}else{
                vec![(n,-ring),(n,ring),(-ring,n),(ring,n)]
            };
            for (dx,dz) in positions{
                let key=(cx+dx,cz+dz);
                // Do not schedule original chunks beyond the Java border.
                let wx=key.0 as i64*16;
                let wz=key.1 as i64*16;
                if wx < -30_000_000||wx+15 >30_000_000
                    ||wz < -30_000_000||wz+15>30_000_000{
                    continue;
                }
                if seen.insert(key){
                    out.push(FarChunkRequest{cx:key.0,cz:key.1,
                        span:lod_for(ring)});
                    if out.len()>MAX_PLANNED_CHUNKS{
                        return Err(reject("Far chunk plan exceeds read-only memory budget"));
                    }
                }
            }
        }
    }
    Ok(out)
}
pub fn empty_far_view(center_x:i32,center_z:i32,
    y_min:i32,y_max:i32,radius:u16)->Result<SurfaceView,WorldError>{
    validate_height(y_min,y_max)?;
    let (x,z,side)=extrema(center_x,center_z,radius)?;
    let _=Bounds::new((x,y_min,z),(x,y_min,z))?;
    Ok(SurfaceView{
        x,z,y_min,y_max,width:side,depth:side,
        voxels:Vec::new(),missing_columns:0,
        source_chunks:0,source_tiles:Vec::new()
    })
}
/// Read a SINGLE verified source chunk. LOD spans cover a visual tile,
/// sampling exactly one original column at its center. They are never
/// passed to editing APIs, nor allowed to masquerade as exact picks.
pub fn inspect_far_chunk(session:&Session,request:FarChunkRequest,
    y_min:i32,y_max:i32)->Result<FarChunkPatch,WorldError>{
    validate_height(y_min,y_max)?;
    if !matches!(request.span,1|4|8|16){
        return Err(reject("Far preview LOD must be 1,4,8 or 16"));
    }
    let x=i32::try_from(request.cx as i64*16)
        .map_err(|_|reject("Source chunk X overflow"))?;
    let z=i32::try_from(request.cz as i64*16)
        .map_err(|_|reject("Source chunk Z overflow"))?;
    let _=Bounds::new((x,y_min,z),(x,y_min,z))?;
    let _=Bounds::new((x+15,y_max,z+15),(x+15,y_max,z+15))?;
    let mut tile=SourceChunkTile{chunk_x:request.cx,chunk_z:request.cz,
        source_present:false,known_columns:0,
        unknown_columns:0,exposed_voxels:0};
    let Some(chunk)=read_world_chunk(session.original_world(),session.dimension(),
        request.cx,request.cz)? else {
        tile.unknown_columns=256/(request.span as usize*request.span as usize);
        return Ok(FarChunkPatch{tile,voxels:Vec::new()});
    };
    tile.source_present=true;
    // Traverse only sections that actually exist, highest first: do not
    // inspect 384 heights through the full 128-section linear search
    // for each original block position.
    let mut sections:Vec<_>=chunk.sections.iter().filter(|section|{
        let start=section.y as i32*16;
        start<=y_max&&start+15>=y_min
    }).collect();
    sections.sort_unstable_by(|a,b|b.y.cmp(&a.y));
    let stride=request.span as i32;
    let mut voxels=Vec::new();
    for dz in (0..16).step_by(stride as usize){
        for dx in (0..16).step_by(stride as usize){
            let sx=(dx+stride/2).min(15);
            let sz=(dz+stride/2).min(15);
            let mut known=false;
            let mut chosen=None;
            'height: for section in &sections{
                let y0=y_min.max(section.y as i32*16);
                let y1=y_max.min(section.y as i32*16+15);
                for y in (y0..=y1).rev(){
                    let Some(original)=section.block(sx as u8,y.rem_euclid(16) as u8,
                        sz as u8)else{continue};
                    known=true;
                    let wx=x+sx;let wz=z+sz;
                    let state=session.effective_state((wx,y,wz),original);
                    if matches!(state.name.as_str(),
                        "minecraft:air"|"minecraft:cave_air"|"minecraft:void_air"|"numeric:0"){
                        continue;
                    }
                    chosen=Some((y,state));
                    break 'height;
                }
            }
            if known{tile.known_columns+=1;}
            else{tile.unknown_columns+=1;}
            if let Some((y,state))=chosen{
                let wx=x+dx;let wz=z+dz;
                let kind=material(&state);
                let exact=request.span==1;
                voxels.push(SurfaceVoxel{
                    x:wx,y,z:wz,state,
                    original_block_entity:exact&&chunk.has_block_entity(wx,y,wz),
                    faces:ALL_FACES,kind,
                    staged:exact&&session.is_staged_block((wx,y,wz)),
                    span:request.span,
                });
            }
        }
    }
    tile.exposed_voxels=voxels.len();
    Ok(FarChunkPatch{tile,voxels})
}
pub fn append_chunk(view:&mut SurfaceView,patch:FarChunkPatch){
    view.source_chunks+=1;
    view.missing_columns+=patch.tile.unknown_columns;
    view.source_tiles.push(patch.tile);
    view.voxels.extend(patch.voxels);
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn far_view_plans_distant_horizon_without_scanning_every_cell(){
        let p=plan_far_chunks(0,0,128).unwrap();
        assert!(!p.is_empty());
        assert!(p.len()<=MAX_PLANNED_CHUNKS);
        assert_eq!(p[0],FarChunkRequest{cx:0,cz:0,span:1});
        assert!(p.iter().any(|r|r.cx.abs()==128||r.cz.abs()==128));
        assert!(p.iter().any(|r|r.span==16));
        let unique:std::collections::BTreeSet<_>=p.iter()
            .map(|r|(r.cx,r.cz)).collect();
        assert_eq!(unique.len(),p.len());
        let view=empty_far_view(0,0,-64,-17,128).unwrap();
        assert_eq!(view.width,4112);
        assert_eq!(view.depth,4112);
        assert!(view.voxels.is_empty());
        assert!(empty_far_view(0,0,-64,447,128).is_ok());
        assert!(empty_far_view(0,0,-64,448,128).is_err());
        let extreme=plan_far_chunks(0,0,1024).unwrap();
        assert!(extreme.iter().any(|r|r.cx.abs()==1024||r.cz.abs()==1024));
        assert!(extreme.len()<=MAX_PLANNED_CHUNKS);
        let view=empty_far_view(0,0,-64,255,1024).unwrap();
        assert_eq!(view.width,32784);
        assert_eq!(view.depth,32784);
        assert!(plan_far_chunks(0,0,1025).is_err());
    }
    #[test]
    fn far_view_negative_coordinates_use_floor_chunk_grid(){
        let p=plan_far_chunks(-17,-31,16).unwrap();
        assert_eq!((p[0].cx,p[0].cz),(-2,-2));
    }
}
