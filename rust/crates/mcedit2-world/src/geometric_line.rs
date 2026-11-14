//! 原版生成器使用的 16 位定点三维 Bresenham；末端不重复绘制，保留原算法的平局顺序。
use crate::{editing::EditWorkBudget, WorldError};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub start: [f64; 3],
    pub end: [f64; 3],
}

fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}

fn fixed(point: [f64; 3]) -> Result<[i64; 3], WorldError> {
    let mut result = [0; 3];
    for (axis, value) in point.into_iter().enumerate() {
        if !value.is_finite()
            || value.floor() < f64::from(i32::MIN)
            || value.floor() > f64::from(i32::MAX)
        {
            return Err(invalid("直线端点须为 i32 方块范围内的有限坐标"));
        }
        result[axis] = (value * 65536.).floor() as i64;
    }
    Ok(result)
}

pub fn plot_cancellable(
    input: Input,
    budget: EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Vec<[i32; 3]>, WorldError> {
    let mut point = fixed(input.start)?;
    let end = fixed(input.end)?;
    let distance: [i64; 3] = std::array::from_fn(|axis| (end[axis] - point[axis]).abs());
    let steps: [i64; 3] = std::array::from_fn(|axis| {
        if end[axis] > point[axis] {
            65536
        } else {
            -65536
        }
    });
    let mut major = 0;
    for axis in 1..3 {
        if distance[axis] > distance[major] {
            major = axis;
        }
    }
    let count =
        usize::try_from((distance[major] + 65535) / 65536).map_err(|_| invalid("直线格数溢出"))?;
    // 同时覆盖 Rust 点列、JSON 交接及解释器对象，不能先分配后检查。
    budget.check(
        count
            .checked_mul(384)
            .ok_or_else(|| invalid("直线工作集溢出"))?,
    )?;
    let mut result = Vec::with_capacity(count);
    let mut error = [-distance[major]; 3];
    for index in 0..count {
        if index.is_multiple_of(4096) && cancelled() {
            return Err(invalid("直线栅格化已取消"));
        }
        let mut cell = [0; 3];
        for axis in 0..3 {
            cell[axis] =
                i32::try_from(point[axis] >> 16).map_err(|_| invalid("直线格坐标超出 i32"))?;
        }
        result.push(cell);
        for axis in 0..3 {
            error[axis] += 2 * distance[axis];
        }
        for axis in 0..3 {
            if axis == major {
                continue;
            }
            while error[axis] >= 0 {
                point[axis] += steps[axis];
                error[axis] -= 2 * distance[major];
            }
        }
        point[major] += steps[major];
    }
    if cancelled() {
        return Err(invalid("直线栅格化已取消"));
    }
    Ok(result)
}
