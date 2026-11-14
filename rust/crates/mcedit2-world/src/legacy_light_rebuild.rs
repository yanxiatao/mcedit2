//! 从真实方块全量计算旧数字格式光照；两圈计算来源只发布内圈，外部未知光不会影响发布结果。
use super::*;

fn ring(centers: &BTreeSet<(i32, i32)>, radius: i32) -> Result<BTreeSet<(i32, i32)>, WorldError> {
    let mut result = BTreeSet::new();
    for &(x, z) in centers {
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                result.insert((
                    x.checked_add(dx)
                        .ok_or_else(|| invalid("完整光照 X 范围溢出"))?,
                    z.checked_add(dz)
                        .ok_or_else(|| invalid("完整光照 Z 范围溢出"))?,
                ));
            }
        }
    }
    Ok(result)
}
fn point(coord: (i32, i32), index: usize) -> Result<Position, WorldError> {
    let y = (index / 256) as i32;
    let z = ((index / 16) % 16) as i32;
    let x = (index % 16) as i32;
    Ok(Position {
        x: coord
            .0
            .checked_mul(16)
            .and_then(|base| base.checked_add(x))
            .ok_or_else(|| invalid("完整光照 X 坐标溢出"))?,
        y,
        z: coord
            .1
            .checked_mul(16)
            .and_then(|base| base.checked_add(z))
            .ok_or_else(|| invalid("完整光照 Z 坐标溢出"))?,
    })
}
fn flag(channel: Channel) -> u8 {
    if channel == Channel::Block {
        1
    } else {
        2
    }
}
fn enqueue(
    queue: &mut VecDeque<(Position, Channel)>,
    pending: &mut BTreeMap<(i32, i32), Vec<u8>>,
    p: Position,
    channel: Channel,
) {
    let (coord, index) = NbtVolume::coord(p);
    let bits = &mut pending.get_mut(&coord).expect("光照队列只接受实际计算区块")[index];
    let bit = flag(channel);
    if *bits & bit == 0 {
        *bits |= bit;
        queue.push_back((p, channel));
    }
}

pub(super) fn run<F: FnMut() -> bool>(
    candidate: &NbtVolume,
    traits: &Traits,
    changed: &[Position],
    has_skylight: bool,
    budget: usize,
    memory_budget: usize,
    cancelled: F,
) -> Result<BTreeMap<(i32, i32), Vec<u8>>, WorldError> {
    // 包含三份体积缓冲、逐通道排队位图及每格最多两项的传播队列，原始 NBT 另计。
    let raw_bytes = candidate
        .chunks
        .values()
        .try_fold(0_usize, |sum, chunk| sum.checked_add(chunk.raw.len()))
        .ok_or_else(|| invalid("完整光照原始载荷大小溢出"))?;
    let work_bytes = candidate
        .chunks
        .len()
        .checked_mul(4_000_000)
        .and_then(|bytes| {
            raw_bytes
                .checked_mul(3)
                .and_then(|raw| bytes.checked_add(raw))
        })
        .ok_or_else(|| invalid("完整光照工作集大小溢出"))?;
    if work_bytes > memory_budget {
        return Err(invalid("完整光照计算超过工作内存预算"));
    }
    let centers = changed
        .iter()
        .map(|p| (p.x.div_euclid(16), p.z.div_euclid(16)))
        .collect::<BTreeSet<_>>();
    let publish = ring(&centers, 1)?;
    let compute = ring(&centers, 2)?;
    let mut work = Work {
        world: candidate.clone(),
        traits,
        remaining: budget,
        cancelled,
        has_skylight,
    };
    let mut actual = Vec::new();
    for &coord in &compute {
        work.tick()?;
        if work.world.get_chunk(coord)?.is_some() {
            actual.push(coord);
        }
    }
    let mut pending = BTreeMap::new();
    let height_limit = candidate.height_limit();
    let cells = height_limit as usize * 256;
    for &coord in &actual {
        let ids = work.world.chunks[&coord].ids.clone();
        let mut heights = vec![0_i32; 256];
        for column in 0..256 {
            for y in (0..height_limit as usize).rev() {
                work.tick()?;
                if traits.get(ids[y * 256 + column])?.opacity > 0 {
                    heights[column] = (y + 1) as i32;
                    break;
                }
            }
        }
        let mut block = vec![0; cells];
        let mut sky = vec![0; cells];
        for index in 0..cells {
            work.tick()?;
            block[index] = traits.get(ids[index])?.emission;
            if has_skylight && index / 256 >= heights[index % 256] as usize {
                sky[index] = 15;
            }
        }
        let chunk = work.world.chunks.get_mut(&coord).expect("完整计算来源存在");
        chunk.block = block;
        chunk.sky = sky;
        chunk.heights = heights;
        // 此标志仅表示本次工作缓冲已由真实方块初始化；磁盘完成标记在完整计算成功后才写入候选。
        chunk.complete = true;
        pending.insert(coord, vec![0_u8; cells]);
    }
    let mut queue = VecDeque::new();
    for &coord in &actual {
        for index in 0..cells {
            work.tick()?;
            let p = point(coord, index)?;
            if has_skylight {
                let mut sky = work.light(p, Channel::Sky)?;
                let opacity = work.opacity(p)?;
                for neighbour in neighbours(p)? {
                    let incoming: u8 = if neighbour.y >= height_limit {
                        15
                    } else if neighbour.y < 0 {
                        0
                    } else {
                        let key = (neighbour.x.div_euclid(16), neighbour.z.div_euclid(16));
                        // 这里只使用已证明缺失区块的开天空边界，不读取计算范围之外的未知来源。
                        if compute.contains(&key) && work.world.get_chunk(key)?.is_none() {
                            15
                        } else {
                            0
                        }
                    };
                    sky = sky.max(incoming.saturating_sub(opacity));
                }
                work.set(p, Channel::Sky, sky)?;
                if sky > 0 {
                    enqueue(&mut queue, &mut pending, p, Channel::Sky);
                }
            }
            if work.light(p, Channel::Block)? > 0 {
                enqueue(&mut queue, &mut pending, p, Channel::Block);
            }
        }
    }
    while let Some((p, channel)) = queue.pop_front() {
        work.tick()?;
        let (coord, index) = NbtVolume::coord(p);
        pending.get_mut(&coord).expect("队列拥有来源位图")[index] &= !flag(channel);
        let light = work.light(p, channel)?;
        for neighbour in neighbours(p)? {
            work.tick()?;
            if !(0..height_limit).contains(&neighbour.y) {
                continue;
            }
            let key = (neighbour.x.div_euclid(16), neighbour.z.div_euclid(16));
            if !compute.contains(&key) || work.world.get_chunk(key)?.is_none() {
                continue;
            }
            let value = light.saturating_sub(work.opacity(neighbour)?);
            if value > work.light(neighbour, channel)? {
                work.set(neighbour, channel, value)?;
                enqueue(&mut queue, &mut pending, neighbour, channel);
            }
        }
    }
    // 最远外侧计算误差到发布区域至少相隔 16 格；光强 <=15 且每条传播边至少衰减 1。
    // 外圈只用于计算，既不声明已完成，也不把其临时光照写入世界。
    Ok(work
        .world
        .changed_chunks_in(Some(&publish))?
        .into_iter()
        .collect())
}
