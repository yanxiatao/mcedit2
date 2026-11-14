//! 已保存实体的真实眼位与原版亮度修饰，不以渲染模型位置猜测光照。
use crate::{entity_catalog::EntityVisual, nbt_document::Value, saved_light::Levels};

#[derive(Debug, Clone, Copy)]
pub enum Object {
    DroppedItem,
    ExperienceOrb,
    ItemFrame { glow: bool },
}
#[derive(Debug, Clone, Copy)]
pub struct Probe {
    pub position: [i32; 3],
    block_units: u16,
    block_minimum: u8,
    fire: bool,
    legacy: bool,
    marker_volume: Option<[[i32; 3]; 2]>,
    packed_fullbright: bool,
    fixed_levels: Option<Levels>,
}
/// 保留原版 packed 光照的 1/16 级余量；旧版经验球的增量为 7.5 级而非整数 7。
#[derive(Debug, Clone, Copy)]
pub struct Brightness {
    levels: Levels,
    block_fraction: u8,
}
impl Brightness {
    pub fn levels(self) -> Levels {
        self.levels
    }
    pub fn block_fraction(self) -> u8 {
        self.block_fraction
    }
}
impl Probe {
    pub fn supports_living(id: &str) -> bool {
        crate::entity_physics::supports(id)
    }
    /// 已核实的实体类型共用同一分派；物品 payload 的 ID 不参与眼位选择。
    pub fn read_entity(
        id: &str,
        visual: &EntityVisual,
        position: [f64; 3],
    ) -> Result<Self, String> {
        if crate::entity_catalog::is_dropped(id) {
            Self::read(Object::DroppedItem, visual, position)
        } else if crate::entity_catalog::is_experience_orb(id) {
            Self::read(Object::ExperienceOrb, visual, position)
        } else if crate::hanging::is_frame(id) {
            let source = crate::hanging::Frame::read(visual)?;
            Self::read(
                Object::ItemFrame {
                    glow: id == "minecraft:glow_item_frame",
                },
                visual,
                source.center(),
            )
        } else if Self::supports_living(id) {
            let body = crate::entity_physics::read(id, visual)?;
            let emissive = matches!(
                id,
                "minecraft:magma_cube"
                    | "minecraft:blaze"
                    | "minecraft:vex"
                    | "minecraft:allay"
                    | "minecraft:wither"
                    | "minecraft:shulker_bullet"
                    | "minecraft:wither_skull"
                    | "minecraft:dragon_fireball"
            );
            let emissive = emissive
                || crate::entity_projectiles::Projectile::read(id)
                    .is_some_and(|projectile| projectile.fullbright());
            let minimum = if id == "minecraft:glow_squid" {
                glow_squid_light(visual)?
            } else if emissive {
                15
            } else {
                0
            };
            let mut probe = Self::from_eye(visual, position, body.eye, 0, minimum)?;
            if crate::entity_display::Display::read(id).is_some() {
                probe.fixed_levels = crate::entity_display::Display::brightness(visual)?;
            }
            // 旧实体覆盖完整 packed light；现代 renderer 只覆盖方块光。
            probe.packed_fullbright = emissive && visual.source_version == Some(1343);
            if body.fire_immune {
                probe.fire = false;
            }
            if let Some([width, height]) = body.marker_volume {
                let half = f64::from(width) / 2.;
                probe.marker_volume = Some([
                    point([position[0] - half, position[1], position[2] - half])?,
                    point([
                        position[0] + half,
                        position[1] + f64::from(height),
                        position[2] + half,
                    ])?,
                ]);
            }
            Ok(probe)
        } else {
            Err(format!("实体 {id} 没有此类已核实的光照绑定"))
        }
    }
    pub fn read(object: Object, visual: &EntityVisual, position: [f64; 3]) -> Result<Self, String> {
        let version = visual.source_version.ok_or("实体光照缺少源 DataVersion")?;
        let legacy = match version {
            1343 => true,
            3465 | 3955 => false,
            _ => return Err(format!("未核实 DataVersion={version} 的实体光照眼位")),
        };
        let (eye, block_units, block_minimum) = match object {
            Object::DroppedItem => (0.25_f32 * 0.85_f32, 0, 0),
            Object::ExperienceOrb => (
                (if legacy { 0.25_f32 } else { 0.5_f32 }) * 0.85_f32,
                if legacy { 120 } else { 112 },
                0,
            ),
            Object::ItemFrame { glow } => {
                if legacy && glow {
                    return Err("旧版没有发光展示框".into());
                }
                (
                    if legacy { 0.5_f32 * 0.85_f32 } else { 0. },
                    0,
                    if glow { 5 } else { 0 },
                )
            }
        };
        let mut probe = Self::from_eye(visual, position, eye, block_units, block_minimum)?;
        if probe.fire && !legacy && matches!(object, Object::DroppedItem) {
            probe.fire = !crate::item_fire::resistant(visual, version)?;
        }
        Ok(probe)
    }
    fn from_eye(
        visual: &EntityVisual,
        position: [f64; 3],
        eye: f32,
        block_units: u16,
        block_minimum: u8,
    ) -> Result<Self, String> {
        let position = point([position[0], position[1] + f64::from(eye), position[2]])?;
        let fire = match visual.source_field_optional("Fire")? {
            None => false,
            Some(Value::Short(value)) => value > 0,
            _ => return Err("实体 Fire 必须为 Short".into()),
        };
        Ok(Self {
            position,
            block_units,
            block_minimum,
            fire,
            legacy: visual.source_version == Some(1343),
            marker_volume: None,
            packed_fullbright: false,
            fixed_levels: None,
        })
    }
    /// 所有候选按原生 BlockPos.betweenClosed 的 X / Y / Z 步进顺序捕获。
    pub fn positions(&self) -> impl Iterator<Item = [i32; 3]> {
        let [low, high] = self.marker_volume.unwrap_or([self.position; 2]);
        let constant =
            self.fixed_levels.is_some() || self.packed_fullbright || self.fire && self.legacy;
        (low[2]..=high[2])
            .flat_map(move |z| {
                (low[1]..=high[1]).flat_map(move |y| (low[0]..=high[0]).map(move |x| [x, y, z]))
            })
            .filter(move |_| !constant)
    }
    pub fn sample(
        &self,
        source: &crate::scene_annotations::Lighting,
    ) -> Result<Option<Brightness>, String> {
        if let Some(levels) = self.fixed_levels {
            return Ok(Some(Brightness {
                levels,
                block_fraction: 0,
            }));
        }
        // 原生已覆盖完整 packed light 时不读取无关的保存通道；独立预览保持其显式语义。
        if self.packed_fullbright || self.fire && self.legacy {
            return Ok(match source {
                crate::scene_annotations::Lighting::Preview => None,
                crate::scene_annotations::Lighting::Saved { .. } => Some(self.brightness(Levels {
                    block: None,
                    sky: None,
                })),
            });
        }
        if self.marker_volume.is_none() {
            return source
                .sample(self.position)
                .map(|levels| levels.map(|l| self.brightness(l)))
                .map_err(|e| e.to_string());
        }
        let Some(skylight) = source.has_skylight()? else {
            return Ok(None);
        };
        let mut selected = None;
        let mut maximum = -1;
        for position in self.positions() {
            let levels = source
                .sample(position)
                .map_err(|e| e.to_string())?
                .ok_or("世界 Marker 光照探针缺少保存来源")?;
            let block = levels.block.ok_or("Marker 盔甲架的候选源格缺少方块光")?;
            let sky = if skylight {
                levels.sky.ok_or("Marker 盔甲架的候选源格缺少天空光")?
            } else {
                0
            };
            let brightness = i32::from(block.max(sky));
            if brightness > maximum {
                maximum = brightness;
                selected = Some(levels);
            }
            if maximum == 15 {
                break;
            }
        }
        selected
            .map(|levels| Some(self.brightness(levels)))
            .ok_or_else(|| "Marker 盔甲架没有实际光照候选源格".into())
    }
    pub fn brightness(&self, mut levels: Levels) -> Brightness {
        if let Some(fixed) = self.fixed_levels {
            return Brightness {
                levels: fixed,
                block_fraction: 0,
            };
        }
        if self.packed_fullbright || self.fire && self.legacy {
            return Brightness {
                levels: Levels {
                    block: Some(15),
                    sky: Some(15),
                },
                block_fraction: 0,
            };
        }
        if self.fire || self.block_minimum == 15 {
            levels.block = Some(15);
        }
        let mut block_fraction = 0;
        if let Some(block) = levels.block {
            let units = (u16::from(block.max(self.block_minimum)) * 16 + self.block_units).min(240);
            levels.block = Some((units / 16) as u8);
            block_fraction = (units % 16) as u8;
        }
        Brightness {
            levels,
            block_fraction,
        }
    }
}
/// GlowSquidRenderer 的 clampedLerp(0, 15, 1 - darkTicks / 10)，再按 Java 向零取整。
/// 这是方块光下限；不会覆盖实际天空光，也不会按伤害倒计时改变纹理颜色。
fn glow_squid_light(visual: &EntityVisual) -> Result<u8, String> {
    let ticks = match visual.source_field_optional("DarkTicksRemaining")? {
        None => 0,
        Some(Value::Int(ticks)) => ticks,
        _ => return Err("发光鱿鱼 DarkTicksRemaining 必须为 Int".into()),
    };
    let factor = (1.0_f32 - ticks as f32 / 10.0_f32).clamp(0., 1.);
    Ok((factor * 15.0_f32) as u8)
}

fn point(position: [f64; 3]) -> Result<[i32; 3], String> {
    let mut point = [0; 3];
    for i in 0..3 {
        let value = position[i].floor();
        if !value.is_finite() || value < i32::MIN as f64 || value > i32::MAX as f64 {
            return Err("实体光照眼位超出可表示源坐标".into());
        }
        point[i] = value as i32;
    }
    Ok(point)
}
