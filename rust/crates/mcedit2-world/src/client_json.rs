//! 客户端数据的单文档 JSON 语法；直接按 Gson lenient token 规则读取，不先严格解析再兜底。
//! 支持注释、无引号/单引号字符串、分号及等号分隔、数组空项；结构层级沿用资源解析的 128 层预算。
//! 字符串必须可表示为 UTF-8；设置、网络协议及编辑器自己的 JSON 不使用这个入口。
use serde_json::{Map, Number, Value};

/// Gson Number.intValue：十进制向零截断后取低 32 位，避免经 f64 丢失整数精度。
pub fn integer(value: &Value) -> Result<i32, String> {
    let Value::Number(number) = value else {
        return Err("客户端字段需要数字".into());
    };
    let text = number.to_string();
    let (mantissa, exponent) =
        text.split_once(['e', 'E'])
            .map_or(Ok((text.as_str(), 0_i64)), |(m, e)| {
                e.parse::<i64>()
                    .map(|e| (m, e))
                    .map_err(|_| "客户端数字指数超过 Java 范围".to_owned())
            })?;
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.strip_prefix('-').unwrap_or(mantissa);
    let fraction = mantissa.split_once('.').map_or(0, |(_, f)| f.len());
    let scale = i64::try_from(fraction)
        .map_err(|_| "客户端数字小数位过多")?
        .checked_sub(exponent)
        .and_then(|n| i32::try_from(n).ok())
        .ok_or("客户端数字尺度超过 Java 范围")?;
    let digits = mantissa.bytes().filter(|b| *b != b'.');
    let take = if scale > 0 {
        digits.clone().count().saturating_sub(scale as usize)
    } else {
        digits.clone().count()
    };
    let mut result = digits.take(take).fold(0_u32, |n, b| {
        n.wrapping_mul(10).wrapping_add(u32::from(b - b'0'))
    });
    if scale < 0 {
        result = result.wrapping_mul(10_u32.wrapping_pow(scale.unsigned_abs()));
    }
    Ok(if negative {
        0_u32.wrapping_sub(result)
    } else {
        result
    } as i32)
}

