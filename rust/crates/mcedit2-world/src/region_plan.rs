//! In-memory Java MCA/MCR region image planning. NEVER writes original files.
//!
//! The builder preserves all original region sectors (including padding,
//! compression streams, unreferenced sectors and all untouched chunk slots).
//! Edited chunks are compressed and appended as fresh sectors; only their
//! 4-byte location and 4-byte timestamp header entries are changed.
//! A future disk transaction must revalidate the WHOLE original file and
//! stage backups before replacing it.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read,Write};
use std::path::PathBuf;
use flate2::{Compression,read::{GzDecoder,ZlibDecoder},write::{GzEncoder,ZlibEncoder}};
use sha2::{Digest,Sha256};
use crate::editing::NativeChunkCandidate;
use crate::sections::{decode_sections,patch_named_growing_palette,patch_original_numeric_arrays,patch_original_block_entity_scalar,Chunk,InPaletteEdit,Layout};
use crate::{WorldError,WorldSummary};

const SECTOR:usize=4096;
const HEADER:usize=2*SECTOR;
const MAX_IMAGE:usize=256*1024*1024;
const MAX_NBT:usize=64*1024*1024;
const MAX_COMPRESSED:usize=32*1024*1024;
const MAX_CHUNK_SECTORS:usize=255;
const MAX_REGION_SECTORS:usize=0x00ff_ffff;
fn refuse(text:impl Into<String>)->WorldError{WorldError::Invalid(text.into())}
fn hash(raw:&[u8])->String{Sha256::digest(raw).iter().map(|b|format!("{b:02x}")).collect()}
#[derive(Debug,Clone,Copy)]
struct Slot {start:usize,sectors:usize}
fn parse_slots(bytes:&[u8])->Result<[Option<Slot>;1024],WorldError>{
    if bytes.len()<HEADER||bytes.len()%SECTOR!=0||bytes.len()>MAX_IMAGE {
        return Err(refuse("Region image is truncated, not sector-aligned or exceeds 256 MiB"));
    }
    let sector_count=bytes.len()/SECTOR;
    let mut occupied=vec![false;sector_count];
    occupied[0]=true;occupied[1]=true;
    let mut slots=[None;1024];
    for (idx,slot) in slots.iter_mut().enumerate(){
        let head=idx*4;
        let entry=u32::from_be_bytes(bytes[head..head+4].try_into().unwrap());
        if entry==0{continue;}
        let start=(entry>>8) as usize;
        let count=(entry&255) as usize;
        let end=start.checked_add(count)
            .ok_or_else(||refuse("Region pointer arithmetic overflow"))?;
        if start<2||count==0||end>sector_count{
            return Err(refuse(format!("Invalid original region sector pointer slot {idx}")));
        }
        for used in &mut occupied[start..end]{
            if *used {return Err(refuse(format!(
                "Original region contains overlapping sector allocations at slot {idx}"
            )));}
            *used=true;
        }
        let off=start*SECTOR;
        let length=u32::from_be_bytes(bytes[off..off+4].try_into().unwrap()) as usize;
        if length<1||length+4>count*SECTOR||length>MAX_COMPRESSED{
            return Err(refuse(format!("Malformed original compressed chunk length slot {idx}")));
        }
        *slot=Some(Slot{start,sectors:count});
    }
    Ok(slots)
}
fn bounded_decompress(reader:impl Read)->Result<Vec<u8>,WorldError>{
    let mut out=Vec::new();
    reader.take((MAX_NBT+1) as u64).read_to_end(&mut out)?;
    if out.len()>MAX_NBT{return Err(refuse("Original decompressed NBT exceeds 64 MiB"));}
    Ok(out)
}
fn original_nbt(image:&[u8],slot:Slot)->Result<(u8,Vec<u8>),WorldError>{
    let base=slot.start*SECTOR;
    let length=u32::from_be_bytes(image[base..base+4].try_into().unwrap()) as usize;
    if length<1||length+4>slot.sectors*SECTOR||length>MAX_COMPRESSED {
        return Err(refuse("Touched original chunk has invalid compressed length"));
    }
    let method=image[base+4];
    let compressed=&image[base+5..base+4+length];
    // A compressor flag with bit 7 set denotes an external .mcc stream.
    // We do not rewrite, relocate or invent external sidecar data.
    let nbt=match method{
        1=>bounded_decompress(GzDecoder::new(compressed))?,
        2=>bounded_decompress(ZlibDecoder::new(compressed))?,
        3 if compressed.len()<=MAX_NBT=>compressed.to_vec(),
        3=>return Err(refuse("Raw original NBT exceeds 64 MiB")),
        _=>return Err(refuse(format!(
            "Touched chunk compression {method} unsupported (external .mcc refused)"
        ))),
    };
    Ok((method,nbt))
}
fn compress(method:u8,nbt:&[u8])->Result<Vec<u8>,WorldError>{
    if nbt.len()>MAX_NBT{return Err(refuse("Modified NBT exceeds 64 MiB"));}
    let encoded=match method {
        1=>{let mut zip=GzEncoder::new(Vec::new(),Compression::default());
            zip.write_all(nbt)?;zip.finish()?},
        2=>{let mut zip=ZlibEncoder::new(Vec::new(),Compression::default());
            zip.write_all(nbt)?;zip.finish()?},
        3=>nbt.to_vec(),
        _=>return Err(refuse("Cannot encode unknown/external chunk compression")),
    };
    if encoded.len()>MAX_COMPRESSED{return Err(refuse("Compressed output exceeds 32 MiB"));}
    Ok(encoded)
}
fn equivalent_metadata(old:&Chunk,new:&Chunk)->bool{
    old.layout==new.layout&&old.data_version==new.data_version
        &&old.x==new.x&&old.z==new.z
        &&old.block_entities==new.block_entities
        &&old.sections.len()==new.sections.len()
}
fn count_actual_edits(original:&Chunk,updated:&Chunk)->Result<usize,WorldError>{
    if !equivalent_metadata(original,updated){
        return Err(refuse("New chunk metadata/Block Entity locations differ from source"));
    }
    let mut changed=0;
    for section in &original.sections{
        let dest=updated.sections.iter().find(|s|s.y==section.y)
            .ok_or_else(||refuse("Patched NBT deleted or moved original Section"))?;
        for idx in 0..4096usize {
            let old=section.palette.get(section.indices[idx] as usize)
                .ok_or_else(||refuse("Original palette index missing"))?;
            let new=dest.palette.get(dest.indices[idx] as usize)
                .ok_or_else(||refuse("Patched palette index missing"))?;
            if old!=new{
                let lx=(idx%16) as i32;
                let lz=((idx/16)%16) as i32;
                let y=section.y as i32*16+(idx/256) as i32;
                if original.block_entities.iter().any(|&(x,by,z)|
                    original.x.is_some_and(|cx|x.div_euclid(16)==cx
                        &&x.rem_euclid(16)==lx)
                    &&by==y
                    &&original.z.is_some_and(|cz|z.div_euclid(16)==cz
                        &&z.rem_euclid(16)==lz)
                ){
                    return Err(refuse("Patched chunk changed a Block Entity block state"));
                }
                changed+=1;
            }
        }
    }
    Ok(changed)
}
/// Complete, chronological original-BE surgery proof. The only supported
/// change is an EXISTING Byte/Short/Int/Long/String tag within a fixed BE.
#[derive(Debug,Clone)]
pub struct BeReplayStep {
    pub xyz:[i32;3],
    pub field:String,
    pub value:String,
}
impl BeReplayStep {
    fn chunk(&self)->(i32,i32){
        (self.xyz[0].div_euclid(16),self.xyz[2].div_euclid(16))
    }
}
fn replay_be_proofs(raw:&[u8],proofs:&[&BeReplayStep])
    ->Result<Vec<u8>,WorldError>{
    let mut patched=raw.to_vec();
    for step in proofs{
        patched=patch_original_block_entity_scalar(
            &patched,(step.xyz[0],step.xyz[1],step.xyz[2]),
            &step.field,&step.value
        ).map_err(|e|refuse(format!("Exact BE NBT scalar replay rejected: {e}")))?;
    }
    Ok(patched)
}

