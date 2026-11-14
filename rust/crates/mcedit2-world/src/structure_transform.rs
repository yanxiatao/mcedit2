//! 结构网格的 XYZ 旋转、逐轴缩放与重采样；预览和实际粘贴使用同一份结果。
use crate::{
    clipboard::{BiomeVolume, BlockClipboard, OpaqueBlockEntity},
    editing::EditWorkBudget,
    sections::State,
    WorldError,
};
use std::collections::{BTreeMap, HashMap};
#[path = "structure_entity_transform.rs"]
mod entity;
#[path = "structure_hanging.rs"]
pub(crate) mod hanging;
#[path = "structure_orientation.rs"]
mod orientation;

fn invalid(message: impl ToString) -> WorldError {
    WorldError::Invalid(message.to_string())
}
type Matrix = [[f64; 3]; 3];
fn multiply(a: Matrix, b: Matrix) -> Matrix {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|i| a[r][i] * b[i][c]).sum()))
}
fn vector(matrix: Matrix, point: [f64; 3]) -> [f64; 3] {
    matrix.map(|row| (0..3).map(|i| row[i] * point[i]).sum())
}
fn rotation(angles: [f64; 3]) -> Matrix {
    fn sin_cos(angle: f64) -> (f64, f64) {
        let angle = angle.rem_euclid(360.);
        match angle {
            0. => (0., 1.),
            90. => (1., 0.),
            180. => (0., -1.),
            270. => (-1., 0.),
            _ => angle.to_radians().sin_cos(),
        }
    }
    let (sx, cx) = sin_cos(angles[0]);
    let (sy, cy) = sin_cos(angles[1]);
    let (sz, cz) = sin_cos(angles[2]);
    multiply(
        [[cz, -sz, 0.], [sz, cz, 0.], [0., 0., 1.]],
        multiply(
            [[cy, 0., sy], [0., 1., 0.], [-sy, 0., cy]],
            [[1., 0., 0.], [0., cx, -sx], [0., sx, cx]],
        ),
    )
}

