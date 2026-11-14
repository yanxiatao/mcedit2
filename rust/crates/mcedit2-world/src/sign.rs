//! 告示牌的原始双面文字与方块状态；文字保留组件结构，交互事件不会被执行。
use crate::sections::{parse_root, State, Tag};
use serde_json::Value;

pub const WOODS: [&str; 11] = [
    "oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "cherry", "bamboo",
    "crimson", "warped",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mount {
    Standing,
    Wall,
    Hanging,
    WallHanging,
    Legacy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Side {
    pub messages: [Value; 4],
    pub filtered_messages: Option<[Value; 4]>,
    pub color: u8,
    pub glowing: bool,
}
impl Default for Side {
    fn default() -> Self {
        Self {
            messages: std::array::from_fn(|_| Value::String(String::new())),
            filtered_messages: None,
            color: 15,
            glowing: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub state: State,
    pub wood: String,
    pub mount: Mount,
    pub front: Side,
    pub back: Side,
    pub waxed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Style {
    pub font: String,
    pub color: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underlined: bool,
    pub strikethrough: bool,
    pub obfuscated: bool,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            font: "minecraft:default".into(),
            color: None,
            bold: false,
            italic: false,
            underlined: false,
            strikethrough: false,
            obfuscated: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}
const CHAT_COLORS: [u32; 16] = [
    0x000000, 0x0000aa, 0x00aa00, 0x00aaaa, 0xaa0000, 0xaa00aa, 0xffaa00, 0xaaaaaa, 0x555555,
    0x5555ff, 0x55ff55, 0x55ffff, 0xff5555, 0xff55ff, 0xffff55, 0xffffff,
];

/// 将静态组件展开成保留样式的连续文本；翻译来自当前资源语言表。
/// selector / score / nbt 等需要游戏执行上下文的组件明确报错，绝不执行点击事件。
pub fn spans(
    component: &Value,
    translate: &impl Fn(&str) -> Option<String>,
) -> Result<Vec<Span>, String> {
    collect_spans(component, translate, true)
}
/// Component.getString 语义：展开实际内容，保留字符串中的原始格式码，不进行字形排版。
pub fn plain_text(
    component: &Value,
    translate: &impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    Ok(collect_spans(component, translate, false)?
        .into_iter()
        .map(|span| span.text)
        .collect())
}
struct Collected {
    spans: Vec<Span>,
    formatted: bool,
}
fn collect_spans(
    component: &Value,
    translate: &impl Fn(&str) -> Option<String>,
    formatted: bool,
) -> Result<Vec<Span>, String> {
    fn append(out: &mut Collected, text: &str, style: &Style) {
        if !out.formatted {
            out.spans.push(Span {
                text: text.to_owned(),
                style: style.clone(),
            });
            return;
        }
        fn flush(out: &mut Vec<Span>, text: &mut String, style: &Style) {
            if text.is_empty() {
                return;
            }
            if let Some(previous) = out.last_mut().filter(|span| span.style == *style) {
                previous.text.push_str(text);
                text.clear();
            } else {
                out.push(Span {
                    text: std::mem::take(text),
                    style: style.clone(),
                });
            }
        }
        let mut current = style.clone();
        let mut buffer = String::new();
        let mut chars = text.chars();
        while let Some(ch) = chars.next() {
            if ch != '\u{a7}' {
                buffer.push(ch);
                continue;
            }
            flush(&mut out.spans, &mut buffer, &current);
            let Some(code) = chars.next().map(|ch| ch.to_ascii_lowercase()) else {
                break;
            };
            if let Some(color) = code.to_digit(16) {
                current.color = Some(CHAT_COLORS[color as usize]);
                current.bold = false;
                current.italic = false;
                current.underlined = false;
                current.strikethrough = false;
                current.obfuscated = false;
            } else {
                match code {
                    'k' => current.obfuscated = true,
                    'l' => current.bold = true,
                    'm' => current.strikethrough = true,
                    'n' => current.underlined = true,
                    'o' => current.italic = true,
                    'r' => current = style.clone(),
                    _ => {}
                }
            }
        }
        flush(&mut out.spans, &mut buffer, &current);
    }
    fn visit(
        value: &Value,
        inherited: &Style,
        translate: &impl Fn(&str) -> Option<String>,
        out: &mut Collected,
        depth: usize,
    ) -> Result<Style, String> {
        if depth > 64 {
            return Err("文字组件展开超过 64 层".into());
        }
        match value {
            Value::String(text) => {
                append(out, text, inherited);
                Ok(inherited.clone())
            }
            Value::Array(items) => {
                let (first, rest) = items.split_first().ok_or("组件数组不能为空")?;
                let parent = visit(first, inherited, translate, out, depth + 1)?;
                for item in rest {
                    visit(item, &parent, translate, out, depth + 1)?;
                }
                Ok(parent)
            }
            Value::Object(fields) => {
                let mut style = inherited.clone();
                for (key, value) in [
                    ("bold", &mut style.bold),
                    ("italic", &mut style.italic),
                    ("underlined", &mut style.underlined),
                    ("strikethrough", &mut style.strikethrough),
                    ("obfuscated", &mut style.obfuscated),
                ] {
                    if let Some(field) = fields.get(key) {
                        *value = field
                            .as_bool()
                            .ok_or_else(|| format!("文字样式 {key} 必须为布尔值"))?;
                    }
                }
                if let Some(font) = fields.get("font") {
                    let font = font.as_str().ok_or("文字 font 必须为资源 ID")?;
                    let (namespace, path) = font.split_once(':').unwrap_or(("minecraft", font));
                    if namespace.is_empty()
                        || path.is_empty()
                        || !namespace.bytes().all(|b| {
                            b.is_ascii_lowercase()
                                || b.is_ascii_digit()
                                || matches!(b, b'_' | b'-' | b'.')
                        })
                        || !path.bytes().all(|b| {
                            b.is_ascii_lowercase()
                                || b.is_ascii_digit()
                                || matches!(b, b'_' | b'-' | b'.' | b'/')
                        })
                    {
                        return Err("文字 font 资源 ID 无效".into());
                    }
                    style.font = format!("{namespace}:{path}");
                }
                if let Some(color) = fields.get("color") {
                    let color = color.as_str().ok_or("文字 color 必须为名称或 RGB 字符串")?;
                    const NAMES: [&str; 16] = [
                        "black",
                        "dark_blue",
                        "dark_green",
                        "dark_aqua",
                        "dark_red",
                        "dark_purple",
                        "gold",
                        "gray",
                        "dark_gray",
                        "blue",
                        "green",
                        "aqua",
                        "red",
                        "light_purple",
                        "yellow",
                        "white",
                    ];
                    const COLORS: [u32; 16] = CHAT_COLORS;
                    style.color = Some(
                        if let Some(hex) = color.strip_prefix('#').filter(|hex| {
                            hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit())
                        }) {
                            u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?
                        } else {
                            COLORS[NAMES
                                .iter()
                                .position(|name| *name == color)
                                .ok_or_else(|| format!("未知文字颜色 {color}"))?]
                        },
                    );
                }
                let kinds = ["text", "translate", "selector", "score", "keybind", "nbt"]
                    .into_iter()
                    .filter(|key| fields.contains_key(*key))
                    .collect::<Vec<_>>();
                if kinds.len() != 1 {
                    return Err("文字组件必须有且仅有一个内容类型".into());
                }
                match kinds[0] {
                    "text" => append(
                        out,
                        fields["text"].as_str().ok_or("文字 text 必须为字符串")?,
                        &style,
                    ),
                    "translate" => {
                        let key = fields["translate"]
                            .as_str()
                            .ok_or("translate 必须为字符串")?;
                        let format = translate(key)
                            .or_else(|| {
                                fields
                                    .get("fallback")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned)
                            })
                            .unwrap_or_else(|| key.to_owned());
                        let args = fields
                            .get("with")
                            .map(|v| v.as_array().ok_or("翻译 with 必须为数组"))
                            .transpose()?;
                        let mut chars = format.chars().peekable();
                        let mut sequential = 0usize;
                        let mut literal = String::new();
                        while let Some(character) = chars.next() {
                            if character != '%' {
                                literal.push(character);
                                continue;
                            }
                            if chars.peek() == Some(&'%') {
                                chars.next();
                                literal.push('%');
                                continue;
                            }
                            append(out, &literal, &style);
                            literal.clear();
                            let mut number = String::new();
                            while chars.peek().is_some_and(char::is_ascii_digit) {
                                number.push(chars.next().ok_or("翻译参数索引被截断")?);
                            }
                            let index = if number.is_empty() {
                                let index = sequential;
                                sequential += 1;
                                index
                            } else {
                                if chars.next() != Some('$') {
                                    return Err("翻译格式参数索引缺少 $".into());
                                }
                                number
                                    .parse::<usize>()
                                    .ok()
                                    .and_then(|n| n.checked_sub(1))
                                    .ok_or("翻译参数索引无效")?
                            };
                            if chars.next() != Some('s') {
                                return Err("翻译仅支持原版 %s / %n$s / %% 格式".into());
                            }
                            let arg = args
                                .and_then(|args| args.get(index))
                                .ok_or("翻译缺少对应参数")?;
                            match arg {
                                Value::Bool(_) | Value::Number(_) => {
                                    append(out, &arg.to_string(), &style)
                                }
                                _ => {
                                    visit(arg, &style, translate, out, depth + 1)?;
                                }
                            }
                        }
                        append(out, &literal, &style);
                    }
                    kind => return Err(format!("文字组件 {kind} 需要游戏执行上下文")),
                }
                if let Some(extra) = fields.get("extra") {
                    for item in extra.as_array().ok_or("组件 extra 必须为数组")? {
                        visit(item, &style, translate, out, depth + 1)?;
                    }
                }
                Ok(style)
            }
            _ => Err("文字组件类型无效".into()),
        }
    }
    let mut out = Collected {
        spans: Vec::new(),
        formatted,
    };
    visit(component, &Style::default(), translate, &mut out, 0)?;
    Ok(out.spans)
}

pub fn is_sign_id(id: &str) -> bool {
    matches!(id, "Sign" | "minecraft:sign" | "minecraft:hanging_sign")
}

fn named(name: &str) -> Option<(&'static str, Mount)> {
    let path = name.strip_prefix("minecraft:")?;
    if matches!(path, "standing_sign" | "wall_sign") {
        return Some(("oak", Mount::Legacy));
    }
    for (suffix, mount) in [
        ("_wall_hanging_sign", Mount::WallHanging),
        ("_hanging_sign", Mount::Hanging),
        ("_wall_sign", Mount::Wall),
        ("_sign", Mount::Standing),
    ] {
        if let Some(wood) = path.strip_suffix(suffix) {
            return WOODS
                .iter()
                .copied()
                .find(|candidate| *candidate == wood)
                .map(|wood| (wood, mount));
        }
    }
    None
}
pub fn is_sign_state_name(name: &str) -> bool {
    named(name).is_some()
}

fn unique<'a, 'b>(tag: &'a Tag<'b>, name: &str) -> Result<Option<&'a Tag<'b>>, String> {
    let mut fields = tag.fields.iter().filter(|(key, _)| key == name);
    let field = fields.next().map(|(_, value)| value);
    if fields.next().is_some() {
        return Err(format!("告示牌 NBT 重复字段 {name}"));
    }
    Ok(field)
}
fn string(tag: &Tag<'_>) -> Result<String, String> {
    tag.string().map_err(|e| e.to_string())
}
fn property<'a>(state: &'a State, name: &str) -> Result<&'a str, String> {
    let mut entries = state.properties.iter().filter(|(key, _)| key == name);
    let value = entries
        .next()
        .ok_or_else(|| format!("告示牌状态缺少 {name}"))?;
    if entries.next().is_some() {
        return Err(format!("告示牌状态重复 {name}"));
    }
    Ok(&value.1)
}
fn boolean(tag: &Tag<'_>) -> Result<bool, String> {
    if tag.kind != 1 || tag.raw.len() != 1 || tag.raw[0] > 1 {
        return Err("告示牌布尔字段必须为 Byte 0/1".into());
    }
    Ok(tag.raw[0] != 0)
}
fn color(tag: &Tag<'_>) -> Result<u8, String> {
    let color = string(tag)?;
    crate::banner::COLORS
        .iter()
        .position(|name| *name == color)
        .map(|index| index as u8)
        .ok_or_else(|| format!("告示牌染料颜色无效：{color}"))
}
fn component(tag: &Tag<'_>) -> Result<Value, String> {
    let encoded = string(tag)?;
    parse_component(&encoded)
}
/// 编辑与显示共用客户端组件语法；保留事件与动态组件，不执行它们。
pub fn parse_component(encoded: &str) -> Result<Value, String> {
    let value: Value = crate::client_json::from_str(encoded)
        .map_err(|e| format!("告示牌文字不是合法组件 JSON：{e}"))?;
    fn shape(value: &Value, depth: usize) -> Result<(), String> {
        if depth > 64 {
            return Err("告示牌文字组件嵌套超过 64 层".into());
        }
        match value {
            Value::String(_) | Value::Object(_) => Ok(()),
            Value::Array(items) if !items.is_empty() => {
                for item in items {
                    shape(item, depth + 1)?;
                }
                Ok(())
            }
            _ => Err("告示牌文字组件必须为 String、Compound 对应对象或非空组件数组".into()),
        }
    }
    shape(&value, 0)?;
    Ok(value)
}
fn lines(tag: &Tag<'_>) -> Result<[Value; 4], String> {
    if tag.kind != 9 || tag.items.len() != 4 {
        return Err("告示牌 messages 必须恰好包含四行".into());
    }
    tag.items
        .iter()
        .map(component)
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| "告示牌行数不匹配".into())
}
fn side(tag: &Tag<'_>) -> Result<Side, String> {
    if tag.kind != 10 {
        return Err("告示牌文字面必须为 Compound".into());
    }
    Ok(Side {
        messages: lines(unique(tag, "messages")?.ok_or("告示牌文字面缺少 messages")?)?,
        filtered_messages: unique(tag, "filtered_messages")?.map(lines).transpose()?,
        color: unique(tag, "color")?.map(color).transpose()?.unwrap_or(15),
        glowing: unique(tag, "has_glowing_text")?
            .map(boolean)
            .transpose()?
            .unwrap_or(false),
    })
}

