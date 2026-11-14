//! 紧凑世界面的分段上传与跨区块透明排序，所有分段由实际设备限制决定。
#[cfg(test)]
use crate::spectator::StreamMesh;
use crate::{spectator::CameraUniform, world_mesh::WorldQuad};
use iced::wgpu;
#[cfg(test)]
use std::sync::Arc;

/// 四个唯一顶点构成两个三角形，允许 GPU 复用重复角点的着色结果。
pub(super) const QUAD_INDICES: [u16; 6] = [0, 1, 2, 0, 2, 3];
/// 前六项也用于普通四角面；长扇按跨度选择共享前缀，无须每份几何重建索引。
pub(super) fn shared_indices() -> Vec<u16> {
    (1..=super::encoding::MAX_FAN_EDGES)
        .flat_map(|i| [0, i as u16, (i + 1) as u16])
        .collect()
}

/// 透视只依赖眼点；正交只依赖观察方向，平移 / 缩放不改变其相对深度。
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub(super) enum SortKey {
    #[default]
    Source,
    Perspective([f32; 3]),
    Orthographic([f32; 3]),
}
impl SortKey {
    /// 半格内的眼点移动沿用同一顺序，避免每帧重新排序和上传整份透明索引。
    pub fn reference(mut camera: CameraUniform) -> CameraUniform {
        if camera.position[3] != 1.0 {
            for axis in &mut camera.position[..3] {
                *axis = axis.round();
            }
        }
        camera
    }
    pub fn camera(camera: Option<&CameraUniform>) -> Self {
        match camera {
            Some(camera) if camera.position[3] == 1.0 => {
                Self::Orthographic(std::array::from_fn(|i| camera.forward[i]))
            }
            Some(camera) => Self::Perspective(std::array::from_fn(|i| camera.position[i])),
            None => Self::Source,
        }
    }
}
/// 与 1.21.1 VertexSorting.byDistance 的距离平方降序相同；正交采用线性视深。
pub(super) fn sort_distance(center: [f32; 3], camera: &CameraUniform) -> f32 {
    if camera.position[3] == 1.0 {
        (0..3)
            .map(|i| (center[i] - camera.position[i]) * camera.forward[i])
            .sum()
    } else {
        (0..3)
            .map(|i| (center[i] - camera.position[i]).powi(2))
            .sum()
    }
}

