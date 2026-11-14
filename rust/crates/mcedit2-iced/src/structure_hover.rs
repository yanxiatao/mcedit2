//! 独立结构各视图的悬停状态，只描述实际显示源，不改变选区或正式文档。
use crate::{
    hover_query::WorldKey, language::Language, spectator::PresentedView, textures::TextureBank,
};
use iced::{
    widget::canvas::{self, Frame},
    Color, Point, Size,
};
use mcedit2_world::{clipboard::BlockClipboard, editing::render_state};
use std::sync::Mutex;

/// 同一实际帧和光标只求交一次；缓存键使用网格弱身份，不保活旧文档。
#[derive(Default)]
pub(crate) struct Scene {
    cache: Mutex<SceneCache>,
}
#[derive(Default)]
struct SceneCache {
    key: Option<WorldKey>,
    position: Option<[i32; 3]>,
}
impl Scene {
    pub(crate) fn position(&self, shown: &PresentedView, point: Option<Point>) -> Option<[i32; 3]> {
        let key = point.map(|point| WorldKey::new(shown, point));
        let mut cache = self.cache.lock().expect("结构悬停缓存锁");
        if cache.key != key {
            cache.position = point
                .and_then(|point| shown.pick_terrain(point)?.owner.map(|owner| owner.position));
            cache.key = key;
        }
        cache.position
    }
}

fn index(size: [u16; 3], position: [i32; 3]) -> usize {
    (position[1] as usize * usize::from(size[2]) + position[2] as usize) * usize::from(size[0])
        + position[0] as usize
}
fn lines(
    source: &BlockClipboard,
    position: [i32; 3],
    bank: Option<&TextureBank>,
    language: Language,
) -> [String; 3] {
    let absolute: [i64; 3] =
        std::array::from_fn(|axis| i64::from(source.source_min[axis]) + i64::from(position[axis]));
    let coordinate = format!(
        "{} {},{},{} · {} {},{},{}",
        language.pick("局部", "Local"),
        position[0],
        position[1],
        position[2],
        language.pick("来源", "Source"),
        absolute[0],
        absolute[1],
        absolute[2]
    );
    let at = index(source.size, position);
    let block = if source.mask.as_ref().is_some_and(|mask| !mask[at]) {
        language
            .pick("未声明格（不视为空气）", "Undeclared cell (not air)")
            .to_owned()
    } else {
        let state = &source.blocks[at];
        let state_text = render_state(state);
        match bank.and_then(|bank| bank.block_name(&state.name, language)) {
            Some(name) => format!("{name} · {state_text}"),
            None => state_text,
        }
    };
    let biome = if let Some(biomes) = &source.biomes {
        let cell = if biomes.resolution == 1 {
            position
        } else {
            std::array::from_fn(|axis| (position[axis] + source.source_min[axis].rem_euclid(4)) / 4)
        };
        biomes.cells[index(biomes.size, cell)].clone()
    } else if let Some(biomes) = &source.numeric_biomes {
        format!("{} {}", language.pick("数字", "Numeric"), biomes.cells[at])
    } else {
        language.pick("未保存", "Not stored").to_owned()
    };
    [
        coordinate,
        block,
        format!("{}: {biome}", language.pick("群系", "Biome")),
    ]
}

/// 按画布宽度换行，完整状态不因 UTF-8 字节切割而损坏。
fn wrap(lines: [String; 3], width: f32) -> Vec<String> {
    let columns = (width / 12.).floor() as usize;
    let mut result = Vec::new();
    for line in lines {
        let mut current = String::new();
        let mut used = 0;
        for character in line.chars() {
            let units = if character.is_ascii() { 1 } else { 2 };
            if used + units > columns * 2 && !current.is_empty() {
                result.push(std::mem::take(&mut current));
                used = 0;
            }
            current.push(character);
            used += units;
        }
        result.push(current);
    }
    result
}

pub(crate) fn draw(
    source: &BlockClipboard,
    position: Option<[i32; 3]>,
    bank: Option<&TextureBank>,
    language: Language,
    frame: &mut Frame,
) {
    let Some(position) = position else { return };
    let width = (frame.width() - 24.).min(720.);
    // 窄窗格留给实际内容；布局恢复后重新显示，数据和工具输入保持。
    if width < 72. || frame.height() < 70. {
        return;
    }
    let mut lines = wrap(lines(source, position, bank, language), width);
    let visible = ((frame.height() - 28.) / 16.).floor() as usize;
    if lines.len() > visible {
        lines.truncate(visible);
        lines.last_mut().expect("布局至少容纳两行").push_str(" …");
    }
    let height = lines.len() as f32 * 16. + 12.;
    let top = (frame.height() - height - 8.).max(8.);
    frame.fill_rectangle(
        Point::new(8., top),
        Size::new(width + 8., height),
        Color::from_rgba(0.03, 0.04, 0.06, 0.88),
    );
    for (line, content) in lines.into_iter().enumerate() {
        frame.fill_text(canvas::Text {
            content,
            position: Point::new(12., top + 6. + line as f32 * 16.),
            size: 12.0.into(),
            color: Color::WHITE,
            ..Default::default()
        });
    }
}
