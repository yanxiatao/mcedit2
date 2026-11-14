//! 容器属性草稿；所有修改交回对象文档的既有批次入口。
use iced::{
    widget::{button, checkbox, column, row, text, text_input},
    Element,
};
use mcedit2_world::{
    container_data::Snapshot,
    nbt_document::{Document, Operation},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
#[derive(Clone, Debug)]
pub(crate) enum Action {
    Input(Arc<Snapshot>, String, String),
    Apply(Arc<Snapshot>),
    Reset(Arc<Snapshot>),
}
#[derive(Default)]
pub(crate) struct State {
    cache: Mutex<Option<(Arc<()>, Option<i32>, Result<Arc<Snapshot>, String>)>>,
    inputs: BTreeMap<String, String>,
}
fn plain(
    property: &mcedit2_world::container_data::Property,
    version: Option<i32>,
    bank: Option<&crate::textures::TextureBank>,
    language: crate::language::Language,
) -> Result<String, String> {
    if property.name != "CustomName" || version == Some(1343) || !property.present {
        return Ok(property.text());
    }
    let value: serde_json::Value =
        serde_json::from_str(&property.text()).map_err(|e| format!("名称组件无效：{e}"))?;
    mcedit2_world::sign::spans(&value, &|key| {
        bank.and_then(|b| b.translation(key, language))
            .map(str::to_owned)
    })
    .map(|v| v.into_iter().map(|s| s.text).collect())
}
impl State {
    pub fn invalidate(&mut self) {
        *self.cache.lock().expect("容器属性缓存锁被毒化") = None;
        self.inputs.clear();
    }
    fn capture(&self, doc: &Document, version: Option<i32>) -> Result<Arc<Snapshot>, String> {
        let mut cache = self.cache.lock().expect("容器属性缓存锁被毒化");
        if let Some((_, _, result)) = cache
            .as_ref()
            .filter(|(revision, v, _)| Arc::ptr_eq(revision, &doc.revision()) && *v == version)
        {
            return result.clone();
        }
        let result = Snapshot::capture(doc, version)
            .map_err(|e| e.to_string())
            .and_then(|s| s.ok_or("当前对象不是已知容器".into()))
            .map(Arc::new);
        if let Err(error) = &result {
            log::error!("[container] {error}");
        }
        *cache = Some((doc.revision(), version, result.clone()));
        result
    }
    pub fn update(
        &mut self,
        doc: &Document,
        action: Action,
        editable: bool,
        bank: Option<&crate::textures::TextureBank>,
        language: crate::language::Language,
    ) -> Result<Vec<Operation>, String> {
        let source = match &action {
            Action::Input(s, ..) | Action::Apply(s) | Action::Reset(s) => s,
        };
        if !source.current(doc) {
            return Err("容器属性已经过期，请使用当前文档".into());
        }
        match action {
            Action::Input(source, name, value) => {
                if !source.properties.iter().any(|p| p.name == name) {
                    return Err("该属性不属于当前容器".into());
                }
                self.inputs.insert(name, value);
                Ok(Vec::new())
            }
            Action::Reset(_) => {
                self.inputs.clear();
                Ok(Vec::new())
            }
            Action::Apply(source) => {
                if !editable {
                    return Err("只读或正在处理任务，不能修改容器属性".into());
                }
                let mut input = self.inputs.clone();
                if let Some(name) = input.get("CustomName").cloned() {
                    let property = source
                        .properties
                        .iter()
                        .find(|p| p.name == "CustomName")
                        .ok_or("当前容器没有名称属性")?;
                    if !name.is_empty() && name == plain(property, source.version, bank, language)?
                    {
                        input.remove("CustomName");
                    } else if !name.is_empty() && source.version != Some(1343) {
                        input.insert(
                            "CustomName".into(),
                            serde_json::to_string(&serde_json::json!({"text":name}))
                                .map_err(|e| e.to_string())?,
                        );
                    }
                }
                source.edits(doc, &input).map_err(|e| e.to_string())
            }
        }
    }
}
pub(crate) fn view<'a>(
    state: &'a State,
    doc: &Document,
    version: Option<i32>,
    bank: Option<&crate::textures::TextureBank>,
    language: crate::language::Language,
    editable: bool,
) -> Element<'a, Action> {
    let source = match state.capture(doc, version) {
        Ok(s) => s,
        Err(error) => return text(format!("容器属性：{error}")).into(),
    };
    let mut body = column![text("容器属性").size(18)].spacing(5);
    for property in &source.properties {
        let original = match plain(property, version, bank, language) {
            Ok(s) => s,
            Err(error) => return text(error).into(),
        };
        let value = state
            .inputs
            .get(&property.name)
            .cloned()
            .unwrap_or(original);
        let name = property.name.clone();
        let snapshot = Arc::clone(&source);
        if property.boolean {
            let checked = value != "0";
            body = body.push(
                checkbox(checked)
                    .label(property.label)
                    .on_toggle(move |checked| {
                        Action::Input(
                            Arc::clone(&snapshot),
                            name.clone(),
                            if checked { "1" } else { "0" }.into(),
                        )
                    }),
            );
        } else {
            body = body.push(
                column![
                    text(property.label),
                    text_input("", &value).on_input(move |value| Action::Input(
                        Arc::clone(&snapshot),
                        name.clone(),
                        value
                    ))
                ]
                .spacing(2),
            );
        }
    }
    if source.properties.is_empty() {
        body = body.push(text("此容器没有独立属性；可在物品页编辑内容。"));
    }
    body = body.push(
        row![
            button("应用全部属性").on_press_maybe(
                (editable && !state.inputs.is_empty()).then(|| Action::Apply(Arc::clone(&source)))
            ),
            button("取消属性修改").on_press(Action::Reset(source))
        ]
        .spacing(4),
    );
    body = body.push(text("编辑保存值，不运行合成、酿造或播放。修改名称会重设文字样式。").size(12));
    body.into()
}
