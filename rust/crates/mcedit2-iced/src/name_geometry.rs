//! 常显 / 选中实体的源名称；复用资源字体、文本组件、运行时图集和相机面片。
use crate::{
    billboard,
    font_glyphs::Fonts,
    language::Language,
    sign_text,
    textures::{RuntimeTexture, TextureAtlas, TextureBank},
    world_mesh::WorldQuad,
};
use mcedit2_world::{
    entity_catalog::EntityVisual,
    entity_name,
    sign::{Side, Style},
};
use std::{collections::BTreeMap, sync::Arc};
pub(crate) struct Prepared {
    parts: Vec<sign_text::Part>,
    width: f32,
    lines: usize,
    pub always_visible: bool,
}
fn fragment(text: &str, style: &Style) -> serde_json::Value {
    let mut value = serde_json::json!({"text":text,"font":style.font,"bold":style.bold,"italic":style.italic,
        "underlined":style.underlined,"strikethrough":style.strikethrough,"obfuscated":style.obfuscated});
    if let Some(color) = style.color {
        value["color"] = format!("#{color:06x}").into();
    }
    value
}
pub(crate) fn prepare(
    visual: &EntityVisual,
    profile_name: Option<&str>,
    bank: &TextureBank,
    fonts: &mut Fonts<'_>,
    language: Language,
    stop: &dyn Fn() -> bool,
) -> Result<Option<Prepared>, String> {
    let Some(source) = entity_name::Source::read(visual, profile_name)? else {
        return Ok(None);
    };
    let spans = mcedit2_world::sign::spans(&source.component, &|key| {
        bank.translation(key, language).map(str::to_owned)
    })?;
    let mut lines = vec![Vec::new()];
    for span in spans {
        for (index, text) in span.text.split('\n').enumerate() {
            if stop() {
                return Err("名称标签排版已取消".into());
            }
            if index != 0 {
                lines.push(Vec::new());
            }
            lines
                .last_mut()
                .expect("名称至少有一行")
                .push(fragment(text, &span.style));
        }
    }
    let count = lines.len();
    let mut textures = BTreeMap::<String, Arc<RuntimeTexture>>::new();
    let budget = bank.atlas().ok_or("名称准备缺少图集")?.remaining_bytes();
    let mut result = Prepared {
        parts: Vec::new(),
        width: 0.,
        lines: count,
        always_visible: source.always_visible,
    };
    for (index, line) in lines.into_iter().enumerate() {
        let mut side = Side::default();
        side.messages[0] = serde_json::Value::Array(line);
        if let Some(text) = sign_text::prepare(
            &side,
            bank,
            fonts,
            language,
            u32::MAX,
            9,
            0xffffff,
            None,
            stop,
        )? {
            result.width = result.width.max(text.widths[0]);
            for mut part in text.parts {
                let retained = result
                    .parts
                    .len()
                    .checked_add(1)
                    .and_then(|n| n.checked_mul(std::mem::size_of::<sign_text::Part>()))
                    .ok_or("名称几何工作集溢出")?;
                if retained as u64 > budget {
                    return Err("名称几何超过剩余资源预算".into());
                }
                result.parts.try_reserve(1).map_err(|e| e.to_string())?;
                part.texture = Arc::clone(
                    textures
                        .entry(part.texture.id.clone())
                        .or_insert_with(|| Arc::clone(&part.texture)),
                );
                for point in &mut part.corners {
                    point[1] += 18. + index as f32 * 9.;
                }
                result.parts.push(part);
            }
        }
    }
    if stop() {
        return Err("名称标签准备已取消".into());
    }
    Ok(Some(result))
}
impl Prepared {
    pub fn textures(&self) -> impl Iterator<Item = &Arc<RuntimeTexture>> {
        self.parts.iter().map(|part| &part.texture)
    }
    /// 位置来自真实模型范围；所有字形共用同一个源锚点，转向不改变字形间的排版。
    pub fn quads(&self, atlas: &TextureAtlas, anchor: [f32; 3]) -> Result<Vec<WorldQuad>, String> {
        use crate::material_mode as mode;
        let flags = 2
            | 32
            | mode::BLEND_DEPTH_WRITE
            | mode::SINGLE_SIDED
            | mode::NO_MIPMAP
            | mode::NAME_LABEL;
        let scale = 0.025;
        let rise = (self.lines - 1) as f32 * 9.;
        let half = (self.width * 0.5).floor();
        let uv = [atlas.white.sample(0.5, 0.5); 4];
        let mut quads = vec![billboard::sprite(
            anchor,
            [(-half - 1.) * scale, -8. * scale],
            [(self.width - half + 1.) * scale, (1. + rise) * scale],
            uv,
            [0., 0., 0., 0.25],
            flags,
            atlas.white.page,
        )];
        for part in &self.parts {
            let tile = atlas
                .entries
                .get(&part.texture.id)
                .ok_or_else(|| format!("名称图集缺少 {}", part.texture.id))?;
            let [u0, v0, u1, v1] = part.uv;
            let uv = [[u0, v0], [u0, v1], [u1, v1], [u1, v0]].map(|[u, v]| tile.sample(u, v));
            let corners = std::array::from_fn(|i| {
                [
                    part.corners[i][0] * scale,
                    (rise - part.corners[i][1]) * scale,
                    -0.001 - part.depth * scale,
                ]
            });
            quads.push(billboard::parallelogram(
                anchor, corners, uv, [1.; 4], flags, tile.page,
            ));
        }
        Ok(quads)
    }
}
