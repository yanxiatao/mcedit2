//! 原版射线实体列表：有限前向射线、真实存储槽和完整实体树，不依赖地形遮挡。
use super::*;

#[derive(Debug, Clone, Copy)]
pub struct Ray {
    pub origin: [f64; 3],
    pub direction: [f64; 3],
    pub distance: f64,
    pub width: f64,
}
impl Ray {
    pub fn validated(mut self) -> Result<Self, WorldError> {
        let length = self.direction[0]
            .hypot(self.direction[1])
            .hypot(self.direction[2]);
        if !self.origin.into_iter().all(f64::is_finite)
            || !length.is_finite()
            || length == 0.
            || !self.distance.is_finite()
            || self.distance <= 0.
            || !self.width.is_finite()
            || self.width <= 0.
        {
            return Err(refuse("实体射线需要有限坐标、非零方向、正距离及正半径"));
        }
        self.direction = self.direction.map(|value| value / length);
        if (0..3)
            .any(|axis| !(self.origin[axis] + self.direction[axis] * self.distance).is_finite())
        {
            return Err(refuse("实体射线终点不是有限坐标"));
        }
        for axis in [0, 2] {
            let end = self.origin[axis] + self.direction[axis] * self.distance;
            if self.origin[axis].min(end) - self.width <= -f64::from(MAX_COORD)
                || self.origin[axis].max(end) + self.width >= f64::from(MAX_COORD)
            {
                return Err(refuse("实体射线超出支持的 X/Z 坐标范围"));
            }
        }
        Ok(self)
    }
    /// 与原版实体工具一致，围绕保存的 Pos 向上半格采样；不编造实体眼位或碰撞箱。
    fn measure(self, position: [f64; 3]) -> Option<(f64, f64)> {
        let delta = std::array::from_fn::<_, 3, _>(|axis| {
            let center = if axis == 1 { 0.5 } else { 0. };
            position[axis] + center - self.origin[axis]
        });
        let distance = (0..3)
            .map(|axis| delta[axis] * self.direction[axis])
            .sum::<f64>();
        if distance < 0. || distance > self.distance {
            return None;
        }
        let lateral = (0..3)
            .map(|axis| (delta[axis] - distance * self.direction[axis]).powi(2))
            .sum::<f64>()
            .sqrt();
        (lateral < self.width).then_some((distance, lateral))
    }
    fn crosses(self, chunk: ChunkPos) -> bool {
        let mut near: f64 = 0.;
        let mut far = self.distance;
        for (axis, coordinate) in [(0, chunk.0), (2, chunk.1)] {
            let lo = f64::from(coordinate) * 16. - self.width;
            let hi = f64::from(coordinate) * 16. + 16. + self.width;
            let direction = self.direction[axis];
            if direction == 0. {
                if self.origin[axis] < lo || self.origin[axis] > hi {
                    return false;
                }
            } else {
                let a = (lo - self.origin[axis]) / direction;
                let b = (hi - self.origin[axis]) / direction;
                near = near.max(a.min(b));
                far = far.min(a.max(b));
                if near > far {
                    return false;
                }
            }
        }
        true
    }
    fn chunks(
        self,
        budget: EditWorkBudget,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<BTreeSet<ChunkPos>, WorldError> {
        let mut cell = [
            (self.origin[0] / 16.).floor() as i32,
            (self.origin[2] / 16.).floor() as i32,
        ];
        let step = [0, 2].map(|axis| {
            if self.direction[axis] > 0. {
                1
            } else if self.direction[axis] < 0. {
                -1
            } else {
                0
            }
        });
        let mut next = std::array::from_fn::<_, 2, _>(|i| {
            let axis = [0, 2][i];
            if step[i] == 0 {
                f64::INFINITY
            } else {
                (f64::from(cell[i] + i32::from(step[i] > 0)) * 16. - self.origin[axis])
                    / self.direction[axis]
            }
        });
        let stride = [0, 2].map(|axis| {
            if self.direction[axis] == 0. {
                f64::INFINITY
            } else {
                16. / self.direction[axis].abs()
            }
        });
        let padding = (self.width / 16.).ceil() as i32;
        let side =
            usize::try_from(i64::from(padding) * 2 + 1).map_err(|_| refuse("射线邻域尺寸溢出"))?;
        budget.check(
            side.checked_mul(side)
                .and_then(|n| n.checked_mul(64))
                .ok_or_else(|| refuse("射线邻域工作集溢出"))?,
        )?;
        let mut result = BTreeSet::new();
        loop {
            for x in cell[0] - padding..=cell[0] + padding {
                for z in cell[1] - padding..=cell[1] + padding {
                    stop(cancelled)?;
                    if self.crosses((x, z)) {
                        result.insert((x, z));
                        budget.check(
                            result
                                .len()
                                .checked_mul(64)
                                .ok_or_else(|| refuse("射线槽目录溢出"))?,
                        )?;
                    }
                }
            }
            if next[0].min(next[1]) > self.distance {
                break;
            }
            let advance = [next[0] <= next[1], next[1] <= next[0]];
            for axis in 0..2 {
                if advance[axis] {
                    cell[axis] += step[axis];
                    next[axis] += stride[axis];
                }
            }
        }
        Ok(result)
    }
}
fn stop(cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
    if cancelled() {
        Err(refuse("射线实体查询已取消"))
    } else {
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct Entity {
    pub id: Option<String>,
    pub position: [f64; 3],
    pub distance: f64,
    pub lateral: f64,
    pub sha256: String,
}
#[derive(Debug, Clone)]
pub struct WorldHit {
    pub target: ObjectTarget,
    pub entity: Entity,
}
#[derive(Debug, Clone)]
pub struct DocumentHit {
    pub root: usize,
    pub path: String,
    pub entity: Entity,
}
#[derive(Debug, Clone)]
pub struct Found<T> {
    pub ray: Ray,
    pub hits: Vec<T>,
    pub chunks: usize,
    pub absent_chunks: usize,
    pub unpositioned: usize,
}
impl Session {
    pub fn entities_on_ray_cancellable(
        &self,
        ray: Ray,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Found<WorldHit>, WorldError> {
        let ray = ray.validated()?;
        let chunks = ray.chunks(budget, &mut cancelled)?;
        let enumeration = chunks
            .len()
            .checked_mul(64)
            .ok_or_else(|| refuse("实体射线槽目录溢出"))?;
        let mut found = Found {
            ray,
            hits: Vec::new(),
            chunks: chunks.len(),
            absent_chunks: 0,
            unpositioned: 0,
        };
        let mut retained = enumeration;
        for chunk in chunks {
            stop(&mut cancelled)?;
            let catalog = if let Some(raw) = self.entity_overlay.get(&chunk) {
                budget.check(
                    retained
                        .checked_add(
                            raw.len()
                                .checked_mul(8)
                                .ok_or_else(|| refuse("实体源工作集溢出"))?,
                        )
                        .ok_or_else(|| refuse("实体源工作集溢出"))?,
                )?;
                self.inspect_entities(chunk.0, chunk.1)?
            } else {
                let Some((source, raw)) =
                    crate::entity_catalog::source(&self.world, &self.dimension, chunk.0, chunk.1)?
                else {
                    found.absent_chunks += 1;
                    continue;
                };
                budget.check(
                    retained
                        .checked_add(
                            raw.len()
                                .checked_mul(8)
                                .ok_or_else(|| refuse("实体源工作集溢出"))?,
                        )
                        .ok_or_else(|| refuse("实体源工作集溢出"))?,
                )?;
                crate::entity_catalog::parse(&raw, &source, chunk.0, chunk.1)?
            };
            for entity in catalog.entities {
                stop(&mut cancelled)?;
                let Some(position) = entity.pos else {
                    found.unpositioned += 1;
                    continue;
                };
                let Some((distance, lateral)) = ray.measure(position) else {
                    continue;
                };
                retained = retained
                    .checked_add(
                        256 + entity.path.len() + entity.id.as_ref().map_or(0, String::len),
                    )
                    .ok_or_else(|| refuse("射线结果工作集溢出"))?;
                budget.check(retained)?;
                found.hits.push(WorldHit {
                    target: ObjectTarget::Entity {
                        chunk,
                        path: entity.path,
                    },
                    entity: Entity {
                        id: entity.id,
                        position,
                        distance,
                        lateral,
                        sha256: entity.sha256,
                    },
                });
            }
        }
        found.hits.sort_by(|a, b| {
            a.entity
                .distance
                .total_cmp(&b.entity.distance)
                .then_with(|| a.target.cmp(&b.target))
        });
        stop(&mut cancelled)?;
        Ok(found)
    }
}
impl crate::structure_document::Document {
    pub fn entities_on_ray_cancellable(
        &self,
        ray: Ray,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Found<DocumentHit>, WorldError> {
        let ray = ray.validated()?;
        let content = self.content();
        let mut found = Found {
            ray,
            hits: Vec::new(),
            chunks: 0,
            absent_chunks: 0,
            unpositioned: 0,
        };
        let mut retained = content.work_bytes()?;
        for (index, entity) in content.entities.iter().enumerate() {
            stop(&mut cancelled)?;
            budget.check(
                retained
                    .checked_add(
                        entity
                            .compound
                            .len()
                            .checked_mul(8)
                            .ok_or_else(|| refuse("结构实体工作集溢出"))?,
                    )
                    .ok_or_else(|| refuse("结构实体工作集溢出"))?,
            )?;
            let mut raw = vec![10, 0, 0];
            raw.extend_from_slice(&entity.compound);
            let root = crate::sections::parse_root(&raw).map_err(refuse)?;
            collect(
                &root,
                index,
                String::new(),
                content.source_min,
                &mut found,
                &mut retained,
                budget,
                &mut cancelled,
            )?;
        }
        found.hits.sort_by(|a, b| {
            a.entity
                .distance
                .total_cmp(&b.entity.distance)
                .then_with(|| a.root.cmp(&b.root))
                .then_with(|| a.path.cmp(&b.path))
        });
        stop(&mut cancelled)?;
        Ok(found)
    }
}
fn collect(
    tag: &crate::sections::Tag<'_>,
    root: usize,
    path: String,
    source_min: [i32; 3],
    found: &mut Found<DocumentHit>,
    retained: &mut usize,
    budget: EditWorkBudget,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), WorldError> {
    stop(cancelled)?;
    if tag.kind != 10 {
        return Err(refuse("结构实体树节点必须为 Compound"));
    }
    if let Some(pos) = tag.field("Pos") {
        if pos.kind != 9
            || pos.items.len() != 3
            || pos
                .items
                .iter()
                .any(|item| item.kind != 6 || item.raw.len() != 8)
        {
            return Err(refuse("结构实体 Pos 必须为三项 Double 列表"));
        }
        let position = std::array::from_fn(|axis| {
            f64::from_be_bytes(pos.items[axis].raw.try_into().expect("已核对 Double 宽度"))
                - f64::from(source_min[axis])
        });
        if !position.into_iter().all(f64::is_finite) {
            return Err(refuse("结构实体 Pos 包含非有限数"));
        }
        if let Some((distance, lateral)) = found.ray.measure(position) {
            let id = tag
                .field("id")
                .map(|id| id.string().map_err(refuse))
                .transpose()?;
            *retained = retained
                .checked_add(256 + path.len() + id.as_ref().map_or(0, String::len))
                .ok_or_else(|| refuse("结构射线结果工作集溢出"))?;
            budget.check(*retained)?;
            found.hits.push(DocumentHit {
                root,
                path: path.clone(),
                entity: Entity {
                    id,
                    position,
                    distance,
                    lateral,
                    sha256: nbt_sha(tag.raw),
                },
            });
        }
    } else {
        found.unpositioned += 1;
    }
    if let Some(passengers) = tag.field("Passengers") {
        if passengers.kind != 9 {
            return Err(refuse("结构实体 Passengers 必须为列表"));
        }
        for (index, passenger) in passengers.items.iter().enumerate() {
            collect(
                passenger,
                root,
                format!("{path}/Passengers/{index}"),
                source_min,
                found,
                retained,
                budget,
                cancelled,
            )?;
        }
    }
    if let Some(vehicle) = tag.field("Riding") {
        collect(
            vehicle,
            root,
            format!("{path}/Riding"),
            source_min,
            found,
            retained,
            budget,
            cancelled,
        )?;
    }
    Ok(())
}
