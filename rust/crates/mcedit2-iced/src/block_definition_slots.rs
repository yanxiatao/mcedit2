//! 模型纹理变量表；查询按实际模型快照缓存，选择和移除绑定都绑定所见定义。
use super::{current_bank, App, Message};
use crate::{
    resource_models::{ModelBank, TextureSlot},
    textures::TextureBank,
};
use iced::{
    widget::{button, column, row, text},
    Element,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};

struct Snapshot {
    models: Weak<ModelBank>,
    model: String,
    overrides: String,
    result: Result<Arc<Vec<TextureSlot>>, String>,
}
#[derive(Default)]
pub(super) struct Cache(Mutex<Option<Snapshot>>);
impl Cache {
    fn query(
        &self,
        bank: &TextureBank,
        model: &str,
        overrides: &str,
    ) -> Result<Arc<Vec<TextureSlot>>, String> {
        let models = bank.definition_model_identity();
        let mut cache = self.0.lock().expect("模型纹理变量缓存锁被毒化");
        if let Some(snapshot) = cache.as_ref().filter(|snapshot| {
            snapshot.models.ptr_eq(&models)
                && snapshot.model == model
                && snapshot.overrides == overrides
        }) {
            return snapshot.result.clone();
        }
        let result = serde_json::from_str::<BTreeMap<String, String>>(overrides)
            .map_err(|error| format!("纹理覆盖 JSON 无效：{error}"))
            .and_then(|overrides| bank.definition_texture_slots(model, &overrides))
            .map(|slots| {
                for slot in &slots {
                    if let Err(error) = &slot.effective {
                        log::error!("[definition-slots] #{}：{error}", slot.alias);
                    }
                }
                Arc::new(slots)
            });
        if let Err(error) = &result {
            log::error!("[definition-slots] {error}");
        }
        *cache = Some(Snapshot {
            models,
            model: model.into(),
            overrides: overrides.into(),
            result: result.clone(),
        });
        result
    }
}
#[derive(Debug)]
pub(crate) struct Ticket {
    pub revision: u64,
    index: usize,
    bank: Arc<TextureBank>,
    model: String,
    overrides: String,
}
#[derive(Debug, Clone)]
pub(crate) struct Request {
    pub ticket: Arc<Ticket>,
    alias: String,
    pub remove: bool,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Select(Request),
    Page(usize),
}
pub(super) fn update(app: &mut App, action: Action) -> Result<(), String> {
    match action {
        Action::Page(page) => app.block_definitions.slot_page = page,
        Action::Select(request) => {
            let ticket = &request.ticket;
            current_bank(app, &ticket.bank)?;
            let entry = app
                .block_definitions
                .draft
                .get_mut(ticket.index)
                .ok_or("方块定义不存在")?;
            if entry.model != ticket.model || entry.textures != ticket.overrides {
                return Err("模型或纹理覆盖已改变，请使用当前变量表".into());
            }
            if request.remove {
                let mut textures: BTreeMap<String, String> =
                    serde_json::from_str(&entry.textures).map_err(|error| error.to_string())?;
                textures.retain(|key, _| key.strip_prefix('#').unwrap_or(key) != request.alias);
                entry.textures =
                    serde_json::to_string(&textures).map_err(|error| error.to_string())?;
            } else {
                app.block_definitions.alias = request.alias;
            }
        }
    }
    Ok(())
}
pub(super) fn view<'a>(
    state: &'a super::State,
    bank: Arc<TextureBank>,
    index: usize,
) -> Element<'a, Message> {
    let entry = &state.draft[index];
    let slots = match state.slots.query(&bank, &entry.model, &entry.textures) {
        Ok(slots) => slots,
        Err(error) => return text(format!("模型纹理变量：{error}")).into(),
    };
    let ticket = Arc::new(Ticket {
        revision: state.revision,
        index,
        bank,
        model: entry.model.clone(),
        overrides: entry.textures.clone(),
    });
    let send = |action| Message::BlockDefinitions(super::Action::Slots(action));
    let mut body = column![
        text(format!("模型及父链纹理变量（{} 项）", slots.len())).size(15),
        text("选中变量后，从下方图片列表赋值；组合子模型的独立纹理空间不合并。").size(12)
    ]
    .spacing(4);
    if slots.is_empty() {
        body = body.push(text("该模型没有声明纹理变量。"));
    }
    let page = state.slot_page.min(slots.len().saturating_sub(1) / 20);
    for slot in slots.iter().skip(page * 20).take(20) {
        let selected = state.alias.trim().trim_start_matches('#') == slot.alias;
        let mut controls = row![button(text(format!("#{}", slot.alias)))
            .style(if selected {
                button::primary
            } else {
                button::secondary
            })
            .on_press(send(Action::Select(Request {
                ticket: Arc::clone(&ticket),
                alias: slot.alias.clone(),
                remove: false
            })))]
        .spacing(5);
        if slot.overridden {
            controls =
                controls.push(button("恢复模型值").on_press(send(Action::Select(Request {
                    ticket: Arc::clone(&ticket),
                    alias: slot.alias.clone(),
                    remove: true,
                }))));
        }
        let effective = match &slot.effective {
            Ok(Some(texture)) => format!("当前引用：{texture}"),
            Ok(None) => "当前：未绑定".into(),
            Err(error) => format!("当前：{error}"),
        };
        body = body.push(
            column![
                controls.wrap(),
                text(effective).size(12),
                text(match &slot.authored {
                    Some(value) => format!("模型声明：{value}"),
                    None => "模型声明：未绑定".into(),
                })
                .size(12)
            ]
            .spacing(2),
        );
    }
    body.push(
        row![
            button("变量上一页")
                .on_press_maybe((page > 0).then_some(send(Action::Page(page.saturating_sub(1))))),
            text(format!("{}/{}", page + 1, slots.len().div_ceil(20).max(1))),
            button("变量下一页").on_press_maybe(
                ((page + 1) * 20 < slots.len()).then_some(send(Action::Page(page + 1)))
            ),
        ]
        .spacing(5),
    )
    .into()
}