pub(super) fn segment_faces(max_buffer_size: u64) -> usize {
    (max_buffer_size / std::mem::size_of::<WorldQuad>() as u64).min(u32::MAX as u64) as usize
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WorldSpan {
    pub page: usize,
    pub single_sided: bool,
    pub depth_equal: bool,
    pub facing: u8,
    pub index_count: u32,
    pub start: u32,
    pub end: u32,
}
/// 原始页号仍保存在每个面内；元数据只按实际 GPU 数组绑定归并。
pub(super) fn page_group(page: u32, groups: &[usize]) -> usize {
    if groups.is_empty() {
        page as usize
    } else {
        groups[page as usize]
    }
}
fn append_span(spans: &mut Vec<WorldSpan>, quad: &WorldQuad, index: u32, groups: &[usize]) {
    let facing = quad.axis_facing();
    let page = page_group(quad.atlas_page, groups);
    if let Some(last) = spans.last_mut() {
        if last.page == page
            && last.single_sided == quad.single_sided()
            && last.facing == facing
            && last.depth_equal == (quad.mode & crate::material_mode::DEPTH_EQUAL != 0)
        {
            last.end = index + 1;
            return;
        }
    }
    spans.push(WorldSpan {
        index_count: QUAD_INDICES.len() as u32,
        page,
        single_sided: quad.single_sided(),
        depth_equal: quad.mode & crate::material_mode::DEPTH_EQUAL != 0,
        facing,
        start: index,
        end: index + 1,
    });
}
#[cfg(test)]
fn page_spans(quads: &[WorldQuad], groups: &[usize]) -> Vec<WorldSpan> {
    let mut spans = Vec::new();
    for (index, quad) in quads.iter().enumerate() {
        append_span(&mut spans, quad, index as u32, groups);
    }
    spans
}
#[derive(Clone)]
pub(super) struct Segment {
    pub buffer: wgpu::Buffer,
    pub compact: bool,
    pub regional: bool,
    pub translated: bool,
    #[cfg(test)]
    pub hinted_faces: usize,
    spans: std::sync::Arc<[WorldSpan]>,
    patches: std::sync::Arc<[Patch]>,
    visibility: std::sync::Arc<Visibility>,
}
/// 同一来源 / 高度 / 朝向的空间判断只做一次，材质和拓扑跨度共享保守包围体。
struct Visibility {
    index: std::sync::Arc<super::visibility::Index>,
    bounds: std::sync::Arc<[crate::frustum::MeshBounds]>,
    groups: std::sync::Arc<[usize]>,
    facing: crate::frustum::FacingIndex,
}
impl Visibility {
    fn build(patches: &[Patch], cancelled: &dyn Fn() -> bool) -> Result<Option<Self>, String> {
        let mut keys: std::collections::BTreeMap<(usize, i64, u8), usize> =
            std::collections::BTreeMap::new();
        let mut bounds: Vec<crate::frustum::MeshBounds> = Vec::new();
        let mut groups = Vec::with_capacity(patches.len());
        for (i, patch) in patches.iter().enumerate() {
            if i.is_multiple_of(1024) && cancelled() {
                return Ok(None);
            }
            let group = patch.group.and_then(|key| keys.get(&key).copied());
            let group = if let Some(group) = group {
                bounds[group] = bounds[group].union(patch.bounds);
                group
            } else {
                let group = bounds.len();
                bounds.push(patch.bounds);
                if let Some(key) = patch.group {
                    keys.insert(key, group);
                }
                group
            };
            groups.push(group);
        }
        let Some(index) =
            super::visibility::Index::build(bounds.len(), |i| bounds[i], false, cancelled)?
        else {
            return Ok(None);
        };
        let Some(facing) = crate::frustum::FacingIndex::build(
            patches.iter().map(|patch| (&patch.bounds, patch.facing)),
            cancelled,
        )?
        else {
            return Ok(None);
        };
        Ok(Some(Self {
            index,
            bounds: bounds.into(),
            groups: groups.into(),
            facing,
        }))
    }
}
#[derive(Clone)]
pub(super) struct Patch {
    pub group: Option<(usize, i64, u8)>,
    pub page: usize,
    pub start: u32,
    pub end: u32,
    pub bounds: crate::frustum::MeshBounds,
    pub facing: u8,
}
impl Segment {
    pub fn spans(&self) -> &[WorldSpan] {
        &self.spans
    }
    #[cfg(test)]
    pub fn patches(&self) -> &[Patch] {
        &self.patches
    }
    pub fn selection(
        &self,
        view: Option<&crate::frustum::Frustum>,
    ) -> Option<super::visibility::Stamp> {
        let facing = match view {
            Some(view) => Some(self.visibility.facing.key(view)?),
            None => None,
        };
        self.visibility.index.stamp(
            view,
            self.visibility.bounds.len(),
            |i| self.visibility.bounds[i],
            facing,
        )
    }
    #[cfg(test)]
    pub fn visibility_group_count(&self) -> usize {
        self.visibility.bounds.len()
    }
    pub fn visibility_mask(
        &self,
        view: Option<&crate::frustum::Frustum>,
    ) -> Option<std::sync::Arc<super::visibility::Mask>> {
        self.visibility
            .index
            .visible(view, self.visibility.bounds.len(), |i| {
                self.visibility.bounds[i]
            })
    }
    pub fn visible_ranges_cached<'a>(
        &'a self,
        span: &'a WorldSpan,
        view: Option<&'a crate::frustum::Frustum>,
        mask: Option<std::sync::Arc<super::visibility::Mask>>,
    ) -> VisibleRanges<'a> {
        let mut ranges = visible_patch_ranges(&self.patches, span, view);
        ranges.mask = mask;
        ranges.groups = Some(&self.visibility.groups);
        ranges
    }
    #[cfg(test)]
    pub fn visible_ranges<'a>(
        &'a self,
        span: &'a WorldSpan,
        view: Option<&'a crate::frustum::Frustum>,
    ) -> VisibleRanges<'a> {
        self.visible_ranges_cached(span, view, self.visibility_mask(view))
    }
}
pub(super) struct VisibleRanges<'a> {
    patches: &'a [Patch],
    span: &'a WorldSpan,
    view: Option<&'a crate::frustum::Frustum>,
    cursor: u32,
    patch_index: usize,
    mask: Option<std::sync::Arc<super::visibility::Mask>>,
    groups: Option<&'a [usize]>,
}
fn visible_patch_ranges<'a>(
    patches: &'a [Patch],
    span: &'a WorldSpan,
    view: Option<&'a crate::frustum::Frustum>,
) -> VisibleRanges<'a> {
    // 上传阶段按实例范围递增构造高度片，定位本跨度即可，不重复扫描其他材质/朝向片。
    let begin = patches.partition_point(|patch| patch.end <= span.start);
    VisibleRanges {
        patches: &patches[begin..],
        span,
        view,
        cursor: span.start,
        patch_index: begin,
        mask: None,
        groups: None,
    }
}
impl Iterator for VisibleRanges<'_> {
    type Item = (u32, u32);
    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.span.end {
            return None;
        }
        if let Some(view) = self.view {
            while let Some((patch, remaining)) = self.patches.split_first() {
                let index = self.patch_index;
                self.patch_index += 1;
                self.patches = remaining;
                if patch.start >= self.span.end {
                    self.patches = &[];
                    break;
                }
                if patch.page != self.span.page
                    || patch.end <= self.cursor
                    || (view.facing_visible(&patch.bounds, patch.facing)
                        && self.mask.as_ref().map_or_else(
                            || patch.bounds.visible_in(view),
                            |mask| mask.contains(self.groups.map_or(index, |groups| groups[index])),
                        ))
                {
                    continue;
                }
                // 只减去有完整几何证明的不可见片；未标记区间仍提交。
                // 相邻可见片自然合为一段，但绝不跨越已证明不可见的中间片。
                let start = self.cursor;
                let hidden_start = patch.start.max(start);
                self.cursor = patch.end.min(self.span.end);
                if hidden_start > start {
                    return Some((start, hidden_start));
                }
            }
        }
        let start = self.cursor;
        self.cursor = self.span.end;
        (start < self.cursor).then_some((start, self.cursor))
    }
}
#[derive(Default, Clone)]
pub(super) struct Buffers {
    pub segments: Vec<Segment>,
}
impl Buffers {
    /// 后台已完成几何、布局及可见性准备；事件线程只创建缓冲和提交字节。
    pub fn append_prepared(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        prepared: PreparedSegment,
    ) -> Result<(), String> {
        let bytes = prepared.encoding.bytes();
        if bytes.is_empty() || bytes.len() as u64 > device.limits().max_buffer_size {
            return Err("后台面批次为空或超过实际设备缓冲限制".into());
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("MCEdit2 无损编码源面实例缓冲"),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytes);
        self.segments.push(Segment {
            buffer,
            compact: prepared.encoding.compact(),
            regional: prepared.encoding.regional(),
            translated: prepared.encoding.translated(),
            #[cfg(test)]
            hinted_faces: prepared.hinted_faces,
            spans: prepared.spans.into(),
            patches: prepared.patches.into(),
            visibility: std::sync::Arc::new(prepared.visibility),
        });
        Ok(())
    }
}

