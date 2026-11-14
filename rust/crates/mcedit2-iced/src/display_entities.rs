//! 展示实体的静态内容：真实模型 / 字体、保存变换和亮度共用物品准备与世界后端。
use super::*;
use crate::{font_glyphs::Fonts, language::Language, root_transform::Affine};
use mcedit2_world::{
    entity_display::Display,
    inventory,
    sign::{Side, Style},
};

pub(crate) fn supported(id: &str) -> bool {
    matches!(Display::read(id), Some(Display::Item | Display::Text))
}
fn number(value: &Value) -> Result<f32, String> {
    let value = match value {
        Value::Byte(v) => f32::from(*v),
        Value::Short(v) => f32::from(*v),
        Value::Int(v) => *v as f32,
        Value::Long(v) => *v as f32,
        Value::Float(v) => *v,
        Value::Double(v) => *v as f32,
        _ => return Err("展示实体变换必须使用数字".into()),
    };
    if !value.is_finite() {
        return Err("展示实体变换包含非有限数字".into());
    }
    Ok(value)
}
fn vector<const N: usize>(value: &Value) -> Result<[f32; N], String> {
    let Value::List(list) = value else {
        return Err("展示实体向量必须为数字列表".into());
    };
    if list.items.len() != N {
        return Err(format!("展示实体向量必须有 {N} 个分量"));
    }
    let mut out = [0.; N];
    for (out, value) in out.iter_mut().zip(&list.items) {
        *out = number(value)?;
    }
    Ok(out)
}
fn required<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    field(Some(value), key).ok_or_else(|| format!("展示实体变换缺少 {key}"))
}
fn rotation(value: &Value) -> Result<[f32; 4], String> {
    if matches!(value, Value::List(_)) {
        return vector(value);
    }
    let axis: [f32; 3] = vector(required(value, "axis")?)?;
    let angle = number(required(value, "angle")?)?;
    let length = axis.iter().map(|v| v * v).sum::<f32>().sqrt();
    if length == 0. {
        return Err("展示实体旋转轴不能为零".into());
    }
    let (s, c) = (angle * 0.5).sin_cos();
    Ok([
        axis[0] / length * s,
        axis[1] / length * s,
        axis[2] / length * s,
        c,
    ])
}
fn text_value(visual: &EntityVisual, key: &str, default: &str) -> Result<String, String> {
    match visual.source_field_optional(key)? {
        None => Ok(default.into()),
        Some(Value::String(value)) => Ok(value),
        _ => Err(format!("展示实体 {key} 必须为 String")),
    }
}
fn integer(visual: &EntityVisual, key: &str, default: i32) -> Result<i32, String> {
    match visual.source_field_optional(key)? {
        None => Ok(default),
        Some(Value::Int(value)) => Ok(value),
        _ => Err(format!("展示实体 {key} 必须为 Int")),
    }
}
fn boolean(visual: &EntityVisual, key: &str) -> Result<bool, String> {
    match visual.source_field_optional(key)? {
        None => Ok(false),
        Some(Value::Byte(value)) => Ok(value != 0),
        _ => Err(format!("展示实体 {key} 必须为 Byte")),
    }
}
fn transform(visual: &EntityVisual) -> Result<Affine, String> {
    Display::version(visual)?;
    let billboard = text_value(visual, "billboard", "fixed")?;
    if !matches!(
        billboard.as_str(),
        "fixed" | "vertical" | "horizontal" | "center"
    ) {
        return Err(format!("未知展示实体 billboard={billboard}"));
    }
    let saved = match visual.source_field_optional("transformation")? {
        None => Affine::IDENTITY,
        Some(value @ Value::Compound(_)) => {
            let translation: [f32; 3] = vector(required(&value, "translation")?)?;
            let scale: [f32; 3] = vector(required(&value, "scale")?)?;
            let left = rotation(required(&value, "left_rotation")?)?;
            let right = rotation(required(&value, "right_rotation")?)?;
            Affine::parse(
                &serde_json::json!({"origin":"corner", "translation":translation, "scale":scale, "left_rotation":left, "right_rotation":right}),
            )?
        }
        Some(value @ Value::List(_)) => {
            let values: [f32; 16] = vector(&value)?;
            if [values[3], values[7], values[11], values[15]] != [0., 0., 0., 1.] {
                return Err("展示实体透视矩阵无法表示为当前场景的仿射变换".into());
            }
            Affine(std::array::from_fn(|row| {
                std::array::from_fn(|column| values[column * 4 + row])
            }))
        }
        _ => return Err("展示实体 transformation 必须为分解变换或 16 分量矩阵".into()),
    };
    let [yaw, pitch] = visual.rotation.ok_or("展示实体缺少实际 Rotation")?;
    // 展示内容固定在保存朝向；相机追随与插值不是静态编辑展示的完成门槛。
    Affine::parse(&serde_json::json!({"origin":"corner", "rotation":{"y":-yaw}}))?
        .compose(Affine::parse(
            &serde_json::json!({"origin":"corner", "rotation":{"x":pitch}}),
        )?)?
        .compose(saved)
}
pub(crate) fn block_quads(
    visual: &EntityVisual,
    bank: &TextureBank,
) -> Result<Vec<WorldQuad>, String> {
    let version = Display::version(visual)?;
    if bank.item_data_version() != Some(version) {
        return Err("方块展示实体资源与源版本不同".into());
    }
    let transform = transform(visual)?;
    let Some(value) = visual.source_field_optional("block_state")? else {
        return Ok(Vec::new());
    };
    let mut bytes = vec![10, 0, 0];
    bytes.extend(value.payload().map_err(|error| error.to_string())?);
    let state =
        mcedit2_world::sections::decode_block_state(&bytes).map_err(|error| error.to_string())?;
    if matches!(
        state.name.as_str(),
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
    ) {
        return Ok(Vec::new());
    }
    let brightness = Display::brightness(visual)?
        .map(|levels| crate::lighting::fixed(crate::lighting::encode(levels)));
    let mut out = crate::spectator::single_block_quads(bank, &state, &|| false)?;
    for quad in &mut out {
        quad.xyz = quad
            .xyz
            .map(|point| transform.point(point.map(|v| v + 0.5)));
        if transform.determinant() < 0. {
            quad.reverse_winding();
        }
        quad.classify_axis_facing();
        if let Some(code) = brightness {
            quad.light = code;
        }
    }
    Ok(out)
}
fn item(
    visual: &EntityVisual,
    version: i32,
    bank: &TextureBank,
    online_skins: bool,
    stop: &dyn Fn() -> bool,
    context: Option<ItemContext<'_>>,
    position: [f64; 3],
    transform: Affine,
    prepared: &mut Prepared,
) -> Result<(), String> {
    let Some(value) = visual.source_field_optional("item")? else {
        return Ok(());
    };
    let Some(entry) =
        inventory::read_stack(&value, Some(version)).map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    let id = if entry.id.contains(':') {
        entry.id
    } else {
        format!("minecraft:{}", entry.id)
    };
    let display = text_value(visual, "item_display", "none")?;
    if !matches!(
        display.as_str(),
        "none"
            | "thirdperson_lefthand"
            | "thirdperson_righthand"
            | "firstperson_lefthand"
            | "firstperson_righthand"
            | "head"
            | "gui"
            | "ground"
            | "fixed"
    ) {
        return Err(format!("未知展示实体 item_display={display}"));
    }
    let key = Appearance::new(&id, Some(version), Some(&value)).with_online_skins(online_skins);
    let mut properties = key.properties()?;
    crate::item_world::apply_dropped(
        &id,
        &value,
        visual.rotation.ok_or("展示实体缺少 Rotation")?[0],
        version,
        context.map(|c| c.environment),
        std::array::from_fn(|i| position[i] + context.map_or(0., |c| c.offset[i])),
        &mut properties,
    )?;
    let mut scene = bank.item_scene_context(&key.model_id()?, &properties, &display)?;
    if scene.builtin {
        for texture in crate::item_specials::prepare_materials(bank, &key, &mut scene, stop)? {
            prepared
                .textures
                .insert(texture.id.clone(), Arc::new(texture));
        }
    }
    crate::item_icons::generated_context(bank, &mut scene, true, stop)?;
    let transform = transform.compose(scene.transform)?;
    for mut face in scene.quads {
        if stop() {
            return Err("物品展示实体准备已取消".into());
        }
        face.xyz = face.xyz.map(|point| transform.point(point));
        let tint = key.tint(face.tint)?;
        for color in &mut face.colors {
            for i in 0..3 {
                color[i] *= tint[i];
            }
        }
        if face.winding * transform.determinant() < 0. {
            face.xyz.swap(1, 3);
            face.uv.swap(1, 3);
            face.colors.swap(1, 3);
        }
        prepared.faces.push(face);
    }
    Ok(())
}
fn fragment(ch: char, style: &Style) -> serde_json::Value {
    let mut value = serde_json::json!({"text":ch.to_string(), "font":style.font, "bold":style.bold, "italic":style.italic,
        "underlined":style.underlined, "strikethrough":style.strikethrough, "obfuscated":false});
    if let Some(color) = style.color {
        value["color"] = format!("#{color:06x}").into();
    }
    value
}
fn face(
    texture: String,
    xyz: [[f32; 3]; 4],
    uv: [[f32; 2]; 4],
    alpha: f32,
    transform: Affine,
) -> Quad {
    let mut face = Quad {
        texture,
        xyz: xyz.map(|p| transform.point(p)),
        uv,
        tint: None,
        shade: false,
        colors: [[1., 1., 1., alpha]; 4],
        winding: 1.,
        cull: false,
        mode: Some(2 | crate::material_mode::BLEND_DEPTH_WRITE | crate::material_mode::NO_MIPMAP),
    };
    if transform.determinant() < 0. {
        face.xyz.swap(1, 3);
        face.uv.swap(1, 3);
        face.colors.swap(1, 3);
    }
    face
}
fn text(
    visual: &EntityVisual,
    bank: &TextureBank,
    language: Language,
    stop: &dyn Fn() -> bool,
    transform: Affine,
    prepared: &mut Prepared,
) -> Result<(), String> {
    let raw = text_value(visual, "text", "{\"text\":\"\"}")?;
    let component: serde_json::Value =
        serde_json::from_str(&raw).map_err(|error| format!("展示实体 text 无效：{error}"))?;
    let spans = mcedit2_world::sign::spans(&component, &|key| {
        bank.translation(key, language).map(str::to_owned)
    })?;
    let limit = integer(visual, "line_width", 200)? as f32;
    let align = text_value(visual, "alignment", "center")?;
    if !matches!(align.as_str(), "left" | "center" | "right") {
        return Err(format!("未知展示实体 alignment={align}"));
    }
    let opacity = match visual.source_field_optional("text_opacity")? {
        None => 255,
        Some(Value::Byte(value)) => value as u8,
        _ => return Err("展示实体 text_opacity 必须为 Byte".into()),
    };
    let shadow = boolean(visual, "shadow")?;
    let background = integer(visual, "background", 0x40000000)? as u32;
    let background = if boolean(visual, "default_background")? {
        0x40000000
    } else {
        background
    };
    let mut fonts = Fonts::new(bank, language);
    let mut lines = vec![Vec::new()];
    let mut width = 0.;
    for span in spans {
        for ch in span.text.chars() {
            if stop() {
                return Err("文字展示实体排版已取消".into());
            }
            if ch == '\n' {
                lines.push(Vec::new());
                width = 0.;
                continue;
            }
            let glyph = fonts.glyph_or_missing(&span.style.font, ch, stop)?;
            let advance = glyph.advance
                + if span.style.bold {
                    glyph.bold_offset
                } else {
                    0.
                };
            if width + advance > limit && !lines[lines.len() - 1].is_empty() {
                lines.push(Vec::new());
                width = 0.;
            }
            let current = lines.len() - 1;
            lines[current].push(fragment(ch, &span.style));
            width += advance;
        }
    }
    let count = lines.len();
    let mut layouts = Vec::new();
    let mut max_width = 0f32;
    for line in lines {
        let mut side = Side::default();
        side.messages[0] = serde_json::Value::Array(line);
        let layout = crate::sign_text::prepare(
            &side,
            bank,
            &mut fonts,
            language,
            u32::MAX,
            9,
            0xffffff,
            None,
            stop,
        )?;
        if let Some(layout) = &layout {
            max_width = max_width.max(layout.widths[0]);
        }
        layouts.push(layout);
    }
    let height = count as f32 * 9.;
    if background >> 24 != 0 {
        let id = format!("mcedit2:runtime/display/background/{background:08x}");
        let [a, r, g, b] = background.to_be_bytes();
        prepared.textures.insert(
            id.clone(),
            Arc::new(RuntimeTexture {
                id: id.clone(),
                width: 1,
                height: 1,
                rgba: vec![r, g, b, a],
                sampler: None,
            }),
        );
        let (left, right, top) = (
            (-max_width * 0.5 - 1.) * 0.025,
            (max_width * 0.5 + 1.) * 0.025,
            (height + 1.) * 0.025,
        );
        prepared.faces.push(face(
            id,
            [
                [left, -0.025, 0.],
                [left, top, 0.],
                [right, top, 0.],
                [right, -0.025, 0.],
            ],
            [[0., 1.], [0., 0.], [1., 0.], [1., 1.]],
            1.,
            transform,
        ));
    }
    if opacity == 0 {
        return Ok(());
    }
    let alpha = if opacity == 255 {
        1.
    } else {
        -f32::from(opacity) / 255.
    };
    for (row, layout) in layouts.into_iter().enumerate() {
        let Some(layout) = layout else {
            continue;
        };
        let shift = match align.as_str() {
            "left" => (layout.widths[0] - max_width) * 0.5,
            "right" => (max_width - layout.widths[0]) * 0.5,
            _ => 0.,
        };
        for part in layout.parts {
            prepared
                .textures
                .entry(part.texture.id.clone())
                .or_insert_with(|| Arc::clone(&part.texture));
            let xyz = part.corners.map(|p| {
                [
                    (p[0] + shift) * 0.025,
                    (height - p[1] - 18. - row as f32 * 9.) * 0.025,
                    -0.001 - part.depth * 0.025,
                ]
            });
            let [u0, v0, u1, v1] = part.uv;
            let uv = [[u0, v0], [u0, v1], [u1, v1], [u1, v0]];
            if shadow {
                let mut shadow = face(
                    part.texture.id.clone(),
                    xyz.map(|p| [p[0] + 0.025, p[1] - 0.025, p[2] + 0.0005]),
                    uv,
                    alpha,
                    transform,
                );
                for color in &mut shadow.colors {
                    color[..3].fill(0.25);
                }
                prepared.faces.push(shadow);
            }
            prepared
                .faces
                .push(face(part.texture.id.clone(), xyz, uv, alpha, transform));
        }
    }
    Ok(())
}
pub(super) fn prepare(
    visual: &EntityVisual,
    id: &str,
    position: [f64; 3],
    bank: &TextureBank,
    online_skins: bool,
    stop: &dyn Fn() -> bool,
    context: Option<ItemContext<'_>>,
    language: Language,
) -> Result<Prepared, String> {
    let version = Display::version(visual)?;
    if bank.item_data_version() != Some(version) {
        return Err("展示实体资源与源版本不同".into());
    }
    let transform = transform(visual)?;
    let mut prepared = Prepared {
        center: [position[0] - 0.5, position[1], position[2] - 0.5],
        faces: Vec::new(),
        textures: BTreeMap::new(),
        diagnostics: Vec::new(),
    };
    match Display::read(id) {
        Some(Display::Item) => item(
            visual,
            version,
            bank,
            online_skins,
            stop,
            context,
            position,
            transform,
            &mut prepared,
        )?,
        Some(Display::Text) => text(visual, bank, language, stop, transform, &mut prepared)?,
        _ => return Err(format!("没有物品 / 文本展示实体准备入口 {id}")),
    }
    Ok(prepared)
}
