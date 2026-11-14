//! 普通区域按后台实际完成顺序上传，独立保留在途缓冲及源偏移。
use super::*;

impl Pending {
    pub(super) fn advance_regions(
        &mut self,
        pipeline: &Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        budget: &mut Budget,
    ) -> Result<bool, String> {
        let stream = self
            .target
            .stream_mesh
            .as_ref()
            .ok_or("区域上传没有源网格")?;
        let Some(definitions) = self.region_definitions.as_ref() else {
            return Ok(false);
        };
        let groups = self
            .page_groups
            .as_ref()
            .ok_or("区域准备没有图集绑定定义")?;
        let hints = self.hints.as_ref().ok_or("区域材质提示丢失")?;
        let y32 = opaque_y32(pipeline);
        let bytes = budget
            .capacity()
            .min(device.limits().max_buffer_size.min(usize::MAX as u64) as usize);
        while !budget.exhausted() {
            while self.next_region_request < definitions.len() && !budget.exhausted() {
                let index = self.next_region_request;
                let (key, source) = &definitions[index];
                let old = self
                    .staged_regions
                    .get(key)
                    .filter(|r| r.reusable(source, y32, groups, hints))
                    .or_else(|| {
                        pipeline
                            .stream_regions
                            .get(key)
                            .filter(|r| r.reusable(source, y32, groups, hints))
                    });
                if let Some(old) = old {
                    if self.region_offsets.is_none() {
                        self.region_offsets = Some(regional::OffsetUpload::new(
                            stream.origin,
                            Some(&old.offsets),
                            device,
                            &pipeline.region_layout,
                        )?);
                    }
                    if !self
                        .region_offsets
                        .as_mut()
                        .ok_or("区域源偏移上传身份丢失")?
                        .advance(source, queue, budget)
                    {
                        return Ok(false);
                    }
                    let mut resident = old.clone();
                    resident.commands = Default::default();
                    resident.offsets = self
                        .region_offsets
                        .take()
                        .ok_or("已完成区域源偏移丢失")?
                        .offsets;
                    self.regions.insert(*key, resident);
                    self.region_revision += 1;
                    self.next_region += 1;
                } else {
                    if self.chunk_worker.is_none() {
                        self.chunk_worker = Some(opaque::Worker::start()?);
                    }
                    if !self
                        .chunk_worker
                        .as_mut()
                        .ok_or("区域准备线程丢失")?
                        .region(
                            index,
                            Arc::clone(source),
                            bytes,
                            y32,
                            Arc::clone(groups),
                            Arc::clone(hints),
                            self.target.atlas.clone(),
                        )?
                    {
                        break;
                    }
                }
                self.next_region_request += 1;
            }
            if self.next_region == definitions.len() {
                return Ok(true);
            }
            if budget.exhausted() {
                return Ok(false);
            }
            let event = if let Some(event) = self.region_event.take() {
                event
            } else {
                match self
                    .chunk_worker
                    .as_mut()
                    .ok_or("区域准备线程身份丢失")?
                    .poll_ready()?
                {
                    Some(opaque::Event::Regional(event)) => event,
                    Some(_) => return Err("源区块结果越过区域准备阶段".into()),
                    None => return Ok(false),
                }
            };
            let index = match &event {
                opaque::RegionalEvent::Batch { region, .. }
                | opaque::RegionalEvent::Complete { region, .. } => *region,
            };
            if index >= self.next_region_request {
                return Err("区域结果没有对应的已提交请求".into());
            }
            let (key, source) = &definitions[index];
            if self.regions.contains_key(key) {
                return Err("区域在完成后重复返回结果".into());
            }
            if !self.region_progress.contains_key(&index) {
                self.region_progress.insert(
                    index,
                    RegionalProgress {
                        transfer: Transfer::default(),
                        offsets: regional::OffsetUpload::new(
                            stream.origin,
                            None,
                            device,
                            &pipeline.region_layout,
                        )?,
                    },
                );
            }
            let progress = self
                .region_progress
                .get_mut(&index)
                .ok_or("在途区域上传状态丢失")?;
            if !progress.offsets.advance(source, queue, budget) {
                self.region_event = Some(event);
                return Ok(false);
            }
            match event {
                opaque::RegionalEvent::Batch { region, segment } => {
                    let needed = segment.bytes();
                    if needed > budget.capacity() {
                        return Err("单个普通区域批次超过 GPU 上传容量".into());
                    }
                    if needed > budget.allowance() || budget.exhausted() {
                        self.region_event = Some(opaque::RegionalEvent::Batch { region, segment });
                        return Ok(false);
                    }
                    let bounds = segment.bounds;
                    if segment.compact() {
                        self.compact_faces += segment.faces;
                        self.compact_saved_bytes +=
                            segment.faces * std::mem::size_of::<encoding::RegionalFull>() - needed;
                    }
                    self.prepared_faces += segment.faces;
                    progress
                        .transfer
                        .buffers
                        .append_prepared(device, queue, segment)?;
                    progress.transfer.bounds = Some(
                        progress
                            .transfer
                            .bounds
                            .map_or(bounds, |prior| prior.union(bounds)),
                    );
                    budget.consume(needed);
                }
                opaque::RegionalEvent::Complete { animated, .. } => {
                    let progress = self
                        .region_progress
                        .remove(&index)
                        .ok_or("已完成区域上传状态丢失")?;
                    let mut resident = regional::Resident {
                        source: Arc::clone(source),
                        buffers: Arc::new(progress.transfer.buffers),
                        bounds: progress.transfer.bounds,
                        groups: Arc::clone(groups),
                        hints: Arc::clone(hints),
                        animated,
                        y32,
                        commands: commands::Cache::default(),
                        offsets: progress.offsets.offsets,
                    };
                    let bindings = self
                        .prepared_commands
                        .get_or_insert_with(|| commands::Bindings::prepared(pipeline, &self.pages));
                    // 首次指令和间接参数缓冲也提前完成；视锥变化时继续复用已有参数容量。
                    let _ = resident.commands.opaque(
                        device,
                        queue,
                        bindings,
                        &resident.offsets.binding,
                        &resident.buffers,
                        None,
                    );
                    self.regions.insert(*key, resident);
                    self.region_revision += 1;
                    self.next_region += 1;
                    if !self.frozen && self.latest.is_some() {
                        self.retarget_ready = true;
                        return Ok(false);
                    }
                }
            }
        }
        Ok(self.next_region == definitions.len())
    }
}
