//! 原生流体的 PNG 选择与 UV；世界透视、投影 GPU、Canvas 和拾取共用。
use crate::{
    gpu_mesh::FaceStyle,
    isometric::Face,
    textures::{AlphaMode, AtlasUv, TextureBank, MISSING_TEXTURE_ID},
};
use mcedit2_world::{
    fluid_surface::{FluidSurface, Kind},
    sections::State,
    surface::SurfaceVoxel,
};

#[derive(Clone, Copy)]
pub(crate) struct Style {
    pub face: FaceStyle,
    pub tint: [f32; 3],
}

#[derive(Clone, Copy)]
struct Sprite<'a> {
    slot: &'a AtlasUv,
    alpha: AlphaMode,
    missing: bool,
}
/// 一次后台建模 / 点选复用四张流体图片、侧面 overlay 与缺失材质。
pub(crate) struct Cache<'a> {
    bank: Option<&'a TextureBank>,
    sprites: [Option<Option<Sprite<'a>>>; 6],
    invalid_reported: bool,
}
impl<'a> Cache<'a> {
    pub(crate) fn new(bank: Option<&'a TextureBank>) -> Self {
        Self {
            bank,
            sprites: [None; 6],
            invalid_reported: false,
        }
    }
    fn sprite(&mut self, index: usize) -> Option<Sprite<'a>> {
        if let Some(sprite) = self.sprites[index] {
            return sprite;
        }
        let bank = self.bank?;
        let atlas = bank.atlas()?;
        let id = [
            "minecraft:water_still",
            "minecraft:water_flow",
            "minecraft:lava_still",
            "minecraft:lava_flow",
            "minecraft:water_overlay",
            MISSING_TEXTURE_ID,
        ][index];
        let (slot, missing) = if let Some(slot) = atlas.entries.get(id) {
            (slot, index == 5)
        } else {
            log::error!("[fluid-material] 流体贴图未进入图集：{id}；显示缺失材质");
            let Some(slot) = atlas.entries.get(MISSING_TEXTURE_ID) else {
                log::error!("[fluid-material] 图集同时缺少缺失材质图片");
                self.sprites[index] = Some(None);
                return None;
            };
            (slot, true)
        };
        let alpha = if missing {
            AlphaMode::Opaque
        } else {
            let Some(alpha) = bank.sprite_alpha(id) else {
                log::error!("[fluid-material] 已打包流体贴图缺少 Alpha 分类：{id}");
                self.sprites[index] = Some(None);
                return None;
            };
            alpha
        };
        let sprite = Some(Sprite {
            slot,
            alpha,
            missing,
        });
        self.sprites[index] = Some(sprite);
        sprite
    }
    pub(crate) fn style(
        &mut self,
        voxel: &SurfaceVoxel,
        state: &State,
        face: Face,
    ) -> Option<Style> {
        self.style_in(voxel, state, face, None)
    }
    pub(crate) fn style_in(
        &mut self,
        voxel: &SurfaceVoxel,
        state: &State,
        face: Face,
        colors: Option<&mut crate::textures::TintContext<'_>>,
    ) -> Option<Style> {
        if voxel.span != 1 {
            return None;
        }
        let bank = self.bank?;
        let surface = voxel.fluid_surface()?;
        let base = match surface.kind() {
            Kind::Water => 0,
            Kind::Lava => 2,
        };
        let (raw, flowing) = coordinates(surface, face);
        let index = if bank.procedural_fluid_supported(state, (voxel.x, voxel.y, voxel.z)) {
            if surface.overlay_faces() & face.flag() != 0 {
                4
            } else {
                base + usize::from(flowing)
            }
        } else {
            if !self.invalid_reported {
                log::error!(
                    "[fluid-material] 原生流体声明的模型无法解释：{}；显示缺失材质",
                    state.name
                );
                self.invalid_reported = true;
            }
            5
        };
        let sprite = self.sprite(index)?;
        let raw = if sprite.missing {
            [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
        } else {
            raw
        };
        Some(Style {
            face: (
                raw.map(|p| sprite.slot.sample(p[0], p[1])),
                sprite.alpha,
                sprite.slot.page,
            ),
            tint: if sprite.missing {
                [1.0; 3]
            } else {
                colors.map_or_else(
                    || bank.native_fluid_tint(surface.kind(), voxel.biome.as_deref()),
                    |colors| {
                        colors.fluid(
                            surface.kind(),
                            voxel.biome.as_deref(),
                            (voxel.x, voxel.y, voxel.z),
                        )
                    },
                )
            },
        })
    }
}

/// UV 始终对应实际生成的顶点顺序；侧面使用 flow 图的半幅并按液面高度裁取。
fn coordinates(surface: FluidSurface, face: Face) -> ([[f32; 2]; 4], bool) {
    let vertices = crate::models::fluid_face_vertices(surface, face);
    let [x, z] = surface.flow();
    let moving = x != 0.0 || z != 0.0;
    let original = face.corners();
    let raw = std::array::from_fn(|index| {
        let p = vertices[index];
        let v = if original[index][1] < 0.0 {
            0.5
        } else {
            (0.5 - p[1]) * 0.5
        };
        match face {
            Face::Top if moving => {
                // 等价于 atan2(z, x) - PI/2 的正弦 / 余弦，流向已在源阶段归一化。
                let (sin, cos) = (-x * 0.25, z * 0.25);
                let (sx, sz) = (p[0] * 2.0, p[2] * 2.0);
                [0.5 + sx * cos + sz * sin, 0.5 + sx * -sin + sz * cos]
            }
            Face::Top | Face::Bottom => [p[0] + 0.5, p[2] + 0.5],
            Face::NegZ => [(0.5 - p[0]) * 0.5, v],
            Face::PosZ => [(p[0] + 0.5) * 0.5, v],
            Face::PosX => [(0.5 - p[2]) * 0.5, v],
            Face::NegX => [(p[2] + 0.5) * 0.5, v],
        }
    });
    (
        raw,
        if matches!(face, Face::Top | Face::Bottom) {
            face == Face::Top && moving
        } else {
            true
        },
    )
}