/// 角度按右手正方向依次绕 X/Y/Z 旋转，再沿世界轴缩放；负缩放即镜像。
/// rotate_states 显式决定是否同时转换可表示的方块朝向，不隐式修改未知属性。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub rotation: [f64; 3],
    pub scale: [f64; 3],
    pub rotate_states: bool,
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            rotation: [0.; 3],
            scale: [1.; 3],
            rotate_states: true,
        }
    }
}
impl Transform {
    pub fn validate(self) -> Result<(), WorldError> {
        if !self
            .rotation
            .into_iter()
            .chain(self.scale)
            .all(f64::is_finite)
            || self.scale.contains(&0.)
        {
            return Err(invalid("旋转必须为有限角度，缩放必须为非零有限数"));
        }
        Ok(())
    }
    pub fn is_identity(self) -> bool {
        self.rotation.into_iter().all(|v| v.rem_euclid(360.) == 0.) && self.scale == [1.; 3]
    }
    /// 重复克隆只旋转步进方向，比例只作用于结构本身，与原版平铺规则一致。
    pub fn rotate_offset(self, offset: [f64; 3]) -> [f64; 3] {
        vector(rotation(self.rotation), offset)
    }
    pub(crate) fn mapping(self, size: [u16; 3]) -> Result<Mapping, WorldError> {
        self.validate()?;
        if size.contains(&0) {
            return Err(invalid("结构尺寸不能为零"));
        }
        let r = rotation(self.rotation);
        let forward: Matrix = std::array::from_fn(|i| r[i].map(|v| v * self.scale[i]));
        let inverse: Matrix =
            std::array::from_fn(|i| std::array::from_fn(|j| r[j][i] / self.scale[j]));
        let center = size.map(|n| f64::from(n) * 0.5);
        let extent = forward.map(|row| (0..3).map(|i| row[i].abs() * center[i]).sum::<f64>());
        let min = std::array::from_fn(|i| center[i] - extent[i]);
        let mut output = [0u16; 3];
        let mut offset = [0i32; 3];
        for i in 0..3 {
            let span = (extent[i] * 2.).ceil();
            if !(1. ..=f64::from(u16::MAX)).contains(&span)
                || min[i].floor() < f64::from(i32::MIN)
                || min[i].floor() > f64::from(i32::MAX)
            {
                return Err(invalid("变换后的结构尺寸或偏移超出可编辑范围"));
            }
            output[i] = span as u16;
            offset[i] = min[i].floor() as i32;
        }
        let rounded = rotation(
            self.rotation
                .map(|a| ((a.rem_euclid(360.) + 45.) / 90.).floor() * 90.),
        );
        let basis = std::array::from_fn(|i| rounded[i].map(|v| v * self.scale[i].signum()));
        let facing = std::array::from_fn(|i| r[i].map(|v| v * self.scale[i].signum()));
        Ok(Mapping {
            forward,
            inverse,
            basis,
            facing,
            center,
            min,
            output,
            offset,
        })
    }
}
pub(crate) struct Mapping {
    forward: Matrix,
    inverse: Matrix,
    pub(super) basis: Matrix,
    facing: Matrix,
    center: [f64; 3],
    min: [f64; 3],
    pub output: [u16; 3],
    pub offset: [i32; 3],
}
pub(crate) fn transform_entity(
    compound: &[u8],
    origin: [i32; 3],
    size: [u16; 3],
    transform: Transform,
    version: Option<i32>,
    variants: &BTreeMap<String, crate::painting::Variant>,
    dimensions: &crate::entity_editing::DimensionChange,
) -> Result<Vec<u8>, WorldError> {
    entity::transform(
        compound,
        origin,
        &transform.mapping(size)?,
        transform.rotate_states,
        version,
        variants,
        dimensions,
    )
}
impl Mapping {
    pub(super) fn point(&self, source: [f64; 3]) -> [f64; 3] {
        let p = vector(
            self.forward,
            std::array::from_fn(|i| source[i] - self.center[i]),
        );
        std::array::from_fn(|i| p[i] + self.center[i] - self.min[i])
    }
    fn sample(&self, target: [usize; 3], size: [u16; 3]) -> Option<[usize; 3]> {
        let p = vector(
            self.inverse,
            std::array::from_fn(|i| target[i] as f64 + 0.5 + self.min[i] - self.center[i]),
        );
        let p: [f64; 3] = std::array::from_fn(|i| (p[i] + self.center[i]).floor());
        p.iter()
            .zip(size)
            .all(|(&v, n)| v >= 0. && v < f64::from(n))
            .then(|| p.map(|v| v as usize))
    }
    pub(super) fn direction(&self, source: [f64; 3]) -> [f64; 3] {
        vector(self.facing, source)
    }
    pub(super) fn grid_direction(&self, source: [i32; 3]) -> [i32; 3] {
        vector(self.basis, source.map(f64::from)).map(|v| v as i32)
    }
    pub(super) fn orientation_identity(&self) -> bool {
        self.basis == [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]
    }
}
fn index(p: [usize; 3], size: [u16; 3]) -> usize {
    (p[1] * usize::from(size[2]) + p[2]) * usize::from(size[0]) + p[0]
}
fn biome(clip: &BlockClipboard, p: [usize; 3]) -> Result<Option<String>, WorldError> {
    let Some(b) = &clip.biomes else {
        return Ok(None);
    };
    let at = match b.resolution {
        1 => index(p, clip.size),
        4 => {
            let q = std::array::from_fn(|i| (p[i] + clip.source_min[i].rem_euclid(4) as usize) / 4);
            index(q, b.size)
        }
        _ => return Err(invalid("结构生物群系分辨率不受支持")),
    };
    Ok(Some(
        b.cells
            .get(at)
            .ok_or_else(|| invalid("结构生物群系采样越界"))?
            .clone(),
    ))
}

