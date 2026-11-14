//! 按实际所有权统计源体素缓冲与共享载荷；不保活源，不代表进程 RSS 或分配器开销。
use crate::{sections::State, surface::SurfaceVoxel};
use std::{
    collections::{HashMap, HashSet},
    mem,
    sync::Arc,
};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Usage {
    pub voxels: usize,
    pub inline_bytes: usize,
    pub state_bytes: usize,
    pub biome_bytes: usize,
    pub fluid_bytes: usize,
    pub descriptor_bytes: usize,
    pub unique_states: usize,
    pub unique_biomes: usize,
    pub unique_fluids: usize,
    pub unique_descriptors: usize,
}
impl Usage {
    pub fn shared_bytes(self) -> usize {
        self.state_bytes + self.biome_bytes + self.fluid_bytes + self.descriptor_bytes
    }
    pub fn total_bytes(self) -> usize {
        self.inline_bytes + self.shared_bytes()
    }
}
/// 一次统计中对同一 Arc 只收费一次；缓冲容量由实际拥有 Vec 的调用方提供。
#[derive(Default)]
pub struct Footprint {
    states: HashSet<usize>,
    biomes: HashSet<usize>,
    fluids: HashSet<usize>,
    descriptors: HashSet<usize>,
    usage: Usage,
    allocations: Allocations,
}
/// 读取线程冻结的去重分配列表；不再为长期缓存保留构建用 HashSet。
#[derive(Default)]
pub struct Allocations {
    states: Vec<(usize, usize)>,
    biomes: Vec<(usize, usize)>,
    fluids: Vec<(usize, usize)>,
    descriptors: Vec<(usize, usize)>,
    usage: Usage,
}
impl Allocations {
    pub fn usage(&self) -> Usage {
        self.usage
    }
    /// 分配列表的堆存储；对象本体由持有方计算。
    pub fn metadata_bytes(&self) -> usize {
        (self.states.capacity()
            + self.biomes.capacity()
            + self.fluids.capacity()
            + self.descriptors.capacity())
            * mem::size_of::<(usize, usize)>()
    }
}
impl std::fmt::Debug for Allocations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Allocations").field(&self.usage).finish()
    }
}
/// 活动缓冲的共享分配账本；加入、替换和释放按同一分配身份计费。
#[derive(Default)]
pub struct Ledger {
    references: HashMap<(u8, usize), (usize, usize)>,
    usage: Usage,
}
impl Ledger {
    pub fn usage(&self) -> Usage {
        self.usage
    }
    /// 表自身的结构估算，和源载荷分开统计。
    pub fn metadata_bytes(&self) -> usize {
        mem::size_of::<Self>()
            + self.references.capacity() * (mem::size_of::<((u8, usize), (usize, usize))>() + 1)
    }
    fn shared(&mut self, kind: usize, bytes: usize, add: bool) {
        let (cost, count) = match kind {
            0 => (&mut self.usage.state_bytes, &mut self.usage.unique_states),
            1 => (&mut self.usage.biome_bytes, &mut self.usage.unique_biomes),
            2 => (&mut self.usage.fluid_bytes, &mut self.usage.unique_fluids),
            3 => (
                &mut self.usage.descriptor_bytes,
                &mut self.usage.unique_descriptors,
            ),
            _ => unreachable!("四类共享载荷"),
        };
        if add {
            *cost += bytes;
            *count += 1;
        } else {
            *cost -= bytes;
            *count -= 1;
        }
    }
    pub fn add(&mut self, allocations: &Allocations) {
        self.usage.inline_bytes += allocations.usage.inline_bytes;
        self.usage.voxels += allocations.usage.voxels;
        for (kind, entries) in [
            &allocations.states,
            &allocations.biomes,
            &allocations.fluids,
            &allocations.descriptors,
        ]
        .into_iter()
        .enumerate()
        {
            for &(id, bytes) in entries {
                let entry = self.references.entry((kind as u8, id)).or_default();
                if entry.0 == 0 {
                    *entry = (1, bytes);
                    self.shared(kind, bytes, true);
                } else {
                    assert_eq!(entry.1, bytes, "存活的共享分配大小不能改变");
                    entry.0 += 1;
                }
            }
        }
    }
    pub fn remove(&mut self, allocations: &Allocations) {
        self.usage.inline_bytes -= allocations.usage.inline_bytes;
        self.usage.voxels -= allocations.usage.voxels;
        for (kind, entries) in [
            &allocations.states,
            &allocations.biomes,
            &allocations.fluids,
            &allocations.descriptors,
        ]
        .into_iter()
        .enumerate()
        {
            for &(id, bytes) in entries {
                let key = (kind as u8, id);
                let entry = self
                    .references
                    .get_mut(&key)
                    .expect("释放的源分配必须已加入账本");
                assert_eq!(entry.1, bytes, "释放的共享分配大小不能改变");
                entry.0 -= 1;
                if entry.0 == 0 {
                    self.references.remove(&key);
                    self.shared(kind, bytes, false);
                }
            }
        }
        if self.references.is_empty() {
            self.references = HashMap::new();
        }
    }
}
fn arc_bytes<T>(payload: usize) -> usize {
    let alignment = mem::align_of::<usize>().max(mem::align_of::<T>());
    (2 * mem::size_of::<usize>() + payload).div_ceil(alignment) * alignment
}
impl Footprint {
    pub fn seal(mut self) -> Allocations {
        self.allocations.usage = self.usage;
        self.allocations
    }
    pub fn usage(&self) -> Usage {
        self.usage
    }
    pub fn shared_state(&mut self, state: &Arc<State>) {
        if self.states.insert(Arc::as_ptr(state) as usize) {
            self.usage.unique_states += 1;
            let bytes = arc_bytes::<State>(mem::size_of::<State>())
                + state.name.capacity()
                + state.properties.capacity() * mem::size_of::<(String, String)>()
                + state
                    .properties
                    .iter()
                    .map(|(key, value)| key.capacity() + value.capacity())
                    .sum::<usize>();
            self.usage.state_bytes += bytes;
            self.allocations
                .states
                .push((Arc::as_ptr(state) as usize, bytes));
        }
    }
    /// capacity 与切片必须来自同一个完整源缓冲；共享载荷跨缓冲仍只统计一次。
    pub fn include(&mut self, voxels: &[SurfaceVoxel], capacity: usize) {
        self.usage.voxels += voxels.len();
        self.usage.inline_bytes += capacity * mem::size_of::<SurfaceVoxel>();
        for voxel in voxels {
            let descriptor = voxel.descriptor_id();
            if !self.descriptors.insert(descriptor) {
                continue;
            }
            let bytes =
                arc_bytes::<crate::surface::VoxelData>(mem::size_of::<crate::surface::VoxelData>());
            self.usage.unique_descriptors += 1;
            self.usage.descriptor_bytes += bytes;
            self.allocations.descriptors.push((descriptor, bytes));
            self.shared_state(&voxel.state);
            if let Some(biome) = &voxel.biome {
                if self.biomes.insert(biome.allocation_id()) {
                    self.usage.unique_biomes += 1;
                    let bytes = biome.allocation_bytes();
                    self.usage.biome_bytes += bytes;
                    self.allocations.biomes.push((biome.allocation_id(), bytes));
                }
            }
            if let Some(fluid) = &voxel.fluid {
                if self.fluids.insert(Arc::as_ptr(fluid) as usize) {
                    self.usage.unique_fluids += 1;
                    let bytes = arc_bytes::<crate::fluid_surface::FluidSurface>(mem::size_of_val(
                        fluid.as_ref(),
                    ));
                    self.usage.fluid_bytes += bytes;
                    self.allocations
                        .fluids
                        .push((Arc::as_ptr(fluid) as usize, bytes));
                }
            }
        }
    }
}
