//! 能力协商后的批量间接绘制；每个来源、材质与顺序边界保持不变。
use super::{Draw, UsedBindings};
use iced::wgpu;
use std::sync::Arc;

struct Batch {
    buffer: usize,
    page: usize,
    pipeline: usize,
    arguments: usize,
    offset: u64,
    count: u32,
}
pub(in crate::gpu_scene) struct Indirect {
    bindings: Arc<UsedBindings>,
    offset: wgpu::BindGroup,
    buffers: Vec<wgpu::Buffer>,
    arguments: Vec<wgpu::Buffer>,
    batches: Vec<Batch>,
}
pub(super) fn available(device: &wgpu::Device) -> bool {
    device.features().contains(
        wgpu::Features::INDIRECT_FIRST_INSTANCE | wgpu::Features::MULTI_DRAW_INDIRECT_COUNT,
    ) && device.limits().max_buffer_size >= 20
}
impl Indirect {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bindings: &Arc<UsedBindings>,
        offset: &wgpu::BindGroup,
        buffers: &super::world::Buffers,
        draws: &[Draw],
        reusable: Vec<wgpu::Buffer>,
    ) -> Self {
        // 五个四字节参数符合 DrawIndexedIndirectArgs ABI；每个缓冲受实际设备限制。
        let capacity = (device.limits().max_buffer_size / 20).min(4096) as usize;
        let mut arguments = Vec::new();
        let mut reusable = reusable.into_iter();
        let mut batches: Vec<Batch> = Vec::new();
        for (buffer_index, part) in draws.chunks(capacity).enumerate() {
            let data: Vec<[u32; 5]> = part
                .iter()
                .map(|d| [d.index_count, d.end - d.start, 0, 0, d.start])
                .collect();
            let bytes = bytemuck::cast_slice(&data);
            let size = bytes.len() as u64;
            let argument_buffer = reusable
                .next()
                .filter(|buffer| buffer.size() >= size)
                .unwrap_or_else(|| {
                    device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("MCEdit2 常驻不透明间接指令"),
                        size: size
                            .next_power_of_two()
                            .min(device.limits().max_buffer_size),
                        usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    })
                });
            // 只回收没有其他逻辑帧持有的指令缓冲。Queue 保证上一提交先读完旧值，
            // 下一提交再看到这里的更新；转头不再反复创建、映射和销毁缓冲。
            queue.write_buffer(&argument_buffer, 0, bytes);
            arguments.push(argument_buffer);
            for (index, draw) in part.iter().enumerate() {
                let pipeline = draw.pipeline_index();
                if let Some(last) = batches.last_mut().filter(|b| {
                    b.buffer == draw.buffer
                        && b.page == draw.page
                        && b.pipeline == pipeline
                        && b.arguments == buffer_index
                }) {
                    last.count += 1;
                } else {
                    batches.push(Batch {
                        buffer: draw.buffer,
                        page: draw.page,
                        pipeline,
                        arguments: buffer_index,
                        offset: index as u64 * 20,
                        count: 1,
                    });
                }
            }
        }
        Self {
            bindings: Arc::clone(bindings),
            offset: offset.clone(),
            buffers: buffers.segments.iter().map(|s| s.buffer.clone()).collect(),
            arguments,
            batches,
        }
    }
    pub(super) fn into_arguments(self) -> Vec<wgpu::Buffer> {
        self.arguments
    }
    pub(super) fn draw_count(&self) -> usize {
        self.batches.len()
    }
    pub(super) fn execute(&self, pass: &mut wgpu::RenderPass<'_>) {
        // bundle 执行后会清空状态，所以每个批次集合都完整绑定自己的资源。
        pass.set_bind_group(1, &self.offset, &[]);
        pass.set_index_buffer(
            self.bindings.quad_indices.slice(..),
            wgpu::IndexFormat::Uint16,
        );
        let (mut buffer, mut page, mut pipeline) = (None, None, None);
        for batch in &self.batches {
            if pipeline != Some(batch.pipeline) {
                pass.set_pipeline(self.bindings.pipeline(batch.pipeline));
                pipeline = Some(batch.pipeline);
            }
            if buffer != Some(batch.buffer) {
                pass.set_vertex_buffer(0, self.buffers[batch.buffer].slice(..));
                buffer = Some(batch.buffer);
            }
            if page != Some(batch.page) {
                pass.set_bind_group(0, self.bindings.page(batch.page), &[]);
                page = Some(batch.page);
            }
            pass.multi_draw_indexed_indirect(
                &self.arguments[batch.arguments],
                batch.offset,
                batch.count,
            );
        }
    }
}
