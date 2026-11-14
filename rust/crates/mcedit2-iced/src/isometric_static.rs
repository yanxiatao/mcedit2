//! 等轴 GPU、Canvas 和拾取共用的静态模型投影及材质。
use super::*;
use crate::textures::StaticObjTriangle;

pub(super) struct Geometry {
    triangles: Vec<Arc<StaticObjTriangle>>,
    pub extent: f32,
}
impl Geometry {
    pub fn new(triangles: Vec<StaticObjTriangle>) -> Self {
        let extent = triangles
            .iter()
            .flat_map(|t| t.xyz.iter().flatten())
            .fold(0.0f32, |max, value| max.max(value.abs()));
        Self {
            triangles: triangles.into_iter().map(Arc::new).collect(),
            extent,
        }
    }
    /// 以实际三角形投影，不把 OBJ 转换成包围盒或占位方块。
    pub fn append(
        &self,
        view: &IsometricSurface,
        projection: crate::orthographic_framing::Projection,
        voxel_index: usize,
        size: Size,
        out: &mut Vec<RenderFace>,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<()> {
        let voxel = &view.surface.voxels[voxel_index];
        let (yaw, pitch) = view.angles();
        let look = [
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            yaw.cos() * pitch.cos(),
        ];
        for triangle in &self.triangles {
            if cancelled() {
                return None;
            }
            if triangle
                .cull
                .is_some_and(|face| voxel.faces & face.flag() == 0)
            {
                continue;
            }
            let a: [f32; 3] = std::array::from_fn(|i| triangle.xyz[1][i] - triangle.xyz[0][i]);
            let b: [f32; 3] = std::array::from_fn(|i| triangle.xyz[2][i] - triangle.xyz[0][i]);
            let normal = [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ];
            if triangle.mode & crate::material_mode::SINGLE_SIDED != 0
                && normal.iter().zip(look).map(|(a, b)| a * b).sum::<f32>() <= 0.0
            {
                continue;
            }
            let projected = triangle.xyz.map(|local| {
                projection.project([
                    voxel.x as f64 + local[0] as f64,
                    voxel.y as f64 + local[1] as f64,
                    voxel.z as f64 + local[2] as f64,
                ])
            });
            let corners = [
                projected[0].0,
                projected[1].0,
                projected[2].0,
                projected[2].0,
            ];
            if corners.iter().all(|p| p.x < 0.0)
                || corners.iter().all(|p| p.x > size.width)
                || corners.iter().all(|p| p.y < 0.0)
                || corners.iter().all(|p| p.y > size.height)
            {
                continue;
            }
            // 主轴只用于编辑时的邻接方向；投影与深度始终取实际三角形。
            let axis = (0..3)
                .max_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
                .unwrap();
            let face = match (axis, normal[axis] >= 0.0) {
                (0, true) => Face::PosX,
                (0, false) => Face::NegX,
                (1, true) => Face::Top,
                (1, false) => Face::Bottom,
                (_, true) => Face::PosZ,
                (_, false) => Face::NegZ,
            };
            out.push(RenderFace {
                model_state: None,
                voxel_index,
                element_index: usize::MAX,
                static_triangle: Some(Arc::clone(triangle)),
                fluid_layer: false,
                pos: (voxel.x, voxel.y, voxel.z),
                face,
                corners,
                vertex_depths: [
                    projected[0].1,
                    projected[1].1,
                    projected[2].1,
                    projected[2].1,
                ],
                depth: projected.iter().map(|(_, depth)| depth).sum::<f32>() / 3.0,
                translucent: triangle.mode & 3 == 2,
                color: material_color(voxel),

                block_entity: voxel.original_block_entity,
                staged: voxel.staged,
            });
        }
        Some(())
    }
}

pub(crate) struct Style {
    pub uv: [[f32; 2]; 3],
    pub rgba: [[f32; 4]; 3],
    pub mode: u32,
    pub page: u32,
}
impl RenderFace {
    pub(crate) fn state<'a>(
        &'a self,
        source: &'a SurfaceVoxel,
    ) -> &'a mcedit2_world::sections::State {
        self.model_state.as_deref().unwrap_or(&source.state)
    }
    #[cfg(test)]
    pub(crate) fn static_style(&self, view: &IsometricSurface) -> Option<Style> {
        let mut colors = view
            .texture_bank
            .as_ref()
            .map(|b| b.tint_context(&view.surface.biomes));
        self.static_style_in(view, colors.as_mut())
    }
    pub(crate) fn static_style_in(
        &self,
        view: &IsometricSurface,
        colors: Option<&mut crate::textures::TintContext<'_>>,
    ) -> Option<Style> {
        let triangle = self.static_triangle.as_ref()?;
        let voxel = &view.surface.voxels[self.voxel_index];
        let tint = if let Some(tint_index) = triangle.tint_index {
            colors.map_or([1.0; 3], |colors| {
                colors.block(&voxel.state, tint_index, voxel.biome.as_deref(), self.pos)
            })
        } else {
            [1.0; 3]
        };
        let rgba = triangle.rgba.map(|mut rgba| {
            for i in 0..3 {
                rgba[i] *= tint[i];
            }
            if !view.show_textures {
                rgba[0] *= self.color.r;
                rgba[1] *= self.color.g;
                rgba[2] *= self.color.b;
                rgba[3] *= self.color.a;
            }
            rgba
        });
        let atlas = view.texture_bank.as_ref().and_then(|bank| bank.atlas());
        let white = atlas.as_ref().map(|atlas| &atlas.white);
        Some(Style {
            uv: if view.show_textures {
                triangle.uv
            } else {
                [white.map_or([0.5; 2], |uv| uv.sample(0.5, 0.5)); 3]
            },
            page: if view.show_textures {
                triangle.page
            } else {
                white.map_or(0, |uv| uv.page)
            },
            rgba,
            mode: triangle.mode,
        })
    }
    pub(super) fn static_covered(&self, view: &IsometricSurface, p: Point) -> Option<()> {
        // 普通材质只读取 Alpha；加色层按实际 RGB 贡献判断覆盖。
        let style = self.static_style_in(view, None)?;
        let weights = triangle_weights(p, [self.corners[0], self.corners[1], self.corners[2]])?;
        let uv = std::array::from_fn(|axis| (0..3).map(|i| style.uv[i][axis] * weights[i]).sum());
        let pixel = view
            .texture_bank
            .as_ref()?
            .atlas_sample(uv, style.page, view.gpu_quality)?;
        let rgba = std::array::from_fn(|axis| {
            pixel[axis]
                * (0..3)
                    .map(|i| style.rgba[i][axis] * weights[i])
                    .sum::<f32>()
        });
        crate::material_mode::covered_color_f32(style.mode, rgba).then_some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::textures::java_obj_tests::{fixture, isometric_scene};
    use image::ImageEncoder;

    #[test]
    fn native_composite_keeps_static_json_geometry_materials_and_source_picking() {
        let fixture = fixture();
        let view = isometric_scene(fixture.open());
        let size = Size::new(800.0, 600.0);
        let faces = view.faces_cached(size);
        assert_eq!(faces.len(), 3);
        assert_eq!(
            faces.iter().filter(|f| f.static_triangle.is_some()).count(),
            2
        );
        let vertices = crate::gpu_mesh::build_cancellable(&view, size, &|| false).unwrap();
        assert_eq!(vertices.len(), 18, "两个实际三角形和一个 JSON 四边形");
        let mut offset = 0;
        for face in &*faces {
            if let Some(style) = face.static_style(&view) {
                for (vertex, i) in vertices[offset..offset + 6].iter().zip([0, 1, 2, 0, 2, 2]) {
                    assert_eq!(vertex.xy, [face.corners[i].x, face.corners[i].y]);
                    assert_eq!(vertex.uv, style.uv[i]);
                    assert_eq!(vertex.rgba, style.rgba[i]);
                    assert_eq!(vertex.mode, style.mode);
                    assert_eq!(vertex.atlas_page, style.page);
                }
                let center = Point::new(
                    face.corners[..3].iter().map(|p| p.x).sum::<f32>() / 3.0,
                    face.corners[..3].iter().map(|p| p.y).sum::<f32>() / 3.0,
                );
                assert_eq!(view.pick(center, size), Some((0, 0, 0)));
                offset += 6;
            } else {
                offset += 6;
            }
        }
        let mut plain = view.clone();
        plain.show_textures = false;
        let without = plain.faces(size);
        assert_eq!(without.len(), faces.len());
        for (a, b) in faces.iter().zip(&without) {
            assert_eq!(a.corners, b.corners);
            assert_eq!(a.vertex_depths, b.vertex_depths);
        }
        let mut behind = view.clone();
        behind.yaw_degrees = 0;
        assert!(
            behind
                .faces(size)
                .iter()
                .all(|f| f.static_triangle.is_none()),
            "单面 OBJ 背面应剔除"
        );
    }

    #[test]
    fn authored_solid_and_transparent_vertex_alpha_match_canvas_and_picking() {
        let fixture = fixture();
        // PNG 与顶点都有 Alpha；solid 必须仍可见，translucent 必须拒绝透明像素。
        for name in ["red", "blue"] {
            let mut bytes = Vec::new();
            image::codecs::png::PngEncoder::new(&mut bytes)
                .write_image(&[255, 255, 255, 0], 1, 1, image::ExtendedColorType::Rgba8)
                .unwrap();
            fixture.put(&format!("assets/demo/textures/item/{name}.png"), &bytes);
        }
        let view = isometric_scene(fixture.open());
        let size = Size::new(800.0, 600.0);
        for face in view
            .faces_cached(size)
            .iter()
            .filter(|f| f.static_triangle.is_some())
        {
            let center = Point::new(
                face.corners[..3].iter().map(|p| p.x).sum::<f32>() / 3.0,
                face.corners[..3].iter().map(|p| p.y).sum::<f32>() / 3.0,
            );
            let style = face.static_style(&view).unwrap();
            let solid = style.mode & crate::material_mode::FORCE_OPAQUE_ALPHA != 0;
            assert_eq!(face.static_covered(&view, center).is_some(), solid);
            assert_eq!(view.pick(center, size).is_some(), solid);
        }
    }

    #[test]
    fn static_source_culling_and_shared_geometry_are_preserved() {
        let fixture = fixture();
        fixture.put(
            "assets/demo/models/block/assembly.json",
            br#"{"parent":"demo:block/base"}"#,
        );
        let mut view = isometric_scene(fixture.open());
        let size = Size::new(800.0, 600.0);
        let surface = Arc::make_mut(&mut view.surface);
        let mut second = surface.voxels[0].clone();
        second.x = 2;
        surface.voxels.push(second);
        let faces = view.faces(size);
        assert_eq!(faces.len(), 2);
        assert!(Arc::ptr_eq(
            faces[0].static_triangle.as_ref().unwrap(),
            faces[1].static_triangle.as_ref().unwrap()
        ));
        Arc::make_mut(&mut view.surface).voxels[0].faces = 0;
        assert!(
            view.faces(size).iter().all(|f| f.pos.0 == 2),
            "原始相邻面遮挡不能丢失"
        );
    }

    #[test]
    fn root_geometry_outside_source_voxel_uses_its_real_projected_envelope() {
        let fixture = fixture();
        fixture.put("assets/demo/models/block/assembly.json",br#"{"parent":"demo:block/base","transform":{"translation":[20,0,0],"origin":"center"}}"#);
        let mut view = isometric_scene(fixture.open());
        let size = Size::new(800.0, 600.0);
        let voxel = &view.surface.voxels[0];
        let triangles = view
            .texture_bank
            .as_ref()
            .unwrap()
            .static_obj_at(&voxel.state, (voxel.x, voxel.y, voxel.z))
            .unwrap();
        let mut center = [0.; 3];
        for point in triangles[0].xyz {
            for i in 0..3 {
                center[i] += f64::from(point[i]) / 3.;
            }
        }
        view.zoom = 4.;
        let projected = view.project(center, size).0;
        view.pan_x = size.width / 2. - projected.x;
        view.pan_y = size.height / 2. - projected.y;
        let source = view.project([0.; 3], size).0;
        assert!(source.x < 0. || source.x > size.width || source.y < 0. || source.y > size.height);
        let faces = view.faces_cached(size);
        assert_eq!(faces.len(), 1, "源方块已离屏时仍需显示其进入视口的真实模型");
        assert!(faces[0].static_triangle.is_some());
        let center = Point::new(
            faces[0].corners[..3].iter().map(|p| p.x).sum::<f32>() / 3.0,
            faces[0].corners[..3].iter().map(|p| p.y).sum::<f32>() / 3.0,
        );
        assert_eq!(view.pick(center, size), Some((0, 0, 0)));
    }

    #[test]
    fn dense_model_has_one_source_outline_instead_of_internal_triangle_edges() {
        let fixture = fixture();
        let mut view = isometric_scene(fixture.open());
        let cached = view.faces_cached(Size::new(800.0, 600.0));
        let dense: Vec<_> = cached.iter().cycle().take(5000).cloned().collect();
        assert!(view.source_outlines(&dense).is_empty());
        view.selected = Some((0, 0, 0));
        let outlines = view.source_outlines(&dense);
        assert_eq!(outlines.len(), 1);
        assert_eq!(
            outlines[0].corners,
            IsometricSurface::bounds_corners([0; 3], [0; 3])
        );
        assert_eq!(outlines[0].tint, Color::from_rgb(0.10, 1.0, 0.91));
        assert_eq!(outlines[0].width, 2.7);
        view.selected = None;
        view.hovered = Some((0, 0, 0));
        let hover = view.source_outlines(&dense);
        assert_eq!(hover.len(), 1);
        assert_eq!(hover[0].tint, Color::from_rgb(1.0, 0.90, 0.31));
        assert_eq!(hover[0].width, 1.8);
        assert!(
            view.source_outlines(&[]).is_empty(),
            "无可见源面时不能宣称有源方块轮廓"
        );
    }

    #[test]
    fn source_outlines_keep_edit_status_and_never_highlight_sampled_lod_tiles() {
        let fixture = fixture();
        let mut view = isometric_scene(fixture.open());
        let voxel = &mut Arc::make_mut(&mut view.surface).voxels[0];
        voxel.staged = true;
        let size = Size::new(800.0, 600.0);
        let staged = view.source_outlines(&view.faces(size));
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].tint, Color::from_rgb(0.19, 0.85, 0.99));
        Arc::make_mut(&mut view.surface).voxels[0].original_block_entity = true;
        let faces = view.faces(size);
        let entity = view.source_outlines(&faces);
        assert_eq!(entity.len(), 1);
        assert_eq!(entity[0].tint, Color::from_rgb(1.0, 0.62, 0.20));
        view.selected = Some((0, 0, 0));
        Arc::make_mut(&mut view.surface).voxels[0].span = 16;
        assert!(view.source_outlines(&faces).is_empty());
    }
}
