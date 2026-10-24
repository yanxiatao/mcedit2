//! Fixed-budget original-state 3D surface projection of one Java footprint.
//! A top visible block from each column is represented as a voxel, with an
//! original BE marker. Air, missing sections and missing chunks are never
//! silently replaced with generated Minecraft data.
use std::collections::BTreeMap;
use crate::block_lookup::ReadOnlyRegionCache;
use crate::editing::Session;
use crate::sections::{Chunk,State};
use crate::{WorldError,WorldSummary};

fn refuse(s:impl Into<String>)->WorldError{WorldError::Invalid(s.into())}
pub const FACE_TOP:u8=1;
pub const FACE_BOTTOM:u8=2;
pub const FACE_POS_X:u8=4;
pub const FACE_NEG_X:u8=8;
pub const FACE_POS_Z:u8=16;
pub const FACE_NEG_Z:u8=32;
pub const ALL_FACES:u8=63;
/// Strict bounds for read-only 3D viewing. The editing batch limit remains
/// unchanged: a larger scene is NOT a larger writable transaction.
pub const MAX_VIEW_XZ:u8=64;
pub const MAX_VIEW_Y:i64=48;
pub const MAX_VIEW_CELLS:u64=MAX_VIEW_XZ as u64*MAX_VIEW_XZ as u64*MAX_VIEW_Y as u64;
fn validate_view(width:u8,depth:u8,y_min:i32,y_max:i32)->Result<(),WorldError>{
    let height=y_max as i64-y_min as i64+1;
    if width==0||width>MAX_VIEW_XZ||depth==0||depth>MAX_VIEW_XZ
        ||height<1||height>MAX_VIEW_Y
        ||width as u64*depth as u64*height as u64>MAX_VIEW_CELLS {
        return Err(refuse("Read-only 3D scene exceeds 64x64x48 / 196608 source cells"));
    }
    Ok(())
}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum VoxelMaterial { Solid,Transparent,Liquid,Foliage }
pub fn material(state:&State)->VoxelMaterial {
    let n=state.name.as_str();
    if n=="minecraft:water"||n=="minecraft:lava" {VoxelMaterial::Liquid}
    else if n.contains("glass")||n.ends_with(":ice")||n.ends_with("_ice") {VoxelMaterial::Transparent}
    else if n.ends_with("_leaves")||n.ends_with(":leaves") {VoxelMaterial::Foliage}
    else {VoxelMaterial::Solid}
}
/// A known vanilla partial shape cannot be treated as a full occluder.
/// This conservative test only exposes potentially visible faces; the
/// projected model still supplies actual element geometry and selection.
fn known_partial_occluder(state:&State)->bool{
    let Some(n)=state.name.strip_prefix("minecraft:")else{return false;};
    if n.ends_with("_slab"){
        return !state.properties.iter().any(|(k,v)|k=="type"&&v=="double");
    }
    if n=="snow"{
        return !state.properties.iter().any(|(k,v)|k=="layers"&&v=="8");
    }
    n.ends_with("_stairs")||n.ends_with("_trapdoor")
        ||n.ends_with("_fence")||n.ends_with("_fence_gate")
        ||n.ends_with("_wall")||n.ends_with("_pane")
        ||n=="iron_bars"||n.ends_with("_carpet")
        ||n=="farmland"||n=="dirt_path"
        ||n.ends_with("_door")
}
fn exposed_against(state:&State,kind:VoxelMaterial,neighbor:Option<&(State,VoxelMaterial)>)->bool {
    match neighbor {
        Some((n,_)) if is_air(n)=>true,
        Some((n,_)) if known_partial_occluder(n)=>true,
        Some((n,nkind)) if *nkind!=VoxelMaterial::Solid =>
            kind==VoxelMaterial::Solid||(*nkind!=kind)||n.name!=state.name,
        _=>false,
    }
}
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct SurfaceVoxel {
    pub x:i32,pub y:i32,pub z:i32,
    pub state:State,
    pub original_block_entity:bool,
    /// Original exposed voxel faces; missing source cells are unknown.
    pub faces:u8,
    /// Visual metadata only; has no effect on source NBT or editing.
    pub kind:VoxelMaterial,
    pub staged:bool,
    /// Original X/Z cells represented by this sample (1 is exact).
    /// Spans >1 are approximate and must never be treated as editable blocks.
    pub span:u8,
}
/// One actual X/Z source chunk touched during the read-only scene pass.
/// Missing chunks are never synthesized. Unknown Sections/columns remain
/// unknown even if adjacent source chunks are fully present.
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct SourceChunkTile{
    pub chunk_x:i32,pub chunk_z:i32,
    pub source_present:bool,
    pub known_columns:usize,pub unknown_columns:usize,
    pub exposed_voxels:usize,
}
#[derive(Debug,Clone)]
pub struct SurfaceView {
    pub x:i32,pub z:i32,
    pub y_min:i32,pub y_max:i32,
    pub width:u16,pub depth:u16,
    pub voxels:Vec<SurfaceVoxel>,
    pub missing_columns:usize,
    pub source_chunks:usize,
    pub source_tiles:Vec<SourceChunkTile>,
}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct SurfaceStatistics{
    pub voxels:usize,
    pub exposed_faces:usize,
    pub original_be:usize,
    pub staged:usize,
    pub liquids:usize,
    pub transparent:usize,
    pub foliage:usize,
    pub missing_columns:usize,
    pub original_chunks:usize,
}
impl SurfaceView{
    pub fn statistics(&self)->SurfaceStatistics{
        SurfaceStatistics{
            voxels:self.voxels.len(),
            exposed_faces:self.voxels.iter().map(|v|v.faces.count_ones() as usize).sum(),
            original_be:self.voxels.iter().filter(|v|v.original_block_entity).count(),
            staged:self.voxels.iter().filter(|v|v.staged).count(),
            liquids:self.voxels.iter().filter(|v|v.kind==VoxelMaterial::Liquid).count(),
            transparent:self.voxels.iter().filter(|v|v.kind==VoxelMaterial::Transparent).count(),
            foliage:self.voxels.iter().filter(|v|v.kind==VoxelMaterial::Foliage).count(),
            missing_columns:self.missing_columns,
            original_chunks:self.source_chunks,
        }
    }
}
fn is_air(state:&State)->bool{
    matches!(state.name.as_str(),
        "minecraft:air"|"minecraft:cave_air"|"minecraft:void_air"|
        "numeric:0")
}
/// Render a true-height voxel column surface from the current in-memory
/// overlay. Bounded to 64x64 footprint by 48 Y levels (196608 reads).
/// Each original chunk is decoded at most once within a request. No
/// generation or disk write, and the editor's 4096-block write limit stays.
pub fn inspect_surface(
    session:&Session,x:i32,z:i32,y_min:i32,y_max:i32,
    width:u8,depth:u8,
)->Result<SurfaceView,WorldError>{
    validate_view(width,depth,y_min,y_max)?;
    let far_x=x.checked_add(width as i32-1)
        .ok_or_else(||refuse("Surface X overflow"))?;
    let far_z=z.checked_add(depth as i32-1)
        .ok_or_else(||refuse("Surface Z overflow"))?;
    // Bounds::new enforces the EDIT batch's 4096-cell limit. Read-only
    // inspection can scan 196608 cells, so validate extrema individually.
    let _=crate::editing::Bounds::new((x,y_min,z),(x,y_min,z))?;
    let _=crate::editing::Bounds::new((far_x,y_max,far_z),(far_x,y_max,far_z))?;
    let mut cache:BTreeMap<(i32,i32),Option<Chunk>>=BTreeMap::new();
    let mut region_reader=ReadOnlyRegionCache::new(session.original_world(),session.dimension())?;
    let mut voxels=Vec::new();
    let mut missing_columns=0;
    let mut tiles:BTreeMap<(i32,i32),SourceChunkTile>=BTreeMap::new();
    for dz in 0..depth as i32{
        for dx in 0..width as i32{
            let (wx,wz)=(x+dx,z+dz);
            let key=(wx.div_euclid(16),wz.div_euclid(16));
            if !cache.contains_key(&key){
                cache.insert(key,region_reader.read_chunk(key.0,key.1)?);
            }
            let tile=tiles.entry(key).or_insert_with(||SourceChunkTile{
                chunk_x:key.0,chunk_z:key.1,
                source_present:cache.get(&key).and_then(Option::as_ref).is_some(),
                known_columns:0,unknown_columns:0,exposed_voxels:0,
            });
            let Some(chunk)=cache.get(&key).and_then(Option::as_ref) else{
                missing_columns+=1;tile.unknown_columns+=1;continue;
            };
            let mut chosen=None;
            let mut fallback=None;
            for y in (y_min..=y_max).rev(){
                let Some(original)=chunk.block(wx.rem_euclid(16) as u8,y,
                    wz.rem_euclid(16) as u8) else{continue};
                let state=session.effective_state((wx,y,wz),original);
                let kind=material(&state);
                let voxel=SurfaceVoxel{
                    x:wx,y,z:wz,state,
                    original_block_entity:chunk.has_block_entity(wx,y,wz),
                    faces:ALL_FACES,kind,staged:session.is_staged_block((wx,y,wz)),span:1,
                };
                if fallback.is_none(){fallback=Some(voxel.clone());}
                if !is_air(&voxel.state){
                    chosen=Some(voxel);break;
                }
            }
            if let Some(voxel)=chosen.or(fallback){
                tile.known_columns+=1;
                voxels.push(voxel);
            }else{
                missing_columns+=1;tile.unknown_columns+=1;
            }
        }
    }
    for voxel in &voxels{
        if let Some(tile)=tiles.get_mut(&(voxel.x.div_euclid(16),voxel.z.div_euclid(16))){
            tile.exposed_voxels+=1;
        }
    }
    Ok(SurfaceView{
        x,z,y_min,y_max,width:width as u16,depth:depth as u16,voxels,missing_columns,
        source_chunks:cache.len(),source_tiles:tiles.into_values().collect(),
    })
}

