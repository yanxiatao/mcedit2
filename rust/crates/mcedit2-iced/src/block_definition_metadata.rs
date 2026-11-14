//! 原 Configure Blocks 的保存属性；保留未修改字段和未知键，不改注册表或世界光照。
use iced::{
    widget::{column, text, text_input},
    Element,
};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Field {
    Opacity,
    Brightness,
    UnlocalizedName,
    EnglishName,
}
impl Field {
    const ALL: [Self; 4] = [
        Self::Opacity,
        Self::Brightness,
        Self::UnlocalizedName,
        Self::EnglishName,
    ];
    fn key(self) -> &'static str {
        match self {
            Self::Opacity => "opacity",
            Self::Brightness => "brightness",
            Self::UnlocalizedName => "unlocalizedName",
            Self::EnglishName => "englishName",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Opacity => "不透明度（旧保存属性）",
            Self::Brightness => "亮度（旧保存属性）",
            Self::UnlocalizedName => "未本地化名称（旧保存属性）",
            Self::EnglishName => "英文名称（旧保存属性）",
        }
    }
    fn value(self, input: &str) -> Result<Value, String> {
        match self {
            Self::Opacity | Self::Brightness => input
                .trim()
                .parse::<i64>()
                .map(Value::from)
                .map_err(|_| format!("{}必须为整数", self.label())),
            Self::UnlocalizedName | Self::EnglishName => Ok(Value::String(input.to_owned())),
        }
    }
}
#[derive(Clone)]
pub(super) struct Draft {
    source: Value,
    edits: BTreeMap<Field, String>,
}
impl Draft {
    pub(super) fn new(source: Value) -> Self {
        Self {
            source,
            edits: BTreeMap::new(),
        }
    }
    /// 用户明确添加旧属性时采用原 BlockDefinition 的默认值，不创建世界注册项。
    pub(super) fn defaults(name: &str) -> Self {
        Self::new(
            serde_json::json!({"opacity":15,"brightness":0,"unlocalizedName":name,"englishName":name}),
        )
    }
    pub(super) fn missing(&self) -> bool {
        self.source.as_object().is_some_and(|object| {
            Field::ALL
                .iter()
                .any(|field| !object.contains_key(field.key()))
        })
    }
    pub(super) fn add_missing(&mut self, name: &str) -> Result<(), String> {
        let object = self
            .source
            .as_object_mut()
            .ok_or("旧保存属性必须为 JSON 对象")?;
        let defaults = Self::defaults(name);
        for (key, value) in defaults.source.as_object().expect("旧属性默认值为对象") {
            object.entry(key.clone()).or_insert_with(|| value.clone());
        }
        Ok(())
    }
    pub(super) fn edit(&mut self, field: Field, input: String) -> Result<(), String> {
        if self
            .source
            .as_object()
            .ok_or("旧保存属性必须为 JSON 对象")?
            .get(field.key())
            .is_none()
        {
            return Err("旧属性字段不存在，请使用当前定义控件".into());
        }
        self.edits.insert(field, input);
        Ok(())
    }
    pub(super) fn value(&self) -> Result<Value, String> {
        let mut value = self.source.clone();
        let object = value.as_object_mut().ok_or("旧保存属性必须为 JSON 对象")?;
        for (&field, input) in &self.edits {
            object.insert(field.key().into(), field.value(input)?);
        }
        Ok(value)
    }
    pub(super) fn view(&self) -> Element<'_, (Field, String)> {
        let Some(object) = self.source.as_object() else {
            return text("旧保存属性不是 JSON 对象，无法编辑；请修正定义文件后重新导入。").into();
        };
        let mut body = column![
            text("旧编辑器保存属性").size(15),
            text("名称、亮度与不透明度随定义文件保存；不改变注册 ID、资源名称或存档光照。")
                .size(12)
        ]
        .spacing(5);
        for field in Field::ALL {
            let Some(original) = object.get(field.key()) else {
                continue;
            };
            let value = self
                .edits
                .get(&field)
                .cloned()
                .unwrap_or_else(|| match original {
                    Value::String(value) => value.clone(),
                    value => value.to_string(),
                });
            body = body.push(
                column![
                    text(field.label()).size(12),
                    text_input(field.label(), &value).on_input(move |value| (field, value)),
                ]
                .spacing(2),
            );
            if let Some(input) = self.edits.get(&field) {
                if let Err(error) = field.value(input) {
                    body = body.push(text(error).size(12));
                }
            }
        }
        if let Some(flags) = object.get("rotationFlags") {
            body = body.push(text(format!("旧旋转标记：{flags}")).size(12));
        }
        body.push(
            text("其余原始字段保持原文；模型、纹理、实际状态与模型旋转使用上方设置。").size(12),
        )
        .into()
    }
}
