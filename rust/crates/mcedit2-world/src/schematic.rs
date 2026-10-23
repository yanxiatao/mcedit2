//! Read-only old MCEdit Alpha numeric-ID .schematic importer.
use std::collections::BTreeMap;
use std::fs::{self,OpenOptions};
use std::io::Write;
use std::path::Path;
use std::io::Read;
use flate2::{read::GzDecoder,write::GzEncoder,Compression};
use crate::sections::{parse_root,State,Tag};
use crate::editing::{parse_state,render_state};
use crate::clipboard::{self,BlockClipboard};
use crate::{WorldError,WorldSummary};

fn err(s:impl Into<String>)->WorldError{WorldError::Invalid(s.into())}
fn short(root:&Tag<'_>,name:&str)->Result<u16,WorldError>{
    let t=root.require(name,2).map_err(|e|err(e.to_string()))?;
    let size=i16::from_be_bytes(t.raw.try_into().map_err(|_|err("Invalid schematic short"))?);
    if size<=0{return Err(err(format!("Nonpositive schematic {name}")));}
    Ok(size as u16)
}
fn array<'a>(root:&'a Tag<'_>,name:&str,n:usize)->Result<&'a [u8],WorldError>{
    let t=root.require(name,7).map_err(|e|err(e.to_string()))?;
    if t.raw.len()!=4+n||i32::from_be_bytes(t.raw[..4].try_into().unwrap())!=n as i32{
        return Err(err(format!("Schematic {name} has incorrect length")));
    }
    Ok(&t.raw[4..])
}
fn nibble(a:&[u8],i:usize)->u8{
    if i&1==0{a[i/2]&15}else{a[i/2]>>4}
}
pub fn decode_legacy(world:&WorldSummary,dimension:&str,compressed:&[u8])
    ->Result<BlockClipboard,WorldError>{
    if compressed.len()>8*1024*1024{return Err(err("Schematic exceeds 8 MiB compressed"));}
    let mut raw=Vec::new();
    GzDecoder::new(compressed).take(16*1024*1024+1)
        .read_to_end(&mut raw)?;
    if raw.len()>16*1024*1024{return Err(err("Schematic exceeds 16 MiB expanded"));}
    let root=parse_root(&raw).map_err(|e|err(format!("Bad original schematic NBT: {e}")))?;
    let materials=root.require("Materials",8).map_err(|e|err(e.to_string()))?;
    if materials.raw!=[0,5,b'A',b'l',b'p',b'h',b'a']{
        return Err(err("Only original Materials=Alpha numeric schematics are supported"));
    }
    for field in ["TileEntities","Entities","TileTicks"]{
        if let Some(tag)=root.field(field){
            if tag.kind!=9||!tag.items.is_empty(){
                return Err(err(format!("Schematic {field} contains NBT which cannot safely be dropped")));
            }
        }
    }
    let w=short(&root,"Width")?;
    let h=short(&root,"Height")?;
    let l=short(&root,"Length")?;
    let n=(w as usize).checked_mul(h as usize)
        .and_then(|x|x.checked_mul(l as usize))
        .ok_or_else(||err("Schematic size overflow"))?;
    if n==0||n>4096{return Err(err("Schematic must contain 1..4096 blocks"));}
    let blocks=array(&root,"Blocks",n)?;
    let data=array(&root,"Data",n)?;
    let add=root.field("AddBlocks").map(|tag|{
        if tag.kind!=7||tag.raw.len()!=4+n.div_ceil(2)
            ||i32::from_be_bytes(tag.raw[..4].try_into().unwrap())!=n.div_ceil(2) as i32 {
            return Err(err("Invalid original schematic AddBlocks length"));
        }
        Ok(&tag.raw[4..])
    }).transpose()?;
    let mut states=Vec::with_capacity(n);
    for i in 0..n{
        if data[i]>15{return Err(err("Schematic metadata > 15"));}
        let high=add.map_or(0,|v|nibble(v,i)) as u16;
        states.push(State{
            name:format!("numeric:{}",blocks[i] as u16|(high<<8)),
            properties:vec![("data".into(),data[i].to_string())]
        });
    }
    clipboard::new(world,dimension,true,[0,0,0],[w,h,l],states)
}
pub fn import_legacy(world:&WorldSummary,dimension:&str,path:&Path)
    ->Result<BlockClipboard,WorldError>{
    if path.extension().and_then(|x|x.to_str())!=Some("schematic"){
        return Err(err("Expected a .schematic file"));
    }
    let meta=fs::symlink_metadata(path)?;
    if !meta.file_type().is_file()||meta.file_type().is_symlink()
        ||meta.len()>8*1024*1024{
        return Err(err("Schematic must be a regular file of <= 8 MiB"));
    }
    decode_legacy(world,dimension,&fs::read(path)?)
}

