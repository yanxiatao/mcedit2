//! 在任何逐方块扫描前核对流式来源；只为变化的 patch 准备邻域数据。
use super::{MeshChunk, SpectatorScene};
use crate::{
    chunk_mesh_cache::{ChunkMeshCache, Style},
    spectator_stream::ChunkContext,
};
use mcedit2_world::surface::SurfaceVoxel;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

pub(super) struct Work {
    pub style: Style,
    pub cached: Vec<Option<Vec<MeshChunk>>>,
    pub needed: Vec<bool>,
    pub visible: HashSet<(i32, i32)>,
    pub has_distant: bool,
}
impl Work {
    pub fn prepare(
        view: &SpectatorScene,
        sources: &[&[SurfaceVoxel]],
        contexts: Option<&[ChunkContext]>,
        y_max: i32,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Self> {
        let style = ChunkMeshCache::style(
            view.texture_bank.as_ref(),
            view.show_textures,
            view.block_layers,
        );
        let mut work = Self {
            style,
            cached: vec![None; sources.len()],
            needed: vec![false; sources.len()],
            visible: HashSet::new(),
            has_distant: false,
        };
        let Some(contexts) = contexts else {
            work.needed.fill(true);
            for source in sources {
                for voxel in *source {
                    if cancelled() {
                        return None;
                    }
                    work.visible
                        .insert((voxel.x.div_euclid(16), voxel.z.div_euclid(16)));
                    work.has_distant |= voxel.span > 1;
                }
            }
            return Some(work);
        };
        if sources.len() != contexts.len() + 1 {
            log::error!(
                "[mesh] 流式来源数量与邻域证明不匹配：{} / {}",
                sources.len(),
                contexts.len()
            );
            return None;
        }
        let near = sources[0];
        work.needed[0] = true;
        if near.is_empty() {
            work.cached[0] = Some(Vec::new());
        }
        for voxel in near {
            if cancelled() {
                return None;
            }
            work.visible
                .insert((voxel.x.div_euclid(16), voxel.z.div_euclid(16)));
            work.has_distant |= voxel.span > 1;
        }
        {
            let mut cache = view
                .chunk_cache
                .lock()
                .expect("世界区块网格来源缓存锁被毒化");
            for (index, context) in contexts.iter().enumerate() {
                if cancelled() {
                    return None;
                }
                work.visible.insert(context.coordinates());
                work.cached[index + 1] = cache.get_patch(&work.style, context, y_max);
            }
        }
        let by_source = contexts
            .iter()
            .enumerate()
            .map(|(index, context)| (Arc::as_ptr(&context.source) as usize, index + 1))
            .collect::<HashMap<_, _>>();
        for (index, context) in contexts.iter().enumerate() {
            if cancelled() {
                return None;
            }
            if work.cached[index + 1].is_none() {
                work.needed[index + 1] = true;
                work.has_distant |= context.sampled_y_max(y_max).is_some();
                for neighbor in &context.neighbors {
                    let Some(&source) = by_source.get(&(Arc::as_ptr(neighbor) as usize)) else {
                        log::error!("[mesh] 区块邻域来源不在当前快照中");
                        return None;
                    };
                    work.needed[source] = true;
                }
            }
            // 近景可能截取一个区块的部分高度；它的液面仍依赖相邻完整来源。
            if !near.is_empty() && context.near.is_some() {
                work.needed[index + 1] = true;
            }
        }
        Some(work)
    }
}
