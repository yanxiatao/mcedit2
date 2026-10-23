//! Pure-Rust original-world block edit TRANSACTIONS (in-memory only).
//!
//! A prepared batch reads ORIGINAL Java chunk data, then stages effective
//! states into an overlay. No function here writes level.dat, .mca, .mcr,
//! original Alpha .dat, external entity sidecars, or unknown modded NBT.
//! The future persistent writer must separately prove byte preservation,
//! atomic backups, schema safety and all-version coverage before exposure.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use crate::{patch,region_plan,clipboard,schematic};
use crate::clipboard::{BlockClipboard,HorizontalTransform};
use crate::region_transaction::{self,RegionWriteReceipt};
use crate::region_plan::NativeRegionCandidate;

use crate::block_lookup::{parse_xyz, read_world_block, read_world_chunk, Lookup};
use crate::sections::{decode_sections,patch_named_growing_palette,patch_original_numeric_arrays,Chunk,InPaletteEdit,Layout,State};
use crate::block_lookup::read_world_chunk_bytes;
use sha2::{Digest,Sha256};
use crate::{WorldError, WorldSummary};

const MAX_CELLS: usize = 4096;
const MAX_HISTORY: usize = 100;
const MAX_COORD: i32 = 30_000_000;
const MIN_Y: i32 = -2048;
const MAX_Y: i32 = 4095;

type Position = (i32, i32, i32);
type ChunkPos = (i32, i32);

fn refuse(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub minimum: [i32; 3],
    pub maximum: [i32; 3],
}
impl Bounds {
    pub fn new(first: Position, second: Position) -> Result<Self, WorldError> {
        let a = [first.0, first.1, first.2];
        let b = [second.0, second.1, second.2];
        let mut minimum = [0; 3];
        let mut maximum = [0; 3];
        for i in 0..3 {
            minimum[i] = a[i].min(b[i]);
            maximum[i] = a[i].max(b[i]);
        }
        for i in [0, 2] {
            if minimum[i] <= -MAX_COORD || maximum[i] >= MAX_COORD {
                return Err(refuse("Selection exceeds native X/Z coordinate limits"));
            }
        }
        if minimum[1] < MIN_Y || maximum[1] > MAX_Y {
            return Err(refuse("Selection exceeds supported Y limits"));
        }
        let box_ = Self { minimum, maximum };
        if box_.volume()? > MAX_CELLS {
            return Err(refuse(format!(
                "Edit selection is over {MAX_CELLS} blocks; use smaller batches"
            )));
        }
        Ok(box_)
    }
    pub fn volume(&self) -> Result<usize, WorldError> {
        let mut count: u64 = 1;
        for i in 0..3 {
            let span = (self.maximum[i] as i64 - self.minimum[i] as i64 + 1) as u64;
            count = count.checked_mul(span).ok_or_else(|| refuse("Selection volume overflow"))?;
        }
        usize::try_from(count).map_err(|_| refuse("Selection volume overflows address size"))
    }
}
/// Two endpoint triples separated by semicolon:
/// "-4, -63, 0; 4, -63, 3", allowing reversed selections.
pub fn parse_bounds(input: &str) -> Result<Bounds, WorldError> {
    let (first, second) = input.split_once(';')
        .ok_or_else(|| refuse("Enter X1,Y1,Z1; X2,Y2,Z2"))?;
    if second.contains(';') { return Err(refuse("Selection requires exactly two XYZ triples")); }
    Bounds::new(parse_xyz(first)?, parse_xyz(second)?)
}
fn registry_part(part: &str, is_path: bool) -> bool {
    !part.is_empty() && part.len() <= 256 && part.bytes().all(|b|
        b.is_ascii_lowercase() || b.is_ascii_digit()
            || b == b'_' || b == b'-' || b == b'.'
            || (is_path && b == b'/')
    )
}
fn property_part(part: &str) -> bool {
    !part.is_empty() && part.len() <= 256 && part.bytes().all(|b|
        b.is_ascii_lowercase() || b.is_ascii_digit()
            || matches!(b, b'_' | b'-' | b'.' | b'/' | b':')
    )
}
/// Explicit original state syntax, e.g. minecraft:oak_log[axis=y] or
/// numeric:301[data=7]. No guessed mapping of legacy modded numeric IDs.
pub fn parse_state(input: &str) -> Result<State, WorldError> {
    let input = input.trim();
    let (identity, props_text) = if let Some((name, suffix)) = input.split_once('[') {
        (name, Some(suffix.strip_suffix(']').ok_or_else(|| refuse(
            "Block state property list must end with ]"
        ))?))
    } else { (input, None) };
    let (namespace, path) = identity.split_once(':')
        .ok_or_else(|| refuse("Block state needs explicit namespace:path"))?;
    if !registry_part(namespace, false) || !registry_part(path, true)
        || path.contains(':') || identity.len() > 512
    {
        return Err(refuse("Invalid or ambiguous original block identity"));
    }
    let mut properties: Vec<(String, String)> = Vec::new();
    if let Some(values) = props_text {
        if values.is_empty() { return Err(refuse("Empty property list is ambiguous")); }
        for component in values.split(',') {
            let (key, value) = component.split_once('=')
                .ok_or_else(|| refuse("Expected a=value in block state properties"))?;
            if !property_part(key) || !property_part(value) || value.contains('=') {
                return Err(refuse("Invalid block state property name or value"));
            }
            if properties.iter().any(|(existing, _)| existing == key) {
                return Err(refuse("Duplicate block state property name"));
            }
            properties.push((key.to_owned(), value.to_owned()));
            if properties.len() > 256 { return Err(refuse("Too many block properties")); }
        }
    }
    properties.sort_unstable();
    if namespace == "numeric" {
        let number = path.parse::<u16>().map_err(|_| refuse(
            "Original numeric block ID must be 0..4095"
        ))?;
        if number > 4095 || properties.len() != 1 || properties[0].0 != "data"
            || properties[0].1.parse::<u8>().is_err()
            || properties[0].1.parse::<u8>().unwrap() > 15
        {
            return Err(refuse("Numeric block identity requires numeric:ID[data=0..15]"));
        }
    }
    Ok(State { name: identity.to_owned(), properties })
}
pub fn render_state(state: &State) -> String {
    if state.properties.is_empty() { return state.name.clone(); }
    let members: Vec<_> = state.properties.iter().map(|(key, value)| format!("{key}={value}")).collect();
    format!("{}[{}]", state.name, members.join(","))
}

