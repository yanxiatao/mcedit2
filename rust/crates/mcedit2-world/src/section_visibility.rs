//! 完整源 Section 的空域连通性；只接受显示模型已经证明的不透明整格。
use std::sync::Arc;

/// 方向顺序与 `surface_visibility::Direction::ALL` 一致；每行六位表示入口到出口。
pub const OPEN: u64 = (1_u64 << 36) - 1;

#[derive(Clone, Debug)]
enum Occupancy {
    Empty,
    Full,
    Partial(Arc<[u64; 64]>),
}

/// 与不可变区块来源一同发布；部分形状与透明模型在连通性中保留为空域。
#[derive(Clone, Debug)]
pub struct Section {
    pub y: i8,
    pub connections: u64,
    occupancy: Occupancy,
}
impl Section {
    /// 位索引采用源格式的 x + 16*z + 256*y，不重新排列世界数据。
    pub fn from_opaque(y: i8, opaque: [u64; 64]) -> Self {
        if opaque.iter().all(|word| *word == 0) {
            return Self {
                y,
                connections: OPEN,
                occupancy: Occupancy::Empty,
            };
        }
        if opaque.iter().all(|word| *word == u64::MAX) {
            return Self {
                y,
                connections: 0,
                occupancy: Occupancy::Full,
            };
        }
        let mut visited = opaque;
        let mut queue = Vec::<usize>::with_capacity(4096);
        let mut connections = 0;
        for start in 0..4096 {
            if visited[start / 64] & (1 << (start % 64)) != 0 {
                continue;
            }
            // 封闭在 Section 内部的空腔没有跨段入口，不必为它建立连通分量。
            let (x, z, y) = (start % 16, start / 16 % 16, start / 256);
            if x > 0 && x < 15 && y > 0 && y < 15 && z > 0 && z < 15 {
                continue;
            }
            visited[start / 64] |= 1 << (start % 64);
            queue.clear();
            queue.push(start);
            let mut head = 0;
            let mut faces = 0_u8;
            while head < queue.len() {
                let cell = queue[head];
                head += 1;
                let (x, z, y) = (cell % 16, cell / 16 % 16, cell / 256);
                for (face, neighbor) in [
                    (0, (y < 15).then_some(cell + 256)),
                    (1, (y > 0).then(|| cell - 256)),
                    (2, (x < 15).then_some(cell + 1)),
                    (3, (x > 0).then(|| cell - 1)),
                    (4, (z < 15).then_some(cell + 16)),
                    (5, (z > 0).then(|| cell - 16)),
                ] {
                    if let Some(next) = neighbor {
                        if visited[next / 64] & (1 << (next % 64)) == 0 {
                            visited[next / 64] |= 1 << (next % 64);
                            queue.push(next);
                        }
                    } else {
                        faces |= 1 << face;
                    }
                }
            }
            for entry in 0..6 {
                if faces & (1 << entry) != 0 {
                    connections |= u64::from(faces) << (entry * 6);
                }
            }
            if connections == OPEN {
                break;
            }
        }
        Self {
            y,
            connections,
            occupancy: Occupancy::Partial(Arc::new(opaque)),
        }
    }
    /// 穿墙观察以及近裁剪面穿入实体时，调用方据此关闭空域剔除。
    pub fn opaque(&self, x: u8, y: u8, z: u8) -> bool {
        let index = usize::from(x) + 16 * usize::from(z) + 256 * usize::from(y);
        assert!(x < 16 && y < 16 && z < 16, "Section 局部坐标越界");
        match &self.occupancy {
            Occupancy::Empty => false,
            Occupancy::Full => true,
            Occupancy::Partial(words) => words[index / 64] & (1 << (index % 64)) != 0,
        }
    }
    pub fn allocated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + if matches!(self.occupancy, Occupancy::Partial(_)) {
                512
            } else {
                0
            }
    }
}