impl BlockClipboard {
    /// 源证明由组合传输独立保留；输出是冻结的、可复用的变换结果，不伪造绑定源。
    /// 逆向采样体素中心，四角旋转用精确正余弦；任意角度的盒外格留在 mask 外。
    pub fn resampled_cancellable(
        &self,
        transform: Transform,
        cancelled: impl FnMut() -> bool,
    ) -> Result<Self, WorldError> {
        self.resampled_with_origins_cancellable(transform, cancelled)
            .map(|(clip, _)| clip)
    }
    pub fn resampled_with_budget_cancellable(
        &self,
        transform: Transform,
        budget: EditWorkBudget,
        cancelled: impl FnMut() -> bool,
    ) -> Result<Self, WorldError> {
        self.resampled_with_origins_budget_cancellable(transform, budget, cancelled)
            .map(|(clip, _)| clip)
    }
    /// 每个目标 BE 记录实际采样的源条目，支持一对多缩放而不猜测载荷配对。
    pub(crate) fn resampled_with_origins_cancellable(
        &self,
        transform: Transform,
        cancelled: impl FnMut() -> bool,
    ) -> Result<(Self, Vec<usize>), WorldError> {
        self.resampled_with_origins_budget_cancellable(
            transform,
            EditWorkBudget::default(),
            cancelled,
        )
    }
    pub(crate) fn resampled_with_origins_budget_cancellable(
        &self,
        transform: Transform,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<(Self, Vec<usize>), WorldError> {
        self.check_work(budget)?;
        if cancelled() {
            return Err(invalid("结构变换已取消"));
        }
        let volume = self.size.iter().map(|v| usize::from(*v)).product::<usize>();
        if volume == 0
            || volume != self.blocks.len()
            || self.mask.as_ref().is_some_and(|m| m.len() != volume)
        {
            return Err(invalid("变换源结构的尺寸、方块或掩码不完整"));
        }
        transform.validate()?;
        if transform.is_identity() {
            let mut result = self.clone();
            result.source_bound = false;
            return Ok((result, (0..self.block_entities.len()).collect()));
        }
        let map = transform.mapping(self.size)?;
        let output = map
            .output
            .iter()
            .try_fold(1usize, |a, &v| a.checked_mul(usize::from(v)))
            .ok_or_else(|| invalid("变换体积溢出"))?;
        let largest = self
            .blocks
            .iter()
            .map(crate::clipboard::state_bytes)
            .max()
            .ok_or_else(|| invalid("变换源没有方块"))?;
        let biome_bytes = self.biomes.as_ref().map_or(0, |volume| {
            volume
                .cells
                .iter()
                .map(|value| value.len() + std::mem::size_of::<String>())
                .max()
                .unwrap_or(0)
        }) + usize::from(self.numeric_biomes.is_some())
            * std::mem::size_of::<i32>();
        budget.check(
            self.work_bytes()?
                .checked_mul(2)
                .and_then(|n| {
                    output
                        .checked_mul((largest + 32 + biome_bytes) * 4)
                        .and_then(|out| n.checked_add(out))
                })
                .ok_or_else(|| invalid("重采样工作集溢出"))?,
        )?;
        let mut result = self.clone();
        result.source_bound = false;
        result.size = map.output;
        result.offset = std::array::from_fn(|i| self.offset[i]);
        for i in 0..3 {
            result.offset[i] = result.offset[i]
                .checked_add(map.offset[i])
                .ok_or_else(|| invalid("变换后结构偏移溢出"))?;
        }
        result.blocks.clear();
        result.block_entities.clear();
        result.scheduled_ticks.clear();
        let mut mask = Vec::new();
        let mut colors = Vec::new();
        let mut numeric_colors = Vec::new();
        let mut origins = Vec::new();
        let be = self
            .block_entities
            .iter()
            .enumerate()
            .map(|(source_index, b)| {
                let p = b.position.map(|v| v as usize);
                (index(p, self.size), (source_index, b))
            })
            .collect::<HashMap<_, _>>();
        let mut ticks = HashMap::<usize, Vec<&crate::scheduled_ticks::Tick>>::new();
        for tick in &self.scheduled_ticks {
            ticks
                .entry(index(tick.position.map(|v| v as usize), self.size))
                .or_default()
                .push(tick);
        }
        let mut states = BTreeMap::<String, State>::new();
        let mut payload = 0usize;
        for y in 0..usize::from(map.output[1]) {
            for z in 0..usize::from(map.output[2]) {
                for x in 0..usize::from(map.output[0]) {
                    if result.blocks.len().is_multiple_of(256) && cancelled() {
                        return Err(invalid("结构变换已取消"));
                    }
                    let original = map
                        .sample([x, y, z], self.size)
                        .filter(|p| self.mask.as_ref().is_none_or(|m| m[index(*p, self.size)]));
                    let Some(p) = original else {
                        // 稀疏格仍占密集数组的一个索引，其值没有读取或写入权限。
                        if let Some(value) = biome(self, [0, 0, 0])? {
                            colors.push(value);
                        }
                        if let Some(volume) = &self.numeric_biomes {
                            numeric_colors.push(volume.cells[0]);
                        }
                        mask.push(false);
                        result
                            .blocks
                            .push(crate::editing::parse_state(if self.numeric {
                                "numeric:0[data=0]"
                            } else {
                                "minecraft:air"
                            })?);
                        continue;
                    };
                    let at = index(p, self.size);
                    let source = &self.blocks[at];
                    let state = if transform.rotate_states && !map.orientation_identity() {
                        let key = crate::editing::render_state(source);
                        if !states.contains_key(&key) {
                            let next = if source.name.starts_with("numeric:") {
                                crate::clipboard::reorient_numeric_with(
                                    source,
                                    self.data_version,
                                    self.source_release.as_deref(),
                                    |state| orientation::state(state, &map),
                                )?
                            } else {
                                orientation::state(source, &map)?
                            };
                            states.insert(key.clone(), next);
                        }
                        states[&key].clone()
                    } else {
                        source.clone()
                    };
                    result.blocks.push(state);
                    mask.push(true);
                    if let Some(value) = biome(self, p)? {
                        colors.push(value);
                    }
                    if let Some(biomes) = &self.numeric_biomes {
                        numeric_colors.push(
                            *biomes
                                .cells
                                .get(at)
                                .ok_or_else(|| invalid("数字 biome 重采样索引越界"))?,
                        );
                    }
                    if let Some(entries) = ticks.get(&at) {
                        for entry in entries {
                            let mut absolute = [0; 3];
                            for axis in 0..3 {
                                absolute[axis] = self.source_min[axis]
                                    .checked_add([x, y, z][axis] as i32)
                                    .ok_or_else(|| invalid("变换计划更新坐标溢出"))?;
                            }
                            let compound =
                                crate::scheduled_ticks::relocate(&entry.compound, absolute)?;
                            payload = payload
                                .checked_add(compound.len())
                                .ok_or_else(|| invalid("变换载荷大小溢出"))?;
                            if payload > budget.bytes() / 4 {
                                return Err(invalid("缩放后的计划更新载荷超过结构预算"));
                            }
                            result.scheduled_ticks.push(crate::scheduled_ticks::Tick {
                                position: [x as i32, y as i32, z as i32],
                                kind: entry.kind,
                                compound,
                            });
                        }
                    }
                    if let Some((source_index, entry)) = be.get(&at) {
                        let absolute = std::array::from_fn(|i| {
                            self.source_min[i].checked_add([x, y, z][i] as i32)
                        });
                        let [Some(ax), Some(ay), Some(az)] = absolute else {
                            return Err(invalid("变换 BE 坐标溢出"));
                        };
                        let bytes = entity::block_entity(
                            &entry.compound,
                            (ax, ay, az),
                            &map,
                            transform.rotate_states,
                        )?;
                        payload = payload
                            .checked_add(bytes.len())
                            .ok_or_else(|| invalid("变换载荷大小溢出"))?;
                        if payload > budget.bytes() / 4 {
                            return Err(invalid("缩放后的方块实体载荷超过结构预算"));
                        }
                        result.block_entities.push(OpaqueBlockEntity {
                            position: [x as i32, y as i32, z as i32],
                            compound: bytes,
                        });
                        origins.push(*source_index);
                    }
                }
            }
        }
        if !mask.iter().any(|v| *v) {
            return Err(invalid("此变换没有可采样的源方块"));
        }
        let sparse = mask.iter().any(|v| !*v);
        if self.biomes.is_some() {
            result.biomes = Some(BiomeVolume {
                resolution: 1,
                size: map.output,
                cells: colors,
            });
        }
        result.mask = sparse.then_some(mask);
        if let Some(source) = &self.numeric_biomes {
            result.numeric_biomes = Some(crate::numeric_biome::Volume {
                size: map.output,
                cells: numeric_colors,
                data_version: source.data_version,
            });
        }
        for entry in &mut result.entities {
            if cancelled() {
                return Err(invalid("结构实体变换已取消"));
            }
            entry.compound = entity::transform(
                &entry.compound,
                self.source_min,
                &map,
                transform.rotate_states,
                self.data_version,
                &self.painting_variants,
                &crate::entity_editing::DimensionChange::within(self.entity_dimension.as_deref()),
            )?;
            // 以最终写入 NBT 的 Double 为准，避免世界大坐标加减后的舍入
            // 与另算一次局部坐标不同；不以误差阈值掩盖载荷不一致。
            let mut raw = vec![10, 0, 0];
            raw.extend_from_slice(&entry.compound);
            let root = crate::sections::parse_root(&raw).map_err(invalid)?;
            let position = root.require("Pos", 9).map_err(invalid)?;
            entry.position = std::array::from_fn(|i| {
                f64::from_be_bytes(
                    position.items[i]
                        .raw
                        .try_into()
                        .expect("实体变换已验证 Double 坐标"),
                ) - f64::from(self.source_min[i])
            });
            if entry
                .position
                .iter()
                .zip(map.output)
                .any(|(&v, n)| !v.is_finite() || v < 0. || v >= f64::from(n))
            {
                return Err(invalid("变换后的实体根位置超出结构包围盒"));
            }
            if let Some(p) = entry.block_position {
                entry.block_position = Some(
                    if let Some(anchor) =
                        hanging::block_position(&root, self.source_min, entry.position)?
                    {
                        anchor
                    } else {
                        map.point(p.map(|v| f64::from(v) + 0.5))
                            .map(|v| v.floor() as i32)
                    },
                );
            }
        }
        crate::entity_references::relocate_clipboard(
            &self.entities,
            &mut result.entities,
            self.data_version,
        )?;
        result.check_work(budget)?;
        Ok((result, origins))
    }
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::Interchange {
        tests::structure_transform_pipeline();
    }
}
#[cfg(test)]
#[path = "structure_transform_tests.rs"]
mod tests;