/// Complete memory-only Region rewrite candidate; original path and original
/// file SHA-256 are retained for the later backup/atomic writer.
#[derive(Debug,Clone)]
pub struct NativeRegionCandidate {
    pub path:PathBuf,
    pub original_region_sha256:String,
    pub region_x:i32,
    pub region_z:i32,
    pub modified_chunks:usize,
    /// Exact replay list required for every proposed Block Entity change.
    pub be_replay:Vec<BeReplayStep>,
    pub original_bytes:usize,
    pub planned_bytes:usize,
    /// Full candidate image; NEVER write this directly without a transaction.
    pub image:Vec<u8>,
}
fn plan_region_image(
    original:&[u8],region_x:i32,region_z:i32,
    changes:&[&NativeChunkCandidate],timestamp:u32,
)->Result<Vec<u8>,WorldError>{
    plan_region_image_with_be(original,region_x,region_z,changes,&[],timestamp)
}
fn plan_region_image_with_be(
    original:&[u8],region_x:i32,region_z:i32,
    changes:&[&NativeChunkCandidate],proofs:&[BeReplayStep],timestamp:u32,
)->Result<Vec<u8>,WorldError>{
    if changes.is_empty()||changes.len()>1024{
        return Err(refuse("Region plan requires 1..1024 existing changed chunks"));
    }
    let locations=parse_slots(original)?;
    let mut seen=BTreeSet::new();
    let mut image=original.to_vec();
    for change in changes{
        if change.chunk_x.div_euclid(32)!=region_x
            ||change.chunk_z.div_euclid(32)!=region_z {
            return Err(refuse("Region edit belongs to another original region"));
        }
        let local_x=change.chunk_x.rem_euclid(32) as usize;
        let local_z=change.chunk_z.rem_euclid(32) as usize;
        let slot_index=local_z*32+local_x;
        if !seen.insert(slot_index) {return Err(refuse("Duplicate edited region chunk slot"));}
        let old=locations[slot_index]
            .ok_or_else(||refuse("Region image has no original chunk for candidate"))?;
        let (compression,nbt)=original_nbt(original,old)?;
        if hash(&nbt)!=change.original_nbt_sha256 {
            return Err(refuse("Original decompressed chunk SHA-256 changed since preview"));
        }
        let decoded=decode_sections(&nbt)
            .map_err(|e|refuse(format!("Original region NBT rejected: {e}")))?;
        if decoded.x.is_some_and(|x|x!=change.chunk_x)
            ||decoded.z.is_some_and(|z|z!=change.chunk_z) {
            return Err(refuse("Original NBT coordinates mismatch region chunk slot"));
        }
        let patched=decode_sections(&change.patched_nbt)
            .map_err(|e|refuse(format!("Modified region NBT rejected: {e}")))?;
        let changed=count_actual_edits(&decoded,&patched)?;
        let proof_count=proofs.iter().filter(|step|
            step.chunk()==(change.chunk_x,change.chunk_z)
        ).count();
        if changed!=change.edited_blocks || (changed==0&&proof_count==0) {
            return Err(refuse("Planned actual block/BE changes mismatch source preview"));
        }
        if proof_count>0{
            let ops=proofs.iter().filter(|step|
                step.chunk()==(change.chunk_x,change.chunk_z)
            ).collect::<Vec<_>>();
            // The only BE edits allowed are exact replayed scalar operations.
            let expected=if changed==0{nbt.clone()}else{
                // The block-only replay is constructed from the actual
                // original/updated palettes by the transaction verifier.
                let mut edits=Vec::new();
                for section in &decoded.sections{
                    let next=patched.sections.iter().find(|s|s.y==section.y)
                        .ok_or_else(||refuse("BE candidate Section missing"))?;
                    for idx in 0..4096usize{
                        let old=&section.palette[section.indices[idx] as usize];
                        let new=&next.palette[next.indices[idx] as usize];
                        if old!=new{
                            edits.push(InPaletteEdit{
                                local_x:(idx%16) as u8,
                                local_z:((idx/16)%16) as u8,
                                y:section.y as i32*16+(idx/256) as i32,
                                original:old.clone(),replacement:new.clone(),
                            });
                        }
                    }
                }
                edits.sort_by_key(|e|(e.local_x,e.y,e.local_z));
                match decoded.layout{
                    Layout::Named113|Layout::Modern118=>
                        patch_named_growing_palette(&nbt,&edits),
                    Layout::McRegion128|Layout::NumericAnvil112=>
                        patch_original_numeric_arrays(&nbt,&edits),
                }.map_err(|e|refuse(format!("BE planner block replay failed: {e}")))?
            };
            if replay_be_proofs(&expected,&ops)?!=change.patched_nbt{
                return Err(refuse("BE Region candidate changes NBT beyond declared scalar proofs"));
            }
        }
        let encoded=compress(compression,&change.patched_nbt)?;
        let length=encoded.len().checked_add(1)
            .ok_or_else(||refuse("Compressed chunk size overflow"))?;
        let used=length.checked_add(4)
            .ok_or_else(||refuse("Chunk sector allocation overflow"))?;
        let sectors=used.div_ceil(SECTOR);
        if sectors==0||sectors>MAX_CHUNK_SECTORS{
            return Err(refuse("Edited chunk needs more than 255 internal sectors; sidecar unsupported"));
        }
        let sector=image.len()/SECTOR;
        let final_sectors=sector.checked_add(sectors)
            .ok_or_else(||refuse("Region sector pointer overflow"))?;
        let new_len=final_sectors.checked_mul(SECTOR)
            .ok_or_else(||refuse("Region output size overflow"))?;
        if sector>MAX_REGION_SECTORS||new_len>MAX_IMAGE{
            return Err(refuse("Region plan exceeds sector address/image safety budget"));
        }
        image.resize(new_len,0);
        let start=sector*SECTOR;
        image[start..start+4].copy_from_slice(&(length as u32).to_be_bytes());
        image[start+4]=compression;
        image[start+5..start+5+encoded.len()].copy_from_slice(&encoded);
        let offset=slot_index*4;
        image[offset..offset+4].copy_from_slice(
            &(((sector as u32)<<8)|(sectors as u32)).to_be_bytes()
        );
        let when=4096+slot_index*4;
        image[when..when+4].copy_from_slice(&timestamp.to_be_bytes());
    }
    // Validate the entire proposed sector map and EVERY replaced chunk's
    // resulting compressed NBT, not just its decoded in-memory candidate.
    let new_slots=parse_slots(&image)?;
    for change in changes{
        let index=change.chunk_z.rem_euclid(32) as usize*32
            +change.chunk_x.rem_euclid(32) as usize;
        let (_,roundtrip)=original_nbt(&image,new_slots[index]
            .ok_or_else(||refuse("Planned chunk location vanished"))?)?;
        if roundtrip!=change.patched_nbt{
            return Err(refuse("Compressed region candidate failed full NBT roundtrip"));
        }
    }
    // Unedited sector allocations and their complete raw byte contents stay
    // at their exact original offsets; only edited header entries differ.
    for index in 0..1024 {
        if seen.contains(&index){continue;}
        if locations[index].map(|s|(s.start,s.sectors))
            !=new_slots[index].map(|s|(s.start,s.sectors)) {
            return Err(refuse("Plan changed an unedited original sector pointer"));
        }
        let head=index*4;
        if image[head..head+4]!=original[head..head+4]
            ||image[4096+head..4096+head+4]!=original[4096+head..4096+head+4]{
            return Err(refuse("Plan unexpectedly changed an unedited header entry"));
        }
    }
    if image[HEADER..original.len()]!=original[HEADER..]{
        return Err(refuse("Region plan modified an original sector payload"));
    }
    Ok(image)
}


