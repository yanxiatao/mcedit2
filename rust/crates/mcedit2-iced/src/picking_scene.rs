//! 不可变场景的区块索引；世界、实体和编辑预览共用拾取、范围与交接证明。
use super::{branch, Bounds, Kind, Node, Ray, WorldCamera};
use crate::spectator::MeshChunk;
use iced::{Point, Size};
use std::{
    collections::HashMap,
    ops::Deref,
    sync::{Arc, Mutex, Weak},
};

type Column = (i32, i32, u8);

/// 冻结区块列表与其索引一起发布；不提供可变借用，避免改几何后沿用旧索引。
#[derive(Clone, Debug)]
pub(crate) struct Chunks(Arc<Data>);

#[derive(Debug)]
struct Data {
    items: Vec<MeshChunk>,
    layers: [u64; 4],
    bounds: Option<Bounds>,
    query: Query,
    /// 只保留弱身份；逐帧交接判断不能保活上一整个世界。
    coverage: Mutex<Option<(Weak<Data>, bool)>>,
}

#[derive(Debug)]
enum Query {
    Indexed {
        bounds: Vec<Bounds>,
        ids: Vec<usize>,
        order: Vec<usize>,
        nodes: Vec<Node>,
        columns: HashMap<Column, Bounds>,
    },
    Joined {
        base: Chunks,
        overlay: Chunks,
    },
}

impl Default for Chunks {
    fn default() -> Self {
        Vec::new().into()
    }
}
impl From<Vec<MeshChunk>> for Chunks {
    /// 来源构建线程在发布前冻结数据；这里只读区块根包围盒，不遍历面或源方块。
    fn from(items: Vec<MeshChunk>) -> Self {
        let mut bounds = Vec::with_capacity(items.len());
        let mut ids = Vec::with_capacity(items.len());
        let mut columns = HashMap::<Column, Bounds>::new();
        let mut layers = [0; 4];
        for (id, chunk) in items.iter().enumerate() {
            let layer = usize::from(chunk.key.2);
            layers[layer / 64] |= 1u64 << (layer % 64);
            let Some(root) = chunk.picking.nodes.first() else {
                continue;
            };
            let origin = chunk.picking.origin;
            let volume = Bounds {
                min: std::array::from_fn(|i| root.bounds.min[i] + f64::from(origin[i])),
                max: std::array::from_fn(|i| root.bounds.max[i] + f64::from(origin[i])),
                view_offset_z: root.bounds.view_offset_z,
            };
            bounds.push(volume);
            ids.push(id);
            columns
                .entry((chunk.key.0, chunk.key.1, chunk.key.2))
                .and_modify(|old| *old = old.union(volume))
                .or_insert(volume);
        }
        let mut order = (0..bounds.len()).collect::<Vec<_>>();
        let mut nodes = Vec::new();
        if !bounds.is_empty() {
            branch(&mut order, &mut nodes, &bounds, 0, bounds.len());
        }
        let volume = nodes.first().map(|node| node.bounds);
        Self(Arc::new(Data {
            items,
            layers,
            bounds: volume,
            query: Query::Indexed {
                bounds,
                ids,
                order,
                nodes,
                columns,
            },
            coverage: Mutex::new(None),
        }))
    }
}
impl Deref for Chunks {
    type Target = [MeshChunk];
    fn deref(&self) -> &Self::Target {
        &self.0.items
    }
}
impl<'a> IntoIterator for &'a Chunks {
    type Item = &'a MeshChunk;
    type IntoIter = std::slice::Iter<'a, MeshChunk>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Chunks {
    /// 仅比较绘制所需的有序几何来源；不会把拾取证明或实体元数据视为已经更新。
    /// 合成索引先比较两侧根身份，未变化的地形不逐区块扫描。
    pub(crate) fn same_geometry(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        match (&self.0.query, &other.0.query) {
            (
                Query::Joined {
                    base: a,
                    overlay: b,
                },
                Query::Joined {
                    base: c,
                    overlay: d,
                },
            ) => a.same_geometry(c) && b.same_geometry(d),
            _ => {
                self.len() == other.len()
                    && self.iter().zip(other.iter()).all(|(a, b)| {
                        a.key == b.key
                            && a.origin == b.origin
                            && Arc::ptr_eq(&a.quads(), &b.quads())
                    })
            }
        }
    }
    /// 合成只追加引用和索引根；不在事件线程重建整幅地形的 BVH 或列范围表。
    pub(crate) fn join(&self, overlay: &Self) -> Self {
        if self.is_empty() {
            return overlay.clone();
        }
        if overlay.is_empty() {
            return self.clone();
        }
        let mut items = Vec::with_capacity(self.len() + overlay.len());
        items.extend(self.iter().cloned());
        items.extend(overlay.iter().cloned());
        let bounds = match (self.0.bounds, overlay.0.bounds) {
            (Some(a), Some(b)) => Some(a.union(b)),
            (a, b) => a.or(b),
        };
        Self(Arc::new(Data {
            items,
            layers: std::array::from_fn(|i| self.0.layers[i] | overlay.0.layers[i]),
            bounds,
            query: Query::Joined {
                base: self.clone(),
                overlay: overlay.clone(),
            },
            coverage: Mutex::new(None),
        }))
    }