/// 真实原始 BE 载荷，不含 Compound 根头；旧文字不经过数据修复，也不丢弃组件样式。
pub fn parse(compound: &[u8], state: &State, version: Option<i32>) -> Result<Source, String> {
    let mut raw = Vec::with_capacity(compound.len() + 3);
    raw.extend_from_slice(&[10, 0, 0]);
    raw.extend_from_slice(compound);
    read(
        &parse_root(&raw).map_err(|e| e.to_string())?,
        state,
        version,
    )
}
pub(crate) fn read(tag: &Tag<'_>, state: &State, _version: Option<i32>) -> Result<Source, String> {
    if tag.kind != 10 {
        return Err("告示牌载荷必须为Compound".into());
    }
    let id = string(unique(tag, "id")?.ok_or("告示牌缺少 id")?)?;
    if !is_sign_id(&id) {
        return Err(format!("方块实体 {id} 不是告示牌"));
    }
    let (wood, mount) = if let Some(numeric) = state.name.strip_prefix("numeric:") {
        if numeric.parse::<u16>().ok().filter(|id| *id > 0).is_none()
            || property(state, "data")?
                .parse::<u8>()
                .ok()
                .filter(|data| *data < 16)
                .is_none()
        {
            return Err("告示牌数字状态 ID / data 无效".into());
        }
        ("oak", Mount::Legacy)
    } else {
        named(&state.name).ok_or_else(|| format!("方块 {} 不是已核实的原版告示牌", state.name))?
    };
    let hanging = matches!(mount, Mount::Hanging | Mount::WallHanging);
    if hanging != (id == "minecraft:hanging_sign") {
        return Err("告示牌方块状态与普通 / 悬挂 BE 类型不匹配".into());
    }
    if mount != Mount::Legacy {
        match mount {
            Mount::Standing | Mount::Hanging => {
                if property(state, "rotation")?
                    .parse::<u8>()
                    .ok()
                    .filter(|rotation| *rotation < 16)
                    .is_none()
                {
                    return Err("告示牌 rotation 必须为 0..15".into());
                }
                if mount == Mount::Hanging
                    && !matches!(property(state, "attached")?, "true" | "false")
                {
                    return Err("悬挂告示牌 attached 无效".into());
                }
            }
            Mount::Wall | Mount::WallHanging => {
                if !matches!(
                    property(state, "facing")?,
                    "north" | "south" | "west" | "east"
                ) {
                    return Err("墙挂告示牌 facing 必须为水平方向".into());
                }
            }
            Mount::Legacy => unreachable!(),
        }
    }
    let front_tag = unique(tag, "front_text")?;
    let back_tag = unique(tag, "back_text")?;
    let old = (1..=4)
        .map(|i| unique(tag, &format!("Text{i}")))
        .collect::<Result<Vec<_>, _>>()?;
    let (front, back) = if front_tag.is_some() || back_tag.is_some() {
        if old.iter().any(Option::is_some) {
            return Err("告示牌同时包含旧 Text 字段和现代双面文字".into());
        }
        (
            front_tag.map(side).transpose()?.unwrap_or_default(),
            back_tag.map(side).transpose()?.unwrap_or_default(),
        )
    } else {
        let mut front = Side::default();
        for (line, tag) in front.messages.iter_mut().zip(old) {
            if let Some(tag) = tag {
                // 数字布局不能证明使用早期纯文本编码；未核实该旧格式时明确诊断。
                *line = component(tag)?;
            }
        }
        let filtered = (1..=4)
            .map(|i| unique(tag, &format!("FilteredText{i}")))
            .collect::<Result<Vec<_>, _>>()?;
        if filtered.iter().any(Option::is_some) {
            let mut messages = front.messages.clone();
            for (line, tag) in messages.iter_mut().zip(filtered) {
                if let Some(tag) = tag {
                    *line = component(tag)?;
                }
            }
            front.filtered_messages = Some(messages);
        }
        front.color = unique(tag, "Color")?.map(color).transpose()?.unwrap_or(15);
        front.glowing = unique(tag, "GlowingText")?
            .map(boolean)
            .transpose()?
            .unwrap_or(false);
        (front, Side::default())
    };
    Ok(Source {
        state: state.clone(),
        wood: wood.into(),
        mount,
        front,
        back,
        waxed: unique(tag, "is_waxed")?
            .map(boolean)
            .transpose()?
            .unwrap_or(false),
    })
}
