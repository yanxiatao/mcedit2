//! Synthetic original disk region tests for native in-memory edit groups.
use flate2::{write::ZlibEncoder, Compression};
use mcedit2_world::editing::{parse_bounds, parse_state, render_state, Bounds, Session};
use mcedit2_world::{DimensionSummary, WorldSummary};
use mcedit2_world::block_lookup::Lookup;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct NativeWorld {
    path: PathBuf,
    original_region: Vec<u8>,
    modern: bool,
}
impl Drop for NativeWorld {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.path); }
}
fn tag(kind: u8, name: &str, payload: &[u8]) -> Vec<u8> {
    let mut data = vec![kind];
    data.extend((name.len() as u16).to_be_bytes());
    data.extend(name.as_bytes());
    data.extend(payload);
    data
}
fn compound(fields: Vec<Vec<u8>>) -> Vec<u8> {
    let mut body = fields.concat();
    body.push(0);
    body
}
fn nbt_string(value: &str) -> Vec<u8> {
    let mut bytes = (value.len() as u16).to_be_bytes().to_vec();
    bytes.extend(value.as_bytes());
    bytes
}
fn byte_array(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut payload = (bytes.len() as i32).to_be_bytes().to_vec();
    payload.extend(bytes);
    tag(7, name, &payload)
}
fn modern_chunk(id: &str) -> Vec<u8> {
    let states = ["minecraft:stone", "minecraft:oak_log"];
    let mut palette = vec![10];
    palette.extend(2i32.to_be_bytes());
    for state in states {
        let selected = if state == "minecraft:stone" { id } else { state };
        palette.extend(compound(vec![tag(8, "Name", &nbt_string(selected))]));
    }
    let mut longs = (256i32).to_be_bytes().to_vec();
    longs.extend(vec![0u8; 256 * 8]);
    let section = compound(vec![
        tag(1, "Y", &[(-4i8) as u8]),
        tag(10, "block_states", &compound(vec![
            tag(9, "palette", &palette),
            tag(12, "data", &longs),
        ])),
        tag(10, "OpaqueModData", &compound(vec![
            tag(8, "unknown:payload", &nbt_string("never rewrite me")),
        ])),
    ]);
    let mut sections = vec![10];
    sections.extend(1i32.to_be_bytes());
    sections.extend(section);
    tag(10, "", &compound(vec![
        tag(3, "DataVersion", &5125i32.to_be_bytes()),
        tag(3, "xPos", &0i32.to_be_bytes()),
        tag(3, "zPos", &0i32.to_be_bytes()),
        tag(9, "sections", &sections),
        tag(9, "block_entities", &[10, 0, 0, 0, 0]),
    ]))
}
fn region_bytes(chunk: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(chunk).unwrap();
    let compressed = encoder.finish().unwrap();
    assert!(compressed.len() + 5 <= 4096);
    let mut region = vec![0u8; 3 * 4096];
    region[0..4].copy_from_slice(&((2u32 << 8) | 1).to_be_bytes());
    let start = 2 * 4096;
    region[start..start + 4]
        .copy_from_slice(&((compressed.len() + 1) as u32).to_be_bytes());
    region[start + 4] = 2;
    region[start + 5..start + 5 + compressed.len()].copy_from_slice(&compressed);
    region
}
fn old_mcregion_chunk() -> Vec<u8> {
    tag(10, "", &compound(vec![tag(10, "Level", &compound(vec![
        tag(3, "xPos", &0i32.to_be_bytes()),
        tag(3, "zPos", &0i32.to_be_bytes()),
        byte_array("Blocks", &vec![1; 32768]),
        byte_array("Data", &vec![0; 16384]),
        tag(8, "OriginalModData", &nbt_string("opaque")),
    ]))]))
}
impl NativeWorld {
    fn create(modern: bool) -> Self {
        let name = format!("mcedit2-edit-{}-{}", std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed));
        let path = std::env::temp_dir().join(name);
        fs::create_dir_all(path.join("region")).unwrap();
        let chunk = if modern { modern_chunk("minecraft:stone") } else { old_mcregion_chunk() };
        let original_region = region_bytes(&chunk);
        let ext = if modern { "mca" } else { "mcr" };
        fs::write(path.join(format!("region/r.0.0.{ext}")), &original_region).unwrap();
        Self { path, original_region, modern }
    }
    fn metadata(&self) -> WorldSummary {
        WorldSummary {
            root: self.path.clone(),
            level_name: Some("Original 3D test".to_owned()),
            data_version: self.modern.then_some(5125),
            storage_format: Some(if self.modern { 19133 } else { 19132 }),
            version_name: Some(if self.modern { "26.4 Snapshot 5" } else { "Beta" }.to_owned()),
            dimensions: vec![DimensionSummary {
                name: "minecraft:overworld".into(),
                region_dir: self.path.join("region"),
                regions: 1, chunks: 1,
                extension: if self.modern { "mca" } else { "mcr" }.into(),
            }],
            warnings: vec![],
        }
    }
    fn bytes_now(&self) -> Vec<u8> {
        fs::read(self.path.join(format!("region/r.0.0.{}", if self.modern {"mca"} else {"mcr"}))).unwrap()
    }
}

#[test]
fn safe_named_snapshot_selection_commit_undo_redo_and_no_disk_writes() {
    let original = NativeWorld::create(true);
    let mut session = Session::new(original.metadata(), "minecraft:overworld").unwrap();
    let bounds = Bounds::new((0, -64, 0), (1, -64, 1)).unwrap();
    let stone = parse_state("minecraft:stone").unwrap();
    let replacement = parse_state("minecraft:oak_log[axis=y]").unwrap();
    let preview = session.prepare(bounds, &replacement, Some(&stone)).unwrap();
    assert_eq!(preview.examined, 4);
    assert_eq!(preview.matched, 4);
    assert_eq!(preview.changed(), 4);
    assert_eq!(preview.source_chunks, 1);
    assert_eq!(session.pending_cells(), 0);
    assert_eq!(original.bytes_now(), original.original_region);
    assert_eq!(session.commit(preview).unwrap(), 4);
    assert_eq!(session.undo_depth(), 1);
    assert_eq!(session.pending_cells(), 4);
    match session.visible_block((1, -64, 1)).unwrap() {
        Lookup::Present(info) => assert_eq!(info.state, replacement),
        other => panic!("unexpected staged block: {other:?}"),
    }
    assert_eq!(original.bytes_now(), original.original_region);
    assert!(session.undo().unwrap());
    assert_eq!(session.pending_cells(), 0);
    assert_eq!(session.redo_depth(), 1);
    assert!(session.redo().unwrap());
    assert_eq!(session.pending_cells(), 4);
    session.discard();
    assert_eq!(session.pending_cells(), 0);
    assert_eq!(session.undo_depth(), 0);
    assert_eq!(original.bytes_now(), original.original_region);
}

#[test]
fn layered_original_edit_transactions_recover_exact_prior_state() {
    let original = NativeWorld::create(true);
    let mut editor = Session::new(original.metadata(), "minecraft:overworld").unwrap();
    let box_ = parse_bounds(" 1, -64, 1; 0, -64, 0 ").unwrap();
    let first = parse_state("minecraft:oak_log[axis=x]").unwrap();
    let second = parse_state("minecraft:gold_block").unwrap();
    editor.commit(editor.prepare(box_, &first, None).unwrap()).unwrap();
    assert_eq!(editor.pending_cells(), 4);
    let p = editor.prepare(box_, &second, Some(&first)).unwrap();
    assert_eq!(p.matched, 4);
    editor.commit(p).unwrap();
    assert_eq!(editor.undo_depth(), 2);
    assert!(editor.undo().unwrap());
    match editor.visible_block((0, -64, 0)).unwrap() {
        Lookup::Present(b) => assert_eq!(b.state, first),
        other => panic!("unexpected block after undo: {other:?}"),
    }
    assert!(editor.undo().unwrap());
    assert_eq!(editor.pending_cells(), 0);
    assert!(editor.redo().unwrap());
    assert_eq!(editor.pending_cells(), 4);
    assert_eq!(original.bytes_now(), original.original_region);
}

#[test]
fn old_numeric_mcregion_states_remain_numeric_without_id_mapping() {
    let original = NativeWorld::create(false);
    let mut editor = Session::new(original.metadata(), "minecraft:overworld").unwrap();
    let box_ = Bounds::new((2, 64, 4), (3, 64, 4)).unwrap();
    let first = parse_state("numeric:1[data=0]").unwrap();
    let new = parse_state("numeric:301[data=7]").unwrap();
    assert_eq!(editor.prepare(box_, &new, Some(&first)).unwrap().changed(), 2);
    let p = editor.prepare(box_, &new, Some(&first)).unwrap();
    assert_eq!(editor.commit(p).unwrap(), 2);
    assert!(editor.undo().unwrap());
    assert!(editor.redo().unwrap());
    assert!(editor.prepare(box_, &parse_state("minecraft:stone").unwrap(), None).is_err());
    assert_eq!(original.bytes_now(), original.original_region);
}

