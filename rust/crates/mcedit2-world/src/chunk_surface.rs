//! 自由视角使用原始区块的全部 Section；不截取相机附近高度，不生成高度图墙。
use crate::{
    block_lookup::ReadOnlyRegionCache,
    editing::Session,
    far_view::FarChunkPatch,
    sections::{Chunk, Section},
    surface::{self, SourceChunkTile},
    surface_visibility::Direction,
    WorldError,
};
use std::collections::BTreeMap;

// 流体角点包含斜邻格；仅读取四个正邻块会在区块角上丢失真实高度。
const NEIGHBOR_CHUNKS: [(i32, i32); 9] = [
    (0, 0),
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (-1, -1),
    (-1, 1),
    (1, -1),
    (1, 1),
];

pub fn inspect_full_chunk_cached(
    session: &Session,
    reader: &mut ReadOnlyRegionCache<'_>,
    cx: i32,
    cz: i32,
) -> Result<FarChunkPatch, WorldError> {
    inspect_full_chunk_cancellable(session, reader, cx, cz, &|| false)
}
pub fn inspect_full_chunk_cancellable(
    session: &Session,
    reader: &mut ReadOnlyRegionCache<'_>,
    cx: i32,
    cz: i32,
    cancelled: &dyn Fn() -> bool,
) -> Result<FarChunkPatch, WorldError> {
    inspect_full_chunk_with_visibility_cancellable(
        session,
        reader,
        cx,
        cz,
        &mut crate::surface_visibility::Native,
        cancelled,
    )
}
pub fn inspect_full_chunk_with_visibility_cancellable(
    session: &Session,
    reader: &mut ReadOnlyRegionCache<'_>,
    cx: i32,
    cz: i32,
    visibility: &mut dyn crate::surface_visibility::Visibility,
    cancelled: &dyn Fn() -> bool,
) -> Result<FarChunkPatch, WorldError> {
    let mut chunks = BTreeMap::new();
    let mut on_disk = false;
    for (dx, dz) in NEIGHBOR_CHUNKS {
        if cancelled() {
            return Err(WorldError::Invalid("区块读取已取消".into()));
        }
        let key = (
            cx.checked_add(dx)
                .ok_or_else(|| WorldError::Invalid("区块 X 溢出".into()))?,
            cz.checked_add(dz)
                .ok_or_else(|| WorldError::Invalid("区块 Z 溢出".into()))?,
        );
        let (chunk, present) = session.visible_chunk_base_shared_cached(reader, key.0, key.1)?;
        if key == (cx, cz) {
            on_disk = present;
        }
        chunks.insert(key, chunk);
    }
    let patch = inspect_sources_inner(
        session,
        &chunks,
        (cx, cz, on_disk),
        Some(&mut *reader),
        visibility,
        cancelled,
    )?;
    Ok(patch)
}
pub fn inspect_full_chunk_parallel(
    session: &Session,
    shared: &crate::block_lookup::SharedRegionBatch,
    reader: &mut ReadOnlyRegionCache<'_>,
    cx: i32,
    cz: i32,
    cancelled: &dyn Fn() -> bool,
) -> Result<FarChunkPatch, WorldError> {
    inspect_full_chunk_parallel_with_visibility(
        session,
        shared,
        reader,
        cx,
        cz,
        &mut crate::surface_visibility::Native,
        cancelled,
    )
}
pub fn inspect_full_chunk_parallel_with_visibility(
    session: &Session,
    shared: &crate::block_lookup::SharedRegionBatch,
    reader: &mut ReadOnlyRegionCache<'_>,
    cx: i32,
    cz: i32,
    visibility: &mut dyn crate::surface_visibility::Visibility,
    cancelled: &dyn Fn() -> bool,
) -> Result<FarChunkPatch, WorldError> {
    let started = std::time::Instant::now();
    let mut chunks = BTreeMap::new();
    let mut on_disk = false;
    for (dx, dz) in NEIGHBOR_CHUNKS {
        if cancelled() {
            return Err(WorldError::Invalid("区块读取已取消".into()));
        }
        let x = cx
            .checked_add(dx)
            .ok_or_else(|| WorldError::Invalid("区块 X 溢出".into()))?;
        let z = cz
            .checked_add(dz)
            .ok_or_else(|| WorldError::Invalid("区块 Z 溢出".into()))?;
        let (chunk, present) = session.visible_chunk_base_parallel(shared, reader, x, z)?;
        if (x, z) == (cx, cz) {
            on_disk = present;
        }
        chunks.insert((x, z), chunk);
    }
    let neighbors = started.elapsed();
    let patch = inspect_sources_inner(
        session,
        &chunks,
        (cx, cz, on_disk),
        Some(&mut *reader),
        visibility,
        cancelled,
    )?;
    log::debug!(
        "[chunk-read] {cx},{cz} neighbors_us={} scan_us={} voxels={}",
        neighbors.as_micros(),
        (started.elapsed() - neighbors).as_micros(),
        patch.voxels.len()
    );
    Ok(patch)
}
fn air(state: &crate::sections::State) -> bool {
    matches!(
        state.name.as_str(),
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" | "numeric:0"
    )
}
#[cfg(test)]
fn inspect_sources<C: std::borrow::Borrow<Chunk>>(
    session: &Session,
    chunks: &BTreeMap<(i32, i32), Option<C>>,
    cx: i32,
    cz: i32,
    source_present: bool,
    reader: Option<&mut ReadOnlyRegionCache<'_>>,
) -> Result<FarChunkPatch, WorldError> {
    inspect_sources_inner(
        session,
        chunks,
        (cx, cz, source_present),
        reader,
        &mut crate::surface_visibility::Native,
        &|| false,
    )
}
#[derive(Clone, Copy)]
struct PaletteCell {
    kind: surface::VoxelMaterial,
    air: bool,
    fixed: Option<crate::surface_visibility::CellInfo>,
}
fn prepare_palette(
    section: &Section,
    visibility: &mut dyn crate::surface_visibility::Visibility,
    allow_fixed: bool,
) -> Vec<PaletteCell> {
    section
        .palette
        .iter()
        .map(|state| {
            let kind = surface::material(state);
            let air = air(state);
            PaletteCell {
                kind,
                air,
                fixed: if allow_fixed && !air {
                    visibility.fixed_cell(state, kind)
                } else {
                    None
                },
            }
        })
        .collect()
}
fn inspect_sources_inner<C: std::borrow::Borrow<Chunk>>(
    session: &Session,
    chunks: &BTreeMap<(i32, i32), Option<C>>,
    location: (i32, i32, bool),
    mut reader: Option<&mut ReadOnlyRegionCache<'_>>,
    visibility: &mut dyn crate::surface_visibility::Visibility,
    cancelled: &dyn Fn() -> bool,
) -> Result<FarChunkPatch, WorldError> {
    let (cx, cz, source_present) = location;
    let x0 = cx
        .checked_mul(16)
        .filter(|x| (-30_000_000..=29_999_984).contains(x))
        .ok_or_else(|| WorldError::Invalid("区块 X 超出世界边界".into()))?;
    let z0 = cz
        .checked_mul(16)
        .filter(|z| (-30_000_000..=29_999_984).contains(z))
        .ok_or_else(|| WorldError::Invalid("区块 Z 超出世界边界".into()))?;
    let mut tile = SourceChunkTile {
        chunk_x: cx,
        chunk_z: cz,
        source_present: false,
        known_columns: 0,
        unknown_columns: 256,
        exposed_voxels: 0,
    };
    let Some(chunk) = chunks
        .get(&(cx, cz))
        .and_then(Option::as_ref)
        .map(<C as std::borrow::Borrow<Chunk>>::borrow)
    else {
        return Ok(FarChunkPatch {
            biome_neighbors: Default::default(),
            biomes: None,
            tile,
            voxels: Vec::new(),
            sections: None,
        });
    };
    let (biome, be) = if let Some(reader) = reader.as_deref_mut() {
        surface::staged_metadata_cached(session, reader, cx, cz)?
    } else {
        surface::staged_metadata(session, cx, cz)?
    };
    let biome = biome.as_ref().unwrap_or(chunk);
    let be = be.as_ref().unwrap_or(chunk);
    tile.source_present = source_present;
    if !chunk.sections.is_empty() {
        tile.known_columns = 256;
        tile.unknown_columns = 0;
    }
    let mut voxels = Vec::new();
    let mut local_palette = surface::VoxelPalette::default();
    let voxel_palette = match reader.as_deref_mut() {
        Some(reader) => reader.display_palette(),
        None => &mut local_palette,
    };
    let record_sections = visibility.track_sections();
    let mut section_visibility = Vec::new();
    let has_overlay = session.has_block_overlay(cx, cz);
    let neighbor_columns = Direction::ALL.map(|direction| {
        let [dx, _, dz] = direction.offset();
        if dx == 0 && dz == 0 {
            return (Some(chunk), has_overlay);
        }
        let column = chunks
            .get(&(cx + dx, cz + dz))
            .and_then(Option::as_ref)
            .map(<C as std::borrow::Borrow<Chunk>>::borrow);
        (column, session.has_block_overlay(cx + dx, cz + dz))
    });
    for section in &chunk.sections {
        if section.indices.len() != 4096 {
            return Err(WorldError::Invalid("源 Section 格数不是 4096".into()));
        }
        let mut opaque = [0_u64; 64];
        if !has_overlay
            && section.palette.len() == 1
            && air(&section.palette[0])
            && section.indices.iter().all(|&index| index == 0)
        {
            if record_sections {
                section_visibility.push(crate::section_visibility::Section::from_opaque(
                    section.y, opaque,
                ));
            }
            continue;
        }
        let neighbor_sections: [Option<&Section>; 6] = std::array::from_fn(|index| {
            let column = neighbor_columns[index].0?;
            let dy = Direction::ALL[index].offset()[1];
            let y = i8::try_from(i32::from(section.y) + dy).ok()?;
            let index = column
                .sections
                .binary_search_by_key(&y, |section| section.y)
                .ok()?;
            Some(&column.sections[index])
        });
        let palette_cells = prepare_palette(section, visibility, !has_overlay);
        let mut palette_fluids = vec![None; section.palette.len()];
        let neighbor_palettes: [Option<Vec<PaletteCell>>; 6] = std::array::from_fn(|side| {
            neighbor_sections[side]
                .map(|section| prepare_palette(section, visibility, !neighbor_columns[side].1))
        });
        // 加权变体也只在真实位置求值一次，同段邻格直接复用；不把随机模型当作固定模型。
        let prepare_cells = visibility.prepare_cells();
        let mut cells = Vec::new();
        if prepare_cells {
            cells.reserve_exact(4096);
            for (index, &palette) in section.indices.iter().enumerate() {
                if index % 1024 == 0 && cancelled() {
                    return Err(WorldError::Invalid("区块扫描已取消".into()));
                }
                let original = section
                    .palette
                    .get(palette as usize)
                    .ok_or_else(|| WorldError::Invalid("源 Section 调色板索引越界".into()))?;
                let prepared = palette_cells[palette as usize];
                let position = (
                    x0 + (index % 16) as i32,
                    i32::from(section.y) * 16 + (index / 256) as i32,
                    z0 + ((index / 16) % 16) as i32,
                );
                let state = if has_overlay {
                    session.effective_state_ref(position, original)
                } else {
                    original
                };
                let kind = if has_overlay {
                    surface::material(state)
                } else {
                    prepared.kind
                };
                let info = if air(state) {
                    None
                } else if !has_overlay && prepared.fixed.is_some() {
                    prepared.fixed
                } else {
                    visibility.evaluate_cell(crate::surface_visibility::Cell {
                        state,
                        kind,
                        position,
                    })
                };
                cells.push(info);
            }
        }
        for (index, palette) in section.indices.iter().enumerate() {
            if index % 1024 == 0 && cancelled() {
                return Err(WorldError::Invalid("区块扫描已取消".into()));
            }
            let original = section
                .palette
                .get(*palette as usize)
                .ok_or_else(|| WorldError::Invalid("源 Section 调色板索引越界".into()))?;
            let lx = (index % 16) as i32;
            let lz = ((index / 16) % 16) as i32;
            let ly = (index / 256) as i32;
            let (x, y, z) = (x0 + lx, i32::from(section.y) * 16 + ly, z0 + lz);
            let state = if has_overlay {
                session.effective_state_ref((x, y, z), original)
            } else {
                original
            };
            let prepared = palette_cells[*palette as usize];
            if if has_overlay {
                air(state)
            } else {
                prepared.air
            } {
                continue;
            }
            let kind = if has_overlay {
                surface::material(state)
            } else {
                prepared.kind
            };
            let fixed = cells.get(index).copied().flatten().or(if has_overlay {
                None
            } else {
                prepared.fixed
            });
            let mut faces = 0;
            let cell = crate::surface_visibility::Cell {
                state,
                kind,
                position: (x, y, z),
            };
            if record_sections
                && fixed.map_or_else(
                    || visibility.opaque_cell(cell),
                    |fixed| fixed.opaque == surface::ALL_FACES,
                )
            {
                opaque[index / 64] |= 1 << (index % 64);
            }
            let has_fluid = if !has_overlay {
                if let Some(cached) = palette_fluids[*palette as usize] {
                    cached
                } else {
                    let fixed = visibility.fixed_fluid(state, kind)?;
                    palette_fluids[*palette as usize] = fixed;
                    match fixed {
                        Some(fluid) => fluid,
                        None => matches!(
                            visibility.fluid_sample(cell)?,
                            crate::fluid_surface::Sample::Fluid(..)
                        ),
                    }
                }
            } else {
                matches!(
                    visibility.fluid_sample(cell)?,
                    crate::fluid_surface::Sample::Fluid(..)
                )
            };
            // 段内六邻格已在真实坐标预计算。封闭的普通格无需再建立六次状态 / 模型查询。
            // 液体、内部几何、边界及未知格继续走完整逻辑，不以方块名称猜测遮挡。
            if !has_fluid
                && fixed.is_some_and(|info| {
                    !info.unconditional
                        && info.ordinary_faces
                        && (1..15).contains(&lx)
                        && (1..15).contains(&ly)
                        && (1..15).contains(&lz)
                        && [
                            (index + 256, surface::FACE_BOTTOM),
                            (index - 256, surface::FACE_TOP),
                            (index + 1, surface::FACE_NEG_X),
                            (index - 1, surface::FACE_POS_X),
                            (index + 16, surface::FACE_NEG_Z),
                            (index - 16, surface::FACE_POS_Z),
                        ]
                        .into_iter()
                        .all(|(i, flag)| {
                            cells
                                .get(i)
                                .copied()
                                .flatten()
                                .is_some_and(|n| n.opaque & flag != 0)
                        })
                })
            {
                continue;
            }
            for (side, direction) in Direction::ALL.into_iter().enumerate() {
                let [dx, dy, dz] = direction.offset();
                let flag = direction.flag();
                let (nx, ny, nz) = (lx + dx, ly + dy, lz + dz);
                let horizontal_edge = !(0..16).contains(&nx) || !(0..16).contains(&nz);
                // 未知水平邻块不代表空气：不生成贯穿全高度的临时边界墙。
                if horizontal_edge && neighbor_columns[side].0.is_none() {
                    continue;
                }
                let neighbor_section = if horizontal_edge || !(0..16).contains(&ny) {
                    neighbor_sections[side]
                } else {
                    Some(section)
                };
                let position = (x + dx, y + dy, z + dz);
                let overlay = if horizontal_edge {
                    neighbor_columns[side].1
                } else {
                    has_overlay
                };
                let neighbor = neighbor_section.and_then(|near| {
                    let index =
                        (nx & 15) as usize + 16 * (nz & 15) as usize + 256 * (ny & 15) as usize;
                    let palette = *near.indices.get(index)? as usize;
                    let original = near.palette.get(palette)?;
                    let prepared = if std::ptr::eq(near, section) {
                        palette_cells.get(palette)?
                    } else {
                        neighbor_palettes[side].as_ref()?.get(palette)?
                    };
                    let state = if overlay {
                        session.effective_state_ref(position, original)
                    } else {
                        original
                    };
                    let kind = if overlay {
                        surface::material(state)
                    } else {
                        prepared.kind
                    };
                    let cell = crate::surface_visibility::Cell {
                        state,
                        kind,
                        position,
                    };
                    let info = if std::ptr::eq(near, section) {
                        cells.get(index).copied().flatten()
                    } else if !overlay && prepared.fixed.is_some() {
                        prepared.fixed
                    } else if prepare_cells && !air(state) {
                        visibility.evaluate_cell(cell)
                    } else {
                        None
                    };
                    Some((cell, info))
                });
                // 只有实际模型证明隐藏时才省去逐面查询；液体与未知面仍沿原规则。
                let hidden = fixed.is_some_and(|source| {
                    !source.visible
                        || (source.ordinary_faces
                            && neighbor.is_some_and(|(_, neighbor)| {
                                neighbor
                                    .is_some_and(|n| n.opaque & direction.opposite().flag() != 0)
                            }))
                });
                if !hidden && visibility.exposed(cell, direction, neighbor.map(|(cell, _)| cell)) {
                    faces |= flag;
                }
            }
            let unconditional = fixed.map_or_else(
                || visibility.unconditional(cell),
                |fixed| fixed.unconditional,
            );
            if faces == 0 && !unconditional && kind == surface::VoxelMaterial::Liquid {
                continue;
            }
            if faces == 0 && !unconditional && !has_fluid {
                continue;
            }
            let fluid = if has_fluid {
                let available =
                    crate::surface_visibility::available_fluid_faces((x, y, z), None, |cx, cz| {
                        chunks.get(&(cx, cz)).and_then(Option::as_ref).is_some()
                    });
                crate::surface_visibility::fluid_surface(
                    visibility,
                    cell,
                    available,
                    None,
                    &mut |(nx, ny, nz)| {
                        chunks
                            .get(&(nx.div_euclid(16), nz.div_euclid(16)))
                            .and_then(Option::as_ref)
                            .map(<C as std::borrow::Borrow<Chunk>>::borrow)
                            .and_then(|chunk| {
                                crate::surface_visibility::fluid_state(
                                    chunk,
                                    nx.rem_euclid(16) as u8,
                                    ny,
                                    nz.rem_euclid(16) as u8,
                                )
                            })
                            .map(|state| {
                                let state = session.effective_state_ref((nx, ny, nz), state);
                                crate::surface_visibility::Cell {
                                    state,
                                    kind: surface::material(state),
                                    position: (nx, ny, nz),
                                }
                            })
                    },
                )?
            } else {
                None
            };
            if fluid.is_some_and(|fluid| fluid.faces() == 0)
                && (kind == surface::VoxelMaterial::Liquid || (faces == 0 && !unconditional))
            {
                continue;
            }
            voxels.push({
                let data = crate::surface::VoxelData {
                    state: voxel_palette.state(state),
                    biome: voxel_palette.biome(biome.biome((x - x0) as u8, y, (z - z0) as u8)),
                    original_block_entity: be.has_block_entity(x, y, z),
                    faces,
                    kind,
                    staged: session.is_staged_block((x, y, z))
                        || session.creation_origin(cx, cz, y),
                    span: 1,
                    fluid: voxel_palette.fluid(fluid),
                    light: Default::default(),
                };
                voxel_palette.voxel([x, y, z], data)
            });
        }
        if record_sections {
            section_visibility.push(crate::section_visibility::Section::from_opaque(
                section.y, opaque,
            ));
        }
    }
    tile.exposed_voxels = voxels.len();
    voxels.shrink_to_fit();
    // 九宫格已经为剔面和液体读取；只带出群系调色板，不重新读盘或保留方块 NBT。
    let mut biome_neighbors = Vec::new();
    for (&coordinate, source) in chunks {
        if coordinate == (cx, cz) {
            continue;
        }
        let Some(source) = source
            .as_ref()
            .map(<C as std::borrow::Borrow<Chunk>>::borrow)
        else {
            continue;
        };
        if cancelled() {
            return Err(WorldError::Invalid("区块读取已取消".into()));
        }
        let (staged, _) = if let Some(reader) = reader.as_deref_mut() {
            surface::staged_metadata_cached(session, reader, coordinate.0, coordinate.1)?
        } else {
            surface::staged_metadata(session, coordinate.0, coordinate.1)?
        };
        biome_neighbors.push((
            coordinate,
            crate::biome_source::Column::from_chunk(staged.as_ref().unwrap_or(source))?,
        ));
    }
    Ok(FarChunkPatch {
        biome_neighbors: biome_neighbors.into(),
        biomes: Some(crate::biome_source::Column::from_chunk(biome)?),
        tile,
        voxels,
        sections: record_sections.then(|| section_visibility.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sections::{Layout, Section, State};
    fn chunk(cx: i32, positions: &[(usize, i8)]) -> Chunk {
        let mut sections = BTreeMap::new();
        for &(index, y) in positions {
            let section = sections.entry(y).or_insert_with(|| Section {
                y,
                palette: vec![
                    State {
                        name: "minecraft:air".into(),
                        properties: vec![],
                    },
                    State {
                        name: "minecraft:stone".into(),
                        properties: vec![],
                    },
                ],
                indices: vec![0; 4096],
                biomes: Some(crate::sections::BiomePalette {
                    names: vec!["minecraft:plains".into()],
                    indices: vec![0; 64],
                }),
            });
            section.indices[index] = 1;
        }
        Chunk {
            data_version: Some(3955),
            x: Some(cx),
            z: Some(0),
            layout: Layout::Modern118,
            numeric_biomes: None,
            sections: sections.into_values().collect(),
            block_entities: Default::default(),
            light: Default::default(),
        }
    }
    fn session() -> Session {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../test_files/AnvilWorld_1.8");
        let world = crate::inspect_world(&root).unwrap();
        let dim = world.dimensions[0].name.clone();
        Session::new(world, dim).unwrap()
    }

    #[test]
    fn all_source_sections_and_verified_horizontal_neighbors_are_used() {
        let sources = BTreeMap::from([
            ((0, 0), Some(chunk(0, &[(15, -4), (3, 20)]))),
            ((1, 0), Some(chunk(1, &[(0, -4)]))),
        ]);
        let result = inspect_sources(&session(), &sources, 0, 0, true, None).unwrap();
        assert_eq!(result.voxels.len(), 2);
        assert_eq!(result.voxels[0].y, -64);
        assert_eq!(result.voxels[1].y, 320);
        assert_eq!(result.voxels[0].faces & surface::FACE_POS_X, 0);
        assert_ne!(result.voxels[0].faces & surface::FACE_BOTTOM, 0);
        assert_eq!(
            result.voxels[1].faces,
            surface::ALL_FACES & !surface::FACE_NEG_Z
        );
        assert!(result.voxels.iter().all(|v| v.span == 1));
        assert!(std::sync::Arc::ptr_eq(
            &result.voxels[0].state,
            &result.voxels[1].state
        ));
        assert_eq!(
            result.voxels[0].biome.as_ref().unwrap().allocation_id(),
            result.voxels[1].biome.as_ref().unwrap().allocation_id()
        );
        let mut changed = result.clone();
        std::sync::Arc::make_mut(&mut changed.voxels[0].state)
            .properties
            .push(("test".into(), "changed".into()));
        assert!(result.voxels.iter().all(|v| v.state.properties.is_empty()));
        assert!(changed.voxels[1].state.properties.is_empty());
        let reread = inspect_sources(&session(), &sources, 0, 0, true, None).unwrap();
        assert_eq!(result.voxels, reread.voxels);
        assert_eq!(
            result
                .biome_neighbors
                .iter()
                .map(|(p, _)| *p)
                .collect::<Vec<_>>(),
            vec![(1, 0)]
        );
        let mut colors = crate::biome_source::Region::default();
        colors.insert(0, 0, result.biomes.clone().unwrap());
        for (coordinate, column) in result.biome_neighbors.iter() {
            colors.insert(coordinate.0, coordinate.1, column.clone());
        }
        let arrived = inspect_sources(&session(), &sources, 1, 0, true, None).unwrap();
        let mut complete = colors.clone();
        complete.insert(1, 0, arrived.biomes.unwrap());
        complete.reuse_columns(&colors);
        assert!(
            colors.same_neighborhood(&complete, 0, 0),
            "实际邻列随后到达不能改变已有颜色来源"
        );
        let mut changed_biomes = sources.clone();
        changed_biomes
            .get_mut(&(1, 0))
            .unwrap()
            .as_mut()
            .unwrap()
            .sections[0]
            .biomes
            .as_mut()
            .unwrap()
            .names[0] = "minecraft:forest".into();
        let edited = inspect_sources(&session(), &changed_biomes, 1, 0, true, None).unwrap();
        complete.insert(1, 0, edited.biomes.unwrap());
        complete.reuse_columns(&colors);
        assert!(
            !colors.same_neighborhood(&complete, 0, 0),
            "真实群系变化仍须使颜色缓存失效"
        );

        let stacked = BTreeMap::from([((0, 0), Some(chunk(0, &[(3841, -4), (1, -3)])))]);
        let stacked = inspect_sources(&session(), &stacked, 0, 0, true, None).unwrap();
        assert_eq!(stacked.voxels.len(), 2);
        assert_eq!(stacked.voxels[0].faces & surface::FACE_TOP, 0);
        assert_eq!(stacked.voxels[1].faces & surface::FACE_BOTTOM, 0);
    }

    #[test]
    fn missing_source_chunk_is_reported_without_synthetic_terrain() {
        let result =
            inspect_sources::<Chunk>(&session(), &BTreeMap::new(), 0, 0, false, None).unwrap();
        assert!(result.voxels.is_empty());
        assert!(!result.tile.source_present);
        assert!(result.biome_neighbors.is_empty());
        assert_eq!(result.tile.unknown_columns, 256);
    }
}