/// 读取由 Minecraft 客户端解释的 UTF-8 文本，拒绝未消费的尾部数据。
pub fn from_slice(bytes: &[u8]) -> Result<Value, String> {
    from_str(std::str::from_utf8(bytes).map_err(|e| format!("客户端 JSON 不是 UTF-8：{e}"))?)
}
/// 同一文档语法供嵌入在 NBT 中的文字组件和资源定义复用。
pub fn from_str(text: &str) -> Result<Value, String> {
    let mut input = Input { text, at: 0 };
    if text.starts_with('\u{feff}') {
        input.at = '\u{feff}'.len_utf8();
    }
    input.space()?;
    if input.remaining().starts_with(")]}'\n") {
        input.at += 5;
    }
    let mut frames = Vec::<Frame>::new();
    let mut completed = None;
    loop {
        if let Some(value) = completed.take() {
            match frames.last_mut() {
                Some(Frame::Array { values, state }) => {
                    values.push(value);
                    *state = ArrayState::Separator;
                }
                Some(Frame::Object {
                    values,
                    name,
                    state,
                }) => {
                    values.insert(name.take().expect("JSON 对象值必须持有字段名"), value);
                    *state = ObjectState::Separator;
                }
                None => {
                    input.space()?;
                    return if input.at == text.len() {
                        Ok(value)
                    } else {
                        Err(input.error("文档尾部存在额外数据"))
                    };
                }
            }
            continue;
        }
        let next = input.peek()?;
        match frames.last_mut() {
            Some(Frame::Array {
                state: ArrayState::Separator,
                ..
            }) => match next {
                Some(b']') => {
                    input.at += 1;
                    completed = Some(frames.pop().unwrap().finish());
                }
                Some(b',' | b';') => {
                    input.at += 1;
                    if let Some(Frame::Array { state, .. }) = frames.last_mut() {
                        *state = ArrayState::Value;
                    }
                }
                _ => return Err(input.error("数组需要分隔符或结束括号")),
            },
            Some(Frame::Object {
                state: ObjectState::Separator,
                ..
            }) => match next {
                Some(b'}') => {
                    input.at += 1;
                    completed = Some(frames.pop().unwrap().finish());
                }
                Some(b',' | b';') => {
                    input.at += 1;
                    if let Some(Frame::Object { state, .. }) = frames.last_mut() {
                        *state = ObjectState::Name;
                    }
                }
                _ => return Err(input.error("对象需要分隔符或结束括号")),
            },
            Some(Frame::Object {
                state: ObjectState::First | ObjectState::Name,
                ..
            }) => {
                let first = matches!(
                    frames.last(),
                    Some(Frame::Object {
                        state: ObjectState::First,
                        ..
                    })
                );
                if next == Some(b'}') && first {
                    input.at += 1;
                    completed = Some(frames.pop().unwrap().finish());
                    continue;
                }
                let name = match next {
                    Some(b'\'' | b'"') => input.quoted()?,
                    Some(_) => input.bare()?.to_owned(),
                    None => return Err(input.error("对象缺少字段名")),
                };
                match input.peek()? {
                    Some(b':') => input.at += 1,
                    Some(b'=') => {
                        input.at += 1;
                        if input.remaining().starts_with('>') {
                            input.at += 1;
                        }
                    }
                    _ => return Err(input.error("字段名之后缺少冒号或等号")),
                }
                if let Some(Frame::Object {
                    name: slot, state, ..
                }) = frames.last_mut()
                {
                    *slot = Some(name);
                    *state = ObjectState::Value;
                }
            }
            _ => {
                if let Some(Frame::Array { state, .. }) = frames.last() {
                    if next == Some(b']') && *state == ArrayState::First {
                        input.at += 1;
                        completed = Some(frames.pop().unwrap().finish());
                        continue;
                    }
                    if matches!(next, Some(b',' | b';' | b']')) {
                        completed = Some(Value::Null);
                        continue;
                    }
                }
                match next {
                    Some(b'{') | Some(b'[') => {
                        if frames.len() == 128 {
                            return Err(input.error("结构超过 128 层资源预算"));
                        }
                        input.at += 1;
                        frames.push(if next == Some(b'{') {
                            Frame::Object {
                                values: Map::new(),
                                name: None,
                                state: ObjectState::First,
                            }
                        } else {
                            Frame::Array {
                                values: Vec::new(),
                                state: ArrayState::First,
                            }
                        });
                    }
                    Some(b'\'' | b'"') => completed = Some(Value::String(input.quoted()?)),
                    Some(_) => {
                        let literal = input.bare()?;
                        completed = Some(if literal.eq_ignore_ascii_case("null") {
                            Value::Null
                        } else if literal.eq_ignore_ascii_case("true") {
                            Value::Bool(true)
                        } else if literal.eq_ignore_ascii_case("false") {
                            Value::Bool(false)
                        } else if literal.len() < 1024 && number(literal.as_bytes()) {
                            Value::Number(
                                literal
                                    .parse::<Number>()
                                    .map_err(|e| format!("客户端 JSON 数字无效：{e}"))?,
                            )
                        } else {
                            Value::String(literal.to_owned())
                        });
                    }
                    None if frames.is_empty() => return Err(input.error("文档缺少值")),
                    None => return Err(input.error("文档在结构结束前结束")),
                }
            }
        }
    }
}
/// 资源定义和纹理属性必须是对象，不能把普通文本误当成空模型。
pub fn object(bytes: &[u8]) -> Result<Value, String> {
    let value = from_slice(bytes)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err("客户端 JSON 根节点必须是对象".into())
    }
}
/// GsonHelper 资源入口读取一个严格 JSON 对象后返回，不继续消费后面的文档。
/// 保留未消费范围供编辑器报告来源；设置、协议和 NBT 文本仍沿用各自的整文档合同。
pub struct ResourceObject {
    pub value: Value,
    pub trailing: Option<(usize, usize)>,
}
impl ResourceObject {
    pub fn diagnostic(&self) -> Option<String> {
        self.trailing.map(|(at, length)| {
            format!("客户端读取首个资源对象后返回；字节 {at} 起有 {length} 字节未消费内容")
        })
    }
}
pub fn resource_object(bytes: &[u8]) -> Result<ResourceObject, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|e| format!("客户端资源 JSON 不是 UTF-8：{e}"))?;
    let (text, bom) = text
        .strip_prefix('\u{feff}')
        .map_or((text, 0), |rest| (rest, 3));
    let mut input = serde_json::Deserializer::from_str(text).into_iter::<Value>();
    let value = input
        .next()
        .ok_or("客户端资源 JSON 缺少对象")?
        .map_err(|e| format!("客户端资源 JSON 无效：{e}"))?;
    if !value.is_object() {
        return Err("客户端资源 JSON 根节点必须是对象".into());
    }
    let at = input.byte_offset();
    let trailing = (!text[at..].trim().is_empty()).then_some((at + bom, text.len() - at));
    Ok(ResourceObject { value, trailing })
}
#[derive(PartialEq, Eq)]
enum ArrayState {
    First,
    Value,
    Separator,
}
enum ObjectState {
    First,
    Name,
    Value,
    Separator,
}
enum Frame {
    Array {
        values: Vec<Value>,
        state: ArrayState,
    },
    Object {
        values: Map<String, Value>,
        name: Option<String>,
        state: ObjectState,
    },
}
impl Frame {
    fn finish(self) -> Value {
        match self {
            Self::Array { values, .. } => Value::Array(values),
            Self::Object { values, .. } => Value::Object(values),
        }
    }
}
struct Input<'a> {
    text: &'a str,
    at: usize,
}
impl<'a> Input<'a> {
    fn remaining(&self) -> &'a str {
        &self.text[self.at..]
    }
    fn error(&self, message: &str) -> String {
        let prefix = &self.text[..self.at];
        let line = prefix.bytes().filter(|c| *c == b'\n').count() + 1;
        let column = prefix.rsplit('\n').next().unwrap().chars().count() + 1;
        format!("{message}，行 {line} 列 {column}（字节 {}）", self.at)
    }
    fn space(&mut self) -> Result<(), String> {
        loop {
            while matches!(
                self.text.as_bytes().get(self.at),
                Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
            ) {
                self.at += 1;
            }
            let rest = self.remaining();
            if rest.starts_with("/*") {
                let end = rest[2..]
                    .find("*/")
                    .ok_or_else(|| self.error("块注释没有结束"))?;
                self.at += 2 + end + 2;
            } else if rest.starts_with("//") || rest.starts_with('#') {
                self.at += rest.find(['\n', '\r']).unwrap_or(rest.len());
            } else {
                return Ok(());
            }
        }
    }
    fn peek(&mut self) -> Result<Option<u8>, String> {
        self.space()?;
        Ok(self.text.as_bytes().get(self.at).copied())
    }
    fn bare(&mut self) -> Result<&'a str, String> {
        let start = self.at;
        while self.text.as_bytes().get(self.at).is_some_and(|c| {
            !matches!(
                c,
                b'{' | b'}'
                    | b'['
                    | b']'
                    | b':'
                    | b','
                    | b';'
                    | b'='
                    | b'/'
                    | b'\\'
                    | b'#'
                    | b' '
                    | b'\t'
                    | b'\n'
                    | b'\r'
                    | b'\x0c'
            )
        }) {
            self.at += 1;
        }
        if self.at == start {
            return Err(self.error("缺少字段名或值"));
        }
        Ok(&self.text[start..self.at])
    }
    fn hex_unit(&mut self) -> Result<u16, String> {
        let end = self
            .at
            .checked_add(4)
            .ok_or_else(|| self.error("转义位置溢出"))?;
        let bytes = self
            .text
            .as_bytes()
            .get(self.at..end)
            .ok_or_else(|| self.error("Unicode 转义没有结束"))?;
        let mut value = 0u16;
        for byte in bytes {
            value = (value << 4)
                | match byte {
                    b'0'..=b'9' => u16::from(byte - b'0'),
                    b'a'..=b'f' => u16::from(byte - b'a' + 10),
                    b'A'..=b'F' => u16::from(byte - b'A' + 10),
                    _ => return Err(self.error("Unicode 转义包含非十六进制字符")),
                };
        }
        self.at = end;
        Ok(value)
    }
    fn quoted(&mut self) -> Result<String, String> {
        let quote = self.text.as_bytes()[self.at];
        self.at += 1;
        let mut out = String::new();
        loop {
            let ch = self
                .remaining()
                .chars()
                .next()
                .ok_or_else(|| self.error("字符串没有结束"))?;
            self.at += ch.len_utf8();
            if u32::from(ch) == u32::from(quote) {
                return Ok(out);
            }
            if ch != '\\' {
                out.push(ch);
                continue;
            }
            let escape = self
                .remaining()
                .chars()
                .next()
                .ok_or_else(|| self.error("转义没有结束"))?;
            self.at += escape.len_utf8();
            out.push(match escape {
                '\'' | '"' | '\\' | '/' | '\n' => escape,
                'b' => '\x08',
                'f' => '\x0c',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                'u' => {
                    let unit = self.hex_unit()?;
                    let scalar = if (0xd800..=0xdbff).contains(&unit) {
                        if !self.remaining().starts_with("\\u") {
                            return Err(self.error("Unicode 高代理项没有低代理项"));
                        }
                        self.at += 2;
                        let low = self.hex_unit()?;
                        if !(0xdc00..=0xdfff).contains(&low) {
                            return Err(self.error("Unicode 低代理项无效"));
                        }
                        0x10000 + ((u32::from(unit) - 0xd800) << 10) + u32::from(low) - 0xdc00
                    } else {
                        u32::from(unit)
                    };
                    char::from_u32(scalar)
                        .ok_or_else(|| self.error("Unicode 字符不能表示为 UTF-8"))?
                }
                _ => return Err(self.error("未知字符串转义")),
            });
        }
    }
}
fn number(bytes: &[u8]) -> bool {
    let mut i = usize::from(bytes.first() == Some(&b'-'));
    if bytes.get(i) == Some(&b'0') {
        i += 1;
    } else {
        if !bytes.get(i).is_some_and(|c| matches!(c, b'1'..=b'9')) {
            return false;
        }
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let start = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == bytes.len()
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category != crate::test_categories::Category::Formats {
        return;
    }
    let native: Value =
        serde_json::from_str(include_str!("../tests/fixtures/java-resource-1.21.1.json")).unwrap();
    for row in native["documents"].as_array().unwrap() {
        let input = row["input"].as_str().unwrap();
        let parsed = resource_object(input.as_bytes());
        assert_eq!(
            parsed.is_ok(),
            row["accepted"].as_bool().unwrap(),
            "{input:?}"
        );
        if let Ok(parsed) = parsed {
            assert_eq!(parsed.value, row["value"], "{input:?}");
        }
    }
    let bom = resource_object("\u{feff}{} tail".as_bytes()).unwrap();
    assert_eq!(bom.trailing, Some((5, 5)));
    assert!(resource_object(b"{} \r\n\t").unwrap().trailing.is_none());
    fn shape(value: &Value) -> Value {
        match value {
            Value::Null => serde_json::json!({"type":"null"}),
            Value::Array(values) => {
                serde_json::json!({"type":"array","value":values.iter().map(shape).collect::<Vec<_>>()})
            }
            Value::Object(values) => {
                serde_json::json!({"type":"object","value":values.iter().map(|(k,v)|(k.clone(),shape(v))).collect::<Map<_,_>>()})
            }
            Value::String(value) => serde_json::json!({"type":"string","value":value}),
            Value::Bool(value) => serde_json::json!({"type":"boolean","value":value.to_string()}),
            Value::Number(value) => serde_json::json!({"type":"number","value":value.to_string()}),
        }
    }
    let reference: Value =
        serde_json::from_str(include_str!("../tests/fixtures/client-json-2.10.1.json")).unwrap();
    // Gson 保留数值原文，serde_json 会规范化指数符号；比较无精度损失的数值表示。
    fn normalize_numbers(shape: &mut Value) {
        if shape["type"] == "number" {
            shape["value"] = Value::String(
                shape["value"]
                    .as_str()
                    .unwrap()
                    .parse::<Number>()
                    .unwrap()
                    .to_string(),
            );
        } else {
            match shape.get_mut("value") {
                Some(Value::Array(items)) => {
                    for item in items {
                        normalize_numbers(item);
                    }
                }
                Some(Value::Object(fields)) => {
                    for item in fields.values_mut() {
                        normalize_numbers(item);
                    }
                }
                _ => {}
            }
        }
    }
    let mut differences = Vec::new();
    for row in reference.as_array().unwrap() {
        let text = row["input"].as_str().unwrap();
        let result = from_str(text);
        if result.is_ok() != row["accepted"].as_bool().unwrap() {
            differences.push(format!("客户端 JSON 接受范围不同：{text:?} => {result:?}"));
            continue;
        }
        if let Ok(value) = result {
            let actual = shape(&value);
            let mut expected = row["shape"].clone();
            normalize_numbers(&mut expected);
            if actual != expected {
                differences.push(format!(
                    "客户端 JSON 值不同：{text:?} => {actual:?}，参照 {expected:?}"
                ));
            }
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
    assert!(from_str(&format!("{}0{}", "[".repeat(129), "]".repeat(129))).is_err());
    assert!(from_str(r#""\uD800""#).is_err());
    assert!(object(b"bare model text").is_err());
}