/// Strict Sponge v2 block-state palette reader. Import fails rather than
/// discarding any original BlockEntity, Entity or BiomeData.
pub fn decode_sponge_v2(world:&WorldSummary,dimension:&str,compressed:&[u8])
    ->Result<BlockClipboard,WorldError>{
    if compressed.len()>8*1024*1024{return Err(err("Sponge schematic exceeds 8 MiB"));}
    let mut raw=Vec::new();
    GzDecoder::new(compressed).take(16*1024*1024+1)
        .read_to_end(&mut raw)?;
    if raw.len()>16*1024*1024{return Err(err("Sponge expanded NBT exceeds 16 MiB"));}
    let root=parse_root(&raw).map_err(|e|err(format!("Sponge NBT invalid: {e}")))?;
    let v=root.require("Version",3).map_err(|e|err(e.to_string()))?;
    if v.i32().map_err(|e|err(e.to_string()))?!=2{
        return Err(err("Only Sponge schematic Version 2 is supported"));
    }
    let dv=root.require("DataVersion",3).map_err(|e|err(e.to_string()))?
        .i32().map_err(|e|err(e.to_string()))?;
    if world.data_version!=Some(dv){
        return Err(err("Sponge schematic DataVersion differs from target world; no state conversion is implemented"));
    }
    for name in ["BlockEntities","TileEntities","Entities","TileTicks"]{
        if let Some(value)=root.field(name){
            if value.kind!=9 || !value.items.is_empty(){
                return Err(err(format!("Sponge {name} is nonempty or malformed; lossless import refused")));
            }
        }
    }
    for name in ["BiomeData","BiomePalette","BiomePaletteMax"]{
        if root.field(name).is_some(){
            return Err(err("Sponge biome information cannot be preserved by block-only import"));
        }
    }
    if let Some(offset)=root.field("Offset"){
        if offset.kind!=11 || offset.raw!=[0,0,0,3,0,0,0,0,0,0,0,0,0,0,0,0]{
            return Err(err("Nonzero Sponge schematic Offset is unsupported"));
        }
    }
    let w=short(&root,"Width")?;
    let h=short(&root,"Height")?;
    let l=short(&root,"Length")?;
    let n=(w as usize).checked_mul(h as usize)
        .and_then(|x|x.checked_mul(l as usize))
        .ok_or_else(||err("Sponge schematic dimensions overflow"))?;
    if n==0 || n>4096{return Err(err("Sponge block-only paste budget is 4096 cells"));}
    let palette=root.require("Palette",10).map_err(|e|err(e.to_string()))?;
    if palette.fields.is_empty() || palette.fields.len()>4096{
        return Err(err("Invalid Sponge block-state palette size"));
    }
    if let Some(limit)=root.field("PaletteMax"){
        let maximum=limit.i32().map_err(|e|err(e.to_string()))?;
        if maximum!=palette.fields.len() as i32{
            return Err(err("Sponge PaletteMax does not match exact palette size"));
        }
    }
    let mut states=vec![None;palette.fields.len()];
    for (name,value) in &palette.fields{
        let index=value.i32().map_err(|e|err(e.to_string()))?;
        let index=usize::try_from(index).map_err(|_|err("Negative Sponge palette index"))?;
        if index>=states.len()||states[index].is_some(){
            return Err(err("Duplicate or out-of-range Sponge palette index"));
        }
        let block=parse_state(name)?;
        if block.name.starts_with("numeric:"){
            return Err(err("Sponge named palette cannot contain numeric:ID states"));
        }
        states[index]=Some(block);
    }
    let states=states.into_iter().map(|item|
        item.ok_or_else(||err("Gap in Sponge palette indices"))
    ).collect::<Result<Vec<_>,_>>()?;
    let packed=root.require("BlockData",7).map_err(|e|err(e.to_string()))?;
    if packed.raw.len()<4{return Err(err("Truncated Sponge BlockData"));}
    let count=i32::from_be_bytes(packed.raw[..4].try_into().unwrap());
    if count<0||count as usize+4!=packed.raw.len(){
        return Err(err("Sponge BlockData ByteArray length mismatch"));
    }
    let mut pos=0usize;
    let mut blocks=Vec::with_capacity(n);
    let bytes=&packed.raw[4..];
    for _ in 0..n{
        let mut index=0u32;
        let mut terminated=false;
        for shift in (0..35).step_by(7){
            let next=*bytes.get(pos).ok_or_else(||err("Truncated Sponge block VarInt"))?;
            pos+=1;
            if shift==28 && next & 0xf0!=0{
                return Err(err("Sponge VarInt integer overflow"));
            }
            index|=((next&0x7f) as u32)<<shift;
            if next&0x80==0{terminated=true;break;}
        }
        if !terminated{return Err(err("Sponge VarInt is longer than five bytes"));}
        let state=states.get(index as usize)
            .ok_or_else(||err("Sponge block index missing from palette"))?;
        blocks.push(state.clone());
    }
    if pos!=bytes.len(){return Err(err("Trailing unused Sponge BlockData VarInts"));}
    clipboard::new(world,dimension,false,[0,0,0],[w,h,l],blocks)
}
pub fn import_sponge_v2(world:&WorldSummary,dimension:&str,path:&Path)
    ->Result<BlockClipboard,WorldError>{
    if path.extension().and_then(|s|s.to_str())!=Some("schem"){
        return Err(err("Expected Sponge .schem file"));
    }
    let meta=fs::symlink_metadata(path)?;
    if !meta.file_type().is_file()||meta.file_type().is_symlink()
        ||meta.len()>8*1024*1024{
        return Err(err("Sponge .schem must be a regular file <= 8 MiB"));
    }
    decode_sponge_v2(world,dimension,&fs::read(path)?)
}
fn nbt_tag(kind:u8,name:&str,data:&[u8])->Result<Vec<u8>,WorldError>{
    let n=u16::try_from(name.len()).map_err(|_|err("Sponge NBT field too long"))?;
    let mut tag=vec![kind];
    tag.extend_from_slice(&n.to_be_bytes());
    tag.extend_from_slice(name.as_bytes());
    tag.extend_from_slice(data);
    Ok(tag)
}
/// Export one BE-free selected modern block clipboard to original Sponge
/// schematic v2 GZip NBT. Never overwrites an existing file or source world.
pub fn export_sponge_v2(world:&WorldSummary,clip:&BlockClipboard,path:&Path)
    ->Result<(),WorldError>{
    clip.validate(world,&clip.dimension)?;
    if clip.numeric{return Err(err("Sponge v2 writer requires named BlockStates"));}
    let dv=world.data_version.ok_or_else(||err("World has no DataVersion for Sponge export"))?;
    if path.extension().and_then(|s|s.to_str())!=Some("schem"){
        return Err(err("Sponge export requires .schem extension"));
    }
    let parent=path.parent().filter(|p|!p.as_os_str().is_empty())
        .unwrap_or_else(||Path::new("."));
    if fs::canonicalize(parent)?.starts_with(fs::canonicalize(&world.root)?){
        return Err(err("Sponge files must be exported outside source world"));
    }
    let mut lookup:BTreeMap<String,u32>=BTreeMap::new();
    let mut ordered=Vec::new();
    let mut block_data=Vec::new();
    for block in &clip.blocks{
        let name=render_state(block);
        let index=if let Some(value)=lookup.get(&name){*value}else{
            let id=u32::try_from(lookup.len()).map_err(|_|err("Too many palette entries"))?;
            lookup.insert(name.clone(),id);
            ordered.push((name,id));
            id
        };
        let mut value=index;
        loop{
            let mut next=(value&0x7f) as u8;
            value>>=7;
            if value!=0{next|=0x80;}
            block_data.push(next);
            if value==0{break;}
        }
    }
    let mut palette=Vec::new();
    for (name,index) in ordered{
        palette.extend(nbt_tag(3,&name,&(index as i32).to_be_bytes())?);
    }
    palette.push(0);
    let mut fields=Vec::new();
    fields.extend(nbt_tag(3,"Version",&2i32.to_be_bytes())?);
    fields.extend(nbt_tag(3,"DataVersion",&dv.to_be_bytes())?);
    for (name,value) in [("Width",clip.size[0]),("Height",clip.size[1]),
        ("Length",clip.size[2])]{
        fields.extend(nbt_tag(2,name,&value.to_be_bytes())?);
    }
    let mut offset=3i32.to_be_bytes().to_vec();
    offset.extend([0u8;12]);
    fields.extend(nbt_tag(11,"Offset",&offset)?);
    fields.extend(nbt_tag(3,"PaletteMax",&(lookup.len() as i32).to_be_bytes())?);
    fields.extend(nbt_tag(10,"Palette",&palette)?);
    let mut payload=(block_data.len() as i32).to_be_bytes().to_vec();
    payload.extend(block_data);
    fields.extend(nbt_tag(7,"BlockData",&payload)?);
    fields.extend(nbt_tag(9,"BlockEntities",&[10,0,0,0,0])?);
    fields.extend(nbt_tag(9,"Entities",&[10,0,0,0,0])?);
    fields.push(0);
    let raw=nbt_tag(10,"Schematic",&fields)?;
    if raw.len()>16*1024*1024{return Err(err("Uncompressed Sponge NBT too large"));}
    let mut zipper=GzEncoder::new(Vec::new(),Compression::default());
    zipper.write_all(&raw)?;
    let compressed=zipper.finish()?;
    if compressed.len()>8*1024*1024{return Err(err("Compressed Sponge NBT too large"));}
    let mut file=OpenOptions::new().write(true).create_new(true).open(path)?;
    let written=file.write_all(&compressed).and_then(|_|file.sync_all());
    drop(file);
    if let Err(e)=written{
        let _=fs::remove_file(path);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder,Compression};
    use std::io::Write;
    fn tag(kind:u8,key:&str,data:&[u8])->Vec<u8>{
        let mut out=vec![kind];out.extend((key.len() as u16).to_be_bytes());
        out.extend(key.as_bytes());out.extend(data);out
    }
    fn sample(include_be:bool)->Vec<u8>{
        let mut fields=vec![
            tag(2,"Width",&2i16.to_be_bytes()),
            tag(2,"Height",&1i16.to_be_bytes()),
            tag(2,"Length",&1i16.to_be_bytes()),
            tag(8,"Materials",&[0,5,b'A',b'l',b'p',b'h',b'a']),
            tag(7,"Blocks",&[0,0,0,2,1,45]),
            tag(7,"Data",&[0,0,0,2,0,7]),
            tag(7,"AddBlocks",&[0,0,0,1,0x10]),
        ];
        if include_be{
            let mut list=vec![10];list.extend(1i32.to_be_bytes());
            list.push(0);
            fields.push(tag(9,"TileEntities",&list));
        }
        let mut body=fields.concat();body.push(0);
        let raw=tag(10,"Schematic",&body);
        let mut zip=GzEncoder::new(Vec::new(),Compression::default());
        zip.write_all(&raw).unwrap();zip.finish().unwrap()
    }

#[test]
fn strict_sponge_v2_named_block_roundtrip_preserves_states_and_properties(){
    use crate::clipboard;
    use crate::sections::State;
    use std::sync::atomic::{AtomicU64,Ordering};
    static NEXT:AtomicU64=AtomicU64::new(0);
    let location=std::env::temp_dir().join(format!(
        "mcedit-sponge-test-{}-{}",std::process::id(),
        NEXT.fetch_add(1,Ordering::Relaxed)
    ));
    fs::create_dir_all(&location).unwrap();
    let world=WorldSummary{
        root:location.clone(),level_name:None,data_version:Some(3955),
        storage_format:None,version_name:None,dimensions:vec![],warnings:vec![]
    };
    let blocks=vec![
        State{name:"minecraft:stone".into(),properties:vec![]},
        State{name:"minecraft:oak_log".into(),properties:vec![("axis".into(),"y".into())]},
        State{name:"minecraft:stone".into(),properties:vec![]},
        State{name:"mod:custom_machine".into(),properties:vec![("facing".into(),"north".into())]},
    ];
    let clipboard=clipboard::new(
        &world,"minecraft:overworld",false,[0,0,0],[2,1,2],blocks.clone()
    ).unwrap();
    let output=std::env::temp_dir().join(format!(
        "mcedit2-sponge-{}-{}.schem",std::process::id(),
        NEXT.fetch_add(1,Ordering::Relaxed)
    ));
    export_sponge_v2(&world,&clipboard,&output).unwrap();
    let restored=import_sponge_v2(&world,"minecraft:overworld",&output).unwrap();
    assert_eq!(restored.size,[2,1,2]);
    assert_eq!(restored.blocks,blocks);
    assert!(export_sponge_v2(&world,&clipboard,&output).is_err());
    let wrong_version=WorldSummary{data_version:Some(5023),..world.clone()};
    assert!(import_sponge_v2(&wrong_version,"minecraft:overworld",&output).is_err());
    fs::remove_file(output).unwrap();
    fs::remove_dir_all(location).unwrap();
}
    #[test]
    fn old_schematic_preserves_addblocks_and_refuses_block_entities(){
        let w=WorldSummary{
            root:std::env::temp_dir().join("mcedit-schem-unit"),
            level_name:None,data_version:None,storage_format:None,
            version_name:None,dimensions:vec![],warnings:vec![],
        };
        let clip=decode_legacy(&w,"minecraft:overworld",&sample(false)).unwrap();
        assert_eq!(clip.size,[2,1,1]);
        assert_eq!(clip.blocks[1].name,"numeric:301");
        assert_eq!(clip.blocks[1].properties[0].1,"7");
        assert!(decode_legacy(&w,"minecraft:overworld",&sample(true)).is_err());
    }
}
