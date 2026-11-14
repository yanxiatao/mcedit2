//! 命令方块、告示牌的版本化表单；只产生现有文档操作，不执行命令或事件。
use crate::{
    container_data,
    nbt_document::{Document, Operation, Value},
    sign, WorldError,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
fn error(text: impl Into<String>) -> WorldError {
    WorldError::Invalid(text.into())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Command,
    Sign,
}
pub fn kind(id: &str, version: Option<i32>) -> Option<Kind> {
    if !matches!(version, Some(1343 | 3465 | 3955)) {
        return None;
    }
    match id {
        "Control" | "minecraft:command_block" => Some(Kind::Command),
        "Sign" | "minecraft:sign" => Some(Kind::Sign),
        "minecraft:hanging_sign" if version != Some(1343) => Some(Kind::Sign),
        _ => None,
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editor {
    Text,
    Command,
    Component,
    Boolean,
    Number,
    Dye,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Plain,
    Json,
}
#[derive(Debug, Clone)]
pub struct Input {
    pub text: String,
    pub mode: Mode,
}
#[derive(Debug, Clone)]
pub struct Property {
    pub path: String,
    pub label: String,
    pub value: Value,
    pub editor: Editor,
    pub removable: bool,
}
impl Property {
    pub fn display(
        &self,
        mode: Mode,
        translate: &impl Fn(&str) -> Option<String>,
    ) -> Result<String, WorldError> {
        match &self.value {
            Value::String(text) if self.editor == Editor::Component && mode == Mode::Plain => {
                sign::spans(&sign::parse_component(text).map_err(error)?, translate)
                    .map(|spans| spans.into_iter().map(|span| span.text).collect())
                    .map_err(error)
            }
            Value::String(text) => Ok(text.clone()),
            Value::Int(value) => Ok(value.to_string()),
            Value::Long(value) => Ok(value.to_string()),
            Value::Byte(value) => Ok(value.to_string()),
            _ => Err(error("表单字段类型未实现")),
        }
    }
    /// 将一种编辑表示转为原生字段值，供模式转换和整组提交共用。
    pub fn input_value(
        &self,
        input: &Input,
        translate: &impl Fn(&str) -> Option<String>,
    ) -> Result<Value, WorldError> {
        let invalid = |e| error(format!("{}：{e}", self.label));
        Ok(match &self.value {
            Value::Byte(_) => {
                let value: i8 = input.text.parse().map_err(invalid)?;
                if self.editor == Editor::Boolean && !matches!(value, 0 | 1) {
                    return Err(error("布尔值必须为 0 或 1"));
                }
                Value::Byte(value)
            }
            Value::Int(_) => Value::Int(input.text.trim().parse().map_err(invalid)?),
            Value::Long(_) => Value::Long(input.text.trim().parse().map_err(invalid)?),
            Value::String(before) if self.editor == Editor::Component => {
                if input.mode == Mode::Plain {
                    if input.text == self.display(Mode::Plain, translate)? {
                        return Ok(self.value.clone());
                    }
                    Value::String(
                        serde_json::to_string(&serde_json::json!({"text":input.text}))
                            .map_err(|e| error(e.to_string()))?,
                    )
                } else {
                    let parsed = sign::parse_component(&input.text)
                        .map_err(|e| error(format!("{}：{e}", self.label)))?;
                    if parsed
                        == sign::parse_component(before)
                            .map_err(|e| error(format!("{}：{e}", self.label)))?
                    {
                        return Ok(self.value.clone());
                    }
                    Value::String(input.text.clone())
                }
            }
            Value::String(_) => {
                if self.editor == Editor::Dye
                    && !crate::banner::COLORS.contains(&input.text.as_str())
                {
                    return Err(error(format!("未知染料颜色：{}", input.text)));
                }
                Value::String(input.text.clone())
            }
            _ => return Err(error("表单字段类型未实现")),
        })
    }
}
#[derive(Debug, Clone)]
pub struct Snapshot {
    revision: Arc<()>,
    pub kind: Kind,
    pub version: i32,
    pub properties: Vec<Property>,
    /// 可显式增加或移除的四行过滤文本；键为其完整 NBT 路径。
    pub filtered: BTreeMap<String, bool>,
    parents: BTreeMap<String, Value>,
}
/// 可选的固定表单路径；缺失与类型错误分开，路径不由用户输入。
fn optional(doc: &Document, path: &str) -> Result<Option<Value>, WorldError> {
    let (parent, name) = path.rsplit_once('/').ok_or_else(|| error("表单路径无效"))?;
    if parent.is_empty() {
        return doc.inspect_field("", name);
    }
    let Some(container) = optional(doc, parent)? else {
        return Ok(None);
    };
    match container {
        Value::Compound(_) => doc.inspect_field(parent, name),
        Value::List(list) => {
            let index: usize = name
                .parse()
                .map_err(|e| error(format!("表单行号无效：{e}")))?;
            if index >= list.items.len() {
                return Err(error("表单文字行号越界"));
            }
            doc.inspect(path).map(Some)
        }
        _ => Err(error(format!("表单路径 {parent} 不是 Compound 或 List"))),
    }
}
impl Snapshot {
    pub fn capture(doc: &Document, version: Option<i32>) -> Result<Option<Self>, WorldError> {
        let id = match doc.inspect_field("", "id")? {
            Some(Value::String(id)) => id,
            None => return Ok(None),
            _ => return Err(error("对象 id 必须为 String")),
        };
        let Some(kind) = kind(&id, version) else {
            return Ok(None);
        };
        let canonical = match id.as_str() {
            "Control" => "minecraft:command_block",
            "Sign" => "minecraft:sign",
            _ => &id,
        };
        let reference = &container_data::references(version).expect("已识别的表单版本")[canonical];
        let defaults = Document::new(reference.nbt.clone())?;
        let mut result = Self {
            revision: doc.revision(),
            kind,
            version: version.expect("已识别的表单版本"),
            properties: Vec::new(),
            filtered: BTreeMap::new(),
            parents: BTreeMap::new(),
        };
        if kind == Kind::Command {
            for (name, label, editor, removable, absent) in [
                (
                    "Command",
                    "命令",
                    Editor::Command,
                    false,
                    Value::String(String::new()),
                ),
                (
                    "CustomName",
                    "自定义名称",
                    if version == Some(1343) {
                        Editor::Text
                    } else {
                        Editor::Component
                    },
                    true,
                    Value::String("{\"text\":\"\"}".into()),
                ),
                (
                    "TrackOutput",
                    "追踪输出",
                    Editor::Boolean,
                    false,
                    Value::Byte(1),
                ),
                (
                    "SuccessCount",
                    "成功次数",
                    Editor::Number,
                    false,
                    Value::Int(0),
                ),
                (
                    "LastOutput",
                    "上次输出",
                    Editor::Component,
                    true,
                    Value::String("{\"text\":\"\"}".into()),
                ),
                (
                    "UpdateLastExecution",
                    "更新上次执行时间",
                    Editor::Boolean,
                    false,
                    Value::Byte(1),
                ),
                (
                    "LastExecution",
                    "上次执行的刻数",
                    Editor::Number,
                    true,
                    Value::Long(-1),
                ),
                ("auto", "始终激活", Editor::Boolean, false, Value::Byte(0)),
                ("powered", "已充能", Editor::Boolean, false, Value::Byte(0)),
                (
                    "conditionMet",
                    "条件已满足",
                    Editor::Boolean,
                    false,
                    Value::Byte(0),
                ),
            ] {
                let default = defaults.inspect_field("", name)?.unwrap_or(absent);
                result.add(
                    doc,
                    format!("/{name}"),
                    label.into(),
                    default,
                    editor,
                    removable,
                )?;
            }
        } else if version == Some(1343) {
            if doc.inspect_field("", "front_text")?.is_some()
                || doc.inspect_field("", "back_text")?.is_some()
            {
                return Err(error(
                    "1.12.2 告示牌不能使用现代双面文字；请核对实际数据版本",
                ));
            }
            for line in 1..=4 {
                let name = format!("Text{line}");
                result.add(
                    doc,
                    format!("/{name}"),
                    format!("第 {line} 行"),
                    defaults.inspect(&format!("/{name}"))?,
                    Editor::Component,
                    false,
                )?;
            }
        } else {
            for line in 1..=4 {
                if doc.inspect_field("", &format!("Text{line}"))?.is_some() {
                    return Err(error(
                        "现代告示牌仍含旧 Text 字段；请先在 NBT 页核对版本与来源",
                    ));
                }
            }
            for face in ["front_text", "back_text"] {
                let parent = format!("/{face}");
                let default = defaults.inspect(&parent)?;
                if let Some(value) = optional(doc, &parent)? {
                    if !matches!(value, Value::Compound(_)) {
                        return Err(error("告示牌文字面必须为 Compound"));
                    }
                }
                result.parents.insert(parent.clone(), default);
                let path = format!("{parent}/messages");
                let messages = optional(doc, &path)?.unwrap_or(defaults.inspect(&path)?);
                result.lines(doc, &path, messages.clone(), false)?;
                let filtered = format!("{parent}/filtered_messages");
                let current = optional(doc, &filtered)?;
                result.filtered.insert(filtered.clone(), current.is_some());
                result.lines(doc, &filtered, current.unwrap_or(messages), true)?;
                for (name, label, editor) in [
                    ("color", "文字颜色", Editor::Dye),
                    ("has_glowing_text", "发光文字", Editor::Boolean),
                ] {
                    let path = format!("{parent}/{name}");
                    result.add(
                        doc,
                        path.clone(),
                        label.into(),
                        defaults.inspect(&path)?,
                        editor,
                        false,
                    )?;
                }
            }
            result.add(
                doc,
                "/is_waxed".into(),
                "已上蜡".into(),
                defaults.inspect("/is_waxed")?,
                Editor::Boolean,
                false,
            )?;
        }
        Ok(Some(result))
    }
    fn lines(
        &mut self,
        doc: &Document,
        path: &str,
        value: Value,
        filtered: bool,
    ) -> Result<(), WorldError> {
        let Value::List(list) = &value else {
            return Err(error("告示牌文字必须为 String 列表"));
        };
        if list.element_type != 8 || list.items.len() != 4 {
            return Err(error("告示牌文字必须恰好包含四行 String"));
        }
        self.parents.insert(path.into(), value.clone());
        for (line, value) in list.items.iter().enumerate() {
            self.add(
                doc,
                format!("{path}/{line}"),
                format!("{}第 {} 行", if filtered { "过滤后" } else { "" }, line + 1),
                value.clone(),
                Editor::Component,
                false,
            )?;
        }
        Ok(())
    }
    fn add(
        &mut self,
        doc: &Document,
        path: String,
        label: String,
        default: Value,
        editor: Editor,
        removable: bool,
    ) -> Result<(), WorldError> {
        let value = optional(doc, &path)?.unwrap_or(default.clone());
        if value.kind() != default.kind() {
            return Err(error(format!("{label} 与原生字段类型不符")));
        }
        if editor == Editor::Component {
            let Value::String(text) = &value else {
                unreachable!()
            };
            sign::parse_component(text).map_err(error)?;
        }
        if editor == Editor::Dye {
            let Value::String(text) = &value else {
                unreachable!()
            };
            if !crate::banner::COLORS.contains(&text.as_str()) {
                return Err(error(format!("未知告示牌颜色：{text}")));
            }
        }
        self.properties.push(Property {
            path,
            label,
            value,
            editor,
            removable,
        });
        Ok(())
    }
    pub fn current(&self, doc: &Document) -> bool {
        Arc::ptr_eq(&self.revision, &doc.revision())
    }
    fn ensure(
        &self,
        doc: &mut Document,
        path: &str,
        operations: &mut Vec<Operation>,
    ) -> Result<(), WorldError> {
        if path.is_empty() {
            return Ok(());
        }
        let (parent, name) = path.rsplit_once('/').ok_or_else(|| error("表单路径无效"))?;
        self.ensure(doc, parent, operations)?;
        if optional(doc, path)?.is_some() {
            return Ok(());
        }
        let value = self
            .parents
            .get(path)
            .ok_or_else(|| error(format!("表单缺少已核实的父节点 {path}")))?
            .clone();
        let operation = Operation::InsertField {
            path: parent.into(),
            name: name.into(),
            value,
        };
        doc.apply(&operation)?;
        operations.push(operation);
        Ok(())
    }
    /// 草稿整组验证后生成操作；调用方通过一次 apply_batch 提交为一次撤销。
    pub fn edits(
        &self,
        doc: &Document,
        inputs: &BTreeMap<String, Input>,
        groups: &BTreeMap<String, bool>,
        removed: &BTreeSet<String>,
        translate: &impl Fn(&str) -> Option<String>,
    ) -> Result<Vec<Operation>, WorldError> {
        if !self.current(doc) {
            return Err(error("表单文档已经变化，请使用当前表单"));
        }
        let mut working = doc.clone();
        let mut operations = Vec::new();
        for (path, enabled) in groups {
            if !self.filtered.contains_key(path) {
                return Err(error("未知的过滤文字组"));
            }
            if *enabled {
                self.ensure(&mut working, path, &mut operations)?;
            } else if optional(&working, path)?.is_some() {
                let operation = Operation::Remove { path: path.clone() };
                working.apply(&operation)?;
                operations.push(operation);
            }
        }
        for path in removed {
            if !self
                .properties
                .iter()
                .any(|p| &p.path == path && p.removable)
            {
                return Err(error("该字段不能由表单删除"));
            }
            if optional(&working, path)?.is_some() {
                let operation = Operation::Remove { path: path.clone() };
                working.apply(&operation)?;
                operations.push(operation);
            }
        }
        for (path, input) in inputs {
            let property = self
                .properties
                .iter()
                .find(|p| &p.path == path)
                .ok_or_else(|| error("该字段不属于当前表单"))?;
            if removed.contains(path)
                || self.filtered.keys().any(|group| {
                    path.starts_with(&(group.clone() + "/"))
                        && !groups.get(group).copied().unwrap_or(self.filtered[group])
                })
            {
                continue;
            }
            let value = property.input_value(input, translate)?;
            if value == property.value {
                continue;
            }
            let (parent, name) = path.rsplit_once('/').expect("表单路径");
            self.ensure(&mut working, parent, &mut operations)?;
            let operation = if optional(&working, path)?.is_some() {
                Operation::Replace {
                    path: path.clone(),
                    value,
                }
            } else {
                Operation::InsertField {
                    path: parent.into(),
                    name: name.into(),
                    value,
                }
            };
            working.apply(&operation)?;
            operations.push(operation);
        }
        // NBT 批次的所有路径指向同一个源快照。新父节点中的后续编辑必须折入
        // 该节点的一次插入，不能把虚拟文档里的新路径当成原始路径提交。
        let inserted = operations
            .iter()
            .filter_map(|operation| match operation {
                Operation::InsertField { path, name, .. } => Some(format!("{path}/{name}")),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let roots = inserted
            .iter()
            .filter(|path| {
                !inserted
                    .iter()
                    .any(|parent| *path != parent && path.starts_with(&(parent.clone() + "/")))
            })
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut result = Vec::new();
        for operation in operations {
            let path = match &operation {
                Operation::InsertField { path, name, .. } => format!("{path}/{name}"),
                Operation::Replace { path, .. } | Operation::Remove { path } => path.clone(),
                _ => unreachable!("表单只产生字段插入、替换与删除"),
            };
            if roots
                .iter()
                .any(|root| path != *root && path.starts_with(&(root.clone() + "/")))
            {
                continue;
            }
            result.push(match operation {
                Operation::InsertField {
                    path: parent, name, ..
                } if roots.contains(&path) => Operation::InsertField {
                    path: parent,
                    name,
                    value: working.inspect(&path)?,
                },
                operation => operation,
            });
        }
        Ok(result)
    }
}
