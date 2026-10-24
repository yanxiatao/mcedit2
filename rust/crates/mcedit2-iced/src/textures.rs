//! Optional non-mutating Java resource-pack block PNG reader.
//! Supports an extracted pack directory or a ZIP, never executes or extracts
//! pack data; missing/unknown textures retain the existing safe color guide.
use std::collections::BTreeMap;
use std::fs::{self,File};
use std::io::{Read,Seek};
use std::path::Path;
use image::ImageFormat;
use mcedit2_world::sections::State;
use crate::resource_models::ModelBank;

const MAX_TEXTURES:usize=1024;
const MAX_TEXTURE_BYTES:u64=4*1024*1024;
const MAX_PIXEL_EDGE:u32=512;
const MAX_RGBA_MEMORY:usize=48*1024*1024;
const ATLAS_CELL:u32=32;
const ATLAS_STRIDE:u32=34;
const ATLAS_COLUMNS:u32=33;
/// Rendering semantics, derived from actual PNG alpha instead of block ID
/// guesses. Liquid/foliage may override opaque PNGs in the viewport.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
#[repr(u32)]
pub(crate) enum AlphaMode{Opaque=0,Cutout=1,Blend=2}
impl AlphaMode{
    fn from_pixels(pixels:&[u8])->Self{
        let mut zero=false;
        let mut partial=false;
        for alpha in pixels.iter().skip(3).step_by(4){
            if *alpha==0{zero=true;}else if *alpha<255{partial=true;}
            if partial{return Self::Blend;}
        }
        if zero{Self::Cutout}else{Self::Opaque}
    }
}
#[derive(Clone,Debug)]
pub(crate) struct Tile {
    pub width:u32,
    pub height:u32,
    pub rgba:Vec<u8>,
}
impl Tile {
    pub fn alpha_mode(&self)->AlphaMode{AlphaMode::from_pixels(&self.rgba)}
    pub fn pixel(&self,u:f32,v:f32)->[u8;4]{
        let x=(u.clamp(0.0,0.999_999)*self.width as f32) as usize;
        let y=(v.clamp(0.0,0.999_999)*self.height as f32) as usize;
        let at=(y*self.width as usize+x)*4;
        [self.rgba[at],self.rgba[at+1],self.rgba[at+2],self.rgba[at+3]]
    }
}
/// UVs refer to the center of each 32px atlas texel; slots have a
/// 1-pixel duplicated gutter on all sides to prevent texture bleeding.
#[derive(Clone,Debug,PartialEq)]
pub(crate) struct AtlasUv{pub start:[f32;2],pub end:[f32;2]}
fn orient_uv(corners:[[f32;2];4],permutation:[u8;4])->[[f32;2];4]{
    permutation.map(|index|corners[index as usize])
}
impl AtlasUv{
    pub fn corners_for(&self,uv:[f32;4],rotation:u16)->[[f32;2];4]{
        let mut corners=[
            self.sample(uv[0]/16.0,uv[1]/16.0),
            self.sample(uv[2]/16.0,uv[1]/16.0),
            self.sample(uv[2]/16.0,uv[3]/16.0),
            self.sample(uv[0]/16.0,uv[3]/16.0),
        ];
        for _ in 0..rotation/90{
            corners.rotate_right(1);
        }
        corners
    }
    pub fn sample(&self,u:f32,v:f32)->[f32;2]{
        let u=u.clamp(0.0,1.0);
        let v=v.clamp(0.0,1.0);
        [self.start[0]+(self.end[0]-self.start[0])*u,
         self.start[1]+(self.end[1]-self.start[1])*v]
    }
}
#[derive(Debug)]
pub(crate) struct TextureAtlas{
    pub width:u32,pub height:u32,pub rgba:Vec<u8>,
    pub entries:BTreeMap<String,AtlasUv>,
    pub white:AtlasUv,
}
impl TextureAtlas{
    fn create(tiles:&BTreeMap<String,Tile>)->Self{
        let slots=tiles.len()+1; // always reserve white for color-only faces
        let rows=(slots as u32).div_ceil(ATLAS_COLUMNS);
        let width=ATLAS_COLUMNS*ATLAS_STRIDE;
        let height=rows*ATLAS_STRIDE;
        let mut rgba=vec![0u8;width as usize*height as usize*4];
        let uv=|slot:usize|{
            let x=(slot as u32%ATLAS_COLUMNS)*ATLAS_STRIDE+1;
            let y=(slot as u32/ATLAS_COLUMNS)*ATLAS_STRIDE+1;
            AtlasUv{
                start:[(x as f32+0.5)/width as f32,(y as f32+0.5)/height as f32],
                end:[(x as f32+ATLAS_CELL as f32-0.5)/width as f32,
                    (y as f32+ATLAS_CELL as f32-0.5)/height as f32],
            }
        };
        // GPU linear filtering interpolates neighboring RGB even when the
        // neighboring PNG alpha is zero. Expand nearby opaque RGB *only*
        // into zero-alpha pixels, leaving original alpha/cutout mask intact.
        // This avoids dark fringes on leaf/pane cutouts in Detailed mode.
        fn padded_tile(tile:&Tile)->Vec<[u8;4]>{
            let n=ATLAS_CELL as usize;
            let mut data=Vec::with_capacity(n*n);
            for y in 0..n{
                for x in 0..n{
                    data.push(tile.pixel((x as f32+0.5)/n as f32,
                        (y as f32+0.5)/n as f32));
                }
            }
            if tile.alpha_mode()==AlphaMode::Cutout{
                let old=data.clone();
                for y in 0..n{
                    for x in 0..n{
                        if old[y*n+x][3]!=0{continue;}
                        'nearest: for radius in 1_i32..=3{
                            for dy in -radius..=radius{
                                let dx=radius-dy.abs();
                                for nx in [x as i32-dx,x as i32+dx]{
                                    let ny=y as i32+dy;
                                    if nx<0||ny<0||nx>=n as i32||ny>=n as i32{
                                        continue;
                                    }
                                    let candidate=old[ny as usize*n+nx as usize];
                                    if candidate[3]==255{
                                        data[y*n+x][..3].copy_from_slice(&candidate[..3]);
                                        break 'nearest;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            data
        }
        let write=|slot:usize, tile:&Tile,rgba:&mut Vec<u8>|{
            let x0=(slot as u32%ATLAS_COLUMNS)*ATLAS_STRIDE;
            let y0=(slot as u32/ATLAS_COLUMNS)*ATLAS_STRIDE;
            let padded=padded_tile(tile);
            for y in 0..ATLAS_STRIDE{
                for x in 0..ATLAS_STRIDE{
                    let sx=(x as i32-1).clamp(0,31) as usize;
                    let sy=(y as i32-1).clamp(0,31) as usize;
                    let px=padded[sy*ATLAS_CELL as usize+sx];
                    let at=(((y0+y)*width+(x0+x))*4) as usize;
                    rgba[at..at+4].copy_from_slice(&px);
                }
            }
        };
        let white_tile=Tile{width:1,height:1,rgba:vec![255,255,255,255]};
        write(0,&white_tile,&mut rgba);
        let mut entries=BTreeMap::new();
        for (i,(name,tile)) in tiles.iter().enumerate(){
            write(i+1,tile,&mut rgba);
            entries.insert(name.clone(),uv(i+1));
        }
        Self{width,height,rgba,entries,white:uv(0)}
    }
}
#[derive(Clone,Debug,Default)]
pub(crate) struct TextureBank{
    tiles:BTreeMap<String,Tile>,
    models:ModelBank,
    atlas:Option<std::sync::Arc<TextureAtlas>>,
    pub source:String,
    pub warnings:Vec<String>,
}
fn texture_id(path:&str)->Option<String>{
    let parts:Vec<_>=path.split('/').collect();
    if parts.len()<5||parts[0]!="assets"||parts[2]!="textures"
        ||parts[3]!="block"||!path.ends_with(".png"){return None;}
    let namespace=parts[1];
    if namespace.is_empty()||namespace.len()>80
        ||!namespace.bytes().all(|c|c.is_ascii_lowercase()||c.is_ascii_digit()||c==b'_'||c==b'-')
    {return None;}
    let relative=parts[4..].join("/");
    let name=relative.strip_suffix(".png")?;
    if name.is_empty()||name.len()>150
        ||!name.bytes().all(|c|c.is_ascii_lowercase()||c.is_ascii_digit()
            ||c==b'_'||c==b'-'||c==b'/')
    {return None;}
    Some(format!("{namespace}:{name}"))
}
fn decode_png(bytes:&[u8])->Result<Tile,String>{
    if bytes.len() as u64>MAX_TEXTURE_BYTES{return Err("PNG exceeds 4MiB".into());}
    if bytes.len()<24 || &bytes[..8]!=b"\x89PNG\r\n\x1a\n"{
        return Err("Invalid PNG header".into());
    }
    let source_width=u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let source_height=u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if source_width==0||source_height==0||source_width>MAX_PIXEL_EDGE
        ||source_height>4096{return Err("PNG header exceeds safe dimensions".into());}
    let image=image::load_from_memory_with_format(bytes,ImageFormat::Png)
        .map_err(|e|format!("Invalid PNG: {e}"))?.into_rgba8();
    let (width,height)=image.dimensions();
    if width==0||height==0||width>MAX_PIXEL_EDGE||height>4096 {
        return Err("Texture PNG exceeds decoded frame safeguards".into());
    }
    // Minecraft animated textures store their frames vertically; we display
    // frame zero until animated sprites gain their own timeline.
    let frame_h=if height>=width&&height%width==0 {width}else{height};
    if frame_h>MAX_PIXEL_EDGE{return Err("Animation frame exceeds 512px".into());}
    let pixels=image.into_raw();
    let end=width as usize*frame_h as usize*4;
    Ok(Tile{width,height:frame_h,rgba:pixels[..end].to_vec()})
}
impl TextureBank {
    fn insert(&mut self,id:String,data:&[u8],memory:&mut usize)->Result<(),String>{
        if self.tiles.len()>=MAX_TEXTURES{return Err("Texture limit (1024) reached".into());}
        let tile=decode_png(data)?;
        let next=memory.checked_add(tile.rgba.len()).ok_or("Texture memory overflow")?;
        if next>MAX_RGBA_MEMORY{return Err("Texture bank exceeds 48MiB".into());}
        *memory=next;
        self.tiles.insert(id,tile);
        Ok(())
    }
    fn walk(&mut self,root:&Path,folder:&Path,memory:&mut usize,
        depth:u8,entries:&mut usize)->Result<(),String>{
        if depth>20{return Err("Resource-pack folder nesting over 20".into());}
        if !folder.is_dir(){return Ok(());}
        for item in fs::read_dir(folder).map_err(|e|e.to_string())?{
            let item=item.map_err(|e|e.to_string())?;
            *entries+=1;
            if *entries>16384{return Err("Resource pack has excessive directory entries".into());}
            let ty=item.file_type().map_err(|e|e.to_string())?;
            if ty.is_symlink(){continue;}
            let path=item.path();
            if ty.is_dir(){
                self.walk(root,&path,memory,depth+1,entries)?;
            }else if ty.is_file(){
                let Ok(relative)=path.strip_prefix(root)else{continue};
                let name=relative.to_string_lossy().replace('\\',"/");
                let id=texture_id(&name);
                let is_model=name.ends_with(".json")&&(name.contains("/models/block/")
                    ||name.contains("/blockstates/"));
                if id.is_none()&&!is_model{continue;}
                let meta=fs::metadata(&path).map_err(|e|e.to_string())?;
                if meta.len()>(if is_model{256*1024}else{MAX_TEXTURE_BYTES}){
                    self.warnings.push(format!("Skipped oversize {name}"));continue;
                }
                let bytes=fs::read(&path).map_err(|e|e.to_string())?;
                if is_model{
                    if let Err(e)=self.models.insert(&name,&bytes){
                        self.warnings.push(format!("Skipped model {name}: {e}"));
                    }
                }else if let Some(id)=id{
                    if let Err(e)=self.insert(id.clone(),&bytes,memory){
                        self.warnings.push(format!("Skipped {id}: {e}"));
                    }
                }
            }
        }
        Ok(())
    }
    fn read_zip<R:Read+Seek>(&mut self,r:R)->Result<(),String>{
        let mut archive=zip::ZipArchive::new(r).map_err(|e|e.to_string())?;
        if archive.len()>65536{return Err("ZIP has excessive entries".into());}
        let mut memory=0;
        for i in 0..archive.len(){
            let mut member=archive.by_index(i).map_err(|e|e.to_string())?;
            let name=member.name().to_string();
            let id=texture_id(&name);
            let is_model=name.ends_with(".json")&&(name.contains("/models/block/")
                ||name.contains("/blockstates/"));
            if id.is_none()&&!is_model{continue;}
            if member.size()>(if is_model{256*1024}else{MAX_TEXTURE_BYTES}){
                self.warnings.push(format!("Skipped oversize {name}"));continue;
            }
            let mut bytes=Vec::new();
            member.by_ref().take(if is_model{256*1024+1}else{MAX_TEXTURE_BYTES+1})
                .read_to_end(&mut bytes).map_err(|e|e.to_string())?;
            if is_model{
                if let Err(e)=self.models.insert(&name,&bytes){
                    self.warnings.push(format!("Skipped model {name}: {e}"));
                }
            }else if let Some(id)=id{
                if let Err(e)=self.insert(id.clone(),&bytes,&mut memory){
                    self.warnings.push(format!("Skipped {id}: {e}"));
                }
            }
        }
        Ok(())
    }
    pub fn open(path:&Path)->Result<Self,String>{
        if path.as_os_str().is_empty(){return Err("Select a resource-pack directory or ZIP".into());}
        let mut out=Self{source:path.display().to_string(),..Self::default()};
        if path.is_dir(){
            let root=path.canonicalize().map_err(|e|e.to_string())?;
            let mut memory=0;
            let mut entries=0;
            out.walk(&root,&root.join("assets"),&mut memory,0,&mut entries)?;
        }else if path.is_file(){
            if !matches!(path.extension().and_then(|v|v.to_str()),Some("zip"|"jar")){
                return Err("Only directories, .zip packs and .jar client assets are supported".into());
            }
            let file=File::open(path).map_err(|e|e.to_string())?;
            out.read_zip(file)?;
        }else{return Err("Resource pack does not exist".into());}
        if out.tiles.is_empty(){return Err("Pack contains no supported assets/*/textures/block/*.png".into());}
        out.atlas=Some(std::sync::Arc::new(TextureAtlas::create(&out.tiles)));
        Ok(out)
    }
    pub fn count(&self)->usize{self.tiles.len()}
    pub fn atlas(&self)->Option<&std::sync::Arc<TextureAtlas>>{self.atlas.as_ref()}
    /// Sample the same packed PNG alpha bytes used by the GPU. The hit test
    /// only uses it as a conservative pixel mask; no world data is mutated.
    pub fn atlas_pixel(&self,uv:[f32;2])->Option<[u8;4]>{
        let atlas=self.atlas.as_ref()?;
        if !uv[0].is_finite()||!uv[1].is_finite(){return None;}
        let x=(uv[0].clamp(0.0,0.999_999)*atlas.width as f32) as usize;
        let y=(uv[1].clamp(0.0,0.999_999)*atlas.height as f32) as usize;
        let at=(y*atlas.width as usize+x)*4;
        let rgba=atlas.rgba.get(at..at+4)?;
        Some([rgba[0],rgba[1],rgba[2],rgba[3]])
    }
    pub fn atlas_size(&self)->Option<(u32,u32)>{
        self.atlas.as_ref().map(|a|(a.width,a.height))
    }
    /// Resolve GPU UV and material alpha once per source state/face/element.
    /// The fallback may use old per-block named textures, but never another
    /// namespace with a coincidentally matching resource path.
    pub fn element_unshaded(&self,state:&State,element_index:usize)->Option<bool>{
        self.models.element_unshaded(state,element_index)
    }
    pub fn face_cull_direction(&self,state:&State,face:crate::isometric::Face,
        element_index:usize)->Option<Option<crate::isometric::Face>>{
        self.models.face_cull_direction(state,face,element_index)
    }
    pub fn face_defined(&self,state:&State,face:crate::isometric::Face,
        element_index:usize)->Option<bool>{
        self.models.face_defined(state,face,element_index)
    }
    pub fn atlas_face_style(&self,state:&State,face:crate::isometric::Face,
        element_index:usize)->Option<([[f32;2];4],AlphaMode)>{
        let id_spec=self.models.element_face(state,face,element_index);
        let (id,uv,rot,perm)=match id_spec{
            Some(spec) if self.tiles.contains_key(&spec.texture)=>
                (spec.texture,spec.uv,spec.rotation,spec.uvlock),
            _=>(self.resolve_id(state,face)?,[0.0,0.0,16.0,16.0],0,[0,1,2,3]),
        };
        let region=self.atlas.as_ref()?.entries.get(&id)?;
        let alpha=self.tiles.get(&id)?.alpha_mode();
        Some((orient_uv(region.corners_for(uv,rot),perm),alpha))
    }
    pub fn alpha_mode(&self,state:&State,face:crate::isometric::Face)->Option<AlphaMode>{
        self.resolve(state,face).map(Tile::alpha_mode)
    }
    pub fn atlas_face_corners(&self,state:&State,face:crate::isometric::Face,
        element_index:usize)->Option<[[f32;2];4]>{
        if let Some(spec)=self.models.element_face(state,face,element_index){
            if let Some(region)=self.atlas.as_ref()?.entries.get(&spec.texture){
                return Some(orient_uv(region.corners_for(spec.uv,spec.rotation),spec.uvlock));
            }
        }
        self.atlas_uv(state,face).map(|r|r.corners_for([0.0,0.0,16.0,16.0],0))
    }
    pub fn atlas_uv(&self,state:&State,face:crate::isometric::Face)->Option<AtlasUv>{
        let id=self.resolve_id(state,face)?;
        self.atlas.as_ref()?.entries.get(&id).cloned()
    }
    pub fn model_count(&self)->usize{self.models.count()}
    pub fn geometry(&self,state:&State)->Option<Vec<crate::models::Part>>{
        self.models.parts(state)
    }
    pub fn get(&self,id:&str)->Option<&Tile>{self.tiles.get(id)}
    pub fn resolve_id(&self,state:&State,face:crate::isometric::Face)->Option<String>{
        let (ns,name)=state.name.split_once(':')?;
        if ns=="numeric"{return None;}
        if let Some(key)=self.models.texture(state,face){
            if self.tiles.contains_key(&key){return Some(key);}
        }
        let top=face==crate::isometric::Face::Top;
        let bottom=face==crate::isometric::Face::Bottom;
        let mut candidates=Vec::with_capacity(6);
        if name=="grass_block"{
            candidates.push(if top{"grass_block_top"}else if bottom{"dirt"}else{"grass_block_side"}.to_string());
        }else if name=="mycelium"{
            candidates.push(if top{"mycelium_top"}else if bottom{"dirt"}else{"mycelium_side"}.into());
        }else if name=="podzol"{
            candidates.push(if top{"podzol_top"}else if bottom{"dirt"}else{"podzol_side"}.into());
        }else if name.ends_with("_log")||name.ends_with("_stem")||name.ends_with("_wood"){
            let axis=state.properties.iter().find(|(k,_)|k=="axis").map(|(_,v)|v.as_str()).unwrap_or("y");
            let end=match axis{"x"=>matches!(face,crate::isometric::Face::PosX|crate::isometric::Face::NegX),
                "z"=>matches!(face,crate::isometric::Face::PosZ|crate::isometric::Face::NegZ),
                _=>top||bottom};
            if end{candidates.push(format!("{name}_top"));}
            candidates.push(name.to_owned());
        }else if name=="water"||name=="lava"{
            candidates.push(format!("{name}_still"));
            candidates.push(format!("{name}_flow"));
        }else if name=="sandstone"{
            candidates.push(if top{"sandstone_top"}else if bottom{"sandstone_bottom"}else{"sandstone"}.into());
        }
        candidates.push(name.to_owned());
        // Namespace is retained: never project one mod's asset onto another.
        for key in candidates{
            let id=format!("{ns}:{key}");
            if self.tiles.contains_key(&id){return Some(id);}
        }
        None
    }
    pub fn resolve<'a>(&'a self,state:&State,face:crate::isometric::Face)->Option<&'a Tile>{
        self.get(&self.resolve_id(state,face)?)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::isometric::Face;
    fn state(name:&str,properties:Vec<(String,String)>)->State{State{name:name.into(),properties}}
    fn solid(color:[u8;4])->Tile{Tile{width:1,height:1,rgba:color.to_vec()}}
    #[test]fn cutout_atlas_avoids_black_filter_fringe_without_changing_alpha(){
        let mut tiles=BTreeMap::new();
        tiles.insert("test:leaves".into(),Tile{width:2,height:1,rgba:vec![
            83,183,61,255,0,0,0,0,
        ]});
        let atlas=TextureAtlas::create(&tiles);
        let uv=atlas.entries.get("test:leaves").unwrap().sample(0.53,0.5);
        let x=(uv[0]*atlas.width as f32) as usize;
        let y=(uv[1]*atlas.height as f32) as usize;
        let at=(y*atlas.width as usize+x)*4;
        assert_eq!(&atlas.rgba[at..at+4],[83,183,61,0]);
        assert_eq!(&tiles["test:leaves"].rgba[4..8],[0,0,0,0]);
        let far=atlas.entries.get("test:leaves").unwrap().sample(0.95,0.5);
        let fx=(far[0]*atlas.width as f32) as usize;
        let fy=(far[1]*atlas.height as f32) as usize;
        assert_eq!(atlas.rgba[(fy*atlas.width as usize+fx)*4+3],0);
    }
    #[test]fn atlas_packs_sorted_ids_without_cross_tile_color_bleed(){
        let mut tiles=BTreeMap::new();
        tiles.insert("a:first".into(),solid([255,0,0,255]));
        tiles.insert("b:second".into(),solid([0,0,255,128]));
        let atlas=TextureAtlas::create(&tiles);
        assert_eq!(atlas.width,ATLAS_COLUMNS*ATLAS_STRIDE);
        let one=atlas.entries.get("a:first").unwrap().sample(0.5,0.5);
        let two=atlas.entries.get("b:second").unwrap().sample(0.5,0.5);
        let pixel=|uv:[f32;2]|{
            let x=(uv[0]*atlas.width as f32) as usize;
            let y=(uv[1]*atlas.height as f32) as usize;
            atlas.rgba[(y*atlas.width as usize+x)*4..][..4].to_vec()
        };
        assert_eq!(pixel(one),vec![255,0,0,255]);
        assert_eq!(pixel(two),vec![0,0,255,128]);
        assert_eq!(pixel(atlas.white.sample(0.5,0.5)),vec![255,255,255,255]);
    }
    #[test]fn atlas_alpha_sampling_retains_transparent_holes(){
        let mut bank=TextureBank::default();
        bank.tiles.insert("test:bars".into(),Tile{width:2,height:1,
            rgba:vec![255,255,255,255,0,0,0,0]});
        bank.atlas=Some(std::sync::Arc::new(TextureAtlas::create(&bank.tiles)));
        let rect=bank.atlas.as_ref().unwrap().entries.get("test:bars").unwrap();
        let solid=rect.sample(0.2,0.5);
        let hole=rect.sample(0.8,0.5);
        assert_eq!(bank.atlas_pixel(solid).unwrap()[3],255);
        assert_eq!(bank.atlas_pixel(hole).unwrap()[3],0);
        assert_eq!(bank.atlas_pixel([f32::NAN,0.2]),None);
    }
    #[test]fn alpha_modes_distinguish_cutout_from_true_translucency(){
        assert_eq!(AlphaMode::from_pixels(&[1,2,3,255,0,0,0,255]),AlphaMode::Opaque);
        assert_eq!(AlphaMode::from_pixels(&[0,0,0,0,1,2,3,255]),AlphaMode::Cutout);
        assert_eq!(AlphaMode::from_pixels(&[0,0,0,0,1,2,3,128]),AlphaMode::Blend);
        assert_eq!(AlphaMode::from_pixels(&[0,0,0,0]),AlphaMode::Cutout);
    }
    #[test]fn one_pass_gpu_face_style_preserves_cutout_and_uv_coordinates(){
        let mut bank=TextureBank::default();
        bank.tiles.insert("test:casing".into(),Tile{width:2,height:1,
            rgba:vec![10,20,30,255,20,30,40,0]});
        bank.atlas=Some(std::sync::Arc::new(TextureAtlas::create(&bank.tiles)));
        let state=state("test:casing",vec![]);
        let (uv,alpha)=bank.atlas_face_style(&state,Face::Top,0).unwrap();
        assert_eq!(alpha,AlphaMode::Cutout);
        let first=bank.atlas_uv(&state,Face::Top).unwrap();
        assert_eq!(uv[0],first.start);
    }
    #[test]fn uvlock_handles_corner_reflection_as_well_as_rotation(){
        let slot=AtlasUv{start:[0.2,0.3],end:[0.4,0.5]};
        let uv=slot.corners_for([4.0,0.0,12.0,16.0],0);
        let mirrored=orient_uv(uv,[1,0,3,2]);
        assert_eq!(mirrored[0],uv[1]);
        assert_eq!(mirrored[1],uv[0]);
        assert_eq!(mirrored[3],uv[2]);
        assert!(mirrored.iter().flatten().all(|x|x.is_finite()));
    }
    #[test]fn per_face_uv_crop_and_rotation_stay_inside_atlas_slot(){
        let slot=AtlasUv{start:[0.2,0.3],end:[0.4,0.5]};
        let straight=slot.corners_for([4.0,0.0,12.0,16.0],0);
        let turned=slot.corners_for([4.0,0.0,12.0,16.0],90);
        assert_eq!(straight[0],[0.25,0.3]);
        assert_eq!(turned[1],straight[0]);
        assert!((straight[2][0]-0.35).abs()<1e-6);
    }
    #[test]fn complete_atlas_capacity_stays_within_memory_limits(){
        let mut tiles=BTreeMap::new();
        for i in 0..MAX_TEXTURES{
            tiles.insert(format!("test:block_{i}"),solid([1,2,3,4]));
        }
        let atlas=TextureAtlas::create(&tiles);
        assert!(atlas.rgba.len()<6_000_000);
        assert_eq!(atlas.entries.len(),MAX_TEXTURES);
        assert_eq!(atlas.height,ATLAS_STRIDE*32);
    }
    #[test]fn invalid_png_header_rejected_before_allocating_pixels(){
        assert!(decode_png(b"not a png").is_err());
        let mut bytes=vec![0u8;32];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes[16..20].copy_from_slice(&10000u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&1u32.to_be_bytes());
        assert!(decode_png(&bytes).unwrap_err().contains("dimensions"));
    }
    #[test]fn path_parser_is_namespaced_and_does_not_accept_traversal(){
        assert_eq!(texture_id("assets/minecraft/textures/block/stone.png"),Some("minecraft:stone".into()));
        assert_eq!(texture_id("assets/example/textures/block/machine/casing.png"),Some("example:machine/casing".into()));
        assert_eq!(texture_id("assets/../textures/block/test.png"),None);
        assert_eq!(texture_id("assets/minecraft/textures/item/stone.png"),None);
    }
    #[test]fn exact_face_rule_and_mod_namespace(){
        let mut bank=TextureBank::default();
        bank.tiles.insert("minecraft:oak_log".into(),solid([17,0,0,255]));
        bank.tiles.insert("minecraft:oak_log_top".into(),solid([81,0,0,255]));
        let log=state("minecraft:oak_log",vec![("axis".into(),"x".into())]);
        assert_eq!(bank.resolve(&log,Face::PosX).unwrap().pixel(0.4,0.4)[0],81);
        assert_eq!(bank.resolve(&log,Face::Top).unwrap().pixel(0.4,0.4)[0],17);
        assert!(bank.resolve(&state("mod:oak_log",vec![]),Face::Top).is_none());
    }
    #[test]fn nearest_sampler_clamps_and_preserves_alpha(){
        let tile=Tile{width:2,height:2,rgba:vec![1,2,3,255,4,5,6,160,7,8,9,255,10,11,12,0]};
        assert_eq!(tile.pixel(0.75,0.25),[4,5,6,160]);
        assert_eq!(tile.pixel(9.0,9.0),[10,11,12,0]);
    }
}