#[derive(Debug, Clone)]
struct SourceCheck {
    position: Position,
    original: State,
    effective: State,
}
#[derive(Debug, Clone)]
struct Change {
    position: Position,
    original: State,
    before: State,
    after: State,
}
#[derive(Debug, Clone)]
pub struct Preview {
    root: PathBuf,
    dimension: String,
    data_version: Option<i32>,
    changes: Vec<Change>,
    source_checks: Vec<SourceCheck>,
    pub is_copy: bool,
    pub is_move: bool,
    pub is_fill: bool,
    pub examined: usize,
    pub matched: usize,
    pub skipped: usize,
    pub source_chunks: usize,
}
impl Preview {
    pub fn changed(&self) -> usize { self.changes.len() }
    pub fn summary(&self) -> String {
        format!(
            "{}: {} original cells checked; {} matching filter; \
             {} staged block changes; {} unaffected; {} original chunks. \
             This is an IN-MEMORY transaction; the world is not saved.",
            if self.is_move { "3D block-only move" }
                else if self.is_copy { "3D block-only clone" }
                else if self.is_fill { "Connected 3D fill" } else { "3D selection" },
            self.examined, self.matched, self.changed(), self.skipped, self.source_chunks
        )
    }
}

/// Reuses a strictly decoded original chunk across every block in a batch.
/// Never creates absent chunks or Section data.
fn original_chunk<'a>(
    chunks: &'a mut BTreeMap<ChunkPos, Option<Chunk>>,
    world: &WorldSummary, dimension: &str, position: Position,
) -> Result<&'a Chunk, WorldError> {
    let key = (position.0.div_euclid(16), position.2.div_euclid(16));
    if !chunks.contains_key(&key) {
        chunks.insert(key, read_world_chunk(world, dimension, key.0, key.1)?);
    }
    chunks.get(&key).and_then(Option::as_ref).ok_or_else(|| refuse(format!(
        "Original Java chunk {},{} is absent; no block clone or generation allowed", key.0, key.1
    )))
}

/// Small original-world X/Z plane, preserving exact block state identities.
#[derive(Debug, Clone)]
pub struct Slice {
    pub bounds: Bounds,
    pub source_chunks: usize,
    pub present: usize,
    pub missing_chunks: usize,
    pub missing_sections: usize,
    pub block_entities: usize,
    pub rows: Vec<Vec<SliceCell>>,
}
#[derive(Debug, Clone)]
pub enum SliceCell {
    MissingChunk,
    MissingSection,
    Present { state: State, original_block_entity: bool },
}
impl Slice {
    /// Stable per-slice symbols; @ always marks original Block Entities.
    pub fn text_map(&self) -> String {
        const SYMBOLS: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        let mut symbols: BTreeMap<String, char> = BTreeMap::new();
        let mut out = format!(
            "X={}..{}, Y={}, Z={}..{} | chunks={}, known={}, missing chunks={}, missing Sections={}, BE={}\n",
            self.bounds.minimum[0], self.bounds.maximum[0], self.bounds.minimum[1],
            self.bounds.minimum[2], self.bounds.maximum[2], self.source_chunks,
            self.present, self.missing_chunks, self.missing_sections, self.block_entities,
        );
        for (index, row) in self.rows.iter().enumerate() {
            out.push_str(&format!("Z={:>8} |", self.bounds.minimum[2] + index as i32));
            for cell in row {
                let symbol = match cell {
                    SliceCell::MissingChunk => ' ',
                    SliceCell::MissingSection => '?',
                    SliceCell::Present { original_block_entity: true, .. } => '@',
                    SliceCell::Present { state, .. } => {
                        let name = render_state(state);
                        if let Some(letter) = symbols.get(&name) { *letter }
                        else {
                            let letter = SYMBOLS.chars().nth(symbols.len()).unwrap_or('*');
                            symbols.insert(name, letter);
                            letter
                        }
                    },
                };
                out.push(symbol);
            }
            out.push_str("|\n");
        }
        out.push_str("Legend: space=no original chunk; ?=no Section; @=Block Entity; *=other states\n");
        for (state, symbol) in &symbols { out.push_str(&format!("{symbol} = {state}\n")); }
        out
    }
}


/// Verified, UNSAVED original-byte chunk patch candidate. A future writer
/// MUST recheck the original NBT digest, region sector layout, backups,
/// heightmaps, lighting and transaction integrity before any disk write.

