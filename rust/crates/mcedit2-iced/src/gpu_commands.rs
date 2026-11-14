//! 常驻网格的绘制指令缓存；可见范围与实际 GPU 资源身份共同决定复用。
//! 只缓存提交命令，不保留另一份几何或改变各通道的绘制顺序。
use super::{transparent, world, Pipeline, COLOR_FORMAT, DEPTH_FORMAT};
use crate::frustum::Frustum;
use crate::material_mode::Pass;
use iced::wgpu;
use std::collections::BTreeMap;
use std::sync::Arc;
#[path = "gpu_indirect.rs"]
mod indirect;

pub(super) fn native_indirect(device: &wgpu::Device) -> bool {
    indirect::available(device)
}

#[derive(Clone)]
pub(super) enum Command {
    Bundle(wgpu::RenderBundle),
    Indirect(Arc<indirect::Indirect>),
}
/// 合并连续 bundle 的执行，间接批次按原序完整绑定后绘制。
pub(super) fn execute(pass: &mut wgpu::RenderPass<'_>, commands: &[Command]) {
    let mut cursor = 0;
    while cursor < commands.len() {
        match &commands[cursor] {
            Command::Indirect(record) => {
                record.execute(pass);
                cursor += 1;
            }
            Command::Bundle(_) => {
                let begin = cursor;
                while cursor < commands.len() && matches!(commands[cursor], Command::Bundle(_)) {
                    cursor += 1;
                }
                pass.execute_bundles(commands[begin..cursor].iter().map(|c| match c {
                    Command::Bundle(bundle) => bundle,
                    Command::Indirect(_) => unreachable!("已限定连续 bundle 区间"),
                }));
            }
        }
    }
}

/// 同一完整帧共用实际管线和图集绑定，避免每区块复制整份身份列表。
pub(super) struct Bindings {
    opaque: [wgpu::RenderPipeline; 10],
    transparent: [wgpu::RenderPipeline; 4],
    tripwire: [wgpu::RenderPipeline; 4],
    quad_indices: wgpu::Buffer,
    fallback: wgpu::BindGroup,
    pages: Vec<wgpu::BindGroup>,
    page_groups: Vec<usize>,
}
impl Bindings {
    fn pipelines(&self, stage: Pass) -> &[wgpu::RenderPipeline] {
        match stage {
            Pass::Opaque => &self.opaque,
            Pass::Tripwire => &self.tripwire,
            Pass::Translucent => &self.transparent,
        }
    }
    fn new(p: &Pipeline) -> Self {
        Self::with_pages(p, &p.atlas_pages)
    }
    pub(super) fn prepared(p: &Pipeline, pages: &[super::atlas::Page]) -> Arc<Self> {
        Arc::new(Self::with_pages(p, pages))
    }
    fn with_pages(p: &Pipeline, pages: &[super::atlas::Page]) -> Self {
        Self {
            opaque: [
                p.world_opaque.clone(),
                p.world_opaque_culled.clone(),
                p.world_region[0].clone(),
                p.world_region[1].clone(),
                p.world_region[2].clone(),
                p.world_region[3].clone(),
                p.world_region[4].clone(),
                p.world_region[5].clone(),
                p.world_decal.clone(),
                p.world_decal_culled.clone(),
            ],
            transparent: [
                p.world_transparent.clone(),
                p.world_transparent_culled.clone(),
                p.world_additive[0].clone(),
                p.world_additive[1].clone(),
            ],
            tripwire: [
                p.world_tripwire.clone(),
                p.world_tripwire_culled.clone(),
                p.world_additive[2].clone(),
                p.world_additive[3].clone(),
            ],
            quad_indices: p.quad_indices.clone(),
            fallback: p.atlas_bind_group.clone(),
            pages: pages.iter().map(|page| page.1.clone()).collect(),
            page_groups: pages.iter().map(|page| page.2.binding_page).collect(),
        }
    }
    fn matches(&self, p: &Pipeline) -> bool {
        self.opaque[0] == p.world_opaque
            && self.opaque[1] == p.world_opaque_culled
            && self.opaque[2..8] == p.world_region
            && self.opaque[8] == p.world_decal
            && self.opaque[9] == p.world_decal_culled
            && self.transparent[0] == p.world_transparent
            && self.transparent[1] == p.world_transparent_culled
            && self.transparent[2..4] == p.world_additive[0..2]
            && self.tripwire[0] == p.world_tripwire
            && self.tripwire[1] == p.world_tripwire_culled
            && self.tripwire[2..4] == p.world_additive[2..4]
            && self.quad_indices == p.quad_indices
            && self.fallback == p.atlas_bind_group
            && self.pages.len() == p.atlas_pages.len()
            && self
                .pages
                .iter()
                .zip(&p.atlas_pages)
                .all(|(binding, page)| binding == &page.1)
            && self
                .page_groups
                .iter()
                .zip(&p.atlas_pages)
                .all(|(group, page)| *group == page.2.binding_page)
    }
    fn page(&self, index: usize) -> Option<&wgpu::BindGroup> {
        // 未加载图集是明确的无纹理模式；已有图集中的越界页不能改绑白图。
        if self.pages.is_empty() {
            Some(&self.fallback)
        } else {
            self.pages.get(index)
        }
    }
    fn group(&self, page: usize) -> usize {
        if self.page_groups.is_empty() {
            usize::MAX
        } else {
            self.page_groups[page]
        }
    }
}