/// Reconstruct the expected edited NBT with our ONLY allowed block-byte
/// adapters, then compare EVERY output byte. This is essential: a public
/// NativeRegionCandidate must never smuggle changes to opaque modded tags,
/// entities, inventory, or Block Entity NBT into an on-disk transaction.
fn verify_exact_block_only_replay(
    original_nbt:&[u8],patched_nbt:&[u8],before:&Chunk,after:&Chunk,
    proofs:&[&BeReplayStep],
)->Result<(),WorldError>{
    if !equivalent_metadata(before,after) {
        return Err(refuse("Replayed NBT metadata or BE coordinates differ"));
    }
    let mut edits=Vec::new();
    for section in &before.sections {
        let updated=after.sections.iter().find(|s|s.y==section.y)
            .ok_or_else(||refuse("Original NBT Section missing during replay"))?;
        for index in 0..4096usize {
            let initial=section.palette.get(section.indices[index] as usize)
                .ok_or_else(||refuse("Original state index invalid during replay"))?;
            let final_state=updated.palette.get(updated.indices[index] as usize)
                .ok_or_else(||refuse("Replacement state index invalid during replay"))?;
            if initial!=final_state {
                edits.push(InPaletteEdit {
                    local_x:(index%16) as u8,
                    local_z:((index/16)%16) as u8,
                    y:section.y as i32*16+(index/256) as i32,
                    original:initial.clone(),replacement:final_state.clone(),
                });
            }
        }
    }
    if edits.is_empty()&&proofs.is_empty(){
        return Err(refuse("No actual block or authenticated BE scalar edits in candidate"));
    }
    // Session's overlay is an ordered BTreeMap by world X,Y,Z. Repeating the
    // same deterministic order reconstructs palette growth insertion order.
    edits.sort_by_key(|e|(e.local_x,e.y,e.local_z));
    let expected=if edits.is_empty(){original_nbt.to_vec()}else{
        match before.layout {
            Layout::Named113|Layout::Modern118=>
                patch_named_growing_palette(original_nbt,&edits),
            Layout::McRegion128|Layout::NumericAnvil112=>
                patch_original_numeric_arrays(original_nbt,&edits),
        }.map_err(|e|refuse(format!("Canonical NBT block-only replay rejected: {e}")))?
    };
    let expected=replay_be_proofs(&expected,proofs)?;
    if expected!=patched_nbt {
        return Err(refuse("Region candidate has unproven changes to opaque original NBT"));
    }
    Ok(())
}
/// Prove that a candidate differs from the original region in exactly its
/// declared chunk-header entries and newly appended sectors. Reject forged,
/// stale, reordered or partially overwritten Region images before staging.
pub(crate) fn verify_candidate(original:&[u8],candidate:&NativeRegionCandidate)
    -> Result<(),WorldError>{
    if hash(original)!=candidate.original_region_sha256
        || original.len()!=candidate.original_bytes
        || candidate.image.len()!=candidate.planned_bytes {
        return Err(refuse("Region candidate original digest or image size differs"));
    }
    if candidate.be_replay.len()>24{
        return Err(refuse("Region BE proof exceeds 24 original scalar operations"));
    }
    let original_slots=parse_slots(original)?;
    let image_slots=parse_slots(&candidate.image)?;
    if candidate.image.len()<=original.len()
        || candidate.image.get(HEADER..original.len())!=Some(&original[HEADER..]) {
        return Err(refuse("Region candidate failed original-sector preservation"));
    }
    let original_end=original.len()/SECTOR;
    let mut modified=0;
    let mut touched=BTreeSet::new();
    let mut ranges=Vec::new();
    for slot_index in 0..1024 {
        let header=slot_index*4;
        let old=original_slots[slot_index];
        let new=image_slots[slot_index];
        let same=old.map(|s|(s.start,s.sectors))==new.map(|s|(s.start,s.sectors));
        if same {
            if original[header..header+4]!=candidate.image[header..header+4]
                || original[4096+header..4096+header+4]
                    !=candidate.image[4096+header..4096+header+4] {
                return Err(refuse("Unmodified original region header/timestamp was changed"));
            }
            continue;
        }
        let source=old.ok_or_else(||refuse("Candidate created a previously missing original chunk"))?;
        let destination=new.ok_or_else(||refuse("Candidate removed an original chunk"))?;
        if destination.start<original_end {
            return Err(refuse("Candidate repointed a chunk into original sector bytes"));
        }
        let (source_method,source_nbt)=original_nbt(original,source)?;
        let (target_method,target_nbt)=original_nbt(&candidate.image,destination)?;
        if source_method!=target_method {
            return Err(refuse("Candidate changed original chunk compression method"));
        }
        let before=decode_sections(&source_nbt)
            .map_err(|e|refuse(format!("Original chunk decode failed: {e}")))?;
        let after=decode_sections(&target_nbt)
            .map_err(|e|refuse(format!("Replacement chunk decode failed: {e}")))?;
        if before.x.is_some_and(|x|x.div_euclid(32)!=candidate.region_x
            ||x.rem_euclid(32)!=(slot_index%32) as i32)
            ||before.z.is_some_and(|z|z.div_euclid(32)!=candidate.region_z
            ||z.rem_euclid(32)!=(slot_index/32) as i32) {
            return Err(refuse("Candidate original chunk coordinate does not match its Region slot"));
        }
        let chunk_coord=(
            candidate.region_x*32+(slot_index%32) as i32,
            candidate.region_z*32+(slot_index/32) as i32
        );
        touched.insert(chunk_coord);
        let proofs=candidate.be_replay.iter()
            .filter(|step|step.chunk()==chunk_coord).collect::<Vec<_>>();
        if count_actual_edits(&before,&after)?==0&&proofs.is_empty(){
            return Err(refuse("Region candidate replaced a chunk without proven edits"));
        }
        verify_exact_block_only_replay(&source_nbt,&target_nbt,&before,&after,&proofs)?;
        ranges.push((destination.start,destination.start+destination.sectors));
        modified+=1;
    }
    if candidate.be_replay.iter().any(|step|!touched.contains(&step.chunk())){
        return Err(refuse("BE proof refers to a chunk not changed by the Region image"));
    }
    if modified!=candidate.modified_chunks || modified==0 {
        return Err(refuse("Region candidate modified-chunk count mismatch"));
    }
    ranges.sort_unstable();
    let mut expected_start=original_end;
    for (start,end) in ranges {
        if start!=expected_start {
            return Err(refuse("Candidate has unexplained appended sector bytes or gaps"));
        }
        expected_start=end;
    }
    if expected_start!=candidate.image.len()/SECTOR {
        return Err(refuse("Region candidate has unreferenced appended sectors"));
    }
    Ok(())
}