/// Orthogonal vertical inspection plane, Y always runs from highest row
/// to lowest, with exactly one horizontal axis varying. Missing original
/// source cells are NOT generated as air.
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum VerticalAxis { AlongX, AlongZ }
#[derive(Debug,Clone)]
pub struct VerticalSlice {
    pub bounds:Bounds,
    pub axis:VerticalAxis,
    pub source_chunks:usize,
    pub block_entities:usize,
    pub missing_chunks:usize,
    pub missing_sections:usize,
    pub rows:Vec<Vec<SliceCell>>,
}
impl VerticalSlice {
    pub fn coordinate(&self,row:usize,column:usize)->Option<(i32,i32,i32)>{
        let height=(self.bounds.maximum[1]-self.bounds.minimum[1]+1) as usize;
        let width=match self.axis{
            VerticalAxis::AlongX=>(self.bounds.maximum[0]-self.bounds.minimum[0]+1) as usize,
            VerticalAxis::AlongZ=>(self.bounds.maximum[2]-self.bounds.minimum[2]+1) as usize,
        };
        if row>=height||column>=width{return None;}
        let y=self.bounds.maximum[1]-row as i32;
        match self.axis{
            VerticalAxis::AlongX=>Some((
                self.bounds.minimum[0]+column as i32,y,self.bounds.minimum[2],
            )),
            VerticalAxis::AlongZ=>Some((
                self.bounds.minimum[0],y,self.bounds.minimum[2]+column as i32,
            )),
        }
    }
    pub fn text_map(&self)->String{
        let mut out=format!(
            "Vertical {:?} original plane; BE={}, missing chunks={}, missing Sections={}\n",
            self.axis,self.block_entities,self.missing_chunks,self.missing_sections
        );
        for (line,row) in self.rows.iter().enumerate(){
            out.push_str(&format!("Y={:>5} |",self.bounds.maximum[1]-line as i32));
            for cell in row{
                let glyph=match cell{
                    SliceCell::MissingChunk=>' ',
                    SliceCell::MissingSection=>'?',
                    SliceCell::Present {original_block_entity:true,..}=>'@',
                    SliceCell::Present {state,..}=>state.name.chars().last().unwrap_or('*'),
                };
                out.push(glyph);
            }
            out.push_str("|\n");
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct NativeChunkCandidate {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub edited_blocks: usize,
    pub original_nbt_sha256: String,
    pub patched_nbt: Vec<u8>,
}

/// Strict 3D original block-state transaction overlay. Nothing is saved.
#[derive(Debug, Clone)]
pub struct Session {
    world: WorldSummary,
    dimension: String,
    overlay: BTreeMap<Position, State>,
    undo: Vec<Vec<Change>>,
    redo: Vec<Vec<Change>>,
}
impl Session {
    pub fn new(world: WorldSummary, dimension: impl Into<String>) -> Result<Self, WorldError> {
        let dimension = dimension.into();
        if !world.dimensions.iter().any(|d| d.name == dimension) {
            return Err(refuse("Native edit session dimension is not part of selected world"));
        }
        Ok(Self { world, dimension, overlay: BTreeMap::new(), undo: Vec::new(), redo: Vec::new() })
    }
    pub fn original_world(&self) -> &WorldSummary { &self.world }
    pub fn dimension(&self) -> &str { &self.dimension }
    pub fn pending_cells(&self) -> usize { self.overlay.len() }
    pub fn undo_depth(&self) -> usize { self.undo.len() }
    pub fn redo_depth(&self) -> usize { self.redo.len() }
    pub fn has_pending_changes(&self) -> bool { !self.overlay.is_empty() }
    /// Reads actual original world NBT only when there is no staged overlay.
    pub fn visible_block(&self, position: Position) -> Result<Lookup, WorldError> {
        if let Some(state) = self.overlay.get(&position) {
            return Ok(Lookup::Present(crate::block_lookup::BlockDetails {
                state: state.clone(), chunk_x: position.0.div_euclid(16),
                chunk_z: position.2.div_euclid(16),
                original_data_version: self.world.data_version,
                // Report actual original world layout when queried, instead
                // of lying about snapshot / older numeric storage format.
                layout: match read_world_block(
                    &self.world, &self.dimension, position.0, position.1, position.2
                )? {
                    Lookup::Present(existing) => existing.layout,
                    _ => return Err(refuse("Pending cell lost original chunk/Section")),
                },
                block_x: position.0, block_y: position.1, block_z: position.2,
            }));
        }
        read_world_block(&self.world, &self.dimension, position.0, position.1, position.2)
    }
    /// Inspect an ORIGINAL modded Block Entity, reporting exact field kinds
    /// and a source NBT digest without rewriting any of its contents.
    pub fn inspect_original_block_entity(
        &self,position:Position,
    )->Result<Option<crate::sections::OriginalBlockEntity>,WorldError>{
        let (cx,cz)=(position.0.div_euclid(16),position.2.div_euclid(16));
        let Some(raw)=read_world_chunk_bytes(&self.world,&self.dimension,cx,cz)? else {
            return Ok(None);
        };
        let decoded=decode_sections(&raw)
            .map_err(|e|refuse(format!("Original BE chunk invalid: {e}")))?;
        if decoded.x.is_some_and(|x|x!=cx)||decoded.z.is_some_and(|z|z!=cz){
            return Err(refuse("Original Block Entity chunk position mismatch"));
        }
        crate::sections::inspect_original_block_entity(&raw,position)
            .map_err(|e|refuse(format!("Original Block Entity NBT invalid: {e}")))
    }
    /// Group by original chunk; inspect each native NBT at most once.
    /// Entire preview is read-only. Missing original chunks/Sections are
    /// REFUSED: no auto-created air, chunk or legacy ID registry is assumed.
    pub fn prepare(&self, bounds: Bounds, replacement: &State,
                   only_if: Option<&State>) -> Result<Preview, WorldError> {
        if bounds.volume()? > MAX_CELLS {
            return Err(refuse("Original block edit batch exceeds limit"));
        }
        let mut chunks: BTreeMap<ChunkPos, Option<Chunk>> = BTreeMap::new();
        let mut changes = Vec::new();
        let mut matched = 0usize;
        let mut examined = 0usize;
        for y in bounds.minimum[1]..=bounds.maximum[1] {
            for z in bounds.minimum[2]..=bounds.maximum[2] {
                for x in bounds.minimum[0]..=bounds.maximum[0] {
                    let position = (x, y, z);
                    let key = (x.div_euclid(16), z.div_euclid(16));
                    if !chunks.contains_key(&key) {
                        chunks.insert(key, read_world_chunk(
                            &self.world, &self.dimension, key.0, key.1
                        )?);
                    }
                    let chunk = chunks.get(&key).and_then(Option::as_ref)
                        .ok_or_else(|| refuse(format!(
                            "No ORIGINAL Java chunk at {},{}; edit refused", key.0, key.1
                        )))?;
                    let original = chunk.block(
                        x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8
                    ).ok_or_else(|| refuse(format!(
                        "Original chunk {},{} has no Section at Y={y}; edit refused",
                        key.0, key.1
                    )))?;
                    // Never blindly place a named state into an old numeric
                    // world or reinterpret unknown numeric IDs as registry IDs.
                    let original_numeric = matches!(chunk.layout,
                        Layout::McRegion128 | Layout::NumericAnvil112);
                    if original_numeric != replacement.name.starts_with("numeric:") {
                        return Err(refuse(
                            "Target state layout mismatches original numeric/named Java world"
                        ));
                    }
                    if let Some(filter) = only_if {
                        if filter.name.starts_with("numeric:") != original_numeric {
                            return Err(refuse("Filter state layout mismatches original world"));
                        }
                    }
                    examined += 1;
                    let before = self.overlay.get(&position).unwrap_or(original);
                    if only_if.is_some_and(|filter| filter != before) {
                        continue;
                    }
                    matched += 1;
                    if before != replacement {
                        if chunk.has_block_entity(x, y, z) {
                            return Err(refuse(format!(
                                "Original Block Entity at {x},{y},{z}; block replacement refused without BE-aware transaction"
                            )));
                        }
                        changes.push(Change {
                            position, original: original.clone(),
                            before: before.clone(), after: replacement.clone(),
                        });
                    }
                }
            }
        }
        Ok(Preview {
            root: self.world.root.clone(), dimension: self.dimension.clone(),
            data_version: self.world.data_version, changes,
            source_checks: Vec::new(), is_copy: false, is_move:false, is_fill: false, examined, matched,
            skipped: examined - matched, source_chunks: chunks.len(),
        })
    }
    /// An explicitly bounded read-only horizontal view. Missing source
    /// chunks/Sections are distinguishable from air; no files are modified.
    pub fn inspect_slice(&self, bounds: Bounds) -> Result<Slice, WorldError> {
        if bounds.minimum[1] != bounds.maximum[1] {
            return Err(refuse("X/Z slice requires one fixed Y value"));
        }
        let width = (bounds.maximum[0] - bounds.minimum[0] + 1) as usize;
        let height = (bounds.maximum[2] - bounds.minimum[2] + 1) as usize;
        if width > 64 || height > 64 || bounds.volume()? > MAX_CELLS {
            return Err(refuse("X/Z slice exceeds 64x64 / 4096 cells"));
        }
        let mut chunks: BTreeMap<ChunkPos, Option<Chunk>> = BTreeMap::new();
        let mut rows = Vec::with_capacity(height);
        let (mut present, mut missing_chunks, mut missing_sections, mut block_entities)=(0,0,0,0);
        let y = bounds.minimum[1];
        for z in bounds.minimum[2]..=bounds.maximum[2] {
            let mut row = Vec::with_capacity(width);
            for x in bounds.minimum[0]..=bounds.maximum[0] {
                let key = (x.div_euclid(16), z.div_euclid(16));
                if !chunks.contains_key(&key) {
                    chunks.insert(key, read_world_chunk(&self.world,&self.dimension,key.0,key.1)?);
                }
                let cell = match chunks.get(&key).and_then(Option::as_ref) {
                    None => { missing_chunks += 1; SliceCell::MissingChunk },
                    Some(chunk) => match chunk.block(x.rem_euclid(16) as u8,y,z.rem_euclid(16) as u8) {
                        None => { missing_sections += 1; SliceCell::MissingSection },
                        Some(original) => {
                            present += 1;
                            let is_be = chunk.has_block_entity(x,y,z);
                            if is_be { block_entities += 1; }
                            SliceCell::Present {
                                state: self.overlay.get(&(x,y,z)).unwrap_or(original).clone(),
                                original_block_entity: is_be,
                            }
                        },
                    },
                };
                row.push(cell);
            }
            rows.push(row);
        }
        Ok(Slice {bounds,source_chunks:chunks.len(),present,missing_chunks,
            missing_sections,block_entities,rows})
    }
    /// Inspect an X/Y or Z/Y vertical plane (up to 64x64). Only existing
    /// original chunks/Sections are read. No edits and no world writes.
    pub fn inspect_vertical_slice(&self,bounds:Bounds)->Result<VerticalSlice,WorldError>{
        let x_static=bounds.minimum[0]==bounds.maximum[0];
        let z_static=bounds.minimum[2]==bounds.maximum[2];
        if !x_static && !z_static {
            return Err(refuse("Vertical slice must keep X or Z fixed"));
        }
        let axis=if z_static{VerticalAxis::AlongX}else{VerticalAxis::AlongZ};
        let width=match axis{
            VerticalAxis::AlongX=>(bounds.maximum[0]-bounds.minimum[0]+1) as usize,
            VerticalAxis::AlongZ=>(bounds.maximum[2]-bounds.minimum[2]+1) as usize,
        };
        let height=(bounds.maximum[1]-bounds.minimum[1]+1) as usize;
        if width>64||height>64||bounds.volume()?>MAX_CELLS{
            return Err(refuse("Vertical original plane exceeds 64x64 cells"));
        }
        let mut cache:BTreeMap<ChunkPos,Option<Chunk>>=BTreeMap::new();
        let (mut be,mut missing_chunks,mut missing_sections)=(0,0,0);
        let mut rows=Vec::with_capacity(height);
        for y in (bounds.minimum[1]..=bounds.maximum[1]).rev(){
            let mut line=Vec::with_capacity(width);
            for offset in 0..width{
                let (x,z)=match axis{
                    VerticalAxis::AlongX=>(bounds.minimum[0]+offset as i32,bounds.minimum[2]),
                    VerticalAxis::AlongZ=>(bounds.minimum[0],bounds.minimum[2]+offset as i32),
                };
                let key=(x.div_euclid(16),z.div_euclid(16));
                if !cache.contains_key(&key){
                    cache.insert(key,read_world_chunk(
                        &self.world,&self.dimension,key.0,key.1
                    )?);
                }
                let cell=match cache.get(&key).and_then(Option::as_ref){
                    None=>{missing_chunks+=1;SliceCell::MissingChunk},
                    Some(chunk)=>match chunk.block(
                        x.rem_euclid(16) as u8,y,z.rem_euclid(16) as u8
                    ){
                        None=>{missing_sections+=1;SliceCell::MissingSection},
                        Some(original)=>{
                            let is_be=chunk.has_block_entity(x,y,z);
                            if is_be{be+=1;}
                            SliceCell::Present{
                                state:self.overlay.get(&(x,y,z)).unwrap_or(original).clone(),
                                original_block_entity:is_be,
                            }
                        }
                    },
                };
                line.push(cell);
            }
            rows.push(line);
        }
        Ok(VerticalSlice {
            bounds,axis,source_chunks:cache.len(),
            block_entities:be,missing_chunks,missing_sections,rows,
        })
    }
    /// Bounded 6-neighbor flood fill over the *effective* unsaved block
    /// states. All cells are read before preview; no BEs are mutated.
    /// Disconnected equal states are intentionally left untouched.
    pub fn prepare_connected_fill(
        &self, bounds: Bounds, seed: Position, replacement: &State,
    ) -> Result<Preview, WorldError> {
        let volume = bounds.volume()?;
        let within = |pos: Position| -> bool {
            pos.0 >= bounds.minimum[0] && pos.0 <= bounds.maximum[0]
                && pos.1 >= bounds.minimum[1] && pos.1 <= bounds.maximum[1]
                && pos.2 >= bounds.minimum[2] && pos.2 <= bounds.maximum[2]
        };
        if !within(seed) { return Err(refuse("Flood fill seed must lie within selected 3D bounds")); }
        let mut chunks: BTreeMap<ChunkPos, Option<Chunk>> = BTreeMap::new();
        let mut snapshot: BTreeMap<Position, (State, State, bool, bool)> = BTreeMap::new();
        for y in bounds.minimum[1]..=bounds.maximum[1] {
            for z in bounds.minimum[2]..=bounds.maximum[2] {
                for x in bounds.minimum[0]..=bounds.maximum[0] {
                    let position = (x,y,z);
                    let chunk = original_chunk(&mut chunks,&self.world,&self.dimension,position)?;
                    let original = chunk.block(x.rem_euclid(16) as u8,y,z.rem_euclid(16) as u8)
                        .ok_or_else(|| refuse("Flood fill selection contains missing original Section"))?;
                    let effective = self.overlay.get(&position).unwrap_or(original).clone();
                    let is_numeric = matches!(chunk.layout,Layout::McRegion128 | Layout::NumericAnvil112);
                    snapshot.insert(position,(original.clone(),effective,
                        chunk.has_block_entity(x,y,z),is_numeric));
                }
            }
        }
        let seed_state = snapshot.get(&seed)
            .ok_or_else(|| refuse("Flood fill seed not present in original selection"))?.1.clone();
        let mut visited: BTreeSet<Position> = BTreeSet::new();
        let mut pending = VecDeque::from([seed]);
        let mut changes = Vec::new();
        while let Some(pos) = pending.pop_front() {
            if !within(pos) || !visited.insert(pos) { continue; }
            let (original,effective,is_be,is_numeric) = snapshot.get(&pos)
                .ok_or_else(|| refuse("Flood fill selection incomplete"))?;
            if effective != &seed_state { continue; }
            if *is_numeric != replacement.name.starts_with("numeric:") {
                return Err(refuse("Flood fill target mismatches original numeric/named layout"));
            }
            if effective != replacement {
                if *is_be { return Err(refuse(format!(
                    "Connected flood fill would change original Block Entity at {},{},{}",
                    pos.0,pos.1,pos.2
                ))); }
                changes.push(Change {
                    position:pos,original:original.clone(),
                    before:effective.clone(),after:replacement.clone(),
                });
            }
            for next in [
                (pos.0-1,pos.1,pos.2),(pos.0+1,pos.1,pos.2),
                (pos.0,pos.1-1,pos.2),(pos.0,pos.1+1,pos.2),
                (pos.0,pos.1,pos.2-1),(pos.0,pos.1,pos.2+1),
            ] {
                if within(next) && !visited.contains(&next) {
                    pending.push_back(next);
                }
            }
        }
        let matched = changes.len();
        Ok(Preview {
            root:self.world.root.clone(),dimension:self.dimension.clone(),
            data_version:self.world.data_version,changes,
            source_checks:Vec::new(),is_copy:false,is_move:false,is_fill:true,
            examined:volume,matched,skipped:volume-matched,
            source_chunks:chunks.len(),
        })
    }
    /// Freeze the EFFECTIVE source states (including previous in-memory edits)
    /// and construct a block-only copy. All original BE NBT remains untouched.
    /// Source and destination are fully read and preflighted before returning
    /// a preview; overlap is safe because the source is snapshotted first.
    pub fn prepare_copy(
        &self, source: Bounds, destination_min: Position,
    ) -> Result<Preview, WorldError> {
        let volume = source.volume()?;
        if volume > MAX_CELLS {
            return Err(refuse("Clone source exceeds safe block batch size"));
        }
        let offsets = [
            source.maximum[0] - source.minimum[0],
            source.maximum[1] - source.minimum[1],
            source.maximum[2] - source.minimum[2],
        ];
        let maximum = (
            destination_min.0.checked_add(offsets[0])
                .ok_or_else(|| refuse("Clone X destination overflow"))?,
            destination_min.1.checked_add(offsets[1])
                .ok_or_else(|| refuse("Clone Y destination overflow"))?,
            destination_min.2.checked_add(offsets[2])
                .ok_or_else(|| refuse("Clone Z destination overflow"))?,
        );
        let destination = Bounds::new(destination_min, maximum)?;
        if destination.minimum != [destination_min.0, destination_min.1, destination_min.2] {
            return Err(refuse("Clone destination bounds reversed"));
        }
        let mut chunks: BTreeMap<ChunkPos, Option<Chunk>> = BTreeMap::new();
        let mut snapshots: Vec<(Position, State)> = Vec::with_capacity(volume);
        let mut source_checks = Vec::with_capacity(volume);
        for y in source.minimum[1]..=source.maximum[1] {
            for z in source.minimum[2]..=source.maximum[2] {
                for x in source.minimum[0]..=source.maximum[0] {
                    let position = (x, y, z);
                    let chunk = original_chunk(&mut chunks, &self.world, &self.dimension, position)?;
                    let original = chunk.block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                        .ok_or_else(|| refuse("Clone source Section is absent"))?;
                    if chunk.has_block_entity(x, y, z) {
                        return Err(refuse(format!(
                            "Clone source contains original Block Entity at {x},{y},{z}"
                        )));
                    }
                    let effective = self.overlay.get(&position).unwrap_or(original).clone();
                    source_checks.push(SourceCheck {
                        position, original: original.clone(), effective: effective.clone(),
                    });
                    snapshots.push((position, effective));
                }
            }
        }
        let mut changes = Vec::new();
        for (position, replacement) in snapshots {
            let x = destination_min.0 + (position.0 - source.minimum[0]);
            let y = destination_min.1 + (position.1 - source.minimum[1]);
            let z = destination_min.2 + (position.2 - source.minimum[2]);
            let target = (x, y, z);
            let chunk = original_chunk(&mut chunks, &self.world, &self.dimension, target)?;
            let original = chunk.block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                .ok_or_else(|| refuse("Clone destination Section is absent"))?;
            let source_numeric = replacement.name.starts_with("numeric:");
            let target_numeric = matches!(chunk.layout, Layout::McRegion128 | Layout::NumericAnvil112);
            if source_numeric != target_numeric {
                return Err(refuse("Clone source and destination use incompatible block ID layouts"));
            }
            let before = self.overlay.get(&target).unwrap_or(original);
            if before != &replacement {
                if chunk.has_block_entity(x, y, z) {
                    return Err(refuse(format!(
                        "Clone destination contains original Block Entity at {x},{y},{z}"
                    )));
                }
                changes.push(Change {
                    position: target, original: original.clone(),
                    before: before.clone(), after: replacement,
                });
            }
        }
        Ok(Preview {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            data_version: self.world.data_version,
            changes, source_checks, is_copy: true, is_move:false, is_fill: false,
            examined: volume, matched: volume,
            skipped: 0, source_chunks: chunks.len(),
        })
    }
    /// Atomic block-only MOVE, including overlapping source and destination.
    /// Read the whole original and EFFECTIVE source first, then construct
    /// a frozen final map: source -> actual air, destination -> source state.
    /// Every original Block Entity in either area refuses the entire move.
    pub fn prepare_move(
        &self,source:Bounds,destination_min:Position,
    )->Result<Preview,WorldError>{
        let volume=source.volume()?;
        let offsets=[
            source.maximum[0]-source.minimum[0],
            source.maximum[1]-source.minimum[1],
            source.maximum[2]-source.minimum[2],
        ];
        let max=(
            destination_min.0.checked_add(offsets[0])
                .ok_or_else(||refuse("Move X coordinate overflow"))?,
            destination_min.1.checked_add(offsets[1])
                .ok_or_else(||refuse("Move Y coordinate overflow"))?,
            destination_min.2.checked_add(offsets[2])
                .ok_or_else(||refuse("Move Z coordinate overflow"))?,
        );
        let destination=Bounds::new(destination_min,max)?;
        if destination.minimum!=[destination_min.0,destination_min.1,destination_min.2]{
            return Err(refuse("Move destination bounds were reversed"));
        }
        let mut chunks:BTreeMap<ChunkPos,Option<Chunk>>=BTreeMap::new();
        let mut source_checks=Vec::with_capacity(volume);
        let mut snapshots=Vec::with_capacity(volume);
        let mut final_states:BTreeMap<Position,State>=BTreeMap::new();
        for y in source.minimum[1]..=source.maximum[1]{
            for z in source.minimum[2]..=source.maximum[2]{
                for x in source.minimum[0]..=source.maximum[0]{
                    let pos=(x,y,z);
                    let chunk=original_chunk(&mut chunks,&self.world,&self.dimension,pos)?;
                    if chunk.has_block_entity(x,y,z){
                        return Err(refuse("Cannot move original Block Entity without preserving full opaque NBT"));
                    }
                    let original=chunk.block(x.rem_euclid(16) as u8,
                        y,z.rem_euclid(16) as u8)
                        .ok_or_else(||refuse("Move source missing original Section"))?;
                    let effective=self.overlay.get(&pos).unwrap_or(original).clone();
                    let air=if matches!(chunk.layout,Layout::McRegion128|Layout::NumericAnvil112){
                        parse_state("numeric:0[data=0]")?
                    }else{
                        parse_state("minecraft:air")?
                    };
                    source_checks.push(SourceCheck{
                        position:pos,original:original.clone(),effective:effective.clone(),
                    });
                    snapshots.push((pos,effective));
                    final_states.insert(pos,air);
                }
            }
        }
        for (position,state) in snapshots{
            let target=(
                destination_min.0+(position.0-source.minimum[0]),
                destination_min.1+(position.1-source.minimum[1]),
                destination_min.2+(position.2-source.minimum[2]),
            );
            let target_chunk=original_chunk(&mut chunks,&self.world,&self.dimension,target)?;
            if target_chunk.has_block_entity(target.0,target.1,target.2){
                return Err(refuse("Cannot overwrite original Block Entity with moved blocks"));
            }
            let source_numeric=state.name.starts_with("numeric:");
            let dest_numeric=matches!(target_chunk.layout,Layout::McRegion128|Layout::NumericAnvil112);
            if source_numeric!=dest_numeric{
                return Err(refuse("Move crosses incompatible numeric/named original chunk layouts"));
            }
            if target_chunk.block(target.0.rem_euclid(16) as u8,target.1,
                target.2.rem_euclid(16) as u8).is_none(){
                return Err(refuse("Move destination original Section does not exist"));
            }
            final_states.insert(target,state);
        }
        if final_states.len()>MAX_CELLS {
            return Err(refuse("Combined Move source/destination exceeds 4096 changed candidate cells"));
        }
        let mut changes=Vec::new();
        for (position,after) in final_states{
            let chunk=original_chunk(&mut chunks,&self.world,&self.dimension,position)?;
            let original=chunk.block(position.0.rem_euclid(16) as u8,
                position.1,position.2.rem_euclid(16) as u8)
                .ok_or_else(||refuse("Move affected original Section disappeared"))?;
            let before=self.overlay.get(&position).unwrap_or(original);
            if before!=&after{
                changes.push(Change{
                    position,original:original.clone(),
                    before:before.clone(),after,
                });
            }
        }
        Ok(Preview {
            root:self.world.root.clone(),
            dimension:self.dimension.clone(),data_version:self.world.data_version,
            changes,source_checks,is_copy:false,is_move:true,is_fill:false,
            examined:volume,matched:volume,skipped:0,source_chunks:chunks.len(),
        })
    }
    fn set_effective(&mut self, change: &Change, state: &State) {
        if state == &change.original {
            self.overlay.remove(&change.position);
        } else {
            self.overlay.insert(change.position, state.clone());
        }
    }
    /// Re-check original disk NBT and current staged states for ALL targets
    /// before staging the first change; no partial mutations on refusal.
    pub fn commit(&mut self, prepared: Preview) -> Result<usize, WorldError> {
        if prepared.root != self.world.root
            || prepared.dimension != self.dimension
            || prepared.data_version != self.world.data_version {
            return Err(refuse("Prepared edit belongs to different original world/version/dimension"));
        }
        if prepared.changes.len() > MAX_CELLS {
            return Err(refuse("Prepared edit exceeds safe batch size"));
        }
        let mut chunks: BTreeMap<ChunkPos, Option<Chunk>> = BTreeMap::new();
        for check in &prepared.source_checks {
            let (x, y, z) = check.position;
            let chunk = original_chunk(&mut chunks, &self.world, &self.dimension, check.position)?;
            let original = chunk.block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                .ok_or_else(|| refuse("Original clone source Section disappeared"))?;
            if original != &check.original || chunk.has_block_entity(x, y, z) {
                return Err(refuse("Original clone source changed or acquired Block Entity after preview"));
            }
            if self.overlay.get(&check.position).unwrap_or(original) != &check.effective {
                return Err(refuse("Staged clone source changed after preview"));
            }
        }
        for change in &prepared.changes {
            let (x, y, z) = change.position;
            let key = (x.div_euclid(16), z.div_euclid(16));
            if !chunks.contains_key(&key) {
                chunks.insert(key, read_world_chunk(&self.world, &self.dimension, key.0, key.1)?);
            }
            let original = chunks.get(&key).and_then(Option::as_ref)
                .and_then(|v| v.block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8))
                .ok_or_else(|| refuse("Prepared edit original chunk/Section disappeared"))?;
            if original != &change.original {
                return Err(refuse("Original world NBT changed since edit preview"));
            }
            if chunks.get(&key).and_then(Option::as_ref)
                .is_some_and(|chunk| chunk.has_block_entity(x, y, z)) {
                return Err(refuse("Original Block Entity occupies planned edit; commit refused"));
            }
            let currently = self.overlay.get(&change.position).unwrap_or(original);
            if currently != &change.before {
                return Err(refuse("Staged block changed since edit preview"));
            }
        }
        let count = prepared.changes.len();
        if count == 0 { return Ok(0); }
        for change in &prepared.changes { self.set_effective(change, &change.after); }
        self.undo.push(prepared.changes);
        if self.undo.len() > MAX_HISTORY { self.undo.remove(0); }
        self.redo.clear();
        Ok(count)
    }
    /// Attempt a STRICT in-memory original-byte NBT save preview for
    /// already-existing named palette states only. No files are touched.
    pub fn prepare_native_palette_candidates(
        &self,
    ) -> Result<Vec<NativeChunkCandidate>, WorldError> {
        if self.overlay.is_empty() {
            return Err(refuse("No staged block edits for native NBT preview"));
        }
        let mut grouped: BTreeMap<ChunkPos,Vec<(Position,State)>>=BTreeMap::new();
        for (&pos,target) in &self.overlay {
            grouped.entry((pos.0.div_euclid(16),pos.2.div_euclid(16)))
                .or_default().push((pos,target.clone()));
        }
        let mut result=Vec::with_capacity(grouped.len());
        for ((cx,cz),positions) in grouped {
            let raw=read_world_chunk_bytes(&self.world,&self.dimension,cx,cz)?
                .ok_or_else(||refuse("Original chunk disappeared before NBT planning"))?;
            let chunk=decode_sections(&raw)
                .map_err(|e|refuse(format!("Original NBT refused: {e}")))?;
            if chunk.x.is_some_and(|v|v!=cx)||chunk.z.is_some_and(|v|v!=cz) {
                return Err(refuse("Original chunk coordinates mismatch before NBT planning"));
            }
            if chunk.block_entities.iter().any(|&(x,_,z)|
                x.div_euclid(16)!=cx || z.div_euclid(16)!=cz
            ) {return Err(refuse("Original Block Entity ownership is invalid"));}
            let mut edits=Vec::with_capacity(positions.len());
            for (pos,replacement) in positions {
                if chunk.has_block_entity(pos.0,pos.1,pos.2) {
                    return Err(refuse("Native NBT preview would replace Block Entity"));
                }
                let original=chunk.block(pos.0.rem_euclid(16) as u8,pos.1,
                    pos.2.rem_euclid(16) as u8)
                    .ok_or_else(||refuse("Original NBT preview Section missing"))?;
                edits.push(InPaletteEdit {
                    local_x:pos.0.rem_euclid(16) as u8,
                    y:pos.1,
                    local_z:pos.2.rem_euclid(16) as u8,
                    original:original.clone(),replacement,
                });
            }
            let modified=match chunk.layout {
                Layout::Named113 | Layout::Modern118 =>
                    patch_named_growing_palette(&raw,&edits),
                Layout::McRegion128 | Layout::NumericAnvil112 =>
                    patch_original_numeric_arrays(&raw,&edits),
            }.map_err(|e|refuse(format!(
                "Native original-byte preview cannot safely encode chunk {cx},{cz}: {e}"
            )))?;
            let original_nbt_sha256: String=Sha256::digest(&raw).iter()
                .map(|v|format!("{v:02x}")).collect();
            result.push(NativeChunkCandidate {
                chunk_x:cx,chunk_z:cz,edited_blocks:edits.len(),
                original_nbt_sha256,patched_nbt:modified,
            });
        }
        Ok(result)
    }
    /// Preview complete append-only MCA/MCR images for staged block edits.
    /// Never replaces original region data, never writes any file.
    pub fn prepare_native_region_candidates(
        &self, timestamp: u32,
    ) -> Result<Vec<NativeRegionCandidate>, WorldError> {
        let candidates=self.prepare_native_palette_candidates()?;
        region_plan::plan_world_regions(
            &self.world,&self.dimension,&candidates,timestamp,
        )
    }
    /// Experimental single-Region world write. Refuses any staged edit that
    /// touches a second Region; manual-only library API, not exposed in iced.
    /// A successful atomic replacement clears the overlay and returns the
    /// persistent original backup + pending marker for explicit finalization.
    pub fn apply_single_region_save(
        &mut self,timestamp:u32,
    )->Result<RegionWriteReceipt,WorldError>{
        let mut images=self.prepare_native_region_candidates(timestamp)?;
        if images.len()!=1 {
            return Err(refuse("Multi-Region edits are not an atomic transaction; export a journal instead"));
        }
        let candidate=images.pop().ok_or_else(||refuse("Missing single Region save candidate"))?;
        let receipt=region_transaction::apply_single_region(
            &self.world,&self.dimension,&candidate)?;
        self.discard();
        Ok(receipt)
    }
    /// Capture a block-only snapshot of a source selection. Original BE
    /// coordinates always refuse capture, rather than silently copying a
    /// machine block while losing its inventory or modded NBT.
    pub fn capture_clipboard(&self,bounds:Bounds)->Result<BlockClipboard,WorldError>{
        let volume=bounds.volume()?;
        let width=u16::try_from(bounds.maximum[0]-bounds.minimum[0]+1)
            .map_err(|_|refuse("Clipboard X size overflow"))?;
        let height=u16::try_from(bounds.maximum[1]-bounds.minimum[1]+1)
            .map_err(|_|refuse("Clipboard Y size overflow"))?;
        let depth=u16::try_from(bounds.maximum[2]-bounds.minimum[2]+1)
            .map_err(|_|refuse("Clipboard Z size overflow"))?;
        let mut chunks=BTreeMap::new();
        let mut blocks=Vec::with_capacity(volume);
        let mut numeric:Option<bool>=None;
        for y in bounds.minimum[1]..=bounds.maximum[1]{
            for z in bounds.minimum[2]..=bounds.maximum[2]{
                for x in bounds.minimum[0]..=bounds.maximum[0]{
                    let pos=(x,y,z);
                    let chunk=original_chunk(&mut chunks,&self.world,&self.dimension,pos)?;
                    if chunk.has_block_entity(x,y,z){
                        return Err(refuse("Clipboard selection contains original Block Entity; block-only copy refused"));
                    }
                    let id_numeric=matches!(chunk.layout,Layout::McRegion128|Layout::NumericAnvil112);
                    if numeric.is_some_and(|found|found!=id_numeric) {
                        return Err(refuse("Clipboard selection mixes numeric and named chunk schemas"));
                    }
                    numeric=Some(id_numeric);
                    let original=chunk.block(x.rem_euclid(16) as u8,y,z.rem_euclid(16) as u8)
                        .ok_or_else(||refuse("Clipboard source Section absent"))?;
                    blocks.push(self.overlay.get(&pos).unwrap_or(original).clone());
                }
            }
        }
        clipboard::new(&self.world,&self.dimension,numeric.unwrap_or(false),
            bounds.minimum,[width,height,depth],blocks)
    }
    /// Export a portable same-world snapshot without changing the source
    /// Minecraft world. Existing clipboard files are never overwritten.
    pub fn export_clipboard(&self,bounds:Bounds,path:&Path)->Result<usize,WorldError>{
        let copied=self.capture_clipboard(bounds)?;
        clipboard::save(&self.world,path,&copied)?;
        Ok(copied.blocks.len())
    }
    /// Create one fully-preflighted, reversible paste Preview.
    /// Pasting never carries Block Entity/entity data and never creates
    /// absent chunks/Sections. Source is a frozen copied block snapshot.
    pub fn prepare_paste_clipboard(
        &self,clip:&BlockClipboard,destination_min:Position,
    )->Result<Preview,WorldError>{
        clip.validate(&self.world,&self.dimension)?;
        let max=(
            destination_min.0.checked_add(clip.size[0] as i32-1)
                .ok_or_else(||refuse("Clipboard X paste overflow"))?,
            destination_min.1.checked_add(clip.size[1] as i32-1)
                .ok_or_else(||refuse("Clipboard Y paste overflow"))?,
            destination_min.2.checked_add(clip.size[2] as i32-1)
                .ok_or_else(||refuse("Clipboard Z paste overflow"))?,
        );
        let box_=Bounds::new(destination_min,max)?;
        if box_.minimum!=[destination_min.0,destination_min.1,destination_min.2] {
            return Err(refuse("Clipboard destination bounds reversed"));
        }
        let mut chunks=BTreeMap::new();
        let mut changes=Vec::new();
        let mut index=0usize;
        for y in destination_min.1..=max.1{
            for z in destination_min.2..=max.2{
                for x in destination_min.0..=max.0{
                    let pos=(x,y,z);
                    let chunk=original_chunk(&mut chunks,&self.world,&self.dimension,pos)?;
                    if chunk.has_block_entity(x,y,z){
                        return Err(refuse("Clipboard paste target contains original Block Entity"));
                    }
                    let numeric=matches!(chunk.layout,Layout::McRegion128|Layout::NumericAnvil112);
                    if numeric!=clip.numeric {
                        return Err(refuse("Clipboard block identity is incompatible with target layout"));
                    }
                    let original=chunk.block(x.rem_euclid(16) as u8,y,z.rem_euclid(16) as u8)
                        .ok_or_else(||refuse("Clipboard paste target Section absent"))?;
                    let after=clip.blocks[index].clone();
                    index+=1;
                    let before=self.overlay.get(&pos).unwrap_or(original);
                    if before!=&after{
                        changes.push(Change{
                            position:pos,original:original.clone(),
                            before:before.clone(),after,
                        });
                    }
                }
            }
        }
        Ok(Preview{
            root:self.world.root.clone(),dimension:self.dimension.clone(),
            data_version:self.world.data_version,
            changes,source_checks:Vec::new(),is_copy:true,is_move:false,is_fill:false,
            examined:index,matched:index,skipped:0,source_chunks:chunks.len(),
        })
    }
    /// Rotation and mirrors are applied BEFORE normal destination checks,
    /// so modified BE targets and missing Sections still fail atomically.
    pub fn prepare_paste_clipboard_file_transformed(
        &self,path:&Path,destination_min:Position,orientation:HorizontalTransform,
    )->Result<Preview,WorldError>{
        let source=clipboard::load(&self.world,&self.dimension,path)?;
        let rotated=source.transformed(orientation)?;
        self.prepare_paste_clipboard(&rotated,destination_min)
    }
    pub fn prepare_paste_clipboard_file(
        &self,path:&Path,destination_min:Position,
    )->Result<Preview,WorldError>{
        let clip=clipboard::load(&self.world,&self.dimension,path)?;
        self.prepare_paste_clipboard(&clip,destination_min)
    }
    /// Import old numeric MCEdit schematic as a block-only paste preview.
    /// Nonempty entities, TileEntities and ticks are rejected, not lost.
    pub fn prepare_import_legacy_schematic(
        &self,path:&Path,destination_min:Position,
    )->Result<Preview,WorldError>{
        let source=schematic::import_legacy(&self.world,&self.dimension,path)?;
        self.prepare_paste_clipboard(&source,destination_min)
    }
    /// Import Sponge v2 palette-based named blocks; DataVersion must match.
    pub fn prepare_import_sponge_v2(
        &self,path:&Path,destination_min:Position,
    )->Result<Preview,WorldError>{
        let block_clip=schematic::import_sponge_v2(&self.world,&self.dimension,path)?;
        self.prepare_paste_clipboard(&block_clip,destination_min)
    }
    /// Export only BE-free named blocks to Sponge v2; no entity/biome data.
    pub fn export_sponge_v2(
        &self,bounds:Bounds,path:&Path,
    )->Result<usize,WorldError>{
        let block_clip=self.capture_clipboard(bounds)?;
        schematic::export_sponge_v2(&self.world,&block_clip,path)?;
        Ok(block_clip.blocks.len())
    }
    /// Write an external original-NBT-verified edit journal, never a world file.
    pub fn export_patch(&self, path: &Path) -> Result<usize, WorldError> {
        if self.overlay.is_empty() {return Err(refuse("No pending blocks to export"));}
        let bytes=patch::encode(&self.world,&self.dimension,&self.overlay)?;
        patch::save(path,&self.world,&bytes)?;
        Ok(self.overlay.len())
    }
    /// Import one fully preflighted original-world journal as a single Undo.
    pub fn import_patch(&mut self, path: &Path) -> Result<usize, WorldError> {
        if !self.overlay.is_empty() {
            return Err(refuse("Export or discard existing edits before importing"));
        }
        let bytes=patch::load(path)?;
        let entries=patch::decode(&self.world,&self.dimension,&bytes)?;
        let changes:Vec<Change>=entries.into_iter().map(|(position,original,after)|
            Change { position,before:original.clone(),original,after }
        ).collect();
        let count=changes.len();
        if count>0 {
            for change in &changes { self.set_effective(change,&change.after); }
            self.undo.push(changes);
            if self.undo.len()>MAX_HISTORY {self.undo.remove(0);}
            self.redo.clear();
        }
        Ok(count)
    }
    pub fn undo(&mut self) -> Result<bool, WorldError> {
        let Some(group) = self.undo.last() else { return Ok(false) };
        for change in group {
            if self.overlay.get(&change.position).unwrap_or(&change.original) != &change.after {
                return Err(refuse("Undo refused: staged block state no longer matches transaction"));
            }
        }
        let group = self.undo.pop().unwrap();
        for change in &group { self.set_effective(change, &change.before); }
        self.redo.push(group);
        Ok(true)
    }
    pub fn redo(&mut self) -> Result<bool, WorldError> {
        let Some(group) = self.redo.last() else { return Ok(false) };
        for change in group {
            if self.overlay.get(&change.position).unwrap_or(&change.original) != &change.before {
                return Err(refuse("Redo refused: staged block state no longer matches transaction"));
            }
        }
        let group = self.redo.pop().unwrap();
        for change in &group { self.set_effective(change, &change.after); }
        self.undo.push(group);
        Ok(true)
    }
    /// Explicit discard only; no world files are written.
    pub fn discard(&mut self) {
        self.overlay.clear();
        self.undo.clear();
        self.redo.clear();
    }
}