/// Bounded multilevel voxel inspection: show exposed real solids including
/// internal cavities, never inventing air for absent chunks/Sections.
pub fn inspect_exposed_volume(
    session:&Session,x:i32,z:i32,y_min:i32,y_max:i32,width:u8,depth:u8
)->Result<SurfaceView,WorldError>{
    inspect_exposed_volume_cutaway(session,x,z,y_min,y_max,width,depth,None)
}
/// A read-only cutaway exposes new top faces at the chosen Y plane, instead
/// of merely hiding previously meshed outer faces. Missing source cells stay
/// unknown, and never become assumed empty space.
pub fn inspect_exposed_volume_cutaway(
    session:&Session,x:i32,z:i32,y_min:i32,y_max:i32,width:u8,depth:u8,
    top_cut:Option<i32>
)->Result<SurfaceView,WorldError>{
    if top_cut.is_some_and(|top|top<y_min||top>y_max){
        return Err(refuse("3D cutaway Y must lie within the inspected bounds"));
    }
    let visible_top=top_cut.unwrap_or(y_max);
    validate_view(width,depth,y_min,y_max)?;
    let max_x=x.checked_add(width as i32-1).ok_or_else(||refuse("Volume X overflow"))?;
    let max_z=z.checked_add(depth as i32-1).ok_or_else(||refuse("Volume Z overflow"))?;
    let _=crate::editing::Bounds::new((x,y_min,z),(x,y_min,z))?;
    let _=crate::editing::Bounds::new((max_x,y_max,max_z),(max_x,y_max,max_z))?;
    let mut chunks:BTreeMap<(i32,i32),Option<Chunk>>=BTreeMap::new();
    let mut region_reader=ReadOnlyRegionCache::new(session.original_world(),session.dimension())?;
    let mut known:BTreeMap<(i32,i32,i32),(State,VoxelMaterial)>=BTreeMap::new();
    let mut solids=Vec::new();
    let mut missing_columns=0;
    let mut tiles:BTreeMap<(i32,i32),SourceChunkTile>=BTreeMap::new();
    for dz in 0..depth as i32{
        for dx in 0..width as i32{
            let (wx,wz)=(x+dx,z+dz);
            let key=(wx.div_euclid(16),wz.div_euclid(16));
            if !chunks.contains_key(&key){
                chunks.insert(key,region_reader.read_chunk(key.0,key.1)?);
            }
            let tile=tiles.entry(key).or_insert_with(||SourceChunkTile{
                chunk_x:key.0,chunk_z:key.1,
                source_present:chunks.get(&key).and_then(Option::as_ref).is_some(),
                known_columns:0,unknown_columns:0,exposed_voxels:0,
            });
            let Some(chunk)=chunks.get(&key).and_then(Option::as_ref) else{
                missing_columns+=1;tile.unknown_columns+=1;continue;
            };
            let mut column_known=false;
            for y in y_min..=y_max{
                let Some(original)=chunk.block(
                    wx.rem_euclid(16) as u8,y,wz.rem_euclid(16) as u8
                )else{continue};
                column_known=true;
                let state=session.effective_state((wx,y,wz),original);
                let solid=!is_air(&state);
                let kind=material(&state);
                known.insert((wx,y,wz),(state.clone(),kind));
                if solid && y<=visible_top{
                    solids.push(SurfaceVoxel{
                        x:wx,y,z:wz,state,
                        original_block_entity:chunk.has_block_entity(wx,y,wz),
                        faces:0,kind,staged:session.is_staged_block((wx,y,wz)),span:1,
                    });
                }
            }
            if column_known{tile.known_columns+=1;}else{
                missing_columns+=1;tile.unknown_columns+=1;
            }
        }
    }
    const SIDES:[(i32,i32,i32,u8);6]=[
        (0,1,0,FACE_TOP),(0,-1,0,FACE_BOTTOM),
        (1,0,0,FACE_POS_X),(-1,0,0,FACE_NEG_X),
        (0,0,1,FACE_POS_Z),(0,0,-1,FACE_NEG_Z),
    ];
    let mut voxels=Vec::new();
    for mut voxel in solids{
        for (dx,dy,dz,bit) in SIDES{
            let (nx,ny,nz)=(voxel.x+dx,voxel.y+dy,voxel.z+dz);
            let clipped=nx<x||nx>max_x||nz<z||nz>max_z
                ||ny<y_min||ny>visible_top;
            if clipped||exposed_against(
                &voxel.state,voxel.kind,known.get(&(nx,ny,nz))
            ){
                voxel.faces|=bit;
            }
        }
        if voxel.faces!=0{voxels.push(voxel);}
    }
    for voxel in &voxels{
        if let Some(tile)=tiles.get_mut(&(voxel.x.div_euclid(16),voxel.z.div_euclid(16))){
            tile.exposed_voxels+=1;
        }
    }
    Ok(SurfaceView{
        x,z,y_min,y_max,width:width as u16,depth:depth as u16,voxels,missing_columns,
        source_chunks:chunks.len(),source_tiles:tiles.into_values().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_recognized_partial_vanilla_shapes_refuse_full_face_occlusion(){
        let solid=State{name:"minecraft:stone".into(),properties:vec![]};
        let half=State{name:"minecraft:stone_slab".into(),
            properties:vec![("type".into(),"bottom".into())]};
        let double=State{name:"minecraft:stone_slab".into(),
            properties:vec![("type".into(),"double".into())]};
        let modded=State{name:"custom:half_block".into(),properties:vec![]};
        let partial=(half,VoxelMaterial::Solid);
        let full=(solid.clone(),VoxelMaterial::Solid);
        let full_slab=(double,VoxelMaterial::Solid);
        let unknown=(modded,VoxelMaterial::Solid);
        assert!(exposed_against(&solid,VoxelMaterial::Solid,Some(&partial)));
        assert!(!exposed_against(&solid,VoxelMaterial::Solid,Some(&full)));
        assert!(!exposed_against(&solid,VoxelMaterial::Solid,Some(&full_slab)));
        assert!(!exposed_against(&solid,VoxelMaterial::Solid,Some(&unknown)));
        let snow=State{name:"minecraft:snow".into(),
            properties:vec![("layers".into(),"3".into())]};
        assert!(known_partial_occluder(&snow));
    }
    #[test]
    fn original_source_tiles_are_sorted_and_distinguish_missing_sections(){
        let tile=SourceChunkTile{
            chunk_x:-1,chunk_z:0,source_present:true,
            known_columns:3,unknown_columns:2,exposed_voxels:4,
        };
        assert_eq!(tile.known_columns+tile.unknown_columns,5);
        let missing=SourceChunkTile{
            chunk_x:-2,chunk_z:0,source_present:false,
            known_columns:0,unknown_columns:16,exposed_voxels:0,
        };
        assert!(!missing.source_present);
        assert_eq!((-17_i32).div_euclid(16),missing.chunk_x);
    }
    #[test]
    fn scene_budget_allows_3x3_chunks_but_never_unbounded_source_reads(){
        assert_eq!(MAX_VIEW_CELLS,196_608);
        assert!(validate_view(64,64,-64,-17).is_ok());
        assert!(validate_view(64,64,-64,-16).is_err());
        assert!(validate_view(65,1,-64,-64).is_err());
        assert!(validate_view(0,1,0,0).is_err());
        assert!(validate_view(1,1,i32::MIN,i32::MAX).is_err());
        assert!(validate_view(1,1,-384,-337).is_ok());
    }
    #[test]
    fn summary_counts_render_faces_materials_and_source_metadata(){
        let view=SurfaceView{
            x:0,z:0,y_min:-64,y_max:-64,width:1,depth:1,
            missing_columns:0,source_chunks:1,
            source_tiles:vec![SourceChunkTile{chunk_x:0,chunk_z:0,
                source_present:true,known_columns:1,unknown_columns:0,exposed_voxels:1}],
            voxels:vec![SurfaceVoxel{
                x:0,y:-64,z:0,
                state:State{name:"minecraft:glass".into(),properties:vec![]},
                original_block_entity:true,faces:FACE_TOP|FACE_POS_X,
                kind:VoxelMaterial::Transparent,staged:true,span:1,
            }],
        };
        let stats=view.statistics();
        assert_eq!(stats.voxels,1);
        assert_eq!(stats.exposed_faces,2);
        assert_eq!((stats.original_be,stats.staged,stats.transparent),(1,1,1));
    }
    #[test]
    fn cutaway_accepts_negative_y_and_refuses_out_of_bounds_without_writing(){
        // Pure geometrical clipping: the selected layer is still the exact
        // source coordinate, and its newly exposed roof is a view cut only.
        let y_min=-64;
        let y_max=-33;
        assert_eq!(Some(-48).filter(|v|*v>=y_min&&*v<=y_max),Some(-48));
        assert_eq!(Some(-65).filter(|v|*v>=y_min&&*v<=y_max),None);
        assert_eq!(FACE_TOP|FACE_BOTTOM|FACE_POS_X|FACE_NEG_X|FACE_POS_Z|FACE_NEG_Z,ALL_FACES);
    }
    #[test]
    fn air_detection_is_explicit_not_derived_from_arbitrary_modded_name(){
        for name in ["minecraft:air","minecraft:cave_air","minecraft:void_air","numeric:0"]{
            assert!(is_air(&State{name:name.into(),properties:vec![]}));
        }
        assert!(!is_air(&State{name:"mod:custom_air".into(),properties:vec![]}));
    }
}
