//! 区域内单位平面的通用矩形合并；只改变 GPU 表示，保留完整源网格。
use super::{Definition, Face, Hints, MeshBounds};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// 原材质像素逐格重复，不缩放为一张拉伸图片。仅 GPU 编码使用此位。
pub(super) const REPEAT: u32 = 1 << 16;
use super::encoding::{fan_indices, BOUNDARY_FAN, MAX_FAN_EDGES};
const MAX_CELLS: usize = (64 << 20) / 96;
const MAX_VISITS: usize = MAX_CELLS * 8;
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct Cell {
    plane: u64,
    phase: [u64; 2],
    a: i32,
    b: i32,
    axis: u8,
}
fn plane_bits(value: f64) -> u64 {
    if value == 0.0 {
        0
    } else {
        value.to_bits()
    }
}
fn axes(axis: usize) -> [usize; 2] {
    match axis {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    }
}
fn flat_axis<T: PartialEq>(lo: &[T; 3], hi: &[T; 3]) -> Option<usize> {
    let mut result = None;
    for axis in 0..3 {
        if lo[axis] == hi[axis] {
            if result.replace(axis).is_some() {
                return None;
            }
        }
    }
    result
}
/// 单个批次按最坏四格包围矩形估计，空间细分不丢弃任何源几何。
pub(super) fn target_faces() -> usize {
    MAX_CELLS / 4
}
fn side_mask(mode: u32, axis: usize) -> u8 {
    let facing =
        (mode & crate::material_mode::AXIS_FACING_MASK) >> crate::material_mode::AXIS_FACING_SHIFT;
    if mode & crate::material_mode::SINGLE_SIDED != 0
        && (1..=6).contains(&facing)
        && ((facing - 1) / 2) as usize == axis
    {
        if (facing - 1) % 2 == 0 {
            1
        } else {
            2
        }
    } else {
        3
    }
}
struct Rect {
    lower: [f64; 2],
    upper: [f64; 2],
    mask: u8,
}
#[derive(Default)]
struct Plane {
    rects: Vec<Rect>,
    lattices: BTreeSet<[u64; 2]>,
}
pub(super) struct Guard {
    cells: HashMap<Cell, [u8; 2]>,
}
impl Guard {
    /// 所有来源平面的覆盖计数都参与证明，避免把底层合并后改变共面贴层顺序。
    /// 此预算只限制优化证明；超过时记录原因，源几何仍完整进入普通上传。
    pub fn build(
        source: &Definition,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Self>, String> {
        let count = source
            .members
            .iter()
            .try_fold(0usize, |n, m| n.checked_add(m.source.len()))
            .ok_or("区域平面证明数量溢出")?;
        let mut planes: BTreeMap<(u8, u64), Plane> = BTreeMap::new();
        for member in &source.members {
            let delta: [f64; 3] = std::array::from_fn(|i| {
                (i64::from(member.origin[i]) - i64::from(source.origin[i])) as f64
            });
            for (ordinal, quad) in member.source.iter().enumerate() {
                if ordinal.is_multiple_of(1024) && cancelled() {
                    return Ok(None);
                }
                // 动态朝向的源面不提供固定平面的合并或覆盖证明。
                if quad.mode & crate::material_mode::BILLBOARD != 0 {
                    continue;
                }
                let mut lo = [f64::INFINITY; 3];
                let mut hi = [f64::NEG_INFINITY; 3];
                for p in &quad.xyz {
                    for axis in 0..3 {
                        let value = f64::from(p[axis]) + delta[axis];
                        if !value.is_finite() {
                            return Err("区域源几何包含非有限坐标".into());
                        }
                        lo[axis] = lo[axis].min(value);
                        hi[axis] = hi[axis].max(value);
                    }
                }
                let Some(axis) = flat_axis(&lo, &hi) else {
                    continue;
                };
                let [a, b] = axes(axis);
                let plane = planes
                    .entry((axis as u8, plane_bits(lo[axis])))
                    .or_default();
                if hi[a] - lo[a] == 1.0 && hi[b] - lo[b] == 1.0 {
                    plane.lattices.insert([
                        plane_bits(lo[a] - lo[a].floor()),
                        plane_bits(lo[b] - lo[b].floor()),
                    ]);
                }
                plane.rects.push(Rect {
                    lower: [lo[a], lo[b]],
                    upper: [hi[a], hi[b]],
                    mask: side_mask(quad.mode, axis),
                });
            }
        }
        let mut result = Self {
            cells: HashMap::new(),
        };
        result
            .cells
            .try_reserve(count.min(MAX_CELLS))
            .map_err(|error| format!("区域覆盖索引分配失败：{error}"))?;
        let mut visits = 0usize;
        for ((axis, plane), work) in planes {
            for phase in work.lattices {
                let offset = phase.map(f64::from_bits);
                for rect in &work.rects {
                    let lower: [f64; 2] =
                        std::array::from_fn(|i| (rect.lower[i] - offset[i]).floor());
                    let upper: [f64; 2] =
                        std::array::from_fn(|i| (rect.upper[i] - offset[i]).ceil());
                    if (0..2).any(|i| {
                        lower[i] < f64::from(i32::MIN) || upper[i] > f64::from(i32::MAX) + 1.0
                    }) {
                        log::warn!("[gpu-greedy] 源平面超出证明索引范围，保留源几何");
                        return Ok(None);
                    }
                    let area = (upper[0] - lower[0]) * (upper[1] - lower[1]);
                    if area > MAX_VISITS.saturating_sub(visits) as f64 {
                        log::warn!("[gpu-greedy] 平面覆盖证明超出工作预算，保留源几何");
                        return Ok(None);
                    }
                    for y in lower[1] as i64..upper[1] as i64 {
                        for x in lower[0] as i64..upper[0] as i64 {
                            if visits.is_multiple_of(1024) && cancelled() {
                                return Ok(None);
                            }
                            visits += 1;
                            let key = Cell {
                                plane,
                                phase,
                                a: x as i32,
                                b: y as i32,
                                axis,
                            };
                            if result.cells.len() >= MAX_CELLS && !result.cells.contains_key(&key) {
                                log::warn!("[gpu-greedy] 平面覆盖证明超过内存预算，保留源几何");
                                return Ok(None);
                            }
                            let coverage = result.cells.entry(key).or_insert([0; 2]);
                            for side in 0..2 {
                                if rect.mask & (1 << side) != 0 {
                                    coverage[side] = (coverage[side] + 1).min(2);
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(Some(result))
    }
    fn unique(&self, cell: Cell, mode: u32) -> bool {
        let mask = side_mask(mode, cell.axis as usize);
        self.cells.get(&cell).is_some_and(|coverage| {
            (0..2).all(|side| mask & (1 << side) == 0 || coverage[side] == 1)
        })
    }
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    plane: u64,
    phase: [u64; 2],
    page: u32,
    hint: u32,
    mode: u32,
    rgba: [u32; 4],
    light: u32,
    corners: u32,
    uv: [u32; 4],
    bucket: i64,
    axis: u8,
}
struct Candidate {
    key: Key,
    cell: [i32; 2],
    index: usize,
    u_on_a: bool,
    repeated: bool,
    limits: [i32; 2],
}
fn candidate(
    index: usize,
    face: &Face,
    source: &Definition,
    hints: &Hints,
    guard: &Guard,
    y32: bool,
) -> Option<Candidate> {
    let q = face.compact?;
    if q.mode
        & (crate::material_mode::VIEW_OFFSET_Z
            | crate::material_mode::FONT_TEXT
            | crate::material_mode::SIGN_NEAR_OUTLINE
            | crate::material_mode::BILLBOARD)
        != 0
    {
        return None;
    }
    let axis = flat_axis(&q.minimum, &q.maximum)?;
    let [a, b] = axes(axis);
    if f64::from(q.maximum[a]) - f64::from(q.minimum[a]) != 1.0
        || f64::from(q.maximum[b]) - f64::from(q.minimum[b]) != 1.0
        || [q.minimum[a], q.minimum[b]]
            .iter()
            .any(|v| f64::from(*v) < f64::from(i32::MIN) || f64::from(*v) >= f64::from(i32::MAX))
    {
        return None;
    }
    let lower = [f64::from(q.minimum[a]), f64::from(q.minimum[b])];
    let phase = lower.map(|v| plane_bits(v - v.floor()));
    let cell = Cell {
        plane: plane_bits(f64::from(q.minimum[axis])),
        phase,
        a: lower[0].floor() as i32,
        b: lower[1].floor() as i32,
        axis: axis as u8,
    };
    if !guard.unique(cell, q.mode) {
        return None;
    }
    let geometry: [u32; 4] = std::array::from_fn(|i| {
        ((q.corners >> (i * 5 + a)) & 1) | (((q.corners >> (i * 5 + b)) & 1) << 1)
    });
    let uv: [u32; 4] = std::array::from_fn(|i| (q.corners >> (i * 5 + 3)) & 3);
    let constant_uv = q
        .uv_minimum
        .iter()
        .zip(q.uv_maximum)
        .all(|(a, b)| a.to_bits() == b.to_bits());
    let mut geom_seen = 0;
    let mut uv_seen = 0;
    for i in 0..4 {
        geom_seen |= 1 << geometry[i];
        uv_seen |= 1 << uv[i];
        if (geometry[i] ^ geometry[(i + 1) % 4]).count_ones() != 1
            || (!constant_uv && (uv[i] ^ uv[(i + 1) % 4]).count_ones() != 1)
        {
            return None;
        }
    }
    if geom_seen != 15 || (!constant_uv && uv_seen != 15) {
        return None;
    }
    let zero = geometry.iter().position(|v| *v == 0)?;
    let one = geometry.iter().position(|v| *v == 1)?;
    let u_on_a = (uv[zero] & 1) != (uv[one] & 1);
    let member = &source.members[face.member];
    let original = &member.source[face.index];
    let hint = if constant_uv {
        q.atlas_page
    } else {
        hints.repeat_hint(original)?.get()
    };
    let uv_limits = if constant_uv {
        [i32::MAX; 2]
    } else {
        hints.repeat_limits(original)
    };
    let limits = if u_on_a {
        uv_limits
    } else {
        [uv_limits[1], uv_limits[0]]
    };
    Some(Candidate {
        key: Key {
            plane: cell.plane,
            phase,
            page: original.atlas_page,
            hint,
            mode: q.mode,
            rgba: q.rgba.map(f32::to_bits),
            light: q.light,
            corners: q.corners,
            uv: [
                q.uv_minimum[0].to_bits(),
                q.uv_minimum[1].to_bits(),
                q.uv_maximum[0].to_bits(),
                q.uv_maximum[1].to_bits(),
            ],
            bucket: if y32 {
                super::super::y_bucket(original, member.origin[1])
            } else {
                0
            },
            axis: axis as u8,
        },
        cell: [cell.a, cell.b],
        index,
        u_on_a,
        repeated: !constant_uv,
        limits,
    })
}
pub(super) fn merge(
    faces: &mut Vec<Face>,
    source: &Definition,
    hints: &Hints,
    guard: &Guard,
    y32: bool,
    cancelled: &dyn Fn() -> bool,
) -> Result<bool, String> {
    let mut candidates = Vec::new();
    for (index, face) in faces.iter().enumerate() {
        if index.is_multiple_of(1024) && cancelled() {
            return Ok(false);
        }
        if let Some(c) = candidate(index, face, source, hints, guard, y32) {
            candidates.push(c);
        }
    }
    if candidates.len() < 2 {
        return Ok(true);
    }
    candidates.sort_unstable_by_key(|c| (c.key, c.cell[1], c.cell[0]));
    let mut consumed = vec![false; faces.len()];
    let mut merged = Vec::new();
    let mut processed = 0usize;
    for group in candidates.chunk_by(|a, b| a.key == b.key) {
        let mut cells = BTreeMap::new();
        for c in group {
            if cells.insert((c.cell[1], c.cell[0]), c).is_some() {
                return Err("唯一平面证明中存在重复单位面".into());
            }
        }
        while let Some((&(row, col), &first)) = cells.first_key_value() {
            if processed.is_multiple_of(1024) && cancelled() {
                return Ok(false);
            }
            let mut width = 1i32;
            while width < first.limits[0]
                && col
                    .checked_add(width)
                    .is_some_and(|x| cells.contains_key(&(row, x)))
            {
                width += 1;
            }
            let mut height = 1i32;
            while height < first.limits[1]
                && row
                    .checked_add(height)
                    .is_some_and(|y| (0..width).all(|x| cells.contains_key(&(y, col + x))))
            {
                height += 1;
            }
            // 一份矩形实例使用共享的可变扇索引；保留全部单位边界顶点。
            // 连同退化顶点计算，唯一顶点至少减少四倍，才抵偿重复采样与裁剪的额外开销。
            let edges = 2 * (i64::from(width) + i64::from(height));
            let combine = edges <= i64::from(MAX_FAN_EDGES)
                && i64::from(fan_indices(edges as u32) / 3 + 1)
                    < i64::from(width) * i64::from(height);
            for y in 0..height {
                for x in 0..width {
                    if processed.is_multiple_of(1024) && cancelled() {
                        return Ok(false);
                    }
                    let c = cells
                        .remove(&(row + y, col + x))
                        .ok_or("连续矩形的单位面丢失")?;
                    if combine {
                        consumed[c.index] = true;
                    }
                    processed += 1;
                }
            }
            if !combine {
                continue;
            }
            let source_face = &faces[first.index];
            let mut face = source_face.clone();
            let mut q = face.compact.ok_or("矩形合并缺少紧凑编码")?;
            let [a, b] = axes(first.key.axis as usize);
            let phase = first.key.phase.map(f64::from_bits);
            let endpoints = [
                f64::from(col) + phase[0],
                (i64::from(col) + i64::from(width)) as f64 + phase[0],
                f64::from(row) + phase[1],
                (i64::from(row) + i64::from(height)) as f64 + phase[1],
            ];
            if endpoints.iter().any(|v| f64::from(*v as f32) != *v) {
                return Err("合并后的网格端点无法精确表示".into());
            }
            q.minimum[a] = endpoints[0] as f32;
            q.maximum[a] = endpoints[1] as f32;
            q.minimum[b] = endpoints[2] as f32;
            q.maximum[b] = endpoints[3] as f32;
            if first.repeated {
                q.uv_minimum = [0.0; 2];
                q.uv_maximum = if first.u_on_a {
                    [width as f32, height as f32]
                } else {
                    [height as f32, width as f32]
                };
                q.mode |= REPEAT | super::super::super::material_hints::FLAG;
                q.atlas_page = first.key.hint;
            }
            q.mode |= BOUNDARY_FAN;
            face.merged_bounds = Some(MeshBounds::from_corners(q.minimum, q.maximum));
            face.compact = Some(q);
            merged.push(face);
        }
    }
    if merged.is_empty() {
        return Ok(true);
    }
    let mut next = Vec::with_capacity(faces.len());
    next.extend(
        faces
            .iter()
            .zip(consumed)
            .filter(|(_, used)| !*used)
            .map(|(face, _)| face.clone()),
    );
    next.extend(merged);
    *faces = next;
    Ok(true)
}