    pub(crate) fn world_bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        self.0.bounds.map(|b| (b.min, b.max))
    }

    pub(crate) fn contains_layer(&self, layer: u8) -> bool {
        let layer = usize::from(layer);
        self.0.layers[layer / 64] & (1u64 << (layer % 64)) != 0
    }

    fn column(&self, key: Column) -> Option<Bounds> {
        match &self.0.query {
            Query::Indexed { columns, .. } => columns.get(&key).copied(),
            Query::Joined { base, overlay } => match (base.column(key), overlay.column(key)) {
                (Some(a), Some(b)) => Some(a.union(b)),
                (a, b) => a.or(b),
            },
        }
    }

    pub(crate) fn covers(&self, previous: &Self) -> bool {
        let mut cache = self.0.coverage.lock().expect("场景拾取覆盖缓存锁被毒化");
        if let Some((identity, result)) = cache.as_ref() {
            if identity.ptr_eq(&Arc::downgrade(&previous.0)) {
                return *result;
            }
        }
        let covered = previous.iter().all(|chunk| {
            let Some((minimum, maximum)) = chunk.picking.world_bounds() else {
                return false;
            };
            self.column((chunk.key.0, chunk.key.1, chunk.key.2))
                .is_some_and(|bounds| {
                    (0..3).all(|i| {
                        bounds.min[i] <= minimum[i] + 1e-5 && bounds.max[i] >= maximum[i] - 1e-5
                    })
                })
        });
        *cache = Some((Arc::downgrade(&previous.0), covered));
        covered
    }

    fn coarse(bounds: Bounds, ray: Ray) -> bool {
        // 世界基底和区块基底的 f64 运算次序略异；粗筛扩大边界，最后仍用原区块求交。
        let padded = Bounds {
            min: bounds.min.map(|v| v - 1e-6),
            max: bounds.max.map(|v| v + 1e-6),
            ..bounds
        };
        padded.enter(ray, f64::INFINITY).is_some()
    }

    fn collect(&self, ray: Ray, offset: usize, result: &mut Vec<usize>) {
        if self
            .0
            .bounds
            .is_none_or(|bounds| !Self::coarse(bounds, ray))
        {
            return;
        }
        match &self.0.query {
            Query::Joined { base, overlay } => {
                base.collect(ray, offset, result);
                overlay.collect(ray, offset + base.len(), result);
            }
            Query::Indexed {
                bounds,
                ids,
                order,
                nodes,
                ..
            } => {
                let mut stack = Vec::with_capacity(32);
                stack.push(0);
                while let Some(id) = stack.pop() {
                    let node = &nodes[id];
                    if !Self::coarse(node.bounds, ray) {
                        continue;
                    }
                    match &node.kind {
                        Kind::Branch(a, b) => {
                            stack.push(*b);
                            stack.push(*a);
                        }
                        Kind::Leaf(range) => {
                            for &id in &order[range.clone()] {
                                if Self::coarse(bounds[id], ray) {
                                    result.push(offset + ids[id]);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn candidates(
        &self,
        camera: WorldCamera,
        point: Point,
        size: Size,
    ) -> Vec<(f64, usize)> {
        let Some(ray) = Ray::new(camera, [0; 3], point, size) else {
            return Vec::new();
        };
        let mut ids = Vec::new();
        self.collect(ray, 0, &mut ids);
        let mut result = ids
            .into_iter()
            .filter_map(|index| {
                self[index]
                    .picking
                    .enter(camera, point, size, f64::INFINITY)
                    .map(|near| (near, index))
            })
            .collect::<Vec<_>>();
        // BVH 遍历顺序不是绘制顺序；等深度候选仍严格按原区块序排列。
        result.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        result
    }
}
