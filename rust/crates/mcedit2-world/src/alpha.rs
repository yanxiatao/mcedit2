//! Original Alpha (and early Beta) individual Java chunk files.
//! Storage: world/{base36(cx & 63)}/{base36(cz & 63)}/c.{cx36}.{cz36}.dat
//! Only original read-only files. Unknown NBT is NEVER reserialized.
use std::fs;
use std::io::{Read};
use std::path::{Path,PathBuf};
use flate2::read::GzDecoder;
use crate::sections::{decode_sections,Chunk};
use crate::WorldError;

const MAX_CHUNKS:usize=2_000_000;
const MAX_CHUNK_FILE:u64=32*1024*1024;
const MAX_NBT:usize=64*1024*1024;

fn invalid(x:impl Into<String>)->WorldError{WorldError::Invalid(x.into())}
fn radix(mut unsigned:u32)->String{
    if unsigned==0{return "0".into();}
    let alphabet=b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut tmp=Vec::new();
    while unsigned>0 {
        tmp.push(alphabet[(unsigned%36) as usize] as char);
        unsigned/=36;
    }
    tmp.iter().rev().collect()
}
fn signed36(value:i32)->String{
    if value<0 {format!("-{}",radix(value.unsigned_abs()))}
    else {radix(value as u32)}
}
fn parse_signed36(text:&str)->Option<i32>{
    if let Some(positive)=text.strip_prefix('-') {
        let magnitude=u32::from_str_radix(positive,36).ok()?;
        if magnitude==2147483648 {Some(i32::MIN)}
        else {i32::try_from(magnitude).ok().map(|x|-x)}
    } else {
        i32::from_str_radix(text,36).ok()
    }
}
pub fn alpha_chunk_path(world:&Path,x:i32,z:i32)->PathBuf{
    world.join(radix((x&63) as u32))
         .join(radix((z&63) as u32))
         .join(format!("c.{}.{}.dat",signed36(x),signed36(z)))
}
/// Validate every original directory and filename, never recursively
/// traverse arbitrary paths or follow symlinks outside a world.
pub fn find_alpha_chunks(world:&Path)->Result<Vec<(i32,i32,PathBuf)>,WorldError>{
    let mut result=Vec::new();
    for first in fs::read_dir(world)?{
        let first=first?;
        if !first.file_type()?.is_dir(){continue;}
        let name=first.file_name().to_string_lossy().to_string();
        let Some(xbucket)=u32::from_str_radix(&name,36).ok() else {continue;};
        if xbucket>=64||name!=radix(xbucket){continue;}
        for second in fs::read_dir(first.path())?{
            let second=second?;
            if !second.file_type()?.is_dir(){continue;}
            let name=second.file_name().to_string_lossy().to_string();
            let Some(zbucket)=u32::from_str_radix(&name,36).ok() else {continue;};
            if zbucket>=64||name!=radix(zbucket){continue;}
            for file in fs::read_dir(second.path())? {
                let file=file?;
                if !file.file_type()?.is_file(){continue;}
                let name=file.file_name().to_string_lossy().to_string();
                let components:Vec<_>=name.split('.').collect();
                if components.len()!=4||components[0]!="c"||components[3]!="dat"{
                    continue;
                }
                let (Some(x),Some(z))=(
                    parse_signed36(components[1]),parse_signed36(components[2])
                ) else {continue;};
                if ((x&63) as u32)!=xbucket||((z&63) as u32)!=zbucket
                    ||alpha_chunk_path(world,x,z)!=file.path(){
                    return Err(invalid("Alpha original chunk filename/bucket mismatch"));
                }
                result.push((x,z,file.path()));
                if result.len()>MAX_CHUNKS {
                    return Err(invalid("Alpha world exceeds chunk indexing safety budget"));
                }
            }
        }
    }
    result.sort_by_key(|x|(x.0,x.1));
    result.dedup_by(|a,b|a.0==b.0&&a.1==b.1);
    Ok(result)
}
/// Decode one original Alpha chunk using its canonical path and check
/// both original internal chunk coordinates before allowing inspection.
pub fn read_alpha_chunk_bytes(world:&Path,x:i32,z:i32)->Result<Option<Vec<u8>>,WorldError>{
    let p=alpha_chunk_path(world,x,z);
    if !p.is_file(){return Ok(None);}
    // Canonicalize to guard against file symlinks and other path tricks.
    let canon_world=world.canonicalize()?;
    let canon_file=p.canonicalize()?;
    if !canon_file.starts_with(&canon_world) {
        return Err(invalid("Alpha chunk resolved outside original world"));
    }
    let size=fs::metadata(&canon_file)?.len();
    if size>MAX_CHUNK_FILE {
        return Err(invalid("Original Alpha chunk gzip exceeds 32 MiB"));
    }
    let file=fs::File::open(&canon_file)?;
    let mut decoder=GzDecoder::new(file);
    let mut bytes=Vec::new();
    decoder.by_ref().take((MAX_NBT+1) as u64).read_to_end(&mut bytes)?;
    if bytes.len()>MAX_NBT {
        return Err(invalid("Original Alpha chunk NBT exceeds 64 MiB"));
    }
    let chunk=decode_sections(&bytes)
        .map_err(|e|invalid(format!("Alpha chunk original NBT invalid: {e}")))?;
    if chunk.x!=Some(x)||chunk.z!=Some(z) {
        return Err(invalid(format!(
            "Alpha chunk coordinates mismatch: file ({x},{z}) vs stored {:?},{:?}",
            chunk.x,chunk.z
        )));
    }
    Ok(Some(bytes))
}
/// Decode original Alpha block data without returning the raw bytes to callers.
pub fn read_alpha_chunk(world:&Path,x:i32,z:i32)->Result<Option<Chunk>,WorldError>{
    read_alpha_chunk_bytes(world,x,z)?.map(|raw|
        decode_sections(&raw).map_err(|e|invalid(format!("Alpha chunk NBT invalid: {e}")))
    ).transpose()
}
#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder,Compression};
    use std::io::Write;
    fn tag(kind:u8,name:&str,payload:&[u8])->Vec<u8>{
        let mut value=vec![kind];
        value.extend((name.len() as u16).to_be_bytes());value.extend(name.as_bytes());
        value.extend(payload);value
    }
    fn compound(children:Vec<Vec<u8>>)->Vec<u8>{
        let mut x=children.concat();x.push(0);x
    }
    fn array(name:&str,v:&[u8])->Vec<u8>{
        let mut bytes=(v.len() as i32).to_be_bytes().to_vec();
        bytes.extend(v);tag(7,name,&bytes)
    }
    #[test]
    fn canonical_alpha_hash_for_negative_chunk_coordinates(){
        let path=alpha_chunk_path(Path::new("world"),-13,44);
        assert_eq!(path,PathBuf::from("world/1f/18/c.-d.18.dat"));
        assert_eq!(parse_signed36("-zik0zj"),Some(-2147483647));
    }
    #[test]
    fn gzip_original_alpha_chunk_reads_and_refuses_wrong_internal_position(){
        let root=std::env::temp_dir().join(format!("mcedit2-alpha-{}",std::process::id()));
        let _=fs::remove_dir_all(&root);
        let path=alpha_chunk_path(&root,-13,44);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let body=compound(vec![
            tag(3,"xPos",&(-13i32).to_be_bytes()),
            tag(3,"zPos",&44i32.to_be_bytes()),
            array("Blocks",&vec![0u8;32768]),
            array("Data",&vec![0u8;16384]),
            tag(8,"OpaqueMod",&[0,4,b't',b'e',b's',b't']),
        ]);
        let chunk=tag(10,"",&compound(vec![tag(10,"Level",&body)]));
        let mut encoder=GzEncoder::new(Vec::new(),Compression::default());
        encoder.write_all(&chunk).unwrap();
        fs::write(&path,encoder.finish().unwrap()).unwrap();
        assert_eq!(find_alpha_chunks(&root).unwrap().len(),1);
        let loaded=read_alpha_chunk(&root,-13,44).unwrap().unwrap();
        assert_eq!(loaded.x,Some(-13));
        assert_eq!(loaded.z,Some(44));
        assert_eq!(loaded.sections.len(),8);
        assert!(read_alpha_chunk(&root,-12,44).unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
