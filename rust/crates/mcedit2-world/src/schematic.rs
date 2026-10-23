//! Read-only old MCEdit Alpha numeric-ID .schematic importer.
use std::fs;
use std::path::Path;
use std::io::Read;
use flate2::read::GzDecoder;
use crate::sections::{parse_root,State,Tag};
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
