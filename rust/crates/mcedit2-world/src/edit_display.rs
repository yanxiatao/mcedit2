//! 显示使用当前磁盘派生数据；原始字节和几何来源继续承担历史重放证明。
use super::*;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Completion {
    /// 外层 None 表示没有显式差异，内层 None 表示字段确实被移除。
    pub ready: Option<Option<bool>>,
    pub status: Option<Option<String>>,
}
impl Session {
    pub(super) fn changed_derived_regions(
        &self,
        archive: &crate::history_source::Archive,
    ) -> Result<BTreeSet<ChunkPos>, WorldError> {
        let dimension = self
            .world
            .dimensions
            .iter()
            .find(|dimension| dimension.name == self.dimension)
            .ok_or_else(|| refuse("派生数据显示来源缺少当前维度"))?;
        let mut regions = BTreeSet::new();
        for path in archive.changed_paths_since(self.origin.as_ref()) {
            if dimension.region_dir == self.world.root
                && crate::alpha::is_chunk_path(&self.world.root, path)
            {
                let chunk = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(crate::alpha::chunk_coordinates)
                    .expect("Alpha 路径已核对");
                regions.insert((chunk.0.div_euclid(32), chunk.1.div_euclid(32)));
                continue;
            }
            if path.parent() != Some(dimension.region_dir.as_path()) {
                continue;
            }
            if crate::is_region_filename(path) {
                let mut parts = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .expect("Region 文件名已核对")
                    .split('.')
                    .skip(1);
                let x = parts
                    .next()
                    .expect("Region X 已核对")
                    .parse::<i32>()
                    .expect("Region X 已核对");
                let z = parts
                    .next()
                    .expect("Region Z 已核对")
                    .parse::<i32>()
                    .expect("Region Z 已核对");
                for coordinate in [x, z] {
                    coordinate
                        .checked_mul(32)
                        .and_then(|value| value.checked_add(31))
                        .ok_or_else(|| refuse("派生数据显示来源的 Region 坐标超出编辑范围"))?;
                }
                regions.insert((x, z));
            } else if let Some(name) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix("c."))
                .and_then(|name| name.strip_suffix(".mcc"))
            {
                let (x, z) = name
                    .split_once('.')
                    .ok_or_else(|| refuse("派生数据显示来源的 MCC 文件名无效"))?;
                let x = x
                    .parse::<i32>()
                    .map_err(|_| refuse("派生数据显示来源的 MCC X 无效"))?;
                let z = z
                    .parse::<i32>()
                    .map_err(|_| refuse("派生数据显示来源的 MCC Z 无效"))?;
                regions.insert((x.div_euclid(32), z.div_euclid(32)));
            }
        }
        Ok(regions)
    }
    pub(super) fn needs_display_projection(&self, chunk: ChunkPos) -> bool {
        self.origin.is_some()
            || self.creation.contains_key(&chunk)
            || self.be_overlay.contains_key(&chunk)
            || self.display_light_pending(chunk)
    }
    fn display_light_pending(&self, chunk: ChunkPos) -> bool {
        (-1..=1).any(|dx| {
            (-1..=1).any(|dz| {
                match (chunk.0.checked_add(dx), chunk.1.checked_add(dz)) {
                    (Some(x), Some(z)) => self.pending_geometry.contains(&(x, z)),
                    _ => false, // 坐标范围之外没有可寻址的相邻区块。
                }
            })
        })
    }
    pub(super) fn project_display_chunk(
        &self,
        coordinate: ChunkPos,
        source: Arc<Chunk>,
        current: Option<Arc<Chunk>>,
    ) -> Result<Arc<Chunk>, WorldError> {
        let pending = self.pending_geometry.contains(&coordinate);
        if self.origin.is_some() && current.is_none() && !pending {
            return Err(refuse(
                "显示区块的实际磁盘槽已缺失，且当前没有恢复该几何的编辑",
            ));
        }
        if let Some(current) = &current {
            if source.layout != current.layout || source.data_version != current.data_version {
                return Err(refuse("显示区块的历史与当前派生数据格式不同"));
            }
        }
        let completion = self.display_completion(coordinate, &source)?;
        // 当前尚不存在的新槽或恢复槽只有已核实历史中的数组，保持待重算状态。
        let light = current
            .as_ref()
            .filter(|_| !self.region_base_changed(coordinate))
            .map_or(&source.light, |current| &current.light)
            .for_display(
                &source.layout,
                source.data_version,
                completion.ready,
                completion.status,
                self.display_light_pending(coordinate),
            );
        let mut source = Arc::unwrap_or_clone(source);
        source.light = light;
        Ok(Arc::new(source))
    }
}