pub(super) fn bindings(pipeline: &mut Pipeline) -> Arc<Bindings> {
    if let Some(old) = pipeline
        .command_bindings
        .as_ref()
        .filter(|old| old.matches(pipeline))
    {
        return Arc::clone(old);
    }
    let next = Arc::new(Bindings::new(pipeline));
    // 镜头外也释放真正变化的资源；追加无关实体页不使地形指令整体失效。
    for chunk in pipeline.stream_chunks.values_mut() {
        chunk.ordered_commands.refresh_bindings(&next);
    }
    for region in pipeline.stream_regions.values_mut() {
        region.commands.refresh_bindings(&next);
    }
    pipeline.world_sorted.refresh_commands(&next);
    pipeline.command_bindings = Some(Arc::clone(&next));
    next
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Draw {
    buffer: usize,
    source: usize,
    page: usize,
    single_sided: bool,
    depth_equal: bool,
    additive: bool,
    compact: bool,
    regional: bool,
    translated: bool,
    index_count: u32,
    start: u32,
    end: u32,
}
impl Draw {
    fn pipeline_index(&self) -> usize {
        if self.additive {
            return 2 + usize::from(self.single_sided);
        }
        if self.depth_equal {
            return 8 + usize::from(self.single_sided);
        }
        usize::from(self.single_sided)
            + usize::from(self.regional)
                * (2 + usize::from(self.compact) * 2 + usize::from(self.translated) * 2)
    }
}

/// 可共享的已编码快照；暂存数组属于各个视图，不随流式快照复制。
#[derive(Default)]
pub(super) struct Cache {
    ready: Option<Arc<Recorded>>,
    scratch: Vec<Draw>,
    selection: Vec<super::visibility::Stamp>,
    selection_scratch: Vec<super::visibility::Stamp>,
    selection_ready: bool,
    #[cfg(test)]
    pub range_builds: usize,
    #[cfg(test)]
    pub selection_reuses: usize,
}
/// 指令只持有实际使用的 GPU 页和管线，不保活旧整套图集。
struct UsedBindings {
    pipelines: BTreeMap<usize, wgpu::RenderPipeline>,
    pages: BTreeMap<usize, wgpu::BindGroup>,
    quad_indices: wgpu::Buffer,
}
impl UsedBindings {
    fn new(bindings: &Bindings, stage: Pass, draws: &[Draw]) -> Self {
        let mut result = Self {
            pipelines: BTreeMap::new(),
            pages: BTreeMap::new(),
            quad_indices: bindings.quad_indices.clone(),
        };
        for draw in draws {
            let index = draw.pipeline_index();
            result
                .pipelines
                .entry(index)
                .or_insert_with(|| bindings.pipelines(stage)[index].clone());
            result.pages.entry(draw.page).or_insert_with(|| {
                bindings
                    .page(draw.page)
                    .expect("绘制命令引用的图集页必须存在")
                    .clone()
            });
        }
        result
    }
    fn matches(&self, bindings: &Bindings, stage: Pass) -> bool {
        self.quad_indices == bindings.quad_indices
            && self
                .pipelines
                .iter()
                .all(|(index, pipeline)| pipeline == &bindings.pipelines(stage)[*index])
            && self
                .pages
                .iter()
                .all(|(index, page)| Some(page) == bindings.page(*index))
    }
    fn page(&self, index: usize) -> &wgpu::BindGroup {
        self.pages.get(&index).expect("已冻结指令的实际材质页")
    }
    fn pipeline(&self, index: usize) -> &wgpu::RenderPipeline {
        self.pipelines.get(&index).expect("已冻结指令的实际管线")
    }
}
struct Recorded {
    bundle: Command,
    bindings: Arc<UsedBindings>,
    offset: wgpu::BindGroup,
    buffers: Vec<wgpu::Buffer>,
    formats: Vec<[bool; 3]>,
    sources: Vec<Option<wgpu::BindGroup>>,
    indexed: bool,
    stage: Pass,
    draws: Vec<Draw>,
    faces: usize,
}
impl Clone for Cache {
    fn clone(&self) -> Self {
        Self {
            ready: self.ready.clone(),
            scratch: Vec::new(),
            selection: self.selection.clone(),
            selection_scratch: Vec::new(),
            selection_ready: self.selection_ready,
            #[cfg(test)]
            range_builds: 0,
            #[cfg(test)]
            selection_reuses: 0,
        }
    }
}
pub(super) struct Prepared {
    pub bundle: Command,
    pub reused: bool,
    pub draws: usize,
    pub faces: usize,
}

#[derive(Clone, Copy)]
enum Geometry<'a> {
    Opaque(&'a world::Buffers),
    Indexed {
        buffer: &'a wgpu::Buffer,
        sources: &'a transparent::Sources,
    },
}
impl<'a> Geometry<'a> {
    fn buffer_count(self) -> usize {
        match self {
            Self::Opaque(buffers) => buffers.segments.len(),
            Self::Indexed { .. } => 1,
        }
    }
    fn buffer(self, index: usize) -> &'a wgpu::Buffer {
        match self {
            Self::Opaque(buffers) => &buffers.segments[index].buffer,
            Self::Indexed { buffer, .. } => buffer,
        }
    }
    fn format(self, index: usize) -> [bool; 3] {
        match self {
            Self::Opaque(buffers) => {
                let segment = &buffers.segments[index];
                [segment.compact, segment.regional, segment.translated]
            }
            Self::Indexed { .. } => [false; 3],
        }
    }
}

