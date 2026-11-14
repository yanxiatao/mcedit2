//! 按不可变源区块准备及编译，联合动态材质后发布完整场景；不保留历史整帧链。
use super::*;
use crate::textures::{runtime_atlas, AtlasUv};
use std::{fmt, path::PathBuf};

pub(super) struct Prepared {
    pub language: crate::language::Language,
    pub equipment: BTreeMap<usize, crate::item_geometry::Prepared>,
    pub names: BTreeMap<usize, crate::name_geometry::Prepared>,
    pub runtime_textures: BTreeMap<String, Arc<RuntimeTexture>>,
    pub frames: BTreeMap<usize, Result<crate::item_geometry::Prepared, String>>,
    pub skins: BTreeMap<usize, Result<Option<(String, bool)>, String>>,
    pub player_names: BTreeMap<String, String>,
    pub block_visuals: BTreeMap<usize, Result<(BlockVisual, Option<String>), String>>,
    pub sign_materials: BTreeMap<usize, Arc<crate::sign_geometry::Prepared>>,
    pub banner_materials: BTreeMap<usize, Arc<crate::banner_geometry::Prepared>>,
    pub diagnostics: Vec<String>,
}
#[derive(PartialEq)]
struct Settings {
    maps: Option<mcedit2_world::map_edit::View>,
    item_world: Option<crate::item_world::Environment>,
    resource: crate::textures::VisibilityIdentity,
    version: Option<i32>,
    root: Option<PathBuf>,
    language: crate::language::Language,
    textures: bool,
    entities: bool,
    online: bool,
    dimension: (usize, usize),
}
type Coordinate = (i32, i32);
// Arc 被 Part 的源引用持有，地址不会在缓存存活时被重用。重复源段仍保留其实际顺序。
type SourceKey = (usize, usize);
struct Part {
    sources: Vec<MarkerRef>,
    skins: SkinResults,
    heads: HeadResults,
    prepared: Arc<Prepared>,
    materials: Vec<(String, AtlasUv)>,
    geometry: Arc<Geometry>,
}
struct Geometry {
    visibility: Visibility,
    chunks: crate::picking::Chunks,
    entities: BTreeMap<Coordinate, Vec<MarkerRef>>,
    blocks: BTreeMap<[i32; 3], ManagedBlockVisual>,
    rendered: usize,
    missing: usize,
    invisible: usize,
    diagnostics: Vec<String>,
    diagnostic_counts: BTreeMap<String, usize>,
    names: BTreeMap<String, String>,
}
impl From<(Frame, BTreeMap<String, usize>)> for Geometry {
    fn from((frame, diagnostic_counts): (Frame, BTreeMap<String, usize>)) -> Self {
        Self {
            visibility: frame.visibility,
            chunks: frame.chunks,
            entities: frame.entities,
            blocks: frame.managed_blocks,
            rendered: frame.rendered,
            missing: frame.missing,
            invisible: frame.invisible,
            diagnostics: frame.diagnostics,
            diagnostic_counts,
            names: frame.player_names,
        }
    }
}
struct Joined {
    sources: Vec<MeshChunk>,
    mesh: MeshChunk,
}
#[derive(Clone, Copy, Default, serde::Serialize, Debug)]
pub(super) struct Statistics {
    pub prepared_parts: usize,
    pub reused_preparations: usize,
    pub compiled_parts: usize,
    pub reused_geometry: usize,
    pub built_meshes: usize,
    pub reused_meshes: usize,
    pub runtime_pages_built: usize,
    pub runtime_pages_reused: usize,
}
impl Statistics {
    pub fn add(&mut self, other: Self) {
        self.prepared_parts += other.prepared_parts;
        self.reused_preparations += other.reused_preparations;
        self.compiled_parts += other.compiled_parts;
        self.reused_geometry += other.reused_geometry;
        self.built_meshes += other.built_meshes;
        self.reused_meshes += other.reused_meshes;
        self.runtime_pages_built += other.runtime_pages_built;
        self.runtime_pages_reused += other.runtime_pages_reused;
    }
}
pub(super) struct Assembly {
    selected: Option<MarkerRef>,
    settings: Settings,
    snapshot: Arc<Snapshot>,
    parts: BTreeMap<SourceKey, Arc<Part>>,
    joined: BTreeMap<Coordinate, Joined>,
    runtime: Arc<runtime_atlas::Set>,
    pub statistics: Statistics,
}
impl fmt::Debug for Assembly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Assembly")
            .field("parts", &self.parts.len())
            .field("statistics", &self.statistics)
            .finish()
    }
}
/// 快照交接期间仍可核对的编译上下文；未知维度只允许保留缓存，不能发布画面。
pub(super) struct Context<'a> {
    pub maps: Option<&'a mcedit2_world::map_edit::View>,
    pub item_world: Option<&'a crate::item_world::Environment>,
    pub bank: &'a TextureBank,
    pub version: Option<i32>,
    pub root: &'a std::path::Path,
    pub language: crate::language::Language,
    pub textures: bool,
    pub entities: bool,
    pub online: bool,
    pub dimension: Option<(usize, usize)>,
}
pub(super) fn cache_matches_context(frame: &Frame, context: Context<'_>) -> bool {
    frame.assembly.as_ref().is_some_and(|old| {
        let s = &old.settings;
        s.resource == context.bank.visibility_id()
            && s.maps.as_ref() == context.maps
            && s.item_world.as_ref() == context.item_world
            && s.version == context.version
            && s.root.as_deref() == Some(context.root)
            && s.language == context.language
            && s.textures == context.textures
            && s.entities == context.entities
            && s.online == context.online
            && context
                .dimension
                .is_none_or(|dimension| s.dimension == dimension)
            && context
                .bank
                .atlas()
                .is_some_and(|base| Arc::ptr_eq(base, &old.runtime.base))
    })
}
pub(super) fn cache_matches(frame: &Frame, input: &Input) -> bool {
    cache_matches_context(
        frame,
        Context {
            maps: input.maps.as_ref(),
            item_world: input.item_world.as_ref(),
            bank: &input.bank,
            version: input.version,
            root: &input.root,
            language: input.language,
            textures: input.show_textures,
            entities: input.show_entities,
            online: input.online_skins,
            dimension: Some(input.snapshot.key),
        },
    )
}
fn same_results<K: Ord, V>(
    a: &BTreeMap<K, Result<Arc<V>, String>>,
    b: &BTreeMap<K, Result<Arc<V>, String>>,
) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|((ak, av), (bk, bv))| {
            ak == bk
                && match (av, bv) {
                    (Ok(a), Ok(b)) => Arc::ptr_eq(a, b),
                    (Err(a), Err(b)) => a == b,
                    _ => false,
                }
        })
}
fn dependencies(
    markers: &[MarkerRef],
    skins: &SkinResults,
    heads: &HeadResults,
    online: bool,
) -> (SkinResults, HeadResults) {
    let mut selected_skins = SkinResults::new();
    let mut selected_heads = HeadResults::new();
    for marker in markers.iter().map(MarkerRef::marker) {
        if online && marker.kind == Kind::Player {
            if let Some((uuid, result)) =
                player_uuid(marker).and_then(|id| skins.get(&id).map(|r| (id, r)))
            {
                selected_skins.insert(uuid, result.clone());
            }
        }
        if let Some(BlockVisual::Skull(source)) = marker.block_visual.as_deref() {
            if let Some(profile) = source
                .profile
                .as_ref()
                .filter(|_| source.kind == mcedit2_world::skull::Kind::Player)
            {
                let key = crate::skull_skin::key(profile, source.legacy_rotation.is_some());
                if let Some(result) = heads.get(&key) {
                    selected_heads.insert(key, result.clone());
                }
            }
        }
    }
    (selected_skins, selected_heads)
}
fn same_meshes(a: &[MeshChunk], b: &[MeshChunk]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.key == b.key
                && a.origin == b.origin
                && Arc::ptr_eq(&a.quads(), &b.quads())
                && Arc::ptr_eq(&a.picking, &b.picking)
        })
}
fn join(sources: &[MeshChunk], cancelled: &dyn Fn() -> bool) -> Result<MeshChunk, String> {
    if sources.len() == 1 {
        return Ok(sources[0].clone());
    }
    let origin = sources[0].origin;
    let mut faces = Vec::new();
    let mut owners = Vec::new();
    for source in sources {
        if cancelled() {
            return Err("实体区块合并已取消".into());
        }
        if source.origin != origin {
            return Err("同一实体区块的几何原点不一致".into());
        }
        let offset = faces.len();
        faces.extend_from_slice(&source.quads());
        owners.extend(
            source
                .picking
                .source_groups()
                .iter()
                .cloned()
                .map(|mut group| {
                    group.quads = group.quads.start + offset..group.quads.end + offset;
                    group
                }),
        );
    }
    let quads = Arc::new(faces);
    let picking = Arc::new(crate::picking::Index::build(
        origin,
        Arc::clone(&quads),
        owners,
    )?);
    Ok(MeshChunk::new(sources[0].key, origin, quads, picking))
}

