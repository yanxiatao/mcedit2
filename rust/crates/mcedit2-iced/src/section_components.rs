//! 将可往返的 Section 入口折叠为连通分量；封闭邻段只作为终点，不连通其余入口。
const INACTIVE: i32 = i32::MIN;

/// 不可达段的三维前缀计数。小范围直接读位，较大范围固定八次取数。
pub(super) struct Volume {
    nodes: Vec<u8>,
    hidden: Vec<u32>,
    size: [usize; 3],
}
pub(super) fn volume_len(size: [usize; 3]) -> Option<usize> {
    size.into_iter()
        .try_fold(1_usize, |n, side| n.checked_mul(side.checked_add(1)?))
}
impl Volume {
    pub fn build(
        size: [usize; 3],
        nodes: Vec<u8>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Self>, String> {
        let count = volume_len(size).ok_or("空域范围前缀数量溢出")?;
        let mut hidden = Vec::new();
        hidden
            .try_reserve_exact(count)
            .map_err(|e| format!("无法分配空域范围计数：{e}"))?;
        hidden.resize(count, 0_u32);
        let row = size[0] + 1;
        let layer = row * (size[2] + 1);
        for y in 1..=size[1] {
            for z in 1..=size[2] {
                if cancelled() {
                    return Ok(None);
                }
                let mut sum = 0_u32;
                for x in 1..=size[0] {
                    if x.is_multiple_of(4096) && cancelled() {
                        return Ok(None);
                    }
                    let source = ((y - 1) * size[2] + z - 1) * size[0] + x - 1;
                    sum += u32::from(nodes[source] == 0);
                    let at = y * layer + z * row + x;
                    hidden[at] =
                        sum + hidden[at - layer] + hidden[at - row] - hidden[at - layer - row];
                }
            }
        }
        Ok(Some(Self {
            nodes,
            hidden,
            size,
        }))
    }
    pub fn visible(&self, node: usize) -> bool {
        self.nodes[node] != 0
    }
    pub fn hidden_count(&self, low: [usize; 3], high: [usize; 3]) -> u32 {
        let count = (0..3).map(|i| high[i] - low[i] + 1).product::<usize>();
        if count <= 8 {
            let mut hidden = 0;
            for y in low[1]..=high[1] {
                for z in low[2]..=high[2] {
                    let start = (y * self.size[2] + z) * self.size[0] + low[0];
                    hidden += self.nodes[start..start + high[0] - low[0] + 1]
                        .iter()
                        .filter(|v| **v == 0)
                        .count() as u32;
                }
            }
            return hidden;
        }
        let row = self.size[0] + 1;
        let layer = row * (self.size[2] + 1);
        let slice = |y| {
            let at = |z, x| self.hidden[y * layer + z * row + x];
            at(high[2] + 1, high[0] + 1) + at(low[2], low[0])
                - at(low[2], high[0] + 1)
                - at(high[2] + 1, low[0])
        };
        slice(high[1] + 1) - slice(low[1])
    }
}

pub(super) fn neighbor(node: usize, size: [usize; 3], face: usize) -> Option<usize> {
    let x = node % size[0];
    let z = node / size[0] % size[2];
    let y = node / (size[0] * size[2]);
    match face {
        0 => (y + 1 < size[1]).then_some(node + size[0] * size[2]),
        1 => (y > 0).then(|| node - size[0] * size[2]),
        2 => (x + 1 < size[0]).then_some(node + 1),
        3 => (x > 0).then(|| node - 1),
        4 => (z + 1 < size[2]).then_some(node + size[0]),
        5 => (z > 0).then(|| node - size[0]),
        _ => unreachable!("Section 只有六个入口"),
    }
}
fn diagonal(connections: u64, face: usize) -> bool {
    connections & (1 << (face * 7)) != 0
}
fn find(parents: &mut [i32], mut node: usize) -> usize {
    let mut root = node;
    while parents[root] >= 0 {
        root = parents[root] as usize;
    }
    while node != root {
        let next = parents[node] as usize;
        parents[node] = root as i32;
        node = next;
    }
    root
}
fn join(parents: &mut [i32], a: usize, b: usize) {
    let (mut a, mut b) = (find(parents, a), find(parents, b));
    if a == b {
        return;
    }
    if parents[a] > parents[b] {
        std::mem::swap(&mut a, &mut b);
    }
    parents[a] += parents[b];
    parents[b] = a as i32;
}

pub(super) fn build(
    connections: &[u64],
    size: [usize; 3],
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Vec<i32>>, String> {
    let count = connections.len().checked_mul(6).ok_or("空域入口数量溢出")?;
    if count >= i32::MAX as usize {
        return Err("空域入口数量超过分量索引范围".into());
    }
    let mut labels = Vec::new();
    labels
        .try_reserve_exact(count)
        .map_err(|e| format!("无法分配空域分量：{e}"))?;
    labels.resize(count, INACTIVE);
    for (node, &bits) in connections.iter().enumerate() {
        if node.is_multiple_of(1024) && cancelled() {
            return Ok(None);
        }
        if bits >> 36 != 0 {
            return Err("源 Section 连通矩阵含无效入口位".into());
        }
        for a in 0..6 {
            let row = (bits >> (a * 6)) & 63;
            if row != 0 && !diagonal(bits, a) {
                return Err("源 Section 连通矩阵缺少入口自身".into());
            }
            for b in a + 1..6 {
                if (row >> b & 1) != (bits >> (b * 6 + a) & 1) {
                    return Err("源 Section 连通矩阵不是对称空域".into());
                }
            }
        }
        for face in [0, 2, 4] {
            if let Some(next) = neighbor(node, size, face) {
                if diagonal(bits, face) && diagonal(connections[next], face ^ 1) {
                    let (a, b) = (node * 6 + face, next * 6 + (face ^ 1));
                    // 每个入口只属于一个相邻边，尚未参与段内合并。
                    labels[a] = -1;
                    labels[b] = -1;
                    join(&mut labels, a, b);
                }
            }
        }
    }
    for (node, &bits) in connections.iter().enumerate() {
        if node.is_multiple_of(1024) && cancelled() {
            return Ok(None);
        }
        for a in 0..6 {
            let entry = node * 6 + a;
            if labels[entry] == INACTIVE {
                continue;
            }
            for b in a + 1..6 {
                let exit = node * 6 + b;
                if labels[exit] != INACTIVE && bits & (1 << (a * 6 + b)) != 0 {
                    join(&mut labels, entry, exit);
                }
            }
        }
    }
    // 原地压缩，保留负根直到所有入口都已查根；不再分配第二份同规模标签。
    for port in 0..count {
        if port.is_multiple_of(4096) && cancelled() {
            return Ok(None);
        }
        if labels[port] >= 0 {
            let root = find(&mut labels, port);
            labels[port] = root as i32;
        }
    }
    for (port, label) in labels.iter_mut().enumerate() {
        if *label == INACTIVE {
            *label = -1;
        } else if *label < 0 {
            *label = port as i32;
        }
    }
    Ok(Some(labels))
}

pub(super) struct Seeds {
    pub roots: Vec<i32>,
    pub extra: Vec<usize>,
}
pub(super) fn seeds(start: usize, size: [usize; 3], connections: &[u64], labels: &[i32]) -> Seeds {
    let mut result = Seeds {
        roots: Vec::with_capacity(36),
        extra: vec![start],
    };
    // 与原观察规则相同：相机所在段从全部六面出发，即使其内有封闭空腔。
    for face in 0..6 {
        let Some(next) = neighbor(start, size, face) else {
            continue;
        };
        result.extra.push(next);
        let exits = (connections[next] >> ((face ^ 1) * 6)) & 63;
        for exit in 0..6 {
            if exits & (1 << exit) == 0 {
                continue;
            }
            let label = labels[next * 6 + exit];
            if label >= 0 {
                result.roots.push(label);
            } else if let Some(terminal) = neighbor(next, size, exit) {
                result.extra.push(terminal);
            }
        }
    }
    result.roots.sort_unstable();
    result.roots.dedup();
    result.extra.sort_unstable();
    result.extra.dedup();
    result
}

pub(super) fn reachable(
    size: [usize; 3],
    connections: &[u64],
    labels: &[i32],
    roots: &[i32],
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<(Vec<u8>, usize)>, String> {
    let mut nodes = Vec::new();
    nodes
        .try_reserve_exact(connections.len())
        .map_err(|e| format!("无法分配空域可达掩码：{e}"))?;
    nodes.resize(connections.len(), 0);
    for (node, &bits) in connections.iter().enumerate() {
        if node.is_multiple_of(1024) && cancelled() {
            return Ok(None);
        }
        let mut exits = 0_u64;
        for face in 0..6 {
            let label = labels[node * 6 + face];
            if label >= 0 && roots.binary_search(&label).is_ok() {
                nodes[node] = 1;
                exits |= (bits >> (face * 6)) & 63;
            }
        }
        for face in 0..6 {
            if exits & (1 << face) != 0 {
                if let Some(next) = neighbor(node, size, face) {
                    nodes[next] = 1;
                }
            }
        }
    }
    let visible = nodes.iter().filter(|node| **node != 0).count();
    Ok((!cancelled()).then_some((nodes, visible)))
}