#[test]
fn refusal_for_missing_chunks_sections_oversize_and_mismatched_layout() {
    let original = NativeWorld::create(true);
    let editor = Session::new(original.metadata(), "minecraft:overworld").unwrap();
    let state = parse_state("minecraft:air").unwrap();
    assert!(editor.prepare(Bounds::new((15,-64,0),(16,-64,0)).unwrap(), &state, None).is_err());
    assert!(editor.prepare(Bounds::new((0,320,0),(1,320,0)).unwrap(), &state, None).is_err());
    assert!(editor.prepare(Bounds::new((0,-64,0),(0,-64,0)).unwrap(),
                           &parse_state("numeric:5[data=0]").unwrap(), None).is_err());
    assert!(Bounds::new((0,0,0),(64,64,64)).is_err());
    assert!(Bounds::new((0,-3000,0),(0,-3000,0)).is_err());
    assert_eq!(editor.pending_cells(), 0);
    assert_eq!(original.bytes_now(), original.original_region);
}

#[test]
fn planned_edits_refuse_original_world_change_after_preview() {
    let source = NativeWorld::create(true);
    let mut editor = Session::new(source.metadata(), "minecraft:overworld").unwrap();
    let bounds = Bounds::new((0,-64,0),(0,-64,0)).unwrap();
    let next = parse_state("minecraft:gold_block").unwrap();
    let pending = editor.prepare(bounds, &next, None).unwrap();
    let changed = region_bytes(&modern_chunk("minecraft:granite"));
    let filepath = source.path.join("region/r.0.0.mca");
    fs::write(&filepath, &changed).unwrap();
    assert!(editor.commit(pending).is_err());
    assert_eq!(editor.pending_cells(), 0);
    assert_eq!(editor.undo_depth(), 0);
    assert_eq!(fs::read(filepath).unwrap(), changed);
}

#[test]
fn target_parser_strictly_rejects_unsafe_and_ambiguous_ids() {
    let accepted = parse_state("minecraft:oak_log[waterlogged=false,axis=y]").unwrap();
    assert_eq!(render_state(&accepted), "minecraft:oak_log[axis=y,waterlogged=false]");
    assert_eq!(parse_state("numeric:4095[data=15]").unwrap().name, "numeric:4095");
    for invalid in [
        "oak_log", "minecraft:", "Minecraft:stone", "minecraft:stone[]",
        "minecraft:stone[x=1,x=2]", "numeric:4096[data=3]",
        "numeric:12", "numeric:12[data=16]", "minecraft:stone[x=]",
        "minecraft:stone[x=y", "minecraft:stone[x=y]junk",
    ] { assert!(parse_state(invalid).is_err(), "accepted invalid {invalid}"); }
}


fn chunk_with_be(state: &str, position: (i32, i32, i32)) -> Vec<u8> {
    let mut raw = modern_chunk(state);
    let empty = tag(9, "block_entities", &[10, 0, 0, 0, 0]);
    let mut entries = vec![10];
    entries.extend(1i32.to_be_bytes());
    entries.extend(compound(vec![
        tag(8, "id", &nbt_string("mod:precious_machine")),
        tag(3, "x", &position.0.to_be_bytes()),
        tag(3, "y", &position.1.to_be_bytes()),
        tag(3, "z", &position.2.to_be_bytes()),
        tag(8, "UnrecognizedPayload", &nbt_string("KEEP EXACTLY")),
        tag(5, "MachineHeat", &25.0f32.to_bits().to_be_bytes()),
        tag(6, "MachinePressure", &1234.5f64.to_bits().to_be_bytes()),
        tag(10, "ModdedCompound", &compound(vec![
            tag(4, "CycleCount", &123i64.to_be_bytes()),
            tag(8, "OwnerNote", &nbt_string("OPAQUE NESTED VALUE")),
            tag(10, "Inner", &compound(vec![
                tag(3, "BurnTime", &14i32.to_be_bytes()),
                tag(8, "UnknownNested", &nbt_string("keep deep sibling")),
            ])),
        ])),
    ]));
    let replacement = tag(9, "block_entities", &entries);
    let at = raw.windows(empty.len()).position(|part| part == empty).unwrap();
    raw.splice(at..at + empty.len(), replacement);
    raw
}
fn overwrite_region(world: &NativeWorld, chunk: &[u8]) {
    let extension = if world.modern { "mca" } else { "mcr" };
    fs::write(
        world.path.join(format!("region/r.0.0.{extension}")),
        region_bytes(chunk),
    ).unwrap();
}

#[test]
fn original_modded_block_entity_is_protected_but_neighbor_can_be_staged() {
    let world = NativeWorld::create(true);
    let chunk = chunk_with_be("minecraft:stone", (1, -64, 1));
    overwrite_region(&world, &chunk);
    let untouched = world.bytes_now();
    let mut editor = Session::new(world.metadata(), "minecraft:overworld").unwrap();
    let target = parse_state("minecraft:diamond_block").unwrap();
    let be = Bounds::new((1, -64, 1), (1, -64, 1)).unwrap();
    let neighbor = Bounds::new((2, -64, 1), (2, -64, 1)).unwrap();
    assert!(editor.prepare(be, &target, None).is_err());
    assert!(editor.prepare(Bounds::new((0,-64,0),(2,-64,1)).unwrap(), &target, None).is_err());
    assert_eq!(editor.pending_cells(), 0);
    assert_eq!(editor.commit(editor.prepare(neighbor, &target, None).unwrap()).unwrap(), 1);
    assert_eq!(world.bytes_now(), untouched);
}

#[test]
fn late_block_entity_appearance_cancels_prepared_transaction_atomically() {
    let world = NativeWorld::create(true);
    let mut editor = Session::new(world.metadata(), "minecraft:overworld").unwrap();
    let target = parse_state("minecraft:gold_block").unwrap();
    let box_ = Bounds::new((0, -64, 0), (2, -64, 0)).unwrap();
    let prepared = editor.prepare(box_, &target, None).unwrap();
    overwrite_region(&world, &chunk_with_be("minecraft:stone", (1,-64,0)));
    let on_disk = world.bytes_now();
    assert!(editor.commit(prepared).is_err());
    assert_eq!(editor.pending_cells(), 0);
    assert_eq!(editor.undo_depth(), 0);
    assert_eq!(world.bytes_now(), on_disk);
}

#[test]
fn block_entity_with_invalid_or_external_coordinates_fails_closed() {
    let world = NativeWorld::create(true);
    overwrite_region(&world, &chunk_with_be("minecraft:stone", (16,-64,0)));
    let editor = Session::new(world.metadata(), "minecraft:overworld").unwrap();
    let target = parse_state("minecraft:gold_block").unwrap();
    assert!(editor.prepare(Bounds::new((0,-64,0),(0,-64,0)).unwrap(), &target, None).is_err());
    assert_eq!(editor.pending_cells(), 0);
}


#[test]
fn clone_uses_frozen_effective_source_with_overlap_and_one_undo() {
    let world = NativeWorld::create(true);
    let mut editor = Session::new(world.metadata(), "minecraft:overworld").unwrap();
    let first = Bounds::new((0,-64,0),(0,-64,0)).unwrap();
    let gold = parse_state("minecraft:gold_block").unwrap();
    editor.commit(editor.prepare(first,&gold,None).unwrap()).unwrap();
    let clone = editor.prepare_copy(Bounds::new((0,-64,0),(1,-64,0)).unwrap(),(1,-64,0)).unwrap();
    assert!(clone.is_copy);
    assert_eq!(clone.changed(), 1);
    assert_eq!(editor.commit(clone).unwrap(), 1);
    match editor.visible_block((1,-64,0)).unwrap() {
        Lookup::Present(v) => assert_eq!(v.state,gold),
        other => panic!("Unexpected clone result {other:?}"),
    }
    match editor.visible_block((2,-64,0)).unwrap() {
        Lookup::Present(v) => assert_eq!(v.state,parse_state("minecraft:stone").unwrap()),
        other => panic!("Unexpected frozen original {other:?}"),
    }
    assert_eq!(editor.undo_depth(),2);
    assert!(editor.undo().unwrap());
    assert_eq!(editor.pending_cells(),1);
    assert!(editor.redo().unwrap());
    assert_eq!(world.bytes_now(),world.original_region);
}

#[test]
fn clone_refuses_original_block_entities_in_source_and_destination() {
    let world = NativeWorld::create(true);
    overwrite_region(&world,&chunk_with_be("minecraft:stone",(1,-64,0)));
    let mut editor = Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let source_be = Bounds::new((1,-64,0),(1,-64,0)).unwrap();
    let source_safe = Bounds::new((0,-64,0),(0,-64,0)).unwrap();
    assert!(editor.prepare_copy(source_be,(4,-64,0)).is_err());
    // Source is different from destination, but replacing an existing BE
    // must fail even if its unknown NBT is valid.
    let gold = parse_state("minecraft:gold_block").unwrap();
    editor.commit(editor.prepare(source_safe,&gold,None).unwrap()).unwrap();
    assert!(editor.prepare_copy(source_safe,(1,-64,0)).is_err());
    assert_eq!(editor.pending_cells(),1);
}