/// Group current verified original-NBT chunk candidates by their existing
/// MCA/MCR file. The original bytes are re-read and hashed before returning.
pub(crate) fn plan_world_regions(
    world:&WorldSummary,dimension:&str,changes:&[NativeChunkCandidate],timestamp:u32,
)->Result<Vec<NativeRegionCandidate>,WorldError>{
    plan_world_regions_with_be(world,dimension,changes,&[],timestamp)
}
pub(crate) fn plan_world_regions_with_be(
    world:&WorldSummary,dimension:&str,changes:&[NativeChunkCandidate],
    proofs:&[BeReplayStep],timestamp:u32,
)->Result<Vec<NativeRegionCandidate>,WorldError>{
    if proofs.len()>24{return Err(refuse("Too many BE scalar proofs"));}
    let changed=changes.iter().map(|v|(v.chunk_x,v.chunk_z))
        .collect::<BTreeSet<_>>();
    if proofs.iter().any(|p|!changed.contains(&p.chunk())){
        return Err(refuse("Region planner was supplied an unrelated BE edit proof"));
    }
    let dim=world.dimensions.iter().find(|dim|dim.name==dimension)
        .ok_or_else(||refuse("Unknown source dimension for region planning"))?;
    if dim.extension.contains("c.x.z.dat") {
        return Err(refuse("Alpha individual chunk files require a separate save planner"));
    }
    if changes.is_empty() {return Err(refuse("No native chunk candidates to plan"));}
    let mut groups:BTreeMap<(i32,i32),Vec<&NativeChunkCandidate>>=BTreeMap::new();
    for candidate in changes {
        groups.entry((candidate.chunk_x.div_euclid(32),
            candidate.chunk_z.div_euclid(32))).or_default().push(candidate);
    }
    let mut result=Vec::with_capacity(groups.len());
    for ((rx,rz),changed) in groups{
        let mca=dim.region_dir.join(format!("r.{rx}.{rz}.mca"));
        let mcr=dim.region_dir.join(format!("r.{rx}.{rz}.mcr"));
        if mca.exists()&&mcr.exists(){
            return Err(refuse("Ambiguous original region: .mca and .mcr both exist"));
        }
        let path=if mca.exists(){mca}else if mcr.exists(){mcr}
            else{return Err(refuse("Original Region file missing; never create chunks"))};
        if fs::symlink_metadata(&path)?.file_type().is_symlink(){
            return Err(refuse("Original region symlink refused"));
        }
        let original=fs::read(&path)?;
        let stamp=hash(&original);
        let region_proofs=proofs.iter().filter(|p|
            p.chunk().0.div_euclid(32)==rx&&p.chunk().1.div_euclid(32)==rz
        ).cloned().collect::<Vec<_>>();
        let image=plan_region_image_with_be(
            &original,rx,rz,&changed,&region_proofs,timestamp
        )?;
        // Catch concurrent modifications during planning.
        if hash(&fs::read(&path)?)!=stamp{
            return Err(refuse("Original region bytes changed during dry-run; refuse"));
        }
        result.push(NativeRegionCandidate{
            path,region_x:rx,region_z:rz,
            original_region_sha256:stamp,
            modified_chunks:changed.len(),
            be_replay:region_proofs,
            original_bytes:original.len(),planned_bytes:image.len(),
            image,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sections::{patch_named_growing_palette,InPaletteEdit,State};
    fn tag(kind:u8,name:&str,payload:&[u8])->Vec<u8>{
        let mut out=vec![kind];out.extend((name.len() as u16).to_be_bytes());
        out.extend(name.as_bytes());out.extend(payload);out
    }
    fn strdata(s:&str)->Vec<u8>{
        let mut v=(s.len() as u16).to_be_bytes().to_vec();
        v.extend(s.as_bytes());v
    }
    fn compound(tags:Vec<Vec<u8>>)->Vec<u8>{let mut v=tags.concat();v.push(0);v}
    fn chunk(cx:i32)->Vec<u8>{
        let palette={
            let mut p=vec![10];p.extend(1i32.to_be_bytes());
            p.extend(compound(vec![tag(8,"Name",&strdata("minecraft:stone"))]));p
        };
        let mut sections=vec![10];sections.extend(1i32.to_be_bytes());
        sections.extend(compound(vec![
            tag(1,"Y",&[0]),
            tag(10,"block_states",&compound(vec![tag(9,"palette",&palette)])),
            tag(8,"OpaqueModdedTag",&strdata("leave this original")),
        ]));
        tag(10,"",&compound(vec![
            tag(3,"xPos",&cx.to_be_bytes()),tag(3,"zPos",&0i32.to_be_bytes()),
            tag(3,"DataVersion",&3955i32.to_be_bytes()),
            tag(9,"sections",&sections),tag(9,"block_entities",&[10,0,0,0,0]),
        ]))
    }
    fn payload(method:u8,nbt:&[u8])->Vec<u8>{
        let compressed=compress(method,nbt).unwrap();
        let mut packet=vec![0;SECTOR];
        assert!(compressed.len()+5<=SECTOR);
        packet[..4].copy_from_slice(&((compressed.len()+1) as u32).to_be_bytes());
        packet[4]=method;
        packet[5..5+compressed.len()].copy_from_slice(&compressed);
        packet
    }
    fn region(method:u8)->Vec<u8>{
        let mut image=vec![0u8;4*SECTOR];
        image[0..4].copy_from_slice(&((2u32<<8)|1).to_be_bytes());
        image[4..8].copy_from_slice(&((3u32<<8)|1).to_be_bytes());
        image[4096..4100].copy_from_slice(&17u32.to_be_bytes());
        image[4100..4104].copy_from_slice(&23u32.to_be_bytes());
        image[2*SECTOR..3*SECTOR].copy_from_slice(&payload(method,&chunk(0)));
        image[3*SECTOR..4*SECTOR].copy_from_slice(&payload(2,&chunk(1)));
        image
    }
    fn changed()->NativeChunkCandidate{
        let source=chunk(0);
        let altered=patch_named_growing_palette(&source,&[InPaletteEdit{
            local_x:4,y:0,local_z:5,
            original:State{name:"minecraft:stone".into(),properties:vec![]},
            replacement:State{name:"minecraft:gold_block".into(),properties:vec![]},
        }]).unwrap();
        NativeChunkCandidate{
            chunk_x:0,chunk_z:0,edited_blocks:1,
            original_nbt_sha256:hash(&source),
            patched_nbt:altered,
        }
    }
    #[test]
    fn original_zlib_gzip_and_raw_sectors_survive_append_only_edit_plan(){
        for method in [1u8,2,3]{
            let original=region(method);
            let new=plan_region_image(&original,0,0,&[&changed()],42).unwrap();
            assert!(new.len()>original.len());
            assert_eq!(&new[HEADER..original.len()],&original[HEADER..]);
            assert_eq!(&new[4..8],&original[4..8]);
            assert_eq!(&new[4100..4104],&original[4100..4104]);
            assert_eq!(&new[4096..4100],&42u32.to_be_bytes());
            assert_eq!(parse_slots(&new).unwrap()[1].unwrap().start,3);
            let target=parse_slots(&new).unwrap()[0].unwrap();
            assert!(target.start>=4);
            let (actual_method,raw)=original_nbt(&new,target).unwrap();
            assert_eq!(actual_method,method);
            assert_eq!(raw,changed().patched_nbt);
            assert_eq!(original,region(method)); // original input remains byte identical
        }
    }
    #[test]
    fn stale_digest_duplicate_missing_and_cross_region_fail_without_writing(){
        let original=region(2);
        let candidate=changed();
        let mut stale=candidate.clone();
        stale.original_nbt_sha256="0".repeat(64);
        assert!(plan_region_image(&original,0,0,&[&stale],3).is_err());
        assert!(plan_region_image(&original,0,0,&[&candidate,&candidate],3).is_err());
        assert!(plan_region_image(&original,1,0,&[&candidate],3).is_err());
        let mut missing=candidate.clone();missing.chunk_x=2;
        assert!(plan_region_image(&original,0,0,&[&missing],3).is_err());
        assert_eq!(original,region(2));
    }
    #[test]
    fn overlapping_invalid_and_external_chunk_sectors_are_refused(){
        let mut overlap=region(2);
        overlap[4..8].copy_from_slice(&((2u32<<8)|1).to_be_bytes());
        assert!(plan_region_image(&overlap,0,0,&[&changed()],5).is_err());
        let mut invalid=region(2);
        invalid[0..4].copy_from_slice(&((250u32<<8)|1).to_be_bytes());
        assert!(plan_region_image(&invalid,0,0,&[&changed()],5).is_err());
        let mut external=region(2);
        external[2*SECTOR+4]=130;
        assert!(plan_region_image(&external,0,0,&[&changed()],5).is_err());
    }
}
