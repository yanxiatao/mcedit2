//! 自由视角的完整源面：四个独立角点，GPU 按固定拓扑展开三角形。
use crate::gpu_mesh::Vertex;

pub(crate) const TRIANGLES: [[usize; 3]; 2] = [[0, 1, 2], [0, 2, 3]];

/// 四角数据在不同模型来源中绕序不同；以实际源法向判断，保持固定对角线。
pub(crate) fn winding_reversed(xyz: &[[f32; 3]; 4], normal: [f32; 3]) -> bool {
    for [i, j, k] in TRIANGLES {
        let a: [f32; 3] = std::array::from_fn(|axis| xyz[j][axis] - xyz[i][axis]);
        let b: [f32; 3] = std::array::from_fn(|axis| xyz[k][axis] - xyz[i][axis]);
        let cross = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let facing = cross.iter().zip(normal).map(|(a, b)| a * b).sum::<f32>();
        if facing != 0. {
            return facing < 0.;
        }
    }
    false
}

/// 保留逐角点位置、UV、颜色及完整材质标记，不携带二维投影字段。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct WorldQuad {
    /// 普通源面是完整四角；BILLBOARD 保存前三角及第四槽的旋转锚点。
    /// 几何查询使用 positions / viewed，不能将锚点误作第四个可见角。
    pub xyz: [[f32; 3]; 4],
    pub uv: [[f32; 2]; 4],
    pub rgba: [[f32; 4]; 4],
    pub mode: u32,
    pub atlas_page: u32,
    /// 显式满亮，或携带通道存在位的原始光照编码。
    pub light: u32,
}
impl WorldQuad {
    /// 名称共用面片锚点；告示牌文字继续使用原有颜色通道保存源位置。
    pub(crate) fn visibility_anchor(&self) -> [f32; 3] {
        if self.mode & crate::material_mode::NAME_LABEL != 0 {
            self.xyz[3]
        } else {
            [self.rgba[0][0], self.rgba[0][1], self.rgba[0][2]]
        }
    }
    pub(crate) fn single_sided(&self) -> bool {
        self.mode & crate::material_mode::SINGLE_SIDED != 0
    }
    /// 轴向单面可按整批平面范围保守剔除；旋转面和双面始终交给 GPU。
    pub(crate) fn axis_facing(&self) -> u8 {
        if self.mode & crate::material_mode::BILLBOARD != 0 {
            return 0;
        }
        (self.mode >> crate::material_mode::AXIS_FACING_SHIFT & 7) as u8
    }
    pub(crate) fn classify_axis_facing(&mut self) {
        if self.mode & crate::material_mode::BILLBOARD != 0 {
            return;
        }
        let facing = self.compute_axis_facing();
        self.mode = (self.mode & !crate::material_mode::AXIS_FACING_MASK)
            | (u32::from(facing) << crate::material_mode::AXIS_FACING_SHIFT);
    }
    fn compute_axis_facing(&self) -> u8 {
        if !self.single_sided() || self.mode & crate::material_mode::BILLBOARD != 0 {
            return 0;
        }
        let mut result = 0;
        for [i, j, k] in TRIANGLES {
            let a: [f32; 3] = std::array::from_fn(|axis| self.xyz[j][axis] - self.xyz[i][axis]);
            let b: [f32; 3] = std::array::from_fn(|axis| self.xyz[k][axis] - self.xyz[i][axis]);
            let n = [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ];
            if n.iter().all(|v| *v == 0.0) {
                continue;
            }
            let Some(axis) = (0..3).find(|&axis| {
                n[axis] != 0.0 && n[(axis + 1) % 3] == 0.0 && n[(axis + 2) % 3] == 0.0
            }) else {
                return 0;
            };
            if !self.xyz.iter().all(|p| p[axis] == self.xyz[0][axis]) {
                return 0;
            }
            let facing = 1 + axis as u8 * 2 + u8::from(n[axis] < 0.0);
            if result != 0 && result != facing {
                return 0;
            }
            result = facing;
        }
        result
    }
    pub(crate) fn reverse_winding(&mut self) {
        if self.mode & crate::material_mode::BILLBOARD != 0 {
            self.xyz[1] = self.positions()[3];
        } else {
            self.xyz.swap(1, 3);
        }
        self.uv.swap(1, 3);
        if !crate::guardian_eye::encoded(&self) {
            self.rgba.swap(1, 3);
        }
        let facing = self.axis_facing();
        if (1..=6).contains(&facing) {
            let flipped = if facing % 2 == 1 {
                facing + 1
            } else {
                facing - 1
            };
            self.mode = (self.mode & !crate::material_mode::AXIS_FACING_MASK)
                | (u32::from(flipped) << crate::material_mode::AXIS_FACING_SHIFT);
        }
    }
    /// 几何和 UV/颜色一同反转，固定 0-2 对角线；不旋转或镜像实际材质。
    pub(crate) fn align_winding(&mut self, normal: [f32; 3]) -> bool {
        if winding_reversed(&self.positions(), normal) {
            self.reverse_winding();
            return true;
        }
        false
    }
    pub(crate) fn ordered_opaque(&self) -> bool {
        self.mode & (crate::material_mode::ORDERED_OPAQUE | crate::material_mode::DEPTH_EQUAL) != 0
            && !self.blended()
    }
    pub(crate) fn blended(&self) -> bool {
        self.mode & 3 == 2
    }
    pub(crate) fn positions(&self) -> [[f32; 3]; 4] {
        crate::billboard::positions(self.xyz, self.mode)
    }
    pub(crate) fn viewed(mut self, basis: crate::billboard::Basis, viewer: [f64; 3]) -> Self {
        if crate::guardian_eye::encoded(&self) {
            let offset = crate::guardian_eye::offset(&self, viewer);
            for point in &mut self.xyz {
                for i in 0..3 {
                    point[i] += offset[i];
                }
            }
            self.rgba = self
                .rgba
                .map(|color| crate::material_mode::vertex_color(self.mode, color));
        }
        self.xyz = crate::billboard::orient(self.xyz, self.mode, basis);
        if self.mode & crate::material_mode::BILLBOARD != 0 {
            self.mode &=
                !(crate::material_mode::BILLBOARD | crate::material_mode::AXIS_FACING_MASK);
        }
        self
    }
    /// 旋转面以真实锚点为球心覆盖所有相机方向；普通源面保留精确 AABB。
    pub(crate) fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let points = self.positions();
        if crate::guardian_eye::encoded(self) {
            let margin = crate::guardian_eye::margin(self);
            return (
                std::array::from_fn(|i| {
                    (points.iter().map(|p| p[i]).fold(f32::INFINITY, f32::min) - margin[i])
                        .next_down()
                }),
                std::array::from_fn(|i| {
                    (points
                        .iter()
                        .map(|p| p[i])
                        .fold(f32::NEG_INFINITY, f32::max)
                        + margin[i])
                        .next_up()
                }),
            );
        }
        if self.mode & crate::material_mode::BILLBOARD != 0 {
            let anchor = self.xyz[3];
            let radius = points
                .iter()
                .map(|p| {
                    (0..3)
                        .map(|i| (f64::from(p[i]) - f64::from(anchor[i])).powi(2))
                        .sum::<f64>()
                        .sqrt()
                })
                .fold(0_f64, f64::max) as f32;
            let radius = radius.next_up();
            return (
                anchor.map(|v| (v - radius).next_down()),
                anchor.map(|v| (v + radius).next_up()),
            );
        }
        (
            std::array::from_fn(|i| points.iter().map(|p| p[i]).fold(f32::INFINITY, f32::min)),
            std::array::from_fn(|i| {
                points
                    .iter()
                    .map(|p| p[i])
                    .fold(f32::NEG_INFINITY, f32::max)
            }),
        )
    }
    pub(crate) fn center(&self) -> [f32; 3] {
        let positions = self.positions();
        std::array::from_fn(|i| positions.iter().map(|p| p[i]).sum::<f32>() * 0.25)
    }
    #[inline]
    pub(crate) fn translated(mut self, delta: [f32; 3]) -> Self {
        if crate::guardian_eye::encoded(&self) {
            for i in 0..3 {
                self.rgba[0][i] += delta[i];
            }
        }
        if self.mode & (crate::material_mode::SIGN_NEAR_OUTLINE | crate::material_mode::FONT_TEXT)
            != 0
        {
            for anchor in &mut self.rgba {
                for i in 0..3 {
                    anchor[i] += delta[i];
                }
            }
        }
        for point in &mut self.xyz {
            for (coordinate, offset) in point.iter_mut().zip(delta) {
                *coordinate += offset;
            }
        }
        self
    }
    /// 面片与文字按源锚点保留排序身份；转向不会改变源顺序或要求重建网格。
    pub(crate) fn sort_center(&self) -> [f32; 3] {
        if self.mode & crate::material_mode::BILLBOARD != 0 {
            self.xyz[3]
        } else if self.mode & crate::material_mode::FONT_TEXT != 0 {
            std::array::from_fn(|i| self.rgba[0][i])
        } else {
            self.center()
        }
    }
    /// 仅在命中候选面时于栈上展开；索引与显示缓存仍共享紧凑的四角数据。
    pub(crate) fn triangle(&self, corners: [usize; 3]) -> [Vertex; 3] {
        let points = self.positions();
        let billboard = self.mode & crate::material_mode::BILLBOARD != 0;
        corners.map(|i| Vertex {
            xy: if billboard {
                [self.xyz[3][0], self.xyz[3][1]]
            } else {
                [0.; 2]
            },
            depth: if billboard { self.xyz[3][2] } else { 0. },
            xyz: points[i],
            uv: self.uv[i],
            rgba: self.rgba[i],
            mode: self.mode,
            world_mode: 1,
            atlas_page: self.atlas_page,
            light: self.light,
        })
    }
    #[cfg(test)]
    pub(crate) fn expanded(&self) -> [Vertex; 6] {
        let [a, b] = TRIANGLES.map(|indices| self.triangle(indices));
        [a[0], a[1], a[2], b[0], b[1], b[2]]
    }
    #[cfg(test)]
    pub(crate) fn from_vertices(vertices: &[Vertex]) -> Result<Self, String> {
        if vertices.len() != 6 || vertices.iter().any(|v| v.world_mode != 1) {
            return Err("测试源面必须包含六个世界坐标顶点".into());
        }
        if vertices[0] != vertices[3]
            || vertices[2] != vertices[4]
            || vertices.iter().any(|v| {
                v.mode != vertices[0].mode
                    || v.atlas_page != vertices[0].atlas_page
                    || v.light != vertices[0].light
            })
        {
            return Err("测试源面的共享角点或材质不一致".into());
        }
        let corners = [vertices[0], vertices[1], vertices[2], vertices[5]];
        Ok(Self {
            xyz: if vertices[0].mode & crate::material_mode::BILLBOARD != 0 {
                [
                    corners[0].xyz,
                    corners[1].xyz,
                    corners[2].xyz,
                    [corners[0].xy[0], corners[0].xy[1], corners[0].depth],
                ]
            } else {
                corners.map(|v| v.xyz)
            },
            uv: corners.map(|v| v.uv),
            rgba: corners.map(|v| v.rgba),
            mode: vertices[0].mode,
            atlas_page: vertices[0].atlas_page,
            light: vertices[0].light,
        })
    }
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_corner_attributes_and_material_survive_triangle_expansion() {
        let face = WorldQuad {
            xyz: [
                [-1.0, 0.0, 0.0],
                [1.0, 0.2, 0.0],
                [0.8, 1.0, 0.4],
                [-0.4, 0.8, 0.2],
            ],
            uv: [[0.1, 0.2], [0.9, 0.2], [0.7, 0.8], [0.2, 0.9]],
            rgba: [
                [1.0, 0.0, 0.0, 0.1],
                [0.0, 1.0, 0.0, 0.3],
                [0.0, 0.0, 1.0, 0.7],
                [0.5, 0.8, 0.2, 1.0],
            ],
            mode: 2 | 4 | 16 | (37 << 8),
            atlas_page: 70_000,
            light: crate::lighting::FULLBRIGHT,
        };
        let expanded = face.expanded();
        assert_eq!(WorldQuad::from_vertices(&expanded).unwrap(), face);
        for (vertex, corner) in expanded.iter().zip([0, 1, 2, 0, 2, 3]) {
            assert_eq!(vertex.xyz, face.xyz[corner]);
            assert_eq!(vertex.uv, face.uv[corner]);
            assert_eq!(vertex.rgba, face.rgba[corner]);
            assert_eq!(vertex.mode, face.mode);
            assert_eq!(vertex.atlas_page, 70_000);
        }
        assert_eq!(std::mem::size_of::<WorldQuad>(), 156);
        assert!(std::mem::size_of::<WorldQuad>() * 2 < std::mem::size_of_val(&expanded));
        let moved = face.translated([16.0, -32.0, 48.0]);
        assert_eq!(moved.xyz[2], [16.8, -31.0, 48.4]);
        assert_eq!(moved.uv, face.uv);
        assert_eq!(moved.rgba, face.rgba);
    }

    fn standalone_triangle_keeps_degenerate_second_triangle_and_rejects_inconsistent_data() {
        let mut face = WorldQuad {
            xyz: [[0.0; 3]; 4],
            uv: [[0.0; 2]; 4],
            rgba: [[1.0; 4]; 4],
            mode: 0,
            atlas_page: 0,
            light: crate::lighting::FULLBRIGHT,
        };
        face.xyz[1] = [1.0, 0.0, 0.0];
        face.xyz[2] = [0.0, 1.0, 0.0];
        face.xyz[3] = face.xyz[2];
        assert_eq!(face.expanded()[4], face.expanded()[5]);
        let mut vertices = face.expanded();
        vertices[3].rgba[0] = 0.0;
        assert!(WorldQuad::from_vertices(&vertices).is_err());
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::WorldGeometry {
            full_corner_attributes_and_material_survive_triangle_expansion();
            standalone_triangle_keeps_degenerate_second_triangle_and_rejects_inconsistent_data();
        }
    }
}