pub(super) struct PreparedSegment {
    encoding: super::encoding::Encoded,
    spans: Vec<WorldSpan>,
    patches: Vec<Patch>,
    visibility: Visibility,
    pub bounds: crate::frustum::MeshBounds,
    pub faces: usize,
    #[cfg(test)]
    hinted_faces: usize,
}
impl PreparedSegment {
    pub fn bytes(&self) -> usize {
        self.encoding.bytes().len()
    }
    pub fn compact(&self) -> bool {
        self.encoding.compact()
    }
    /// 区域批次的跨度按材质归并，高度片仍分别保留原区块与朝向证明。
    pub fn regional(
        encoding: super::encoding::Encoded,
        spans: Vec<WorldSpan>,
        patches: Vec<Patch>,
        bounds: crate::frustum::MeshBounds,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Self>, String> {
        let faces = encoding.len();
        if faces == 0 || faces > u32::MAX as usize || !encoding.regional() {
            return Err("区域面批次编码或实例数量无效".into());
        }
        let Some(visibility) = Visibility::build(&patches, cancelled)? else {
            return Ok(None);
        };
        #[cfg(test)]
        let hinted_faces = encoding.hinted_count();
        Ok(Some(Self {
            #[cfg(test)]
            hinted_faces,
            encoding,
            spans,
            patches,
            visibility,
            bounds,
            faces,
        }))
    }
    /// 分类阶段已经完成无损编码；按轻量索引读取原面，一次生成范围和高度片。
    pub fn prepare(
        source: &[WorldQuad],
        indices: &[usize],
        encoding: super::encoding::Encoded,
        bucket_origin: Option<i32>,
        groups: &[usize],
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Self>, String> {
        let count = encoding.len();
        if count == 0 || count != indices.len() || count > u32::MAX as usize {
            return Err("后台面编码与源索引数量不一致或超出设备实例范围".into());
        }
        let mut bounds: Option<crate::frustum::MeshBounds> = None;
        let mut spans = Vec::new();
        let mut patches: Vec<Patch> = Vec::new();
        let mut previous_patch = None;
        for (position, &index) in indices.iter().enumerate() {
            if position.is_multiple_of(1024) && cancelled() {
                return Ok(None);
            }
            let quad = source.get(index).ok_or("后台不透明面源索引越界")?;
            let volume = crate::frustum::MeshBounds::from_quad(quad);
            bounds = Some(bounds.map_or(volume, |old| old.union(volume)));
            append_span(&mut spans, quad, position as u32, groups);
            if let Some(origin) = bucket_origin {
                let page = page_group(quad.atlas_page, groups);
                let key = (
                    page,
                    quad.single_sided(),
                    quad.axis_facing(),
                    super::upload::y_bucket(quad, origin),
                );
                if previous_patch == Some(key) {
                    let patch = patches.last_mut().expect("高度片键对应已生成的范围");
                    patch.bounds = patch.bounds.union(volume);
                    patch.end = position as u32 + 1;
                } else {
                    patches.push(Patch {
                        group: Some((0, key.3, quad.axis_facing())),
                        page,
                        start: position as u32,
                        end: position as u32 + 1,
                        bounds: volume,
                        facing: quad.axis_facing(),
                    });
                    previous_patch = Some(key);
                }
            }
        }
        let Some(visibility) = Visibility::build(&patches, cancelled)? else {
            return Ok(None);
        };
        #[cfg(test)]
        let hinted_faces = encoding.hinted_count();
        Ok(Some(Self {
            #[cfg(test)]
            hinted_faces,
            encoding,
            spans,
            patches,
            visibility,
            bounds: bounds.ok_or("后台面批次没有几何")?,
            faces: count,
        }))
    }
}

/// 不可变源面的排序数据；完整四角数据只保留在源 Arc 与常驻 GPU 分配中。
#[derive(Clone, Copy)]
pub(super) struct LocalFace {
    center: [f32; 3],
    bounds: crate::frustum::MeshBounds,
    page: u32,
    mode: u32,
}
impl LocalFace {
    pub fn new(quad: &WorldQuad) -> Self {
        Self {
            center: quad.sort_center(),
            bounds: crate::frustum::MeshBounds::from_quad(quad),
            page: quad.atlas_page,
            mode: quad.mode,
        }
    }
    pub fn placed(
        self,
        delta: [f32; 3],
        address: super::residency::Address,
        placement: u32,
    ) -> TransparentFace {
        TransparentFace {
            center: std::array::from_fn(|i| self.center[i] + delta[i]),
            bounds: self.bounds.translated(delta),
            page: self.page,
            mode: self.mode,
            address,
            placement,
            #[cfg(test)]
            source: None,
        }
    }
}
/// 已提交帧只保存轻量元数据和稳定源地址，镜头排序不复制完整源面。
#[derive(Clone)]
pub(super) struct TransparentFace {
    center: [f32; 3],
    bounds: crate::frustum::MeshBounds,
    page: u32,
    mode: u32,
    pub address: super::residency::Address,
    pub placement: u32,
    // 既有测试辅助从原始源按地址物化，不在生产缓存中保留第二份几何。
    #[cfg(test)]
    source: Option<(Arc<Vec<WorldQuad>>, usize, [f32; 3])>,
}
impl TransparentFace {
    #[cfg(test)]
    pub fn new(source: &Arc<Vec<WorldQuad>>, index: usize, delta: [f32; 3]) -> Self {
        let mut face = LocalFace::new(&source[index]).placed(
            delta,
            super::residency::Address {
                bank: 0,
                packed_face: index as u32,
            },
            0,
        );
        face.source = Some((Arc::clone(source), index, delta));
        face
    }
    #[cfg(test)]
    pub fn quad(&self) -> WorldQuad {
        let (source, index, delta) = self.source.as_ref().expect("测试面必须保留源地址");
        source[*index].translated(*delta)
    }
    pub fn bounds(&self) -> crate::frustum::MeshBounds {
        self.bounds
    }
    pub fn page(&self) -> usize {
        self.page as usize
    }
    pub fn single_sided(&self) -> bool {
        self.mode & crate::material_mode::SINGLE_SIDED != 0
    }
    pub fn tripwire(&self) -> bool {
        crate::material_mode::pass(self.mode) == crate::material_mode::Pass::Tripwire
    }
    pub fn additive(&self) -> bool {
        self.mode & crate::material_mode::ADDITIVE != 0
    }
}
#[cfg(test)]
pub(super) fn sorted_faces(
    faces: &[TransparentFace],
    order: &mut Vec<usize>,
    camera: Option<&CameraUniform>,
) -> Vec<WorldQuad> {
    let profile =
        *super::FRAME_PROFILE.get_or_init(|| std::env::var_os("MCEDIT2_FRAME_PROFILE").is_some());
    let started = profile.then(std::time::Instant::now);
    sort_faces(faces, order, camera);
    let sort_elapsed = started.map(|start| start.elapsed());
    let materialize = profile.then(std::time::Instant::now);
    let result = order.iter().map(|&i| faces[i].quad()).collect();
    if let (Some(sort), Some(start)) = (sort_elapsed, materialize) {
        eprintln!(
            "[frame-profile] transparent_sort sort_us={} materialize_us={} faces={}",
            sort.as_micros(),
            start.elapsed().as_micros(),
            faces.len()
        );
    }
    result
}
#[cfg(test)]
pub(super) fn sort_faces(
    faces: &[TransparentFace],
    order: &mut Vec<usize>,
    camera: Option<&CameraUniform>,
) {
    let _ = sort_faces_cancellable(faces, order, camera, &|| false);
}
#[cfg(test)]
pub(super) fn sort_faces_cancellable(
    faces: &[TransparentFace],
    order: &mut Vec<usize>,
    camera: Option<&CameraUniform>,
    cancelled: &dyn Fn() -> bool,
) -> bool {
    let mut sorting = Sorting::acquire();
    if !sorting.sort(faces, camera, cancelled) {
        return false;
    }
    order.clear();
    order.extend_from_slice(sorting.indices());
    true
}

pub(super) use crate::radix_sort::Sorting;
impl Sorting {
    pub(super) fn sort(
        &mut self,
        faces: &[TransparentFace],
        camera: Option<&CameraUniform>,
        cancelled: &dyn Fn() -> bool,
    ) -> bool {
        if let Some(camera) = camera {
            return self.sort_indices(
                faces.len(),
                |index| {
                    let bits = sort_distance(faces[index].center, camera).to_bits();
                    !(if bits & 0x8000_0000 != 0 {
                        !bits
                    } else {
                        bits ^ 0x8000_0000
                    })
                },
                cancelled,
            );
        }
        self.source_order(faces.len(), cancelled)
    }
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transparent_sort_keeps_chunk_sources_origins_and_distance_order() {
        use crate::spectator::MeshChunk;
        let face = WorldQuad {
            xyz: [[0.0, 0.0, 3.0]; 4],
            uv: [[0.0; 2]; 4],
            rgba: [[1.0; 4]; 4],
            mode: 2,
            atlas_page: 7,
            light: crate::lighting::FULLBRIGHT,
        };
        let far = Arc::new(vec![face.translated([-16.0, 0.0, 0.0])]);
        let near = Arc::new(vec![face.translated([16.0, 0.0, -2.0])]);
        let stream = StreamMesh {
            terrain: None,
            visibility: None,
            entities: None,
            complete: true,
            origin: [0; 3],
            atlas: None,
            chunks: vec![
                MeshChunk::new(
                    (1, 0, 1, 0, 0),
                    [16, 0, 0],
                    Arc::clone(&far),
                    Default::default(),
                ),
                MeshChunk::new(
                    (-1, 0, 1, 0, 0),
                    [-16, 0, 0],
                    Arc::clone(&near),
                    Default::default(),
                ),
            ]
            .into(),
        };
        let sources = stream
            .chunks
            .iter()
            .map(|part| {
                TransparentFace::new(
                    &part.quads(),
                    0,
                    super::super::origin_delta(part.origin, stream.origin),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(sources[0].quad(), far[0].translated([16., 0., 0.]));
        assert_eq!(sources[1].quad(), near[0].translated([-16., 0., 0.]));
        let mut order = Vec::new();
        let mut camera = CameraUniform {
            forward: [0.0, 0.0, 1.0, 0.0],
            ..Default::default()
        };
        let result = sorted_faces(&sources, &mut order, Some(&camera));
        assert_eq!(result[0], face);
        assert_eq!(result[1].xyz[0], [0.0, 0.0, 1.0]);
        camera.position[2] = 80.0;
        let translated = sorted_faces(&sources, &mut order, Some(&camera));
        assert_eq!(translated, vec![result[1], result[0]]);
        camera.forward[2] = -1.0;
        let reversed = sorted_faces(&sources, &mut order, Some(&camera));
        assert_eq!(reversed, translated);
        let points = Arc::new(
            (0..4096)
                .map(|i| {
                    let mut q = face;
                    q.xyz = [[0., 0., ((i * 7919 % 173) as f32 - 86.) / 7.]; 4];
                    q
                })
                .collect::<Vec<_>>(),
        );
        let samples = (0..points.len())
            .map(|i| TransparentFace::new(&points, i, [0.; 3]))
            .collect::<Vec<_>>();
        let mut actual = Vec::new();
        sort_faces(&samples, &mut actual, Some(&camera));
        let mut expected = (0..samples.len()).collect::<Vec<_>>();
        let depths = samples
            .iter()
            .map(|face| {
                (0..3)
                    .map(|i| (face.center[i] - camera.position[i]).powi(2))
                    .sum::<f32>()
            })
            .collect::<Vec<_>>();
        expected.sort_unstable_by(|a, b| depths[*b].total_cmp(&depths[*a]).then_with(|| a.cmp(b)));
        assert_eq!(actual, expected);
    }

    fn adapter_limits_segment_complete_instances_and_page_runs_preserve_order() {
        assert_eq!(segment_faces(151), 0);
        assert_eq!(segment_faces(156), 1);
        assert_eq!(segment_faces(156 * 3 + 155), 3);
        assert_eq!(segment_faces(u64::MAX), u32::MAX as usize);
        let face = WorldQuad {
            xyz: [[0.0; 3]; 4],
            uv: [[0.0; 2]; 4],
            rgba: [[1.0; 4]; 4],
            mode: 0,
            atlas_page: 70_000,
            light: crate::lighting::FULLBRIGHT,
        };
        let mut other = face;
        other.atlas_page = 1;
        assert_eq!(
            page_spans(&[face, face, other, face], &[]),
            vec![
                WorldSpan {
                    index_count: QUAD_INDICES.len() as u32,
                    page: 70_000,
                    single_sided: false,
                    depth_equal: false,
                    facing: 0,
                    start: 0,
                    end: 2
                },
                WorldSpan {
                    index_count: QUAD_INDICES.len() as u32,
                    page: 1,
                    single_sided: false,
                    depth_equal: false,
                    facing: 0,
                    start: 2,
                    end: 3
                },
                WorldSpan {
                    index_count: QUAD_INDICES.len() as u32,
                    page: 70_000,
                    single_sided: false,
                    depth_equal: false,
                    facing: 0,
                    start: 3,
                    end: 4
                },
            ]
        );
    }

    fn y32_cross_boundary_keeps_one_full_source_quad() {
        let quad = WorldQuad {
            xyz: [
                [0., 31.8, 0.],
                [1., 32.2, 0.],
                [1., 32.2, 1.],
                [0., 31.8, 1.],
            ],
            uv: [[0.; 2]; 4],
            rgba: [[1.; 4]; 4],
            mode: 0,
            atlas_page: 1,
            light: crate::lighting::FULLBRIGHT,
        };
        assert_eq!(super::super::upload::y_bucket(&quad, 0), 1);
        assert_eq!(super::super::upload::y_bucket(&quad, -64), -1);
        let bounds = crate::frustum::MeshBounds::from_quad(&quad);
        let camera = CameraUniform {
            position: [0.5, 32.3, -2., 0.],
            forward: [0., 0., 1., 0.],
            right: [1., 0., 0., 0.],
            up: [0., 1., 0., 0.],
            projection: [1., 1., 0.1, 100.],
        };
        assert!(bounds.visible(&camera), "跨边界的完整几何必须保守可见");
        let local_camera = CameraUniform {
            position: [0.; 4],
            forward: [0., 0., 1., 0.],
            right: [1., 0., 0., 0.],
            up: [0., 1., 0., 0.],
            projection: [1., 1., 0.1, 100.],
        };
        let patch = |page, start, x| {
            let q = WorldQuad {
                xyz: [[x, 0., 3.]; 4],
                ..quad
            };
            Patch {
                group: None,
                facing: 0,
                page,
                start,
                end: start + 1,
                bounds: crate::frustum::MeshBounds::from_quad(&q),
            }
        };
        let patches = vec![
            patch(0, 0, 0.),
            patch(0, 1, 100.),
            patch(0, 2, 0.),
            patch(1, 3, 100.),
        ];
        let local_view = crate::frustum::Frustum::new(&local_camera);
        let span = WorldSpan {
            index_count: QUAD_INDICES.len() as u32,
            page: 0,
            single_sided: false,
            depth_equal: false,
            facing: 0,
            start: 0,
            end: 3,
        };
        assert_eq!(
            visible_patch_ranges(&patches, &span, Some(&local_view)).collect::<Vec<_>>(),
            vec![(0, 1), (2, 3)],
            "分别提交两端可见片，不夹带已证明不可见的中间片"
        );
        let outside = WorldSpan {
            index_count: QUAD_INDICES.len() as u32,
            page: 1,
            single_sided: false,
            depth_equal: false,
            facing: 0,
            start: 3,
            end: 4,
        };
        assert_eq!(
            visible_patch_ranges(&patches, &outside, Some(&local_view)).collect::<Vec<_>>(),
            vec![]
        );
        assert_eq!(
            visible_patch_ranges(&patches, &span, None).collect::<Vec<_>>(),
            vec![(0, 3)]
        );
    }

    fn profile_contiguous_transparency() {
        if std::env::var_os("MCEDIT2_TRANSPARENT_CPU_PROFILE").is_none() {
            return;
        }
        let base = WorldQuad {
            xyz: [[0.; 3]; 4],
            uv: [[0.; 2]; 4],
            rgba: [[1.; 4]; 4],
            mode: 2,
            atlas_page: 0,
            light: crate::lighting::FULLBRIGHT,
        };
        let chunks = (0..368)
            .map(|chunk| {
                Arc::new(
                    (0..667)
                        .map(|i| {
                            let mut quad = base;
                            quad.xyz =
                                [[(i % 16) as f32, (i % 255) as f32, (chunk % 16) as f32]; 4];
                            quad.atlas_page = (i % 2) as u32;
                            quad
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let start = std::time::Instant::now();
        let cached = chunks
            .iter()
            .enumerate()
            .flat_map(|(chunk, source)| {
                (0..source.len())
                    .map(move |i| TransparentFace::new(source, i, [chunk as f32, 0., 0.]))
            })
            .collect::<Vec<_>>();
        let build = start.elapsed();
        let camera = CameraUniform {
            forward: [0.3, 0.2, 0.9, 0.],
            ..Default::default()
        };
        let mut order = Vec::new();
        let start = std::time::Instant::now();
        sort_faces(&cached, &mut order, Some(&camera));
        let sort = start.elapsed();
        let start = std::time::Instant::now();
        let old = order
            .iter()
            .map(|&index| {
                chunks[index / 667][index % 667].translated([(index / 667) as f32, 0., 0.])
            })
            .collect::<Vec<_>>();
        let old_time = start.elapsed();
        let start = std::time::Instant::now();
        let new = order
            .iter()
            .map(|&index| cached[index].quad())
            .collect::<Vec<_>>();
        let new_time = start.elapsed();
        assert_eq!(old, new);
        eprintln!(
            "[transparent-cpu-profile] faces={} cache_bytes={} build_us={} sort_us={} original_materialize_us={} cached_materialize_us={}",
            cached.len(),
            cached.len() * std::mem::size_of::<TransparentFace>(),
            build.as_micros(),
            sort.as_micros(),
            old_time.as_micros(),
            new_time.as_micros()
        );
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Rendering {
            transparent_sort_keeps_chunk_sources_origins_and_distance_order();
            adapter_limits_segment_complete_instances_and_page_runs_preserve_order();
            y32_cross_boundary_keeps_one_full_source_quad();
            profile_contiguous_transparency();
        }
    }
}
