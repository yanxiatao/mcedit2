//! 透明源面按来源身份常驻；租约保护旧帧，槽位回收不改变任何活跃地址。
use super::upload::Budget;
use crate::world_mesh::WorldQuad;
use iced::wgpu;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, Weak,
    },
};

const STRIDE: usize = std::mem::size_of::<WorldQuad>();
// iced 的真实窗口设备只开放两个绑定组。组 1 同时携带原点 uniform 与透明来源，
// 不另建组 2；slab 从 binding 1 开始，binding 0 留给共享的 chunk_offset。
const OFFSET_BINDING: u32 = 8;
const SLOT_BITS: u32 = 29;
const SLOT_MASK: u32 = (1 << SLOT_BITS) - 1;
pub(super) const SHADER_MARKER: &str = "// MCEDIT2_TRANSPARENT_RESIDENCY";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Address {
    pub bank: usize,
    pub packed_face: u32,
}
/// 物理租约不移动；完整帧将实际使用的 slab 密集绑定到设备允许的槽位。
/// 每份映射随该帧固定，旧帧和镜头排序仍解释自己的地址，互不改写。
#[derive(Default)]
pub(super) struct Mapping {
    slots: BTreeMap<(usize, u32), Address>,
}
impl Mapping {
    pub fn translate(&self, physical: Address) -> Result<Address, String> {
        let slot = physical.packed_face >> SLOT_BITS;
        let logical = self
            .slots
            .get(&(physical.bank, slot))
            .ok_or("透明面引用了当前帧未绑定的物理 slab")?;
        Ok(Address {
            bank: logical.bank,
            packed_face: logical.packed_face | (physical.packed_face & SLOT_MASK),
        })
    }
}
pub(super) struct SnapshotBindings {
    pub groups: Vec<Option<wgpu::BindGroup>>,
    pub mapping: Arc<Mapping>,
}
#[derive(Clone)]
struct Region {
    ordinal: usize,
    bank: usize,
    slab: usize,
    first: usize,
    len: usize,
}
struct Growth {
    buffer: wgpu::Buffer,
    capacity: usize,
    copied: usize,
}
#[derive(Default)]
struct Slab {
    buffer: Option<wgpu::Buffer>,
    capacity: usize,
    high: usize,
    written: usize,
    used: usize,
    free: BTreeMap<usize, usize>,
    growth: Option<Growth>,
}
impl Slab {
    fn take(&mut self, wanted: usize, maximum: usize) -> Option<(usize, usize)> {
        let (first, len) = if let Some((&first, &available)) = self.free.first_key_value() {
            self.free.remove(&first);
            let len = wanted.min(available);
            if len < available {
                self.free.insert(first + len, available - len);
            }
            (first, len)
        } else {
            let len = wanted.min(maximum - self.high);
            if len == 0 {
                return None;
            }
            let first = self.high;
            self.high += len;
            (first, len)
        };
        self.used += len;
        Some((first, len))
    }
    fn release(&mut self, first: usize, len: usize) {
        self.used -= len;
        if self.used == 0 {
            *self = Self::default();
            return;
        }
        let mut start = first;
        let mut length = len;
        if let Some((&before, &count)) = self.free.range(..first).next_back() {
            if before + count == first {
                self.free.remove(&before);
                start = before;
                length += count;
            }
        }
        if let Some(count) = self.free.remove(&(start + length)) {
            length += count;
        }
        if start + length == self.high {
            self.high = start;
        } else {
            self.free.insert(start, length);
        }
        if self.high <= self.capacity {
            self.growth = None;
        }
    }
    /// 所有租约先完成地址规划，再一次分配所需容量；旧数据复制遵守同一帧预算。
    fn advance(
        &mut self,
        maximum: usize,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        budget: &mut Budget,
    ) -> Result<bool, String> {
        if self.high <= self.capacity {
            self.growth = None;
            return Ok(true);
        }
        if budget.exhausted() {
            return Ok(false);
        }
        if self.growth.is_none() {
            let capacity = self.high.next_power_of_two().min(maximum);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("MCEdit2 分帧扩容透明 slab"),
                size: (capacity * STRIDE) as u64,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            self.growth = Some(Growth {
                buffer,
                capacity,
                copied: 0,
            });
        }
        let growth = self.growth.as_mut().ok_or("透明缓冲扩容状态丢失")?;
        if growth.copied < self.written {
            if budget.capacity() < STRIDE {
                return Err("扩容预算不足以复制一个完整透明面".into());
            }
            while growth.copied < self.written && !budget.exhausted() {
                let count = (budget.allowance() / STRIDE).min(self.written - growth.copied);
                if count == 0 {
                    return Ok(false);
                }
                let source = self.buffer.as_ref().ok_or("透明扩容缺少已写入来源")?;
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("MCEdit2 透明 slab 有界扩容复制"),
                });
                let offset = (growth.copied * STRIDE) as u64;
                encoder.copy_buffer_to_buffer(
                    source,
                    offset,
                    &growth.buffer,
                    offset,
                    (count * STRIDE) as u64,
                );
                queue.submit([encoder.finish()]);
                growth.copied += count;
                budget.consume(count * STRIDE);
            }
            if growth.copied < self.written {
                return Ok(false);
            }
        }
        let growth = self.growth.take().ok_or("透明缓冲扩容产物丢失")?;
        self.buffer = Some(growth.buffer);
        self.capacity = growth.capacity;
        Ok(self.high <= self.capacity)
    }
}
struct Bank {
    slabs: Vec<Slab>,
}
impl Bank {
    fn new(width: usize) -> Self {
        Self {
            slabs: (0..width).map(|_| Slab::default()).collect(),
        }
    }
    fn empty(&self) -> bool {
        self.slabs.iter().all(|slab| slab.used == 0)
    }
}
struct State {
    banks: Vec<Bank>,
    sources: HashMap<usize, Weak<Allocation>>,
}
struct Inner {
    state: Mutex<State>,
    width: usize,
    maximum: usize,
    dummy: wgpu::Buffer,
}
pub(super) struct Allocation {
    owner: Arc<Inner>,
    source: Arc<Vec<WorldQuad>>,
    regions: Vec<Region>,
    count: usize,
    uploaded: AtomicUsize,
}
impl Allocation {
    pub fn len(&self) -> usize {
        self.count
    }
    pub fn uploaded(&self) -> usize {
        self.uploaded.load(Ordering::Acquire)
    }
    pub fn complete(&self) -> bool {
        self.uploaded() == self.count
    }
    pub fn same_source(&self, source: &Arc<Vec<WorldQuad>>) -> bool {
        Arc::ptr_eq(&self.source, source)
    }
    pub fn address(&self, ordinal: usize) -> Result<Address, String> {
        if ordinal >= self.count {
            return Err("透明面序号超出常驻租约".into());
        }
        let index = self
            .regions
            .partition_point(|region| region.ordinal <= ordinal)
            - 1;
        let region = &self.regions[index];
        let local = region.first + ordinal - region.ordinal;
        Ok(Address {
            bank: region.bank,
            packed_face: ((region.slab as u32) << SLOT_BITS) | local as u32,
        })
    }
}
impl Drop for Allocation {
    fn drop(&mut self) {
        let mut state = self
            .owner
            .state
            .lock()
            .expect("回收透明租约时 Arena 锁被毒化");
        release(&mut state, &self.regions);
        let key = Arc::as_ptr(&self.source) as usize;
        if state
            .sources
            .get(&key)
            .is_some_and(|cached| cached.as_ptr() == self as *const Self)
        {
            state.sources.remove(&key);
        }
    }
}
fn release(state: &mut State, regions: &[Region]) {
    for region in regions {
        state.banks[region.bank].slabs[region.slab].release(region.first, region.len);
    }
    while state.banks.last().is_some_and(Bank::empty) {
        state.banks.pop();
    }
}
pub(super) struct Arena {
    inner: Arc<Inner>,
}
pub(super) fn bank_width(device: &wgpu::Device) -> usize {
    let width = device
        .limits()
        .max_storage_buffers_per_shader_stage
        .saturating_sub(1)
        .min(7) as usize;
    assert!(width > 0, "设备必须至少提供两个 vertex storage binding");
    width
}
impl Arena {
    pub fn new(device: &wgpu::Device) -> Self {
        let limits = device.limits();
        let maximum = (limits
            .max_buffer_size
            .min(u64::from(limits.max_storage_buffer_binding_size))
            / STRIDE as u64)
            .min(u64::from(SLOT_MASK) + 1) as usize;
        assert!(maximum > 0, "设备必须能够存储一个完整透明面");
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    banks: Vec::new(),
                    sources: HashMap::new(),
                }),
                width: bank_width(device),
                maximum,
                dummy: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("MCEdit2 稀疏透明 bank 空绑定"),
                    size: STRIDE as u64,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                }),
            }),
        }
    }
    pub fn reserve(
        &self,
        source: Arc<Vec<WorldQuad>>,
        count: usize,
    ) -> Result<Arc<Allocation>, String> {
        if count > source.len() {
            return Err("透明常驻面数量超过来源几何".into());
        }
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|error| format!("透明常驻分配锁失败：{error}"))?;
        let key = Arc::as_ptr(&source) as usize;
        if let Some(existing) = state.sources.get(&key).and_then(Weak::upgrade) {
            if existing.count != count {
                drop(state);
                return Err("同一透明来源的面数量发生变化".into());
            }
            return Ok(existing);
        }
        let regions = self.allocate(&mut state, count, None)?;
        let allocation = Arc::new(Allocation {
            owner: Arc::clone(&self.inner),
            source,
            regions,
            count,
            uploaded: AtomicUsize::new(0),
        });
        state.sources.insert(key, Arc::downgrade(&allocation));
        Ok(allocation)
    }
    fn allocate(
        &self,
        state: &mut State,
        count: usize,
        only: Option<&mut BTreeSet<usize>>,
    ) -> Result<Vec<Region>, String> {
        let mut selected = only;
        let mut regions = Vec::new();
        let mut ordinal = 0;
        while ordinal < count {
            let mut found = None;
            for (bank_index, bank) in state.banks.iter_mut().enumerate() {
                if selected
                    .as_ref()
                    .is_some_and(|set| !set.contains(&bank_index))
                {
                    continue;
                }
                for (slab_index, slab) in bank.slabs.iter_mut().enumerate() {
                    if let Some((first, len)) = slab.take(count - ordinal, self.inner.maximum) {
                        found = Some(Region {
                            ordinal,
                            bank: bank_index,
                            slab: slab_index,
                            first,
                            len,
                        });
                        break;
                    }
                }
                if found.is_some() {
                    break;
                }
            }
            if let Some(region) = found {
                ordinal += region.len;
                regions.push(region);
                continue;
            }
            // 空 bank 编号可复用；仍被旧帧租约引用的 bank 不参与搬移目标。
            let bank = state
                .banks
                .iter()
                .position(Bank::empty)
                .unwrap_or(state.banks.len());
            if bank == state.banks.len() {
                if bank == usize::MAX {
                    release(state, &regions);
                    return Err("透明 bank 地址溢出".into());
                }
                state.banks.push(Bank::new(self.inner.width));
            }
            if let Some(set) = selected.as_mut() {
                set.insert(bank);
            }
        }
        Ok(regions)
    }
    /// 只推进尚未就绪的缓冲；旧绑定在整个扩容期间保持可用。
    pub fn advance(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        budget: &mut Budget,
    ) -> Result<bool, String> {
        advance_slabs(&self.inner, device, queue, budget)
    }
    pub fn upload(
        &self,
        allocation: &Arc<Allocation>,
        start: usize,
        quads: &[WorldQuad],
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<(), String> {
        if !Arc::ptr_eq(&allocation.owner, &self.inner) {
            return Err("透明上传租约属于其他 Arena".into());
        }
        let end = start.checked_add(quads.len()).ok_or("透明上传范围溢出")?;
        if start != allocation.uploaded() || end > allocation.count {
            return Err("透明上传必须从租约已完成游标连续推进".into());
        }
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|error| format!("透明上传锁失败：{error}"))?;
        for region in &allocation.regions {
            let lo = start.max(region.ordinal);
            let hi = end.min(region.ordinal + region.len);
            if lo >= hi {
                continue;
            }
            // 每次解析当前版本，不能写进 reserve 时捕获的扩容前缓冲。
            let slab = &mut state.banks[region.bank].slabs[region.slab];
            if slab.growth.is_some() || slab.high > slab.capacity {
                return Err("透明缓冲尚未完成准备，不能写入新几何".into());
            }
            let buffer = slab.buffer.as_ref().ok_or("透明租约缺少当前 slab")?;
            queue.write_buffer(
                buffer,
                ((region.first + lo - region.ordinal) * STRIDE) as u64,
                bytemuck::cast_slice(&quads[lo - start..hi - start]),
            );
            slab.written = slab.written.max(region.first + hi - region.ordinal);
        }
        allocation.uploaded.store(end, Ordering::Release);
        Ok(())
    }
    pub fn bindings(
        &self,
        allocations: &[Arc<Allocation>],
        offsets: &wgpu::Buffer,
        zero_offset: &wgpu::Buffer,
        layout: &wgpu::BindGroupLayout,
        device: &wgpu::Device,
    ) -> Result<SnapshotBindings, String> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|error| format!("透明快照绑定锁失败：{error}"))?;
        let mut slabs = BTreeSet::new();
        for allocation in allocations {
            if !Arc::ptr_eq(&allocation.owner, &self.inner) || !allocation.complete() {
                return Err("透明快照含未完成或外部租约".into());
            }
            slabs.extend(
                allocation
                    .regions
                    .iter()
                    .map(|region| (region.bank, region.slab)),
            );
        }
        let slabs = slabs.into_iter().collect::<Vec<_>>();
        let mut mapping = Mapping::default();
        let mut bindings = Vec::with_capacity(slabs.len().div_ceil(self.inner.width));
        for (logical_bank, group) in slabs.chunks(self.inner.width).enumerate() {
            let mut entries = group
                .iter()
                .enumerate()
                .map(|(slot, &(bank, slab))| {
                    mapping.slots.insert(
                        (bank, slab as u32),
                        Address {
                            bank: logical_bank,
                            packed_face: (slot as u32) << SLOT_BITS,
                        },
                    );
                    let slab = &state.banks[bank].slabs[slab];
                    if slab.growth.is_some() || slab.high > slab.capacity {
                        return Err("透明缓冲扩容未完成，不能发布新绑定".into());
                    }
                    let buffer = slab
                        .buffer
                        .as_ref()
                        .ok_or("完整透明租约引用了未分配的 slab 缓冲")?;
                    Ok(wgpu::BindGroupEntry {
                        binding: slot as u32 + 1,
                        resource: buffer.as_entire_binding(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            // 布局必须提供所有槽位；仅未使用的布局槽绑定同一个最小空缓冲。
            for slot in group.len()..self.inner.width {
                entries.push(wgpu::BindGroupEntry {
                    binding: slot as u32 + 1,
                    resource: self.inner.dummy.as_entire_binding(),
                });
            }
            entries.push(wgpu::BindGroupEntry {
                binding: OFFSET_BINDING,
                resource: offsets.as_entire_binding(),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: 0,
                resource: zero_offset.as_entire_binding(),
            });
            bindings.push(Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("MCEdit2 透明常驻 slab 密集帧快照"),
                layout,
                entries: &entries,
            })));
        }
        Ok(SnapshotBindings {
            groups: bindings,
            mapping: Arc::new(mapping),
        })
    }
    pub fn slab_count(allocations: &[Arc<Allocation>]) -> usize {
        allocations
            .iter()
            .flat_map(|allocation| {
                allocation
                    .regions
                    .iter()
                    .map(|region| (region.bank, region.slab))
            })
            .collect::<BTreeSet<_>>()
            .len()
    }
    pub fn clear(&self) {
        let mut state = self.inner.state.lock().expect("清理透明 Arena 时锁被毒化");
        state.sources.retain(|_, source| source.strong_count() != 0);
        while state.banks.last().is_some_and(Bank::empty) {
            state.banks.pop();
        }
        state.banks.shrink_to_fit();
        state.sources.shrink_to_fit();
    }
    pub fn compact(&self, allocations: &[Arc<Allocation>]) -> Result<Compaction, String> {
        let mut unique = BTreeMap::<usize, Arc<Allocation>>::new();
        for allocation in allocations {
            if !Arc::ptr_eq(&allocation.owner, &self.inner) || !allocation.complete() {
                return Err("压实仅接受本 Arena 的完整透明租约".into());
            }
            unique
                .entry(Arc::as_ptr(allocation) as usize)
                .or_insert_with(|| Arc::clone(allocation));
        }
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|error| format!("透明压实规划锁失败：{error}"))?;
        let slabs = unique
            .values()
            .flat_map(|allocation| {
                allocation
                    .regions
                    .iter()
                    .map(|region| (region.bank, region.slab))
            })
            .collect::<BTreeSet<_>>();
        let count = unique
            .values()
            .try_fold(0_usize, |total, allocation| {
                total.checked_add(allocation.count)
            })
            .ok_or("透明压实面数量溢出")?;
        let capacity = slabs
            .iter()
            .try_fold(0_usize, |total, &(bank, slab)| {
                total.checked_add(state.banks[bank].slabs[slab].capacity)
            })
            .ok_or("透明压实容量溢出")?;
        let banks = slabs
            .iter()
            .map(|&(bank, _)| bank)
            .collect::<BTreeSet<_>>()
            .len();
        let sparse = count > 0
            && (capacity > count.saturating_mul(2)
                || slabs.len() > count.div_ceil(self.inner.maximum).saturating_mul(2)
                || banks
                    > count
                        .div_ceil(self.inner.maximum * self.inner.width)
                        .saturating_mul(2));
        if !sparse {
            return Ok(Compaction {
                owner: Arc::clone(&self.inner),
                allocations: allocations.to_vec(),
                copies: Vec::new(),
                next: 0,
                changed: false,
            });
        }
        let mut selected = BTreeSet::new();
        let mut plans = Vec::with_capacity(unique.len());
        for old in unique.values() {
            match self.allocate(&mut state, old.count, Some(&mut selected)) {
                Ok(regions) => plans.push((Arc::clone(old), regions)),
                Err(error) => {
                    for (_, regions) in &plans {
                        release(&mut state, regions);
                    }
                    return Err(error);
                }
            }
        }
        drop(state);
        let mut replacements = HashMap::new();
        let mut copies = Vec::new();
        for (old, regions) in plans {
            let next = Arc::new(Allocation {
                owner: Arc::clone(&self.inner),
                source: Arc::clone(&old.source),
                regions,
                count: old.count,
                uploaded: AtomicUsize::new(0),
            });
            replacements.insert(Arc::as_ptr(&old) as usize, Arc::clone(&next));
            copies.push((old, next));
        }
        Ok(Compaction {
            owner: Arc::clone(&self.inner),
            allocations: allocations
                .iter()
                .map(|allocation| Arc::clone(&replacements[&(Arc::as_ptr(allocation) as usize)]))
                .collect(),
            copies,
            next: 0,
            changed: true,
        })
    }
}
fn advance_slabs(
    inner: &Inner,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    budget: &mut Budget,
) -> Result<bool, String> {
    let mut state = inner
        .state
        .lock()
        .map_err(|error| format!("透明缓冲准备锁失败：{error}"))?;
    for bank in &mut state.banks {
        for slab in &mut bank.slabs {
            if !slab.advance(inner.maximum, device, queue, budget)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
/// 压实只搬 GPU 数据；每次推进仍受同一帧的字节、传输和时间预算限制。
pub(super) struct Compaction {
    owner: Arc<Inner>,
    allocations: Vec<Arc<Allocation>>,
    copies: Vec<(Arc<Allocation>, Arc<Allocation>)>,
    next: usize,
    changed: bool,
}
impl Compaction {
    pub fn advance(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        budget: &mut Budget,
    ) -> Result<bool, String> {
        if !advance_slabs(&self.owner, device, queue, budget)? {
            return Ok(false);
        }
        if self.next < self.copies.len() && budget.capacity() < STRIDE {
            return Err("压实预算不足以复制一个完整透明面".into());
        }
        let mut state = self
            .owner
            .state
            .lock()
            .map_err(|error| format!("透明压实复制锁失败：{error}"))?;
        let mut encoder = None;
        let mut completed = Vec::new();
        while self.next < self.copies.len() && !budget.exhausted() {
            let (old, new) = &self.copies[self.next];
            let start = new.uploaded();
            if start == new.count {
                self.next += 1;
                continue;
            }
            let old_region = &old.regions[old
                .regions
                .partition_point(|region| region.ordinal <= start)
                - 1];
            let new_region = &new.regions[new
                .regions
                .partition_point(|region| region.ordinal <= start)
                - 1];
            let count = (budget.allowance() / STRIDE)
                .min(old_region.ordinal + old_region.len - start)
                .min(new_region.ordinal + new_region.len - start);
            if count == 0 {
                break;
            }
            let source = state.banks[old_region.bank].slabs[old_region.slab]
                .buffer
                .as_ref()
                .ok_or("压实来源 slab 不存在")?;
            let destination = state.banks[new_region.bank].slabs[new_region.slab]
                .buffer
                .as_ref()
                .ok_or("压实目标 slab 不存在")?;
            let commands = encoder.get_or_insert_with(|| {
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("MCEdit2 透明常驻分帧压实"),
                })
            });
            commands.copy_buffer_to_buffer(
                source,
                ((old_region.first + start - old_region.ordinal) * STRIDE) as u64,
                destination,
                ((new_region.first + start - new_region.ordinal) * STRIDE) as u64,
                (count * STRIDE) as u64,
            );
            let slab = &mut state.banks[new_region.bank].slabs[new_region.slab];
            slab.written = slab
                .written
                .max(new_region.first + start + count - new_region.ordinal);
            // 同一次调用只推进一个连续区间，提交之后才公布游标。
            completed.push((Arc::clone(new), start + count));
            budget.consume(count * STRIDE);
            if start + count == new.count {
                self.next += 1;
            } else {
                break;
            }
        }
        if let Some(encoder) = encoder {
            queue.submit([encoder.finish()]);
        }
        for (allocation, end) in completed {
            allocation.uploaded.store(end, Ordering::Release);
        }
        Ok(self.next == self.copies.len())
    }
    pub fn finish(self) -> Result<Vec<Arc<Allocation>>, String> {
        if self
            .allocations
            .iter()
            .any(|allocation| !allocation.complete())
        {
            return Err("透明压实尚未完成，不能发布替代地址".into());
        }
        if self.changed {
            let mut state = self
                .owner
                .state
                .lock()
                .map_err(|error| format!("透明压实交接锁失败：{error}"))?;
            for allocation in &self.allocations {
                state.sources.insert(
                    Arc::as_ptr(&allocation.source) as usize,
                    Arc::downgrade(allocation),
                );
            }
        }
        Ok(self.allocations)
    }
}
pub(super) fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let mut entries = (0..bank_width(device))
        .map(|index| wgpu::BindGroupLayoutEntry {
            binding: index as u32 + 1,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(STRIDE as u64),
            },
            count: None,
        })
        .collect::<Vec<_>>();
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: OFFSET_BINDING,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(16),
        },
        count: None,
    });
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(16),
        },
        count: None,
    });
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("MCEdit2 透明 bank 与帧偏移布局"),
        entries: &entries,
    })
}
pub(super) fn shader(template: &str, device: &wgpu::Device) -> String {
    let width = bank_width(device);
    let mut source = String::new();
    for slab in 0..width {
        let binding = slab + 1;
        source.push_str(&format!(
            "@group(1) @binding({binding}) var<storage,read> transparent_slab_{slab}:array<u32>;\n"
        ));
    }
    source.push_str(&format!("@group(1) @binding({OFFSET_BINDING}) var<storage,read> transparent_offsets:array<vec4<f32>>;\n"));
    source.push_str("struct TransparentCorner {position:vec3<f32>,anchor:vec3<f32>,uv:vec2<f32>,rgba:vec4<f32>,mode:u32,atlas_page:u32,light:u32};\nfn transparent_corner(face:u32,corner:u32)->TransparentCorner {let base=(face&536870911u)*MCEDIT2_SOURCE_WORDS;let p=base+corner*3u;let uv=base+12u+corner*2u;let c=base+20u+corner*4u;var out:TransparentCorner;switch(face>>29u){\n");
    // 每角只选择一次 slab，避免每个属性字重复执行动态选择。
    for slab in 0..width {
        source.push_str(&format!(
            "case {slab}u:{{out.position=bitcast<vec3<f32>>(vec3<u32>(transparent_slab_{slab}[p],transparent_slab_{slab}[p+1u],transparent_slab_{slab}[p+2u]));out.uv=bitcast<vec2<f32>>(vec2<u32>(transparent_slab_{slab}[uv],transparent_slab_{slab}[uv+1u]));out.rgba=bitcast<vec4<f32>>(vec4<u32>(transparent_slab_{slab}[c],transparent_slab_{slab}[c+1u],transparent_slab_{slab}[c+2u],transparent_slab_{slab}[c+3u]));out.mode=transparent_slab_{slab}[base+36u];out.atlas_page=transparent_slab_{slab}[base+37u];out.light=transparent_slab_{slab}[base+38u];if((out.mode&262144u)!=0u){{out.anchor=bitcast<vec3<f32>>(vec3<u32>(transparent_slab_{slab}[base+9u],transparent_slab_{slab}[base+10u],transparent_slab_{slab}[base+11u]));if(corner==3u){{out.position=bitcast<vec3<f32>>(vec3<u32>(transparent_slab_{slab}[base],transparent_slab_{slab}[base+1u],transparent_slab_{slab}[base+2u]))+bitcast<vec3<f32>>(vec3<u32>(transparent_slab_{slab}[base+6u],transparent_slab_{slab}[base+7u],transparent_slab_{slab}[base+8u]))-bitcast<vec3<f32>>(vec3<u32>(transparent_slab_{slab}[base+3u],transparent_slab_{slab}[base+4u],transparent_slab_{slab}[base+5u]));}}}}}}\n"
        ));
    }
    source.push_str("default:{}\n}return out;}\n");
    template.replace(
        SHADER_MARKER,
        &source.replace("MCEDIT2_SOURCE_WORDS", &format!("{}u", STRIDE / 4)),
    )
}