impl Recorded {
    fn matches(
        &self,
        bindings: &Arc<Bindings>,
        offset: &wgpu::BindGroup,
        geometry: Geometry<'_>,
        stage: Pass,
    ) -> bool {
        let same_sources = match geometry {
            Geometry::Opaque(_) => self.sources.is_empty(),
            Geometry::Indexed { sources, .. } => {
                self.sources.len() == sources.segments.len()
                    && self
                        .sources
                        .iter()
                        .zip(&sources.segments)
                        .all(|(a, b)| a.as_ref() == b.as_ref().map(|source| &source.binding))
            }
        };
        self.indexed == matches!(geometry, Geometry::Indexed { .. })
            && self.stage == stage
            && self.bindings.matches(bindings, stage)
            && &self.offset == offset
            && self.buffers.len() == geometry.buffer_count()
            && self
                .buffers
                .iter()
                .enumerate()
                .all(|(i, b)| b == geometry.buffer(i))
            && self.formats.len() == geometry.buffer_count()
            && self
                .formats
                .iter()
                .enumerate()
                .all(|(i, f)| *f == geometry.format(i))
            && same_sources
    }
    fn reused(&self) -> Prepared {
        Prepared {
            bundle: self.bundle.clone(),
            reused: true,
            draws: match &self.bundle {
                Command::Bundle(_) => self.draws.len(),
                Command::Indirect(r) => r.draw_count(),
            },
            faces: self.faces,
        }
    }
}