#[test]
fn clone_preview_is_invalidated_by_source_overlay_or_original_disk_change() {
    let world = NativeWorld::create(true);
    let mut editor = Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let src = Bounds::new((0,-64,0),(1,-64,0)).unwrap();
    let clone = editor.prepare_copy(src,(4,-64,0)).unwrap();
    let first = Bounds::new((0,-64,0),(0,-64,0)).unwrap();
    let gold = parse_state("minecraft:gold_block").unwrap();
    editor.commit(editor.prepare(first,&gold,None).unwrap()).unwrap();
    assert!(editor.commit(clone).is_err());
    assert_eq!(editor.pending_cells(),1);
    editor.discard();
    let clone = editor.prepare_copy(src,(4,-64,0)).unwrap();
    overwrite_region(&world,&chunk_with_be("minecraft:stone",(0,-64,0)));
    let bytes = world.bytes_now();
    assert!(editor.commit(clone).is_err());
    assert_eq!(editor.pending_cells(),0);
    assert_eq!(world.bytes_now(),bytes);
}

#[test]
fn clone_legacy_numeric_preserves_original_id_and_never_writes() {
    let world=NativeWorld::create(false);
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let one=Bounds::new((0,64,0),(0,64,0)).unwrap();
    let modified=parse_state("numeric:301[data=7]").unwrap();
    editor.commit(editor.prepare(one,&modified,None).unwrap()).unwrap();
    let clone=editor.prepare_copy(one,(7,64,7)).unwrap();
    assert_eq!(editor.commit(clone).unwrap(),1);
    match editor.visible_block((7,64,7)).unwrap(){
        Lookup::Present(v)=>assert_eq!(v.state,modified),
        other=>panic!("Unexpected old numeric clone {other:?}"),
    }
    assert!(editor.prepare_copy(one,(0,-200,0)).is_err());
    assert_eq!(world.bytes_now(),world.original_region);
}

#[test]
fn read_only_slice_distinguishes_missing_chunks_and_current_overlay() {
    let world=NativeWorld::create(true);
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let bounds=Bounds::new((14,-64,0),(17,-64,1)).unwrap();
    let original=editor.inspect_slice(bounds).unwrap();
    assert_eq!(original.present,4);
    assert_eq!(original.missing_chunks,4);
    assert_eq!(original.rows.len(),2);
    assert_eq!(original.rows[0].len(),4);
    let gold=parse_state("minecraft:gold_block").unwrap();
    let cell=Bounds::new((15,-64,0),(15,-64,0)).unwrap();
    editor.commit(editor.prepare(cell,&gold,None).unwrap()).unwrap();
    assert!(editor.inspect_slice(bounds).unwrap().text_map().contains("minecraft:gold_block"));
    assert_eq!(world.bytes_now(),world.original_region);
}
#[test]
fn slice_shows_be_marker_without_changing_opaque_nbt() {
    let world=NativeWorld::create(true);
    overwrite_region(&world,&chunk_with_be("minecraft:stone",(0,-64,0)));
    let bytes=world.bytes_now();
    let editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let slice=editor.inspect_slice(Bounds::new((0,-64,0),(1,-64,1)).unwrap()).unwrap();
    assert_eq!(slice.block_entities,1);
    assert!(slice.text_map().contains("|@"));
    assert_eq!(world.bytes_now(),bytes);
    assert!(editor.inspect_slice(Bounds::new((0,-64,0),(0,-63,0)).unwrap()).is_err());
}
#[test]
fn numeric_slice_displays_original_modded_id() {
    let world=NativeWorld::create(false);
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let cell=Bounds::new((0,64,0),(0,64,0)).unwrap();
    let state=parse_state("numeric:301[data=7]").unwrap();
    editor.commit(editor.prepare(cell,&state,None).unwrap()).unwrap();
    assert!(editor.inspect_slice(cell).unwrap().text_map().contains("numeric:301[data=7]"));
    assert_eq!(world.bytes_now(),world.original_region);
}

#[test]
fn connected_fill_respects_staged_state_barrier_and_undo_group() {
    let world=NativeWorld::create(true);
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let barrier=Bounds::new((1,-64,0),(1,-64,0)).unwrap();
    let gold=parse_state("minecraft:gold_block").unwrap();
    let diamond=parse_state("minecraft:diamond_block").unwrap();
    editor.commit(editor.prepare(barrier,&gold,None).unwrap()).unwrap();
    let bounds=Bounds::new((0,-64,0),(2,-64,0)).unwrap();
    let fill=editor.prepare_connected_fill(bounds,(0,-64,0),&diamond).unwrap();
    assert!(fill.is_fill);
    assert_eq!(fill.changed(),1);
    editor.commit(fill).unwrap();
    let check=|session: &Session, x|match session.visible_block((x,-64,0)).unwrap(){
        Lookup::Present(v)=>v.state,
        other=>panic!("Unexpected fill state {other:?}"),
    };
    assert_eq!(check(&editor,0),diamond);
    assert_eq!(check(&editor,1),gold);
    assert_eq!(check(&editor,2),parse_state("minecraft:stone").unwrap());
    assert!(editor.undo().unwrap());
    assert_eq!(check(&editor,0),parse_state("minecraft:stone").unwrap());
    assert_eq!(world.bytes_now(),world.original_region);
}
#[test]
fn connected_fill_refuses_be_and_missing_original_chunks_atomically() {
    let world=NativeWorld::create(true);
    overwrite_region(&world,&chunk_with_be("minecraft:stone",(1,-64,0)));
    let bytes=world.bytes_now();
    let editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let target=parse_state("minecraft:gold_block").unwrap();
    assert!(editor.prepare_connected_fill(
        Bounds::new((0,-64,0),(2,-64,0)).unwrap(),(0,-64,0),&target).is_err());
    assert!(editor.prepare_connected_fill(
        Bounds::new((15,-64,0),(16,-64,0)).unwrap(),(15,-64,0),&target).is_err());
    assert!(editor.prepare_connected_fill(
        Bounds::new((0,-64,0),(2,-64,0)).unwrap(),(4,-64,0),&target).is_err());
    assert_eq!(world.bytes_now(),bytes);
}
#[test]
fn connected_fill_legacy_numeric_guards_identity_and_late_change() {
    let world=NativeWorld::create(false);
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let bounds=Bounds::new((0,64,0),(1,64,1)).unwrap();
    let modded=parse_state("numeric:301[data=7]").unwrap();
    let pending=editor.prepare_connected_fill(bounds,(0,64,0),&modded).unwrap();
    assert_eq!(pending.changed(),4);
    assert_eq!(editor.commit(pending).unwrap(),4);
    assert!(editor.prepare_connected_fill(bounds,(0,64,0),
        &parse_state("minecraft:stone").unwrap()).is_err());
    assert!(editor.undo().unwrap());
    assert!(editor.redo().unwrap());
    assert_eq!(world.bytes_now(),world.original_region);
}

#[test]
fn exact_native_chunk_bytes_match_stored_compressed_chunk_source() {
    use mcedit2_world::block_lookup::{read_world_chunk,read_world_chunk_bytes};
    use mcedit2_world::sections::decode_sections;
    for modern in [false,true] {
        let source=NativeWorld::create(modern);
        let world=source.metadata();
        let bytes=read_world_chunk_bytes(&world,"minecraft:overworld",0,0)
            .unwrap().expect("original NBT source");
        let parsed=decode_sections(&bytes).unwrap();
        let original=read_world_chunk(&world,"minecraft:overworld",0,0).unwrap().unwrap();
        assert_eq!(parsed.layout,original.layout);
        assert_eq!(parsed.x,original.x);
        assert_eq!(parsed.z,original.z);
        assert!(read_world_chunk_bytes(&world,"minecraft:overworld",1,0).unwrap().is_none());
        assert_eq!(source.bytes_now(),source.original_region);
    }
}

