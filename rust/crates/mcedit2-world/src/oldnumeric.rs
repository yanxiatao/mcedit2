//! Lossless read-only numeric-ID Java chunks: Anvil 1.2.1-1.12.2
//! and MCRegion Beta-era 16x128x16 original arrays.
use std::collections::HashMap;
use super::sections::{original_block_entities, Chunk, DecodeError, Layout, Section, State, Tag};
type Result<T> = std::result::Result<T, DecodeError>;
fn err(s: impl Into<String>) -> DecodeError { DecodeError(s.into()) }

fn byte_array<'a>(tag: &'a Tag<'_>, expected: usize, label: &str) -> Result<&'a [u8]> {
    if tag.kind != 7 || tag.raw.len() != expected + 4 {
        return Err(err(format!("{label}: invalid original byte array length")));
    }
    let n = i32::from_be_bytes(tag.raw[..4].try_into().unwrap());
    if n != expected as i32 { return Err(err(format!("{label}: count mismatch"))); }
    Ok(&tag.raw[4..])
}
fn nibble(bytes: &[u8], index: usize) -> u8 {
    if index & 1 == 0 { bytes[index / 2] & 15 } else { bytes[index / 2] >> 4 }
}
struct Builder {
    palette: Vec<State>,
    known: HashMap<(u16,u8),u16>,
    indices: Vec<u16>,
}
impl Builder {
    fn new() -> Self {
        Self { palette: Vec::new(), known: HashMap::new(), indices: Vec::with_capacity(4096) }
    }
    fn put(&mut self, id: u16, meta: u8) -> Result<()> {
        let pair=(id,meta);
        let index = if let Some(&i)=self.known.get(&pair) {i} else {
            let i=u16::try_from(self.palette.len()).map_err(|_|err("Numeric palette overflow"))?;
            self.known.insert(pair,i);
            self.palette.push(State {
                // Never make up a modern registry name for modded numeric IDs.
                name:format!("numeric:{id}"),
                properties:vec![("data".to_owned(),meta.to_string())]
            });
            i
        };
        self.indices.push(index);
        Ok(())
    }
    fn finish(self,y:i8)->Section {
        Section{y,palette:self.palette,indices:self.indices}
    }
}
pub(super) fn decode_anvil(root:&Tag<'_>,level:&Tag<'_>,list:&Tag<'_>)->Result<Chunk>{
    if list.kind!=9 || list.items.len()>32 {return Err(err("Invalid numeric Sections list"));}
    let mut sections=Vec::new();
    for section in &list.items {
        if section.kind!=10 {return Err(err("Original numeric Section is not Compound"));}
        let y=section.require("Y",1)?.byte()?;
        if sections.iter().any(|s:&Section|s.y==y){return Err(err("Duplicate original section Y"));}
        let blocks=byte_array(section.require("Blocks",7)?,4096,"Blocks")?;
        let data=byte_array(section.require("Data",7)?,2048,"Data")?;
        let add=section.field("Add").map(|x|byte_array(x,2048,"Add")).transpose()?;
        let mut builder=Builder::new();
        for i in 0..4096 {
            let high=add.map_or(0,|a|nibble(a,i)) as u16;
            builder.put(blocks[i] as u16 | (high << 8),nibble(data,i))?;
        }
        sections.push(builder.finish(y));
    }
    sections.sort_by_key(|s|s.y);
    Ok(Chunk {
        data_version:root.field("DataVersion").map(Tag::i32).transpose()?,
        x:level.field("xPos").map(Tag::i32).transpose()?,
        z:level.field("zPos").map(Tag::i32).transpose()?,
        layout:Layout::NumericAnvil112,sections,
        block_entities:original_block_entities(level,false)?,
    })
}
pub(super) fn decode_mcregion(root:&Tag<'_>,level:&Tag<'_>)->Result<Chunk>{
    let blocks=byte_array(level.require("Blocks",7)?,32768,"MCRegion Blocks")?;
    let data=byte_array(level.require("Data",7)?,16384,"MCRegion Data")?;
    let mut sections=Vec::new();
    for section_y in 0..8 {
        let mut builder=Builder::new();
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    let original=(x*16+z)*128+section_y*16+y;
                    builder.put(blocks[original] as u16,nibble(data,original))?;
                }
            }
        }
        sections.push(builder.finish(section_y as i8));
    }
    Ok(Chunk {
        data_version:root.field("DataVersion").map(Tag::i32).transpose()?,
        x:level.field("xPos").map(Tag::i32).transpose()?,
        z:level.field("zPos").map(Tag::i32).transpose()?,
        layout:Layout::McRegion128,sections,
        block_entities:original_block_entities(level,false)?,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use super::super::sections::decode_sections;
    fn tag(kind:u8,name:&str,data:&[u8])->Vec<u8>{
        let mut v=vec![kind];v.extend((name.len() as u16).to_be_bytes());
        v.extend(name.as_bytes());v.extend(data);v
    }
    fn compound(children:Vec<Vec<u8>>)->Vec<u8>{
        let mut v=children.concat();v.push(0);v
    }
    fn ba(name:&str,payload:&[u8])->Vec<u8>{
        let mut v=(payload.len() as i32).to_be_bytes().to_vec();
        v.extend(payload);tag(7,name,&v)
    }
    fn old_chunk(children:Vec<Vec<u8>>)->Vec<u8>{
        tag(10,"",&compound(vec![tag(10,"Level",&compound(children))]))
    }
    #[test]
    fn original_anvil_numeric_meta_and_add_are_not_lost(){
        let (mut blocks,mut data,mut add)=(vec![0;4096],vec![0;2048],vec![0;2048]);
        let n=(2*16+3)*16+4;
        blocks[n]=45;data[n/2]|=7 << ((n%2)*4);add[n/2]|=1 << ((n%2)*4);
        let section=compound(vec![tag(1,"Y",&[0]),ba("Blocks",&blocks),
          ba("Data",&data),ba("Add",&add),tag(8,"mod:opaque",&[0,1,42])]);
        let mut list=vec![10];list.extend(1i32.to_be_bytes());list.extend(section);
        let input=old_chunk(vec![tag(9,"Sections",&list)]);
        let untouched=input.clone();
        let output=decode_sections(&input).unwrap();
        assert_eq!(output.layout,Layout::NumericAnvil112);
        assert_eq!(output.block(4,2,3).unwrap().name,"numeric:301");
        assert_eq!(output.block(4,2,3).unwrap().properties[0].1,"7");
        assert_eq!(input,untouched);
    }
    #[test]
    fn mcregion_128_high_arrays_reorder_without_writing(){
        let (mut blocks,mut data)=(vec![0;32768],vec![0;16384]);
        let (x,z,y)=(3,7,100);
        let n=(x*16+z)*128+y;
        blocks[n]=77;data[n/2]|=4 << ((n%2)*4);
        let result=decode_sections(&old_chunk(vec![ba("Blocks",&blocks),ba("Data",&data)])).unwrap();
        assert_eq!(result.layout,Layout::McRegion128);
        assert_eq!(result.sections.len(),8);
        assert_eq!(result.block(x as u8,y as i32,z as u8).unwrap().name,"numeric:77");
        assert_eq!(result.block(x as u8,y as i32,z as u8).unwrap().properties[0].1,"4");
    }

    #[test]
    fn original_numeric_anvil_can_insert_missing_high_id_add_without_rewriting_opaque_nbt(){
        let blocks=vec![1u8;4096];
        let data=vec![0u8;2048];
        let original=old_chunk(vec![
            tag(3,"xPos",&0i32.to_be_bytes()),
            tag(3,"zPos",&0i32.to_be_bytes()),
            tag(9,"Sections",&{
                let section=compound(vec![tag(1,"Y",&[0]),
                    ba("Blocks",&blocks),ba("Data",&data),
                    tag(8,"Opaque",&[0,4,b's',b'a',b'f',b'e'])]);
                let mut list=vec![10];list.extend(1i32.to_be_bytes());list.extend(section);list
            })
        ]);
        let original_state=State{name:"numeric:1".into(),properties:vec![("data".into(),"0".into())]};
        let desired=State{name:"numeric:301".into(),properties:vec![("data".into(),"7".into())]};
        let result=super::super::sections::patch_original_numeric_arrays(&original,&[
            super::super::sections::InPaletteEdit{
                local_x:4,y:2,local_z:3,original:original_state,replacement:desired.clone(),
            }
        ]).unwrap();
        let decoded=decode_sections(&result).unwrap();
        assert_eq!(decoded.block(4,2,3),Some(&desired));
        assert_eq!(decoded.block(5,2,3).unwrap().name,"numeric:1");
        assert!(result.windows(b"safe".len()).any(|v|v==b"safe"));
        assert!(original.len()<result.len()); // only Add TAG was inserted
    }
    #[test]
    fn original_mcregion_byte_patch_preserves_neighbors_and_rejects_12bit_ids(){
        let original=old_chunk(vec![
            ba("Blocks",&vec![1u8;32768]),ba("Data",&vec![0;16384]),
            tag(8,"opaque",&[0,4,b's',b'a',b'f',b'e'])
        ]);
        let before=State{name:"numeric:1".into(),properties:vec![("data".into(),"0".into())]};
        let target=State{name:"numeric:254".into(),properties:vec![("data".into(),"15".into())]};
        let edit=super::super::sections::InPaletteEdit{
            local_x:3,y:100,local_z:7,original:before.clone(),replacement:target.clone(),
        };
        let result=super::super::sections::patch_original_numeric_arrays(&original,&[edit]).unwrap();
        let decoded=decode_sections(&result).unwrap();
        assert_eq!(decoded.block(3,100,7),Some(&target));
        assert_eq!(decoded.block(3,99,7),Some(&before));
        assert_eq!(original.len(),result.len());
        assert!(result.windows(b"safe".len()).any(|v|v==b"safe"));
        let unsafe_edit=super::super::sections::InPaletteEdit{
            local_x:3,y:100,local_z:7,original:before,
            replacement:State{name:"numeric:301".into(),
                properties:vec![("data".into(),"7".into())]},
        };
        assert!(super::super::sections::patch_original_numeric_arrays(&original,&[unsafe_edit]).is_err());
    }
    #[test]
    fn missing_or_truncated_original_arrays_are_refused(){
        assert!(decode_sections(&old_chunk(vec![ba("Blocks",&[0;100]),ba("Data",&[0;16384])])).is_err());
    }
}