impl Cache {
    /// 只合并全局顺序中连续且绑定一致的区间；不跨不可见间隙、不调整透明面的顺序。
    fn append(&mut self, draw: Draw) {
        if let Some(previous) = self.scratch.last_mut().filter(|old| {
            old.buffer == draw.buffer
                && old.source == draw.source
                && old.page == draw.page
                && old.single_sided == draw.single_sided
                && old.depth_equal == draw.depth_equal
                && old.compact == draw.compact
                && old.regional == draw.regional
                && old.translated == draw.translated
                && old.index_count == draw.index_count
                && old.end == draw.start
        }) {
            previous.end = draw.end;
        } else {
            self.scratch.push(draw);
        }
    }
    #[cfg(test)]
    pub(super) fn draw_count(&self) -> usize {
        self.ready.as_ref().map_or(0, |r| match &r.bundle {
            Command::Bundle(_) => r.draws.len(),
            Command::Indirect(i) => i.draw_count(),
        })
    }
    pub(super) fn invalidate(&mut self) {
        self.ready = None;
        self.scratch.clear();
        self.selection.clear();
        self.selection_scratch.clear();
        self.selection_ready = false;
    }
    pub(super) fn refresh_bindings(&mut self, bindings: &Bindings) {
        if self
            .ready
            .as_ref()
            .is_some_and(|ready| !ready.bindings.matches(bindings, ready.stage))
        {
            self.invalidate();
        }
    }
    fn reuse_selection(
        &mut self,
        bindings: &Arc<Bindings>,
        offset: &wgpu::BindGroup,
        geometry: Geometry<'_>,
        stage: Pass,
    ) -> Option<Prepared> {
        let result = self
            .ready
            .as_ref()
            .filter(|ready| {
                self.selection_ready
                    && self.selection == self.selection_scratch
                    && ready.matches(bindings, offset, geometry, stage)
            })
            .map(|ready| ready.reused());
        #[cfg(test)]
        if result.is_some() {
            self.selection_reuses += 1;
        }
        result
    }
    fn remember_selection(&mut self, valid: bool) {
        self.selection_ready = valid;
        if valid {
            std::mem::swap(&mut self.selection, &mut self.selection_scratch);
        } else {
            self.selection.clear();
        }
        self.selection_scratch.clear();
    }
    pub fn opaque(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bindings: &Arc<Bindings>,
        offset: &wgpu::BindGroup,
        buffers: &world::Buffers,
        view: Option<&Frustum>,
    ) -> Option<Prepared> {
        self.selection_scratch.clear();
        let mut selected = true;
        for segment in &buffers.segments {
            if let Some(selection) = segment.selection(view) {
                self.selection_scratch.push(selection);
            } else {
                selected = false;
                break;
            }
        }
        if selected {
            if let Some(prepared) =
                self.reuse_selection(bindings, offset, Geometry::Opaque(buffers), Pass::Opaque)
            {
                self.selection_scratch.clear();
                return Some(prepared);
            }
        }
        #[cfg(test)]
        {
            self.range_builds += 1;
        }
        self.scratch.clear();
        for (buffer, segment) in buffers.segments.iter().enumerate() {
            let visibility = if selected {
                self.selection_scratch[buffer].mask()
            } else {
                segment.visibility_mask(view)
            };
            for span in segment.spans() {
                for (start, end) in segment.visible_ranges_cached(span, view, visibility.clone()) {
                    self.append(Draw {
                        buffer,
                        source: 0,
                        page: bindings.group(span.page),
                        single_sided: span.single_sided,
                        depth_equal: span.depth_equal,
                        additive: false,
                        compact: segment.compact,
                        regional: segment.regional,
                        translated: segment.translated,
                        index_count: span.index_count,
                        start,
                        end,
                    });
                }
            }
        }
        let result = self.prepare(
            device,
            Some(queue),
            bindings,
            offset,
            Geometry::Opaque(buffers),
            Pass::Opaque,
        );
        self.remember_selection(selected && result.is_some());
        result
    }
    pub fn transparent(
        &mut self,
        device: &wgpu::Device,
        bindings: &Arc<Bindings>,
        offset: &wgpu::BindGroup,
        buffer: &wgpu::Buffer,
        spans: impl Iterator<Item = transparent::Span>,
        sources: &transparent::Sources,
        tripwire: bool,
        selection: Option<super::visibility::Stamp>,
    ) -> Option<Prepared> {
        self.selection_scratch.clear();
        let selected = selection.is_some();
        self.selection_scratch.extend(selection);
        let stage = if tripwire {
            Pass::Tripwire
        } else {
            Pass::Translucent
        };
        if selected {
            if let Some(prepared) = self.reuse_selection(
                bindings,
                offset,
                Geometry::Indexed { buffer, sources },
                stage,
            ) {
                self.selection_scratch.clear();
                return Some(prepared);
            }
        }
        #[cfg(test)]
        {
            self.range_builds += 1;
        }
        self.scratch.clear();
        for span in spans {
            self.append(Draw {
                buffer: 0,
                source: span.source,
                page: bindings.group(span.page),
                single_sided: span.single_sided,
                compact: false,
                depth_equal: false,
                additive: span.additive,
                regional: false,
                translated: false,
                index_count: world::QUAD_INDICES.len() as u32,
                start: span.start,
                end: span.end,
            });
        }
        let result = self.prepare(
            device,
            None,
            bindings,
            offset,
            Geometry::Indexed { buffer, sources },
            stage,
        );
        self.remember_selection(selected && result.is_some());
        result
    }
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: Option<&wgpu::Queue>,
        bindings: &Arc<Bindings>,
        offset: &wgpu::BindGroup,
        geometry: Geometry<'_>,
        stage: Pass,
    ) -> Option<Prepared> {
        if self.scratch.is_empty() {
            return None;
        }
        let indexed = matches!(geometry, Geometry::Indexed { .. });
        let same = self.ready.as_ref().is_some_and(|ready| {
            ready.matches(bindings, offset, geometry, stage) && ready.draws == self.scratch
        });
        if let Some(ready) = self.ready.as_ref().filter(|_| same) {
            return Some(ready.reused());
        }
        if let Geometry::Indexed { sources, .. } = geometry {
            if let Some(invalid) = self.scratch.iter().find(|draw| {
                sources
                    .segments
                    .get(draw.source)
                    .and_then(Option::as_ref)
                    .is_none()
            }) {
                log::error!(
                    "[gpu-commands] 透明绘制引用了未驻留的存储组 {}，拒绝编码该批次",
                    invalid.source
                );
                self.invalidate();
                return None;
            }
        }
        // Cache 的副本可能仍服务于旧完整帧。只有记录和实际指令都独占时，
        // 才允许重写参数；共享帧继续保有自己的不可变指令与 GPU 缓冲。
        let (recycled_draws, arguments) = self.ready.take().and_then(Arc::into_inner).map_or_else(
            || (Vec::new(), Vec::new()),
            |retired| {
                let mut draws = retired.draws;
                draws.clear();
                let arguments = match retired.bundle {
                    Command::Indirect(record) => Arc::into_inner(record)
                        .map(indirect::Indirect::into_arguments)
                        .unwrap_or_default(),
                    Command::Bundle(_) => Vec::new(),
                };
                (draws, arguments)
            },
        );
        let mut draw_count = self.scratch.len();
        let used = Arc::new(UsedBindings::new(bindings, stage, &self.scratch));
        let bundle = if let Geometry::Opaque(buffers) = geometry {
            if indirect::available(device) {
                let record = indirect::Indirect::new(
                    device,
                    queue.expect("不透明指令编码必须提供实际提交队列"),
                    &used,
                    offset,
                    buffers,
                    &self.scratch,
                    arguments,
                );
                draw_count = record.draw_count();
                Some(Command::Indirect(Arc::new(record)))
            } else {
                None
            }
        } else {
            None
        };
        let bundle = bundle.unwrap_or_else(|| {
            let mut encoder =
                device.create_render_bundle_encoder(&wgpu::RenderBundleEncoderDescriptor {
                    label: Some("MCEdit2 常驻源网格绘制指令"),
                    color_formats: &[Some(COLOR_FORMAT)],
                    depth_stencil: Some(wgpu::RenderBundleDepthStencil {
                        format: DEPTH_FORMAT,
                        // 两个实际 render pass 都具有 depth_ops；即使透明管线不写 Z，附件也非只读。
                        depth_read_only: false,
                        stencil_read_only: true,
                    }),
                    sample_count: 1,
                    multiview: None,
                });
            if !indexed {
                encoder.set_bind_group(1, offset, &[]);
            }
            encoder.set_index_buffer(used.quad_indices.slice(..), wgpu::IndexFormat::Uint16);
            let (mut buffer, mut page, mut source, mut cull) = (None, None, None, None);
            for draw in &self.scratch {
                let pipeline_index = draw.pipeline_index();
                if cull != Some(pipeline_index) {
                    encoder.set_pipeline(used.pipeline(pipeline_index));
                    cull = Some(pipeline_index);
                }
                if buffer != Some(draw.buffer) {
                    encoder.set_vertex_buffer(0, geometry.buffer(draw.buffer).slice(..));
                    buffer = Some(draw.buffer);
                }
                if page != Some(draw.page) {
                    encoder.set_bind_group(0, used.page(draw.page), &[]);
                    page = Some(draw.page);
                }
                if let Geometry::Indexed { sources, .. } = geometry {
                    if source != Some(draw.source) {
                        let bank = sources.segments[draw.source]
                            .as_ref()
                            .expect("绘制前已验证存储组");
                        encoder.set_bind_group(1, &bank.binding, &[]);
                        source = Some(draw.source);
                    }
                }
                encoder.draw_indexed(0..draw.index_count, 0, draw.start..draw.end);
            }
            let recorded_bundle = encoder.finish(&wgpu::RenderBundleDescriptor {
                label: Some("MCEdit2 已编码源网格绘制指令"),
            });
            Command::Bundle(recorded_bundle)
        });
        let buffers = (0..geometry.buffer_count())
            .map(|i| geometry.buffer(i).clone())
            .collect();
        let formats = (0..geometry.buffer_count())
            .map(|i| geometry.format(i))
            .collect();
        let sources = if let Geometry::Indexed { sources, .. } = geometry {
            sources
                .segments
                .iter()
                .map(|source| source.as_ref().map(|source| source.binding.clone()))
                .collect()
        } else {
            Vec::new()
        };
        let draws = std::mem::replace(&mut self.scratch, recycled_draws);
        let faces = draws
            .iter()
            .map(|draw| (draw.end - draw.start) as usize)
            .sum();

        self.ready = Some(Arc::new(Recorded {
            bundle: bundle.clone(),
            bindings: used,
            offset: offset.clone(),
            buffers,
            formats,
            sources,
            indexed,
            stage,
            draws,
            faces,
        }));
        Some(Prepared {
            bundle,
            reused: false,
            draws: draw_count,
            faces,
        })
    }
}

