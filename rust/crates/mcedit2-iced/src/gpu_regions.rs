//! 不透明几何按空间区域归并；源整数原点无损打包，编辑与拾取仍使用原始网格。
use super::super::{commands, encoding, material_hints::Hints};
use super::{
    chunk_key,
    opaque::{Event, RegionalEvent},
    world, ChunkKey,
};
use crate::{frustum::MeshBounds, spectator::StreamMesh, world_mesh::WorldQuad};
use std::{collections::BTreeMap, sync::Arc};

#[path = "gpu_greedy.rs"]
mod greedy;
#[path = "gpu_region_plan.rs"]
mod plan;
pub(in crate::gpu_scene) use plan::Plan;
pub(in crate::gpu_scene) type Key = (i32, i32, i32, i32, bool);
// 非精确来源继续按 2×2 区块合批；精确地形由独立的 16³ Section 常驻。
const REGION_SIDE: i32 = 32;
#[derive(Clone)]
pub(in crate::gpu_scene) struct Member {
    key: ChunkKey,
    arrival: usize,
    pub origin: [i32; 3],
    pub source: Arc<Vec<WorldQuad>>,
    geometry: Arc<crate::mesh_geometry::Geometry>,
}
pub(in crate::gpu_scene) struct Definition {
    pub origin: [i32; 3],
    pub members: Vec<Member>,
    translate: bool,
}
impl Definition {
    pub fn keys(&self) -> impl Iterator<Item = ChunkKey> + '_ {
        self.members.iter().map(|member| member.key)
    }
    pub fn uses_only(&self, wanted: &std::collections::HashSet<ChunkKey>) -> bool {
        self.members
            .iter()
            .all(|member| wanted.contains(&member.key))
    }
    pub fn same(&self, other: &Self) -> bool {
        self.translate == other.translate
            && self.origin == other.origin
            && self.members.len() == other.members.len()
            && self
                .members
                .iter()
                .zip(&other.members)
                .all(|(a, b)| a.key == b.key && Arc::ptr_eq(&a.source, &b.source))
    }
}
#[derive(Clone)]
pub(in crate::gpu_scene) struct Resident {
    pub source: Arc<Definition>,
    pub buffers: Arc<world::Buffers>,
    pub bounds: Option<MeshBounds>,
    pub groups: Arc<[usize]>,
    pub y32: bool,
    pub commands: commands::Cache,
    pub offsets: Arc<Offsets>,
    pub hints: Arc<Hints>,
    pub animated: Arc<super::atlas::AnimatedMaterials>,
}
/// 安装时按区域键固定排列，逐帧遍历不再追踪树节点；复用检查仍按键查询。
#[derive(Default)]
pub(in crate::gpu_scene) struct Residents(Vec<(Key, Resident)>);
impl From<BTreeMap<Key, Resident>> for Residents {
    fn from(regions: BTreeMap<Key, Resident>) -> Self {
        Self(regions.into_iter().collect())
    }
}
impl Residents {
    pub fn get(&self, key: &Key) -> Option<&Resident> {
        self.0
            .binary_search_by_key(key, |(key, _)| *key)
            .ok()
            .map(|i| &self.0[i].1)
    }
    pub fn get_mut(&mut self, key: &Key) -> Option<&mut Resident> {
        self.0
            .binary_search_by_key(key, |(key, _)| *key)
            .ok()
            .map(|i| &mut self.0[i].1)
    }
    pub fn values(&self) -> impl Iterator<Item = &Resident> {
        self.0.iter().map(|(_, resident)| resident)
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Resident> {
        self.0.iter_mut().map(|(_, resident)| resident)
    }
    pub fn at_mut(&mut self, index: usize) -> &mut Resident {
        &mut self.0[index].1
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
}
/// 只排序小记录，视锥另存；容量跨帧复用，索引同时确定等深区域的源顺序。
#[derive(Default)]
pub(in crate::gpu_scene) struct DrawList {
    pub entries: Vec<Draw>,
    pub views: Vec<Option<crate::frustum::Frustum>>,
}
pub(in crate::gpu_scene) struct Draw {
    pub depth: f32,
    pub resident: usize,
    pub view: usize,
}
impl DrawList {
    pub fn prepare(
        &mut self,
        residents: &Residents,
        view: Option<&crate::frustum::Frustum>,
        origin: [i32; 3],
        skip: impl Fn([i32; 3]) -> bool,
    ) {
        self.clear();
        for (index, (key, resident)) in residents.0.iter().enumerate() {
            if key.4 && skip(resident.source.origin) {
                continue;
            }
            if resident.buffers.segments.is_empty() {
                continue;
            }
            let delta = super::super::origin_delta(resident.source.origin, origin);
            let local = view.map(|v| v.at_offset([delta[0], delta[1], delta[2], 0.]));
            if local
                .as_ref()
                .zip(resident.bounds.as_ref())
                .is_some_and(|(v, b)| !b.visible_in(v))
            {
                continue;
            }
            let depth = local
                .as_ref()
                .zip(resident.bounds.as_ref())
                .map_or(0.0, |(v, b)| {
                    b.near_depth(&v.camera).max(v.camera.projection[2])
                });
            self.entries.push(Draw {
                depth,
                resident: index,
                view: self.views.len(),
            });
            self.views.push(local);
        }
        self.entries.sort_unstable_by(|a, b| {
            a.depth
                .total_cmp(&b.depth)
                .then_with(|| a.resident.cmp(&b.resident))
        });
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.views.clear();
    }
}
pub(in crate::gpu_scene) struct Offsets {
    buffer: iced::wgpu::Buffer,
    pub binding: iced::wgpu::BindGroup,
    pub origin: [i32; 3],
    integer_offset: u64,
}
pub(super) struct OffsetUpload {
    pub offsets: Arc<Offsets>,
    uploaded: usize,
}
impl OffsetUpload {
    pub fn new(
        origin: [i32; 3],
        old: Option<&Arc<Offsets>>,
        device: &iced::wgpu::Device,
        layout: &iced::wgpu::BindGroupLayout,
    ) -> Result<Self, String> {
        if let Some(old) = old.filter(|old| old.origin == origin) {
            return Ok(Self {
                offsets: Arc::clone(old),
                uploaded: 1,
            });
        }
        let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment).max(1);
        let integer_offset = 16u64.div_ceil(alignment) * alignment;
        let size = integer_offset + 32;
        if size > device.limits().max_buffer_size {
            return Err("区域原点缓冲超过设备限制".into());
        }
        let buffer = device.create_buffer(&iced::wgpu::BufferDescriptor {
            label: Some("MCEdit2 区域精确源原点"),
            size,
            usage: iced::wgpu::BufferUsages::UNIFORM | iced::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let binding = device.create_bind_group(&iced::wgpu::BindGroupDescriptor {
            label: Some("MCEdit2 区域源原点绑定"),
            layout,
            entries: &[
                iced::wgpu::BindGroupEntry {
                    binding: 0,
                    resource: iced::wgpu::BindingResource::Buffer(iced::wgpu::BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: std::num::NonZeroU64::new(16),
                    }),
                },
                iced::wgpu::BindGroupEntry {
                    binding: 9,
                    resource: iced::wgpu::BindingResource::Buffer(iced::wgpu::BufferBinding {
                        buffer: &buffer,
                        offset: integer_offset,
                        size: std::num::NonZeroU64::new(32),
                    }),
                },
            ],
        });
        Ok(Self {
            offsets: Arc::new(Offsets {
                buffer,
                binding,
                origin,
                integer_offset,
            }),
            uploaded: 0,
        })
    }
    pub fn advance(
        &mut self,
        source: &Definition,
        queue: &iced::wgpu::Queue,
        budget: &mut super::Budget,
    ) -> bool {
        if self.uploaded == 1 {
            return true;
        }
        if budget.exhausted() || budget.allowance() < 48 {
            return false;
        }
        let a = source.origin;
        let b = self.offsets.origin;
        let value = [a[0], a[1], a[2], 0, b[0], b[1], b[2], 0];
        let d = super::super::origin_delta(a, b);
        queue.write_buffer(
            &self.offsets.buffer,
            0,
            bytemuck::bytes_of(&[d[0], d[1], d[2], 0.0]),
        );
        queue.write_buffer(
            &self.offsets.buffer,
            self.offsets.integer_offset,
            bytemuck::bytes_of(&value),
        );
        budget.consume(48);
        self.uploaded = 1;
        true
    }
}
impl Resident {
    pub fn reusable(
        &self,
        source: &Definition,
        y32: bool,
        groups: &[usize],
        hints: &Hints,
    ) -> bool {
        self.y32 == y32
            && self.source.same(source)
            && self.hints.compatible(
                hints,
                &self.groups,
                groups,
                self.source
                    .members
                    .iter()
                    .flat_map(|member| member.geometry.used_pages().iter().copied()),
            )
    }
}
fn definitions(
    stream: &StreamMesh,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<(Key, Arc<Definition>)>> {
    let mut roots: BTreeMap<Key, Definition> = BTreeMap::new();
    for (index, part) in stream.chunks.iter().enumerate() {
        if index.is_multiple_of(512) && cancelled() {
            return None;
        }
        let [x, y, z] = part.origin;
        let exact = part.key.2 == 1;
        let side = if exact { 16 } else { REGION_SIDE };
        let key = (x.div_euclid(side), y, z.div_euclid(side), side, exact);
        roots
            .entry(key)
            .or_insert_with(|| Definition {
                origin: [key.0 * side, key.1, key.2 * side],
                members: Vec::new(),
                translate: false,
            })
            .members
            .push(Member {
                key: chunk_key(part),
                arrival: index,
                origin: part.origin,
                source: Arc::clone(part.quads()),
                geometry: Arc::clone(part.geometry()),
            });
    }
    let mut pending = roots
        .into_iter()
        .map(|(key, source)| (key.3, key.4, source))
        .collect::<Vec<_>>();
    let mut result = Vec::new();
    while let Some((side, exact, mut source)) = pending.pop() {
        if cancelled() {
            return None;
        }
        // 无普通不透明面时，整组只由有序 / 透明通道绘制，不创建空区域证明与缓冲。
        if !source
            .members
            .iter()
            .any(|member| member.geometry.has_regular())
        {
            continue;
        }
        let faces = source
            .members
            .iter()
            .fold(0usize, |n, m| n.saturating_add(m.source.len()));
        if side > 16 && faces > greedy::target_faces() {
            let child = side / 2;
            let mut children: BTreeMap<(i32, i32), Definition> = BTreeMap::new();
            for member in source.members {
                let key = (
                    member.origin[0].div_euclid(child),
                    member.origin[2].div_euclid(child),
                );
                children
                    .entry(key)
                    .or_insert_with(|| Definition {
                        origin: [key.0 * child, source.origin[1], key.1 * child],
                        members: Vec::new(),
                        translate: false,
                    })
                    .members
                    .push(member);
            }
            pending.extend(children.into_values().map(|source| (child, exact, source)));
            continue;
        }
        let region = super::super::origin_delta(source.origin, stream.origin);
        source.translate = source.members.iter().all(|member| {
            let offset = super::super::origin_delta(member.origin, stream.origin);
            (0..3).all(|axis| {
                f64::from(region[axis])
                    + (i64::from(member.origin[axis]) - i64::from(source.origin[axis])) as f64
                    == f64::from(offset[axis])
            })
        });
        let key = (
            source.origin[0].div_euclid(side),
            source.origin[1],
            source.origin[2].div_euclid(side),
            side,
            exact,
        );
        result.push((key, Arc::new(source)));
    }
    result.sort_by_key(|(key, source)| (source.members[0].arrival, *key));
    (!cancelled()).then_some(result)
}
#[derive(Clone)]
struct Face {
    member: usize,
    index: usize,
    compact: Option<encoding::CompactQuad>,
    merged_bounds: Option<MeshBounds>,
}
impl Face {
    fn index_count(&self) -> u32 {
        self.compact.as_ref().map_or(6, |q| q.index_count())
    }
}
/// 只接受精确可表示的换基；两种求和拥有同一个数学值，最后一次 f32 舍入与原投影相同。
fn translated(
    quad: &WorldQuad,
    member: &Member,
    source: &Definition,
) -> Option<encoding::CompactQuad> {
    if !source.translate
        || quad.mode & (crate::material_mode::FONT_TEXT | crate::material_mode::SIGN_NEAR_OUTLINE)
            != 0
    {
        return None;
    }
    let mut shifted = *quad;
    for (index, (to, from)) in shifted
        .xyz
        .iter_mut()
        .flatten()
        .zip(quad.xyz.iter().flatten())
        .enumerate()
    {
        let axis = index % 3;
        let delta = i64::from(member.origin[axis]) - i64::from(source.origin[axis]);
        let exact = f64::from(*from) + delta as f64;
        let value = exact as f32;
        if f64::from(value) != exact {
            return None;
        }
        *to = value;
    }
    encoding::CompactQuad::encode(&shifted)
}
fn flush(
    index: usize,
    source: &Definition,
    faces: &mut Vec<Face>,
    compact: bool,
    rebased: bool,
    guard: Option<&greedy::Guard>,
    y32: bool,
    groups: &[usize],
    hints: &Hints,
    sender: &crate::render_wake::Sender<Result<Event, String>>,
    cancelled: &dyn Fn() -> bool,
) -> Result<bool, String> {
    if faces.is_empty() {
        return Ok(true);
    }
    if cancelled() {
        return Ok(false);
    }
    if rebased {
        if let Some(guard) = guard {
            if !greedy::merge(faces, source, hints, guard, y32, cancelled)? {
                return Ok(false);
            }
        }
    }
    faces.sort_by_cached_key(|face| {
        let member = &source.members[face.member];
        let quad = &member.source[face.index];
        (
            world::page_group(quad.atlas_page, groups),
            quad.single_sided(),
            face.index_count(),
            quad.axis_facing(),
            if y32 {
                super::y_bucket(quad, member.origin[1])
            } else {
                0
            },
            face.member,
        )
    });
    let mut encoded_full = Vec::new();
    let mut encoded_compact = Vec::new();
    let mut encoded_translated = Vec::new();
    if rebased {
        encoded_translated.reserve(faces.len());
    } else if compact {
        encoded_compact.reserve(faces.len());
    } else {
        encoded_full.reserve(faces.len());
    }
    let mut spans: Vec<world::WorldSpan> = Vec::new();
    let mut patches: Vec<world::Patch> = Vec::new();
    let mut previous = None;
    let mut bounds: Option<MeshBounds> = None;
    for (position, face) in faces.iter().enumerate() {
        if position.is_multiple_of(1024) && cancelled() {
            return Ok(false);
        }
        let member = &source.members[face.member];
        let quad = &member.source[face.index];
        let dx = i64::from(member.origin[0]) - i64::from(source.origin[0]);
        let dz = i64::from(member.origin[2]) - i64::from(source.origin[2]);
        if !(0..128).contains(&dx)
            || !(0..128).contains(&dz)
            || member.origin[1] != source.origin[1]
        {
            return Err("区域源原点未满足无损分组约束".into());
        }
        let packed_origin = dx as u32 | ((dz as u32) << 7);
        if rebased {
            encoded_translated.push(face.compact.ok_or("精确区域面缺少编码")?);
        } else if compact {
            encoded_compact.push(encoding::RegionalCompact {
                quad: face.compact.ok_or("区域紧凑面缺少已完成的编码")?,
                origin_xz: packed_origin,
            });
        } else {
            encoded_full.push(encoding::RegionalFull {
                quad: hints.encode(*quad),
                origin_xz: packed_origin,
            });
        }
        let page = world::page_group(quad.atlas_page, groups);
        let single_sided = quad.single_sided();
        let delta = super::super::origin_delta(member.origin, source.origin);
        let volume = face.merged_bounds.unwrap_or_else(|| {
            MeshBounds::from_quad(quad)
                .translated(delta)
                .expanded(delta.into_iter().map(f32::abs).fold(0.0, f32::max) * f32::EPSILON * 8.0)
        });
        bounds = Some(bounds.map_or(volume, |b| b.union(volume)));
        let index_count = face.index_count();
        let end = position as u32 + 1;
        if let Some(last) = spans.last_mut().filter(|s| {
            s.page == page && s.single_sided == single_sided && s.index_count == index_count
        }) {
            last.end = end;
        } else {
            spans.push(world::WorldSpan {
                index_count,
                page,
                single_sided,
                depth_equal: false,
                facing: 0,
                start: position as u32,
                end,
            });
        }
        let key = (
            page,
            single_sided,
            face.member,
            if y32 {
                super::y_bucket(quad, member.origin[1])
            } else {
                0
            },
            quad.axis_facing(),
        );
        if previous == Some(key) {
            let last = patches.last_mut().ok_or("区域可见性片身份丢失")?;
            last.end = end;
            last.bounds = last.bounds.union(volume);
        } else {
            patches.push(world::Patch {
                group: Some((face.member, key.3, quad.axis_facing())),
                page,
                start: position as u32,
                end,
                bounds: volume,
                facing: quad.axis_facing(),
            });
            previous = Some(key);
        }
    }
    let encoded = if rebased {
        encoding::Encoded::RegionalTranslated(encoded_translated)
    } else if compact {
        encoding::Encoded::RegionalCompact(encoded_compact)
    } else {
        encoding::Encoded::RegionalFull(encoded_full)
    };
    let Some(segment) = world::PreparedSegment::regional(
        encoded,
        spans,
        patches,
        bounds.ok_or("区域批次没有几何")?,
        cancelled,
    )?
    else {
        return Ok(false);
    };
    faces.clear();
    Ok(sender
        .send(Ok(Event::Regional(RegionalEvent::Batch {
            region: index,
            segment,
        })))
        .is_ok())
}
pub(super) fn prepare(
    index: usize,
    source: &Definition,
    count: usize,
    y32: bool,
    groups: &[usize],
    hints: &Hints,
    atlas: Option<&crate::textures::TextureAtlas>,
    sender: &crate::render_wake::Sender<Result<Event, String>>,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    let mut compact = Vec::new();
    let mut full = Vec::new();
    let mut rebased = Vec::new();
    let guard = if source.translate && hints.merge_enabled() {
        greedy::Guard::build(source, cancelled)?
    } else {
        None
    };
    for (member, part) in source.members.iter().enumerate() {
        for (ordinal, i) in part.geometry.regular().enumerate() {
            let quad = &part.source[i];
            if ordinal.is_multiple_of(1024) && cancelled() {
                return Ok(());
            }
            let local = translated(quad, part, source);
            let encoded = local
                .or_else(|| encoding::CompactQuad::encode(quad))
                .map(|encoded| encoded.hinted(hints.hint(quad)));
            let is_rebased = local.is_some();
            let is_compact = encoded.is_some();
            let stride = if is_rebased {
                std::mem::size_of::<encoding::CompactQuad>()
            } else if is_compact {
                std::mem::size_of::<encoding::RegionalCompact>()
            } else {
                std::mem::size_of::<encoding::RegionalFull>()
            };
            let selected = if is_rebased {
                &mut rebased
            } else if is_compact {
                &mut compact
            } else {
                &mut full
            };
            selected.push(Face {
                member,
                index: i,
                compact: encoded,
                merged_bounds: None,
            });
            let limit = (count / stride).min(u32::MAX as usize);
            if selected.len() == limit
                && !flush(
                    index,
                    source,
                    selected,
                    is_compact,
                    is_rebased,
                    guard.as_ref(),
                    y32,
                    groups,
                    hints,
                    sender,
                    cancelled,
                )?
            {
                return Ok(());
            }
        }
    }
    for (faces, compact, rebased) in [
        (&mut rebased, true, true),
        (&mut compact, true, false),
        (&mut full, false, false),
    ] {
        if !flush(
            index,
            source,
            faces,
            compact,
            rebased,
            guard.as_ref(),
            y32,
            groups,
            hints,
            sender,
            cancelled,
        )? {
            return Ok(());
        }
    }
    if !cancelled() {
        let animated = super::atlas::animated_materials(
            atlas,
            source.members.iter().flat_map(|part| {
                part.geometry.regular().map(|index| {
                    let quad = &part.source[index];
                    (quad.atlas_page, quad.uv)
                })
            }),
            cancelled,
        )?;
        let _ = sender.send(Ok(Event::Regional(RegionalEvent::Complete {
            region: index,
            animated: Arc::new(animated),
        })));
    }
    Ok(())
}