fn selection_in(sources: &[MarkerRef], selected: Option<&MarkerRef>) -> Option<usize> {
    selected.and_then(|selected| sources.iter().position(|source| source.same(selected)))
}
pub(super) fn assemble(
    snapshot: Arc<Snapshot>,
    bank: &TextureBank,
    version: Option<i32>,
    show_textures: bool,
    show_entities: bool,
    cancelled: &dyn Fn() -> bool,
    root: Option<&std::path::Path>,
    online_skins: bool,
    skin_results: &SkinResults,
    head_results: &HeadResults,
    language: crate::language::Language,
    previous_frame: Option<&Arc<Frame>>,
    item_world: Option<&crate::item_world::Environment>,
    selected: Option<&MarkerRef>,
    visibility: Visibility,
    maps: Option<&mcedit2_world::map_edit::View>,
) -> Result<Frame, String> {
    let check = || {
        if cancelled() {
            Err("实体场景编译已取消".to_string())
        } else {
            Ok(())
        }
    };
    check()?;
    let base = bank
        .atlas()
        .cloned()
        .ok_or("实体模型缺少已完成的纹理图集")?;
    let settings = Settings {
        maps: maps.cloned(),
        item_world: item_world.cloned(),
        resource: bank.visibility_id(),
        version,
        root: root.map(std::path::Path::to_path_buf),
        language,
        textures: show_textures,
        entities: show_entities,
        online: online_skins,
        dimension: snapshot.key,
    };
    let previous = previous_frame
        .and_then(|frame| frame.assembly.as_ref())
        .filter(|old| old.settings == settings && Arc::ptr_eq(&old.runtime.base, &base));
    let mut groups: Vec<(SourceKey, Vec<MarkerRef>)> = Vec::new();
    let mut occurrences = BTreeMap::<usize, usize>::new();
    for (index, source) in snapshot.markers.iter().enumerate() {
        if index.is_multiple_of(1024) {
            check()?;
        }
        let id = Arc::as_ptr(&source.chunk) as usize;
        if groups.last().is_none_or(|(key, _)| key.0 != id) {
            let occurrence = occurrences.entry(id).or_default();
            groups.push(((id, *occurrence), Vec::new()));
            *occurrence += 1;
        }
        groups.last_mut().unwrap().1.push(source.clone());
    }
    let mut fonts = crate::font_glyphs::Fonts::new(bank, language);
    let mut prepared_parts = Vec::new();
    let mut textures: BTreeMap<String, Arc<RuntimeTexture>> = BTreeMap::new();
    let mut statistics = Statistics::default();
    for (key, sources) in groups {
        check()?;
        let (skins, heads) = dependencies(
            &sources,
            skin_results,
            head_results,
            show_entities && online_skins,
        );
        let old = previous.and_then(|old| old.parts.get(&key));
        let unchanged = old.filter(|old| {
            old.sources.len() == sources.len()
                && old
                    .sources
                    .iter()
                    .zip(&sources)
                    .all(|(a, b)| Arc::ptr_eq(&a.chunk, &b.chunk) && a.index == b.index)
                && same_results(&old.skins, &skins)
                && same_results(&old.heads, &heads)
        });
        let prepared = if let Some(old) = unchanged {
            statistics.reused_preparations += 1;
            Arc::clone(&old.prepared)
        } else {
            statistics.prepared_parts += 1;
            Arc::new(Frame::prepare(
                &sources,
                bank,
                version,
                show_textures,
                show_entities,
                cancelled,
                root,
                online_skins,
                &skins,
                &heads,
                language,
                &mut fonts,
                item_world,
                maps,
            )?)
        };
        for (id, texture) in &prepared.runtime_textures {
            if let Some(old) = textures.get(id) {
                if !Arc::ptr_eq(old, texture) && !old.same(texture) {
                    return Err(format!("同一运行时材质 ID 对应不同像素：{id}"));
                }
            } else {
                textures.insert(id.clone(), Arc::clone(texture));
            }
        }
        prepared_parts.push((key, sources, skins, heads, prepared));
    }
    let (runtime, runtime_statistics) =
        runtime_atlas::reconcile(base, textures, previous.map(|old| &old.runtime), cancelled)?;
    statistics.runtime_pages_built = runtime_statistics.built;
    statistics.runtime_pages_reused = runtime_statistics.reused;
    let atlas = Arc::clone(&runtime.atlas);
    let render_bank = bank.with_runtime_atlas(Arc::clone(&atlas))?;
    let mut parts = BTreeMap::new();
    let mut diagnostic_counts = BTreeMap::<String, usize>::new();
    let mut contributions: BTreeMap<Coordinate, Vec<MeshChunk>> = BTreeMap::new();
    let mut frame = Frame {
        snapshot: Arc::clone(&snapshot),
        show_textures,
        show_entities,
        visibility,
        atlas,
        chunks: Default::default(),
        entities: BTreeMap::new(),
        managed_blocks: BTreeMap::new(),
        resource_identity: bank.visibility_id(),
        rendered: 0,
        missing: 0,
        invisible: 0,
        diagnostics: Vec::new(),
        player_names: BTreeMap::new(),
        assembly: None,
    };
    for (key, sources, skins, heads, prepared) in prepared_parts {
        check()?;
        let materials = prepared
            .runtime_textures
            .keys()
            .map(|id| {
                frame
                    .atlas
                    .entries
                    .get(id)
                    .cloned()
                    .map(|uv| (id.clone(), uv))
                    .ok_or_else(|| format!("活动材质未进入联合图集：{id}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let old = previous.and_then(|old| old.parts.get(&key)).filter(|old| {
            Arc::ptr_eq(&old.prepared, &prepared)
                && old.materials == materials
                && old
                    .geometry
                    .visibility
                    .same_for(visibility, sources.iter().map(MarkerRef::marker))
                && selection_in(&sources, previous.unwrap().selected.as_ref())
                    == selection_in(&sources, selected)
                && previous
                    .unwrap()
                    .snapshot
                    .same_biomes(&snapshot.biomes, &sources)
        });
        let geometry = if let Some(old) = old {
            statistics.reused_geometry += 1;
            Arc::clone(&old.geometry)
        } else {
            statistics.compiled_parts += 1;
            let subset = Arc::new(
                snapshot
                    .subset(sources.clone(), cancelled)
                    .ok_or("实体源段索引构建已取消")?,
            );
            Arc::new(Geometry::from(Frame::compile_prepared(
                subset,
                &render_bank,
                show_textures,
                show_entities,
                visibility,
                cancelled,
                online_skins,
                &prepared,
                Arc::clone(&frame.atlas),
                selected,
            )?))
        };
        for (position, visual) in &geometry.blocks {
            let mut visual = visual.clone();
            if let Some(group) = &mut visual.group {
                *group += frame.entities.get(&visual.chunk).map_or(0, Vec::len);
            }
            frame.managed_blocks.insert(*position, visual);
        }
        for (coordinate, owners) in &geometry.entities {
            frame
                .entities
                .entry(*coordinate)
                .or_default()
                .extend(owners.iter().cloned());
        }
        for chunk in geometry.chunks.iter() {
            contributions
                .entry((chunk.key.0, chunk.key.1))
                .or_default()
                .push(chunk.clone());
        }
        frame.rendered += geometry.rendered;
        frame.missing += geometry.missing;
        frame.invisible += geometry.invisible;
        frame
            .diagnostics
            .extend(geometry.diagnostics.iter().cloned());
        for (diagnostic, count) in &geometry.diagnostic_counts {
            *diagnostic_counts.entry(diagnostic.clone()).or_default() += count;
        }
        frame
            .player_names
            .extend(geometry.names.iter().map(|(k, v)| (k.clone(), v.clone())));
        parts.insert(
            key,
            match old {
                Some(old) => Arc::clone(old),
                None => Arc::new(Part {
                    sources,
                    skins,
                    heads,
                    prepared,
                    materials,
                    geometry,
                }),
            },
        );
    }
    let mut joined = BTreeMap::new();
    let mut chunks = Vec::with_capacity(contributions.len());
    for (coordinate, sources) in contributions {
        check()?;
        let mesh = if let Some(old) = previous
            .and_then(|old| old.joined.get(&coordinate))
            .filter(|old| same_meshes(&old.sources, &sources))
        {
            statistics.reused_meshes += 1;
            old.mesh.clone()
        } else {
            statistics.built_meshes += 1;
            join(&sources, cancelled)?
        };
        chunks.push(mesh.clone());
        joined.insert(coordinate, Joined { sources, mesh });
    }
    frame.chunks = match previous_frame
        .filter(|old| previous.is_some() && same_meshes(&old.chunks, &chunks))
    {
        Some(old) => old.chunks.clone(),
        None => chunks.into(),
    };
    let source_diagnostics = std::mem::take(&mut frame.diagnostics);
    frame.diagnostics = diagnostic_counts
        .into_iter()
        .map(|(diagnostic, count)| format!("{count} 个 {diagnostic}"))
        .collect();
    frame.diagnostics.extend(source_diagnostics);
    frame.assembly = Some(Arc::new(Assembly {
        selected: selected.cloned(),
        settings,
        snapshot,
        parts,
        joined,
        runtime,
        statistics,
    }));
    check()?;
    Ok(frame)
}