#[cfg(test)]
pub(super) fn verify_binding_dependencies(
    pipeline: &Pipeline,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) {
    let region = pipeline
        .stream_regions
        .values()
        .find(|region| !region.buffers.segments.is_empty())
        .expect("检查场景需要实际常驻的普通几何");
    let bindings = Arc::new(Bindings::new(pipeline));
    let mut cache = region.commands.clone();
    cache
        .opaque(
            device,
            queue,
            &bindings,
            &region.offsets.binding,
            &region.buffers,
            None,
        )
        .expect("实际普通面应生成绘制指令");
    let mut extended = Bindings::new(pipeline);
    extended.pages.push(extended.fallback.clone());
    extended.page_groups.push(extended.pages.len() - 1);
    let extended = Arc::new(extended);
    cache.refresh_bindings(&extended);
    assert!(
        cache
            .opaque(
                device,
                queue,
                &extended,
                &region.offsets.binding,
                &region.buffers,
                None
            )
            .unwrap()
            .reused,
        "追加未使用的 GPU 材质页不能重建地形指令"
    );
    let used_page = *cache
        .ready
        .as_ref()
        .unwrap()
        .bindings
        .pages
        .keys()
        .next()
        .unwrap();
    let mut changed = Bindings::new(pipeline);
    assert_ne!(changed.pages[used_page], pipeline.composite_bind_group);
    changed.pages[used_page] = pipeline.composite_bind_group.clone();
    cache.refresh_bindings(&changed);
    assert!(
        cache.ready.is_none(),
        "实际引用的 GPU 绑定变化必须释放旧指令"
    );
}