fn journal_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "mcedit2-original-journal-{}-{}.mcedit2patch",
        std::process::id(),COUNTER.fetch_add(1,Ordering::Relaxed)
    ))
}
#[test]
fn external_journal_restores_old_numeric_and_modern_original_blocks_without_world_writes(){
    for modern in [false,true]{
        let world=NativeWorld::create(modern);
        let path=journal_path();
        let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
        let y=if modern{-64}else{64};
        let bounds=Bounds::new((0,y,0),(1,y,1)).unwrap();
        let replacement=parse_state(if modern {
            "minecraft:emerald_block"
        }else{"numeric:301[data=7]"}).unwrap();
        editor.commit(editor.prepare(bounds,&replacement,None).unwrap()).unwrap();
        let before=world.bytes_now();
        assert_eq!(editor.export_patch(&path).unwrap(),4);
        assert_eq!(world.bytes_now(),before);
        let mut reopened=Session::new(world.metadata(),"minecraft:overworld").unwrap();
        assert_eq!(reopened.import_patch(&path).unwrap(),4);
        assert_eq!(reopened.pending_cells(),4);
        assert_eq!(reopened.undo_depth(),1);
        match reopened.visible_block((0,y,0)).unwrap(){
            Lookup::Present(value)=>assert_eq!(value.state,replacement),
            other=>panic!("Unexpected journal result {other:?}"),
        }
        assert!(reopened.undo().unwrap());
        assert_eq!(reopened.pending_cells(),0);
        assert!(reopened.redo().unwrap());
        assert_eq!(reopened.pending_cells(),4);
        assert!(reopened.export_patch(&path).is_err(),"create-new must not overwrite");
        assert_eq!(world.bytes_now(),before);
        fs::remove_file(&path).unwrap();
    }
}
#[test]
fn journal_refuses_changed_opaque_nbt_and_preserves_empty_session(){
    let world=NativeWorld::create(true);
    let path=journal_path();
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let cell=Bounds::new((1,-64,0),(1,-64,0)).unwrap();
    let gold=parse_state("minecraft:gold_block").unwrap();
    editor.commit(editor.prepare(cell,&gold,None).unwrap()).unwrap();
    editor.export_patch(&path).unwrap();
    // Change only the NBT payload/BE list, leaving this target block state equal.
    overwrite_region(&world,&chunk_with_be("minecraft:stone",(8,-64,8)));
    let actual=world.bytes_now();
    let mut recovered=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    assert!(recovered.import_patch(&path).is_err());
    assert_eq!(recovered.pending_cells(),0);
    assert_eq!(recovered.undo_depth(),0);
    assert_eq!(world.bytes_now(),actual);
    fs::remove_file(path).unwrap();
}
#[test]
fn malformed_journal_duplicate_coordinate_and_foreign_dimension_fail_closed(){
    let world=NativeWorld::create(true);
    let path=journal_path();
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let target=parse_state("minecraft:diamond_block").unwrap();
    let cell=Bounds::new((0,-64,0),(0,-64,0)).unwrap();
    editor.commit(editor.prepare(cell,&target,None).unwrap()).unwrap();
    editor.export_patch(&path).unwrap();
    let original=fs::read(&path).unwrap();
    let mut json:serde_json::Value=serde_json::from_slice(&original).unwrap();
    json["dimension"]=serde_json::json!("minecraft:the_end");
    fs::write(&path,serde_json::to_vec(&json).unwrap()).unwrap();
    let mut empty=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    assert!(empty.import_patch(&path).is_err());
    let mut json:serde_json::Value=serde_json::from_slice(&original).unwrap();
    let duplicated=json["edits"][0].clone();
    json["edits"].as_array_mut().unwrap().push(duplicated);
    fs::write(&path,serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(empty.import_patch(&path).is_err());
    assert_eq!(empty.pending_cells(),0);
    fs::write(&path,b"not a journal").unwrap();
    assert!(empty.import_patch(&path).is_err());
    fs::remove_file(path).unwrap();
}
#[test]
fn journal_cannot_be_saved_inside_the_actual_world_or_over_existing_checkpoint(){
    let world=NativeWorld::create(true);
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let bounds=Bounds::new((0,-64,0),(0,-64,0)).unwrap();
    editor.commit(editor.prepare(bounds,&parse_state("minecraft:gold_block").unwrap(),None).unwrap()).unwrap();
    let unsafe_path=world.path.join("my-edits.mcedit2patch");
    assert!(editor.export_patch(&unsafe_path).is_err());
    assert!(!unsafe_path.exists());
    let outside=journal_path();
    fs::write(&outside,b"existing user checkpoint").unwrap();
    assert!(editor.export_patch(&outside).is_err());
    assert_eq!(fs::read(&outside).unwrap(),b"existing user checkpoint");
    fs::remove_file(outside).unwrap();
}

#[test]
fn original_byte_native_palette_candidate_is_in_memory_and_retains_opaque_data(){
    use mcedit2_world::sections::decode_sections;
    let world=NativeWorld::create(true);
    let original=world.bytes_now();
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let source=Bounds::new((0,-64,0),(2,-64,1)).unwrap();
    let oak=parse_state("minecraft:oak_log").unwrap();
    editor.commit(editor.prepare(source,&oak,None).unwrap()).unwrap();
    let candidates=editor.prepare_native_palette_candidates().unwrap();
    assert_eq!(candidates.len(),1);
    let chunk=&candidates[0];
    assert_eq!(chunk.edited_blocks,6);
    assert_eq!(chunk.original_nbt_sha256.len(),64);
    assert_eq!(decode_sections(&chunk.patched_nbt).unwrap().block(2,-64,1).unwrap(),&oak);
    assert!(chunk.patched_nbt.windows(b"never rewrite me".len())
        .any(|v|v==b"never rewrite me"));
    assert_eq!(world.bytes_now(),original);
}
#[test]
fn native_nbt_preview_grows_palette_and_edits_legacy_numeric_without_mutations(){
    let world=NativeWorld::create(true);
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let single=Bounds::new((0,-64,0),(0,-64,0)).unwrap();
    editor.commit(editor.prepare(single,&parse_state("minecraft:gold_block").unwrap(),None).unwrap()).unwrap();
    let candidate=editor.prepare_native_palette_candidates().unwrap();
    assert_eq!(candidate.len(),1);
    assert_eq!(mcedit2_world::sections::decode_sections(&candidate[0].patched_nbt)
        .unwrap().block(0,-64,0).unwrap().name,"minecraft:gold_block");
    assert_eq!(editor.pending_cells(),1);
    assert_eq!(world.bytes_now(),world.original_region);
    let legacy=NativeWorld::create(false);
    let mut old=Session::new(legacy.metadata(),"minecraft:overworld").unwrap();
    let b=Bounds::new((0,64,0),(0,64,0)).unwrap();
    // MCRegion uses 8-bit numeric IDs; the positive path must use <= 255.
    // The separate region_image_plan_rejects_unrepresentable_* regression
    // explicitly tests that a 12-bit ID (301) is refused.
    old.commit(old.prepare(b,&parse_state("numeric:201[data=7]").unwrap(),None).unwrap()).unwrap();
    let candidates=old.prepare_native_palette_candidates().unwrap();
    assert_eq!(candidates.len(),1);
    assert_eq!(mcedit2_world::sections::decode_sections(&candidates[0].patched_nbt)
        .unwrap().block(0,64,0).unwrap().name,"numeric:201");
    assert_eq!(legacy.bytes_now(),legacy.original_region);
}

#[test]
fn session_produces_complete_append_only_region_image_without_changing_java_world(){
    for modern in [true,false] {
        let fixture=NativeWorld::create(modern);
        let original=fixture.bytes_now();
        let mut editor=Session::new(fixture.metadata(),"minecraft:overworld").unwrap();
        let y=if modern{-64}else{64};
        let replacement=parse_state(if modern{
            "minecraft:gold_block"
        }else{"numeric:42[data=9]"}).unwrap();
        let bounds=Bounds::new((0,y,0),(1,y,0)).unwrap();
        editor.commit(editor.prepare(bounds,&replacement,None).unwrap()).unwrap();
        let images=editor.prepare_native_region_candidates(1_600_000_000).unwrap();
        assert_eq!(images.len(),1);
        let candidate=&images[0];
        assert_eq!(candidate.modified_chunks,1);
        assert_eq!(candidate.original_bytes,original.len());
        assert_eq!(candidate.original_region_sha256.len(),64);
        assert_eq!(candidate.region_x,0);
        assert_eq!(candidate.region_z,0);
        assert!(candidate.planned_bytes>candidate.original_bytes);
        assert_eq!(&candidate.image[8192..original.len()],&original[8192..]);
        assert_eq!(&candidate.image[4096..4100],&1_600_000_000u32.to_be_bytes());
        assert_eq!(fixture.bytes_now(),original,"dry run MUST NOT save a world");
        assert_eq!(editor.pending_cells(),2,"planning MUST preserve Undo overlay");
        assert!(editor.undo().unwrap());
        assert_eq!(fixture.bytes_now(),original);
    }
}

#[test]
fn region_image_plan_rejects_unrepresentable_mcregion_modded_id_without_disk_changes(){
    let fixture=NativeWorld::create(false);
    let original=fixture.bytes_now();
    let mut editor=Session::new(fixture.metadata(),"minecraft:overworld").unwrap();
    let bounds=Bounds::new((0,64,0),(0,64,0)).unwrap();
    editor.commit(editor.prepare(bounds,&parse_state("numeric:301[data=7]").unwrap(),None).unwrap()).unwrap();
    assert!(editor.prepare_native_region_candidates(12).is_err());
    assert_eq!(editor.pending_cells(),1);
    assert_eq!(fixture.bytes_now(),original);
}

#[test]
fn one_region_transaction_can_be_finalized_and_retains_original_backup(){
    use mcedit2_world::region_transaction::finalize_single_region;
    let fixture=NativeWorld::create(true);
    let before=fixture.bytes_now();
    let mut editor=Session::new(fixture.metadata(),"minecraft:overworld").unwrap();
    let box_=Bounds::new((0,-64,0),(1,-64,0)).unwrap();
    let target=parse_state("minecraft:gold_block").unwrap();
    editor.commit(editor.prepare(box_,&target,None).unwrap()).unwrap();
    let receipt=editor.apply_single_region_save(23456).unwrap();
    assert_eq!(editor.pending_cells(),0);
    assert_ne!(fixture.bytes_now(),before);
    assert_eq!(fs::read(&receipt.backup_path).unwrap(),before);
    assert!(receipt.marker_path.is_file());
    let finished=finalize_single_region(&fixture.metadata(),"minecraft:overworld",0,0,"mca").unwrap();
    assert!(finished.marker_path.is_file());
    assert_eq!(fs::read(&finished.backup_path).unwrap(),before);
    let actual=mcedit2_world::block_lookup::read_world_block(
        &fixture.metadata(),"minecraft:overworld",0,-64,0).unwrap();
    match actual {
        Lookup::Present(block)=>assert_eq!(block.state,target),
        other=>panic!("Unexpected saved region {other:?}"),
    }
}
#[test]
fn pending_region_crash_recovery_restores_original_verbatim(){
    use mcedit2_world::region_transaction::{rollback_pending_region,finalize_single_region};
    let fixture=NativeWorld::create(false);
    let before=fixture.bytes_now();
    let mut editor=Session::new(fixture.metadata(),"minecraft:overworld").unwrap();
    let cell=Bounds::new((1,64,1),(1,64,1)).unwrap();
    editor.commit(editor.prepare(cell,&parse_state("numeric:7[data=3]").unwrap(),None).unwrap()).unwrap();
    let receipt=editor.apply_single_region_save(99).unwrap();
    assert_ne!(fixture.bytes_now(),before);
    let restored=rollback_pending_region(&fixture.metadata(),"minecraft:overworld",0,0,"mcr").unwrap();
    assert_eq!(fixture.bytes_now(),before);
    assert_eq!(fs::read(&restored.backup_path).unwrap(),before);
    assert!(restored.marker_path.exists());
    assert!(!receipt.marker_path.exists());
    assert!(finalize_single_region(&fixture.metadata(),"minecraft:overworld",0,0,"mcr").is_err());
}
#[test]
fn stale_or_tampered_region_backup_refuses_unsafe_recovery(){
    use mcedit2_world::region_transaction::{rollback_pending_region,finalize_single_region};
    let fixture=NativeWorld::create(true);
    let mut editor=Session::new(fixture.metadata(),"minecraft:overworld").unwrap();
    let cell=Bounds::new((1,-64,1),(1,-64,1)).unwrap();
    editor.commit(editor.prepare(cell,&parse_state("minecraft:diamond_block").unwrap(),None).unwrap()).unwrap();
    let receipt=editor.apply_single_region_save(4).unwrap();
    let current=fixture.bytes_now();
    fs::write(&receipt.backup_path,b"tampered backup").unwrap();
    assert!(rollback_pending_region(&fixture.metadata(),"minecraft:overworld",0,0,"mca").is_err());
    assert!(finalize_single_region(&fixture.metadata(),"minecraft:overworld",0,0,"mca").is_err());
    assert_eq!(fixture.bytes_now(),current);
}
#[test]
fn second_save_refuses_unresolved_marker_and_does_not_clobber_backup(){
    let fixture=NativeWorld::create(true);
    let mut editor=Session::new(fixture.metadata(),"minecraft:overworld").unwrap();
    let cell=Bounds::new((1,-64,1),(1,-64,1)).unwrap();
    editor.commit(editor.prepare(cell,&parse_state("minecraft:gold_block").unwrap(),None).unwrap()).unwrap();
    let receipt=editor.apply_single_region_save(88).unwrap();
    let saved=fixture.bytes_now();
    let backup=fs::read(&receipt.backup_path).unwrap();
    let mut followup=Session::new(fixture.metadata(),"minecraft:overworld").unwrap();
    followup.commit(followup.prepare(
        cell,&parse_state("minecraft:diamond_block").unwrap(),None
    ).unwrap()).unwrap();
    assert!(followup.apply_single_region_save(89).is_err());
    assert_eq!(fixture.bytes_now(),saved);
    assert_eq!(fs::read(receipt.backup_path).unwrap(),backup);
}

#[test]
fn incomplete_pending_region_write_stage_can_be_recovered_without_modifying_original(){
    use mcedit2_world::region_transaction::rollback_pending_region;
    let fixture=NativeWorld::create(true);
    let before=fixture.bytes_now();
    let mut editor=Session::new(fixture.metadata(),"minecraft:overworld").unwrap();
    let target=parse_state("minecraft:gold_block").unwrap();
    let point=Bounds::new((0,-64,0),(0,-64,0)).unwrap();
    editor.commit(editor.prepare(point,&target,None).unwrap()).unwrap();
    // The final rename is intentionally not invoked. Simulate a valid
    // pending state produced by a crash between stage fsync and rename.
    let plans=editor.prepare_native_region_candidates(678).unwrap();
    let candidate=&plans[0];
    let original_sha=candidate.original_region_sha256.clone();
    let new_sha={
        use sha2::{Digest,Sha256};
        Sha256::digest(&candidate.image).iter().map(|b|format!("{b:02x}")).collect::<String>()
    };
    let original_path=fixture.path.join("region/r.0.0.mca");
    let marker_path=original_path.with_file_name("r.0.0.mca.mcedit2.pending.json");
    let backup_path=original_path.with_file_name(format!("r.0.0.mca.mcedit2.{original_sha}.backup"));
    let stage_path=original_path.with_file_name("r.0.0.mca.mcedit2.stage");
    fs::write(&backup_path,&before).unwrap();
    fs::write(&stage_path,&candidate.image).unwrap();
    let marker=serde_json::json!({
        "version":1,
        "original_sha256":original_sha,
        "replacement_sha256":new_sha,
        "original_size":before.len(),
        "replacement_size":candidate.image.len(),
    });
    fs::write(&marker_path,serde_json::to_vec(&marker).unwrap()).unwrap();
    let receipt=rollback_pending_region(&fixture.metadata(),"minecraft:overworld",0,0,"mca").unwrap();
    assert_eq!(fixture.bytes_now(),before);
    assert!(!marker_path.exists());
    assert!(!stage_path.exists());
    assert!(receipt.marker_path.exists());
    assert_eq!(fs::read(&backup_path).unwrap(),before);
}

#[test]
fn canonical_region_candidate_identity_and_transaction_marker_are_recoverable(){
    use mcedit2_world::region_transaction::{finalize_single_region,rollback_pending_region};
    let fixture=NativeWorld::create(true);
    let original=fixture.bytes_now();
    let mut editor=Session::new(fixture.metadata(),"minecraft:overworld").unwrap();
    let target=parse_state("minecraft:gold_block").unwrap();
    let p=Bounds::new((0,-64,0),(0,-64,0)).unwrap();
    editor.commit(editor.prepare(p,&target,None).unwrap()).unwrap();
    let receipt=editor.apply_single_region_save(1024).unwrap();
    assert_ne!(fixture.bytes_now(),original);
    assert_eq!(fs::read(&receipt.backup_path).unwrap(),original);
    assert!(receipt.marker_path.is_file());
    let recovered=rollback_pending_region(
        &fixture.metadata(),"minecraft:overworld",0,0,"mca"
    ).unwrap();
    assert_eq!(fixture.bytes_now(),original);
    assert!(recovered.marker_path.is_file());
    assert!(finalize_single_region(
        &fixture.metadata(),"minecraft:overworld",0,0,"mca"
    ).is_err());
}

#[test]
fn original_world_block_clipboard_survives_reopen_and_pastes_one_undo_group(){
    let fixture=NativeWorld::create(true);
    let unchanged=fixture.bytes_now();
    let path=std::env::temp_dir().join(format!(
        "mcedit2-block-clipboard-{}-{}.mcedit2clip",
        std::process::id(),COUNTER.fetch_add(1,Ordering::Relaxed)
    ));
    let mut editor=Session::new(fixture.metadata(),"minecraft:overworld").unwrap();
    let area=Bounds::new((0,-64,0),(1,-64,1)).unwrap();
    let state=parse_state("minecraft:oak_log").unwrap();
    editor.commit(editor.prepare(area,&state,None).unwrap()).unwrap();
    assert_eq!(editor.export_clipboard(area,&path).unwrap(),4);
    assert!(editor.export_clipboard(area,&path).is_err(),"must never overwrite existing clipboard");
    assert_eq!(fixture.bytes_now(),unchanged);
    let mut reopened=Session::new(fixture.metadata(),"minecraft:overworld").unwrap();
    let preview=reopened.prepare_paste_clipboard_file(&path,(4,-64,4)).unwrap();
    assert_eq!(preview.changed(),4);
    reopened.commit(preview).unwrap();
    match reopened.visible_block((5,-64,5)).unwrap(){
        Lookup::Present(block)=>assert_eq!(block.state,state),
        other=>panic!("Unexpected staged clipboard state: {other:?}"),
    }
    assert_eq!(reopened.undo_depth(),1);
    assert!(reopened.undo().unwrap());
    assert_eq!(reopened.pending_cells(),0);
    assert_eq!(fixture.bytes_now(),unchanged);
    fs::remove_file(path).unwrap();
}
#[test]
fn original_block_only_clipboard_refuses_be_wrong_dimension_and_corrupted_size(){
    let fixture=NativeWorld::create(true);
    let path=std::env::temp_dir().join(format!(
        "mcedit2-block-clipboard-invalid-{}-{}.mcedit2clip",
        std::process::id(),COUNTER.fetch_add(1,Ordering::Relaxed)
    ));
    let mut editor=Session::new(fixture.metadata(),"minecraft:overworld").unwrap();
    let select=Bounds::new((0,-64,0),(0,-64,0)).unwrap();
    editor.export_clipboard(select,&path).unwrap();
    let original=fs::read(&path).unwrap();
    let mut obj:serde_json::Value=serde_json::from_slice(&original).unwrap();
    obj["size"]=serde_json::json!([16,16,16]);
    fs::write(&path,serde_json::to_vec(&obj).unwrap()).unwrap();
    assert!(editor.prepare_paste_clipboard_file(&path,(1,-64,1)).is_err());
    obj=serde_json::from_slice(&original).unwrap();
    obj["dimension"]=serde_json::json!("minecraft:the_end");
    fs::write(&path,serde_json::to_vec(&obj).unwrap()).unwrap();
    assert!(editor.prepare_paste_clipboard_file(&path,(1,-64,1)).is_err());
    assert_eq!(editor.pending_cells(),0);
    fs::remove_file(path).unwrap();
    overwrite_region(&fixture,&chunk_with_be("minecraft:stone",(0,-64,0)));
    let newer=Session::new(fixture.metadata(),"minecraft:overworld").unwrap();
    assert!(newer.capture_clipboard(select).is_err());
}

#[test]
fn opaque_original_block_entity_inspection_preserves_moded_fields_verbatim(){
    let fixture=NativeWorld::create(true);
    let chunk=chunk_with_be("minecraft:stone",(2,-64,3));
    overwrite_region(&fixture,&chunk);
    let original=fixture.bytes_now();
    let editor=Session::new(fixture.metadata(),"minecraft:overworld").unwrap();
    let value=editor.inspect_original_block_entity((2,-64,3)).unwrap().unwrap();
    assert_eq!(value.raw_id.as_deref(),Some("mod:precious_machine"));
    assert_eq!(value.original_compound_sha256.len(),64);
    assert!(value.fields.iter().any(|field|
        field.name=="UnrecognizedPayload"&&field.kind==8
    ));
    assert!(value.report().contains("Original Block Entity"));
    assert!(editor.inspect_original_block_entity((1,-64,3)).unwrap().is_none());
    assert_eq!(fixture.bytes_now(),original);
}

#[test]
fn original_vertical_plane_reports_negative_y_missing_sections_and_exact_block_selection(){
    use mcedit2_world::editing::{VerticalAxis,SliceCell};
    let world=NativeWorld::create(true);
    let editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let area=Bounds::new((0,-65,0),(2,-63,0)).unwrap();
    let plane=editor.inspect_vertical_slice(area).unwrap();
    assert_eq!(plane.axis,VerticalAxis::AlongX);
    assert_eq!(plane.rows.len(),3);
    assert_eq!(plane.rows[0].len(),3);
    assert_eq!(plane.coordinate(0,2),Some((2,-63,0)));
    assert_eq!(plane.coordinate(2,0),Some((0,-65,0)));
    assert_eq!(plane.coordinate(3,0),None);
    assert_eq!(plane.missing_sections,3);
    assert!(matches!(plane.rows[2][0],SliceCell::MissingSection));
    assert_eq!(world.bytes_now(),world.original_region);
    let z_plane=editor.inspect_vertical_slice(
        Bounds::new((0,-64,0),(0,-63,2)).unwrap()
    ).unwrap();
    assert_eq!(z_plane.axis,VerticalAxis::AlongZ);
    assert_eq!(z_plane.coordinate(0,2),Some((0,-63,2)));
}

#[test]
fn original_numeric_and_named_moves_clear_source_preserve_overlap_and_undo(){
    for modern in [false,true] {
        let world=NativeWorld::create(modern);
        let unchanged=world.bytes_now();
        let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
        let y=if modern{-64}else{64};
        let source=Bounds::new((0,y,0),(1,y,0)).unwrap();
        let replacement=parse_state(if modern{
            "minecraft:gold_block"
        }else{"numeric:5[data=2]"}).unwrap();
        editor.commit(editor.prepare(source,&replacement,None).unwrap()).unwrap();
        let preview=editor.prepare_move(source,(1,y,0)).unwrap();
        assert!(preview.is_move);
        assert!(preview.changed()>0);
        editor.commit(preview).unwrap();
        let air=if modern{
            parse_state("minecraft:air").unwrap()
        }else{
            parse_state("numeric:0[data=0]").unwrap()
        };
        match editor.visible_block((0,y,0)).unwrap() {
            Lookup::Present(details)=>assert_eq!(details.state,air),
            other=>panic!("Missing moved-source block {other:?}"),
        }
        for x in [1,2]{
            match editor.visible_block((x,y,0)).unwrap(){
                Lookup::Present(details)=>assert_eq!(details.state,replacement),
                other=>panic!("Missing move destination {other:?}"),
            }
        }
        assert!(editor.undo().unwrap());
        match editor.visible_block((0,y,0)).unwrap(){
            Lookup::Present(details)=>assert_eq!(details.state,replacement),
            other=>panic!("Failed to undo moved source {other:?}"),
        }
        assert_eq!(world.bytes_now(),unchanged);
    }
}
#[test]
fn original_be_refuses_move_of_source_or_destination_without_partial_stage(){
    let world=NativeWorld::create(true);
    overwrite_region(&world,&chunk_with_be("minecraft:stone",(1,-64,0)));
    let original=world.bytes_now();
    let editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let source=Bounds::new((0,-64,0),(1,-64,0)).unwrap();
    assert!(editor.prepare_move(source,(3,-64,0)).is_err());
    let from=Bounds::new((2,-64,0),(2,-64,0)).unwrap();
    assert!(editor.prepare_move(from,(1,-64,0)).is_err());
    assert_eq!(editor.pending_cells(),0);
    assert_eq!(world.bytes_now(),original);
}

#[test]
fn legacy_numeric_schematic_export_import_roundtrips_addblocks_and_meta(){
    use mcedit2_world::schematic;
    let world=NativeWorld::create(false);
    let original=world.bytes_now();
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let selection=Bounds::new((0,64,0),(1,64,1)).unwrap();
    let desired=parse_state("numeric:301[data=13]").unwrap();
    editor.commit(editor.prepare(selection,&desired,None).unwrap()).unwrap();
    let path=std::env::temp_dir().join(format!(
        "mcedit2-classic-export-{}-{}.schematic",std::process::id(),
        COUNTER.fetch_add(1,Ordering::Relaxed)
    ));
    assert_eq!(editor.export_legacy_schematic(selection,&path).unwrap(),4);
    let restored=schematic::import_legacy(&world.metadata(),"minecraft:overworld",&path).unwrap();
    assert_eq!(restored.size,[2,1,2]);
    assert!(restored.blocks.iter().all(|s|s==&desired));
    assert!(editor.export_legacy_schematic(selection,&path).is_err());
    fs::remove_file(path).unwrap();
    assert_eq!(world.bytes_now(),original);
}

#[test]
fn bounded_isometric_voxel_surface_uses_original_height_and_overlay_without_world_writes(){
    let world=NativeWorld::create(true);
    let original=world.bytes_now();
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    assert!(editor.inspect_surface(0,0,-64,-33,16,16).is_ok());
    assert!(editor.inspect_surface(0,0,-64,-32,16,16).is_err());
    assert!(editor.inspect_surface(0,0,-64,-33,17,16).is_err());
    let view=editor.inspect_surface(0,0,-64,-63,2,2).unwrap();
    assert_eq!(view.voxels.len(),4);
    assert_eq!(view.missing_columns,0);
    assert_eq!(view.voxels[0].y,-63);
    let area=Bounds::new((0,-63,0),(0,-63,0)).unwrap();
    editor.commit(editor.prepare(area,&parse_state("minecraft:gold_block").unwrap(),None).unwrap()).unwrap();
    let changed=editor.inspect_surface(0,0,-64,-63,2,2).unwrap();
    assert_eq!(changed.voxels[0].state.name,"minecraft:gold_block");
    assert_eq!(world.bytes_now(),original);
}

#[test]
fn surgical_be_scalar_candidate_changes_only_target_and_preserves_other_nbt(){
    use mcedit2_world::sections;
    let world=NativeWorld::create(true);
    let original_chunk=chunk_with_be("minecraft:stone",(2,-64,3));
    overwrite_region(&world,&original_chunk);
    let original_file=world.bytes_now();
    let patched=sections::patch_original_block_entity_scalar(
        &original_chunk,(2,-64,3),"UnrecognizedPayload","KEEP REPLACED EXACTLY"
    ).unwrap();
    assert!(patched.windows(b"KEEP REPLACED EXACTLY".len())
        .any(|x|x==b"KEEP REPLACED EXACTLY"));
    assert!(!patched.windows(b"KEEP EXACTLY".len())
        .any(|x|x==b"KEEP EXACTLY"));
    let original_inspection=sections::inspect_original_block_entity(
        &original_chunk,(2,-64,3)
    ).unwrap().unwrap();
    let updated=sections::inspect_original_block_entity(
        &patched,(2,-64,3)
    ).unwrap().unwrap();
    assert_eq!(updated.raw_id,original_inspection.raw_id);
    assert_ne!(updated.original_compound_sha256,original_inspection.original_compound_sha256);
    assert_eq!(updated.original_field_count,original_inspection.original_field_count);
    assert!(sections::patch_original_block_entity_scalar(
        &original_chunk,(2,-64,3),"id","mod:replacement"
    ).is_err());
    assert!(sections::patch_original_block_entity_scalar(
        &original_chunk,(2,-64,3),"unknown","any"
    ).is_err());
    let editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    assert!(editor.prepare_original_be_scalar(
        (2,-64,3),"UnrecognizedPayload","SECOND"
    ).is_ok());
    assert_eq!(editor.pending_cells(),0);
    assert_eq!(world.bytes_now(),original_file);
}

#[test]
fn external_original_be_nbt_snapshot_preserves_modded_compound_and_detects_source_drift(){
    let world=NativeWorld::create(true);
    overwrite_region(&world,&chunk_with_be("minecraft:stone",(2,-64,3)));
    let original=world.bytes_now();
    let editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let file=std::env::temp_dir().join(format!(
        "mcedit2-original-be-{}-{}.mcedit2be",
        std::process::id(),COUNTER.fetch_add(1,Ordering::Relaxed)
    ));
    let bytes=editor.export_original_be_snapshot((2,-64,3),&file).unwrap();
    assert!(bytes>0);
    let (position,length)=editor.verify_original_be_snapshot(&file).unwrap();
    assert_eq!(position,(2,-64,3));
    assert_eq!(length,bytes);
    assert!(editor.export_original_be_snapshot((2,-64,3),&file).is_err());
    assert_eq!(world.bytes_now(),original);
    overwrite_region(&world,&chunk_with_be("minecraft:gold_block",(2,-64,3)));
    assert!(editor.verify_original_be_snapshot(&file).is_err());
    fs::remove_file(file).unwrap();
}

#[test]
fn declarative_plugin_preview_uses_original_chunk_safety_and_a_single_undo(){
    let world=NativeWorld::create(true);
    let before=world.bytes_now();
    let path=std::env::temp_dir().join(format!(
        "mcedit2-test-{}-{}.mcedit2plugin.json",
        std::process::id(),COUNTER.fetch_add(1,Ordering::Relaxed)
    ));
    let manifest=serde_json::json!({
        "schema_version":1,"name":"test original-only 3D replacement",
        "only_data_version":5125,
        "only_dimension":"minecraft:overworld",
        "operation":{
            "kind":"replace_blocks",
            "minimum":[0,-64,0],"maximum":[1,-64,1],
            "with_state":"minecraft:gold_block",
            "only_if_state":"minecraft:stone"
        }
    });
    fs::write(&path,serde_json::to_vec(&manifest).unwrap()).unwrap();
    let mut session=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let preview=session.prepare_plugin_file(&path).unwrap();
    assert_eq!(preview.changed(),4);
    assert_eq!(session.pending_cells(),0);
    session.commit(preview).unwrap();
    assert_eq!(session.pending_cells(),4);
    assert!(session.undo().unwrap());
    assert_eq!(session.pending_cells(),0);
    assert_eq!(world.bytes_now(),before);
    fs::remove_file(path).unwrap();
}

#[test]
fn native_multilayer_original_exposed_volume_culls_hidden_blocks_and_shows_real_cavities(){
    use mcedit2_world::surface::{FACE_TOP,FACE_POS_X,FACE_NEG_X};
    let world=NativeWorld::create(true);
    let original=world.bytes_now();
    let mut session=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let solid=session.inspect_exposed_volume(0,0,-64,-62,3,3).unwrap();
    assert_eq!(solid.voxels.len(),26);
    assert!(!solid.voxels.iter().any(|v|(v.x,v.y,v.z)==(1,-63,1)));
    let hollow=Bounds::new((1,-63,1),(1,-63,1)).unwrap();
    let change=session.prepare(hollow,&parse_state("minecraft:air").unwrap(),None).unwrap();
    session.commit(change).unwrap();
    let exposed=session.inspect_exposed_volume(0,0,-64,-62,3,3).unwrap();
    assert_eq!(exposed.voxels.len(),26);
    assert!(exposed.voxels.iter().any(|v|
        (v.x,v.y,v.z)==(1,-64,1)&&v.faces&FACE_TOP!=0
    ));
    assert!(exposed.voxels.iter().any(|v|
        (v.x,v.y,v.z)==(0,-63,1)&&v.faces&FACE_POS_X!=0
    ));
    assert!(exposed.voxels.iter().any(|v|
        (v.x,v.y,v.z)==(2,-63,1)&&v.faces&FACE_NEG_X!=0
    ));
    assert!(session.inspect_exposed_volume(0,0,-64,-32,3,3).is_err());
    assert_eq!(world.bytes_now(),original);
}

#[test]
fn original_be_scalar_memory_staging_is_undoable_and_never_disk_writable(){
    let world=NativeWorld::create(true);
    overwrite_region(&world,&chunk_with_be("minecraft:stone",(2,-64,3)));
    let untouched=world.bytes_now();
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let candidate=editor.prepare_be_edit(
        (2,-64,3),"UnrecognizedPayload","FIRST STAGED"
    ).unwrap();
    editor.commit_be_edit(candidate).unwrap();
    assert_eq!(editor.pending_be_chunks(),1);
    assert!(editor.has_pending_changes());
    assert!(editor.prepare_native_region_candidates(123).is_err());
    assert!(editor.export_patch(&journal_path()).is_err());
    assert_ne!(
        editor.inspect_staged_be((2,-64,3)).unwrap().unwrap().original_compound_sha256,
        editor.inspect_original_block_entity((2,-64,3)).unwrap().unwrap().original_compound_sha256
    );
    assert!(editor.undo_be_edit().unwrap());
    assert_eq!(editor.pending_be_chunks(),0);
    assert!(editor.redo_be_edit().unwrap());
    assert_eq!(editor.pending_be_chunks(),1);
    editor.discard();
    assert!(!editor.has_pending_changes());
    assert_eq!(world.bytes_now(),untouched);
}
#[test]
fn be_scalar_stale_preview_and_changed_disk_are_rejected(){
    let world=NativeWorld::create(true);
    overwrite_region(&world,&chunk_with_be("minecraft:stone",(2,-64,3)));
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let first=editor.prepare_be_edit((2,-64,3),"UnrecognizedPayload","ONE").unwrap();
    editor.commit_be_edit(first).unwrap();
    let stale=editor.prepare_be_edit((2,-64,3),"UnrecognizedPayload","TWO").unwrap();
    let next=editor.prepare_be_edit((2,-64,3),"UnrecognizedPayload","THREE").unwrap();
    editor.commit_be_edit(next).unwrap();
    assert!(editor.commit_be_edit(stale).is_err());
    overwrite_region(&world,&chunk_with_be("minecraft:gold_block",(2,-64,3)));
    assert!(editor.undo_be_edit().is_err());
    assert_eq!(editor.pending_be_chunks(),1);
}

#[test]
fn be_journal_restores_modded_scalar_operations_and_separate_undo_atomically(){
    let world=NativeWorld::create(true);
    overwrite_region(&world,&chunk_with_be("minecraft:stone",(2,-64,3)));
    let original=world.bytes_now();
    let path=std::env::temp_dir().join(format!(
        "mcedit2-be-replay-{}-{}.mcedit2bejournal.json",
        std::process::id(),COUNTER.fetch_add(1,Ordering::Relaxed)
    ));
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    for text in ["FIRST MOD NBT","SECOND MOD NBT"]{
        let candidate=editor.prepare_be_edit((2,-64,3),"UnrecognizedPayload",text).unwrap();
        editor.commit_be_edit(candidate).unwrap();
    }
    assert_eq!(editor.export_be_journal(&path).unwrap(),2);
    assert!(editor.export_be_journal(&path).is_err(),"BE journal must be create-new");
    let mut recovered=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    assert_eq!(recovered.import_be_journal(&path).unwrap(),2);
    assert_eq!(recovered.pending_be_chunks(),1);
    assert_eq!(
        recovered.inspect_staged_be((2,-64,3)).unwrap().unwrap().original_compound_sha256,
        editor.inspect_staged_be((2,-64,3)).unwrap().unwrap().original_compound_sha256
    );
    assert!(recovered.undo_be_edit().unwrap());
    assert!(recovered.redo_be_edit().unwrap());
    assert!(recovered.prepare_native_region_candidates(1).is_err());
    assert_eq!(world.bytes_now(),original);
    fs::remove_file(path).unwrap();
}
#[test]
fn be_journal_source_drift_and_invalid_entry_are_refused_without_partial_import(){
    let world=NativeWorld::create(true);
    overwrite_region(&world,&chunk_with_be("minecraft:stone",(2,-64,3)));
    let path=std::env::temp_dir().join(format!(
        "mcedit2-be-tamper-{}-{}.mcedit2bejournal.json",
        std::process::id(),COUNTER.fetch_add(1,Ordering::Relaxed)
    ));
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let candidate=editor.prepare_be_edit((2,-64,3),"UnrecognizedPayload","VERIFY").unwrap();
    editor.commit_be_edit(candidate).unwrap();
    editor.export_be_journal(&path).unwrap();
    let original_bytes=fs::read(&path).unwrap();
    let mut file:serde_json::Value=serde_json::from_slice(&original_bytes).unwrap();
    file["edits"][0]["resulting_chunk_sha256"]=serde_json::json!("0".repeat(64));
    fs::write(&path,serde_json::to_vec(&file).unwrap()).unwrap();
    let mut empty=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    assert!(empty.import_be_journal(&path).is_err());
    assert_eq!(empty.pending_be_chunks(),0);
    fs::write(&path,&original_bytes).unwrap();
    overwrite_region(&world,&chunk_with_be("minecraft:gold_block",(2,-64,3)));
    assert!(empty.import_be_journal(&path).is_err());
    assert_eq!(empty.pending_be_chunks(),0);
    fs::remove_file(path).unwrap();
}

#[test]
fn be_only_and_combined_block_be_region_candidates_preserve_opaque_data(){
    let world=NativeWorld::create(true);
    overwrite_region(&world,&chunk_with_be("minecraft:stone",(2,-64,3)));
    let unchanged=world.bytes_now();
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let change=editor.prepare_be_edit(
        (2,-64,3),"UnrecognizedPayload","BE EXACT REPLAY"
    ).unwrap();
    editor.commit_be_edit(change).unwrap();
    let preview=editor.preview_be_region_candidates(47).unwrap();
    assert_eq!(preview.len(),1);
    assert_eq!(preview[0].be_replay.len(),1);
    assert!(preview[0].image.len()>unchanged.len());
    assert!(mcedit2_world::region_transaction::apply_single_region(
        &world.metadata(),"minecraft:overworld",&preview[0]
    ).is_err(),"BE Region preview remains in-memory only");
    assert_eq!(world.bytes_now(),unchanged);
    let area=Bounds::new((0,-64,0),(0,-64,0)).unwrap();
    editor.commit(editor.prepare(
        area,&parse_state("minecraft:gold_block").unwrap(),None
    ).unwrap()).unwrap();
    let mixed=editor.preview_be_region_candidates(49).unwrap();
    assert_eq!(mixed.len(),1);
    assert_eq!(mixed[0].be_replay.len(),1);
    assert_eq!(world.bytes_now(),unchanged);
}

#[test]
fn explicitly_confirmed_existing_be_scalar_region_save_is_recoverable_verbatim(){
    use mcedit2_world::region_transaction::rollback_pending_region;
    let world=NativeWorld::create(true);
    overwrite_region(&world,&chunk_with_be("minecraft:stone",(2,-64,3)));
    let unchanged=world.bytes_now();
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let be=editor.prepare_be_edit(
        (2,-64,3),"UnrecognizedPayload","SAVED WITH FULL NBT PROOF"
    ).unwrap();
    editor.commit_be_edit(be).unwrap();
    assert!(editor.apply_authenticated_be_single_region_save(56,"SAVE").is_err());
    assert_eq!(world.bytes_now(),unchanged);
    let receipt=editor.apply_authenticated_be_single_region_save(
        56,"SAVE_BE_NBT"
    ).unwrap();
    assert_ne!(world.bytes_now(),unchanged);
    assert_eq!(fs::read(&receipt.backup_path).unwrap(),unchanged);
    assert!(receipt.marker_path.is_file());
    let readback=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let updated=readback.inspect_original_block_entity((2,-64,3))
        .unwrap().unwrap();
    assert_eq!(updated.raw_id.as_deref(),Some("mod:precious_machine"));
    assert_ne!(updated.original_compound_sha256,
        mcedit2_world::sections::inspect_original_block_entity(
            &chunk_with_be("minecraft:stone",(2,-64,3)),(2,-64,3)
        ).unwrap().unwrap().original_compound_sha256);
    rollback_pending_region(&world.metadata(),"minecraft:overworld",0,0,"mca").unwrap();
    assert_eq!(world.bytes_now(),unchanged);
}

#[test]
fn opaque_be_float_and_double_scalar_edits_preserve_all_other_modded_nbt(){
    let source=chunk_with_be("minecraft:stone",(2,-64,3));
    let baseline=mcedit2_world::sections::inspect_original_block_entity(
        &source,(2,-64,3)
    ).unwrap().unwrap();
    let changed=mcedit2_world::sections::patch_original_block_entity_scalar(
        &source,(2,-64,3),"MachineHeat","27.5"
    ).unwrap();
    let changed=mcedit2_world::sections::patch_original_block_entity_scalar(
        &changed,(2,-64,3),"MachinePressure","4567.25"
    ).unwrap();
    let after=mcedit2_world::sections::inspect_original_block_entity(
        &changed,(2,-64,3)
    ).unwrap().unwrap();
    assert_eq!(after.raw_id,baseline.raw_id);
    assert_eq!(after.original_field_count,baseline.original_field_count);
    assert_ne!(after.original_compound_sha256,baseline.original_compound_sha256);
    for bad in ["NaN","+inf","-inf","1e9999"] {
        assert!(mcedit2_world::sections::patch_original_block_entity_scalar(
            &source,(2,-64,3),"MachineHeat",bad
        ).is_err(),"Accepted invalid Float {bad}");
    }
    assert!(mcedit2_world::sections::patch_original_block_entity_scalar(
        &source,(2,-64,3),"MachineHeat","25.0"
    ).is_err(),"Identical scalar values cannot produce unnecessary edits");
}

#[test]
fn compound_path_scalar_surgery_changes_one_leaf_without_reserializing_siblings(){
    let source=chunk_with_be("minecraft:stone",(2,-64,3));
    let changed=mcedit2_world::sections::patch_original_block_entity_scalar(
        &source,(2,-64,3),"ModdedCompound/Inner/BurnTime","80"
    ).unwrap();
    assert!(changed.windows(b"keep deep sibling".len()).any(|v|v==b"keep deep sibling"));
    assert!(changed.windows(b"OPAQUE NESTED VALUE".len()).any(|v|v==b"OPAQUE NESTED VALUE"));
    assert!(changed.windows(b"KEEP EXACTLY".len()).any(|v|v==b"KEEP EXACTLY"));
    assert!(mcedit2_world::sections::patch_original_block_entity_scalar(
        &source,(2,-64,3),"ModdedCompound/Inner/BurnTime","14"
    ).is_err());
    for path in [
        "ModdedCompound/Inner/UnknownNested/more","ModdedCompound//CycleCount",
        "ModdedCompound/id","ModdedCompound/Inner/x",
        "ModdedCompound/NoSuchField","ModdedCompound/Inner/BurnTime/",
    ]{
        assert!(mcedit2_world::sections::patch_original_block_entity_scalar(
            &source,(2,-64,3),path,"100"
        ).is_err(),"Accepted invalid nested path {path}");
    }
}

#[test]
fn declarative_modded_be_batch_is_all_or_nothing_without_world_writes(){
    let world=NativeWorld::create(true);
    overwrite_region(&world,&chunk_with_be("minecraft:stone",(2,-64,3)));
    let saved=world.bytes_now();
    let path=std::env::temp_dir().join(format!(
        "mcedit2-batch-{}-{}.mcedit2beops.json",
        std::process::id(),COUNTER.fetch_add(1,Ordering::Relaxed)
    ));
    let mut session=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let manifest=|invalid:bool|serde_json::json!({
        "version":1,"original_world_root":world.metadata().root.to_string_lossy(),
        "dimension":"minecraft:overworld",
        "data_version":world.metadata().data_version,
        "operations":[
            {"xyz":[2,-64,3],"field":"ModdedCompound/Inner/BurnTime","value":"90"},
            {"xyz":[2,-64,3],
                "field":if invalid{"no_such_field"}else{"MachineHeat"},
                "value":"99.5"}
        ]
    });
    fs::write(&path,serde_json::to_vec(&manifest(true)).unwrap()).unwrap();
    assert!(session.stage_be_operation_file(&path).is_err());
    assert_eq!(session.pending_be_chunks(),0);
    fs::write(&path,serde_json::to_vec(&manifest(false)).unwrap()).unwrap();
    assert_eq!(session.preview_be_operation_file(&path).unwrap(),(2,1));
    assert_eq!(session.pending_be_chunks(),0);
    assert_eq!(session.stage_be_operation_file(&path).unwrap(),2);
    assert_eq!(session.pending_be_chunks(),1);
    assert!(session.undo_be_edit().unwrap());
    assert!(session.undo_be_edit().unwrap());
    assert_eq!(session.pending_be_chunks(),0);
    assert_eq!(world.bytes_now(),saved);
    fs::remove_file(path).unwrap();
}

#[test]
fn region_batch_manifest_detects_stale_source_and_unmodified_world(){
    let world=NativeWorld::create(true);
    let unchanged=world.bytes_now();
    let path=std::env::temp_dir().join(format!(
        "mcedit2-regions-{}-{}.mcedit2regionplan.json",
        std::process::id(),COUNTER.fetch_add(1,Ordering::Relaxed)
    ));
    let mut editor=Session::new(world.metadata(),"minecraft:overworld").unwrap();
    let area=Bounds::new((0,-64,0),(1,-64,1)).unwrap();
    let preview=editor.prepare(area,&parse_state("minecraft:gold_block").unwrap(),None).unwrap();
    editor.commit(preview).unwrap();
    let report=editor.prepare_region_batch_report(99).unwrap();
    assert_eq!(report.regions.len(),1);
    assert_eq!(report.regions[0].edited_chunks,1);
    assert_eq!(editor.export_region_batch_report(99,&path).unwrap(),1);
    assert_eq!(editor.verify_region_batch_report(&path).unwrap(),1);
    assert!(editor.export_region_batch_report(99,&path).is_err());
    assert_eq!(world.bytes_now(),unchanged);
    overwrite_region(&world,&modern_chunk("minecraft:stone"));
    assert!(editor.verify_region_batch_report(&path).is_err());
    fs::remove_file(path).unwrap();
}
