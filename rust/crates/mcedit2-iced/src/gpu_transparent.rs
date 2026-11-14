//! 透明几何常驻 GPU；转向只上传排序后的八字节面与位置索引。
//! 源缓冲按设备存储绑定限制分段，绘制仍保持跨区块、跨图集的全局顺序。
use super::{commands, upload::Budget, world::TransparentFace};
use crate::frustum::{Frustum, MeshBounds};
use iced::wgpu;

/// 每个实例只上传常驻面地址和该帧的位置索引。
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    face: u32,
    placement: u32,
}
const INSTANCE_BYTES: usize = std::mem::size_of::<Instance>();
#[derive(Clone)]
pub(super) struct SourceSegment {
    pub binding: wgpu::BindGroup,
}
/// 不可变完整帧快照；lease、偏移和绑定共同保护旧画面及后台镜头排序。
#[derive(Clone, Default)]
pub(super) struct Sources {
    pub segments: Vec<Option<SourceSegment>>,
    allocations: std::sync::Arc<[std::sync::Arc<super::residency::Allocation>]>,
    offsets: Option<wgpu::Buffer>,
    uploaded_offsets: usize,
    ready: bool,
    slabs: usize,
    mapping: std::sync::Arc<super::residency::Mapping>,
}
impl Sources {
    pub fn same_snapshot(&self, other: &Self) -> bool {
        self.offsets == other.offsets
            && std::sync::Arc::ptr_eq(&self.allocations, &other.allocations)
            && std::sync::Arc::ptr_eq(&self.mapping, &other.mapping)
    }
    pub fn address(
        &self,
        physical: super::residency::Address,
    ) -> Result<super::residency::Address, String> {
        self.mapping.translate(physical)
    }
    pub fn slab_count(&self) -> usize {
        self.slabs
    }
    pub fn advance(
        &mut self,
        allocations: &[std::sync::Arc<super::residency::Allocation>],
        offsets: &[[f32; 4]],
        zero_offset: &wgpu::Buffer,
        arena: &super::residency::Arena,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        budget: &mut Budget,
    ) -> Result<bool, String> {
        if self.ready {
            return Ok(true);
        }
        if self.offsets.is_none() {
            let bytes = (offsets.len().max(1) as u64)
                .checked_mul(16)
                .ok_or("透明位置表大小溢出")?;
            if bytes
                > device
                    .limits()
                    .max_buffer_size
                    .min(u64::from(device.limits().max_storage_buffer_binding_size))
            {
                return Err("透明位置表超出设备 storage binding 容量".into());
            }
            self.offsets = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("MCEdit2 完整帧透明位置表"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.allocations = allocations.to_vec().into();
        }
        if !offsets.is_empty() && budget.capacity() < 16 {
            return Err("GPU 上传预算不足以容纳一个透明位置".into());
        }
        while self.uploaded_offsets < offsets.len() && !budget.exhausted() {
            let count = (budget.allowance() / 16).min(offsets.len() - self.uploaded_offsets);
            if count == 0 {
                return Ok(false);
            }
            let end = self.uploaded_offsets + count;
            queue.write_buffer(
                self.offsets.as_ref().expect("透明位置表已分配"),
                (self.uploaded_offsets * 16) as u64,
                bytemuck::cast_slice(&offsets[self.uploaded_offsets..end]),
            );
            budget.consume(count * 16);
            self.uploaded_offsets = end;
        }
        if self.uploaded_offsets < offsets.len() {
            return Ok(false);
        }
        if budget.exhausted() {
            return Ok(false);
        }
        let snapshot = arena.bindings(
            &self.allocations,
            self.offsets.as_ref().expect("透明位置表已分配"),
            zero_offset,
            layout,
            device,
        )?;
        self.segments = snapshot
            .groups
            .into_iter()
            .map(|binding| binding.map(|binding| SourceSegment { binding }))
            .collect();
        self.mapping = snapshot.mapping;
        self.slabs = super::residency::Arena::slab_count(&self.allocations);
        self.ready = true;
        Ok(true)
    }
}
#[derive(Clone, Copy)]
pub(super) struct Span {
    pub source: usize,
    pub page: usize,
    /// 仅为整段的硬件剔除提示；混合段使用片段着色器中的逐面材质位。
    pub single_sided: bool,
    pub tripwire: bool,
    pub additive: bool,
    pub start: u32,
    pub end: u32,
    bounds: MeshBounds,
}
pub(super) struct Segment {
    pub buffer: wgpu::Buffer,
    spans: std::sync::Arc<[Span]>,
    tripwire: bool,
    pub commands: commands::Cache,
    pub tripwire_commands: commands::Cache,
    visibility: std::sync::Arc<super::visibility::Index>,
}
impl Segment {
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }
    pub fn prepare_commands(
        &mut self,
        device: &wgpu::Device,
        bindings: &std::sync::Arc<commands::Bindings>,
        offset: &wgpu::BindGroup,
        sources: &Sources,
        view: Option<&Frustum>,
        tripwire: bool,
    ) -> Option<commands::Prepared> {
        if tripwire != self.tripwire {
            return None;
        }
        let selection =
            self.visibility
                .stamp(view, self.spans.len(), |i| self.spans[i].bounds, None);
        let spans = VisibleSpans {
            spans: &self.spans,
            cursor: 0,
            view,
            mask: selection.as_ref().and_then(super::visibility::Stamp::mask),
        };
        let commands = if tripwire {
            &mut self.tripwire_commands
        } else {
            &mut self.commands
        };
        commands.transparent(
            device,
            bindings,
            offset,
            &self.buffer,
            spans.filter(|span| span.tripwire == tripwire),
            sources,
            tripwire,
            selection,
        )
    }
    /// 只过滤全局排序中的不可见连续片，不重排或重建 GPU 索引。
    /// 相邻可见且绑定相同的片重新合并，避免元数据分片增加正常绘制次数。
    #[cfg(test)]
    pub fn visible_spans<'a>(&'a self, view: Option<&'a Frustum>) -> VisibleSpans<'a> {
        VisibleSpans {
            spans: &self.spans,
            cursor: 0,
            view,
            mask: self
                .visibility
                .visible(view, self.spans.len(), |i| self.spans[i].bounds),
        }
    }
}
pub(super) struct VisibleSpans<'a> {
    spans: &'a [Span],
    cursor: usize,
    view: Option<&'a Frustum>,
    mask: Option<std::sync::Arc<super::visibility::Mask>>,
}
impl Iterator for VisibleSpans<'_> {
    type Item = Span;
    fn next(&mut self) -> Option<Span> {
        let visible = |index: usize| {
            self.mask.as_ref().map_or_else(
                || {
                    self.view
                        .is_none_or(|view| self.spans[index].bounds.visible_in(view))
                },
                |mask| mask.contains(index),
            )
        };
        let index = if let Some(mask) = &self.mask {
            mask.next(self.cursor)?
        } else {
            (self.cursor..self.spans.len()).find(|index| visible(*index))?
        };
        self.cursor = index + 1;
        let mut run = self.spans[index];
        while let Some(next) = self.spans.get(self.cursor) {
            if next.start != run.end
                || next.source != run.source
                || next.page != run.page
                || next.tripwire != run.tripwire
                || next.additive != run.additive
                || !visible(self.cursor)
            {
                break;
            }
            run.end = next.end;
            // 材质位是逐面的实际规则；混合批次由现有片段着色器按真实朝向剔除。
            run.single_sided &= next.single_sided;
            run.bounds = run.bounds.union(next.bounds);
            self.cursor += 1;
        }
        Some(run)
    }
}
/// 后台打包与常驻源上传使用相同的设备限制；不把 Device 带入排序线程。
#[derive(Clone)]
pub(super) struct OrderLayout {
    index_segment: usize,
    page_groups: std::sync::Arc<[usize]>,
}
impl OrderLayout {
    pub fn for_device(device: &wgpu::Device, page_groups: std::sync::Arc<[usize]>) -> Self {
        Self {
            index_segment: (device.limits().max_buffer_size / INSTANCE_BYTES as u64)
                .min(u32::MAX as u64) as usize,
            page_groups,
        }
    }
}
struct PreparedSegment {
    tripwire: bool,
    indices: Vec<Instance>,
    spans: Vec<Span>,
    visibility: std::sync::Arc<super::visibility::Index>,
}
/// 排序线程交出的完整命令元数据；渲染线程只消费已打包的八字节实例。
pub(super) struct PreparedOrder {
    segments: std::collections::VecDeque<PreparedSegment>,
    tripwire: bool,
    banks: std::collections::BTreeSet<usize>,
}
impl PreparedOrder {
    pub fn prepare(
        faces: &[TransparentFace],
        order: &[usize],
        layout: OrderLayout,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Self>, String> {
        let profile = *super::FRAME_PROFILE
            .get_or_init(|| std::env::var_os("MCEDIT2_FRAME_PROFILE").is_some());
        let started = profile.then(std::time::Instant::now);
        if faces.len() != order.len() || (!order.is_empty() && layout.index_segment == 0) {
            return Err("透明排序与常驻源面数量不匹配".into());
        }
        let mut result = Self {
            segments: std::collections::VecDeque::new(),
            tripwire: false,
            banks: std::collections::BTreeSet::new(),
        };
        // 两个通道本来就在独立 render pass 中执行；各通道保留原全局排序的稳定子序列。
        // 分开索引后，绊线不再把普通透明绘制切成大量细碎区间，也不需要重复处理另一通道的顶点。
        for tripwire in [false, true] {
            let mut indices = Vec::new();
            let mut spans = Vec::<Span>::new();
            for (offset, &index) in order.iter().enumerate() {
                if offset.is_multiple_of(1024) && cancelled() {
                    return Ok(None);
                }
                let face = faces.get(index).ok_or("透明排序索引越界")?;
                if face.tripwire() != tripwire {
                    continue;
                }
                let source = face.address.bank;
                result.banks.insert(source);
                result.tripwire |= tripwire;
                let page = if layout.page_groups.is_empty() {
                    usize::MAX
                } else {
                    *layout
                        .page_groups
                        .get(face.page())
                        .ok_or("透明面引用的图集页越界")?
                };
                let single_sided = face.single_sided();
                let additive = face.additive();
                let bounds = face.bounds();
                let start = indices.len() as u32;
                indices.push(Instance {
                    face: face.address.packed_face,
                    placement: face.placement,
                });
                if let Some(last) = spans.last_mut().filter(|s| {
                    s.source == source
                        && s.page == page
                        && s.additive == additive
                        && s.end - s.start < 64
                }) {
                    last.end += 1;
                    last.single_sided &= single_sided;
                    last.bounds = last.bounds.union(bounds);
                } else {
                    spans.push(Span {
                        source,
                        page,
                        single_sided,
                        tripwire,
                        additive,
                        start,
                        end: start + 1,
                        bounds,
                    });
                }
                if indices.len() == layout.index_segment {
                    if !result.finish_segment(tripwire, &mut indices, &mut spans, cancelled)? {
                        return Ok(None);
                    }
                }
            }
            if !result.finish_segment(tripwire, &mut indices, &mut spans, cancelled)? {
                return Ok(None);
            }
        }
        if let Some(started) = started {
            let mut groups = std::collections::BTreeSet::new();
            let (mut source_switches, mut page_switches, mut spans) = (0, 0, 0);
            for segment in &result.segments {
                let mut previous = None;
                for span in &segment.spans {
                    if let Some((source, page)) = previous {
                        source_switches += usize::from(source != span.source);
                        page_switches += usize::from(page != span.page);
                    }
                    previous = Some((span.source, span.page));
                    groups.insert(span.page);
                    spans += 1;
                }
            }
            log::debug!("[frame-profile] transparent_order faces={} banks={} atlas_groups={} source_switches={} page_switches={} spans={} prepare_us={}",
                faces.len(), result.banks.len(), groups.len(), source_switches, page_switches, spans, started.elapsed().as_micros());
        }
        Ok((!cancelled()).then_some(result))
    }
    fn finish_segment(
        &mut self,
        tripwire: bool,
        indices: &mut Vec<Instance>,
        spans: &mut Vec<Span>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<bool, String> {
        if indices.is_empty() {
            return Ok(true);
        }
        let Some(visibility) =
            super::visibility::Index::build(spans.len(), |i| spans[i].bounds, true, cancelled)?
        else {
            return Ok(false);
        };
        self.segments.push_back(PreparedSegment {
            tripwire,
            indices: std::mem::take(indices),
            spans: std::mem::take(spans),
            visibility,
        });
        Ok(true)
    }
    pub fn validate_sources(&self, sources: &Sources) -> Result<(), String> {
        if self.banks.iter().any(|&bank| {
            sources
                .segments
                .get(bank)
                .and_then(Option::as_ref)
                .is_none()
        }) {
            return Err("后台透明索引引用了未驻留的源 bank".into());
        }
        Ok(())
    }
}

/// 绘制指令只引用此缓冲和绑定，不依赖排序数据内容；同容量回收时一并保留。
struct Recycled {
    buffer: wgpu::Buffer,
    commands: commands::Cache,
    tripwire_commands: commands::Cache,
}
#[derive(Default)]
pub(super) struct Order {
    pub segments: Vec<Segment>,
    tripwire: bool,
    /// 仅包含已经退出活动顺序的缓冲；上传不覆盖仍显示的那一组。
    reusable: Vec<Recycled>,
    uploading: Option<Recycled>,
    uploaded_indices: usize,
}
impl Order {
    pub fn has_tripwire(&self) -> bool {
        self.tripwire
    }
    pub fn with_recycled(previous: &mut Self) -> Self {
        Self {
            reusable: std::mem::take(&mut previous.reusable),
            ..Self::default()
        }
    }
    /// 完整顺序切换后回收旧组；只保留一组与当前段数相当的容量。
    pub fn recycle(&mut self, previous: Self) {
        self.reusable.clear();
        self.reusable.extend(
            previous
                .segments
                .into_iter()
                .take(self.segments.len())
                .map(|s| Recycled {
                    buffer: s.buffer,
                    commands: s.commands,
                    tripwire_commands: s.tripwire_commands,
                }),
        );
    }
    pub fn refresh_commands(&mut self, bindings: &super::commands::Bindings) {
        for segment in &mut self.segments {
            segment.commands.refresh_bindings(bindings);
            segment.tripwire_commands.refresh_bindings(bindings);
        }
        for recycled in self.reusable.iter_mut().chain(self.uploading.iter_mut()) {
            recycled.commands.refresh_bindings(bindings);
            recycled.tripwire_commands.refresh_bindings(bindings);
        }
    }
    pub fn advance(
        &mut self,
        prepared: &mut PreparedOrder,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        budget: &mut Budget,
    ) -> Result<bool, String> {
        if !prepared.segments.is_empty() && budget.capacity() < INSTANCE_BYTES {
            return Err("GPU 上传预算不足以容纳一个透明面索引".into());
        }
        self.tripwire = prepared.tripwire;
        while let Some(segment) = prepared.segments.front() {
            if budget.exhausted() {
                return Ok(false);
            }
            if self.uploading.is_none() {
                let needed = (segment.indices.len() * INSTANCE_BYTES) as u64;
                if needed == 0 || needed > device.limits().max_buffer_size {
                    return Err("后台透明索引分段超过实际 GPU 缓冲限制".into());
                }
                // 优先使用最小的足够容量，避免把完整段留给短尾段后再额外分配。
                let reusable = self
                    .reusable
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| item.buffer.size() >= needed)
                    .min_by_key(|(_, item)| item.buffer.size())
                    .map(|(index, _)| index);
                self.uploading = Some(if let Some(index) = reusable {
                    self.reusable.swap_remove(index)
                } else {
                    Recycled {
                        buffer: device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("MCEdit2 透明面顺序索引"),
                            size: needed,
                            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        }),
                        commands: Default::default(),
                        tripwire_commands: Default::default(),
                    }
                });
            }
            let count = (budget.allowance() / INSTANCE_BYTES)
                .min(65536)
                .min(segment.indices.len() - self.uploaded_indices);
            if count == 0 {
                return Ok(false);
            }
            let end = self.uploaded_indices + count;
            queue.write_buffer(
                &self
                    .uploading
                    .as_ref()
                    .expect("当前透明索引段已分配缓冲")
                    .buffer,
                (self.uploaded_indices * INSTANCE_BYTES) as u64,
                bytemuck::cast_slice(&segment.indices[self.uploaded_indices..end]),
            );
            budget.consume(count * INSTANCE_BYTES);
            self.uploaded_indices = end;
            if end == segment.indices.len() {
                let segment = prepared.segments.pop_front().expect("当前后台索引段存在");
                let recycled = self.uploading.take().expect("完整索引段持有 GPU 缓冲");
                self.segments.push(Segment {
                    buffer: recycled.buffer,
                    tripwire: segment.tripwire,
                    spans: segment.spans.into(),
                    commands: recycled.commands,
                    tripwire_commands: recycled.tripwire_commands,
                    visibility: segment.visibility,
                });
                self.uploaded_indices = 0;
            }
        }
        Ok(true)
    }
}
