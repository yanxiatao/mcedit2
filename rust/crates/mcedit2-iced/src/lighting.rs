//! 全亮度显示常量及共享 GPU 参数；显示入口不计算或采集世界光照。
use mcedit2_world::saved_light::Levels;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, OnceLock, Weak};

pub(crate) const FULLBRIGHT: u32 = 0;
const FIXED: u32 = 1 << 31;
/// 不读取光照贴图的固定层；与 packed 15 / 15 查表值分别表达。
pub(crate) const UNLIT: u32 = FIXED;
fn is_unlit(code: u32) -> bool {
    code & !FIXED == 0
}
const SAVED: u32 = 0x400;
const BLOCK_PRESENT: u32 = 0x100;
const SKY_PRESENT: u32 = 0x200;
/// 原版 packed FULL_BRIGHT：仍查光照表，且不被父对象的来源光照覆盖。
pub(crate) const PACKED_FULLBRIGHT: u32 = FIXED | SAVED | BLOCK_PRESENT | SKY_PRESENT | 0xff;

pub(crate) fn fixed(code: u32) -> u32 {
    if code == FULLBRIGHT {
        code
    } else {
        FIXED | code
    }
}
/// 原始通道与存在位原样编码；不在传输时把未知光照改成满亮。
pub(crate) fn encode(levels: Levels) -> u32 {
    SAVED
        | levels.block.map_or(0, |n| u32::from(n) | BLOCK_PRESENT)
        | levels.sky.map_or(0, |n| (u32::from(n) << 4) | SKY_PRESENT)
}
pub(crate) fn shade(color: iced::Color, light: [f32; 3]) -> iced::Color {
    iced::Color::from_rgba(
        color.r * light[0],
        color.g * light[1],
        color.b * light[2],
        color.a,
    )
}
pub(crate) fn error_geometry(
    renderer: &iced::Renderer,
    size: iced::Size,
    error: &str,
) -> iced::widget::canvas::Geometry {
    let mut frame = iced::widget::canvas::Frame::new(renderer, size);
    frame.fill_text(iced::widget::canvas::Text {
        content: format!("无法生成光照预览：{error}"),
        color: iced::Color::from_rgb(1., 0.4, 0.35),
        ..Default::default()
    });
    frame.into_geometry()
}
/// 视图文件中的光照参数记录；画面固定使用全亮度表，不应用保存的光照开关。

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub enabled: bool,
    pub daylight: f32,
    pub gamma: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            daylight: 1.,
            gamma: 1.,
        }
    }
}
impl Settings {
    pub(crate) fn validate(self) -> Result<(), String> {
        if !self.daylight.is_finite()
            || !(0. ..=1.).contains(&self.daylight)
            || !self.gamma.is_finite()
            || !(0. ..=3.).contains(&self.gamma)
        {
            return Err("预览时间必须为 0..1，亮度必须为 0..3".into());
        }
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Uniform {
    pub control: [u32; 4],
    pub colors: [[f32; 4]; 256],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Key([u32; 6]);

/// 租约由真实视图状态持有，不能由 iced 的共享管线保活。
#[derive(Debug, Default)]
pub(crate) struct Report {
    table: Mutex<Weak<Table>>,
}
/// 不可变几何只记录通道缺失情况，参数更新时不重新遍历所有源面。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Usage {
    missing: u32,
}
impl Usage {
    pub(crate) fn observe(&mut self, code: u32) {
        if !is_unlit(code) {
            self.missing |= !code & (SAVED | BLOCK_PRESENT | SKY_PRESENT);
        }
    }
    pub(crate) fn check(self, table: &Table) -> Result<(), String> {
        if !table.settings.enabled {
            return Ok(());
        }
        let required = SAVED | BLOCK_PRESENT | if table.has_skylight { SKY_PRESENT } else { 0 };
        if self.missing & required != 0 {
            return Err("可见源几何缺少该维度需要的保存光照，不能生成光照预览".into());
        }
        Ok(())
    }
}

pub(crate) struct Table {
    pub settings: Settings,
    pub minimum: f32,
    pub the_end: bool,
    pub has_skylight: bool,
    uniform: Uniform,
    errors: Mutex<Vec<(Weak<Report>, String)>>,
}
impl std::fmt::Debug for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Table")
            .field("settings", &self.settings)
            .field("minimum", &self.minimum)
            .field("the_end", &self.the_end)
            .field("has_skylight", &self.has_skylight)
            .finish()
    }
}
impl Default for Table {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            minimum: 0.,
            the_end: false,
            has_skylight: true,
            uniform: Uniform {
                control: [0, 1_f32.to_bits(), 0, 0],
                colors: [[1.; 4]; 256],
            },
            errors: Mutex::new(Vec::new()),
        }
    }
}
impl Table {
    pub(crate) fn uniform(&self) -> &Uniform {
        &self.uniform
    }
    pub(crate) fn unlit() -> Arc<Self> {
        static TABLE: OnceLock<Arc<Table>> = OnceLock::new();
        Arc::clone(TABLE.get_or_init(|| Arc::new(Self::default())))
    }
    pub(crate) fn key(&self) -> Key {
        if !self.settings.enabled {
            return Key([0, self.settings.daylight.to_bits(), 0, 0, 0, 0]);
        }
        Key([
            1,
            self.settings.daylight.to_bits(),
            self.settings.gamma.to_bits(),
            self.minimum.to_bits(),
            u32::from(self.the_end),
            u32::from(self.has_skylight),
        ])
    }
    pub(crate) fn enter(self: &Arc<Self>, owner: &Arc<Report>) {
        let next = Arc::downgrade(self);
        let mut previous = owner.table.lock().expect("光照视图归属锁被毒化");
        if previous.ptr_eq(&next) {
            return;
        }
        let old = std::mem::replace(&mut *previous, next);
        drop(previous);
        if let Some(old) = old.upgrade() {
            let mut errors = old.errors.lock().expect("光照诊断锁被毒化");
            let before = errors.len();
            let owner = Arc::downgrade(owner);
            errors.retain(|(entry, _)| !entry.ptr_eq(&owner));
            let changed = before != errors.len();
            drop(errors);
            if changed {
                crate::render_wake::notify();
            }
        }
    }
    /// 错误按真实视图租约记录；关闭视图后弱引用退出，不受共享管线寿命影响。
    pub(crate) fn report(self: &Arc<Self>, owner: &Arc<Report>, result: Result<(), String>) {
        self.enter(owner);
        let owner = Arc::downgrade(owner);
        let mut errors = self.errors.lock().expect("光照诊断锁被毒化");
        errors.retain(|(owner, _)| owner.strong_count() != 0);
        let index = errors.iter().position(|(old, _)| old.ptr_eq(&owner));
        let changed = match (index, result) {
            (Some(index), Err(error)) if errors[index].1 == error => false,
            (Some(index), Err(error)) => {
                log::error!("[lighting] {error}");
                errors[index].1 = error;
                true
            }
            (None, Err(error)) => {
                log::error!("[lighting] {error}");
                errors.push((owner, error));
                true
            }
            (Some(index), Ok(())) => {
                errors.remove(index);
                true
            }
            (None, Ok(())) => false,
        };
        drop(errors);
        if changed {
            crate::render_wake::notify();
        }
    }
    pub(crate) fn rgb(&self, code: u32) -> Result<[f32; 3], String> {
        if !self.settings.enabled || is_unlit(code) {
            return Ok([1.; 3]);
        }
        self.validate(code)?;
        let block = code & 15;
        let sky = (code >> 4) & 15;
        let fraction = ((code >> 12) & 15) as f32 / 16.;
        let a = self.uniform.colors[(block * 16 + sky) as usize];
        let b = self.uniform.colors[((block + 1).min(15) * 16 + sky) as usize];
        Ok(std::array::from_fn(|i| {
            a[i] * (1. - fraction) + b[i] * fraction
        }))
    }
    pub(crate) fn validate(&self, code: u32) -> Result<(), String> {
        let required = SAVED | BLOCK_PRESENT | if self.has_skylight { SKY_PRESENT } else { 0 };
        if !is_unlit(code) && code & required != required {
            return Err("可见源面缺少该维度需要的保存光照，不能生成光照预览".into());
        }
        Ok(())
    }
}
